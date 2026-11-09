#!/usr/bin/env python3
"""No-Docker tests for the pinned local command adapter."""
import hashlib
import subprocess
import tempfile
import unittest
from pathlib import Path

from stage0_docker import Docker

OWNER = 'a' * 32
IDENT = 'b' * 64


class DockerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        path = Path('/usr/bin/true').resolve()
        self.pin = {'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}
        self.calls = []
        self.output = b''
        def invoke(argv, **kwargs):
            self.calls.append((argv, kwargs))
            return subprocess.CompletedProcess(argv, 0, self.output, b'')
        self.docker = Docker(self.pin, self.tmp.name, owner=OWNER, invoke=invoke)

    def test_exact_scoped_inventory_and_inspect(self):
        self.output = (IDENT + '\n').encode()
        self.assertEqual(self.docker.inventory(OWNER), [IDENT])
        argv, kwargs = self.calls[-1]
        self.assertIn('label=metrum.cert.owner=' + OWNER, argv)
        self.assertEqual(argv[1:3], ['--host', 'unix:///var/run/docker.sock'])
        self.assertEqual(kwargs['env']['PATH'], '/nonexistent')
        self.output = (OWNER + '\n').encode()
        self.assertEqual(self.docker.inspect_owner(IDENT), OWNER)
        self.docker.remove(IDENT)
        self.assertEqual(self.calls[-1][0][-3:], ['rm', '-f', IDENT])

    def test_bad_arguments_or_outputs_fail_before_removal(self):
        for ident in ('../bad', 'invalid', IDENT + 'x'):
            with self.subTest(ident=ident), self.assertRaises(ValueError): self.docker.remove(ident)
        self.assertFalse(self.calls)
        self.output = (IDENT + '\n' + IDENT + '\n').encode()
        with self.assertRaises(ValueError): self.docker.inventory(OWNER)
        self.output = b'<no value>\n'
        with self.assertRaises(ValueError): self.docker.inspect_owner(IDENT)
        self.output = ('c' * 32 + '\n').encode()
        with self.assertRaisesRegex(ValueError, 'mismatch'): self.docker.remove(IDENT)
        self.assertNotIn('rm', self.calls[-1][0])
        with self.assertRaisesRegex(ValueError, 'inventory owner'): self.docker.inventory('c' * 32)

    def test_real_reconciler_with_fake_docker_transport(self):
        import stage0_cleanup
        self.docker.config.rmdir()
        alive = [True]
        calls = []
        def invoke(argv, **_kwargs):
            calls.append(argv)
            if 'ps' in argv: out = (IDENT + '\n').encode() if alive[0] else b''
            elif 'inspect' in argv: out = (OWNER + '\n').encode()
            elif 'rm' in argv: alive[0] = False; out = (IDENT + '\n').encode()
            else: raise AssertionError('unexpected command')
            return subprocess.CompletedProcess(argv, 0, out, b'')
        docker = Docker(self.pin, self.tmp.name, owner=OWNER, invoke=invoke)
        self.assertEqual(stage0_cleanup.reconcile(OWNER, docker, creation_closed=True), 'leak_removed')
        self.assertFalse(alive[0])
        self.assertEqual(sum('rm' in call for call in calls), 1)

    def test_cleanup_deadline_bounds_subprocess_and_fails_closed(self):
        self.docker.config.rmdir()
        now = [10.0]
        docker = Docker(self.pin, self.tmp.name, owner=OWNER, deadline=12.0,
                        clock=lambda: now[0], invoke=self.docker.invoke)
        self.output = b''
        self.assertEqual(docker.inventory(OWNER), [])
        self.assertEqual(self.calls[-1][1]['timeout'], 2.0)
        now[0] = 12.0
        with self.assertRaisesRegex(ValueError, 'deadline elapsed'): docker.inventory(OWNER)
        self.assertEqual(len(self.calls), 1)

    def test_changed_pin_or_config_fails_before_subprocess(self):
        self.docker.pin['sha256'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'changed'): self.docker.inventory(OWNER)
        self.assertFalse(self.calls)
        self.docker.pin = self.pin.copy()
        self.docker.config.chmod(0o755)
        with self.assertRaisesRegex(ValueError, 'config changed'): self.docker.inventory(OWNER)
        self.docker.config.chmod(0o700)
        (self.docker.config / 'config.json').write_text('{}')
        with self.assertRaisesRegex(ValueError, 'config changed'): self.docker.inventory(OWNER)
        self.assertFalse(self.calls)


if __name__ == '__main__': unittest.main()
