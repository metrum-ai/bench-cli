// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Measurement gates shared by the chat load generators.
//!
//! Empty runs fail before any request is sent. First-any-token TTFT is the
//! normal streaming measurement. `--infer-ttft-from-first-byte` may copy HTTP
//! time-to-first-byte into `ttft_s` and must say so.

use serde::{Deserialize, Serialize};

/// Where a recorded TTFT came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TtftSource {
    /// First observed reasoning or visible output delta on a streaming response.
    Stream,
    /// HTTP time-to-first-byte used because visible-token TTFT was absent.
    FirstByteApprox,
    /// Actual first reasoning token used because visible-token TTFT was absent.
    FirstTokenFallback,
}

impl TtftSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::FirstByteApprox => "first_byte_approx",
            Self::FirstTokenFallback => "first_token_fallback",
        }
    }
}

/// One request's resolved TTFT.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedTtft {
    pub ttft_s: Option<f64>,
    pub source: Option<TtftSource>,
}

/// Warning or failure outcome for a chat run's TTFT coverage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtftAudit {
    pub approx_count: usize,
    pub warning: Option<String>,
}

/// Fail when warmup consumes the whole run.
///
/// `num_requests` is the total number of requests issued, including warmup.
pub fn ensure_warmup_leaves_measurement(
    warmup_requests: u64,
    num_requests: u64,
) -> anyhow::Result<()> {
    if num_requests == 0 || warmup_requests >= num_requests {
        anyhow::bail!(
            "no measured requests: warmup_requests ({warmup_requests}) must be < num_requests ({num_requests})"
        );
    }
    Ok(())
}

/// Fail when a strategic stage would record no measured requests.
///
/// Strategic warmup is added on top of `requests_per_stage`, so a positive
/// warmup count does not consume the measured set.
pub fn ensure_requests_per_stage(requests_per_stage: u64) -> anyhow::Result<()> {
    if requests_per_stage == 0 {
        anyhow::bail!("no measured requests: requests_per_stage must be > 0");
    }
    Ok(())
}

/// Pick visible-token TTFT, or first-byte time when inference is requested.
///
/// Inference does not run when a streaming visible-token time is present.
/// It never substitutes full response latency.
pub fn resolve_ttft(
    streaming: bool,
    stream_ttft_s: Option<f64>,
    first_byte_s: Option<f64>,
    infer_from_first_byte: bool,
) -> ResolvedTtft {
    if streaming {
        if let Some(ttft) = stream_ttft_s {
            return ResolvedTtft {
                ttft_s: Some(ttft),
                source: Some(TtftSource::Stream),
            };
        }
    }
    if infer_from_first_byte {
        if let Some(first) = first_byte_s {
            return ResolvedTtft {
                ttft_s: Some(first),
                source: Some(TtftSource::FirstByteApprox),
            };
        }
    }
    ResolvedTtft {
        ttft_s: None,
        source: None,
    }
}

/// Resolve the first observed reasoning or visible token, otherwise the legacy
/// HTTP first-byte approximation. The fallback flag remains a compatible no-op.
pub fn resolve_ttft_with_fallback(
    streaming: bool,
    stream_ttft_s: Option<f64>,
    first_reasoning_s: Option<f64>,
    first_byte_s: Option<f64>,
    infer_from_first_byte: bool,
    _first_token_fallback: bool,
) -> ResolvedTtft {
    let first_any = [stream_ttft_s, first_reasoning_s]
        .into_iter()
        .flatten()
        .reduce(f64::min);
    resolve_ttft(streaming, first_any, first_byte_s, infer_from_first_byte)
}

