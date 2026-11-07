// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
use metrum_ai_bench::{
    cache_metric_capture::NativeMetricArgs, cache_metrics::ProfileId,
    cache_phase_cli::PhaseMetricArgs, telemetry::TelemetryConfig,
};
fn fixture() -> (PhaseMetricArgs, NativeMetricArgs, TelemetryConfig) {
    let url = "http://127.0.0.1:9/metrics";
    let p = ProfileId::VllmApc;
    (
        PhaseMetricArgs {
            cache_phase_source: Some("engine".into()),
            cache_phase_epoch: Some("PRIVATE_CONTINUITY".into()),
            ..PhaseMetricArgs::default()
        },
        NativeMetricArgs {
            cache_metrics_url: Some(url.into()),
            cache_metrics_profile: Some(p),
            cache_metrics_source_revision: Some(p.source_revision().into()),
        },
        TelemetryConfig::from_metrics_url(url, 1000),
    )
}
#[test]
fn boundary_budgets_preserve_defaults_and_reject_overflow() {
    use metrum_ai_bench::cache_phase_runtime::boundary_budget_ms;
    assert_eq!(boundary_budget_ms(None).unwrap(), 5000);
    let (_, _, mut cfg) = fixture();
    cfg.timeout_ms = 500;
    assert_eq!(boundary_budget_ms(Some(&cfg)).unwrap(), 2000);
    cfg.timeout_ms = u64::MAX;
    assert!(boundary_budget_ms(Some(&cfg)).is_err());
}
#[test]
fn phase_selection_is_disabled_by_default_and_never_infers_source_or_continuity() {
    assert!(PhaseMetricArgs::default()
        .resolve(None, false, &NativeMetricArgs::default())
        .unwrap()
        .is_none());
    let (mut args, native, cfg) = fixture();
    assert!(args.resolve(Some(&cfg), true, &native).is_ok());
    assert!(!format!("{args:?}").contains("PRIVATE_CONTINUITY"));
    args.cache_phase_source = Some("missing".into());
    assert!(args.resolve(Some(&cfg), true, &native).is_err());
    args.cache_phase_source = None;
    assert!(args.resolve(Some(&cfg), true, &native).is_err());
}
#[test]
fn phase_binding_and_required_controls_fail_before_traffic() {
    let (mut args, mut native, mut cfg) = fixture();
    assert!(args.resolve(Some(&cfg), false, &native).is_err());
    cfg.sources[0].url = "http://127.0.0.1:9/other".into();
    assert!(args.resolve(Some(&cfg), true, &native).is_err());
    cfg.sources[0].url = native.cache_metrics_url.clone().unwrap();
    args.cache_phase_required = true;
    args.cache_phase_epoch = None;
    assert!(args.resolve(Some(&cfg), true, &native).is_err());
    args.cache_phase_epoch = Some("declared".into());
    native.cache_metrics_source_revision = None;
    assert!(args.resolve(Some(&cfg), true, &native).is_err());
    native.cache_metrics_source_revision = Some(ProfileId::VllmApc.source_revision().into());
    assert!(args.resolve(Some(&cfg), true, &native).is_ok());
    args.cache_phase_settle_ms = 10_001;
    assert!(args.resolve(Some(&cfg), true, &native).is_err());
    args.cache_phase_settle_ms = 0;
    cfg.sources[0].insecure_tls = true;
    assert!(args.resolve(Some(&cfg), true, &native).is_err());
}
#[test]
fn missing_capture_exports_unknown_without_fake_zero_windows_or_private_identity() {
    let (args, native, cfg) = fixture();
    let plan = args.resolve(Some(&cfg), true, &native).unwrap().unwrap();
    let report = plan.report("PRIVATE_RUN", None, None, None, None);
    assert_eq!(report["unknown"], "capture_unavailable");
    assert_eq!(report["physical_cache_state"], "unknown");
    let text = report.to_string();
    assert!(
        !text.contains("PRIVATE")
            && !text.contains("127.0.0.1")
            && !text.contains("fetch_start_ns")
    );
}
