//! Phase S2 — full save → load round-trip EQUALITY (the headline).
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §3.10 / §3.11 (LOAD) + §5. These tests build
//! a world (POB components + an owning component, NO Entity fields), `save_world`
//! it, `load_world` it into a FRESH `EcsMaster`, and assert every component value
//! survives the round-trip (compared as a multiset — loaded entities get fresh ids,
//! so values are matched, not entity identities). They also cover an empty world, a
//! multi-archetype world, a truncated / malformed stream (→ `LoadError`, world left
//! consistent), a POB layout-fingerprint mismatch (→ `LoadError`), and a file type
//! absent in the load process (→ `types_skipped`).
//!
//! These exercise the S2 runtime `unsafe` (the writer's `copy_nonoverlapping` into
//! a fresh pool, `deserialize_fn` into reserved uninit rows, and the
//! rollback-on-error drop), so the suite is also run under Miri-TB.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_macros::Component;

use boyko_serialize::{
    LoadEntityPolicy, LoadError, SaveOptions, load_world, save_world,
};

// ── Test components (NO Entity fields — the S2 boundary) ───────────────────────

/// POB: `#[repr(C)]`, all-float fields → `PlainOldBytes` (blitted whole-column).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Position {
    x: f32,
    y: f32,
    z: f32,
}

/// POB: `#[repr(C)]`, all-int fields.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Velocity {
    dx: i32,
    dy: i32,
}

/// Owning component (`String` + `Vec<u8>`) → `SerializeViaFn` (decode path).
#[derive(Component, Clone, PartialEq, Debug)]
struct Inventory {
    name: String,
    flags: Vec<u8>,
}

/// A POB row's value, decomposed into raw bits for an order-stable multiset
/// compare (`f32` is not `Ord`, and loaded entity ids are fresh).
type PosVelBits = ((u32, u32, u32), (i32, i32));

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Saves `world` to a fresh byte buffer.
fn save(world: &EcsMaster) -> Vec<u8> {
    let mut out = Vec::new();
    save_world(world, &SaveOptions::default(), &mut out).expect("save");
    out
}

/// Collects all `Position` values in a world as a sorted multiset (loaded entity
/// ids are fresh, so equality is by value).
fn positions_sorted(world: &mut EcsMaster) -> Vec<(u32, u32, u32)> {
    let mut v: Vec<(u32, u32, u32)> = world
        .query::<&Position, ()>()
        .iter()
        .map(|p| (p.x.to_bits(), p.y.to_bits(), p.z.to_bits()))
        .collect();
    v.sort_unstable();
    v
}

// ════════════════════════════════════════════════════════════════════════════
// Single-archetype POB world: full equality
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn pob_single_archetype_roundtrip_equality() {
    let mut src = EcsMaster::new();
    let arch = src.get_or_create_archetype(&[Position::component_id(), Velocity::component_id()]);
    let rows = [
        (Position { x: 1.0, y: 2.0, z: 3.0 }, Velocity { dx: 10, dy: 20 }),
        (Position { x: 4.5, y: 5.5, z: 6.5 }, Velocity { dx: -7, dy: 8 }),
        (Position { x: -1.25, y: 0.0, z: 99.0 }, Velocity { dx: 0, dy: 0 }),
    ];
    for (p, v) in rows {
        src.spawn_two(arch, p, v).expect("spawn_two");
    }

    let bytes = save(&src);

    let mut dst = EcsMaster::new();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load");

    assert_eq!(report.entities_loaded, 3);
    assert_eq!(report.archetypes_loaded, 1);
    assert_eq!(report.columns_blitted, 2, "Position + Velocity are POB blits");
    assert_eq!(report.columns_decoded, 0);
    assert_eq!(report.types_skipped, 0);
    assert_eq!(dst.entity_count(), 3, "all three entities materialized");

    // Compare the full (Position, Velocity) multiset.
    let mut got: Vec<PosVelBits> = dst
        .query::<(&Position, &Velocity), ()>()
        .iter()
        .map(|(p, v)| ((p.x.to_bits(), p.y.to_bits(), p.z.to_bits()), (v.dx, v.dy)))
        .collect();
    got.sort_unstable();
    let mut want: Vec<PosVelBits> = rows
        .iter()
        .map(|(p, v)| ((p.x.to_bits(), p.y.to_bits(), p.z.to_bits()), (v.dx, v.dy)))
        .collect();
    want.sort_unstable();
    assert_eq!(got, want, "every (Position, Velocity) value must round-trip");
}

