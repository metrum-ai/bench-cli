// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Opt-in bounded native-metric capture and deliberate public projection. No
//! requests by default; no labels, endpoint URLs or raw invalid values exported.

use crate::cache_metrics::{
    probe, HistogramPart, HistogramSpec, MetricSpec, Probe, ProfileId, Role, Unit, Unknown,
};
use crate::telemetry::MetricType;
use serde::Serialize;

const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(clap::Args, Clone, Default)]
pub struct NativeMetricArgs {
    #[arg(
        long,
        requires = "cache_metrics_profile",
        help = "Opt-in native metric discovery endpoint; unauthenticated bounded GET outside measurement"
    )]
    pub cache_metrics_url: Option<String>,
    #[arg(
        long,
        value_enum,
        requires = "cache_metrics_url",
        help = "Explicit source-pinned native profile; not backend autodetection"
    )]
    pub cache_metrics_profile: Option<ProfileId>,
    #[arg(
        long,
        requires = "cache_metrics_profile",
        help = "Operator-declared exact source SHA; absent/unsupported revisions retain Unknown values"
    )]
    pub cache_metrics_source_revision: Option<String>,
}

impl std::fmt::Debug for NativeMetricArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeMetricArgs")
            .field("configured", &self.cache_metrics_url.is_some())
            .field("profile", &self.cache_metrics_profile)
            .finish_non_exhaustive()
    }
}

