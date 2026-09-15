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

/// Serializable mirror of [`CommonBenchArgs`] for `summary.v3.config`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EffectiveCommonArgs {
    pub seed: u64,
    pub warmup_requests: u32,
    pub request_rate: Option<f64>,
    pub arrival: String,
    pub max_concurrency: Option<u32>,
    pub load_balancer: LoadBalancer,
    pub ignore_eos: bool,
    pub min_tokens: Option<u32>,
    pub extra_body_json: Option<String>,
    pub system_prompt: Option<String>,
    pub unique_prompts: bool,
    pub tokenizer: Option<String>,
    pub slos: Vec<String>,
    pub throughput_bin_seconds: f64,
}

impl From<&CommonBenchArgs> for EffectiveCommonArgs {
    fn from(common: &CommonBenchArgs) -> Self {
        Self {
            seed: common.seed,
            warmup_requests: common.warmup_requests,
            request_rate: common.request_rate,
            arrival: common.arrival.clone(),
            max_concurrency: common.max_concurrency,
            load_balancer: common.load_balancer,
            ignore_eos: common.ignore_eos,
            min_tokens: common.min_tokens,
            extra_body_json: common.extra_body_json.clone(),
            system_prompt: common.system_prompt.clone(),
            unique_prompts: common.unique_prompts,
            tokenizer: common.tokenizer.clone(),
            slos: common.slos.clone(),
            throughput_bin_seconds: common.throughput_bin_seconds,
        }
    }
}

impl CommonBenchArgs {
    /// Prefix prompt with a run-scoped nonce when `--unique-prompts` is set.
    ///
    /// Format: `[nonce-{run_id}-{seed}-{seq}] {prompt}`
    pub fn unique_prompt(prompt: &str, seq: u64, unique: bool, seed: u64, run_id: &str) -> String {
        if unique {
            format!("[nonce-{run_id}-{seed}-{seq}] {prompt}")
        } else {
            prompt.to_string()
        }
    }

    /// Template string recorded when unique prompts are enabled.
    pub fn unique_prompt_nonce_template(unique: bool) -> Option<String> {
        if unique {
            Some("[nonce-{run_id}-{seed}-{seq}]".to_string())
        } else {
            None
        }
    }

    /// Effective system message for chat-style modalities.
    ///
    /// - CLI omitted → default
    /// - CLI empty string → disabled (`None`)
    /// - CLI non-empty → that string
    pub fn effective_system_prompt(&self, default: &str) -> Option<String> {
        match self.system_prompt.as_deref() {
            None => Some(default.to_string()),
            Some("") => None,
            Some(s) => Some(s.to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_prompt_includes_run_id_seed_seq() {
        let out = CommonBenchArgs::unique_prompt("hello", 3, true, 7, "abc-run");
        assert_eq!(out, "[nonce-abc-run-7-3] hello");
    }

    #[test]
    fn unique_prompt_disabled_is_verbatim() {
        let out = CommonBenchArgs::unique_prompt("hello", 3, false, 7, "abc-run");
        assert_eq!(out, "hello");
    }

    #[test]
    fn effective_system_prompt_default_empty_override() {
        let mut args = CommonBenchArgs {
            seed: 0,
            warmup_requests: 0,
            request_rate: None,
            arrival: "constant".into(),
            max_concurrency: None,
            load_balancer: LoadBalancer::RoundRobin,
            ignore_eos: false,
            min_tokens: None,
            extra_body_json: None,
            system_prompt: None,
            unique_prompts: false,
            tokenizer: None,
            slos: vec![],
            throughput_bin_seconds: 10.0,
        };
        assert_eq!(
            args.effective_system_prompt("default"),
            Some("default".into())
        );
        args.system_prompt = Some(String::new());
        assert_eq!(args.effective_system_prompt("default"), None);
        args.system_prompt = Some("custom".into());
        assert_eq!(
            args.effective_system_prompt("default"),
            Some("custom".into())
        );
    }
}
