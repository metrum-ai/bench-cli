// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Client-side observed concurrency: semaphore occupancy and cap engagement.

use crate::stats::DistSummary;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Live outstanding-request gauge plus acquire/wait counters for a run or stage.
#[derive(Debug, Default)]
pub struct InFlightTracker {
    current: AtomicU64,
    max: AtomicU64,
    samples: Mutex<Vec<f64>>,
    acquires: AtomicU64,
    waits: AtomicU64,
    cap: u32,
}

/// Mean / p50 / max in-flight and fraction of acquires that blocked on the cap.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ObservedConcurrency {
    /// Configured outstanding-request cap for this tracker.
    pub cap: u32,
    pub in_flight_mean: Option<f64>,
    pub in_flight_p50: Option<f64>,
    pub in_flight_max: Option<f64>,
    /// Fraction of acquires that found the semaphore exhausted (`try_acquire` miss).
    pub cap_engagement_fraction: Option<f64>,
    pub acquire_count: u64,
    pub wait_count: u64,
}

impl InFlightTracker {
    pub fn new(cap: u32) -> Self {
        Self {
            current: AtomicU64::new(0),
            max: AtomicU64::new(0),
            samples: Mutex::new(Vec::new()),
            acquires: AtomicU64::new(0),
            waits: AtomicU64::new(0),
            cap: cap.max(1),
        }
    }

    pub fn cap(&self) -> u32 {
        self.cap
    }

    /// Requests currently inside the gauge.
    pub fn current(&self) -> u64 {
        self.current.load(Ordering::Acquire)
    }

