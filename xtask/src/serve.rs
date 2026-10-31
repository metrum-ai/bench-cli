// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Docker serving commands driven by the checked-in campaign smoke data.

use anyhow::{bail, Context, Result};
use chrono::{SecondsFormat, Utc};
use clap::Args;
use reqwest::blocking::Client;
use serde_json::{json, Value};
use serde_yaml::Value as YamlValue;
use std::{
    env, fs,
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, Instant},
};

#[derive(Args)]
pub struct ServeArgs {
    modality: String,
    operation: String,
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    image: Option<String>,
    /// Replacement server flags, separated by spaces. Use SUT notes and sources with overrides.
    #[arg(long)]
    args: Option<String>,
    #[arg(long)]
    notes: Option<String>,
    #[arg(long, value_delimiter = ' ')]
    sources: Vec<String>,
    #[arg(long, default_value_t = 8000)]
    port: u16,
    #[arg(long, default_value_t = 1800)]
    ready_timeout_seconds: u64,
    #[arg(long)]
    logs_output: Option<PathBuf>,
}

struct Config {
    modality: String,
    image: String,
    model: String,
    args: Vec<String>,
    env: Vec<String>,
    entrypoint: Vec<String>,
    sources: Vec<String>,
    notes: String,
}

fn load_config(args: &ServeArgs) -> Result<Config> {
    let path = args
        .root
        .join(".claude/skills/campaign/serving/ci-smoke.yaml");
    let document: YamlValue = serde_yaml::from_slice(
        &fs::read(&path).with_context(|| format!("read {}", path.display()))?,
    )?;
    let entry = &document[&args.modality];
    if entry.is_null() {
        bail!(
            "unknown serving modality {:?}; see {}",
            args.modality,
            path.display()
        );
    }
    let strings = |key: &str| -> Result<Vec<String>> {
        entry[key]
            .as_sequence()
            .context(format!("{key} must be a list"))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .context(format!("{key} entries must be strings"))
            })
            .collect()
    };
    Ok(Config {
        modality: args.modality.clone(),
        image: entry["image"]
            .as_str()
            .context("image is required")?
            .to_owned(),
        model: entry["model"]
            .as_str()
            .context("model is required")?
            .to_owned(),
        args: strings("args")?,
        env: strings("env")?,
        entrypoint: strings("entrypoint")?,
        sources: strings("sources")?,
        notes: entry["notes"]
            .as_str()
            .context("notes are required")?
            .to_owned(),
    })
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_./:=+-".contains(&byte))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn hf_cache_path() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| "/tmp".to_owned());
    env::var_os("HF_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(home).join(".cache/huggingface"))
}

fn prepare_hf_cache(cache: &std::path::Path) -> Result<()> {
    fs::create_dir_all(cache)
        .with_context(|| format!("create Hugging Face cache directory {}", cache.display()))
}

fn docker_args(
    config: &Config,
    image: &str,
    model: &str,
    args: &[String],
    port: u16,
    cache: &std::path::Path,
) -> Vec<String> {
    let name = format!("metrum-live-{}", config.modality);
    let mut command = vec![
        "run".into(),
        "-d".into(),
        "--name".into(),
        name,
        "--gpus".into(),
        env::var("GPU_DEVICES").unwrap_or_else(|_| "device=0".into()),
        "--ipc=host".into(),
        "-p".into(),
        format!("{port}:8000"),
        "-v".into(),
        format!("{}:/root/.cache/huggingface", cache.display()),
    ];
    if env::var_os("HF_TOKEN").is_some() {
        command.extend(["-e".into(), "HF_TOKEN".into()]);
    }
    for value in &config.env {
        command.extend(["-e".into(), value.clone()]);
    }
    command.push(image.to_owned());
    command.extend(config.entrypoint.iter().cloned());
    command.push(model.to_owned());
    command.extend(args.iter().cloned());
    command
}

