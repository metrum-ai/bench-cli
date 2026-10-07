// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::util::repo_root;
use anyhow::{bail, Context, Result};

/// Port of cargo xtask release-notes-data-points awk + `cat -s` (squeeze blank lines).
pub fn transform(doc: &str) -> String {
    let mut fenced = false;
    let mut folded = false;
    let mut raw_lines: Vec<String> = Vec::new();

    for line in doc.lines() {
        if line.starts_with("```") {
            fenced = !fenced;
            raw_lines.push(line.to_string());
            continue;
        }
        if fenced {
            raw_lines.push(line.to_string());
            continue;
        }
        if line.starts_with("<!--") {
            continue;
        }
        if line == "# Data points" {
            raw_lines.push("## Data points".to_string());
            continue;
        }
        let mut line_out = line.to_string();
        if line == "## `summary.v3` quantities" && !folded {
            raw_lines.push(
                "<details><summary>Every counted field and when it fires</summary>".to_string(),
            );
            raw_lines.push(String::new());
            folded = true;
        }
        if line_out.starts_with('#') {
            // sub(/^#/, "##") — prepend one #
            line_out = format!("#{line_out}");
        }
        raw_lines.push(line_out);
    }
    if folded {
        raw_lines.push(String::new());
        raw_lines.push("</details>".to_string());
    }

    // cat -s: squeeze consecutive blank lines to one
    let mut out = Vec::new();
    let mut prev_blank = false;
    for line in raw_lines {
        let blank = line.is_empty();
        if blank && prev_blank {
            continue;
        }
        out.push(line);
        prev_blank = blank;
    }
    let mut s = out.join("\n");
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

pub fn run() -> Result<()> {
    let root = repo_root()?;
    let doc_path = root.join("docs/DATA_POINTS.md");
    if !doc_path.is_file() {
        bail!("error: {} missing", doc_path.display());
    }
    let doc = std::fs::read_to_string(&doc_path).context("read DATA_POINTS.md")?;
    print!("{}", transform(&doc));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_summary_section_and_drops_comments() {
        let doc = "\
<!-- gen -->
# Data points

Intro

## `summary.v3` quantities

| a | b |
| - | - |

## Other

text
";
        let out = transform(doc);
        assert!(out.starts_with("## Data points\n"));
        assert!(!out.contains("<!--"));
        assert!(out.contains("<details><summary>Every counted field and when it fires</summary>"));
        assert!(out.contains("### `summary.v3` quantities"));
        assert!(out.contains("</details>"));
        // Both-outcome: empty input is still valid transform
        assert_eq!(transform(""), "\n");
    }
}
