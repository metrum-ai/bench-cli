# Changelog

## 1.0.0 (2026-09-18)

First stable release after the 1.0.0-rc series.

### Added
- `docs/REASONING_MODELS.md`: operator guide for thinking models (TTFT vs
  first reasoning, `--extra-body-json` / `reasoning_effort`, `--max-tokens`
  probe procedure). Linked from README, METRICS, LIMITATIONS, and CLAUDE.md.
- README Quickstart (60 seconds): publishable dummy-server LLM run with
  `--sut` / `--require-sut` and `test-data/llm-hi.jsonl`.
- CLAUDE.md agent notes for running benchmarks and listing every non-deprecated
  binary.
- `--quiet` and `NO_BANNER=1`: suppress ASCII banner art; one-line identity
  remains. Documented in README Install and regenerated `docs/CLI.md`.

### Fixed
- `metrum-ai-bench-prompts` Hub checksum verification is scoped to the
  requested dataset config so `full` and `sample` no longer collide on shared
  parquet basenames (#101).
- Prompt mix selection prefers a unique draw from an exact ISL/OSL cell before
  the sparse hill-climber, so `--count-slack 0` and `--no-repeats` succeed when
  the target bucket is fully populated (#102).
- `metrum-ai-bench-prompts` and `metrum-ai-bench-strategic` expose clap
  `-V` / `--version`; strategic also supports `--version-only` (#99).
- `docs/RELEASING.md` cosign verify example uses the v-prefixed archive names
  that the release workflow actually attaches (#100).

### Changed
- `--url` and `--api-key` are mutually required at clap parse time on llm,
  vlm, asr, and imagegen (endpoints-file path unchanged). Help text states
  Bearer-token semantics and the `dummy` placeholder.
- `--extra-body-json` help points at reasoning_effort examples and the run
  manifest; imagegen `--extra-body-file` help filled; `scripts/render_cli_help.sh`
  covers unified, strategic, and mock-server.
- Banner prints only on interactive TTY after argument parse; non-TTY / quiet
  sessions get `Metrum AI Bench <tool> <version>`.
- README reordered for agent scanning: Quickstart, Tools entry points,
  Reasoning models, Publishing (with SUT example), Prompt library, Dummy
  server. Dummy-server `go run` commands use `(cd dummy-model-server && …)`
  because the Go module lives in that subdirectory.
- `docs/METRICS.md`: TTFT vs first reasoning under its own heading.
- `docs/LIMITATIONS.md`: NVIDIA smoke matrix wording is coverage-only; SUT
  flags described as shipped.

### Planned
- SUT block should carry first-class dataset provenance fields (`dataset`,
  `dataset_revision`, `dataset_rows`) rather than only free-form `extra` /
  notes.

## 1.0.0-rc.6 (2026-09-17)

Release candidate: `rand` 0.10.2 (soundness) and prompt-library mix extractor.

### Added
- `metrum-ai-bench-prompts` (also `metrum-ai-bench prompts -- …`): select a
  reproducible ISL/OSL mix from
  [`metrum-ai/prompt-library`](https://huggingface.co/datasets/metrum-ai/prompt-library)
  by mean or median within absolute tolerances; preferred `--count` may vary
  within `--count-slack` and source rows may repeat. Writes JSONL for
  `metrum-ai-bench-llm` plus a selection report with recommended
  `--num-requests` / `--max-tokens`. Docs: `docs/PROMPT_LIBRARY.md`.

### Changed
- Direct dependency `rand` 0.9.5 → 0.10.2 (soundness fixes in 0.10.1/0.10.2;
  `Rng` → `RngExt` call sites). Transitive `tokenizers` still uses `rand` 0.9.x.

## 1.0.0-rc.5 (2026-09-17)

Release candidate: naming alignment, publication SUT block, policy drafts, and CI gates.

### Added
- `--sut <PATH>` embeds an operator-declared system-under-test block (JSON/YAML) into the summary as `sut`, labelled `provenance: "declared"`. Absent → `"sut": null` plus a stderr notice.
- `--require-sut` (env `METRUM_AI_BENCH_REQUIRE_SUT=1`) refuses to run without a valid SUT block; implies `--redact-hostname`. Use for any run intended for publication.
- `--redact-hostname` (env `METRUM_AI_BENCH_REDACT_HOSTNAME=1`) writes `environment.hostname: null`.
- `examples/sut.example.{json,yaml}`.
- TRADEMARKS.md, docs/RESULTS_PUBLICATION_POLICY.md, docs/CLAIMS_LEDGER.md, docs/NAMING.md (drafts pending counsel review).
- docs/LIMITATIONS.md; docs/datasets/DATASET_CARD.draft.md (not published).
- docs/RELEASING.md (publish variables and cosign notes).

### Changed
- Crate renamed `metrumbench` → `metrum-ai-bench` to match the `metrum-ai-bench` binary. Not yet published to crates.io; no migration needed.
- Mock server binary renamed `metrumbench-mock-server` → `metrum-ai-bench-mock-server`.
- MLPerf interoperability export: SUT name field `"MetrumBench"` → `"Metrum AI Bench"` (label only; no metric or schema change).
- Tarball prefix and Homebrew formula follow the crate name.
- Summary schema v3: optional `sut` field added; `environment.hostname` is now nullable. Additive; readers must treat both as optional.
- `scripts/live/campaign.sh` / `matrix_smoke.sh` pass `--sut sut.json`.
- docs/COMPARISON.md rewritten against current code and the September 2026 landscape (AIPerf replaces retired GenAI-Perf; InferenceX and vLLM/SGLang bench_serving added; MLPerf export described as the unofficial interoperability export it is).
- docs/reviews/QUALITY_ASSESSMENT_REPORT.md carries a disposition header; current verdict lives in SCORECARD_1.0.0.md.
- docs/SMOKE_RESULTS.md states the matrix is NVIDIA-only with Instinct in progress.
- README: publishing-a-result section, security and provenance, known limitations, trademark notice.
- CI enforces the product-naming rule (docs/NAMING.md) via scripts/check_headers.sh; DCO sign-off enforced on PRs.
- crates.io and Homebrew publish now require explicit repository variables (`CRATES_IO_PUBLISH`, `HOMEBREW_PUBLISH`) and never run for `-rc.` tags.
- Build-provenance attestation is required when the repository is public.
- Added CodeQL (Rust), cargo-geiger, cargo-outdated/udeps weekly jobs; gitleaks custom rules for account IDs, cleartext passwords, SSH public keys, internal hostnames.

## 1.0.0-rc.4 (2026-09-16)

Release candidate after retracting the mistagged GA and Dependabot maintenance.

- Release hygiene: deleted mistagged non-prerelease `v1.0.0` (tag + GitHub
  Release) so `1.0.0` remains available for eventual GA; marked existing
  `1.0.0-rc.*` releases as prerelease; release workflow now sets `prerelease`
  automatically for `-rc.` / `-alpha.` / `-beta.` tags.
- Dependencies: `base64` 0.23.1, `sha2` 0.11.0, optional `tokenizers` 0.23.2;
  GitHub Actions bumps for checkout, setup-go, rust-cache, upload-artifact, and
  softprops/action-gh-release.

## 1.0.0-rc.3 (2026-09-16)

Release candidate: cross-platform release binaries via cargo-zigbuild.

- Release pipeline: tagged builds use a pinned `ghcr.io/rust-cross/cargo-zigbuild`
  image on Linux instead of native macOS / ARM Ubuntu compile runners. Targets
  remain `x86_64`/`aarch64` `*-unknown-linux-gnu` (dynamically linked glibc,
  2.17 floor) and `*-apple-darwin`. Not musl; TLS remains rustls.
- Binary identity differs from rc.2 (Zig linker / glibc floor / Darwin SDK in
  the zigbuild image). Compile-free smoke jobs unpack each archive and run
  `metrum-ai-bench --help` / `selftest` on matching Linux and macOS runners
  before GitHub Release, crates.io, and Homebrew publish.

## 1.0.0-rc.2 (2026-09-16)

Release candidate after the 1.0.0 measurement residuals and OSS-readiness docs.

- Measurement residuals (N-02–N-07, N-09, F-09, F-14): stamp monotonic
  `send_offset_s` and derive the window and closed-loop bins from it; normalize
  trailing/short throughput bins by actual width; VLM maps in-stream `error`
  events and writes failed records on preprocess/body-build skips; stamp
  `effective_max_concurrency`; classify TCP reset as `connect` and use Instant
  for failure latency; drop `imagegen.request.v1` (artifact SHA-256 on
  `request.v3` `modality_labels`); omit zero token throughput for non-token
  modalities; MLPerf export no longer contains `Result is : VALID`; clarify
  `--ca-cert` must be a CA certificate.
- Docs / packaging (OSS readiness): public README install and examples; move ASR
  notes under `docs/`; Dependabot + weekly `cargo deny`; SECURITY no-unsafe
  sentence; tighter `Cargo.toml` exclude; smoke matrix and true `ttft_s` in
  `SMOKE_RESULTS` (N-01).

## 1.0.0-rc.1 (2026-09-15)

Feature baseline for the 1.0 line. A mistagged non-prerelease `v1.0.0` pointing
at this same line was deleted; `1.0.0` is reserved for the eventual GA.

Breaking / schema notes:
- New campaigns reject unversioned or legacy (non-`request.v*` / `summary.v*`) JSONL lines.
- Field-additive schema bump to `request.v3` / `summary.v3` / related config stamps (existing numeric meanings unchanged; keep a v2 reader for 0.1.82 audit).
- Dual unversioned modality summaries removed; console and JSONL use `RunSummary` only.
- `metrumbench-*` shims remain through v1.x and will be removed in v2.0.
- Verified on Shadeform RTXPro6000 campaign `v1rc1-20260915-190838` (tag `v1.0.0-rc.1`); see `docs/SMOKE_RESULTS.md`.

- Public readiness: full `cargo deny check` in CI; replace `ntp`/`lru` (std SNTP +
  hand-rolled VLM image LRU); drop compile-time wall-clock datetime for
  reproducible builds; MSRV 1.85 CI job; SHA-pinned Actions; govulncheck for
  dummy-model-server; dummy-server body limits / timeouts / non-root Docker;
  issue/PR templates; refreshed `THIRD_PARTY_LICENSES`.
- Remove dual legacy summaries (F-05, F-25, F-26): modality binaries write only
  `request.v3` + `summary.v3` via `JsonlSink`; console stats render from
  `RunSummary` / `DistSummary` (type 7). Imagegen `--summary-json` writes
  summary.v3 (no stdout pretty duplicate). Unused `modality.rs` / `transport.rs`
  deleted. `metrumbench-*` shims kept through v1.x with explicit v2.0 removal
  notices.
- Strategic honesty (F-14, F-16): sweep points carry DistSummary (`n`, errors,
  p99_unreliable), redacted config, shared warm HTTP pool; `--slo e2e=` for
  goodput (without SLOs `goodput_equals_throughput`); MLPerf export files start
  with an UNOFFICIAL disclaimer and never emit bare `Result is : VALID`.
- Campaign `validate` rejects unversioned/legacy JSONL lines; `request.v2` /
  `summary.v2` remain accepted for 0.1.82 regression audit
  (`record::accepts_audit_schema`).
- Modality CLI parity (F-08, F-10, F-21, F-22, F-28): non-streaming LLM reports `ttft_s: null` and stops the clock after the full body is read; VLM honors `--system-prompt` / `--min-tokens` / `--tokenizer` and stamps `effective_system_prompt`; ASR drops conflicting legacy `throughput.rtfx` (measured-phase `rtfx_client` only); imagegen accepts base or full `/images/generations` URLs, excludes decode/hash/write from service latency, and uses the measured-phase window for legacy throughput; `--summary-json` is optional for imagegen; shared `--fail-on-error` (default off) for consistent exit policy across modalities.
- Transport parity (F-06, F-12, F-13, F-23): typed `RequestError` mapping at the failure site via `from_reqwest` / `from_status` (timeout/connect/5xx no longer depend on Display substrings); optional `first_byte_s` on `request.v3`; shared `--ca-cert` / `--insecure` (stamped into `config.common`, never secrets); least-inflight temporary ejection after connect failure; SSE blank-line framing with multiline `data:` joined by `\n`.
- Deferred stretch: `connect_s` (connection-established Instant) remains post-v1 / feature-flagged; TTFT continues to include connect by design.
- Summary v3 effective config: stamp `config` (`run_id`, common args, effective system prompt, sanitized `body_template`, unique-prompt nonce template) on all four binaries; unique-prompt nonces are `[nonce-{run_id}-{seed}-{seq}]`. Add `usage_missing_count`, nullable `completion_tokens_per_second` with `completion_tokens_source` (`server_usage` / `tokenizer_fallback`), and `p90_unreliable` / `p95_unreliable` on distributions.
- Modality runners (VLM, ASR, imagegen): roll out the shared `runner.rs` contract already used by LLM: in-task `started_at` / flush via `JsonlSink`, SIGINT+SIGTERM `StopFlag`, closed-loop schedule omission, and `window_seconds` from measured record span. VLM no longer drops warmup records (metrics skip by `phase` only).
- LLM runner: shared `runner.rs` timestamps send/completion inside the task, writes `request.v3` immediately, handles SIGINT/SIGTERM, and derives `window_seconds` from records (closed-loop ~7.9 req/s at c=4/n=16 on the dummy). Schema bump to `request.v3` / `summary.v3` (field-additive). E2e covers window, flush-during-launch, and SIGTERM JSONL prefix.

## v0.1.82 (2026-09-15)

- Load scheduler: `FakeClock` for deterministic open-loop tests; `docs/CLI.md` regenerated from clap `--help` via `scripts/render_cli_help.sh`.

## v0.1.81 (2026-09-15)

- ASR: `--normalizer {whisper-english,whisper-basic,none}` selects the text normalization applied to both sides of WER/CER, and the choice is recorded in `config.normalizer`. WER/CER are pinned by a hand-computed reference table.
- VLM: source image bytes are sent unchanged unless `--max-image-dimension` forces a resize or the new `--reencode-jpeg` is requested; images are no longer decoded when neither applies. Per-request records carry `modality_metrics.image_bytes` and `image_count`.
- Dummy-server end-to-end coverage for VLM (streaming TTFT/ITL, non-streaming without a fabricated TTFT, payload preservation), ASR (RTFx, normalizer selection), imagegen (monotonic latency, warmup exclusion, shared summary), and seeded open-loop determinism for constant and Poisson arrivals.
- Adversarial stream coverage: SSE frames flushed mid-event, a missing `data: [DONE]` sentinel, role-only streams classified `no_output_token`, and reasoning deltas kept out of TTFT. Ctrl-C is covered end to end: records stay on disk and the summary is marked `partial`.
- CI runs the dummy-server end-to-end tests instead of skipping them (`METRUM_BENCH_REQUIRE_DUMMY=1`), and gates `gofmt`.

## v0.1.80 (2026-09-14)

- Measurement core: complete-line SSE parser, Hyndman–Fan type 7 percentiles, ITL vs N−1 TPOT, per-request JSONL (`metrum-ai-bench.request.v2`), Ctrl-C partial summaries, `--warmup-requests`, `--seed`, `--request-rate` / `--arrival`, `--ignore-eos`, `--extra-body-json`, `--unique-prompts`.
- VLM: optional `--streaming` TTFT (non-streaming no longer fabricates TTFT); images preloaded before the measurement window.
- ASR: Whisper-like text normalization for WER; request clock starts after audio is read; `throughput.rtfx` = total audio seconds / wall time.
- Imagegen: monotonic `Instant` latency; seeded prompt shuffle; `--warmup-requests`.
- Dummy-server e2e test for LLM streaming timing (TTFT ~120 ms, RT ~500 ms at latency=100ms, chunk-interval=20ms, max_tokens=20).

## v0.1.79 (skipped)

- Intentionally skipped; numbering jumps from v0.1.78 to v0.1.80. No release artifacts were published for v0.1.79.

## v0.1.78 (2026-05-02)

- Extended license validity date until July 31, 2026.
- Updated license check tests and operator-facing license documentation to reflect the new expiry date.
- **metrumbench-llm**: treat `--ramp-up-seconds 0` as no ramp-up so throughput
  denominators cover the actual measured workload instead of a late
  post-drain metrics window. Positive ramp-up values retain ramp-up behavior.

## v0.1.77 (2026-02-19)

- Version bump (metrumbench crate). Multi-endpoint support remains at v0.1.76 behavior for metrumbench-llm, metrumbench-vlm, and metrumbench-asr.
- **metrumbench-asr**: `--input` and `--ground-truth` accept a local JSONL path or an `http://` / `https://` URL (blocking GET; same pattern as metrumbench-llm prompt URLs). Shared helper `read_utf8_from_path_or_url` in `metrumbench::prompt_inputs`.
- Dummy model server (metrumbench-vlm mode): accept `content` as string or array of parts for compatibility with metrumbench-vlm client.

## v0.1.76 (2026-02-19)

### metrumbench-vlm and metrumbench-asr multi-endpoint support

- **metrumbench-vlm** and **metrumbench-asr** now support the same multi-endpoint workflow as metrumbench-llm: optional `--endpoints-file` (YAML), weighted round-robin, per-endpoint and aggregate metrics, and data_log schema with `config.endpoint` / `config.endpoints` and `metrics.per_endpoint`.
- **metrumbench-vlm**: `--url` and `--api-key` are optional when `--endpoints-file` is provided; exactly one of (single `--url`/`--api-key`) or `--endpoints-file` is required.
- **metrumbench-asr**: Same mutual exclusivity; provide either `--url` + `--api-key` or `--endpoints-file`. Other required args (scenario, num_requests, input, model) unchanged.
- Shared endpoint resolution lives in the `metrumbench` lib (`metrumbench::endpoints`) for all three tools.

## v0.1.75 (2026-02-19)

### metrumbench-llm multi-endpoint support

- **Multi-endpoint benchmarking**: Use `--endpoints-file` with a YAML file to distribute requests across multiple endpoints with per-endpoint credentials and optional weights (weighted round-robin).
- **Single-endpoint unchanged**: `--url` and `--api-key` still work as before when not using `--endpoints-file`. Exactly one of (single `--url`/`--api-key`) or `--endpoints-file` is required.
- **Per-endpoint and aggregate metrics**: Console output and data_log summary include per-endpoint blocks and an AGGREGATE block when multiple endpoints are used. JSONL summary adds `config.endpoint` / `config.endpoints` and `metrics.per_endpoint`.

### Deprecation notice (data_log / JSONL output)

- **The previous data_log / JSONL output shape, format, and schema is deprecated as of this release.**
- **The old format will be disabled (removed) in the next release.** Consumers that parse the data_log JSONL line must migrate to the new schema (e.g. `config.endpoint` or `config.endpoints`, `metrics.per_endpoint`) before upgrading to the next release.

## v0.1.68-beta (2025-04-09)
- Updated rand crate to version 0.8.5 for improved random number generation
- Enhanced random word selection for unique ID generation
- Improved code quality and maintainability
- Fixed deprecated function usage in random number generation
- Updated documentation to reflect dependency changes

## v0.1.64 (2025-04-08)
- Extended license validity date until May 31, 2025
- Updated license check message to reflect new expiry date
- Fixed bug in metrumbench-asr tool that prevented running more requests than available audio samples
- Implemented round-robin audio sample selection for longer benchmark runs
- Minor documentation improvements

## v0.1.63 (2025-04-07)
- Enhanced metrumbench-asr audio transcription benchmarking tool
- Improved accuracy metrics collection and reporting
- Added support for comprehensive audio format analysis
- Enhanced error handling and retry logic
- Updated documentation for audio transcription features

## v0.1.62 (2025-04-06)
- Added support for direct file path audio sources in metrumbench-asr
- Enhanced audio format detection and validation
- Improved caching mechanism for audio files
- Updated documentation with new audio source examples

## v0.1.61 (2025-04-06)
- Initial release of metrumbench-asr audio transcription benchmarking tool
- Support for OpenAI Whisper and compatible APIs
- Implementation of detailed accuracy metrics (WER, CER, RTF)
- Basic audio format support and caching

## v0.1.60 (2025-04-05)
- Enhanced ramp-up functionality with improved metrics collection
- Added validation for ramp-up period configuration
- Updated documentation with comprehensive ramp-up examples
- Improved error handling during ramp-up transitions

## v0.1.59 (2025-04-05)
- Added ramp-up functionality for gradual concurrency increase
- Implemented configurable ramp-up period in seconds
- Added validation for ramp-up period against stop-after period
- Enhanced metrics collection to separate ramp-up and steady-state periods
- Updated documentation with ramp-up examples and usage guidelines
- Fixed unused variable warning in rustyphalanx.rs

## v0.1.58 (2025-04-04)
- Added server-side image download option to metrumbench-vlm tool
- Enhanced image handling with configurable download modes
- Updated documentation with new feature examples
- Improved image URL handling in request payloads
- Added support for direct URL passing in vision model requests
- Fixed unused import warning in rustyphalanx.rs

## v0.1.57 (2025-04-03)
- Added platform-specific release archives (macos/linux, arm64/x86_64)
- Updated build system to use gtar when available on macOS
- Enhanced release naming convention for better platform support
- Updated documentation for platform-specific releases
- Improved build system integration for cross-platform support

## v0.1.56 (2025-03-25)
- Updated Python dependencies with explicit version requirements
- Added matplotlib and numpy as core dependencies
- Enhanced build system integration for tools
- Improved development workflow automation
- Updated Rust dependencies to latest versions
- Added comprehensive documentation for dependency versions
- Enhanced build system documentation
- Updated usage examples and workflow guides
- Added new README.md in reporting/ directory

## v0.1.55 (2025-03-24)
- Enhanced charting module with automated Makefile integration
- Added new `make charts` target for easy chart generation
- Updated requirements.txt with explicit matplotlib and numpy dependencies
- Created comprehensive README.md in reporting/ directory
- Added automatic cleanup of generated charts in make clean target
- Improved documentation for charting tools and data formats
- Added support for word-to-token conversion factor (1.33) in visualizations

## v0.1.54 (2025-03-24)
- Added Python-based charting module in reporting/ directory for visualization of benchmarking results
- Enhanced performance metrics visualization with charts for requests/sec, token throughput, and TTFT
- Added comprehensive documentation for the charting tools
- Improved API documentation and usage examples
- Added additional clarity on tool interactions and workflow

## v0.1.53 (2025-03-22)
- First implementation of basic charting functionality
- Fixed documentation to use PATH consistently
- Updated examples across multiple readme files
- Improved announcement process for new releases
- Added missing documentation for library modules

## v0.1.52 (2025-03-21)
- Extended license validity date until April 30, 2025
- Enhanced scenario-combo.sh script with randomized testing scenarios
- Limited the maximum runtime for individual scenarios to 60 seconds
- Reduced the number of test combinations in scenario-combo.sh for more efficient testing
- Updated documentation to reflect license and feature changes

## v0.1.51 (2025-03-20)
- Added word count statistics for rustyphalanx tool
- Implemented word count metrics for both prompt and completion text
- Updated metrics output to include word-based throughput measurements
- Enhanced JSON log output with word count metrics and statistics
- Updated documentation to reflect word count functionality

## v0.1.50 (2025-03-19)
- Enhanced documentation for metrumbench-asr audio transcription benchmarking tool
- Added detailed usage examples and implementation notes
- Improved command-line argument documentation with required parameter indicators
- Added detailed explanation of metrics calculations (WER, CER, RTF)
- Documented file caching behavior and verbose_json format support
- Added comprehensive testing instructions for metrumbench-asr tool

## v0.1.49 (2025-03-18)
- Added metrumbench-asr tool for audio transcription API benchmarking
- Implemented support for multipart form uploads with audio files
- Added support for both URL-based and direct file path audio sources
- Integrated with OpenAI-compatible audio transcription APIs
- Comprehensive metrics for audio processing (RTF, WER, CER, throughput)
- Detailed documentation for audio transcription testing

## v0.1.48 (2025-03-18)
- Updated documentation for improved consistency and accuracy
- Added Release Plan document with upcoming features through May 2025
- Fixed parameter inconsistencies in tools documentation
- Improved build system with automatic version-based archive creation
- Added new license expiry note in announcements

## v0.1.47 (2025-03-11)
- Added test cases for new features
- Enhanced error handling for edge cases
- Fixed minor bugs in CSV parsing
- Updated dependencies

## v0.1.46 (2025-03-09)
- Added examples for API-based testing
- Created basic wrapper server for simplified API testing
- Improved documentation for server configuration

## v0.1.43 (2025-03-03)
- Enhanced metrumbench-vlm tool to support multiple images per prompt
- Added support for semicolon-delimited URLs in CSV input files
- Improved metrics tracking for multi-image requests
- Updated documentation to clarify multi-image capabilities

## v0.1.42 (2025-02-26)
- Updated license validity date
- Added new CLI option for restricting output length

## v0.1.41 (2025-02-18)
- Enhanced documentation
- Updated version requirements

## v0.1.40 (2025-02-10)
- Added sample file support
- Fixed timeout issues with large requests

## v0.1.39 (2025-02-05)
- Added support for Azure DevOps binary releases
- Removed S3 repository references

## v0.1.38 (2025-01-31)
- Added metrumbench-vlm tool for vision model benchmarking
- Implemented image processing and caching
- Added detailed image metrics collection

## v0.1.37 (2025-01-24)
- Enhanced ISO customizer with additional package options
- Added support for custom environment scripts

## v0.1.36 (2025-01-17)
- Added ISO customizer for creating custom Ubuntu images
- Implemented cloud-init configuration

## v0.1.35 (2025-01-10)
- Added container launch tool with YAML configuration
- Added subprocess launch tool with YAML configuration

## v0.1.34 (2025-01-03)
- Improved error handling and retry logic
- Added detailed logging capabilities

## v0.1.33 (2024-12-27)
- Added JSONL to CSV conversion utility
- Enhanced token counting logic

## v0.1.32 (2024-12-20)
- Added support for extracting prompts with specific criteria
- Implemented length-based filtering

## v0.1.31 (2024-12-13)
- Added support for waiting for vLLM service availability
- Implemented health check polling

## v0.1.30 (2024-12-06)
- Added support for streaming responses
- Improved connection pooling

## v0.1.29 (2024-11-29)
- Added support for multiple request modes
- Enhanced metrics collection

## v0.1.28 (2024-11-22)
- Initial public release
- Implemented basic load testing functionality
- Added support for OpenAI-compatible endpoints
