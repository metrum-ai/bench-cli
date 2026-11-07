// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Shared chat/completions SSE consumption and visible-output timing.

use crate::error::RequestError;
use crate::sse::SseParser;
use futures_util::{Stream, StreamExt};
use serde::Serialize;
use std::pin::Pin;
use std::time::{Duration, Instant};

/// Whether the selected non-streaming choice contains visible text or tool output.
/// This does not imply a first-token timestamp was observed.
pub fn response_has_visible_output(response: &serde_json::Value) -> bool {
    let Some(choices) = response
        .get("choices")
        .and_then(serde_json::Value::as_array)
    else {
        return false;
    };
    let Some(choice) = choices
        .iter()
        .find(|c| c.get("index").and_then(serde_json::Value::as_u64) == Some(0))
        .or_else(|| choices.first().filter(|c| c.get("index").is_none()))
    else {
        return false;
    };
    let nonempty = |value: &serde_json::Value| value.as_str().is_some_and(|s| !s.is_empty());
    if choice.get("text").is_some_and(nonempty) {
        return true;
    }
    let Some(message) = choice.get("message") else {
        return false;
    };
    message.get("content").is_some_and(|content| {
        nonempty(content)
            || content
                .as_array()
                .is_some_and(|parts| parts.iter().any(|p| p.get("text").is_some_and(nonempty)))
    }) || message
        .get("tool_calls")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|calls| !calls.is_empty())
        || message
            .get("function_call")
            .is_some_and(serde_json::Value::is_object)
}

#[derive(Debug)]
pub struct ChatStreamResult {
    pub latency: Duration,
    /// First visible output token time. `None` when the stream finished without
    /// visible content (reasoning-only or empty completion).
    pub ttft: Option<Duration>,
    pub first_reasoning: Option<Duration>,
    /// Finish reason for choice index 0, when supplied by the server.
    pub finish_reason: Option<String>,
    pub itl: Vec<Duration>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    /// True only when both prompt and completion usage counts were reported.
    pub usage_complete: bool,
    pub prompt_tokens_present: bool,
    pub completion_tokens_present: bool,
    /// Server-reported reasoning tokens from the usage chunk; `None` when absent.
    pub reasoning_tokens: Option<u64>,
    pub completion_text: String,
    /// Bounded SSE event timeline (#313): index, timestamp and categories
    /// per event — never prompt text, content or secrets. `Some` only when
    /// the caller enabled tracing (`--sse-trace-first-n`).
    pub trace: Option<Vec<StreamEventTrace>>,
}

impl ChatStreamResult {
    /// Classify a completed stream independently of the TTFT fallback policy.
    pub fn output_error(&self) -> Option<RequestError> {
        if self.ttft.is_some() {
            None
        } else if self.first_reasoning.is_some() && self.finish_reason.as_deref() == Some("length")
        {
            Some(RequestError::ReasoningTruncated)
        } else {
            Some(RequestError::NoOutputToken)
        }
    }
}

/// Server usage observed before a stream error. Incomplete counts stay marked
/// missing and therefore cannot be promoted into full aggregate rates.
#[derive(Debug, Clone, Default)]
pub struct PartialUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub reasoning_tokens: Option<u64>,
    pub usage_complete: bool,
    pub prompt_tokens_present: bool,
    pub completion_tokens_present: bool,
}

/// Stream failure with any usage that arrived before the failure.
#[derive(Debug)]
pub struct ChatStreamFailure {
    pub error: RequestError,
    pub usage: PartialUsage,
}

impl std::fmt::Display for ChatStreamFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.error, f)
    }
}
impl std::error::Error for ChatStreamFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// One SSE event in a traced request (#313): categories and timing only.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StreamEventTrace {
    /// Event index in arrival order.
    pub index: u32,
    /// Milliseconds from request send.
    pub t_ms: u64,
    /// Detected categories for this event (may be several on one delta).
    pub categories: Vec<&'static str>,
}

#[derive(Default)]
struct Consumer {
    parser: SseParser,
    trace: Option<Vec<StreamEventTrace>>,
    first_token_time: Option<Duration>,
    first_reasoning_time: Option<Duration>,
    last_token_at: Option<Duration>,
    itl: Vec<Duration>,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    prompt_tokens_present: bool,
    completion_tokens_present: bool,
    reasoning_tokens: Option<u64>,
    done: bool,
    saw_finish: bool,
    finish_reason: Option<String>,
    completion_text: String,
}

