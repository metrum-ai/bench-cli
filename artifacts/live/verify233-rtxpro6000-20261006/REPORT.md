<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# #233 fix verification on 1x RTX PRO 6000 Blackwell Server Edition (2026-10-06)

Metrum AI Bench CLI evidence bundle for tracking issue #233. It records the live run on 2026-10-06 that
re-verified the eight #233 fixes (#226, #224, #232, #227, #230, #231, #216, #242) on a real GPU after
they merged. It follows the layout of the H100 bundle
[`epic184-h100-20261005`](../epic184-h100-20261005/REPORT.md), which found most of these issues. This
is a validation run, not a publication under
[RESULTS_PUBLICATION_POLICY.md](../../../docs/RESULTS_PUBLICATION_POLICY.md). The numbers show that
the fixes behave correctly on real engines. They are not tuned throughput claims, and RTX PRO 6000 numbers
are not comparable one-to-one with the H100 bundle.

Every number below is followed by the file in this bundle that proves it. Paths are relative to this
directory.

## Summary

- `checks.txt` (the output of `scripts/check_verify.py`): **11 PASS, 0 FAIL**. Rerunning the checker on
  this redacted bundle gives byte-identical output, and `docs/queries/analyze.py` at `d43d9bc` reproduces
  `llm/analyze.txt` byte for byte (see [Reproduce the checks](#reproduce-the-checks)).
- **Six of the eight fixes are verified live with a number that proves them: #226, #224, #232, #230,
  #231, #216.** A further check of #224 found that the bundled checker's own #224 test was vacuous. An
  independent recompute in `recompute_224.txt` closes that gap and passes on every stage of all four
  sweeps. See [Checker note](#checker-note-the-224-check-in-check_verifypy-is-vacuous).
- **#227 is only partly verified.** The telemetry abort wrote a full `summary.v3` and a `partial=true`
  NDJSON summary row, with exit 1. But the single SIGINT most likely arrived after the process had
  exited, so this run does not prove "Ctrl-C during the drain after a telemetry abort keeps the summary".
  See [#227 timing](#227-timing-the-sigint-most-likely-missed-the-drain).
- **#242 works on a real engine**, with 14/14 requests carrying a 2.2 MB 2048x2048 image. This run shows
  that the code path works, but it cannot isolate the latency-window change itself. The before/after
  timing proof is the e2e test in PR #246.
- **H200 was not run**: Shadeform had no H200 offer at any check.

## Hardware and stack

| Item | Value | Proof |
|------|-------|-------|
| Provider | Shadeform, cloud `massedcompute`, region `beltsville-usa-1`, $2.19/h (`hourly_price` 219 cents) | `instance.json` |
| Instance | `5bf3ef99-a285-42f9-9d8c-c6b2945d0814` (`metrum-widen-20261006-015435`), created 2026-10-06T01:54:36Z, deleted 02:26:39Z | `instance.json`, `instance-deleted.json` |
| GPU | 1x NVIDIA RTX PRO 6000 Blackwell Server Edition. all-smi reports `memory_total_bytes` 102641958912 (95.6 GiB, the 96 GB SKU; SUT declares 95). Power limit 600 W (current, default and max), min 300 W. VBIOS 98.02.81.00.01 | `llm/sut.json` (`gpu`), `llm/f226.ndjson.gz` (`all_smi_gpu_memory_total_bytes`, `all_smi_gpu_power_limit_*_watts`, `vbios_version` label) |
| Driver | NVIDIA 580.126.09 | `*/sut.json` (`driver_version`) |
| OS | Ubuntu 22.04 (glibc 2.35), kernel `Linux 6.8.0-90-generic`. The dispatcher `metrum-ai-bench-cli` needs glibc 2.39, so `preflight` ran in an `ubuntu:24.04` container on the host network. The per-modality binaries ran on the host. | `llm/sut.json` (`host_os`), `scripts/verify_fixes.sh` (`DISPATCH=`) |
| LLM / VLM / ASR engine | `vllm/vllm-openai:v0.31.0` (released 2026-10-04) | `llm/sut.json`, `vlm/sut.json`, `asr/sut.json` (`runtime`, `extra.image`) |
| ImageGen engine | `vllm/vllm-omni:v0.30.0`, `vllm serve ... --omni` | `imagegen/sut.json`, `imagegen/serve.txt` |
| Telemetry | Metrum all-smi fork at `http://127.0.0.1:9090/metrics` (65 series matched), plus the engine `/metrics` page (vLLM 154 series on the LLM lane; VLM 385, ASR 345 and imagegen 78 to 218 with the `engine` source). The fork version `v0.26.3-metrum.4` comes from the operator record. No bundled file stamps it. | `*/telemetry.yaml`, `llm/f226.stderr`, `*/run.stderr` (probe lines) |
| Bench binaries | `/opt/metrum-bench/bin/*`, built from `main` at `d43d9bc` (`fix(vlm,asr,strategic): build request bodies before the send time (#242) (#246)`), which contains all eight fix PRs. Tip builds print the last release version, so `--version` reads `1.5.3`. The commit comes from the operator's build record. No bundled file stamps it. | `llm/binary.txt`, `llm/dispatcher-version.txt`, `*/run.stdout` first line |
| Prompts (LLM) | Hugging Face `metrum-ai/prompt-library`, revision `0666f62e581b482838ae2e17b333ee36ff3d01b0`, config `sample`, profile `chat-short` v1, 160 rows selected (preferred 512) | `llm/prompts-report.json`, `llm/prompts.jsonl.gz` |

Models and serving flags come from each lane's SUT `runtime.config`. Sources are in SUT `extra.launcher_sources`:

| Lane | Model (revision) | Serve flags | Sources |
|------|------------------|-------------|---------|
| LLM | `Qwen/Qwen3-8B` (`b968826d9c46dd6066d109eabc6255188de91218`) | `--reasoning-parser qwen3 --max-model-len 32768` | [vLLM v0.31.0 release notes](https://github.com/vllm-project/vllm/releases/tag/v0.31.0), [vLLM reasoning outputs](https://docs.vllm.ai/en/latest/features/reasoning_outputs.html), [Qwen3-8B model card](https://huggingface.co/Qwen/Qwen3-8B) |
| VLM | `Qwen/Qwen3-VL-8B-Instruct` (`0c351dd01ed87e9c1b53cbc748cba10e6187ff3b`) | `--max-model-len 128000 --limit-mm-per-prompt.video 0 --async-scheduling --mm-processor-cache-gb 0`, `OMP_NUM_THREADS=1` | [vLLM v0.31.0 release notes](https://github.com/vllm-project/vllm/releases/tag/v0.31.0), [Qwen3-VL-8B-Instruct model card](https://huggingface.co/Qwen/Qwen3-VL-8B-Instruct) |
| ASR | `openai/whisper-large-v3-turbo` (`41f01f3fe87f28c78e2fbf8b568835947dd65ed9`) | plain vLLM speech-to-text, `--max-model-len 448` (vLLM-Omni ASR still blocked by [vllm-omni#5722](https://github.com/vllm-project/vllm-omni/issues/5722)) | [vLLM v0.31.0 release notes](https://github.com/vllm-project/vllm/releases/tag/v0.31.0), [vLLM OpenAI-compatible server](https://docs.vllm.ai/en/latest/serving/openai_compatible_server.html), [whisper-large-v3-turbo model card](https://huggingface.co/openai/whisper-large-v3-turbo) |
| ImageGen | `Tongyi-MAI/Z-Image-Turbo` (`f332072aa78be7aecdf3ee76d5c247082da564a6`) | `vllm serve ... --omni`; requests use 9 steps, guidance 0.0, 1024x1024 | [vllm-omni v0.30.0 CUDA install](https://github.com/vllm-project/vllm-omni/blob/v0.30.0/docs/getting_started/installation/gpu/cuda.inc.md), [vllm-omni image generation API](https://docs.vllm.ai/projects/vllm-omni/en/latest/serving/image_generation_api/), [Z-Image-Turbo model card](https://huggingface.co/Tongyi-MAI/Z-Image-Turbo) |

## What ran

The exact commands are in `scripts/verify_fixes.sh` (LLM lane, on the GPU host with the LLM server up) and
`scripts/live_modalities.sh <vlm|asr|imagegen>`. Runs whose numbers describe the SUT (`f226`, the three
modality single runs) used `--sut <file> --require-sut`. The strategic sweeps, `f227` and the VLM big-image
run ran without `--sut`, as their stderr says, so they are validation data only.

| Run | Shape | Result | Proof |
|-----|-------|--------|-------|
| `f230` preflight, thinking on / off | dispatcher `preflight` in `ubuntu:24.04` | both `preflight: ok`, exit 0 | `llm/f230-preflight-thinking.txt`, `llm/f230-preflight-nothink.txt` |
| `f226` warmup barrier | c=16, 256 requests with only 4 warmup (warmup < concurrency), `--max-tokens 64`, thinking off | 252/252 ok, 21.325 req/s, 1202.048 completion tok/s, TTFT p50 0.038 s | `llm/f226.stdout`, `llm/f226.summary.json` |
| `f227` telemetry abort | c=4, `--require-telemetry --require-telemetry-failures 3`; all-smi killed at about 8 s, one SIGINT at about 14 s | exit 1, `summary.v3` written (46/46 measured ok, `partial: true`) | `llm/f227.exit`, `llm/f227.stderr`, `llm/f227.summary.json` |
| LLM strategic sweep | concurrency 1,2,4,8,16,32,64; 128 per stage, 16 warmup; `--csv` | 0 errors; 1.495 to 65.351 req/s; p95 0.768 to 1.065 s; knee c=16 | `llm/llm-sweep.stdout.json`, `llm/llm-sweep.csv` |
| analyze.py | on the LLM sweep NDJSON | per-stage power 320.5 to 362.0 W mean, J/output token 4.022 (c=1) to 0.111 (c=64), `kv_cache_util_at_knee` 0.00726 at c=16 | `llm/analyze.txt`, `llm/analyze.json` |
| #216 launcher gating | `scripts/live/serve/llm.sh sut`, offline | exits 1 / 0 / 0 as expected | `llm/f216.txt` |
| VLM single run | c=8, 64 requests (8 warmup), `--max-tokens 128` | 56/56 ok, 5.519 req/s, 644.884 completion tok/s, TTFT p50 0.049 s | `vlm/run.stdout`, `vlm/run.summary.json` |
| VLM big image (#242) | 2048x2048 PNG (1,668,494 bytes), c=4, 16 requests (2 warmup), `--max-tokens 64` | 14/14 ok, bytes sent p50 2,225,076, first byte p50 0.128 s, TTFT p50 0.182 s | `vlm/big.stdout`, `vlm/big.summary.json`, `vlm/big.png.sha256` |
| VLM strategic sweep | `--kind vlm`, concurrency 1,2,4,8,16; 32 per stage; `--csv` | 0 errors; 0.700 to 10.503 req/s; p95 1.416 to 1.569 s; no knee (`no_bend`) | `vlm/sweep.stdout.json`, `vlm/sweep.csv` |
| ASR single run | c=8, 96 requests, `test-data/asr/` ground truth | 96/96 ok, 60.313 req/s, latency p50 0.110 s, WER 0.0 and CER 0.0 on all 96 requests | `asr/run.stdout`, `asr/run.summary.json`, `asr/run.jsonl.gz` (`modality_metrics`) |
| ASR strategic sweep | `--kind asr`, concurrency 1,2,4,8,16; 48 per stage; `--csv` | 0 errors; 25.22 to 78.09 req/s; p95 0.053 to 0.362 s; knee c=2 (saturation) | `asr/sweep.stdout.json`, `asr/sweep.csv` |
| ImageGen single run | c=1, 8 requests, 9 steps, guidance 0.0 | 8/8 ok, 0.599 req/s, latency p50 1.624 s; 8 PNGs with 8 distinct sha256 | `imagegen/run.stdout`, `imagegen/run.summary.json`, `imagegen/images.sha256` |
| ImageGen strategic sweep | `--kind imagegen`, concurrency 1 to 5; 6 per stage; `--csv` | 0 errors; flat 0.615 to 0.632 req/s; p95 1.630 to 7.963 s; knee c=1 (saturation) | `imagegen/sweep.stdout.json`, `imagegen/sweep.csv` |

## #233 fixes: live result

`checks.txt` is the as-run output of `scripts/check_verify.py`. It reproduces byte for byte on this bundle.
Each table gives the fix, the live result, the number that proves it, the file, and the merged PR.

### #226 warmup is a barrier before measured requests (PR #236, `e3c14ef`)

| Live result | Number that proves it | File | Merged PR |
|-------------|-----------------------|------|-----------|
| PASS. With 4 warmup requests at c=16, no measured request starts before the last warmup request finishes. Observed concurrency counts only measured requests. | Last warmup end 0.801018 s, first measured send 0.801172 s (gap 0.154 ms, never negative). `acquire_count` 252 = 252 measured requests. `in_flight_max` 16 with cap 16. Since only 4 connections were warmed, 12 measured requests opened fresh connections, as expected. | `checks.txt` (#226), `llm/f226.jsonl.gz` (`phase`, `send_offset_s`, `latency_s`), `llm/f226.summary.json` (`observed_concurrency`) | #236 |

### #224 stage window ends at the latest completion (PR #237, `c00c121`)

| Live result | Number that proves it | File | Merged PR |
|-------------|-----------------------|------|-----------|
| PASS (by the independent recompute). Every strategic point's `throughput` equals successes / (latest successful completion minus earliest measured send), computed from the per-request CSV. | Max relative mismatch **4.35e-15** over 22 stages (LLM 7, VLM 5, ASR 5, imagegen 5); every stage measured 100% successes. The `checks.txt` #224 line (0.00e+00 over 7 stages) is vacuous; see the checker note below. | `recompute_224.txt`, `scripts/recompute_224.py`, `*/sweep.csv`, `llm/llm-sweep.csv`, `*/sweep.stdout.json` | #237 |

**Before and after, against the H100 bundle.** This is the case that led to the issue. In
`epic184-h100-20261005` the imagegen c=5 point reported **0.999 req/s**, but its 6 measured requests spanned
8.97 s in the NDJSON (about 0.669 req/s), out of line with c=1 to c=4 (0.661 to 0.680). In this run every
imagegen stage reports a value that matches the CSV recompute, and the curve is flat. A one-image-at-a-time
server should behave this way:

| Imagegen sweep point | c=1 | c=2 | c=3 | c=4 | c=5 |
|---|---|---|---|---|---|
| H100 2026-10-05, reported req/s (before #237) | 0.661 | 0.680 | 0.678 | 0.673 | **0.999** (NDJSON-derived about 0.669) |
| RTX PRO 6000 2026-10-06, reported req/s (after #237) | 0.615 | 0.632 | 0.631 | 0.629 | **0.628** |
| RTX PRO 6000, CSV recompute | 0.615 | 0.632 | 0.631 | 0.629 | 0.628 (window 9.558 s) |

Proof: `../epic184-h100-20261005/imagegen/sweep.stdout.json`, `imagegen/sweep.stdout.json`, `recompute_224.txt`.
The GPUs differ, so compare only the shape: before the fix the c=5 point jumped by 48%, and after it the
curve is flat. Do not compare the absolute values across the two GPUs.

### #232 no knee on sweeps without a p95 bend (PR #239, `3b9998c`)

| Live result | Number that proves it | File | Merged PR |
|-------------|-----------------------|------|-----------|
| PASS. The nearly linear VLM sweep no longer reports a knee. Sweeps with a real p95 bend or saturation still report one, with the method named. | **VLM**: `reason: no_bend`, `p95_rise` 0.1078 (+10.8%, below the 0.2 threshold), throughput 0.700 to 10.503 req/s, near-linear. The H100 run reported knee c=2 on a comparable curve (+11%). **LLM**: knee c=16, `method: kneedle`, `p95_rise` 0.388. **ASR**: knee c=2, `method: saturation`, `saturated_index` 2 (c=4), `p95_rise` 5.83; throughput 25.2, 40.1, 56.4, 69.1, 78.1 req/s. **ImageGen**: knee c=1, `method: saturation`, `saturated_index` 1 (c=2), `p95_rise` 3.89; flat 0.615 to 0.632 req/s while p95 grows from 1.63 s to 7.96 s. | `checks.txt` (#232 lines), `*/sweep.stdout.json` (`knee_detection`, `points`), `vlm/sweep.stderr` (`note: no knee ...`) | #239 |

### #227 telemetry follow-ups: signal handling after a telemetry abort (PR #241, `7b4686d`)

| Live result | Number that proves it | File | Merged PR |
|-------------|-----------------------|------|-----------|
| PARTIAL. The `--require-telemetry` mid-run abort path is verified: new requests stopped, in-flight work drained, a full `summary.v3` was written, and the NDJSON ends with a `partial: true` summary row. The "one Ctrl-C during that drain keeps the summary" half was **not exercised**, because the SIGINT most likely arrived after exit (see below). | Exit 1 (not 130). Data log ends with `summary.v3`, `partial: true`, 46/46 measured ok. NDJSON: 3 `scrape_error` rows at t0+8.00, 8.50 and 9.00 s, then `summary` with `partial: true`, `request_rows` 50, `dropped_telemetry_rows` 0. stderr: `telemetry source all-smi failed 3 consecutive scrapes (require-telemetry); stopping new requests and draining in-flight work`. all-smi was restarted afterwards (89 `all_smi_` lines). | `checks.txt` (#227), `llm/f227.exit`, `llm/f227.stderr`, `llm/f227.summary.json`, `llm/f227.ndjson.gz`, `llm/allsmi-restarted.txt` | #241 |

#### #227 timing: the SIGINT most likely missed the drain

`scripts/verify_fixes.sh` kills all-smi 8 s after launch and sends SIGINT 6 s later (about 14 s after
launch). The NDJSON gives `t0_wall` 02:10:08.206Z. The third failed scrape, which triggers the abort, came at
t0+9.00 s (02:10:17.2Z). The last in-flight request finished at t0+9.67 s, and the measure stage ended at
the same instant (`stage.t_end_ns` 9671607735). So the drain was over about 4 s before the SIGINT was sent,
and the process most likely exited before the signal arrived. The exit code cannot tell the cases apart:
1 is what the fixed code returns when a signal comes during the drain, and also what a run that never saw
the signal returns. The pre-fix bug showed up only as exit 130 with no summary, and that did not happen.
The console line from `kill` was not kept, so this bundle cannot confirm whether the signal was delivered.
**Recommended follow-up:** rerun with a long drain (for example `--max-tokens 2048` with thinking on) and
send SIGINT about 0.5 s after the abort line. Then check that the exit is not 130 and that `summary.v3` is
present. PR #241's unit and e2e tests remain the evidence for that half.

### #230 preflight streaming probe on thinking models (PR #244, `2c010ca`)

| Live result | Number that proves it | File | Merged PR |
|-------------|-----------------------|------|-----------|
| PASS. Against Qwen3-8B with `--reasoning-parser qwen3` and thinking on, `streaming_first_token` now passes on a reasoning delta. In the H100 run it failed with `no output token`. | Thinking on: `streaming_first_token PASS first token was reasoning in 57 ms (no visible content within the probe's max_tokens)`, `preflight: ok`, exit 0. Thinking off: `PASS first visible token in 16 ms`, exit 0. Before: `../epic184-h100-20261005/llm/r5-preflight.txt` (FAIL). | `checks.txt` (#230), `llm/f230-preflight-thinking.txt`, `llm/f230-preflight-nothink.txt` | #244 |

### #231 analyze.py histogram percentiles below the first bucket (PR #234, `3beb329`)

| Live result | Number that proves it | File | Merged PR |
|-------------|-----------------------|------|-----------|
| PASS. Prefill and queue time below the first bucket bound print as a bound, not as interpolated values. | `vllm:request_prefill_time_seconds` and `vllm:request_queue_time_seconds` read `<=0.3` / `<=0.3` (p50 / p95) in all 7 stages. The fake `0.15` / `0.285` from the H100 run is gone. Other histograms still print values, for example TTFT p50 0.0254 s at c=1. | `checks.txt` (#231), `llm/analyze.txt` (`## engine histograms`) | #234 |

### #216 launcher gating on IMAGE / SERVE_ARGS overrides (PR #235, `4846376`)

| Live result | Number that proves it | File | Merged PR |
|-------------|-----------------------|------|-----------|
| PASS. `scripts/live/serve/llm.sh sut` refuses an `IMAGE` override without fresh notes and sources, and accepts it with them. | Override without notes: exit 1. With `SUT_NOTES_OVERRIDE` and `SOURCES_OVERRIDE`: exit 0. Default: exit 0. | `checks.txt` (#216), `llm/f216.txt` | #235 |

### #242 request bodies built before the send time (PR #246, `d43d9bc`)

| Live result | Number that proves it | File | Merged PR |
|-------------|-----------------------|------|-----------|
| PASS (path works live). The prebuilt-bytes VLM path sends a 2048x2048 inline image to vLLM 0.31.0 without error. This run has no pre-fix baseline on the same host, so it cannot measure how much encoding time left the window. That timing proof is the e2e test `tests/e2e_payload_window.rs` in PR #246 (on `origin/main` sources 1.144 s against a 0.271 s reference, FAIL; on the fix branch 0.285 s against 0.279 s, pass). | 14/14 measured ok at c=4. Bytes sent p50 2,225,076 per request. First byte p50 0.128 s, TTFT p50 0.182 s (0.049 s with the small test image). Prompt tokens 57,792 in total, about 4,128 per request. The higher TTFT matches server-side prefill of a much larger image. | `checks.txt` (#242), `vlm/big.summary.json`, `vlm/big.stdout`, `vlm/run.summary.json`, `vlm/big-prompts.jsonl`, `vlm/big.png.sha256` | #246 |

### Checker note: the #224 check in `check_verify.py` is vacuous

`scripts/check_verify.py` is committed exactly as run, so that `checks.txt` reproduces. While reviewing it,
I found that its #224 block matches CSV rows to sweep points with `r.get("load") or r.get("stage_load")`.
The strategic CSV names that column `stage` (see the header of `llm/llm-sweep.csv`). Every point therefore
matched zero rows and was skipped, and `worst` stayed at its initial 0.0. The `#224 ... 0.00e+00 over 7
stages` PASS line is real output, but it tested nothing. `scripts/recompute_224.py` does the same recompute
keyed on `stage`, also applies it to the three modality sweeps, and passes (`recompute_224.txt`, max 4.35e-15).
The other `check_verify.py` checks read real fields. The four modality sweep lines always print PASS,
because they record the `knee_detection` and points for review rather than assert on them. Their
assertions are in the #232 table above.

## Modality sweep knees

| Sweep | Points (concurrency: req/s, p95 s) | `knee_detection` | Reading |
|-------|-----------------------------------|------------------|---------|
| LLM chat | 1: 1.495, 0.768 / 2: 2.908, 0.793 / 4: 5.871, 0.773 / 8: 11.236, 0.786 / 16: 20.762, 0.818 / 32: 37.808, 0.914 / 64: 65.351, 1.065 | index 4 (c=16), `kneedle`, `p95_rise` 0.388 | Real p95 bend after c=16 |
| VLM | 1: 0.700, 1.454 / 2: 1.357, 1.493 / 4: 2.848, 1.416 / 8: 5.586, 1.467 / 16: 10.503, 1.569 | `no_bend`, `p95_rise` 0.108 (+10.8%) | Near-linear, so no knee (the #232 fix) |
| ASR | 1: 25.22, 0.053 / 2: 40.06, 0.073 / 4: 56.38, 0.103 / 8: 69.10, 0.164 / 16: 78.09, 0.362 | index 1 (c=2), `saturation`, `saturated_index` 2, `p95_rise` 5.83 | Saturated from c=4 (`saturated_index` 2); p95 rises 5.8x from its minimum by c=16 |
| ImageGen | 1: 0.615, 1.630 / 2: 0.632, 3.190 / 3: 0.631, 4.776 / 4: 0.629, 6.361 / 5: 0.628, 7.963 | index 0 (c=1), `saturation`, `saturated_index` 1, `p95_rise` 3.89 | Server runs one image at a time; throughput is flat at about 0.63 req/s, so extra concurrency only queues |

Proof: `llm/llm-sweep.stdout.json`, `vlm/sweep.stdout.json`, `asr/sweep.stdout.json`, `imagegen/sweep.stdout.json`.

## Operator notes and caveats

1. **The VLM and ASR SUT `notes` say "H100".** `scripts/live_modalities.sh` was reused from the H100 run
   with its `SUT_NOTES_OVERRIDE` strings unchanged. As a result, `vlm/sut.json` and `asr/sut.json` (and the
   SUT copies in their `run.summary.json` and `run.ndjson.gz` run rows) end with "epic #184 live validation
   on 1x H100 PCIe". Their `gpu`, `driver_version` and `host_os` fields are correct (RTX PRO 6000, 580.126.09,
   6.8.0-90). The evidence is committed as recorded and not rewritten. Read the hardware from `gpu`, not
   `notes`. The `epic184-*` scenario names in those runs come from the same reuse. The LLM lane SUT is
   correct.
2. **#227 SIGINT timing.** See [#227 timing](#227-timing-the-sigint-most-likely-missed-the-drain).
3. **#224 checker.** See [Checker note](#checker-note-the-224-check-in-check_verifypy-is-vacuous).
4. **Unrecorded provenance.** The build commit `d43d9bc` and all-smi `v0.26.3-metrum.4` come from the
   operator's record. No file in this bundle stamps them. Tip builds print `1.5.3`.

## GPUs not run

**H200: not run.** Shadeform had no H200 offer at 2026-10-05T21:08:39Z, 2026-10-06T00:02:01Z (both in the H100
bundle), 2026-10-06T01:43Z or 01:54:23Z (this run, per the orchestrator's record). The RTX PRO 6000 that the
H100 bundle could not get became available at 01:54:23Z and is the GPU in this bundle. Proof: `availability.txt`.

## Cost and deletion

| Item | Value | Proof |
|------|-------|-------|
| Instance lifetime | 01:54:36Z to 02:26:39Z, 32 min 3 s | `instance.json` (`created_at_utc`, `deleted_at_utc`) |
| Cost | 0.534 h x $2.19/h = **about $1.17** | `instance.json` (`hourly_price` 219) |
| Deletion proof | `GET /instances/<id>/info` returned `status: deleted`, `deleted_at` 2026-10-06T02:26:39.427648Z | `instance-deleted.json` |

## Bundle contents

| Path | Content |
|------|---------|
| `REPORT.md` | This report |
| `checks.txt` | `check_verify.py` output as run (11 PASS, 0 FAIL); reproduces byte for byte on this bundle |
| `recompute_224.txt` | `recompute_224.py` output on all four sweeps (corrected #224 check) |
| `availability.txt` | Shadeform availability checks for RTX PRO 6000 and H200 |
| `instance.json`, `instance-deleted.json` | Shadeform instance record (IP omitted) and the API deletion record |
| `scripts/` | `verify_fixes.sh`, `live_modalities.sh` (exact commands run on the GPU host), `check_verify.py` (as run), `recompute_224.py` (added for this bundle) |
| `llm/` | `binary.txt`, `dispatcher-version.txt`, `serve-sut.json` (launcher SUT), `sut.json`, `telemetry.yaml`, `prompts-report.json`, `prompts.jsonl.gz`; `f230-*.txt`; `f226` and `f227` `.stdout`, `.stderr`, `.summary.json` (final `summary.v3` line), `.jsonl.gz` (data log), `.ndjson.gz`; `f227.exit`, `allsmi-restarted.txt`; `llm-sweep.stdout.json`, `.stderr`, `.csv`, `.ndjson.gz`; `analyze.txt`, `analyze.json`; `f216.txt` |
| `vlm/`, `asr/`, `imagegen/` | `sut.json`, `models.json`, `telemetry.yaml`, `serve.txt` (launcher output, renamed from `serve.log`, which `.gitignore` excludes), `run.stdout`, `run.stderr`, `run.summary.json`, `run.jsonl.gz`, `run.ndjson.gz`, `sweep.stdout.json`, `sweep.stderr`, `sweep.csv`, `sweep.ndjson.gz` |
| `vlm/big.*` | #242 run: `big-prompts.jsonl`, `big.stdout`, `big.stderr`, `big.summary.json`, `big.jsonl.gz`, `big.ndjson.gz`, `big.png.sha256` |
| `imagegen/images.sha256`, `vlm/big.png.sha256` | sha256 and size of the 8 generated PNGs and of the 2048x2048 input PNG. The images are not committed. |

All NDJSON files and data logs are included in full, gzip'd (`gzip -9n`). The bundle is about 3.8 MB, under the
5 MB budget, so no sampling was needed. The CSVs and `vlm/big-prompts.jsonl` are force-added, because
`.gitignore` excludes `*.csv` and `*.jsonl`.

### Redaction

Before commit, these were rewritten in every file, including inside the gzip'd ones: the GPU UUID to
`GPU-REDACTED`, the host name to `redacted-host`, and the home-directory path in the launcher logs to `<HOME>`.
The instance IP appeared only in the Shadeform instance record and is omitted from `instance.json`. The SSH
`known_hosts` file was not copied. No temporary-directory paths and no key material were present. The scripts
pass `--api-key dummy`, and the Shadeform key was never written to any file. The Shadeform instance id is
kept. Redaction only replaces strings, so every JSON and NDJSON line still parses, and the checker reproduces
its output exactly. A grep for `GPU-` followed by hex, the original host name, the instance IP, `/home/`, `/tmp/`, `KEY`,
`Bearer`, `sk-` and `hf_` tokens over the plain and decompressed files finds nothing. The only `--api-key`
value is `dummy`.

## Reproduce the checks

```bash
B=artifacts/live/verify233-rtxpro6000-20261006
T=$(mktemp -d); cp -r $B/llm $B/vlm $B/asr $B/imagegen $T/
find $T -name '*.gz' -exec gunzip {} +
python3 $B/scripts/check_verify.py $T/llm $T/vlm $T/asr $T/imagegen | cmp - $B/checks.txt && echo identical
python3 $B/scripts/recompute_224.py $T/llm $T/llm/llm-sweep.stdout.json $T/llm/llm-sweep.csv
for m in vlm asr imagegen; do python3 $B/scripts/recompute_224.py $T/$m; done
python3 docs/queries/analyze.py $T/llm/llm-sweep.ndjson $T/llm/llm-sweep.stdout.json
```
