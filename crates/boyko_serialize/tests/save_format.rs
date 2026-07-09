//! Phase S1 — byte-inspection tests for the two-pass save.
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §7 Phase S1 ("Tests: round-trip-inspect a
//! POB-only world; an owning component"). NO round-trip equality yet — that needs
//! the S2 loader. These tests build a small world, save it, and assert the file
//! HEADER fields + that a known `PlainOldBytes` column's bytes appear verbatim at
//! its recorded [`ColumnRegion`] offset, and that an owning `String` component
//! encodes without panic + is marked `SerializeViaFn` in the type table.
//!
//! World-builder style mirrors `boyko_ecs/tests/d4_typed_write.rs`:
//! `EcsMaster::new()` + `get_or_create_archetype(&[ids])` + `spawn_one/two`, with
//! component ids minted by `#[derive(Component)]`'s `component_id()`.

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::{self, Serializability, fnv1a_64};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_macros::Component;

use boyko_serialize::format::{
    ArchetypeBlock, ColumnRegion, ENDIAN_LITTLE, FORMAT_VERSION, MAGIC, PERSIST_TICKS_FLAG,
    PTR_WIDTH, SaveHeader, TypeTableEntry,
};
use boyko_serialize::{LoadEntityPolicy, SaveOptions, load_world, save_world};

// ── Test components ──────────────────────────────────────────────────────────

// POB: repr(C), all f32 fields → PlainOldBytes (blitted whole-column).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Pos {
    x: f32,
    y: f32,
    z: f32,
}

// POB: repr(C), all-int fields.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Vel {
    dx: i32,
    dy: i32,
}

// Owning component (String) → SerializeViaFn.
#[derive(Component, Clone)]
#[repr(C)]
struct Label {
    text: String,
}

// ── Little-endian readers over the saved bytes ───────────────────────────────

fn read_u32(bytes: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap())
}

fn read_u64(bytes: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap())
}

/// Parses the fixed-size header from the saved bytes (v2: 80 B — the v1 64-byte
/// image plus the Dense plan D4 dense-region descriptor at offsets 64..80).
fn parse_header(bytes: &[u8]) -> SaveHeader {
    assert!(bytes.len() >= SaveHeader::SIZE, "file shorter than the header");
    SaveHeader {
        magic: bytes[0..8].try_into().unwrap(),
        format_version: read_u32(bytes, 8),
        endianness: bytes[12],
        ptr_width: bytes[13],
        flags: u16::from_le_bytes(bytes[14..16].try_into().unwrap()),
        type_table_off: read_u64(bytes, 16),
        archetype_table_off: read_u64(bytes, 24),
        entity_table_off: read_u64(bytes, 32),
        var_data_off: read_u64(bytes, 40),
        type_count: read_u32(bytes, 48),
        archetype_count: read_u32(bytes, 52),
        entity_count: read_u64(bytes, 56),
        dense_table_off: read_u64(bytes, 64),
        dense_store_count: read_u32(bytes, 72),
        _pad: 0,
    }
}

/// Reads one [`TypeTableEntry`] from the type table by index.
fn parse_type_entry(bytes: &[u8], type_table_off: u64, index: usize) -> TypeTableEntry {
    let off = type_table_off as usize + index * TypeTableEntry::SIZE;
    TypeTableEntry {
        stable_name_hash: read_u64(bytes, off),
        layout_fingerprint: read_u64(bytes, off + 8),
        size: read_u32(bytes, off + 16),
        align: read_u32(bytes, off + 20),
        name_off: read_u32(bytes, off + 24),
        name_len: read_u32(bytes, off + 28),
        format_version: u16::from_le_bytes(bytes[off + 32..off + 34].try_into().unwrap()),
        serializability: bytes[off + 34],
        _pad: bytes[off + 35..off + 40].try_into().unwrap(),
    }
}

