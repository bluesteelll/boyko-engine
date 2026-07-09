//! Phase S2 hardening — the LOAD path against HOSTILE / foreign input (S2 review
//! W1 / W2 / O1).
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §3.11 (LOAD) + §5 (C3). The shipped
//! `load_roundtrip` suite covers a TRUNCATED / corrupt self-save; this suite covers
//! the three robustness items the S2 code review flagged for a file that a DIFFERENT
//! build (or an adversary) authored:
//!
//! * **W1** — a column whose stable name resolves to a registered enable tag
//!   (`StorageKind::Bitset`). A bitset id has NO `ComponentPool`, so feeding it to
//!   the writer would panic; the loader must skip it (a clean
//!   `types_bitset_skipped`), never panic.
//! * **W2** — a header/block with a hostile element `count` (`type_count` /
//!   `entity_count` / `component_count`). The loader must reject the truncation as a
//!   `LoadError::Truncated` WITHOUT first reserving a multi-GiB `Vec`.
//! * **O1** — a `deserialize_fn` that PANICS partway through a multi-row decode
//!   column. The `ArchetypeLoadGuard` rollback must leave the destination world
//!   consistent (no half-loaded archetype, `entity_count == 0`), drop every
//!   successfully-decoded row exactly once, and never construct the failed row.
//!
//! These exercise the load `unsafe` (the writer's reserved-uninit decode + the
//! panic-path `Drop` rollback), so the suite is also run under Miri-TB.

use std::cell::Cell;
use std::sync::atomic::{AtomicI64, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry::{
    self, Serializability, StorageKind,
};
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::serialize::{DecodeError, LoadCursor, SaveCursor, Wire};
use boyko_macros::Component;

use boyko_serialize::{LoadEntityPolicy, LoadError, SaveOptions, load_world, save_world};

// ── Shared POB component ───────────────────────────────────────────────────────

/// A plain POB component (all-float, `#[repr(C)]`) used as a committed column in
/// the W1 and O1 scenarios.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Position {
    x: f32,
    y: f32,
    z: f32,
}

/// Saves `world` to a fresh byte buffer.
fn save(world: &EcsMaster) -> Vec<u8> {
    let mut out = Vec::new();
    save_world(world, &SaveOptions::default(), &mut out).expect("save");
    out
}

// ════════════════════════════════════════════════════════════════════════════
// W1 — a file column resolving to a registered enable tag is SKIPPED, not a panic
// ════════════════════════════════════════════════════════════════════════════
//
// The saver never emits a bitset column (`create_archetype` filters bitset ids out
// of the archetype signature, so they are absent from `component_ids()`), but a
// corrupt / foreign file CAN name one. For the loader's `resolve_stable_name` to
// resolve such a name to a bitset id, that id must have BOTH serialize metadata
// (the name index keys on it) AND `StorageKind::Bitset`. The `#[derive(Component)]`
// path XORs the two (a `storage = "bitset"` derive suppresses the serialize
// install), so this scenario is reachable only via a hand-written `Component` impl
// that opts into both — exactly the cross-build case where the OTHER build saved the
// type as a normal table component and THIS build reclassified it as an enable tag.

/// A POB component for the W1 scenario with a FIXED stable name (same byte length
/// as the enable tag's), so the in-place name-pool rewrite below shifts no offsets.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
#[component(stable_name = "w1.pos")]
struct W1Pos {
    a: f32,
}

/// A hand-written enable-tag component: it installs serialize metadata + a stable
/// name (so a foreign file's column name resolves to it) AND classifies itself
/// `StorageKind::Bitset` (so it has no `ComponentPool`). The combination is the W1
/// cross-build hazard; a normal `#[derive]` cannot produce it. Its stable name is
/// the SAME byte length as `W1Pos`'s so the in-place rewrite is offset-stable.
#[derive(Clone, Copy)]
struct W1EnableTag;

