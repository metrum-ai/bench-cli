// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! One local benchmark smoke cell using prebuilt binaries and checked-in fixtures.

use anyhow::{bail, Context, Result};
use clap::Args;
use serde_json::{json, Value};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Args)]
pub struct LocalSmokeArgs {
    modality: String,
    #[arg(long, default_value = "http://127.0.0.1:8000")]
    url: String,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    sut: Option<PathBuf>,
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long)]
    num_requests: Option<u32>,
    #[arg(long, default_value_t = 2)]
    concurrency: u32,
    #[arg(long, default_value_t = 1)]
    warmup: u32,
    #[arg(long, default_value_t = 7)]
    seed: u64,
    #[arg(long, default_value = ".")]
    root: PathBuf,
}

fn rewrite_jsonl_paths(source: &Path, target: &Path, field: &str, root: &Path) -> Result<()> {
    let contents =
        fs::read_to_string(source).with_context(|| format!("read {}", source.display()))?;
    let mut output = String::new();
    for line in contents.lines().filter(|line| !line.trim().is_empty()) {
        let mut row: Value = serde_json::from_str(line)
            .with_context(|| format!("parse row in {}", source.display()))?;
        let relative = row[field]
            .as_str()
            .context(format!("missing {field} path in {}", source.display()))?;
        let path = Path::new(relative);
        row[field] = json!(if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        }
        .display()
        .to_string());
        output.push_str(&serde_json::to_string(&row)?);
        output.push('\n');
    }
    fs::write(target, output).with_context(|| format!("write {}", target.display()))?;
    Ok(())
}

fn stamp_sut(path: &Path, key: &str, value: &Value) -> Result<()> {
    let mut sut: Value = serde_json::from_slice(&fs::read(path)?)?;
    if sut["extra"].is_null() {
        sut["extra"] = json!({});
    }
    sut["extra"][key] = value.clone();
    fs::write(path, format!("{}\n", serde_json::to_string_pretty(&sut)?))?;
    Ok(())
}

