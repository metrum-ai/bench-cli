// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Request-level server reports, never client overlap or inferred tier evidence.
//! The only accepted source in v1 is OpenAI Chat Completions usage details.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEMA_VERSION: &str = "cached-token-evidence.v1";
pub const SOURCE: &str = "openai-chat-usage.v1:/prompt_tokens_details/cached_tokens";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountStatus {
    #[default]
    Missing,
    Null,
    Reported,
    Invalid,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitCohort {
    Cold,
    Partial,
    Full,
    #[default]
    Unknown,
}

/// Bounded stream accumulator. Conflicting reports never silently pick a winner.
/// Counters remain raw reports even when contradictory; classification then stays
/// Unknown. Neither a report nor a cohort identifies a participating cache tier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedTokenEvidence {
    pub schema_version: String,
    pub cached_tokens: Option<u64>,
    pub prompt_tokens: Option<u64>,
    pub cached_status: CountStatus,
    pub prompt_status: CountStatus,
    pub source: Option<String>,
    pub units: String,
    pub conflicting_reports: bool,
    pub invalid_report_seen: bool,
    pub null_report_seen: bool,
}

impl Default for CachedTokenEvidence {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION.into(),
            cached_tokens: None,
            prompt_tokens: None,
            cached_status: CountStatus::Missing,
            prompt_status: CountStatus::Missing,
            source: None,
            units: "tokens".into(),
            conflicting_reports: false,
            invalid_report_seen: false,
            null_report_seen: false,
        }
    }
}

fn parse_count(value: &Value) -> (CountStatus, Option<u64>) {
    if value.is_null() {
        (CountStatus::Null, None)
    } else if let Some(count) = value.as_u64() {
        (CountStatus::Reported, Some(count))
    } else {
        (CountStatus::Invalid, None)
    }
}

impl CachedTokenEvidence {
    pub fn from_usage(usage: &Value) -> Self {
        let mut evidence = Self::default();
        evidence.observe_usage(usage);
        evidence
    }

    /// Missing fields do not erase previous usage-only chunk observations.
    /// Explicit null/invalid fields remain distinguishable from absent fields.
    pub fn observe_usage(&mut self, usage: &Value) {
        if let Some(value) = usage.get("prompt_tokens") {
            let (status, count) = parse_count(value);
            if status == CountStatus::Reported {
                self.conflicting_reports |=
                    self.prompt_tokens.is_some() && self.prompt_tokens != count;
                self.prompt_tokens = count;
                self.prompt_status = status;
            } else if status == CountStatus::Invalid {
                self.prompt_status = status;
                self.invalid_report_seen = true;
            } else if self.prompt_status == CountStatus::Missing {
                self.prompt_status = status;
            }
        }
        if let Some(value) = usage.pointer("/prompt_tokens_details/cached_tokens") {
            let (status, count) = parse_count(value);
            if status == CountStatus::Reported {
                self.conflicting_reports |=
                    self.cached_tokens.is_some() && self.cached_tokens != count;
                self.cached_tokens = count;
                self.cached_status = status;
            } else if status == CountStatus::Invalid {
                self.cached_status = status;
                self.invalid_report_seen = true;
            } else {
                self.null_report_seen = true;
                if self.cached_status == CountStatus::Missing {
                    self.cached_status = status;
                }
            }
            self.source = Some(SOURCE.into());
        } else if usage
            .get("prompt_tokens_details")
            .is_some_and(|details| !details.is_null() && !details.is_object())
        {
            self.cached_status = CountStatus::Invalid;
            self.source = Some(SOURCE.into());
            self.invalid_report_seen = true;
        }
    }

    pub fn contradictory(&self) -> bool {
        self.conflicting_reports
            || matches!((self.cached_tokens, self.prompt_tokens), (Some(cached), Some(prompt)) if cached > prompt)
    }

