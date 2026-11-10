// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Synthetic pinned-collector examples, not a captured TRT-LLM deployment.
use metrum_ai_bench::cache_metrics::{probe, ProfileId, Role, Unit, Unknown};
use metrum_ai_bench::telemetry::MetricType;

const CORE: &str="# TYPE trtllm_prompt_cached_tokens_total counter\ntrtllm_prompt_cached_tokens_total{model_name=\"private-model\"} 0\n";

#[test]
fn trt_views_keep_private_level_identity_without_summing_or_inventing_query_denominators() {
    let page = format!(
        "{CORE}\
# TYPE trtllm_prompt_cache_hit_tokens_total counter\n\
trtllm_prompt_cache_hit_tokens_total{{dp_rank=\"0\",cache_tier=\"gpu\",cache_level=\"0\"}} 10\n\
trtllm_prompt_cache_hit_tokens_total{{dp_rank=\"0\",cache_tier=\"gpu\",cache_level=\"1\"}} 20\n\
trtllm_prompt_cache_hit_tokens_total{{dp_rank=\"0\",cache_tier=\"connector\",cache_level=\"\"}} 4\n\
# TYPE trtllm_kv_cache_iter_connector_reused_blocks_total counter\n\
trtllm_kv_cache_iter_connector_reused_blocks_total{{dp_rank=\"0\"}} 2\n\
# TYPE trtllm_kv_cache_onboard_bytes_total counter\n\
trtllm_kv_cache_onboard_bytes_total{{dp_rank=\"0\"}} 64\n\
# TYPE trtllm_kv_cache_offload_bytes_total counter\n\
trtllm_kv_cache_offload_bytes_total{{dp_rank=\"0\"}} 128\n\
# TYPE trtllm_kv_cache_tokens_per_block gauge\n\
trtllm_kv_cache_tokens_per_block{{dp_rank=\"0\"}} 16\n"
    );
    let p = ProfileId::TrtLlm;
    let report = probe(p, Some(p.source_revision()), &page).unwrap();
    assert!(report.missing_required_metrics.is_empty());
    assert_eq!(report.observations.len(), 8);
    assert!(report.observations.iter().all(|row| row.value.is_some()));
    let tiers: Vec<_> = report
        .observations
        .iter()
        .filter(|row| row.spec.role == Role::ReportedCacheAttribution)
        .collect();
    assert_eq!(tiers.len(), 3);
    assert_eq!(tiers[0].labels["cache_level"], "0");
    assert_eq!(tiers[1].labels["cache_level"], "1");
    assert_eq!(tiers[0].value, Some(10.0));
    assert_eq!(tiers[1].value, Some(20.0));
    let accepted = report
        .observations
        .iter()
        .find(|row| row.spec.role == Role::ConnectorAccepted)
        .unwrap();
    assert_eq!(accepted.spec.unit, Unit::Blocks);
    assert_eq!(accepted.value, Some(2.0));
    let layout = report
        .observations
        .iter()
        .find(|row| row.spec.role == Role::Layout)
        .unwrap();
    assert_eq!(layout.spec.kind, MetricType::Gauge);
    assert_eq!(layout.spec.unit, Unit::TokensPerBlock);
    assert!(
        !p.metrics().iter().any(|spec| spec.role == Role::Query),
        "miss counts are not query counts; no automatic hit/query denominator"
    );
}

#[test]
fn trt_missing_core_fractional_bytes_and_triton_names_never_become_valid_zero_or_aliases() {
    let p = ProfileId::TrtLlm;
    let missing=probe(p,Some(p.source_revision()),"# TYPE trtllm_kv_cache_offload_bytes_total counter\ntrtllm_kv_cache_offload_bytes_total 1\n").unwrap();
    assert!(missing
        .observations
        .iter()
        .all(|row| row.value.is_none() && row.unknown == Some(Unknown::MissingRequired)));
    let invalid=probe(p,Some(p.source_revision()),&format!("{CORE}# TYPE trtllm_kv_cache_offload_bytes_total counter\ntrtllm_kv_cache_offload_bytes_total 1.5\n")).unwrap();
    assert_eq!(
        invalid
            .observations
            .iter()
            .find(|row| row.spec.role == Role::Write)
            .unwrap()
            .unknown,
        Some(Unknown::InvalidValue)
    );
    let alias=probe(p,Some(p.source_revision()),"# TYPE nv_trt_llm_kv_cache_block_metrics gauge\nnv_trt_llm_kv_cache_block_metrics{kv_cache_block_metrics_type=\"used\"} 5\n").unwrap();
    assert!(alias.observations.is_empty());
    assert_eq!(alias.unmatched_samples, 1);
    assert!(!alias.missing_required_metrics.is_empty());
}
