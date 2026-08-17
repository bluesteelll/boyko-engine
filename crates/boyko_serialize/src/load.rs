//! World load — the `CopyIntoWorld` + `Remap` driver (Phase S2).
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §3.10 / §3.11 (LOAD) + §5 (C1/C2/W4). This
//! is the file-format PARSER half of the loader: it validates the header, resolves
//! the file's type table to running `ComponentId`s once, parses each archetype
//! block, classifies each column into a
//! [`LoadColumn`] instruction, and
//! hands one archetype at a time to
//! [`load_archetype`] — the
//! `boyko_ecs`-side WRITER that owns the crate-private row-write primitives.
//!
//! # Scope (Phase S2 + S2.5)
//!
//! S2 ships the headline `CopyIntoWorld` round-trip; S2.5 adds the ENTITY-REMAP
//! pass so saved `Entity` references survive a round-trip. Each archetype is loaded
//! with its Entity fields holding their RAW saved ids and the
//! [`LoadEntityMap`] records the
//! saved→fresh mapping; then a SEPARATE whole-world pass
//! ([`remap_loaded_entities`],
//! invoked here after the archetype loop) rewrites every remappable component
//! (`ChildOf` / an `#[entities]`-annotated field) to its freshly-allocated
//! `Entity`. A plain `Entity` field WITHOUT `#[entities]` stays the raw saved id
//! (the C4 explicit-opt-in decision). An unmapped saved id is a loud `LoadError`
//! (`Decode(UnmappedEntity)`), never silent dangling-ref corruption. `MmapInPlace`
//! (S3) and `PreserveIds` (W2) are out of scope — only `CopyIntoWorld` + `Remap`.
//!
//! # Untrusted-bytes discipline (C3)
//!
//! Every field read is bounds-checked against the input slice and every offset /
//! length is validated before use — a malformed stream surfaces as a [`LoadError`],
//! never UB. A `PlainOldBytes` column's `layout_fingerprint` is compared before any
//! blit (C2); a mismatch with no decode fallback is a HARD error. The per-element
//! `deserialize_fn` validates each bit-restricted field on read; on its first `Err`
//! the partially-loaded archetype is rolled back to empty by the writer.
//!
//! Two additional robustness guards harden the path against a HOSTILE (not just
//! truncated) file (the S2 review W1/W2):
//!
//! * **W1 (enable-tag column)**: a file column whose stable name resolves to a
//!   registered enable tag ([`StorageKind::Bitset`]) is SKIPPED in pass 1. A bitset
//!   id is filtered out of every archetype signature and has no `ComponentPool`, so
//!   feeding it to the writer would hit a pool-less `expect`; skipping it (counted
//!   in [`LoadReport::types_bitset_skipped`]) keeps a corrupt/foreign file a clean
//!   skip, never a panic. A self-save never emits such a column.
//! * **W2 (allocation DoS)**: every `Vec::with_capacity` driven by an untrusted
//!   count (`type_count`, `entity_count`, `component_count`) is capped against the
//!   bytes that could possibly back it (`capacity_hint`), so a forged count cannot
//!   force a multi-GiB reservation before the per-element bounds check rejects the
//!   stream.

use std::path::Path;

use boyko_ecs::ecs::core::component::component_registry::{self, Serializability, StorageKind};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::serialize::{
    LoadColumn, LoadEntityMap, load_archetype, load_dense_store, load_dense_store_via_fn,
    remap_loaded_entities, required_ctor_in_set,
};
use boyko_ecs::ecs::identifiers::primitives::{ComponentId, EntityId};
use boyko_log::codes::W0901;

use crate::error::LoadError;
use crate::format::{
    ArchetypeBlock, ColumnRegion, DenseStoreBlock, ENDIAN_LITTLE, FORMAT_VERSION, MAGIC,
    PERSIST_TICKS_FLAG, SaveHeader, TypeTableEntry, native_endianness,
};

/// Which strategy the loader uses to place saved entities (plan §3.10). v1 ships
/// only [`Remap`](LoadEntityPolicy::Remap); `PreserveIds` is deferred (W2).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum LoadEntityPolicy {
    /// Allocate FRESH entity ids for every saved entity and record the saved→fresh
    /// mapping (the normal case). Cross-entity references (`ChildOf` /
    /// `#[entities]`) are remapped through that map by the S2.5 pass.
    Remap,
}

/// Diagnostics returned by a successful load (plan §3.10).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LoadReport {
    /// Total entities materialized into the world.
    pub entities_loaded: u64,
    /// Archetypes created (one per file archetype block that loaded ≥ 0 rows).
    pub archetypes_loaded: u32,
    /// `PlainOldBytes` columns loaded with a single `copy_nonoverlapping`.
    pub columns_blitted: u32,
    /// `SerializeViaFn` columns loaded by the per-element `deserialize_fn`.
    pub columns_decoded: u32,
    /// File types that resolved to NO registered component in this build (W1
    /// lenient default): their columns were skipped entirely.
    pub types_skipped: u32,
    /// File types that resolved to a registered component classified as an enable
    /// tag ([`StorageKind::Bitset`]): these have NO `ComponentPool` (filtered out of
    /// every archetype signature), so their columns were skipped entirely (W1
    /// hardening). A self-save never emits such a column — the saver's signature
    /// excludes a bitset id — but a corrupt/foreign file CAN name one, and skipping
    /// it (rather than feeding the pool-less id to the writer) keeps a hostile file
    /// a clean skip instead of a panic. The owning entity stays valid without the
    /// tag (matching the clone path, which also drops the enable bit).
    pub types_bitset_skipped: u32,
    /// Columns that carried no data (a ViaFn column with no decoder, or an
    /// `Ignore`/skipped component the running archetype still includes) and were
    /// default-constructed via a `#[require]` ctor — or, when no ctor exists,
    /// excluded from the loaded archetype.
    pub types_defaulted: u32,
    /// Dense plan D4 — dense stores restored (one per file `DenseStoreBlock` whose
    /// type resolved to a registered, still-dense component). `0` for a table-only
    /// file (the 0%-gate).
    pub dense_stores_loaded: u32,
    /// Dense plan D4 — total dense memberships restored across all dense stores
    /// (the sum of each restored store's member count).
    pub dense_members_loaded: u64,
    /// Dense plan D4 — dense `DenseStoreBlock`s that carried memberships but no
    /// decodable data and were SKIPPED (their memberships dropped). Two cases reach
    /// here, both genuinely undecodable (NOT a data-loss regression):
    ///
    /// * an [`Ignore`](Serializability::Ignore) dense type — not serializable by
    ///   contract (the saver records memberships only);
    /// * a [`SerializeViaFn`](Serializability::SerializeViaFn) dense type with NO
    ///   installed `deserialize_fn` (the S1 memberships-only boundary — the saver
    ///   emitted a zero-length data run).
    ///
    /// An OWNING (`SerializeViaFn`) dense block WITH a decoder is now DECODED (the
    /// v1.1 per-member ViaFn dense path — counted in [`Self::dense_stores_loaded`] /
    /// [`Self::dense_members_loaded`]), not skipped. This counter (+ `boyko-W0901`,
    /// emitted in every profile since rung L8a) makes a genuine skip OBSERVABLE
    /// rather than silent data loss.
    /// Mirrors `types_skipped` / `types_bitset_skipped` on the table side.
    pub dense_stores_skipped: u32,
    /// Dense plan D4 — total dense memberships dropped by the undecodable dense
    /// skips counted in [`Self::dense_stores_skipped`] (the sum of each skipped
    /// block's `member_count`). The owning entities stay valid without their dense
    /// membership/value; this records how many were dropped.
    pub dense_members_skipped: u64,
    /// The file's [`PERSIST_TICKS_FLAG`] header bit, round-tripped from
    /// `SaveOptions::persist_ticks` (a save/load residual fix). `true` when the file
    /// was saved with that option set. Per-row tick VALUES are NOT restored by this
    /// rung regardless of this flag — every row is stamped fresh at the load-time
    /// `current_tick` (matching a plain load), same as before; this field exists so
    /// the option's recorded intent is OBSERVABLE on load rather than a silent
    /// header no-op, and so a future tick-value-persisting rung has the header bit
    /// already reserved and wired end to end.
    pub persist_ticks_flag: bool,
}

