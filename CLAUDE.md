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

## Xtask gates
- Headers/naming: `cargo xtask check-headers`
- CLI help doc: `cargo xtask render-cli-help` (after clap changes)
- Data points doc: `cargo xtask render-data-points`
- Headline gate: `cargo xtask assert-headline <modality> <data_log.jsonl>`

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
All benchmark-running guidance for agents lives in [AGENTS.md](AGENTS.md): the
default campaign order, hard rules, defaults, and truth order. Start there
before any run against a real model server.

## Available Tools
- `metrum-ai-bench-cli`: unified dispatcher for `llm`, `vlm`, `asr`, `imagegen`,
  `prompts`, `selftest`, `preflight`, `sut`, and `compare`.
- `metrum-ai-bench-cli-llm`: text chat/completions load measurement.
- `metrum-ai-bench-cli-vlm`: vision-language chat load measurement.
- `metrum-ai-bench-cli-asr`: audio transcription load measurement.
- `metrum-ai-bench-cli-imagegen`: image generation load measurement.
- `metrum-ai-bench-cli-prompts`: ISL/OSL mix selection from `metrum-ai/prompt-library`.
- `metrum-ai-bench-cli-strategic`: concurrency/rate sweeps, knee, sessions, telemetry NDJSON, and exports (separate binary).
- `metrum-ai-bench-cli-mock-server`: deterministic OpenAI-compatible mock for strategic fixtures (`--telemetry-fixture`).
