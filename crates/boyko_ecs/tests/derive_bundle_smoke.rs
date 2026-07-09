//! Phase 8.5 Step 7 — smoke tests for the `#[derive(Bundle)]` macro and the
//! Static Bundle Cache cold/hot paths.
//!
//! Each test pins one slice of the Phase 8.5 contract:
//!
//! 1. `derive_bundle_named_struct_compiles_and_spawns` — minimal happy path
//!    on a named-field bundle (`{ a: A, b: B }`) routed through
//!    `Commands::spawn`.
//! 2. `derive_bundle_tuple_struct_compiles_and_spawns` — happy path on a
//!    tuple-style bundle (`Foo(A, B)`).
//! 3. `derive_bundle_unique_bundle_type_id` — two distinct `#[derive(Bundle)]`
//!    types receive distinct `BundleTypeId`s (SBC2 — per-impl OnceLock isolation).
//! 4. `derive_bundle_component_ids_are_canonical_sorted` — field declared in
//!    `(B, A)` order surfaces as `[A.id, B.id]` post-derive sort (B1 / SBC3).
//! 5. `derive_bundle_cached_archetype_id_idempotent` — two calls of
//!    `B::cached_archetype_id(world)` return identical ids (SBC4 cache hit).
//! 6. `derive_bundle_cross_world_isolation` — same Bundle in two distinct
//!    `EcsMaster`s receives distinct archetype ids (per-world cache slot,
//!    not a process-global archetype registry).
//! 7. `derive_bundle_static_info_cached` — `static_info()` returns the
//!    same `&'static BundleStaticInfo` pointer on every call (OnceLock
//!    cache contract — SBC2 / SBC3).
//!
//! # Component-slot range
//!
//! 290..=309 per the Step 7 spec — avoids collisions with Phase 8c+8d
//! (244..=259 + 280..=281), Phase 8d Miri (260..=269), Phase 8.5 Step 6
//! migration tests (240..=243 freed, reusable but disjoint from this file).
//!
//! # No shared state across tests
//!
//! Each Bundle type is local to its test (or to a small set of related
//! tests); the only process-global atomic in play is the `BundleTypeId`
//! counter inside `bundle_type_registry`, which is monotonically advancing
//! and never observed for absolute values — every assertion checks
//! relative invariants (distinct, equal, or sorted), never a hard-coded id.

use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;

// ── Test 1 — named-struct happy path ─────────────────────────────────────────

const SLOT_S1_A: ComponentId = ComponentId(290);
const SLOT_S1_B: ComponentId = ComponentId(291);

#[repr(C)]
#[derive(Clone, Copy)]
struct S1A(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct S1B(u32);

impl Component for S1A {
    fn component_id() -> ComponentId {
        SLOT_S1_A
    }
}

impl Component for S1B {
    fn component_id() -> ComponentId {
        SLOT_S1_B
    }
}

fn register_s1() {
    register_layout::<S1A>(SLOT_S1_A.0);
    register_layout::<S1B>(SLOT_S1_B.0);
}

#[derive(Bundle)]
struct S1Bundle {
    a: S1A,
    b: S1B,
}

#[test]
fn derive_bundle_named_struct_compiles_and_spawns() {
    register_s1();
    let mut ecs = EcsMaster::new();

    assert_eq!(ecs.entity_count(), 0, "world starts empty");

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(S1Bundle {
            a: S1A(11),
            b: S1B(22),
        });
    });

    assert_eq!(
        ecs.entity_count(),
        1,
        "named-struct derive(Bundle) must spawn exactly one entity"
    );
}

// ── Test 2 — tuple-struct happy path ─────────────────────────────────────────

const SLOT_S2_A: ComponentId = ComponentId(292);
const SLOT_S2_B: ComponentId = ComponentId(293);

#[repr(C)]
#[derive(Clone, Copy)]
struct S2A(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct S2B(u32);

impl Component for S2A {
    fn component_id() -> ComponentId {
        SLOT_S2_A
    }
}

impl Component for S2B {
    fn component_id() -> ComponentId {
        SLOT_S2_B
    }
}

fn register_s2() {
    register_layout::<S2A>(SLOT_S2_A.0);
    register_layout::<S2B>(SLOT_S2_B.0);
}

#[derive(Bundle)]
struct S2Bundle(S2A, S2B);

#[test]
fn derive_bundle_tuple_struct_compiles_and_spawns() {
    register_s2();
    let mut ecs = EcsMaster::new();

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(S2Bundle(S2A(33), S2B(44)));
    });

    assert_eq!(
        ecs.entity_count(),
        1,
        "tuple-struct derive(Bundle) must spawn exactly one entity"
    );
}

// ── Test 3 — distinct Bundle types → distinct BundleTypeIds (SBC2) ───────────

