# Changelog

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
