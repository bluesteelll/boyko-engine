//! Phase 11 — `EntityCommands` chaining + despawn integration tests.
//!
//! Exercises the FULL apply path:
//!
//! ```text
//! system body --Commands::spawn(b).id()-->  pre-allocated Entity
//!                                                |
//!                            FunctionSystem::apply (APP3)
//!                                                v
//!                       SpawnAtCommand::apply --> create_entity_at
//!                          (Insert/Remove/Despawn similarly)
//! ```
//!
//! Each test pins one slice of the contract:
//!
//! 1. `commands_spawn_returns_entity_handle` — `cmds.spawn(b).id()` returns
//!    a freshly-reserved `Entity` before apply (EC2 / EC13).
//! 2. `entity_commands_insert_migrates_archetype` — `.insert(B)` on an
//!    existing entity migrates the archetype (plan §7.2).
//! 3. `entity_commands_insert_replace_in_place_fast_path` —
//!    `.insert(SameBundleAsAlreadyHosted)` exercises the replace-in-place
//!    fast path (plan §7.4 W-N1).
//! 4. `entity_commands_remove_migrates_to_smaller_archetype` —
//!    `.remove::<C>()` migrates to source \\ {C}.
//! 5. `entity_commands_despawn_kills_entity` — `.despawn()` on an existing
//!    entity removes it from the world.
//! 6. `commands_despawn_convenience_kills_entity` —
//!    `Commands::despawn(id)` is equivalent.
//! 7. `commands_entity_handle_for_existing` — `Commands::entity(id)` wraps
//!    an existing entity with a chainable handle.
//! 8. `chained_spawn_insert_id` — full `.spawn(a).insert(b).id()` pipeline.
//! 9. `reserve_entity_yields_distinct_ids` — `Commands::reserve_entity`
//!    monotonically advances the atomic counter.
//!
//! # Component-slot range
//!
//! 600..=619, away from the existing Phase 8.5 / 8c+8d / 10 ranges.

// Test oracle model: the std collections / `Arc<Mutex<_>>` / `Rc` in this suite are
// the REFERENCE implementations and cross-thread observation channels the engine's
// VM-native structures (ComponentPool columns, BitSet/BitMask, SparseMap, the dense
// stores) are differentially verified against - never engine data itself.
// An integration-test target: compiled out of every shipping build.
#![allow(clippy::disallowed_types)]

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;

// ── Component types ──────────────────────────────────────────────────────────

// Phase 11 test ComponentId slots: 411..=413.
// MAX_COMPONENTS = 512 hard cap. Phase 10 used 380-410; Phase 11 picks the next contiguous range.
const SLOT_POS: ComponentId = ComponentId(411);
const SLOT_VEL: ComponentId = ComponentId(412);
const SLOT_HEALTH: ComponentId = ComponentId(413);

#[repr(C)]
#[derive(Clone, Copy)]
struct Pos { x: f32, y: f32, z: f32 }

#[repr(C)]
#[derive(Clone, Copy)]
struct Vel { vx: f32, vy: f32, vz: f32 }

#[repr(C)]
#[derive(Clone, Copy)]
struct Health(i32);

impl Component for Pos {
    fn component_id() -> ComponentId { SLOT_POS }
}
impl Component for Vel {
    fn component_id() -> ComponentId { SLOT_VEL }
}
impl Component for Health {
    fn component_id() -> ComponentId { SLOT_HEALTH }
}

fn register_all() {
    register_layout::<Pos>(SLOT_POS.0);
    register_layout::<Vel>(SLOT_VEL.0);
    register_layout::<Health>(SLOT_HEALTH.0);
}

// ── Bundles ──────────────────────────────────────────────────────────────────

#[derive(Bundle)]
struct PosBundle { p: Pos }

#[derive(Bundle)]
struct PosVelBundle { p: Pos, v: Vel }

#[derive(Bundle)]
struct HealthBundle { h: Health }

#[derive(Bundle)]
struct VelBundle { v: Vel }

// ── Test 1 ───────────────────────────────────────────────────────────────────

/// `Commands::spawn(b)` returns an `EntityCommands<'_, '_>` whose `id()`
/// produces a freshly-reserved `Entity`. EC2 / EC13: the entity is real
/// (atomic counter minted it) but query-invisible until apply.
#[test]
fn commands_spawn_returns_entity_handle() {
    register_all();
    let mut ecs = EcsMaster::new();

    // We can't easily smuggle the id out of the closure (Send + Sync
    // bound), so probe through an `Arc<AtomicU64>` and verify post-apply
    // that the entity is alive.
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    let observed = Arc::new(AtomicU64::new(u64::MAX));
    let probe = Arc::clone(&observed);

    ecs.run_system(move |mut cmds: Commands| {
        let id = cmds.spawn(PosBundle { p: Pos { x: 1.0, y: 2.0, z: 3.0 } }).id();
        probe.store(id.id().0 as u64, Ordering::Relaxed);
    });

    let captured = observed.load(Ordering::Relaxed);
    assert_ne!(captured, u64::MAX, "spawn().id() must surface a real ID");
    assert_eq!(ecs.entity_count(), 1, "apply must register exactly one entity");
}

