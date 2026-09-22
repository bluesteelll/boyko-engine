//! **BOUNDARY B0 — the REORDER half of the id-difference harness.**
//!
//! A `OnceLock` prelude runs **before any fixture `component_id()` touch**: it probes the
//! shared id budget, mints [`K`] spacer ids, touches `Decoy`, and only then calls
//! `fixtures::touch_all`. Every test in this file calls the prelude first, so thread order
//! is irrelevant — and separate `tests/*.rs` files are separate **processes**, so this
//! binary and `boundary_roundtrip.rs` cannot contaminate each other's id space.
//!
//! What the pair proves is that the boundary's stream is **name-keyed**: B5 dumps the same
//! fixture entity in this shifted process and asserts the bytes are identical to the blob
//! captured in the other one. Any `ComponentId`, field index or byte offset that leaked
//! into the stream moves those bytes. That claim is worth nothing unless the ids provably
//! differ, which is this file's assertion.
//!
//! # The invocation is part of the gate (CORE D23)
//!
//! ```text
//! cargo test -p reflect-fixture --features reflect-fixture/reflect \
//!     --test boundary_roundtrip --test boundary_id_reorder -- --test-threads=1
//! ```
//!
//! The output must read `running [1-9]` for **both** binaries. This file is
//! `#![cfg(feature = "reflect")]`, so a plain `cargo test -p reflect-fixture` compiles it
//! to nothing and exits 0 — on the green side **and on every red side**.
//!
//! # RED MUTATION — delete the `prelude()` call from the test body
//!
//! ~~`K = 0`~~ was B0's headline RED and **it does not fire**. MEASURED 2026-08-27,
//! `rustc 1.97.1`: the prelude carries two `K`-INDEPENDENT id consumers ahead of the
//! fixtures — the `__reflect_b0_probe` tag the budget clause mints, and the `Decoy` touch
//! — so the shift is `2 + K` and is non-zero at *every* `K`. The budget clause is a LATER
//! repair of this rung, and it is what introduced the probe: **the repair disarmed the red
//! it was written beside** (D23). Nor is a cleverer assertion the fix — a parameter cannot
//! be the subject of a gate written in terms of that parameter, and every candidate of the
//! form `assert_eq!(id, CAPTURED + 2 + K)` moves with `K` and is equally blind.
//!
//! The observable that *does* depend on what this harness exists to prove is **whether the
//! prelude ran before the first touch**. So: delete the `prelude()` call from the test
//! below (or move it after the first `component_id()` touch). This binary then mints in
//! the capture binary's order, `Pod3` lands on `CAPTURED_POD3_ID`, and the `assert_ne!`
//! reds.
//!
//! That mutation doubles as the **only mechanical enforcement** of this file's "every test
//! calls the prelude first" rule, which is otherwise a discipline claim with no gate: a
//! future test that touches a fixture without calling the prelude mints that fixture ahead
//! of the spacers and silently weakens the reorder — the dump still round-trips, and only
//! the shifted-id premise is gone.
//!
//! # SECOND RED — `K = MAX_COMPONENTS`
//!
//! The **budget assertion** must fire, with its own message, and
//! `register_tag_exhausted_panic` must **not** be reached. (The BOUNDARY plan named
//! `register_enable_tag_exhausted_panic` at all three of this rung's sites, one of them
//! this red's acceptance criterion — a different function, reachable only from
//! `register_enable_tag`, which this harness never calls. An observer running the red
//! against that name would truthfully report "we did not see it" while learning nothing.
//! D25.)
#![cfg(feature = "reflect")]

use std::sync::OnceLock;

use boyko_ecs::ecs::core::component::component::Component as ComponentTrait;
use boyko_ecs::ecs::core::component::component_registry::MAX_COMPONENTS;
use boyko_ecs::prelude::EcsMaster;

use crate::fixtures::ids::{CAPTURED_FIXTURE_IDS, CAPTURED_POD3_ID};
use crate::fixtures::{FIXTURE_NAMES, FIXTURE_TYPE_COUNT, Pod3, ids_by_type, touch_all};

mod fixtures;

