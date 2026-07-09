//! Phase 8.5 Step 8 — Miri suite for the Static Bundle Cache.
//!
//! Run under `cargo +nightly miri test --test miri_phase8_5`. The tests
//! are NOT `#[cfg(miri)]`-gated; they run under the regular `cargo test`
//! harness too, as smoke tests for the Phase 8.5 cache paths. The
//! convention mirrors `tests/miri_phase8a.rs` and `tests/miri_phase8cd.rs`
//! (see those files' top comments for the rationale — Miri-friendly
//! tests stay cheap enough that the dev-profile cost is negligible).
//!
//! # Coverage
//!
//! 1. `miri_bundle_cached_archetype_id_no_ub` — repeated calls of
//!    `B::cached_archetype_id(world)` exercise the per-impl
//!    `OnceLock<BundleStaticInfo>` and the per-world boxed-array slot
//!    under Miri's Stacked / Tree Borrows. Hot-path Acquire load on a
//!    stable heap address (the `Box` slot).
//!
//! 2. `miri_bundle_cross_world_isolation_no_ub` — two distinct
//!    `EcsMaster`s exercise the per-world boxed-array independence (no
//!    cross-world provenance leakage).
//!
//! 3. `miri_bundle_first_spawn_then_repeated` — cold + hot paths back to
//!    back: the first spawn pays the `get_or_create_archetype` cost and
//!    `OnceLock::set`s the slot; subsequent spawns hit the hot path. Miri
//!    would surface any retag mistake on the cold→hot transition.
//!
//! 4. `miri_bundle_many_distinct_bundles_no_ub` — register many distinct
//!    Bundle types (10) to exercise the per-impl `OnceLock` storage and
//!    the per-world boxed-array slot range. 10 bundles is enough to walk
//!    several `BundleTypeId.0` values without paying the Miri cost of
//!    registering all 1024 (the `Box<[OnceLock<ArchetypeId>; 1024]>` slot
//!    array is allocated regardless — the test only exercises a subset
//!    of indices to bound Miri's wall-clock cost). Replaces the Round 2
//!    `cache_vec_growth_no_realloc_ub` test which is impossible by
//!    construction after the C1 boxed-array fix.
//!
//! # Component-slot range
//!
//! 320..=339 (20 slots — Step 8 Miri spec). Test 4 declares 10 bundles
//! each containing a single distinct component, consuming slots 320..=329.
//! Tests 1, 2, 3 use slots 330..=335 (≤ 6 components total). Leaves
//! 336..=339 free for future expansion in this file.

use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;

// ── Test 1 components ───────────────────────────────────────────────────────

const SLOT_M1_A: ComponentId = ComponentId(330);
const SLOT_M1_B: ComponentId = ComponentId(331);

