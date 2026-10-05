// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Time-weighted run metrics from request intervals (Metrum AI Bench, #195).
//!
//! Each measured success is an interval `[send, send + latency]`, split at
//! the first generated token into a prefill and a decode phase. A sweep line
//! over those intervals gives step functions (requests in flight, tokens in
//! flight, aggregate token rates) whose time-weighted averages are reported
//! over the run window. `observed_concurrency` stays the client semaphore
//! view sampled at each send; these blocks weight every instant equally.

use serde::Serialize;

/// One measured success, in seconds relative to any shared epoch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RequestSpan {
    /// Send time.
    pub start_s: f64,
    /// Send to completion.
    pub latency_s: f64,
    /// Send to first generated token (prefill end); `None` when the row has
    /// no stream-measured first token (unary, or first-byte TTFT).
    pub prefill_end_s: Option<f64>,
    /// Prompt tokens; `None` when the row has no token accounting.
    pub input_tokens: Option<u64>,
    /// Completion tokens; `None` when the row has no token accounting.
    pub output_tokens: Option<u64>,
}

/// Time-weighted statistics of one step function over the run window.
///
/// `n` counts the requests that contribute. Every other field is `null`
/// when `n = 0` or the window is empty: not applicable, never zero.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct TimeWeightedStat {
    /// Requests eligible for the block (before clipping to the window).
    pub n: usize,
    /// Integral over the window divided by `window_seconds`.
    pub avg: Option<f64>,
    /// Integral divided by `active_s` (time with at least one contributing
    /// request open); `null` when `active_s = 0`.
    pub active_avg: Option<f64>,
    /// Peak instantaneous value inside the window.
    pub max: Option<f64>,
    /// Seconds of the window with at least one contributing request open.
    pub active_s: Option<f64>,
}

/// The six time-weighted blocks stamped on `summary.v3` and strategic points.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct TimeWeightedMetrics {
    /// Requests in flight (send to completion), every measured success.
    pub effective_concurrency: TimeWeightedStat,
    /// Requests in prefill (send to first generated token).
    pub effective_prefill_concurrency: TimeWeightedStat,
    /// Requests in decode (first generated token to completion).
    pub effective_decode_concurrency: TimeWeightedStat,
    /// KV-cache occupancy proxy: prompt tokens during prefill, plus output
    /// tokens accrued linearly across decode.
    pub tokens_in_flight: TimeWeightedStat,
    /// Aggregate prompt tokens/s, each request's prompt spread uniformly over its prefill.
    pub effective_prefill_throughput: TimeWeightedStat,
    /// Aggregate output tokens/s, each request's output spread uniformly over its decode.
    pub effective_decode_throughput: TimeWeightedStat,
}

/// A piece of one request's contribution: `level + slope * (t - start)` on `[start, end)`.
#[derive(Debug, Clone, Copy)]
struct Segment {
    start: f64,
    end: f64,
    level: f64,
    slope: f64,
}

impl Segment {
    fn flat(start: f64, end: f64, level: f64) -> Self {
        Self {
            start,
            end,
            level,
            slope: 0.0,
        }
    }
}

/// Compute all blocks from `spans` over the window `[window_start_s, window_start_s + window_s]`.
///
/// Intervals are clipped to the window, so a request still open at the
/// window end only contributes its in-window part.
pub fn compute(spans: &[RequestSpan], window_start_s: f64, window_s: f64) -> TimeWeightedMetrics {
    let mut all = Vec::new();
    let mut prefill = Vec::new();
    let mut decode = Vec::new();
    let mut tokens = Vec::new();
    let mut prefill_rate = Vec::new();
    let mut decode_rate = Vec::new();
    let (mut n_split, mut n_tokens, mut n_prefill_rate, mut n_decode_rate) = (0, 0, 0, 0);
    for span in spans {
        if !span.start_s.is_finite() || !span.latency_s.is_finite() || span.latency_s < 0.0 {
            continue;
        }
        let start = span.start_s - window_start_s;
        let end = start + span.latency_s;
        all.push(Segment::flat(start, end, 1.0));
        let Some(first) = span
            .prefill_end_s
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(0.0, span.latency_s))
        else {
            continue;
        };
        n_split += 1;
        let split = start + first;
        let decode_s = span.latency_s - first;
        prefill.push(Segment::flat(start, split, 1.0));
        decode.push(Segment::flat(split, end, 1.0));
        if let (Some(isl), Some(osl)) = (span.input_tokens, span.output_tokens) {
            n_tokens += 1;
            let isl = isl as f64;
            tokens.push(Segment::flat(start, split, isl));
            tokens.push(Segment {
                start: split,
                end,
                level: isl,
                slope: if decode_s > 0.0 {
                    osl as f64 / decode_s
                } else {
                    0.0
                },
            });
        }
        // A phase of zero length has no defined rate; the row is skipped.
        if let Some(isl) = span.input_tokens.filter(|&t| t > 0) {
            if first > 0.0 {
                n_prefill_rate += 1;
                prefill_rate.push(Segment::flat(start, split, isl as f64 / first));
            }
        }
        if let Some(osl) = span.output_tokens.filter(|&t| t > 0) {
            if decode_s > 0.0 {
                n_decode_rate += 1;
                decode_rate.push(Segment::flat(split, end, osl as f64 / decode_s));
            }
        }
    }
    TimeWeightedMetrics {
        effective_concurrency: sweep(&all, all.len(), window_s),
        effective_prefill_concurrency: sweep(&prefill, n_split, window_s),
        effective_decode_concurrency: sweep(&decode, n_split, window_s),
        tokens_in_flight: sweep(&tokens, n_tokens, window_s),
        effective_prefill_throughput: sweep(&prefill_rate, n_prefill_rate, window_s),
        effective_decode_throughput: sweep(&decode_rate, n_decode_rate, window_s),
    }
}

