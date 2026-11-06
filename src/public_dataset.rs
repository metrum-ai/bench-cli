// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Versioned public-input provenance and bounded, pinned Hub transport.
//! Ingestion never executes dataset scripts, tools, or repository code.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Schema version for canonical inputs and validation manifests.
pub const SCHEMA_VERSION: &str = "kv-dataset-manifest.v1";

/// Digest of exact bytes (not normalized text).
pub fn sha256(bytes: &[u8]) -> String {
    crate::prompt_library::hex_digest(Sha256::digest(bytes).as_slice())
}

fn hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn safe_segments(value: &str) -> bool {
    !value.is_empty()
        && value.split('/').all(|s| {
            !s.is_empty()
                && s != "."
                && s != ".."
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}

/// Exact source file, including a caller-supplied digest independent of Hub metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceFile {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// Immutable dataset selection. Config/split are metadata, never inferred from filenames.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DatasetOrigin {
    pub source_id: String,
    pub resolved_revision: String,
    pub config: String,
    pub split: String,
    pub files: Vec<SourceFile>,
}

impl DatasetOrigin {
    /// Validate all URL/cache components before network access.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            safe_segments(&self.source_id) && self.source_id.split('/').count() == 2,
            "invalid dataset ID"
        );
        crate::prompt_library::resolve_revision(&self.source_id, &self.resolved_revision, false)?;
        ensure!(hex(&self.resolved_revision, 40), "revision must be pinned");
        ensure!(
            !self.config.is_empty() && !self.split.is_empty(),
            "config and split are required"
        );
        ensure!(!self.files.is_empty(), "source files are required");
        let mut paths = std::collections::BTreeSet::new();
        for file in &self.files {
            ensure!(safe_segments(&file.path), "invalid source path");
            ensure!(hex(&file.sha256, 64), "invalid file digest");
            ensure!(paths.insert(&file.path), "duplicate source path");
        }
        Ok(())
    }
}

/// Access policy must be reviewed before payload access. Unknown is not permissive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessDecision {
    Allowed,
    Denied,
    ReviewRequired,
}

/// Rights and privacy metadata. No credentials or acceptance tokens belong here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RightsPolicy {
    pub decision: AccessDecision,
    pub license: Option<String>,
    pub license_text_sha256: Option<String>,
    pub component_terms: BTreeMap<String, String>,
    pub gated: bool,
    pub terms_review: Option<String>,
    pub redistribution_allowed: bool,
    pub privacy_handling: String,
    pub unresolved: Vec<String>,
}

impl RightsPolicy {
    /// Fail closed; explicit policy review is required for unknown component rights.
    pub fn authorize(&self) -> Result<()> {
        ensure!(
            self.decision == AccessDecision::Allowed,
            "source access denied or requires review"
        );
        ensure!(
            self.license.as_ref().is_some_and(|s| !s.trim().is_empty()),
            "unknown license requires review"
        );
        ensure!(
            self.license_text_sha256
                .as_ref()
                .is_some_and(|s| hex(s, 64)),
            "license text digest required"
        );
        ensure!(
            !self.privacy_handling.trim().is_empty(),
            "privacy policy required"
        );
        ensure!(self.unresolved.is_empty(), "unresolved source rights");
        ensure!(
            !self.gated
                || self
                    .terms_review
                    .as_ref()
                    .is_some_and(|s| !s.trim().is_empty()),
            "gated source requires recorded terms review"
        );
        Ok(())
    }
}

/// Bounded ingestion budget, checked on cache hits as well as downloads.
#[derive(Debug, Clone, Copy)]
pub struct IngestionLimits {
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_rows: usize,
}

/// Fetch immutable files into a content-addressed cache. No mirror fallback.
/// A corrupt cache entry fails closed rather than silently replacing evidence.
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
            "file byte budget exceeded"
        );
        total = total
            .checked_add(file.bytes)
            .context("source byte count overflow")?;
        ensure!(
            total <= limits.max_total_bytes,
            "total byte budget exceeded"
        );
    }
    std::fs::create_dir_all(cache)?;
    let mut paths = Vec::new();
    for file in &origin.files {
        let dest = cache.join(file.sha256.to_ascii_lowercase());
        if dest.exists() {
            verify_file(&dest, file)?;
        } else {
            ensure!(!offline, "offline cache miss for {}", file.path);
            let url = hub_file_url(origin, file)?;
            let client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()?;
            let response = client
                .get(url)
                .headers(crate::prompt_library::hf_headers())
                .send()
                .context("Hub request failed")?;
            ensure!(
                response.status().is_success(),
                "Hub download returned {}",
                response.status()
            );
            let mut tmp = tempfile::NamedTempFile::new_in(cache)?;
            copy_verified(response, &mut tmp, file)?;
            tmp.flush()?;
            tmp.persist(&dest).map_err(|e| e.error)?;
        }
        paths.push(dest);
    }
    Ok(paths)
}