    pub fn cohort(&self) -> HitCohort {
        if self.schema_version != SCHEMA_VERSION
            || self.source.as_deref() != Some(SOURCE)
            || self.units != "tokens"
            || self.cached_status != CountStatus::Reported
            || self.prompt_status != CountStatus::Reported
            || self.invalid_report_seen
            || self.contradictory()
        {
            return HitCohort::Unknown;
        }
        match (self.cached_tokens, self.prompt_tokens) {
            (Some(_), Some(0)) => HitCohort::Unknown,
            (Some(0), Some(_)) => HitCohort::Cold,
            (Some(cached), Some(prompt)) if cached == prompt => HitCohort::Full,
            (Some(_), Some(_)) => HitCohort::Partial,
            _ => HitCohort::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_null_and_zero_remain_distinct() {
        let missing = CachedTokenEvidence::from_usage(&json!({"prompt_tokens":8}));
        let null = CachedTokenEvidence::from_usage(
            &json!({"prompt_tokens":8,"prompt_tokens_details":{"cached_tokens":null}}),
        );
        let zero = CachedTokenEvidence::from_usage(
            &json!({"prompt_tokens":8,"prompt_tokens_details":{"cached_tokens":0}}),
        );
        assert_eq!(missing.cached_status, CountStatus::Missing);
        assert_eq!(null.cached_status, CountStatus::Null);
        assert_eq!(zero.cached_tokens, Some(0));
        assert_eq!(missing.cohort(), HitCohort::Unknown);
        assert_eq!(null.cohort(), HitCohort::Unknown);
        assert_eq!(zero.cohort(), HitCohort::Cold);
        assert_eq!(zero.source.as_deref(), Some(SOURCE));
    }

    #[test]
    fn partial_full_and_contradictory_reports_preserve_raw_counts() {
        for (cached, expected) in [
            (3, HitCohort::Partial),
            (8, HitCohort::Full),
            (9, HitCohort::Unknown),
        ] {
            let evidence = CachedTokenEvidence::from_usage(
                &json!({"prompt_tokens":8,"prompt_tokens_details":{"cached_tokens":cached}}),
            );
            assert_eq!(evidence.cached_tokens, Some(cached));
            assert_eq!(evidence.cohort(), expected);
            assert_eq!(evidence.contradictory(), cached > 8);
        }
    }

    #[test]
    fn invalid_counts_denominators_and_unversioned_aliases_are_unknown() {
        for invalid in [json!(-1), json!(1.5), json!("4"), json!(true), json!(1e30)] {
            let evidence = CachedTokenEvidence::from_usage(
                &json!({"prompt_tokens":8,"prompt_tokens_details":{"cached_tokens":invalid}}),
            );
            assert_eq!(evidence.cached_status, CountStatus::Invalid);
            assert_eq!(evidence.cohort(), HitCohort::Unknown);
        }
        for usage in [
            json!({"cached_tokens":8,"prompt_tokens":8}),
            json!({"input_tokens_details":{"cached_tokens":8},"input_tokens":8}),
            json!({"prompt_tokens":null,"prompt_tokens_details":{"cached_tokens":0}}),
            json!({"prompt_tokens":0,"prompt_tokens_details":{"cached_tokens":0}}),
        ] {
            assert_eq!(
                CachedTokenEvidence::from_usage(&usage).cohort(),
                HitCohort::Unknown
            );
        }
    }

    #[test]
    fn unsupported_schema_source_or_units_cannot_be_classified() {
        let evidence = CachedTokenEvidence::from_usage(
            &json!({"prompt_tokens":8,"prompt_tokens_details":{"cached_tokens":8}}),
        );
        let mut wrong_units = evidence.clone();
        wrong_units.units = "bytes".into();
        let mut wrong_source = evidence.clone();
        wrong_source.source = Some("unversioned_alias".into());
        let mut wrong_schema = evidence;
        wrong_schema.schema_version = "future.v99".into();
        for unknown in [wrong_units, wrong_source, wrong_schema] {
            assert_eq!(unknown.cohort(), HitCohort::Unknown);
        }
    }

    #[test]
    fn null_is_unknown_not_a_conflicting_numeric_report() {
        let mut evidence = CachedTokenEvidence::from_usage(
            &json!({"prompt_tokens":null,"prompt_tokens_details":{"cached_tokens":null}}),
        );
        evidence
            .observe_usage(&json!({"prompt_tokens":8,"prompt_tokens_details":{"cached_tokens":4}}));
        assert_eq!(evidence.cohort(), HitCohort::Partial);
        assert!(evidence.null_report_seen);
        assert!(!evidence.conflicting_reports);
        evidence.observe_usage(&json!({"prompt_tokens_details":{"cached_tokens":null}}));
        assert_eq!(evidence.cached_tokens, Some(4));
        assert_eq!(evidence.cohort(), HitCohort::Partial);
        evidence.observe_usage(&json!({"prompt_tokens":9}));
        assert!(evidence.conflicting_reports);
        assert_eq!(evidence.cohort(), HitCohort::Unknown);
    }

    #[test]
    fn usage_only_chunks_accumulate_but_conflicting_reports_fail_closed() {
        let mut evidence = CachedTokenEvidence::default();
        evidence.observe_usage(&json!({"prompt_tokens_details":{"cached_tokens":4}}));
        assert_eq!(evidence.cohort(), HitCohort::Unknown);
        evidence.observe_usage(&json!({"prompt_tokens":8}));
        assert_eq!(evidence.cohort(), HitCohort::Partial);
        evidence.observe_usage(&json!({"prompt_tokens_details":{"cached_tokens":4}}));
        assert_eq!(evidence.cohort(), HitCohort::Partial);
        evidence.observe_usage(&json!({"prompt_tokens_details":{"cached_tokens":0}}));
        assert_eq!(evidence.cached_tokens, Some(0));
        assert!(evidence.conflicting_reports);
        assert_eq!(evidence.cohort(), HitCohort::Unknown);
    }
}
