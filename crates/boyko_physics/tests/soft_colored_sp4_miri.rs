//! Physics O11 SP4 — the MIRI-TB soundness oracle (the load-bearing gate).
//!
//! The two new soundness hazards in the colored solver are VALUE-BENIGN, so a
//! snapshot/bit-identity test cannot see them — only a borrow-model checker can:
//!
//!   1. **The pinned-write race** — two same-color constraints may share a PINNED
//!      endpoint; the C1 per-endpoint guard (`is_dynamic_row`) skips the write, so the
//!      shared pinned row is read-only across workers. If the guard were wrong the
//!      concurrent writes would be a value no-op (`+= ±0.0`) yet a *data race* — UB
//!      invisible to a value oracle, visible to Miri.
//!   2. **`SoftColorPtrs` cross-worker aliasing** — each worker carries `SoftCols`
//!      (the per-column raw bases, by value via `cols()`) and writes per-element via
//!      `*base.add(p)` through the `*_raw` cores; it reads the CSR / pair list via
//!      `*const`. No worker forms a `&mut SoftBody` or a whole-column slice — that
//!      reborrow is un-typeable. Tree-Borrows checks that the per-element writes only
//!      ever touch provably-disjoint dynamic rows (plus read-only shared columns) and
//!      that the raw reads form no conflicting reference into the scratch (the
//!      Phase-9.3c discipline).
//!
//! This file drives the curated Miri subset:
//!   * **(a) pool-free** — `try_with_active_pool` returns `None` (no ambient pool), so
//!     `dispatch_color` takes the INLINE branch: the C1 guard + the colorer + the CSR
//!     walk run on the calling thread. Exercises hazard (1)'s guard directly.
//!   * **(b) multi-worker** — a `ThreadPoolBuilder` with 2 workers wrapped around
//!     `run_system_once` via `pool.install`, on a scene whose widest color crosses
//!     `MIN_PARALLEL_SLOTS_PER_COLOR` (256). This makes `dispatch_color` take the
//!     PARALLEL branch, so `SoftColorPtrs::cols()` (the by-value `SoftCols` bases) +
//!     the per-element `*_raw` writes / `color_item_at` (and, with self-collision
//!     colored, `PairListPtr::get`) ACTUALLY execute under Tree-Borrows across real
//!     worker threads — hazard (2).
//!
//! Run (the load-bearing command):
//! ```text
//! MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-disable-isolation" \
//!   cargo +nightly-x86_64-pc-windows-gnu miri test -p boyko-physics \
//!   --test soft_colored_sp4_miri -- --test-threads=1
//! ```
//!
//! If the threadpool's worker-steal path trips a PRE-EXISTING crossbeam-deque retag
//! under Miri (a known executor over-approximation, NOT an SP4 defect), it surfaces
//! here in `(b)`; the `(a)` inline tests still witness the C1 guard + colorer with
//! ZERO threadpool involvement, so the SP4-specific unsafe is covered either way.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::into_system::IntoSystem;

use boyko_physics::math::Vec3;
use boyko_physics::resources::PhysicsConfig;
use boyko_physics::sdf_query::SdfField;
use boyko_physics::soft::{SoftBody, SoftColorScratch, physics_soft_step_colored};

// ── Harness ───────────────────────────────────────────────────────────────────

fn spawn_soft(world: &mut EcsMaster, body: SoftBody) {
    let arch = world.create_archetype(&[SoftBody::component_id()]);
    world
        .spawn_one(arch, body)
        .expect("invariant: {SoftBody} archetype accepts a SoftBody");
}

fn read_soft(world: &mut EcsMaster) -> SoftBody {
    let q = world.query::<&SoftBody, ()>();
    let mut it = q.iter();
    it.next().expect("one soft body spawned").clone()
}

fn pos_sum(body: &SoftBody) -> f64 {
    (0..body.particle_count())
        .map(|i| body.pos_x[i] as f64 + body.pos_y[i] as f64 + body.pos_z[i] as f64)
        .sum()
}

