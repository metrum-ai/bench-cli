<!--
Copyright (c) 2026 Metrum AI, Inc.
SPDX-License-Identifier: Apache-2.0
-->

# Shadeform live tests

Helpers for optional GPU smoke runs against real vLLM / SGLang on Shadeform.
**Do not create instances unless you intend to pay for them.** `create` is
**dry-run by default** and only POSTs when you pass `--execute`.

## Prerequisites

- `curl`, `jq`, and a Shadeform API key
- Built binaries on `PATH` (after the rename): `metrum-ai-bench-llm`,
  `metrum-ai-bench-vlm`, and `wait_for_vllm`
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

# Real create — only when you are ready to spend money
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
then teardown. See [docs/CAMPAIGN.md](../../docs/CAMPAIGN.md).

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

## Teardown — always trap DELETE

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
