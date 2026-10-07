// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::util::{repo_root, run_git};
use anyhow::{bail, Result};
use regex::Regex;

pub fn run(base: &str, head: &str) -> Result<()> {
    let root = repo_root()?;
    check_range(&root, base, head)
}

pub fn check_range(root: &std::path::Path, base: &str, head: &str) -> Result<()> {
    let re = Regex::new(r"^Signed-off-by: .+ <.+@.+>$").unwrap();
    let list = run_git(
        &["rev-list", &format!("{base}..{head}"), "--no-merges"],
        root,
    )?;
    let mut bad = false;
    for c in list.lines().filter(|l| !l.is_empty()) {
        let body = run_git(&["log", "-1", "--format=%B", c], root)?;
        if !body.lines().any(|l| re.is_match(l)) {
            eprintln!("DCO: commit {c} lacks Signed-off-by (use git commit -s)");
            bad = true;
        }
    }
    if bad {
        bail!("check-dco failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(cwd: &std::path::Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[test]
    fn dco_pass_and_fail() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        git(root, &["init", "-q"]);
        git(root, &["config", "user.email", "dco@example.com"]);
        git(root, &["config", "user.name", "DCO Test"]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git(root, &["add", "a.txt"]);
        // Create signed commit via -s
        let st = Command::new("git")
            .args(["commit", "-qs", "-m", "signed"])
            .current_dir(root)
            .status()
            .unwrap();
        assert!(st.success());
        let base = git(root, &["rev-parse", "HEAD"]);

        std::fs::write(root.join("b.txt"), "b\n").unwrap();
        git(root, &["add", "b.txt"]);
        git(root, &["commit", "-q", "-m", "unsigned"]);
        let head = git(root, &["rev-parse", "HEAD"]);

        assert!(
            check_range(root, &base, &head).is_err(),
            "unsigned commit must fail"
        );

        // signed-only range: base's parent..base
        let empty_base = git(root, &["rev-list", "--max-parents=0", "HEAD"]);
        // From root commit to first (signed) commit: need a range with only signed.
        // Amend approach: check base^..base when base is signed — but root has no parent.
        // Instead: create another signed commit after resetting unsigned.
        git(root, &["reset", "--hard", &base]);
        std::fs::write(root.join("c.txt"), "c\n").unwrap();
        git(root, &["add", "c.txt"]);
        let st = Command::new("git")
            .args(["commit", "-qs", "-m", "also signed"])
            .current_dir(root)
            .status()
            .unwrap();
        assert!(st.success());
        let head2 = git(root, &["rev-parse", "HEAD"]);
        assert!(
            check_range(root, &base, &head2).is_ok(),
            "signed commits must pass"
        );
        let _ = empty_base;
    }
}
