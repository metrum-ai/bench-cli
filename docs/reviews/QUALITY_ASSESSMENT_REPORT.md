<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Metrum AI Bench: production-fidelity quality assessment

| Field | Value |
|---|---|
| Repository | `/home/<user>/src/bench-cli` (crate `metrumbench`, binaries `metrum-ai-bench-*`) |
| HEAD commit | `858bb757e1ba33e6b61216d67e43311d13133b1a` ("docs(smoke): refresh SMOKE_RESULTS from retained-GPU rerun (#5)") |
| `Cargo.toml` version | `0.1.82` |
| Assessment date | 2026-09-15 (UTC) |
| Host | 16 cores, 30 GiB RAM, Linux 6.8.0-139-generic, rustc 1.97.1 (8bab26f4f 2026-07-14), cargo 1.97.1, go 1.26.4 linux/amd64, python 3, gcc, openssl |
| Tree state at start | clean except untracked `docs/QUALITY_ASSESSMENT_PROMPT.md`; gitignored `env.json`, `live-results/`, `target/` present |
| Revision | 2 (2026-09-15): tightened after maintainer review; verdict, scores, and findings unchanged; added the distinction between bookkeeping and gaming defects, the F-05 inversion, acceptance tests for F-01/F-04, and split trackers |

Every `file:line` in this report refers to commit `858bb757`. Every command
run, its exit code, and the raw numbers quoted in the body are in Appendix A.
Helper sources (recomputation script, adversarial servers, TLS proxy, clock
shim) are in Appendix B. Verdict labels: VERIFIED means the behavior was
observed by running the built release binary; SUSPECTED means it was read in
the code but not exercised; UNDETERMINED states what would be needed.

---

## 1. Executive summary

**Public-use verdict: NO-GO at `0.1.82`.** The tree builds, formats, lints,
and passes all 117 tests with the Go dummy server required, the license and
governance files are in place, no live credential is tracked or in history,
and the per-request measurements (TTFT, E2E, ITL, TPOT) are correct to within
a few milliseconds of the dummy server's known timing profile. But the
`summary.v2` record, which is the line the documentation tells consumers to
read, carries a throughput number that is wrong by a factor between 2 and 190
in every closed-loop run I made and in every raw GPU result file under
`live-results/`, and the same file carries a second, legacy summary with a
different (correct) throughput. A reader cannot tell which to trust. The
run configuration that would let a buyer detect a flattering setup (seed,
warmup count, arrival process, `ignore_eos`, injected system prompt, unique
prompt nonce, SLO thresholds) is not written into any output. And the README
claim that records are flushed incrementally is false: nothing reaches disk
until the launch loop finishes, so a SIGTERM or SIGKILL loses the entire run.

Two of those three are different kinds of defect and should be read
differently. The throughput window (F-01) is a drain-order bookkeeping bug:
it happens to flatter the number, but nobody has to turn a knob to get it,
and the per-request records it is derived from are correct. The unrecorded
configuration (F-03), the injected default system prompt, and the two
disagreeing summaries in one file (F-05) are the anti-gaming holes: they let
a motivated run leave no trace. The scorecard keeps them apart (H = 1 for the
latter, M = 2 for the former); a reader should too.

### Persona verdicts

**Persona A (capacity owner).** I would not use the `summary.v2`
`requests_per_second`, `completion_tokens_per_second`, or `goodput` fields
for a replica-count decision. In the reference run they read 15.8 req/s
where the true served rate was 7.9 req/s; in a 3000-request run they read
46,250 req/s where the truth was 246 req/s; in the overload scenario the tool
reported the *offered* rate (39.9 req/s) as throughput while the server was
serving 7.9 req/s (Finding F-01). The per-request records are sound, so a
capacity owner who recomputes the window from `started_at + latency_s` gets
a correct answer, but that defeats the purpose of a summary. Open-loop
scheduling, queue-delay capture, and the coordinated-omission latency field
do work correctly in open loop and are the strongest part of the tool for
this persona (Section 7). The tool cannot be pointed at a TLS gateway that
uses a private CA (F-13), and it gives no connect/first-byte decomposition
(Q1).

**Persona B (stack engineer).** Comparing two engines' TTFT, ITL, and TPOT
distributions from the per-request records is trustworthy: the SSE parser
survived every framing variant I threw at it except a spec-compliant
multi-line `data:` event (F-23), role-only and reasoning-only streams are
correctly classified as `no_output_token`, and reasoning time is separated.
Two caveats block a clean A/B: in closed loop, `per_endpoint.*.latency_s`,
`coordinated_omission_latency_s`, and e2e goodput are polluted by a
queue-delay term that is really "time since the run started" (F-02), and the
default system prompt "You are a helpful assistant." is injected into every
chat request and never recorded (F-03). Non-streaming LLM runs report a
fabricated TTFT equal to E2E (F-08).

**Persona C (buyer).** The output file does not let a buyer catch a flattered
result. None of `--seed`, `--warmup-requests`, `--request-rate`,
`--arrival`, `--max-concurrency`, `--ignore-eos`, `--min-tokens`,
`--system-prompt`, `--unique-prompts`, `--extra-body-json`, `--slo`, or the
throughput bin width is written to any record by any of the four modality
binaries; the imagegen binary writes no `config` block at all (F-03). A
vendor can run with `--ignore-eos --min-tokens 512 --system-prompt ""` and
ship a file indistinguishable from a natural-stopping run. The published
`docs/SMOKE_RESULTS.md` numbers do trace exactly to the raw files, but those
raw files carry the wrong throughput and polluted per-endpoint latencies, and
the VLM cells silently dropped their eight warmup records (F-15, F-10).

### Three most important findings

1. **F-01 (CRITICAL, VERIFIED).** `summary.v2.window_seconds` is computed
   from timestamps taken in the result-collection loop, which runs only
   after every request has been launched, so `sent = now - latency` is wrong
   for every request that finished before launching ended. Reference run:
   window 1.011 s for a 2.020 s run, throughput 15.83 vs 7.92 req/s. Live
   GPU cell `llm/c1-n64`: window 0.49 s, 113.6 req/s reported for a run whose
   records span about 14 s. Location `src/bin/metrum-ai-bench-llm.rs:1720-1723`.
   Closed loop and open loop fail differently: closed loop inflates by about
   `N / (2 x concurrency)`; open-loop overload reports the offered rate as
   served throughput. Neither is caught by the test suite, which feeds
   `window_seconds` into `RunSummary::from_records` as an input.
2. **F-03 (CRITICAL, VERIFIED).** No shared workload flag reaches the output.
   Reproduction: run with `--ignore-eos --min-tokens 5 --system-prompt ""
   --extra-body-json '{"temperature":0.9,"top_p":0.5}'`; the request body
   carries all of them (captured at the server), the data log carries none.
3. **F-04 (CRITICAL, VERIFIED).** The data log stays at 0 bytes for the entire
   3000-request run and is written only after the last request is launched;
   SIGTERM (exit 143) and SIGKILL (exit 137) leave an empty file. README line
   47 and `docs/OUTPUT_SCHEMA.md` line 10 claim the opposite.

### Three things the tool does that are genuinely valuable

1. **Open-loop arrivals with recorded queue delay and seeded reproducibility.**
   Two runs with `--seed 7 --request-rate 30 --arrival poisson --unique-prompts`
   produced byte-identical seq-to-prompt mappings and identical Poisson
   offsets at a body-logging proxy; a third run with `--seed 8` differed.
   Under overload (`--request-rate 40` against an 8 req/s server) the record's
   `queue_delay_s` reached 3.6 s and `coordinated_omission_latency_s.max` was
   4.12 s while `latency_s` stayed at 0.50 s. vLLM's `benchmark_serving.py`
   does not expose queue delay per request.
2. **Byte-level SSE robustness with typed first-token semantics.** CRLF,
   comment and `id:` lines, two events per TCP write, events split across
   three writes, a 3-byte UTF-8 character split at every byte offset, a
   64 KiB event, a missing `[DONE]`, a finish-with-usage-then-close, a
   role-only stream, and a reasoning-only stream all produced the expected
   token counts, TTFT, and error classes. `first_reasoning_s` is reported
   separately from `ttft_s` (0.051 s vs 0.061 s against a dummy that emits a
   reasoning delta 10 ms before content).
3. **Knee detection that found the true knee.** Against a server with four
   workers of 100 ms each (capacity 40 req/s), the concurrency sweep
   1,2,4,8,16 reported the knee at concurrency 4 and the rate sweep 10,20,40,80
   reported it at 40 req/s, both correct. The exports around it (no `n`, no
   reliability flag, MLPerf files without a disclaimer) are the problem, not
   the estimator.

---

## 2. Scorecard

Anchor: 1 means a persona would be actively misled; 3 means usable with
caveats the output itself discloses; 5 means the output alone suffices to
make the decision and catch a misleading result.

| Axis | Score | One-line justification |
|---|---|---|
| R. Real-world fidelity | 3 | TTFT/E2E include connection, gateway hops, and buffering as a user would see them (a 300 ms header-holding proxy moved TTFT from 0.121 to 0.426 s), open loop and 429/503 storms run and are counted; but the record cannot separate connect time from first byte from first token, private-CA TLS cannot be tested at all, and multi-turn exists only in the non-streaming strategic runner. |
| H. Honesty and anti-gaming | 1 | `ignore_eos`, `min_tokens`, system-prompt removal, unique-prompt nonce, warmup count, seed, arrival, cap, and SLO thresholds leave no trace in the file; a hidden system prompt is injected by default; the same file carries two throughput numbers that differ by 2x. |
| M. Measurement correctness | 2 | Per-request TTFT, E2E, ITL, TPOT (N-1), type-7 percentiles, error rate, and pooled/per-endpoint distributions recompute exactly (0 mismatches over 133-185 fields per run); but the throughput window is wrong by 2x to 190x, closed-loop queue delay corrupts three distributions, non-streaming TTFT is fabricated, and 5xx/timeout/refused errors collapse to `other`. |
| I. Differentiated value | 3 | Per-request queue delay, reasoning-vs-visible TTFT, `pooled_mixture` with full per-endpoint distributions, seeded Poisson schedule, byte-identical image payload with recorded size, and a correct knee on a real saturation curve are all demonstrated; none of the reference tools gives all of these. |
| O. Operator experience | 2 | Clone to a correct per-request number is under ten minutes; but the only summary the docs point to is wrong, nothing is on disk until the end, SIGTERM loses everything, a second Ctrl-C is ignored, imagegen takes a base URL where the others take a full path, and one failed request makes the exit code non-zero. |
| E. Engineering quality | 2 | `stats`, `sse`, `summary`, `record`, `load` are shared and tested against numeric references; but each binary still carries 1,400-2,100 lines including a private `Metrics` struct, a second (and third) percentile estimator, a legacy summary block, and per-binary warmup and window logic that has already drifted (four different window definitions). |
| P. Public readiness | 3 | Apache-2.0, NOTICE, THIRD_PARTY_LICENSES, SECURITY, CODEOWNERS, gitleaks in CI, headers enforced, signed release artifacts and SBOM; `cargo deny check` fails on four advisories (one vulnerability) and CI gates only licenses; formula and license inventory say 0.1.80. |
| C. Competitive context | n/a | Appendix C. |

---

## 3. Direct answers to the Section 6 questions

1. **TTFT clock.** Started at `Instant::now()` at `src/bin/metrum-ai-bench-llm.rs:786`, before `client.post(...).send()`, after the body is built; stopped when the first choice with non-empty `delta.content`, `text`, or tool-call `arguments` is parsed (`src/sse.rs:75-98`). It includes connection establishment or pool reuse, TLS, request write, gateway hops, server queue, and prefill. That is the user-experienced number and is correct to include; but the record has no connect, first-byte, or headers-received timestamp, so a reader cannot separate a 300 ms gateway hold from prefill (VERIFIED: TTFT 0.426 s through a header-holding proxy vs 0.121 s direct, and no field explains the difference). `docs/METRICS.md` states the definition but not that it includes connection setup (F-13, Q1).
2. **First token.** Non-empty `delta.content`, `text`, or non-empty `function_call.arguments` / `tool_calls[].function.arguments`. Role-only deltas and `reasoning_content`/`reasoning` deltas do not count; the first reasoning delta sets `first_reasoning_s`. If nothing visible arrives the request is `no_output_token` (VERIFIED with dummy `-role-only` and a reasoning-only adversarial stream). An in-stream `{"error":...}` object is `api_error` in the LLM binary (VERIFIED) but ignored in the VLM binary (SUSPECTED, `src/bin/metrum-ai-bench-vlm.rs:697-736`).
3. **Token counts.** From server `usage` only. Missing `usage` yields a success with `completion_tokens: 0`, `usage_missing: true` on the record, and `completion_tokens_per_second: 0.0` in the summary with no summary-level flag (VERIFIED in streaming and non-streaming). The local tokenizer counts (`tokenized_*`) are separate fields and are not used as a fallback; the `RequestError::UsageMissing` variant is never constructed (F-11).
4. **ITL and TPOT.** ITL is one sample per SSE event carrying a visible token, timestamped at parse time (`llm.rs:924-936`); two events in one TCP read yield a near-zero interval. TPOT is `(latency - ttft) / (completion_tokens - 1)`, undefined for fewer than two tokens (`src/record.rs:157-164`; VERIFIED 20.21 ms against a 20 ms chunk interval). Stalls appear as large values in the per-record `itl_s` array and in the pooled `itl_s.max`; there is no stall count or threshold.
5. **Percentiles.** Hyndman-Fan type 7 in `src/stats.rs:8-28`, labeled `percentile_method` and carrying `n` in every `DistSummary`. It is not the only estimator in the tree: the LLM binary's console and legacy block use nearest-rank with `round()` (`llm.rs:291-298`, `1083-1089`), the VLM and ASR binaries use nearest-rank with `ceil()-1` (`vlm.rs:294-301`, `asr.rs:311-327`). Only p99 is flagged (`p99_unreliable` when `n < 100`); p95 at `n = 24` in the published VLM rows (1.2 samples above the estimate) is not flagged (F-24).
6. **Throughput window.** Documented as first measured send to last measured completion. Implemented in the LLM binary as `min(sent) .. max(finished)` where `finished` is the time the collection loop reached the handle, not the completion time (`llm.rs:1720-1723`); VLM uses whole-run elapsed including console printing (`vlm.rs:1796`); ASR uses loop-start to drain including warmup (`asr.rs:1595`); imagegen uses whole-run elapsed including warmup (`imagegen.rs:1102`). `window_seconds` is recorded; its definition is not, and the bin width is not (F-01, F-20).
7. **Open loop.** `latency_s` excludes queue delay; `coordinated_omission_latency_s` includes it; `docs/METRICS.md` calls the latter "the headline open-loop latency" but the field literally named `latency_s` is the other one (VERIFIED: 0.501 s vs 4.12 s max under overload). `--concurrency` is mandatory and acts as the cap unless `--max-concurrency` is larger; there is no field saying the cap engaged, only nonzero `queue_delay_s` (F-09).
8. **Reproducibility from seed.** Yes for prompt order and arrival schedule. Two runs with `--seed 7` produced identical seq-to-prompt maps and identical Poisson offsets at the proxy; `--seed 8` differed (Appendix A.8). No sampling `seed` is sent to the server, so completions are not reproducible.
9. **Unrequested parameters.** Chat mode injects `{"role":"system","content":"You are a helpful assistant."}` (`llm.rs:1374`; VLM injects "...capable of understanding images." at `vlm.rs:1153` and ignores `--system-prompt`), `temperature: 0.1`, `stream_options.include_usage: true` when streaming, ASR sends `language=en` and `timestamp_granularities[]=word`, VLM sends `detail: "low"`. Of these only `temperature`, `language`, and `image_detail` are in the legacy `config`; nothing from `CommonBenchArgs` is (F-03).
10. **Prefix cache.** Both states run. `--unique-prompts` prefixes `[nonce-{seq}]` (VERIFIED at the server); the default repeats prompts verbatim (VERIFIED). Neither state is recorded anywhere in the output (F-03). The nonce is the sequence number, so it repeats across runs and a warm server would hit its prefix cache on a rerun.
11. **Failures.** `attempted = successes + errors` over measure-phase records; `error_rate = errors / attempted`; `requests_per_second = successes / window` (v2) but `(successes + errors) / elapsed` in the legacy block and console. Failed requests are excluded from every latency distribution (VERIFIED) and recorded with `latency_s: 0.0` (F-07). Classification: 429 → `rate_limit`, mid-stream error object → `api_error`, role-only → `no_output_token`; HTTP 503, connection refused, connection reset, and request timeout all → `other` (F-06).
12. **SIGINT and SIGKILL.** SIGINT stops issuance, drains in-flight work, writes all completed records and a `partial: true` summary (VERIFIED: 754 records, exit 0, 1.0 s after the signal). Because nothing is flushed before the drain, SIGKILL and SIGTERM leave a 0-byte file (VERIFIED). A consumer of a partial file must select lines by `schema_version`, accept a missing summary, and recompute the window from `started_at + latency_s` because `completed_at` is collection time (F-04, F-19).
13. **Per-endpoint.** Complete: each endpoint gets `attempted/successes/errors` and full `latency_s/ttft_s/tpot_s/itl_s` distributions, and `pooled_mixture: true` is set with two or more endpoints (VERIFIED with a dead replica). But the per-endpoint `latency_s` is the corrected latency while the pooled `latency_s` is not (`summary.rs:142` vs `:253`), so in closed loop the per-endpoint number is wrong (F-02).
14. **NTP.** Opt-in via `--ntp-check`; the offset is recorded as `environment.ntp_offset_ms` (VERIFIED: 31 ms) and never gates the run.
15. **ASR.** Default `whisper-english`, recorded as legacy `config.normalizer` (VERIFIED), applied to both sides (`src/asr.rs:88-89`). Per-request `rtfx_client = manifest duration / client wall time` (VERIFIED 39.8). Server and client inference time are separate keys `inference_seconds_server` / `inference_seconds_client` (VERIFIED both appear depending on response format). The legacy block also reports `throughput.rtfx = 59.5` for the same run (total audio over whole-run elapsed including warmup and setup): two RTFx definitions in one file (F-21).
16. **VLM.** The 70-byte PNG arrived byte-identical at the proxy with `image/png` and `detail: low`, and `modality_metrics.image_bytes = 70` (VERIFIED). Preprocessing happens before the timer but its duration is not reported anywhere. Streaming TTFT is measured (0.1207 s against a 100 ms + 20 ms dummy); non-streaming TTFT is null, not derived (VERIFIED).
17. **Imagegen.** `Instant`-based latency serialized as f64 milliseconds with sub-millisecond resolution (102.428 ms; VERIFIED), warmup excluded from counts and distributions but not from the throughput denominator (4 requests over 0.308 s that included 2 warmup requests), SHA-256 per artifact verified against the file on disk. Base64 decode, PNG decode, hash, and the synchronous file write are inside the timed window (SUSPECTED impact; `imagegen.rs:994-1004`).
18. **Strategic runner.** Knee correct on both sweeps against a known saturation point (VERIFIED). Validity rate 1.0 matched a hand count of 12/12 schema-valid responses; goodput equals throughput because there is no SLO input (`strategic.rs:79`). The MLPerf files contain no disclaimer; `mlperf_log_summary.txt` prints `Result is : VALID` and identical "Scheduled" and "Completed" rates over all stages pooled (F-14).
19. **Published results.** All ten LLM/VLM rows and the ASR and imagegen rows recompute exactly from `live-results/campaign-oss-20260915-smoke-rerun` (Section 6). Units are stated. Sample sizes are 56 and 24 and the raw summaries carry `p99_unreliable: true`, but the document shows p95 without a flag and does not mention that the raw `requests_per_second` in every one of those files is wrong (F-15). Dummy rows are labeled dummy-certified.
20. **What each persona learns that reference tools would not tell them.** A: per-request `queue_delay_s` and a corrected latency under overload, and a cap that is explicit rather than implicit. B: reasoning time separated from visible TTFT, `usage_missing` per record, byte-identical image payload size. C: `pooled_mixture` labeling and full per-endpoint distributions. All three are verified in Section 7; none is enough while F-01 and F-03 stand.
21. **Credentials.** One gitignored file `env.json` (62 bytes, one key `SHADEFORM_API_KEY`) exists in the working tree. It is covered by `.gitignore` (`env.json`, `env.*.json`), is not tracked, and its value appears nowhere in any tracked file or in any commit on any ref (checked in-process without printing it). No other credential-shaped string was found in tracked files. Presence only; no value is reproduced here.

---

## 4. Findings

Ordered by severity. "Blocks public use" names the persona who would act on a
wrong number. Line numbers are at `858bb757`.

### F-01 `summary.v2` throughput window is measured in the collection loop, not at completion

- **Severity:** CRITICAL. **Status:** VERIFIED.
- **Location:** `src/bin/metrum-ai-bench-llm.rs:1679-1723`, consumed at `src/summary.rs:182-183`.
- **What the code does:**
  ```rust
  for handle in handles {                                   // llm.rs:1679
      ...
      let finished = Instant::now();                        // llm.rs:1720
      let sent = finished.checked_sub(response_time).unwrap_or(finished);
      if phase == metrumbench::record::Phase::Measure {
          window_start = Some(window_start.map_or(sent, |s| s.min(sent)));
          window_end = Some(window_end.map_or(finished, |e| e.max(finished)));
  ```
  The `for handle in handles` loop starts only after the launch loop (`llm.rs:1587-1677`) has issued the last request. In closed loop the launch loop blocks on the semaphore, so by the time collection starts all but the last `concurrency` requests are already complete; `finished` for each of them is "now", and `sent = now - latency` is a fiction. The window collapses to roughly one or two request durations regardless of `N`.
- **Why it matters:** Persona A reads `requests_per_second`, `completion_tokens_per_second`, and `goodput.requests_per_second` as served capacity. All three divide by this window. Two distinct failure modes: in closed loop the last wave is still in flight when the join loop starts, so the window is about two request durations and the factor is about `N / (2 x concurrency)`; in open-loop overload the launch loop paces at the offered rate, so the reported throughput tracks the *offered* rate rather than the served rate (39.9 vs 7.9 req/s below). This is a bookkeeping defect, not a gaming knob; it is classified under M, not H.
- **Why the tests did not catch it:** `tests/core_measurement.rs::summary_matches_hand_computed_reference_16_requests` passes `window_seconds = 2.01956` into `RunSummary::from_records` as an input, so it pins the arithmetic given a window, never the window itself. No test compares `window_seconds` to `max(started_at + latency_s) - min(started_at)` on a real closed-loop run.
- **Reproduction:** `docs/REPRODUCING.md` command (16 requests, concurrency 4, 100 ms + 20 x 20 ms dummy):
  ```
  window_seconds = 1.011043719   requests_per_second = 15.825   completion_tokens_per_second = 316.5
  legacy metrics.timing: metrics_collection_seconds = 2.0210, requests_per_second = 7.917
  recompute.py true window (started_at + latency_s) = 2.020315 s -> 7.9196 req/s
  ```
  3000 requests at concurrency 8: `window_seconds = 0.0649`, `requests_per_second = 46249.5`; true window 12.20 s, 245.95 req/s. Open-loop overload (`--request-rate 40` against 8 req/s capacity, cap 4): reported 39.89 req/s, true 7.86 req/s. Live GPU file `live-results/campaign-oss-20260915-smoke-rerun/llm/c1-n64/results.jsonl`: `window_seconds = 0.4929`, `requests_per_second = 113.6` for 56 measured requests of 0.244 s each at concurrency 1 (true about 4 req/s).