impl Component for W1EnableTag {
    const STORAGE_IS_BITSET: bool = true;

    fn component_id() -> boyko_ecs::ecs::identifiers::primitives::ComponentId {
        use std::sync::OnceLock;
        static ID: OnceLock<boyko_ecs::ecs::identifiers::primitives::ComponentId> =
            OnceLock::new();
        *ID.get_or_init(|| {
            let raw = component_registry::register_new::<Self>();
            // Install serialize metadata + the C1 stable-name index so a file column
            // carrying this stable name resolves to this id (the W1 hazard premise),
            // and classify the id as a bitset enable tag (no `ComponentPool`).
            component_registry::install_serialize_fn::<Self>(raw);
            component_registry::register_stable_name::<Self>(raw);
            component_registry::install_storage_kind::<Self>(raw);
            boyko_ecs::ecs::identifiers::primitives::ComponentId(raw)
        })
    }

    // A fixed stable name, same byte length as `W1Pos`'s `"w1.pos"`.
    fn stable_name() -> &'static str {
        "w1.tag"
    }

    // Classify as a serializable POB type so `install_serialize_fn` records non-Ignore
    // metadata (the name index resolves it). The bitset filter — not this — is what
    // keeps it out of every pool.
    fn serializability_runtime() -> Serializability {
        Serializability::PlainOldBytes
    }
}

