# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""CPU fake-process cleanup tests; no GPU result is produced."""
import hashlib
import json
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

    def test_auxiliary_perf_orphan_is_failure_and_only_owner_removed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); (root / 'evidence').mkdir(); (root / 'results').mkdir()
            fake = root / 'docker'; state = root / 'state'; state.touch()
            fake.write_text('#!/bin/sh\nprintf "%s\\n" "$1" >> "$DOCKER_LOG"\n'
                            'case "$1" in ps) test ! -f "$STATE" || echo fixture;; inspect) echo expected;; '
                            'logs) :;; rm) rm -f "$STATE";; *) exit 2;; esac\nexit 0\n')
            fake.chmod(0o755)
            env = {**os.environ, 'PATH': str(root) + os.pathsep + os.environ['PATH'], 'DOCKER_LOG': str(root / 'calls'),
                   'STATE': str(state), 'OUT': str(root), 'CERT_HOME': str(HERE)}
            script = 'source "$CERT_HOME/common.sh"; CERT_DIR="$CERT_HOME"; CERT_OUT="$OUT"; CERT_STARTED=0; CERT_AUX_OWNER=expected; CERT_CASES=(); cert_on_exit 0'
            result = subprocess.run(['bash', '-c', script], env=env, capture_output=True, text=True, timeout=10)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual((root / 'calls').read_text().splitlines(), ['ps', 'inspect', 'logs', 'rm', 'ps'])
            self.assertFalse(state.exists())
            self.assertTrue((root / 'results/lane-error.json').exists())

    def test_only_matching_owner_removed_and_absence_checked(self):
        code, calls = self.run_cleanup('expected')
        self.assertEqual(code, 0)
        self.assertEqual(calls, ['inspect', 'logs', 'rm', 'ps'])

    def test_mismatch_never_removed(self):
        code, calls = self.run_cleanup('someone-else')
        self.assertNotEqual(code, 0)
        self.assertEqual(calls, ['inspect'])

    def test_hashed_case_log_survives_final_server_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); evidence = root / 'evidence'; evidence.mkdir(); (root / 'results').mkdir()
            fake = root / 'docker'
            fake.write_text('#!/bin/sh\ncase "$1" in logs) printf "startup\\n"; '
                            '[ "${PHASE:-}" != teardown ] || printf "workload-complete\\n";; '
                            'rm) :;; *) exit 2;; esac\n')
            fake.chmod(0o755)
            record = {'ID': 'BC-GPU-001', 'result': 'FAIL', 'candidate_sha': 'a' * 40,
                      'evidence_paths': ['evidence/server.log']}
            (root / 'record.json').write_text(json.dumps(record))
            env = {**os.environ, 'PATH': str(root) + os.pathsep + os.environ['PATH'],
                   'OUT': str(root), 'CERT_HOME': str(HERE)}
            script = ('set -e; source "$CERT_HOME/common.sh"; CERT_DIR="$CERT_HOME"; CERT_OUT="$OUT"; '
                      'CERT_STARTED=1; CERT_CONTAINER=fixture; CERT_CASES=(); cert_server_logs; '
                      'python3 "$CERT_HOME/result_writer.py" --out "$OUT" --record "$OUT/record.json"; '
                      'export PHASE=teardown; cert_on_exit 0')
            completed = subprocess.run(['bash', '-c', script], env=env, capture_output=True, text=True, timeout=10)
            self.assertEqual(completed.returncode, 0, completed.stderr)
            result = json.loads((root / 'results/BC-GPU-001.json').read_text())
            retained = (evidence / 'server.log').read_bytes()
            self.assertEqual(result['output_hashes']['evidence/server.log'], hashlib.sha256(retained).hexdigest())
            self.assertEqual(retained, b'startup\n')
            final = (evidence / 'server-teardown.log').read_bytes()
            self.assertEqual(final, b'startup\nworkload-complete\n')
            self.assertEqual((evidence / 'server-teardown.sha256').read_text().split()[0], hashlib.sha256(final).hexdigest())


if __name__ == '__main__': unittest.main()
