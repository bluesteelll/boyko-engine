//! Two-pass world save (Phase S1).
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §3.11 (SAVE — two-pass, W3) + §3.9 (file
//! format). Pass 1 walks the world read-only and computes EXACT byte sizes per
//! column and all file offsets, growing the output buffer ONCE. Pass 2 fills the
//! reserved regions: a `PlainOldBytes` column is blitted with one
//! `copy_nonoverlapping(column_base, region, count*stride)`; a `SerializeViaFn`
//! column loops rows through the registry `serialize_fn`; an `Ignore` column is
//! skipped.
//!
//! # Read-only-borrow soundness
//!
//! `world` is borrowed `&EcsMaster` for the WHOLE function — both passes. The
//! Pass-1 column walk captures each live POB column's base pointer
//! (`ComponentPool::buffer_ptr`, a write-once VM-stable base, Phase X.I) and
//! reuses it in Pass 2. No structural op runs between the passes, so the captured
//! base stays valid; the saver never mutates the world.
//!
//! # S1 boundary: the encode glue is not yet derived
//!
//! Phase S0 installs `serialize_fn = None` for EVERY component — even
//! `SerializeViaFn`-classified ones — because the derive does not yet emit a
//! per-element encoder (that is a later macro phase; this crate must not touch
//! `boyko_macros`). So in S1 a `SerializeViaFn` column whose `serialize_fn` is
//! `None` is encoded as a ZERO-length column region: the type table still records
//! it as `SerializeViaFn` (round-trip-inspect can see the classification) and the
//! save never panics on an owning component. When the derive emits the encoder,
//! the SAME loop here drives it with no change to this driver — the `is_some()`
//! branch is already wired.

use std::path::Path;
use std::ptr;

use boyko_ecs::ecs::core::component::component_registry::{self, Serializability};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::serialize::SaveCursor;
use boyko_ecs::ecs::identifiers::primitives::{ComponentId, EntityId};

use crate::error::SaveError;
use crate::format::{
    ArchetypeBlock, COLUMN_REGION_ALIGN, ColumnRegion, DenseStoreBlock, PERSIST_TICKS_FLAG,
    SaveHeader, TypeTableEntry,
};

/// Options controlling a save (plan §3.10).
#[derive(Default)]
pub struct SaveOptions {
    /// Persist per-row change-detection ticks. Setting this ORs
    /// [`PERSIST_TICKS_FLAG`] into the header's `flags` bit 0 (round-tripped — a
    /// loader observes the save-time intent via `LoadReport::persist_ticks_flag`),
    /// but S1 still always resets every row's ticks to the load-time `current_tick`:
    /// per-row tick-VALUE persistence is deferred to a later rung (plan §6).
    /// Setting this today changes only the recorded header bit, not the loaded tick
    /// values.
    pub persist_ticks: bool,
    /// When `Some`, only components for which the predicate returns `true` are
    /// serialized; the rest are skipped. `None` serializes every non-`Ignore`
    /// component.
    pub include_filter: Option<fn(ComponentId) -> bool>,
}

/// A per-column plan computed in Pass 1 and consumed in Pass 2.
struct ColumnPlan {
    /// Index into the file's distinct-type table (`TypeTableEntry[]`).
    type_index: u32,
    /// The classification: drives the blit-vs-fn-ptr-vs-skip branch in Pass 2.
    serializability: Serializability,
    /// The live column base pointer (`ComponentPool::buffer_ptr`), captured in
    /// Pass 1 and reused in Pass 2 for the POB blit. Null for an empty column.
    src_base: *const u8,
    /// Pre-serialized bytes for a `SerializeViaFn` column (computed once in Pass 1
    /// by running `serialize_fn` per row); empty for POB / `Ignore` / a ViaFn
    /// column whose `serialize_fn` is not yet installed (the S1 boundary).
    via_fn_bytes: Vec<u8>,
    /// The file offset this column's data is written at (computed in Pass 1).
    data_off: u64,
    /// The byte length of this column's data region (`count*stride` for POB, the
    /// `via_fn_bytes` length for ViaFn, 0 for `Ignore` / a ZST tag).
    byte_len: u64,
}

/// A per-archetype plan computed in Pass 1.
struct ArchetypePlan {
    columns: Vec<ColumnPlan>,
    entity_count: usize,
    /// Base pointer of the archetype's `entity_ids: Vec<EntityId>` column,
    /// captured O(1) in Pass 1 (no copy, no intermediate `Vec`). Pass 2 blits
    /// `entity_count` little-endian `u64`s from it in one memcpy (O-1). Valid
    /// for the whole `&world` borrow — a read-only save runs no structural op
    /// (Phase X.I stable-base discipline, same as the POB column `src_base`).
    entity_ids_base: *const EntityId,
    block_off: usize,
    type_indices_off: usize,
    column_regions_off: usize,
    entity_rows_off: usize,
}

/// A per-dense-store plan computed in Pass 1 and consumed in Pass 2 (Dense plan
/// D4 / Decision 7). A dense store's slot→entity map is NOT the archetype entity
/// list (a dense component is signature-excluded), so the saver gathers a
/// COMPACTED snapshot — the live members in `for_each_live` (insertion) order —
/// into owned buffers in Pass 1 (a read-only logical compaction; the live save
/// runs no structural op, so it cannot call the mutating `compact()`).
struct DenseStorePlan {
    /// Index into the file's distinct-type table.
    type_index: u32,
    /// The classification (drives blit-vs-fn-ptr; v1 dense is POB).
    serializability: Serializability,
    /// The live member count (== `s2e` length == column rows after compaction).
    member_count: usize,
    /// The saved owning `EntityId.0` per live slot, in slot order (the `s2e` table).
    s2e: Vec<u64>,
    /// The compacted live column bytes (`member_count * stride`), gathered in slot
    /// order. For a POB store this is the per-member byte image concatenated; for a
    /// ViaFn store it is the per-element `serialize_fn` run.
    data: Vec<u8>,
    /// The file offset of the `s2e` table (computed in Pass 1b).
    s2e_off: u64,
    /// The file offset of the column data (computed in Pass 1b).
    data_off: u64,
}

/// One distinct serialized type, deduplicated across archetypes.
struct TypeEntry {
    component_id: usize,
    stable_name_hash: u64,
    layout_fingerprint: u64,
    size: u32,
    align: u32,
    format_version: u16,
    serializability: Serializability,
    name_off: u32,
    name_len: u32,
    /// The stable name bytes (copied into the name pool in Pass 2).
    name: &'static str,
}

/// Rounds `off` up to the next multiple of `align` (a power of two). `None` on
/// overflow.
#[inline]
fn align_up(off: usize, align: usize) -> Option<usize> {
    debug_assert!(align.is_power_of_two());
    let mask = align - 1;
    off.checked_add(mask).map(|v| v & !mask)
}

/// Serializes `world` into `out` (appended), returning the number of bytes
/// written. Two-pass (plan §3.11 W3): no realloc mid-fill.
///
/// The world is read ONLY — no mutation. Components classified `Ignore` (or
/// excluded by `opts.include_filter`) are skipped; `PlainOldBytes` columns are
/// blitted; `SerializeViaFn` columns are encoded per-row via the registry
/// `serialize_fn` when one is installed (the S1 boundary leaves them zero-length
/// until the derive emits the encoder).
///
/// # Errors
///
/// [`SaveError::SizeOverflow`] if computing the file layout overflows `usize`.
pub fn save_world(
    world: &EcsMaster,
    opts: &SaveOptions,
    out: &mut Vec<u8>,
) -> Result<usize, SaveError> {
    let start_len = out.len();

    // ── Pass 1a: collect distinct types + per-archetype/column plan ──
    let mut types: Vec<TypeEntry> = Vec::new();
    let mut archetype_plans: Vec<ArchetypePlan> = Vec::new();

    for archetype in world.archetype_master().iter_archetypes() {
        let entity_count = archetype.entity_count();
        let mut columns: Vec<ColumnPlan> = Vec::new();

        for &component_id in archetype.component_ids() {
            let cid: usize = component_id.0;

            let info = component_registry::get_serialize_info(cid);
            let serializability = info
                .map(|i| i.serializability)
                .unwrap_or(Serializability::Ignore);
            let excluded = opts
                .include_filter
                .map(|f| !f(component_id))
                .unwrap_or(false);
            if serializability == Serializability::Ignore || excluded {
                continue;
            }
            let info = info.expect("a non-Ignore classification implies installed info");

            let pool = match archetype.component_pools().get_pool(component_id) {
                Some(p) => p,
                None => continue,
            };
            let stride = pool.component_layout().size();
            let count = pool.count();
            debug_assert_eq!(
                count, entity_count,
                "a live column's row count must equal the archetype entity count"
            );

            let type_index = intern_type(&mut types, cid, info)?;

            // Pass 1 sizing of a `SerializeViaFn` column: run the encoder once per
            // row into a scratch cursor (kept + reused in Pass 2 — `serialize_fn`
            // runs exactly once total). Zero-length when no encoder is installed
            // (the S1 boundary).
            let mut via_fn_bytes = Vec::new();
            if serializability == Serializability::SerializeViaFn
                && let Some(serialize_fn) = info.serialize_fn
            {
                let src_base = pool.buffer_ptr();
                let mut cursor = SaveCursor::new(&mut via_fn_bytes);
                for row in 0..count {
                    // SAFETY: `row < count == pool.count()`, so the slot at
                    // `src_base + row*stride` is a live, initialized value of this
                    // component's type, aligned per `buffer_ptr`'s SIMD-A1
                    // contract; `src_base` is the pool's stable write-once base.
                    let row_ptr = unsafe { src_base.add(row * stride) };
                    // SAFETY: `row_ptr` satisfies `SerializeFn`'s contract (live,
                    // aligned, initialized `C`, readable for `size_of::<C>()`); the
                    // cursor is a valid append-only sink (registry `SerializeFn`
                    // safety doc).
                    unsafe { serialize_fn(row_ptr, &mut cursor) };
                }
            }

            let (byte_len, src_base): (u64, *const u8) = match serializability {
                Serializability::PlainOldBytes => {
                    let len = (count as u64)
                        .checked_mul(stride as u64)
                        .ok_or(SaveError::SizeOverflow)?;
                    // A ZST tag (stride 0) has len 0 and needs no source.
                    let base = if len == 0 {
                        ptr::null()
                    } else {
                        pool.buffer_ptr()
                    };
                    (len, base)
                }
                Serializability::SerializeViaFn => (via_fn_bytes.len() as u64, ptr::null()),
                Serializability::Ignore => (0, ptr::null()),
            };

            columns.push(ColumnPlan {
                type_index,
                serializability,
                src_base,
                via_fn_bytes,
                data_off: 0,
                byte_len,
            });
        }

        // O-1: capture the entity-id column base (O(1) — no per-row gather, no
        // intermediate `Vec<u64>`). Pass 2 blits it in one memcpy. The base is
        // stable for the whole `&world` borrow (a read-only save runs no
        // structural op), exactly like the POB column `src_base`.
        let id_slice = archetype.entity_ids_slice();
        debug_assert_eq!(
            id_slice.len(),
            entity_count,
            "entity_ids slice length must equal the archetype entity count"
        );
        let entity_ids_base = id_slice.as_ptr();

        archetype_plans.push(ArchetypePlan {
            columns,
            entity_count,
            entity_ids_base,
            block_off: 0,
            type_indices_off: 0,
            column_regions_off: 0,
            entity_rows_off: 0,
        });
    }

    // ── Pass 1a-dense: gather a COMPACTED snapshot of every live dense store ──
    // (Dense plan D4 / Decision 7). A dense component is signature-excluded, so its
    // slot→entity association is the store's `s2e`, serialized alongside the data.
    // The live members are gathered in `for_each_live` (insertion) order — a
    // read-only logical compaction that skips tombstones, yielding the same
    // tombstone-free canonical order a `compact()` would, without mutating the
    // world. Empty for a table-only world (the 0%-gate: `dense_ids()` is empty).
    let mut dense_plans: Vec<DenseStorePlan> = Vec::new();
    for &cid in world.dense_registry().dense_ids() {
        let component_id = cid.0;
        let info = component_registry::get_serialize_info(component_id);
        let serializability = info
            .map(|i| i.serializability)
            .unwrap_or(Serializability::Ignore);
        let excluded = opts
            .include_filter
            .map(|f| !f(cid))
            .unwrap_or(false);
        if serializability == Serializability::Ignore || excluded {
            continue;
        }
        let info = info.expect("a non-Ignore classification implies installed info");
        let Some(store) = world.dense_registry().store(cid) else {
            continue;
        };
        let member_count = store.live_count();
        if member_count == 0 {
            continue; // an empty dense store emits no block (matching the archetypes).
        }
        let type_index = intern_type(&mut types, component_id, info)?;
        let stride = store.stride_bytes();

        // Gather the live members in slot (insertion) order. `for_each_live` yields
        // `(slot, entity)` skipping tombstones; the compacted snapshot is the live
        // members concatenated in that order — deterministic for a fixed op order.
        let mut s2e: Vec<u64> = Vec::with_capacity(member_count);
        let mut data: Vec<u8> = Vec::with_capacity(member_count.checked_mul(stride).ok_or(SaveError::SizeOverflow)?);
        match serializability {
            Serializability::PlainOldBytes => {
                store.for_each_live(|slot, entity| {
                    s2e.push(entity.0 as u64);
                    if stride != 0 {
                        data.extend_from_slice(store.row_bytes(slot));
                    }
                });
            }
            Serializability::SerializeViaFn => {
                // A ViaFn dense component: run the encoder per live member into the
                // shared `data` cursor. Zero-length when no encoder is installed (the
                // S1 boundary — same as the table ViaFn path).
                if let Some(serialize_fn) = info.serialize_fn {
                    store.for_each_live(|slot, entity| {
                        s2e.push(entity.0 as u64);
                        let row = store.row_bytes(slot);
                        let mut cursor = SaveCursor::new(&mut data);
                        // SAFETY: `row.as_ptr()` is a live, aligned, initialized `C`
                        //   value readable for `stride` bytes (the `row_bytes`
                        //   contract); the cursor is a valid append-only sink (registry
                        //   `SerializeFn` safety doc).
                        unsafe { serialize_fn(row.as_ptr(), &mut cursor) };
                    });
                } else {
                    // No encoder yet: record memberships only (zero-length data),
                    // mirroring the table ViaFn S1 boundary.
                    store.for_each_live(|_slot, entity| s2e.push(entity.0 as u64));
                }
            }
            Serializability::Ignore => continue,
        }
        debug_assert_eq!(s2e.len(), member_count, "dense s2e length must equal live count");

        dense_plans.push(DenseStorePlan {
            type_index,
            serializability,
            member_count,
            s2e,
            data,
            s2e_off: 0,
            data_off: 0,
        });
    }

    // ── Pass 1b: lay out every region; compute all file offsets ──
    let type_table_off = SaveHeader::SIZE;
    let type_table_len = types
        .len()
        .checked_mul(TypeTableEntry::SIZE)
        .ok_or(SaveError::SizeOverflow)?;

    // Name pool (concatenated stable names) directly after the type table.
    let name_pool_off = type_table_off
        .checked_add(type_table_len)
        .ok_or(SaveError::SizeOverflow)?;
    let mut cursor = name_pool_off;
    for t in &mut types {
        let len = t.name.len();
        t.name_off = u32::try_from(cursor).map_err(|_| SaveError::SizeOverflow)?;
        t.name_len = u32::try_from(len).map_err(|_| SaveError::SizeOverflow)?;
        cursor = cursor.checked_add(len).ok_or(SaveError::SizeOverflow)?;
    }
    let name_pool_end = cursor;

    // Archetype-block region: [header | type_indices | column_regions | rows] per
    // block. Column DATA lives in a separate region after all blocks.
    let archetype_table_off = name_pool_end;
    let mut cursor = archetype_table_off;
    for plan in &mut archetype_plans {
        let cc = plan.columns.len();
        plan.block_off = cursor;
        cursor = cursor
            .checked_add(ArchetypeBlock::SIZE)
            .ok_or(SaveError::SizeOverflow)?;
        plan.type_indices_off = cursor;
        cursor = cursor
            .checked_add(cc.checked_mul(4).ok_or(SaveError::SizeOverflow)?)
            .ok_or(SaveError::SizeOverflow)?;
        plan.column_regions_off = cursor;
        cursor = cursor
            .checked_add(
                cc.checked_mul(ColumnRegion::SIZE)
                    .ok_or(SaveError::SizeOverflow)?,
            )
            .ok_or(SaveError::SizeOverflow)?;
        plan.entity_rows_off = cursor;
        cursor = cursor
            .checked_add(plan.entity_count.checked_mul(8).ok_or(SaveError::SizeOverflow)?)
            .ok_or(SaveError::SizeOverflow)?;
    }

    // Column data region (continues from the archetype-table end in `cursor`):
    // each non-empty column aligned to COLUMN_REGION_ALIGN for a future mmap-cast
    // load.
    for plan in &mut archetype_plans {
        for col in &mut plan.columns {
            if col.byte_len == 0 {
                col.data_off = cursor as u64; // valid offset, no bytes reserved.
                continue;
            }
            // COLUMN_REGION_ALIGN (32) >= every supported component's alignment
            // (the pool's SIMD-A1 base bound is 32), so it satisfies the future
            // mmap-cast load's per-column alignment requirement (plan §3.9).
            let aligned = align_up(cursor, COLUMN_REGION_ALIGN).ok_or(SaveError::SizeOverflow)?;
            col.data_off = aligned as u64;
            cursor = aligned
                .checked_add(col.byte_len as usize)
                .ok_or(SaveError::SizeOverflow)?;
        }
    }
    let column_data_end = cursor;

    // Entity table + var_data: S2 populates these; S1 reserves zero-length regions
    // at the end so the header offsets are valid.
    let entity_table_off = column_data_end;
    let var_data_off = entity_table_off;
    let mut cursor = var_data_off;

    // Dense-store region (Dense plan D4): per store `[DenseStoreBlock header | s2e
    // u64[] | column data]`. The block headers come first (so the loader strides
    // them contiguously), then each store's body. Empty for a table-only world —
    // `dense_table_off` then points at the file end and `dense_store_count == 0`,
    // so no dense bytes are emitted (the 0%-gate).
    let dense_table_off = cursor;
    // Block headers (contiguous `DenseStoreBlock[dense_store_count]`).
    cursor = cursor
        .checked_add(
            dense_plans
                .len()
                .checked_mul(DenseStoreBlock::SIZE)
                .ok_or(SaveError::SizeOverflow)?,
        )
        .ok_or(SaveError::SizeOverflow)?;
    // Per-store body: `s2e` table (8 * member_count) then the column data
    // (COLUMN_REGION_ALIGN-rounded, like a table column, for a future mmap-cast).
    for plan in &mut dense_plans {
        plan.s2e_off = cursor as u64;
        cursor = cursor
            .checked_add(plan.member_count.checked_mul(8).ok_or(SaveError::SizeOverflow)?)
            .ok_or(SaveError::SizeOverflow)?;
        if plan.data.is_empty() {
            plan.data_off = cursor as u64; // valid offset, no bytes reserved.
            continue;
        }
        let aligned = align_up(cursor, COLUMN_REGION_ALIGN).ok_or(SaveError::SizeOverflow)?;
        plan.data_off = aligned as u64;
        cursor = aligned
            .checked_add(plan.data.len())
            .ok_or(SaveError::SizeOverflow)?;
    }

    let added = cursor; // file size == appended slice length.

    // Reserve capacity ONCE — capacity only, NO zero-fill (principle 5). Pass 2
    // appends every byte of the body sequentially in file-offset order, so each
    // byte is written exactly once: real content via `extend_from_slice` /
    // `copy_nonoverlapping`, alignment-padding gaps via an explicit zero-append.
    // The previous `out.resize(start_len + added, 0)` zero-filled the whole ~14 MB
    // body only to overwrite almost all of it — pure memset waste.
    out.reserve(added);

    // ── Pass 2: write the body sequentially, file-offset order ──
    // `out.len()` advances in lockstep with the file offset; `start_len + o` is the
    // buffer position of file offset `o`. The regions below are emitted in EXACTLY
    // the order Pass 1 laid them out (header → type table → name pool → archetype
    // blocks/bodies → column data), so a plain append reproduces the layout with no
    // backpatch and no gaps other than the column-region alignment padding.

    // Header — section offsets are all known from Pass 1, so write the real header
    // up front (no backpatch needed).
    let mut header = SaveHeader::new();
    header.type_table_off = type_table_off as u64;
    header.archetype_table_off = archetype_table_off as u64;
    header.entity_table_off = entity_table_off as u64;
    header.var_data_off = var_data_off as u64;
    header.type_count = u32::try_from(types.len()).map_err(|_| SaveError::SizeOverflow)?;
    header.archetype_count =
        u32::try_from(archetype_plans.len()).map_err(|_| SaveError::SizeOverflow)?;
    header.entity_count = world.entity_count() as u64;
    header.dense_table_off = dense_table_off as u64;
    header.dense_store_count =
        u32::try_from(dense_plans.len()).map_err(|_| SaveError::SizeOverflow)?;
    if opts.persist_ticks {
        header.flags |= PERSIST_TICKS_FLAG;
    }
    out.extend_from_slice(header.as_bytes());

    // Type table (contiguous `TypeTableEntry[]`), then the name pool (the
    // concatenated stable names, in the same interning order their `name_off`s were
    // assigned in Pass 1 — so appending each entry then, separately, each name
    // reproduces the type-table-then-name-pool layout).
    for t in &types {
        let entry = TypeTableEntry {
            stable_name_hash: t.stable_name_hash,
            layout_fingerprint: t.layout_fingerprint,
            size: t.size,
            align: t.align,
            name_off: t.name_off,
            name_len: t.name_len,
            format_version: t.format_version,
            serializability: t.serializability as u8,
            _pad: [0; 5],
        };
        out.extend_from_slice(type_entry_bytes(&entry));
    }
    for t in &types {
        out.extend_from_slice(t.name.as_bytes());
    }

    // Archetype-block region: per block `[header | type_indices | column_regions |
    // entity_rows]`, contiguous with no inter-section gaps (Pass 1 used plain
    // `checked_add` with no alignment rounding here).
    for plan in &archetype_plans {
        let cc = u32::try_from(plan.columns.len()).map_err(|_| SaveError::SizeOverflow)?;
        let block = ArchetypeBlock {
            component_count: cc,
            entity_count: u32::try_from(plan.entity_count).map_err(|_| SaveError::SizeOverflow)?,
            type_indices_off: u32::try_from(plan.type_indices_off)
                .map_err(|_| SaveError::SizeOverflow)?,
            column_regions_off: u32::try_from(plan.column_regions_off)
                .map_err(|_| SaveError::SizeOverflow)?,
            entity_rows_off: u32::try_from(plan.entity_rows_off)
                .map_err(|_| SaveError::SizeOverflow)?,
            _pad: 0,
        };
        out.extend_from_slice(archetype_block_bytes(&block));

        for col in &plan.columns {
            out.extend_from_slice(&col.type_index.to_le_bytes());
        }
        for col in &plan.columns {
            let region = ColumnRegion {
                data_off: col.data_off,
                byte_len: col.byte_len,
            };
            out.extend_from_slice(column_region_bytes(&region));
        }
        // Entity-row table (`u64[entity_count]`): the saved `EntityId.0`s in row
        // order. O-1: on a 64-bit little-endian target the archetype's
        // `&[EntityId]` byte image IS the little-endian `u64[]` table, so blit
        // the whole column in ONE memcpy instead of a per-row loop.
        #[cfg(all(target_endian = "little", target_pointer_width = "64"))]
        {
            // The reinterpret is byte-identical to the per-row
            // `(eid.0 as u64).to_le_bytes()` form ONLY when `EntityId` is
            // exactly 8 bytes (usize == u64); the cfg guarantees that here, and
            // this const guard documents/locks the invariant.
            const _: () = assert!(core::mem::size_of::<EntityId>() == 8);
            let len = plan.entity_count * core::mem::size_of::<EntityId>();
            // SAFETY: `entity_ids_base` is the stable base of the archetype's
            // `entity_ids: Vec<EntityId>`, captured in Pass 1; `world` is
            // borrowed `&` and no structural op runs during the read-only save,
            // so it is valid for `entity_count` initialized `EntityId`s
            // (asserted == slice len in Pass 1). `EntityId` is
            // `#[repr(transparent)]` over `usize`, so on a 64-bit little-endian
            // target these `len` bytes equal the previous per-row LE `u64`
            // images byte-for-byte. The slice is read-only and consumed
            // immediately by `extend_from_slice` (a copy into `out`), disjoint
            // from the engine VM.
            let bytes =
                unsafe { core::slice::from_raw_parts(plan.entity_ids_base as *const u8, len) };
            out.extend_from_slice(bytes);
        }
        #[cfg(not(all(target_endian = "little", target_pointer_width = "64")))]
        {
            // Scalar fallback (big-endian or non-64-bit usize): reproduce the
            // canonical little-endian `u64[]` table element by element.
            // SAFETY: same stable-base contract as the fast arm; reconstruct the
            // `&[EntityId]` for `entity_count` elements (== slice len, Pass 1).
            let ids =
                unsafe { core::slice::from_raw_parts(plan.entity_ids_base, plan.entity_count) };
            for &eid in ids {
                out.extend_from_slice(&(eid.0 as u64).to_le_bytes());
            }
        }
    }

    // Column data region: per column, in plan order. Each non-empty column was laid
    // out at a `COLUMN_REGION_ALIGN`-rounded offset (`col.data_off`); the rounding
    // bytes between the previous region's end and this column's start are the ONLY
    // gaps in the body and are zeroed explicitly here. An empty column (`byte_len ==
    // 0`) reserved no bytes and was given the un-rounded cursor as its `data_off`,
    // so it contributes nothing and needs no padding.
    for plan in &archetype_plans {
        for col in &plan.columns {
            if col.byte_len == 0 {
                continue; // ZST tag / Ignore / S1-boundary ViaFn: zero-length region.
            }
            // Explicit zero-fill of the alignment-padding gap. `col.data_off` is the
            // 32-rounded file offset; `out.len() - start_len` is the current file
            // offset (the previous region's end). The difference is the pad width.
            let cur_off = out.len() - start_len;
            let pad = (col.data_off as usize) - cur_off;
            debug_assert!(
                pad < COLUMN_REGION_ALIGN,
                "alignment padding must be smaller than the alignment"
            );
            out.resize(out.len() + pad, 0);
            debug_assert_eq!(
                out.len() - start_len,
                col.data_off as usize,
                "write head must be at the column's planned offset after padding"
            );

            match col.serializability {
                Serializability::PlainOldBytes => {
                    debug_assert!(!col.src_base.is_null());
                    let len = col.byte_len as usize;
                    // Append the column blit in one shot. `extend_from_slice` over a
                    // `&[u8]` aliasing the live column copies `len` bytes with no
                    // zero-fill; build the source slice from the captured base.
                    //
                    // SAFETY: `src_base` is the live POB column base captured in
                    // Pass 1 from `ComponentPool::buffer_ptr` (a write-once VM-stable
                    // base, valid for `count*stride == len` initialized bytes — every
                    // byte of a POB type is all-bits-valid, plan C3). `world` is still
                    // borrowed `&` and no structural op ran since capture, so the base
                    // is valid for reads of `len` bytes. The slice is consumed
                    // immediately by `extend_from_slice` (a `copy_nonoverlapping` into
                    // freshly-reserved `out` capacity), so it never outlives the
                    // borrow; source and dest are disjoint (engine-owned VM vs `out`).
                    let src = unsafe { std::slice::from_raw_parts(col.src_base, len) };
                    out.extend_from_slice(src);
                }
                Serializability::SerializeViaFn => {
                    // `byte_len != 0` here ⇒ an encoder was installed and produced
                    // these bytes in Pass 1.
                    out.extend_from_slice(&col.via_fn_bytes);
                }
                Serializability::Ignore => {}
            }
        }
    }

    // Dense-store region (Dense plan D4): the block headers first (contiguous),
    // then each store's body `[s2e u64[] | column data]`. The column data is
    // COLUMN_REGION_ALIGN-rounded (the only gaps in this region, zeroed explicitly).
    for plan in &dense_plans {
        let block = DenseStoreBlock {
            type_index: plan.type_index,
            member_count: u32::try_from(plan.member_count).map_err(|_| SaveError::SizeOverflow)?,
            serializability: plan.serializability as u8,
            _pad: [0; 7],
            s2e_off: plan.s2e_off,
            data_off: plan.data_off,
            data_byte_len: plan.data.len() as u64,
        };
        out.extend_from_slice(dense_store_block_bytes(&block));
    }
    for plan in &dense_plans {
        // `s2e` table: the saved owning `EntityId.0`s in slot order, little-endian.
        for &eid in &plan.s2e {
            out.extend_from_slice(&eid.to_le_bytes());
        }
        // Column data: zero-fill the alignment-padding gap, then append the bytes.
        if plan.data.is_empty() {
            continue;
        }
        let cur_off = out.len() - start_len;
        let pad = (plan.data_off as usize) - cur_off;
        debug_assert!(
            pad < COLUMN_REGION_ALIGN,
            "dense alignment padding must be smaller than the alignment"
        );
        out.resize(out.len() + pad, 0);
        debug_assert_eq!(
            out.len() - start_len,
            plan.data_off as usize,
            "write head must be at the dense column's planned offset after padding"
        );
        out.extend_from_slice(&plan.data);
    }

    // Every byte of the `added` region has now been written exactly once: the header
    // image, type table, name pool, and each archetype block/body via
    // `extend_from_slice`, the column blits via `extend_from_slice`, the dense-store
    // region (block headers + s2e + column data), and each alignment gap via an
    // explicit zero `resize`. No `set_len` over uninit capacity was used. This proves
    // no uninitialized byte reaches the file (the info-leak / non-reproducible-save
    // hazard).
    debug_assert_eq!(
        out.len(),
        start_len + added,
        "Pass 2 must write exactly `total` bytes; mismatch means a region was mis-sized or mis-ordered"
    );

    Ok(added)
}

/// Serializes `world` and writes the bytes to `path`, returning the byte count.
///
/// # Errors
///
/// [`SaveError::SizeOverflow`] (layout overflow) or [`SaveError::Io`] (write
/// failure).
pub fn save_world_to_file(
    world: &EcsMaster,
    opts: &SaveOptions,
    path: &Path,
) -> Result<usize, SaveError> {
    let mut buf = Vec::new();
    let written = save_world(world, opts, &mut buf)?;
    std::fs::write(path, &buf)?;
    Ok(written)
}

/// Interns a distinct type into the type table, returning its file-local index.
fn intern_type(
    types: &mut Vec<TypeEntry>,
    component_id: usize,
    info: &'static component_registry::SerializeInfo,
) -> Result<u32, SaveError> {
    if let Some(idx) = types.iter().position(|t| t.component_id == component_id) {
        return Ok(idx as u32);
    }
    let (size, align) = component_layout(component_id);
    types.push(TypeEntry {
        component_id,
        stable_name_hash: info.stable_name_hash,
        layout_fingerprint: info.layout_fingerprint,
        size: u32::try_from(size).map_err(|_| SaveError::SizeOverflow)?,
        align: u32::try_from(align).map_err(|_| SaveError::SizeOverflow)?,
        format_version: info.format_version,
        serializability: info.serializability,
        name_off: 0,
        name_len: 0,
        name: info.stable_name,
    });
    Ok((types.len() - 1) as u32)
}

/// Returns `(size, align)` of a registered component. The id came from a live
/// archetype's `component_ids()`, so its registry layout is always installed.
fn component_layout(component_id: usize) -> (usize, usize) {
    let layout = component_registry::get_layout(component_id)
        .expect("invariant: a component in a live archetype has an installed layout")
        .layout();
    (layout.size(), layout.align())
}

/// Reinterprets a [`TypeTableEntry`] as its `#[repr(C)]` byte image.
#[inline]
fn type_entry_bytes(entry: &TypeTableEntry) -> &[u8] {
    // SAFETY: `TypeTableEntry` is `#[repr(C)]` with an explicit, initialized `_pad`
    // byte and only all-bits-valid integer fields, so all `SIZE` bytes are
    // initialized; the slice borrows `entry`.
    unsafe {
        std::slice::from_raw_parts(
            entry as *const TypeTableEntry as *const u8,
            TypeTableEntry::SIZE,
        )
    }
}

/// Reinterprets an [`ArchetypeBlock`] as its `#[repr(C)]` byte image.
#[inline]
fn archetype_block_bytes(block: &ArchetypeBlock) -> &[u8] {
    // SAFETY: `ArchetypeBlock` is `#[repr(C)]` with an explicit, initialized `_pad`
    // and all-bits-valid integer fields; all `SIZE` bytes are initialized; the
    // slice borrows `block`.
    unsafe {
        std::slice::from_raw_parts(
            block as *const ArchetypeBlock as *const u8,
            ArchetypeBlock::SIZE,
        )
    }
}

/// Reinterprets a [`ColumnRegion`] as its `#[repr(C)]` byte image.
#[inline]
fn column_region_bytes(region: &ColumnRegion) -> &[u8] {
    // SAFETY: `ColumnRegion` is `#[repr(C)]`, two `u64`s, no padding, all
    // all-bits-valid; all `SIZE` bytes are initialized; the slice borrows `region`.
    unsafe {
        std::slice::from_raw_parts(
            region as *const ColumnRegion as *const u8,
            ColumnRegion::SIZE,
        )
    }
}

/// Reinterprets a [`DenseStoreBlock`] as its `#[repr(C)]` byte image.
#[inline]
fn dense_store_block_bytes(block: &DenseStoreBlock) -> &[u8] {
    // SAFETY: `DenseStoreBlock` is `#[repr(C)]` with an explicit, initialized `_pad`
    // and all-bits-valid integer fields; all `SIZE` bytes are initialized; the slice
    // borrows `block`.
    unsafe {
        std::slice::from_raw_parts(
            block as *const DenseStoreBlock as *const u8,
            DenseStoreBlock::SIZE,
        )
    }
}
