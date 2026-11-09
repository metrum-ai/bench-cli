// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Controlled CPU-only HTTP sink for the VLM payload-window timing test.
//! It retains complete wire bodies for validation after all timing samples,
//! so server-side image/JSON decoding cannot dominate one half of a comparison.

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use serde_json::{json, Value};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::oneshot;

async fn chat(State(bodies): State<Arc<Mutex<Vec<Bytes>>>>, body: Bytes) -> Json<Value> {
    bodies.lock().expect("capture mutex").push(body);
    tokio::time::sleep(Duration::from_millis(100)).await;
    Json(
        json!({"id": "payload-window", "object": "chat.completion", "created": 1,
        "model": "dummy", "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"},
        "finish_reason": "stop"}], "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}),
    )
}

pub struct PayloadServer {
    port: u16,
    bodies: Arc<Mutex<Vec<Bytes>>>,
    shutdown: Option<oneshot::Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl PayloadServer {
    pub fn start(max_body: usize) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("payload listener");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let port = listener.local_addr().expect("listener address").port();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::clone(&bodies);
        let (shutdown, signal) = oneshot::channel();
        let (started_tx, started_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("payload runtime");
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).expect("tokio listener");
                let app = Router::new()
                    .route("/v1/chat/completions", post(chat))
                    .layer(DefaultBodyLimit::max(max_body))
                    .with_state(state);
                started_tx.send(()).expect("start notification");
                axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = signal.await;
                    })
                    .await
                    .expect("payload server");
            });
        });
        started_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("payload server startup");
        Self {
            port,
            bodies,
            shutdown: Some(shutdown),
            worker: Some(worker),
        }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/v1/chat/completions", self.port)
    }

    /// Check every measured and warmup request after timing, including the exact
    /// image bytes. No body parsing or image validation occurs during timing.
    pub fn validate(&self, png: &[u8], expected_requests: usize) {
        let expected_url = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        );
        let decoded = image::load_from_memory(png).expect("valid PNG fixture");
        assert_eq!((decoded.width(), decoded.height()), (4096, 4096));
        let bodies = self.bodies.lock().expect("capture mutex");
        assert_eq!(
            bodies.len(),
            expected_requests,
            "all reference and tool requests must reach the fixture"
        );
        for body in bodies.iter() {
            let parsed: Value = serde_json::from_slice(body).expect("valid request JSON");
            assert_eq!(parsed["model"], "dummy");
            let images: Vec<&str> = parsed["messages"]
                .as_array()
                .expect("messages")
                .iter()
                .filter_map(|message| message["content"].as_array())
                .flat_map(|content| content.iter())
                .filter_map(|part| part["image_url"]["url"].as_str())
                .collect();
            assert_eq!(images.len(), 1, "exactly one image per request");
            assert!(
                images[0] == expected_url,
                "wire image differs from pinned PNG fixture"
            );
        }
    }
}

impl Drop for PayloadServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(worker) = self.worker.take() {
            worker.join().expect("payload server shutdown");
        }
    }
}
