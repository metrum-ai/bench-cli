// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Arrival scheduling: closed-loop concurrency and open-loop constant/Poisson.

use rand::rngs::StdRng;
use rand::Rng;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrivalKind {
    ClosedLoop,
    Constant,
    Poisson,
}

#[derive(Debug, Clone)]
pub struct RequestSlot {
    pub seq: u64,
    pub scheduled_delay: Duration,
}

/// Issue `n` slots. Closed-loop: all delays zero (caller gates on semaphore).
/// Open-loop: delays from a constant or exponential inter-arrival.
pub fn schedule(
    kind: ArrivalKind,
    n: u64,
    request_rate: f64,
    rng: &mut StdRng,
) -> Vec<RequestSlot> {
    match kind {
        ArrivalKind::ClosedLoop => (0..n)
            .map(|seq| RequestSlot {
                seq,
                scheduled_delay: Duration::ZERO,
            })
            .collect(),
        ArrivalKind::Constant => {
            let dt = if request_rate > 0.0 {
                1.0 / request_rate
            } else {
                0.0
            };
            (0..n)
                .map(|seq| RequestSlot {
                    seq,
                    scheduled_delay: Duration::from_secs_f64(seq as f64 * dt),
                })
                .collect()
        }
        ArrivalKind::Poisson => {
            let mut t = 0.0;
            let lambda = request_rate.max(0.0);
            (0..n)
                .map(|seq| {
                    if seq > 0 && lambda > 0.0 {
                        let u: f64 = rng.random::<f64>().clamp(f64::EPSILON, 1.0);
                        t += -u.ln() / lambda;
                    }
                    RequestSlot {
                        seq,
                        scheduled_delay: Duration::from_secs_f64(t),
                    }
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn closed_loop_zero_delays() {
        let mut rng = StdRng::seed_from_u64(1);
        let slots = schedule(ArrivalKind::ClosedLoop, 5, 0.0, &mut rng);
        assert!(slots.iter().all(|s| s.scheduled_delay.is_zero()));
    }

    #[test]
    fn constant_rate_evenly_spaced() {
        let mut rng = StdRng::seed_from_u64(1);
        let slots = schedule(ArrivalKind::Constant, 3, 10.0, &mut rng);
        assert_eq!(slots[1].scheduled_delay, Duration::from_millis(100));
        assert_eq!(slots[2].scheduled_delay, Duration::from_millis(200));
    }

    #[test]
    fn poisson_seeded_is_deterministic() {
        let mut a = StdRng::seed_from_u64(42);
        let mut b = StdRng::seed_from_u64(42);
        let sa = schedule(ArrivalKind::Poisson, 20, 5.0, &mut a);
        let sb = schedule(ArrivalKind::Poisson, 20, 5.0, &mut b);
        assert_eq!(
            sa.iter().map(|s| s.scheduled_delay).collect::<Vec<_>>(),
            sb.iter().map(|s| s.scheduled_delay).collect::<Vec<_>>()
        );
    }

    #[test]
    fn poisson_arrivals_have_expected_mean_interval() {
        let mut rng = StdRng::seed_from_u64(42);
        let slots = schedule(ArrivalKind::Poisson, 10_001, 20.0, &mut rng);
        let intervals: Vec<f64> = slots
            .windows(2)
            .map(|pair| {
                pair[1]
                    .scheduled_delay
                    .saturating_sub(pair[0].scheduled_delay)
                    .as_secs_f64()
            })
            .collect();
        let mean = intervals.iter().sum::<f64>() / intervals.len() as f64;
        assert!((mean - 0.05).abs() < 0.002, "mean={mean}");
    }
}
