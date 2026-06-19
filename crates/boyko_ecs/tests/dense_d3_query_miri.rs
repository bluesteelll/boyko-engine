//! Dense plan D3 — Miri Tree-Borrows + data-race validation for the dense
//! Query integration (W2 — the code-reviewer-flagged "no Miri twin" gap).
//!
//! Every test drives a D3 unsafe surface through the PUBLIC query path
//! (`EcsMaster::query` / `run_system` → `Query`), so the actual unsafe code is
//! interpreted by Miri, not modelled by a hand-rolled `DenseStore` poke. The
//! surfaces, each in its own test for per-surface reporting:
//!
//! * (a) MIXED `iter_mut` write-through `Query<(&Transform, &mut Body)>` — the
//!   per-row gather `entity_ids[row] → slot_of → row_ptr as *mut → &mut` plus
//!   the write must land; covers `&mut T::fetch`'s dense arm + `dense_item`.
//! * (b) `dense_iter_mut` round-trip — the contiguous `DenseCursor` stride
//!   (`base + slot*stride`, the `live` bitmap read, `s2e` read) + the `&mut`
//!   built by `&mut T::dense_item`; the write must persist.
//! * (c) MIXED query where SOME rows have the dense member and some DON'T — the
//!   per-row `dense_row_passes` skip interleaved with the gather (no OOB / no
//!   stale-slot read on the skipped rows).
//! * (d) the W1 paths — `Option<&Body>` / `Option<&mut Body>` / `AnyOf<(&Body,)>`
//!   over present + absent entities: the conditional inner-fetch
//!   (`if D::dense_row_passes { Some(D::fetch) } else { None }`) must produce
//!   `None` on a miss WITHOUT dereferencing a missing slot, and the `Some`
//!   write-through must land.
//! * (e) `With<Body>` / `Without<Body>` per-row membership — the dense-seeded
//!   candidate scan + per-row trim.
//!
//! Assert per surface: zero UB (Tree Borrows, data-race, OOB), correct results,
//! and that every `&mut` write persists.
//!
//! The 8 leak reports from the Commands / `run_system` RawVec apply path are
//! PRE-EXISTING (see the phase-19 / command-queue memory) and unrelated to D3;
//! they are suppressed by `-Zmiri-ignore-leaks` per the run line below.
//!
//! Run (toolchain note — nightly GNU):
//! ```text
//! RUSTUP_TOOLCHAIN=nightly-x86_64-pc-windows-gnu \
//!   MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks -Zmiri-disable-isolation" \
//!   cargo miri test -p boyko-ecs --test dense_d3_query_miri
//! ```

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::data::AnyOf;
use boyko_ecs::ecs::core::iters::query::filter::{With, Without};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

/// 16-byte POD dense "body" payload (signature-excluded, global column).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[component(storage = "dense")]
#[repr(C)]
struct Body {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

/// A plain TABLE component the dense `Body` rides alongside.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Transform {
    px: f32,
    py: f32,
}

#[derive(Bundle)]
struct TransformBody {
    t: Transform,
    b: Body,
}

#[derive(Bundle)]
struct TransformOnly {
    t: Transform,
}

#[inline]
fn body(x: f32) -> Body {
    Body { x, y: x + 1.0, z: x + 2.0, w: x + 3.0 }
}

/// Spawn one `(Transform, Body)` via a one-shot system (the real structural
/// path). Small counts keep Miri tractable.
fn spawn_tb(ecs: &mut EcsMaster, x: f32) {
    ecs.run_system(move |mut cmds: Commands| {
        cmds.spawn(TransformBody { t: Transform { px: x, py: -x }, b: body(x) });
    });
}

/// Spawn one `Transform`-only entity (no dense member) — the skip / absent row.
fn spawn_t(ecs: &mut EcsMaster, x: f32) {
    ecs.run_system(move |mut cmds: Commands| {
        cmds.spawn(TransformOnly { t: Transform { px: x, py: -x } });
    });
}

