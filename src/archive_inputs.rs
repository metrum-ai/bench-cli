// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Pinned ZIP members parsed as data, never extracted under source filenames.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dataset_formats::{read_rows, DatasetFormat};
use crate::public_dataset::{IngestionLimits, SourceFile};

/// Exact derived member bytes and their explicit parser. No recursive archives.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchiveMember {
    pub file: SourceFile,
    pub format: DatasetFormat,
}

/// Preserve member lineage rather than flattening different source rows together.
#[derive(Debug, Clone)]
pub struct ArchiveRows {
    pub member: SourceFile,
    pub rows: Vec<Value>,
}

fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('/').all(|s| {
            !s.is_empty()
                && s != "."
                && s != ".."
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}

/// Verify the pinned outer archive and every selected member. Selected order is
/// explicit, not ZIP directory order. Byte/row/member-count budgets fail closed.
/// Symlinks/traversal names are rejected even though no source name is extracted.
pub fn read_zip_members(
    path: &Path,
    source: &SourceFile,
    selected: &[ArchiveMember],
    limits: IngestionLimits,
    max_archive_entries: usize,
) -> Result<Vec<ArchiveRows>> {
    ensure!(
        source.bytes <= limits.max_file_bytes.min(limits.max_total_bytes),
        "archive byte budget exceeded"
    );
    crate::public_dataset::verify_file(path, source)?;
    let mut archive =
        zip::ZipArchive::new(std::fs::File::open(path)?).context("invalid source ZIP")?;
    ensure!(
        archive.len() <= max_archive_entries,
        "archive entry budget exceeded"
    );
    let mut names = BTreeSet::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let entry_name = entry.name()?;
        let name = if entry.is_dir() {
            entry_name.strip_suffix('/').unwrap_or(&entry_name)
        } else {
            &entry_name
        };
        ensure!(
            safe_name(name) && names.insert(entry_name.to_string()),
            "invalid or duplicate archive path"
        );
        ensure!(
            !entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000),
            "archive symlinks are unsupported"
        );
    }
    let mut requested = BTreeSet::new();
    let mut total = 0u64;
    for member in selected {
        ensure!(
            safe_name(&member.file.path) && requested.insert(&member.file.path),
            "invalid or duplicate selected member path"
        );
        ensure!(
            member.file.bytes <= limits.max_file_bytes,
            "member byte budget exceeded"
        );
        total = total
            .checked_add(member.file.bytes)
            .context("archive member byte count overflow")?;
        ensure!(
            total <= limits.max_total_bytes,
            "archive aggregate uncompressed byte budget exceeded"
        );
    }
    ensure!(
        !selected.is_empty(),
        "explicit archive member selection required"
    );
    let mut output = Vec::new();
    let mut rows_left = limits.max_rows;
    for member in selected {
        let entry = archive.by_name(&member.file.path)?;
        ensure!(
            !entry.is_dir() && entry.size() == member.file.bytes,
            "archive member declared size mismatch"
        );
        let mut tmp = tempfile::NamedTempFile::new()?;
        let copied = std::io::copy(
            &mut entry.take(member.file.bytes.saturating_add(1)),
            &mut tmp,
        )?;
        ensure!(
            copied == member.file.bytes,
            "archive member actual size mismatch"
        );
        tmp.flush()?;
        let rows = read_rows(
            tmp.path(),
            member.format,
            &member.file,
            IngestionLimits {
                max_rows: rows_left,
                ..limits
            },
        )?;
        rows_left = rows_left
            .checked_sub(rows.len())
            .context("archive row budget exceeded")?;
        output.push(ArchiveRows {
            member: member.file.clone(),
            rows,
        });
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::public_dataset::sha256;
    fn limits() -> IngestionLimits {
        IngestionLimits {
            max_file_bytes: 16384,
            max_total_bytes: 16384,
            max_rows: 8,
        }
    }
    fn source(path: &Path) -> SourceFile {
        let bytes = std::fs::read(path).unwrap();
        SourceFile {
            path: "fixture.zip".into(),
            sha256: sha256(&bytes),
            bytes: bytes.len() as u64,
        }
    }
    fn member(path: &str, bytes: &[u8]) -> ArchiveMember {
        ArchiveMember {
            file: SourceFile {
                path: path.into(),
                sha256: sha256(bytes),
                bytes: bytes.len() as u64,
            },
            format: DatasetFormat::Jsonl,
        }
    }
    #[test]
    fn pinned_members_explicit_order_limits_and_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.zip");
        let one = b"{\"id\":1}\n";
        let two = b"{\"id\":2}\n";
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for (name, data) in [("a/one.jsonl", one), ("two.jsonl", two)] {
            writer
                .start_file(
                    name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )
                .unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
        let source = source(&path);
        let members = vec![member("two.jsonl", two), member("a/one.jsonl", one)];
        let rows = read_zip_members(&path, &source, &members, limits(), 8).unwrap();
        assert_eq!(rows[0].rows[0]["id"], 2);
        assert_eq!(rows[1].rows[0]["id"], 1);
        assert!(read_zip_members(&path, &source, &members, limits(), 1).is_err());
        assert!(read_zip_members(
            &path,
            &source,
            &members,
            IngestionLimits {
                max_rows: 1,
                ..limits()
            },
            8
        )
        .is_err());
        let mut bad = members.clone();
        bad[0].file.sha256 = "f".repeat(64);
        assert!(read_zip_members(&path, &source, &bad, limits(), 8).is_err());
        let mut bad = members.clone();
        bad[0].file.path = "../outside".into();
        assert!(read_zip_members(&path, &source, &bad, limits(), 8).is_err());
        let mut bad = members;
        bad[0].file.bytes += 1;
        assert!(read_zip_members(&path, &source, &bad, limits(), 8).is_err());
    }
    #[test]
    fn archive_symlinks_are_inert_and_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("symlink.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        writer
            .add_symlink(
                "link",
                "../outside",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.finish().unwrap();
        assert!(read_zip_members(
            &path,
            &source(&path),
            &[member("link", b"../outside")],
            limits(),
            8
        )
        .is_err());
        assert!(!dir.path().join("outside").exists());
    }
}
