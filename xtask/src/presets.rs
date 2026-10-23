// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Validate the checked-in campaign preset catalog before it reaches a live run.

use anyhow::{bail, Context, Result};
use serde_yaml::Value;
use std::{fs, path::Path};

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

pub fn run(root: &Path) -> Result<()> {
    let directory = root.join(".claude/skills/campaign/presets");
    for name in REQUIRED {
        let path = directory.join(format!("{name}.yaml"));
        let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        validate(&path, &text)?;
        println!("PASS {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_campaign_catalog_passes_schema_checks() {
        run(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()).unwrap();
    }

    #[test]
    fn malformed_or_unattributed_preset_is_rejected() {
        let path = Path::new("broken.yaml");
        assert!(validate(path, "workloads: [llm]\n").is_err());
        let attributed = "# Copyright (c) 2026 Metrum AI, Inc.\n# SPDX-License-Identifier: Apache-2.0\n# Provenance: test\nworkloads: [typo]\n";
        assert!(validate(path, attributed).is_err());
    }
}
