<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Agent runbook

End-to-end procedure for a coding agent running a publishable campaign. Every command uses placeholders. No model, GPU, cloud, or engine is a requirement of this document; the operator confirms those in [INTAKE.md](INTAKE.md). If a campaign anecdote disagrees with a binary `--help` or the source, the `--help` and source win.

Conventions:

- Timestamps are ISO 8601 UTC with milliseconds, for example `2026-10-06T12:00:00.000Z`.
- Work under `<campaign-root>` = `bench/campaigns/<UTC-compact>`.
- Binaries run on the GPU host from `/opt/bench/bin` (or `$HOME/bench/bin` without sudo), never on the coordinator.
- Every reported number needs a local summary line and a local per-request log for the same `run_id`.
- A lane with no matching asset, no server recipe, or a busy GPU is `not_run`. Never substitute.
- Fenced blocks tagged `<!-- doc-check -->` are mock-testable. Blocks tagged `<!-- doc-check: requires-gpu reason="..." -->` need a GPU, a cloud host, or a real model.

## Step 1: Host bootstrap

Goal: install verified binaries, source fixtures, and a manifest on each host, idempotently and without secrets.

The bootstrap script is `examples/agent/host_bootstrap.sh`. It detects `uname -m`, verifies the SHA-256 and the Sigstore bundle, unpacks to `/opt/bench/bin` (or `$HOME/bench/bin` when `/opt` is not writable), unpacks the matching source tag to `/opt/bench/repo` for `test-data/`, runs `selftest`, and writes a manifest. Set `MIRROR` for an air-gapped host.

<!-- doc-check: requires-gpu reason="bootstrap installs GPU-host binaries and runs on the serving host" -->
```bash
TAG="<tag>"
TARGET="<target-triple>"
ARCHIVE="metrum-ai-bench-cli-${TAG}-${TARGET}.tar.gz"
BASE="<release-url-or-mirror>"
PREFIX="${PREFIX:-/opt/bench}"
MIRROR="${MIRROR:-}"

# Fetch archive, checksum, and Sigstore bundle from BASE (or MIRROR when set).
# sha256sum -c the published .sha256 file.
# Verify the bundle with the identity and issuer from RELEASING.md:
cosign verify-blob \
  --bundle "${ARCHIVE}.sigstore.json" \
  --certificate-identity-regexp 'https://github.com/metrum-ai/bench-cli/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  "${ARCHIVE}"

# Unpack binaries to "${PREFIX}/bin" and the source tag to "${PREFIX}/repo".
"${PREFIX}/bin/metrum-ai-bench-cli" selftest
```

Files produced: `${PREFIX}/bin/metrum-ai-bench-cli*`, `${PREFIX}/repo/test-data/`, `${PREFIX}/env/MANIFEST.json`, saved `help/<binary>.txt`.

Check that proves success: `metrum-ai-bench-cli selftest` prints `selftest: ok` and exits 0, and the manifest lists every installed path with its SHA-256.

Failure: retry once from the mirror. If it fails again, mark every lane on that host `failed: bootstrap` and stop. Do not fall back to an unverified archive.

## Step 2: Serve

Goal: one confirmed server per lane, pinned by image digest, ready before any sweep.

Record the image digest, model revision, dtype, GPU indices, loopback port, and `--served-model-name` in the SUT `runtime.config` and `notes`. Launch flags come from a recorded serving entry when the model ID is exact; otherwise search the model card and engine docs and record the source URLs. Engine map: [SERVING.md](SERVING.md).

Readiness is a successful `GET /v1/models` that lists the served model, not an open port.

<!-- doc-check: requires-gpu reason="serving a model needs a GPU host and an engine image" -->
```bash
PORT="<loopback-port>"
MODEL="<model-id>"
curl -fsS "http://127.0.0.1:${PORT}/v1/models" | grep -F "${MODEL}"
```

Files produced: lane container or process record, saved server log, `sut.json` runtime block.

Check that proves success: the model ID appears in `/v1/models` within the 30 minute readiness window.

