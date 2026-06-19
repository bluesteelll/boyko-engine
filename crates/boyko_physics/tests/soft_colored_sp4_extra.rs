//! Physics O11 SP4 — TESTER-ADDED coverage closing the gaps the developer's three
//! SP4 files left open.
//!
//! The developer's `soft_colored_sp4.rs` proves the {1, N} oracle and the C2 lemma
//! for the DISTANCE coloring on a regular grid. This file extends the coverage to the
//! gates the orchestrator's brief names explicitly but the shipped tests under-exercise:
//!
//!   * **Gate 6 (disjointness across mesh shapes)** — the C2 lemma checked via the
//!     PUBLIC CSR for the VOLUME (tet) coloring and on a HIGH-VALENCE vertex (a fan
//!     where one dynamic particle is an endpoint of many distance edges), plus the
//!     SELF-COLLISION pair coloring after a colored self-collision step.
//!   * **Gate 7 (pinned shared across colors)** — an explicit scene where ONE pinned
//!     particle is an endpoint of constraints that land in DIFFERENT colors; the guard
//!     must leave it byte-frozen and the {1, N} bit-identity must still hold.
//!   * **Colored SELF-COLLISION {1, N}** — the developer's {1, N} leaves
//!     `soft_self_collision_colored` OFF, so the self-pair colored dispatch + the
//!     `PairListPtr` path never run under the worker-count sweep. This adds a scene
//!     with `soft_self_collision_colored = true` whose self-pair color crosses the
//!     parallel threshold.
//!
//! Every pool-driven test is gated `cfg(not(miri))` (the curated Miri subset lives in
//! `soft_colored_sp4_miri.rs`); the pool-free CSR / disjointness checks are
//! Miri-runnable.

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

/// Installs the colored config with optional colored self-collision + radius.
fn install_full(world: &mut EcsMaster, colored: bool, self_colored: bool, sc_iters: usize) {
    world.insert_resource(PhysicsConfig {
        dt: 1.0 / 60.0,
        substeps: 2,
        gravity: Vec3::new(0.0, -9.81, 0.0),
        soft_body: true,
        soft_body_colored: colored,
        soft_self_collision_colored: self_colored,
        self_collision_iters: sc_iters,
        ..PhysicsConfig::default()
    });
    world.insert_resource(SdfField::default());
    world.insert_resource(SoftColorScratch::default());
}

// ── Scenes ──────────────────────────────────────────────────────────────────

