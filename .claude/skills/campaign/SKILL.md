<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Skill: run a bench campaign

## When to use

The operator wants a measured campaign on hosts they own or on a cloud they name. Modalities are selected in one intake. Default is LLM chat only.

## Modalities are selected at intake

Ask once, in one table, with a proposed default on every row. The operator answers `go` or edits rows. Do not ask again after confirmation unless something blocks the run.

`Workloads` is a list. Proposed default: `llm` only. Allowed values:

- `llm` (chat)
- `vlm`
- `asr`
- `imagegen`
- `embeddings`
- `rerank`

Run sweep and report sections only for the selected set. `llm` uses the closed-loop strategic sweep and, when confirmed, the open-loop rate sweep. `vlm`, `asr`, and `imagegen` use the matching bench binary, with `--scenario` from that binary's `--help`, cwd `/opt/bench/repo`. `embeddings` and `rerank` use `metrum-ai-bench-cli-strategic --kind`. There is no `scripts/live/serve/` launcher for embeddings or rerank. A selected lane with no serving entry is `not_run`. Do not invent a server recipe to make it run.

## Presets

YAML data under `.claude/skills/campaign/presets/`. The skill loads a preset and applies `key=value` overrides. Resolution order: explicit argument, then preset, then skill default. Print the resolved table before running and copy it into the SUT notes and `REPORT.md`.

The presets are:
- `llm-matrix.yaml`
- `vlm-matrix.yaml`
- `asr-matrix.yaml`
- `imagegen-matrix.yaml`
- `smoke-cell.yaml`
- `telemetry-ladder.yaml`
- `aiperf-bakeoff.yaml`
- `parity-pair.yaml`

Serving entries for exact model ids live next to the presets. No web search when the operator named an exact Hugging Face id and an entry exists. Search only when the model is not exact or has no entry, then write the result back as a dated entry.

## Procedure

This skill is instructions for an agent on the operator machine (the coordinator). Benchmarks run on GPU hosts the operator owns or rents. Binaries run on the host from `/opt/bench/bin`, never on the coordinator.

1. **Discover, then ask once.** Own hosts: GPU inventory, `uname -m`, CPU, memory, disk, Docker or Podman, NVIDIA Container Toolkit. Cloud: current provider docs, GPU types, counts, prices. Then one intake table. Required rows include hosts, credentials location (never secret values), GPUs per host, workloads, lane placement, models, quantization, serving engine and pinned image digest, bench-cli release tag and sha256, telemetry recorder, LLM prompts, ISL/OSL windows, concurrency sweep, requests per stage, open-loop yes/no, SLOs, repetitions and seeds, budget, optional price per hour, binary download source, deliverables. Defaults: LLM chat only; one workload per GPU; vLLM latest stable image pinned by digest; latest bench-cli release; all-smi from `chetan-metrum-ai/all-smi`; prompts from `metrum-ai/prompt-library` config `full`, profiles `chat-medium` and `rag-medium`; concurrency doubling for LLM until max batch or 256, VLM and ASR 1 to 64, image generation 1 to 8; 200 requests per stage (image generation 32); open loop yes for LLM at 0.25x to 1.25x of the closed-loop knee; TTFT 0.5 s and 1 s, TPOT 0.05 s, E2E 10 s; 3 repetitions; request and shuffle seed 7; prompt seed 42; warmup 0; own-host wall clock 8 h; cloud instances only after caps are stated. After `go`, write `choices.json`, re-resolve revisions and digests, and record them. Work under `bench/campaigns/<UTC>/` with ISO 8601 UTC timestamps that include milliseconds.

2. **Rules.** Secrets stay in process memory. Never print, log, commit, or pass them on a command line. `HF_TOKEN` goes to the remote download through SSH stdin, then unset. No `env`, `set`, or `printenv` on a host. Campaign files use the Metrum copyright line and SPDX `Apache-2.0` (JSON gets `_copyright` only when the schema allows unknown fields). Prebuilt binaries only. Verify sha256 and the Sigstore bundle when one is published. If no release asset matches the host arch, the lane is `not_run`. Own hosts are borrowed: touch only `/opt/bench` or `$HOME/bench`, the model directory, and this campaign's containers and processes. A busy GPU is `not_run: gpu busy`. Every reported number needs a local summary line and a local per-request log for that `run_id`. A failed lane stays `failed` or `not_run`. Never invent a number, a knee, or a chart. If this skill disagrees with pinned bench-cli docs or a binary `--help`, the docs and `--help` win. Log each disagreement in `NOTES.md` with a UTC timestamp.

3. **Coordinator.** Record the coordinator OS in `choices.json`. Python 3.10+ is allowed only under `bench/campaigns/<UTC>/` on the coordinator, using `pathlib`, OpenSSH, and `subprocess` timeouts. Do not add that Python to the repo, to `.claude/skills/*/scripts/`, or to `scripts/`. Do not depend on `timeout`, `setsid`, `rsync`, or `zstd` existing on the coordinator. Copy results with `rsync` when present, otherwise `tar` over SSH checked against a sha256 manifest written on the host. Long work runs detached on the host (`tmux`, or `setsid nohup` when the host has it). Each runner writes `status.json` and a heartbeat. Poll those files. After a restart, `--resume` and do not start a second runner while a heartbeat is under 5 minutes old.

