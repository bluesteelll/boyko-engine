//! S2 propagation — Miri Tree-Borrows target for the DESCENT unsafe.
//!
//! This test ACTUALLY EXERCISES the propagation descent's raw-pointer aliasing
//! discipline: the parent-read (`read_global` — `*const u8 → GlobalTransform`,
//! copied out by value) and the child-write (`write_global` — `*mut u8 →
//! GlobalTransform`) over the SAME `GlobalTransform` pool, different rows. The
//! tree is both DEEP (a long chain) and WIDE (fans at several depths) so the
//! descent visits many distinct rows and re-uses its frontier across pops — the
//! exact code path whose soundness `write_global`'s `// SAFETY:` (the value-copy
//! discipline) claims.
//!
//! It deliberately does NOT use `App` (its `ThreadPool` `Arc` teardown leaks,
//! which would muddy the TB signal). The propagation is driven through
//! `world.run_system(propagate_transforms)` directly, so the only `unsafe` under
//! test is the descent's pool access — nothing in the harness allocates a pool
//! the schedule executor would tear down.
//!
//! # Exact Miri filter (MUST cover the descent)
//!
//! ```powershell
//! $env:MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks"
//! cargo +nightly miri test -p boyko-scene --test miri_descent_deep_wide
//! ```
//!
//! Every test in THIS FILE runs the descent (`propagate_transforms` over a
//! multi-row tree), so the `--test miri_descent_deep_wide` filter cannot pass
//! while skipping the descent unsafe — there is no non-descent test in this
//! binary to give a false green.

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_macros::Bundle;
use boyko_math::{Affine3A, Quat, Vec3};
use boyko_threadpool::ThreadPoolBuilder;

use boyko_scene::propagation::compute_global_transform;
use boyko_scene::{GlobalTransform, Transform, propagate_transforms};

#[derive(Bundle)]
struct SpatialBundle {
    transform: Transform,
    global: GlobalTransform,
}

#[inline]
fn spatial(transform: Transform) -> SpatialBundle {
    SpatialBundle { transform, global: GlobalTransform::IDENTITY }
}

/// A tick-advance no-op. Built once per world to lift the change tick off
/// `Tick::ZERO` BEFORE spawning, so the first `propagate_transforms` run sees the
/// spawned spatial rows as dirty and ACTUALLY RUNS THE DESCENT (a tick-0 spawn
/// would be excluded by the `(last_run, this_run]` lower bound, and the descent
/// — the unsafe under test — would be silently skipped, a false Miri green).
fn noop_exclusive(_w: &mut EcsMaster) {}

/// Builds a world with its change tick lifted off ZERO (via a one-shot noop
/// schedule), ready for spawns whose first propagation run will exercise the
/// descent. The schedule's `ThreadPool` `Arc` teardown leaks under Miri — run
/// with `-Zmiri-ignore-leaks` (per the file header), exactly as the kernel's
/// threadpool-bearing Miri suites do; it is orthogonal to the descent's TB signal.
fn world_with_lifted_tick() -> (EcsMaster, Schedule) {
    let mut world = EcsMaster::new();
    let mut b = ScheduleBuilder::new(ThreadPoolBuilder::new().num_threads(2).build());
    b.add_system(noop_exclusive);
    let mut ticker = b.build(&mut world);
    ticker.run(&mut world);
    (world, ticker)
}

fn spawn(world: &mut EcsMaster, t: Transform, parent: Option<Entity>) -> Entity {
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: boyko_ecs::ecs::core::system::Commands| {
        let mut ec = cmds.spawn(spatial(t));
        if let Some(p) = parent {
            ec.set_parent(p);
        }
        *probe.lock().expect("probe") = Some(ec.id());
    });
    sink.lock().expect("probe").expect("handle")
}

fn local_for(i: usize) -> Transform {
    let a = (i as f32) * 0.21;
    Transform {
        translation: Vec3::new(i as f32, -(i as f32) * 0.5, (i as f32) * 0.3),
        rotation: Quat::new(0.0, 0.0, a.sin(), a.cos()).normalize(),
        scale: Vec3::new(1.0 + (i as f32) * 0.05, 1.0, 1.0),
    }
}

/// Combined relative-or-absolute float comparison. The propagation composes
/// top-down (`parent.global ∘ child.local`) while `compute_global_transform`
/// folds bottom-up; the two use the SAME scalar ops but in a different
/// ASSOCIATION order, so over a deep chain with growing scales the results differ
/// in the last f32 bits (≈ machine epsilon, amplified by magnitude). A pure
/// absolute epsilon is too tight at magnitude ~10^4; a relative term tracks the
/// real (tiny) error without masking a genuine wrong-product divergence.
#[track_caller]
fn close(a: f32, b: f32) -> bool {
    const ABS: f32 = 1e-3;
    const REL: f32 = 1e-5;
    (a - b).abs() <= ABS + REL * a.abs().max(b.abs())
}