/// Reads the single archetype block (S1 tests use one-archetype worlds).
fn parse_block(bytes: &[u8], block_off: u64) -> ArchetypeBlock {
    let off = block_off as usize;
    ArchetypeBlock {
        component_count: read_u32(bytes, off),
        entity_count: read_u32(bytes, off + 4),
        type_indices_off: read_u32(bytes, off + 8),
        column_regions_off: read_u32(bytes, off + 12),
        entity_rows_off: read_u32(bytes, off + 16),
        _pad: read_u32(bytes, off + 20),
    }
}

fn parse_column_region(bytes: &[u8], regions_off: u32, col: usize) -> ColumnRegion {
    let off = regions_off as usize + col * ColumnRegion::SIZE;
    ColumnRegion {
        data_off: read_u64(bytes, off),
        byte_len: read_u64(bytes, off + 8),
    }
}

/// Returns the file-local type index of `column` within the single archetype
/// block, plus its column position (they coincide with insertion order).
fn type_index_of(bytes: &[u8], type_indices_off: u32, col: usize) -> u32 {
    read_u32(bytes, type_indices_off as usize + col * 4)
}

// ════════════════════════════════════════════════════════════════════════════
// POB-only world: header + verbatim column bytes
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn pob_world_header_and_verbatim_column_bytes() {
    let mut world = EcsMaster::new();
    let arch = world.get_or_create_archetype(&[Pos::component_id(), Vel::component_id()]);

    let rows = [
        (Pos { x: 1.0, y: 2.0, z: 3.0 }, Vel { dx: 10, dy: 20 }),
        (Pos { x: 4.5, y: 5.5, z: 6.5 }, Vel { dx: -7, dy: 8 }),
        (Pos { x: -1.25, y: 0.0, z: 99.0 }, Vel { dx: 0, dy: 0 }),
    ];
    for (p, v) in rows {
        world.spawn_two(arch, p, v).expect("spawn_two POB");
    }

    let mut out = Vec::new();
    let written = save_world(&world, &SaveOptions::default(), &mut out).expect("save");
    assert_eq!(written, out.len(), "returned length must equal bytes written");

    // ── Header ──
    let header = parse_header(&out);
    assert_eq!(header.magic, MAGIC, "magic must be b\"BOYKOSAV\"");
    assert_eq!(header.format_version, FORMAT_VERSION);
    assert_eq!(header.endianness, ENDIAN_LITTLE, "v1 writes little-endian");
    assert_eq!(header.ptr_width, PTR_WIDTH);
    assert_eq!(header.ptr_width, 8, "engine target is 64-bit");
    assert_eq!(header.type_count, 2, "Pos + Vel are two distinct types");
    assert_eq!(header.archetype_count, 1, "one archetype");
    assert_eq!(header.entity_count, 3, "three entities");
    assert_eq!(header.flags, 0);

    // ── Single archetype block ──
    let block = parse_block(&out, header.archetype_table_off);
    assert_eq!(block.component_count, 2);
    assert_eq!(block.entity_count, 3);

    // Locate the Pos column (the column whose file-local type entry has Pos's
    // stable-name hash). component_ids() order is not guaranteed, so resolve by
    // name hash.
    let pos_hash = fnv1a_64(
        component_registry::get_serialize_info(Pos::component_id().0)
            .unwrap()
            .stable_name
            .as_bytes(),
    );
    let mut pos_col: Option<usize> = None;
    for col in 0..block.component_count as usize {
        let ti = type_index_of(&out, block.type_indices_off, col);
        let entry = parse_type_entry(&out, header.type_table_off, ti as usize);
        if entry.stable_name_hash == pos_hash {
            assert_eq!(entry.serializability, Serializability::PlainOldBytes as u8);
            assert_eq!(entry.size, std::mem::size_of::<Pos>() as u32);
            assert_eq!(entry.align, std::mem::align_of::<Pos>() as u32);
            pos_col = Some(col);
        }
    }
    let pos_col = pos_col.expect("Pos column must be present");

    // The Pos column region holds count * stride bytes.
    let region = parse_column_region(&out, block.column_regions_off, pos_col);
    let stride = std::mem::size_of::<Pos>();
    assert_eq!(region.byte_len as usize, 3 * stride, "POB region == count*stride");

    // The bytes at the region must equal the verbatim little-endian f32 encoding
    // of the three Pos rows (in spawn/row order).
    let mut expected = Vec::new();
    for (p, _) in rows {
        expected.extend_from_slice(&p.x.to_le_bytes());
        expected.extend_from_slice(&p.y.to_le_bytes());
        expected.extend_from_slice(&p.z.to_le_bytes());
    }
    let start = region.data_off as usize;
    let actual = &out[start..start + region.byte_len as usize];
    assert_eq!(actual, &expected[..], "POB column bytes must appear verbatim");

    // The column region must be aligned (the SIMD-cast guarantee, plan §3.9).
    assert_eq!(region.data_off % 32, 0, "POB column region must be 32-aligned");
}

