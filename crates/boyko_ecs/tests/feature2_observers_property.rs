//! Feature 2 — property-based invariants for entity-targeted observers across
//! random attach / migrate / detach / despawn sequences.
//!
//! `proptest` generates the inputs; the invariants pin the
//! `docs/OBSERVERS-PLAN.md` property line ("random attach/detach/migrate/despawn
//! → no fire after detach, no double-fire, sticky-bit invariant"). Because an
//! `ObserverFn` is a non-capturing fn-ptr, each test owns a PRIVATE
//! process-global fire counter, a private runner, and a private component type
//! — the test binary runs `#[test]` fns in PARALLEL, so a shared counter would
//! race between the two property tests. Each `proptest` case resets its own
//! counter at entry and runs on a FRESH `EcsMaster`; `proptest` runs the cases
//! WITHIN one test sequentially, so the per-test counter is safe.
//!
//! Bounds are small (<= 48 entities, 8 cases) — `EcsMaster::new` plus the
//! per-entity store probes are not free, and the invariants do not need scale.

use std::sync::atomic::{AtomicUsize, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::component::observers::{ObserverContext, ObserverKind};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::{Bundle, Component};
use proptest::prelude::*;

const SEQ: Ordering = Ordering::SeqCst;

// ── Invariant 1 — its own type + counter ─────────────────────────────────────

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Base1(u32);
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Mig1(u32);
#[derive(Bundle)]
struct Mig1Bundle {
    m: Mig1,
}

static P1_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn p1_add(_w: DeferredEcsMaster<'_>, ctx: ObserverContext) {
    debug_assert_eq!(ctx.kind, ObserverKind::Add);
    P1_FIRES.fetch_add(1, SEQ);
}

// ── Invariant 2 — its own type + counter ─────────────────────────────────────

#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Base2(u32);
#[derive(Component, Clone, Copy)]
#[repr(C)]
struct Mig2(u32);
#[derive(Bundle)]
struct Mig2Bundle {
    m: Mig2,
}

static P2_FIRES: AtomicUsize = AtomicUsize::new(0);

unsafe fn p2_add(_w: DeferredEcsMaster<'_>, _ctx: ObserverContext) {
    P2_FIRES.fetch_add(1, SEQ);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 8, ..ProptestConfig::default() })]

    /// Invariant 1 — entity-targeting + no double-fire across migration.
    ///
    /// Spawn `n` entities in `{Base1}`; attach the `on_add(Mig1)` entity
    /// observer to a RANDOM subset; insert `Mig1` into ALL `n` (migrating each
    /// `{Base1}` -> `{Base1,Mig1}`). The fire count must equal the
    /// OBSERVED-subset size (entity-targeting: un-observed entities do not fire)
    /// and each observed entity fires exactly once (no double-fire from the
    /// sticky archetype bit being shared across the whole archetype).
    #[test]
    fn observed_subset_fires_exactly_once_each(
        observed in proptest::collection::vec(any::<bool>(), 1..=48),
    ) {
        P1_FIRES.store(0, SEQ);
        let n = observed.len();
        let expected = observed.iter().filter(|&&b| b).count();

        let mut ecs = EcsMaster::new();
        let base = ecs.create_archetype(&[Base1::component_id()]);
        let _ = Mig1::component_id();

        let mut ents = Vec::with_capacity(n);
        for i in 0..n {
            ents.push(ecs.spawn_one(base, Base1(i as u32)).expect("spawn"));
        }
        for (i, &obs) in observed.iter().enumerate() {
            if obs {
                ecs.observe_entity(ents[i], ObserverKind::Add, Mig1::component_id(), p1_add);
            }
        }
        for &e in &ents {
            ecs.run_system(move |mut cmds: Commands| {
                cmds.entity(e).insert(Mig1Bundle { m: Mig1(0) });
            });
        }

        prop_assert_eq!(
            P1_FIRES.load(SEQ),
            expected,
            "exactly the observed subset fired, once each (entity-targeting + no double-fire)"
        );
    }

    /// Invariant 2 — no fire after detach. Attach, detach via
    /// `remove_observer_any`, THEN migrate → zero fires (the sticky archetype
    /// bit stays set, but the per-entity store probe misses).
    #[test]
    fn no_fire_after_detach(
        n in 1usize..=24,
    ) {
        P2_FIRES.store(0, SEQ);
        let mut ecs = EcsMaster::new();
        let base = ecs.create_archetype(&[Base2::component_id()]);
        let _ = Mig2::component_id();

        let mut ents = Vec::with_capacity(n);
        let mut ids = Vec::with_capacity(n);
        for i in 0..n {
            let e = ecs.spawn_one(base, Base2(i as u32)).expect("spawn");
            let id = ecs.observe_entity(e, ObserverKind::Add, Mig2::component_id(), p2_add);
            ids.push(id);
            ents.push(e);
        }
        for id in ids {
            prop_assert!(ecs.remove_observer_any(id), "a registered observer is removable");
        }
        for &e in &ents {
            ecs.run_system(move |mut cmds: Commands| {
                cmds.entity(e).insert(Mig2Bundle { m: Mig2(0) });
            });
        }
        prop_assert_eq!(
            P2_FIRES.load(SEQ),
            0,
            "no observer fires after every observer was detached"
        );
    }
}
