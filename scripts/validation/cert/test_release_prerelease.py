# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Exercise both actual release-workflow prerelease classifiers offline."""
import re
import subprocess
import textwrap
import unittest
from pathlib import Path


WORKFLOW = Path(__file__).resolve().parents[3] / '.github/workflows/release.yml'


class ReleasePrereleaseTests(unittest.TestCase):
    def test_identical_classifiers_and_release_wiring(self):
        workflow = WORKFLOW.read_text(encoding='utf-8')
        copies = re.findall(r'(?m)^          is_prerelease\(\) \{\n(?:^            .*\n)+^          \}', workflow)
        self.assertEqual(len(copies), 2)
        self.assertEqual(copies[0], copies[1])
        self.assertIn('if is_prerelease "${RELEASE_TAG}"; then', workflow)
        self.assertIn('if ! is_prerelease "${t}"; then', workflow)
        self.assertIn("prerelease: ${{ steps.release-kind.outputs.prerelease == 'true' }}", workflow)
        for function in copies:
            for tag, expected in [('v1.8.0', False), ('v1.8.0-rc.1', True),
                                  ('v1.8.0-preview.1', True), ('v1.8.0-beta.2', True),
                                  ('v1.8.0-alpha.1', True), ('v1.8.0-dev', True),
                                  ('v1.8.0+build-1', False)]:
                with self.subTest(tag=tag):
                    result = subprocess.run(['bash', '-c', textwrap.dedent(function) + '\nis_prerelease "$1"',
                                             'test', tag], check=False)
                    self.assertEqual(result.returncode, 0 if expected else 1)


if __name__ == '__main__':
    unittest.main()