fn hub_file_url(origin: &DatasetOrigin, file: &SourceFile) -> Result<reqwest::Url> {
    origin.validate()?;
    ensure!(safe_segments(&file.path), "invalid source path");
    ensure!(hex(&file.sha256, 64), "invalid file digest");
    let mut url = reqwest::Url::parse("https://huggingface.co/")?;
    // URL's segment API percent-encodes each component, rather than treating
    // source metadata as URL syntax. Validation remains deliberately stricter.
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("invalid Hub base URL"))?
        .extend(["datasets"])
        .extend(origin.source_id.split('/'))
        .extend(["resolve", &origin.resolved_revision])
        .extend(file.path.split('/'));
    Ok(url)
}

fn copy_verified(reader: impl Read, writer: &mut impl Write, file: &SourceFile) -> Result<()> {
    let mut limited = reader.take(
        file.bytes
            .checked_add(1)
            .context("file byte count overflow")?,
    );
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut buf = [0u8; 8192];
    loop {
        let n = limited.read(&mut buf)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        ensure!(count <= file.bytes, "download exceeds declared size");
        hash.update(&buf[..n]);
        writer.write_all(&buf[..n])?;
    }
    ensure!(count == file.bytes, "truncated source file");
    ensure!(
        crate::prompt_library::hex_digest(hash.finalize().as_slice())
            .eq_ignore_ascii_case(&file.sha256),
        "source checksum mismatch"
    );
    Ok(())
}

fn verify_file(path: &Path, file: &SourceFile) -> Result<()> {
    ensure!(
        std::fs::metadata(path)?.len() == file.bytes,
        "cached source size mismatch"
    );
    copy_verified(std::fs::File::open(path)?, &mut std::io::sink(), file)
}

/// Explicit body type; answers and patches are not part of the request payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanonicalBody {
    Prompt {
        text: String,
    },
    DocumentQuestion {
        document_id: String,
        document: String,
        question: String,
    },
    Session {
        session_id: String,
        history_mode: HistoryMode,
        messages: Vec<Message>,
    },
    StructuralTrace {
        timestamp: u64,
        time_unit: TimeUnit,
        input_tokens: u64,
        output_tokens: u64,
        block_size: Option<u64>,
        prefix_hashes: Option<Vec<String>>,
        post_template: bool,
    },
}

/// Replay history does not guarantee live generated-token identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryMode {
    Recorded,
    Generated,
}

/// Source timestamp units, never inferred from magnitude.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimeUnit {
    Seconds,
    Milliseconds,
    Microseconds,
}

/// Typed role; arbitrary role strings are rejected by serde.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// Tool calls are inert data, never executable ingestion instructions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub tool_call_ids: Vec<String>,
    pub tool_result_id: Option<String>,
}

/// Stable lineage for one canonical target; a source row can yield many targets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalRecord {
    pub record_id: String,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub source_row_id: String,
    pub source_file_sha256: String,
    pub group_id: Option<String>,
    pub turn_index: Option<u64>,
    pub output_budget: Option<u64>,
    pub body: CanonicalBody,
}

