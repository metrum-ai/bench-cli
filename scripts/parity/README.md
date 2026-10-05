<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Parity count harness

Metrum AI Bench CLI vs NVIDIA AIPerf: how many data points each tool reports
for the same workload. This harness backs the counts in epic
[#184](https://github.com/metrum-ai/bench-cli/issues/184). Metric issues
there quote its before and after numbers. It counts outputs. It does not
measure performance, so never publish its latencies.

## One command

```bash
scripts/parity/run_pair.sh live-results/parity      # plain, reasoning, slo
scripts/parity/run_tele.sh live-results/parity      # optional: telemetry ingest
```

`run_pair.sh` prints the scenario table in the epic format and saves it as
`OUT/table.md`:

```text
| Scenario   | AIPerf quantities / values / per-request | Bench quantities / values / per-request |
|------------|------------------------------------------|-----------------------------------------|
| plain      | 65 / 653 / 33                            | ...                                     |
| reasoning  | 70 / 700 / 36                            | ...                                     |
| slo        | 68 / 656 / 34                            | ...                                     |
```

The AIPerf column above is the harness output for AIPerf 0.13.0 at the
defaults, and it matches the 2026-10-05 epic table. A second table lists
distributions, nested blocks, duplicate quantities, measured requests, and
the median TTFT/E2E ratio per run.

Both scripts share `OUT/counts.jsonl`, so they can run in either order.
`run_pair.sh` replaces only the client-scenario rows, `run_tele.sh` replaces
only the `telemetry` rows, and each prints every table present.

Every run uses the same prompts, concurrency 4, and 64 measured requests plus
4 warmup. A fresh mock is started for each tool, so `/metrics` starts at zero.

## Requirements

- `python3` (standard library only for the harness itself) and `curl`.
- Bench binaries, found the same way the live scripts find them
  (`scripts/live/lib/bench_bin.sh`). The order is `BENCH_BIN_DIR`, then
  `../bench-cli-rust/target/release` (the shared Rust lane build), then
  `bin/`, `target/release`, `target/rel-user/release`, then `PATH`. The
  harness never builds. `run_pair.sh` needs `metrum-ai-bench-cli-llm`, and
  `run_tele.sh` needs `metrum-ai-bench-cli-strategic`. Each run logs the
  binary path, `--version`, and checkout into `OUT/sut.json` `notes`.
- AIPerf 0.13.0. The harness uses `AIPERF=/path/to/aiperf`, else `aiperf` on
  `PATH`. With `PARITY_INSTALL_AIPERF=1` it pip installs `aiperf==0.13.0` into
  `OUT/.aiperf-venv`. Upstream docs for the pieces used here (v0.13.0):
  [CLI options](https://github.com/ai-dynamo/aiperf/blob/v0.13.0/docs/cli-options.md),
  [`single_turn` custom dataset](https://github.com/ai-dynamo/aiperf/blob/v0.13.0/docs/tutorials/custom-dataset.md),
  [`--goodput`](https://github.com/ai-dynamo/aiperf/blob/v0.13.0/docs/tutorials/goodput.md),
  [`--server-metrics`](https://github.com/ai-dynamo/aiperf/blob/v0.13.0/docs/server-metrics/server-metrics.md)
  ([JSON schema](https://github.com/ai-dynamo/aiperf/blob/v0.13.0/docs/server-metrics/server-metrics-json-schema.md)),
  [profile exports](https://github.com/ai-dynamo/aiperf/blob/v0.13.0/docs/tutorials/working-with-profile-exports.md)
  ([JSON export schema](https://github.com/ai-dynamo/aiperf/blob/v0.13.0/docs/reference/json-export-schema.md)).
- AIPerf tokenizes client-side, as in the epic run, with
  `AIPERF_TOKENIZER=gpt2` by default. It needs Hugging Face Hub access or a
  local cache. The mock's words are single GPT-2 tokens, so server usage
  and client counts agree, and the `usage_*_diff_pct` metrics read 0.

## Pacing rule

`mock_server.py` streams each token as its own flushed HTTP chunk
(`TCP_NODELAY`, chunked transfer, absolute deadlines):

```text
first token  = PREFILL_MS + PER_PROMPT_TOKEN_MS * prompt_tokens + ITL_MS
token i      = first token + (i - 1) * ITL_MS
output       = 60-100% of max_tokens (or max_completion_tokens)
```

- The defaults are `PREFILL_MS=40`, `PER_PROMPT_TOKEN_MS=0.05` and
  `ITL_MS=8`, which gives TTFT/E2E of about 0.1 to 0.2 at
  `MAX_TOKENS=64`.
- The output length comes from an RNG seeded by `MOCK_SEED`, the prompt
  text and the cap. Both tools therefore see the same length for the same
  prompt.
- Usage arrives in a final chunk with `choices: []`. Prompt tokens are a
  whitespace word count.
- The `reasoning` scenario sends the first 30% of tokens as
  `delta.reasoning_content` and reports `completion_tokens_details.reasoning_tokens`.
- `GET /metrics` serves a vLLM-style page with counters, gauges, and exactly
  one histogram, `vllm:e2e_request_latency_seconds`.

### Why a single-chunk mock is invalid

A mock that writes the whole SSE body at once delivers the first token and the
last token in the same read. TTFT then equals E2E, and ITL, TPOT, decode time,
and per-user decode rates collapse to zero or noise. Both tools still fill in
every field, so a count against such a mock looks complete while every
streaming number is meaningless. The Rust `metrum-ai-bench-cli-mock-server`
works this way: it builds one response string. So never count against it.

`count_points.py` enforces the rule, and it fails closed. Every measured
success must carry a TTFT/E2E pair (Bench `ttft_s` and `latency_s`, AIPerf
`time_to_first_token` and `request_latency` in `profile_export.jsonl`).
Zero pairs (for example no `--streaming`, or a renamed field), fewer pairs
than successes, or a median TTFT/E2E above 0.9 (`--max-ttft-ratio`) all exit
3 with the reason, and `run_pair.sh` stops. A missing `profile_export.jsonl`
is an error, not a skip. `mock_server.py` also refuses `--itl-ms 0`.

The launcher also refuses to measure a stale server. Before starting the
mock or the fork page it checks that the port has no listener. The mock
echoes a per-start nonce on `/health`, and the wait succeeds only for that
nonce while the new PID is alive.

## Scenarios

| Scenario    | Mock          | Bench flags                                   | AIPerf flags |
|-------------|---------------|-----------------------------------------------|--------------|
| `plain`     | default       | `--mode chat --streaming`                     | `--endpoint-type chat --streaming` |
| `reasoning` | `--reasoning` | same                                          | same |
| `slo`       | default       | `--slo ttft=0.2 --slo e2e=2 --price-per-hour 2.0` | `--goodput "time_to_first_token:200 request_latency:2000"` |

AIPerf has no cost metric, so the `slo` row compares goodput plus Bench cost.
The thresholds come from `SLO_TTFT_S`, `SLO_E2E_S` and `PRICE_PER_HOUR`.

## Counting rules (`count_points.py`)

| Column      | AIPerf (`profile_export_aiperf.json`, `profile_export.jsonl`) | Bench (`--data-log` JSONL) |
|-------------|------------------------------------------|----------------------------|
| quantities  | each top-level block with a `unit` | every `DistSummary` (a dict with `n`, `p50` and `p99`) at any depth, named by its dotted path; each top-level numeric scalar; each top-level block whose numeric leaves outside any `DistSummary` are non-empty (`goodput`, `observed_concurrency`) |
| values      | numeric leaves of those blocks | numeric leaves of those quantities |
| per-request | metric names on `profiling` records | non-null numeric fields on `phase=measure` `request.v3` records, except `seq` and `error` |
| duplicates  | quantities whose numbers equal an earlier quantity's | same |

A few details matter for before and after counts:

- **Nested distributions.** A block that holds several distributions counts
  each one as its own quantity, for example `a.b` for a `DistSummary` at
  `summary["a"]["b"]`. The block itself counts once more only if numeric
  leaves remain outside those distributions. Leaves of nested
  non-distribution dicts (for example `goodput.thresholds_s`) join their
  top-level block.
- **Dotted names.** Per-request dicts flatten to dotted names
  (`modality_metrics.prompt_words`), and each name is one field. A list of
  numbers (`itl_s`) counts once.
- **Duplicates.** Numbers are rounded to 12 decimals before comparing, and
  quantities with a single number are never compared, because lone scalars
  collide on 0 or 1 by chance. Duplicates still count as quantities; the
  column shows how many are copies.
- **Errors.** Data whose presence depends on the error rate is left out of
  the headline counts and reported as `error_quantities`: Bench
  `errors_by_type` and per-request `error`, AIPerf `error_summary` and
  top-level `error_*` blocks. A failed request therefore does not change the
  counts. Always-present rate fields (`errors`, `error_rate`,
  `request_error_rate`) still count.

Some things never count: booleans, strings, nulls, warmup data, run metadata
(AIPerf `input_config`/`run_info`, Bench `config`/`environment`/`sut`), and
Bench `per_endpoint`. The last is a per-endpoint copy of the headline
distributions and is reported separately as `per_endpoint_quantities`. The
Bench summary is picked by `schema_version`, not by line position.

An AIPerf full distribution carries 15 numbers (avg, p1 to p99, min, max,
std, count, sum). Its time-weighted blocks (`effective_*`, `active_*`,
`tokens_in_flight`) carry 8. A Bench `DistSummary` carries 10.

Count a single run by hand:

```bash
python3 scripts/parity/count_points.py bench  OUT/plain/bench.jsonl --full
python3 scripts/parity/count_points.py aiperf OUT/plain/aiperf --full
python3 scripts/parity/count_points.py table  OUT/counts.jsonl
```

## Telemetry (`run_tele.sh`, `fork_page.py`)

`fork_page.py serve` replays a captured Metrum all-smi fork `/metrics` page
on `FORK_PORT` (default 19090). Gauges jitter within bounds, counters grow,
`*_info` stays fixed, and `/metric` returns 404 as on v0.26.3-metrum.4. The
default page, `fixtures/all-smi-fork-h100.prom`, is a redacted capture from a
Shadeform H100 PCIe host with 73 `all_smi_*` names, 46 of them
`all_smi_gpu_*`. The series set is whatever the page holds. To use another
host's page:

```bash
python3 scripts/parity/fork_page.py capture http://GPU_HOST:9090/metrics my-page.prom
FORK_PAGE=my-page.prom scripts/parity/run_tele.sh live-results/parity
```

`capture` replaces `gpu_uuid`, `hostname`, `instance` and `host` with
placeholders.

`run_tele.sh` runs two tools:

- `metrum-ai-bench-cli-strategic`, with a generated `--telemetry` YAML that
  includes every `^all_smi_` and `^vllm:` series, plus `--ndjson`.
- `aiperf profile --server-metrics <fork page>`. AIPerf also scrapes the
  mock's own `/metrics` by default.

It then counts both:

| Tool   | values | series | names |
|--------|--------|--------|-------|
| Bench  | raw `telemetry` rows on the request clock | distinct (source, metric, labels) | per source |
| AIPerf | numeric `stats` and `buckets` leaves per series, profiling phase | series in `server_metrics_export.json` | per endpoint URL |

## Knobs

| Env | Default | Meaning |
|-----|---------|---------|
| `TOOLS` | `bench aiperf` | Run one side only (for example `TOOLS=aiperf`). |
| `CONCURRENCY`, `REQUESTS`, `WARMUP` | `4`, `64`, `4` | Load shape. Bench gets `--num-requests REQUESTS+WARMUP --warmup-requests WARMUP`. |
| `MAX_TOKENS` | `64` | Output cap for both tools. |
| `PROMPTS` | generated | JSONL with `prompt`, for example from `metrum-ai-bench-cli-prompts`. |
| `MOCK_PORT`, `FORK_PORT` | `18080`, `19090` | Local ports. |
| `PREFILL_MS`, `PER_PROMPT_TOKEN_MS`, `ITL_MS`, `MOCK_SEED` | `40`, `0.05`, `8`, `0` | Pacing. |
| `AIPERF`, `AIPERF_TOKENIZER`, `PARITY_INSTALL_AIPERF` | unset, `gpt2`, `0` | AIPerf location and tokenizer. |
| `SCRAPE_MS` | `500` | Bench telemetry scrape interval. |

Outputs go under `OUT/<scenario>/`: tool logs, `bench.jsonl`, `aiperf/`,
`count-*.json`, and `OUT/counts.jsonl` plus `OUT/table*.md`. The default
`OUT` is under `live-results/`, which is gitignored.