/// Sweep-line statistics of the sum of `segments` over `[0, window_s]`.
fn sweep(segments: &[Segment], n: usize, window_s: f64) -> TimeWeightedStat {
    if n == 0 || !window_s.is_finite() || window_s <= 0.0 {
        return TimeWeightedStat {
            n,
            ..TimeWeightedStat::default()
        };
    }
    // Value inside the window is `a + b * t`; events carry the deltas.
    let mut events: Vec<(f64, i64, f64, f64)> = Vec::with_capacity(segments.len() * 2);
    let mut integral = 0.0;
    for seg in segments {
        let start = seg.start.max(0.0);
        let end = seg.end.min(window_s);
        if end <= start {
            continue;
        }
        let level = seg.level + seg.slope * (start - seg.start);
        let width = end - start;
        integral += width * level + seg.slope * width * width / 2.0;
        let a = level - seg.slope * start;
        events.push((start, 1, a, seg.slope));
        events.push((end, -1, -a, -seg.slope));
    }
    events.sort_by(|x, y| x.0.total_cmp(&y.0));
    let (mut open, mut a, mut b) = (0i64, 0.0f64, 0.0f64);
    let mut max = 0.0f64;
    let mut active_s = 0.0;
    let mut i = 0;
    while i < events.len() {
        let t = events[i].0;
        if open > 0 {
            max = max.max(a + b * t);
        }
        while i < events.len() && events[i].0 == t {
            open += events[i].1;
            a += events[i].2;
            b += events[i].3;
            i += 1;
        }
        if open > 0 {
            max = max.max(a + b * t);
            if let Some(next) = events.get(i) {
                active_s += next.0 - t;
            }
        } else {
            // Drop float residue so a closed gap restarts from exactly zero.
            a = 0.0;
            b = 0.0;
        }
    }
    TimeWeightedStat {
        n,
        avg: Some(integral / window_s),
        active_avg: (active_s > 0.0).then(|| integral / active_s),
        max: Some(max),
        active_s: Some(active_s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: f64, latency: f64, first: Option<f64>, isl: u64, osl: u64) -> RequestSpan {
        RequestSpan {
            start_s: start,
            latency_s: latency,
            prefill_end_s: first,
            input_tokens: Some(isl),
            output_tokens: Some(osl),
        }
    }

    fn assert_stat(
        stat: &TimeWeightedStat,
        n: usize,
        avg: f64,
        active: f64,
        max: f64,
        active_s: f64,
    ) {
        assert_eq!(stat.n, n, "{stat:?}");
        let close = |got: Option<f64>, want: f64| {
            let got = got.unwrap_or_else(|| panic!("null in {stat:?}"));
            assert!((got - want).abs() < 1e-12, "{got} != {want} in {stat:?}");
        };
        close(stat.avg, avg);
        close(stat.active_avg, active);
        close(stat.max, max);
        close(stat.active_s, active_s);
    }

    /// Two overlapping requests in a 4 s window, then 1 s idle:
    /// A = [0, 2], first token at 0.5, 100 in / 30 out.
    /// B = [1, 3], first token at 1.5 (abs 2.5), 50 in / 10 out.
    #[test]
    fn synthetic_intervals_give_exact_values() {
        let spans = [
            span(10.0, 2.0, Some(0.5), 100, 30),
            span(11.0, 2.0, Some(1.5), 50, 10),
        ];
        let m = compute(&spans, 10.0, 4.0);
        // In flight: 1 on [0,1), 2 on [1,2), 1 on [2,3). Integral 4.
        assert_stat(&m.effective_concurrency, 2, 1.0, 4.0 / 3.0, 2.0, 3.0);
        // Prefill: A [0,0.5), B [1,2.5). Integral 2, active 2.
        assert_stat(&m.effective_prefill_concurrency, 2, 0.5, 1.0, 1.0, 2.0);
        // Decode: A [0.5,2), B [2.5,3). Integral 2, active 2.
        assert_stat(&m.effective_decode_concurrency, 2, 0.5, 1.0, 1.0, 2.0);
        // Prefill rates: A 200 tok/s for 0.5 s, B 100/3 tok/s for 1.5 s; total 150 tokens.
        assert_stat(&m.effective_prefill_throughput, 2, 37.5, 75.0, 200.0, 2.0);
        // Decode rates: A 20 tok/s for 1.5 s, B 20 tok/s for 0.5 s; total 40 tokens.
        assert_stat(&m.effective_decode_throughput, 2, 10.0, 20.0, 20.0, 2.0);
        // Tokens: A = 100*2 + 30*1.5/2 = 222.5; B = 50*2 + 10*0.5/2 = 102.5.
        // Peak just before t = 2: A at 100 + 30 = 130, B in prefill at 50.
        assert_stat(&m.tokens_in_flight, 2, 325.0 / 4.0, 325.0 / 3.0, 180.0, 3.0);
    }

    /// Little's law: unclipped effective concurrency equals throughput times mean latency.
    #[test]
    fn effective_concurrency_matches_littles_law() {
        let spans = [
            span(0.0, 1.0, None, 0, 0),
            span(0.25, 0.5, None, 0, 0),
            span(1.0, 1.0, None, 0, 0),
        ];
        let m = compute(&spans, 0.0, 2.0);
        let rps = 3.0 / 2.0;
        let mean_latency = 2.5 / 3.0;
        assert!((m.effective_concurrency.avg.unwrap() - rps * mean_latency).abs() < 1e-12);
        assert_eq!(m.effective_concurrency.max, Some(2.0));
        // No first-token split: phase blocks are not applicable.
        assert_eq!(m.effective_prefill_concurrency, TimeWeightedStat::default());
        assert_eq!(m.tokens_in_flight, TimeWeightedStat::default());
    }

    #[test]
    fn intervals_are_clipped_to_the_window() {
        // [1, 3] in a [0, 2] window: half inside.
        let m = compute(&[span(1.0, 2.0, Some(1.0), 10, 8)], 0.0, 2.0);
        assert_stat(&m.effective_concurrency, 1, 0.5, 1.0, 1.0, 1.0);
        // Prefill [1,2) fully in; decode [2,3) fully out.
        assert_stat(&m.effective_prefill_throughput, 1, 5.0, 10.0, 10.0, 1.0);
        assert_eq!(m.effective_decode_throughput.n, 1);
        assert_eq!(m.effective_decode_throughput.avg, Some(0.0));
        assert_eq!(m.effective_decode_throughput.active_avg, None);
    }

    #[test]
    fn empty_and_degenerate_inputs_report_null() {
        assert_eq!(compute(&[], 0.0, 1.0), TimeWeightedMetrics::default());
        let m = compute(&[span(0.0, 1.0, Some(0.5), 1, 1)], 0.0, 0.0);
        assert_eq!(m.effective_concurrency.n, 1);
        assert_eq!(m.effective_concurrency.avg, None);
        // Zero-length decode: no rate, but concurrency still counts the row.
        let m = compute(&[span(0.0, 1.0, Some(1.0), 4, 2)], 0.0, 1.0);
        assert_eq!(m.effective_decode_throughput.n, 0);
        assert_stat(&m.effective_prefill_throughput, 1, 4.0, 4.0, 4.0, 1.0);
        assert_stat(&m.tokens_in_flight, 1, 4.0, 4.0, 4.0, 1.0);
        // Rows without token accounting skip the token blocks only.
        let unaccounted = RequestSpan {
            input_tokens: None,
            output_tokens: None,
            ..span(0.0, 1.0, Some(0.5), 0, 0)
        };
        let m = compute(&[unaccounted], 0.0, 1.0);
        assert_eq!(m.effective_decode_concurrency.n, 1);
        assert_eq!(m.tokens_in_flight.n, 0);
        assert_eq!(m.effective_prefill_throughput.n, 0);
    }

    #[test]
    fn first_token_is_clamped_into_the_request() {
        let m = compute(&[span(0.0, 1.0, Some(5.0), 2, 0)], 0.0, 1.0);
        assert_stat(&m.effective_prefill_concurrency, 1, 1.0, 1.0, 1.0, 1.0);
        assert_eq!(m.effective_decode_concurrency.active_s, Some(0.0));
    }
}
