// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Per-request HTTP phase trace (Metrum AI Bench): connect, DNS, connection
//! reuse, body bytes, chunks, and receive time.
//!
//! A task-local [`ConnectSlot`] collects one request's trace:
//!
//! - [`ConnectTimingLayer`] (reqwest `connector_layer`) times the whole
//!   connector call. When the pool supplies a live connection the connector
//!   is never invoked, so `connect_s` is `0.0` and `connection_reused` is true.
//!   DNS and, for HTTPS, the TLS handshake run inside this one connector call.
//! - [`TimedResolver`] (reqwest `dns_resolver`) times name resolution inside
//!   the connector. It resolves with `getaddrinfo` on a blocking thread, the
//!   same path as reqwest's default resolver. IP-literal hosts skip it.
//! - [`send`] records request body bytes and the response-headers instant.
//! - [`counted`] / [`read_body`] count response body chunks and bytes and the
//!   last-chunk instant, so `receive_s` is headers to body end.
//!
//! TCP connect and TLS handshake are not split: reqwest runs both inside one
//! opaque connector future, so only their sum (`connect_s - dns_s`) is known.

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use pin_project_lite::pin_project;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tower_layer::Layer;
use tower_service::Service;

tokio::task_local! {
    static CONNECT_SLOT: Arc<ConnectSlot>;
}

#[derive(Debug, Default)]
struct TraceState {
    connect_attempted: bool,
    connect: Option<Duration>,
    dns: Option<Duration>,
    bytes_sent: Option<u64>,
    headers_at: Option<Instant>,
    body_end_at: Option<Instant>,
    bytes_received: u64,
    chunks_received: u64,
}

/// Per-request slot written by the connector layer, the resolver, and the
/// send/body helpers in this module.
#[derive(Debug, Default)]
pub struct ConnectSlot {
    state: Mutex<TraceState>,
}

/// Snapshot of one request's HTTP phase trace. See the module docs for what
/// each field measures.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HttpTrace {
    /// Connector seconds; `0.0` when no new connection completed.
    pub connect_s: f64,
    /// True when the connector was not invoked (pooled connection).
    pub connection_reused: bool,
    /// Resolver seconds; `0.0` when no lookup ran (pool hit or IP literal).
    pub dns_s: f64,
    /// Request body bytes; `None` when the body length is unknown.
    pub bytes_sent: Option<u64>,
    /// Response headers to last body chunk; `None` unless the body was read.
    pub receive_s: Option<f64>,
    /// Response body bytes after content decoding; `None` unless the body was read.
    pub bytes_received: Option<u64>,
    /// Response body chunks yielded by the client; `None` unless the body was read.
    pub chunks_received: Option<u64>,
}

impl ConnectSlot {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn with_state<R>(&self, f: impl FnOnce(&mut TraceState) -> R) -> R {
        // A poisoned lock still holds plain counters; keep recording.
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut guard)
    }

    fn record(&self, elapsed: Duration) {
        self.with_state(|s| s.connect = Some(elapsed));
    }

    /// Seconds of connector work, or `0.0` when the pool supplied a live connection.
    pub fn take(&self) -> f64 {
        self.with_state(|s| s.connect.map_or(0.0, |d| d.as_secs_f64()))
    }

    /// Snapshot the trace collected so far.
    pub fn trace(&self) -> HttpTrace {
        self.with_state(|s| {
            let receive_s = match (s.headers_at, s.body_end_at) {
                (Some(headers), Some(end)) => {
                    Some(end.saturating_duration_since(headers).as_secs_f64())
                }
                _ => None,
            };
            let body_read = s.body_end_at.is_some();
            HttpTrace {
                connect_s: s.connect.map_or(0.0, |d| d.as_secs_f64()),
                connection_reused: !s.connect_attempted,
                dns_s: s.dns.map_or(0.0, |d| d.as_secs_f64()),
                bytes_sent: s.bytes_sent,
                receive_s,
                bytes_received: body_read.then_some(s.bytes_received),
                chunks_received: body_read.then_some(s.chunks_received),
            }
        })
    }
}