impl CanonicalRecord {
    /// Check source linkage and structural invariants without executing payloads.
    pub fn validate(&self, origin: &DatasetOrigin) -> Result<()> {
        ensure!(
            !self.record_id.is_empty() && !self.source_row_id.is_empty(),
            "record identity required"
        );
        ensure!(
            origin
                .files
                .iter()
                .any(|f| f.sha256.eq_ignore_ascii_case(&self.source_file_sha256)),
            "record file digest is not in origin"
        );
        match &self.body {
            CanonicalBody::Prompt { text } => ensure!(!text.trim().is_empty(), "empty prompt"),
            CanonicalBody::DocumentQuestion {
                document_id,
                document,
                question,
            } => {
                ensure!(
                    !document_id.is_empty()
                        && !document.trim().is_empty()
                        && !question.trim().is_empty(),
                    "empty document/question"
                );
            }
            CanonicalBody::Session {
                session_id,
                messages,
                ..
            } => {
                ensure!(
                    !session_id.is_empty() && !messages.is_empty(),
                    "empty session"
                );
                let mut pending = std::collections::BTreeSet::new();
                let mut used = std::collections::BTreeSet::new();
                for message in messages {
                    ensure!(
                        !message.content.trim().is_empty() || !message.tool_call_ids.is_empty(),
                        "empty message"
                    );
                    ensure!(
                        message.tool_call_ids.is_empty() || message.role == Role::Assistant,
                        "only assistant may call tools"
                    );
                    ensure!(
                        message.tool_result_id.is_none() || message.role == Role::Tool,
                        "only tool may supply result ID"
                    );
                    if message.role == Role::Tool {
                        let id = message
                            .tool_result_id
                            .as_ref()
                            .context("tool result ID required")?;
                        ensure!(pending.remove(id), "unmatched tool result");
                    } else {
                        ensure!(
                            pending.is_empty(),
                            "tool calls must be resolved before next message"
                        );
                    }
                    for id in &message.tool_call_ids {
                        ensure!(
                            !id.is_empty() && used.insert(id),
                            "duplicate or empty tool call ID"
                        );
                        pending.insert(id);
                    }
                }
                ensure!(pending.is_empty(), "unresolved tool calls");
            }
            CanonicalBody::StructuralTrace {
                input_tokens,
                block_size,
                prefix_hashes,
                ..
            } => {
                if let Some(hashes) = prefix_hashes {
                    let block = block_size.context("hash trace requires block size")?;
                    ensure!(block > 0, "zero source block size");
                    let count = input_tokens / block + u64::from(input_tokens % block != 0);
                    ensure!(
                        hashes.len() as u64 == count,
                        "prefix hash count does not cover input length"
                    );
                    ensure!(
                        hashes.iter().all(|h| !h.is_empty()),
                        "empty source block hash"
                    );
                } else {
                    ensure!(block_size.is_none_or(|b| b > 0), "zero source block size");
                }
            }
        }
        Ok(())
    }
}

/// Measurement status distinguishes publisher metadata from measured evidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum Evidence<T> {
    Measured(T),
    Publisher(T),
    Unknown,
}

/// Digest reference to the separately serialized validation manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ValidationManifestRef {
    pub schema_version: String,
    pub sha256: String,
}

/// Exact report envelope shared with the KV study report contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DatasetEnvelope {
    pub source_id: String,
    pub resolved_revision: String,
    pub config: String,
    pub split: String,
    pub files: Vec<SourceFile>,
    pub adapter_id: String,
    pub adapter_version: String,
    pub license_spdx: Option<String>,
    pub validation_manifest: ValidationManifestRef,
}

/// Shared contract with report consumers. Token measurements are attached by #422.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ValidationManifest {
    pub schema_version: String,
    pub origin: DatasetOrigin,
    pub rights: RightsPolicy,
    pub adapter_id: String,
    pub adapter_version: String,
    pub field_mapping: BTreeMap<String, String>,
    pub transformations: Vec<String>,
    pub tokenizer_revision: Option<String>,
    pub chat_template_sha256: Option<String>,
    pub generator_version: Option<String>,
    pub corpus_rows: Evidence<u64>,
    pub selected_source_rows: u64,
    pub target_count: u64,
    pub selected_ids_sha256: String,
    pub body_order_sha256: String,
    pub checks: BTreeMap<String, Evidence<String>>,
    pub exclusions: Vec<String>,
    pub limitations: Vec<String>,
}

impl ValidationManifest {
    /// Fingerprint all contract fields; ordered maps make serialization deterministic.
    pub fn fingerprint(&self) -> Result<String> {
        ensure!(
            self.schema_version == SCHEMA_VERSION,
            "unsupported dataset schema version"
        );
        self.origin.validate()?;
        // Denied/unknown policies are still reportable; only payload access is gated.
        ensure!(
            !self.adapter_id.is_empty()
                && !self.adapter_version.is_empty()
                && !self.field_mapping.is_empty(),
            "adapter ID, version and mapping required"
        );
        ensure!(
            self.chat_template_sha256
                .as_ref()
                .is_none_or(|s| hex(s, 64)),
            "invalid template digest"
        );
        ensure!(
            hex(&self.selected_ids_sha256, 64) && hex(&self.body_order_sha256, 64),
            "invalid selection fingerprint"
        );
        Ok(sha256(&serde_json::to_vec(self)?))
    }

