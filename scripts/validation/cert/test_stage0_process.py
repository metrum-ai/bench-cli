#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Offline controls for bounded Stage-0 launcher processes."""
import io
import math
import signal
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import MagicMock, call, patch

import stage0_process


class Stage0ProcessTests(unittest.TestCase):
    def run_fake(self, waits, cap=10, grace=2):
        process = MagicMock(pid=9876)
        process.wait.side_effect = waits
        with patch.object(stage0_process.subprocess, 'Popen', return_value=process) as start, \
                patch.object(stage0_process.os, 'killpg') as kill:
            result = stage0_process.run_bounded(['/tool', '--case'], cwd='/', env={'PATH': '/bin'},
                                                stdout=io.StringIO(), stderr=io.StringIO(),
                                                cap_seconds=cap, grace_seconds=grace)
        self.assertTrue(start.call_args.kwargs['start_new_session'])
        self.assertEqual(start.call_args.kwargs['env'], {'PATH': '/bin'})
        return result, kill

    def test_normal_exit_no_signals(self):
        result, kill = self.run_fake([0])
        self.assertEqual(result, stage0_process.ProcessOutcome(0, False, False))
        kill.assert_not_called()

    def test_timeout_term_and_graceful_exit(self):
        result, kill = self.run_fake([subprocess.TimeoutExpired('job', 8), 143])
        self.assertEqual(result, stage0_process.ProcessOutcome(143, True, False))
        kill.assert_called_once_with(9876, signal.SIGTERM)

    def test_timeout_term_then_kill(self):
        result, kill = self.run_fake([subprocess.TimeoutExpired('job', 8),
                                      subprocess.TimeoutExpired('job', 2), -signal.SIGKILL])
        self.assertEqual(result, stage0_process.ProcessOutcome(-signal.SIGKILL, True, True))
        self.assertEqual(kill.call_args_list, [call(9876, signal.SIGTERM), call(9876, signal.SIGKILL)])

    def test_parent_exception_terminates_process_group(self):
        process = MagicMock(pid=9876)
        process.wait.side_effect = [KeyboardInterrupt(), -signal.SIGTERM]
        with patch.object(stage0_process.subprocess, 'Popen', return_value=process), \
                patch.object(stage0_process.os, 'killpg') as kill:
            with self.assertRaises(KeyboardInterrupt):
                stage0_process.run_bounded(['/tool'], cwd='/', env={}, stdout=None, stderr=None,
                                           cap_seconds=10, grace_seconds=2)
        kill.assert_called_once_with(9876, signal.SIGTERM)

    def test_invalid_inputs_rejected_before_launch(self):
        for cap, grace in [(0, 1), (2, 2), (math.nan, 1), (math.inf, 1), (True, 1)]:
            with self.subTest(cap=cap, grace=grace), patch.object(stage0_process.subprocess, 'Popen') as start:
                with self.assertRaises(ValueError):
                    stage0_process.run_bounded(['/tool'], cwd='/', env={}, stdout=None, stderr=None,
                                               cap_seconds=cap, grace_seconds=grace)
                start.assert_not_called()

    def test_real_child_receives_term(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / 'stdout'
            with out.open('wb') as stream:
                result = stage0_process.run_bounded(['/usr/bin/python3', '-c',
                    'import time; time.sleep(10)'], cwd=tmp, env={}, stdout=stream, stderr=stream,
                    cap_seconds=0.4, grace_seconds=0.2)
            self.assertTrue(result.timed_out)
            self.assertEqual(result.returncode, -signal.SIGTERM)
            self.assertFalse(result.escalated)


if __name__ == '__main__':
    unittest.main()
