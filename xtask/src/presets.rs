// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Validate the checked-in campaign preset catalog before it reaches a live run.

use anyhow::{bail, Context, Result};
use serde_yaml::Value;
use std::{
    fs,
    io::Write,
    net::TcpListener,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const REQUIRED: &[&str] = &[
    "llm-matrix",
    "vlm-matrix",
    "asr-matrix",
    "imagegen-matrix",
    "smoke-cell",
    "telemetry-ladder",
    "aiperf-bakeoff",
    "parity-pair",
];

fn positive_int(value: &Value) -> bool {
    value.as_u64().is_some_and(|number| number > 0)
}

fn validate(path: &Path, text: &str) -> Result<()> {
    if !text
        .lines()
        .any(|line| line.trim() == "# Copyright (c) 2026 Metrum AI, Inc.")
        || !text
            .lines()
            .any(|line| line.trim() == "# SPDX-License-Identifier: Apache-2.0")
    {
        bail!("{}: missing copyright/SPDX comments", path.display());
    }
    if !text.lines().any(|line| line.starts_with("# Provenance:")) {
        bail!("{}: missing provenance comment", path.display());
    }
    let document: Value =
        serde_yaml::from_str(text).with_context(|| format!("parse preset {}", path.display()))?;
    let object = document
        .as_mapping()
        .context("preset root must be a mapping")?;
    if let Some(workloads) = object.get(Value::String("workloads".into())) {
        let workloads = workloads
            .as_sequence()
            .context("workloads must be a list")?;
        if workloads.is_empty()
            || workloads.iter().any(|workload| {
                workload.as_str().is_none_or(|name| {
                    !["llm", "vlm", "asr", "imagegen", "embeddings", "rerank"].contains(&name)
                })
            })
        {
            bail!(
                "{}: workloads must contain recognized modality names",
                path.display()
            );
        }
    } else if !["parity-pair", "smoke-cell"].contains(
        &path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default(),
    ) {
        bail!("{}: workloads is required", path.display());
    }
    if let Some(concurrency) = object.get(Value::String("concurrency".into())) {
        let values = concurrency
            .as_sequence()
            .context("concurrency must be a list")?;
        if values.is_empty() || values.iter().any(|value| !positive_int(value)) {
            bail!(
                "{}: concurrency must contain positive integers",
                path.display()
            );
        }
    }
    if let Some(requests) = object.get(Value::String("requests".into())) {
        if !positive_int(requests) && requests.as_str().is_none() {
            bail!(
                "{}: requests must be a positive integer or an expression string",
                path.display()
            );
        }
    }
    for key in ["warmup", "seed", "max_tokens", "budget"] {
        if let Some(value) = object.get(Value::String(key.into())) {
            if !positive_int(value) && value.as_str().is_none() {
                bail!(
                    "{}: {key} must be a positive integer or documented expression",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

fn integer(value: Option<&Value>, default: u64) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(default)
}

fn write_paths(source: &Path, target: &Path, field: &str, root: &Path) -> Result<()> {
    let contents = fs::read_to_string(source)?;
    let mut output = fs::File::create(target)?;
    for line in contents.lines().filter(|line| !line.trim().is_empty()) {
        let mut row: serde_json::Value = serde_json::from_str(line)?;
        let path = row[field].as_str().context("fixture path missing")?;
        row[field] = serde_json::json!(root.join(path).display().to_string());
        writeln!(output, "{}", serde_json::to_string(&row)?)?;
    }
    Ok(())
}

fn workload_args(
    workload: &str,
    preset: &Value,
    root: &Path,
    url: &str,
    dir: &Path,
) -> Result<(String, Vec<String>)> {
    let name = match workload {
        "llm" => "metrum-ai-bench-cli-llm",
        "vlm" => "metrum-ai-bench-cli-vlm",
        "asr" => "metrum-ai-bench-cli-asr",
        "imagegen" => "metrum-ai-bench-cli-imagegen",
        _ => bail!("no runnable mock command for workload {workload}"),
    };
    let concurrency = preset["concurrency"]
        .as_sequence()
        .and_then(|values| values.first())
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let seed = integer(preset.get("seed"), 7);
    let max_tokens = preset
        .get("max_tokens")
        .and_then(Value::as_u64)
        .or_else(|| {
            preset["max_tokens"].as_str().and_then(|text| {
                text.split(|c: char| !c.is_ascii_digit())
                    .find_map(|part| part.parse().ok())
            })
        })
        .unwrap_or(4);
    let data = dir.join(format!("{workload}.jsonl"));
    let mut args = Vec::new();
    match workload {
        "llm" => {
            fs::write(&data, "{\"prompt\":\"Say hello\"}\n")?;
            args.extend([
                "--url".into(),
                format!("{url}/v1/chat/completions"),
                "--mode".into(),
                "chat".into(),
                "--streaming".into(),
                "--prompts".into(),
                data.display().to_string(),
                "--max-tokens".into(),
                max_tokens.min(8).to_string(),
                "--seed".into(),
                seed.to_string(),
            ]);
        }
        "vlm" => {
            write_paths(
                &root.join("test-data/vlm/prompts.jsonl"),
                &data,
                "image_url",
                root,
            )?;
            args.extend([
                "--url".into(),
                format!("{url}/v1/chat/completions"),
                "--streaming".into(),
                "--prompts".into(),
                data.display().to_string(),
                "--max-tokens".into(),
                max_tokens.min(8).to_string(),
            ]);
        }
        "asr" => {
            write_paths(&root.join("test-data/asr/input.jsonl"), &data, "path", root)?;
            args.extend([
                "--url".into(),
                format!("{url}/v1/audio/transcriptions"),
                "--input".into(),
                data.display().to_string(),
                "--ground-truth".into(),
                root.join("test-data/asr/truth.jsonl").display().to_string(),
                "--response-format".into(),
                "json".into(),
            ]);
        }
        _ => args.extend([
            "--url".into(),
            format!("{url}/v1"),
            "--prompt".into(),
            "mock preset smoke".into(),
            "--size".into(),
            "64x64".into(),
            "--num-inference-steps".into(),
            "1".into(),
            "--guidance-scale".into(),
            "0".into(),
            "--seed".into(),
            seed.to_string(),
            "--artifact-dir".into(),
            dir.join("artifacts").display().to_string(),
            "--response-format".into(),
            "b64_json".into(),
        ]),
    }
    args.extend([
        "--api-key".into(),
        "dummy".into(),
        "--scenario".into(),
        format!("check-presets-{workload}"),
        "--model".into(),
        "mock".into(),
        "--num-requests".into(),
        "1".into(),
        "--warmup-requests".into(),
        "0".into(),
        "--concurrency".into(),
        concurrency.min(2).to_string(),
        "--data-log".into(),
        dir.join("results.jsonl").display().to_string(),
    ]);
    if workload != "imagegen" {
        args.extend(["--log-level".into(), "error".into()]);
    }
    Ok((name.to_owned(), args))
}

fn run_mock_commands(root: &Path) -> Result<()> {
    let debug = root.join("target/debug");
    let mock = debug.join("metrum-ai-bench-cli-mock-server");
    if !mock.is_file() {
        bail!("check-presets requires the already-built target/debug mock server; run cargo build --bins first");
    }
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    let mut server = Command::new(&mock)
        .args(["--listen", &format!("127.0.0.1:{port}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("start mock server")?;
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::blocking::Client::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline
        && !client
            .get(format!("{base}/health"))
            .send()
            .is_ok_and(|response| response.status().is_success())
    {
        thread::sleep(Duration::from_millis(50));
    }
    let result = (|| -> Result<()> {
        if !client
            .get(format!("{base}/health"))
            .send()
            .is_ok_and(|response| response.status().is_success())
        {
            bail!("mock server did not become ready");
        }
        let temp = tempfile::tempdir()?;
        let directory = root.join(".claude/skills/campaign/presets");
        for name in REQUIRED {
            let path = directory.join(format!("{name}.yaml"));
            let text =
                fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
            validate(&path, &text)?;
            let preset: Value = serde_yaml::from_str(&text)?;
            let workloads = if let Some(values) = preset["workloads"].as_sequence() {
                values.iter().filter_map(Value::as_str).collect::<Vec<_>>()
            } else if *name == "smoke-cell" {
                vec!["llm", "vlm", "asr", "imagegen"]
            } else {
                vec!["llm"]
            };
            for workload in workloads {
                let dir = temp.path().join(format!("{name}-{workload}"));
                fs::create_dir_all(&dir)?;
                let (binary, args) = workload_args(workload, &preset, root, &base, &dir)?;
                let executable = debug.join(&binary);
                if !executable.is_file() {
                    bail!(
                        "check-presets requires already-built binary {}",
                        executable.display()
                    );
                }
                let output = Command::new(executable)
                    .args(args)
                    .current_dir(root)
                    .output()
                    .with_context(|| format!("run preset {name} for {workload}"))?;
                if !output.status.success() {
                    bail!(
                        "preset {name} for {workload} failed against mock (exit {}): {}",
                        output.status,
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                let rows = fs::read_to_string(dir.join("results.jsonl"))
                    .with_context(|| format!("preset {name} did not write request rows"))?;
                if rows.lines().filter(|line| !line.trim().is_empty()).count() == 0 {
                    bail!("preset {name} for {workload} produced no request rows");
                }
                println!("PASS {name}/{workload}: executed against Rust mock, N>0");
            }
        }
        Ok(())
    })();
    let _ = server.kill();
    let _ = server.wait();
    result
}

pub fn run(root: &Path) -> Result<()> {
    run_mock_commands(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_campaign_catalog_passes_schema_checks() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let directory = root.join(".claude/skills/campaign/presets");
        for name in REQUIRED {
            let path = directory.join(format!("{name}.yaml"));
            let text = fs::read_to_string(&path).unwrap();
            validate(&path, &text).unwrap();
        }
    }

    #[test]
    fn malformed_or_unattributed_preset_is_rejected() {
        let path = Path::new("broken.yaml");
        assert!(validate(path, "workloads: [llm]\n").is_err());
        let attributed = "# Copyright (c) 2026 Metrum AI, Inc.\n# SPDX-License-Identifier: Apache-2.0\n# Provenance: test\nworkloads: [typo]\n";
        assert!(validate(path, attributed).is_err());
    }
}
