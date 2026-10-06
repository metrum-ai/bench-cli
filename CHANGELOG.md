# Changelog

## Unreleased

### Documentation
- Telemetry docs state that Prometheus series are selected at runtime from
  `--telemetry` YAML. The binary does not embed a metric catalog. Agents curl
  the live `/metrics` page (all-smi, the serving engine, and any other
  exporter) and set `include` from that response.
- `docs/TELEMETRY.md` compares that open scrape with AIPerf. AIPerf's metrics
  reference is a named client catalog. Its `--server-metrics` (on by default)
  ingests any Prometheus page, including the Metrum all-smi fork, and it also
  exports raw time-stamped scrapes (Parquet by default, opt-in JSONL, and
  per-record GPU telemetry JSONL). Its power-efficiency family (avg only)
  comes from `--gpu-telemetry` alone (DCGM, pynvml, amdsmi). Bench CLI writes
  the series matched by the YAML `include` into one NDJSON next to the
  per-request JSONL, so GPU series from any exporter can be correlated per
  request. The publish-20261002 campaign is the example (all-smi and the
  serving engine in one NDJSON per cell).
- Remaining all-smi `v0.26.3-metrum.3` references in
  `docs/TELEMETRY.md`, `docs/telemetry/exporters.md`,
  `docs/telemetry/examples/all-smi.yaml`, and the `ALL_SMI_RELEASE` default
  in `scripts/e2e/sut-setup.sh` now point to `v0.26.3-metrum.4`.
