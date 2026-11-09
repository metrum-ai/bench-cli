#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Bound a Stage-0 launcher process; this alone is not a smoke controller.

The launched process may create new sessions or containers. Process-group
signalling is therefore not cleanup proof: the controller must independently
verify all owned processes, containers and provider resources.
"""
import math
import os
import signal
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class ProcessOutcome:
    returncode: int
    timed_out: bool
    escalated: bool


def run_bounded(argv, *, cwd, env, stdout, stderr, cap_seconds, grace_seconds):
    """Run argv until the run deadline, then TERM and, after grace, KILL.

    The hard wall cap includes cleanup grace. The caller must validate all
    arguments and perform independent resource reconciliation even on success.
    """
    if (not isinstance(argv, list) or not argv or not all(isinstance(x, str) and x for x in argv)
            or type(cap_seconds) not in (int, float) or type(grace_seconds) not in (int, float)
            or not math.isfinite(cap_seconds) or not math.isfinite(grace_seconds)
            or not 0 < grace_seconds < cap_seconds):
        raise ValueError('invalid bounded process configuration')
    if not isinstance(env, dict) or not all(isinstance(k, str) and isinstance(v, str) for k, v in env.items()):
        raise ValueError('bounded process environment must be explicit strings')
    started = time.monotonic()
    process = subprocess.Popen(argv, cwd=Path(cwd), env=env, stdout=stdout, stderr=stderr, start_new_session=True)
    def remaining(deadline):
        return max(0.0, deadline - time.monotonic())
    term_at, kill_at = started + cap_seconds - grace_seconds, started + cap_seconds
    def send(sig):
        try:
            os.killpg(process.pid, sig)
        except ProcessLookupError:
            pass  # Process completed between the timeout and the signal.
    try:
        try:
            return ProcessOutcome(process.wait(timeout=remaining(term_at)), False, False)
        except subprocess.TimeoutExpired:
            send(signal.SIGTERM)
            try:
                return ProcessOutcome(process.wait(timeout=remaining(kill_at)), True, False)
            except subprocess.TimeoutExpired:
                send(signal.SIGKILL)
                # Do not hang indefinitely on an uninterruptible process. The
                # independent provider backstop remains responsible for it.
                return ProcessOutcome(process.wait(timeout=1), True, True)
    except BaseException:
        # Parent interruption must not leave the direct process group running.
        # Never grant a new grace period beyond the original hard deadline.
        send(signal.SIGTERM)
        try:
            process.wait(timeout=remaining(kill_at))
        except subprocess.TimeoutExpired:
            send(signal.SIGKILL)
            process.wait(timeout=1)
        raise
