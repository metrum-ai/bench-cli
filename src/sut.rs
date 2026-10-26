// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! System-under-test declarations for publishable runs.
//!
//! `--sut` embeds an operator-supplied block. `sut init` can write a template
//! or probe **local** host facts (`--probe`); it does not SSH-audit the remote
//! serving host. See `field_provenance` for observed vs declared fields.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Operator-declared system under test embedded in `summary.v3`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Sut {
    /// Top-level provenance label: typically `"declared"`, or `"mixed"` when
    /// `sut init --probe` filled local fields (see `field_provenance`).
    #[serde(default = "declared")]
    pub provenance: String,
    pub name: Option<String>,
    pub vendor: Option<String>,
    pub gpu: Option<SutGpu>,
    pub cpu: Option<String>,
    pub memory_gb: Option<u64>,
    pub driver_version: Option<String>,
    pub runtime: Option<SutRuntime>,
    pub model: Option<SutModel>,
    pub host_os: Option<String>,
    pub notes: Option<String>,
    /// Optional declared cost inputs (not measured).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<SutCost>,
    /// Per-field provenance (`observed` vs `declared`) when `sut init --probe` fills locals.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub field_provenance: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
}

fn declared() -> String {
    "declared".into()
}

fn observed() -> String {
    "observed".into()
}

/// Write a SUT JSON template, optionally probing **local** host facts.
///
/// Probing never SSHs to the serving host. Without NVIDIA tooling, GPU fields
/// stay as declared placeholders and a warning is printed.
pub fn init_sut(path: &Path, probe: bool, force: bool) -> anyhow::Result<SutInitResult> {
    if path.exists() && !force {
        anyhow::bail!(
            "sut init: {} already exists (pass --force to overwrite)",
            path.display()
        );
    }
    let mut warnings = Vec::new();
    let sut = if probe {
        probe_local_sut(&mut warnings)?
    } else {
        template_sut()
    };
    let json = serde_json::to_string_pretty(&sut)?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, format!("{json}\n"))
        .map_err(|e| anyhow::anyhow!("sut init: failed to write {}: {e}", path.display()))?;
    Ok(SutInitResult {
        path: path.to_path_buf(),
        sut,
        probed: probe,
        warnings,
    })
}

/// Result of [`init_sut`].
#[derive(Debug)]
pub struct SutInitResult {
    pub path: std::path::PathBuf,
    pub sut: Sut,
    pub probed: bool,
    pub warnings: Vec<String>,
}

/// Operator-editable declared template (same shape as `examples/sut.example.json`).
pub fn template_sut() -> Sut {
    Sut {
        provenance: declared(),
        name: Some("example-host / GPU x1".into()),
        vendor: Some("Example OEM".into()),
        gpu: Some(SutGpu {
            model: Some("L40S".into()),
            count: Some(1),
            memory_gb: Some(48),
        }),
        cpu: Some("Example CPU".into()),
        memory_gb: Some(256),
        driver_version: Some("NVIDIA 580.xx".into()),
        runtime: Some(SutRuntime {
            name: Some("vllm".into()),
            version: Some("latest".into()),
            config: Some(
                "vllm serve example/model --tensor-parallel-size 1 --dtype auto (replace with exact launch)"
                    .into(),
            ),
        }),
        model: Some(SutModel {
            id: Some("example/model".into()),
            revision: None,
            quantization: None,
            kv_dtype: None,
            kernel_evidence: None,
            quality_evidence: None,
        }),
        host_os: Some("Ubuntu 22.04".into()),
        notes: Some(
            "Template from `sut init`. Replace placeholders. Remote serving host is not probed."
                .into(),
        ),
        cost: Some(SutCost {
            price_per_hour: Some(3.6),
            currency: Some("USD".into()),
        }),
        field_provenance: BTreeMap::new(),
        extra: BTreeMap::new(),
    }
}

