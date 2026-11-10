// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

#[test]
#[cfg(feature = "cert-probes")]
fn six_cpu_cases_reconcile_real_product_outputs_and_reject_controls_offline() {
    let output = std::process::Command::new("python3")
        .args([
            "-m",
            "unittest",
            "discover",
            "-s",
            "scripts/validation/cert",
            "-p",
            "test_kv_cpu.py",
            "-v",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env(
            "KV_CPU_PROMPTS",
            env!("CARGO_BIN_EXE_metrum-ai-bench-cli-prompts"),
        )
        .env(
            "KV_CPU_PROBE",
            env!("CARGO_BIN_EXE_metrum-ai-bench-cli-cert-kv-cpu"),
        )
        .output()
        .expect("run offline CPU certification integration tests");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