Failure: retry the launch once with the same pinned digest. If it still fails, mark the lane `failed: serve` and move to the next lane.

Between repetitions, stop the server and every leftover engine worker, then wait for GPU free memory to recover before the next start:

<!-- doc-check: requires-gpu reason="stopping engine workers and reading GPU memory needs a GPU host" -->
```bash
# Prefer the container stop or the recorded PID. When killing by pattern, use
# a bracket so the pattern cannot match the caller's own command line.
pkill -f '[E]ngineCore' || true
pkill -f '[D]iffusionWorker' || true

# Never trust a stale listener. Wait until free memory is back above 85%.
until nvidia-smi --query-gpu=memory.free,memory.total --format=csv,noheader,nounits \
  | awk -F, '{ if ($1/$2 < 0.85) exit 1 }'; do sleep 5; done

# Re-probe readiness; an open port alone is not a live server.
curl -fsS "http://127.0.0.1:${PORT}/v1/models" | grep -F "${MODEL}"
```

A stale listener is not a live server. Always re-probe `/v1/models` after a restart. A `pkill -f` pattern must not match the caller's command line; the bracket form and PID files both avoid that.

## Step 3: Telemetry

Goal: durable hardware and engine time series that cover every sweep window.

Run the recorder on the serving host and scrape it with `--ndjson --telemetry`. Verify the installed recorder's own `--help` first; do not assume flag names.

<!-- doc-check: requires-gpu reason="the all-smi recorder needs a GPU host" -->
```bash
# Verify the installed recorder's subcommands and flags before use.
all-smi doctor --json
all-smi api --port 9090 &
# Confirm the path. On the Metrum fork v0.26.3-metrum.4 the path is /metrics;
# /metric returned 404 on 2026-10-02.
curl -fsS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:9090/metrics

# `record` output and interval flags must be confirmed with:
#   all-smi record --help
# If the installed fork accepts an output path and an interval in seconds,
# start the recorder detached, for example:
#   all-smi record -o "${CAMPAIGN}/telemetry.ndjson.zst" -i 1 &
# The exact spelling is not verifiable from this repository; confirm it on the
# host and record the command in the manifest. A stale PID file must not skip
# the start: check that the process is alive, not just that the file exists.
```

