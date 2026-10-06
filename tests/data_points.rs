// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! #202 Metrum AI Bench published data-point counts, derived from the serde
//! schemas and checked against real runs.
//!
//! Every schema struct is built twice with exhaustive struct literals (no
//! `..`): a maximal instance with every `Option` set, and a minimal one with
//! every `Option` unset, empty collections, and zero skip counters. The
//! compiler rejects a new field until both fixtures list it, and
//! [`schema_fixtures_cover_every_field`] fails when a maximal fixture leaves a
//! field out of the serialized JSON. Counting walks the serialized JSON with
//! the rules of `scripts/parity/count_points.py`, so a schema count and a
//! harness count use one definition. Optional fields (in the maximal JSON but
//! not the minimal one) must each have a documented [`Condition`].
//!
//! `docs/DATA_POINTS.md` is generated here. Regenerate it with
//! `scripts/render_data_points.sh`; [`data_points_doc_is_current`] fails CI
//! when the committed file is stale.

mod common;

use chrono::Utc;
use metrum_ai_bench::concurrency::ObservedConcurrency;
use metrum_ai_bench::error::RequestError;
use metrum_ai_bench::isl_osl::IslOslValidation;
use metrum_ai_bench::measurement::TtftSource;
use metrum_ai_bench::record::{Phase, RequestRecord, SCHEMA_VERSION_REQUEST};
use metrum_ai_bench::stats::DistSummary;
use metrum_ai_bench::strategic::SweepPoint;
use metrum_ai_bench::summary::{EndpointSummary, GoodputSummary, RunSummary, SloConfig};
use metrum_ai_bench::sweep_modality::{self, ImageDigests};
use metrum_ai_bench::telemetry::{RequestRow, TelemetryRunInfo};
use metrum_ai_bench::time_weighted::{TimeWeightedMetrics, TimeWeightedStat};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Run-metadata keys never counted (as in `count_points.py`).
const SUMMARY_EXCLUDED: &[&str] = &["schema_version", "config", "environment", "sut"];
/// Counted separately: per-endpoint copies and error-shaped data.
const SUMMARY_SEPARATE: &[&str] = &["per_endpoint", "errors_by_type"];
/// Top-level request keys never counted (as in `count_points.py`).
const REQUEST_SKIP: &[&str] = &["seq", "error"];
/// Open per-modality maps: keys are set by each binary, not by the schema.
const REQUEST_OPEN_MAPS: &[&str] = &["modality_metrics", "modality_labels"];
/// `modality_metrics` keys the llm binary writes on every success.
const LLM_MODALITY_KEYS: &[&str] = &["completion_words", "prompt_words"];
/// Telemetry NDJSON request-row keys never counted, plus the open map of
/// YAML-selected series.
const REQUEST_ROW_SKIP: &[&str] = &["seq", "error", "telemetry_at_done"];
/// Sweep-point keys never counted.
const SWEEP_EXCLUDED: &[&str] = &["config"];

const DOC: &str = "docs/DATA_POINTS.md";
const BLESS_ENV: &str = "METRUM_BENCH_BLESS_DATA_POINTS";

// Conditions

/// What a run needs for an optional field to fire. An optional field lists
/// one or more conditions and fires when any of them holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Condition {
    HttpPath,
    Usage,
    Streaming,
    TtftApprox,
    ReasoningStream,
    ReasoningUsage,
    Tokenizer,
    OpenLoop,
    Slo,
    UserTpsSlo,
    Price,
    IslOslTargets,
    Ndjson,
    Validation,
    MeasuredSuccess,
    VlmSweep,
    AsrSweep,
    ImagegenSweep,
}

impl Condition {
    fn name(self) -> &'static str {
        match self {
            Self::HttpPath => "http",
            Self::Usage => "usage",
            Self::Streaming => "streaming",
            Self::TtftApprox => "ttft-approx",
            Self::ReasoningStream => "reasoning-stream",
            Self::ReasoningUsage => "reasoning-usage",
            Self::Tokenizer => "tokenizer",
            Self::OpenLoop => "open-loop",
            Self::Slo => "slo",
            Self::UserTpsSlo => "user-tps-slo",
            Self::Price => "price",
            Self::IslOslTargets => "isl-osl-targets",
            Self::Ndjson => "ndjson",
            Self::Validation => "validation",
            Self::MeasuredSuccess => "measured-success",
            Self::VlmSweep => "vlm-sweep",
            Self::AsrSweep => "asr-sweep",
            Self::ImagegenSweep => "imagegen-sweep",
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Self::HttpPath => {
                "Set by the bundled binaries' HTTP request path (llm, vlm, asr, \
                 imagegen, strategic). Response body fields (`receive_s`, \
                 `bytes_received`, `chunks_received`) are on successes only."
            }
            Self::Usage => "The server reports `usage` token counts.",
            Self::Streaming => {
                "`--streaming` chat or vlm. `itl_s` needs two or more content \
                 chunks, `decode_tok_s` a positive decode time and output tokens."
            }
            Self::TtftApprox => {
                "`--infer-ttft-from-first-byte` on a non-streaming chat run \
                 (TTFT approximated from the first response byte)."
            }
            Self::ReasoningStream => "The server streams `reasoning_content` deltas.",
            Self::ReasoningUsage => {
                "The server reports `usage.completion_tokens_details.reasoning_tokens` \
                 (or an accepted variant)."
            }
            Self::Tokenizer => {
                "`--tokenizer` is set (client token counts; also fills \
                 `usage_missing` rows)."
            }
            Self::OpenLoop => "Open-loop arrivals (`--request-rate`).",
            Self::Slo => "At least one `--slo` threshold.",
            Self::UserTpsSlo => "`--slo user_tps=...`.",
            Self::Price => "`--price-per-hour` or SUT `cost.price_per_hour`.",
            Self::IslOslTargets => "`--isl-target` or `--osl-target`.",
            Self::Ndjson => "`--ndjson PATH`.",
            Self::Validation => "Strategic response validation (`--json-schema` or `--tools`).",
            Self::MeasuredSuccess => "The stage has at least one measured request or success.",
            Self::VlmSweep => "`metrum-ai-bench-cli-strategic --kind vlm`.",
            Self::AsrSweep => {
                "`metrum-ai-bench-cli-strategic --kind asr`. `wer` and `cer` are \
                 `n = 0` without `--ground-truth`; `audio_duration_s` and \
                 `rtfx_client` are `n = 0` without a sample `duration`."
            }
            Self::ImagegenSweep => {
                "`metrum-ai-bench-cli-strategic --kind imagegen`. `image_digests` \
                 counts decoded `b64_json` images (0 for `url` responses)."
            }
        }
    }
}

use Condition::*;

/// Optional `summary.v3` quantities and when they fire.
const SUMMARY_ROWS: &[(&str, &[Condition])] = &[
    ("completion_tokens_per_second", &[Usage, Tokenizer]),
    ("completion_tokens_total", &[Usage, Tokenizer]),
    ("connection_reuse_rate", &[HttpPath]),
    ("connections_reused", &[HttpPath]),
    ("cost_per_million_output_tokens", &[Price]),
    ("input_tokens_per_second", &[Usage, Tokenizer]),
    ("isl_osl", &[IslOslTargets]),
    ("observed_concurrency", &[HttpPath]),
    ("price_per_hour", &[Price]),
    ("prompt_tokens_total", &[Usage, Tokenizer]),
    ("reasoning_tokens_total", &[ReasoningUsage]),
    ("telemetry", &[Ndjson]),
    ("total_tokens_per_second", &[Usage, Tokenizer]),
    ("ttft_approx_count", &[TtftApprox]),
    ("visible_completion_tokens_total", &[ReasoningUsage]),
];