// ════════════════════════════════════════════════════════════════════════════
// Owning component: full equality through the decode path
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn owning_component_roundtrip_equality() {
    let mut src = EcsMaster::new();
    let arch = src.get_or_create_archetype(&[Inventory::component_id()]);
    let inventories = [
        Inventory { name: "sword".to_string(), flags: vec![1, 2, 3] },
        Inventory { name: "".to_string(), flags: vec![] },
        Inventory { name: "potion of healing".to_string(), flags: vec![255, 0, 128, 64] },
    ];
    for inv in inventories.iter().cloned() {
        src.spawn_one(arch, inv).expect("spawn owning");
    }

    let bytes = save(&src);

    let mut dst = EcsMaster::new();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load owning");

    assert_eq!(report.entities_loaded, 3);
    assert_eq!(report.columns_decoded, 1, "Inventory is a SerializeViaFn decode");
    assert_eq!(report.columns_blitted, 0);

    let mut got: Vec<Inventory> = dst
        .query::<&Inventory, ()>()
        .iter()
        .cloned()
        .collect();
    got.sort_by(|a, b| a.name.cmp(&b.name));
    let mut want: Vec<Inventory> = inventories.to_vec();
    want.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(got, want, "every Inventory value must round-trip through decode");
}

// ════════════════════════════════════════════════════════════════════════════
// Mixed POB + owning in one archetype: blit + decode in the same load
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn mixed_pob_and_owning_in_one_archetype() {
    let mut src = EcsMaster::new();
    let arch = src.get_or_create_archetype(&[Position::component_id(), Inventory::component_id()]);
    src.spawn_two(
        arch,
        Position { x: 7.0, y: 8.0, z: 9.0 },
        Inventory { name: "mixed".to_string(), flags: vec![42] },
    )
    .expect("spawn mixed");

    let bytes = save(&src);
    let mut dst = EcsMaster::new();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load mixed");

    assert_eq!(report.entities_loaded, 1);
    assert_eq!(report.columns_blitted, 1, "Position blit");
    assert_eq!(report.columns_decoded, 1, "Inventory decode");

    let got: Vec<(Position, Inventory)> = dst
        .query::<(&Position, &Inventory), ()>()
        .iter()
        .map(|(p, i)| (*p, i.clone()))
        .collect();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, Position { x: 7.0, y: 8.0, z: 9.0 });
    assert_eq!(got[0].1, Inventory { name: "mixed".to_string(), flags: vec![42] });
}

// ════════════════════════════════════════════════════════════════════════════
// Multi-archetype world: distinct shapes round-trip together
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn multi_archetype_roundtrip_equality() {
    let mut src = EcsMaster::new();
    let a_pos = src.get_or_create_archetype(&[Position::component_id()]);
    let a_pos_vel =
        src.get_or_create_archetype(&[Position::component_id(), Velocity::component_id()]);

    src.spawn_one(a_pos, Position { x: 100.0, y: 0.0, z: 0.0 }).expect("spawn pos");
    src.spawn_one(a_pos, Position { x: 200.0, y: 0.0, z: 0.0 }).expect("spawn pos");
    src.spawn_two(a_pos_vel, Position { x: 1.0, y: 1.0, z: 1.0 }, Velocity { dx: 5, dy: 6 })
        .expect("spawn pos+vel");

    let bytes = save(&src);
    let mut dst = EcsMaster::new();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load multi");

    assert_eq!(report.entities_loaded, 3);
    assert_eq!(report.archetypes_loaded, 2, "two distinct archetype shapes");

    // All three Positions survive (across both archetypes).
    let got = positions_sorted(&mut dst);
    let want = {
        let mut v = vec![
            (100.0f32.to_bits(), 0.0f32.to_bits(), 0.0f32.to_bits()),
            (200.0f32.to_bits(), 0.0f32.to_bits(), 0.0f32.to_bits()),
            (1.0f32.to_bits(), 1.0f32.to_bits(), 1.0f32.to_bits()),
        ];
        v.sort_unstable();
        v
    };
    assert_eq!(got, want, "every Position across both archetypes round-trips");

    // The one Velocity survives in the second archetype.
    let vels: Vec<(i32, i32)> =
        dst.query::<&Velocity, ()>().iter().map(|v| (v.dx, v.dy)).collect();
    assert_eq!(vels, vec![(5, 6)]);
}