/// Decide whether a chat run's TTFT coverage is publishable.
///
/// Prints one stderr warning when approximation is used, or when a
/// non-streaming run leaves TTFT unmeasured. Does not warn just because the
/// flag was parsed.
pub fn audit_chat_ttft(
    streaming: bool,
    infer_from_first_byte: bool,
    measured_successes: usize,
    missing_ttft: usize,
    approx_count: usize,
    no_output_token_errors: usize,
) -> anyhow::Result<TtftAudit> {
    if streaming && (missing_ttft > 0 || (measured_successes == 0 && no_output_token_errors > 0)) {
        if infer_from_first_byte && missing_ttft > 0 {
            anyhow::bail!(
                "streaming run could not approximate TTFT: HTTP time-to-first-byte is missing for {missing_ttft} measured success(es)"
            );
        }
        anyhow::bail!(
            "streaming run produced no visible-token TTFT. Pass --infer-ttft-from-first-byte to approximate from HTTP time-to-first-byte"
        );
    }
    if !streaming && infer_from_first_byte && missing_ttft > 0 {
        anyhow::bail!(
            "--infer-ttft-from-first-byte could not approximate TTFT: HTTP time-to-first-byte is missing for {missing_ttft} measured success(es)"
        );
    }
    if approx_count > 0 {
        let warning = format!(
            "approximated TTFT from HTTP time-to-first-byte for {approx_count} request(s); this is not visible-token TTFT"
        );
        eprintln!("warning: {warning}");
        return Ok(TtftAudit {
            approx_count,
            warning: Some(warning),
        });
    }
    if !streaming && !infer_from_first_byte && measured_successes > 0 {
        let warning = "TTFT is not measured without --streaming. Pass --infer-ttft-from-first-byte to approximate from HTTP time-to-first-byte".to_string();
        eprintln!("warning: {warning}");
        return Ok(TtftAudit {
            approx_count: 0,
            warning: Some(warning),
        });
    }
    Ok(TtftAudit {
        approx_count: 0,
        warning: None,
    })
}

/// Inputs for the completed-stream aware audit used by chat runs.
pub struct ChatTtftAuditInput {
    pub streaming: bool,
    pub infer_from_first_byte: bool,
    pub first_token_fallback: bool,
    pub allow_missing: bool,
    pub successes: usize,
    pub missing_successes: usize,
    pub no_answer: usize,
    pub unresolved_no_answer: usize,
    pub reasoning_truncated: usize,
    pub measured: usize,
    pub first_byte_count: usize,
    pub first_token_count: usize,
    pub timeout_flag: &'static str,
    pub thinking_toggle_supported: bool,
}

/// Preserve outcome errors while allowing explicit opt-ins to relax only TTFT coverage.
pub fn audit_chat_ttft_observed(input: ChatTtftAuditInput) -> anyhow::Result<TtftAudit> {
    if input.streaming && !input.allow_missing {
        let missing = input.missing_successes > 0
            || (input.successes == 0 && input.no_answer > 0 && input.unresolved_no_answer > 0);
        if missing {
            if input.measured > 0 && input.reasoning_truncated == input.measured {
                let toggle = if input.thinking_toggle_supported {
                    " or disable thinking where supported with --extra-body-json '{\"chat_template_kwargs\":{\"enable_thinking\":false}}'"
                } else {
                    " or disable thinking in the server configuration where supported"
                };
                anyhow::bail!("all requests spent the token budget on reasoning without a visible answer; raise --max-tokens and {}{}", input.timeout_flag, toggle);
            }
            anyhow::bail!("streaming run produced no actual first-token TTFT; use --allow-missing-ttft or --infer-ttft-from-first-byte for an HTTP header approximation");
        }
    }
    if !input.streaming && input.infer_from_first_byte && input.missing_successes > 0 {
        anyhow::bail!("--infer-ttft-from-first-byte could not approximate TTFT: HTTP time-to-first-byte is missing");
    }
    let warning = if input.first_byte_count > 0 {
        Some(format!("approximated TTFT from HTTP time-to-first-byte for {} request(s); this is not visible-token TTFT", input.first_byte_count))
    } else if input.first_token_count > 0 {
        Some(format!(
            "used first-token TTFT fallback for {} request(s); this is not visible-answer TTFT",
            input.first_token_count
        ))
    } else if !input.streaming && !input.infer_from_first_byte && input.successes > 0 {
        Some("TTFT is not measured without --streaming. Pass --infer-ttft-from-first-byte to approximate from HTTP time-to-first-byte".to_string())
    } else {
        None
    };
    if let Some(ref message) = warning {
        eprintln!("warning: {message}");
    }
    Ok(TtftAudit {
        approx_count: input.first_byte_count,
        warning,
    })
}

