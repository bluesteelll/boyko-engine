//! The shared instrument: what this box's clock can and cannot express.
//!
//! Included by every bench in this crate via `#[path = "instrument.rs"] mod instrument;` rather
//! than copied, because it encodes a rule that was got WRONG once and would be got wrong again in
//! each copy.
//!
//! # The rule, and the measurement that produced it
//!
//! `log_enabled_cost` used `se.max(med * 0.02)` as its spread floor. Four separate process launches
//! then returned **byte-identical medians and byte-identical spreads** — which no live timer does.
//! Every reading was an exact integer multiple of 0.390625 ns: a 100 ns platform tick divided by a
//! 256-call block. The readings were not stable, they were QUANTIZED, and the "spread" was 2 % of
//! the answer and nothing else — a number that scales with the subject and says nothing about the
//! instrument.
//!
//! `sink_sustained_rate` had it too, at a finer quantum: 41.02 and 9.54 ns are exactly 8204 and
//! 1908 multiples of its 0.005 ns/call resolution, and both `se` values were exactly 2 % of their
//! medians. Same defect, second file — which is why the rule lives here now and not in either.
//!
//! # What a floor must be
//!
//! **The instrument's resolution, never a fraction of the reading.** A floor drawn from the subject
//! tells you how big the answer is; a floor drawn from the clock tells you how well you can see it,
//! which is the only thing a floor is for. `03-STATISTICS.md` S2 is this argument one level up: a
//! zero-expectation control measures DRIFT, not resolution.

use std::time::Instant;

/// The clock's smallest non-zero step, in nanoseconds. **Measured, never assumed.**
///
/// 100 ns is a fact about this platform, not about the bench, and a bench that hard-codes it
/// reports someone else's machine when it moves.
pub fn clock_quantum_ns() -> f64 {
    let mut smallest = f64::INFINITY;
    for _ in 0..1000 {
        let t0 = Instant::now();
        let mut d = t0.elapsed();
        while d.as_nanos() == 0 {
            d = t0.elapsed();
        }
        smallest = smallest.min(d.as_nanos() as f64);
    }
    smallest
}

/// Per-call resolution for a leg timed in blocks of `calls`.
///
/// A per-call reading cannot express a difference finer than this, however many rounds are
/// averaged: averaging identical quantized readings produces the same quantized reading.
pub fn resolution_ns(calls: u32) -> f64 {
    clock_quantum_ns() / f64::from(calls)
}

pub fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).expect("no NaN in a timing sample"));
    v[v.len() / 2]
}

pub fn quantile(sorted: &[f64], q: f64) -> f64 {
    let idx = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[idx]
}

/// Median, and a floor that is the instrument's resolution rather than a fraction of the reading.
///
/// Half a quantum is the smallest difference this clock can express; nothing below it is a
/// measurement, whatever the arithmetic says.
pub fn med_and_floor(samples: &mut [f64], resolution: f64) -> (f64, f64) {
    let med = median(samples);
    let iqr = quantile(samples, 0.75) - quantile(samples, 0.25);
    let se = iqr / (samples.len() as f64).sqrt();
    (med, se.max(resolution / 2.0))
}

/// Whether a reading is indistinguishable from the instrument's own floor.
///
/// Two quanta, not one: a leg that lands on the first or second tick is reporting the clock, and
/// which of the two it lands on is not a property of the code.
pub fn at_floor(med: f64, resolution: f64) -> bool {
    med <= resolution * 2.0
}