/// Probe local OS/CPU/memory and optional nvidia-smi GPU facts.
pub fn probe_local_sut(warnings: &mut Vec<String>) -> anyhow::Result<Sut> {
    let mut field_provenance = BTreeMap::new();
    let host_os = probe_host_os();
    if let Some(ref os) = host_os {
        field_provenance.insert("host_os".into(), observed());
        let _ = os;
    } else {
        warnings.push("sut init --probe: could not read host OS (/etc/os-release)".into());
    }
    let cpu = probe_cpu();
    if cpu.is_some() {
        field_provenance.insert("cpu".into(), observed());
    } else {
        warnings.push("sut init --probe: could not read CPU model".into());
    }
    let memory_gb = probe_memory_gb();
    if memory_gb.is_some() {
        field_provenance.insert("memory_gb".into(), observed());
    } else {
        warnings.push("sut init --probe: could not read host memory".into());
    }

    let (gpu, driver_version, gpu_warnings) = probe_nvidia();
    warnings.extend(gpu_warnings);
    if gpu.is_some() {
        field_provenance.insert("gpu".into(), observed());
    }
    if driver_version.is_some() {
        field_provenance.insert("driver_version".into(), observed());
    }

    let name = match &gpu {
        Some(g) => {
            let model = g.model.as_deref().unwrap_or("GPU");
            let count = g.count.unwrap_or(1);
            Some(format!("local-host / {model} x{count}"))
        }
        None => Some("local-host".into()),
    };
    field_provenance.insert("name".into(), observed());

    let provenance = if field_provenance.values().any(|v| v == "observed") {
        "mixed".into()
    } else {
        declared()
    };

    Ok(Sut {
        provenance,
        name,
        vendor: None,
        gpu: gpu.or(Some(SutGpu {
            model: Some("REPLACE_ME".into()),
            count: Some(1),
            memory_gb: None,
        })),
        cpu: cpu.or(Some("REPLACE_ME".into())),
        memory_gb,
        driver_version,
        runtime: Some(SutRuntime {
            name: Some("REPLACE_ME".into()),
            version: None,
            config: Some("Fill vendor Docker launch args after web search".into()),
        }),
        model: Some(SutModel {
            id: Some("REPLACE_ME".into()),
            revision: None,
            quantization: None,
            kv_dtype: None,
            kernel_evidence: None,
            quality_evidence: None,
        }),
        host_os: host_os.or(Some(std::env::consts::OS.into())),
        notes: Some(
            "Generated by `sut init --probe`. Observed fields are local client-host facts only; runtime/model/vendor remain operator-declared. This does not verify the remote serving host."
                .into(),
        ),
        cost: None,
        field_provenance,
        extra: BTreeMap::new(),
    })
}

fn probe_host_os() -> Option<String> {
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    Some(std::env::consts::OS.into())
}

fn probe_cpu() -> Option<String> {
    if let Ok(text) = std::fs::read_to_string("/proc/cpuinfo") {
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("model name") {
                let name = value.trim().trim_start_matches(':').trim();
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

fn probe_memory_gb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some((kb / 1024 / 1024).max(1));
        }
    }
    None
}

