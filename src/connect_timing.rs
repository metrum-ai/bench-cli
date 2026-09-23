// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Per-request TCP/TLS connect timing via reqwest `connector_layer`.
//!
//! When the HTTP client reuses a pooled connection the connector is not called
//! and [`ConnectSlot::take`] reports `0.0` (pool hit). A fresh connect records
//! the full connector duration (DNS is inside the connector; TLS is included
//! for HTTPS).

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tower_layer::Layer;
use tower_service::Service;

tokio::task_local! {
    static CONNECT_SLOT: Arc<ConnectSlot>;
}

/// Per-request slot written by the connector layer when a new connection is made.
#[derive(Debug, Default)]
pub struct ConnectSlot {
    called: AtomicBool,
    ns: AtomicU64,
}

impl ConnectSlot {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn record(&self, elapsed: Duration) {
        self.called.store(true, Ordering::Release);
        self.ns.store(
            elapsed.as_nanos().min(u128::from(u64::MAX)) as u64,
            Ordering::Release,
        );
    }

    /// Seconds of connector work, or `0.0` when the pool supplied a live connection.
    pub fn take(&self) -> f64 {
        if self.called.load(Ordering::Acquire) {
            self.ns.load(Ordering::Acquire) as f64 / 1_000_000_000.0
        } else {
            0.0
        }
    }
}

/// Run `fut` with a connect slot visible to [`ConnectTimingLayer`].
pub async fn with_connect_slot<F, T>(slot: Arc<ConnectSlot>, fut: F) -> T
where
    F: Future<Output = T>,
{
    CONNECT_SLOT.scope(slot, fut).await
}

/// Tower layer that times each connector `call` into the task-local [`ConnectSlot`].
#[derive(Clone, Debug, Default)]
pub struct ConnectTimingLayer;

impl<S> Layer<S> for ConnectTimingLayer {
    type Service = ConnectTimingService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ConnectTimingService { inner }
    }
}

#[derive(Clone, Debug)]
pub struct ConnectTimingService<S> {
    inner: S,
}

impl<S, Request> Service<Request> for ConnectTimingService<S>
where
    S: Service<Request>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ConnectTimingFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let slot = CONNECT_SLOT.try_with(Arc::clone).ok();
        ConnectTimingFuture {
            inner: self.inner.call(req),
            start: Instant::now(),
            slot,
            recorded: false,
        }
    }
}

/// Future that records connector elapsed time into the task-local slot on success.
pub struct ConnectTimingFuture<F> {
    inner: F,
    start: Instant,
    slot: Option<Arc<ConnectSlot>>,
    recorded: bool,
}

impl<F, T, E> Future for ConnectTimingFuture<F>
where
    F: Future<Output = Result<T, E>>,
{
    type Output = Result<T, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Safety: we never move `inner` after pinning; only other fields are plain data.
        let (inner, start, slot, recorded) = unsafe {
            let this = self.get_unchecked_mut();
            (
                Pin::new_unchecked(&mut this.inner),
                this.start,
                &this.slot,
                &mut this.recorded,
            )
        };
        match inner.poll(cx) {
            Poll::Ready(Ok(value)) => {
                if !*recorded {
                    *recorded = true;
                    if let Some(slot) = slot {
                        slot.record(start.elapsed());
                    }
                }
                Poll::Ready(Ok(value))
            }
            Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pool_hit_reports_zero_without_connector_call() {
        let slot = ConnectSlot::new();
        let observed = with_connect_slot(Arc::clone(&slot), async { slot.take() }).await;
        assert_eq!(observed, 0.0);
    }

    #[tokio::test]
    async fn connector_call_records_elapsed() {
        let slot = ConnectSlot::new();
        with_connect_slot(Arc::clone(&slot), async {
            CONNECT_SLOT.with(|s| s.record(Duration::from_millis(12)));
        })
        .await;
        let secs = slot.take();
        assert!((secs - 0.012).abs() < 1e-6, "got {secs}");
    }
}
