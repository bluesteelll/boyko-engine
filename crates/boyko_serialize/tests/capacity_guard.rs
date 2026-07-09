//! Phase S3 C2 — the pool reserve-ceiling guard is now WRITER-side and LOUD.
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §3.11 (LOAD) + §5 (C2). The load WRITER
//! (`boyko_ecs::ecs::core::serialize::load_archetype`) reserves pool capacity for an
//! archetype's `n` rows via `Archetype::reserve_capacity(n)`. That call previously
//! `.expect()`-PANICKED when `n` exceeded a hosted pool's reserve ceiling; C2 turns
//! it into a LOUD, release-level `Err(LoadWriteError::CapacityExceeded)` that the
//! `boyko_serialize` loader maps to [`LoadError::CapacityExceeded`].
//!
//! # Why the OLD per-block load-side guard could not shadow site 302
//!
//! `reserve_capacity`'s ceiling check is ADDITIVE on the pool's current `len`
//! (`len + n <= reserve_rows`). The removed per-block guard capped `entity_count`
//! per file block IN ISOLATION. When TWO file blocks dedup-collapse onto ONE running
//! archetype — because `create_archetype` dedups identical id-sets (and `Ignore` /
//! skipped / bitset columns drop out of the signature) — the second block appends at
//! `start_row = pool.len > 0`. Two blocks with `e1 <= ceiling` and `e2 <= ceiling`
//! but `e1 + e2 > ceiling` BOTH pass an isolated per-block check yet overflow the
//! pool in aggregate at the second block's reserve. The writer-side gate sees the
//! additive `len` and is therefore the single authoritative check.
//!
//! # Test shape
//!
//! This test hand-forges a two-block snapshot whose blocks carry the SAME single
//! zero-sized POB column (same file-local type index ⇒ same id-set ⇒ dedup-collapse).
//! Block 1 carries a small real `e1`; block 2 is forged with `e2 == ceiling`, so
//! `e1 + e2 > ceiling`. A ZST POB column costs ZERO data bytes per row (its
//! `ColumnRegion.byte_len` is `n * 0 == 0`), so the only size cost is the per-block
//! entity-row table (`n * 8` bytes). Block 1's reserve succeeds (commits `e1` rows);
//! block 2's `reserve_capacity(ceiling)` trips Phase A (`e1 + ceiling > ceiling`)
//! BEFORE committing, returning the loud `Err` — never an `.expect()` panic.
//!
//! 0%-gate: `load_archetype` / `reserve_capacity` are reachable ONLY from
//! `load_world` (a cold path) — no spawn / iter / schedule path calls the changed
//! writer code, so the hot-path perf gate is unaffected (see the report grep-proof).

use std::panic::{AssertUnwindSafe, catch_unwind};

use boyko_ecs::ecs::constants::POOL_MAX_ROWS;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_macros::Component;

use boyko_serialize::format::{ArchetypeBlock, ColumnRegion, SaveHeader, TypeTableEntry};
use boyko_serialize::{LoadEntityPolicy, LoadError, SaveOptions, load_world, save_world};

/// A ZERO-sized POB component (a normal `StorageKind::Normal` ZST, NOT an enable
/// tag) — so its file column survives load classification (a bitset tag would be
/// W1-skipped with no pool) and its pool routes to the `POOL_MAX_ROWS` ZST ceiling.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct ZstMarker;

// ── Little-endian byte helpers over the forged snapshot ────────────────────────

fn read_u32(bytes: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap())
}

fn read_u64(bytes: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap())
}

