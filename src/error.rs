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
