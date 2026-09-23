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
    use tokio::sync::Semaphore;

    #[tokio::test]
    async fn tracks_occupancy_and_cap_engagement() {
        let tracker = Arc::new(InFlightTracker::new(2));
        let sem = Arc::new(Semaphore::new(2));
        let p1 = acquire_with_engagement(Arc::clone(&sem), &tracker)
            .await
            .unwrap();
        let g1 = tracker.guard();
        assert_eq!(g1.in_flight, 1);
        let p2 = acquire_with_engagement(Arc::clone(&sem), &tracker)
            .await
            .unwrap();
        let g2 = tracker.guard();
        assert_eq!(g2.in_flight, 2);
        // Cap saturated: next acquire waits.
        let sem3 = Arc::clone(&sem);
        let tracker3 = Arc::clone(&tracker);
        let waiter = tokio::spawn(async move { acquire_with_engagement(sem3, &tracker3).await });
        tokio::task::yield_now().await;
        drop(p1);
        drop(g1);
        let p3 = waiter.await.unwrap().unwrap();
        let snap = tracker.snapshot();
        assert_eq!(snap.cap, 2);
        assert_eq!(snap.in_flight_max, Some(2.0));
        assert_eq!(snap.wait_count, 1);
        assert_eq!(snap.acquire_count, 3);
        assert!((snap.cap_engagement_fraction.unwrap() - 1.0 / 3.0).abs() < 1e-12);
        drop(p2);
        drop(g2);
        drop(p3);
    }
}
