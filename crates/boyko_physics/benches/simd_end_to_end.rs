//! O1 END-TO-END: what the `simd` switch is worth across a WHOLE `SoftStepSolver`
//! step, on the ~10k-body pyramid.
//!
//! # The gap this closes
//!
//! `docs/MEASUREMENT-QUEUE.md` §4 states it outright: *"No bench prices `simd`
//! end-to-end through `SoftStepSolver`, which is the one number the decision to flip
//! it would most benefit from. Building it means an A/B of `SoftStepSolver::step` at
//! `simd ∈ {false, true}` over the pyramid fixture."* Every figure quoted for `simd`
//! to date is a KERNEL ratio — `benches/simd_o1.rs` times `refresh_inertia` and
//! `apply_gravity` in isolation, which answers "how much faster is the widened
//! loop", not "how much faster is the step". This bench answers the second question
//! and ONLY the second question.
//!
//! **This number must not be reconciled against the 3.8× that circulates.** That
//! figure is a kernel ratio from a transcription built outside this repository at a
//! non-shipped profile; it measures a different quantity. Amdahl bounds the
//! whole-step ratio at whatever fraction of the step the widened passes occupy, and
//! the `o1_widened_fraction` group below MEASURES that fraction rather than
//! asserting it.
//!
//! # Shape: RUNTIME-SWITCHED, both arms in ONE binary
//!
//! `simd` is a [`PhysicsConfig`] FIELD, not a `cfg`. `SoftStepSolver::solve` reads
//! it per call (`soft_step.rs`, `let use_simd = config.simd;`) and forwards the bool
//! to the two dispatchers it gates. So both arms are reachable in one process from
//! two configs that differ in exactly one field — no rebuild between arms, and
//! therefore no between-build drift. `simd_o1.rs` takes the same shape; this bench
//! copies it deliberately.
//!
//! # What `simd` actually gates in `SoftStepSolver::solve`
//!
//! Exactly two per-substep passes, each run ONCE per substep:
//!   * `simd::apply_gravity(bodies_eff, snapshot, gravity, h, use_simd)` — step (1)
//!   * `Self::refresh_inertia(bodies_eff, snapshot, use_simd)` — step (5) tail
//!
//! and NOT `simd::position_integrate`, which the solver calls with a hard-coded
//! `false` on a MEASURED decision (the SoA kernel regresses ~1.6× on this AoS
//! `BodyState` layout). Everything else in the step — `build_bodies`,
//! `build_constraints`, `warm_start_apply`, `solve_velocities` × `substeps ×
//! (1 + relax_iterations)`, `apply_restitution`, `store_and_swap`, `write_back` —
//! is scalar in both arms. That surround is the Amdahl denominator.
//!
//! # Groups
//!
//! * `o1_step_simd_ab` — the headline: one whole `SoftStepSolver::solve` on the
//!   pyramid at `simd = false` vs `simd = true`. THE ratio to report.
//! * `o1_widened_fraction` — the two gated passes alone, at the pyramid's exact body
//!   count, in BOTH arms. Their scalar times × `substeps` over the scalar whole-step
//!   time give the widened fraction of the step; their own scalar/simd ratio is the
//!   in-binary proof that the two arms are NOT the same code path (see below).
//!
//! # Anti-vacuity
//!
//! A runtime switch that silently does nothing produces a ~1.00× ratio that looks
//! like a measurement. Three guards stand against that reading:
//!   1. A compile-time check that this binary CONTAINS an AVX2 arm. Without
//!      `target_feature = "avx2"` (which `-C target-cpu=x86-64-v3` from
//!      `.cargo/config.toml` supplies) both dispatchers `cfg` away to the scalar
//!      body and the flag is a no-op — the bench PANICS rather than reporting 1.00×.
//!   2. The scene is asserted non-degenerate: > 0 contacts, > 0 dynamic bodies, and
//!      a step that actually moves a body.
//!   3. `o1_widened_fraction` times the gated kernels in the SAME binary and SAME
//!      run, so a whole-step ratio near unity can be read against a kernel ratio
//!      that is not: "the arms differ, the step is just dominated by the surround"
//!      is then a MEASURED statement rather than an excuse.
//!
//! # The arms are bit-identical, which is why this is a pure timing A/B
//!
//! The O1 kernels are gated bit-identical to their scalar oracles (that identity is
//! what allowed `simd` to default to `true` on 2026-09-03). So the two arms traverse
//! the IDENTICAL sequence of body states across criterion's iterations — the step
//! mutates the snapshot, but it mutates it the same way in both arms. `assert_arms_
//! agree_bitwise` below checks that on this fixture before any timing, so the A/B is
//! not comparing two different trajectories.
//!
//! # Running
//!
//! ```text
//! cargo bench -p boyko-physics --bench simd_end_to_end
//! ```
//!
//! ⚠ Pass NO `RUSTFLAGS`. It REPLACES `target.<triple>.rustflags` rather than
//! appending, which would drop the `-C target-cpu=x86-64-v3` baseline and DELETE the
//! AVX2 kernels — i.e. delete the thing being measured, while the run still produces
//! a number. Guard 1 above turns that mistake into a panic instead of a result.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

