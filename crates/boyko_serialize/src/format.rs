//! On-disk binary file format (Phase S1).
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §3.9 (file format). Every type here is
//! `#[repr(C)]` with a pinned, const-asserted layout: these structs are written
//! to disk verbatim (`copy_nonoverlapping` of their bytes) and read back by the
//! S2 loader, so the field order / size / offsets ARE the wire contract. All
//! multi-byte integers are little-endian on disk (the header records the
//! endianness; v1 rejects a mismatch on load — plan O2). The structs store native
//! integers; the saver writes their native-endian bytes and the header's
//! `endianness` byte lets the loader reject a foreign-endian file rather than
//! byteswap (no byteswap path in v1).
//!
//! # Why fixed-width, naturally-aligned fields
//!
//! Natural alignment (u64 at 8-aligned offsets, u32 at 4-aligned) means the
//! compiler inserts no implicit padding, so the `#[repr(C)]` byte image is fully
//! specified — the const-asserts below pin every size/offset so a field reorder
//! or width change is a compile error, not a silent format break.

/// The 8-byte file magic: `b"BOYKOSAV"`. The first bytes of every snapshot; the
/// loader rejects a file that does not start with it.
pub const MAGIC: [u8; 8] = *b"BOYKOSAV";

/// The current file format version written into [`SaveHeader::format_version`].
/// Bumped on any incompatible change to the on-disk layout. (Distinct from a
/// per-component `format_version`, which versions a single component's bytes.)
///
/// `2` (Dense plan D4): the header grew the `dense_table_off` / `dense_store_count`
/// fields and the trailing dense-store region (the compacted live dense columns +
/// their `s2e` slot→EntityId tables). A v1 file is rejected on load
/// (`UnsupportedVersion`); a v2 file with no dense store (`dense_store_count == 0`)
/// is byte-equivalent to the v1 body plus the 16-byte header growth (the 0%-gate
/// for a table-only world: no dense region bytes are emitted).
pub const FORMAT_VERSION: u32 = 2;

/// Endianness marker for [`SaveHeader::endianness`]: little-endian (the common
/// target; the only value v1 produces, plan §2.2 / O2).
pub const ENDIAN_LITTLE: u8 = 0;

/// Endianness marker for [`SaveHeader::endianness`]: big-endian. Never produced by
/// v1 (no byteswap path); reserved so the loader can name a foreign file.
pub const ENDIAN_BIG: u8 = 1;

/// The pointer width (in bytes) this build targets, written into
/// [`SaveHeader::ptr_width`]. v1 supports only 64-bit (plan §3.11 / O2).
pub const PTR_WIDTH: u8 = (usize::BITS / 8) as u8;

/// Returns [`ENDIAN_LITTLE`] / [`ENDIAN_BIG`] for the build's native endianness.
#[inline]
pub const fn native_endianness() -> u8 {
    if cfg!(target_endian = "little") {
        ENDIAN_LITTLE
    } else {
        ENDIAN_BIG
    }
}

/// The required alignment of every blittable (POB) column region within the file
/// (plan §3.9): `max(SIMD_BUFFER_ALIGN, align)` so an mmap-cast loader (S4) can
/// take aligned SIMD loads from a column start. S1 lays POB regions at offsets
/// rounded up to at least this; per-column the saver uses
/// `max(COLUMN_REGION_ALIGN, component_align)`.
pub const COLUMN_REGION_ALIGN: usize = 32;

/// [`SaveHeader::flags`] bit 0 — set by the saver when `SaveOptions::persist_ticks`
/// was requested, and round-tripped by the loader into `LoadReport::persist_ticks_flag`
/// (a save/load residual fix: the option used to be a silent no-op, recorded
/// nowhere on disk). This flag records the save-time INTENT only — S1 still always
/// resets every row's ticks to the load-time `current_tick` regardless of this bit;
/// per-row tick-VALUE persistence is deferred to a later rung (plan §6). Bits
/// 1..16 stay reserved (always 0).
pub const PERSIST_TICKS_FLAG: u16 = 1 << 0;