fn write_u32(bytes: &mut [u8], off: usize, v: u32) {
    bytes[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

/// Header field offsets (from `format.rs::SaveHeader`).
const HDR_ARCH_TABLE_OFF: usize = 24;
const HDR_ARCH_COUNT: usize = 52;
const HDR_ENTITY_COUNT: usize = 56;

/// Saves a one-block world holding a single `ZstMarker` column with `e1` entities.
fn save_one_zst_block(e1: u32) -> Vec<u8> {
    let mut world = EcsMaster::new();
    let arch = world.get_or_create_archetype(&[ZstMarker::component_id()]);
    for _ in 0..e1 {
        world.spawn_one(arch, ZstMarker).expect("spawn zst marker");
    }
    let mut out = Vec::new();
    save_world(&world, &SaveOptions::default(), &mut out).expect("save");
    out
}

/// Forges a SECOND archetype block (appended at the file tail) that dedup-collapses
/// onto the first (same single `ZstMarker` column ⇒ same id-set) but declares `e2`
/// entities. All new bytes are appended after the existing body, so every absolute
/// offset already in the file stays valid; only `archetype_count` (and the
/// informational `entity_count`) are patched.
fn forge_second_collapsing_block(base: &mut Vec<u8>, e2: u32) {
    // The first block is the only one; read its type-index array to learn the
    // file-local type index of the single ZST column (so the forged block names the
    // SAME type → the same id-set → a dedup-collapse).
    let arch_table_off = read_u64(base, HDR_ARCH_TABLE_OFF) as usize;
    let first_component_count = read_u32(base, arch_table_off);
    assert_eq!(first_component_count, 1, "fixture: one-column ZST block expected");
    let first_type_indices_off = read_u32(base, arch_table_off + 8) as usize;
    let zst_type_index = read_u32(base, first_type_indices_off);

    // Append the forged block's BODY first (type-index array, column-region array,
    // entity-row table), recording each region's absolute offset, then append the
    // 24-byte block header pointing at them. Appending at the tail keeps every prior
    // absolute offset valid; the loader walks blocks via `entity_rows_off + e*8`, so
    // the existing first block's "next block" offset is irrelevant (the loader is
    // driven by `archetype_count`, reading blocks back-to-back from
    // `archetype_table_off` — see the note below).
    //
    // NOTE on block ordering: the loader reads `archetype_count` blocks starting at
    // `archetype_table_off`, advancing each time to `entity_rows_off + entity_count*8`
    // (`load.rs::load_one_archetype` return). So block 2 must be reachable as that
    // computed offset AFTER block 1. The real saver lays block 1's header at
    // `archetype_table_off` and its entity-row table such that the next-block offset
    // equals the tail of block 1's body. To make the FORGED block 2 land where the
    // loader looks, we append block 2's HEADER immediately at the post-block-1 offset
    // the loader computes, then its body further along.
    let first_entity_count = read_u32(base, arch_table_off + 4);
    let first_entity_rows_off = read_u32(base, arch_table_off + 16) as usize;
    let block2_header_off = first_entity_rows_off + first_entity_count as usize * 8;
    assert_eq!(
        block2_header_off,
        base.len(),
        "fixture: block 1's entity-row table must end at the file tail (one-block save)"
    );

    // Layout the forged block 2: [header(24)] [type_indices(4)] [column_regions(16)]
    // [entity_rows(e2*8)].
    let type_indices_off = block2_header_off + ArchetypeBlock::SIZE;
    let column_regions_off = type_indices_off + 4;
    let entity_rows_off = column_regions_off + ColumnRegion::SIZE;

    // Header.
    let mut hdr = vec![0u8; ArchetypeBlock::SIZE];
    write_u32(&mut hdr, 0, 1); // component_count
    write_u32(&mut hdr, 4, e2); // entity_count (forged)
    write_u32(&mut hdr, 8, type_indices_off as u32);
    write_u32(&mut hdr, 12, column_regions_off as u32);
    write_u32(&mut hdr, 16, entity_rows_off as u32);
    write_u32(&mut hdr, 20, 0); // _pad
    base.extend_from_slice(&hdr);

    // Type-index array: the single ZST type index (same as block 1 ⇒ collapse).
    base.extend_from_slice(&zst_type_index.to_le_bytes());

    // Column-region array: a ZST POB column carries zero data (`byte_len == n*0 == 0`).
    let region = ColumnRegion { data_off: 0, byte_len: 0 };
    base.extend_from_slice(&region.data_off.to_le_bytes());
    base.extend_from_slice(&region.byte_len.to_le_bytes());

    // Entity-row table: `e2` saved ids. Values are irrelevant (the reserve trips
    // before any row is materialized); zeroed bytes are valid `EntityId`s on the
    // read path. This is the only sizeable allocation (e2 * 8 bytes).
    base.extend(std::iter::repeat_n(0u8, e2 as usize * 8));

    // Patch the header counts: one more block, and the (informational) total entity
    // count.
    write_u32(base, HDR_ARCH_COUNT, 2);
    let total = read_u64(base, HDR_ENTITY_COUNT) + e2 as u64;
    base[HDR_ENTITY_COUNT..HDR_ENTITY_COUNT + 8].copy_from_slice(&total.to_le_bytes());
}

#[test]
fn two_blocks_collapsing_over_ceiling_is_a_loud_err_not_a_panic() {
    // Touch the component so the loader can resolve its stable name (the W1 contract).
    let _ = ZstMarker::component_id();

    let ceiling = POOL_MAX_ROWS;
    let e1: u32 = 4; // small real first block (commits 4 ZST rows — cheap)
    // Block 2 declares the full ceiling, so `e1 + e2 = 4 + ceiling > ceiling`.
    let e2: u32 = u32::try_from(ceiling).expect("POOL_MAX_ROWS fits u32 (< u32::MAX by invariant)");

    let mut snapshot = save_one_zst_block(e1);
    forge_second_collapsing_block(&mut snapshot, e2);

    // Sanity: the type-table layout we assumed (TypeTableEntry::SIZE, ColumnRegion)
    // is the format module's — referencing them keeps this test pinned to the format.
    const _: usize = TypeTableEntry::SIZE;
    const _: usize = SaveHeader::SIZE;

    let mut dst = EcsMaster::new();
    // The load MUST NOT panic — it must return the loud capacity error. `catch_unwind`
    // turns a regression (a re-introduced `.expect()` panic at site 302) into a test
    // failure rather than an abort.
    let result = catch_unwind(AssertUnwindSafe(|| {
        load_world(&mut dst, &snapshot, LoadEntityPolicy::Remap)
    }));

    match result {
        Ok(Ok(_)) => panic!(
            "load_world unexpectedly SUCCEEDED on a two-block snapshot whose collapsed \
             archetype declares {e1} + {ceiling} > ceiling rows"
        ),
        Ok(Err(LoadError::CapacityExceeded { requested, .. })) => {
            // The additive request the writer rejected is `e1 + e2`.
            assert!(
                requested as u64 >= ceiling as u64,
                "the rejected request must be the additive over-ceiling count"
            );
        }
        Ok(Err(other)) => panic!(
            "load_world returned the wrong error variant (expected CapacityExceeded): {other:?}"
        ),
        Err(_) => panic!(
            "load_world PANICKED on the additive over-ceiling collapse — C2 site 302 \
             must return Err(LoadError::CapacityExceeded), never .expect()-panic"
        ),
    }

    // The first block's rows were committed then the second block was rejected before
    // any of its rows materialized; the world holds only the first block's entities
    // (the loader does not roll back already-loaded archetypes on a later block's
    // error — the error propagates with the partial world, which is acceptable for a
    // hostile-input rejection: no UB, no panic).
    assert_eq!(
        dst.entity_count(),
        e1 as usize,
        "only the first (valid) block's rows should have materialized"
    );
}
