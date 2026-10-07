// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Unified strategic benchmark runner for chat, embeddings, reranking, VLM,
//! ASR and image generation. Metrum AI.

use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use metrum_ai_bench::strategic::{
    controlled_messages, detect_knee_on_axis, export_csv, export_html, export_mlperf,
    load_sessions, now_unix_ns, scrape_metrics, summarize_stage_with_options, BenchRecord,
    MlperfScenario, PrefixControl, ServerMetrics, Validity,
};
use metrum_ai_bench::sweep_modality::ModalitySample;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use serde_json::{json, Value};
#[cfg(feature = "otlp")]
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum EndpointKind {
    Chat,
    Embeddings,
    Rerank,
    /// Chat completions with `image_url` parts (metrum-ai-bench-cli-vlm bodies).
    Vlm,
    /// `/v1/audio/transcriptions` multipart uploads (metrum-ai-bench-cli-asr forms).
    Asr,
    /// `/v1/images/generations` (metrum-ai-bench-cli-imagegen bodies).
    Imagegen,
}

impl EndpointKind {
    /// Chat completions bodies: streaming, TTFT, reasoning and chat controls.
    fn chat_like(self) -> bool {
        matches!(self, Self::Chat | Self::Vlm)
    }

    /// Kinds whose `usage` output tokens are generated text (#191 `osl_tokens`).
    fn generates_output(self) -> bool {
        matches!(self, Self::Chat | Self::Vlm | Self::Asr)
    }

    /// Stage `modality_metrics` keys; empty for chat, embeddings and rerank.
    fn modality_keys(self) -> &'static [&'static str] {
        match self {
            Self::Vlm => metrum_ai_bench::sweep_modality::VLM_KEYS,
            Self::Asr => metrum_ai_bench::sweep_modality::ASR_KEYS,
            Self::Imagegen => metrum_ai_bench::sweep_modality::IMAGEGEN_KEYS,
            Self::Chat | Self::Embeddings | Self::Rerank => &[],
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ImageDetail {
    Low,
    High,
}

impl ImageDetail {
    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::High => "high",
        }
    }
}

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Sweep and benchmark OpenAI-compatible inference endpoints"
)]
struct Args {
    #[arg(long, help = "Print version information and exit")]
    version_only: bool,
    #[arg(long, required_unless_present = "version_only")]
    url: Option<String>,
    #[arg(long, env = "OPENAI_API_KEY", default_value = "")]
    api_key: String,
    #[arg(long, required_unless_present = "version_only")]
    model: Option<String>,
    #[arg(long, value_enum, default_value = "chat")]
    kind: EndpointKind,
    #[arg(
        long,
        conflicts_with = "tools",
        help = "Stream chat and vlm responses to measure TTFT; embeddings, rerank, asr and imagegen remain unary"
    )]
    streaming: bool,
    #[arg(
        long,
        default_value_t = false,
        help = "When visible-token TTFT is missing, approximate it from HTTP time-to-first-byte and record provenance"
    )]
    infer_ttft_from_first_byte: bool,
    #[arg(long, default_value_t = 100)]
    requests_per_stage: u64,
    #[arg(long, default_value = "1,2,4,8")]
    sweep: String,
    #[arg(long, value_enum, default_value = "concurrency")]
    sweep_by: SweepBy,
    #[arg(
        long,
        default_value_t = 256,
        value_parser = clap::value_parser!(u32).range(1..),
        help = "Maximum outstanding requests during a rate sweep"
    )]
    max_in_flight: u32,
    #[arg(
        long,
        default_value = "Hello",
        help = "Single prompt string (ignored when --prompts or --sessions is set)"
    )]
    prompt: String,
    #[arg(
        long,
        conflicts_with = "sessions",
        help = "JSONL prompt file (objects with \"prompt\"; vlm rows also carry images, as metrum-ai-bench-cli-vlm --prompts); chat and imagegen also accept an http(s) URL; cycles across requests"
    )]
    prompts: Option<String>,
    #[arg(
        long,
        help = "Max completion tokens for chat and vlm bodies; required for vlm and when chat uses --prompts, recommended for all chat sweeps"
    )]
    max_tokens: Option<u32>,
    #[arg(
        long,
        default_value_t = false,
        help = "Send ignore_eos=true in chat request bodies (engine extension; for fixed-length throughput studies)"
    )]
    ignore_eos: bool,
    #[arg(
        long,
        value_name = "N",
        help = "Send min_tokens=N in chat request bodies (engine extension; must be <= --max-tokens)"
    )]
    min_tokens: Option<u32>,
    #[arg(
        long,
        value_name = "JSON",
        help = "Merge extra JSON object fields into chat, vlm or imagegen request bodies"
    )]
    extra_body_json: Option<String>,
    #[arg(
        long,
        help = "Sampling temperature for chat and vlm bodies; omitted from chat bodies when unset, vlm defaults to 0.1 as metrum-ai-bench-cli-vlm"
    )]
    temperature: Option<f32>,
    #[arg(
        long = "image",
        value_name = "PATH_OR_URL",
        help = "--kind vlm: image attached to --prompt (repeatable; local path, http(s) or data: URL); ignored with --prompts"
    )]
    images: Vec<String>,
    #[arg(
        long,
        value_enum,
        default_value = "low",
        help = "--kind vlm: image_url detail"
    )]
    image_detail: ImageDetail,
    #[arg(
        long,
        value_name = "PIXELS",
        help = "--kind vlm: downscale images whose longer side exceeds PIXELS (re-encoded as PNG)"
    )]
    max_image_dimension: Option<u32>,
    #[arg(
        long,
        value_name = "PATH",
        help = "--kind asr: audio samples JSONL (id, path or url, format, optional duration), as metrum-ai-bench-cli-asr --input"
    )]
    audio_samples: Option<String>,
    #[arg(
        long,
        value_name = "PATH",
        help = "--kind asr: reference transcripts JSONL (id, transcript) for stage WER/CER"
    )]
    ground_truth: Option<String>,
    #[arg(
        long,
        default_value = "verbose_json",
        value_parser = ["verbose_json", "json", "text", "srt", "vtt"],
        help = "--kind asr: transcription response_format"
    )]
    asr_response_format: String,
    #[arg(
        long,
        default_value = "en",
        help = "--kind asr: language form field (empty to omit)"
    )]
    language: String,
    #[arg(
        long,
        value_enum,
        default_value_t,
        help = "--kind asr: text normalization applied to both sides of WER/CER"
    )]
    normalizer: metrum_ai_bench::asr::Normalizer,
    #[arg(
        long,
        default_value = "1024x1024",
        help = "--kind imagegen: image size"
    )]
    image_size: String,
    #[arg(
        long,
        default_value_t = 1,
        value_parser = clap::value_parser!(u32).range(1..),
        help = "--kind imagegen: images per request (n)"
    )]
    images_per_request: u32,
    #[arg(
        long,
        default_value = "b64_json",
        value_parser = ["b64_json", "url"],
        help = "--kind imagegen: response_format; b64_json images are decoded and digested"
    )]
    image_response_format: String,
    #[arg(
        long,
        default_value_t = 0,
        help = "Per-stage warmup requests excluded from measured aggregates (cold-start control)"
    )]
    warmup_requests: u64,
    #[arg(
        long,
        default_value_t = 0,
        help = "RNG seed used when --shuffle-prompts is set"
    )]
    seed: u64,
    #[arg(long, help = "Shuffle --prompts with --seed before cycling")]
    shuffle_prompts: bool,
    #[arg(long)]
    sessions: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "shared")]
    prefix_control: PrefixControl,
    #[arg(long)]
    shared_prefix: Option<String>,
    #[arg(long)]
    json_schema: Option<PathBuf>,
    #[arg(long)]
    tools: Option<PathBuf>,
    #[arg(long)]
    metrics_url: Option<String>,
    #[arg(long, default_value_t = 250)]
    metrics_interval_ms: u64,
    #[arg(
        long,
        value_name = "PATH",
        help = "Tagged NDJSON run log (run/stage/request/telemetry/summary rows)"
    )]
    ndjson: Option<PathBuf>,
    #[arg(
        long,
        value_name = "PATH",
        help = "Telemetry scrape YAML (Prometheus /metrics or /metric sources)"
    )]
    telemetry: Option<PathBuf>,
    #[arg(
        long,
        default_value_t = false,
        help = "Abort mid-run after N consecutive scrape failures on any source (default N=3); a failed startup probe fails the run with or without this flag"
    )]
    require_telemetry: bool,
    #[arg(
        long,
        default_value_t = metrum_ai_bench::telemetry::DEFAULT_REQUIRE_FAILURES,
        help = "Consecutive scrape failures before --require-telemetry aborts"
    )]
    require_telemetry_failures: u32,
    #[arg(long, default_value = "metrum-ai-bench-cli-report.html")]
    html: PathBuf,
    #[arg(long, default_value = "metrum-ai-bench-cli-requests.csv")]
    csv: PathBuf,
    #[arg(long)]
    mlperf_dir: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "server")]
    mlperf_scenario: MlperfScenario,
    #[arg(long)]
    otlp_endpoint: Option<String>,
    #[arg(long, default_value = "metrum-ai-bench-cli")]
    otlp_service_name: String,
    #[arg(long, default_value_t = 300)]
    timeout_seconds: u64,
    #[arg(
        long = "slo",
        value_name = "METRIC=VALUE",
        help = "Repeatable goodput threshold: e2e=, ttft=, tpot= (streaming, seconds); user_tps= (tok/s per in-flight user)"
    )]
    slos: Vec<String>,
    #[arg(
        long,
        value_name = "USD_PER_HOUR",
        help = "Declared platform cost ($/hour); overrides sut.cost.price_per_hour for stage cost_per_million_output_tokens"
    )]
    price_per_hour: Option<f64>,
    #[arg(
        long,
        value_name = "TOKENS",
        help = "Expected input tokens for runtime ISL validation (overrides mix-report)"
    )]
    isl_target: Option<f64>,
    #[arg(
        long,
        value_name = "TOKENS",
        help = "Expected output tokens for runtime OSL validation (overrides mix-report)"
    )]
    osl_target: Option<f64>,
    #[arg(
        long,
        default_value_t = 0.0,
        long_help = "Allowed absolute deviation from --isl-target (tokens). Clap default 0.0. Without --prompt-mix-report, 0.0 means exact match. With --prompt-mix-report, 0.0 is a sentinel that uses the report's ISL tolerance. Pass an explicit value for publishable runs."
    )]
    isl_tolerance: f64,
    #[arg(
        long,
        default_value_t = 0.0,
        long_help = "Allowed absolute deviation from --osl-target (tokens). Clap default 0.0. Without --prompt-mix-report, 0.0 means exact match. With --prompt-mix-report, 0.0 is a sentinel that uses the report's OSL tolerance. Required for a meaningful --fail-on-osl-mismatch gate."
    )]
    osl_tolerance: f64,
    #[arg(
        long,
        value_name = "PATH",
        help = "Prompt-library mix report JSON; fills ISL/OSL targets when CLI targets are unset"
    )]
    prompt_mix_report: Option<PathBuf>,
    #[arg(
        long,
        default_value_t = false,
        help = "Exit non-zero when measured OSL mismatches exceed --osl-tolerance"
    )]
    fail_on_osl_mismatch: bool,
    #[arg(
        long,
        value_name = "PATH",
        help = "Operator-declared SUT block (JSON/YAML) embedded in sweep summary and HTML"
    )]
    sut: Option<PathBuf>,
    #[arg(
        long,
        default_value_t = false,
        env = "METRUM_AI_BENCH_REQUIRE_SUT",
        help = "Refuse to run without a complete --sut block (gpu.model, gpu.count, driver_version, runtime.name/version/config, host_os); implies --redact-hostname"
    )]
    require_sut: bool,
    #[arg(
        long,
        default_value_t = false,
        env = "METRUM_AI_BENCH_REDACT_HOSTNAME",
        help = "Reserved for parity with modality binaries (strategic stamps SUT only)"
    )]
    redact_hostname: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SweepBy {
    Concurrency,
    Rate,
}

