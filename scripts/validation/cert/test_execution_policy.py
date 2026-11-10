#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
import hashlib
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import execution_policy as policy


def pin(path):
    path = Path(path).resolve(strict=True)
    return {'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}


class ExecutionPolicyTests(unittest.TestCase):
    def tools(self):
        return {'bash': pin('/bin/bash'), 'env': pin('/usr/bin/env'), 'python3': pin(sys.executable)}

    def test_shadowed_path_and_injection_variables_are_not_inherited(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            shadow = root / 'shadow'
            shadow.mkdir()
            marker = root / 'shadow-used'
            fake = shadow / 'python3'
            fake.write_text(f'#!/bin/sh\ntouch "{marker}"\nexit 0\n')
            fake.chmod(0o755)
            env = policy.prepare(self.tools(), root, {
                'PATH': str(shadow), 'PYTHONPATH': str(shadow), 'BASH_ENV': str(fake),
                'LD_PRELOAD': str(fake), 'CUDA_VISIBLE_DEVICES': '0', 'CERT_OFFLINE': '1',
            })
            self.assertNotIn('PYTHONPATH', env)
            self.assertNotIn('BASH_ENV', env)
            self.assertNotIn('LD_PRELOAD', env)
            self.assertEqual(env['CUDA_VISIBLE_DEVICES'], '0')
            result = subprocess.run([self.tools()['bash']['path'], '-c',
                                     'python3 -c "print(12345)"'], env=env,
                                    capture_output=True, text=True, check=True)
            self.assertEqual(result.stdout.strip(), '12345')
            self.assertFalse(marker.exists(), 'PATH-shadowed interpreter ran')
            policy.verify(self.tools(), root / 'tool-bin')

    def test_changed_tool_or_link_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            local = root / 'tool'
            original = Path('/bin/true').read_bytes()
            local.write_bytes(original)
            local.chmod(0o755)
            tools = {**self.tools(), 'tool': pin(local)}
            policy.prepare(tools, root, {})
            local.write_bytes(original + b'changed')
            with self.assertRaisesRegex(ValueError, 'changed'):
                policy.verify(tools, root / 'tool-bin')
            local.write_bytes(original)
            (root / 'tool-bin/tool').unlink()
            (root / 'tool-bin/tool').symlink_to('/bin/true')
            with self.assertRaisesRegex(ValueError, 'link changed'):
                policy.verify(tools, root / 'tool-bin')

    def test_rejects_pinned_script_shim(self):
        with tempfile.TemporaryDirectory() as temporary:
            shim = Path(temporary) / 'tool'
            shim.write_text('#!/bin/sh\nexit 0\n')
            shim.chmod(0o755)
            with self.assertRaisesRegex(ValueError, 'script shim'):
                policy.validate_tools({**self.tools(), 'tool': pin(shim)})

    def test_reject_ambiguous_and_noncanonical_pins(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'tools.json'
            path.write_text('{"bash":{},"bash":{}}')
            with self.assertRaisesRegex(ValueError, 'duplicate'):
                policy.load_manifest(path)
            tools = self.tools()
            tools['python3']['path'] = './relative'
            with self.assertRaisesRegex(ValueError, 'path'):
                policy.validate_tools(tools)


if __name__ == '__main__':
    unittest.main()
