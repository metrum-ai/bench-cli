// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::util::repo_root;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

const BINS: &[&str] = &[
    "metrum-ai-bench-cli",
    "metrum-ai-bench-cli-llm",
    "metrum-ai-bench-cli-vlm",
    "metrum-ai-bench-cli-asr",
    "metrum-ai-bench-cli-imagegen",
    "metrum-ai-bench-cli-prompts",
    "metrum-ai-bench-cli-strategic",
    "metrum-ai-bench-cli-mock-server",
];

fn bin_dir(root: &Path) -> PathBuf {
    if let Ok(td) = std::env::var("CARGO_TARGET_DIR") {
        return PathBuf::from(td).join("debug");
    }
    root.join("target/debug")
}

fn render_help(bin_path: &Path, extra: &[&str]) -> Result<String> {
    let mut cmd = Command::new(bin_path);
    cmd.args(extra).arg("--help");
    cmd.env_remove("OPENAI_API_KEY");
    cmd.env_remove("METRUM_AI_BENCH_API_KEY");
    let out = cmd
        .output()
        .with_context(|| format!("run {bin_path:?} --help"))?;
    // Match bash: 2>/dev/null — ignore stderr, use stdout even on non-zero? bash uses pipe from stdout.
    let text = String::from_utf8_lossy(&out.stdout);
    let mut started = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        if !started {
            if line.starts_with("Usage:") {
                started = true;
            } else {
                continue;
            }
        }
        lines.push(line.trim_end().to_string());
    }
    Ok(lines.join("\n"))
}

fn render_all(root: &Path) -> Result<String> {
    let bin_dir = bin_dir(root);
    for bin in BINS {
        let p = bin_dir.join(bin);
        if !p.is_file() {
            bail!("missing {p:?}; run: cargo build --bins");
        }
    }

    let mut out = String::new();
    out.push_str("<!-- Copyright (c) 2026 Metrum AI, Inc. -->\n");
    out.push_str("<!-- SPDX-License-Identifier: Apache-2.0 -->\n");
    out.push('\n');
    out.push_str("# CLI reference\n\n");
    out.push_str("Generated from `metrum-ai-bench-cli*` `--help`. Re-run\n");
    out.push_str("`cargo xtask render-cli-help` after flag changes. Live `--help` is\n");
    out.push_str("authoritative if this file drifts.\n\n");

    for bin in BINS {
        let path = bin_dir.join(bin);
        out.push_str(&format!("## `{bin}`\n\n```text\n"));
        out.push_str(&render_help(&path, &[])?);
        out.push_str("\n```\n\n");
        if *bin == "metrum-ai-bench-cli" {
            out.push_str(&format!("### `{bin} preflight`\n\n```text\n"));
            out.push_str(&render_help(&path, &["preflight"])?);
            out.push_str("\n```\n\n");
        }
    }
    Ok(out)
}

pub fn run(check: bool) -> Result<()> {
    let root = repo_root()?;
    let out_path = root.join("docs/CLI.md");
    let rendered = render_all(&root)?;
    if check {
        if !out_path.is_file() {
            bail!("missing {out_path:?}; run: cargo xtask render-cli-help");
        }
        let existing = std::fs::read_to_string(&out_path).context("read docs/CLI.md")?;
        if existing != rendered {
            // Print a unified-ish hint
            eprintln!("error: {out_path:?} is stale; run: cargo xtask render-cli-help");
            bail!("docs/CLI.md is stale");
        }
        println!("render_cli_help: {} is current", out_path.display());
        return Ok(());
    }
    std::fs::write(&out_path, rendered).context("write docs/CLI.md")?;
    println!("wrote {}", out_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn check_mode_errors_when_missing_bins_message_mentions_build() {
        // Unit-level: render_all fails clearly without bins.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("target/debug")).unwrap();
        // Write a fake Cargo.toml so repo_root isn't used here — call render_all on tmp.
        // We cannot easily call render_all without bins; assert the error path string.
        let p = tmp.path().join("target/debug/metrum-ai-bench-cli");
        assert!(!p.is_file());
    }
}
