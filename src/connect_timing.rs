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
//!   Connector or resolver work that finishes after the response headers (a
//!   background connect that lost the race to a pooled connection) is not
//!   booked to the request. Resolver time is held as pending and becomes
//!   `dns_s` only when its connect completes before the headers.
//! - [`TimedResolver`] (reqwest `dns_resolver`) times name resolution inside
//!   the connector. It resolves with `getaddrinfo` on a blocking thread, the
//!   same path as reqwest's default resolver. IP-literal hosts skip it.
//! - [`send`] records request body bytes and the response-headers instant.
//! - [`counted`] / [`read_body`] count response body chunks and bytes and the
//!   body-end instant, so `receive_s` is headers to body end. Counting is
//!   lock-free per chunk; the slot is written once at body end or on drop.
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
    /// A connector call finished before the response headers arrived.
    connect_completed: bool,
    connect: Option<Duration>,
    dns: Option<Duration>,
    /// Resolver time not yet tied to a connect that carried this request.
    pending_dns: Option<Duration>,
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
    /// True when no connector call finished before the response headers
    /// (pooled connection). Without headers, true only if none was attempted.
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

    /// Add connector time. Work that finishes after the response headers did
    /// not carry this request (a connect that lost the race to a pooled
    /// connection), so it is not booked here. A connect that finishes first
    /// also books the resolver time held as pending.
    fn record(&self, elapsed: Duration) {
        self.with_state(|s| {
            if s.headers_at.is_none() {
                s.connect_completed = true;
                s.connect = Some(s.connect.unwrap_or_default() + elapsed);
                if let Some(pending) = s.pending_dns.take() {
                    s.dns = Some(s.dns.unwrap_or_default() + pending);
                }
            }
        });
    }

    /// Hold resolver time as pending. It becomes `dns_s` only when its connect
    /// completes before the headers (see [`Self::record`]), so a losing
    /// background lookup never shows `dns_s > 0` on a pooled request. A
    /// request that never got headers (lookup or connect failed) keeps it.
    fn record_dns(&self, elapsed: Duration) {
        self.with_state(|s| {
            if s.headers_at.is_none() {
                s.pending_dns = Some(s.pending_dns.unwrap_or_default() + elapsed);
            }
        });
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
                // With headers, only a connect that finished first can have
                // carried the request; without them, any attempt counts.
                connection_reused: if s.headers_at.is_some() {
                    !s.connect_completed
                } else {
                    !s.connect_attempted
                },
                // Without headers the request failed; its lookups were its own.
                dns_s: (s.dns.unwrap_or_default()
                    + s.headers_at
                        .map_or(s.pending_dns, |_| None)
                        .unwrap_or_default())
                .as_secs_f64(),
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
    /// Body stream that counts chunks and bytes, then writes them to the
    /// task-local slot once, at body end or on drop. Counting stays in plain
    /// fields so the per-chunk (per-token, for SSE) path takes no lock.
    pub struct Counted<S> {
        #[pin]
        inner: S,
        slot: Option<Arc<ConnectSlot>>,
        chunks: u64,
        bytes: u64,
        last_at: Option<Instant>,
        flushed: bool,
    }

    impl<S> PinnedDrop for Counted<S> {
        fn drop(this: Pin<&mut Self>) {
            let this = this.project();
            // Dropped before the body end: book what was read, ending at the last chunk.
            if let Some(end) = *this.last_at {
                flush_body(this.slot, this.flushed, *this.chunks, *this.bytes, end);
            }
        }
    }
}

/// Write body counters into the slot once.
fn flush_body(
    slot: &Option<Arc<ConnectSlot>>,
    flushed: &mut bool,
    chunks: u64,
    bytes: u64,
    end: Instant,
) {
    if std::mem::replace(flushed, true) {
        return;
    }
    if let Some(slot) = slot {
        slot.with_state(|s| {
            s.chunks_received += chunks;
            s.bytes_received += bytes;
            s.body_end_at = Some(end);
        });
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
        if this.slot.is_some() {
            match &polled {
                Poll::Ready(Some(Ok(chunk))) => {
                    *this.chunks += 1;
                    *this.bytes += chunk.as_ref().len() as u64;
                    *this.last_at = Some(Instant::now());
                }
                Poll::Ready(None) => flush_body(
                    this.slot,
                    this.flushed,
                    *this.chunks,
                    *this.bytes,
                    Instant::now(),
                ),
                Poll::Ready(Some(Err(_))) | Poll::Pending => {}
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
        chunks: 0,
        bytes: 0,
        last_at: None,
        flushed: false,
    }
}

/// Read a whole response body through [`counted`].
pub async fn read_body(response: reqwest::Response) -> reqwest::Result<Bytes> {
    // Preallocate from Content-Length (capped) so large bodies are not
    // regrown inside the timed window.
    let capacity = response
        .content_length()
        .map_or(0, |len| len.min(64 << 20) as usize);
    let stream = counted(response.bytes_stream());
    futures_util::pin_mut!(stream);
    let mut body = Vec::with_capacity(capacity);
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
            // The guard records on drop, so a failed lookup and one cancelled
            // by `connect_timeout` both keep their time.
            let timer = DnsTimer {
                slot,
                start: Instant::now(),
            };
            let resolved = tokio::net::lookup_host((host.as_str(), 0)).await;
            drop(timer);
            let addrs: Vec<SocketAddr> = resolved?.collect();
            let addrs: reqwest::dns::Addrs = Box::new(addrs.into_iter());
            Ok(addrs)
        })
    }
}

