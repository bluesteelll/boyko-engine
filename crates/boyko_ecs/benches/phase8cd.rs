//! Phase 8c+8d Step 11 — Criterion bench suite for `FunctionSystem` /
//! `Commands` / `CommandQueue` hot paths.
//!
//! Targets per plan §25.5:
//!
//! | Bench                                          | Target |
//! |------------------------------------------------|--------|
//! | bench_function_system_run_unsafe_empty_hoisted | ≤ 5 ns  |
//! | bench_function_system_run_unsafe_res_param_hoisted | ≤ 8 ns  |
//! | bench_run_closure_once_reused_vs_phase_8a_baseline | ≤ 30 ns |
//! | bench_run_closure_once_first_call_cold (O2')   | ≈ 1.2 µs |
//! | bench_commands_spawn_one_enqueue (CommandQueue::push variant) | ≤ 20 ns |
//! | bench_command_queue_empty_apply                | ≤ 3 ns  |
//! | bench_command_queue_spawn_arity_1_apply        | comparable to direct create_entity |
//! | bench_into_system_via_run_system (one-shot cold) | ~ 1.2 µs |
//!
//! All benches use `criterion::black_box` on inputs and outputs to defeat
//! constant-folding. Pre-build the world / resource / queue state OUTSIDE
//! the timed loop so the measured cost is the per-call hot path, not setup.
//!
//! No `#[inline(always)]` is used in this file — cycle counts can mislead
//! and criterion's framework handles dispatch overhead correctly without
//! it (see CLAUDE.md principle #7).

// Phase X.E: opt-in low-variance allocator for A/B signal extraction.
// OFF by default (`cargo bench` keeps the production system heap for honest
// absolutes); `cargo bench --features bench-alloc` swaps in mimalloc, which
// is far more deterministic and exposes structural signals the system heap
// masks (the documented ±20-30% variance source). See docs/BENCHMARKING.md.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::hint::black_box;

use boyko_ecs::ecs::core::commands::Command;
use boyko_ecs::ecs::core::commands::command_queue::CommandQueue;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::{Commands, IntoSystem, Res, ResMut};
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::{Bundle, Resource};
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};

// ── Resources for the hot-path benches ──────────────────────────────────────

