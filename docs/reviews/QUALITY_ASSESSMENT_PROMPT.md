<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Assessment prompt: Metrum AI Bench, real-world value review

Paste everything below the line into a fresh session that has shell access to
this repository. The prompt is self-contained; it does not depend on any prior
conversation.

---

You are reviewing the repository at `/home/<user>/src/bench-cli` (Rust crate
`metrumbench`, binaries published as `metrum-ai-bench-*`). Record the HEAD
commit, the `Cargo.toml` version, the date, and the host (cores, RAM, kernel,
rustc, go) at the top of your report. Every line reference in the report must
point at that commit.

## 1. Who you are and what you are judging

You are a senior engineer who runs inference in production and has been burned
by benchmark numbers that did not survive contact with real traffic. You are
not a benchmarking-tool enthusiast and you are not a hardware vendor. You are
deciding whether this tool would change a real decision you have to make:

- **Persona A, capacity owner.** Runs a serving fleet behind a load balancer
  with TLS, auth, and a gateway. Needs to know how many replicas to buy for a
  given SLO under realistic arrivals.
- **Persona B, stack engineer.** Compares engines, quantizations, and
  configurations of the same model. Needs numbers that move only when the
  server changes, not when the client does.
- **Persona C, buyer.** Is shown vendor benchmark results and needs to know
  which knobs could have flattered them and whether the output file reveals
  that.

The vendor who wants the highest possible number is not a persona. Do not
propose changes whose only effect is a larger headline number.

## 2. Stance: the rules that decide what counts as a finding

Apply these before writing any finding. Several of them reverse conclusions
that a parity-checklist review would reach.

1. **Measure what the user experiences.** A production request pays DNS, TCP,
   TLS, authentication, gateway hops, server queueing, prefill, and decode.
   Time to first token that includes connection setup is the number a user
   sees. Including it is correct. The finding, if any, is about
   **decomposition and disclosure**: can the operator separate connect time,
   time to first byte, and time to first token from the same record, and does
   the output state which one the headline is? A knob that removes a real
   cost (connection pre-warming, warmup exclusion, prefix cache warming,
   `ignore_eos`) is legitimate only if it is opt-in and stamped into the
   output so a reader can see it was used. Criticizing the tool for
   including TLS in TTFT is wrong; criticizing it for not letting the reader
   tell TLS from prefill is right.
2. **Realistic workload beats synthetic purity.** Real traffic stops at EOS,
   hits the prefix cache some of the time, arrives in bursts, contains
   multi-turn conversations, gets 429s, and traverses proxies that buffer or
   re-chunk. The question is whether the tool can run the realistic case and
   report it honestly, and separately whether it can run the controlled case
   when the operator asks. Do not treat the controlled case as the default
   truth.
3. **Comparison tools are context, not the standard.** GenAI-Perf (retired; see AIPerf), vLLM
   `benchmark_serving.py`, guidellm, LLMPerf, MLPerf LoadGen, and InferenceX
   are references for what a metric name conventionally means. A missing
   feature relative to them is a finding only when one of the three personas
   cannot make a decision without it. A feature that exists here and there
   but is wrong here is a correctness finding, not a parity finding.
4. **Innovation is judged by what it lets someone learn or catch.** Something
   is innovative if it produces a decision-relevant fact that the other tools
   cannot, or exposes a misleading result that they would let pass. A feature
   that exists nowhere else but produces nothing an operator would act on
   counts for nothing.
5. **Honesty is a property of defaults and output, not of documentation.**
   Ask whether a careless or motivated user can produce a flattering result
   without leaving a trace in the output file. Every sampling parameter,
   hidden prompt, cache state, warmup exclusion, window definition, and
   estimator must be recoverable from the record alone.
6. **Claims are not evidence.** `CHANGELOG.md`, `docs/`, the README, and the
   earlier report at `(removed from main; prior OSS readiness assessment)` are lists of claims.
   Every claimed fix and every documented behavior you rely on must be
   reproduced by running the built binary. Mark anything you did not
   reproduce as SUSPECTED or UNDETERMINED.
7. **Numbers over narrative.** Recompute every headline statistic from the
   per-request records with an independent script and report the
   disagreement. A metric you could not recompute is a finding.
