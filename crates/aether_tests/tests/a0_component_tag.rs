//! Rung A0 end-to-end: an `aether!` block's components and tags are REAL engine components —
//! spawnable, queryable, hook-firing — through the same `boyko_macros` derive every hand-written
//! component uses (Decision A3: if the derive's behavior holds, Aether's does, because Aether
//! emits the surface the derive consumes).

use boyko_ecs::ecs::core::component::hooks::HookContext;
use boyko_ecs::ecs::core::component::hooks::deferred_master::DeferredEcsMaster;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;

use aether::aether;

/// The §3.1 `on_add` hook target — a REAL Phase-14a hook fn, so the test proves the hook PAIR
/// forwards, not merely that the attribute parses.
///
/// # Safety
/// The hook contract (`boyko_macros`' `component(on_add = …)` doc): called by the kernel during
/// the deferred hook flush with a live world and the added entity's context; this body only
/// bumps a counter.
unsafe fn count_add(_world: DeferredEcsMaster<'_>, _ctx: HookContext) {
    ADDS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

static ADDS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

aether! {
    component Health {
        current: f32,
        max: f32,
        on_add = count_add,
    }

    component Regen {
        per_second: f32,
    }

    tag Player;
    tag Stunned(bitset);
}

#[test]
fn aether_components_spawn_query_and_fire_hooks() {
    use boyko_ecs::ecs::core::component::component::Component as _;

    let mut world = EcsMaster::new();
    let arch = world.create_archetype(&[Health::component_id(), Regen::component_id()]);

    let before = ADDS.load(core::sync::atomic::Ordering::Relaxed);
    let e = world
        .spawn_two(arch, Health { current: 40.0, max: 100.0 }, Regen { per_second: 2.5 })
        .expect("spawn Health+Regen");

    // The Phase-14a on_add hook forwarded through Aether's `on_add = count_add` and fired
    // exactly once for the one Health added.
    assert_eq!(
        ADDS.load(core::sync::atomic::Ordering::Relaxed) - before,
        1,
        "the aether-forwarded on_add hook must fire once per added Health"
    );

    // The components are real storage-backed data, not just types that compile.
    let h = world.get_component::<Health>(e).expect("Health was spawned");
    assert_eq!((h.current, h.max), (40.0, 100.0));
    let r = world.get_component::<Regen>(e).expect("Regen was spawned");
    assert_eq!(r.per_second, 2.5);

    // The tags are spawnable ZST components; the bitset tag's EnableTag storage backend is the
    // derive's business and is pinned by the derive's own tests.
    let arch_p = world.create_archetype(&[Player::component_id()]);
    let ep = world.spawn_one(arch_p, Player).expect("spawn Player");
    assert!(world.get_component::<Player>(ep).is_some(), "the ZST tag is a real component");

    // The bitset tag lives in the EnableTag store, not in archetype storage — its API is
    // `enable`/`is_enabled` (O(1) toggle, no migration), never a spawn. The first draft of this
    // test spawned it like a plain ZST and the kernel refused — which is the storage backend
    // doing its job, kept here as the assertion.
    world.enable::<Stunned>(ep);
    assert!(world.is_enabled::<Stunned>(ep), "the bitset tag toggles on");
    world.disable::<Stunned>(ep);
    assert!(!world.is_enabled::<Stunned>(ep), "and off — no migration either way");
}