/// Records resolver elapsed time into the slot when dropped.
struct DnsTimer {
    slot: Option<Arc<ConnectSlot>>,
    start: Instant,
}

impl Drop for DnsTimer {
    fn drop(&mut self) {
        if let Some(slot) = &self.slot {
            slot.record_dns(self.start.elapsed());
        }
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
    async fn late_connect_after_headers_is_not_booked() {
        let slot = ConnectSlot::new();
        slot.record_dns(Duration::from_millis(2));
        slot.record(Duration::from_millis(5));
        slot.record(Duration::from_millis(7));
        slot.with_state(|s| {
            s.connect_attempted = true;
            s.headers_at = Some(Instant::now());
        });
        // A losing background connect finishing after the headers.
        slot.record_dns(Duration::from_millis(40));
        slot.record(Duration::from_millis(50));
        let trace = slot.trace();
        assert!(
            (trace.connect_s - 0.012).abs() < 1e-9,
            "{}",
            trace.connect_s
        );
        assert!((trace.dns_s - 0.002).abs() < 1e-9, "{}", trace.dns_s);
        assert!(trace.dns_s <= trace.connect_s);
        assert!(!trace.connection_reused);

        // Only late work: the request rode a pooled connection.
        let pooled = ConnectSlot::new();
        pooled.with_state(|s| {
            s.connect_attempted = true;
            s.headers_at = Some(Instant::now());
        });
        pooled.record(Duration::from_millis(50));
        let trace = pooled.trace();
        assert!(trace.connection_reused);
        assert_eq!(trace.connect_s, 0.0);
    }

    /// PR #222 review: a lookup whose connect loses the race to a pooled
    /// connection (connect completes after the headers) books no DNS time.
    #[tokio::test]
    async fn losing_background_lookup_books_no_dns() {
        let slot = ConnectSlot::new();
        slot.with_state(|s| s.connect_attempted = true);
        slot.record_dns(Duration::from_millis(3));
        slot.with_state(|s| s.headers_at = Some(Instant::now()));
        slot.record(Duration::from_millis(9));
        let trace = slot.trace();
        assert_eq!(trace.dns_s, 0.0);
        assert_eq!(trace.connect_s, 0.0);
        assert!(trace.connection_reused);
    }

    /// PR #222 review: a lookup cancelled by `connect_timeout` keeps its time.
    #[tokio::test]
    async fn cancelled_lookup_keeps_dns_time() {
        let slot = ConnectSlot::new();
        with_connect_slot(Arc::clone(&slot), async {
            let timer = DnsTimer {
                slot: current_slot(),
                start: Instant::now() - Duration::from_millis(5),
            };
            let lookup = async move {
                let _timer = timer;
                std::future::pending::<()>().await;
            };
            let timed_out = tokio::time::timeout(Duration::from_millis(1), lookup).await;
            assert!(timed_out.is_err());
        })
        .await;
        assert!(slot.trace().dns_s >= 0.005, "{}", slot.trace().dns_s);
    }

    #[tokio::test]
    async fn failed_lookup_keeps_dns_time() {
        let slot = ConnectSlot::new();
        let result = with_connect_slot(Arc::clone(&slot), async {
            let name: reqwest::dns::Name = "no-such-host.invalid".parse().expect("name");
            reqwest::dns::Resolve::resolve(&TimedResolver, name).await
        })
        .await;
        assert!(result.is_err());
        assert!(slot.trace().dns_s > 0.0);
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

    /// A body dropped before its end books the chunks read so far.
    #[tokio::test]
    async fn dropped_stream_books_chunks_read() {
        let slot = ConnectSlot::new();
        with_connect_slot(Arc::clone(&slot), async {
            let chunks: Vec<Result<Bytes, ()>> = vec![
                Ok(Bytes::from_static(b"abc")),
                Ok(Bytes::from_static(b"de")),
            ];
            let stream = counted(futures_util::stream::iter(chunks));
            futures_util::pin_mut!(stream);
            assert!(stream.next().await.is_some());
        })
        .await;
        let trace = slot.trace();
        assert_eq!(trace.chunks_received, Some(1));
        assert_eq!(trace.bytes_received, Some(3));
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
