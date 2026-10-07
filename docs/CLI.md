<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# CLI reference

Generated from `metrum-ai-bench-cli*` `--help`. Re-run
`cargo xtask render-cli-help` after flag changes. Live `--help` is
authoritative if this file drifts.

## `metrum-ai-bench-cli`

```text
Usage: metrum-ai-bench-cli <COMMAND>

Commands:
  llm        Text chat/completions benchmark
  vlm        Vision-language chat benchmark
  asr        Audio transcription benchmark
  imagegen   Image generation benchmark
  prompts    Select an ISL/OSL mix from metrum-ai/prompt-library
  selftest   Print client environment JSON and `selftest: ok` (exit 0 on success)
  preflight  Probe an OpenAI-compatible serving endpoint before a long run
  sut        Write or probe a system-under-test declaration
  compare    Compare two or more strategic sweep summaries or request CSVs
  help       Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

### `metrum-ai-bench-cli preflight`

```text
Usage: metrum-ai-bench-cli preflight [OPTIONS] --url <URL> --api-key <API_KEY>

Options:
      --url <URL>
          Endpoint URL (base or full `/v1/chat/completions` path)
      --api-key <API_KEY>
          API key sent as a Bearer token (use `dummy` when the server ignores it)
      --model <MODEL>
          Model id for chat/streaming probes [default: dummy]
      --connect-timeout <CONNECT_TIMEOUT>
          Connect timeout in seconds [default: 10]
      --request-timeout <REQUEST_TIMEOUT>
          Request timeout in seconds [default: 60]
      --latency-samples <LATENCY_SAMPLES>
          Number of unary latency samples [default: 3]
      --extra-body-json <EXTRA_BODY_JSON>
          Extra JSON object merged into chat probe bodies (for example `{"chat_template_kwargs":{"enable_thinking":false}}` on thinking models)
      --json
          Also print the machine-readable JSON report after the table
  -h, --help
          Print help
```

## `metrum-ai-bench-cli-llm`

```text
Usage: metrum-ai-bench-cli-llm [OPTIONS] --scenario <SCENARIO> --num-requests <NUM_REQUESTS> --concurrency <CONCURRENCY> --prompts <PROMPTS> --mode <MODE> --model <MODEL> --data-log <DATA_LOG> --max-tokens <MAX_TOKENS>

