//! Rung L0's gate — **G4**, the three-way side-effect probe.
//!
//! The claim under test is not "the right thing was printed" (nothing is printed at this rung) but
//! the property the whole cost model rests on: **argument expressions are evaluated when the gate
//! chain passes and are NOT evaluated when any link of it fails.** A test of the output could
//! never see the difference between "the record was suppressed" and "the arguments ran and then
//! the record was suppressed", and it is the second that costs a shipped title.
//!
//! Three legs, each with a named broken input:
//!
//! | Leg | Setup | Expect | Reds when |
//! |---|---|---|---|
//! | (a) | compile ceiling BELOW the site, runtime armed to `Trace` | **0** | gate (a) is dropped from the chain, or the arguments are hoisted out of the `if` |
//! | (b) | compile ceiling admits, runtime armed to `Trace` | **1000** | the enabled arm stops evaluating its arguments — i.e. the probe cannot pass by suppressing everything |
//! | (c) | compile ceiling admits, runtime `Off` | **0** | gate (c) is dropped, or `.bss`-zero stops meaning `Off` |
//!
//! Leg (b) is what makes the other two mean anything. A one-sided probe that only checks for zero
//! is green on a macro that expands to nothing at all.
//!
//! # The fourth leg is NOT here, and is not silently missing
//!
//! G4's sibling **G2** exercises the *per-profile* compile ceiling (`GLOBAL_CEILING`), gate (b) of
//! the chain. That constant comes from `BOYKO_PROFILE`, read by a build script that lands at the
//! joint rung **J1** — before it exists there is one value of `GLOBAL_CEILING` in every build and
//! no configuration in which the leg could go red. It lands with the axis it tests. What IS
//! covered here is the per-target compile ceiling, which is a `const` this rung owns.
//!
//! # Each leg owns its own control byte
//!
//! `CONTROL` is process-global and `cargo test` runs these concurrently in one process. The three
//! legs therefore address **three different engine ids**, and no other test in this file writes
//! them. Sharing one row between an "armed" leg and an "off" leg is a coin flip, not a gate.

use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_log::{Fontbake, Level, LogTarget, Render, TargetControl, TargetId, Ui};
use boyko_log::{debug, set_target_control};

const ITERATIONS: usize = 1000;

/// Counts one evaluation. Used as a macro argument, so calling it is exactly the side effect the
/// gate chain must prevent.
///
/// `SeqCst` rather than `Relaxed` on purpose: this is an instrument, not a hot path, and an
/// instrument whose own ordering is a question is an instrument that cannot settle one.
fn bump(counter: &AtomicUsize) -> u32 {
    counter.fetch_add(1, Ordering::SeqCst) as u32
}

// ── Leg (a): a target whose COMPILE ceiling is below the site ────────────────────
//
// The engine table's rows all carry `Level::Trace`, so a low-ceiling target has to be declared
// here. It reuses an engine id rather than minting one, which keeps `TargetId`'s constructor set
// closed -- adding a public constructor so a test could build one would trade the invariant that
// makes the hot-path `get_unchecked` sound for a test's convenience.

/// Compile ceiling `Warn`: a `debug!` on this target is deleted by gate (a) in every build.
struct LowCeiling;
impl LogTarget for LowCeiling {
    const NAME: &'static str = "probe-low-ceiling";
    const ID: TargetId = <Render as LogTarget>::ID;
    const STATIC_CEILING: Level = Level::Warn;
}

/// Compile ceiling `Trace`, so gates (a) and (b) both pass and leg (c) can isolate the runtime one.
struct FullCeilingArmed;
impl LogTarget for FullCeilingArmed {
    const NAME: &'static str = "probe-full-ceiling-armed";
    const ID: TargetId = <Ui as LogTarget>::ID;
    const STATIC_CEILING: Level = Level::Trace;
}

/// As above, on its own row, left at the `.bss`-zero default.
struct FullCeilingOff;
impl LogTarget for FullCeilingOff {
    const NAME: &'static str = "probe-full-ceiling-off";
    const ID: TargetId = <Fontbake as LogTarget>::ID;
    const STATIC_CEILING: Level = Level::Trace;
}

static A_HITS: AtomicUsize = AtomicUsize::new(0);
static B_HITS: AtomicUsize = AtomicUsize::new(0);
static C_HITS: AtomicUsize = AtomicUsize::new(0);