8. **Code volume is not a finding.** Duplication, file length, and style are
   findings only when they have produced a drift you can demonstrate in the
   output or a bug you can trigger.
9. **Never print secret values.** The tree may contain gitignored files with
   live credentials (for example `env.json`). Report their presence, their
   gitignore coverage, and whether any tracked file or git history contains
   them. Do not paste the value into the report, a log, or a command line.

## 3. Scope and environment

Read all of:

- `src/*.rs` (shared core: `sse.rs`, `stats.rs`, `summary.rs`, `record.rs`,
  `load.rs`, `transport.rs`, `strategic.rs`, `asr.rs`, `tokenizer.rs`,
  `environment.rs`, `error.rs`, `jsonl.rs`, `endpoints.rs`,
  `prompt_inputs.rs`, `args_common.rs`, `modality.rs`, `lib.rs`).
- `src/bin/*.rs` (six primary binaries, four deprecated shims,
  `metrum-ai-bench-mock-server.rs`).
- `tests/**`, `dummy-model-server/**` (Go), `scripts/**`, `packaging/**`,
  `.github/workflows/*.yml`, `Cargo.toml`, `Cargo.lock`, `deny.toml`,
  `.gitleaks.toml`, `rust-toolchain.toml`, `Makefile`, `test-data/**`.
- `README.md`, `CHANGELOG.md`, `CONTRIBUTING.md`, `SECURITY.md`, `NOTICE`,
  `THIRD_PARTY_LICENSES`, and every file under `docs/` including
  `SMOKE_RESULTS.md`, `REPRODUCING.md`, `METRICS.md`,
  `OUTPUT_SCHEMA.md`, `CLI.md`, `COMPARISON.md`, `STRATEGIC_BENCHMARKING.md`,
  `HISTORY_REWRITE.md`.
- `live-results/**` if present. It is gitignored and may hold real GPU runs
  from the campaign described in `scripts/live/README.md`. Use it as evidence of how
  the tool behaves against real vLLM and SGLang, and cross-check
  `docs/SMOKE_RESULTS.md` against the raw files it claims to summarize.

Build and test commands are in `CLAUDE.md`. The Go dummy server accepts
`-port`, `-latency`, `-chunk-interval`, and `-mode` flags and models a
known timing profile, which makes it an oracle: read its handlers and derive
the expected TTFT, end-to-end, and inter-token intervals before running the
client against it.

## 4. Method, in order

Run every step and keep the raw output for the appendix. Do not skip a step
because the docs say it is covered.

### 4.1 Static pass

1. Read every file in section 3. For each shared-core module, note whether
   each of the four modality binaries actually routes through it or carries
   a private copy. The binaries are still 1,400 to 2,100 lines each; find out
   what is in them.
2. Enumerate every hidden default that changes the server's work or the
   reported number: injected system prompt, temperature, `max_tokens`
   handling, `stream_options`, `ignore_eos`, `min_tokens`, `seed` sent to the
   server, tokenizer choice, JPEG re-encoding, audio read placement, request
   and connect timeouts, pool sizing, HTTP version, TLS verification, retry
   behavior. For each, state the default, whether it is recorded in the
   output `config` block, and which persona is misled if it is not.
3. Enumerate every place a request can succeed with a missing or zero
   measurement (no output token, missing `usage`, zero completion tokens,
   `[DONE]` absent, truncated stream) and how each is classified in the
   record.
4. Enumerate the percentile, mean, standard deviation, MAD, bootstrap, and
   throughput-bin implementations. There must be exactly one of each. Check
   that every distribution carries `n` and that undefined values serialize as
   null.
5. Read `src/strategic.rs` and `metrum-ai-bench-strategic.rs` as a separate
   unit: sweep stages, Kneedle knee detection, validity rules for JSON
   schema and tool calls, server `/metrics` scraping, CSV, HTML, MLPerf-style
   export, and OTLP. For each, decide whether it is implemented, whether it is
   correct, and whether its output labels its own limitations (the MLPerf
   export in particular must say it is not an audited result).

### 4.2 Toolchain gates

Run and record exit codes and full output:

