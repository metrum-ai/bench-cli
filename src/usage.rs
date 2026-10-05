// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Server `usage` object helpers shared by the chat runners (Metrum AI Bench).

use serde_json::Value;

/// Locations of the reasoning token count inside a `usage` object, checked in
/// order. The first one that holds a non-negative integer wins.
///
/// * `completion_tokens_details.reasoning_tokens`: OpenAI Chat Completions,
///   SGLang, DeepSeek, OpenRouter and most OpenAI-compatible servers.
/// * `output_tokens_details.reasoning_tokens`: OpenAI Responses API shape.
/// * `reasoning_tokens`: flat variant some gateways emit at the top level.
pub const REASONING_TOKEN_POINTERS: [&str; 3] = [
    "/completion_tokens_details/reasoning_tokens",
    "/output_tokens_details/reasoning_tokens",
    "/reasoning_tokens",
];

/// Reasoning token count reported by the server, or `None` when no accepted
/// variant is present (never a fabricated `0`). A reported `0` stays `Some(0)`.
pub fn reasoning_tokens(usage: &Value) -> Option<u64> {
    REASONING_TOKEN_POINTERS
        .iter()
        .find_map(|pointer| usage.pointer(pointer).and_then(non_negative_count))
}

/// `completion_tokens - reasoning_tokens` when the server reported reasoning
/// and the two counts are consistent; `None` otherwise.
pub fn visible_completion_tokens(completion_tokens: u64, reasoning: Option<u64>) -> Option<u64> {
    completion_tokens.checked_sub(reasoning?)
}

fn non_negative_count(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.0 && v.fract() == 0.0)
            .map(|v| v as u64)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_openai_completion_tokens_details() {
        let usage =
            json!({"completion_tokens": 40, "completion_tokens_details": {"reasoning_tokens": 12}});
        assert_eq!(reasoning_tokens(&usage), Some(12));
    }

    #[test]
    fn reads_responses_and_flat_variants() {
        assert_eq!(
            reasoning_tokens(&json!({"output_tokens_details": {"reasoning_tokens": 5}})),
            Some(5)
        );
        assert_eq!(reasoning_tokens(&json!({"reasoning_tokens": 7})), Some(7));
    }

    #[test]
    fn first_variant_wins() {
        let usage =
            json!({"completion_tokens_details": {"reasoning_tokens": 3}, "reasoning_tokens": 9});
        assert_eq!(reasoning_tokens(&usage), Some(3));
    }

    #[test]
    fn absent_or_null_is_none_and_reported_zero_is_zero() {
        assert_eq!(reasoning_tokens(&json!({"completion_tokens": 4})), None);
        assert_eq!(
            reasoning_tokens(&json!({"completion_tokens_details": null})),
            None
        );
        assert_eq!(
            reasoning_tokens(&json!({"completion_tokens_details": {"reasoning_tokens": null}})),
            None
        );
        assert_eq!(
            reasoning_tokens(&json!({"completion_tokens_details": {"reasoning_tokens": 0}})),
            Some(0)
        );
    }

    #[test]
    fn rejects_negative_and_non_numeric() {
        assert_eq!(reasoning_tokens(&json!({"reasoning_tokens": -1})), None);
        assert_eq!(reasoning_tokens(&json!({"reasoning_tokens": "12"})), None);
        assert_eq!(reasoning_tokens(&json!({"reasoning_tokens": 1.5})), None);
    }

    #[test]
    fn visible_tokens_subtract_reasoning_only_when_consistent() {
        assert_eq!(visible_completion_tokens(40, Some(12)), Some(28));
        assert_eq!(visible_completion_tokens(40, None), None);
        assert_eq!(visible_completion_tokens(10, Some(12)), None);
    }
}
