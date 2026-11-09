# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
import subprocess
import tempfile
import unittest
from pathlib import Path

import harness_integrity


class HarnessIntegrityTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.cmd('init', '-q')
        for name in ('scripts/validation/cert/case_launcher.py', 'scripts/validation/cert/validate_results.py',
                     'scripts/validation/cert/result_writer.py', 'xtask/src/main.rs', 'Cargo.toml',
                     'Cargo.lock', 'xtask/Cargo.toml', 'src/count_points.rs', '.cargo/config.toml',
                     'scripts/validation/perf/runner.sh', 'scripts/check_release_readme_links.py',
                     'test-data/vlm/shapes-512.png'):
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('fixture ' + name + '\n')
        self.cmd('add', '.')
        self.cmd('-c', 'user.name=Synthetic', '-c', 'user.email=synthetic@example.invalid',
                 'commit', '-qm', 'Synthetic fixture')
        self.sha = self.cmd('rev-parse', 'HEAD').strip()

    def cmd(self, *args):
        return subprocess.run(['/usr/bin/git', '-C', str(self.root), *args], check=True,
                              capture_output=True, text=True).stdout

    def test_candidate_tree_and_tamper_controls(self):
        pins = harness_integrity.verify(self.root, self.sha)
        self.assertEqual(len(pins), 12)
        source = self.root / 'scripts/validation/cert/case_launcher.py'
        source.write_text('modified\n')
        with self.assertRaisesRegex(ValueError, 'differs from candidate'):
            harness_integrity.verify(self.root, self.sha)
        self.cmd('checkout', '--', str(source.relative_to(self.root)))
        source.chmod(0o755)
        with self.assertRaisesRegex(ValueError, 'mode differs'):
            harness_integrity.verify(self.root, self.sha)
        source.chmod(0o644)
        extra = source.parent / 'evil.py'
        extra.write_text('print(1)\n')
        with self.assertRaisesRegex(ValueError, 'untracked'):
            harness_integrity.verify(self.root, self.sha)
        extra.unlink()
        (self.root / 'build.rs').write_text('untracked\n')
        with self.assertRaisesRegex(ValueError, 'untracked'):
            harness_integrity.verify(self.root, self.sha)
        (self.root / 'build.rs').unlink()
        source.unlink()
        with self.assertRaisesRegex(ValueError, 'missing'):
            harness_integrity.verify(self.root, self.sha)

    def test_relative_pins_are_portable_but_reject_escape_or_duplicate(self):
        local = [{'path': '/first/repo/scripts/a.py', 'sha256': 'a' * 64}]
        other = [{'path': '/second/repo/scripts/a.py', 'sha256': 'a' * 64}]
        self.assertEqual(harness_integrity.relative_pins('/first/repo', local),
                         harness_integrity.relative_pins('/second/repo', other))
        with self.assertRaises(ValueError):
            harness_integrity.relative_pins('/first/repo', local + local)
        with self.assertRaises(ValueError):
            harness_integrity.relative_pins('/first/repo', [{'path': '/first/repo/../evil', 'sha256': 'a' * 64}])
        with self.assertRaises(ValueError):
            harness_integrity.relative_pins('/first/repo', other)

    def test_commit_must_match_checked_out_tree(self):
        source = self.root / 'scripts/validation/cert/case_launcher.py'
        source.write_text('next commit\n')
        self.cmd('add', '.')
        self.cmd('-c', 'user.name=Synthetic', '-c', 'user.email=synthetic@example.invalid',
                 'commit', '-qm', 'Next fixture')
        next_sha = self.cmd('rev-parse', 'HEAD').strip()
        self.assertNotEqual(next_sha, self.sha)
        with self.assertRaisesRegex(ValueError, 'differs from candidate'):
            harness_integrity.verify(self.root, self.sha)
        self.assertEqual(len(harness_integrity.verify(self.root, next_sha)), 12)


if __name__ == '__main__':
    unittest.main()
