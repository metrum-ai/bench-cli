// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Declared constructed literals, never recovery of original text from hashes.
use crate::public_dataset::{sha256, DatasetOrigin, RightsPolicy, SourceFile};
use crate::trace_adapters::{normalize_jsonl, SourceTrace, TraceLimits, TraceMapping};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::Path,
};

pub struct TraceSource<'a> {
    pub bytes: &'a [u8],
    pub origin: &'a DatasetOrigin,
    pub file: &'a SourceFile,
    pub rights: &'a RightsPolicy,
    pub mapping: &'a TraceMapping,
    pub limits: TraceLimits,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextConstructionConfig {
    pub tokenizer_revision: String,
    pub tokenizer_sha256: String,
    /// Operator-reviewed ASCII word literals, not ingested scripts or token IDs.
    pub alphabet: Vec<String>,
    pub seed: u64,
    pub max_tokenizer_bytes: usize,
    pub max_rows: usize,
    pub max_output_tokens: u64,
    pub max_output_bytes: usize,
    pub max_candidate_bytes: usize,
    pub max_tokenization_calls: u64,
    pub max_tokenized_bytes: u64,
    pub max_attempts_per_segment: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstructedRequest {
    pub source_row_id: String,
    pub source_line: usize,
    pub text: String,
    pub text_sha256: String,
    /// Raw constructed prompt tokens, NOT final chat-template/rendered tokens.
    pub raw_prompt_token_ids: Vec<u32>,
    pub known_source_full_nodes: Vec<u64>,
    pub constructed_tail_tokens: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstructedTrace {
    pub schema_version: String,
    pub source_trace: SourceTrace,
    pub config: TextConstructionConfig,
    pub requests: Vec<ConstructedRequest>,
    pub tokenization_calls: u64,
    pub tokenized_bytes: u64,
    pub constructed_raw_prefixes_validated: bool,
    pub source_original_text_recovered: bool,
    pub rendered_token_overlap: String,
    pub physical_cache_evidence: String,
    pub construction_sha256: String,
}
struct Segment {
    parent: Option<u64>,
    suffix: String,
    tokens: Vec<u32>,
}
struct Builder<'a> {
    tokenizer: tokenizers::Tokenizer,
    config: &'a TextConstructionConfig,
    domain: String,
    calls: u64,
    bytes: u64,
}
impl Builder<'_> {
    fn encode(&mut self, text: &str) -> Result<Vec<u32>> {
        ensure!(
            text.len() <= self.config.max_candidate_bytes,
            "constructed candidate byte limit"
        );
        self.calls = self
            .calls
            .checked_add(1)
            .context("tokenization count overflow")?;
        self.bytes = self
            .bytes
            .checked_add(text.len() as u64)
            .context("tokenized byte overflow")?;
        ensure!(
            self.calls <= self.config.max_tokenization_calls
                && self.bytes <= self.config.max_tokenized_bytes,
            "construction tokenizer work budget"
        );
        self.tokenizer
            .encode(text, false)
            .map(|e| e.get_ids().to_vec())
            .map_err(|e| anyhow::anyhow!("constructed text tokenization: {e}"))
    }
    fn segment(
        &mut self,
        prefix: &str,
        expected: &[u32],
        count: usize,
        label: &str,
        occupied: &BTreeSet<Vec<u32>>,
    ) -> Result<(String, Vec<u32>)> {
        ensure!(count > 0, "positive constructed segment size required");
        for attempt in 0..self.config.max_attempts_per_segment {
            let mut suffix = String::new();
            for index in 0..count {
                let digest = sha256(&serde_json::to_vec(&(
                    &self.domain,
                    self.config.seed,
                    label,
                    attempt,
                    index,
                ))?);
                let pick =
                    u64::from_str_radix(&digest[..16], 16)? % self.config.alphabet.len() as u64;
                let fragment = &self.config.alphabet[pick as usize];
                let bytes = prefix
                    .len()
                    .checked_add(suffix.len())
                    .and_then(|n| n.checked_add(fragment.len()))
                    .and_then(|n| {
                        n.checked_add(
                            usize::from(!prefix.is_empty()) + usize::from(!suffix.is_empty()),
                        )
                    })
                    .context("construction byte overflow")?;
                ensure!(
                    bytes <= self.config.max_candidate_bytes,
                    "constructed candidate byte limit"
                );
                if !suffix.is_empty() {
                    suffix.push(' ');
                }
                suffix.push_str(fragment);
            }
            let candidate = append(prefix, &suffix);
            let tokens = self.encode(&candidate)?;
            if tokens.len() == expected.len() + count && tokens.starts_with(expected) {
                let segment = tokens[expected.len()..].to_vec();
                if !occupied.contains(&segment) {
                    return Ok((suffix, segment));
                }
            }
        }
        anyhow::bail!("cannot construct exact noncolliding token segment within attempt limit")
    }
}
fn append(prefix: &str, suffix: &str) -> String {
    if prefix.is_empty() {
        suffix.to_owned()
    } else {
        format!("{prefix} {suffix}")
    }
}