// ════════════════════════════════════════════════════════════════════════════
// Owning component: encodes without panic + marked SerializeViaFn
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn owning_string_component_encodes_and_is_marked_via_fn() {
    let mut world = EcsMaster::new();
    let arch = world.get_or_create_archetype(&[Label::component_id()]);
    world
        .spawn_one(arch, Label { text: "hello".to_string() })
        .expect("spawn owning");
    world
        .spawn_one(arch, Label { text: "world!!".to_string() })
        .expect("spawn owning");

    // Must NOT panic on an owning component; with the S1.5 derived encoder the
    // column now carries real bytes (was zero-length under the S1 boundary).
    let mut out = Vec::new();
    let written = save_world(&world, &SaveOptions::default(), &mut out).expect("save owning");
    assert_eq!(written, out.len());

    let header = parse_header(&out);
    assert_eq!(header.magic, MAGIC);
    assert_eq!(header.type_count, 1, "one type (Label)");
    assert_eq!(header.archetype_count, 1);
    assert_eq!(header.entity_count, 2);

    // The single type-table entry must be marked SerializeViaFn.
    let entry = parse_type_entry(&out, header.type_table_off, 0);
    assert_eq!(
        entry.serializability,
        Serializability::SerializeViaFn as u8,
        "an owning String component must be marked SerializeViaFn in the type table"
    );

    // The Label column now encodes via the derived `serialize_fn` (Phase S1.5): each
    // `String` is a `u32` LE length prefix + the UTF-8 bytes. "hello" → 4 + 5 = 9,
    // "world!!" → 4 + 7 = 11, total 20 bytes for the two rows.
    let block = parse_block(&out, header.archetype_table_off);
    assert_eq!(block.component_count, 1);
    assert_eq!(block.entity_count, 2);
    let region = parse_column_region(&out, block.column_regions_off, 0);
    let expected = (4 + 5) + (4 + 7);
    assert_eq!(
        region.byte_len, expected,
        "S1.5: a SerializeViaFn String column encodes u32-length-prefixed UTF-8 bytes"
    );
    assert!(region.byte_len > 0, "owning column must now carry real bytes");

    // The encoded bytes must appear verbatim at the recorded offset: row 0's prefix
    // is 5, then "hello"; row 1's prefix is 7, then "world!!".
    let start = region.data_off as usize;
    assert_eq!(read_u32(&out, start), 5, "row 0 String length prefix");
    assert_eq!(&out[start + 4..start + 9], b"hello", "row 0 UTF-8 bytes");
    assert_eq!(read_u32(&out, start + 9), 7, "row 1 String length prefix");
    assert_eq!(&out[start + 13..start + 20], b"world!!", "row 1 UTF-8 bytes");
}