#[derive(Clone, Debug, Default)]
struct Input {
    /// JSON request body (`Null` for ASR multipart uploads and for VLM,
    /// which sends `json` instead).
    body: Arc<Value>,
    /// `--kind vlm`: the body serialized once at setup and shared by every
    /// request, so a large base64 body is never encoded or copied inside
    /// the send window (#242).
    json: Option<bytes::Bytes>,
    session_id: Option<String>,
    turn: Option<usize>,
    /// `--kind asr`: the audio upload sent as multipart instead of `body`.
    upload: Option<Arc<AudioUpload>>,
    /// `--kind vlm`: image count and payload bytes in `json`.
    images: Option<(usize, u64)>,
    /// `--kind imagegen`: images asked for and whether `b64_json` images are
    /// decoded, read from the body actually sent (after `--extra-body-json`).
    imagegen: Option<ImagegenRequest>,
}

#[derive(Clone, Copy, Debug)]
struct ImagegenRequest {
    requested: u32,
    decode: bool,
}

/// One preloaded audio sample for `--kind asr`.
#[derive(Debug)]
struct AudioUpload {
    file_name: String,
    format: String,
    /// Shared with every request's multipart part; never copied per request.
    bytes: bytes::Bytes,
    seconds: Option<f64>,
    reference: Option<String>,
}

fn parse_sweep(value: &str) -> Result<Vec<f64>> {
    let stages: Vec<f64> = value
        .split(',')
        .map(|part| {
            let number: f64 = part
                .trim()
                .parse()
                .with_context(|| format!("invalid sweep value {part:?}"))?;
            if !number.is_finite() || number <= 0.0 {
                bail!("sweep values must be finite and greater than zero");
            }
            Ok(number)
        })
        .collect::<Result<_>>()?;
    if stages.is_empty() {
        bail!("sweep must contain at least one value");
    }
    if stages.windows(2).any(|pair| pair[0] >= pair[1]) {
        bail!("sweep values must be strictly increasing");
    }
    Ok(stages)
}

fn read_json(path: &Path) -> Result<Value> {
    serde_json::from_reader(
        std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?,
    )
    .with_context(|| format!("parse {}", path.display()))
}

struct ChatBodyOpts<'a> {
    model: &'a str,
    prompt: &'a str,
    streaming: bool,
    max_tokens: Option<u32>,
    ignore_eos: bool,
    min_tokens: Option<u32>,
    temperature: Option<f32>,
    extra_body: Option<&'a Value>,
    shared_prefix: Option<&'a str>,
    prefix_control: PrefixControl,
    session_key: &'a str,
    schema: Option<&'a Value>,
    tools: Option<&'a Value>,
}

fn apply_chat_controls(
    body: &mut Value,
    max_tokens: Option<u32>,
    ignore_eos: bool,
    min_tokens: Option<u32>,
    temperature: Option<f32>,
    extra_body: Option<&Value>,
) -> Result<()> {
    if let Some(max_tokens) = max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if let Some(temperature) = temperature {
        body["temperature"] = json!(decimal_f32(temperature));
    }
    if ignore_eos {
        body["ignore_eos"] = json!(true);
    }
    if let Some(min_tokens) = min_tokens {
        body["min_tokens"] = json!(min_tokens);
    }
    if let Some(extra) = extra_body {
        let Some(dst) = body.as_object_mut() else {
            bail!("chat body must be a JSON object");
        };
        let Some(src) = extra.as_object() else {
            bail!("--extra-body-json must be a JSON object");
        };
        for (key, value) in src {
            dst.insert(key.clone(), value.clone());
        }
    }
    Ok(())
}

fn chat_body(opts: ChatBodyOpts<'_>) -> Result<Value> {
    let messages = controlled_messages(
        &[json!({"role":"user","content":opts.prompt})],
        opts.shared_prefix,
        opts.prefix_control,
        opts.session_key,
    );
    let mut body = json!({"model":opts.model,"messages":messages,"stream":opts.streaming});
    if opts.streaming {
        body["stream_options"] = json!({"include_usage": true});
    }
    apply_chat_controls(
        &mut body,
        opts.max_tokens,
        opts.ignore_eos,
        opts.min_tokens,
        opts.temperature,
        opts.extra_body,
    )?;
    add_structured(&mut body, opts.schema, opts.tools);
    Ok(body)
}

/// An `f32` flag as the `f64` with the same shortest decimal form, so `0.1`
/// stays `0.1` in JSON instead of `0.10000000149011612`.
fn decimal_f32(value: f32) -> f64 {
    value.to_string().parse().unwrap_or(f64::from(value))
}

fn parse_extra_body(raw: Option<&str>) -> Result<Option<Value>> {
    match raw {
        None => Ok(None),
        Some(text) => {
            let value: Value =
                serde_json::from_str(text).context("--extra-body-json must be valid JSON")?;
            if !value.is_object() {
                bail!("--extra-body-json must be a JSON object");
            }
            Ok(Some(value))
        }
    }
}

fn validate_chat_controls(args: &Args) -> Result<()> {
    if (args.ignore_eos || args.min_tokens.is_some()) && !args.kind.chat_like() {
        bail!("--ignore-eos and --min-tokens are only valid for --kind chat or vlm");
    }
    if args.extra_body_json.is_some()
        && !matches!(
            args.kind,
            EndpointKind::Chat | EndpointKind::Vlm | EndpointKind::Imagegen
        )
    {
        bail!("--extra-body-json is only valid for --kind chat, vlm or imagegen");
    }
    if args.temperature.is_some() && !args.kind.chat_like() {
        bail!("--temperature is only valid for --kind chat or vlm");
    }
    if let (Some(min_tokens), Some(max_tokens)) = (args.min_tokens, args.max_tokens) {
        if min_tokens > max_tokens {
            bail!("--min-tokens ({min_tokens}) must be <= --max-tokens ({max_tokens})");
        }
    }
    if args.min_tokens.is_some() && args.max_tokens.is_none() {
        bail!("--min-tokens requires --max-tokens");
    }
    Ok(())
}

