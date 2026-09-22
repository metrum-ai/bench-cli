<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Naming

Status: Approved September 21, 2026. Owner: CEO.

Canonical product and artifact names for Metrum AI Bench CLI. See also
[TRADEMARKS.md](../TRADEMARKS.md).

## Reason

Router and related surfaces drifted across three names. This document and CI
checks exist to keep formal, binary, crate, and commercial forms aligned.

## Approved and forbidden names

| Form | Approved | Forbidden (examples) |
|------|----------|----------------------|
| Formal | `Metrum AI Bench CLI` | `MetrumBench`, `Insights CLI`, `Bench by Metrum` |
| Binary | `metrum-ai-bench` | `metrumbench` as the primary product binary name in new docs |
| Crate | `metrum-ai-bench` | alternate crate names implying a different product |
| Commercial | `Metrum AI Bench Platform` | unofficial "Platform" variants |
| Transition | `Metrum AI Bench CLI, formerly Metrum Insights CLI` | presenting the former name as the current product name |

Forbidden list is non-exhaustive; prefer the approved table when in doubt.

## CI pointer

Naming enforcement lives in:

- `scripts/check_headers.sh` - `check_naming`
- `.naming-allow` - allowlist exceptions

CI failures for naming should be fixed by aligning text to this table or, when
justified, adding a documented allowlist entry.

## Transition form

Use the transition form **`Metrum AI Bench CLI, formerly Metrum Insights CLI`**
only where historical context is necessary.

**Never retrofit** published studies: leave historical titles, captions, and
citations as originally published; use the approved current names for new work.