// ── Test 2 ───────────────────────────────────────────────────────────────────

/// `EntityCommands::insert(B)` migrates the entity to the source ∪ B
/// archetype when B introduces new components.
#[test]
fn entity_commands_insert_migrates_archetype() {
    register_all();
    let mut ecs = EcsMaster::new();

    // Step A: spawn an entity with just Pos.
    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(PosBundle { p: Pos { x: 1.0, y: 2.0, z: 3.0 } });
    });
    assert_eq!(ecs.entity_count(), 1);

    let entity = ecs.iter_entities().next().expect("one entity exists");
    let src_arch_id = ecs.get_entity_archetype_id(entity).expect("src archetype");

    // Step B: insert Vel via EntityCommands handle.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(entity).insert(VelBundle { v: Vel { vx: 0.1, vy: 0.2, vz: 0.3 } });
    });

    // Entity remains alive; archetype changed to include Vel.
    assert!(ecs.has_entity(entity), "entity still alive after insert");
    let new_arch_id = ecs.get_entity_archetype_id(entity).expect("new archetype");
    assert_ne!(src_arch_id, new_arch_id, "archetype migrated on insert");
    assert!(ecs.has_component(entity, SLOT_POS));
    assert!(ecs.has_component(entity, SLOT_VEL));
}

// ── Test 3 ───────────────────────────────────────────────────────────────────

/// `EntityCommands::insert(B)` where B's component_ids ⊆ source.component_ids
/// hits the replace-in-place fast path (plan §7.4 W-N1) — same archetype,
/// new bytes overwrite the old.
#[test]
fn entity_commands_insert_replace_in_place_fast_path() {
    register_all();
    let mut ecs = EcsMaster::new();

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(PosBundle { p: Pos { x: 100.0, y: 200.0, z: 300.0 } });
    });
    let entity = ecs.iter_entities().next().expect("one entity");
    let arch_before = ecs.get_entity_archetype_id(entity).expect("arch");

    // Insert PosBundle again → replace-in-place (target == source ⇒
    // canonicalization invariant ⇒ bundle ⊆ source).
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(entity).insert(PosBundle { p: Pos { x: 1.0, y: 2.0, z: 3.0 } });
    });

    let arch_after = ecs.get_entity_archetype_id(entity).expect("arch");
    assert_eq!(arch_before, arch_after, "in-place replace must NOT migrate");
    let pos = ecs.get_component::<Pos>(entity).expect("Pos still present");
    assert_eq!(pos.x, 1.0, "Pos.x must reflect the new bytes");
    assert_eq!(pos.y, 2.0);
    assert_eq!(pos.z, 3.0);
}

// ── Test 4 ───────────────────────────────────────────────────────────────────

/// `EntityCommands::remove::<C>()` migrates the entity to source \\ {C}.
#[test]
fn entity_commands_remove_migrates_to_smaller_archetype() {
    register_all();
    let mut ecs = EcsMaster::new();

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(PosVelBundle {
            p: Pos { x: 1.0, y: 2.0, z: 3.0 },
            v: Vel { vx: 0.0, vy: 0.0, vz: 0.0 },
        });
    });
    let entity = ecs.iter_entities().next().expect("entity");
    assert!(ecs.has_component(entity, SLOT_POS));
    assert!(ecs.has_component(entity, SLOT_VEL));

    // Remove Vel.
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(entity).remove::<Vel>();
    });

    assert!(ecs.has_entity(entity), "entity still alive after remove");
    assert!(ecs.has_component(entity, SLOT_POS), "Pos retained");
    assert!(!ecs.has_component(entity, SLOT_VEL), "Vel removed");
}

// ── Test 5 ───────────────────────────────────────────────────────────────────

/// `EntityCommands::despawn()` kills the entity at apply time.
#[test]
fn entity_commands_despawn_kills_entity() {
    register_all();
    let mut ecs = EcsMaster::new();

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(PosBundle { p: Pos { x: 0.0, y: 0.0, z: 0.0 } });
    });
    let entity = ecs.iter_entities().next().expect("entity");
    assert!(ecs.has_entity(entity));

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(entity).despawn();
    });

    assert!(!ecs.has_entity(entity), "despawn must remove from world");
    assert_eq!(ecs.entity_count(), 0);
}

// ── Test 6 ───────────────────────────────────────────────────────────────────

