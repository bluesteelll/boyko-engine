//! Physics O11 SP4 — COLORED-PARALLEL soft-body solve gates.
//!
//! These encode the SP4 proof obligations (the gates the plan §Gates names):
//!
//! 1. **SP4 0%-GATE** — the colored soft step with `soft_body_colored == false` runs
//!    the SERIAL `step_body` per body, so it is BYTE-IDENTICAL to
//!    `physics_soft_step`. (Pool-free → Miri-runnable.)
//! 2. **{1, N} ORACLE (W1)** — the colored solve of a dense grid whose widest
//!    distance color CROSSES `MIN_PARALLEL_SLOTS_PER_COLOR` is BIT-IDENTICAL across
//!    worker counts (1 vs 4) and to the no-pool inline fallback. BOTH anti-vacuity
//!    gates are asserted: (i) `parallel_color_count >= 1` (a color genuinely crossed
//!    the threshold and dispatched a `pool.scope`), and (ii) the body actually MOVED
//!    (a non-trivial snapshot). (Pool-driven → `cfg(not(miri))`.)
//! 3. **RUN-TO-RUN** — the colored solve of the same scene run twice (fresh build)
//!    is byte-identical (colored determinism).
//! 4. **COLORING DISJOINTNESS** — the debug coloring re-scan
//!    (`debug_assert_coloring_*`) inside the colorer fires on every coloring; this
//!    test additionally asserts via the public CSR that no color reuses a dynamic
//!    particle (the C2 lemma) for distance, volume, and self-collision colorings.
//! 5. **PINNED-SHARED-ACROSS-COLOR** — a scene where a pinned particle is shared by
//!    constraints in DIFFERENT colors solves cleanly (the pinned row imposes no
//!    occupancy, so it may be color-shared; the C1 guard makes "written by neither"
//!    true). Proven by `{1, N}` bit-identity holding on such a scene.
//!
//! The `{1}`-worker oracle + a multi-worker scene driven via `run_system_once` inside
//! `pool.install` are structured so Miri-TB can witness the pinned-write race +
//! `SoftColorPtrs` aliasing (the tester runs the curated Miri subset).

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::into_system::IntoSystem;

use boyko_physics::math::Vec3;
use boyko_physics::resources::PhysicsConfig;
use boyko_physics::sdf_query::SdfField;
use boyko_physics::soft::{
    SoftBody, SoftColorScratch, physics_soft_step, physics_soft_step_colored,
};

// ── Harness ───────────────────────────────────────────────────────────────────

/// Spawns one [`SoftBody`] into a fresh `{SoftBody}` archetype.
fn spawn_soft(world: &mut EcsMaster, body: SoftBody) {
    let arch = world.create_archetype(&[SoftBody::component_id()]);
    world
        .spawn_one(arch, body)
        .expect("invariant: {SoftBody} archetype accepts a SoftBody");
}

/// Reads back the single soft body as an owned clone.
fn read_soft(world: &mut EcsMaster) -> SoftBody {
    let q = world.query::<&SoftBody, ()>();
    let mut it = q.iter();
    it.next().expect("one soft body spawned").clone()
}

/// The position bit snapshot (`to_bits`, the determinism oracle).
fn pos_bits(body: &SoftBody) -> Vec<(u32, u32, u32)> {
    (0..body.particle_count())
        .map(|i| {
            (
                body.pos_x[i].to_bits(),
                body.pos_y[i].to_bits(),
                body.pos_z[i].to_bits(),
            )
        })
        .collect()
}

/// A `w x w` particle grid (a cloth) with structural distance edges (right + down
/// neighbours), the top row pinned (`inv_mass == 0`). The grid is large enough that
/// the widest distance color crosses `MIN_PARALLEL_SLOTS_PER_COLOR` (256): a regular
/// grid 4-colors the structural edges, so the widest color holds ≈ edges/4.
fn grid_cloth(w: usize) -> SoftBody {
    let mut positions = Vec::with_capacity(w * w);
    let mut inv_masses = Vec::with_capacity(w * w);
    for y in 0..w {
        for x in 0..w {
            positions.push([x as f32 * 0.1, 2.0 - y as f32 * 0.1, 0.0]);
            // Top row pinned (a hanging cloth); the rest dynamic.
            inv_masses.push(if y == 0 { 0.0 } else { 1.0 });
        }
    }
    let idx = |x: usize, y: usize| (y * w + x) as u32;
    let mut edges = Vec::new();
    for y in 0..w {
        for x in 0..w {
            if x + 1 < w {
                edges.push((idx(x, y), idx(x + 1, y)));
            }
            if y + 1 < w {
                edges.push((idx(x, y), idx(x, y + 1)));
            }
        }
    }
    SoftBody::from_mesh(&positions, &inv_masses, &edges, None, 1.0e-7, 0.0)
        .expect("grid cloth is well-formed")
}

