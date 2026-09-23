// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Workload mix selection from the Metrum AI prompt library.
//!
//! Selects a reproducible multiset of rows whose ISL/OSL mean or median land
//! inside CLI absolute tolerances. Preferred `--count` is a soft target;
//! repeats are a solver operator.

use anyhow::{anyhow, bail, Context, Result};
use clap::ValueEnum;
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::RowAccessor;
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path, PathBuf};

pub const HINT_TEMPLATE: &str =
    "\n\nPlease aim for approximately {target_output_length} words in your response.";
pub const REPORT_SCHEMA_VERSION: &str = "metrum-ai-bench-cli.prompt-mix.v1";
pub const DEFAULT_DATASET: &str = "metrum-ai/prompt-library";

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    Words,
    Tokens,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LengthStat {
    Mean,
    Median,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningFilter {
    Any,
    True,
    False,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IslTokenBasis {
    SuppliedTarget,
}

/// Named, versioned ISL/OSL workload profiles for publishable compares.
pub const WORKLOAD_PROFILE_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkloadProfile {
    /// Short interactive chat smoke (256 / 64 tokens).
    ChatShort,
    /// Default publishable chat (512 / 128 tokens).
    ChatMedium,
    /// Retrieval-augmented generation (2048 / 256 tokens).
    RagMedium,
    /// Long-context summarization (4096 / 512 tokens).
    SummarizeLong,
    /// Coding-assistant turns (1024 / 512 tokens).
    CodeMedium,
}

/// Fixed targets and recommended absolute tolerances for a [`WorkloadProfile`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkloadProfileSpec {
    pub name: &'static str,
    pub version: u32,
    pub isl_target: f64,
    pub osl_target: f64,
    pub isl_tolerance: f64,
    pub osl_tolerance: f64,
}

impl WorkloadProfile {
    pub fn spec(self) -> WorkloadProfileSpec {
        match self {
            Self::ChatShort => WorkloadProfileSpec {
                name: "chat-short",
                version: WORKLOAD_PROFILE_VERSION,
                isl_target: 256.0,
                osl_target: 64.0,
                isl_tolerance: 32.0,
                osl_tolerance: 16.0,
            },
            Self::ChatMedium => WorkloadProfileSpec {
                name: "chat-medium",
                version: WORKLOAD_PROFILE_VERSION,
                isl_target: 512.0,
                osl_target: 128.0,
                isl_tolerance: 64.0,
                osl_tolerance: 32.0,
            },
            Self::RagMedium => WorkloadProfileSpec {
                name: "rag-medium",
                version: WORKLOAD_PROFILE_VERSION,
                isl_target: 2048.0,
                osl_target: 256.0,
                isl_tolerance: 128.0,
                osl_tolerance: 64.0,
            },
            Self::SummarizeLong => WorkloadProfileSpec {
                name: "summarize-long",
                version: WORKLOAD_PROFILE_VERSION,
                isl_target: 4096.0,
                osl_target: 512.0,
                isl_tolerance: 256.0,
                osl_tolerance: 64.0,
            },
            Self::CodeMedium => WorkloadProfileSpec {
                name: "code-medium",
                version: WORKLOAD_PROFILE_VERSION,
                isl_target: 1024.0,
                osl_target: 512.0,
                isl_tolerance: 128.0,
                osl_tolerance: 64.0,
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct LibraryRow {
    pub ordinal: u64,
    pub prompt: String,
    pub target_output_length: i64,
    pub reasoning: bool,
    pub target_input_tokens: i64,
    pub target_output_tokens: i64,
}

impl LibraryRow {
    pub fn rendered_prompt(&self) -> String {
        format!(
            "{}{}",
            self.prompt,
            HINT_TEMPLATE.replace(
                "{target_output_length}",
                &self.target_output_length.to_string()
            )
        )
    }

    /// Python `str.split()`-compatible word count (Unicode whitespace).
    pub fn word_count(text: &str) -> u64 {
        text.split_whitespace().count() as u64
    }

    pub fn isl(&self, unit: LengthUnit, basis: IslTokenBasis) -> f64 {
        match unit {
            LengthUnit::Words => Self::word_count(&self.rendered_prompt()) as f64,
            LengthUnit::Tokens => match basis {
                IslTokenBasis::SuppliedTarget => self.target_input_tokens as f64,
            },
        }
    }

    pub fn osl(&self, unit: LengthUnit) -> f64 {
        match unit {
            LengthUnit::Words => self.target_output_length as f64,
            LengthUnit::Tokens => self.target_output_tokens as f64,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SelectRequest {
    pub count: usize,
    pub count_slack: usize,
    pub seed: u64,
    pub isl_target: f64,
    pub isl_unit: LengthUnit,
    pub isl_stat: LengthStat,
    pub isl_tolerance: f64,
    pub osl_target: f64,
    pub osl_unit: LengthUnit,
    pub osl_stat: LengthStat,
    pub osl_tolerance: f64,
    pub isl_token_basis: IslTokenBasis,
    pub reasoning: ReasoningFilter,
    pub max_repeats: usize,
    pub work_limit: u32,
    pub osl_tokens_per_word: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct SelectedMix {
    pub rows: Vec<LibraryRow>,
    pub isl_achieved: f64,
    pub osl_achieved: f64,
    pub isl_gap: f64,
    pub osl_gap: f64,
    pub extra_copies: usize,
    pub work_used: u32,
}

#[derive(Clone, Debug)]
pub struct SelectFailure {
    pub message: String,
    pub isl_achieved: Option<f64>,
    pub osl_achieved: Option<f64>,
    pub isl_gap: Option<f64>,
    pub osl_gap: Option<f64>,
    pub selected_count: usize,
    pub candidate_count: usize,
    pub work_used: u32,
    pub proven_empty: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Score {
    excess: f64,
    count_dev: usize,
    extra_copies: usize,
}

impl Score {
    fn better_than(self, other: Score) -> bool {
        if (self.excess - other.excess).abs() > 1e-12 {
            return self.excess < other.excess;
        }
        if self.count_dev != other.count_dev {
            return self.count_dev < other.count_dev;
        }
        self.extra_copies < other.extra_copies
    }
}

pub fn mix_mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<f64>() / values.len() as f64)
}

/// Even-n median is the arithmetic mean of the two central values.
pub fn mix_median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    if n % 2 == 1 {
        Some(sorted[n / 2])
    } else {
        Some((sorted[n / 2 - 1] + sorted[n / 2]) / 2.0)
    }
}

fn apply_stat(values: &[f64], stat: LengthStat) -> Option<f64> {
    match stat {
        LengthStat::Mean => mix_mean(values),
        LengthStat::Median => mix_median(values),
    }
}

fn count_extra_copies(mix: &[usize]) -> usize {
    let mut hist: HashMap<usize, usize> = HashMap::new();
    for &i in mix {
        *hist.entry(i).or_insert(0) += 1;
    }
    hist.values().map(|c| c.saturating_sub(1)).sum()
}

fn multiplicity_ok(mix: &[usize], max_repeats: usize) -> bool {
    let mut hist: HashMap<usize, usize> = HashMap::new();
    for &i in mix {
        let next = hist.entry(i).or_insert(0);
        *next += 1;
        if *next > max_repeats {
            return false;
        }
    }
    true
}

struct Cand {
    row: LibraryRow,
    isl: f64,
    osl: f64,
}

fn evaluate(
    mix: &[usize],
    cands: &[Cand],
    req: &SelectRequest,
) -> Option<(Score, f64, f64, f64, f64)> {
    if mix.is_empty() {
        return None;
    }
    let isl: Vec<f64> = mix.iter().map(|&i| cands[i].isl).collect();
    let osl: Vec<f64> = mix.iter().map(|&i| cands[i].osl).collect();
    let isl_achieved = apply_stat(&isl, req.isl_stat)?;
    let osl_achieved = apply_stat(&osl, req.osl_stat)?;
    let isl_gap = isl_achieved - req.isl_target;
    let osl_gap = osl_achieved - req.osl_target;
    let excess =
        (isl_gap.abs() - req.isl_tolerance).max(0.0) + (osl_gap.abs() - req.osl_tolerance).max(0.0);
    let score = Score {
        excess,
        count_dev: mix.len().abs_diff(req.count),
        extra_copies: count_extra_copies(mix),
    };
    Some((score, isl_achieved, osl_achieved, isl_gap, osl_gap))
}

fn filter_candidates(rows: &[LibraryRow], req: &SelectRequest) -> Vec<Cand> {
    rows.iter()
        .filter(|row| match req.reasoning {
            ReasoningFilter::Any => true,
            ReasoningFilter::True => row.reasoning,
            ReasoningFilter::False => !row.reasoning,
        })
        .map(|row| Cand {
            isl: row.isl(req.isl_unit, req.isl_token_basis),
            osl: row.osl(req.osl_unit),
            row: row.clone(),
        })
        .collect()
}

fn seed_mix(cands: &[Cand], req: &SelectRequest, rng: &mut StdRng) -> Vec<usize> {
    let n = cands.len();
    let target_len = req
        .count
        .clamp(1, req.count.saturating_add(req.count_slack));
    let mut mix = Vec::with_capacity(target_len);
    let mut used = vec![0usize; n];
    let mut unused: Vec<usize> = (0..n).collect();
    while mix.len() < target_len {
        if !unused.is_empty() {
            let pick = rng.random_range(0..unused.len());
            let idx = unused.swap_remove(pick);
            mix.push(idx);
            used[idx] += 1;
            continue;
        }
        let idx = rng.random_range(0..n);
        if used[idx] >= req.max_repeats {
            if used.iter().all(|&c| c >= req.max_repeats) {
                break;
            }
            continue;
        }
        mix.push(idx);
        used[idx] += 1;
    }
    if mix.is_empty() {
        mix.push(0);
    }
    mix
}

fn try_move(mix: &[usize], cands: &[Cand], req: &SelectRequest, rng: &mut StdRng) -> Vec<usize> {
    let n = cands.len();
    let min_len = req.count.saturating_sub(req.count_slack).max(1);
    let max_len = req.count.saturating_add(req.count_slack).max(min_len);
    let mut trial = mix.to_vec();
    match rng.random_range(0..5) {
        0 => {
            let slot = rng.random_range(0..trial.len());
            trial[slot] = rng.random_range(0..n);
        }
        1 if trial.len() < max_len => {
            let src = trial[rng.random_range(0..trial.len())];
            trial.push(src);
        }
        2 if trial.len() < max_len => {
            trial.push(rng.random_range(0..n));
        }
        3 if trial.len() > min_len => {
            let slot = rng.random_range(0..trial.len());
            trial.swap_remove(slot);
        }
        _ => {
            let slot = rng.random_range(0..trial.len());
            let nearest = (0..n)
                .min_by(|&a, &b| {
                    let da = (cands[a].isl - req.isl_target).abs()
                        + (cands[a].osl - req.osl_target).abs();
                    let db = (cands[b].isl - req.isl_target).abs()
                        + (cands[b].osl - req.osl_target).abs();
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or(0);
            trial[slot] = nearest;
        }
    }
    trial
}

/// When an entire (ISL, OSL) cell already sits inside both tolerances, draw from
/// that cell directly: unique rows first, then bounded repeats. Falls through
/// when no constant cell can satisfy the request (mixed-value mean/median).
fn try_homogeneous_bucket(
    cands: &[Cand],
    req: &SelectRequest,
    rng: &mut StdRng,
) -> Option<SelectedMix> {
    struct HomogeneousPick {
        score: Score,
        n: usize,
        isl: f64,
        osl: f64,
        isl_gap: f64,
        osl_gap: f64,
        idxs: Vec<usize>,
    }

    let min_len = req.count.saturating_sub(req.count_slack).max(1);
    let max_len = req.count.saturating_add(req.count_slack).max(min_len);

    let mut groups: BTreeMap<(u64, u64), Vec<usize>> = BTreeMap::new();
    for (idx, cand) in cands.iter().enumerate() {
        groups
            .entry((cand.isl.to_bits(), cand.osl.to_bits()))
            .or_default()
            .push(idx);
    }

    let mut best: Option<HomogeneousPick> = None;
    for ((isl_bits, osl_bits), idxs) in groups {
        let isl = f64::from_bits(isl_bits);
        let osl = f64::from_bits(osl_bits);
        let isl_gap = isl - req.isl_target;
        let osl_gap = osl - req.osl_target;
        let excess = (isl_gap.abs() - req.isl_tolerance).max(0.0)
            + (osl_gap.abs() - req.osl_tolerance).max(0.0);
        if excess > 0.0 {
            continue;
        }
        let unique = idxs.len();
        let capacity = unique.saturating_mul(req.max_repeats);
        if capacity < min_len {
            continue;
        }
        let n = req.count.clamp(min_len, max_len.min(capacity));
        let unique_used = n.min(unique);
        let score = Score {
            excess: 0.0,
            count_dev: n.abs_diff(req.count),
            extra_copies: n.saturating_sub(unique_used),
        };
        let better = match &best {
            None => true,
            Some(best_pick) => score.better_than(best_pick.score),
        };
        if better {
            best = Some(HomogeneousPick {
                score,
                n,
                isl,
                osl,
                isl_gap,
                osl_gap,
                idxs,
            });
        }
    }

    let HomogeneousPick {
        score,
        n,
        isl,
        osl,
        isl_gap,
        osl_gap,
        mut idxs,
    } = best?;
    for i in (1..idxs.len()).rev() {
        let j = rng.random_range(0..=i);
        idxs.swap(i, j);
    }
    let mut mix = Vec::with_capacity(n);
    let mut used = vec![0usize; idxs.len()];
    for (slot, &idx) in idxs.iter().enumerate() {
        if mix.len() >= n {
            break;
        }
        mix.push(idx);
        used[slot] = 1;
    }
    while mix.len() < n {
        let mut progressed = false;
        for (slot, &idx) in idxs.iter().enumerate() {
            if mix.len() >= n {
                break;
            }
            if used[slot] < req.max_repeats {
                mix.push(idx);
                used[slot] += 1;
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    if mix.len() != n || !multiplicity_ok(&mix, req.max_repeats) {
        return None;
    }
    Some(SelectedMix {
        rows: mix.iter().map(|&i| cands[i].row.clone()).collect(),
        isl_achieved: isl,
        osl_achieved: osl,
        isl_gap,
        osl_gap,
        extra_copies: score.extra_copies,
        work_used: 0,
    })
}

pub fn select_mix(rows: &[LibraryRow], req: &SelectRequest) -> Result<SelectedMix, SelectFailure> {
    if req.count == 0 {
        return Err(SelectFailure {
            message: "--count must be at least 1".into(),
            isl_achieved: None,
            osl_achieved: None,
            isl_gap: None,
            osl_gap: None,
            selected_count: 0,
            candidate_count: 0,
            work_used: 0,
            proven_empty: true,
        });
    }
    if req.max_repeats == 0 {
        return Err(SelectFailure {
            message: "--max-repeats must be at least 1".into(),
            isl_achieved: None,
            osl_achieved: None,
            isl_gap: None,
            osl_gap: None,
            selected_count: 0,
            candidate_count: 0,
            work_used: 0,
            proven_empty: true,
        });
    }
    let cands = filter_candidates(rows, req);
    if cands.is_empty() {
        return Err(SelectFailure {
            message: "no rows remain after the reasoning filter".into(),
            isl_achieved: None,
            osl_achieved: None,
            isl_gap: None,
            osl_gap: None,
            selected_count: 0,
            candidate_count: 0,
            work_used: 0,
            proven_empty: true,
        });
    }
    let mut rng = StdRng::seed_from_u64(req.seed);
    if let Some(exact) = try_homogeneous_bucket(&cands, req, &mut rng) {
        return Ok(exact);
    }
    let mut mix = seed_mix(&cands, req, &mut rng);
    let mut best_mix = mix.clone();
    let mut best_eval = evaluate(&mix, &cands, req);
    let mut work_used = 0u32;
    for _ in 0..req.work_limit {
        work_used += 1;
        let trial = try_move(&mix, &cands, req, &mut rng);
        if !multiplicity_ok(&trial, req.max_repeats) {
            continue;
        }
        let Some(eval) = evaluate(&trial, &cands, req) else {
            continue;
        };
        let improve = match best_eval {
            None => true,
            Some(best) => eval.0.better_than(best.0),
        };
        if improve {
            mix = trial;
            best_mix = mix.clone();
            best_eval = Some(eval);
            if eval.0.excess == 0.0 && eval.0.count_dev == 0 && eval.0.extra_copies == 0 {
                break;
            }
        } else if rng.random::<f64>() < 0.02 {
            mix = trial;
        }
    }
    match best_eval {
        Some((score, isl_achieved, osl_achieved, isl_gap, osl_gap)) if score.excess == 0.0 => {
            Ok(SelectedMix {
                rows: best_mix.iter().map(|&i| cands[i].row.clone()).collect(),
                isl_achieved,
                osl_achieved,
                isl_gap,
                osl_gap,
                extra_copies: score.extra_copies,
                work_used,
            })
        }
        Some((_, isl_achieved, osl_achieved, isl_gap, osl_gap)) => Err(SelectFailure {
            message: format!(
                "no mix within ISL/OSL tolerances (best ISL gap {isl_gap:+.4}, OSL gap {osl_gap:+.4}, n={}, preferred {})",
                best_mix.len(),
                req.count
            ),
            isl_achieved: Some(isl_achieved),
            osl_achieved: Some(osl_achieved),
            isl_gap: Some(isl_gap),
            osl_gap: Some(osl_gap),
            selected_count: best_mix.len(),
            candidate_count: cands.len(),
            work_used,
            proven_empty: false,
        }),
        None => Err(SelectFailure {
            message: "selector produced an empty mix".into(),
            isl_achieved: None,
            osl_achieved: None,
            isl_gap: None,
            osl_gap: None,
            selected_count: 0,
            candidate_count: cands.len(),
            work_used,
            proven_empty: true,
        }),
    }
}

pub fn recommended_max_tokens(mix: &SelectedMix, req: &SelectRequest) -> Result<u32> {
    match req.osl_unit {
        LengthUnit::Tokens => {
            let max = mix
                .rows
                .iter()
                .map(|row| row.target_output_tokens)
                .max()
                .unwrap_or(1);
            Ok(u32::try_from(max.max(1)).unwrap_or(u32::MAX))
        }
        LengthUnit::Words => {
            let r = req.osl_tokens_per_word.ok_or_else(|| {
                anyhow!("--osl-tokens-per-word is required when --osl-unit words")
            })?;
            if !(r.is_finite() && r > 0.0) {
                bail!("--osl-tokens-per-word must be a positive finite number");
            }
            let max = mix
                .rows
                .iter()
                .map(|row| (row.target_output_length as f64 * r).ceil() as i64)
                .max()
                .unwrap_or(1);
            Ok(u32::try_from(max.max(1)).unwrap_or(u32::MAX))
        }
    }
}

pub fn write_jsonl(path: &Path, mix: &SelectedMix) -> Result<()> {
    let mut file = File::create(path).with_context(|| format!("create {}", path.display()))?;
    for (i, row) in mix.rows.iter().enumerate() {
        let rec = json!({
            "prompt": row.rendered_prompt(),
            "source_ordinal": row.ordinal,
            "target_output_tokens": row.target_output_tokens,
            "target_output_length": row.target_output_length,
            "slot": i,
        });
        serde_json::to_writer(&mut file, &rec)?;
        file.write_all(b"\n")?;
    }
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

pub fn mix_checksum(mix: &SelectedMix) -> String {
    let mut hasher = Sha256::new();
    for row in &mix.rows {
        hasher.update(row.ordinal.to_le_bytes());
        hasher.update(row.rendered_prompt().as_bytes());
        hasher.update(b"\n");
    }
    hex_digest(hasher.finalize().as_slice())
}

pub fn selection_report(
    dataset: &str,
    revision: &str,
    config: &str,
    split: &str,
    req: &SelectRequest,
    mix: &SelectedMix,
    recommended_max_tokens: u32,
) -> Result<Value> {
    selection_report_with_profile(
        dataset,
        revision,
        config,
        split,
        req,
        mix,
        recommended_max_tokens,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn selection_report_with_profile(
    dataset: &str,
    revision: &str,
    config: &str,
    split: &str,
    req: &SelectRequest,
    mix: &SelectedMix,
    recommended_max_tokens: u32,
    profile: Option<WorkloadProfileSpec>,
) -> Result<Value> {
    let mut hist: BTreeMap<String, usize> = BTreeMap::new();
    for row in &mix.rows {
        *hist.entry(row.ordinal.to_string()).or_insert(0) += 1;
    }
    let mut report = json!({
        "schema_version": REPORT_SCHEMA_VERSION,
        "dataset": dataset,
        "revision": revision,
        "config": config,
        "split": split,
        "seed": req.seed,
        "preferred_count": req.count,
        "selected_count": mix.rows.len(),
        "count_slack": req.count_slack,
        "isl": {
            "target": req.isl_target,
            "unit": req.isl_unit,
            "stat": req.isl_stat,
            "tolerance": req.isl_tolerance,
            "achieved": mix.isl_achieved,
            "gap": mix.isl_gap,
            "basis": req.isl_token_basis,
            "counting_scope": match req.isl_unit {
                LengthUnit::Words => "rendered prompt including output-word hint; excludes system/chat framing and nonces",
                LengthUnit::Tokens => "source target_input_tokens (supplied-target); hint is not included",
            },
        },
        "osl": {
            "target": req.osl_target,
            "unit": req.osl_unit,
            "stat": req.osl_stat,
            "tolerance": req.osl_tolerance,
            "achieved": mix.osl_achieved,
            "gap": mix.osl_gap,
        },
        "repeats": {
            "max_repeats": req.max_repeats,
            "extra_copies": mix.extra_copies,
            "histogram": hist,
        },
        "recommended_max_tokens": recommended_max_tokens,
        "recommended_num_requests": mix.rows.len(),
        "source_ordinals": mix.rows.iter().map(|row| row.ordinal).collect::<Vec<_>>(),
        "schedule_sha256": mix_checksum(mix),
        "hint_template": HINT_TEMPLATE,
        "work_used": mix.work_used,
        "osl_tokens_per_word": req.osl_tokens_per_word,
    });
    if let Some(profile) = profile {
        report["profile"] = json!({
            "name": profile.name,
            "version": profile.version,
        });
    }
    Ok(report)
}

pub fn load_jsonl(path: &Path) -> Result<Vec<LibraryRow>> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut rows = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&line)
            .with_context(|| format!("{}:{}: invalid JSON", path.display(), i + 1))?;
        rows.push(row_from_json(&value, i as u64)?);
    }
    Ok(rows)
}

fn json_i64(value: &Value, field: &str) -> Result<i64> {
    value
        .get(field)
        .and_then(|v| v.as_i64().or_else(|| v.as_u64().map(|u| u as i64)))
        .ok_or_else(|| anyhow!("missing integer field `{field}`"))
}

fn row_from_json(value: &Value, fallback_ordinal: u64) -> Result<LibraryRow> {
    let prompt = value
        .get("prompt")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing string field `prompt`"))?
        .to_string();
    let reasoning = match value.get("reasoning") {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => matches!(s.to_ascii_lowercase().as_str(), "true" | "1" | "yes"),
        _ => false,
    };
    let ordinal = value
        .get("source_ordinal")
        .and_then(|v| v.as_u64())
        .unwrap_or(fallback_ordinal);
    Ok(LibraryRow {
        ordinal,
        prompt,
        target_output_length: json_i64(value, "target_output_length")?,
        reasoning,
        target_input_tokens: json_i64(value, "target_input_tokens")?,
        target_output_tokens: json_i64(value, "target_output_tokens")?,
    })
}

pub fn load_parquet_files(paths: &[PathBuf], start_ordinal: u64) -> Result<Vec<LibraryRow>> {
    let mut rows = Vec::new();
    let mut ordinal = start_ordinal;
    for path in paths {
        let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let reader = SerializedFileReader::new(file)
            .with_context(|| format!("parquet {}", path.display()))?;
        let descr = reader.metadata().file_metadata().schema_descr();
        let mut idx = ColumnIndex::default();
        for i in 0..descr.num_columns() {
            match descr.column(i).name() {
                "prompt" => idx.prompt = Some(i),
                "target_output_length" => idx.target_output_length = Some(i),
                "reasoning" => idx.reasoning = Some(i),
                "target_input_tokens" => idx.target_input_tokens = Some(i),
                "target_output_tokens" => idx.target_output_tokens = Some(i),
                _ => {}
            }
        }
        idx.require()?;
        for rec in reader.get_row_iter(None)? {
            let rec = rec?;
            rows.push(LibraryRow {
                ordinal,
                prompt: rec.get_string(idx.prompt.unwrap())?.to_string(),
                target_output_length: parquet_i64(&rec, idx.target_output_length.unwrap())?,
                reasoning: parquet_bool(&rec, idx.reasoning.unwrap())?,
                target_input_tokens: parquet_i64(&rec, idx.target_input_tokens.unwrap())?,
                target_output_tokens: parquet_i64(&rec, idx.target_output_tokens.unwrap())?,
            });
            ordinal += 1;
        }
    }
    Ok(rows)
}

#[derive(Default)]
struct ColumnIndex {
    prompt: Option<usize>,
    target_output_length: Option<usize>,
    reasoning: Option<usize>,
    target_input_tokens: Option<usize>,
    target_output_tokens: Option<usize>,
}

impl ColumnIndex {
    fn require(&self) -> Result<()> {
        for (name, idx) in [
            ("prompt", self.prompt),
            ("target_output_length", self.target_output_length),
            ("reasoning", self.reasoning),
            ("target_input_tokens", self.target_input_tokens),
            ("target_output_tokens", self.target_output_tokens),
        ] {
            if idx.is_none() {
                bail!("parquet shard missing column `{name}`");
            }
        }
        Ok(())
    }
}

fn parquet_i64(row: &parquet::record::Row, idx: usize) -> Result<i64> {
    if let Ok(v) = row.get_long(idx) {
        return Ok(v);
    }
    if let Ok(v) = row.get_int(idx) {
        return Ok(i64::from(v));
    }
    bail!("cannot read integer column at index {idx}")
}

fn parquet_bool(row: &parquet::record::Row, idx: usize) -> Result<bool> {
    if let Ok(v) = row.get_bool(idx) {
        return Ok(v);
    }
    if let Ok(s) = row.get_string(idx) {
        return Ok(matches!(
            s.to_ascii_lowercase().as_str(),
            "true" | "1" | "yes"
        ));
    }
    bail!("cannot read boolean/string reasoning column")
}

pub fn apply_sample_index(rows: &mut [LibraryRow], index: &[u64]) -> Result<()> {
    if index.len() != rows.len() {
        bail!(
            "sample-index.json length {} does not match {} sample rows",
            index.len(),
            rows.len()
        );
    }
    for (row, source_line) in rows.iter_mut().zip(index.iter()) {
        if *source_line == 0 {
            bail!("sample-index.json uses one-based source lines; found 0");
        }
        row.ordinal = source_line - 1;
    }
    Ok(())
}

pub fn parse_sample_index(text: &str) -> Result<Vec<u64>> {
    let value: Value = serde_json::from_str(text)?;
    match value {
        Value::Array(items) => items
            .into_iter()
            .map(|item| {
                item.as_u64()
                    .or_else(|| item.get("source_line").and_then(Value::as_u64))
                    .ok_or_else(|| anyhow!("sample-index.json entries must be integers"))
            })
            .collect(),
        _ => bail!("sample-index.json must be a JSON array"),
    }
}

#[derive(Clone, Debug)]
pub struct DatasetRef {
    pub repo: String,
    pub revision: String,
    pub config: String,
    pub split: String,
}

pub fn default_cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("metrum-ai-bench-cli")
        .join("prompt-library")
}

pub fn resolve_revision(repo: &str, revision: &str, allow_moving: bool) -> Result<String> {
    let trimmed = revision.trim();
    if trimmed.len() == 40 && trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Ok(trimmed.to_ascii_lowercase());
    }
    if !allow_moving {
        bail!(
            "refusing moving revision `{trimmed}`; pass a 40-character commit SHA or --allow-moving-revision"
        );
    }
    let url = format!("https://huggingface.co/api/datasets/{repo}/revision/{trimmed}");
    let value = hf_get_json(&url)?;
    value
        .get("sha")
        .or_else(|| value.get("commitId"))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("Hugging Face revision lookup for `{trimmed}` returned no sha"))
}

fn hf_headers() -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::USER_AGENT,
        reqwest::header::HeaderValue::from_static("metrum-ai-bench-cli-prompts"),
    );
    if let Ok(token) = std::env::var("HF_TOKEN") {
        if !token.is_empty() {
            if let Ok(value) = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")) {
                headers.insert(reqwest::header::AUTHORIZATION, value);
            }
        }
    }
    headers
}

fn hf_get_json(url: &str) -> Result<Value> {
    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(url)
        .headers(hf_headers())
        .send()
        .with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    let text = resp.text()?;
    if !status.is_success() {
        bail!("GET {url} returned {status}: {text}");
    }
    serde_json::from_str(&text).with_context(|| format!("parse JSON from {url}"))
}

fn hf_download(url: &str, dest: &Path) -> Result<()> {
    if dest.exists() {
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(url)
        .headers(hf_headers())
        .send()
        .with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        bail!("GET {url} returned {status}");
    }
    let bytes = resp.bytes()?;
    let tmp = dest.with_extension("download");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

pub fn load_hub_dataset(
    spec: &DatasetRef,
    cache_dir: &Path,
    offline: bool,
) -> Result<(Vec<LibraryRow>, String)> {
    let sha = spec.revision.clone();
    let base = cache_dir
        .join(spec.repo.replace('/', "--"))
        .join(&sha)
        .join(&spec.config)
        .join(&spec.split);
    std::fs::create_dir_all(&base)?;
    if !offline {
        let tree_url = format!(
            "https://huggingface.co/api/datasets/{}/tree/{}/data/{}",
            spec.repo, sha, spec.config
        );
        let listing = hf_get_json(&tree_url)?;
        let files = listing
            .as_array()
            .ok_or_else(|| anyhow!("unexpected tree listing from {tree_url}"))?;
        for file in files {
            let path = file.get("path").and_then(Value::as_str).unwrap_or_default();
            if !path.ends_with(".parquet") {
                continue;
            }
            let name = Path::new(path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let url = format!(
                "https://huggingface.co/datasets/{}/resolve/{}/{}",
                spec.repo, sha, path
            );
            hf_download(&url, &base.join(name))?;
        }
        if spec.config == "sample" {
            let url = format!(
                "https://huggingface.co/datasets/{}/resolve/{}/sample-index.json",
                spec.repo, sha
            );
            let _ = hf_download(&url, &base.join("sample-index.json"));
        }
        let sums = format!(
            "https://huggingface.co/datasets/{}/resolve/{}/checksums.sha256",
            spec.repo, sha
        );
        let _ = hf_download(&sums, &base.join("checksums.sha256"));
    }
    let mut parquet_paths: Vec<PathBuf> = std::fs::read_dir(&base)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("parquet"))
        .collect();
    parquet_paths.sort();
    if parquet_paths.is_empty() {
        bail!(
            "no parquet shards in {} (download the `{}/{}` config or pass --local-parquet/--local-jsonl)",
            base.display(),
            spec.config,
            spec.split
        );
    }
    verify_cached_checksums(&base, &spec.config)?;
    let mut rows = load_parquet_files(&parquet_paths, 0)?;
    let index_path = base.join("sample-index.json");
    if spec.config == "sample" && index_path.exists() {
        let text = std::fs::read_to_string(&index_path)?;
        let index = parse_sample_index(&text)?;
        apply_sample_index(&mut rows, &index)?;
    }
    Ok((rows, sha))
}

fn verify_cached_checksums(dir: &Path, config: &str) -> Result<()> {
    let sums = dir.join("checksums.sha256");
    if !sums.exists() {
        return Ok(());
    }
    let text = std::fs::read_to_string(&sums)?;
    let prefix = format!("data/{config}/");
    let mut seen_basenames: HashMap<String, String> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (digest, name) = line
            .split_once(char::is_whitespace)
            .ok_or_else(|| anyhow!("invalid checksums.sha256 line"))?;
        let name = name.trim().trim_start_matches('*').trim_start_matches("./");
        let path = Path::new(name);
        if path.components().any(|c| matches!(c, Component::ParentDir)) {
            bail!("forbidden path in checksums.sha256: {name}");
        }
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.to_string());
        if file_name == "checksums.sha256" {
            continue;
        }
        if !name.starts_with(&prefix) {
            continue;
        }
        let digest = digest.trim().to_ascii_lowercase();
        if let Some(prev) = seen_basenames.get(&file_name) {
            if prev != &digest {
                bail!("checksum basename collision for {file_name} under config {config}");
            }
        } else {
            seen_basenames.insert(file_name.clone(), digest.clone());
        }
        let path = dir.join(&file_name);
        if !path.exists() {
            continue;
        }
        let bytes = std::fs::read(&path)?;
        let actual = hex_digest(Sha256::digest(&bytes).as_slice());
        if actual != digest {
            bail!("checksum mismatch for {}", path.display());
        }
    }
    Ok(())
}

pub fn format_select_failure(err: &SelectFailure, req: &SelectRequest) -> String {
    let kind = if err.proven_empty {
        "proven impossibility"
    } else {
        "search budget exhausted or no mix inside tolerances"
    };
    format!(
        "{}\n  kind: {}\n  candidates: {}\n  best_n: {}\n  preferred_count: {}\n  count_slack: {}\n  ISL target {} {:?} {:?} ±{} achieved {:?} gap {:?}\n  OSL target {} {:?} {:?} ±{} achieved {:?} gap {:?}\n  work_used: {}\n  reasoning: {:?}",
        err.message,
        kind,
        err.candidate_count,
        err.selected_count,
        req.count,
        req.count_slack,
        req.isl_target,
        req.isl_unit,
        req.isl_stat,
        req.isl_tolerance,
        err.isl_achieved,
        err.isl_gap,
        req.osl_target,
        req.osl_unit,
        req.osl_stat,
        req.osl_tolerance,
        err.osl_achieved,
        err.osl_gap,
        err.work_used,
        req.reasoning
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        ordinal: u64,
        prompt: &str,
        isl_tok: i64,
        osl_tok: i64,
        osl_words: i64,
        reasoning: bool,
    ) -> LibraryRow {
        LibraryRow {
            ordinal,
            prompt: prompt.to_string(),
            target_output_length: osl_words,
            reasoning,
            target_input_tokens: isl_tok,
            target_output_tokens: osl_tok,
        }
    }

    fn base_req() -> SelectRequest {
        SelectRequest {
            count: 4,
            count_slack: 4,
            seed: 1,
            isl_target: 10.0,
            isl_unit: LengthUnit::Tokens,
            isl_stat: LengthStat::Mean,
            isl_tolerance: 0.0,
            osl_target: 10.0,
            osl_unit: LengthUnit::Tokens,
            osl_stat: LengthStat::Mean,
            osl_tolerance: 0.0,
            isl_token_basis: IslTokenBasis::SuppliedTarget,
            reasoning: ReasoningFilter::Any,
            max_repeats: 8,
            work_limit: 20_000,
            osl_tokens_per_word: None,
        }
    }

    #[test]
    fn workload_profiles_are_versioned_and_positive() {
        for profile in [
            WorkloadProfile::ChatShort,
            WorkloadProfile::ChatMedium,
            WorkloadProfile::RagMedium,
            WorkloadProfile::SummarizeLong,
            WorkloadProfile::CodeMedium,
        ] {
            let spec = profile.spec();
            assert_eq!(spec.version, WORKLOAD_PROFILE_VERSION);
            assert!(spec.isl_target > 0.0);
            assert!(spec.osl_target > 0.0);
            assert!(spec.isl_tolerance >= 0.0);
            assert!(spec.osl_tolerance >= 0.0);
        }
        assert_eq!(WorkloadProfile::ChatShort.spec().name, "chat-short");
        assert_eq!(WorkloadProfile::RagMedium.spec().isl_target, 2048.0);
    }

    #[test]
    fn python_split_semantics_on_unicode_whitespace() {
        assert_eq!(LibraryRow::word_count("a\tb\nc  d"), 4);
        assert_eq!(LibraryRow::word_count("a\u{00a0}b"), 2);
        assert_eq!(LibraryRow::word_count("  only  "), 1);
    }

    #[test]
    fn even_median_is_mean_of_two_central() {
        assert_eq!(mix_median(&[1.0, 3.0, 5.0, 7.0]), Some(4.0));
        assert_eq!(mix_median(&[1.0, 3.0, 5.0]), Some(3.0));
    }

    #[test]
    fn hint_is_included_in_word_isl() {
        let row = row(0, "one two", 32, 32, 22, false);
        let words = row.isl(LengthUnit::Words, IslTokenBasis::SuppliedTarget);
        assert!(words > 2.0, "hint must increase word ISL, got {words}");
        assert_eq!(
            row.isl(LengthUnit::Tokens, IslTokenBasis::SuppliedTarget),
            32.0
        );
    }

    #[test]
    fn joint_mean_selects_matching_rows() {
        let rows = vec![
            row(0, "a", 10, 10, 10, false),
            row(1, "b", 10, 10, 10, false),
            row(2, "c", 10, 10, 10, false),
            row(3, "d", 10, 10, 10, false),
            row(4, "e", 900, 900, 900, false),
        ];
        let mix = select_mix(&rows, &base_req()).expect("feasible");
        assert_eq!(mix.rows.len(), 4);
        assert_eq!(mix.isl_achieved, 10.0);
        assert_eq!(mix.osl_achieved, 10.0);
        assert_eq!(mix.extra_copies, 0);
    }

    #[test]
    fn repeats_required_to_hit_mean() {
        let rows = vec![
            row(0, "keep", 10, 10, 10, false),
            row(1, "noise", 900, 900, 900, false),
        ];
        let mut req = base_req();
        req.count = 4;
        req.count_slack = 0;
        req.max_repeats = 8;
        let mix = select_mix(&rows, &req).expect("repeats should solve");
        assert_eq!(mix.rows.len(), 4);
        assert!(mix.extra_copies >= 3);
        assert!(mix.rows.iter().all(|r| r.ordinal == 0));
    }

    #[test]
    fn no_repeats_cannot_solve_repeats_required_case() {
        let rows = vec![
            row(0, "keep", 10, 10, 10, false),
            row(1, "noise", 900, 900, 900, false),
        ];
        let mut req = base_req();
        req.count = 4;
        req.count_slack = 0;
        req.max_repeats = 1;
        let err = select_mix(&rows, &req).expect_err("should fail");
        assert!(!err.proven_empty);
    }

    #[test]
    fn count_slack_allows_leaving_preferred_size() {
        let rows = vec![
            row(0, "keep", 10, 10, 10, false),
            row(1, "noise", 900, 900, 900, false),
        ];
        let mut req = base_req();
        req.count = 5;
        req.count_slack = 4;
        req.max_repeats = 1;
        let mix = select_mix(&rows, &req).expect("n=1 should work");
        assert_eq!(mix.rows.len(), 1);
        assert_eq!(mix.isl_achieved, 10.0);
    }

    #[test]
    fn reasoning_filter_and_empty_set() {
        let rows = vec![row(0, "a", 10, 10, 10, false)];
        let mut req = base_req();
        req.reasoning = ReasoningFilter::True;
        let err = select_mix(&rows, &req).expect_err("empty");
        assert!(err.proven_empty);
    }

    #[test]
    fn duplicate_prompt_different_osl_is_distinct() {
        let rows = vec![
            row(0, "same", 32, 32, 20, false),
            row(1, "same", 32, 256, 180, false),
        ];
        let mut req = base_req();
        req.count = 1;
        req.count_slack = 0;
        req.isl_target = 32.0;
        req.osl_target = 256.0;
        req.isl_tolerance = 0.0;
        req.osl_tolerance = 0.0;
        req.max_repeats = 1;
        let mix = select_mix(&rows, &req).expect("pick the 256-token variant");
        assert_eq!(mix.rows[0].ordinal, 1);
        assert!(mix.rows[0].rendered_prompt().contains("180"));
    }

    #[test]
    fn odd_and_even_median_targets() {
        let rows: Vec<_> = (0..7)
            .map(|i| row(i, "x", 10 + i as i64, 20, 20, false))
            .collect();
        let mut req = base_req();
        req.isl_stat = LengthStat::Median;
        req.osl_stat = LengthStat::Median;
        req.isl_target = 13.0;
        req.osl_target = 20.0;
        req.count = 5;
        req.count_slack = 0;
        req.isl_tolerance = 0.0;
        req.osl_tolerance = 0.0;
        req.max_repeats = 1;
        let mix = select_mix(&rows, &req).expect("odd median");
        assert_eq!(mix.rows.len(), 5);
        assert_eq!(mix.isl_achieved, 13.0);

        req.count = 4;
        req.isl_target = 12.5;
        let mix = select_mix(&rows, &req).expect("even median");
        assert_eq!(mix.rows.len(), 4);
        assert_eq!(mix.isl_achieved, 12.5);
    }

    #[test]
    fn exact_joint_bucket_unique_no_repeats() {
        let mut rows: Vec<_> = (0..80)
            .map(|i| row(i, "exact", 1024, 1024, 700, false))
            .collect();
        rows.extend((80..120).map(|i| row(i, "same-isl", 1024, 768, 500, false)));
        rows.extend((120..160).map(|i| row(i, "noise", 384, 512, 300, false)));
        let mut req = base_req();
        req.count = 40;
        req.count_slack = 0;
        req.seed = 7;
        req.isl_target = 1024.0;
        req.osl_target = 1024.0;
        req.isl_stat = LengthStat::Median;
        req.osl_stat = LengthStat::Median;
        req.isl_tolerance = 0.0;
        req.osl_tolerance = 0.0;
        req.max_repeats = 1;
        let mix = select_mix(&rows, &req).expect("exact bucket");
        assert_eq!(mix.rows.len(), 40);
        assert_eq!(mix.isl_achieved, 1024.0);
        assert_eq!(mix.osl_achieved, 1024.0);
        assert_eq!(mix.extra_copies, 0);
        assert!(mix.rows.iter().all(|r| r.target_input_tokens == 1024));
        assert!(mix.rows.iter().all(|r| r.target_output_tokens == 1024));
        let mut ordinals: Vec<_> = mix.rows.iter().map(|r| r.ordinal).collect();
        ordinals.sort_unstable();
        ordinals.dedup();
        assert_eq!(ordinals.len(), 40);
    }

    #[test]
    fn exact_bucket_preferred_over_same_isl_other_osl() {
        let mut rows: Vec<_> = (0..20)
            .map(|i| row(i, "joint", 1024, 1024, 700, false))
            .collect();
        rows.extend((20..120).map(|i| row(i, "other-osl", 1024, 768, 500, false)));
        let mut req = base_req();
        req.count = 10;
        req.count_slack = 0;
        req.seed = 3;
        req.isl_target = 1024.0;
        req.osl_target = 1024.0;
        req.isl_stat = LengthStat::Median;
        req.osl_stat = LengthStat::Median;
        req.isl_tolerance = 0.0;
        req.osl_tolerance = 0.0;
        req.max_repeats = 1;
        let mix = select_mix(&rows, &req).expect("joint cell");
        assert_eq!(mix.rows.len(), 10);
        assert_eq!(mix.isl_achieved, 1024.0);
        assert_eq!(mix.osl_achieved, 1024.0);
        assert_eq!(mix.extra_copies, 0);
        assert!(mix.rows.iter().all(|r| r.target_output_tokens == 1024));
    }

    #[test]
    fn exact_bucket_is_deterministic_for_seed() {
        let rows: Vec<_> = (0..50)
            .map(|i| row(i, "exact", 1024, 1024, 700, false))
            .collect();
        let mut req = base_req();
        req.count = 12;
        req.count_slack = 0;
        req.seed = 99;
        req.isl_target = 1024.0;
        req.osl_target = 1024.0;
        req.isl_stat = LengthStat::Median;
        req.osl_stat = LengthStat::Median;
        req.max_repeats = 1;
        let a = select_mix(&rows, &req).expect("first");
        let b = select_mix(&rows, &req).expect("second");
        let a_ord: Vec<_> = a.rows.iter().map(|r| r.ordinal).collect();
        let b_ord: Vec<_> = b.rows.iter().map(|r| r.ordinal).collect();
        assert_eq!(a_ord, b_ord);
    }

    #[test]
    fn verify_checksums_scopes_to_requested_config() {
        let dir = tempfile::tempdir().unwrap();
        let payload = b"full-shard-bytes";
        let full_digest = hex_digest(Sha256::digest(payload).as_slice());
        let sample_digest = hex_digest(Sha256::digest(b"sample-shard-bytes").as_slice());
        std::fs::write(dir.path().join("train-00000.parquet"), payload).unwrap();
        let sums = format!(
            "{full_digest} data/full/train-00000.parquet\n{sample_digest} data/sample/train-00000.parquet\n"
        );
        std::fs::write(dir.path().join("checksums.sha256"), sums).unwrap();
        verify_cached_checksums(dir.path(), "full").expect("full config");
        let err = verify_cached_checksums(dir.path(), "sample").expect_err("sample mismatch");
        assert!(
            err.to_string().contains("checksum mismatch"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn verify_checksums_rejects_parent_dir_components() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("checksums.sha256"),
            "abc data/full/../secret.parquet\n",
        )
        .unwrap();
        let err = verify_cached_checksums(dir.path(), "full").expect_err("forbidden");
        assert!(err.to_string().contains("forbidden path"));
    }

    #[test]
    fn load_jsonl_roundtrip_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rows.jsonl");
        std::fs::write(
            &path,
            r#"{"prompt":"hello world","target_output_length":22,"reasoning":true,"target_input_tokens":32,"target_output_tokens":48,"source_ordinal":9}
"#,
        )
        .unwrap();
        let rows = load_jsonl(&path).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ordinal, 9);
        assert!(rows[0].reasoning);
        assert_eq!(rows[0].target_output_tokens, 48);
    }
}
