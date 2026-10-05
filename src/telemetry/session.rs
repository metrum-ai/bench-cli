// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! One telemetry lifecycle shared by every benchmark binary (#196).
//!
//! A [`TelemetrySession`] owns the NDJSON writer, probes and spawns the
//! Prometheus scrapers, writes the `run` row, turns modality request records
//! into `request` and `stage` rows on the same [`RunEpoch`], and closes the
//! file with a `summary` row. The strategic sweep and the llm, vlm, asr and
//! imagegen binaries all go through it, so every `telemetry.v1` file has the
//! same kinds and one monotonic clock.

use super::config::TelemetryConfig;
use super::epoch::RunEpoch;
use super::row::{
    PhaseKind, RequestRow, Row, RunRow, StageRow, SummaryRow, TelemetrySourceStamp,
    TELEMETRY_SCHEMA_VERSION,
};
use super::scraper::{build_telemetry_client, probe_sources, spawn_scrapers};
use super::writer::{NdjsonWriter, NdjsonWriterHandle, WriterStats};
use crate::record::{Phase, RequestRecord};
use crate::runner::StopFlag;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

/// Longest wait for queued rows before the `summary` row counts them.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);

/// Telemetry flags shared by the llm, vlm, asr and imagegen binaries.
///
/// Same names, defaults and semantics as `metrum-ai-bench-cli-strategic`.
#[derive(clap::Args, Debug, Clone, Default)]
pub struct TelemetryArgs {
    #[arg(
        long,
        value_name = "PATH",
        help = "Tagged NDJSON run log (run/stage/request/telemetry/summary rows, telemetry.v1)"
    )]
    pub ndjson: Option<PathBuf>,

    #[arg(
        long,
        value_name = "PATH",
        help = "Telemetry scrape YAML (Prometheus /metrics or /metric sources); requires --ndjson"
    )]
    pub telemetry: Option<PathBuf>,

    #[arg(
        long,
        default_value_t = false,
        help = "Fail the run when a telemetry source cannot be scraped (startup probe, or N consecutive failures mid-run; default N=3)"
    )]
    pub require_telemetry: bool,

    #[arg(
        long,
        default_value_t = super::config::DEFAULT_REQUIRE_FAILURES,
        help = "Consecutive scrape failures before --require-telemetry aborts"
    )]
    pub require_telemetry_failures: u32,
}

impl TelemetryArgs {
    /// Validate flag combinations and load the YAML. Call before any request.
    pub fn resolve_config(&self) -> Result<Option<TelemetryConfig>> {
        if self.require_telemetry && self.telemetry.is_none() {
            bail!("--require-telemetry needs --telemetry YAML");
        }
        if self.telemetry.is_some() && self.ndjson.is_none() {
            bail!("--telemetry requires --ndjson PATH");
        }
        self.telemetry
            .as_deref()
            .map(TelemetryConfig::load)
            .transpose()
    }

    /// Start a session when `--ndjson` is set; `None` otherwise.
    pub async fn start_session(
        &self,
        config: Option<&TelemetryConfig>,
        stamp: RunStamp,
        abort: Option<StopFlag>,
    ) -> Result<Option<TelemetrySession>> {
        let Some(path) = &self.ndjson else {
            return Ok(None);
        };
        let options = SessionOptions {
            ndjson: path.clone(),
            config,
            require_telemetry: self.require_telemetry,
            require_failures: self.require_telemetry_failures,
            abort,
        };
        TelemetrySession::start(Arc::new(RunEpoch::new()), stamp, options)
            .await
            .map(Some)
    }
}

/// Identity stamped into the `run` row.
#[derive(Debug, Clone)]
pub struct RunStamp {
    pub run_id: String,
    pub tool_version: String,
    pub sut: Option<Value>,
    /// Redacted run config (never API keys).
    pub config: Value,
}

/// How a session writes and scrapes.
pub struct SessionOptions<'a> {
    pub ndjson: PathBuf,
    pub config: Option<&'a TelemetryConfig>,
    pub require_telemetry: bool,
    pub require_failures: u32,
    /// Run stop flag tripped when a required source fails mid-run, so the
    /// binary stops issuing requests instead of benchmarking blind.
    pub abort: Option<StopFlag>,
}