#[derive(Resource)]
struct Cd8ResA(#[allow(dead_code)] u32);

#[derive(Resource)]
struct Cd8ResB(#[allow(dead_code)] u32);

// ── Component for the spawn-throughput bench ────────────────────────────────

const SLOT_BENCH_C: ComponentId = ComponentId(259);

#[repr(C)]
#[derive(Clone, Copy)]
struct Cd8C(u32);

impl Component for Cd8C {
    fn component_id() -> ComponentId {
        SLOT_BENCH_C
    }
}

/// Phase 8.5 derived bundle wrapping the single `Cd8C` component for the
/// arity-1 spawn-throughput bench. Declared at module scope (not inside
/// the bench fn) so the per-impl `BundleStaticInfo` `OnceLock` slot is
/// stable across `iter_batched` invocations — first iteration pays the
/// cold init cost; every subsequent iteration observes the cached payload
/// (the Phase 8.5 hot-path SBC4 target ≈ 3 ns).
#[derive(Bundle)]
struct Cd8CBundle {
    c: Cd8C,
}

// ── No-op Command for queue throughput tests ────────────────────────────────

/// Zero-sized no-op command. `Drop` is the trivial compiler-generated one,
/// so the per-cmd cost in `apply` is dominated by the queue's
/// `consume_and_drop_glue` + cursor advance, not by user-side work.
struct Noop;

impl Command for Noop {
    fn apply(self, _world: &mut EcsMaster) {
        // black_box keeps the call from being optimised into nothing.
        black_box(());
    }
}

// =============================================================================
// 1. bench_function_system_run_unsafe_empty_hoisted (≤ 5 ns)
// =============================================================================
//
// HOISTED FunctionSystem: `IntoSystem::into_system` is invoked ONCE outside
// the timed loop; each iteration calls `run_cached_system` which only pays
// the `initialize` cost on the first iter (FS1 idempotence ⇒ no-op
// thereafter). Empty body ⇒ `apply` is a no-op too. The measured cost is
// the FunctionSystem trampoline: state.as_mut + UnsafeEcsCell::new_mutable
// + get_param + body call + apply forward.

fn bench_function_system_run_unsafe_empty_hoisted(c: &mut Criterion) {
    let mut ecs = EcsMaster::new();
    let mut sys = IntoSystem::into_system(|| {
        black_box(());
    });
    // Warm the cache: first call pays the cold initialize cost.
    ecs.run_cached_system(&mut sys);

    c.bench_function("function_system_run_unsafe_empty_hoisted", |b| {
        b.iter(|| {
            ecs.run_cached_system(&mut sys);
        });
    });
}

// =============================================================================
// 2. bench_function_system_run_unsafe_res_param_hoisted (≤ 8 ns)
// =============================================================================
//
// HOISTED FunctionSystem with a single `Res<R>` param. Per-call cost =
// dispatch (≤ 5 ns) + get_param (≤ 3 ns). The body touches the resource
// through Deref so the param fetch is observably exercised.

fn bench_function_system_run_unsafe_res_param_hoisted(c: &mut Criterion) {
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(Cd8ResA(42));
    let mut sys = IntoSystem::into_system(|r: Res<Cd8ResA>| {
        black_box((*r).0);
    });
    ecs.run_cached_system(&mut sys);

    c.bench_function("function_system_run_unsafe_res_param_hoisted", |b| {
        b.iter(|| {
            ecs.run_cached_system(&mut sys);
        });
    });
}

// =============================================================================
// 3. bench_run_closure_once_reused_vs_phase_8a_baseline (≤ 30 ns)
// =============================================================================
//
// `EcsMaster::run_closure_once(|...| ...)` — the public alias that Phase 8c
// rewires to `run_system`. Each call rebuilds a FunctionSystem from
// scratch (one-shot path), so the measured cost is initialize + dispatch
// + body + apply per call. The closure is identical between iterations,
// so monomorphisation caches the type's vtable / SystemParam state slot
// in a single CPU register-bank — measured cost stays well below the
// Phase 8a 960 ns rebuild figure (since the rebuild is now a thin
// FunctionSystem stack object, not a heap-allocated FnOnceSystem).

fn bench_run_closure_once_reused_vs_phase_8a_baseline(c: &mut Criterion) {
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(Cd8ResB(7));

    c.bench_function("run_closure_once_reused_vs_phase_8a_baseline", |b| {
        b.iter(|| {
            ecs.run_closure_once(|r: Res<Cd8ResB>| {
                black_box((*r).0);
            });
        });
    });
}

// =============================================================================
// 4. bench_run_closure_once_first_call_cold (O2' — ≈ 1.2 µs)
// =============================================================================
//
// First call's cumulative cost: ≤ 1 µs initialize + ≤ 30 ns dispatch +
// closure body + apply. Use `iter_batched` so each iteration starts with
// a fresh `EcsMaster` ⇒ the `Resources` slab is empty ⇒ the closure can't
// touch a resource (or we'd need to re-insert it per iter). We use a
// no-param body so the cold cost is the pure FunctionSystem construction.

fn bench_run_closure_once_first_call_cold(c: &mut Criterion) {
    // Per-iter EcsMaster construction (the textbook "cold call" recipe)
    // exhausts the global arena's address space under criterion's
    // typical sample size — EcsMaster currently retains its arena slabs
    // at drop (Phase 7 design). We measure cold-init cost from the
    // run_closure_once side ONLY: the per-iter cost is `IntoSystem
    // ::into_system + initialize + run_unsafe + apply` rebuilt every
    // iter (no caching across calls, per plan §1.2).
    //
    // The world is hoisted; the "cold" property still holds because
    // `run_closure_once` rebuilds the FunctionSystem each call. Phase 9
    // will revisit the EcsMaster-drop policy so this bench can use the
    // ideal `iter_batched(EcsMaster::new, ...)` pattern.
    let mut ecs = EcsMaster::new();
    c.bench_function("run_closure_once_first_call_cold", |b| {
        b.iter(|| {
            ecs.run_closure_once(|| {
                black_box(());
            });
        });
    });
}

// =============================================================================
// 5. bench_command_queue_push (≤ 20 ns target — proxy for Commands::spawn)
// =============================================================================
//
// `CommandQueue::push<Noop>` — the lowest-level enqueue path. Two
// `write_unaligned` calls + amortised `Vec::reserve` cost. Each iteration
// fills the queue with N=64 pushes from a fresh queue so the amortised
// reserve cost is folded into the measurement; `iter_batched` resets to
// a fresh queue between batches so the heap allocation budget stays
// bounded.

fn bench_command_queue_push(c: &mut Criterion) {
    c.bench_function("command_queue_push", |b| {
        b.iter_batched(
            CommandQueue::__test_new,
            |mut q| {
                for _ in 0..64 {
                    q.__test_push(Noop);
                }
                black_box(q);
            },
            BatchSize::SmallInput,
        );
    });
}

// =============================================================================
// 6. bench_command_queue_apply_empty (≤ 3 ns target)
// =============================================================================
//
// `CommandQueue::apply` on an empty queue — the D1 early-out path. The
// queue's `bytes.is_empty()` check fires first; no RawCommandQueue mint,
// no loop. The measured cost is two stack reads + one conditional jump.

fn bench_command_queue_apply_empty(c: &mut Criterion) {
    let mut ecs = EcsMaster::new();
    let mut q = CommandQueue::__test_new();

    c.bench_function("command_queue_apply_empty", |b| {
        b.iter(|| {
            q.__test_apply(&mut ecs);
        });
    });
}

// =============================================================================
// 7. bench_command_queue_spawn_arity_1_apply
// =============================================================================
//
// Full enqueue + apply cycle for an arity-1 SpawnCommand. Each iteration
// starts with a fresh world (so the entity_count doesn't grow unbounded)
// and timed scope covers: enqueue → apply → create_entity. Baseline for
// comparison with the direct `EcsMaster::spawn_one` path.

fn bench_command_queue_spawn_arity_1_apply(c: &mut Criterion) {
    // Resolve the slot once outside the loop. The Phase 8.5 derived
    // `Cd8CBundle` resolves its destination archetype lazily inside
    // `SpawnCommand::apply` via `Cd8CBundle::cached_archetype_id` (SBC4) —
    // hot path ≈ 3 ns after the first iteration warms both the bundle's
    // `BundleStaticInfo` `OnceLock` and the per-world
    // `bundle_archetype_cache` slot.
    register_layout::<Cd8C>(SLOT_BENCH_C.0);

    c.bench_function("command_queue_spawn_arity_1_apply", |b| {
        b.iter_batched(
            EcsMaster::new,
            |mut ecs| {
                ecs.run_system(|mut cmds: Commands| {
                    cmds.spawn(Cd8CBundle { c: Cd8C(7) });
                });
                black_box(ecs);
            },
            BatchSize::SmallInput,
        );
    });
}

// =============================================================================
// 8. bench_into_system_via_run_system (cold one-shot cost)
// =============================================================================
//
// Mirrors #4 but goes through `run_system` explicitly (rather than the
// `run_closure_once` alias). With a tuple param the cost includes the
// per-element get_param walk. Used to compare the two entry points for
// any future divergence.

fn bench_into_system_via_run_system(c: &mut Criterion) {
    // Hoist the world OUTSIDE the timed loop. Each iter re-uses the same
    // EcsMaster + resources; the cold cost we measure is the
    // `IntoSystem::into_system(...)` rebuild + initialize on EVERY call
    // (Phase 8c semantic: `run_system` rebuilds the FunctionSystem each
    // call — see plan §1.2 "rebuilds FunctionSystem each call").
    //
    // The original `iter_batched(EcsMaster::new, ...)` pattern exhausted
    // the global arena's address space under high iteration counts —
    // EcsMaster currently retains its arena slabs at drop (Phase 7 design).
    // Sharing one world across iters is the proper "many cold inits, one
    // world" measurement.
    let mut ecs = EcsMaster::new();
    ecs.insert_resource(Cd8ResA(11));
    ecs.insert_resource(Cd8ResB(22));

    c.bench_function("into_system_via_run_system_cold_2_param", |b| {
        b.iter(|| {
            ecs.run_system(|(a, b): (Res<Cd8ResA>, ResMut<Cd8ResB>)| {
                // Touch both resources so the get_param walk isn't elided.
                let _ = black_box((*a).0);
                let _ = black_box(b.0);
            });
        });
    });
}

criterion_group!(
    phase8cd_benches,
    bench_function_system_run_unsafe_empty_hoisted,
    bench_function_system_run_unsafe_res_param_hoisted,
    bench_run_closure_once_reused_vs_phase_8a_baseline,
    bench_run_closure_once_first_call_cold,
    bench_command_queue_push,
    bench_command_queue_apply_empty,
    bench_command_queue_spawn_arity_1_apply,
    bench_into_system_via_run_system,
);
criterion_main!(phase8cd_benches);
