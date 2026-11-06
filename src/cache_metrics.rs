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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileId {
    VllmApc,
    VllmExternal,
    Sglang,
    Lmcache,
    DynamoKvbm,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Tokens,
    Requests,
    Bytes,
    Blocks,
    Ratio,
}
impl Unit {
    fn declaration(self) -> &'static str {
        match self {
            Self::Tokens => "tokens",
            Self::Requests => "requests",
            Self::Bytes => "bytes",
            Self::Blocks => "blocks",
            Self::Ratio => "ratio",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}
#[derive(Debug, Clone, Copy)]
pub struct MetricSpec {
    pub metric: &'static str,
    pub kind: MetricType,
    pub unit: Unit,
    pub role: Role,
    pub required: bool,
}
const fn c(metric: &'static str, unit: Unit, role: Role) -> MetricSpec {
    MetricSpec {
        metric,
        kind: MetricType::Counter,
        unit,
        role,
        required: true,
    }
}
const fn g(metric: &'static str, role: Role) -> MetricSpec {
    MetricSpec {
        metric,
        kind: MetricType::Gauge,
        unit: Unit::Ratio,
        role,
        required: false,
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
const SGLANG: &[MetricSpec] = &[
    c("sglang:cached_tokens_total", Unit::Tokens, Role::Hit),
    g("sglang:cache_hit_rate", Role::ReportedHitRate),
];
const LMCACHE: &[MetricSpec] = &[
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
];
const KVBM: &[MetricSpec] = &[
    c("kvbm_offload_blocks_d2h", Unit::Blocks, Role::Transfer),
    c("kvbm_onboard_blocks_h2d", Unit::Blocks, Role::Transfer),
    c("kvbm_matched_tokens", Unit::Tokens, Role::Matched),
    g("kvbm_host_cache_hit_rate", Role::ReportedHitRate),
    g("kvbm_disk_cache_hit_rate", Role::ReportedHitRate),
    g("kvbm_object_cache_hit_rate", Role::ReportedHitRate),
];

impl ProfileId {
    pub fn source_revision(self) -> &'static str {
        match self {
            Self::VllmApc | Self::VllmExternal => "aa4022dc8176db8f769f93445d845f8755f9b4ca",
            Self::Sglang => "188cf7da66938ca2e1ea9cbf7ebbf9a2c81edc2a",
            Self::Lmcache => "34f8a409e3631eff51d0b6fff67ca9c68f123377",
            Self::DynamoKvbm => "fa9852014950ed0aea74e56e1de441fc6de15307",
        }
    }
    pub fn metrics(self) -> &'static [MetricSpec] {
        match self {
            Self::VllmApc => VLLM_APC,
            Self::VllmExternal => VLLM_EXTERNAL,
            Self::Sglang => SGLANG,
            Self::Lmcache => LMCACHE,
            Self::DynamoKvbm => KVBM,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    static SGLANG_MATCHER: OnceLock<RegexSet> = OnceLock::new();
    static LMCACHE_MATCHER: OnceLock<RegexSet> = OnceLock::new();
    static KVBM_MATCHER: OnceLock<RegexSet> = OnceLock::new();
    let matcher = match profile {
        ProfileId::VllmApc => &APC,
        ProfileId::VllmExternal => &EXTERNAL,
        ProfileId::Sglang => &SGLANG_MATCHER,
        ProfileId::Lmcache => &LMCACHE_MATCHER,
        ProfileId::DynamoKvbm => &KVBM_MATCHER,
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
                || seen_samples.contains(format!("{}_total", parts[0]).as_str())
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
        let expected_type = if spec.kind == MetricType::Counter {
            "counter"
        } else {
            "gauge"
        };
        // Prometheus and OpenMetrics counter TYPE family spellings; these are
        // declaration spellings, not aliases for accepted sample names.
        let direct = types.get(spec.metric).copied();
        let family = (spec.kind == MetricType::Counter)
            .then(|| spec.metric.strip_suffix("_total"))
            .flatten()
            .and_then(|base| types.get(base).copied());
        let declaration = match (direct, family) {
            (Some(a), Some(b)) if a != b => Some(None),
            (Some(a), _) => Some(a),
            (_, b) => b,
        };
        let direct_unit = units.get(spec.metric).copied();
        let family_unit = (spec.kind == MetricType::Counter)
            .then(|| spec.metric.strip_suffix("_total"))
            .flatten()
            .and_then(|base| units.get(base).copied());
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
        } else if declared_unit.is_some_and(|unit| unit != Some(spec.unit.declaration())) {
            Some(Unknown::UnitNotCompatible)
        } else if sample.labels.len() > 16
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
            || (spec.kind == MetricType::Counter && sample.value.fract() != 0.0)
        {
            Some(Unknown::InvalidValue)
        } else if spec.kind == MetricType::Counter && sample.value >= MAX_EXACT_INTEGER {
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
