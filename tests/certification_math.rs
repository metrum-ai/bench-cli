// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Deterministic fixed-clock cross-modality certification fixtures.
use chrono::Utc;
use metrum_ai_bench::measurement::{ResolvedTtft, TtftSource};
use metrum_ai_bench::record::{Phase, RequestRecord};
use metrum_ai_bench::strategic::{export_csv, stage_window_seconds, summarize_stage, BenchRecord};
use metrum_ai_bench::summary::{RunSummary, SloConfig};
use std::time::Duration;

fn row(seq: u64, send_s: u64, latency_s: u64, vlm: bool) -> RequestRecord {
    let now = Utc::now();
    let mut row = RequestRecord::success(
        seq,
        Phase::Measure,
        "fixture".into(),
        now,
        now,
        Duration::from_secs(latency_s),
        Some(Duration::from_millis(500)),
        Some(Duration::from_millis(200)),
        vec![Duration::from_millis(300)],
        4,
        8,
        12,
    )
    .with_reasoning_tokens(Some(3))
    .with_resolved_ttft(ResolvedTtft {
        ttft_s: Some(0.2),
        source: Some(TtftSource::Stream),
    })
    .with_chat_outcome(None, false)
    .with_send_offset(Duration::from_secs(send_s));
    if vlm {
        row.modality_metrics.insert("image_count".into(), 1.0);
    }
    row
}

// BC-MATH-004: identical overlapping clocks through modality JSON, strategic CSV
// and the emitted NDJSON request representation. ITL stays visible-only.
#[test]
fn bc_math_004_streaming_modalities_and_strategic_rates() {
    let close =
        |actual: Option<f64>, expected: f64| assert!((actual.unwrap() - expected).abs() < 1e-9);
    for vlm in [false, true] {
        let mut rows = vec![row(0, 0, 4, vlm), row(1, 1, 5, vlm)];
        let mut warmup = row(2, 0, 20, vlm);
        warmup.phase = Phase::Warmup;
        rows.push(warmup);
        let window = metrum_ai_bench::runner::window_seconds_from_records(&rows);
        assert_eq!(window, 6.0);
        let summary = RunSummary::from_records(&rows, window, false);
        close(summary.requests_per_second, 2.0 / 6.0);
        close(summary.answered_requests_per_second, 2.0 / 6.0);
        close(summary.goodput.requests_per_second, 2.0 / 6.0);
        close(summary.input_tokens_per_second, 8.0 / 6.0);
        close(summary.completion_tokens_per_second, 16.0 / 6.0);
        close(summary.total_tokens_per_second, 24.0 / 6.0);
        close(
            summary.non_reasoning_completion_tokens_per_second,
            10.0 / 6.0,
        );
        close(summary.ttft_s.avg, 0.2);
        close(summary.ttft_visible_answer_s.avg, 0.5);
        close(summary.itl_s.avg, 0.3);
        close(summary.time_to_second_token_s.avg, 0.8);
        close(summary.tpot_s.avg, (3.8 / 7.0 + 4.8 / 7.0) / 2.0);
        for request in &rows[..2] {
            let json = serde_json::to_value(request).unwrap();
            let telemetry = metrum_ai_bench::telemetry::request_row("fixture", request, 0, 1.0);
            assert_eq!(telemetry.ttft_s, json["ttft_s"].as_f64());
            assert_eq!(
                telemetry.ttft_visible_answer_s,
                json["ttft_visible_answer_s"].as_f64()
            );
            assert_eq!(telemetry.answer_completed, Some(true));
            assert_eq!(telemetry.outcome.as_deref(), Some("answer"));
            assert_eq!(telemetry.usage_missing, Some(false));
        }
        let records: Vec<BenchRecord> = rows
            .iter()
            .map(|r| BenchRecord {
                seq: r.seq,
                stage: 1.0,
                endpoint: "fixture".into(),
                send_offset_s: r.send_offset_s,
                sent_unix_ns: (r.send_offset_s.unwrap() * 1e9) as u128,
                latency_s: r.latency_s,
                service_latency_s: r.latency_s,
                ttft_s: r.ttft_s,
                ttft_source: r.ttft_source,
                ttft_visible_answer_s: r.ttft_visible_answer_s,
                first_reasoning_s: r.first_reasoning_s,
                itl_s: r.itl_s.clone(),
                success: r.is_success(),
                input_tokens: r.prompt_tokens,
                output_tokens: r.completion_tokens,
                reasoning_tokens: r.reasoning_tokens,
                usage_missing: r.usage_missing,
                outcome: r.outcome.clone(),
                answer_completed: r.answer_completed,
                warmup: r.phase == Phase::Warmup,
                ..Default::default()
            })
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("records.csv");
        export_csv(&csv, &records).unwrap();
        let decoded: Vec<BenchRecord> = csv::Reader::from_path(csv)
            .unwrap()
            .deserialize()
            .map(Result::unwrap)
            .collect();
        assert_eq!(stage_window_seconds(&decoded), Some(6.0));
        let point = summarize_stage(1.0, &decoded, 6.0, &SloConfig::default(), None);
        close(point.throughput, 2.0 / 6.0);
        close(point.answered_requests_per_second, 2.0 / 6.0);
        close(point.goodput, 2.0 / 6.0);
        close(point.input_tokens_per_second, 8.0 / 6.0);
        close(point.completion_tokens_per_second, 16.0 / 6.0);
        close(point.total_tokens_per_second, 24.0 / 6.0);
        close(point.non_reasoning_completion_tokens_per_second, 10.0 / 6.0);
        close(point.ttft_s.avg, 0.2);
        close(point.ttft_visible_answer_s.avg, 0.5);
        close(point.time_to_second_token_s.avg, 0.8);
        assert_eq!(decoded[0].itl_s, vec![0.3]);
        close(decoded[0].tpot_s(), 3.8 / 7.0);
        close(decoded[1].tpot_s(), 4.8 / 7.0);
    }
}