impl NativeMetricArgs {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.cache_metrics_url.is_some() == self.cache_metrics_profile.is_some(),
            "cache metric discovery requires both endpoint and profile"
        );
        anyhow::ensure!(
            self.cache_metrics_source_revision.is_none() || self.cache_metrics_profile.is_some(),
            "cache metric source revision requires a profile"
        );
        if let Some(revision) = &self.cache_metrics_source_revision {
            anyhow::ensure!(
                revision.len() == 40 && revision.bytes().all(|b| b.is_ascii_hexdigit()),
                "cache metric source revision must be a full source SHA"
            );
        }
        if let Some(url) = &self.cache_metrics_url {
            let valid = reqwest::Url::parse(url).ok().is_some_and(|url| {
                matches!(url.scheme(), "http" | "https")
                    && url.username().is_empty()
                    && url.password().is_none()
            });
            anyhow::ensure!(
                valid,
                "cache metric endpoint requires HTTP(S) without URL credentials"
            );
        }
        Ok(())
    }
    pub async fn capture_if_enabled(&self) -> anyhow::Result<Option<NativeMetricCapture>> {
        self.validate()?;
        match (&self.cache_metrics_url, self.cache_metrics_profile) {
            (Some(url), Some(profile)) => Ok(Some(
                capture(url, profile, self.cache_metrics_source_revision.as_deref()).await,
            )),
            _ => Ok(None),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureFailure {
    Fetch,
    HttpStatus,
    TooLarge,
    InvalidUtf8,
    ProbeBudget,
    Deadline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclaredSourceStatus {
    NotDeclared,
    MatchesReference,
    Unsupported,
}

#[derive(Serialize)]
pub struct MetricCapability {
    pub metric: &'static str,
    pub expected_family_type: &'static str,
    pub unit: Unit,
    pub role: Role,
    pub required_for_profile: bool,
    pub cumulative: bool,
    pub histogram: Option<HistogramSpec>,
}

#[derive(Serialize)]
pub struct ProfileCapabilities {
    pub schema_version: &'static str,
    pub profile: ProfileId,
    pub reference_source_revision: &'static str,
    pub operator_declared_source_status: DeclaredSourceStatus,
    pub release_aliases: Vec<&'static str>,
    pub release_restriction: Option<&'static str>,
    pub complete_profile_support: bool,
    pub executable_version_verified: bool,
    pub physical_tier_validation: bool,
    pub phase_delta_validation: bool,
    pub histogram_distribution_validation: bool,
    pub quantile_reconstruction: bool,
    pub unmapped_engines: Vec<&'static str>,
    pub semantic_qualifiers: Vec<&'static str>,
    pub descriptors: Vec<MetricCapability>,
}

fn expected_family_type(spec: &MetricSpec) -> &'static str {
    if spec.histogram.is_some() {
        "histogram"
    } else {
        match spec.kind {
            MetricType::Counter => "counter",
            MetricType::Gauge => "gauge",
            _ => "unsupported",
        }
    }
}

impl ProfileCapabilities {
    pub fn new(profile: ProfileId, revision: Option<&str>) -> Self {
        Self {
            schema_version: "cache-metric-capabilities.v1",
            profile,
            reference_source_revision: profile.source_revision(),
            operator_declared_source_status: match revision {
                None => DeclaredSourceStatus::NotDeclared,
                Some(revision) if revision == profile.source_revision() => {
                    DeclaredSourceStatus::MatchesReference
                }
                _ => DeclaredSourceStatus::Unsupported,
            },
            release_aliases: Vec::new(),
            release_restriction: matches!(profile, ProfileId::DynamoKvbm)
                .then_some("legacy Dynamo <=1.6.x only; later releases do not ship KVBM"),
            complete_profile_support: false,
            executable_version_verified: false,
            physical_tier_validation: false,
            phase_delta_validation: false,
            histogram_distribution_validation: false,
            quantile_reconstruction: false,
            unmapped_engines: Vec::new(),
            semantic_qualifiers: if profile == ProfileId::TrtLlm {
                vec![
                "source-pinned TRT-LLM collector slice; not Triton backend metric aliases",
                "connector reuse and connector-attributed tokens count accepted loads before async completion",
                "manager-counter decreases update exporter baseline without decrement; monotonic export is not cache-manager continuity proof",
                "manager/iteration/total/tier views overlap; never sum them",
                "prefetched disk pages count as host attribution; attribution is not physical-tier participation",
                "block and tokens-per-block gauges are not sufficient for byte conversion",
            ]
            } else {
                Vec::new()
            },
            descriptors: profile
                .metrics()
                .iter()
                .map(|spec| MetricCapability {
                    metric: spec.metric,
                    expected_family_type: expected_family_type(spec),
                    unit: spec.unit,
                    role: spec.role,
                    required_for_profile: spec.required,
                    cumulative: matches!(
                        spec.kind,
                        MetricType::Counter | MetricType::HistogramBucket
                    ),
                    histogram: spec.histogram,
                })
                .collect(),
        }
    }
}

#[derive(Serialize)]
pub struct PublicMetricSample {
    pub series_id: usize,
    pub metric: &'static str,
    pub expected_family_type: &'static str,
    pub histogram_part: Option<HistogramPart>,
    pub unit: Unit,
    pub role: Role,
    pub value: Option<f64>,
    pub unknown: Option<Unknown>,
    /// Only validated, non-sensitive semantic dimensions are projected.
    pub cache_source: Option<String>,
    pub bucket_upper_bound: Option<String>,
}

#[derive(Serialize)]
pub struct NativeMetricCapture {
    pub schema_version: &'static str,
    pub profile: ProfileId,
    pub reference_source_revision: &'static str,
    pub capabilities: ProfileCapabilities,
    pub declared_source_revision: Option<String>,
    pub version_evidence: &'static str,
    pub series_id_scope: &'static str,
    pub attribution: &'static str,
    pub fetch_started_unix_ms: u64,
    pub fetch_finished_unix_ms: u64,
    pub failure: Option<CaptureFailure>,
    pub missing_required_metrics: Vec<&'static str>,
    pub missing_metrics: Vec<&'static str>,
    pub samples: Vec<PublicMetricSample>,
    #[serde(skip)]
    raw: Option<Probe>,
}

impl NativeMetricCapture {
    fn empty(profile: ProfileId, revision: Option<&str>) -> Self {
        Self {
            schema_version: "cache-metric-capture.v1",
            profile,
            reference_source_revision: profile.source_revision(),
            capabilities: ProfileCapabilities::new(profile, revision),
            // Arbitrary operator text is not public version provenance.
            declared_source_revision: revision
                .filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                .map(str::to_owned),
            version_evidence: "operator_declared_source_revision_not_executable_attestation",
            series_id_scope:
                "this_snapshot_only; private_full_labels; no_replica_or_tier_summation",
            attribution: "reported_endpoint_metrics_not_request_or_physical_cache_proof",
            fetch_started_unix_ms: now_ms(),
            fetch_finished_unix_ms: 0,
            failure: None,
            missing_required_metrics: Vec::new(),
            missing_metrics: Vec::new(),
            samples: Vec::new(),
            raw: None,
        }
    }
    /// Private identities for subsequent same-source phase matching, never part
    /// of the Serialize projection. Callers must not publish/debug-dump labels.
    pub fn private_probe(&self) -> Option<&Probe> {
        self.raw.as_ref()
    }
    pub fn from_probe(result: Probe, revision: Option<&str>) -> Self {
        let mut report = Self::empty(result.profile, revision);
        report.missing_metrics = result.missing_metrics.clone();
        report.missing_required_metrics = result.missing_required_metrics.clone();
        report.samples = result
            .observations
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let valid = row.unknown.is_none();
                PublicMetricSample {
                    series_id: index + 1,
                    metric: row.spec.metric,
                    expected_family_type: expected_family_type(&row.spec),
                    histogram_part: row.spec.histogram.map(|h| h.part),
                    unit: row.spec.unit,
                    role: row.spec.role,
                    value: row.value,
                    unknown: row.unknown,
                    cache_source: if valid && row.spec.metric == "sglang:cached_tokens_total" {
                        row.labels.get("cache_source").cloned()
                    } else {
                        None
                    },
                    bucket_upper_bound: if valid && row.spec.kind == MetricType::HistogramBucket {
                        row.labels.get("le").cloned()
                    } else {
                        None
                    },
                }
            })
            .collect();
        report.raw = Some(result);
        report.fetch_finished_unix_ms = now_ms();
        report
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

/// The caller deliberately selects this endpoint/profile. GET sends no workload
/// body or auth inherited from inference requests. Redirects are disabled. Bounds
/// and deadline apply to the entire fetch; unavailable evidence never becomes 0.
pub async fn capture(url: &str, profile: ProfileId, revision: Option<&str>) -> NativeMetricCapture {
    let mut report = NativeMetricCapture::empty(profile, revision);
    let started = report.fetch_started_unix_ms;
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| CaptureFailure::Fetch)?;
        let url = reqwest::Url::parse(url).map_err(|_| CaptureFailure::Fetch)?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(CaptureFailure::Fetch);
        }
        let mut response = client
            .get(url)
            .header(reqwest::header::ACCEPT, "text/plain")
            .send()
            .await
            .map_err(|_| CaptureFailure::Fetch)?;
        if !response.status().is_success() {
            return Err(CaptureFailure::HttpStatus);
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_BYTES as u64)
        {
            return Err(CaptureFailure::TooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| CaptureFailure::Fetch)? {
            if chunk.len() > MAX_BYTES.saturating_sub(bytes.len()) {
                return Err(CaptureFailure::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| CaptureFailure::InvalidUtf8)?;
        probe(profile, revision, text).map_err(|_| CaptureFailure::ProbeBudget)
    })
    .await;
    match result {
        Ok(Ok(probe)) => {
            report = NativeMetricCapture::from_probe(probe, revision);
            report.fetch_started_unix_ms = started;
        }
        Ok(Err(reason)) => report.failure = Some(reason),
        Err(_) => report.failure = Some(CaptureFailure::Deadline),
    }
    report.fetch_finished_unix_ms = now_ms();
    report
}