    /// Record whether this acquire blocked on the semaphore (cap engagement).
    pub fn note_acquire(&self, waited: bool) {
        self.acquires.fetch_add(1, Ordering::Relaxed);
        if waited {
            self.waits.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Increment in-flight, sample occupancy, return the post-increment count.
    pub fn enter(&self) -> u64 {
        let now = self.current.fetch_add(1, Ordering::AcqRel) + 1;
        self.max.fetch_max(now, Ordering::Relaxed);
        if let Ok(mut samples) = self.samples.lock() {
            samples.push(now as f64);
        }
        now
    }

    /// Decrement in-flight after a request completes (or is cancelled).
    pub fn leave(&self) {
        let prev = self.current.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(prev > 0, "InFlightTracker::leave underflow");
    }

    /// RAII guard that enters on create and leaves on drop.
    pub fn guard(self: &Arc<Self>) -> InFlightGuard {
        let in_flight = self.enter();
        InFlightGuard {
            tracker: Arc::clone(self),
            in_flight,
        }
    }

    /// Clear occupancy samples, the max gauge and the acquire/wait counters
    /// so a later [`Self::snapshot`] covers only what follows, such as the
    /// measured phase after the warmup barrier (#226). Requests still inside
    /// the gauge keep counting in `current`; after the barrier that is zero.
    pub fn reset_counts(&self) {
        if let Ok(mut samples) = self.samples.lock() {
            samples.clear();
        }
        self.max.store(self.current(), Ordering::Relaxed);
        self.acquires.store(0, Ordering::Relaxed);
        self.waits.store(0, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> ObservedConcurrency {
        let samples = self.samples.lock().map(|g| g.clone()).unwrap_or_default();
        let dist = DistSummary::from_values(&samples);
        let acquires = self.acquires.load(Ordering::Relaxed);
        let waits = self.waits.load(Ordering::Relaxed);
        let cap_engagement_fraction = if acquires == 0 {
            None
        } else {
            Some(waits as f64 / acquires as f64)
        };
        ObservedConcurrency {
            cap: self.cap,
            in_flight_mean: dist.avg,
            in_flight_p50: dist.p50,
            in_flight_max: {
                let m = self.max.load(Ordering::Relaxed);
                if m == 0 && samples.is_empty() {
                    None
                } else {
                    Some(m as f64)
                }
            },
            cap_engagement_fraction,
            acquire_count: acquires,
            wait_count: waits,
        }
    }
}

/// Holds one in-flight slot for the lifetime of a request task.
pub struct InFlightGuard {
    tracker: Arc<InFlightTracker>,
    pub in_flight: u64,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.tracker.leave();
    }
}

/// One request's hold on the cap: its in-flight gauge entry plus its permit.
///
/// `guard` is declared before `permit`, and struct fields drop in declaration
/// order, so the gauge is always left before the permit is freed: on a normal
/// drop, an early return, a cancelled task, or a panic unwinding. Freeing the
/// permit first would let the next waiter acquire it and enter the gauge while
/// this request still counts, reading cap+1 and biasing
/// `in_flight_{mean,p50,max}` upward (#189).
///
/// `P` is generic only so tests can observe drop order; binaries use the
/// default [`tokio::sync::OwnedSemaphorePermit`].
#[must_use = "dropping the slot releases the concurrency permit"]
pub struct InFlightSlot<P = tokio::sync::OwnedSemaphorePermit> {
    guard: InFlightGuard,
    _permit: P,
}

impl<P> InFlightSlot<P> {
    /// Enter the gauge for a request that already holds `permit`.
    pub fn new(tracker: &Arc<InFlightTracker>, permit: P) -> Self {
        Self {
            guard: tracker.guard(),
            _permit: permit,
        }
    }

    /// Outstanding count including this request, sampled when it entered.
    pub fn in_flight(&self) -> u64 {
        self.guard.in_flight
    }
}

/// Acquire a permit, counting cap engagement when `try_acquire` misses.
pub async fn acquire_with_engagement(
    semaphore: Arc<tokio::sync::Semaphore>,
    tracker: &InFlightTracker,
) -> Result<tokio::sync::OwnedSemaphorePermit, tokio::sync::AcquireError> {
    match Arc::clone(&semaphore).try_acquire_owned() {
        Ok(permit) => {
            tracker.note_acquire(false);
            Ok(permit)
        }
        Err(_) => {
            let permit = semaphore.acquire_owned().await?;
            tracker.note_acquire(true);
            Ok(permit)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::{OwnedSemaphorePermit, Semaphore};

    /// #226: after the warmup barrier the snapshot covers measured slots only.
    #[tokio::test]
    async fn reset_counts_drops_earlier_acquires_and_samples() {
        let tracker = Arc::new(InFlightTracker::new(1));
        let sem = Arc::new(Semaphore::new(1));
        let warm = acquire_with_engagement(Arc::clone(&sem), &tracker)
            .await
            .unwrap();
        drop(InFlightSlot::new(&tracker, warm));
        tracker.reset_counts();
        let empty = tracker.snapshot();
        assert_eq!(empty.acquire_count, 0);
        assert_eq!(empty.in_flight_max, None);
        assert_eq!(empty.cap_engagement_fraction, None);
        let measured = acquire_with_engagement(Arc::clone(&sem), &tracker)
            .await
            .unwrap();
        let slot = InFlightSlot::new(&tracker, measured);
        let snap = tracker.snapshot();
        assert_eq!(snap.acquire_count, 1);
        assert_eq!(snap.in_flight_max, Some(1.0));
        drop(slot);
    }

    #[tokio::test]
    async fn tracks_occupancy_and_cap_engagement() {
        let tracker = Arc::new(InFlightTracker::new(2));
        let sem = Arc::new(Semaphore::new(2));
        let p1 = acquire_with_engagement(Arc::clone(&sem), &tracker)
            .await
            .unwrap();
        let s1 = InFlightSlot::new(&tracker, p1);
        assert_eq!(s1.in_flight(), 1);
        let p2 = acquire_with_engagement(Arc::clone(&sem), &tracker)
            .await
            .unwrap();
        let s2 = InFlightSlot::new(&tracker, p2);
        assert_eq!(s2.in_flight(), 2);
        // Cap saturated: next acquire waits.
        let sem3 = Arc::clone(&sem);
        let tracker3 = Arc::clone(&tracker);
        let waiter = tokio::spawn(async move { acquire_with_engagement(sem3, &tracker3).await });
        tokio::task::yield_now().await;
        drop(s1);
        let p3 = waiter.await.unwrap().unwrap();
        let snap = tracker.snapshot();
        assert_eq!(snap.cap, 2);
        assert_eq!(snap.in_flight_max, Some(2.0));
        assert_eq!(snap.wait_count, 1);
        assert_eq!(snap.acquire_count, 3);
        assert!((snap.cap_engagement_fraction.unwrap() - 1.0 / 3.0).abs() < 1e-12);
        drop(s2);
        drop(p3);
    }

    /// Waiter that acquires a permit and enters the gauge, like a request task.
    fn spawn_request(
        sem: &Arc<Semaphore>,
        tracker: &Arc<InFlightTracker>,
    ) -> tokio::task::JoinHandle<InFlightSlot> {
        let sem = Arc::clone(sem);
        let tracker = Arc::clone(tracker);
        tokio::spawn(async move {
            let permit = acquire_with_engagement(sem, &tracker).await.unwrap();
            InFlightSlot::new(&tracker, permit)
        })
    }

    #[tokio::test]
    async fn request_slot_keeps_occupancy_within_cap() {
        for cap in [1u32, 4] {
            let tracker = Arc::new(InFlightTracker::new(cap));
            let sem = Arc::new(Semaphore::new(cap as usize));
            let mut held = Vec::new();
            for _ in 0..cap {
                held.push(spawn_request(&sem, &tracker).await.unwrap());
            }
            // Every slot is busy; the next request waits on the semaphore.
            let waiter = spawn_request(&sem, &tracker);
            tokio::task::yield_now().await;
            for slot in held {
                drop(slot);
                // Let the woken waiter run before anything else is released.
                tokio::task::yield_now().await;
            }
            drop(waiter.await.unwrap());
            let snap = tracker.snapshot();
            let cap = f64::from(cap);
            assert_eq!(snap.in_flight_max, Some(cap));
            assert!(snap.in_flight_mean.unwrap() <= cap);
            assert_eq!(snap.wait_count, 1);
            assert_eq!(tracker.current(), 0);
        }
    }

    /// Documents the #189 hazard: permit first lets the waiter enter at cap+1.
    #[tokio::test]
    async fn releasing_permit_before_guard_overcounts() {
        let tracker = Arc::new(InFlightTracker::new(1));
        let sem = Arc::new(Semaphore::new(1));
        let permit: OwnedSemaphorePermit = Arc::clone(&sem).acquire_owned().await.unwrap();
        let guard = tracker.guard();
        let waiter = spawn_request(&sem, &tracker);
        tokio::task::yield_now().await;
        drop(permit);
        tokio::task::yield_now().await;
        drop(guard);
        drop(waiter.await.unwrap());
        assert_eq!(tracker.snapshot().in_flight_max, Some(2.0));
    }

    /// Stand-in permit that records the gauge at the moment it is released.
    struct ProbePermit {
        tracker: Arc<InFlightTracker>,
        seen_at_release: Arc<Mutex<Option<u64>>>,
    }

    impl Drop for ProbePermit {
        fn drop(&mut self) {
            *self.seen_at_release.lock().unwrap() = Some(self.tracker.current());
        }
    }

    fn probe_slot(
        tracker: &Arc<InFlightTracker>,
    ) -> (InFlightSlot<ProbePermit>, Arc<Mutex<Option<u64>>>) {
        let seen = Arc::new(Mutex::new(None));
        let permit = ProbePermit {
            tracker: Arc::clone(tracker),
            seen_at_release: Arc::clone(&seen),
        };
        (InFlightSlot::new(tracker, permit), seen)
    }

    #[test]
    fn request_slot_leaves_gauge_before_releasing_permit() {
        let tracker = Arc::new(InFlightTracker::new(1));
        let (slot, seen) = probe_slot(&tracker);
        assert_eq!(tracker.current(), 1);
        drop(slot);
        assert_eq!(*seen.lock().unwrap(), Some(0));
    }

    #[test]
    fn request_slot_leaves_gauge_before_releasing_permit_on_panic() {
        let tracker = Arc::new(InFlightTracker::new(1));
        let (slot, seen) = probe_slot(&tracker);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _slot = slot;
            panic!("request task panicked");
        }));
        assert!(result.is_err());
        assert_eq!(*seen.lock().unwrap(), Some(0));
        assert_eq!(tracker.current(), 0);
    }
}