Options:
      --version-only
          Print version information and exit

      --ntp-check
          Opt-in NTP clock check; records offset when available (does not hard-fail)

      --scenario <SCENARIO>
          Descriptor for the scenario being run

      --url <URL>
          URL of the AI model endpoint (use --endpoints-file for multiple). Required with --api-key.

      --endpoints-file <ENDPOINTS_FILE>
          Path to endpoints config file (YAML, curl-style). Mutually exclusive with --url/--api-key

      --num-requests <NUM_REQUESTS>
          Number of requests to send (must be >= 1)

      --concurrency <CONCURRENCY>
          Number of concurrent requests (must be >= 1)

      --prompts <PROMPTS>
          Path to a JSONL file or http(s) URL of JSONL prompts (one JSON object per line with "prompt" field)

      --mode <MODE>
          Mode of operation: 'chat' or 'completion'

          [possible values: chat, completion]

      --streaming
          Enable streaming mode

      --infer-ttft-from-first-byte
          When visible-token TTFT is missing, approximate it from HTTP time-to-first-byte and record provenance

      --log-level <LOG_LEVEL>
          Log level: error, warn, info, debug, trace

          [default: warn]

      --model <MODEL>
          Model identifier

      --data-log <DATA_LOG>
          Path to the data log file

      --seed <SEED>
          RNG seed for shuffle/arrival/unique prompts

          [default: 0]

      --warmup-requests <WARMUP_REQUESTS>
          Warmup requests excluded from measurement

          [default: 0]

      --request-rate <REQUEST_RATE>
          Open-loop request rate (req/s). Omit for closed-loop concurrency

      --arrival <ARRIVAL>
          Arrival process when --request-rate is set: constant|poisson

          [default: constant]

      --max-concurrency <MAX_CONCURRENCY>
          Hard cap for outstanding requests in open-loop mode

      --load-balancer <LOAD_BALANCER>
          [default: round-robin]
          [possible values: round-robin, least-inflight]

      --ignore-eos
          Send ignore_eos=true in the request body

      --min-tokens <MIN_TOKENS>
          min_tokens (vLLM / compatible servers)

      --extra-body-json <EXTRA_BODY_JSON>
          Extra JSON object merged into the request body, e.g. '{"reasoning_effort":"medium"}'. Recorded in the run manifest. See docs/REASONING_MODELS.md.

      --system-prompt <SYSTEM_PROMPT>
          Optional system prompt for chat (omit for none; empty string also disables)

      --unique-prompts
          Prefix each prompt with a unique nonce to avoid prefix-cache hits

      --tokenizer <TOKENIZER>
          Path to tokenizer.json (requires build feature `tokenizer`)

      --slo <METRIC=VALUE>
          Repeatable goodput threshold: ttft=, tpot=, e2e= (seconds); user_tps= (tok/s per in-flight user)

      --throughput-bin-seconds <THROUGHPUT_BIN_SECONDS>
          Throughput dispersion bin width in seconds

          [default: 10]

      --ca-cert <PATH>
          Additional PEM CA certificate for TLS (must be a CA with basic constraints, not a self-signed leaf; use --insecure for self-signed leaves)

      --insecure
          Disable TLS certificate verification (opt-in; stamped into config)

      --fail-on-error
          Exit non-zero if any measured request failed (default: exit 0 after writing results)

      --price-per-hour <USD_PER_HOUR>
          Declared platform cost ($/hour); overrides sut.cost.price_per_hour for cost_per_million_output_tokens

      --isl-target <TOKENS>
          Expected mean/median input tokens for runtime ISL validation (overrides mix-report)

      --osl-target <TOKENS>
          Expected mean/median output tokens for runtime OSL validation (overrides mix-report)

      --isl-tolerance <ISL_TOLERANCE>
          Allowed absolute deviation from --isl-target (tokens). Clap default 0.0. Without --prompt-mix-report, 0.0 means exact match. With --prompt-mix-report, 0.0 is a sentinel that uses the report's ISL tolerance. Pass an explicit value for publishable runs.

          [default: 0]

      --osl-tolerance <OSL_TOLERANCE>
          Allowed absolute deviation from --osl-target (tokens). Clap default 0.0. Without --prompt-mix-report, 0.0 means exact match. With --prompt-mix-report, 0.0 is a sentinel that uses the report's OSL tolerance. Required for a meaningful --fail-on-osl-mismatch gate.

          [default: 0]

      --prompt-mix-report <PATH>
          Prompt-library mix report JSON; fills ISL/OSL targets when CLI targets are unset

      --fail-on-osl-mismatch
          Exit non-zero when measured OSL mismatches exceed --osl-tolerance (publishable gate)

      --sut <PATH>
          Operator-declared SUT block (JSON/YAML) embedded in summary.v3 as sut

      --require-sut
          Refuse to run without a complete --sut block (gpu.model, gpu.count, driver_version, runtime.name/version/config, host_os); implies --redact-hostname

          [env: METRUM_AI_BENCH_REQUIRE_SUT=]

      --redact-hostname
          Write environment.hostname as null

          [env: METRUM_AI_BENCH_REDACT_HOSTNAME=]

      --quiet
          Suppress ASCII banner art (one-line identity still prints). Also set NO_BANNER=1.

      --ndjson <PATH>
          Tagged NDJSON run log (run/stage/request/telemetry/summary rows, telemetry.v1)

      --telemetry <PATH>
          Telemetry scrape YAML (Prometheus /metrics or /metric sources); requires --ndjson

      --require-telemetry
          Abort mid-run after N consecutive scrape failures on any source (default N=3); a failed startup probe fails the run with or without this flag

      --require-telemetry-failures <REQUIRE_TELEMETRY_FAILURES>
          Consecutive scrape failures before --require-telemetry aborts

          [default: 3]

      --max-tokens <MAX_TOKENS>
          Maximum number of tokens

      --temperature <TEMPERATURE>
          Temperature for sampling

          [default: 0.1]

      --debug-log <DEBUG_LOG>
          Path to the debug log file

          [default: debug.log]

      --error-log <ERROR_LOG>
          Path to the error log file

          [default: error.log]

      --request-timeout <REQUEST_TIMEOUT>
          Request timeout in seconds

          [default: 300]

      --connect-timeout <CONNECT_TIMEOUT>
          Connect timeout in seconds

          [default: 30]

      --pool-idle-timeout <POOL_IDLE_TIMEOUT>
          Pool idle timeout in seconds

          [default: 60]

      --tcp-keepalive <TCP_KEEPALIVE>
          TCP keepalive in seconds

          [default: 60]

      --api-key <API_KEY>
          API key sent as a Bearer token. Required with --url. Use any placeholder such as "dummy" for servers that do not check it. Use --endpoints-file for multiple endpoints.

      --stop-after-seconds <STOP_AFTER_SECONDS>
          Stop sending new requests after N seconds

      --ramp-up-seconds <RAMP_UP_SECONDS>
          Ramp up period in seconds to gradually increase concurrency

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

## `metrum-ai-bench-cli-vlm`

