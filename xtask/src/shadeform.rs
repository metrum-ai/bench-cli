// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Shadeform lifecycle commands. Mutating commands require explicit confirmation.

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use reqwest::blocking::{Client, Response};
use serde_json::{json, Value};
use std::{env, fs, path::PathBuf, thread, time::Duration};

const PREFERRED_GPU_TYPES: &[&str] = &["RTXPro6000", "B200", "H200", "H100", "H100_nvl", "L40S"];
const DEFAULT_API_BASE: &str = "https://api.shadeform.ai/v1";

#[derive(Subcommand)]
pub enum ShadeformCommand {
    /// List available instances, ordered by preferred GPU, then GPU count and price.
    Types,
    /// Select the best currently available preferred GPU instance.
    Pick,
    /// Build a create payload; POST only when --execute is given.
    Create(CreateArgs),
    /// Fetch instance state and public IP.
    Info { id: String },
    /// Poll instance info until it is active and has a public IP.
    Wait {
        id: String,
        #[arg(long, env = "WAIT_MAX_ATTEMPTS", default_value_t = 60)]
        max_attempts: u32,
        #[arg(long, env = "WAIT_DELAY_SECONDS", default_value_t = 10)]
        delay_seconds: u64,
    },
    /// Request deletion and verify the result with GET /instances/{id}/info.
    Delete { id: String },
}

#[derive(Args)]
pub struct CreateArgs {
    #[arg(long, default_value = "vllm")]
    engine: String,
    #[arg(long, default_value = "llm")]
    modality: String,
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    cloud: Option<String>,
    #[arg(long)]
    region: Option<String>,
    #[arg(long = "type")]
    instance_type: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    image: Option<String>,
    /// Engine arguments are passed through as one string by the Shadeform API.
    #[arg(long)]
    extra_args: Option<String>,
    /// Actually POST the create payload. Without this flag, only print it.
    #[arg(long)]
    execute: bool,
}

struct Shadeform {
    client: Client,
    base: String,
    key: String,
}

impl Shadeform {
    fn new() -> Result<Self> {
        let key = resolve_api_key()?;
        Ok(Self {
            client: Client::builder().timeout(Duration::from_secs(30)).build()?,
            base: env::var("SHADEFORM_API_BASE").unwrap_or_else(|_| DEFAULT_API_BASE.to_owned()),
            key,
        })
    }

    fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Response> {
        let mut request = self
            .client
            .request(
                method,
                format!("{}{}", self.base.trim_end_matches('/'), path),
            )
            .header("X-API-KEY", &self.key)
            .header("Accept", "application/json");
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .with_context(|| format!("Shadeform request {path}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            bail!("Shadeform API {path} returned HTTP {status}: {text}");
        }
        Ok(response)
    }

    fn get(&self, path: &str) -> Result<Value> {
        Ok(self.request(reqwest::Method::GET, path, None)?.json()?)
    }

    fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let response = self.request(reqwest::Method::POST, path, Some(body))?;
        let text = response.text()?;
        if text.trim().is_empty() {
            Ok(Value::Null)
        } else {
            Ok(serde_json::from_str(&text).context("Shadeform returned invalid JSON")?)
        }
    }

    fn delete_verified(&self, id: &str) -> Result<Value> {
        let deleted = self.post(&format!("/instances/{id}/delete"), &json!({}))?;
        let verified = self.get(&format!("/instances/{id}/info"))?;
        let (status, _) = status_and_ip(&verified);
        if !["deleted", "terminated"].contains(&status.as_str()) {
            bail!("delete request returned but GET /info verification did not confirm deletion (status={status})");
        }
        Ok(json!({"delete":deleted,"verified_info":verified}))
    }
}

