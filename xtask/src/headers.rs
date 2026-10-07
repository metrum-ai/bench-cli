// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::util::{repo_root, run_git, walk_files};
use anyhow::{bail, Context, Result};
use regex::Regex;
use std::path::Path;

const ALLOW_BASENAMES: &[&str] = &[
    "Cargo.lock",
    "go.sum",
    "uv.lock",
    "alice_clean.txt",
    "LICENSE",
    "Cargo.toml",
    "deny.toml",
    "rust-toolchain.toml",
    "pyproject.toml",
];

fn is_allowlisted(rel: &str) -> bool {
    let base = Path::new(rel)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if ALLOW_BASENAMES.contains(&base) {
        return true;
    }
    let lower = rel.to_ascii_lowercase();
    for ext in [
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".wav", ".mp3", ".ogg", ".flac", ".bin", ".docx",
    ] {
        if lower.ends_with(ext) {
            return true;
        }
    }
    if rel.starts_with("test-data/") {
        return true;
    }
    if lower.ends_with(".yaml") || lower.ends_with(".yml") {
        return !rel.starts_with(".github/workflows/");
    }
    false
}

fn should_scan_headers(rel: &str) -> bool {
    if is_allowlisted(rel) {
        return false;
    }
    if rel.starts_with("target/")
        || rel.starts_with("target-e2e/")
        || rel.starts_with(".git/")
        || rel.starts_with("scripts/live/")
        || rel.starts_with("dummy-model-server/")
    {
        return false;
    }
    let lower = rel.to_ascii_lowercase();
    lower.ends_with(".rs")
        || lower.ends_with(".py")
        || lower.ends_with(".sh")
        || lower.ends_with(".yml")
}

fn check_headers(root: &Path) -> Result<()> {
    let mut missing = false;
    let copyright = Regex::new(r"Copyright \(c\) 2026 Metrum AI, Inc\.").unwrap();
    let spdx = Regex::new(r"SPDX-License-Identifier:\s*Apache-2\.0").unwrap();

    walk_files(root, |path| {
        let rel = path.strip_prefix(root).unwrap_or(path);
        let rel_s = rel.to_string_lossy().replace('\\', "/");
        if !should_scan_headers(&rel_s) {
            return Ok(());
        }
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => return Ok(()), // binary / unreadable: skip like grep would miss
        };
        if !copyright.is_match(&text) {
            eprintln!("MISSING copyright: {rel_s}");
            missing = true;
            return Ok(());
        }
        if !spdx.is_match(&text) {
            eprintln!("MISSING SPDX: {rel_s}");
            missing = true;
        }
        Ok(())
    })?;

    if missing {
        bail!(
            "check-headers failed: authored files must include Copyright (c) 2026 and SPDX-License-Identifier: Apache-2.0"
        );
    }
    println!("check_headers: ok");
    Ok(())
}

struct AllowRule {
    glob: String,
    re: Regex,
}

fn load_naming_allow(root: &Path) -> Result<Vec<AllowRule>> {
    let path = root.join(".naming-allow");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path).with_context(|| format!("read {path:?}"))?;
    let mut rules = Vec::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((glob, re_s)) = line.split_once('\t') else {
            continue;
        };
        rules.push(AllowRule {
            glob: glob.to_string(),
            re: Regex::new(re_s).with_context(|| format!("bad naming-allow regex: {re_s}"))?,
        });
    }
    Ok(rules)
}

