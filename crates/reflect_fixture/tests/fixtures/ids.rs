//! **BOUNDARY B0 — the committed capture of the fixture ids.**
//!
//! One process cannot hand a `ComponentId` to another, and the two halves of the
//! id-difference harness are two `tests/*.rs` files and therefore two processes. So the
//! capture ordering's ids are *committed*, here, and the two binaries read them from
//! opposite sides:
//!
//! * `boundary_roundtrip.rs` asserts the live ids **equal** these — that is what makes
//!   these constants a MEASUREMENT rather than a magic number;
//! * `boundary_id_reorder.rs` asserts the live ids **differ** from them.
//!
//! ⚠️ **Neither side is a gate on its own.** An `assert_ne!` against a committed constant
//! passes for essentially every value of that constant, so without the capture endpoint
//! beside it a drifting capture ordering would leave B0 green while B5's golden blob had
//! been captured at an id no longer in play (D22(b)). The discriminating thing is the
//! *pair*.
//!
//! ⚠️ **This file is B0's, not B5's.** B5's Lands originally created it, which made the
//! first rung of the ladder read an artefact the sixth rung writes. A rung creating its
//! own instrument is legitimate; a rung whose gate reads an artefact five rungs
//! downstream is not (D22(a)).
//!
//! # Capture procedure — the constants are RE-DERIVED, never edited
//!
//! ```text
//! cargo test -p reflect-fixture --features reflect-fixture/reflect \
//!     --test boundary_roundtrip -- --nocapture
//! ```
//!
//! The capture binary prints `B0 capture: [...]` on every run. Copy the printed vector
//! into [`CAPTURED_FIXTURE_IDS`] and the `Pod3` slot into [`CAPTURED_POD3_ID`], then
//! re-run: the assertion is now closed over a value the binary actually observed.
//!
//! **Gate 1 reding *is* the signal that the capture ordering moved** — the answer is to
//! find out why it moved and whether B5's committed blob is still valid, then re-derive.
//! Editing the constant to match a new observation without that step converts the gate
//! into a rubber stamp.

use super::FIXTURE_TYPE_COUNT;

/// Every fixture id the capture binary mints, in [`super::touch_all`]'s canonical order.
///
/// MEASURED 2026-08-27, worktree `D:/wt/reflect`, `rustc 1.97.1`: the capture binary
/// creates no `EcsMaster` and mints no tag, so the fixtures take the first
/// `FIXTURE_TYPE_COUNT` ids of the process's shared 512-id space, densely, from zero.
///
/// The **whole vector** is pinned and not `Pod3` alone: a pin on one slot is blind to a
/// swap among the others, and B5's byte-identical golden blob is captured over all of
/// them.
pub const CAPTURED_FIXTURE_IDS: [usize; FIXTURE_TYPE_COUNT] = [0, 1, 2, 3, 4, 5, 6, 7];

/// The `Pod3` id at capture — the constant the BOUNDARY plan names at B0 and B5, kept as
/// its own item because that is how both rungs' gates spell it.
///
/// It is written out rather than indexed out of [`CAPTURED_FIXTURE_IDS`]: indexing would
/// make its identity depend on `Pod3` occupying slot 0, which is the very thing the
/// vector assertion is there to check.
pub const CAPTURED_POD3_ID: usize = 0;
