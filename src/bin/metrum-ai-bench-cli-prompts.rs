// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Select a reproducible ISL/OSL mix from metrum-ai/prompt-library and write
//! JSONL for metrum-ai-bench-cli-llm.

use clap::Parser;
use metrum_ai_bench::prompt_library::{
    default_cache_dir, format_select_failure, load_hub_dataset, load_jsonl, load_parquet_files,
    recommended_max_tokens, resolve_revision, select_mix, selection_report_with_profile,
    write_jsonl, DatasetRef, IslTokenBasis, LengthStat, LengthUnit, ReasoningFilter, SelectRequest,
    WorkloadProfile, DEFAULT_DATASET,
};
use std::path::PathBuf;
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser, Debug)]
#[command(
    name = "metrum-ai-bench-cli-prompts",
    author,
    version,
    about = "Select ISL/OSL mixes from metrum-ai/prompt-library for metrum-ai-bench-cli-llm"
)]
struct Args {
    #[arg(long, help = "Print version information and exit")]
    version_only: bool,

    #[arg(
        long,
        default_value_t = false,
        help = "Suppress ASCII banner art (one-line identity still prints). Also set NO_BANNER=1."
    )]
    quiet: bool,

    #[arg(long, default_value = DEFAULT_DATASET)]
    dataset: String,

    #[arg(
        long,
        default_value = "main",
        help = "Dataset revision (default: main = latest). Pass a 40-char commit SHA to pin. Branch/tag names resolve to the current commit."
    )]
    revision: String,

    #[arg(long, default_value = "sample", help = "Dataset config: sample|full")]
    config: String,

    #[arg(long, default_value = "train")]
    split: String,

    #[arg(
        long,
        help = "Deprecated no-op: floating refs (including default main) always resolve. Kept for CLI compatibility."
    )]
    allow_moving_revision: bool,

    #[arg(
        long,
        help = "Fail unless --revision is a 40-character commit SHA (publication pin)"
    )]
    require_pinned_revision: bool,

    #[arg(long, help = "Cache directory for Hub downloads")]
    cache_dir: Option<PathBuf>,

    #[arg(long, help = "Do not download; use files already in the cache")]
    offline: bool,

    #[arg(
        long,
        help = "Load rows from local parquet shards (repeatable); skips Hub"
    )]
    local_parquet: Vec<PathBuf>,

    #[arg(
        long,
        help = "Load rows from a local JSONL file with full metadata; skips Hub"
    )]
    local_jsonl: Option<PathBuf>,

    #[arg(
        long,
        value_parser = clap::value_parser!(u32).range(1..),
        required_unless_present = "version_only",
        help = "Preferred mix size (soft target; actual size may differ within --count-slack)"
    )]
    count: Option<u32>,

    #[arg(
        long,
        help = "Max absolute deviation from --count (default: max(count, 32))"
    )]
    count_slack: Option<u32>,

    #[arg(long, default_value_t = 0, help = "RNG seed for selection")]
    seed: u64,

    #[arg(
        long,
        value_enum,
        help = "Named versioned ISL/OSL profile (chat-short, chat-medium, rag-medium, summarize-long, code-medium); conflicts with --isl-target/--osl-target"
    )]
    profile: Option<WorkloadProfile>,

    #[arg(
        long,
        required_unless_present_any = ["version_only", "profile"],
        help = "ISL target (same units as --isl-unit); omitted when --profile is set"
    )]
    isl_target: Option<f64>,

    #[arg(long, value_enum, default_value_t = LengthUnit::Tokens)]
    isl_unit: LengthUnit,

    #[arg(long, value_enum, default_value_t = LengthStat::Median)]
    isl_stat: LengthStat,

    #[arg(long, default_value_t = 0.0, help = "Absolute ISL tolerance")]
    isl_tolerance: f64,

    #[arg(
        long,
        required_unless_present_any = ["version_only", "profile"],
        help = "OSL target (same units as --osl-unit); omitted when --profile is set"
    )]
    osl_target: Option<f64>,

    #[arg(long, value_enum, default_value_t = LengthUnit::Tokens)]
    osl_unit: LengthUnit,

    #[arg(long, value_enum, default_value_t = LengthStat::Median)]
    osl_stat: LengthStat,

    #[arg(long, default_value_t = 0.0, help = "Absolute OSL tolerance")]
    osl_tolerance: f64,

    #[arg(long, value_enum, default_value_t = IslTokenBasis::SuppliedTarget)]
    isl_token_basis: IslTokenBasis,

    #[arg(long, value_enum, default_value_t = ReasoningFilter::Any)]
    reasoning: ReasoningFilter,

    #[arg(
        long,
        default_value_t = 8,
        value_parser = clap::value_parser!(u32).range(1..),
        help = "Max copies of one source row"
    )]
    max_repeats: u32,

    #[arg(long, help = "Disable repeats (equivalent to --max-repeats 1)")]
    no_repeats: bool,

    #[arg(
        long,
        help = "Tokens-per-word factor for recommending --max-tokens when --osl-unit words"
    )]
    osl_tokens_per_word: Option<f64>,

    #[arg(
        long,
        default_value_t = 50_000,
        help = "Selector work / iteration budget"
    )]
    select_work_limit: u32,

    #[arg(
        long,
        required_unless_present = "version_only",
        help = "Write selected prompts as JSONL for metrum-ai-bench-cli-llm"
    )]
    output: Option<PathBuf>,

    #[arg(
        long,
        required_unless_present = "version_only",
        help = "Write selection report JSON"
    )]
    report: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    if args.version_only {
        println!("metrum-ai-bench-cli-prompts version {VERSION}");
        return ExitCode::SUCCESS;
    }

    metrum_ai_bench::banner::print_banner(VERSION, "metrum-ai-bench-cli-prompts", args.quiet);

    if let Err(error) = run(args) {
        eprintln!("error: {error:#}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run(args: Args) -> anyhow::Result<()> {
    let count = args
        .count
        .ok_or_else(|| anyhow::anyhow!("--count is required"))? as usize;
    let profile_spec = args.profile.map(|profile| profile.spec());
    let (isl_target, osl_target, isl_tolerance, osl_tolerance) = if let Some(spec) = profile_spec {
        if args.isl_target.is_some() || args.osl_target.is_some() {
            anyhow::bail!(
                "--profile cannot be combined with --isl-target/--osl-target (use one or the other)"
            );
        }
        let isl_tol = if args.isl_tolerance > 0.0 {
            args.isl_tolerance
        } else {
            spec.isl_tolerance
        };
        let osl_tol = if args.osl_tolerance > 0.0 {
            args.osl_tolerance
        } else {
            spec.osl_tolerance
        };
        (spec.isl_target, spec.osl_target, isl_tol, osl_tol)
    } else {
        let isl_target = args
            .isl_target
            .ok_or_else(|| anyhow::anyhow!("--isl-target is required (or pass --profile)"))?;
        let osl_target = args
            .osl_target
            .ok_or_else(|| anyhow::anyhow!("--osl-target is required (or pass --profile)"))?;
        (
            isl_target,
            osl_target,
            args.isl_tolerance,
            args.osl_tolerance,
        )
    };
    let output = args
        .output
        .ok_or_else(|| anyhow::anyhow!("--output is required"))?;
    let report_path = args
        .report
        .ok_or_else(|| anyhow::anyhow!("--report is required"))?;

    if isl_target <= 0.0 || osl_target <= 0.0 {
        anyhow::bail!("--isl-target and --osl-target must be positive");
    }
    if isl_tolerance < 0.0 || osl_tolerance < 0.0 {
        anyhow::bail!("tolerances must be nonnegative");
    }
    if matches!(args.osl_unit, LengthUnit::Words) && args.osl_tokens_per_word.is_none() {
        anyhow::bail!("--osl-tokens-per-word is required when --osl-unit words");
    }
    if !matches!(args.config.as_str(), "sample" | "full") {
        anyhow::bail!("--config must be sample or full");
    }

    let count_slack = args
        .count_slack
        .map(|v| v as usize)
        .unwrap_or(count.max(32));
    let max_repeats = if args.no_repeats {
        1
    } else {
        args.max_repeats as usize
    };

    let (rows, revision, config_label, split_label, dataset_label) = if let Some(jsonl) =
        args.local_jsonl.as_ref()
    {
        let rows = load_jsonl(jsonl)?;
        (
            rows,
            "local-jsonl".to_string(),
            "local".to_string(),
            "local".to_string(),
            jsonl.display().to_string(),
        )
    } else if !args.local_parquet.is_empty() {
        let mut paths = args.local_parquet.clone();
        paths.sort();
        let rows = load_parquet_files(&paths, 0)?;
        (
            rows,
            "local-parquet".to_string(),
            "local".to_string(),
            "local".to_string(),
            "local-parquet".to_string(),
        )
    } else {
        let revision_arg = args.revision.trim();
        if args.require_pinned_revision {
            let pinned = revision_arg.len() == 40
                && revision_arg.bytes().all(|b| b.is_ascii_hexdigit());
            if !pinned {
                anyhow::bail!(
                    "--require-pinned-revision needs a 40-character commit SHA; got `{revision_arg}`"
                );
            }
        }
        // Latest by default: floating refs (main/tags/branches) always resolve to a SHA.
        let _ = args.allow_moving_revision; // deprecated; floating resolve is unconditional
        let sha = resolve_revision(&args.dataset, revision_arg, true)?;
        let cache = args.cache_dir.unwrap_or_else(default_cache_dir);
        let spec = DatasetRef {
            repo: args.dataset.clone(),
            revision: sha.clone(),
            config: args.config.clone(),
            split: args.split.clone(),
        };
        let (rows, sha) = load_hub_dataset(&spec, &cache, args.offline)?;
        (
            rows,
            sha,
            args.config.clone(),
            args.split.clone(),
            args.dataset.clone(),
        )
    };

    if rows.is_empty() {
        anyhow::bail!("dataset contained zero rows");
    }

    let req = SelectRequest {
        count,
        count_slack,
        seed: args.seed,
        isl_target,
        isl_unit: args.isl_unit,
        isl_stat: args.isl_stat,
        isl_tolerance,
        osl_target,
        osl_unit: args.osl_unit,
        osl_stat: args.osl_stat,
        osl_tolerance,
        isl_token_basis: args.isl_token_basis,
        reasoning: args.reasoning,
        max_repeats,
        work_limit: args.select_work_limit,
        osl_tokens_per_word: args.osl_tokens_per_word,
    };

    let mix = match select_mix(&rows, &req) {
        Ok(mix) => mix,
        Err(err) => {
            anyhow::bail!("{}", format_select_failure(&err, &req));
        }
    };

    let max_tokens = recommended_max_tokens(&mix, &req)?;
    let report = selection_report_with_profile(
        &dataset_label,
        &revision,
        &config_label,
        &split_label,
        &req,
        &mix,
        max_tokens,
        profile_spec,
    )?;

    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    if let Some(parent) = report_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    write_jsonl(&output, &mix)?;
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report)?)?;

    println!(
        "selected_count={} preferred_count={} isl={:.4} (gap {:+.4}) osl={:.4} (gap {:+.4}) recommended_max_tokens={} extra_copies={}",
        mix.rows.len(),
        req.count,
        mix.isl_achieved,
        mix.isl_gap,
        mix.osl_achieved,
        mix.osl_gap,
        max_tokens,
        mix.extra_copies
    );
    println!("wrote {}", output.display());
    println!("wrote {}", report_path.display());
    Ok(())
}