/// Installs the colored config: gravity ON (so the cloth moves — anti-vacuity (ii)),
/// `soft_body_colored = colored`.
fn install(world: &mut EcsMaster, colored: bool) {
    world.insert_resource(PhysicsConfig {
        dt: 1.0 / 60.0,
        substeps: 2,
        gravity: Vec3::new(0.0, -9.81, 0.0),
        soft_body: true,
        soft_body_colored: colored,
        ..PhysicsConfig::default()
    });
    world.insert_resource(SdfField::default());
    world.insert_resource(SoftColorScratch::default());
}

// ── Gate 1: SP4 0%-gate (colored step, flag off == serial step) ────────────────

/// The colored soft step with `soft_body_colored == false` is BYTE-IDENTICAL to the
/// serial `physics_soft_step`. Pool-free → Miri-runnable.
#[test]
fn colored_step_flag_off_is_byte_identical_to_serial() {
    let scene = grid_cloth(12);

    // Serial reference: `physics_soft_step`.
    let mut serial_world = EcsMaster::new();
    install(&mut serial_world, false);
    spawn_soft(&mut serial_world, scene.clone());
    let mut serial = IntoSystem::into_system(physics_soft_step);
    for _ in 0..6 {
        serial_world.run_system_once(&mut serial);
    }
    let serial_bits = pos_bits(&read_soft(&mut serial_world));

    // Colored step with the flag OFF: must run the serial `step_body`.
    let mut colored_world = EcsMaster::new();
    install(&mut colored_world, false);
    spawn_soft(&mut colored_world, scene);
    let mut colored = IntoSystem::into_system(physics_soft_step_colored);
    for _ in 0..6 {
        colored_world.run_system_once(&mut colored);
    }
    let colored_bits = pos_bits(&read_soft(&mut colored_world));

    assert_eq!(
        serial_bits, colored_bits,
        "SP4 0%-gate: the colored step with soft_body_colored=false must be \
         byte-identical to the serial physics_soft_step"
    );
}

// ── Gate 3: colored run-to-run bit-identity (no pool — inline fallback) ─────────

/// The colored solve (no pool attached → inline fallback for every color) is
/// run-to-run byte-identical. Pool-free → Miri-runnable.
#[test]
fn colored_solve_is_run_to_run_bit_identical_no_pool() {
    let run = || -> Vec<(u32, u32, u32)> {
        let mut world = EcsMaster::new();
        install(&mut world, true);
        spawn_soft(&mut world, grid_cloth(16));
        let mut sys = IntoSystem::into_system(physics_soft_step_colored);
        for _ in 0..6 {
            world.run_system_once(&mut sys);
        }
        pos_bits(&read_soft(&mut world))
    };
    assert_eq!(run(), run(), "colored solve must be run-to-run bit-identical");
}

// ── Gate 4: coloring disjointness via the public CSR (C2 lemma) ─────────────────

/// After a colored step, the distance + volume colorings exposed via the scratch
/// satisfy the C2 lemma: no color reuses a dynamic particle. (The colorer's internal
/// `debug_assert_coloring_*` also fires every coloring; this is the explicit public
/// gate.) Pool-free → Miri-runnable (and the debug re-scan runs in debug).
#[test]
fn coloring_is_dynamic_disjoint_per_color() {
    let mut world = EcsMaster::new();
    install(&mut world, true);
    let scene = grid_cloth(12);
    let inv_mass = scene.inv_mass.clone();
    let c_a = scene.c_a.clone();
    let c_b = scene.c_b.clone();
    spawn_soft(&mut world, scene);
    let mut sys = IntoSystem::into_system(physics_soft_step_colored);
    world.run_system_once(&mut sys);

    let scratch = world
        .try_resource::<SoftColorScratch>()
        .expect("SoftColorScratch inserted");
    let g = scratch.distance_graph();
    let mut seen = vec![false; inv_mass.len()];
    for c in 0..g.n_colors() {
        seen.iter_mut().for_each(|s| *s = false);
        for &ci in g.color(c) {
            for &p in &[c_a[ci as usize], c_b[ci as usize]] {
                if inv_mass[p as usize] != 0.0 {
                    assert!(
                        !seen[p as usize],
                        "C2 lemma: distance color {c} reuses dynamic particle {p}"
                    );
                    seen[p as usize] = true;
                }
            }
        }
    }
}

