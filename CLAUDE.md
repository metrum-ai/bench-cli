<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Metrum AI Bench Development Guide

## Build & Test Commands
- Build: `cargo build` or `make debug`; release: `cargo build --release` or `make release`
- Clean: `cargo clean` or `make clean`
- Run a binary: `cargo run --bin <binary_name>`
- Run all tests: `cargo test --all-targets --all-features`
- Run a specific test: `cargo test <test_name>`
- Test with output: `cargo test -- --nocapture`

## Dummy Server Commands
- Run: `cd dummy-model-server && go run ./cmd/dummy-model-server`
- Test: `cd dummy-model-server && go test ./...`

## Code Style Guidelines
- **Formatting**: 4-space indentation; rustfmt for consistent style
- **Imports**: Standard library first, then external crates, then internal modules
- **Naming**: snake_case for variables/functions, CamelCase for types/traits/structs
- **Error Handling**: Use `anyhow` for propagation; avoid `unwrap()` in production code
- **Documentation**: Use rustdoc comments (`///`) for public APIs
- **Logging**: Structured logging with appropriate levels (error, warn, info, debug, trace)
- **Variables**: Prefer immutable (`let` over `let mut`) when possible
- **CLI Tools**: Use clap's derive API for parsing arguments; implement ValueEnum for enums
- **Testing**: Write unit tests for all public functions; use integration tests for tools

## Running a benchmark (agent notes)
- `--api-key` is required with `--url`. Pass `dummy` for servers that do not check it.
- Always pass `--sut <file> --require-sut` for any run whose numbers will be shared. `examples/sut.example.json` is the template.
- Results go to `--data-log`; one JSONL record per request, final line is the run summary. Schema: `docs/OUTPUT_SCHEMA.md`.
- Thinking models: read `docs/REASONING_MODELS.md` before choosing `--max-tokens`. `no_output_token` in the summary means the cap was too low.
- Prompt files are JSONL only.
- `docs/CLI.md` is generated; run `scripts/render_cli_help.sh` after any clap change.
- Header check: `scripts/check_headers.sh`. Every file keeps the Metrum AI copyright and SPDX lines.
- No em dashes in any docs or user-facing strings.

## Available Tools
- `metrum-ai-bench`: unified dispatcher for `llm`, `vlm`, `asr`, `imagegen`, `prompts`, and `selftest`.
- `metrum-ai-bench-llm`: text chat/completions load measurement.
- `metrum-ai-bench-vlm`: vision-language chat load measurement.
- `metrum-ai-bench-asr`: audio transcription load measurement.
- `metrum-ai-bench-imagegen`: image generation load measurement.
- `metrum-ai-bench-prompts`: ISL/OSL mix selection from `metrum-ai/prompt-library`.
- `metrum-ai-bench-strategic`: concurrency/rate sweeps, knee, sessions, and exports (separate binary).
- `metrum-ai-bench-mock-server`: deterministic OpenAI-compatible mock for strategic fixtures.
- Deprecated legacy modality shims remain for one compatibility release; do not document new usage against them.
