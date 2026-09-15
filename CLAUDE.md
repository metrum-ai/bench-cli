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

## Available Tools
Core: `metrum-ai-bench` with `llm`, `vlm`, `asr`, and `imagegen` subcommands.
Strategic: `metrum-ai-bench-strategic` for sweeps, validity checks, and exports.