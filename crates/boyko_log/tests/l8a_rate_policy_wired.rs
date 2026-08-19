//! The declared delivery policy is APPLIED, not merely printed in the registry.
//!
//! Until this test `rate::admit` had zero production callers. `codes.rs` said so in a comment and
//! gated the consequence with `no_live_row_declares_a_policy_the_emission_path_cannot_honour`,
//! which forbade any row from declaring `EveryN` or `MinIntervalMs` — a *declaration with no
//! effect* being the defect class this campaign keeps finding. That gate is deleted with this
//! rung, and this file is what replaces it: the machinery it was standing in for.
//!
//! # Why a DOWNSTREAM table and a DYNAMIC target
//!
//! Two reasons, both about what a green here is allowed to mean.
//!
//! * A downstream table is where the two shared-state policies will actually be declared — the
//!   engine's own 74 rows are all `Every` or `Once`, and inventing a policy for one of them to
//!   give this test a subject would be putting a claim in the registry to satisfy a test. The
//!   downstream path is also the harder one: its `code_idx` is *minted*, not a compile-time row.
//! * A dynamic target lets the `Log` target be switched fully OFF for the run, so the drain's own
//!   `W0117` and anything `boot` says cannot be counted as this test's records. `DrainStats` is
//!   process-wide; a leg that asserted an exact count without silencing the engine would be
//!   asserting about whatever else happened to emit.
//!
//! It also means the `dyn_warn!` expansion is the one under test, which is one of the six macros
//! the gate was wired into rather than the one that was easiest to reach.

use boyko_log::codes::RatePolicy;
use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::target::{TargetControl, register_dynamic_target, set_target_control};
use boyko_log::{Level, Log, LogTarget, dyn_warn};

mod acme {
    use boyko_log::RatePolicy;

    boyko_log::declare_codes! {
        prefix = "acme",
        (1, W, ACME_W0001, RatePolicy::EveryN(4),          "one widget rebuild in four"),
        (2, W, ACME_W0002, RatePolicy::MinIntervalMs(60_000), "the widget pool is thrashing"),
        (3, W, ACME_W0003, RatePolicy::Every,              "a widget was rebuilt this frame"),
        (4, W, ACME_W0004, RatePolicy::Once,               "the widget budget is nearly spent"),
    }
}

/// Drain, returning how many records the pass moved.
fn drained() -> u64 {
    match drain() {
        DrainResult::Ran(stats) => stats.records,
        DrainResult::Busy => panic!("the drain role is free in this process"),
    }
}

/// One `#[test]`, four legs, in sequence.
///
/// Not four `#[test]`s: `RATE` and `SUPPRESSED` are process-global and the drain role is single.
/// Four functions would race for both, and the flake would look like a rate-limiter bug — which is
/// the one failure this file must never produce falsely.
#[test]
fn the_registry_column_is_applied_by_the_emission_macros() {
    boot(LogConfig {
        console: false,
        sink_thread: false,
        ecs_ring: false,
        file: false,
        file_cap_bytes: 0,
        sink_mode: SinkMode::Manual,
    });
    assert!(enable(), "enable() refused a freshly booted process");

    // The engine goes quiet, so every record this test drains is one this test emitted.
    set_target_control(<Log as LogTarget>::ID, TargetControl::OFF);
    let widgets = register_dynamic_target("acme.widgets", TargetControl::new(Level::Trace, 0, false))
        .expect("a 12-byte name in a fresh table");

    // Whatever boot and enable said goes out before the first snapshot.
    let _ = drained();

    // ── LEG 1: `Every` — the POSITIVE CONTROL, and it comes first on purpose ─────────────────
    //
    // A rate gate that suppressed everything would make legs 2 and 3 pass for the wrong reason.
    // This leg is what makes their green mean "the declared policy", not "the gate is closed".
    let sup0 = boyko_log::rate::suppressed();
    for i in 0..8u32 {
        dyn_warn!(widgets, acme::ACME_W0003, "every {}", i);
    }
    assert_eq!(drained(), 8, "an `Every` row must deliver every occurrence");
    assert_eq!(
        boyko_log::rate::suppressed() - sup0,
        0,
        "an `Every` row must not touch the limiter at all"
    );

    // ── LEG 2: `EveryN(4)` — one in four, and the other three COUNTED ────────────────────────
    //
    // The first occurrence is admitted (the RMW returns the previous count, so occurrence 0 sees
    // 0), then every fourth. Sixteen in, four out, twelve counted.
    let sup1 = boyko_log::rate::suppressed();
    for i in 0..16u32 {
        dyn_warn!(widgets, acme::ACME_W0001, "every-n {}", i);
    }
    assert_eq!(drained(), 4, "16 occurrences at EveryN(4) must deliver 4");
    assert_eq!(
        boyko_log::rate::suppressed() - sup1,
        12,
        "the twelve refused occurrences must be counted, not forgotten"
    );

    // ── LEG 3: `MinIntervalMs` — one window, and the window outlives the test ────────────────
    //
    // 60 s rather than a few ms, and NO sleep: a test that slept to cross a window would be
    // asserting about the scheduler. The claim here is "the second occurrence inside one window
    // is refused", and a window nothing can cross makes it deterministic on any box.
    let sup2 = boyko_log::rate::suppressed();
    for i in 0..8u32 {
        dyn_warn!(widgets, acme::ACME_W0002, "min-interval {}", i);
    }
    assert_eq!(drained(), 1, "8 occurrences inside one MinIntervalMs window must deliver 1");
    assert_eq!(boyko_log::rate::suppressed() - sup2, 7, "the other seven must be counted");

    // ── LEG 4: `Once` is STILL the site's own latch, and that is a decision ──────────────────
    //
    // The macro deliberately does not place an `OnceSite`. A `static` inside a macro expansion
    // cannot be named, and `OnceSite::reset` exists exactly so an observer can reset the latch it
    // is about to test — auto-latching would buy redundancy at the price of making every `Once`
    // site untestable in isolation.
    //
    // So eight occurrences of a `Once` row with no latch at the site deliver eight. This is
    // pinned rather than left implicit: if a later rung decides to auto-latch, this assertion is
    // where that decision has to be argued, and the alternative is a silent behaviour change at
    // all 45 `Once` rows.
    let sup3 = boyko_log::rate::suppressed();
    for i in 0..8u32 {
        dyn_warn!(widgets, acme::ACME_W0004, "once {}", i);
    }
    assert_eq!(
        drained(),
        8,
        "the macro must NOT latch `Once`; the site's own named `OnceSite` does that"
    );
    assert_eq!(
        boyko_log::rate::suppressed() - sup3,
        0,
        "`Once` must not reach the shared limiter -- that would make it per CODE, not per site"
    );

    // ── the policy really is the one the table declared ─────────────────────────────────────
    //
    // Cheap, and it is the link the four legs above cannot see: they prove the macro applies THE
    // POLICY IT READ, not that it read the right one.
    assert_eq!(acme::ACME_W0001.policy(), RatePolicy::EveryN(4));
    assert_eq!(acme::ACME_W0002.policy(), RatePolicy::MinIntervalMs(60_000));
    assert_eq!(acme::ACME_W0003.policy(), RatePolicy::Every);
    assert_eq!(acme::ACME_W0004.policy(), RatePolicy::Once);
    assert_eq!(
        acme::ACME_W0001.policy(),
        acme::DIAGNOSTICS[0].rate,
        "the value the site folds and the value the registry prints must be one token"
    );
}
