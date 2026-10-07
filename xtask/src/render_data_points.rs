// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::util::repo_root;
use anyhow::{bail, Context, Result};
use std::process::Command;

const TEST: &str = "data_points_doc_is_current";

fn run_test(root: &std::path::Path, bless: bool) -> Result<()> {
    let mut cmd = Command::new("cargo");
    cmd.args([
        "test",
        "--locked",
        "--test",
        "data_points",
        "--",
        "--exact",
        TEST,
    ])
    .current_dir(root);
    if bless {
        cmd.env("METRUM_BENCH_BLESS_DATA_POINTS", "1");
    }
    let out = cmd.output().context("cargo test data_points")?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !out.status.success() {
        eprint!("{combined}");
        bail!("data_points test failed");
    }
    if !combined.contains(&format!("test {TEST} ... ok")) {
        eprint!("{combined}");
        bail!("error: {TEST} did not run");
    }
    Ok(())
}

pub fn run(check: bool) -> Result<()> {
    let root = repo_root()?;
    if check {
        run_test(&root, false)?;
        println!("docs/DATA_POINTS.md is current");
    } else {
        run_test(&root, true)?;
        println!("wrote docs/DATA_POINTS.md");
    }
    Ok(())
}
