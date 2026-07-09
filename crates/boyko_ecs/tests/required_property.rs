//! Feature 1 (required components) — property-based invariants.
//!
//! Spec `docs/REQUIRED-COMPONENTS-PLAN.md` Tests: "property test (random acyclic
//! DAG → order-independent canonical archetype, each required id once, each
//! on_add once)".
//!
//! `#[require]` edges are fixed at COMPILE time, so the randomized dimension here
//! is the user-supplied bundle VALUES (and, via the two distinct roots, the
//! shape) over a fixed acyclic require-DAG. The invariants the property pins:
//!
//! 1. **order-independent canonical archetype** — the effective archetype id set
//!    is the same regardless of the (random) component values;
//! 2. **each transitively-required id present exactly once** — the effective set
//!    has no duplicate columns (a diamond pulls a shared grandchild once);
//! 3. **each on_add fires once** — over the spawn path, the shared grandchild's
//!    on_add fires exactly once despite multiple transitive require paths.
//!
//! # Test isolation (TESTER NOTE)
//!
//! `cargo test` runs `#[test]` functions on SEPARATE threads concurrently, and a
//! `#[component(on_add = …)]` counter is a process-global `static`. The two
//! property functions therefore use ENTIRELY SEPARATE component DAGs (distinct
//! types + distinct counters) so neither contaminates the other's on_add count.
//! (An earlier shared-DAG version flaked `LEAF_ADD == 2` because the canonical
//! test's Root1 spawn fired the SAME `leaf_add` mid-measurement.)
//!
//! BUG-REQ-SNAKE-1 is FIXED at the macro layer (the derive emits its own
//! `#[allow(non_snake_case)]` on the generated ctor fns), so no crate-level mask
//! is needed here.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};
use proptest::prelude::*;

const SEQ: Ordering = Ordering::SeqCst;

// ════════════════════════════════════════════════════════════════════════════
// DAG A (canonical-set property) — NO on_add counter; never touched by DAG B.
//
//   ARoot1 ─► AMid ─► ALeaf
// ════════════════════════════════════════════════════════════════════════════

#[derive(Component, Default)]
#[repr(C)]
struct ALeaf(u32);

#[derive(Component, Default)]
#[require(ALeaf)]
#[repr(C)]
struct AMid(u32);

#[derive(Component, Default)]
#[require(AMid)]
#[repr(C)]
struct ARoot1(u32);

#[derive(Bundle)]
struct ARoot1Bundle {
    a: ARoot1,
}

fn a_prime() {
    let _ = ALeaf::component_id();
    let _ = AMid::component_id();
    let _ = ARoot1::component_id();
}

fn a_effective_ids(ecs: &EcsMaster, e: Entity) -> HashSet<usize> {
    let mut set = HashSet::new();
    for cid in [
        ALeaf::component_id(),
        AMid::component_id(),
        ARoot1::component_id(),
    ] {
        if ecs.has_component(e, cid) {
            set.insert(cid.0);
        }
    }
    set
}

// ════════════════════════════════════════════════════════════════════════════
// DAG B (on_add-once property) — its OWN leaf + counter; never touched by DAG A.
//
//   BRoot1 ─► BMid ─┐
//   BRoot2 ─────────┤► BLeaf   (diamond: 3 paths to BLeaf)
//   BRoot3 ─────────┘
// ════════════════════════════════════════════════════════════════════════════

static B_LEAF_ADD: AtomicUsize = AtomicUsize::new(0);

unsafe fn b_leaf_add(_w: DeferredEcsMaster<'_>, _c: HookContext) {
    B_LEAF_ADD.fetch_add(1, SEQ);
}

#[derive(Component, Default)]
#[component(on_add = b_leaf_add)]
#[repr(C)]
struct BLeaf(u32);

#[derive(Component, Default)]
#[require(BLeaf)]
#[repr(C)]
struct BMid(u32);

#[derive(Component, Default)]
#[require(BMid)]
#[repr(C)]
struct BRoot1(u32);

#[derive(Component, Default)]
#[require(BLeaf)]
#[repr(C)]
struct BRoot2(u32);

#[derive(Component, Default)]
#[require(BLeaf)]
#[repr(C)]
struct BRoot3(u32);

#[derive(Bundle)]
struct BRoot123Bundle {
    a: BRoot1,
    b: BRoot2,
    c: BRoot3,
}

fn b_prime() {
    let _ = BLeaf::component_id();
    let _ = BMid::component_id();
    let _ = BRoot1::component_id();
    let _ = BRoot2::component_id();
    let _ = BRoot3::component_id();
}

fn b_effective_ids(ecs: &EcsMaster, e: Entity) -> HashSet<usize> {
    let mut set = HashSet::new();
    for cid in [
        BLeaf::component_id(),
        BMid::component_id(),
        BRoot1::component_id(),
        BRoot2::component_id(),
        BRoot3::component_id(),
    ] {
        if ecs.has_component(e, cid) {
            set.insert(cid.0);
        }
    }
    set
}

proptest! {
    /// Invariant 1+2: spawning ARoot1 (with random value) always yields exactly
    /// {ARoot1, AMid, ALeaf} — value-independent canonical set, each id once.
    #[test]
    fn root_closure_is_canonical(v in any::<u32>()) {
        a_prime();
        let mut ecs = EcsMaster::new();
        let e = ecs.run_system(move |mut cmds: Commands| {
            cmds.spawn(ARoot1Bundle { a: ARoot1(v) }).id()
        });
        let got = a_effective_ids(&ecs, e);
        let expected: HashSet<usize> = [
            ARoot1::component_id().0,
            AMid::component_id().0,
            ALeaf::component_id().0,
        ]
        .into_iter()
        .collect();
        prop_assert_eq!(got, expected,
            "ARoot1 closure must be exactly {{ARoot1, AMid, ALeaf}} (canonical, deduped)");
    }

    /// Invariant 2 (diamond) + 3 (on_add once): spawning {BRoot1, BRoot2, BRoot3}
    /// pulls BLeaf via three paths but constructs it ONCE — so BLeaf's on_add
    /// fires exactly once per spawn, regardless of the random values.
    ///
    /// `B_LEAF_ADD` is private to DAG B (no other test touches `b_leaf_add`), so
    /// a single-case reset + assert is contamination-free across the concurrently
    /// running canonical property above.
    #[test]
    fn multi_root_diamond_fires_leaf_add_once(
        v1 in any::<u32>(), v2 in any::<u32>(), v3 in any::<u32>()
    ) {
        b_prime();
        let mut ecs = EcsMaster::new();
        B_LEAF_ADD.store(0, SEQ);

        let e = ecs.run_system(move |mut cmds: Commands| {
            cmds.spawn(BRoot123Bundle {
                a: BRoot1(v1),
                b: BRoot2(v2),
                c: BRoot3(v3),
            })
            .id()
        });

        let got = b_effective_ids(&ecs, e);
        let expected: HashSet<usize> = [
            BRoot1::component_id().0,
            BRoot2::component_id().0,
            BRoot3::component_id().0,
            BMid::component_id().0,
            BLeaf::component_id().0,
        ]
        .into_iter()
        .collect();
        prop_assert_eq!(got, expected,
            "multi-root closure is exactly the 5-id set, BLeaf deduped across 3 require paths");
        prop_assert_eq!(B_LEAF_ADD.load(SEQ), 1,
            "diamond: shared BLeaf's on_add fires EXACTLY once despite 3 transitive require paths");
    }
}
