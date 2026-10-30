<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Prompt selection design

Metrum AI: group candidates by their joint input/output lengths. Enumerate
bounded bucket compositions across the allowed count range when their total
fits the work limit, including capacities implied by the per-row repeat limit.
Evaluate both statistics for each composition and draw rows deterministically
from the winning buckets, preferring unique rows. Exhaustive completion with
no feasible composition proves infeasibility; budget exhaustion does not.
For larger spaces, construct mixed-median candidates from bucket order
statistics within the same work budget, then fall back to local search.
Preserve the existing homogeneous fast path. Include the best composition and
search outcome in failure reports, and cross-check small inputs against brute
force, including count slack and repeats.