// ════════════════════════════════════════════════════════════════════════════
// Empty world: a valid, well-formed header with zero counts
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn empty_world_saves_a_valid_header() {
    let world = EcsMaster::new();
    let mut out = Vec::new();
    let written = save_world(&world, &SaveOptions::default(), &mut out).expect("save empty");
    assert_eq!(written, out.len());
    assert!(written >= SaveHeader::SIZE, "even an empty world has a header");

    let header = parse_header(&out);
    assert_eq!(header.magic, MAGIC);
    assert_eq!(header.format_version, FORMAT_VERSION);
    assert_eq!(header.entity_count, 0);
    assert_eq!(header.archetype_count, 0);
    // No archetypes were created, so no types were interned.
    assert_eq!(header.type_count, 0);
}

// ════════════════════════════════════════════════════════════════════════════
// include_filter: a column excluded by the filter is not serialized
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn include_filter_excludes_a_component() {
    let mut world = EcsMaster::new();
    let arch = world.get_or_create_archetype(&[Pos::component_id(), Vel::component_id()]);
    world
        .spawn_two(arch, Pos { x: 1.0, y: 2.0, z: 3.0 }, Vel { dx: 4, dy: 5 })
        .expect("spawn");

    // Only Pos passes the filter.
    let opts = SaveOptions {
        persist_ticks: false,
        include_filter: Some(|cid| cid == Pos::component_id()),
    };
    let mut out = Vec::new();
    save_world(&world, &opts, &mut out).expect("save filtered");

    let header = parse_header(&out);
    assert_eq!(header.type_count, 1, "only Pos is serialized");
    let block = parse_block(&out, header.archetype_table_off);
    assert_eq!(block.component_count, 1, "Vel column excluded by the filter");

    let entry = parse_type_entry(&out, header.type_table_off, 0);
    let pos_hash = fnv1a_64(
        component_registry::get_serialize_info(Pos::component_id().0)
            .unwrap()
            .stable_name
            .as_bytes(),
    );
    assert_eq!(entry.stable_name_hash, pos_hash, "the kept column is Pos");
}

// ════════════════════════════════════════════════════════════════════════════
// persist_ticks: the option ORs PERSIST_TICKS_FLAG into the header and the loader
// reads it back (a save/load residual fix — the option used to be a silent no-op,
// recorded nowhere on disk).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn persist_ticks_true_sets_header_flag_and_is_observable_on_load() {
    let mut world = EcsMaster::new();
    let arch = world.get_or_create_archetype(&[Pos::component_id()]);
    world.spawn_one(arch, Pos { x: 1.0, y: 2.0, z: 3.0 }).expect("spawn");

    let opts = SaveOptions { persist_ticks: true, include_filter: None };
    let mut out = Vec::new();
    save_world(&world, &opts, &mut out).expect("save");

    let header = parse_header(&out);
    assert_eq!(
        header.flags & PERSIST_TICKS_FLAG,
        PERSIST_TICKS_FLAG,
        "persist_ticks: true must OR PERSIST_TICKS_FLAG into the header's flags bit 0"
    );

    let mut dst = EcsMaster::new();
    let report = load_world(&mut dst, &out, LoadEntityPolicy::Remap).expect("load");
    assert!(
        report.persist_ticks_flag,
        "the loader must read the header's PERSIST_TICKS_FLAG back into the report"
    );
}

#[test]
fn persist_ticks_false_leaves_header_flag_clear() {
    let mut world = EcsMaster::new();
    let arch = world.get_or_create_archetype(&[Pos::component_id()]);
    world.spawn_one(arch, Pos { x: 1.0, y: 2.0, z: 3.0 }).expect("spawn");

    let opts = SaveOptions { persist_ticks: false, include_filter: None };
    let mut out = Vec::new();
    save_world(&world, &opts, &mut out).expect("save");

    let header = parse_header(&out);
    assert_eq!(
        header.flags & PERSIST_TICKS_FLAG,
        0,
        "persist_ticks: false must leave the header's flags bit 0 clear"
    );

    let mut dst = EcsMaster::new();
    let report = load_world(&mut dst, &out, LoadEntityPolicy::Remap).expect("load");
    assert!(
        !report.persist_ticks_flag,
        "the loader must report the flag clear when the file did not set it"
    );
}
