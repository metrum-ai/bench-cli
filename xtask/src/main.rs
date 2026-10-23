// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Internal CI and release gate tasks. Run via `cargo xtask <cmd>`.

mod assert_headline;
mod bench_bin;
mod campaign_manifest;
mod dco;
mod gitleaks;
mod headers;
mod local_smoke;
mod presets;
mod release_notes;
mod render_cli;
mod render_data_points;
mod serve;
mod shadeform;
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
        Cmd::GitleaksFixtures => gitleaks::run(),
        Cmd::RenderCliHelp { check } => render_cli::run(check),
        Cmd::RenderDataPoints { check } => render_data_points::run(check),
        Cmd::ReleaseNotesDataPoints => release_notes::run(),
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