    /// Build the exact envelope consumed by study reports; never inline raw rows.
    pub fn envelope(&self) -> Result<DatasetEnvelope> {
        Ok(DatasetEnvelope {
            source_id: self.origin.source_id.clone(),
            resolved_revision: self.origin.resolved_revision.to_ascii_lowercase(),
            config: self.origin.config.clone(),
            split: self.origin.split.clone(),
            files: self
                .origin
                .files
                .iter()
                .map(|file| SourceFile {
                    path: file.path.clone(),
                    sha256: file.sha256.to_ascii_lowercase(),
                    bytes: file.bytes,
                })
                .collect(),
            adapter_id: self.adapter_id.clone(),
            adapter_version: self.adapter_version.clone(),
            license_spdx: self.rights.license.clone(),
            validation_manifest: ValidationManifestRef {
                schema_version: self.schema_version.clone(),
                sha256: self.fingerprint()?,
            },
        })
    }

    /// Numeric/metadata summary only: raw payload and source identifiers are omitted.
    pub fn summary(&self) -> String {
        format!("Dataset validation v{}: {} selected source rows; {} targets. Corpus count: {:?}. {} checks; {} limitations. Public availability is not privacy clearance. Logical overlap is not observed cache hits.", self.schema_version, self.selected_source_rows, self.target_count, self.corpus_rows, self.checks.len(), self.limitations.len())
    }
}

