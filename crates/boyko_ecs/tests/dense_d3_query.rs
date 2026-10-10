//! Dense plan D3 — dense Query integration tests.
//!
//! Gates (per the D3 ruling):
//! * Pure-dense via `dense_iter` (`Query<&Body>` / `Query<&mut Body>`): all live
//!   slots in insertion order; `&mut` writes land in the column (round-trip).
//! * Mixed via `Query::iter` (`Query<(&Transform, &mut Body)>`): matches exactly
//!   entities with BOTH; correct per-row value; a `Transform`-without-`Body` row
//!   is skipped (the per-row `dense_row_passes` "None ⟹ skip").
//! * `With<Body>` / `Without<Body>` correct.
//! * Determinism: `dense_iter` yields in insertion (slot) order.
//!
//! All dense components are selected by `#[component(storage = "dense")]`; the
//! table side rides a plain `#[derive(Component)]`. The query path is exercised
//! through the public `EcsMaster::query` (`QueryView`) and `run_system`
//! (`Query` SystemParam) surfaces.

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::core::iters::query::data::AnyOf;
use boyko_ecs::ecs::core::iters::query::filter::{With, Without};
use boyko_macros::{Bundle, Component};
use std::sync::{Arc, Mutex};

/// 16-byte POD dense "body" payload.
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

/// `(Transform, Body)` spawn bundle — a dense `Body` rides a table `Transform`.
#[derive(Bundle)]
struct TransformBody {
    t: Transform,
    b: Body,
}

/// `Transform`-only spawn bundle (no `Body`) — the mixed-skip / `Without` row.
#[derive(Bundle)]
struct TransformOnly {
    t: Transform,
}

#[inline]
fn body(x: f32) -> Body {
    Body { x, y: x + 1.0, z: x + 2.0, w: x + 3.0 }
}

// ════════════════════════════════════════════════════════════════════════════
// Pure-dense: `dense_iter` yields all live, insertion order.
// ════════════════════════════════════════════════════════════════════════════

/// Spawn N `(Transform, Body)` via `Commands::spawn`, in a deterministic order,
/// returning the spawned `Body.x` values in spawn order.
fn spawn_bodies(ecs: &mut EcsMaster, n: usize) -> Vec<f32> {
    let mut xs = Vec::new();
    for i in 0..n {
        let x = i as f32 * 10.0;
        xs.push(x);
        ecs.run_system(move |mut cmds: Commands| {
            cmds.spawn(TransformBody { t: Transform { px: x, py: -x }, b: body(x) });
        });
    }
    xs
}

#[test]
fn dense_iter_yields_all_live_in_insertion_order() {
    let mut ecs = EcsMaster::new();
    let xs = spawn_bodies(&mut ecs, 5);

    let view = ecs.query::<&Body, ()>();
    let got: Vec<f32> = view.dense_iter().map(|(_e, b): (_, &Body)| b.x).collect();

    assert_eq!(
        got, xs,
        "dense_iter must yield every live Body.x in insertion (slot) order"
    );
}

#[test]
fn dense_iter_empty_world_yields_nothing() {
    let mut ecs = EcsMaster::new();
    // No Body ever inserted ⇒ store absent ⇒ dense_iter is empty (no panic).
    let view = ecs.query::<&Body, ()>();
    assert_eq!(view.dense_iter().count(), 0, "no dense store ⇒ empty dense_iter");
}

