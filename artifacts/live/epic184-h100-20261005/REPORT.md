<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Epic #184 live validation on 1x H100 PCIe (2026-10-05)

Metrum AI Bench CLI evidence bundle for tracking issue #233. It records the
live H100 run that validated epic #184 (issues #189 to #199) against real
serving stacks, and it backs findings #230, #231 and #232. This is a
validation run, not a publication under
[RESULTS_PUBLICATION_POLICY.md](../../../docs/RESULTS_PUBLICATION_POLICY.md):
the per-lane numbers prove that features work on real engines. They are not
tuned throughput claims.

Every number below is followed by the file in this bundle that proves it.
Paths are relative to this directory.

## Hardware and stack

| Item | Value | Proof |
|------|-------|-------|
| Provider | Shadeform, cloud `scaleway`, region `paris-france-1`, $3.30/h | `instance.json` |
| Instance | `98ca72ce-d075-49df-8afb-73dd972f6381`, created 2026-10-05T21:10:01Z, deleted 21:35:21Z (verified via `GET /instances/<id>/info`, `status=deleted`) | `instance.json` |
| GPU | 1x NVIDIA H100 PCIe, 79 GB, driver 580.126.20 | `llm/sut.json` (`gpu`, `driver_version`) |
| OS | Ubuntu 24.04, kernel `Linux 6.8.0-106-generic` | `llm/sut.json` (`host_os`) |
| LLM / VLM / ASR engine | `vllm/vllm-openai:v0.31.0` (released 2026-10-04) | `llm/sut.json`, `vlm/sut.json`, `asr/sut.json` (`runtime`) |
| ImageGen engine | `vllm/vllm-omni:v0.30.0 --omni` | `imagegen/sut.json`, `imagegen/serve.txt` |
| Telemetry | Metrum all-smi fork `v0.26.3-metrum.4` at `http://127.0.0.1:9090/metrics`, plus the engine `/metrics` page | `*/telemetry.yaml`, `llm/r1-plain.stderr` (probe lines) |
| Bench binaries | `/opt/metrum-bench/bin/*`, built from `main` at `6f150f3` (`ci(release): publish data-point counts (#202) (#229)`). Tip builds print the last release version, so `--version` reads `1.5.3`. | `llm/binary.txt`, `*/run.stdout` first line |
| Prompts (LLM) | Hugging Face `metrum-ai/prompt-library`, revision `0666f62e581b482838ae2e17b333ee36ff3d01b0`, config `sample`, profile `chat-short` v1, 160 rows extracted to 512 | `llm/prompts-report.json`, `llm/prompts.jsonl.gz`, `llm/sut.json` (`extra.prompt_*`) |

