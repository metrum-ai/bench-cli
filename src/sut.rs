// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Operator-declared system under test. The client cannot observe the server,
//! so this block is declared, not measured, and is labelled as such in output.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Operator-declared system under test embedded in `summary.v3`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Sut {
    /// Always "declared". Present so a reader never mistakes this for observed data.
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
}

fn declared() -> String {
    "declared".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SutGpu {
    pub model: Option<String>,
    pub count: Option<u32>,
    pub memory_gb: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SutRuntime {
    pub name: Option<String>,
    pub version: Option<String>,
    pub config: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SutModel {
    pub id: Option<String>,
    pub revision: Option<String>,
    pub quantization: Option<String>,
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

/// Resolve SUT flags before any request is sent.
///
/// Returns `(sut, redact_hostname)`. `--require-sut` implies hostname redaction.
/// When `sut` is absent and not required, prints a one-line stderr notice.
pub fn resolve_sut_flags(
    sut_path: Option<&Path>,
    require_sut: bool,
    redact_hostname: bool,
) -> anyhow::Result<(Option<Sut>, bool)> {
    let redact = redact_hostname || require_sut;
    match sut_path {
        Some(path) => {
            let sut = load_sut(path)?;
            Ok((Some(sut), redact))
        }
        None if require_sut => Err(anyhow::anyhow!(
            "--require-sut: --sut <PATH> is required (or set METRUM_AI_BENCH_REQUIRE_SUT=1 with --sut)"
        )),
        None => {
            eprintln!(
                "sut: not provided; result is not self-describing (see docs/RESULTS_PUBLICATION_POLICY.md)"
            );
            Ok((None, redact))
        }
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
    fn require_sut_implies_redact() {
        let path = std::env::temp_dir().join("sut_req.json");
        std::fs::write(&path, r#"{"name":"n"}"#).unwrap();
        let (_, redact) = resolve_sut_flags(Some(&path), true, false).unwrap();
        assert!(redact);
    }

    #[test]
    fn absent_sut_ok_without_require() {
        let (sut, redact) = resolve_sut_flags(None, false, false).unwrap();
        assert!(sut.is_none());
        assert!(!redact);
    }
}