```text
Usage: metrum-ai-bench-cli-vlm [OPTIONS] --scenario <SCENARIO> --num-requests <NUM_REQUESTS> --concurrency <CONCURRENCY> --prompts <PROMPTS> --model <MODEL> --data-log <DATA_LOG> --max-tokens <MAX_TOKENS>

Options:
      --version-only
          Print version information and exit

      --ntp-check
          Opt-in NTP clock check; records offset when available (does not hard-fail)

      --scenario <SCENARIO>
          Descriptor for the scenario being run

      --url <URL>
          URL of the AI model endpoint (use --endpoints-file for multiple). Required with --api-key.

      --endpoints-file <ENDPOINTS_FILE>
          Path to endpoints config file (YAML). Mutually exclusive with --url/--api-key

      --num-requests <NUM_REQUESTS>
          Number of requests to send (must be >= 1)

      --concurrency <CONCURRENCY>
          Number of concurrent requests (must be >= 1)

      --prompts <PROMPTS>
          Path to the JSONL file containing prompts (one object per line with "prompt" and "image_urls" or "image_url"; each image is a local path, file:// URI, http(s) URL, or base64 data: URL)

      --log-level <LOG_LEVEL>
          Log level: error, warn, info, debug, trace

          [default: warn]

      --model <MODEL>
          Model identifier

      --data-log <DATA_LOG>
          Path to the data log file

      --seed <SEED>
          RNG seed for shuffle/arrival/unique prompts

          [default: 0]

      --warmup-requests <WARMUP_REQUESTS>
          Warmup requests excluded from measurement

          [default: 0]

      --request-rate <REQUEST_RATE>
          Open-loop request rate (req/s). Omit for closed-loop concurrency

      --arrival <ARRIVAL>
          Arrival process when --request-rate is set: constant|poisson

          [default: constant]

      --max-concurrency <MAX_CONCURRENCY>
          Hard cap for outstanding requests in open-loop mode

      --load-balancer <LOAD_BALANCER>
          [default: round-robin]
          [possible values: round-robin, least-inflight]

      --ignore-eos
          Send ignore_eos=true in the request body

      --min-tokens <MIN_TOKENS>
          min_tokens (vLLM / compatible servers)

      --extra-body-json <EXTRA_BODY_JSON>
          Extra JSON object merged into the request body, e.g. '{"reasoning_effort":"medium"}'. Recorded in the run manifest. See docs/REASONING_MODELS.md.

      --system-prompt <SYSTEM_PROMPT>
          Optional system prompt for chat (omit for none; empty string also disables)

      --unique-prompts
          Prefix each prompt with a unique nonce to avoid prefix-cache hits

      --tokenizer <TOKENIZER>
          Path to tokenizer.json (requires build feature `tokenizer`)

      --slo <METRIC=VALUE>
          Repeatable goodput threshold: ttft=, tpot=, e2e= (seconds); user_tps= (tok/s per in-flight user)

      --throughput-bin-seconds <THROUGHPUT_BIN_SECONDS>
          Throughput dispersion bin width in seconds

          [default: 10]

      --ca-cert <PATH>
          Additional PEM CA certificate for TLS (must be a CA with basic constraints, not a self-signed leaf; use --insecure for self-signed leaves)

      --insecure
          Disable TLS certificate verification (opt-in; stamped into config)

      --fail-on-error
          Exit non-zero if any measured request failed (default: exit 0 after writing results)

      --price-per-hour <USD_PER_HOUR>
          Declared platform cost ($/hour); overrides sut.cost.price_per_hour for cost_per_million_output_tokens

      --isl-target <TOKENS>
          Expected mean/median input tokens for runtime ISL validation (overrides mix-report)

      --osl-target <TOKENS>
          Expected mean/median output tokens for runtime OSL validation (overrides mix-report)

      --isl-tolerance <ISL_TOLERANCE>
          Allowed absolute deviation from --isl-target (tokens). Clap default 0.0. Without --prompt-mix-report, 0.0 means exact match. With --prompt-mix-report, 0.0 is a sentinel that uses the report's ISL tolerance. Pass an explicit value for publishable runs.

          [default: 0]

      --osl-tolerance <OSL_TOLERANCE>
          Allowed absolute deviation from --osl-target (tokens). Clap default 0.0. Without --prompt-mix-report, 0.0 means exact match. With --prompt-mix-report, 0.0 is a sentinel that uses the report's OSL tolerance. Required for a meaningful --fail-on-osl-mismatch gate.

          [default: 0]

      --prompt-mix-report <PATH>
          Prompt-library mix report JSON; fills ISL/OSL targets when CLI targets are unset

      --fail-on-osl-mismatch
          Exit non-zero when measured OSL mismatches exceed --osl-tolerance (publishable gate)

      --sut <PATH>
          Operator-declared SUT block (JSON/YAML) embedded in summary.v3 as sut

      --require-sut
          Refuse to run without a complete --sut block (gpu.model, gpu.count, driver_version, runtime.name/version/config, host_os); implies --redact-hostname

          [env: METRUM_AI_BENCH_REQUIRE_SUT=]

      --redact-hostname
          Write environment.hostname as null

          [env: METRUM_AI_BENCH_REDACT_HOSTNAME=]

      --quiet
          Suppress ASCII banner art (one-line identity still prints). Also set NO_BANNER=1.

      --ndjson <PATH>
          Tagged NDJSON run log (run/stage/request/telemetry/summary rows, telemetry.v1)

      --telemetry <PATH>
          Telemetry scrape YAML (Prometheus /metrics or /metric sources); requires --ndjson

      --require-telemetry
          Abort mid-run after N consecutive scrape failures on any source (default N=3); a failed startup probe fails the run with or without this flag

      --require-telemetry-failures <REQUIRE_TELEMETRY_FAILURES>
          Consecutive scrape failures before --require-telemetry aborts

          [default: 3]

      --streaming
          Enable streaming mode for measured TTFT/ITL

      --infer-ttft-from-first-byte
          When visible-token TTFT is missing, approximate it from HTTP time-to-first-byte and record provenance

      --max-tokens <MAX_TOKENS>
          Maximum number of tokens

      --temperature <TEMPERATURE>
          Temperature for sampling

          [default: 0.1]

      --debug-log <DEBUG_LOG>
          Path to the debug log file

          [default: debug.log]

      --error-log <ERROR_LOG>
          Path to the error log file

          [default: error.log]

      --request-timeout <REQUEST_TIMEOUT>
          Request timeout in seconds

          [default: 120]

      --connect-timeout <CONNECT_TIMEOUT>
          Connect timeout in seconds

          [default: 30]

      --pool-idle-timeout <POOL_IDLE_TIMEOUT>
          Pool idle timeout in seconds

          [default: 60]

      --tcp-keepalive <TCP_KEEPALIVE>
          TCP keepalive in seconds

          [default: 60]

      --api-key <API_KEY>
          API key sent as a Bearer token. Required with --url. Use any placeholder such as "dummy" for servers that do not check it. Use --endpoints-file for multiple endpoints.

      --stop-after-seconds <STOP_AFTER_SECONDS>
          Stop sending new requests after N seconds

      --ramp-up-seconds <RAMP_UP_SECONDS>
          Ramp up period in seconds to gradually increase concurrency

      --num-images-batch <NUM_IMAGES_BATCH>
          Number of images to include per request (OpenAI supports multiple images per prompt)

          [default: 1]

      --image-cache-size <IMAGE_CACHE_SIZE>
          Size of the image cache (must be >= 1)

          [default: 1000]

      --max-image-dimension <MAX_IMAGE_DIMENSION>
          Maximum image dimension (width/height) in pixels, must be >= 1 if set

      --reencode-jpeg
          Re-encode images as JPEG instead of sending the original bytes

      --image-detail <IMAGE_DETAIL>
          Image detail level: 'low' or 'high'

          [default: low]
          [possible values: low, high]

      --server-side-download
          Send http(s) image URLs for the server to download instead of base64 encoding them; local paths, file:// and data: URLs are rejected in this mode

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

## `metrum-ai-bench-cli-asr`

```text
Usage: metrum-ai-bench-cli-asr [OPTIONS]