// ════════════════════════════════════════════════════════════════════════════
// (a) MIXED iter_mut write-through: Query<(&Transform, &mut Body)>.
//     Exercises &mut T::fetch dense arm (row → entity → slot → row_ptr as *mut)
//     + &mut T::dense_item (&mut *(ptr as *mut T)); the write must persist.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_mixed_iter_mut_write_through_lands() {
    let mut ecs = EcsMaster::new();
    for i in 0..4 {
        spawn_tb(&mut ecs, i as f32);
    }

    // Read pass first (the &mut T::fetch read provenance is exercised on the
    // collect; then the &mut write provenance on the mutate pass).
    {
        let mut view = ecs.query::<(&Transform, &mut Body), ()>();
        for (t, b) in view.iter_mut() {
            // SAFETY of the read: the gather proved the slot live; this is the
            // public &mut, so the deref + write are in-contract.
            b.x = t.px * 10.0 + 1.0;
        }
    }

    // Verify the write landed (read back through a read-only mixed query).
    let mut got: Vec<f32> = ecs
        .query::<(&Transform, &Body), ()>()
        .iter()
        .map(|(_t, b): (&Transform, &Body)| b.x)
        .collect();
    got.sort_by(f32::total_cmp);
    assert_eq!(
        got,
        vec![1.0, 11.0, 21.0, 31.0],
        "mixed &mut Body write-through must persist (no UB on the per-row gather)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// (b) dense_iter_mut round-trip: the contiguous DenseCursor stride + &mut.
//     Exercises DenseCursor::next_live (live bitmap read, s2e read,
//     base + slot*stride) + &mut T::dense_item; the write must persist.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_dense_iter_mut_stride_round_trip() {
    let mut ecs = EcsMaster::new();
    for i in 0..5 {
        spawn_tb(&mut ecs, i as f32 * 2.0);
    }

    // Phase 1: stride the contiguous column via dense_iter_mut, write each x.
    {
        let mut view = ecs.query::<&mut Body, ()>();
        for (_e, b) in view.dense_iter_mut() {
            b.x += 100.0;
        }
    }

    // Phase 2: read back via the read-only dense cursor — order preserved.
    let got: Vec<f32> = ecs
        .query::<&Body, ()>()
        .dense_iter()
        .map(|(_e, b): (_, &Body)| b.x)
        .collect();
    assert_eq!(
        got,
        vec![100.0, 102.0, 104.0, 106.0, 108.0],
        "dense_iter_mut contiguous-stride write must persist in slot order (no UB on stride/live/s2e)"
    );
}

#[test]
fn miri_dense_iter_read_only_strides_clean() {
    // Pure-read cursor: exercises next_live + &T::dense_item read provenance.
    let mut ecs = EcsMaster::new();
    for i in 0..6 {
        spawn_tb(&mut ecs, i as f32);
    }
    let view = ecs.query::<&Body, ()>();
    let sum: f32 = view.dense_iter().map(|(_e, b): (_, &Body)| b.x).sum();
    assert_eq!(sum, 0.0 + 1.0 + 2.0 + 3.0 + 4.0 + 5.0, "read cursor sum (no UB)");
}

#[test]
fn miri_dense_iter_empty_store_is_clean() {
    // No Body ever inserted ⇒ NULL store ⇒ DenseCursor::new takes the NULL arm
    // (len == 0). Must not deref the NULL base/live/s2e.
    let mut ecs = EcsMaster::new();
    let view = ecs.query::<&Body, ()>();
    assert_eq!(view.dense_iter().count(), 0, "NULL store ⇒ empty cursor (no NULL deref)");
}

// ════════════════════════════════════════════════════════════════════════════
// (c) MIXED query where SOME rows have the dense member and some DON'T — the
//     per-row dense_row_passes skip interleaved with the gather. The skipped
//     rows must NOT read a missing slot (no OOB / stale read).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_mixed_skip_interleave_no_oob() {
    let mut ecs = EcsMaster::new();

    // Interleave members and non-members so the skip/gather alternate per row.
    spawn_tb(&mut ecs, 0.0); // member
    spawn_t(&mut ecs, 1000.0); // absent
    spawn_tb(&mut ecs, 1.0); // member
    spawn_t(&mut ecs, 1001.0); // absent
    spawn_tb(&mut ecs, 2.0); // member

    // Read-only mixed: (&Transform, &Body) — only the 3 members survive the
    // per-row dense_row_passes trim; the 2 absent rows are skipped (no deref).
    let mut pairs: Vec<(f32, f32)> = ecs
        .query::<(&Transform, &Body), ()>()
        .iter()
        .map(|(t, b): (&Transform, &Body)| (t.px, b.x))
        .collect();
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(
        pairs,
        vec![(0.0, 0.0), (1.0, 1.0), (2.0, 2.0)],
        "mixed query must yield exactly the member rows (skip the absent without OOB)"
    );

    // Mutable mixed over the interleave: the skip must not touch a missing slot
    // on the &mut path either.
    {
        let mut view = ecs.query::<(&Transform, &mut Body), ()>();
        for (_t, b) in view.iter_mut() {
            b.x += 0.5;
        }
    }
    let mut after: Vec<f32> = ecs
        .query::<&Body, ()>()
        .dense_iter()
        .map(|(_e, b): (_, &Body)| b.x)
        .collect();
    after.sort_by(f32::total_cmp);
    assert_eq!(
        after,
        vec![0.5, 1.5, 2.5],
        "mixed &mut over an interleave writes members only (no UB on the skip)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// (d) W1 — Option<&Body> / Option<&mut Body> / AnyOf<(&Body,)> over present +
//     absent: the conditional inner-fetch None-on-miss must not deref a missing
//     slot, and the Some write-through must land.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_option_dense_read_none_on_absent() {
    let mut ecs = EcsMaster::new();
    spawn_tb(&mut ecs, 0.0);
    spawn_t(&mut ecs, 1000.0);
    spawn_tb(&mut ecs, 1.0);

    // Option<&Body>: Some(x) for members, None for the absent — the absent row
    // must take the `None` arm WITHOUT calling D::fetch (no missing-slot deref).
    let mut pairs: Vec<(f32, Option<f32>)> = ecs
        .query::<(&Transform, Option<&Body>), ()>()
        .iter()
        .map(|(t, b): (&Transform, Option<&Body>)| (t.px, b.map(|bb| bb.x)))
        .collect();
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(
        pairs,
        vec![(0.0, Some(0.0)), (1.0, Some(1.0)), (1000.0, None)],
        "Option<&Body> None-on-absence (no deref of the missing slot)"
    );
}

