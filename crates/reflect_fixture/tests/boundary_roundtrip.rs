//! **BOUNDARY B0 — the CAPTURE half of the id-difference harness.**
//!
//! This binary touches the fixtures directly, in `fixtures::touch_all`'s canonical order,
//! and asserts the ids it observes are the ones `fixtures/ids.rs` commits. It is the
//! endpoint that turns `CAPTURED_POD3_ID` from a hand-typed number into a measurement.
//!
//! Its dump → apply body is **B5 gate 1**, and every mechanism that body calls arrives at
//! B1–B3. As B0 first wrote it, this file therefore carried *no assertion at all* — a
//! `running 0 tests` binary, in the rung whose stated purpose is to prevent exactly that
//! (`docs/REFLECTION-PLAN-BOUNDARY.md` D22(b)).
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
//! # RED MUTATION (the capture endpoint)
//!
//! Move `Decoy` ahead of `Pod3` in `fixtures::touch_all`. The `assert_eq!`s below red, and
//! `boundary_id_reorder.rs` stays **green** — its prelude has already minted `Decoy`
//! before it calls `touch_all`. One mutation, one red and one green, is the proof that the
//! two files' assertions read two different endpoints rather than one tautology.
//!
//! ⚠️ **The vector assertion below only fires because minting order and reporting order
//! are two different arrays.** With one array doing both — which is how this rung was
//! first written — the returned vector is `[0, 1, …, 7]` for every permutation of itself,
//! and this file's headline assertion passed the mutation. MEASURED 2026-08-27; see
//! `fixtures::ids_by_type`.
#![cfg(feature = "reflect")]

mod fixtures;

use boyko_ecs::ecs::core::component::component::Component as ComponentTrait;

use crate::fixtures::ids::{CAPTURED_FIXTURE_IDS, CAPTURED_POD3_ID};
use crate::fixtures::{FIXTURE_NAMES, Pod3, ids_by_type};

/// **B0 gate 1** — the capture endpoint.
///
/// Asserts the whole capture-order id vector, then `Pod3`'s id under the name both this
/// rung and B5 spell it by.
#[test]
fn the_capture_ordering_mints_the_ids_the_committed_capture_recorded() {
    // `ids_by_type` calls `touch_all` itself, so the minting order this binary is being
    // checked against cannot be dropped from here by accident.
    let observed = ids_by_type();

    // The capture procedure in `fixtures/ids.rs` reads this line under `--nocapture`.
    println!("B0 capture: {FIXTURE_NAMES:?} = {observed:?}");

    assert_eq!(
        observed, CAPTURED_FIXTURE_IDS,
        "the capture ordering moved: {FIXTURE_NAMES:?} now mints {observed:?}, and \
         `fixtures/ids.rs` commits {CAPTURED_FIXTURE_IDS:?}.\n\
         This is the signal, not the failure -- B5's golden blob is captured over these \
         ids, and `boundary_id_reorder.rs`'s `assert_ne!` is written against them. Find \
         out WHY the order moved and whether B5's committed bytes are still valid before \
         re-deriving the constants; editing them to match a new observation without that \
         step converts this gate into a rubber stamp."
    );

    assert_eq!(
        <Pod3 as ComponentTrait>::component_id().0,
        CAPTURED_POD3_ID,
        "`Pod3` no longer mints at CAPTURED_POD3_ID. The vector assertion above should \
         have caught this first -- if it did not, `CAPTURED_POD3_ID` and \
         `CAPTURED_FIXTURE_IDS` disagree with each other, which means one of them was \
         edited without re-running the capture."
    );
}
