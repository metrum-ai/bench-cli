// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Publish an already-produced campaign manifest into the docs tree.

use anyhow::{bail, Context, Result};
use clap::Args;
use serde_json::Value;
use std::{fs, path::PathBuf};

#[derive(Args)]
pub struct ManifestArgs {
    /// Manifest produced by the campaign run.
    #[arg(long)]
    source: PathBuf,
    /// Campaign identifier used as the destination directory.
    #[arg(long)]
    campaign_id: String,
    #[arg(long, default_value = ".")]
    root: PathBuf,
}

fn has_credential_fields(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, child)| {
            let key = key.to_ascii_lowercase();
            key.contains("password")
                || key == "token"
                || key.ends_with("_token")
                || key.contains("key")
                || key.contains("private")
                || key.contains("secret")
                || has_credential_fields(child)
        }),
        Value::Array(items) => items.iter().any(has_credential_fields),
        _ => false,
    }
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !id.starts_with('.')
}

pub fn run(args: ManifestArgs) -> Result<()> {
    if !safe_id(&args.campaign_id) {
        bail!("invalid campaign id; use letters, numbers, dot, underscore, and hyphen");
    }
    let contents =
        fs::read(&args.source).with_context(|| format!("read {}", args.source.display()))?;
    let manifest: Value =
        serde_json::from_slice(&contents).context("manifest must be valid JSON")?;
    if !manifest.is_object() {
        bail!("campaign manifest must be a JSON object");
    }
    if has_credential_fields(&manifest) {
        bail!("campaign manifest contains credential-like fields; refusing to publish it");
    }
    if let Some(recorded_id) = manifest["campaign_id"].as_str() {
        if recorded_id != args.campaign_id {
            bail!("manifest campaign_id does not match the requested destination");
        }
    } else {
        bail!("campaign manifest is missing campaign_id");
    }
    let smoke_root = args.root.join("docs/smoke");
    fs::create_dir_all(&smoke_root)?;
    if fs::symlink_metadata(&smoke_root)?.file_type().is_symlink() {
        bail!("docs/smoke must not be a symlink");
    }
    let directory = smoke_root.join(&args.campaign_id);
    if directory.exists() && fs::symlink_metadata(&directory)?.file_type().is_symlink() {
        bail!("campaign destination must not be a symlink");
    }
    fs::create_dir_all(&directory)?;
    let destination = directory.join("manifest.json");
    fs::write(
        &destination,
        format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )?;
    println!("wrote {}", destination.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_like_fields_are_rejected_without_echoing_values() {
        let value =
            serde_json::json!({"campaign_id":"id","nested":{"api_token":"not-a-real-secret"}});
        assert!(has_credential_fields(&value));
    }

    #[test]
    fn campaign_ids_cannot_escape_docs_tree() {
        for invalid in ["", "../escape", "a/b", ".hidden", "has space"] {
            assert!(!safe_id(invalid), "{invalid}");
        }
        assert!(safe_id("matrix-20261023_01"));
    }

    #[test]
    fn manifest_is_validated_and_copied_to_campaign_directory() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("repo");
        let source = directory.path().join("manifest.json");
        fs::write(&source, r#"{"campaign_id":"matrix-test","cells":[]}"#).unwrap();
        run(ManifestArgs {
            source: source.clone(),
            campaign_id: "matrix-test".into(),
            root: root.clone(),
        })
        .unwrap();
        let saved: Value = serde_json::from_slice(
            &fs::read(root.join("docs/smoke/matrix-test/manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved["campaign_id"], "matrix-test");
        assert!(run(ManifestArgs {
            source,
            campaign_id: "different".into(),
            root
        })
        .is_err());
    }
}