Options:
      --version-only
          Print version information and exit

      --ntp-check
          Opt-in NTP clock check; records offset when available (does not hard-fail)

      --scenario <SCENARIO>
          Descriptor for the scenario being run

      --url <URL>
          URL of the audio transcription API endpoint. Required with --api-key.

      --num-requests <NUM_REQUESTS>
          Number of requests to send (must be >= 1 when set)

      --concurrency <CONCURRENCY>
          Number of concurrent requests (must be >= 1)

          [default: 10]

      --input <INPUT>
          Path or http(s) URL to a JSONL file listing audio samples (id, path or url, optional format/duration)

      --log-level <LOG_LEVEL>
          Log level: error, warn, info, debug, trace

          [default: info]

      --model <MODEL>
          Model identifier (e.g., 'whisper-1')

      --data-log <DATA_LOG>
          Path to the data log file

          [default: results.jsonl]

      --seed <SEED>
          RNG seed for shuffle/arrival/unique prompts

          [default: 0]

      --warmup-requests <WARMUP_REQUESTS>
          Warmup requests excluded from measurement

          [default: 0]

      --request-rate <REQUEST_RATE>
          Open-loop request rate (req/s). Omit for closed-loop concurrency

      --arrival <ARRIVAL>
          Arrival process when --request-rate is set: constant|poisson

          [default: constant]

      --max-concurrency <MAX_CONCURRENCY>
          Hard cap for outstanding requests in open-loop mode

      --load-balancer <LOAD_BALANCER>
          [default: round-robin]
          [possible values: round-robin, least-inflight]

      --ignore-eos
          Send ignore_eos=true in the request body

      --min-tokens <MIN_TOKENS>
          min_tokens (vLLM / compatible servers)

      --extra-body-json <EXTRA_BODY_JSON>
          Extra JSON object merged into the request body, e.g. '{"reasoning_effort":"medium"}'. Recorded in the run manifest. See docs/REASONING_MODELS.md.

      --system-prompt <SYSTEM_PROMPT>
          Optional system prompt for chat (omit for none; empty string also disables)

      --unique-prompts
          Prefix each prompt with a unique nonce to avoid prefix-cache hits

      --tokenizer <TOKENIZER>
          Path to tokenizer.json (requires build feature `tokenizer`)

      --slo <METRIC=VALUE>
          Repeatable goodput threshold: ttft=, tpot=, e2e= (seconds); user_tps= (tok/s per in-flight user)

      --throughput-bin-seconds <THROUGHPUT_BIN_SECONDS>
          Throughput dispersion bin width in seconds

          [default: 10]

      --ca-cert <PATH>
          Additional PEM CA certificate for TLS (must be a CA with basic constraints, not a self-signed leaf; use --insecure for self-signed leaves)

      --insecure
          Disable TLS certificate verification (opt-in; stamped into config)

      --fail-on-error
          Exit non-zero if any measured request failed (default: exit 0 after writing results)

      --price-per-hour <USD_PER_HOUR>
          Declared platform cost ($/hour); overrides sut.cost.price_per_hour for cost_per_million_output_tokens

      --isl-target <TOKENS>
          Expected mean/median input tokens for runtime ISL validation (overrides mix-report)

      --osl-target <TOKENS>
          Expected mean/median output tokens for runtime OSL validation (overrides mix-report)

      --isl-tolerance <ISL_TOLERANCE>
          Allowed absolute deviation from --isl-target (tokens). Clap default 0.0. Without --prompt-mix-report, 0.0 means exact match. With --prompt-mix-report, 0.0 is a sentinel that uses the report's ISL tolerance. Pass an explicit value for publishable runs.

          [default: 0]

      --osl-tolerance <OSL_TOLERANCE>
          Allowed absolute deviation from --osl-target (tokens). Clap default 0.0. Without --prompt-mix-report, 0.0 means exact match. With --prompt-mix-report, 0.0 is a sentinel that uses the report's OSL tolerance. Required for a meaningful --fail-on-osl-mismatch gate.

          [default: 0]

      --prompt-mix-report <PATH>
          Prompt-library mix report JSON; fills ISL/OSL targets when CLI targets are unset

      --fail-on-osl-mismatch
          Exit non-zero when measured OSL mismatches exceed --osl-tolerance (publishable gate)

      --sut <PATH>
          Operator-declared SUT block (JSON/YAML) embedded in summary.v3 as sut

      --require-sut
          Refuse to run without a complete --sut block (gpu.model, gpu.count, driver_version, runtime.name/version/config, host_os); implies --redact-hostname

          [env: METRUM_AI_BENCH_REQUIRE_SUT=]

      --redact-hostname
          Write environment.hostname as null

          [env: METRUM_AI_BENCH_REDACT_HOSTNAME=]

      --quiet
          Suppress ASCII banner art (one-line identity still prints). Also set NO_BANNER=1.

      --ndjson <PATH>
          Tagged NDJSON run log (run/stage/request/telemetry/summary rows, telemetry.v1)

      --telemetry <PATH>
          Telemetry scrape YAML (Prometheus /metrics or /metric sources); requires --ndjson

      --require-telemetry
          Abort mid-run after N consecutive scrape failures on any source (default N=3); a failed startup probe fails the run with or without this flag

      --require-telemetry-failures <REQUIRE_TELEMETRY_FAILURES>
          Consecutive scrape failures before --require-telemetry aborts

          [default: 3]

      --debug-log <DEBUG_LOG>
          Path to the debug log file

          [default: debug.log]

      --error-log <ERROR_LOG>
          Path to the error log file

          [default: error.log]

      --request-timeout <REQUEST_TIMEOUT>
          Request timeout in seconds

          [default: 120]

      --connect-timeout <CONNECT_TIMEOUT>
          Connect timeout in seconds

          [default: 30]

      --pool-idle-timeout <POOL_IDLE_TIMEOUT>
          Pool idle timeout in seconds

          [default: 60]

      --tcp-keepalive <TCP_KEEPALIVE>
          TCP keepalive in seconds

          [default: 60]

      --api-key <API_KEY>
          API key sent as a Bearer token. Required with --url. Use any placeholder such as "dummy" for servers that do not check it. Use --endpoints-file for multiple endpoints.

      --endpoints-file <ENDPOINTS_FILE>
          Path to YAML file with endpoints (url, api_key, name?, weight?); mutually exclusive with --url/--api-key

      --stop-after-seconds <STOP_AFTER_SECONDS>
          Stop sending new requests after N seconds

      --ground-truth <GROUND_TRUTH>
          Path or http(s) URL to a JSONL file with ground-truth transcripts (id, transcript per line)

      --response-format <RESPONSE_FORMAT>
          Response format: verbose_json, json, text, srt, vtt

          [default: verbose-json]
          [possible values: verbose-json, json, text, srt, vtt]

      --language <LANGUAGE>
          Language code for transcription (e.g. en, es, fr). Sent in the multipart request to avoid vLLM returning null language in verbose_json responses.

          [default: en]

      --normalizer <NORMALIZER>
          Text normalization applied to both sides of WER/CER

          Possible values:
          - whisper-english: Whisper basic normalization plus English contraction and numeral folding
          - whisper-basic:   Case, punctuation and bracketed-filler folding only
          - none:            Compare raw strings

          [default: whisper-english]

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

