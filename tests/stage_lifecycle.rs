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
    reset.settle_delay = reset.timeout;
    assert!(reset
        .validate()
        .unwrap_err()
        .to_string()
        .contains("settle delay"));
    reset.settle_delay += Duration::from_millis(1);
    assert!(reset.validate().is_err());
    reset.settle_delay = Duration::ZERO;
    reset.command.clear();
    assert!(reset.validate().is_err());
}

#[tokio::test]
async fn hook_failure_retains_bounded_marked_stderr_and_timeout_diagnostics() {
    let mut reset = hook(ACK);
    reset.command = "printf 'fixture diagnostic' >&2; exit 9".into();
    let error = reset
        .execute(1, 1.0, &reqwest::Client::new())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("unsuccessfully"));
    assert!(error.contains("fixture diagnostic"));
    assert!(error.contains("complete, at most 4 KiB"));
    reset.command = "printf '%05000d' 0 >&2; printf 'TAIL_MUST_BE_DISCARDED' >&2; exit 9".into();
    let error = reset
        .execute(1, 1.0, &reqwest::Client::new())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("truncated to first 4 KiB"));
    assert_eq!(error.chars().filter(|&c| c == '0').count(), 4096);
    assert!(!error.contains("TAIL_MUST_BE_DISCARDED"));
    reset.command = "printf 'timeout diagnostic' >&2; exec sleep 60".into();
    reset.timeout = Duration::from_millis(200);
    let error = reset
        .execute(1, 1.0, &reqwest::Client::new())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("timed out"));
    assert!(error.contains("timeout diagnostic"));
}

#[cfg(unix)]
fn executing(pid: i32) -> bool {
    if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None)
        == Err(nix::errno::Errno::ESRCH)
    {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        // An adopted zombie cannot execute and may await the container init's
        // reaper. Do not confuse kill(0) visibility with a surviving process.
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
            Ok(stat)
                if stat
                    .rsplit_once(") ")
                    .is_some_and(|(_, rest)| rest.starts_with('Z')) =>
            {
                return false
            }
            _ => {}
        }
    }
    true
}

#[cfg(unix)]
struct ProbeCleanup(i32);
#[cfg(unix)]
impl Drop for ProbeCleanup {
    fn drop(&mut self) {
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(self.0),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
}

#[cfg(unix)]
async fn assert_background_grandchild_cleanup(abort: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("grandchild.pid");
    let mut reset = hook(ACK);
    reset.timeout = Duration::from_secs(2);
    reset.command = format!(
        "sh -c 'echo $$ > \"{}\"; exec sleep 60' & wait",
        path.display()
    );
    let task = tokio::spawn(async move { reset.execute(1, 1.0, &reqwest::Client::new()).await });
    let pid = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(pid) = text.trim().parse::<i32>() {
                    break pid;
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("grandchild startup");
    assert!(pid > 1);
    let _cleanup = ProbeCleanup(pid);
    assert!(executing(pid));
    if abort {
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    } else {
        assert!(task
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("timed out"));
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while executing(pid) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("backgrounded grandchild survived hook cancellation");
}

#[cfg(unix)]
#[tokio::test]
async fn timeout_kills_backgrounded_grandchild_process_group() {
    assert_background_grandchild_cleanup(false).await;
}

#[cfg(unix)]
#[tokio::test]
async fn dropped_hook_future_kills_backgrounded_grandchild_process_group() {
    assert_background_grandchild_cleanup(true).await;
}

#[cfg(unix)]
#[tokio::test]
async fn hook_context_is_environment_data_not_shell_interpolation() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("must-not-run");
    let mut reset = hook(ACK);
    reset.scope = vec![format!("$(touch {})", marker.display())];
    reset.command = r#"test "$BENCH_STAGE_INDEX" = 3 && test "$BENCH_STAGE_LOAD" = 2.5 && printf '{"success":true,"scope":["%s"],"ready":true,"settled":true}' "$BENCH_RESET_SCOPE""#.into();
    let ack = reset
        .execute(3, 2.5, &reqwest::Client::new())
        .await
        .unwrap();
    assert_eq!(ack.scope, reset.scope);
    assert!(
        !marker.exists(),
        "scope data was evaluated as a shell command"
    );
}
