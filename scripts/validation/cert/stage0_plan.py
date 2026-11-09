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
    ids, assigned = set(), {}
    for box in boxes:
        if not isinstance(box, dict) or set(box) != {'id', 'gpu', 'planned_seconds', 'hard_cap_seconds', 'cases'}:
            raise ValueError('invalid box expectation')
        ident, gpu = box['id'], box['gpu']
        if not isinstance(ident, str) or not ident or ident in ids or not isinstance(gpu, str) or not gpu:
            raise ValueError('duplicate or invalid box identity')
        ids.add(ident)
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
            stage0_controller.pinned_manifest(item, expected['candidate_sha'])
            assigned[case] = ident
    if set(assigned) != set(required):
        raise ValueError('box assignments differ from approved selected GPU set')
    if digest(selected_plan_path) != approved_plan_sha or digest(selection_path) != expected['selection_sha256'] or digest(catalog_path) != expected['catalog_sha256']:
        raise ValueError('approved selection, catalog or selected plan changed during preflight')
    stage0_controller.pinned_expectations(expectations_path, expectations_sha)
    return assigned
