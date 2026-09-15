// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Complete-line SSE framing. Never concatenates distinct `data:` payloads.
//! Partial lines are held until the next `feed`. UTF-8 is decoded per complete line.

use serde_json::Value;

/// One parsed SSE `data:` payload (or the `[DONE]` sentinel).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseEvent {
    Done,
    Json(Value),
    /// Non-JSON payload that is not `[DONE]`.
    Raw(String),
}

#[derive(Debug, Default)]
pub struct SseParser {
    pending: Vec<u8>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ingest a TCP/HTTP chunk and return every complete event found.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.pending.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(nl) = self.pending.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.pending.drain(..=nl).collect();
            if line.last() == Some(&b'\n') {
                line.pop();
            }
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if let Some(ev) = parse_line(&line) {
                events.push(ev);
            }
        }
        events
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

fn parse_line(line: &[u8]) -> Option<SseEvent> {
    let text = String::from_utf8_lossy(line);
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.starts_with(':') {
        return None;
    }
    let payload = if let Some(rest) = trimmed.strip_prefix("data: ") {
        rest
    } else {
        let rest = trimmed.strip_prefix("data:")?;
        rest.trim_start()
    };
    if payload == "[DONE]" {
        return Some(SseEvent::Done);
    }
    match serde_json::from_str::<Value>(payload) {
        Ok(v) => Some(SseEvent::Json(v)),
        Err(_) => Some(SseEvent::Raw(payload.to_string())),
    }
}

/// True when a chat/completions stream choice carries a visible output token
/// (`delta.content`, `text`, or tool-call arguments). Role-only chunks are false.
pub fn choice_has_output_token(choice: &Value) -> bool {
    if choice_output_text(choice).is_some_and(|c| !c.is_empty()) {
        return true;
    }
    let Some(delta) = choice.get("delta") else {
        return false;
    };
    delta
        .get("function_call")
        .and_then(|fc| fc.get("arguments"))
        .and_then(|a| a.as_str())
        .is_some_and(|a| !a.is_empty())
        || delta
            .get("tool_calls")
            .and_then(|t| t.as_array())
            .is_some_and(|calls| {
                calls.iter().any(|tc| {
                    tc.get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|a| a.as_str())
                        .is_some_and(|a| !a.is_empty())
                })
            })
}

/// True when a choice carries `reasoning_content` / `reasoning` (not visible text).
pub fn choice_has_reasoning_token(choice: &Value) -> bool {
    let Some(delta) = choice.get("delta") else {
        return false;
    };
    ["reasoning_content", "reasoning"].iter().any(|k| {
        delta
            .get(*k)
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty())
    })
}

pub fn choice_output_text(choice: &Value) -> Option<&str> {
    choice
        .get("delta")
        .and_then(|d| d.get("content"))
        .and_then(|c| c.as_str())
        .or_else(|| choice.get("text").and_then(|t| t.as_str()))
}

pub fn choice_finish_reason(choice: &Value) -> Option<&str> {
    choice.get("finish_reason").and_then(|v| v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_across_two_feeds_is_reassembled() {
        let mut p = SseParser::new();
        let first = p.feed(b"data: {\"id\":");
        assert!(first.is_empty());
        let second = p.feed(b"1}\n");
        assert_eq!(second.len(), 1);
        match &second[0] {
            SseEvent::Json(v) => assert_eq!(v["id"], 1),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn two_events_in_one_feed() {
        let mut p = SseParser::new();
        let ev = p.feed(b"data: {\"a\":1}\n\ndata: {\"b\":2}\n");
        assert_eq!(ev.len(), 2);
    }

    #[test]
    fn utf8_multibyte_split_round_trips() {
        let cjk = "data: {\"c\":\"你\"}\n".as_bytes();
        for i in 1..cjk.len() {
            let mut p = SseParser::new();
            let _ = p.feed(&cjk[..i]);
            let rest = p.feed(&cjk[i..]);
            assert_eq!(rest.len(), 1, "split at {i}");
        }
    }

    #[test]
    fn done_event() {
        let mut p = SseParser::new();
        let ev = p.feed(b"data: [DONE]\n");
        assert_eq!(ev, vec![SseEvent::Done]);
    }

    #[test]
    fn role_only_is_not_output_token() {
        let choice = serde_json::json!({"delta": {"role": "assistant"}});
        assert!(!choice_has_output_token(&choice));
        assert!(!choice_has_reasoning_token(&choice));
    }

    #[test]
    fn reasoning_content_detected() {
        let choice = serde_json::json!({"delta": {"reasoning_content": "think"}});
        assert!(!choice_has_output_token(&choice));
        assert!(choice_has_reasoning_token(&choice));
    }

    #[test]
    fn content_is_output_token() {
        let choice = serde_json::json!({"delta": {"content": "hi"}});
        assert!(choice_has_output_token(&choice));
    }

    #[test]
    fn continuation_without_data_prefix_is_dropped() {
        // Simulates a split *event* where the second TCP read lacks `data: `.
        let mut p = SseParser::new();
        let first = p.feed(b"data: {\"choices\"\n");
        assert!(matches!(&first[0], SseEvent::Raw(_)));
        let ev = p.feed(b":[]}\n");
        assert!(ev.is_empty());
    }

    #[test]
    fn crlf_line_endings() {
        let mut p = SseParser::new();
        let ev = p.feed(b"data: {\"x\":1}\r\n");
        assert_eq!(ev.len(), 1);
    }

    #[test]
    fn comments_id_and_event_lines_are_ignored() {
        let mut p = SseParser::new();
        let ev = p.feed(b": keepalive\nid: 7\nevent: message\ndata:{\"x\":1}\n");
        assert_eq!(ev, vec![SseEvent::Json(serde_json::json!({"x": 1}))]);
    }

    #[test]
    fn large_event_is_not_truncated() {
        let content = "x".repeat(64 * 1024);
        let line = format!("data: {{\"content\":\"{content}\"}}\n");
        let mut p = SseParser::new();
        let ev = p.feed(line.as_bytes());
        assert_eq!(ev.len(), 1);
        assert_eq!(
            match &ev[0] {
                SseEvent::Json(value) => value["content"].as_str().unwrap().len(),
                _ => 0,
            },
            content.len()
        );
    }

    #[test]
    fn malformed_payload_is_counted_as_raw_without_poisoning_next_event() {
        let mut p = SseParser::new();
        let ev = p.feed(b"data: {bad}\ndata: {\"ok\":true}\n");
        assert!(matches!(ev[0], SseEvent::Raw(_)));
        assert_eq!(ev[1], SseEvent::Json(serde_json::json!({"ok": true})));
    }

    #[test]
    fn api_error_object_remains_structured() {
        let mut p = SseParser::new();
        let ev = p.feed(b"data: {\"error\":{\"message\":\"overloaded\"}}\n");
        match &ev[0] {
            SseEvent::Json(value) => assert_eq!(value["error"]["message"], "overloaded"),
            _ => panic!("expected JSON API error"),
        }
    }
}