- **Fix (honest):** compute `sent` and `finished` inside the spawned task with `Instant::now()` before `make_request` and after it returns, and pass them out in the tuple; record `window_start_unix`/`window_end_unix` in the summary. **Acceptance test:** an e2e against the dummy at concurrency 4, 16 requests, asserting `window_seconds` within 5 % of `max(started_at + latency_s) - min(started_at)` over the records, and `requests_per_second` within 5 % of `16 / that window`. F-02 (`queue_delay_s`), F-19 (`completed_at`), and prior D-06 share this root cause (timestamps taken in the collector, not in the task) and should land in the same pull request. **Fix (right):** the same, plus define the window once in `summary.rs` from `started_at + latency_s` of measured records so all four binaries share it (VLM, ASR, and imagegen each use a different whole-run elapsed today; Section 3 Q6).
- **Effort:** S. **Blocks public use:** yes (A, C).

### F-02 Closed-loop `queue_delay_s` is "time since run start" and corrupts three shared distributions

- **Severity:** CRITICAL. **Status:** VERIFIED (LLM, VLM, ASR; imagegen never calls `with_schedule`).
- **Location:** `src/bin/metrum-ai-bench-llm.rs:1611-1614` (`vlm.rs:1402-1403`, `asr.rs:1338`), `src/load.rs:60-65`, `src/summary.rs:143`, `:232`, `:253`, `:283-291`.
- **What the code does:**
  ```rust
  let permit = semaphore.clone().acquire_owned().await?;                 // llm.rs:1611
  ...
  let queue_delay = start_time.elapsed().saturating_sub(slot.scheduled_delay); // llm.rs:1614
  ```
  `load::schedule` returns `scheduled_delay = Duration::ZERO` for every closed-loop slot, so `queue_delay` is the wall time at which the permit was acquired. `record.rs:153` adds it to latency; `summary.rs:143` builds `coordinated_omission_latency_s` from it, `summary.rs:253` uses it for **per-endpoint** `latency_s`, `summary.rs:232` evaluates the e2e SLO against it, and `summary.rs:283-291` bins every record at offset 0.
- **Why it matters:** Persona B comparing two engines from `per_endpoint` sees 1.263 s where the true mean is 0.505 s; the live GPU file `llm/c1-n64` shows `per_endpoint.latency_s.avg = 8.98 s` against a true 0.244 s; any `--slo e2e=` goodput in closed loop is evaluated against run-elapsed time.
- **Reproduction:** reference run: `latency_s.avg = 0.50501`, `coordinated_omission_latency_s.avg = 1.26281`, `per_endpoint["127.0.0.1:18321"].latency_s.avg = 1.26281`, `queue_delay_s` nonzero on 16/16 records, `throughput_bins_rps = {n: 1, p50: 1.6}` for 16 requests in 1.0 s.
- **Fix (honest):** set `scheduled_offset_s = None` and `queue_delay_s = 0` in closed loop; make per-endpoint latency use the same estimator as the pooled block. **Fix (right):** make `RunSummary` carry both `latency_s` and `corrected_latency_s` at every level with the same definition, and bin throughput by actual send time, not scheduled offset.
- **Effort:** S. **Blocks public use:** yes (B, A).

### F-03 No workload flag that changes the server's work is recorded in any output

- **Severity:** CRITICAL. **Status:** VERIFIED.
- **Location:** `src/bin/metrum-ai-bench-llm.rs:1213-1236` (the only `config` block; ends at `"ramp_up_seconds": args.ramp_up_seconds,`), `src/summary.rs:10-31` (`RunSummary` has no config), `src/bin/metrum-ai-bench-imagegen.rs` (no `config` anywhere), `src/args_common.rs:16-99`.
- **What the code does:** the legacy `config` lists `scenario, url, endpoint, endpoints, model, mode, streaming, num_requests, concurrency, max_tokens, temperature, log_level, prompts_file, data_log, debug_log, error_log, request_timeout, connect_timeout, pool_idle_timeout, tcp_keepalive, stop_after_seconds, ramp_up_seconds`. Not one field of `CommonBenchArgs` (`seed, warmup_requests, request_rate, arrival, max_concurrency, load_balancer, ignore_eos, min_tokens, extra_body_json, system_prompt, unique_prompts, tokenizer, slos, throughput_bin_seconds`) is serialized by any binary. The chat body always carries `"role":"system","content":"You are a helpful assistant."` (`llm.rs:1374`) unless `--system-prompt` is given, and that too is unrecorded.
- **Why it matters:** Persona C cannot tell a natural-stopping run from `--ignore-eos --min-tokens 512`, a cold-cache run from a repeated-prompt run, or a warmed run from a cold one. Persona A cannot reproduce a run from its own output.
- **Reproduction:** `--ignore-eos --min-tokens 5 --system-prompt "" --extra-body-json '{"temperature":0.9,"top_p":0.5}'`: the server received `ignore_eos: true, min_tokens: 5, top_p: 0.5`, no system message; the data log's `config` block has none of `ignore_eos, unique_prompts, system_prompt, seed, warmup_requests, request_rate, max_concurrency` (Appendix A.6). The published campaign files show the same: every `config` in `live-results/.../llm/*/results.jsonl` lacks `seed`, `warmup_requests`, and `request_rate` although the commands used `--seed 7 --warmup-requests 8` and `--request-rate`.
- **Fix (honest):** serialize the entire `CommonBenchArgs` plus the effective system prompt and the final request-body template into `RunSummary` as `config`, in all four binaries. **Fix (right):** also stamp `unique_prompts`, `ignore_eos`, and `warmup_requests` on each request record so a partial file is self-describing.
- **Effort:** S. **Blocks public use:** yes (C).

### F-04 Records are not flushed until the launch loop ends; SIGTERM and SIGKILL lose the run

- **Severity:** CRITICAL. **Status:** VERIFIED.
- **Location:** `src/bin/metrum-ai-bench-llm.rs:1679` (collection loop after launch loop), `:1747` (`sink.write(&rec)` inside it), `:1547-1551` (only `ctrl_c` is handled). Claims at `README.md:47` and `docs/OUTPUT_SCHEMA.md:10`.
- **What the code does:** the request task returns `StreamMetrics`; the `RequestRecord` is built and written only when the main task reaches that handle in `for handle in handles`, which begins after the last launch.
- **Why it matters:** Persona A's long soak test that is killed by a scheduler or an OOM leaves nothing; the README promises the opposite.
- **Reproduction:** 3000 requests, concurrency 8: data log 0 bytes at every 0.5 s sample until the end (RSS 6.9 -> 12.6 MB). SIGINT at 3 s: 754 records + `partial: true`, exit 0. SIGKILL at 3 s: 0 records, exit 137. SIGTERM at 2 s: 0 lines, exit 143 (Appendix A.9).
- **Fix (honest):** write the record inside the spawned task through the shared `JsonlSink` and correct the README. **Fix (right):** also handle SIGTERM like Ctrl-C and time out the drain (F-18 belongs in the same pull request). **Acceptance test:** the existing `tests/e2e_adversarial.rs::ctrl_c_writes_a_partial_summary` only checks that records exist *after* the SIGINT drain, which passes today; add a test that the data log has at least one `request.v2` line while the run is still launching (for example, poll the file size during a 200-request run at concurrency 2 against a 250 ms dummy), and one that SIGTERM leaves a parseable prefix.
- **Effort:** S. **Blocks public use:** yes (A).

### F-05 Two summaries per file with different definitions and estimators

- **Severity:** CRITICAL. **Status:** VERIFIED.
- **Location:** `src/bin/metrum-ai-bench-llm.rs:1865-1875` (writes `RunSummary` then `create_log_record`), `:291-298` (`calc_percentile`, nearest-rank `round`), `:1083-1089`, `:1322` (`errors.rate` in percent), `:1331` (rps = successes+errors over elapsed). VLM `vlm.rs:294-301` (`ceil()-1`), ASR `asr.rs:311-327`.
- **Why it matters:** Persona C receives one file with `requests_per_second: 15.83` (v2) and `requests_per_second: 7.92` (legacy), `p99` in seconds by type 7 and `p99_ms` as an integer by nearest rank, `error_rate: 0.5` and `rate: 50.0`. Two summaries would be messy but survivable if the documented one were right. Here `docs/OUTPUT_SCHEMA.md` tells consumers to select `summary.v2`, and v2 is the *wrong* one for throughput while the deprecated legacy block happens to match wall time. That inversion, not the duplication, is what makes this CRITICAL: a careful reader who follows the docs gets the worse number.
- **Reproduction:** Appendix A.3 (reference run, both blocks quoted).
- **Fix (honest):** add `"deprecated": true` and `"authoritative": "metrum-ai-bench.summary.v2"` to the legacy object, or drop it behind a flag. **Fix (right):** remove the legacy block and the private estimators; render the console from `RunSummary`.
- **Effort:** M. **Blocks public use:** yes (C).

### F-06 HTTP 5xx, connection refused, connection reset, and request timeout all classify as `other`

- **Severity:** HIGH. **Status:** VERIFIED.
- **Location:** `src/jsonl.rs:34-55` (substring matching on `err.to_string()`), `src/bin/metrum-ai-bench-llm.rs:835` (`.context("Streaming request failed")`), `:871`/`:992` (`.context(format!("Error type: {}", error_type))`).
- **What the code does:** `anyhow::Error`'s `Display` prints only the outermost context, so the string reaching `classify_error` is "Streaming request failed" or "Error type: server_error"; `parse_http_status` looks for "http error:" and never finds it. reqwest 0.12's `Display` for a timeout or connect failure is "error sending request for url (...)", which contains neither "timeout" nor "connect".
- **Why it matters:** Persona A's error storm shows `errors_by_type: {"other": 40}` for a server returning 503, and cannot distinguish a gateway timeout from a refused connection. The typed enum exists (`src/error.rs`) but is reached only for 429, mid-stream error objects, `no_output_token`, and `stream truncated`.
- **Reproduction:** dummy `-error-rate 1` non-streaming (503): `{"other": 6}`; adversarial 503 streaming: `{"other": 6}`; `http://127.0.0.1:1`: `{"other": 6}`; RST after request: `{"other": 6}`; slow-loris with `--request-timeout 2`: `{"other": 4}` after 4.0 s; 429 storm: `{"rate_limit": 31}`; mid-stream error: `{"api_error": 6}` (Appendix A.5).
- **Fix (honest):** construct `RequestError::HttpStatus`/`RateLimit`/`Timeout`/`Connect` directly at the point of failure using `reqwest::Error::is_timeout()/is_connect()` and the status code, and stop round-tripping through strings. **Effort:** S. **Blocks public use:** yes (A).

### F-07 Failed requests are recorded with `latency_s = 0.0`

- **Severity:** HIGH. **Status:** VERIFIED.
- **Location:** `src/bin/metrum-ai-bench-llm.rs:1828-1836` (`Duration::ZERO`), same pattern `vlm.rs:1775`, `asr.rs:1563`.
- **Why it matters:** a 300 s timeout and an instant refusal look identical; time-to-failure distributions cannot be built; the slow-loris run above shows `latency_s: 0.0` on requests that took 2 s to fail.
- **Fix:** carry the elapsed time out of the error path (the task already knows it). **Effort:** S. **Blocks public use:** no, but misleads A.

### F-08 Non-streaming LLM runs fabricate TTFT and stop the clock before the body is read

- **Severity:** HIGH. **Status:** VERIFIED.
- **Location:** `src/bin/metrum-ai-bench-llm.rs:980` (`let total_time = start_time.elapsed();` before `response.json().await`), `:1045` (`ttft: total_time`).
- **Why it matters:** Persona B comparing streaming and non-streaming engines sees a TTFT distribution with `n = 4` and `avg = 0.0507 s` that is simply E2E. The VLM binary correctly emits `ttft_s: null` here.
- **Reproduction:** dummy `-latency 50ms` non-streaming: `ttft_avg = 0.0507`, `lat_avg = 0.0507`, `tpot_n = 0` (Appendix A.5, `s3e_nonstream`).
- **Fix:** `ttft: None` in non-streaming mode and stop the clock after the body is consumed. **Effort:** S. **Blocks public use:** yes (B).

### F-09 Open-loop headline `latency_s` excludes queue delay; the cap is implicit and unreported

- **Severity:** HIGH. **Status:** VERIFIED.
- **Location:** `src/summary.rs:142` (`latency_s` from `r.latency_s`), `src/bin/metrum-ai-bench-llm.rs:1556` (`max_concurrency.unwrap_or(args.concurrency)`), `docs/METRICS.md` ("Coordinated-omission latency ... is the headline open-loop latency").
- **Reproduction:** `--request-rate 40 --concurrency 4` against a 500 ms server: `latency_s.avg = 0.5012`, `coordinated_omission_latency_s.max = 4.116`, `queue_delay_s.max = 3.615`, no field records that the cap of 4 engaged; with `--max-concurrency 1000`: `coordinated_omission_latency_s.max = 0.503` (Appendix A.5, `s5a`/`s5b`).
- **Fix (honest):** record `effective_max_concurrency` and `cap_engaged_requests` (count of records with `queue_delay_s > 0`) in the summary. **Fix (right):** in open loop make `latency_s` the corrected value and expose `service_latency_s` separately, as the strategic runner already does. **Effort:** S. **Blocks public use:** yes (A).

### F-10 VLM: warmup exclusion by completion order drops records; three shared flags silently ignored

- **Severity:** HIGH. **Status:** VERIFIED.
- **Location:** `src/bin/metrum-ai-bench-vlm.rs:1692-1695` (`if completed < args.common.warmup_requests as usize { completed += 1; continue; }` before the record is written), `:1153` (hard-coded system prompt), `:1182-1202` (no `min_tokens`), no `LocalTokenizer` construction.
- **Reproduction:** 8 requests, `--warmup-requests 2`, concurrency 2: 6 records, 0 with `phase: warmup`, last `seq = 7`. Live `vlm/c*-n32` cells: 24 records for `--num-requests 32 --warmup-requests 8`, zero warmup records. Body captured at a proxy with `--system-prompt "" --min-tokens 3 --ignore-eos --extra-body-json '{"top_p":0.5}'`: system message "You are a helpful assistant capable of understanding images." still present, `min_tokens` absent, `ignore_eos` and `top_p` present (Appendix A.13).
- **Fix:** delete the completion-order skip (phase already handles it) and route the three flags like the LLM binary does. **Effort:** S. **Blocks public use:** yes (B, C).

### F-11 Missing `usage` is a success with zero tokens and zero token throughput

- **Severity:** HIGH. **Status:** VERIFIED.
- **Location:** `src/bin/metrum-ai-bench-llm.rs:1728` (`usage_missing = completion_tokens == 0 && !completion_text.is_empty()`), `src/summary.rs:151` (`completion_tokens` summed over successes), `src/error.rs:16` (`UsageMissing` never constructed).
- **Reproduction:** streaming server that never sends `usage`: three successes, `completion_tokens: 0`, `usage_missing: true`, `itl_n = 5` per record, summary `completion_tokens_per_second = 0.0`, `tpot_s.n = 0`, no summary flag (Appendix A.5, `no_usage_stream`; also `s3f_nonstream_no_usage`).
- **Fix (honest):** add `usage_missing_count` to the summary and set `completion_tokens_per_second: null` when it is nonzero. **Fix (right):** fall back to `tokenized_completion_tokens` when a tokenizer is loaded, and record which count was used. **Effort:** S. **Blocks public use:** yes (B).

### F-12 Least-inflight balancing concentrates traffic on a fast-failing replica

- **Severity:** HIGH. **Status:** VERIFIED.
- **Location:** `src/endpoints.rs:101-111` (min in-flight, no failure memory).
- **Reproduction:** two endpoints, one refusing connections, 20 requests at concurrency 4: round-robin 10/10 (10 errors); least-inflight 2 alive / 18 dead (18 errors), because a refused connection returns in microseconds and the dead endpoint always has zero in-flight (Appendix A.5, `s8b_li`).
- **Fix:** exclude an endpoint for a backoff period after a connect error, and record the ejection in the summary. **Effort:** S. **Blocks public use:** no (A would notice the 90% error rate), but it is the opposite of what an operator expects.

### F-13 TLS gateways with a private CA cannot be benchmarked; no connect or first-byte decomposition

- **Severity:** HIGH. **Status:** VERIFIED.
- **Location:** `Cargo.toml:83` (`reqwest ... features = ["rustls-tls"]`, no `rustls-tls-native-roots`), no `--ca-cert` or `--insecure` flag in any binary (grep for `danger_accept_invalid`, `add_root_certificate`: none). Record fields: `src/record.rs:32-62`.
- **Reproduction:** self-signed TLS terminator in front of the dummy: every request fails with `invalid peer certificate: Other(OtherError(CaUsedAsEndEntity))` in `error.log`, classified `other`; `SSL_CERT_FILE` has no effect. Through a plain proxy that holds response headers for 300 ms: `ttft_s.avg = 0.4262` vs 0.1209 direct, and the record has no field from which a reader could attribute the 0.305 s (Appendix A.1).
- **Fix (honest):** add `connect_s` (time to connection established or pool hit) and `first_byte_s` (headers received) to the record; document that TTFT includes connection setup. **Fix (right):** add `--ca-cert PATH` and, opt-in and stamped into the config, `--insecure`. **Effort:** M. **Blocks public use:** yes (A).

### F-14 MLPerf-style export mimics LoadGen output with no disclaimer and `Result is : VALID`

- **Severity:** HIGH. **Status:** VERIFIED.
- **Location:** `src/strategic.rs:439-473` (`"MLPerf Results Summary"`, `"Scheduled samples per second"` = `"Completed samples per second"` = achieved rate, `"Result is : VALID"` iff zero failures), `src/bin/metrum-ai-bench-strategic.rs:376,388` (all stages pooled).
- **Reproduction:** `mlperf_log_summary.txt` from the concurrency sweep: `Scheduled samples per second : 21.478056 / Completed samples per second : 21.478056 / samples_per_query : 200 / Result is : VALID`; `grep -il 'audit|official|disclaimer'` over the three files: none (Appendix A.12). The only disclaimer is `docs/STRATEGIC_BENCHMARKING.md:39-42`.
- **Fix:** print "NOT AN OFFICIAL OR AUDITED MLPERF RESULT; produced by metrum-ai-bench-strategic without MLPerf LoadGen" as the first line of every exported file, and either export per stage or state which stages were pooled. **Effort:** S. **Blocks public use:** yes (C).

### F-15 Published raw result files carry wrong throughput and polluted per-endpoint latency; VLM cells lost warmup records

- **Severity:** HIGH. **Status:** VERIFIED (raw files under gitignored `live-results/`; the public document itself shows only latency/TTFT percentiles).
- **Location:** `docs/SMOKE_RESULTS.md` (public); `live-results/campaign-oss-20260915-smoke-rerun/{llm,vlm}/*/results.jsonl` (raw).
- **Reproduction:** Section 6 table. Every LLM cell's `summary.v2` has `window_seconds` between 0.489 and 0.537 s and `requests_per_second` between 104 and 115 for a closed-loop run of 56 measured requests at 0.24-0.26 s each; `per_endpoint.latency_s.avg` is 8.98 s (c1), 4.77 s (c2), 2.45 s (c4), 1.31 s (c8) against pooled `latency_s.avg` of 0.244-0.257 s. VLM cells contain 24 records for 32 requests. The document's "490 measured request lines" reconciles to neither 470 (v2 measure phase) nor 478 (plus 8 imagegen v1 lines).
- **Fix:** regenerate after F-01/F-02/F-10; add `requests_per_second` and `n` columns only when they are right; state the sample-size caveat for p95 at `n = 24`. **Effort:** S after the fixes. **Blocks public use:** yes (C), since the raw files are what a buyer would ask for.

### F-16 Strategic sweep points carry no `n`, no error count, no reliability flag, no config; cold pool per stage

- **Severity:** MEDIUM. **Status:** VERIFIED.
- **Location:** `src/strategic.rs:39-48` (`SweepPoint`), `src/bin/metrum-ai-bench-strategic.rs:233` (`reqwest::Client::builder()` per stage), `strategic.rs:86-110` (`detect_knee`: argmax over interior points, always returns one for 3+ points).
- **Reproduction:** `conc.stdout` points have `load, throughput, p50_s, p95_s, p99_s, error_rate, validity_rate, goodput` only; the HTML (1,519 bytes, no external resources) shows `Load, Throughput, p95 seconds, Error, Goodput` and no `n` or p99; CSV has 200 rows with `seq, stage, ... session_id, turn, error` and no aggregate. The knee itself was correct on both sweeps (Appendix A.12).
- **Fix:** build `SweepPoint` from `stats::DistSummary` (gives `n`, `p99_unreliable`, method), embed the command line and stage sizes in the HTML, warm each stage's pool. **Effort:** S. **Blocks public use:** no.

### F-17 `cargo deny check` fails on advisories; CI gates only licenses

- **Severity:** MEDIUM. **Status:** VERIFIED.
- **Location:** `.github/workflows/ci.yml:96-100` (`command: check licenses`), `Cargo.toml:91` (`ntp = "0.5.0"` -> `time 0.1.45`), `Cargo.toml:97` (`lru = "0.14.0"`).
- **Reproduction:** `cargo deny check` exit 1: RUSTSEC-2020-0071 (vulnerability, `time 0.1.45` via `ntp 0.5.0`), RUSTSEC-2026-0002 and RUSTSEC-2026-0253 (unsound, `lru`), RUSTSEC-2025-0058 (unmaintained, `custom_derive`); `licenses ok`, `bans ok`, `sources ok` (Appendix A.2).
- **Fix:** run `cargo deny check` (all) in CI; replace `ntp 0.5` with a maintained SNTP client or drop the check; `cargo update -p lru`. **Effort:** S. **Blocks public use:** no, but a security-conscious buyer will run this first.

### F-18 Second Ctrl-C ignored; SIGTERM unhandled; drain may last `request_timeout`

- **Severity:** MEDIUM. **Status:** VERIFIED (SIGTERM), SUSPECTED (second Ctrl-C, from `tokio::signal::ctrl_c` semantics at `llm.rs:1547`).
- **Fix:** handle SIGTERM, bound the drain, install a second-signal fast exit. **Effort:** S.

### F-19 `completed_at` is collection time, not completion time

- **Severity:** MEDIUM. **Status:** VERIFIED.
- **Location:** `src/record.rs:91`, `:124` (`completed_at: Utc::now()` in the constructor, called from the collection loop).
- **Reproduction:** reference run records 0-3: `completed_at - started_at = 1.515 s`, `latency_s = 0.506`. A consumer that windows on `completed_at` gets the wrong answer; `started_at + latency_s` is the only correct pair.
- **Fix:** pass the completion wall time from the task. **Effort:** S.

### F-20 `throughput_bins_rps` is degenerate in closed loop and its bin width is unrecorded

- **Severity:** MEDIUM. **Status:** VERIFIED. **Location:** `src/summary.rs:276-296`, `src/args_common.rs:98`.
- **Reproduction:** reference run: `{"n": 1, "p50": 1.6}`; every record has `scheduled_offset_s = 0.0` so all land in bin 0 of a 10 s bin over a 1 s window. Fix: bin by actual send offset; write `throughput_bin_seconds` to the summary. **Effort:** S.