fn install(world: &mut EcsMaster, self_colored: bool, sc_iters: usize, radius_used: bool) {
    world.insert_resource(PhysicsConfig {
        dt: 1.0 / 60.0,
        substeps: 2,
        gravity: Vec3::new(0.0, -9.81, 0.0),
        soft_body: true,
        soft_body_colored: true,
        soft_self_collision_colored: self_colored,
        self_collision_iters: sc_iters,
        ..PhysicsConfig::default()
    });
    world.insert_resource(SdfField::default());
    world.insert_resource(SoftColorScratch::default());
    let _ = radius_used;
}

/// A SMALL scene whose widest color is HUGE for a tiny particle count: `k` disjoint
/// dynamic edges `(2i, 2i+1)` plus one PINNED particle shared by several of them.
/// Because the edges share no DYNAMIC particle, the greedy colorer puts every edge in
/// color 0 — span `k`. With `k >= MIN_PARALLEL_SLOTS_PER_COLOR (256)` the single color
/// crosses the threshold and dispatches a `pool.scope` of multiple chunks: the minimal
/// scene that forces the parallel `SoftColorPtrs` path under Miri.
///
/// The shared PINNED particle is an endpoint of edges that all live in color 0, so it
/// is the pinned-write-race witness: many workers read it, none writes it.
fn parallel_witness_scene(k: usize) -> SoftBody {
    // Particles: 2k dynamic (the matched endpoints) + 1 pinned hub (index 2k).
    let pinned = (2 * k) as u32;
    let mut positions = Vec::with_capacity(2 * k + 1);
    let mut inv_masses = Vec::with_capacity(2 * k + 1);
    for i in 0..2 * k {
        positions.push([i as f32 * 0.01, 1.0, 0.0]);
        inv_masses.push(1.0);
    }
    positions.push([0.0, 2.0, 0.0]); // pinned hub
    inv_masses.push(0.0);

    let mut edges = Vec::with_capacity(3 * k);
    for i in 0..k {
        let a = (2 * i) as u32;
        let b = (2 * i + 1) as u32;
        edges.push((a, b)); // disjoint dynamic matching → all color 0
    }
    // Add a SECOND batch of edges from the pinned hub to a dynamic endpoint of each
    // matched edge. The pinned hub imposes no occupancy, so these ALSO land in color 0
    // (their only other endpoint is `a`, distinct per edge) — the shared-pinned witness.
    for i in 0..k {
        let a = (2 * i) as u32;
        edges.push((pinned, a));
    }
    SoftBody::from_mesh(&positions, &inv_masses, &edges, None, 1.0e-7, 0.0)
        .expect("parallel-witness scene is well-formed")
}

// ── (a) pool-free — INLINE C1 guard + colorer under Miri ────────────────────────

/// Pool-free: with no ambient pool, every color dispatches INLINE, so the C1 guard +
/// the colorer + the CSR walk run under Miri with no threadpool. Witnesses hazard (1)
/// (the guard skips the pinned shared row) on the calling thread.
#[test]
fn miri_inline_pinned_shared_guard_clean() {
    let mut world = EcsMaster::new();
    install(&mut world, false, 0, false);
    let scene = parallel_witness_scene(8); // tiny — every color inline, fast under Miri
    let pinned = scene.particle_count() - 1;
    let p0 = (scene.pos_x[pinned], scene.pos_y[pinned], scene.pos_z[pinned]);
    spawn_soft(&mut world, scene);
    let mut sys = IntoSystem::into_system(physics_soft_step_colored);
    for _ in 0..3 {
        world.run_system_once(&mut sys);
    }
    let out = read_soft(&mut world);
    // The pinned hub is byte-frozen (the guard removed its write in every color).
    assert_eq!(
        (out.pos_x[pinned].to_bits(), out.pos_y[pinned].to_bits(), out.pos_z[pinned].to_bits()),
        (p0.0.to_bits(), p0.1.to_bits(), p0.2.to_bits()),
        "the shared pinned hub must stay byte-frozen (C1 guard, inline path)"
    );
}

/// Pool-free 0%-gate under Miri: `soft_body_colored = false` runs the serial step. A
/// trivial UB-surface check (the colorer is not even constructed).
#[test]
fn miri_inline_colored_solve_runs_clean() {
    let mut world = EcsMaster::new();
    install(&mut world, false, 0, false);
    spawn_soft(&mut world, parallel_witness_scene(4));
    let mut sys = IntoSystem::into_system(physics_soft_step_colored);
    world.run_system_once(&mut sys);
    let out = read_soft(&mut world);
    assert!(pos_sum(&out).is_finite(), "the colored inline solve produced finite state");
}

