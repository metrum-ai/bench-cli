<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Metrum AI Bench 1.0.0: re-assessment

| Field | Value |
|---|---|
| HEAD commit | `07073908df7fc587380b4f0d0a8d36b7cdacbf8b` ("docs(smoke): record L40S matrix Shadeform campaign results (#55)") |
| `Cargo.toml` version | `1.0.0` at assessment time (tag `v1.0.0-rc.1`; mistagged `v1.0.0` later retracted) |
| Previous assessment | `858bb757` / 0.1.82, `docs/reviews/QUALITY_ASSESSMENT_REPORT.md` |
| Date | 2026-09-15 (UTC) |
| Host | same as the previous assessment (16 cores, 30 GiB, Linux 6.8, rustc 1.97.1, go 1.26.4) |
| Method | every scenario from the previous assessment re-run against the release binaries with the same helper servers (Appendix B of the previous report), plus a static verification pass of the CHANGELOG 1.0.0 claims |

## 1. Verdict

**GO for public use of the binaries and the JSONL output.** The three
CRITICAL findings that drove the NO-GO at 0.1.82 are fixed and verified by
running the release build: the throughput window now recomputes exactly from
the records in every scenario (ratio 1.000 in reference, multi-endpoint,
open-loop overload, and 3000-request runs), closed-loop records carry no
queue-delay term, the full workload configuration is stamped into
`summary.v3`, records reach disk while the run is issuing, and SIGTERM,
SIGKILL, and a second Ctrl-C all leave a parseable prefix. The second legacy
summary is gone from LLM, VLM, and ASR output.

**Two items must be fixed before anyone is pointed at `docs/SMOKE_RESULTS.md`
or the GitHub Releases page as evidence:**

1. **N-01 (HIGH).** Every "TTFT p50 / TTFT p95" value in the twelve LLM and
   VLM rows of the published `docs/SMOKE_RESULTS.md` is `first_byte_s`
   (response headers received), not `ttft_s`. For `Qwen3.8-27B-FP8` at
   concurrency 128 the document prints 0.041 s; the tool's own `summary.v3`
   in the raw file says `ttft_s.p50 = 1.474 s` and the per-request records
   range from 0.68 s to 37.0 s. The underestimate is 8x to 36x per row. All
   other columns (n, errors, latency p50/p95, window, rps) recompute exactly.
