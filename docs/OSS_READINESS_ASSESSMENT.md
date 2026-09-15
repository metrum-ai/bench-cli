<!-- Copyright (c) 2026 Metrum AI, Inc. All rights reserved. -->

# Open-Source Readiness Assessment: metrumbench (tools/rustyphalanx/cli-tools)

Reviewed tree: `/home/cgadgil/src/insights-4x/common-dev-4.1/tools/rustyphalanx/cli-tools` at commit 400c2206b on branch common-dev-4.1, Cargo.toml version 0.1.80 (the brief cites 0.1.82; no such version exists in the tree). Assessment date: 2026-09-15. Scope: the Rust crate `metrumbench` (11 binaries, 9,804 lines of Rust), mcpserver/, prompt-tools/, scripts/, examples/, tests/, docs/, and the release tooling that ships with it. Everything else in the repository was read only to understand how the binaries are invoked.

Method followed in order: every Rust file read in full; the Python in mcpserver/ and prompt-tools/ read; every shell script, Makefile and Dockerfile read; README.md, CHANGELOG.md, WORKFLOW.md, RELEASE-PLAN.md, docs/ and the .cursor rules read; `cargo build --release`, `cargo test`, `cargo clippy --all-targets -- -W clippy::all` and `cargo fmt --check` run and recorded (Appendix 8.1, 8.2); one benchmark per modality run end to end against tools/dummy-model-server with the emitted JSONL and CSV inspected and the streaming numbers verified by hand against the server's known timing model (Appendix 8.3, 8.11, 8.12); four adversarial SSE inputs constructed and run against the built binary (Appendix 8.5).

## 1. Executive summary

Decision: NO-GO for open-sourcing in the current state. The repository can reach GO after wave (i) of the roadmap (section 7), estimated at about 10 engineer-weeks, provided the measurement defects are fixed rather than documented around.

The three blocking issues:

1. The measurement core produces wrong numbers in ordinary situations. A stream that yields no visible output token is counted as a success with a time to first token of 0 ms (A-01, reproduced). An SSE event split across two TCP reads is lost together with the event that follows it, silently shortening the completion and shifting TTFT to a later chunk (A-02, reproduced). There is no inter-token latency measurement at all; the value labeled TPOT is a per-request mean divided by N rather than N minus 1, and the VLM binary's TTFT is total latency divided by completion tokens rather than a measurement (A-03, reproduced). Six percentile implementations use two different estimators, neither of which matches the numpy default used by every comparison tool (A-04). The throughput window and error-rate denominators differ between binaries and, with ramp-up enabled, the window starts at an arbitrary point in the result-drain loop (A-05, A-08). Nothing is persisted until the run ends, so Ctrl-C or a panic loses everything (A-19, D-05, reproduced).

2. The crate does not build on current stable Rust (a transitive polars dependency, ethnum 1.5.2, fails to compile; fixed by a one-line lockfile update that is not in the tree), and every shipped binary refuses to run because a compiled-in license expired on July 31, 2026 (A-06, F-01, both reproduced). The maintainers have directed that the license mechanism be removed for the public release, with a date extension acceptable only as an internal stopgap; the NTP check is to remain as an opt-in flag that records clock offset rather than a mandatory gate (A-20).

3. The tree is not curated for an audience: no LICENSE, NOTICE, CONTRIBUTING, SECURITY or CI that runs tests (F-06); no crates.io metadata (F-07); a real SSH public key, an example password, an AWS account ID, internal backup hostnames, public IPs of past targets and a personal email in code and docs (F-02); eleven Python stubs, a Dockerfile and a Makefile that only work inside the Metrum monorepo and publish to Restic and ECR (F-05); a README of 1,758 lines with 26 verified statements that contradict the code, including a charting module and a token-inference fallback that do not exist (H-02); and four benchmark binaries carrying four drifted copies of the metrics code with 49 tests, none of which checks a computed number (C-01, E-01).

What is better than expected, with evidence: the clock source for LLM, VLM and ASR intervals is `std::time::Instant` (monotonic) throughout; token counts come from the server `usage` field rather than from chunk counting, which is the right default when `usage` is present (verified against the dummy server: 320 of 320 tokens); the endpoints and prompt-input loaders in `src/` are the one part of the tree with reasonable unit tests; the streaming run's mean TTFT and latency matched the dummy server's timing model to within 1 ms and 5 ms respectively (Appendix 8.3); and metrumbench-imagegen, written later than the other three, already has per-request JSONL records, typed error classes, a schema version and a least-inflight balancer, which is the shape the rest of the crate should take.

The genuine differentiator, once the core is fixed: one static binary covering chat, vision, transcription (WER, CER) and image generation against OpenAI-compatible endpoints, with in-process multi-endpoint distribution. No comparison tool covers ASR or image generation.

## 2. Scorecard

Scores are 1 (unacceptable for a public measurement tool) to 5 (at or above the best comparison tool).

| Axis | Score | Justification |
|---|---|---|
| A. Measurement correctness | 1 | TTFT 0 ms on no-output streams, SSE split-event loss, no ITL, fabricated VLM TTFT, two percentile estimators, undefined window, closed-loop only, no seed, no ignore_eos, no warmup, nothing persisted (A-01 to A-20). |
| B. Statistical rigor | 1 | No std, CI, sample counts or repeat runs; p99 reported as the max at n below 100 without warning; undefined statistics serialized as 0 (B-01 to B-04). |
| C. Architecture and duplication | 2 | Four drifted copies of the metrics path; no library API; inconsistent CLI; utilities and an ISO customizer in a benchmarking crate. Imagegen shows the right shape (C-01 to C-04). |
| D. Code quality and robustness | 2 | No unguarded panic found in the LLM hot path and clippy is clean of bug-class lints, but the SSE parser fails 4 of 4 adversarial inputs, Ctrl-C loses the run, error classification is lost to string parsing, and `cargo fmt --check` fails (D-01 to D-08). |
| E. Test coverage | 1 | 49 tests; zero on any metric, parser byte path, load loop or end-to-end run; two of them pin the license date (E-01, E-02). |
| F. Open-source readiness blockers | 1 | Build broken on stable, license time bomb, secrets and keys in docs, no governance files, no Cargo metadata, release tooling bound to private infrastructure; dependency licenses are compatible (F-01 to F-07). |
| G. Competitive standing | 2 | Behind GenAI-Perf, vLLM, guidellm and MLPerf on every methodology feature in the matrix; ahead only on modality breadth and multi-endpoint distribution (section 6). |
| H. Documentation | 1 | No metric is defined precisely enough to reimplement; 26 code-contradicting README statements; version inconsistent across five places; a published result cannot be reproduced from the repository (H-01 to H-04). |

Finding counts by severity: CRITICAL 12 (A-01, A-02, A-03, A-04, A-05, A-06, A-07, A-08, C-01, E-01, F-01, F-06), HIGH 25 (A-09, A-10, A-11, A-12, A-13, A-14, A-15, A-16, A-17, A-18, A-19, A-20, B-01, B-04, D-01, D-04, D-05, E-02, F-02, F-04, F-05, F-07, H-01, H-02, H-03), MEDIUM 9, LOW 3. Of the 49 findings, 33 are marked as blocking open-source release.

## 3. Findings, ordered by severity

Severity definitions used throughout: CRITICAL produces incorrect published numbers or legally blocks release. HIGH materially misleads users or will draw credible public criticism. MEDIUM is a correctness or maintainability risk a reviewer will notice. LOW is polish.

Status labels: VERIFIED means the reviewer read the code body or ran the command and observed the behavior. SUSPECTED means the pattern was identified but not exercised. UNDETERMINED means the reviewer could not establish the fact with the information available, and the finding states what would be needed.

Line numbers refer to the files as they exist in this working tree at commit 400c2206b (branch common-dev-4.1). Cargo.toml declares version 0.1.80, not 0.1.82 as the assessment brief states; see finding H-06.

---
### 3.0 Direct answers to the assessment questions

Each answer cites the finding that carries the evidence.

