// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! YAML config for multi-source Prometheus `/metrics` (or `/metric`) scraping.

use anyhow::{bail, Context, Result};
use regex::RegexSet;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use super::parser::engine_include_patterns;

pub const MIN_INTERVAL_MS: u64 = 100;
pub const DEFAULT_INTERVAL_MS: u64 = 1000;
pub const DEFAULT_TIMEOUT_MS: u64 = 800;
pub const DEFAULT_MAX_BODY_BYTES: u64 = 16 * 1024 * 1024;
pub const DEFAULT_REQUIRE_FAILURES: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    #[serde(default = "default_interval")]
    pub default_interval_ms: u64,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_max_body")]
    pub max_body_bytes: u64,
    #[serde(default)]
    pub sources: Vec<TelemetrySource>,
}

fn default_interval() -> u64 {
    DEFAULT_INTERVAL_MS
}
fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT_MS
}
fn default_max_body() -> u64 {
    DEFAULT_MAX_BODY_BYTES
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitScale {
    #[serde(default = "one")]
    pub scale: f64,
    pub unit: String,
}

fn one() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetrySource {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub interval_ms: Option<u64>,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub units: BTreeMap<String, UnitScale>,
    /// Environment variable holding a bearer token.
    #[serde(default)]
    pub bearer_env: Option<String>,
    /// Environment variable holding `user:pass` for basic auth.
    #[serde(default)]
    pub basic_auth_env: Option<String>,
    #[serde(default)]
    pub insecure_tls: bool,
    /// Allow startup probe to succeed with zero matched series.
    #[serde(default)]
    pub allow_empty: bool,
}

#[derive(Debug, Clone)]
pub struct CompiledSource {
    pub name: String,
    pub url: String,
    pub interval_ms: u64,
    pub include: RegexSet,
    pub units: BTreeMap<String, UnitScale>,
    pub bearer_env: Option<String>,
    pub basic_auth_env: Option<String>,
    pub insecure_tls: bool,
    pub allow_empty: bool,
}

impl TelemetryConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read telemetry config {}", path.display()))?;
        let cfg: Self = serde_yaml::from_str(&text)
            .with_context(|| format!("parse telemetry YAML {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Desugar legacy `--metrics-url` into a single engine source.
    pub fn from_metrics_url(url: &str, interval_ms: u64) -> Self {
        Self {
            default_interval_ms: interval_ms.max(MIN_INTERVAL_MS),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            sources: vec![TelemetrySource {
                name: "engine".into(),
                url: url.to_string(),
                interval_ms: Some(interval_ms.max(MIN_INTERVAL_MS)),
                include: engine_include_patterns(),
                units: BTreeMap::new(),
                bearer_env: None,
                basic_auth_env: None,
                insecure_tls: false,
                allow_empty: false,
            }],
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.sources.is_empty() {
            bail!("telemetry config has no sources");
        }
        if self.timeout_ms == 0 {
            bail!("timeout_ms must be > 0");
        }
        if self.max_body_bytes == 0 {
            bail!("max_body_bytes must be > 0");
        }
        let mut names = std::collections::HashSet::new();
        for src in &self.sources {
            if src.name.is_empty() {
                bail!("telemetry source name must not be empty");
            }
            if !names.insert(src.name.clone()) {
                bail!("duplicate telemetry source name {}", src.name);
            }
            if src.url.is_empty() {
                bail!("telemetry source {} has empty url", src.name);
            }
            if src.include.is_empty() {
                bail!("telemetry source {} has empty include", src.name);
            }
            let interval = src.interval_ms.unwrap_or(self.default_interval_ms);
            if interval < MIN_INTERVAL_MS {
                bail!(
                    "telemetry source {} interval_ms {interval} is below minimum {MIN_INTERVAL_MS}",
                    src.name
                );
            }
            RegexSet::new(&src.include).with_context(|| {
                format!("compile include regexes for telemetry source {}", src.name)
            })?;
        }
        Ok(())
    }

    pub fn compile(&self) -> Result<Vec<CompiledSource>> {
        self.validate()?;
        let mut out = Vec::with_capacity(self.sources.len());
        for src in &self.sources {
            let interval = src
                .interval_ms
                .unwrap_or(self.default_interval_ms)
                .max(MIN_INTERVAL_MS);
            out.push(CompiledSource {
                name: src.name.clone(),
                url: src.url.clone(),
                interval_ms: interval,
                include: RegexSet::new(&src.include)?,
                units: src.units.clone(),
                bearer_env: src.bearer_env.clone(),
                basic_auth_env: src.basic_auth_env.clone(),
                insecure_tls: src.insecure_tls,
                allow_empty: src.allow_empty,
            });
        }
        Ok(out)
    }

    /// Warn when any source is under 250 ms; print expected rows/s.
    pub fn warn_fast_sources(&self) {
        for src in &self.sources {
            let interval = src.interval_ms.unwrap_or(self.default_interval_ms);
            if interval < 250 {
                let rows_per_s = 1000.0 / interval as f64;
                eprintln!(
                    "warning: telemetry source {} interval_ms={interval} (<250); expected ~{rows_per_s:.1} scrapes/s",
                    src.name
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_minimal_yaml() {
        let yaml = r#"
default_interval_ms: 1000
timeout_ms: 800
sources:
  - name: all-smi
    url: http://127.0.0.1:9090/metric
    interval_ms: 500
    include:
      - "^all_smi_(gpu|cpu|memory)_"
"#;
        let cfg: TelemetryConfig = serde_yaml::from_str(yaml).expect("yaml");
        cfg.validate().expect("valid");
        let compiled = cfg.compile().expect("compile");
        assert_eq!(compiled[0].name, "all-smi");
        assert_eq!(compiled[0].interval_ms, 500);
    }

    #[test]
    fn rejects_interval_below_minimum() {
        let cfg = TelemetryConfig {
            default_interval_ms: 1000,
            timeout_ms: 800,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            sources: vec![TelemetrySource {
                name: "x".into(),
                url: "http://127.0.0.1:1/metrics".into(),
                interval_ms: Some(50),
                include: vec!["^a$".into()],
                units: BTreeMap::new(),
                bearer_env: None,
                basic_auth_env: None,
                insecure_tls: false,
                allow_empty: false,
            }],
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn desugars_metrics_url() {
        let cfg = TelemetryConfig::from_metrics_url("http://127.0.0.1:8000/metrics", 250);
        assert_eq!(cfg.sources.len(), 1);
        assert_eq!(cfg.sources[0].name, "engine");
        cfg.validate().expect("desugar valid");
    }
}