fn resolve_api_key() -> Result<String> {
    let env_key = env::var("SHADEFORM_API_KEY").unwrap_or_default();
    let env_path = env::var_os("ENV_JSON")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../env.json"));
    let file_key = if env_path.is_file() {
        let value: Value = serde_json::from_slice(
            &fs::read(&env_path).with_context(|| format!("read {}", env_path.display()))?,
        )?;
        value["SHADEFORM_API_KEY"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    } else {
        String::new()
    };
    if !env_key.is_empty() && !file_key.is_empty() && env_key != file_key {
        eprintln!(
            "warning: SHADEFORM_API_KEY in the environment differs from {}; using the file value",
            env_path.display()
        );
        return Ok(file_key);
    }
    if !env_key.is_empty() {
        return Ok(env_key);
    }
    if !file_key.is_empty() {
        return Ok(file_key);
    }
    bail!(
        "SHADEFORM_API_KEY unset and no key found in {}",
        env_path.display()
    )
}

fn gpu_rank(gpu: &str) -> Option<usize> {
    PREFERRED_GPU_TYPES
        .iter()
        .position(|candidate| *candidate == gpu)
}

fn available_types(value: &Value) -> Vec<Value> {
    let mut found = Vec::new();
    for item in value["instance_types"].as_array().into_iter().flatten() {
        let Some(rank) = item["gpu_type"].as_str().and_then(gpu_rank) else {
            continue;
        };
        for availability in item["availability"].as_array().into_iter().flatten() {
            if availability["available"] == true {
                let mut row = item.clone();
                row["region"] = availability["region"].clone();
                row["rank"] = json!(rank);
                found.push(row);
            }
        }
    }
    found.sort_by(|left, right| {
        left["rank"]
            .as_u64()
            .cmp(&right["rank"].as_u64())
            .then_with(|| {
                left["num_gpus"]
                    .as_u64()
                    .unwrap_or(1)
                    .cmp(&right["num_gpus"].as_u64().unwrap_or(1))
            })
            .then_with(|| {
                left["hourly_price"]
                    .as_f64()
                    .unwrap_or(f64::INFINITY)
                    .total_cmp(&right["hourly_price"].as_f64().unwrap_or(f64::INFINITY))
            })
    });
    found
}

fn create_payload(args: &CreateArgs, selected: Option<&Value>) -> Result<Value> {
    if !["llm", "vlm", "asr", "imagegen"].contains(&args.modality.as_str()) {
        bail!("unsupported modality: {}", args.modality);
    }
    if !["vllm", "sglang"].contains(&args.engine.as_str()) {
        bail!("unsupported engine: {}", args.engine);
    }
    let cloud = args
        .cloud
        .as_deref()
        .or_else(|| selected.and_then(|v| v["cloud"].as_str()))
        .context("--cloud is required when no instance is available")?;
    let region = args
        .region
        .as_deref()
        .or_else(|| selected.and_then(|v| v["region"].as_str()))
        .context("--region is required when no instance is available")?;
    let instance_type = args
        .instance_type
        .as_deref()
        .or_else(|| selected.and_then(|v| v["shade_instance_type"].as_str()))
        .context("--type is required when no instance is available")?;
    let default_model = match args.modality.as_str() {
        "vlm" => "Qwen/Qwen2.5-VL-7B-Instruct",
        "asr" => "openai/whisper-large-v3",
        "imagegen" => "stabilityai/stable-diffusion-3.5-large",
        _ => "Qwen/Qwen2.5-7B-Instruct",
    };
    let model = args.model.as_deref().unwrap_or(default_model);
    let image = args.image.as_deref().unwrap_or(match args.engine.as_str() {
        "sglang" => "lmsysorg/sglang",
        _ => "vllm/vllm-openai:latest",
    });
    let mut engine_args = match args.engine.as_str() {
        "sglang" => format!(
            "python3 -m sglang.launch_server --model-path {model} --host 0.0.0.0 --port 8000"
        ),
        _ => format!("--model {model} --host 0.0.0.0 --port 8000"),
    };
    let tensor_parallel = args.extra_args.clone().or_else(|| {
        let suffix = instance_type.rsplit_once('x').map(|(_, count)| count)?;
        match suffix {
            "2" | "4" | "8" => Some(format!("--tensor-parallel-size {suffix}")),
            _ => None,
        }
    });
    if let Some(extra) = tensor_parallel {
        engine_args.push(' ');
        engine_args.push_str(&extra);
    }
    let launch = json!({
        "type":"docker",
        "docker_configuration": {
            "image":image,
            "args":engine_args,
            "shared_memory_in_gb":32,
            "port_mappings":[{"host_port":80,"container_port":8000}]
        }
    });
    let mut payload = json!({
        "cloud":cloud,
        "region":region,
        "shade_instance_type":instance_type,
        "shade_cloud":true,
        "name":args.name.as_deref().map(str::to_owned).unwrap_or_else(|| {
            let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
            format!("metrum-live-{timestamp}")
        }),
        "launch_configuration":launch
    });
    if let Ok(id) = env::var("SHADEFORM_SSH_KEY_ID") {
        payload["ssh_key_id"] = json!(id);
    }
    Ok(payload)
}

fn status_and_ip(value: &Value) -> (String, String) {
    let status = value["status"]
        .as_str()
        .or_else(|| value["instance"]["status"].as_str())
        .unwrap_or("unknown")
        .to_owned();
    let ip = ["ip", "public_ip"]
        .iter()
        .find_map(|key| value[*key].as_str())
        .or_else(|| value["instance"]["ip"].as_str())
        .or_else(|| value["instance"]["public_ip"].as_str())
        .unwrap_or("")
        .to_owned();
    (status, ip)
}

pub fn run(command: ShadeformCommand) -> Result<()> {
    match command {
        ShadeformCommand::Types | ShadeformCommand::Pick => {
            let api = Shadeform::new()?;
            let rows = available_types(&api.get("/instances/types")?);
            if matches!(command, ShadeformCommand::Pick) {
                let best = rows
                    .first()
                    .context("no preferred GPU types currently available")?;
                println!("{}", serde_json::to_string_pretty(best)?);
            } else {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            }
        }
        ShadeformCommand::Create(args) => {
            let selected =
                if args.cloud.is_none() || args.region.is_none() || args.instance_type.is_none() {
                    let api = Shadeform::new()?;
                    available_types(&api.get("/instances/types")?)
                        .into_iter()
                        .next()
                } else {
                    None
                };
            let payload = create_payload(&args, selected.as_ref())?;
            println!("{}", serde_json::to_string_pretty(&payload)?);
            if args.execute {
                let response = Shadeform::new()?.post("/instances/create", &payload)?;
                println!("{}", serde_json::to_string_pretty(&response)?);
            } else {
                println!("# dry-run: no instance was created; pass --execute to POST");
            }
        }
        ShadeformCommand::Info { id } => {
            let value = Shadeform::new()?.get(&format!("/instances/{id}/info"))?;
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        ShadeformCommand::Wait {
            id,
            max_attempts,
            delay_seconds,
        } => {
            let api = Shadeform::new()?;
            for attempt in 1..=max_attempts {
                let value = api.get(&format!("/instances/{id}/info"))?;
                let (status, ip) = status_and_ip(&value);
                eprintln!("# wait attempt {attempt}/{max_attempts}: status={status} ip={ip}");
                if ["active", "running", "ready"].contains(&status.as_str()) && !ip.is_empty() {
                    println!("{ip}");
                    return Ok(());
                }
                if ["failed", "error", "deleted", "terminated"].contains(&status.as_str()) {
                    bail!("instance {id} entered terminal state {status}");
                }
                if attempt < max_attempts {
                    thread::sleep(Duration::from_secs(delay_seconds));
                }
            }
            bail!("timed out waiting for instance {id}");
        }
        ShadeformCommand::Delete { id } => {
            let result = Shadeform::new()?.delete_verified(&id)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
    };

    fn mock_delete(status: &str) -> (Shadeform, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_owned();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        drop(requests);
        let server = thread::spawn(move || {
            for (index, stream) in listener.incoming().take(2).enumerate() {
                let mut stream = stream.unwrap();
                let mut request = [0; 4096];
                let length = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..length]).to_string();
                captured
                    .lock()
                    .unwrap()
                    .push(request.lines().next().unwrap().to_owned());
                let body = if index == 0 {
                    r#"{"ok":true}"#.to_owned()
                } else {
                    format!(r#"{{"status":"{status}"}}"#)
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(), body
                )
                .unwrap();
            }
            Arc::try_unwrap(captured).unwrap().into_inner().unwrap()
        });
        (
            Shadeform {
                client: Client::new(),
                base: format!("http://{address}"),
                key: "test-key".to_owned(),
            },
            server,
        )
    }

    #[test]
    fn preferred_types_exclude_unavailable_and_rank_by_preference() {
        let response = json!({"instance_types":[
            {"gpu_type":"H100","availability":[{"available":true,"region":"west"}],"hourly_price":2.0},
            {"gpu_type":"RTX4090","availability":[{"available":true,"region":"west"}],"hourly_price":0.2},
            {"gpu_type":"RTXPro6000","availability":[{"available":false,"region":"east"},{"available":true,"region":"west"}],"hourly_price":4.0}
        ]});
        let result = available_types(&response);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["gpu_type"], "RTXPro6000");
        assert_eq!(result[1]["gpu_type"], "H100");
        assert_eq!(result[0]["region"], "west");
    }

    #[test]
    fn create_is_safe_payload_by_default_and_accepts_explicit_instance() {
        let args = CreateArgs {
            engine: "vllm".to_owned(),
            modality: "llm".to_owned(),
            name: Some("test".to_owned()),
            cloud: Some("cloud".to_owned()),
            region: Some("region".to_owned()),
            instance_type: Some("gpu".to_owned()),
            model: Some("org/model".to_owned()),
            image: Some("engine:tag".to_owned()),
            extra_args: None,
            execute: false,
        };
        let payload = create_payload(&args, None).unwrap();
        assert_eq!(payload["name"], "test");
        assert_eq!(
            payload["launch_configuration"]["docker_configuration"]["image"],
            "engine:tag"
        );
        assert_eq!(
            payload["launch_configuration"]["docker_configuration"]["args"],
            "--model org/model --host 0.0.0.0 --port 8000"
        );
        assert!(!args.execute);
    }

    #[test]
    fn delete_verification_uses_info_and_fails_hard_when_still_active() {
        let (api, server) = mock_delete("deleted");
        let result = api.delete_verified("instance-1").unwrap();
        assert_eq!(result["verified_info"]["status"], "deleted");
        let requests = server.join().unwrap();
        assert_eq!(
            requests,
            [
                "POST /instances/instance-1/delete HTTP/1.1",
                "GET /instances/instance-1/info HTTP/1.1"
            ]
        );

        let (api, server) = mock_delete("active");
        let error = api.delete_verified("instance-2").unwrap_err();
        assert!(error.to_string().contains("GET /info verification"));
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].starts_with("GET /instances/instance-2/info"));
    }

    #[test]
    fn bad_modality_and_engine_are_rejected() {
        let mut args = CreateArgs {
            engine: "bad".to_owned(),
            modality: "llm".to_owned(),
            name: None,
            cloud: Some("c".to_owned()),
            region: Some("r".to_owned()),
            instance_type: Some("t".to_owned()),
            model: None,
            image: None,
            extra_args: None,
            execute: false,
        };
        assert!(create_payload(&args, None).is_err());
        args.engine = "vllm".to_owned();
        args.modality = "bad".to_owned();
        assert!(create_payload(&args, None).is_err());
    }
}
