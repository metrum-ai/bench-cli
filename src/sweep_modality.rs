// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Per-request modality values and their stage aggregates for the strategic
//! `--kind vlm`, `--kind asr` and `--kind imagegen` sweeps (#197). Metrum AI.
//!
//! Keys match the `modality_metrics` names on the modality binaries'
//! `request.v3` records, so a sweep point and a single run read the same.
//! Values are only recorded when measured: a request without a reference
//! transcript has no `wer`, a sample without a duration has no
//! `rtfx_client`, and the stage summary for that key is then `n = 0`.

use crate::stats::DistSummary;
use crate::strategic::BenchRecord;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Stage keys for `--kind vlm`: images per request and their payload bytes.
pub const VLM_KEYS: &[&str] = &["image_bytes", "image_count"];
/// Stage keys for `--kind asr`.
pub const ASR_KEYS: &[&str] = &["audio_duration_s", "cer", "rtfx_client", "wer"];
/// Stage keys for `--kind imagegen`.
pub const IMAGEGEN_KEYS: &[&str] = &["images_requested", "images_returned"];

/// Modality values for one request, kept beside its [`BenchRecord`] (the
/// strategic CSV columns stay unchanged).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModalitySample {
    pub metrics: BTreeMap<String, f64>,
    /// SHA-256 of each decoded `b64_json` image.
    pub image_sha256: Vec<String>,
}

impl ModalitySample {
    /// VLM request: image count and total image payload bytes sent.
    pub fn vlm(image_count: usize, image_bytes: u64) -> Self {
        let mut sample = Self::default();
        sample.put("image_count", image_count as f64);
        sample.put("image_bytes", image_bytes as f64);
        sample
    }

    /// ASR request. WER/CER only with a reference; `rtfx_client` (audio
    /// seconds over client send-to-completion seconds) only with a duration.
    pub fn asr(
        transcript: &str,
        reference: Option<&str>,
        normalizer: crate::asr::Normalizer,
        audio_seconds: Option<f64>,
        service_latency_s: f64,
    ) -> Self {
        let mut sample = Self::default();
        if let Some(reference) = reference {
            if let Some(wer) = crate::asr::word_error_rate(reference, transcript, normalizer) {
                sample.put("wer", wer);
            }
            if let Some(cer) = crate::asr::character_error_rate(reference, transcript, normalizer) {
                sample.put("cer", cer);
            }
        }
        if let Some(seconds) = audio_seconds.filter(|s| s.is_finite() && *s > 0.0) {
            sample.put("audio_duration_s", seconds);
            if let Some(rtfx) = crate::asr::rtfx(seconds, service_latency_s) {
                sample.put("rtfx_client", rtfx);
            }
        }
        sample
    }

    /// Image generation request: requested and returned image counts plus
    /// the digest of every decoded image.
    pub fn imagegen(requested: u32, returned: usize, image_sha256: Vec<String>) -> Self {
        let mut sample = Self::default();
        sample.put("images_requested", f64::from(requested));
        sample.put("images_returned", returned as f64);
        sample.image_sha256 = image_sha256;
        sample
    }

    fn put(&mut self, key: &str, value: f64) {
        if value.is_finite() {
            self.metrics.insert(key.to_string(), value);
        }
    }
}

/// Stage image digest counts for `--kind imagegen`. Both are `0` when no
/// image was decoded (for example `url` responses).
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ImageDigests {
    /// Decoded images over measured successes.
    pub images: usize,
    /// Distinct SHA-256 digests among them.
    pub distinct: usize,
}

fn measured_successes<'a>(
    records: &'a [BenchRecord],
    samples: &'a [ModalitySample],
) -> impl Iterator<Item = &'a ModalitySample> {
    records
        .iter()
        .zip(samples)
        .filter(|(record, _)| !record.warmup && record.success)
        .map(|(_, sample)| sample)
}

/// One distribution per key over measured successes (warmup and failures
/// excluded, like every other stage aggregate). Rows without a key add no
/// sample, so an unmeasured key is `n = 0`. `records` and `samples` are
/// index-aligned.
pub fn stage_metrics(
    keys: &[&str],
    records: &[BenchRecord],
    samples: &[ModalitySample],
) -> BTreeMap<String, DistSummary> {
    keys.iter()
        .map(|key| {
            let values: Vec<f64> = measured_successes(records, samples)
                .filter_map(|sample| sample.metrics.get(*key).copied())
                .collect();
            (key.to_string(), DistSummary::from_values(&values))
        })
        .collect()
}

/// Decoded images and distinct digests over measured successes.
pub fn stage_image_digests(records: &[BenchRecord], samples: &[ModalitySample]) -> ImageDigests {
    let digests: Vec<&String> = measured_successes(records, samples)
        .flat_map(|sample| sample.image_sha256.iter())
        .collect();
    ImageDigests {
        images: digests.len(),
        distinct: digests.iter().collect::<BTreeSet<_>>().len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(seq: u64, success: bool, warmup: bool) -> BenchRecord {
        BenchRecord {
            seq,
            success,
            warmup,
            ..BenchRecord::default()
        }
    }

    #[test]
    fn asr_sample_skips_unmeasured_values() {
        let none = ModalitySample::asr(
            "dummy transcription",
            None,
            crate::asr::Normalizer::default(),
            None,
            0.5,
        );
        assert!(none.metrics.is_empty());
        let full = ModalitySample::asr(
            "dummy transcription",
            Some("Dummy transcription."),
            crate::asr::Normalizer::default(),
            Some(2.0),
            0.5,
        );
        assert_eq!(full.metrics["wer"], 0.0);
        assert_eq!(full.metrics["cer"], 0.0);
        assert_eq!(full.metrics["audio_duration_s"], 2.0);
        assert_eq!(full.metrics["rtfx_client"], 4.0);
    }

    #[test]
    fn stage_metrics_use_measured_successes_only() {
        let records = vec![
            record(0, true, true),
            record(1, true, false),
            record(2, false, false),
            record(3, true, false),
        ];
        let samples = vec![
            ModalitySample::vlm(1, 900),
            ModalitySample::vlm(1, 100),
            ModalitySample::vlm(1, 900),
            ModalitySample::vlm(2, 300),
        ];
        let stage = stage_metrics(VLM_KEYS, &records, &samples);
        assert_eq!(stage["image_bytes"].n, 2);
        assert_eq!(stage["image_bytes"].max, Some(300.0));
        assert_eq!(stage["image_count"].min, Some(1.0));
        let asr = stage_metrics(ASR_KEYS, &records, &samples);
        assert!(asr.values().all(|dist| dist.n == 0), "unmeasured keys n=0");
        assert_eq!(asr.len(), ASR_KEYS.len());
    }

    #[test]
    fn digests_count_distinct_images() {
        let records = vec![
            record(0, true, false),
            record(1, true, false),
            record(2, false, false),
        ];
        let samples = vec![
            ModalitySample::imagegen(2, 2, vec!["a".into(), "b".into()]),
            ModalitySample::imagegen(1, 1, vec!["a".into()]),
            ModalitySample::imagegen(1, 1, vec!["c".into()]),
        ];
        assert_eq!(
            stage_image_digests(&records, &samples),
            ImageDigests {
                images: 3,
                distinct: 2
            }
        );
        assert_eq!(stage_image_digests(&records, &[]), ImageDigests::default());
    }
}
