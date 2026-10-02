<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Serving stacks for each modality

Metrum AI Bench CLI is a client. It measures an OpenAI-compatible HTTP server that you start yourself. This page tells you which server to start for each modality and links to the upstream documentation; it does not reproduce upstream recipes.

Run the bench on the same host as the server, against loopback. Driving a cloud IP from a laptop adds WAN round-trip time to TTFT.

| Modality | Bench binary and endpoint | Recommended engine | Example model we stage | Live launcher | Upstream docs | Live verification |
|---|---|---|---|---|---|---|
| LLM | `metrum-ai-bench-cli-llm`, `/v1/chat/completions` | vLLM (`vllm/vllm-openai`) or SGLang (`lmsysorg/sglang`) | `Qwen/Qwen3-8B` | [`scripts/live/serve/llm.sh`](../scripts/live/serve/llm.sh) | [vLLM OpenAI server](https://docs.vllm.ai/en/latest/serving/openai_compatible_server.html), [reasoning outputs](https://docs.vllm.ai/en/latest/features/reasoning_outputs.html), [SGLang](https://docs.sglang.ai/) | [Ledger](CLAIMS_LEDGER.md) |
| VLM | `metrum-ai-bench-cli-vlm`, `/v1/chat/completions` with `image_url` parts | vLLM multimodal | `Qwen/Qwen3-VL-8B-Instruct` | [`scripts/live/serve/vlm.sh`](../scripts/live/serve/vlm.sh) | [vLLM multimodal inputs](https://docs.vllm.ai/en/latest/features/multimodal_inputs.html), [Qwen3-VL recipe](https://docs.vllm.ai/projects/recipes/en/latest/Qwen/Qwen3-VL.html) | [Ledger](CLAIMS_LEDGER.md) |
| ASR | `metrum-ai-bench-cli-asr`, `/v1/audio/transcriptions` | vLLM speech-to-text | `openai/whisper-large-v3-turbo` | [`scripts/live/serve/asr.sh`](../scripts/live/serve/asr.sh) | [vLLM speech-to-text](https://docs.vllm.ai/en/latest/serving/online_serving/speech_to_text/) | [Ledger](CLAIMS_LEDGER.md) (functional only) |
| ImageGen | `metrum-ai-bench-cli-imagegen`, `/v1/images/generations` | vLLM-Omni (`vllm serve <model> --omni`) | `Tongyi-MAI/Z-Image-Turbo` | [`scripts/live/serve/imagegen.sh`](../scripts/live/serve/imagegen.sh) | [vLLM-Omni image generation API](https://docs.vllm.ai/projects/vllm-omni/en/latest/serving/image_generation_api/), [vLLM-Omni docs](https://docs.vllm.ai/projects/vllm-omni/en/latest/) | [Ledger](CLAIMS_LEDGER.md) (not verified live) |

The Live verification column always defers to [CLAIMS_LEDGER.md](CLAIMS_LEDGER.md). Supported in code is not the same as verified against a real server.

The launchers follow the same rules for every modality:
- **Pinned images.** They pin `vllm/vllm-openai:v0.30.0` and `vllm/vllm-omni:v0.30.0`. Flags were looked up on 2026-10-02, and each script's header cites its sources.
- **SUT output.** Each launcher writes a SUT file that records the exact `docker run` command in `runtime.config`, the Hub model revision, the GPU, and the sources.
- **Other models.** Override with `MODEL=...` plus `SERVE_ARGS_OVERRIDE` / `SOURCES_OVERRIDE` / `SUT_NOTES_OVERRIDE`, and record why in the notes.

Whatever model you serve, look up that model's current vendor-default flags before the run, and record them in the SUT. Do not reuse remembered flags.

## One example per modality

Each example starts the server with the launcher, then runs one smoke cell that ends in [`scripts/live/assert_headline.sh`](../scripts/live/assert_headline.sh):

```bash
cargo build --release --bins
scripts/live/serve/<llm|vlm|asr|imagegen>.sh start
scripts/live/local_smoke.sh --local --modality <llm|vlm|asr|imagegen>
scripts/live/serve/<modality>.sh stop
```

To run against a server you started yourself, use the bench binary directly. Always pass `--sut <file> --require-sut` when the numbers will be shared:

```bash
# LLM / VLM (vLLM)
metrum-ai-bench-cli-llm --url http://127.0.0.1:8000/v1/chat/completions --api-key dummy \
  --model Qwen/Qwen3-8B --mode chat --streaming --prompts prompts.jsonl --max-tokens 128 \
  --num-requests 64 --concurrency 4 --data-log llm.jsonl --sut sut.json --require-sut
```

The ASR and ImageGen guides have their own examples:
- [ASR.md](ASR.md): serving frameworks, valid audio, WER, and Whisper `--max-model-len 448`
- [IMAGEGEN.md](IMAGEGEN.md): vLLM-Omni, request knobs, and server defaults

## Engine notes that affect the numbers

- **Thinking models** (Qwen3, Qwen3.8) put reasoning tokens before visible text. Read [REASONING_MODELS.md](REASONING_MODELS.md) before choosing `--max-tokens`, and pass `chat_template_kwargs` explicitly so the setting is recorded.
- **Preflight cannot disable thinking.** `metrum-ai-bench-cli preflight` has no `--extra-body-json`, so on a thinking model its `streaming_first_token` check fails even when the server is healthy. It failed this way on `Qwen/Qwen3.8-27B-FP8` on 2026-10-02.
- **Hybrid Mamba models need a lower `--max-num-seqs`.** Qwen3.8-27B on one 80 GB GPU needs `--max-num-seqs` at or below the number of Mamba cache blocks vLLM reports (793 on 1x H100 PCIe). The default of 1024 fails at engine start.
- **Use vendor containers, not `pip` wheels,** on stock cloud GPU images; see the docs site [Platforms](https://docs.metrum.ai/metrum-ai-bench-cli/latest/docs/platforms/) page.

## Telemetry

Run the Metrum all-smi fork (https://github.com/chetan-metrum-ai/all-smi) on the serving host with `all-smi api --port 9090`, and scrape it with the strategic binary's `--ndjson --telemetry`. See [TELEMETRY.md](TELEMETRY.md).

The fork's v0.26.3-metrum.4 release serves `/metrics`. On 2026-10-02 its `/metric` path returned 404, so check the path your installed binary serves before relying on the example YAML.
