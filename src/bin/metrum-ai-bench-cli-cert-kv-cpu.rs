// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Offline certification adapter for library-only trace and study-export paths.
//! No serving, fetching, source execution or physical-cache evidence.
use anyhow::{ensure, Context, Result};
use clap::Parser;
use metrum_ai_bench::{
    cached_tokens::CachedTokenEvidence,
    kv_study_export::{PhaseRecords, StudyExport, StudySeed},
    public_dataset::{DatasetOrigin, IngestionLimits, RightsPolicy},
    strategic::BenchRecord,
    trace_adapters::{normalize_jsonl, TraceLimits, TraceMapping},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::Read, path::PathBuf};

#[derive(Parser)]
#[command(
    version,
    about = "Offline trace/export evidence adapter; no physical cache claims"
)]
struct Args {
    #[arg(long)]
    job: PathBuf,
    /// New directory; existing evidence is never overwritten.
    #[arg(long)]
    out: PathBuf,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Job {
    Hf {
        origin: DatasetOrigin,
        rights: RightsPolicy,
        cache: PathBuf,
    },
    Trace {
        origin: DatasetOrigin,
        rights: RightsPolicy,
        mapping: Box<TraceMapping>,
        source: PathBuf,
    },
    Export {
        seed: Value,
        phases: BTreeMap<String, Vec<MockRow>>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MockRow {
    success: bool,
    warmup: bool,
    cached_tokens: Option<u64>,
    ttft_s: f64,
}

fn bounded(path: &std::path::Path, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "offline probe input byte limit");
    Ok(bytes)
}

fn run(args: Args) -> Result<()> {
    let job: Job = serde_json::from_slice(&bounded(&args.job, 4 * 1024 * 1024)?)?;
    // Validate all inputs before reserving an output directory.
    let outputs: Vec<(&str, Vec<u8>)> = match job {
        Job::Hf {
            origin,
            rights,
            cache,
        } => {
            let limits = IngestionLimits {
                max_file_bytes: 16 * 1024 * 1024,
                max_total_bytes: 16 * 1024 * 1024,
                max_rows: 10000,
            };
            ensure!(
                origin.files.len() == 1,
                "exactly one pinned HF file required"
            );
            let paths = metrum_ai_bench::public_dataset::materialize(
                &origin, &rights, &cache, true, limits,
            )?;
            let rows = metrum_ai_bench::public_dataset::read_jsonl(
                std::fs::File::open(&paths[0])?,
                limits,
            )?;
            let mapped = rows
                .into_iter()
                .enumerate()
                .map(|(ordinal, row)| {
                    let id = metrum_ai_bench::public_dataset::sha256(&serde_json::to_vec(&(
                        &origin.source_id,
                        &origin.resolved_revision,
                        &origin.files[0].sha256,
                        ordinal,
                    ))?);
                    Ok(json!({"record_id":id,"source_ordinal":ordinal,"row":row}))
                })
                .collect::<Result<Vec<_>>>()?;
            vec![(
                "hf.json",
                serde_json::to_vec_pretty(&json!({"origin":origin,"rows":mapped}))?,
            )]
        }
        Job::Trace {
            origin,
            rights,
            mapping,
            source,
        } => {
            ensure!(
                origin.files.len() == 1,
                "exactly one pinned trace file required"
            );
            let trace = normalize_jsonl(
                &bounded(&source, 1024 * 1024)?,
                &origin,
                &origin.files[0],
                &rights,
                &mapping,
                TraceLimits {
                    max_source_bytes: 1024 * 1024,
                    max_rows: 64,
                    max_row_bytes: 16384,
                    max_chain_nodes: 4096,
                    max_expanded_nodes: 16384,
                },
            )?;
            vec![("trace.json", serde_json::to_vec_pretty(&trace)?)]
        }
        Job::Export { seed, phases } => {
            ensure!(!phases.is_empty() && phases.len() <= 16, "mock phase limit");
            let rows = phases.into_iter().map(|(id, rows)| {
                ensure!(rows.len() <= 64, "mock row limit");
                let records = rows.into_iter().map(|row| {
                    ensure!(row.ttft_s.is_finite() && row.ttft_s >= 0.0, "invalid mock TTFT");
                    ensure!(row.cached_tokens.is_none_or(|n| n <= 16), "invalid cached tokens");
                    Ok(BenchRecord { success: row.success, warmup: row.warmup,
                        ttft_s: Some(row.ttft_s), ttft_visible_answer_s: Some(row.ttft_s),
                        cache_evidence: row.cached_tokens.map(|n| CachedTokenEvidence::from_usage(
                            &json!({"prompt_tokens":16,"prompt_tokens_details":{"cached_tokens":n}}))),
                        ..Default::default() })
                }).collect::<Result<Vec<_>>>()?;
                Ok((id, records))
            }).collect::<Result<BTreeMap<_, _>>>()?;
            let bindings = rows
                .iter()
                .map(|(id, rows)| (id.clone(), PhaseRecords::Strategic(rows)))
                .collect();
            let export =
                StudyExport::bind(StudySeed::parse(&serde_json::to_vec(&seed)?)?, &bindings)?;
            let data = export.json()?;
            let report_schema: Value =
                serde_json::from_str(include_str!("../../schemas/kv-study-report.v1.json"))?;
            let report_uri = report_schema["$id"]
                .as_str()
                .context("report schema ID")?
                .to_owned();
            let registry = jsonschema::Registry::new()
                .add(
                    &report_uri,
                    jsonschema::Resource::from_contents(report_schema),
                )?
                .prepare()?;
            let schema: Value =
                serde_json::from_str(include_str!("../../schemas/kv-study-export.v1.json"))?;
            ensure!(
                jsonschema::options()
                    .with_registry(&registry)
                    .build(&schema)?
                    .is_valid(&serde_json::from_slice::<Value>(&data)?),
                "export schema mismatch"
            );
            vec![
                ("study.json", data),
                ("study.csv", export.csv()?),
                ("study.html", export.html()?.into_bytes()),
            ]
        }
    };
    std::fs::create_dir(&args.out).context("output directory must be new")?;
    for (name, bytes) in outputs {
        std::fs::write(args.out.join(name), bytes)?;
    }
    Ok(())
}

fn main() -> Result<()> {
    run(Args::parse())
}