fn quantization(model: &str, args: &[String]) -> Result<(Option<String>, String)> {
    if let Ok(value) = env::var("QUANTIZATION") {
        let value = value.trim();
        if value.chars().any(char::is_whitespace) {
            bail!("QUANTIZATION must be one word (or none)");
        }
        return Ok((
            if value.eq_ignore_ascii_case("none")
                || value.eq_ignore_ascii_case("null")
                || value.is_empty()
            {
                None
            } else {
                Some(value.to_owned())
            },
            "env".into(),
        ));
    }
    for (index, value) in args.iter().enumerate() {
        if value == "--quantization" || value == "-q" {
            return Ok((args.get(index + 1).cloned(), "serve_args".into()));
        }
        if let Some(value) = value.strip_prefix("--quantization=") {
            return Ok((Some(value.to_owned()), "serve_args".into()));
        }
    }
    let basename = model
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    let tokens: Vec<_> = basename.split(['-', '.', '_']).collect();
    let mut weak = None;
    for token in tokens.iter().rev() {
        let result: Option<Option<String>> = match *token {
            "nvfp4" | "nvfp4a16" | "mxfp4" | "awq" | "gptq" | "gguf" | "exl2" | "exl3" | "hqq"
            | "aqlm" | "int4wo" | "int8wo" => Some(Some((*token).to_owned())),
            "fp8" | "fp8e4m3" | "fp8e5m2" | "fp8dynamic" => Some(Some("fp8".into())),
            "bnb" | "bitsandbytes" => Some(Some("bitsandbytes".into())),
            "bf16" | "fp16" | "f16" | "fp32" | "f32" => Some(None),
            "int4" | "int8" | "fp4" => {
                weak.get_or_insert(*token);
                None
            }
            _ if token.starts_with('w') && token.contains('a') => {
                let (weight, activation_group) = token.split_once('a').unwrap_or(("", ""));
                let (activation, group) = activation_group
                    .split_once('g')
                    .unwrap_or((activation_group, ""));
                let valid_weight = weight.strip_prefix('w').is_some_and(|part| {
                    !part.is_empty() && part.chars().all(|c| c.is_ascii_digit())
                });
                let valid_activation =
                    !activation.is_empty() && activation.chars().all(|c| c.is_ascii_digit());
                let valid_group = group.is_empty() || group.chars().all(|c| c.is_ascii_digit());
                if valid_weight && valid_activation && valid_group {
                    Some(Some(format!("{weight}a{activation}")))
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(value) = result {
            return Ok((value, "model_name".into()));
        }
    }
    Ok((
        weak.map(str::to_owned),
        if weak.is_some() { "model_name" } else { "none" }.into(),
    ))
}

fn validate_overrides(
    config: &Config,
    model: &str,
    image: &str,
    args: &[String],
    notes: Option<&str>,
    sources: &[String],
) -> Result<()> {
    let changed = model != config.model || image != config.image || args != config.args;
    if changed && (notes.is_none_or(str::is_empty) || sources.is_empty()) {
        bail!("serving overrides require --notes and --sources from current upstream research");
    }
    Ok(())
}

struct SutInput<'a> {
    config: &'a Config,
    image: &'a str,
    model: &'a str,
    args: &'a [String],
    docker: &'a [String],
    sources: &'a [String],
    notes: &'a str,
    revision: Option<&'a str>,
}

fn sut_json(input: SutInput<'_>) -> Result<Value> {
    let SutInput {
        config,
        image,
        model,
        args,
        docker,
        sources,
        notes,
        revision,
    } = input;
    let (quantization, quantization_source) = quantization(model, args)?;
    let gpu = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total,driver_version",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| text.lines().next().map(str::to_owned));
    let (gpu_model, memory_gb, driver) = if let Some(line) = gpu {
        let values: Vec<_> = line.split(',').map(str::trim).collect();
        (
            values.first().copied().unwrap_or("not captured").to_owned(),
            values
                .get(1)
                .and_then(|value| value.parse::<f64>().ok())
                .map(|value| (value / 1024.0).round() as u64),
            values.get(2).map(|value| format!("NVIDIA {value}")),
        )
    } else {
        (
            "not captured (nvidia-smi unavailable)".into(),
            None,
            Some("not captured".into()),
        )
    };
    let mut notes = notes.to_owned();
    if quantization_source == "model_name" {
        notes.push_str(&format!(
            "; model.quantization={} derived from MODEL name {model}",
            quantization.as_deref().unwrap_or("null")
        ));
    }
    let runtime = image
        .split('/')
        .next_back()
        .unwrap_or(image)
        .split(':')
        .collect::<Vec<_>>();
    let home = env::var("HOME").unwrap_or_default();
    let docker_config = docker
        .iter()
        .map(|arg| {
            shell_quote(&if home.is_empty() {
                arg.clone()
            } else {
                arg.replace(&home, "~")
            })
        })
        .collect::<Vec<_>>()
        .join(" ");
    Ok(json!({
        "provenance":"declared", "name":format!("local GPU / {} live smoke", config.modality), "vendor":"NVIDIA",
        "gpu":{"model":gpu_model,"count":1,"memory_gb":memory_gb}, "driver_version":driver,
        "runtime":{"name":runtime[0],"version":runtime.get(1).copied().unwrap_or("unknown"),"config":format!("docker {docker_config}")},
        "model":{"id":model,"revision":revision,"quantization":quantization},
        "host_os":host_os(), "notes":notes,
        "extra":{"launcher_sources":sources.join(" "),"image":image,"quantization_source":quantization_source,"recorded_at_utc":Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)}
    }))
}

fn host_os() -> String {
    Command::new("uname")
        .args(["-s", "-r"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .unwrap_or_else(|| format!("{} {}", env::consts::OS, env::consts::ARCH))
}

fn model_revision(model: &str) -> Option<String> {
    if env::var("HF_HUB_OFFLINE").is_ok_and(|value| value == "1") {
        return None;
    }
    Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .ok()?
        .get(format!(
            "https://huggingface.co/api/models/{model}/revision/main"
        ))
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .json::<Value>()
        .ok()?
        .get("sha")?
        .as_str()
        .map(str::to_owned)
}

fn wait_ready(client: &Client, port: u16, name: &str, timeout: Duration) -> Result<()> {
    let url = format!("http://127.0.0.1:{port}/v1/models");
    let deadline = Instant::now() + timeout;
    loop {
        if client
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(());
        }
        let running = Command::new("docker")
            .args(["ps", "-q", "-f", &format!("name=^{name}$")])
            .output()
            .is_ok_and(|output| output.status.success() && !output.stdout.is_empty());
        if !running {
            bail!(
                "container {name} exited before ready; inspect `cargo xtask serve {} logs`",
                name.strip_prefix("metrum-live-").unwrap_or(name)
            );
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for {url}");
        }
        thread::sleep(Duration::from_secs(10));
    }
}

pub fn run(args: ServeArgs) -> Result<()> {
    let config = load_config(&args)?;
    let model = args.model.as_deref().unwrap_or(&config.model);
    let image = args.image.as_deref().unwrap_or(&config.image);
    let server_args = args
        .args
        .as_deref()
        .map(|value| {
            value
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| config.args.clone());
    validate_overrides(
        &config,
        model,
        image,
        &server_args,
        args.notes.as_deref(),
        &args.sources,
    )?;
    let notes = args.notes.as_deref().unwrap_or(&config.notes);
    let sources = if args.sources.is_empty() {
        &config.sources
    } else {
        &args.sources
    };
    let cache = hf_cache_path();
    let docker = docker_args(&config, image, model, &server_args, args.port, &cache);
    let container = format!("metrum-live-{}", config.modality);
    match args.operation.as_str() {
        "print" => println!(
            "docker {}",
            docker
                .iter()
                .map(|arg| shell_quote(arg))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        "sut" => println!(
            "{}",
            serde_json::to_string_pretty(&sut_json(SutInput {
                config: &config,
                image,
                model,
                args: &server_args,
                docker: &docker,
                sources,
                notes,
                revision: model_revision(model).as_deref(),
            })?)?
        ),
        "stop" => {
            let _ = Command::new("docker")
                .args(["rm", "-f", &container])
                .status();
            println!("stopped {container}");
        }
        "logs" => {
            let output = Command::new("docker").args(["logs", &container]).output()?;
            if !output.status.success() {
                bail!("docker logs exited with {}", output.status);
            }
            if let Some(path) = args.logs_output {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut bytes = output.stdout;
                bytes.extend_from_slice(&output.stderr);
                fs::write(path, bytes)?;
            } else {
                print!("{}", String::from_utf8_lossy(&output.stdout));
                eprint!("{}", String::from_utf8_lossy(&output.stderr));
            }
        }
        "start" => {
            // Docker otherwise creates a missing bind source as root on the host.
            prepare_hf_cache(&cache)?;
            let output = env::var("SERVE_OUT")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    args.root
                        .join(format!("live-results/serve-{}", config.modality))
                });
            fs::create_dir_all(&output)?;
            fs::write(
                output.join("sut.json"),
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&sut_json(SutInput {
                        config: &config,
                        image,
                        model,
                        args: &server_args,
                        docker: &docker,
                        sources,
                        notes,
                        revision: model_revision(model).as_deref(),
                    })?)?
                ),
            )?;
            let _ = Command::new("docker")
                .args(["rm", "-f", &container])
                .status();
            let status = Command::new("docker")
                .args(&docker)
                .status()
                .context("run docker")?;
            if !status.success() {
                bail!("docker run exited with {status}");
            }
            wait_ready(
                &Client::new(),
                args.port,
                &container,
                Duration::from_secs(args.ready_timeout_seconds),
            )?;
            println!(
                "ready {} {} on port {}; SUT at {}",
                config.modality,
                model,
                args.port,
                output.join("sut.json").display()
            );
        }
        operation => {
            bail!("unknown serve operation {operation:?}; expected start|stop|print|logs|sut")
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_args(modality: &str) -> ServeArgs {
        ServeArgs {
            modality: modality.to_owned(),
            operation: "sut".to_owned(),
            root: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .to_path_buf(),
            model: None,
            image: None,
            args: None,
            notes: None,
            sources: Vec::new(),
            port: 8000,
            ready_timeout_seconds: 1,
            logs_output: None,
        }
    }

    #[test]
    fn cache_mount_source_is_created_before_docker_and_errors_name_it() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("fresh-home/.cache/huggingface");
        prepare_hf_cache(&cache).unwrap();
        assert!(cache.is_dir());
        // mkdir is idempotent for an existing source.
        prepare_hf_cache(&cache).unwrap();
        let config = load_config(&test_args("llm")).unwrap();
        let docker = docker_args(
            &config,
            &config.image,
            &config.model,
            &config.args,
            8000,
            &cache,
        );
        assert!(docker.contains(&format!("{}:/root/.cache/huggingface", cache.display())));

        let file = dir.path().join("file");
        fs::write(&file, "not a directory").unwrap();
        let bad = file.join("huggingface");
        let error = prepare_hf_cache(&bad).unwrap_err();
        assert!(format!("{error:#}").contains(&bad.display().to_string()));
    }

    #[test]
    fn serving_defaults_generate_valid_sut_for_all_modalities_offline() {
        for modality in ["llm", "vlm", "asr", "imagegen"] {
            let args = test_args(modality);
            let config = load_config(&args).unwrap();
            let docker = docker_args(
                &config,
                &config.image,
                &config.model,
                &config.args,
                args.port,
                &hf_cache_path(),
            );
            let sut = sut_json(SutInput {
                config: &config,
                image: &config.image,
                model: &config.model,
                args: &config.args,
                docker: &docker,
                sources: &config.sources,
                notes: &config.notes,
                revision: None,
            })
            .unwrap();
            assert_eq!(sut["provenance"], "declared");
            assert_eq!(sut["model"]["id"], config.model);
            assert!(sut["runtime"]["config"]
                .as_str()
                .unwrap()
                .contains(&config.image));
            assert_eq!(sut["extra"]["image"], config.image);
            assert!(!config.sources.is_empty());
        }
    }

    #[test]
    fn serving_overrides_require_both_researched_notes_and_sources() {
        let config = load_config(&test_args("llm")).unwrap();
        assert!(validate_overrides(
            &config,
            &config.model,
            &config.image,
            &config.args,
            None,
            &[]
        )
        .is_ok());
        assert!(
            validate_overrides(&config, "org/other", &config.image, &config.args, None, &[])
                .is_err()
        );
        assert!(validate_overrides(
            &config,
            "org/other",
            &config.image,
            &config.args,
            Some("researched"),
            &[]
        )
        .is_err());
        assert!(validate_overrides(
            &config,
            "org/other",
            &config.image,
            &config.args,
            Some("researched"),
            &["https://example.invalid".into()]
        )
        .is_ok());
    }

    #[test]
    fn quantization_markers_match_existing_serve_sut_contract() {
        for (model, expected) in [
            ("x/Model_W8A8", Some("w8a8")),
            ("x/Model-W8A16", Some("w8a16")),
            ("x/Model-w4a16g128", Some("w4a16")),
            ("x/Model-W4A16-G128", Some("w4a16")),
            ("x/Model-NVFP4A16", Some("nvfp4a16")),
            ("x/Model-FP8E4M3", Some("fp8")),
            ("x/Model-Int4wo", Some("int4wo")),
            ("x/Model-GPTQ-Int4", Some("gptq")),
            ("nvidia/Model-FP4", Some("fp4")),
            ("x/Model-FP8-to-BF16", None),
            ("x/awqward-fp80", None),
        ] {
            assert_eq!(
                quantization(model, &[]).unwrap().0.as_deref(),
                expected,
                "{model}"
            );
        }
        assert_eq!(
            quantization("org/Model-FP8/", &[]).unwrap().0.as_deref(),
            Some("fp8")
        );
    }
}
