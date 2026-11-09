#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline checks for modality server identity and readiness wiring."""
import os
import subprocess
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


class ModalityLaneTests(unittest.TestCase):
    def test_device_and_output_binding_are_exported_to_function_child(self):
        lane = (HERE / 'live_modality_lane.sh').read_text()
        self.assertIn('GPU_DEVICES="device=$CUDA_VISIBLE_DEVICES" SERVE_OUT="$CERT_OUT/evidence/serve"', lane)
        script = ('cert_run(){ env; }; CUDA_VISIBLE_DEVICES=GPU-111; CERT_OUT=/tmp/attempt; '
                  'GPU_DEVICES="device=$CUDA_VISIBLE_DEVICES" SERVE_OUT="$CERT_OUT/evidence/serve" cert_run')
        result = subprocess.run(['bash', '-c', script], capture_output=True, text=True, check=True, timeout=5,
                                env={'PATH': os.defpath})
        self.assertIn('GPU_DEVICES=device=GPU-111\n', result.stdout)
        self.assertIn('SERVE_OUT=/tmp/attempt/evidence/serve\n', result.stdout)

    def test_creation_id_and_readiness_order(self):
        lane = (HERE / 'live_modality_lane.sh').read_text()
        self.assertLess(lane.index('> "$CERT_OUT/evidence/container-id.txt"'), lane.index('cert_r4_image_runtime'))
        self.assertLess(lane.index('> "$CERT_OUT/evidence/models.json"'), lane.index('cert_phase server-ready'))
        subprocess.run(['bash', '-n', str(HERE / 'live_modality_lane.sh')], check=True, timeout=5)


if __name__ == '__main__': unittest.main()