// ════════════════════════════════════════════════════════════════════════════
// Pure-dense mut: `dense_iter_mut` writes land in the column (round-trip).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn dense_iter_mut_writes_round_trip() {
    let mut ecs = EcsMaster::new();
    spawn_bodies(&mut ecs, 4);

    // Phase 1: double every Body.x via dense_iter_mut.
    {
        let mut view = ecs.query::<&mut Body, ()>();
        for (_e, b) in view.dense_iter_mut() {
            b.x *= 2.0;
        }
    }

    // Phase 2: read back via dense_iter — every x doubled, order preserved.
    let view = ecs.query::<&Body, ()>();
    let got: Vec<f32> = view.dense_iter().map(|(_e, b): (_, &Body)| b.x).collect();
    assert_eq!(
        got,
        vec![0.0, 20.0, 40.0, 60.0],
        "dense_iter_mut writes must persist in the column"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Mixed via Query::iter: (&Transform, &mut Body) — both present only; skip
// Transform-without-Body rows; correct per-row value.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn mixed_iter_matches_both_and_skips_transform_without_body() {
    let mut ecs = EcsMaster::new();

    // 3 entities with BOTH Transform + Body.
    for i in 0..3 {
        let x = i as f32;
        ecs.run_system(move |mut cmds: Commands| {
            cmds.spawn(TransformBody { t: Transform { px: x, py: x }, b: body(x * 100.0) });
        });
    }
    // 2 entities with ONLY Transform (no Body) — must be skipped by the mixed
    // dense query (the per-row e2s membership trim).
    for i in 0..2 {
        let x = 1000.0 + i as f32;
        ecs.run_system(move |mut cmds: Commands| {
            cmds.spawn(TransformOnly { t: Transform { px: x, py: x } });
        });
    }

    // Mixed read: collect (Transform.px, Body.x) for matched rows.
    let mut pairs: Vec<(f32, f32)> = ecs
        .query::<(&Transform, &Body), ()>()
        .iter()
        .map(|(t, b): (&Transform, &Body)| (t.px, b.x))
        .collect();
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(
        pairs,
        vec![(0.0, 0.0), (1.0, 100.0), (2.0, 200.0)],
        "mixed iter must yield exactly the Transform+Body rows with correct values"
    );

    // Mixed write-through: scale every matched Body via &mut Body.
    {
        let mut view = ecs.query::<(&Transform, &mut Body), ()>();
        for (_t, b) in view.iter_mut() {
            b.x += 1.0;
        }
    }
    let mut after: Vec<f32> = ecs
        .query::<(&Transform, &Body), ()>()
        .iter()
        .map(|(_t, b): (&Transform, &Body)| b.x)
        .collect();
    after.sort_by(f32::total_cmp);
    assert_eq!(
        after,
        vec![1.0, 101.0, 201.0],
        "mixed &mut Body write-through must persist in the dense column"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// With<Body> / Without<Body>.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn with_dense_filters_to_members() {
    let mut ecs = EcsMaster::new();
    // 2 entities with Body, 3 with only Transform.
    for i in 0..2 {
        let x = i as f32;
        ecs.run_system(move |mut cmds: Commands| {
            cmds.spawn(TransformBody { t: Transform { px: x, py: x }, b: body(x) });
        });
    }
    for i in 0..3 {
        let x = 100.0 + i as f32;
        ecs.run_system(move |mut cmds: Commands| {
            cmds.spawn(TransformOnly { t: Transform { px: x, py: x } });
        });
    }

    // With<Body>: only the Body-bearing rows.
    let with_count = ecs.query::<&Transform, With<Body>>().iter().count();
    assert_eq!(with_count, 2, "With<Body> must keep exactly the 2 Body-bearing rows");

    // Without<Body>: only the non-Body rows.
    let without_count = ecs.query::<&Transform, Without<Body>>().iter().count();
    assert_eq!(
        without_count, 3,
        "Without<Body> must keep exactly the 3 rows lacking Body"
    );

    // Partition coherence: with + without == total Transform rows.
    let total = ecs.query::<&Transform, ()>().iter().count();
    assert_eq!(with_count + without_count, total, "With ∪ Without must partition");
    assert_eq!(total, 5, "5 Transform rows total");
}

#[test]
fn with_dense_values_correct() {
    let mut ecs = EcsMaster::new();
    for i in 0..3 {
        let x = i as f32 * 7.0;
        ecs.run_system(move |mut cmds: Commands| {
            cmds.spawn(TransformBody { t: Transform { px: x, py: x }, b: body(x) });
        });
    }
    // A Body-less row to prove it is excluded by With<Body>.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.spawn(TransformOnly { t: Transform { px: -1.0, py: -1.0 } });
    });

    let mut pxs: Vec<f32> = ecs
        .query::<&Transform, With<Body>>()
        .iter()
        .map(|t: &Transform| t.px)
        .collect();
    pxs.sort_by(f32::total_cmp);
    assert_eq!(
        pxs,
        vec![0.0, 7.0, 14.0],
        "With<Body> must yield exactly the Body-bearing Transforms (px), not the Body-less one"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// W1 — Option<&Dense> / AnyOf<&Dense> yield None-on-absence (not a panic).
//
// A dense component is signature-excluded, so the archetype-level `matches`
// flag is unconditionally `true`. The real per-row membership is the dense
// slot lookup. `Option<&Body>` must therefore yield `Some(&val)` for a row
// with a Body and `None` for a Body-less row — NOT panic on the latter.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn option_dense_yields_some_for_member_none_for_absent() {
    let mut ecs = EcsMaster::new();

    // 3 entities with BOTH Transform + Body; 2 with ONLY Transform.
    for i in 0..3 {
        let x = i as f32;
        ecs.run_system(move |mut cmds: Commands| {
            cmds.spawn(TransformBody { t: Transform { px: x, py: x }, b: body(x * 100.0) });
        });
    }
    for i in 0..2 {
        let x = 1000.0 + i as f32;
        ecs.run_system(move |mut cmds: Commands| {
            cmds.spawn(TransformOnly { t: Transform { px: x, py: x } });
        });
    }

    // `Option<&Body>` over every Transform row: Some(body.x) for members,
    // None for the Body-less rows — must not panic.
    let mut pairs: Vec<(f32, Option<f32>)> = ecs
        .query::<(&Transform, Option<&Body>), ()>()
        .iter()
        .map(|(t, b): (&Transform, Option<&Body>)| (t.px, b.map(|bb| bb.x)))
        .collect();
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

    assert_eq!(
        pairs,
        vec![
            (0.0, Some(0.0)),
            (1.0, Some(100.0)),
            (2.0, Some(200.0)),
            (1000.0, None),
            (1001.0, None),
        ],
        "Option<&Body> must be Some for Body-bearing rows and None for Body-less rows"
    );
}