/// Additive `summary.v3.telemetry` block: where the NDJSON went and how
/// many rows of each kind it holds. Present only when `--ndjson` is set.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct TelemetryRunInfo {
    pub schema_version: &'static str,
    pub ndjson: String,
    /// Configured scrape sources (0 without `--telemetry`).
    pub sources: u64,
    pub request_rows: u64,
    pub stage_rows: u64,
    pub telemetry_rows: u64,
    pub scrape_error_rows: u64,
    /// Telemetry samples dropped under writer backpressure.
    pub dropped_telemetry_rows: u64,
}

/// First send and last completion seen for one phase, in epoch ns.
#[derive(Debug, Clone, Copy)]
struct Window {
    start_ns: u64,
    end_ns: u64,
}

impl Window {
    fn widen(window: &mut Option<Self>, start_ns: u64, end_ns: u64) {
        let next = match *window {
            Some(w) => Self {
                start_ns: w.start_ns.min(start_ns),
                end_ns: w.end_ns.max(end_ns),
            },
            None => Self { start_ns, end_ns },
        };
        *window = Some(next);
    }
}

/// A running NDJSON writer plus its scrapers, on one [`RunEpoch`].
pub struct TelemetrySession {
    epoch: Arc<RunEpoch>,
    run_id: Arc<String>,
    ndjson: PathBuf,
    writer: NdjsonWriter,
    handle: NdjsonWriterHandle,
    scrapers: Vec<JoinHandle<Result<()>>>,
    stop_scrapers: Arc<AtomicBool>,
    require_telemetry: bool,
    sources: u64,
    load: Option<f64>,
    warmup: Option<Window>,
    measure: Option<Window>,
}

impl TelemetrySession {
    /// Open the NDJSON file, probe every source, write the `run` row, then
    /// start the scrapers.
    ///
    /// Any source that fails its startup probe fails the call before a
    /// request is sent, with or without `--require-telemetry`.
    pub async fn start(
        epoch: Arc<RunEpoch>,
        stamp: RunStamp,
        options: SessionOptions<'_>,
    ) -> Result<Self> {
        let (writer, handle) = NdjsonWriter::spawn(options.ndjson.clone())?;
        let run_id = Arc::new(stamp.run_id);
        let mut session = Self {
            epoch,
            run_id,
            ndjson: options.ndjson,
            writer,
            handle,
            scrapers: Vec::new(),
            stop_scrapers: Arc::new(AtomicBool::new(false)),
            require_telemetry: options.require_telemetry,
            sources: 0,
            load: None,
            warmup: None,
            measure: None,
        };
        let mut stamps: Vec<TelemetrySourceStamp> = Vec::new();
        let mut scrape_plan = None;
        if let Some(cfg) = options.config {
            cfg.warn_fast_sources();
            let sources = cfg.compile()?;
            let insecure = sources.iter().any(|s| s.insecure_tls);
            let client = build_telemetry_client(insecure, true)?;
            let probes = probe_sources(&client, cfg, &sources).await.context(
                "telemetry: a configured source could not be scraped at startup; no requests were sent",
            )?;
            session.sources = sources.len() as u64;
            stamps = probes.into_iter().map(|p| p.stamp).collect();
            scrape_plan = Some((client, cfg.clone(), sources));
        }
        // The run row goes first so readers see the epoch before any sample.
        session
            .writer
            .send_priority(Row::Run(RunRow {
                run_id: (*session.run_id).clone(),
                t0_wall: session.epoch.t0_wall_iso(),
                tool_version: stamp.tool_version,
                schema_version: TELEMETRY_SCHEMA_VERSION.to_string(),
                sut: stamp.sut,
                config: stamp.config,
                telemetry_sources: stamps,
            }))
            .await?;
        if let Some((client, cfg, sources)) = scrape_plan {
            let handles = spawn_scrapers(
                client,
                cfg,
                sources,
                Arc::clone(&session.epoch),
                Arc::clone(&session.run_id),
                session.writer.clone(),
                Arc::clone(&session.stop_scrapers),
                options.require_telemetry,
                options.require_failures,
            );
            session.scrapers = handles
                .into_iter()
                .map(|handle| {
                    let abort = options.abort.clone();
                    tokio::spawn(async move {
                        let outcome = match handle.await {
                            Ok(result) => result,
                            Err(err) => Err(anyhow::anyhow!("telemetry scraper panicked: {err}")),
                        };
                        if outcome.is_err() {
                            if let Some(flag) = abort {
                                flag.stop();
                            }
                        }
                        outcome
                    })
                })
                .collect();
        }
        Ok(session)
    }

