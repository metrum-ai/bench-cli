// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Runtime ISL/OSL validation against optional targets (CLI or mix-report metadata).

use crate::record::{Phase, RequestRecord};
use crate::stats::DistSummary;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

/// Targets and tolerances for comparing measured prompt/completion lengths.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IslOslTargets {
    pub isl_target: Option<f64>,
    pub osl_target: Option<f64>,
    pub isl_tolerance: f64,
    pub osl_tolerance: f64,
    pub source: Option<&'static str>,
}

/// Measured ISL/OSL vs targets with mismatch counts (measure-phase successes).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IslOslValidation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isl_target: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub osl_target: Option<f64>,
    pub isl_tolerance: f64,
    pub osl_tolerance: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<&'static str>,
    pub isl_mean: Option<f64>,
    pub isl_p50: Option<f64>,
    pub osl_mean: Option<f64>,
    pub osl_p50: Option<f64>,
    pub isl_mismatch_count: usize,
    pub osl_mismatch_count: usize,
    pub compared: usize,
}

impl IslOslTargets {
    pub fn is_active(&self) -> bool {
        self.isl_target.is_some() || self.osl_target.is_some()
    }

    /// CLI values win; otherwise fill gaps from a prompt-library mix report.
    pub fn resolve(
        cli_isl: Option<f64>,
        cli_osl: Option<f64>,
        isl_tolerance: f64,
        osl_tolerance: f64,
        mix_report: Option<&Path>,
    ) -> anyhow::Result<Self> {
        let mut targets = Self {
            isl_target: cli_isl,
            osl_target: cli_osl,
            isl_tolerance,
            osl_tolerance,
            source: if cli_isl.is_some() || cli_osl.is_some() {
                Some("cli")
            } else {
                None
            },
        };
        if let Some(path) = mix_report {
            let report = load_mix_report(path)?;
            if targets.isl_target.is_none() {
                targets.isl_target = report.isl_target;
            }
            if targets.osl_target.is_none() {
                targets.osl_target = report.osl_target;
            }
            if targets.isl_tolerance == 0.0 {
                if let Some(tol) = report.isl_tolerance {
                    targets.isl_tolerance = tol;
                }
            }
            if targets.osl_tolerance == 0.0 {
                if let Some(tol) = report.osl_tolerance {
                    targets.osl_tolerance = tol;
                }
            }
            if targets.source.is_none() && targets.is_active() {
                targets.source = Some("mix_report");
            }
        }
        if let Some(v) = targets.isl_target {
            if !v.is_finite() || v < 0.0 {
                anyhow::bail!("--isl-target must be finite and non-negative");
            }
        }
        if let Some(v) = targets.osl_target {
            if !v.is_finite() || v < 0.0 {
                anyhow::bail!("--osl-target must be finite and non-negative");
            }
        }
        if !targets.isl_tolerance.is_finite() || targets.isl_tolerance < 0.0 {
            anyhow::bail!("--isl-tolerance must be finite and non-negative");
        }
        if !targets.osl_tolerance.is_finite() || targets.osl_tolerance < 0.0 {
            anyhow::bail!("--osl-tolerance must be finite and non-negative");
        }
        Ok(targets)
    }
}

struct MixReportTargets {
    isl_target: Option<f64>,
    osl_target: Option<f64>,
    isl_tolerance: Option<f64>,
    osl_tolerance: Option<f64>,
}

fn load_mix_report(path: &Path) -> anyhow::Result<MixReportTargets> {
    let file = std::fs::File::open(path)
        .map_err(|e| anyhow::anyhow!("open --prompt-mix-report {}: {e}", path.display()))?;
    let value: Value = serde_json::from_reader(file)
        .map_err(|e| anyhow::anyhow!("parse --prompt-mix-report {}: {e}", path.display()))?;
    Ok(MixReportTargets {
        isl_target: value.pointer("/isl/target").and_then(Value::as_f64),
        osl_target: value.pointer("/osl/target").and_then(Value::as_f64),
        isl_tolerance: value.pointer("/isl/tolerance").and_then(Value::as_f64),
        osl_tolerance: value.pointer("/osl/tolerance").and_then(Value::as_f64),
    })
}

fn measured_isl(record: &RequestRecord) -> Option<f64> {
    if let Some(tokens) = record.tokenized_prompt_tokens {
        return Some(tokens as f64);
    }
    if record.usage_missing {
        return None;
    }
    Some(record.prompt_tokens as f64)
}

fn measured_osl(record: &RequestRecord) -> Option<f64> {
    if let Some(tokens) = record.tokenized_completion_tokens {
        return Some(tokens as f64);
    }
    if record.usage_missing {
        return None;
    }
    Some(record.completion_tokens as f64)
}

fn outside_tolerance(value: f64, target: f64, tolerance: f64) -> bool {
    (value - target).abs() > tolerance
}

/// Summarize measured ISL/OSL against targets. Returns `None` when no targets are set.
pub fn validate_records(
    records: &[RequestRecord],
    targets: &IslOslTargets,
) -> Option<IslOslValidation> {
    if !targets.is_active() {
        return None;
    }
    let successes: Vec<&RequestRecord> = records
        .iter()
        .filter(|r| r.phase == Phase::Measure && r.is_success())
        .collect();
    summarize_pairs(
        targets,
        &successes
            .iter()
            .filter_map(|r| measured_isl(r))
            .collect::<Vec<_>>(),
        &successes
            .iter()
            .filter_map(|r| measured_osl(r))
            .collect::<Vec<_>>(),
        successes.len(),
        &successes
            .iter()
            .map(|r| (measured_isl(r), measured_osl(r)))
            .collect::<Vec<_>>(),
    )
}

