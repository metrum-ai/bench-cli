<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# CLI reference

Generated from `metrum-ai-bench-* --help`. Re-run
`scripts/render_cli_help.sh` after flag changes. Live `--help` is
authoritative if this file drifts.

## `metrum-ai-bench-llm`

```text
Usage: metrum-ai-bench-llm [OPTIONS] --scenario <SCENARIO> --num-requests <NUM_REQUESTS> --concurrency <CONCURRENCY> --prompts <PROMPTS> --mode <MODE> --model <MODEL> --data-log <DATA_LOG> --max-tokens <MAX_TOKENS>

Options:
      --version-only
          Print version information and exit
      --ntp-check
          Opt-in NTP clock check; records offset when available (does not hard-fail)
      --scenario <SCENARIO>
          Descriptor for the scenario being run
      --url <URL>
          URL of the AI model endpoint (use --endpoints-file for multiple)
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
          Extra JSON object merged into the request body
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
          API key for authentication (use --endpoints-file for multiple)
      --stop-after-seconds <STOP_AFTER_SECONDS>
          Stop sending new requests after N seconds
      --ramp-up-seconds <RAMP_UP_SECONDS>
          Ramp up period in seconds to gradually increase concurrency
  -h, --help
          Print help
  -V, --version
          Print version
```

## `metrum-ai-bench-vlm`

```text
Usage: metrum-ai-bench-vlm [OPTIONS] --scenario <SCENARIO> --num-requests <NUM_REQUESTS> --concurrency <CONCURRENCY> --prompts <PROMPTS> --model <MODEL> --data-log <DATA_LOG> --max-tokens <MAX_TOKENS>

Options:
      --version-only
          Print version information and exit
      --ntp-check
          Opt-in NTP clock check; records offset when available (does not hard-fail)
      --scenario <SCENARIO>
          Descriptor for the scenario being run
      --url <URL>
          URL of the AI model endpoint (use --endpoints-file for multiple)
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
          Extra JSON object merged into the request body
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
          API key for authentication (use --endpoints-file for multiple)
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

## `metrum-ai-bench-asr`

```text
Usage: metrum-ai-bench-asr [OPTIONS]

Options:
      --version-only
          Print version information and exit

      --ntp-check
          Opt-in NTP clock check; records offset when available (does not hard-fail)

      --scenario <SCENARIO>
          Descriptor for the scenario being run

      --url <URL>
          URL of the audio transcription API endpoint

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
          Extra JSON object merged into the request body

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
          API key for authentication

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

## `metrum-ai-bench-imagegen`

```text
Usage: metrum-ai-bench-imagegen [OPTIONS] --scenario <SCENARIO> --model <MODEL> --num-requests <NUM_REQUESTS> --concurrency <CONCURRENCY> --data-log <DATA_LOG>

Options:
      --version-only
          Print version information and exit
      --ntp-check
          Opt-in NTP clock check; records offset when available (does not hard-fail)
      --scenario <SCENARIO>
          
      --url <URL>
          OpenAI-compatible base URL, usually ending in /v1
      --api-key <API_KEY>
          API key for --url
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
          
      --extra-body-file <EXTRA_BODY_FILE>
          
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
          [default: metrum-ai-bench-imagegen-artifacts]
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