impl Consumer {
    fn partial_usage(&self) -> PartialUsage {
        PartialUsage {
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            total_tokens: self.total_tokens,
            reasoning_tokens: self.reasoning_tokens,
            usage_complete: self.prompt_tokens_present && self.completion_tokens_present,
            prompt_tokens_present: self.prompt_tokens_present,
            completion_tokens_present: self.completion_tokens_present,
        }
    }

    fn feed(&mut self, bytes: &[u8], elapsed: Duration) -> Result<(), RequestError> {
        for event in self.parser.feed(bytes) {
            // Trace the event's categories and timing only (#313): no
            // prompt text, content, or secrets are ever recorded.
            let mut categories: Vec<&'static str> = Vec::new();
            match &event {
                crate::sse::SseEvent::Done => categories.push("done"),
                crate::sse::SseEvent::Raw(_) => categories.push("other"),
                crate::sse::SseEvent::Json(parsed) => {
                    if parsed.get("error").is_some() {
                        categories.push("error");
                    }
                    if parsed.get("usage").is_some() {
                        categories.push("usage");
                    }
                    if let Some(choices) = parsed.get("choices").and_then(|c| c.as_array()) {
                        for choice in measured_choices(choices) {
                            if crate::sse::choice_finish_reason(choice).is_some() {
                                categories.push("finish");
                            }
                            if crate::sse::choice_has_reasoning_token(choice) {
                                categories.push("reasoning");
                            }
                            if crate::sse::choice_has_output_token(choice) {
                                categories.push("content");
                            }
                        }
                    }
                    if categories.is_empty() {
                        categories.push("other");
                    }
                }
            }
            if let Some(trace) = self.trace.as_mut() {
                trace.push(StreamEventTrace {
                    index: trace.len() as u32,
                    t_ms: elapsed.as_millis() as u64,
                    categories,
                });
            }
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
                        let (prompt_present, completion_present) = usage_presence(usage);
                        self.prompt_tokens_present |= prompt_present;
                        self.completion_tokens_present |= completion_present;
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
                        self.reasoning_tokens =
                            crate::usage::reasoning_tokens(usage).or(self.reasoning_tokens);
                    }
                    if let Some(choices) = parsed.get("choices").and_then(|c| c.as_array()) {
                        for choice in measured_choices(choices) {
                            if let Some(reason) = crate::sse::choice_finish_reason(choice) {
                                self.saw_finish = true;
                                self.finish_reason = Some(reason.to_owned());
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
                crate::sse::SseEvent::Raw(_) => {
                    return Err(RequestError::ParseError {
                        message: "invalid JSON in SSE data event".into(),
                    });
                }
            }
        }
        Ok(())
    }

    fn finish(
        self,
        latency: Duration,
        allow_missing_ttft: bool,
    ) -> Result<ChatStreamResult, RequestError> {
        if !self.done && !self.saw_finish {
            return Err(RequestError::StreamTruncated);
        }
        if self.first_token_time.is_none() && !allow_missing_ttft {
            return Err(RequestError::NoOutputToken);
        }
        Ok(ChatStreamResult {
            latency,
            ttft: self.first_token_time,
            first_reasoning: self.first_reasoning_time,
            finish_reason: self.finish_reason,
            itl: self.itl,
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            total_tokens: self.total_tokens,
            usage_complete: self.prompt_tokens_present && self.completion_tokens_present,
            prompt_tokens_present: self.prompt_tokens_present,
            completion_tokens_present: self.completion_tokens_present,
            reasoning_tokens: self.reasoning_tokens,
            completion_text: self.completion_text,
            trace: self.trace,
        })
    }
}

/// Select choice index 0; servers omitting indices are interpreted by position.
fn measured_choices(choices: &[serde_json::Value]) -> impl Iterator<Item = &serde_json::Value> {
    choices.iter().enumerate().filter_map(|(position, choice)| {
        let selected = choice
            .get("index")
            .and_then(|index| index.as_u64())
            .map_or(position == 0, |index| index == 0);
        selected.then_some(choice)
    })
}

/// Whether the server supplied usable input and output counts in a usage object.
pub fn usage_presence(usage: &serde_json::Value) -> (bool, bool) {
    (
        usage
            .get("prompt_tokens")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        usage
            .get("completion_tokens")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
    )
}

/// Consume an HTTP body after the caller checks its status. `started` is the
/// request send time. Role, reasoning, usage and finish events do not count
/// toward visible TTFT or ITL. First-byte timing stays with the HTTP caller.
pub async fn consume<S, B>(stream: S, started: Instant) -> Result<ChatStreamResult, RequestError>
where
    S: Stream<Item = Result<B, reqwest::Error>>,
    B: AsRef<[u8]>,
{
    consume_with_options(stream, started, false, false).await
}

/// Like [`consume`], but keep a finished stream that never emitted a visible
/// token when `allow_missing_ttft` is true (for `--infer-ttft-from-first-byte`
/// and the preflight streaming probe, #230). `trace` records a bounded
/// category/timestamp timeline per event (#313; never content or secrets).
pub async fn consume_with_options<S, B>(
    stream: S,
    started: Instant,
    allow_missing_ttft: bool,
    trace: bool,
) -> Result<ChatStreamResult, RequestError>
where
    S: Stream<Item = Result<B, reqwest::Error>>,
    B: AsRef<[u8]>,
{
    consume_preserving_usage(stream, started, allow_missing_ttft, trace)
        .await
        .map_err(|failure| failure.error)
}

/// Consume a stream while retaining server usage observed before a failure.
pub async fn consume_preserving_usage<S, B>(
    stream: S,
    started: Instant,
    allow_missing_ttft: bool,
    trace: bool,
) -> Result<ChatStreamResult, ChatStreamFailure>
where
    S: Stream<Item = Result<B, reqwest::Error>>,
    B: AsRef<[u8]>,
{
    consume_with_clock(stream, || started.elapsed(), allow_missing_ttft, trace).await
}

// Keep the production std::time clock at the public boundary; tests can inject
// Tokio's paused clock without changing request timing or the shipped profile.
async fn consume_with_clock<S, B, F>(
    stream: S,
    elapsed: F,
    allow_missing_ttft: bool,
    trace: bool,
) -> Result<ChatStreamResult, ChatStreamFailure>
where
    S: Stream<Item = Result<B, reqwest::Error>>,
    B: AsRef<[u8]>,
    F: Fn() -> Duration,
{
    futures_util::pin_mut!(stream);
    let mut consumer = Consumer {
        trace: trace.then(Vec::new),
        ..Consumer::default()
    };
    while let Some(item) = stream.next().await {
        let bytes = item.map_err(|error| ChatStreamFailure {
            error: RequestError::from_reqwest(&error),
            usage: consumer.partial_usage(),
        })?;
        consumer
            .feed(bytes.as_ref(), elapsed())
            .map_err(|error| ChatStreamFailure {
                error,
                usage: consumer.partial_usage(),
            })?;
        if consumer.done {
            break;
        }
    }
    let usage = consumer.partial_usage();
    // Latency, TTFT and ITL are fixed here, before any post-`[DONE]` drain.
    let result = consumer
        .finish(elapsed(), allow_missing_ttft)
        .map_err(|error| ChatStreamFailure { error, usage });
    if result.is_ok() {
        drain_after_done(stream).await;
    }
    result
}

/// Longest wait for the body end after `[DONE]`.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(250);
/// Most bytes read after `[DONE]` before giving up on the connection.
const DRAIN_BYTE_CAP: usize = 64 * 1024;

/// Read the body to its end after `[DONE]` so hyper sees the terminating
/// chunk and returns the connection to the pool. Dropping the body early
/// discards the connection, and the next request on that slot pays a fresh
/// TCP/TLS connect inside its TTFT. Bounded by [`DRAIN_TIMEOUT`] and
/// [`DRAIN_BYTE_CAP`] so a server that keeps the stream open cannot hang the
/// client; on either limit the connection is dropped as before.
async fn drain_after_done<S, B, E>(mut stream: Pin<&mut S>)
where
    S: Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
{
    let drain = async {
        let mut drained = 0usize;
        while let Some(Ok(bytes)) = stream.next().await {
            drained += bytes.as_ref().len();
            if drained > DRAIN_BYTE_CAP {
                break;
            }
        }
    };
    let _ = tokio::time::timeout(DRAIN_TIMEOUT, drain).await;
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
    fn nonstream_answer_detection_does_not_invent_a_clock() {
        assert!(response_has_visible_output(
            &json!({"choices":[{"index":0,"message":{"content":"answer"}}]})
        ));
        assert!(response_has_visible_output(
            &json!({"choices":[{"index":0,"message":{"tool_calls":[{"id":"call"}]}}]})
        ));
        assert!(response_has_visible_output(
            &json!({"choices":[{"text":"answer"}]})
        ));
        assert!(!response_has_visible_output(
            &json!({"choices":[{"index":0,"message":{"content":"","reasoning":"think"}}]})
        ));
        assert!(!response_has_visible_output(
            &json!({"choices":[{"index":1,"message":{"content":"wrong choice"}}]})
        ));
    }

    #[test]
    fn malformed_sse_payload_is_a_redacted_parse_error() {
        let mut consumer = Consumer::default();
        let error = consumer
            .feed(b"data: {not-json}\n\n", Duration::from_millis(1))
            .unwrap_err();
        assert_eq!(
            error,
            RequestError::ParseError {
                message: "invalid JSON in SSE data event".into()
            }
        );
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
                .finish(Duration::from_millis(20), false)
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
                    .finish(Duration::from_millis(20), false)
                    .expect_err("no output"),
                RequestError::NoOutputToken
            );
            let mut allowed = Consumer::default();
            event(
                &mut allowed,
                json!({"choices":[{"delta":{"reasoning_content":"think"}}]}),
                10,
            )
            .expect("reasoning");
            allowed
                .feed(terminal.as_bytes(), Duration::from_millis(20))
                .expect("terminal");
            let result = allowed
                .finish(Duration::from_millis(20), true)
                .expect("allowed missing ttft");
            assert!(result.ttft.is_none());
        }
    }

    #[test]
    fn measures_only_choice_index_zero() {
        let mut consumer = Consumer::default();
        event(
            &mut consumer,
            json!({"choices":[
                {"index":1,"delta":{"content":"ignored"},"finish_reason":"stop"},
                {"index":0,"delta":{"reasoning_content":"thinking"}}
            ]}),
            10,
        )
        .unwrap();
        event(
            &mut consumer,
            json!({"choices":[{"index":0,"finish_reason":"length"}]}),
            20,
        )
        .unwrap();
        let result = consumer.finish(Duration::from_millis(30), true).unwrap();
        assert_eq!(result.first_reasoning, Some(Duration::from_millis(10)));
        assert_eq!(result.ttft, None);
        assert_eq!(result.finish_reason.as_deref(), Some("length"));
        assert_eq!(
            result.output_error(),
            Some(RequestError::ReasoningTruncated)
        );
        assert_eq!(result.completion_text, "");
    }

    #[test]
    fn no_answer_outcome_requires_reasoning_and_length() {
        for (reasoning, finish, expected) in [
            (true, "length", RequestError::ReasoningTruncated),
            (true, "stop", RequestError::NoOutputToken),
            (false, "length", RequestError::NoOutputToken),
        ] {
            let mut consumer = Consumer::default();
            if reasoning {
                event(
                    &mut consumer,
                    json!({"choices":[{"delta":{"reasoning":"think"}}]}),
                    10,
                )
                .unwrap();
            }
            event(
                &mut consumer,
                json!({"choices":[{"finish_reason":finish}]}),
                20,
            )
            .unwrap();
            let result = consumer.finish(Duration::from_millis(30), true).unwrap();
            assert_eq!(result.output_error(), Some(expected));
        }
    }

    // BC-SSE-001: injected send-relative clocks, independent of wall-clock timing.
    #[test]
    fn bc_sse_001_first_any_visible_and_metadata_ordering() {
        use crate::measurement::{resolve_ttft_with_fallback, TtftSource};
        for (reasoning_ms, answer_ms) in [(1000, 3000), (3000, 1000), (1000, 1000)] {
            let mut consumer = Consumer::default();
            event(&mut consumer, json!({"choices":[{"index":1,"delta":{"content":"ignored"}},{"index":0,"delta":{"role":"assistant"}}]}), 100).unwrap();
            let mut deltas = vec![
                (reasoning_ms, json!({"reasoning_content":"think"})),
                (answer_ms, json!({"content":"answer"})),
            ];
            deltas.sort_by_key(|(ms, _)| *ms);
            for (ms, delta) in deltas {
                event(
                    &mut consumer,
                    json!({"choices":[{"index":0,"delta":delta}]}),
                    ms,
                )
                .unwrap();
            }
            event(
                &mut consumer,
                json!({"choices":[{"index":0,"finish_reason":"stop"}]}),
                4000,
            )
            .unwrap();
            let result = consumer.finish(Duration::from_secs(4), true).unwrap();
            let resolved = resolve_ttft_with_fallback(
                true,
                result.ttft.map(|t| t.as_secs_f64()),
                result.first_reasoning.map(|t| t.as_secs_f64()),
                Some(0.1),
                false,
                false,
            );
            assert_eq!(resolved.ttft_s, Some(1.0));
            assert_eq!(resolved.source, Some(TtftSource::Stream));
            assert_eq!(result.ttft.unwrap().as_millis(), u128::from(answer_ms));
        }
        let mut same_event = Consumer::default();
        event(
            &mut same_event,
            json!({"choices":[{"index":0,"delta":{"reasoning":"think","content":"answer"}}]}),
            1000,
        )
        .unwrap();
        event(
            &mut same_event,
            json!({"choices":[{"index":0,"finish_reason":"stop"}]}),
            2000,
        )
        .unwrap();
        let same = same_event.finish(Duration::from_secs(2), true).unwrap();
        assert_eq!(same.first_reasoning, same.ttft);
        let mut tool = Consumer::default();
        event(&mut tool, json!({"choices":[{"index":0,"delta":{"tool_calls":[{"function":{"arguments":"{"}}]}}]}), 1000).unwrap();
        event(
            &mut tool,
            json!({"choices":[{"index":0,"finish_reason":"tool_calls"}]}),
            2000,
        )
        .unwrap();
        assert_eq!(
            tool.finish(Duration::from_secs(2), true).unwrap().ttft,
            Some(Duration::from_secs(1))
        );
        let mut role = Consumer::default();
        event(&mut role, json!({"choices":[{"index":0,"delta":{"role":"assistant"}},{"index":1,"delta":{"content":"ignore"}}]}), 1000).unwrap();
        event(
            &mut role,
            json!({"choices":[{"index":0,"finish_reason":"stop"}]}),
            2000,
        )
        .unwrap();
        let empty = role.finish(Duration::from_secs(2), true).unwrap();
        assert!(empty.ttft.is_none() && empty.first_reasoning.is_none());
        assert_eq!(empty.output_error(), Some(RequestError::NoOutputToken));
    }

    // BC-SSE-002: finished, unfinished, empty, and repeated terminal events.
    #[test]
    fn bc_sse_002_reasoning_only_terminal_outcomes() {
        for (finish, expected) in [
            ("length", RequestError::ReasoningTruncated),
            ("stop", RequestError::NoOutputToken),
        ] {
            let mut consumer = Consumer::default();
            event(
                &mut consumer,
                json!({"choices":[{"index":0,"delta":{"reasoning_content":"think"}}]}),
                1000,
            )
            .unwrap();
            for _ in 0..2 {
                event(
                    &mut consumer,
                    json!({"choices":[{"index":0,"finish_reason":finish}]}),
                    2000,
                )
                .unwrap();
            }
            let result = consumer.finish(Duration::from_secs(3), true).unwrap();
            let output_error = result.output_error();
            assert_eq!(output_error, Some(expected.clone()));
            let now = chrono::Utc::now();
            let record = crate::record::RequestRecord::success(
                0,
                crate::record::Phase::Measure,
                "fixture".into(),
                now,
                now,
                Duration::from_secs(3),
                result.ttft,
                result.first_reasoning,
                result.itl,
                0,
                0,
                0,
            )
            .with_chat_outcome(output_error, false);
            let summary = crate::summary::RunSummary::from_records(&[record], 3.0, false);
            assert_eq!(summary.attempted, 1);
            assert_eq!(summary.answered_requests, 0);
            assert_eq!(
                summary.ttft_coverage.reasoning_truncated,
                usize::from(finish == "length")
            );
        }
        let mut unfinished = Consumer::default();
        event(
            &mut unfinished,
            json!({"choices":[{"index":0,"delta":{"reasoning":"think"}}]}),
            1000,
        )
        .unwrap();
        assert_eq!(
            unfinished.finish(Duration::from_secs(2), true).unwrap_err(),
            RequestError::StreamTruncated
        );
        let mut empty = Consumer::default();
        empty
            .feed(b"data: [DONE]\n\n", Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            empty
                .finish(Duration::from_secs(1), true)
                .unwrap()
                .output_error(),
            Some(RequestError::NoOutputToken)
        );
        assert_eq!(
            Consumer::default()
                .finish(Duration::from_secs(1), true)
                .unwrap_err(),
            RequestError::StreamTruncated
        );
    }

    // BC-SSE-004: missing, partial and unsplit usage must not fabricate rates or clocks.
    #[test]
    fn bc_sse_004_usage_only_and_incomplete_usage() {
        for (usage, complete, split, presence) in [
            (None, false, false, (false, false)),
            (
                Some(json!({"completion_tokens":8})),
                false,
                false,
                (false, true),
            ),
            (
                Some(json!({"prompt_tokens":4})),
                false,
                false,
                (true, false),
            ),
            (
                Some(json!({"prompt_tokens":4,"completion_tokens":8,"total_tokens":12})),
                true,
                false,
                (true, true),
            ),
            (
                Some(
                    json!({"prompt_tokens":4,"completion_tokens":8,"total_tokens":12,"completion_tokens_details":{"reasoning_tokens":3}}),
                ),
                true,
                true,
                (true, true),
            ),
        ] {
            let mut consumer = Consumer::default();
            if let Some(usage) = usage {
                event(&mut consumer, json!({"choices":[],"usage":usage}), 500).unwrap();
                assert!(
                    consumer.first_token_time.is_none() && consumer.first_reasoning_time.is_none()
                );
            }
            event(
                &mut consumer,
                json!({"choices":[{"index":0,"delta":{"content":"answer"}}]}),
                1000,
            )
            .unwrap();
            event(
                &mut consumer,
                json!({"choices":[{"index":0,"finish_reason":"stop"}]}),
                2000,
            )
            .unwrap();
            let result = consumer.finish(Duration::from_secs(2), true).unwrap();
            assert_eq!(result.ttft, Some(Duration::from_secs(1)));
            assert_eq!(result.usage_complete, complete);
            assert_eq!(
                (
                    result.prompt_tokens_present,
                    result.completion_tokens_present
                ),
                presence
            );
            let now = chrono::Utc::now();
            let output_error = result.output_error();
            let mut row = crate::record::RequestRecord::success(
                0,
                crate::record::Phase::Measure,
                "fixture".into(),
                now,
                now,
                Duration::from_secs(2),
                result.ttft,
                result.first_reasoning,
                result.itl,
                result.prompt_tokens,
                result.completion_tokens,
                result.total_tokens,
            )
            .with_reasoning_tokens(result.reasoning_tokens)
            .with_chat_outcome(output_error, false);
            row.usage_missing = !result.usage_complete;
            let summary = crate::summary::RunSummary::from_records(&[row], 10.0, false);
            assert_eq!(summary.usage_missing_count, usize::from(!complete));
            assert_eq!(
                summary.completion_tokens_per_second,
                complete.then_some(0.8)
            );
            assert_eq!(
                summary.non_reasoning_completion_tokens_per_second,
                split.then_some(0.5)
            );
        }
    }

    #[test]
    fn separated_usage_frames_cover_both_fields_without_starting_clock() {
        let mut consumer = Consumer::default();
        event(
            &mut consumer,
            json!({"choices":[],"usage":{"prompt_tokens":4}}),
            100,
        )
        .unwrap();
        event(
            &mut consumer,
            json!({"choices":[],"usage":{"completion_tokens":8}}),
            200,
        )
        .unwrap();
        assert!(consumer.first_reasoning_time.is_none() && consumer.first_token_time.is_none());
        event(
            &mut consumer,
            json!({"choices":[{"index":0,"finish_reason":"stop"}]}),
            300,
        )
        .unwrap();
        let result = consumer.finish(Duration::from_secs(1), true).unwrap();
        assert!(result.usage_complete);
        assert_eq!((result.prompt_tokens, result.completion_tokens), (4, 8));
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
            .finish(Duration::from_millis(90), false)
            .expect("complete without DONE");
        assert_eq!(result.first_reasoning, Some(Duration::from_millis(10)));
        assert_eq!(result.ttft, Some(Duration::from_millis(30)));
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
        assert_eq!(result.reasoning_tokens, None);
    }

    #[test]
    fn usage_chunk_reasoning_tokens_are_captured() {
        let mut consumer = Consumer::default();
        event(
            &mut consumer,
            json!({"choices":[{"delta":{"content":"hi"}}]}),
            10,
        )
        .expect("content");
        event(
            &mut consumer,
            json!({"choices":[{"finish_reason":"stop"}],"usage":{"prompt_tokens":2,"completion_tokens":9,"total_tokens":11,"completion_tokens_details":{"reasoning_tokens":6}}}),
            20,
        )
        .expect("finish");
        // A later usage-only chunk without details keeps the reported value.
        event(
            &mut consumer,
            json!({"choices":[],"usage":{"prompt_tokens":2,"completion_tokens":9,"total_tokens":11}}),
            25,
        )
        .expect("usage");
        let result = consumer
            .finish(Duration::from_millis(30), false)
            .expect("complete");
        assert_eq!(result.reasoning_tokens, Some(6));
        assert_eq!(result.completion_tokens, 9);
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

    /// PR #222 review: the body is read to its end after `[DONE]` so the
    /// connection can return to the pool, and the tail is not in latency.
    /// Both the 30 ms tail and the injected elapsed clock are virtual: host
    /// scheduling cannot consume the unchanged <30 ms latency budget.
    #[tokio::test(start_paused = true)]
    async fn consume_drains_body_after_done_outside_latency() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let ended = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&ended);
        let head = futures_util::stream::iter(vec![Ok::<_, reqwest::Error>(Vec::from(
            &b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n\n"[..],
        ))]);
        let tail = futures_util::stream::once(async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            Ok(Vec::new())
        })
        .chain(futures_util::stream::poll_fn(move |_| {
            flag.store(true, Ordering::SeqCst);
            std::task::Poll::Ready(None)
        }));
        let started = tokio::time::Instant::now();
        let result = consume_with_clock(head.chain(tail), || started.elapsed(), false, false)
            .await
            .expect("stream");
        assert!(ended.load(Ordering::SeqCst), "body end was not read");
        assert_eq!(started.elapsed(), Duration::from_millis(30));
        assert!(
            result.latency < Duration::from_millis(30),
            "{:?}",
            result.latency
        );
    }
}

