#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline fake GPU identity probe controls."""
import subprocess
import unittest
from unittest.mock import patch

import stage0_device


class DeviceTests(unittest.TestCase):
    def setUp(self):
        self.tools = {'nvidia-smi': {'path': '/pinned/nvidia-smi', 'sha256': 'a' * 64}}
        self.result = subprocess.CompletedProcess([], 0, b'NVIDIA H100, GPU-11111111-1111-1111-1111-111111111111, 550.1\n', b'')
        self.patches = [patch.object(stage0_device.execution_policy, 'validate_tools', return_value=self.tools),
                        patch.object(stage0_device.execution_policy, 'file_hash', return_value='a' * 64),
                        patch.object(stage0_device.subprocess, 'run', return_value=self.result)]
        self.mocks = [p.start() for p in self.patches]
        for p in self.patches: self.addCleanup(p.stop)

    def probe(self, **kwargs):
        return stage0_device.probe(self.tools, device=kwargs.pop('device', '0'),
                                   expected_name=kwargs.pop('expected_name', 'NVIDIA H100'), **kwargs)

    def test_exact_identity_and_pinned_tool(self):
        identity = self.probe()
        self.assertEqual(identity['name'], 'NVIDIA H100')
        self.assertTrue(identity['uuid'].startswith('GPU-'))
        argv = self.mocks[2].call_args.args[0]
        self.assertEqual(argv[:2], ['/pinned/nvidia-smi', '--id=0'])
        self.assertEqual(self.mocks[2].call_args.kwargs['env']['PATH'], '/nonexistent')

    def test_wrong_identity_or_multiple_devices_fails(self):
        with self.assertRaisesRegex(ValueError, 'approved box'): self.probe(expected_name='other')
        self.result.stdout += self.result.stdout
        with self.assertRaisesRegex(ValueError, 'exactly one'): self.probe()
        with self.assertRaisesRegex(ValueError, 'exactly one'): self.probe(device='0,1')
        self.result.stdout = b'NVIDIA H100, GPU-11111111-1111-1111-1111-111111111111, 550.1\n'
        with self.assertRaisesRegex(ValueError, 'approved box'):
            self.probe(device='GPU-22222222-2222-2222-2222-222222222222')
        with self.assertRaisesRegex(ValueError, 'exactly one'): self.probe(device='GPU-11111111')

    def test_changed_tool_or_expired_cap_fails(self):
        self.mocks[1].return_value = 'b' * 64
        with self.assertRaisesRegex(ValueError, 'tool changed'): self.probe()
        self.mocks[1].return_value = 'a' * 64
        with self.assertRaisesRegex(ValueError, 'deadline elapsed'): self.probe(deadline=5, clock=lambda: 5)


if __name__ == '__main__': unittest.main()
