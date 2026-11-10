// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Trusted Unix operator reset hooks; acknowledgements are not independent
//! physical-cache evidence. Never interpolate workload data into shell commands.

use std::collections::BTreeSet;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

const STDERR_LIMIT: usize = 4096;

#[derive(Default)]
struct CapturedStderr {
    bytes: Vec<u8>,
    truncated: bool,
}

impl CapturedStderr {
    fn record(&mut self, bytes: &[u8]) {
        let keep = bytes.len().min(STDERR_LIMIT - self.bytes.len());
        self.bytes.extend_from_slice(&bytes[..keep]);
        self.truncated |= keep < bytes.len();
    }
}

/// Own only the newly spawned hook's process group, never the benchmark's group.
#[cfg(unix)]
struct HookProcessGroup(Option<nix::unistd::Pid>);

#[cfg(unix)]
impl HookProcessGroup {
    fn new(id: u32) -> Result<Self> {
        let id = i32::try_from(id).context("invalid hook process group ID")?;
        anyhow::ensure!(id > 0, "invalid hook process group ID");
        Ok(Self(Some(nix::unistd::Pid::from_raw(id))))
    }

    fn terminate(&mut self) -> Result<()> {
        if let Some(id) = self.0 {
            match nix::sys::signal::killpg(id, nix::sys::signal::Signal::SIGKILL) {
                Ok(()) | Err(nix::errno::Errno::ESRCH) => self.0 = None,
                Err(error) => return Err(error).context("could not terminate hook process group"),
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
impl Drop for HookProcessGroup {
    fn drop(&mut self) {
        if let Err(error) = self.terminate() {
            eprintln!("warning: {error:#}");
        }
    }
}

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
        if !cfg!(unix) {
            bail!("stage reset hooks require Unix /bin/sh");
        }
        if self.command.trim().is_empty() || self.timeout.is_zero() {
            bail!("stage reset requires a nonempty command and positive timeout");
        }
        if self.settle_delay >= self.timeout {
            bail!("stage reset settle delay must be less than the total timeout");
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

    /// Run after drain, before measurement. One deadline covers execution,
    /// readiness and settling. The command/stdout are never echoed; failure
    /// diagnostics include a bounded, escaped stderr prefix. Hooks must not emit
    /// secrets on stderr. Cancellation kills the hook's Unix process group.
    pub async fn execute(
        &self,
        stage_index: usize,
        load: f64,
        client: &reqwest::Client,
    ) -> Result<ResetAcknowledgement> {
        self.validate()?;
        let captured = Mutex::new(CapturedStderr::default());
        let result = tokio::time::timeout(
            self.timeout,
            self.execute_inner(stage_index, load, client, &captured),
        )
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("stage reset/readiness/settling timed out")));
        result.map_err(|error| {
            let stderr = captured.lock().unwrap_or_else(|poison| poison.into_inner());
            if stderr.bytes.is_empty() && !stderr.truncated {
                return error;
            }
            let marker = if stderr.truncated {
                "truncated to first 4 KiB"
            } else {
                "complete, at most 4 KiB"
            };
            // Debug escaping prevents stderr control sequences changing terminal
            // state. Retain only the bounded bytes, not an unbounded output tail.
            anyhow::anyhow!(
                "{error:#}; hook stderr ({marker}): {:?}",
                String::from_utf8_lossy(&stderr.bytes)
            )
        })
    }

    async fn execute_inner(
        &self,
        stage_index: usize,
        load: f64,
        client: &reqwest::Client,
        captured: &Mutex<CapturedStderr>,
    ) -> Result<ResetAcknowledgement> {
        let started_unix_ns = crate::strategic::now_unix_ns();
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args(["-c", &self.command])
            .env("BENCH_STAGE_INDEX", stage_index.to_string())
            .env("BENCH_STAGE_LOAD", load.to_string())
            .env("BENCH_RESET_SCOPE", self.scope.join(","))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.as_std_mut().process_group(0);
        }
        let mut child = command
            .spawn()
            .context("could not spawn trusted stage reset hook")?;
        #[cfg(unix)]
        let mut group = HookProcessGroup::new(child.id().context("hook process ID unavailable")?)?;
        let stdout = child
            .stdout
            .take()
            .context("stage reset stdout unavailable")?;
        let mut stderr = child
            .stderr
            .take()
            .context("stage reset stderr unavailable")?;
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
        let read_stderr = async {
            let mut chunk = [0u8; 8192];
            loop {
                let count = stderr
                    .read(&mut chunk)
                    .await
                    .context("stage reset stderr read failed")?;
                if count == 0 {
                    break;
                }
                captured
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .record(&chunk[..count]);
            }
            Ok::<_, anyhow::Error>(())
        };
        let wait = async { child.wait().await.context("stage reset wait failed") };
        let (bytes, status, ()) = tokio::try_join!(read, wait, read_stderr)?;
        // Kill any backgrounded descendants even after a zero-exit shell. Stop
        // owning the group before potentially long readiness/settling awaits.
        #[cfg(unix)]
        group.terminate()?;
        if !status.success() {
            bail!("stage reset hook exited unsuccessfully ({status})");
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