use boyko_physics::components::{Collider, ColliderShape, RigidBody, RigidBodyMass};
use boyko_physics::manifold::{BodyIndex, ContactPoint, Manifold};
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::{BodyState, PhysicsConfig, SolverScratch};
use boyko_physics::solver::contact::BodyEffective;
use boyko_physics::solver::simd;
use boyko_physics::solver::{RigidSolver, SoftStepSolver};

/// Pyramid base width. `141 · 142 / 2 = 10011` dynamic bodies — the plan's ~10k
/// pyramid, the same fixture `parallel_solve.rs` and `ke16_solve_in_system.rs` use,
/// so this bench's scene is comparable with theirs.
const PYRAMID_BASE: u32 = 141;

/// Number of warm steps before timing, so the warm-start impulse cache is at steady
/// state and the first timed iteration is not measuring a cold cache.
const WARM_STEPS: usize = 4;

// ─────────────────────────────────────────────────────────────────────────────
// Guard 1: this binary must CONTAIN an AVX2 arm, or the A/B is vacuous.
// ─────────────────────────────────────────────────────────────────────────────

/// `true` when the O1 dispatchers compiled their AVX2 arm into this binary.
///
/// The dispatchers in `solver/simd.rs` are gated
/// `#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]`. When that cfg is
/// off there is no SIMD body to reach and the `simd` bool is inert, so BOTH arms of
/// this bench run identical scalar code and the ratio is 1.00× by construction.
const HAS_AVX2_ARM: bool = cfg!(all(target_arch = "x86_64", target_feature = "avx2"));

