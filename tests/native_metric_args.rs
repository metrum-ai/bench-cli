// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use clap::Parser;
use metrum_ai_bench::cache_metric_capture::NativeMetricArgs;

#[derive(Parser)]
struct Flags {
    #[command(flatten)]
    native: NativeMetricArgs,
}

#[tokio::test]
async fn omitted_native_arguments_are_inert_and_partial_groups_are_rejected() {
    let flags = Flags::try_parse_from(["fixture"]).unwrap();
    flags.native.validate().unwrap();
    assert!(flags.native.capture_if_enabled().await.unwrap().is_none());
    for args in [
        vec![
            "fixture",
            "--cache-metrics-url",
            "http://127.0.0.1:1/metrics",
        ],
        vec!["fixture", "--cache-metrics-profile", "vllm-apc"],
        vec![
            "fixture",
            "--cache-metrics-source-revision",
            "0000000000000000000000000000000000000000",
        ],
        vec![
            "fixture",
            "--cache-metrics-url",
            "http://127.0.0.1:1/metrics",
            "--cache-metrics-profile",
            "tensorrt-llm",
        ],
    ] {
        assert!(Flags::try_parse_from(args).is_err());
    }
}

#[tokio::test]
async fn invalid_native_configuration_cannot_trigger_capture_or_echo_private_text() {
    for (url, revision) in [
        (
            "http://PRIVATE_USER:PRIVATE_PASSWORD@127.0.0.1:1/metrics",
            None,
        ),
        ("file:///not-an-input", None),
        ("http://127.0.0.1:1/metrics", Some("PRIVATE_REVISION")),
    ] {
        let flags = NativeMetricArgs {
            cache_metrics_url: Some(url.into()),
            cache_metrics_profile: Some(metrum_ai_bench::cache_metrics::ProfileId::VllmApc),
            cache_metrics_source_revision: revision.map(str::to_owned),
        };
        let error = flags.validate().unwrap_err().to_string();
        assert!(!error.contains("PRIVATE_"));
        assert!(!format!("{flags:?}").contains("PRIVATE_"));
        assert!(
            flags.capture_if_enabled().await.is_err(),
            "invalid configuration fails before HTTP rather than returning a fetch result"
        );
    }
}
