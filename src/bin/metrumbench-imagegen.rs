// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Deprecated shim binary (kept through v1.x; removed in v2.0). Prefer `metrum-ai-bench-imagegen`.

use std::process::Command;

fn main() {
    // Always emit the v2.0 removal notice (including --help).
    eprintln!("notice: `metrumbench-imagegen` shim will be removed in metrumbench v2.0");
    eprintln!(
        "warning: `metrumbench-imagegen` is deprecated and will be removed in v2.0; use `metrum-ai-bench-imagegen` instead"
    );
    let mut sibling = std::env::current_exe().unwrap_or_else(|e| {
        eprintln!("failed to resolve current executable: {e}");
        std::process::exit(127);
    });
    sibling.set_file_name("metrum-ai-bench-imagegen");
    let status = Command::new(&sibling)
        .args(std::env::args_os().skip(1))
        .status();
    match status {
        Ok(s) => std::process::exit(s.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("failed to launch {}: {e}", sibling.display());
            std::process::exit(127);
        }
    }
}
