//! Physics O11 SP4 — the W2 empirical A/B: SERIAL soft step vs COLORED-PARALLEL soft
//! step on a hanging cloth at n ∈ {1k, 10k, 50k} particles.
//!
//! Three arms per size:
//!   * **serial** — `physics_soft_step_colored` with `soft_body_colored = false` (runs
//!     the serial `step_body`), driven by `run_system_once` (no pool).
//!   * **colored-inline** — `soft_body_colored = true`, driven by `run_system_once`
//!     with NO pool attached, so every color falls back to the inline solve. This is
//!     the sub-threshold / no-scheduler cost; it must be ≈ serial (no regression from
//!     coloring + the inline CSR walk).
//!   * **colored-N** — `soft_body_colored = true`, driven inside an N-worker
//!     `pool.install` so colors above `MIN_PARALLEL_SLOTS_PER_COLOR` fan across the
//!     pool. The wall-clock win over serial appears at the larger sizes.
//!
//! Observation bench (NOT a pass/fail gate): the win is size- and machine-dependent.
//! Reports per-step time; derive the empirical serial fraction from the colored-N /
//! serial ratio if desired.
//!
//! ```text
//! cargo bench -p boyko-physics --bench soft_colored_sp4
//! ```

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::into_system::IntoSystem;

use boyko_physics::math::Vec3;
use boyko_physics::resources::PhysicsConfig;
use boyko_physics::sdf_query::SdfField;
use boyko_physics::soft::{SoftBody, SoftColorScratch, physics_soft_step_colored};

use boyko_threadpool::ThreadPoolBuilder;

/// A `w x h` hanging cloth (top row pinned), structural right + down edges. Chosen so
/// the widest distance color (≈ edges / 4 on a regular grid) crosses
/// `MIN_PARALLEL_SLOTS_PER_COLOR` at every benched size.
fn grid_cloth(w: usize, h: usize) -> SoftBody {
    let mut positions = Vec::with_capacity(w * h);
    let mut inv_masses = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            positions.push([x as f32 * 0.05, 4.0 - y as f32 * 0.05, 0.0]);
            inv_masses.push(if y == 0 { 0.0 } else { 1.0 });
        }
    }
    let idx = |x: usize, y: usize| (y * w + x) as u32;
    let mut edges = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if x + 1 < w {
                edges.push((idx(x, y), idx(x + 1, y)));
            }
            if y + 1 < h {
                edges.push((idx(x, y), idx(x, y + 1)));
            }
        }
    }
    SoftBody::from_mesh(&positions, &inv_masses, &edges, None, 1.0e-7, 0.0)
        .expect("grid cloth is well-formed")
}

/// `(w, h)` cloth dims for a target particle count (a near-square grid).
fn dims_for(n: usize) -> (usize, usize) {
    let w = (n as f64).sqrt().round() as usize;
    let h = n.div_ceil(w.max(1));
    (w.max(2), h.max(2))
}

fn build_world(n: usize, colored: bool) -> EcsMaster {
    let (w, h) = dims_for(n);
    let mut world = EcsMaster::new();
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
    let arch = world.create_archetype(&[SoftBody::component_id()]);
    world
        .spawn_one(arch, grid_cloth(w, h))
        .expect("{SoftBody} archetype accepts a SoftBody");
    world
}

fn bench_serial_vs_colored(c: &mut Criterion) {
    let mut group = c.benchmark_group("soft_colored_sp4");
    group.sample_size(20);

    for &n in &[1_000usize, 10_000, 50_000] {
        // ── serial ──
        group.bench_with_input(BenchmarkId::new("serial", n), &n, |b, &n| {
            let mut world = build_world(n, false);
            let mut sys = IntoSystem::into_system(physics_soft_step_colored);
            for _ in 0..10 {
                world.run_system_once(&mut sys);
            }
            b.iter(|| {
                world.run_system_once(black_box(&mut sys));
            });
        });

        // ── colored, inline (no pool) — the no-regression arm ──
        group.bench_with_input(BenchmarkId::new("colored_inline", n), &n, |b, &n| {
            let mut world = build_world(n, true);
            let mut sys = IntoSystem::into_system(physics_soft_step_colored);
            for _ in 0..10 {
                world.run_system_once(&mut sys);
            }
            b.iter(|| {
                world.run_system_once(black_box(&mut sys));
            });
        });

        // ── colored, parallel {2, 4, 8} — the wall-clock-win arm ──
        for &workers in &[2usize, 4, 8] {
            group.bench_with_input(
                BenchmarkId::new(format!("colored_w{workers}"), n),
                &n,
                |b, &n| {
                    let mut world = build_world(n, true);
                    let mut sys = IntoSystem::into_system(physics_soft_step_colored);
                    let pool = ThreadPoolBuilder::new().num_threads(workers).build();
                    pool.install(|_scope| {
                        for _ in 0..10 {
                            world.run_system_once(&mut sys);
                        }
                        b.iter(|| {
                            world.run_system_once(black_box(&mut sys));
                        });
                    });
                },
            );
        }
    }
    group.finish();
}

criterion_group!(benches, bench_serial_vs_colored);
criterion_main!(benches);
