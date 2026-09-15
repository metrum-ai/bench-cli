// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Capture the rustc version used for this build (no wall-clock datetime).

fn main() {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let output = std::process::Command::new(&rustc)
        .arg("--version")
        .output()
        .unwrap_or_else(|e| panic!("failed to run `{rustc} --version`: {e}"));
    let version = String::from_utf8_lossy(&output.stdout);
    let version = version.trim();
    println!("cargo:rustc-env=RUSTC_VERSION_STRING={version}");
    println!("cargo:rerun-if-env-changed=RUSTC");
}
