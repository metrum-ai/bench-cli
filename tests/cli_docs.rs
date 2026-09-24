// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Agent N gate: docs/CLI.md must reflect clap --help for shared workload flags.

use std::process::Command;

fn help_llm() -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-llm"))
        .arg("--help")
        .output()
        .expect("llm --help");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn help_vlm() -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-vlm"))
        .arg("--help")
        .output()
        .expect("vlm --help");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn help_asr() -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-asr"))
        .arg("--help")
        .output()
        .expect("asr --help");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn cli_md() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/CLI.md"))
        .expect("docs/CLI.md")
}

#[test]
fn cli_md_lists_shared_workload_flags_from_llm_help() {
    let md = cli_md();
    let llm = help_llm();
    for flag in [
        "--warmup-requests",
        "--seed",
        "--ignore-eos",
        "--extra-body-json",
        "--unique-prompts",
        "--request-rate",
        "--arrival",
        "--fail-on-error",
    ] {
        assert!(llm.contains(flag), "binary --help missing {flag}");
        assert!(
            md.contains(flag),
            "docs/CLI.md missing {flag}; re-run scripts/render_cli_help.sh"
        );
    }
}

#[test]
fn asr_help_and_cli_md_document_normalizer() {
    assert!(help_asr().contains("--normalizer"));
    assert!(cli_md().contains("--normalizer"));
}

#[test]
fn vlm_help_and_cli_md_document_reencode_jpeg() {
    assert!(help_vlm().contains("--reencode-jpeg"));
    assert!(cli_md().contains("--reencode-jpeg"));
}

#[test]
fn strategic_help_and_cli_md_document_chat_controls() {
    let output = Command::new(env!("CARGO_BIN_EXE_metrum-ai-bench-cli-strategic"))
        .arg("--help")
        .output()
        .expect("strategic --help");
    let help = String::from_utf8_lossy(&output.stdout);
    let md = cli_md();
    for flag in [
        "--ignore-eos",
        "--min-tokens",
        "--extra-body-json",
        "--warmup-requests",
    ] {
        assert!(help.contains(flag), "strategic --help missing {flag}");
        assert!(
            md.contains(flag),
            "docs/CLI.md missing {flag}; re-run scripts/render_cli_help.sh"
        );
    }
}
