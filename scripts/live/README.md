<!--
Copyright (c) 2026 Metrum AI, Inc.
SPDX-License-Identifier: Apache-2.0
-->

# Live tests

## Local live gate (no Shadeform)

One smoke cell per modality against a real serving stack on this host. This
is what `.github/workflows/live-modality-smoke.yml` runs on the `gpu-h100`
runner.

```bash
cargo build --release --bins
scripts/live/serve/asr.sh start          # also writes live-results/serve-asr/sut.json
scripts/live/local_smoke.sh --local --modality asr
scripts/live/serve/asr.sh stop
```

`scripts/live/run_smoke.sh --local --modality <m>` is the same command.

**Search first and reuse binaries.** Before each run, web-search the current
vendor docs for the exact model and engine version, and override the launcher
(`MODEL=`, `SERVE_ARGS_OVERRIDE=`, `SOURCES_OVERRIDE=`, `SUT_NOTES_OVERRIDE=`)
when they differ from the pins here. A launcher's notes and sources describe
its default model only, so `start` exits with an error naming
`SUT_NOTES_OVERRIDE` when `MODEL` is overridden without it. The SUT's
`model.quantization` comes from `QUANTIZATION=` (`none` for null), then
`--quantization`/`-q` in the serve flags, then a quantizer token in the
`MODEL` name (`-FP8` is `fp8`, `-AWQ` is `awq`, `-GPTQ-Int4` is `gptq`), else
null; `extra.quantization_source` records which. The scripts resolve prebuilt binaries
through `lib/bench_bin.sh`: `BENCH_BIN_DIR`, then release tarball `bin/`, then
`target/release`, then `target/rel-user/release`, then `PATH`. They fail
clearly instead of compiling. `local_smoke.sh` records the binary path,
`--version`, and the checkout in the SUT.

**Shadeform key.** When both an exported `SHADEFORM_API_KEY` and `env.json`
are set and disagree, `shadeform.sh` prefers `env.json` and warns on stderr
(without printing key material). Confirm every delete with
`GET /instances/<id>/info` after `down`.

Which engine to use for each modality, the upstream docs for it, and the
pitfalls that change results are in the guides, not here:
[docs/SERVING.md](../../docs/SERVING.md) (index),
[docs/ASR.md](../../docs/ASR.md) (vLLM speech-to-text, Whisper
`--max-model-len 448`), and [docs/IMAGEGEN.md](../../docs/IMAGEGEN.md)
(vLLM-Omni, steps and guidance defaults).

For a Shadeform VM that serves each modality in turn over SSH, with all-smi
telemetry, use `widen_oss_modalities.sh` (`up` holds the VM with a delete
trap), `widen_cell.sh` (one cell plus telemetry sidecars on the GPU host),
and `telemetry_sidecar.py` (Prometheus poller for the modality binaries,
which have no `--telemetry` flag).

| Modality | Launcher | Stack (researched 2026-10-02; sources in each script) | Smoke input |
|---|---|---|---|
| llm | `serve/llm.sh` | Regular vLLM 0.30.0, `Qwen/Qwen3-8B`, `--reasoning-parser qwen3 --max-model-len 32768` | Hub mix: `metrum-ai/prompt-library`, config `sample`, profile `chat-short`; thinking disabled per request |
| vlm | `serve/vlm.sh` | Regular vLLM 0.30.0, `Qwen/Qwen3-VL-8B-Instruct`, Qwen3-VL recipe flags | `test-data/vlm/prompts.jsonl` (512x512 PNG) |
| asr | `serve/asr.sh` | Default `ASR_STACK=vllm`: vLLM 0.30.0 speech-to-text, `openai/whisper-large-v3-turbo`, `--max-model-len 448`. `ASR_STACK=omni`: vllm-omni 0.30.0 `--omni`, the intended stack, blocked until vllm-omni#5722 (no `/v1/audio/transcriptions` under `--omni`) | `test-data/asr/` LibriSpeech clips with `--ground-truth` |
| imagegen | `serve/imagegen.sh` | vllm-omni 0.30.0, `Tongyi-MAI/Z-Image-Turbo`, `--omni` | 1024x1024, 9 steps, guidance 0.0 |

Launchers take `start` (default), `stop`, `print` (show the docker command),
`logs`, and `sut` (print the SUT JSON without docker; add `HF_HUB_OFFLINE=1`
to skip the Hub revision lookup). `scripts/tests/serve_sut_test.sh` is the
offline self-test for the SUT the launchers write. `PORT`, `HF_HOME`, `HF_TOKEN` (passed by name, never written),
`GPU_DEVICES`, and `IMAGE`/`MODEL` overrides are read from the environment.
The SUT they write records the exact docker command in `runtime.config`, the
model revision SHA from the Hub, the GPU from `nvidia-smi`, and the source
URLs in `extra.launcher_sources`.

`scripts/live/assert_headline.sh <modality> <data_log> [--artifact-dir DIR]`
fails the cell when successes are 0 or below `MIN_SUCCESS_RATIO` (default
1.0), when `--require-sut` was not set, when ASR records lack WER/CER, when
VLM records sent no image, or when imagegen produced no decodable PNG/JPEG.
`scripts/tests/assert_headline_test.sh` is its offline self-test.

### LLM prompts come from the Hub