// ════════════════════════════════════════════════════════════════════════════
// Empty world: a valid no-op load
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn empty_world_roundtrip() {
    let src = EcsMaster::new();
    let bytes = save(&src);

    let mut dst = EcsMaster::new();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load empty");

    assert_eq!(report.entities_loaded, 0);
    assert_eq!(report.archetypes_loaded, 0);
    assert_eq!(report.columns_blitted, 0);
    assert_eq!(report.columns_decoded, 0);
    assert_eq!(dst.entity_count(), 0);
}

// ════════════════════════════════════════════════════════════════════════════
// Truncated / malformed bytes → LoadError, destination world left consistent
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn bad_magic_is_rejected() {
    let mut dst = EcsMaster::new();
    let garbage = vec![0u8; 64];
    let err = load_world(&mut dst, &garbage, LoadEntityPolicy::Remap).unwrap_err();
    assert!(matches!(err, LoadError::BadMagic), "got {err:?}");
    assert_eq!(dst.entity_count(), 0, "a rejected load must not touch the world");
}

#[test]
fn truncated_header_is_rejected() {
    let mut dst = EcsMaster::new();
    let short = b"BOYKO".to_vec(); // shorter than the 64-byte header
    let err = load_world(&mut dst, &short, LoadEntityPolicy::Remap).unwrap_err();
    assert!(matches!(err, LoadError::BadMagic), "got {err:?}");
}

#[test]
fn truncated_body_is_rejected_and_world_stays_consistent() {
    let mut src = EcsMaster::new();
    let arch = src.get_or_create_archetype(&[Position::component_id(), Inventory::component_id()]);
    src.spawn_two(
        arch,
        Position { x: 1.0, y: 2.0, z: 3.0 },
        Inventory { name: "data".to_string(), flags: vec![1, 2, 3, 4] },
    )
    .expect("spawn");
    let mut bytes = save(&src);

    // Lop off the trailing column-data bytes so a decode runs off the end (or a POB
    // region is short). The header offsets now point past the truncated end.
    bytes.truncate(bytes.len() - 4);

    let mut dst = EcsMaster::new();
    let err = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).unwrap_err();
    // Either a region bound fails (Truncated) or the decode runs short (Decode) —
    // both are loud LoadErrors, never UB.
    assert!(
        matches!(err, LoadError::Truncated(_) | LoadError::Decode(_)),
        "a truncated body must be a loud LoadError, got {err:?}"
    );
    // The world must be consistent: no half-loaded archetype claiming live rows.
    // (The first archetype may have been created but rolled back to empty; entity
    // count must be 0 since the batch is registered only after full success.)
    assert_eq!(dst.entity_count(), 0, "no entity registered on a failed load");
}

#[test]
fn flipped_endianness_byte_is_rejected() {
    let src = EcsMaster::new();
    let mut bytes = save(&src);
    // Header byte 12 is `endianness`; flip it to the big-endian marker.
    bytes[12] = 1;
    let mut dst = EcsMaster::new();
    let err = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).unwrap_err();
    assert!(matches!(err, LoadError::EndiannessMismatch(1)), "got {err:?}");
}

#[test]
fn wrong_ptr_width_is_rejected() {
    let src = EcsMaster::new();
    let mut bytes = save(&src);
    // Header byte 13 is `ptr_width`; set it to 4 (32-bit, unsupported in v1).
    bytes[13] = 4;
    let mut dst = EcsMaster::new();
    let err = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).unwrap_err();
    assert!(matches!(err, LoadError::PtrWidthMismatch(4)), "got {err:?}");
}

// ════════════════════════════════════════════════════════════════════════════
// Fingerprint mismatch on a POB column → hard LoadError (C2)
// ════════════════════════════════════════════════════════════════════════════
//
// The registry installs a `deserialize_fn` ONLY for `SerializeViaFn` components
// (`install_serialize_fn` gates `deserialize_fn` on the ViaFn classification); a
// `PlainOldBytes` component installs `None` (it blits, never decodes). So a POB
// column whose `layout_fingerprint` no longer matches has NO decode fallback and is
// a C2 HARD ERROR — never a silent garbage blit, never a silent default.