/// Fixed 64-byte file header (plan §3.9). Written first; the `*_off` fields are
/// backpatched after the body is laid out (two-pass save, §3.11 W3).
///
/// Layout (byte offsets, 64-bit target):
/// `magic@0 format_version@8 endianness@12 ptr_width@13 flags@14
///  type_table_off@16 archetype_table_off@24 entity_table_off@32 var_data_off@40
///  type_count@48 archetype_count@52 entity_count@56 dense_table_off@64
///  dense_store_count@72 _pad@76` — total 80 B (v2: the original 64-byte v1 image
/// extended by the 16-byte dense-region descriptor; every v1 field keeps its
/// offset so a v1-aware reader of the leading fields is unaffected).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveHeader {
    /// `b"BOYKOSAV"` — the file magic.
    pub magic: [u8; 8],
    /// The on-disk format version ([`FORMAT_VERSION`]).
    pub format_version: u32,
    /// Endianness marker ([`ENDIAN_LITTLE`] / [`ENDIAN_BIG`]).
    pub endianness: u8,
    /// Pointer width in bytes ([`PTR_WIDTH`]). v1 load rejects `!= 8`.
    pub ptr_width: u8,
    /// Flags bitset ([`PERSIST_TICKS_FLAG`] is bit 0; all other bits reserved and
    /// always 0). Set by the saver from `SaveOptions`, and read back by the loader
    /// into `LoadReport::persist_ticks_flag` — see [`PERSIST_TICKS_FLAG`] for the
    /// intent-vs-values boundary this bit does NOT yet cross.
    pub flags: u16,
    /// Byte offset (from file start) of the [`TypeTableEntry`] array.
    pub type_table_off: u64,
    /// Byte offset of the archetype-block region (a sequence of
    /// [`ArchetypeBlock`] headers each followed by its body).
    pub archetype_table_off: u64,
    /// Byte offset of the saved entity table (S2 populates the per-entity rows;
    /// S1 reserves/positions the region).
    pub entity_table_off: u64,
    /// Byte offset of the trailing `var_data` region (owning heap bytes referenced
    /// by position-independent [`VarRef`] offsets, plan §3.9).
    pub var_data_off: u64,
    /// Number of [`TypeTableEntry`] records (distinct serialized components).
    pub type_count: u32,
    /// Number of [`ArchetypeBlock`] records.
    pub archetype_count: u32,
    /// Total saved entity count (sum of every archetype's entity count).
    pub entity_count: u64,
    /// Dense plan D4 — byte offset (from file start) of the dense-store region (a
    /// sequence of [`DenseStoreBlock`] headers each followed by its body: the
    /// `s2e` table then the compacted live column bytes). Valid only when
    /// `dense_store_count != 0`; otherwise points at the file end (no bytes).
    pub dense_table_off: u64,
    /// Dense plan D4 — number of [`DenseStoreBlock`] records (live dense stores
    /// with ≥ 1 member). `0` for a table-only world (the 0%-gate: no dense bytes).
    pub dense_store_count: u32,
    /// Padding to keep the 80-byte image padding-free (the `u64`s force
    /// 8-alignment). Always zeroed.
    pub _pad: u32,
}

// Pin the wire size + every field offset (the format IS these bytes). Gated to
// 64-bit (the engine's supported target — see CLAUDE.md): the layout is
// width-independent here (no usize/ptr fields), but the const-asserts document the
// canonical 64-bit image. v2 grew the image to 80 B; every v1 field offset is
// unchanged (the dense descriptor is purely appended).
const _: () = assert!(std::mem::size_of::<SaveHeader>() == 80);
const _: () = assert!(std::mem::align_of::<SaveHeader>() == 8);
const _: () = assert!(std::mem::offset_of!(SaveHeader, magic) == 0);
const _: () = assert!(std::mem::offset_of!(SaveHeader, format_version) == 8);
const _: () = assert!(std::mem::offset_of!(SaveHeader, endianness) == 12);
const _: () = assert!(std::mem::offset_of!(SaveHeader, ptr_width) == 13);
const _: () = assert!(std::mem::offset_of!(SaveHeader, flags) == 14);
const _: () = assert!(std::mem::offset_of!(SaveHeader, type_table_off) == 16);
const _: () = assert!(std::mem::offset_of!(SaveHeader, archetype_table_off) == 24);
const _: () = assert!(std::mem::offset_of!(SaveHeader, entity_table_off) == 32);
const _: () = assert!(std::mem::offset_of!(SaveHeader, var_data_off) == 40);
const _: () = assert!(std::mem::offset_of!(SaveHeader, type_count) == 48);
const _: () = assert!(std::mem::offset_of!(SaveHeader, archetype_count) == 52);
const _: () = assert!(std::mem::offset_of!(SaveHeader, entity_count) == 56);
const _: () = assert!(std::mem::offset_of!(SaveHeader, dense_table_off) == 64);
const _: () = assert!(std::mem::offset_of!(SaveHeader, dense_store_count) == 72);
const _: () = assert!(std::mem::offset_of!(SaveHeader, _pad) == 76);

