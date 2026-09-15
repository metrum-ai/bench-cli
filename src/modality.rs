// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Contracts isolating modality-specific payload and response handling from
//! scheduling, transport, persistence, and summary calculation.

use crate::record::RequestRecord;
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ParsedResponse {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub ttft: Option<Duration>,
    pub first_reasoning: Option<Duration>,
    pub itl: Vec<Duration>,
    pub metadata: Value,
}

pub trait RequestBuilder<Item> {
    fn build_request(&self, item: &Item, sequence: u64) -> anyhow::Result<reqwest::Request>;
}

pub trait ResponseParser {
    fn on_bytes(&mut self, bytes: &[u8], elapsed: Duration) -> anyhow::Result<()>;
    fn finish(self) -> anyhow::Result<ParsedResponse>;
}

pub trait MetricExtractor {
    fn record(
        &self,
        sequence: u64,
        endpoint: String,
        latency: Duration,
        parsed: ParsedResponse,
    ) -> RequestRecord;
}
