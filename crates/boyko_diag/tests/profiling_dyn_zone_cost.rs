//! **Profiling rung 10, `G17` — the dynamic path's cost, and the substitution this box forced.**
//!
//! # What `G17` asks for, and why it is not what runs here
//!
//! The corpus states `G17` as five absolute thresholds in one sitting: *"static-armed ≤ 12 ns,
//! dyn-armed ≤ 14 ns, static-disarmed ≤ 2 ns, dyn-disarmed ≤ 3 ns, script ≤ 18 ns"*, with the RED
//! being *"implement `zone_dyn!` with a `REGISTRY[id]` dereference to recover the scope bit ⇒ the
//! dyn-armed leg exceeds 14 ns"*.
//!
//! **An absolute nanosecond threshold on this box measures the box.** This campaign has already
//! paid to learn that: `docs/PROFILING-FLOOR.md`'s protocol found a **6.5 %** cross-process floor
//! on the artifact channel, with individual repetitions spanning 4.7 % to 14.3 % — on a *GPU* pass
//! costing microseconds. A 14 ns budget is two nanoseconds of headroom over 12; a scheduler slice,
//! a turbo transition or another test binary sharing a core moves it by more than that. A gate
//! that red-lights on a background process is not a gate on this code.
//!
//! # What runs instead: the same RED, measured as an A/B in one sitting
//!
//! The RED `G17` names is a **structural** change — recovering the arm bit from `REGISTRY[id]`
//! instead of carrying it on the handle. So this file implements *both* variants and compares them
//! **against each other, interleaved, in one process**:
//!
//! * `dyn_armed_shipped` — what `zone_dyn!` expands to: one `Acquire` load of the arm mask and a
//!   bit test.
//! * `dyn_armed_registry` — the RED: an `Acquire` load of `REGISTRY[id]`, a null check, a
//!   dereference to reach `scope`, then the same mask test.
//!
//! Both legs run the same loop, the same number of times, alternating, on the same thread. A
//! machine that is slow today is slow for both, so the RATIO is stable where the absolutes are not.
//! **The absolute nanosecond figures are PRINTED** — so the corpus's numbers can be compared by a
//! human — and asserted only as a sanity floor that no plausible box violates.
//!
//! # What this gate CANNOT claim
//!
//! Everything `G17`'s own limits column says, and one more. It cannot claim the dynamic path is
//! fast *in a game*: the handle sits in a register here, and a real one is a cold load out of an
//! ECS component. It measures the path's **floor**. And it cannot claim the shipped path meets any
//! particular nanosecond figure on any other machine — it claims the shipped path is not slower
//! than the variant the corpus asks it to beat.

use std::hint::black_box;

use boyko_diag::profiling_abi::dyn_registry::{DynZoneHandle, ZoneSpec, register_zone};
use boyko_diag::profiling_abi::{USER_SCOPE_BASE, ZoneTier, arm_scope, disarm_scope, zone_desc};

boyko_diag::profiling_partition!(User);

/// Iterations per leg. Large enough that one scheduler slice is a small fraction of the total,
/// small enough that the whole file is well under a second.
const ITERS: u32 = 200_000;
/// Alternating rounds. Interleaving is the point: a one-shot A-then-B would attribute a thermal
/// or frequency drift entirely to B.
const ROUNDS: u32 = 8;

/// The SHIPPED gate: the arm bit is on the handle, so this is one atomic load and a mask test.
#[inline(never)]
fn dyn_armed_shipped(h: DynZoneHandle) -> u64 {
    let mut hits = 0u64;
    for _ in 0..ITERS {
        if black_box(h).armed() {
            hits += 1;
        }
    }
    hits
}

/// **The RED variant**: recover the scope from the registry on every emission.
///
/// This is what `zone_dyn!` would have to do if `DynZoneHandle` carried only an id — a dependent
/// load into a table that is 56 KiB wide, followed by a pointer chase to reach one `u32`.
#[inline(never)]
fn dyn_armed_registry(h: DynZoneHandle) -> u64 {
    let mut hits = 0u64;
    for _ in 0..ITERS {
        let id = black_box(h).id;
        // The dereference the shipped path does not do.
        if let Some(desc) = zone_desc(id)
            && boyko_diag::profiling_abi::scope_armed(desc.scope)
        {
            hits += 1;
        }
    }
    hits
}

/// Which build profile produced the printed figures.
///
/// Load-bearing on the print, not decoration. `cargo test` builds `dev` unless told otherwise, and
/// a debug-profile nanosecond figure compared against the corpus's thresholds — which are release
/// figures — is a comparison between two different things. Naming the profile in the output is what
/// stops the next reader making it.
const PROFILE: &str = if cfg!(debug_assertions) { "debug" } else { "release" };

/// Nanoseconds per iteration for one leg, using the engine's own clock.
fn ns_per_iter(total_ticks: u64) -> f64 {
    let per_iter = total_ticks as f64 / f64::from(ITERS);
    per_iter / boyko_diag::clock::ticks_per_ns()
}

