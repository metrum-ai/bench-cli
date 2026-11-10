# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""KVC registration stub. Replace only this module when the runner is ready."""

AVAILABLE = False


def freeze(args):
    """Freeze lane-specific inputs, write args.out, and return its manifest."""
    raise ValueError('KVC: runner not yet available')


def main(argv):
    """Run with ['--case', ID, ...lane arguments]; return a process exit code."""
    raise ValueError('KVC: runner not yet available')


def preflight(manifest):
    """Validate frozen lane-specific identity, inputs and execution requirements."""
    raise ValueError('KVC: runner not yet available')
