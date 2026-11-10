#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Fake-file and wrapper tests; these do not test real kernel cgroup semantics."""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import stage0_cgroup
import stage0_process

HERE = Path(__file__).resolve().parent


class CgroupTests(unittest.TestCase):
    def test_wrapper_joins_before_exec_and_closes_fd(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); procs = root / 'procs'; child = root / 'child'
            fd = os.open(procs, os.O_WRONLY | os.O_CREAT, 0o600)
            try:
                script = 'import os,sys; open(sys.argv[1],"w").write(str(os.getpid())); '\
                         '\ntry: os.fstat(int(sys.argv[2])); raise RuntimeError("join fd leaked")\nexcept OSError: pass'
                out = root / 'log'
                with out.open('wb') as stream:
                    result = stage0_process.run_bounded(['/usr/bin/python3', '-I', str(HERE / 'stage0_cgroup_exec.py'),
                        str(fd), '--', '/usr/bin/python3', '-c', script, str(child), str(fd)],
                        cwd=root, env={}, stdout=stream, stderr=stream, cap_seconds=5, grace_seconds=1, pass_fds=(fd,))
                self.assertEqual(result.returncode, 0, out.read_text())
                self.assertEqual(procs.read_text(), child.read_text())
            finally:
                os.close(fd)

    def test_invalid_descriptor_list_is_rejected_before_spawn(self):
        for value in ([3], (1,), (3, 3), (True,)):
            with self.subTest(value=value), patch.object(subprocess, 'Popen') as popen:
                with self.assertRaises(ValueError):
                    stage0_process.run_bounded(['/tool'], cwd='/', env={}, stdout=None, stderr=None,
                                               cap_seconds=2, grace_seconds=1, pass_fds=value)
                popen.assert_not_called()

    def test_group_kill_and_populated_state_on_fake_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            parent = Path(tmp); child = parent / ('stage0-' + 'a' * 32); child.mkdir()
            (child / 'cgroup.events').write_text('populated 1\nfrozen 0\n')
            (child / 'cgroup.kill').write_text('')
            (child / 'cgroup.procs').write_text('')
            parent_fd = os.open(parent, os.O_RDONLY | os.O_DIRECTORY)
            child_fd = os.open(child, os.O_RDONLY | os.O_DIRECTORY)
            join_fd = os.open(child / 'cgroup.procs', os.O_WRONLY)
            group = stage0_cgroup.Group(parent_fd, child_fd, join_fd, child.name)
            self.assertTrue(group.populated())
            with self.assertRaisesRegex(ValueError, 'populated'): group.close_if_empty()
            group.kill()
            self.assertEqual((child / 'cgroup.kill').read_text(), '1')
            (child / 'cgroup.events').write_text('populated 0\nfrozen 0\n')
            with patch.object(stage0_cgroup.os, 'rmdir') as rmdir:
                group.close_if_empty()
            rmdir.assert_called_once_with(child.name, dir_fd=parent_fd)
            self.assertTrue(group.closed)

    def test_create_checks_kernel_control_files_before_handoff_using_fake_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); real_mkdir = os.mkdir
            def mkdir_with_fake_controls(name, mode, *, dir_fd):
                real_mkdir(name, mode, dir_fd=dir_fd)
                child = root / name
                (child / 'cgroup.procs').write_text('')
                (child / 'cgroup.kill').write_text('')
                (child / 'cgroup.events').write_text('populated 0\n')
            with patch.object(stage0_cgroup, 'require_cgroup2', return_value=root), \
                 patch.object(stage0_cgroup.os, 'mkdir', side_effect=mkdir_with_fake_controls):
                group = stage0_cgroup.create(root)
            try:
                self.assertRegex(group.name, r'^stage0-[a-f0-9]{32}$')
                self.assertFalse(group.populated())
                os.fstat(group.join_fd)
            finally:
                os.close(group.join_fd); os.close(group.group_fd); os.close(group.parent_fd)

    def test_replaced_group_path_is_never_removed(self):
        with tempfile.TemporaryDirectory() as tmp:
            parent = Path(tmp); child = parent / ('stage0-' + 'a' * 32); child.mkdir()
            (child / 'cgroup.events').write_text('populated 0\n')
            (child / 'cgroup.procs').write_text('')
            pfd = os.open(parent, os.O_RDONLY | os.O_DIRECTORY)
            gfd = os.open(child, os.O_RDONLY | os.O_DIRECTORY)
            jfd = os.open(child / 'cgroup.procs', os.O_WRONLY)
            group = stage0_cgroup.Group(pfd, gfd, jfd, child.name)
            try:
                child.rename(parent / 'moved')
                child.mkdir()
                with self.assertRaisesRegex(ValueError, 'path changed'): group.close_if_empty()
                self.assertTrue(child.is_dir())
            finally:
                os.close(jfd); os.close(gfd); os.close(pfd)

    def test_non_cgroup_mount_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaisesRegex(ValueError, 'cgroup v2'):
                stage0_cgroup.require_cgroup2(tmp)


if __name__ == '__main__': unittest.main()
