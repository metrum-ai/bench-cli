<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Metrum AI Bench 1.0.0: scorecard

| Field | Value |
|---|---|
| Assessed commit | `07073908df7fc587380b4f0d0a8d36b7cdacbf8b` (release-candidate line; mistagged `v1.0.0` later retracted) |
| Baseline | `858bb757` (0.1.82), scored 2026-09-15 in `QUALITY_ASSESSMENT_REPORT.md` |
| Evidence | `QUALITY_ASSESSMENT_1.0.0.md` (same host, same scenario battery, release binaries) |
| Scale | 1 = a persona is actively misled; 3 = usable with caveats the output itself discloses; 5 = the output alone suffices to decide and to catch a misleading result |

## Verdict

**GO for public use of the binaries and the JSONL output.** Not yet GO for
citing `docs/SMOKE_RESULTS.md` or the release page as evidence: the published
TTFT columns are first-byte time (N-01, #56), and the prior review's
infrastructure disclosures remain reachable at tags v0.1.80 to v0.1.82 (O-01,
#6).

## Summary

| Axis | 0.1.82 | 1.0.0 | Change | Holds it below 5 |
|---|---|---|---|---|
| R. Real-world fidelity | 3 | **4** | +1 | multi-turn is non-streaming with per-message turns; no `connect_s` |
| H. Honesty and anti-gaming | 1 | **3** | +2 | repository's own published TTFT is mislabeled (N-01); effective cap unrecorded (F-09) |
| M. Measurement correctness | 2 | **4** | +2 | wall-clock window breaks under a clock step (N-02); RST -> `other`; partial-bin normalization |
| I. Differentiated value | 3 | **4** | +1 | knee has no confidence or threshold; strategic path has no TTFT |
| O. Operator experience | 2 | **4** | +2 | VLM drops failed preprocessing silently (N-03); imagegen dual schema (N-04); README has no install path |
| E. Engineering quality | 2 | **3** | +1 | four request loops of 1,000-1,400 lines; VLM lacks the LLM error branch; no test pins the window to a monotonic source |
| P. Public readiness | 3 | **4** | +1 | leaked doc at old tags; no `dependabot.yml`; stray root files |
| **Mean** | **2.3** | **3.7** | **+1.4** | |

## Axis detail

### R. Real-world fidelity: 4 (was 3)

Earned: `first_byte_s` on every record separates gateway time from prefill
(0.4265 s first byte through a 300 ms header-holding proxy against 0.121 s
direct); `--ca-cert` accepts a private CA and `--insecure` is stamped into
config; open-loop queue delay and coordinated-omission latency are correct
under overload (max 4.115 s at 5x the server's capacity); 429 and 503 storms,
buffering proxies, and connection-refused replicas all run and are counted
with typed classes.

Holds it back: the strategic runner is non-streaming, so multi-turn chat has
no TTFT and every message prefix counts as a turn; `connect_s` is deferred,
so TLS handshake time cannot be split from first byte.

To reach 5: streaming multi-turn with per-turn TTFT; `connect_s` behind a
feature flag.

### H. Honesty and anti-gaming: 3 (was 1)

Earned: `summary.v3.config` records seed, warmup, arrival, cap, `ignore_eos`,
`min_tokens`, the effective system prompt (null when disabled), the
unique-prompt nonce template, SLOs, `--insecure`, and `--ca-cert`; one summary
per file; `usage_missing_count` and a nullable token throughput; per-record
`run_id`.

Holds it back: the repository's own `docs/SMOKE_RESULTS.md` prints
`first_byte_s` under the heading TTFT in all twelve GPU rows (0.041 s printed
where the raw summary says 1.474 s), and the table was assembled outside the
tracked generator. The cap in force in open loop is not recorded.

To reach 5: regenerate the published tables from `summary.v3` through
`campaign.sh report` only, with a validation step that recomputes each
printed percentile from the raw records; stamp `effective_max_concurrency`.

### M. Measurement correctness: 4 (was 2)

Earned: window recomputes exactly in all seven re-run scenarios (ratio
1.000; reference 7.918 req/s, 3000-request run 246.9 req/s, overload 7.86
req/s where 0.1.82 reported 39.9); closed-loop queue delay is zero; pooled,
per-endpoint, and CO latency agree; non-streaming TTFT is null; typed errors
with retained failure latency; multi-line SSE parsed; p90 and p95 flags.

Holds it back: the window now derives from wall-clock `started_at`, so a
+3600 s step mid-run yields a 3602 s window and 0.004 req/s while every
interval metric stays correct; a TCP reset after the request is `other`; the
trailing partial throughput bin is divided by the full bin width.

To reach 5: monotonic `send_offset_s` on records with the window derived from
it, plus a clock-step test.

### I. Differentiated value: 4 (was 3)

Earned: everything that was unique at 0.1.82 (per-request queue delay,
reasoning-vs-visible TTFT, `pooled_mixture` with full per-endpoint
distributions, seeded Poisson schedule, byte-identical image payload with
recorded size, correct knee) is now also correct in closed loop; SLO goodput
in the strategic runner (3.95 vs 39.49 req/s at load 8 with `e2e=150ms`).

Holds it back: the knee estimator reports a knee for any three-point curve and
exposes no score; the strategic path has no TTFT.

### O. Operator experience: 4 (was 2)

Earned: records on disk while issuing (629 lines at 2.5 s of a 12 s run);
SIGINT drains and writes `partial: true` in 1.0 s; SIGTERM handled; SIGKILL
leaves 752 parseable lines; second signal exits; `--fail-on-error` default
off; imagegen accepts base and full URLs and `--summary-json` is optional;
console rendered from the same summary as the file, with reliability flags.

Holds it back: VLM image-load and body-build failures leave no record and
cannot trigger `--fail-on-error`; imagegen writes v1 and v3 request lines;
README still lacks install instructions and a status statement.

### E. Engineering quality: 3 (was 2)

Earned: `runner.rs` owns stop flags, schedule semantics, window, and
completion timestamps; `http_client.rs` is the one client builder; dead
traits deleted; private estimators and legacy writers gone; e2e tests now
assert the window against records and flush during launch.

Holds it back: each modality binary still carries its own request loop and
stream handling (VLM is missing the in-stream `error` branch the LLM binary
has); no test covers the wall-clock/monotonic relationship that N-02
exposed; imagegen keeps a second append handle for v1 lines.

### P. Public readiness: 4 (was 3)

Earned: full `cargo deny check` and `MSRV 1.85` are required contexts;
govulncheck clean on go 1.26.6; every action SHA-pinned; secret scanning,
push protection, and Dependabot security updates enabled; issue forms and PR
template; formula test passes; `THIRD_PARTY_LICENSES` at 1.0.0; crate excludes
internal docs; signed releases with SBOM for rc.1 and 1.0.0.

Holds it back: the prior review with verbatim infrastructure details is still
reachable at three public tags; no `.github/dependabot.yml`; non-provider
secret patterns off; `README-METRUMBENCH-ASR.md` and `endpoints-4servers.yaml`
still at the root; single CODEOWNER.

## Gates at 1.0.0

| Gate | Result |
|---|---|
| build, fmt, clippy `-D warnings`, headers | pass |
| tests with `METRUM_BENCH_REQUIRE_DUMMY=1` | 155 passed, 0 failed, 0 skipped |
| `cargo deny check` (all) | pass |
| go vet, go test, govulncheck | pass, no vulnerabilities |
| gitleaks (31 commits) | no leaks |
| branch protection | 5 required contexts |
| independent recompute (7 runs) | window ratio 1.000; 0 numeric mismatches |

## Open items by priority

1. **#6 O-01** leaked review reachable at old tags; rotation unverified
2. N-08 stretch (post-public): knee threshold (#33), strategic streaming TTFT
3. #7, #15 governance leftovers (non-provider patterns; CLI/YAML credentials)

Closed after 1.0.0: **#56 N-01** (PR #61); measurement residuals **#57 #58 #26 #36 #60 #59 #31**
(and superseded **#27 #38**); packaging/docs **#10 #12 #13 #17** via OSS readiness PR.
Prompt library descoped to a separate Hugging Face dataset.

Closed at 1.0.0 as fixed: #8 #9 #11 #14 #16 #18 #19 #20 #21 #22 #23 #24 #25
#28 #29 #30 #34 #35 #37 #39 #40 #41; #32 superseded by #56.
