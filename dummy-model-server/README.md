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
| `-port` | 8000 | Listen port (`0` picks a free port; the startup log line reports it) |
| `-model` | dummy | Model id |
| `-latency` | 0 | Delay before first token / non-stream body |
| `-chunk-interval` | 0 | Delay between stream token events |
| `-tokens-per-sec` | 0 | Token-bucket completion tokens/s (0 = off) |
| `-req-per-sec` | 0 | Token-bucket requests/s (0 = off) → HTTP 429 + `Retry-After` |
| `-max-concurrency` | 0 | Max in-flight `/v1` requests (0 = off) |
| `-error-rate` | 0 | Probability of 503 / mid-stream error |
| `-split-sse` | false | Flush mid-event SSE frames |
| `-omit-done` | false | Omit trailing `data: [DONE]` |
| `-done-tail` | 0 | Delay between `data: [DONE]` and the end of the stream body (e.g. `20ms`) |
| `-role-only` | false | Stream role delta only |
| `-reasoning` | false | Emit `delta.reasoning_content` before content |
| `-reasoning-tokens` | 0 | Emit N reasoning chunks, add N to `completion_tokens`, and report `usage.completion_tokens_details.reasoning_tokens` (non-streaming too); 0 = off |
| `-reasoning-only` | false | Stream `max_tokens` `delta.reasoning_content` chunks, no content, `finish_reason` `length` (a thinking model cut off mid-reasoning); `-role-only` wins if both are set, and `-error-rate` does not inject mid-stream errors here |
| `-include-usage` | true | Default usage on final stream chunk |
| `-seed` | 0 | RNG / image seed |
| `-compat` | openai | `openai` \| `vllm` \| `sglang` |
| `-strict-media` | false | Reject invalid image `data:` URLs and audio uploads with HTTP 400 (see below) |

## Strict media (`-strict-media`)

By default the server accepts any image or audio payload, which lets placeholder
media (1x1 images, zero-filled "MP3" stubs) slip through tests that a real vLLM or
SGLang deployment would fail. `-strict-media` turns on the Metrum AI
`internal/media` checks so the dummy server rejects what real servers reject.
With the flag off, behavior is unchanged.

What it validates:

- **Chat `image_url` parts** (`POST /v1/chat/completions`): every `data:` URL must
  be `data:<mime>;base64,<payload>`, decode as base64 (whitespace and missing
  padding tolerated), have a PNG, JPEG, GIF, or WebP header, and be at least 2x2
  pixels. BMP and TIFF are rejected even though vLLM (PIL) accepts them, so
  keep strict-mode fixtures to the four formats above. Failures return HTTP 400 with
  `{"error":{"message":"Invalid image: <detail>","type":"invalid_request_error","code":400}}`.
- **Audio uploads** (`POST /v1/audio/transcriptions`): the multipart `file` must be
  at least 1024 bytes and a recognized container. WAV files need a sane `fmt `
  chunk (known format tag, 1..8 channels, 1000..384000 Hz, valid bits per sample)
  and a non-empty, non-silent `data` chunk. MP3 files need two consecutive valid
  MPEG audio frame headers (after any ID3v2 tag) with a non-zero frame body.
  FLAC, OGG, M4A/MP4, and WebM are accepted by magic bytes when their body is not
  all zeros. Failures return HTTP 400 with JSON message
  `Invalid or unsupported audio file` (the same text vLLM returns).

What it does not do:

- It does not run a model or decode pixels or the full audio stream; only
  headers and frame structure are inspected.
- It does not fetch or check `http(s)` image references; those are accepted as is.
- It does not check that audio is speech or that an image has meaningful content.

```bash
go run ./cmd/dummy-model-server -port 8000 -strict-media
```

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