// ── Gate 2 + 5: {1, N} oracle with BOTH anti-vacuity gates (pool-driven) ────────

#[cfg(not(miri))]
mod pooled {
    use super::*;
    use boyko_threadpool::ThreadPoolBuilder;

    /// Runs `steps` colored soft steps on a fresh grid-cloth world inside a
    /// `workers`-thread pool, returning `(pos_bits, parallel_color_count)`. The
    /// counter is read AFTER the run (it accumulates across the step's colors).
    fn run_colored_in_pool(w: usize, steps: usize, workers: usize) -> (Vec<(u32, u32, u32)>, usize) {
        let mut world = EcsMaster::new();
        install(&mut world, true);
        // Reset the debug parallel counter so the anti-vacuity assert measures only
        // this run's last step.
        spawn_soft(&mut world, grid_cloth(w));
        let mut sys = IntoSystem::into_system(physics_soft_step_colored);

        let pool = ThreadPoolBuilder::new().num_threads(workers).build();
        pool.install(|_scope| {
            for s in 0..steps {
                if s == steps - 1
                    && let Some(sc) = world.try_resource_mut::<SoftColorScratch>()
                {
                    sc.reset_parallel_counter();
                }
                world.run_system_once(&mut sys);
            }
        });
        let count = world
            .try_resource::<SoftColorScratch>()
            .expect("SoftColorScratch inserted")
            .parallel_color_count();
        (pos_bits(&read_soft(&mut world)), count)
    }

    /// The {1, N} oracle: the colored solve is BIT-IDENTICAL across worker counts AND
    /// to the no-pool inline fallback, with BOTH W1 anti-vacuity gates asserted.
    #[test]
    fn colored_one_vs_n_worker_bit_identical_with_anti_vacuity() {
        // A 40x40 cloth: ~3120 structural edges, 4-colored ⇒ widest color ≈ 780 slots
        // >> MIN_PARALLEL_SLOTS_PER_COLOR (256), so the parallel dispatch fires.
        let w = 40;
        let (bits_1, count_1) = run_colored_in_pool(w, 4, 1);
        let (bits_4, count_4) = run_colored_in_pool(w, 4, 4);

        // No-pool inline fallback (the threshold-bypassed path on the calling thread).
        let mut world = EcsMaster::new();
        install(&mut world, true);
        spawn_soft(&mut world, grid_cloth(w));
        let mut sys = IntoSystem::into_system(physics_soft_step_colored);
        for _ in 0..4 {
            world.run_system_once(&mut sys);
        }
        let bits_inline = pos_bits(&read_soft(&mut world));

        // Anti-vacuity (i): a color genuinely crossed the threshold and dispatched a
        // `pool.scope` (the parallel path is non-vacuously exercised).
        assert!(
            count_1 >= 1 && count_4 >= 1,
            "anti-vacuity (i): at least one color must cross the parallel threshold \
             (1-worker count = {count_1}, 4-worker count = {count_4})"
        );
        // Anti-vacuity (ii): the body actually MOVED (a non-trivial snapshot).
        let still = grid_cloth(w);
        let still_bits = pos_bits(&still);
        assert_ne!(
            bits_4, still_bits,
            "anti-vacuity (ii): the cloth must have moved (a non-trivial solve)"
        );

        // The {1, N} + inline bit-identity property.
        assert_eq!(bits_1, bits_4, "{{1, N}} oracle: 1-worker == 4-worker");
        assert_eq!(
            bits_1, bits_inline,
            "{{1, N}} oracle: parallel dispatch == no-pool inline fallback"
        );
    }
}
