// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::util::repo_root;
use anyhow::{bail, Context, Result};
use std::process::Command;

const FIXTURE_CFG: &str = r#"# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
title = "metrum-ai-bench-cli gitleaks fixture runner"
[extend]
useDefault = true
[[rules]]
id = "aws-account-id-near-account"
description = "12-digit AWS account ID adjacent to account wording"
regex = '''(?i)account[^0-9]{0,40}\b\d{12}\b|\b\d{12}\b[^0-9]{0,40}account'''
keywords = ["account"]
[[rules]]
id = "cleartext-password-assignment"
description = "Cleartext password assignment"
regex = '''(?i)password\s*[:=]\s*\S+'''
keywords = ["password"]
[[rules]]
id = "ssh-public-key-material"
description = "SSH public key material (rsa/ed25519)"
regex = '''ssh-(rsa|ed25519)\s+AAAA[0-9A-Za-z+/=]+'''
keywords = ["ssh-rsa", "ssh-ed25519"]
[[rules]]
id = "internal-hostname-suffix"
description = "Internal hostname matching the org suffix pattern"
regex = '''(?i)\b[a-z0-9][a-z0-9.-]*\.metrum\.ai\b'''
keywords = ["metrum.ai"]
"#;

pub fn run() -> Result<()> {
    let root = repo_root()?;
    if Command::new("gitleaks")
        .arg("version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        // `gitleaks version` may exit 0; also try which via `gitleaks detect --help`
        let probe = Command::new("gitleaks").arg("--help").output();
        if probe.is_err() || !probe.as_ref().unwrap().status.success() {
            bail!("gitleaks_test: gitleaks not installed");
        }
    }

    let tmp = tempfile::NamedTempFile::new().context("temp config")?;
    std::fs::write(tmp.path(), FIXTURE_CFG).context("write temp gitleaks config")?;

    let status = Command::new("gitleaks")
        .args([
            "detect",
            "--no-git",
            "--source",
            "scripts/tests/gitleaks",
            "--config",
        ])
        .arg(tmp.path())
        .arg("--verbose")
        .current_dir(&root)
        .status()
        .context("spawn gitleaks")?;

    if status.success() {
        bail!("gitleaks_test: expected findings in fixture dir, got clean exit");
    }
    println!("gitleaks_test: ok (custom rules fired on fixtures)");
    Ok(())
}