2. **O-01 (still open in history).** The prior internal review containing
   verbatim infrastructure details was removed from `main` (#44), but it is
   still reachable at the public tags `v0.1.80`, `v0.1.81`, and `v0.1.82`.
   Whether the disclosed password was rotated cannot be verified from the
   repository.

## 2. Scorecard (0.1.82 -> 1.0.0)

| Axis | Before | Now | Justification |
|---|---|---|---|
| R. Real-world fidelity | 3 | **4** | `first_byte_s` decomposes gateway time from prefill (0.4265 first byte through a 300 ms header-holding proxy; direct 0.121); `--ca-cert` works with a private CA; open-loop queue delay and corrected latency verified under overload; multi-turn remains non-streaming with per-message turns. |
| H. Honesty and anti-gaming | 1 | **3** | `config` stamps seed, warmup, arrival, cap, `ignore_eos`, `min_tokens`, extective system prompt, nonce template, SLOs, `--insecure`, `--ca-cert`; one summary per file; but the repository's own published results mislabel first-byte time as TTFT (N-01), which is exactly the class of error the axis measures. |
| M. Measurement correctness | 2 | **4** | Window, queue delay, per-endpoint estimator, non-streaming TTFT, typed errors, failure latency, multi-line SSE, p90/p95 flags all verified; residual: wall-clock-derived window is sensitive to a clock step (N-02), connection reset still `other`, partial trailing throughput bin under-normalized. |
| I. Differentiated value | 3 | **4** | Everything that was differentiated is now also correct in closed loop; SLO goodput in the strategic runner; knee still found exactly on the known-saturation server. |
| O. Operator experience | 2 | **4** | Flush during run (629 lines at 2.5 s of a 12 s run), SIGINT/SIGTERM/second-signal behavior, `--fail-on-error` default off, imagegen accepts both URL forms, `--summary-json` optional; residual: VLM drops requests without a record on image-load failure (N-03), imagegen still writes v1 request lines (N-04). |
| E. Engineering quality | 2 | **3** | `runner.rs` owns the bookkeeping that drifted; dead traits deleted; private estimators gone; but each binary still carries 1,000-1,400 lines with its own request loop, VLM misses the in-stream error branch the LLM binary has, and no test pins the wall-clock/monotonic window relationship. |
| P. Public readiness | 3 | **4** | Full `cargo deny check`, govulncheck (go 1.26.6, clean), MSRV job, SHA-pinned actions, secret scanning and push protection enabled, templates, formula fixed, THIRD_PARTY at 1.0.0, crate excludes internal docs; residual: leaked doc reachable at old tags, no `dependabot.yml`, stray root files, README still has no install section. |

## 3. Gates at `0707390`

| Command | Exit | Result |
|---|---|---|
| `cargo build --release` | 0 | ok |
| `METRUM_BENCH_REQUIRE_DUMMY=1 cargo test --all-targets --all-features` | 0 | 84 lib + 71 integration/bin tests passed, 0 failed, 0 skipped (e2e_dummy now 6 tests incl. window and flush) |
| `cargo clippy --all-targets --all-features -- -D warnings` | 0 | ok |
| `cargo fmt --all -- --check` | 0 | ok |
| `cargo deny check` (all) | 0 | advisories ok (RUSTSEC-2024-0436 `paste` ignored with reason), bans ok, licenses ok, sources ok |
| `bash scripts/check_headers.sh` | 0 | ok |
| `go vet && go test ./...` | 0 | ok |
| `govulncheck ./...` (dummy-model-server, go 1.26.6) | 0 | No vulnerabilities found |
| `gitleaks git --redact .` (31 commits) | 0 | no leaks found |
| `gh api .../security_and_analysis` | | secret_scanning enabled, push_protection enabled, dependabot_security_updates enabled, non-provider patterns disabled |
| branch protection contexts | | fmt/clippy/test, coverage 80 %, gitleaks, `cargo deny check (all)`, `MSRV 1.85` |
| `cargo package --list` | | excludes `CLAUDE.md`, `docs/reviews/**`, CAMPAIGN, HISTORY_REWRITE; still ships `README-METRUMBENCH-ASR.md`, `endpoints-4servers.yaml` |
| Homebrew formula `test do` string vs `--help` | | "Benchmark OpenAI-compatible" matches (1) |

## 4. Finding-by-finding verdicts

Evidence is the re-run output in Appendix A; "static" means confirmed by
reading the current code (agent pass, line numbers at `0707390`).

| ID | Verdict | Evidence |
|---|---|---|
| F-01 window | **FIXED** | reference: window 2.021 s, 7.918 req/s (true 7.92); 3000-request run 12.151 s, 246.9 req/s; open-loop overload 5.089 s, 7.86 req/s (was 39.9); ratio tool/true = 1.000 in all seven recomputed runs |
| F-02 closed-loop queue delay | **FIXED** | `queue_delay_s = 0.0`, `scheduled_offset_s` absent in closed loop; pooled = per-endpoint = CO latency = 0.505 s; live campaign per-endpoint avg equals pooled in all 12 cells |
| F-03 config unrecorded | **FIXED** | `summary.v3.config` carries `run_id`, all `CommonBenchArgs`, `effective_system_prompt`, sanitized `body_template`, nonce template, modality map (VLM: detail/resize/reencode; ASR: normalizer/response_format/language; imagegen: size/n/response_format); `--system-prompt ""` yields `effective_system_prompt: null` |
| F-04 no flush; SIGTERM loss | **FIXED** | 629 lines on disk at t=2.5 s of a 12 s run; SIGKILL at 3 s leaves 752 parseable records; SIGINT drains and writes `partial: true` |
| F-05 two summaries | **FIXED** (LLM/VLM/ASR) | zero legacy objects in any LLM/VLM/ASR output; console rendered from `RunSummary` with type-7 and reliability flags. Imagegen still emits `imagegen.request.v1` lines next to `request.v3` (N-04) |
| F-06 error classes | **FIXED** (one residual) | 503 -> `http_status`, refused -> `connect`, timeout -> `timeout` (2.002 s latency retained), 429 -> `rate_limit`, mid-stream -> `api_error`; TCP RST after request -> `other` (N-05) |
| F-07 failure latency 0 | **FIXED** | slow-loris failures carry 2.0017-2.0020 s |
| F-08 non-stream TTFT | **FIXED** | `ttft_s: null`, `tpot_s.n = 0`, clock after body |
| F-09 open-loop headline | **PARTIAL** | `latency_s` is service latency, `coordinated_omission_latency_s` includes queue delay (max 4.115 s under overload), both documented; the effective cap (`--concurrency` when `--max-concurrency` is absent) is still not recorded and no "cap engaged" field exists |
| F-10 VLM | **PARTIAL** | warmup records kept (2 of 8, `phase: warmup`); `--system-prompt ""`, `--min-tokens 3`, `--ignore-eos`, `--extra-body-json` all reach the server (proxy capture); image bytes identical (70); **in-stream `{"error":...}` still ignored** (`vlm.rs:260-315` has no error branch) and image-load/body-build failures `continue` without writing a record (N-03) |
| F-11 usage missing | **FIXED** | `completion_tokens_per_second: null`, `usage_missing_count: 4`, `completion_tokens_source` absent |
| F-12 least-inflight | **FIXED** | dead replica gets 2 of 20 (was 18 of 20); ejection backoff 5 s |
| F-13 TLS / decomposition | **FIXED** | `--ca-cert ca.pem` with a CA-signed leaf: 4/4 success, stamped in config; `--insecure`: success, stamped; `first_byte_s` on every record. A self-signed leaf passed as `--ca-cert` is rejected (`CaUsedAsEndEntity`); documentation should say the file must be a CA certificate (N-09) |
| F-14 MLPerf | **PARTIAL** | first line of all three files: "UNOFFICIAL: This is NOT an audited or submitted MLPerf result..."; but `Result is : VALID (unofficial; see disclaimer)` still contains the exact substring a naive parser matches |
| F-15 published results | **SUPERSEDED by N-01** | old cells regenerated; new document's TTFT columns are wrong |
| F-16 strategic points | **PARTIAL** | `n`, `errors`, `p99_unreliable`, per-point `config`, `slo_thresholds_s`, `goodput_equals_throughput` present; `--slo e2e=150ms` gives goodput 3.95 vs throughput 39.49 at load 8 (correct); knee exact (4 and 40); knee still has no sensitivity threshold |
| F-17 / O-03 advisories | **FIXED** | `cargo deny check` all sections ok; `ntp` and `lru` removed |
| F-18 signals | **FIXED** | SIGTERM handled; second signal exits 130/143 (static, `runner.rs:44-80`) |
| F-19 completed_at | **FIXED** | `completed_at - started_at = 0.506 = latency_s` |
| F-20 throughput bins | **PARTIAL** | bins by actual send in closed loop and `throughput_bin_seconds` recorded; trailing partial bin divided by the full width (52.1 vs 247.9 rps) and a window shorter than one bin yields nonsense (1.0 rps for a 267 rps run) (N-06) |
| F-21 ASR RTFx | **FIXED** (note) | legacy `throughput.rtfx` gone; `rtfx_client` and separate `inference_seconds_server`/`_client` kept; `rtf` and `inference_time_s` still mix provenance (N-07) |
| F-22 imagegen | **PARTIAL** | full and base URLs both work; phase by sequence; decode/hash/write outside `latency_s`; `--summary-json` optional; config stamped; but v1 request lines remain and the artifact hash lives only on them (N-04) |
| F-23 multi-line SSE | **FIXED** | two `data:` lines split at a JSON member boundary: 6 tokens, TTFT 0.0209 s |
| F-24 p95 flag | **FIXED** | `p90_unreliable`/`p95_unreliable` on every distribution and on the console |
| F-25 polish | **FIXED** | console from `RunSummary`; dead traits deleted; nonce `[nonce-{run_id}-{seed}-{seq}]`; exit 0 unless `--fail-on-error` |
| O-01 leaked doc | **PARTIAL** | removed from `main` and from the gitleaks allowlist; present at tags v0.1.80-82; rotation not verifiable |
| O-02 GitHub security | **PARTIAL** | secret scanning, push protection, Dependabot security updates enabled; non-provider patterns still disabled |
| O-04 Go vulns | **FIXED** | go 1.26.6, govulncheck clean |
| O-05 supply chain | **PARTIAL** | all actions SHA-pinned with version comments; no `.github/dependabot.yml` |
| O-06 Homebrew | **FIXED** | formula 1.0.0, assertion matches |
| O-07 packaging | **PARTIAL** | exclude list extended; `README-METRUMBENCH-ASR.md`, `endpoints-4servers.yaml`, `CLAUDE.md` still at root (the latter excluded from the crate) |
| O-08 governance | **PARTIAL** | issue forms and PR template (community profile 100 %); CHANGELOG 1.0.0 entry; single CODEOWNER; README has no install/status section |
| O-09 dummy server | **FIXED** | `http.Server` with `ReadHeaderTimeout`, non-root `USER metrum`, `HEALTHCHECK` (static) |
| O-10 credentials/unwraps | **PARTIAL** | non-test unwrap/expect down to LLM 1, VLM 3, ASR 6, imagegen 3 (all guarded); API key still CLI/YAML only |
| O-11 MSRV | **FIXED** | `MSRV 1.85` job is a required check |
| O-12 local archive branch | **OPEN** | `private-archive-main` still present in the maintainer clone; remote has only `main` |

## 5. New findings at 1.0.0

### N-01 Published TTFT columns are first-byte time (HIGH, VERIFIED)

`docs/SMOKE_RESULTS.md` (commit `0707390`, campaign `matrix-20260915-195537`)
prints "TTFT p50 / TTFT p95" for six LLM and six VLM cells. Recomputing
type-7 percentiles from the raw `request.v3` records:

| Cell | Doc "TTFT" p50/p95 | `first_byte_s` p50/p95 | `ttft_s` p50/p95 | tool `summary.v3.ttft_s.p50` |
|---|---|---|---|---|
| llm Qwen c128 | 0.041 / 0.466 | 0.041 / 0.466 | 1.474 / 33.125 | 1.4739 |
| llm Qwen c32 | 0.039 / 0.184 | 0.039 / 0.184 | 0.622 / 7.308 | |
| llm Qwen c64 | 0.040 / 0.260 | 0.040 / 0.260 | 1.141 / 15.719 | |
| llm gemma c128 | 0.042 / 1.479 | 0.042 / 1.479 | 1.146 / 15.567 | |
| llm gemma c32 | 0.038 / 0.241 | 0.038 / 0.241 | 0.449 / 5.264 | |
| llm gemma c64 | 0.039 / 0.282 | 0.039 / 0.282 | 0.616 / 7.526 | |
| vlm Qwen c8 / c16 / c32 | 0.029/0.040, 0.031/0.097, 0.040/0.144 | identical | 0.321/0.539, 0.331/1.219, 0.647/2.565 | |
| vlm gemma c8 / c16 / c32 | 0.038/0.041, 0.040/0.102, 0.053/0.143 | identical | 0.241/0.282, 0.271/0.794, 0.369/0.601 | |

Every other column (n, err, lat p50/p95, window_s, rps) matches the raw
files to the printed precision. `live-results/.../aggregate.json` carries the
same first-byte values under the key `ttft_p50`, while `campaign.sh:350`
aggregates `rec["ttft_s"]`; the twelve-column tables and the `model`,
`err`, `window`, `rps`, `match` keys are not produced by any tracked script,
so the table was assembled outside the generator. A reader comparing these
rows to any other tool's TTFT would conclude the server answers in 40 ms at
128 concurrent 1,100-token prompts; the truth is 1.5 s median and up to
37 s. **Fix:** regenerate the tables from `summary.v3.ttft_s` (or add a
`first_byte` column with its own header), and make `campaign.sh report` the
only path that writes `SMOKE_RESULTS.md`.

### N-02 Window is derived from the wall clock (MEDIUM, VERIFIED)

`runner::window_seconds_from_records` uses `started_at` (wall) plus
`latency_s`. With the wall clock stepped by +3600 s one second into the
reference workload: interval metrics unchanged (latency 0.504-0.505 s) but
`window_seconds = 3602.02`, `requests_per_second = 0.004`. At 0.1.82 the
window was wrong for a different reason; the fix traded a monotonic source
for a wall-clock one. **Fix:** capture a monotonic send offset from the run
epoch inside the task, store it on the record (`send_offset_s`), and derive
the window from it; keep `started_at` for humans.

### N-03 VLM: in-stream error object ignored; failed preprocessing leaves no record (MEDIUM, static)

`vlm.rs:260-315` has no `parsed.get("error")` branch (the LLM binary has
one at `llm.rs:280-285`); an error event ends as `stream_truncated` or
`no_output_token`. Image-load failures, non-HTTP URLs under
`--server-side-download`, and body-build failures `continue` the request
loop at `vlm.rs:913-1066` without a record, so `attempted` undercounts and
`--fail-on-error` cannot trigger.

### N-04 Imagegen dual request schema (LOW, VERIFIED)

Each request writes an `imagegen.request.v1` line (with `latency_ms` and
the artifact SHA-256) and a `request.v3` line; the CHANGELOG says binaries
write only v3. The hash exists only on the v1 line; the v3 record has no
total-including-decode field.

### N-05 Residual error classification (LOW, VERIFIED)

TCP reset after the request is `other`; failure latency uses
`Utc::now() - started_at` (wall) while success latency is monotonic.

### N-06 Throughput bins edge cases (LOW, VERIFIED)

Trailing partial bin is divided by the full bin width (52.1 rps for the
last 2.15 s of a 247 rps run); a window shorter than one bin reports
`len / bin_seconds` (1.0 rps for a 267 rps multi-endpoint run).

### N-07 Non-token modalities print token throughput (LOW, VERIFIED)

ASR and imagegen summaries print `Completion tokens/sec: 0.000
(server_usage)`; ASR `rtf` and `inference_time_s` mix server and client
provenance while `inference_seconds_server`/`_client` are correct.

### N-08 Strategic residuals (LOW, VERIFIED)

`Result is : VALID` substring retained; knee has no threshold; stage
throughput is wall time over launch plus drain rather than a record-derived
window; strategic path remains non-streaming (no TTFT); multi-turn turns are
per message.

### N-09 `--ca-cert` documentation (LOW, VERIFIED)

A self-signed leaf passed as `--ca-cert` fails with `CaUsedAsEndEntity`;
only a certificate with CA basic constraints works. `--insecure` is the
supported path for self-signed leaves; the help text should say so.

## 6. What a persona can do at 1.0.0

- **Persona A** can size replicas from `summary.v3.requests_per_second`,
  `completion_tokens_per_second`, and `goodput` (all recompute exactly), read
  `coordinated_omission_latency_s` for user-experienced latency under
  overload, and separate gateway time from prefill with `first_byte_s`. Caveat:
  the cap in effect is not stamped (F-09) and a clock step during a run
  corrupts the window (N-02).
- **Persona B** can compare engines from the per-request records and from
  the summary; closed-loop per-endpoint latency is now the same estimator as
  pooled. Caveat: VLM error events and dropped requests (N-03).
- **Persona C** can read the effective configuration from the file, including
  the system prompt and the nonce policy. Caveat: the repository's own
  published table mislabels TTFT (N-01) and the old public tags still carry
  the prior review's disclosures (O-01).

## 7. Appendix A: raw output

```
reference (dummy 100ms + 20x20ms, 16 req, c=4): wall 2.02 s; summary.v3 window 2.021 rps 7.918 ctps 158.36 lat avg 0.5051 co avg 0.5051 per_ep 0.5051 ttft avg 0.1211 tpot 0.02021 itl n 304; rec0 completed-started 0.506 = latency 0.506, first_byte 0.1215, queue_delay 0.0, scheduled_offset absent, run_id present; legacy blocks 0; recompute ratio 1.000, 0 numeric mismatches (only p90/p95_unreliable flags absent from my script)
s5a open loop rate 40 cap 4 (500 ms server): window 5.089 rps 7.86 ctps 39.30 lat avg 0.5013 co max 4.115; recompute ratio 1.000
s5b cap 1000: window 1.478 rps 27.07 co max 0.5035
s5c warmup 4 + SLOs: attempted 36 succ 36 rps 26.16
s8a round-robin dead replica: pooled_mixture true, alive 10/10, dead 0/10, errors_by_type {connect: 10}
s8b least-inflight: alive 18/18, dead 2 (was 18 dead at 0.1.82)
s9a 3000 req c=8: log 348 KB / 629 lines at t=2.55 s; 1.40 MB / 2517 lines at t=10.2 s; final 3000 records, window 12.151, rps 246.9, co max 0.034; RSS 6.8 -> 14.5 MB
s9b SIGINT at 3 s: 752 lines before signal, 761 records after drain, partial true, exit 0 in 1.0 s
s9c SIGKILL at 3 s: 752 parseable records, no summary
s9d SIGINT open loop: exits in 0.50 s, 601 records, partial true
s3b role-only: 4 x no_output_token (latency 0.0003-0.0007 s retained)
s3c reasoning-then-content: first_reasoning 0.0514, ttft 0.0615
s3c2 reasoning-only: 4 x no_output_token, latency 0.102
s3e non-stream: ttft null, tpot n 0, lat 0.0506
s3f non-stream no usage: ctps null, usage_missing true, rps 39.19
no_usage_stream (0.1.82 battery, re-run at 1.0.0 in s3f/s3d): ctps null
s4a 503 non-stream: http_status x6 (latency 0.0002-0.0007)
s4b mid-stream error: api_error x6
s4c 429 storm: rate_limit x31, 1 success
s4c3 503 stream: http_status x6
s4d refused: connect x6
s4e RST: other x6
s4f slow-loris (--request-timeout 2): timeout x4, latency 2.0016-2.0020
s4g open loop during 503 storm: 40 x http_status over 1.95 s (schedule kept)
s2 crlf / two_per_write / split_event / utf8_split / big64k / proxy_buffered: 6 tokens, 5 ITL, ttft 0.021-0.025 (proxy_buffered ttft = latency 0.1416 by construction)
s2 multiline_data (split at JSON member boundary): 6 tokens, 5 ITL, ttft 0.0209 (fixed)
s6b unique prompts: prompt_tokens 22 (nonce [nonce-<run_id>-7-<seq>]), config.unique_prompts true
s7b body at server: ignore_eos true, min_tokens 5, no system message, top_p 0.5; config: ignore_eos true, min_tokens 5, system_prompt "", extra_body_json set, effective_system_prompt null
determinism: seq->prompt identical for two seed-7 runs, different for seed 8; Poisson offsets identical; run_id part of nonce differs per run (by design)
clock step +3600 s after 1 s: latency 0.5043-0.5053 unchanged; window 3602.019 s; rps 0.004 (N-02)
TLS self-signed leaf, no flag: connect x4 (UnknownIssuer / CaUsedAsEndEntity); --ca-cert <self-signed leaf>: connect x4; --ca-cert <CA cert> with CA-signed leaf: 4/4 success, config.ca_cert path stamped; --insecure: 4/4, config.insecure true
gateway holding headers 300 ms: first_byte_s 0.4265, ttft_s 0.4265 (dummy emits first token with headers), latency 0.5102
VLM 8 req c=2 warmup 2: 8 records (2 warmup, 6 measure), ttft 0.1210, image_bytes 70, per_ep = pooled = 0.3027, queue_delay 0
VLM non-stream: ttft null
VLM body via proxy (--system-prompt "" --min-tokens 3 --ignore-eos --extra-body-json top_p): roles [user], min_tokens 3, ignore_eos true, top_p 0.5, image 70 bytes identical; config.effective_system_prompt null; modality {image_detail low, max_image_dimension null, reencode_jpeg false, server_side_download false}
ASR 6 req warmup 2: records 6 (2 warmup), modality_metrics {rtfx_client 39.81, inference_seconds_client 0.1005, inference_time_s, rtf 0.0251, wer 1.0, cer 0.947, audio_duration_s 4.0, bytes_*}; no legacy block; json format -> inference_seconds_server 0.1
imagegen full URL: 6 v3 records (2 warmup) + 6 v1 records; v3 window 0.203 rps 19.68 (measured phase only); hashes verified on v1 lines
strategic conc sweep (K=4 x 100 ms): points n=40, p99_unreliable true; knee load 4; --slo e2e=150ms goodput 9.88/19.72/39.35/3.95/3.94 vs throughput 9.88/19.72/39.35/39.49/39.41
strategic rate sweep: knee 40 (previous run); sessions: validity 1.0, per-message turns, unique prefix per session
MLPerf: line 1 "UNOFFICIAL: This is NOT an audited or submitted MLPerf result. ..."; line 17 "Result is : VALID (unofficial; see disclaimer)"
SMOKE_RESULTS audit: 12/12 rows match on n, err, lat p50/p95, window, rps; 12/12 rows' TTFT columns equal first_byte_s, not ttft_s (N-01); warmup records present (8 per cell); per_endpoint latency = pooled; queue_delay 0; config present; p95_unreliable flags present in raw summaries
```

## 8. Issues

Fixed findings were closed with a comment pointing at this document;
partial findings received a comment stating the residual; N-01 through N-09
were filed as new issues and added to the epics #42 and #43.

### Post-assessment closeout (2026-09-16)

- **N-01 / #56** fixed in PR #61.
- Measurement residuals **N-02–N-07, N-09, F-09, F-14** landed in PR #62
  (closes #57 #58 #26 #36 #60 #59 #31); superseded #27 #38 closed.
- OSS README/packaging/Dependabot in PR #63 (closes #10 #12 #13); #17 closed.
- Prompt library intentionally descoped to a separate Hugging Face dataset.
- Still open / admin: #6 (old tags), #7 (non-provider patterns), #15
  (credentials), N-08 stretch (#33 knee / strategic streaming).

