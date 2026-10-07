<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Campaign intake

Use this intake before any benchmark. Discover the hosts first, propose one complete set of choices, and wait for the operator to say `go` or edit the table. Ask only one round. If a later fact blocks a lane, mark it `not_run` rather than silently changing the confirmed choices.

## Questions

| Setting | What to ask | Proposed default | Where default comes from |
|---|---|---|---|
| Hosts | Which owned hosts or confirmed cloud instances may the campaign use? | Operator-owned hosts from the inventory | Campaign policy; no provider is assumed |
| GPUs per host | Which GPU indices and count may each host use? | Discovered inventory, one workload per GPU | `nvidia-smi` or `amd-smi`; campaign policy |
| Workloads | Which of `llm`, `vlm`, `asr`, `imagegen`, `embeddings`, and `rerank` should run? | `llm` only | Campaign policy |
| Lane placement | Which workload, model, GPU indices, and port belong to each lane? | One workload per available GPU | Campaign policy |
| Models and revisions | What exact model ID and immutable revision should each lane use? | Operator-selected model, pinned revision | Model card and operator choice |
| Quantization or dtype | What exact quantization or dtype should the server use? | Model and engine vendor recommendation | Current model card and serving docs, searched before the run |
| Engine and image | Which engine, version, image digest, and launch arguments should each lane use? | Current stable compatible engine image, pinned by digest | [SERVING.md](SERVING.md), current upstream docs, and model card |
| Bench CLI version | Which release tag, target archive, SHA-256, and Sigstore bundle should be used? | Latest final release with a matching host architecture | GitHub Releases and [RELEASING.md](RELEASING.md) |
| Telemetry | Which exporter version, recorder, engine metrics endpoint, and cadence should run? | Metrum all-smi fork plus engine `/metrics` | [TELEMETRY.md](TELEMETRY.md) |
| Prompt dataset/config/revision/profiles | Which dataset, config, immutable revision, and profiles should LLM use? | `metrum-ai/prompt-library`, `full`, pinned revision, `chat-medium` and `rag-medium` | [PROMPT_LIBRARY.md](PROMPT_LIBRARY.md); publication defaults |
| ISL and OSL windows | What inclusive per-request token windows apply to each LLM profile? | Profile target plus or minus its documented tolerance | [PROMPT_LIBRARY.md](PROMPT_LIBRARY.md); operator may tighten |
| Concurrency sweeps | Which concurrency values should each lane run? | LLM doubles to the lower of engine max batch and 256; VLM/ASR 1 to 64; imagegen 1 to 8 | Campaign policy, then current engine and model limits |
| Requests per stage | How many measured requests should each stage send? | 200; imagegen 32 | Campaign policy |
| Open loop | Should a rate sweep follow the closed-loop LLM sweep? | Yes for LLM, from 0.25x through 1.25x of closed-loop knee throughput | Campaign policy; skip when knee is null |
| SLOs | Which goodput thresholds apply? | TTFT 0.5 s and 1 s, TPOT 0.05 s, E2E 10 s | Campaign policy; operator must choose applicable values |
| Repetitions and seeds | How many repetitions and which request, shuffle, prompt, and bootstrap seeds? | 3 repetitions; request/shuffle/bootstrap seed 7; prompt seed 42 | Campaign and report policy |
| Budget caps | What wall-clock and monetary caps apply, and what action occurs at each cap? | Owned host: 8 h; cloud: no launch until explicit time and money caps exist | Campaign safety policy |
| Price per hour | What declared hourly price and currency should be recorded? | `null` unless the operator provides a current price | SUT cost is declared, not measured |
| Binary download location | Which release URL, local mirror, or air-gapped path supplies each archive and bundle? | Official release URL; use `MIRROR` when set | [RELEASING.md](RELEASING.md) and bootstrap procedure |
| Deliverables | Which raw logs, manifests, tables, and report should be retained? | All raw request and telemetry logs, checksums, SUT, status, tables, and report | Publication and campaign policy |

## Discovery

Run discovery without printing credentials. Record commands and UTC timestamps in the campaign notes.

<!-- doc-check: requires-gpu reason="GPU inventory and a container runtime require a benchmark host" -->
```bash
if command -v nvidia-smi >/dev/null 2>&1; then
  nvidia-smi --query-gpu=index,uuid,name,memory.total,memory.free --format=csv
elif command -v amd-smi >/dev/null 2>&1; then
  amd-smi static --json
else
  echo "no supported GPU inventory command found"
fi
uname -m
uname -sr
(command -v docker && docker version) || (command -v podman && podman version) || true
```

