//! POB (PlainOldBytes) save/load THROUGHPUT bench — bytes/sec (GiB/s).
//!
//! Goal: show whether the codegen-blit path is **memcpy-bound**, i.e. that a
//! `PlainOldBytes` column is serialized with one whole-column
//! `copy_nonoverlapping` and there is NO per-field / reflection-style per-element
//! overhead. If save throughput sits in the single-thread DRAM `memcpy` band
//! (~10-20 GiB/s on a typical desktop), the blit is the bottleneck, not the
//! serializer's bookkeeping.
//!
//! Layout: two POB components summing to 64 B/entity:
//!   - `Transform { m: [f32; 12] }` = 48 B
//!   - `Motion    { v: [f32; 4]  }` = 16 B
//!
//! One archetype, N = 200_000 entities → 12.8 MB of blitted column payload.
//!
//! `criterion::Throughput::Bytes(N * 64)` reports the per-bench `thrpt:` line in
//! GiB/s. The world is built ONCE outside the timed loops; the save buffer is
//! pre-`with_capacity`'d and only `clear()`ed per iteration so we time the blit,
//! not allocation. The load setup (a fresh `EcsMaster`) is the `iter_batched`
//! setup and excluded from timing.

use criterion::{
    BatchSize, Criterion, Throughput, black_box, criterion_group, criterion_main,
};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_macros::Component;
use boyko_serialize::{LoadEntityPolicy, SaveOptions, load_world, save_world};

/// POB: a 4x3 affine transform, 48 B. `#[repr(C)]` + all-`f32` → `PlainOldBytes`,
/// blitted whole-column.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Transform {
    m: [f32; 12],
}

/// POB: a velocity + scalar pad, 16 B. `#[repr(C)]` + all-`f32` → `PlainOldBytes`.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Motion {
    v: [f32; 4],
}

/// Entities in the timed world. 200k × 64 B = 12.8 MB of blitted column data.
const N: u64 = 200_000;
/// Logical bytes/entity (Transform 48 + Motion 16), the `Throughput` numerator.
const BYTES_PER_ENTITY: u64 = 64;

/// Builds a single-archetype world of `N` entities, each carrying a `Transform`
/// and a `Motion`. Done ONCE, outside every timed loop.
fn build_world() -> EcsMaster {
    // Compile-time guard: the 64 B/entity throughput figure is only honest if the
    // two components really sum to 64 B with no `#[repr(C)]` tail padding.
    const _: () = assert!(core::mem::size_of::<Transform>() == 48);
    const _: () = assert!(core::mem::size_of::<Motion>() == 16);
    const _: () = assert!(
        (core::mem::size_of::<Transform>() + core::mem::size_of::<Motion>()) as u64
            == BYTES_PER_ENTITY
    );

    let mut w = EcsMaster::new();
    let arch = w.get_or_create_archetype(&[Transform::component_id(), Motion::component_id()]);
    for i in 0..N {
        let f = i as f32;
        let t = Transform {
            m: [
                f, f + 1.0, f + 2.0, f + 3.0, f + 4.0, f + 5.0, f + 6.0, f + 7.0, f + 8.0,
                f + 9.0, f + 10.0, f + 11.0,
            ],
        };
        let mo = Motion { v: [f, -f, f * 2.0, f * 0.5] };
        w.spawn_two(arch, t, mo).expect("spawn_two");
    }
    w
}

fn bench_pob_throughput(c: &mut Criterion) {
    let world = build_world();

    // Save once up front: report the on-disk size and confirm the payload is
    // dominated by the blitted column data (N*64), not header/table overhead.
    let mut probe = Vec::new();
    save_world(&world, &SaveOptions::default(), &mut probe).expect("probe save");
    let total = probe.len() as u64;
    let payload = N * BYTES_PER_ENTITY;
    eprintln!(
        "[pob_throughput] N={N} entities | serialized world = {total} bytes | \
         blitted column payload = {payload} bytes ({:.2}%) | header/table/entity-row overhead = {} bytes ({:.2}%)",
        payload as f64 / total as f64 * 100.0,
        total - payload,
        (total - payload) as f64 / total as f64 * 100.0,
    );

    let mut group = c.benchmark_group("pob");

    // ── SAVE: pure blit throughput ──────────────────────────────────────────
    // Reuse a pre-sized `out` so the per-iter cost is the `copy_nonoverlapping`
    // column blit + the Pass-1 walk, not buffer (re)allocation.
    group.throughput(Throughput::Bytes(payload));
    let opts = SaveOptions::default();
    let mut out = Vec::with_capacity(total as usize);
    group.bench_function("save", |b| {
        b.iter(|| {
            out.clear();
            save_world(black_box(&world), &opts, &mut out).expect("save");
            black_box(&out);
        });
    });

    // ── LOAD: archetype rebuild + entity alloc + blit ─────────────────────────
    // The fresh `EcsMaster` is the batched setup (excluded from timing); the timed
    // closure is the load itself.
    let bytes = out_to_bytes(&world);
    group.throughput(Throughput::Bytes(payload));
    group.bench_function("load", |b| {
        b.iter_batched(
            EcsMaster::new,
            |mut dst| {
                load_world(&mut dst, black_box(&bytes), LoadEntityPolicy::Remap).expect("load");
                black_box(&dst);
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

/// Saves `world` to a fresh `Vec<u8>` (the immutable load input).
fn out_to_bytes(world: &EcsMaster) -> Vec<u8> {
    let mut bytes = Vec::new();
    save_world(world, &SaveOptions::default(), &mut bytes).expect("save for load input");
    bytes
}

criterion_group!(benches, bench_pob_throughput);
criterion_main!(benches);
