//! Phase S3 item 2 — per-component `format_version` is LOAD-BEARING for blittable
//! columns.
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §3.5 + S3 item 1/2. S3 item 1 made the
//! per-component `format_version` recorded in each [`TypeTableEntry`] gate the POB
//! blit fast path: a `PlainOldBytes` column whose file `format_version` differs from
//! the running type's is a hard [`LoadError::VersionMismatch`] (never a silent blit
//! of stale bytes), with VERSION-FIRST precedence over a simultaneous
//! `FingerprintMismatch` (the version is the deliberate user signal — review W4).
//!
//! These tests stomp the `format_version` u16 of a chosen type-table entry directly
//! in the saved bytes (mirroring the mutation idiom of
//! `load_roundtrip.rs::pob_fingerprint_mismatch_is_a_hard_error`) and assert the
//! load-side behaviour for BOTH the blittable case (Test A → hard error) and the
//! owning case (Test B → the accepted W1 boundary: a ViaFn column re-decodes across
//! a version bump and the loader does NOT detect a same-wire-structure semantic
//! change).
//!
//! The entry offset is `type_table_off + i * TypeTableEntry::SIZE + 32` (review O2 —
//! the per-entry stride is included; `format_version` is at byte offset 32 within
//! the 40-byte `TypeTableEntry`, see `format.rs`); the target entry is confirmed by
//! reading its stable name at that entry, NOT assumed to be entry 0.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_macros::Component;

use boyko_serialize::{LoadEntityPolicy, LoadError, SaveOptions, load_world, save_world};

// ── Test components ──────────────────────────────────────────────────────────

/// POB: `#[repr(C)]`, all-scalar → `PlainOldBytes` (blitted whole-column). The
/// version gate guards exactly this class.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Position {
    x: f32,
    y: f32,
    z: f32,
}

