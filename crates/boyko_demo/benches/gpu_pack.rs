//! Phase 20.1 G2 — pack-cost gate: the 16 B baseline pack vs the 24 B
//! interpolated pack (prev shuffle), 100 k rows.
//!
//! The pack body is MIRRORED on plain slices (a frozen copy of the
//! `sync_gpu_instance` loop in `src/sim/systems/common.rs` — keep them in
//! lockstep). The mirror is chosen for ISOLATION of the 16 → 24 B delta, not
//! for noise reduction (★n7): driving the real system through a schedule would
//! swamp the per-row signal with dispatch cost.
//!
//! ★R1-4 calibrated gate: the 16 B BASELINE mirror is measured FIRST.
//! * baseline <= 3 ns/row → the binding gate is the absolute: 24 B <= 5 ns/row;
//! * baseline >  3 ns/row (the sqrt+div body is divider-port-bound and
//!   machine-dependent) → the binding gate is the RATIO 24 B / 16 B <= 1.6×,
//!   with the absolute reported informationally.
//!
//! ns/row = (criterion time per iteration) / 100 000.

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::hint::black_box;

use boyko_demo::render::instance::GpuInstance;
use boyko_demo::sim::components::{Position, Velocity};

/// Row count matching the Particles-mode population (the heaviest pack).
const ROWS: usize = 100_000;

/// Speed mapped to the top of the color ramp — frozen copy of
/// `common.rs::COLOR_SPEED_MAX`.
const COLOR_SPEED_MAX: f32 = 200.0;

/// Frozen copy of `common.rs::speed_color` (private there; the mirror must not
/// change the per-row arithmetic).
#[inline]
fn speed_color(t: f32) -> [u8; 4] {
    let r = (t.mul_add(2.0, -1.0).max(0.0) * 255.0) as u8;
    let g = ((t * 1.4).min(1.0) * 255.0) as u8;
    let b = (180.0 + t * 75.0) as u8;
    [r, g, b, 255]
}

/// The PRE-20.1 16 B record — the baseline mirror's output shape (pos, scale,
/// packed color; no prev).
#[repr(C)]
#[derive(Clone, Copy)]
struct Gpu16 {
    pos: [f32; 2],
    scale: f32,
    color: u32,
}

const _: () = assert!(size_of::<Gpu16>() == 16);

/// Deterministic input columns (seeded StdRng): positions in the ±100 world
/// box, velocities up to ±220 (the SimParams max_speed scale).
fn make_inputs() -> (Vec<Position>, Vec<Velocity>) {
    let mut rng = StdRng::seed_from_u64(0x2001);
    let positions = (0..ROWS)
        .map(|_| Position {
            x: rng.random_range(-100.0..100.0),
            y: rng.random_range(-100.0..100.0),
        })
        .collect();
    let velocities = (0..ROWS)
        .map(|_| Velocity {
            x: rng.random_range(-220.0..220.0),
            y: rng.random_range(-220.0..220.0),
        })
        .collect();
    (positions, velocities)
}

fn bench_gpu_pack(c: &mut Criterion) {
    let (positions, velocities) = make_inputs();
    let scale = 0.6_f32;

    let mut group = c.benchmark_group("gpu_pack");
    group.throughput(Throughput::Elements(ROWS as u64));

    // ── 16 B BASELINE mirror (measured FIRST, ★R1-4) ─────────────────────────
    // Frozen copy of the PRE-20.1 sync_gpu_instance row body: read pos/vel,
    // sqrt+clamp ramp, write the 16 B record. No prev shuffle.
    let mut out16: Vec<Gpu16> = vec![
        Gpu16 {
            pos: [0.0; 2],
            scale: 0.0,
            color: 0
        };
        ROWS
    ];
    group.bench_function("baseline_16b_pack_100k", |b| {
        b.iter(|| {
            for ((pos, vel), out) in positions.iter().zip(&velocities).zip(out16.iter_mut()) {
                let speed = (vel.x * vel.x + vel.y * vel.y).sqrt();
                let t = (speed / COLOR_SPEED_MAX).clamp(0.0, 1.0);
                let rgba = speed_color(t);
                *out = Gpu16 {
                    pos: [pos.x, pos.y],
                    scale,
                    color: GpuInstance::pack_rgba8(rgba),
                };
            }
            black_box(out16.as_slice());
        });
    });

    // ── 24 B interpolated pack (the Phase-20.1 body) ─────────────────────────
    // Frozen copy of the CURRENT sync_gpu_instance row body (common.rs): the
    // prev shuffle reads the old packed pos (+8 B load) and the record write
    // grows to 24 B (+8 B store).
    let mut out24: Vec<GpuInstance> = positions
        .iter()
        .map(|p| GpuInstance::new([p.x, p.y], scale, [80, 160, 255, 255]))
        .collect();
    group.bench_function("interpolated_24b_pack_100k", |b| {
        b.iter(|| {
            for ((pos, vel), gpu) in positions.iter().zip(&velocities).zip(out24.iter_mut()) {
                let speed = (vel.x * vel.x + vel.y * vel.y).sqrt();
                let t = (speed / COLOR_SPEED_MAX).clamp(0.0, 1.0);
                let prev = gpu.pos;
                *gpu = GpuInstance::with_prev(prev, [pos.x, pos.y], scale, speed_color(t));
            }
            black_box(out24.as_slice());
        });
    });

    group.finish();
}

criterion_group!(benches, bench_gpu_pack);
criterion_main!(benches);