```bash
cargo build --release
cargo test --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo deny check   # if installed; otherwise say so
bash scripts/check_headers.sh
cd dummy-model-server && go vet ./... && go test ./...
```

Run the end-to-end suites with `METRUM_BENCH_REQUIRE_DUMMY=1` so that a
missing dummy server fails instead of skipping, and report how many tests
actually executed versus were skipped.

### 4.3 Reference reproduction

Follow `docs/REPRODUCING.md` exactly. Compare the emitted summary to
`test-data/reference-result.json` field by field. Then derive the expected
values from the dummy server's source and compare again. Report every field
that differs from either, with the tolerance the docs promise.

### 4.4 Independent recomputation

Write a short script (Python or Rust, kept in the appendix) that reads the
per-request JSONL lines from a run, recomputes every summary field
(distributions with Hyndman-Fan type 7, throughput over the declared window,
error rate, goodput against the configured SLOs, ITL pooled over chunk
deltas, TPOT with N minus 1, per-endpoint and pooled distributions) and diffs
them against the summary line the tool wrote. Do this for one run per
modality and for one multi-endpoint run. Any field you cannot recompute from
the records alone is a reproducibility finding.

### 4.5 Real-world scenarios

Construct each of these against the dummy server, the mock server, or a small
local stand-in (a raw-socket SSE server, a local TLS terminator such as
`socat` or a minimal Rust/Go proxy). For each, record what the tool reported,
what the truth was, and whether the output file reveals the difference.

1. **TLS in the path.** Put a self-signed TLS terminator in front of the dummy
   server. Run with and without connection pre-warming or warmup. Confirm
   that TTFT includes the handshake for cold connections, that this is the
   documented and expected behavior, and check whether the record lets the
   reader separate connect time, first byte, and first token. Report what a
   Persona A reader would conclude and whether they would be right.
2. **Gateway re-chunking.** SSE events split across TCP writes, two events in
   one write, CRLF line endings, comment and `id:` lines, a 64 KB event, a
   multi-byte UTF-8 character split at each byte offset.
3. **Server variants.** Missing `data: [DONE]`; finish chunk with `usage`
   then connection close; role-only stream; reasoning-only stream
   (`reasoning_content` deltas and no `content`); stream where `usage` is
   never sent; non-streaming response with no `usage`.
4. **Error storms.** A phase of 429s, a phase of 503s, a phase of connection
   resets, and a slow-loris endpoint that sends headers and nothing else
   until the request timeout. Check the typed error classes, whether failed
   requests are excluded from latency and included in attempted counts, and
   whether open-loop mode keeps its schedule while the server fails.
5. **Overload in open loop.** `--request-rate` above what the server can
   serve, with and without `--max-concurrency`. Confirm that the headline
   latency includes queue delay from scheduled arrival, that the record
   carries `scheduled_offset_s` and `queue_delay_s`, and that the summary
   states when the cap engaged.
6. **Prefix cache realism.** A small prompt file cycled many times, with
   `--unique-prompts` on and off. Confirm both states are stamped in the
   config block. The realistic case (repeats allowed) must be runnable and
   labeled, not forbidden.
7. **Natural stopping.** Run with EOS honored and with `--ignore-eos`.
   Confirm the choice is recorded. Decide whether the tool's defaults favor
   the realistic case or the synthetic case and whether that is disclosed.
8. **Multi-endpoint with one dead replica.** Two endpoints, one refusing
   connections. Check round-robin versus least-inflight behavior, that
   per-endpoint distributions are complete, and that the pooled block is
   marked as a mixture.
9. **Long run and interruption.** A run of several thousand requests at
   moderate concurrency: resident memory over time, records flushed
   incrementally, SIGINT mid-run yielding a `partial: true` summary and no
   lost completed records, SIGKILL leaving a parseable prefix.
10. **Clock step.** If feasible, step the wall clock during a run (or fake it
    with a library shim) and confirm no interval metric moves.
11. **Multi-turn.** Run the strategic runner with a session file. Confirm
    each turn is submitted with its history, that prefix control modes
    behave as documented, and that per-turn TTFT is reported separately from
    first-turn TTFT.
