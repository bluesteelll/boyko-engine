//! Feature 1 — required components `#[require(B, C = ctor)]` integration tests.
//!
//! Spec: `docs/REQUIRED-COMPONENTS-PLAN.md`. This file pins the behavioral
//! contract of the required-components subsystem: declarative `#[require(...)]`
//! on a component auto-inserts every transitively-required ABSENT component on
//! spawn OR insert, constructed from a registered default / capture-free ctor,
//! deps-before-dependent, with the W1 conflict rule (first-DFS / direct-override)
//! and the W2 cycle break.
//!
//! # Why `Commands`, not the direct `spawn_one` API
//!
//! Required-component expansion is wired into the **bundle-resolution funnel**
//! (`cold_register_bundle_archetype` for spawn, `merged_archetype_id` for
//! insert), and the constructor pass lives in `SpawnAtCommand::apply` /
//! `migrate_entity_insert`. The direct `EcsMaster::spawn_one(archetype, value)`
//! API takes a pre-resolved archetype and bypasses the funnel — so every test
//! here spawns/inserts via `Commands` driven by `ecs.run_system(...)`, exactly
//! like the Phase-14a/14b firing-matrix tests.
//!
//! # Why `static` counters
//!
//! A `RequiredCtor` / `HookFn` / `ObserverFn` is a bare `unsafe fn` pointer — it
//! cannot capture. Each test therefore owns a private set of module-level
//! `static` counters plus its own component types, so concurrently-running tests
//! never observe one another's fires.
//!
//! # Component-id strategy
//!
//! Required tests use `#[derive(Component)]` whose ids are minted lazily from
//! the global atomic counter (`register_new`) — they never collide with the
//! explicit `register_layout` slots other test files use, nor with each other.
//!
//! BUG-REQ-SNAKE-1 is FIXED at the macro layer: the `#[require]` derive now
//! emits `#[allow(non_snake_case)]` on its generated `__require_ctor_<TypeName>`
//! fns, so no crate-level mask is needed here — `clippy -D warnings` is clean.

use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};

const SEQ: Ordering = Ordering::SeqCst;

// ════════════════════════════════════════════════════════════════════════════
// Test 1 — require_single: spawn A `#[require(B)]` ⇒ B present, B::default()
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct S1B(u32);
impl Default for S1B {
    fn default() -> Self {
        S1B(99)
    }
}

#[derive(Component)]
#[require(S1B)]
#[repr(C)]
struct S1A(u32);

#[derive(Bundle)]
struct S1ABundle {
    a: S1A,
}

#[test]
fn require_single_auto_inserts_b_with_default() {
    let mut ecs = EcsMaster::new();
    let _ = S1A::component_id();
    let _ = S1B::component_id();

    let e = ecs
        .run_system(|mut cmds: Commands| cmds.spawn(S1ABundle { a: S1A(1) }).id());

    assert!(
        ecs.has_component(e, S1A::component_id()),
        "the explicitly-spawned A is present"
    );
    assert!(
        ecs.has_component(e, S1B::component_id()),
        "require_single: B is auto-inserted by the constructor pass"
    );
    assert_eq!(
        ecs.get_component::<S1B>(e).copied(),
        Some(S1B::default()),
        "auto-inserted B holds B::default() (99)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 2 — require_recursive: A → B → D (transitive closure pulls D)
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug, Default)]
#[repr(C)]
struct R2D(u32);

#[derive(Component, Default)]
#[require(R2D)]
#[repr(C)]
struct R2B(u32);

#[derive(Component)]
#[require(R2B)]
#[repr(C)]
struct R2A(u32);

#[derive(Bundle)]
struct R2ABundle {
    a: R2A,
}

