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

/// Thin fallback for app-level failure messages only.
///
/// Transport failures (timeout, connect, HTTP status) must be mapped at the
/// failure site via [`crate::error::RequestError::from_reqwest`] /
/// [`crate::error::RequestError::from_status`] — not here.
pub fn classify_app_error(err: &dyn std::error::Error) -> crate::error::RequestError {
    use crate::error::RequestError;
    let msg = err.to_string();
    let lower = msg.to_ascii_lowercase();
    if lower.contains("no output token") {
        RequestError::NoOutputToken
    } else if lower.contains("stream truncated") {
        RequestError::StreamTruncated
    } else if lower.contains("api error") {
        RequestError::ApiError { message: msg }
    } else {
        RequestError::Other { message: msg }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::RequestError;

    #[test]
    fn classifies_no_output_token() {
        let e = anyhow::anyhow!("no output token");
        assert!(matches!(
            classify_app_error(e.as_ref()),
            RequestError::NoOutputToken
        ));
    }

    #[test]
    fn classifies_stream_truncated() {
        let e = anyhow::anyhow!("stream truncated");
        assert!(matches!(
            classify_app_error(e.as_ref()),
            RequestError::StreamTruncated
        ));
    }

    #[test]
    fn does_not_map_timeout_via_display() {
        // Transport classification must not live here.
        let e = anyhow::anyhow!("error sending request for url (http://x): timeout");
        assert!(matches!(
            classify_app_error(e.as_ref()),
            RequestError::Other { .. }
        ));
    }
}
