<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Historical schema fixtures

- `v171-non-reasoning.json` is the fixed-input v1.7.1 golden.
- `v171-released-*` and `v172-*` request/summary, strategic JSON/CSV and
  telemetry fixtures were captured from their respective released source
  versions against a CPU-only scripted response with one reasoning delta, one
  visible answer and reported usage. The v4 request headline is visible-answer
  TTFT; the separately recorded first-any clock is earlier. Timing values are
  only recorded fixture evidence, not performance measurements.

Machine-specific identity and paths, credentials, configuration and run IDs
were removed or replaced with neutral placeholders. One unknown optional field
was deliberately added to the telemetry fixture to exercise reader tolerance.
