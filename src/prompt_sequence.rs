// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Effective prompt-sequence emission and prescribed replay (#313).
//!
//! Same seed and same pool do not guarantee the same effective prompt order
//! across clients (the W2 TTFT audit). This module records the effective
//! canonical sequence per slot (so a comparator can VALIDATE workload
//! equivalence instead of trusting an integer seed) and replays a recorded
//! or prescribed order exactly (converter compensation), with both sides
//! leaving the benchmark clients unmodified.

use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::io::Write;
use std::path::Path;

/// One effective slot→prompt assignment (no prompt text).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptSlotEntry {
    /// Slot sequence number (issuance order).
    pub seq: u64,
    /// Pool index the slot used (after the deterministic shuffle, or from
    /// the prescribed sequence).
    pub prompt_index: u64,
    /// First 16 hex chars of sha256(prompt text): a fingerprint, never the
    /// content.
    pub prompt_sha256_16: String,
    /// Prompt length in bytes (length, not content).
    pub prompt_bytes: u64,
    /// Closed-loop wave: seq / effective concurrency.
    pub cohort: u64,
}

/// Fingerprint a prompt without recording its content.
pub fn prompt_fingerprint(prompt: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(prompt.as_bytes());
    let digest = hasher.finalize();
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// Canonical pool index for a prompt: its rank when the pool is sorted by
/// content fingerprint (#313). Stable across shuffles, seeds and clients,
/// so two runs with the same content produce byte-identical sequence logs.
fn canonical_index(prompts: &[String], prompt: &str) -> u64 {
    let fp = prompt_fingerprint(prompt);
    let mut order: Vec<(String, u64)> = prompts
        .iter()
        .enumerate()
        .map(|(i, p)| (prompt_fingerprint(p), i as u64))
        .collect();
    order.sort();
    order
        .iter()
        .position(|(f, _)| f == &fp)
        .expect("prompt is from the same pool") as u64
}

/// The effective assignment for slot `seq`: pool index used by the run.
fn slot_index(seq: u64, pool_len: usize, prescribed: Option<&[u64]>) -> u64 {
    match prescribed {
        Some(order) => order[seq as usize % order.len()],
        None => seq % pool_len as u64,
    }
}

/// Write the effective canonical sequence for a run (#313): one JSONL row
/// per slot — seq, pool index, prompt fingerprint/length, cohort.
pub fn write_sequence_log(
    path: &Path,
    prompts: &[String],
    num_requests: u64,
    effective_concurrency: u64,
    prescribed: Option<&[u64]>,
) -> std::io::Result<()> {
    let mut out = std::fs::File::create(path)?;
    let cohort_width = effective_concurrency.max(1);
    for seq in 0..num_requests {
        let index = slot_index(seq, prompts.len(), prescribed);
        let prompt = &prompts[index as usize];
        let entry = PromptSlotEntry {
            seq,
            // prompt_index is the CANONICAL rank (fingerprint-sorted), not
            // the shuffled/replayed pool position — identical across
            // clients and seeds for identical content (#313).
            prompt_index: canonical_index(prompts, prompt),
            prompt_sha256_16: prompt_fingerprint(prompt),
            prompt_bytes: prompt.len() as u64,
            cohort: seq / cohort_width,
        };
        serde_json::to_writer(&mut out, &entry).map_err(std::io::Error::other)?;
        out.write_all(b"\n")?;
    }
    Ok(())
}

/// Load a prescribed sequence: one JSONL row per slot. Rows from
/// [`write_sequence_log`] carry `prompt_index` and the content
/// `prompt_sha256_16` fingerprint; a hand-written prescription may omit the
/// fingerprint (index-only, validated by range).
pub fn load_prescribed_entries(path: &Path) -> Result<Vec<PromptSlotEntry>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("read prescribed prompt sequence {}: {e}", path.display()))?;
    let mut entries = Vec::new();
    for (line, text) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let entry: PromptSlotEntry = serde_json::from_str(text)
            .map_err(|e| format!("prescribed sequence line {}: {e}", line + 1))?;
        entries.push(entry);
    }
    if entries.is_empty() {
        return Err(format!("prescribed sequence {} is empty", path.display()));
    }
    Ok(entries)
}