/// Resolution of one file-local type to the running build (built once per load).
struct ResolvedType {
    /// The running `ComponentId`, or `None` when the file type is absent in this
    /// build (its columns are skipped → `types_skipped`).
    component_id: Option<ComponentId>,
    /// The file's recorded classification (drives blit-vs-decode-vs-construct).
    serializability: Serializability,
    /// `true` when the running type's `layout_fingerprint` matches the file's (a
    /// POB blit is permitted). Meaningless when `component_id` is `None`.
    fingerprint_ok: bool,
    /// `true` when a `deserialize_fn` is installed for the running type (a ViaFn
    /// decode is permitted, or a POB fingerprint mismatch can demote to decode).
    has_decoder: bool,
    /// `true` when the running type's per-component `format_version` matches the
    /// file's (a POB blit is permitted). Meaningless when `component_id` is `None`
    /// (set `false`). The version is the deliberate user signal that a component's
    /// on-disk bytes changed meaning (plan §3.5; S3 item 1), so it gates the blit
    /// fast path INDEPENDENTLY of `fingerprint_ok`.
    version_ok: bool,
    /// The per-component `format_version` recorded in the file, for a
    /// `VersionMismatch` diagnostic.
    file_format_version: u16,
    /// The running type's per-component `format_version`, for a `VersionMismatch`
    /// diagnostic. `0` when `component_id` is `None`.
    running_format_version: u16,
    /// The file's per-column stride (`size`), used to validate a POB `byte_len`.
    size: usize,
    /// The running type's stable name, for a `FingerprintMismatch` /
    /// `VersionMismatch` diagnostic.
    stable_name: &'static str,
}

/// Loads a world snapshot from `bytes` into `world`, allocating fresh entities and
/// recording the saved→fresh map for the (S2.5) remap pass (plan §3.11 LOAD).
///
/// Always loads into FRESHLY-created archetypes (W4). Returns a [`LoadReport`] on
/// success. On any malformed-stream / version / fingerprint failure returns a
/// [`LoadError`] and leaves `world` consistent — a partially-loaded archetype is
/// rolled back to empty before the error propagates.
///
/// # Contract (W1)
///
/// Every serializable component MUST be registered (touched once — e.g. via
/// `C::component_id()` or one prior spawn) BEFORE `load_world`, since the loader
/// resolves the file's stable names against already-registered ids. A file type
/// with no registered match is counted `types_skipped` (lenient).
///
/// # Version protection (W1 asymmetry)
///
/// Per-component `format_version` protection (S3 item 1) applies to BLITTABLE
/// (`PlainOldBytes`) columns ONLY. A blittable column whose file `format_version`
/// differs from the running type's is a hard [`LoadError::VersionMismatch`] — never
/// a silent blit of stale bytes, even when the `layout_fingerprint` still matches
/// (a same-shape semantic reinterpretation). An OWNING (`SerializeViaFn`) component
/// is RE-DECODED across a `format_version` bump (its `deserialize_fn` rebuilds the
/// value from the wire structure), and the loader CANNOT detect a purely-semantic
/// field reinterpretation that leaves the wire structure unchanged — encode such a
/// change as a wire-structure change (caught by the fingerprint / decoder) or await
/// the future migration hook (plan §6).
///
/// # Errors
///
/// [`LoadError`] on bad magic, an unsupported version, an endianness / ptr-width
/// mismatch, a truncated / corrupt stream, a POB fingerprint mismatch with no
/// decoder, a POB per-component version mismatch ([`LoadError::VersionMismatch`]),
/// or a `deserialize_fn` rejection.
pub fn load_world(
    world: &mut EcsMaster,
    bytes: &[u8],
    policy: LoadEntityPolicy,
) -> Result<LoadReport, LoadError> {
    // v1 ships only `Remap`; the match keeps the signature forward-compatible.
    let LoadEntityPolicy::Remap = policy;

    // ── Step 1: validate the header (RELEASE checks) ───────────────────────────
    let header = read_header(bytes)?;

    // ── Step 2: resolve the type table once ────────────────────────────────────
    let resolved = resolve_type_table(bytes, &header)?;

    // ── Pre-size + the saved→fresh map (populated per archetype below) ─────────
    let mut map = LoadEntityMap::new();
    let mut report = LoadReport {
        // Round-trips the save-time `SaveOptions::persist_ticks` intent (a
        // save/load residual fix) — observable on the report, not a silent no-op.
        persist_ticks_flag: header.flags & PERSIST_TICKS_FLAG != 0,
        ..LoadReport::default()
    };

    // ── Step 4: per archetype (always fresh — start_row == 0, W4) ──────────────
    let archetype_table_off = usize_off(header.archetype_table_off, "archetype_table_off")?;
    let mut block_off = archetype_table_off;
    for _ in 0..header.archetype_count {
        block_off = load_one_archetype(
            world,
            bytes,
            block_off,
            &resolved,
            &mut map,
            &mut report,
        )?;
    }

    // Seal the saved→fresh table before the remap pass; the build is
    // insert-all-then-lookup, so sorting once here lets every `get` below
    // binary-search (and trips the debug guard if a `get` ever precedes this).
    map.finalize();

    // ── Step 4b: restore the dense stores (Dense plan D4 / Decision 7) ─────────
    // AFTER the archetype loop + `map.finalize()` (the dense `s2e` entries remap
    // through the SAME saved→fresh map the archetype loads populated): for each
    // file `DenseStoreBlock` whose type resolves to a registered, still-dense
    // component, blit the compacted column into a fresh `DenseStore` with each saved
    // owning id remapped to its fresh id. A v1 (pre-dense) file carries
    // `dense_store_count == 0`, so this loop runs zero turns (the 0%-gate).
    load_dense_region(world, bytes, &header, &resolved, &map, &mut report)?;

    // ── Step 5: the entity-remap pass (S2.5 / C4) ──────────────────────────────
    // A SEPARATE whole-world pass AFTER every archetype is loaded: rewrite each
    // saved `Entity` reference inside a remappable component (`ChildOf` / an
    // `#[entities]` field) to its freshly-allocated `Entity` via `map`. An unmapped
    // saved id is a loud `LoadError::Decode(UnmappedEntity)`, never a silent
    // dangling reference. A world with no remappable component pays nothing (no
    // pool's `map_entities_fn` is set, so no row is ever visited).
    remap_loaded_entities(world, &map)?;

    Ok(report)
}

