// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Shared opt-in cache mutation and effective-order primitives. Reuse is a
//! payload policy, never evidence of a physical cache hit or a tier reset.

use anyhow::{bail, Context, Result};
use clap::ValueEnum;
use rand::{rngs::StdRng, seq::SliceRandom, SeedableRng};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CachePolicy {
    /// Preserve historical mutation controls; no reuse/isolation assertion.
    #[default]
    LegacyUnspecified,
    /// Do not add policy mutation; not a guarantee of a physical hit.
    Reuse,
    /// Distinct nonce at first-message content for every request.
    BustRequest,
    /// One first-message nonce per invocation, preserving within-run reuse.
    IsolateRun,
}

impl CachePolicy {
    pub fn nonce_scope(self) -> &'static str {
        match self {
            Self::BustRequest => "request",
            Self::IsolateRun => "run",
            Self::Reuse => "none",
            Self::LegacyUnspecified => "existing_legacy_controls",
        }
    }

    pub fn nonce_placement(self) -> &'static str {
        match self {
            Self::BustRequest | Self::IsolateRun => {
                "first_message_content_prefix; chat framing can remain reusable"
            }
            Self::Reuse => "none",
            Self::LegacyUnspecified => "existing_legacy_controls",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderPolicy {
    #[default]
    LegacyUnspecified,
    Preserve,
    SeededShuffle,
    Prescribed,
}

/// Ticket gate for opt-in canonical permit acquisition. Task spawn order alone
/// does not establish FIFO acquisition: a later task can be polled first.
/// A oneshot chain wakes one successor, avoiding a quadratic wake-all queue.
#[derive(Default)]
pub struct IssuanceGate {
    tail: std::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

impl IssuanceGate {
    /// Allocate synchronously in canonical sequence, before spawning requests.
    pub fn ticket(&self) -> IssuanceTicket {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let predecessor = self
            .tail
            .lock()
            .expect("issuance gate poisoned")
            .replace(receiver);
        IssuanceTicket {
            predecessor,
            successor: Some(sender),
        }
    }
}

pub struct IssuanceTicket {
    predecessor: Option<tokio::sync::oneshot::Receiver<()>>,
    successor: Option<tokio::sync::oneshot::Sender<()>>,
}

impl IssuanceTicket {
    pub async fn wait_turn(&mut self) {
        if let Some(predecessor) = self.predecessor.take() {
            let _ = predecessor.await;
        }
    }

    /// Advance after acquiring the request permit. This orders client issuance,
    /// not remote scheduling or physical insertion. Cancellation also releases
    /// the successor; cancelled requests cannot establish observed wire order.
    pub fn advance(self) {
        drop(self);
    }
}

impl Drop for IssuanceTicket {
    fn drop(&mut self) {
        if let Some(successor) = self.successor.take() {
            let _ = successor.send(());
        }
    }
}

/// Serialized once, after all mutation, for both digesting and HTTP issuance.
/// Not serializable as a record: `bytes` can contain sensitive prompt content.
pub struct PreparedBody {
    pub bytes: Vec<u8>,
    pub sha256: String,
}

pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Apply only explicit opt-in policy. Legacy transformations must already have
/// run; callers must reject conflicting old mutation flags for explicit reuse.
/// Nonces precede the first message content, but chat-template tokens before
/// that content may remain reusable. No policy empties old cache occupancy.
pub fn prepare_chat_body(
    mut body: Value,
    policy: CachePolicy,
    run_id: &str,
    sequence: u64,
) -> Result<PreparedBody> {
    if matches!(policy, CachePolicy::BustRequest | CachePolicy::IsolateRun) {
        if run_id.is_empty() {
            bail!("cache isolation requires a nonempty invocation identity");
        }
        let identity = match policy {
            CachePolicy::BustRequest => {
                serde_json::to_vec(&("cache-policy/v1", "request", run_id, sequence))?
            }
            _ => serde_json::to_vec(&("cache-policy/v1", "run", run_id))?,
        };
        let content = body
            .get_mut("messages")
            .and_then(Value::as_array_mut)
            .and_then(|messages| messages.first_mut())
            .and_then(|message| message.get_mut("content"))
            .context("explicit cache mutation requires a first chat message with string content")?;
        let text = content
            .as_str()
            .context("explicit cache mutation requires string message content")?;
        *content = Value::String(format!("[kv-{}] {text}", sha256(&identity)));
    }
    let bytes = serde_json::to_vec(&body)?;
    let sha256 = sha256(&bytes);
    Ok(PreparedBody { bytes, sha256 })
}

/// Versioned digest-list evidence; canonical fallback IDs are `body:<sha256>`.
/// The caller supplies issuance order, retaining duplicates and errors.
pub fn fingerprint_evidence(hashes: &[String]) -> Result<Value> {
    let ids: Vec<String> = hashes.iter().map(|hash| format!("body:{hash}")).collect();
    Ok(serde_json::json!({
        "body_fingerprints_sha256": sha256(&serde_json::to_vec(hashes)?),
        "effective_order_sha256": sha256(&serde_json::to_vec(&ids)?),
        "fingerprinted_requests": hashes.len()
    }))
}

/// Resolve runner-specific historical defaults without silently overriding an
/// explicit order choice when prescribed replay is supplied.
pub fn resolve_order_policy(
    requested: OrderPolicy,
    legacy_default: OrderPolicy,
    has_prescribed: bool,
) -> Result<OrderPolicy> {
    match (requested, has_prescribed) {
        (OrderPolicy::LegacyUnspecified | OrderPolicy::Prescribed, true) => {
            Ok(OrderPolicy::Prescribed)
        }
        (_, true) => bail!("prescribed replay conflicts with explicit preserve/shuffle order"),
        (OrderPolicy::Prescribed, false) => bail!("prescribed order requires --prompt-sequence"),
        (OrderPolicy::LegacyUnspecified, false) => Ok(legacy_default),
        (policy, false) => Ok(policy),
    }
}

/// Resolve explicit canonical order. Preserve duplicates in prescribed order;
/// fingerprint-first replay resolution remains in `prompt_sequence`.
/// Callers resolve legacy defaults themselves, because historical runners differ.
pub fn ordered_indices(
    pool_len: usize,
    policy: OrderPolicy,
    seed: u64,
    prescribed: Option<&[u64]>,
) -> Result<Vec<u64>> {
    if pool_len == 0 {
        bail!("cannot order an empty input pool");
    }
    if matches!(policy, OrderPolicy::Prescribed) {
        let order = prescribed.context("prescribed order requires a resolved sequence")?;
        if order.is_empty() {
            bail!("prescribed order is empty");
        }
        crate::prompt_sequence::validate_prescribed_sequence(order, pool_len)
            .map_err(anyhow::Error::msg)?;
        return Ok(order.to_vec());
    }
    if prescribed.is_some() {
        bail!("a prescribed sequence conflicts with the selected effective order");
    }
    let mut order: Vec<u64> = (0..pool_len as u64).collect();
    match policy {
        OrderPolicy::Preserve => {}
        OrderPolicy::SeededShuffle => order.shuffle(&mut StdRng::seed_from_u64(seed)),
        OrderPolicy::LegacyUnspecified => {
            bail!("caller must resolve its historical order default explicitly")
        }
        OrderPolicy::Prescribed => unreachable!(),
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body() -> Value {
        json!({"model":"mock", "messages":[{"role":"system","content":"root"},
            {"role":"user","content":"question"}], "max_tokens": 4})
    }

    #[test]
    fn golden_reuse_is_byte_identical_and_hashes_post_transformation_bytes() {
        let original = body();
        let prepared = prepare_chat_body(original.clone(), CachePolicy::Reuse, "run-a", 0).unwrap();
        assert_eq!(prepared.bytes, serde_json::to_vec(&original).unwrap());
        assert_eq!(prepared.sha256, sha256(&prepared.bytes));
        let legacy =
            prepare_chat_body(original, CachePolicy::LegacyUnspecified, "run-b", 9).unwrap();
        assert_eq!(prepared.bytes, legacy.bytes);
        let mut transformed = body();
        transformed["messages"][1]["content"] = json!("changed hint");
        let transformed = prepare_chat_body(transformed, CachePolicy::Reuse, "run-a", 0).unwrap();
        assert_ne!(prepared.sha256, transformed.sha256);
    }

    #[test]
    fn bust_is_per_request_and_isolate_is_per_invocation_without_losing_history() {
        let a = prepare_chat_body(body(), CachePolicy::BustRequest, "a", 0).unwrap();
        let b = prepare_chat_body(body(), CachePolicy::BustRequest, "a", 1).unwrap();
        assert_ne!(a.sha256, b.sha256);
        let a = prepare_chat_body(body(), CachePolicy::IsolateRun, "a", 0).unwrap();
        let same = prepare_chat_body(body(), CachePolicy::IsolateRun, "a", 99).unwrap();
        let other = prepare_chat_body(body(), CachePolicy::IsolateRun, "b", 0).unwrap();
        assert_eq!(a.bytes, same.bytes);
        assert_ne!(a.sha256, other.sha256);
        let value: Value = serde_json::from_slice(&a.bytes).unwrap();
        assert_eq!(value["messages"][1], body()["messages"][1]);
        assert!(prepare_chat_body(body(), CachePolicy::BustRequest, "", 0).is_err());
        assert!(
            prepare_chat_body(json!({"messages":[]}), CachePolicy::BustRequest, "a", 0).is_err()
        );
        assert!(prepare_chat_body(
            json!({"messages":[{"content":[]}]}),
            CachePolicy::IsolateRun,
            "a",
            0
        )
        .is_err());
    }

    #[test]
    fn byte_tokenizer_fixture_quantifies_remaining_common_chat_root() {
        // A deliberately tiny, local tokenizer/chat-template fixture, not a
        // production renderer or a warmup-isolation Pass certificate.
        let render = |prepared: PreparedBody| {
            let body: Value = serde_json::from_slice(&prepared.bytes).unwrap();
            let mut tokens = vec![1_u32, 2, 3]; // fixed chat-template framing
            tokens.extend(
                body["messages"][0]["content"]
                    .as_str()
                    .unwrap()
                    .bytes()
                    .map(|byte| u32::from(byte) + 256),
            );
            tokens
        };
        let body = serde_json::json!({"messages":[{"role":"user","content":"same prompt"}]});
        let left =
            render(prepare_chat_body(body.clone(), CachePolicy::BustRequest, "run-a", 0).unwrap());
        let right = render(prepare_chat_body(body, CachePolicy::BustRequest, "run-b", 0).unwrap());
        let shared = left.iter().zip(&right).take_while(|(a, b)| a == b).count();
        // Three template tokens + four literal "[kv-" tokens remain common.
        assert_eq!(shared, 7);
        assert_eq!(
            shared / 4,
            1,
            "busting can leave a full reusable root block"
        );
        assert_ne!(left, right);
    }

    #[tokio::test]
    async fn ticket_gate_orders_reverse_polled_tasks_before_permit_acquisition() {
        let gate = IssuanceGate::default();
        let tickets: Vec<_> = (10..20).map(|seq| (seq, gate.ticket())).collect();
        let seen = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let mut handles = Vec::new();
        for (sequence, mut ticket) in tickets.into_iter().rev() {
            let seen = seen.clone();
            handles.push(tokio::spawn(async move {
                ticket.wait_turn().await;
                seen.lock().await.push(sequence);
                ticket.advance();
            }));
        }
        for handle in handles {
            tokio::time::timeout(std::time::Duration::from_secs(5), handle)
                .await
                .unwrap()
                .unwrap();
        }
        assert_eq!(*seen.lock().await, (10..20).collect::<Vec<_>>());
    }

    #[test]
    fn explicit_order_is_shared_and_prescription_ignores_shuffle_seed() {
        assert_eq!(
            ordered_indices(3, OrderPolicy::Preserve, 1, None).unwrap(),
            [0, 1, 2]
        );
        let a = ordered_indices(12, OrderPolicy::SeededShuffle, 42, None).unwrap();
        assert_eq!(
            a,
            ordered_indices(12, OrderPolicy::SeededShuffle, 42, None).unwrap()
        );
        assert_ne!(
            a,
            ordered_indices(12, OrderPolicy::SeededShuffle, 43, None).unwrap()
        );
        for seed in [1, 99] {
            assert_eq!(
                ordered_indices(3, OrderPolicy::Prescribed, seed, Some(&[2, 0, 2])).unwrap(),
                [2, 0, 2]
            );
        }
        assert!(ordered_indices(0, OrderPolicy::Preserve, 0, None).is_err());
        assert!(ordered_indices(2, OrderPolicy::Prescribed, 0, Some(&[2])).is_err());
        assert!(ordered_indices(2, OrderPolicy::Prescribed, 0, Some(&[])).is_err());
        assert!(ordered_indices(2, OrderPolicy::Prescribed, 0, None).is_err());
        assert!(ordered_indices(2, OrderPolicy::Preserve, 0, Some(&[0])).is_err());
        assert!(ordered_indices(2, OrderPolicy::LegacyUnspecified, 0, None).is_err());
    }
}