fn make_inputs(
    args: &Args,
    model: &str,
    schema: Option<&Value>,
    tools: Option<&Value>,
) -> Result<Vec<Input>> {
    validate_chat_controls(args)?;
    let extra_body = parse_extra_body(args.extra_body_json.as_deref())?;
    if args.prompts.is_some() && args.max_tokens.is_none() {
        bail!("--max-tokens is required when --prompts is set (bounds OSL for comparable sweeps)");
    }
    if matches!(args.kind, EndpointKind::Chat)
        && args.max_tokens.is_none()
        && args.prompts.is_none()
        && args.sessions.is_none()
    {
        eprintln!(
            "warning: chat sweep without --max-tokens; output length is uncontrolled and tok/s is not comparable across configs"
        );
    }
    if let Some(path) = &args.sessions {
        if !matches!(args.kind, EndpointKind::Chat) {
            bail!("--sessions is only valid for chat endpoints");
        }
        let mut inputs = Vec::new();
        for session in load_sessions(path)? {
            for turn in 1..=session.messages.len() {
                let messages = controlled_messages(
                    &session.messages[..turn],
                    args.shared_prefix.as_deref(),
                    args.prefix_control,
                    &session.session_id,
                );
                let mut body = json!({"model":model,"messages":messages,"stream":args.streaming});
                if args.streaming {
                    body["stream_options"] = json!({"include_usage": true});
                }
                apply_chat_controls(
                    &mut body,
                    args.max_tokens,
                    args.ignore_eos,
                    args.min_tokens,
                    args.temperature,
                    extra_body.as_ref(),
                )?;
                add_structured(&mut body, schema, tools);
                inputs.push(Input {
                    body: Arc::new(body),
                    session_id: Some(session.session_id.clone()),
                    turn: Some(turn),
                    ..Input::default()
                });
            }
        }
        return Ok(inputs);
    }
    if let Some(prompts_path) = &args.prompts {
        if !matches!(args.kind, EndpointKind::Chat) {
            bail!("--prompts is only valid for chat endpoints");
        }
        let mut prompts =
            metrum_ai_bench::prompt_inputs::load_metrum_ai_bench_llm_prompts(prompts_path)
                .map_err(|err| anyhow::anyhow!("{err}"))?;
        if args.shuffle_prompts {
            let mut rng = StdRng::seed_from_u64(args.seed);
            prompts.shuffle(&mut rng);
        }
        let mut inputs = Vec::with_capacity(prompts.len());
        for (index, prompt) in prompts.iter().enumerate() {
            let session_key = format!("prompt-{index}");
            inputs.push(Input {
                body: Arc::new(chat_body(ChatBodyOpts {
                    model,
                    prompt,
                    streaming: args.streaming,
                    max_tokens: args.max_tokens,
                    ignore_eos: args.ignore_eos,
                    min_tokens: args.min_tokens,
                    temperature: args.temperature,
                    extra_body: extra_body.as_ref(),
                    shared_prefix: args.shared_prefix.as_deref(),
                    prefix_control: args.prefix_control,
                    session_key: &session_key,
                    schema,
                    tools,
                })?),
                ..Input::default()
            });
        }
        return Ok(inputs);
    }
    let body = match args.kind {
        EndpointKind::Chat => chat_body(ChatBodyOpts {
            model,
            prompt: &args.prompt,
            streaming: args.streaming,
            max_tokens: args.max_tokens,
            ignore_eos: args.ignore_eos,
            min_tokens: args.min_tokens,
            temperature: args.temperature,
            extra_body: extra_body.as_ref(),
            shared_prefix: args.shared_prefix.as_deref(),
            prefix_control: args.prefix_control,
            session_key: "default",
            schema,
            tools,
        })?,
        EndpointKind::Embeddings => json!({"model":model,"input":args.prompt}),
        EndpointKind::Rerank => {
            let documents: Vec<_> = args.prompt.split('|').map(str::trim).collect();
            json!({"model":model,"query":documents.first().copied().unwrap_or(""),"documents":documents.iter().skip(1).collect::<Vec<_>>()})
        }
        EndpointKind::Vlm | EndpointKind::Asr | EndpointKind::Imagegen => {
            bail!("--kind vlm, asr and imagegen inputs are built by their own loaders")
        }
    };
    Ok(vec![Input {
        body: Arc::new(body),
        ..Input::default()
    }])
}

/// What a generation body asks for, read after `--extra-body-json` is merged
/// so an override of `n` or `response_format` is recorded as sent.
fn imagegen_request(body: &Value) -> Result<ImagegenRequest> {
    let requested = body["n"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .context("imagegen body field n must be a positive integer")?;
    let decode = match body["response_format"].as_str() {
        Some("b64_json") => true,
        Some("url") => false,
        other => bail!("imagegen response_format must be b64_json or url, got {other:?}"),
    };
    Ok(ImagegenRequest { requested, decode })
}

/// Reject flags that do not apply to `--kind` before any input is loaded.
fn validate_kind_flags(args: &Args) -> Result<()> {
    let kind = args.kind;
    if !args.images.is_empty() && kind != EndpointKind::Vlm {
        bail!("--image is only valid for --kind vlm");
    }
    if (args.audio_samples.is_some() || args.ground_truth.is_some()) && kind != EndpointKind::Asr {
        bail!("--audio-samples and --ground-truth are only valid for --kind asr");
    }
    if args.max_image_dimension.is_some() && kind != EndpointKind::Vlm {
        bail!("--max-image-dimension is only valid for --kind vlm");
    }
    if kind.modality_keys().is_empty() {
        return Ok(());
    }
    if args.json_schema.is_some() || args.tools.is_some() {
        bail!("--json-schema and --tools are only valid for --kind chat");
    }
    if args.sessions.is_some() {
        bail!("--sessions is only valid for chat endpoints");
    }
    if args.prompts.is_some() && kind == EndpointKind::Asr {
        bail!("--kind asr reads --audio-samples, not --prompts");
    }
    if kind == EndpointKind::Vlm {
        // VLM JSONL rows carry their own images, so --image would be ignored.
        if args.prompts.is_some() && !args.images.is_empty() {
            bail!("--kind vlm takes --image with --prompt, not with --prompts");
        }
        if args.shared_prefix.is_some() {
            bail!("--shared-prefix is not valid for --kind vlm");
        }
    }
    if !kind.chat_like() {
        // Unary uploads and generations: these chat flags would do nothing.
        for (set, flag) in [
            (args.streaming, "--streaming"),
            (args.max_tokens.is_some(), "--max-tokens"),
            (args.shared_prefix.is_some(), "--shared-prefix"),
            (
                args.infer_ttft_from_first_byte,
                "--infer-ttft-from-first-byte",
            ),
        ] {
            if set {
                bail!(
                    "{flag} is not valid for --kind {}",
                    format!("{kind:?}").to_ascii_lowercase()
                );
            }
        }
    }
    validate_chat_controls(args)
}

fn shuffle_if<T>(args: &Args, items: &mut [T]) {
    if args.shuffle_prompts {
        items.shuffle(&mut StdRng::seed_from_u64(args.seed));
    }
}

/// `--kind vlm`: one chat body per prompt row, built with the
/// metrum-ai-bench-cli-vlm request builder. Images load once, before any
/// request, so image I/O never enters a measured latency.
async fn vlm_inputs(args: &Args, model: &str) -> Result<Vec<Input>> {
    let max_tokens = args.max_tokens.context(
        "--kind vlm requires --max-tokens (bounds OSL, as metrum-ai-bench-cli-vlm requires)",
    )?;
    let mut records = match &args.prompts {
        Some(path) => metrum_ai_bench::prompt_inputs::load_metrum_ai_bench_vlm_records(path)
            .map_err(|err| anyhow::anyhow!("{err}"))?,
        None => {
            if args.images.is_empty() {
                bail!("--kind vlm needs --prompts (VLM JSONL) or at least one --image");
            }
            vec![(args.prompt.clone(), args.images.clone())]
        }
    };
    if records.is_empty() {
        bail!("--kind vlm: no prompt rows loaded");
    }
    shuffle_if(args, &mut records);
    let refs = records
        .iter()
        .map(|(_, images)| images.len())
        .sum::<usize>();
    let mut cache = metrum_ai_bench::vlm::ImageCache::new(refs.max(1))
        .map_err(|err| anyhow::anyhow!("{err}"))?;
    let client = reqwest::Client::new();
    let mut inputs = Vec::with_capacity(records.len());
    for (prompt, refs) in &records {
        let mut images = Vec::with_capacity(refs.len());
        for image_ref in refs {
            let image = cache
                .get_or_load(
                    &client,
                    image_ref,
                    args.max_image_dimension,
                    args.timeout_seconds,
                    false,
                )
                .await
                .map_err(|err| {
                    anyhow::anyhow!(
                        "image {}: {err}",
                        metrum_ai_bench::prompt_inputs::image_ref_key(image_ref)
                    )
                })?;
            images.push(image);
        }
        let json = metrum_ai_bench::vlm::build_request_bytes(
            model,
            max_tokens,
            args.temperature.unwrap_or(0.1),
            prompt,
            &images,
            args.image_detail.as_str(),
            false,
            args.streaming,
            args.ignore_eos,
            args.min_tokens,
            args.extra_body_json.as_deref(),
            None,
        )
        .map_err(|err| anyhow::anyhow!("vlm body: {err}"))?;
        inputs.push(Input {
            json: Some(json),
            images: Some((
                images.len(),
                images.iter().map(|image| image.size_bytes).sum(),
            )),
            ..Input::default()
        });
    }
    Ok(inputs)
}

/// `--kind asr`: audio samples (downloaded once when given by URL) read into
/// memory before any request, with their reference transcripts.
async fn asr_inputs(args: &Args) -> Result<Vec<Input>> {
    let path = args.audio_samples.as_deref().context(
        "--kind asr requires --audio-samples JSONL (id, path or url, format, optional duration)",
    )?;
    let mut samples =
        metrum_ai_bench::asr::load_audio_samples(path).map_err(|err| anyhow::anyhow!("{err}"))?;
    if samples.is_empty() {
        bail!("--kind asr: no audio samples in {path}");
    }
    let references = args
        .ground_truth
        .as_deref()
        .map(metrum_ai_bench::asr::load_ground_truth)
        .transpose()
        .map_err(|err| anyhow::anyhow!("{err}"))?;
    shuffle_if(args, &mut samples);
    let client = reqwest::Client::new();
    let mut inputs = Vec::with_capacity(samples.len());
    for sample in samples {
        let local = match (&sample.local_file_path, &sample.url) {
            (Some(local), _) => local.clone(),
            (None, Some(url)) => {
                metrum_ai_bench::asr::download_audio_file(&client, url, &sample.format)
                    .await
                    .map_err(|err| anyhow::anyhow!("download {}: {err}", sample.id))?
            }
            (None, None) => bail!("audio sample {} has no path or url", sample.id),
        };
        let bytes = std::fs::read(&local).with_context(|| format!("read audio {local}"))?;
        // Basename only: the multipart filename never leaks a local path.
        let file_name = Path::new(&local)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("audio.{}", sample.format));
        let reference = references
            .as_ref()
            .and_then(|map| map.get(&sample.id).cloned());
        inputs.push(Input {
            upload: Some(Arc::new(AudioUpload {
                file_name,
                format: sample.format,
                bytes: bytes::Bytes::from(bytes),
                seconds: sample.duration,
                reference,
            })),
            ..Input::default()
        });
    }
    if references.is_some()
        && inputs
            .iter()
            .all(|input| input.upload.as_ref().is_some_and(|u| u.reference.is_none()))
    {
        eprintln!("warning: --ground-truth matched no sample id; stage wer/cer will be n = 0");
    }
    Ok(inputs)
}