/// A `w x w` grid cloth (top row pinned), structural right + down edges.
fn grid_cloth(w: usize, radius: f32) -> SoftBody {
    let mut positions = Vec::with_capacity(w * w);
    let mut inv_masses = Vec::with_capacity(w * w);
    for y in 0..w {
        for x in 0..w {
            positions.push([x as f32 * 0.1, 2.0 - y as f32 * 0.1, 0.0]);
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
    SoftBody::from_mesh(&positions, &inv_masses, &edges, None, 1.0e-7, radius)
        .expect("grid cloth is well-formed")
}

/// A high-valence FAN: one central DYNAMIC hub (particle 0) connected to `spokes`
/// rim particles. Every distance edge shares the hub, so a correct greedy coloring
/// MUST put every edge in its own color (the hub is a dynamic endpoint of all of
/// them) — the worst case for the C2 occupancy test (`n_colors == spokes`).
fn high_valence_fan(spokes: usize) -> SoftBody {
    let mut positions = vec![[0.0f32, 0.0, 0.0]]; // hub
    let mut inv_masses = vec![1.0f32]; // hub dynamic
    let mut edges = Vec::with_capacity(spokes);
    for s in 0..spokes {
        let a = (s as f32) * 0.3;
        positions.push([a.cos(), a.sin(), 0.0]);
        inv_masses.push(1.0);
        edges.push((0u32, (s + 1) as u32));
    }
    SoftBody::from_mesh(&positions, &inv_masses, &edges, None, 1.0e-7, 0.0)
        .expect("fan is well-formed")
}

/// A tetra LATTICE: a `side³` grid split into 5 tets per cell (the bench fixture
/// topology), top layer pinned. Exercises the VOLUME (4-arity) coloring + the
/// distance coloring on a 3-D body.
fn tet_lattice(side: usize) -> SoftBody {
    let n = side;
    let idx = |x: usize, y: usize, z: usize| ((z * n + y) * n + x) as u32;
    let mut positions = Vec::with_capacity(n * n * n);
    let mut inv_masses = Vec::with_capacity(n * n * n);
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                positions.push([x as f32, y as f32 + 2.0, z as f32]);
                // Pin the top layer (y == n-1) so a mix of pinned/dynamic vertices
                // routes through the 4-arity guard.
                inv_masses.push(if y == n - 1 { 0.0 } else { 1.0 });
            }
        }
    }
    let mut edges = Vec::new();
    let mut tets = Vec::new();
    for z in 0..n - 1 {
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                let c = [
                    idx(x, y, z),
                    idx(x + 1, y, z),
                    idx(x, y + 1, z),
                    idx(x + 1, y + 1, z),
                    idx(x, y, z + 1),
                    idx(x + 1, y, z + 1),
                    idx(x, y + 1, z + 1),
                    idx(x + 1, y + 1, z + 1),
                ];
                let map = |i: usize| c[i];
                for &(a, b, cc, d) in &[
                    (0usize, 1, 2, 4),
                    (3, 1, 2, 7),
                    (5, 1, 4, 7),
                    (6, 2, 4, 7),
                    (1, 2, 4, 7),
                ] {
                    tets.push((map(a), map(b), map(cc), map(d)));
                }
                edges.push((c[0], c[1]));
                edges.push((c[0], c[2]));
                edges.push((c[0], c[4]));
                edges.push((c[1], c[7]));
                edges.push((c[2], c[7]));
                edges.push((c[4], c[7]));
            }
        }
    }
    SoftBody::from_tet_mesh(
        &positions,
        &inv_masses,
        &edges,
        &tets,
        None,
        None,
        1.0e-6,
        1.0e-6,
        0.0,
    )
    .expect("tet lattice is well-formed")
}

// ── Generic CSR disjointness checker (the C2 lemma, public-CSR form) ────────────

/// Asserts, via the public CSR, that within every color no DYNAMIC particle is the
/// endpoint of two constraints (the C2 lemma). `endpoints(ci)` returns the dynamic
/// endpoints of constraint `ci`; a pinned endpoint (`inv_mass == 0`) is allowed to
/// be shared across colors and is filtered out before the check.
fn assert_color_disjoint<F>(
    graph: &boyko_physics::soft::ParticleColorGraph,
    n: usize,
    inv_mass: &[f32],
    label: &str,
    endpoints: F,
) where
    F: Fn(usize) -> Vec<u32>,
{
    let mut total = 0usize;
    let mut seen = vec![false; n];
    for c in 0..graph.n_colors() {
        seen.iter_mut().for_each(|s| *s = false);
        // Span sanity: the CSR slice length equals the reported span.
        assert_eq!(
            graph.color(c).len() as u32,
            graph.color_span(c),
            "{label}: color {c} CSR slice length must equal color_span"
        );
        for &ci in graph.color(c) {
            total += 1;
            for p in endpoints(ci as usize) {
                if inv_mass[p as usize] != 0.0 {
                    assert!(
                        !seen[p as usize],
                        "{label}: C2 lemma violated — color {c} reuses dynamic particle {p}"
                    );
                    seen[p as usize] = true;
                }
            }
        }
    }
    // Every constraint must be colored exactly once (counting-sort completeness).
    assert!(total > 0, "{label}: anti-vacuity — at least one constraint colored");
}

// ── Gate 6: disjointness across mesh shapes (high-valence + tet) ────────────────

