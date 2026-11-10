// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Actual stable collector serialization, synthetic metrics; no serving engine.
use metrum_ai_bench::cache_metric_capture::ProfileCapabilities;
use metrum_ai_bench::cache_metrics::{probe, ProfileId, Role, Unit, Unknown};

const PAGE: &str = include_str!("fixtures/trtllm121-collector-synthetic.prom");

#[test]
fn stable_source_defined_families_parse_without_inventing_cached_tokens() {
    let profile = ProfileId::TrtLlm121;
    assert_eq!(
        profile.source_revision(),
        "376f7e1bd8ed543f75014309e3fd4b237e9b0e73"
    );
    let result = probe(profile, Some(profile.source_revision()), PAGE).unwrap();
    assert!(result.missing_required_metrics.is_empty());
    assert!(result
        .observations
        .iter()
        .all(|r| r.value.is_some() && r.unknown.is_none()));
    assert!(!profile
        .metrics()
        .iter()
        .any(|s| s.unit == Unit::Tokens || s.role == Role::Hit));
    let successes = result
        .observations
        .iter()
        .find(|r| r.spec.metric == "trtllm_request_success_total")
        .unwrap();
    assert_eq!(successes.spec.unit, Unit::Requests);
    assert_eq!(successes.spec.role, Role::RequestCompleted);
    assert_eq!(successes.value, Some(1.0));
    let sum = result
        .observations
        .iter()
        .find(|r| r.spec.metric == "trtllm_time_to_first_token_seconds_sum")
        .unwrap();
    assert_eq!(sum.spec.unit, Unit::Seconds);
    assert_eq!(sum.value, Some(0.1));
    let cap = ProfileCapabilities::new(profile, Some(profile.source_revision()));
    assert!(cap
        .semantic_qualifiers
        .iter()
        .any(|s| s.contains("not exported by 1.2.1")));
    assert!(!cap.executable_version_verified);
    assert!(!cap.physical_tier_validation);
    assert!(cap.release_aliases.is_empty());
}

#[test]
fn newer_collector_revision_and_triton_names_are_not_stable_aliases() {
    let p = ProfileId::TrtLlm121;
    let wrong = probe(p, Some(ProfileId::TrtLlm.source_revision()), PAGE).unwrap();
    assert!(
        wrong
            .observations
            .iter()
            .all(|r| r.value.is_none()
                && r.unknown == Some(Unknown::RevisionNotDeclaredOrUnsupported))
    );
    let absent = probe(
        p,
        Some(p.source_revision()),
        "# TYPE nv_trt_llm_request_success counter\nnv_trt_llm_request_success 1\n",
    )
    .unwrap();
    assert!(!absent.missing_required_metrics.is_empty());
    assert!(absent.observations.is_empty());
    let newer = probe(
        ProfileId::TrtLlm,
        Some(ProfileId::TrtLlm.source_revision()),
        PAGE,
    )
    .unwrap();
    assert!(newer
        .missing_required_metrics
        .contains(&"trtllm_prompt_cached_tokens_total"));
}