fn probe_nvidia() -> (Option<SutGpu>, Option<String>, Vec<String>) {
    let mut warnings = Vec::new();
    let output = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total,driver_version",
            "--format=csv,noheader,nounits",
        ])
        .output();
    let output = match output {
        Ok(out) if out.status.success() => out,
        Ok(out) => {
            warnings.push(format!(
                "sut init --probe: nvidia-smi exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
            return (None, None, warnings);
        }
        Err(err) => {
            warnings.push(format!(
                "sut init --probe: nvidia-smi not available ({err}); GPU fields left as placeholders"
            ));
            return (None, None, warnings);
        }
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut models = Vec::new();
    let mut mem_gb = Vec::new();
    let mut drivers = Vec::new();
    for line in stdout.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let parts: Vec<_> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() < 3 {
            continue;
        }
        models.push(parts[0].to_string());
        if let Ok(mib) = parts[1].parse::<f64>() {
            mem_gb.push((mib / 1024.0).round() as u64);
        }
        drivers.push(parts[2].to_string());
    }
    if models.is_empty() {
        warnings.push("sut init --probe: nvidia-smi returned no GPU rows".into());
        return (None, None, warnings);
    }
    let model = if models.iter().all(|m| m == &models[0]) {
        models[0].clone()
    } else {
        models.join(" / ")
    };
    let memory_gb = mem_gb.first().copied();
    let driver_version = drivers.first().cloned().map(|d| format!("NVIDIA {d}"));
    (
        Some(SutGpu {
            model: Some(model),
            count: Some(models.len() as u32),
            memory_gb,
        }),
        driver_version,
        warnings,
    )
}

/// Declared monetary inputs for cost-per-token reporting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SutCost {
    /// Hourly platform/GPU cost in the declared currency (default assumption: USD).
    pub price_per_hour: Option<f64>,
    /// Currency code; omitted means USD for documentation purposes only (no FX).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SutGpu {
    pub model: Option<String>,
    pub count: Option<u32>,
    pub memory_gb: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SutRuntime {
    pub name: Option<String>,
    pub version: Option<String>,
    pub config: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SutModel {
    pub id: Option<String>,
    pub revision: Option<String>,
    pub quantization: Option<String>,
    /// KV-cache dtype actually served (e.g. "fp8_e4m3", "bf16"). Optional;
    /// required under `--sut-profile strict` (#308).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kv_dtype: Option<String>,
    /// Declared kernel and CUDA-graph evidence. Never inferred by the CLI
    /// from the model name (#308): an operator declares it from serving
    /// logs/config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel_evidence: Option<SutKernelEvidence>,
    /// Linked external quality-evidence artifact. Quality results stay
    /// separate from throughput (#308); this block only links them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_evidence: Option<SutQualityEvidence>,
}

/// Declared kernel and CUDA-graph evidence for the served checkpoint (#308).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SutKernelEvidence {
    /// Kernel path actually used (e.g. "FlashInferCutlassNvFp4LinearKernel").
    pub kernel: Option<String>,
    /// CUDA graph mode actually used (e.g. "piecewise", "full", "none").
    pub graph_mode: Option<String>,
    /// How the operator knows: "serving startup log", "launch flag", ...
    pub source: Option<String>,
    /// Optional sha256 (hex) of the log excerpt or config proving it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_sha256: Option<String>,
}

/// Link to external quality evidence (accuracy/quality run), with the
/// statistical surface needed to read it honestly (#308).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SutQualityEvidence {
    /// Link to the quality artifact (release URL, object store, or path).
    pub uri: Option<String>,
    /// sha256 (hex) of the referenced artifact.
    pub sha256: Option<String>,
    /// Sample size of the quality run.
    pub n: Option<u64>,
    /// Error/failure count within the quality run (n >= errors).
    pub errors: Option<u64>,
    /// Confidence-interval methodology label (e.g. "wilson95").
    pub confidence: Option<String>,
}

/// SUT validation profile (#308). `standard` is the `--require-sut`
/// publication baseline; `strict` additionally requires the exact model
/// revision, weight/activation quantization, KV dtype, declared
/// kernel/graph evidence, and a linked quality-evidence artifact, and
/// refuses to infer native kernel choice from the model name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum SutProfile {
    #[default]
    Standard,
    Strict,
}

/// Load a SUT declaration from JSON (`.json`) or YAML (`.yaml`/`.yml`).
pub fn load_sut(path: &Path) -> anyhow::Result<Sut> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("--sut: failed to read {}: {e}", path.display()))?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let sut: Sut = match ext.as_str() {
        "json" => serde_json::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("--sut: invalid JSON in {}: {e}", path.display()))?,
        "yaml" | "yml" => serde_yaml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("--sut: invalid YAML in {}: {e}", path.display()))?,
        _ => anyhow::bail!(
            "--sut: unsupported extension for {} (use .json, .yaml, or .yml)",
            path.display()
        ),
    };
    Ok(sut)
}

/// Validate that a SUT declaration carries the inventory required for publication.
///
/// Called only when `--require-sut` is set. Plain `--sut` remains permissive.
pub fn validate_publishable_sut(sut: &Sut) -> anyhow::Result<()> {
    let mut missing: Vec<&str> = Vec::new();
    match sut.gpu.as_ref() {
        None => missing.push("gpu"),
        Some(gpu) => {
            if !nonblank(gpu.model.as_deref()) {
                missing.push("gpu.model");
            }
            match gpu.count {
                Some(c) if c > 0 => {}
                _ => missing.push("gpu.count"),
            }
        }
    }
    if !nonblank(sut.driver_version.as_deref()) {
        missing.push("driver_version");
    }
    match sut.runtime.as_ref() {
        None => missing.push("runtime"),
        Some(rt) => {
            if !nonblank(rt.name.as_deref()) {
                missing.push("runtime.name");
            }
            if !nonblank(rt.version.as_deref()) {
                missing.push("runtime.version");
            }
            if !nonblank(rt.config.as_deref()) {
                missing.push("runtime.config");
            }
        }
    }
    if !nonblank(sut.host_os.as_deref()) {
        missing.push("host_os");
    }
    if missing.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "--require-sut: incomplete SUT declaration: missing {}",
        missing.join(", ")
    )
}

