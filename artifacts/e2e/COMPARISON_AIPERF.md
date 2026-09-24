<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Metrum AI Bench CLI vs NVIDIA AIPerf (Shadeform bake-off)

Side-by-side study on one Shadeform GPU SUT serving Qwen via vLLM.
This report is generated from the e2e artifact bundle; it does not invent metrics.

## Dataset and ISL/OSL

Both tools drove the same OpenAI-compatible vLLM SUT with prompts selected from
[metrum-ai/prompt-library](https://huggingface.co/datasets/metrum-ai/prompt-library).

This bake-off used a **fixed** Hub mix (`rag-medium`), then swept **concurrency**
(closed-loop) and **request rate** (open-loop). It did **not** sweep multiple ISL/OSL
target pairs in this run.

- dataset: `metrum-ai/prompt-library`
- revision: `0666f62e581b482838ae2e17b333ee36ff3d01b0`
- config / split: `sample` / `train`
- profile: `{'name': 'rag-medium', 'version': 1}`
- selected_count: `48`
- schedule_sha256: `48b0dcd73a72c37696c5d5c28bb4371fc9bbf5b98b0e504cc724a335a507c9bf`

| Axis | Target | Achieved (mix median) | Tolerance | Unit |
|------|--------|------------------------|-----------|------|
| ISL | 2048 | 2000 | ±128 | tokens |
| OSL | 256 | 200 | ±64 | tokens |

AIPerf measured sequence lengths on the same prompt texts (avg over requests):

- input_sequence_length avg: `1614.3` tokens
- output_sequence_length avg: `200.0` tokens

## Workload matrix (what was swept)

| Sweep | Tool | Axis | Points | ISL/OSL |
|-------|------|------|--------|---------|
| Closed-loop concurrency | metrum strategic | concurrency | 1,2,4,8,16,32,64 | fixed `rag-medium` mix |
| Open-loop rate | metrum strategic | rate (req/s target) | 2,4,8,16 | same mix |
| Closed-loop concurrency | AIPerf | concurrency | 1,2,4,8,16,32,64 | same texts (`single_turn`) |
| Promptfoo quality | promptfoo | N/A (functional) | general 3 + coding 2 cases | N/A |
| ISL×OSL matrix | — | — | **not run** | would require multiple Hub mixes / profiles |

## Methodology

| Dimension | metrum-ai-bench-cli-strategic | NVIDIA AIPerf |
|-----------|-------------------------------|---------------|
| Load model | Closed-loop concurrency + open-loop rate sweeps | `aiperf profile --concurrency N` (closed-loop in-flight) |
| Prompts | Hub mix JSONL (`--prompts`) from prompt-library | Same texts via `--input-file` + `--custom-dataset-type single_turn` |
| Streaming | Yes | Yes |
| Output cap | `--max-tokens` + `--ignore-eos` | `--output-tokens-mean` / per-row `output_length` |
| Telemetry | Prometheus scrapers (all-smi, vLLM, node, optional DCGM) into tagged NDJSON | Built-in client metrics; optional server-side hooks (not used here) |
| Quality eval | promptfoo general + coding (separate from load sweep) | Not part of AIPerf bake-off |
| Interrupt / partial summary | SIGINT mid-run (best-effort in this e2e) | Not exercised |
| Artifact shape | Tagged NDJSON (`run`/`stage`/`request`/`telemetry`/`summary`) + HTML | Per-concurrency artifact dirs + profile exports |
| Setup | Copy release binaries + SUT exporters | `pip install aiperf` into a venv on the SUT |

Fairness notes:
- Same Shadeform GPU VM, same vLLM container, same served model id (`sut`).
- Same prompt-library schedule; AIPerf consumes `text` converted 1:1 from the metrum mix.
- Concurrency points match (1..64). Open-loop rate sweep is metrum-only in this bundle.
- Numbers are not claimed as vendor-competitive SLAs; this is an operator bake-off of tooling + methodology on one SUT.


## Setup and run time

| Tool | Setup (s) | Benchmark run (s) | Total (s) | Notes |
|------|-----------|-------------------|-----------|-------|
| metrum strategic (closed) | n/a | 508.0 | 508.0 | closed-loop wall time only (excludes cargo/scp/sut-setup); see cost.txt for full driver elapsed |
| AIPerf | 0.0 | 334.0 | 334.0 | version `0.11.0` |

## Performance comparison (closed-loop)

| Concurrency | Metrum req/s | Metrum out tok/s | Metrum TTFT p50 (s) | AIPerf req/s | AIPerf out tok/s | AIPerf TTFT mean |
|-------------|--------------|------------------|---------------------|--------------|------------------|------------------|
| 1 | 0.090 | 22.954 | 0.344 | 0.117 | 23.317 | 0.094 |
| 2 | 0.174 | 44.526 | 0.132 | 0.222 | 44.438 | 0.133 |
| 4 | 0.341 | 87.302 | 0.164 | 0.435 | 87.095 | 0.157 |
| 8 | 0.670 | 171.558 | 0.236 | 0.855 | 170.967 | 0.225 |
| 16 | 1.248 | 319.548 | 0.341 | 1.586 | 317.156 | 0.353 |
| 32 | 1.247 | 319.214 | 0.355 | 1.586 | 317.148 | 0.355 |
| 64 | 1.247 | 319.205 | 0.353 | 1.587 | 317.306 | 0.350 |

## Closed-loop knee (metrum)

- knee concurrency (load): `4.0`
- knee throughput (req/s): `0.341`
- knee output tok/s: `87.302`
- knee TTFT p50 (s): `0.164`

## Open-loop rate sweep (metrum)

| Target rate | req/s | out tok/s | TTFT p50 (s) | error_rate |
|-------------|-------|-----------|--------------|------------|
| 2 | 0.815 | 208.692 | 0.132 | 0.000 |
| 4 | 0.991 | 253.654 | 0.126 | 0.000 |
| 8 | 1.112 | 284.650 | 0.139 | 0.000 |
| 16 | 1.183 | 302.809 | 0.152 | 0.000 |

Open-loop knee load: `4.0` (req/s `0.991`, tok/s `253.654`).

## Promptfoo scores

Functional quality smoke against the same SUT (`enable_thinking=false`).
Wall-clock budget was capped at ~30 minutes; this run finished in seconds.

| Suite | Cases | Passed | Pass rate |
|-------|-------|--------|-----------|
| general | 3 | 2 | 0.667 |
| coding | 2 | 2 | 1.000 |

| Suite | Case | Success | Score |
|-------|------|---------|-------|
| general | capital_of_france | True | 1 |
| general | multi_step_arithmetic | False | 0 |
| general | causal_reasoning | True | 1 |
| coding | two_sum_indices | True | 1 |
| coding | reverse_words | True | 1 |

Failed cases: `multi_step_arithmetic`.

## Telemetry / energy (metrum closed-loop)

```
## stage power / energy
stage	load	power_n	power_mean_w	power_p95_w	energy_counter_j	energy_trap_j	out_tokens	j_per_out_tok
1.0	1.0	357	1546.65	1597.3	6.14366e+07	275304	4096	14999.2
2.0	2.0	184	1515.8	1525.41	2.8928e+07	138696	4096	7062.49
4.0	4.0	94	1518.47	1527.36	1.46105e+07	70608.6	4096	3567.01
8.0	8.0	48	1525.84	1527.29	7.06234e+06	35858.4	4096	1724.2
16.0	16.0	26	1550.21	1603.45	4.14365e+06	19378.1	4096	1011.63
32.0	32.0	26	1546.82	1587.79	4.10685e+06	19335.5	4096	1002.65
64.0	64.0	25	1549.65	1598.15	4.13934e+06	18595.7	4096	1010.58
```

## Detailed observations

1. **ISL/OSL:** One Hub profile (`rag-medium`) was used. Mix target ISL 2048 / OSL 256; achieved mix medians ~2000 / ~200. AIPerf measured ~1614 input tokens avg and 200 output tokens avg on the same texts. A multi-point ISL×OSL sweep was not part of this e2e.
2. **Closed-loop agreement:** Metrum and AIPerf output-token throughput track closely across concurrency (e.g. ~320 tok/s plateau by concurrency 16).
3. **Open-loop:** Metrum-only rate sweep saturates near ~1.2 req/s / ~300 tok/s by rate 16.
4. **Promptfoo:** Coding suite 2/2; general 2/3 (`multi_step_arithmetic` failed contains-assert).
5. **Client vs platform telemetry:** Metrum joins request rows with all-smi / vLLM / node scrapes in NDJSON; AIPerf focuses on client profile exports.
6. **Setup cost:** AIPerf needs a Python venv + pip (and an explicit `--tokenizer` when the served name is not a Hub id). Metrum ships static binaries + SUT/telemetry YAML.
7. **Operator takeaway:** Use AIPerf for NVIDIA profile exports and synthetic/ISL knobs; use metrum-ai-bench-cli for Hub prompt-library schedules, publishable SUT blocks, Prometheus-joined NDJSON, and companion promptfoo quality gates.

## Per-concurrency wall time (AIPerf)

| Concurrency | Seconds | rc |
|-------------|---------|----|
| 1 | 142 | 0 |
| 2 | 78 | 0 |
| 4 | 42 | 0 |
| 8 | 24 | 0 |
| 16 | 17 | 0 |
| 32 | 15 | 0 |
| 64 | 16 | 0 |

## Sources

- Hub dataset: https://huggingface.co/datasets/metrum-ai/prompt-library
- AIPerf: https://github.com/ai-dynamo/aiperf and https://docs.nvidia.com/aiperf/
- Metrum strategic NDJSON: `docs/TELEMETRY.md`, `docs/OUTPUT_SCHEMA.md`
- Bundle files: `mix-report.json`, `stdout-closed.json`, `stdout-open.json`, `promptfoo-*.json`, `aiperf/`