impl SaveHeader {
    /// The fixed header size in bytes (the body starts here).
    pub const SIZE: usize = 80;

    /// Builds a header with the magic / version / endianness / ptr-width filled in
    /// and the `*_off` / `*_count` fields zeroed (backpatched by the saver after
    /// the body layout is known).
    #[inline]
    pub fn new() -> Self {
        Self {
            magic: MAGIC,
            format_version: FORMAT_VERSION,
            endianness: native_endianness(),
            ptr_width: PTR_WIDTH,
            flags: 0,
            type_table_off: 0,
            archetype_table_off: 0,
            entity_table_off: 0,
            var_data_off: 0,
            type_count: 0,
            archetype_count: 0,
            entity_count: 0,
            dense_table_off: 0,
            dense_store_count: 0,
            _pad: 0,
        }
    }

    /// Reinterprets the header as its `#[repr(C)]` byte image (for writing).
    ///
    /// # Safety
    ///
    /// `SaveHeader` is `#[repr(C)]` with no padding (the const-asserts above pin
    /// every offset) and contains only integer / byte-array fields, all of whose
    /// bit patterns are valid — so every byte of `self` is initialized and reading
    /// it as `[u8; SIZE]` is sound.
    #[inline]
    pub fn as_bytes(&self) -> &[u8; Self::SIZE] {
        // SAFETY: `Self::SIZE == size_of::<SaveHeader>()` (the const-assert above),
        // `self` is a fully-initialized `#[repr(C)]` value with no padding and only
        // all-bits-valid integer/array fields, and the resulting reference borrows
        // `self` so it cannot outlive the header. The cast preserves provenance.
        unsafe { &*(self as *const SaveHeader as *const [u8; Self::SIZE]) }
    }
}

impl Default for SaveHeader {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// One per distinct serialized component (plan §3.9). Records the stable type key,
/// the layout guard, and the storage layout the loader needs to resolve the file
/// type to a running `ComponentId` and to size a blit.
///
/// Layout (byte offsets): `stable_name_hash@0 layout_fingerprint@8 size@16
/// align@20 name_off@24 name_len@28 format_version@32 serializability@34
/// _pad@35..40` — total 40 B. The 5-byte `_pad` makes the named fields sum to the
/// full 40 B (the `u64` fields force 8-alignment), so the struct is **padding-free**
/// and its byte image has no uninitialized bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeTableEntry {
    /// FNV-1a 64 hash of the component's stable name (the resolution bucket key,
    /// plan C1).
    pub stable_name_hash: u64,
    /// The derive-computed blit-validity guard (plan C2): the loader compares it
    /// against the running type's fingerprint before a POB blit.
    pub layout_fingerprint: u64,
    /// `size_of::<C>()` — the column stride (bytes per row). 0 for a ZST tag.
    pub size: u32,
    /// `align_of::<C>()`.
    pub align: u32,
    /// Byte offset (from file start) of this type's stable-name string in the name
    /// pool.
    pub name_off: u32,
    /// Byte length of the stable-name string.
    pub name_len: u32,
    /// The per-component on-disk format version (plan §3.5).
    pub format_version: u16,
    /// The [`Serializability`](boyko_ecs::ecs::core::component::component_registry::Serializability)
    /// discriminant as a `u8` (`PlainOldBytes=0 / SerializeViaFn=1 / Ignore=2`).
    pub serializability: u8,
    /// Explicit trailing padding to 40 B (the `u64` fields force 8-alignment).
    /// Sized so the named fields sum to `size_of` exactly — the struct is then
    /// **padding-free**, so the byte image written to the file has NO
    /// uninitialized bytes (the C1-class uninit-read hazard). Always zeroed.
    pub _pad: [u8; 5],
}

