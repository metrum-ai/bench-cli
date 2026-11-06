# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Exercise the exact dispatch input validator without certifying a candidate."""
import os
from pathlib import Path
import subprocess
import textwrap
import unittest

WORKFLOW = Path(__file__).resolve().parents[3] / '.github/workflows/pretag-certification.yml'


class PretagWorkflowTests(unittest.TestCase):
    def validate(self, sha='a' * 40, version='1.8.0'):
        text = WORKFLOW.read_text()
        script = text.split("          python3 - <<'PY'\n", 1)[1].split('          PY\n', 1)[0]
        return subprocess.run(['python3', '-c', textwrap.dedent(script)],
                              env={**os.environ, 'CERT_CANDIDATE': sha, 'CERT_VERSION': version},
                              capture_output=True).returncode

    def test_valid(self):
        for version in ['1.8.0', '0.0.0', '2.0.0-rc.1+build.42']:
            self.assertEqual(self.validate(version=version), 0)

    def test_invalid_candidate(self):
        for sha in ['', 'main', 'a' * 39, 'a' * 41, 'g' * 40, 'a' * 40 + '\n', '$(id)']:
            self.assertNotEqual(self.validate(sha=sha), 0)

    def test_invalid_version(self):
        for version in ['', 'v1.8.0', '1.8', '01.8.0', '1.8.0-01', '1.8.0;', '1.8.0\n']:
            self.assertNotEqual(self.validate(version=version), 0)

    def test_manual_only_and_candidate_binding(self):
        text = WORKFLOW.read_text()
        triggers = text.split('on:\n', 1)[1].split('\npermissions:', 1)[0]
        self.assertNotIn('pull_request:', triggers)
        self.assertNotIn('push:', triggers)
        self.assertEqual(text.count('ref: ${{ inputs.candidate_sha }}'), 6)
        self.assertEqual(text.count('needs: validate-inputs'), 3)
        self.assertIn('needs: verify', text)
        self.assertIn('needs: dummy-binaries', text)
        self.assertIn('needs: package', text)


if __name__ == '__main__':
    unittest.main()
