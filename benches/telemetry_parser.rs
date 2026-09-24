// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

use criterion::{criterion_group, criterion_main, Criterion};
use metrum_ai_bench::telemetry::parse_exposition;
use regex::RegexSet;
use std::hint::black_box;
use std::time::Duration;

fn dcgm_dump_3mb() -> String {
    // ~3 MiB of DCGM-shaped lines with a mix of included and ignored metrics.
    let mut body = String::with_capacity(3 * 1024 * 1024);
    body.push_str("# HELP DCGM_FI_DEV_POWER_USAGE Power\n");
    body.push_str("# TYPE DCGM_FI_DEV_POWER_USAGE gauge\n");
    let metrics = [
        "DCGM_FI_DEV_POWER_USAGE",
        "DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION",
        "DCGM_FI_DEV_GPU_UTIL",
        "DCGM_FI_DEV_FB_USED",
        "DCGM_FI_DEV_SM_CLOCK",
        "DCGM_FI_DEV_GPU_TEMP",
        "DCGM_FI_PROF_SM_ACTIVE",
        "DCGM_FI_PROF_PIPE_TENSOR_ACTIVE",
        "DCGM_FI_PROF_DRAM_ACTIVE",
        "DCGM_FI_DEV_PCIE_TX_THROUGHPUT",
        "IGNORED_METRIC_A",
        "IGNORED_METRIC_B",
        "IGNORED_METRIC_C",
    ];
    let mut i = 0u64;
    while body.len() < 3 * 1024 * 1024 {
        let metric = metrics[i as usize % metrics.len()];
        let gpu = i % 8;
        body.push_str(&format!(
            "{metric}{{gpu=\"{gpu}\",UUID=\"GPU-{gpu}\",device=\"nvidia{gpu}\"}} {}\n",
            (i % 1000) as f64 * 0.1
        ));
        i += 1;
    }
    body
}

fn bench_parser(c: &mut Criterion) {
    let body = dcgm_dump_3mb();
    let include = RegexSet::new([
        r"^DCGM_FI_DEV_POWER_USAGE$",
        r"^DCGM_FI_DEV_TOTAL_ENERGY_CONSUMPTION$",
        r"^DCGM_FI_DEV_GPU_UTIL$",
        r"^DCGM_FI_DEV_FB_USED$",
        r"^DCGM_FI_DEV_SM_CLOCK$",
        r"^DCGM_FI_DEV_GPU_TEMP$",
        r"^DCGM_FI_PROF_SM_ACTIVE$",
        r"^DCGM_FI_PROF_PIPE_TENSOR_ACTIVE$",
        r"^DCGM_FI_PROF_DRAM_ACTIVE$",
        r"^DCGM_FI_DEV_PCIE_TX_THROUGHPUT$",
    ])
    .expect("regex");

    let mut group = c.benchmark_group("telemetry_parser");
    group.measurement_time(Duration::from_secs(8));
    group.sample_size(30);
    group.bench_function("dcgm_3mb_10_includes", |b| {
        b.iter(|| {
            let result = parse_exposition(black_box(&body), black_box(&include));
            black_box(result.samples.len())
        })
    });
    group.finish();
}

criterion_group!(benches, bench_parser);
criterion_main!(benches);
