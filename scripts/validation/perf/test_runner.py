# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline mount-preparation tests; fake Docker and curl, no GPU or inference."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest

RUNNER = Path(__file__).with_name("runner.sh")


class RunnerCacheTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.home = self.root / "home"
        self.home.mkdir()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.calls = self.root / "docker-calls"
        docker = self.bin / "docker"
        docker.write_text(
            '#!/bin/sh\n'
            'test -d "$HOME/.cache/huggingface/hub" && test -w "$HOME/.cache" || exit 78\n'
            'echo "$1" >> "$FAKE_DOCKER_CALLS"\n'
            'case "$1" in\n'
            'run) for arg in "$@"; do case "$arg" in metrum.cert.owner=*) echo "${arg#*=}" > "$FAKE_OWNER";; esac; done; touch "$FAKE_STATE"; echo fixture;;\n'
            'ps) test ! -f "$FAKE_STATE" || echo fixture;;\n'
            'inspect) if [ -n "$FAKE_WRONG_OWNER" ]; then echo wrong; else IFS= read -r owner < "$FAKE_OWNER"; echo "$owner"; fi;;\n'
            'rm) rm -f "$FAKE_STATE";; logs) :;; *) exit 2;; esac\n'
            'exit 0\n'
        )
        docker.chmod(0o755)
        curl = self.bin / "curl"
        curl.write_text("#!/bin/sh\nexit 0\n")
        curl.chmod(0o755)
        self.env = dict(os.environ, HOME=str(self.home),
                        PATH=f"{self.bin}:{os.environ['PATH']}",
                        DATASETS_DIR=str(self.root / "datasets"), CUDA_VISIBLE_DEVICES="0",
                        FAKE_DOCKER_CALLS=str(self.calls), FAKE_OWNER=str(self.root / 'owner'),
                        FAKE_STATE=str(self.root / 'state'))

    def run_runner(self, *args):
        return subprocess.run(
            ["bash", str(RUNNER), *args, "q38-27b", "/bin/true", "/bin/true",
             str(self.root / "output"), "0"],
            env=self.env, capture_output=True, text=True, timeout=10,
        )

    def test_fresh_home_cache_exists_before_docker(self):
        result = self.run_runner()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.home / ".cache/huggingface/hub").is_dir())
        self.assertEqual(self.calls.read_text().splitlines(), ["run", "logs", "ps", "inspect", "logs", "rm", "ps"])
        self.assertFalse((self.root / 'state').exists())

    def test_mismatched_owner_is_never_removed(self):
        self.env['FAKE_WRONG_OWNER'] = '1'
        result = self.run_runner()
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn('rm', self.calls.read_text().splitlines())
        self.assertTrue((self.root / 'state').exists())

    def test_plan_has_no_cache_or_docker_side_effects(self):
        result = self.run_runner("--plan")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.home / ".cache").exists())
        self.assertFalse(self.calls.exists())

    def test_uncreatable_cache_stops_before_docker(self):
        (self.home / ".cache").write_text("not a directory")
        result = self.run_runner()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.calls.exists())


if __name__ == "__main__":
    unittest.main()
