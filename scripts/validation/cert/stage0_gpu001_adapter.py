#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Narrow BC-GPU-001 Stage-0 adapter; not a complete box controller.

Only a normal production PASS, explicitly predeclared as COMPLETE, is supported.
All other cases and terminal classes fail closed before the first box case.
Provider/backstop identity, liveness and teardown must be independently bound
by the caller; injected callbacks alone are not an armed watchdog.
"""
from pathlib import Path

import r4_image_provenance as r4
import stage0_attempt
import stage0_controller
import stage0_milestones


class Adapter:
    def __init__(self, *, box, gpu, candidate, expectations_sha256, smoke_root, cgroup_root,
                 forwarded, control_index, control_index_sha256, selection_sha256, catalog_sha256,
                 backstop_probe, stop_and_verify):
        self.box, self.gpu, self.candidate = box, gpu, candidate
        self.expectations_sha256, self.smoke_root, self.cgroup_root = expectations_sha256, smoke_root, cgroup_root
        self.forwarded = forwarded
        self.control_index, self.control_index_sha256 = control_index, control_index_sha256
        self.selection_sha256, self.catalog_sha256 = selection_sha256, catalog_sha256
        self.backstop_probe, self.stop_callback = backstop_probe, stop_and_verify
        self.completed = None

    def validate_box(self, box, cases, expectations, expectations_sha256):
        boxes = expectations.get('boxes', [])
        matches = [item for item in boxes if isinstance(item, dict) and item.get('id') == box]
        return (box == self.box and len(matches) == 1 and matches[0].get('gpu') == self.gpu
                and matches[0].get('cgroup_root') == str(self.cgroup_root)
                and expectations_sha256 == self.expectations_sha256
                and expectations.get('candidate_sha') == self.candidate
                and expectations.get('selection_sha256') == self.selection_sha256
                and expectations.get('catalog_sha256') == self.catalog_sha256
                and expectations.get('smoke_root') == str(self.smoke_root)
                and isinstance(cases, list) and bool(cases)
                and all(item['id'] == 'BC-GPU-001' and item['allowed_classes'] == ['COMPLETE'] for item in cases))

    def backstop_armed(self, box):
        return box == self.box and self.backstop_probe(box) is True

    def preflight(self, item):
        return item['id'] == 'BC-GPU-001' and item['allowed_classes'] == ['COMPLETE']

    def launch(self, item):
        self.completed = None
        outcome = stage0_attempt.run(item, candidate=self.candidate, expectations_sha256=self.expectations_sha256,
            smoke_root=self.smoke_root, cgroup_root=self.cgroup_root, expected_gpu=self.gpu, forwarded=self.forwarded)
        if outcome.timed_out or outcome.returncode != 0:
            raise ValueError('unsupported launcher outcome')
        manifest = stage0_controller.pinned_manifest(item, self.candidate)
        output = Path(manifest['cases'][item['id']]['out'])
        record = stage0_milestones.read_json(output / 'results' / (item['id'] + '.json'))
        if record.get('result') != 'PASS':
            raise ValueError('complete classification requires a production PASS record')
        self.completed = (item['id'], output)
        return {'terminal': 'COMPLETE', 'record_result': 'PASS'}

    def verify_milestones(self, item, outcome):
        if self.completed is None or self.completed[0] != item['id'] or outcome != {'terminal': 'COMPLETE', 'record_result': 'PASS'}:
            return False
        output = self.completed[1]
        stage0_milestones.check_r4_identity(output, gpu_name=self.gpu, gpu_uuid=self.forwarded['CUDA_VISIBLE_DEVICES'],
                                             index=r4.INDEX, child=r4.AMD64, config=r4.CONFIG)
        stage0_milestones.check_readiness(output.with_name(output.name + '-lifecycle'), item['id'])
        record = stage0_milestones.check_record(output, case=item['id'], candidate=self.candidate,
            expectations_sha256=self.expectations_sha256, manifest_sha256=item['manifest']['sha256'],
            request_path=item['request_log'])
        if record.get('result') != 'PASS':
            return False
        stage0_milestones.check_execution_intake_rejection(output, index_path=self.control_index,
            index_sha256=self.control_index_sha256, candidate=self.candidate,
            selection_sha256=self.selection_sha256, catalog_sha256=self.catalog_sha256,
            expectations_sha256=self.expectations_sha256)
        return True

    def verify_cleanup(self, item):
        # stage0_attempt returns only after cgroup and container reconciliation;
        # real kernel/daemon behavior has not been verified by fake tests.
        return self.completed is not None and self.completed[0] == item['id']

    def stop_and_verify(self, box):
        return box == self.box and self.stop_callback(box) is True