#[test]
fn enable_tag_column_in_foreign_file_is_skipped_not_panic() {
    // Touch the enable tag so it is registered (serialize metadata + bitset kind).
    let tag_id = W1EnableTag::component_id();
    assert_eq!(
        component_registry::storage_kind(tag_id.0),
        StorageKind::Bitset,
        "the W1 test component must be classified as a bitset enable tag"
    );
    let tag_name = component_registry::get_serialize_info(tag_id.0)
        .expect("the W1 enable tag must have installed serialize metadata")
        .stable_name;

    // Save a normal one-column POB world.
    let mut src = EcsMaster::new();
    let arch = src.get_or_create_archetype(&[W1Pos::component_id()]);
    src.spawn_one(arch, W1Pos { a: 1.0 }).expect("spawn");
    src.spawn_one(arch, W1Pos { a: 4.0 }).expect("spawn");
    let mut bytes = save(&src);

    // Rewrite the single `W1Pos` type entry's stable name (+ hash) to the enable
    // tag's name so its column resolves, on load, to the bitset id. The replacement
    // name is the SAME byte length so no name-pool offsets shift.
    let type_table_off = u64::from_le_bytes(bytes[16..24].try_into().unwrap()) as usize;
    let type_count = u32::from_le_bytes(bytes[48..52].try_into().unwrap()) as usize;
    assert_eq!(type_count, 1, "the saved world has exactly one POB type");

    let entry_off = type_table_off; // the single entry
    let name_off =
        u32::from_le_bytes(bytes[entry_off + 24..entry_off + 28].try_into().unwrap()) as usize;
    let name_len =
        u32::from_le_bytes(bytes[entry_off + 28..entry_off + 32].try_into().unwrap()) as usize;

    assert_eq!(
        name_len,
        tag_name.len(),
        "this in-place rewrite needs the POB type name and the enable-tag name to be \
         the same byte length (\"w1.pos\" vs \"w1.tag\", both 6 bytes)"
    );

    let tag_hash = component_registry::fnv1a_64(tag_name.as_bytes());
    bytes[entry_off..entry_off + 8].copy_from_slice(&tag_hash.to_le_bytes());
    bytes[name_off..name_off + name_len].copy_from_slice(tag_name.as_bytes());

    // Also overwrite the file entry's `layout_fingerprint` (offset +8) to match the
    // enable tag's installed fingerprint (the hand-written impl defaults
    // `LAYOUT_FINGERPRINT == 0`). Without this the column would demote to a
    // `FingerprintMismatch` in classify BEFORE reaching the writer; matching it makes
    // the column classify as a POB Blit so the resolved bitset id reaches
    // `load_archetype` — which, without the W1 guard, panics on the pool-less id. The
    // tag's stride (`size`, offset +16) stays the file's `W1Pos` size (4 bytes), and
    // the column `byte_len == n * 4` already validates, so classify yields a Blit.
    let tag_fp = component_registry::get_serialize_info(tag_id.0)
        .expect("the W1 enable tag has serialize info")
        .layout_fingerprint;
    bytes[entry_off + 8..entry_off + 16].copy_from_slice(&tag_fp.to_le_bytes());

    // Load: the resolved bitset column is SKIPPED (no pool → would have panicked in
    // the writer without the W1 guard), counted in `types_bitset_skipped`. The owning
    // entities still materialize — into a component-less archetype (the tag is simply
    // absent, mirroring the absent-type W1-lenient behavior and the clone path that
    // also drops the enable bit). The crux is: NO panic, the bitset id never reaches
    // the writer's pool-less `expect`.
    let mut dst = EcsMaster::new();
    let report = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap)
        .expect("a foreign bitset column must be a clean skip, never a panic/error");

    assert_eq!(
        report.types_bitset_skipped, 1,
        "the resolved enable-tag column must be skipped as a bitset type"
    );
    assert_eq!(report.columns_blitted, 0, "no POB column survives (the only column was the tag)");
    assert_eq!(
        report.entities_loaded, 2,
        "the entities still materialize (into a component-less archetype) — the tag is \
         absent, not a load failure"
    );
    assert_eq!(
        dst.entity_count(),
        2,
        "the destination world is consistent: both entities loaded, none half-built"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// W2 — a hostile element count is rejected as Truncated, not a giant allocation
// ════════════════════════════════════════════════════════════════════════════
//
// Each test forges a count field to `0xFFFF_FFFF` (or a smaller-but-still-impossible
// value) in a real save and asserts the loader returns `LoadError::Truncated` — the
// `capacity_hint` cap means no multi-GiB `Vec` is ever reserved (the per-element
// bounds check that follows is what surfaces the truncation). A successful return of
// `Truncated` proves both: the loader did not abort on a huge allocation, and the
// forged count was caught.

#[test]
fn hostile_type_count_is_truncated_not_a_giant_alloc() {
    let mut src = EcsMaster::new();
    let arch = src.get_or_create_archetype(&[Position::component_id()]);
    src.spawn_one(arch, Position { x: 1.0, y: 2.0, z: 3.0 }).expect("spawn");
    let mut bytes = save(&src);

    // `type_count` is header byte offset 48 (u32). Forge it to u32::MAX — a count
    // that, un-capped, would reserve `0xFFFF_FFFF * size_of::<ResolvedType>()` bytes.
    bytes[48..52].copy_from_slice(&u32::MAX.to_le_bytes());

    let mut dst = EcsMaster::new();
    let err = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).unwrap_err();
    assert!(
        matches!(err, LoadError::Truncated(_)),
        "a hostile type_count must be a Truncated rejection, got {err:?}"
    );
    assert_eq!(dst.entity_count(), 0, "a rejected load leaves the world consistent");
}

#[test]
fn hostile_archetype_entity_count_is_truncated_not_a_giant_alloc() {
    let mut src = EcsMaster::new();
    let arch = src.get_or_create_archetype(&[Position::component_id()]);
    src.spawn_one(arch, Position { x: 7.0, y: 8.0, z: 9.0 }).expect("spawn");
    let bytes_template = save(&src);

    // The archetype block's `entity_count` is at `archetype_table_off + 4` (u32).
    let archetype_table_off =
        u64::from_le_bytes(bytes_template[24..32].try_into().unwrap()) as usize;
    let mut bytes = bytes_template;
    let ec_off = archetype_table_off + 4;
    bytes[ec_off..ec_off + 4].copy_from_slice(&u32::MAX.to_le_bytes());

    let mut dst = EcsMaster::new();
    let err = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).unwrap_err();
    assert!(
        matches!(err, LoadError::Truncated(_)),
        "a hostile entity_count must be a Truncated rejection, got {err:?}"
    );
    assert_eq!(dst.entity_count(), 0, "a rejected load leaves the world consistent");
}