Files produced: `telemetry.yaml`, `telemetry.ndjson.zst` (or the recorder's native output), the recorder log, and the NDJSON `telemetry` rows written by the bench binary.

Check that proves success: `/metrics` returns HTTP 200, the startup probe prints a nonzero `matched_series`, and the recorder span covers every sweep window.

Failure: retry the recorder start once after removing a stale PID file. If the probe still fails, the run fails closed; mark the lane `failed: telemetry`.

The scrape template is `examples/agent/telemetry.yaml.example`. It scrapes the recorder and the engine `/metrics`, and its `include` patterns do not match `all_smi_process_`. YAML has no exclude key: per-process rows are excluded by not including them, and they must stay off in published runs because their labels can carry secrets.

## Step 4: LLM prompt pool

Goal: a reproducible prompt pool whose server-measured ISL and forced OSL both sit inside the confirmed windows.

Generate the pool with `metrum-ai-bench-cli-prompts` from `metrum-ai/prompt-library`, pinned to a 40-character revision. Then count server ISL with the server's `/tokenize` endpoint using the same chat template the benchmark sends, and calibrate once against `usage.prompt_tokens`. Keep rows inside the ISL window. Force OSL with `--min-tokens` at the lower bound and `--max-tokens` at the upper bound.

<!-- doc-check: requires-gpu reason="prompt extraction needs the pinned dataset and a live server to tokenize" -->
```bash
metrum-ai-bench-cli-prompts \
  --dataset metrum-ai/prompt-library \
  --revision "<40-char-revision>" --require-pinned-revision \
  --config full --count "<4x-requests-per-stage>" --seed 42 \
  --profile chat-medium \
  --output "${CAMPAIGN}/mix.jsonl" --report "${CAMPAIGN}/mix-report.json"
```

Then run `examples/agent/build_prompt_pool.py`, which calls `/tokenize`, calibrates against `usage.prompt_tokens`, and filters to the ISL window. A usable pool has at least 64 distinct source rows. Regenerate once at 3x count if short. If it is still short, the profile is `not_run`; never widen the window.

Files produced: `mix.jsonl`, `mix-report.json`, `prompt_pool.jsonl`, `tokenize_calibration.json`.

Check that proves success: the calibration compares `/tokenize` counts against `usage.prompt_tokens` within a stated tolerance, and every pool row is inside the ISL window.

Failure: retry generation once at 3x count. If the pool is still short, mark the profile `not_run`.

Pass all four target and tolerance flags explicitly. `--isl-tolerance` and `--osl-tolerance` default to 0, so an omitted tolerance means an exact match. Without `--prompt-mix-report`, 0 is not replaced.

## Step 5: Sweeps

Goal: one measured sweep per lane at the confirmed loads, with the selected mix preserved.

LLM uses `metrum-ai-bench-cli-strategic`. VLM, ASR, and imagegen use their modality binaries, one invocation per concurrency level. Embeddings and rerank use `metrum-ai-bench-cli-strategic --kind`. There is no separate embeddings or rerank binary and no live launcher for them.

<!-- doc-check: requires-gpu reason="the sweep talks to a live model endpoint" -->
```bash
metrum-ai-bench-cli-strategic \
  --url "http://127.0.0.1:${PORT}/v1/chat/completions" \
  --model "${MODEL}" --api-key dummy --streaming \
  --prompts "${CAMPAIGN}/prompt_pool.jsonl" \
  --max-tokens "<osl-upper>" --min-tokens "<osl-lower>" --ignore-eos \
  --sweep "<concurrency-list>" --requests-per-stage "<selected-count>" \
  --warmup-requests 0 \
  --isl-target "<isl-target>" --isl-tolerance "<isl-tolerance>" \
  --osl-target "<osl-target>" --osl-tolerance "<osl-tolerance>" \
  --fail-on-osl-mismatch \
  --sut "${CAMPAIGN}/sut.json" --require-sut \
  --ndjson "${CAMPAIGN}/run.ndjson" \
  --telemetry "${CAMPAIGN}/telemetry.yaml" --require-telemetry \
  --csv "${CAMPAIGN}/requests.csv" --html "${CAMPAIGN}/report.html"
```

Files produced: `run.ndjson`, `requests.csv`, `report.html`, `summary.json`, and per-stage rows.

Check that proves success: exit 0, every stage reports `successes == requested`, zero errors, and the OSL gate passes.

Failure: rerun the failed cell once. A second failure stops the lane and keeps completed runs. The OSL gate fires after the full summary is written, so a nonzero exit with a complete summary and NDJSON is a length failure, not a crash. On strategic the gate compares the last stage only.

One concurrency per modality invocation: the modality binaries take a single `--concurrency`, so loop over the confirmed values and give each its own `run_id`. `--scenario` is a required free-form string; use the confirmed scenario name.

## Step 6: Validity

Goal: keep only cells that are valid and complete, and never lose a cell to an exception.

A cell is valid when all hold:

- the CLI exits 0;
- `successes == requested` (`attempted == successes` in the modality summary);
- `errors == 0`;
- for LLM, every measured request is inside both the ISL and OSL windows.

Read the per-request windows from the NDJSON request rows (`input_tokens` and `output_tokens` on strategic rows; `prompt_tokens` and `completion_tokens` on data-log rows). `examples/agent/length_check.py` writes `length_check.json`.

<!-- doc-check -->
```bash
# CI provides CAMPAIGN with a fixture NDJSON; a live cell uses the sweep output.
python3 "${REPO_ROOT}/examples/agent/length_check.py" \
  --ndjson "${CAMPAIGN}/run.ndjson" \
  --isl-target "${ISL_TARGET}" --isl-tolerance "${ISL_TOLERANCE}" \
  --osl-target "${OSL_TARGET}" --osl-tolerance "${OSL_TOLERANCE}" \
  --out "${CAMPAIGN}/length_check.json"
test "$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["pass"])' "${CAMPAIGN}/length_check.json")" = "True"
```

Files produced: `length_check.json` per LLM run, and `status.json` per lane.

Check that proves success: `length_check.json` reports zero out-of-window requests for every stage, and `status.json` records the cell as `ok`.

Failure: rerun the failed cell once. Write `status.json` in a `finally` block that catches `BaseException`, so an interrupted lane still records its state.

## Step 7: Copy and verify

Goal: a durable, verified copy of every artifact before cleanup.

Write a SHA-256 manifest on the host, copy with `rsync` when present otherwise `tar` over SSH, and verify on the coordinator. Decode the recorder output and confirm its span covers every sweep window.

<!-- doc-check: requires-gpu reason="artifacts live on the GPU host and are copied over SSH" -->
```bash
find "${CAMPAIGN}" -type f -print0 | sort -z | xargs -0 sha256sum > "${CAMPAIGN}/SHA256SUMS"
rsync -a "<host>:${CAMPAIGN}/" "${LOCAL}/" || tar -C "${CAMPAIGN}" -cf - . | tar -C "${LOCAL}" -xf -
(cd "${LOCAL}" && sha256sum -c SHA256SUMS)
```

Files produced: `SHA256SUMS`, the copied campaign tree, and `COST.md`.

Check that proves success: `sha256sum -c` passes on the coordinator and the recorder span covers every sweep window.

Failure: re-copy once. If the manifest still fails, mark the affected cells `failed: copy` and do not report them.

Write the inventory to `COST.md` before cleanup: instance IDs and addresses for cloud, `docker ps` and `nvidia-smi` for owned hosts, and the last spend reading.

## Step 8: Report

Goal: a report built only from local files.

Bootstrap CI is 10,000 resamples of the mean with seed 7 and the 2.5th and 97.5th percentiles. With 3 repetitions, state that the interval is the spread of the three run means, reported as the minimum and maximum of those means. Clip telemetry to each lane's sweep window and GPU indices. Busy means utilization of at least 20 percent. Name the window each energy figure covers. Leave a null field null.

Files produced: `report/REPORT.md`, `report/tables/<lane>_stages.csv`, `report/tables/llm_knees.csv`, `report/tables/llm_length_compliance.csv`, `report/tables/telemetry.json`.

Check that proves success: every reported number traces to a local summary line and a local per-request log for the same `run_id`, and every item that did not run appears with its reason.

Failure: no retry. A missing input stays missing and is reported as such.

Knee fields are `knee` (the stage index) and `knee_detection` (`index`, `reason`, `points`, `min_points`, `method`, `p95_rise`, `saturated_index`, `min_p95_rise`, `min_marginal_gain`, `max_error_rate_rise`). `reason` is null exactly when `index` is set. `knee` is null when the sweep has no bend, and `summary.v3` has no knee field.

## Step 9: Agent session resilience

Goal: survive a coordinator restart without duplicate or lost work.

Run long work detached on the host. Each runner writes `status.json` and a heartbeat. Poll those files; do not hold a foreground SSH session. `--resume` is a runner flag, not a bench-cli flag: no bench-cli binary has `--resume`. After a restart, resume the runner and do not start a second runner while a heartbeat is under 5 minutes old.

Files produced: `status.json`, heartbeat file, and runner logs.

Check that proves success: exactly one live runner per lane, and `status.json` advances.

Failure: if a heartbeat is stale, resume once. If a second runner was already live, refuse to start and report the conflict.

## Cleanup

Goal: leave the hosts as found.

Cloud: delete the campaign instances and paste the empty list into `COST.md`. Owned hosts: remove campaign containers, stop the runners and the recorder, and record `docker ps` and `nvidia-smi` in `COST.md`. Keep weights unless told otherwise. Do not clean up before the report is written from local files.
