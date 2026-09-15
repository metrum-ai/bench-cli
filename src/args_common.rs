// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use clap::Args;
use serde::{Deserialize, Serialize};

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadBalancer {
    RoundRobin,
    LeastInflight,
}

/// Shared load/sampling flags for the four benchmark binaries.
#[derive(Args, Debug, Clone)]
pub struct CommonBenchArgs {
    #[arg(
        long,
        default_value_t = 0,
        help = "RNG seed for shuffle/arrival/unique prompts"
    )]
    pub seed: u64,

    #[arg(
        long,
        default_value_t = 0,
        help = "Warmup requests excluded from measurement"
    )]
    pub warmup_requests: u32,

    #[arg(
        long,
        help = "Open-loop request rate (req/s). Omit for closed-loop concurrency"
    )]
    pub request_rate: Option<f64>,

    #[arg(
        long,
        default_value = "constant",
        help = "Arrival process when --request-rate is set: constant|poisson"
    )]
    pub arrival: String,

    #[arg(
        long,
        value_parser = clap::value_parser!(u32).range(1..),
        help = "Hard cap for outstanding requests in open-loop mode"
    )]
    pub max_concurrency: Option<u32>,

    #[arg(long, value_enum, default_value_t = LoadBalancer::RoundRobin)]
    pub load_balancer: LoadBalancer,

    #[arg(
        long,
        default_value_t = false,
        help = "Send ignore_eos=true in the request body"
    )]
    pub ignore_eos: bool,

    #[arg(long, help = "min_tokens (vLLM / compatible servers)")]
    pub min_tokens: Option<u32>,

    #[arg(long, help = "Extra JSON object merged into the request body")]
    pub extra_body_json: Option<String>,

    #[arg(
        long,
        help = "Override the default system prompt (empty string disables it)"
    )]
    pub system_prompt: Option<String>,

    #[arg(
        long,
        default_value_t = false,
        help = "Prefix each prompt with a unique nonce to avoid prefix-cache hits"
    )]
    pub unique_prompts: bool,

    #[arg(
        long,
        help = "Path to tokenizer.json (requires build feature `tokenizer`)"
    )]
    pub tokenizer: Option<String>,

    #[arg(
        long = "slo",
        value_name = "METRIC=SECONDS",
        help = "Repeatable goodput threshold: ttft=, tpot=, e2e="
    )]
    pub slos: Vec<String>,

    #[arg(
        long,
        default_value_t = 10.0,
        help = "Throughput dispersion bin width in seconds"
    )]
    pub throughput_bin_seconds: f64,
}

impl CommonBenchArgs {
    pub fn unique_prompt(prompt: &str, seq: u64, unique: bool) -> String {
        if unique {
            format!("[nonce-{seq}] {prompt}")
        } else {
            prompt.to_string()
        }
    }

    pub fn arrival_kind(&self) -> crate::load::ArrivalKind {
        if self.request_rate.is_none() {
            return crate::load::ArrivalKind::ClosedLoop;
        }
        match self.arrival.to_ascii_lowercase().as_str() {
            "poisson" => crate::load::ArrivalKind::Poisson,
            _ => crate::load::ArrivalKind::Constant,
        }
    }

    pub fn parse_slos(&self) -> anyhow::Result<crate::summary::SloConfig> {
        crate::summary::SloConfig::parse(&self.slos)
    }
}