#[test]
fn miri_option_dense_mut_write_through_skips_absent() {
    let mut ecs = EcsMaster::new();
    spawn_tb(&mut ecs, 0.0);
    spawn_tb(&mut ecs, 1.0);
    spawn_t(&mut ecs, 9.0);

    // Option<&mut Body>: write through Some arms only; None arms skipped (no
    // UB on the conditional inner mut-fetch).
    {
        let mut view = ecs.query::<(&Transform, Option<&mut Body>), ()>();
        let mut none_seen = false;
        for (_t, b) in view.iter_mut() {
            match b {
                Some(bb) => bb.x += 1000.0,
                None => none_seen = true,
            }
        }
        assert!(none_seen, "the Body-less row must surface as Option None");
    }

    let mut got: Vec<f32> = ecs
        .query::<&Body, ()>()
        .dense_iter()
        .map(|(_e, b): (_, &Body)| b.x)
        .collect();
    got.sort_by(f32::total_cmp);
    assert_eq!(
        got,
        vec![1000.0, 1001.0],
        "Option<&mut Body> write-through lands for members only (no UB on the None arm)"
    );
}

#[test]
fn miri_anyof_dense_arm_none_on_absent() {
    let mut ecs = EcsMaster::new();
    spawn_tb(&mut ecs, 0.0);
    spawn_t(&mut ecs, 500.0);
    spawn_tb(&mut ecs, 1.0);

    // AnyOf<(&Body,)>: the arm is Some for members, None for the absent — same
    // conditional inner-fetch as Option, must not deref the missing slot.
    let mut pairs: Vec<(f32, Option<f32>)> = ecs
        .query::<(&Transform, AnyOf<(&Body,)>), ()>()
        .iter()
        .map(|(t, any): (&Transform, (Option<&Body>,))| (t.px, any.0.map(|b| b.x)))
        .collect();
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(
        pairs,
        vec![(0.0, Some(0.0)), (1.0, Some(1.0)), (500.0, None)],
        "AnyOf<(&Body,)> arm None-on-absence (no deref of the missing slot)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// (e) With<Body> / Without<Body> per-row membership — the dense-seeded
//     candidate scan + per-row trim. No UB on the membership read of either
//     the present or absent rows.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn miri_with_without_dense_membership_clean() {
    let mut ecs = EcsMaster::new();
    spawn_tb(&mut ecs, 0.0);
    spawn_tb(&mut ecs, 1.0);
    spawn_t(&mut ecs, 100.0);
    spawn_t(&mut ecs, 101.0);
    spawn_t(&mut ecs, 102.0);

    // With<Body>: collect the px of Body-bearing rows (the membership oracle is
    // the per-row dense slot_of read — must be UB-clean on present rows).
    let mut with_px: Vec<f32> = ecs
        .query::<&Transform, With<Body>>()
        .iter()
        .map(|t: &Transform| t.px)
        .collect();
    with_px.sort_by(f32::total_cmp);
    assert_eq!(with_px, vec![0.0, 1.0], "With<Body> keeps exactly the Body-bearing rows");

    // Without<Body>: the complement (membership read on absent rows, UB-clean).
    let without_count = ecs.query::<&Transform, Without<Body>>().iter().count();
    assert_eq!(without_count, 3, "Without<Body> keeps exactly the Body-less rows");

    let total = ecs.query::<&Transform, ()>().iter().count();
    assert_eq!(with_px.len() + without_count, total, "With ∪ Without partitions the rows");
}