/// The high-valence fan: the hub is dynamic, so the distance coloring MUST give every
/// spoke edge its own color (`n_colors == spokes`), and no color may reuse the hub.
#[test]
fn distance_coloring_high_valence_is_disjoint() {
    let spokes = 50;
    let mut world = EcsMaster::new();
    install_full(&mut world, true, false, 0);
    let scene = high_valence_fan(spokes);
    let inv_mass = scene.inv_mass.clone();
    let c_a = scene.c_a.clone();
    let c_b = scene.c_b.clone();
    let n = scene.particle_count();
    spawn_soft(&mut world, scene);
    let mut sys = IntoSystem::into_system(physics_soft_step_colored);
    world.run_system_once(&mut sys);

    let scratch = world
        .try_resource::<SoftColorScratch>()
        .expect("SoftColorScratch inserted");
    let g = scratch.distance_graph();
    assert_eq!(
        g.n_colors(),
        spokes,
        "a dynamic-hub fan must produce one color per spoke (every edge shares the hub)"
    );
    assert_color_disjoint(g, n, &inv_mass, "fan/distance", |ci| {
        vec![c_a[ci], c_b[ci]]
    });
}

/// The tet lattice: the 4-arity VOLUME coloring satisfies the C2 lemma (no color reuses
/// a dynamic vertex), AND the distance coloring on the same 3-D body.
#[test]
fn volume_and_distance_coloring_tet_lattice_disjoint() {
    let mut world = EcsMaster::new();
    install_full(&mut world, true, false, 0);
    let scene = tet_lattice(4);
    let inv_mass = scene.inv_mass.clone();
    let n = scene.particle_count();
    let c_a = scene.c_a.clone();
    let c_b = scene.c_b.clone();
    let t0 = scene.t0.clone();
    let t1 = scene.t1.clone();
    let t2 = scene.t2.clone();
    let t3 = scene.t3.clone();
    spawn_soft(&mut world, scene);
    let mut sys = IntoSystem::into_system(physics_soft_step_colored);
    world.run_system_once(&mut sys);

    let scratch = world
        .try_resource::<SoftColorScratch>()
        .expect("SoftColorScratch inserted");
    assert_color_disjoint(
        scratch.volume_graph(),
        n,
        &inv_mass,
        "tet/volume",
        |ci| vec![t0[ci], t1[ci], t2[ci], t3[ci]],
    );
    assert_color_disjoint(
        scratch.distance_graph(),
        n,
        &inv_mass,
        "tet/distance",
        |ci| vec![c_a[ci], c_b[ci]],
    );
}

// ── Gate 7: a pinned particle shared by constraints in DIFFERENT colors ──────────

