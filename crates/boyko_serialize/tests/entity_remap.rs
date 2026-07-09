//! Phase S2.5 — ENTITY-REMAP on load (the saved→fresh `Entity` round-trip).
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §3.11 step 5 + §5 C4. After S2 an `Entity`
//! field loaded as its RAW saved id (stale — it pointed at the saved id, not the
//! freshly-loaded entity). S2.5 adds the remap pass: a saved `Entity` reference
//! inside `ChildOf` (the hand-written v1 relationship remap) or an
//! `#[entities]`-annotated derived component is rewritten to the freshly-allocated
//! `Entity`. A plain `Entity` field WITHOUT `#[entities]` is NOT remapped (the C4
//! explicit-opt-in decision). An unmapped saved id is a loud `LoadError`, never a
//! silent dangling reference.
//!
//! These exercise the S2.5 runtime `unsafe` (the remap pass mutates committed
//! component rows in place through a fn-ptr into pool memory + the `ChildOf` /
//! `#[entities]` remap fns), so the suite is also run under Miri-TB.

use std::sync::{Arc, Mutex};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::hierarchy::ChildOf;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Component;

use boyko_serialize::{
    DecodeError, LoadEntityPolicy, LoadError, SaveOptions, load_world, save_world,
};

// ── Test components ──────────────────────────────────────────────────────────

/// A POB marker so a hierarchy node's archetype carries at least one serializable
/// column (so the entity — and thus its saved→fresh mapping — is materialized on
/// load). `u32` payload distinguishes the nodes for the assertions.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Tag(u32);

/// A derived component with an `#[entities]`-annotated `Entity` field — the
/// opt-in remap path. `n` is a payload to confirm the rest of the value also
/// round-trips.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Targeted {
    n: u32,
    #[entities]
    target: Entity,
}

/// A derived component with a PLAIN `Entity` field (NO `#[entities]`) — the
/// explicit-opt-in negative: its `Entity` must stay the RAW saved id on load.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct LooseRef {
    n: u32,
    other: Entity,
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Saves `world` to a fresh byte buffer.
fn save(world: &EcsMaster) -> Vec<u8> {
    let mut out = Vec::new();
    save_world(world, &SaveOptions::default(), &mut out).expect("save");
    out
}

/// Builds a parent → child link via `Commands::add_child` (the canonical hierarchy
/// path), each node carrying `Tag`. Returns the live `(parent, child)` ids. One
/// apply window runs.
fn spawn_parent_child(ecs: &mut EcsMaster, parent_tag: u32, child_tag: u32) -> (Entity, Entity) {
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    ecs.run_system(move |mut cmds: Commands| {
        let parent = cmds.spawn(Tag(parent_tag)).id();
        let child = cmds.spawn(Tag(child_tag)).id();
        cmds.entity(parent).add_child(child);
        let mut v = probe.lock().expect("probe");
        v.push(parent);
        v.push(child);
    });
    let v = sink.lock().expect("probe").clone();
    (v[0], v[1])
}

/// Finds the loaded entity carrying `Tag(value)` (entity ids are fresh after a
/// load, so nodes are identified by their distinguishing payload).
fn entity_with_tag(world: &EcsMaster, value: u32) -> Entity {
    world
        .iter_entities()
        .find(|&e| world.get_component::<Tag>(e).map(|t| t.0) == Some(value))
        .unwrap_or_else(|| panic!("no loaded entity carries Tag({value})"))
}