/// Loads a world snapshot from `path` into `world` (the file convenience wrapper).
///
/// # Errors
///
/// [`LoadError::Truncated`] wrapping the I/O failure category on a read error, or
/// any [`LoadError`] from [`load_world`].
pub fn load_world_from_file(
    world: &mut EcsMaster,
    path: &Path,
    policy: LoadEntityPolicy,
) -> Result<LoadReport, LoadError> {
    let bytes = std::fs::read(path).map_err(|_| LoadError::Truncated("file read failed"))?;
    load_world(world, &bytes, policy)
}

/// Reads + validates the 80-byte file header (plan §3.11 step 1; the v2 header
/// carries the dense-region descriptor at offsets 64..80, `SaveHeader::SIZE == 80`).
/// Every check is a release-level rejection.
fn read_header(bytes: &[u8]) -> Result<SaveHeader, LoadError> {
    if bytes.len() < SaveHeader::SIZE {
        return Err(LoadError::BadMagic);
    }
    if bytes[0..8] != MAGIC {
        return Err(LoadError::BadMagic);
    }
    let format_version = read_u32(bytes, 8)?;
    if format_version != FORMAT_VERSION {
        return Err(LoadError::UnsupportedVersion(format_version));
    }
    let endianness = bytes[12];
    // v1 produces only little-endian and has no byteswap path: reject any file
    // whose endianness is not this build's native marker (which is LE on x86_64).
    if endianness != native_endianness() || endianness != ENDIAN_LITTLE {
        return Err(LoadError::EndiannessMismatch(endianness));
    }
    let ptr_width = bytes[13];
    if ptr_width != 8 {
        return Err(LoadError::PtrWidthMismatch(ptr_width));
    }

    let header = SaveHeader {
        magic: MAGIC,
        format_version,
        endianness,
        ptr_width,
        flags: read_u16(bytes, 14)?,
        type_table_off: read_u64(bytes, 16)?,
        archetype_table_off: read_u64(bytes, 24)?,
        entity_table_off: read_u64(bytes, 32)?,
        var_data_off: read_u64(bytes, 40)?,
        type_count: read_u32(bytes, 48)?,
        archetype_count: read_u32(bytes, 52)?,
        entity_count: read_u64(bytes, 56)?,
        // Dense plan D4 (v2): the dense-region descriptor at offsets 64..80. A v2
        // file always carries the full 80-byte header (the `SaveHeader::SIZE` check
        // is implicit in `read_u64`/`read_u32`'s bounds checks below).
        dense_table_off: read_u64(bytes, 64)?,
        dense_store_count: read_u32(bytes, 72)?,
        _pad: 0,
    };
    Ok(header)
}

/// Resolves every file-local type to the running build once (plan §3.11 step 2,
/// C1/C2): a dense `Vec<ResolvedType>` indexed by the file-local type index.
fn resolve_type_table(
    bytes: &[u8],
    header: &SaveHeader,
) -> Result<Vec<ResolvedType>, LoadError> {
    let table_off = usize_off(header.type_table_off, "type_table_off")?;
    let count = header.type_count as usize;
    // W2: cap the reservation against the input — a `count` larger than
    // `bytes.len() / TypeTableEntry::SIZE` is a guaranteed truncation (each entry is
    // a fixed `SIZE`-byte record), so a hostile `type_count` cannot force a huge
    // up-front allocation before the per-entry bounds check rejects the stream.
    let mut out = Vec::with_capacity(capacity_hint(count, bytes, TypeTableEntry::SIZE));

    for i in 0..count {
        let entry_off = table_off
            .checked_add(i.checked_mul(TypeTableEntry::SIZE).ok_or(OVF)?)
            .ok_or(OVF)?;
        let entry = read_type_entry(bytes, entry_off)?;

        // Read the stable name from the name pool for the resolve (C1).
        let name_off = entry.name_off as usize;
        let name_len = entry.name_len as usize;
        let name_bytes = slice_at(bytes, name_off, name_len, "type name")?;
        let name = core::str::from_utf8(name_bytes)
            .map_err(|_| LoadError::Truncated("type name is not valid UTF-8"))?;

        let serializability = serializability_from_u8(entry.serializability)
            .ok_or(LoadError::Truncated("invalid serializability discriminant"))?;

        // C1: resolve the running ComponentId by stable name (hash-bucketed,
        // full-name confirmed). Absent → record None (skip its columns; W1 lenient).
        let component_id = component_registry::resolve_stable_name(entry.stable_name_hash, name)
            .map(ComponentId);

        let (fingerprint_ok, has_decoder, version_ok, running_format_version, stable_name) =
            match component_id {
                Some(cid) => {
                    let info = component_registry::get_serialize_info(cid.0);
                    let running_fp = info.map(|i| i.layout_fingerprint).unwrap_or(0);
                    let fingerprint_ok = running_fp == entry.layout_fingerprint;
                    let has_decoder = info.is_some_and(|i| i.deserialize_fn.is_some());
                    // Per-component version gate (S3 item 1). A self-save writes
                    // `entry.format_version == info.format_version` (save.rs:543), so
                    // `version_ok` is ALWAYS true on a self-save — existing round-trip
                    // tests are unaffected. The `unwrap_or(0)` (an absent serialize
                    // info) makes a missing-info running type compare against the file
                    // version, which is only reachable for an unregistered id —
                    // already filtered into the `None` arm below.
                    let running_version = info.map(|i| i.format_version).unwrap_or(0);
                    let version_ok = running_version == entry.format_version;
                    let stable_name = info.map(|i| i.stable_name).unwrap_or("<unknown>");
                    (fingerprint_ok, has_decoder, version_ok, running_version, stable_name)
                }
                None => (false, false, false, 0, "<unresolved>"),
            };

        out.push(ResolvedType {
            component_id,
            serializability,
            fingerprint_ok,
            has_decoder,
            version_ok,
            file_format_version: entry.format_version,
            running_format_version,
            size: entry.size as usize,
            stable_name,
        });
    }
    Ok(out)
}