#[test]
fn option_dense_mut_write_through_skips_absent() {
    let mut ecs = EcsMaster::new();
    for i in 0..2 {
        let x = i as f32;
        ecs.run_system(move |mut cmds: Commands| {
            cmds.spawn(TransformBody { t: Transform { px: x, py: x }, b: body(x) });
        });
    }
    ecs.run_system(move |mut cmds: Commands| {
        cmds.spawn(TransformOnly { t: Transform { px: 9.0, py: 9.0 } });
    });

    // `Option<&mut Body>`: write through Some arms only; None arms are skipped
    // (no panic on the Body-less row).
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
        "Option<&mut Body> write-through must land in the dense column for members only"
    );
}

#[test]
fn anyof_dense_arm_none_for_absent_member() {
    let mut ecs = EcsMaster::new();
    // 2 with Body, 2 without — AnyOf<(&Body,)> over a Transform-bounded query.
    for i in 0..2 {
        let x = i as f32;
        ecs.run_system(move |mut cmds: Commands| {
            cmds.spawn(TransformBody { t: Transform { px: x, py: x }, b: body(x * 10.0) });
        });
    }
    for i in 0..2 {
        let x = 500.0 + i as f32;
        ecs.run_system(move |mut cmds: Commands| {
            cmds.spawn(TransformOnly { t: Transform { px: x, py: x } });
        });
    }

    // `(&Transform, AnyOf<(&Body,)>)`: the AnyOf arm is Some for Body rows and
    // None for Body-less rows — must not panic on the absent member.
    let mut pairs: Vec<(f32, Option<f32>)> = ecs
        .query::<(&Transform, AnyOf<(&Body,)>), ()>()
        .iter()
        .map(|(t, any): (&Transform, (Option<&Body>,))| (t.px, any.0.map(|b| b.x)))
        .collect();
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

    assert_eq!(
        pairs,
        vec![
            (0.0, Some(0.0)),
            (1.0, Some(10.0)),
            (500.0, None),
            (501.0, None),
        ],
        "AnyOf<(&Body,)> arm must be Some for members and None for absent members"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Point lookups: `QueryView::get` / `get_mut` on a DENSE `D`.
//
// Pre-fix these NULL-dereferenced: `get`/`get_mut` never called `resolve_dense`,
// so the dense `fetch` read a NULL `fetch.dense`. The fix resolves the store
// pointer AND checks `dense_row_passes` membership — the matched-archetype bitset
// is a conservative `arch_presence` over-approximation, and a dense component is
// OFF the archetype signature, so an entity can sit in a matched archetype
// without being a live store member (which would trip the fetch `slot_of().expect()`).
// ════════════════════════════════════════════════════════════════════════════

/// Spawns a `(Transform, Body)` via `Commands::spawn` and returns its `Entity`
/// (the deferred spawn reserves the id eagerly, so `.id()` is valid immediately).
fn spawn_transform_body(ecs: &mut EcsMaster, x: f32) -> Entity {
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(TransformBody { t: Transform { px: x, py: -x }, b: body(x) })
            .id();
        *probe.lock().expect("probe lock") = Some(e);
    });
    sink.lock().expect("sink lock").take().expect("spawned entity")
}

