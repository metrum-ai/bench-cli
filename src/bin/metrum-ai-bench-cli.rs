// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Unified entry point. Modality implementations remain independently
//! executable for one compatibility release; this dispatcher keeps their
//! complete clap surfaces while presenting one stable top-level command.

use clap::{Parser, Subcommand, ValueEnum};
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
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
    /// Select an ISL/OSL mix from metrum-ai/prompt-library.
    Prompts {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Print client environment JSON and `selftest: ok` (exit 0 on success).
    Selftest,
    /// Probe an OpenAI-compatible serving endpoint before a long run.
    Preflight {
        /// Endpoint URL (base or full `/v1/chat/completions` path).
        #[arg(long)]
        url: String,
        /// API key sent as a Bearer token (use `dummy` when the server ignores it).
        #[arg(long)]
        api_key: String,
        /// Model id for chat/streaming probes.
        #[arg(long, default_value = "dummy")]
        model: String,
        /// Connect timeout in seconds.
        #[arg(long, default_value_t = 10)]
        connect_timeout: u64,
        /// Request timeout in seconds.
        #[arg(long, default_value_t = 60)]
        request_timeout: u64,
        /// Number of unary latency samples.
        #[arg(long, default_value_t = 3)]
        latency_samples: u32,
        /// Also print the machine-readable JSON report after the table.
        #[arg(long, default_value_t = false)]
        json: bool,
    },
    /// Write or probe a system-under-test declaration.
    Sut {
        #[command(subcommand)]
        command: SutCommands,
    },
    /// Compare two or more strategic sweep summaries or request CSVs.
    Compare {
        /// Strategic stdout JSON files (`points[]`) and/or request CSVs.
        #[arg(required = true, num_args = 2..)]
        inputs: Vec<PathBuf>,
        /// Comma-separated labels matching input order (default: file stems).
        #[arg(long, value_delimiter = ',')]
        labels: Option<Vec<String>>,
        /// Output format.
        #[arg(long, value_enum, default_value_t = CompareFormat::Markdown)]
        format: CompareFormat,
        /// Optional output path (default: stdout).
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
enum SutCommands {
    /// Write a SUT JSON template (optionally `--probe` local host facts).
    Init {
        /// Output path.
        #[arg(long, default_value = "sut.json")]
        output: PathBuf,
        /// Probe local nvidia-smi / OS / CPU / memory (never remote SSH).
        #[arg(long, default_value_t = false)]
        probe: bool,
        /// Overwrite an existing file.
        #[arg(long, default_value_t = false)]
        force: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CompareFormat {
    Markdown,
    Json,
}

fn sibling_binary(name: &str) -> std::io::Result<std::path::PathBuf> {
    let current = std::env::current_exe()?;
    Ok(current.with_file_name(format!("{name}{}", std::env::consts::EXE_SUFFIX)))
}

fn environment() -> serde_json::Value {
    metrum_ai_bench::environment::collect(None, None, false)
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
        "schema_version": "metrum-ai-bench-cli.cross-run.v1",
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

fn run_preflight(args: Commands) -> ExitCode {
    let Commands::Preflight {
        url,
        api_key,
        model,
        connect_timeout,
        request_timeout,
        latency_samples,
        json,
    } = args
    else {
        unreachable!()
    };
    match metrum_ai_bench::preflight::run_preflight_blocking(
        &url,
        &api_key,
        &model,
        connect_timeout,
        request_timeout,
        latency_samples,
    ) {
        Ok(report) => {
            print!("{}", metrum_ai_bench::preflight::format_table(&report));
            if json {
                match serde_json::to_string_pretty(&report) {
                    Ok(body) => println!("{body}"),
                    Err(error) => {
                        eprintln!("preflight JSON encode failed: {error}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            if metrum_ai_bench::preflight::exit_failure(&report) {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(error) => {
            eprintln!("preflight failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_sut(command: SutCommands) -> ExitCode {
    match command {
        SutCommands::Init {
            output,
            probe,
            force,
        } => match metrum_ai_bench::sut::init_sut(&output, probe, force) {
            Ok(result) => {
                for warning in &result.warnings {
                    eprintln!("{warning}");
                }
                println!(
                    "sut init: wrote {} (provenance={}, probed={})",
                    result.path.display(),
                    result.sut.provenance,
                    result.probed
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
    }
}

fn run_compare(
    inputs: Vec<PathBuf>,
    labels: Option<Vec<String>>,
    format: CompareFormat,
    output: Option<PathBuf>,
) -> ExitCode {
    match (|| -> anyhow::Result<()> {
        let labels = metrum_ai_bench::compare::resolve_labels(&inputs, labels)?;
        let mut runs = Vec::new();
        for (path, label) in inputs.iter().zip(labels.iter()) {
            runs.push(metrum_ai_bench::compare::load_run(path, label)?);
        }
        let report = metrum_ai_bench::compare::compare_runs(&runs)?;
        let body = match format {
            CompareFormat::Markdown => metrum_ai_bench::compare::format_markdown(&report),
            CompareFormat::Json => serde_json::to_string_pretty(&report)?,
        };
        match output {
            Some(path) => std::fs::write(path, body)?,
            None => print!("{body}"),
        }
        Ok(())
    })() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("compare failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Commands::Selftest => match serde_json::to_string_pretty(&environment()) {
            Ok(value) => {
                println!("{value}");
                println!("selftest: ok");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("self-test failed: {error}");
                ExitCode::FAILURE
            }
        },
        cmd @ Commands::Preflight { .. } => run_preflight(cmd),
        Commands::Sut { command } => run_sut(command),
        Commands::Compare {
            inputs,
            labels,
            format,
            output,
        } => run_compare(inputs, labels, format, output),
        Commands::Llm { args } => match dispatch("metrum-ai-bench-cli-llm", args) {
            Ok(code) => code,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Commands::Vlm { args } => match dispatch("metrum-ai-bench-cli-vlm", args) {
            Ok(code) => code,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Commands::Asr { args } => match dispatch("metrum-ai-bench-cli-asr", args) {
            Ok(code) => code,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Commands::Imagegen { args } => match dispatch("metrum-ai-bench-cli-imagegen", args) {
            Ok(code) => code,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
        Commands::Prompts { args } => match dispatch("metrum-ai-bench-cli-prompts", args) {
            Ok(code) => code,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        },
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
