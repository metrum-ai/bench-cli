<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# CLI reference

Generated from `metrum-ai-bench-cli*` `--help`. Re-run
`scripts/render_cli_help.sh` after flag changes. Live `--help` is
authoritative if this file drifts.

## `metrum-ai-bench-cli`

```text
Usage: metrum-ai-bench-cli <COMMAND>

Commands:
  llm       Text chat/completions benchmark
  vlm       Vision-language chat benchmark
  asr       Audio transcription benchmark
  imagegen  Image generation benchmark
  prompts   Select an ISL/OSL mix from metrum-ai/prompt-library
  selftest  Print client environment JSON and `selftest: ok` (exit 0 on success)
  help      Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
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
          Mode of operation: 'chat' or 'completion' [possible values: chat, completion]
      --streaming
          Enable streaming mode
      --log-level <LOG_LEVEL>
          Log level: error, warn, info, debug, trace [default: warn]
      --model <MODEL>
          Model identifier
      --data-log <DATA_LOG>
          Path to the data log file
      --seed <SEED>
          RNG seed for shuffle/arrival/unique prompts [default: 0]
      --warmup-requests <WARMUP_REQUESTS>
          Warmup requests excluded from measurement [default: 0]
      --request-rate <REQUEST_RATE>
          Open-loop request rate (req/s). Omit for closed-loop concurrency
      --arrival <ARRIVAL>
          Arrival process when --request-rate is set: constant|poisson [default: constant]
      --max-concurrency <MAX_CONCURRENCY>
          Hard cap for outstanding requests in open-loop mode
      --load-balancer <LOAD_BALANCER>
          [default: round-robin] [possible values: round-robin, least-inflight]
      --ignore-eos
          Send ignore_eos=true in the request body
      --min-tokens <MIN_TOKENS>
          min_tokens (vLLM / compatible servers)
      --extra-body-json <EXTRA_BODY_JSON>
          Extra JSON object merged into the request body, e.g. '{"reasoning_effort":"medium"}'. Recorded in the run manifest. See docs/REASONING_MODELS.md.
      --system-prompt <SYSTEM_PROMPT>
          Override the default system prompt (empty string disables it)
      --unique-prompts
          Prefix each prompt with a unique nonce to avoid prefix-cache hits
      --tokenizer <TOKENIZER>
          Path to tokenizer.json (requires build feature `tokenizer`)
      --slo <METRIC=SECONDS>
          Repeatable goodput threshold: ttft=, tpot=, e2e=
      --throughput-bin-seconds <THROUGHPUT_BIN_SECONDS>
          Throughput dispersion bin width in seconds [default: 10]
      --ca-cert <PATH>
          Additional PEM CA certificate for TLS (must be a CA with basic constraints, not a self-signed leaf; use --insecure for self-signed leaves)
      --insecure
          Disable TLS certificate verification (opt-in; stamped into config)
      --fail-on-error
          Exit non-zero if any measured request failed (default: exit 0 after writing results)
      --sut <PATH>
          Operator-declared SUT block (JSON/YAML) embedded in summary.v3 as sut
      --require-sut
          Refuse to run without a valid --sut block; implies --redact-hostname [env: METRUM_AI_BENCH_REQUIRE_SUT=]
      --redact-hostname
          Write environment.hostname as null [env: METRUM_AI_BENCH_REDACT_HOSTNAME=]
      --quiet
          Suppress ASCII banner art (one-line identity still prints). Also set NO_BANNER=1.
      --max-tokens <MAX_TOKENS>
          Maximum number of tokens
      --temperature <TEMPERATURE>
          Temperature for sampling [default: 0.1]
      --debug-log <DEBUG_LOG>
          Path to the debug log file [default: debug.log]
      --error-log <ERROR_LOG>
          Path to the error log file [default: error.log]
      --request-timeout <REQUEST_TIMEOUT>
          Request timeout in seconds [default: 300]
      --connect-timeout <CONNECT_TIMEOUT>
          Connect timeout in seconds [default: 30]
      --pool-idle-timeout <POOL_IDLE_TIMEOUT>
          Pool idle timeout in seconds [default: 60]
      --tcp-keepalive <TCP_KEEPALIVE>
          TCP keepalive in seconds [default: 60]
      --api-key <API_KEY>
          API key sent as a Bearer token. Required with --url. Use any placeholder such as "dummy" for servers that do not check it. Use --endpoints-file for multiple endpoints.
      --stop-after-seconds <STOP_AFTER_SECONDS>
          Stop sending new requests after N seconds
      --ramp-up-seconds <RAMP_UP_SECONDS>
          Ramp up period in seconds to gradually increase concurrency
  -h, --help
          Print help
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
          Path to the JSONL file containing prompts (one object per line with "prompt" and "image_urls" or "image_url")
      --log-level <LOG_LEVEL>
          Log level: error, warn, info, debug, trace [default: warn]
      --model <MODEL>
          Model identifier
      --data-log <DATA_LOG>
          Path to the data log file
      --seed <SEED>
          RNG seed for shuffle/arrival/unique prompts [default: 0]
      --warmup-requests <WARMUP_REQUESTS>
          Warmup requests excluded from measurement [default: 0]
      --request-rate <REQUEST_RATE>
          Open-loop request rate (req/s). Omit for closed-loop concurrency
      --arrival <ARRIVAL>
          Arrival process when --request-rate is set: constant|poisson [default: constant]
      --max-concurrency <MAX_CONCURRENCY>
          Hard cap for outstanding requests in open-loop mode
      --load-balancer <LOAD_BALANCER>
          [default: round-robin] [possible values: round-robin, least-inflight]
      --ignore-eos
          Send ignore_eos=true in the request body
      --min-tokens <MIN_TOKENS>
          min_tokens (vLLM / compatible servers)
      --extra-body-json <EXTRA_BODY_JSON>
          Extra JSON object merged into the request body, e.g. '{"reasoning_effort":"medium"}'. Recorded in the run manifest. See docs/REASONING_MODELS.md.
      --system-prompt <SYSTEM_PROMPT>
          Override the default system prompt (empty string disables it)
      --unique-prompts
          Prefix each prompt with a unique nonce to avoid prefix-cache hits
      --tokenizer <TOKENIZER>
          Path to tokenizer.json (requires build feature `tokenizer`)
      --slo <METRIC=SECONDS>
          Repeatable goodput threshold: ttft=, tpot=, e2e=
      --throughput-bin-seconds <THROUGHPUT_BIN_SECONDS>
          Throughput dispersion bin width in seconds [default: 10]
      --ca-cert <PATH>
          Additional PEM CA certificate for TLS (must be a CA with basic constraints, not a self-signed leaf; use --insecure for self-signed leaves)
      --insecure
          Disable TLS certificate verification (opt-in; stamped into config)
      --fail-on-error
          Exit non-zero if any measured request failed (default: exit 0 after writing results)
      --sut <PATH>
          Operator-declared SUT block (JSON/YAML) embedded in summary.v3 as sut
      --require-sut
          Refuse to run without a valid --sut block; implies --redact-hostname [env: METRUM_AI_BENCH_REQUIRE_SUT=]
      --redact-hostname
          Write environment.hostname as null [env: METRUM_AI_BENCH_REDACT_HOSTNAME=]
      --quiet
          Suppress ASCII banner art (one-line identity still prints). Also set NO_BANNER=1.
      --streaming
          Enable streaming mode for measured TTFT/ITL
      --max-tokens <MAX_TOKENS>
          Maximum number of tokens
      --temperature <TEMPERATURE>
          Temperature for sampling [default: 0.1]
      --debug-log <DEBUG_LOG>
          Path to the debug log file [default: debug.log]
      --error-log <ERROR_LOG>
          Path to the error log file [default: error.log]
      --request-timeout <REQUEST_TIMEOUT>
          Request timeout in seconds [default: 120]
      --connect-timeout <CONNECT_TIMEOUT>
          Connect timeout in seconds [default: 30]
      --pool-idle-timeout <POOL_IDLE_TIMEOUT>
          Pool idle timeout in seconds [default: 60]
      --tcp-keepalive <TCP_KEEPALIVE>
          TCP keepalive in seconds [default: 60]
      --api-key <API_KEY>
          API key sent as a Bearer token. Required with --url. Use any placeholder such as "dummy" for servers that do not check it. Use --endpoints-file for multiple endpoints.
      --stop-after-seconds <STOP_AFTER_SECONDS>
          Stop sending new requests after N seconds
      --ramp-up-seconds <RAMP_UP_SECONDS>
          Ramp up period in seconds to gradually increase concurrency
      --num-images-batch <NUM_IMAGES_BATCH>
          Number of images to include per request (OpenAI supports multiple images per prompt) [default: 1]
      --image-cache-size <IMAGE_CACHE_SIZE>
          Size of the image cache (must be >= 1) [default: 1000]
      --max-image-dimension <MAX_IMAGE_DIMENSION>
          Maximum image dimension (width/height) in pixels, must be >= 1 if set
      --reencode-jpeg
          Re-encode images as JPEG instead of sending the original bytes
      --image-detail <IMAGE_DETAIL>
          Image detail level: 'low' or 'high' [default: low] [possible values: low, high]
      --server-side-download
          Whether to let the server download images instead of base64 encoding them
  -h, --help
          Print help
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
          Override the default system prompt (empty string disables it)

      --unique-prompts
          Prefix each prompt with a unique nonce to avoid prefix-cache hits

      --tokenizer <TOKENIZER>
          Path to tokenizer.json (requires build feature `tokenizer`)

      --slo <METRIC=SECONDS>
          Repeatable goodput threshold: ttft=, tpot=, e2e=

      --throughput-bin-seconds <THROUGHPUT_BIN_SECONDS>
          Throughput dispersion bin width in seconds
          
          [default: 10]

      --ca-cert <PATH>
          Additional PEM CA certificate for TLS (must be a CA with basic constraints, not a self-signed leaf; use --insecure for self-signed leaves)

      --insecure
          Disable TLS certificate verification (opt-in; stamped into config)

      --fail-on-error
          Exit non-zero if any measured request failed (default: exit 0 after writing results)

      --sut <PATH>
          Operator-declared SUT block (JSON/YAML) embedded in summary.v3 as sut

      --require-sut
          Refuse to run without a valid --sut block; implies --redact-hostname
          
          [env: METRUM_AI_BENCH_REQUIRE_SUT=]

      --redact-hostname
          Write environment.hostname as null
          
          [env: METRUM_AI_BENCH_REDACT_HOSTNAME=]

      --quiet
          Suppress ASCII banner art (one-line identity still prints). Also set NO_BANNER=1.

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
          [default: round-robin] [possible values: round-robin, least-inflight, random, weighted-round-robin]
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
          [default: constant] [possible values: constant, poisson]
      --max-concurrency <MAX_CONCURRENCY>
          
      --prompt <PROMPT>
          
      --prompts <PROMPTS>
          
      --prompt-field <PROMPT_FIELD>
          [default: prompt]
      --id-field <ID_FIELD>
          [default: id]
      --shuffle-prompts
          
      --warmup-requests <WARMUP_REQUESTS>
          Warmup requests excluded from summary stats [default: 0]
      --seed <SEED>
          
      --seed-mode <SEED_MODE>
          [default: increment] [possible values: fixed, increment, prompt]
      --n <N>
          [default: 1]
      --size <SIZE>
          [default: 1024x1024]
      --response-format <RESPONSE_FORMAT>
          [default: b64_json] [possible values: b64_json, url]
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
      --sut <PATH>
          Operator-declared SUT block (JSON/YAML) embedded in summary.v3 as sut
      --require-sut
          Refuse to run without a valid --sut block; implies --redact-hostname [env: METRUM_AI_BENCH_REQUIRE_SUT=]
      --redact-hostname
          Write environment.hostname as null [env: METRUM_AI_BENCH_REDACT_HOSTNAME=]
  -h, --help
          Print help
  -V, --version
          Print version
```