#[repr(C)]
#[derive(Clone, Copy)]
struct M1A(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct M1B(u32);

impl Component for M1A {
    fn component_id() -> ComponentId {
        SLOT_M1_A
    }
}

impl Component for M1B {
    fn component_id() -> ComponentId {
        SLOT_M1_B
    }
}

fn register_m1() {
    register_layout::<M1A>(SLOT_M1_A.0);
    register_layout::<M1B>(SLOT_M1_B.0);
}

#[derive(Bundle)]
struct M1Bundle {
    a: M1A,
    b: M1B,
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 1: miri_bundle_cached_archetype_id_no_ub
// ─────────────────────────────────────────────────────────────────────────────

/// Repeated `B::cached_archetype_id(world)` under Miri.
///
/// Each call:
///  * Loads `B::bundle_type_id()` — Acquire on the per-impl `OnceLock`.
///  * Indexes `world.bundle_archetype_cache[id.0]` — pointer arithmetic
///    on the `Box`'s stable heap address.
///  * Loads `OnceLock::get()` on the slot — Acquire, no UB if the slot
///    is initialised.
///
/// Miri's Stacked Borrows / Tree Borrows would surface a retag mistake
/// on the `Box` slot access (the boxed-array layout pin guarantees the
/// inner pointer stays valid across the entire EcsMaster lifetime).
#[test]
fn miri_bundle_cached_archetype_id_no_ub() {
    register_m1();
    let mut ecs = EcsMaster::new();

    // 8 calls in a row — enough to drive the cold→hot transition and
    // several hot-path repeats. Each value is stored into a stack local
    // so the compiler cannot elide the load.
    let id_0 = M1Bundle::cached_archetype_id(&mut ecs);
    let id_1 = M1Bundle::cached_archetype_id(&mut ecs);
    let id_2 = M1Bundle::cached_archetype_id(&mut ecs);
    let id_3 = M1Bundle::cached_archetype_id(&mut ecs);
    let id_4 = M1Bundle::cached_archetype_id(&mut ecs);
    let id_5 = M1Bundle::cached_archetype_id(&mut ecs);
    let id_6 = M1Bundle::cached_archetype_id(&mut ecs);
    let id_7 = M1Bundle::cached_archetype_id(&mut ecs);

    assert_eq!(id_0, id_1, "cache hit must return identical id");
    assert_eq!(id_1, id_2);
    assert_eq!(id_2, id_3);
    assert_eq!(id_3, id_4);
    assert_eq!(id_4, id_5);
    assert_eq!(id_5, id_6);
    assert_eq!(id_6, id_7);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 2: miri_bundle_cross_world_isolation_no_ub
// ─────────────────────────────────────────────────────────────────────────────

/// Two `EcsMaster` instances, each calls `cached_archetype_id` for the
/// SAME Bundle. The cache slots live in distinct `Box` allocations, so
/// Miri's allocation tracking would surface any cross-allocation
/// provenance leak (e.g. if the global `BundleStaticInfo` slot accidentally
/// memoised a `ArchetypeId` from world A into world B).
#[test]
fn miri_bundle_cross_world_isolation_no_ub() {
    register_m1();

    let mut world_a = EcsMaster::new();
    let mut world_b = EcsMaster::new();

    let a1 = M1Bundle::cached_archetype_id(&mut world_a);
    let b1 = M1Bundle::cached_archetype_id(&mut world_b);
    let a2 = M1Bundle::cached_archetype_id(&mut world_a);
    let b2 = M1Bundle::cached_archetype_id(&mut world_b);

    // Each world is internally stable. The two worlds MAY coincidentally
    // produce the same numeric id (both started with archetype counter 0),
    // so the load-bearing invariant is per-world stability.
    assert_eq!(a1, a2, "world_a cache slot stable across calls");
    assert_eq!(b1, b2, "world_b cache slot stable across calls");
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 3: miri_bundle_first_spawn_then_repeated
// ─────────────────────────────────────────────────────────────────────────────

/// Full cold→hot path under Miri. First spawn drives:
///  * `B::cached_archetype_id` cold (OnceLock empty)
///  * `get_or_create_archetype` (archetype registry write)
///  * `OnceLock::set` (slot publish)
///  * `for_each_component_bytes` callback chain
///  * `create_entity` archetype-side memcpy
///
/// Subsequent spawns drive only the hot path: `OnceLock::get` on the
/// already-populated slot. Miri's Tree Borrows tracks the boxed-array
/// slot's tag across both transitions.
#[test]
fn miri_bundle_first_spawn_then_repeated() {
    register_m1();
    let mut ecs = EcsMaster::new();

    // Cold spawn — first call per (M1Bundle, ecs) pair.
    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(M1Bundle {
            a: M1A(1),
            b: M1B(2),
        });
    });
    assert_eq!(ecs.entity_count(), 1, "first cold spawn lands one entity");

    // Hot spawns — three more in succession, each via its own
    // `run_system` to also exercise the per-call FunctionSystem rebuild.
    for i in 0..3u32 {
        ecs.run_system(move |mut cmds: Commands| {
            cmds.spawn(M1Bundle {
                a: M1A(10 + i),
                b: M1B(20 + i),
            });
        });
    }
    assert_eq!(
        ecs.entity_count(),
        4,
        "three hot spawns land on the same archetype, total = 1 + 3 = 4"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 4: miri_bundle_many_distinct_bundles_no_ub
// ─────────────────────────────────────────────────────────────────────────────
//
// Ten distinct single-field bundles, each wrapping a distinct Component
// type. Exercises the per-impl `OnceLock<BundleStaticInfo>` for ten
// independent storage slots AND the per-world `bundle_archetype_cache`
// at ten distinct indices. Replaces the (impossible by construction)
// `cache_vec_growth_no_realloc_ub` test from Round 2 — the boxed-array
// design has no realloc path.

const SLOT_M4_0: ComponentId = ComponentId(320);
const SLOT_M4_1: ComponentId = ComponentId(321);
const SLOT_M4_2: ComponentId = ComponentId(322);
const SLOT_M4_3: ComponentId = ComponentId(323);
const SLOT_M4_4: ComponentId = ComponentId(324);
const SLOT_M4_5: ComponentId = ComponentId(325);
const SLOT_M4_6: ComponentId = ComponentId(326);
const SLOT_M4_7: ComponentId = ComponentId(327);
const SLOT_M4_8: ComponentId = ComponentId(328);
const SLOT_M4_9: ComponentId = ComponentId(329);

macro_rules! make_many_comp {
    ($Name:ident, $Slot:ident) => {
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct $Name(u32);

        impl Component for $Name {
            fn component_id() -> ComponentId {
                $Slot
            }
        }
    };
}

make_many_comp!(M4_0, SLOT_M4_0);
make_many_comp!(M4_1, SLOT_M4_1);
make_many_comp!(M4_2, SLOT_M4_2);
make_many_comp!(M4_3, SLOT_M4_3);
make_many_comp!(M4_4, SLOT_M4_4);
make_many_comp!(M4_5, SLOT_M4_5);
make_many_comp!(M4_6, SLOT_M4_6);
make_many_comp!(M4_7, SLOT_M4_7);
make_many_comp!(M4_8, SLOT_M4_8);
make_many_comp!(M4_9, SLOT_M4_9);

#[derive(Bundle)]
struct M4Bundle0 {
    x: M4_0,
}
#[derive(Bundle)]
struct M4Bundle1 {
    x: M4_1,
}
#[derive(Bundle)]
struct M4Bundle2 {
    x: M4_2,
}
#[derive(Bundle)]
struct M4Bundle3 {
    x: M4_3,
}
#[derive(Bundle)]
struct M4Bundle4 {
    x: M4_4,
}
#[derive(Bundle)]
struct M4Bundle5 {
    x: M4_5,
}
#[derive(Bundle)]
struct M4Bundle6 {
    x: M4_6,
}
#[derive(Bundle)]
struct M4Bundle7 {
    x: M4_7,
}
#[derive(Bundle)]
struct M4Bundle8 {
    x: M4_8,
}
#[derive(Bundle)]
struct M4Bundle9 {
    x: M4_9,
}

fn register_m4() {
    register_layout::<M4_0>(SLOT_M4_0.0);
    register_layout::<M4_1>(SLOT_M4_1.0);
    register_layout::<M4_2>(SLOT_M4_2.0);
    register_layout::<M4_3>(SLOT_M4_3.0);
    register_layout::<M4_4>(SLOT_M4_4.0);
    register_layout::<M4_5>(SLOT_M4_5.0);
    register_layout::<M4_6>(SLOT_M4_6.0);
    register_layout::<M4_7>(SLOT_M4_7.0);
    register_layout::<M4_8>(SLOT_M4_8.0);
    register_layout::<M4_9>(SLOT_M4_9.0);
}

#[test]
fn miri_bundle_many_distinct_bundles_no_ub() {
    register_m4();
    let mut ecs = EcsMaster::new();

    // Each bundle's `cached_archetype_id` exercises a distinct
    // `OnceLock<BundleStaticInfo>` (per-impl) and a distinct
    // `bundle_archetype_cache[id.0]` slot. Storing the ten ids in a stack
    // array keeps the loads observable.
    let ids = [
        M4Bundle0::cached_archetype_id(&mut ecs),
        M4Bundle1::cached_archetype_id(&mut ecs),
        M4Bundle2::cached_archetype_id(&mut ecs),
        M4Bundle3::cached_archetype_id(&mut ecs),
        M4Bundle4::cached_archetype_id(&mut ecs),
        M4Bundle5::cached_archetype_id(&mut ecs),
        M4Bundle6::cached_archetype_id(&mut ecs),
        M4Bundle7::cached_archetype_id(&mut ecs),
        M4Bundle8::cached_archetype_id(&mut ecs),
        M4Bundle9::cached_archetype_id(&mut ecs),
    ];

    // Each archetype is registered around a SINGLE distinct component, so
    // each id must be distinct from every other id in `ids`. Pairwise
    // assertion (10 × 9 / 2 = 45 comparisons) — cheap, exhaustive.
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            assert_ne!(
                ids[i], ids[j],
                "ten single-field bundles with distinct Component types must \
                 receive ten distinct ArchetypeIds (slot {} vs slot {})",
                i, j
            );
        }
    }

    // Hot-path re-reads stay stable.
    assert_eq!(M4Bundle0::cached_archetype_id(&mut ecs), ids[0]);
    assert_eq!(M4Bundle9::cached_archetype_id(&mut ecs), ids[9]);

    // Run a spawn for the first and last bundles to drive the apply path
    // through this many-bundle world under Miri.
    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(M4Bundle0 { x: M4_0(100) });
        cmds.spawn(M4Bundle9 { x: M4_9(900) });
    });
    assert_eq!(ecs.entity_count(), 2, "the two spawns landed");
}
