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

/// Rows per batched flush under sustained load.
const FLUSH_EVERY: u64 = 64;
/// Maximum delay before buffered rows reach the file (#310): sparse runs
/// (low rate, few requests) must still become visible to a tailing reader
/// promptly, so the 64-row batch bound is paired with a time bound.
const FLUSH_INTERVAL_MS: u64 = 250;

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
    /// Rows accepted into the channel (priority sends and kept telemetry).
    enqueued: AtomicU64,
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
            .map_err(|_| anyhow::anyhow!("telemetry NDJSON writer closed"))?;
        self.counters.enqueued.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Best-effort telemetry sample. Returns `true` when the row was dropped.
    pub fn try_send_telemetry(&self, row: Row) -> bool {
        match self.tx.try_send(row) {
            Ok(()) => {
                self.counters.enqueued.fetch_add(1, Ordering::Relaxed);
                false
            }
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

    /// Wait until the writer task has written every row enqueued so far, or
    /// until `timeout` passes. Returns `true` when the channel drained, so a
    /// following [`Self::stats_snapshot`] counts every row sent before it.
    pub async fn flush_pending(&self, timeout: std::time::Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            // `enqueued` is bumped after the send returns, so it can trail
            // `written` briefly; `>=` treats that as drained.
            let enqueued = self.counters.enqueued.load(Ordering::Relaxed);
            if self.counters.written.load(Ordering::Relaxed) >= enqueued {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
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
    let flush_interval = tokio::time::Duration::from_millis(FLUSH_INTERVAL_MS);
    let mut flush_tick = tokio::time::interval(flush_interval);
    flush_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // The first tick fires immediately; it is a no-op (nothing buffered).
    loop {
        tokio::select! {
            row = rx.recv() => {
                let Some(row) = row else { break };
                counters.classify(&row);
                serde_json::to_writer(&mut out, &row).context("serialize telemetry row")?;
                out.write_all(b"\n").context("write telemetry newline")?;
                counters.written.fetch_add(1, Ordering::Relaxed);
                since_flush += 1;
                // Sustained load flushes in FLUSH_EVERY batches without
                // waiting for a tick.
                if since_flush >= FLUSH_EVERY {
                    out.flush().context("flush telemetry NDJSON")?;
                    since_flush = 0;
                }
            }
            _ = flush_tick.tick() => {
                // Time bound (#310): anything buffered reaches the file
                // within the interval, even when rows are sparse and the
                // batch bound would never trigger.
                if since_flush > 0 {
                    out.flush().context("flush telemetry NDJSON")?;
                    since_flush = 0;
                }
            }
        }
    }
    out.flush().context("final flush telemetry NDJSON")?;
    Ok(counters.snapshot())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::row::{Row, RunRow};

    /// #310: a sparse run (far fewer rows than FLUSH_EVERY) must still see
    /// its rows reach the file within the time bound, so a tailing reader
    /// can join mid-run; only the 250 ms interval can surface this row.
    #[tokio::test]
    async fn sparse_rows_flush_to_disk_within_the_time_bound() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sparse.ndjson");
        let (writer, handle) = NdjsonWriter::spawn(path.clone()).unwrap();
        writer
            .send_priority(Row::Run(RunRow {
                run_id: "run-flush-bound".into(),
                t0_wall: "2026-01-01T00:00:00Z".into(),
                tool_version: "test".into(),
                schema_version: "telemetry.v1".into(),
                sut: None,
                config: serde_json::json!({}),
                telemetry_sources: vec![],
            }))
            .await
            .unwrap();

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut observed = false;
        while tokio::time::Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(&path) {
                if text.contains("\"kind\":\"run\"") {
                    observed = true;
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(
            observed,
            "a single row must reach the file without waiting for FLUSH_EVERY rows or shutdown"
        );

        // Both sender clones (writer + handle) must drop before the writer
        // task exits; otherwise shutdown waits on a channel that is never
        // closed (#310 hang review).
        drop(writer);
        let stats = tokio::time::timeout(std::time::Duration::from_secs(5), handle.shutdown())
            .await
            .expect("writer shutdown must finish once all senders are dropped")
            .unwrap();
        assert_eq!(stats.run_rows, 1);
        assert_eq!(stats.written_rows, 1);
    }
}
