// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Deprecated shim binary (kept through 1.2.x; removed in 1.3.0). Prefer `metrum-ai-bench-cli-vlm`.

use std::process::Command;

fn main() {
    // Always emit the deprecation notice (including --help).
    eprintln!(
        "warning: `metrumbench-vlm` is deprecated and will be removed in 1.3.0; use `metrum-ai-bench-cli-vlm` instead"
    );
    let mut sibling = std::env::current_exe().unwrap_or_else(|e| {
        eprintln!("failed to resolve current executable: {e}");
        std::process::exit(127);
    });
    sibling.set_file_name("metrum-ai-bench-cli-vlm");
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
