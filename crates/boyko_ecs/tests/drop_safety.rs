/// Integration tests for drop safety (audit findings C-001, M-001).
///
/// These tests verify:
///   C-001 — `EcsMaster` uses `Box<Arena>` so the arena address is heap-stable
///            and `ArchetypeMaster`'s `NonNull<Arena>` does not dangle after a move.
///   M-001 — `impl Drop for Arena` releases the backing reservation (post-X.F:
///            multi-GB reserve, partially committed); creating multiple
///            `EcsMaster` instances and dropping them must not crash, leak
///            (detectable by Miri), or double-free.
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

/// C-001 / M-001: create and drop EcsMaster in a loop without any archetype operations.
/// Demonstrates that the Box<Arena> fix prevents the dangling-pointer construction and
/// that impl Drop for Arena correctly frees memory each iteration.
#[test]
fn ecs_master_repeated_drop_does_not_crash() {
    for _ in 0..20 {
        let _ecs = EcsMaster::new();
        // _ecs drops here — must not double-free or panic
    }
}

/// C-001: ensure that EcsMaster operations work correctly after construction
/// (i.e., the arena pointer inside ArchetypeMaster is not dangling).
#[test]
fn ecs_master_new_then_box_arena_addr_stable() {
    register_drop_test_components();

    let mut ecs = EcsMaster::new();

    // Create an archetype — triggers use of the arena pointer stored in ArchetypeMaster.
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

    // create_entity uses the arena to add components — would crash/UB if arena ptr dangled.
    let entity = ecs
        .create_entity(arch_id, &[(DROP_POSITION_ID, pos_bytes), (DROP_VELOCITY_ID, vel_bytes)])
        .expect("create_entity must succeed with a stable arena pointer");

    assert!(ecs.has_entity(entity), "entity must be valid after creation");

    // Drop ecs here — arena is freed, must not double-free or crash.
}

/// M-001: with_capacity variant also goes through impl Drop.
#[test]
fn ecs_master_with_capacity_drop_is_safe() {
    for _ in 0..5 {
        let _ecs = EcsMaster::with_capacity(1024, 16);
    }
}

/// C-001 subtle: EcsMaster::new() constructs arena on heap via Box::new BEFORE
/// passing &arena to ArchetypeMaster::new. If the old code (stack arena) were
/// restored, Miri would report a use-after-free here because the EcsMaster is
/// returned by value (potentially moving the stack). This test doesn't assert
/// memory addresses but exercises the full construction + archetype usage path
/// that would trigger Miri's dangling-pointer detection.
#[test]
fn ecs_master_multiple_archetypes_after_construction() {
    register_drop_test_components();

    let mut ecs = EcsMaster::new();

    let arch1 = ecs.create_archetype(&[DROP_POSITION_ID]);
    let arch2 = ecs.create_archetype(&[DROP_POSITION_ID, DROP_VELOCITY_ID]);

    assert_eq!(ecs.archetype_count(), 2, "two archetypes must be registered");

    // Both archetypes touch the arena for their component pools.
    let _ = arch1;
    let _ = arch2;
    // Drop ecs — no panic/double-free expected.
}
