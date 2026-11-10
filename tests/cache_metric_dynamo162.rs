// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Captured standalone registry serialization; synthetic observations, no GPU.
use metrum_ai_bench::cache_metric_capture::ProfileCapabilities;
use metrum_ai_bench::cache_metrics::{probe, ProfileId, Unit, Unknown};

const PAGE: &str = include_str!("fixtures/dynamo162-registry-synthetic.prom");

#[test]
fn stable_registry_exposition_matches_only_the_reviewed_source_slice() {
    let profile = ProfileId::Dynamo162Kvbm;
    assert_eq!(
        profile.source_revision(),
        "28f9765d98ded77149a94d0defee6d54e97f6741"
    );
    let result = probe(profile, Some(profile.source_revision()), PAGE).unwrap();
    assert!(result.missing_required_metrics.is_empty());
    for (name, unit, value) in [
        ("kvbm_matched_tokens", Unit::Tokens, 64.0),
        ("kvbm_offload_blocks_d2h", Unit::Blocks, 8.0),
        ("kvbm_onboard_blocks_h2d", Unit::Blocks, 4.0),
        ("kvbm_host_cache_hit_rate", Unit::Ratio, 0.5),
    ] {
        let row = result
            .observations
            .iter()
            .find(|r| r.spec.metric == name)
            .unwrap();
        assert_eq!(row.spec.unit, unit);
        assert_eq!(row.value, Some(value));
        assert_eq!(row.unknown, None);
    }
    let wrong = probe(profile, Some(ProfileId::DynamoKvbm.source_revision()), PAGE).unwrap();
    assert!(
        wrong
            .observations
            .iter()
            .all(|r| r.value.is_none()
                && r.unknown == Some(Unknown::RevisionNotDeclaredOrUnsupported))
    );
    assert!(!probe(profile, Some(profile.source_revision()), "")
        .unwrap()
        .missing_required_metrics
        .is_empty());
    let cap = ProfileCapabilities::new(profile, Some(profile.source_revision()));
    assert!(!cap.executable_version_verified);
    assert!(!cap.physical_tier_validation);
    assert!(cap.release_aliases.is_empty());
    assert!(cap.release_restriction.is_some());
}
