// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use metrum_ai_bench::public_dataset::{AccessDecision, IngestionLimits, RightsPolicy};
use metrum_ai_bench::trace_adapters::{TraceLimits, TraceSelection};
use metrum_ai_bench::trace_candidates::construct_candidates_jsonl;
use metrum_ai_bench::trace_inputs::FractionalMillis;
use metrum_ai_bench::trace_presets::{qwen_trace_a, QWEN_LICENSE_SHA256};
use metrum_ai_bench::trace_text::{metadata, TextConstructionConfig, TraceSource};
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    #[serde(rename = "$comment")]
    _comment: String,
    config: TextConstructionConfig,
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    anyhow::ensure!(
        args.next().as_deref() == Some("--accept-source-terms"),
        "explicit source policy acceptance required"
    );
    let cache = std::path::PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("cache required"))?,
    );
    let tokenizer = std::path::PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("tokenizer required"))?,
    );
    let config_path = std::path::PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("config required"))?,
    );
    anyhow::ensure!(args.next().is_none(), "unexpected probe argument");
    use std::io::Read;
    let mut config_bytes = Vec::new();
    std::fs::File::open(config_path)?
        .take(65537)
        .read_to_end(&mut config_bytes)?;
    anyhow::ensure!(config_bytes.len() <= 65536, "config byte limit");
    let job: Job = serde_json::from_slice(&config_bytes)?;
    let (origin, mapping) = qwen_trace_a(
        FractionalMillis::Floor,
        Some(TraceSelection {
            start_line: 0,
            max_selected_rows: 4,
        }),
    );
    let rights=RightsPolicy { decision:AccessDecision::Allowed,license:Some("Apache-2.0".into()),license_text_sha256:Some(QWEN_LICENSE_SHA256.into()),component_terms:Default::default(),gated:false,terms_review:None,redistribution_allowed:false,privacy_handling:"Pinned publisher declares anonymous structural hashes/relative time and dataset Apache-2.0; reviewed README/LICENSE/FAQ. No original text recovery or raw payload publication. Publisher privacy assertions are not independent clearance.".into(),unresolved:vec![] };
    // Strict offline mode: another explicit ingestion step must have verified cache.
    let files = metrum_ai_bench::github_source::materialize(
        &origin,
        &rights,
        &cache,
        true,
        IngestionLimits {
            max_file_bytes: 56_354_493,
            max_total_bytes: 56_354_493,
            max_rows: 4,
        },
    )?;
    let bytes = std::fs::read(&files[0])?;
    let result = construct_candidates_jsonl(
        TraceSource {
            bytes: &bytes,
            origin: &origin,
            file: &origin.files[0],
            rights: &rights,
            mapping: &mapping,
            limits: TraceLimits {
                max_source_bytes: 56_354_493,
                max_rows: 4,
                max_row_bytes: 2_000_000,
                max_chain_nodes: 65_536,
                max_expanded_nodes: 1_000_000,
            },
        },
        &tokenizer,
        &job.config,
    )?;
    let mut summary = metadata(&result.construction);
    summary["canonical_candidates"] = serde_json::json!(result.records.len());
    summary["validation_manifest_sha256"] = serde_json::json!(result.manifest.fingerprint()?);
    summary["candidate_issuance"] = serde_json::json!("not_issued");
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}
