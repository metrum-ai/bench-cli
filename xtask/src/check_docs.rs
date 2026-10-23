// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use clap::Subcommand;
use regex::Regex;
use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

#[derive(Subcommand)]
pub enum CheckDocsCommand {
    /// Check relative Markdown links.
    Links {
        #[arg(long)]
        strict: bool,
        files: Vec<PathBuf>,
    },
    /// Run tagged documentation shell blocks.
    Commands { files: Vec<PathBuf> },
}

const DOCS: &[&str] = &[
    "README.md",
    "AGENTS.md",
    "llms.txt",
    "PROPOSALS.md",
    "CHANGELOG.md",
    "docs/SERVING.md",
    "docs/TELEMETRY.md",
    "docs/INTAKE.md",
    "docs/AGENT_RUNBOOK.md",
    "docs/PITFALLS.md",
    "docs/PROMPT_LIBRARY.md",
    "docs/STRATEGIC_BENCHMARKING.md",
];
const PENDING: &[&str] = &[
    "AGENTS.md",
    "llms.txt",
    "docs/INTAKE.md",
    "docs/AGENT_RUNBOOK.md",
    "docs/PITFALLS.md",
    "examples/agent/host_bootstrap.sh",
    "examples/agent/build_prompt_pool.py",
    "examples/agent/length_check.py",
    "examples/agent/telemetry.yaml.example",
    "examples/agent/intake.example.yaml",
];