/// `Commands::despawn(id)` is the convenience wrapper for
/// `cmds.entity(id).despawn()`.
#[test]
fn commands_despawn_convenience_kills_entity() {
    register_all();
    let mut ecs = EcsMaster::new();

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(PosBundle { p: Pos { x: 0.0, y: 0.0, z: 0.0 } });
    });
    let entity = ecs.iter_entities().next().expect("entity");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.despawn(entity);
    });
    assert!(!ecs.has_entity(entity));
}

// ── Test 7 ───────────────────────────────────────────────────────────────────

/// `Commands::entity(id)` produces an `EntityCommands<'_, '_>` handle
/// for an existing entity without spawn-side effects.
#[test]
fn commands_entity_handle_for_existing() {
    register_all();
    let mut ecs = EcsMaster::new();

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(PosBundle { p: Pos { x: 1.0, y: 1.0, z: 1.0 } });
    });
    let entity = ecs.iter_entities().next().expect("entity");

    // Wrap then mutate via the handle.
    ecs.run_system(move |mut cmds: Commands| {
        let mut ec = cmds.entity(entity);
        assert_eq!(ec.id(), entity, "handle preserves the wrapped Entity");
        ec.insert(HealthBundle { h: Health(100) });
    });
    assert!(ecs.has_component(entity, SLOT_HEALTH), "Health inserted via handle");
}

// ── Test 8 ───────────────────────────────────────────────────────────────────

/// Full chain: `cmds.spawn(a).insert(b).id()` exercises the
/// pre-allocate → enqueue → chain → terminal sequence in one statement.
#[test]
fn chained_spawn_insert_id() {
    register_all();
    let mut ecs = EcsMaster::new();

    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    let observed = Arc::new(AtomicU64::new(u64::MAX));
    let probe = Arc::clone(&observed);

    ecs.run_system(move |mut cmds: Commands| {
        let id = cmds
            .spawn(PosBundle { p: Pos { x: 1.0, y: 2.0, z: 3.0 } })
            .insert(VelBundle { v: Vel { vx: 1.0, vy: 1.0, vz: 1.0 } })
            .id();
        probe.store(id.id().0 as u64, Ordering::Relaxed);
    });

    let captured = observed.load(Ordering::Relaxed);
    assert_ne!(captured, u64::MAX, "spawn-insert-id chain must surface an id");

    // After apply the entity should have both components — the spawn
    // landed in PosBundle's archetype, the insert migrated to PosBundle ∪
    // VelBundle.
    let entity = ecs.iter_entities().next().expect("entity");
    assert!(ecs.has_component(entity, SLOT_POS));
    assert!(ecs.has_component(entity, SLOT_VEL));
}

// ── Test 9 (EC4) ─────────────────────────────────────────────────────────────

/// `EntityCommands::reborrow` produces a handle with a shorter `'a`
/// lifetime while preserving `'s`. After the reborrow returns, the
/// original handle remains usable.
#[test]
fn entity_commands_reborrow_preserves_original_handle() {
    register_all();
    let mut ecs = EcsMaster::new();

    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(PosBundle { p: Pos { x: 0.0, y: 0.0, z: 0.0 } });
    });
    let entity = ecs.iter_entities().next().expect("entity");

    ecs.run_system(move |mut cmds: Commands| {
        let mut ec = cmds.entity(entity);
        {
            // Reborrow with shorter lifetime; pass to a helper.
            let mut shorter = ec.reborrow();
            shorter.insert(HealthBundle { h: Health(50) });
        }
        // Original `ec` still usable.
        ec.insert(VelBundle { v: Vel { vx: 1.0, vy: 0.0, vz: 0.0 } });
    });

    assert!(ecs.has_component(entity, SLOT_HEALTH), "reborrow's insert landed");
    assert!(ecs.has_component(entity, SLOT_VEL), "original's insert landed");
}

// ── Test 10 ──────────────────────────────────────────────────────────────────

/// `Commands::reserve_entity` returns distinct IDs across calls. This
/// exposes the atomic-counter path without enqueueing any spawn.
#[test]
fn commands_reserve_entity_yields_distinct_ids() {
    register_all();
    let mut ecs = EcsMaster::new();

    use std::sync::Arc;
    use std::sync::{Mutex};
    let collected: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&collected);

    ecs.run_system(move |cmds: Commands| {
        let mut probe = probe.lock().expect("not poisoned");
        for _ in 0..16 {
            probe.push(cmds.reserve_entity().id().0 as u64);
        }
    });

    let ids = collected.lock().expect("not poisoned").clone();
    assert_eq!(ids.len(), 16);
    let mut seen = std::collections::HashSet::new();
    for id in &ids {
        assert!(seen.insert(*id), "reserved IDs must be distinct (atomic counter)");
    }
    // Reserves never registered any entity — they leak by design (one
    // ID per missed apply; counter marches forward).
    assert_eq!(ecs.entity_count(), 0, "reserve_entity does not register");
}