`scripts/live/lib/hub_prompts.sh` wraps `metrum-ai-bench-cli-prompts`. All
LLM smoke and campaign scripts use it; none write handmade prompt JSONL. With
no dataset given it uses `metrum-ai/prompt-library` (public, Apache-2.0),
config `sample`, profile `chat-short` (256 / 64). `PROMPT_DATASET`,
`PROMPT_CONFIG`, `PROMPT_REVISION`, `PROMPT_PROFILE`, `PROMPT_LOCAL_JSONL`,
and `PROMPT_LOCAL_PARQUET` override it; an explicit local file wins over the
Hub. Dataset, resolved revision SHA, profile, and row count go into the SUT
`notes` and `extra.prompt_*`. VLM, ASR, and imagegen have no Hub prompt
dataset and use the local fixtures above.

# Shadeform live tests

Helpers for optional GPU smoke runs against real vLLM / SGLang on Shadeform.
**Do not create instances unless you intend to pay for them.** `create` is
**dry-run by default** and only POSTs when you pass `--execute`.

## Prerequisites

- `curl`, `jq`, and a Shadeform API key
- Built binaries on `PATH` or under `target/{release,debug}`:
  `metrum-ai-bench-cli-llm`, `metrum-ai-bench-cli-vlm`, and
  `metrum-ai-bench-cli-prompts` (`wait_for_vllm` was never shipped;
  `run_smoke.sh` now polls `/v1/models` with curl)
- `env.json` at the repo root (gitignored) **or** `SHADEFORM_API_KEY` in the
  environment. Never commit `env.json` or paste key values into PRs/logs.
- Optional `SHADEFORM_SSH_KEY_ID` selects an uploaded key for diagnostic SSH.

```json
{
  "SHADEFORM_API_KEY": "…"
}
```

API base: `https://api.shadeform.ai/v1`  
Auth header: `X-API-KEY`

## GPU preference order

`pick` / `create` choose the first **available** type in this order (skip
consumer SKUs such as RTX 4090 / 5090):

1. RTX 6000 Pro Blackwell Server Edition (`gpu_type` `RTXPro6000`)
2. B200
3. H200
4. H100 (including `H100_nvl`)
5. L40S

Within a tier, prefer fewer GPUs, then lower hourly price.

## Models and images (<10B)

| Role | Hugging Face model | Docker image |
|------|--------------------|--------------|
| LLM  | `Qwen/Qwen2.5-7B-Instruct` | `vllm/vllm-openai:latest` |
| VLM  | `Qwen/Qwen2.5-VL-7B-Instruct` | `vllm/vllm-openai:latest` |
| Alt  | same 7B-class instruct models | `lmsysorg/sglang` |

## Commands

```bash
# List available preferred types (cloud / type / region / price)
./scripts/live/shadeform.sh types

# Print one preferred pick as JSON
./scripts/live/shadeform.sh pick

# Dry-run create (prints curl + JSON payload; does NOT POST)
./scripts/live/shadeform.sh create --engine vllm --modality llm
./scripts/live/shadeform.sh create --engine sglang --modality llm

# Real create - only when you are ready to spend money
./scripts/live/shadeform.sh create --engine vllm --modality llm --execute

# Wait until the instance is active; prints IP
./scripts/live/shadeform.sh wait <instance-id>

# Delete (always use a trap; see below)
./scripts/live/shadeform.sh delete <instance-id>

# Run benches against a running endpoint
./scripts/live/shadeform.sh run-llm --url http://HOST/v1/chat/completions
./scripts/live/shadeform.sh run-vlm --url http://HOST/v1/chat/completions \
  --prompts path/to/vlm-prompts.jsonl
```

The create payload maps container port 8000 to public host port 80 using
Shadeform's current `port_mappings` schema. `run-vlm` enables streaming so its
TTFT is measured rather than shown as an undefined legacy zero.

Orchestrated smoke (requires an **already running** OpenAI-compatible endpoint;
does not create Shadeform instances):

```bash
./scripts/live/run_smoke.sh --host HOST --port 8000
```

Results should go under gitignored `live-results/` (added by the hygiene PR).

## End-of-work campaign (parallel retained instances)

After local tests and release verification are green, launch **one GPU per
lane in parallel**, keep them until validate + `docs/SMOKE_RESULTS.md` exist,
then teardown. See [published smoke results](../../docs/SMOKE_RESULTS.md).

```bash
./scripts/live/campaign.sh plan
./scripts/live/campaign.sh launch          # dry-run
./scripts/live/campaign.sh launch --execute
./scripts/live/campaign.sh sweep --execute
./scripts/live/campaign.sh validate
./scripts/live/campaign.sh report          # writes docs/SMOKE_RESULTS.md
./scripts/live/campaign.sh teardown --execute
```

Artifacts ship via GitHub Releases (not private backup tooling).

## Teardown - always trap DELETE

Shadeform bills while the VM exists. **Always** register a trap that deletes
the instance on exit (success, failure, or Ctrl-C):

```bash
INSTANCE_ID=""
cleanup() {
  if [[ -n "${INSTANCE_ID}" ]]; then
    ./scripts/live/shadeform.sh delete "${INSTANCE_ID}" || true
  fi
}
trap cleanup EXIT

# After create --execute:
INSTANCE_ID="$(jq -r .id <<<"${CREATE_JSON}")"
./scripts/live/shadeform.sh wait "${INSTANCE_ID}"
# … run benches …
# delete runs automatically via trap
```

`shadeform.sh create` prints the same trap snippet in its dry-run / execute
output. Prefer `POST …/instances/{id}/delete` via the `delete` subcommand
rather than leaving orphaned GPUs.
