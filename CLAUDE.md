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
- Multi-host campaigns (intake, lane placement, sweeps, report, cleanup) follow `.claude/skills/campaign/SKILL.md`. The operator selects modalities in that intake. The default is LLM chat only.
- Before every benchmark against a real model: **web search online** for the current best-known / vendor-default serving config for that exact model and engine (vLLM, SGLang, etc.). Do not reuse memorized launch flags; record sources and chosen args in the SUT `runtime.config` / `notes`. See docs-site Agent-driven benchmarking.
- Search first covers more than launch flags. Check workload compatibility, the serving framework, the model card, request parameters, and a sensible concurrency/ISL/OSL sweep. Link out to upstream docs instead of copying recipes into the repo. Engine map: `docs/SERVING.md` (LLM and VLM on regular vLLM; ImageGen on vLLM-Omni; ASR on vLLM-Omni once vllm-omni#5722 lands, regular vLLM speech-to-text until then, see `docs/ASR.md`). Live status per modality: `docs/CLAIMS_LEDGER.md`.
- Prefer prebuilt binaries unless a from-source build is wanted. Order: (1) a release tarball's `ROOT/bin`, (2) an existing `target/release` or `target/rel-user/release` build in this checkout, (3) only then one `cargo build --release --bins` in this single checkout, never in another worktree. Build when the operator asks or when a needed fix exists only in the working tree. Live scripts resolve bins via `scripts/live/lib/bench_bin.sh` and fail rather than compile. Record the binary path, `--version`, and commit in SUT notes; tip builds print the last release version.
- Shadeform: when both an exported `SHADEFORM_API_KEY` and `env.json` are set and disagree, `scripts/live/shadeform.sh` prefers `env.json` and warns on stderr (no key material). Confirm every delete via `GET /instances/<id>/info`.
- Default publishable prompts: extract from Hugging Face `metrum-ai/prompt-library` (`metrum-ai-bench-cli-prompts`), not a handmade one-liner.
- `--api-key` is required with `--url`. Pass `dummy` for servers that do not check it.
- Always pass `--sut <file> --require-sut` for any run whose numbers will be shared. `examples/sut.example.json` is the template.
- Results go to `--data-log`; one JSONL record per request, final line is the run summary. Schema: `docs/OUTPUT_SCHEMA.md`.
- Thinking models: read `docs/REASONING_MODELS.md` before choosing `--max-tokens`. `no_output_token` in the summary means the cap was too low.
- Prompt files are JSONL only. Strategic sweeps: prefer `--prompts` + `--max-tokens` + `--warmup-requests` on GPU.
- Strategic telemetry: `--ndjson` + `--telemetry` YAML (Prometheus GET only). The series list is not compiled into the binary. Curl the live pages (Metrum all-smi fork at `http://127.0.0.1:9090/metrics`, the serving engine `/metrics`, and any other exporter) and set YAML `include` from that response. `/metric` returns 404 on all-smi v0.26.3-metrum.4. Example YAMLs are starting points. Offline analysis: `docs/TELEMETRY.md`, `docs/queries/analyze.py`. No in-binary SQL/`correlate`.
- Charts are not a CLI responsibility; analyze CSV/JSON/NDJSON with a separate prompt or script.
- `docs/CLI.md` is generated; run `scripts/render_cli_help.sh` after any clap change.
- Header check: `scripts/check_headers.sh`. Every file keeps the Metrum AI copyright and SPDX lines.
- No em dashes in any docs or user-facing strings.

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