/// Resolve a prescribed sequence to pool indices (#313).
///
/// Fingerprint-first: a recorded entry replays by content (sha256-16), so a
/// sequence log from another client's run transfers exactly even when the
/// shuffle mapping differs. An entry without a fingerprint falls back to
/// `prompt_index` (range-validated by [`validate_prescribed_sequence`]).
/// Content not present in the pool is a hard error — replay never silently
/// substitutes a different prompt.
pub fn resolve_prescribed_order(
    entries: &[PromptSlotEntry],
    prompts: &[String],
) -> Result<Vec<u64>, String> {
    // fingerprint -> all pool indices with that content (duplicate prompts
    // share a fingerprint); slots repeat content far more often than the
    // pool does, so matches cycle deterministically rather than consuming.
    let mut by_fingerprint: std::collections::BTreeMap<String, Vec<u64>> =
        std::collections::BTreeMap::new();
    for (index, prompt) in prompts.iter().enumerate() {
        by_fingerprint
            .entry(prompt_fingerprint(prompt))
            .or_default()
            .push(index as u64);
    }
    let mut used_per_fingerprint: std::collections::BTreeMap<String, u64> =
        std::collections::BTreeMap::new();
    let mut order = Vec::with_capacity(entries.len());
    for (slot, entry) in entries.iter().enumerate() {
        let index = match by_fingerprint.get(&entry.prompt_sha256_16) {
            Some(candidates) => {
                let seen = used_per_fingerprint
                    .entry(entry.prompt_sha256_16.clone())
                    .or_insert(0);
                let index = candidates[(*seen as usize) % candidates.len()];
                *seen += 1;
                index
            }
            None if !entry.prompt_sha256_16.is_empty() => {
                return Err(format!(
                    "prescribed sequence slot {slot}: no pool prompt matches the recorded fingerprint {}; the pool content differs from the recorded run",
                    entry.prompt_sha256_16
                ));
            }
            None => {
                validate_prescribed_sequence(&[entry.prompt_index], prompts.len())?;
                entry.prompt_index
            }
        };
        order.push(index);
    }
    Ok(order)
}

/// Validate a prescribed sequence against the pool size (#313).
pub fn validate_prescribed_sequence(order: &[u64], pool_len: usize) -> Result<(), String> {
    if pool_len == 0 {
        return Err("prompt pool is empty".to_string());
    }
    for (slot, index) in order.iter().enumerate() {
        if *index >= pool_len as u64 {
            return Err(format!(
                "prescribed sequence slot {slot}: prompt_index {index} out of range for a pool of {pool_len}"
            ));
        }
    }
    Ok(())
}

/// Reorder the prompt pool so slot `i` uses the prescribed pool index
/// (as resolved by [`resolve_prescribed_order`]: fingerprint-matched).
///
/// With the shuffle skipped, the run's `pool[seq % pool.len()]` mapping
/// yields exactly the prescribed sequence (repeating the order for runs
/// longer than it). The original pool is consumed; callers keep the run
/// deterministic by recording the order.
pub fn apply_prescribed_sequence(prompts: &mut Vec<String>, order: &[u64]) {
    let original: Vec<String> = std::mem::take(prompts);
    let new_len = order.len();
    prompts.extend((0..new_len).map(|i| original[order[i] as usize].clone()));
}

/// Seeded, bounded start-stagger jitter for closed-loop/fixed-concurrency
/// runs (#313): prevents persistent phase-locking of replacement waves
/// (which confounds TTFT cohort comparisons). Opt-in only; never applied to
/// scored or existing runs unless the operator passes the flag.
pub struct StartJitter {
    /// Uniform draw bound in milliseconds per slot.
    pub max_ms: u64,
    /// Seed for the jitter draws (recorded in the run config).
    pub seed: u64,
}

