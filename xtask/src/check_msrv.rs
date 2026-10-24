// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Debug, Deserialize)]
struct Package {
    name: String,
    version: String,
    rust_version: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RustVersion(u64, u64, u64);

pub fn run(msrv: &str) -> Result<()> {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--locked",
            "--format-version",
            "1",
            "--all-features",
        ])
        .output()
        .context("run cargo metadata for the locked dependency set")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let metadata =
        String::from_utf8(output.stdout).context("cargo metadata output was not UTF-8")?;
    check_metadata(&metadata, msrv)
}

fn check_metadata(metadata_json: &str, msrv: &str) -> Result<()> {
    let supported = parse_version(msrv).with_context(|| format!("invalid MSRV {msrv:?}"))?;
    let metadata: Metadata = serde_json::from_str(metadata_json).context("parse cargo metadata")?;

    let mut offenders = Vec::new();
    for package in &metadata.packages {
        let Some(required) = &package.rust_version else {
            continue;
        };
        let required_version = parse_version(required).with_context(|| {
            format!(
                "unsupported rust-version for {} {}: {required:?}",
                package.name, package.version
            )
        })?;
        if required_version > supported {
            offenders.push(format!(
                "{} {} requires Rust {}",
                package.name, package.version, required
            ));
        }
    }

    if !offenders.is_empty() {
        offenders.sort();
        bail!(
            "locked packages exceed Rust {msrv}:\n  {}",
            offenders.join("\n  ")
        );
    }

    println!(
        "locked MSRV check passed: all {} packages support Rust {msrv}",
        metadata.packages.len()
    );
    Ok(())
}

fn parse_version(value: &str) -> Result<RustVersion> {
    let parts = value.split('.').collect::<Vec<_>>();
    if !(2..=3).contains(&parts.len()) {
        bail!("expected major.minor or major.minor.patch version");
    }

    let parse_part = |part: &str| {
        part.parse::<u64>()
            .context("version components must be non-negative integers")
    };
    let major = parse_part(parts[0])?;
    let minor = parse_part(parts[1])?;
    let patch = if parts.len() == 3 {
        parse_part(parts[2])?
    } else {
        0
    };
    Ok(RustVersion(major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::check_metadata;

    const UUID_METADATA: &str = r#"{
        "packages": [{
            "name": "uuid",
            "version": "1.28.0",
            "rust_version": "1.89.0"
        }]
    }"#;

    #[test]
    fn locked_package_msrv_accepts_rust_189() {
        assert!(check_metadata(UUID_METADATA, "1.89.0").is_ok());
    }

    #[test]
    fn locked_package_msrv_rejects_rust_188_with_offender() {
        let error = check_metadata(UUID_METADATA, "1.88.0").unwrap_err();
        assert!(error
            .to_string()
            .contains("uuid 1.28.0 requires Rust 1.89.0"));
    }
}
