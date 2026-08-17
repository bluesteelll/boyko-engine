//! `sink_sustained_rate` vs `sink_sustained_rate_binary` — **the revert clause's own instrument**.
//!
//! The clause it was built to answer read `≥ 5×`, and it FIRED: four sittings measured
//! 4.30× / 4.63× / 4.68× / 4.54×. The owner ruled (2026-08-17) that the `5×` was an estimate
//! written before anything was measured, not a requirement, and kept L13b.
//!
//! **So the bound below is a REGRESSION GUARD, not an acceptance threshold.** `≥ 4.0×` sits below
//! the observed minimum of `4.30×` with margin for a slower box; its job is to catch the format
//! losing what it has. It is deliberately NOT pinned to the measured value: a bound set to today's
//! number reds on ordinary variance, and a gate that cries wolf is a gate that gets ignored.
//!
//! # What is measured, stated exactly, because the honest scope is narrower than the row's name
//!
//! The two paths are IDENTICAL from the macro gate down to the drain and identical again from the
//! sink's `write` down to the OS. They differ in one place: what turns a record's payload into
//! bytes. So that is what is timed — `record::render_payload` against `binary::encode_record`,
//! over the same payload, in one sitting, interleaved.
//!
//! This is deliberately **not** reported as an end-to-end sink rate, because the binary sink has no
//! destination of its own yet. A bench that timed a shared file write on both legs would dilute the
//! ratio with a constant both formats pay, and the revert clause is a question about the format.
//! Reporting a diluted number as the row's verdict would be the same defect this campaign chases:
//! a measurement whose name claims more than it measured.
//!
//! # Why the ratio and not the two absolutes
//!
//! An absolute records·s⁻¹ on one box is a fact about the box. The clause's threshold is a RATIO,
//! measured in one sitting, which is the form that survives a different machine — and both legs
//! ride the same drift, so the drift cancels in the quotient rather than being modelled.

use std::hint::black_box;
use std::time::Instant;

use boyko_log::record::DspBuf;
use boyko_log::sink::binary::{RECORD_HEADER_BYTES, RecordFrame, encode_record};
use boyko_log::site::LogFormatter;

/// Records per timed leg. Large enough that the clock's own resolution is far below the reading,
/// small enough that a round is short against a scheduler slice.
const CALLS: u32 = 20_000;

/// The regression guard, set from four measured sittings (4.30-4.68x) rather than estimated.
///
/// Below the observed MINIMUM, with margin, because a slower box shifts both legs and a bound
/// pinned to the fastest reading would red on a machine that is merely different.
const MIN_RATIO: f64 = 4.0;

/// Rounds in the sitting. Legs are interleaved, never blocked, so a thermal or frequency drift
/// lands on both and cancels in the ratio.
const ROUNDS: usize = 41;

/// Payload shape: a `u32` and a short `&str`, which is the ladder's `log_enabled_2u32`/`_str32`
/// middle ground and the commonest real record.
fn payload_bytes() -> Vec<u8> {
    // Tag bytes mirror `record`'s value encoding: this is the shape a drain hands to a sink.
    let mut v = Vec::new();
    v.push(2u8); // u32 tag
    v.extend_from_slice(&42_424_242u32.to_le_bytes());
    v.push(9u8); // str tag
    let s = b"frame budget exceeded";
    v.push(s.len() as u8);
    v.extend_from_slice(s);
    v
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).expect("no NaN in a timing sample"));
    v[v.len() / 2]
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    let idx = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[idx]
}

/// Median and a spread floor, per `03-STATISTICS.md` S2.
///
/// The floor is mandatory and not decoration: a control whose expected value is exactly zero
/// measures DRIFT, not RESOLUTION, and a verdict drawn from the raw spread of such a control
/// reports the instrument's own wobble as a result.
fn se_floor(samples: &mut [f64]) -> (f64, f64) {
    let med = median(samples);
    let iqr = quantile(samples, 0.75) - quantile(samples, 0.25);
    let se = iqr / (samples.len() as f64).sqrt();
    (med, se.max(med * 0.02))
}

#[inline(never)]
fn time<F: FnMut()>(mut f: F) -> f64 {
    let mut barrier = 0u8;
    let t0 = Instant::now();
    for _ in 0..CALLS {
        black_box(&mut barrier);
        f();
    }
    let ns = t0.elapsed().as_nanos() as f64 / f64::from(CALLS);
    black_box(barrier);
    ns
}

fn main() {
    let payload = payload_bytes();
    let mut text_buf = DspBuf::<512>::new();
    let mut bin_buf = [0u8; 512];

    let mut text = Vec::with_capacity(ROUNDS);
    let mut binary = Vec::with_capacity(ROUNDS);
    let mut text2 = Vec::with_capacity(ROUNDS);

    for _ in 0..ROUNDS {
        // A-B-A': the twin is the same leg measured twice around the other, so a drift that would
        // otherwise look like a difference between formats shows up as a difference between A and
        // A' instead -- where it cannot be mistaken for the result.
        text.push(time(|| {
            text_buf.clear();
            let mut f = LogFormatter::new(&mut text_buf);
            boyko_log::record::render_payload(black_box(&payload), "budget {} {}", &mut f);
            black_box(&text_buf);
        }));
        binary.push(time(|| {
            let frame = RecordFrame {
                site_id: 7,
                tsc_delta: 1_234_567,
                flags: 0,
                epoch_lo: 0,
                payload: black_box(&payload),
            };
            let n = encode_record(black_box(&mut bin_buf), &frame);
            black_box(n);
        }));
        text2.push(time(|| {
            text_buf.clear();
            let mut f = LogFormatter::new(&mut text_buf);
            boyko_log::record::render_payload(black_box(&payload), "budget {} {}", &mut f);
            black_box(&text_buf);
        }));
    }

    let (med_text, se_text) = se_floor(&mut text);
    let (med_bin, se_bin) = se_floor(&mut binary);
    let (med_text2, _) = se_floor(&mut text2);

    let twin_gap = (med_text - med_text2).abs();
    let ratio = med_text / med_bin;
    let rate_text = 1e9 / med_text;
    let rate_bin = 1e9 / med_bin;

    println!("sink_sustained_rate         : {med_text:8.2} ns/rec  ({rate_text:>12.0} rec/s)");
    println!("sink_sustained_rate_binary  : {med_bin:8.2} ns/rec  ({rate_bin:>12.0} rec/s)");
    println!("  se(text)={se_text:.3} ns  se(binary)={se_bin:.3} ns  A-vs-A' gap={twin_gap:.3} ns");
    println!("  frame header = {RECORD_HEADER_BYTES} B, payload = {} B", payload.len());
    println!("  ratio text/binary = {ratio:.2}x   (guard: >= {MIN_RATIO}x, and >= 3 M rec/s)");

    // The twin decides whether the instrument measured anything. If A and A' differ by more than
    // the difference under test, the sitting drifted and the ratio is a fact about the machine's
    // clock, not about the formats.
    let separation = (med_text - med_bin).abs();
    let verdict = if twin_gap > separation {
        "NOT MEASURABLE (instrument): the A-vs-A' twin drifted further than the legs differ"
    } else if separation < (se_text + se_bin) {
        "NOT RESOLVED: the legs are within their combined spread floor"
    } else if ratio >= MIN_RATIO && rate_bin >= 3e6 {
        "PASS: the binary format still shows the speed that justifies it"
    } else {
        "REGRESSION: the binary format has lost throughput it previously had"
    };
    println!("  verdict: {verdict}");
}