### F-21 ASR reports two RTFx definitions and mixes server/client time in `rtf`

- **Severity:** MEDIUM. **Status:** VERIFIED. **Location:** `src/bin/metrum-ai-bench-asr.rs:1053` (`throughput.rtfx` = all audio seconds including warmup over `Metrics.start_time.elapsed()`), `:1447-1452` (per-request `rtfx_client`), `:746-750` (`inference_time` server if present else client).
- **Reproduction:** same run: per-request `rtfx_client = 39.83`, legacy `throughput.rtfx = 59.47`, `total_audio_seconds = 18.0` (six files including two warmup) (Appendix A.13). Fix: drop the legacy aggregate or define it from measured records only and label it. **Effort:** S.

### F-22 Imagegen: no config block, base-URL semantics differ, warmup inside the throughput denominator, client work inside the timer

- **Severity:** MEDIUM. **Status:** VERIFIED (config, URL, denominator), SUSPECTED (timer contents, `imagegen.rs:994-1004`).
- **Location:** `src/bin/metrum-ai-bench-imagegen.rs:600` (`normalize_base_url`), `:884` (`format!("{}/images/generations", ...)`), `:1102` (`duration_seconds` whole run), `:1181-1182`, `:440` (`Phase::for_seq(index, ...)` where `index` is completion order).
- **Reproduction:** `--url .../v1/images/generations` (the form the other three binaries take) yields six `http_error 404`; `--url .../v1` works. Six requests with two warmup: `request_count = 4`, `duration_seconds = 0.308` (includes the two warmup requests), `images_per_second = 12.99` (Appendix A.13). Fix: accept either URL form, exclude warmup from the window, add `config`. **Effort:** S.

### F-23 A spec-compliant multi-line `data:` event is dropped

- **Severity:** MEDIUM. **Status:** VERIFIED. **Location:** `src/sse.rs:32` (splits on `\n` and treats each `data:` line as one event; the test at `sse.rs:187-195` documents the drop).
- **Reproduction:** first content delta sent as two `data:` lines terminated by a blank line: `completion` counted 5 of 6 tokens, `ttft_s = 0.0411` instead of 0.021 (the second delta), no error (Appendix A.5, `s2_multiline_data`). Real inference servers emit one line per event, so this is a proxy-hardening item, not a headline risk. Fix: buffer `data:` lines until the blank line and join with `\n`. **Effort:** S.

### F-24 Only p99 carries a reliability flag

- **Severity:** MEDIUM. **Status:** VERIFIED (code and published rows). **Location:** `src/stats.rs:31-33`, `:101`.
- **Why it matters:** `docs/SMOKE_RESULTS.md` prints p95 at `n = 24` (1.2 samples above the estimate) and `n = 56` (2.8) without a flag. Fix: emit `p95_unreliable`/`p90_unreliable` by the same rule, or a single `min_reliable_percentile`. **Effort:** S.

### F-25 Console output uses a third estimator and a third throughput denominator

- **Severity:** LOW. **Status:** VERIFIED. **Location:** `src/bin/metrum-ai-bench-llm.rs:291-298`, `:429-471`, `:670-673`. The console prints `Requests/Second: 7.92 (successful: 16, failed: 0)` using `(successes + errors) / elapsed`, `p99` by nearest rank, and `Duration` debug formatting. Fix: render from `RunSummary`. **Effort:** S.

### F-26 Dead shared abstractions and per-binary drift

- **Severity:** LOW (maintainability; the drift it produced is F-01/F-02/F-05/F-10/F-22). **Status:** VERIFIED by grep. `src/modality.rs` (`RequestBuilder`, `ResponseParser`, `MetricExtractor`) and `src/transport.rs` (`Transport`) are referenced by no binary. Each modality binary keeps a private `Metrics`/`EndpointMetrics`, percentile closures, and a legacy summary writer (LLM 1,254 lines of a 2,134-line file are in `Metrics`, `create_log_record`, and error logging).

### F-27 Version identity drift in shipped metadata

- **Severity:** LOW. **Status:** VERIFIED. `Cargo.toml` 0.1.82, `CHANGELOG.md` top entry v0.1.82, `--version-only` 0.1.82; `THIRD_PARTY_LICENSES:2` says "for version 0.1.80"; `packaging/homebrew/metrumbench.rb:7` says `version "0.1.80"` (the release workflow regenerates it, so the checked-in copy is a template). No v0.1.79 changelog entry.

### F-28 Operator friction

- **Severity:** LOW. **Status:** VERIFIED. Exit code is 1 whenever any request failed (`llm.rs:1882`), so a 1/32 429 run fails a CI step; `debug.log` and `error.log` default into the working directory; `--concurrency` is mandatory in open loop; imagegen requires `--summary-json` while the others do not.

### F-29 Unique-prompt nonce repeats across runs

- **Severity:** LOW. **Status:** VERIFIED (`[nonce-{seq}]`, `src/args_common.rs:104`). A rerun against a warm server hits the prefix cache for every prompt. Fix: fold the seed and a run id into the nonce and record it.

---

## 5. Regression against the prior report

Prior findings are from `(removed from main; prior OSS readiness assessment)`; claimed status is
from `CHANGELOG.md` v0.1.80 to v0.1.82 (which never cites a finding ID, so the
mapping is by content). The reproduction column names the command in
Appendix A. Verdicts rest on behavior I ran, not on the presence of a test.

| ID | Claimed status | Reproduction | Observed | Verdict |
|---|---|---|---|---|
| A-01 no-output stream = success, TTFT 0 | v0.1.81 "role-only streams classified `no_output_token`" | A.5 `s3b_role_only`, `s3c2_reasoning_only` | 4/4 records `no_output_token`, `ttft_s.n = 0` in both cases | FIXED |
| A-02 split SSE loses events | v0.1.80 "complete-line SSE parser" | A.5 `s2_split_event`, `s2_utf8_split`, `s2_two_per_write` | 6 tokens, 5 ITL samples, correct TTFT in every case; only a multi-line `data:` event is dropped (F-23) | FIXED |
| A-03 no ITL, TPOT / N, VLM TTFT fabricated | v0.1.80 "ITL vs N-1 TPOT"; VLM non-streaming no longer fabricates | A.3 reference (`tpot_s.avg = 0.020215`, `itl_s.n = 304`), A.13 VLM non-streaming `ttft_s = null`, A.5 `s3e_nonstream` | ITL and N-1 TPOT correct; VLM null; **LLM** non-streaming still sets `ttft = latency` (F-08) | PARTIAL |
| A-04 two estimators, six copies, no small-n warning | v0.1.80 "Hyndman-Fan type 7" | A.3 (`percentile_method`, `n`, `p99_unreliable` present); legacy block in same file | v2 correct and flagged; legacy/console still nearest-rank `round` (LLM) and `ceil-1` (VLM/ASR) (F-05, F-25) | PARTIAL |
| A-05 window starts at arbitrary drain instant | no claim | A.3, A.9 | Window now collapses to collection-loop time; 2x to 190x inflation (F-01) | NOT FIXED |
| A-06 build broken, license expired | no claim | A.2 build; `grep check_license\|from_ymd_opt\|METRUM_SKIP` in `src/` = 0 | Builds on stable 1.97.1; no license gate | FIXED |
| A-07 closed loop only | v0.1.80 `--request-rate`/`--arrival` | A.8, A.5 `s5*` | Constant and Poisson schedules, `scheduled_offset_s`, `queue_delay_s` present and seeded | FIXED |
| A-08 drifted denominators | no claim | A.3 legacy vs v2 | v2 shared; legacy blocks remain with `(succ+err)/elapsed` (LLM) and `num_requests/elapsed` (VLM, code) | PARTIAL |
| A-09 no ignore_eos/min_tokens/seed/top_p; hidden system prompt | v0.1.80 `--ignore-eos`, `--extra-body-json` | A.6 bodies at server | All sent correctly by the LLM binary; none recorded; VLM ignores `--min-tokens`/`--system-prompt`; no sampling seed flag (F-03, F-10) | PARTIAL |
| A-10 no `--seed` | v0.1.80 `--seed` | A.8 | Identical seq-to-prompt map and Poisson offsets across two seed-7 runs; seed-8 differs | FIXED |
| A-11 prompts cycled, prefix-cache hits | v0.1.80 `--unique-prompts` | A.6 | Nonce prefix works; off by default; unrecorded; repeats across runs (F-29) | PARTIAL |
| A-12 TTFT includes connect; no warmup | v0.1.80 `--warmup-requests` | A.5 `s5c` (36 of 40 measured), A.13 VLM | Warmup excluded in LLM/ASR/imagegen; VLM drops the records from the file (F-10); connection setup still inside TTFT by design and undecomposed (F-13) | PARTIAL |
| A-13 usage missing -> 0 silently | no claim | A.5 `s3f`, `no_usage_stream` | `usage_missing: true` per record, but success with 0 tokens and `completion_tokens_per_second = 0.0` unflagged (F-11) | PARTIAL |
| A-14 missing `[DONE]` = error | v0.1.81 | A.5 `s3a_omit_done`, `s3g_finish_usage_close` | Both succeed with full token counts | FIXED |
| A-15 pooled mixture, per-endpoint lacks percentiles | no claim | A.5 `s8a_rr` | `pooled_mixture: true`, full per-endpoint distributions; but per-endpoint latency is the corrected value and wrong in closed loop (F-02) | PARTIAL |
| A-16 ASR normalizer, RTF inverted, mixed timing | v0.1.80/81 normalizer, `throughput.rtfx` | A.13 ASR | `config.normalizer` recorded, `rtfx_client` correct, server/client seconds separate; legacy `throughput.rtfx` differs (59.5 vs 39.8) and `rtf` mixes provenance (F-21) | PARTIAL |
| A-17 VLM preprocessing, JPEG re-encode | v0.1.81 bytes unchanged | A.13 proxy capture | 70-byte PNG byte-identical, `image_bytes = 70`, preload before window; preprocessing time not reported | FIXED |
| A-18 imagegen integer-ms wall clock | v0.1.80 monotonic | A.13 imagegen | `latency_ms = 102.428` (sub-ms, `Instant`) | FIXED |
| A-19 nothing persisted until end | v0.1.80 per-request JSONL, Ctrl-C | A.9 | 0 bytes on disk for the whole run; Ctrl-C drains and writes; SIGTERM/SIGKILL lose all; RSS +1.9 KB/request (F-04) | NOT FIXED |
| A-20 mandatory NTP gate | no claim | A.14 | Opt-in `--ntp-check`, offset 31 ms recorded in `environment`, never gates | FIXED |
| B-01 no std/CI/runs | no claim | A.3 (`std`, `mad`), A.11 `--runs 3` | Present; cross-run CI inherits the wrong rps (F-01) | FIXED |
| B-02 outlier disclosure | no claim | A.3 | Per-request records with `seq`/`phase` allow post-hoc detection; no outlier count | PARTIAL |
| B-03 no `n`, zero for undefined | no claim | A.3, `cargo test` `undefined_statistics_serialize_as_null_not_zero` | v2 carries `n` and nulls; legacy block still emits 0 | PARTIAL |
| B-04 scalar rates without dispersion | no claim | A.3 | `throughput_bins_rps` exists but is degenerate in closed loop (F-20) | PARTIAL |
| C-01 four drifted copies | no claim | Section 4.1 static pass | Core shared (`stats`, `sse`, `summary`, `record`, `load`); each binary still carries private `Metrics`, estimators, legacy writer, and its own window and warmup logic; the drift produced F-01/F-10/F-22 | PARTIAL |
| C-02 no library API | no claim | `src/lib.rs` | `RunSummary::from_records` is public and used by tests; no `run(config)` API; traits in `modality.rs`/`transport.rs` unused | PARTIAL |
| C-03 inconsistent CLI | v0.1.82 CLI.md regenerated | A.2 render diff (0 lines), A.13 imagegen 404 | Docs match `--help` exactly; `--request-timeout` 300 (LLM/imagegen) vs 120 (VLM/ASR); imagegen wants a base URL and `--summary-json` | PARTIAL |
| C-04 utilities, polars | no claim | `Cargo.toml`, `[[bin]]` list | Removed | FIXED |
| D-01 ASR NaN paths | no claim | not exercised | Legacy `errors.rate` is `errors/(succ+err)` with a zero-guard at `asr.rs:1064` per code read; `words_per_second` guarded at `:748` | UNDETERMINED (needs a zero-latency text-format server) |
| D-02 error type by string prefix | v0.1.81 typed `no_output_token` | A.5 `s4*` | Typed enum exists; 5xx/refused/reset/timeout collapse to `other`; legacy still `"Error type"` (F-06) | PARTIAL |
| D-03 no self-test, unredacted debug log | no claim | A.11 `selftest`, `environment` block | Present; VLM redacts bodies at debug level; LLM debug log still prints full request bodies | PARTIAL |
| D-04 SSE parser untested | v0.1.80/81 | `cargo test` (13 `sse` unit tests, 5 adversarial e2e), A.5 | Byte-level tests exist and behavior verified | FIXED |
| D-05 Ctrl-C loses run | v0.1.80/81 | A.9 `S9b`, `S9d` | 754 records + `partial: true`, exit 0, 1.0 s after SIGINT | FIXED |
| D-06 bookkeeping coupled to drain order | no claim | A.3 `completed_at`, F-01 | `completed_at` and the window are still taken in the drain loop | NOT FIXED |
| D-07 clippy/fmt | no claim | A.2 | Both exit 0 with `-D warnings`; CI gates them | FIXED |
| D-08 dead code | no claim | grep | `main.rs.txt`, `.docx`, `mcpserver/` gone; unused `modality.rs`/`transport.rs` traits and `--max-endpoint-failures` remain | PARTIAL |
| E-01 no test on a computed number | v0.1.80/81 e2e timing, WER table | A.2 (117 passed, 0 skipped with `METRUM_BENCH_REQUIRE_DUMMY=1`) | `core_measurement` golden summary, numpy-reference percentile test, WER table, dummy timing e2e all execute | FIXED |
| E-02 untestable structure | v0.1.82 `FakeClock` | `src/load.rs` | `FakeClock` exists and is used only by scheduler unit tests; binaries do not take a clock or transport | PARTIAL |
| F-01 license expiry gate | no claim | grep = 0; binaries run on 2026-09-15 | Gone | FIXED |
| F-02 SSH key, password, AWS id, IPs | no claim | A.0 greps over tracked files and full history | None of the listed strings present; gitleaks in CI with full history; `env.json` gitignored and absent from history | FIXED |
| F-03 NOTICE entries | no claim | `NOTICE`, `THIRD_PARTY_LICENSES`, `deny.toml` | Present, webpki-roots attributed; inventory labeled 0.1.80 (F-27) | FIXED |
| F-04 unshipped dataset | no claim | grep `prompt-library` in README/docs = 0 | Removed; `test-data/README.md` documents provenance | FIXED |
| F-05 platform coupling | no claim | grep restic/ECR; `schema_version` in every record type | Only a comment in `scripts/live/campaign.sh` about dropping legacy fields | FIXED |
| F-06 governance and CI | v0.1.81 CI e2e | tree, `.github/workflows` | LICENSE, NOTICE, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CODEOWNERS, headers enforced, fmt/clippy/test/gitleaks/coverage/licenses jobs | FIXED |
| F-07 Cargo metadata | no claim | `Cargo.toml`, CI `cargo package --allow-dirty` | Present; release workflow signs, attests, and emits SBOM | FIXED |
| H-01 metric definitions absent | no claim | `docs/METRICS.md` vs behavior | Exists; window definition and "headline open-loop latency" statement contradict the implementation (F-01, F-09); TTFT's inclusion of connection setup unstated | PARTIAL |
| H-02 README contradicts code | v0.1.82 CLI.md | A.2 render diff; `README.md:47` | CLI docs exact; "flushed incrementally" is false (F-04) | PARTIAL |
| H-03 not reproducible from repo | v0.1.80 partial | A.3 `docs/REPRODUCING.md` | Timing ranges reproduce; the summary's throughput does not; the config that produced the file is not in the file | PARTIAL |
| H-04 version identity | no claim | A.2 | `Cargo.toml`, `CHANGELOG.md`, `--version-only` agree on 0.1.82; `THIRD_PARTY_LICENSES` and the checked-in formula say 0.1.80; no v0.1.79 entry | PARTIAL |

Totals over 49 prior findings: FIXED 22, PARTIAL 23, NOT FIXED 3 (A-05, A-19,
D-06), UNDETERMINED 1 (D-01). Of the 33 the prior report marked as blocking,
A-05 and A-19 remain open and have grown into F-01 and F-04.

---

## 6. Published-results audit (`docs/SMOKE_RESULTS.md`)

Source of truth: `live-results/campaign-oss-20260915-smoke-rerun/` (gitignored,
present on this host). Method: `smoke_audit.py` (Appendix B.5) recomputed
type-7 p50/p95 of `latency_s` and `ttft_s` over measure-phase successes in
each cell's `results.jsonl` and compared to the document to 3 decimals.

| Row | Doc n / lat p50 / p95 / TTFT p50 / p95 | Recomputed | Match | Raw-file caveats |
|---|---|---|---|---|
| llm c1-n64 | 56 / 0.244 / 0.247 / 0.051 / 0.055 | 56 / 0.244 / 0.247 / 0.051 / 0.055 | yes | `window_seconds 0.493`, `requests_per_second 113.6` (wrong, F-01); `per_endpoint.latency_s.avg 8.978` vs pooled 0.244 (F-02); `p99_unreliable: true`; config lacks seed/warmup |
| llm c2-n64 | 56 / 0.257 / 0.258 / 0.067 / 0.068 | same | yes | window 0.517, rps 108.4; per-endpoint avg 4.769 |
| llm c4-n64 | 56 / 0.253 / 0.254 / 0.066 / 0.067 | same | yes | window 0.506, rps 110.6; per-endpoint avg 2.448 |
| llm c8-n64 | 56 / 0.252 / 0.253 / 0.066 / 0.067 | same | yes | window 0.504, rps 111.0; per-endpoint avg 1.306 |
| llm rate16-n64 | 56 / 0.257 / 0.267 / 0.070 / 0.080 | same | yes | open loop: `scheduled_offset_s` set, queue delay max 0.002 s, per-endpoint avg 0.258 (clean); window 0.528, rps 106.0 (wrong) |
| llm rate4-n64 | 56 / 0.243 / 0.245 / 0.051 / 0.054 | same | yes | window 0.489, rps 114.5 (wrong; offered rate was 4/s) |
| llm rate8-n64 | 56 / 0.258 / 0.263 / 0.068 / 0.073 | same | yes | window 0.537, rps 104.2 (wrong) |
| vlm c1-n32 | 24 / 0.285 / 0.345 / 0.053 / 0.057 | same | yes | 24 records for 32 requests (8 warmup dropped, F-10); window 9.13 (VLM uses whole-run elapsed), rps 2.63; per-endpoint avg 5.865 |
| vlm c2-n32 | 24 / 0.284 / 0.356 / 0.067 / 0.068 | same | yes | per-endpoint avg 3.058 |
| vlm c4-n32 | 24 / 0.292 / 0.385 / 0.066 / 0.068 | same | yes | per-endpoint avg 1.687 |
| asr c1-n8 (dummy) | 6 / 0.101 s / RTFx 39.76 / WER 1.0 | 6 / 0.1006 / 39.756 / 1.0 | yes | `CERTIFICATION.txt: dummy-certified`; config `normalizer: whisper-english`, model `whisper-dummy`, loopback URL |
| imagegen c1-n8 (dummy) | 6 images / 7.372 images/s / 101.255 ms p50 | `summary.json`: 6 / 7.3725 / 101.255 (`n = 6`, `p99_unreliable: true`) | yes | `duration_seconds 0.814` includes 2 warmup requests (F-22); no config block |

Other checks:

- **Units.** Stated in headers ("latency p50 (s)", "Latency p50 (ms)", "RTFx client", "Images/s"). The LLM/VLM tables give no unit in the header; values are seconds. Minor.
- **Sample sizes and flags.** `n` is shown. Every raw summary has `p99_unreliable: true`; the document shows p95 only, at `n = 56` (2.8 samples above p95) and `n = 24` (1.2 samples). The tool emits no p95 flag (F-24), so the document carries none. A reader should treat the VLM p95 values as maxima.
- **Dummy rows.** Labeled "dummy-certified" in their section headers and in `CERTIFICATION.txt`; they sit in separate tables under a separate heading. Adequate.
- **Provenance.** `sut.json` gives cloud, region, instance, GPU, driver, CUDA, vLLM version, image digest, torch; `command.txt` gives the exact CLI. The prompts file is retained. Missing for a rerun: vLLM launch flags (max-num-seqs, prefix caching, dtype/quant), and because F-03 the effective system prompt and `--seed 7 --warmup-requests 8` exist only in `command.txt`, not in the result files.
- **Counts.** "12 result files" matches. "490 measured request lines" does not reconcile: 528 v2 request lines (470 `measure`, 58 `warmup`) plus 8 imagegen v1 lines; measure-phase plus all imagegen = 478, plus warmup = 536.
- **Conclusion.** The published latency/TTFT/RTFx/WER/images-per-second numbers are exactly traceable and honestly labeled. The raw files behind them, which is what a buyer would request next, contain a wrong `requests_per_second` in all ten GPU cells and a wrong `per_endpoint.latency_s` in all seven closed-loop cells.

---

## 7. Differentiated value

Each subsection is a capability none of GenAI-Perf, vLLM `benchmark_serving.py`,
guidellm, LLMPerf, MLPerf LoadGen, or InferenceX offers in this form, judged
by what a persona can learn or catch.

### 7.1 Per-request queue delay under open-loop overload (Persona A)

- **Decision:** how much of user-visible latency is arrival queueing at the client cap versus service time, and whether the offered rate exceeded capacity.
- **Correct?** Yes in open loop: `queue_delay_s.max = 3.615 s`, `coordinated_omission_latency_s.max = 4.116 s`, `latency_s = 0.501 s` at `--request-rate 40` against 8 req/s capacity (A.5 `s5a`). Wrong in closed loop (F-02).
- **Demonstrated?** `tests/e2e_determinism.rs` and `tests/core_measurement.rs::coordinated_omission_includes_queue_delay`; no test drives an overloaded server.
- **To be trustworthy:** F-02, F-09 (name the headline, record the cap).

### 7.2 Reasoning time separated from visible TTFT (Persona B)

- **Decision:** whether a reasoning model's slow first visible token is prefill or thinking.
- **Correct?** Yes: `first_reasoning_s = 0.0513`, `ttft_s = 0.0615` against a dummy emitting reasoning 10 ms before content; reasoning-only streams are `no_output_token` (A.5 `s3c`, `s3c2`). GenAI-Perf counts the first delta of any kind.
- **Demonstrated?** `tests/e2e_adversarial.rs::reasoning_deltas_are_separated_from_ttft`.
- **To be trustworthy:** keep `first_reasoning_s` on failed records too (it is dropped on `no_output_token`).

### 7.3 Seeded, reproducible prompt order and Poisson schedule (Persona C)