#[test]
fn pob_fingerprint_mismatch_is_a_hard_error() {
    let mut src = EcsMaster::new();
    let arch = src.get_or_create_archetype(&[Position::component_id()]);
    src.spawn_one(arch, Position { x: 1.5, y: 2.5, z: 3.5 }).expect("spawn");
    src.spawn_one(arch, Position { x: -4.0, y: 0.0, z: 7.25 }).expect("spawn");
    let mut bytes = save(&src);

    // Corrupt the single type entry's `layout_fingerprint` (at type_table_off + 8) so
    // the blit guard fails. Position is POB with no decoder → hard error.
    let type_table_off = u64::from_le_bytes(bytes[16..24].try_into().unwrap()) as usize;
    let fp_off = type_table_off + 8;
    bytes[fp_off] ^= 0xFF;

    let mut dst = EcsMaster::new();
    let err = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).unwrap_err();
    assert!(
        matches!(err, LoadError::FingerprintMismatch(_)),
        "a POB column with a corrupt fingerprint and no decoder must hard-error, got {err:?}"
    );
    assert_eq!(dst.entity_count(), 0, "a rejected load must not register any entity");
}

// ════════════════════════════════════════════════════════════════════════════
// A file type absent in the load process → types_skipped (W1 lenient)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn absent_file_type_is_skipped() {
    // Save a world with two components, then rewrite ONE type's stable name (+ hash)
    // in the saved bytes to a name no registered component carries. On load that
    // type resolves to nothing → its column is skipped (types_skipped), and the
    // remaining column still loads.
    let mut src = EcsMaster::new();
    let arch = src.get_or_create_archetype(&[Position::component_id(), Velocity::component_id()]);
    src.spawn_two(arch, Position { x: 1.0, y: 2.0, z: 3.0 }, Velocity { dx: 9, dy: 9 })
        .expect("spawn");
    let mut bytes = save(&src);

    // Locate the Velocity type entry and overwrite its stable_name_hash + name-pool
    // bytes with an unregistered name. The name pool sits directly after the type
    // table; each entry records (name_off, name_len). We must replace the name with
    // one of the SAME byte length so no offsets shift.
    let type_table_off = u64::from_le_bytes(bytes[16..24].try_into().unwrap()) as usize;
    let type_count = u32::from_le_bytes(bytes[48..52].try_into().unwrap()) as usize;

    let vel_name = boyko_ecs::ecs::core::component::component_registry::get_serialize_info(
        Velocity::component_id().0,
    )
    .unwrap()
    .stable_name;

    let mut rewrote = false;
    for i in 0..type_count {
        let entry_off = type_table_off + i * 40;
        let name_off = u32::from_le_bytes(bytes[entry_off + 24..entry_off + 28].try_into().unwrap())
            as usize;
        let name_len = u32::from_le_bytes(bytes[entry_off + 28..entry_off + 32].try_into().unwrap())
            as usize;
        let name = std::str::from_utf8(&bytes[name_off..name_off + name_len]).unwrap();
        if name == vel_name {
            // Replace the name bytes with a same-length unregistered name. Build a
            // filler of identical length that no real component carries.
            let ghost: Vec<u8> = (0..name_len).map(|_| b'Z').collect();
            let ghost_hash = boyko_ecs::ecs::core::component::component_registry::fnv1a_64(&ghost);
            bytes[entry_off..entry_off + 8].copy_from_slice(&ghost_hash.to_le_bytes());
            bytes[name_off..name_off + name_len].copy_from_slice(&ghost);
            rewrote = true;
            break;
        }
    }
    assert!(rewrote, "the Velocity type entry must be found and rewritten");

    let mut dst = EcsMaster::new();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).expect("load with ghost");

    assert_eq!(report.types_skipped, 1, "the unregistered Velocity-shaped type is skipped");
    assert_eq!(report.entities_loaded, 1, "the entity still loads (with Position only)");
    // Position survived; Velocity was skipped (the loaded archetype has no Velocity).
    let positions = positions_sorted(&mut dst);
    assert_eq!(
        positions,
        vec![(1.0f32.to_bits(), 2.0f32.to_bits(), 3.0f32.to_bits())]
    );
    let vel_count = dst.query::<&Velocity, ()>().iter().count();
    assert_eq!(vel_count, 0, "the skipped Velocity column is absent in the loaded world");
}