## `metrum-ai-bench-cli-imagegen`

```text
Usage: metrum-ai-bench-cli-imagegen [OPTIONS] --scenario <SCENARIO> --model <MODEL> --num-requests <NUM_REQUESTS> --concurrency <CONCURRENCY> --data-log <DATA_LOG>

Options:
      --version-only
          Print version information and exit

      --quiet
          Suppress ASCII banner art if printed (one-line identity). Also set NO_BANNER=1.

      --ntp-check
          Opt-in NTP clock check; records offset when available (does not hard-fail)

      --scenario <SCENARIO>


      --url <URL>
          OpenAI-compatible base URL, usually ending in /v1. Required with --api-key.

      --api-key <API_KEY>
          API key sent as a Bearer token. Required with --url. Use any placeholder such as "dummy" for servers that do not check it. Use --endpoints-file for multiple endpoints.

      --endpoint <ENDPOINT>
          Repeatable endpoint URL for multi-endpoint mode

      --endpoints-file <ENDPOINTS_FILE>
          JSON or JSONL endpoint file

      --load-balancer <LOAD_BALANCER>
          [default: round-robin]
          [possible values: round-robin, least-inflight, random, weighted-round-robin]

      --endpoint-health-check


      --health-path <HEALTH_PATH>
          [default: /models]

      --max-endpoint-failures <MAX_ENDPOINT_FAILURES>
          [default: 2]

      --endpoint-retry-attempts <ENDPOINT_RETRY_ATTEMPTS>
          [default: 0]

      --endpoint-retry-backoff-ms <ENDPOINT_RETRY_BACKOFF_MS>
          [default: 100]

      --model <MODEL>


      --num-requests <NUM_REQUESTS>


      --concurrency <CONCURRENCY>


      --request-rate <REQUEST_RATE>
          Open-loop request rate (requests/second)

      --arrival <ARRIVAL>
          [default: constant]
          [possible values: constant, poisson]

      --max-concurrency <MAX_CONCURRENCY>


      --prompt <PROMPT>


      --prompts <PROMPTS>


      --prompt-field <PROMPT_FIELD>
          [default: prompt]

      --id-field <ID_FIELD>
          [default: id]

      --shuffle-prompts


      --warmup-requests <WARMUP_REQUESTS>
          Warmup requests excluded from summary stats

          [default: 0]

      --seed <SEED>
          Base RNG seed. With the default --seed-mode increment, request N uses --seed + N. Set --seed once. Do not also set a per-row seed in --prompts or a seed field in --extra-body-json/--extra-body-file: the row seed pins that request and the extra body overrides the computed seed.

      --seed-mode <SEED_MODE>
          Seed policy. increment (default): request N uses --seed + N unless the prompt row sets seed. fixed: every request uses --seed. prompt: use the row seed, falling back to --seed. A duplicate --seed on the command line does not error; the last value wins.

          [default: increment]
          [possible values: fixed, increment, prompt]

      --n <N>
          [default: 1]

      --size <SIZE>
          [default: 1024x1024]

      --response-format <RESPONSE_FORMAT>
          [default: b64_json]
          [possible values: b64_json, url]

      --negative-prompt <NEGATIVE_PROMPT>


      --num-inference-steps <NUM_INFERENCE_STEPS>


      --guidance-scale <GUIDANCE_SCALE>


      --true-cfg-scale <TRUE_CFG_SCALE>


      --extra-body-json <EXTRA_BODY_JSON>
          Extra JSON object merged into the request body, e.g. '{"reasoning_effort":"medium"}'. Recorded in the run manifest. See docs/REASONING_MODELS.md.

      --extra-body-file <EXTRA_BODY_FILE>
          Path to a JSON object file merged into the request body (alternative to --extra-body-json). Recorded via the merged body template.

      --request-timeout <REQUEST_TIMEOUT>
          [default: 300]

      --connect-timeout <CONNECT_TIMEOUT>
          [default: 30]

      --pool-idle-timeout <POOL_IDLE_TIMEOUT>
          [default: 60]

      --tcp-keepalive <TCP_KEEPALIVE>
          [default: 60]

      --ca-cert <PATH>
          Additional PEM CA certificate for TLS (private gateways)

      --insecure
          Disable TLS certificate verification (opt-in; stamped into config)

      --artifact-dir <ARTIFACT_DIR>
          [default: metrum-ai-bench-cli-imagegen-artifacts]

      --data-log <DATA_LOG>


      --ndjson <PATH>
          Tagged NDJSON run log (run/stage/request/telemetry/summary rows, telemetry.v1)

      --telemetry <PATH>
          Telemetry scrape YAML (Prometheus /metrics or /metric sources); requires --ndjson

      --require-telemetry
          Abort mid-run after N consecutive scrape failures on any source (default N=3); a failed startup probe fails the run with or without this flag

      --require-telemetry-failures <REQUIRE_TELEMETRY_FAILURES>
          Consecutive scrape failures before --require-telemetry aborts

          [default: 3]

      --summary-json <SUMMARY_JSON>
          Optional path to write summary.v3 JSON (same schema as the data-log summary line)

      --debug-log <DEBUG_LOG>
          [default: debug.log]

      --error-log <ERROR_LOG>
          [default: error.log]

      --save-response-json


      --no-save-images


      --overwrite-artifacts


      --fail-on-error
          Exit non-zero if any measured request failed (default: exit 0 after writing results)

      --price-per-hour <USD_PER_HOUR>
          Declared platform cost ($/hour); overrides sut.cost.price_per_hour for cost_per_million_output_tokens

      --sut <PATH>
          Operator-declared SUT block (JSON/YAML) embedded in summary.v3 as sut

      --require-sut
          Refuse to run without a complete --sut block (gpu.model, gpu.count, driver_version, runtime.name/version/config, host_os); implies --redact-hostname

          [env: METRUM_AI_BENCH_REQUIRE_SUT=]

      --redact-hostname
          Write environment.hostname as null

          [env: METRUM_AI_BENCH_REDACT_HOSTNAME=]

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

## `metrum-ai-bench-cli-prompts`

```text
Usage: metrum-ai-bench-cli-prompts --count <COUNT> (--profile <PROFILE> | --isl-target <TOKENS> --osl-target <TOKENS>) --output <PATH> --report <PATH> [OPTIONS]