/// Validate strategic stage rows (`input_tokens` / `output_tokens`).
pub fn validate_token_counts(
    rows: &[(u64, u64)],
    targets: &IslOslTargets,
) -> Option<IslOslValidation> {
    if !targets.is_active() {
        return None;
    }
    let isl: Vec<f64> = rows.iter().map(|(i, _)| *i as f64).collect();
    let osl: Vec<f64> = rows.iter().map(|(_, o)| *o as f64).collect();
    let pairs: Vec<(Option<f64>, Option<f64>)> = rows
        .iter()
        .map(|(i, o)| (Some(*i as f64), Some(*o as f64)))
        .collect();
    summarize_pairs(targets, &isl, &osl, rows.len(), &pairs)
}

fn summarize_pairs(
    targets: &IslOslTargets,
    isl_vals: &[f64],
    osl_vals: &[f64],
    compared: usize,
    pairs: &[(Option<f64>, Option<f64>)],
) -> Option<IslOslValidation> {
    let isl_dist = DistSummary::from_values(isl_vals);
    let osl_dist = DistSummary::from_values(osl_vals);
    let mut isl_mismatch_count = 0usize;
    let mut osl_mismatch_count = 0usize;
    for (isl, osl) in pairs {
        if let (Some(target), Some(value)) = (targets.isl_target, *isl) {
            if outside_tolerance(value, target, targets.isl_tolerance) {
                isl_mismatch_count += 1;
            }
        }
        if let (Some(target), Some(value)) = (targets.osl_target, *osl) {
            if outside_tolerance(value, target, targets.osl_tolerance) {
                osl_mismatch_count += 1;
            }
        }
    }
    Some(IslOslValidation {
        isl_target: targets.isl_target,
        osl_target: targets.osl_target,
        isl_tolerance: targets.isl_tolerance,
        osl_tolerance: targets.osl_tolerance,
        source: targets.source,
        isl_mean: isl_dist.avg,
        isl_p50: isl_dist.p50,
        osl_mean: osl_dist.avg,
        osl_p50: osl_dist.p50,
        isl_mismatch_count,
        osl_mismatch_count,
        compared,
    })
}

/// Warn (and optionally fail) when OSL mismatches exceed zero.
pub fn enforce_osl_gate(
    validation: &IslOslValidation,
    fail_on_osl_mismatch: bool,
) -> anyhow::Result<()> {
    if validation.osl_target.is_none() {
        return Ok(());
    }
    if validation.osl_mismatch_count == 0 {
        return Ok(());
    }
    let msg = format!(
        "OSL mismatch: {}/{} measured successes outside target {:?} ± {}",
        validation.osl_mismatch_count,
        validation.compared,
        validation.osl_target,
        validation.osl_tolerance
    );
    if fail_on_osl_mismatch {
        anyhow::bail!("{msg}");
    }
    eprintln!("warning: {msg}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::RequestRecord;
    use chrono::Utc;
    use std::time::Duration;

    fn ok(prompt: u64, completion: u64) -> RequestRecord {
        let started = Utc::now();
        let latency = Duration::from_millis(100);
        RequestRecord::success(
            0,
            Phase::Measure,
            "ep".into(),
            started,
            started + chrono::Duration::from_std(latency).unwrap(),
            latency,
            Some(Duration::from_millis(20)),
            None,
            vec![],
            prompt,
            completion,
            prompt + completion,
        )
    }

    #[test]
    fn counts_osl_mismatches_beyond_tolerance() {
        let targets = IslOslTargets {
            isl_target: Some(100.0),
            osl_target: Some(50.0),
            isl_tolerance: 10.0,
            osl_tolerance: 5.0,
            source: Some("cli"),
        };
        let records = [ok(100, 50), ok(100, 80), ok(200, 50)];
        let v = validate_records(&records, &targets).expect("active");
        assert_eq!(v.osl_mismatch_count, 1);
        assert_eq!(v.isl_mismatch_count, 1);
        assert_eq!(v.compared, 3);
        assert!((v.osl_mean.unwrap() - 60.0).abs() < 1e-12);
    }

    #[test]
    fn inactive_without_targets() {
        assert!(validate_records(&[ok(1, 1)], &IslOslTargets::default()).is_none());
    }

    #[test]
    fn fail_gate_errors_on_mismatch() {
        let v = IslOslValidation {
            isl_target: None,
            osl_target: Some(10.0),
            isl_tolerance: 0.0,
            osl_tolerance: 0.0,
            source: Some("cli"),
            isl_mean: None,
            isl_p50: None,
            osl_mean: Some(20.0),
            osl_p50: Some(20.0),
            isl_mismatch_count: 0,
            osl_mismatch_count: 2,
            compared: 2,
        };
        assert!(enforce_osl_gate(&v, true).is_err());
        assert!(enforce_osl_gate(&v, false).is_ok());
    }
}
