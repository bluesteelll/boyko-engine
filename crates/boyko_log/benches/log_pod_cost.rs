//! `log_pod_12b` — **≤ 20 ns, and `dsp!` of the same value must be ≥ 5× slower**.
//!
//! # Why this row, like `log_disabled_warn`, is not capped at 256 calls
//!
//! `LogPod` is not a record argument: it reaches a sink through `encode_pod` / `fmt_pod`, not
//! through the emission macros. So nothing is published, no lane fills, and no drain is needed —
//! the block is 200 000 calls and the resolution is 0.0005 ns rather than 0.391. The 256-call cap
//! is a lane limit, and it applies to legs that use the lane.
//!
//! # What the legs are, and the mistake the first draft made
//!
//! The first version timed `encode_pod` **and** `fmt_pod` on the POD leg against `dsp!` alone on
//! the control, and reported 118 ns vs 80 ns — the POD path looking 1.5× SLOWER than the thing it
//! exists to replace. That was a comparison of a round trip against a one-way trip.
//!
//! **`fmt_pod` does not run on the emitting thread.** The emitter pays `encode_pod`; the render
//! happens later, on the sink, off the hot path. `dsp!` pays its whole render AT the call site.
//! So the row's comparison is `encode_pod` against `dsp!`, and putting the decode on the POD side
//! charged the emitter for work it never does.
//!
//! Three legs are timed and all three are printed, so nothing is hidden by the correction:
//!
//! * **`encode_pod`** — what the EMITTER pays. This is the row's subject.
//! * **`dsp!`** — what the emitter pays instead, without the POD path. This is the row's control.
//! * **`fmt_pod`** — what the SINK pays later. Reported because a format that merely moves cost
//!   has not removed it, and a reader is entitled to see where it went.
//!
//! # The `5×` is measured before it is judged
//!
//! It is an estimate written before anything was measured, exactly like L13b's `5×` and
//! `log_enabled_sampled_out`'s `6 ns`. This bench REPORTS the ratio and says whether the estimate
//! held; it does not quietly re-cut itself to whatever passes. Re-cutting is an owner's call, and
//! the reading is what that call is made on.

use std::fmt;
use std::hint::black_box;
use std::time::Instant;

#[path = "instrument.rs"]
mod instrument;
use instrument::{at_floor, med_and_floor, resolution_ns};

use boyko_log::record::{DspBuf, LogPod};
use boyko_log::site::LogFormatter;

/// Twelve bytes of fields, which is the row's name: three `u32`s, no padding to hide behind.
#[derive(Clone, Copy)]
struct Hit12 {
    dmg: u32,
    target: u32,
    frame: u32,
}

boyko_log::impl_log_pod!(Hit12 { dmg: u32, target: u32, frame: u32 });

/// The `Display` the POD path exists instead of. Deliberately the SAME three fields in the same
/// order: a comparison against a cheaper message would be a comparison against a different message.
impl fmt::Display for Hit12 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "dmg={} target={} frame={}", self.dmg, self.target, self.frame)
    }
}

/// Calls per timed block. Bounded by patience, not by the lane: nothing is published.
const CALLS: u32 = 200_000;

/// Rounds in the sitting, interleaved A-B-A'.
const ROUNDS: usize = 21;

/// The row's absolute bound.
const BOUND_NS: f64 = 20.0;

