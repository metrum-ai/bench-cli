// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Source pins are not authorization: callers supply reviewed rights policy.
use crate::public_dataset::{DatasetOrigin, SourceFile};
use crate::trace_adapters::{
    HashCoverage, HashEncoding, SourceDomain, TraceMapping, TraceSelection,
};
use crate::trace_inputs::{FractionalMillis, TraceTimeScale, TraceTimeUnit};

pub const QWEN_TRACE_REVISION: &str = "5f7439c51ec248a0c585f7d90a41a6f57773b912";
pub const QWEN_LICENSE_SHA256: &str =
    "58d1e17ffe5109a7ae296caafcadfdbe6a7d176f0bc4ab01e12a689b0499d8bd";

/// Pinned anonymous traceA metadata. Preserve publisher chat_id as row identity
/// and physical source_line separately. Parent/type/turn fields are retained,
/// NOT guessed dependencies.
/// Hashes describe post-template input, so do not frame them a second time.
/// Partial tail symbols are not reusable full blocks. No original text recovered.
pub fn qwen_trace_a(
    rounding: FractionalMillis,
    selection: Option<TraceSelection>,
) -> (DatasetOrigin, TraceMapping) {
    let origin = DatasetOrigin {
        source_id: "alibaba-edu/qwen-bailian-usagetraces-anon".into(),
        resolved_revision: QWEN_TRACE_REVISION.into(),
        config: "traceA".into(),
        split: "source".into(),
        files: vec![SourceFile {
            path: "qwen_traceA_blksz_16.jsonl".into(),
            sha256: "07cedc9ed8aff301994ac68ed4aede8123b7603673575eeba9dd677de663db17".into(),
            bytes: 56_354_493,
        }],
    };
    let mapping = TraceMapping {
        adapter_id: "qwen-anonymous-post-template".into(),
        adapter_version: "1".into(),
        row_id_field: Some("chat_id".into()),
        source_domain: SourceDomain::GitHubPublic,
        selection,
        parent_root_sentinel: Some(serde_json::json!(-1)),
        metadata_fields: vec![
            "chat_id".into(),
            "parent_chat_id".into(),
            "type".into(),
            "turn".into(),
        ],
        input_tokens_field: "input_length".into(),
        output_tokens_field: Some("output_length".into()),
        timestamp_field: Some("timestamp".into()),
        time_unit: TraceTimeUnit::Seconds,
        time_scale: TraceTimeScale::default(),
        rounding,
        hashes_field: Some("hash_ids".into()),
        block_tokens: 16,
        hash_coverage: HashCoverage::IncludingPartialTail,
        hash_encoding: HashEncoding::CompleteChain,
        prefix_parent_field: None,
        session_parent_field: None,
    };
    (origin, mapping)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_pins_keep_selection_units_and_parent_semantics_explicit() {
        let (origin, mapping) = qwen_trace_a(
            FractionalMillis::Reject,
            Some(TraceSelection {
                start_line: 0,
                max_selected_rows: 256,
            }),
        );
        origin.validate().unwrap();
        assert_eq!(origin.files[0].bytes, 56_354_493);
        assert_eq!(mapping.row_id_field.as_deref(), Some("chat_id"));
        assert_eq!(mapping.block_tokens, 16);
        assert!(mapping.session_parent_field.is_none() && mapping.prefix_parent_field.is_none());
        assert!(mapping.metadata_fields.contains(&"parent_chat_id".into()));
        assert_eq!(mapping.selection.unwrap().max_selected_rows, 256);
    }
}