// ════════════════════════════════════════════════════════════════════════════
// ChildOf: a parent → child link round-trips with the child's ChildOf remapped
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn child_of_remaps_to_loaded_parent_not_saved_id() {
    let mut src = EcsMaster::new();
    let (saved_parent, saved_child) = spawn_parent_child(&mut src, 1, 2);
    assert_eq!(
        src.get_component::<ChildOf>(saved_child).map(|c| c.0),
        Some(saved_parent),
        "source child's ChildOf == parent",
    );

    let bytes = save(&src);

    let mut dst = EcsMaster::new();
    // The running build must have the components registered before load (W1).
    let _ = Tag::component_id();
    let _ = ChildOf::component_id();
    load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load hierarchy");

    let loaded_parent = entity_with_tag(&dst, 1);
    let loaded_child = entity_with_tag(&dst, 2);

    let child_of = dst
        .get_component::<ChildOf>(loaded_child)
        .map(|c| c.0)
        .expect("loaded child carries ChildOf");

    assert_eq!(
        child_of, loaded_parent,
        "the loaded child's ChildOf must point at the LOADED parent (remapped)",
    );
    // The remap must NOT leave the raw saved id (unless it happened to coincide —
    // a fresh world allocates from id 0, so the saved parent id is almost surely
    // not the loaded parent id; assert the remap actually moved it off the saved id
    // when they differ).
    if loaded_parent != saved_parent {
        assert_ne!(
            child_of, saved_parent,
            "ChildOf must be remapped off the stale saved id",
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// ChildOf: a multi-level chain grandparent → parent → child all remap
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn child_of_multi_level_chain_all_remap() {
    let mut src = EcsMaster::new();
    let sink: Arc<Mutex<Vec<Entity>>> = Arc::new(Mutex::new(Vec::new()));
    let probe = Arc::clone(&sink);
    src.run_system(move |mut cmds: Commands| {
        let grandparent = cmds.spawn(Tag(10)).id();
        let parent = cmds.spawn(Tag(20)).id();
        let child = cmds.spawn(Tag(30)).id();
        cmds.entity(grandparent).add_child(parent);
        cmds.entity(parent).add_child(child);
        let mut v = probe.lock().expect("probe");
        v.push(grandparent);
        v.push(parent);
        v.push(child);
    });

    let bytes = save(&src);

    let mut dst = EcsMaster::new();
    let _ = Tag::component_id();
    let _ = ChildOf::component_id();
    load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load chain");

    let g = entity_with_tag(&dst, 10);
    let p = entity_with_tag(&dst, 20);
    let c = entity_with_tag(&dst, 30);

    // The grandparent is the root (no ChildOf).
    assert!(
        dst.get_component::<ChildOf>(g).is_none(),
        "the grandparent root has no ChildOf",
    );
    assert_eq!(
        dst.get_component::<ChildOf>(p).map(|x| x.0),
        Some(g),
        "parent's ChildOf remaps to the loaded grandparent",
    );
    assert_eq!(
        dst.get_component::<ChildOf>(c).map(|x| x.0),
        Some(p),
        "child's ChildOf remaps to the loaded parent",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// #[entities] field: the annotated Entity remaps to the loaded entity
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn entities_field_remaps_to_loaded_entity() {
    let mut src = EcsMaster::new();
    // Two entities: a "subject" carrying Tag, and a "referrer" carrying Targeted
    // whose annotated `target` points at the subject's saved id.
    let arch_subject = src.get_or_create_archetype(&[Tag::component_id()]);
    let subject = src.spawn_one(arch_subject, Tag(7)).expect("spawn subject");

    let arch_ref =
        src.get_or_create_archetype(&[Tag::component_id(), Targeted::component_id()]);
    src.spawn_two(arch_ref, Tag(8), Targeted { n: 42, target: subject })
        .expect("spawn referrer");

    let bytes = save(&src);

    let mut dst = EcsMaster::new();
    let _ = Tag::component_id();
    let _ = Targeted::component_id();
    load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load #[entities]");

    let loaded_subject = entity_with_tag(&dst, 7);
    let loaded_referrer = entity_with_tag(&dst, 8);

    let targeted = dst
        .get_component::<Targeted>(loaded_referrer)
        .copied()
        .expect("loaded referrer carries Targeted");

    assert_eq!(targeted.n, 42, "the non-Entity payload round-trips");
    assert_eq!(
        targeted.target, loaded_subject,
        "the #[entities] target remaps to the LOADED subject",
    );
    if loaded_subject != subject {
        assert_ne!(
            targeted.target, subject,
            "the #[entities] target must be remapped off the stale saved id",
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// A plain Entity field (no #[entities]) is NOT remapped (the C4 negative)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn plain_entity_field_is_not_remapped() {
    let mut src = EcsMaster::new();
    let arch_subject = src.get_or_create_archetype(&[Tag::component_id()]);
    let subject = src.spawn_one(arch_subject, Tag(70)).expect("spawn subject");

    let arch_ref =
        src.get_or_create_archetype(&[Tag::component_id(), LooseRef::component_id()]);
    src.spawn_two(arch_ref, Tag(80), LooseRef { n: 5, other: subject })
        .expect("spawn loose referrer");

    let bytes = save(&src);

    let mut dst = EcsMaster::new();
    let _ = Tag::component_id();
    let _ = LooseRef::component_id();
    load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load LooseRef");

    let loaded_referrer = entity_with_tag(&dst, 80);
    let loose = dst
        .get_component::<LooseRef>(loaded_referrer)
        .copied()
        .expect("loaded referrer carries LooseRef");

    assert_eq!(loose.n, 5, "the non-Entity payload round-trips");
    // The C4 explicit-opt-in decision: a plain Entity field keeps its RAW saved id.
    assert_eq!(
        loose.other, subject,
        "a plain Entity field (no #[entities]) must stay the raw saved id",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// An unmapped / dangling saved ref → a loud LoadError (never silent)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn unmapped_saved_ref_is_a_loud_load_error() {
    // Build a referrer whose `#[entities]` target points at an entity that is NOT
    // saved (a dangling reference). The simplest way: spawn the referrer pointing
    // at a never-saved id. We point at a high id that no saved entity will carry.
    let mut src = EcsMaster::new();
    let arch_ref =
        src.get_or_create_archetype(&[Tag::component_id(), Targeted::component_id()]);
    // A dangling target: an Entity id far above any id this world allocates.
    let dangling = Entity::new(
        boyko_ecs::ecs::identifiers::primitives::EntityId(999_999),
        0,
    );
    src.spawn_two(arch_ref, Tag(100), Targeted { n: 1, target: dangling })
        .expect("spawn dangling referrer");

    let bytes = save(&src);

    let mut dst = EcsMaster::new();
    let _ = Tag::component_id();
    let _ = Targeted::component_id();
    let err = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).unwrap_err();
    assert!(
        matches!(err, LoadError::Decode(DecodeError::UnmappedEntity)),
        "an unmapped saved ref must be a loud LoadError, got {err:?}",
    );
}

// ════════════════════════════════════════════════════════════════════════════
// A ChildOf pointing at a never-saved parent → a loud LoadError
// ════════════════════════════════════════════════════════════════════════════
//
// NOTE on the construction: a `ChildOf` whose parent does not exist is REMOVED at
// insert time by the validating `on_insert` hook (boyko-engine's dangling-link
// guard), so it can never be SAVED through the normal API. To exercise the
// `ChildOf`-specific dangling path we byte-patch a VALID save: spawn a real
// parent → child, save, then bump ONLY the child's encoded `ChildOf` parent id (the
// 12-byte `[id u64 LE][gen u32 LE]` record the S1.5 `Wire` `Entity` codec writes
// into the `ChildOf` column's DATA region) to a never-saved sentinel. The exact
// column-data offset is parsed from the saved archetype block (mirroring
// `load_roundtrip::absent_file_type_is_skipped`) so the patch never touches a
// table u64 that happens to equal the parent id.

#[test]
fn child_of_dangling_parent_is_a_loud_load_error() {
    let mut src = EcsMaster::new();
    let _ = spawn_parent_child(&mut src, 1, 2);

    let mut bytes = save(&src);

    // Locate the `ChildOf` column's data region by parsing the file. The child's
    // archetype is `{Tag, ChildOf}`; `ChildOf` is the lone `SerializeViaFn` (decode)
    // column. Walk each archetype block's columns and find the one whose
    // `serializability` byte (in its resolved type entry) is `SerializeViaFn` (1).
    let pos = childof_column_data_offset(&bytes);

    // Patch the encoded parent id (first 8 bytes of the 12-byte ChildOf record) to a
    // sentinel that no saved entity carries.
    let sentinel: u64 = 777_777;
    bytes[pos..pos + 8].copy_from_slice(&sentinel.to_le_bytes());

    let mut dst = EcsMaster::new();
    let _ = Tag::component_id();
    let _ = ChildOf::component_id();
    let err = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).unwrap_err();
    assert!(
        matches!(err, LoadError::Decode(DecodeError::UnmappedEntity)),
        "a ChildOf with a dangling parent must be a loud LoadError, got {err:?}",
    );
}

/// Parses the saved bytes to return the file offset of the first `SerializeViaFn`
/// (decode) column's data region — the `ChildOf` column in the hierarchy save. A
/// small format walker so the dangling-`ChildOf` patch targets the exact encoded id
/// (never a POB column region / entity-row table u64). Mirrors the byte-parsing in
/// `load_roundtrip::absent_file_type_is_skipped`.
fn childof_column_data_offset(bytes: &[u8]) -> usize {
    let rd_u32 = |off: usize| u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
    let rd_u64 = |off: usize| u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap());

    let type_table_off = rd_u64(16) as usize;
    let archetype_table_off = rd_u64(24) as usize;
    let archetype_count = rd_u32(52) as usize;

    // Each `TypeTableEntry` is 40 bytes; `serializability` is byte 34. `1` == ViaFn.
    let type_is_via_fn = |type_index: usize| {
        let entry_off = type_table_off + type_index * 40;
        bytes[entry_off + 34] == 1
    };

    // Each `ArchetypeBlock` header is 24 bytes: component_count(u32),
    // entity_count(u32), type_indices_off(u32), column_regions_off(u32),
    // entity_rows_off(u32), pad(u32). Walk blocks until a ViaFn column is found.
    let mut block_off = archetype_table_off;
    for _ in 0..archetype_count {
        let component_count = rd_u32(block_off) as usize;
        let entity_count = rd_u32(block_off + 4) as usize;
        let type_indices_off = rd_u32(block_off + 8) as usize;
        let column_regions_off = rd_u32(block_off + 12) as usize;
        let entity_rows_off = rd_u32(block_off + 16) as usize;

        for c in 0..component_count {
            let type_index = rd_u32(type_indices_off + c * 4) as usize;
            if type_is_via_fn(type_index) {
                // `ColumnRegion` is 16 bytes: data_off(u64), byte_len(u64).
                let region_off = column_regions_off + c * 16;
                return rd_u64(region_off) as usize;
            }
        }
        // Advance to the next block (past this block's entity-row table).
        block_off = entity_rows_off + entity_count * 8;
    }
    panic!("no SerializeViaFn (ChildOf) column found in the save");
}