/// Spawns a `Transform`-ONLY entity (no `Body`): the SAME `{Transform}` archetype
/// as `spawn_transform_body` (a dense `Body` is off the archetype signature), so it
/// is a NON-MEMBER sitting in a matched (`arch_presence`-seeded) archetype.
fn spawn_transform_only(ecs: &mut EcsMaster, px: f32) -> Entity {
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let e = cmds.spawn(TransformOnly { t: Transform { px, py: -px } }).id();
        *probe.lock().expect("probe lock") = Some(e);
    });
    sink.lock().expect("sink lock").take().expect("spawned entity")
}

#[test]
fn dense_get_returns_the_live_member() {
    // The direct null-deref regression witness: `get` on a dense member now
    // resolves the store and returns the value (pre-fix: read of a NULL fetch.dense).
    let mut ecs = EcsMaster::new();
    let e = spawn_transform_body(&mut ecs, 42.0);
    let got = ecs.query::<&Body, ()>().get(e).copied();
    assert_eq!(got, Some(body(42.0)), "get on a dense member yields its value");
}

#[test]
fn dense_get_mut_mutation_persists() {
    let mut ecs = EcsMaster::new();
    let e = spawn_transform_body(&mut ecs, 10.0);
    {
        let mut view = ecs.query::<&mut Body, ()>();
        let b = view.get_mut(e).expect("live dense member");
        b.x = 99.0;
    }
    let got = ecs.query::<&Body, ()>().get(e).map(|b| b.x);
    assert_eq!(got, Some(99.0), "get_mut write-through to the dense column persists");
}

#[test]
fn dense_get_non_member_in_matched_archetype_is_none_not_panic() {
    // The membership-guard witness: a Transform-only entity shares the {Transform}
    // archetype with a real Body member (dense Body is off-signature), so Body's
    // `arch_presence` bitset MATCHES its archetype — the guard, not the bitset, must
    // reject it. Pre-guard this reached the dense fetch's `slot_of().expect()` panic.
    let mut ecs = EcsMaster::new();
    let _member = spawn_transform_body(&mut ecs, 1.0);
    let non_member = spawn_transform_only(&mut ecs, 2.0);
    assert!(
        ecs.query::<&Body, ()>().get(non_member).is_none(),
        "a non-member in a matched (arch_presence) archetype must be None, not a panic"
    );
    let mut view = ecs.query::<&mut Body, ()>();
    assert!(
        view.get_mut(non_member).is_none(),
        "get_mut mirrors get for a non-member"
    );
}