pub fn run(root: &Path, command: CheckDocsCommand) -> Result<()> {
    match command {
        CheckDocsCommand::Links { strict, files } => links(root, strict, files).map(|_| ()),
        CheckDocsCommand::Commands { files } => commands(root, files).map(|_| ()),
    }
}
fn links(root: &Path, strict: bool, files: Vec<PathBuf>) -> Result<usize> {
    let files = if files.is_empty() {
        DOCS.iter().map(PathBuf::from).collect()
    } else {
        files
    };
    let re = Regex::new(r#"\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)"#)?;
    let mut status = false;
    let mut checked = 0;
    let mut pending = 0;
    for file in files {
        let path = root.join(&file);
        if !path.is_file() {
            println!("check_doc_links: skip missing {}", file.display());
            continue;
        }
        let text = fs::read_to_string(&path)?;
        for cap in re.captures_iter(&text) {
            let target = &cap[1];
            if ["http://", "https://", "mailto:", "data:"]
                .iter()
                .any(|p| target.starts_with(p))
                || target.starts_with('#')
            {
                continue;
            }
            let clean = target
                .split(['#', '?'])
                .next()
                .unwrap_or("")
                .replace("%20", " ");
            if clean.is_empty() {
                continue;
            }
            checked += 1;
            let resolved = if clean.starts_with('/') {
                root.join(clean.trim_start_matches('/'))
            } else {
                path.parent().unwrap_or(root).join(&clean)
            };
            if resolved.exists() {
                continue;
            }
            let rel = resolved
                .strip_prefix(root)
                .unwrap_or(&resolved)
                .to_string_lossy()
                .replace('\\', "/");
            if PENDING.contains(&rel.as_str()) {
                println!(
                    "check_doc_links: pending {}: {} (lands in a sibling commit)",
                    file.display(),
                    target
                );
                pending += 1;
                if strict {
                    status = true;
                }
            } else {
                eprintln!(
                    "check_doc_links: BROKEN {}: {} -> {}",
                    file.display(),
                    target,
                    rel
                );
                status = true;
            }
        }
    }
    println!("check_doc_links: checked {checked}, pending {pending}");
    if status {
        anyhow::bail!("broken documentation links (checked {checked}, pending {pending})");
    }
    Ok(checked)
}
fn commands(root: &Path, files: Vec<PathBuf>) -> Result<usize> {
    let names = if files.is_empty() {
        vec![
            "README.md",
            "AGENTS.md",
            "llms.txt",
            "PROPOSALS.md",
            "docs/INTAKE.md",
            "docs/AGENT_RUNBOOK.md",
            "docs/PITFALLS.md",
            "docs/PROMPT_LIBRARY.md",
            "docs/STRATEGIC_BENCHMARKING.md",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    } else {
        files
    };
    let tag = Regex::new(r"^\s*<!--\s*doc-check\s*(?::\s*(.*?))?\s*-->\s*$")?;
    let fixture = tempfile::tempdir()?;
    let campaign = fixture.path().join("campaign");
    fs::create_dir_all(&campaign)?;
    fs::write(campaign.join("run.ndjson"), "{\"schema_version\":\"telemetry.v1\",\"kind\":\"request\",\"stage\":\"c1\",\"warmup\":false,\"success\":true,\"input_tokens\":8,\"output_tokens\":4}\n{\"schema_version\":\"telemetry.v1\",\"kind\":\"request\",\"stage\":\"c1\",\"warmup\":false,\"success\":true,\"input_tokens\":8,\"output_tokens\":4}\n")?;
    let mut server: Option<Child> = None;
    let mock_url = if let Some(binary) = [
        root.join("target/debug/metrum-ai-bench-cli-mock-server"),
        root.join("target/debug/metrum-ai-bench-cli-mock-server.exe"),
    ]
    .into_iter()
    .find(|p| p.is_file())
    {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);
        server = Some(
            Command::new(binary)
                .args([
                    "--listen",
                    &format!("127.0.0.1:{port}"),
                    "--telemetry-fixture",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
        );
        let url = format!("http://127.0.0.1:{port}");
        for _ in 0..100 {
            if reqwest::blocking::get(format!("{url}/health")).is_ok() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        url
    } else {
        String::new()
    };
    let mut count = 0;
    let mut skipped = 0;
    let mut failed = false;
    for file in names {
        let path = root.join(&file);
        if !path.is_file() {
            eprintln!("check_doc_commands: skip missing {}", file.display());
            continue;
        }
        let text = fs::read_to_string(&path)?;
        let lines: Vec<_> = text.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            let Some(cap) = tag.captures(lines[i]) else {
                i += 1;
                continue;
            };
            let tag_body = cap.get(1).map_or("", |m| m.as_str());
            let mut j = i + 1;
            while j < lines.len() && lines[j].trim().is_empty() {
                j += 1;
            }
            if j >= lines.len() || !lines[j].trim_start().starts_with("```") {
                eprintln!(
                    "check_doc_commands: {}:{}: tag without fenced block",
                    file.display(),
                    i + 1
                );
                failed = true;
                i += 1;
                continue;
            }
            let info = lines[j].trim().trim_start_matches('`').trim();
            let lang = info.split_whitespace().next().unwrap_or("");
            if !["", "bash", "sh"].contains(&lang) {
                eprintln!(
                    "check_doc_commands: {}:{}: unsupported language {lang}",
                    file.display(),
                    j + 1
                );
                failed = true;
                i = j + 1;
                continue;
            }
            let start = j + 1;
            let mut end = start;
            while end < lines.len() && !lines[end].trim_start().starts_with("```") {
                end += 1;
            }
            if end == lines.len() {
                eprintln!(
                    "check_doc_commands: {}:{}: unterminated fence",
                    file.display(),
                    j + 1
                );
                failed = true;
                break;
            }
            if tag_body.contains("requires-gpu") {
                println!(
                    "check_doc_commands: skip {}:{} (requires-gpu)",
                    file.display(),
                    i + 1
                );
                skipped += 1;
            } else {
                println!("check_doc_commands: run {}:{}", file.display(), i + 1);
                let script = lines[start..end].join("\n");
                let result = Command::new("bash")
                    .arg("-euo")
                    .arg("pipefail")
                    .arg("-c")
                    .arg(script)
                    .current_dir(root)
                    .env("REPO_ROOT", root)
                    .env("MOCK_URL", &mock_url)
                    .env("CAMPAIGN", &campaign)
                    .env("ISL_TARGET", "8")
                    .env("ISL_TOLERANCE", "0")
                    .env("OSL_TARGET", "4")
                    .env("OSL_TOLERANCE", "0")
                    .status()
                    .context("running documentation block")?;
                if !result.success() {
                    eprintln!("check_doc_commands: FAILED {}:{}", file.display(), i + 1);
                    failed = true;
                }
                count += 1;
            }
            i = end + 1;
        }
    }
    println!("check_doc_commands: ran {count}, skipped {skipped}");
    if let Some(child) = server.as_mut() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if failed {
        anyhow::bail!("documentation command checks failed");
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn valid_fixture_link_passes_with_a_link_to_check() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("target.md"), "target").unwrap();
        fs::write(d.path().join("doc.md"), "[valid](target.md)").unwrap();
        assert_eq!(links(d.path(), false, vec!["doc.md".into()]).unwrap(), 1);
    }
    #[test]
    fn broken_fixture_link_fails() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("doc.md"), "[bad](missing.md)").unwrap();
        let error = links(d.path(), false, vec!["doc.md".into()]).unwrap_err();
        assert!(error.to_string().contains("checked 1"));
    }
    #[test]
    fn valid_fixture_command_passes() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join("doc.md"),
            "<!-- doc-check -->\n```bash\nprintf 'fixture passed'\n```\n",
        )
        .unwrap();
        assert_eq!(commands(d.path(), vec!["doc.md".into()]).unwrap(), 1);
    }
    #[test]
    fn failing_fixture_command_fails() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join("doc.md"),
            "<!-- doc-check -->\n```bash\nfalse\n```\n",
        )
        .unwrap();
        assert!(commands(d.path(), vec!["doc.md".into()]).is_err());
    }
}