| Question | Answer | Evidence |
|---|---|---|
| A1 Where is the TTFT clock read; before or after DNS, TCP, TLS; connection reuse; monotonic? | `Instant::now()` at metrumbench-llm.rs:772 before `send()`, so DNS, TCP and TLS are inside TTFT for every connection that is not already pooled; the pool is sized to concurrency but never pre-warmed; the clock is monotonic in llm, vlm and asr; metrumbench-imagegen uses `chrono::Utc::now()` (wall clock) for its headline latency. | A-12, A-18 |
| A2 What counts as the first token; reasoning deltas? | The first parsed chunk whose `delta.content` or `text` is non-empty, or whose `delta.function_call.arguments` or `tool_calls[].function.arguments` is non-empty (`stream_choice_has_output_token`, llm:1929-1955). Role-only and empty deltas are skipped correctly. `reasoning_content` is not recognized: there is no `reasoning_text_from_object` or any reference to reasoning in the tree, so a reasoning model's first reasoning token is not counted and a stream with only reasoning content yields TTFT 0 recorded as success. | A-01 |
| A3 Token counting method and error | Server `usage` only; no chunk counting, no tokenizer. Correct when `stream_options.include_usage` is honored (verified 320 of 320 tokens against the dummy server). When usage is absent the count is 0 and the run is still a success; the README's claimed word-based inference does not exist. Error introduced: 100 percent (zero) on servers without usage; 0 percent otherwise. | A-13, H-02 |
| A4 ITL versus TPOT | Not distinct: no ITL exists. TPOT is `(latency - ttft) / completion_tokens` per request (divides by N, not N minus 1: 4.0 percent low on the reference run). The first interval after TTFT is included in the numerator. Undocumented. | A-03, Appendix 8.3 |
| A5 Percentile estimator | Two nearest-order-statistic formulas, `round((n-1)p)` in llm and imagegen and `ceil(np)-1` in vlm and asr; six implementations; no interpolation; no minimum-sample warning; p99 equals the maximum for n up to about 100. | A-04, Appendix 8.6 |
| A6 Throughput definition | Total tokens (successes only) divided by a window that is `start_time.elapsed()` at print time (includes drain) or, with ramp-up, the elapsed time since an arbitrary instant in the drain loop. Request rate uses successes plus errors in llm, `num_requests` (including never-sent) in vlm, successes only in asr. In-flight requests at the end are always counted because the window closes after the last completion. | A-05, A-08 |
| A7 Load model; Poisson; coordinated omission | Closed-loop fixed concurrency via semaphore in all four binaries. No request-rate mode, no Poisson, no scheduled-arrival timestamps, so coordinated omission is neither measured nor correctable. | A-07 |
| A8 Warmup and steady state | No warmup phase. Ramp-up (llm) adds permits over time and discards results drained before the ramp deadline; steady state is assumed, never detected; vlm's ramp-up only paces launches. | A-05, A-12, C-01 |
| A9 Seeding and determinism | No `--seed` in llm, vlm or asr; `thread_rng` for prompt shuffle (llm), per-request record choice (vlm), prompt shuffle and random balancer (imagegen). `resolve_seed` exists only in imagegen and selects the seed sent to the server. Two runs with identical arguments do not produce identical request sequences. Refuted. | A-10 |
| A10 Output length control | `max_tokens` only; no `ignore_eos`, `min_tokens`, `seed`, `top_p`; temperature default 0.1; hidden system prompt on every chat request; only imagegen accepts extra body fields. | A-09 |
| A11 Input length and prefix caching | Word count only (whitespace split); no tokenizer; prompts cycled unmodified so every request beyond the dataset size is a prefix-cache hit; shared system prompt guarantees a common prefix. | A-11 |
| A12 Error accounting and goodput | Failed requests are excluded from latency and included in llm request rate; no goodput or SLO; silent success paths: no output token (TTFT 0), missing usage (0 tokens), split SSE events (lost content). No `unwrap_or` on the hot path hides a transport error, but `first_token_time.unwrap_or_default()` at llm:1034 hides the absence of a token. | A-01, A-02, A-13 |
| A13 Concurrency model and client saturation | Semaphore-bounded spawns, one `JoinHandle` per request retained until the end; default tokio runtime; no client overhead measurement. Measured ceiling on this 16-core host with the dummy server co-located: about 16.7k streaming req/s at 8 tokens, about 6.0k streaming req/s at 64 tokens (380k SSE tokens/s), about 40k non-streaming req/s, using 4 to 5 cores; resident memory grows about 1.5 KB per completed request. | A-19, D-03, Appendix 8.7 |
| A14 Multi-endpoint behavior | Static weighted round-robin chosen at launch (llm, vlm, asr); imagegen adds least-inflight and random. Per-endpoint vectors exist but the headline `metrics` block pools all endpoints into one distribution; vlm and asr per-endpoint JSONL has counts and rates only. | A-15 |
| A15 NTP dependency | Mandatory at startup in 10 of 11 binaries; contacts four public pools; retries for 30 s; aborts the run if unreachable or offset above 1 s; the offset is discarded, not recorded. It is not used for distributed-clock correction; its only downstream consumer is the license check. Direction from the maintainers: keep as an opt-in flag that records the offset. | A-20 |
| A16 ASR | WER: whitespace split, case-insensitive exact word match, standard Levenshtein; no punctuation, number or spelling normalization (Whisper normalizer absent), so punctuated hypotheses score 100 percent WER against unpunctuated references. RTF is `inference_time / audio_duration` (inverse of the leaderboard's RTFx); `inference_time` is a server field if present else client elapsed, mixed silently; audio file read is inside the timer. | A-16 |
| A16 VLM | Image fetch, decode, optional resize, JPEG re-encode and base64 happen on the launch thread while holding a concurrency permit, before the request timer starts; excluded from latency, never reported, and serializing concurrency on cache misses. Image tokens are whatever the server reports in `usage`. The LRU cache hides fetch cost from measurement by design and from the report by omission. TTFT is fabricated as `latency / completion_tokens`. | A-17, A-03 |
| A16 ImageGen | Latency unit is integer milliseconds from wall-clock differences (`num_milliseconds()`); image base64 decode, PNG decode for dimensions, SHA-256 and file write happen inside the attempt timer and therefore inside the client latency; not attributed separately. | A-18 |
| B1 to B5 | No std, CI, or run-to-run variance anywhere; no repeat-run support; no outlier handling or disclosure; no sample-size validation; every rate field is a bare scalar (list in B-04). | B-01 to B-04 |
| C1 Duplication | About 2,100 of 5,203 lines in llm, vlm and asr are structural duplicates; drift confirmed in the percentile estimator, error rate denominator, request-rate denominator, steady-state definition, ramp-up algorithm, error classification and logger fallback level. | C-01 |
| C4 Library API | None for measurement; `src/lib.rs` exports only endpoints, prompt loaders, banner, NTP, license, IDs. | C-02 |
| C5 CLI | Four Args structs with different defaults, requiredness and `--url` semantics; no vocabulary match to GenAI-Perf or vLLM flags. | C-03 |
| D1 Panics | No unguarded panic found in the LLM drain path; ASR has silent NaN and inf propagation on zero inference time; any panic anywhere loses the run because nothing is persisted. | D-01, A-19 |
| D4 Streaming robustness | Fails split events, missing DONE, role-only streams; no test feeds bytes through the parser. | A-02, A-14, D-04 |
| D5 Ctrl-C | Loses everything; exit 130 with no output file (reproduced). | D-05 |
| D7 clippy | 65 warning lines, 49 unique locations across 13 lints, none a runtime bug; `cargo fmt --check` fails on one signature. | D-07, Appendix 8.2 |
| E1 Coverage | 49 tests; none on metric computation, parser bytes, load loop or e2e; llvm-cov not installed; structural gap stated per function. | E-01 |
| F1 License gate | Defined at lib.rs:281-322; ten call sites; bypass `METRUM_SKIP_LICENSE_CHECK`; removal plan with tests given. | F-01 |
| F3 Licenses | All 470 packages permissive; NOTICE entries needed for webpki-roots (CDLA-Permissive-2.0) and xxhash-rust (BSL-1.0); polars-arrow-format uses a license file. | F-03 |
| F5 Platform coupling | No PostgREST, JWT or control plane needed at runtime by the binaries; all release, distribution, Docker and helper scripts require the monorepo, Restic, ECR or a Google Chat webhook; mcpserver is an empty package. | F-05 |
| H3 Reproducibility | Not possible from the repository alone: dataset unshipped, order unseeded, sampling incomplete, window undefined, environment unrecorded, per-request records absent, binaries expired. | H-03 |


### [CRITICAL] A-01 A request whose stream never produced an output token is recorded as a success with TTFT = 0

- Axis: A2, A12
- Status: VERIFIED (code read; reproduced against an adversarial SSE server, see Appendix 8.5 "roleonly")
- Location: src/bin/metrumbench-llm.rs:1034; src/bin/metrumbench-llm.rs:863; src/bin/metrumbench-llm.rs:1744-1754
- What the code does: In streaming mode `first_token_time` starts as `None` (line 863) and is set only when `stream_choice_has_output_token` returns true. At the end of the stream the function returns the tuple with `first_token_time.unwrap_or_default()` (line 1034). `Duration::default()` is zero. The caller at lines 1709-1754 treats every `Ok(...)` as a success and pushes the zero TTFT into `ttft_times`.

```rust
        trace!("Request completed successfully");
        let completion_word_count = count_words(&completion_text);
        Ok((
            start_time.elapsed(),
            first_token_time.unwrap_or_default(),
            prompt_tokens,
            completion_tokens,
            total_tokens,
            prompt_word_count,
            completion_word_count,
        ))
```

- Why it is wrong: A stream that carries only a role delta and a finish chunk (which is what a server returns when the model emits an empty completion, when content filtering suppresses output, or when a reasoning model returns only `reasoning_content` which this parser does not recognize) is counted as a successful request with a 0 ms time to first token. Every TTFT aggregate (min, avg, p50, p90, p95, p99) is pulled toward zero. TPOT for such a request becomes `response_time / completion_tokens` (line 1733-1743) because `ttft` is zero, which silently redefines TPOT for that sample. The min TTFT in the JSONL output will read `0`.
- Failure scenario: 100 requests to a vLLM endpoint running a reasoning model configured with `--reasoning-parser deepseek_r1` and `max_tokens` small enough that only reasoning tokens are produced. All 100 streams contain `delta.reasoning_content` and no `delta.content`. The run reports 0 errors, TTFT min/avg/p50/p99 all 0 ms, and completion_words 0, while `completion_tokens` from `usage` is non-zero. A vendor could publish "0 ms TTFT" from this output.
- Comparison: vLLM's benchmark_serving.py marks a request as failed (`output.success = False`) when no token was received; GenAI-Perf drops requests without a first response from the TTFT distribution and reports the count. LLMPerf raises on empty generations.
- Fix: Make `first_token_time` an `Option<Duration>` in the result type; record the request as an error of type `no_output_token` when it is `None`, or at minimum exclude it from the TTFT distribution and report the count of such requests in the summary. Add `reasoning_content` and `reasoning` deltas to `stream_choice_has_output_token` with a separate flag so the first reasoning token and the first visible token are both recorded.
- Effort: S
- Blocks open-source release: yes

---

### [CRITICAL] A-02 SSE events split across TCP reads are dropped, and the following event is discarded with them

- Axis: D4, A2, A3
- Status: VERIFIED (code read; reproduced against an adversarial SSE server, see Appendix 8.5 "split")
- Location: src/bin/metrumbench-llm.rs:881-897; src/bin/metrumbench-llm.rs:952-975
- What the code does: Each `bytes` item from `response.bytes_stream()` is decoded with `String::from_utf8_lossy` and iterated with `text.lines()` (line 885). A line that starts with `data: ` has its payload appended to `json_buffer` (line 896). If the JSON does not parse and the error is EOF, the code `continue`s with the buffer intact. Any line that does not start with `data: ` is ignored (the `if` at line 895 has no `else`). When the next `data: ` line arrives, its payload is appended to the still-incomplete buffer, producing a concatenation of two objects that fails to parse with a non-EOF error, which increments `error_count` and clears the buffer (lines 959-974).

```rust
                    for line in text.lines() {
                        let line = line.trim();
                        // Skip empty lines and control messages
                        if line.is_empty() || line == "data: [DONE]" {
                            ...
                        }
                        if line.starts_with("data: ") {
                            json_buffer.push_str(&line[6..]);
                            // Attempt to parse the current buffer as JSON
                            match serde_json::from_str::<Value>(&json_buffer) {
```

- Why it is wrong: HTTP chunk boundaries do not align with SSE event boundaries. A `data:` line split across two reads loses the first fragment's continuation (it lacks the `data: ` prefix and is skipped), then poisons the buffer so that the next complete event is also lost. Two tokens' worth of content vanish from `completion_text`, the word count is wrong, and if the lost event was the first content chunk TTFT is attributed to the next surviving chunk. After three such incidents in one request the request is failed with "Too many JSON parsing errors" (line 963-970), so error rate rises with server chunking behavior rather than with server errors. The buffer is also cleared on any non-EOF parse error, so a legitimately large event that happens to be split at a point where the partial text is syntactically invalid but not EOF (for example inside a string escape) is also discarded.
- Failure scenario: A server behind an nginx proxy with `proxy_buffering off` emits 4 KB events; TCP segments arrive in 1448-byte pieces. Every event spanning a segment boundary is lost along with its successor. On a 300-token response roughly two thirds of the content is missing; `completion_words` is one third of the true value while `completion_tokens` from `usage` is correct, and the mismatch is not reported. The reproduction in Appendix 8.5 shows a 6-event stream where the tool's `completion_text` drops "Hello" and " world" and records only "! done".
- Comparison: vLLM's benchmark_serving.py uses `aiohttp` `content.iter_any()` with an explicit `data: ` line framing and accumulates partial lines across chunks; GenAI-Perf uses Triton's SSE client with correct framing; guidellm uses httpx's `aiter_lines()` which handles partial lines.
- Fix: Replace the per-read `text.lines()` loop with a byte accumulator that only extracts complete lines terminated by `\n`, keeps the trailing partial line in the buffer for the next read, and decodes UTF-8 only on complete lines (or use `tokio_util::codec::LinesCodec` / an `eventsource-stream` crate). Parse each complete `data:` payload independently; never concatenate payloads from different lines.
- Effort: S
- Blocks open-source release: yes

---

### [CRITICAL] A-03 TPOT is computed from a single per-request interval and named "time per output token"; there is no inter-token latency measurement at all

- Axis: A4, A3
- Status: VERIFIED
- Location: src/bin/metrumbench-llm.rs:1733-1743; src/bin/metrumbench-vlm.rs:1449-1459; src/bin/metrumbench-vlm.rs:661-666
- What the code does: In metrumbench-llm TPOT is `(response_time - ttft) / completion_tokens` where `completion_tokens` comes from the server `usage` field (lines 1733-1743). No per-chunk timestamps are recorded anywhere in `make_request`; the only clock reads are at the start (line 772), at first token (line 943), and at the end (line 1033). In metrumbench-vlm there is no streaming, so `ttft` is defined as `total_time / completion_tokens` (lines 661-666) and TPOT is `(total_time - ttft) / completion_tokens`.

```rust
                let tpot = if completion_tokens > 0 {
                    response_time.checked_sub(ttft).and_then(|gen_time| {
                        if gen_time.is_zero() {
                            None
                        } else {
                            Some(gen_time.as_secs_f64() / completion_tokens as f64)
                        }
                    })
                } else {
                    None
                };
```

- Why it is wrong: The per-request TPOT is a mean over the decode phase of one request. The reported "p99 TPOT" is therefore the 99th percentile of per-request means, which is a different and much smoother quantity than the 99th percentile inter-token latency that MLPerf, GenAI-Perf and vLLM report. Stalls inside a stream (preemption, KV cache swap, scheduler hiccups) are invisible. In metrumbench-vlm, TTFT is not measured at all: it is an arithmetic fraction of total latency and the JSONL field `ttft.p50_ms` is fabricated. A reviewer comparing the source will identify the VLM TTFT as invented.
- Failure scenario: Two servers each produce 200 tokens in 4.0 s after a 0.2 s TTFT. Server A emits tokens at a steady 19 ms. Server B emits 199 tokens in 1.0 s then stalls 3.0 s before the last token. Both report identical TPOT (19 ms) at every percentile. GenAI-Perf would report B's p99 ITL as 3000 ms.
- Comparison: vLLM's benchmark_serving.py records `itl` as a list of per-chunk deltas and reports mean/median/p99 ITL separately from `tpot = (latency - ttft) / (output_len - 1)`. GenAI-Perf reports both "Inter Token Latency" (per chunk, normalized by tokens per chunk) and "Output Token Throughput per Request". MLPerf Inference server scenarios constrain TPOT per token with a 99th percentile bound.
- Fix: Record `Instant::now()` for every chunk that carries an output token; emit a per-request vector of inter-chunk deltas; compute ITL percentiles over all deltas across all requests; keep TPOT as `(latency - ttft) / (completion_tokens - 1)` and document the `- 1`. Remove the VLM TTFT fabrication and either add streaming to metrumbench-vlm or report TTFT as not measured.
- Effort: M
- Blocks open-source release: yes

---

### [CRITICAL] A-04 Percentile estimators differ between binaries and none is documented; p99 at small n is reported without warning

- Axis: A5, B4, C1
- Status: VERIFIED (code read; numeric comparison in Appendix 8.6)
- Location: src/bin/metrumbench-llm.rs:280-290; src/bin/metrumbench-llm.rs:474-477; src/bin/metrumbench-llm.rs:1155-1173; src/bin/metrumbench-vlm.rs:261-271; src/bin/metrumbench-asr.rs:288-307; src/bin/metrumbench-imagegen.rs:1077-1080
- What the code does: metrumbench-llm and metrumbench-imagegen use `round((n-1) * p / 100)` as the index into the sorted sample. metrumbench-vlm and metrumbench-asr use `ceil(n * p / 100) - 1`. There are six separate implementations of the index formula (three in metrumbench-llm alone: `calc_percentile` for `Duration`, the inline closure `calc_stats_f64`, and the two closures in `create_log_record`). None interpolates. No implementation checks that `n` is large enough for the requested percentile.

```rust
    // metrumbench-llm.rs:280
    fn calc_percentile(sorted_values: &[Duration], percentile: f64) -> Duration {
        if sorted_values.is_empty() {
            return Duration::default();
        }
        let index = (((sorted_values.len() - 1) as f64 * percentile / 100.0).round() as usize)
            .min(sorted_values.len() - 1);
```

```rust
    // metrumbench-vlm.rs:261 and metrumbench-asr.rs:288
    fn calc_percentile(sorted_values: &[Duration], percentile: f64) -> Duration {
        if sorted_values.is_empty() {
            return Duration::default();
        }
        let index =
            ((sorted_values.len() as f64 * percentile / 100.0).ceil() as usize).saturating_sub(1);
```

- Why it is wrong: The two formulas return different elements of the same sample. For n = 20 at p50 the LLM formula returns element 10 (round(9.5) rounds away from zero in Rust) while the VLM/ASR formula returns element 9. In the Appendix 8.6 run on a lognormal sample of 20 values the p50 differs by 8.6 percent between binaries (1134.31 versus 1044.67). Neither matches numpy's default linear interpolation (Hyndman and Fan type 7), which is what GenAI-Perf, vLLM benchmark_serving.py (`np.percentile`) and LLMPerf use, so cross-tool comparison of medians is invalid even on identical raw data. At n below 100 the "p99" is simply the maximum for both formulas, and the tool prints it without any warning; a p99 from a 20-request run is one sample.
- Failure scenario: A vendor runs metrumbench-llm with `--num-requests 50` and publishes p99 TTFT. The p99 is the single slowest request (index round(49*0.99) = 49). A competing vendor's GenAI-Perf run with the same server reports type-7 interpolation between the 49th and 50th order statistics. The numbers differ, both are "p99", and the dispute cannot be resolved from the documentation because the estimator is not described anywhere in README.md.
- Comparison: numpy default is type 7 linear interpolation. MLPerf LoadGen enforces a minimum query count per scenario (tens of thousands of queries for LLM server scenarios) precisely so that a p99 is meaningful, and states the estimator in its rules. GenAI-Perf prints the sample count next to every percentile table.
- Fix: One `stats` module with a single `percentile(sorted: &[f64], p: f64, method: Method) -> f64` implementing type 7 by default (with the method recorded in the JSONL output), unit tested against a table generated by numpy or R. Emit a warning and a `percentile_reliability` field when `n * (1 - p/100) < 1` (for p99 that means n below 100). Delete the five other implementations.
- Effort: S
- Blocks open-source release: yes

---

### [CRITICAL] A-05 Requests that fail before ramp-up completes are dropped from the error count while their absence still inflates the steady-state window denominator inconsistently; with ramp-up, throughput is divided by a window that starts at the first post-ramp completion, not at the first post-ramp send

- Axis: A6, A8, A12
- Status: VERIFIED
- Location: src/bin/metrumbench-llm.rs:1719-1731; src/bin/metrumbench-llm.rs:1795-1798; src/bin/metrumbench-llm.rs:1722-1724; src/bin/metrumbench-llm.rs:1188-1192; src/bin/metrumbench-llm.rs:1403-1405
- What the code does: Results are consumed in launch order (`for handle in handles`, line 1678). `metrics_start_time` is set to `Instant::now()` at the moment the first result is drained after the ramp-up wall time has elapsed (line 1724), which is an arbitrary point in the drain loop, not a request boundary. Successes drained before that instant are skipped entirely (line 1729); errors drained before that instant are not recorded (line 1796). Every throughput figure divides by `metrics_start_time.elapsed()` evaluated at print time (lines 375-379, 1188-1192), which includes the entire drain tail after the last request was launched.

```rust
                if let Some(ramp_up) = effective_ramp_up {
                    // Check if ramp-up period has completed
                    if !metrics_started && ramp_up_start.elapsed().as_secs() >= ramp_up {
                        metrics_started = true;
                        metrics.metrics_start_time = Some(Instant::now());
                        info!("Ramp-up complete. Starting metrics collection...");
                    }
                    // If still in ramp-up phase, skip recording success
                    if !metrics_started {
                        continue;
                    }
                }
```

- Why it is wrong: Because handles are awaited in launch order, the drain loop cannot reach a post-ramp result until every earlier request has completed. If request 1 takes 90 s, `metrics_start_time` is set at 90 s even with `--ramp-up-seconds 10`, and the requests launched between 10 s and 90 s that already completed are then counted as "steady state" against a window that began at 90 s. Requests per second can exceed the true rate by an arbitrary factor. Conversely, without ramp-up, the denominator is `start_time.elapsed()` at print time, which includes the drain phase during which concurrency falls from `concurrency` to zero, so throughput is understated relative to the steady-state rate; the CHANGELOG entry for v0.1.78 acknowledges the "late post-drain metrics window" for `--ramp-up-seconds 0` but the same defect remains for any positive ramp-up value. The JSONL fields `steady_state_seconds` and `metrics_collection_seconds` are both set to the same `metrics_elapsed_secs` (lines 1409-1410), so the output claims a steady-state measurement it does not have.
- Failure scenario: `--num-requests 400 --concurrency 40 --ramp-up-seconds 20` against a server with a bimodal latency of 1 s or 60 s. The first drained handle to satisfy `elapsed >= 20` is drained at roughly 60 s. All 360 or so completed requests drained after that instant are divided by (total_time - 60 s), giving a requests-per-second figure several times the true one.
- Comparison: vLLM benchmark_serving.py measures `benchmark_duration` from first request send to last request completion with no warmup subtraction, and documents it. GenAI-Perf runs a configurable warmup request count that is excluded, then measures over the measurement interval with in-flight requests at the boundary counted by completion time. MLPerf LoadGen defines the measurement window as the interval between the first and last issued query in the steady-state phase and requires a minimum duration.
- Fix: Attach `sent_at` and `completed_at` Instants to every request result. Define the measurement window explicitly: start at the first `sent_at` after the warmup boundary, end at the `completed_at` of the last request launched. Compute throughput as tokens completed inside the window divided by window length; report the window boundaries in the output. Record errors regardless of phase, tagged with phase.
- Effort: M
- Blocks open-source release: yes

---

### [CRITICAL] A-06 The crate does not build on current stable Rust, and the shipped binaries refuse to run because the compiled-in license expired on July 31, 2026

- Axis: F1, D1
- Status: VERIFIED (build output in Appendix 8.1; license check output in Appendix 8.4)
- Location: Cargo.lock:848-851 (ethnum 1.5.2); Cargo.toml:63 (polars dependency); src/lib.rs:288-290; src/lib.rs:300-321; call sites listed in F-01
- What the code does: `cargo build --release` with rustc 1.97.1 fails compiling `ethnum v1.5.2`, a transitive dependency of `polars 0.48.1`, with `error[E0512]: cannot transmute between types of different sizes`. polars is used only by three utility binaries (add_column_to_jsonl.rs:2, extract_prompts.rs:3, jsonl_to_csv.rs:2) but it is a crate-wide dependency, so no binary builds. Separately, `license::check_license` compares `Utc::now()` against a hard-coded `NaiveDate::from_ymd_opt(2026, 7, 31)` and returns `Err` after that date unless `METRUM_SKIP_LICENSE_CHECK` is set.

```rust
    pub fn license_expiry_date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 31).expect("Invalid expiry date")
    }
    ...
        if now > expiry {
            let msg = expired_license_message(license_expiry_date());
            error!("{}", msg);
            // line 315 prints a U+274C cross mark glyph followed by "Error: {msg}"
            println!("Please contact {} for renewal.\n", LICENSE_CONTACT);
            return Err(msg.into());
        }
```

- Why it is wrong: A public repository whose `cargo build` fails on stable is dead on arrival. A public binary that exits with "License expired ... Contact chetan@metrum.ai for renewal" under Apache 2.0 is a contradiction. Both are visible within the first minute of any external evaluation. The lockfile pin to ethnum 1.5.2 is resolved by `cargo update -p ethnum` (to 1.5.3), which the reviewer had to apply to complete this assessment; that change must land in the repository.
- Failure scenario: `git clone && cargo build --release` on a fresh machine with rustup stable fails. With the lockfile fixed, `metrumbench-llm --url ... ` prints the license error and exits non-zero on any date after 2026-07-31 (reproduced on 2026-09-15, Appendix 8.4).
- Comparison: Not applicable; none of the comparison tools has a time bomb or a broken lockfile.
- Fix: Update the lockfile (ethnum 1.5.3), add a `rust-toolchain.toml` and a CI job that builds on stable and MSRV. Remove `mod license` and all ten call sites (F-01). Move polars-dependent utilities into a separate crate or feature so the benchmark binaries do not depend on polars.
- Effort: S
- Blocks open-source release: yes

---

### [CRITICAL] A-07 No open-loop load generation: every binary is closed-loop at fixed concurrency, so the saturation knee and request-rate service levels cannot be measured

- Axis: A7
- Status: VERIFIED
- Location: src/bin/metrumbench-llm.rs:1586; src/bin/metrumbench-llm.rs:1635; src/bin/metrumbench-vlm.rs:1144; src/bin/metrumbench-vlm.rs:1201; src/bin/metrumbench-asr.rs:1345; src/bin/metrumbench-asr.rs:1371; src/bin/metrumbench-imagegen.rs:315; src/bin/metrumbench-imagegen.rs:321
- What the code does: Each binary creates a `tokio::sync::Semaphore` with `concurrency` permits and calls `acquire_owned().await` before spawning each request. There is no `--request-rate`, no arrival-time schedule, no Poisson or gamma inter-arrival option, and no timestamps recording intended versus actual send time.

```rust
    let semaphore = Arc::new(Semaphore::new(initial_permits));
    ...
        let permit = semaphore.clone().acquire_owned().await?;
        let client = client.clone();
        let prompt = prompts[i % prompts.len()].clone();
```

- Why it is wrong: Closed-loop load adapts to the server: when the server slows down, the client sends less. The measured throughput is therefore always "what the server can do at this concurrency", never "how the server behaves at a fixed offered load". It cannot find the knee of the latency-throughput curve, cannot reproduce an MLPerf server scenario (Poisson arrivals at a target QPS with a latency constraint), and suffers from coordinated omission: slow responses suppress the arrivals that would have observed the slowness. The README markets the tool for capacity claims ("Identifying performance degradation points", README.md:674), which this design cannot support.
- Failure scenario: A server with a hard throughput ceiling of 10 req/s is benchmarked at `--concurrency 100`. Requests queue on the server, latency rises to 10 s, throughput reads 10 req/s, and the tool reports a valid-looking result. An open-loop test at 12 req/s would show unbounded queue growth and expose the ceiling; this tool cannot express that test.
- Comparison: GenAI-Perf has `--request-rate` with Poisson or constant arrivals; vLLM benchmark_serving.py has `--request-rate` with `--burstiness` (gamma inter-arrival) and `--max-concurrency`; guidellm has `--rate-type poisson|constant|sweep`; MLPerf LoadGen's Server scenario is Poisson by definition; InferenceX runs concurrency sweeps but publishes the full curve.
- Fix: Add an arrival scheduler: `--request-rate <rps>` with `--arrival {constant,poisson}` seeded from `--seed`, spawning requests at scheduled times regardless of outstanding count (with an optional hard `--max-concurrency` cap that is reported as a distinct "backpressure engaged" condition). Record `scheduled_at`, `sent_at`, and `completed_at` per request so coordinated omission can be corrected (latency measured from `scheduled_at`).
- Effort: L
- Blocks open-source release: yes (for a tool that will be compared to GenAI-Perf and MLPerf)

---

### [CRITICAL] A-08 Metric code is duplicated four times and the copies have drifted: TTFT, TPOT, percentile, throughput denominator and error rate are all computed differently per binary

- Axis: C1, A6, A12
- Status: VERIFIED
- Location: Enumerated in section C-01 below; representative drift: src/bin/metrumbench-llm.rs:1394-1396 versus src/bin/metrumbench-vlm.rs:810 versus src/bin/metrumbench-asr.rs:1128; src/bin/metrumbench-llm.rs:1403-1405 versus src/bin/metrumbench-vlm.rs:814
- What the code does: The error rate in metrumbench-llm is `errors / (successes + errors) * 100` (line 1394-1396). In metrumbench-vlm it is `errors / args.num_requests * 100` (line 810), which is wrong whenever `--stop-after-seconds` truncates the run or an image load failure `continue`s past a request without ever spawning it. Requests per second in metrumbench-vlm is `args.num_requests / elapsed` (line 814), which counts requests that were never sent. The VLM `steady_state_seconds` is `elapsed - ramp_up_seconds` (line 818) while the LLM version is the metrics window (line 1410). The ASR `requests_per_second` in `throughput` (line 1124) uses successes only, while `request_rate.requests_per_second` (line 1147) uses completed count, and `timing` has no rate at all.

```rust
            // metrumbench-vlm.rs:808-820
            "errors": {
                "count": metrics.errors.len(),
                "rate": (metrics.errors.len() as f64 / args.num_requests as f64) * 100.0
            },
            "timing": {
                "total_time_seconds": metrics.start_time.elapsed().as_secs_f64(),
                "requests_per_second": args.num_requests as f64 / metrics.start_time.elapsed().as_secs_f64(),
```

- Why it is wrong: The same field name (`metrics.errors.rate`, `metrics.timing.requests_per_second`) means different things in different binaries' output files, and the downstream ingestion in insights-cli (`output_ingestion.py:883-947`) maps them into one database schema. A consumer comparing an LLM run to a VLM run is comparing incompatible definitions. Drift between copies of metric code means any future fix must be applied four times and will be missed at least once, which is exactly what has already happened.
- Failure scenario: `metrumbench-vlm --num-requests 1000 --stop-after-seconds 30` sends 120 requests, 10 fail. Output reports `errors.rate = 1.0` (percent) and `requests_per_second = 1000 / 31`. The true error rate is 8.3 percent and the true request rate is 120 / 31.
- Comparison: All comparison tools compute metrics in one place per tool.
- Fix: See section 4 (refactoring plan). A single `metrics` module with one `Summary::from_records(&[RequestRecord], window)` used by all binaries.
- Effort: L (as part of the refactor), S for the immediate VLM denominator fix
- Blocks open-source release: yes

---

### [HIGH] A-09 Output length is not controlled: `ignore_eos` is never set, `temperature` defaults to 0.1 not 0, no `seed` is sent, and `max_tokens` is only an upper bound

- Axis: A10, A9
- Status: VERIFIED
- Location: src/bin/metrumbench-llm.rs:1434-1473; src/bin/metrumbench-llm.rs:100-101; src/bin/metrumbench-vlm.rs:993-1049; src/bin/metrumbench-vlm.rs:92-93
- What the code does: `build_request_body` in metrumbench-llm sends `model`, `max_tokens`, `temperature`, `stream`, `messages` or `prompt`, and `stream_options.include_usage` when streaming. Nothing else. There is no `ignore_eos`, no `min_tokens`, no `seed`, no `top_p`, no way to pass extra body fields. Temperature defaults to 0.1. metrumbench-vlm's `build_request_body` sends `model`, `messages`, `max_tokens`, `temperature` and never sets `stream` (it is always non-streaming). metrumbench-imagegen is the only binary with `--extra-body-json` (line 143-147).

```rust
        MetrumBenchLLMMode::Chat => json!({
            "model": model,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "stream": streaming,
            "messages": [
                {"role": "system", "content": "You are a helpful assistant."},
                {"role": "user", "content": prompt}
            ]
        }),
```

- Why it is wrong: Without `ignore_eos: true` (vLLM, SGLang, TensorRT-LLM all support it) the number of decode steps depends on when the model chooses to stop, which depends on the model, the sampling temperature and the prompt. Output-token throughput comparisons across models or across quantizations are then comparing different workloads. The README acknowledges the problem in the section titled "Why It's STUPID to Rely Only on max_tokens Parameter" (README.md:991-1035) and recommends prompt engineering as the mitigation, which is not a control. The hard-coded system prompt "You are a helpful assistant." (line 1449) is added to every chat request and changes the prompt token count relative to what the user's dataset specifies.
- Failure scenario: Model A (verbose) and Model B (terse) are benchmarked with `--max-tokens 512`. A generates 500 tokens per request, B generates 80. B shows 6x higher requests per second and lower "completion tokens per second"; neither number says anything about serving efficiency.
- Comparison: vLLM's benchmark_serving.py exposes `--ignore-eos` and sizes `max_tokens` per request from the dataset's output length; GenAI-Perf has `--output-tokens-mean` with `--output-tokens-mean-deterministic` (Triton `min_tokens`) and `--extra-inputs ignore_eos:true` for OpenAI endpoints; guidellm pins output length per request and sends `ignore_eos` to vLLM-compatible servers; MLPerf fixes output length by dataset and reference.
- Fix: Add `--ignore-eos` (default on for non-OpenAI backends), `--min-tokens`, `--seed`, `--top-p`, `--extra-body-json` (as metrumbench-imagegen already has), and `--no-system-prompt` / `--system-prompt <text>`. Record every sampling parameter in the output `config` block.
- Effort: S
- Blocks open-source release: yes

---

### [HIGH] A-10 No seed, no determinism: prompt order is shuffled with `thread_rng` and cannot be reproduced

- Axis: A9
- Status: VERIFIED
- Location: src/bin/metrumbench-llm.rs:1577; src/bin/metrumbench-vlm.rs:1203-1205; src/bin/metrumbench-vlm.rs:1218-1220; src/bin/metrumbench-vlm.rs:1304-1306; src/bin/metrumbench-imagegen.rs:562-565; src/bin/metrumbench-imagegen.rs:711-715; src/bin/metrumbench-imagegen.rs:731-739
- What the code does: metrumbench-llm shuffles the prompt list once with `rand::thread_rng()` (line 1577) and then cycles it. metrumbench-vlm picks a random record per request with `records.choose(&mut rand::thread_rng())` (line 1204) and picks additional images per batch the same way. metrumbench-imagegen has `--seed` and `--seed-mode`, but `resolve_seed` (line 731) only determines the `seed` value sent to the server in the request body; the client-side `--shuffle-prompts` uses `rand::thread_rng()` (line 564) and the `Random` load balancer uses `rand::thread_rng()` (line 713). There is no `--seed` flag in metrumbench-llm, metrumbench-vlm or metrumbench-asr. The brief's question "find resolve_seed" is answered: it exists only in metrumbench-imagegen and does not seed any client-side random choice.

```rust
    // metrumbench-llm.rs:1576-1577
    // Shuffle prompts once for even distribution, then cycle through them deterministically
    prompts.shuffle(&mut rand::thread_rng());
```

- Why it is wrong: Two runs with identical arguments send different prompt sequences. With prefix caching enabled on the server (vLLM default since v0.6), the order in which repeated prompts arrive changes cache hit rates and therefore TTFT. A published result cannot be reproduced even by its author. The comment "then cycle through them deterministically" is misleading: the cycle is deterministic given the shuffle, but the shuffle is not.
- Failure scenario: Same server, same dataset of 8 prompts, `--num-requests 64 --concurrency 8`. Run 1 happens to send the same prompt to all 8 initial slots; run 2 sends 8 distinct prompts. Run 1's TTFT p50 is lower by the prefill cost of 7 cached prompts.
- Comparison: vLLM benchmark_serving.py has `--seed` that seeds `random` and `numpy`; GenAI-Perf has `--random-seed`; guidellm has `--random-seed`; MLPerf LoadGen has fixed seeds published per round.
- Fix: Add `--seed <u64>` to every binary; construct `rand::rngs::StdRng::seed_from_u64(seed)` once and pass `&mut rng` to every `shuffle`, `choose`, `gen_range`, and to the arrival scheduler. Record the seed in output. Add a test that two runs against the dummy server with the same seed produce byte-identical request sequences (captured by the server's request log).
- Effort: S
- Blocks open-source release: yes

---

### [HIGH] A-11 Prompt datasets are cycled unmodified, so every request after the first `len(prompts)` hits a warm prefix cache; input length is never measured or controlled by the tool

- Axis: A11
- Status: VERIFIED
- Location: src/bin/metrumbench-llm.rs:1637; src/bin/metrumbench-llm.rs:1449; src/bin/metrumbench-vlm.rs:1203-1205; src/bin/metrumbench-asr.rs:1375
- What the code does: `prompts[i % prompts.len()]` (line 1637) selects the prompt. There is no uniqueness enforcement, no random prefix injection, no per-request salt, and no token counting on the client. Prompt "length" is reported as a whitespace word count (`count_words`, line 1917-1919). The system prompt "You are a helpful assistant." is identical on every request, guaranteeing a shared prefix even across distinct user prompts.
- Why it is wrong: On vLLM with `enable_prefix_caching` (default on), any repeated prompt skips prefill. With a 100-prompt dataset and 1000 requests, 90 percent of requests have a fully cached prefill and TTFT collapses. The tool reports this as server performance. Input-length control is delegated entirely to the dataset author; the tool cannot state the tokenized input length it actually sent, only a word count, and README.md:263 and README.md:560 claim the tool "can infer the token counts from the word counts", which no code path implements (see H-02).
- Failure scenario: `--num-requests 1000 --prompts 8-line file`. After the first 8 requests, every request is a prefix-cache hit. Reported TTFT p50 is a fraction of the value a fresh-prompt run would show; a vendor publishes the smaller number.
- Comparison: vLLM benchmark_serving.py supports `random` datasets with per-request random token prefixes and `--random-prefix-len`; GenAI-Perf synthesizes unique prompts per request with a tokenizer and reports `Input Sequence Length` statistics in tokens; guidellm generates prompts with `prompt_tokens` control and unique random text; InferenceX runs with prefix caching explicitly disabled or documented.
- Fix: Add `--unique-prompts` (default on) that prefixes each prompt with a per-request random token sequence of configurable length (seeded), or a per-request nonce; integrate a tokenizer (the `tokenizers` crate can load Hugging Face tokenizer.json files) to measure and report input tokens; expose `--system-prompt none`. Document server-side prefix caching as a confound.
- Effort: M
- Blocks open-source release: yes

---

### [HIGH] A-12 TTFT is measured from before connection establishment and includes TLS handshake and the request body upload, with no connection warmup

- Axis: A1
- Status: VERIFIED
- Location: src/bin/metrumbench-llm.rs:772; src/bin/metrumbench-llm.rs:798-805; src/bin/metrumbench-llm.rs:1566-1572; src/bin/metrumbench-asr.rs:732; src/bin/metrumbench-vlm.rs:596
- What the code does: `start_time = Instant::now()` is taken before `client.post(url)...send().await`. The clock source is `std::time::Instant` (monotonic), which is correct. The reqwest `Client` is built with `pool_max_idle_per_host(concurrency)` and a 60 s idle timeout, so connections are reused once established, but nothing pre-establishes them: the first `concurrency` requests of every run each pay DNS, TCP and (for https) TLS setup inside their TTFT. There is no warmup phase (A8) so these requests are included in every distribution. In the ASR binary the timer also includes reading the audio file from disk (`tokio::fs::read`, line 746) and building the multipart body.
- Why it is wrong: For a 20-request run at concurrency 20, 100 percent of TTFT samples include connection setup. For https endpoints with a remote CA chain this is tens to hundreds of milliseconds, comparable to or larger than the true TTFT of a small model. The number reported as TTFT is "time to first byte including connection setup", which is not how vLLM or GenAI-Perf define it.
- Failure scenario: `--num-requests 20 --concurrency 20` against an https endpoint with 80 ms TLS handshake and 40 ms true TTFT. Reported TTFT avg is roughly 120 ms; the server's own metric says 40 ms.
- Comparison: vLLM's benchmark_serving.py uses a shared aiohttp session and also starts its TTFT clock before the POST, but it issues a single test request before the measured run and its per-request records let the reader drop the first samples; GenAI-Perf reuses gRPC/HTTP connections and has `--warmup-request-count`; MLPerf requires a warmup phase and excludes it.
- Fix: Add `--warmup-requests N` (default at least equal to concurrency) whose results are recorded in a separate `warmup` block and excluded from all statistics; optionally issue a `HEAD` or `GET /v1/models` per pooled connection before the measurement phase to force handshakes. Record `connect_ms` separately when reqwest exposes it, or measure the time to response headers versus time to first token as two fields.
- Effort: S
- Blocks open-source release: yes

---

### [HIGH] A-13 Token counts are taken exclusively from the server `usage` field with fallback to zero; missing usage silently produces zero tokens and infinite or zero rates

- Axis: A3
- Status: VERIFIED
- Location: src/bin/metrumbench-llm.rs:914-930; src/bin/metrumbench-llm.rs:1099-1113; src/bin/metrumbench-llm.rs:1019-1028; src/bin/metrumbench-vlm.rs:647-659; src/bin/metrumbench-vlm.rs:661-666
- What the code does: Streaming: each parsed chunk's `usage` object overwrites `prompt_tokens`, `completion_tokens`, `total_tokens` if present; chunks without `usage` increment `missing_token_stats`, and a warning is logged only if more than 50 percent of chunks lacked usage (lines 1019-1028), which is always true for OpenAI-style streams where only the final chunk carries usage, so the warning fires on every normal run and is meaningless. Non-streaming: `usage` missing yields `unwrap_or(0)`. No chunk counting, no local tokenization. metrumbench-vlm returns an error if `usage` is missing (line 647) but then divides `total_time` by `completion_tokens` for the fabricated TTFT.
- Why it is wrong: The method is correct when the server reports usage in the final chunk and `stream_options.include_usage` is honored. It is silently wrong on servers that do not honor `include_usage` (older TGI, some proxies, Azure OpenAI without the flag, llama.cpp before 2024-10): the request is a success with 0 completion tokens, TPOT is `None` (skipped, shrinking the sample without notice), and token throughput is 0. There is no client-side cross-check. A run with 0 total tokens and 0 errors is accepted. The README claims (README.md:263, 560) that the tool infers tokens from words when usage is absent; it does not.
- Failure scenario: Benchmark against an OpenAI-compatible gateway that strips `usage` from streams. Output: `completion.total = 0`, `completion.per_second = 0.0`, `tpot` block all zeros, `errors.count = 0`. The run is reported as a clean success.
- Comparison: vLLM benchmark_serving.py tokenizes the returned text with the model tokenizer and reports both; GenAI-Perf tokenizes output with the specified tokenizer; LLMPerf counts with the tokenizer; guidellm uses the tokenizer and falls back to usage.
- Fix: Treat a success with `completion_tokens == 0` and non-empty `completion_text` as an error class `usage_missing`; add optional tokenizer-based counting (`--tokenizer <path or HF id>`) using the `tokenizers` crate and report both `usage` and `tokenized` counts with their disagreement; delete the misleading 50 percent warning.
- Effort: M
- Blocks open-source release: yes

---

### [HIGH] A-14 Stream error handling loops on a failed stream and `[DONE]` is a soft requirement: a server that closes the connection without `[DONE]` is an error, while a server that sends `[DONE]` mid-content is a truncated success

- Axis: D4, A12
- Status: VERIFIED (code read; "nodone" reproduction in Appendix 8.5)
- Location: src/bin/metrumbench-llm.rs:980-1002; src/bin/metrumbench-llm.rs:1003-1015; src/bin/metrumbench-llm.rs:888-892
- What the code does: On `Some(Err(e))` from the byte stream the code logs, increments `error_count`, and unless `error_count >= 3` loops back to `stream.next().await` on the same errored stream (lines 980-1002). On `None` with `done == false` it returns an error "Stream ended unexpectedly" (lines 1003-1015). On `data: [DONE]` it sets `done = true` and breaks out of the inner loop without checking whether a `finish_reason` was ever seen.
- Why it is wrong: reqwest's body stream is fused after an error; polling it again yields `None`, so the `error_count` retry path can never recover and the "Too many stream errors" message can never be reached in practice, while a transport error after 199 of 200 tokens is reported as "Stream ended unexpectedly" with `first_token_time` as the only diagnostic (line 1005-1008 logs the TTFT under the label "Last received data"). A server that omits `[DONE]` (some OpenAI-compatible proxies, all SSE emitters that follow the Responses API) fails every request even though the content and `usage` arrived intact.
- Failure scenario: A gateway that terminates streams with a finish chunk and connection close but no `[DONE]` sentinel: 100 percent error rate, zero latency samples, exit code 1.
- Comparison: vLLM benchmark_serving.py treats `[DONE]` as optional and considers a request complete when the stream ends after receiving a finish chunk; GenAI-Perf uses the transport's end-of-stream.
- Fix: Consider a stream complete when either `[DONE]` is seen or the body ends after a chunk carrying `finish_reason`; classify body-ended-without-finish as `truncated_stream`; remove the retry loop on a fused stream.
- Effort: S
- Blocks open-source release: yes

---

### [HIGH] A-15 Multi-endpoint aggregate statistics pool samples from endpoints with different performance into one distribution, and the ASR/VLM per-endpoint JSONL omits latency percentiles

- Axis: A14
- Status: VERIFIED
- Location: src/endpoints.rs:83-95; src/bin/metrumbench-llm.rs:1624-1633; src/bin/metrumbench-llm.rs:400-410; src/bin/metrumbench-llm.rs:1208-1277; src/bin/metrumbench-vlm.rs:705-722; src/bin/metrumbench-asr.rs:1022-1039; src/bin/metrumbench-imagegen.rs:694-729
- What the code does: Endpoint selection is a static weighted round-robin index (`fetch_add % weighted_list.len()`, line 1629), applied at launch time regardless of endpoint health. The AGGREGATE block and the top-level `metrics` object pool all endpoints' response times, TTFTs and TPOTs into one sorted vector for percentiles (lines 400-410, 1132-1137). Per-endpoint JSONL in metrumbench-llm carries p50 and p99 response time (lines 1238-1239) but only averages for TTFT and TPOT (lines 1241-1250). metrumbench-vlm's `per_endpoint` carries only counts and requests per second (lines 713-719); metrumbench-asr's carries counts, characters and words (lines 1030-1036). Only metrumbench-imagegen offers `least-inflight` and `random` balancers (lines 694-729).
- Why it is wrong: The top-level `metrics.ttft.p99_ms` for a two-endpoint run is the 99th percentile of a mixture distribution. If endpoint A has 50 ms TTFT and endpoint B has 500 ms, the pooled p50 is whichever is more heavily weighted, and the pooled p99 is B's tail. Downstream ingestion (insights-cli output_ingestion.py:906-919) reads only the pooled values. Because round-robin does not react to a failing endpoint, a dead endpoint receives its full weight of requests and drags the pooled error rate up while the healthy endpoint's numbers are never visible as percentiles in the VLM and ASR outputs.
- Failure scenario: Two replicas, one degraded. `metrics.response_times.p50_ms` is the fast replica; `p99_ms` is the slow replica. The reader sees one server with a heavy tail. The per-endpoint blocks for VLM and ASR cannot disambiguate because they have no percentiles.
- Comparison: None of GenAI-Perf, vLLM benchmark_serving.py or guidellm accepts multiple endpoints; they expect a load balancer in front and measure the balanced service as one system. That is the defensible design: if the tool distributes load itself, per-endpoint distributions must be first-class and the aggregate must be labeled as a mixture.
- Fix: Make `EndpointMetrics` carry the full record vector and compute the same `Summary` per endpoint as for the aggregate; label the aggregate `pooled_mixture: true`; add `least_inflight` to the LLM/VLM/ASR binaries via the shared endpoints module (imagegen already has the code).
- Effort: M
- Blocks open-source release: no (feature is optional), but the pooled-only JSONL for VLM/ASR should be fixed before launch

---

### [HIGH] A-16 metrumbench-asr WER uses no text normalization beyond lowercase; RTF is inverted relative to the Open ASR Leaderboard's RTFx and is computed from a server-reported or client-measured time inconsistently

- Axis: A16
- Status: VERIFIED
- Location: src/bin/metrumbench-asr.rs:611-658; src/bin/metrumbench-asr.rs:661-709; src/bin/metrumbench-asr.rs:814-821; src/bin/metrumbench-asr.rs:1453-1458; src/bin/metrumbench-asr.rs:746-747
- What the code does: `word_error_rate` splits on whitespace, compares words case-insensitively (`to_lowercase()`, line 641), and runs a standard O(n*m) Levenshtein over words. There is no punctuation stripping, no number normalization, no British/American spelling normalization, no removal of filler words, no Unicode NFKC. `character_error_rate` compares characters including whitespace and punctuation. `inference_time` is taken from a non-standard `inference_time` field in the JSON response if present and finite, otherwise it is the client-measured elapsed time (lines 814-818), so RTF mixes server-side and client-side timings across requests without labeling which was used. RTF is `inference_time / audio_duration` (line 1458); the Open ASR Leaderboard defines RTFx as `audio_duration / processing_time` (higher is better). Audio file read time is inside the timed region (line 746 is after `start_time` at line 732).

```rust
            let substitution_cost =
                if ref_words[i - 1].to_lowercase() == hyp_words[j - 1].to_lowercase() {
                    0
                } else {
                    1
                };
```

- Why it is wrong: Whisper-style output "Hello, world." versus reference "hello world" is scored as 2 substitutions out of 2 words: WER 100 percent. Every published ASR WER (Open ASR Leaderboard, Whisper paper, NeMo) applies the Whisper English normalizer or an equivalent before scoring; a WER from this tool is not comparable to any published WER and will be several times higher for punctuated hypotheses. Reporting RTF where the field is understood to be RTFx (or vice versa) inverts the ranking. Mixing server-reported and client-measured times in one distribution is not a measurement.
- Failure scenario: Reference "hello world", hypothesis "Hello, world." Tool WER = 1.0. jiwer with the Whisper normalizer = 0.0.
- Comparison: Open ASR Leaderboard uses `EnglishTextNormalizer` from Whisper and jiwer; reports WER and RTFx = total audio seconds / total transcription seconds computed client-side on a fixed dataset with a warmup.
- Fix: Port the Whisper basic/English normalizer (or vendor an existing Rust port) and apply it to both reference and hypothesis; expose `--normalizer {whisper-english,whisper-basic,none}`; report RTFx as audio seconds divided by client-measured wall time with the definition in the field name (`rtfx_client`); if a server-side time is read, put it in a separate field. Move the file read before `start_time`.
- Effort: M
- Blocks open-source release: yes (for the ASR binary)

---

### [HIGH] A-17 metrumbench-vlm image decode, resize, JPEG re-encode and base64 encoding happen on the launch thread before the request is spawned, are excluded from request latency, and the LRU cache makes the first request per image pay this cost inside the launch loop, serializing all concurrency

- Axis: A16, A13
- Status: VERIFIED
- Location: src/bin/metrumbench-vlm.rs:1201; src/bin/metrumbench-vlm.rs:1256-1264; src/bin/metrumbench-vlm.rs:859-949; src/bin/metrumbench-vlm.rs:926-936; src/bin/metrumbench-vlm.rs:596
- What the code does: The semaphore permit is acquired at line 1201. Then, still on the main task and before `tokio::spawn`, `image_cache.get_or_load(...).await` fetches or reads the image, decodes it with the `image` crate, optionally resizes with Lanczos3, converts to RGB8, re-encodes as JPEG at default quality, and base64-encodes (lines 903-936). Only then is the request spawned. `make_request` starts its timer at line 596 after all of this. The cache is a single `LruCache` owned by the main task; there is no parallel prefetch.
- Why it is wrong: Image preprocessing is serialized on one thread while holding a concurrency permit, so the effective concurrency during cache misses is well below `--concurrency`, and the launch loop rather than the server bounds throughput. The preprocessing cost is invisible in every reported metric. Re-encoding every image as JPEG changes the bytes the server sees (PNG inputs become lossy JPEG; the README says images are "encoded directly", README.md:302), so the server's image token count and decode work differ from what the dataset author intended. The `--server-side-download` path sends `size_bytes: 0` and `0x0` dimensions into the image statistics (lines 1248-1254).
- Failure scenario: 1000 distinct 4 MB images, `--concurrency 32`. Each cache miss costs roughly 150 ms of decode and encode on the launch thread; the launch loop can issue at most about 6 requests per second regardless of the server, and the report attributes the resulting low throughput to the model.
- Comparison: GenAI-Perf pre-generates all multimodal payloads before the measurement phase; vLLM benchmark_serving.py preprocesses the dataset up front and passes ready base64 strings.
- Fix: Preprocess the whole dataset (bounded by `--image-cache-size`) before the measurement window, in parallel with `tokio::task::spawn_blocking`, and report preprocessing time as its own block; send original bytes with their real MIME type unless `--reencode-jpeg` is requested; treat images as immutable payloads during the run.
- Effort: M
- Blocks open-source release: yes (for the VLM binary)

---

### [HIGH] A-18 metrumbench-imagegen measures latency with the wall clock (`chrono::Utc::now`) and truncates to whole milliseconds

- Axis: A1, A16
- Status: VERIFIED
- Location: src/bin/metrumbench-imagegen.rs:583; src/bin/metrumbench-imagegen.rs:616-617; src/bin/metrumbench-imagegen.rs:658-659
- What the code does: `started_at = Utc::now()` (line 583) and `completed_at = Utc::now()` (line 616); the logical request latency is `(completed_at - started_at).num_milliseconds() as f64` (line 617). A monotonic `attempt_start = Instant::now()` exists for per-attempt latency (line 592) but the headline `latency_ms` and every summary percentile derive from the wall-clock difference.

```rust
                let completed_at = Utc::now();
                let logical_latency_ms = (completed_at - started_at).num_milliseconds() as f64;
```

- Why it is wrong: `SystemTime`-derived intervals are affected by NTP slews and steps during the run; the tool itself enforces NTP synchronization at startup (in the other binaries) precisely because the clock is expected to be adjusted. Truncation to integer milliseconds loses sub-millisecond resolution that the other binaries preserve via `Duration`. The brief flags any wall-clock interval measurement as a bug; this is one.
- Failure scenario: chrony steps the clock by 250 ms during a 10-minute run. One request's latency is off by 250 ms; if the step is negative and the request was short, `num_milliseconds()` is negative and enters the percentile sort as a negative latency.
- Comparison: All comparison tools use `time.perf_counter()` or equivalent monotonic clocks for intervals.
- Fix: Use `Instant::now()` for `started` and `completed`, keep `Utc::now()` only for the recorded ISO timestamps, and compute `latency_ms` as `elapsed.as_secs_f64() * 1000.0`.
- Effort: S
- Blocks open-source release: yes

---

### [HIGH] A-19 The client has no bounded worker pool beyond the semaphore, holds every request handle and every result in memory, and writes nothing until the end, so a crash or Ctrl-C loses the entire run

- Axis: A13, D3, D5
- Status: VERIFIED
- Location: src/bin/metrumbench-llm.rs:1605; src/bin/metrumbench-llm.rs:1650-1664; src/bin/metrumbench-llm.rs:1678; src/bin/metrumbench-llm.rs:1811-1823; src/bin/metrumbench-vlm.rs:1146; src/bin/metrumbench-vlm.rs:1521-1533; src/bin/metrumbench-asr.rs:1347; src/bin/metrumbench-asr.rs:1549-1561; src/bin/metrumbench-imagegen.rs:343
- What the code does: Each binary pushes every `JoinHandle` into `handles` (one per request, `Vec<JoinHandle<...>>`), then awaits them sequentially in launch order. Every completed request's `Duration`s, token counts, word counts and full error strings are kept in `Vec`s until the end. The single summary JSONL line is written only after `print_stats` (line 1811). There is no signal handler (`tokio::signal` is not referenced anywhere in `src/`). metrumbench-imagegen is the exception for data records: it writes one JSONL line per request through a `Mutex<File>` as they complete (line 343), but its summary is also end-of-run only.
- Why it is wrong: The `handles` vector grows with `num_requests`, not `concurrency`, and each handle owns the request's result until drained, so memory grows linearly with the run. At 100k requests with error strings that include the full request payload and response body (lines 855-858 embed `error_body`; the VLM version embeds `headers`), memory use is hundreds of megabytes. A `SIGINT` at minute 45 of a 60-minute run terminates the process with no output file; the exit path that writes the data log is never reached. The `--stop-after-seconds` flag only stops launching; it does not write partial results early.
- Failure scenario: `--num-requests 100000 --concurrency 64`. Operator presses Ctrl-C after 99,000 completions because the server started returning 503s. Data log: not created. Debug log: 99,000 request logs at warn level. Nothing to analyze.
- Comparison: vLLM benchmark_serving.py writes results at the end too but is typically run for minutes; GenAI-Perf writes profile export incrementally; guidellm checkpoints progress and handles `KeyboardInterrupt` by writing a partial report; MLPerf LoadGen logs every query to disk as it completes.
- Fix: Use a bounded `mpsc` channel from request tasks to a single recorder task that appends one JSONL line per request as it completes (per-request records are also the raw material for correct percentiles and window definitions); install a `tokio::signal::ctrl_c` handler that stops issuing, drains in-flight requests with a deadline, and writes the summary with a `partial: true` marker.
- Effort: M
- Blocks open-source release: yes

---

### [HIGH] A-20 NTP check is a mandatory gate on benchmark start, contacts four public NTP pools, blocks the async runtime, and is applied even to pure file-transform utilities

- Axis: A15, F5
- Status: VERIFIED
- Location: src/lib.rs:36-122; src/bin/metrumbench-llm.rs:1516; src/bin/metrumbench-vlm.rs:1081; src/bin/metrumbench-asr.rs:1192; src/bin/add_column_to_jsonl.rs:58; src/bin/extract_prompts.rs:94; src/bin/jsonl_to_csv.rs:114; src/bin/launch_container_with_yaml_config.rs:149; src/bin/launch_subprocess_with_yaml_config.rs:125; src/bin/wait_for_vllm.rs:33
- What the code does: `check_ntp_sync` sends NTP requests to `pool.ntp.org`, `time.google.com`, `time.cloudflare.com`, `time.apple.com` (plus `METRUM_NTP_SERVER`), retrying for up to `METRUM_NTP_TIMEOUT` seconds (default 30), and returns `Err` if all fail or if the offset exceeds 1 s. It uses the blocking `ntp` crate inside `#[tokio::main]` async mains. All interval timing in the benchmarks uses `Instant`, so the wall-clock offset does not affect any reported latency or throughput. The measured offset is logged at debug level and then discarded; it is not written to the output record. The only hard consumer of wall-clock correctness is `license::check_license`, which runs immediately after (line 1519). Even `add_column_to_jsonl`, a pure file transform, refuses to run without NTP reachability.
- Why it is wrong: A benchmark that cannot start in an air-gapped lab, a GPU cluster with egress blocked, or a CI runner without UDP 123 egress is unusable in exactly the environments where serious benchmarking happens. The 30-second retry adds up to 30 s of startup delay on every failure. The insights-cli agent already sets `METRUM_SKIP_NTP_CHECK=true` and `METRUM_SKIP_LICENSE_CHECK=true` by default for local control planes (insights-cli/insights_cli/agent/run_lifecycle.py:391-393), which confirms that the platform's own operators treat the gate as an obstacle. The information the check gathers (host clock offset from a reference) is genuinely useful when benchmark timestamps must be correlated with server-side metrics or with other hosts, but the code throws it away and uses it only as a gate.
- Failure scenario: Benchmark host in a DMZ with UDP egress blocked: every binary exits after 30 s with "Could not verify system clock synchronization".
- Comparison: No comparison tool contacts NTP or gates on it. MLPerf LoadGen records host clock information in its log without blocking.
- Fix (per the maintainers' direction that the time check should remain available as an option): keep `timecheck` as an opt-in `--ntp-check` flag (default off) on the four benchmark binaries only; when enabled, record `clock_offset_ms`, `ntp_server`, and `ntp_checked_at` in the output `environment` block instead of discarding them; make a failure to reach NTP a warning plus `clock_offset_ms: null`, with a separate `--require-ntp` flag for users who want the hard gate; run the check with `tokio::task::spawn_blocking` or an async NTP client so it does not block the runtime; remove the call from the six utility binaries. Delete the `METRUM_SKIP_NTP_CHECK` environment variable once the default is off.
- Effort: S
- Blocks open-source release: yes (in its current mandatory form)

### [HIGH] B-01 No dispersion is reported for any throughput figure, no standard deviation for any latency, no confidence interval anywhere, and no repeat-run support

- Axis: B1, B2, B5
- Status: VERIFIED
- Location: src/bin/metrumbench-llm.rs:1279-1417 (entire summary record); src/bin/metrumbench-llm.rs:416-448 (calc_stats returns min, max, avg, p50, p90, p95, p99 only); src/bin/metrumbench-vlm.rs:724-834; src/bin/metrumbench-asr.rs:1041-1155; src/bin/metrumbench-imagegen.rs:1050-1066
- What the code does: Every latency distribution is summarized as min, max, mean and four percentiles. No implementation computes a standard deviation, a median absolute deviation, or a bootstrap interval. Throughput values (`requests_per_second`, `tokens.*.per_second`, `words.*.per_second`, `images_per_second`, `audio_per_second`) are single scalars with no variance, because the tool computes one number from totals over the whole window rather than from per-interval samples. There is no `--repeat` or `--runs` flag; each invocation appends exactly one summary line to the data log and there is no code that reads previous lines back to aggregate across runs.
- Why it is wrong: A published throughput number without a dispersion estimate cannot be compared to another. Run-to-run variance on GPU inference servers is commonly 3 to 10 percent from thermal state, KV cache occupancy and scheduler nondeterminism; a single-run figure presented to three decimal places (the tool prints `Requests/Second: 7.92` and stores `7.922655240627804`) implies a precision the measurement does not have.
- Failure scenario: Vendor A reports 3120.5 tokens/s from one run; vendor B reports 3050.2 from one run of a competing product. Neither can say whether the 2.3 percent gap is real.
- Comparison: vLLM benchmark_serving.py reports std for TTFT, TPOT and ITL; GenAI-Perf reports avg, min, max, p99, p90, p75, p50, p25 and std, and supports `--measurement-interval` with multiple passes to a stability threshold; MLPerf requires repeated runs and reports the minimum acceptable across runs for some benchmarks.
- Fix: Add std and MAD to the shared stats module; compute throughput over fixed sub-windows (for example 10 s bins) inside the measurement window and report mean and std across bins; add `--runs N` that executes the workload N times and reports per-run values plus mean, std and a 95 percent bootstrap interval; write per-request records so external tools can compute intervals.
- Effort: M
- Blocks open-source release: no, but it will be the first criticism from anyone with a statistics background

---

### [MEDIUM] B-02 Outliers are neither detected nor disclosed; the summary silently includes connection-setup requests and client-side stalls

- Axis: B3
- Status: VERIFIED
- Location: src/bin/metrumbench-llm.rs:230-239 (record_success pushes every value unconditionally); src/bin/metrumbench-llm.rs:1566-1572 (no warmup); src/bin/metrumbench-llm.rs:1678 (drain order)
- What the code does: Every successful sample is pushed into the distributions. There is no trimming, no winsorization, no flagging of samples beyond a threshold, and no per-request record from which a reader could identify outliers after the fact.
- Why it is wrong: This is acceptable only if the raw records are available. They are not (see A-19). The summary is therefore both the only artifact and an unqualified one. Because there is no warmup, the first `concurrency` requests, which include TLS handshake and any server cold start (CUDA graph capture, first-batch compilation), sit inside min, avg and the percentiles.
- Failure scenario: A first request that triggers a 12 s CUDA graph capture on the server becomes the max and the p99 of a 100-request run, and there is no field indicating that it was request number 1.
- Comparison: GenAI-Perf excludes warmup requests; MLPerf discards the first N samples per its rules; vLLM benchmark_serving.py publishes per-request records in the result JSON so the reader can filter.
- Fix: Emit per-request records with a sequence number and phase tag; add warmup; do not trim by default but report the count of samples above 3 MAD as `outlier_count` so the reader is alerted.
- Effort: S
- Blocks open-source release: no

---

### [MEDIUM] B-03 Every headline number is presented without a sample count next to it; the JSONL `p99` at n = 16 in this assessment's own run is the sample maximum

- Axis: B4, B5
- Status: VERIFIED (Appendix 8.3: p99 505 ms equals max 505 ms for n = 16)
- Location: src/bin/metrumbench-llm.rs:493-510 (console print); src/bin/metrumbench-llm.rs:1330-1356 (JSONL fields carry no `n`)
- What the code does: The console prints percentiles without the sample count on the same line; the JSONL `response_times`, `ttft` and `tpot` objects contain seven numbers and no count. `successful_requests` exists elsewhere in `timing`, but `tpot` has its own (smaller) sample size because requests with `completion_tokens == 0` or zero decode time are excluded (lines 1733-1743) and that count is not reported anywhere.
- Why it is wrong: A reader of `tpot.p99_s` cannot know how many samples produced it. In the reviewer's non-streaming run the `tpot` block reports all zeros because the sample set was empty (Appendix 8.3), which is indistinguishable in the file from a measured zero.
- Failure scenario: A run with 3 successful streaming requests and 97 non-streaming or empty completions reports `tpot.p99_s` from 3 samples with no indication.
- Comparison: GenAI-Perf prints the request count in the table header; vLLM prints "Successful requests" directly above the latency table and uses NaN rather than 0 for undefined statistics.
- Fix: Add `n` to every distribution object; serialize undefined statistics as `null`, never `0`.
- Effort: S
- Blocks open-source release: yes (the zero-for-undefined convention produces false published numbers)

---

### [HIGH] B-04 Every single-number-without-dispersion output field, enumerated

- Axis: B5
- Status: VERIFIED
- Location: src/bin/metrumbench-llm.rs:1279-1417; src/bin/metrumbench-vlm.rs:724-834; src/bin/metrumbench-asr.rs:1041-1155; src/bin/metrumbench-imagegen.rs:1019-1047
- What the code does: The following fields are emitted as a single scalar with no variance, interval, or sample count in the same object:
  - metrumbench-llm `metrics.words.prompt.per_second`, `metrics.words.completion.per_second`, `metrics.tokens.prompt.per_second`, `metrics.tokens.completion.per_second`, `metrics.tokens.total.per_second`, `metrics.timing.requests_per_second`, `metrics.timing.steady_state_requests_per_second`, `metrics.errors.rate`, and every `avg_ms`/`avg_s`/`avg` field (rt, ttft, tpot, tokens x3, words x2), plus `per_endpoint.*.requests_per_second`, `per_endpoint.*.tokens.prompt_per_second`, `per_endpoint.*.tokens.completion_per_second`, `per_endpoint.*.ttft.avg_ms`, `per_endpoint.*.tpot.avg_s`.
  - metrumbench-vlm the same set plus `metrics.images.images_per_request.avg`; `per_endpoint.*.requests_per_second` is the only per-endpoint rate.
  - metrumbench-asr `throughput.audio_per_second`, `throughput.characters_per_second`, `throughput.words_per_second`, `throughput.requests_per_second`, `request_rate.requests_per_second`, `request_rate.bytes_per_second`, `rtf.avg`, `accuracy.wer.avg`, `accuracy.cer.avg`.
  - metrumbench-imagegen `images_per_second`, `requests_per_second`, `success_rate`, `latency_ms.mean`, `image_bytes.mean`, `endpoints[].images_per_second`, `endpoints[].requests_per_second`.
- Why it is wrong: See B-01. The list is provided so the fix can be verified field by field.
- Fix: See B-01.
- Effort: included in B-01
- Blocks open-source release: no

---

### [CRITICAL] C-01 Duplication inventory: the four benchmark binaries carry four copies of the load loop, metrics store, statistics, request path and output writer, and the copies have drifted

- Axis: C1
- Status: VERIFIED
- Location: enumerated below
- What the code does: `src/lib.rs` (322 lines) contains only banner, NTP, license, compile-time info and ID generation. `src/endpoints.rs` (222 lines) and `src/prompt_inputs.rs` (390 lines) are the only shared logic. The four benchmark binaries total 6460 lines (llm 2077, vlm 1541, asr 1585, imagegen 1257). The table below lists each duplicated block with its line range per binary and the drift observed between copies. "imagegen" is listed where a structurally equivalent block exists; it was written independently and shares no text with the other three.

| Block | metrumbench-llm.rs | metrumbench-vlm.rs | metrumbench-asr.rs | metrumbench-imagegen.rs | Drift between copies |
|---|---|---|---|---|---|
| CLI `Args` struct (scenario, url, endpoints_file, num_requests, concurrency, prompts/input, log_level, model, data_log, debug_log, error_log, request_timeout, connect_timeout, pool_idle_timeout, tcp_keepalive, api_key, stop_after_seconds, ramp_up_seconds) | 45-138 | 43-155 | 25-116 | 53-184 | `request_timeout` default 300 (llm:109) vs 120 (vlm:101, asr:72); `log_level` default warn (llm, vlm) vs info (asr); ASR makes scenario/num_requests/model/input `Option` and validates by hand (asr:1171-1179) while others use clap required; imagegen has no `--stop-after-seconds`, no `--ramp-up-seconds`, no `--streaming`, no `--log-level`; only llm has `--restic-tag` |
| `EndpointMetrics` struct | 140-153 | 157-169 | 139-156 | absent (per-outcome vector instead) | llm has `error_types` map; vlm/asr do not; asr tracks bytes and words instead of tokens |
| `Metrics` struct | 155-192 | 171-186 | 158-182 | 265-268 | llm has `metrics_start_time`, `error_timestamps`, `error_types`; vlm has neither; asr has `request_start_times`/`request_end_times` that are pushed (asr:1409, 1483) but never read |
| `Metrics::new` | 194-215 | 188-206 | 184-211 | derive Default | field-set differences as above |
| `record_success` | 217-254 | 208-250 | 213-275 | outcomes.push (358) | tpot is `Option<f64>` (llm) vs `Option<Duration>` (vlm); asr signature has 13 parameters |
| `record_error` | 256-277 | 252-259 | 277-286 | none | llm parses error type from the message and records timestamps; vlm/asr store the raw string only |
| `calc_percentile` (Duration) | 280-290 | 261-271 | 288-298 | 1077-1080 (f64) | Estimator differs: round((n-1)p) in llm/imagegen vs ceil(np)-1 in vlm/asr (see A-04) |
| f64 percentile | 466-487 (closure), 1155-1162 (closure), 1164-1173 (u64 closure) | none (tpot is Duration) | 300-307 | 1068-1080 | three implementations inside llm alone |
| `calc_stats_usize` / `calc_stats_u64` closures | 293-307, 450-464 | 385-399 | none | none | median as `sorted[len/2]` (upper median for even n) with no percentile call |
| `print_compact_block` | 310-371 | 273-320 | 309-337 | none | llm prints TPOT and per-type token rates; vlm prints Token Rate only; asr prints chars/words |
| `print_stats` | 374-698 | 322-584 | 339-607 | none (prints JSON) | llm uses metrics window; vlm/asr use `start_time.elapsed()` always; vlm "Average Request Latency" is `total_time / requests` (vlm:509), which is not a latency, while llm computes the mean of response times (llm:668-676) |
| `make_request` | 763-1129 | 587-682 | 722-834 | 741-884 | llm: streaming SSE parser; vlm: no streaming, fabricated TTFT; asr: multipart; each has its own error classification strings |
| `create_log_record` | 1131-1418 | 684-835 | 997-1155 | 951-1048 (`build_summary`) | Field sets, denominators and error-rate definitions differ (see A-08); `unique_id`, `human_readable_id`, `timestamp`, `compile_info` header block is identical text in llm/vlm/asr and absent in imagegen |
| `build_request_body` | 1434-1473 | 993-1049 | inline in make_request 757-774 | 752-786 | llm sends `stream`, `stream_options`; vlm never streams; only imagegen supports extra body |
| logger setup (`CombinedLogger::init` with WriteLogger x2 + TermLogger) | 1522-1559 | 1086-1124 | 1197-1252 | 410 (`File::create(debug_log)` only, never written to) | asr creates parent directories (asr:1207-1222); llm/vlm do not; imagegen creates an empty debug.log and never logs to it |
| reqwest `Client::builder()` block | 1566-1572 | 1131-1137 | 1260-1266 | 287-291 | imagegen omits `.timeout()` and `pool_max_idle_per_host` |
| ramp-up permit scheduler | 1580-1601 (semaphore permits added by a spawned task) | 1167-1188 and 1405-1411 (sleep between launches computed from `current_concurrency`, semaphore fixed at full concurrency) | none | none | Two different ramp-up algorithms under the same flag name; the vlm version does not limit concurrency at all, it only paces launches |
| launch loop (`'request_loop`, stop_after check, endpoint pick, permit, spawn) | 1612-1671 | 1155-1412 | 1359-1439 | 320-360 | vlm picks prompts randomly, llm/asr cycle; asr picks endpoint before acquiring the permit |
| drain loop with ramp-up gating | 1678-1802 | 1417-1512 | 1444-1540 | 361-363 | error recording during ramp-up: llm suppresses (1796), vlm records (1508), asr has no ramp-up |
| data log append + exit-code-on-errors | 1811-1833 | 1521-1540 | 1549-1584 | 374-376 | identical text in llm/vlm/asr |
| `count_words` | 1917-1919 | none | inline `split_whitespace().count()` 1454 | none | |
| `log_detailed_error` / `log_reqwest_error_details` | 1838-1915 | inline 1487-1507 | inline 1519-1522 | none | 78 lines of error-chain logging exist only in llm |
| `substitute_env_vars` | n/a | n/a | n/a | n/a | duplicated between launch_container_with_yaml_config.rs:44-59 (missing var warns and substitutes empty string) and launch_subprocess_with_yaml_config.rs:41-54 (missing var is an error): same name, opposite behavior |
| log-level string to `LevelFilter` match | 1522-1529 | 1087-1094 | 1198-1205 | none | fallback Warn (llm, vlm) vs Info (asr); also in launch_*_with_yaml_config.rs:130-137 and 106-113 and iso_customizer.rs:122-129 (seven copies total) |

- Why it is wrong: By line count, roughly 2100 of the 5203 lines in llm, vlm and asr are structural duplicates of each other (Args, Metrics, stats, print_stats, create_log_record, logger, client, loops, output). The drift column shows that at least eight of those blocks already compute or report something different under the same name. Every finding in section A that touches TTFT, TPOT, percentiles or denominators has to be fixed in three or four places, and the history (CHANGELOG v0.1.78 fixed ramp-up zero handling in llm only; the vlm ramp-up validation at vlm:1072-1078 still treats 0 as a ramp-up) shows fixes are not propagated.
- Failure scenario: A future fix to the percentile estimator lands in metrumbench-llm; metrumbench-asr keeps `ceil(np)-1`; a customer comparing LLM p50 to ASR p50 latency is comparing two different order statistics.
- Comparison: guidellm has one `Benchmarker` and pluggable `Backend`/`RequestLoader`; GenAI-Perf has one profile-data parser with per-endpoint-type converters; vLLM benchmark_serving.py has one `benchmark()` loop with per-backend `request_func`s.
- Fix: Section 4.
- Effort: L
- Blocks open-source release: yes (because the drift is producing incorrect numbers today, not because duplication is unattractive)

---

### [MEDIUM] C-02 No library API: all measurement logic lives in `fn main` binaries and cannot be embedded, tested in-process or reused

- Axis: C4
- Status: VERIFIED
- Location: Cargo.toml:4 (`autobins = false`, 11 `[[bin]]` entries, no `[lib]` config beyond the implicit src/lib.rs); src/lib.rs:1-2 (`pub mod endpoints; pub mod prompt_inputs;`)
- What the code does: The public library surface is `endpoints`, `prompt_inputs`, `banner`, `timecheck`, `compile_time_info`, `unique_id`, `license`. None of `Metrics`, `make_request`, `build_request_body`, `create_log_record` or the SSE parser is in the library; all are private items in `src/bin/*.rs`. The insights-cli agent consumes the tool by spawning the binary and parsing its JSONL output (insights-cli/insights_cli/agent/job_lifecycle.py:538-598), which is the only integration path available.
- Why it is wrong: A benchmarking tool that can only be driven as a subprocess cannot be unit tested at the function level (see E-01), cannot be embedded in a CI harness or a notebook, and cannot expose a stable `Result` type for consumers; the JSONL schema is the de facto API and it has already been declared deprecated once (CHANGELOG.md:34-37) without a version field in the record.
- Fix: Section 4: `metrumbench-core` crate with `pub struct BenchmarkConfig`, `pub async fn run(config) -> Result<Report>`, `pub struct Report` with `serde` and a `schema_version` field; thin binaries.
- Effort: L (part of the refactor)
- Blocks open-source release: no

---

### [MEDIUM] C-03 CLI surface is inconsistent across the four binaries and does not match the vocabulary GenAI-Perf and vLLM users expect

- Axis: C5
- Status: VERIFIED
- Location: Args structs cited in C-01; metrumbench-imagegen.rs:62 (`--url` is a base URL "usually ending in /v1") versus metrumbench-llm.rs:54-58 (`--url` is the full completions URL); metrumbench-imagegen.rs:68 (`--endpoint` repeatable) has no counterpart; metrumbench-asr.rs:47 uses `--input` where the others use `--prompts`; metrumbench-imagegen.rs:71 accepts JSON or JSONL endpoint files while endpoints.rs:63 accepts YAML only; metrumbench-asr.rs:104 `--response-format` default is written `verbose-json` while the enum renders `verbose_json`
- What the code does: Four independent clap structs with the divergences listed. The same conceptual flag has different defaults (`--request-timeout` 300 vs 120), different requiredness (`--scenario` required in llm/vlm/imagegen, optional-then-validated in asr), and different semantics (`--url`). There is no `--request-rate`, `--num-prompts`, `--dataset`, `--tokenizer`, `--seed`, `--warmup`, `--output-len`, `--input-len`, `--goodput`, `--percentile-metrics`, `--save-result`, or `--profile` equivalent. Output files default to `debug.log` and `error.log` in the current directory (llm:103-107), so two concurrent invocations in the same directory clobber each other's logs.
- Why it is wrong: A user arriving from vLLM's `benchmark_serving.py --request-rate 10 --num-prompts 1000 --dataset-name random --random-input-len 1024 --random-output-len 128 --seed 0` has no mapping onto this CLI. Four binaries with four Args structs guarantee continued divergence.
- Fix: One binary `metrumbench` with subcommands `llm`, `vlm`, `asr`, `imagegen` sharing a `#[command(flatten)] CommonArgs` struct (endpoint, load, output, logging, seed, warmup), per-modality flags in a second flattened struct; adopt the vLLM/GenAI-Perf names where a concept matches (`--request-rate`, `--num-prompts`, `--seed`, `--tokenizer`, `--warmup-requests`, `--result-dir`); keep the old binary names as deprecated shims for one release.
- Effort: M
- Blocks open-source release: no, but it should be done before the first tagged release because renaming flags later is a breaking change

---

### [LOW] C-04 The six utility binaries and iso_customizer do not belong in a benchmarking crate

- Axis: C2, F5
- Status: VERIFIED
- Location: Cargo.toml:22-53; src/bin/iso_customizer.rs (1253 lines, invokes `sudo mount`, `rsync`, `xorriso`); src/bin/launch_container_with_yaml_config.rs (269 lines, shells out to `docker run`); src/bin/launch_subprocess_with_yaml_config.rs; src/bin/add_column_to_jsonl.rs, extract_prompts.rs, jsonl_to_csv.rs (polars)
- What the code does: 2067 of the crate's 9804 Rust lines implement an Ubuntu ISO customizer, two YAML-driven process launchers, a vLLM health poller and three polars-based JSONL utilities. `iso_customizer` embeds a cloud-init template that installs Docker, NVIDIA drivers from a third-party GitHub repository (`Scotchman0/NVIDIA_Drivers`, iso_customizer.rs:493), pyenv, poetry, Rust, Go and the AWS CLI, and runs `sudo mount -o loop` (iso_customizer.rs:355-364). `iso_customizer` also declares its own `VERSION = "0.2.2"` (line 15) independent of the crate version.
- Why it is wrong: These tools drag in polars (168 of 466 dependency packages, see D-08), make the crate's security surface include privileged mount operations, and dilute the identity of the repository. A reviewer evaluating "an LLM benchmark" will not expect `sudo mount` in the source tree. The three polars utilities are also replaceable by `jq` one-liners.
- Fix: Move `iso_customizer`, `launch_*_with_yaml_config`, `nvidia_stack`, and the release scripts into a separate internal repository; drop the polars utilities or rewrite `jsonl_to_csv` with `serde_json` + `csv` (both already dependencies) and remove polars.
- Effort: S
- Blocks open-source release: no, but polars removal is required to fix the build (A-06) without pinning ethnum

### [HIGH] D-01 Panic paths in long runs: `Duration / u32` division by zero is guarded but `Duration::from_secs_f64` on non-finite input, `sorted[len / 2]` on empty input, and `unwrap()` on user-controlled `Option`s are not

- Axis: D1
- Status: VERIFIED (code read); SUSPECTED for runtime occurrence of each
- Location: src/bin/metrumbench-asr.rs:1453 (`Duration::from_secs_f64(inference_time)`); src/bin/metrumbench-asr.rs:1456-1457 (division by `inference_time` which may be 0.0); src/bin/metrumbench-llm.rs:305 (`sorted[len / 2]` inside a non-empty guard, safe) versus src/bin/metrumbench-llm.rs:1762 (`(completed * 100) / (args.num_requests as usize)`, safe because clap enforces >= 1); src/bin/metrumbench-asr.rs:1048-1056 (`args.scenario.as_ref().unwrap()` and three siblings, guarded by the manual check at 1171-1179); src/bin/metrumbench-vlm.rs:1205, 1306 (`records.first().expect("non-empty records")`, guarded by loader); src/bin/metrumbench-imagegen.rs:590 (`runtime.get(&ep.name).expect(...)`: panics if two endpoints share a hostname because `HashMap` keys collapse while `endpoints` keeps both); src/bin/metrumbench-imagegen.rs:709 (`min_by_key(...).unwrap()` on a non-empty list, safe); src/bin/metrumbench-imagegen.rs:1064 (`sorted[sorted.len() - 1]` inside non-empty guard, safe); src/bin/metrumbench-vlm.rs:300, 311, 369 (`sum::<Duration>() / len as u32`: `Duration` division panics on zero divisor; guarded by `is_empty` checks or `.max(1)`); src/bin/jsonl_to_csv.rs:95 (`column_data.get(col).unwrap()`, safe by construction)
- What the code does: In metrumbench-asr, when the server returns `inference_time: 0` or the elapsed time rounds to zero (text format, line 821, on a very fast server), `words_per_second = word_count / 0.0` is `inf` or `NaN`, which then flows into `sum` and percentile sorts with `partial_cmp(...).unwrap_or(Equal)`, silently corrupting the ASR throughput block. `Duration::from_secs_f64` panics if given a negative or non-finite value; line 814-818 filters negatives and non-finite for the server-supplied path only, so the client-measured path cannot be negative and this is safe. In metrumbench-imagegen, `EndpointRuntime` is keyed by `ep.name` (line 283) and names default to the hostname (line 479-488); two endpoints on the same host with different ports both yield the host:port string so this is safe for `--endpoint` lists but two records in an endpoints file with the same explicit `name` produce one runtime entry and the second endpoint's `runtime.get(...).expect(...)` still succeeds (same key); the panic requires a name present in `endpoints` but absent in `runtime`, which cannot happen given both are built from the same list. The reviewer downgrades that item to no-panic.
- Why it is wrong: The tool is used for 60-minute runs. Any panic in the drain loop discards all results (see A-19) because nothing is persisted until the end. The reviewer found no unguarded panic in the LLM drain path; the risk concentrates in ASR's floating-point path (NaN propagation is silent, not a panic) and in the general absence of per-request persistence, which turns any panic anywhere into total loss.
- Failure scenario: metrumbench-asr with `--response-format text` against a stub server that answers in under a microsecond: `inference_time` is 0.0 for some requests; `words_per_second` is `inf`; `Avg Words/Second` prints `inf`; the JSONL `throughput.words_per_second` is finite only because it is recomputed from totals (line 1123), so the console and JSONL disagree.
- Comparison: vLLM benchmark_serving.py guards divisions and reports NaN via numpy; GenAI-Perf validates non-empty distributions before computing.
- Fix: Route all per-request rate computations through a `safe_div` returning `Option<f64>`, serialize `None` as `null`; add `debug_assert!(x.is_finite())` on every recorded f64; persist per-request records incrementally so a panic loses at most one record; add a `std::panic::set_hook` that flushes the recorder.
- Effort: S
- Blocks open-source release: no

---

### [MEDIUM] D-02 Error handling mixes `Box<dyn Error + Send + Sync>`, `anyhow::Error`, `String` and a `(String, Option<u16>, String)` tuple; error type classification is done by string prefix

- Axis: D2, A12
- Status: VERIFIED
- Location: src/bin/metrumbench-llm.rs:771 (return type `Box<dyn Error + Send + Sync>`); src/bin/metrumbench-llm.rs:820-822, 855-858 (`anyhow::anyhow!(...).context(...).into()` so the context chain is flattened into a boxed error whose `Display` is only the outermost context); src/bin/metrumbench-llm.rs:257-262 (`record_error` derives `error_type` as the text before the first colon of `e.to_string()`); src/bin/metrumbench-llm.rs:836-842 and 1056-1062 (HTTP status classified into `rate_limit`, `authentication`, `bad_request`, `server_error`, `unknown_error` and then attached only as a context string, never as a typed field); src/bin/metrumbench-imagegen.rs:750 (tuple error type); src/bin/metrumbench-vlm.rs:1508 (`format!("{:?}", e)` recorded as the error message)
- What the code does: In metrumbench-llm an HTTP 429 becomes `anyhow!("HTTP error: 429 ... - body").context("Request failed with status 429").context("Error type: rate_limit").into()`. `e.to_string()` on the resulting `Box<dyn Error>` yields "Error type: rate_limit" (the outermost context), so `record_error` splits on ':' and files it under type "Error type". Every HTTP error therefore lands in a single bucket named "Error type" in `metrics.errors.types`; transport errors land under "HTTP request failed" or "Streaming request failed". The 429 versus 500 distinction the code computes is lost before it reaches the output.

```rust
        let error_type = error.split(':').next().unwrap_or("unknown").to_string();
```

- Why it is wrong: Users need to distinguish rate limiting from server errors from client timeouts to interpret a run. The tool computes the classification and then throws it away. The VLM binary records `{:?}` of the error, which for a `Box<dyn Error>` wrapping a `String` prints the string with escaped quotes and, for reqwest errors, a multi-line debug struct, so `error_counts` grouping (vlm:477-481) treats each distinct URL or timing as a distinct error type.
- Failure scenario: 100 requests, 50 return 429 and 50 return 503. `metrics.errors.types` = `{"Error type": 100}`.
- Comparison: guidellm records a typed `error` enum per request; GenAI-Perf reports HTTP status counts; vLLM records the error string per request in the output JSON.
- Fix: Define `enum RequestError { Timeout, Connect, HttpStatus(u16), StreamTruncated, ParseError, NoOutputToken, UsageMissing, ... }` with `#[derive(Serialize)]`; record it as a field; keep the human message separately; use `thiserror` for the enum and `anyhow` only at the binary boundary.
- Effort: S
- Blocks open-source release: no

---

### [MEDIUM] D-03 Connection pool and runtime are left at defaults with one exception; `pool_max_idle_per_host` equals concurrency but there is no pre-warm; HTTP/2 is not forced off or on; the debug log grows without bound at info level

- Axis: D3, A13
- Status: VERIFIED
- Location: src/bin/metrumbench-llm.rs:1566-1572; src/bin/metrumbench-llm.rs:1475 (`#[tokio::main]` default multi-thread runtime, worker threads = cores); src/bin/metrumbench-llm.rs:1532-1559 (`WriteLogger` at `log_level` to `debug_log`, `TermLogger` at the same level); src/bin/metrumbench-llm.rs:786-794 (per-request debug logging of the full pretty-printed payload)
- What the code does: The reqwest client sets `timeout`, `connect_timeout`, `pool_max_idle_per_host(concurrency)`, `pool_idle_timeout`, `tcp_keepalive`. It does not set `http1_only()`/`http2_prior_knowledge()`, `tcp_nodelay` (reqwest default is true), or `pool_max_idle_per_host` above concurrency for multi-endpoint runs where several hosts share the pool. The tokio runtime is the default. At `--log-level debug` every request logs its full pretty-printed JSON payload to both the file and the terminal, which for a VLM run with base64 images is megabytes per request (the VLM binary redacts payloads containing "base64", vlm:600-605; the LLM binary does not redact anything).
- Why it is wrong: Nothing here is a measurement error, but the client's own overhead is never measured or reported, so a user cannot tell whether a plateau is the server or the client. The reviewer measured the client ceiling on this 16-core host against a zero-latency dummy server on the same host (Appendix 8.7): about 16,700 streaming requests/s at 8 tokens each, about 6,000 streaming requests/s at 64 tokens each (about 380k tokens/s of SSE parsing), and about 40,000 non-streaming requests/s, with the client consuming roughly 4 to 5 cores. These are far above any single inference server's rate, so the client is not the bottleneck for LLM serving, but the numbers are host-specific and the tool gives users no way to obtain them.
- Fix: Add a `--self-test` mode that runs the load loop against an in-process null server and reports the client's own ceiling; log the runtime configuration (worker threads, pool size) into the output `environment` block; redact request bodies at debug level in all binaries; log per-request lines only at trace.
- Effort: S
- Blocks open-source release: no

---

### [HIGH] D-04 Streaming parser robustness: no test exists for any malformed input, and the parser fails four of the four adversarial cases the reviewer constructed

- Axis: D4, E1
- Status: VERIFIED (Appendix 8.5)
- Location: src/bin/metrumbench-llm.rs:877-1017 (parser); src/bin/metrumbench-llm.rs:1964-2077 (tests: nine tests of `stream_choice_*` helpers on single JSON values, none exercising the byte-stream loop)
- What the code does: The parser is an inline `while !done` loop over `bytes_stream()` with the defects described in A-02 and A-14. The unit tests cover `stream_choice_output_text` and `stream_choice_has_output_token` on hand-built `serde_json::Value`s. There is no test that feeds bytes through the loop. Multi-byte UTF-8 split across a read is handled by `String::from_utf8_lossy` producing U+FFFD replacement characters in `completion_text`, which changes the word count for CJK output and is not detected.
- Reproduction results (Appendix 8.5): "split" (one event split across two chunk writes) succeeds with 2 of 4 content deltas lost and TTFT attributed to the third delta (201 ms instead of about 100 ms); "roleonly" succeeds with TTFT 0 ns; "nodone" (finish chunk plus usage, then connection close) fails with "Unexpected stream termination"; "multitok" (5 words per chunk, usage says 20 tokens) succeeds and reports TPOT 20 ms per token from usage while the true inter-chunk interval is 100 ms, illustrating that no ITL is measured.
- Why it is wrong: An SSE parser is the measurement instrument for TTFT and ITL. An untested instrument that is wrong on the first four inputs a reviewer tries is not fit for publishing numbers.
- Fix: Extract `fn parse_sse_bytes(&mut self, chunk: &[u8]) -> Vec<SseEvent>` as a pure function in the core crate; test it with the adversarial corpus from Appendix 8.5 plus: CRLF line endings, `event:` and `id:` fields, comment lines starting with `:`, a `data:` line with no space, a 64 KB single event, a 4-byte UTF-8 character split at every byte offset, `[DONE]` followed by trailing data, and a chunk containing two complete events.
- Effort: S
- Blocks open-source release: yes

---

### [HIGH] D-05 Ctrl-C loses the entire run; there is no signal handling in any binary

- Axis: D5
- Status: VERIFIED (Appendix 8.8: SIGINT after 6 s of a 400-request run; exit 130; no data log written)
- Location: `tokio::signal` is not referenced anywhere under src/; src/bin/metrumbench-llm.rs:1811-1823 (data log written only at end of main)
- What the code does: See A-19. The reviewer ran `metrumbench-llm --num-requests 400 --concurrency 2` against the dummy server, sent SIGINT after 6 s, and observed exit code 130 with no `ctrlc.jsonl` created and zero "Request completed" lines in the debug log at info level (per-request completion is logged at debug level only, so the info-level log had nothing either).
- Why it is wrong: Operators stop runs when they see errors. The current design punishes stopping with total data loss.
- Fix: See A-19.
- Effort: included in A-19
- Blocks open-source release: yes

---

### [MEDIUM] D-06 Concurrency correctness: metrics are single-threaded by construction, which is safe but forces the drain loop to consume results in launch order and delays every timestamp-dependent decision

- Axis: D6
- Status: VERIFIED
- Location: src/bin/metrumbench-llm.rs:1678-1802; src/bin/metrumbench-imagegen.rs:358 (`metrics.lock().await.outcomes.push(outcome)` inside each task)
- What the code does: In llm, vlm and asr the `Metrics` struct lives on the main task and is mutated only in the drain loop, so there is no shared mutable state and no contention. The consequence is that all "time-dependent" logic in the drain loop (ramp-up gating, progress logging) runs at drain time, not at completion time, which produces the A-05 defect. In imagegen, each task takes a `tokio::sync::Mutex` on a `Vec` and on the data-log `File`; with `writeln!` on a `std::fs::File` inside an async mutex the write is a blocking syscall on the runtime thread, but at image-generation request rates this is immaterial.
- Why it is wrong: Not a data race; a design that couples measurement bookkeeping to the order of `JoinHandle` awaits. Any per-request timestamp must be captured inside the request task and carried in the result, which the code does for `response_time` and `ttft` but not for `completed_at`.
- Fix: Carry `sent_at: Instant` and `completed_at: Instant` in the result tuple (or record struct); make phase decisions from those, not from the drain loop's clock.
- Effort: S (folded into A-05)
- Blocks open-source release: no

---

### [LOW] D-07 clippy: 65 warning lines (33 unique by lint and location), none indicating a runtime bug; `cargo fmt --check` fails on one function signature

- Axis: D7
- Status: VERIFIED (Appendix 8.2)
- Location: full list in Appendix 8.2; representative: src/bin/metrumbench-llm.rs:1131 and 1434 (too many arguments), src/bin/metrumbench-asr.rs:213 (13 arguments), src/bin/metrumbench-llm.rs:288 (`.clone()` on `Duration`, which is `Copy`), src/prompt_inputs.rs:119 (rustfmt diff)
- What the code does: `cargo clippy --all-targets -- -W clippy::all` exits 0 with warnings in the categories listed in Appendix 8.2 (`clone_on_copy` x9, `unnecessary_cast` x5, `single_component_path_imports` x4, `needless_range_loop` x4, `too_many_arguments` x8, `redundant_closure` x3, `len_zero` x3, `type_complexity` x2, `manual_strip` x2, `for_kv_map` x2, one each of `useless_format`, `manual_map`-style, `unnecessary_to_owned`). `cargo fmt --check` exits 1 with a single diff in `src/prompt_inputs.rs:119`.
- Why it is wrong: None of these is a correctness bug. `too_many_arguments` on `record_success` (13 parameters in asr) and `create_log_record` are symptoms of the missing record struct. A repository that fails `cargo fmt --check` on day one signals the absence of CI.
- Fix: `cargo fmt`; `cargo clippy --fix` for the mechanical ones; introduce `RequestRecord` to eliminate the argument-count warnings; add a CI job that fails on both.
- Effort: S
- Blocks open-source release: no

---

### [LOW] D-08 Dead code, unused state and vestigial files

- Axis: D8
- Status: VERIFIED
- Location and inventory:
  - src/bin/metrumbench-asr.rs:179-180, 1409, 1483: `request_start_times` and `request_end_times` are pushed on every request and never read.
  - src/bin/metrumbench-imagegen.rs:83-84 `--max-endpoint-failures` is parsed, serialized into nothing, and never read (the `failures` counter at 615/648 is maintained and never consulted).
  - src/bin/metrumbench-imagegen.rs:170-171, 410: `--debug-log` creates an empty file and no logger is ever initialized; nothing is written to it.
  - src/bin/metrumbench-llm.rs:136-137 `--restic-tag` is accepted by the benchmark binary solely so that the Python wrapper (metrumbench-llm-wrapper:45-48) can parse it from the same argv; the Rust code never reads it.
  - src/bin/metrumbench-asr.rs:1570-1577: `cleanup_temp_files` is a nested function that logs one line and returns `Ok(())`; its comment says cleanup was intentionally disabled.
  - src/bin/metrumbench-vlm.rs:952-961: `ImageContentOptions` carries commented-out fields (`// format: String, // quality: u8, // etc.`).
  - src/bin/metrumbench-vlm.rs:844, 976-978: comments say "Add this field" and "We'll need to modify the ImageData struct" for a field that already exists.
  - src/bin/metrumbench-llm.rs:1836: comment "Add helper function for word counting if not already present" above an unrelated function.
  - src/bin/metrumbench-llm.rs:1019-1028: the "Token usage statistics missing in X% of chunks" warning fires on every normal OpenAI-style stream (Appendix 8.3 shows it 16 times for 16 requests) and conveys nothing.
  - src/bin/iso_customizer.rs:15: `const VERSION: &str = "0.2.2"` shadows the crate version.
  - main.rs.txt (0 bytes), debug.log (0 bytes), error.log (0 bytes) are present in the crate root; main.rs.txt is tracked in git.
  - RustyPhalanx.docx (18 KB, tracked) is an older copy of the README that references an S3 bucket (`s3://metrum-shared-temp/`) as the install source.
  - README-METRUMBENCH-ASR.md documents flags (`--audio-file`, `--output-format`, `--metrics wer,cer,bleu,rouge`, `--duration`, `--config`) and providers (Google, Amazon, Azure, Deepgram, AssemblyAI) that do not exist in metrumbench-asr.rs; none of those flags is defined in the Args struct at asr:25-116.
  - mcpserver/: pyproject.toml declares a `fastmcp` dependency; `mcpserver/__init__.py` is 0 bytes and README.md is 0 bytes. There is no server. RUSTYPHALANX_MCP_INTERFACE.md (symlinked as mcpserver/MCPINTERFACE.md) is a design note with a Flask pseudocode server that shells out to argv built from untrusted parameters (`args.extend(["--" + key, str(value)])`, line 365).
  - prompt-tools/prompt-tools/main.py prints "Hello from prompt-tools!" and nothing else.
  - The `TODO`/`FIXME` grep over src/ and tests/ returns zero hits; the vestigial state above is undocumented rather than flagged.
- Why it is wrong: Each item is small; together they tell a reviewer the tree has not been curated for an audience.
- Fix: Delete the items listed; either implement mcpserver or remove the directory; regenerate README-METRUMBENCH-ASR.md from the actual Args struct.
- Effort: S
- Blocks open-source release: no (the ASR README and mcpserver stub are HIGH for H-02 and F-05 respectively and are counted there)

---

### [CRITICAL] E-01 Test coverage: 49 tests, none of which exercises a metric computation, the SSE byte parser, the load loop, or an end-to-end run

- Axis: E1
- Status: VERIFIED (Appendix 8.2 lists all 49 test names)
- Location: tests/cli_validation.rs (9 tests, all assert that a bad flag produces a non-zero exit); src/lib.rs:125-161 (4 tests: two assert the skip env vars work, two assert the license date is July 31, 2026); src/endpoints.rs:126-222 (6 tests on YAML loading); src/prompt_inputs.rs:219-390 (13 tests on JSONL loading, including two that spin up a raw TCP HTTP server); src/bin/metrumbench-llm.rs:1964-2077 (10 tests: ramp-up zero, OOM string matcher x2, `stream_choice_*` x7); src/bin/metrumbench-imagegen.rs:1092-1257 (3 tests: size parsing, two summary tests); src/bin/iso_customizer.rs:1138-1253 (4 tests on grub patching and rsync args)
- What the code does: `cargo llvm-cov` is not installed and could not be installed within this assessment (no network install was attempted because it would modify the reviewer's toolchain; the structural gap is stated instead). Of 9804 lines of Rust, the functions with any test are: `hostname_from_url`, `load_endpoints_file`, `load_metrumbench_llm_prompts`, `load_metrumbench_vlm_records`, `normalize_image_ref`, `is_http_url`, `read_utf8_from_path_or_url`, `effective_ramp_up_seconds`, `error_message_indicates_oom`, `stream_choice_output_text`, `stream_choice_has_output_token`, `parse_size`, `build_summary` (imagegen), `customize_iso`, `mounted_iso_contents_source`, `rsync_extract_args`, `check_ntp_sync` (skip path), `check_license` (skip path and date). Zero tests for: any `calc_percentile`, any `calc_stats*`, `record_success`, `record_error`, `print_stats`, `create_log_record` (llm/vlm/asr), `make_request` (all four), the SSE loop, `build_request_body` (llm/vlm), `word_error_rate`, `character_error_rate`, `download_audio_file`, `load_audio_samples`, `load_ground_truth`, `ImageCache::get_or_load`, `format_image_content`, `resolve_seed`, `choose_endpoint`, the ramp-up scheduler, `count_words`, `parse_token_count`, `classify_stream_error`. By function count in the four benchmark binaries, 3 of roughly 60 functions have tests, and two of those three are in imagegen.
- Why it is wrong: The tests that exist check that the CLI rejects zero and that the license is what it is. Nothing checks that a number is right. The two tests in src/lib.rs that pin the license date will have to be deleted with the license mechanism.
- Fix: Section 5.
- Effort: L
- Blocks open-source release: yes

---

### [HIGH] E-02 Untestable as written: the request path, the metrics, and the output writer are all private free functions and closures inside `main`, reachable only by spawning the binary

- Axis: E3
- Status: VERIFIED
- Location: src/bin/metrumbench-llm.rs:416-487 (three statistics closures defined inside `print_stats`); src/bin/metrumbench-llm.rs:1155-1173 (two percentile closures inside `create_log_record`); src/bin/metrumbench-llm.rs:763-1129 (`make_request` takes a live `reqwest::Client` and a URL, so it can only be tested with a socket server); src/bin/metrumbench-llm.rs:1612-1802 (the load loop is inline in `main` with `Args` captured)
- What the code does: Statistics are closures, not functions, so they cannot be called from a test. The SSE parser is a loop body inside `make_request`, not a function over bytes. The load loop reads `args` directly and spawns tasks with `tokio::spawn`, so it cannot be driven with a fake transport or a fake clock. `create_log_record` takes `&Args`, so building its input in a test requires constructing all 20 fields (imagegen's tests do exactly this at 1104-1148 and 1179-1223, 90 lines of fixture per test).
- Fix: In the core crate: `struct RequestRecord`, `fn summarize(records: &[RequestRecord], window: Window) -> Summary` (pure), `struct SseParser` with `fn feed(&mut self, bytes: &[u8]) -> Vec<Event>` (pure), `trait Transport { async fn send(&self, req) -> Result<ResponseStream> }` with a mock implementation, `struct Clock` injectable via a trait for deterministic tests of the scheduler and window logic.
- Effort: included in the refactor
- Blocks open-source release: no (but it is the precondition for E-01)

### [CRITICAL] F-01 License expiry gate: definition, all call sites, bypass, and removal plan

- Axis: F1
- Status: VERIFIED
- Location (definition): src/lib.rs:281-322 (`pub mod license`: `LICENSE_CONTACT`, `license_expiry_date`, `expired_license_message`, `check_license`); src/lib.rs:4-9 (`env_var_is_truthy`, shared with the NTP skip)
- Location (call sites, ten in total, all verified by reading the body):
  1. src/bin/metrumbench-llm.rs:1519 `metrumbench::license::check_license()?;`
  2. src/bin/metrumbench-vlm.rs:1084
  3. src/bin/metrumbench-asr.rs:1195
  4. src/bin/add_column_to_jsonl.rs:59
  5. src/bin/extract_prompts.rs:95
  6. src/bin/jsonl_to_csv.rs:115
  7. src/bin/launch_container_with_yaml_config.rs:150
  8. src/bin/launch_subprocess_with_yaml_config.rs:126
  9. src/bin/wait_for_vllm.rs:34
  10. src/bin/iso_customizer.rs:1054
  metrumbench-imagegen.rs has no license or NTP call (verified: neither identifier appears in the file).
- Location (tests that pin the mechanism): src/lib.rs:138-160 (`check_license_allows_explicit_skip`, `license_expiry_date_is_extended_through_end_of_july_2026`, `license_message_tracks_configured_expiry_date`)
- Location (documentation that advertises it): README.md:9-11 (badges "license-Proprietary" and "license expires July 31, 2026"), README.md:1754, README.md:1756-1758 ("This software is proprietary and licensed for use until July 31, 2026"), README-METRUMBENCH-ASR.md:123, CHANGELOG.md:5-6, announce.md:9-10 and 33, RELEASE-PLAN.md:54-56
- Location (platform wiring of the bypass): insights-cli/insights_cli/agent/run_lifecycle.py:383-393 (maps `SKIP_LICENSE_CHECK` to `METRUM_SKIP_LICENSE_CHECK` and defaults it to `true` for local control planes)
- Bypass: environment variable `METRUM_SKIP_LICENSE_CHECK` set to `1`, `true`, `yes` or `on` (case-insensitive, trimmed), evaluated by `env_var_is_truthy` at src/lib.rs:4-9 and consumed at src/lib.rs:301. The reviewer used this variable, not a source edit, for every run in this assessment.
- What the code does: On every start of ten of eleven binaries, compares `Utc::now().naive_utc()` to 2026-07-31 23:59:59 and exits with the message "License expired. This version is valid until July 31, 2026. Contact chetan@metrum.ai for renewal." The check was reproduced on 2026-09-15 (Appendix 8.4): exit code 1, no output file.
- Why it is wrong: Incompatible with Apache 2.0 in spirit and in practice; every binary in the release tarball currently on disk (`metrumbench-0.1.77-chetan-dev-linux-x86_64.tar.gz`) is dead. The maintainers have stated (during this assessment) that the mechanism is to be removed for the open-source release, and that extending the date is acceptable only as a stopgap for internal builds. This finding records the removal plan.
- Removal plan:
  1. Delete `pub mod license` (src/lib.rs:281-322) and the three tests at src/lib.rs:138-160; keep `env_var_is_truthy` only if the NTP opt-in (A-20) still needs it, otherwise delete it too.
  2. Delete the ten call sites listed above and the `use` of `log::error` where it becomes unused.
  3. Delete `LICENSE_CONTACT` and the personal email from the banner (src/lib.rs:24) and from the `Support:` line; replace with the repository URL.
  4. Remove the `METRUM_SKIP_LICENSE_CHECK` mapping from insights-cli run_lifecycle.py:385-386 and 393 in the same change set (platform side), or leave it as a harmless no-op for one release.
  5. Rewrite README.md:9-11, 1750-1758, README-METRUMBENCH-ASR.md:121-123, RELEASE-PLAN.md:54-56, announce.md to describe Apache 2.0; delete the "license expires" badge.
  6. Add tests: `cli_starts_without_network_and_without_env_vars` (spawn `metrumbench-llm --version-only` with a scrubbed environment and assert exit 0 in under 1 s); `no_time_bomb` (grep-based test over src/ asserting that no `NaiveDate::from_ymd_opt` literal remains outside test code; or a `cargo deny`-style ban list); CI job runs the full test suite with `faketime` set to a date 5 years ahead.
  7. Add LICENSE (Apache-2.0 text), NOTICE, and `license = "Apache-2.0"` in Cargo.toml (F-07).
- Effort: S
- Blocks open-source release: yes

---

### [HIGH] F-02 Personal email, internal hostnames, AWS account ID, a real SSH public key, an example password, public IP addresses, a Google Drive folder, a Google Chat webhook and a developer's home directory path are embedded in code, configuration and documentation

- Axis: F2
- Status: VERIFIED
- Location and inventory (all in tools/rustyphalanx/cli-tools unless stated):
  - Personal email `chetan@metrum.ai`: src/lib.rs:24 (banner "Support:"), src/lib.rs:286 (`LICENSE_CONTACT`), src/lib.rs:159 (test assertion), RELEASE-PLAN.md:43, README.md via license message. Also `insights-cli` defaults at docker-compose.yml:786-787 (context only).
  - Internal backup host `backups.metrum.ai` with credential-in-URL template `rest:https://USERNAME:PASSWORD@backups.metrum.ai/metrum-insights`: README.md:71, announce.md:19, Makefile:44, 49, 54.
  - AWS account ID and ECR registry `121701826775.dkr.ecr.us-east-1.amazonaws.com`: Makefile:37-42.
  - Restic snapshot ID `7e656dcc` baked into the Docker build as the source of "prompts CSV files": Dockerfile.metrumbench-llm:49, 64-65.
  - A 2048-bit RSA SSH public key (`ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQCZ8qkaOaJ81hKd3Zq4WF/3bjpwxMyq5EBQcW7IOE9e08Lc+...`) that appears to be a real operator key rather than a placeholder: README.md:1376, 1521, 1590; WORKFLOW.md:228, 285. A second key with `example@email.com` at examples/cloud-init-config.yaml:13 is plausibly a placeholder.
  - Cleartext example password `metrum@123` for a `metrum` sudo user: README.md:1516, 1589; WORKFLOW.md:227, 280.
  - Public IP addresses of past benchmark targets: README.md:277 (`3.238.233.118`), README.md:1642 (same), README.md:1721 (`98.82.115.115`).
  - Google Drive folder ID `175t00121DzdgKYiiG_mpH_deJAtO8SEa`: README.md:10, 12.
  - Google Chat webhook space `AAAALDT9lKQ` (default announcement target): metrum_ops/rustyphalanx_cli.py:47 (repo root, pulled into the release tarball by `_release_members`, rustyphalanx_cli.py:579-583).
  - Developer home path `/Users/cgadgil/dev/src/metrum-ai/insights/2026/insights-productization/...`: docs/multi-endpoint-plan.md:247.
  - Third-party GitHub repositories run with sudo at first boot by the ISO template: `github.com/Scotchman0/NVIDIA_Drivers` (iso_customizer.rs:493, cloud-init-sample.yaml:32, examples/cloud-init/cloud-init-sample.yaml:32) and `github.com/scalers-ai/llm-tools` (README.md:1530).
  - Git author emails in history for this directory include `ubuntu@ip-172-31-31-182.us-west-1.compute.internal` (2 commits) and `cgadgil@metriumml.com` (2 commits); history rewriting is outside this assessment's scope but should be considered when extracting the subtree.
  - An untracked but on-disk 46 MB release tarball `metrumbench-0.1.77-chetan-dev-linux-x86_64.tar.gz` containing `metrum_ops/` Python with `billing.cpython-311.pyc` and other platform bytecode, plus binaries named `langbench`, `spartan`, `babelphish` (earlier product names); it is gitignored via `*.md5` only for the checksum and by no rule for the tarball, so it would be committed by a careless `git add .`.
- Why it is wrong: The SSH key and the password are the items a security researcher will screenshot. The AWS account ID and ECR path are enumeration aids. The email and internal hostnames are noise that make the project look like an internal dump.
- Fix: Replace the SSH key with a truncated placeholder (`ssh-ed25519 AAAA...EXAMPLE user@example.com`) and rotate the real key if it is in use; replace `metrum@123` with a `<password>` placeholder; delete Restic, ECR and Google Chat targets from Makefile, Dockerfile and docs (they belong in the internal repo); replace public IPs with `<host>`; remove the Drive badges; add a `.gitignore` entry for `*.tar.gz`; run `gitleaks` or `trufflehog` in CI; scrub history with `git filter-repo` when extracting the subtree.
- Effort: S
- Blocks open-source release: yes

---

### [MEDIUM] F-03 Dependency licenses are compatible with Apache 2.0, with three items requiring a NOTICE entry or an explicit decision

- Axis: F3
- Status: VERIFIED for the license field of all 470 packages via `cargo metadata` (Appendix 8.9); `cargo deny` is not installed and was not run
- Location: Cargo.lock (470 packages, 466 unique in the normal-dependency graph)
- What the reviewer found: 465 of 470 packages declare a permissive SPDX expression containing MIT, Apache-2.0, BSD-2/3-Clause, ISC, Unicode-3.0, Zlib, 0BSD, CC0-1.0 or Unlicense. The remaining items:
  - `metrumbench 0.1.80`: no `license` field (the crate itself). Must be fixed (F-07).
  - `polars-arrow-format 0.1.0`: `license-file = ./LICENSE` rather than an SPDX expression; the file is Apache-2.0 (upstream is arrow2's format crate) but this must be verified in the vendored copy and recorded. Disappears if polars is removed (C-04).
  - `webpki-roots 1.0.7`: `CDLA-Permissive-2.0` (Mozilla's root store data). Compatible with redistribution; requires attribution in NOTICE.
  - `xxhash-rust 0.8.15`: `BSL-1.0` (Boost Software License), permissive, requires the license text to accompany source distributions. Comes in via polars.
  - `r-efi 5.3.0`: `MIT OR Apache-2.0 OR LGPL-2.1-or-later`, a choice expression; select MIT. Comes in via `getrandom` on UEFI targets and is not compiled for Linux or macOS.
  - `ryu 1.0.23`: `Apache-2.0 OR BSL-1.0`; select Apache-2.0.
  - One package under `(MIT OR Apache-2.0) AND NCSA` (compiler-builtins or similar); NCSA is permissive.
  No GPL, LGPL-only, MPL, SSPL or proprietary license appears in the graph.
- Fix: Add `deny.toml` with an allow list of the SPDX identifiers above and `cargo deny check licenses` in CI; generate a THIRD_PARTY_LICENSES file with `cargo about`; remove polars to eliminate the two ambiguous entries.
- Effort: S
- Blocks open-source release: no (compatible), but NOTICE is required

---

### [HIGH] F-04 Bundled data provenance

- Axis: F4
- Status: VERIFIED for presence; UNDETERMINED for licensing where marked
- Inventory:
  - `test-data/tiny.png` (70 bytes, 1x1 RGBA PNG). Trivially generated; no copyright concern. Provenance not recorded. Recommend a comment in a `test-data/README.md`.
  - `test-data/dummy.mp3` (104 bytes, MPEG layer III header with no meaningful audio). Trivially generated; no copyright concern. Provenance not recorded.
  - `test-data/dummy-endpoints-4.yaml`, `endpoints-4servers.yaml`: configuration, authored in-repo.
  - `prompt-tools/prompt-tools/alice_clean.txt` (140,389 bytes): the text of Alice's Adventures in Wonderland by Lewis Carroll, public domain worldwide (author died 1898). The file is a stripped derivative of a Project Gutenberg ebook (prompt-tools README.md:22 and 165-167 say so). Project Gutenberg's trademark license requires that the Gutenberg header and license be retained if the text is distributed under the Gutenberg name; the header has been removed ("clean ASCII"). Because the underlying text is public domain, distributing it without the Gutenberg header is permitted provided the Gutenberg name is not used. The README calls it "Alice's Adventures in Wonderland from Project Gutenberg". Fix: rephrase to "public domain text (Lewis Carroll, 1865)" and drop the Gutenberg attribution, or restore the header. Status: acceptable with the wording fix.
  - `prompt-tools/prompt-tools/sample.txt` (295 bytes): authored in-repo.
  - `examples/cloud-init-config.yaml`, `examples/cloud-init/cloud-init-sample.yaml`, `cloud-init-sample.yaml` (the last two are byte-identical, verified with `diff`): authored in-repo.
  - `vllm-launch-config.yaml`: authored in-repo.
  - `prompt-library.jsonl`: referenced throughout README.md:916-968 ("Based on analysis of the included prompt-library.jsonl file", 115,845 prompts) and gitignored (.gitignore:2). It is not in the tree. Its provenance, license, and the tokenizer used to compute the quoted length distribution are UNDETERMINED. The Dockerfile restores "prompts CSV files" from a private Restic snapshot instead. A public release that documents a dataset it does not ship, and whose only distribution path is a private backup, is incomplete.
  - The README's Hugging Face examples (README.md:768-836) reference `open-thoughts/OpenThoughts-114k`, `open-r1/OpenR1-Math-220k`, `KodCode/KodCode-V1`, `huggan/smithsonian_butterflies`, `nlphuji/flickr30k`. These are not bundled; users download them under their own licenses. Note that flickr30k's license restricts commercial use, which a vendor publishing benchmark results should be told.
- Fix: Add `test-data/README.md` and `prompt-tools/README.md` provenance notes; either publish `prompt-library.jsonl` with a license and generation script, or remove every README reference to it; note dataset license restrictions next to the Hugging Face examples.
- Effort: S
- Blocks open-source release: yes (a documented but unshipped dataset is a reproducibility blocker; see H-03)

---

### [HIGH] F-05 Coupling to the Insights platform and to Metrum infrastructure

- Axis: F5
- Status: VERIFIED
- What is coupled and how:
  - Release tooling: `announce`, `create_release`, `create_multi_arch_release`, `nvidia_stack`, `run_scenarios`, `scenario-combo`, `test-container-local`, `scripts/compare_audio_models`, `scripts/run_metrumbench_asr`, `scripts/run_metrumbench_vlm_single`, `scripts/test_metrumbench_vlm` are all 15 to 23 line Python stubs that import `metrum_ops.rustyphalanx_cli` from the platform repository root via `_bootstrap_root.py` (which searches for a `metrum_ops` directory upward and in `/app`, `/usr/local`, `/workspace`). None of them works outside the monorepo. `metrum_ops/rustyphalanx_cli.py` (761 lines) posts release announcements to a Google Chat webhook, builds Restic-published tarballs, and installs NVIDIA drivers. README.md:1707-1722 documents `scenario-combo` as a user-facing tool.
  - Distribution: README.md:65-92 gives Restic against `backups.metrum.ai` as the only installation method; there is no crates.io, GitHub Releases, Homebrew or container-registry path. `.github/workflows/release-rustyphalanx.yml` publishes only to Restic.
  - Docker image: `Dockerfile.metrumbench-llm` installs restic in the runtime image, restores prompt CSVs from a private snapshot at build time using BuildKit secrets, and uses `metrumbench-llm-wrapper` (Python) as the entrypoint, which backs up outputs to Restic after each run. The wrapper hard-codes `/app/metrumbench-llm` and `/app` as defaults.
  - `--restic-tag` flag on the benchmark binary exists only for the wrapper (D-08).
  - Output contract: the JSONL schema is consumed field-by-field by insights-cli `output_ingestion.py:883-947` and job_lifecycle.py; the deprecation notice in CHANGELOG.md:34-37 promises a schema break "in the next release" without a `schema_version` field to negotiate it (imagegen alone has `schema_version`, imagegen.rs:912, 1020).
  - The insights-cli agent sets `METRUM_SKIP_NTP_CHECK` and `METRUM_SKIP_LICENSE_CHECK` on behalf of the binaries (run_lifecycle.py:391-393).
  - mcpserver/: an empty Python package (0-byte `__init__.py`, 0-byte README.md) with a `fastmcp` dependency and a symlink to a design document. It does not require PostgREST or a JWT because it does not exist; nothing in this crate requires PostgREST, a Metrum JWT, or the control plane at runtime. The benchmark binaries themselves are standalone HTTP clients.
- What must be decoupled or made optional: remove the 11 Python stubs and `_bootstrap_root.py`; remove Restic, ECR and Google Chat from Makefile, Dockerfile and CI; replace the Docker entrypoint with the binary itself; drop `--restic-tag`; add `schema_version` to every output record; either implement or delete mcpserver; publish via GitHub Releases and crates.io.
- Effort: M
- Blocks open-source release: yes

---

### [CRITICAL] F-06 Missing open-source governance files and CI, confirmed individually

- Axis: F6
- Status: VERIFIED (Appendix 8.10)
- Absent in tools/rustyphalanx/cli-tools: `LICENSE`, `LICENSE.md`, `NOTICE`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`, `CODEOWNERS`, `.github/` (any workflow at crate level), `rust-toolchain.toml`, `deny.toml`, `clippy.toml`, `rustfmt.toml`, `.cargo/config.toml`. Zero source files carry an SPDX or copyright header (`grep -rl 'SPDX\|Copyright' src tests` returns nothing). The repository root also has no `LICENSE` or `NOTICE` (checked). The only CI touching the crate is `.github/workflows/release-rustyphalanx.yml`, which builds a tarball and publishes to Restic; it runs no tests, no clippy, no fmt.
- Present but misleading: README.md:9 links a "License: Proprietary" badge to a `LICENSE` file that does not exist.
- Fix: Add the files; adopt Apache-2.0 with `SPDX-License-Identifier: Apache-2.0` and a copyright line in every source file (the organization's header standard applies); add `ci.yml` running fmt, clippy with `-D warnings`, test, `cargo deny`, and a smoke run against the dummy server on stable and MSRV.
- Effort: S
- Blocks open-source release: yes

---

### [HIGH] F-07 Cargo.toml lacks every field required for crates.io publication and builds all eleven binaries by default

- Axis: F7
- Status: VERIFIED
- Location: Cargo.toml:1-5 (only `name`, `version`, `edition`, `autobins`), Cargo.toml:6-53 (11 `[[bin]]` entries), Cargo.toml:55-83 (dependencies)
- Missing: `description`, `license` (or `license-file`), `repository`, `homepage`, `documentation`, `readme`, `keywords`, `categories`, `authors` (optional but conventional), `rust-version` (MSRV), `exclude`/`include` (the crate root contains a 46 MB tarball, a .docx, and Python that would be packaged by `cargo publish`). `version` is 0.1.80 while README.md:1 says v0.1.78 and the assessment brief says 0.1.82; CHANGELOG.md's latest entry is v0.1.78; the tarball on disk is 0.1.77. No `[profile.release]` settings (no `lto`, `codegen-units`, `strip`), so binaries are 8 to 10 MB. `tracing` and `tracing-subscriber` are declared (Cargo.toml:82-83) and never used by any source file (verified by grep: zero references); `polars` is needed by three utilities only; `ntp`, `compile-time`, `sha2`, `lru`, `image`, `base64`, `tempfile` are each used by one or two binaries.
- Fix: Fill the metadata; set `rust-version`; split into a workspace (`metrumbench-core`, `metrumbench-cli`, `metrumbench-tools` for the utilities, or drop the utilities); use features to gate optional modalities; add `[profile.release] lto = "thin", codegen-units = 1, strip = true`.
- Effort: S
- Blocks open-source release: yes (`cargo publish` will refuse without `license` and `description`)

---

### [HIGH] H-01 Metric definitions: what is defined precisely enough to reimplement, and what is not

- Axis: H1
- Status: VERIFIED
- Location: README.md (1758 lines) and docs/multi-endpoint-plan.md:148-152
- Definitions that exist in the documentation, quoted:
  - docs/multi-endpoint-plan.md:148-152: "Token Rate = Total Tokens / Test Duration; Prompt Token Rate = Prompt Tokens / Test Duration; Completion Token Rate = Completion Tokens / Test Duration; Req Rate = Total Requests / Test Duration". "Test Duration" is not defined; the code uses the metrics window or total elapsed depending on ramp-up (A-05).
  - README.md:686-694: "`steady_state_seconds` equals `total_time_seconds - ramp_up_seconds`" (true only in metrumbench-vlm, vlm:818; metrumbench-llm sets it to the metrics window, llm:1410) and "`steady_state_requests_per_second` is calculated using only steady-state data".
  - README.md:1045: "Request Timeout: Total time allowed for a single request to complete (including response generation)".
  - README.md:258-259: "Word counts: Measures actual linguistic units ... Words per second: Real-world throughput metrics". `count_words` is `split_whitespace().count()` (llm:1917-1919); the README does not say so.
- Definitions that do not exist anywhere in the documentation: time to first token (which chunk counts, whether role deltas count, whether reasoning deltas count, whether connection setup is included); time per output token (formula, N versus N-1, exclusion of TTFT interval); inter-token latency (does not exist as a metric); response time (start and end events); the percentile estimator; which requests are included in each distribution (successes only for latency, successes plus errors for request rate: llm:322, 643); error rate denominator (differs per binary, A-08); RTF versus RTFx (ASR); WER normalization (none, A-16); `inference_time` source (server field or client clock, asr:814-818); `tokens.*.per_second` window; the meaning of `min`/`max`/`avg` on `total_tokens` when `usage` is absent (they are zeros); VLM TTFT (fabricated, A-03); imagegen `latency_ms` (wall clock, integer ms, A-18); `images_per_second` window.
- Why it is wrong: A reader cannot reimplement a single latency metric from the documentation. The only metric that is defined (token rate) is defined with an undefined denominator.
- Fix: Add `docs/METRICS.md` with, per metric: name, JSONL path, unit, formula, clock events, inclusion rule, estimator, and a worked example against the dummy server (this assessment's Appendix 8.3 is a starting point).
- Effort: S
- Blocks open-source release: yes

---

### [HIGH] H-02 Documentation that does not match the code, README.md checked line by line

- Axis: H2
- Status: VERIFIED
- Discrepancies (README.md unless stated; each checked against the Args struct or the function body):
  1. README.md:1 "MetrumBench v0.1.78" versus Cargo.toml:3 `version = "0.1.80"`.
  2. README.md:59 "Windows support coming soon" and RELEASE-PLAN.md items dated March to May 2025 presented as upcoming in a document last touched in 2026.
  3. README.md:61 "For OpenAI-compatible servers, use the full chat-completions URL ... the tool does not append a path" is true for metrumbench-llm/vlm/asr but false for metrumbench-imagegen, which appends `/images/generations` (imagegen.rs:751) and expects a base URL ending in `/v1` (imagegen.rs:62).
  4. README.md:65-92 installation via Restic: not usable by the public; README.md:86 `export PATH=$PATH:$PWD/target/release` after `cd metrumbench/bin` is a wrong path (the tarball places binaries under `metrumbench/target/release`, verified by listing the on-disk tarball).
  5. README.md:96-108 "List of Tools" omits `metrumbench-imagegen` entirely; `metrumbench-imagegen` is not mentioned anywhere in README.md (verified by grep).
  6. README.md:209 and 227-228 "Starts with 1 concurrent request; Linearly increases to 100 concurrent requests over 60 seconds": true for metrumbench-llm (permit scheduler, llm:1589-1601); false for metrumbench-vlm, which never reduces the semaphore below full concurrency and only sleeps between launches (vlm:1144, 1405-1411).
  7. README.md:263 and 560: "MetrumBench can infer the token counts from the word counts" and "For API providers that don't report token counts, MetrumBench will infer approximate token counts based on the word counts." No code path does this; `unwrap_or(0)` at llm:1099-1113 and `unwrap_or(prompt_tokens)` at llm:916-927 leave tokens at zero.
  8. README.md:301-303 "Images are encoded directly in the request payload": images are decoded, optionally resized, re-encoded as JPEG and then base64 encoded (vlm:903-936).
  9. README.md:399-405 metrumbench-vlm argument table omits `--streaming` because the VLM binary has none, yet README.md:496-500 promises "Response times and throughput" including "Image processing times", which are not measured (A-17).
  10. README.md:562-609 "Visualizing Results with Charting Tools ... located in the `reporting/` directory ... `python democharts.py`": no `reporting/` directory exists in the crate or the repository (verified: `find` for `democharts.py` returns nothing).
  11. README.md:632 `--request-timeout` default "120": the LLM binary's default is 300 (llm:109); 120 is the VLM and ASR default.
  12. README.md:615-638 "MetrumBench Arguments" table omits `--version-only`, `--restic-tag`, `--ramp-up-seconds` is present but `--stop-after-seconds` interaction is described only for ramp-up.
  13. README.md:686 versus llm:1410: `steady_state_seconds` definition (see H-01).
  14. README.md:742-748 `extract_prompts` "Output CSV file path": the tool writes JSONL (extract_prompts.rs:83-88).
  15. README.md:785-836: three Python snippets create CSV files "for metrumbench-llm" (`df.to_csv("openthoughts_prompts.csv")`, "Create CSV file for metrumbench-llm") while the tool rejects CSV with an error (prompt_inputs.rs:13-25) and README.md:625 says JSONL.
  16. README.md:1055-1061: examples pass `--prompts simple_prompts.csv`, `standard_prompts.csv`, `complex_prompts.csv`, which the binary rejects.
  17. README.md:1282-1283: "Convert filtered JSONL back to CSV for metrumbench-llm" followed by a `jsonl_to_csv` invocation whose output is unusable as `--prompts`.
  18. README.md:916-968: quotes a token-length distribution for `prompt-library.jsonl`, which is not shipped (F-04).
  19. README.md:1353-1361 and 1381-1396: `iso_customizer --password` is documented in the example and the table; no `--password` argument exists (iso_customizer.rs:17-76). README.md:1476 "Configures the user password if specified" is therefore also false.
  20. README.md:1686-1694: `launch_subprocess_with_yaml_config --config-file`: the flag is `--config` (launch_subprocess_with_yaml_config.rs:14-15).
  21. WORKFLOW.md:174-178: `add_column_to_jsonl --column-value`: the flag is `--value` (add_column_to_jsonl.rs:21-23). WORKFLOW.md:55: URL path `/api/openai/chat/completions` is not an OpenAI-compatible path.
  22. README-METRUMBENCH-ASR.md in its entirety: `--audio-file`, `--output-format`, `--metrics wer,cer,bleu,rouge`, `--duration`, `--config config.yaml`, BLEU and ROUGE metrics, and six commercial providers; none exists in metrumbench-asr.rs. The real flags (`--input`, `--ground-truth`, `--response-format`, `--language`) are not mentioned.
  23. README.md:1730-1748 "Roadmap" lists "April 2025 Warmup Request Support" and "Hugging Face Tokenizer Integration" as upcoming; neither is implemented as of September 2026.
  24. CHANGELOG.md:34-37 (v0.1.75) says "The old format will be disabled (removed) in the next release"; v0.1.76, 0.1.77 and 0.1.78 have shipped and the format is unchanged.
  25. docs/multi-endpoint-plan.md:108-112 and 235-239 describe per-request JSONL lines (`{"endpoint": ..., "response_time": 0.5, "ttft": 0.1}`); the implementation writes one summary line per run and no per-request lines.
  26. announce.md:38-42 lists "Improved streaming JSON assembly" as an improvement; the assembly defect is A-02.
- Documentation that matches the code and is worth keeping: README.md:407-426 (VLM JSONL input contract, matches prompt_inputs.rs:136-208); README.md:644-667 (endpoints YAML format, matches endpoints.rs); README.md:1037-1242 timeout semantics (match reqwest usage) except the default at item 11.
- Fix: Regenerate every argument table from `--help` output (clap can emit Markdown via `clap_mangen`/`clap-markdown`); delete the charting, Restic, ISO and dataset sections that do not correspond to shipped code; rewrite README-METRUMBENCH-ASR.md from scratch.
- Effort: M
- Blocks open-source release: yes

---

### [HIGH] H-03 Reproducibility: a third party cannot reproduce a published Metrum AI result from this repository alone

- Axis: H3
- Status: VERIFIED
- What is missing, in dependency order:
  1. The prompt dataset (`prompt-library.jsonl`) is not in the repository and its generation method is undocumented (F-04).
  2. The prompt sequence is nondeterministic (A-10); even with the dataset, the request order differs per run.
  3. Sampling parameters are incomplete: no `seed`, no `ignore_eos`, hidden system prompt (A-09), so the server's work differs per run.
  4. The measurement window is undefined (A-05) and there is no warmup (A-12), so two runs of the same length with different drain tails give different throughput.
  5. The output record omits the environment: no hostname, OS, CPU count, kernel, client version of reqwest, tokio worker count, TLS mode, or server-reported model version; only `compile_info` (rustc version and build time) and `metrumbench_version` are present (llm:1283-1284).
  6. There are no per-request records, so a published p99 cannot be re-derived or audited.
  7. The percentile estimator is undocumented and differs by binary (A-04).
  8. The binaries cannot be built from the repository on a current toolchain without a lockfile change (A-06), and the released binaries refuse to run (F-01).
  9. The console output includes wall-clock timestamps only in the log lines; the JSONL `timestamp` is run end time (`Utc::now()` at record creation, llm:1282), not start time.
- Fix: Ship the dataset or a generator; add `--seed`; record a complete `environment` and `server` block (query `/v1/models` and, when available, the server's version endpoint); write per-request records; publish the estimator; add a `docs/REPRODUCING.md` with an exact command that reproduces a checked-in reference result against the dummy server to within stated tolerances (the reviewer's Appendix 8.3 run is a candidate reference).
- Effort: M
- Blocks open-source release: yes

---

### [MEDIUM] H-04 Version identity is inconsistent across five places

- Axis: H2, F7
- Status: VERIFIED
- Location: Cargo.toml:3 (0.1.80); README.md:1 and 1750 (v0.1.78); CHANGELOG.md:3 (latest entry v0.1.78); announce.md:1 (v0.1.78); README-METRUMBENCH-ASR.md:1 (v0.1.78); iso_customizer.rs:15 (0.2.2); on-disk tarball (0.1.77); assessment brief (0.1.82, not found anywhere in the tree).
- Fix: Single source of truth in Cargo.toml; CI check that CHANGELOG has an entry for the current version; drop the per-binary version constant.
- Effort: S
- Blocks open-source release: no

## 4. Refactoring plan

### 4.1 Target architecture

One Cargo workspace, three crates, one binary with subcommands. The modality differences are confined to three traits. Everything that measures or summarizes is shared and pure.

```
metrumbench/
  Cargo.toml                      workspace; [workspace.package] license, repository, rust-version
  LICENSE  NOTICE  CONTRIBUTING.md  CODE_OF_CONDUCT.md  SECURITY.md  CODEOWNERS
  deny.toml  rustfmt.toml  clippy.toml  rust-toolchain.toml
  .github/workflows/ci.yml        fmt, clippy -D warnings, test, deny, smoke vs dummy server, MSRV
  .github/workflows/release.yml   GitHub Releases artifacts for linux/macos x86_64/arm64, crates.io publish
  docs/
    METRICS.md                    every metric: formula, clock events, estimator, inclusion rules
    OUTPUT_SCHEMA.md              JSON schema for per-request records and run summary, versioned
    REPRODUCING.md                exact commands to reproduce the reference result vs dummy server
    COMPARISON.md                 mapping of flags and metrics to GenAI-Perf, vLLM, guidellm, MLPerf
  crates/
    metrumbench-core/             library, no CLI, no I/O to stdout
      src/lib.rs
      src/config.rs               BenchmarkConfig (load, endpoints, dataset, sampling, output, seed)
      src/clock.rs                trait Clock { fn now(&self) -> Instant }; RealClock; test FakeClock
      src/load/
        mod.rs                    trait ArrivalSchedule; ClosedLoop(concurrency); OpenLoop(rate, Poisson|Constant); Sweep
        scheduler.rs              issues RequestSlot { seq, scheduled_at } honoring schedule + max_concurrency
        warmup.rs                 Phase::Warmup | Measure | Drain; window computation
      src/transport/
        mod.rs                    trait Transport { async fn send(&self, req: HttpRequest) -> Result<ByteStream> }
        reqwest_transport.rs      real client, pool config, prewarm
        mock.rs                   scripted bytes with timing, for tests
      src/sse.rs                  SseParser::feed(&[u8]) -> Vec<SseEvent>; pure; complete-line framing; UTF-8 safe
      src/modality/
        mod.rs                    trait RequestBuilder { fn build(&self, item: &DatasetItem, cfg) -> HttpRequest }
                                  trait ResponseParser { fn on_headers(..); fn on_event(&mut self, ev, at: Instant); fn finish(self) -> ParsedResponse }
                                  trait MetricExtractor { fn extract(&self, parsed: &ParsedResponse, timing: &Timing) -> RequestRecord }
        openai_chat.rs            chat + completion, streaming + non-streaming, reasoning_content aware
        openai_vision.rs          image parts, preprocessed payloads, streaming reuse of openai_chat parser
        openai_audio.rs           multipart transcription, WER/CER with normalizer, RTFx
        openai_images.rs          images/generations, artifact hashing
      src/dataset/
        mod.rs                    trait Dataset { fn next(&mut self, rng) -> DatasetItem }; Jsonl, Synthetic(random tokens, prefix control), Multiturn (later)
        tokenizer.rs              optional feature "tokenizer": HF tokenizer.json via `tokenizers` crate
      src/record.rs               RequestRecord { seq, phase, endpoint, scheduled_at, sent_at, first_byte_at, first_token_at, first_reasoning_at, completed_at, chunk_times: Vec<Instant>, usage, tokenized, error: Option<RequestError>, ... }
      src/stats.rs                percentile(type7 default, configurable), mean, std, mad, bootstrap CI; n carried with every distribution
      src/summary.rs              Summary::from_records(&[RequestRecord], Window, SloConfig): latency dists, ITL dist over all chunk deltas, throughput over window and per-bin, goodput, per-endpoint, mixture flag
      src/sink.rs                 trait Sink { fn record(&mut self, r: &RequestRecord); fn summary(&mut self, s: &Summary) }; JsonlSink (incremental), StdoutTable, later OTLP
      src/error.rs                enum RequestError (thiserror), serde-tagged
    metrumbench-cli/              binary `metrumbench` with subcommands llm | vlm | asr | imagegen | selftest
      src/main.rs                 clap; CommonArgs flattened; signal handling; exit codes
      src/args_common.rs          endpoint(s), load (--concurrency | --request-rate --arrival), --num-prompts, --duration, --warmup-requests, --seed, --tokenizer, --result-dir, --ntp-check, --goodput ttft:...,tpot:...
      src/args_llm.rs ...         per-modality flags
    metrumbench-dummy-server/     optional: Rust port of tools/dummy-model-server for hermetic tests (or keep the Go server as a test fixture)
  tests/
    golden/                       hand-verified inputs and expected summaries
    e2e/                          spawns the CLI against the dummy server
```

What stays per binary (per subcommand): argument definitions specific to the modality, the three trait implementations, and modality-specific output fields (image statistics, WER, artifact hashes). Everything else (scheduling, transport, SSE, records, stats, summary, sinks, signal handling, logging, endpoints, datasets) is shared and lives in `metrumbench-core`.

The iso_customizer, launch_*_with_yaml_config, wait_for_vllm, nvidia_stack, release scripts and polars utilities move to an internal repository. `wait_for_vllm` can be replaced by `metrumbench wait --url` if desired (30 lines in the CLI crate).

### 4.2 Ordered sequence of refactors

Each step lists files touched, risk, and the tests that must exist and pass before the step starts (so the step cannot silently change numbers).

| Step | Change | Files touched | Risk | Tests required before starting |
|---|---|---|---|---|
| 0 | Fix the build and remove the gates: update ethnum in Cargo.lock; delete `mod license` and 10 call sites; make NTP opt-in (`--ntp-check`) and remove it from utilities; add LICENSE, NOTICE, SPDX headers, Cargo metadata, CI (fmt, clippy, test) | Cargo.toml, Cargo.lock, src/lib.rs, all 10 binaries, README | Low | `cli_starts_without_network`; existing 49 tests |
| 1 | Introduce `RequestRecord` and incremental JSONL sink in metrumbench-llm only, without changing any computed number: capture `sent_at`, `first_token_at`, `completed_at`, chunk timestamps; write one line per request; keep the old summary byte-for-byte | metrumbench-llm.rs | Low (additive) | Golden test: summary JSON for a scripted dummy-server run is unchanged except new fields |
| 2 | Extract `stats.rs` (type 7 percentile, std, mad, n) into the library; replace the six percentile implementations; record the estimator name in output. This changes numbers deliberately | src/stats.rs (new), all four binaries | Medium (numbers change; must be announced in CHANGELOG as the estimator fix) | `stats_percentile_matches_reference_table` (numpy/R generated), `stats_small_n_flags_unreliable` |
| 3 | Extract `sse.rs` with `SseParser::feed`; replace the inline loop in metrumbench-llm; fix A-02, A-14 | src/sse.rs (new), metrumbench-llm.rs | Medium | Adversarial SSE corpus tests (D-04 list) all green on the new parser before wiring |
| 4 | Extract `summary.rs`: `Summary::from_records` with explicit `Window`; fix A-05, A-08 denominators; add ITL from chunk timestamps, TPOT with N-1, `no_output_token` error class (A-01, A-03) | src/summary.rs (new), metrumbench-llm.rs | High (every number changes; this is the methodology release) | Golden summaries recomputed by an independent Python script from the per-request JSONL of step 1; window tests with FakeClock |
| 5 | Extract `transport.rs` and `modality/openai_chat.rs`; metrumbench-llm becomes a thin main over core | src/transport/, src/modality/, metrumbench-llm.rs | Medium | Mock-transport e2e: scripted stream produces expected RequestRecords |
| 6 | Port metrumbench-vlm onto core: reuse openai_chat parser with streaming; preprocess images before the window (A-17); delete fabricated TTFT (A-03) | modality/openai_vision.rs, metrumbench-vlm.rs | Medium | Golden VLM run vs dummy server; image preprocessing timing recorded |
| 7 | Port metrumbench-asr onto core: normalizer, RTFx, typed inference-time source (A-16) | modality/openai_audio.rs, metrumbench-asr.rs | Medium | `wer_matches_jiwer_reference` table (generated with jiwer + Whisper normalizer offline and checked in) |
| 8 | Port metrumbench-imagegen onto core: monotonic clock (A-18), keep its per-request records and balancers, move least-inflight into core for all modalities | modality/openai_images.rs, metrumbench-imagegen.rs, load/ | Low | Golden imagegen run |
| 9 | Add open-loop scheduler (Poisson, constant), `--seed` everywhere, warmup phase, sampling controls (`ignore_eos`, `min_tokens`, `seed`, extra body), unique-prompt salting, optional tokenizer (A-07, A-09, A-10, A-11, A-12, A-13) | load/, dataset/, args | Medium | Determinism test (same seed, identical request log on dummy server); Poisson inter-arrival distribution test; coordinated-omission test with FakeClock |
| 10 | Collapse to one binary with subcommands; keep old names as shims for one release; regenerate docs from clap | metrumbench-cli | Low | CLI snapshot tests (`--help` output) |
| 11 | Remove polars utilities, iso_customizer, launchers, Python stubs; workspace split; crates.io publish dry run | Cargo.toml, src/bin/*, scripts | Low | `cargo publish --dry-run` in CI |

### 4.3 Line-count estimate and risk

Current: 9804 lines of Rust, of which 6460 are the four benchmark binaries and 2067 are unrelated utilities.

Estimated after refactor: core library about 3200 lines (scheduler 300, transport 250, SSE 200, four modality modules 4 x 250, dataset and tokenizer 350, record and stats 300, summary 450, sinks 200, config and errors 200); CLI about 700 lines; tests about 2500 lines (currently 290 plus inline). Net production code drops from roughly 8500 (excluding the current tests) to roughly 3900, a reduction of about 55 percent, while the test volume increases roughly eightfold.

Risk: Steps 2 and 4 change published numbers by design (estimator, window, TPOT denominator). They must ship as one clearly labeled methodology release with a CHANGELOG section that states the old and new formula for each affected field and a migration note for insights-cli ingestion (output_ingestion.py:883-947). The remaining steps are behavior-preserving and are protected by golden tests. The largest schedule risk is the tokenizer integration in step 9 (the `tokenizers` crate compiles ONNX-free but is a heavy dependency; gate it behind a feature).

## 5. Test plan

Named tests, in priority order. Tests marked (P0) must exist before the first public tag; (P1) within 90 days; (P2) strategic.

Golden-file metric tests (pure, no network):
1. (P0) `summary_matches_hand_computed_reference_16_requests`: feed the 16 `RequestRecord`s from Appendix 8.3 (dummy server, 100 ms latency, 20 x 20 ms chunks) and assert TTFT mean 120 ms plus tolerance, RT mean 500 ms plus tolerance, ITL mean 20.0 ms, TPOT (N-1) 20.0 ms, completion tokens/s 158.45, requests/s 7.92, n = 16 on every distribution.
2. (P0) `summary_excludes_no_output_token_requests_from_ttft_and_counts_them`: a record with no first token yields `errors.by_type.no_output_token = 1` and TTFT n decremented.
3. (P0) `summary_window_excludes_warmup_and_drain`: with FakeClock, records in warmup and after the last issue are excluded; throughput denominator equals window length exactly.
4. (P0) `summary_error_rate_denominator_is_attempted_requests`: 120 attempted, 10 failed yields 8.33 percent regardless of `num_requests`.
5. (P0) `summary_undefined_stats_serialize_as_null_not_zero`.
6. (P1) `summary_per_endpoint_distributions_are_independent_and_aggregate_is_flagged_mixture`.
7. (P1) `summary_goodput_counts_requests_meeting_all_slos`.

Percentile and statistics tests:
8. (P0) `percentile_type7_matches_numpy_reference_table`: check in a table of 200 (sample, p, expected) triples generated by numpy.percentile with default method for n in {1, 2, 3, 10, 11, 20, 50, 99, 100, 101, 1000} and p in {50, 90, 95, 99, 99.9}.
9. (P0) `percentile_small_n_sets_unreliable_flag`: n = 50 at p99 flags; n = 100 at p99 does not.
10. (P1) `std_and_mad_match_reference`; `bootstrap_ci_covers_true_mean_in_95_percent_of_synthetic_trials` (seeded).

SSE parser tests (pure bytes in, events out):
11. (P0) `sse_event_split_across_two_feeds_is_reassembled` (the "split" case from Appendix 8.5).
12. (P0) `sse_two_events_in_one_feed_both_emitted`.
13. (P0) `sse_utf8_multibyte_split_at_every_offset_round_trips` (a 4-byte emoji-free CJK character split at each of its byte offsets; content compared byte-exact).
14. (P0) `sse_stream_without_done_but_with_finish_reason_is_complete`.
15. (P0) `sse_stream_closed_before_finish_is_truncated_error`.
16. (P0) `sse_role_only_then_finish_yields_no_output_token`.
17. (P0) `sse_reasoning_content_delta_sets_first_reasoning_at_not_first_token_at`.
18. (P1) `sse_crlf_line_endings`, `sse_comment_and_id_lines_ignored`, `sse_data_without_space_after_colon`, `sse_64kb_single_event`, `sse_malformed_json_event_is_counted_not_fatal`, `sse_error_object_in_stream_is_api_error`.

End-to-end tests against the dummy server (hermetic; the Go server is started by the test or replaced by a Rust mock server):
19. (P0) `e2e_llm_streaming_matches_server_timing_model` (this assessment's Appendix 8.3 as the oracle, tolerances 5 ms on means).
20. (P0) `e2e_llm_nonstreaming_ttft_equals_latency_and_itl_is_null`.
21. (P0) `e2e_ctrl_c_writes_partial_summary_and_all_completed_records`.
22. (P0) `e2e_http_503_counted_as_error_with_status_and_excluded_from_latency` (dummy server `-error-rate`).
23. (P0) `e2e_429_counted_as_rate_limit`.
24. (P1) `e2e_vlm_streaming_ttft_is_measured_not_derived`.
25. (P1) `e2e_asr_wer_with_normalizer_matches_jiwer_table` (checked-in table from jiwer + Whisper normalizer for 50 reference/hypothesis pairs including punctuation, casing, numbers, contractions).
26. (P1) `e2e_imagegen_latency_uses_monotonic_clock_and_subms_resolution`.
27. (P1) `e2e_multi_endpoint_weights_distribute_within_tolerance`.

Determinism and scheduling:
28. (P0) `same_seed_produces_identical_request_sequence` (dummy server request log compared byte-for-byte across two runs).
29. (P1) `poisson_arrivals_have_expected_interarrival_distribution` (KS test on 10k scheduled times with FakeClock).
30. (P1) `open_loop_latency_measured_from_scheduled_at_captures_coordinated_omission` (FakeClock, one slow response, assert subsequent latencies include queueing delay).
31. (P1) `closed_loop_never_exceeds_concurrency`.

Clock independence:
32. (P0) `all_intervals_use_instant_not_systemtime`: a lint-style test that greps the core crate for `Utc::now()`/`SystemTime::now()` outside `record.timestamp` fields; plus `summary_unaffected_by_wall_clock_step` using FakeClock with a simulated wall-clock step.

Robustness:
33. (P0) `no_panic_on_zero_completion_tokens_zero_duration_and_empty_run` (property-style over RequestRecord).
34. (P1) `memory_bounded_at_100k_requests` (records streamed to sink; resident set measured below a threshold in CI).

Release hygiene:
35. (P0) `cli_starts_without_network_and_env` (no NTP, no license).
36. (P0) `help_snapshot_matches_docs` (generated CLI reference is byte-identical to docs).
37. (P0) `cargo deny check licenses` and `cargo publish --dry-run` in CI.

Coverage target: 80 percent line coverage of `metrumbench-core` measured with `cargo llvm-cov` in CI, reported on every PR.

## 6. Competitive gap table (axis G)

Feature legend for the comparison columns: OL = open-loop load with request rate; PA = Poisson arrivals; GP = goodput and SLO reporting; PC = prefix-cache-aware or unique-prompt datasets; MT = multi-turn sessions; SO = structured-output benchmarking; TK = tokenizer-accurate token counts; RP = HTML or plot reports; OT = OpenTelemetry export; MF = MLPerf-compatible result format; SW = sweep or scan modes; SM = server-side metric correlation (scraping /metrics).

| Tool | What it does that metrumbench does not | What metrumbench does better | Would a source-reading reviewer find metrumbench more or less trustworthy |
|---|---|---|---|
| NVIDIA GenAI-Perf | OL, PA, GP (`--goodput ttft:... itl:...`), PC (synthetic prompts with tokenizer-controlled length and uniqueness), MT (`--num-sessions`, `--session-turns-mean`), TK (HF tokenizer), RP (CSV, JSON, console table with std), SW (`analyze` subcommand sweeping concurrency or rate), warmup, per-chunk ITL, embeddings and rankings endpoints, Triton and OpenAI backends, telemetry collection from DCGM. | Single static binary with no Python environment; multi-endpoint weighted distribution in one process; ASR and image-generation modalities (GenAI-Perf has neither); JSONL summary with compile provenance. | Less trustworthy. GenAI-Perf's SSE handling, tokenizer-based counts, documented metric definitions and published estimator (numpy) are all things this tool lacks or gets wrong (A-01 to A-05). |
| vLLM benchmark_serving.py | OL with `--request-rate` and `--burstiness` (gamma), `--max-concurrency` cap, GP (`--goodput`), PC (random dataset with `--random-prefix-len`, ShareGPT, sonnet, HF datasets), TK (model tokenizer for output counts), per-request `itl` list and std, `--ignore-eos`, `--seed`, single test request before measurement, per-request results in the JSON, percentile selection (`--metric-percentiles`), multiple backends (vLLM, TGI, OpenAI chat/completions, TensorRT-LLM, DeepSpeed-MII, SGLang). | No Python or model download needed for the client; lower client CPU per request (measured about 40k non-streaming requests/s here); ASR, VLM and image generation. | Less trustworthy. benchmark_serving.py is short, widely read, and its TTFT/TPOT/ITL formulas are visible in one function; a reviewer will compare `(latency - ttft) / (output_len - 1)` against this tool's `/ completion_tokens` and the six percentile implementations. |
| LLMPerf (Ray) | Token-level output length control, tokenizer counting, per-request result files, correctness test mode (`llm_correctness`), many API backends (Anthropic, Bedrock, Vertex, Together, Fireworks, LiteLLM), Ray-based distributed clients. | Static binary, no Ray; streaming SSE parsing in-process; multi-endpoint; non-LLM modalities. | Roughly comparable on load model (LLMPerf is also closed-loop by concurrency), less trustworthy on token accounting (LLMPerf tokenizes; this tool trusts `usage`). |
| MLCommons MLPerf Inference (LoadGen, LLM server and offline scenarios) | PA by definition in the Server scenario, latency constraints (TTFT and TPOT p99 bounds), minimum query counts and durations, accuracy checks against reference outputs, audited and published results, MF (result JSON and summary format), determinism by fixed seeds, warmup rules, a compliance test suite. | Ease of use: MLPerf requires a system-under-test harness per model; metrumbench is a one-command client. Broader modality coverage in one tool (MLPerf has separate benchmarks). | Much less trustworthy. MLPerf's value is auditability; this tool has no per-request records, undefined windows, and a license time bomb. |
| guidellm (vLLM project) | OL with `--rate-type constant|poisson|sweep`, synthetic data with token-length distributions, tokenizer, warmup and cooldown percentages, per-request records, HTML report (RP), YAML/JSON output with schema, Pydantic-validated results, checkpointing, multi-turn chat datasets. | Static binary; multi-endpoint; ASR and image modalities; lower client overhead. | Less trustworthy. guidellm's benchmarker abstraction and typed results are what section 4 proposes; this tool is four divergent copies. |
| InferenceX | Publishes full concurrency sweep curves with methodology, standardized ISL/OSL pairs, prefix caching disabled or disclosed, per-config repeatability, hardware and software stack disclosure, results database and leaderboard. | Not comparable in scope: InferenceX is a results program built on a runner (it uses vLLM and other benchmark clients); metrumbench is a client only. Metrumbench can drive non-LLM endpoints. | Less trustworthy as a measurement instrument; InferenceX's disclosure requirements (seed, ISL/OSL, cache state, repetitions) are exactly the gaps in H-03. |

Matrix of the enumerated features (Y implemented, N absent, P partial):

| Feature | GenAI-Perf | vLLM bench | LLMPerf | MLPerf | guidellm | metrumbench |
|---|---|---|---|---|---|---|
| Open-loop load (OL) | Y | Y | N | Y | Y | N |
| Poisson arrivals (PA) | Y | Y (gamma) | N | Y | Y | N |
| Goodput and SLO (GP) | Y | Y | N | Y (constraints) | P | N |
| Prefix-cache-aware datasets (PC) | Y | Y | P | Y (fixed) | Y | N |
| Multi-turn sessions (MT) | Y | P | N | N | Y | N |
| Structured output benchmarking (SO) | N | Y (`benchmark_serving_structured_output.py`) | N | N | N | N |
| Tokenizer-accurate counts (TK) | Y | Y | Y | Y | Y | N |
| HTML or plot reports (RP) | P (CSV/JSON) | N | N | N | Y | N (README references a non-existent `reporting/` directory) |
| OpenTelemetry export (OT) | N | N | N | N | N | N |
| MLPerf-compatible format (MF) | N | N | N | Y | N | N |
| Sweep and scan modes (SW) | Y | N (external loop) | N | N | Y | N (an internal `scenario-combo` Python stub, not shipped) |
| Server-side metric correlation (SM) | Y (DCGM, Triton metrics) | N | N | N | N | N |
| Per-chunk inter-token latency | Y | Y | Y | Y | Y | N |
| Warmup phase | Y | P (single test request) | N | Y | Y | N |
| Seeded determinism | Y | Y | Y | Y | Y | N (imagegen sends a server seed only) |
| Per-request records | Y | Y | Y | Y | Y | N (imagegen only) |
| Multi-endpoint in one process | N | N | N | N | N | Y |
| ASR benchmark | N | N | N | N (separate benchmark) | N | Y |
| Image generation benchmark | N | N | N | N (separate benchmark) | N | Y |
| VLM benchmark | Y (multimodal) | Y (VisionArena, HF datasets) | N | N | Y | Y (non-streaming only) |

The three things this tool must fix to be taken seriously:

1. Measure what it claims to measure: per-chunk timestamps and a real inter-token latency distribution; TTFT that excludes connection setup via warmup and that records requests with no output token as failures rather than as 0 ms; a correct, tested SSE framing layer; one documented percentile estimator with sample counts. (Findings A-01, A-02, A-03, A-04, A-12, D-04.)
2. Define the workload and the window: seeded request order, `ignore_eos`/`min_tokens`/`seed` controls, unique-prompt salting or tokenizer-controlled synthetic prompts, an explicit measurement window with warmup and drain excluded and per-request records that make the window auditable, and open-loop arrivals so the saturation curve can be measured. (Findings A-05, A-07, A-09, A-10, A-11, A-19.)
3. Ship one implementation: a core library with a single metrics path shared by all four modalities, tested against golden files and an adversarial corpus, without a license gate or a mandatory NTP call. (Findings A-06, A-08, C-01, E-01, F-01.)

The one genuine differentiator: a single static binary that benchmarks four OpenAI-compatible modalities (chat/completions, vision chat, audio transcription with WER/CER, and image generation) and can distribute load across several endpoints with per-endpoint accounting, with no Python runtime or model download on the client. None of the six comparison tools covers ASR or image generation, and none does in-process multi-endpoint distribution. That differentiator is real but only becomes an asset once the measurement core is correct; today it multiplies the defects by four.

## 7. Prioritized roadmap

Effort is in engineer-weeks (ew) for one experienced Rust engineer familiar with the code; a second engineer roughly halves wall-clock time for wave (i) and (ii) because most items are independent.

### Wave (i): blocks open-source release

| # | Item | Findings | Effort (ew) |
|---|---|---|---|
| 1 | Fix the build (ethnum), add rust-toolchain.toml, CI with fmt/clippy/test | A-06, D-07 | 0.4 |
| 2 | Remove the license mechanism and all ten call sites, tests, docs, badges; make NTP opt-in `--ntp-check` recording offset, remove from utilities | F-01, A-20 | 0.4 |
| 3 | Scrub secrets, keys, passwords, internal hosts, IPs, account IDs, Drive links, home paths; add gitleaks to CI; plan history filter for the subtree extraction | F-02 | 0.4 |
| 4 | LICENSE, NOTICE, SPDX headers, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, CODEOWNERS, Cargo metadata, deny.toml, THIRD_PARTY_LICENSES | F-03, F-06, F-07 | 0.4 |
| 5 | Remove platform coupling: Python stubs, Restic, ECR, Google Chat, Docker wrapper, `--restic-tag`, mcpserver stub; move iso_customizer and launchers out; drop polars utilities | F-05, C-04 | 0.6 |
| 6 | Per-request record struct and incremental JSONL sink; Ctrl-C handler writing partial summary; `schema_version` on every record | A-19, D-05, F-05 | 1.0 |
| 7 | SSE parser as a pure function with the adversarial corpus tests; fix split events, missing DONE, role-only streams as `no_output_token` errors; `reasoning_content` awareness | A-01, A-02, A-14, D-04 | 1.0 |
| 8 | Shared stats module: type 7 percentile, n, std, null for undefined, small-n flag; replace six implementations | A-04, B-03 | 0.6 |
| 9 | Shared summary with explicit window (warmup, measure, drain), per-chunk ITL, TPOT with N-1, unified error-rate and rate denominators across binaries; remove VLM fabricated TTFT; fix imagegen wall clock | A-03, A-05, A-08, A-18, D-06 | 1.5 |
| 10 | Warmup phase; `--seed` threaded through all RNGs; `--ignore-eos`, `--min-tokens`, `--seed` (server), `--extra-body-json`, system prompt control; unique-prompt salting | A-09, A-10, A-11, A-12 | 1.0 |
| 11 | Typed error enum with HTTP status; `usage_missing` classification; delete the misleading usage warning | A-13, D-02 | 0.4 |
| 12 | docs/METRICS.md, docs/OUTPUT_SCHEMA.md, README rewrite generated from clap, delete false sections, rewrite ASR README, dataset provenance notes, ship or unreference prompt-library | H-01, H-02, H-03, F-04, H-04 | 1.0 |
| 13 | Golden and e2e tests from section 5 marked P0 (about 25 tests) | E-01, E-02 | 1.5 |
| | Wave (i) total | | 10.2 |

### Wave (ii): needed within 90 days of launch to survive scrutiny

| # | Item | Findings | Effort (ew) |
|---|---|---|---|
| 14 | Open-loop scheduler: `--request-rate`, `--arrival constant|poisson`, `--max-concurrency`, coordinated-omission-corrected latency from `scheduled_at` | A-07 | 2.0 |
| 15 | Optional tokenizer (`tokenizers` crate, HF tokenizer.json) for input and output counts; report usage and tokenized side by side | A-11, A-13 | 1.5 |
| 16 | Dispersion: std, MAD, per-bin throughput, `--runs N` with cross-run aggregation and bootstrap CI | B-01, B-02, B-04 | 1.5 |
| 17 | Goodput and SLO flags (`--slo ttft=...,tpot=...,e2e=...`) | G | 0.5 |
| 18 | Core library extraction completed (transport trait, modality traits, mock transport); thin CLI with subcommands; deprecated shims | C-01, C-02, C-03 | 3.0 |
| 19 | VLM: streaming support, image preprocessing before the window, original bytes by default | A-17, A-03 | 1.0 |
| 20 | ASR: Whisper normalizer port, RTFx, typed inference-time source, file read outside timer | A-16 | 1.0 |
| 21 | Per-endpoint full distributions, least-inflight balancer for all modalities, mixture flag | A-15 | 0.8 |
| 22 | Client self-test mode and environment block (host, cores, runtime config, server model info) | D-03, H-03 | 0.6 |
| 23 | P1 tests from section 5; llvm-cov in CI with an 80 percent gate on core | E-01 | 1.5 |
| 24 | docs/COMPARISON.md and docs/REPRODUCING.md with a checked-in reference result | H-03, G | 0.6 |
| | Wave (ii) total | | 14.0 |

### Wave (iii): strategic

| # | Item | Effort (ew) |
|---|---|---|
| 25 | Sweep mode (concurrency or rate scan) producing the latency-throughput curve in one run, with automatic knee detection | 2.0 |
| 26 | Server-side metric correlation: scrape vLLM/SGLang/TensorRT-LLM `/metrics` during the window and attach KV-cache usage, preemptions, running/waiting queues to the summary | 2.0 |
| 27 | Multi-turn session datasets with shared prefix control | 2.0 |
| 28 | Structured output (JSON schema, tool-call) benchmarking with validity rate as a goodput dimension | 1.5 |
| 29 | HTML report with plots (single self-contained file) and CSV export of per-request records | 1.5 |
| 30 | OpenTelemetry (OTLP) export of per-request spans and summary metrics | 1.5 |
| 31 | MLPerf-compatible result export (LoadGen-style summary and detail logs) for the Server and Offline scenarios | 2.5 |
| 32 | Rust dummy server as a test fixture and as a published `metrumbench-mock-server` for CI users | 1.5 |
| 33 | Embeddings and reranking endpoints | 1.0 |
| 34 | crates.io and Homebrew distribution, signed release artifacts, SBOM | 1.0 |
| | Wave (iii) total | 16.5 |

Grand total: about 40.7 engineer-weeks, of which 10.2 are required before a public repository can be created without immediate retraction risk.

## 8. Appendix: command output

All commands were run on 2026-09-15 (UTC) on a 16-core Linux x86_64 host (kernel 6.8.0-139-generic, 30 GB RAM) with rustc 1.97.1 and cargo 1.97.1 (stable). Long outputs are trimmed to the relevant lines; trimmed regions are marked `[...]`. Output glyphs outside ASCII printed by the binaries (a U+274C cross mark in the license and NTP error paths) are replaced by `[U+274C]` in this appendix.

### 8.1 cargo build --release

First attempt, unmodified tree (commit 400c2206b, Cargo.lock pinning ethnum 1.5.2):

```
$ cargo build --release
   Compiling rmp v0.8.15
   Compiling polars-utils v0.48.1
   [...]
   Compiling ethnum v1.5.2
   [...]
error[E0512]: cannot transmute between types of different sizes, or dependently-sized types
  --> /home/cgadgil/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/ethnum-1.5.2/src/error.rs:16:14
   |
16 |     unsafe { mem::transmute(()) }
   |              ^^^^^^^^^^^^^^
   |
   = note: source type: `()` (0 bits)
   = note: target type: `TryFromIntError` (8 bits)

For more information about this error, try `rustc --explain E0512`.
error: could not compile `ethnum` (lib) due to 1 previous error
warning: build failed, waiting for other jobs to finish...
```

Diagnosis and workaround applied for the remainder of the assessment (the lockfile was restored to the committed state afterwards; `git status` shows a clean tree):

```
$ cargo update -p ethnum --dry-run
    Updating crates.io index
     Locking 1 package to latest compatible version
    Updating ethnum v1.5.2 -> v1.5.3
$ cargo update -p ethnum
$ cargo build --release
    Finished `release` profile [optimized] target(s) in 3m 12s
EXIT=0
```

Zero compiler warnings in the release build. Binary sizes: metrumbench-llm 8,041,640 bytes; metrumbench-vlm 10,228,816; metrumbench-asr 8,044,856; metrumbench-imagegen 9,531,136.

### 8.2 cargo fmt, cargo test, cargo clippy

```
$ cargo fmt --check
Diff in .../src/prompt_inputs.rs:116:
 /// Path may be a local file path or an http:// / https:// URL; content must be JSONL.
 /// Minimum contract: one object per line with "prompt" (string).
 /// Preserves embedded newlines in prompt text. Fails fast with file/line or URL/status errors.
-pub fn load_metrumbench_llm_prompts(path: &str) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
+pub fn load_metrumbench_llm_prompts(
+    path: &str,
+) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
FMT_EXIT=1
```

```
$ METRUM_SKIP_NTP_CHECK=1 cargo test
test result: ok. 23 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out   (lib: endpoints 6, prompt_inputs 13, lib 4)
test result: ok. 0 passed  (add_column_to_jsonl)
test result: ok. 0 passed  (extract_prompts)
test result: ok. 4 passed  (iso_customizer)
test result: ok. 0 passed  (jsonl_to_csv)
test result: ok. 0 passed  (launch_container_with_yaml_config)
test result: ok. 0 passed  (launch_subprocess_with_yaml_config)
test result: ok. 0 passed  (metrumbench-asr)
test result: ok. 3 passed  (metrumbench-imagegen)
test result: ok. 10 passed (metrumbench-llm)
test result: ok. 0 passed  (metrumbench-vlm)
test result: ok. 0 passed  (wait_for_vllm)
test result: ok. 9 passed  (tests/cli_validation.rs)
test result: ok. 0 passed  (doc-tests)
EXIT=0
Total: 49 passed, 0 failed. Wall time for the test step including compilation: 96 s.
```

All 49 test names:

```
endpoints::tests::test_hostname_from_url
endpoints::tests::test_load_endpoints_empty_file_fails
endpoints::tests::test_load_endpoints_empty_url_fails
endpoints::tests::test_load_endpoints_invalid_yaml
endpoints::tests::test_load_endpoints_valid_yaml_defaults_name_and_weight
endpoints::tests::test_load_endpoints_yaml_with_name_and_weight
prompt_inputs::tests::test_is_http_url
prompt_inputs::tests::test_load_metrumbench_llm_prompts_malformed_json
prompt_inputs::tests::test_load_metrumbench_llm_prompts_missing_prompt
prompt_inputs::tests::test_load_metrumbench_llm_prompts_url_inside_tokio_runtime
prompt_inputs::tests::test_load_metrumbench_llm_prompts_valid
prompt_inputs::tests::test_load_metrumbench_vlm_records_missing_image_urls
prompt_inputs::tests::test_load_metrumbench_vlm_records_valid
prompt_inputs::tests::test_normalize_image_ref
prompt_inputs::tests::test_read_utf8_from_path_or_url_file
prompt_inputs::tests::test_read_utf8_from_path_or_url_http
prompt_inputs::tests::test_reject_csv_metrumbench_llm
prompt_inputs::tests::test_reject_csv_metrumbench_vlm
prompt_inputs::tests::test_reject_csv_url
tests::check_license_allows_explicit_skip
tests::check_ntp_sync_allows_explicit_skip
tests::license_expiry_date_is_extended_through_end_of_july_2026
tests::license_message_tracks_configured_expiry_date
tests::customize_iso_fails_when_boot_config_is_not_at_iso_root          (iso_customizer)
tests::customize_iso_patches_default_and_hwe_grub_entries_for_cloud_init (iso_customizer)
tests::mounted_iso_contents_source_uses_trailing_separator_for_rsync    (iso_customizer)
tests::rsync_extract_args_keep_copied_iso_tree_writable                 (iso_customizer)
tests::parses_size                                                       (imagegen)
tests::summary_computes_images_per_second                                (imagegen)
tests::summary_keeps_error_http_status                                   (imagegen)
tests::zero_ramp_up_is_treated_as_no_ramp_up                             (llm)
tests::oom_classifier_matches_explicit_oom_errors                        (llm)
tests::oom_classifier_does_not_match_oom_substrings                      (llm)
tests::stream_choice_output_text_ignores_role_only_chunks                (llm)
tests::stream_choice_output_text_preserves_empty_content                 (llm)
tests::stream_choice_output_text_extracts_non_empty_delta_content        (llm)
tests::stream_choice_output_text_supports_completion_text_chunks         (llm)
tests::stream_choice_output_token_detects_function_call_arguments        (llm)
tests::stream_choice_output_token_detects_tool_call_arguments            (llm)
tests::stream_choice_output_token_ignores_finish_chunks                  (llm)
metrumbench_llm_rejects_concurrency_zero                                 (cli_validation)
metrumbench_llm_rejects_invalid_mode
metrumbench_llm_rejects_num_requests_zero
metrumbench_vlm_rejects_concurrency_zero
metrumbench_vlm_rejects_image_cache_size_zero
metrumbench_vlm_rejects_invalid_image_detail
metrumbench_asr_rejects_concurrency_zero
metrumbench_asr_rejects_num_requests_zero
metrumbench_asr_rejects_invalid_response_format
```

```
$ cargo clippy --all-targets -- -W clippy::all
EXIT=0 (warnings only). 65 warning lines; unique warnings by lint and location:

clone_on_copy (9): metrumbench-asr.rs:294:9, 390:17, 391:17; metrumbench-llm.rs:286:9, 440:17, 441:17; metrumbench-vlm.rs:267:9, 375:17, 376:17
unnecessary_cast (5): extract_prompts.rs:48:25, 49:25, 64:26, 65:26; metrumbench-vlm.rs:579:54
single_component_path_imports (4): iso_customizer.rs:6:1; launch_container_with_yaml_config.rs:2:1; launch_subprocess_with_yaml_config.rs:2:1; metrumbench-vlm.rs:4:1
needless_range_loop (4): metrumbench-asr.rs:630:14, 633:14, 680:14, 683:14
too_many_arguments (8): metrumbench-llm.rs:218:5 (10/7), 310:5 (10/7); metrumbench-asr.rs:213:5 (14/7), 309:5 (8/7), 722:1 (8/7); metrumbench-imagegen.rs:741:1 (8/7); metrumbench-vlm.rs:208:5 (9/7), 273:5 (9/7)
redundant_closure (3): metrumbench-llm.rs:1102:18, 1107:18, 1112:18
len_zero (3): metrumbench-asr.rs:1580:8; metrumbench-llm.rs:1826:8; metrumbench-vlm.rs:1536:8
type_complexity (2): endpoints.rs:60:6; prompt_inputs.rs:142:6
manual_strip (2, 4 spans): metrumbench-llm.rs:895:25, 896:50; prompt_inputs.rs:129:5, 130:9
for_kv_map (2): launch_container_with_yaml_config.rs:200:13, 234:13
useless_format (1): iso_customizer.rs:869:33
manual_repeat_n (1): metrumbench-imagegen.rs:719:32
useless_conversion (1): jsonl_to_csv.rs:101:39
```

None of the clippy findings indicates a runtime bug. `cargo llvm-cov` and `cargo deny` are not installed on the host and were not installed during the assessment.

### 8.3 End-to-end run against the dummy model server, streaming

Server: `tools/dummy-model-server/bin/dummy-model-server -mode metrumbench-llm -port 18321 -latency 100ms -chunk-interval 20ms`. The server sleeps 100 ms before the first byte, then emits one SSE chunk per token with a 20 ms sleep before each chunk (including the first), each chunk carrying `delta.content = "."`, then a finish chunk with `usage`, then `data: [DONE]`. Prompt tokens are `len(content)/4` over all messages including the injected system prompt.

Client: `metrumbench-llm --num-requests 16 --concurrency 4 --max-tokens 20 --mode chat --streaming --prompts prompts.jsonl` (8 unique prompts) with `METRUM_SKIP_NTP_CHECK=1 METRUM_SKIP_LICENSE_CHECK=1`.

Console output (banner removed):

```
metrumbench-llm version 0.1.80
01:46:16 [WARN] Token usage statistics missing in 90.9% of chunks      (printed 16 times, once per request)
Response Time Statistics:
Min: 504.348844ms, Max: 505.421103ms, Avg: 504.812111ms
p50: 504.724363ms, p90: 505.402087ms, p95: 505.402087ms, p99: 505.421103ms
Time to First Token Statistics:
Min: 120.647598ms, Max: 121.2401ms, Avg: 120.873991ms
p50: 120.790571ms, p90: 121.222316ms, p95: 121.222316ms, p99: 121.2401ms
Prompt Tokens Statistics:
Min: 17, Max: 19, Avg: 18.00, Median: 18
Total Prompt Tokens: 288
Completion Tokens Statistics:
Min: 20, Max: 20, Avg: 20.00, Median: 20
Total Completion Tokens: 320
Total Tokens Statistics:
Min: 37, Max: 39, Avg: 38.00, Median: 38
Total Tokens Processed: 608
Time Per Output Token Statistics:
Min: 0.019181s, Max: 0.019217s, Avg: 0.019197s
p50: 0.019204s, p90: 0.019213s, p95: 0.019213s, p99: 0.019217s
Word Count Statistics:
Prompt Words:
  Min: 8, Max: 10, Avg: 8.62, Median: 9
  Total Prompt Words: 138
  Words Per Second: 68.33
Completion Words:
  Min: 1, Max: 1, Avg: 1.00, Median: 1
  Total Completion Words: 16
  Words Per Second: 7.92
Error Analysis:
Total Errors: 0
Timing Statistics:
Total Time: 2.019513668s
Average Request Latency: 504.812111ms
Requests/Second: 7.92 (successful: 16, failed: 0)
Prompt Tokens/Second: 142.61
Completion Tokens/Second: 158.45
Total Tokens/Second: 301.06
Scenario: stream-check
Version: 0.1.80
EXIT=0
```

JSONL summary record (metrics subset; note integer millisecond truncation on every `_ms` field):

```json
"response_times": {"avg_ms": 504, "max_ms": 505, "min_ms": 504, "p50_ms": 504, "p90_ms": 505, "p95_ms": 505, "p99_ms": 505},
"ttft":           {"avg_ms": 120, "max_ms": 121, "min_ms": 120, "p50_ms": 120, "p90_ms": 121, "p95_ms": 121, "p99_ms": 121},
"tpot":           {"avg_s": 0.019196906, "max_s": 0.0192169052, "min_s": 0.0191810291, "p50_s": 0.01920383825, "p90_s": 0.0192130188, "p95_s": 0.0192130188, "p99_s": 0.0192169052},
"timing":         {"failed_requests": 0, "metrics_collection_seconds": 2.019524959, "ramp_up_seconds": null, "requests_per_second": 7.922655240627804, "steady_state_requests_per_second": 7.922655240627804, "steady_state_seconds": 2.019524959, "successful_requests": 16, "total_time_seconds": 2.01956175},
"tokens": {"completion": {"total": 320, "per_second": 158.45310481255606, ...}, "prompt": {"total": 288, ...}, "total": {"total": 608, ...}},
"words":  {"completion": {"total": 16, "avg": 1.0, ...}, "prompt": {"total": 138, "avg": 8.625, ...}},
"errors": {"count": 0, "messages": [], "oom_occurred": false, "rate": 0.0, "types": {}}
```

Top-level keys: `compile_info`, `config`, `human_readable_id`, `metrics`, `metrumbench_version`, `timestamp`, `unique_id`. There is no `schema_version`, no `environment`, no per-request array.

Hand verification against the server's timing model:

```
RT   expected 500 ms (100 latency + 20 chunks x 20 ms); measured avg 504.812 ms; client and transport overhead 4.812 ms
TTFT expected 120 ms (100 + first 20 ms chunk sleep); measured avg 120.874 ms; overhead 0.874 ms
TPOT as the tool defines it = (RT - TTFT) / 20 = 19.1969 ms; tool reports 19.1969 ms      (arithmetic matches)
True server inter-token interval = 20.000 ms; (RT - TTFT) / 19 = 20.2073 ms
  The tool divides by N = 20 instead of N - 1 = 19 intervals and reports an ITL 4.0 percent low.
Completion tokens/s = 320 / 2.01956 s = 158.45; tool reports 158.45                            (matches)
Requests/s = 16 / 2.01956 s = 7.923; tool reports 7.923                                       (matches)
Ideal closed-loop rate at concurrency 4 = 4 x 20 / 0.504812 = 158.47 tok/s; equal here because 16 requests
  form exactly 4 full waves with no partial drain tail. With 17 requests the window would include a
  0.5 s tail at concurrency 1 and the reported rate would drop to about 340 / 2.52 = 134.9 tok/s.
Completion words = 1 per request because the server sends 20 "." chunks with no whitespace; the tool's
  word count (split_whitespace) sees one word for 20 tokens.
```

Prompt tokens: the server counts `len(content)/4` over both messages. The injected system prompt "You are a helpful assistant." is 28 characters, adding 7 prompt tokens to every request, which is why the prompt token minimum is 17 for an 8-word user prompt.

### 8.4 License gate reproduction

```
$ date -u
Tue Sep 15 01:46:09 AM UTC 2026
$ METRUM_SKIP_NTP_CHECK=1 metrumbench-llm --scenario lic --url http://127.0.0.1:18321/v1/chat/completions --api-key dummy \
    --num-requests 1 --concurrency 1 --prompts prompts.jsonl --mode chat --model dummy --data-log lic.jsonl --max-tokens 4
        AI Performance Testing Tools: metrumbench-llm v0.1.80
        LLM load testing tools from Metrum AI Inc., 2024
        Support: chetan@metrum.ai
metrumbench-llm version 0.1.80
[U+274C] Error: License expired. This version is valid until July 31, 2026. Contact chetan@metrum.ai for renewal.
Please contact chetan@metrum.ai for renewal.
Error: "License expired. This version is valid until July 31, 2026. Contact chetan@metrum.ai for renewal."
EXIT=1
ls: cannot access 'lic.jsonl': No such file or directory
```

### 8.5 Adversarial SSE server reproductions

A 60-line Python raw-socket server (`adversarial_sse_server.py`, in the assessment scratch directory) answers any POST with a chunked `text/event-stream`. Four modes were run with `metrumbench-llm --num-requests 1 --concurrency 1 --streaming --max-tokens 8 --log-level trace`.

Mode `split`: events = role delta; content "Hello" sent as two TCP writes split inside the JSON at `"choices"`, 50 ms apart; content " world"; content "!"; content " done"; finish chunk with `usage {prompt 10, completion 4}`; `[DONE]`.

```
debug log:
  Received chunk: {... "delta": {"role": "assistant"} ...}
  Buffering incomplete JSON chunk...
  JSON PARSING ERROR: key must be a string at line 1 column 65
  Received chunk: {... "delta": {"content": "!"} ...}
  Received chunk: {... "delta": {"content": " done"} ...}
  Received chunk: {... "finish_reason": "stop" ... "usage": {"prompt_tokens": 10, "completion_tokens": 4, ...}}
  Request completed - RT: 302.13972ms, TTFT: 201.685612ms, Tokens: 14
result: EXIT=0, errors 0, completion words 2 (true value 3: "Hello", "world!", "done"), TTFT 201 ms
        (true first content chunk arrived at about 100 ms). The "Hello" event and the following " world"
        event were both lost.
```

Mode `roleonly`: events = role delta; 300 ms pause; finish chunk with usage; `[DONE]`.

```
debug log:
  Received chunk: {... "delta": {"role": "assistant"} ...}
  Received chunk: {... "finish_reason": "stop" ... "usage": {...}}
  Request completed - RT: 301.421297ms, TTFT: 0ns, Tokens: 14
console:
  Time to First Token Statistics:
  Min: 0ns, Max: 0ns, Avg: 0ns
  p50: 0ns, p90: 0ns, p95: 0ns, p99: 0ns
result: EXIT=0, errors 0, ttft all fields 0, tpot 0.0754 s (= RT / 4 because TTFT is zero), completion words 0,
        completion tokens 4 (from usage).
```

Mode `nodone`: events = role delta; content "Hello"; finish chunk with usage; connection closed without `data: [DONE]`.

```
debug log:
  Received chunk: {... "delta": {"content": "Hello"} ...}
  Received chunk: {... "finish_reason": "stop" ... "usage": {...}}
  Stream ended unexpectedly - Last received data: Some(51.012876ms)
result: EXIT=1, errors 1 ("Unexpected stream termination"), no latency samples recorded, although content,
        finish_reason and usage all arrived intact.
```

Mode `multitok`: events = role delta; four content chunks of "one two three four five " 100 ms apart; finish chunk with `usage {completion_tokens: 20}`; `[DONE]`.

```
result: EXIT=0, TTFT 101 ms (correct), completion tokens 20 (from usage), completion words 20,
        tpot 0.0200 s per token. The true inter-chunk interval was 100 ms; no inter-token or inter-chunk
        latency is measured, so a 100 ms stall between chunks is invisible.
```

### 8.6 Percentile estimator comparison

Pure-Python reimplementation of the three index formulas found in the code and two reference estimators, on a seeded lognormal sample (`percentile_compare.py` in the scratch directory). `llm_round` is `round((n-1)p)` (metrumbench-llm.rs:284, metrumbench-imagegen.rs:1078); `vlm_asr_ceil` is `ceil(np)-1` (metrumbench-vlm.rs:266, metrumbench-asr.rs:293); `linear_t7` is Hyndman-Fan type 7 (numpy default); `nearest_rank` is type 1.

```
n=10
  p50  llm_round=   707.59  vlm_asr_ceil=   707.59  linear_t7=   772.29  nearest_rank=   707.59  max=  1541.62
  p90  llm_round=  1149.79  vlm_asr_ceil=  1149.79  linear_t7=  1188.97  nearest_rank=  1149.79  max=  1541.62
  p95  llm_round=  1541.62  vlm_asr_ceil=  1541.62  linear_t7=  1365.30  nearest_rank=  1541.62  max=  1541.62
  p99  llm_round=  1541.62  vlm_asr_ceil=  1541.62  linear_t7=  1506.36  nearest_rank=  1541.62  max=  1541.62
n=20
  p50  llm_round=  1134.31  vlm_asr_ceil=  1044.67  linear_t7=  1089.49  nearest_rank=  1044.67  max=  2313.78
  p90  llm_round=  1598.90  vlm_asr_ceil=  1598.90  linear_t7=  1609.16  nearest_rank=  1598.90  max=  2313.78
  p95  llm_round=  1701.49  vlm_asr_ceil=  1701.49  linear_t7=  1732.11  nearest_rank=  1701.49  max=  2313.78
  p99  llm_round=  2313.78  vlm_asr_ceil=  2313.78  linear_t7=  2197.44  nearest_rank=  2313.78  max=  2313.78
n=50
  p50  llm_round=   828.57  vlm_asr_ceil=   828.57  linear_t7=   830.43  nearest_rank=   828.57  max=  3069.51
  p90  llm_round=  1953.53  vlm_asr_ceil=  1953.53  linear_t7=  1961.25  nearest_rank=  1953.53  max=  3069.51
  p95  llm_round=  2856.79  vlm_asr_ceil=  2856.79  linear_t7=  2826.45  nearest_rank=  2856.79  max=  3069.51
  p99  llm_round=  3069.51  vlm_asr_ceil=  3069.51  linear_t7=  2990.40  nearest_rank=  3069.51  max=  3069.51
n=100
  p50  llm_round=   990.83  vlm_asr_ceil=   974.41  linear_t7=   982.62  nearest_rank=   974.41  max=  3991.06
  p90  llm_round=  1860.23  vlm_asr_ceil=  1860.23  linear_t7=  1861.51  nearest_rank=  1860.23  max=  3991.06
  p95  llm_round=  2459.13  vlm_asr_ceil=  2459.13  linear_t7=  2459.42  nearest_rank=  2459.13  max=  3991.06
  p99  llm_round=  2910.17  vlm_asr_ceil=  2910.17  linear_t7=  2920.98  nearest_rank=  2910.17  max=  3991.06
n=1000
  p50  llm_round=   995.54  vlm_asr_ceil=   993.93  linear_t7=   994.74  nearest_rank=   993.93  max=  5787.04
  p90  llm_round=  1871.62  vlm_asr_ceil=  1871.62  linear_t7=  1871.73  nearest_rank=  1871.62  max=  5787.04
  p95  llm_round=  2268.64  vlm_asr_ceil=  2268.64  linear_t7=  2268.98  nearest_rank=  2268.64  max=  5787.04
  p99  llm_round=  3333.01  vlm_asr_ceil=  3333.01  linear_t7=  3333.48  nearest_rank=  3333.01  max=  5787.04
```

Observations: at n = 20 the two in-tree formulas disagree on p50 by 8.6 percent (1134.31 versus 1044.67); at n = 100 they disagree on p50 by 1.7 percent; p95 and p99 equal the sample maximum for n at or below 20 and p99 equals the maximum for n at or below 100 under `llm_round` (index round(99 x 0.99) = 98, which is the maximum for n = 100 only when rounding lands on 99; for n = 50 it is exactly the maximum). Neither formula matches numpy's default.

### 8.7 Client saturation measurement

Dummy server in metrumbench-llm mode with no injected latency and no chunk interval, on the same 16-core host as the client (server CPU is therefore included in the host load and the figures are lower bounds on client capacity).

```
streaming, 4000 requests, concurrency 64, max_tokens 8:
  Total Time: 239.865183ms   Requests/Second: 16676.03   Completion Tokens/Second: 133408.25   Total Tokens/Second: 433576.80
  Average Request Latency: 3.515992ms
  wall=0.25 s user=0.67 s sys=0.40 s maxrss=19200 KB cpu=432%
non-streaming, 4000 requests, concurrency 64, max_tokens 8:
  Total Time: 100.778839ms   Requests/Second: 39690.85   Completion Tokens/Second: 317526.82   Total Tokens/Second: 1031962.16
  Average Request Latency: 1.37505ms
  wall=0.11 s user=0.34 s sys=0.21 s maxrss=17920 KB cpu=509%
streaming, 2000 requests, concurrency 64, max_tokens 64 (64 SSE chunks per request):
  Total Time: 334.826066ms   Requests/Second: 5973.25   Completion Tokens/Second: 382287.97
  wall=0.34 s user=0.88 s sys=0.46 s maxrss=15488 KB cpu=392%
non-streaming, 20000 requests, concurrency 128 (memory growth check):
  Total Time: 389.709523ms   Requests/Second: 51320.27 (successful: 20000, failed: 0)
  wall=0.40 s user=1.76 s sys=0.89 s maxrss=49024 KB cpu=655%
```

Interpretation: the client parses roughly 380k SSE tokens per second using about 4 cores, and its resident memory grows with request count (17.9 MB at 4000 requests to 49.0 MB at 20000 requests, roughly 1.5 KB per completed request held until exit, consistent with A-19). No production inference server approaches these rates, so the client is not the bottleneck for LLM serving on this class of host; the tool does not itself report any of these numbers.

### 8.8 Ctrl-C behavior

```
$ metrumbench-llm --num-requests 400 --concurrency 2 --max-tokens 20 --streaming ... --data-log ctrlc.jsonl &
$ sleep 6; kill -INT $PID; wait $PID
exit code after SIGINT: 130
ls: cannot access 'ctrlc.jsonl': No such file or directory
```

Roughly 24 requests had completed at the time of the signal (2 concurrent, about 0.5 s each). No summary and no per-request data were written.

### 8.9 Dependency license summary (cargo metadata, 470 packages)

```
227  MIT OR Apache-2.0
102  MIT
 21  MIT/Apache-2.0
 21  Apache-2.0 OR MIT
 18  Unicode-3.0
 15  Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT
  7  MIT OR Apache-2.0 OR Zlib
  6  Unlicense OR MIT
  5  BSD-3-Clause
  4  BSD-2-Clause
  4  Apache-2.0/MIT
  4  Unlicense/MIT
  3  Zlib OR Apache-2.0 OR MIT
  3  Apache-2.0 OR ISC OR MIT
  3  MIT AND Apache-2.0
  3  Apache-2.0
  2  Zlib
  2  BSD-3-Clause OR Apache-2.0
  2  ISC
  2  BSD-2-Clause OR Apache-2.0 OR MIT
  1 each: 0BSD OR MIT OR Apache-2.0; Apache-2.0 WITH LLVM-exception; CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception;
          CC0-1.0 OR MIT-0 OR Apache-2.0; Apache-2.0 / MIT; CC0-1.0 OR Apache-2.0; (MIT OR Apache-2.0) AND NCSA;
          MIT OR Zlib OR Apache-2.0; MIT OR Apache-2.0 OR LGPL-2.1-or-later (r-efi); Apache-2.0 AND ISC;
          Apache-2.0 OR BSL-1.0 (ryu); (MIT OR Apache-2.0) AND Unicode-3.0; CDLA-Permissive-2.0 (webpki-roots);
          BSL-1.0 (xxhash-rust); license-file only (polars-arrow-format); none (metrumbench itself)
Dependency graph: 466 unique packages with polars; 298 without polars (polars subtree: 340 packages, of which 168 are
exclusive to it). Cargo.toml declares `tracing` and `tracing-subscriber`, which no source file uses.
```

### 8.10 Governance and CI file check

```
ABSENT  LICENSE            ABSENT  LICENSE.md         ABSENT  NOTICE             ABSENT  CONTRIBUTING.md
ABSENT  CODE_OF_CONDUCT.md ABSENT  SECURITY.md        ABSENT  CODEOWNERS         ABSENT  .github (crate level)
ABSENT  rust-toolchain     ABSENT  rust-toolchain.toml ABSENT deny.toml          ABSENT  clippy.toml
ABSENT  rustfmt.toml       ABSENT  .cargo
SPDX or Copyright headers in src/ and tests/: 0 files
Repository root LICENSE or NOTICE: absent
Repository CI referencing the crate: .github/workflows/release-rustyphalanx.yml only (make release, md5 check, Restic publish; no tests)
```

### 8.11 VLM, ASR and image generation runs against the dummy server

metrumbench-vlm, 8 requests, concurrency 2, `--max-tokens 20`, server latency 100 ms, input `test-data/tiny.png`:

```
Response Time Statistics:  Min: 100.64275ms, Max: 101.475711ms, Avg: 100.937563ms
Time to First Token Statistics:  Min: 5.032137ms, Max: 5.073785ms, Avg: 5.046877ms     (= RT / 20 completion tokens; not measured)
Time Per Output Token Statistics: Min: 4.78053ms, Max: 4.820096ms, Avg: 4.794533ms   (= (RT - RT/20) / 20)
Total Errors: 0     Average Requests/Second: 19.79     Total Images Processed: 8
JSONL timing: requests_per_second 19.78, steady_state_requests_per_second 8.0  (denominator clamped to 1.0 s by .max(1.0) at vlm:820; the two fields disagree by 2.5x on the same run)
JSONL images: image size 631 bytes per image after JPEG re-encoding of a 70-byte 1x1 PNG
```

metrumbench-asr, 4 requests, concurrency 2, `--response-format json`, durations 2.0 s and 4.0 s, ground truth "hello world" and "Hello, world.":

```
Audio Files Processed: 4     Request Rate: 19.73 req/sec
Response Time:  Min: 101.06145ms ... Avg: 101.257935ms
Inference Time: Min: 100ms, Max: 100ms, Avg: 100ms          (client-measured; dummy server sends no inference_time field)
Real-Time Factor (RTF): Min: 0.03, Max: 0.05, Avg: 0.04     (inference / duration; the leaderboard convention RTFx is the inverse)
Word Error Rate (WER): 100.0% on all four                    (dummy transcript versus reference; also 100% for punctuation-only differences, see A-16)
Character Error Rate (CER): Min: 138.5%, Max: 154.5%         (CER above 100 percent because the hypothesis is longer than the reference; not clamped or explained)
JSONL throughput.audio_per_second: 59.13 (12 s of audio / 0.203 s wall)
```

metrumbench-imagegen, 4 requests, concurrency 2, `--size 64x64 --no-save-images`, server latency 100 ms:

```
latency_ms {'max': 102.0, 'mean': 102.0, 'min': 102.0, 'p50': 102.0, 'p90': 102.0, 'p95': 102.0, 'p99': 102.0}   (integer milliseconds from wall clock; per-attempt monotonic latency was 102.699369 ms)
requests_per_second 19.49   images_per_second 19.49   duration_seconds 0.2053
per-request record present in img.jsonl with schema_version "metrumbench-imagegen.request.v1"
```

### 8.12 CSV conversion of the streaming result

```
$ jsonl_to_csv --input-jsonl-url file://.../stream.jsonl --output-csv-filename stream.csv
EXIT=0; 114 columns, 1 data row (one summary line per run)
metrics_response_times_p50_ms = 504        metrics_ttft_p50_ms = 120        metrics_tpot_p50_s = 0.01920383825
metrics_timing_requests_per_second = 7.922655240627804
metrics_per_endpoint_127.0.0.1:18321_ttft_avg_ms = 120.8739913125     (per-endpoint average keeps sub-ms precision; the top-level ttft avg_ms is truncated to 120)
metrics_per_endpoint_127.0.0.1:18321_response_times_p99_ms = 505
compile_info_rustc_version = 1.97.1
```

The CSV column names embed the endpoint host and port (`metrics_per_endpoint_127.0.0.1:18321_...`), so the schema changes with every endpoint, which prevents concatenating CSVs from different runs, the workflow the README recommends (README.md:174-179).