/// How many spacer ids the prelude burns before the fixtures.
///
/// **Chosen as the smallest value that satisfies B5, not the largest that fits.** The
/// capture binary occupies ids `0..FIXTURE_TYPE_COUNT`; here the probe takes id 0, the
/// spacers take `1..=K`, `Decoy` takes `K + 1` and the remaining fixtures follow. B5 gate 3
/// needs `Decoy`'s reorder id to collide with nothing the capture used, i.e.
/// `K + 1 >= FIXTURE_TYPE_COUNT`, so `K = FIXTURE_TYPE_COUNT - 1 = 7` — which additionally
/// makes the two processes' whole fixture id sets disjoint, the property
/// [`the_prelude_moves_every_fixture_id_off_the_captured_ones`] asserts.
///
/// `K` is a **tuning knob** for how large the shift is. It is not, and cannot be, the
/// subject of this file's gate — see the RED MUTATION note in the module header.
const K: usize = 7;

/// One-shot guard: the prelude must run exactly once, and before anything in this process
/// touches a fixture.
static PRELUDE: OnceLock<()> = OnceLock::new();

/// Probes the shared id budget, burns [`K`] spacer ids, touches `Decoy`, then mints the
/// fixtures in `fixtures::touch_all`'s canonical order.
///
/// # The budget clause, and why it is not decoration
///
/// Dynamic tags draw from the **same** 512-id `NEXT_ID` counter as every typed component,
/// and `try_register_tag_by_name` returns `None` once `NEXT_ID >= MAX_COMPONENTS`, which
/// `EcsMaster::register_tag` turns into `register_tag_exhausted_panic`
/// (`crates/boyko_ecs/src/ecs/core/ecs_master/tag_api.rs:68`, defined at `:245`). A `K`
/// picked by eye works until a future test-binary link order pushes this binary's
/// registrations past 512 — and then it panics in a kernel function whose name mentions
/// neither reflection nor this test, and nobody connects the two.
///
/// There is no public accessor for the high-water mark (`next_id_for_test` is
/// `pub(crate)`), so the prelude **probes**: the first tag it mints reports the counter's
/// current value, and the assertion spends the remainder. The message is the deliverable.
fn prelude() {
    PRELUDE.get_or_init(|| {
        let mut ecs = EcsMaster::new();

        // The probe IS the read: the id this tag lands on is the current `NEXT_ID`.
        let probe = ecs.register_tag("__reflect_b0_probe").component_id().0;
        let budget = MAX_COMPONENTS - probe - 1;

        assert!(
            K + FIXTURE_TYPE_COUNT <= budget,
            "B0's spacer count K={K} plus {FIXTURE_TYPE_COUNT} fixture types exceeds the \
             {budget} ids left in this binary's shared {MAX_COMPONENTS}-id budget (the \
             probe landed at {probe}). Lower K, or split this test binary -- do NOT let it \
             reach register_tag_exhausted_panic, whose message names neither reflection nor \
             this harness."
        );

        // Cold, once per process, and the only place a name is built at runtime: `K` must
        // stay a runtime-checkable quantity, so a `[&str; K]` table -- which would turn the
        // second RED into a COMPILE error and silence the budget assertion it is meant to
        // fire -- is deliberately not used here.
        for i in 0..K {
            ecs.register_tag(&format!("__reflect_b0_spacer_{i}"));
        }

        // `Decoy` ahead of the rest, so its reorder id is the one B5 gate 3 reads.
        <fixtures::Decoy as ComponentTrait>::component_id();

        touch_all();
    });
}

/// **B0 gate 2** — the ids provably moved.
#[test]
fn the_prelude_moves_every_fixture_id_off_the_captured_ones() {
    prelude();

    // First executable statement after the prelude, and spelled exactly as B5 preconditions
    // on it.
    assert_ne!(
        <Pod3 as ComponentTrait>::component_id().0,
        CAPTURED_POD3_ID,
        "`Pod3` minted at the captured id in the REORDER binary, so this process's ids did \
         not move and every downstream claim that the stream is name-keyed is being proved \
         over two identical id spaces. The cause is almost always that something touched a \
         fixture before `prelude()` ran."
    );

    let observed = ids_by_type();
    println!("B0 reorder (K={K}): {FIXTURE_NAMES:?} = {observed:?}");

    for (slot, id) in observed.iter().enumerate() {
        assert!(
            !CAPTURED_FIXTURE_IDS.contains(id),
            "fixture `{}` minted at id {id} here, which is an id the CAPTURE process also \
             used ({CAPTURED_FIXTURE_IDS:?}). B5 gate 3 applies a blob captured over those \
             ids in this process and asserts `Decoy` is unchanged -- an overlap makes that \
             assertion pass for the wrong reason. Raise K.",
            FIXTURE_NAMES[slot]
        );
    }
}
