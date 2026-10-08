<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Serving stacks for each modality

Metrum AI Bench CLI is a benchmark harness: it measures an OpenAI-compatible HTTP server that you start yourself. Use this page to pick the server for each modality, then follow the upstream links for recipes. This repository does not copy them.

**Search first.** Before any real-backend run, web-search the current vendor docs for that exact model, engine, and engine version. Check:
- the serving framework and image tag
- the launch arguments
- the context length and memory flags
- the request parameters (for example thinking toggles and diffusion steps)
- a sensible concurrency and ISL/OSL sweep

Record what you chose, with source URLs, in the SUT `runtime.config` and `notes`. Do not reuse flags from memory or from an older run.

## Engine map

| Modality | Bench binary and endpoint | Serving framework | Example model we stage | Live launcher | Upstream docs | Live verification |
|---|---|---|---|---|---|---|
| LLM | `metrum-ai-bench-cli-llm`, `/v1/chat/completions` | **Regular vLLM** (`vllm/vllm-openai`); SGLang also works | `Qwen/Qwen3-8B` | [`serve/llm.sh`](../scripts/live/serve/llm.sh) | [vLLM OpenAI server](https://docs.vllm.ai/en/latest/serving/openai_compatible_server.html), [reasoning outputs](https://docs.vllm.ai/en/latest/features/reasoning_outputs.html), [vLLM recipes](https://docs.vllm.ai/projects/recipes/en/latest/), [SGLang](https://docs.sglang.ai/) | [Ledger](CLAIMS_LEDGER.md) |
| VLM | `metrum-ai-bench-cli-vlm`, `/v1/chat/completions` with `image_url` parts | **Regular vLLM** multimodal | `Qwen/Qwen3-VL-8B-Instruct` | [`serve/vlm.sh`](../scripts/live/serve/vlm.sh) | [vLLM multimodal inputs](https://docs.vllm.ai/en/latest/features/multimodal_inputs.html), [Qwen3-VL recipe](https://docs.vllm.ai/projects/recipes/en/latest/Qwen/Qwen3-VL.html) | [Ledger](CLAIMS_LEDGER.md) |
| ASR | `metrum-ai-bench-cli-asr`, `/v1/audio/transcriptions` | **vLLM-Omni** is the intended stack. Until [vllm-omni#5722](https://github.com/vllm-project/vllm-omni/issues/5722) lands, serve with regular vLLM speech-to-text (see [ASR.md](ASR.md#serving-frameworks)). | `openai/whisper-large-v3-turbo` | [`serve/asr.sh`](../scripts/live/serve/asr.sh) (`ASR_STACK=vllm` default, `ASR_STACK=omni`) | [vLLM speech-to-text](https://docs.vllm.ai/en/latest/serving/online_serving/speech_to_text/), [vLLM-Omni docs](https://docs.vllm.ai/projects/vllm-omni/en/latest/) | [Ledger](CLAIMS_LEDGER.md) |
| ImageGen | `metrum-ai-bench-cli-imagegen`, `/v1/images/generations` | **vLLM-Omni** (`vllm serve <model> --omni`) | `Tongyi-MAI/Z-Image-Turbo` | [`serve/imagegen.sh`](../scripts/live/serve/imagegen.sh) | [vLLM-Omni image generation API](https://docs.vllm.ai/projects/vllm-omni/en/latest/serving/image_generation_api/), [vLLM-Omni docs](https://docs.vllm.ai/projects/vllm-omni/en/latest/) | [Ledger](CLAIMS_LEDGER.md) |

The Live verification column always defers to [CLAIMS_LEDGER.md](CLAIMS_LEDGER.md). Supported in code is not the same as verified against a real server.

**Why ASR is not on vLLM-Omni yet (checked 2026-10-02).** In vllm-omni v0.30.0, starting a server with `--omni` sets the engine's supported tasks to `generate` and `speech` only (`vllm_omni/engine/omni_engine_base.py`). The API server therefore never builds the transcription handler, and `/v1/audio/transcriptions` is unavailable under `--omni` for every model. Upstream tracks the fix in [vllm-omni#5722](https://github.com/vllm-project/vllm-omni/issues/5722).

Regular vLLM documents Whisper on that endpoint today ([speech-to-text](https://docs.vllm.ai/en/latest/serving/online_serving/speech_to_text/)), so `serve/asr.sh` defaults to it. `ASR_STACK=omni` exists to re-validate the intended path when the fix ships. Live ASR results so far (the 2026-10-02 widen) used regular vLLM 0.30.0. Re-validate on vLLM-Omni before calling ASR verified on it.

## Use prebuilt binaries

Unless you want a from-source build, run binaries that already exist. Check these in order:

1. An unpacked GitHub Release tarball: `ROOT/bin/metrum-ai-bench-cli*`.
2. An existing release build in this checkout: `target/release/`, or `target/rel-user/release/` if the release was built with `--target-dir` because `target/release` was not writable.
3. Only if neither has what you need, run one `cargo build --release --bins` in the single primary checkout, then reuse that output. Never build in a second git worktree.

Building is the right call in two cases: the operator asks for a from-source build, or the fix you need exists only in the working tree. Debug builds are fine for plumbing tests, but never for numbers you report.

The live scripts resolve binaries in this order through [`scripts/live/lib/bench_bin.sh`](../scripts/live/lib/bench_bin.sh). `BENCH_BIN_DIR` overrides it. If nothing is found they fail with a clear message; they never compile. `local_smoke.sh` writes the binary path, the `--version` output, and the checkout's `git describe` into the SUT, because a build from an untagged commit still prints the last release version.

## One example per modality

```bash
# Uses existing binaries; see "Use prebuilt binaries"
scripts/live/serve/<llm|vlm|asr|imagegen>.sh start
scripts/live/local_smoke.sh --local --modality <llm|vlm|asr|imagegen>
scripts/live/serve/<modality>.sh stop
```

Each cell ends in `cargo xtask assert-headline`.

To run against a server you started yourself, call the bench binary directly. Always pass `--sut <file> --require-sut` when the numbers will be shared:

```bash
metrum-ai-bench-cli-llm --url http://127.0.0.1:8000/v1/chat/completions --api-key dummy \
  --model Qwen/Qwen3-8B --mode chat --streaming --prompts prompts.jsonl --max-tokens 128 \
  --num-requests 64 --concurrency 4 --data-log llm.jsonl --sut sut.json --require-sut
```

The ASR and ImageGen guides have their own examples:
- [ASR.md](ASR.md): serving frameworks, valid audio, WER, and Whisper `--max-model-len 448`
- [IMAGEGEN.md](IMAGEGEN.md): vLLM-Omni, request knobs, and server defaults

LLM prompts come from Hugging Face [`metrum-ai/prompt-library`](https://huggingface.co/datasets/metrum-ai/prompt-library) through `metrum-ai-bench-cli-prompts` (defaults: config `sample`, profile `chat-short`); see [PROMPT_LIBRARY.md](PROMPT_LIBRARY.md). VLM, ASR, and ImageGen use the fixtures in `test-data/`.

## Pitfalls specific to this CLI

- **Thinking models** (Qwen3, Qwen3.8) emit reasoning tokens before visible text. Read [REASONING_MODELS.md](REASONING_MODELS.md) before choosing `--max-tokens`, and set `chat_template_kwargs` explicitly so the choice is recorded.
- **Preflight on thinking models.** `metrum-ai-bench-cli preflight` sends a small streaming probe (`max_tokens` 8). A thinking model can spend all of it on reasoning, so `streaming_first_token` passes with `first token was reasoning in N ms` when only reasoning deltas arrive (#230). Before that fix the check failed with `no output token` on a healthy server (`Qwen/Qwen3.8-27B-FP8` on 2026-10-02, `Qwen/Qwen3-8B` on vLLM 0.31.0 on 2026-10-05). To see a visible token instead, pass `--extra-body-json '{"chat_template_kwargs":{"enable_thinking":false}}'`. See [REASONING_MODELS.md](REASONING_MODELS.md#preflight-on-thinking-models).
- **Hybrid Mamba models** (Qwen3.8-27B) need `--max-num-seqs` at or below the Mamba cache block count vLLM reports. On 1x H100 PCIe that count was 793; the default of 1024 fails at engine start.
- **Run the bench on the serving host.** Driving a cloud IP from a laptop puts WAN round-trip time into TTFT.
- **Prefer vendor containers over `pip` wheels** on stock cloud images; see the docs site [Platforms](https://docs.metrum.ai/metrum-ai-bench-cli/latest/docs/platforms/) page.

## Telemetry

Run the Metrum all-smi fork (https://github.com/chetan-metrum-ai/all-smi) on the serving host with `all-smi api --port 9090`, and scrape it with the strategic binary's `--ndjson --telemetry`. See [TELEMETRY.md](TELEMETRY.md).

Check which path your installed binary serves before you rely on the example YAML. On 2026-10-02, v0.26.3-metrum.4 served `/metrics`, and `/metric` returned 404.