/// Verify original source bytes/rights before normalizing; no public constructor
/// accepts a deserialized trace as implicitly trusted source evidence. Complete
/// known full-prefix coverage is required. Tails are newly seeded literals, NOT
/// original hash contents. Final runtime rendering/issuance is a separate stage.
pub fn construct_jsonl(
    source: TraceSource<'_>,
    tokenizer_path: &Path,
    config: &TextConstructionConfig,
) -> Result<ConstructedTrace> {
    let trace = normalize_jsonl(
        source.bytes,
        source.origin,
        source.file,
        source.rights,
        source.mapping,
        source.limits,
    )?;
    ensure!(
        !config.tokenizer_revision.is_empty() && config.tokenizer_sha256.len() == 64,
        "pinned tokenizer identity required"
    );
    ensure!(
        config.max_attempts_per_segment > 0
            && config.max_attempts_per_segment <= 64
            && !config.alphabet.is_empty()
            && config.alphabet.len() <= 64,
        "construction alphabet/attempt limit"
    );
    ensure!(
        config.alphabet.iter().all(|s| !s.is_empty()
            && s.len() <= 128
            && s.bytes().all(|b| b.is_ascii_alphanumeric())),
        "alphabet requires bounded ASCII word literals"
    );
    ensure!(
        trace.requests.len() <= config.max_rows,
        "constructed row limit"
    );
    let total_tokens = trace.requests.iter().try_fold(0u64, |sum, r| {
        sum.checked_add(r.source_input_tokens)
            .context("constructed token sum overflow")
    })?;
    ensure!(
        total_tokens <= config.max_output_tokens && usize::try_from(total_tokens).is_ok(),
        "constructed output token limit"
    );
    let mut bytes = Vec::new();
    std::fs::File::open(tokenizer_path)?
        .take(
            u64::try_from(config.max_tokenizer_bytes)?
                .checked_add(1)
                .context("tokenizer byte bound overflow")?,
        )
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= config.max_tokenizer_bytes && sha256(&bytes) == config.tokenizer_sha256,
        "tokenizer byte limit/checksum mismatch"
    );
    let tokenizer = tokenizers::Tokenizer::from_bytes(bytes)
        .map_err(|e| anyhow::anyhow!("pinned tokenizer: {e}"))?;
    ensure!(
        tokenizer.get_truncation().is_none() && tokenizer.get_padding().is_none(),
        "construction forbids tokenizer truncation/padding"
    );
    let mut builder = Builder {
        tokenizer,
        config,
        domain: trace.normalization_sha256.clone(),
        calls: 0,
        bytes: 0,
    };
    let mut nodes: BTreeMap<u64, Segment> = BTreeMap::new();
    let mut siblings: BTreeMap<Option<u64>, BTreeSet<Vec<u32>>> = BTreeMap::new();
    let mut requests = Vec::new();
    let mut output_bytes = 0usize;
    let block = trace.mapping.block_tokens;
    for row in &trace.requests {
        let chain = row
            .known_full_prefix_nodes
            .as_ref()
            .context("hash-to-text requires known source full-prefix evidence")?;
        ensure!(
            chain.len() as u64 == row.source_input_tokens / block,
            "partial/uncertain full-prefix coverage cannot be constructed as known"
        );
        let mut text = String::new();
        let mut expected = Vec::new();
        let mut parent = None;
        for id in chain {
            if !nodes.contains_key(id) {
                let occupied = siblings.entry(parent).or_default();
                let (suffix, tokens) = builder.segment(
                    &text,
                    &expected,
                    usize::try_from(block)?,
                    &format!("node:{id}"),
                    occupied,
                )?;
                occupied.insert(tokens.clone());
                nodes.insert(
                    *id,
                    Segment {
                        parent,
                        suffix,
                        tokens,
                    },
                );
            }
            let segment = &nodes[id];
            ensure!(
                segment.parent == parent,
                "constructed source node has conflicting parent"
            );
            text = append(&text, &segment.suffix);
            expected.extend_from_slice(&segment.tokens);
            parent = Some(*id);
        }
        let tail = row.source_input_tokens % block;
        if tail > 0 {
            let (suffix, tokens) = builder.segment(
                &text,
                &expected,
                usize::try_from(tail)?,
                &format!("tail:{}", row.source_row_id),
                &BTreeSet::new(),
            )?;
            text = append(&text, &suffix);
            expected.extend(tokens);
        }
        ensure!(
            !text.is_empty(),
            "empty source input is not a constructed workload"
        );
        let final_tokens = builder.encode(&text)?;
        ensure!(
            final_tokens == expected && final_tokens.len() as u64 == row.source_input_tokens,
            "final constructed prefix/length mismatch"
        );
        output_bytes = output_bytes
            .checked_add(text.len())
            .context("constructed output byte overflow")?;
        ensure!(
            output_bytes <= config.max_output_bytes,
            "constructed aggregate output byte limit"
        );
        requests.push(ConstructedRequest {
            source_row_id: row.source_row_id.clone(),
            source_line: row.source_line,
            text_sha256: sha256(text.as_bytes()),
            text,
            raw_prompt_token_ids: final_tokens,
            known_source_full_nodes: chain.clone(),
            constructed_tail_tokens: tail,
        });
    }
    let digest = sha256(&serde_json::to_vec(&(
        "constructed-source-trace.v1",
        &trace.normalization_sha256,
        config,
        &requests,
    ))?);
    Ok(ConstructedTrace {
        schema_version: "constructed-source-trace.v1".into(),
        source_trace: trace,
        config: config.clone(),
        requests,
        tokenization_calls: builder.calls,
        tokenized_bytes: builder.bytes,
        constructed_raw_prefixes_validated: true,
        source_original_text_recovered: false,
        rendered_token_overlap: "unknown".into(),
        physical_cache_evidence: "unavailable".into(),
        construction_sha256: digest,
    })
}

