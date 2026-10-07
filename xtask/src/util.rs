// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Repository root (directory containing the workspace Cargo.toml with metrum-ai-bench-cli).
pub fn repo_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir().context("current_dir")?;
    loop {
        let cargo = dir.join("Cargo.toml");
        if cargo.is_file() {
            let text =
                std::fs::read_to_string(&cargo).with_context(|| format!("read {cargo:?}"))?;
            if text.contains("name = \"metrum-ai-bench-cli\"") {
                return Ok(dir);
            }
        }
        if !dir.pop() {
            bail!("could not find bench-cli repo root from cwd");
        }
    }
}

pub fn run_git(args: &[&str], cwd: &Path) -> Result<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .context("spawn git")?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Walk directory tree; call `f` for each regular file. Skip `target`, `target-e2e`, `.git`.
pub fn walk_files(root: &Path, mut f: impl FnMut(&Path) -> Result<()>) -> Result<()> {
    fn walk(dir: &Path, f: &mut dyn FnMut(&Path) -> Result<()>) -> Result<()> {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e).with_context(|| format!("read_dir {dir:?}")),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "target" || name == "target-e2e" || name == ".git" {
                continue;
            }
            let ft = entry.file_type()?;
            if ft.is_dir() {
                walk(&path, f)?;
            } else if ft.is_file() {
                f(&path)?;
            }
        }
        Ok(())
    }
    walk(root, &mut f)
}
