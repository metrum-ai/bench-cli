// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use metrum_ai_bench::public_dataset::{AccessDecision, IngestionLimits, RightsPolicy};
use metrum_ai_bench::trace_adapters::{metadata, normalize_jsonl, TraceLimits, TraceSelection};
use metrum_ai_bench::trace_inputs::FractionalMillis;
use metrum_ai_bench::trace_presets::{qwen_trace_a, QWEN_LICENSE_SHA256};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    anyhow::ensure!(
        args.next().as_deref() == Some("--accept-source-terms"),
        "explicit source policy acceptance required"
    );
    let cache = std::path::PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("cache path required"))?,
    );
    let offline = match args.next().as_deref() {
        None => false,
        Some("--offline") => true,
        _ => anyhow::bail!("unsupported source probe argument"),
    };
    anyhow::ensure!(args.next().is_none(), "unexpected source probe argument");
    let (origin, mapping) = qwen_trace_a(
        FractionalMillis::Floor,
        Some(TraceSelection {
            start_line: 0,
            max_selected_rows: 256,
        }),
    );
    let rights = RightsPolicy { decision:AccessDecision::Allowed, license:Some("Apache-2.0".into()), license_text_sha256:Some(QWEN_LICENSE_SHA256.into()), component_terms:Default::default(), gated:false, terms_review:None, redistribution_allowed:false, privacy_handling:"Pinned publisher declares anonymous structural hashes/relative time and dataset Apache-2.0; reviewed README/LICENSE/FAQ. No original text recovery or raw payload publication. Publisher privacy assertions are not independent clearance.".into(), unresolved:vec![] };
    let paths = metrum_ai_bench::github_source::materialize(
        &origin,
        &rights,
        &cache,
        offline,
        IngestionLimits {
            max_file_bytes: 56_354_493,
            max_total_bytes: 56_354_493,
            max_rows: 256,
        },
    )?;
    let bytes = std::fs::read(&paths[0])?;
    let trace = normalize_jsonl(
        &bytes,
        &origin,
        &origin.files[0],
        &rights,
        &mapping,
        TraceLimits {
            max_source_bytes: 56_354_493,
            max_rows: 256,
            max_row_bytes: 2_000_000,
            max_chain_nodes: 65_536,
            max_expanded_nodes: 1_000_000,
        },
    )?;
    println!("{}", serde_json::to_string_pretty(&metadata(&trace))?);
    Ok(())
}