#[test]
fn leg_a_compile_ceiling_below_evaluates_nothing_even_with_the_runtime_armed() {
    // Arm the runtime as wide as it goes, so the ONLY thing refusing the site is the per-target
    // compile ceiling. Without this the leg would be indistinguishable from leg (c).
    set_target_control(
        <LowCeiling as LogTarget>::ID,
        TargetControl::new(Level::Trace, 0, false),
    );
    assert_eq!(
        boyko_log::runtime_ceiling(<LowCeiling as LogTarget>::ID),
        Level::Trace as u8,
        "the leg must be armed, or it proves nothing about the compile ceiling"
    );

    for _ in 0..ITERATIONS {
        debug!(LowCeiling, "probe {}", bump(&A_HITS));
    }

    assert_eq!(
        A_HITS.load(Ordering::SeqCst),
        0,
        "a site above its target's compile ceiling must not evaluate its arguments"
    );

    set_target_control(<LowCeiling as LogTarget>::ID, TargetControl::OFF);
}

#[test]
fn leg_b_both_ceilings_admitting_evaluates_every_argument() {
    set_target_control(
        <FullCeilingArmed as LogTarget>::ID,
        TargetControl::new(Level::Trace, 0, false),
    );

    for _ in 0..ITERATIONS {
        debug!(FullCeilingArmed, "probe {}", bump(&B_HITS));
    }

    assert_eq!(
        B_HITS.load(Ordering::SeqCst),
        ITERATIONS,
        "the enabled arm must evaluate its arguments exactly once per call; without this leg the \
         other two are satisfied by a macro that expands to nothing"
    );

    set_target_control(<FullCeilingArmed as LogTarget>::ID, TargetControl::OFF);
}

#[test]
fn leg_c_runtime_off_evaluates_nothing() {
    // Deliberately does NOT write its control byte first: the state under test is the loader's
    // zero, which is the state every process starts in and the one a title that never enables
    // logging stays in.
    assert_eq!(
        boyko_log::target_control(<FullCeilingOff as LogTarget>::ID),
        TargetControl::OFF,
        "this leg's row must still be at the .bss-zero default"
    );

    for _ in 0..ITERATIONS {
        debug!(FullCeilingOff, "probe {}", bump(&C_HITS));
    }

    assert_eq!(
        C_HITS.load(Ordering::SeqCst),
        0,
        "a site whose target is Off at run time must not evaluate its arguments"
    );
}

#[test]
fn every_level_macro_gates_on_its_own_level() {
    // The five macros are five separate expansions; a copy-paste that left `Level::Info` inside
    // `trace!` would pass leg (b) and change nothing observable at this rung.
    static HITS: AtomicUsize = AtomicUsize::new(0);

    struct WarnCeiling;
    impl LogTarget for WarnCeiling {
        const NAME: &'static str = "probe-per-level";
        const ID: TargetId = <boyko_log::Image as LogTarget>::ID;
        const STATIC_CEILING: Level = Level::Warn;
    }

    set_target_control(
        <WarnCeiling as LogTarget>::ID,
        TargetControl::new(Level::Trace, 0, false),
    );

    // A `Warn` compile ceiling admits error! and warn!, and deletes info!/debug!/trace!.
    //
    // Real registry codes rather than the bare `1u16`/`2u16` this line carried until the macros
    // began taking the TYPED newtype. The numbers were arbitrary and unregistered, which is the
    // shape that let a class and a number drift apart in the first place; a gate about the CEILING
    // has no reason to be the last place in the tree emitting a code that resolves to nothing.
    boyko_log::error!(WarnCeiling, boyko_log::codes::E2001, "e {}", bump(&HITS));
    boyko_log::warn!(WarnCeiling, boyko_log::codes::W0103, "w {}", bump(&HITS));
    boyko_log::info!(WarnCeiling, "i {}", bump(&HITS));
    debug!(WarnCeiling, "d {}", bump(&HITS));
    boyko_log::trace!(WarnCeiling, "t {}", bump(&HITS));

    assert_eq!(
        HITS.load(Ordering::SeqCst),
        2,
        "exactly error! and warn! may survive a Warn compile ceiling"
    );

    set_target_control(<WarnCeiling as LogTarget>::ID, TargetControl::OFF);
}