For remote discovery, use the operator's named SSH configuration. Do not infer a cloud provider, create an instance, or install software before confirmation.

## One round only

Present the completed table once. The operator either replies `go` or edits rows. After `go`, save the resolved values, immutable revisions, image digests, release checksums, and source URLs. Do not ask piecemeal follow-ups. A new blocker changes lane status to `failed` or `not_run`; it does not authorize a substitute GPU, model, engine, or workload.

## Secrets

Never ask for secret values in chat. Ask where credentials live, for example an SSH agent, a named credentials file, a secret manager reference, or a host environment already prepared by the operator. Never echo, log, commit, add to command lines, or include secrets in manifests. Use `dummy` only for endpoints that do not validate an API key.

## Filled example: one-GPU host

All values below are placeholders. Replace every angle-bracketed value before saying `go`.

| Setting | Confirmed placeholder value |
|---|---|
| Hosts | `<host-alias>` |
| GPUs per host | `<gpu-index-0>` on `<host-alias>` |
| Workloads | `llm` |
| Lane placement | `llm -> <host-alias>:<gpu-index-0>:<loopback-port>` |
| Models and revisions | `<model-id>@<40-char-revision>` |
| Quantization or dtype | `<dtype-or-quantization>` |
| Engine and image | `<engine>@<version>`, `<image>@sha256:<digest>` |
| Bench CLI version | `<tag>`, `<target>`, `<archive-sha256>` |
| Telemetry | `<all-smi-version>` plus `<engine-metrics-url>` |
| Prompt dataset/config/revision/profiles | `<dataset>`, `full`, `<40-char-revision>`, `chat-medium` |
| ISL and OSL windows | `ISL <lower>..<upper>; OSL <lower>..<upper>` |
| Concurrency sweeps | `<1,2,4,...>` |
| Requests per stage | `<count>` |
| Open loop | `<yes-or-no>; <rates-or-knee-multipliers>` |
| SLOs | `<metric=value list>` |
| Repetitions and seeds | `<repetitions>; request <seed>; prompt <seed>; bootstrap <seed>` |
| Budget caps | `<hours>; <currency amount or not-applicable>` |
| Price per hour | `<amount-or-null> <currency>` |
| Binary download location | `<release-url-or-mirror-path>` |
| Deliverables | `<campaign-directory>; raw logs; report` |

## Filled example: four-host campaign

All values below are placeholders. Each lane stays on its confirmed host and GPU set.

| Setting | Confirmed placeholder value |
|---|---|
| Hosts | `<host-a>`, `<host-b>`, `<host-c>`, `<host-d>` |
| GPUs per host | `<count-a>`, `<count-b>`, `<count-c>`, `<count-d>` |
| Workloads | `<selected-workload-list>` |
| Lane placement | `<lane-a> -> <host-a>:<gpu-set-a>`; `<lane-b> -> <host-b>:<gpu-set-b>`; `<lane-c> -> <host-c>:<gpu-set-c>`; `<lane-d> -> <host-d>:<gpu-set-d>` |
| Models and revisions | `<lane-a-model>@<revision-a>` through `<lane-d-model>@<revision-d>` |
| Quantization or dtype | `<per-lane-values>` |
| Engine and image | `<per-lane-engine-versions-and-image-digests>` |
| Bench CLI version | `<tag>`, `<per-architecture-assets-and-sha256>` |
| Telemetry | `<recorder-version>` plus each lane's `<engine-metrics-url>` |
| Prompt dataset/config/revision/profiles | `<dataset>`, `full`, `<40-char-revision>`, `<profiles>` |
| ISL and OSL windows | `<per-LLM-profile-windows>` |
| Concurrency sweeps | `<per-lane-lists>` |
| Requests per stage | `<per-lane-counts>` |
| Open loop | `<per-LLM-lane-policy>` |
| SLOs | `<per-lane-metric=value lists>` |
| Repetitions and seeds | `<repetitions-and-all-seeds>` |
| Budget caps | `<campaign-hours>; <total-currency-cap>; <per-host-action>` |
| Price per hour | `<per-host-prices-or-null>` |
| Binary download location | `<release-url-or-mirror-path-per-architecture>` |
| Deliverables | `<coordinator-directory>; manifests; raw logs; tables; report` |