Options:
      --version-only
          Print version information and exit

      --quiet
          Suppress ASCII banner art (one-line identity still prints). Also set NO_BANNER=1.

      --dataset <DATASET>
          [default: metrum-ai/prompt-library]

      --revision <REVISION>
          Dataset revision (default: main = latest). Pass a 40-char commit SHA to pin. Branch/tag names resolve to the current commit.

          [default: main]

      --config <CONFIG>
          Dataset config: sample|full

          [default: sample]

      --split <SPLIT>
          [default: train]

      --allow-moving-revision
          Deprecated no-op: floating refs (including default main) always resolve. Kept for CLI compatibility.

      --require-pinned-revision
          Fail unless --revision is a 40-character commit SHA (publication pin)

      --cache-dir <CACHE_DIR>
          Cache directory for Hub downloads

      --offline
          Do not download; use files already in the cache

      --local-parquet <LOCAL_PARQUET>
          Load rows from local parquet shards (repeatable); skips Hub

      --local-jsonl <LOCAL_JSONL>
          Load rows from a local JSONL file with full metadata; skips Hub

      --count <COUNT>
          Preferred mix size (soft target; actual size may differ within --count-slack)

      --count-slack <COUNT_SLACK>
          Max absolute deviation from --count (default: max(count, 32))

      --seed <SEED>
          RNG seed for selection

          [default: 0]

      --profile <PROFILE>
          Named versioned ISL/OSL profile (chat-short, chat-medium, rag-medium, summarize-long, code-medium); conflicts with --isl-target/--osl-target

          Possible values:
          - chat-short:     Short interactive chat smoke (256 / 64 tokens)
          - chat-medium:    Default publishable chat (512 / 128 tokens)
          - rag-medium:     Retrieval-augmented generation (2048 / 256 tokens)
          - summarize-long: Long-context summarization (4096 / 512 tokens)
          - code-medium:    Coding-assistant turns (1024 / 512 tokens)

      --isl-target <ISL_TARGET>
          ISL target (same units as --isl-unit); omitted when --profile is set

      --isl-unit <ISL_UNIT>
          [default: tokens]
          [possible values: words, tokens]

      --isl-stat <ISL_STAT>
          [default: median]
          [possible values: mean, median]

      --isl-tolerance <ISL_TOLERANCE>
          Absolute ISL tolerance

          [default: 0]

      --osl-target <OSL_TARGET>
          OSL target (same units as --osl-unit); omitted when --profile is set

      --osl-unit <OSL_UNIT>
          [default: tokens]
          [possible values: words, tokens]

      --osl-stat <OSL_STAT>
          [default: median]
          [possible values: mean, median]

      --osl-tolerance <OSL_TOLERANCE>
          Absolute OSL tolerance

          [default: 0]

      --isl-token-basis <ISL_TOKEN_BASIS>
          [default: supplied-target]
          [possible values: supplied-target]

      --reasoning <REASONING>
          [default: any]
          [possible values: any, true, false]

      --max-repeats <MAX_REPEATS>
          Max copies of one source row

          [default: 8]

      --no-repeats
          Disable repeats (equivalent to --max-repeats 1)

      --osl-tokens-per-word <OSL_TOKENS_PER_WORD>
          Tokens-per-word factor for recommending --max-tokens when --osl-unit words

      --select-work-limit <SELECT_WORK_LIMIT>
          Selector work / iteration budget

          [default: 50000]

      --output <OUTPUT>
          Write selected prompts as JSONL for metrum-ai-bench-cli-llm

      --report <REPORT>
          Write selection report JSON

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