/// `--kind imagegen`: one generation body per prompt, built with the
/// metrum-ai-bench-cli-imagegen request builder.
fn imagegen_inputs(args: &Args, model: &str) -> Result<Vec<Input>> {
    let extra = parse_extra_body(args.extra_body_json.as_deref())?;
    let mut prompts = match &args.prompts {
        Some(path) => metrum_ai_bench::prompt_inputs::load_metrum_ai_bench_llm_prompts(path)
            .map_err(|err| anyhow::anyhow!("{err}"))?,
        None => vec![args.prompt.clone()],
    };
    shuffle_if(args, &mut prompts);
    prompts
        .iter()
        .map(|prompt| {
            let body = metrum_ai_bench::imagegen::GenerationBody {
                model,
                prompt,
                n: args.images_per_request,
                size: &args.image_size,
                response_format: &args.image_response_format,
                extra: extra.as_ref().and_then(Value::as_object),
                ..Default::default()
            }
            .to_json();
            Ok(Input {
                imagegen: Some(imagegen_request(&body)?),
                body: Arc::new(body),
                ..Input::default()
            })
        })
        .collect()
}

fn add_structured(body: &mut Value, schema: Option<&Value>, tools: Option<&Value>) {
    if let Some(schema) = schema {
        body["response_format"] = json!({
            "type":"json_schema",
            "json_schema":{"name":"benchmark_output","strict":true,"schema":schema}
        });
    }
    if let Some(tools) = tools {
        body["tools"] = tools.clone();
        body["tool_choice"] = json!("required");
    }
}

