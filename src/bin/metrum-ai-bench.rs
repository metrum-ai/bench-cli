// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Unified entry point. Modality implementations remain independently
//! executable for one compatibility release; this dispatcher keeps their
//! complete clap surfaces while presenting one stable top-level command.

use clap::{Parser, Subcommand};
use std::ffi::OsString;
use std::io::Write;
use std::process::{Command, ExitCode};

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "Benchmark OpenAI-compatible inference endpoints"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Text chat/completions benchmark.
    Llm {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Vision-language chat benchmark.
    Vlm {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Audio transcription benchmark.
    Asr {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Image generation benchmark.
    Imagegen {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Print client environment and verify local runtime capabilities.
    Selftest,
}

fn sibling_binary(name: &str) -> std::io::Result<std::path::PathBuf> {
    let current = std::env::current_exe()?;
    Ok(current.with_file_name(format!("{name}{}", std::env::consts::EXE_SUFFIX)))
}

fn environment() -> serde_json::Value {
    metrum_ai_bench::environment::collect(None, None)
}

fn extract_runs(args: &mut Vec<OsString>) -> Result<u32, String> {
    let mut runs = 1;
    let mut index = 0;
    while index < args.len() {
        let text = args[index].to_string_lossy();
        if text == "--runs" {
            let value = args
                .get(index + 1)
                .ok_or_else(|| "--runs requires a value".to_string())?
                .to_string_lossy()
                .parse::<u32>()
                .map_err(|_| "--runs must be a positive integer".to_string())?;
            if value == 0 {
                return Err("--runs must be at least 1".into());
            }
            runs = value;
            args.drain(index..=index + 1);
            continue;
        }
        if let Some(value) = text.strip_prefix("--runs=") {
            runs = value
                .parse::<u32>()
                .map_err(|_| "--runs must be a positive integer".to_string())?;
            if runs == 0 {
                return Err("--runs must be at least 1".into());
            }
            args.remove(index);
            continue;
        }
        index += 1;
    }
    Ok(runs)
}

fn argument_value(args: &[OsString], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].to_string_lossy().into_owned())
        .or_else(|| {
            args.iter().find_map(|arg| {
                arg.to_string_lossy()
                    .strip_prefix(&format!("{name}="))
                    .map(str::to_string)
            })
        })
}

fn append_cross_run(path: &str, seed: u64, expected_runs: usize) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(path)?;
    let summaries: Vec<serde_json::Value> = text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .filter(|value: &serde_json::Value| {
            value["schema_version"] == metrum_ai_bench::record::SCHEMA_VERSION_SUMMARY
        })
        .rev()
        .take(expected_runs)
        .collect();
    let request_rates: Vec<f64> = summaries
        .iter()
        .filter_map(|value| value["requests_per_second"].as_f64())
        .collect();
    let token_rates: Vec<f64> = summaries
        .iter()
        .filter_map(|value| value["completion_tokens_per_second"].as_f64())
        .collect();
    let aggregate = serde_json::json!({
        "schema_version": "metrum-ai-bench.cross-run.v1",
        "runs": summaries.len(),
        "requests_per_second": metrum_ai_bench::stats::DistSummary::from_values(&request_rates),
        "requests_per_second_ci95": metrum_ai_bench::stats::bootstrap_mean_ci(&request_rates, 0.95, 10_000, seed),
        "completion_tokens_per_second": metrum_ai_bench::stats::DistSummary::from_values(&token_rates),
        "completion_tokens_per_second_ci95": metrum_ai_bench::stats::bootstrap_mean_ci(&token_rates, 0.95, 10_000, seed.wrapping_add(1)),
    });
    let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
    serde_json::to_writer(&mut file, &aggregate)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn dispatch(binary: &str, mut args: Vec<OsString>) -> Result<ExitCode, String> {
    let runs = extract_runs(&mut args)?;
    let data_log = argument_value(&args, "--data-log");
    let seed = argument_value(&args, "--seed")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    for _ in 0..runs {
        let status = sibling_binary(binary)
            .and_then(|path| Command::new(path).args(&args).status())
            .map_err(|error| format!("failed to launch {binary}: {error}"))?;
        if !status.success() {
            return Ok(ExitCode::from(status.code().unwrap_or(1) as u8));
        }
    }
    if runs > 1 {
        let path = data_log.ok_or_else(|| "--runs requires --data-log".to_string())?;
        append_cross_run(&path, seed, runs as usize)
            .map_err(|error| format!("cross-run aggregation failed: {error}"))?;
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if matches!(cli.command, Commands::Selftest) {
        match serde_json::to_string_pretty(&environment()) {
            Ok(value) => {
                println!("{value}");
                return ExitCode::SUCCESS;
            }
            Err(error) => {
                eprintln!("self-test failed: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    let (binary, args) = match cli.command {
        Commands::Llm { args } => ("metrum-ai-bench-llm", args),
        Commands::Vlm { args } => ("metrum-ai-bench-vlm", args),
        Commands::Asr { args } => ("metrum-ai-bench-asr", args),
        Commands::Imagegen { args } => ("metrum-ai-bench-imagegen", args),
        Commands::Selftest => unreachable!(),
    };
    match dispatch(binary, args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_runs_without_forwarding_it() {
        let mut args = vec![
            OsString::from("--runs"),
            OsString::from("3"),
            OsString::from("--data-log"),
            OsString::from("out.jsonl"),
        ];
        assert_eq!(extract_runs(&mut args).unwrap(), 3);
        assert_eq!(
            argument_value(&args, "--data-log").as_deref(),
            Some("out.jsonl")
        );
        assert!(!args.iter().any(|arg| arg == "--runs"));
    }

    #[test]
    fn rejects_zero_runs() {
        let mut args = vec![OsString::from("--runs=0")];
        assert!(extract_runs(&mut args).is_err());
    }
}