12. **Sweep and knee.** Run a concurrency sweep and a rate sweep against the
    mock server with a known saturation point. Check whether the knee the tool
    reports is the true knee and whether the HTML and CSV outputs are
    self-contained and honest about sample sizes.
13. **Modality checks.** VLM: confirm image bytes are sent unchanged unless
    resize or re-encode is requested, that preprocessing happens outside the
    measured window and is reported, and that streaming TTFT is measured
    rather than derived. ASR: confirm the normalizer applies to both sides,
    that RTFx is audio seconds over client wall time, and that server-reported
    and client-measured inference times are separate fields. Imagegen:
    confirm monotonic latency, warmup exclusion, and per-request artifact
    hashes.

### 4.6 Regression against the prior report

`(removed from main; prior OSS readiness assessment)` lists findings A-01 through H-04 against a
previous tree, and `CHANGELOG.md` v0.1.80 through v0.1.82 claims to have fixed
most of them. Produce a table with one row per prior finding: finding ID,
claimed status, your reproduction command, observed result, and verdict
(FIXED, PARTIAL, NOT FIXED, NOT APPLICABLE). Do not accept a test's existence
as proof; run the behavior.

### 4.7 Published results audit

Treat `docs/SMOKE_RESULTS.md` as a public claim. Check that every number in it
can be traced to a raw file under `live-results/` (if present), that units are
stated, that sample sizes support the percentiles shown (p95 at n=56, for
example, must carry the reliability flag the tool itself emits), that
dummy-certified rows are labeled as such and are not presented next to real
GPU rows in a way a reader could confuse, and that the system-under-test
provenance is complete enough for someone else to rerun it.

## 5. Assessment axes and scorecard

Score each axis 1 to 5. Anchor: 1 means a Persona would be actively misled;
3 means usable with caveats the output itself discloses; 5 means the output
alone is sufficient to make the decision and to catch a misleading result.

| Axis | Question it answers |
|---|---|
| R. Real-world fidelity | Does the headline number reflect what a user of the service experiences, and can the reader decompose it? Can realistic workloads (EOS, cache hits, bursts, multi-turn, TLS, proxies, 429s) be run and are they labeled? |
| H. Honesty and anti-gaming | Can a flattering result be produced without a trace in the output? Are defaults conservative? Is every knob that removes real cost stamped into the record? Are published results in this repo labeled correctly? |
| M. Measurement correctness | Are TTFT, ITL, TPOT, E2E, throughput, error rate, goodput, WER, RTFx, and image metrics computed correctly, from one implementation, with the estimator and window stated and recomputable? |
| I. Differentiated value | What does this tool let one of the three personas learn or catch that the reference tools would not? Is each such capability correct and demonstrated (tests, live results)? |
| O. Operator experience | Time from clone to a trustworthy number; behavior under failure; long-run stability; whether the output format is usable without the tool's own docs; whether `--help`, `docs/CLI.md`, and behavior agree. |
| E. Engineering quality | Is the shared core actually shared? Do tests check computed numbers and byte-level parsing? Does CI gate what matters? Is what remains in the binaries justified? |
| P. Public readiness | License, notices, secret hygiene (presence and history, never values), packaging, release automation, version consistency, dependency policy. |
| C. Competitive context | Appendix only. List gaps versus the reference tools, and for each say which persona decision depends on it. Gaps with no dependent decision are listed and scored zero weight. |

## 6. Questions that must be answered explicitly

Answer each in one or two sentences with a finding reference.

1. Where is the TTFT clock started and stopped, what does the interval
   include, and can the reader separate connection setup from server time
   from the same record? Is the headline the user-experienced number, and is
   that stated?
2. What counts as the first token? How are role deltas, reasoning deltas,
   tool-call arguments, and empty content handled, and what happens when
   nothing visible arrives?
3. Where do token counts come from, what happens when the server omits
   `usage`, and if a local tokenizer is available, are both counts shown side
   by side with their disagreement?
4. Is ITL measured per chunk? Is TPOT `N minus 1`? Do stalls inside a stream
   show up anywhere?
5. Which percentile estimator is used, is it the only one in the tree, is
   `n` printed next to every percentile, and is small-sample p99 flagged?
6. What is the throughput window, exactly, in closed loop and in open loop,
   with and without warmup? Is it recorded in the output?