fn nonblank(value: Option<&str>) -> bool {
    value.map(str::trim).is_some_and(|s| !s.is_empty())
}

fn valid_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
}

/// Validate the strict publication profile (#308). Caller first applies the
/// publishable baseline; strict adds the exact model revision, the
/// weight/activation quantization string, the KV dtype, the declared
/// kernel/graph evidence, and the linked quality-evidence artifact. Model
/// names never imply a native kernel choice: strict refuses to run without
/// operator-declared evidence.
pub fn validate_strict_sut(sut: &Sut) -> anyhow::Result<()> {
    let mut missing: Vec<&str> = Vec::new();
    match sut.model.as_ref() {
        None => missing.push("model"),
        Some(model) => {
            if !nonblank(model.id.as_deref()) {
                missing.push("model.id");
            }
            if !nonblank(model.revision.as_deref()) {
                missing.push("model.revision");
            }
            if !nonblank(model.quantization.as_deref()) {
                missing.push("model.quantization");
            }
            if !nonblank(model.kv_dtype.as_deref()) {
                missing.push("model.kv_dtype");
            }
            match model.kernel_evidence.as_ref() {
                None => missing.push("model.kernel_evidence"),
                Some(kernel) => {
                    if !nonblank(kernel.kernel.as_deref()) {
                        missing.push("model.kernel_evidence.kernel");
                    }
                    if !nonblank(kernel.graph_mode.as_deref()) {
                        missing.push("model.kernel_evidence.graph_mode");
                    }
                    if !nonblank(kernel.source.as_deref()) {
                        missing.push("model.kernel_evidence.source");
                    }
                    if let Some(sha) = kernel.log_sha256.as_deref() {
                        if !valid_sha256_hex(sha) {
                            anyhow::bail!(
                                "--sut-profile strict: model.kernel_evidence.log_sha256 must be 64 hex chars, got {sha:?}"
                            );
                        }
                    }
                }
            }
            match model.quality_evidence.as_ref() {
                None => missing.push("model.quality_evidence"),
                Some(quality) => {
                    if !nonblank(quality.uri.as_deref()) {
                        missing.push("model.quality_evidence.uri");
                    }
                    match quality.sha256.as_deref() {
                        Some(sha) if valid_sha256_hex(sha) => {}
                        Some(sha) => anyhow::bail!(
                            "--sut-profile strict: model.quality_evidence.sha256 must be 64 hex chars, got {sha:?}"
                        ),
                        None => missing.push("model.quality_evidence.sha256"),
                    }
                    match quality.n {
                        Some(n) if n >= 1 => {}
                        _ => missing.push("model.quality_evidence.n"),
                    }
                    match (quality.errors, quality.n) {
                        (Some(errors), Some(n)) if errors <= n => {}
                        _ => missing.push("model.quality_evidence.errors"),
                    }
                    if !nonblank(quality.confidence.as_deref()) {
                        missing.push("model.quality_evidence.confidence");
                    }
                }
            }
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "--sut-profile strict: incomplete strict SUT declaration: missing {} — strict never infers kernel/graph choices from the model name; declare observed serving evidence instead",
        missing.join(", ")
    )
}

/// Resolve SUT flags before any request is sent.
///
/// Returns `(sut, redact_hostname)`. `--require-sut` implies hostname redaction
/// and rejects declarations missing publication inventory fields.
/// When `sut` is absent and not required, prints a one-line stderr notice.
pub fn resolve_sut_flags(
    sut_path: Option<&Path>,
    require_sut: bool,
    redact_hostname: bool,
) -> anyhow::Result<(Option<Sut>, bool)> {
    resolve_sut_profile_flags(sut_path, require_sut, redact_hostname, SutProfile::Standard)
}

/// Like [`resolve_sut_flags`], with the validation profile (#308).
/// `strict` implies the `--require-sut` baseline plus the strict gates.
pub fn resolve_sut_profile_flags(
    sut_path: Option<&Path>,
    require_sut: bool,
    redact_hostname: bool,
    profile: SutProfile,
) -> anyhow::Result<(Option<Sut>, bool)> {
    let require_sut = require_sut || profile == SutProfile::Strict;
    let redact = redact_hostname || require_sut;
    match sut_path {
        Some(path) => {
            let sut = load_sut(path)?;
            if require_sut {
                validate_publishable_sut(&sut)?;
            }
            if profile == SutProfile::Strict {
                validate_strict_sut(&sut)?;
            }
            warn_quantization(&sut);
            Ok((Some(sut), redact))
        }
        None if require_sut => Err(anyhow::anyhow!(
            "--require-sut: --sut <PATH> is required (or set METRUM_AI_BENCH_REQUIRE_SUT=1 with --sut)"
        )),
        None => {
            eprintln!(
                "sut: not provided; result is not self-describing (see README.md#publishing-a-result)"
            );
            Ok((None, redact))
        }
    }
}

/// Warn that quantized/accelerated weights measure speed, not answer quality.
pub fn warn_quantization(sut: &Sut) {
    let Some(q) = sut
        .model
        .as_ref()
        .and_then(|m| m.quantization.as_deref())
        .map(str::trim)
        .filter(|q| !q.is_empty())
    else {
        return;
    };
    eprintln!(
        "sut: model.quantization={q:?}; this run measures performance, not answer quality (see docs/LIMITATIONS.md)"
    );
}

/// True when host is loopback or RFC1918 / link-local (safe for publishable TTFT).
pub fn url_host_is_local(host: &str) -> bool {
    let host = host.trim().trim_matches(|c| c == '[' || c == ']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
            std::net::IpAddr::V6(v6) => {
                v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00 /* ULA */
            }
        };
    }
    false
}

