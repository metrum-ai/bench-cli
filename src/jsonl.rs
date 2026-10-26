// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;

/// Collision policy for the run data log (#305).
///
/// Repeated commands append to the same data log by default: the file stays
/// valid NDJSON, but an unattended consumer expecting one run per file can
/// silently ingest several runs. `--data-log-mode` makes the policy
/// explicit before any benchmark traffic is issued.
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
#[value(rename_all = "kebab-case")]
pub enum DataLogMode {
    /// Default, unchanged behavior: append request and summary rows to the
    /// file. Each run keeps its unique `run_id`, so rows stay attributable.
    Append,
    /// Fail before any traffic when the data log path already exists.
    RefuseExisting,
    /// Truncate the existing data log before the run, so runs never mix.
    Overwrite,
    /// Treat the `--data-log` value as a directory (created if missing) and
    /// write one file per run, named by the run's unique `run_id`.
    NewDir,
}

/// Resolve the effective data-log path for a run (#305).
///
/// Returns the path `JsonlSink::create` should open. `RefuseExisting` and a
/// non-directory `NewDir` target fail before any traffic; `Overwrite`
/// truncates eagerly so a failed run cannot leave stale rows behind.
///
/// Ordering contract (#305 review): this must be the LAST configuration
/// step before traffic, because `Overwrite` truncates the previous log
/// immediately. Any pre-traffic validation (e.g. #304 `--validate-model`)
/// must run BEFORE this call so a failed validation never destroys an
/// existing log.
pub fn prepare_data_log(
    mode: DataLogMode,
    requested: &str,
    run_id: &str,
) -> Result<String, String> {
    match mode {
        DataLogMode::Append => Ok(requested.to_string()),
        DataLogMode::RefuseExisting => {
            // Atomic create (O_EXCL), not check-then-open (#305 review):
            // two concurrent runs pointed at the same fresh path cannot
            // both pass a `Path::exists` check and then interleave rows.
            // The sink appends to the file created here.
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(requested)
            {
                Ok(_) => Ok(requested.to_string()),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    Err(format!(
                        "data log '{requested}' already exists; use --data-log-mode overwrite to replace it, append to add this run to it, or new-dir for per-run files"
                    ))
                }
                Err(e) => Err(format!("failed to create data log '{requested}': {e}")),
            }
        }
        DataLogMode::Overwrite => {
            // Truncate now (creates if missing) so the run starts from an
            // empty log and no previous rows can mix into it.
            File::create(requested)
                .map_err(|e| format!("failed to truncate data log '{requested}': {e}"))?;
            Ok(requested.to_string())
        }
        DataLogMode::NewDir => {
            let dir = Path::new(requested);
            if dir.exists() && !dir.is_dir() {
                return Err(format!(
                    "data log '{requested}' exists and is not a directory; new-dir mode treats --data-log as a per-run directory"
                ));
            }
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("failed to create data log directory '{requested}': {e}"))?;
            Ok(dir
                .join(format!("{run_id}.jsonl"))
                .to_string_lossy()
                .into_owned())
        }
    }
}

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
    fn prepare_data_log_append_returns_requested_path() {
        assert_eq!(
            prepare_data_log(DataLogMode::Append, "out.jsonl", "run-1").unwrap(),
            "out.jsonl"
        );
        // Appending never touches the filesystem here; existing rows stay.
    }

    #[test]
    fn prepare_data_log_refuse_existing_is_atomic_two_preparers_one_path() {
        // O_EXCL semantics (#305 review): with a fresh path, the first
        // prepare creates the log atomically and the second must be refused
        // — check-then-open would let both through and interleave rows.
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("fresh.jsonl");
        assert!(
            prepare_data_log(DataLogMode::RefuseExisting, log.to_str().unwrap(), "run-1").is_ok(),
            "first preparer wins the fresh path"
        );
        let err = prepare_data_log(DataLogMode::RefuseExisting, log.to_str().unwrap(), "run-2")
            .expect_err("second preparer must be refused");
        assert!(
            err.contains("already exists"),
            "refusal must name the collision: {err}"
        );
    }

    #[test]
    fn prepare_data_log_refuse_existing_fails_only_when_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let fresh = dir.path().join("fresh.jsonl");
        // Missing file: allowed, path unchanged.
        assert_eq!(
            prepare_data_log(
                DataLogMode::RefuseExisting,
                fresh.to_str().unwrap(),
                "run-1"
            )
            .unwrap(),
            fresh.to_str().unwrap()
        );
        // Existing file: refused before any traffic.
        std::fs::write(&fresh, "previous run rows\n").unwrap();
        let err = prepare_data_log(
            DataLogMode::RefuseExisting,
            fresh.to_str().unwrap(),
            "run-2",
        )
        .unwrap_err();
        assert!(
            err.contains("already exists"),
            "refusal must name the collision: {err}"
        );
    }

    #[test]
    fn prepare_data_log_overwrite_truncates_previous_rows() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("out.jsonl");
        std::fs::write(&log, "stale rows from a previous run\n").unwrap();
        let path =
            prepare_data_log(DataLogMode::Overwrite, log.to_str().unwrap(), "run-1").unwrap();
        assert_eq!(path, log.to_str().unwrap());
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "",
            "overwrite must truncate so runs cannot mix"
        );
    }

    #[test]
    fn prepare_data_log_new_dir_writes_one_file_per_run() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("runs");
        // Directory is created when missing.
        let first = prepare_data_log(DataLogMode::NewDir, logs.to_str().unwrap(), "run-a").unwrap();
        let second =
            prepare_data_log(DataLogMode::NewDir, logs.to_str().unwrap(), "run-b").unwrap();
        assert!(first.ends_with("run-a.jsonl"), "{first}");
        assert!(second.ends_with("run-b.jsonl"), "{second}");
        assert_ne!(first, second, "each run gets its own file");
        // prepare_data_log creates the directory; the sink then creates the
        // per-run file, so at this point the dir exists and the file does
        // not yet.
        assert!(logs.is_dir(), "new-dir creates the directory");
        assert!(
            !Path::new(&first).exists(),
            "per-run file is not created eagerly"
        );
    }

    #[test]
    fn prepare_data_log_new_dir_rejects_existing_file_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("not-a-dir");
        std::fs::write(&target, "x").unwrap();
        let err =
            prepare_data_log(DataLogMode::NewDir, target.to_str().unwrap(), "run-1").unwrap_err();
        assert!(
            err.contains("not a directory"),
            "error must explain the new-dir contract: {err}"
        );
    }

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
