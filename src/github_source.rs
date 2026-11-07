// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Anonymous pinned GitHub source transport, sharing verified ingestion cache.
//! Never route GitHub requests through Hub authentication or mirror fallbacks.
use crate::public_dataset::{
    copy_verified, verify_file, DatasetOrigin, IngestionLimits, RightsPolicy, SourceFile,
};
use anyhow::{ensure, Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};

fn file_url(origin: &DatasetOrigin, file: &SourceFile) -> Result<reqwest::Url> {
    origin.validate()?;
    ensure!(
        origin
            .files
            .iter()
            .any(|f| f.path == file.path && f.sha256 == file.sha256 && f.bytes == file.bytes),
        "file not in pinned origin"
    );
    let mut url = reqwest::Url::parse("https://media.githubusercontent.com/")?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("invalid source base URL"))?
        .push("media")
        .extend(origin.source_id.split('/'))
        .push(&origin.resolved_revision)
        .extend(file.path.split('/'));
    Ok(url)
}

/// Explicit GitHub domain, pinned revision, rights, budgets, shared content cache.
/// Local cache hits use the same length/digest verification as Hub materialization.
/// Network requests have NO Hub headers or authorization, including redirects.
pub fn materialize(
    origin: &DatasetOrigin,
    rights: &RightsPolicy,
    cache: &Path,
    offline: bool,
    limits: IngestionLimits,
) -> Result<Vec<PathBuf>> {
    origin.validate()?;
    rights.authorize()?;
    let mut total = 0u64;
    for file in &origin.files {
        ensure!(
            file.bytes <= limits.max_file_bytes,
            "source file limit exceeded"
        );
        total = total
            .checked_add(file.bytes)
            .context("source bytes overflow")?;
        ensure!(
            total <= limits.max_total_bytes,
            "aggregate source limit exceeded"
        );
    }
    std::fs::create_dir_all(cache)?;
    let mut result = Vec::new();
    for file in &origin.files {
        let destination = cache.join(file.sha256.to_ascii_lowercase());
        if destination.exists() {
            verify_file(&destination, file)?;
        } else {
            ensure!(!offline, "offline verified source cache miss");
            // Do not call hf_headers or inherit any authorization from a Hub client.
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() >= 5 {
                        return attempt.error("source redirect limit");
                    }
                    let url = attempt.url();
                    if url.scheme() != "https"
                        || !matches!(
                            url.host_str(),
                            Some(
                                "media.githubusercontent.com"
                                    | "raw.githubusercontent.com"
                                    | "github.com"
                            )
                        )
                    {
                        return attempt.error("source redirect outside declared domain");
                    }
                    attempt.follow()
                }))
                .build()?;
            let response = client
                .get(file_url(origin, file)?)
                .send()
                .context("public source download failed")?;
            ensure!(
                response.status().is_success(),
                "public source download status {}",
                response.status()
            );
            let mut temporary = tempfile::NamedTempFile::new_in(cache)?;
            copy_verified(response, &mut temporary, file)?;
            temporary.flush()?;
            temporary.persist(&destination).map_err(|e| e.error)?;
        }
        result.push(destination);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::public_dataset::{sha256, AccessDecision};
    fn fixture() -> (DatasetOrigin, RightsPolicy, IngestionLimits) {
        (
            DatasetOrigin {
                source_id: "example/synthetic-trace".into(),
                resolved_revision: "a".repeat(40),
                config: "default".into(),
                split: "test".into(),
                files: vec![SourceFile {
                    path: "trace.jsonl".into(),
                    sha256: sha256(b"abc"),
                    bytes: 3,
                }],
            },
            RightsPolicy {
                decision: AccessDecision::Allowed,
                license: Some("Apache-2.0".into()),
                license_text_sha256: Some("b".repeat(64)),
                component_terms: Default::default(),
                gated: false,
                terms_review: None,
                redistribution_allowed: false,
                privacy_handling: "synthetic test only".into(),
                unresolved: vec![],
            },
            IngestionLimits {
                max_file_bytes: 3,
                max_total_bytes: 3,
                max_rows: 1,
            },
        )
    }
    #[test]
    fn anonymous_url_is_pinned_and_cannot_escape_source_domain() {
        let (mut origin, _, _) = fixture();
        let url = file_url(&origin, &origin.files[0]).unwrap();
        assert_eq!(url.host_str(), Some("media.githubusercontent.com"));
        assert!(url.path().contains(&origin.resolved_revision));
        assert!(url.username().is_empty() && url.password().is_none() && url.query().is_none());
        origin.files[0].path = "../escape".into();
        assert!(file_url(&origin, &origin.files[0]).is_err());
    }
    #[test]
    fn offline_cache_uses_shared_verification_and_fails_closed() {
        let (origin, rights, limits) = fixture();
        let cache = tempfile::tempdir().unwrap();
        assert!(materialize(&origin, &rights, cache.path(), true, limits).is_err());
        let path = cache.path().join(&origin.files[0].sha256);
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            materialize(&origin, &rights, cache.path(), true, limits).unwrap(),
            vec![path.clone()]
        );
        assert!(materialize(
            &origin,
            &rights,
            cache.path(),
            true,
            IngestionLimits {
                max_total_bytes: 2,
                ..limits
            }
        )
        .is_err());
        std::fs::write(&path, b"bad").unwrap();
        assert!(materialize(&origin, &rights, cache.path(), true, limits).is_err());
    }
}
