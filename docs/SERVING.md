<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Serving stacks for each modality

Metrum AI Bench CLI is a benchmark harness: it measures an OpenAI-compatible HTTP server that you start yourself. Use this page to pick the server for each modality, then follow the upstream links for recipes. This repository does not copy them.

**Search when the model is not exactly specified.** For an exact model ID with a recorded serving entry, use that entry and record its sources in SUT `runtime.config` and `notes`; if no entry exists, use engine defaults and state that choice. For an under-specified model, search current vendor docs and the model card, then record the pinned model, image tag, launch arguments, context/memory settings, request parameters, sources, and chosen sweep. CI smoke defaults in `.claude/skills/campaign/serving/ci-smoke.yaml` are for reproducible CI only, not live serving recommendations.

## Engine map

| Modality | Bench binary and endpoint | Serving framework | Example model we stage | Rust command | Upstream docs | Live verification |
|---|---|---|---|---|---|---|
| LLM | `metrum-ai-bench-cli-llm`, `/v1/chat/completions` | **Regular vLLM** (`vllm/vllm-openai`); SGLang also works | `Qwen/Qwen3-8B` | `cargo xtask serve llm {start,stop,print,logs,sut}` | [vLLM OpenAI server](https://docs.vllm.ai/en/latest/serving/openai_compatible_server.html), [reasoning outputs](https://docs.vllm.ai/en/latest/features/reasoning_outputs.html), [vLLM recipes](https://docs.vllm.ai/projects/recipes/en/latest/), [SGLang](https://docs.sglang.ai/) | [Ledger](CLAIMS_LEDGER.md) |
| VLM | `metrum-ai-bench-cli-vlm`, `/v1/chat/completions` with `image_url` parts | **Regular vLLM** multimodal | `Qwen/Qwen3-VL-8B-Instruct` | `cargo xtask serve vlm {start,stop,print,logs,sut}` | [vLLM multimodal inputs](https://docs.vllm.ai/en/latest/features/multimodal_inputs.html), [Qwen3-VL recipe](https://docs.vllm.ai/projects/recipes/en/latest/Qwen/Qwen3-VL.html) | [Ledger](CLAIMS_LEDGER.md) |
| ASR | `metrum-ai-bench-cli-asr`, `/v1/audio/transcriptions` | **vLLM-Omni** is the intended stack. Until [vllm-omni#5722](https://github.com/vllm-project/vllm-omni/issues/5722) lands, serve with regular vLLM speech-to-text (see [ASR.md](ASR.md#serving-frameworks)). | `openai/whisper-large-v3-turbo` | `cargo xtask serve asr {start,stop,print,logs,sut}` | [vLLM speech-to-text](https://docs.vllm.ai/en/latest/serving/online_serving/speech_to_text/), [vLLM-Omni docs](https://docs.vllm.ai/projects/vllm-omni/en/latest/) | [Ledger](CLAIMS_LEDGER.md) |
| ImageGen | `metrum-ai-bench-cli-imagegen`, `/v1/images/generations` | **vLLM-Omni** (`vllm serve <model> --omni`) | `Tongyi-MAI/Z-Image-Turbo` | `cargo xtask serve imagegen {start,stop,print,logs,sut}` | [vLLM-Omni image generation API](https://docs.vllm.ai/projects/vllm-omni/en/latest/serving/image_generation_api/), [vLLM-Omni docs](https://docs.vllm.ai/projects/vllm-omni/en/latest/) | [Ledger](CLAIMS_LEDGER.md) |

The Live verification column always defers to [CLAIMS_LEDGER.md](CLAIMS_LEDGER.md). Supported in code is not the same as verified against a real server.

**Why ASR is not on vLLM-Omni yet (checked 2026-10-02).** In vllm-omni v0.30.0, starting a server with `--omni` sets the engine's supported tasks to `generate` and `speech` only (`vllm_omni/engine/omni_engine_base.py`). The API server therefore never builds the transcription handler, and `/v1/audio/transcriptions` is unavailable under `--omni` for every model. Upstream tracks the fix in [vllm-omni#5722](https://github.com/vllm-project/vllm-omni/issues/5722).

Regular vLLM documents Whisper on that endpoint today ([speech-to-text](https://docs.vllm.ai/en/latest/serving/online_serving/speech_to_text/)), so the checked-in CI serving data uses regular vLLM. Re-validate on vLLM-Omni before calling ASR verified on it.

## Use prebuilt binaries

Unless you want a from-source build, run binaries that already exist. Check these in order:

1. An unpacked GitHub Release tarball: `ROOT/bin/metrum-ai-bench-cli*`.
2. An existing release build in this checkout: `target/release/`, or `target/rel-user/release/` if the release was built with `--target-dir` because `target/release` was not writable.
3. Only if neither has what you need, run one `cargo build --release --bins` in the single primary checkout, then reuse that output. Never build in a second git worktree.

Building is the right call in two cases: the operator asks for a from-source build, or the fix you need exists only in the working tree. Debug builds are fine for plumbing tests, but never for numbers you report.

`cargo xtask bench-bin <binary>` resolves prebuilt binaries in this order: `BENCH_BIN_DIR`, `ROOT/bin`, `target/release`, `target/rel-user/release`, then `PATH`. It never compiles or selects a debug build implicitly. `cargo xtask local-smoke` records the resolved binary path, `--version`, and checkout `git describe` in the SUT.

## One example per modality

```bash
# Uses existing binaries; see "Use prebuilt binaries"
cargo xtask serve <llm|vlm|asr|imagegen> start
cargo xtask local-smoke <llm|vlm|asr|imagegen>
cargo xtask serve <modality> stop
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

Run the Metrum all-smi fork (https://github.com/chetan-metrum-ai/all-smi) on the serving host with `all-smi api --port 9090`, and scrape it with `--ndjson --telemetry` on `metrum-ai-bench-cli-strategic`, `-llm`, `-vlm`, `-asr`, or `-imagegen`. See [TELEMETRY.md](TELEMETRY.md).

Check which path your installed binary serves before you rely on the example YAML. On 2026-10-02, v0.26.3-metrum.4 served `/metrics`, and `/metric` returned 404.