/// Warn when `--url` is not loopback / private (WAN RTT lands in TTFT).
///
/// Silence with `METRUM_AI_BENCH_ALLOW_REMOTE_URL=1`.
pub fn warn_remote_benchmark_url(url: &str) {
    if std::env::var_os("METRUM_AI_BENCH_ALLOW_REMOTE_URL").is_some_and(|v| v == "1") {
        return;
    }
    let Some(host) = host_from_http_url(url) else {
        return;
    };
    if url_host_is_local(host) {
        return;
    }
    eprintln!(
        "url: {host} is not loopback/RFC1918; WAN RTT is included in TTFT. Prefer running the load generator on the serving host (loopback). Set METRUM_AI_BENCH_ALLOW_REMOTE_URL=1 to silence."
    );
}

fn host_from_http_url(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split('/').next()?;
    let hostport = authority.rsplit('@').next()?;
    let host = if hostport.starts_with('[') {
        hostport.trim_start_matches('[').split(']').next()?
    } else {
        hostport.split(':').next()?
    };
    Some(host)
}

/// Warn for every endpoint URL in a resolved list.
pub fn warn_remote_benchmark_urls<'a>(urls: impl IntoIterator<Item = &'a str>) {
    for url in urls {
        warn_remote_benchmark_url(url);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn load_json_and_yaml_parity() {
        let dir = std::env::temp_dir();
        let json_path = dir.join("sut_test.json");
        let yaml_path = dir.join("sut_test.yaml");
        let json = r#"{"name":"box","gpu":{"model":"L40S","count":1},"provenance":"declared"}"#;
        let yaml = "name: box\ngpu:\n  model: L40S\n  count: 1\n";
        std::fs::write(&json_path, json).unwrap();
        std::fs::write(&yaml_path, yaml).unwrap();
        let a = load_sut(&json_path).unwrap();
        let b = load_sut(&yaml_path).unwrap();
        assert_eq!(a.name, b.name);
        assert_eq!(a.gpu, b.gpu);
        assert_eq!(a.provenance, "declared");
    }

    #[test]
    fn unknown_field_errors() {
        let path = std::env::temp_dir().join("sut_unknown.json");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, r#"{{"typo_field":1}}"#).unwrap();
        let err = load_sut(&path).unwrap_err().to_string();
        assert!(
            err.contains("typo_field") || err.contains("unknown"),
            "{err}"
        );
    }

    #[test]
    fn require_sut_without_path_fails() {
        let err = resolve_sut_flags(None, true, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("--require-sut"), "{err}");
    }

    #[test]
    fn url_host_classifies_local_and_remote() {
        assert!(url_host_is_local("127.0.0.1"));
        assert!(url_host_is_local("localhost"));
        assert!(url_host_is_local("10.1.2.3"));
        assert!(url_host_is_local("192.168.1.1"));
        assert!(url_host_is_local("172.16.0.9"));
        assert!(!url_host_is_local("8.8.8.8"));
        assert!(!url_host_is_local("example.com"));
    }

    #[test]
    fn require_sut_implies_redact() {
        let path = std::env::temp_dir().join("sut_req.json");
        let complete = r#"{
            "name":"n",
            "gpu":{"model":"L40S","count":1},
            "driver_version":"NVIDIA 580.xx",
            "runtime":{"name":"vllm","version":"0.10.0","config":"vllm serve example --tensor-parallel-size 1"},
            "host_os":"Ubuntu 22.04"
        }"#;
        std::fs::write(&path, complete).unwrap();
        let (_, redact) = resolve_sut_flags(Some(&path), true, false).unwrap();
        assert!(redact);
    }

    #[test]
    fn require_sut_rejects_incomplete_declaration() {
        let path = std::env::temp_dir().join("sut_incomplete.json");
        std::fs::write(&path, r#"{}"#).unwrap();
        let err = resolve_sut_flags(Some(&path), true, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("--require-sut"), "{err}");
        assert!(err.contains("gpu"), "{err}");
        assert!(err.contains("driver_version"), "{err}");
        assert!(err.contains("runtime"), "{err}");
        assert!(err.contains("host_os"), "{err}");
        // Without --require-sut, incomplete declarations still load.
        let (sut, _) = resolve_sut_flags(Some(&path), false, false).unwrap();
        assert!(sut.is_some());
    }

    #[test]
    fn require_sut_rejects_blank_and_zero_count() {
        let path = std::env::temp_dir().join("sut_blank.json");
        std::fs::write(
            &path,
            r#"{
                "gpu":{"model":"  ","count":0},
                "driver_version":"",
                "runtime":{"name":"vllm","version":" ","config":""},
                "host_os":"   "
            }"#,
        )
        .unwrap();
        let err = resolve_sut_flags(Some(&path), true, false)
            .unwrap_err()
            .to_string();
        for field in [
            "gpu.model",
            "gpu.count",
            "driver_version",
            "runtime.version",
            "runtime.config",
            "host_os",
        ] {
            assert!(err.contains(field), "missing {field} in {err}");
        }
    }

    #[test]
    fn template_satisfies_publishable_validation() {
        validate_publishable_sut(&template_sut()).unwrap();
    }

    #[test]
    fn load_cost_block() {
        let path = std::env::temp_dir().join("sut_cost.json");
        std::fs::write(
            &path,
            r#"{"name":"box","cost":{"price_per_hour":3.6,"currency":"USD"}}"#,
        )
        .unwrap();
        let sut = load_sut(&path).unwrap();
        assert_eq!(sut.cost.as_ref().unwrap().price_per_hour, Some(3.6));
        assert_eq!(sut.cost.as_ref().unwrap().currency.as_deref(), Some("USD"));
    }

    #[test]
    fn init_template_is_loadable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sut.json");
        let result = init_sut(&path, false, false).unwrap();
        assert!(!result.probed);
        assert_eq!(result.sut.provenance, "declared");
        let loaded = load_sut(&path).unwrap();
        assert_eq!(loaded.name, result.sut.name);
    }

    #[test]
    fn init_probe_marks_field_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("probed.json");
        let result = init_sut(&path, true, false).unwrap();
        assert!(result.probed);
        assert!(
            result.sut.provenance == "mixed" || result.sut.provenance == "declared",
            "provenance={}",
            result.sut.provenance
        );
        assert!(
            !result.sut.field_provenance.is_empty() || !result.warnings.is_empty(),
            "expected observed fields or warnings"
        );
        let _ = load_sut(&path).unwrap();
    }

    #[test]
    fn init_refuses_overwrite_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sut.json");
        init_sut(&path, false, false).unwrap();
        let err = init_sut(&path, false, false).unwrap_err().to_string();
        assert!(err.contains("--force"), "{err}");
    }
}

