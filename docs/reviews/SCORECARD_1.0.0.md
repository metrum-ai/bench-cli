<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

> **Historical review of 1.0.0.** Superseded by the v1.1.2 state; policy docs
> and `metrum-ai/prompt-library` are live. See CHANGELOG.

# Metrum AI Bench CLI 1.0.0: scorecard

| Field | Value |
|---|---|
| Assessed line | v1.0.0 re-cut after history sanitization, shared chat stream extraction, and strategic streaming TTFT |
| Prior assessed commit | `07073908` (release-candidate line; mistagged `v1.0.0` later retracted) |
| Baseline | `858bb757` (0.1.82), scored 2026-09-15 in `QUALITY_ASSESSMENT_REPORT.md` |
| Evidence | `QUALITY_ASSESSMENT_1.0.0.md` plus post-1.0.0 remediation in this tree |
| Scale | 1 = a persona is actively misled; 3 = usable with caveats the output itself discloses; 5 = the output alone suffices to decide and to catch a misleading result |

## Verdict

**GO for public use of the binaries and the JSONL output.**

**GO to cite the release page and binary hashes** once the rewritten `v1.0.0`
tag and signed assets are live.

**Not yet GO for citing `docs/SMOKE_RESULTS.md` as evidence** until that table
is regenerated only from `summary.v3` / `campaign.sh report` with a check that
printed TTFT columns equal `ttft_s` (N-01 was fixed in tooling; republish the
matrix before marketing cites those rows).

Counsel-pending trademark / naming / claims / publication-policy drafts were
removed from the public tree pending sign-off. External docs no longer claim
those files are the signed policy of record. Marketing may describe
`--sut` / `--require-sut` as the publication manifest requirement; it must not
cite archived draft policy filenames as live counsel-approved text.

## Summary

| Axis | 0.1.82 | 1.0.0 | Change | Holds it below 5 |
|---|---|---|---|---|
| R. Real-world fidelity | 3 | **4** | +1 | no `connect_s`; strategic multi-turn still one HTTP turn per message prefix |
| H. Honesty and anti-gaming | 1 | **4** | +3 | published smoke matrix still needs a generator-only republish; open-loop effective cap unrecorded |
| M. Measurement correctness | 2 | **4** | +2 | wall-clock window breaks under a clock step; RST -> `other`; partial-bin normalization |
| I. Differentiated value | 3 | **4** | +1 | knee has no confidence or threshold |
| O. Operator experience | 2 | **4** | +2 | imagegen dual schema (N-04) |
| E. Engineering quality | 2 | **4** | +2 | four modality request loops remain; no monotonic-window pin test |
| P. Public readiness | 3 | **4** | +1 | GitHub Support cache purge and credential rotation follow-ups remain |
| **Mean** | **2.3** | **4.0** | **+1.7** | |

## Axis detail

### R. Real-world fidelity: 4

Earned: `first_byte_s` on every chat record; `--ca-cert` / `--insecure`;
open-loop queue delay and coordinated-omission latency; typed 429/503 and
refused connections. Strategic chat now has opt-in `--streaming` with
per-turn TTFT via the shared chat stream consumer.

Holds it back: `connect_s` is still deferred; multi-turn remains one HTTP
request per message prefix rather than a live conversation socket.

### H. Honesty and anti-gaming: 4

Earned: rich `summary.v3.config`; nullable token throughput;
`usage_missing_count`; synthesized-SSE TTFT caveat documented in
`docs/LIMITATIONS.md` and the customer docs Known limitations page. Tooling
no longer prints `first_byte_s` as TTFT (N-01 closed in generator).

Holds it back: the checked-in smoke matrix still needs a clean republish
before it is citable; open-loop effective concurrency cap is unrecorded.

### M. Measurement correctness: 4

Unchanged residuals: wall-clock-derived window under a clock step; some RST
paths map to `other`; trailing partial throughput bin width.

### I. Differentiated value: 4

Earned previously unique metrics plus strategic streaming TTFT and TTFT SLO
enforcement when `--streaming` is set. Knee estimator still lacks a score or
confidence threshold.

### O. Operator experience: 4

Earned: live JSONL, SIGINT/`partial: true`, README install path, VLM
preprocess failures emit request records (`emit_preprocess_failure`), shared
mid-stream `{"error":...}` handling for LLM and VLM.

Holds it back: imagegen still writes dual request schemas.

### E. Engineering quality: 4

Earned: shared `src/chat_stream.rs` for LLM, VLM, and strategic streaming;
focused e2e coverage for mid-stream API errors and strategic session TTFT.
The stale claim that VLM lacked the LLM in-stream error branch is closed.

Holds it back: four modality outer request loops remain on purpose (ASR
multipart, imagegen retries, VLM preprocess); no test pins the throughput
window to a monotonic source.

### P. Public readiness: 4

Earned: Dependabot config present; README install; crate excludes internal
docs; signed releases with SBOM; draft counsel files removed from the public
tree; leak path purged from published refs as part of the v1.0.0 re-cut.

Holds it back: GitHub Support purge of cached objects; org audit follow-up
for the 2026-09-16 visibility change; credential rotation confirmation.

## What marketing may cite

Allowed after the rewritten `v1.0.0` assets are live:

- Product name **Metrum AI Bench CLI**
- Apache-2.0 license and GitHub Release binary hashes / Sigstore bundles
- Capability claims grounded in README / CLI help for shipped flags
- `--sut` / `--require-sut` as the publication manifest requirement

Do not cite until republished or signed off:

- Numbers from the current `docs/SMOKE_RESULTS.md` table
- Archived counsel drafts (trademarks, naming, claims ledger, publication
  policy) as live signed policy text

## Open items by priority

1. GitHub Support sensitive-data purge + confirm credential rotation
2. Org audit log: who flipped the repository public on 2026-09-16
3. Republish `docs/SMOKE_RESULTS.md` from `campaign.sh report` only
4. N-08 stretch: knee threshold; `connect_s`
5. Governance leftovers (#7, #15)

Closed in this remediation line: shared chat stream / VLM error parity;
strategic streaming TTFT; synthesized-SSE limitation docs; public draft
policy archival; history purge of `docs/OSS_READINESS_ASSESSMENT.md` from
published refs.
