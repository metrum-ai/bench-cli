<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Pitfalls

Fifteen failure modes seen in agent-driven campaigns. Each entry gives the symptom, the cause, and the fix or flag. Verified behavior from `--help` and source wins over a campaign anecdote.

## 1. Length tolerances silently default to zero

Symptom: a run with `--isl-target` and `--osl-target` set fails or warns even though the lengths look close, or a run with no tolerance passes only by exact coincidence.

Cause: `--isl-tolerance` and `--osl-tolerance` both default to `0.0`. `IslOslTargets::resolve` replaces a zero tolerance with the mix-report tolerance only when `--prompt-mix-report` is supplied. Without that report, zero means exact match.

Fix: pass all four flags explicitly, for example `--isl-target`, `--isl-tolerance`, `--osl-target`, and `--osl-tolerance`, on every LLM invocation.

## 2. There is no ISL gate

Symptom: the run exits 0 with an out-of-window input length.

Cause: there is no `--fail-on-isl-mismatch`. ISL mismatches are counted only. `--fail-on-osl-mismatch` gates OSL, not ISL.

Fix: gate ISL yourself from the NDJSON request rows, reading `input_tokens` (strategic) or `prompt_tokens` (data log). Use `examples/agent/length_check.py`.

## 3. ISL basis mismatch

Symptom: the pool looks correct but the run reports an ISL mismatch.

Cause: the dataset field `target_input_tokens` is a supplied target that excludes the generation hint and chat template. `--isl-token-basis` only supports `supplied-target`, so the tokenizer count and the served `usage.prompt_tokens` can differ. Word ISL includes the hint text; supplied token ISL does not.

Fix: count server ISL with the server's `/tokenize` endpoint using the same chat template the benchmark sends, then calibrate once against `usage.prompt_tokens` before filtering the pool. Record the calibration.

## 4. The OSL gate fires after the summary

Symptom: the CLI exits nonzero, but a complete summary, CSV, and HTML exist.

Cause: `--fail-on-osl-mismatch` fails when any compared success is outside tolerance. On strategic it gates the last stage only and writes the summary, CSV, and HTML before it exits 1. On modality binaries it is one run, so any out-of-window request fails it.

Fix: distinguish a length failure from a crash by checking that the summary and NDJSON are complete. Tighten `--min-tokens` and `--max-tokens`, or rerun. Do not read the nonzero exit as a transport failure.

## 5. `--count` is required and the usage line hides it

Symptom: `metrum-ai-bench-cli-prompts` errors on a missing argument, or a wrapper builds the command from the usage line and omits `--count`.

Cause: `--count` is required unless `--version-only` is set, but the usage line currently omits it.

Fix: always pass `--count`. Use the mix report's `selected_count` for the downstream `--num-requests`.

## 6. Modality binaries do take NDJSON and telemetry

Symptom: a lane runs without telemetry because the operator assumed only strategic scrapes.

Cause: a stale belief that VLM, ASR, and imagegen lack `--ndjson` and `--telemetry`. They already accept both through `TelemetryArgs`.

Fix: pass `--ndjson` and `--telemetry` to every modality binary. Each modality binary takes one `--concurrency`, so invoke it once per concurrency level. `--scenario` is a required free-form string with no enum. GPU series can also come from an `all-smi record` file when the recorder runs separately.

## 7. Fixture paths resolve against the current directory

Symptom: `file not found` for prompts, images, audio, or schemas that exist in the source tree.

Cause: relative fixture paths resolve against the process working directory.

Fix: start bench clients with cwd `/opt/bench/repo` so `test-data/` resolves, or pass absolute paths. Record the cwd in the manifest.

## 8. Imagegen seeds get overridden

Symptom: every image in a run is identical, or the intended seed sequence is lost.

Cause: `--seed-mode` defaults to `increment`. A seed on a prompt row or in `--extra-body-json` overrides the CLI seed. Passing `--seed` twice does not error; the last value wins.

Fix: use `--seed-mode increment` with one `--seed`. Do not also set a seed on the row or in the extra body. Verify the resolved body template in the summary.

## 9. Preflight and probe scope

Symptom: a healthy server fails preflight, or an ASR or imagegen lane passes preflight but fails the sweep.

Cause: `preflight` is chat-only (`/v1/chat/completions`). It does not exercise transcription or image generation.

Fix: use preflight for chat endpoints. For ASR, run a curl probe on a fixture clip with a known transcript before the sweep. For imagegen, require all successes before trusting a cell.

## 10. SUT schema is strict

Symptom: `--sut` fails to load with an unknown-field error.

Cause: the SUT schema uses `deny_unknown_fields`. Allowed top-level keys are `provenance`, `name`, `vendor`, `gpu`, `cpu`, `memory_gb`, `driver_version`, `runtime`, `model`, `host_os`, `notes`, `cost`, `field_provenance`, and `extra`. Nested objects are also strict.

Fix: keep to the allowed keys. Put a GPU UUID or index in `notes` (string) and in `extra` with string values. `--require-sut` also needs `gpu.model`, `gpu.count`, `driver_version`, `runtime.name`, `runtime.version`, `runtime.config`, and `host_os`.

## 11. Knee field names and nulls

Symptom: a report crashes or shows a knee of 0 where there is none.

Cause: there is no `knee_rps`. A sweep point's `load` is the sweep axis and `throughput` is successful requests per second. `knee` is the stage index and is null when there is no bend. `knee_detection.reason` is null exactly when `index` is set. A rising throughput curve with less than a 20 percent p95 rise reports `no_bend`, and `summary.v3` has no knee field at all.

Fix: read `knee` and `knee_detection` from the strategic stdout or summary. Report null, not 0, when there is no knee. Do not look for a knee in a modality `summary.v3`.

## 12. Mix preservation and requests per stage

Symptom: the measured ISL and OSL drift from the selected mix.

Cause: `metrum-ai-bench-cli-llm` shuffles and cycles the pool under one global `--max-tokens`, and strategic restarts measured indexing at zero after warmup. A `--num-requests` (modality) or `--requests-per-stage` (strategic) value that differs from the selected count changes the mix. Warmup shifts it.

Fix: for the modality binary, set `--num-requests` to `report.selected_count` and `--warmup-requests 0`. For strategic, set `--requests-per-stage` to `selected_count` and keep `--warmup-requests 0` when mix fidelity matters.

## 13. Leftover engine workers and pkill self-match

Symptom: a restarted server fails to allocate GPU memory, or the kill command kills its own caller.

Cause: `EngineCore` (vLLM) or `DiffusionWorker` (vLLM-Omni) survive a container stop. A `pkill -f` pattern can also match the shell or script that contains the pattern in its command line.

Fix: stop the container or kill the recorded PID, then kill leftover workers. Use a bracket pattern such as `pkill -f '[E]ngineCore'` so the pattern cannot match the caller's own command line. Wait until free GPU memory is back above 85 percent before restarting, and re-probe `/v1/models`; a stale listener is not a live server.

## 14. WAN time inflates TTFT

Symptom: TTFT is far higher than the server's own logs suggest, and repeated runs are noisy.

Cause: the load generator runs on a laptop or coordinator and crosses a WAN to the serving host.

Fix: run the bench binary on the serving host. If it must run remotely, say so in the report and do not compare the TTFT with on-host runs.

## 15. Energy includes idle after short lanes

Symptom: joules per token look too high for a lane that finished quickly.

Cause: an energy figure integrates over a window that includes idle time after the last request.

Fix: name the exact window each energy figure covers and clip telemetry to the measured sweep window. Do not divide a whole-host energy delta by a short lane's token count.