- **Decision:** whether two runs from two vendors sent the same requests in the same order.
- **Correct?** Yes: identical seq-to-prompt map and offsets at the proxy across two seed-7 runs (A.8).
- **Demonstrated?** `tests/e2e_determinism.rs` checks offsets and phases; my proxy check adds prompt identity.
- **To be trustworthy:** record the seed (F-03) and make the unique-prompt nonce depend on it (F-29).

### 7.4 `pooled_mixture` with full per-endpoint distributions (Persona A, C)

- **Decision:** whether a fleet-level p99 is a mixture of a fast and a slow replica.
- **Correct?** Counting and labeling yes (`alive` 10/10, `dead` 0/10, `pooled_mixture: true`); the per-endpoint latency estimator is wrong in closed loop (F-02) and least-inflight routing is wrong with a fast-failing replica (F-12).
- **Demonstrated?** `summary.rs` unit test and my dead-replica run; no e2e with two live servers.

### 7.5 Byte-identical image payload with recorded size (Persona B)

- **Decision:** whether a VLM comparison is confounded by client-side re-encoding.
- **Correct?** Yes: 70 bytes in, 70 bytes at the proxy, `image_bytes = 70`, `image/png`, `detail: low` (A.13). GenAI-Perf and vLLM's bench re-encode or synthesize images.
- **To be trustworthy:** record `detail` and the resize/re-encode flags in the v2 summary (they are only in the legacy config) and report preprocessing time.

### 7.6 Knee detection on a measured throughput/latency curve (Persona A)

- **Decision:** the concurrency or rate at which to cap a replica.
- **Correct?** Yes on a server with a known knee: concurrency 4 and 40 req/s found exactly (A.12). The estimator always returns a knee for three or more points and has no sensitivity threshold (`strategic.rs:86-110`), so on a flat or noisy curve it will report noise.
- **Demonstrated?** `strategic.rs::knee_finds_curve_bend` on a synthetic 4-point curve; the mock server has no saturation model, so the e2e cannot check the knee.
- **To be trustworthy:** add the Kneedle sensitivity threshold and print the distance score; give the mock server a `--workers` capacity.

### 7.7 ASR: normalizer recorded, server and client inference seconds separate (Persona B)

- **Correct?** Yes (`config.normalizer`, `inference_seconds_server` vs `inference_seconds_client`, `rtfx_client`); undermined by a second aggregate RTFx and an unlabeled mixed `rtf` (F-21).

### 7.8 Imagegen artifact hashing (Persona C)

- **Correct?** Yes: SHA-256 per returned image verified against the file on disk (A.13). Lets a buyer prove the images were real and distinct. Hashing and file writes sit inside the timed window (F-22).

---

## 8. Roadmap (ordered by real-world value per engineer-week)

1. **Fix the throughput window and closed-loop queue delay (F-01, F-02, F-19; one pull request).** Persona A's capacity decision and Persona B's per-endpoint comparison. Two small changes in `llm.rs` (timestamp inside the task) and `summary.rs` (one latency definition, `None` schedule in closed loop), plus the acceptance test above, then regenerate `live-results`. Under one week including the other three binaries. The four per-binary window definitions are the same architectural failure as the unused `modality.rs`/`transport.rs` traits: a shared `RunSummary::from_records` cannot protect the result when each binary invents its own denominator, so the window must move into the shared core.
2. **Stamp the configuration into `summary.v2` and each record (F-03, F-09).** Persona C's ability to catch `ignore_eos`, hidden prompts, warmup, cap, and SLOs. One week.
3. **Flush records from the task; handle SIGTERM (F-04, F-18; one pull request).** Persona A's soak tests. Under one week, including the flush-during-run test above.
4. **Type the errors at the failure site and keep time-to-failure (F-06, F-07).** Persona A's error-storm analysis. Under one week.
5. **Retire the legacy summary and console estimators (F-05, F-25).** Persona C's single source of truth. One to two weeks; coordinate with consumers per `docs/OUTPUT_SCHEMA.md`.
6. **Connect and first-byte timestamps, `--ca-cert` (F-13).** Persona A behind a gateway. One week.
7. **VLM parity with the LLM binary (F-10).** Persona B. Under one week.
8. **Usage-missing accounting and tokenizer fallback (F-11).** Persona B against servers that omit `usage`. Under one week.
9. **Strategic exports: `n`, flags, config, MLPerf disclaimer, per-stage pooling (F-14, F-16).** Persona C. One week.
10. **Republish `docs/SMOKE_RESULTS.md` with throughput and reconciled counts (F-15).** After items 1, 2, 7.
11. **`cargo deny` in CI and dependency updates (F-17).** Half a day.
12. **Streaming multi-turn with per-turn TTFT in the strategic runner.** Persona B for chat workloads; currently non-streaming only. Two weeks.

**Not recommended (parity with no dependent decision):** dataset integrations (ShareGPT, sonnet) because prompt files already work; a Python API; gRPC/Triton transports; per-token trace export beyond `itl_s`; embedding/rerank sweeps beyond what exists; a TUI. Also not recommended: any change to the TTFT definition to exclude connection setup by default. The user pays it; keep it and decompose it.

---

## 9. Competitive context (Appendix C)

Gaps versus reference tools, with the persona decision that depends on each.
Rows marked "none" carry zero weight.

| Capability | GenAI-Perf | vLLM bench | guidellm | LLMPerf | MLPerf LoadGen | Metrum AI Bench at 858bb757 | Dependent decision |
|---|---|---|---|---|---|---|---|
| Open-loop Poisson arrivals | yes | yes | yes | no | yes (Server) | yes, seeded | A: capacity under realistic arrivals |
| Per-request queue delay recorded | no | no | partial | no | yes (scheduled vs issued) | yes (open loop) | A: attribute latency to client cap |
| Goodput vs SLO | yes | yes | no | no | yes | yes (LLM/VLM/ASR v2); strategic runner has no SLO | A: replicas for an SLO |
| Local tokenizer counts | yes | yes | yes | yes | n/a | optional feature, side-by-side fields | B: engines with different `usage` semantics |
| Reasoning vs visible TTFT | no | no | no | no | n/a | yes | B: reasoning models |
| Connect / first-byte decomposition | no | no | no | no | n/a | no | A: gateway vs server attribution |
| Private CA / insecure TLS | yes (via env) | yes | yes | yes | n/a | no | A: internal gateways |
| Multi-endpoint with per-endpoint stats | no | no | no | no | no | yes, labeled mixture | A, C: fleet vs replica |
| Sweep with knee | no (profile) | no | yes | no | no | yes, correct knee, weak labeling | A: cap selection |
| Multi-turn with history | no | partial (datasets) | no | no | no | non-streaming only, per-message turns | B: chat serving; none until streaming |
| Prefix-cache control | yes (synthetic prefixes) | yes (shared prefix datasets) | no | no | n/a | nonce on/off, unrecorded | C: cache-flattered results |
| Incremental record flush | yes | yes | yes | yes | yes | claimed, absent | A: soak tests |
| MLPerf-format output | no | no | no | no | authoritative | mimicry without disclaimer | C: none legitimately; risk of misuse |
| ASR WER with named normalizer | no | no | no | no | yes (Whisper task) | yes | B: ASR engines |
| Image bytes unchanged, size recorded | no | no | no | no | n/a | yes | B: VLM engines |
| HTML report | yes | no | yes | no | no | minimal, no `n` | none |
| Dataset integrations | yes | yes | yes | yes | yes | no | none (JSONL suffices) |
| OTLP export | yes | no | no | no | no | yes (feature-gated) | none |

---

## 10. Appendix A: commands, exit codes, and raw numbers

All commands were run from `/home/<user>/src/bench-cli` or the scratchpad
directory `/tmp/claude-1000/.../scratchpad` (abbreviated `$S`). Binaries are
the release build copied to `$S/bin/`. The Go dummy server was built once
with `go build -o $S/bin/dummy-model-server ./cmd/dummy-model-server`.

### A.0 Environment and secret hygiene

```
git rev-parse HEAD                      -> 858bb757e1ba33e6b61216d67e43311d13133b1a
grep -m1 ^version Cargo.toml            -> version = "0.1.82"
date -u                                 -> Tue Sep 15 03:04:50 PM UTC 2026
nproc / free -g / uname -r              -> 16 / 30 GiB / 6.8.0-139-generic
rustc --version                         -> rustc 1.97.1 (8bab26f4f 2026-07-14)
go version                              -> go1.26.4 linux/amd64
which socat                             -> not installed (Go/Python stand-ins used)
stat env.json                           -> 62 bytes, mode -rw-rw-r--, keys: ['SHADEFORM_API_KEY']
git ls-files --error-unmatch env.json   -> not tracked
git log --all -- env.json env.chetan 'env.*.json' create_release.env -> (empty)
python: value in `git log --all -p` output? -> False; in working tree files other than env.json? -> []
git grep -nE '(sk-[A-Za-z0-9]{10,}|AKIA[0-9A-Z]{16}|hf_[A-Za-z0-9]{20,}|Bearer [A-Za-z0-9._-]{20,}|api[_-]?key...)' -> 0 hits
grep -rn 'restic|ecr\.|<aws-account-id>|<password>|ssh-rsa AAAA...<REDACTED>|<public-ip>|<public-ip>|<backup-host>' (tracked, excluding prior report) -> only docs/CAMPAIGN.md:58 and scripts/live/campaign.sh:295-299 (comments about dropping legacy fields)
grep -rn 'check_license|from_ymd_opt|METRUM_SKIP_LICENSE|METRUM_SKIP_NTP' src/ -> 0
```

### A.1 Toolchain gates

| Command | Exit | Trimmed output |
|---|---|---|
| `cargo build --release` | 0 | `Finished release profile` |
| `METRUM_BENCH_REQUIRE_DUMMY=1 cargo test --all-targets --all-features` | 0 | 22 test binaries; 117 passed, 0 failed, 0 ignored; `grep -c skipping` = 0. Per binary: lib 66, metrum-ai-bench 2, imagegen 3, llm 10, strategic 1, cli_docs 3, cli_validation 10, core_measurement 6, e2e_adversarial 5, e2e_asr 2, e2e_determinism 2, e2e_dummy 1, e2e_imagegen 2, e2e_strategic 1, e2e_vlm 3 |
| `cargo clippy --all-targets --all-features -- -D warnings` | 0 | `Finished dev profile` |
| `cargo fmt --all -- --check` | 0 | (no output) |
| `cargo deny check` | 1 | `advisories FAILED, bans ok, licenses ok, sources ok`; RUSTSEC-2025-0058 custom_derive unmaintained; RUSTSEC-2026-0002 and RUSTSEC-2026-0253 lru unsound (upgrade to >=0.16.3 / >=0.18.2); RUSTSEC-2020-0071 time 0.1.45 segfault via `ntp v0.5.0 -> metrumbench v0.1.82` |
| `bash scripts/check_headers.sh` | 0 | `check_headers: ok` |
| `cd dummy-model-server && go vet ./... && go test ./...` | 0 / 0 | audio, config, images, limiter, openai ok; server, sse, vision have no test files |
| `bash scripts/render_cli_help.sh` then `git diff docs/CLI.md` | 0 | 0 lines of drift (file restored with `git checkout -- docs/CLI.md`) |
| `cargo package --list --allow-dirty` | 0 | ships docs/, packaging/, endpoints-4servers.yaml, README-METRUMBENCH-ASR.md; excludes dummy-model-server, scripts/live, OSS assessment |

### A.2 Dummy-server oracle (derived from `dummy-model-server/internal/openai/handler.go` and `sse/sse.go`)

For `-latency L -chunk-interval C` and a chat stream with `max_tokens = N`:
the handler sleeps `L` after reading the body, then for each of `N` tokens
sleeps `C` and writes one `data:` event with `delta.content = "."`, then a
finish chunk with `usage {completion_tokens: N}` and `data: [DONE]`. Headers
are flushed with the first write. Therefore:

| Metric | Expected | Reference run (A.3) measured |
|---|---|---|
| TTFT | `L + C` = 120 ms | 120.93 ms mean (min 120.68, max 121.77) |
| E2E | `L + N*C` = 500 ms | 505.01 ms mean (min 504.35, max 505.79) |
| ITL | `C` = 20 ms, `N-1` = 19 samples per request | 20.21 ms mean, n = 304 = 16 x 19 |
| TPOT | `(E2E - TTFT)/(N-1)` = 20 ms | 20.21 ms mean |
| Throughput at concurrency 4, 16 requests | 4 waves x ~0.505 s = 2.02 s; 7.92 req/s | tool 15.83 req/s over a 1.011 s window; legacy 7.92; wall 2.02 s |
| With `-reasoning`, `-latency 40ms -chunk-interval 10ms` | first reasoning 50 ms, TTFT 60 ms | 51.3 ms, 61.5 ms |
| `-role-only` | no visible token | `no_output_token` |
| `-omit-done` | finish chunk seen, no `[DONE]` | success |

### A.3 Reference reproduction (`docs/REPRODUCING.md`, exit 0, wall 2.02 s, RSS 6.1 MB)

```
target/release/metrum-ai-bench-llm --url http://127.0.0.1:18321/v1/chat/completions --api-key dummy \
  --scenario reference --num-requests 16 --concurrency 4 --prompts metrum-prompts.jsonl --mode chat \
  --streaming --model dummy --max-tokens 20 --seed 7 --warmup-requests 0 --data-log metrum-reference.jsonl --log-level error
lines: 16 request.v2, 1 summary.v2, 1 legacy
```

