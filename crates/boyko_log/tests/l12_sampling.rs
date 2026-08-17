//! G10: sampling is EXACT, it is counted on its own column, and it suppresses **delivery only**.
//!
//! # Leg (e) is the one this corpus records as specified wrongly TWICE
//!
//! Once at a rung where `SAMPLE_CTR` did not exist, and once asserting **500** argument evaluations
//! when the true count is 1000 — because the gate chain is a *level* test, so arguments are
//! evaluated to build the tuple long before the sample decision is reached.
//!
//! So (e) asserts **both numbers together**: 1000 evaluations AND 500 deliveries over 1000 emits at
//! shift 1. Asserting only the delivery count would pass a design that moved evaluation behind the
//! sample decision — which would be faster, and would break the documented guarantee that an
//! argument expression runs exactly once per call.

use std::sync::atomic::{AtomicU32, Ordering};

use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::target::{LogTarget, TargetControl, set_target_control, target_stats};
use boyko_log::{Ecs, Level};

static EVALS: AtomicU32 = AtomicU32::new(0);

/// A side-effecting argument: it counts every time it is evaluated.
fn bump() -> u32 {
    EVALS.fetch_add(1, Ordering::Relaxed)
}

#[test]
fn sampling_is_exact_counted_separately_and_never_suppresses_argument_evaluation() {
    let path = std::env::temp_dir().join("boyko_l12_sampling.log");
    let _ = std::fs::remove_file(&path);
    assert!(boyko_log::sink::file::set_path(path.to_str().expect("a UTF-8 temp path")));
    boot(LogConfig {
        console: false,
        sink_thread: false,
        ecs_ring: false,
        file: true,
        file_cap_bytes: 0,
        sink_mode: SinkMode::Manual,
    });
    assert!(enable(), "enable() refused a freshly booted process");

    let id = <Ecs as LogTarget>::ID;
    const N: u32 = 1000;

    // ── (b) THE CONTROL LEG: shift = 0 delivers ALL of them ──────────────────────────────────
    //
    // First, because a test that only ever samples cannot tell "sampling works" from "the target
    // was off" -- the positive control this whole campaign keeps finding missing.
    boyko_log::sample::reset_counters(id.index());
    set_target_control(id, TargetControl::new(Level::Trace, 0, false));
    let (d0, _, s0, _) = target_stats(id);
    // Drained every 64 records. The lane is 16 KiB and 1000 records do not fit -- the first draft
    // emitted all of them and drained once, and the CONTROL LEG caught it: 573 of 1000 delivered,
    // because the lane overflowed and counted the rest as dropped. That is the lane behaving
    // correctly and the test being wrong, and it is exactly what a positive control is for.
    for i in 0..N {
        boyko_log::info!(Ecs, "unsampled {}", 1u32);
        if i % 64 == 63 {
            let _ = drain();
        }
    }
    let _ = drain();
    let (d1, _, s1, _) = target_stats(id);
    assert_eq!(
        d1 - d0,
        u64::from(N),
        "shift 0 must deliver EVERY record -- a shortfall here means the lane overflowed, and          every sampling number below would be measured against a broken baseline"
    );
    assert_eq!(s1 - s0, 0, "shift 0 must sample nothing out");

    // ── (a) EXACTNESS at shift = 1, and (e) evaluation is UNAFFECTED ─────────────────────────
    boyko_log::sample::reset_counters(id.index());
    set_target_control(id, TargetControl::new(Level::Trace, 1, false));
    EVALS.store(0, Ordering::Relaxed);
    let (d2, _, s2, _) = target_stats(id);
    for i in 0..N {
        boyko_log::info!(Ecs, "half {}", bump());
        if i % 64 == 63 {
            let _ = drain();
        }
    }
    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free in this process") };
    let (d3, _, s3, _) = target_stats(id);

    let delivered = d3 - d2;
    let sampled_out = s3 - s2;

    assert_eq!(delivered, u64::from(N >> 1), "shift 1 must deliver EXACTLY n>>1, with no drift");
    assert_eq!(sampled_out, u64::from(N) - u64::from(N >> 1), "and count the rest, never silently");
    assert_eq!(delivered + sampled_out, u64::from(N), "every emit is accounted for on one column");

    // (e). THE NUMBER THAT MAKES THE DISTINCTION LEGIBLE.
    assert_eq!(
        EVALS.load(Ordering::Relaxed),
        N,
        "sampling suppressed ARGUMENT EVALUATION -- it must suppress DELIVERY only. A caller with \
         a side-effecting argument gets that side effect on every call, whatever the shift, and a \
         design that moved evaluation behind the sample decision would be faster and WRONG"
    );

    // ── the sampled records really are on disk, and only half of them ───────────────────────
    let text = std::fs::read_to_string(&path).expect("the sink's file is readable");
    // "half", not "sampled": the control leg's message is "unsampled", and `"sampled "` is a
    // SUBSTRING of it -- the first draft counted 1500 of 500 because it matched both phases. A
    // matcher that silently spans two populations is the measurement equivalent of a gate that
    // cannot fail.
    let on_disk = text.matches("half ").count();
    assert_eq!(on_disk as u64, delivered, "the census and the file must agree on what arrived");

    set_target_control(id, TargetControl::OFF);
}