/// Bounded JSONL decoding in source order. No scripts or format auto-detection.
pub fn read_jsonl(reader: impl Read, limits: IngestionLimits) -> Result<Vec<serde_json::Value>> {
    use std::io::BufRead;
    let mut input = std::io::BufReader::new(reader.take(limits.max_file_bytes.saturating_add(1)));
    let mut rows = Vec::new();
    let mut total = 0u64;
    loop {
        let mut line = Vec::new();
        let n = input.read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        ensure!(
            total <= limits.max_file_bytes.min(limits.max_total_bytes),
            "JSONL byte budget exceeded"
        );
        ensure!(rows.len() < limits.max_rows, "JSONL row budget exceeded");
        if line.iter().all(u8::is_ascii_whitespace) {
            bail!("empty JSONL row {}", rows.len() + 1);
        }
        rows.push(
            serde_json::from_slice(&line)
                .with_context(|| format!("invalid JSONL row {}", rows.len() + 1))?,
        );
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (DatasetOrigin, RightsPolicy, IngestionLimits) {
        (
            DatasetOrigin {
                source_id: "example/fixture".into(),
                resolved_revision: "a".repeat(40),
                config: "default".into(),
                split: "test".into(),
                files: vec![SourceFile {
                    path: "data/test.jsonl".into(),
                    sha256: sha256(b"{}\n"),
                    bytes: 3,
                }],
            },
            RightsPolicy {
                decision: AccessDecision::Allowed,
                license: Some("Apache-2.0".into()),
                license_text_sha256: Some("b".repeat(64)),
                component_terms: BTreeMap::new(),
                gated: false,
                terms_review: None,
                redistribution_allowed: false,
                privacy_handling: "synthetic fixture; metadata-only evidence".into(),
                unresolved: vec![],
            },
            IngestionLimits {
                max_file_bytes: 16,
                max_total_bytes: 16,
                max_rows: 2,
            },
        )
    }
    #[test]
    fn offline_cache_and_corruption() {
        let (origin, rights, limits) = fixture();
        let dir = tempfile::tempdir().unwrap();
        assert!(materialize(&origin, &rights, dir.path(), true, limits).is_err());
        let path = dir.path().join(&origin.files[0].sha256);
        std::fs::write(&path, b"{}\n").unwrap();
        assert_eq!(
            materialize(&origin, &rights, dir.path(), true, limits).unwrap(),
            vec![path.clone()]
        );
        std::fs::write(&path, b"[]\n").unwrap();
        assert!(materialize(&origin, &rights, dir.path(), true, limits).is_err());
        std::fs::write(&path, b"x").unwrap();
        assert!(materialize(&origin, &rights, dir.path(), true, limits).is_err());
    }
    #[test]
    fn pins_paths_rights_and_budgets_fail_closed() {
        let (mut origin, mut rights, mut limits) = fixture();
        origin.resolved_revision = "main".into();
        assert!(origin.validate().is_err());
        origin.resolved_revision = "a".repeat(40);
        for path in ["../secret", "/absolute", "data//x", "x?token=foo", "x\\y"] {
            origin.files[0].path = path.into();
            assert!(origin.validate().is_err());
        }
        origin.files[0].path = "data/test.jsonl".into();
        rights.decision = AccessDecision::ReviewRequired;
        assert!(rights.authorize().is_err());
        rights.decision = AccessDecision::Allowed;
        rights.gated = true;
        assert!(rights.authorize().is_err());
        rights.terms_review = Some("review-v1".into());
        assert!(rights.authorize().is_ok());
        rights.license = None;
        assert!(rights.authorize().is_err());
        rights.license = Some("Apache-2.0".into());
        let dir = tempfile::tempdir().unwrap();
        limits.max_file_bytes = 2;
        assert!(materialize(&origin, &rights, dir.path(), true, limits).is_err());
        limits.max_file_bytes = 16;
        limits.max_total_bytes = 2;
        assert!(materialize(&origin, &rights, dir.path(), true, limits).is_err());
    }
    #[test]
    fn url_components_and_digest_are_validated_before_cache_or_network() {
        let (mut origin, rights, limits) = fixture();
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("must-not-be-created");
        for path in [
            "",
            "/absolute",
            "back\\slash",
            "..",
            ".",
            "a/../b",
            "a/./b",
            "a?query",
            "a#fragment",
            "a//b",
            "a%2fb",
        ] {
            origin.files[0].path = path.into();
            assert!(origin.validate().is_err(), "accepted path {path:?}");
            assert!(hub_file_url(&origin, &origin.files[0]).is_err());
            // Offline by construction, even if a future validation regression occurs.
            assert!(materialize(&origin, &rights, &cache, true, limits).is_err());
            assert!(!cache.exists());
        }
        origin.files[0].path = "data/test.jsonl".into();
        for digest in [
            "".into(),
            "a".repeat(63),
            "a".repeat(65),
            "g".repeat(64),
            "../outside".into(),
        ] {
            origin.files[0].sha256 = digest;
            assert!(materialize(&origin, &rights, &cache, true, limits).is_err());
            assert!(!cache.exists());
        }
        origin.files[0].sha256 = sha256(b"{}\n");
        let url = hub_file_url(&origin, &origin.files[0]).unwrap();
        assert_eq!(
            url.as_str(),
            format!(
                "https://huggingface.co/datasets/example/fixture/resolve/{}/data/test.jsonl",
                "a".repeat(40)
            )
        );
        assert!(url.query().is_none() && url.fragment().is_none());
    }

    #[test]
    fn streaming_size_checksum_and_truncation() {
        let (origin, _, _) = fixture();
        let file = &origin.files[0];
        for input in [b"{}\n".as_slice(), b"{}", b"{}\nextra", b"[]\n"] {
            assert_eq!(
                copy_verified(input, &mut Vec::new(), file).is_ok(),
                input == b"{}\n"
            );
        }
    }
    #[test]
    fn jsonl_order_unicode_and_limits() {
        let (_, _, mut limits) = fixture();
        let data = "{\"x\":\"é\"}\n{}\n";
        limits.max_file_bytes = 64;
        limits.max_total_bytes = 64;
        let rows = read_jsonl(data.as_bytes(), limits).unwrap();
        assert_eq!(rows[0]["x"], "é");
        assert_eq!(rows.len(), 2);
        limits.max_rows = 1;
        assert!(read_jsonl(data.as_bytes(), limits).is_err());
        limits.max_rows = 2;
        limits.max_file_bytes = 3;
        assert!(read_jsonl(data.as_bytes(), limits).is_err());
        assert!(read_jsonl(b"{\n".as_slice(), limits).is_err());
        assert!(read_jsonl(b"\n".as_slice(), limits).is_err());
    }
    #[test]
    fn fingerprint_invalidation_and_summary() {
        let (origin, rights, _) = fixture();
        let mut m = ValidationManifest {
            schema_version: SCHEMA_VERSION.into(),
            origin,
            rights,
            adapter_id: "fixture".into(),
            adapter_version: "v1".into(),
            field_mapping: BTreeMap::from([("text".into(), "prompt".into())]),
            transformations: vec![],
            tokenizer_revision: None,
            chat_template_sha256: None,
            generator_version: None,
            corpus_rows: Evidence::Unknown,
            selected_source_rows: 1,
            target_count: 2,
            selected_ids_sha256: "a".repeat(64),
            body_order_sha256: "b".repeat(64),
            checks: BTreeMap::new(),
            exclusions: vec![],
            limitations: vec!["convenience sample".into()],
        };
        let initial = m.fingerprint().unwrap();
        assert_eq!(initial, m.fingerprint().unwrap());
        let envelope = m.envelope().unwrap();
        assert_eq!(envelope.validation_manifest.sha256, initial);
        let value = serde_json::to_value(envelope).unwrap();
        let keys: std::collections::BTreeSet<_> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            std::collections::BTreeSet::from([
                "source_id",
                "resolved_revision",
                "config",
                "split",
                "files",
                "adapter_id",
                "adapter_version",
                "license_spdx",
                "validation_manifest"
            ])
        );
        m.tokenizer_revision = Some("c".repeat(40));
        assert_ne!(initial, m.fingerprint().unwrap());
        let previous = m.fingerprint().unwrap();
        m.chat_template_sha256 = Some("d".repeat(64));
        assert_ne!(previous, m.fingerprint().unwrap());
        let previous = m.fingerprint().unwrap();
        m.adapter_version = "v2".into();
        assert_ne!(previous, m.fingerprint().unwrap());
        let previous = m.fingerprint().unwrap();
        m.origin.files[0].sha256 = "e".repeat(64);
        assert_ne!(previous, m.fingerprint().unwrap());
        assert!(m.summary().contains("not observed cache hits"));
        m.schema_version = "kv-dataset-manifest.v999".into();
        assert!(m.fingerprint().is_err());
    }
    #[test]
    fn canonical_linkage_tools_and_partial_blocks() {
        let (origin, _, _) = fixture();
        let mut record = CanonicalRecord {
            record_id: "target-1".into(),
            session_id: None,
            trace_id: None,
            source_row_id: "row-1".into(),
            source_file_sha256: origin.files[0].sha256.clone(),
            group_id: None,
            turn_index: None,
            output_budget: Some(8),
            body: CanonicalBody::Prompt {
                text: "hello".into(),
            },
        };
        assert!(record.validate(&origin).is_ok());
        record.source_file_sha256 = "f".repeat(64);
        assert!(record.validate(&origin).is_err());
        record.source_file_sha256 = origin.files[0].sha256.clone();
        record.body = CanonicalBody::Prompt { text: " ".into() };
        assert!(record.validate(&origin).is_err());
        record.body = CanonicalBody::DocumentQuestion {
            document_id: "doc".into(),
            document: "context".into(),
            question: "question".into(),
        };
        assert!(record.validate(&origin).is_ok());
        record.body = CanonicalBody::StructuralTrace {
            timestamp: 1,
            time_unit: TimeUnit::Milliseconds,
            input_tokens: 513,
            output_tokens: 8,
            block_size: Some(512),
            prefix_hashes: Some(vec!["a".into(), "b".into()]),
            post_template: true,
        };
        assert!(record.validate(&origin).is_ok());
        if let CanonicalBody::StructuralTrace { prefix_hashes, .. } = &mut record.body {
            *prefix_hashes = Some(vec!["a".into()]);
        }
        assert!(record.validate(&origin).is_err());
        if let CanonicalBody::StructuralTrace { prefix_hashes, .. } = &mut record.body {
            *prefix_hashes = None;
        }
        assert!(record.validate(&origin).is_ok());
        let call = Message {
            role: Role::Assistant,
            content: String::new(),
            tool_call_ids: vec!["call-1".into()],
            tool_result_id: None,
        };
        let result = Message {
            role: Role::Tool,
            content: "result".into(),
            tool_call_ids: vec![],
            tool_result_id: Some("call-1".into()),
        };
        record.body = CanonicalBody::Session {
            session_id: "s1".into(),
            history_mode: HistoryMode::Recorded,
            messages: vec![call.clone(), result.clone()],
        };
        assert!(record.validate(&origin).is_ok());
        if let CanonicalBody::Session { messages, .. } = &mut record.body {
            *messages = vec![result.clone(), call.clone()];
        }
        assert!(record.validate(&origin).is_err());
        if let CanonicalBody::Session { messages, .. } = &mut record.body {
            *messages = vec![call];
        }
        assert!(record.validate(&origin).is_err());
    }
    #[test]
    fn typed_roles_reject_unknown() {
        assert!(serde_json::from_str::<Role>("\"arbitrary\"").is_err());
        assert_eq!(
            serde_json::from_str::<Role>("\"tool\"").unwrap(),
            Role::Tool
        );
    }
}