Field-by-field against `test-data/reference-result.json` (tolerances are the
document's ranges):

| Field | Expected | Observed | Verdict |
|---|---|---|---|
| attempted / successes / errors | 16 / 16 / 0 | 16 / 16 / 0 | match |
| ttft mean | 100-200 ms | 120.93 ms | match |
| latency mean | 420-650 ms | 505.01 ms | match |
| itl mean | 10-40 ms | 20.21 ms | match |
| percentile_method | hyndman_fan_type7 | hyndman_fan_type7 | match |
| (not in reference) requests_per_second | true 7.92 | 15.83 | F-01 |
| (not in reference) coordinated_omission_latency_s.avg | should equal latency in closed loop, 0.505 | 1.263 | F-02 |
| (not in reference) per_endpoint latency avg | 0.505 | 1.263 | F-02 |
| (not in reference) throughput_bins_rps | 16 requests / 2.02 s | `{n:1, p50:1.6}` | F-20 |

summary.v2 excerpt:
```
"window_seconds": 1.011043719, "requests_per_second": 15.825230600141834, "completion_tokens_per_second": 316.50461200283667,
"latency_s": {"n":16,"min":0.504346583,"max":0.505788333,"avg":0.50501209075,"std":0.000528354,"mad":0.000410858,"p50":0.504918619,"p90":0.505778700,"p95":0.505787649,"p99":0.505788196,"percentile_method":"hyndman_fan_type7","p99_unreliable":true},
"coordinated_omission_latency_s": {"n":16,"min":0.505779336,"max":2.020319688,"avg":1.262809038,...},
"ttft_s": {"n":16,"min":0.120683797,"max":0.12177068,"avg":0.120931526,...},
"tpot_s": {"n":16,"avg":0.020214767,...}, "itl_s": {"n":304,"avg":0.020212118,"p99_unreliable":false,...},
"throughput_bins_rps": {"n":1,"min":1.6,"max":1.6,"avg":1.6,"std":null,...}, "goodput": {"count":16,"requests_per_second":15.8252,"fraction_of_attempted":1.0,"thresholds_s":{}},
"pooled_mixture": false, "per_endpoint": {"127.0.0.1:18321": {"attempted":16,"successes":16,"errors":0,"latency_s":{"n":16,"avg":1.262809038,...}}},
"environment": {"architecture":"x86_64","cpu_cores":16,"hostname":"<hostname>","ntp_offset_ms":null,"os":"linux","package_version":"0.1.82","rustc_version":"1.97.1","server_model":"dummy","tls_backend":"rustls","tokio_worker_threads":16}, "partial": false
```
Legacy record excerpt:
```
"config": {"concurrency":4,"connect_timeout":30,"data_log":"metrum-reference.jsonl","debug_log":"debug.log","endpoint":"127.0.0.1:18321","endpoints":null,"error_log":"error.log","log_level":"error","max_tokens":20,"mode":"chat","model":"dummy","num_requests":16,"pool_idle_timeout":60,"prompts_file":"metrum-prompts.jsonl","ramp_up_seconds":null,"request_timeout":300,"scenario":"reference","stop_after_seconds":null,"streaming":true,"tcp_keepalive":60,"temperature":0.10000000149011612,"url":"http://127.0.0.1:18321/v1/chat/completions"}
"metrics.timing": {"failed_requests":0,"metrics_collection_seconds":2.020958336,"requests_per_second":7.917036049178601,"steady_state_requests_per_second":7.917036049178601,"successful_requests":16,"total_time_seconds":2.02099151}
"metrics.response_times": {"avg_ms":505,"max_ms":505,"min_ms":504,"p50_ms":504,"p90_ms":505,"p95_ms":505,"p99_ms":505}
"metrics.ttft": {"avg_ms":120,"max_ms":121,"min_ms":120,"p50_ms":120,"p90_ms":121,"p95_ms":121,"p99_ms":121}
```
Record timestamps (seq 0-3): `completed_at - started_at = 1.515 s`, `latency_s = 0.506`, `queue_delay_s = 0.000`, `scheduled_offset_s = 0.0`; seq 4-7: wall delta 1.009 s, latency 0.504, `queue_delay_s = 0.506`.
Console tail: `Requests/Second: 7.92 (successful: 16, failed: 0)`, `p50: 0.020217s, p90: 0.020230s, p95: 0.020230s, p99: 0.020238s` (TPOT, nearest-rank).

### A.4 Independent recomputation (`recompute.py`, Appendix B.1)

| Run | records / measured | tool window | true window (started_at + latency_s) | ratio | fields compared | mismatches | rps tool / true |
|---|---|---|---|---|---|---|---|
| reference (LLM, closed, C=4) | 16 / 16 | 1.011 | 2.020 | 0.500 | 133 | 0 | 15.83 / 7.92 |
| s8a_rr (2 endpoints, one dead) | 20 / 20 | 0.0526 | 0.0835 | 0.630 | 185 | 0 | 190.0 / 119.8 |
| s5a (open loop, rate 40, cap 4, 500 ms server) | 40 / 40 | 1.003 | 5.091 | 0.197 | 133 | 0 | 39.89 / 7.86 |
| s5c (open loop, cap 1000, warmup 4, SLOs) | 40 / 36 | 1.003 | 1.376 | 0.729 | 135 | 2 (`goodput.thresholds_s.*` not recomputable: SLOs are not in the records) | 35.91 / 26.16 |
| VLM streaming (C=2, warmup 2) | 6 / 6 | 1.214 | 0.910 | 1.333 | 133 | 0 | 4.94 / 6.59 |
| ASR (C=2, warmup 2) | 6 / 4 | 0.302 | 0.201 | 1.504 | 133 | 0 | 13.23 / 19.90 |
| long run (3000, C=8) | 3000 / 3000 | 0.0649 | 12.198 | 0.005 | 133 | 0 | 46249.5 / 245.95 |

Fields not recomputable from records alone: the window definition,
`throughput_bin_seconds`, SLO thresholds (only echoed when set), and
`environment`. Imagegen per-request records are `imagegen.request.v1` with
`latency_ms` and no phase, so the shared script cannot consume them; the v1
summary was checked by hand (A.13).

### A.5 Scenario battery (`scenarios_llm.sh`, Appendix B.4; rerun `scen2` after fixing a port bug in my script)

Base flags: `--mode chat --model dummy --api-key dummy --log-level error`;
4-line prompt file. Columns: records, errors_by_type, TTFT avg, latency avg,
notes.

| Scenario | Server | Observed |
|---|---|---|
| s3a_omit_done | dummy `-omit-done -chunk-interval 5ms` | 4 ok, ct=10, `ttft 0.0057 lat 0.0524` |
| s3b_role_only | dummy `-role-only` | 4 x `no_output_token`, `latency_s 0.0`, exit 1 |
| s3c_reasoning_then_content | dummy `-reasoning -latency 40ms -chunk-interval 10ms` | 4 ok; `first_reasoning_s 0.0513`, `ttft 0.0615` |
| s3c2_reasoning_only | adversarial `reasoning_only` | 4 x `no_output_token`; `first_reasoning_s` absent on failed records |
| s3d_no_usage_stream | dummy `-include-usage=false` | client forces `stream_options.include_usage: true`, dummy honors it; ct=10 (not a valid test of missing usage) |
| no_usage_stream | adversarial `no_usage_stream` | 3 ok, ct=0, pt=0, `usage_missing: true`, `itl_n 5`, summary `completion_tokens_per_second 0.0`, `tpot_s.n 0` |
| s3e_nonstream | dummy `-latency 50ms`, no `--streaming` | 4 ok, `ttft 0.0507 == lat 0.0507`, `tpot_n 0`, `itl_n 0` |
| s3f_nonstream_no_usage | adversarial | 4 ok, ct=0, `usage_missing: true`, ctps 0.0 |
| s3g_finish_usage_close | adversarial | 4 ok, ct=5, `ttft 0.0211` |
| s2_crlf | adversarial (CRLF, `: comment`, `id:`, `event:`) | 3 ok, ct=6, itl 5, ttft 0.0208 |
| s2_two_per_write | adversarial | 3 ok, ct=6, itl 5 |
| s2_split_event | adversarial (3 writes per event) | 3 ok, ct=6, itl 5, ttft 0.0250 |
| s2_utf8_split | adversarial (3-byte char split at offsets 1,2,3) | 3 ok, ct=6, itl 5 |
| s2_big64k | adversarial (64 KiB first delta) | 3 ok, ct=6, itl 5 |
| s2_multiline_data | adversarial (two `data:` lines, one JSON) | 3 ok, **itl 4** (5 of 6 tokens), **ttft 0.0411** (second token) |
| s2_proxy_buffered | adversarial (whole body in one write) | 3 ok, `ttft 0.1415 == lat 0.1416` (faithful to what the user sees) |
| s4a_503_nonstream | dummy `-error-rate 1 -seed 1` | 6 x `other`, exit 1, legacy `types {"Error type": 6}` |
| s4b_midstream_error | dummy `-error-rate 1` streaming | 6 x `api_error` |
| s4c_429_storm | dummy `-max-concurrency 1`, C=8, 32 req | 31 x `rate_limit`, 1 ok, `error_rate 0.969`, exit 1 |
| s4c2_429_all | adversarial 429 | 6 x `rate_limit` |
| s4c3_503_all_stream | adversarial 503 | 6 x `other` |
| s4d_conn_refused | `http://127.0.0.1:1` | 6 x `other`; error.log `Primary Error: error sending request for url (...)` |
| s4e_conn_reset | adversarial RST | 6 x `other` |
| s4f_slowloris | adversarial headers-only, `--request-timeout 2` | 4 x `other` after 4.0 s wall, `latency_s 0.0`, legacy type `stream error` |
| s4g_openloop_during_failure | dummy `-error-rate 1`, `--request-rate 20 --concurrency 100`, 40 req | 40 x `other` over 1.977 s (schedule kept: 40/20 = 2.0 s) |
| s5a_rate40_cap4 | dummy `-latency 500ms`, `--request-rate 40 --concurrency 4` | 40 ok, `latency avg 0.5012`, `co_latency max 4.1156`, `queue_delay max 3.615`, tool rps 39.89, legacy 7.86, wall 5.09 s |
| s5b_rate40_cap1000 | same, `--max-concurrency 1000` | `co_latency max 0.5030`, tool rps 39.90, legacy 27.08, wall 1.48 s |
| s5c + `--warmup-requests 4 --slo e2e=600ms --slo ttft=550ms` | same | attempted 36, goodput count 36, thresholds echoed |
| s6a_repeat_prompts / s6b_unique_prompts | dummy | bodies: `"Second prompt"` repeated vs `"[nonce-0] Second prompt"`, `"[nonce-1] Fourth one"`, ...; config has neither flag |
| s7a / s7b | dummy | s7b body: `"ignore_eos": true, "min_tokens": 5`, no system message, `top_p 0.5`, temperature 0.9; config unchanged |
| s8a_rr (round-robin, one dead endpoint) | dummy + `127.0.0.1:1` | `pooled_mixture true`, alive 10/10/0, dead 10/0/10, `errors_by_type {"other": 10}` |
| s8b_li (least-inflight) | same | alive 2/2/0, dead 18/0/18 |

### A.6 Bodies captured at the server (debug log at `--log-level debug`)

```
s7a default body:  {"max_tokens":5,"messages":[{"content":"You are a helpful assistant.","role":"system"},{"content":"Second prompt","role":"user"}],"model":"dummy","stream":true,"stream_options":{"include_usage":true},"temperature":0.1}
s7b body:          {"ignore_eos":true,"max_tokens":5,"messages":[{"content":"Second prompt","role":"user"}],"min_tokens":5,"model":"dummy","stream":true,"stream_options":{"include_usage":true},"temperature":0.9,"top_p":0.5}
plain proxy body:  proto HTTP/1.1, tls false, keys [max_tokens, messages, model, stream, stream_options, temperature], temperature 0.10000000149011612
```
Note: the debug log interleaves concurrent tasks, so it is not an order oracle; A.8 uses a proxy instead.

### A.7 Scenario 1: TLS and gateway (`proxy.go`, Appendix B.3; cert via `openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj /CN=localhost -addext subjectAltName=DNS:localhost,IP:127.0.0.1`)

```
S1a https://127.0.0.1:18443 (self-signed terminator -> dummy): exit 1; records {"kind":"other","message":"Streaming request failed"}, latency_s 0.0
     error.log: Caused by (level 2): invalid peer certificate: Other(OtherError(CaUsedAsEndEntity))
S1a2 same with SSL_CERT_FILE=cert.pem: exit 1, 2 x "other" (rustls-tls uses compiled-in webpki roots)
S1b plain proxy hop, C=4, 8 requests, cold pool: ttft 0.1234-0.1266, lat 0.5106-0.5153; record keys containing connect/byte/header: []
S1c proxy holding response headers 300 ms: ttft avg 0.4262 (direct 0.1209), latency avg 0.5103
```
Persona A reading S1c would conclude prefill takes 0.43 s; the truth is 0.12 s prefill plus a 0.30 s gateway hold, and nothing in the record distinguishes them.

### A.8 Determinism (proxy-captured bodies, `--unique-prompts --seed 7 --request-rate 30 --arrival poisson`, 15 requests, C=3, 5 prompts)

```
seq->prompt seed7 run a: [Bravo, Delta, Echo, Charlie, Alpha, Bravo, Delta, Echo, Charlie, Alpha, Bravo, Delta, Echo, Charlie, Alpha]
seq->prompt seed7 run b: identical (a==b True)
seq->prompt seed8 run c: [Delta, Bravo, Alpha, Echo, Charlie, ...] (a==c False)
poisson scheduled offsets identical a==b: True; a==c: False; first four (a): 0.0, 0.03457, 0.043468, 0.044112
arrival order at server by nonce, both runs: 0..14
```

### A.9 Scenario 9: long run and interruption (`longrun.sh`, dummy `-latency 20ms -chunk-interval 2ms`, `--max-tokens 5 --concurrency 8`)

```
S9a 3000 requests: samples t=0.0s rss 6884 KB log 0 B; t=2.0 rss 8576 log 0; t=4.1 rss 9812 log 0; t=6.1 rss 10824 log 0; t=8.2 rss 11692 log 0; t=10.2 rss 12584 log 0 (log stayed 0 bytes until exit)
     final: records 3000, attempted 3000, window 0.065 s, rps 46249.5, partial false, co_latency max 12.198, per_endpoint latency avg 6.089
S9b SIGINT at 3 s: lines before signal 0; process gone 1.0 s later; exit 0; 754 records, 0 unparseable, summary present, partial true, attempted 754
S9c SIGKILL at 3 s: lines before 0; exit 137; 0 records, no summary
S9d SIGINT during open loop (rate 200, cap 8): exited 0.50 s after signal; 601 records, partial true
SIGTERM at 2 s (separate run): exit 143; 0 lines
```

### A.10 Scenario 10: clock step (`clockstep.c`, Appendix B.6; `LD_PRELOAD`, `CLOCKSTEP_AFTER_MS=1000 CLOCKSTEP_JUMP_S=3600`, reference workload)

```
exit 0; started_at span across records 3601.5 s (wall clock stepped +3600 s mid-run)
latency_s min/max 0.5047 / 0.5070; ttft_s min/max 0.1206 / 0.1215; summary latency avg 0.50556
```
No interval metric moved (all are `Instant`-based). A consumer windowing on `started_at` would see a one-hour run.

### A.11 Operator checks

```
metrum-ai-bench --help          -> llm, vlm, asr, imagegen, selftest
metrum-ai-bench selftest        -> environment JSON (architecture, cpu_cores 16, hostname, ntp_offset_ms null, package_version 0.1.82, rustc 1.97.1, tls_backend rustls, tokio_worker_threads 16)
metrum-ai-bench llm -- --runs 3 ... --seed 3 (8 req, C=2, 50 ms + 10 x 5 ms dummy): exit 0; per-run rps 38.71 / 38.78 / 38.83 (windows 0.207 / 0.206 / 0.206; true about 20 req/s);
     cross-run.v1: requests_per_second {n 3, avg 38.77, std 0.586, p99_unreliable true}, ci95 percentile_bootstrap 10000 resamples [38.71, 38.83]
--ntp-check (METRUM_NTP_TIMEOUT=3): exit 0; "NTP clock offset: 31ms"; environment.ntp_offset_ms 31
exit code with 31 of 32 requests rate-limited: 1 ("Test completed with errors")
imagegen with --url .../v1/images/generations: 6 x http_error 404 (binary appends /images/generations to a base URL); with --url .../v1: success
imagegen without --summary-json: clap error "required arguments were not provided: --summary-json"
```

### A.12 Scenario 11/12: strategic runner against `saturating_server.py` (K=4 workers x 100 ms, capacity 40 req/s; Appendix B.7)

```
concurrency sweep 1,2,4,8,16; 40 requests per stage; --metrics-url .../metrics
  load=1  thr=9.87  p50=0.101 p95=0.102 p99=0.103
  load=2  thr=19.76 p50=0.101 p95=0.101 p99=0.102
  load=4  thr=39.44 p50=0.101 p95=0.102 p99=0.102   <- knee reported here (true knee)
  load=8  thr=39.34 p50=0.203 p95=0.204 p99=0.204
  load=16 thr=39.33 p50=0.406 p95=0.409 p99=0.409
  server_metrics {kv_cache_usage 1.0, preemptions 0.0, requests_running 4.0, requests_waiting 12.0}
rate sweep 10,20,40,80 req/s; 40 per stage
  load=10 thr=9.99 p95=0.103; load=20 thr=19.49 p95=0.103; load=40 thr=36.78 p95=0.111  <- knee; load=80 thr=38.13 p95=0.559
sessions run (2 sessions, --prefix-control unique --shared-prefix "You are terse." --json-schema schema.json, 12 requests, C=2)
  points[0]: validity_rate 1.0, goodput 19.79 == throughput 19.79; knee null (single stage)
  hand count from sess.csv: 12 rows, 12 success, 12 valid; turns 1..5; sessions s1, s2
  server saw per request: roles su / sua / suau / ... (every message prefix is a "turn", including assistant-terminated ones); system "[session:s1] You are terse." / "[session:s2] You are terse."
CSV header: seq,stage,endpoint,scheduled_unix_ns,sent_unix_ns,latency_s,queue_delay_s,service_latency_s,success,valid,input_tokens,output_tokens,session_id,turn,error   (200 rows for the concurrency sweep)
HTML: 1519 bytes, 0 <script>, 0 external href/src; table columns Load, Throughput, p95 seconds, Error, Goodput; no n, no p99, no config
mlperf_conc/mlperf_log_summary.txt:
  MLPerf Results Summary / SUT name : MetrumBench / Scenario : Server / Mode : PerformanceOnly
  Scheduled samples per second : 21.478056 / Completed samples per second : 21.478056
  50.00 percentile latency (ns) : 101483338 ... 99.90 percentile latency (ns) : 408839533
  Test Parameters Used / samples_per_query : 200 / duration (s) : 9.311830 / Result is : VALID
grep -il 'audit|official|not a|unofficial|disclaimer' mlperf_conc/* mlperf_rate/*  -> none
mlperf_log_detail.txt first line: :::MLLOG {"key":"sample","value":{"id":0,"scheduled_time_ns":...,"latency_ns":102745615,"success":true},"metadata":{"file":"metrumbench","lineno":0}}
```
No TTFT exists in the strategic path (non-streaming); per-turn latency is recoverable from the CSV `turn` column only.

### A.13 Scenario 13: modalities (`modalities.sh`; dummy `-latency 100ms -chunk-interval 20ms`)

```
VLM streaming, 8 req, C=2, --warmup-requests 2, test-data/tiny.png (70 bytes):
  records 6 (warmup 0, measure 6); last rec seq 7 ttft 0.1207 lat 0.3027 ct 10 itl_n 9 modality_metrics {image_bytes 70, image_count 1} queue_delay 0.9107
  v2: attempted 6, window 1.214, rps 4.94, latency avg 0.3033, co_latency max 1.2134, per_endpoint latency avg 0.9102
  legacy config keys: concurrency connect_timeout data_log debug_log endpoint endpoints error_log image_cache_size image_detail log_level max_image_dimension max_tokens model num_images_batch num_requests pool_idle_timeout prompts_file ramp_up_seconds reencode_jpeg request_timeout scenario server_side_download stop_after_seconds tcp_keepalive temperature url
VLM non-streaming: ttft_s null, tpot/itl empty, latency avg 0.1007
VLM body at proxy with --system-prompt "" --min-tokens 3 --ignore-eos --extra-body-json '{"top_p":0.5}':
  keys [ignore_eos, max_tokens, messages, model, stream, stream_options, temperature, top_p]; roles [system, user]; system "You are a helpful assistant capable of understanding images."; min_tokens absent; ignore_eos true; top_p 0.5; image data:image/png detail low, 70 bytes, byte-identical True
  (debug log at --log-level debug prints "Built request body: (redacted: contains image data)")
ASR, 6 req, C=2, warmup 2, ground truth for both ids, default verbose-json:
  records 6 (warmup 2, measure 4); last rec modality_metrics {cer 0.947, inference_seconds_client 0.10043, rtfx_client 39.83, wer 1.0}; v2 attempted 4, window 0.302, rps 13.23, completion_tokens_per_second 0.0
  legacy config normalizer whisper-english, response_format verbose_json; legacy accuracy wer {avg 1.0 ...} cer {avg 0.897 ...} (no n); legacy throughput {rtfx 59.466, total_audio_seconds 18.0, requests_per_second 19.82, words_per_second 39.64}
ASR --response-format json --normalizer none: modality_metrics has inference_seconds_server 0.1 (dummy returns inference_time only for json)
Imagegen --url http://127.0.0.1:19703/v1, 6 req, C=2, --warmup-requests 2, --size 64x64:
  requests 000001-000006 latency_ms 102.404-102.616 status success hash_verified True n_returned 1 bytes 540/590 mime image/png seed None
  v2 summary: attempted 4, rps 13.000, window 0.308, latency avg 0.10253, co_latency max 0.1026
  v1 summary.json: request_count 4, images_generated 4, images_per_second 12.988, duration 0.308, latency_ms {n 4, p50 102.542, p99_unreliable true}; config block present: False
```

### A.14 Published-results audit raw output (`smoke_audit.py`, Appendix B.5)

```
llm c1-n64  doc 56 0.244/0.247 0.051/0.055 | recomputed 56 0.244/0.247 0.051/0.055 -> MATCH | p99_unreliable True, window 0.4929, rps_tool 113.61 | config seed/warmup/request_rate/ignore_eos all absent | scheduled_offset set False, queue_delay max 15.451, latency avg 0.2444, co_latency avg 8.9777, per_endpoint latency avg 8.9777
llm c2-n64  MATCH | window 0.5167 rps 108.38 | queue_delay max 8.003, per_endpoint avg 4.7685
llm c4-n64  MATCH | window 0.5063 rps 110.60 | queue_delay max 3.855, per_endpoint avg 2.4476
llm c8-n64  MATCH | window 0.5043 rps 111.04 | queue_delay max 1.822, per_endpoint avg 1.3055
llm rate16  MATCH | window 0.5282 rps 106.02 | scheduled_offset True, queue_delay max 0.002, per_endpoint avg 0.2581
llm rate4   MATCH | window 0.4890 rps 114.53 | queue_delay max 0.002, per_endpoint avg 0.2446
llm rate8   MATCH | window 0.5372 rps 104.24 | queue_delay max 0.002, per_endpoint avg 0.2591
vlm c1-n32  MATCH | window 9.1347 rps 2.63 | queue_delay max 8.843, per_endpoint avg 5.8650; 24 records, 0 warmup (command: --warmup-requests 8 --num-requests 32)
vlm c2-n32  MATCH | window 4.7776 rps 5.02 | per_endpoint avg 3.0579
vlm c4-n32  MATCH | window 2.5605 rps 9.37 | per_endpoint avg 1.6865
asr c1-n8   doc n=6 lat p50 0.101 rtfx 39.76 wer 1.0 | recomputed n=6 0.1006 39.756 1.0 (wer samples 6)
imagegen c1-n8  doc 6 images, 7.372 images/s, 101.255 ms | summary.json images_generated 6, images_per_second 7.3725, latency_ms p50 101.255, n 6, p99_unreliable true, duration 0.8138, request_count 6
counts: 12 results.jsonl; 528 v2 request lines (470 measure, 58 warmup); 8 imagegen v1 lines; doc says 490 measured
CERTIFICATION.txt (asr, imagegen): dummy-certified
```

## 11. Appendix B: helper sources

All helpers carry the Metrum AI copyright header. They live in the assessment scratchpad and are reproduced here in full so the appendix is self-contained.

### B.1 `recompute.py` (independent recomputation of `summary.v2`)

```python
#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent recomputation of metrum-ai-bench summary.v2 from request.v2 lines.

Usage: recompute.py results.jsonl [--tol 1e-9]

Recomputes every summary.v2 field that is derivable from the per-request
records and prints a diff table.  Two windows are computed:

  window_tool   : the window_seconds the tool wrote (used to check the
                  tool's own arithmetic).
  window_true   : first measured send to last measured completion,
                  derived from started_at (wall) + latency_s, which is the
                  only send/complete pair the record carries that is not
                  corrupted by collection-time timestamps.

Percentiles: Hyndman-Fan type 7 (numpy 'linear').
"""
import json
import math
import sys
from datetime import datetime


def q7(sorted_vals, p):
    n = len(sorted_vals)
    if n == 0:
        return None
    if n == 1:
        return sorted_vals[0]
    h = (n - 1) * p / 100.0
    lo = math.floor(h)
    hi = math.ceil(h)
    if lo == hi:
        return sorted_vals[lo]
    w = h - lo
    return sorted_vals[lo] * (1 - w) + sorted_vals[hi] * w


def dist(vals):
    s = sorted(v for v in vals if v is not None and math.isfinite(v))
    n = len(s)
    if n == 0:
        return {"n": 0, "min": None, "max": None, "avg": None, "std": None, "mad": None,
                "p50": None, "p90": None, "p95": None, "p99": None, "p99_unreliable": True}
    avg = sum(s) / n
    std = math.sqrt(sum((x - avg) ** 2 for x in s) / (n - 1)) if n > 1 else None
    med = q7(s, 50)
    mad = q7(sorted(abs(x - med) for x in s), 50)
    return {"n": n, "min": s[0], "max": s[-1], "avg": avg, "std": std, "mad": mad,
            "p50": q7(s, 50), "p90": q7(s, 90), "p95": q7(s, 95), "p99": q7(s, 99),
            "p99_unreliable": n * 0.01 < 1.0}


def iso(ts):
    return datetime.fromisoformat(ts.replace("Z", "+00:00")).timestamp()


def tpot(r):
    if r.get("ttft_s") is None:
        return None
    gen = max(r["latency_s"] - r["ttft_s"], 0.0)
    ct = r.get("completion_tokens", 0)
    if ct <= 1 or gen == 0.0:
        return None
    return gen / (ct - 1)


def summarize(records, window, slos, bin_s):
    measured = [r for r in records if r.get("phase") == "measure"]
    ok = [r for r in measured if r.get("error") is None]
    bad = [r for r in measured if r.get("error") is not None]
    ebt = {}
    for r in bad:
        k = r["error"]["kind"]
        ebt[k] = ebt.get(k, 0) + 1
    corrected = [r["latency_s"] + r.get("queue_delay_s", 0.0) for r in ok]
    itl = [x for r in ok for x in r.get("itl_s", [])]
    ct = sum(r.get("completion_tokens", 0) for r in ok)
    w = window if window > 0 else 1e-9

    def meets(r):
        if "e2e" in slos and r["latency_s"] + r.get("queue_delay_s", 0.0) > slos["e2e"]:
            return False
        if "ttft" in slos and (r.get("ttft_s") is None or r["ttft_s"] > slos["ttft"]):
            return False
        if "tpot" in slos and (tpot(r) is None or tpot(r) > slos["tpot"]):
            return False
        return True

    good = [r for r in ok if meets(r)]
    # throughput bins as the tool defines them (by scheduled_offset_s)
    count = max(1, math.ceil(w / bin_s)) if bin_s > 0 else 0
    bins = [0] * count
    any_sched = False
    for r in ok:
        so = r.get("scheduled_offset_s")
        if so is not None:
            any_sched = True
            idx = int(so // bin_s)
            if idx < count:
                bins[idx] += 1
    if ok and (not any_sched or all(b == 0 for b in bins)):
        bins_rps = [len(ok) / w]
    else:
        bins_rps = [b / bin_s for b in bins]
    per_ep = {}
    for r in measured:
        per_ep.setdefault(r["endpoint"], []).append(r)
    pe = {}
    for name, rows in per_ep.items():
        s_ok = [r for r in rows if r.get("error") is None]
        pe[name] = {
            "attempted": len(rows), "successes": len(s_ok), "errors": len(rows) - len(s_ok),
            "latency_s(corrected, as tool)": dist([r["latency_s"] + r.get("queue_delay_s", 0.0) for r in s_ok]),
            "latency_s(uncorrected)": dist([r["latency_s"] for r in s_ok]),
            "ttft_s": dist([r.get("ttft_s") for r in s_ok if r.get("ttft_s") is not None]),
            "tpot_s": dist([tpot(r) for r in s_ok if tpot(r) is not None]),
            "itl_s": dist([x for r in s_ok for x in r.get("itl_s", [])]),
        }
    return {
        "attempted": len(measured), "successes": len(ok), "errors": len(bad),
        "error_rate": (len(bad) / len(measured)) if measured else 0.0,
        "errors_by_type": ebt,
        "requests_per_second": len(ok) / w,
        "completion_tokens_per_second": ct / w,
        "latency_s": dist([r["latency_s"] for r in ok]),
        "coordinated_omission_latency_s": dist(corrected),
        "ttft_s": dist([r.get("ttft_s") for r in ok if r.get("ttft_s") is not None]),
        "tpot_s": dist([tpot(r) for r in ok if tpot(r) is not None]),
        "itl_s": dist(itl),
        "throughput_bins_rps": dist(bins_rps),
        "goodput": {"count": len(good), "requests_per_second": len(good) / w,
                    "fraction_of_attempted": (len(good) / len(measured)) if measured else 0.0},
        "pooled_mixture": len(pe) > 1,
        "per_endpoint": pe,
    }


def flatten(prefix, v, out):
    if isinstance(v, dict):
        for k, x in v.items():
            flatten(f"{prefix}.{k}" if prefix else k, x, out)
    else:
        out[prefix] = v


def main():
    path = sys.argv[1]
    tol = 1e-9
    if "--tol" in sys.argv:
        tol = float(sys.argv[sys.argv.index("--tol") + 1])
    records, summaries, legacy = [], [], []
    for line in open(path):
        line = line.strip()
        if not line:
            continue
        d = json.loads(line)
        sv = d.get("schema_version", "")
        if "request.v" in sv:
            records.append(d)
        elif "summary.v" in sv:
            summaries.append(d)
        elif "config" in d and "metrics" in d:
            legacy.append(d)
    if not summaries:
        print("NO summary.v2 line found; records:", len(records))
        sys.exit(2)
    tool = summaries[-1]
    slos = {k: v for k, v in tool.get("goodput", {}).get("thresholds_s", {}).items()}
    # bin width is NOT in the record; assume the CLI default (10s) unless overridden
    bin_s = float(sys.argv[sys.argv.index("--bin") + 1]) if "--bin" in sys.argv else 10.0
    measured = [r for r in records if r.get("phase") == "measure"]
    sends = [iso(r["started_at"]) for r in measured]
    ends = [iso(r["started_at"]) + r["latency_s"] for r in measured if r.get("error") is None]
    window_true = (max(ends) - min(sends)) if sends and ends else 0.0
    all_sends = [iso(r["started_at"]) for r in records]
    all_ends = [iso(r["started_at"]) + r["latency_s"] for r in records]
    window_all = (max(all_ends) - min(all_sends)) if records else 0.0
    print(f"records={len(records)} measured={len(measured)} summaries={len(summaries)} legacy={len(legacy)}")
    print(f"window_tool={tool['window_seconds']:.6f}s  window_true(started_at+latency, measured)={window_true:.6f}s  window_all_phases={window_all:.6f}s  ratio tool/true={tool['window_seconds']/window_true if window_true else float('nan'):.3f}")
    mine_toolwin = summarize(records, tool["window_seconds"], slos, bin_s)
    mine_truewin = summarize(records, window_true, slos, bin_s)
    ft, fm = {}, {}
    flatten("", {k: v for k, v in tool.items() if k not in ("environment", "schema_version", "partial", "window_seconds")}, ft)
    flatten("", mine_toolwin, fm)
    mismatches = []
    for k in sorted(ft):
        if k.endswith("percentile_method"):
            continue
        kk = k.replace("per_endpoint", "per_endpoint")
        # per_endpoint latency in the tool is corrected; map to our labelled key
        mk = kk
        if ".latency_s." in kk and kk.startswith("per_endpoint"):
            mk = kk.replace(".latency_s.", ".latency_s(corrected, as tool).")
        if mk not in fm:
            mismatches.append((k, ft[k], "<not recomputable>"))
            continue
        a, b = ft[k], fm[mk]
        if isinstance(a, (int, float)) and isinstance(b, (int, float)) and not isinstance(a, bool):
            if not math.isclose(a, b, rel_tol=tol, abs_tol=tol):
                mismatches.append((k, a, b))
        elif a != b:
            mismatches.append((k, a, b))
    print(f"\nFields compared (tool window): {len(ft)}; mismatches: {len(mismatches)}")
    for k, a, b in mismatches:
        print(f"  MISMATCH {k}: tool={a} mine={b}")
    print("\nHeadline rates recomputed over the TRUE window (first measured send -> last measured completion):")
    print(f"  requests_per_second: tool={tool['requests_per_second']:.4f}  true-window={mine_truewin['requests_per_second']:.4f}")
    print(f"  completion_tokens_per_second: tool={tool['completion_tokens_per_second']:.3f}  true-window={mine_truewin['completion_tokens_per_second']:.3f}")
    print(f"  goodput.requests_per_second: tool={tool['goodput']['requests_per_second']:.4f}  true-window={mine_truewin['goodput']['requests_per_second']:.4f}")
    if legacy:
        lt = legacy[-1]["metrics"].get("timing", {})
        print(f"  legacy record requests_per_second={lt.get('requests_per_second')} over metrics_collection_seconds={lt.get('metrics_collection_seconds')} (denominator = success+errors)")
    qd = [r.get("queue_delay_s", 0.0) for r in measured if r.get("error") is None]
    if qd:
        print(f"\nqueue_delay_s over measured successes: max={max(qd):.4f} mean={sum(qd)/len(qd):.4f} nonzero={sum(1 for x in qd if x > 0)}/{len(qd)}")
    print("Fields NOT recomputable from records alone: window_seconds (definition), throughput_bin_seconds (not recorded), SLO thresholds (only present if set), environment.")


if __name__ == "__main__":
    main()
```

### B.2 `adversarial_sse.py` (raw-socket server for framing, error, and variant scenarios)

```python
#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Raw-socket adversarial OpenAI-compatible server for metrum-ai-bench review.

Usage: adversarial_sse.py PORT MODE [TOKENS] [DELAY_MS]

Modes (each request gets the same treatment):
  crlf            : SSE with CRLF line endings, `id:` and `event:` lines, and `: comment` lines
  two_per_write   : two complete events written in one TCP write
  split_event     : every event split across 3 TCP writes at arbitrary byte offsets
  utf8_split      : a 3-byte UTF-8 char split at byte offset (request counter mod 3)+1
  big64k          : first content delta is a 64 KiB string
  multiline_data  : spec-compliant event with two `data:` lines forming one JSON payload
  no_usage_stream : stream never sends `usage`; still sends finish + [DONE]
  finish_usage_close : finish chunk with usage, then TCP close without [DONE]
  reasoning_only  : only reasoning_content deltas, then finish + usage + [DONE]
  nonstream_no_usage : non-streaming JSON with no `usage` block
  http429         : always 429 with Retry-After
  http503         : always 503
  reset           : accept, read request, close socket without response
  slowloris       : send 200 headers, then nothing (hold until client closes)
  proxy_buffered  : whole SSE body written in one write after all delays (a buffering gateway)
The server logs every request body as JSON lines to stdout (for seed / prefix / ignore_eos checks).
"""
import json
import socket
import sys
import threading
import time

PORT = int(sys.argv[1])
MODE = sys.argv[2]
TOKENS = int(sys.argv[3]) if len(sys.argv) > 3 else 5
DELAY = (int(sys.argv[4]) / 1000.0) if len(sys.argv) > 4 else 0.02
counter = 0
lock = threading.Lock()


def chunk(delta, finish=None, usage=None, obj="chat.completion.chunk"):
    d = {"id": "x", "object": obj, "created": 0, "model": "adv",
         "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}
    if usage is not None:
        d["usage"] = usage
    return json.dumps(d, ensure_ascii=False)


def read_request(conn):
    conn.settimeout(10)
    buf = b""
    while b"\r\n\r\n" not in buf:
        part = conn.recv(65536)
        if not part:
            return None, None
        buf += part
    head, _, body = buf.partition(b"\r\n\r\n")
    length = 0
    for line in head.split(b"\r\n"):
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":")[1])
    while len(body) < length:
        part = conn.recv(65536)
        if not part:
            break
        body += part
    return head.decode("latin1"), body


def send_all(conn, data):
    conn.sendall(data)


def handle(conn, addr):
    global counter
    try:
        head, body = read_request(conn)
        if head is None:
            return
        with lock:
            counter += 1
            my = counter
        try:
            print(json.dumps({"n": my, "body": json.loads(body.decode("utf-8"))}), flush=True)
        except Exception:
            print(json.dumps({"n": my, "raw": body.decode("utf-8", "replace")}), flush=True)
        usage = {"prompt_tokens": 7, "completion_tokens": TOKENS, "total_tokens": 7 + TOKENS}
        sse_head = (b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n"
                    b"Cache-Control: no-cache\r\nConnection: close\r\n\r\n")
        if MODE == "http429":
            b = b'{"error":{"message":"rate limit exceeded","type":"rate_limit_error"}}'
            send_all(conn, b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1\r\nContent-Type: application/json\r\nContent-Length: %d\r\nConnection: close\r\n\r\n" % len(b) + b)
            return
        if MODE == "http503":
            b = b"service unavailable"
            send_all(conn, b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: %d\r\nConnection: close\r\n\r\n" % len(b) + b)
            return
        if MODE == "reset":
            conn.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, b"\x01\x00\x00\x00\x00\x00\x00\x00")
            conn.close()
            return
        if MODE == "slowloris":
            send_all(conn, sse_head)
            conn.settimeout(600)
            try:
                while conn.recv(1):
                    pass
            except Exception:
                pass
            return
        if MODE == "nonstream_no_usage":
            time.sleep(DELAY)
            b = json.dumps({"id": "x", "object": "chat.completion", "created": 0, "model": "adv",
                            "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hello there friend."}, "finish_reason": "stop"}]}).encode()
            send_all(conn, b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: %d\r\nConnection: close\r\n\r\n" % len(b) + b)
            return
        send_all(conn, sse_head)
        nl = b"\r\n" if MODE == "crlf" else b"\n"

        def ev(payload, extra_lines=b""):
            return extra_lines + b"data: " + payload.encode("utf-8") + nl + nl

        if MODE == "proxy_buffered":
            out = b""
            time.sleep(DELAY)
            for i in range(TOKENS):
                time.sleep(DELAY)
                out += ev(chunk({"content": "."}))
            out += ev(chunk({}, "stop", usage)) + b"data: [DONE]" + nl + nl
            send_all(conn, out)
            return
        if MODE == "reasoning_only":
            for i in range(TOKENS):
                time.sleep(DELAY)
                send_all(conn, ev(chunk({"reasoning_content": "hmm"})))
            send_all(conn, ev(chunk({}, "stop", usage)) + b"data: [DONE]" + nl + nl)
            return
        if MODE == "multiline_data":
            time.sleep(DELAY)
            payload = chunk({"content": "hello"})
            cut = len(payload) // 2
            send_all(conn, b"data: " + payload[:cut].encode() + nl + b"data: " + payload[cut:].encode() + nl + nl)
            for i in range(TOKENS - 1):
                time.sleep(DELAY)
                send_all(conn, ev(chunk({"content": "."})))
            send_all(conn, ev(chunk({}, "stop", usage)) + b"data: [DONE]" + nl + nl)
            return
        time.sleep(DELAY)
        if MODE == "two_per_write":
            # exactly TOKENS content deltas, written two events per TCP write
            i = 0
            while i < TOKENS:
                if i > 0:
                    time.sleep(DELAY)
                pair = ev(chunk({"content": "."}))
                if i + 1 < TOKENS:
                    pair += ev(chunk({"content": "."}))
                send_all(conn, pair)
                i += 2
            send_all(conn, ev(chunk({}, "stop", usage)) + b"data: [DONE]" + nl + nl)
            return
        for i in range(TOKENS):
            if i > 0:
                time.sleep(DELAY)
            if MODE == "big64k" and i == 0:
                content = "x" * 65536
            elif MODE == "utf8_split":
                content = "你"
            else:
                content = "."
            extra = b""
            if MODE == "crlf":
                extra = b": keepalive" + nl + b"id: %d" % i + nl + b"event: message" + nl
            e = ev(chunk({"content": content}), extra)
            if MODE == "two_per_write" and i + 1 < TOKENS:
                # emit pairs
                e2 = ev(chunk({"content": "."}))
                send_all(conn, e + e2)
                i += 1
                continue
            if MODE == "split_event":
                a, b2 = len(e) // 3, 2 * len(e) // 3
                send_all(conn, e[:a]); time.sleep(0.002)
                send_all(conn, e[a:b2]); time.sleep(0.002)
                send_all(conn, e[b2:])
                continue
            if MODE == "utf8_split":
                idx = e.find("你".encode("utf-8"))
                off = idx + (my % 3) + 1
                send_all(conn, e[:off]); time.sleep(0.003)
                send_all(conn, e[off:])
                continue
            send_all(conn, e)
        if MODE == "two_per_write":
            pass
        fin = ev(chunk({}, "stop", None if MODE == "no_usage_stream" else usage))
        if MODE == "finish_usage_close":
            send_all(conn, fin)
            return
        send_all(conn, fin + b"data: [DONE]" + nl + nl)
    except Exception as e:
        print(json.dumps({"error": str(e)}), flush=True)
    finally:
        try:
            conn.close()
        except Exception:
            pass


def main():
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", PORT))
    srv.listen(512)
    print(json.dumps({"listening": PORT, "mode": MODE}), flush=True)
    while True:
        conn, addr = srv.accept()
        threading.Thread(target=handle, args=(conn, addr), daemon=True).start()


if __name__ == "__main__":
    main()
```

### B.3 `proxy.go` (body-logging reverse proxy with optional TLS termination and header delay)

```go
// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0
//
// Body-logging reverse proxy with optional TLS termination, used to observe
// what metrum-ai-bench actually sends (seed order, nonce prefixes, ignore_eos)
// and to place a TLS handshake in the request path.
//
//   go run proxy.go -listen :18443 -target http://127.0.0.1:18321 -cert cert.pem -key key.pem -log bodies.jsonl
//   go run proxy.go -listen :18080 -target http://127.0.0.1:18321 -log bodies.jsonl        (plain)
//   -delay-headers 300ms : hold the upstream response headers for 300ms (a slow gateway)
package main

import (
	"bytes"
	"encoding/json"
	"flag"
	"io"
	"log"
	"net/http"
	"net/http/httputil"
	"net/url"
	"os"
	"sync"
	"sync/atomic"
	"time"
)

func main() {
	listen := flag.String("listen", ":18080", "listen address")
	target := flag.String("target", "http://127.0.0.1:18321", "upstream base URL")
	cert := flag.String("cert", "", "TLS cert PEM (enables TLS)")
	key := flag.String("key", "", "TLS key PEM")
	logPath := flag.String("log", "bodies.jsonl", "request body log (JSONL)")
	delayHeaders := flag.Duration("delay-headers", 0, "hold upstream response headers for this long")
	flag.Parse()

	u, err := url.Parse(*target)
	if err != nil {
		log.Fatal(err)
	}
	f, err := os.Create(*logPath)
	if err != nil {
		log.Fatal(err)
	}
	var mu sync.Mutex
	var seq int64
	rp := httputil.NewSingleHostReverseProxy(u)
	rp.FlushInterval = -1
	if *delayHeaders > 0 {
		rp.ModifyResponse = func(r *http.Response) error { time.Sleep(*delayHeaders); return nil }
	}
	h := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		n := atomic.AddInt64(&seq, 1)
		body, _ := io.ReadAll(r.Body)
		r.Body = io.NopCloser(bytes.NewReader(body))
		var parsed any
		_ = json.Unmarshal(body, &parsed)
		rec := map[string]any{"n": n, "t": time.Now().UnixNano(), "path": r.URL.Path, "proto": r.Proto, "tls": r.TLS != nil, "body": parsed}
		b, _ := json.Marshal(rec)
		mu.Lock()
		f.Write(b)
		f.Write([]byte("\n"))
		f.Sync()
		mu.Unlock()
		rp.ServeHTTP(w, r)
	})
	srv := &http.Server{Addr: *listen, Handler: h}
	log.Printf("proxy listening on %s -> %s tls=%v", *listen, *target, *cert != "")
	if *cert != "" {
		log.Fatal(srv.ListenAndServeTLS(*cert, *key))
	}
	log.Fatal(srv.ListenAndServe())
}
```

### B.4 `scenarios_llm.sh` (scenario battery; the `p=$P` form is the corrected version)

```bash
#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
# Scenario battery for metrum-ai-bench-llm against the Go dummy server and the
# Python adversarial server. Each scenario writes to $OUT/<name>/.
set -u
S=/tmp/claude-1000/-home-cgadgil-src-bench-cli/4d3b7b21-f028-418b-9578-a4c2900e1257/scratchpad
BIN=$S/bin/metrum-ai-bench-llm
DUMMY=$S/bin/dummy-model-server
ADV=$S/tools/adversarial_sse.py
OUT=$S/scen2
mkdir -p $OUT
PORT=19100
pids=()
cleanup() { for p in "${pids[@]:-}"; do kill $p 2>/dev/null; done; }
trap cleanup EXIT

start_dummy() { # name args...
  local name=$1; shift
  PORT=$((PORT+1)); local port=$PORT
  $DUMMY -port $port "$@" > $OUT/$name.dummy.log 2>&1 & pids+=($!)
  for i in $(seq 1 50); do curl -s -o /dev/null http://127.0.0.1:$port/v1/models && break; sleep 0.1; done
  P=$port
}
start_adv() { # name mode tokens delay_ms
  local name=$1 mode=$2 tokens=${3:-5} delay=${4:-20}
  PORT=$((PORT+1)); local port=$PORT
  python3 $ADV $port $mode $tokens $delay > $OUT/$name.adv.log 2>&1 & pids+=($!)
  sleep 0.4
  P=$port
}
run() { # name url extra-args...
  local name=$1 url=$2; shift 2
  local d=$OUT/$name; mkdir -p $d
  printf '%s\n' '{"prompt":"Hi there"}' '{"prompt":"Second prompt"}' '{"prompt":"Third"}' '{"prompt":"Fourth one"}' > $d/prompts.jsonl
  echo "$BIN --url $url --api-key dummy --scenario $name --prompts $d/prompts.jsonl --mode chat --model dummy --data-log $d/out.jsonl --debug-log $d/debug.log --error-log $d/error.log $*" > $d/command.txt
  ( cd $d && /usr/bin/time -f 'wall=%e rss_kb=%M' $BIN --url $url --api-key dummy --scenario $name --prompts $d/prompts.jsonl --mode chat --model dummy --data-log $d/out.jsonl --debug-log $d/debug.log --error-log $d/error.log "$@" > stdout.txt 2> stderr.txt; echo "exit=$?" >> stderr.txt )
  echo "== $name: $(tail -2 $d/stderr.txt | tr '\n' ' ')"
  python3 - "$d/out.jsonl" <<'EOF'
import json,sys
recs=[];summ=None;legacy=None
for l in open(sys.argv[1]):
    d=json.loads(l); sv=d.get('schema_version','')
    if 'request.v' in sv: recs.append(d)
    elif 'summary.v' in sv: summ=d
    elif 'config' in d: legacy=d
print(f"   records={len(recs)} ok={sum(1 for r in recs if r.get('error') is None)} errors={[ (r['seq'],r['error']['kind'],r['latency_s']) for r in recs if r.get('error')][:6]}")
if summ: print(f"   summary: attempted={summ['attempted']} succ={summ['successes']} err={summ['errors']} rate={summ['error_rate']:.3f} by_type={summ['errors_by_type']} window={summ['window_seconds']:.3f} rps={summ['requests_per_second']:.2f} ctps={summ['completion_tokens_per_second']:.1f} ttft_n={summ['ttft_s']['n']} ttft_avg={summ['ttft_s']['avg']} lat_avg={summ['latency_s']['avg']} co_lat_max={summ['coordinated_omission_latency_s']['max']} tpot_n={summ['tpot_s']['n']} itl_n={summ['itl_s']['n']} partial={summ['partial']} pooled={summ['pooled_mixture']} per_ep={ {k:(v['attempted'],v['successes'],v['errors'],v['latency_s']['avg']) for k,v in summ['per_endpoint'].items()} }")
else: print("   NO summary.v2")
if recs:
    r=recs[0]; print(f"   rec0: ttft={r.get('ttft_s')} first_reasoning={r.get('first_reasoning_s')} lat={r['latency_s']:.4f} ct={r['completion_tokens']} pt={r['prompt_tokens']} usage_missing={r['usage_missing']} itl_n={len(r.get('itl_s',[]))} qd={r['queue_delay_s']:.4f} so={r.get('scheduled_offset_s')} tok={r.get('tokenized_completion_tokens')}")
if legacy:
    c=legacy['config']; print(f"   legacy.config has ignore_eos={ 'ignore_eos' in c } unique_prompts={ 'unique_prompts' in c } system_prompt={ 'system_prompt' in c } seed={ 'seed' in c } warmup={ 'warmup_requests' in c } request_rate={ 'request_rate' in c } max_concurrency={ 'max_concurrency' in c } ; legacy rps={legacy['metrics']['timing']['requests_per_second']:.2f} err_rate_pct={legacy['metrics']['errors']['rate']:.1f} types={legacy['metrics']['errors']['types']}")
EOF
}

echo "######## S3 server variants"
start_dummy s3a -omit-done -chunk-interval 5ms; p=$P;      run s3a_omit_done  http://127.0.0.1:$p/v1/chat/completions --num-requests 4 --concurrency 2 --max-tokens 10 --streaming --log-level error
start_dummy s3b -role-only; p=$P;                          run s3b_role_only  http://127.0.0.1:$p/v1/chat/completions --num-requests 4 --concurrency 2 --max-tokens 10 --streaming --log-level error
start_dummy s3c -reasoning -latency 40ms -chunk-interval 10ms; p=$P; run s3c_reasoning_then_content http://127.0.0.1:$p/v1/chat/completions --num-requests 4 --concurrency 2 --max-tokens 10 --streaming --log-level error
start_adv s3c2 reasoning_only 5 20; p=$P;                  run s3c2_reasoning_only http://127.0.0.1:$p/v1/chat/completions --num-requests 4 --concurrency 2 --max-tokens 10 --streaming --log-level error
start_dummy s3d -include-usage=false -chunk-interval 5ms; p=$P; run s3d_no_usage_stream http://127.0.0.1:$p/v1/chat/completions --num-requests 4 --concurrency 2 --max-tokens 10 --streaming --log-level error
start_dummy s3e -latency 50ms; p=$P;                       run s3e_nonstream  http://127.0.0.1:$p/v1/chat/completions --num-requests 4 --concurrency 2 --max-tokens 10 --log-level error
start_adv s3f nonstream_no_usage 5 50; p=$P;               run s3f_nonstream_no_usage http://127.0.0.1:$p/v1/chat/completions --num-requests 4 --concurrency 2 --max-tokens 10 --log-level error
start_adv s3g finish_usage_close 5 20; p=$P;               run s3g_finish_usage_close http://127.0.0.1:$p/v1/chat/completions --num-requests 4 --concurrency 2 --max-tokens 10 --streaming --log-level error

echo "######## S2 gateway re-chunking"
for m in crlf two_per_write split_event utf8_split big64k multiline_data proxy_buffered; do
  start_adv s2_$m $m 6 20; p=$P; run s2_$m http://127.0.0.1:$p/v1/chat/completions --num-requests 3 --concurrency 1 --max-tokens 6 --streaming --log-level error
done

echo "######## S4 error storms"
start_dummy s4a -error-rate 1 -seed 1; p=$P;               run s4a_503_nonstream http://127.0.0.1:$p/v1/chat/completions --num-requests 6 --concurrency 2 --max-tokens 10 --log-level error
start_dummy s4b -error-rate 1 -seed 1 -chunk-interval 5ms; p=$P; run s4b_midstream_error http://127.0.0.1:$p/v1/chat/completions --num-requests 6 --concurrency 2 --max-tokens 10 --streaming --log-level error
start_dummy s4c -max-concurrency 1 -latency 100ms -chunk-interval 5ms; p=$P; run s4c_429_storm http://127.0.0.1:$p/v1/chat/completions --num-requests 32 --concurrency 8 --max-tokens 5 --streaming --log-level error
start_adv s4c2 http429 5 20; p=$P;                         run s4c2_429_all http://127.0.0.1:$p/v1/chat/completions --num-requests 6 --concurrency 2 --max-tokens 5 --streaming --log-level error
start_adv s4c3 http503 5 20; p=$P;                         run s4c3_503_all_stream http://127.0.0.1:$p/v1/chat/completions --num-requests 6 --concurrency 2 --max-tokens 5 --streaming --log-level error
run s4d_conn_refused http://127.0.0.1:1/v1/chat/completions --num-requests 6 --concurrency 2 --max-tokens 5 --streaming --log-level error
start_adv s4e reset 5 20; p=$P;                            run s4e_conn_reset http://127.0.0.1:$p/v1/chat/completions --num-requests 6 --concurrency 2 --max-tokens 5 --streaming --log-level error
start_adv s4f slowloris 5 20; p=$P;                        run s4f_slowloris http://127.0.0.1:$p/v1/chat/completions --num-requests 4 --concurrency 2 --max-tokens 5 --streaming --log-level error --request-timeout 2
start_dummy s4g -error-rate 1 -seed 1; p=$P;               run s4g_openloop_during_failure http://127.0.0.1:$p/v1/chat/completions --num-requests 40 --concurrency 100 --max-tokens 5 --request-rate 20 --arrival constant --log-level error

echo "######## S5 overload open loop"
start_dummy s5 -latency 500ms; p=$P;
run s5a_rate40_cap4 http://127.0.0.1:$p/v1/chat/completions --num-requests 40 --concurrency 4 --max-tokens 5 --streaming --request-rate 40 --arrival constant --log-level error
run s5b_rate40_cap1000 http://127.0.0.1:$p/v1/chat/completions --num-requests 40 --concurrency 4 --max-concurrency 1000 --max-tokens 5 --streaming --request-rate 40 --arrival constant --log-level error
run s5c_rate40_cap1000_warmup4 http://127.0.0.1:$p/v1/chat/completions --num-requests 40 --concurrency 4 --max-concurrency 1000 --max-tokens 5 --streaming --request-rate 40 --arrival constant --warmup-requests 4 --log-level error --slo e2e=600ms --slo ttft=550ms

exit 0
echo "######## S6/S7 prefix + eos (bodies captured at debug level)"
start_dummy s6 -chunk-interval 2ms; p=$P;
run s6a_repeat_prompts http://127.0.0.1:$p/v1/chat/completions --num-requests 8 --concurrency 2 --max-tokens 5 --streaming --log-level debug --seed 7
run s6b_unique_prompts http://127.0.0.1:$p/v1/chat/completions --num-requests 8 --concurrency 2 --max-tokens 5 --streaming --log-level debug --seed 7 --unique-prompts
run s7a_eos_default http://127.0.0.1:$p/v1/chat/completions --num-requests 4 --concurrency 2 --max-tokens 5 --streaming --log-level debug --seed 7
run s7b_ignore_eos http://127.0.0.1:$p/v1/chat/completions --num-requests 4 --concurrency 2 --max-tokens 5 --streaming --log-level debug --seed 7 --ignore-eos --min-tokens 5 --system-prompt "" --extra-body-json '{"temperature":0.9,"top_p":0.5}'
echo "######## S8b determinism (same seed twice, different seed once)"
run s8b_seed7_run1 http://127.0.0.1:$p/v1/chat/completions --num-requests 16 --concurrency 3 --max-tokens 5 --streaming --log-level debug --seed 7 --warmup-requests 2
run s8b_seed7_run2 http://127.0.0.1:$p/v1/chat/completions --num-requests 16 --concurrency 3 --max-tokens 5 --streaming --log-level debug --seed 7 --warmup-requests 2
run s8b_seed8_run1 http://127.0.0.1:$p/v1/chat/completions --num-requests 16 --concurrency 3 --max-tokens 5 --streaming --log-level debug --seed 8 --warmup-requests 2

echo "######## S8 multi-endpoint with one dead replica"
cat > $OUT/endpoints_dead.yaml <<EOF
- url: http://127.0.0.1:$p/v1/chat/completions
  api_key: dummy
  name: alive
- url: http://127.0.0.1:1/v1/chat/completions
  api_key: dummy
  name: dead
EOF
d=$OUT/s8a_rr; mkdir -p $d; printf '%s\n' '{"prompt":"Hi"}' > $d/prompts.jsonl
( cd $d && $BIN --endpoints-file $OUT/endpoints_dead.yaml --scenario s8a --prompts $d/prompts.jsonl --mode chat --model dummy --data-log $d/out.jsonl --debug-log $d/debug.log --error-log $d/error.log --num-requests 20 --concurrency 4 --max-tokens 5 --streaming --log-level error --load-balancer round-robin > stdout.txt 2> stderr.txt; echo "exit=$?" >> stderr.txt )
d=$OUT/s8b_li; mkdir -p $d; printf '%s\n' '{"prompt":"Hi"}' > $d/prompts.jsonl
( cd $d && $BIN --endpoints-file $OUT/endpoints_dead.yaml --scenario s8b --prompts $d/prompts.jsonl --mode chat --model dummy --data-log $d/out.jsonl --debug-log $d/debug.log --error-log $d/error.log --num-requests 20 --concurrency 4 --max-tokens 5 --streaming --log-level error --load-balancer least-inflight > stdout.txt 2> stderr.txt; echo "exit=$?" >> stderr.txt )
for n in s8a_rr s8b_li; do echo "== $n"; python3 - $OUT/$n/out.jsonl <<'EOF'
import json,sys
for l in open(sys.argv[1]):
    d=json.loads(l)
    if 'summary.v' in d.get('schema_version',''):
        print('  pooled_mixture=',d['pooled_mixture'],'attempted',d['attempted'],'succ',d['successes'],'err',d['errors'],'by_type',d['errors_by_type'],'rps',round(d['requests_per_second'],2))
        for k,v in d['per_endpoint'].items(): print('   ',k,{kk:(vv if not isinstance(vv,dict) else (vv['n'],vv['avg'])) for kk,vv in v.items()})
    elif 'config' in d: print('  legacy per_endpoint keys:', list(d['metrics']['per_endpoint'].keys()), 'legacy rps', round(d['metrics']['timing']['requests_per_second'],2))
EOF
done
echo "######## done"
```

### B.5 `smoke_audit.py` (published-results tracing)

```python
#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Trace every number in docs/SMOKE_RESULTS.md back to live-results raw files."""
import json, math, os, re, sys
ROOT = "/home/<user>/src/bench-cli/live-results/campaign-oss-20260915-smoke-rerun"
DOC = open("/home/<user>/src/bench-cli/docs/SMOKE_RESULTS.md").read()

def q7(s, p):
    n = len(s)
    if n == 0: return None
    if n == 1: return s[0]
    h = (n - 1) * p / 100
    lo, hi = math.floor(h), math.ceil(h)
    return s[lo] if lo == hi else s[lo] * (1 - (h - lo)) + s[hi] * (h - lo)

def load(path):
    recs, summ, legacy = [], None, None
    for line in open(path):
        d = json.loads(line)
        sv = d.get("schema_version", "")
        if "request.v" in sv: recs.append(d)
        elif "metrum-ai-bench.summary.v2" in sv: summ = d
        elif "config" in d or "imagegen.summary" in sv: legacy = d
    return recs, summ, legacy

rows = re.findall(r"^\| (c\d+-n\d+|rate\d+-n\d+) \| (\d+) \| ([\d.]+) \| ([\d.]+) \| ([\d.]+) \| ([\d.]+) \|$", DOC, re.M)
print(f"doc rows with n/latency/ttft: {len(rows)}")
lane_for = {}
for lane in ("llm", "vlm"):
    for cell in os.listdir(os.path.join(ROOT, lane)):
        lane_for[(lane, cell)] = os.path.join(ROOT, lane, cell, "results.jsonl")
seen = set()
total_measured = 0
for cell, n, lp50, lp95, tp50, tp95 in rows:
    cands = [(l, c) for (l, c) in lane_for if c == cell and (l, c) not in seen]
    # doc lists LLM rows first then VLM; disambiguate by table order
    lane = "llm" if ("llm", cell) in lane_for and ("llm", cell) not in seen else "vlm"
    seen.add((lane, cell))
    recs, summ, legacy = load(lane_for[(lane, cell)])
    meas = [r for r in recs if r.get("phase") == "measure" and r.get("error") is None]
    total_measured += len([r for r in recs if r.get("phase") == "measure"])
    lat = sorted(r["latency_s"] for r in meas)
    ttft = sorted(r["ttft_s"] for r in meas if r.get("ttft_s") is not None)
    mine = (len(meas), q7(lat, 50), q7(lat, 95), q7(ttft, 50), q7(ttft, 95))
    doc = (int(n), float(lp50), float(lp95), float(tp50), float(tp95))
    ok = mine[0] == doc[0] and all(abs(round(m, 3) - d) < 0.0015 for m, d in zip(mine[1:], doc[1:]))
    flags = f"p99_unreliable(tool)={summ['latency_s']['p99_unreliable'] if summ else '?'} p95_samples_above_p95={len(lat)*0.05:.1f} window={summ['window_seconds'] if summ else '?'} rps_tool={summ['requests_per_second'] if summ else '?'}"
    cfg = legacy["config"] if legacy and "config" in legacy else {}
    print(f"{lane:3} {cell:11} doc n={doc[0]} lat p50/p95={doc[1]}/{doc[2]} ttft p50/p95={doc[3]}/{doc[4]} | recomputed n={mine[0]} {mine[1]:.3f}/{mine[2]:.3f} {mine[3]:.3f}/{mine[4]:.3f} -> {'MATCH' if ok else 'MISMATCH'} | {flags} | config has seed={'seed' in cfg} warmup={'warmup_requests' in cfg} request_rate={'request_rate' in cfg} ignore_eos={'ignore_eos' in cfg}")
    if not summ:
        print("   !! no summary.v2 in", lane_for[(lane, cell)])
    else:
        # queue delay pollution check (closed loop)
        qd = [r.get("queue_delay_s", 0) for r in meas]
        pe = list(summ["per_endpoint"].values())[0]["latency_s"]["avg"] if summ["per_endpoint"] else None
        print(f"      closed/open loop: scheduled_offset set={any(r.get('scheduled_offset_s') for r in meas)} queue_delay max={max(qd):.3f} | tool latency avg={summ['latency_s']['avg']:.4f} co_latency avg={summ['coordinated_omission_latency_s']['avg']:.4f} per_endpoint latency avg={pe:.4f}")

# ASR and imagegen rows
asr_path = os.path.join(ROOT, "asr", "c1-n8", "results.jsonl")
recs, summ, legacy = load(asr_path)
meas = [r for r in recs if r.get("phase") == "measure" and r.get("error") is None]
lat = sorted(r["latency_s"] for r in meas)
rtfx = sorted(r["modality_metrics"]["rtfx_client"] for r in meas if "rtfx_client" in r.get("modality_metrics", {}))
wer = sorted(r["modality_metrics"]["wer"] for r in meas if "wer" in r.get("modality_metrics", {}))
print(f"asr c1-n8: doc n=6 lat p50=0.101 rtfx p50=39.76 wer p50=1.0 | recomputed n={len(meas)} lat p50={q7(lat,50):.4f} rtfx p50={q7(rtfx,50) if rtfx else None} wer p50={q7(wer,50) if wer else None} (wer samples={len(wer)}) transcripts={[r.get('modality_metrics',{}).get('wer') for r in meas][:3]}")
img_path = os.path.join(ROOT, "imagegen", "c1-n8", "results.jsonl")
v1 = []
v1s = None
v2 = None
for line in open(img_path):
    d = json.loads(line)
    sv = d.get("schema_version", "")
    if sv.endswith("imagegen.request.v1"): v1.append(d)
    elif sv.endswith("imagegen.summary.v1"): v1s = d
    elif "metrum-ai-bench.summary.v2" in sv: v2 = d
ok = [r for r in v1 if r["status"] == "success" and int(r["request_id"]) > 2]
print(f"imagegen c1-n8: doc successful images=6 images/s=7.372 latency p50 ms=101.255 | v1 summary images_generated={v1s and v1s['images_generated']} images_per_second={v1s and round(v1s['images_per_second'],3)} latency_ms p50={v1s and v1s['latency_ms'].get('p50')} duration_s={v1s and round(v1s['duration_seconds'],3)} request_count={v1s and v1s['request_count']} | v1 records={len(v1)} post-warmup successes={len(ok)} | v2 attempted={v2 and v2['attempted']} v2 rps={v2 and round(v2['requests_per_second'],3)}")
print(f"doc claims 'Validation: 12 result files, 490 measured request lines'. Counting measured request lines across all results.jsonl:")
count_files = 0; count_meas = 0; count_all = 0
for dp, dn, fn in os.walk(ROOT):
    for f in fn:
        if f == "results.jsonl":
            count_files += 1
            for line in open(os.path.join(dp, f)):
                d = json.loads(line)
                sv = d.get("schema_version", "")
                if "request.v" in sv:
                    count_all += 1
                    if d.get("phase") == "measure": count_meas += 1
                    elif "imagegen" in sv: count_meas += 1
print(f"   files={count_files} request lines={count_all} measure-phase(+imagegen v1)={count_meas}")
print("units stated in doc tables:", "latency p50 |" in DOC and "(s)" in DOC)
```

### B.6 `clockstep.c` (LD_PRELOAD wall-clock step shim)

```c
/* Copyright (c) 2026 Metrum AI, Inc.
 * SPDX-License-Identifier: Apache-2.0
 *
 * LD_PRELOAD shim: after CLOCKSTEP_AFTER_MS milliseconds of process life,
 * CLOCK_REALTIME (and gettimeofday/time) jump forward by CLOCKSTEP_JUMP_S
 * seconds. CLOCK_MONOTONIC is untouched, exactly like a real NTP step.
 * Build: gcc -shared -fPIC -o clockstep.so clockstep.c -ldl
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdlib.h>
#include <sys/time.h>
#include <time.h>

static int (*real_cg)(clockid_t, struct timespec *) = 0;
static struct timespec t0;
static int inited = 0;
static long after_ms = 2000;
static long jump_s = 3600;

static void init(void) {
    if (inited) return;
    real_cg = dlsym(RTLD_NEXT, "clock_gettime");
    const char *a = getenv("CLOCKSTEP_AFTER_MS");
    const char *j = getenv("CLOCKSTEP_JUMP_S");
    if (a) after_ms = atol(a);
    if (j) jump_s = atol(j);
    real_cg(CLOCK_MONOTONIC, &t0);
    inited = 1;
}

static int stepped(void) {
    struct timespec now;
    real_cg(CLOCK_MONOTONIC, &now);
    long ms = (now.tv_sec - t0.tv_sec) * 1000 + (now.tv_nsec - t0.tv_nsec) / 1000000;
    return ms >= after_ms;
}

int clock_gettime(clockid_t clk, struct timespec *ts) {
    init();
    int r = real_cg(clk, ts);
    if (r == 0 && (clk == CLOCK_REALTIME || clk == CLOCK_REALTIME_COARSE) && stepped()) {
        ts->tv_sec += jump_s;
    }
    return r;
}

int gettimeofday(struct timeval *tv, void *tz) {
    struct timespec ts;
    int r = clock_gettime(CLOCK_REALTIME, &ts);
    if (r == 0 && tv) { tv->tv_sec = ts.tv_sec; tv->tv_usec = ts.tv_nsec / 1000; }
    (void)tz;
    return r;
}

time_t time(time_t *t) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    if (t) *t = ts.tv_sec;
    return ts.tv_sec;
}
```

### B.7 `saturating_server.py` (known-capacity server for knee checks)

```python
#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Non-streaming OpenAI-compatible server with a KNOWN saturation point.

K workers, each request takes service_ms of exclusive worker time (FIFO queue).
Capacity = K / service_s requests per second. Below capacity latency == service;
at closed-loop concurrency C > K latency ~= service * C / K.

  saturating_server.py PORT K SERVICE_MS
Also serves GET /metrics with vllm:num_requests_running / waiting and a log of
request bodies on stdout.
"""
import asyncio, json, sys, time
from aiohttp import web

PORT, K, SERVICE = int(sys.argv[1]), int(sys.argv[2]), float(sys.argv[3]) / 1000.0
sem = asyncio.Semaphore(K)
running = 0
waiting = 0
count = 0


async def chat(request):
    global running, waiting, count
    body = await request.json()
    count += 1
    print(json.dumps({"n": count, "messages": body.get("messages"), "session": body.get("session_id")}), flush=True)
    waiting += 1
    async with sem:
        waiting -= 1
        running += 1
        await asyncio.sleep(SERVICE)
        running -= 1
    content = "ok"
    if body.get("response_format", {}).get("type") == "json_schema":
        content = json.dumps({"answer": "mock", "count": 1})
    resp = {"id": "sat", "object": "chat.completion", "created": int(time.time()), "model": body.get("model", "sat"),
            "choices": [{"index": 0, "message": {"role": "assistant", "content": content}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 8, "completion_tokens": 4, "total_tokens": 12}}
    return web.json_response(resp)


async def metrics(request):
    txt = f"# HELP vllm:num_requests_running x\nvllm:num_requests_running {running}\nvllm:num_requests_waiting {waiting}\nvllm:gpu_cache_usage_perc {min(1.0, running / K):.2f}\nvllm:num_preemptions_total 0\n"
    return web.Response(text=txt, content_type="text/plain")


app = web.Application()
app.router.add_post("/v1/chat/completions", chat)
app.router.add_get("/metrics", metrics)
web.run_app(app, host="127.0.0.1", port=PORT, print=None)
```

### B.8 `longrun.sh` and `modalities.sh`

```bash
#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
# S9: long run, RSS over time, incremental flush, SIGINT, SIGKILL.
set -u
S=/tmp/claude-1000/-home-cgadgil-src-bench-cli/4d3b7b21-f028-418b-9578-a4c2900e1257/scratchpad
BIN=$S/bin/metrum-ai-bench-llm; DUMMY=$S/bin/dummy-model-server
OUT=$S/scen/s9; mkdir -p $OUT; cd $OUT
$DUMMY -port 19501 -latency 20ms -chunk-interval 2ms > dummy.log 2>&1 & DP=$!
sleep 0.8
printf '%s\n' '{"prompt":"Hi there"}' > prompts.jsonl
common="--url http://127.0.0.1:19501/v1/chat/completions --api-key dummy --prompts prompts.jsonl --mode chat --model dummy --max-tokens 5 --streaming --log-level error --concurrency 8"

echo "### S9a full 3000-request run: RSS and data-log size sampled every 0.5s"
rm -f full.jsonl
$BIN $common --scenario s9a --num-requests 3000 --data-log full.jsonl --debug-log d.log --error-log e.log > full.stdout 2> full.stderr & P=$!
t0=$(date +%s.%N)
while kill -0 $P 2>/dev/null; do
  rss=$(awk '/VmRSS/{print $2}' /proc/$P/status 2>/dev/null); sz=$(stat -c %s full.jsonl 2>/dev/null || echo 0); lines=$(wc -l < full.jsonl 2>/dev/null || echo 0)
  echo "t=$(echo "$(date +%s.%N) - $t0" | bc | cut -c1-5)s rss_kb=$rss log_bytes=$sz log_lines=$lines"
  sleep 0.5
done
wait $P; echo "exit=$?"
python3 - full.jsonl <<'EOF'
import json,sys
n=0;s=None
for l in open(sys.argv[1]):
    d=json.loads(l)
    if 'request.v' in d.get('schema_version',''): n+=1
    elif 'summary.v' in d.get('schema_version',''): s=d
print(f"records={n} summary.attempted={s['attempted']} succ={s['successes']} window={s['window_seconds']:.3f} rps={s['requests_per_second']:.1f} partial={s['partial']} co_lat_max={s['coordinated_omission_latency_s']['max']:.3f} per_ep_lat_avg={[v['latency_s']['avg'] for v in s['per_endpoint'].values()]}")
EOF

echo "### S9b SIGINT at ~3s into a 3000-request run"
rm -f int.jsonl
$BIN $common --scenario s9b --num-requests 3000 --data-log int.jsonl --debug-log d.log --error-log e.log > int.stdout 2> int.stderr & P=$!
sleep 3; echo "lines_before_sigint=$(wc -l < int.jsonl 2>/dev/null || echo 0)"; kill -INT $P; ti=$(date +%s.%N)
sleep 1; echo "alive_1s_after_sigint=$(kill -0 $P 2>/dev/null && echo yes || echo no)"; kill -INT $P 2>/dev/null
for i in $(seq 1 60); do kill -0 $P 2>/dev/null || break; sleep 0.5; done
echo "exited_after_sigint_s=$(echo "$(date +%s.%N) - $ti" | bc | cut -c1-5) alive=$(kill -0 $P 2>/dev/null && echo yes || echo no)"
wait $P 2>/dev/null; echo "exit=$?"
python3 - int.jsonl <<'EOF'
import json,sys
n=0;s=None;bad=0
for l in open(sys.argv[1]):
    try: d=json.loads(l)
    except Exception: bad+=1; continue
    if 'request.v' in d.get('schema_version',''): n+=1
    elif 'summary.v' in d.get('schema_version',''): s=d
print(f"records={n} unparseable_lines={bad} summary_present={s is not None} partial={s and s['partial']} attempted={s and s['attempted']} succ={s and s['successes']}")
EOF

echo "### S9c SIGKILL at ~3s"
rm -f kill.jsonl
$BIN $common --scenario s9c --num-requests 3000 --data-log kill.jsonl --debug-log d.log --error-log e.log > kill.stdout 2> kill.stderr & P=$!
sleep 3; echo "lines_before_sigkill=$(wc -l < kill.jsonl 2>/dev/null || echo 0)"; kill -KILL $P; wait $P 2>/dev/null; echo "exit=$?"
python3 - kill.jsonl <<'EOF'
import json,sys
n=0;bad=0;s=None
for l in open(sys.argv[1]):
    try: d=json.loads(l)
    except Exception: bad+=1; continue
    if 'request.v' in d.get('schema_version',''): n+=1
    elif 'summary.v' in d.get('schema_version',''): s=d
print(f"records={n} unparseable_lines={bad} summary_present={s is not None}")
EOF

echo "### S9d SIGINT during open-loop run (rate 200/s, cap 8) -- does the schedule loop break promptly?"
rm -f int2.jsonl
$BIN $common --scenario s9d --num-requests 3000 --request-rate 200 --max-concurrency 8 --data-log int2.jsonl --debug-log d.log --error-log e.log > int2.stdout 2> int2.stderr & P=$!
sleep 3; kill -INT $P; ti=$(date +%s.%N)
for i in $(seq 1 60); do kill -0 $P 2>/dev/null || break; sleep 0.5; done
echo "exited_after_sigint_s=$(echo "$(date +%s.%N) - $ti" | bc | cut -c1-5)"; wait $P 2>/dev/null; echo "exit=$?"
python3 - int2.jsonl <<'EOF'
import json,sys
n=0;s=None
for l in open(sys.argv[1]):
    d=json.loads(l)
    if 'request.v' in d.get('schema_version',''): n+=1
    elif 'summary.v' in d.get('schema_version',''): s=d
print(f"records={n} partial={s and s['partial']} attempted={s and s['attempted']}")
EOF
kill $DP

# ---- modalities.sh ----
#!/usr/bin/env bash
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
# S13: VLM / ASR / imagegen runtime checks against the Go dummy server.
set -u
S=/tmp/claude-1000/-home-cgadgil-src-bench-cli/4d3b7b21-f028-418b-9578-a4c2900e1257/scratchpad
R=/home/<user>/src/bench-cli
DUMMY=$S/bin/dummy-model-server
OUT=$S/scen/s13; mkdir -p $OUT; cd $OUT
$DUMMY -port 19701 -latency 100ms -chunk-interval 20ms > dummy.log 2>&1 & DP=$!
sleep 0.8
summ() { python3 - "$1" <<'EOF'
import json,sys
recs=[];s=None;leg=[]
for l in open(sys.argv[1]):
    d=json.loads(l); sv=d.get('schema_version','')
    if 'request.v' in sv: recs.append(d)
    elif 'summary.v' in sv and 'metrum-ai-bench.summary' in sv: s=d
    else: leg.append(d)
print(f"  records={len(recs)} phases={ {p:sum(1 for r in recs if r.get('phase')==p) for p in ('warmup','measure')} } errors={[r['error'] for r in recs if r.get('error')][:3]}")
if recs:
    r=recs[-1]; print(f"  last rec: seq={r.get('seq')} ttft={r.get('ttft_s')} lat={r.get('latency_s')} ct={r.get('completion_tokens')} usage_missing={r.get('usage_missing')} itl_n={len(r.get('itl_s',[]))} mm={r.get('modality_metrics')} qd={r.get('queue_delay_s')} so={r.get('scheduled_offset_s')}")
if s: print(f"  v2: attempted={s['attempted']} succ={s['successes']} window={s['window_seconds']:.3f} rps={s['requests_per_second']:.2f} ctps={s['completion_tokens_per_second']:.1f} lat_avg={s['latency_s']['avg']} ttft_n={s['ttft_s']['n']} co_max={s['coordinated_omission_latency_s']['max']} per_ep_lat={[v['latency_s']['avg'] for v in s['per_endpoint'].values()]} partial={s['partial']}")
for d in leg:
    if 'config' in d: print(f"  legacy config keys: {sorted(d['config'].keys())}")
    if 'schema_version' in d and 'imagegen' in d['schema_version']: print(f"  {d['schema_version']}: {json.dumps({k:d[k] for k in d if k in ('latency_ms','request_id','image_artifacts','n_returned','seed','images_per_second','requests_per_second','request_count','successful_requests','duration_seconds','warmup_requests','partial')})[:600]}")
EOF
}
IMG=$R/test-data/tiny.png
echo "### VLM streaming, warmup 2, concurrency 2, 8 requests"
printf '{"prompt":"Describe","image_urls":["%s"]}\n' "$IMG" > vlm_prompts.jsonl
$S/bin/metrum-ai-bench-vlm --url http://127.0.0.1:19701/v1/chat/completions --api-key dummy --scenario vlm --num-requests 8 --concurrency 2 --prompts vlm_prompts.jsonl --model dummy --max-tokens 10 --streaming --warmup-requests 2 --data-log vlm_stream.jsonl --debug-log d.log --error-log e.log --log-level error > vlm_stream.stdout 2> vlm_stream.stderr; echo "exit=$?"; summ vlm_stream.jsonl
echo "  tiny.png bytes=$(stat -c %s $IMG)"
echo "### VLM non-streaming, no warmup"
$S/bin/metrum-ai-bench-vlm --url http://127.0.0.1:19701/v1/chat/completions --api-key dummy --scenario vlm2 --num-requests 4 --concurrency 2 --prompts vlm_prompts.jsonl --model dummy --max-tokens 10 --data-log vlm_nonstream.jsonl --debug-log d.log --error-log e.log --log-level error > vlm_ns.stdout 2> vlm_ns.stderr; echo "exit=$?"; summ vlm_nonstream.jsonl
echo "### VLM with --system-prompt '' --min-tokens 3 (are they honored? check debug log body)"
$S/bin/metrum-ai-bench-vlm --url http://127.0.0.1:19701/v1/chat/completions --api-key dummy --scenario vlm3 --num-requests 2 --concurrency 1 --prompts vlm_prompts.jsonl --model dummy --max-tokens 10 --streaming --system-prompt "" --min-tokens 3 --ignore-eos --data-log vlm3.jsonl --debug-log vlm3_debug.log --error-log e.log --log-level debug > vlm3.stdout 2> vlm3.stderr; echo "exit=$?"
grep -o '"messages":\[[^]]*\]' vlm3_debug.log | head -1 | cut -c1-200; grep -c '"role":"system"' vlm3_debug.log; grep -o '"min_tokens":[0-9]*' vlm3_debug.log | head -1; grep -o '"ignore_eos":[a-z]*' vlm3_debug.log | head -1
grep -o '"image_url":{"url":"data:[^;]*;base64,[A-Za-z0-9+/=]*' vlm3_debug.log | head -1 | sed 's/.*base64,//' | base64 -d 2>/dev/null | cmp - $IMG && echo "  image bytes sent == tiny.png (byte-identical)"

echo "### ASR with ground truth, verbose-json default"
printf '{"id":"a1","path":"%s","format":"mp3","duration":2.0}\n{"id":"a2","path":"%s","format":"mp3","duration":4.0}\n' $R/test-data/dummy.mp3 $R/test-data/dummy.mp3 > asr_in.jsonl
printf '{"id":"a1","transcript":"Hello world, this is a test."}\n{"id":"a2","transcript":"the quick brown fox"}\n' > asr_truth.jsonl
$S/bin/metrum-ai-bench-asr --url http://127.0.0.1:19701/v1/audio/transcriptions --api-key dummy --scenario asr --num-requests 6 --concurrency 2 --input asr_in.jsonl --ground-truth asr_truth.jsonl --model whisper-1 --data-log asr.jsonl --debug-log d.log --error-log e.log --log-level error --warmup-requests 2 > asr.stdout 2> asr.stderr; echo "exit=$?"; summ asr.jsonl
python3 - asr.jsonl <<'EOF'
import json
for l in open('asr.jsonl'):
    d=json.loads(l)
    if 'config' in d: print("  legacy: normalizer=",d['config'].get('normalizer'),"response_format=",d['config'].get('response_format'),"| metrics keys:",sorted(d['metrics'].keys())[:20]); print("  legacy wer block:", d['metrics'].get('wer') or d['metrics'].get('accuracy')); print("  legacy throughput:", d['metrics'].get('throughput'))
EOF
echo "### ASR response-format json + normalizer none"
$S/bin/metrum-ai-bench-asr --url http://127.0.0.1:19701/v1/audio/transcriptions --api-key dummy --scenario asr2 --num-requests 4 --concurrency 2 --input asr_in.jsonl --ground-truth asr_truth.jsonl --model whisper-1 --response-format json --normalizer none --data-log asr2.jsonl --debug-log d.log --error-log e.log --log-level error > asr2.stdout 2> asr2.stderr; echo "exit=$?"; summ asr2.jsonl
grep -o '"transcript"' dummy.log | head -1; grep -m3 'text' asr2.stdout | head -3

echo "### Imagegen 6 requests, warmup 2, concurrency 2"
printf '{"prompt":"a cat"}\n{"prompt":"a dog"}\n' > img_prompts.jsonl
$S/bin/metrum-ai-bench-imagegen --url http://127.0.0.1:19701/v1/images/generations --api-key dummy --scenario img --num-requests 6 --concurrency 2 --prompts img_prompts.jsonl --model dummy-image --size 64x64 --warmup-requests 2 --data-log img.jsonl --error-log e.log --artifact-dir img_artifacts > img.stdout 2> img.stderr; echo "exit=$?"; summ img.jsonl
python3 - img.jsonl <<'EOF'
import json,hashlib,os
for l in open('img.jsonl'):
    d=json.loads(l); sv=d.get('schema_version','')
    if sv.endswith('imagegen.request.v1'):
        a=d['image_artifacts'][0] if d['image_artifacts'] else None
        ok = a and os.path.exists(a['path']) and hashlib.sha256(open(a['path'],'rb').read()).hexdigest()==a['sha256']
        print(f"  req {d['request_id']} latency_ms={d['latency_ms']} status={d['status']} artifact_hash_verified={ok} n_returned={d['n_returned']}")
    elif sv.endswith('imagegen.summary.v1'):
        print(f"  v1 summary: request_count={d['request_count']} images_generated={d['images_generated']} images_per_second={d['images_per_second']:.3f} rps={d['requests_per_second']:.3f} duration={d['duration_seconds']:.3f} latency_ms={ {k:d['latency_ms'][k] for k in ('n','mean','p50') if k in d['latency_ms']} } keys={sorted(d.keys())}")
    elif 'metrum-ai-bench.summary.v2' in sv:
        print(f"  v2 summary: attempted={d['attempted']} rps={d['requests_per_second']:.3f} window={d['window_seconds']:.3f} lat_avg={d['latency_s']['avg']} errors_by_type={d['errors_by_type']}")
EOF
ls img_artifacts | head -3
kill $DP
```

---

## 12. Open-source readiness, compliance, and security checklist

Scope: everything a maintainer or a corporate open-source review board checks
before and after publishing, beyond measurement correctness. The repository
`github.com/metrum-ai/bench-cli` was already PUBLIC at review time, so every
FAIL below is live. Values of anything sensitive are never reproduced here;
only counts and categories. Commands and raw output are in Appendix A.15.

| # | Area | Check | Result | Evidence / issue |
|---|---|---|---|---|
| L1 | Licensing | `LICENSE` is the full Apache-2.0 text; `Cargo.toml` `license = "Apache-2.0"`; GitHub detects apache-2.0 | PASS | 202-line LICENSE; `gh repo view` licenseInfo apache-2.0 |
| L2 | Licensing | Copyright + SPDX header on every authored file (org policy) | PASS with gaps | `scripts/check_headers.sh` ok; six tracked files carry no copyright line: `.gitignore`, `CHANGELOG.md`, `THIRD_PARTY_LICENSES`, `dummy-model-server/go.mod`, `endpoints-4servers.yaml`, `test-data/dummy-endpoints-4.yaml`; the checker skips `dummy-model-server/*`, `scripts/live/*`, `*.md`, and non-workflow YAML |
| L3 | Licensing | Third-party license compatibility (Rust) | PASS | `cargo deny check licenses` ok; graph contains 0BSD, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2/3, BSL-1.0, CDLA-Permissive-2.0 (webpki-roots, attributed in NOTICE), ISC, MIT, MIT-0, Unicode-3.0, Unlicense, Zlib; `r-efi` offers LGPL-2.1-or-later as one alternative of a permissive dual license (target-specific dependency) |
| L4 | Licensing | Third-party license inventory current | FAIL (minor) | `THIRD_PARTY_LICENSES:2` "for version 0.1.80"; lists CC0-1.0 and NCSA which no crate in the 0.1.82 graph uses |
| L5 | Licensing | Go dependencies | PASS | one module, `golang.org/x/time` (BSD-3-Clause) |
| L6 | Licensing | Test fixture provenance | PASS | `test-data/README.md`; no third-party creative content in the tree (Project Gutenberg text existed only in private history, not on any pushed ref) |
| L7 | Trademark | Third-party marks | ATTENTION | MLPerf export files reproduce LoadGen headings verbatim with `Result is : VALID` and no disclaimer (F-14); "MLPerf" is an MLCommons mark |
| S1 | Secrets | Live credentials in tree or history | PASS | gitleaks 8.30.1 over 19 commits, with and without the repo allowlist: 0 findings; `env.json` gitignored, untracked, absent from history; in-process check confirmed its value appears nowhere else |
| S2 | Secrets / privacy | Internal infrastructure details in tracked, public files | **FAIL (HIGH)** | `(removed from main; prior OSS readiness assessment)` (tracked, 200 KB) reproduces verbatim the prior review's leaked items: 1 SSH public key, 1 AWS account id and ECR registry, 2 occurrences of a cleartext password, 7 occurrences of a personal email, 2 backup hostnames, 2 public IPs, 4 chat-webhook references. `.gitleaks.toml:10` allowlists this file "by policy". The repository is public, so these are published. Issue O-01 |
| S3 | Secrets | GitHub secret scanning and push protection | FAIL | `security_and_analysis`: secret_scanning disabled, push_protection disabled, dependabot_security_updates disabled; vulnerability alerts enabled (0 open) |
| S4 | Dependencies | Rust advisories | FAIL | `cargo deny check` exit 1: RUSTSEC-2020-0071 (vulnerability, `time 0.1.45` via `ntp 0.5.0`), RUSTSEC-2026-0002 and -0253 (unsound, `lru 0.14`), RUSTSEC-2025-0058 (unmaintained, `custom_derive`); CI runs `check licenses` only (F-17) |
| S5 | Dependencies | Go vulnerabilities | FAIL (toolchain) | `govulncheck`: GO-2026-6090, GO-2026-6089, GO-2026-5972, GO-2026-5856 in go1.26.4 stdlib reached from `main.go:25` and `types.go:105`; fixed in 1.26.5/1.26.6; not run in CI |
| S6 | Supply chain | Actions pinned to immutable SHAs | FAIL | all 20 `uses:` are mutable tags (`@v4`, `@stable`, `@cargo-llvm-cov`) |
| S7 | Supply chain | Dependency update automation | FAIL | no `.github/dependabot.yml` (cargo, gomod, github-actions) |
| S8 | Supply chain | Release integrity | PASS | Sigstore keyless `sign-blob`, `attest-build-provenance`, CycloneDX SBOM, sha256 per tarball; releases v0.1.80-82 exist |
| S9 | Supply chain | Workflow permissions | PASS | CI `contents: read`; release `contents: write, id-token: write, attestations: write` scoped to the release workflow; Homebrew tap push uses a repository secret |
| S10 | Code | `unsafe` | PASS | 0 occurrences |
| S11 | Code | `unwrap`/`expect` outside tests (CLAUDE.md: avoid) | ATTENTION | ASR 11, imagegen 3, VLM 1, `endpoints.rs` 1, `environment.rs` 1 (guarded per code read) |
| S12 | Code | Credential handling | ATTENTION | `--api-key` accepted only on the command line or in an endpoints YAML for the four modality binaries (visible in process listings and shell history; documented in `SECURITY.md:22`); strategic runner also reads `OPENAI_API_KEY`; debug logging masks the header (`llm.rs:804`) but prints full request bodies (prompts) at `--log-level debug` (VLM redacts) |
| S13 | Code | TLS | PASS with gap | rustls, verification always on, no insecure switch; no private-CA option (F-13) |
| S14 | Code | Network egress by default | PASS | none unless `--ntp-check` (4 public NTP pools), an `http(s)://` prompt source, or `--otlp-endpoint` |
| S15 | Shipped server | `dummy-model-server` hardening | ATTENTION | `io.ReadAll` on request bodies with no limit (`types.go:105`), `http.ListenAndServe` with no read/header/idle timeouts (`main.go:25`), binds all interfaces, Docker image runs as root, base images by tag (`golang:1.26-alpine`, `alpine:3.20`), no HEALTHCHECK; acceptable for a loopback test tool, not for the published `docker-compose.yaml` exposing 8000-8003 |
| G1 | Governance | README, LICENSE, CONTRIBUTING, CODE_OF_CONDUCT (with enforcement contact), SECURITY, CODEOWNERS | PASS | GitHub community profile 87 %; CoC contact `<email>` |
| G2 | Governance | Issue and PR templates | FAIL | absent (`community/profile`: issue_template false, pull_request_template false) |
| G3 | Governance | DCO sign-off (required by CONTRIBUTING) | PASS | every commit on `main` carries `Signed-off-by` |
| G4 | Governance | Branch protection | PASS | `main` requires `fmt, clippy, test (stable)`, `core line coverage (80%)`, `secret scanning (gitleaks)`, `dependency licenses (cargo-deny)`, strict |
| G5 | Governance | Bus factor | ATTENTION | `CODEOWNERS` names one person; one author across all 19 commits |
| G6 | Governance | CHANGELOG continuity | FAIL (minor) | no v0.1.79 entry; v0.1.80 and later never cite the prior findings they close |
| G7 | Governance | Internal-only material in the public tree | FAIL | `CLAUDE.md` (agent instructions), `docs/QUALITY_ASSESSMENT_PROMPT.md`, `(removed from main; prior OSS readiness assessment)`, `CONTRIBUTING.md:44` pointing contributors to the internal assessment |
| G8 | History | Public history free of private ancestry | PASS on remote, ATTENTION locally | remote has only `main` and three tags; the maintainer clone has a local `private-archive-main` branch (11 commits importing from the "insights monorepo") that a `git push --all` would publish |
| P1 | Packaging | crates.io | PASS | name `metrumbench` is unregistered; `cargo publish --dry-run` runs in the release workflow; crates.io publish is skipped without a token by design |
| P2 | Packaging | Crate contents | FAIL (minor) | `cargo package --list` ships `CLAUDE.md`, `README-METRUMBENCH-ASR.md`, `endpoints-4servers.yaml`, `docs/CAMPAIGN.md`, `docs/HISTORY_REWRITE.md`, `docs/SMOKE_RESULTS.md`, `docs/QUALITY_ASSESSMENT_*.md`, `test-data/dummy-endpoints-4.yaml` |
| P3 | Packaging | Homebrew formula | FAIL | `test do` asserts `"Sweep and benchmark"` in `metrum-ai-bench --help` (0 matches) so `brew test` fails; checked-in formula says 0.1.80 with placeholder sha256 (regenerated at release, but the tracked copy is what users read) |
| P4 | Packaging | MSRV | UNDETERMINED | `rust-version = "1.85"` declared; no 1.85 toolchain locally and no MSRV job in CI |
| P5 | Packaging | Binary naming | ATTENTION | crate `metrumbench`, binaries `metrum-ai-bench-*`, deprecated shims `metrumbench-*`, formula `metrumbench`, mock server `metrumbench-mock-server` |
| P6 | Packaging | Reproducible builds | ATTENTION | `compile_time::datetime_str!()` embeds the build time in every binary and in `compile_info` of every result file |
| D1 | Docs | README install path | FAIL (minor) | no install instructions from GitHub Releases, Homebrew, or cargo; no status/maturity statement; no badges |
| D2 | Docs | Claims match behavior | FAIL | `README.md:47` and `docs/OUTPUT_SCHEMA.md:10` (incremental flush, F-04); `docs/METRICS.md` window and headline-latency statements (F-01, F-09) |
| D3 | Docs | CLI reference current | PASS | `scripts/render_cli_help.sh` output identical to `docs/CLI.md` |
| C1 | CI | Gates | PASS with gaps | fmt, clippy `-D warnings`, tests with dummy required, headers, gitleaks (full history), coverage 80 % on core, `cargo package`; missing: `cargo deny check` (advisories, bans), `govulncheck`, MSRV, `cargo audit` schedule |
| C2 | CI | Release pipeline health | PASS | v0.1.82 release succeeded on tag push; v0.1.81 failed (crates.io token) and was fixed by #2 |

### Appendix A.15 raw output for Section 12

```
gh repo view: nameWithOwner metrum-ai/bench-cli, visibility PUBLIC, licenseInfo apache-2.0, topics ai-inference benchmarking llm load-testing rust
gitleaks git --redact . (repo config)          -> 19 commits scanned, no leaks found, exit 0
gitleaks git --redact --config /dev/null .     -> 19 commits scanned, no leaks found, exit 0
grep -c over (removed from main; prior OSS readiness assessment): 'ssh-rsa AAAA' 1 | AWS account id 1 | password string 2 | personal email 7 | backup host 2 | public IP #1 1 | public IP #2 1 | 'dkr.ecr' 1 | webhook refs 4 | private key blocks 0
same patterns across all tracked files -> only (removed from main; prior OSS readiness assessment)
govulncheck ./... (dummy-model-server): GO-2026-6090 crypto/tls (fixed 1.26.6), GO-2026-6089 net/http (1.26.6), GO-2026-5972 encoding/asn1 (1.26.6), GO-2026-5856 crypto/tls (1.26.5); exit status 3
cargo deny list --layout license: 0BSD 1, Apache-2.0 193, Apache-2.0 WITH LLVM-exception, BSD-2-Clause 1, BSD-3-Clause 4, BSL-1.0 1, CDLA-Permissive-2.0 1 (webpki-roots 1.0.9), ISC 5, LGPL-2.1-or-later 2 (r-efi, dual-licensed), MIT-0 1, MIT 239, Unicode-3.0 19, Unlicense 8, Zlib 7
grep -rn 'unsafe ' src/ -> 0
non-test unwrap/expect: asr 11, imagegen 3, vlm 1, endpoints.rs 1, environment.rs 1
git branch -a -> main, private-archive-main, remotes/origin/main; git ls-remote --heads -> main only; tags v0.1.80 v0.1.81 v0.1.82
git rev-list --all --count -> 19; main -> 8 commits, all with Signed-off-by; single author
gh api .../branches/main/protection -> required contexts: "fmt, clippy, test (stable)", "core line coverage (80%)", "secret scanning (gitleaks)", "dependency licenses (cargo-deny)", strict true
gh api .../ -q .security_and_analysis -> dependabot_security_updates disabled, secret_scanning disabled, secret_scanning_push_protection disabled
gh api -i .../vulnerability-alerts -> 204 (enabled); open dependabot alerts: 0
gh api .../community/profile -> health 87 %, issue_template false, pull_request_template false
gh release list -> v0.1.82 (Latest), v0.1.81, v0.1.80
gh run list -> Release [push v0.1.82] success; Release [push v0.1.81] failure; all CI runs success
packaging/homebrew/metrumbench.rb:35 assert_match "Sweep and benchmark" -> metrum-ai-bench --help matches 0; :36 "Deterministic mock" -> mock-server --help matches 1
curl crates.io/api/v1/crates/metrumbench -> "crate `metrumbench` does not exist"
rustup toolchain list -> stable, 1.80 (no 1.85 for an MSRV check)
cargo package --list --allow-dirty -> includes CLAUDE.md README-METRUMBENCH-ASR.md endpoints-4servers.yaml docs/CAMPAIGN.md docs/HISTORY_REWRITE.md docs/QUALITY_ASSESSMENT_PROMPT.md docs/QUALITY_ASSESSMENT_REPORT.md docs/SMOKE_RESULTS.md test-data/dummy-endpoints-4.yaml
tracked files without 'Copyright (c) 2026 Metrum AI': .gitignore CHANGELOG.md THIRD_PARTY_LICENSES dummy-model-server/go.mod endpoints-4servers.yaml test-data/dummy-endpoints-4.yaml
workflow 'uses:' pins: 20, all mutable tags
```
---

## 13. GitHub issues filed

Every finding in Sections 4 and 12 was filed in `metrum-ai/bench-cli` on
2026-09-15 with labels `security`, `compliance`, `measurement`,
`oss-readiness`, and `severity/*`. F-17 (advisories) is filed as O-03 and
F-26 through F-29 are grouped into F-25.
After review the tracker was split: #42 is the measurement epic (root-cause
PR groups A-D), and a separate compliance epic holds O-01 through O-12.
Issues #19 and #35 are cross-linked to #18 and #34 to #21 as shared root
causes; they stay open as separate acceptance criteria.

