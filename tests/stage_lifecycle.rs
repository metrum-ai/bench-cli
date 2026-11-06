// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use metrum_ai_bench::stage_lifecycle::ResetHook;
use std::time::Duration;

fn hook(reply: &str) -> ResetHook {
    ResetHook {
        command: format!("printf '%s' '{reply}'"),
        scope: vec!["resident".into(), "external".into()],
        timeout: Duration::from_secs(2),
        ready_url: None,
        settle_delay: Duration::ZERO,
    }
}

const ACK: &str = r#"{"success":true,"scope":["resident","external"],"ready":true,"settled":true}"#;

#[tokio::test]
async fn trusted_reset_requires_success_exact_scope_readiness_and_settling() {
    let client = reqwest::Client::new();
    let ack = hook(ACK).execute(1, 4.0, &client).await.unwrap();
    assert!(ack.acknowledged && ack.ready && ack.settled);
    assert_eq!(ack.scope, ["resident", "external"]);
    assert!(ack.started_unix_ns <= ack.acknowledged_unix_ns);
    assert!(ack.acknowledged_unix_ns <= ack.ready_unix_ns);
    assert!(ack.ready_unix_ns <= ack.settled_unix_ns);
    for bad in [
        ACK.replace("\"success\":true", "\"success\":false"),
        ACK.replace("\"ready\":true", "\"ready\":false"),
        ACK.replace("\"settled\":true", "\"settled\":false"),
        ACK.replace("[\"resident\",\"external\"]", "[\"resident\"]"),
        "not-json".to_string(),
    ] {
        assert!(hook(&bad).execute(1, 4.0, &client).await.is_err());
    }
    let mut failed = hook(ACK);
    failed.command = "exit 7".into();
    assert!(failed
        .execute(1, 4.0, &client)
        .await
        .unwrap_err()
        .to_string()
        .contains("unsuccessfully"));
    failed.command = "exec sleep 5".into();
    failed.timeout = Duration::from_millis(20);
    assert!(failed
        .execute(1, 4.0, &client)
        .await
        .unwrap_err()
        .to_string()
        .contains("timed out"));
}

#[tokio::test]
async fn readiness_http_success_false_is_not_ready() {
    use axum::{routing::get, Json, Router};
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/ready",
        get(|| async { Json(serde_json::json!({"success": false})) }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut reset = hook(ACK);
    reset.ready_url = Some(format!("http://{address}/ready"));
    reset.timeout = Duration::from_millis(150);
    assert!(reset
        .execute(1, 1.0, &reqwest::Client::new())
        .await
        .unwrap_err()
        .to_string()
        .contains("timed out"));
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn explicit_ready_response_and_settle_delay_are_recorded() {
    use axum::{routing::get, Json, Router};
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().route(
        "/ready",
        get(|| async { Json(serde_json::json!({"ready": true})) }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut reset = hook(ACK);
    reset.ready_url = Some(format!("http://{address}/ready"));
    reset.settle_delay = Duration::from_millis(20);
    let ack = reset
        .execute(1, 1.0, &reqwest::Client::new())
        .await
        .unwrap();
    assert!(ack.settled_unix_ns >= ack.ready_unix_ns + 20_000_000);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
    let mut oversized = hook(ACK);
    oversized.command = "printf '%070000d' 0".into();
    assert!(oversized
        .execute(1, 1.0, &reqwest::Client::new())
        .await
        .unwrap_err()
        .to_string()
        .contains("64 KiB"));
}

#[test]
fn scope_command_deadline_validated_without_execution() {
    let mut reset = hook(ACK);
    reset.scope.clear();
    assert!(reset.validate().is_err());
    reset.scope = vec!["resident".into(), "resident".into()];
    assert!(reset.validate().is_err());
    reset.scope = vec!["resident".into()];
    reset.timeout = Duration::ZERO;
    assert!(reset.validate().is_err());
    reset.timeout = Duration::from_secs(1);
    reset.command.clear();
    assert!(reset.validate().is_err());
}