fn current_slot() -> Option<Arc<ConnectSlot>> {
    CONNECT_SLOT.try_with(Arc::clone).ok()
}

/// Run `fut` with a connect slot visible to [`ConnectTimingLayer`] and the helpers.
pub async fn with_connect_slot<F, T>(slot: Arc<ConnectSlot>, fut: F) -> T
where
    F: Future<Output = T>,
{
    CONNECT_SLOT.scope(slot, fut).await
}

/// Build and send `builder`, recording request body bytes and the instant the
/// response headers arrive. Body bytes come from an in-memory body, else from
/// the `Content-Length` header (multipart forms with known part sizes).
pub async fn send(builder: reqwest::RequestBuilder) -> reqwest::Result<reqwest::Response> {
    let (client, request) = builder.build_split();
    let request = request?;
    let bytes_sent = request
        .body()
        .and_then(|body| body.as_bytes())
        .map(|body| body.len() as u64)
        .or_else(|| {
            request
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse().ok())
        });
    let slot = current_slot();
    if let Some(slot) = &slot {
        slot.with_state(|s| s.bytes_sent = bytes_sent);
    }
    let response = client.execute(request).await?;
    if let Some(slot) = &slot {
        slot.with_state(|s| s.headers_at = Some(Instant::now()));
    }
    Ok(response)
}

pin_project! {
    /// Body stream that counts chunks and bytes into the task-local slot.
    pub struct Counted<S> {
        #[pin]
        inner: S,
        slot: Option<Arc<ConnectSlot>>,
    }
}

impl<S, B, E> Stream for Counted<S>
where
    S: Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
{
    type Item = Result<B, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        let polled = this.inner.poll_next(cx);
        if let (Poll::Ready(item), Some(slot)) = (&polled, this.slot.as_ref()) {
            let now = Instant::now();
            match item {
                Some(Ok(chunk)) => slot.with_state(|s| {
                    s.chunks_received += 1;
                    s.bytes_received += chunk.as_ref().len() as u64;
                    s.body_end_at = Some(now);
                }),
                None => slot.with_state(|s| s.body_end_at = Some(now)),
                Some(Err(_)) => {}
            }
        }
        polled
    }
}

/// Wrap a response body stream so chunks, bytes, and body end are traced.
pub fn counted<S>(stream: S) -> Counted<S> {
    Counted {
        inner: stream,
        slot: current_slot(),
    }
}

/// Read a whole response body through [`counted`].
pub async fn read_body(response: reqwest::Response) -> reqwest::Result<Bytes> {
    let stream = counted(response.bytes_stream());
    futures_util::pin_mut!(stream);
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk?);
    }
    Ok(Bytes::from(body))
}

/// DNS resolver that times each lookup into the task-local slot. Resolution
/// is `getaddrinfo` on a blocking thread, as in reqwest's default resolver.
#[derive(Clone, Debug, Default)]
pub struct TimedResolver;

impl reqwest::dns::Resolve for TimedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let slot = current_slot();
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let start = Instant::now();
            let addrs: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            if let Some(slot) = slot {
                let elapsed = start.elapsed();
                slot.with_state(|s| s.dns = Some(s.dns.unwrap_or_default() + elapsed));
            }
            let addrs: reqwest::dns::Addrs = Box::new(addrs.into_iter());
            Ok(addrs)
        })
    }
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
        let slot = current_slot();
        if let Some(slot) = &slot {
            slot.with_state(|s| s.connect_attempted = true);
        }
        ConnectTimingFuture {
            inner: self.inner.call(req),
            start: Instant::now(),
            slot,
            recorded: false,
        }
    }
}

pin_project! {
    /// Future that records connector elapsed time into the task-local slot on success.
    pub struct ConnectTimingFuture<F> {
        #[pin]
        inner: F,
        start: Instant,
        slot: Option<Arc<ConnectSlot>>,
        recorded: bool,
    }
}

