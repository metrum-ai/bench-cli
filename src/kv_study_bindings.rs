// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//! Typed projection of the landed final-wire producer, not a renderer or nonce
//! algorithm. Missing/invalid binding never becomes a known zero quantity.
use crate::cache_workload::PolicyEffects;
use serde::Serialize;
#[derive(Serialize)]
pub struct ObservedFinalBody {
    pub attempt_ordinal: usize,
    pub body_sha256: Option<String>,
    pub policy_effects: Option<PolicyEffects>,
    pub unknown: Option<&'static str>,
}
impl ObservedFinalBody {
    pub(crate) fn project(ordinal: usize, digest: Option<&str>, effects: Option<&str>) -> Self {
        let hash = digest
            .filter(|s| {
                s.len() == 64
                    && s.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            .map(str::to_owned);
        // Consume the landed contract itself, not a second copy of its
        // overhead/residual validation rules.
        static VALIDATOR: std::sync::OnceLock<jsonschema::Validator> = std::sync::OnceLock::new();
        let validator = VALIDATOR.get_or_init(|| {
            let contract: serde_json::Value =
                serde_json::from_str(include_str!("../schemas/kv-study-report.v1.json"))
                    .expect("bundled contract");
            jsonschema::validator_for(
                &serde_json::json!({"$defs":contract["$defs"],"$ref":"#/$defs/policy_effects"}),
            )
            .expect("bundled policy effects schema")
        });
        let effect = effects
            .filter(|s| s.len() <= 64 * 1024)
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .filter(|v| validator.is_valid(v))
            .and_then(|v| serde_json::from_value::<PolicyEffects>(v).ok());
        let unknown = if hash.is_none() {
            Some("final_body_binding_unavailable_or_invalid")
        } else if effect.is_none() {
            Some("policy_effects_unavailable_or_invalid")
        } else {
            None
        };
        Self {
            attempt_ordinal: ordinal,
            policy_effects: if hash.is_some() { effect } else { None },
            body_sha256: hash,
            unknown,
        }
    }
}