4. **Hosts and budget.** Own hosts: `infra/hosts.json` and `infra/ssh_config`. Unreachable for 30 minutes means `not_run`. Cloud: only the confirmed provider, only the confirmed type and count, label `bench-<lane>-<UTC compact>`. Save instance ids, addresses, and prices to `infra/instances.json` and nothing else from the create response. If the GPU type is unavailable, stop and ask. Do not substitute. One watchdog on the coordinator polls every 5 minutes, logs `COST.md`, and on a crossed cap deletes cloud instances or stops own-host containers and runners. Start it detached. Stop it after cleanup. State that `cargo xtask shadeform` is how cloud instances are created and deleted, and that lane steps never call a cloud API. If the xtask command does not exist yet, stop and report that #254 / #256 are not landed, and do not invent a cloud client.

5. **Bootstrap.** Idempotent `setup/host_bootstrap.sh <lane>` with no secrets: GPU and host inventory under `/opt/bench/env/`, pinned bench-cli and recorder verified and unpacked to `/opt/bench/bin`, bench-cli source tag unpacked to `/opt/bench/repo` for `test-data/`, `metrum-ai-bench-cli selftest` (failure stops the lane), weights via `hf download` at the pinned revision, `MANIFEST.json`. Save each binary `--help` under the campaign `help/` directory and use those flags.

6. **Serve.** One server per lane. Pinned image digest, confirmed model, revision, dtype, GPU indices, port bound to `127.0.0.1`, `--served-model-name` set to the model id. Launch flags come from a recorded serving entry when the model id is exact. Search the model card and engine docs only when the id is not exact or no entry exists, then record sources in the SUT `notes` and save a dated serving entry. Ready means `GET /v1/models` succeeds, waiting up to 30 minutes. Between repetitions, remove the container or kill leftover engine workers on that GPU, and wait until free memory is back above 85%. A stale listener is not a live server.

7. **Telemetry.** Before the first sweep: `all-smi doctor --json`, `all-smi api --port 9090` on localhost, `all-smi record` until the final copy, and `/metrics` returns 200. `telemetry.yaml` scrapes the recorder and the engine `/metrics`, filters GPU series to the lane's indices, and excludes per-process series. `include` is taken from a live curl of `/metrics`, not from memory.

8. **Sweeps.** Selected lanes run in parallel for the confirmed repetitions. Shuffle closed-loop jobs with the confirmed seed. Restart the server between repetitions. Writes go to `lanes/<lane>/runs/<run_id>/`.
   - LLM: build the prompt pool on the host after the server is ready (`metrum-ai-bench-cli-prompts` with `--require-pinned-revision`, count 4x requests per stage). Count server ISL with `POST /tokenize` using the same chat template the benchmark sends. Calibrate once with `max_tokens: 1`. Keep rows inside the ISL window. A usable pool has at least 64 distinct source rows. Regenerate once at 3x count if short. If still short, the profile is `not_run`. Never widen the window. Force OSL with `--min-tokens` at the lower bound and `--max-tokens` at the upper bound, and say so in the report. Closed loop uses `metrum-ai-bench-cli-strategic` with explicit ISL and OSL tolerances (they default to 0), `--fail-on-osl-mismatch`, `--sut --require-sut`, `--ndjson --telemetry --require-telemetry`. Before the first sweep, run one 8-request stage and record which NDJSON field the ISL check reads. Open loop, when confirmed, sweeps rate at the confirmed multiples of `knee.throughput`. If there is no knee, write `open-loop-skipped.txt`.
   - Other selected modalities: one binary invocation per concurrency level. ASR must pass a curl probe on a fixture clip with a known transcript before the sweep. Image generation sets size, steps, guidance, and seed explicitly, writes an artifact directory per cell, and stays labeled smoke-scale unless a larger run was confirmed.
   - Keep a cell only when the CLI exits 0, successes equal requested, errors are 0, and for LLM every request is inside both windows (`length_check.json`). Rerun a failed cell once. A second failure stops that lane and keeps completed runs. `status.json` is written in a `finally` block that catches `BaseException`.

9. **Copy, then clean up.** Copy to the coordinator every 30 minutes and once more when every lane is done. Verify against the host sha256 manifest. Decode the recorder file and confirm its span covers every sweep. Write the inventory to `COST.md` before cleanup. Cloud: delete campaign instances and paste the empty list into `COST.md` with the last spend reading. Own hosts: remove campaign containers and stop runners and the recorder. Record `docker ps` and `nvidia-smi` in `COST.md`. Keep weights unless told otherwise. Do not clean up before the report is written from local files.

10. **Report.** `report/REPORT.md` and `report/tables/` from local files only: `<lane>_stages.csv`, `llm_knees.csv`, `llm_length_compliance.csv`, `telemetry.json`. Bootstrap CI is 10,000 resamples of the mean, seed 7, 2.5th and 97.5th percentiles. With 3 repetitions, say that the interval is the spread of the three means. Clip telemetry to each lane's sweep window and GPU indices. Busy means utilization of at least 20%. State which window each joule figure covers. Leave null fields null. Include system under test, methodology, validity, per-lane stages with knee CI and every `run_id`, cost, and every item that did not run and why.

11. **Order.** Discover, intake, wait for `go`. Write `choices.json`, `NOTES.md`, `COST.md`, start the watchdog. Create instances or verify hosts. Bootstrap. Selftest must pass. Telemetry, weights, servers, probes. LLM pools and the 8-request length check. All lanes, polling heartbeats. Copy and verify. Report. Clean up and stop the watchdog.