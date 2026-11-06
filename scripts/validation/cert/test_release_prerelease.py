# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Exercise release and docs-deployment prerelease policy offline."""
import os
import re
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path


WORKFLOWS = Path(__file__).resolve().parents[3] / '.github/workflows'


class ReleasePrereleaseTests(unittest.TestCase):
    def test_identical_classifiers_and_release_wiring(self):
        workflow = (WORKFLOWS / 'release.yml').read_text(encoding='utf-8')
        deploy = (WORKFLOWS / 'deploy-docs.yml').read_text(encoding='utf-8')
        pattern = r'(?m)^          is_prerelease\(\) \{\n(?:^            .*\n)+^          \}'
        release_copies = re.findall(pattern, workflow)
        deploy_copies = re.findall(pattern, deploy)
        self.assertEqual(len(release_copies), 3)
        self.assertEqual(len(deploy_copies), 1)
        copies = release_copies + deploy_copies
        self.assertTrue(all(copy == copies[0] for copy in copies))
        self.assertIn('if is_prerelease "${RELEASE_TAG}"; then', workflow)
        self.assertIn('if ! is_prerelease "${t}"; then', workflow)
        self.assertIn("prerelease: ${{ steps.release-kind.outputs.prerelease == 'true' }}", workflow)
        self.assertIn("needs.verify.outputs.prerelease == 'false'", workflow)
        for function in copies:
            for tag, expected in [('v1.8.0', False), ('v1.8.0-rc.1', True),
                                  ('v1.8.0-preview.1', True), ('v1.8.0-beta.2', True),
                                  ('v1.8.0-alpha.1', True), ('v1.8.0-dev', True),
                                  ('v1.8.0+build-1', False)]:
                with self.subTest(tag=tag):
                    result = subprocess.run(['bash', '-c', textwrap.dedent(function) + '\nis_prerelease "$1"',
                                             'test', tag], check=False)
                    self.assertEqual(result.returncode, 0 if expected else 1)

    def test_deploy_resolver_never_promotes_preview(self):
        deploy = (WORKFLOWS / 'deploy-docs.yml').read_text(encoding='utf-8')
        step = deploy.split('      - name: Resolve docs VERSION, LATEST policy, and RESTIC_ENV\n', 1)[1]
        script = textwrap.dedent(step.split('        run: |\n', 1)[1].split('      - name:', 1)[0])
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            def git(*args):
                subprocess.run(['git', *args], cwd=root, check=True, capture_output=True)
            git('init', '-q')
            git('-c', 'user.name=Test', '-c', 'user.email=test@example.invalid', 'commit', '--allow-empty', '-qm', 'fixture')
            git('tag', 'v1.7.2')
            git('tag', 'v1.8.0-preview.1')
            for tag, latest, latest_version in [('v1.8.0-preview.1', '0', 'v1.7.2'),
                                                ('v1.8.0-rc.1', '0', 'v1.7.2'),
                                                ('v1.8.0', '1', 'v1.8.0')]:
                with self.subTest(tag=tag):
                    output = root / 'output'
                    output.write_text('', encoding='utf-8')
                    env = {**os.environ, 'GITHUB_REF_TYPE': 'tag', 'GITHUB_REF_NAME': tag,
                           'GITHUB_OUTPUT': str(output), 'GITHUB_STEP_SUMMARY': str(root / 'summary')}
                    subprocess.run(['bash', '-c', script], cwd=root, env=env, check=True, capture_output=True)
                    values = dict(line.split('=', 1) for line in output.read_text().splitlines())
                    self.assertEqual(values['latest'], latest)
                    self.assertEqual(values['latest_version'], latest_version)
                    self.assertEqual(values['restic_env'], 'production')


if __name__ == '__main__':
    unittest.main()