Models and serving flags (from each lane's SUT `runtime.config`; sources in SUT `extra.launcher_sources`):

| Lane | Model (revision) | Serve flags | Sources |
|------|------------------|-------------|---------|
| LLM | `Qwen/Qwen3-8B` (`b968826d9c46dd6066d109eabc6255188de91218`) | `--reasoning-parser qwen3 --max-model-len 32768` | [vLLM v0.31.0 release notes](https://github.com/vllm-project/vllm/releases/tag/v0.31.0), [vLLM reasoning outputs](https://docs.vllm.ai/en/latest/features/reasoning_outputs.html), [Qwen3-8B model card](https://huggingface.co/Qwen/Qwen3-8B) |
| VLM | `Qwen/Qwen3-VL-8B-Instruct` (`0c351dd01ed87e9c1b53cbc748cba10e6187ff3b`) | `--max-model-len 128000 --limit-mm-per-prompt.video 0 --async-scheduling --mm-processor-cache-gb 0`, `OMP_NUM_THREADS=1` | [vLLM v0.31.0 release notes](https://github.com/vllm-project/vllm/releases/tag/v0.31.0), [Qwen3-VL-8B-Instruct model card](https://huggingface.co/Qwen/Qwen3-VL-8B-Instruct) |
| ASR | `openai/whisper-large-v3-turbo` | plain vLLM speech-to-text, `--max-model-len 448` (vLLM-Omni ASR still blocked by [vllm-omni#5722](https://github.com/vllm-project/vllm-omni/issues/5722)) | [vLLM v0.31.0 release notes](https://github.com/vllm-project/vllm/releases/tag/v0.31.0), [vLLM OpenAI-compatible server](https://docs.vllm.ai/en/latest/serving/openai_compatible_server.html), [whisper-large-v3-turbo model card](https://huggingface.co/openai/whisper-large-v3-turbo) |
| ImageGen | `Tongyi-MAI/Z-Image-Turbo` | `vllm serve ... --omni`; requests use 9 steps, guidance 0.0, 1024x1024 | `imagegen/sut.json`, [Z-Image-Turbo model card](https://huggingface.co/Tongyi-MAI/Z-Image-Turbo), [vllm-omni v0.30.0](https://github.com/vllm-project/vllm-omni/releases/tag/v0.30.0) |

## What ran

Exact commands: `scripts/live_llm.sh` (LLM lane), `scripts/live_modalities.sh <vlm|asr|imagegen>`, and
`scripts/imagegen-rerun.sh` (see [Operator errors](#operator-errors-and-reruns)). All single runs used
`--sut <file> --require-sut`, `--telemetry` and `--ndjson`. The strategic sweeps ran without `--sut`, as their
stderr says, so they are validation data only.

| Run | Shape | Result | Proof |
|-----|-------|--------|-------|
| LLM R1 plain (thinking off) | c=16, 512 requests (32 warmup), `--max-tokens 64` | 480/480 ok, 25.860 req/s, 1450.471 completion tok/s, TTFT p50 0.031 s | `llm/r1-plain.stdout`, `llm/r1-plain.summary.json` |
| LLM R2 reasoning (thinking on) | c=8, 64 requests (8 warmup), `--max-tokens 2048` | 51/56 measured ok, 5 `no_output_token` (plus 1 in warmup), 512.428 completion tok/s, `reasoning_tokens_total` 22460 | `llm/r2-reasoning.stdout`, `llm/r2-reasoning.summary.json`, `llm/r2-reasoning.stderr` |
| LLM R3 strategic chat sweep | concurrency 1,2,4,8,16,32,64; 128 requests per stage | 0 errors; completion tok/s 101.3 to 4948.1; p95 0.632 to 0.892 s; knee c=32 | `llm/r3-sweep.stdout.json` |
| LLM R4 analyze.py | on R3 NDJSON | power, energy, J/token, utilization, KV, engine histograms, `kv_cache_util_at_knee` 0.0113 | `llm/r4-analyze.txt` (as run), `llm/r4-analyze-main-3beb329.txt` (rerun with the #231 fix) |
| LLM R5 selftest / preflight | dispatcher | selftest ok; preflight FAIL on `streaming_first_token` (finding #230) | `llm/r5-selftest.txt`, `llm/r5-preflight.txt` |
| VLM single run | c=8, 64 requests (8 warmup), `--max-tokens 128` | 56/56 ok, 6.576 req/s, 750.446 completion tok/s, TTFT p50 0.041 s | `vlm/run.stdout`, `vlm/run.summary.json` |
| VLM strategic sweep | `--kind vlm`, concurrency 1,2,4,8,16; 32 per stage | 0 errors; 0.940 to 12.366 req/s; p95 1.200 to 1.333 s; knee c=2 (finding #232) | `vlm/sweep.stdout.json` |
| ASR single run | c=8, 96 requests, `test-data/asr/` ground truth | 96/96 ok, 50.820 req/s, latency p50 0.146 s, WER 0.0 and CER 0.0 on all 96 requests | `asr/run.stdout`, `asr/run.summary.json`, `asr/run.jsonl.gz` (`modality_metrics`) |
| ASR strategic sweep | `--kind asr`, concurrency 1,2,4,8,16; 48 per stage | 0 errors; 19.52 to 63.15 req/s; CER 0.0 per stage; knee c=8 | `asr/sweep.stdout.json` |
| ImageGen single run | c=1, 8 requests, 9 steps, guidance 0.0 | 8/8 ok, 0.646 req/s, latency p50 1.536 s; 8 PNGs with 8 distinct sha256 | `imagegen/run.stdout`, `imagegen/run.summary.json`, `imagegen/images.sha256` |
| ImageGen strategic sweep | `--kind imagegen`, concurrency 1 to 5; 6 per stage | 0 errors; 6/6 distinct image digests per stage; knee c=4 | `imagegen/sweep.stdout.json` |

## Epic #184 issues: live result

`checks.txt` holds the output of `scripts/check_llm.py` and `scripts/check_mod.py`, regenerated on 2026-10-06.
The output is byte-identical on the raw source directories and on this redacted bundle (after gunzip).

| Issue | What it delivers | Live result | Proof |
|-------|------------------|-------------|-------|
| #189 | Observed concurrency never exceeds the cap | PASS. R1 `in_flight_max` 16 with cap 16; R2 8 with cap 8. ASR, VLM and imagegen runs also stay at their caps. | `checks.txt` (#189 lines), `llm/r1-plain.summary.json` (`observed_concurrency`) |
| #190 | Knee only with at least 5 sweep points | PASS. Every sweep reports `knee_detection.min_points=5` with `reason=null`: LLM 7 points, knee c=32; VLM, ASR, imagegen 5 points each. | `llm/r3-sweep.stdout.json`, `*/sweep.stdout.json` (`knee_detection`), `checks.txt` |
| #191 | Summaries for already-recorded per-request fields | PASS. `first_byte_s`, `queue_delay_s`, `first_reasoning_s`, `isl_tokens` and `osl_tokens` are present. n=480 for first byte, ISL and OSL in R1; `first_reasoning_s` n=51 in R2. `queue_delay_s` has n=0 because closed-loop runs have no pacing queue. | `checks.txt` (#191), `llm/r1-plain.summary.json`, `llm/r2-reasoning.summary.json` |
| #192 | Reasoning token counts | PASS. R2 `reasoning_tokens` is non-null on 51/51 successes, total 22460. `reasoning + visible == completion` on every request (0 mismatches). R1 (thinking off) reports a total of 0. | `checks.txt` (#192), `llm/r2-reasoning.summary.json` |
| #193 | Token totals and derived rates | PASS. R1 `prompt_tokens_total` 125904, `completion_tokens_total` 26923. `completion_tokens_total / window` = 1450.471, equal to `completion_tokens_per_second`. Input 6783.05 tok/s, total 8233.52 tok/s, time to second token p50 0.041 s, user tok/s p50 93.0. | `checks.txt` (#193), `llm/r1-plain.stdout` |
| #194 | HTTP phase trace | PASS. R1 `connection_reuse_rate` 1.0 with 0 fresh measured connections (limit 16); R2 0.961 with 2 fresh (limit 8). DNS, connect, bytes and chunk distributions are present. | `checks.txt` (#194), `llm/r1-plain.stdout` |
| #195 | Time-weighted concurrency and throughput | PASS. R1 `effective_concurrency.avg` 15.587 and max 16. Little's law check: sum(latency)/window = 15.5867. R2: 5.330 both ways. | `checks.txt` (#195), `llm/r1-plain.stdout` |
| #196 | `--telemetry` and `--ndjson` in llm, vlm, asr, imagegen | PASS. All four binaries wrote `run`, `telemetry`, `request`, `stage` and `summary` rows with 0 scrape errors and 0 dropped rows. Data-log `send_offset_s` joins NDJSON `t_sent_ns` within 4e-6 ns on every row. Engine series: VLM 100, ASR 100, imagegen 60 (`vllm_omni:`). | `checks.txt` (#196), `*/run.summary.json` (`telemetry`), `*/run.ndjson.gz` |
| #197 | Strategic sweeps for VLM, ASR, image generation | PASS. `config.kind` is `vlm`, `asr` and `imagegen`; each sweep has 5 points with error rate 0, and each NDJSON ends with `summary`, `partial=false`. ASR stages carry WER/CER/RTFx; imagegen stages carry image digests (6 distinct of 6). | `checks.txt` (#197), `*/sweep.stdout.json`, `*/sweep.ndjson.gz` |
| #198 | Default telemetry YAML includes | PASS. Probes matched all-smi 65 and vLLM 154 series. The LLM NDJSON holds 6 vLLM histograms (TTFT, ITL, E2E, queue, prefill, decode) and 4 prefix-cache counters. It has no per-core CPU rows and no `all_smi_process_*` rows, since process rows are off by default. | `llm/r1-plain.stderr` (probe), `checks.txt` (#198 lines), `llm/r1-plain.ndjson.gz` (prefix-cache counters), `llm/telemetry.yaml` |
| #199 | analyze.py reads what we store | PASS with finding. R4 computed per-stage power, energy, J/output token, `gpu_util_mean`, `sm_active_p50`, `sm_occupancy_p50`, `tensor_active_p50`, `hollow_util_mean`, `kv_cache_util_mean`, `preemptions_delta`, engine histogram p50/p95, and `kv_cache_util_at_knee` 0.0113 at knee c=32. Prefill and queue time read exactly 0.15 / 0.285 s in every stage, which led to finding #231. | `llm/r4-analyze.txt` |

Note on `checks.txt`: the orchestrator's original `check_llm.py` read the telemetry series name from `name`,
but `telemetry.v1` rows store it in `metric`, so its #196 `series_names` and #198 lines came out empty. The
bundled `scripts/check_llm.py` is fixed to read `metric`, and it no longer counts `all_smi_cpu_core_count` (a
host count gauge) as a per-core series. No other check logic changed.

## Findings filed

| Issue | State (2026-10-06) | Finding | Proof |
|-------|--------------------|---------|-------|
| #230 | open | `preflight` reports `streaming_first_token FAIL ... no output token` against a healthy Qwen3-8B with `--reasoning-parser qwen3`. The 8-token probe is spent in `reasoning_content`. | `llm/r5-preflight.txt` |
| #231 | closed by #234 (`3beb329`) | analyze.py interpolated engine histogram percentiles below the first bucket bound from 0, which printed fake values (prefill and queue 0.15 / 0.285 s in every stage). After the fix it reports bounds (`<=0.3`). | `llm/r4-analyze.txt` vs `llm/r4-analyze-main-3beb329.txt` |
| #232 | open | VLM sweep knee at c=2 although the curve is nearly linear: throughput 0.94 to 12.37 req/s, p95 1.200 to 1.333 s (+11%). | `vlm/sweep.stdout.json` (`knee_detection`, `points`) |

Other observations from this run (no new issue filed):

- **R2 errors are the token cap, as expected.** 5 measured and 1 warmup requests hit `max_tokens` 2048 while still reasoning
  (`no_output_token`). See [docs/REASONING_MODELS.md](../../../docs/REASONING_MODELS.md). Proof: `llm/r2-reasoning.stdout`, `llm/r2-reasoning.stderr`.
- **ImageGen sweep c=5 throughput is overstated.** The point reports 0.999 req/s, but its 6 measured requests span
  8.97 s in `imagegen/sweep.ndjson.gz`, about 0.669 req/s, in line with the other stages (0.661 to 0.680).
  The c=1 to c=4 points match NDJSON-derived values. Stage-window accounting is tracked in the open issue #224.
  This bundle does not confirm that #224 is the root cause. Proof: `imagegen/sweep.stdout.json`, `imagegen/sweep.ndjson.gz`.
- **The engine serves one image at a time.** In the imagegen sweep, throughput stays near 0.67 req/s at every
  concurrency while p95 grows about linearly (1.52 s at c=1 to 7.46 s at c=5), so requests queue on the server.
  The imagegen numbers are smoke-scale and not a throughput claim. Proof: `imagegen/sweep.stdout.json`.

## Operator errors and reruns

All three were operator mistakes in the ad hoc run scripts, not product defects. Each was fixed and rerun on the
same instance before teardown.

1. **Duplicate `vllm` telemetry source (about 21:18Z).** The first `live_llm.sh` merged
   `docs/telemetry/examples/all-smi.yaml` (which already includes a `vllm` source) with `vllm.yaml` without
   deduplicating. The binary
   correctly refused to start: `Error: duplicate telemetry source name vllm`. Fix: the script dedupes sources by
   name before merging (the inline Python in `scripts/live_llm.sh`). Rerun: R1 to R5 as committed.
2. **Base URL instead of the endpoint (about 21:18Z).** `--url http://127.0.0.1:8000` was passed to `llm`, which
   expects the full endpoint. The server answered `404 Not Found {"detail":"Not Found"}`. Fix: `--url .../v1/chat/completions`.
   Every committed script passes the full endpoint (`/v1/chat/completions`, `/v1/audio/transcriptions`,
   `/v1/images/generations`). Rerun: the LLM lane as committed.
3. **`--guidance` instead of `--guidance-scale` (about 21:34Z).** `scripts/live_modalities.sh imagegen` passed
   `--guidance 0.0`. clap rejected it (`tip: a similar argument exists: '--guidance-scale'`), so the single
   imagegen run did not execute. The imagegen sweep in the same script passes `guidance_scale` via
   `--extra-body-json` and was unaffected. Rerun: the single run was repeated by hand with `--guidance-scale 0.0`
   (`scripts/imagegen-rerun.sh`). `scripts/live_modalities.sh` is committed as run, so it still contains the
   rejected flag. `imagegen/run.jsonl.gz` records `body_template.guidance_scale: 0.0`.

## SKUs not run

RTX PRO 6000 Blackwell Server Edition and H200 were requested but **not run**. Shadeform listed both GPU types
(`RTXPro6000`, `H200`) with no available offer at 2026-10-05T21:08:39Z or at 2026-10-06T00:02:01Z. No instance
could be created, so this bundle has no results for either SKU. Proof: `availability.txt`.

## Bundle contents

| Path | Content |
|------|---------|
| `REPORT.md` | This report |
| `checks.txt` | `check_llm.py` (fixed, see note above) and `check_mod.py` output |
| `instance.json` | Shadeform instance record (IP and SSH host key omitted) |
| `availability.txt` | RTX PRO 6000 / H200 availability checks |
| `scripts/` | `live_llm.sh`, `live_modalities.sh`, `imagegen-rerun.sh` (exact commands run on the GPU host) and `check_llm.py` (series-key fix applied), `check_mod.py` |
| `llm/` | `binary.txt`, `serve-sut.json` (launcher SUT), `sut.json` (with prompt stamp), `telemetry.yaml`, `prompts-report.json`, `prompts.jsonl.gz`; for R1 and R2: `.stdout`, `.stderr`, `.summary.json` (final `summary.v3` line), `.jsonl.gz` (data log), `.ndjson.gz`; R3 `r3-sweep.stdout.json`, `.stderr`, `.ndjson.gz`; R4 analyze outputs; R5 selftest and preflight |
| `vlm/`, `asr/`, `imagegen/` | `sut.json`, `models.json`, `telemetry.yaml`, `serve.txt` (launcher output; renamed from `serve.log`, which `.gitignore` excludes), `run.stdout`, `run.stderr`, `run.summary.json`, `run.jsonl.gz`, `run.ndjson.gz`, `sweep.stdout.json`, `sweep.stderr`, `sweep.ndjson.gz` |
| `imagegen/images.sha256` | sha256 and size of the 8 generated PNGs. The PNGs are not committed. |

All NDJSON and data logs are included in full, gzip'd. The bundle is about 2.5 MB, so no sampling was needed.
Decompress with `gunzip -k`.

### Redaction

Before commit, these were rewritten in every file: the GPU UUID to `GPU-REDACTED`, the host name to
`redacted-host`, and home-directory and temporary-directory paths to `<HOME>`, `<TMP>` or `<SCRATCH>`. The instance IP did not
appear in any run file; it is omitted from `instance.json`. No key material was present. The scripts pass
`--api-key dummy`, and the Shadeform key was never written to any file. The Shadeform instance id is kept.
Redaction only replaces strings, so every JSON and NDJSON line still parses, and the check scripts reproduce
their output exactly.

## Reproduce the checks

```bash
B=artifacts/live/epic184-h100-20261005
L=$(mktemp -d)/live
for m in llm vlm asr imagegen; do mkdir -p $L/epic184-$m; done
for f in r1-plain r2-reasoning; do
  gunzip -c $B/llm/$f.jsonl.gz > $L/epic184-llm/$f.jsonl
  gunzip -c $B/llm/$f.ndjson.gz > $L/epic184-llm/$f.ndjson
done
gunzip -c $B/llm/r3-sweep.ndjson.gz > $L/epic184-llm/r3-sweep.ndjson
cp $B/llm/r3-sweep.stdout.json $L/epic184-llm/
for m in vlm asr imagegen; do
  gunzip -c $B/$m/run.jsonl.gz > $L/epic184-$m/run.jsonl
  for k in run sweep; do gunzip -c $B/$m/$k.ndjson.gz > $L/epic184-$m/$k.ndjson; done
  cp $B/$m/sweep.stdout.json $L/epic184-$m/
done
python3 $B/scripts/check_llm.py $L/epic184-llm
python3 $B/scripts/check_mod.py $L
python3 docs/queries/analyze.py $L/epic184-llm/r3-sweep.ndjson $L/epic184-llm/r3-sweep.stdout.json
```