## `metrum-ai-bench-cli-prompts`

```text
Usage: metrum-ai-bench-cli-prompts [OPTIONS]

Options:
      --version-only
          Print version information and exit
      --quiet
          Suppress ASCII banner art (one-line identity still prints). Also set NO_BANNER=1.
      --dataset <DATASET>
          [default: metrum-ai/prompt-library]
      --revision <REVISION>
          Pinned dataset revision (40-char commit SHA unless --allow-moving-revision)
      --config <CONFIG>
          Dataset config: sample|full [default: sample]
      --split <SPLIT>
          [default: train]
      --allow-moving-revision
          Allow floating revisions such as main (resolves to a commit)
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
          RNG seed for selection [default: 0]
      --isl-target <ISL_TARGET>
          ISL target (same units as --isl-unit)
      --isl-unit <ISL_UNIT>
          [default: tokens] [possible values: words, tokens]
      --isl-stat <ISL_STAT>
          [default: median] [possible values: mean, median]
      --isl-tolerance <ISL_TOLERANCE>
          Absolute ISL tolerance [default: 0]
      --osl-target <OSL_TARGET>
          OSL target (same units as --osl-unit)
      --osl-unit <OSL_UNIT>
          [default: tokens] [possible values: words, tokens]
      --osl-stat <OSL_STAT>
          [default: median] [possible values: mean, median]
      --osl-tolerance <OSL_TOLERANCE>
          Absolute OSL tolerance [default: 0]
      --isl-token-basis <ISL_TOKEN_BASIS>
          [default: supplied-target] [possible values: supplied-target]
      --reasoning <REASONING>
          [default: any] [possible values: any, true, false]
      --max-repeats <MAX_REPEATS>
          Max copies of one source row [default: 8]
      --no-repeats
          Disable repeats (equivalent to --max-repeats 1)
      --osl-tokens-per-word <OSL_TOKENS_PER_WORD>
          Tokens-per-word factor for recommending --max-tokens when --osl-unit words
      --select-work-limit <SELECT_WORK_LIMIT>
          Selector work / iteration budget [default: 50000]
      --output <OUTPUT>
          Write selected prompts as JSONL for metrum-ai-bench-cli-llm
      --report <REPORT>
          Write selection report JSON
  -h, --help
          Print help
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
          [env: OPENAI_API_KEY=] [default: ""]
      --model <MODEL>
          
      --kind <KIND>
          [default: chat] [possible values: chat, embeddings, rerank]
      --streaming
          Stream chat responses to measure TTFT; embeddings and rerank remain JSON
      --requests-per-stage <REQUESTS_PER_STAGE>
          [default: 100]
      --sweep <SWEEP>
          [default: 1,2,4,8]
      --sweep-by <SWEEP_BY>
          [default: concurrency] [possible values: concurrency, rate]
      --max-in-flight <MAX_IN_FLIGHT>
          Maximum outstanding requests during a rate sweep [default: 256]
      --prompt <PROMPT>
          Single prompt string (ignored when --prompts or --sessions is set) [default: Hello]
      --prompts <PROMPTS>
          JSONL prompt file or http(s) URL (objects with "prompt"); cycles across requests
      --max-tokens <MAX_TOKENS>
          Max completion tokens for chat bodies; required when --prompts is set, recommended for all chat sweeps
      --warmup-requests <WARMUP_REQUESTS>
          Per-stage warmup requests excluded from measured aggregates (cold-start control) [default: 0]
      --seed <SEED>
          RNG seed used when --shuffle-prompts is set [default: 0]
      --shuffle-prompts
          Shuffle --prompts with --seed before cycling
      --sessions <SESSIONS>
          
      --prefix-control <PREFIX_CONTROL>
          [default: shared] [possible values: shared, unique, none]
      --shared-prefix <SHARED_PREFIX>
          
      --json-schema <JSON_SCHEMA>
          
      --tools <TOOLS>
          
      --metrics-url <METRICS_URL>
          
      --metrics-interval-ms <METRICS_INTERVAL_MS>
          [default: 250]
      --html <HTML>
          [default: metrum-ai-bench-cli-report.html]
      --csv <CSV>
          [default: metrum-ai-bench-cli-requests.csv]
      --mlperf-dir <MLPERF_DIR>
          
      --mlperf-scenario <MLPERF_SCENARIO>
          [default: server] [possible values: server, offline]
      --otlp-endpoint <OTLP_ENDPOINT>
          
      --otlp-service-name <OTLP_SERVICE_NAME>
          [default: metrum-ai-bench-cli]
      --timeout-seconds <TIMEOUT_SECONDS>
          [default: 300]
      --slo <METRIC=SECONDS>
          Repeatable goodput threshold: e2e=, ttft= (when streaming); tpot= accepted but not measured
      --sut <PATH>
          Operator-declared SUT block (JSON/YAML) embedded in sweep summary and HTML
      --require-sut
          Refuse to run without a valid --sut block; implies --redact-hostname [env: METRUM_AI_BENCH_REQUIRE_SUT=]
      --redact-hostname
          Reserved for parity with modality binaries (strategic stamps SUT only) [env: METRUM_AI_BENCH_REDACT_HOSTNAME=]
  -h, --help
          Print help
  -V, --version
          Print version
```

## `metrum-ai-bench-cli-mock-server`

```text
Usage: metrum-ai-bench-cli-mock-server [OPTIONS]

Options:
      --listen <LISTEN>          [default: 127.0.0.1:8080]
      --latency-ms <LATENCY_MS>  [default: 0]
      --fail-every <FAIL_EVERY>  [default: 0]
  -h, --help                     Print help
  -V, --version                  Print version
```

