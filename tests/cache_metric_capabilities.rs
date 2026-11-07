// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use metrum_ai_bench::cache_metric_capture::{DeclaredSourceStatus, ProfileCapabilities};
use metrum_ai_bench::cache_metrics::{HistogramPart, ProfileId, Unit};

#[test]
fn exact_declared_source_compatibility_never_claims_executable_or_complete_support() {
    for profile in [
        ProfileId::VllmApc,
        ProfileId::VllmExternal,
        ProfileId::VllmOffload,
        ProfileId::Sglang,
        ProfileId::Lmcache,
        ProfileId::DynamoKvbm,
        ProfileId::TrtLlm,
    ] {
        let cap = ProfileCapabilities::new(profile, Some(profile.source_revision()));
        assert_eq!(
            cap.operator_declared_source_status,
            DeclaredSourceStatus::MatchesReference
        );
        assert!(!cap.complete_profile_support);
        assert!(!cap.executable_version_verified);
        assert!(!cap.physical_tier_validation);
        assert!(!cap.phase_delta_validation);
        assert!(!cap.histogram_distribution_validation);
        assert!(!cap.quantile_reconstruction);
        assert!(cap.release_aliases.is_empty());
        assert!(cap.unmapped_engines.is_empty());
        assert_eq!(
            cap.semantic_qualifiers.is_empty(),
            profile != ProfileId::TrtLlm
        );
        assert_eq!(cap.descriptors.len(), profile.metrics().len());
        assert_eq!(
            cap.release_restriction.is_some(),
            profile == ProfileId::DynamoKvbm
        );
        let missing = ProfileCapabilities::new(profile, None);
        assert_eq!(
            missing.operator_declared_source_status,
            DeclaredSourceStatus::NotDeclared
        );
        let invalid = ProfileCapabilities::new(profile, Some("PRIVATE_DECLARATION"));
        assert_eq!(
            invalid.operator_declared_source_status,
            DeclaredSourceStatus::Unsupported
        );
        assert!(!serde_json::to_string(&invalid)
            .unwrap()
            .contains("PRIVATE_DECLARATION"));
    }
}

#[test]
fn machine_descriptors_preserve_parent_histogram_units_and_gauge_non_cumulative_semantics() {
    let cap = ProfileCapabilities::new(ProfileId::Lmcache, None);
    let mut parts = 0;
    for desc in cap.descriptors {
        if desc.expected_family_type == "gauge" {
            assert!(!desc.cumulative);
        }
        if let Some(parent) = desc.histogram {
            parts += 1;
            assert_eq!(desc.expected_family_type, "histogram");
            assert!(desc.cumulative);
            assert_eq!(parent.unit, Unit::Seconds);
            assert_eq!(
                desc.unit,
                match parent.part {
                    HistogramPart::Sum => Unit::Seconds,
                    _ => Unit::Observations,
                }
            );
        }
    }
    assert_eq!(parts, 3);
}
