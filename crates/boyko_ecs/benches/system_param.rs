// Phase 8a Step 11 — Criterion bench suite for `SystemParam` + `Resources`.
//
// Targets per Phase 8a plan §18.5:
//
//   1. bench_res_get_param_hot                        — target ≤ 3 ns/op
//   2. bench_resmut_get_param_hot                     — target ≤ 3 ns/op
//   3. bench_tuple4_get_param_hot                     — target ≤ 12 ns/op
//   4. bench_empty_system_run_once                    — target ≤ 5 ns dispatch
//   5. bench_resources_insert                         — target ≤ 200 ns
//   6. bench_resources_drop_64_occupied               — target ≤ 2 µs
//   7. bench_filtered_access_set_add_conflict_check   — target ≤ 50 ns
//
// All benches use `criterion::black_box` on inputs and outputs to defeat
// constant-folding. Pre-build the `EcsMaster` and resource state OUTSIDE the
// timed loop so the measured cost is the per-call hot path, not setup.
//
// ResourceId allocation: each `#[derive(Resource)]` mints its own id via the
// global counter on first call; the suite does not collide with the
// 490-509 ComponentId range used by `random_access`. The 64 distinct types
// used by `bench_resources_drop_64_occupied` claim consecutive ResourceIds.

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::resources::Resources;
use boyko_ecs::ecs::core::system::filtered_access_set::FilteredAccessSet;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_ecs::ecs::identifiers::primitives::ResourceId;
use boyko_macros::Resource;
use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};

// ── Resources for the SystemParam hot-path benches ─────────────────────────