fn binary_identity(binary: &Path, root: &Path) -> Result<String> {
    let version = Command::new(binary)
        .arg("--version")
        .output()
        .context("run bench --version")?;
    let version = String::from_utf8_lossy(&version.stdout)
        .lines()
        .next()
        .unwrap_or("unknown")
        .to_owned();
    let checkout = Command::new("git")
        .args([
            "-C",
            &root.display().to_string(),
            "describe",
            "--always",
            "--dirty",
            "--tags",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "not a git checkout".into());
    Ok(format!(
        "path={} version={} checkout={checkout}",
        binary.display(),
        version
    ))
}

fn run_child(binary: &Path, args: &[String], cwd: &Path, out: &Path) -> Result<()> {
    let mut command = Command::new(binary);
    command
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let rendered = std::iter::once(binary.display().to_string())
        .chain(args.iter().cloned())
        .map(|arg| {
            if arg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_./:=+-".contains(c))
            {
                arg
            } else {
                format!("'{}'", arg.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    fs::write(out.join("command.txt"), format!("{rendered}\n"))?;
    eprintln!("# local smoke command: {rendered}");
    let result = command
        .output()
        .with_context(|| format!("run {}", binary.display()))?;
    fs::write(out.join("stdout.txt"), &result.stdout)?;
    fs::write(out.join("stderr.txt"), &result.stderr)?;
    print!("{}", String::from_utf8_lossy(&result.stdout));
    eprint!("{}", String::from_utf8_lossy(&result.stderr));
    fs::write(
        out.join("exit_code.txt"),
        format!("{}\n", result.status.code().unwrap_or(1)),
    )?;
    if !result.status.success() {
        bail!("benchmark process failed with {}", result.status);
    }
    Ok(())
}
pub fn run(args: LocalSmokeArgs) -> Result<()> {
    if !["llm", "vlm", "asr", "imagegen"].contains(&args.modality.as_str()) {
        bail!("unsupported modality {}", args.modality);
    }
    let root = fs::canonicalize(&args.root)
        .with_context(|| format!("resolve repository root {}", args.root.display()))?;
    let base_url = args.url.trim_end_matches('/').to_owned();
    let timeout = env::var("READY_TIMEOUT_S")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
    loop {
        if client
            .get(format!("{base_url}/v1/models"))
            .send()
            .is_ok_and(|response| response.status().is_success())
        {
            break;
        }
        if std::time::Instant::now() >= deadline {
            bail!("timed out waiting for {base_url}/v1/models");
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
    let model_default = match args.modality.as_str() {
        "llm" => "Qwen/Qwen3-8B",
        "vlm" => "Qwen/Qwen3-VL-8B-Instruct",
        "asr" => "openai/whisper-large-v3-turbo",
        _ => "Tongyi-MAI/Z-Image-Turbo",
    };
    let model = args.model.as_deref().unwrap_or(model_default);
    let default_count = match args.modality.as_str() {
        "llm" => 16,
        "vlm" => 8,
        "asr" => 6,
        _ => 4,
    };
    let count = args.num_requests.unwrap_or(default_count);
    let out = args.out.unwrap_or_else(|| {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        root.join(format!("live-results/local-{}-{stamp}", args.modality))
    });
    fs::create_dir_all(&out)?;
    let sut_source = args
        .sut
        .unwrap_or_else(|| root.join(format!("live-results/serve-{}/sut.json", args.modality)));
    if !sut_source.is_file() {
        bail!(
            "SUT file {} not found; start cargo xtask serve {} start or pass --sut",
            sut_source.display(),
            args.modality
        );
    }
    let sut = out.join("sut.json");
    fs::copy(&sut_source, &sut)?;
    let data_log = out.join("results.jsonl");
    let debug_log = out.join("debug.log");
    let error_log = out.join("error.log");
    let binary_name = match args.modality.as_str() {
        "llm" => "metrum-ai-bench-cli-llm",
        "vlm" => "metrum-ai-bench-cli-vlm",
        "asr" => "metrum-ai-bench-cli-asr",
        _ => "metrum-ai-bench-cli-imagegen",
    };
    let binary = super::bench_bin::resolve(&root, binary_name)?;
    let identity = binary_identity(&binary, &root)?;
    let mut sut_json: Value = serde_json::from_slice(&fs::read(&sut)?)?;
    let previous = sut_json["notes"].as_str().unwrap_or("");
    sut_json["notes"] = json!(format!("{previous}; bench binary: {identity}"));
    if sut_json["extra"].is_null() {
        sut_json["extra"] = json!({});
    }
    sut_json["extra"]["bench_binary"] = json!(identity);
    fs::write(
        &sut,
        format!("{}\n", serde_json::to_string_pretty(&sut_json)?),
    )?;
    let mut command_args = Vec::<String>::new();
    let mut measured_count = count;
    let artifact_dir = out.join("artifacts");
    match args.modality.as_str() {
        "llm" => {
            let prompt_bin = super::bench_bin::resolve(&root, "metrum-ai-bench-cli-prompts")?;
            let prompts = out.join("prompts.jsonl");
            let report_path = out.join("prompt-mix.json");
            let mut prompt_args = vec![
                "--quiet".into(),
                "--count".into(),
                (count + args.warmup).to_string(),
                "--seed".into(),
                env::var("PROMPT_SEED").unwrap_or_else(|_| "7".into()),
                "--output".into(),
                prompts.display().to_string(),
                "--report".into(),
                report_path.display().to_string(),
            ];
            if let Ok(local_jsonl) = env::var("PROMPT_LOCAL_JSONL") {
                prompt_args.extend(["--local-jsonl".into(), local_jsonl]);
            } else if let Ok(local_parquet) = env::var("PROMPT_LOCAL_PARQUET") {
                prompt_args.extend(["--local-parquet".into(), local_parquet]);
            } else {
                let revision = env::var("PROMPT_REVISION").unwrap_or_else(|_| "main".into());
                for (key, default) in [
                    ("PROMPT_DATASET", "metrum-ai/prompt-library"),
                    ("PROMPT_CONFIG", "sample"),
                ] {
                    prompt_args.extend([
                        "--".to_owned() + &key[7..].to_ascii_lowercase(),
                        env::var(key).unwrap_or_else(|_| default.into()),
                    ]);
                }
                prompt_args.extend(["--revision".into(), revision.clone()]);
                if !revision.chars().all(|c| c.is_ascii_hexdigit()) || revision.len() != 40 {
                    prompt_args.push("--allow-moving-revision".into());
                }
            }
            prompt_args.extend([
                "--profile".into(),
                env::var("PROMPT_PROFILE").unwrap_or_else(|_| "chat-short".into()),
            ]);
            let prompt_output = Command::new(&prompt_bin)
                .args(&prompt_args)
                .output()
                .context("extract Hub prompts")?;
            if !prompt_output.status.success() {
                bail!(
                    "prompt extraction failed: {}",
                    String::from_utf8_lossy(&prompt_output.stderr)
                );
            }
            let report: Value = serde_json::from_slice(&fs::read(&report_path)?)?;
            let selected = report["selected_count"]
                .as_u64()
                .context("prompt report missing selected_count")? as u32;
            measured_count = selected;
            let recommended = report["recommended_max_tokens"].as_u64().unwrap_or(64);
            let note = format!("prompts: dataset={} revision={} config={} profile={} rows={} recommended_max_tokens={recommended}", report["dataset"].as_str().unwrap_or("local"), report["revision"].as_str().unwrap_or("n/a"), report["config"].as_str().unwrap_or("n/a"), report["profile"]["name"].as_str().unwrap_or("custom"), selected);
            let mut sut_json: Value = serde_json::from_slice(&fs::read(&sut)?)?;
            sut_json["notes"] = json!(format!(
                "{}; {note}",
                sut_json["notes"].as_str().unwrap_or("")
            ));
            fs::write(
                &sut,
                format!("{}\n", serde_json::to_string_pretty(&sut_json)?),
            )?;
            for (key, value) in [
                (
                    "prompt_dataset",
                    report["dataset"].as_str().unwrap_or("local").to_owned(),
                ),
                (
                    "prompt_revision",
                    report["revision"].as_str().unwrap_or("n/a").to_owned(),
                ),
                (
                    "prompt_config",
                    report["config"].as_str().unwrap_or("n/a").to_owned(),
                ),
                (
                    "prompt_profile",
                    report["profile"]["name"]
                        .as_str()
                        .unwrap_or("custom")
                        .to_owned(),
                ),
                ("prompt_rows", selected.to_string()),
                (
                    "prompt_schedule_sha256",
                    report["schedule_sha256"]
                        .as_str()
                        .unwrap_or("n/a")
                        .to_owned(),
                ),
            ] {
                stamp_sut(&sut, key, &json!(value))?;
            }
            command_args.extend([
                "--url".into(),
                format!("{base_url}/v1/chat/completions"),
                "--mode".into(),
                "chat".into(),
                "--streaming".into(),
                "--prompts".into(),
                prompts.display().to_string(),
                "--max-tokens".into(),
                env::var("MAX_TOKENS").unwrap_or_else(|_| recommended.to_string()),
                "--log-level".into(),
                "warn".into(),
                "--extra-body-json".into(),
                env::var("LLM_EXTRA_BODY").unwrap_or_else(|_| {
                    r#"{"chat_template_kwargs":{"enable_thinking":false}}"#.into()
                }),
            ]);
        }
        "vlm" => {
            let prompts = out.join("prompts.jsonl");
            rewrite_jsonl_paths(
                &root.join("test-data/vlm/prompts.jsonl"),
                &prompts,
                "image_url",
                &root,
            )?;
            command_args.extend([
                "--url".into(),
                format!("{base_url}/v1/chat/completions"),
                "--streaming".into(),
                "--prompts".into(),
                prompts.display().to_string(),
                "--max-tokens".into(),
                env::var("MAX_TOKENS").unwrap_or_else(|_| "64".into()),
                "--log-level".into(),
                "warn".into(),
            ]);
        }
        "asr" => {
            let input = out.join("input.jsonl");
            rewrite_jsonl_paths(
                &root.join("test-data/asr/input.jsonl"),
                &input,
                "path",
                &root,
            )?;
            command_args.extend([
                "--url".into(),
                format!("{base_url}/v1/audio/transcriptions"),
                "--input".into(),
                input.display().to_string(),
                "--ground-truth".into(),
                root.join("test-data/asr/truth.jsonl").display().to_string(),
                "--response-format".into(),
                "json".into(),
                "--log-level".into(),
                "warn".into(),
            ]);
        }
        _ => command_args.extend([
            "--url".into(),
            format!("{base_url}/v1"),
            "--prompt".into(),
            env::var("IMAGEGEN_PROMPT").unwrap_or_else(|_| {
                "A red circle, a blue square, and a green triangle on a white background".into()
            }),
            "--size".into(),
            env::var("IMAGEGEN_SIZE").unwrap_or_else(|_| "1024x1024".into()),
            "--num-inference-steps".into(),
            env::var("IMAGEGEN_STEPS").unwrap_or_else(|_| "9".into()),
            "--guidance-scale".into(),
            env::var("IMAGEGEN_GUIDANCE").unwrap_or_else(|_| "0.0".into()),
            "--seed".into(),
            args.seed.to_string(),
            "--artifact-dir".into(),
            artifact_dir.display().to_string(),
        ]),
    }
    command_args.extend([
        "--api-key".into(),
        "dummy".into(),
        "--scenario".into(),
        format!("local-smoke-{}", args.modality),
        "--model".into(),
        model.into(),
        "--num-requests".into(),
        measured_count.to_string(),
        "--warmup-requests".into(),
        args.warmup.to_string(),
        "--concurrency".into(),
        args.concurrency.to_string(),
        "--data-log".into(),
        data_log.display().to_string(),
        "--debug-log".into(),
        debug_log.display().to_string(),
        "--error-log".into(),
        error_log.display().to_string(),
        "--sut".into(),
        sut.display().to_string(),
        "--require-sut".into(),
    ]);
    if args.modality != "imagegen" {
        command_args.extend(["--seed".into(), args.seed.to_string()]);
    }
    fs::create_dir_all(&artifact_dir)?;
    run_child(&binary, &command_args, &root, &out)?;
    let mut check_args = vec![
        "assert-headline".to_owned(),
        args.modality.clone(),
        data_log.display().to_string(),
    ];
    if args.modality == "imagegen" {
        check_args.extend(["--artifact-dir".into(), artifact_dir.display().to_string()]);
    }
    let check = Command::new(env::current_exe()?)
        .args(check_args)
        .status()
        .context("run headline assertion")?;
    if !check.success() {
        bail!("headline assertion failed with {check}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_paths_are_resolved_and_sut_extra_stays_string_compatible() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("input.jsonl");
        let target = dir.path().join("out.jsonl");
        fs::write(
            &source,
            "{\"id\":\"row1\",\"path\":\"test-data/asr/a.wav\"}\n",
        )
        .unwrap();
        rewrite_jsonl_paths(&source, &target, "path", dir.path()).unwrap();
        let row: Value = serde_json::from_str(fs::read_to_string(&target).unwrap().trim()).unwrap();
        assert_eq!(
            row["path"],
            dir.path().join("test-data/asr/a.wav").display().to_string()
        );
        let sut = dir.path().join("sut.json");
        fs::write(&sut, r#"{"notes":"declared","extra":{}}"#).unwrap();
        stamp_sut(&sut, "prompt_rows", &json!("17")).unwrap();
        let value: Value = serde_json::from_slice(&fs::read(sut).unwrap()).unwrap();
        assert_eq!(value["extra"]["prompt_rows"], "17");
    }
}
