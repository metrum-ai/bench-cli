// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Source-pinned native cache semantics, not live engine certification.
//! Operator-declared revisions are not independently observed versions. Keep raw
//! label identity private; do not serialize these observations as public reports.

use crate::telemetry::{parse_exposition, MetricType};
use regex::RegexSet;
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const SCHEMA_VERSION: &str = "cache-metric-profile.v1";
const MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SERIES: usize = 256;
const MAX_EXACT_INTEGER: f64 = 9_007_199_254_740_992.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileId {
    VllmApc,
    VllmExternal,
    VllmOffload,
    /// vLLM source revision 4ba787ec, as identified by the v0.33.0 image.
    Vllm4baApc,
    Vllm4baExternal,
    Vllm4baOffload,
    Sglang,
    /// SGLang v0.5.23 source slice; independently selected, never a release alias.
    Sglang0523,
    Lmcache,
    DynamoKvbm,
    TrtLlm,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Tokens,
    Requests,
    Bytes,
    Blocks,
    Ratio,
    Seconds,
    Events,
    Keys,
    EvictionCandidates,
    Observations,
    TokensPerBlock,
}
impl Unit {
    fn declaration(self) -> &'static str {
        match self {
            Self::Tokens => "tokens",
            Self::Requests => "requests",
            Self::Bytes => "bytes",
            Self::Blocks => "blocks",
            Self::Ratio => "ratio",
            Self::Seconds => "seconds",
            Self::Events => "events",
            Self::Keys => "keys",
            Self::EvictionCandidates => "eviction_candidates",
            Self::Observations => "observations",
            Self::TokensPerBlock => "tokens_per_block",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Query,
    Hit,
    Stored,
    Read,
    Write,
    Transfer,
    Utilization,
    ReportedHitRate,
    Matched,
    Residency,
    Eviction,
    Failure,
    TransferTime,
    OperationTime,
    Miss,
    Reuse,
    ConnectorAccepted,
    ReportedCacheAttribution,
    Allocation,
    Capacity,
    Layout,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistogramPart {
    Bucket,
    Count,
    Sum,
}
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct HistogramSpec {
    pub family: &'static str,
    pub unit: Unit,
    pub part: HistogramPart,
}
#[derive(Debug, Clone, Copy)]
pub struct MetricSpec {
    pub metric: &'static str,
    pub kind: MetricType,
    pub unit: Unit,
    pub role: Role,
    pub required: bool,
    /// Parent TYPE/UNIT evidence; count/bucket units are observations, not the
    /// parent duration/size unit. Scalar kind expresses cumulative semantics.
    pub histogram: Option<HistogramSpec>,
}
const fn c(metric: &'static str, unit: Unit, role: Role) -> MetricSpec {
    MetricSpec {
        metric,
        kind: MetricType::Counter,
        unit,
        role,
        required: true,
        histogram: None,
    }
}
impl MetricSpec {
    const fn optional(mut self) -> Self {
        self.required = false;
        self
    }
}
const fn g(metric: &'static str, role: Role) -> MetricSpec {
    gauge(metric, Unit::Ratio, role)
}
const fn gauge(metric: &'static str, unit: Unit, role: Role) -> MetricSpec {
    MetricSpec {
        metric,
        kind: MetricType::Gauge,
        unit,
        role,
        required: false,
        histogram: None,
    }
}
const fn h(
    metric: &'static str,
    family: &'static str,
    part: HistogramPart,
    unit: Unit,
    role: Role,
) -> MetricSpec {
    MetricSpec {
        metric,
        kind: match part {
            HistogramPart::Bucket => MetricType::HistogramBucket,
            _ => MetricType::Counter,
        },
        unit: match part {
            HistogramPart::Sum => unit,
            _ => Unit::Observations,
        },
        role,
        required: false,
        histogram: Some(HistogramSpec { family, unit, part }),
    }
}

const VLLM_APC: &[MetricSpec] = &[
    c("vllm:prefix_cache_queries_total", Unit::Tokens, Role::Query),
    c("vllm:prefix_cache_hits_total", Unit::Tokens, Role::Hit),
    g("vllm:kv_cache_usage_perc", Role::Utilization),
];
const VLLM_EXTERNAL: &[MetricSpec] = &[
    c(
        "vllm:external_prefix_cache_queries_total",
        Unit::Tokens,
        Role::Query,
    ),
    c(
        "vllm:external_prefix_cache_hits_total",
        Unit::Tokens,
        Role::Hit,
    ),
];
// Flat transfer metrics only. Deprecated transfer_type-labelled aggregates are
// intentionally not aliases and must not be added to these directional totals.
const VLLM_OFFLOAD: &[MetricSpec] = &[
    h(
        "vllm:kv_offload_lookup_sync_delay_seconds_bucket",
        "vllm:kv_offload_lookup_sync_delay_seconds",
        HistogramPart::Bucket,
        Unit::Seconds,
        Role::OperationTime,
    ),
    h(
        "vllm:kv_offload_lookup_sync_delay_seconds_count",
        "vllm:kv_offload_lookup_sync_delay_seconds",
        HistogramPart::Count,
        Unit::Seconds,
        Role::OperationTime,
    ),
    h(
        "vllm:kv_offload_lookup_sync_delay_seconds_sum",
        "vllm:kv_offload_lookup_sync_delay_seconds",
        HistogramPart::Sum,
        Unit::Seconds,
        Role::OperationTime,
    ),
    c("vllm:kv_offload_load_bytes_total", Unit::Bytes, Role::Read),
    c(
        "vllm:kv_offload_store_bytes_total",
        Unit::Bytes,
        Role::Write,
    ),
    c(
        "vllm:kv_offload_load_time_total",
        Unit::Seconds,
        Role::TransferTime,
    )
    .optional(),
    c(
        "vllm:kv_offload_store_time_total",
        Unit::Seconds,
        Role::TransferTime,
    )
    .optional(),
    c(
        "vllm:kv_offload_allocation_failure_total",
        Unit::Events,
        Role::Failure,
    )
    .optional(),
];
const SGLANG: &[MetricSpec] = &[
    c("sglang:cached_tokens_total", Unit::Tokens, Role::Hit),
    g("sglang:cache_hit_rate", Role::ReportedHitRate),
];
const LMCACHE: &[MetricSpec] = &[
    h(
        "lmcache:time_to_retrieve_bucket",
        "lmcache:time_to_retrieve",
        HistogramPart::Bucket,
        Unit::Seconds,
        Role::OperationTime,
    ),
    h(
        "lmcache:time_to_retrieve_count",
        "lmcache:time_to_retrieve",
        HistogramPart::Count,
        Unit::Seconds,
        Role::OperationTime,
    ),
    h(
        "lmcache:time_to_retrieve_sum",
        "lmcache:time_to_retrieve",
        HistogramPart::Sum,
        Unit::Seconds,
        Role::OperationTime,
    ),
    c(
        "lmcache:num_requested_tokens_total",
        Unit::Tokens,
        Role::Query,
    ),
    c("lmcache:num_hit_tokens_total", Unit::Tokens, Role::Hit),
    c(
        "lmcache:num_stored_tokens_total",
        Unit::Tokens,
        Role::Stored,
    ),
    c(
        "lmcache:num_remote_read_bytes_total",
        Unit::Bytes,
        Role::Read,
    ),
    c(
        "lmcache:num_remote_write_bytes_total",
        Unit::Bytes,
        Role::Write,
    ),
    c(
        "lmcache:num_retrieve_requests_total",
        Unit::Requests,
        Role::Read,
    ),
    gauge("lmcache:local_cache_usage", Unit::Bytes, Role::Residency),
    gauge("lmcache:remote_cache_usage", Unit::Bytes, Role::Residency),
    gauge("lmcache:local_storage_usage", Unit::Bytes, Role::Residency),
    c(
        "lmcache:local_cpu_evict_count_total",
        Unit::Events,
        Role::Eviction,
    )
    .optional(),
    c(
        "lmcache:local_cpu_evict_keys_count_total",
        Unit::Keys,
        Role::Eviction,
    )
    .optional(),
    c(
        "lmcache:local_cpu_evict_failed_count_total",
        // local_cpu_backend passes num_candidates, which may be up to 64 per
        // failed selection. This is neither failed calls nor evicted keys.
        Unit::EvictionCandidates,
        Role::Failure,
    )
    .optional(),
];
const KVBM: &[MetricSpec] = &[
    c("kvbm_offload_blocks_d2h", Unit::Blocks, Role::Transfer),
    c("kvbm_onboard_blocks_h2d", Unit::Blocks, Role::Transfer),
    c("kvbm_matched_tokens", Unit::Tokens, Role::Matched),
    g("kvbm_host_cache_hit_rate", Role::ReportedHitRate),
    g("kvbm_disk_cache_hit_rate", Role::ReportedHitRate),
    g("kvbm_object_cache_hit_rate", Role::ReportedHitRate),
    c("kvbm_object_read_failures", Unit::Blocks, Role::Failure).optional(),
    c("kvbm_object_write_failures", Unit::Blocks, Role::Failure).optional(),
];

// TRT-LLM's collector exposes several overlapping views. Keep legacy manager
// reuse separate from iteration reuse and tier attribution. Connector acceptance
// precedes async completion; layout gauges do not support block->byte conversion.
const TRTLLM: &[MetricSpec] = &[
    c("trtllm_prompt_cached_tokens_total", Unit::Tokens, Role::Hit),
    c(
        "trtllm_prompt_cache_hit_tokens_total",
        Unit::Tokens,
        Role::ReportedCacheAttribution,
    )
    .optional(),
    c(
        "trtllm_kv_cache_reused_blocks_total",
        Unit::Blocks,
        Role::Reuse,
    )
    .optional(),
    c(
        "trtllm_kv_cache_missed_blocks_total",
        Unit::Blocks,
        Role::Miss,
    )
    .optional(),
    c(
        "trtllm_kv_cache_iter_reused_blocks_total",
        Unit::Blocks,
        Role::Reuse,
    )
    .optional(),
    c(
        "trtllm_kv_cache_iter_full_reused_blocks_total",
        Unit::Blocks,
        Role::Reuse,
    )
    .optional(),
    c(
        "trtllm_kv_cache_iter_partial_reused_blocks_total",
        Unit::Blocks,
        Role::Reuse,
    )
    .optional(),
    c(
        "trtllm_kv_cache_iter_missed_blocks_total",
        Unit::Blocks,
        Role::Miss,
    )
    .optional(),
    c(
        "trtllm_kv_cache_iter_connector_reused_blocks_total",
        Unit::Blocks,
        Role::ConnectorAccepted,
    )
    .optional(),
    c(
        "trtllm_kv_cache_gen_alloc_blocks_total",
        Unit::Blocks,
        Role::Allocation,
    )
    .optional(),
    c(
        "trtllm_kv_cache_onboard_bytes_total",
        Unit::Bytes,
        Role::Read,
    )
    .optional(),
    c(
        "trtllm_kv_cache_offload_bytes_total",
        Unit::Bytes,
        Role::Write,
    )
    .optional(),
    c(
        "trtllm_kv_cache_disk_prefetch_blocks_total",
        Unit::Blocks,
        Role::Transfer,
    )
    .optional(),
    c(
        "trtllm_kv_cache_intra_device_copy_bytes_total",
        Unit::Bytes,
        Role::Transfer,
    )
    .optional(),
    g("trtllm_kv_cache_hit_rate", Role::ReportedHitRate),
    g("trtllm_kv_cache_iter_reuse_rate", Role::ReportedHitRate),
    g("trtllm_kv_cache_utilization", Role::Utilization),
    g("trtllm_kv_cache_host_utilization", Role::Utilization),
    gauge("trtllm_kv_cache_max_blocks", Unit::Blocks, Role::Capacity),
    gauge("trtllm_kv_cache_free_blocks", Unit::Blocks, Role::Capacity),
    gauge("trtllm_kv_cache_used_blocks", Unit::Blocks, Role::Residency),
    gauge(
        "trtllm_kv_cache_tokens_per_block",
        Unit::TokensPerBlock,
        Role::Layout,
    ),
];

impl ProfileId {
    pub fn source_revision(self) -> &'static str {
        match self {
            Self::VllmApc | Self::VllmExternal | Self::VllmOffload => {
                "aa4022dc8176db8f769f93445d845f8755f9b4ca"
            }
            Self::Vllm4baApc | Self::Vllm4baExternal | Self::Vllm4baOffload => {
                "4ba787ec9959e566d3b587a796dff07673324da5"
            }
            Self::Sglang => "188cf7da66938ca2e1ea9cbf7ebbf9a2c81edc2a",
            Self::Sglang0523 => "297e54fd105e974cdf401e5b4d20253a22c16f90",
            Self::Lmcache => "34f8a409e3631eff51d0b6fff67ca9c68f123377",
            Self::DynamoKvbm => "fa9852014950ed0aea74e56e1de441fc6de15307",
            Self::TrtLlm => "a3febfbfd0324bded62007ffd7d5241a06cbfc00",
        }
    }
    pub fn metrics(self) -> &'static [MetricSpec] {
        match self {
            Self::VllmApc | Self::Vllm4baApc => VLLM_APC,
            Self::VllmExternal | Self::Vllm4baExternal => VLLM_EXTERNAL,
            Self::VllmOffload | Self::Vllm4baOffload => VLLM_OFFLOAD,
            Self::Sglang | Self::Sglang0523 => SGLANG,
            Self::Lmcache => LMCACHE,
            Self::DynamoKvbm => KVBM,
            Self::TrtLlm => TRTLLM,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unknown {
    RevisionNotDeclaredOrUnsupported,
    TypeNotDeclaredOrConflicting,
    InvalidValue,
    CounterPrecision,
    DuplicateIdentity,
    InvalidLabels,
    MissingRequired,
    UnitNotCompatible,
}

/// Deliberately not Serialize: labels may contain private workload/worker data.
/// Future public projection must explicitly bound and redact labels; never sum
/// distinct label sets, even if they look like replicas or storage partitions.
#[derive(Debug)]
pub struct Observation {
    pub spec: MetricSpec,
    pub labels: BTreeMap<String, String>,
    pub raw_value: f64,
    pub value: Option<f64>,
    pub unknown: Option<Unknown>,
}
#[derive(Debug)]
pub struct Probe {
    pub schema_version: &'static str,
    pub profile: ProfileId,
    pub observations: Vec<Observation>,
    pub missing_metrics: Vec<&'static str>,
    pub missing_required_metrics: Vec<&'static str>,
    pub unmatched_samples: usize,
    pub malformed_lines: u64,
}
#[derive(Debug, PartialEq, Eq)]
pub enum ProbeError {
    PageTooLarge,
    TooManySeries,
}

/// Probe existing exposition without altering scraper/filter defaults. Exact
/// sample names only; a `_total` suffix alone never establishes observed TYPE.
/// No observed value, including zero, proves physical tier participation.
pub fn probe(
    profile: ProfileId,
    declared_revision: Option<&str>,
    text: &str,
) -> Result<Probe, ProbeError> {
    if text.len() > MAX_PAGE_BYTES {
        return Err(ProbeError::PageTooLarge);
    }
    static APC: OnceLock<RegexSet> = OnceLock::new();
    static EXTERNAL: OnceLock<RegexSet> = OnceLock::new();
    static OFFLOAD: OnceLock<RegexSet> = OnceLock::new();
    static SGLANG_MATCHER: OnceLock<RegexSet> = OnceLock::new();
    static LMCACHE_MATCHER: OnceLock<RegexSet> = OnceLock::new();
    static KVBM_MATCHER: OnceLock<RegexSet> = OnceLock::new();
    static TRTLLM_MATCHER: OnceLock<RegexSet> = OnceLock::new();
    let matcher = match profile {
        ProfileId::VllmApc | ProfileId::Vllm4baApc => &APC,
        ProfileId::VllmExternal | ProfileId::Vllm4baExternal => &EXTERNAL,
        ProfileId::VllmOffload | ProfileId::Vllm4baOffload => &OFFLOAD,
        ProfileId::Sglang | ProfileId::Sglang0523 => &SGLANG_MATCHER,
        ProfileId::Lmcache => &LMCACHE_MATCHER,
        ProfileId::DynamoKvbm => &KVBM_MATCHER,
        ProfileId::TrtLlm => &TRTLLM_MATCHER,
    };
    // Bound relevant series, not every unrelated histogram on an engine page.
    // Reuse the parser's filter-before-label-parsing behavior.
    let include = matcher.get_or_init(|| {
        RegexSet::new(
            profile
                .metrics()
                .iter()
                .map(|spec| format!("^{}$", regex::escape(spec.metric))),
        )
        .expect("constant profile metric expressions")
    });
    let parsed = parse_exposition(text, include);
    if parsed.samples.len() > MAX_SERIES {
        return Err(ProbeError::TooManySeries);
    }
    // Keep actual declarations separate from the legacy parser's type inference.
    let mut types: BTreeMap<&str, Option<&str>> = BTreeMap::new();
    let mut units: BTreeMap<&str, Option<&str>> = BTreeMap::new();
    let mut seen_samples = std::collections::BTreeSet::new();
    for line in text.lines() {
        if line.trim() == "# EOF" {
            break;
        }
        if let Some(rest) = line.trim().strip_prefix("# TYPE ") {
            let parts: Vec<_> = rest.split_whitespace().collect();
            if parts.len() != 2 {
                continue;
            }
            if seen_samples.contains(parts[0])
                || ["_total", "_bucket", "_count", "_sum"]
                    .iter()
                    .any(|suffix| seen_samples.contains(format!("{}{suffix}", parts[0]).as_str()))
            {
                // A declaration arriving after its family is not TYPE evidence
                // for samples that preceded it.
                if !types.contains_key(parts[0]) {
                    types.insert(parts[0], None);
                }
            }
            types
                .entry(parts[0])
                .and_modify(|old| {
                    if *old != Some(parts[1]) {
                        *old = None;
                    }
                })
                .or_insert(Some(parts[1]));
        } else if let Some(rest) = line.trim().strip_prefix("# UNIT ") {
            let parts: Vec<_> = rest.split_whitespace().collect();
            if parts.len() == 2 {
                units
                    .entry(parts[0])
                    .and_modify(|old| {
                        if *old != Some(parts[1]) {
                            *old = None;
                        }
                    })
                    .or_insert(Some(parts[1]));
            }
        } else if !line.trim().is_empty() && !line.trim().starts_with('#') {
            if let Some(name) = line.trim().split(['{', ' ', '\t']).next() {
                seen_samples.insert(name);
            }
        }
    }
    let missing_metrics: Vec<_> = profile
        .metrics()
        .iter()
        .filter(|spec| !parsed.samples.iter().any(|row| row.metric == spec.metric))
        .map(|spec| spec.metric)
        .collect();
    let missing_required_metrics: Vec<_> = profile
        .metrics()
        .iter()
        .filter(|spec| spec.required && missing_metrics.contains(&spec.metric))
        .map(|spec| spec.metric)
        .collect();
    let mut observations = Vec::new();
    let mut unmatched_samples = usize::try_from(parsed.skipped_lines).unwrap_or(usize::MAX);
    let mut identities: BTreeMap<(String, BTreeMap<String, String>), Vec<usize>> = BTreeMap::new();
    for sample in parsed.samples {
        let Some(&spec) = profile
            .metrics()
            .iter()
            .find(|spec| spec.metric == sample.metric)
        else {
            unmatched_samples += 1;
            continue;
        };
        let expected_type = if spec.histogram.is_some() {
            "histogram"
        } else if spec.kind == MetricType::Counter {
            "counter"
        } else {
            "gauge"
        };
        // Prometheus and OpenMetrics counter TYPE family spellings; these are
        // declaration spellings, not aliases for accepted sample names.
        let direct = types.get(spec.metric).copied();
        let family_name = spec.histogram.map(|h| h.family).or_else(|| {
            (spec.kind == MetricType::Counter)
                .then(|| spec.metric.strip_suffix("_total"))
                .flatten()
        });
        let family = family_name.and_then(|base| types.get(base).copied());
        let declaration = match (direct, family) {
            (Some(a), Some(b)) if a != b => Some(None),
            (Some(a), _) => Some(a),
            (_, b) => b,
        };
        let direct_unit = units.get(spec.metric).copied();
        let family_unit = family_name.and_then(|base| units.get(base).copied());
        let declared_unit = match (direct_unit, family_unit) {
            (Some(a), Some(b)) if a != b => Some(None),
            (Some(a), _) => Some(a),
            (_, b) => b,
        };
        let unknown = if declared_revision != Some(profile.source_revision()) {
            Some(Unknown::RevisionNotDeclaredOrUnsupported)
        } else if !missing_required_metrics.is_empty() {
            Some(Unknown::MissingRequired)
        } else if declaration != Some(Some(expected_type)) {
            Some(Unknown::TypeNotDeclaredOrConflicting)
        } else if declared_unit.is_some_and(|unit| {
            unit != Some(spec.histogram.map_or(spec.unit, |h| h.unit).declaration())
        }) {
            Some(Unknown::UnitNotCompatible)
        } else if (spec.kind == MetricType::HistogramBucket
            && !sample.labels.get("le").is_some_and(|le| {
                le == "+Inf"
                    || le
                        .parse::<f64>()
                        .is_ok_and(|bound| bound.is_finite() && bound >= 0.0)
            }))
            || sample.labels.len() > 16
            || sample
                .labels
                .iter()
                .any(|(k, v)| k.len() > 128 || v.len() > 256)
            || (spec.metric == "sglang:cached_tokens_total"
                && !matches!(
                    sample.labels.get("cache_source").map(String::as_str),
                    Some("device" | "host" | "storage" | "total")
                ))
        {
            Some(Unknown::InvalidLabels)
        } else if !sample.value.is_finite()
            || sample.value < 0.0
            || (spec.unit == Unit::Ratio && sample.value > 1.0)
            || (spec.unit != Unit::Seconds
                && spec.unit != Unit::Ratio
                && sample.value.fract() != 0.0)
        {
            Some(Unknown::InvalidValue)
        } else if matches!(spec.kind, MetricType::Counter | MetricType::HistogramBucket)
            && sample.value >= MAX_EXACT_INTEGER
        {
            Some(Unknown::CounterPrecision)
        } else {
            None
        };
        identities
            .entry((sample.metric, sample.labels.clone()))
            .or_default()
            .push(observations.len());
        observations.push(Observation {
            spec,
            labels: sample.labels,
            raw_value: sample.value,
            value: unknown.is_none().then_some(sample.value),
            unknown,
        });
    }
    for positions in identities.values().filter(|positions| positions.len() > 1) {
        for &index in positions {
            observations[index].value = None;
            observations[index].unknown = Some(Unknown::DuplicateIdentity);
        }
    }
    Ok(Probe {
        schema_version: SCHEMA_VERSION,
        profile,
        observations,
        missing_metrics,
        missing_required_metrics,
        unmatched_samples,
        malformed_lines: parsed.malformed_lines,
    })
}
