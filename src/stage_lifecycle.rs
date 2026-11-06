// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Opt-in trusted operator reset hooks. These acknowledgements are not independent
//! proof of engine/tier behavior; callers must retain the declared scope.

use std::collections::BTreeSet;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

/// Trusted shell command plus the exact intended reset scope.
pub struct ResetHook {
    pub command: String,
    pub scope: Vec<String>,
    pub timeout: Duration,
    pub ready_url: Option<String>,
    pub settle_delay: Duration,
}

/// Non-sensitive acknowledgement retained in run summaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResetAcknowledgement {
    pub scope: Vec<String>,
    pub acknowledged: bool,
    pub ready: bool,
    pub settled: bool,
    pub started_unix_ns: u128,
    pub acknowledged_unix_ns: u128,
    pub ready_unix_ns: u128,
    pub settled_unix_ns: u128,
}

#[derive(Deserialize)]
struct HookReply {
    success: bool,
    scope: Vec<String>,
    ready: bool,
    settled: bool,
}

impl ResetHook {
    /// Validate without executing a command or sending traffic.
    pub fn validate(&self) -> Result<()> {
        if self.command.trim().is_empty() || self.timeout.is_zero() {
            bail!("stage reset requires a nonempty command and positive timeout");
        }
        let scope: BTreeSet<_> = self.scope.iter().collect();
        if scope.is_empty()
            || scope.len() != self.scope.len()
            || scope.iter().any(|s| s.trim().is_empty())
        {
            bail!("stage reset requires a nonempty unique tier scope");
        }
        if let Some(url) = &self.ready_url {
            crate::endpoints::validate_endpoint_url(url)
                .map_err(|_| anyhow::anyhow!("invalid stage readiness URL"))?;
        }
        Ok(())
    }

    /// Run after outstanding requests have drained and before measured issuance.
    /// The total deadline covers execution and readiness. Command/output text is
    /// intentionally excluded from errors because it may contain operator secrets.
    pub async fn execute(
        &self,
        stage_index: usize,
        load: f64,
        client: &reqwest::Client,
    ) -> Result<ResetAcknowledgement> {
        self.validate()?;
        tokio::time::timeout(self.timeout, self.execute_inner(stage_index, load, client))
            .await
            .map_err(|_| anyhow::anyhow!("stage reset/readiness timed out"))?
    }

    async fn execute_inner(
        &self,
        stage_index: usize,
        load: f64,
        client: &reqwest::Client,
    ) -> Result<ResetAcknowledgement> {
        let started_unix_ns = crate::strategic::now_unix_ns();
        let mut child = tokio::process::Command::new("/bin/sh")
            .args(["-c", &self.command])
            .env("BENCH_STAGE_INDEX", stage_index.to_string())
            .env("BENCH_STAGE_LOAD", load.to_string())
            .env("BENCH_RESET_SCOPE", self.scope.join(","))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("could not spawn trusted stage reset hook")?;
        let stdout = child
            .stdout
            .take()
            .context("stage reset stdout unavailable")?;
        let read = async {
            let mut bytes = Vec::new();
            stdout
                .take(65537)
                .read_to_end(&mut bytes)
                .await
                .context("stage reset stdout read failed")?;
            if bytes.len() > 65536 {
                bail!("stage reset acknowledgement exceeds 64 KiB");
            }
            Ok::<_, anyhow::Error>(bytes)
        };
        let wait = async { child.wait().await.context("stage reset wait failed") };
        let (bytes, status) = tokio::try_join!(read, wait)?;
        if !status.success() {
            bail!("stage reset hook exited unsuccessfully");
        }
        let reply: HookReply = serde_json::from_slice(&bytes).map_err(|_| {
            anyhow::anyhow!(
                "stage reset hook must emit a JSON scope/readiness/settling acknowledgement"
            )
        })?;
        let actual: BTreeSet<_> = reply.scope.iter().collect();
        let intended: BTreeSet<_> = self.scope.iter().collect();
        if !reply.success
            || !reply.settled
            || actual != intended
            || actual.len() != reply.scope.len()
        {
            bail!("stage reset failed or did not acknowledge every intended tier and settled transfer");
        }
        let acknowledged_unix_ns = crate::strategic::now_unix_ns();
        if let Some(url) = &self.ready_url {
            loop {
                if let Ok(response) = client.get(url).send().await {
                    if response.status().is_success() {
                        // HTTP 200 alone is not readiness evidence. Require an
                        // explicit JSON ready=true and never override success=false.
                        if let Ok(value) = response.json::<serde_json::Value>().await {
                            if (value.get("success").is_none()
                                || value.get("success").and_then(|v| v.as_bool()) == Some(true))
                                && value.get("ready").and_then(|v| v.as_bool()) == Some(true)
                            {
                                break;
                            }
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        } else if !reply.ready {
            bail!("stage reset did not acknowledge readiness");
        }
        let ready_unix_ns = crate::strategic::now_unix_ns();
        tokio::time::sleep(self.settle_delay).await;
        let settled_unix_ns = crate::strategic::now_unix_ns();
        Ok(ResetAcknowledgement {
            started_unix_ns,
            acknowledged_unix_ns,
            ready_unix_ns,
            settled_unix_ns,
            scope: self.scope.clone(),
            acknowledged: true,
            ready: true,
            settled: true,
        })
    }
}
