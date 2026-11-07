// Copyright (c) 2026 Metrum AI, Inc. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Exact source trace time normalization. Not runtime request-point selection.
//! No epoch inference, implicit rebasing, sorting or actual scheduling. Relative
//! offsets require an explicit origin and subtract exact decimals before rounding.

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceTimeUnit {
    Seconds,
    Milliseconds,
    Microseconds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceTimeScale {
    pub numerator: u64,
    pub denominator: u64,
}

impl Default for TraceTimeScale {
    fn default() -> Self {
        Self {
            numerator: 1,
            denominator: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FractionalMillis {
    Reject,
    Floor,
}

/// Source metadata may be private; this is not automatically public evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedTraceTime {
    pub source_value: String,
    pub source_unit: TraceTimeUnit,
    pub scale: TraceTimeScale,
    pub rounding: FractionalMillis,
    pub milliseconds: u64,
    pub discarded_fraction: bool,
}

/// Accept an exact nonnegative decimal lexeme, never an f64 conversion.
/// Supports at most 18 fractional digits and checked u128 intermediate arithmetic.
/// Oversize, invalid, overflowing or unsupported precision fails without clipping.
/// Callers must retain the numeric lexeme when decoding JSON/CSV source records.
pub fn normalize_trace_time(
    value: &str,
    unit: TraceTimeUnit,
    scale: TraceTimeScale,
    rounding: FractionalMillis,
) -> Result<NormalizedTraceTime> {
    ensure!(
        scale.numerator > 0 && scale.denominator > 0,
        "trace time scale must be positive"
    );
    let (mut numerator, mut denominator) = decimal_ratio(value)?;
    let (unit_n, unit_d) = match unit {
        TraceTimeUnit::Seconds => (1000, 1),
        TraceTimeUnit::Milliseconds => (1, 1),
        TraceTimeUnit::Microseconds => (1, 1000),
    };
    (numerator, denominator) = multiply_ratio(numerator, denominator, unit_n, unit_d)?;
    (numerator, denominator) = multiply_ratio(
        numerator,
        denominator,
        scale.numerator.into(),
        scale.denominator.into(),
    )?;
    let discarded_fraction = numerator % denominator != 0;
    ensure!(
        !discarded_fraction || rounding == FractionalMillis::Floor,
        "trace timestamp has sub-millisecond precision; explicit flooring required"
    );
    let milliseconds =
        u64::try_from(numerator / denominator).context("trace milliseconds exceed u64")?;
    Ok(NormalizedTraceTime {
        source_value: value.to_owned(),
        source_unit: unit,
        scale,
        rounding,
        milliseconds,
        discarded_fraction,
    })
}

/// Declared source-relative offset, not an achieved arrival or issuance time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedTraceOffset {
    pub source_value: String,
    pub origin_value: String,
    pub source_unit: TraceTimeUnit,
    pub scale: TraceTimeScale,
    pub rounding: FractionalMillis,
    pub offset_milliseconds: u64,
    pub discarded_fraction: bool,
}

/// Subtract explicit source origin BEFORE unit conversion/scaling/rounding.
/// Never subtract independently rounded timestamps. Fail on negative differences,
/// oversized lexemes, checked arithmetic overflow or undeclared precision loss.
pub fn normalize_trace_offset(
    value: &str,
    origin: &str,
    unit: TraceTimeUnit,
    scale: TraceTimeScale,
    rounding: FractionalMillis,
) -> Result<NormalizedTraceOffset> {
    ensure!(
        scale.numerator > 0 && scale.denominator > 0,
        "trace time scale must be positive"
    );
    let (n, d) = decimal_ratio(value)?;
    let (on, od) = decimal_ratio(origin)?;
    let common = gcd(d, od);
    let left = n
        .checked_mul(od / common)
        .context("trace origin subtraction overflow")?;
    let right = on
        .checked_mul(d / common)
        .context("trace origin subtraction overflow")?;
    let numerator = left
        .checked_sub(right)
        .context("source timestamp precedes explicit origin")?;
    let denominator = d
        .checked_mul(od / common)
        .context("trace origin denominator overflow")?;
    let common = gcd(numerator, denominator);
    let (unit_n, unit_d) = match unit {
        TraceTimeUnit::Seconds => (1000, 1),
        TraceTimeUnit::Milliseconds => (1, 1),
        TraceTimeUnit::Microseconds => (1, 1000),
    };
    let (n, d) = multiply_ratio(numerator / common, denominator / common, unit_n, unit_d)?;
    let (n, d) = multiply_ratio(n, d, scale.numerator.into(), scale.denominator.into())?;
    let discarded_fraction = n % d != 0;
    ensure!(
        !discarded_fraction || rounding == FractionalMillis::Floor,
        "trace offset has sub-millisecond precision; explicit flooring required"
    );
    Ok(NormalizedTraceOffset {
        source_value: value.into(),
        origin_value: origin.into(),
        source_unit: unit,
        scale,
        rounding,
        offset_milliseconds: u64::try_from(n / d)
            .context("trace offset milliseconds exceed u64")?,
        discarded_fraction,
    })
}

fn decimal_ratio(value: &str) -> Result<(u128, u128)> {
    ensure!(
        !value.is_empty() && value.len() <= 64,
        "invalid trace timestamp length"
    );
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    ensure!(
        !whole.is_empty() && whole.bytes().all(|b| b.is_ascii_digit()),
        "invalid trace timestamp integer"
    );
    ensure!(
        fraction.len() <= 18 && fraction.bytes().all(|b| b.is_ascii_digit()),
        "invalid trace timestamp fraction"
    );
    ensure!(
        !value.contains('.') || !fraction.is_empty(),
        "empty trace timestamp fraction"
    );
    let d = 10u128.pow(fraction.len() as u32);
    let integer: u128 = whole.parse().context("trace timestamp integer overflow")?;
    let fractional: u128 = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse()
            .context("trace timestamp fraction overflow")?
    };
    let n = integer
        .checked_mul(d)
        .and_then(|n| n.checked_add(fractional))
        .context("trace timestamp overflow")?;
    let common = gcd(n, d);
    Ok((n / common, d / common))
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn multiply_ratio(n: u128, d: u128, factor_n: u128, factor_d: u128) -> Result<(u128, u128)> {
    // Cross-cancel before multiplying so large reducible ratios remain usable.
    let left = gcd(n, factor_d);
    let right = gcd(factor_n, d);
    Ok((
        (n / left)
            .checked_mul(factor_n / right)
            .context("trace time numerator overflow")?,
        (d / right)
            .checked_mul(factor_d / left)
            .context("trace time denominator overflow")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_origin_subtraction_precedes_rounding() {
        let scale = TraceTimeScale::default();
        let offset = normalize_trace_offset(
            "1.0015",
            "1.0007",
            TraceTimeUnit::Seconds,
            scale,
            FractionalMillis::Floor,
        )
        .unwrap();
        assert_eq!(offset.offset_milliseconds, 0);
        assert!(offset.discarded_fraction);
        let wrong = normalize_trace_time(
            "1.0015",
            TraceTimeUnit::Seconds,
            scale,
            FractionalMillis::Floor,
        )
        .unwrap()
        .milliseconds
            - normalize_trace_time(
                "1.0007",
                TraceTimeUnit::Seconds,
                scale,
                FractionalMillis::Floor,
            )
            .unwrap()
            .milliseconds;
        assert_eq!(wrong, 1); // The regression: individually rounded origins lose phase.
        assert!(normalize_trace_offset(
            "1.0015",
            "1.0007",
            TraceTimeUnit::Seconds,
            scale,
            FractionalMillis::Reject
        )
        .is_err());
        assert_eq!(offset.origin_value, "1.0007");
        assert_eq!(
            serde_json::from_value::<NormalizedTraceOffset>(serde_json::to_value(&offset).unwrap())
                .unwrap(),
            offset
        );
    }

    #[test]
    fn relative_large_origin_units_scale_and_equal_time_are_exact() {
        for (value, origin, unit, scale, expected) in [
            (
                "100000000000000000001",
                "100000000000000000000",
                TraceTimeUnit::Seconds,
                TraceTimeScale::default(),
                1000,
            ),
            (
                "10",
                "7",
                TraceTimeUnit::Seconds,
                TraceTimeScale {
                    numerator: 1,
                    denominator: 3,
                },
                1000,
            ),
            (
                "1250",
                "250",
                TraceTimeUnit::Microseconds,
                TraceTimeScale::default(),
                1,
            ),
            (
                "0001.000",
                "1",
                TraceTimeUnit::Milliseconds,
                TraceTimeScale::default(),
                0,
            ),
        ] {
            let offset =
                normalize_trace_offset(value, origin, unit, scale, FractionalMillis::Reject)
                    .unwrap();
            assert_eq!(offset.offset_milliseconds, expected);
            assert!(!offset.discarded_fraction);
        }
    }

    #[test]
    fn relative_invalid_negative_overflow_or_precision_fails_without_inference() {
        for (value, origin, scale) in [
            ("1", "2", TraceTimeScale::default()),
            ("1", "-1", TraceTimeScale::default()),
            (
                "1",
                "0",
                TraceTimeScale {
                    numerator: 1,
                    denominator: 0,
                },
            ),
            ("1", "0.0000000000000000001", TraceTimeScale::default()),
            ("18446744073709551616", "0", TraceTimeScale::default()),
        ] {
            assert!(normalize_trace_offset(
                value,
                origin,
                TraceTimeUnit::Seconds,
                scale,
                FractionalMillis::Reject
            )
            .is_err());
        }
    }

    #[test]
    fn exact_units_scaling_and_lexeme_preservation() {
        let normalized = normalize_trace_time(
            "92.516",
            TraceTimeUnit::Seconds,
            TraceTimeScale::default(),
            FractionalMillis::Reject,
        )
        .unwrap();
        assert_eq!(normalized.milliseconds, 92_516);
        assert_eq!(normalized.source_value, "92.516");
        assert!(!normalized.discarded_fraction);
        assert_eq!(
            normalize_trace_time(
                "1.5",
                TraceTimeUnit::Seconds,
                TraceTimeScale {
                    numerator: 2,
                    denominator: 3
                },
                FractionalMillis::Reject
            )
            .unwrap()
            .milliseconds,
            1000
        );
        assert_eq!(
            normalize_trace_time(
                "0001.000",
                TraceTimeUnit::Milliseconds,
                TraceTimeScale::default(),
                FractionalMillis::Reject
            )
            .unwrap()
            .milliseconds,
            1
        );
        let max = u64::MAX.to_string();
        assert_eq!(
            normalize_trace_time(
                &max,
                TraceTimeUnit::Seconds,
                TraceTimeScale {
                    numerator: 1,
                    denominator: 1000
                },
                FractionalMillis::Reject
            )
            .unwrap()
            .milliseconds,
            u64::MAX
        );
        assert_eq!(
            normalize_trace_time(
                "0.000000",
                TraceTimeUnit::Seconds,
                TraceTimeScale::default(),
                FractionalMillis::Reject
            )
            .unwrap()
            .milliseconds,
            0
        );
    }

    #[test]
    fn sub_millisecond_loss_requires_explicit_policy() {
        for (value, unit, expected) in [
            ("1250", TraceTimeUnit::Microseconds, 1),
            ("0.0009", TraceTimeUnit::Seconds, 0),
        ] {
            assert!(normalize_trace_time(
                value,
                unit,
                TraceTimeScale::default(),
                FractionalMillis::Reject
            )
            .is_err());
            let normalized = normalize_trace_time(
                value,
                unit,
                TraceTimeScale::default(),
                FractionalMillis::Floor,
            )
            .unwrap();
            assert_eq!(normalized.milliseconds, expected);
            assert!(normalized.discarded_fraction);
            assert_eq!(normalized.source_unit, unit);
        }
    }

    #[test]
    fn malformed_overflow_and_unknown_unit_fail_closed() {
        for value in [
            "",
            "-1",
            "+1",
            "1e3",
            "NaN",
            ".5",
            "1.",
            " 1",
            "1.2.3",
            "1.0000000000000000001",
            "18446744073709551616",
        ] {
            assert!(
                normalize_trace_time(
                    value,
                    TraceTimeUnit::Milliseconds,
                    TraceTimeScale::default(),
                    FractionalMillis::Reject
                )
                .is_err(),
                "invalid fixture should fail"
            );
        }
        assert!(normalize_trace_time(
            "1",
            TraceTimeUnit::Seconds,
            TraceTimeScale {
                numerator: 0,
                denominator: 1
            },
            FractionalMillis::Reject
        )
        .is_err());
        assert!(normalize_trace_time(
            "1",
            TraceTimeUnit::Seconds,
            TraceTimeScale {
                numerator: 1,
                denominator: 0
            },
            FractionalMillis::Reject
        )
        .is_err());
        assert!(serde_json::from_str::<TraceTimeUnit>("\"inferred\"").is_err());
        assert!(normalize_trace_time(
            &"9".repeat(65),
            TraceTimeUnit::Milliseconds,
            TraceTimeScale::default(),
            FractionalMillis::Floor
        )
        .is_err());
    }
}