/// Owning component (`String` + `Vec<u8>`) → `SerializeViaFn` (decode path). Its
/// `deserialize_fn` re-decodes across a `format_version` bump, so a stomped version
/// is INVISIBLE (the accepted W1 boundary).
#[derive(Component, Clone, PartialEq, Debug)]
struct Inventory {
    name: String,
    flags: Vec<u8>,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Saves `world` to a fresh byte buffer.
fn save(world: &EcsMaster) -> Vec<u8> {
    let mut out = Vec::new();
    save_world(world, &SaveOptions::default(), &mut out).expect("save");
    out
}

/// Locates the type-table entry whose recorded stable name equals `name` and
/// returns its absolute byte offset (`type_table_off + i * 40`). Mirrors the
/// name-pool walk in `load_roundtrip.rs::absent_file_type_is_skipped` — the entry
/// records `(name_off, name_len)`, so the name is read from the name pool. Panics if
/// no entry matches (a bug in the test fixture).
fn find_type_entry_off(bytes: &[u8], name: &str) -> usize {
    let type_table_off = u64::from_le_bytes(bytes[16..24].try_into().unwrap()) as usize;
    let type_count = u32::from_le_bytes(bytes[48..52].try_into().unwrap()) as usize;
    for i in 0..type_count {
        let entry_off = type_table_off + i * 40;
        let name_off =
            u32::from_le_bytes(bytes[entry_off + 24..entry_off + 28].try_into().unwrap()) as usize;
        let name_len =
            u32::from_le_bytes(bytes[entry_off + 28..entry_off + 32].try_into().unwrap()) as usize;
        let got = std::str::from_utf8(&bytes[name_off..name_off + name_len]).unwrap();
        if got == name {
            return entry_off;
        }
    }
    panic!("no type entry named '{name}' in the saved bytes");
}

/// Reads the per-component `format_version` u16 at the entry whose absolute offset is
/// `entry_off` (byte offset 32 within the entry).
fn read_format_version(bytes: &[u8], entry_off: usize) -> u16 {
    u16::from_le_bytes(bytes[entry_off + 32..entry_off + 34].try_into().unwrap())
}

/// Writes `v` into the per-component `format_version` u16 at `entry_off + 32`.
fn write_format_version(bytes: &mut [u8], entry_off: usize, v: u16) {
    bytes[entry_off + 32..entry_off + 34].copy_from_slice(&v.to_le_bytes());
}

// ════════════════════════════════════════════════════════════════════════════
// Test A — a POB column with a stomped format_version is a hard VersionMismatch
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn pob_format_version_mismatch_is_a_hard_error() {
    let pos_name = component_registry::get_serialize_info(Position::component_id().0)
        .expect("invariant: Position has serialize info")
        .stable_name;

    let mut src = EcsMaster::new();
    let arch = src.get_or_create_archetype(&[Position::component_id()]);
    src.spawn_one(arch, Position { x: 1.5, y: 2.5, z: 3.5 }).expect("spawn");
    src.spawn_one(arch, Position { x: -4.0, y: 0.0, z: 7.25 }).expect("spawn");
    let mut bytes = save(&src);

    // Locate Position's type entry (NOT assumed to be entry 0) and read the running
    // version it was saved with, then stomp the FILE version to differ from it.
    let entry_off = find_type_entry_off(&bytes, pos_name);
    let running = read_format_version(&bytes, entry_off);
    let stomped = running.wrapping_add(1);
    write_format_version(&mut bytes, entry_off, stomped);

    let mut dst = EcsMaster::new();
    let err = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).unwrap_err();
    match err {
        LoadError::VersionMismatch { file, running: r, .. } => {
            assert_eq!(file, stomped, "the error must report the stomped file version");
            assert_eq!(r, running, "the error must report the running version");
        }
        other => panic!("a POB column with a stomped format_version must be a VersionMismatch, got {other:?}"),
    }
    assert_eq!(
        dst.entity_count(),
        0,
        "a rejected load must roll back / never register any entity"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Test B — an OWNING (ViaFn) column re-decodes across a version bump (W1 boundary)
// ════════════════════════════════════════════════════════════════════════════
//
// This pins the DELIBERATE v1 boundary documented on `LoadError::VersionMismatch`:
// the per-component version gate guards BLITTABLE columns only. An owning component
// has a `deserialize_fn` (it rebuilds the value from the wire structure), so a
// stomped `format_version` leaves the load UNAFFECTED — the loader cannot detect a
// purely-semantic field reinterpretation that keeps the wire structure unchanged.

#[test]
fn owning_format_version_bump_is_decoded_not_an_error() {
    // The owning boundary requires a real decoder; assert it before relying on it.
    assert!(
        component_registry::get_serialize_info(Inventory::component_id().0)
            .expect("invariant: Inventory has serialize info")
            .deserialize_fn
            .is_some(),
        "Inventory must be SerializeViaFn (a decoder installed) for this boundary test"
    );
    let inv_name = component_registry::get_serialize_info(Inventory::component_id().0)
        .unwrap()
        .stable_name;

    let mut src = EcsMaster::new();
    let arch = src.get_or_create_archetype(&[Inventory::component_id()]);
    src.spawn_one(
        arch,
        Inventory { name: "sword".to_string(), flags: vec![1, 2, 3] },
    )
    .expect("spawn owning");
    src.spawn_one(arch, Inventory { name: String::new(), flags: Vec::new() })
        .expect("spawn empty owning");
    let mut bytes = save(&src);

    // Stomp Inventory's file format_version. A ViaFn column ignores it (re-decodes).
    let entry_off = find_type_entry_off(&bytes, inv_name);
    let running = read_format_version(&bytes, entry_off);
    write_format_version(&mut bytes, entry_off, running.wrapping_add(7));

    let mut dst = EcsMaster::new();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap)
        .expect("a ViaFn column re-decodes across a version bump — no error (W1 boundary)");
    assert_eq!(
        report.columns_decoded, 1,
        "the owning column must be DECODED (not blitted) across the version bump"
    );
    assert_eq!(dst.entity_count(), 2, "both owning rows must load");

    // Values must round-trip despite the stomped version (the decoder is version-blind).
    let mut got: Vec<(String, Vec<u8>)> = dst
        .query::<&Inventory, ()>()
        .iter()
        .map(|i| (i.name.clone(), i.flags.clone()))
        .collect();
    got.sort();
    let mut want = vec![
        ("sword".to_string(), vec![1u8, 2, 3]),
        (String::new(), Vec::new()),
    ];
    want.sort();
    assert_eq!(got, want, "owning values must survive a version-bumped load");
}