/// Optional `request.v3` numeric fields and when they fire.
const REQUEST_ROWS: &[(&str, &[Condition])] = &[
    ("bytes_received", &[HttpPath]),
    ("bytes_sent", &[HttpPath]),
    ("chunks_received", &[HttpPath]),
    ("connect_s", &[HttpPath]),
    ("decode_s", &[Streaming, TtftApprox]),
    ("decode_tok_s", &[Streaming, TtftApprox]),
    ("dns_s", &[HttpPath]),
    ("first_byte_s", &[HttpPath]),
    ("first_reasoning_s", &[ReasoningStream]),
    ("in_flight_at_send", &[HttpPath]),
    ("itl_s", &[Streaming]),
    ("prefill_s", &[Streaming, TtftApprox]),
    ("reasoning_tokens", &[ReasoningUsage]),
    ("receive_s", &[HttpPath]),
    ("scheduled_offset_s", &[OpenLoop]),
    ("send_offset_s", &[HttpPath]),
    ("tokenized_completion_tokens", &[Tokenizer]),
    ("tokenized_prompt_tokens", &[Tokenizer]),
    ("ttft_s", &[Streaming, TtftApprox]),
    ("visible_completion_tokens", &[ReasoningUsage]),
];

/// Optional strategic sweep-point quantities and when they fire.
const SWEEP_ROWS: &[(&str, &[Condition])] = &[
    ("completion_tokens_per_second", &[Usage]),
    ("completion_tokens_total", &[Usage]),
    ("connection_reuse_rate", &[HttpPath]),
    ("connections_reused", &[HttpPath]),
    ("cost_per_million_output_tokens", &[Price]),
    ("error_rate", &[MeasuredSuccess]),
    ("image_digests", &[ImagegenSweep]),
    ("input_tokens_per_second", &[Usage]),
    ("isl_osl", &[IslOslTargets]),
    ("observed_concurrency", &[HttpPath]),
    ("p50_s", &[MeasuredSuccess]),
    ("p95_s", &[MeasuredSuccess]),
    ("p99_s", &[MeasuredSuccess]),
    ("prompt_tokens_total", &[Usage]),
    ("reasoning_tokens_total", &[ReasoningUsage]),
    ("slo_thresholds_s", &[Slo]),
    ("total_tokens_per_second", &[Usage]),
    ("ttft_approx_count", &[TtftApprox]),
    ("users_at_slo", &[UserTpsSlo]),
    ("users_meeting_user_tps", &[UserTpsSlo]),
    ("validity_rate", &[Validation]),
    ("visible_completion_tokens_total", &[ReasoningUsage]),
];

/// Optional telemetry NDJSON `request` row numeric fields and when they fire.
const REQUEST_ROW_ROWS: &[(&str, &[Condition])] = &[
    ("reasoning_tokens", &[ReasoningUsage]),
    ("t_first_ns", &[HttpPath]),
    ("ttft_s", &[Streaming, TtftApprox]),
];

