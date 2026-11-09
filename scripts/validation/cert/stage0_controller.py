#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Per-box Stage-0 control flow. NOT a runnable GPU controller by itself.

A production adapter must bind these hooks to frozen production launcher,
independent watchdog, real evidence checks and owned-resource verification.
Until that adapter and full case coverage are reviewed this module cannot grant
a box barrier or authorize certification.
"""
import hashlib
import json
import re
import signal
from pathlib import Path

import manifest_tool

CASE = re.compile(r'BC-[A-Z]+-[0-9]{3}\Z')
SHA = re.compile(r'[a-f0-9]{64}\Z')
CLASSES = frozenset({'COMPLETE', 'INCOMPLETE_GRID_FAIL', 'PRE_CANDIDATE_CONTROLLED_STOP'})
PREVIEW_CASES = frozenset({'BC-PERF-001', 'BC-PERF-002', 'BC-TELEM-001', 'BC-PERF-003'})


def pinned_expectations(path, expected):
    if not isinstance(expected, str) or not SHA.fullmatch(expected):
        raise ValueError('expected expectations SHA-256 is required')
    path = Path(path)
    if path.is_symlink() or not path.is_file():
        raise ValueError('expectations must be a regular non-symlink file')
    raw = path.read_bytes()
    if hashlib.sha256(raw).hexdigest() != expected:
        raise ValueError('expectations hash differs from independently approved digest')
    value = json.loads(raw, object_pairs_hook=manifest_tool.unique_object)
    if not isinstance(value, dict):
        raise ValueError('expectations must be an object')
    return value


def box_cases(expectations, box, required):
    """A controller may not omit, duplicate, invent, or silently reorder cases."""
    if not isinstance(expectations.get('candidate_sha'), str) or not re.fullmatch(r'[a-f0-9]{40}', expectations['candidate_sha']):
        raise ValueError('expectations require a full candidate SHA')
    if not isinstance(box, str) or not box or not isinstance(required, list) or not required:
        raise ValueError('explicit box and assigned cases required')
    if (any(not isinstance(x, str) or not CASE.fullmatch(x) for x in required)
            or len(required) != len(set(required))):
        raise ValueError('invalid assigned case list')
    boxes = expectations.get('boxes')
    if not isinstance(boxes, list):
        raise ValueError('expectations boxes must be a list')
    matches = [x for x in boxes if isinstance(x, dict) and x.get('id') == box]
    if len(matches) != 1 or not isinstance(matches[0].get('cases'), list):
        raise ValueError('box must have exactly one case plan')
    cases = matches[0]['cases']
    actual = []
    for item in cases:
        fields = {'id', 'allowed_classes', 'cap_seconds', 'grace_seconds', 'manifest', 'request_log'}
        if not isinstance(item, dict) or set(item) != fields:
            raise ValueError('invalid case expectation')
        case, classes = item['id'], item['allowed_classes']
        if not isinstance(case, str) or not CASE.fullmatch(case):
            raise ValueError('invalid case ID')
        if (not isinstance(classes, list) or not classes or any(not isinstance(x, str) for x in classes)
                or len(classes) != len(set(classes)) or not set(classes) <= CLASSES):
            raise ValueError('invalid allowed terminal classes')
        if 'PRE_CANDIDATE_CONTROLLED_STOP' in classes and case not in PREVIEW_CASES:
            raise ValueError('pre-candidate stop is restricted to performance cases')
        cap, grace, manifest, request_log = (item['cap_seconds'], item['grace_seconds'],
                                             item['manifest'], item['request_log'])
        if type(cap) is not int or type(grace) is not int or not 0 < grace < cap:
            raise ValueError('case cap and cleanup grace must be positive integers')
        if (not isinstance(manifest, dict) or set(manifest) != {'path', 'sha256'}
                or not isinstance(manifest['path'], str) or not Path(manifest['path']).is_absolute()
                or '..' in Path(manifest['path']).parts
                or not isinstance(manifest['sha256'], str) or not SHA.fullmatch(manifest['sha256'])):
            raise ValueError('case manifest must be absolute and hash-pinned')
        if (not isinstance(request_log, str) or not request_log or not Path(request_log).parts
                or Path(request_log).is_absolute() or '..' in Path(request_log).parts
                or '\x00' in request_log):
            raise ValueError('case request log must be relative')
        actual.append(case)
    if actual != required:
        raise ValueError('box case plan differs from independently assigned ordered cases')
    return cases


def pinned_manifest(item, candidate):
    path = Path(item['manifest']['path'])
    if path.is_symlink() or not path.is_file():
        raise ValueError('case manifest must be a regular non-symlink file')
    raw = path.read_bytes()
    if hashlib.sha256(raw).hexdigest() != item['manifest']['sha256']:
        raise ValueError('case manifest hash changed')
    manifest = json.loads(raw, object_pairs_hook=manifest_tool.unique_object)
    if (not isinstance(manifest, dict) or manifest.get('candidate_sha') != candidate
            or not isinstance(manifest.get('cases'), dict) or list(manifest['cases']) != [item['id']]):
        raise ValueError('manifest must bind the candidate and one expected case')
    return manifest


def run_box(expectations_path, expected_sha256, box, required, adapter):
    """Run the per-box state machine with a reviewed, independently bound adapter.

    Adapter methods must raise on uncertainty; this function treats only literal
    True from each verification hook as success. ``terminal`` must be a typed
    classification derived by the adapter from real case evidence; a raw error
    message is not an incomplete-grid classification. Returning from this helper
    is not a persisted or authenticated barrier.
    """
    def frozen_cases():
        return box_cases(pinned_expectations(expectations_path, expected_sha256), box, required)

    def must(name, *args):
        if getattr(adapter, name)(*args) is not True:
            raise ValueError(f'{name} did not verify')

    previous = {}
    def interrupted(signum, _frame):
        raise InterruptedError(f'Stage-0 controller interrupted by signal {signum}')

    try:
        for sig in (signal.SIGINT, signal.SIGTERM):
            previous[sig] = signal.signal(sig, interrupted)
        cases = frozen_cases()
        expectations = pinned_expectations(expectations_path, expected_sha256)
        candidate = expectations['candidate_sha']
        # Refuse an entire box before the first case if any assigned case lacks
        # an implemented and approved runtime adapter or its box identity differs.
        must('validate_box', box, cases, expectations, expected_sha256)
        must('backstop_armed', box)
        for item in cases:
            frozen_cases()  # detect changes before every case
            pinned_manifest(item, candidate)
            must('backstop_armed', box)
            must('preflight', item)
            outcome = adapter.launch(item)
            if not isinstance(outcome, dict) or set(outcome) != {'terminal', 'record_result'}:
                raise ValueError('invalid terminal observation')
            terminal, result = outcome['terminal'], outcome['record_result']
            if terminal not in item['allowed_classes'] or terminal not in CLASSES:
                raise ValueError('unexpected terminal class')
            if ((terminal == 'COMPLETE' and result != 'PASS')
                    or (terminal == 'INCOMPLETE_GRID_FAIL' and result != 'FAIL')
                    or (terminal == 'PRE_CANDIDATE_CONTROLLED_STOP' and result is not None)):
                raise ValueError('terminal class and record result disagree')
            # A preview stop has a different evidence contract and is not a
            # production record or runner/writer coverage. The adapter must
            # explicitly verify its checkpoint, candidate fence and ack.
            if terminal == 'PRE_CANDIDATE_CONTROLLED_STOP':
                must('verify_preview_boundary', item, outcome)
            else:
                must('verify_milestones', item, outcome)
            must('verify_cleanup', item)
            pinned_manifest(item, candidate)
        frozen_cases()
        must('backstop_armed', box)
        return {'classification': 'Stage-0 smoke, not certification', 'box': box,
                'expectations_sha256': expected_sha256, 'cases': list(required)}
    except BaseException as original:
        # Failure must request stop even on interruption or unexpected adapter errors.
        try:
            stopped = adapter.stop_and_verify(box)
        except BaseException as cleanup:
            raise RuntimeError('Stage-0 failed and independent box cleanup was not verified') from cleanup
        if stopped is not True:
            raise RuntimeError('Stage-0 failed and independent box cleanup was not verified') from original
        raise
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)