#[test]
fn hostile_component_count_is_truncated_not_a_giant_alloc() {
    let mut src = EcsMaster::new();
    let arch = src.get_or_create_archetype(&[Position::component_id()]);
    src.spawn_one(arch, Position { x: 1.0, y: 1.0, z: 1.0 }).expect("spawn");
    let bytes_template = save(&src);

    // The archetype block's `component_count` is the first u32 at
    // `archetype_table_off`.
    let archetype_table_off =
        u64::from_le_bytes(bytes_template[24..32].try_into().unwrap()) as usize;
    let mut bytes = bytes_template;
    bytes[archetype_table_off..archetype_table_off + 4]
        .copy_from_slice(&u32::MAX.to_le_bytes());

    let mut dst = EcsMaster::new();
    let err = load_world(&mut dst, &bytes, LoadEntityPolicy::Remap).unwrap_err();
    assert!(
        matches!(err, LoadError::Truncated(_)),
        "a hostile component_count must be a Truncated rejection, got {err:?}"
    );
    assert_eq!(dst.entity_count(), 0, "a rejected load leaves the world consistent");
}

// ════════════════════════════════════════════════════════════════════════════
// O1 — a deserialize_fn that PANICS mid-column rolls the archetype back cleanly
// ════════════════════════════════════════════════════════════════════════════
//
// `PanicPayload` is a `Wire` leaf field that panics in `wire_read` on a configured
// row index (the 2nd row of a >= 3-row column) and Drop-counts every CONSTRUCTED
// instance. The owning component `OPanicOwning { p: PanicPayload }` is therefore
// classified `SerializeViaFn` (its decode loops the rows through `wire_read`). A
// panic mid-decode unwinds through `load_archetype`'s decode loop (no `&mut
// Archetype` live at the `deserialize_fn` call) and the `ArchetypeLoadGuard::Drop`
// rolls back: it drops the rows committed before the panic exactly once and leaves
// the fresh archetype empty.

thread_local! {
    /// The 0-based row index at which `PanicPayload::wire_read` panics (set per test).
    static PANIC_AT_ROW: Cell<i64> = const { Cell::new(-1) };
    /// How many `wire_read` calls have happened this load (reset per test).
    static READS_SEEN: Cell<i64> = const { Cell::new(0) };
}

/// Net count of live `PanicPayload` instances (constructed minus dropped). After a
/// clean rollback it must return to its pre-load value (every decoded row dropped
/// exactly once; the panicking row never constructed).
static LIVE_PAYLOADS: AtomicI64 = AtomicI64::new(0);

/// A `Wire` leaf that Drop-counts every constructed instance and panics in
/// `wire_read` at a configured row. Drop-counting proves the rollback dropped the
/// decoded rows exactly once and never constructed the failed row.
#[derive(Clone, PartialEq, Debug)]
struct PanicPayload {
    val: u32,
}

impl PanicPayload {
    fn new(val: u32) -> Self {
        LIVE_PAYLOADS.fetch_add(1, Ordering::SeqCst);
        Self { val }
    }
}

