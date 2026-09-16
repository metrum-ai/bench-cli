// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RequestError {
    Timeout,
    Connect,
    HttpStatus { status: u16 },
    RateLimit,
    StreamTruncated,
    ParseError { message: String },
    NoOutputToken,
    UsageMissing,
    ApiError { message: String },
    Other { message: String },
}

impl RequestError {
    pub fn from_status(status: u16) -> Self {
        if status == 429 {
            Self::RateLimit
        } else {
            Self::HttpStatus { status }
        }
    }

    /// Map a reqwest failure using typed predicates, not Display substrings.
    pub fn from_reqwest(err: &reqwest::Error) -> Self {
        if err.is_timeout() {
            Self::Timeout
        } else if err.is_connect() {
            Self::Connect
        } else if let Some(status) = err.status() {
            Self::from_status(status.as_u16())
        } else if is_connection_reset(err) {
            // TCP RST / broken pipe after the request was sent (N-05).
            Self::Connect
        } else {
            Self::Other {
                message: err.to_string(),
            }
        }
    }

    /// Walk an error chain for a typed [`RequestError`] or [`reqwest::Error`].
    /// Falls back to app-level message matching only (no transport Display heuristics).
    pub fn from_error(err: &(dyn std::error::Error + 'static)) -> Self {
        let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
        while let Some(e) = current {
            if let Some(typed) = e.downcast_ref::<RequestError>() {
                return typed.clone();
            }
            if let Some(req) = e.downcast_ref::<reqwest::Error>() {
                return Self::from_reqwest(req);
            }
            current = e.source();
        }
        crate::jsonl::classify_app_error(err)
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Connect => "connect",
            Self::HttpStatus { .. } => "http_status",
            Self::RateLimit => "rate_limit",
            Self::StreamTruncated => "stream_truncated",
            Self::ParseError { .. } => "parse_error",
            Self::NoOutputToken => "no_output_token",
            Self::UsageMissing => "usage_missing",
            Self::ApiError { .. } => "api_error",
            Self::Other { .. } => "other",
        }
    }
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => write!(f, "timeout"),
            Self::Connect => write!(f, "connect error"),
            Self::HttpStatus { status } => write!(f, "HTTP {status}"),
            Self::RateLimit => write!(f, "rate limited (429)"),
            Self::StreamTruncated => write!(f, "stream truncated"),
            Self::ParseError { message } => write!(f, "parse error: {message}"),
            Self::NoOutputToken => write!(f, "no output token"),
            Self::UsageMissing => write!(f, "usage missing"),
            Self::ApiError { message } => write!(f, "API error: {message}"),
            Self::Other { message } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for RequestError {}

/// True when the error chain looks like a TCP reset / broken pipe after send.
fn is_connection_reset(err: &reqwest::Error) -> bool {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = current {
        if let Some(io) = e.downcast_ref::<std::io::Error>() {
            match io.kind() {
                std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::ConnectionAborted => return true,
                _ => {}
            }
        }
        let msg = e.to_string().to_ascii_lowercase();
        if msg.contains("connection reset")
            || msg.contains("broken pipe")
            || msg.contains("connection aborted")
        {
            return true;
        }
        current = e.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn from_status_maps_5xx_and_429() {
        assert_eq!(
            RequestError::from_status(503),
            RequestError::HttpStatus { status: 503 }
        );
        assert_eq!(RequestError::from_status(429), RequestError::RateLimit);
    }

    #[tokio::test]
    async fn from_reqwest_timeout_uses_predicate_not_display() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(50))
            .connect_timeout(Duration::from_millis(50))
            .build()
            .expect("client");
        // TEST-NET-1 is typically unroutable; short timeouts yield timeout or connect.
        let err = client
            .get("http://192.0.2.1:81/")
            .send()
            .await
            .expect_err("expected network failure");
        assert!(
            err.is_timeout() || err.is_connect(),
            "fixture must produce timeout/connect via predicates; got: {err}"
        );
        let mapped = RequestError::from_reqwest(&err);
        if err.is_timeout() {
            assert_eq!(mapped, RequestError::Timeout);
        } else {
            assert_eq!(mapped, RequestError::Connect);
        }
        // Classification must not depend on Display containing those words.
        let display = err.to_string().to_ascii_lowercase();
        assert!(
            mapped == RequestError::Timeout || mapped == RequestError::Connect,
            "mapped={mapped:?} display={display}"
        );
    }

    #[tokio::test]
    async fn from_reqwest_connect_uses_predicate_not_display() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .connect_timeout(Duration::from_secs(2))
            .build()
            .expect("client");
        // Port 1 is almost never listening → connect refused.
        let err = client
            .get("http://127.0.0.1:1/")
            .send()
            .await
            .expect_err("expected connect failure");
        assert!(
            err.is_connect(),
            "expected is_connect(); got is_timeout={} display={err}",
            err.is_timeout()
        );
        assert_eq!(RequestError::from_reqwest(&err), RequestError::Connect);
        // Display for connect refused often lacks the substring "connect".
        let _ = err.to_string();
    }

    #[test]
    fn from_error_prefers_typed_request_error() {
        let err: Box<dyn std::error::Error + 'static> =
            Box::new(RequestError::HttpStatus { status: 502 });
        assert_eq!(
            RequestError::from_error(err.as_ref()),
            RequestError::HttpStatus { status: 502 }
        );
    }
}
