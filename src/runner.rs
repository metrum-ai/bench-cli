// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Shared load-runner helpers: stop signals, measured windows, schedule semantics.
//!
//! Modality binaries still own request construction; this module owns the
//! bookkeeping that must not drift (F-01, F-02, F-04, F-18, F-19, N-02).

use crate::load::ArrivalKind;
use crate::record::{Phase, RequestRecord};
use chrono::{DateTime, Utc};
use log::warn;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Shared flag: stop issuing new requests (SIGINT / SIGTERM / stop-after).
#[derive(Clone, Default)]
pub struct StopFlag {
    inner: Arc<AtomicBool>,
}

impl StopFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stop(&self) {
        self.inner.store(true, Ordering::SeqCst);
    }

    pub fn is_stopped(&self) -> bool {
        self.inner.load(Ordering::SeqCst)
    }

    pub fn as_atomic(&self) -> Arc<AtomicBool> {
        self.inner.clone()
    }
}

/// Install SIGINT and (on Unix) SIGTERM handlers that set `flag`.
/// A second signal after stop is already set exits the process immediately.
pub fn install_stop_handlers(flag: StopFlag) {
    let flag_ctrl = flag.clone();
    tokio::spawn(async move {
        loop {
            if tokio::signal::ctrl_c().await.is_err() {
                break;
            }
            if flag_ctrl.is_stopped() {
                warn!("Second Ctrl-C; exiting without waiting for drain");
                std::process::exit(130);
            }
            warn!("Ctrl-C received; stopping new requests and draining in-flight work");
            flag_ctrl.stop();
        }
    });

    #[cfg(unix)]
    {
        let flag_term = flag;
        tokio::spawn(async move {
            use tokio::signal::unix::{signal, SignalKind};
            let Ok(mut sigterm) = signal(SignalKind::terminate()) else {
                return;
            };
            loop {
                if sigterm.recv().await.is_none() {
                    break;
                }
                if flag_term.is_stopped() {
                    warn!("Second SIGTERM; exiting without waiting for drain");
                    std::process::exit(143);
                }
                warn!("SIGTERM received; stopping new requests and draining in-flight work");
                flag_term.stop();
            }
        });
    }
}

/// Open-loop queue delay after the scheduled arrival; closed-loop is always zero.
pub fn queue_delay_for_slot(
    kind: ArrivalKind,
    run_elapsed: Duration,
    scheduled_delay: Duration,
) -> Duration {
    match kind {
        ArrivalKind::ClosedLoop => Duration::ZERO,
        ArrivalKind::Constant | ArrivalKind::Poisson => run_elapsed.saturating_sub(scheduled_delay),
    }
}

/// Whether schedule fields should appear on the record.
pub fn should_record_schedule(kind: ArrivalKind) -> bool {
    !matches!(kind, ArrivalKind::ClosedLoop)
}

/// Monotonic send offset from the run epoch when present; otherwise wall
/// `started_at` (legacy records / unit fixtures without `send_offset_s`).
pub fn send_offset_seconds(record: &RequestRecord) -> f64 {
    record
        .send_offset_s
        .unwrap_or_else(|| datetime_to_unix_secs(record.started_at))
}

/// Measured window: first measured send → last measured successful completion.
/// Prefer monotonic `send_offset_s` so wall-clock steps cannot inflate the
/// window (N-02). Falls back to `started_at` for records that lack the field.
pub fn window_seconds_from_records(records: &[RequestRecord]) -> f64 {
    let measured: Vec<&RequestRecord> = records
        .iter()
        .filter(|r| r.phase == Phase::Measure)
        .collect();
    if measured.is_empty() {
        return 0.0;
    }
    let mut min_send = f64::INFINITY;
    let mut max_end = f64::NEG_INFINITY;
    for r in &measured {
        let send = send_offset_seconds(r);
        min_send = min_send.min(send);
        if r.is_success() {
            max_end = max_end.max(send + r.latency_s);
        }
    }
    if !min_send.is_finite() || !max_end.is_finite() || max_end < min_send {
        return 0.0;
    }
    max_end - min_send
}

fn datetime_to_unix_secs(ts: DateTime<Utc>) -> f64 {
    ts.timestamp() as f64 + f64::from(ts.timestamp_subsec_nanos()) / 1e9
}

/// Wall-clock completion = send + measured latency (avoids collection-loop skew).
pub fn completed_at_from_start(started_at: DateTime<Utc>, latency: Duration) -> DateTime<Utc> {
    started_at + chrono::Duration::from_std(latency).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::RequestRecord;
    use chrono::TimeZone;

    fn sample(seq: u64, started: DateTime<Utc>, latency_ms: u64) -> RequestRecord {
        RequestRecord::success(
            seq,
            Phase::Measure,
            "ep".into(),
            started,
            completed_at_from_start(started, Duration::from_millis(latency_ms)),
            Duration::from_millis(latency_ms),
            Some(Duration::from_millis(50)),
            None,
            vec![],
            1,
            2,
            3,
        )
    }

    #[test]
    fn window_spans_first_send_to_last_completion() {
        let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let records = vec![
            sample(0, t0, 500),
            sample(1, t0 + chrono::Duration::milliseconds(500), 500),
            sample(2, t0 + chrono::Duration::milliseconds(1000), 500),
            sample(3, t0 + chrono::Duration::milliseconds(1500), 500),
        ];
        let w = window_seconds_from_records(&records);
        assert!((w - 2.0).abs() < 1e-6, "window={w}");
    }

    #[test]
    fn window_prefers_send_offset_over_wall_clock_step() {
        let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let mut early = sample(0, t0, 500).with_send_offset(Duration::from_millis(0));
        let mut late = sample(1, t0 + chrono::Duration::hours(1), 500)
            .with_send_offset(Duration::from_millis(500));
        // Wall clock jumped +1h between sends; monotonic offsets stay small.
        early.started_at = t0;
        late.started_at = t0 + chrono::Duration::hours(1);
        let w = window_seconds_from_records(&[early, late]);
        assert!(
            (w - 1.0).abs() < 1e-6,
            "window should ignore wall step, got {w}"
        );
    }

    #[test]
    fn closed_loop_queue_delay_is_zero() {
        assert_eq!(
            queue_delay_for_slot(
                ArrivalKind::ClosedLoop,
                Duration::from_secs(5),
                Duration::ZERO
            ),
            Duration::ZERO
        );
    }
}
