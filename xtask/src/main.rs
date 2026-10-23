// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Internal CI and release gate tasks. Run via `cargo xtask <cmd>`.

mod analysis_contract;
mod assert_headline;
#[path = "../../src/count_points.rs"]
mod count_points;
mod dco;
mod fixtures;
mod gitleaks;
mod headers;
mod local_smoke;
mod presets;
mod release_notes;
mod render_cli;
mod render_data_points;
mod serve;
mod shadeform;
mod telemetry_analysis;
mod util;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "xtask", about = "bench-cli CI and release gate tasks")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Fail when authored sources lack Copyright / SPDX, or naming rules fail.
    CheckHeaders,
    /// Fail when any commit in base..head lacks a Signed-off-by trailer.
    CheckDco { base: String, head: String },
    /// Count data points in one or more Bench data logs.
    CountPoints {
        files: Vec<PathBuf>,
        #[arg(long = "aiperf-dir", action = clap::ArgAction::Append)]
        aiperf_dirs: Vec<PathBuf>,
        #[arg(long)]
        scenario: Option<String>,
        #[arg(long)]
        append: Option<PathBuf>,
        #[arg(long, value_name = "COUNTS_JSONL")]
        table: Option<PathBuf>,
        #[arg(long, default_value_t = 0.9)]
        max_ttft_ratio: f64,
    },
    /// Assert custom gitleaks rules fire on fixture files (shells out to gitleaks).
    GitleaksFixtures,
    /// Regenerate or check docs/CLI.md from clap --help.
    RenderCliHelp {
        #[arg(long)]
        check: bool,
    },
    /// Regenerate or check docs/DATA_POINTS.md via the data_points integration test.
    RenderDataPoints {
        #[arg(long)]
        check: bool,
    },
    /// Print docs/DATA_POINTS.md shaped for GitHub release notes.
    ReleaseNotesDataPoints,
    /// Analyze a completed telemetry NDJSON file offline.
    TelemetryAnalysis {
        ndjson: PathBuf,
        #[arg(long)]
        summary: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Generate and validate checked-in test fixtures.
    Fixtures {
        #[command(subcommand)]
        command: fixtures::FixturesCommand,
    },
    /// Copy a validated campaign manifest into docs/smoke/<campaign-id>/.
    CampaignManifest(campaign_manifest::ManifestArgs),
    /// Run one local modality smoke using prebuilt binaries and campaign fixtures.
    LocalSmoke(local_smoke::LocalSmokeArgs),
    /// Check the campaign preset catalog's required schema and provenance.
    CheckPresets {
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Resolve a prebuilt benchmark binary; never compiles.
    BenchBin {
        name: String,
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Start, inspect, and stop a serving container using checked-in smoke data.
    Serve(serve::ServeArgs),
    /// Manage Shadeform GPU instance types and lifecycle.
    Shadeform {
        #[command(subcommand)]
        command: shadeform::ShadeformCommand,
    },
    /// Fail a smoke cell whose data log cannot back a headline claim.
    AssertHeadline {
        modality: String,
        data_log: PathBuf,
        #[arg(long)]
        artifact_dir: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::CheckHeaders => headers::run(),
        Cmd::CheckDco { base, head } => dco::run(&base, &head),
        Cmd::CountPoints {
            files,
            aiperf_dirs,
            scenario,
            append,
            table,
            max_ttft_ratio,
        } => {
            if let Some(path) = table {
                print!("{}", count_points::table(&path)?);
                return Ok(());
            }
            let mut counts = Vec::new();
            for file in files {
                let mut count = count_points::count_bench(&file, max_ttft_ratio)?;
                let label = scenario.clone().unwrap_or_else(|| {
                    file.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("bench")
                        .to_owned()
                });
                count["scenario"] = serde_json::json!(label);
                counts.push(count);
            }
            for directory in aiperf_dirs {
                let mut count = count_points::count_aiperf(&directory, max_ttft_ratio)?;
                let label = scenario.clone().unwrap_or_else(|| {
                    directory
                        .parent()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                        .unwrap_or("aiperf")
                        .to_owned()
                });
                count["scenario"] = serde_json::json!(label);
                counts.push(count);
            }
            if let Some(path) = append {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)?;
                for count in &counts {
                    writeln!(file, "{}", count)?;
                }
            }
            for count in counts {
                println!("{}", serde_json::to_string_pretty(&count)?);
            }
            Ok(())
        }
        Cmd::GitleaksFixtures => gitleaks::run(),
        Cmd::RenderCliHelp { check } => render_cli::run(check),
        Cmd::RenderDataPoints { check } => render_data_points::run(check),
        Cmd::ReleaseNotesDataPoints => release_notes::run(),
        Cmd::Fixtures { command } => fixtures::run(command),
        Cmd::TelemetryAnalysis {
            ndjson,
            summary,
            json,
        } => telemetry_analysis::run(&ndjson, summary.as_deref(), json),
        Cmd::CampaignManifest(args) => campaign_manifest::run(args),
        Cmd::LocalSmoke(args) => local_smoke::run(args),
        Cmd::CheckPresets { root } => presets::run(&root),
        Cmd::BenchBin { name, root } => {
            println!("{}", bench_bin::resolve(&root, &name)?.display());
            Ok(())
        }
        Cmd::Serve(args) => serve::run(args),
        Cmd::Shadeform { command } => shadeform::run(command),
        Cmd::AssertHeadline {
            modality,
            data_log,
            artifact_dir,
        } => assert_headline::run(&modality, &data_log, artifact_dir.as_deref()),
    }
}

mod bench_bin;
mod campaign_manifest;