/// Construct audit input from request records, excluding warmup and drain.
pub fn audit_chat_request_records(
    records: &[crate::record::RequestRecord],
    streaming: bool,
    infer_from_first_byte: bool,
    first_token_fallback: bool,
    allow_missing: bool,
) -> anyhow::Result<TtftAudit> {
    let rows: Vec<_> = records
        .iter()
        .filter(|r| r.phase == crate::record::Phase::Measure)
        .collect();
    let no_answer = |r: &&crate::record::RequestRecord| {
        matches!(
            r.error,
            Some(
                crate::error::RequestError::NoOutputToken
                    | crate::error::RequestError::ReasoningTruncated
            )
        )
    };
    audit_chat_ttft_observed(ChatTtftAuditInput {
        streaming,
        infer_from_first_byte,
        first_token_fallback,
        allow_missing,
        successes: rows.iter().filter(|r| r.is_success()).count(),
        missing_successes: rows
            .iter()
            .filter(|r| r.is_success() && r.ttft_s.is_none())
            .count(),
        no_answer: rows.iter().copied().filter(no_answer).count(),
        unresolved_no_answer: rows
            .iter()
            .copied()
            .filter(no_answer)
            .filter(|r| r.ttft_s.is_none())
            .count(),
        reasoning_truncated: rows
            .iter()
            .filter(|r| {
                matches!(
                    r.error,
                    Some(crate::error::RequestError::ReasoningTruncated)
                )
            })
            .count(),
        measured: rows.len(),
        first_byte_count: rows
            .iter()
            .filter(|r| r.ttft_source == Some(TtftSource::FirstByteApprox))
            .count(),
        first_token_count: rows
            .iter()
            .filter(|r| r.ttft_source == Some(TtftSource::FirstTokenFallback))
            .count(),
        timeout_flag: "--request-timeout",
        thinking_toggle_supported: true,
    })
}

/// Count measured successes without TTFT and first-byte approximations.
pub fn count_ttft_gaps<'a, I>(records: I) -> (usize, usize, usize)
where
    I: IntoIterator<Item = &'a MeasuredTtftRow>,
{
    let mut successes = 0usize;
    let mut missing = 0usize;
    let mut approx = 0usize;
    for row in records {
        if !row.success || row.warmup {
            continue;
        }
        successes += 1;
        match row.ttft_source {
            Some(TtftSource::FirstByteApprox) => approx += 1,
            Some(TtftSource::Stream | TtftSource::FirstTokenFallback) => {}
            None if row.ttft_s.is_none() => missing += 1,
            None => {}
        }
    }
    (successes, missing, approx)
}

/// Thin view used by [`count_ttft_gaps`].
#[derive(Debug, Clone, Copy)]
pub struct MeasuredTtftRow {
    pub success: bool,
    pub warmup: bool,
    pub ttft_s: Option<f64>,
    pub ttft_source: Option<TtftSource>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warmup_must_leave_a_measured_request() {
        assert!(ensure_warmup_leaves_measurement(0, 4).is_ok());
        assert!(ensure_warmup_leaves_measurement(3, 4).is_ok());
        let err = ensure_warmup_leaves_measurement(4, 4).unwrap_err();
        assert!(err.to_string().contains("warmup_requests (4)"));
        assert!(err.to_string().contains("num_requests (4)"));
        assert!(ensure_warmup_leaves_measurement(5, 4).is_err());
        assert!(ensure_warmup_leaves_measurement(0, 0).is_err());
    }

