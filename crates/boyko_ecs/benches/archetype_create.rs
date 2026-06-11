//! Phase X.I W6 — XI-B2 archetype-creation cost.
//!
//! Measures `Archetype::create_by_ids` in isolation. Post-X.I an archetype's
//! pools are RESERVE-ONLY at creation: one `VmReservation::reserve` per pool
//! (no commit charge), zero tick allocation, no arena carve, no chunk Vec —
//! creation collapses to a few VA-reservation syscalls plus small
//! bookkeeping (plan D3).
//!
//! # Gate (binding, docs/PHASE-XI-PLAN.md §Metrics XI-B2)
//!
//! * `archetype_create/3x192B` — **gate <= 25 us, prediction 2-5 us**, vs an
//!   estimated ~150-400 us pre-X.I. The deleted pre-X.I terms: 6 x 256 KiB
//!   per-element-initialized tick `Box<[UnsafeCell<Tick>]>`es (the dominant
//!   memset + page-fault cost), 3 x arena `allocate_layout` carves, and
//!   3 x 128-`Chunk` bookkeeping Vecs.
//! * `archetype_create/8x4B` — mirrors the legacy 8-component shape of
//!   `benches/archetype.rs` (8 x u32). Untargeted companion number.
//!
//! # Measurement shape
//!
//! `iter_batched_ref` with a fresh `Arena::new()` per iteration in SETUP —
//! the arena is reserve-only (~1 us, Phase X.C) and vestigial post-X.I D8
//! (pools ignore it; retired outright in X.J). The timed region is
//! `create_by_ids` ONLY; the created `Archetype` is returned as the routine
//! output so its drop (pool-reservation release) stays OUTSIDE the timed
//! region.
//!
//! `BatchSize::PerIteration`, NOT SmallInput: each 3x192B archetype reserves
//! ~3.2 GiB of VA (~1 GiB data + 2 tick sub-regions per pool, D2 sizing) and
//! each setup arena reserves another ~4 GiB. SmallInput materializes
//! `iters/10` inputs per batch — at a us-scale body that is thousands of
//! live multi-GiB reservations, which threatens the VA space / VAD budget.
//! PerIteration holds at most one input + one output alive; its per-
//! iteration timing overhead (~tens of ns) is noise against the us-scale
//! body.
//!
//! # Component-slot reservation
//!
//! This bench claims ids **450-452** (3 x 192 B) and **440-447** (8 x 4 B),
//! verified free across every bench/test binary in the workspace. (The
//! 453-460 run was rejected: 460 is taken by the
//! `src/ecs/core/iters/query/chunk_iter.rs` lib tests.)

// Phase X.E: opt-in low-variance allocator for A/B signal extraction.
// OFF by default (`cargo bench` keeps the production system heap for honest
// absolutes); `cargo bench --features bench-alloc` swaps in mimalloc. See
// docs/BENCHMARKING.md.
#[cfg(feature = "bench-alloc")]
#[global_allocator]
static BENCH_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use boyko_ecs::ecs::core::archetype::archetype::Archetype;
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use boyko_ecs::ecs::memory::arena::Arena;
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};

// --- Component registration ---

const IDS_3X192: [ComponentId; 3] = [ComponentId(450), ComponentId(451), ComponentId(452)];
const IDS_8X4: [ComponentId; 8] = [
    ComponentId(440),
    ComponentId(441),
    ComponentId(442),
    ComponentId(443),
    ComponentId(444),
    ComponentId(445),
    ComponentId(446),
    ComponentId(447),
];

fn register_bench_components() {
    // Pod-style payloads; `register_layout` only needs size/align, no
    // `Component` impl. The registry OnceLocks make this idempotent.
    macro_rules! reg_192 {
        ($id:expr, $t:ident) => {{
            #[repr(C)]
            struct $t([u64; 24]); // 192 B
            component_registry::register_layout::<$t>($id);
        }};
    }
    macro_rules! reg_4 {
        ($id:expr, $t:ident) => {{
            #[repr(C)]
            struct $t(u32); // 4 B
            component_registry::register_layout::<$t>($id);
        }};
    }
    reg_192!(450, Big450);
    reg_192!(451, Big451);
    reg_192!(452, Big452);
    reg_4!(440, Small440);
    reg_4!(441, Small441);
    reg_4!(442, Small442);
    reg_4!(443, Small443);
    reg_4!(444, Small444);
    reg_4!(445, Small445);
    reg_4!(446, Small446);
    reg_4!(447, Small447);
}

fn bench_archetype_create(c: &mut Criterion) {
    register_bench_components();

    let mut group = c.benchmark_group("archetype_create");

    // XI-B2 headline: 3 components x 192 B.
    group.bench_function("3x192B", |b| {
        b.iter_batched_ref(
            Arena::new,
            // The archetype keeps a vestigial never-dereferenced `*const
            // Arena` (X.I D8); returning it as the output keeps its drop
            // (pool VA-reservation release) outside the timed region.
            |arena| Archetype::create_by_ids(ArchetypeId(900), &IDS_3X192, arena),
            BatchSize::PerIteration,
        );
    });

    // Legacy 8 x u32 shape (companion to benches/archetype.rs).
    group.bench_function("8x4B", |b| {
        b.iter_batched_ref(
            Arena::new,
            |arena| Archetype::create_by_ids(ArchetypeId(901), &IDS_8X4, arena),
            BatchSize::PerIteration,
        );
    });

    group.finish();
}

criterion_group!(archetype_create, bench_archetype_create);
criterion_main!(archetype_create);