fn response_tokens(kind: EndpointKind, response: &Value) -> (u64, u64) {
    match kind {
        // ASR replies carry `usage` only when the server reports it.
        EndpointKind::Chat | EndpointKind::Vlm | EndpointKind::Asr => (
            response
                .pointer("/usage/prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            response
                .pointer("/usage/completion_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        ),
        EndpointKind::Embeddings => (
            response
                .pointer("/usage/prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            0,
        ),
        EndpointKind::Rerank => (
            response
                .pointer("/usage/total_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            0,
        ),
        EndpointKind::Imagegen => (0, 0),
    }
}

/// Server-reported reasoning tokens for chat and vlm responses; `None` when
/// absent and for the other kinds (#192).
fn response_reasoning_tokens(kind: EndpointKind, response: &Value) -> Option<u64> {
    match kind {
        EndpointKind::Chat | EndpointKind::Vlm => response
            .get("usage")
            .and_then(metrum_ai_bench::usage::reasoning_tokens),
        EndpointKind::Embeddings
        | EndpointKind::Rerank
        | EndpointKind::Asr
        | EndpointKind::Imagegen => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_one_request(
    args: &Args,
    url: &str,
    stage: f64,
    input: Input,
    validator: Option<&Validity>,
    seq: Arc<AtomicU64>,
    client: &reqwest::Client,
    semaphore: Arc<Semaphore>,
    inflight_tracker: Arc<metrum_ai_bench::concurrency::InFlightTracker>,
    schedule_epoch: Instant,
    schedule_epoch_unix_ns: u128,
    run_epoch: Arc<metrum_ai_bench::telemetry::RunEpoch>,
    run_id: Arc<String>,
    ndjson: Option<metrum_ai_bench::telemetry::NdjsonWriter>,
    scheduled_offset: Option<Duration>,
    warmup: bool,
    modality: Arc<ModalityOptions>,
) -> tokio::task::JoinHandle<(BenchRecord, ModalitySample)> {
    let client = client.clone();
    let url = url.to_string();
    let api_key = args.api_key.clone();
    let validator = validator.cloned();
    let kind = args.kind;
    let streaming = args.streaming && kind.chat_like();
    let infer_ttft = args.infer_ttft_from_first_byte;
    let sequence = seq.fetch_add(1, Ordering::Relaxed);
    tokio::spawn(async move {
        let permit = metrum_ai_bench::concurrency::acquire_with_engagement(
            Arc::clone(&semaphore),
            &inflight_tracker,
        )
        .await
        .expect("semaphore closed");
        let mut request_slot = Some(metrum_ai_bench::concurrency::InFlightSlot::new(
            &inflight_tracker,
            permit,
        ));
        let in_flight_at_send = request_slot.as_ref().map_or(0, |slot| slot.in_flight());
        // An ASR form (and its request) is assembled here, after the permit
        // (only in-flight requests hold one) and before the send clock; the
        // audio bytes are shared. A VLM body was serialized at setup and is
        // shared too (#242). Other JSON requests are built after the clock
        // starts and serialize inside the send call, as before #197, so
        // chat, embeddings and rerank `service_latency_s` is unchanged.
        let prebuilt_json = input.json.clone().map(|json| {
            Ok(client
                .post(&url)
                .bearer_auth(&api_key)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(json))
        });
        let upload_request = input.upload.as_ref().map(|upload| {
            metrum_ai_bench::asr::transcription_form(
                &modality.model,
                &modality.asr_response_format,
                &modality.language,
                upload.file_name.clone(),
                &upload.format,
                upload.bytes.clone(),
            )
            .map(|form| client.post(&url).bearer_auth(&api_key).multipart(form))
            .map_err(|err| anyhow::anyhow!("{err}"))
        });
        let sent = Instant::now();
        let t_sent_ns = run_epoch.elapsed_ns();
        let sent_unix_ns = now_unix_ns();
        // Closed loop keeps scheduled == sent exactly. Stage summaries rely on
        // that equality to report queue_delay_s only for open-loop stages (#191).
        let (scheduled, scheduled_unix_ns) =
            scheduled_offset.map_or((sent, sent_unix_ns), |offset| {
                (
                    schedule_epoch + offset,
                    schedule_epoch_unix_ns + offset.as_nanos(),
                )
            });
        let t_sched_ns = scheduled
            .checked_duration_since(run_epoch.mono())
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(t_sent_ns);
        let connect_slot = metrum_ai_bench::connect_timing::ConnectSlot::new();
        let request = upload_request
            .or(prebuilt_json)
            .unwrap_or_else(|| Ok(client.post(&url).bearer_auth(&api_key).json(&*input.body)));
        let result = match request {
            Ok(request) => metrum_ai_bench::connect_timing::with_connect_slot(
                Arc::clone(&connect_slot),
                metrum_ai_bench::connect_timing::send(request),
            )
            .await
            .map_err(anyhow::Error::from),
            Err(err) => Err(err),
        };
        let mut first_byte_s = None;
        let mut t_first_ns = None;
        let mut stream_ttft_s = None;
        let mut first_reasoning_s = None;
        let mut itl_s = Vec::new();
        let body_slot = Arc::clone(&connect_slot);
        let result: Result<Reply> = metrum_ai_bench::connect_timing::with_connect_slot(body_slot, async {
            let response = result?;
            first_byte_s.replace(sent.elapsed().as_secs_f64());
            t_first_ns.replace(run_epoch.elapsed_ns());
            let response = response.error_for_status()?;
            if streaming {
                let stream = metrum_ai_bench::chat_stream::consume_with_options(
                    metrum_ai_bench::connect_timing::counted(response.bytes_stream()),
                    sent,
                    infer_ttft,
                )
                .await?;
                stream_ttft_s = stream.ttft.map(|d| d.as_secs_f64());
                first_reasoning_s = stream.first_reasoning.map(|d| d.as_secs_f64());
                itl_s = stream.itl.iter().map(|d| d.as_secs_f64()).collect();
                let mut usage = json!({
                    "prompt_tokens": stream.prompt_tokens,
                    "completion_tokens": stream.completion_tokens
                });
                if let Some(reasoning) = stream.reasoning_tokens {
                    usage["completion_tokens_details"] = json!({"reasoning_tokens": reasoning});
                }
                Ok(Reply::Json(json!({
                    "choices": [{"message": {"role": "assistant", "content": stream.completion_text}}],
                    "usage": usage
                })))
            } else if kind.modality_keys().is_empty() || kind == EndpointKind::Vlm {
                let body = metrum_ai_bench::connect_timing::read_body(response).await?;
                Ok(Reply::Json(serde_json::from_slice::<Value>(&body)?))
            } else {
                // ASR and imagegen: the clock stops once the body is read and
                // parsing happens after it (see docs/METRICS.md for how this
                // differs from the ASR binary).
                Ok(Reply::Raw(metrum_ai_bench::connect_timing::read_body(response).await?))
            }
        })
        .await;
        let http_trace = connect_slot.trace();
        let completed = Instant::now();
        let t_done_ns = run_epoch.elapsed_ns();
        // Modality parsing, WER/CER and image decoding are client work after
        // the body is read: free the slot first so they never hold a permit.
        if !kind.modality_keys().is_empty() {
            drop(request_slot.take());
        }
        let service_latency_s = completed.saturating_duration_since(sent).as_secs_f64();
        let (result, modality_sample) = match result {
            Ok(reply) => match modality
                .finish(kind, &input, reply, service_latency_s)
                .await
            {
                Ok((value, sample)) => (Ok(value), sample),
                Err(err) => (Err(err), ModalitySample::default()),
            },
            Err(err) => (Err(err), ModalitySample::default()),
        };
        let (success, valid, input_tokens, output_tokens, reasoning_tokens, error) = match result {
            Ok(value) => {
                let (input_tokens, output_tokens) = response_tokens(kind, &value);
                (
                    true,
                    validator.as_ref().map(|check| check.validate(&value)),
                    input_tokens,
                    output_tokens,
                    response_reasoning_tokens(kind, &value),
                    None,
                )
            }
            Err(error) => (false, None, 0, 0, None, Some(error.to_string())),
        };
        // InFlightSlot leaves the gauge before freeing the permit (#189).
        drop(request_slot);
        let resolved = metrum_ai_bench::measurement::resolve_ttft(
            streaming,
            stream_ttft_s,
            first_byte_s,
            infer_ttft,
        );
        let record = BenchRecord {
            seq: sequence,
            stage,
            endpoint: url,
            scheduled_unix_ns,
            sent_unix_ns,
            latency_s: completed.saturating_duration_since(scheduled).as_secs_f64(),
            queue_delay_s: sent.saturating_duration_since(scheduled).as_secs_f64(),
            service_latency_s,
            first_byte_s,
            connect_s: Some(http_trace.connect_s),
            ttft_s: resolved.ttft_s,
            ttft_source: resolved.source,
            prefill_s: None,
            decode_s: None,
            decode_tok_s: None,
            itl_s,
            in_flight_at_send: Some(in_flight_at_send),
            success,
            valid,
            input_tokens,
            output_tokens,
            session_id: input.session_id,
            turn: input.turn,
            error: error.clone(),
            warmup,
            first_reasoning_s,
            reasoning_tokens,
            connection_reused: Some(http_trace.connection_reused),
            dns_s: Some(http_trace.dns_s),
            bytes_sent: http_trace.bytes_sent,
            // Body fields only for successes, as on modality request records.
            receive_s: http_trace.receive_s.filter(|_| success),
            bytes_received: http_trace.bytes_received.filter(|_| success),
            chunks_received: http_trace.chunks_received.filter(|_| success),
            // Same origin as the NDJSON `t_sent_ns`; monotonic (#224).
            send_offset_s: Some(t_sent_ns as f64 / 1e9),
        }
        .with_phase_metrics();
        if let Some(writer) = ndjson {
            let _ = writer
                .send_priority(metrum_ai_bench::telemetry::Row::Request(
                    metrum_ai_bench::telemetry::RequestRow {
                        run_id: (*run_id).clone(),
                        seq: sequence,
                        stage,
                        warmup,
                        t_sched_ns,
                        t_sent_ns,
                        t_first_ns,
                        t_done_ns,
                        success,
                        input_tokens,
                        output_tokens,
                        reasoning_tokens: record.reasoning_tokens,
                        latency_s: record.latency_s,
                        queue_delay_s: record.queue_delay_s,
                        service_latency_s: record.service_latency_s,
                        ttft_s: record.ttft_s,
                        ttft_source: record.ttft_source.map(|s| s.as_str().to_string()),
                        error,
                        telemetry_at_done: None,
                    },
                ))
                .await;
        }
        (record, modality_sample)
    })
}

/// Modality settings stamped as `config.modality` (#197); `None` for chat,
/// embeddings and rerank.
fn modality_config(args: &Args, inputs: &[Input]) -> Option<Value> {
    match args.kind {
        EndpointKind::Vlm => Some(json!({
            "pool_images": inputs.iter().filter_map(|input| input.images).map(|(count, _)| count).sum::<usize>(),
            "image_detail": args.image_detail.as_str(),
            "max_image_dimension": args.max_image_dimension,
            "temperature": decimal_f32(args.temperature.unwrap_or(0.1)),
        })),
        EndpointKind::Asr => Some(json!({
            "audio_samples": args.audio_samples,
            "ground_truth": args.ground_truth,
            "references_matched": inputs
                .iter()
                .filter(|input| input.upload.as_ref().is_some_and(|u| u.reference.is_some()))
                .count(),
            "response_format": args.asr_response_format,
            "language": args.language,
            "normalizer": args.normalizer.to_string(),
        })),
        EndpointKind::Imagegen => Some(json!({
            "image_size": args.image_size,
            "images_per_request": args.images_per_request,
            "image_response_format": args.image_response_format,
        })),
        EndpointKind::Chat | EndpointKind::Embeddings | EndpointKind::Rerank => None,
    }
}

/// A response as read inside the timed window.
enum Reply {
    /// Parsed in the window (chat, embeddings, rerank and vlm, as before #197).
    Json(Value),
    /// ASR and imagegen bodies, parsed after the clock stops.
    Raw(bytes::Bytes),
}

/// Modality settings shared by every request of a run (built once).
struct ModalityOptions {
    model: String,
    asr_response_format: String,
    language: String,
    normalizer: metrum_ai_bench::asr::Normalizer,
}

impl ModalityOptions {
    fn from_args(args: &Args) -> Self {
        Self {
            model: args.model.clone().unwrap_or_default(),
            asr_response_format: args.asr_response_format.clone(),
            language: args.language.clone(),
            normalizer: args.normalizer,
        }
    }

    /// Parse a reply and derive its modality values. An unparseable ASR body
    /// or an undecodable imagegen image fails the request, as in their
    /// binaries. ASR scoring and image decoding run on the blocking pool so
    /// they never stall the runtime thread that polls other in-flight requests.
    async fn finish(
        &self,
        kind: EndpointKind,
        input: &Input,
        reply: Reply,
        service_latency_s: f64,
    ) -> Result<(Value, ModalitySample)> {
        match (kind, reply) {
            (EndpointKind::Vlm, Reply::Json(value)) => {
                let sample = input
                    .images
                    .map(|(count, bytes)| ModalitySample::vlm(count, bytes))
                    .unwrap_or_default();
                Ok((value, sample))
            }
            (EndpointKind::Asr, Reply::Raw(body)) => {
                // WER/CER edit distance is O(n * m) in transcript length, so
                // parsing and scoring run on the blocking pool, like imagegen
                // decoding, and never stall other in-flight requests.
                let upload = input.upload.clone();
                let response_format = self.asr_response_format.clone();
                let normalizer = self.normalizer;
                tokio::task::spawn_blocking(move || {
                    let parsed = metrum_ai_bench::asr::parse_transcription(
                        &response_format,
                        &String::from_utf8_lossy(&body),
                    )
                    .map_err(|err| anyhow::anyhow!("{err}"))?;
                    let upload = upload.as_deref();
                    let sample = ModalitySample::asr(
                        &parsed.text,
                        upload.and_then(|u| u.reference.as_deref()),
                        normalizer,
                        upload.and_then(|u| u.seconds),
                        service_latency_s,
                    );
                    // Token usage only when the server reports it (json formats).
                    let value = parsed
                        .usage
                        .map_or(Value::Null, |usage| json!({ "usage": usage }));
                    Ok((value, sample))
                })
                .await
                .context("transcription scoring task failed")?
            }
            (EndpointKind::Imagegen, Reply::Raw(body)) => {
                let request = input
                    .imagegen
                    .context("imagegen input without its request")?;
                let (returned, digests) = tokio::task::spawn_blocking(move || {
                    let parsed: Value = serde_json::from_slice(&body)
                        .map_err(|err| anyhow::anyhow!("schema_error: {err}"))?;
                    metrum_ai_bench::imagegen::response_image_digests(&parsed, request.decode)
                        .map_err(|(kind, message)| anyhow::anyhow!("{kind}: {message}"))
                })
                .await
                .context("image decode task failed")??;
                Ok((
                    Value::Null,
                    ModalitySample::imagegen(request.requested, returned, digests),
                ))
            }
            (_, Reply::Json(value)) => Ok((value, ModalitySample::default())),
            (_, Reply::Raw(body)) => {
                Ok((serde_json::from_slice(&body)?, ModalitySample::default()))
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_stage(
    args: &Args,
    url: &str,
    stage: f64,
    inputs: &[Input],
    validator: Option<&Validity>,
    seq: Arc<AtomicU64>,
    client: &reqwest::Client,
    run_epoch: Arc<metrum_ai_bench::telemetry::RunEpoch>,
    run_id: Arc<String>,
    ndjson: Option<metrum_ai_bench::telemetry::NdjsonWriter>,
    stop: &metrum_ai_bench::runner::StopFlag,
) -> Result<(
    Vec<BenchRecord>,
    Vec<ModalitySample>,
    f64,
    metrum_ai_bench::concurrency::ObservedConcurrency,
)> {
    let concurrency = match args.sweep_by {
        SweepBy::Concurrency => stage.ceil() as usize,
        SweepBy::Rate => args.max_in_flight as usize,
    }
    .max(1);
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let inflight_tracker = Arc::new(metrum_ai_bench::concurrency::InFlightTracker::new(
        concurrency as u32,
    ));
    let modality = Arc::new(ModalityOptions::from_args(args));
    let capacity = args.warmup_requests.saturating_add(args.requests_per_stage) as usize;
    let mut records = Vec::with_capacity(capacity);
    let mut samples = Vec::with_capacity(capacity);

    // Phase 1: fully complete warmup before measurement begins.
    if args.warmup_requests > 0 && !stop.is_stopped() {
        let warmup_epoch = Instant::now();
        let warmup_unix_ns = now_unix_ns();
        let t_start_ns = run_epoch.elapsed_ns();
        let mut warmup_handles = Vec::with_capacity(args.warmup_requests as usize);
        for index in 0..args.warmup_requests {
            if stop.is_stopped() {
                break;
            }
            let input = inputs[index as usize % inputs.len()].clone();
            warmup_handles.push(spawn_one_request(
                args,
                url,
                stage,
                input,
                validator,
                Arc::clone(&seq),
                client,
                Arc::clone(&semaphore),
                Arc::clone(&inflight_tracker),
                warmup_epoch,
                warmup_unix_ns,
                Arc::clone(&run_epoch),
                Arc::clone(&run_id),
                ndjson.clone(),
                None,
                true,
                Arc::clone(&modality),
            ));
        }
        for handle in warmup_handles {
            let (record, sample) = handle.await.context("warmup request task failed")?;
            records.push(record);
            samples.push(sample);
        }
        let t_end_ns = run_epoch.elapsed_ns();
        if let Some(writer) = &ndjson {
            writer
                .send_priority(metrum_ai_bench::telemetry::Row::Stage(
                    metrum_ai_bench::telemetry::StageRow {
                        run_id: (*run_id).clone(),
                        stage,
                        load: stage,
                        phase: metrum_ai_bench::telemetry::PhaseKind::Warmup,
                        t_start_ns,
                        t_end_ns,
                    },
                ))
                .await?;
        }
    }

    // Phase 2: reset measurement epoch; measured prompts restart at index 0.
    // Observed concurrency covers measured slots only (#226).
    inflight_tracker.reset_counts();
    let measure_epoch = Instant::now();
    let measure_unix_ns = now_unix_ns();
    let t_start_ns = run_epoch.elapsed_ns();
    let mut measure_handles = Vec::with_capacity(args.requests_per_stage as usize);
    for index in 0..args.requests_per_stage {
        if stop.is_stopped() {
            break;
        }
        let scheduled_offset = matches!(args.sweep_by, SweepBy::Rate)
            .then(|| Duration::from_secs_f64(index as f64 / stage));
        if let Some(offset) = scheduled_offset {
            tokio::time::sleep(offset.saturating_sub(measure_epoch.elapsed())).await;
        }
        let input = inputs[index as usize % inputs.len()].clone();
        measure_handles.push(spawn_one_request(
            args,
            url,
            stage,
            input,
            validator,
            Arc::clone(&seq),
            client,
            Arc::clone(&semaphore),
            Arc::clone(&inflight_tracker),
            measure_epoch,
            measure_unix_ns,
            Arc::clone(&run_epoch),
            Arc::clone(&run_id),
            ndjson.clone(),
            scheduled_offset,
            false,
            Arc::clone(&modality),
        ));
    }
    for handle in measure_handles {
        let (record, sample) = handle.await.context("request task failed")?;
        records.push(record);
        samples.push(sample);
    }
    let t_end_ns = run_epoch.elapsed_ns();
    if let Some(writer) = &ndjson {
        writer
            .send_priority(metrum_ai_bench::telemetry::Row::Stage(
                metrum_ai_bench::telemetry::StageRow {
                    run_id: (*run_id).clone(),
                    stage,
                    load: stage,
                    phase: metrum_ai_bench::telemetry::PhaseKind::Measure,
                    t_start_ns,
                    t_end_ns,
                },
            ))
            .await?;
    }

    let measured_seconds = metrum_ai_bench::strategic::stage_window_seconds(&records)
        .unwrap_or_else(|| measure_epoch.elapsed().as_secs_f64());
    Ok((
        records,
        samples,
        measured_seconds,
        inflight_tracker.snapshot(),
    ))
}

fn aggregate_server(samples: &[ServerMetrics]) -> ServerMetrics {
    let maximum = |select: fn(&ServerMetrics) -> Option<f64>| {
        samples.iter().filter_map(select).max_by(f64::total_cmp)
    };
    ServerMetrics {
        kv_cache_usage: maximum(|sample| sample.kv_cache_usage),
        preemptions: samples
            .last()
            .and_then(|sample| sample.preemptions)
            .zip(samples.first().and_then(|sample| sample.preemptions))
            .map(|(last, first)| (last - first).max(0.0)),
        requests_running: maximum(|sample| sample.requests_running),
        requests_waiting: maximum(|sample| sample.requests_waiting),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.version_only {
        println!("metrum-ai-bench-cli-strategic version {VERSION}");
        return Ok(());
    }
    let url = args
        .url
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--url is required"))?;
    let model = args
        .model
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--model is required"))?;
    metrum_ai_bench::sut::warn_remote_benchmark_url(&url);
    metrum_ai_bench::measurement::ensure_requests_per_stage(args.requests_per_stage)?;
    let (sut_block, _redact_hostname) = metrum_ai_bench::sut::resolve_sut_flags(
        args.sut.as_deref(),
        args.require_sut,
        args.redact_hostname,
    )?;
    let sut_json = sut_block
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .context("serialize sut")?;
    let stages = parse_sweep(&args.sweep)?;
    // Flag checks before any file is read, so a wrong flag says so first.
    validate_kind_flags(&args)?;
    let schema = args.json_schema.as_deref().map(read_json).transpose()?;
    let tools = args.tools.as_deref().map(read_json).transpose()?;
    if schema.is_some() && tools.is_some() {
        bail!("--json-schema and --tools are mutually exclusive");
    }
    let validator = match (&schema, &tools) {
        (Some(schema), None) => Some(Validity::json_schema(schema)?),
        (None, Some(tools)) => Some(Validity::tool_names(tools)?),
        _ => None,
    };
    let inputs = match args.kind {
        EndpointKind::Vlm => vlm_inputs(&args, &model).await?,
        EndpointKind::Asr => asr_inputs(&args).await?,
        EndpointKind::Imagegen => imagegen_inputs(&args, &model)?,
        EndpointKind::Chat | EndpointKind::Embeddings | EndpointKind::Rerank => {
            make_inputs(&args, &model, schema.as_ref(), tools.as_ref())?
        }
    };
    let run_epoch = Arc::new(metrum_ai_bench::telemetry::RunEpoch::new());
    let run_id = Arc::new(metrum_ai_bench::unique_id::generate_uuid());
    let stop = metrum_ai_bench::runner::StopFlag::new();
    metrum_ai_bench::runner::install_stop_handlers(stop.clone());

    let telemetry_cfg = match (&args.telemetry, &args.metrics_url) {
        (Some(path), _) => Some(metrum_ai_bench::telemetry::TelemetryConfig::load(path)?),
        (None, Some(url)) if args.ndjson.is_some() || args.require_telemetry => Some(
            metrum_ai_bench::telemetry::TelemetryConfig::from_metrics_url(
                url,
                args.metrics_interval_ms,
            ),
        ),
        _ => None,
    };
    if args.require_telemetry && telemetry_cfg.is_none() {
        bail!("--require-telemetry needs --telemetry YAML or --metrics-url");
    }
    if telemetry_cfg.is_some() && args.ndjson.is_none() {
        bail!("--telemetry / telemetry via --metrics-url requires --ndjson PATH");
    }

    // Same optional keys as the stdout config (#197), so NDJSON-only analysis
    // sees the kind settings; chat/embeddings/rerank run rows are unchanged.
    let mut run_config = json!({
        "url": url,
        "model": model,
        "kind": format!("{:?}", args.kind).to_ascii_lowercase(),
        "sweep": args.sweep,
        "sweep_by": format!("{:?}", args.sweep_by).to_ascii_lowercase(),
        "requests_per_stage": args.requests_per_stage,
        "warmup_requests": args.warmup_requests,
        "streaming": args.streaming,
        "telemetry": args.telemetry,
        "metrics_url": args.metrics_url,
    });
    if let Some(temperature) = args.temperature {
        run_config["temperature"] = json!(decimal_f32(temperature));
    }
    if let Some(modality) = modality_config(&args, &inputs) {
        run_config["modality"] = modality;
    }
    // One shared lifecycle with the modality binaries (#196): writer, probe,
    // run row, scrapers, drained summary row.
    let mut telemetry_session = match &args.ndjson {
        Some(path) => Some(
            metrum_ai_bench::telemetry::TelemetrySession::start(
                Some(Arc::clone(&run_epoch)),
                metrum_ai_bench::telemetry::RunStamp {
                    run_id: (*run_id).clone(),
                    tool_version: VERSION.to_string(),
                    sut: sut_json.clone(),
                    config: run_config,
                },
                metrum_ai_bench::telemetry::SessionOptions {
                    ndjson: path.clone(),
                    config: telemetry_cfg.as_ref(),
                    require_telemetry: args.require_telemetry,
                    require_failures: args.require_telemetry_failures,
                    abort: Some(stop.clone()),
                },
            )
            .await?,
        ),
        None => None,
    };
    let ndjson_writer = telemetry_session.as_ref().map(|s| s.writer());

    // Legacy whole-run aggregate for stdout when --metrics-url is set without NDJSON scrapers.
    let stop_legacy = Arc::new(AtomicBool::new(false));
    let server_samples = Arc::new(Mutex::new(Vec::new()));
    let legacy_scraper = if telemetry_cfg.is_none() {
        if let Some(metrics_url) = args.metrics_url.clone() {
            let stop_flag = stop_legacy.clone();
            let samples = server_samples.clone();
            let interval = Duration::from_millis(args.metrics_interval_ms.max(50));
            Some(tokio::spawn(async move {
                let client = reqwest::Client::new();
                while !stop_flag.load(Ordering::Relaxed) {
                    if let Ok(sample) = scrape_metrics(&client, &metrics_url).await {
                        samples.lock().await.push(sample);
                    }
                    tokio::time::sleep(interval).await;
                }
            }))
        } else {
            None
        }
    } else {
        None
    };
    let sequence = Arc::new(AtomicU64::new(0));
    let slos = metrum_ai_bench::summary::SloConfig::parse(&args.slos)?;
    let price =
        metrum_ai_bench::summary::resolve_price_per_hour(args.price_per_hour, sut_block.as_ref());
    let price_per_hour = price.map(|(value, _)| value);
    let mut redacted_config = json!({
        "url": url,
        "model": model,
        "kind": format!("{:?}", args.kind).to_ascii_lowercase(),
        "sweep_by": format!("{:?}", args.sweep_by).to_ascii_lowercase(),
        "requests_per_stage": args.requests_per_stage,
        "warmup_requests": args.warmup_requests,
        "max_in_flight": args.max_in_flight,
        "max_tokens": args.max_tokens,
        "ignore_eos": args.ignore_eos,
        "min_tokens": args.min_tokens,
        "extra_body_json": args.extra_body_json,
        "prompts": args.prompts,
        "prompt_pool_size": inputs.len(),
        "seed": args.seed,
        "shuffle_prompts": args.shuffle_prompts,
        "timeout_seconds": args.timeout_seconds,
        "slos": args.slos,
        "price_per_hour": price_per_hour,
        "price_provenance": price.map(|(_, provenance)| provenance),
        "prefix_control": format!("{:?}", args.prefix_control).to_ascii_lowercase(),
        "json_schema": args.json_schema.is_some(),
        "tools": args.tools.is_some(),
        "streaming": args.streaming,
        "sut": sut_json,
        "require_sut": args.require_sut,
        "ndjson": args.ndjson,
        // Secrets intentionally omitted (api_key never stamped).
    });
    // Additive keys only when used, so chat/embeddings/rerank configs are unchanged.
    if let Some(temperature) = args.temperature {
        redacted_config["temperature"] = json!(decimal_f32(temperature));
    }
    if let Some(modality) = modality_config(&args, &inputs) {
        redacted_config["modality"] = modality;
    }
    let redact_hostname = args.redact_hostname || args.require_sut;
    let environment =
        metrum_ai_bench::environment::collect(None, Some(model.clone()), redact_hostname);
    // Warm connection pool across stages (single shared client with connect timing).
    let client = metrum_ai_bench::http_client::build_http_client(
        metrum_ai_bench::http_client::HttpClientOptions {
            request_timeout: Some(Duration::from_secs(args.timeout_seconds)),
            connect_timeout: Duration::from_secs(args.timeout_seconds.clamp(1, 30)),
            pool_max_idle_per_host: args.max_in_flight as usize,
            pool_idle_timeout: Duration::from_secs(90),
            tcp_keepalive: Duration::from_secs(60),
            ca_cert: None,
            insecure: false,
        },
    )?;
    let isl_osl_targets = metrum_ai_bench::isl_osl::IslOslTargets::resolve(
        args.isl_target,
        args.osl_target,
        args.isl_tolerance,
        args.osl_tolerance,
        args.prompt_mix_report.as_deref(),
    )?;
    let started = Instant::now();
    let mut all_records = Vec::new();
    let mut points = Vec::new();
    let mut any_osl_validation = None;
    let mut partial = false;
    for stage in stages {
        if stop.is_stopped() {
            partial = true;
            break;
        }
        let (records, samples, seconds, observed) = run_stage(
            &args,
            &url,
            stage,
            &inputs,
            validator.as_ref(),
            sequence.clone(),
            &client,
            Arc::clone(&run_epoch),
            Arc::clone(&run_id),
            ndjson_writer.clone(),
            &stop,
        )
        .await?;
        if stop.is_stopped() {
            partial = true;
        }
        let token_rows: Vec<(u64, u64)> = records
            .iter()
            .filter(|r| !r.warmup && r.success)
            .map(|r| (r.input_tokens, r.output_tokens))
            .collect();
        let isl_osl =
            metrum_ai_bench::isl_osl::validate_token_counts(&token_rows, &isl_osl_targets);
        if let Some(ref v) = isl_osl {
            any_osl_validation = Some(v.clone());
        }
        let measured_successes: Vec<_> =
            records.iter().filter(|r| !r.warmup && r.success).collect();
        let missing_ttft = measured_successes
            .iter()
            .filter(|r| r.ttft_s.is_none())
            .count();
        let approx_count = measured_successes
            .iter()
            .filter(|r| {
                matches!(
                    r.ttft_source,
                    Some(metrum_ai_bench::measurement::TtftSource::FirstByteApprox)
                )
            })
            .count();
        let no_output_token_errors = records
            .iter()
            .filter(|r| {
                !r.warmup
                    && r.error
                        .as_deref()
                        .is_some_and(|msg| msg.contains("no output token"))
            })
            .count();
        let ttft_audit = if args.kind.chat_like() {
            metrum_ai_bench::measurement::audit_chat_ttft(
                args.streaming,
                args.infer_ttft_from_first_byte,
                measured_successes.len(),
                missing_ttft,
                approx_count,
                no_output_token_errors,
            )?
        } else {
            metrum_ai_bench::measurement::TtftAudit {
                approx_count: 0,
                warning: None,
            }
        };
        let mut point = summarize_stage_with_options(
            stage,
            &records,
            seconds,
            &slos,
            Some(redacted_config.clone()),
            price_per_hour,
            Some(observed),
            isl_osl,
            args.kind.generates_output(),
        );
        point.modality_metrics = metrum_ai_bench::sweep_modality::stage_metrics(
            args.kind.modality_keys(),
            &records,
            &samples,
        );
        if args.kind == EndpointKind::Imagegen {
            point.image_digests = Some(metrum_ai_bench::sweep_modality::stage_image_digests(
                &records, &samples,
            ));
        }
        point.ttft_approx_count = ttft_audit.approx_count;
        point.ttft_warning = ttft_audit.warning;
        points.push(point);
        all_records.extend(records);
    }
    stop_legacy.store(true, Ordering::Relaxed);
    if let Some(session) = telemetry_session.as_mut() {
        if let Err(err) = session.join_scrapers().await {
            // Close the NDJSON (partial) so rows already queued are kept.
            drop(ndjson_writer);
            if let Some(session) = telemetry_session.take() {
                if let Err(close) = session.finish(true).await {
                    eprintln!("warning: {close:#}");
                }
            }
            return Err(err);
        }
    }
    if let Some(scraper) = legacy_scraper {
        scraper.await?;
    }
    let duration_s = started.elapsed().as_secs_f64();
    let server = aggregate_server(&server_samples.lock().await);
    let knee = detect_knee_on_axis(
        &points,
        match args.sweep_by {
            SweepBy::Concurrency => metrum_ai_bench::strategic::KneeLoadAxis::Concurrency,
            SweepBy::Rate => metrum_ai_bench::strategic::KneeLoadAxis::Rate,
        },
    );
    // Single-stage and sessions runs never expect a knee; keep them quiet.
    if points.len() >= 2 {
        if let Some(note) = knee.note() {
            eprintln!("note: {note}");
        }
    }
    export_csv(&args.csv, &all_records)?;
    export_html(
        &args.html,
        "Metrum AI Bench strategic sweep",
        &points,
        &knee,
        &server,
        sut_json.as_ref(),
    )?;
    if let Some(directory) = &args.mlperf_dir {
        export_mlperf(directory, args.mlperf_scenario, &all_records, duration_s)?;
    }
    #[cfg(feature = "otlp")]
    if let Some(endpoint) = &args.otlp_endpoint {
        let headers: BTreeMap<String, String> = std::env::var("OTEL_EXPORTER_OTLP_HEADERS")
            .ok()
            .map(|value| {
                value
                    .split(',')
                    .filter_map(|pair| pair.split_once('='))
                    .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
                    .collect()
            })
            .unwrap_or_default();
        metrum_ai_bench::strategic::export_otlp(
            &reqwest::Client::new(),
            endpoint,
            &args.otlp_service_name,
            &all_records,
            &headers,
        )
        .await?;
    }
    #[cfg(not(feature = "otlp"))]
    if args.otlp_endpoint.is_some() {
        bail!("OTLP export requires a build with --features otlp");
    }
    drop(ndjson_writer);
    let dropped_telemetry_rows = match telemetry_session {
        Some(session) => session.finish(partial).await?.dropped_telemetry_rows,
        None => 0,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "schema_version": "metrum-ai-bench-cli.strategic.v1",
            "tool_version": VERSION,
            "environment": environment,
            "config": redacted_config,
            "points": points,
            "knee": knee.index.map(|index| &points[index]),
            "knee_detection": knee,
            "server_metrics": server,
            "sut": sut_json,
            "records_csv": args.csv,
            "html_report": args.html,
            "partial": partial,
            "ndjson": args.ndjson,
            "dropped_telemetry_rows": dropped_telemetry_rows,
        }))?
    );
    if let Some(ref validation) = any_osl_validation {
        metrum_ai_bench::isl_osl::enforce_osl_gate(validation, args.fail_on_osl_mismatch)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_require_max_tokens() {
        let args = Args::try_parse_from([
            "bench",
            "--url",
            "http://localhost",
            "--model",
            "dummy",
            "--prompts",
            "/tmp/missing.jsonl",
        ])
        .expect("args");
        let err = make_inputs(&args, "dummy", None, None).expect_err("max-tokens required");
        assert!(err.to_string().contains("--max-tokens"));
    }

    #[test]
    fn chat_body_includes_max_tokens() {
        let args = Args::try_parse_from([
            "bench",
            "--url",
            "http://localhost",
            "--model",
            "dummy",
            "--max-tokens",
            "32",
            "--streaming",
        ])
        .expect("args");
        let inputs = make_inputs(&args, "dummy", None, None).expect("inputs");
        assert_eq!(inputs[0].body["max_tokens"], 32);
        assert_eq!(inputs[0].body["stream"], true);
    }

    #[test]
    fn chat_body_includes_ignore_eos_and_min_tokens() {
        let args = Args::try_parse_from([
            "bench",
            "--url",
            "http://localhost",
            "--model",
            "dummy",
            "--max-tokens",
            "64",
            "--ignore-eos",
            "--min-tokens",
            "64",
            "--extra-body-json",
            r#"{"temperature":0.0}"#,
        ])
        .expect("args");
        let inputs = make_inputs(&args, "dummy", None, None).expect("inputs");
        assert_eq!(inputs[0].body["ignore_eos"], true);
        assert_eq!(inputs[0].body["min_tokens"], 64);
        assert_eq!(inputs[0].body["temperature"], 0.0);
    }

    #[test]
    fn min_tokens_must_not_exceed_max_tokens() {
        let args = Args::try_parse_from([
            "bench",
            "--url",
            "http://localhost",
            "--model",
            "dummy",
            "--max-tokens",
            "16",
            "--min-tokens",
            "32",
        ])
        .expect("args");
        let err = make_inputs(&args, "dummy", None, None).expect_err("min>max");
        assert!(err.to_string().contains("--min-tokens"));
    }

    #[test]
    fn ignore_eos_rejected_for_embeddings() {
        let args = Args::try_parse_from([
            "bench",
            "--url",
            "http://localhost",
            "--model",
            "dummy",
            "--kind",
            "embeddings",
            "--ignore-eos",
        ])
        .expect("args");
        let err = make_inputs(&args, "dummy", None, None).expect_err("chat-only");
        assert!(err.to_string().contains("chat"));
    }

    #[test]
    fn streaming_only_changes_chat_inputs() {
        for kind in ["chat", "embeddings", "rerank"] {
            for streaming in [false, true] {
                let mut flags = vec![
                    "bench",
                    "--url",
                    "http://localhost",
                    "--model",
                    "dummy",
                    "--kind",
                    kind,
                ];
                if streaming {
                    flags.push("--streaming");
                }
                let args = Args::try_parse_from(flags).expect("args");
                let inputs = make_inputs(&args, "dummy", None, None).expect("inputs");
                let body = &inputs[0].body;
                if kind == "chat" {
                    assert_eq!(body["stream"], streaming);
                    assert_eq!(
                        body["stream_options"]["include_usage"].as_bool(),
                        streaming.then_some(true)
                    );
                } else {
                    assert!(body.get("stream").is_none());
                    assert!(body.get("stream_options").is_none());
                }
            }
        }
    }

    fn imagegen_input(decode: bool) -> Input {
        Input {
            imagegen: Some(ImagegenRequest {
                requested: 1,
                decode,
            }),
            ..Input::default()
        }
    }

    fn options() -> ModalityOptions {
        ModalityOptions {
            model: "m".into(),
            asr_response_format: "json".into(),
            language: String::new(),
            normalizer: metrum_ai_bench::asr::Normalizer::default(),
        }
    }

    #[tokio::test]
    async fn undecodable_image_fails_the_request() {
        let reply = Reply::Raw(bytes::Bytes::from_static(
            br#"{"data":[{"b64_json":"!!not base64"}]}"#,
        ));
        let err = options()
            .finish(EndpointKind::Imagegen, &imagegen_input(true), reply, 0.1)
            .await
            .expect_err("decode error");
        assert!(err.to_string().contains("decode_error"), "{err}");
        let not_json = Reply::Raw(bytes::Bytes::from_static(b"<html>"));
        let err = options()
            .finish(EndpointKind::Imagegen, &imagegen_input(true), not_json, 0.1)
            .await
            .expect_err("schema error");
        assert!(err.to_string().contains("schema_error"), "{err}");
    }

    #[tokio::test]
    async fn url_images_count_without_digests() {
        let reply = Reply::Raw(bytes::Bytes::from_static(
            br#"{"data":[{"url":"http://x/1.png"}]}"#,
        ));
        let (_, sample) = options()
            .finish(EndpointKind::Imagegen, &imagegen_input(false), reply, 0.1)
            .await
            .expect("url reply");
        assert_eq!(sample.metrics["images_returned"], 1.0);
        assert!(sample.image_sha256.is_empty());
    }

    #[tokio::test]
    async fn asr_reply_keeps_server_usage_only() {
        let reply = Reply::Raw(bytes::Bytes::from_static(
            br#"{"text":"hello","usage":{"prompt_tokens":3,"completion_tokens":2}}"#,
        ));
        let (value, _) = options()
            .finish(EndpointKind::Asr, &Input::default(), reply, 0.1)
            .await
            .expect("asr reply");
        assert_eq!(response_tokens(EndpointKind::Asr, &value), (3, 2));
        let bare = Reply::Raw(bytes::Bytes::from_static(br#"{"text":"hello"}"#));
        let (value, _) = options()
            .finish(EndpointKind::Asr, &Input::default(), bare, 0.1)
            .await
            .expect("asr reply");
        assert_eq!(response_tokens(EndpointKind::Asr, &value), (0, 0));
    }

    #[test]
    fn imagegen_request_reads_the_sent_body() {
        let request = imagegen_request(&json!({"n": 4, "response_format": "url"})).expect("ok");
        assert_eq!((request.requested, request.decode), (4, false));
        assert!(imagegen_request(&json!({"n": 1, "response_format": "png"})).is_err());
        assert!(imagegen_request(&json!({"n": "two", "response_format": "url"})).is_err());
    }

    #[test]
    fn temperature_keeps_its_decimal_form() {
        assert_eq!(decimal_f32(0.1), 0.1);
        let args = Args::try_parse_from([
            "bench",
            "--url",
            "http://localhost",
            "--model",
            "dummy",
            "--max-tokens",
            "8",
            "--temperature",
            "0.7",
        ])
        .expect("args");
        let inputs = make_inputs(&args, "dummy", None, None).expect("inputs");
        assert_eq!(inputs[0].body["temperature"], 0.7);
    }

    #[test]
    fn sweep_parser_rejects_invalid_values() {
        assert_eq!(parse_sweep("1,2.5,4").unwrap(), vec![1.0, 2.5, 4.0]);
        assert!(parse_sweep("1,0").is_err());
        assert!(parse_sweep("2,1").is_err());
        assert!(parse_sweep("x").is_err());
    }

    #[test]
    fn chat_body_default_is_user_only() {
        let body = chat_body(ChatBodyOpts {
            model: "dummy",
            prompt: "hello",
            streaming: false,
            max_tokens: Some(16),
            ignore_eos: false,
            min_tokens: None,
            temperature: None,
            extra_body: None,
            shared_prefix: None,
            prefix_control: PrefixControl::None,
            session_key: "s0",
            schema: None,
            tools: None,
        })
        .expect("body");
        let messages = body["messages"].as_array().expect("messages");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "hello");
    }
}
