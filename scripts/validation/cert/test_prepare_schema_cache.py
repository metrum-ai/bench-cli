# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Unit checks for dependency preparation; not a cache or certification receipt."""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name('prepare_schema_cache.sh')


class SchemaCachePreparationTests(unittest.TestCase):
    def run_with_cargo(self, fail_fetch=False):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            calls = root / 'calls'
            cargo = root / 'cargo'
            cargo.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$CALLS"\n'
                             + ('[ "$1" != fetch ] || exit 9\n' if fail_fetch else '')
                             + 'exit 0\n')
            cargo.chmod(0o755)
            env = {**os.environ, 'PATH': str(root) + os.pathsep + os.environ.get('PATH', ''), 'CALLS': str(calls)}
            result = subprocess.run(['bash', str(SCRIPT)], capture_output=True, text=True, env=env, timeout=10)
            return result, calls.read_text().splitlines()

    def test_fetch_precedes_offline_no_run_build(self):
        result, calls = self.run_with_cargo()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, ['fetch --locked', 'test --locked --offline --test schema_history --test schema_migration --no-run'])
        self.assertIn('not certification evidence', result.stdout)

    def test_fetch_failure_stops_before_offline_build(self):
        result, calls = self.run_with_cargo(fail_fetch=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, ['fetch --locked'])


if __name__ == '__main__':
    unittest.main()