/// The row's ratio estimate. Reported against, never silently re-cut.
const ESTIMATED_RATIO: f64 = 5.0;

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
    let v = Hit12 { dmg: 4242, target: 77, frame: 1_234_567 };
    let mut bytes = [0u8; 64];
    let mut out = DspBuf::<128>::new();

    let mut pod = Vec::with_capacity(ROUNDS);
    let mut dsp = Vec::with_capacity(ROUNDS);
    let mut pod2 = Vec::with_capacity(ROUNDS);
    let mut render = Vec::with_capacity(ROUNDS);

    for _ in 0..ROUNDS {
        pod.push(time(|| {
            // SAFETY: `bytes` is 64 long and `POD_LEN` is 12; `encode_pod` writes exactly
            //   `POD_LEN` initialised bytes and never reads the struct's padding.
            unsafe { black_box(&v).encode_pod(bytes.as_mut_ptr()) };
            black_box(&bytes);
        }));
        dsp.push(time(|| {
            out.clear();
            let mut f = LogFormatter::new(&mut out);
            f.write_str(boyko_log::dsp!(black_box(v), 128));
            black_box(&out);
        }));
        pod2.push(time(|| {
            // SAFETY: as above.
            unsafe { black_box(&v).encode_pod(bytes.as_mut_ptr()) };
            black_box(&bytes);
        }));
        render.push(time(|| {
            out.clear();
            let mut f = LogFormatter::new(&mut out);
            <Hit12 as LogPod>::fmt_pod(black_box(&bytes[..<Hit12 as LogPod>::POD_LEN]), &mut f);
            black_box(&out);
        }));
    }

    let resolution = resolution_ns(CALLS);
    let (med_pod, se_pod) = med_and_floor(&mut pod, resolution);
    let (med_dsp, se_dsp) = med_and_floor(&mut dsp, resolution);
    let (med_pod2, _) = med_and_floor(&mut pod2, resolution);
    let (med_render, _) = med_and_floor(&mut render, resolution);

    let twin_gap = (med_pod - med_pod2).abs();
    let ratio = med_dsp / med_pod;

    println!("instrument: resolution {resolution:.5} ns/call over {CALLS}-call blocks");
    println!("log_pod_12b (encode_pod, EMITTER pays) : {med_pod:8.3} ns  (se {se_pod:.4})  bound {BOUND_NS}");
    println!("control  dsp! same value (emitter pays) : {med_dsp:8.3} ns  (se {se_dsp:.4})");
    println!("        fmt_pod (SINK pays, later)      : {med_render:8.3} ns");
    println!("  POD_LEN = {} B   A-vs-A' twin gap = {twin_gap:.4} ns", <Hit12 as LogPod>::POD_LEN);
    println!("  ratio dsp!/pod = {ratio:.2}x   (estimate said >= {ESTIMATED_RATIO}x)");

    if at_floor(med_pod, resolution) {
        println!("  verdict: NOT MEASURABLE (instrument): the POD leg is at the clock's floor");
        return;
    }
    if twin_gap > med_pod * 0.05 {
        println!("  verdict: NOT MEASURABLE (instrument): the A-vs-A' twin drifted over 5% of the leg");
        return;
    }
    if (med_dsp - med_pod).abs() < se_pod + se_dsp {
        println!("  verdict: NOT RESOLVED: the two legs are within their combined floor");
        return;
    }
    // THE POD PATH DOES NOT REDUCE TOTAL WORK, IT MOVES IT. Stated in the output, not left for a
    // reader to derive: `encode_pod + fmt_pod` costs MORE end to end than `dsp!` does, and the
    // whole purchase is that the emitting thread pays the first number instead of the second.
    // "75x faster" without this line is the kind of claim this campaign exists to prevent.
    let total_pod = med_pod + med_render;
    println!(
        "  total work: pod {total_pod:.1} ns (emitter {med_pod:.2} + sink {med_render:.1}) vs dsp! {med_dsp:.1} ns -- the path MOVES cost off the emitter, it does not remove it"
    );

    let abs = if med_pod <= BOUND_NS { "PASS" } else { "OVER BOUND" };
    // The two clauses are reported SEPARATELY. An absolute that passes and a ratio that misses is
    // one fact each, and a single verdict would hide whichever of them was inconvenient.
    let rat = if ratio >= ESTIMATED_RATIO { "MET" } else { "MISSED -- estimate, owner's call" };
    println!("  verdict: absolute {abs} ({med_pod:.3} ns vs {BOUND_NS}); ratio estimate {rat} ({ratio:.2}x)");
}