const _: () = assert!(std::mem::size_of::<TypeTableEntry>() == 40);
const _: () = assert!(std::mem::align_of::<TypeTableEntry>() == 8);
const _: () = assert!(std::mem::offset_of!(TypeTableEntry, stable_name_hash) == 0);
const _: () = assert!(std::mem::offset_of!(TypeTableEntry, layout_fingerprint) == 8);
const _: () = assert!(std::mem::offset_of!(TypeTableEntry, size) == 16);
const _: () = assert!(std::mem::offset_of!(TypeTableEntry, align) == 20);
const _: () = assert!(std::mem::offset_of!(TypeTableEntry, name_off) == 24);
const _: () = assert!(std::mem::offset_of!(TypeTableEntry, name_len) == 28);
const _: () = assert!(std::mem::offset_of!(TypeTableEntry, format_version) == 32);
const _: () = assert!(std::mem::offset_of!(TypeTableEntry, serializability) == 34);
const _: () = assert!(std::mem::offset_of!(TypeTableEntry, _pad) == 35);

impl TypeTableEntry {
    /// The wire size in bytes of one type-table entry.
    pub const SIZE: usize = 40;
}

/// The fixed header of one archetype block (plan §3.9). Followed in the file by
/// the block body: the file-local type-index array, the per-column
/// [`ColumnRegion`] array, then the entity rows (S2 populates the rows; S1 lays
/// out the columns).
///
/// Layout (byte offsets): `component_count@0 entity_count@4 type_indices_off@8
/// column_regions_off@12 entity_rows_off@16 _pad@20` — total 24 B.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchetypeBlock {
    /// Number of serialized columns in this archetype (== the file-local
    /// type-index count == the [`ColumnRegion`] count).
    pub component_count: u32,
    /// Number of entities (rows) in this archetype.
    pub entity_count: u32,
    /// Byte offset (from file start) of this block's file-local type-index array
    /// (`u32[component_count]`, each indexing the [`TypeTableEntry`] array).
    pub type_indices_off: u32,
    /// Byte offset of this block's [`ColumnRegion`] array
    /// (`ColumnRegion[component_count]`).
    pub column_regions_off: u32,
    /// Byte offset of this block's entity-row table (`u64[entity_count]` saved
    /// `EntityId`s, plan §3.9; S2 reads them for the remap map).
    pub entity_rows_off: u32,
    /// Padding to keep the record 4-aligned and a round 24 bytes. Written as 0.
    pub _pad: u32,
}

const _: () = assert!(std::mem::size_of::<ArchetypeBlock>() == 24);
const _: () = assert!(std::mem::align_of::<ArchetypeBlock>() == 4);
const _: () = assert!(std::mem::offset_of!(ArchetypeBlock, component_count) == 0);
const _: () = assert!(std::mem::offset_of!(ArchetypeBlock, entity_count) == 4);
const _: () = assert!(std::mem::offset_of!(ArchetypeBlock, type_indices_off) == 8);
const _: () = assert!(std::mem::offset_of!(ArchetypeBlock, column_regions_off) == 12);
const _: () = assert!(std::mem::offset_of!(ArchetypeBlock, entity_rows_off) == 16);
const _: () = assert!(std::mem::offset_of!(ArchetypeBlock, _pad) == 20);

impl ArchetypeBlock {
    /// The wire size in bytes of one archetype-block header.
    pub const SIZE: usize = 24;
}

/// Where one column's bytes live in the file (plan §3.9). For a POB column this is
/// the blitted data region (`byte_len == count * stride`); for a `SerializeViaFn`
/// column it is the per-element encoded run.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColumnRegion {
    /// Byte offset (from file start) of this column's data.
    pub data_off: u64,
    /// Byte length of this column's data.
    pub byte_len: u64,
}

const _: () = assert!(std::mem::size_of::<ColumnRegion>() == 16);
const _: () = assert!(std::mem::align_of::<ColumnRegion>() == 8);
const _: () = assert!(std::mem::offset_of!(ColumnRegion, data_off) == 0);
const _: () = assert!(std::mem::offset_of!(ColumnRegion, byte_len) == 8);

impl ColumnRegion {
    /// The wire size in bytes of one column region.
    pub const SIZE: usize = 16;
}