impl<F, T, E> Future for ConnectTimingFuture<F>
where
    F: Future<Output = Result<T, E>>,
{
    type Output = Result<T, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.inner.poll(cx) {
            Poll::Ready(Ok(value)) => {
                if !*this.recorded {
                    *this.recorded = true;
                    if let Some(slot) = this.slot {
                        slot.record(this.start.elapsed());
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
        let trace = slot.trace();
        assert!(trace.connection_reused);
        assert_eq!(trace.dns_s, 0.0);
        assert_eq!(trace.receive_s, None);
        assert_eq!(trace.bytes_received, None);
        assert_eq!(trace.chunks_received, None);
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

    #[tokio::test]
    async fn connector_layer_marks_fresh_connection() {
        let slot = ConnectSlot::new();
        let mut service =
            ConnectTimingLayer.layer(tower_service_fn(|()| async { Ok::<_, ()>(()) }));
        with_connect_slot(Arc::clone(&slot), async {
            service.call(()).await.expect("connect");
        })
        .await;
        assert!(!slot.trace().connection_reused);
    }

    #[tokio::test]
    async fn failed_connect_is_not_reuse() {
        let slot = ConnectSlot::new();
        let mut service =
            ConnectTimingLayer.layer(tower_service_fn(|()| async { Err::<(), _>("refused") }));
        with_connect_slot(Arc::clone(&slot), async {
            assert!(service.call(()).await.is_err());
        })
        .await;
        let trace = slot.trace();
        assert!(!trace.connection_reused);
        assert_eq!(trace.connect_s, 0.0);
    }

    #[tokio::test]
    async fn counted_stream_tracks_chunks_bytes_and_receive() {
        let slot = ConnectSlot::new();
        with_connect_slot(Arc::clone(&slot), async {
            CONNECT_SLOT.with(|s| s.with_state(|st| st.headers_at = Some(Instant::now())));
            let chunks: Vec<Result<Bytes, ()>> = vec![
                Ok(Bytes::from_static(b"abc")),
                Ok(Bytes::from_static(b"de")),
            ];
            let stream = counted(futures_util::stream::iter(chunks));
            futures_util::pin_mut!(stream);
            while stream.next().await.is_some() {}
        })
        .await;
        let trace = slot.trace();
        assert_eq!(trace.chunks_received, Some(2));
        assert_eq!(trace.bytes_received, Some(5));
        assert!(trace.receive_s.expect("receive") >= 0.0);
    }

    #[tokio::test]
    async fn empty_body_reports_zero_chunks() {
        let slot = ConnectSlot::new();
        with_connect_slot(Arc::clone(&slot), async {
            let stream = counted(futures_util::stream::iter(Vec::<Result<Bytes, ()>>::new()));
            futures_util::pin_mut!(stream);
            assert!(stream.next().await.is_none());
        })
        .await;
        let trace = slot.trace();
        assert_eq!(trace.chunks_received, Some(0));
        assert_eq!(trace.bytes_received, Some(0));
        // Headers were never stamped, so receive time is unknown.
        assert_eq!(trace.receive_s, None);
    }

    #[tokio::test]
    async fn resolver_records_dns_time() {
        let slot = ConnectSlot::new();
        let addrs = with_connect_slot(Arc::clone(&slot), async {
            let name: reqwest::dns::Name = "localhost".parse().expect("name");
            reqwest::dns::Resolve::resolve(&TimedResolver, name)
                .await
                .expect("resolve")
                .count()
        })
        .await;
        assert!(addrs > 0);
        assert!(slot.trace().dns_s > 0.0);
    }

    /// Minimal `Service` from a closure for connector-layer tests.
    fn tower_service_fn<F, Fut, T, E>(f: F) -> ServiceFn<F>
    where
        F: FnMut(()) -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        ServiceFn(f)
    }

    struct ServiceFn<F>(F);

    impl<F, Fut, T, E> Service<()> for ServiceFn<F>
    where
        F: FnMut(()) -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        type Response = T;
        type Error = E;
        type Future = Fut;

        fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), E>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: ()) -> Fut {
            (self.0)(req)
        }
    }
}