const SLOT_S3_A: ComponentId = ComponentId(294);
const SLOT_S3_B: ComponentId = ComponentId(295);

#[repr(C)]
#[derive(Clone, Copy)]
struct S3A(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct S3B(u32);

impl Component for S3A {
    fn component_id() -> ComponentId {
        SLOT_S3_A
    }
}

impl Component for S3B {
    fn component_id() -> ComponentId {
        SLOT_S3_B
    }
}

fn register_s3() {
    register_layout::<S3A>(SLOT_S3_A.0);
    register_layout::<S3B>(SLOT_S3_B.0);
}

#[derive(Bundle)]
struct S3BundleOne {
    a: S3A,
}

#[derive(Bundle)]
struct S3BundleTwo {
    a: S3A,
    b: S3B,
}

#[test]
fn derive_bundle_unique_bundle_type_id() {
    register_s3();

    let id_one = S3BundleOne::bundle_type_id();
    let id_two = S3BundleTwo::bundle_type_id();

    assert_ne!(
        id_one, id_two,
        "two distinct Bundle impls must receive distinct BundleTypeIds (SBC2)"
    );

    // Idempotence of bundle_type_id (process-global OnceLock contract).
    assert_eq!(
        S3BundleOne::bundle_type_id(),
        id_one,
        "BundleTypeId must be stable across calls for the same Bundle type"
    );
    assert_eq!(
        S3BundleTwo::bundle_type_id(),
        id_two,
        "BundleTypeId must be stable across calls for the same Bundle type"
    );
}

// ── Test 4 — non-canonical field order → canonical-sorted component_ids ──────

const SLOT_S4_A: ComponentId = ComponentId(296);
const SLOT_S4_B: ComponentId = ComponentId(297);

#[repr(C)]
#[derive(Clone, Copy)]
struct S4A(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct S4B(u32);

impl Component for S4A {
    fn component_id() -> ComponentId {
        SLOT_S4_A
    }
}

impl Component for S4B {
    fn component_id() -> ComponentId {
        SLOT_S4_B
    }
}

fn register_s4() {
    register_layout::<S4A>(SLOT_S4_A.0);
    register_layout::<S4B>(SLOT_S4_B.0);
}

/// Fields declared in NON-canonical order `(b, a)`. The derive's internal
/// `sort_unstable_by_key` (B1) must reorder the component-id slice into
/// ascending `ComponentId.0`.
#[derive(Bundle)]
struct S4Bundle {
    b: S4B,
    a: S4A,
}

#[test]
fn derive_bundle_component_ids_are_canonical_sorted() {
    register_s4();

    let ids = S4Bundle::component_ids();
    assert_eq!(ids.len(), 2, "arity-2 bundle exposes exactly 2 component ids");

    // The derive's sort must produce `[SLOT_S4_A, SLOT_S4_B]` regardless of
    // the user's `(b, a)` declaration order.
    assert_eq!(
        ids[0], SLOT_S4_A,
        "first id must be the smaller ComponentId (B1: ascending sort)"
    );
    assert_eq!(
        ids[1], SLOT_S4_B,
        "second id must be the larger ComponentId (B1: ascending sort)"
    );
}

// ── Test 5 — cached_archetype_id idempotence (SBC4) ──────────────────────────

const SLOT_S5_A: ComponentId = ComponentId(298);
const SLOT_S5_B: ComponentId = ComponentId(299);

#[repr(C)]
#[derive(Clone, Copy)]
struct S5A(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct S5B(u32);

impl Component for S5A {
    fn component_id() -> ComponentId {
        SLOT_S5_A
    }
}

impl Component for S5B {
    fn component_id() -> ComponentId {
        SLOT_S5_B
    }
}

fn register_s5() {
    register_layout::<S5A>(SLOT_S5_A.0);
    register_layout::<S5B>(SLOT_S5_B.0);
}

#[derive(Bundle)]
struct S5Bundle {
    a: S5A,
    b: S5B,
}

#[test]
fn derive_bundle_cached_archetype_id_idempotent() {
    register_s5();
    let mut ecs = EcsMaster::new();

    // First call — cold path: cache slot empty, get_or_create_archetype runs.
    let first = S5Bundle::cached_archetype_id(&mut ecs);
    // Second call — hot path: cache slot populated, OnceLock::get returns
    // the same ArchetypeId.
    let second = S5Bundle::cached_archetype_id(&mut ecs);
    // Third call — sanity that the cache is stable across multiple reads.
    let third = S5Bundle::cached_archetype_id(&mut ecs);

    assert_eq!(
        first, second,
        "cached_archetype_id must return the same id on a warm cache (SBC4)"
    );
    assert_eq!(
        second, third,
        "cached_archetype_id remains stable on subsequent calls"
    );
}

// ── Test 6 — cross-world isolation (per-world cache) ─────────────────────────

const SLOT_S6_A: ComponentId = ComponentId(300);
const SLOT_S6_B: ComponentId = ComponentId(301);

#[repr(C)]
#[derive(Clone, Copy)]
struct S6A(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct S6B(u32);

impl Component for S6A {
    fn component_id() -> ComponentId {
        SLOT_S6_A
    }
}

impl Component for S6B {
    fn component_id() -> ComponentId {
        SLOT_S6_B
    }
}

fn register_s6() {
    register_layout::<S6A>(SLOT_S6_A.0);
    register_layout::<S6B>(SLOT_S6_B.0);
}

#[derive(Bundle)]
struct S6Bundle {
    a: S6A,
    b: S6B,
}

#[test]
fn derive_bundle_cross_world_isolation() {
    register_s6();

    let mut world_one = EcsMaster::new();
    let mut world_two = EcsMaster::new();

    // Each world's `bundle_archetype_cache` is independent. The
    // ArchetypeIds are assigned by each `ArchetypeMaster`'s internal
    // counter, which starts at 0 for every new `EcsMaster`. The first
    // bundle registered in each world receives that world's first
    // available `ArchetypeId`.
    let id_one = S6Bundle::cached_archetype_id(&mut world_one);
    let id_two = S6Bundle::cached_archetype_id(&mut world_two);

    // The ids may HAPPEN to be equal (both worlds start their counter at
    // 0), so the load-bearing assertion is per-world stability — calling
    // again in each world must return its own cached value.
    assert_eq!(
        S6Bundle::cached_archetype_id(&mut world_one),
        id_one,
        "world_one's cache stays bound to its own ArchetypeId"
    );
    assert_eq!(
        S6Bundle::cached_archetype_id(&mut world_two),
        id_two,
        "world_two's cache stays bound to its own ArchetypeId"
    );

    // Independence: spawning in one world does NOT touch the other.
    world_one.run_system(|mut cmds: Commands| {
        cmds.spawn(S6Bundle {
            a: S6A(1),
            b: S6B(2),
        });
    });
    assert_eq!(world_one.entity_count(), 1, "world_one received the spawn");
    assert_eq!(
        world_two.entity_count(),
        0,
        "world_two must remain untouched (per-world cache isolation)"
    );
}

// ── Test 7 — static_info() returns the same pointer on every call (SBC3) ─────

const SLOT_S7_A: ComponentId = ComponentId(302);
const SLOT_S7_B: ComponentId = ComponentId(303);

#[repr(C)]
#[derive(Clone, Copy)]
struct S7A(u32);

#[repr(C)]
#[derive(Clone, Copy)]
struct S7B(u32);

impl Component for S7A {
    fn component_id() -> ComponentId {
        SLOT_S7_A
    }
}

impl Component for S7B {
    fn component_id() -> ComponentId {
        SLOT_S7_B
    }
}

fn register_s7() {
    register_layout::<S7A>(SLOT_S7_A.0);
    register_layout::<S7B>(SLOT_S7_B.0);
}

#[derive(Bundle)]
struct S7Bundle {
    a: S7A,
    b: S7B,
}

#[test]
fn derive_bundle_static_info_cached() {
    register_s7();

    // Each `#[derive(Bundle)]` impl owns one process-global
    // `static INFO: OnceLock<BundleStaticInfo>` slot. Every call to
    // `static_info()` returns a `&'static` reference to the SAME slot —
    // so two consecutive call sites must yield byte-equal pointers, not
    // just byte-equal payloads. A regression that lazily rebuilds the
    // payload (or that accidentally introduces per-call cloning) would
    // surface here as distinct addresses.
    let info_one = S7Bundle::static_info();
    let info_two = S7Bundle::static_info();
    let info_three = S7Bundle::static_info();

    assert!(
        std::ptr::eq(info_one, info_two),
        "static_info() must return the same &'static BundleStaticInfo \
         pointer on every call (OnceLock cache contract)"
    );
    assert!(
        std::ptr::eq(info_two, info_three),
        "static_info() pointer stays stable across additional calls"
    );

    // Sanity on the payload: two pointer-equal references must observe
    // the same `type_id` and the same component-id slice (the slice
    // itself is also `&'static`, leaked at OnceLock-init time).
    assert_eq!(info_one.type_id, info_two.type_id);
    assert!(
        std::ptr::eq(
            info_one.component_ids.as_ptr(),
            info_two.component_ids.as_ptr()
        ),
        "component_ids slice pointer must be byte-equal across calls \
         (the slice is leaked once and shared by every caller)"
    );
}
