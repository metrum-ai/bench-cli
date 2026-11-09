#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Verify selected-case execution bindings at final inventory intake.

The caller must independently approve the index digest and trust the executing
host and validator closure. Hashes detect changes, not a forged entire bundle.
"""
import datetime as dt
import hashlib
import json
import re
from pathlib import Path

import manifest_tool

SHA256 = re.compile(r'[0-9a-f]{64}\Z')
ID = re.compile(r'BC-[A-Z]+-[0-9]{3}\Z')
NAME = re.compile(r'[A-Za-z0-9][A-Za-z0-9_.+-]*\Z')


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def read_json(path):
    with Path(path).open(encoding='utf-8') as stream:
        return json.load(stream, object_pairs_hook=manifest_tool.unique_object,
                         parse_constant=lambda value: (_ for _ in ()).throw(ValueError(f'invalid JSON number: {value}')))


class ExecutionIndex:
    def __init__(self, path, expected_hash, root, candidate, selection_hash, catalog_hash):
        if not isinstance(expected_hash, str) or not SHA256.fullmatch(expected_hash):
            raise ValueError('independently approved execution-index SHA-256 is required')
        if digest(path) != expected_hash:
            raise ValueError('execution-index hash mismatch')
        self.root = Path(root).resolve(strict=True)
        self.candidate = candidate
        self.entries = {}
        self.used = set()
        data = read_json(path)
        if (not isinstance(data, dict) or data.get('candidate_sha') != candidate
                or data.get('selection_sha256') != selection_hash or data.get('catalog_sha256') != catalog_hash):
            raise ValueError('execution-index candidate, selection or catalog mismatch')
        plan_path = self.pin(data.get('selected_plan'))
        plan = read_json(plan_path)
        if (not isinstance(plan, dict) or plan.get('candidate_sha') != candidate
                or plan.get('selection_sha256') != selection_hash or plan.get('catalog_sha256') != catalog_hash
                or not isinstance(plan.get('manifests'), list)):
            raise ValueError('selected execution plan does not match candidate or selection')
        self.planned_manifests = set()
        for item in plan['manifests']:
            if (not isinstance(item, dict) or not isinstance(item.get('sha256'), str)
                    or not SHA256.fullmatch(item['sha256']) or item['sha256'] in self.planned_manifests):
                raise ValueError('invalid or duplicate selected-plan manifest digest')
            self.planned_manifests.add(item['sha256'])
        entries = data.get('entries')
        if not isinstance(entries, list):
            raise ValueError('execution-index entries must be a list')
        for entry in entries:
            if not isinstance(entry, dict) or set(entry) not in ({'ID', 'record_sha256', 'manifest', 'phases'},
                                                                  {'ID', 'record_sha256', 'manifest', 'phases', 'structural_manifest'}):
                raise ValueError('invalid execution-index entry')
            case = entry['ID']
            if not isinstance(case, str) or not ID.fullmatch(case) or case in self.entries:
                raise ValueError('invalid or duplicate execution-index case')
            if not isinstance(entry['record_sha256'], str) or not SHA256.fullmatch(entry['record_sha256']):
                raise ValueError('invalid execution-record digest')
            self.entries[case] = entry

    def pin(self, value):
        if not isinstance(value, dict) or set(value) != {'path', 'sha256'}:
            raise ValueError('execution proof requires path and sha256')
        name, expected = value['path'], value['sha256']
        if (not isinstance(name, str) or not name or Path(name).is_absolute()
                or '..' in Path(name).parts or not isinstance(expected, str) or not SHA256.fullmatch(expected)):
            raise ValueError('invalid execution proof path or digest')
        path = self.root / name
        current = self.root
        for part in Path(name).parts:
            current = current / part
            if current.is_symlink():
                raise ValueError('execution proof traverses a symlink')
        if not path.resolve().is_relative_to(self.root) or not path.is_file():
            raise ValueError('execution proof is missing or outside evidence root')
        if digest(path) != expected:
            raise ValueError('execution proof hash mismatch')
        return path

    def authorize(self, record, record_path, expected_lane):
        case = record.get('ID')
        entry = self.entries.get(case)
        if entry is None:
            raise ValueError('fresh PASS lacks execution-index entry')
        if digest(record_path) != entry['record_sha256'] or read_json(record_path) != record:
            raise ValueError('execution record differs from approved index')
        manifest_path = self.pin(entry['manifest'])
        manifest = read_json(manifest_path)
        if (not isinstance(manifest, dict) or manifest.get('candidate_sha') != self.candidate
                or not isinstance(expected_lane, str) or not expected_lane or manifest.get('lane') != expected_lane
                or not isinstance(manifest.get('cases'), dict) or set(manifest['cases']) != {case}):
            raise ValueError('execution manifest candidate, case or lane mismatch')
        # Recheck the candidate's source tree independently at intake. The
        # evidence may come from another checkout, so compare relative names and
        # SHA-256 rather than requiring identical execution-host absolute paths.
        import harness_integrity
        repo = Path(__file__).resolve().parents[3]
        actual_closure = harness_integrity.verify(repo, self.candidate)
        if (harness_integrity.relative_pins(manifest.get('harness_repo'), manifest.get('harness_closure'))
                != harness_integrity.relative_pins(str(repo), actual_closure)):
            raise ValueError('execution harness closure differs from candidate')
        parent = manifest.get('parent_structural_manifest')
        if parent is not None:
            if not isinstance(parent, dict) or not isinstance(parent.get('sha256'), str):
                raise ValueError('invalid structural parent pin')
            parent_path = self.pin(entry.get('structural_manifest'))
            if digest(parent_path) != parent['sha256']:
                raise ValueError('runtime structural parent digest mismatch')
            structural = read_json(parent_path)
            changed = {'parent_structural_manifest', 'generated_bindings', 'lane_manifest', 'cases', 'inputs'}
            if (not isinstance(structural, dict) or 'generated_bindings' in structural
                    or {k: v for k, v in manifest.items() if k not in changed}
                    != {k: v for k, v in structural.items() if k not in changed}):
                raise ValueError('runtime changed frozen structural identity')
            if (not isinstance(structural.get('inputs'), list) or not isinstance(manifest.get('inputs'), list)
                    or any(item not in manifest['inputs'] for item in structural['inputs'])):
                raise ValueError('runtime omitted frozen structural inputs')
            from runtime_inputs import commands
            try:
                expected_cases = commands(structural['cases'], structural['lane_manifest']['path'],
                                          manifest['lane_manifest']['path'])
            except (KeyError, TypeError) as exc:
                raise ValueError('runtime lacks the frozen lane command') from exc
            if manifest['cases'] != expected_cases:
                raise ValueError('runtime changed the frozen case command')
            source_digest = parent['sha256']
        else:
            if 'structural_manifest' in entry:
                raise ValueError('unexpected structural manifest proof')
            source_digest = entry['manifest']['sha256']
        if source_digest not in self.planned_manifests:
            raise ValueError('execution manifest is not bound to the frozen selected plan')
        inputs, versions = record.get('input_hashes'), record.get('versions')
        if not isinstance(inputs, dict) or inputs.get('execution_manifest') != entry['manifest']['sha256']:
            raise ValueError('record does not bind the execution manifest')
        if record.get('input_integrity_errors', []) != []:
            raise ValueError('record lacks clean input-integrity status')
        tools = manifest.get('tools')
        if not isinstance(versions, dict) or versions.get('execution_tools') != tools or not isinstance(tools, dict):
            raise ValueError('record tool pins differ from execution manifest')
        if not {'bash', 'env', 'python3'} <= tools.keys():
            raise ValueError('execution tool pins are incomplete')
        for name, item in tools.items():
            if (not isinstance(name, str) or not NAME.fullmatch(name) or not isinstance(item, dict)
                    or set(item) != {'path', 'sha256'} or not isinstance(item['path'], str)
                    or not Path(item['path']).is_absolute() or str(Path(item['path'])) != item['path']
                    or '..' in Path(item['path']).parts or not isinstance(item['sha256'], str)
                    or not SHA256.fullmatch(item['sha256'])):
                raise ValueError('invalid absolute execution tool pin')
        outputs = record.get('output_hashes')
        paths = record.get('evidence_paths')
        if not isinstance(paths, list) or not isinstance(outputs, dict) or set(paths) != set(outputs):
            raise ValueError('record lacks complete output hashes')
        for name in paths:
            self.pin({'path': name, 'sha256': outputs[name]})
        phases_path = self.pin(entry['phases'])
        phases = [json.loads(line, object_pairs_hook=manifest_tool.unique_object)
                  for line in phases_path.read_text(encoding='utf-8').splitlines() if line.strip()]
        if (not phases or any(not isinstance(item, dict) or item.get('case') != case for item in phases)
                or sum(item.get('phase') == 'inputs-ready' for item in phases) != 1
                or sum(item.get('phase') == 'record-validated' for item in phases) != 1
                or phases[-1].get('phase') != 'record-validated'):
            raise ValueError('case lifecycle is incomplete or mismatched')
        try:
            clocks = [dt.datetime.fromisoformat(item['utc'].replace('Z', '+00:00')) for item in phases]
            end = dt.datetime.fromisoformat(record['end_utc'].replace('Z', '+00:00'))
            if (end.tzinfo is None or any(clock.tzinfo is None for clock in clocks)
                    or clocks != sorted(clocks) or clocks[-1] < end):
                raise ValueError('case lifecycle clocks are inconsistent')
        except (KeyError, TypeError, AttributeError) as exc:
            raise ValueError('case lifecycle timestamps are invalid') from exc
        self.used.add(case)

    def unused_errors(self):
        return [f'unused execution-index entry {case}' for case in sorted(self.entries.keys() - self.used)]
