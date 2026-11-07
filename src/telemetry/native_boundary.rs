// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Strict opt-in acquisition for the existing boundary GET, never a sampler.
use super::{
    config::{CompiledSource, TelemetryConfig},
    epoch::RunEpoch,
};
use crate::cache_metrics::{probe, Probe, ProfileId};
use std::time::Duration;

// Private binding/result deliberately do not implement Debug or Serialize.
pub struct NativeBoundaryConfig {
    pub profile: ProfileId,
    pub source_revision: Option<String>,
}
pub struct NativeBoundaryCapture {
    pub fetch_start_ns: u64,
    pub fetch_end_ns: u64,
    pub probe: Option<Probe>,
    pub failure: Option<NativeBoundaryFailure>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeBoundaryFailure {
    InvalidUrl,
    UnsupportedTransport,
    Fetch,
    Timeout,
    HttpStatus,
    TooLarge,
    InvalidUtf8,
    Parse,
    Write,
}
pub(super) struct NativeBody {
    pub text: String,
    pub capture: NativeBoundaryCapture,
}

pub(super) async fn fetch(
    cfg: &TelemetryConfig,
    src: &CompiledSource,
    epoch: &RunEpoch,
    tag: &str,
    options: &NativeBoundaryConfig,
) -> Result<NativeBody, Box<NativeBoundaryCapture>> {
    let start = epoch.elapsed_ns();
    let work = async {
        let url = reqwest::Url::parse(&src.url).map_err(|_| NativeBoundaryFailure::InvalidUrl)?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(NativeBoundaryFailure::InvalidUrl);
        }
        if src.insecure_tls {
            return Err(NativeBoundaryFailure::UnsupportedTransport);
        }
        let timeout = Duration::from_millis(cfg.timeout_ms.min(src.interval_ms).clamp(1, 5000));
        let limit = cfg.max_body_bytes.min(4 * 1024 * 1024);
        // No inference client/default headers. Explicit telemetry auth is retained;
        // transport policy is opt-in and replaces, not duplicates, the same GET.
        let headers =
            super::scraper::auth_headers(src).map_err(|_| NativeBoundaryFailure::Fetch)?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .build()
            .map_err(|_| NativeBoundaryFailure::Fetch)?;
        let acquire = async {
            let mut response = client
                .get(url)
                .headers(headers)
                .header("x-metrum-telemetry-boundary", tag)
                .send()
                .await
                .map_err(fetch_error)?;
            if !response.status().is_success() {
                return Err(NativeBoundaryFailure::HttpStatus);
            }
            if response.content_length().is_some_and(|n| n > limit) {
                return Err(NativeBoundaryFailure::TooLarge);
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(fetch_error)? {
                let len = bytes
                    .len()
                    .checked_add(chunk.len())
                    .ok_or(NativeBoundaryFailure::TooLarge)?;
                if u64::try_from(len).map_err(|_| NativeBoundaryFailure::TooLarge)? > limit {
                    return Err(NativeBoundaryFailure::TooLarge);
                }
                bytes.extend_from_slice(&chunk);
            }
            let end = epoch.elapsed_ns();
            let text = String::from_utf8(bytes).map_err(|_| NativeBoundaryFailure::InvalidUtf8)?;
            Ok((text, end))
        };
        tokio::time::timeout(timeout, acquire)
            .await
            .map_err(|_| NativeBoundaryFailure::Timeout)?
    };
    match work.await {
        Ok((text, end)) => {
            // Native semantic coverage is independent of successful transport
            // and legacy row coverage. Keep the same body for legacy parsing
            // even when the native profile's series limit is exceeded.
            let (probe, failure) =
                match probe(options.profile, options.source_revision.as_deref(), &text) {
                    Ok(probe) => (Some(probe), None),
                    Err(_) => (None, Some(NativeBoundaryFailure::Parse)),
                };
            Ok(NativeBody {
                text,
                capture: NativeBoundaryCapture {
                    fetch_start_ns: start,
                    fetch_end_ns: end,
                    probe,
                    failure,
                },
            })
        }
        Err(reason) => Err(Box::new(NativeBoundaryCapture {
            fetch_start_ns: start,
            fetch_end_ns: epoch.elapsed_ns(),
            probe: None,
            failure: Some(reason),
        })),
    }
}
fn fetch_error(err: reqwest::Error) -> NativeBoundaryFailure {
    if err.is_timeout() {
        NativeBoundaryFailure::Timeout
    } else {
        NativeBoundaryFailure::Fetch
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[tokio::test]
    async fn native_boundary_transport_failures_are_coded_bounded_and_never_zero() {
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = hits.clone();
        let app=Router::new()
            .route("/metrics",get(move |headers:axum::http::HeaderMap| {let seen=seen.clone();async move {
                assert!(!headers.contains_key("authorization"));seen.fetch_add(1,Ordering::SeqCst);
                "# TYPE vllm:prefix_cache_queries_total counter\nvllm:prefix_cache_queries_total{worker=\"PRIVATE\"} 10\n# TYPE vllm:prefix_cache_hits_total counter\nvllm:prefix_cache_hits_total{worker=\"PRIVATE\"} 0\n"
            }}))
            .route("/redirect",get(||async {(axum::http::StatusCode::FOUND,[("location","/metrics")])}))
            .route("/invalid",get(||async {vec![255u8]}))
            .route("/many", get(|| async {
                let mut text=String::from("# TYPE vllm:prefix_cache_queries_total counter\n");
                for i in 0..300 {text.push_str(&format!("vllm:prefix_cache_queries_total{{w=\"{i}\"}} 1\n"));}
                text
            }))
            .route("/large",get(||async {vec![b'x';20_000]}))
            .route("/chunked",get(||async {axum::body::Body::from_stream(futures_util::stream::iter([Ok::<_,std::io::Error>(vec![b'x';10_000]),Ok(vec![b'y';10_000])]))}))
            .route("/error",get(||async {(axum::http::StatusCode::INTERNAL_SERVER_ERROR,"PRIVATE_ERROR_BODY")}))
            .route("/slow",get(||async {std::future::pending::<String>().await}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let profile = ProfileId::VllmApc;
        let options = NativeBoundaryConfig {
            profile,
            source_revision: Some(profile.source_revision().into()),
        };
        let mut cfg = TelemetryConfig::from_metrics_url(&format!("http://{addr}/metrics"), 1000);
        cfg.timeout_ms = 1000;
        cfg.max_body_bytes = 16_384;
        let mut src = cfg.compile().unwrap().remove(0);
        let epoch = RunEpoch::new();
        let good = match fetch(&cfg, &src, &epoch, "before", &options).await {
            Ok(body) => body,
            Err(_) => panic!("live local fixture should capture"),
        };
        assert!(good.capture.failure.is_none());
        assert!(good.capture.probe.is_some());
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        src.url = format!("http://{addr}/many");
        let many = match fetch(&cfg, &src, &epoch, "before", &options).await {
            Ok(body) => body,
            Err(_) => panic!("native series-limit failure must retain successful legacy body"),
        };
        assert_eq!(many.capture.failure, Some(NativeBoundaryFailure::Parse));
        assert!(many.capture.probe.is_none());
        assert!(many.text.contains("w=\"299\""));
        for (path, expected) in [
            ("redirect", NativeBoundaryFailure::HttpStatus),
            ("invalid", NativeBoundaryFailure::InvalidUtf8),
            ("large", NativeBoundaryFailure::TooLarge),
            ("chunked", NativeBoundaryFailure::TooLarge),
            ("error", NativeBoundaryFailure::HttpStatus),
            ("slow", NativeBoundaryFailure::Timeout),
        ] {
            src.url = format!("http://{addr}/{path}");
            cfg.timeout_ms = if path == "slow" { 50 } else { 1000 };
            let failed = match fetch(&cfg, &src, &epoch, "after", &options).await {
                Ok(_) => panic!("expected {expected:?}"),
                Err(failed) => failed,
            };
            assert_eq!(failed.failure, Some(expected));
            assert!(failed.probe.is_none());
            assert!(failed.fetch_end_ns >= failed.fetch_start_ns);
        }
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "redirect must not fetch target"
        );
        for url in [
            format!("http://PRIVATE_USER:PRIVATE_PASSWORD@{addr}/metrics"),
            format!("http://{addr}/metrics#fragment"),
            "ftp://example.invalid/metrics".into(),
        ] {
            src.url = url;
            let failure = match fetch(&cfg, &src, &epoch, "before", &options).await {
                Ok(_) => panic!("invalid URL must fail"),
                Err(f) => f,
            };
            assert_eq!(failure.failure, Some(NativeBoundaryFailure::InvalidUrl));
            assert!(failure.probe.is_none());
        }
        src.url = format!("http://{addr}/metrics");
        src.insecure_tls = true;
        let failure = match fetch(&cfg, &src, &epoch, "before", &options).await {
            Ok(_) => panic!("insecure transport must not silently weaken native evidence"),
            Err(f) => f,
        };
        assert_eq!(
            failure.failure,
            Some(NativeBoundaryFailure::UnsupportedTransport)
        );
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }
}