#[test]
fn require_recursive_pulls_transitive_grandchild() {
    let mut ecs = EcsMaster::new();
    let _ = R2A::component_id();
    let _ = R2B::component_id();
    let _ = R2D::component_id();

    let e = ecs.run_system(|mut cmds: Commands| cmds.spawn(R2ABundle { a: R2A(7) }).id());

    assert!(ecs.has_component(e, R2A::component_id()), "A present");
    assert!(
        ecs.has_component(e, R2B::component_id()),
        "require_recursive: direct-required B present"
    );
    assert!(
        ecs.has_component(e, R2D::component_id()),
        "require_recursive: transitively-required D present (A → B → D)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 3 — require_diamond: A requires B,C; both require D ⇒ D constructed ONCE
// ════════════════════════════════════════════════════════════════════════════

/// D's ctor count — proves the diamond constructs D exactly once (no double
/// construct). A `Default` impl on a real type would not be enough to count
/// constructions, so D carries a counting custom ctor via `#[require(D3D = ...)]`
/// is NOT how the diamond pulls D (B and C pull D via plain `#[require(D3D)]`,
/// using `Default`). To count we give D3D a `Default` that bumps a static.
static D3_DEFAULT_COUNT: AtomicUsize = AtomicUsize::new(0);

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct D3D(u32);
impl Default for D3D {
    fn default() -> Self {
        D3_DEFAULT_COUNT.fetch_add(1, SEQ);
        D3D(0)
    }
}

#[derive(Component, Default)]
#[require(D3D)]
#[repr(C)]
struct D3B(u32);

#[derive(Component, Default)]
#[require(D3D)]
#[repr(C)]
struct D3C(u32);

#[derive(Component)]
#[require(D3B, D3C)]
#[repr(C)]
struct D3A(u32);

#[derive(Bundle)]
struct D3ABundle {
    a: D3A,
}

#[test]
fn require_diamond_constructs_shared_grandchild_once() {
    let mut ecs = EcsMaster::new();
    let _ = D3A::component_id();
    let _ = D3B::component_id();
    let _ = D3C::component_id();
    let _ = D3D::component_id();
    D3_DEFAULT_COUNT.store(0, SEQ);

    let e = ecs.run_system(|mut cmds: Commands| cmds.spawn(D3ABundle { a: D3A(1) }).id());

    assert!(ecs.has_component(e, D3B::component_id()), "B present");
    assert!(ecs.has_component(e, D3C::component_id()), "C present");
    assert!(ecs.has_component(e, D3D::component_id()), "shared D present");
    assert_eq!(
        D3_DEFAULT_COUNT.load(SEQ),
        1,
        "require_diamond: D constructed EXACTLY once despite two require paths (B,C → D)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 4 — require_conflict_direct_wins: A requires B(ctor1) + C; C requires
//          B(ctor2). A's DIRECT B declaration OVERRIDES C's inherited one (W1).
// ════════════════════════════════════════════════════════════════════════════

// Default is never used (both require edges supply explicit ctors); derive it
// to satisfy the bare `#[require]` form's `T: Default` bound without a manual
// clippy::derivable_impls hit.
#[derive(Component, Clone, Copy, PartialEq, Debug, Default)]
#[repr(C)]
struct CD4B(u32);

// C requires B via ctor2 → CD4B(222).
#[derive(Component, Default)]
#[require(CD4B = CD4B(222))]
#[repr(C)]
struct CD4C(u32);

// A directly requires B via ctor1 → CD4B(111), AND requires C (which pulls B
// via ctor2). The DIRECT declaration on A must win → CD4B(111).
#[derive(Component)]
#[require(CD4B = CD4B(111), CD4C)]
#[repr(C)]
struct CD4A(u32);

#[derive(Bundle)]
struct CD4ABundle {
    a: CD4A,
}

#[test]
fn require_conflict_direct_declaration_wins() {
    let mut ecs = EcsMaster::new();
    let _ = CD4A::component_id();
    let _ = CD4B::component_id();
    let _ = CD4C::component_id();

    let e = ecs.run_system(|mut cmds: Commands| cmds.spawn(CD4ABundle { a: CD4A(1) }).id());

    assert!(ecs.has_component(e, CD4C::component_id()), "C present");
    assert_eq!(
        ecs.get_component::<CD4B>(e).copied(),
        Some(CD4B(111)),
        "W1 direct rule: A's DIRECT #[require(B = ctor1)] overrides C's inherited #[require(B = ctor2)]"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 5 — require_conflict_sibling_first_dfs (W1): two siblings both
//          transitively pull grandchild D with DIFFERENT ctors, neither directly
//          declares D ⇒ first-DFS (earlier-listed sibling's) ctor wins.
// ════════════════════════════════════════════════════════════════════════════

// Default is never used (both sibling edges supply explicit ctors); derive it.
#[derive(Component, Clone, Copy, PartialEq, Debug, Default)]
#[repr(C)]
struct SD5D(u32);

// Sibling 1 pulls D via ctor → SD5D(1).
#[derive(Component, Default)]
#[require(SD5D = SD5D(1))]
#[repr(C)]
struct SD5B(u32);

// Sibling 2 pulls D via a DIFFERENT ctor → SD5D(2).
#[derive(Component, Default)]
#[require(SD5D = SD5D(2))]
#[repr(C)]
struct SD5C(u32);

// A requires B then C (B listed first). Neither A nor its direct requires
// declares D directly — D is only reachable transitively through B and C. The
// first-DFS-reached ctor (B's, SD5D(1)) wins.
#[derive(Component)]
#[require(SD5B, SD5C)]
#[repr(C)]
struct SD5A(u32);

#[derive(Bundle)]
struct SD5ABundle {
    a: SD5A,
}

#[test]
fn require_conflict_sibling_first_dfs_wins() {
    let mut ecs = EcsMaster::new();
    let _ = SD5A::component_id();
    let _ = SD5B::component_id();
    let _ = SD5C::component_id();
    let _ = SD5D::component_id();

    let e = ecs.run_system(|mut cmds: Commands| cmds.spawn(SD5ABundle { a: SD5A(1) }).id());

    assert_eq!(
        ecs.get_component::<SD5D>(e).copied(),
        Some(SD5D(1)),
        "W1 inherited rule: first-DFS sibling (B, listed before C) wins the shared grandchild D's ctor"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 6 — require_custom_ctor: `#[require(C = C(42))]` on a NO-Default type via
//          a capture-free expr (the no-Default escape hatch).
// ════════════════════════════════════════════════════════════════════════════

// NOTE: no `Default` impl — must be constructed via the `= expr` ctor only.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct CC6NoDefault(u32);

#[derive(Component)]
#[require(CC6NoDefault = CC6NoDefault(42))]
#[repr(C)]
struct CC6A(u32);

#[derive(Bundle)]
struct CC6ABundle {
    a: CC6A,
}

#[test]
fn require_custom_ctor_on_no_default_type() {
    let mut ecs = EcsMaster::new();
    let _ = CC6A::component_id();
    let _ = CC6NoDefault::component_id();

    let e = ecs.run_system(|mut cmds: Commands| cmds.spawn(CC6ABundle { a: CC6A(1) }).id());

    assert_eq!(
        ecs.get_component::<CC6NoDefault>(e).copied(),
        Some(CC6NoDefault(42)),
        "require_custom_ctor: a no-Default type is constructed via the capture-free `= expr` ctor"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 7 — require_does_not_overwrite_present: spawn A + B (B explicit
//          non-default) ⇒ the explicit B is KEPT (present ⇒ skip, no overwrite).
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct OW7B(u32);
impl Default for OW7B {
    fn default() -> Self {
        OW7B(999)
    }
}

#[derive(Component)]
#[require(OW7B)]
#[repr(C)]
struct OW7A(u32);

// A bundle that supplies BOTH A and an EXPLICIT non-default B.
#[derive(Bundle)]
struct OW7Bundle {
    a: OW7A,
    b: OW7B,
}

#[test]
fn require_does_not_overwrite_present_component() {
    let mut ecs = EcsMaster::new();
    let _ = OW7A::component_id();
    let _ = OW7B::component_id();

    let e = ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(OW7Bundle {
            a: OW7A(1),
            b: OW7B(7),
        })
        .id()
    });

    assert_eq!(
        ecs.get_component::<OW7B>(e).copied(),
        Some(OW7B(7)),
        "present ⇒ skip: the explicit B(7) is KEPT, NOT overwritten by B::default() (999)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test 8 — spawn_a_requires_b_fires_on_add: spawn path fires B's on_add (the
//          SpawnAtCommand::apply path iterates the FULL archetype → required
//          rows fire automatically).
// ════════════════════════════════════════════════════════════════════════════

use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;

static SP8_B_ADD: AtomicUsize = AtomicUsize::new(0);
static SP8_B_INSERT: AtomicUsize = AtomicUsize::new(0);

unsafe fn sp8_b_add(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    SP8_B_ADD.fetch_add(1, SEQ);
}
unsafe fn sp8_b_insert(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    SP8_B_INSERT.fetch_add(1, SEQ);
}

#[derive(Component, Default)]
#[component(on_add = sp8_b_add, on_insert = sp8_b_insert)]
#[repr(C)]
struct SP8B(u32);

#[derive(Component)]
#[require(SP8B)]
#[repr(C)]
struct SP8A(u32);

#[derive(Bundle)]
struct SP8ABundle {
    a: SP8A,
}

#[test]
fn spawn_a_requires_b_fires_b_on_add_once() {
    let mut ecs = EcsMaster::new();
    let _ = SP8A::component_id();
    let _ = SP8B::component_id();
    SP8_B_ADD.store(0, SEQ);
    SP8_B_INSERT.store(0, SEQ);

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(SP8ABundle { a: SP8A(1) });
    });

    assert_eq!(
        SP8_B_ADD.load(SEQ),
        1,
        "spawn path: required B's on_add fires exactly once (SpawnAtCommand iterates the FULL archetype)"
    );
    assert_eq!(
        SP8_B_INSERT.load(SEQ),
        1,
        "spawn path: required B's on_insert fires exactly once"
    );
}
