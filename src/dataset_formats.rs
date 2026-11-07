// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Explicit pinned file formats; no dataset scripts or executable format sniffing.

use std::io::Read;
use std::path::Path;

use anyhow::{ensure, Context, Result};
use parquet::file::reader::{FileReader, SerializedFileReader};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::public_dataset::{IngestionLimits, SourceFile};

/// Only declared non-executable file formats are accepted.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DatasetFormat {
    JsonArray,
    Jsonl,
    Parquet,
}

/// Verify exact source bytes, then decode ordered JSON objects under byte/row
/// limits. Parquet metadata and decoded JSON budgets are checked separately.
/// Limits do not constitute a general allocator/process sandbox for parsers.
pub fn read_rows(
    path: &Path,
    format: DatasetFormat,
    source: &SourceFile,
    limits: IngestionLimits,
) -> Result<Vec<Value>> {
    ensure!(
        source.bytes <= limits.max_file_bytes.min(limits.max_total_bytes),
        "source file byte budget exceeded"
    );
    crate::public_dataset::verify_file(path, source)?;
    let rows = match format {
        DatasetFormat::Jsonl => {
            crate::public_dataset::read_jsonl(std::fs::File::open(path)?, limits)?
        }
        DatasetFormat::JsonArray => {
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(source.bytes.saturating_add(1))
                .read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 == source.bytes,
                "source changed while decoding"
            );
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|_| anyhow::anyhow!("invalid source JSON array"))?;
            let Value::Array(rows) = value else {
                anyhow::bail!("source JSON must be an array");
            };
            ensure!(rows.len() <= limits.max_rows, "source row budget exceeded");
            rows
        }
        DatasetFormat::Parquet => {
            let reader = SerializedFileReader::new(std::fs::File::open(path)?)
                .context("invalid source Parquet")?;
            let metadata = reader.metadata();
            let count = usize::try_from(metadata.file_metadata().num_rows())
                .context("invalid Parquet row count")?;
            ensure!(count <= limits.max_rows, "Parquet row budget exceeded");
            let mut uncompressed = 0u64;
            for group in metadata.row_groups() {
                let bytes =
                    u64::try_from(group.total_byte_size()).context("invalid Parquet group size")?;
                uncompressed = uncompressed
                    .checked_add(bytes)
                    .context("Parquet group byte count overflow")?;
                ensure!(
                    uncompressed <= limits.max_total_bytes,
                    "Parquet uncompressed metadata budget exceeded"
                );
            }
            let mut rows = Vec::with_capacity(count);
            let mut bytes = 0u64;
            for row in reader.get_row_iter(None)? {
                ensure!(
                    rows.len() < limits.max_rows,
                    "Parquet actual row budget exceeded"
                );
                let value = row?.to_json_value();
                bytes = bytes
                    .checked_add(serde_json::to_vec(&value)?.len() as u64)
                    .context("decoded Parquet byte count overflow")?;
                ensure!(
                    bytes <= limits.max_total_bytes,
                    "decoded Parquet byte budget exceeded"
                );
                rows.push(value);
            }
            ensure!(
                rows.len() == count,
                "Parquet declared/actual row count mismatch"
            );
            rows
        }
    };
    ensure!(
        rows.iter().all(Value::is_object),
        "dataset source rows must be objects"
    );
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::public_dataset::sha256;
    fn limits() -> IngestionLimits {
        IngestionLimits {
            max_file_bytes: 65536,
            max_total_bytes: 65536,
            max_rows: 8,
        }
    }
    fn source(path: &Path) -> SourceFile {
        let bytes = std::fs::read(path).unwrap();
        SourceFile {
            path: "fixture".into(),
            sha256: sha256(&bytes),
            bytes: bytes.len() as u64,
        }
    }
    #[test]
    fn local_json_formats_unicode_order_missing_and_malformed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture");
        std::fs::write(&path, "[{\"text\":\"é\"},{\"text\":\"two\"}]").unwrap();
        let rows = read_rows(&path, DatasetFormat::JsonArray, &source(&path), limits()).unwrap();
        assert_eq!(rows[0]["text"], "é");
        assert_eq!(rows[1]["text"], "two");
        assert!(read_rows(
            &path,
            DatasetFormat::JsonArray,
            &source(&path),
            IngestionLimits {
                max_rows: 1,
                ..limits()
            }
        )
        .is_err());
        std::fs::write(&path, "{\"text\":\"é\"}\n{\"text\":\"two\"}\n").unwrap();
        assert_eq!(
            read_rows(&path, DatasetFormat::Jsonl, &source(&path), limits()).unwrap(),
            rows
        );
        for bytes in [b"{}".as_slice(), b"[", b"[1]"] {
            std::fs::write(&path, bytes).unwrap();
            assert!(read_rows(&path, DatasetFormat::JsonArray, &source(&path), limits()).is_err());
        }
        assert!(read_rows(
            &dir.path().join("missing"),
            DatasetFormat::Jsonl,
            &source(&path),
            limits()
        )
        .is_err());
        let mut bad = source(&path);
        bad.sha256 = "a".repeat(64);
        assert!(read_rows(&path, DatasetFormat::JsonArray, &bad, limits()).is_err());
    }
    #[test]
    fn locally_written_pinned_parquet_and_limits() {
        use parquet::{
            data_type::{ByteArray, ByteArrayType},
            file::writer::SerializedFileWriter,
            schema::parser::parse_message_type,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.parquet");
        let schema =
            parse_message_type("message fixture { REQUIRED BINARY text (UTF8); }").unwrap();
        let mut writer = SerializedFileWriter::new(
            std::fs::File::create(&path).unwrap(),
            std::sync::Arc::new(schema),
            Default::default(),
        )
        .unwrap();
        let mut group = writer.next_row_group().unwrap();
        let mut column = group.next_column().unwrap().unwrap();
        column
            .typed::<ByteArrayType>()
            .write_batch(&[ByteArray::from("one"), ByteArray::from("é")], None, None)
            .unwrap();
        column.close().unwrap();
        group.close().unwrap();
        writer.close().unwrap();
        let file = source(&path);
        let rows = read_rows(&path, DatasetFormat::Parquet, &file, limits()).unwrap();
        assert_eq!(rows[0]["text"], "one");
        assert_eq!(rows[1]["text"], "é");
        assert!(read_rows(
            &path,
            DatasetFormat::Parquet,
            &file,
            IngestionLimits {
                max_rows: 1,
                ..limits()
            }
        )
        .is_err());
        assert!(read_rows(
            &path,
            DatasetFormat::Parquet,
            &file,
            IngestionLimits {
                max_file_bytes: 1,
                ..limits()
            }
        )
        .is_err());
        std::fs::write(&path, "not parquet").unwrap();
        assert!(read_rows(&path, DatasetFormat::Parquet, &source(&path), limits()).is_err());
    }
}