impl Drop for PanicPayload {
    fn drop(&mut self) {
        LIVE_PAYLOADS.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Wire for PanicPayload {
    fn wire_write(&self, c: &mut SaveCursor<'_>) {
        c.write_u32(self.val);
    }

    fn wire_read(c: &mut LoadCursor<'_>) -> Result<Self, DecodeError> {
        // Consume the bytes FIRST (so the cursor stays well-formed for the rows the
        // rollback never reaches), then decide whether to panic on this row.
        let val = c.read_u32()?;
        let this_row = READS_SEEN.with(|r| {
            let n = r.get();
            r.set(n + 1);
            n
        });
        let panic_row = PANIC_AT_ROW.with(|p| p.get());
        if panic_row >= 0 && this_row == panic_row {
            panic!("PanicPayload::wire_read injected panic at row {this_row}");
        }
        // Count the constructed instance only on the success path (a panic above
        // never reaches here, so the failed row is never counted/constructed).
        Ok(PanicPayload::new(val))
    }
}

/// An owning component whose single `Wire` field can panic on decode → classified
/// `SerializeViaFn`, so its load runs the row-by-row decode path the rollback guard
/// protects.
#[derive(Component, Clone, PartialEq, Debug)]
struct OPanicOwning {
    p: PanicPayload,
}

#[test]
fn deserialize_panic_mid_column_rolls_back_cleanly() {
    // Register the POB column id BEFORE the owning id so the canonical (id-sorted)
    // column order commits the POB column FIRST — exercising the "prior committed
    // POB column" the rollback must also unwind (its per-row drop_fn is a no-op for
    // a `Copy` POB type, but the guard still walks it).
    let pos_id = Position::component_id();
    let owning_id = OPanicOwning::component_id();

    // Build a 3-row world: a committed POB column (Position) + the owning column.
    let mut src = EcsMaster::new();
    let arch = src.get_or_create_archetype(&[pos_id, owning_id]);
    for i in 0..3u32 {
        src.spawn_two(
            arch,
            Position { x: i as f32, y: 0.0, z: 0.0 },
            OPanicOwning { p: PanicPayload { val: 100 + i } },
        )
        .expect("spawn");
    }
    let bytes = save(&src);

    // Snapshot the live-payload count AFTER the save (the save read the live world's
    // payloads by reference — it never constructs a `PanicPayload`). The 3 live ones
    // belong to `src` and are dropped when `src` is dropped, NOT by the load.
    let live_before = LIVE_PAYLOADS.load(Ordering::SeqCst);

    // Arm the panic at row 1 (the 2nd decoded row of the >= 3-row column) and reset
    // the per-load read counter.
    PANIC_AT_ROW.with(|p| p.set(1));
    READS_SEEN.with(|r| r.set(0));

    let mut dst = EcsMaster::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        load_world(&mut dst, &bytes, LoadEntityPolicy::Remap)
    }));
    // Disarm so a `src` Drop (or any later decode) does not re-trigger the panic.
    PANIC_AT_ROW.with(|p| p.set(-1));

    assert!(result.is_err(), "the injected deserialize panic must unwind out of load_world");

    // The destination world is consistent: no entity was registered (the batch is
    // registered only after every column commits), so the rolled-back archetype
    // holds no rows.
    assert_eq!(
        dst.entity_count(),
        0,
        "a mid-decode panic must leave the destination world with zero entities"
    );

    // Row 0 of the owning column WAS decoded (one constructed `PanicPayload`) and the
    // rollback's `drop_at` dropped it exactly once; row 1 panicked inside `wire_read`
    // BEFORE constructing its `PanicPayload`, so it was never created or dropped. The
    // net live count is therefore back to `live_before` (the 3 still-live `src`
    // payloads), proving exactly-once drop of the decoded row and no leak.
    let live_after = LIVE_PAYLOADS.load(Ordering::SeqCst);
    assert_eq!(
        live_after, live_before,
        "the decoded row must be dropped exactly once and the failed row never \
         constructed (no leak, no double-drop) — net live payloads must be unchanged"
    );

    // Exactly two `wire_read` calls happened: row 0 (decoded) + row 1 (panicked); the
    // loop never reached row 2.
    let reads = READS_SEEN.with(|r| r.get());
    assert_eq!(reads, 2, "decode must stop at the panicking row (rows 0 and 1 only)");

    // Dropping `src` now drops its 3 live payloads; keep it alive until here so the
    // `live_before` snapshot is meaningful.
    drop(src);
}