- Example telemetry YAML `include` defaults now cover more of the live
  pages (#198). `docs/telemetry/examples/vllm.yaml` adds the TTFT, ITL, E2E,
  queue, prefill, and decode histograms (`_bucket` / `_sum` / `_count`) and
  the prefix-cache counters. `docs/telemetry/examples/all-smi.yaml` adds
  chassis, energy, and NVLink topology series, drops per-core
  `all_smi_cpu_core_utilization` through a CPU allowlist, and embeds the same
  vLLM source as `vllm.yaml`. `all_smi_process_*` stays a commented-out
  opt-in: those rows carry `user` and `command` labels and must not be used
  in published runs. Matched series go from 82 to 66 on a recorded all-smi
  metrum.4 page and from 6 to 154 on a synthetic vLLM v0.30.0 page.
- Comparison hygiene (#201). `docs/reviews/QUALITY_ASSESSMENT_PROMPT.md` now
  points reviewers at AIPerf and `scripts/parity/README.md` instead of the
  retired GenAI-Perf and the deleted `docs/COMPARISON.md`.
  `docs/reviews/QUALITY_ASSESSMENT_REPORT.md` gains a header pointer to epic
  #184 for current comparison work, and `artifacts/e2e/COMPARISON_AIPERF.md`
  is marked as dated history. No metric or schema change.
- New design note `docs/design/AIPERF_INGEST.md` (#205), indexed from the new
  `docs/design/README.md`. It maps AIPerf 0.11.0 and 0.13.0
  `profile_export.jsonl` records to `request.v3` and `summary.v3` (including
  the #191 distributions) and lists gaps in both directions. The importer
  does not copy AIPerf's `cli_command`. It recommends a narrow
  `metrum-ai-bench-cli import aiperf` after #192 to #195, with telemetry
  joins after #196. Design only; no behavior change.

### Added
- `scripts/parity/`: a data-point count harness for Metrum AI Bench CLI vs
  NVIDIA AIPerf (#204). One command, `scripts/parity/run_pair.sh OUT`, runs
  the `plain`, `reasoning`, and `slo` scenarios against a fresh paced mock
  (`mock_server.py`, one flushed SSE chunk per token, vLLM-style `/metrics`
  with one histogram) and prints quantities / values / per-request in the
  epic format. Its AIPerf 0.13.0 column reproduces the epic #184 table
  (65/653/33, 70/700/36, 68/656/34). It counts outputs and does not measure
  performance. Optional `run_tele.sh` counts telemetry ingest against a
  replayed Metrum all-smi fork `/metrics` page (`fork_page.py`,
  `fixtures/all-smi-fork-h100.prom`). See `scripts/parity/README.md`.
- `scripts/parity/count_points.py` refuses a run whose median TTFT/E2E over
  measured requests is above 0.9 (`--max-ttft-ratio`) and exits 3, because a
  single-chunk mock (such as `metrum-ai-bench-cli-mock-server`) collapses
  TTFT into E2E and makes every streaming field meaningless.
- `summary.v3` (additive) summarizes per-request fields that were recorded
  but not aggregated: `first_byte_s`, `queue_delay_s`, `first_reasoning_s`,
  `isl_tokens`, and `osl_tokens` as type-7 `DistSummary` blocks, plus optional
  `isl_tokens_source` / `osl_tokens_source` (`server_usage`,
  `tokenizer_fallback`, or `mixed`; omitted when there are no samples). Each
  `per_endpoint` entry carries the same seven fields. Only measured successes
  count. `queue_delay_s` covers only open-loop (`--request-rate`) requests and
  is `n=0` in closed loop. ISL/OSL prefer server usage, use tokenizer counts
  only for `usage_missing` rows, and skip rows with no usage (ASR, imagegen)
  rather than counting them as zero (#191).
- Strategic sweep points JSON gains `first_byte_s`, `queue_delay_s`,
  `first_reasoning_s`, `isl_tokens`, and `osl_tokens` distributions.
  `queue_delay_s` is `n=0` for `--sweep-by concurrency` stages; ISL/OSL use
  server usage only and skip rows with zero input and output tokens;
  `osl_tokens` is `n=0` for embeddings and rerank stages, and rerank
  `isl_tokens` is `usage.total_tokens` (all input). The
  strategic request CSV gains a trailing optional `first_reasoning_s` column
  (#191).
- Console summary prints First byte, Queue delay, First reasoning, ISL tokens
  (source), and OSL tokens (source) lines when the distribution has samples
  (#191).
- Reasoning token counts (#192, additive, no schema version bump). LLM and
  VLM read the server-reported count from `usage` (final usage chunk when
  streaming, response `usage` otherwise), accepting
  `completion_tokens_details.reasoning_tokens`,
  `output_tokens_details.reasoning_tokens`, and flat `reasoning_tokens`; the
  first non-zero value wins, and `0` only when every reported location is
  `0`. vLLM reports it from v0.28.0 when started with `--reasoning-parser`.
  Unreported, `null`, negative, or non-integer values stay `null`
  (never `0`); there is no tokenizer fallback. `request.v3` gains
  `reasoning_tokens` and `visible_completion_tokens`
  (`completion_tokens - reasoning_tokens`, `null` when reasoning is
  unreported or exceeds `completion_tokens`), always serialized and always
  `null` for ASR and imagegen.
- `summary.v3` gains `reasoning_tokens` and `visible_completion_tokens`
  type-7 distributions plus `reasoning_tokens_total` and
  `visible_completion_tokens_total` (`null` when `n=0`); each `per_endpoint`
  entry gains the two distributions. The console prints Reasoning tokens
  (with total) and Visible completion tokens lines when `n > 0`.
  `completion_tokens`, OSL, completion tok/s, and cost per million output
  tokens still count reasoning as output (#192).
- Strategic sweep points gain `reasoning_tokens`, `reasoning_tokens_total`,
  `visible_completion_tokens`, and `visible_completion_tokens_total` (chat
  stages only); the request CSV gains a
  trailing optional `reasoning_tokens` column after `first_reasoning_s`
  (older CSVs still load); `telemetry.v1` request rows gain an optional
  `reasoning_tokens`, omitted when not reported (#192).
- `dummy-model-server` `-reasoning-tokens N` streams N `reasoning_content`
  chunks and reports them in `usage.completion_tokens_details.reasoning_tokens`
  (also added to `completion_tokens` / `total_tokens`), streaming and
  non-streaming. The default `0` leaves the payload unchanged (#192).
- Token totals, rates, and per-user latency fields (#193, additive, no
  schema version bump). `summary.v3` gains top-level (not `per_endpoint`)
  `prompt_tokens_total` and `completion_tokens_total` (server usage wins,
  tokenizer fills `usage_missing` rows, same accounting as
  `completion_tokens_per_second`; `null` when no row reports tokens or a
  `usage_missing` row lacks a tokenizer count),
  `input_tokens_per_second = prompt_tokens_total / window_seconds`, and
  `total_tokens_per_second = (prompt_tokens_total + completion_tokens_total) / window_seconds`
  (`null` unless both totals exist). It also gains type-7 distributions
  `prefill_tps_per_user` (`isl_tokens / min(first_reasoning_s, ttft_s)`, the
  first generated token of any kind so a thinking model's reasoning phase is
  not counted as prefill; rows with ISL > 0 and visible-token TTFT > 0;
  first-byte approximations excluded; TTFT includes connect and queueing), `time_to_second_token_s`
  (`ttft_s + itl_s[0]`, time to the second content chunk), and `user_tps`
  (`completion_tokens / latency_s`, the `user_tps=` SLO definition). The
  console prints the new lines when they have values.
- Strategic sweep points gain `prompt_tokens_total`,
  `completion_tokens_total`, `input_tokens_per_second`,
  `total_tokens_per_second`, `prefill_tps_per_user`
  (`input_tokens / min(first_reasoning_s, ttft_s)`), and `time_to_second_token_s`, from server usage
  only. A total is `null` when no success reported that field, and
  `completion_tokens_total` and `total_tokens_per_second` are `null` for
  embeddings and rerank stages (#193).
- Time-weighted concurrency and throughput (#195, additive, no schema
  version bump). `summary.v3` (top level, not `per_endpoint`) and every
  strategic sweep point gain `effective_concurrency`,
  `effective_prefill_concurrency`, `effective_decode_concurrency`,
  `tokens_in_flight`, `effective_prefill_throughput`, and
  `effective_decode_throughput`, each `{n, avg, active_avg, max, active_s}`
  from a sweep line over measured-success intervals clipped to the window
  (`avg` over `window_seconds`, `active_avg` over time with a request open;
  `null` when `n=0`, never 0-filled). The phase split is the first generated
  token, `min(first_reasoning_s, ttft_s)`; rows with first-byte approximated
  or no TTFT count only in `effective_concurrency`. `effective_concurrency`
  follows Little's law and is distinct from the unchanged
  `observed_concurrency`; `tokens_in_flight` is a KV-cache occupancy proxy;
  when every success streams with a positive decode time,
  `effective_decode_throughput.avg` matches `completion_tokens_per_second`. Strategic uses `service_latency_s` (no
  client queue delay) and server usage only. Matches AIPerf effective and
  active metrics. The console prints one line per block with `n > 0`.
- New launcher subcommand `sut` prints the SUT JSON to stdout without docker
  or a GPU. `HF_HUB_OFFLINE=1` skips the Hub revision lookup
  (`model.revision` is then null). The offline self-test
  `scripts/tests/serve_sut_test.sh` runs in CI and checks
  `model.quantization`, `notes`, and the `MODEL` override guard. No Rust or
  summary/request schema change (#203).
- HTTP phase trace (#194, additive, no schema version bump). `request.v3`
  gains optional `connection_reused` (no connector call finished
  before the response headers, so a pooled connection carried it), `dns_s` (resolver time inside `connect_s`, `0.0` on a pool hit
  or IP-literal host), `bytes_sent` (request body bytes), and, for successes
  only, `receive_s` (headers to last body chunk), `bytes_received` (body bytes
  after content decoding), and `chunks_received` (HTTP client body chunks,
  not SSE events). Each is omitted when absent. TCP connect and TLS
  handshake are not split because reqwest runs both in one connector future;
  `connect_s - dns_s` is their sum (see `docs/METRICS.md`).
- `summary.v3` and strategic sweep points gain `dns_s`, `receive_s`,
  `bytes_sent`, `bytes_received`, and `chunks_received` type-7 distributions
  (`n=0` when none) plus `connections_reused` and `connection_reuse_rate`
  (`null` when no measured success carries `connection_reused`). The console
  prints Receive, DNS, Bytes sent/received, Chunks received, and Connections
  reused lines when present. The strategic request CSV gains trailing
  optional `connection_reused`, `dns_s`, `bytes_sent`, `receive_s`,
  `bytes_received`, and `chunks_received` columns after `reasoning_tokens`
  (#194).
- `docs/queries/analyze.py` computes the `docs/telemetry/ANALYSIS.md`
  derived metrics per measured stage: `gpu_util_mean`, `sm_active_p50`,
  `sm_occupancy_p50`, `tensor_active_p50`, `hollow_util_mean`,
  `kv_cache_util_mean` (ratios 0 to 1), and `preemptions_delta`. GPU sources
  are the Metrum all-smi fork gauges (`all_smi_gpu_utilization`,
  `all_smi_gpu_sm_active_ratio`, `all_smi_gpu_sm_occupancy`,
  `all_smi_gpu_tensor_active_ratio`,
  `all_smi_gpu_hollow_utilization_ratio`) with DCGM PROF fallbacks
  (`DCGM_FI_PROF_SM_ACTIVE`, `DCGM_FI_PROF_SM_OCCUPANCY`,
  `DCGM_FI_PROF_PIPE_TENSOR_ACTIVE`, and `DCGM_FI_PROF_GR_ENGINE_ACTIVE`
  minus `DCGM_FI_PROF_SM_ACTIVE` for hollow). A metric with no source series
  is null, never 0. Engine histogram p50/p95 come from bucket deltas with
  Prometheus `histogram_quantile` interpolation; a label set is dropped
  whole if any of its buckets resets in the stage. An optional second
  argument (the strategic stdout JSON) adds `kv_cache_util_at_knee`, read
  from `knee_detection` (#190) and null with a reason when there is no knee
  (`knee_index_out_of_range` when `knee_detection.index` is outside the
  points). Older outputs fall back to the legacy `knee` field, ignored when
  fewer than 5 points have a `p95_s` (matching #190).
  `--json` prints machine-readable output (#199).
- Recorded fixtures `docs/queries/fixtures/sweep5` and `sweep3` (`.ndjson`
  plus trimmed `.stdout.json`, from strategic against
  `metrum-ai-bench-cli-mock-server --telemetry-fixture`, with a build that
  includes #190 so the stdout carries `knee_detection`; `record.sh`
  re-records them) and a stdlib `unittest` suite
  `docs/queries/test_analyze.py`, now run in CI. The mock energy counter
  advances by a fixed step per scrape, so J/token from these fixtures is not
  meaningful (see `docs/telemetry/ANALYSIS.md`). No Rust or schema change
  (#199).
- Strategic `--kind vlm`, `--kind asr`, and `--kind imagegen` sweeps (#197,
  additive, schema stays `metrum-ai-bench-cli.strategic.v1`). Requests use
  the modality binaries' builders, now in the library (`src/vlm.rs`,
  `src/asr.rs`, `src/imagegen.rs`); modality binary output is unchanged. New
  flags: `--temperature` (chat and vlm; omitted from chat bodies when unset,
  vlm default 0.1), `--image` (repeatable), `--image-detail`,
  `--max-image-dimension`, `--audio-samples`, `--ground-truth`,
  `--asr-response-format`, `--language`, `--normalizer`, `--image-size`,
  `--images-per-request`, and `--image-response-format`. `--extra-body-json`
  now also works for vlm and imagegen, and `--ignore-eos` / `--min-tokens`
  for vlm. ASR audio and VLM images load before the first request, so file
  I/O is not in latency. For asr and imagegen the sweep clock stops when the
  body is read; parsing, WER/CER and image decode/hash run after it, on the
  blocking pool and after the concurrency slot is released, and an
  undecodable `b64_json` image fails the request. That matches the imagegen
  binary; the ASR binary's clock also covers form building and response
  parsing (see `docs/METRICS.md`). These flag combinations are rejected:
  `--streaming`, `--max-tokens`, `--shared-prefix` and
  `--infer-ttft-from-first-byte` for asr and imagegen; `--shared-prefix`, and
  `--image` together with `--prompts`, for vlm; `--image` and
  `--max-image-dimension` outside vlm; `--audio-samples` and `--ground-truth`
  outside asr; `--prompts` for asr; `--json-schema`, `--tools` and
  `--sessions` for modality kinds. Other kind flags are ignored where they do
  not apply. The imagegen binary still writes each image as it decodes.
- Strategic sweep points gain `modality_metrics` (key to type-7
  distribution over measured successes, same names as modality `request.v3`:
  VLM `image_count`, `image_bytes`; ASR `wer`, `cer`, `rtfx_client`,
  `audio_duration_s`; imagegen `images_requested`, `images_returned`; `n=0`
  when unmeasured, omitted for chat, embeddings, and rerank) and, for
  imagegen only, `image_digests` `{images, distinct}` (0/0 for `url`
  responses). Strategic `config` gains a `modality` object for modality kinds
  and `temperature` when `--temperature` is set. `osl_tokens` and
  `completion_tokens_total` come from usage for vlm and asr and are
  `n=0` / `null` for imagegen; `ttft_s` is `n=0` for asr and imagegen. The
  telemetry NDJSON `run` row `config.kind` can be `vlm`, `asr`, or
  `imagegen`, and its `config` carries the same `modality` and `temperature`
  keys. Request CSV columns and chat, embeddings, and rerank output are
  unchanged (#197).
- Run telemetry for every binary (#196, additive, no schema version bump).
  `metrum-ai-bench-cli-llm`, `-vlm`, `-asr`, and `-imagegen` gain the
  strategic telemetry flags `--ndjson PATH`, `--telemetry YAML`,
  `--require-telemetry`, and `--require-telemetry-failures N` (default 3),
  backed by one shared `TelemetrySession` that strategic now uses too. Same
  YAML schema, same NDJSON row kinds (`run`, `stage`, `request`,
  `telemetry`, `scrape_error`, `summary`), same
  `metrum-ai-bench-cli.telemetry.v1`, and one monotonic epoch shared by
  request and telemetry rows. Each `request.v3` record becomes a `request`
  row (`t_sent_ns` equals `send_offset_s` in nanoseconds: with `--ndjson`
  the run clock starts at the NDJSON epoch, after the telemetry probes, so
  data-log offsets and `*_ns` fields share one origin; `t_done_ns = t_sent_ns +
  latency_s`, `service_latency_s` is the record's `latency_s`, row
  `latency_s` adds `queue_delay_s`); `warmup` and `measure` `stage` rows
  span the phase's first send to last completion and can overlap at
  concurrency above 1. See `docs/TELEMETRY.md` "Modality binaries" and
  `docs/OUTPUT_SCHEMA.md`.
- `summary.v3` gains an optional top-level `telemetry` object, present only
  with `--ndjson`: `schema_version`, `ndjson` (file name only), `sources` (configured
  source count, `0` without `--telemetry`), and integer `request_rows`,
  `stage_rows`, `telemetry_rows`, `scrape_error_rows`, and
  `dropped_telemetry_rows`. Omitted otherwise. Strategic sweep points do not
  gain it; strategic stdout already reports `ndjson` and
  `dropped_telemetry_rows` per run (#196).
- `docs/DATA_POINTS.md`: published data-point counts per release (#202),
  generated from the serde schemas by `tests/data_points.rs` and
  `scripts/render_data_points.sh`. It lists `summary.v3` quantities,
  distributions, blocks, and value slots, the 10-number distribution width,
  `request.v3` numeric fields, strategic sweep-point quantities, and
  `telemetry.v1` `request` row fields, with every optional field and the
  condition that fires it (streaming, reasoning, open loop, SLO, price,
  ISL/OSL targets, tokenizer, `--ndjson`). Telemetry series are selected by YAML `include`
  and get no fixed count. A real llm run against dummy-model-server checks
  that the fired fields equal the documented set (plain run: 47 summary
  quantities and 21 per-request fields, as in the `scripts/parity/` harness).
  CI fails when the file is stale; the release workflow checks it on the
  tag and adds it to the GitHub release notes after the generated changes
  (`scripts/release_notes_data_points.sh`). No schema change.

### Changed
- VLM, ASR, and imagegen now record `connect_s` and the HTTP phase trace
  (before, only LLM and strategic recorded `connect_s`). VLM `prefill_s` is
  now `ttft_s - connect_s`, as for LLM, so it can read lower than in earlier
  VLM runs. Imagegen with retries records the last attempt. ASR
  `modality_metrics.bytes_sent` / `bytes_received` (audio file bytes and
  response text length) are unchanged and differ from the new top-level wire
  body byte fields (#194).
- Strategic knee detection now needs at least 5 measured stages, that is
  stages with a p95 (`KNEE_MIN_POINTS`: both endpoints plus 3 interior
  candidates). Stages without a p95 do not count, so interior gaps cannot
  leave a single candidate that is always returned. With 3 stages Kneedle has one interior candidate and
  always returned the middle stage, so 3- and 4-stage sweeps that used to
  report a knee now report `"knee": null`. Plan sweeps with 5 or more loads
  when the knee matters (#190).
- Strategic stdout JSON (`metrum-ai-bench-cli.strategic.v1`) gains an
  always-present `knee_detection` object: `index` (integer or null),
  `reason` (`insufficient_points`, `missing_latency`, `flat_curve`, or null),
  `points` (measured stages), and `min_points` (5). `reason` is null exactly when `index` is
  set. Additive; `knee` is unchanged in shape. The CLI prints
  `note: no knee: ...` on stderr for runs with 2 or more stages and the HTML report shows the same sentence.
  `scripts/e2e/write_aiperf_comparison.py` prints the reason next to a
  missing knee. See `docs/OUTPUT_SCHEMA.md` and
  `docs/STRATEGIC_BENCHMARKING.md` (#190).
- With `--require-telemetry`, a source that fails N consecutive scrapes
  (`--require-telemetry-failures`, default 3) now also trips the run stop
  flag, so no new requests or stages are issued; in-flight requests finish
  and the binary exits non-zero after writing `summary.v3` and closing the
  NDJSON with `partial: true` (strategic now closes it too). Without the flag
  a scraper exit stays a warning. An NDJSON write failure no longer costs a
  modality run its `summary.v3`. Before, the run kept issuing load and only
  failed at the end. Strategic CLI flags and stdout are otherwise unchanged
  (#196).
- `scripts/live/telemetry_sidecar.py` is deprecated (docstring and a stderr
  notice) but not removed. Pass `--ndjson` and `--telemetry` to the bench
  binary instead; the sidecar stays for binaries built before #196.

### Fixed
- Strategic knee detection no longer reports a knee on a sweep with no
  meaningful bend, and no longer misses a saturated sweep whose p95 stays flat
  (#232). Kneedle always returns the interior stage farthest from the chord,
  so a nearly linear sweep still got a knee. A knee now has two candidates,
  and the earlier stage wins. (1) Kneedle, when the largest p95 rise over the
  running minimum, `max_j (p95_j / min_{i<=j} p95_i - 1)`, is at least 20%
  (`KNEE_MIN_P95_RISE`, provisional from two live curves); Kneedle runs from
  that baseline to that peak, so an inflated cold first stage, a mid-sweep
  bend that recovers, or a last stage below the first cannot hide the bend.
  (2) The stage before the first saturated stage: on concurrency sweeps a
  relative throughput gain under half the relative load gain (on 2x steps a
  throughput ratio below 1.5; one-wave stages skipped), and on any sweep an
  error rate 5 points above the lowest earlier stage. Rate sweeps use only the
  error rate, since their stage window ends at the latest completion (#224).
  This catches load shedding (fast 429/503 keep success p95 flat while
  throughput plateaus), and a later all-failure stage no longer overrides an
  earlier bend. A sweep with neither reports `"knee": null` with the new
  `knee_detection.reason` `no_bend`; `missing_latency` now applies only when
  neither candidate exists. `knee_detection` gains `method` (`kneedle` or
  `saturation`), `p95_rise`, `saturated_index`, and the thresholds
  `min_p95_rise`, `min_marginal_gain` (concurrency sweeps; null on rate
  sweeps) and `max_error_rate_rise` (all additive to
  `metrum-ai-bench-cli.strategic.v1`). Library API: `KneeDetection` no longer
  derives `Eq` (it now has `f64` fields), and `detect_knee_on_axis` with
  `KneeLoadAxis` is new. The stderr note and the HTML report say "no knee: p95
  latency rises less than 20% above its running minimum and no stage is
  saturated". Behavior change: bend-free sweeps used to report a knee and now
  report none; such a sweep usually has not reached saturation, so check error
  rate and throughput scaling and extend the sweep. On the live H100 #184
  validation (vLLM 0.31.0) the Qwen3-VL-8B sweep at c=1..16 (p95 +11%, 2x
  steps gaining 1.87x to 1.97x) moves from a knee at index 1 to `no_bend`, and
  the LLM sweep at c=1..64 (p95 +41%, smallest step gain 1.77x) keeps its knee
  at c=32. A threshold on the normalized chord distance cannot separate them
  (LLM peak 0.095, VLM 0.164). `docs/queries/analyze.py` passes `no_bend`
  through as `kv_cache_util_at_knee_reason` (no code change, new test), and
  `scripts/e2e/write_aiperf_comparison.py` explains `no_bend` next to a
  missing knee. See `docs/OUTPUT_SCHEMA.md` and
  `docs/STRATEGIC_BENCHMARKING.md`.
- The strategic stage window now ends at the latest successful completion
  instead of the completion of the last-started request (#224). It runs from
  the earliest measured send (any outcome) to the latest successful
  completion, or the latest completion of any outcome when the stage has no
  success; warmup is excluded. In `--sweep-by concurrency` sends are stamped
  after the semaphore, which does not release in spawn order, so the
  last-started request often finished well before the stage ended and the
  window was cut short. Strategic stage `throughput`, `goodput`, token rates
  and `cost_per_million_output_tokens` therefore read lower than earlier
  versions: slightly lower at low concurrency; -21% at concurrency 16 in one
  64-request mock run (-2% in another); the old error depended on task
  scheduling and could be larger at other concurrency or request counts. A
  stage whose last-finishing request failed can read slightly higher. The
  time-weighted blocks and `compare` now take this same window, so numbers
  recomputed from the CSV match the live strategic output (before, `compare`
  ended at the latest completion of any outcome). The window is measured on a
  monotonic clock: the strategic CSV gains a trailing `send_offset_s` column
  (seconds from the run start, the NDJSON `t_sent_ns` origin; additive), so an
  NTP step cannot stretch a stage. `compare` falls back to wall-clock
  `sent_unix_ns` only for older CSVs without the column. Compare strategic
  rates across versions with care.
- `--warmup-requests` is now a barrier in `metrum-ai-bench-cli-llm`, `-vlm`,
  `-asr`, and `-imagegen` (`metrum_ai_bench::runner::WarmupBarrier`).
  Measured requests start only after every warmup request completes (success
  or error), so the `warmup` and `measure` NDJSON stage windows no longer
  overlap at concurrency above 1 and measurement no longer shares the server
  with warmup traffic. Open-loop (`--request-rate`) measured schedules shift
  by the barrier wait and keep their seeded inter-arrival gaps, so measured
  `scheduled_offset_s` / `t_sched_ns` carry the shifted value and
  `queue_delay_s` no longer counts warmup time. Strategic already ran a
  per-stage barrier; `tests/e2e_warmup_barrier.rs` now covers it. Runs from
  earlier versions with `--concurrency` above `--warmup-requests` may include
  warmup overlap in early measured latency and `connect_s`, while open-loop
  measured `scheduled_offset_s` / `t_sched_ns` and `throughput_bins_rps` now
  start at the first measured request, so compare across versions with care.
  No schema change (#226).
- Open-loop `throughput_bins_rps` bins start at the first measured
  `scheduled_offset_s` instead of offset 0, so measured requests scheduled
  after warmup no longer fall past the last bin and drop out of the bins.
  Closed loop already anchored at the first measured send (#226).
- `observed_concurrency` (`in_flight_*`, `cap_engagement_fraction`,
  `acquire_count`, `wait_count`) now covers the measured phase only in
  `metrum-ai-bench-cli-llm`, `-vlm`, `-asr`, `-imagegen` and each strategic
  stage. The tracker resets at the warmup barrier, so `acquire_count` equals
  the measured requests dispatched; before, warmup acquires and occupancy
  were included. No schema change (#226).
- `docs/queries/analyze.py` engine histogram p50/p95 no longer invent values
  at the bucket edges (#231). A rank in the first finite bucket used to be
  interpolated from 0, and a rank in `+Inf` used to return the highest finite
  bound. Both now give a null `pN` with `pN_reason` (`below_first_bucket` /
  `above_last_bucket`) and `pN_bound` (that bucket bound) in `--json`
  `engine_histograms`; the text table prints `<=0.3` / `>60`. In-range
  interpolation is unchanged. A live vLLM 0.31.0 run had reported
  `request_prefill_time` p50/p95 = 0.15/0.285 in every stage because every
  value was below the 0.3 s first bound.
- `metrum-ai-bench-cli-asr` no longer records `rtfx_client: 0` for a sample
  whose `duration` is 0; like `audio_duration_s`, it is omitted (no usable
  duration, no real-time factor). Strategic `--kind asr` follows the same
  rule (`asr::rtfx`, #197).
- The telemetry NDJSON `run` row is now always written before any
  `telemetry` sample; scrapers could previously emit samples ahead of it.
  The NDJSON `summary` row now waits (up to 2 s) for queued rows to drain
  instead of a fixed 50 ms sleep, so its row counts are exact (#196).
- Streaming clients (LLM, VLM, strategic chat, preflight) now read the body
  to its end after `data: [DONE]`. They used to stop at `[DONE]` and drop the
  body before the terminating HTTP chunk arrived, so hyper discarded the
  connection instead of returning it to the pool. Against servers that flush
  `[DONE]` and the stream end in separate writes, about half the requests
  (the parity mock at concurrency 4: 35 of 64) opened a fresh TCP/TLS
  connection, and that connect time landed inside their TTFT. The drain is
  bounded (250 ms, 64 KiB) so a server that holds the stream open cannot hang
  the client, and it runs after `latency_s`, TTFT, and ITL are fixed. Earlier
  streaming runs may include connect time in TTFT for many requests and
  should be re-measured before comparing TTFT. `receive_s`,
  `bytes_received`, and `chunks_received` now include the drained tail. No
  schema change (PR #222 review).
- `dns_s` books resolver time only with a connect that completes before the
  response headers, so a losing background connect can no longer leave
  `dns_s > 0` on a row with `connect_s = 0.0` and `connection_reused = true`.
  A lookup cancelled by `connect_timeout` keeps its elapsed time instead of
  reading `0.0`. The response body counter no longer takes the trace lock per
  chunk; it writes chunks, bytes, and body end once (PR #222 review).
- dummy-model-server gains `-done-tail DURATION`, a delay between
  `data: [DONE]` and the end of the stream body, used by the connection-reuse
  e2e test.
- `observed_concurrency.in_flight_max` / `in_flight_mean` / `in_flight_p50`
  and per-request `in_flight_at_send` no longer read `cap + 1`. LLM, ASR, VLM,
  and strategic released the semaphore permit before the in-flight guard, so
  the next request could enter while the previous one still counted. The new
  `metrum_ai_bench::concurrency::InFlightSlot` holds both and, by field drop
  order, always leaves the gauge before freeing the permit, including when a
  request task panics. Imagegen already released in that order. No schema
  change (#189).
- `observed_concurrency` values from 1.5.3 and earlier are biased upward and
  should be re-measured rather than compared directly with new runs. For
  example, publish-20261002T162512Z g5-a1 at cap 1 reported
  `in_flight_max` 2.0.
- `scripts/live/serve/*.sh` SUTs now fill `model.quantization` instead of
  leaving it null for quantized checkpoints. `scripts/live/serve/common.sh`
  takes it from `QUANTIZATION` (`none` records null), else from
  `--quantization` / `-q` in `SERVE_ARGS`, else from a quantizer token in the
  `MODEL` name (for example `-FP8` is `fp8`, `-AWQ` is `awq`, `-GPTQ-Int4` is
  `gptq`, `-W4A16-G128` is `w4a16`), else null. The rightmost method marker
  wins, so `-FP8-to-BF16` is unquantized. `QUANTIZATION` is trimmed, must be
  one word, and treats `none` / `null` in any case as null. New SUT field `extra.quantization_source` records which
  one applied (`env`, `serve_args`, `model_name`, or `none`), and `notes` gains
  a line when the value was derived from the model name (#203).
- Each launcher sets `DEFAULT_MODEL` and `DEFAULT_IMAGE`. When `MODEL`,
  `IMAGE`, or `SERVE_ARGS_OVERRIDE` differs from the launcher default, `start`
  and `sut` exit with an error naming each missing variable,
  `SUT_NOTES_OVERRIDE` (researched notes) and `SOURCES_OVERRIDE` (source
  URLs), so a SUT never carries serving notes or sources researched for a
  different model, image, or flag set. An override equal to the default is
  not an override. `print`, `stop`, `logs`, and the default path are
  unchanged (#203, #216).
- E2E tests no longer reserve a free port, drop it, and then start a server
  on it, which let another process take the port first. The
  `metrum-ai-bench-cli-mock-server` "listening on" line now prints the
  address it actually bound instead of repeating `--listen`, so `--listen
  127.0.0.1:0` picks a free port and reports it. The Go
  `dummy-model-server` binds before serving, `-port 0` picks a free port, and
  the startup line and image URLs use the real port. The e2e helpers in
  `tests/common/mod.rs` start both servers on port 0 and read the port from
  that line, and they now also kill the server that `go run` starts instead
  of orphaning it. No metric or schema change (#211).
- `docs/queries/analyze.py` `counter_delta` no longer returns last minus
  first across a counter reset (any drop between samples), which understated
  or went negative. It now returns null, so `energy_j` falls back to the
  trapezoid over power and `preemptions_delta` is null. The helper
  `percentile_nearest` is renamed `percentile_type7`; it already computed
  Hyndman-Fan type 7 and only the name was wrong, so the rename changes no
  values (#199).
- `docs/queries/analyze.py` no longer doubles GPU power and energy when one
  GPU is scraped by more than one exporter. It summed every matching row, so
  a GPU seen by both all-smi and DCGM read twice the power and energy. It
  now picks one power source and one energy counter per stage, all-smi first
  (`all_smi_gpu_power_consumption_watts`,
  `all_smi_gpu_energy_hw_millijoules_total`), then DCGM
  (`DCGM_FI_DEV_POWER_USAGE`, `DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION`), then
  other GPU series, with the name-based match only when none of those is
  present. Chassis, node, `ipmi`, and `redfish` meters are never counted as
  GPU power. The chosen series are reported as `sources.power` and
  `sources.energy_counter`. The doubling predates the derived metrics (#199).

## 1.5.3 (2026-10-02)

### Documentation
- Engine map corrected. LLM and VLM run on regular vLLM, and ImageGen runs on
  vLLM-Omni. ASR's intended stack is vLLM-Omni, but in vllm-omni v0.30.0
  `--omni` exposes only the generate and speech tasks, so
  `/v1/audio/transcriptions` is unavailable (vllm-omni#5722). Until that
  lands, ASR is served with regular vLLM speech-to-text. The prior live ASR
  PASS (2026-10-02 widen) used regular vLLM; re-validate on vLLM-Omni when
  the fix ships. Updated `docs/SERVING.md`, `docs/ASR.md`, `CLAUDE.md`, and
  `scripts/live/README.md`.
- README "Before you benchmark (agents and operators)". It states that the
  CLI is a client, that every real-backend run starts with a web search of
  vendor docs (workload, framework, model card, engine args, request params,
  sweep), that the Hub prompt library is the LLM default, that prebuilt
  binaries come before building, and where live status lives. Search-first
  notes were added to SERVING, ASR, IMAGEGEN, REASONING_MODELS,
  STRATEGIC_BENCHMARKING, and TELEMETRY.
- TELEMETRY and CLAUDE.md note that all-smi fork v0.26.3-metrum.4 serves
  `/metrics` (`/metric` returns 404).

### Changed
- `scripts/live/serve/asr.sh` takes `ASR_STACK=vllm` (default,
  `vllm/vllm-openai:v0.30.0`) or `ASR_STACK=omni`
  (`vllm/vllm-omni:v0.30.0`, `--omni`, for re-validation once
  vllm-omni#5722 lands). Each stack writes its own sources into the SUT.
- Live scripts resolve prebuilt binaries through the new
  `scripts/live/lib/bench_bin.sh`, in this order:
  `BENCH_BIN_DIR`, release tarball `bin/`, `target/release`,
  `target/rel-user/release`, `PATH`.
  They never compile and never pick debug builds implicitly.
  `local_smoke.sh` records the binary path, `--version`, and checkout in the
  SUT.
- `docs/SERVING.md`: one table mapping each modality to its recommended
  engine (vLLM, SGLang, vLLM multimodal, vLLM speech-to-text, vLLM-Omni),
  example model, `scripts/live/serve/` launcher, upstream docs, and ledger
  status, plus engine notes that change results.
- `docs/IMAGEGEN.md`: new guide for `metrum-ai-bench-cli-imagegen` on
  vLLM-Omni. It covers request knobs and their server defaults (50 steps
  when `num_inference_steps` is omitted versus 9 for Z-Image-Turbo),
  `b64_json` responses, artifacts, and the not-verified-live status.
- `docs/ASR.md`: a Serving frameworks section (vLLM speech-to-text with the
  launcher and upstream links, Whisper `--max-model-len 448`, and other
  `/v1/audio/transcriptions` backends). It links vLLM's audio docs instead of
  listing codecs.
- README and `scripts/live/README.md` link the new guides.
- `docs/CLAIMS_LEDGER.md` separates `Verified-in-code` from
  `Verified-live (<campaign id>, <date>)` and rates live status per
  modality: LLM and VLM verified live in the 2026-10-01 readiness review,
  ASR functional only (no WER yet), image generation not verified live.
- README points to the ledger for live-verification status.
- `docs/ASR.md`: valid-audio requirement, WER/CER from `--ground-truth`, and
  Whisper `--max-model-len 448` on vLLM 0.30.0.
- `docs/LIMITATIONS.md` (and the docs site): what `dummy-model-server`
  does and does not validate, with and without `-strict-media`.
- `docs/RELEASING.md`: a smoke cell with 0 successes blocks a release until
  triaged in an issue.

### Added
- `dummy-model-server -strict-media` rejects media a real server rejects:
  `data:` image URLs that are not base64, do not decode as PNG, JPEG, GIF,
  or WebP, or are smaller than 2x2 return HTTP 400 on chat completions;
  transcription uploads under 1024 bytes, with an unknown container, a
  malformed WAV or MP3 header, or an all-zero body return HTTP 400
  `Invalid or unsupported audio file` (the vLLM error text). The default
  stays permissive. The Rust e2e harness (`tests/common::spawn_dummy`) now
  starts the dummy in strict mode; `spawn_dummy_permissive` opts out.
- e2e tests prove strict mode turns a header-only MP3 (ASR) and a 1x1 PNG
  (VLM) into classified `http_status` 400 records.

- Real media fixtures. `test-data/asr/` has three LibriSpeech `test-clean`
  utterances (CC BY 4.0) as 16 kHz mono WAV, 395 KB in total, with exact
  transcripts in `truth.jsonl` and an `input.jsonl` manifest, so ASR
  quickstarts report WER and CER. `scripts/fetch_asr_fixtures.sh`
  regenerates them byte for byte. `test-data/vlm/shapes-512.png` is a
  512x512 PNG with shapes and a word, written by
  `scripts/gen_vlm_fixture.py`, plus `test-data/vlm/prompts.jsonl`.
- `tests/e2e_fixtures.rs` checks that the shipped fixtures pass
  `-strict-media` and that the negative fixture does not.
- Release archives include `NOTICE` and `THIRD_PARTY_LICENSES`, and the
  release smoke checks that the new fixtures are present.

- Live modality gate. `.github/workflows/live-modality-smoke.yml` runs one
  smoke cell per modality on a real serving stack (runner label `gpu-h100`)
  on `workflow_dispatch` and `v*` tags. `scripts/live/serve/{llm,vlm,asr,imagegen}.sh`
  start vLLM 0.30.0 (Qwen3-8B, Qwen3-VL-8B-Instruct, Whisper large-v3-turbo
  with `--max-model-len 448`) and vllm-omni 0.30.0 (Z-Image-Turbo) and write
  a SUT with the exact launch command and sources.
  `scripts/live/local_smoke.sh --local --modality <m>` (also
  `run_smoke.sh --local`) runs a cell on the same host.
  `scripts/live/assert_headline.sh` fails a cell with 0 successes, a
  success ratio below `MIN_SUCCESS_RATIO`, no `--require-sut`, ASR records
  without WER/CER, VLM records without images, or imagegen without
  decodable artifacts; CI runs its offline self-test.
- `release.yml` job `live-gate` calls the live gate and blocks
  `github-release` and `crates-io` when repository variable
  `LIVE_GATE_REQUIRED` is `true` (off by default until a GPU runner exists).
- `deploy/github-runners-bench-cli`: opt-in `runner-gpu` service (compose
  profile `gpu`, labels from `GPU_LABELS`) with GPU reservation, host
  networking, the host Docker socket, and a shared Hugging Face cache.

### Changed
- LLM live scripts (`matrix_smoke.sh`, `campaign.sh`, `shadeform.sh run-llm`,
  `run_smoke.sh`) extract prompts with `metrum-ai-bench-cli-prompts` through
  `scripts/live/lib/hub_prompts.sh` instead of writing handmade JSONL.
  Defaults are `metrum-ai/prompt-library`, config `sample`, profile
  `chat-short`; `PROMPT_*` variables or a local JSONL/parquet override them.
  The mix's dataset, revision SHA, profile, and row count are stamped into
  the SUT.
- Live VLM cells use `test-data/vlm/shapes-512.png` instead of a 2x2 PNG.
- `run_smoke.sh` polls `/v1/models` with curl; it previously required a
  `wait_for_vllm` binary that the project never shipped.
- `test-data/dummy.mp3` is now `test-data/negative/header-only-invalid.mp3`.
  It was never audio (an MPEG frame header plus 100 zero bytes) and every
  real server rejects it. README, `docs/ASR.md`, and the docs site examples
  use `test-data/asr/` and `test-data/vlm/` instead.
- `scripts/live/matrix_smoke.sh` ASR cells upload the LibriSpeech clips with
  `--ground-truth`, and the manifest uses `duration` (the old `duration_s`
  key was ignored by `metrum-ai-bench-cli-asr`).
- `tests/e2e_asr.rs` uploads a generated 16 kHz mono PCM WAV sine tone
  instead of a 30-byte fake MP3.

### Fixed
- VLM accepts inline `data:<mime>;base64,<payload>` image URLs in prompt
  files. They were previously read as local file paths and failed with
  "Failed to read local image file". Whitespace in the payload is ignored,
  padding is optional, and the MIME type sent is sniffed from the decoded
  bytes. Non-base64 `data:` URLs fail with an error that names the scheme.
  The image cache keys `data:` entries by SHA-256 digest, and logs show the
  digest instead of the payload. `--server-side-download` still rejects
  `data:` entries.
- Imagegen `--seed-mode increment` with a single `--prompt` no longer pins
  every request to the CLI `--seed`. The synthetic prompt row leaves `seed`
  unset so increment can apply `seed + request_index`. Explicit per-row seeds
  in JSONL still override. Confirmed on campaign `publish-20261002T162512Z`
  (unique PNG digests under increment).
- `metrum-ai-bench-cli preflight` accepts `--extra-body-json` and merges it
  into chat probe bodies (needed for thinking models that require
  `chat_template_kwargs.enable_thinking=false` to emit visible tokens).
- Default all-smi scrape URL in telemetry examples/config is
  `http://127.0.0.1:9090/metrics` (Metrum fork `v0.26.3-metrum.4`; `/metric`
  returns 404).
- `docs/queries/analyze.py` power correlation: skip power limit/cap gauges,
  and scale `*millijoule*` energy counters to joules unless the metric name
  contains `raw`.
- `scripts/live/shadeform.sh` prefers `env.json`'s `SHADEFORM_API_KEY` when
  the process environment also sets a different key, and warns on stderr
  (no key material printed).

## 1.5.2 (2026-09-30)

### Fixed
- Reject empty measurements before any request: modality binaries fail when
  `warmup_requests >= num_requests`; strategic fails when
  `requests_per_stage == 0`.
- Streaming chat runs that produce no visible-token TTFT now fail unless
  `--infer-ttft-from-first-byte` is set. That opt-in copies HTTP
  time-to-first-byte into `ttft_s`, stamps `ttft_source=first_byte_approx`,
  and prints a stderr warning with the approximated request count. Non-streaming
  chat warns that TTFT is unmeasured.
- LLM chat no longer injects a default system prompt. Omitted `--system-prompt`
  matches strategic's user-only body so both binaries publish comparable
  server `usage.prompt_tokens`.
- Runtime ISL/OSL validation prefers server usage token counts; tokenizer
  lengths apply only when `usage_missing` is set. `isl_osl.length_basis`
  records `server_usage` or `tokenizer`.

### Added
- `--infer-ttft-from-first-byte` on LLM, VLM, and strategic chat.
- `ttft_s` distribution, `ttft_approx_count`, and `ttft_warning` on strategic
  sweep points; per-request `ttft_source` on request JSONL / strategic CSV /
  NDJSON.
- Quickstart points real GPU servers at vendor Docker images (F27).

## 1.5.1 (2026-09-24)

### Changed
- `metrum-ai-bench-cli-prompts` defaults to Hub revision `main` (latest) and
  always resolves floating refs to a commit SHA recorded in `--report`. Pass an
  explicit 40-character SHA to pin; use `--require-pinned-revision` for
  publication gates. `--allow-moving-revision` is a deprecated no-op.
- Shadeform e2e defaults to `PROMPT_LIBRARY_REVISION=main` (no hardcoded SHA).

## 1.5.0 (2026-09-24)

### Added
- Strategic telemetry NDJSON (`--ndjson`) with tagged `run`/`stage`/`request`/
  `telemetry`/`scrape_error`/`summary` rows on a shared monotonic epoch.
- Multi-source Prometheus scrape config (`--telemetry` YAML), startup probes,
  `--require-telemetry`, and `--metrics-url` desugaring into the same writer.
- Mock server `--telemetry-fixture` serving canned DCGM/all-smi/vLLM exposition
  on `/metrics` and `/metric`.
- Docs: `docs/TELEMETRY.md`, exporter examples (default: Metrum all-smi fork
  `/metric`), offline analysis recipes under `docs/queries/` (no in-binary SQL).
- Shadeform e2e helper scripts: Hub-pinned prompt-library extract, promptfoo
  general/coding suites, interrupt/resume helpers, and optional on-host AIPerf
  bake-off driver. Redacted run evidence under `artifacts/e2e/`.

### Fixed
- Shadeform e2e no longer falls back to synthetic prompts when Hub extract
  fails. `scripts/e2e/run-shadeform.sh` requires pinned
  `metrum-ai/prompt-library` (`--revision`, `--config sample`, `--profile`
  `rag-medium`, `--output`) and asserts `mix-report.json`.

### Changed
- Prefer published Metrum all-smi release binaries in telemetry install docs
  (instead of `cargo install --git` only).

## 1.4.0 (2026-09-24)

### Removed
- `docs/COMPARISON.md`, docs-site Comparison page, and vs-other-tools bake-off
  artifacts (`docs/reviews/BAKEOFF_LLM_AIPERF.md`, `docs/reviews/bakeoff/`,
  `scripts/live/bakeoff/`). This repository does not retain comparison against
  other measurement tools. `metrum-ai-bench-cli compare` remains a helper that
  diffs two of our own strategic run summaries.

### Changed
- **Breaking (publication gate):** `--require-sut` rejects incomplete SUT
  declarations before any request: requires `gpu.model`, `gpu.count` (>0),
  `driver_version`, `runtime.name`, `runtime.version`, `runtime.config`, and
  `host_os`. Plain `--sut` stays permissive for partial manifests.
- Strategic warmup is a hard barrier: warmups fully complete, then the
  measurement epoch resets and measured prompt indexing restarts at zero.
- Strategic stdout adds `schema_version`, `tool_version`, `environment`, and
  `config` for publication-policy manifests (keeps `points` for compare).

### Added
- Strategic chat: `--ignore-eos`, `--min-tokens`, and `--extra-body-json` for
  fixed-length throughput studies (engine extensions; stamped in stage config).

## 1.3.1 (2026-09-23)

### Fixed
- `metrum-ai-bench-cli-prompts --config sample`: accept Hub object-shaped
  `sample-index.json` with a `source_lines` array (bare JSON arrays still work).

### Added
- Performance methodology **Workload** section: prompt source, ISL/OSL,
  named profiles, warmup, client headroom (live docs).
- Expanded on-host LLM vs AIPerf bake-off report
  (`docs/reviews/BAKEOFF_LLM_AIPERF.md`) with stage latency tables.

### Changed
- Performance methodology: document recorded `connect_s` / prefill proxy
  (no longer "deferred post-v1").

## 1.3.0 (2026-09-23)

### Removed
- Homebrew packaging and release automation deleted
  `packaging/homebrew/`, `scripts/render_homebrew_formula.sh`, and the
  release workflow `homebrew` job / `publish_homebrew` input. Install from
  GitHub Release tarballs, crates.io, or source.
- Deprecated shim binaries `metrumbench-*` and
  pre-1.2.0 `metrum-ai-bench*` names. Only `metrum-ai-bench-cli*` binaries
  remain.

### Added
- `metrum-ai-bench-cli preflight --url …`: reachability, chat probe, streaming
  first-token smoke, and latency sample with a pass/fail table; remediation
  points at Docker-first Platforms docs. Exit non-zero when a probe fails.
- `metrum-ai-bench-cli sut init [--probe]`: write a SUT JSON template; `--probe`
  fills local nvidia-smi / OS / CPU / memory with `field_provenance` marking
  observed vs declared fields (never remote SSH).
- `metrum-ai-bench-cli compare`: labeled delta table (Markdown/JSON) over two or
  more strategic stdout summaries or request CSVs.
- Strategic `--html` report: labeled axes, ticks, grid, and point tooltips
  (still a static SVG, not a chart package).
- Docs: agent-driven multi-run compare/chart prompt; SUT provenance and
  preflight limits.
- Observed concurrency on modality `summary.v3` and strategic stages:
  in-flight mean/p50/max, cap-engagement fraction, optional
  `in_flight_at_send` (#145).
- Phase/transport diagnostics: `connect_s` (connector TCP/TLS; `0` = pool hit),
  `prefill_s` proxy, `decode_s` / `decode_tok_s` from e2e−ttft (#146). TTFT
  still includes queue/TLS by design.
- Runtime ISL/OSL validation: `--isl-target` / `--osl-target` /
  `--prompt-mix-report`, mismatch counts on summary, `--fail-on-osl-mismatch`
  (#147).
- Modality summaries stamp `--scenario` into `summary.v3.config.common.scenario`.
- `metrum-ai-bench-cli-strategic`: `--sut` / `--require-sut` (publication parity);
  SUT embedded in stage config, stdout JSON, and HTML.
- Warn when `--url` is not loopback/RFC1918 (WAN RTT in TTFT); silence with
  `METRUM_AI_BENCH_ALLOW_REMOTE_URL=1`.
- Warn when SUT `model.quantization` is set (performance, not answer quality).
- `metrum-ai-bench-cli-strategic`: `--prompts` (JSONL / Hub-extractable mix),
  `--max-tokens` (required with `--prompts`), `--warmup-requests` (per-stage;
  excluded from aggregates), optional `--shuffle-prompts` / `--seed`. Stage
  `config` stamps workload fields; CSV rows carry `warmup`.
- Strategic streaming captures ITL; honors `--slo tpot=` and `--slo user_tps=`
  (tok/s per in-flight user) with per-stage `users_at_slo` / `user_tps` fields.
- Optional `--price-per-hour` and `sut.cost.price_per_hour` emit
  `cost_per_million_output_tokens` on modality `summary.v3` and strategic
  sweep points (`null` when price or token rate is absent).
- `metrum-ai-bench-cli-prompts --profile` named versioned ISL/OSL workloads
  (`chat-short`, `chat-medium`, `rag-medium`, `summarize-long`, `code-medium`).
- Docs: agent-driven path defaults to Hugging Face `metrum-ai/prompt-library`;
  agents must web-search current vendor/model serving defaults on every run;
  platforms page leads with vendor Docker for vLLM/SGLang and notes Blackwell /
  SGLang SWA caveats.

## 1.2.0 (2026-09-23)

### Changed
- Crate, binaries, release archives, and Homebrew formula rename to
  **`metrum-ai-bench-cli`** (was `metrum-ai-bench`) to match the formal product
  name. JSONL schema ids are now `metrum-ai-bench-cli.*.v3`; pre-1.2.0
  `metrum-ai-bench.*.v*` ids remain readable for audit.
- Deprecated shims keep the old `metrum-ai-bench*` binary names through
  **1.3.0** (stderr removal notice), in addition to `metrumbench-*`.
- crates.io publishes **`metrum-ai-bench-cli`**. The incorrectly named
  `metrum-ai-bench` 1.1.2 / 1.1.3 crates are yanked.

## 1.1.3 (2026-09-23)

### Added
- Docs: [Agent-driven benchmarking](external-docs/content/docs/agent-driven.mdx)
  with a drop-in `AGENTS.md` / `CLAUDE.md` hint and a copy-paste sample prompt
  for Claude Code, Codex, and OpenCode.
- Release archives stage `examples/` and `test-data/` (smoke asserts
  `sut.example.json` and `llm-hi.jsonl`).
- Docs quickstart: tarball `bin/dummy-model-server`, SHA-256 / Sigstore verify,
  unpack + `PATH` commands; `/docs` client redirect to quickstart.
- `metrum-ai-bench-cli selftest` prints a final `selftest: ok` line after the
  environment JSON (exit 0 = success).

### Changed
- Comparison docs: AIPerf already ships ASR, image, video, and VLM; stop
  claiming multi-modality as what Metrum AI Bench CLI adds over AIPerf.
- Deprecated shims `metrumbench-*` will be removed in **1.3.0** (stderr warning
  on every invocation, including `--help`). Release notes state they ship as
  back-compat aliases until then.
- Smoke-results intro notes package `1.0.0-rc.1` predates `--require-sut` and
  that the page is a smoke summary, not a
  [RESULTS_PUBLICATION_POLICY.md](docs/RESULTS_PUBLICATION_POLICY.md)
  publication.
- Limitations "No agent mode" clarifies workloads under test vs driving the
  CLI from a coding agent.
- GitHub Release assets no longer attach orphan `metrum-ai-bench.rb`; formula
  still generates for the gated Homebrew tap job.
- README quickstart notes docs-site latency bands assume `--max-tokens 20`.

## 1.1.2 (2026-09-22)

### Added
- GitHub Release tarballs include `TRADEMARKS.md` and the policy docs under
  `docs/` (`RESULTS_PUBLICATION_POLICY.md`, `CLAIMS_LEDGER.md`, `NAMING.md`,
  `COMPARISON.md`). Unpack smoke asserts those files are present.
- Release verify requires the release tag commit to be an ancestor of `main`.

### Changed
- Dropped the short form `Bench CLI` from naming policy so it matches
  `TRADEMARKS.md`; customer-facing docs use the formal name.
- CodeQL no longer soft-fails (`continue-on-error` removed); the repo is public.
- Self-hosted runner systemd unit uses `User=runner` and `/home/runner/...`
  paths instead of a personal home directory.
- Dataset docs treat [`metrum-ai/prompt-library`](https://huggingface.co/datasets/metrum-ai/prompt-library)
  as published (Apache-2.0). `docs/datasets/DATASET_CARD.md` is the local card;
  `DATASET_CARD.draft.md` is a superseded pointer, not an unpublished Hub set.

## 1.1.1 (2026-09-22)

### Added
- Restored the counsel-approved trademark, naming, claims-ledger, and
  results-publication policies, approved September 21, 2026.
- Restored the customer-facing results-publication guide.

### Changed
- Comparison guidance pins GuideLLM 0.7 and labels the landscape review as
  documentation research rather than a side-by-side benchmark.
- Smoke-results documentation states the measured package (`1.0.0-rc.1`) and
  hardware without repeated non-citable warnings, and uses the canonical
  **Metrum AI Bench CLI** name.
- Naming CI accepts the approved transition form
  **Metrum AI Bench CLI, formerly Metrum Insights CLI** and rejects incomplete
  product names such as `Metrum Bench CLI`.

## 1.1.0 (2026-09-19)

### Added
- GitHub Release tarballs include a static `bin/dummy-model-server` for the
  same four Linux and macOS targets as the Rust binaries, so unpacking a
  release does not require a Go toolchain.

## 1.0.1 (2026-09-19)

### Added
- `deploy/github-runners-bench-cli/`: compose-based `bench-cli` self-hosted
  runner pool assets for pikachu (`Dockerfile`, `docker-compose.yml`,
  `.env.example`, `bootstrap.sh`) plus manual `pool` operations and an optional
  `gha-bench-cli-pool.service`.

### Changed
- CI now includes a `self-hosted` cargo test job on
  `[self-hosted, linux, x64, bench-cli]` with fork-PR isolation and serialized
  per-ref execution to keep untrusted code off the org runner host.

## 1.0.0 (2026-09-18)

First stable release after the 1.0.0-rc series. This tag was re-cut after a
history rewrite that removes `docs/OSS_READINESS_ASSESSMENT.md` from every
published ref; release archives and Sigstore bundles are regenerated for the
rewritten commit.

### Added
- `docs/REASONING_MODELS.md`: operator guide for thinking models (TTFT vs
  first reasoning, `--extra-body-json` / `reasoning_effort`, `--max-tokens`
  probe procedure). Linked from README, METRICS, LIMITATIONS, and CLAUDE.md.
- README Quickstart (60 seconds): publishable dummy-server LLM run with
  `--sut` / `--require-sut` and `test-data/llm-hi.jsonl`.
- CLAUDE.md agent notes for running benchmarks and listing every non-deprecated
  binary.
- `--quiet` and `NO_BANNER=1`: suppress ASCII banner art; one-line identity
  remains. Documented in README Install and regenerated `docs/CLI.md`.
- Shared `chat_stream` consumer for LLM, VLM, and strategic chat streaming.
- `metrum-ai-bench-cli-strategic --streaming`: opt-in SSE for chat turns with
  per-turn `first_byte_s` / `ttft_s` and TTFT SLO enforcement.
- Known limitation: gateways that synthesize SSE from unary upstream calls
  report total latency as TTFT (undetectable client-side).

### Fixed
- `metrum-ai-bench-cli-prompts` Hub checksum verification is scoped to the
  requested dataset config so `full` and `sample` no longer collide on shared
  parquet basenames (#101).
- Prompt mix selection prefers a unique draw from an exact ISL/OSL cell before
  the sparse hill-climber, so `--count-slack 0` and `--no-repeats` succeed when
  the target bucket is fully populated (#102).
- `metrum-ai-bench-cli-prompts` and `metrum-ai-bench-cli-strategic` expose clap
  `-V` / `--version`; strategic also supports `--version-only` (#99).
- `docs/RELEASING.md` cosign verify example uses the v-prefixed archive names
  that the release workflow actually attaches (#100).

### Changed
- Customer-facing product name standardized as **Metrum AI Bench CLI**.
- Counsel-pending drafts (`TRADEMARKS.md`, `docs/NAMING.md`,
  `docs/CLAIMS_LEDGER.md`, `docs/RESULTS_PUBLICATION_POLICY.md`) and internal
  campaign/evidence docs removed from the public tree pending sign-off.
- `--url` and `--api-key` are mutually required at clap parse time on llm,
  vlm, asr, and imagegen (endpoints-file path unchanged). Help text states
  Bearer-token semantics and the `dummy` placeholder.
- `--extra-body-json` help points at reasoning_effort examples and the run
  manifest; imagegen `--extra-body-file` help filled; `scripts/render_cli_help.sh`
  covers unified, strategic, and mock-server.
- Banner prints only on interactive TTY after argument parse; non-TTY / quiet
  sessions get `Metrum AI Bench <tool> <version>`.
- README reordered for agent scanning: Quickstart, Tools entry points,
  Reasoning models, Publishing (with SUT example), Prompt library, Dummy
  server. Dummy-server `go run` commands use `(cd dummy-model-server && …)`
  because the Go module lives in that subdirectory.
- `docs/METRICS.md`: TTFT vs first reasoning under its own heading.
- `docs/LIMITATIONS.md`: NVIDIA smoke matrix wording is coverage-only; SUT
  flags described as shipped.

### Planned
- SUT block should carry first-class dataset provenance fields (`dataset`,
  `dataset_revision`, `dataset_rows`) rather than only free-form `extra` /
  notes.

## 1.0.0-rc.6 (2026-09-17)

Release candidate: `rand` 0.10.2 (soundness) and prompt-library mix extractor.

### Added
- `metrum-ai-bench-cli-prompts` (also `metrum-ai-bench-cli prompts -- …`): select a
  reproducible ISL/OSL mix from
  [`metrum-ai/prompt-library`](https://huggingface.co/datasets/metrum-ai/prompt-library)
  by mean or median within absolute tolerances; preferred `--count` may vary
  within `--count-slack` and source rows may repeat. Writes JSONL for
  `metrum-ai-bench-cli-llm` plus a selection report with recommended
  `--num-requests` / `--max-tokens`. Docs: `docs/PROMPT_LIBRARY.md`.

### Changed
- Direct dependency `rand` 0.9.5 → 0.10.2 (soundness fixes in 0.10.1/0.10.2;
  `Rng` → `RngExt` call sites). Transitive `tokenizers` still uses `rand` 0.9.x.

## 1.0.0-rc.5 (2026-09-17)

Release candidate: naming alignment, publication SUT block, policy drafts, and CI gates.

### Added
- `--sut <PATH>` embeds an operator-declared system-under-test block (JSON/YAML) into the summary as `sut`, labelled `provenance: "declared"`. Absent → `"sut": null` plus a stderr notice.
- `--require-sut` (env `METRUM_AI_BENCH_REQUIRE_SUT=1`) refuses to run without a valid SUT block; implies `--redact-hostname`. Use for any run intended for publication.
- `--redact-hostname` (env `METRUM_AI_BENCH_REDACT_HOSTNAME=1`) writes `environment.hostname: null`.
- `examples/sut.example.{json,yaml}`.
- TRADEMARKS.md, docs/RESULTS_PUBLICATION_POLICY.md, docs/CLAIMS_LEDGER.md, docs/NAMING.md (drafts pending counsel review).
- docs/LIMITATIONS.md; docs/datasets/DATASET_CARD.draft.md (not published).
- docs/RELEASING.md (publish variables and cosign notes).

### Changed
- Crate renamed `metrumbench` → `metrum-ai-bench-cli` to match the `metrum-ai-bench-cli` binary. Not yet published to crates.io; no migration needed.
- Mock server binary renamed `metrumbench-mock-server` → `metrum-ai-bench-cli-mock-server`.
- MLPerf interoperability export: SUT name field `"MetrumBench"` → `"Metrum AI Bench"` (label only; no metric or schema change).
- Tarball prefix and Homebrew formula follow the crate name.
- Summary schema v3: optional `sut` field added; `environment.hostname` is now nullable. Additive; readers must treat both as optional.
- `scripts/live/campaign.sh` / `matrix_smoke.sh` pass `--sut sut.json`.
- docs/COMPARISON.md rewritten against current code and the September 2026 landscape (AIPerf replaces retired GenAI-Perf; InferenceX and vLLM/SGLang bench_serving added; MLPerf export described as the unofficial interoperability export it is).
- docs/reviews/QUALITY_ASSESSMENT_REPORT.md carries a disposition header; current verdict lives in SCORECARD_1.0.0.md.
- docs/SMOKE_RESULTS.md states the matrix is NVIDIA-only with Instinct in progress.
- README: publishing-a-result section, security and provenance, known limitations, trademark notice.
- CI enforces the product-naming rule (docs/NAMING.md) via scripts/check_headers.sh; DCO sign-off enforced on PRs.
- crates.io and Homebrew publish now require explicit repository variables (`CRATES_IO_PUBLISH`, `HOMEBREW_PUBLISH`) and never run for `-rc.` tags.
- Build-provenance attestation is required when the repository is public.
- Added CodeQL (Rust), cargo-geiger, cargo-outdated/udeps weekly jobs; gitleaks custom rules for account IDs, cleartext passwords, SSH public keys, internal hostnames.

## 1.0.0-rc.4 (2026-09-16)

Release candidate after retracting the mistagged GA and Dependabot maintenance.

- Release hygiene: deleted mistagged non-prerelease `v1.0.0` (tag + GitHub
  Release) so `1.0.0` remains available for eventual GA; marked existing
  `1.0.0-rc.*` releases as prerelease; release workflow now sets `prerelease`
  automatically for `-rc.` / `-alpha.` / `-beta.` tags.
- Dependencies: `base64` 0.23.1, `sha2` 0.11.0, optional `tokenizers` 0.23.2;
  GitHub Actions bumps for checkout, setup-go, rust-cache, upload-artifact, and
  softprops/action-gh-release.

## 1.0.0-rc.3 (2026-09-16)

Release candidate: cross-platform release binaries via cargo-zigbuild.

- Release pipeline: tagged builds use a pinned `ghcr.io/rust-cross/cargo-zigbuild`
  image on Linux instead of native macOS / ARM Ubuntu compile runners. Targets
  remain `x86_64`/`aarch64` `*-unknown-linux-gnu` (dynamically linked glibc,
  2.17 floor) and `*-apple-darwin`. Not musl; TLS remains rustls.
- Binary identity differs from rc.2 (Zig linker / glibc floor / Darwin SDK in
  the zigbuild image). Compile-free smoke jobs unpack each archive and run
  `metrum-ai-bench-cli --help` / `selftest` on matching Linux and macOS runners
  before GitHub Release, crates.io, and Homebrew publish.

## 1.0.0-rc.2 (2026-09-16)

Release candidate after the 1.0.0 measurement residuals and OSS-readiness docs.

- Measurement residuals (N-02–N-07, N-09, F-09, F-14): stamp monotonic
  `send_offset_s` and derive the window and closed-loop bins from it; normalize
  trailing/short throughput bins by actual width; VLM maps in-stream `error`
  events and writes failed records on preprocess/body-build skips; stamp
  `effective_max_concurrency`; classify TCP reset as `connect` and use Instant
  for failure latency; drop `imagegen.request.v1` (artifact SHA-256 on
  `request.v3` `modality_labels`); omit zero token throughput for non-token
  modalities; MLPerf export no longer contains `Result is : VALID`; clarify
  `--ca-cert` must be a CA certificate.
- Docs / packaging (OSS readiness): public README install and examples; move ASR
  notes under `docs/`; Dependabot + weekly `cargo deny`; SECURITY no-unsafe
  sentence; tighter `Cargo.toml` exclude; smoke matrix and true `ttft_s` in
  `SMOKE_RESULTS` (N-01).

## 1.0.0-rc.1 (2026-09-15)

Feature baseline for the 1.0 line. A mistagged non-prerelease `v1.0.0` pointing
at this same line was deleted; `1.0.0` is reserved for the eventual GA.

Breaking / schema notes:
- New campaigns reject unversioned or legacy (non-`request.v*` / `summary.v*`) JSONL lines.
- Field-additive schema bump to `request.v3` / `summary.v3` / related config stamps (existing numeric meanings unchanged; keep a v2 reader for 0.1.82 audit).
- Dual unversioned modality summaries removed; console and JSONL use `RunSummary` only.
- `metrumbench-*` shims remain through v1.x and will be removed in v2.0.
- Verified on Shadeform RTXPro6000 campaign `v1rc1-20260915-190838` (tag `v1.0.0-rc.1`); see `docs/SMOKE_RESULTS.md`.

- Public readiness: full `cargo deny check` in CI; replace `ntp`/`lru` (std SNTP +
  hand-rolled VLM image LRU); drop compile-time wall-clock datetime for
  reproducible builds; MSRV 1.85 CI job; SHA-pinned Actions; govulncheck for
  dummy-model-server; dummy-server body limits / timeouts / non-root Docker;
  issue/PR templates; refreshed `THIRD_PARTY_LICENSES`.
- Remove dual legacy summaries (F-05, F-25, F-26): modality binaries write only
  `request.v3` + `summary.v3` via `JsonlSink`; console stats render from
  `RunSummary` / `DistSummary` (type 7). Imagegen `--summary-json` writes
  summary.v3 (no stdout pretty duplicate). Unused `modality.rs` / `transport.rs`
  deleted. `metrumbench-*` shims kept through v1.x with explicit v2.0 removal
  notices.
- Strategic honesty (F-14, F-16): sweep points carry DistSummary (`n`, errors,
  p99_unreliable), redacted config, shared warm HTTP pool; `--slo e2e=` for
  goodput (without SLOs `goodput_equals_throughput`); MLPerf export files start
  with an UNOFFICIAL disclaimer and never emit bare `Result is : VALID`.
- Campaign `validate` rejects unversioned/legacy JSONL lines; `request.v2` /
  `summary.v2` remain accepted for 0.1.82 regression audit
  (`record::accepts_audit_schema`).
- Modality CLI parity (F-08, F-10, F-21, F-22, F-28): non-streaming LLM reports `ttft_s: null` and stops the clock after the full body is read; VLM honors `--system-prompt` / `--min-tokens` / `--tokenizer` and stamps `effective_system_prompt`; ASR drops conflicting legacy `throughput.rtfx` (measured-phase `rtfx_client` only); imagegen accepts base or full `/images/generations` URLs, excludes decode/hash/write from service latency, and uses the measured-phase window for legacy throughput; `--summary-json` is optional for imagegen; shared `--fail-on-error` (default off) for consistent exit policy across modalities.
- Transport parity (F-06, F-12, F-13, F-23): typed `RequestError` mapping at the failure site via `from_reqwest` / `from_status` (timeout/connect/5xx no longer depend on Display substrings); optional `first_byte_s` on `request.v3`; shared `--ca-cert` / `--insecure` (stamped into `config.common`, never secrets); least-inflight temporary ejection after connect failure; SSE blank-line framing with multiline `data:` joined by `\n`.
- Deferred stretch: `connect_s` (connection-established Instant) remains post-v1 / feature-flagged; TTFT continues to include connect by design.
- Summary v3 effective config: stamp `config` (`run_id`, common args, effective system prompt, sanitized `body_template`, unique-prompt nonce template) on all four binaries; unique-prompt nonces are `[nonce-{run_id}-{seed}-{seq}]`. Add `usage_missing_count`, nullable `completion_tokens_per_second` with `completion_tokens_source` (`server_usage` / `tokenizer_fallback`), and `p90_unreliable` / `p95_unreliable` on distributions.
- Modality runners (VLM, ASR, imagegen): roll out the shared `runner.rs` contract already used by LLM: in-task `started_at` / flush via `JsonlSink`, SIGINT+SIGTERM `StopFlag`, closed-loop schedule omission, and `window_seconds` from measured record span. VLM no longer drops warmup records (metrics skip by `phase` only).
- LLM runner: shared `runner.rs` timestamps send/completion inside the task, writes `request.v3` immediately, handles SIGINT/SIGTERM, and derives `window_seconds` from records (closed-loop ~7.9 req/s at c=4/n=16 on the dummy). Schema bump to `request.v3` / `summary.v3` (field-additive). E2e covers window, flush-during-launch, and SIGTERM JSONL prefix.

## v0.1.82 (2026-09-15)

- Load scheduler: `FakeClock` for deterministic open-loop tests; `docs/CLI.md` regenerated from clap `--help` via `scripts/render_cli_help.sh`.

## v0.1.81 (2026-09-15)

- ASR: `--normalizer {whisper-english,whisper-basic,none}` selects the text normalization applied to both sides of WER/CER, and the choice is recorded in `config.normalizer`. WER/CER are pinned by a hand-computed reference table.
- VLM: source image bytes are sent unchanged unless `--max-image-dimension` forces a resize or the new `--reencode-jpeg` is requested; images are no longer decoded when neither applies. Per-request records carry `modality_metrics.image_bytes` and `image_count`.
- Dummy-server end-to-end coverage for VLM (streaming TTFT/ITL, non-streaming without a fabricated TTFT, payload preservation), ASR (RTFx, normalizer selection), imagegen (monotonic latency, warmup exclusion, shared summary), and seeded open-loop determinism for constant and Poisson arrivals.
- Adversarial stream coverage: SSE frames flushed mid-event, a missing `data: [DONE]` sentinel, role-only streams classified `no_output_token`, and reasoning deltas kept out of TTFT. Ctrl-C is covered end to end: records stay on disk and the summary is marked `partial`.
- CI runs the dummy-server end-to-end tests instead of skipping them (`METRUM_BENCH_REQUIRE_DUMMY=1`), and gates `gofmt`.

## v0.1.80 (2026-09-14)

- Measurement core: complete-line SSE parser, Hyndman–Fan type 7 percentiles, ITL vs N−1 TPOT, per-request JSONL (`metrum-ai-bench-cli.request.v2`), Ctrl-C partial summaries, `--warmup-requests`, `--seed`, `--request-rate` / `--arrival`, `--ignore-eos`, `--extra-body-json`, `--unique-prompts`.
- VLM: optional `--streaming` TTFT (non-streaming no longer fabricates TTFT); images preloaded before the measurement window.
- ASR: Whisper-like text normalization for WER; request clock starts after audio is read; `throughput.rtfx` = total audio seconds / wall time.
- Imagegen: monotonic `Instant` latency; seeded prompt shuffle; `--warmup-requests`.
- Dummy-server e2e test for LLM streaming timing (TTFT ~120 ms, RT ~500 ms at latency=100ms, chunk-interval=20ms, max_tokens=20).

## v0.1.79 (skipped)

- Intentionally skipped; numbering jumps from v0.1.78 to v0.1.80. No release artifacts were published for v0.1.79.

## v0.1.78 (2026-05-02)

- Extended license validity date until July 31, 2026.
- Updated license check tests and operator-facing license documentation to reflect the new expiry date.
- **metrumbench-llm**: treat `--ramp-up-seconds 0` as no ramp-up so throughput
  denominators cover the actual measured workload instead of a late
  post-drain metrics window. Positive ramp-up values retain ramp-up behavior.

## v0.1.77 (2026-02-19)

- Version bump (metrumbench crate). Multi-endpoint support remains at v0.1.76 behavior for metrumbench-llm, metrumbench-vlm, and metrumbench-asr.
- **metrumbench-asr**: `--input` and `--ground-truth` accept a local JSONL path or an `http://` / `https://` URL (blocking GET; same pattern as metrumbench-llm prompt URLs). Shared helper `read_utf8_from_path_or_url` in `metrumbench::prompt_inputs`.
- Dummy model server (metrumbench-vlm mode): accept `content` as string or array of parts for compatibility with metrumbench-vlm client.

## v0.1.76 (2026-02-19)

### metrumbench-vlm and metrumbench-asr multi-endpoint support

- **metrumbench-vlm** and **metrumbench-asr** now support the same multi-endpoint workflow as metrumbench-llm: optional `--endpoints-file` (YAML), weighted round-robin, per-endpoint and aggregate metrics, and data_log schema with `config.endpoint` / `config.endpoints` and `metrics.per_endpoint`.
- **metrumbench-vlm**: `--url` and `--api-key` are optional when `--endpoints-file` is provided; exactly one of (single `--url`/`--api-key`) or `--endpoints-file` is required.
- **metrumbench-asr**: Same mutual exclusivity; provide either `--url` + `--api-key` or `--endpoints-file`. Other required args (scenario, num_requests, input, model) unchanged.
- Shared endpoint resolution lives in the `metrumbench` lib (`metrumbench::endpoints`) for all three tools.

## v0.1.75 (2026-02-19)

### metrumbench-llm multi-endpoint support

- **Multi-endpoint benchmarking**: Use `--endpoints-file` with a YAML file to distribute requests across multiple endpoints with per-endpoint credentials and optional weights (weighted round-robin).
- **Single-endpoint unchanged**: `--url` and `--api-key` still work as before when not using `--endpoints-file`. Exactly one of (single `--url`/`--api-key`) or `--endpoints-file` is required.
- **Per-endpoint and aggregate metrics**: Console output and data_log summary include per-endpoint blocks and an AGGREGATE block when multiple endpoints are used. JSONL summary adds `config.endpoint` / `config.endpoints` and `metrics.per_endpoint`.

### Deprecation notice (data_log / JSONL output)

- **The previous data_log / JSONL output shape, format, and schema is deprecated as of this release.**
- **The old format will be disabled (removed) in the next release.** Consumers that parse the data_log JSONL line must migrate to the new schema (e.g. `config.endpoint` or `config.endpoints`, `metrics.per_endpoint`) before upgrading to the next release.

## v0.1.68-beta (2025-04-09)
- Updated rand crate to version 0.8.5 for improved random number generation
- Enhanced random word selection for unique ID generation
- Improved code quality and maintainability
- Fixed deprecated function usage in random number generation
- Updated documentation to reflect dependency changes

## v0.1.64 (2025-04-08)
- Extended license validity date until May 31, 2025
- Updated license check message to reflect new expiry date
- Fixed bug in metrumbench-asr tool that prevented running more requests than available audio samples
- Implemented round-robin audio sample selection for longer benchmark runs
- Minor documentation improvements

## v0.1.63 (2025-04-07)
- Enhanced metrumbench-asr audio transcription benchmarking tool
- Improved accuracy metrics collection and reporting
- Added support for comprehensive audio format analysis
- Enhanced error handling and retry logic
- Updated documentation for audio transcription features

## v0.1.62 (2025-04-06)
- Added support for direct file path audio sources in metrumbench-asr
- Enhanced audio format detection and validation
- Improved caching mechanism for audio files
- Updated documentation with new audio source examples

## v0.1.61 (2025-04-06)
- Initial release of metrumbench-asr audio transcription benchmarking tool
- Support for OpenAI Whisper and compatible APIs
- Implementation of detailed accuracy metrics (WER, CER, RTF)
- Basic audio format support and caching

## v0.1.60 (2025-04-05)
- Enhanced ramp-up functionality with improved metrics collection
- Added validation for ramp-up period configuration
- Updated documentation with comprehensive ramp-up examples
- Improved error handling during ramp-up transitions

## v0.1.59 (2025-04-05)
- Added ramp-up functionality for gradual concurrency increase
- Implemented configurable ramp-up period in seconds
- Added validation for ramp-up period against stop-after period
- Enhanced metrics collection to separate ramp-up and steady-state periods
- Updated documentation with ramp-up examples and usage guidelines
- Fixed unused variable warning in rustyphalanx.rs

## v0.1.58 (2025-04-04)
- Added server-side image download option to metrumbench-vlm tool
- Enhanced image handling with configurable download modes
- Updated documentation with new feature examples
- Improved image URL handling in request payloads
- Added support for direct URL passing in vision model requests
- Fixed unused import warning in rustyphalanx.rs

## v0.1.57 (2025-04-03)
- Added platform-specific release archives (macos/linux, arm64/x86_64)
- Updated build system to use gtar when available on macOS
- Enhanced release naming convention for better platform support
- Updated documentation for platform-specific releases
- Improved build system integration for cross-platform support

## v0.1.56 (2025-03-25)
- Updated Python dependencies with explicit version requirements
- Added matplotlib and numpy as core dependencies
- Enhanced build system integration for tools
- Improved development workflow automation
- Updated Rust dependencies to latest versions
- Added comprehensive documentation for dependency versions
- Enhanced build system documentation
- Updated usage examples and workflow guides
- Added new README.md in reporting/ directory

## v0.1.55 (2025-03-24)
- Enhanced charting module with automated Makefile integration
- Added new `make charts` target for easy chart generation
- Updated requirements.txt with explicit matplotlib and numpy dependencies
- Created comprehensive README.md in reporting/ directory
- Added automatic cleanup of generated charts in make clean target
- Improved documentation for charting tools and data formats
- Added support for word-to-token conversion factor (1.33) in visualizations

## v0.1.54 (2025-03-24)
- Added Python-based charting module in reporting/ directory for visualization of benchmarking results
- Enhanced performance metrics visualization with charts for requests/sec, token throughput, and TTFT
- Added comprehensive documentation for the charting tools
- Improved API documentation and usage examples
- Added additional clarity on tool interactions and workflow

## v0.1.53 (2025-03-22)
- First implementation of basic charting functionality
- Fixed documentation to use PATH consistently
- Updated examples across multiple readme files
- Improved announcement process for new releases
- Added missing documentation for library modules

## v0.1.52 (2025-03-21)
- Extended license validity date until April 30, 2025
- Enhanced scenario-combo.sh script with randomized testing scenarios
- Limited the maximum runtime for individual scenarios to 60 seconds
- Reduced the number of test combinations in scenario-combo.sh for more efficient testing
- Updated documentation to reflect license and feature changes

## v0.1.51 (2025-03-20)
- Added word count statistics for rustyphalanx tool
- Implemented word count metrics for both prompt and completion text
- Updated metrics output to include word-based throughput measurements
- Enhanced JSON log output with word count metrics and statistics
- Updated documentation to reflect word count functionality

## v0.1.50 (2025-03-19)
- Enhanced documentation for metrumbench-asr audio transcription benchmarking tool
- Added detailed usage examples and implementation notes
- Improved command-line argument documentation with required parameter indicators
- Added detailed explanation of metrics calculations (WER, CER, RTF)
- Documented file caching behavior and verbose_json format support
- Added comprehensive testing instructions for metrumbench-asr tool

## v0.1.49 (2025-03-18)
- Added metrumbench-asr tool for audio transcription API benchmarking
- Implemented support for multipart form uploads with audio files
- Added support for both URL-based and direct file path audio sources
- Integrated with OpenAI-compatible audio transcription APIs
- Comprehensive metrics for audio processing (RTF, WER, CER, throughput)
- Detailed documentation for audio transcription testing

## v0.1.48 (2025-03-18)
- Updated documentation for improved consistency and accuracy
- Added Release Plan document with upcoming features through May 2025
- Fixed parameter inconsistencies in tools documentation
- Improved build system with automatic version-based archive creation
- Added new license expiry note in announcements

## v0.1.47 (2025-03-11)
- Added test cases for new features
- Enhanced error handling for edge cases
- Fixed minor bugs in CSV parsing
- Updated dependencies

## v0.1.46 (2025-03-09)
- Added examples for API-based testing
- Created basic wrapper server for simplified API testing
- Improved documentation for server configuration

## v0.1.43 (2025-03-03)
- Enhanced metrumbench-vlm tool to support multiple images per prompt
- Added support for semicolon-delimited URLs in CSV input files
- Improved metrics tracking for multi-image requests
- Updated documentation to clarify multi-image capabilities

## v0.1.42 (2025-02-26)
- Updated license validity date
- Added new CLI option for restricting output length

## v0.1.41 (2025-02-18)
- Enhanced documentation
- Updated version requirements

## v0.1.40 (2025-02-10)
- Added sample file support
- Fixed timeout issues with large requests

## v0.1.39 (2025-02-05)
- Added support for Azure DevOps binary releases
- Removed S3 repository references

## v0.1.38 (2025-01-31)
- Added metrumbench-vlm tool for vision model benchmarking
- Implemented image processing and caching
- Added detailed image metrics collection

## v0.1.37 (2025-01-24)
- Enhanced ISO customizer with additional package options
- Added support for custom environment scripts

## v0.1.36 (2025-01-17)
- Added ISO customizer for creating custom Ubuntu images
- Implemented cloud-init configuration

## v0.1.35 (2025-01-10)
- Added container launch tool with YAML configuration
- Added subprocess launch tool with YAML configuration

## v0.1.34 (2025-01-03)
- Improved error handling and retry logic
- Added detailed logging capabilities

## v0.1.33 (2024-12-27)
- Added JSONL to CSV conversion utility
- Enhanced token counting logic

## v0.1.32 (2024-12-20)
- Added support for extracting prompts with specific criteria
- Implemented length-based filtering

## v0.1.31 (2024-12-13)
- Added support for waiting for vLLM service availability
- Implemented health check polling

## v0.1.30 (2024-12-06)
- Added support for streaming responses
- Improved connection pooling

## v0.1.29 (2024-11-29)
- Added support for multiple request modes
- Enhanced metrics collection

## v0.1.28 (2024-11-22)
- Initial public release
- Implemented basic load testing functionality
- Added support for OpenAI-compatible endpoints