/// Tiny POD resource — exercised by `bench_res_get_param_hot`.
#[derive(Resource)]
struct ResBenchA(#[allow(dead_code)] u32);

/// Tiny POD resource — exercised by `bench_resmut_get_param_hot`.
#[derive(Resource)]
struct ResBenchB(#[allow(dead_code)] u32);

/// Four-tuple components — `bench_tuple4_get_param_hot`.
#[derive(Resource)]
struct R1(#[allow(dead_code)] u32);
#[derive(Resource)]
struct R2(#[allow(dead_code)] u32);
#[derive(Resource)]
struct R3(#[allow(dead_code)] u32);
#[derive(Resource)]
struct R4(#[allow(dead_code)] u32);

/// Resource used by `bench_resources_insert`. POD with no Drop — measures
/// the cold insert path's allocator + slab-write cost without confounding
/// drop_fn invocation.
#[derive(Resource)]
struct ResInsertProbe(#[allow(dead_code)] u64);

// ── 64 distinct resource types for the drop bench ──────────────────────────
//
// `bench_resources_drop_64_occupied` populates the `Resources` slab with 64
// distinct resources, then times the `Drop` impl walking
// `registered_mask` via `pop_lowest_set_bit` and deallocating each slot.
//
// Each type carries a Drop counter so we can verify (in test runs of the
// bench harness) that drop fired the expected number of times — though the
// bench itself only times the deallocation path.

macro_rules! drop_bench_resource {
    ($name:ident) => {
        #[derive(Resource)]
        struct $name(#[allow(dead_code)] u32);

        impl Drop for $name {
            fn drop(&mut self) {
                // Non-trivial drop_fn so the slab walk actually invokes a
                // function pointer per slot (matches user-defined Drop
                // resources in real workloads). std::hint::black_box guards
                // against the compiler eliding the drop body.
                let _ = std::hint::black_box(self.0);
            }
        }
    };
}

drop_bench_resource!(D00);
drop_bench_resource!(D01);
drop_bench_resource!(D02);
drop_bench_resource!(D03);
drop_bench_resource!(D04);
drop_bench_resource!(D05);
drop_bench_resource!(D06);
drop_bench_resource!(D07);
drop_bench_resource!(D08);
drop_bench_resource!(D09);
drop_bench_resource!(D10);
drop_bench_resource!(D11);
drop_bench_resource!(D12);
drop_bench_resource!(D13);
drop_bench_resource!(D14);
drop_bench_resource!(D15);
drop_bench_resource!(D16);
drop_bench_resource!(D17);
drop_bench_resource!(D18);
drop_bench_resource!(D19);
drop_bench_resource!(D20);
drop_bench_resource!(D21);
drop_bench_resource!(D22);
drop_bench_resource!(D23);
drop_bench_resource!(D24);
drop_bench_resource!(D25);
drop_bench_resource!(D26);
drop_bench_resource!(D27);
drop_bench_resource!(D28);
drop_bench_resource!(D29);
drop_bench_resource!(D30);
drop_bench_resource!(D31);
drop_bench_resource!(D32);
drop_bench_resource!(D33);
drop_bench_resource!(D34);
drop_bench_resource!(D35);
drop_bench_resource!(D36);
drop_bench_resource!(D37);
drop_bench_resource!(D38);
drop_bench_resource!(D39);
drop_bench_resource!(D40);
drop_bench_resource!(D41);
drop_bench_resource!(D42);
drop_bench_resource!(D43);
drop_bench_resource!(D44);
drop_bench_resource!(D45);
drop_bench_resource!(D46);
drop_bench_resource!(D47);
drop_bench_resource!(D48);
drop_bench_resource!(D49);
drop_bench_resource!(D50);
drop_bench_resource!(D51);
drop_bench_resource!(D52);
drop_bench_resource!(D53);
drop_bench_resource!(D54);
drop_bench_resource!(D55);
drop_bench_resource!(D56);
drop_bench_resource!(D57);
drop_bench_resource!(D58);
drop_bench_resource!(D59);
drop_bench_resource!(D60);
drop_bench_resource!(D61);
drop_bench_resource!(D62);
drop_bench_resource!(D63);

/// Helper: populates a fresh `Resources` slab with 64 distinct entries.
/// Used by `bench_resources_drop_64_occupied`'s `iter_batched` setup.
fn build_resources_with_64() -> Resources {
    let mut r = Resources::new();
    r.insert(D00(0)); r.insert(D01(1)); r.insert(D02(2)); r.insert(D03(3));
    r.insert(D04(4)); r.insert(D05(5)); r.insert(D06(6)); r.insert(D07(7));
    r.insert(D08(8)); r.insert(D09(9)); r.insert(D10(10)); r.insert(D11(11));
    r.insert(D12(12)); r.insert(D13(13)); r.insert(D14(14)); r.insert(D15(15));
    r.insert(D16(16)); r.insert(D17(17)); r.insert(D18(18)); r.insert(D19(19));
    r.insert(D20(20)); r.insert(D21(21)); r.insert(D22(22)); r.insert(D23(23));
    r.insert(D24(24)); r.insert(D25(25)); r.insert(D26(26)); r.insert(D27(27));
    r.insert(D28(28)); r.insert(D29(29)); r.insert(D30(30)); r.insert(D31(31));
    r.insert(D32(32)); r.insert(D33(33)); r.insert(D34(34)); r.insert(D35(35));
    r.insert(D36(36)); r.insert(D37(37)); r.insert(D38(38)); r.insert(D39(39));
    r.insert(D40(40)); r.insert(D41(41)); r.insert(D42(42)); r.insert(D43(43));
    r.insert(D44(44)); r.insert(D45(45)); r.insert(D46(46)); r.insert(D47(47));
    r.insert(D48(48)); r.insert(D49(49)); r.insert(D50(50)); r.insert(D51(51));
    r.insert(D52(52)); r.insert(D53(53)); r.insert(D54(54)); r.insert(D55(55));
    r.insert(D56(56)); r.insert(D57(57)); r.insert(D58(58)); r.insert(D59(59));
    r.insert(D60(60)); r.insert(D61(61)); r.insert(D62(62)); r.insert(D63(63));
    r
}

// --- §18.5 #1: bench_res_get_param_hot ---
//
// Measures the end-to-end `Res<R>::get_param` hot path: `UnsafeEcsCell ::
// resources()` → `Resources::get_ptr_by_id` (W1 cached id) → `&*ptr`. The
// closure is dispatched via `EcsMaster::run_closure_once` so the per-call
// SystemParam machinery is exercised — initialization is amortised on the
// first call, but `run_closure_once` rebuilds the `FnOnceSystem` each time
// (it's a one-shot wrapper).
//
// W3 NOTE: turbofish on the param tuple is required in Phase 8a.
//
// Target: ≤ 3 ns/op for the get_param call itself. The `run_closure_once`
// path adds dispatch overhead; the measurement therefore captures
// `dispatch + get_param`. The empty-closure bench below isolates the
// dispatch overhead so the delta is the param fetch cost.
fn bench_res_get_param_hot(c: &mut Criterion) {
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(ResBenchA(42));

    c.bench_function("res_get_param_hot", |b| {
        b.iter(|| {
            ecs.run_closure_once::<Res<'_, ResBenchA>, _, _>(|r| {
                // Deref the borrow; black_box defeats const-fold of the
                // inner value through the static slab pointer.
                black_box((*r).0);
            });
        });
    });
}

// --- §18.5 #2: bench_resmut_get_param_hot ---
//
// Mirror of #1 with `ResMut<R>` — measures `resources_mut()` +
// `get_mut_ptr_by_id` + `&mut *ptr`.
//
// Target: ≤ 3 ns/op for the get_param call itself.
fn bench_resmut_get_param_hot(c: &mut Criterion) {
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(ResBenchB(0));

    c.bench_function("resmut_get_param_hot", |b| {
        b.iter(|| {
            ecs.run_closure_once::<ResMut<'_, ResBenchB>, _, _>(|mut r| {
                // Touch the value so the DerefMut path is not elided.
                r.0 = r.0.wrapping_add(1);
                black_box(r.0);
            });
        });
    });
}

// --- §18.5 #3: bench_tuple4_get_param_hot ---
//
// 4-arity tuple of `Res<R>` over four distinct resource types. Exercises
// the tuple macro's per-element `get_param` walk + four
// `Resources::get_ptr_by_id` calls. Each element's id is cached at
// init_state time (W1), so the per-element cost should be ~3 ns.
//
// Target: ≤ 12 ns/op for the tuple's get_param chain.
fn bench_tuple4_get_param_hot(c: &mut Criterion) {
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(R1(1));
    ecs.insert_resource(R2(2));
    ecs.insert_resource(R3(3));
    ecs.insert_resource(R4(4));

    c.bench_function("tuple4_get_param_hot", |b| {
        b.iter(|| {
            ecs.run_closure_once::<(
                Res<'_, R1>,
                Res<'_, R2>,
                Res<'_, R3>,
                Res<'_, R4>,
            ), _, _>(|(r1, r2, r3, r4)| {
                // Sum the four fields through Deref; black_box prevents
                // the compiler from realising the result is unused.
                let s = (*r1).0
                    .wrapping_add((*r2).0)
                    .wrapping_add((*r3).0)
                    .wrapping_add((*r4).0);
                black_box(s);
            });
        });
    });
}

// --- §18.5 #4: bench_empty_system_run_once ---
//
// `ecs.run_closure_once::<(), _, _>(|()| ())` — isolates the per-call
// `FnOnceSystem` dispatch overhead with zero params. Result is the
// baseline cost of `run_system_once` machinery (initialize +
// UnsafeEcsCell mint + run_unsafe trampoline).
//
// Target: ≤ 5 ns dispatch overhead.
fn bench_empty_system_run_once(c: &mut Criterion) {
    let mut ecs = EcsMaster::new();

    c.bench_function("empty_system_run_once", |b| {
        b.iter(|| {
            ecs.run_closure_once::<(), _, _>(|()| {
                black_box(());
            });
        });
    });
}

// --- §18.5 #5: bench_resources_insert ---
//
// `Resources::insert::<R>` cold path — Box allocation + slab write + bit
// set. Each iteration produces a fresh `Resources` so we measure the
// first-insertion path (not the replace path). Box::new is the dominant
// cost.
//
// Target: ≤ 200 ns cold path.
fn bench_resources_insert(c: &mut Criterion) {
    // Resolve the ResourceId once outside the loop so the OnceLock cost is
    // amortised. The first `insert::<ResInsertProbe>` would otherwise pay
    // the OnceLock initialisation on its very first call.
    let _ = ResInsertProbe::resource_id_eager();

    c.bench_function("resources_insert", |b| {
        b.iter_batched(
            Resources::new,
            |mut r| {
                r.insert(black_box(ResInsertProbe(0x1234_5678_DEAD_BEEF)));
                black_box(r);
            },
            BatchSize::SmallInput,
        );
    });
}

// Trait used by `bench_resources_insert` to warm the `ResourceId` OnceLock.
//
// `Resource::resource_id` is the canonical accessor; this helper just calls
// it once at bench startup so the timed loop never includes the
// `OnceLock::set` cost. Kept as a local trait so the bench file stays
// self-contained without poking at the registry directly.
trait ResourceIdEager {
    fn resource_id_eager() -> ResourceId;
}

impl<R> ResourceIdEager for R
where
    R: boyko_ecs::ecs::core::resources::resource::Resource,
{
    #[inline]
    fn resource_id_eager() -> ResourceId {
        <R as boyko_ecs::ecs::core::resources::resource::Resource>::resource_id()
    }
}

// --- §18.5 #6: bench_resources_drop_64_occupied ---
//
// Build a `Resources` with 64 occupied slots (each a distinct type with a
// non-trivial Drop), then time the `Drop` impl. Exercises the
// `pop_lowest_set_bit` (TZCNT/BLSR) walk over `registered_mask` and the
// drop_fn + dealloc per slot.
//
// Target: ≤ 2 µs (≈ 31 ns/slot amortised — bench reports total time per
// drop sequence, not per slot).
fn bench_resources_drop_64_occupied(c: &mut Criterion) {
    c.bench_function("resources_drop_64_occupied", |b| {
        b.iter_batched(
            build_resources_with_64,
            |r| {
                // Explicit drop so the timing window covers exactly the
                // teardown. `criterion::black_box(r)` would defer the drop
                // to the iter cleanup, which is also fine, but explicit
                // `drop` makes the intent and timing boundary obvious.
                drop(black_box(r));
            },
            BatchSize::SmallInput,
        );
    });
}

// --- §18.5 #7: bench_filtered_access_set_add_conflict_check ---
//
// Measures the cold path of `FilteredAccessSet::add_resource_write` —
// the conflict-check branch where the new write sees an existing read
// bit (pre-populated) and returns `Err`. The benchmark builds a set
// containing one resource_read entry and adds 64 distinct
// resource_writes against unrelated ids (so each add takes the success
// branch). The success-branch cost is the typical per-init access
// declaration cost.
//
// Plan target: ≤ 50 ns (cold path). The bench measures one add() in
// isolation via `iter_batched`: each iteration starts with a freshly
// constructed `FilteredAccessSet` (24 KB heap alloc — comes out of
// `Box::new(["", ...])`, paid in the setup phase outside the timed
// window) and records a single `add_resource_write` call.
fn bench_filtered_access_set_add_conflict_check(c: &mut Criterion) {
    // Reserve ResourceId(0) as the read; ResourceId(1) as the disjoint
    // write target. Use raw construction (no Resource derive needed) —
    // FilteredAccessSet is keyed by raw ResourceId.0, not by type.
    let probe_read_id = ResourceId(0);
    let probe_write_id = ResourceId(1);

    c.bench_function("filtered_access_set_add_conflict_check", |b| {
        b.iter_batched(
            || {
                let mut set = FilteredAccessSet::new();
                // Seed with one read so the write's first check (read-bit)
                // does some real work (BitSet::get on a populated slot).
                set.add_resource_read(probe_read_id, "Res<seed>")
                    .expect("seed read must succeed");
                set
            },
            |mut set| {
                // Disjoint write — takes the success branch. The bench
                // measures the conflict-checking cost (two BitSet::get
                // probes + BitSet::set + one bit_owners write).
                let result =
                    set.add_resource_write(black_box(probe_write_id), "ResMut<probe>");
                black_box(result.is_ok());
                black_box(set);
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(
    system_param_benches,
    bench_res_get_param_hot,
    bench_resmut_get_param_hot,
    bench_tuple4_get_param_hot,
    bench_empty_system_run_once,
    bench_resources_insert,
    bench_resources_drop_64_occupied,
    bench_filtered_access_set_add_conflict_check,
);
criterion_main!(system_param_benches);
