//! The `Once` register: `fired=1` is a site keeping its declaration, `fired=5` is one that is not.
//!
//! # What this makes possible that a grep could not
//!
//! `Once` is the one policy the emission macros deliberately do not apply — the latch is a named
//! `OnceSite` the site declares. Until this register, checking that a declaration was honoured
//! meant grepping identifier uses and looking for an `OnceSite` in the same file, which cannot
//! tell an emitter from a `use`, a doc link or a test assertion. Measured that way, 39 (identifier,
//! file) pairs across eight crates look suspect, and the number is an upper bound with no way to
//! tighten it.
//!
//! The register answers the question directly, at run time, per site. A site that fires twice
//! declared `Once` and does not latch, and there is no interpretation of `fired=2` under which it
//! did.
//!
//! # Three legs, and the third is what stops the other two being vacuous
//!
//! A register that recorded EVERY site would show leg B's five as readily as leg A's one, and both
//! assertions would pass while the row meant nothing. Leg C emits from an `Every` site and asserts
//! it is **absent**.

use boyko_log::codes::OnceSite;
use boyko_log::lifecycle::{DrainResult, LogConfig, SinkMode, boot, drain, enable};
use boyko_log::once_sites;
use boyko_log::target::{TargetControl, register_dynamic_target, set_target_control};
use boyko_log::{Level, Log, LogTarget, dyn_warn};

mod acme {
    use boyko_log::RatePolicy;

    boyko_log::declare_codes! {
        prefix = "acme",
        (1, W, LATCHED,   RatePolicy::Once,  "a widget budget warning behind a latch"),
        (2, W, UNLATCHED, RatePolicy::Once,  "a widget budget warning with no latch"),
        (3, W, PLAIN,     RatePolicy::Every, "a widget was rebuilt this frame"),
    }
}

fn drained() {
    let DrainResult::Ran(_) = drain() else { panic!("the drain role is free in this process") };
}

/// Rows the register holds for one code number.
fn rows_for(code: u16) -> Vec<once_sites::OnceRow> {
    once_sites::walk().filter(|r| r.code == code).collect()
}

#[test]
fn the_register_tells_a_latched_once_site_from_an_unlatched_one() {
    boot(LogConfig {
        console: false,
        sink_thread: false,
        ecs_ring: false,
        file: false,
        binary: false,
        file_cap_bytes: 0,
        sink_mode: SinkMode::Manual,
    });
    assert!(enable(), "enable() refused a freshly booted process");

    // The engine goes quiet so the register holds this test's sites and nothing else. Every engine
    // `Once` site that fired during boot would otherwise be in the walk, and a test that filtered
    // by code would still be reading a register it did not control.
    set_target_control(<Log as LogTarget>::ID, TargetControl::OFF);
    let widgets = register_dynamic_target("acme.once", TargetControl::new(Level::Trace, 0, false))
        .expect("a 9-byte name in a fresh table");
    drained();

    // ── LEG A: a `Once` site WITH its latch ─────────────────────────────────────────────────
    //
    // Five occurrences, one emission. This is what every one of the registry's 45 `Once` rows is
    // supposed to look like.
    static LATCH: OnceSite = OnceSite::new();
    for i in 0..5u32 {
        if LATCH.claim() {
            dyn_warn!(widgets, acme::LATCHED, "latched {}", i);
        }
    }
    drained();
    let a = rows_for(1);
    assert_eq!(a.len(), 1, "the latched site must appear exactly once in the register: {a:?}");
    assert_eq!(a[0].fired, 1, "a latched `Once` site fires once per process");
    assert!(a[0].file.contains("l8a_once_register"), "the row must name the emitting file");
    assert!(!a[0].counted, "this row declares `Once`, not `OnceCounted`");

    // ── LEG B: a `Once` site with NO latch — THE DETECTOR ───────────────────────────────────
    //
    // Five occurrences, five emissions, one row reading `fired=5`. The registry row promises
    // `Once`; the site delivers five. That is the defect, stated as a number rather than inferred
    // from the absence of a token in a file.
    for i in 0..5u32 {
        dyn_warn!(widgets, acme::UNLATCHED, "unlatched {}", i);
    }
    drained();
    let b = rows_for(2);
    assert_eq!(b.len(), 1, "one SITE, however many times it fired: {b:?}");
    assert_eq!(
        b[0].fired, 5,
        "a `Once` row emitted five times from one site must read fired=5 -- this is the whole \
         point of the register"
    );

    // ── LEG C: an `Every` site is NOT the register's business ───────────────────────────────
    //
    // Without this the two assertions above would pass on a register that recorded every site, and
    // `fired` would mean "records from this site" rather than "times this `Once` site fired".
    for i in 0..5u32 {
        dyn_warn!(widgets, acme::PLAIN, "plain {}", i);
    }
    drained();
    assert!(
        rows_for(3).is_empty(),
        "an `Every` site must not enter the `Once` register; if it does, `fired` is just a \
         per-site record count wearing a policy's name"
    );

    // Nothing was lost on the way in: the register is far larger than three sites, so an overflow
    // here would mean the probe is wrong rather than the table full.
    assert_eq!(once_sites::overflowed(), 0, "three sites cannot overflow a 128-entry register");
    assert!(once_sites::len() >= 2, "both `Once` sites are held: {}", once_sites::len());

    // ── LEG D: THE FIRST REAL DEFECT THE REGISTER FOUND — `W0111` in `census::rows()` ───────
    //
    // `rows()` is a PUBLIC ITERATOR. It used to emit `W0111` -- a row declaring `Once` -- from
    // inside the walk, so a host rendering a census overlay produced one record per unsunk target
    // per frame. Ten walks here; the register must show the engine's `W0111` site absent, because
    // a query may not have a diagnostic as a side effect.
    //
    // The register counts at EMISSION, before the per-sink filters, which is what lets this leg
    // switch every sink off -- the condition it needs -- without losing its own observation.
    //
    // `Log` IS RE-ARMED FIRST, and the first draft of this leg forgot to. `W0111` is emitted on the
    // `Log` target, which the top of this test switched Off so the register would hold only its own
    // sites -- so gate (c) refused the report and BOTH legs below read an empty register. Leg D
    // would have passed for the wrong reason: not "the query no longer emits" but "nothing could
    // emit at all". A control that cannot fire proves nothing about the thing it controls.
    set_target_control(<Log as LogTarget>::ID, TargetControl::new(Level::Trace, 0, false));
    for slot in 0..4 {
        boyko_log::sink::slot::set_state(slot, boyko_log::sink::slot::SinkState::Off);
    }
    let w0111 = boyko_log::codes::W0111.number();
    for _ in 0..10 {
        let _unsunk =
            boyko_log::census::rows().filter(|r| r.status_str().contains("unsunk")).count();
    }
    drained();
    assert!(
        rows_for(w0111).is_empty(),
        "walking `census::rows()` emitted W0111: a query with a diagnostic side effect, on a \
         path a host may walk every frame. Register says: {:?}",
        rows_for(w0111)
    );

    // ── LEG E: `print` reports it, ONCE, however many times it runs ─────────────────────────
    for _ in 0..3 {
        boyko_log::census::print();
    }
    drained();
    let w = rows_for(w0111);
    assert_eq!(w.len(), 1, "the engine's W0111 site must be in the register after `print`: {w:?}");
    assert_eq!(
        w[0].fired, 1,
        "three `print` passes over the same misconfiguration must produce ONE report -- the row \
         declares `Once`, and this is the assertion that makes the declaration mean something"
    );
}