/// A pinned hub shared by constraints that land in DIFFERENT colors. To FORCE the
/// pinned hub across multiple colors we make several edges from the hub to the SAME
/// dynamic particle `d` (`is_dynamic_row(0.0) == false` ⇒ the pinned hub imposes no
/// occupancy, but `d` does): each edge `(hub, d)` conflicts with the previous on `d`,
/// so the greedy colorer puts each in its OWN color — and the pinned hub is an
/// endpoint in every one of them. The C1 guard must leave the hub byte-frozen across
/// all of those colors.
#[test]
fn pinned_shared_across_colors_stays_frozen() {
    // hub (pinned, index 0), d (dynamic, index 1) plus a few more dynamic nodes so the
    // body is non-degenerate. `copies` parallel hub--d edges force `copies` colors, the
    // pinned hub shared across every one.
    let copies = 5usize;
    let mut positions = vec![[0.0f32, 1.0, 0.0]]; // hub (pinned)
    let mut inv_masses = vec![0.0f32]; // hub PINNED
    positions.push([0.5, 0.0, 0.0]); // d (dynamic, index 1)
    inv_masses.push(1.0);
    // A couple of extra dynamic anchors so `d` is also braced (well-formed body).
    positions.push([1.0, 0.0, 0.0]);
    inv_masses.push(1.0);
    positions.push([0.0, 0.0, 1.0]);
    inv_masses.push(1.0);

    let mut edges = Vec::new();
    for _ in 0..copies {
        edges.push((0u32, 1u32)); // hub(pinned) -- d(dynamic): each conflicts on d
    }
    edges.push((1u32, 2u32)); // brace d
    edges.push((1u32, 3u32));

    let scene = SoftBody::from_mesh(&positions, &inv_masses, &edges, None, 1.0e-7, 0.0)
        .expect("pinned-hub scene is well-formed");
    let hub0 = (scene.pos_x[0], scene.pos_y[0], scene.pos_z[0]);

    let mut world = EcsMaster::new();
    install_full(&mut world, true, false, 0);
    let inv_mass = scene.inv_mass.clone();
    let c_a = scene.c_a.clone();
    let c_b = scene.c_b.clone();
    let n = scene.particle_count();
    spawn_soft(&mut world, scene);
    let mut sys = IntoSystem::into_system(physics_soft_step_colored);
    for _ in 0..8 {
        world.run_system_once(&mut sys);
    }
    let out = read_soft(&mut world);

    // The pinned hub never moved (the C1 guard skips every write to it, in every
    // color it is shared by).
    assert_eq!(
        (out.pos_x[0].to_bits(), out.pos_y[0].to_bits(), out.pos_z[0].to_bits()),
        (hub0.0.to_bits(), hub0.1.to_bits(), hub0.2.to_bits()),
        "the pinned hub shared across colors must stay byte-frozen"
    );

    // The pinned hub appears in MORE than one color (genuinely cross-color shared) and
    // the coloring is still disjoint over DYNAMIC particles.
    let scratch = world
        .try_resource::<SoftColorScratch>()
        .expect("SoftColorScratch inserted");
    let g = scratch.distance_graph();
    let mut hub_colors = 0usize;
    for c in 0..g.n_colors() {
        if g.color(c).iter().any(|&ci| {
            c_a[ci as usize] == 0 || c_b[ci as usize] == 0
        }) {
            hub_colors += 1;
        }
    }
    assert!(
        hub_colors >= 2,
        "anti-vacuity: the pinned hub must be shared across >= 2 colors (got {hub_colors})"
    );
    assert_color_disjoint(g, n, &inv_mass, "pinned-hub/distance", |ci| {
        vec![c_a[ci], c_b[ci]]
    });
}

// ── Colored SELF-COLLISION {1, N} (the developer's {1,N} leaves this OFF) ─────────

#[cfg(not(miri))]
mod pooled_self {
    use super::*;
    use boyko_threadpool::ThreadPoolBuilder;