/// Loads one archetype block at `block_off`, returning the offset just past this
/// block's entity-row table (the next block's header start). Resolves the block's
/// columns into [`LoadColumn`] instructions and calls
/// [`load_archetype`].
fn load_one_archetype(
    world: &mut EcsMaster,
    bytes: &[u8],
    block_off: usize,
    resolved: &[ResolvedType],
    map: &mut LoadEntityMap,
    report: &mut LoadReport,
) -> Result<usize, LoadError> {
    let block = read_archetype_block(bytes, block_off)?;
    let column_count = block.component_count as usize;
    let entity_count = block.entity_count as usize;

    // Parse the per-row saved EntityIds (the entity-row table).
    let rows_off = block.entity_rows_off as usize;
    // W2: cap against the input — the entity-row table is `u64[entity_count]`, so a
    // `count` larger than `bytes.len() / 8` is a guaranteed truncation; the loop
    // below still bounds-checks every row read.
    let mut saved_entity_ids = Vec::with_capacity(capacity_hint(entity_count, bytes, 8));
    for r in 0..entity_count {
        let off = rows_off
            .checked_add(r.checked_mul(8).ok_or(OVF)?)
            .ok_or(OVF)?;
        saved_entity_ids.push(EntityId(read_u64(bytes, off)? as usize));
    }

    // ── Column pass 1: read every column descriptor + resolve its running id ───
    // A no-data column's default-construct decision needs the FULL present-id set
    // of this archetype (a component X is constructible only if some OTHER present
    // component requires it), so descriptors are gathered first, then classified.
    let type_indices_off = block.type_indices_off as usize;
    let column_regions_off = block.column_regions_off as usize;

    // W2: cap against the input — every column mandates a 4-byte file-local type
    // index AND a 16-byte `ColumnRegion`, so it occupies at least 20 bytes on the
    // wire; a `column_count` larger than `bytes.len() / 20` is a guaranteed
    // truncation. The loop below still bounds-checks each index + region read.
    const MIN_COLUMN_BYTES: usize = 4 + ColumnRegion::SIZE;
    let column_cap = capacity_hint(column_count, bytes, MIN_COLUMN_BYTES);
    let mut descs: Vec<ColumnDesc> = Vec::with_capacity(column_cap);
    let mut present_ids: Vec<ComponentId> = Vec::with_capacity(column_cap);
    let mut skipped = 0u32;
    let mut bitset_skipped = 0u32;

    for c in 0..column_count {
        let ti_off = type_indices_off
            .checked_add(c.checked_mul(4).ok_or(OVF)?)
            .ok_or(OVF)?;
        let type_index = read_u32(bytes, ti_off)? as usize;
        if type_index >= resolved.len() {
            return Err(LoadError::Truncated("column type index out of range"));
        }

        let region_off = column_regions_off
            .checked_add(c.checked_mul(16).ok_or(OVF)?)
            .ok_or(OVF)?;
        let data_off = usize_off(read_u64(bytes, region_off)?, "column data_off")?;
        let byte_len = usize_off(read_u64(bytes, region_off + 8)?, "column byte_len")?;

        match resolved[type_index].component_id {
            // W1: a resolved id that is an enable tag (`StorageKind::Bitset`) is
            // filtered OUT of every archetype signature and has NO `ComponentPool`
            // (see `ArchetypeMaster::create_archetype` /
            // `Archetype::filtered_signature_mask`). The saver never emits a bitset
            // column (a self-save's signature excludes it), but a corrupt/foreign
            // file CAN name one — and pushing it into `present_ids` / `descs` would
            // make the writer's `get_pool_mut(cid).expect(...)` panic for an id with
            // no pool. Treat it like an absent type (skip it; the entity stays valid
            // without the tag), mirroring the bitset skip in
            // `clone/materialize.rs`. Counted in `types_bitset_skipped`.
            Some(cid) if component_registry::storage_kind(cid.0) == StorageKind::Bitset => {
                bitset_skipped += 1;
            }
            Some(cid) => {
                present_ids.push(cid);
                descs.push(ColumnDesc {
                    type_index,
                    cid,
                    data_off,
                    byte_len,
                });
            }
            None => {
                // The file type is absent in this build (W1 lenient): skip it.
                skipped += 1;
            }
        }
    }

    // ── Pool-capacity gate (C2): authoritative on the WRITER side ──────────────
    // The writer's `Archetype::reserve_capacity(n)` is the SINGLE authoritative pool
    // row-ceiling gate. A forged `entity_count` (== `n`) for a ZST/tiny-stride column
    // — or two file blocks that dedup-collapse onto ONE running archetype, summing to
    // `e1 + e2 > ceiling` on an ADDITIVE pool `len` — is rejected by the writer as a
    // LOUD `LoadWriteError::CapacityExceeded` (mapped to `LoadError::CapacityExceeded`
    // via `?` below), never an `.expect()` panic. The earlier per-block load-side
    // pre-check could NOT see the additive pool `len`, so it could not shadow the
    // block-collapse case; the writer-side gate does. (`load_archetype` /
    // `reserve_capacity` are reachable ONLY from `load_world`, a cold path — the
    // C1 0%-gate is preserved.)

    // ── Column pass 2: classify each present column into a LoadColumn ───────────
    let mut ids: Vec<ComponentId> = Vec::with_capacity(descs.len());
    let mut columns: Vec<LoadColumn<'_>> = Vec::with_capacity(descs.len());
    let mut blitted = 0u32;
    let mut decoded = 0u32;
    let mut defaulted = 0u32;

    for d in &descs {
        let rt = &resolved[d.type_index];
        let col = classify_column(bytes, rt, d, &present_ids, entity_count)?;
        match col {
            ColumnPlan::Blit(lc) => {
                blitted += 1;
                ids.push(d.cid);
                columns.push(lc);
            }
            ColumnPlan::Decode(lc) => {
                decoded += 1;
                ids.push(d.cid);
                columns.push(lc);
            }
            ColumnPlan::Construct(lc) => {
                defaulted += 1;
                ids.push(d.cid);
                columns.push(lc);
            }
            ColumnPlan::ExcludeDefaulted => {
                // No data + no ctor: omit from the loaded archetype (the entity
                // stays valid; the component is simply absent). Counted defaulted.
                defaulted += 1;
            }
        }
    }

    // Canonical-sort the id set so the fresh archetype dedups to the same id
    // regardless of column order, keeping `columns` aligned to `ids`. `LoadColumn`
    // carries its own id, so a stable sort of pairs is simplest.
    let mut paired: Vec<(ComponentId, LoadColumn<'_>)> =
        ids.into_iter().zip(columns).collect();
    paired.sort_by_key(|(id, _)| id.0);
    let ids: Vec<ComponentId> = paired.iter().map(|(id, _)| *id).collect();
    let columns: Vec<LoadColumn<'_>> = paired.into_iter().map(|(_, c)| c).collect();

    // Write the archetype (create → reserve → write → commit → register). On a
    // decode error the writer rolls the fresh archetype back to empty.
    let loaded = load_archetype(world, &ids, &columns, &saved_entity_ids, map)?;

    // Commit the report only after a successful write.
    report.entities_loaded += loaded as u64;
    report.archetypes_loaded += 1;
    report.columns_blitted += blitted;
    report.columns_decoded += decoded;
    report.types_skipped += skipped;
    report.types_bitset_skipped += bitset_skipped;
    report.types_defaulted += defaulted;

    // Advance to the next block: past this block's entity-row table.
    let next_off = rows_off
        .checked_add(entity_count.checked_mul(8).ok_or(OVF)?)
        .ok_or(OVF)?;
    Ok(next_off)
}

/// Parses + restores the dense-store region (Dense plan D4 / Decision 7).
///
/// Strides the `DenseStoreBlock[dense_store_count]` headers at `dense_table_off`;
/// for each one whose type resolves to a registered component that is STILL dense
/// in this build, reads the `s2e` table + the compacted column bytes, validates the
/// byte length, and hands them to the crate-private
/// [`load_dense_store`] writer (which remaps each saved owning id and rebuilds the
/// store). A block whose type is absent / no longer dense is a clean skip, and a
/// fingerprint-or-version mismatch on a POB blit is a hard error, matching the table
/// column policy. An OWNING (`SerializeViaFn`) dense block WITH an installed decoder
/// is DECODED per-member (the v1.1 dense ViaFn path — one cursor over the whole
/// variable-length run, mirroring the table `Decode` column), counted in
/// [`LoadReport::dense_stores_loaded`] / [`LoadReport::dense_members_loaded`]. A
/// `SerializeViaFn` block with NO decoder (the S1 memberships-only boundary) or an
/// `Ignore` block carries no decodable data and stays an OBSERVABLE skip, recorded
/// in [`LoadReport::dense_stores_skipped`] / [`LoadReport::dense_members_skipped`]
/// (plus `boyko-W0901`, in every profile), NOT silently dropped. Runs zero turns for a
/// `dense_store_count == 0` file (the 0%-gate).
///
/// # Duplicate `type_index` hardening
///
/// The saver emits AT MOST ONE block per live dense type, but a hostile/corrupt
/// file can repeat a `type_index` across two blocks. `DenseStore::insert`'s
/// target-is-empty precondition (the fresh-world-load contract enforced in
/// `load_writer.rs`) is a `debug_assert!` only, so a release build would run a
/// second insert pass into the SAME store with no defense — a stale slot from the
/// first pass stays "live" while `e2s` is overwritten to the second pass's slot for
/// any shared saved entity, an aliased/corrupted dense iteration, never a panic.
/// This function rejects a repeated `type_index` itself, RELEASE-level, before
/// [`load_dense_store`] / [`load_dense_store_via_fn`] is ever reached.
fn load_dense_region(
    world: &mut EcsMaster,
    bytes: &[u8],
    header: &SaveHeader,
    resolved: &[ResolvedType],
    map: &LoadEntityMap,
    report: &mut LoadReport,
) -> Result<(), LoadError> {
    if header.dense_store_count == 0 {
        return Ok(());
    }
    let table_off = usize_off(header.dense_table_off, "dense_table_off")?;
    let count = header.dense_store_count as usize;
    // Tracks every `type_index` seen so far (one bool per file-local type). Sized by
    // `resolved.len()`, itself capped by `resolve_type_table`'s own W2 guard against
    // the real type-table bytes — NOT by this untrusted `dense_store_count` — so this
    // allocation cannot be driven to an unbounded size by a hostile header.
    let mut seen_type_index = vec![false; resolved.len()];
    for i in 0..count {
        let block_off = table_off
            .checked_add(i.checked_mul(DenseStoreBlock::SIZE).ok_or(OVF)?)
            .ok_or(OVF)?;
        let block = read_dense_store_block(bytes, block_off)?;

        let type_index = block.type_index as usize;
        if type_index >= resolved.len() {
            return Err(LoadError::Truncated("dense store type index out of range"));
        }
        if seen_type_index[type_index] {
            return Err(LoadError::Truncated("duplicate dense store type index"));
        }
        seen_type_index[type_index] = true;

        let rt = &resolved[type_index];
        let member_count = block.member_count as usize;

        // Resolve the running id. Absent → skip the whole block (lenient, W1).
        let Some(cid) = rt.component_id else {
            continue;
        };
        // A file dense store whose running type is NOT (or no longer) dense is
        // skipped — the membership cannot be reconstructed into a non-dense store
        // (mirrors the table-side bitset skip). The owning entity stays valid.
        if component_registry::storage_kind(cid.0) != StorageKind::Dense {
            continue;
        }

        // Read the `s2e` table (`u64[member_count]`, slot order).
        let s2e_off = usize_off(block.s2e_off, "dense s2e_off")?;
        let mut saved_s2e: Vec<u64> = Vec::with_capacity(capacity_hint(member_count, bytes, 8));
        for r in 0..member_count {
            let off = s2e_off
                .checked_add(r.checked_mul(8).ok_or(OVF)?)
                .ok_or(OVF)?;
            saved_s2e.push(read_u64(bytes, off)?);
        }

        // Read the compacted column bytes. v1 dense is POB: validate the byte
        // length against the running stride + fingerprint/version before the blit.
        let data_off = usize_off(block.data_off, "dense data_off")?;
        let data_byte_len = usize_off(block.data_byte_len, "dense data_byte_len")?;
        match rt.serializability {
            Serializability::PlainOldBytes => {
                if !(rt.fingerprint_ok && rt.version_ok) {
                    // A dense POB store whose shape/version changed: HARD error
                    // (never a silent garbage blit), version-first like the table
                    // column path.
                    if !rt.version_ok {
                        return Err(LoadError::VersionMismatch {
                            name: rt.stable_name,
                            file: rt.file_format_version,
                            running: rt.running_format_version,
                        });
                    }
                    return Err(LoadError::FingerprintMismatch(rt.stable_name));
                }
                let expected = member_count.checked_mul(rt.size).ok_or(OVF)?;
                if data_byte_len != expected {
                    return Err(LoadError::Truncated(
                        "dense store data_byte_len != member_count * stride",
                    ));
                }
                let member_bytes = slice_at(bytes, data_off, data_byte_len, "dense store data")?;
                let loaded = load_dense_store(world, cid, member_bytes, &saved_s2e, map)?;
                report.dense_stores_loaded += 1;
                report.dense_members_loaded += loaded as u64;
            }
            Serializability::SerializeViaFn => {
                // v1.1 OWNING dense decode: the saver ran `serialize_fn` per live
                // member into ONE shared cursor (save.rs ViaFn dense path), so the
                // block's `data` is a VARIABLE-LENGTH concatenated run — its length is
                // `data_byte_len` (NOT `member_count * stride`; each element is
                // self-describing). The writer reads `member_count` elements from one
                // cursor over the whole run, mirroring the table `Decode` column. A
                // ViaFn dense column with NO installed decoder (the S1 boundary — the
                // saver emitted a zero-length data run, memberships only) cannot be
                // reconstructed: skip it OBSERVABLY, exactly as before.
                let Some(deserialize_fn) = component_registry::get_serialize_info(cid.0)
                    .and_then(|info| info.deserialize_fn)
                else {
                    // No decoder installed — the S1 memberships-only boundary. Skip
                    // observably (the owning entity stays valid without the value).
                    report.dense_stores_skipped += 1;
                    report.dense_members_skipped += member_count as u64;
                    warn_dense_viafn_skipped(rt.stable_name, member_count);
                    continue;
                };
                // The encoded run occupies exactly `data_byte_len` bytes at
                // `data_off` (variable-length; validated in-bounds by `slice_at`).
                let member_data =
                    slice_at(bytes, data_off, data_byte_len, "dense store ViaFn data")?;
                let loaded =
                    load_dense_store_via_fn(world, cid, deserialize_fn, member_data, &saved_s2e, map)?;
                report.dense_stores_loaded += 1;
                report.dense_members_loaded += loaded as u64;
            }
            Serializability::Ignore => {
                // An `Ignore` dense block carries memberships but no decodable data
                // (its contract — not serializable). It stays an OBSERVABLE skip: the
                // owning entity stays valid without the dense membership, and the
                // counters (+ `boyko-W0901`) make the drop visible instead of a
                // silent data loss. Mirrors the table-side `types_skipped` /
                // `types_bitset_skipped` counters.
                report.dense_stores_skipped += 1;
                report.dense_members_skipped += member_count as u64;
                warn_dense_viafn_skipped(rt.stable_name, member_count);
                continue;
            }
        }
    }
    Ok(())
}

/// Reports `boyko-W0901` for a genuinely-undecodable dense block (an `Ignore` type, or a
/// `SerializeViaFn` type with no installed `deserialize_fn`): names the component + dropped
/// member count so a dense store's memberships do not vanish without a trace. An OWNING dense
/// block WITH a decoder is decoded, not warned about.
///
/// # Two things about this function changed at rung L8a, and both were arguments the tree settled
///
/// It **was `#[cfg(debug_assertions)]`**, with a release no-op beside it and a doc line saying
/// "the `LoadReport` counters carry the signal there". They do not carry it far: `LoadReport` is a
/// return value, and a host that drops it — or logs only its totals — turns a save that silently
/// lost a component's data into a save that loaded fine. The gate is dropped because the condition
/// is already fully computed in release: `report.dense_stores_skipped += 1` sits on the line above
/// each call site, in every profile. Un-gating costs the release build one `#[cold]` call on a
/// path that was already writing to memory.
///
/// It also **no longer says "no log-crate dependency"**. That comment cited "the project's
/// existing diagnostic pattern (`boyko_rhi`)" — a pattern this campaign is retiring — and the
/// dependency it refused was a third-party facade, which `boyko_log` is not.
///
/// `RatePolicy::Every`: the subject is a component type, and a save carrying three undecodable
/// dense stores has three different things to say.
#[cold]
#[inline(never)]
fn warn_dense_viafn_skipped(name: &str, member_count: usize) {
    boyko_log::warn!(
        boyko_log::Serialize,
        W0901,
        "dense store for component `{}` carries no decodable data (Ignore, or SerializeViaFn \
         with no installed deserialize_fn); skipped {} member(s) (recorded in \
         LoadReport::dense_stores_skipped)",
        name,
        member_count
    );
}

/// Reads one [`DenseStoreBlock`] header from its 40-byte image at `off`.
fn read_dense_store_block(bytes: &[u8], off: usize) -> Result<DenseStoreBlock, LoadError> {
    let _ = slice_at(bytes, off, DenseStoreBlock::SIZE, "dense store block")?;
    Ok(DenseStoreBlock {
        type_index: read_u32(bytes, off)?,
        member_count: read_u32(bytes, off + 4)?,
        serializability: bytes[off + 8],
        _pad: [0; 7],
        s2e_off: read_u64(bytes, off + 16)?,
        data_off: read_u64(bytes, off + 24)?,
        data_byte_len: read_u64(bytes, off + 32)?,
    })
}

/// A column's raw descriptor gathered in pass 1 (resolved id + data region).
struct ColumnDesc {
    /// Index into the resolved type table.
    type_index: usize,
    /// The running `ComponentId` (always `Some` — skipped columns are excluded in
    /// pass 1).
    cid: ComponentId,
    /// Byte offset of the column's data region.
    data_off: usize,
    /// Byte length of the column's data region.
    byte_len: usize,
}

/// The per-column classification result.
enum ColumnPlan<'a> {
    Blit(LoadColumn<'a>),
    Decode(LoadColumn<'a>),
    Construct(LoadColumn<'a>),
    /// No data + no ctor: exclude from the loaded archetype.
    ExcludeDefaulted,
}

/// Classifies one resolved column into a [`LoadColumn`] write instruction (plan
/// §3.11 step 4 + C2). `present_ids` is the full present-id set of this archetype,
/// used to resolve a no-data column's default-construct ctor; `n` is the archetype's
/// entity count, used to validate a POB column's `byte_len`.
fn classify_column<'a>(
    bytes: &'a [u8],
    rt: &ResolvedType,
    d: &ColumnDesc,
    present_ids: &[ComponentId],
    n: usize,
) -> Result<ColumnPlan<'a>, LoadError> {
    match rt.serializability {
        Serializability::PlainOldBytes => {
            // The blit fast path requires BOTH a matching layout fingerprint AND a
            // matching per-component `format_version` (S3 item 1): the version is the
            // deliberate user signal that the bytes changed meaning, so a same-shape
            // semantic reinterpretation (fingerprint still matches) is still rejected.
            if rt.fingerprint_ok && rt.version_ok {
                // POB blit fast path. RELEASE-validate `byte_len == n * stride` (C3:
                // a corrupt shorter length would leave the pool tail uninit; a longer
                // one would overrun the reserved region — both UB), then slice it.
                let expected = n.checked_mul(rt.size).ok_or(OVF)?;
                if d.byte_len != expected {
                    return Err(LoadError::Truncated(
                        "POB column byte_len != entity_count * stride",
                    ));
                }
                let col_bytes = slice_at(bytes, d.data_off, d.byte_len, "POB column")?;
                Ok(ColumnPlan::Blit(LoadColumn::Blit {
                    component_id: d.cid,
                    bytes: col_bytes,
                }))
            } else if rt.has_decoder {
                // NOTE (W2): this demote branch is DEAD for POB today —
                // `install_serialize_fn` installs `(None, None)` for a `PlainOldBytes`
                // type (component_registry.rs ~1866), so `rt.has_decoder` is always
                // false on this arm. It is kept SYMMETRIC with the fingerprint path
                // (and so a future POB decoder would activate here). If a POB decoder
                // is ever installed, this branch decodes UNTRUSTED bytes — it MUST get
                // a dedicated fuzz case before that activation ships.
                debug_assert!(
                    rt.serializability != Serializability::PlainOldBytes,
                    "W2: a POB column reached the has_decoder demote branch — \
                     install_serialize_fn installs no POB decoder, so this is dead \
                     today; a future activation must add a fuzz case first"
                );
                decode_column(bytes, d)
            } else if !rt.version_ok {
                // W4 VERSION-FIRST precedence: the version is the deliberate user
                // signal, so a simultaneous version + fingerprint mismatch reports
                // VersionMismatch. HARD error — never a silent stale-bytes blit.
                Err(LoadError::VersionMismatch {
                    name: rt.stable_name,
                    file: rt.file_format_version,
                    running: rt.running_format_version,
                })
            } else {
                // C2 HARD ERROR: a blittable column whose shape changed (version still
                // matches), with no decode fallback — never a silent garbage blit.
                Err(LoadError::FingerprintMismatch(rt.stable_name))
            }
        }
        Serializability::SerializeViaFn => {
            // TODO(S-future): explicit ViaFn version-migration policy hook — see
            // SERIALIZATION-PLAN.md §6. A ViaFn column is re-decoded across a
            // `format_version` bump (W1 asymmetry): the loader cannot detect a
            // purely-semantic field reinterpretation that leaves the wire structure
            // unchanged. Until the migration hook lands, encode such a change as a
            // wire-structure change (caught by the fingerprint / decoder).
            if d.byte_len == 0 || !rt.has_decoder {
                // The S1.5 non-Wire demotion case (column carries no data) OR no
                // decoder installed: default-construct via a require ctor if one
                // exists, else exclude from the archetype.
                construct_or_exclude(d.cid, present_ids)
            } else {
                decode_column(bytes, d)
            }
        }
        Serializability::Ignore => {
            // The saver never emits an `Ignore` column with data; if the running
            // archetype would include it, default-construct or exclude.
            construct_or_exclude(d.cid, present_ids)
        }
    }
}

/// Builds a `Decode` column instruction (`rt.has_decoder` already proved a decoder
/// exists).
fn decode_column<'a>(bytes: &'a [u8], d: &ColumnDesc) -> Result<ColumnPlan<'a>, LoadError> {
    let info = component_registry::get_serialize_info(d.cid.0)
        .ok_or(LoadError::Truncated("decode column lost its serialize info"))?;
    let deserialize_fn = info
        .deserialize_fn
        .ok_or(LoadError::Truncated("decode column has no deserialize_fn"))?;
    let col_bytes = slice_at(bytes, d.data_off, d.byte_len, "decode column")?;
    Ok(ColumnPlan::Decode(LoadColumn::Decode {
        component_id: d.cid,
        deserialize_fn,
        bytes: col_bytes,
    }))
}