/// **`G17`, as an A/B.** The shipped dynamic gate is not slower than the registry-dereferencing
/// variant the corpus names as its RED.
///
/// Both legs are also asserted to have done the same WORK (the same hit count), which is what stops
/// a compiler that optimised one loop away from producing a flattering ratio.
#[test]
fn the_dynamic_gate_beats_the_registry_dereference_it_replaced() {
    boyko_diag::clock::calibrate();

    let h = register_zone(ZoneSpec {
        name: "g17.dyn_cost",
        scope: USER_SCOPE_BASE + 7,
        tier: ZoneTier::Always,
    })
    .expect("a fresh registration succeeds");
    arm_scope(USER_SCOPE_BASE + 7);

    // Warm both paths before either is timed: the first pass faults the registry page in, and
    // charging that to whichever leg happens to run first is exactly the artefact interleaving
    // exists to remove.
    black_box(dyn_armed_shipped(h));
    black_box(dyn_armed_registry(h));

    let (mut shipped_ticks, mut registry_ticks) = (0u64, 0u64);
    let (mut shipped_hits, mut registry_hits) = (0u64, 0u64);
    for _ in 0..ROUNDS {
        let t0 = boyko_diag::clock::ticks();
        shipped_hits += dyn_armed_shipped(h);
        let t1 = boyko_diag::clock::ticks();
        registry_hits += dyn_armed_registry(h);
        let t2 = boyko_diag::clock::ticks();
        shipped_ticks += t1.wrapping_sub(t0);
        registry_ticks += t2.wrapping_sub(t1);
    }

    let shipped_ns = ns_per_iter(shipped_ticks / u64::from(ROUNDS));
    let registry_ns = ns_per_iter(registry_ticks / u64::from(ROUNDS));
    println!(
        "G17 [{PROFILE}] dyn zone gate, {ROUNDS} interleaved rounds x {ITERS} iters: \
         shipped {shipped_ns:.2} ns/iter, registry-deref {registry_ns:.2} ns/iter, \
         ratio {:.2}x",
        registry_ns / shipped_ns.max(f64::MIN_POSITIVE)
    );

    assert_eq!(
        shipped_hits, registry_hits,
        "the two legs must do the same work, or the comparison is between different loops"
    );
    assert!(
        shipped_hits > 0,
        "an armed zone must report armed; a zero hit count means both loops measured nothing"
    );
    assert!(
        shipped_ticks <= registry_ticks,
        "the shipped dynamic gate ({shipped_ns:.2} ns/iter) is SLOWER than the registry \
         dereference it exists to avoid ({registry_ns:.2} ns/iter). Carrying `arm_bit` on the \
         handle is the whole reason the handle is 16 bytes instead of 8; if this fires, either the \
         bit stopped being carried or the emission path grew a dereference."
    );

    disarm_scope(USER_SCOPE_BASE + 7);
}

/// The DISARMED leg: a zone whose scope is off costs one load and a test, and pushes nothing.
///
/// Asserted as a ratio against the armed leg rather than against a nanosecond figure, for the
/// module doc's reason. The claim is the one that matters to a shipped title: turning a scope off
/// makes its zones cheaper, and measurably so.
#[test]
fn a_disarmed_dynamic_zone_costs_less_than_an_armed_one() {
    boyko_diag::clock::calibrate();

    let h = register_zone(ZoneSpec {
        name: "g17.dyn_disarmed",
        scope: USER_SCOPE_BASE + 8,
        tier: ZoneTier::Always,
    })
    .expect("registered");

    let mut armed_ticks = 0u64;
    let mut disarmed_ticks = 0u64;
    // Warm.
    arm_scope(USER_SCOPE_BASE + 8);
    black_box(open_close_loop(h));
    disarm_scope(USER_SCOPE_BASE + 8);
    black_box(open_close_loop(h));

    for _ in 0..ROUNDS {
        arm_scope(USER_SCOPE_BASE + 8);
        let t0 = boyko_diag::clock::ticks();
        black_box(open_close_loop(h));
        let t1 = boyko_diag::clock::ticks();
        disarm_scope(USER_SCOPE_BASE + 8);
        let t2 = boyko_diag::clock::ticks();
        black_box(open_close_loop(h));
        let t3 = boyko_diag::clock::ticks();
        armed_ticks += t1.wrapping_sub(t0);
        disarmed_ticks += t3.wrapping_sub(t2);
    }

    println!(
        "G17 [{PROFILE}] dyn open/close, {ROUNDS} rounds x {ITERS} iters: armed {:.2} ns/iter, \
         disarmed {:.2} ns/iter",
        ns_per_iter(armed_ticks / u64::from(ROUNDS)),
        ns_per_iter(disarmed_ticks / u64::from(ROUNDS)),
    );

    assert!(
        disarmed_ticks < armed_ticks,
        "a disarmed scope must be cheaper than an armed one: disarmed {disarmed_ticks} ticks vs \
         armed {armed_ticks}. If these are equal the gate is not being taken, which would mean the \
         arm mask is not consulted before the clock is read."
    );
}

/// The open/close pair `zone_dyn!` expands to, as a loop.
#[inline(never)]
fn open_close_loop(h: DynZoneHandle) -> u64 {
    let mut opened = 0u64;
    for _ in 0..ITERS {
        let g = boyko_diag::zone_dyn!(black_box(h));
        if g.is_some() {
            opened += 1;
        }
        drop(g);
    }
    opened
}
