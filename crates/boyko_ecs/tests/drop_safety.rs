/// Integration tests for drop safety (audit findings C-001, M-001 lineage).
///
/// Phase X.J: the shared `Box<Arena>` (the original C-001 subject) is gone —
/// component storage is per-pool `VmReservation`s (Phase X.I). These tests
/// remain as world construct/use/teardown smoke coverage:
///   - repeated `EcsMaster` construction + drop must not crash, leak
///     (detectable by Miri), or double-free (M-001 lineage: every pool's
///     reservation is released exactly once with the matching deallocator);
///   - archetype/entity creation after construction must work (C-001
///     lineage: no dangling storage pointers after the world is moved).
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;

// Use very high IDs to avoid colliding with in-crate test registrations.
const DROP_POSITION_ID: ComponentId = ComponentId(480);
const DROP_VELOCITY_ID: ComponentId = ComponentId(481);

#[repr(C)]
struct DropPosition { x: f32, y: f32, z: f32 }

#[repr(C)]
struct DropVelocity { vx: f32, vy: f32, vz: f32 }

fn register_drop_test_components() {
    component_registry::register_layout::<DropPosition>(DROP_POSITION_ID.0);
    component_registry::register_layout::<DropVelocity>(DROP_VELOCITY_ID.0);
}

/// C-001 / M-001 lineage: create and drop EcsMaster in a loop without any
/// archetype operations; construction + teardown must be leak- and crash-free.
#[test]
fn ecs_master_repeated_drop_does_not_crash() {
    for _ in 0..20 {
        let _ecs = EcsMaster::new();
        // _ecs drops here — must not double-free or panic
    }
}

/// C-001 lineage: ensure that EcsMaster operations work correctly after
/// construction (no dangling storage pointers).
#[test]
fn ecs_master_new_then_box_arena_addr_stable() {
    register_drop_test_components();

    let mut ecs = EcsMaster::new();

    // Create an archetype — mints per-pool reservations under ArchetypeMaster.
    let arch_id = ecs.create_archetype(&[DROP_POSITION_ID, DROP_VELOCITY_ID]);

    // Allocate component data.
    let pos = DropPosition { x: 1.0, y: 2.0, z: 3.0 };
    let vel = DropVelocity { vx: 0.1, vy: 0.2, vz: 0.3 };

    let pos_bytes = unsafe {
        std::slice::from_raw_parts(
            &pos as *const _ as *const u8,
            std::mem::size_of::<DropPosition>(),
        )
    };
    let vel_bytes = unsafe {
        std::slice::from_raw_parts(
            &vel as *const _ as *const u8,
            std::mem::size_of::<DropVelocity>(),
        )
    };

    // create_entity writes into the pools — would crash/UB on dangling storage.
    let entity = ecs
        .create_entity(arch_id, &[(DROP_POSITION_ID, pos_bytes), (DROP_VELOCITY_ID, vel_bytes)])
        .expect("create_entity must succeed against stable pool storage");

    assert!(ecs.has_entity(entity), "entity must be valid after creation");

    // Drop ecs here — pool reservations are released, must not double-free or crash.
}

/// M-001: with_capacity variant also goes through impl Drop.
#[test]
fn ecs_master_with_capacity_drop_is_safe() {
    for _ in 0..5 {
        let _ecs = EcsMaster::with_capacity(1024, 16);
    }
}

/// C-001 lineage: exercises the full construction + archetype usage path
/// that would trigger Miri's dangling-pointer detection if any storage
/// pointer were minted against a since-moved location.
#[test]
fn ecs_master_multiple_archetypes_after_construction() {
    register_drop_test_components();

    let mut ecs = EcsMaster::new();

    let arch1 = ecs.create_archetype(&[DROP_POSITION_ID]);
    let arch2 = ecs.create_archetype(&[DROP_POSITION_ID, DROP_VELOCITY_ID]);

    assert_eq!(ecs.archetype_count(), 2, "two archetypes must be registered");

    // Both archetypes own live component pools at drop time.
    let _ = arch1;
    let _ = arch2;
    // Drop ecs — no panic/double-free expected.
}