/// Decides between default-constructing a no-data column (some OTHER present
/// component `#[require]`s `cid`, so a ctor exists in the require-closure) and
/// excluding it from the loaded archetype.
///
/// A ctor is registered in the registry ONLY as a `#[require(X)]` edge from another
/// component — there is no standalone "default ctor for X". So a no-data column is
/// constructible iff `cid` is reachable as a required component from some present
/// id; otherwise it is EXCLUDED from the fresh archetype (the entity stays valid,
/// the component is simply absent). This is the documented no-default-path branch
/// (plan §3.11 step 4 — "If no default path exists, document + skip into a
/// types_defaulted count").
fn construct_or_exclude<'a>(
    cid: ComponentId,
    present_ids: &[ComponentId],
) -> Result<ColumnPlan<'a>, LoadError> {
    match required_ctor_in_set(present_ids, cid) {
        Some(ctor) => Ok(ColumnPlan::Construct(LoadColumn::Construct {
            component_id: cid,
            ctor,
        })),
        None => Ok(ColumnPlan::ExcludeDefaulted),
    }
}

// ── Byte-slice readers (every read bounds-checked; LE per the format) ──────────

/// A reusable "size overflow" load error.
const OVF: LoadError = LoadError::Truncated("offset/length arithmetic overflow");

