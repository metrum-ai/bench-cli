// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::sync::Mutex;

/// Append-only JSONL writer, safe to share across request tasks via `Arc`.
pub struct JsonlSink {
    inner: Mutex<BufWriter<File>>,
}

impl JsonlSink {
    pub fn create(path: &str) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            inner: Mutex::new(BufWriter::new(file)),
        })
    }

    pub fn write(&self, value: &impl Serialize) -> anyhow::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|e| anyhow::anyhow!("jsonl sink poisoned: {e}"))?;
        serde_json::to_writer(&mut *guard, value)?;
        guard.write_all(b"\n")?;
        guard.flush()?;
        Ok(())
    }
}

pub fn classify_error(err: &dyn std::error::Error) -> crate::error::RequestError {
    use crate::error::RequestError;
    let msg = err.to_string();
    let lower = msg.to_ascii_lowercase();
    if lower.contains("no output token") {
        RequestError::NoOutputToken
    } else if lower.contains("stream truncated") || lower.contains("truncated") {
        RequestError::StreamTruncated
    } else if lower.contains("timeout") {
        RequestError::Timeout
    } else if lower.contains("connect") {
        RequestError::Connect
    } else if lower.contains("429") || lower.contains("rate_limit") {
        RequestError::RateLimit
    } else if let Some(status) = parse_http_status(&lower) {
        RequestError::from_status(status)
    } else if lower.contains("api error") {
        RequestError::ApiError { message: msg }
    } else {
        RequestError::Other { message: msg }
    }
}

fn parse_http_status(lower: &str) -> Option<u16> {
    let idx = lower.find("http error:")?;
    let rest = lower[idx + "http error:".len()..].trim();
    let code: u16 = rest
        .split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())?
        .parse()
        .ok()?;
    Some(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::RequestError;

    #[test]
    fn classifies_no_output_token() {
        let e = anyhow::anyhow!("no output token");
        assert!(matches!(
            classify_error(e.as_ref()),
            RequestError::NoOutputToken
        ));
    }
}