impl StartJitter {
    /// Deterministic jitter for one slot in **nanoseconds** ([0, max_ms) ms).
    /// Drawn from the (seed, seq) pair so slot order never shifts the draw.
    pub fn delay_ns(&self, seq: u64) -> u64 {
        if self.max_ms == 0 {
            return 0;
        }
        // A small keyed mix: seq and seed → uniform value in [0, max_ns).
        // Deterministic across runs and independent of the prompt shuffle.
        let mut hasher = sha2::Sha256::new();
        hasher.update(b"metrum-start-jitter");
        hasher.update(self.seed.to_le_bytes());
        hasher.update(seq.to_le_bytes());
        let digest = hasher.finalize();
        let draw = u64::from_le_bytes(digest[..8].try_into().unwrap());
        let max_ns = self.max_ms.saturating_mul(1_000_000);
        draw % max_ns
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_and_content_free() {
        let fp = prompt_fingerprint("hello world");
        assert_eq!(fp.len(), 16);
        assert_eq!(fp, prompt_fingerprint("hello world"));
        assert_ne!(fp, prompt_fingerprint("hello world!"));
        assert!(
            !fp.contains("hello"),
            "no content leaks into the fingerprint"
        );
    }

    #[test]
    fn write_sequence_log_records_index_fingerprint_cohort() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seq.jsonl");
        let prompts = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        write_sequence_log(&path, &prompts, 7, 4, None).unwrap();
        let rows: Vec<PromptSlotEntry> = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[0].prompt_index, 0);
        assert_eq!(rows[6].prompt_index, 0, "pool cycles");
        assert_eq!(rows[0].prompt_sha256_16, prompt_fingerprint("alpha"));
        assert_eq!(rows[0].prompt_bytes, 5);
        assert_eq!(rows[4].cohort, 1, "cohort = seq / cap");
        assert_eq!(rows[3].cohort, 0);
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            !raw.contains("alpha"),
            "no prompt text in the sequence log: {raw}"
        );
    }

    #[test]
    fn replay_reorders_the_pool_by_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seq.jsonl");
        let prompts = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        write_sequence_log(&path, &prompts, 3, 1, Some(&[2, 0, 1])).unwrap();
        let entries = load_prescribed_entries(&path).unwrap();
        // Replay against a DIFFERENT pool order: fingerprints resolve it.
        let mut shuffled = vec!["c".to_string(), "a".to_string(), "b".to_string()];
        let order = resolve_prescribed_order(&entries, &shuffled).unwrap();
        apply_prescribed_sequence(&mut shuffled, &order);
        assert_eq!(shuffled, vec!["c", "a", "b"], "replay order is exact");
        // Index range still validated for hand-written (index-only) rows.
        let err = validate_prescribed_sequence(&[3], 3).unwrap_err();
        assert!(err.contains("out of range"), "{err}");
    }

    #[test]
    fn replay_rejects_content_missing_from_the_pool() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seq.jsonl");
        let prompts = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        write_sequence_log(&path, &prompts, 1, 1, None).unwrap();
        let entries = load_prescribed_entries(&path).unwrap();
        let err = resolve_prescribed_order(&entries, &["different".to_string()]).unwrap_err();
        assert!(err.contains("pool content differs"), "{err}");
    }

    #[test]
    fn load_prescribed_rejects_empty_and_malformed() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty.jsonl");
        std::fs::write(&empty, "\n").unwrap();
        assert!(load_prescribed_entries(&empty)
            .unwrap_err()
            .contains("empty"));
        let bad = dir.path().join("bad.jsonl");
        std::fs::write(&bad, "{\"seq\":0}\n").unwrap();
        let err = load_prescribed_entries(&bad).unwrap_err();
        assert!(err.contains("line 1"), "{err}");
    }

    #[test]
    fn jitter_is_bounded_deterministic_and_seeded() {
        let jitter = StartJitter {
            max_ms: 50,
            seed: 42,
        };
        let same = StartJitter {
            max_ms: 50,
            seed: 42,
        };
        let other = StartJitter {
            max_ms: 50,
            seed: 43,
        };
        for seq in 0..20 {
            let d = jitter.delay_ns(seq);
            assert!(d < 50_000_000, "bounded by max_ms");
            assert_eq!(d, same.delay_ns(seq), "same (seed, seq) → same draw");
        }
        assert!(
            (0..20).any(|seq| jitter.delay_ns(seq) != other.delay_ns(seq)),
            "different seed → different draws"
        );
        // Zero bound disables jitter entirely.
        assert_eq!(StartJitter { max_ms: 0, seed: 1 }.delay_ns(7), 0);
        // Distinct draws for distinct slots (no constant wave shift).
        let draws: std::collections::BTreeSet<u64> = (0..8).map(|s| jitter.delay_ns(s)).collect();
        assert!(draws.len() > 1, "jitter varies across slots");
    }
}
