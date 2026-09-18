// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Shared chat/completions SSE consumption and visible-output timing.

use crate::error::RequestError;
use crate::sse::SseParser;
use futures_util::{Stream, StreamExt};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct ChatStreamResult {
    pub latency: Duration,
    pub ttft: Duration,
    pub first_reasoning: Option<Duration>,
    pub itl: Vec<Duration>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub completion_text: String,
}

#[derive(Default)]
struct Consumer {
    parser: SseParser,
    first_token_time: Option<Duration>,
    first_reasoning_time: Option<Duration>,
    last_token_at: Option<Duration>,
    itl: Vec<Duration>,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    done: bool,
    saw_finish: bool,
    completion_text: String,
}

impl Consumer {
    fn feed(&mut self, bytes: &[u8], elapsed: Duration) -> Result<(), RequestError> {
        for event in self.parser.feed(bytes) {
            match event {
                crate::sse::SseEvent::Done => {
                    self.done = true;
                }
                crate::sse::SseEvent::Json(parsed) => {
                    if let Some(error) = parsed.get("error") {
                        return Err(RequestError::ApiError {
                            message: error.to_string(),
                        });
                    }
                    if let Some(usage) = parsed.get("usage") {
                        self.prompt_tokens = usage
                            .get("prompt_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(self.prompt_tokens);
                        self.completion_tokens = usage
                            .get("completion_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(self.completion_tokens);
                        self.total_tokens = usage
                            .get("total_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(self.total_tokens);
                    }
                    if let Some(choices) = parsed.get("choices").and_then(|c| c.as_array()) {
                        for choice in choices {
                            if crate::sse::choice_finish_reason(choice).is_some() {
                                self.saw_finish = true;
                            }
                            if self.first_reasoning_time.is_none()
                                && crate::sse::choice_has_reasoning_token(choice)
                            {
                                self.first_reasoning_time = Some(elapsed);
                            }
                            if crate::sse::choice_has_output_token(choice) {
                                let now = elapsed;
                                if self.first_token_time.is_none() {
                                    self.first_token_time = Some(elapsed);
                                } else if let Some(prev) = self.last_token_at {
                                    self.itl.push(now.saturating_sub(prev));
                                }
                                self.last_token_at = Some(now);
                            }
                            if let Some(content) = crate::sse::choice_output_text(choice) {
                                self.completion_text.push_str(content);
                            }
                        }
                    }
                }
                crate::sse::SseEvent::Raw(_) => {}
            }
        }
        Ok(())
    }

    fn finish(self, latency: Duration) -> Result<ChatStreamResult, RequestError> {
        if !self.done && !self.saw_finish {
            return Err(RequestError::StreamTruncated);
        }
        let ttft = self.first_token_time.ok_or(RequestError::NoOutputToken)?;
        Ok(ChatStreamResult {
            latency,
            ttft,
            first_reasoning: self.first_reasoning_time,
            itl: self.itl,
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            total_tokens: self.total_tokens,
            completion_text: self.completion_text,
        })
    }
}

/// Consume an HTTP body after the caller checks its status. `started` is the
/// request send time. Role, reasoning, usage and finish events do not count
/// toward visible TTFT or ITL. First-byte timing stays with the HTTP caller.
pub async fn consume<S, B>(stream: S, started: Instant) -> Result<ChatStreamResult, RequestError>
where
    S: Stream<Item = Result<B, reqwest::Error>>,
    B: AsRef<[u8]>,
{
    futures_util::pin_mut!(stream);
    let mut consumer = Consumer::default();
    while let Some(item) = stream.next().await {
        let bytes = item.map_err(|error| RequestError::from_reqwest(&error))?;
        consumer.feed(bytes.as_ref(), started.elapsed())?;
        if consumer.done {
            break;
        }
    }
    consumer.finish(started.elapsed())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn event(consumer: &mut Consumer, value: Value, ms: u64) -> Result<(), RequestError> {
        consumer.feed(
            format!("data: {value}\n\n").as_bytes(),
            Duration::from_millis(ms),
        )
    }

    #[test]
    fn mid_stream_error_is_api_error() {
        let mut consumer = Consumer::default();
        event(
            &mut consumer,
            json!({"choices":[{"delta":{"content":"hi"}}]}),
            10,
        )
        .expect("content");
        let error = event(&mut consumer, json!({"error":{"message":"overloaded"}}), 20)
            .expect_err("API error");
        assert_eq!(
            error,
            RequestError::ApiError {
                message: r#"{"message":"overloaded"}"#.into()
            }
        );
    }

    #[test]
    fn missing_done_and_finish_is_truncated() {
        let mut consumer = Consumer::default();
        event(
            &mut consumer,
            json!({"choices":[{"delta":{"content":"hi"}}]}),
            10,
        )
        .expect("content");
        assert_eq!(
            consumer
                .finish(Duration::from_millis(20))
                .expect_err("truncated"),
            RequestError::StreamTruncated
        );
    }

    #[test]
    fn completed_reasoning_only_stream_has_no_output_token() {
        for terminal in [
            "data: [DONE]\n\n",
            "data: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\n",
        ] {
            let mut consumer = Consumer::default();
            event(
                &mut consumer,
                json!({"choices":[{"delta":{"reasoning_content":"think"}}]}),
                10,
            )
            .expect("reasoning");
            consumer
                .feed(terminal.as_bytes(), Duration::from_millis(20))
                .expect("terminal");
            assert_eq!(
                consumer
                    .finish(Duration::from_millis(20))
                    .expect_err("no output"),
                RequestError::NoOutputToken
            );
        }
    }

    #[test]
    fn reasoning_and_metadata_do_not_affect_visible_ttft_or_itl() {
        let mut consumer = Consumer::default();
        for (ms, delta) in [
            (5, json!({"role":"assistant"})),
            (10, json!({"reasoning_content":"think"})),
            (30, json!({"content":"hello"})),
            (40, json!({"reasoning":"more"})),
            (60, json!({"content":" world"})),
        ] {
            event(&mut consumer, json!({"choices":[{"delta":delta}]}), ms).expect("event");
        }
        event(&mut consumer, json!({"choices":[{"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}}), 80).expect("finish");
        let result = consumer
            .finish(Duration::from_millis(90))
            .expect("complete without DONE");
        assert_eq!(result.first_reasoning, Some(Duration::from_millis(10)));
        assert_eq!(result.ttft, Duration::from_millis(30));
        assert_eq!(result.itl, vec![Duration::from_millis(30)]);
        assert_eq!(result.completion_text, "hello world");
        assert_eq!(
            (
                result.prompt_tokens,
                result.completion_tokens,
                result.total_tokens
            ),
            (2, 3, 5)
        );
    }

    #[tokio::test]
    async fn consume_reassembles_chunks_and_stops_at_done() {
        let chunks: Vec<Result<&[u8], reqwest::Error>> = vec![
            Ok(b"data: {\"choices\":[{\"delta\":{\"content\":"),
            Ok(b"\"hi\"}}]}\n\ndata: [DONE]\n\n"),
            Ok(b"data: {\"error\":\"after done\"}\n\n"),
        ];
        let result = consume(futures_util::stream::iter(chunks), Instant::now())
            .await
            .expect("stream");
        assert_eq!(result.completion_text, "hi");
    }
}
