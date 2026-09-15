// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Deprecated shim binary. Prefer `metrum-ai-bench-vlm`.

use std::process::Command;

fn main() {
    eprintln!("warning: `metrumbench-vlm` is deprecated; use `metrum-ai-bench-vlm` instead");
    let mut sibling = std::env::current_exe().unwrap_or_else(|e| {
        eprintln!("failed to resolve current executable: {e}");
        std::process::exit(127);
    });
    sibling.set_file_name("metrum-ai-bench-vlm");
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