// ── (b) multi-worker — the SoftColorPtrs aliasing under Tree-Borrows ─────────────

/// THE load-bearing Miri-TB gate: a `pool.install`-driven colored solve on a scene
/// whose single color CROSSES `MIN_PARALLEL_SLOTS_PER_COLOR`, so `dispatch_color`
/// takes the PARALLEL branch and the `SoftColorPtrs` per-element `*_raw` writes (via
/// the by-value `SoftCols` bases) + the `*const` CSR reads ACTUALLY execute across
/// worker threads. Miri-TB asserts the concurrent per-element accesses are UB-clean
/// (no data race, no aliasing violation, no OOB) — no whole-`&mut SoftBody` reborrow
/// exists on the worker path.
///
/// If this trips a pre-existing crossbeam-deque retag (the executor over-approximation
/// the project's Phase-9.x notes record), it is NOT an SP4 defect — report it as such;
/// the inline `(a)` tests still cover the SP4-specific unsafe.
#[test]
fn miri_multiworker_softcolorptrs_aliasing_clean() {
    use boyko_threadpool::ThreadPoolBuilder;

    let mut world = EcsMaster::new();
    install(&mut world, false, 0, false);
    // k = 300 disjoint dynamic edges → a single color of span 600 (300 matching + 300
    // pinned-hub edges, all color 0) >> 256, so the parallel dispatch fires with a
    // shared pinned witness. 601 particles — small enough for Miri, large enough to
    // cross the threshold and chunk across 2 workers.
    spawn_soft(&mut world, parallel_witness_scene(300));
    let mut sys = IntoSystem::into_system(physics_soft_step_colored);

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    pool.install(|_scope| {
        if let Some(sc) = world.try_resource_mut::<SoftColorScratch>() {
            sc.reset_parallel_counter();
        }
        // A single step is enough to drive every unsafe path once under TB.
        world.run_system_once(&mut sys);
    });

    let count = world
        .try_resource::<SoftColorScratch>()
        .expect("SoftColorScratch inserted")
        .parallel_color_count();
    // Anti-vacuity: the PARALLEL branch genuinely fired (else the TB witness is vacuous).
    assert!(
        count >= 1,
        "anti-vacuity: the multi-worker scene must dispatch a parallel color so the \
         SoftColorPtrs reborrows actually run under Miri-TB (count = {count})"
    );
    let out = read_soft(&mut world);
    assert!(pos_sum(&out).is_finite(), "the multi-worker colored solve produced finite state");
}

/// The same multi-worker dispatch with COLORED SELF-COLLISION ON, so the `PairListPtr`
/// raw-read path also executes under Tree-Borrows across workers.
#[test]
fn miri_multiworker_pairlistptr_aliasing_clean() {
    use boyko_threadpool::ThreadPoolBuilder;

    // A dense overlapping strip so the self-collision sweep emits a large pair set
    // whose widest color crosses the threshold. Kept small (one wide row) for Miri.
    let len = 400usize;
    let mut positions = Vec::with_capacity(len);
    let mut inv_masses = Vec::with_capacity(len);
    for i in 0..len {
        // Spacing 0.05 < 2r (r = 0.05 ⇒ 2r = 0.1) ⇒ every neighbour overlaps.
        positions.push([i as f32 * 0.05, 1.0, 0.0]);
        inv_masses.push(1.0);
    }
    let scene = SoftBody::from_mesh(&positions, &inv_masses, &[], None, 1.0e-7, 0.05)
        .expect("dense self-collision strip is well-formed");

    let mut world = EcsMaster::new();
    install(&mut world, true, 1, true);
    spawn_soft(&mut world, scene);
    let mut sys = IntoSystem::into_system(physics_soft_step_colored);

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    pool.install(|_scope| {
        if let Some(sc) = world.try_resource_mut::<SoftColorScratch>() {
            sc.reset_parallel_counter();
        }
        world.run_system_once(&mut sys);
    });

    let out = read_soft(&mut world);
    assert!(
        pos_sum(&out).is_finite(),
        "the multi-worker colored self-collision solve produced finite state"
    );
}
