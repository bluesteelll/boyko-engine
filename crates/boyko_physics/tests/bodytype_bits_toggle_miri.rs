//! Miri-tractable coverage of the BodyType → EnableTag-bit refactor's NEW unsafe
//! surface: the `IsEnabled<Simulated>` query-data fetch (`(*col).test(row)`), the
//! `enable` / `disable` bit toggle, and the direct (threadpool-free) gather read.
//!
//! The full pipeline (`bodytype_bits_toggle.rs`) spins up `boyko_threadpool`, whose
//! spin loop is intractable under Miri (the pool is already loom + Miri proven in
//! the ECS Phase-9 series). These tests drive ONLY `&mut EcsMaster` + `world.query`
//! directly — NO threadpool — so `cargo +nightly miri test` validates the new
//! `IsEnabled` fetch path, the toggle RMW, and the column NULL / page deref for UB
//! under Tree Borrows.
//!
//! Run (per the task toolchain):
//! ```text
//! RUSTUP_TOOLCHAIN=nightly-x86_64-pc-windows-gnu \
//!   MIRIFLAGS="-Zmiri-tree-borrows -Zmiri-ignore-leaks" \
//!   cargo miri test -p boyko-physics --test bodytype_bits_toggle_miri
//! ```

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::data_is_enabled::IsEnabled;

use boyko_physics::components::{
    Collider, ColliderShape, Kinematic, RigidBody, RigidBodyBundle, RigidBodyMass, Simulated,
};
use boyko_physics::math::{Mat3, Quat, Vec3};

/// Views a `#[repr(C)]` POD value as its raw bytes for the `create_entity` path.
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `#[repr(C)]` `T`; we view its `size_of::<T>()`
    // bytes read-only for the borrow — the exact layout the pool stores.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

fn spawn(world: &mut EcsMaster, x: f32) -> Entity {
    let archetype = world.bundle_archetype_id_for::<RigidBodyBundle>();
    let body = RigidBody {
        position: Vec3::new(x, 0.0, 0.0),
        linear_velocity: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        angular_velocity: Vec3::ZERO,
    };
    let mass = RigidBodyMass {
        inv_inertia: Mat3::IDENTITY,
        inv_mass: 1.0,
        restitution: 0.0,
        friction: 0.5,
    };
    let collider = Collider {
        shape: ColliderShape::Sphere { radius: 0.5 },
        layer: 1,
        mask: 1,
    };
    world
        .create_entity(
            archetype,
            &[
                (RigidBody::component_id(), as_bytes(&body)),
                (RigidBodyMass::component_id(), as_bytes(&mass)),
                (Collider::component_id(), as_bytes(&collider)),
            ],
        )
        .expect("invariant: RigidBodyBundle archetype accepts the three columns")
}

/// Exercises the `IsEnabled<Simulated>` fetch on a column that EXISTS (the bit was
/// toggled), reading the bit per row in iter order. Drives the new `(*col).test`
/// deref so Miri checks the paged column read for UB.
#[test]
fn is_enabled_fetch_reads_toggled_bit_miri() {
    let mut world = EcsMaster::new();
    let e0 = spawn(&mut world, 0.0);
    let _e1 = spawn(&mut world, 1.0);
    let e2 = spawn(&mut world, 2.0);
    world.enable::<Simulated>(e0);
    world.enable::<Simulated>(e2);

    let q = world.query::<(&RigidBody, IsEnabled<Simulated>), ()>();
    let bits: Vec<bool> = q.iter().map(|(_, on)| on).collect();
    assert_eq!(bits, vec![true, false, true], "fetch yields the per-row bit in iter order");
}

/// Exercises the `IsEnabled<Simulated>` fetch on an archetype with NO column for
/// the tag (the bit was never toggled anywhere): the fetch's NULL short-circuit
/// must return `false` for every row (no deref of a null pointer).
#[test]
fn is_enabled_fetch_null_column_reads_false_miri() {
    let mut world = EcsMaster::new();
    spawn(&mut world, 0.0);
    spawn(&mut world, 1.0);
    // No `enable::<Simulated>` call anywhere → no EnableColumn for the tag.

    let q = world.query::<(&RigidBody, IsEnabled<Simulated>), ()>();
    let all_false = q.iter().all(|(_, on)| !on);
    assert!(all_false, "a NULL column reads `false` for every row (no null deref)");
}

/// Exercises the toggle RMW + re-read: enable, read true, disable, read false —
/// driving the `set_enable_bit` write path and the `IsEnabled` fetch over the same
/// live column under Miri.
#[test]
fn toggle_then_refetch_is_consistent_miri() {
    let mut world = EcsMaster::new();
    let e = spawn(&mut world, 0.0);

    world.enable::<Simulated>(e);
    {
        let q = world.query::<(&RigidBody, IsEnabled<Simulated>), ()>();
        assert!(q.iter().all(|(_, on)| on), "after enable the bit reads true");
    }

    world.disable::<Simulated>(e);
    {
        let q = world.query::<(&RigidBody, IsEnabled<Simulated>), ()>();
        assert!(q.iter().all(|(_, on)| !on), "after disable the bit reads false");
    }
}

/// Two distinct tags on the same body: `IsEnabled<Simulated>` and
/// `IsEnabled<Kinematic>` resolve to DIFFERENT columns and read independently.
#[test]
fn two_tags_read_independently_miri() {
    let mut world = EcsMaster::new();
    let e = spawn(&mut world, 0.0);
    world.enable::<Kinematic>(e);
    // Simulated left OFF.

    let q = world.query::<(&RigidBody, IsEnabled<Simulated>, IsEnabled<Kinematic>), ()>();
    let rows: Vec<(bool, bool)> = q.iter().map(|(_, s, k)| (s, k)).collect();
    assert_eq!(rows, vec![(false, true)], "Simulated OFF, Kinematic ON read independently");
}