/// Metadata only: private literals/identifiers/source hashes are not printed.
/// Constructor validation is declared evidence, not a deserialization check.
pub fn metadata(trace: &ConstructedTrace) -> serde_json::Value {
    serde_json::json!({
        "$comment":"Copyright (c) 2026 Metrum AI, Inc. All rights reserved.",
        "schema_version":"constructed-source-trace-summary.v1",
        "construction_sha256":trace.construction_sha256,
        "source_normalization_sha256":trace.source_trace.normalization_sha256,
        "rows":trace.requests.len(),
        "tokenization_calls":trace.tokenization_calls,
        "tokenized_bytes":trace.tokenized_bytes,
        "declared_constructed_raw_prefixes_validated":trace.constructed_raw_prefixes_validated,
        "validation_scope":"constructor_only_not_revalidated_on_deserialization",
        "rendered_token_overlap":"unknown",
        "physical_cache_evidence":"unavailable",
        "original_text_recovery_claimed":false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::public_dataset::AccessDecision;
    use crate::trace_adapters::{HashCoverage, HashEncoding, SourceDomain};
    use crate::trace_inputs::{FractionalMillis, TraceTimeScale, TraceTimeUnit};
    struct Fixture {
        bytes: Vec<u8>,
        origin: DatasetOrigin,
        file: SourceFile,
        rights: RightsPolicy,
        mapping: TraceMapping,
        config: TextConstructionConfig,
        dir: tempfile::TempDir,
    }
    fn fixture() -> Fixture {
        let bytes = br#"{"id":"private-row-a","input":5,"hashes":[1,2,99]}
{"id":"private-row-b","input":5,"hashes":[1,3,100]}
{"id":"private-row-c","input":5,"hashes":[1,2,101]}"#
            .to_vec();
        let file = SourceFile {
            path: "synthetic.jsonl".into(),
            sha256: sha256(&bytes),
            bytes: bytes.len() as u64,
        };
        let origin = DatasetOrigin {
            source_id: "example/synthetic-trace".into(),
            resolved_revision: "a".repeat(40),
            config: "test".into(),
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
            privacy_handling: "synthetic test only".into(),
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
            output_tokens_field: None,
            timestamp_field: None,
            time_unit: TraceTimeUnit::Seconds,
            time_scale: TraceTimeScale::default(),
            rounding: FractionalMillis::Reject,
            hashes_field: Some("hashes".into()),
            block_tokens: 2,
            hash_coverage: HashCoverage::IncludingPartialTail,
            hash_encoding: HashEncoding::CompleteChain,
            prefix_parent_field: None,
            session_parent_field: None,
        };
        let tokenizer=br#"{"version":"1.0","truncation":null,"padding":null,"added_tokens":[],"normalizer":null,"pre_tokenizer":{"type":"Whitespace"},"post_processor":null,"decoder":null,"model":{"type":"WordLevel","vocab":{"[UNK]":0,"red":1,"blue":2,"green":3,"white":4},"unk_token":"[UNK]"}}"#;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tokenizer.json"), tokenizer).unwrap();
        let config = TextConstructionConfig {
            tokenizer_revision: "synthetic-wordlevel-v1".into(),
            tokenizer_sha256: sha256(tokenizer),
            alphabet: vec!["red".into(), "blue".into(), "green".into(), "white".into()],
            seed: 7,
            max_tokenizer_bytes: 4096,
            max_rows: 3,
            max_output_tokens: 15,
            max_output_bytes: 1000,
            max_candidate_bytes: 1000,
            max_tokenization_calls: 100,
            max_tokenized_bytes: 100000,
            max_attempts_per_segment: 64,
        };
        Fixture {
            bytes,
            origin,
            file,
            rights,
            mapping,
            config,
            dir,
        }
    }
    fn run(f: &Fixture, config: &TextConstructionConfig) -> Result<ConstructedTrace> {
        construct_jsonl(
            TraceSource {
                bytes: &f.bytes,
                origin: &f.origin,
                file: &f.file,
                rights: &f.rights,
                mapping: &f.mapping,
                limits: TraceLimits {
                    max_source_bytes: 10000,
                    max_rows: 3,
                    max_row_bytes: 1000,
                    max_chain_nodes: 100,
                    max_expanded_nodes: 100,
                },
            },
            &f.dir.path().join("tokenizer.json"),
            config,
        )
    }
    #[test]
    fn full_nodes_reused_siblings_distinct_tail_constructed_and_identity_private() {
        let f = fixture();
        let result = run(&f, &f.config).unwrap();
        let rows = &result.requests;
        assert_eq!(rows.len(), 3);
        for row in rows {
            assert_eq!(row.raw_prompt_token_ids.len(), 5);
            assert_eq!(row.constructed_tail_tokens, 1);
            assert!(!row.text.contains("private-row"));
        }
        assert_eq!(
            &rows[0].raw_prompt_token_ids[..2],
            &rows[1].raw_prompt_token_ids[..2]
        );
        assert_ne!(
            &rows[0].raw_prompt_token_ids[2..4],
            &rows[1].raw_prompt_token_ids[2..4]
        );
        assert_eq!(
            &rows[0].raw_prompt_token_ids[..4],
            &rows[2].raw_prompt_token_ids[..4]
        );
        assert!(!result.source_original_text_recovered);
        assert_eq!(result.rendered_token_overlap, "unknown");
        assert_eq!(result.physical_cache_evidence, "unavailable");
        assert_eq!(
            result.construction_sha256,
            run(&f, &f.config).unwrap().construction_sha256
        );
        assert_ne!(
            result.construction_sha256,
            run(
                &f,
                &TextConstructionConfig {
                    seed: 8,
                    ..f.config.clone()
                }
            )
            .unwrap()
            .construction_sha256
        );
    }
    #[test]
    fn resource_pin_and_impossible_noncolliding_alphabet_fail_closed() {
        let f = fixture();
        assert!(run(
            &f,
            &TextConstructionConfig {
                max_output_tokens: 14,
                ..f.config.clone()
            }
        )
        .is_err());
        assert!(run(
            &f,
            &TextConstructionConfig {
                max_tokenization_calls: 1,
                ..f.config.clone()
            }
        )
        .is_err());
        assert!(run(
            &f,
            &TextConstructionConfig {
                max_tokenized_bytes: 1,
                ..f.config.clone()
            }
        )
        .is_err());
        assert!(run(
            &f,
            &TextConstructionConfig {
                max_candidate_bytes: 1,
                ..f.config.clone()
            }
        )
        .is_err());
        assert!(run(
            &f,
            &TextConstructionConfig {
                max_output_bytes: 1,
                ..f.config.clone()
            }
        )
        .is_err());
        assert!(run(
            &f,
            &TextConstructionConfig {
                tokenizer_sha256: "c".repeat(64),
                ..f.config.clone()
            }
        )
        .is_err());
        assert!(run(
            &f,
            &TextConstructionConfig {
                alphabet: vec!["red".into()],
                ..f.config.clone()
            }
        )
        .is_err());
    }
    #[test]
    fn explicit_candidates_preserve_lineage_without_runtime_history_or_targets() {
        let f = fixture();
        let candidates = crate::trace_candidates::construct_candidates_jsonl(
            TraceSource {
                bytes: &f.bytes,
                origin: &f.origin,
                file: &f.file,
                rights: &f.rights,
                mapping: &f.mapping,
                limits: TraceLimits {
                    max_source_bytes: 10000,
                    max_rows: 3,
                    max_row_bytes: 1000,
                    max_chain_nodes: 100,
                    max_expanded_nodes: 100,
                },
            },
            &f.dir.path().join("tokenizer.json"),
            &f.config,
        )
        .unwrap();
        assert_eq!(candidates.records.len(), 3);
        assert_eq!(candidates.manifest.adapter_id, "constructed-source-trace");
        assert_eq!(
            candidates.manifest.corpus_rows,
            crate::public_dataset::Evidence::Unknown
        );
        assert_eq!(
            candidates.manifest.checks["rendered_token_overlap"],
            crate::public_dataset::Evidence::Unknown
        );
        for (record, constructed) in candidates
            .records
            .iter()
            .zip(&candidates.construction.requests)
        {
            record.validate().unwrap();
            assert_eq!(record.source_row_id, constructed.source_row_id);
            assert_eq!(
                record.source.validation_manifest.as_ref().unwrap().sha256,
                candidates.manifest.fingerprint().unwrap()
            );
            assert!(record.arrival_offset_ms.is_none() && record.expected_output_tokens.is_none());
            assert!(
                record.session_id.is_none()
                    && record.turn_index.is_none()
                    && record.group_id.is_none()
            );
            assert!(
                record.record_id.starts_with("constructed-")
                    && record
                        .trace_id
                        .as_ref()
                        .unwrap()
                        .starts_with("constructed-")
            );
            assert_eq!(
                record.body,
                crate::canonical_input::CanonicalBody::Prompt(constructed.text.clone())
            );
        }
        let encoded = serde_json::to_string(&candidates.records[0]).unwrap();
        let restored: crate::canonical_input::CanonicalRecord =
            serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored.source_row_id, candidates.records[0].source_row_id);
        assert_eq!(
            restored.source.adapter_id.as_deref(),
            Some("constructed-source-trace")
        );
    }

    #[test]
    fn artifact_schema_roundtrip_and_false_recovery_evidence_rejected() {
        let f = fixture();
        let trace = run(&f, &f.config).unwrap();
        let source_schema: serde_json::Value =
            serde_json::from_str(include_str!("../schemas/structural-source-trace.v1.json"))
                .unwrap();
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../schemas/constructed-source-trace.v1.json"))
                .unwrap();
        let registry = jsonschema::Registry::new()
            .add(
                "https://github.com/metrum-ai/bench-cli/schemas/structural-source-trace.v1.json",
                source_schema,
            )
            .unwrap()
            .prepare()
            .unwrap();
        let validator = jsonschema::options()
            .with_registry(&registry)
            .build(&schema)
            .unwrap();
        let value = serde_json::to_value(&trace).unwrap();
        assert!(validator.is_valid(&value));
        let restored: ConstructedTrace = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(restored.requests[0].text, trace.requests[0].text);
        let mut false_claim = value.clone();
        false_claim["source_original_text_recovered"] = serde_json::json!(true);
        assert!(!validator.is_valid(&false_claim));
        let mut false_claim = value;
        false_claim["rendered_token_overlap"] = serde_json::json!("pass");
        assert!(!validator.is_valid(&false_claim));
    }

    #[test]
    fn tokenizer_padding_truncation_and_private_summary_are_explicit() {
        let f = fixture();
        let summary = metadata(&run(&f, &f.config).unwrap()).to_string();
        assert!(!summary.contains("private-row") && !summary.contains("blue"));
        let path = f.dir.path().join("tokenizer.json");
        let original: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        for (key, value) in [
            (
                "padding",
                serde_json::json!({"strategy":"BatchLongest","direction":"Right","pad_to_multiple_of":null,"pad_id":0,"pad_type_id":0,"pad_token":"[UNK]"}),
            ),
            (
                "truncation",
                serde_json::json!({"direction":"Right","max_length":2,"strategy":"LongestFirst","stride":0}),
            ),
        ] {
            let mut changed = original.clone();
            changed[key] = value;
            let bytes = serde_json::to_vec(&changed).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            let error = run(
                &f,
                &TextConstructionConfig {
                    tokenizer_sha256: sha256(&bytes),
                    ..f.config.clone()
                },
            )
            .unwrap_err();
            assert!(error
                .to_string()
                .contains("forbids tokenizer truncation/padding"));
        }
    }

    #[test]
    fn unknown_or_partial_evidence_and_unreviewed_source_never_become_known() {
        let mut f = fixture();
        f.mapping.hashes_field = None;
        assert!(run(&f, &f.config).is_err());
        f.mapping.hashes_field = Some("hashes".into());
        f.mapping.hash_coverage = HashCoverage::PartialPrefix;
        // Remove known full coverage while retaining a real pinned-source checksum.
        f.bytes = br#"{"id":"private","input":8,"hashes":[1]}"#.to_vec();
        f.file.bytes = f.bytes.len() as u64;
        f.file.sha256 = sha256(&f.bytes);
        f.origin.files = vec![f.file.clone()];
        assert!(run(&f, &f.config).is_err());
        f.rights.decision = AccessDecision::ReviewRequired;
        assert!(run(&f, &f.config).is_err());
    }
}
