<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->
# P1/P2 Stage-0 performance smoke — proposal

**Status: design only; not implemented or approved.** Stage-0 smoke is not
certification evidence. Full certification must be run separately with the
unchanged full grid, durations, thresholds and validation.

The current full-grid runners perform a 30-second baseline warmup and a
180-second baseline measurement before candidate work. Short time caps can
therefore expire before the first candidate request. Externally terminating a
runner can also prevent its provisional record from being written; a fallback
infrastructure FAIL is not runner or writer coverage.

## Candidate design

Add an explicitly smoke-origin-bound and expectations-bound short collection
profile to the existing P1/P2 production runners. Run one baseline and one
candidate W1/c1 point through the existing `run_one` path with reviewed, bounded
point durations and no warmup, then stop the grid with a reserved exit. Require
an actually completed candidate request, candidate binary hash, server and
metric provenance, and completed point evidence. A missing or failed point is
an unexpected failure, not the reserved incomplete-grid class.

The production writer must always emit a structured FAIL for this smoke
profile, even if comparisons appear favourable. Only an independently reviewed
binding among terminal class, record, exit and evidence may classify the result
as `INCOMPLETE_GRID_FAIL`; an arbitrary timeout or matching error text must not.
The production intake guard must reject the marked smoke record. Cleanup must
independently verify process and labelled-container cleanup, including the
exporter and model server when applicable.

## Review and implementation questions

- Freeze the duration, stop parameters and minimum completed-request evidence
  in independently reviewed expectations.
- Define exact P1 telemetry and P2 model/image identity checks.
- Bind smoke mode to both the approved plan and the marked output root so it
  cannot be used on an unmarked certification output.
- Verify that interruption and cleanup cannot transform a crash into the
  reserved exit. Test changed manifests, missing evidence, writer fallback,
  unexpected PASS and failed cleanup.
- Review feasible case caps after including server startup, collection and
  cleanup. BC-PERF-003 uses a separate runner and is not covered by this design;
  its first baseline cell also requests a 30-second warmup and 180-second
  measurement before the candidate cell begins.
