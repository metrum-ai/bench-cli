# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Exercise the exact dispatch input validator without certifying a candidate."""
import os
from pathlib import Path
import re
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

    def assert_checkout_bindings(self, text):
        # Check each step rather than allowing an unpinned step to borrow another's ref.
        steps = text.split('      - uses: actions/checkout@')[1:]
        self.assertEqual(len(steps), 6)
        for step in steps:
            block = step.split('\n      - ', 1)[0]
            self.assertEqual(re.findall(r'^\s+ref:\s*(.+)$', block, re.M), ['${{ inputs.candidate_sha }}'])
            self.assertEqual(re.findall(r'^\s+persist-credentials:\s*(.+)$', block, re.M), ['false'])

    def test_each_checkout_rejects_branch_substitution_or_persisted_credentials(self):
        text = WORKFLOW.read_text()
        for field, value in [('ref: ${{ inputs.candidate_sha }}', 'ref: main'),
                             ('persist-credentials: false', 'persist-credentials: true')]:
            matches = list(re.finditer(re.escape(field), text))
            self.assertEqual(len(matches), 6)
            for index, match in enumerate(matches):
                with self.subTest(field=field, checkout=index), self.assertRaises(AssertionError):
                    self.assert_checkout_bindings(text[:match.start()] + value + text[match.end():])

    def test_manual_only_and_candidate_binding(self):
        text = WORKFLOW.read_text()
        triggers = text.split('on:\n', 1)[1].split('\npermissions:', 1)[0]
        self.assertNotIn('pull_request:', triggers)
        self.assertNotIn('push:', triggers)
        self.assert_checkout_bindings(text)
        self.assertEqual(text.count('needs: validate-inputs'), 3)
        self.assertIn('needs: verify', text)
        self.assertIn('needs: dummy-binaries', text)
        self.assertIn('needs: package', text)


if __name__ == '__main__':
    unittest.main()