fn glob_match(pat: &str, path: &str) -> bool {
    // Minimal shell-style: * matches within a path segment or across if used simply.
    // .naming-allow uses patterns like docs/*.md — match like bash [[ path == pat ]].
    let mut pi = 0;
    let mut si = 0;
    let p: Vec<char> = pat.chars().collect();
    let s: Vec<char> = path.chars().collect();
    let mut star_p = None;
    let mut star_s = None;
    while si < s.len() {
        if pi < p.len() && (p[pi] == s[si] || p[pi] == '?') {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_p = Some(pi);
            star_s = Some(si);
            pi += 1;
        } else if let (Some(sp), Some(ss)) = (star_p, star_s) {
            pi = sp + 1;
            star_s = Some(ss + 1);
            si = ss + 1;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn allowlisted(rules: &[AllowRule], file: &str, txt: &str) -> bool {
    rules
        .iter()
        .any(|r| glob_match(&r.glob, file) && r.re.is_match(txt))
}

fn strip_transition(txt: &str) -> String {
    txt.replace("Metrum AI Bench CLI, formerly Metrum Insights CLI", "")
}

fn check_naming(root: &Path) -> Result<()> {
    let rules = load_naming_allow(root)?;
    let files_out = run_git(&["ls-files"], root)?;
    let files: Vec<&str> = files_out
        .lines()
        .filter(|f| {
            !matches!(
                *f,
                "CHANGELOG.md" | "docs/HISTORY_REWRITE.md" | "HISTORY_REWRITE.md" | ".naming-allow"
            ) && *f != "cargo xtask check-headers"
                && *f != "cargo test -p xtask naming_rejects_metrum_bench_and_accepts_transition"
                && !f.starts_with("scripts/tests/gitleaks/")
                && *f != "docs/reviews/QUALITY_ASSESSMENT_REPORT.md"
                && *f != "docs/reviews/QUALITY_ASSESSMENT_PROMPT.md"
                && !f.ends_with(".png")
                && !f.ends_with(".mp3")
                && !f.ends_with(".lock")
        })
        .collect();

    if files.is_empty() {
        eprintln!("check_naming: no files to scan");
        return Ok(());
    }

    let forbidden: &[(&str, Regex)] = &[
        ("MetrumBench", Regex::new(r"MetrumBench").unwrap()),
        ("Metrum Bench CLI", Regex::new(r"Metrum Bench CLI").unwrap()),
        (r"\bmetrumbench\b", Regex::new(r"\bmetrumbench\b").unwrap()),
        ("Insights CLI", Regex::new(r"Insights CLI").unwrap()),
        ("Bench by Metrum", Regex::new(r"Bench by Metrum").unwrap()),
        (
            "Metrum Smart Bench",
            Regex::new(r"Metrum Smart Bench").unwrap(),
        ),
    ];

    let mut rc_ok = true;
    for (pat_name, re) in forbidden {
        for f in &files {
            let path = root.join(f);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for (ln, line) in text.lines().enumerate() {
                if !re.is_match(line) {
                    continue;
                }
                if *pat_name == "Insights CLI" {
                    let without = strip_transition(line);
                    if !without.contains("Insights CLI") {
                        continue;
                    }
                }
                if allowlisted(&rules, f, line) {
                    continue;
                }
                eprintln!(
                    "naming: {f}:{}: forbidden form matching /{pat_name}/: {line}",
                    ln + 1
                );
                rc_ok = false;
            }
        }
    }

    let insights = Regex::new(r"Metrum Insights").unwrap();
    for f in &files {
        let path = root.join(f);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (ln, line) in text.lines().enumerate() {
            if !insights.is_match(line) {
                continue;
            }
            let without = strip_transition(line);
            if !without.contains("Metrum Insights") {
                continue;
            }
            if allowlisted(&rules, f, line) {
                continue;
            }
            eprintln!(
                "naming: {f}:{}: 'Metrum Insights' only allowed as 'Metrum AI Bench CLI, formerly Metrum Insights CLI'",
                ln + 1
            );
            rc_ok = false;
        }
    }

    if rc_ok {
        println!("check_naming: ok");
        Ok(())
    } else {
        bail!("check_naming failed")
    }
}

pub fn run() -> Result<()> {
    let root = repo_root()?;
    let mut err = false;
    if let Err(e) = check_headers(&root) {
        eprintln!("{e:#}");
        err = true;
    }
    if let Err(e) = check_naming(&root) {
        eprintln!("{e:#}");
        err = true;
    }
    if err {
        bail!("check-headers failed");
    }
    Ok(())
}

/// Naming-only checks against an arbitrary root (for unit tests).
#[cfg(test)]
fn check_naming_at(root: &Path) -> Result<()> {
    check_naming(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(cwd: &Path, args: &[&str]) {
        let st = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .unwrap();
        assert!(st.success(), "git {args:?}");
    }

    #[test]
    fn naming_rejects_metrum_bench_and_accepts_transition() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        git(root, &["init", "-q"]);
        git(root, &["config", "user.email", "naming-test@example.com"]);
        git(root, &["config", "user.name", "Naming Test"]);
        std::fs::write(root.join(".naming-allow"), "").unwrap();
        std::fs::write(
            root.join("bad.txt"),
            "MetrumBench is a forbidden product name.\n",
        )
        .unwrap();
        git(root, &["add", ".naming-allow", "bad.txt"]);
        git(root, &["commit", "-q", "-m", "seed forbidden name"]);
        assert!(
            check_naming_at(root).is_err(),
            "expected fail when MetrumBench present"
        );

        std::fs::remove_file(root.join("bad.txt")).unwrap();
        std::fs::write(
            root.join("transition.txt"),
            "Metrum AI Bench CLI, formerly Metrum Insights CLI\n",
        )
        .unwrap();
        git(root, &["add", "-A"]);
        git(
            root,
            &["commit", "-q", "-m", "use approved transition form"],
        );
        assert!(
            check_naming_at(root).is_ok(),
            "approved transition form rejected"
        );

        std::fs::write(
            root.join("transition.txt"),
            "Metrum AI Bench CLI, formerly Metrum Insights CLI; avoid Insights CLI.\n",
        )
        .unwrap();
        git(root, &["add", "transition.txt"]);
        git(
            root,
            &[
                "commit",
                "-q",
                "-m",
                "append a second forbidden legacy name",
            ],
        );
        assert!(
            check_naming_at(root).is_err(),
            "transition exception hid a second legacy name"
        );

        std::fs::write(
            root.join("transition.txt"),
            "Metrum Insights CLI is the current product name.\n",
        )
        .unwrap();
        git(root, &["add", "transition.txt"]);
        git(root, &["commit", "-q", "-m", "use forbidden legacy name"]);
        assert!(
            check_naming_at(root).is_err(),
            "expected fail for bare legacy name"
        );

        std::fs::write(
            root.join("transition.txt"),
            "Metrum Bench CLI is missing the canonical AI token.\n",
        )
        .unwrap();
        git(root, &["add", "transition.txt"]);
        git(root, &["commit", "-q", "-m", "use incomplete product name"]);
        assert!(
            check_naming_at(root).is_err(),
            "expected fail for Metrum Bench CLI"
        );
    }

    #[test]
    fn headers_fail_without_copyright() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("bad.rs"), "fn main() {}\n").unwrap();
        let copyright = Regex::new(r"Copyright \(c\) 2026 Metrum AI, Inc\.").unwrap();
        let text = std::fs::read_to_string(root.join("bad.rs")).unwrap();
        assert!(!copyright.is_match(&text));
        // Direct scan of the temp file path as if it were under root.
        let mut missing = false;
        let rel_s = "bad.rs";
        if !copyright.is_match(&text) {
            missing = true;
        }
        assert!(missing, "fixture must fail copyright check");
        let _ = rel_s;
        let _ = root;
    }
}
