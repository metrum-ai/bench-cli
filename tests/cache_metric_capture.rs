// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use axum::{routing::get, Router};
use metrum_ai_bench::cache_metric_capture::{capture, CaptureFailure};
use metrum_ai_bench::cache_metrics::ProfileId;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[tokio::test]
async fn opt_in_capture_redacts_private_labels_and_preserves_unavailable_not_zero() {
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let app=Router::new().route("/private/metrics",get(move || {
        let seen=seen.clone(); async move { seen.fetch_add(1,Ordering::SeqCst);
            "# TYPE vllm:prefix_cache_queries_total counter\nvllm:prefix_cache_queries_total{worker=\"PRIVATE_HOST\"} 10\n# TYPE vllm:prefix_cache_hits_total counter\nvllm:prefix_cache_hits_total{worker=\"PRIVATE_HOST\"} 0\n" }
    })).route("/redirect",get(||async { (axum::http::StatusCode::FOUND,[("location","/private/metrics")]) }))
    .route("/large",get(||async {vec![b'x';4*1024*1024+1]}))
    .route("/invalid",get(||async {vec![255u8]}));
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let p = ProfileId::VllmApc;
    let url = format!("http://{address}/private/metrics");
    let report = capture(&url, p, Some(p.source_revision())).await;
    assert!(report.failure.is_none());
    assert!(report.missing_required_metrics.is_empty());
    assert_eq!(report.samples.len(), 2);
    assert_eq!(report.samples[1].value, Some(0.0));
    let json = serde_json::to_string(&report).unwrap();
    assert!(!json.contains("PRIVATE_HOST"));
    assert!(!json.contains("/private/metrics"));
    assert!(!json.contains(&address.to_string()));
    let unknown = capture(&url, p, None).await;
    assert!(unknown
        .samples
        .iter()
        .all(|sample| sample.value.is_none() && sample.unknown.is_some()));
    for (route, reason) in [
        ("redirect", CaptureFailure::HttpStatus),
        ("large", CaptureFailure::TooLarge),
        ("invalid", CaptureFailure::InvalidUtf8),
    ] {
        let report = capture(
            &format!("http://{address}/{route}"),
            p,
            Some(p.source_revision()),
        )
        .await;
        assert_eq!(report.failure, Some(reason));
        assert!(report.samples.is_empty());
    }
    assert_eq!(
        count.load(Ordering::SeqCst),
        2,
        "redirect must not trigger another metric fetch"
    );
    let rejected = capture(
        &format!("http://PRIVATE_USER:PRIVATE_PASSWORD@{address}/private/metrics"),
        p,
        Some("not a revision PRIVATE_CONTEXT"),
    )
    .await;
    assert_eq!(rejected.failure, Some(CaptureFailure::Fetch));
    let json = serde_json::to_string(&rejected).unwrap();
    assert!(!json.contains("PRIVATE_"));
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}