| Finding | Issue |
|---|---|
| O-01 | https://github.com/metrum-ai/bench-cli/issues/6 |
| O-02 | https://github.com/metrum-ai/bench-cli/issues/7 |
| O-03 | https://github.com/metrum-ai/bench-cli/issues/8 |
| O-04 | https://github.com/metrum-ai/bench-cli/issues/9 |
| O-05 | https://github.com/metrum-ai/bench-cli/issues/10 |
| O-06 | https://github.com/metrum-ai/bench-cli/issues/11 |
| O-07 | https://github.com/metrum-ai/bench-cli/issues/12 |
| O-08 | https://github.com/metrum-ai/bench-cli/issues/13 |
| O-09 | https://github.com/metrum-ai/bench-cli/issues/14 |
| O-10 | https://github.com/metrum-ai/bench-cli/issues/15 |
| O-11 | https://github.com/metrum-ai/bench-cli/issues/16 |
| O-12 | https://github.com/metrum-ai/bench-cli/issues/17 |
| F-01 | https://github.com/metrum-ai/bench-cli/issues/18 |
| F-02 | https://github.com/metrum-ai/bench-cli/issues/19 |
| F-03 | https://github.com/metrum-ai/bench-cli/issues/20 |
| F-04 | https://github.com/metrum-ai/bench-cli/issues/21 |
| F-05 | https://github.com/metrum-ai/bench-cli/issues/22 |
| F-06 | https://github.com/metrum-ai/bench-cli/issues/23 |
| F-07 | https://github.com/metrum-ai/bench-cli/issues/24 |
| F-08 | https://github.com/metrum-ai/bench-cli/issues/25 |
| F-09 | https://github.com/metrum-ai/bench-cli/issues/26 |
| F-10 | https://github.com/metrum-ai/bench-cli/issues/27 |
| F-11 | https://github.com/metrum-ai/bench-cli/issues/28 |
| F-12 | https://github.com/metrum-ai/bench-cli/issues/29 |
| F-13 | https://github.com/metrum-ai/bench-cli/issues/30 |
| F-14 | https://github.com/metrum-ai/bench-cli/issues/31 |
| F-15 | https://github.com/metrum-ai/bench-cli/issues/32 |
| F-16 | https://github.com/metrum-ai/bench-cli/issues/33 |
| F-18 | https://github.com/metrum-ai/bench-cli/issues/34 |
| F-19 | https://github.com/metrum-ai/bench-cli/issues/35 |
| F-20 | https://github.com/metrum-ai/bench-cli/issues/36 |
| F-21 | https://github.com/metrum-ai/bench-cli/issues/37 |
| F-22 | https://github.com/metrum-ai/bench-cli/issues/38 |
| F-23 | https://github.com/metrum-ai/bench-cli/issues/39 |
| F-24 | https://github.com/metrum-ai/bench-cli/issues/40 |
| F-25 | https://github.com/metrum-ai/bench-cli/issues/41 |
| measurement epic | https://github.com/metrum-ai/bench-cli/issues/42 |
| compliance epic | https://github.com/metrum-ai/bench-cli/issues/43 |