#[cfg(test)]
mod trace_tests {
    use super::*;

    /// #313: tracing records categories and timestamps only — never content.
    #[tokio::test]
    async fn trace_records_categories_and_timestamps_never_content() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"secret thinking\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":1}}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let stream =
            futures_util::stream::iter(vec![Ok::<_, reqwest::Error>(bytes::Bytes::from(body))]);
        let result = consume_with_options(stream, Instant::now(), false, true)
            .await
            .expect("stream parses");
        let trace = result.trace.expect("trace enabled");
        let serialized = serde_json::to_string(&trace).unwrap();
        assert!(
            !serialized.contains("hi") && !serialized.contains("secret"),
            "no content in the trace: {serialized}"
        );
        let categories: Vec<&str> = trace
            .iter()
            .flat_map(|e| e.categories.iter().copied())
            .collect();
        assert!(categories.contains(&"reasoning"), "{categories:?}");
        assert!(categories.contains(&"content"), "{categories:?}");
        assert!(categories.contains(&"usage"), "{categories:?}");
        assert!(categories.contains(&"finish"), "{categories:?}");
        assert!(categories.contains(&"done"), "{categories:?}");
        // Indices are arrival-ordered and timestamps are ms from send.
        for (i, event) in trace.iter().enumerate() {
            assert_eq!(event.index as usize, i);
        }
    }

    #[tokio::test]
    async fn no_trace_without_the_flag() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n\n";
        let stream =
            futures_util::stream::iter(vec![Ok::<_, reqwest::Error>(bytes::Bytes::from(body))]);
        let result = consume_with_options(stream, Instant::now(), false, false)
            .await
            .expect("stream parses");
        assert!(result.trace.is_none());
    }
}