7. In open loop, does headline latency include queue delay from the scheduled
   arrival? Is the cap-engaged condition reported?
8. Is the request sequence reproducible from the seed alone? Prove it with
   two runs and a diff of the dummy server's request log.
9. Which sampling and prompt parameters are sent that the operator did not
   specify, and are all of them in the `config` block?
10. Can the realistic prefix-cache case and the controlled unique-prompt case
    both be run, and does the record say which one it was?
11. How are failures classified, which denominators do error rate and request
    rate use, and are failed requests kept out of latency distributions?
12. What happens on SIGINT and SIGKILL, and what does a consumer of the JSONL
    have to do to recover a partial run?
13. Are per-endpoint distributions complete, and is the pooled block marked as
    a mixture?
14. Is the NTP check opt-in, and when enabled, is the offset recorded rather
    than used as a gate?
15. ASR: which normalizer is applied by default, is the choice recorded, is
    RTFx the client-side definition, and are server and client inference
    times separate fields?
16. VLM: are image bytes sent unchanged by default, is preprocessing outside
    the measured window and reported, and is streaming TTFT measured?
17. Imagegen: monotonic clock, sub-millisecond resolution, warmup exclusion,
    artifact hashing?
18. Strategic runner: is the reported knee the true knee on a server with a
    known saturation point? Do validity rate and goodput agree with a hand
    count? Does the MLPerf export label itself as non-audited?
19. Are the numbers in `docs/SMOKE_RESULTS.md` traceable, unit-labeled,
    reliability-flagged, and honestly separated between real and
    dummy-certified rows?
20. What would each of the three personas learn from this tool that the
    reference tools would not tell them, and is each of those things
    verified?
21. Does anything in the tree, tracked or untracked, contain a live credential,
    and is every such file covered by `.gitignore` and absent from history?
    Report presence only.

## 7. Report format

Write the report as a single Markdown file with the Metrum AI copyright header.
Sections, in order:

1. **Executive summary.** GO or NO-GO for public use and for each persona a
   one-paragraph verdict: would you use it, for what decision, with what
   caveat. Then the three most important findings and the three things the
   tool does that are genuinely valuable, each with evidence.
2. **Scorecard** per section 5, one line of justification per axis.
3. **Direct answers** to the section 6 questions.
4. **Findings**, ordered by severity, using this template:
   - ID and title.
   - Severity: CRITICAL (a persona would publish or act on a wrong number, or
     a legal or security block), HIGH (a persona is materially misled unless
     they read the source), MEDIUM (a correctness or maintainability risk a
     reviewer will notice), LOW (polish).
   - Status: VERIFIED (you ran it and observed it), SUSPECTED (code read,
     not exercised), UNDETERMINED (state what would be needed).
   - Location: `file:line` at the recorded commit.
   - What the code does, quoted.
   - Why it matters to which persona, in real-world terms.
   - Reproduction: the exact command and observed output.
   - Fix: the smallest change that makes the output honest, then the change
     that makes it right. Do not propose a fix whose only effect is a more
     flattering number.
   - Effort: S, M, L.
   - Blocks public use: yes or no, with the persona affected.
5. **Regression table** per section 4.6.
6. **Published-results audit** per section 4.7.
7. **Differentiated value**, one subsection per capability that no reference
   tool offers, each with: what decision it enables, whether it is correct,
   whether it is demonstrated, and what would make it trustworthy if it is
   not.
8. **Roadmap**, ordered by real-world value per engineer-week, not by parity.
   Each item names the persona and the decision it improves. Parity items
   with no dependent decision go in a final "not recommended" list with the
   reason.
9. **Competitive context**, as an appendix table per axis C.
10. **Appendix**: every command run with exit code and trimmed output, the
    recomputation script, the adversarial server sources, the derived
    expected values from the dummy server, and the raw numbers behind every
    figure quoted in the body.

Writing rules: cite line numbers at the recorded commit; quote code rather
than paraphrase it; every number in the body appears in the appendix; label
anything not reproduced; never include a secret value; do not pad with
findings that would not change a decision for one of the three personas; do
not credit or penalize the tool for matching or missing a reference tool's
feature unless a decision depends on it.