/// A position-independent reference into the `var_data` region (plan §3.9, the
/// rkyv relative-pointer technique). `offset` is a SELF-relative `i64` (an `i64`,
/// not `i32`, to avoid the 2 GiB overflow); `len` is the byte length. Emitted by
/// the owning `serialize_fn` encode path (S2 derive emission) — S1 ships the type
/// so the format module is complete.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VarRef {
    /// Self-relative byte offset (signed; computed against the field's own write
    /// head) to the start of the referenced bytes in `var_data`.
    pub offset: i64,
    /// Byte length of the referenced run.
    pub len: u64,
}

const _: () = assert!(std::mem::size_of::<VarRef>() == 16);
const _: () = assert!(std::mem::align_of::<VarRef>() == 8);
const _: () = assert!(std::mem::offset_of!(VarRef, offset) == 0);
const _: () = assert!(std::mem::offset_of!(VarRef, len) == 8);

impl VarRef {
    /// The wire size in bytes of one var-ref.
    pub const SIZE: usize = 16;
}

/// The fixed header of one dense-store block (Dense plan D4 / Decision 7). One per
/// live dense `ComponentId` (≥ 1 member). Followed in the file by the block body:
/// the `s2e` saved-`EntityId` table (`u64[member_count]`, slot order) then the
/// compacted live column bytes (`member_count * stride`, a POB blit or a per-element
/// `serialize_fn` run).
///
/// Unlike a TABLE column (whose row→entity association IS the archetype's entity
/// list), a dense store's slot→entity map is the `s2e` array — so it is serialized
/// here alongside the data. On load each saved `s2e` entry is remapped to its fresh
/// id via the [`LoadEntityMap`](boyko_ecs::ecs::core::serialize::LoadEntityMap)
/// (the same machinery `ChildOf` / `#[entities]` use), then the column is rebuilt.
///
/// Layout (byte offsets): `type_index@0 member_count@4 serializability@8
/// _pad@9..16 s2e_off@16 data_off@24 data_byte_len@32` — total 40 B (the `u64`
/// offsets force 8-alignment; the explicit `_pad` makes the image padding-free).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DenseStoreBlock {
    /// Index into the file's [`TypeTableEntry`] array (the dense component's type).
    pub type_index: u32,
    /// Number of live members (slots) in this store at save time (== the s2e table
    /// length == the live column row count after the read-only logical compaction).
    pub member_count: u32,
    /// The [`Serializability`](boyko_ecs::ecs::core::component::component_registry::Serializability)
    /// discriminant as a `u8` (drives the blit-vs-fn-ptr branch — mirrors
    /// [`TypeTableEntry::serializability`]).
    pub serializability: u8,
    /// Explicit padding to the next `u64` (s2e_off). Always zeroed → padding-free
    /// image.
    pub _pad: [u8; 7],
    /// Byte offset (from file start) of this store's `s2e` table
    /// (`u64[member_count]`, the saved owning `EntityId.0` per live slot in slot
    /// order).
    pub s2e_off: u64,
    /// Byte offset of this store's compacted live column bytes.
    pub data_off: u64,
    /// Byte length of the column bytes (`member_count * stride` for a POB store; the
    /// `serialize_fn` run length for a ViaFn store; 0 for a ZST dense type).
    pub data_byte_len: u64,
}

const _: () = assert!(std::mem::size_of::<DenseStoreBlock>() == 40);
const _: () = assert!(std::mem::align_of::<DenseStoreBlock>() == 8);
const _: () = assert!(std::mem::offset_of!(DenseStoreBlock, type_index) == 0);
const _: () = assert!(std::mem::offset_of!(DenseStoreBlock, member_count) == 4);
const _: () = assert!(std::mem::offset_of!(DenseStoreBlock, serializability) == 8);
const _: () = assert!(std::mem::offset_of!(DenseStoreBlock, _pad) == 9);
const _: () = assert!(std::mem::offset_of!(DenseStoreBlock, s2e_off) == 16);
const _: () = assert!(std::mem::offset_of!(DenseStoreBlock, data_off) == 24);
const _: () = assert!(std::mem::offset_of!(DenseStoreBlock, data_byte_len) == 32);

impl DenseStoreBlock {
    /// The wire size in bytes of one dense-store-block header.
    pub const SIZE: usize = 40;
}
