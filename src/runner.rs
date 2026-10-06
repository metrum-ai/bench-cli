// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Shared load-runner helpers: stop signals, measured windows, schedule semantics.
//!
//! Modality binaries still own request construction; this module owns the
//! bookkeeping that must not drift (F-01, F-02, F-04, F-18, F-19, N-02).

use crate::concurrency::InFlightTracker;
use crate::load::{ArrivalKind, RequestSlot};
use crate::record::{Phase, RequestRecord};
use chrono::{DateTime, Utc};
use log::{error, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

/// Shared flag: stop issuing new requests (SIGINT / SIGTERM / stop-after /
/// `--require-telemetry` abort).
///
/// A real signal is tracked apart from the stop itself (#227), so an internal
/// stop such as a telemetry abort never turns the next Ctrl-C into a hard
/// exit: only a second real signal skips the drain and the summary writes.
#[derive(Clone, Default)]
pub struct StopFlag {
    inner: Arc<AtomicBool>,
    signaled: Arc<AtomicBool>,
}

/// What a received SIGINT or SIGTERM should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalAction {
    /// First real signal: stop new requests, drain, write the summary.
    Drain,
    /// Second real signal: exit now without waiting for drain.
    HardExit,
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

    /// Record a real SIGINT/SIGTERM and stop the run. Returns `HardExit`
    /// only when an earlier real signal was already recorded; a stop from
    /// any other source (telemetry abort, stop-after) still drains.
    pub fn on_signal(&self) -> SignalAction {
        let already = self.signaled.swap(true, Ordering::SeqCst);
        self.stop();
        if already {
            SignalAction::HardExit
        } else {
            SignalAction::Drain
        }
    }
}

/// Install SIGINT and (on Unix) SIGTERM handlers that set `flag`.
/// A second real signal exits the process immediately; a first signal after
/// an internal stop (for example a `--require-telemetry` abort) still drains
/// so `summary.v3` and the NDJSON summary row are written (#227).
pub fn install_stop_handlers(flag: StopFlag) {
    let flag_ctrl = flag.clone();
    tokio::spawn(async move {
        loop {
            if tokio::signal::ctrl_c().await.is_err() {
                break;
            }
            if flag_ctrl.on_signal() == SignalAction::HardExit {
                warn!("Second Ctrl-C; exiting without waiting for drain");
                std::process::exit(130);
            }
            warn!("Ctrl-C received; stopping new requests and draining in-flight work");
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
                if flag_term.on_signal() == SignalAction::HardExit {
                    warn!("Second SIGTERM; exiting without waiting for drain");
                    std::process::exit(143);
                }
                warn!("SIGTERM received; stopping new requests and draining in-flight work");
            }
        });
    }
}

/// Barrier between the warmup and measure phases (#226).
///
/// Measured requests start only after every warmup request has completed, so
/// the measure phase sees a warmed, drained server and the `warmup` and
/// `measure` stage windows never overlap. Open-loop schedules keep their
/// inter-arrival gaps: measured slots shift by the time the barrier added, so
/// the first measured request is due at the barrier and none of them burst to
/// catch up on time spent in warmup. Metrum AI Bench strategic sweeps apply the
/// same rule per stage.
#[derive(Debug, Default)]
pub struct WarmupBarrier {
    warmup_requests: u64,
    shift: Duration,
    join_errors: usize,
    tracker: Option<Arc<InFlightTracker>>,
}

impl WarmupBarrier {
    /// Barrier for the first `warmup_requests` slots; 0 disables it.
    pub fn new(warmup_requests: u32) -> Self {
        Self {
            warmup_requests: u64::from(warmup_requests),
            ..Self::default()
        }
    }

    /// Reset `tracker` at the barrier so `observed_concurrency` covers the
    /// measured phase only (#226).
    pub fn with_tracker(mut self, tracker: Arc<InFlightTracker>) -> Self {
        self.tracker = Some(tracker);
        self
    }

