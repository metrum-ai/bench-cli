// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Fixed-length bulk responses exercise body reads independently of TCP write
/// boundaries. DONE is at HTTP body EOF, without a delayed transfer terminator.
pub struct PoolFixture {
    pub url: String,
    pub body_bytes: usize,
    worker: Option<JoinHandle<usize>>,
}

impl PoolFixture {
    pub fn start(pooled: bool, requests: usize, tokens: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let (wire, body_bytes) = response(pooled, tokens);
        let worker = thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            let mut served = 0;
            let mut connections = 0;
            while served < requests {
                assert!(std::time::Instant::now() < deadline, "fixture timed out");
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(e) => panic!("accept: {e}"),
                };
                connections += 1;
                // Avoid delaying the complete response behind a prior TCP ACK.
                stream.set_nodelay(true).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                loop {
                    if !read_request(&mut stream) {
                        break;
                    }
                    // No per-event sleeps or assumed packet boundaries. Queue
                    // the complete, fixed-length response including DONE.
                    stream.write_all(&wire).unwrap();
                    stream.flush().unwrap();
                    served += 1;
                    if !pooled || served == requests {
                        break;
                    }
                }
            }
            connections
        });
        Self {
            url,
            body_bytes,
            worker: Some(worker),
        }
    }

    pub fn finish(mut self) -> usize {
        self.worker.take().unwrap().join().expect("fixture worker")
    }
}

// A bulk body is intentional: chunks_received counts HTTP body reads, not SSE
// events or server writes. Eight MiB exceeds the locked HTTP/1 reader's adaptive
// buffer by a wide margin, even if the receiving task is descheduled.
const BULK_COMMENT_BYTES: usize = 8 * 1024 * 1024;

fn response(pooled: bool, tokens: usize) -> (Vec<u8>, usize) {
    let mut body = vec![b'p'; BULK_COMMENT_BYTES];
    body[0] = b':'; // Valid SSE comment, ignored rather than fabricated output.
    body.extend_from_slice(b"\n\n");
    let token = b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"x\"}}]}\n\n";
    for _ in 0..tokens {
        body.extend_from_slice(token);
    }
    write!(body, "data: {{\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":1,\"completion_tokens\":{tokens},\"total_tokens\":{}}}}}\n\ndata: [DONE]\n\n", tokens + 1).unwrap();
    let size = body.len();
    let mut wire = Vec::new();
    write!(wire, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {size}\r\nConnection: {}\r\n\r\n", if pooled { "keep-alive" } else { "close" }).unwrap();
    wire.extend_from_slice(&body);
    (wire, size)
}

#[test]
fn response_has_complete_fixed_length_and_ignored_bulk_comment() {
    use metrum_ai_bench::sse::{SseEvent, SseParser};
    for pooled in [true, false] {
        let (wire, size) = response(pooled, 8);
        let header_end = wire.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let header = std::str::from_utf8(&wire[..header_end]).unwrap();
        assert!(header.contains(&format!("Content-Length: {size}\r\n")));
        assert!(!header.contains("Transfer-Encoding:"));
        let body = &wire[header_end..];
        assert_eq!(body.len(), size);
        assert!(size > BULK_COMMENT_BYTES);
        assert!(body.ends_with(b"data: [DONE]\n\n"));
        let events = SseParser::new().feed(body);
        assert_eq!(events.len(), 10); // Eight tokens, finish/usage, DONE.
        assert!(matches!(events.last(), Some(SseEvent::Done)));
        assert_eq!(events.iter().filter(|event| matches!(event, SseEvent::Json(value) if value["choices"][0]["delta"]["content"] == "x")).count(), 8);
    }
}

fn read_request(stream: &mut TcpStream) -> bool {
    let mut header = Vec::new();
    let mut byte = [0];
    while !header.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => header.push(byte[0]),
            _ => return false,
        }
    }
    let text = String::from_utf8(header).unwrap();
    let length = text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).unwrap();
    true
}
