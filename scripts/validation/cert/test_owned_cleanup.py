# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU fake-process cleanup tests; no GPU result is produced."""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


class CleanupTests(unittest.TestCase):
    def run_cleanup(self, actual):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); (root / 'evidence').mkdir(); (root / 'results').mkdir()
            fake = root / 'docker'
            fake.write_text('#!/bin/sh\nprintf "%s\\n" "$1" >> "$DOCKER_LOG"\n'
                            'case "$1" in inspect) printf "%s\\n" "$ACTUAL_OWNER";; logs) :;; rm) :;; ps) :;; *) exit 2;; esac\n')
            fake.chmod(0o755)
            env = {**os.environ, 'PATH': str(root) + os.pathsep + os.environ['PATH'], 'DOCKER_LOG': str(root / 'calls'),
                   'ACTUAL_OWNER': actual, 'OUT': str(root), 'CERT_HOME': str(HERE)}
            script = 'source "$CERT_HOME/common.sh"; CERT_DIR="$CERT_HOME"; CERT_OUT="$OUT"; CERT_STARTED=1; CERT_CONTAINER=fixture; CERT_CONTAINER_OWNER=expected; CERT_CASES=(); cert_on_exit 0'
            result = subprocess.run(['bash', '-c', script], env=env, capture_output=True, text=True, timeout=10)
            calls = (root / 'calls').read_text().splitlines()
            return result.returncode, calls

    def test_only_matching_owner_removed_and_absence_checked(self):
        code, calls = self.run_cleanup('expected')
        self.assertEqual(code, 0)
        self.assertEqual(calls, ['inspect', 'logs', 'rm', 'ps'])

    def test_mismatch_never_removed(self):
        code, calls = self.run_cleanup('someone-else')
        self.assertNotEqual(code, 0)
        self.assertEqual(calls, ['inspect'])


if __name__ == '__main__': unittest.main()