/// Caps a file-supplied element `count` against the bytes that could possibly back
/// it, returning a safe `Vec::with_capacity` hint (W2 — allocation-DoS guard).
///
/// A `count` is untrusted: a hostile header/block can name `0xFFFF_FFFF` elements
/// to force a multi-GiB up-front reservation BEFORE the per-element bounds checks
/// in the parse loop would catch the truncation. Each element occupies at least
/// `min_elem_size` bytes on the wire (1 for a `u64` row-id table, the record size
/// for a fixed-width entry table), so a `count` larger than
/// `bytes.len() / min_elem_size` is a guaranteed truncation and cannot be satisfied.
///
/// The returned hint is `count.min(bytes.len() / min_elem_size)`: for a valid file
/// it equals `count` (the full capacity hint is kept), and for a hostile count it
/// is clamped to at most the input length — the subsequent parse loop still
/// bounds-checks every element and surfaces the real truncation as a [`LoadError`].
/// `min_elem_size` must be `>= 1` (a zero would divide-by-zero); every caller passes
/// a fixed positive record stride.
#[inline]
fn capacity_hint(count: usize, bytes: &[u8], min_elem_size: usize) -> usize {
    debug_assert!(min_elem_size >= 1, "capacity_hint: min_elem_size must be >= 1");
    count.min(bytes.len() / min_elem_size)
}