/// One optional field and the conditions that fire it.
type Row = (String, &'static [Condition]);

fn rows(table: &[(&str, &'static [Condition])]) -> Vec<Row> {
    table.iter().map(|(k, c)| (k.to_string(), *c)).collect()
}

fn summary_optional() -> Vec<Row> {
    rows(SUMMARY_ROWS)
}

fn request_optional() -> Vec<Row> {
    rows(REQUEST_ROWS)
}

fn request_row_optional() -> Vec<Row> {
    rows(REQUEST_ROW_ROWS)
}

/// Sweep-point rows plus one `modality_metrics.<key>` distribution per
/// strategic modality key, read from `sweep_modality` (#197).
fn sweep_optional() -> Vec<Row> {
    let mut out = rows(SWEEP_ROWS);
    for (keys, kind) in modality_kinds() {
        out.extend(
            keys.iter()
                .map(|key| (format!("modality_metrics.{key}"), kind)),
        );
    }
    out
}

/// Strategic modality key lists and the sweep kind that fills each.
fn modality_kinds() -> [(&'static [&'static str], &'static [Condition]); 3] {
    [
        (sweep_modality::VLM_KEYS, &[VlmSweep]),
        (sweep_modality::ASR_KEYS, &[AsrSweep]),
        (sweep_modality::IMAGEGEN_KEYS, &[ImagegenSweep]),
    ]
}

/// Conditions a plain closed-loop `--streaming` llm run against a server
/// that reports usage meets (the parity harness `plain` scenario).
const PLAIN_RUN: &[Condition] = &[HttpPath, Usage, Streaming];

// Fixtures

fn dist_full() -> DistSummary {
    DistSummary::from_values(&[1.0, 2.0, 3.0, 4.0, 5.0])
}

fn dist_empty() -> DistSummary {
    DistSummary::from_values(&[])
}

fn tw_full() -> TimeWeightedStat {
    TimeWeightedStat {
        n: 2,
        avg: Some(1.5),
        active_avg: Some(2.0),
        max: Some(2.0),
        active_s: Some(0.75),
    }
}

fn tw_metrics(stat: fn() -> TimeWeightedStat) -> TimeWeightedMetrics {
    TimeWeightedMetrics {
        effective_concurrency: stat(),
        effective_prefill_concurrency: stat(),
        effective_decode_concurrency: stat(),
        tokens_in_flight: stat(),
        effective_prefill_throughput: stat(),
        effective_decode_throughput: stat(),
    }
}

fn observed_concurrency() -> ObservedConcurrency {
    ObservedConcurrency {
        cap: 4,
        in_flight_mean: Some(3.5),
        in_flight_p50: Some(4.0),
        in_flight_max: Some(4.0),
        cap_engagement_fraction: Some(0.25),
        acquire_count: 8,
        wait_count: 2,
    }
}

fn isl_osl() -> IslOslValidation {
    IslOslValidation {
        isl_target: Some(128.0),
        osl_target: Some(64.0),
        isl_tolerance: 8.0,
        osl_tolerance: 8.0,
        source: Some("cli"),
        length_basis: Some("server_usage"),
        isl_mean: Some(127.0),
        isl_p50: Some(128.0),
        osl_mean: Some(63.0),
        osl_p50: Some(64.0),
        isl_mismatch_count: 0,
        osl_mismatch_count: 1,
        compared: 8,
    }
}

fn telemetry_info() -> TelemetryRunInfo {
    TelemetryRunInfo {
        schema_version: "metrum-ai-bench-cli.telemetry.v1",
        ndjson: "run.ndjson".into(),
        sources: 1,
        request_rows: 8,
        stage_rows: 2,
        telemetry_rows: 40,
        scrape_error_rows: 0,
        dropped_telemetry_rows: 0,
    }
}

fn all_slos() -> SloConfig {
    SloConfig {
        ttft_s: Some(0.2),
        tpot_s: Some(0.05),
        e2e_s: Some(2.0),
        user_tps: Some(20.0),
    }
}

/// SLO threshold map exactly as the summary builder writes it.
fn summary_thresholds() -> BTreeMap<String, f64> {
    RunSummary::from_records_with_options(&[], 1.0, false, &all_slos(), 10.0)
        .goodput
        .thresholds_s
}

fn endpoint_summary(dist: fn() -> DistSummary) -> EndpointSummary {
    EndpointSummary {
        attempted: 8,
        successes: 8,
        errors: 0,
        latency_s: dist(),
        ttft_s: dist(),
        tpot_s: dist(),
        itl_s: dist(),
        first_byte_s: dist(),
        queue_delay_s: dist(),
        first_reasoning_s: dist(),
        isl_tokens: dist(),
        isl_tokens_source: Some("server_usage"),
        osl_tokens: dist(),
        osl_tokens_source: Some("server_usage"),
        reasoning_tokens: dist(),
        visible_completion_tokens: dist(),
    }
}

fn summary_max() -> RunSummary {
    let d = dist_full;
    RunSummary {
        schema_version: metrum_ai_bench::record::SCHEMA_VERSION_SUMMARY,
        attempted: 8,
        successes: 7,
        errors: 1,
        error_rate: 0.125,
        errors_by_type: BTreeMap::from([("timeout".to_string(), 1)]),
        window_seconds: 2.0,
        requests_per_second: 3.5,
        usage_missing_count: 1,
        completion_tokens_per_second: Some(100.0),
        completion_tokens_source: Some("server_usage"),
        prompt_tokens_total: Some(400),
        completion_tokens_total: Some(200),
        input_tokens_per_second: Some(200.0),
        total_tokens_per_second: Some(300.0),
        latency_s: d(),
        coordinated_omission_latency_s: d(),
        ttft_s: d(),
        tpot_s: d(),
        itl_s: d(),
        connect_s: d(),
        prefill_s: d(),
        decode_s: d(),
        decode_tok_s: d(),
        first_byte_s: d(),
        queue_delay_s: d(),
        first_reasoning_s: d(),
        dns_s: d(),
        receive_s: d(),
        bytes_sent: d(),
        bytes_received: d(),
        chunks_received: d(),
        connections_reused: Some(6),
        connection_reuse_rate: Some(6.0 / 7.0),
        prefill_tps_per_user: d(),
        time_to_second_token_s: d(),
        user_tps: d(),
        isl_tokens: d(),
        isl_tokens_source: Some("server_usage"),
        osl_tokens: d(),
        osl_tokens_source: Some("server_usage"),
        reasoning_tokens: d(),
        reasoning_tokens_total: Some(50),
        visible_completion_tokens: d(),
        visible_completion_tokens_total: Some(150),
        throughput_bins_rps: d(),
        goodput: GoodputSummary {
            count: 6,
            requests_per_second: 3.0,
            fraction_of_attempted: 0.75,
            thresholds_s: summary_thresholds(),
        },
        pooled_mixture: false,
        per_endpoint: BTreeMap::from([("ep-0".to_string(), endpoint_summary(dist_full))]),
        environment: json!({}),
        sut: None,
        price_per_hour: Some(2.0),
        price_provenance: Some("cli"),
        cost_per_million_output_tokens: Some(5.5),
        observed_concurrency: Some(observed_concurrency()),
        time_weighted: tw_metrics(tw_full),
        isl_osl: Some(isl_osl()),
        ttft_approx_count: 1,
        ttft_warning: Some("approximated".into()),
        telemetry: Some(telemetry_info()),
        partial: false,
        config: None,
    }
}

fn summary_min() -> RunSummary {
    let d = dist_empty;
    RunSummary {
        schema_version: metrum_ai_bench::record::SCHEMA_VERSION_SUMMARY,
        attempted: 0,
        successes: 0,
        errors: 0,
        error_rate: 0.0,
        errors_by_type: BTreeMap::new(),
        window_seconds: 0.0,
        requests_per_second: 0.0,
        usage_missing_count: 0,
        completion_tokens_per_second: None,
        completion_tokens_source: None,
        prompt_tokens_total: None,
        completion_tokens_total: None,
        input_tokens_per_second: None,
        total_tokens_per_second: None,
        latency_s: d(),
        coordinated_omission_latency_s: d(),
        ttft_s: d(),
        tpot_s: d(),
        itl_s: d(),
        connect_s: d(),
        prefill_s: d(),
        decode_s: d(),
        decode_tok_s: d(),
        first_byte_s: d(),
        queue_delay_s: d(),
        first_reasoning_s: d(),
        dns_s: d(),
        receive_s: d(),
        bytes_sent: d(),
        bytes_received: d(),
        chunks_received: d(),
        connections_reused: None,
        connection_reuse_rate: None,
        prefill_tps_per_user: d(),
        time_to_second_token_s: d(),
        user_tps: d(),
        isl_tokens: d(),
        isl_tokens_source: None,
        osl_tokens: d(),
        osl_tokens_source: None,
        reasoning_tokens: d(),
        reasoning_tokens_total: None,
        visible_completion_tokens: d(),
        visible_completion_tokens_total: None,
        throughput_bins_rps: d(),
        goodput: GoodputSummary {
            count: 0,
            requests_per_second: 0.0,
            fraction_of_attempted: 0.0,
            thresholds_s: BTreeMap::new(),
        },
        pooled_mixture: false,
        per_endpoint: BTreeMap::new(),
        environment: json!({}),
        sut: None,
        price_per_hour: None,
        price_provenance: None,
        cost_per_million_output_tokens: None,
        observed_concurrency: None,
        time_weighted: TimeWeightedMetrics::default(),
        isl_osl: None,
        ttft_approx_count: 0,
        ttft_warning: None,
        telemetry: None,
        partial: false,
        config: None,
    }
}

fn request_max() -> RequestRecord {
    let now = Utc::now();
    RequestRecord {
        schema_version: SCHEMA_VERSION_REQUEST,
        run_id: Some("run".into()),
        seq: 4,
        phase: Phase::Measure,
        endpoint: "ep-0".into(),
        started_at: now,
        completed_at: now,
        send_offset_s: Some(0.5),
        scheduled_offset_s: Some(0.5),
        queue_delay_s: 0.0,
        latency_s: 0.4,
        first_byte_s: Some(0.05),
        connect_s: Some(0.0),
        connection_reused: Some(true),
        dns_s: Some(0.0),
        bytes_sent: Some(200),
        receive_s: Some(0.3),
        bytes_received: Some(4000),
        chunks_received: Some(40),
        ttft_s: Some(0.1),
        ttft_source: Some(TtftSource::Stream),
        prefill_s: Some(0.1),
        decode_s: Some(0.3),
        decode_tok_s: Some(100.0),
        first_reasoning_s: Some(0.08),
        itl_s: vec![0.01, 0.01],
        in_flight_at_send: Some(4),
        prompt_tokens: 50,
        completion_tokens: 31,
        total_tokens: 81,
        reasoning_tokens: Some(10),
        visible_completion_tokens: Some(21),
        tokenized_prompt_tokens: Some(50),
        tokenized_completion_tokens: Some(31),
        usage_missing: false,
        modality_metrics: BTreeMap::from([("prompt_words".to_string(), 40.0)]),
        modality_labels: BTreeMap::from([("image_0_sha256".to_string(), "00".to_string())]),
        error: Some(RequestError::Timeout),
        partial: false,
    }
}

fn request_min() -> RequestRecord {
    let now = Utc::now();
    RequestRecord {
        schema_version: SCHEMA_VERSION_REQUEST,
        run_id: None,
        seq: 4,
        phase: Phase::Measure,
        endpoint: "ep-0".into(),
        started_at: now,
        completed_at: now,
        send_offset_s: None,
        scheduled_offset_s: None,
        queue_delay_s: 0.0,
        latency_s: 0.4,
        first_byte_s: None,
        connect_s: None,
        connection_reused: None,
        dns_s: None,
        bytes_sent: None,
        receive_s: None,
        bytes_received: None,
        chunks_received: None,
        ttft_s: None,
        ttft_source: None,
        prefill_s: None,
        decode_s: None,
        decode_tok_s: None,
        first_reasoning_s: None,
        itl_s: Vec::new(),
        in_flight_at_send: None,
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        reasoning_tokens: None,
        visible_completion_tokens: None,
        tokenized_prompt_tokens: None,
        tokenized_completion_tokens: None,
        usage_missing: false,
        modality_metrics: BTreeMap::new(),
        modality_labels: BTreeMap::new(),
        error: None,
        partial: false,
    }
}

fn sweep_max() -> SweepPoint {
    let d = dist_full;
    let thresholds = metrum_ai_bench::strategic::summarize_stage(1.0, &[], 1.0, &all_slos(), None)
        .slo_thresholds_s;
    SweepPoint {
        load: 4.0,
        n: 8,
        errors: 0,
        throughput: 3.5,
        latency_s: d(),
        p50_s: Some(0.4),
        p95_s: Some(0.5),
        p99_s: Some(0.6),
        p99_unreliable: true,
        error_rate: Some(0.0),
        validity_rate: Some(1.0),
        goodput: 3.0,
        goodput_equals_throughput: false,
        slo_thresholds_s: thresholds,
        user_tps: d(),
        users_at_slo: Some(3.0),
        users_meeting_user_tps: Some(6),
        completion_tokens_per_second: Some(100.0),
        prompt_tokens_total: Some(400),
        completion_tokens_total: Some(200),
        input_tokens_per_second: Some(200.0),
        total_tokens_per_second: Some(300.0),
        prefill_tps_per_user: d(),
        time_to_second_token_s: d(),
        cost_per_million_output_tokens: Some(5.5),
        observed_concurrency: Some(observed_concurrency()),
        time_weighted: tw_metrics(tw_full),
        connect_s: d(),
        prefill_s: d(),
        decode_s: d(),
        decode_tok_s: d(),
        ttft_s: d(),
        first_byte_s: d(),
        queue_delay_s: d(),
        first_reasoning_s: d(),
        dns_s: d(),
        receive_s: d(),
        bytes_sent: d(),
        bytes_received: d(),
        chunks_received: d(),
        connections_reused: Some(6),
        connection_reuse_rate: Some(0.75),
        isl_tokens: d(),
        osl_tokens: d(),
        reasoning_tokens: d(),
        reasoning_tokens_total: Some(50),
        visible_completion_tokens: d(),
        visible_completion_tokens_total: Some(150),
        ttft_approx_count: 1,
        ttft_warning: Some("approximated".into()),
        isl_osl: Some(isl_osl()),
        modality_metrics: modality_kinds()
            .iter()
            .flat_map(|(keys, _)| keys.iter())
            .map(|key| (key.to_string(), dist_full()))
            .collect(),
        image_digests: Some(ImageDigests {
            images: 4,
            distinct: 3,
        }),
        config: None,
    }
}

fn sweep_min() -> SweepPoint {
    let d = dist_empty;
    SweepPoint {
        load: 4.0,
        n: 0,
        errors: 0,
        throughput: 0.0,
        latency_s: d(),
        p50_s: None,
        p95_s: None,
        p99_s: None,
        p99_unreliable: true,
        error_rate: None,
        validity_rate: None,
        goodput: 0.0,
        goodput_equals_throughput: true,
        slo_thresholds_s: None,
        user_tps: d(),
        users_at_slo: None,
        users_meeting_user_tps: None,
        completion_tokens_per_second: None,
        prompt_tokens_total: None,
        completion_tokens_total: None,
        input_tokens_per_second: None,
        total_tokens_per_second: None,
        prefill_tps_per_user: d(),
        time_to_second_token_s: d(),
        cost_per_million_output_tokens: None,
        observed_concurrency: None,
        time_weighted: TimeWeightedMetrics::default(),
        connect_s: d(),
        prefill_s: d(),
        decode_s: d(),
        decode_tok_s: d(),
        ttft_s: d(),
        first_byte_s: d(),
        queue_delay_s: d(),
        first_reasoning_s: d(),
        dns_s: d(),
        receive_s: d(),
        bytes_sent: d(),
        bytes_received: d(),
        chunks_received: d(),
        connections_reused: None,
        connection_reuse_rate: None,
        isl_tokens: d(),
        osl_tokens: d(),
        reasoning_tokens: d(),
        reasoning_tokens_total: None,
        visible_completion_tokens: d(),
        visible_completion_tokens_total: None,
        ttft_approx_count: 0,
        ttft_warning: None,
        isl_osl: None,
        modality_metrics: BTreeMap::new(),
        image_digests: None,
        config: None,
    }
}

fn request_row_max() -> RequestRow {
    RequestRow {
        run_id: "run".into(),
        seq: 4,
        stage: 4.0,
        warmup: false,
        t_sched_ns: 1,
        t_sent_ns: 2,
        t_first_ns: Some(3),
        t_done_ns: 4,
        success: true,
        input_tokens: 50,
        output_tokens: 31,
        reasoning_tokens: Some(10),
        latency_s: 0.4,
        queue_delay_s: 0.0,
        service_latency_s: 0.4,
        ttft_s: Some(0.1),
        ttft_source: Some("stream".into()),
        error: Some("timeout".into()),
        telemetry_at_done: Some(BTreeMap::from([("gpu_util".to_string(), 50.0)])),
    }
}

fn request_row_min() -> RequestRow {
    RequestRow {
        run_id: "run".into(),
        seq: 4,
        stage: 4.0,
        warmup: false,
        t_sched_ns: 1,
        t_sent_ns: 2,
        t_first_ns: None,
        t_done_ns: 4,
        success: false,
        input_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: None,
        latency_s: 0.4,
        queue_delay_s: 0.0,
        service_latency_s: 0.4,
        ttft_s: None,
        ttft_source: None,
        error: None,
        telemetry_at_done: None,
    }
}

fn to_json<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("serialize schema fixture")
}

// Counting (the rules of scripts/parity/count_points.py)

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Distribution,
    Block,
    Scalar,
}

impl Shape {
    fn label(self) -> &'static str {
        match self {
            Self::Distribution => "distribution",
            Self::Block => "block",
            Self::Scalar => "scalar",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Quantity {
    shape: Shape,
    values: usize,
}

fn is_num(value: &Value) -> bool {
    value.is_number()
}

fn numeric_leaves(value: &Value) -> usize {
    match value {
        Value::Number(_) => 1,
        Value::Object(map) => map.values().map(numeric_leaves).sum(),
        Value::Array(items) => items.iter().map(numeric_leaves).sum(),
        _ => 0,
    }
}

fn is_dist(map: &serde_json::Map<String, Value>) -> bool {
    ["n", "p50", "p99"].iter().all(|k| map.contains_key(*k))
}

/// Record every distribution under `node` by dotted path; return the count
/// of numeric leaves outside any distribution.
fn split_dists(node: &Value, path: &str, found: &mut BTreeMap<String, Quantity>) -> usize {
    match node {
        Value::Number(_) => 1,
        Value::Object(map) if is_dist(map) => {
            found.insert(
                path.to_string(),
                Quantity {
                    shape: Shape::Distribution,
                    values: numeric_leaves(node),
                },
            );
            0
        }
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| split_dists(v, &format!("{path}.{k}"), found))
            .sum(),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(i, v)| split_dists(v, &format!("{path}[{i}]"), found))
            .sum(),
        _ => 0,
    }
}

/// Quantities of one summary-shaped object: every distribution at any depth,
/// each top-level numeric scalar, and each top-level block with numeric
/// leaves outside its distributions. Nulls never count.
fn quantities(summary: &Value, skip: &[&str]) -> BTreeMap<String, Quantity> {
    let mut out = BTreeMap::new();
    for (key, value) in summary.as_object().expect("summary object") {
        if skip.contains(&key.as_str()) {
            continue;
        }
        if is_num(value) {
            out.insert(
                key.clone(),
                Quantity {
                    shape: Shape::Scalar,
                    values: 1,
                },
            );
        } else if value.is_object() {
            let leftover = split_dists(value, key, &mut out);
            if leftover > 0 {
                out.insert(
                    key.clone(),
                    Quantity {
                        shape: Shape::Block,
                        values: leftover,
                    },
                );
            }
        }
    }
    out
}

/// Distinct non-null numeric fields of one record, dotted for nested maps; a
/// list of numbers counts once.
fn fields(row: &Value, skip: &[&str]) -> BTreeSet<String> {
    fn walk(value: &Value, prefix: &str, top: bool, skip: &[&str], out: &mut BTreeSet<String>) {
        for (key, v) in value.as_object().expect("record object") {
            if top && skip.contains(&key.as_str()) {
                continue;
            }
            let name = format!("{prefix}{key}");
            match v {
                Value::Number(_) => {
                    out.insert(name);
                }
                Value::Array(items) if !items.is_empty() && items.iter().all(is_num) => {
                    out.insert(name);
                }
                Value::Object(_) => walk(v, &format!("{name}."), false, skip, out),
                _ => {}
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(row, "", true, skip, &mut out);
    out
}

fn without_prefixes(set: BTreeSet<String>, prefixes: &[&str]) -> BTreeSet<String> {
    set.into_iter()
        .filter(|f| !prefixes.iter().any(|p| f.starts_with(&format!("{p}."))))
        .collect()
}

// Schema counts

struct QuantitySchema {
    all: BTreeMap<String, Quantity>,
    optional: BTreeSet<String>,
}

impl QuantitySchema {
    fn derive(max: &Value, min: &Value, skip: &[&str]) -> Self {
        let all = quantities(max, skip);
        let always = quantities(min, skip);
        let optional = all
            .keys()
            .filter(|k| !always.contains_key(*k))
            .cloned()
            .collect();
        Self { all, optional }
    }

    fn count(&self, shape: Shape) -> usize {
        self.all.values().filter(|q| q.shape == shape).count()
    }

    fn values(&self) -> usize {
        self.all.values().map(|q| q.values).sum()
    }

    /// Quantities a run meeting `active` fires.
    fn expected(&self, table: &[Row], active: &[Condition]) -> BTreeSet<String> {
        self.all
            .keys()
            .filter(|k| !self.optional.contains(*k) || fires(table, k, active))
            .cloned()
            .collect()
    }
}

struct FieldSchema {
    all: BTreeSet<String>,
    optional: BTreeSet<String>,
}

impl FieldSchema {
    fn derive(max: &Value, min: &Value, skip: &[&str], open: &[&str]) -> Self {
        let all = without_prefixes(fields(max, skip), open);
        let always = without_prefixes(fields(min, skip), open);
        let optional = all.difference(&always).cloned().collect();
        Self { all, optional }
    }

    fn expected(&self, table: &[Row], active: &[Condition]) -> BTreeSet<String> {
        self.all
            .iter()
            .filter(|k| !self.optional.contains(*k) || fires(table, k, active))
            .cloned()
            .collect()
    }
}

fn conditions_of(table: &[Row], key: &str) -> &'static [Condition] {
    table
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, conditions)| *conditions)
        .unwrap_or_else(|| panic!("optional field {key} has no documented condition"))
}

fn fires(table: &[Row], key: &str, active: &[Condition]) -> bool {
    conditions_of(table, key)
        .iter()
        .any(|condition| active.contains(condition))
}

struct Schemas {
    summary: QuantitySchema,
    per_endpoint: QuantitySchema,
    request: FieldSchema,
    sweep: QuantitySchema,
    request_row: FieldSchema,
    dist_width: Vec<String>,
    time_weighted_width: usize,
    thresholds: usize,
}

fn schemas() -> Schemas {
    let skip_summary: Vec<&str> = SUMMARY_EXCLUDED
        .iter()
        .chain(SUMMARY_SEPARATE)
        .copied()
        .collect();
    let summary_max = to_json(&summary_max());
    let endpoint = to_json(&endpoint_summary(dist_full));
    let endpoint_min = to_json(&endpoint_summary(dist_empty));
    let dist = to_json(&dist_full());
    let dist_width = dist
        .as_object()
        .expect("dist")
        .iter()
        .filter(|(_, v)| is_num(v))
        .map(|(k, _)| k.clone())
        .collect();
    Schemas {
        summary: QuantitySchema::derive(&summary_max, &to_json(&summary_min()), &skip_summary),
        per_endpoint: QuantitySchema::derive(&endpoint, &endpoint_min, &[]),
        request: FieldSchema::derive(
            &to_json(&request_max()),
            &to_json(&request_min()),
            REQUEST_SKIP,
            REQUEST_OPEN_MAPS,
        ),
        sweep: QuantitySchema::derive(
            &to_json(&sweep_max()),
            &to_json(&sweep_min()),
            SWEEP_EXCLUDED,
        ),
        request_row: FieldSchema::derive(
            &to_json(&request_row_max()),
            &to_json(&request_row_min()),
            REQUEST_ROW_SKIP,
            &[],
        ),
        dist_width,
        time_weighted_width: numeric_leaves(&to_json(&tw_full())),
        thresholds: summary_max["goodput"]["thresholds_s"]
            .as_object()
            .map_or(0, |m| m.len()),
    }
}

// Rendering

fn condition_cell(table: &[Row], key: &str, optional: bool) -> String {
    if !optional {
        return "always".into();
    }
    conditions_of(table, key)
        .iter()
        .map(|c| format!("`{}`", c.name()))
        .collect::<Vec<_>>()
        .join(" or ")
}

fn quantity_table(out: &mut String, schema: &QuantitySchema, table: &[Row]) {
    out.push_str("| Quantity | Shape | Values | Fires |\n|---|---|---|---|\n");
    for (name, q) in &schema.all {
        let _ = writeln!(
            out,
            "| `{name}` | {} | {} | {} |",
            q.shape.label(),
            q.values,
            condition_cell(table, name, schema.optional.contains(name))
        );
    }
}

fn field_table(out: &mut String, schema: &FieldSchema, table: &[Row]) {
    out.push_str("| Field | Fires |\n|---|---|\n");
    for name in &schema.all {
        let _ = writeln!(
            out,
            "| `{name}` | {} |",
            condition_cell(table, name, schema.optional.contains(name))
        );
    }
}

fn render(s: &Schemas) -> String {
    let mut out = String::new();
    let plain_quantities = s.summary.expected(&summary_optional(), PLAIN_RUN).len();
    let plain_fields =
        s.request.expected(&request_optional(), PLAIN_RUN).len() + LLM_MODALITY_KEYS.len();
    let dist_names = s
        .dist_width
        .iter()
        .map(|k| format!("`{k}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let _ = write!(
        out,
        r#"<!-- Copyright (c) 2026 Metrum AI, Inc. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Data points

<!-- Generated by scripts/render_data_points.sh from tests/data_points.rs. Do not edit by hand. -->

How many data points Metrum AI Bench reports, derived from the serde schemas
in this release (`summary.v3`, `request.v3`, the strategic sweep point, and
the `telemetry.v1` NDJSON `request` row). `tests/data_points.rs` builds every
schema struct with every optional field set and with every optional field
unset, serializes both, and counts the JSON. CI fails when this file is stale,
and the release workflow publishes it in the release notes.

## Headline counts

| Output | Quantities | Distributions | Blocks | Scalars | Values | Always | Optional |
|---|---|---|---|---|---|---|---|
| `summary.v3` run summary | {sq} | {sd} | {sb} | {ss} | {sv} | {sa} | {so} |
| `summary.v3` `per_endpoint`, each endpoint | {eq} | {ed} | {eb} | {es} | {ev} | {ea} | {eo} |
| Strategic sweep point (`points[]`) | {pq} | {pd} | {pb} | {ps} | {pv} | {pa} | {po} |

| Per-request output | Numeric fields | Always | Optional |
|---|---|---|---|
| `request.v3` (`--data-log`), schema fields | {rq} | {ra} | {ro} |
| `telemetry.v1` `request` row (`--ndjson`) | {wq} | {wa} | {wo} |

- **Telemetry series** are selected by the YAML `include` patterns of each
  `--telemetry` source at run time. There is no fixed series count: the set is
  whatever the scraped pages expose that the patterns match. Each sample is
  one `telemetry` row; `telemetry_at_done` on `request` rows is a last-seen
  map over the same series.
- **`request.v3` `modality_metrics`** is an open map that each modality binary
  fills, so it is not in the schema count. `metrum-ai-bench-cli-llm` writes
  {n_llm} keys on every success: {llm_keys}. See `docs/OUTPUT_SCHEMA.md` for
  vlm, asr, and imagegen.
- **Strategic modality keys:** the sweep-point row counts the
  `modality_metrics` distributions of every `--kind` ({modality_split}). One
  sweep carries only its own kind's keys, plus `image_digests` for imagegen;
  chat, embeddings, and rerank sweeps carry none.
- **Counted separately:** `per_endpoint` (one block per endpoint, above) and
  `errors_by_type` (one count per observed error type). Never counted: run
  metadata (`config`, `environment`, `sut`, `schema_version`), strings,
  booleans, and nulls.

## Widths

- A distribution (`DistSummary`) carries {dw} numbers:
  {dist_names}.
  Its `percentile_method` and `p90_unreliable` / `p95_unreliable` /
  `p99_unreliable` flags are not counted.
- A time-weighted block (`effective_*`, `tokens_in_flight`) carries
  {tw} numbers: `n`, `avg`, `active_avg`, `max`, `active_s`.
- `goodput.thresholds_s` holds up to {th} thresholds (`ttft`, `tpot`, `e2e`,
  `user_tps`), one per `--slo`.

## Counting rules

These are the rules of `scripts/parity/count_points.py`, so schema counts
and harness counts agree on what a quantity is.

- **Quantity:** each distribution at any depth (named by its dotted path),
  each top-level numeric scalar, and each
  top-level block whose numeric leaves outside its distributions are
  non-empty (`goodput`, `observed_concurrency`, `effective_concurrency`).
- **Values:** numeric slots in those quantities, all set. A run fills fewer:
  a distribution with `n = 0` carries only `n` (the rest are `null`), and an
  optional field that does not fire carries nothing.
- **Per-request field:** a numeric field of a record, dotted for nested maps.
  A list of numbers (`itl_s`) counts once. `seq` and `error` never count.
  Counts describe a successful request; a failed one carries the always set
  plus the request-side `http` fields.
- **Optional:** in the maximal record but absent or `null` in the minimal
  one. Each optional field fires when any condition in its row holds.

## Conditions

| Condition | Meaning |
|---|---|
"#,
        sq = s.summary.all.len(),
        sd = s.summary.count(Shape::Distribution),
        sb = s.summary.count(Shape::Block),
        ss = s.summary.count(Shape::Scalar),
        sv = s.summary.values(),
        sa = s.summary.all.len() - s.summary.optional.len(),
        so = s.summary.optional.len(),
        eq = s.per_endpoint.all.len(),
        ed = s.per_endpoint.count(Shape::Distribution),
        eb = s.per_endpoint.count(Shape::Block),
        es = s.per_endpoint.count(Shape::Scalar),
        ev = s.per_endpoint.values(),
        ea = s.per_endpoint.all.len() - s.per_endpoint.optional.len(),
        eo = s.per_endpoint.optional.len(),
        pq = s.sweep.all.len(),
        pd = s.sweep.count(Shape::Distribution),
        pb = s.sweep.count(Shape::Block),
        ps = s.sweep.count(Shape::Scalar),
        pv = s.sweep.values(),
        pa = s.sweep.all.len() - s.sweep.optional.len(),
        po = s.sweep.optional.len(),
        rq = s.request.all.len(),
        ra = s.request.all.len() - s.request.optional.len(),
        ro = s.request.optional.len(),
        wq = s.request_row.all.len(),
        wa = s.request_row.all.len() - s.request_row.optional.len(),
        wo = s.request_row.optional.len(),
        llm_keys = LLM_MODALITY_KEYS
            .iter()
            .map(|k| format!("`{k}`"))
            .collect::<Vec<_>>()
            .join(" and "),
        n_llm = LLM_MODALITY_KEYS.len(),
        modality_split = modality_kinds()
            .iter()
            .map(|(keys, kind)| format!(
                "{} {}",
                kind[0].name().trim_end_matches("-sweep"),
                keys.len()
            ))
            .collect::<Vec<_>>()
            .join(", "),
        dw = s.dist_width.len(),
        tw = s.time_weighted_width,
        th = s.thresholds,
    );
    let mut used: BTreeSet<Condition> = BTreeSet::new();
    for table in [
        summary_optional(),
        request_optional(),
        sweep_optional(),
        request_row_optional(),
    ] {
        used.extend(table.iter().flat_map(|(_, c)| c.iter().copied()));
    }
    for condition in used {
        let _ = writeln!(out, "| `{}` | {} |", condition.name(), condition.describe());
    }
    let plain = PLAIN_RUN
        .iter()
        .map(|c| format!("`{}`", c.name()))
        .collect::<Vec<_>>()
        .join(", ");
    let _ = write!(
        out,
        r#"
## Plain llm run

A closed-loop `metrum-ai-bench-cli-llm --mode chat --streaming` run against a
server that reports usage, with no reasoning, SLO, price, ISL/OSL targets,
tokenizer, or `--ndjson`, meets {plain}. It fires **{plain_quantities}**
summary quantities and **{plain_fields}** per-request fields (schema fields
plus the llm `modality_metrics` keys).
`tests/data_points.rs` runs exactly that against dummy-model-server and
asserts both numbers, and that every fired field is in the schema count.
They match the `plain` row of the parity harness (`scripts/parity/`), which
counts quantities and per-request fields with the same rules. The harness
`values` column counts every slot of each distribution a run reports, null
or not (#245), but outside distributions only the numbers one run produced.
It can therefore be lower than the schema `values` here, which counts every
numeric slot.

## `summary.v3` quantities

"#
    );
    quantity_table(&mut out, &s.summary, &summary_optional());
    out.push_str("\n## `summary.v3` `per_endpoint` quantities (each endpoint)\n\n");
    quantity_table(&mut out, &s.per_endpoint, &[]);
    out.push_str("\n## Strategic sweep-point quantities\n\n");
    quantity_table(&mut out, &s.sweep, &sweep_optional());
    out.push_str("\n## `request.v3` fields\n\n");
    field_table(&mut out, &s.request, &request_optional());
    out.push_str("\n## `telemetry.v1` `request` row fields (`--ndjson`)\n\n");
    field_table(&mut out, &s.request_row, &request_row_optional());
    out
}

fn doc_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(DOC)
}

// Tests

/// One `pub` field of a schema struct, read from its source.
struct DeclaredField {
    name: String,
    ty: String,
    flatten: bool,
}

fn source(path: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// `pub` fields declared on `pub struct NAME` in `path`, in order. A field
/// whose layout this parser does not expect makes the coverage test fail
/// loudly (a name mismatch), never pass silently.
fn declared_fields(path: &str, name: &str) -> Vec<DeclaredField> {
    let src = source(path);
    let start = src
        .find(&format!("pub struct {name} {{"))
        .unwrap_or_else(|| panic!("pub struct {name} not found in {path}"));
    let body = &src[start..];
    let end = body.find("\n}").expect("struct end");
    let mut fields = Vec::new();
    let mut flatten = false;
    for line in body[..end].lines().skip(1) {
        let trimmed = line.trim();
        if trimmed.starts_with("#[serde(") && trimmed.contains("flatten") {
            flatten = true;
        }
        if let Some(rest) = line.strip_prefix("    pub ") {
            let (field, ty) = rest.split_once(':').expect("pub field: type");
            fields.push(DeclaredField {
                name: field.trim().to_string(),
                ty: ty.trim().trim_end_matches(',').to_string(),
                flatten,
            });
            flatten = false;
        }
    }
    fields
}

/// Serialized key names of `name`: flattened fields expand to their own.
fn serialized_names(path: &str, name: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for field in declared_fields(path, name) {
        if field.flatten {
            let ty = field.ty.rsplit("::").next().unwrap_or(&field.ty);
            assert_eq!(
                ty, "TimeWeightedMetrics",
                "{name}.{}: teach the coverage test this flattened type",
                field.name
            );
            out.extend(serialized_names("src/time_weighted.rs", ty));
        } else {
            out.insert(field.name);
        }
    }
    out
}

/// Every schema struct and nested block the counts walk, with its maximal
/// fixture and its uncounted keys: run metadata and error data that the
/// fixture may leave unset or `null`, and that are not type-checked.
fn maximal_cases() -> Vec<(&'static str, &'static str, Value, &'static [&'static str])> {
    let summary = to_json(&summary_max());
    vec![
        (
            "src/summary.rs",
            "RunSummary",
            summary.clone(),
            &["config", "sut", "environment"],
        ),
        (
            "src/record.rs",
            "RequestRecord",
            to_json(&request_max()),
            &["error"],
        ),
        (
            "src/strategic.rs",
            "SweepPoint",
            to_json(&sweep_max()),
            &["config"],
        ),
        (
            "src/telemetry/row.rs",
            "RequestRow",
            to_json(&request_row_max()),
            &["error"],
        ),
        ("src/stats.rs", "DistSummary", to_json(&dist_full()), &[]),
        (
            "src/time_weighted.rs",
            "TimeWeightedStat",
            to_json(&tw_full()),
            &[],
        ),
        (
            "src/summary.rs",
            "GoodputSummary",
            summary["goodput"].clone(),
            &[],
        ),
        (
            "src/summary.rs",
            "EndpointSummary",
            to_json(&endpoint_summary(dist_full)),
            &[],
        ),
        (
            "src/concurrency.rs",
            "ObservedConcurrency",
            to_json(&observed_concurrency()),
            &[],
        ),
        (
            "src/isl_osl.rs",
            "IslOslValidation",
            to_json(&isl_osl()),
            &[],
        ),
        (
            "src/telemetry/session.rs",
            "TelemetryRunInfo",
            to_json(&telemetry_info()),
            &[],
        ),
        (
            "src/sweep_modality.rs",
            "ImageDigests",
            to_json(&ImageDigests {
                images: 4,
                distinct: 3,
            }),
            &[],
        ),
    ]
}

/// Map value types that make a field an open map of scalars, not a block.
const SCALAR_TYPES: &[&str] = &["f64", "u64", "usize", "String"];

/// The struct a field serializes as when it is an object: `Option<T>` and
/// `BTreeMap<String, T>` unwrap to `T`. `None` for a map of scalars.
fn object_type(ty: &str) -> Option<&str> {
    let mut ty = ty.trim();
    if let Some(inner) = ty.strip_prefix("Option<") {
        ty = inner.strip_suffix('>').expect("Option<..>");
    }
    if ty.contains("BTreeMap<") {
        let (_, value) = ty.rsplit_once(", ").expect("BTreeMap<K, V>");
        ty = value.strip_suffix('>').expect("BTreeMap<..>");
    }
    let base = ty.rsplit("::").next().unwrap_or(ty);
    (!SCALAR_TYPES.contains(&base)).then_some(base)
}

/// Path of the first `null` anywhere under `value`, skipping open maps.
fn first_null(value: &Value, path: &str) -> Option<String> {
    match value {
        Value::Null => Some(path.to_string()),
        Value::Object(map) => map
            .iter()
            .filter(|(k, _)| !REQUEST_OPEN_MAPS.contains(&k.as_str()) && *k != "telemetry_at_done")
            .find_map(|(k, v)| first_null(v, &format!("{path}.{k}"))),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(i, v)| first_null(v, &format!("{path}[{i}]"))),
        _ => None,
    }
}

#[test]
fn schema_fixtures_cover_every_field() {
    let cases = maximal_cases();
    let mut registered: BTreeSet<&str> = cases.iter().map(|(_, name, _, _)| *name).collect();
    registered.insert("TimeWeightedMetrics"); // flattened; its blocks are registered
    for (path, name, json, uncounted) in &cases {
        let keys: BTreeSet<String> = json.as_object().expect("object").keys().cloned().collect();
        let expected: BTreeSet<String> = serialized_names(path, name)
            .into_iter()
            .filter(|k| !uncounted.contains(&k.as_str()) || json.get(k).is_some())
            .collect();
        assert_eq!(
            keys, expected,
            "{name}: the maximal fixture must serialize every field; set each Option to Some"
        );
        for (key, value) in json.as_object().unwrap() {
            if uncounted.contains(&key.as_str()) {
                continue;
            }
            if let Some(null) = first_null(value, &format!("{name}.{key}")) {
                panic!("{null} is null in the maximal fixture");
            }
        }
        // A nested block must have its own maximal case, or its fields
        // would be counted without a coverage check.
        for field in declared_fields(path, name) {
            let is_object = json.get(&field.name).is_some_and(Value::is_object);
            if field.flatten || !is_object || uncounted.contains(&field.name.as_str()) {
                continue;
            }
            if let Some(ty) = object_type(&field.ty) {
                assert!(
                    registered.contains(ty),
                    "{name}.{}: register {ty} in maximal_cases()",
                    field.name
                );
            }
        }
    }
}

/// A minimal fixture that sets an `Option` would publish an optional field
/// as "always": every `Option` field must be absent or `null` there.
#[test]
fn minimal_fixtures_leave_every_option_unset() {
    let cases: [(&str, &str, Value); 6] = [
        ("src/summary.rs", "RunSummary", to_json(&summary_min())),
        ("src/record.rs", "RequestRecord", to_json(&request_min())),
        ("src/strategic.rs", "SweepPoint", to_json(&sweep_min())),
        (
            "src/telemetry/row.rs",
            "RequestRow",
            to_json(&request_row_min()),
        ),
        ("src/stats.rs", "DistSummary", to_json(&dist_empty())),
        (
            "src/time_weighted.rs",
            "TimeWeightedStat",
            to_json(&TimeWeightedStat::default()),
        ),
    ];
    for (path, name, json) in cases {
        for field in declared_fields(path, name) {
            if field.ty.starts_with("Option<") {
                assert!(
                    json.get(&field.name).is_none_or(Value::is_null),
                    "{name}.{} is set in the minimal fixture",
                    field.name
                );
            }
        }
    }
}

#[test]
fn every_optional_field_has_a_condition_and_no_stale_rows() {
    let s = schemas();
    let check = |label: &str, optional: &BTreeSet<String>, table: &[Row]| {
        let documented: BTreeSet<String> = table.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(
            optional, &documented,
            "{label}: optional set derived from the schema != documented conditions"
        );
        assert!(table.iter().all(|(_, c)| !c.is_empty()), "{label}");
    };
    check("summary.v3", &s.summary.optional, &summary_optional());
    check("request.v3", &s.request.optional, &request_optional());
    check("sweep point", &s.sweep.optional, &sweep_optional());
    check(
        "telemetry request row",
        &s.request_row.optional,
        &request_row_optional(),
    );
    assert!(s.per_endpoint.optional.is_empty());
}

#[test]
fn widths_match_the_parity_harness() {
    let s = schemas();
    assert_eq!(s.dist_width.len(), 10, "{:?}", s.dist_width);
    assert_eq!(s.time_weighted_width, 5);
    assert_eq!(s.thresholds, 4);
}

#[test]
fn data_points_doc_is_current() {
    let rendered = render(&schemas());
    if std::env::var(BLESS_ENV).is_ok_and(|v| v == "1") {
        std::fs::write(doc_path(), &rendered).expect("write docs/DATA_POINTS.md");
        return;
    }
    let committed = std::fs::read_to_string(doc_path()).unwrap_or_default();
    assert!(
        committed == rendered,
        "{DOC} is stale; run scripts/render_data_points.sh and commit it"
    );
}

#[test]
fn rendered_doc_has_no_em_dashes() {
    assert!(!render(&schemas()).contains('\u{2014}'));
}

// Real runs against dummy-model-server

fn run_llm(url: &str, dir: &Path, name: &str, extra: &[&str]) -> PathBuf {
    let prompts = dir.join("prompts.jsonl");
    std::fs::write(
        &prompts,
        "{\"prompt\":\"Count the primes below fifty.\"}\n{\"prompt\":\"Name three rivers.\"}\n",
    )
    .expect("prompts");
    let data_log = dir.join(format!("{name}.jsonl"));
    let status = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"))
        .args([
            "--url",
            url,
            "--api-key",
            "dummy",
            "--scenario",
            "data-points",
            "--num-requests",
            "6",
            "--warmup-requests",
            "2",
            "--concurrency",
            "2",
            "--prompts",
            prompts.to_str().expect("utf8"),
            "--mode",
            "chat",
            "--model",
            "dummy",
            "--max-tokens",
            "8",
            "--streaming",
            "--data-log",
            data_log.to_str().expect("utf8"),
            "--debug-log",
            dir.join(format!("{name}-debug.log"))
                .to_str()
                .expect("utf8"),
            "--error-log",
            dir.join(format!("{name}-error.log"))
                .to_str()
                .expect("utf8"),
            "--log-level",
            "error",
        ])
        .args(extra)
        .status()
        .expect("run llm");
    assert!(status.success(), "{name}: llm bench failed");
    data_log
}

/// Fired summary quantities and per-request fields of one run.
fn fired(data_log: &Path) -> (BTreeSet<String>, BTreeSet<String>) {
    let skip: Vec<&str> = SUMMARY_EXCLUDED
        .iter()
        .chain(SUMMARY_SEPARATE)
        .copied()
        .collect();
    let summary = common::summary_record(data_log).expect("summary");
    let quantities = quantities(&summary, &skip).into_keys().collect();
    let mut per_request = BTreeSet::new();
    for row in common::request_records(data_log)
        .iter()
        .filter(|r| r["phase"] == "measure")
    {
        assert!(row.get("error").is_none(), "{row}");
        per_request.extend(fields(row, REQUEST_SKIP));
    }
    (quantities, per_request)
}

fn modality_fields() -> BTreeSet<String> {
    LLM_MODALITY_KEYS
        .iter()
        .map(|k| format!("modality_metrics.{k}"))
        .collect()
}

/// Acceptance for #202: a plain run fires exactly the published plain counts
/// (every non-optional field plus the optional fields whose conditions hold),
/// and a run that meets more conditions fires their fields too. Every fired
/// field is in the schema count.
#[test]
fn published_counts_match_real_llm_runs() {
    // Paced like the parity harness, so TTFT and ITL are real intervals.
    let Some(dummy) = common::spawn_dummy(&["-chunk-interval", "1ms"]) else {
        common::skip("go not available");
        return;
    };
    let reasoning = common::spawn_dummy(&["-chunk-interval", "1ms", "-reasoning-tokens", "3"])
        .expect("second dummy");
    let dir = tempfile::tempdir().expect("tempdir");
    let s = schemas();

    let plain = run_llm(&dummy.url("/v1/chat/completions"), dir.path(), "plain", &[]);
    let (quantities, per_request) = fired(&plain);
    assert_eq!(
        quantities,
        s.summary.expected(&summary_optional(), PLAIN_RUN),
        "plain summary quantities"
    );
    let mut expected_fields = s.request.expected(&request_optional(), PLAIN_RUN);
    expected_fields.extend(modality_fields());
    assert_eq!(per_request, expected_fields, "plain per-request fields");
    let doc = render(&s);
    assert!(doc.contains(&format!("It fires **{}**", quantities.len())));
    assert!(doc.contains(&format!("**{}** per-request fields", per_request.len())));

    // Reasoning, open loop, SLOs, price, ISL/OSL targets and NDJSON together.
    let ndjson = dir.path().join("full.ndjson");
    let full = run_llm(
        &reasoning.url("/v1/chat/completions"),
        dir.path(),
        "full",
        &[
            "--request-rate",
            "50",
            "--slo",
            "ttft=2",
            "--slo",
            "user_tps=1",
            "--price-per-hour",
            "2.0",
            "--isl-target",
            "8",
            "--osl-target",
            "8",
            "--ndjson",
            ndjson.to_str().expect("utf8"),
        ],
    );
    let active = [
        HttpPath,
        Usage,
        Streaming,
        ReasoningStream,
        ReasoningUsage,
        OpenLoop,
        Slo,
        UserTpsSlo,
        Price,
        IslOslTargets,
        Ndjson,
    ];
    let (quantities, per_request) = fired(&full);
    assert_eq!(
        quantities,
        s.summary.expected(&summary_optional(), &active),
        "full-run summary quantities"
    );
    let mut expected_fields = s.request.expected(&request_optional(), &active);
    expected_fields.extend(modality_fields());
    assert_eq!(per_request, expected_fields, "full-run per-request fields");

    // telemetry.v1 request rows: subset of the schema, superset of the always set.
    let rows: Vec<Value> = std::fs::read_to_string(&ndjson)
        .expect("ndjson")
        .lines()
        .map(|line| serde_json::from_str(line).expect("ndjson row"))
        .filter(|row: &Value| row["kind"] == "request" && row["warmup"] == false)
        .collect();
    assert!(!rows.is_empty());
    let mut row_fields = BTreeSet::new();
    for row in &rows {
        let mut row = row.clone();
        row.as_object_mut().unwrap().remove("kind");
        row_fields.extend(fields(&row, REQUEST_ROW_SKIP));
    }
    assert_eq!(
        row_fields,
        s.request_row.expected(&request_row_optional(), &active),
        "telemetry request-row fields"
    );
}