    /// Shared clock for request and stage timestamps.
    pub fn epoch(&self) -> Arc<RunEpoch> {
        Arc::clone(&self.epoch)
    }

    pub fn run_id(&self) -> Arc<String> {
        Arc::clone(&self.run_id)
    }

    /// Writer handle for callers that emit their own request and stage rows.
    pub fn writer(&self) -> NdjsonWriter {
        self.writer.clone()
    }

    /// Modality runs have one stage. `load` is the offered load: requests
    /// per second for open-loop runs, the concurrency cap otherwise. It
    /// fills `stage` and `load` on request and stage rows.
    pub fn set_load(&mut self, load: f64) {
        self.load = Some(load);
    }

    /// Write one `request` row for a modality record and widen its phase
    /// window. `run_start` is the `Instant` that `send_offset_s` counts from.
    pub async fn record_request(
        &mut self,
        record: &RequestRecord,
        run_start: Instant,
    ) -> Result<()> {
        let base_ns = run_start
            .saturating_duration_since(self.epoch.mono())
            .as_nanos() as u64;
        let row = request_row(&self.run_id, record, base_ns, self.load.unwrap_or(0.0));
        let window = match record.phase {
            Phase::Warmup => Some(&mut self.warmup),
            Phase::Measure => Some(&mut self.measure),
            Phase::Drain => None,
        };
        if let Some(window) = window {
            Window::widen(window, row.t_sent_ns, row.t_done_ns);
        }
        self.writer.send_priority(Row::Request(row)).await
    }

    /// Stop the scrapers and wait for them. A scraper that failed is an
    /// error under `--require-telemetry` and a warning otherwise. Safe to
    /// call more than once.
    pub async fn join_scrapers(&mut self) -> Result<()> {
        self.stop_scrapers.store(true, Ordering::Relaxed);
        let mut first_error = None;
        for handle in self.scrapers.drain(..) {
            let outcome = match handle.await {
                Ok(result) => result,
                Err(err) => Err(anyhow::anyhow!("telemetry scraper join failed: {err}")),
            };
            if let Err(err) = outcome {
                if self.require_telemetry {
                    first_error.get_or_insert(err);
                } else {
                    eprintln!("warning: telemetry scraper exited: {err:#}");
                }
            }
        }
        match first_error {
            Some(err) => Err(err.context("telemetry required (--require-telemetry)")),
            None => Ok(()),
        }
    }

    /// Write the stage windows seen by [`Self::record_request`], then the
    /// `summary` row, and close the file. Call after [`Self::join_scrapers`].
    pub async fn finish(mut self, partial: bool) -> Result<TelemetryRunInfo> {
        if !self.scrapers.is_empty() {
            // Strict callers check join_scrapers themselves; here only the
            // scrape tasks must be gone before the file closes.
            if let Err(err) = self.join_scrapers().await {
                eprintln!("warning: {err:#}");
            }
        }
        let load = self.load.unwrap_or(0.0);
        for (phase, window) in [
            (PhaseKind::Warmup, self.warmup),
            (PhaseKind::Measure, self.measure),
        ] {
            if let Some(window) = window {
                self.writer
                    .send_priority(Row::Stage(StageRow {
                        run_id: (*self.run_id).clone(),
                        stage: load,
                        load,
                        phase,
                        t_start_ns: window.start_ns,
                        t_end_ns: window.end_ns,
                    }))
                    .await?;
            }
        }
        if !self.writer.flush_pending(FLUSH_TIMEOUT).await {
            eprintln!("warning: telemetry NDJSON writer did not drain; summary counts may lag");
        }
        let snap = self.writer.stats_snapshot();
        let dropped = self.writer.dropped_telemetry_rows();
        self.writer
            .send_priority(Row::Summary(SummaryRow {
                run_id: (*self.run_id).clone(),
                partial,
                dropped_telemetry_rows: dropped,
                request_rows: snap.request_rows,
                telemetry_rows: snap.telemetry_rows,
                scrape_error_rows: snap.scrape_error_rows,
                stage_rows: snap.stage_rows,
            }))
            .await?;
        let ndjson = self.ndjson.clone();
        let sources = self.sources;
        drop(self.writer);
        let stats: WriterStats = self.handle.shutdown().await?;
        Ok(TelemetryRunInfo {
            schema_version: TELEMETRY_SCHEMA_VERSION,
            ndjson: display_path(&ndjson),
            sources,
            request_rows: stats.request_rows,
            stage_rows: stats.stage_rows,
            telemetry_rows: stats.telemetry_rows,
            scrape_error_rows: stats.scrape_error_rows,
            dropped_telemetry_rows: stats.dropped_telemetry_rows,
        })
    }
}

