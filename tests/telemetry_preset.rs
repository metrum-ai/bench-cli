// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Integration test: the shipped all-smi fork + vLLM telemetry preset (#311)
//! selects exactly the documented series from the fixtures.

use std::collections::BTreeSet;
use std::path::PathBuf;

use metrum_ai_bench::telemetry::TelemetryConfig;

fn repo_file(path: &str) -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path))
        .unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// Selected series of `source_name` in `cfg` applied to `exposition`, via
/// the library's doc-hidden preview helper (no third-party regex types on
/// the public surface, #326 review).
fn selected_series(cfg: &TelemetryConfig, source_name: &str, exposition: &str) -> BTreeSet<String> {
    cfg.preview_source_selection(source_name, exposition)
        .expect("preview selection")
        .into_iter()
        .collect()
}

#[test]
fn preset_parses_and_selects_the_documented_series() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cfg = TelemetryConfig::load(&dir.join("examples/telemetry/all-smi-vllm.yaml"))
        .expect("preset YAML must parse");
    let sources = cfg.compile().expect("preset must compile");
    assert_eq!(sources.len(), 2, "preset has exactly the two sources");

    let all_smi = sources
        .iter()
        .find(|s| s.name == "all-smi")
        .expect("all-smi source");
    assert_eq!(all_smi.interval_ms, 500);
    assert_eq!(all_smi.url, "http://127.0.0.1:9090/metrics");
    // Unit scaling for the hardware-millijoule energy counter (#311).
    let scale = all_smi
        .units
        .get("all_smi_gpu_energy_hw_millijoules_total")
        .expect("millijoule unit scaling present");
    assert!((scale.scale - 0.001).abs() < f64::EPSILON);
    assert_eq!(scale.unit, "J");

    let vllm = sources
        .iter()
        .find(|s| s.name == "vllm")
        .expect("vllm source");
    assert_eq!(vllm.interval_ms, 1000);
    assert_eq!(vllm.url, "http://127.0.0.1:8000/metrics");

    // The all-smi fixture is the redacted H100 capture from the fork
    // (v0.26.3-metrum.4); the preset's GPU group (power/energy/utilization/
    // memory/temperature/clock/frequency) selects exactly the series below.
    let all_smi_selected = selected_series(
        &cfg,
        "all-smi",
        &repo_file("test-data/telemetry/all-smi-fork-h100.prom"),
    );
    let expected_all_smi: BTreeSet<String> = [
        "all_smi_chassis_power_watts",
        "all_smi_cpu_core_count",
        "all_smi_cpu_frequency_mhz",
        "all_smi_cpu_socket_count",
        "all_smi_cpu_socket_frequency_mhz",
        "all_smi_cpu_socket_utilization",
        "all_smi_cpu_thread_count",
        "all_smi_cpu_utilization",
        "all_smi_energy_consumed_joules_total",
        "all_smi_gpu_clock_graphics_max_mhz",
        "all_smi_gpu_energy_hw_millijoules_total",
        "all_smi_gpu_frequency_mhz",
        "all_smi_gpu_hollow_utilization_ratio",
        "all_smi_gpu_memory_bandwidth_utilization",
        "all_smi_gpu_memory_total_bytes",
        "all_smi_gpu_memory_used_bytes",
        "all_smi_gpu_power_consumption_watts",
        "all_smi_gpu_power_limit_current_watts",
        "all_smi_gpu_power_limit_max_watts",
        "all_smi_gpu_temperature_celsius",
        "all_smi_gpu_temperature_threshold_max_operating_celsius",
        "all_smi_gpu_temperature_threshold_shutdown_celsius",
        "all_smi_gpu_temperature_threshold_slowdown_celsius",
        "all_smi_gpu_utilization",
        "all_smi_gpu_utilization_sample_max",
        "all_smi_gpu_utilization_sample_p50",
        "all_smi_gpu_utilization_sample_p95",
        "all_smi_memory_available_bytes",
        "all_smi_memory_buffers_bytes",
        "all_smi_memory_cached_bytes",
        "all_smi_memory_free_bytes",
        "all_smi_memory_total_bytes",
        "all_smi_memory_used_bytes",
        "all_smi_memory_utilization",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(
        all_smi_selected, expected_all_smi,
        "preset selects exactly the documented all-smi series"
    );
    // Known non-targets stay out (exporter status, process-scoped, PCIe).
    for absent in [
        "all_smi_up",
        "all_smi_build_info",
        "all_smi_gpu_pcie_gen_current",
    ] {
        assert!(
            !all_smi_selected.contains(absent),
            "preset must not select {absent}"
        );
    }

    // The vLLM fixture mirrors the 0.32.0 study server (one sample per
    // series); the preset selects the full documented engine set.
    let vllm_selected = selected_series(
        &cfg,
        "vllm",
        &repo_file("test-data/telemetry/vllm-0.32.prom"),
    );
    let expected_vllm: BTreeSet<String> = [
        "vllm:e2e_request_latency_seconds_bucket",
        "vllm:e2e_request_latency_seconds_count",
        "vllm:e2e_request_latency_seconds_sum",
        "vllm:external_prefix_cache_hits_total",
        "vllm:external_prefix_cache_queries_total",
        "vllm:generation_tokens_total",
        "vllm:gpu_cache_usage_perc",
        "vllm:inter_token_latency_seconds_bucket",
        "vllm:inter_token_latency_seconds_count",
        "vllm:inter_token_latency_seconds_sum",
        "vllm:kv_cache_usage_perc",
        "vllm:num_preemptions_total",
        "vllm:num_requests_running",
        "vllm:num_requests_waiting",
        "vllm:prefix_cache_hits_total",
        "vllm:prefix_cache_queries_total",
        "vllm:prompt_tokens_cached_total",
        "vllm:prompt_tokens_total",
        "vllm:time_to_first_token_seconds_bucket",
        "vllm:time_to_first_token_seconds_count",
        "vllm:time_to_first_token_seconds_sum",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(
        vllm_selected, expected_vllm,
        "preset selects exactly the documented vLLM series"
    );
    for absent in [
        "vllm:request_queue_time_seconds_bucket",
        "vllm:request_queue_time_seconds_sum",
        "vllm:request_queue_time_seconds_count",
        "vllm:request_success_total",
        "vllm:cache_config_info",
    ] {
        assert!(
            !vllm_selected.contains(absent),
            "preset must not select {absent}"
        );
    }
}

#[test]
fn four_node_variant_compiles_to_eight_loopback_sources() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cfg = TelemetryConfig::load(&dir.join("examples/telemetry/all-smi-vllm-4node.yaml"))
        .expect("four-node YAML must parse");
    let sources = cfg.compile().expect("four-node YAML must compile");
    assert_eq!(sources.len(), 8, "four lanes, two sources each");
    let names: BTreeSet<&str> = sources.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names.len(), 8, "source names are unique per lane");
    for name in ["node-a", "node-b", "node-c", "node-d"] {
        assert!(names.contains(&format!("{name}/all-smi").as_str()));
        assert!(names.contains(&format!("{name}/vllm").as_str()));
    }
    // Loopback forwarding placeholders only: no host identities.
    for src in &sources {
        assert!(
            src.url.starts_with("http://127.0.0.1:"),
            "four-node URLs are loopback placeholders: {}",
            src.url
        );
    }
}
