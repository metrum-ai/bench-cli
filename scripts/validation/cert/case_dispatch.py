#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Explicit case-to-runner dispatch; unknown selected cases fail closed."""
import os
from pathlib import Path

RUNNERS = {
    "g1_clock_runner.py": {"BC-GPU-001"},
    "g1_truncation_runner.py": {"BC-GPU-002"},
    "g1_variants_runner.py": {"BC-GPU-003"},
    "g2_terminal_runner.py": {"BC-GPU-004"},
    "g2_mixed_runner.py": {"BC-GPU-005"},
    "g2_usage_runner.py": {"BC-GPU-006"},
    "g2_fault_runner.py": {"BC-GPU-007"},
    "g3_runner.py": {"BC-GPU-008", "BC-GPU-009"},
    "g4_edge_runner.py": {"BC-GPU-010"},
    "g4_rates_runner.py": {"BC-GPU-012"},
    "g4_modality_runner.py": {"BC-GPU-011"},
    "g5_strategic_runner.py": {"BC-GPU-013"},
    "g5_modes_runner.py": {"BC-GPU-014"},
    "g5_endpoints_runner.py": {"BC-GPU-015"},
    "g6_counter_runner.py": {"BC-COMP-003"},
    "g6_schema_runner.py": {"BC-GPU-016"},
    "g7_runner.py": {"BC-PERF-003"},
    "r_runner.py": {"BC-REASON-001", "BC-REASON-002", "BC-REASON-003", "BC-REASON-004", "BC-HW-001"},
    "l1_runner.py": {"BC-LIVE-001"},
    "live_modality_runner.py": {"BC-LIVE-002", "BC-LIVE-003", "BC-LIVE-004"},
    "d_default_runner.py": {"BC-REASON-006"},
    "comp_reference_runner.py": {"BC-COMP-002"},
    "v_runner.py": {"BC-REASON-005"},
    "w_runner.py": {"BC-INTEG-001"},
    "perf_all_smi.py": {"BC-PERF-001", "BC-TELEM-001"},
    "perf_p1_runner.py": {"BC-PERF-001", "BC-TELEM-001"},
    "perf_p2_runner.py": {"BC-PERF-002"},
}


def selected(case):
    target = os.environ.get("CERT_CASE_ID")
    return target is None or target == case


def run_selected(argv, case):
    if not case:
        return True
    if len(argv) >= 5 and argv[:3] == ["cargo", "--locked", "xtask"] and argv[3] == "serve":
        return True  # modality server setup; never a sibling measurement workload
    if len(argv) < 2 or Path(argv[1]).name not in RUNNERS:
        raise ValueError("selected-case execution requires a known case runner")
    return case in RUNNERS[Path(argv[1]).name]


if __name__ == "__main__":
    import sys
    try:
        raise SystemExit(0 if run_selected(sys.argv[2:], sys.argv[1]) else 10)
    except ValueError as exc:
        raise SystemExit(str(exc)) from exc