    /// Call before dispatching `slot`. On the first measured slot this awaits
    /// every handle in `handles` (all warmup tasks, since dispatch is in `seq`
    /// order) and removes them, then returns the slot with its schedule
    /// shifted. Other slots return with the shift already fixed (zero during
    /// warmup).
    pub async fn before_slot<T>(
        &mut self,
        slot: RequestSlot,
        handles: &mut Vec<JoinHandle<T>>,
        run_start: Instant,
    ) -> RequestSlot {
        if self.warmup_requests > 0 && slot.seq == self.warmup_requests {
            for handle in handles.drain(..) {
                if let Err(e) = handle.await {
                    error!("Warmup task join error: {e}");
                    self.join_errors += 1;
                }
            }
            self.shift = run_start.elapsed().saturating_sub(slot.scheduled_delay);
            if let Some(tracker) = &self.tracker {
                tracker.reset_counts();
            }
        }
        self.shifted(slot)
    }

    /// `slot` with the barrier shift applied (identity before the barrier).
    pub fn shifted(&self, slot: RequestSlot) -> RequestSlot {
        RequestSlot {
            seq: slot.seq,
            scheduled_delay: slot.scheduled_delay.saturating_add(self.shift),
        }
    }

    /// Warmup tasks that panicked or were cancelled while the barrier awaited
    /// them; callers add these to their join-error count.
    pub fn join_errors(&self) -> usize {
        self.join_errors
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

    fn slot(seq: u64, ms: u64) -> RequestSlot {
        RequestSlot {
            seq,
            scheduled_delay: Duration::from_millis(ms),
        }
    }

    #[test]
    fn first_signal_after_internal_stop_drains() {
        // #227: a telemetry abort stops the run without a signal.
        let flag = StopFlag::new();
        flag.clone().stop();
        assert!(flag.is_stopped());
        assert_eq!(flag.on_signal(), SignalAction::Drain);
        assert_eq!(flag.on_signal(), SignalAction::HardExit);
    }

    #[test]
    fn second_real_signal_hard_exits_across_clones() {
        let flag = StopFlag::new();
        let other = flag.clone();
        assert_eq!(flag.on_signal(), SignalAction::Drain);
        assert!(other.is_stopped());
        assert_eq!(other.on_signal(), SignalAction::HardExit);
    }

    #[tokio::test]
    async fn warmup_barrier_awaits_warmup_tasks_before_first_measured_slot() {
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let mut handles = vec![tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            flag.store(true, Ordering::SeqCst);
        })];
        let mut barrier = WarmupBarrier::new(1);
        let run_start = Instant::now();
        let measured = barrier
            .before_slot(slot(1, 0), &mut handles, run_start)
            .await;
        assert!(done.load(Ordering::SeqCst), "warmup must finish first");
        assert!(handles.is_empty(), "warmup handles are consumed");
        assert!(measured.scheduled_delay >= Duration::from_millis(30));
        assert_eq!(barrier.join_errors(), 0);
    }

    #[tokio::test]
    async fn warmup_barrier_keeps_open_loop_gaps_after_shift() {
        let mut handles: Vec<JoinHandle<()>> = vec![tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(20)).await;
        })];
        let mut barrier = WarmupBarrier::new(1);
        let run_start = Instant::now();
        let first = barrier
            .before_slot(slot(1, 5), &mut handles, run_start)
            .await;
        let second = barrier
            .before_slot(slot(2, 105), &mut handles, run_start)
            .await;
        assert_eq!(
            second.scheduled_delay - first.scheduled_delay,
            Duration::from_millis(100)
        );
        assert!(first.scheduled_delay >= Duration::from_millis(20));
    }

    #[tokio::test]
    async fn warmup_barrier_is_identity_without_warmup() {
        let mut handles: Vec<JoinHandle<()>> = vec![tokio::spawn(async {})];
        let mut barrier = WarmupBarrier::new(0);
        let out = barrier
            .before_slot(slot(0, 7), &mut handles, Instant::now())
            .await;
        assert_eq!(out.scheduled_delay, Duration::from_millis(7));
        assert_eq!(handles.len(), 1, "no barrier, no drain");
    }

    #[tokio::test]
    async fn warmup_barrier_counts_join_errors() {
        let mut handles: Vec<JoinHandle<()>> = vec![tokio::spawn(async { panic!("boom") })];
        let mut barrier = WarmupBarrier::new(1);
        barrier
            .before_slot(slot(1, 0), &mut handles, Instant::now())
            .await;
        assert_eq!(barrier.join_errors(), 1);
    }

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