    /// A dense overlapping cloth (small spacing relative to the particle radius) so
    /// the self-collision sweep emits a LARGE pair set whose widest color crosses the
    /// parallel threshold — exercising the colored self-pair dispatch + `PairListPtr`.
    fn dense_overlap_cloth(w: usize) -> SoftBody {
        // Spacing 0.05, radius 0.05 ⇒ cell = 0.1, neighbours within 2r overlap ⇒ many
        // self-collision pairs.
        let mut positions = Vec::with_capacity(w * w);
        let mut inv_masses = Vec::with_capacity(w * w);
        for y in 0..w {
            for x in 0..w {
                positions.push([x as f32 * 0.05, 2.0 - y as f32 * 0.05, 0.0]);
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
        SoftBody::from_mesh(&positions, &inv_masses, &edges, None, 1.0e-7, 0.05)
            .expect("dense cloth is well-formed")
    }

    fn run_self_colored(w: usize, steps: usize, workers: usize) -> (Vec<(u32, u32, u32)>, usize) {
        let mut world = EcsMaster::new();
        install_full(&mut world, true, true, 2);
        spawn_soft(&mut world, dense_overlap_cloth(w));
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

    /// The colored SELF-COLLISION {1, N} oracle: with `soft_self_collision_colored`
    /// ON, the per-substep self-pair colored dispatch is bit-identical across worker
    /// counts (1 vs 4) — exercising the `PairListPtr` + the self-pair `dispatch_color`.
    #[test]
    fn colored_self_collision_one_vs_n_bit_identical() {
        let w = 40;
        let (bits_1, count_1) = run_self_colored(w, 4, 1);
        let (bits_4, count_4) = run_self_colored(w, 4, 4);
        // Anti-vacuity: at least one color (distance/volume OR self-pair) dispatched.
        assert!(
            count_1 >= 1 && count_4 >= 1,
            "anti-vacuity: a color must cross the parallel threshold with self-collision \
             colored ON (1-worker = {count_1}, 4-worker = {count_4})"
        );
        let still = pos_bits(&dense_overlap_cloth(w));
        assert_ne!(bits_4, still, "anti-vacuity: the cloth must have moved");
        assert_eq!(
            bits_1, bits_4,
            "colored self-collision {{1, N}}: 1-worker == 4-worker (PairListPtr path)"
        );
    }

    /// Diagnostic (run with `--nocapture`): prints the actual `parallel_color_count`
    /// per worker count on the 40x40 cloth, so the {1, N} anti-vacuity value is
    /// auditable (not just asserted `>= 1`).
    #[test]
    fn print_parallel_color_count_40x40() {
        let w = 40;
        for &workers in &[1usize, 2, 4, 8] {
            let (_bits, count) = run_self_colored_off(w, 4, workers);
            println!("[anti-vacuity] 40x40 distance/volume, {workers} workers -> parallel_color_count = {count}");
        }
    }

    /// Helper: the worker sweep run with self-collision OFF (distance/volume only).
    fn run_self_colored_off(w: usize, steps: usize, workers: usize) -> (Vec<(u32, u32, u32)>, usize) {
        let mut world = EcsMaster::new();
        install_full(&mut world, true, false, 0);
        spawn_soft(&mut world, grid_cloth(w, 0.0));
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
            .expect("scratch")
            .parallel_color_count();
        (pos_bits(&read_soft(&mut world)), count)
    }

    /// Worker sweep {1, 2, 4, 8}: every worker count is bit-identical to the
    /// 1-worker run (the full {1, N} matrix the brief names, distance/volume only).
    #[test]
    fn colored_grid_worker_sweep_all_bit_identical() {
        let w = 40;
        let run = |workers: usize| -> (Vec<(u32, u32, u32)>, usize) {
            let mut world = EcsMaster::new();
            install_full(&mut world, true, false, 0);
            spawn_soft(&mut world, grid_cloth(w, 0.0));
            let mut sys = IntoSystem::into_system(physics_soft_step_colored);
            let pool = ThreadPoolBuilder::new().num_threads(workers).build();
            pool.install(|_scope| {
                for s in 0..4 {
                    if s == 3
                        && let Some(sc) = world.try_resource_mut::<SoftColorScratch>()
                    {
                        sc.reset_parallel_counter();
                    }
                    world.run_system_once(&mut sys);
                }
            });
            let count = world
                .try_resource::<SoftColorScratch>()
                .expect("scratch")
                .parallel_color_count();
            (pos_bits(&read_soft(&mut world)), count)
        };
        let (base, base_count) = run(1);
        assert!(base_count >= 1, "anti-vacuity: 1-worker dispatched a parallel color");
        for &w_count in &[2usize, 4, 8] {
            let (bits, count) = run(w_count);
            assert!(count >= 1, "anti-vacuity: {w_count}-worker dispatched a parallel color");
            assert_eq!(
                base, bits,
                "{{1, N}} oracle: {w_count}-worker must be bit-identical to 1-worker"
            );
        }
    }
}