/// Converts a file `u64` offset to a `usize`, rejecting an overflow on a narrower
/// target (already excluded by the ptr-width check, but kept robust). `what` names
/// the field for the diagnostic.
#[inline]
fn usize_off(value: u64, what: &'static str) -> Result<usize, LoadError> {
    usize::try_from(value).map_err(|_| LoadError::Truncated(what))
}

/// Returns `bytes[off..off+len]`, or [`LoadError::Truncated`] when the range
/// exceeds the slice (a hostile offset / length).
#[inline]
fn slice_at<'a>(
    bytes: &'a [u8],
    off: usize,
    len: usize,
    what: &'static str,
) -> Result<&'a [u8], LoadError> {
    let end = off.checked_add(len).ok_or(OVF)?;
    bytes
        .get(off..end)
        .ok_or(LoadError::Truncated(what))
}

#[inline]
fn read_u16(bytes: &[u8], off: usize) -> Result<u16, LoadError> {
    let s = slice_at(bytes, off, 2, "u16")?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

#[inline]
fn read_u32(bytes: &[u8], off: usize) -> Result<u32, LoadError> {
    let s = slice_at(bytes, off, 4, "u32")?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

#[inline]
fn read_u64(bytes: &[u8], off: usize) -> Result<u64, LoadError> {
    let s = slice_at(bytes, off, 8, "u64")?;
    Ok(u64::from_le_bytes([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
    ]))
}

/// Reads one [`TypeTableEntry`] from its 40-byte image at `off` (field-by-field LE,
/// matching the saver's `#[repr(C)]` byte layout).
fn read_type_entry(bytes: &[u8], off: usize) -> Result<TypeTableEntry, LoadError> {
    // Bounds-check the whole record once.
    let _ = slice_at(bytes, off, TypeTableEntry::SIZE, "type table entry")?;
    Ok(TypeTableEntry {
        stable_name_hash: read_u64(bytes, off)?,
        layout_fingerprint: read_u64(bytes, off + 8)?,
        size: read_u32(bytes, off + 16)?,
        align: read_u32(bytes, off + 20)?,
        name_off: read_u32(bytes, off + 24)?,
        name_len: read_u32(bytes, off + 28)?,
        format_version: read_u16(bytes, off + 32)?,
        serializability: bytes[off + 34],
        _pad: [0; 5],
    })
}

/// Reads one [`ArchetypeBlock`] header from its 24-byte image at `off`.
fn read_archetype_block(bytes: &[u8], off: usize) -> Result<ArchetypeBlock, LoadError> {
    let _ = slice_at(bytes, off, ArchetypeBlock::SIZE, "archetype block")?;
    Ok(ArchetypeBlock {
        component_count: read_u32(bytes, off)?,
        entity_count: read_u32(bytes, off + 4)?,
        type_indices_off: read_u32(bytes, off + 8)?,
        column_regions_off: read_u32(bytes, off + 12)?,
        entity_rows_off: read_u32(bytes, off + 16)?,
        _pad: 0,
    })
}

/// Maps the on-disk `serializability` byte back to the enum (the C3 validate-on-read
/// obligation: an out-of-range discriminant is a corrupt file, not a transmute).
#[inline]
fn serializability_from_u8(value: u8) -> Option<Serializability> {
    match value {
        0 => Some(Serializability::PlainOldBytes),
        1 => Some(Serializability::SerializeViaFn),
        2 => Some(Serializability::Ignore),
        _ => None,
    }
}

#[cfg(test)]
mod l8a_w0901 {
    use super::*;
    use boyko_log::probe::{arm, watch, watched};

    #[test]
    fn w0901_reports_each_skipped_component_because_the_subject_is_a_type() {
        // `Every`, and this is the assertion that makes the choice mean something: a save
        // carrying three undecodable dense stores has three different things to say, and a
        // `Once` here would name one component and drop the other two names on the floor.
        //
        // It also pins the un-gating. This reporter was `#[cfg(debug_assertions)]` with a release
        // no-op beside it, on the argument that `LoadReport`'s counters carried the signal --
        // they carry a NUMBER, and a host that logs only totals cannot tell which component's
        // data vanished. The test runs in every profile now because the function does.
        arm::<boyko_log::Serialize>();

        watch(b'W', W0901.number());
        warn_dense_viafn_skipped("Velocity", 12);
        warn_dense_viafn_skipped("Health", 3);
        warn_dense_viafn_skipped("Inventory", 0);
        assert_eq!(watched(), 3, "one record per skipped component type");
    }
}
