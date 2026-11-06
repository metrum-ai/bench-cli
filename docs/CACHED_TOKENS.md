<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Request cached-token evidence and TTFT cohorts

## Evidence contract (`cached-token-evidence.v1`)

Chat request records retain an optional `cache_evidence` extension. Core record
schema readers remain compatible: absent extension means unknown, never zero.
Strategic CSV stores the same object as a JSON-encoded cell; structured JSON uses
the object directly. Old CSV inputs without the column remain readable.

The only accepted counter location in this version is
`usage.prompt_tokens_details.cached_tokens`, with source identifier
`openai-chat-usage.v1:/prompt_tokens_details/cached_tokens` and units `tokens`.
This describes a response protocol, not verified participation by an engine or
physical tier. No flat or alternate API field is silently treated as equivalent.

| Evidence field | Meaning |
| --- | --- |
| `cached_tokens` | Most recent well-formed nonnegative integer report, or null if none |
| `prompt_tokens` | Most recent well-formed reported prompt-token denominator, or null |
| `cached_status`, `prompt_status` | Missing, null, reported or invalid observation state |
| `source` | Accepted versioned pointer when the cached field was observed |
| `units` | Expected counter unit, tokens; never bytes, blocks or utilization |
| `null_report_seen` | A cached-token leaf explicitly reported null |
| `invalid_report_seen` | A malformed cached/prompt count or cached-details object was seen |
| `conflicting_reports` | Different well-formed numeric reports arrived for the same per-request count |

Only JSON integer counts are accepted. Negative/fractional/string counts are
invalid, not rounded or saturated. A well-formed count greater than the reported
prompt total is retained for diagnosis but cannot establish a hit cohort. Changed
numeric reports also stay contradictory. Missing or null is not a competing
numeric report: it does not overwrite a previously known count or fabricate zero.
An invalid report latches an uncertainty flag even if a later chunk looks valid.

Unary and SSE paths retain the same evidence. SSE usage-only chunks after output
are consumed, including usage preceding a stream error. An HTTP error body is not
promoted into a successful usage report. Retained evidence on a failed request
is diagnostic only; failures are excluded from cohort timing distributions.

## Inferred cohort contract (`cached-token-cohorts.v1`)

Cohorts compare a consistent **server report**, not client text overlap:

- **cold:** reported cached count is zero and the prompt denominator is positive;
- **partial:** cached count is positive and smaller than the prompt denominator;
- **full:** both positive counts are equal;
- **unknown:** absent/null/invalid/conflicting evidence, unsupported source/version
  or units, missing/zero denominator, or cached count greater than prompt count.

These are inferred report cohorts, not direct observations of local residency,
external retrieval or saved prefill work. Keep logical token-prefix overlap and
physical-tier metrics separate. Never assign an aggregate external hit counter
to an individual request.

Measured cohort counts include every measured request exactly once. Errors and
invalid responses are counted as Unknown while retaining raw evidence in their
records. Warmup and drain have explicit exclusion counts and do not silently enter
measured timing distributions. Grouping retains original source turn numbering
and prefix group; absent grouping information stays absent rather than being
invented from prompt text or aggregate hit rates.

Within each group, timing eligibility and missing/invalid timing counts are
explicit. Headline TTFT distributions are separated by provenance (`stream`,
`first_token_fallback`, `first_byte_approx`, `unprovenanced`). Visible-answer TTFT
is separate. Reasoning-first output must not be relabeled visible output, and an
HTTP-first-byte approximation must not be relabeled a streamed token. Empty
populations have null statistics, not a fabricated zero or speedup.

None of these client timings is independently measured server prefill duration.
Server prefill/queue histograms need their own metric semantics and phase windows;
subtracting client TTFT values does not establish prefill savings.

## Runtime output

Strategic chat output adds an independently versioned `cache_cohorts` extension
(`cached-token-cohort-report.v1`). `all_stages` is a descriptive union;
`stages[]` retains each numeric `load` and its own cohort summary. Prefer the
stage-specific populations for comparisons: different offered loads must not be
silently treated as one performance condition. Non-chat output has no applicable
chat-counter report.

The LLM chat runner prints a bounded cohort overview alongside its existing
console summary. Per-request evidence remains in JSONL. Library adapters
`cache_report::request_cohorts` and `cache_report::strategic_cohorts` return the
serializable model for study exporters without changing legacy summary record
schemas. Console truncation is labelled and never averages group percentiles
into an invented global percentile.

Runtime timing eligibility retains the runner's existing success/validity
policy, including its explicit strict/non-strict handling of reasoning-only
truncation. A non-strict reasoning-only response does not acquire a visible-token
timestamp merely because it is retained by the legacy success policy.

## Validation boundary

Offline fixtures cover missing/null/zero, partial/full, inconsistent counts,
malformed counts and version/unit rejection. Mock HTTP tests cover unary/SSE,
usage-only final chunks, usage before stream failure, warmup exclusion and cohort
population reconciliation across both chat runners. These are client protocol
and reporting tests, not live engine/offload or performance validation.
