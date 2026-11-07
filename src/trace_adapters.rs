// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Verified structural source traces, not original text or runtime scheduling.
use crate::public_dataset::{sha256, DatasetOrigin, RightsPolicy, SourceFile};
use crate::trace_inputs::{
    normalize_trace_time, FractionalMillis, NormalizedTraceTime, TraceTimeScale, TraceTimeUnit,
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{value::RawValue, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HashCoverage {
    FullBlocksOnly,
    IncludingPartialTail,
    PartialPrefix,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HashEncoding {
    CompleteChain,
    ParentDelta { reused_full_blocks_field: String },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceDomain {
    LocalPinned,
    HuggingFaceHub,
    GitHubPublic,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceSelection {
    pub start_line: usize,
    pub max_selected_rows: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceMapping {
    pub adapter_id: String,
    pub adapter_version: String,
    /// None explicitly uses physical line ordinal, not an invented publisher ID.
    pub row_id_field: Option<String>,
    pub source_domain: SourceDomain,
    pub selection: Option<TraceSelection>,
    pub parent_root_sentinel: Option<Value>,
    /// Optional source columns retained privately, not interpreted as dependencies.
    pub metadata_fields: Vec<String>,
    pub input_tokens_field: String,
    pub output_tokens_field: Option<String>,
    pub timestamp_field: Option<String>,
    pub time_unit: TraceTimeUnit,
    pub time_scale: TraceTimeScale,
    pub rounding: FractionalMillis,
    pub hashes_field: Option<String>,
    pub block_tokens: u64,
    pub hash_coverage: HashCoverage,
    pub hash_encoding: HashEncoding,
    pub prefix_parent_field: Option<String>,
    pub session_parent_field: Option<String>,
}
#[derive(Debug, Clone, Copy)]
pub struct TraceLimits {
    pub max_source_bytes: usize,
    pub max_rows: usize,
    pub max_row_bytes: usize,
    pub max_chain_nodes: usize,
    pub max_expanded_nodes: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceRequest {
    pub source_row_id: String,
    pub source_line: usize,
    pub source_attributes: BTreeMap<String, Value>,
    pub source_input_tokens: u64,
    pub source_output_tokens: Option<u64>,
    /// Unit-converted source time, not origin-rebased arrival or achieved timing.
    pub source_time: Option<NormalizedTraceTime>,
    pub prefix_parent: Option<String>,
    pub session_parent: Option<String>,
    pub reused_parent_full_blocks: Option<u64>,
    /// Interned prefix-tree nodes, not unordered raw hash labels or token IDs.
    pub known_full_prefix_nodes: Option<Vec<u64>>,
    pub partial_tail_symbol_sha256: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceTrace {
    pub schema_version: String,
    pub source: DatasetOrigin,
    pub mapping: TraceMapping,
    pub license_spdx: Option<String>,
    pub rights_policy_sha256: String,
    pub source_file_sha256: String,
    pub normalization_sha256: String,
    pub requests: Vec<TraceRequest>,
    /// Physical lines, NOT a validated full-corpus record count.
    pub source_physical_lines: usize,
    pub rendered_token_overlap: String,
    pub physical_cache_evidence: String,
}
struct Row {
    request: TraceRequest,
    hashes: Option<Vec<String>>,
    reused: usize,
}
fn identifier(value: &Value) -> Result<String> {
    if let Some(s) = value.as_str().filter(|s| !s.is_empty()) {
        return Ok(s.into());
    }
    if let Some(n) = value.as_u64() {
        return Ok(n.to_string());
    }
    anyhow::bail!("source identity must be nonempty string or unsigned integer")
}
fn parent(row: &Value, field: &Option<String>, root: &Option<Value>) -> Result<Option<String>> {
    field
        .as_ref()
        .and_then(|key| row.get(key))
        .filter(|v| !v.is_null() && root.as_ref() != Some(*v))
        .map(identifier)
        .transpose()
}
fn check_parents(rows: &[Row], ids: &BTreeMap<String, usize>, session: bool) -> Result<()> {
    for start in 0..rows.len() {
        let mut path = BTreeSet::new();
        let mut current = Some(start);
        while let Some(i) = current {
            ensure!(
                path.len() < 256 && path.insert(i),
                "source parent cycle or depth limit"
            );
            let p = if session {
                &rows[i].request.session_parent
            } else {
                &rows[i].request.prefix_parent
            };
            current = p
                .as_ref()
                .map(|p| ids.get(p).copied().context("missing source parent"))
                .transpose()?;
        }
    }
    Ok(())
}

/// Parse only reviewed, pinned file bytes. Numeric time is read as RawValue,
/// never converted through f64. Preserve physical line order; do not sort/rebase,
/// infer session dependencies, recover text, execute code or issue requests.
pub fn normalize_jsonl(
    bytes: &[u8],
    origin: &DatasetOrigin,
    file: &SourceFile,
    rights: &RightsPolicy,
    mapping: &TraceMapping,
    limits: TraceLimits,
) -> Result<SourceTrace> {
    origin.validate()?;
    rights.authorize()?;
    ensure!(
        origin
            .files
            .iter()
            .any(|f| f.path == file.path && f.bytes == file.bytes && f.sha256 == file.sha256),
        "trace file not in pinned origin"
    );
    ensure!(
        bytes.len() <= limits.max_source_bytes
            && bytes.len() as u64 == file.bytes
            && sha256(bytes) == file.sha256,
        "trace byte limit/length/checksum mismatch"
    );
    ensure!(
        !mapping.adapter_id.is_empty()
            && !mapping.adapter_version.is_empty()
            && mapping.block_tokens > 0,
        "trace adapter identity/block size required"
    );
    if matches!(mapping.hash_encoding, HashEncoding::ParentDelta { .. }) {
        ensure!(
            mapping.hashes_field.is_some() && mapping.prefix_parent_field.is_some(),
            "delta encoding requires hashes/prefix-parent mapping"
        );
    }
    let data = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    ensure!(!data.is_empty(), "empty trace input");
    ensure!(
        mapping.metadata_fields.len() <= 32
            && mapping
                .metadata_fields
                .iter()
                .all(|f| !f.is_empty() && f.len() <= 128),
        "source metadata field limit"
    );
    let source_physical_lines = data.split(|b| *b == b'\n').count();
    if let Some(selection) = mapping.selection {
        ensure!(
            selection.max_selected_rows > 0
                && selection.max_selected_rows <= limits.max_rows
                && selection.start_line < source_physical_lines,
            "invalid declared trace selection"
        );
    }
    let mut rows = Vec::new();
    let mut ids = BTreeMap::new();
    for (line, bytes) in data.split(|b| *b == b'\n').enumerate() {
        if let Some(selection) = mapping.selection {
            if line < selection.start_line
                || line - selection.start_line >= selection.max_selected_rows
            {
                continue;
            }
        }
        ensure!(
            rows.len() < limits.max_rows
                && !bytes.is_empty()
                && bytes.len() <= limits.max_row_bytes,
            "trace row/count limit or blank row"
        );
        let raw: BTreeMap<String, Box<RawValue>> = serde_json::from_slice(bytes)
            .map_err(|_| anyhow::anyhow!("invalid trace JSON object"))?;
        let row: Value =
            serde_json::from_slice(bytes).map_err(|_| anyhow::anyhow!("invalid trace row"))?;
        let id = match &mapping.row_id_field {
            Some(field) => identifier(&row[field])?,
            None => format!("line:{line}"),
        };
        ensure!(
            ids.insert(id.clone(), rows.len()).is_none(),
            "duplicate source row identity"
        );
        let input = row[&mapping.input_tokens_field]
            .as_u64()
            .context("unsigned input token length required")?;
        let output = mapping
            .output_tokens_field
            .as_ref()
            .map(|key| {
                row[key]
                    .as_u64()
                    .context("unsigned output token length required")
            })
            .transpose()?;
        let time = mapping
            .timestamp_field
            .as_ref()
            .map(|key| {
                let raw = raw.get(key).context("missing source time")?.get();
                let lexeme = if raw.starts_with('"') {
                    serde_json::from_str::<String>(raw)?
                } else {
                    raw.to_owned()
                };
                normalize_trace_time(
                    &lexeme,
                    mapping.time_unit,
                    mapping.time_scale,
                    mapping.rounding,
                )
            })
            .transpose()?;
        let hashes = mapping
            .hashes_field
            .as_ref()
            .map(|key| {
                let hashes = row[key]
                    .as_array()
                    .context("declared trace hashes must be an array")?;
                ensure!(
                    hashes.len() <= limits.max_chain_nodes,
                    "source hash-chain limit"
                );
                hashes
                    .iter()
                    .map(|h| {
                        ensure!(
                            h.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 256)
                                || h.as_u64().is_some(),
                            "invalid opaque hash symbol"
                        );
                        // Keep numeric and string labels distinct; IDs are not tokenizer IDs.
                        Ok(serde_json::to_string(h)?)
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?;
        let reused = match &mapping.hash_encoding {
            HashEncoding::CompleteChain => 0,
            HashEncoding::ParentDelta {
                reused_full_blocks_field,
            } => usize::try_from(
                row[reused_full_blocks_field]
                    .as_u64()
                    .context("delta full-block reuse count required")?,
            )
            .context("delta reuse overflow")?,
        };
        rows.push(Row {
            request: TraceRequest {
                source_row_id: id,
                source_line: line,
                source_attributes: mapping
                    .metadata_fields
                    .iter()
                    .filter_map(|key| row.get(key).map(|value| (key.clone(), value.clone())))
                    .collect(),
                source_input_tokens: input,
                source_output_tokens: output,
                source_time: time,
                prefix_parent: parent(
                    &row,
                    &mapping.prefix_parent_field,
                    &mapping.parent_root_sentinel,
                )?,
                session_parent: parent(
                    &row,
                    &mapping.session_parent_field,
                    &mapping.parent_root_sentinel,
                )?,
                reused_parent_full_blocks: match mapping.hash_encoding {
                    HashEncoding::CompleteChain => None,
                    HashEncoding::ParentDelta { .. } => Some(reused as u64),
                },
                known_full_prefix_nodes: None,
                partial_tail_symbol_sha256: None,
            },
            hashes,
            reused,
        });
    }
    check_parents(&rows, &ids, false)?;
    check_parents(&rows, &ids, true)?;
    let mut chains: Vec<Option<Vec<String>>> = vec![None; rows.len()];
    let mut done = vec![false; rows.len()];
    let mut expanded = 0usize;
    while done.iter().any(|d| !d) {
        let mut progress = false;
        for i in 0..rows.len() {
            if done[i] {
                continue;
            }
            let delta = matches!(mapping.hash_encoding, HashEncoding::ParentDelta { .. });
            let parent_index = if delta {
                rows[i].request.prefix_parent.as_ref().map(|p| ids[p])
            } else {
                None
            };
            if parent_index.is_some_and(|p| !done[p]) {
                continue;
            }
            let mut chain = Vec::new();
            if let Some(p) = parent_index {
                let full = rows[p].request.source_input_tokens / mapping.block_tokens;
                let known = chains[p]
                    .as_ref()
                    .context("delta parent has no hash evidence")?;
                ensure!(
                    rows[i].reused as u64 <= full && rows[i].reused <= known.len(),
                    "delta cannot reuse parent partial/unknown tail"
                );
                chain.extend_from_slice(&known[..rows[i].reused]);
            } else {
                ensure!(rows[i].reused == 0, "root cannot reuse parent blocks")
            }
            if let Some(hashes) = &rows[i].hashes {
                ensure!(
                    chain
                        .len()
                        .checked_add(hashes.len())
                        .is_some_and(|n| n <= limits.max_chain_nodes),
                    "expanded chain limit"
                );
                chain.extend_from_slice(hashes);
                let full = rows[i].request.source_input_tokens / mapping.block_tokens;
                let tail = u64::from(
                    !rows[i]
                        .request
                        .source_input_tokens
                        .is_multiple_of(mapping.block_tokens),
                );
                match mapping.hash_coverage {
                    HashCoverage::FullBlocksOnly => {
                        ensure!(chain.len() as u64 == full, "full-block hash count mismatch")
                    }
                    HashCoverage::IncludingPartialTail => ensure!(
                        chain.len() as u64 == full + tail,
                        "full/tail hash count mismatch"
                    ),
                    HashCoverage::PartialPrefix => ensure!(
                        chain.len() as u64 <= full,
                        "partial-prefix count exceeds full blocks"
                    ),
                }
                expanded = expanded
                    .checked_add(chain.len())
                    .context("expanded trace node overflow")?;
                ensure!(
                    expanded <= limits.max_expanded_nodes,
                    "aggregate expanded trace node limit"
                );
                chains[i] = Some(chain);
            }
            done[i] = true;
            progress = true;
        }
        ensure!(progress, "unresolved prefix graph");
    }
    let mut intern = BTreeMap::new();
    for (row, chain) in rows.iter_mut().zip(chains) {
        if let Some(mut chain) = chain {
            let full = row.request.source_input_tokens / mapping.block_tokens;
            if chain.len() as u64 > full {
                row.request.partial_tail_symbol_sha256 =
                    Some(sha256(chain.pop().context("missing tail")?.as_bytes()));
            }
            let mut parent = None;
            let mut nodes = Vec::new();
            for symbol in chain {
                let next = intern.len() as u64;
                let id = *intern.entry((parent, symbol)).or_insert(next);
                nodes.push(id);
                parent = Some(id);
            }
            row.request.known_full_prefix_nodes = Some(nodes);
        }
    }
    let requests: Vec<_> = rows.into_iter().map(|r| r.request).collect();
    let rights_policy_sha256 = sha256(&serde_json::to_vec(rights)?);
    let digest = sha256(&serde_json::to_vec(&(
        origin,
        mapping,
        &rights_policy_sha256,
        &file.sha256,
        &requests,
    ))?);
    Ok(SourceTrace {
        schema_version: "structural-source-trace.v1".into(),
        source: origin.clone(),
        mapping: mapping.clone(),
        license_spdx: rights.license.clone(),
        rights_policy_sha256,
        source_file_sha256: file.sha256.clone(),
        normalization_sha256: digest,
        requests,
        source_physical_lines,
        rendered_token_overlap: "unknown".into(),
        physical_cache_evidence: "unavailable".into(),
    })
}

/// Default evidence omits source IDs, raw times, hash chains and source payload.
/// Source artifacts are private; counts/digests do not establish corpus coverage.
pub fn metadata(trace: &SourceTrace) -> Value {
    serde_json::json!({
        "$comment":"Copyright (c) 2026 Metrum AI, Inc. All rights reserved.",
        "schema_version":"structural-source-trace-summary.v1",
        "source_file_sha256":trace.source_file_sha256,
        "normalization_sha256":trace.normalization_sha256,
        "rows":trace.requests.len(),
        "source_physical_lines":trace.source_physical_lines,
        "selection":trace.mapping.selection,
        "rows_with_declared_hash_evidence":trace.requests.iter().filter(|r| r.known_full_prefix_nodes.is_some()).count(),
        "rows_with_rounded_time":trace.requests.iter().filter(|r| r.source_time.as_ref().is_some_and(|t| t.discarded_fraction)).count(),
        "rendered_token_overlap":"unknown",
        "physical_cache_evidence":"unavailable",
        "corpus_coverage":"unknown"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::public_dataset::AccessDecision;
    fn setup(
        bytes: &[u8],
    ) -> (
        DatasetOrigin,
        SourceFile,
        RightsPolicy,
        TraceMapping,
        TraceLimits,
    ) {
        let file = SourceFile {
            path: "synthetic.jsonl".into(),
            sha256: sha256(bytes),
            bytes: bytes.len() as u64,
        };
        let origin = DatasetOrigin {
            source_id: "example/synthetic-trace".into(),
            resolved_revision: "a".repeat(40),
            config: "default".into(),
            split: "test".into(),
            files: vec![file.clone()],
        };
        let rights = RightsPolicy {
            decision: AccessDecision::Allowed,
            license: Some("Apache-2.0".into()),
            license_text_sha256: Some("b".repeat(64)),
            component_terms: BTreeMap::new(),
            gated: false,
            terms_review: None,
            redistribution_allowed: false,
            privacy_handling: "synthetic fixture only".into(),
            unresolved: vec![],
        };
        let mapping = TraceMapping {
            adapter_id: "synthetic".into(),
            adapter_version: "1".into(),
            row_id_field: Some("id".into()),
            source_domain: SourceDomain::LocalPinned,
            selection: None,
            parent_root_sentinel: None,
            metadata_fields: vec![],
            input_tokens_field: "input".into(),
            output_tokens_field: Some("output".into()),
            timestamp_field: Some("time".into()),
            time_unit: TraceTimeUnit::Seconds,
            time_scale: TraceTimeScale::default(),
            rounding: FractionalMillis::Floor,
            hashes_field: Some("hashes".into()),
            block_tokens: 16,
            hash_coverage: HashCoverage::IncludingPartialTail,
            hash_encoding: HashEncoding::ParentDelta {
                reused_full_blocks_field: "reuse".into(),
            },
            prefix_parent_field: Some("prefix_parent".into()),
            session_parent_field: Some("session_parent".into()),
        };
        let limits = TraceLimits {
            max_source_bytes: 16384,
            max_rows: 10,
            max_row_bytes: 8192,
            max_chain_nodes: 10,
            max_expanded_nodes: 30,
        };
        (origin, file, rights, mapping, limits)
    }
    #[test]
    fn delta_uses_full_parent_blocks_and_preserves_exact_time_and_order() {
        let bytes = br#"{"id":"a","input":23,"output":4294967296,"time":0.0019,"hashes":["A","tail"],"reuse":0}
{"id":"b","input":32,"output":0,"time":0.001,"hashes":["B"],"reuse":1,"prefix_parent":"a"}
{"id":"c","input":16,"output":1,"time":0.002,"hashes":["B"],"reuse":0,"session_parent":"a"}
"#;
        let (origin, file, rights, mapping, limits) = setup(bytes);
        let result = normalize_jsonl(bytes, &origin, &file, &rights, &mapping, limits).unwrap();
        let schema: Value =
            serde_json::from_str(include_str!("../schemas/structural-source-trace.v1.json"))
                .unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let artifact = serde_json::to_value(&result).unwrap();
        assert!(validator.is_valid(&artifact));
        let mut false_evidence = artifact.clone();
        false_evidence["rendered_token_overlap"] = serde_json::json!("pass");
        assert!(!validator.is_valid(&false_evidence));
        let roundtrip: SourceTrace = serde_json::from_value(artifact).unwrap();
        assert_eq!(roundtrip.normalization_sha256, result.normalization_sha256);
        assert_eq!(result.requests[1].reused_parent_full_blocks, Some(1));
        assert_eq!(result.mapping.block_tokens, 16);
        assert_eq!(result.source.source_id, origin.source_id);
        assert_eq!(result.license_spdx.as_deref(), Some("Apache-2.0"));
        assert_eq!(result.requests[0].source_output_tokens, Some(4294967296));
        assert_eq!(
            result.requests[0]
                .source_time
                .as_ref()
                .unwrap()
                .source_value,
            "0.0019"
        );
        assert!(
            result.requests[0]
                .source_time
                .as_ref()
                .unwrap()
                .discarded_fraction
        );
        assert_eq!(result.requests[0].source_line, 0);
        assert_eq!(result.requests[1].prefix_parent.as_deref(), Some("a"));
        assert!(result.requests[1].session_parent.is_none());
        let a = result.requests[0].known_full_prefix_nodes.as_ref().unwrap();
        let b = result.requests[1].known_full_prefix_nodes.as_ref().unwrap();
        let c = result.requests[2].known_full_prefix_nodes.as_ref().unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0], b[0]);
        assert_ne!(b[1], c[0]);
        assert!(result.requests[0].partial_tail_symbol_sha256.is_some());
        assert_eq!(result.rendered_token_overlap, "unknown");
        assert_eq!(result.physical_cache_evidence, "unavailable");
    }
    #[test]
    fn length_time_only_never_proves_overlap_and_byte_rights_limits_hold() {
        let bytes = br#"{"id":"private-row-label","input":100,"output":0,"time":"0.002"}"#;
        let (origin, file, mut rights, mut mapping, limits) = setup(bytes);
        mapping.hash_encoding = HashEncoding::CompleteChain;
        mapping.hashes_field = None;
        let result = normalize_jsonl(bytes, &origin, &file, &rights, &mapping, limits).unwrap();
        assert!(result.requests[0].known_full_prefix_nodes.is_none());
        assert_eq!(result.rendered_token_overlap, "unknown");
        let summary = metadata(&result).to_string();
        assert!(!summary.contains("private-row-label"));
        assert!(!summary.contains("0.002"));
        assert!(!summary.contains("synthetic-trace"));
        let schema: Value =
            serde_json::from_str(include_str!("../schemas/structural-source-trace.v1.json"))
                .unwrap();
        assert!(jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&serde_json::to_value(&result).unwrap()));
        assert!(normalize_jsonl(b"changed", &origin, &file, &rights, &mapping, limits).is_err());
        assert!(normalize_jsonl(
            bytes,
            &origin,
            &file,
            &rights,
            &mapping,
            TraceLimits {
                max_rows: 0,
                ..limits
            }
        )
        .is_err());
        rights.unresolved.push("rights unknown".into());
        assert!(normalize_jsonl(bytes, &origin, &file, &rights, &mapping, limits).is_err());
    }
    #[test]
    fn declared_selection_line_identity_root_sentinel_and_private_attributes() {
        let bytes =
            br#"{"input":16,"output":0,"time":0,"hashes":[1],"parent":-1,"type":"private-kind"}
{"input":16,"output":0,"time":1,"hashes":[1],"parent":-1,"type":"private-kind"}"#;
        let (origin, file, rights, mut mapping, limits) = setup(bytes);
        mapping.row_id_field = None;
        mapping.selection = Some(TraceSelection {
            start_line: 1,
            max_selected_rows: 1,
        });
        mapping.hash_encoding = HashEncoding::CompleteChain;
        mapping.hash_coverage = HashCoverage::FullBlocksOnly;
        mapping.session_parent_field = Some("parent".into());
        mapping.parent_root_sentinel = Some(serde_json::json!(-1));
        mapping.metadata_fields = vec!["type".into()];
        let trace = normalize_jsonl(bytes, &origin, &file, &rights, &mapping, limits).unwrap();
        assert_eq!(trace.requests.len(), 1);
        assert_eq!(trace.source_physical_lines, 2);
        assert_eq!(trace.requests[0].source_row_id, "line:1");
        assert!(trace.requests[0].session_parent.is_none());
        assert_eq!(
            trace.requests[0].source_attributes["type"],
            serde_json::json!("private-kind")
        );
        assert!(!metadata(&trace).to_string().contains("private-kind"));
        mapping.parent_root_sentinel = None;
        assert!(normalize_jsonl(bytes, &origin, &file, &rights, &mapping, limits).is_err());
        mapping.selection = Some(TraceSelection {
            start_line: 10,
            max_selected_rows: 1,
        });
        assert!(normalize_jsonl(bytes, &origin, &file, &rights, &mapping, limits).is_err());
    }

    #[test]
    fn complete_partial_prefix_and_forward_dependencies_remain_explicit() {
        let bytes = br#"{"id":"child","input":16,"output":1,"time":2,"hashes":["same"],"session_parent":"root"}
{"id":"root","input":16,"output":1,"time":1,"hashes":["same"]}"#;
        let (origin, file, rights, mut mapping, limits) = setup(bytes);
        mapping.hash_encoding = HashEncoding::CompleteChain;
        mapping.hash_coverage = HashCoverage::FullBlocksOnly;
        let trace = normalize_jsonl(bytes, &origin, &file, &rights, &mapping, limits).unwrap();
        assert_eq!(trace.requests[0].source_row_id, "child");
        assert_eq!(
            trace.requests[0].source_time.as_ref().unwrap().milliseconds,
            2000
        );
        assert_eq!(trace.requests[0].reused_parent_full_blocks, None);
        assert_eq!(
            trace.requests[0].known_full_prefix_nodes,
            trace.requests[1].known_full_prefix_nodes
        );
        mapping.hash_coverage = HashCoverage::PartialPrefix;
        assert!(normalize_jsonl(bytes, &origin, &file, &rights, &mapping, limits).is_ok());
        assert!(normalize_jsonl(
            bytes,
            &origin,
            &file,
            &rights,
            &mapping,
            TraceLimits {
                max_expanded_nodes: 1,
                ..limits
            }
        )
        .is_err());
        assert!(normalize_jsonl(
            bytes,
            &origin,
            &file,
            &rights,
            &mapping,
            TraceLimits {
                max_chain_nodes: 0,
                ..limits
            }
        )
        .is_err());
        assert!(normalize_jsonl(
            bytes,
            &origin,
            &file,
            &rights,
            &mapping,
            TraceLimits {
                max_row_bytes: 1,
                ..limits
            }
        )
        .is_err());
        let wrong_file = SourceFile {
            path: "unlisted.jsonl".into(),
            ..file.clone()
        };
        assert!(normalize_jsonl(bytes, &origin, &wrong_file, &rights, &mapping, limits).is_err());
    }

    #[test]
    fn cycles_missing_parents_bad_counts_and_partial_tail_reuse_fail() {
        for bytes in [
            r#"{"id":"x","input":16,"output":0,"time":0,"hashes":[1],"reuse":0,"prefix_parent":"x"}"#,
            r#"{"id":"x","input":16,"output":0,"time":0,"hashes":[1],"reuse":0,"session_parent":"missing"}"#,
            r#"{"id":"x","input":32,"output":0,"time":0,"hashes":[1],"reuse":0}"#,
            "{\"id\":\"a\",\"input\":23,\"output\":0,\"time\":0,\"hashes\":[1,2],\"reuse\":0}\n{\"id\":\"b\",\"input\":32,\"output\":0,\"time\":0,\"hashes\":[],\"reuse\":2,\"prefix_parent\":\"a\"}",
        ] {
            let (origin,file,rights,mapping,limits)=setup(bytes.as_bytes());
            assert!(normalize_jsonl(bytes.as_bytes(),&origin,&file,&rights,&mapping,limits).is_err());
        }
    }
}
