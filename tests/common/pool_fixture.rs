// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Fixed-length responses make the DONE bytes and HTTP body EOF coincide.
/// Unlike chunked responses there is no trailing chunk terminator left to drain.
pub struct PoolFixture {
    pub url: String,
    worker: Option<JoinHandle<usize>>,
}

impl PoolFixture {
    pub fn start(pooled: bool, requests: usize, tokens: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
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
                // Do not let Nagle coalesce paced SSE writes behind delayed ACKs.
                stream.set_nodelay(true).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                loop {
                    if !read_request(&mut stream) {
                        break;
                    }
                    let mut chunks = Vec::new();
                    for _ in 0..tokens {
                        chunks.push(
                            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"x\"}}]}\n\n"
                                .to_owned(),
                        );
                    }
                    chunks.push(format!("data: {{\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":1,\"completion_tokens\":{tokens},\"total_tokens\":{}}}}}\n\ndata: [DONE]\n\n", tokens + 1));
                    let size: usize = chunks.iter().map(String::len).sum();
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {size}\r\nConnection: {}\r\n\r\n", if pooled { "keep-alive" } else { "close" }).unwrap();
                    for chunk in chunks {
                        stream.write_all(chunk.as_bytes()).unwrap();
                        stream.flush().unwrap();
                        thread::sleep(Duration::from_millis(10));
                    }
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
            worker: Some(worker),
        }
    }

    pub fn finish(mut self) -> usize {
        self.worker.take().unwrap().join().expect("fixture worker")
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