#[cfg(test)]
mod strict_tests {
    use super::*;

    /// A complete strict SUT declaration (all strict fields populated).
    fn strict_sut_json() -> serde_json::Value {
        serde_json::json!({
            "provenance": "declared",
            "gpu": {"model": "H100", "count": 1, "memory_gb": 80},
            "driver_version": "580.126.09",
            "runtime": {"name": "vllm", "version": "0.32.0", "config": "vllm serve m --flags"},
            "host_os": "Linux",
            "model": {
                "id": "vendor/model",
                "revision": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
                "quantization": "NVFP4 W4A4",
                "kv_dtype": "fp8_e4m3",
                "kernel_evidence": {
                    "kernel": "FlashInferCutlassNvFp4LinearKernel",
                    "graph_mode": "piecewise",
                    "source": "serving startup log"
                },
                "quality_evidence": {
                    "uri": "https://example.invalid/quality.json",
                    "sha256": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
                    "n": 100,
                    "errors": 2,
                    "confidence": "wilson95"
                }
            }
        })
    }

    fn strict_sut() -> Sut {
        serde_json::from_value(strict_sut_json()).expect("strict example parses")
    }

    fn strict_err(mut json: serde_json::Value) -> String {
        let sut: Sut = serde_json::from_value(std::mem::take(&mut json)).expect("parses");
        validate_publishable_sut(&sut).expect("baseline passes");
        validate_strict_sut(&sut)
            .expect_err("strict must reject")
            .to_string()
    }