#[track_caller]
fn assert_affine_eq(got: Affine3A, want: Affine3A, ctx: &str) {
    for r in 0..3 {
        let g = got.matrix3.rows[r];
        let w = want.matrix3.rows[r];
        assert!(
            close(g.x, w.x) && close(g.y, w.y) && close(g.z, w.z),
            "{ctx}: matrix3 row {r} mismatch: got {g:?} want {w:?}"
        );
    }
    let gt = got.translation;
    let wt = want.translation;
    assert!(
        close(gt.x, wt.x) && close(gt.y, wt.y) && close(gt.z, wt.z),
        "{ctx}: translation mismatch: got {gt:?} want {wt:?}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// A DEEP chain — the descent walks many parent-read / child-write hops in one
// run, re-popping the frontier at each depth (small node count to keep Miri fast).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn descent_deep_chain_is_sound_and_correct() {
    const DEPTH: usize = 12;
    let (mut world, _ticker) = world_with_lifted_tick();
    let mut chain = Vec::with_capacity(DEPTH);
    let mut parent = None;
    for i in 0..DEPTH {
        let e = spawn(&mut world, local_for(i), parent);
        chain.push(e);
        parent = Some(e);
    }

    world.run_system(propagate_transforms);

    // Fold the reference world affine down the chain and compare each hop.
    let mut want = Affine3A::IDENTITY;
    for (i, &e) in chain.iter().enumerate() {
        want = if i == 0 { local_for(0).to_affine() } else { want.mul(local_for(i).to_affine()) };
        let got = world.get_component::<GlobalTransform>(e).expect("global").affine();
        assert_affine_eq(got, want, &format!("deep chain depth {i}"));
        // The order-free reference impl agrees too.
        assert_affine_eq(got, compute_global_transform(&world, e), &format!("deep chain ref {i}"));
    }
}

// ════════════════════════════════════════════════════════════════════════════
// A DEEP + WIDE tree — fans at the root and at an interior node, so one descent
// run composes many distinct rows (and re-descends shared ancestors), the worst
// case for the value-copy aliasing discipline.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn descent_deep_wide_tree_is_sound_and_correct() {
    let (mut world, mut ticker) = world_with_lifted_tick();

    // Root with a wide fan; one fan child gets its own deep+wide subtree.
    let root = spawn(&mut world, local_for(1), None);

    const ROOT_FAN: usize = 6;
    let mut fan = Vec::with_capacity(ROOT_FAN);
    for i in 0..ROOT_FAN {
        fan.push(spawn(&mut world, local_for(10 + i), Some(root)));
    }

    // Deepen one fan child into a 4-level chain, each level a small fan of 3.
    let mut deep_nodes = Vec::new();
    let mut frontier = vec![fan[0]];
    for level in 0..4 {
        let mut next = Vec::new();
        for &p in &frontier {
            for k in 0..3 {
                let c = spawn(&mut world, local_for(100 + level * 10 + k), Some(p));
                deep_nodes.push(c);
                next.push(c);
            }
        }
        frontier = next;
    }

    world.run_system(propagate_transforms);

    // Verify EVERY entity (root, fan, deep subtree) against the independent
    // order-free reference — proves the raw-pointer descent wrote correct values
    // to every row it touched.
    let mut all = vec![root];
    all.extend_from_slice(&fan);
    all.extend_from_slice(&deep_nodes);
    for (i, &e) in all.iter().enumerate() {
        let got = world.get_component::<GlobalTransform>(e).expect("global").affine();
        let want = compute_global_transform(&world, e);
        assert_affine_eq(got, want, &format!("deep-wide entity #{i}"));
    }

    // Re-run on the unchanged tree (still-frame skip path), then move the root and
    // re-run (re-descent path) — both exercise the descent unsafe again. Advance
    // the tick (reused ticker) before the move so the next run observes it.
    world.run_system(propagate_transforms);
    ticker.run(&mut world);
    let new_root = local_for(2);
    world.run_system(move |mut cmds: boyko_ecs::ecs::core::system::Commands| {
        cmds.entity(root).insert(new_root);
    });
    world.run_system(propagate_transforms);
    for (i, &e) in all.iter().enumerate() {
        let got = world.get_component::<GlobalTransform>(e).expect("global").affine();
        let want = compute_global_transform(&world, e);
        assert_affine_eq(got, want, &format!("deep-wide entity #{i} after root move"));
    }
}