    #[test]
    fn strategic_stage_needs_measured_requests() {
        assert!(ensure_requests_per_stage(1).is_ok());
        let err = ensure_requests_per_stage(0).unwrap_err();
        assert!(err.to_string().contains("requests_per_stage"));
    }

    #[test]
    fn stream_ttft_wins_over_inference() {
        let resolved = resolve_ttft(true, Some(0.2), Some(0.05), true);
        assert_eq!(resolved.ttft_s, Some(0.2));
        assert_eq!(resolved.source, Some(TtftSource::Stream));
    }

    #[test]
    fn first_any_is_independent_of_compatibility_flags() {
        for fallback in [false, true] {
            for infer in [false, true] {
                let reasoning =
                    resolve_ttft_with_fallback(true, None, Some(0.4), Some(0.02), infer, fallback);
                assert_eq!(reasoning.ttft_s, Some(0.4));
                assert_eq!(reasoning.source, Some(TtftSource::Stream));
                let both = resolve_ttft_with_fallback(
                    true,
                    Some(0.9),
                    Some(0.4),
                    Some(0.02),
                    infer,
                    fallback,
                );
                assert_eq!(both.ttft_s, Some(0.4));
                assert_eq!(both.source, Some(TtftSource::Stream));
                let visible = resolve_ttft_with_fallback(
                    true,
                    Some(0.1),
                    Some(0.4),
                    Some(0.02),
                    infer,
                    fallback,
                );
                assert_eq!(visible.ttft_s, Some(0.1));
                let absent =
                    resolve_ttft_with_fallback(true, None, None, Some(0.02), infer, fallback);
                assert_eq!(absent.ttft_s, if infer { Some(0.02) } else { None });
            }
        }
    }

    #[test]
    fn inference_copies_first_byte_and_not_e2e() {
        let resolved = resolve_ttft(false, None, Some(0.05), true);
        assert_eq!(resolved.ttft_s, Some(0.05));
        assert_eq!(resolved.source, Some(TtftSource::FirstByteApprox));
        let missing = resolve_ttft(true, None, None, true);
        assert_eq!(missing.ttft_s, None);
        assert_eq!(missing.source, None);
    }

    #[test]
    fn inference_off_leaves_ttft_empty() {
        let resolved = resolve_ttft(false, None, Some(0.05), false);
        assert_eq!(resolved.ttft_s, None);
        assert_eq!(resolved.source, None);
    }

    #[test]
    fn non_streaming_without_flag_warns_and_succeeds() {
        let audit = audit_chat_ttft(false, false, 4, 4, 0, 0).expect("ok");
        assert_eq!(audit.approx_count, 0);
        assert!(audit.warning.unwrap().contains("without --streaming"));
    }

    #[test]
    fn approximation_warns_with_count() {
        let audit = audit_chat_ttft(false, true, 3, 0, 3, 0).expect("ok");
        assert_eq!(audit.approx_count, 3);
        let warning = audit.warning.unwrap();
        assert!(warning.contains("3"));
        assert!(warning.contains("not visible-token TTFT"));
    }

    #[test]
    fn streaming_without_ttft_fails_until_inference() {
        let err = audit_chat_ttft(true, false, 0, 0, 0, 2).unwrap_err();
        assert!(err.to_string().contains("visible-token TTFT"));
        let audit = audit_chat_ttft(true, true, 2, 0, 2, 0).expect("approx");
        assert_eq!(audit.approx_count, 2);
    }

    #[test]
    fn missing_first_byte_fails_when_inference_is_on() {
        let err = audit_chat_ttft(false, true, 1, 1, 0, 0).unwrap_err();
        assert!(err.to_string().contains("time-to-first-byte is missing"));
        let err = audit_chat_ttft(true, true, 1, 1, 0, 0).unwrap_err();
        assert!(err.to_string().contains("time-to-first-byte is missing"));
    }

    #[test]
    fn flag_alone_does_not_warn() {
        let audit = audit_chat_ttft(true, true, 2, 0, 0, 0).expect("stream ttft present");
        assert!(audit.warning.is_none());
    }
}