## `metrum-ai-bench-cli-strategic`

```text
Usage: metrum-ai-bench-cli-strategic [OPTIONS]

Options:
      --version-only
          Print version information and exit

      --url <URL>


      --api-key <API_KEY>
          [env: OPENAI_API_KEY]
          [default: ""]

      --model <MODEL>


      --kind <KIND>
          Possible values:
          - chat
          - embeddings
          - rerank
          - vlm:        Chat completions with `image_url` parts (metrum-ai-bench-cli-vlm bodies)
          - asr:        `/v1/audio/transcriptions` multipart uploads (metrum-ai-bench-cli-asr forms)
          - imagegen:   `/v1/images/generations` (metrum-ai-bench-cli-imagegen bodies)

          [default: chat]

      --streaming
          Stream chat and vlm responses to measure TTFT; embeddings, rerank, asr and imagegen remain unary

      --infer-ttft-from-first-byte
          When visible-token TTFT is missing, approximate it from HTTP time-to-first-byte and record provenance

      --requests-per-stage <REQUESTS_PER_STAGE>
          [default: 100]

      --sweep <SWEEP>
          [default: 1,2,4,8]

      --sweep-by <SWEEP_BY>
          [default: concurrency]
          [possible values: concurrency, rate]

      --max-in-flight <MAX_IN_FLIGHT>
          Maximum outstanding requests during a rate sweep

          [default: 256]

      --prompt <PROMPT>
          Single prompt string (ignored when --prompts or --sessions is set)

          [default: Hello]

      --prompts <PROMPTS>
          JSONL prompt file (objects with "prompt"; vlm rows also carry images, as metrum-ai-bench-cli-vlm --prompts); chat and imagegen also accept an http(s) URL; cycles across requests

      --max-tokens <MAX_TOKENS>
          Max completion tokens for chat and vlm bodies; required for vlm and when chat uses --prompts, recommended for all chat sweeps

      --ignore-eos
          Send ignore_eos=true in chat request bodies (engine extension; for fixed-length throughput studies)

      --min-tokens <N>
          Send min_tokens=N in chat request bodies (engine extension; must be <= --max-tokens)

      --extra-body-json <JSON>
          Merge extra JSON object fields into chat, vlm or imagegen request bodies

      --temperature <TEMPERATURE>
          Sampling temperature for chat and vlm bodies; omitted from chat bodies when unset, vlm defaults to 0.1 as metrum-ai-bench-cli-vlm

      --image <PATH_OR_URL>
          --kind vlm: image attached to --prompt (repeatable; local path, http(s) or data: URL); ignored with --prompts

      --image-detail <IMAGE_DETAIL>
          --kind vlm: image_url detail

          [default: low]
          [possible values: low, high]

      --max-image-dimension <PIXELS>
          --kind vlm: downscale images whose longer side exceeds PIXELS (re-encoded as PNG)

      --audio-samples <PATH>
          --kind asr: audio samples JSONL (id, path or url, format, optional duration), as metrum-ai-bench-cli-asr --input

      --ground-truth <PATH>
          --kind asr: reference transcripts JSONL (id, transcript) for stage WER/CER

      --asr-response-format <ASR_RESPONSE_FORMAT>
          --kind asr: transcription response_format

          [default: verbose_json]
          [possible values: verbose_json, json, text, srt, vtt]

      --language <LANGUAGE>
          --kind asr: language form field (empty to omit)

          [default: en]

      --normalizer <NORMALIZER>
          --kind asr: text normalization applied to both sides of WER/CER

          Possible values:
          - whisper-english: Whisper basic normalization plus English contraction and numeral folding
          - whisper-basic:   Case, punctuation and bracketed-filler folding only
          - none:            Compare raw strings

          [default: whisper-english]

      --image-size <IMAGE_SIZE>
          --kind imagegen: image size

          [default: 1024x1024]

      --images-per-request <IMAGES_PER_REQUEST>
          --kind imagegen: images per request (n)

          [default: 1]

      --image-response-format <IMAGE_RESPONSE_FORMAT>
          --kind imagegen: response_format; b64_json images are decoded and digested

          [default: b64_json]
          [possible values: b64_json, url]

      --warmup-requests <WARMUP_REQUESTS>
          Per-stage warmup requests excluded from measured aggregates (cold-start control)

          [default: 0]

      --seed <SEED>
          RNG seed used when --shuffle-prompts is set

          [default: 0]

      --shuffle-prompts
          Shuffle --prompts with --seed before cycling

      --sessions <SESSIONS>


      --prefix-control <PREFIX_CONTROL>
          [default: shared]
          [possible values: shared, unique, none]

      --shared-prefix <SHARED_PREFIX>


      --json-schema <JSON_SCHEMA>


      --tools <TOOLS>


      --metrics-url <METRICS_URL>


      --metrics-interval-ms <METRICS_INTERVAL_MS>
          [default: 250]

      --ndjson <PATH>
          Tagged NDJSON run log (run/stage/request/telemetry/summary rows)

      --telemetry <PATH>
          Telemetry scrape YAML (Prometheus /metrics or /metric sources)

      --require-telemetry
          Abort mid-run after N consecutive scrape failures on any source (default N=3); a failed startup probe fails the run with or without this flag

      --require-telemetry-failures <REQUIRE_TELEMETRY_FAILURES>
          Consecutive scrape failures before --require-telemetry aborts

          [default: 3]

      --html <HTML>
          [default: metrum-ai-bench-cli-report.html]

      --csv <CSV>
          [default: metrum-ai-bench-cli-requests.csv]

      --mlperf-dir <MLPERF_DIR>


      --mlperf-scenario <MLPERF_SCENARIO>
          [default: server]
          [possible values: server, offline]

      --otlp-endpoint <OTLP_ENDPOINT>


      --otlp-service-name <OTLP_SERVICE_NAME>
          [default: metrum-ai-bench-cli]

      --timeout-seconds <TIMEOUT_SECONDS>
          [default: 300]

      --slo <METRIC=VALUE>
          Repeatable goodput threshold: e2e=, ttft=, tpot= (streaming, seconds); user_tps= (tok/s per in-flight user)

      --price-per-hour <USD_PER_HOUR>
          Declared platform cost ($/hour); overrides sut.cost.price_per_hour for stage cost_per_million_output_tokens

      --isl-target <TOKENS>
          Expected input tokens for runtime ISL validation (overrides mix-report)

      --osl-target <TOKENS>
          Expected output tokens for runtime OSL validation (overrides mix-report)

      --isl-tolerance <ISL_TOLERANCE>
          Allowed absolute deviation from --isl-target (tokens). Clap default 0.0. Without --prompt-mix-report, 0.0 means exact match. With --prompt-mix-report, 0.0 is a sentinel that uses the report's ISL tolerance. Pass an explicit value for publishable runs.

          [default: 0]

      --osl-tolerance <OSL_TOLERANCE>
          Allowed absolute deviation from --osl-target (tokens). Clap default 0.0. Without --prompt-mix-report, 0.0 means exact match. With --prompt-mix-report, 0.0 is a sentinel that uses the report's OSL tolerance. Required for a meaningful --fail-on-osl-mismatch gate.

          [default: 0]

      --prompt-mix-report <PATH>
          Prompt-library mix report JSON; fills ISL/OSL targets when CLI targets are unset

      --fail-on-osl-mismatch
          Exit non-zero when measured OSL mismatches exceed --osl-tolerance

      --sut <PATH>
          Operator-declared SUT block (JSON/YAML) embedded in sweep summary and HTML

      --require-sut
          Refuse to run without a complete --sut block (gpu.model, gpu.count, driver_version, runtime.name/version/config, host_os); implies --redact-hostname

          [env: METRUM_AI_BENCH_REQUIRE_SUT=]

      --redact-hostname
          Reserved for parity with modality binaries (strategic stamps SUT only)

          [env: METRUM_AI_BENCH_REDACT_HOSTNAME=]

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

## `metrum-ai-bench-cli-mock-server`

```text
Usage: metrum-ai-bench-cli-mock-server [OPTIONS]

Options:
      --listen <LISTEN>          Address to bind; port 0 picks a free port, reported on the startup line [default: 127.0.0.1:8080]
      --latency-ms <LATENCY_MS>  [default: 0]
      --fail-every <FAIL_EVERY>  [default: 0]
      --telemetry-fixture        Serve canned DCGM and all-smi Prometheus fixtures on /metrics and /metric
  -h, --help                     Print help
  -V, --version                  Print version
```