    #[test]
    fn strict_accepts_complete_declaration() {
        let sut = strict_sut();
        validate_publishable_sut(&sut).expect("baseline passes");
        validate_strict_sut(&sut).expect("strict passes");
    }

    #[test]
    fn strict_rejects_missing_kernel_and_graph_evidence() {
        let mut j = strict_sut_json();
        j.pointer_mut("/model")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("kernel_evidence");
        let err = strict_err(j);
        assert!(err.contains("model.kernel_evidence"), "{err}");
        assert!(
            err.contains("never infers kernel/graph choices from the model name"),
            "strict must state the no-inference rule: {err}"
        );
    }

    #[test]
    fn strict_rejects_missing_revision_quantization_and_kv_dtype() {
        for key in ["revision", "quantization", "kv_dtype"] {
            let mut j = strict_sut_json();
            j.pointer_mut("/model")
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(key);
            let err = strict_err(j);
            assert!(err.contains(&format!("model.{key}")), "{key}: {err}");
        }
    }

    #[test]
    fn strict_rejects_missing_quality_evidence() {
        let mut j = strict_sut_json();
        j.pointer_mut("/model")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("quality_evidence");
        let err = strict_err(j);
        assert!(err.contains("model.quality_evidence"), "{err}");
    }

    #[test]
    fn strict_rejects_malformed_quality_hash() {
        let mut j = strict_sut_json();
        j.pointer_mut("/model/quality_evidence")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("sha256".into(), serde_json::json!("not-hex"));
        let sut: Sut = serde_json::from_value(j).expect("parses");
        validate_publishable_sut(&sut).expect("baseline");
        let err = validate_strict_sut(&sut)
            .expect_err("bad hash must fail")
            .to_string();
        assert!(err.contains("sha256"), "{err}");
    }

    #[test]
    fn strict_rejects_errors_exceeding_n() {
        let mut j = strict_sut_json();
        j.pointer_mut("/model/quality_evidence")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("errors".into(), serde_json::json!(500));
        let err = strict_err(j);
        assert!(err.contains("quality_evidence.errors"), "{err}");
    }

    #[test]
    fn standard_profile_never_requires_strict_fields() {
        // Baseline alone: no model block at all is fine for standard runs.
        let sut = serde_json::from_value::<Sut>(serde_json::json!({
            "provenance": "declared",
            "gpu": {"model": "H100", "count": 1, "memory_gb": 80},
            "driver_version": "580.126.09",
            "runtime": {"name": "vllm", "version": "0.32.0", "config": "vllm serve m"},
            "host_os": "Linux"
        }))
        .expect("parses");
        validate_publishable_sut(&sut).expect("baseline passes without model evidence");
    }

    #[test]
    fn shipped_strict_example_passes_strict_validation() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("examples/sut.strict.example.json");
        let sut = load_sut(&path).expect("strict example loads");
        validate_publishable_sut(&sut).expect("baseline");
        validate_strict_sut(&sut).expect("shipped strict example passes strict validation");
    }
}
