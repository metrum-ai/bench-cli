#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Bind the proposed box plan to the independently approved production set.

This is preflight only: it does not approve the plan, start a watchdog or grant a
box barrier. GPU identities and actual case milestones require runtime checks.
The selected manifest plan is not the post-execution intake index.
"""
import hashlib
import json
from pathlib import Path

import manifest_tool
import result_writer
import selected_set
import stage0_controller


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def validate(expectations_path, expectations_sha, *, selection_path, catalog_path,
             selected_plan_path, approved_plan_sha):
    expected = stage0_controller.pinned_expectations(expectations_path, expectations_sha)
    if (not isinstance(approved_plan_sha, str) or not stage0_controller.SHA.fullmatch(approved_plan_sha)
            or digest(selected_plan_path) != approved_plan_sha
            or expected.get('selected_plan_sha256') != approved_plan_sha):
        raise ValueError('selected plan differs from supervisor-approved digest')
    if expected.get('selection_sha256') != digest(selection_path) or expected.get('catalog_sha256') != digest(catalog_path):
        raise ValueError('selection or catalog differs from approved expectations')
    smoke_name = expected.get('smoke_root')
    if not isinstance(smoke_name, str) or not Path(smoke_name).is_absolute():
        raise ValueError('canonical smoke root required')
    smoke_root = Path(smoke_name)
    if smoke_root.is_symlink() or not smoke_root.is_dir() or smoke_root.resolve(strict=True) != smoke_root:
        raise ValueError('canonical smoke root required')
    origin = result_writer.smoke_origin(smoke_root)
    if origin is None or origin[0] != smoke_root or origin[1]['expectations_sha256'] != expectations_sha:
        raise ValueError('smoke root is not bound to approved expectations')
    plan = json.loads(Path(selected_plan_path).read_text(), object_pairs_hook=manifest_tool.unique_object)
    selection = json.loads(Path(selection_path).read_text(), object_pairs_hook=manifest_tool.unique_object)
    if expected.get('candidate_sha') != selection.get('candidate_sha') or plan.get('candidate_sha') != selection.get('candidate_sha'):
        raise ValueError('candidate identity differs')
    # Production preflight recomputes selection and validates manifests, N/A
    # records and permitted pretrigger deferrals. This is not a launch.
    entries = selected_set.load_set(selection_path, catalog_path, selected_plan_path,
                                    allow_pretrigger=bool(plan.get('deferred_at_trigger', [])))
    required = {case: (str(path.resolve()), sha) for case, path, _manifest, sha in entries}
    if len(required) != len(entries):
        raise ValueError('duplicate selected-set entry')
    boxes = expected.get('boxes')
    if not isinstance(boxes, list) or not boxes:
        raise ValueError('box plan missing')
    ids, assigned, outputs = set(), {}, set()
    for box in boxes:
        if not isinstance(box, dict) or set(box) != {'id', 'gpu', 'cgroup_root', 'planned_seconds', 'hard_cap_seconds', 'cases'}:
            raise ValueError('invalid box expectation')
        ident, gpu = box['id'], box['gpu']
        if not isinstance(ident, str) or not ident or ident in ids or not isinstance(gpu, str) or not gpu:
            raise ValueError('duplicate or invalid box identity')
        ids.add(ident)
        cgroup_root = box['cgroup_root']
        if (not isinstance(cgroup_root, str) or not Path(cgroup_root).is_absolute()
                or '..' in Path(cgroup_root).parts or str(Path(cgroup_root)) != cgroup_root):
            raise ValueError('box needs a canonical delegated cgroup path')
        planned, cap = box['planned_seconds'], box['hard_cap_seconds']
        if type(planned) is not int or type(cap) is not int or planned <= 0 or cap * 4 != planned * 5:
            raise ValueError('box hard cap must be exactly 1.25 times planned duration')
        items = box['cases']
        if not isinstance(items, list):
            raise ValueError('box cases missing')
        names = [item.get('id') if isinstance(item, dict) else None for item in items]
        stage0_controller.box_cases(expected, ident, names)
        if sum(item['cap_seconds'] for item in items) > planned:
            raise ValueError('case caps exceed planned box duration')
        for item in items:
            case = item['id']
            if case in assigned:
                raise ValueError('case assigned to more than one box')
            pin = item['manifest']
            if case not in required or (pin['path'], pin['sha256']) != required[case]:
                raise ValueError('case manifest differs from approved selected plan')
            manifest = stage0_controller.pinned_manifest(item, expected['candidate_sha'])
            entry = manifest['cases'][case]
            name = entry.get('out') if isinstance(entry, dict) else None
            if not isinstance(name, str) or not Path(name).is_absolute():
                raise ValueError('case output must be an absolute smoke path')
            output = Path(name)
            lifecycle = output.with_name(output.name + '-lifecycle')
            control = output.with_name(output.name + '-stage0-control')
            paths = (output, lifecycle, control)
            if (output.resolve() != output or output == smoke_root or not output.is_relative_to(smoke_root)
                    or any(path.is_symlink() or path.resolve() != path or path.exists() for path in paths)
                    or any(path == other or path.is_relative_to(other) or other.is_relative_to(path)
                           for path in paths for other in outputs)):
                raise ValueError('case output is reused, overlapping or outside smoke root')
            outputs.update(paths)
            assigned[case] = ident
    if set(assigned) != set(required):
        raise ValueError('box assignments differ from approved selected GPU set')
    if digest(selected_plan_path) != approved_plan_sha or digest(selection_path) != expected['selection_sha256'] or digest(catalog_path) != expected['catalog_sha256']:
        raise ValueError('approved selection, catalog or selected plan changed during preflight')
    stage0_controller.pinned_expectations(expectations_path, expectations_sha)
    if result_writer.smoke_origin(smoke_root) != origin:
        raise ValueError('smoke root marker changed during preflight')
    return assigned
