//! O1 A/B: the scalar vs AVX2 in-solver `refresh_inertia` (`R · I⁻¹_local · Rᵀ`)
//! and the gravity/position/quaternion integrate, on a 10k-body substep workload.
//!
//! Reports the measured speed-up (the plan targets ~1.4–3× per RESEARCH-FAST-MATH
//! normalize-batch 3.24×). The SIMD path is bit-identical to scalar (the
//! `simd_o1` differential proptest is the gate) — this bench measures ONLY the
//! speed delta, never a value change.
//!
//! Build the SIMD arm with AVX2:
//! ```text
//! RUSTFLAGS="-C target-feature=+avx2" cargo bench -p boyko-physics --bench simd_o1
//! ```
//! Without `+avx2` the dispatcher runs scalar in both arms (the bench then shows
//! ~parity — a non-AVX2 build has no SIMD path to measure).

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};

use boyko_physics::components::ColliderShape;
use boyko_physics::math::{Mat3, Quat, Vec3};
use boyko_physics::resources::BodyState;
use boyko_physics::solver::contact::BodyEffective;
use boyko_physics::solver::simd;

/// A tiny deterministic splitmix64 RNG (no external dep) for reproducible inputs.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn f32_in(&mut self, range: f32) -> f32 {
        let u = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
        (u * 2.0 - 1.0) * range
    }
}

/// Builds `n` random dynamic bodies (eff + snapshot) — the substep working set.
fn scene(n: usize) -> (Vec<BodyEffective>, Vec<BodyState>) {
    let mut rng = Rng::new(0x00a1_0001_dead_beef);
    let mut eff = Vec::with_capacity(n);
    let mut snap = Vec::with_capacity(n);
    for _ in 0..n {
        let q = Quat::new(
            rng.f32_in(1.0),
            rng.f32_in(1.0),
            rng.f32_in(1.0),
            rng.f32_in(1.0),
        )
        .normalize();
        let inv_mass = 0.2 + rng.f32_in(1.0).abs();
        let radius = 0.5 + rng.f32_in(1.0).abs();
        let inv = inv_mass * 5.0 / (2.0 * radius * radius);
        let local = Mat3::from_diagonal(Vec3::new(inv, inv * 1.3, inv * 0.7));
        snap.push(BodyState {
            inv_inertia: Mat3::ZERO,
            inv_inertia_local: local,
            position: Vec3::new(rng.f32_in(50.0), rng.f32_in(50.0), rng.f32_in(50.0)),
            linear_velocity: Vec3::new(rng.f32_in(20.0), rng.f32_in(20.0), rng.f32_in(20.0)),
            angular_velocity: Vec3::new(rng.f32_in(10.0), rng.f32_in(10.0), rng.f32_in(10.0)),
            rotation: q,
            inv_mass,
            restitution: 0.0,
            friction: 0.5,
            simulated: true,
            kinematic: false,
            is_sensor: false,
            shape: ColliderShape::Sphere { radius },
        });
        eff.push(BodyEffective {
            inv_mass,
            inv_inertia: Mat3::ZERO,
            linear_velocity: snap.last().unwrap().linear_velocity,
            angular_velocity: snap.last().unwrap().angular_velocity,
        });
    }
    (eff, snap)
}

fn bench_o1(c: &mut Criterion) {
    const N: usize = 10_000;
    let gravity = Vec3::new(0.0, -9.81, 0.0);
    let h = 1.0 / 240.0;

    let mut group = c.benchmark_group("o1_refresh_inertia");
    {
        let (eff0, snap) = scene(N);
        group.bench_function("scalar", |b| {
            let mut eff = eff0.clone();
            b.iter(|| {
                simd::refresh_inertia_scalar(black_box(&mut eff), black_box(&snap));
                black_box(eff[0].inv_inertia.rows[0].x);
            });
        });
        group.bench_function("avx2", |b| {
            let mut eff = eff0.clone();
            b.iter(|| {
                simd::refresh_inertia(black_box(&mut eff), black_box(&snap), true);
                black_box(eff[0].inv_inertia.rows[0].x);
            });
        });
    }
    group.finish();

    let mut group = c.benchmark_group("o1_gravity");
    {
        let (eff0, snap) = scene(N);
        group.bench_function("scalar", |b| {
            let mut eff = eff0.clone();
            b.iter(|| {
                simd::apply_gravity_scalar(black_box(&mut eff), black_box(&snap), gravity, h);
                black_box(eff[0].linear_velocity.y);
            });
        });
        group.bench_function("avx2", |b| {
            let mut eff = eff0.clone();
            b.iter(|| {
                simd::apply_gravity(black_box(&mut eff), black_box(&snap), gravity, h, true);
                black_box(eff[0].linear_velocity.y);
            });
        });
    }
    group.finish();

    let mut group = c.benchmark_group("o1_position_integrate");
    {
        let (eff, snap0) = scene(N);
        group.bench_function("scalar", |b| {
            let mut snap = snap0.clone();
            b.iter(|| {
                simd::position_integrate_scalar(black_box(&eff), black_box(&mut snap), h);
                black_box(snap[0].rotation.w);
            });
        });
        group.bench_function("avx2", |b| {
            let mut snap = snap0.clone();
            b.iter(|| {
                simd::position_integrate(black_box(&eff), black_box(&mut snap), h, true);
                black_box(snap[0].rotation.w);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_o1);
criterion_main!(benches);
