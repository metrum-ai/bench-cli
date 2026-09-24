// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::row::Row;
use anyhow::{Context, Result};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Bounded channel capacity for all NDJSON rows.
pub const TELEMETRY_CHANNEL_CAPACITY: usize = 8192;

const FLUSH_EVERY: u64 = 64;

#[derive(Debug, Default, Clone)]
pub struct WriterStats {
    pub written_rows: u64,
    pub dropped_telemetry_rows: u64,
    pub request_rows: u64,
    pub telemetry_rows: u64,
    pub scrape_error_rows: u64,
    pub stage_rows: u64,
    pub run_rows: u64,
    pub summary_rows: u64,
}

#[derive(Default)]
struct Counters {
    written: AtomicU64,
    dropped: AtomicU64,
    request: AtomicU64,
    telemetry: AtomicU64,
    scrape_error: AtomicU64,
    stage: AtomicU64,
    run: AtomicU64,
    summary: AtomicU64,
}

impl Counters {
    fn snapshot(&self) -> WriterStats {
        WriterStats {
            written_rows: self.written.load(Ordering::Relaxed),
            dropped_telemetry_rows: self.dropped.load(Ordering::Relaxed),
            request_rows: self.request.load(Ordering::Relaxed),
            telemetry_rows: self.telemetry.load(Ordering::Relaxed),
            scrape_error_rows: self.scrape_error.load(Ordering::Relaxed),
            stage_rows: self.stage.load(Ordering::Relaxed),
            run_rows: self.run.load(Ordering::Relaxed),
            summary_rows: self.summary.load(Ordering::Relaxed),
        }
    }

    fn classify(&self, row: &Row) {
        match row {
            Row::Run(_) => {
                self.run.fetch_add(1, Ordering::Relaxed);
            }
            Row::Stage(_) => {
                self.stage.fetch_add(1, Ordering::Relaxed);
            }
            Row::Telemetry(_) => {
                self.telemetry.fetch_add(1, Ordering::Relaxed);
            }
            Row::Request(_) => {
                self.request.fetch_add(1, Ordering::Relaxed);
            }
            Row::ScrapeError(_) => {
                self.scrape_error.fetch_add(1, Ordering::Relaxed);
            }
            Row::Summary(_) => {
                self.summary.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Handle used by request tasks and scrapers to enqueue rows.
#[derive(Clone)]
pub struct NdjsonWriter {
    tx: mpsc::Sender<Row>,
    counters: Arc<Counters>,
}

/// Join handle for the background writer task.
pub struct NdjsonWriterHandle {
    join: JoinHandle<Result<WriterStats>>,
    tx: mpsc::Sender<Row>,
}

impl NdjsonWriter {
    /// Spawn a writer task that appends NDJSON to `path`.
    pub fn spawn(path: PathBuf) -> Result<(Self, NdjsonWriterHandle)> {
        let file = File::create(&path)
            .with_context(|| format!("create telemetry NDJSON {}", path.display()))?;
        let (tx, rx) = mpsc::channel(TELEMETRY_CHANNEL_CAPACITY);
        let counters = Arc::new(Counters::default());
        let counters_bg = Arc::clone(&counters);
        let join = tokio::spawn(async move { writer_loop(file, rx, counters_bg).await });
        let writer = Self {
            tx: tx.clone(),
            counters: Arc::clone(&counters),
        };
        let handle = NdjsonWriterHandle { join, tx };
        Ok((writer, handle))
    }

    /// Await send for request, stage, run, summary, and scrape_error rows.
    /// These rows are never intentionally dropped.
    pub async fn send_priority(&self, row: Row) -> Result<()> {
        self.tx
            .send(row)
            .await
            .map_err(|_| anyhow::anyhow!("telemetry NDJSON writer closed"))
    }

    /// Best-effort telemetry sample. Returns `true` when the row was dropped.
    pub fn try_send_telemetry(&self, row: Row) -> bool {
        match self.tx.try_send(row) {
            Ok(()) => false,
            Err(mpsc::error::TrySendError::Full(_)) | Err(mpsc::error::TrySendError::Closed(_)) => {
                self.counters.dropped.fetch_add(1, Ordering::Relaxed);
                true
            }
        }
    }

    pub fn dropped_telemetry_rows(&self) -> u64 {
        self.counters.dropped.load(Ordering::Relaxed)
    }

    /// Counts of rows already accepted by the writer task (may lag the channel).
    pub fn stats_snapshot(&self) -> WriterStats {
        self.counters.snapshot()
    }
}

impl NdjsonWriterHandle {
    /// Drop the sender side and wait for the writer to flush and exit.
    pub async fn shutdown(self) -> Result<WriterStats> {
        drop(self.tx);
        match self.join.await {
            Ok(Ok(stats)) => Ok(stats),
            Ok(Err(err)) => Err(err),
            Err(err) => Err(anyhow::anyhow!("telemetry writer task panicked: {err}")),
        }
    }
}

async fn writer_loop(
    file: File,
    mut rx: mpsc::Receiver<Row>,
    counters: Arc<Counters>,
) -> Result<WriterStats> {
    let mut out = BufWriter::new(file);
    let mut since_flush = 0u64;
    while let Some(row) = rx.recv().await {
        counters.classify(&row);
        serde_json::to_writer(&mut out, &row).context("serialize telemetry row")?;
        out.write_all(b"\n").context("write telemetry newline")?;
        counters.written.fetch_add(1, Ordering::Relaxed);
        since_flush += 1;
        if since_flush >= FLUSH_EVERY {
            out.flush().context("flush telemetry NDJSON")?;
            since_flush = 0;
        }
    }
    out.flush().context("final flush telemetry NDJSON")?;
    Ok(counters.snapshot())
}