/// End-of-run close for an optional session: join the scrapers, then write
/// stage and summary rows. Returns the `summary.v3.telemetry` stamp and the
/// `--require-telemetry` verdict. Callers write their own summary first and
/// raise the verdict after, so a failed requirement still leaves the data.
pub async fn close_session(
    session: Option<TelemetrySession>,
    stopped: bool,
) -> Result<(Option<TelemetryRunInfo>, Result<()>)> {
    let Some(mut session) = session else {
        return Ok((None, Ok(())));
    };
    let verdict = session.join_scrapers().await;
    let info = session.finish(stopped || verdict.is_err()).await?;
    Ok((Some(info), verdict))
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

fn secs_to_ns(seconds: f64) -> u64 {
    if seconds.is_finite() && seconds > 0.0 {
        (seconds * 1e9).round() as u64
    } else {
        0
    }
}

/// Map a modality `request.v3` record onto a `telemetry.v1` request row.
///
/// `base_ns` is the run start (`send_offset_s` origin) on the epoch. The
/// row's `latency_s` includes queue delay, as on strategic rows, and
/// `service_latency_s` is the record's send-to-done `latency_s`. `t_first_ns`
/// is response headers (`first_byte_s`), as on strategic rows; it is omitted
/// when the record has no first byte.
pub fn request_row(run_id: &str, record: &RequestRecord, base_ns: u64, load: f64) -> RequestRow {
    let t_sent_ns = base_ns + secs_to_ns(record.send_offset_s.unwrap_or(0.0));
    let t_sched_ns = record
        .scheduled_offset_s
        .map_or(t_sent_ns, |offset| base_ns + secs_to_ns(offset));
    RequestRow {
        run_id: run_id.to_string(),
        seq: record.seq,
        stage: load,
        warmup: record.phase == Phase::Warmup,
        t_sched_ns,
        t_sent_ns,
        t_first_ns: record.first_byte_s.map(|s| t_sent_ns + secs_to_ns(s)),
        t_done_ns: t_sent_ns + secs_to_ns(record.latency_s),
        success: record.is_success(),
        input_tokens: record.prompt_tokens,
        output_tokens: record.completion_tokens,
        reasoning_tokens: record.reasoning_tokens,
        latency_s: record.queue_delay_s + record.latency_s,
        queue_delay_s: record.queue_delay_s,
        service_latency_s: record.latency_s,
        ttft_s: record.ttft_s,
        ttft_source: record.ttft_source.map(|s| s.as_str().to_string()),
        error: record.error.as_ref().map(|e| e.to_string()),
        telemetry_at_done: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn record(seq: u64, phase: Phase, send_offset: f64, latency: f64) -> RequestRecord {
        let now = Utc::now();
        RequestRecord::success(
            seq,
            phase,
            "default".into(),
            now,
            now,
            Duration::from_secs_f64(latency),
            None,
            None,
            Vec::new(),
            7,
            11,
            18,
        )
        .with_send_offset(Duration::from_secs_f64(send_offset))
        .with_first_byte(Duration::from_millis(20))
    }

    #[test]
    fn request_row_shares_epoch_offsets() {
        let rec = record(3, Phase::Measure, 0.5, 0.25);
        let row = request_row("run-1", &rec, 1_000, 4.0);
        assert_eq!(row.t_sent_ns, 1_000 + 500_000_000);
        assert_eq!(row.t_sched_ns, row.t_sent_ns);
        assert_eq!(row.t_first_ns, Some(row.t_sent_ns + 20_000_000));
        assert_eq!(row.t_done_ns, row.t_sent_ns + 250_000_000);
        assert_eq!((row.input_tokens, row.output_tokens), (7, 11));
        assert!(row.success && !row.warmup);
        assert_eq!(row.stage, 4.0);
        assert!((row.service_latency_s - 0.25).abs() < 1e-12);
    }

    #[test]
    fn open_loop_row_keeps_schedule_and_queue_delay() {
        let rec = record(0, Phase::Warmup, 1.2, 0.1)
            .with_schedule(Duration::from_secs(1), Duration::from_millis(200));
        let row = request_row("run-1", &rec, 0, 2.0);
        assert_eq!(row.t_sched_ns, 1_000_000_000);
        assert_eq!(row.t_sent_ns, 1_200_000_000);
        assert!(row.warmup);
        assert!((row.latency_s - 0.3).abs() < 1e-9);
        assert!((row.queue_delay_s - 0.2).abs() < 1e-9);
    }

    #[test]
    fn failed_record_has_error_and_no_first_byte() {
        let now = Utc::now();
        let rec = RequestRecord::failed(
            1,
            Phase::Measure,
            "default".into(),
            now,
            now,
            Duration::from_millis(5),
            crate::error::RequestError::Timeout,
        )
        .with_send_offset(Duration::from_millis(10));
        let row = request_row("run-1", &rec, 0, 1.0);
        assert!(!row.success);
        assert!(row.error.is_some());
        assert_eq!(row.t_first_ns, None);
        assert_eq!(row.t_done_ns, 15_000_000);
    }

    #[test]
    fn args_validate_flag_combinations() {
        let args = TelemetryArgs {
            require_telemetry: true,
            ..Default::default()
        };
        let err = args.resolve_config().expect_err("require without yaml");
        assert!(err.to_string().contains("--require-telemetry"));
        let args = TelemetryArgs {
            telemetry: Some("t.yaml".into()),
            ..Default::default()
        };
        let err = args.resolve_config().expect_err("yaml without ndjson");
        assert!(err.to_string().contains("--ndjson"));
        assert!(TelemetryArgs::default()
            .resolve_config()
            .expect("no flags")
            .is_none());
    }

    #[tokio::test]
    async fn session_without_sources_writes_rows_on_one_clock() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("run.ndjson");
        let args = TelemetryArgs {
            ndjson: Some(path.clone()),
            ..Default::default()
        };
        let stamp = RunStamp {
            run_id: "run-1".into(),
            tool_version: "0.0.0".into(),
            sut: None,
            config: serde_json::json!({"binary": "test"}),
        };
        let mut session = args
            .start_session(None, stamp, None)
            .await
            .expect("start")
            .expect("session");
        session.set_load(2.0);
        let run_start = Instant::now();
        session
            .record_request(&record(0, Phase::Warmup, 0.0, 0.01), run_start)
            .await
            .expect("warmup row");
        session
            .record_request(&record(1, Phase::Measure, 0.02, 0.01), run_start)
            .await
            .expect("measure row");
        session.join_scrapers().await.expect("no scrapers");
        let info = session.finish(false).await.expect("finish");
        assert_eq!(info.request_rows, 2);
        assert_eq!(info.stage_rows, 2);
        assert_eq!(info.sources, 0);
        let rows: Vec<Value> = std::fs::read_to_string(&path)
            .expect("read")
            .lines()
            .map(|l| serde_json::from_str(l).expect("json"))
            .collect();
        let kinds: Vec<&str> = rows.iter().map(|r| r["kind"].as_str().unwrap()).collect();
        assert_eq!(
            kinds,
            ["run", "request", "request", "stage", "stage", "summary"]
        );
        assert_eq!(rows[3]["phase"], "warmup");
        assert_eq!(rows[4]["phase"], "measure");
        assert_eq!(rows[4]["t_start_ns"], rows[2]["t_sent_ns"]);
        assert_eq!(rows[4]["t_end_ns"], rows[2]["t_done_ns"]);
        assert_eq!(rows[5]["request_rows"], 2);
        assert_eq!(rows[5]["stage_rows"], 2);
    }
}