/// Panics unless this binary can actually take the SIMD arm.
// The constant operand is the POINT, not an oversight: this guard exists to turn a
// silently-inert switch into a panic instead of a plausible-looking 1.00x ratio, and
// the thing it must catch — a build whose `target_feature = "avx2"` was lost — is a
// compile-time fact. `clippy::assertions_on_constants` assumes a constant assert is
// dead weight; here it is the anti-vacuity gate.
#[allow(clippy::assertions_on_constants)]
fn assert_avx2_arm_present() {
    assert!(
        HAS_AVX2_ARM,
        "VACUOUS: this binary has NO AVX2 arm (target_feature = \"avx2\" is off), so \
         `simd = true` and `simd = false` run the SAME scalar code and any ratio this \
         bench prints is 1.00x by construction, not by measurement. The baseline comes \
         from `-C target-cpu=x86-64-v3` in .cargo/config.toml; the usual cause of losing \
         it is passing RUSTFLAGS, which REPLACES target.<triple>.rustflags instead of \
         appending. Re-run with no RUSTFLAGS set."
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Fixture — the pyramid, lifted verbatim in shape from parallel_solve.rs
// ─────────────────────────────────────────────────────────────────────────────

fn dyn_sphere(position: Vec3) -> BodyState {
    let body = RigidBody {
        position,
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,
        inv_mass: 1.0,
        restitution: 0.0,
        friction: 0.5,
    };
    let collider = Collider {
        shape: ColliderShape::Sphere { radius: 0.5 },
        layer: 1,
        mask: 1,
    };
    BodyState::from_columns(&body, &mass, &collider, false, true, false)
}

fn static_floor() -> BodyState {
    let body = RigidBody {
        position: Vec3::new(0.0, -50.0, 0.0),
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: Mat3::ZERO,
        inv_mass: 0.0,
        restitution: 0.0,
        friction: 0.5,
    };
    let collider = Collider {
        shape: ColliderShape::Sphere { radius: 50.0 },
        layer: 1,
        mask: 1,
    };
    BodyState::from_columns(&body, &mass, &collider, false, false, false)
}

fn manifold(a: u32, b: u32, normal: Vec3, anchor: Vec3) -> Manifold {
    let mut m = Manifold::new(BodyIndex(a), BodyIndex(b));
    m.normal = normal;
    m.points[0] = ContactPoint {
        anchor_a: anchor,
        anchor_b: anchor,
        separation: -0.05,
        feature_id: 0,
    };
    m.count = 1;
    m
}

/// A big single-ISLAND pyramid: `base` rows wide at the bottom, narrowing by one per
/// level. Every sphere rests on the two below it plus a lateral neighbour, and the
/// bottom row rests on the shared static floor.
fn pyramid_scene(base: u32) -> (Vec<BodyState>, Vec<Manifold>) {
    let mut row_first = Vec::with_capacity(base as usize + 1);
    let mut acc = 0u32;
    for r in 0..base {
        row_first.push(acc);
        acc += base - r;
    }
    row_first.push(acc);
    let n_dyn = acc;

    let mut bodies: Vec<BodyState> = Vec::with_capacity(n_dyn as usize + 1);
    for r in 0..base {
        let count = base - r;
        let y = 0.5 + r as f32 * 0.86;
        for i in 0..count {
            let x = (i as f32 - (count as f32 - 1.0) * 0.5) * 1.02 + (r as f32 * 0.5);
            bodies.push(dyn_sphere(Vec3::new(x, y, 0.0)));
        }
    }
    let floor_row = n_dyn;
    bodies.push(static_floor());

    let id = |r: u32, i: u32| row_first[r as usize] + i;
    let mut manifolds = Vec::new();
    for r in 0..base {
        let count = base - r;
        for i in 0..count {
            let me = id(r, i);
            if r == 0 {
                manifolds.push(manifold(me, floor_row, Vec3::new(0.0, -1.0, 0.0), bodies[me as usize].position));
            }
            if i + 1 < count {
                manifolds.push(manifold(me, id(r, i + 1), Vec3::new(1.0, 0.0, 0.0), bodies[me as usize].position));
            }
            if r + 1 < base {
                let above_count = base - (r + 1);
                if i < above_count {
                    manifolds.push(manifold(me, id(r + 1, i), Vec3::new(0.0, 1.0, 0.0), bodies[me as usize].position));
                }
                if i >= 1 && (i - 1) < above_count {
                    manifolds.push(manifold(me, id(r + 1, i - 1), Vec3::new(0.0, 1.0, 0.0), bodies[me as usize].position));
                }
            }
        }
    }
    (bodies, manifolds)
}

/// The production config, with `simd` as the single varied field.
fn config(simd: bool) -> PhysicsConfig {
    PhysicsConfig {
        dt: 1.0 / 60.0,
        simd,
        ..PhysicsConfig::default()
    }
}

/// Runs `steps` whole solver steps from a fresh copy of `bodies` and returns the
/// final snapshot.
fn run_steps(cfg: &PhysicsConfig, bodies: &[BodyState], manifolds: &[Manifold], steps: usize) -> Vec<BodyState> {
    let mut solver = SoftStepSolver::default();
    let mut scratch = SolverScratch::with_capacity(bodies.len());
    scratch.set_bodies(bodies);
    for _ in 0..steps {
        scratch.touched.reset(scratch.bodies().len());
        solver.solve(cfg, manifolds, &mut scratch);
    }
    scratch.bodies().to_vec()
}

// ─────────────────────────────────────────────────────────────────────────────
// Guards 2 and 3 — the scene does work, and the arms agree bitwise
// ─────────────────────────────────────────────────────────────────────────────

/// Asserts the fixture is non-degenerate AND that a step actually moves a body, so
/// the timed region is not an early-return.
///
/// `SoftStepSolver::solve` has exactly one legitimate early-return (`!has_dynamic`),
/// and a scene that hit it would time a few microseconds in both arms and report a
/// clean 1.00×. This rules that out by observation rather than by reading the code.
fn assert_scene_does_work(bodies: &[BodyState], manifolds: &[Manifold]) {
    assert!(!manifolds.is_empty(), "pyramid must have > 0 contacts");
    let n_dyn = bodies.iter().filter(|b| b.simulated && b.inv_mass != 0.0).count();
    assert!(n_dyn > 0, "pyramid must have > 0 dynamic bodies (got {n_dyn})");

    let before = bodies[0].position;
    let after = run_steps(&config(false), bodies, manifolds, 1)[0].position;
    assert!(
        (after.x - before.x).abs() + (after.y - before.y).abs() + (after.z - before.z).abs() > 0.0,
        "a single step must MOVE body 0 — an unmoved body means the timed region \
         early-returned and both arms would report a vacuous 1.00x"
    );
}

/// Asserts the two arms produce BIT-IDENTICAL state after `WARM_STEPS` steps.
///
/// This is not a redundant copy of the crate's own differential proptest: it makes
/// the A/B legitimate. Criterion iterates the step repeatedly on a mutating
/// snapshot, so if the arms diverged numerically they would, after a few hundred
/// iterations, be solving DIFFERENT scenes and the ratio would be a comparison of
/// two workloads rather than of two implementations.
fn assert_arms_agree_bitwise(bodies: &[BodyState], manifolds: &[Manifold]) {
    let scalar = run_steps(&config(false), bodies, manifolds, WARM_STEPS);
    let widened = run_steps(&config(true), bodies, manifolds, WARM_STEPS);
    assert_eq!(scalar.len(), widened.len(), "arms must produce the same body count");
    for (i, (s, w)) in scalar.iter().zip(widened.iter()).enumerate() {
        assert_eq!(
            s.position.x.to_bits(),
            w.position.x.to_bits(),
            "arm divergence at body {i}: scalar x = {}, simd x = {} — the A/B would be \
             comparing two different trajectories, not two implementations",
            s.position.x,
            w.position.x
        );
        assert_eq!(
            s.linear_velocity.y.to_bits(),
            w.linear_velocity.y.to_bits(),
            "arm divergence at body {i} (linear_velocity.y)"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Group 1 — THE headline: the whole step, both arms
// ─────────────────────────────────────────────────────────────────────────────

fn bench_step_ab(c: &mut Criterion) {
    assert_avx2_arm_present();

    let (bodies, manifolds) = pyramid_scene(PYRAMID_BASE);
    assert_scene_does_work(&bodies, &manifolds);
    assert_arms_agree_bitwise(&bodies, &manifolds);

    let n_contacts = manifolds.len();
    let n_dyn = bodies.iter().filter(|b| b.simulated && b.inv_mass != 0.0).count();
    println!(
        "\n[simd_end_to_end] pyramid base={PYRAMID_BASE}: {} bodies ({n_dyn} dynamic), \
         {n_contacts} contacts, substeps={}, relax={}\n\
         [simd_end_to_end] AVX2 arm compiled in: {HAS_AVX2_ARM}\n",
        bodies.len(),
        config(true).substeps,
        config(true).relax_iterations,
    );

    let mut group = c.benchmark_group("o1_step_simd_ab");
    group.sample_size(30);
    group.throughput(Throughput::Elements(n_contacts as u64));

    for (label, simd_on) in [("scalar", false), ("simd", true)] {
        let cfg = config(simd_on);
        group.bench_with_input(BenchmarkId::new(label, n_contacts), &n_contacts, |b, &_n| {
            let mut solver = SoftStepSolver::default();
            let mut scratch = SolverScratch::with_capacity(bodies.len());
            scratch.set_bodies(&bodies);
            for _ in 0..WARM_STEPS {
                scratch.touched.reset(scratch.bodies().len());
                solver.solve(&cfg, &manifolds, &mut scratch);
            }
            b.iter(|| {
                scratch.touched.reset(scratch.bodies().len());
                solver.solve(black_box(&cfg), black_box(&manifolds), &mut scratch);
            });
        });
    }
    group.finish();
}

// ─────────────────────────────────────────────────────────────────────────────
// Group 2 — the widened fraction, at the pyramid's exact body count
// ─────────────────────────────────────────────────────────────────────────────

/// Builds the `BodyEffective` column the solver derives from a snapshot, so the two
/// gated kernels are timed on the pyramid's REAL working set (10012 bodies) rather
/// than on `simd_o1`'s synthetic 10k random scene.
fn effective_column(bodies: &[BodyState]) -> Vec<BodyEffective> {
    bodies
        .iter()
        .map(|b| BodyEffective {
            inv_mass: b.inv_mass,
            inv_inertia: b.inv_inertia,
            linear_velocity: b.linear_velocity,
            angular_velocity: b.angular_velocity,
        })
        .collect()
}

/// Times the two passes `simd` gates, alone, in both arms.
///
/// Why this group exists: the whole-step ratio is bounded by Amdahl at the fraction
/// of the step these two passes occupy, and that fraction is DERIVABLE from this
/// group rather than assumed. With `substeps = S`, the step runs each pass `S` times,
/// so
///
/// ```text
/// widened_fraction ≈ S · (gravity_scalar + refresh_inertia_scalar) / step_scalar
/// ```
///
/// and the whole-step ratio's ceiling is `1 / (1 - f · (1 - 1/kernel_ratio))`.
///
/// It is also the in-binary proof that the arms are distinct: if the whole-step ratio
/// comes back at ~1.00× the question is immediately "is the switch inert?", and a
/// kernel ratio measured in the SAME run answers it.
fn bench_widened_fraction(c: &mut Criterion) {
    assert_avx2_arm_present();

    let (bodies, _manifolds) = pyramid_scene(PYRAMID_BASE);
    let eff0 = effective_column(&bodies);
    let gravity = Vec3::new(0.0, -9.81, 0.0);
    let cfg = config(true);
    let h = cfg.dt / cfg.substeps.max(1) as f32;

    let mut group = c.benchmark_group("o1_widened_fraction");
    group.sample_size(30);
    group.throughput(Throughput::Elements(bodies.len() as u64));

    for (label, simd_on) in [("refresh_inertia_scalar", false), ("refresh_inertia_simd", true)] {
        group.bench_function(label, |b| {
            let mut eff = eff0.clone();
            b.iter(|| {
                simd::refresh_inertia(black_box(&mut eff), black_box(&bodies), simd_on);
                black_box(eff[0].inv_inertia.rows[0].x);
            });
        });
    }

    for (label, simd_on) in [("apply_gravity_scalar", false), ("apply_gravity_simd", true)] {
        group.bench_function(label, |b| {
            let mut eff = eff0.clone();
            b.iter(|| {
                simd::apply_gravity(black_box(&mut eff), black_box(&bodies), gravity, h, simd_on);
                black_box(eff[0].linear_velocity.y);
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_step_ab, bench_widened_fraction);
criterion_main!(benches);
