//! EnableTag Wave 3 / Step 6 — cross-archetype enable-bit migration, end-to-end
//! through the PUBLIC deferred API.
//!
//! When an entity migrates archetypes (insert / remove a normal component, or
//! attach / detach a dynamic tag), its EnableTag bits live per-archetype-per-row
//! and must be COPIED from the source row to the target append row, else a
//! toggled flag is silently lost on the next structural op. Step 6 wires the
//! 3-phase paged copy into all four migration helpers; this suite proves it
//! survives through the real `EntityCommands` apply path (the in-crate unit tests
//! in `migration_helpers.rs` exercise the helpers directly).
//!
//! The flag is a DYNAMIC enable tag (`register_enable_tag` + `enable_id` /
//! `is_enabled_id`): the public surface that does not require the Wave-5
//! `#[component(storage = "bitset")]` derive. The migrated tag is a DYNAMIC
//! normal tag (`register_tag` → `add_tag` / `remove_tag`). Data components use
//! `#[derive(Bundle)]` (the only way to construct the sealed `Bundle`), with
//! component ids in the grep-verified-free block [333, 340) (disjoint from the
//! 320-332 block the in-crate Step-6 unit tests use).

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::register_layout;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::ComponentId;
use boyko_macros::Bundle;

// ── Fixed ids: free block [333, 340) ────────────────────────────────────────
const SLOT_POS: ComponentId = ComponentId(333);
const SLOT_VEL: ComponentId = ComponentId(334);

#[repr(C)]
#[derive(Clone, Copy)]
struct Pos {
    x: f32,
    y: f32,
}
impl Component for Pos {
    fn component_id() -> ComponentId {
        SLOT_POS
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Vel {
    dx: f32,
    dy: f32,
}
impl Component for Vel {
    fn component_id() -> ComponentId {
        SLOT_VEL
    }
}

#[derive(Bundle)]
struct PosBundle {
    p: Pos,
}

#[derive(Bundle)]
struct VelBundle {
    v: Vel,
}

fn register() {
    register_layout::<Pos>(SLOT_POS.0);
    register_layout::<Vel>(SLOT_VEL.0);
}

/// Spawns one `Pos` entity and returns its handle.
fn spawn_pos(ecs: &mut EcsMaster) -> Entity {
    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(PosBundle {
            p: Pos { x: 1.0, y: 2.0 },
        });
    });
    ecs.iter_entities().next().expect("one entity spawned")
}

// ── Insert migration (EntityCommands::insert) ────────────────────────────────

#[test]
fn insert_migration_preserves_enable_bit_via_public_api() {
    register();
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_enable_tag("step6_insert_public");
    let e = spawn_pos(&mut ecs);

    ecs.enable_id(e, tag);
    assert!(ecs.is_enabled_id(e, tag), "flag set before migration");

    // Insert Vel → migrate [Pos] → [Pos, Vel].
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(VelBundle {
            v: Vel { dx: 0.5, dy: 0.5 },
        });
    });

    assert!(ecs.has_component(e, SLOT_VEL), "insert migrated the entity");
    assert!(
        ecs.is_enabled_id(e, tag),
        "the enable bit must survive an insert migration through the public API"
    );
}

// ── Remove migration (EntityCommands::remove) ────────────────────────────────

#[test]
fn remove_migration_preserves_enable_bit_via_public_api() {
    register();
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_enable_tag("step6_remove_public");

    // Spawn [Pos, Vel].
    ecs.run_system(|mut cmds: Commands| {
        cmds.spawn(PosBundle {
            p: Pos { x: 0.0, y: 0.0 },
        })
        .insert(VelBundle {
            v: Vel { dx: 1.0, dy: 1.0 },
        });
    });
    let e = ecs.iter_entities().next().expect("entity");
    assert!(ecs.has_component(e, SLOT_VEL));

    ecs.enable_id(e, tag);
    assert!(ecs.is_enabled_id(e, tag));

    // Remove Vel → migrate [Pos, Vel] → [Pos].
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).remove::<Vel>();
    });

    assert!(!ecs.has_component(e, SLOT_VEL), "remove migrated the entity");
    assert!(
        ecs.is_enabled_id(e, tag),
        "the enable bit must survive a remove migration through the public API"
    );
}

// ── Attach-tag migration (EntityCommands::add_tag) ───────────────────────────

#[test]
fn attach_tag_migration_preserves_enable_bit_via_public_api() {
    register();
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_enable_tag("step6_attach_public");
    let ztag = ecs.register_tag("step6_attach_ztag");

    let e = spawn_pos(&mut ecs);
    ecs.enable_id(e, tag);
    assert!(ecs.is_enabled_id(e, tag));

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).add_tag(ztag);
    });

    assert!(ecs.has_tag(e, ztag), "add_tag migrated the entity");
    assert!(
        ecs.is_enabled_id(e, tag),
        "the enable bit must survive an attach-tag migration through the public API"
    );
}

// ── Detach-tag migration (EntityCommands::remove_tag) ────────────────────────

#[test]
fn detach_tag_migration_preserves_enable_bit_via_public_api() {
    register();
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_enable_tag("step6_detach_public");
    let ztag = ecs.register_tag("step6_detach_ztag");

    // Spawn [Pos], then attach the tag (migrates to [Pos, ztag]).
    let e = spawn_pos(&mut ecs);
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).add_tag(ztag);
    });
    assert!(ecs.has_tag(e, ztag));

    ecs.enable_id(e, tag);
    assert!(ecs.is_enabled_id(e, tag));

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).remove_tag(ztag);
    });

    assert!(!ecs.has_tag(e, ztag), "remove_tag migrated the entity");
    assert!(
        ecs.is_enabled_id(e, tag),
        "the enable bit must survive a detach-tag migration through the public API"
    );
}

// ── Multi-hop: chain migrations, the bit must follow every hop ───────────────

#[test]
fn multi_hop_migration_chain_preserves_enable_bit() {
    register();
    let mut ecs = EcsMaster::new();
    let tag = ecs.register_enable_tag("step6_multihop_public");
    let ztag = ecs.register_tag("step6_multihop_ztag");

    let e = spawn_pos(&mut ecs);
    ecs.enable_id(e, tag);

    // [Pos] -insert Vel-> [Pos, Vel] -add ztag-> [Pos, Vel, ztag]
    //        -remove Vel-> [Pos, ztag] -remove_tag ztag-> [Pos]
    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).insert(VelBundle {
            v: Vel { dx: 1.0, dy: 1.0 },
        });
    });
    assert!(ecs.is_enabled_id(e, tag), "bit survives hop 1 (insert)");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).add_tag(ztag);
    });
    assert!(ecs.is_enabled_id(e, tag), "bit survives hop 2 (add_tag)");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).remove::<Vel>();
    });
    assert!(ecs.is_enabled_id(e, tag), "bit survives hop 3 (remove)");

    ecs.run_system(move |mut cmds: Commands| {
        cmds.entity(e).remove_tag(ztag);
    });
    assert!(ecs.is_enabled_id(e, tag), "bit survives hop 4 (remove_tag)");

    // The entity is back in [Pos]; the flag is still set, and clearing it works.
    assert!(ecs.is_enabled_id(e, tag));
    ecs.disable_id(e, tag);
    assert!(!ecs.is_enabled_id(e, tag), "disable after the chain still works");
}
