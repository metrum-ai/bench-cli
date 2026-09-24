// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use chrono::{DateTime, Utc};
use std::time::Instant;

/// Shared monotonic epoch for one strategic run.
///
/// All `*_ns` fields in NDJSON rows are nanoseconds since `mono`. Wall clock
/// (`t0_wall`) is captured once at construction for ISO 8601 UTC anchoring.
#[derive(Debug, Clone)]
pub struct RunEpoch {
    mono: Instant,
    t0_wall: DateTime<Utc>,
}

impl RunEpoch {
    pub fn new() -> Self {
        Self {
            mono: Instant::now(),
            t0_wall: Utc::now(),
        }
    }

    /// Nanoseconds since the shared monotonic epoch.
    pub fn elapsed_ns(&self) -> u64 {
        self.mono.elapsed().as_nanos() as u64
    }

    /// Wall-clock ISO 8601 UTC string for the run start (`t0_wall`).
    pub fn t0_wall_iso(&self) -> String {
        self.t0_wall
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }

    pub fn t0_wall(&self) -> DateTime<Utc> {
        self.t0_wall
    }

    pub fn mono(&self) -> Instant {
        self.mono
    }
}

impl Default for RunEpoch {
    fn default() -> Self {
        Self::new()
    }
}
