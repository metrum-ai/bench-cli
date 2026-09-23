<!-- Copyright (c) 2026 Metrum AI, Inc. SPDX-License-Identifier: Apache-2.0 -->

# dummy-model-server

OpenAI / vLLM / SGLang–compatible HTTP stub for hermetic **metrum-ai-bench-cli** tests across LLM, VLM, ASR, and image-generation modalities. Copyright (c) 2026 Metrum AI, Inc. Licensed under Apache-2.0.

## Build & run

```bash
cd dummy-model-server
go test ./...
go vet ./...
go run ./cmd/dummy-model-server -port 8000 -latency 100ms -chunk-interval 20ms
```

Binary:

```bash
go build -o bin/dummy-model-server ./cmd/dummy-model-server
./bin/dummy-model-server -port 8000
```

`GET /health` returns 200 for `wait_for_vllm`.

GitHub Release archives ship this binary at `bin/dummy-model-server` for Linux
and macOS (`x86_64` and `aarch64`). Unpacking a release does not require Go.

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `-port` | 8000 | Listen port |
| `-model` | dummy | Model id |
| `-latency` | 0 | Delay before first token / non-stream body |
| `-chunk-interval` | 0 | Delay between stream token events |
| `-tokens-per-sec` | 0 | Token-bucket completion tokens/s (0 = off) |
| `-req-per-sec` | 0 | Token-bucket requests/s (0 = off) → HTTP 429 + `Retry-After` |
| `-max-concurrency` | 0 | Max in-flight `/v1` requests (0 = off) |
| `-error-rate` | 0 | Probability of 503 / mid-stream error |
| `-split-sse` | false | Flush mid-event SSE frames |
| `-omit-done` | false | Omit trailing `data: [DONE]` |
| `-role-only` | false | Stream role delta only |
| `-reasoning` | false | Emit `delta.reasoning_content` before content |
| `-include-usage` | true | Default usage on final stream chunk |
| `-seed` | 0 | RNG / image seed |
| `-compat` | openai | `openai` \| `vllm` \| `sglang` |

## Timing model (appendix 8.3)

With `-latency 100ms -chunk-interval 20ms` and `max_tokens=20`:

- TTFT ≈ 120 ms (latency + first chunk interval)
- Response time ≈ 500 ms (100 + 20×20)
- One SSE `data:` event per token, final chunk with `usage`, then `data: [DONE]`

Prompt tokens = `len(content)/4` over message text (`image_url` adds 256).

## Endpoints

- `GET /health`, `/healthz`, `/ready`
- `GET /v1/models`
- `POST /v1/chat/completions` (string or multimodal `image_url` content; stream + non-stream)
- `POST /v1/completions`
- `POST /v1/audio/transcriptions` (multipart `file` + `model`)
- `POST /v1/images/generations` (`prompt`, `size`, `n` → `b64_json`)

## Docker Compose soak

```bash
docker compose -f dummy-model-server/docker-compose.yaml up --build
```

| Service | Port | Rate ceilings |
|---------|------|---------------|
| llm | 8000 | 50 req/s, 2000 tokens/s |
| vlm | 8001 | 20 req/s, 500 tokens/s |
| asr | 8002 | 10 req/s |
| imagegen | 8003 | 5 req/s |

## Compat notes

- **vLLM:** honors `ignore_eos` (fixed `max_tokens` length), `stream_options.include_usage`, optional `-reasoning` deltas.
- **SGLang:** unknown request JSON fields do not 400.
- **OpenAI:** standard chat / completions / transcriptions / images surface.
