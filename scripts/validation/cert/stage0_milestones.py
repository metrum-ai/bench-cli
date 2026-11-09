#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Fail-closed Stage-0 evidence checks; no certification result is emitted.

These checks consume existing production evidence. They do not prove that the
execution host is trustworthy, implement a provider backstop, or start cases.
"""
import datetime as dt
import json
import re
from pathlib import Path

import execution_evidence
import execution_intake
import manifest_tool
import result_writer
import validate_results

SHA = re.compile(r'[a-f0-9]{64}\Z')
CASE = re.compile(r'BC-[A-Z]+-[0-9]{3}\Z')


def read_json(path):
    value = json.loads(Path(path).read_text(encoding='utf-8'), object_pairs_hook=manifest_tool.unique_object)
    if not isinstance(value, dict):
        raise ValueError('expected a JSON object')
    return value


def local_file(root, name):
    root = Path(root).resolve(strict=True)
    relative = Path(name)
    if relative.is_absolute() or not relative.parts or any(part in ('.', '..') for part in relative.parts):
        raise ValueError('evidence path must be relative to the smoke case')
    current = root
    for part in relative.parts:
        current = current / part
        if current.is_symlink():
            raise ValueError('symlinked evidence is not accepted')
    if not current.is_file() or not current.resolve(strict=True).is_relative_to(root):
        raise ValueError('evidence missing or outside the smoke case')
    return current


def check_record(case_root, *, case, candidate, expectations_sha256, manifest_sha256, request_path):
    """Verify a marked production record and one bound successful request.

    This checks the production rejection directly, not a certification intake
    run. The caller must independently check GPU/image, readiness and cleanup.
    """
    if (not CASE.fullmatch(case) or not re.fullmatch(r'[a-f0-9]{40}', candidate)
            or not SHA.fullmatch(expectations_sha256) or not SHA.fullmatch(manifest_sha256)):
        raise ValueError('invalid case, candidate, or digest')
    root = Path(case_root).resolve(strict=True)
    origin = result_writer.smoke_origin(root)
    if origin is None or origin[1]['expectations_sha256'] != expectations_sha256:
        raise ValueError('unapproved smoke root')
    path = local_file(root, f'results/{case}.json')
    record = read_json(path)
    if record.get('ID') != case or record.get('candidate_sha') != candidate:
        raise ValueError('smoke record does not match the case/candidate')
    if record.get('evidence_class') != result_writer.SMOKE_CLASS or record.get('stage0_expectations_sha256') != expectations_sha256:
        raise ValueError('record is not bound to the smoke classification')
    inputs = record.get('input_hashes')
    if not isinstance(inputs, dict) or inputs.get('execution_manifest') != manifest_sha256:
        raise ValueError('record is not bound to the executed manifest')
    # A launcher fallback failure is not a record emitted by the case runner.
    if record.get('infra_history'):
        raise ValueError('infrastructure fallback is not runner/writer coverage')
    paths = record.get('evidence_paths')
    hashes = record.get('output_hashes')
    if (not isinstance(paths, list) or not paths or any(not isinstance(p, str) for p in paths)
            or len(set(paths)) != len(paths) or request_path not in paths):
        raise ValueError('request log is not listed in the writer evidence')
    if not isinstance(hashes, dict) or set(hashes) != set(paths):
        raise ValueError('writer evidence hashes are incomplete')
    for name in paths:
        if not isinstance(hashes[name], str) or not SHA.fullmatch(hashes[name]) or hashes[name] != execution_evidence.digest(local_file(root, name)):
            raise ValueError('writer evidence output hash differs')
    evidence = local_file(root, request_path)
    completed = False
    with evidence.open(encoding='utf-8') as stream:
        for line in stream:
            if not line.strip():
                continue
            row = json.loads(line, object_pairs_hook=manifest_tool.unique_object)
            if (isinstance(row, dict) and re.fullmatch(r'metrum-ai-bench-cli\.request\.v[0-9]+', str(row.get('schema_version', '')))
                    and row.get('phase') == 'measure' and row.get('error') is None
                    and row.get('outcome') == 'answer' and row.get('answer_completed') is True):
                try:
                    start = dt.datetime.fromisoformat(row['started_at'].replace('Z', '+00:00'))
                    end = dt.datetime.fromisoformat(row['completed_at'].replace('Z', '+00:00'))
                    completed = start.tzinfo is not None and end.tzinfo is not None and end >= start
                except (TypeError, ValueError, KeyError, AttributeError):
                    continue
                if completed:
                    break
    if not completed:
        raise ValueError('no successful measured request in bound runner evidence')
    try:
        validate_results.check_record(record, candidate, root, dt.datetime.now(dt.timezone.utc))
    except result_writer.NonCertificationEvidence:
        pass
    else:
        raise ValueError('isolated production intake did not reject smoke evidence')
    return record


def check_execution_intake_rejection(case_root, *, index_path, index_sha256, candidate,
                                     selection_sha256, catalog_sha256, expectations_sha256):
    """Exercise the production execution-index smoke-root guard in isolation.

    The control index is a scratch file outside the smoke tree, not a real
    certification submission. This check does not show that a full index or
    selected-set inventory would otherwise be accepted.
    """
    if (not isinstance(candidate, str) or not re.fullmatch(r'[a-f0-9]{40}', candidate)
            or any(not isinstance(x, str) or not SHA.fullmatch(x) for x in
                   (index_sha256, selection_sha256, catalog_sha256, expectations_sha256))):
        raise ValueError('invalid isolated intake inputs')
    root = Path(case_root).resolve(strict=True)
    origin = result_writer.smoke_origin(root)
    if origin is None or origin[1]['expectations_sha256'] != expectations_sha256:
        raise ValueError('unapproved smoke root')
    index = Path(index_path)
    if (index.is_symlink() or not index.is_file() or result_writer.smoke_origin(index.resolve().parent) is not None
            or execution_evidence.digest(index) != index_sha256):
        raise ValueError('control index must be pinned outside the smoke root')
    try:
        execution_intake.ExecutionIndex(index, index_sha256, root, candidate, selection_sha256, catalog_sha256)
    except result_writer.NonCertificationEvidence:
        origin = result_writer.smoke_origin(root)
        if (origin is None or origin[1]['expectations_sha256'] != expectations_sha256
                or result_writer.smoke_origin(index.resolve().parent) is not None
                or execution_evidence.digest(index) != index_sha256):
            raise ValueError('isolated intake inputs changed during rejection check')
        return
    raise ValueError('isolated production execution intake did not reject the smoke root')


def check_readiness(lifecycle_root, case):
    path = local_file(lifecycle_root, 'phases.jsonl')
    with path.open(encoding='utf-8') as stream:
        events = [json.loads(line, object_pairs_hook=manifest_tool.unique_object) for line in stream if line.strip()]
    if not any(isinstance(e, dict) and e.get('case') == case and e.get('phase') == 'server-ready'
               and isinstance(e.get('utc'), str) and e['utc'] for e in events):
        raise ValueError('missing production server-ready phase')


def check_server_evidence(case_root, *, model, record):
    """Verify the common server's bound startup log and ready models response."""
    if not isinstance(model, str) or not model:
        raise ValueError('approved model is required')
    name = 'evidence/server.log'
    if name not in record.get('evidence_paths', []):
        raise ValueError('startup server log is not bound by the writer')
    log = local_file(case_root, name)
    if log.stat().st_size == 0:
        raise ValueError('startup server log is empty')
    data = read_json(local_file(case_root, 'evidence/models.json'))
    entries = data.get('data')
    if not isinstance(entries, list) or not any(isinstance(entry, dict) and entry.get('id') == model for entry in entries):
        raise ValueError('ready server does not report the approved model')


def check_r4_identity(case_root, *, gpu_name, gpu_uuid, index, child, config):
    gpu = local_file(case_root, 'evidence/gpu.csv').read_text(encoding='utf-8').strip().split(',', 1)
    if len(gpu) != 2 or gpu[0].strip() != gpu_name or not gpu[1].strip():
        raise ValueError('GPU identity or driver differs from the reviewed expectation')
    import stage0_cleanup
    import stage0_settlement
    receipt = read_json(local_file(case_root, 'evidence/r4-image-runtime.json'))
    expected = {'image_index': index, 'platform_manifest': child, 'image_config': config,
                'platform': 'linux/amd64', 'container_verified': True, 'gpu_uuid': gpu_uuid,
                'container_id': stage0_settlement.container_id(case_root),
                'container_owner': stage0_cleanup.read_owner(case_root, 'server'),
                'gpu_visible': {'name': gpu_name, 'uuid': gpu_uuid, 'driver': gpu[1].strip()}}
    if receipt != expected:
        raise ValueError('runtime container identity differs from the reviewed expectation')
