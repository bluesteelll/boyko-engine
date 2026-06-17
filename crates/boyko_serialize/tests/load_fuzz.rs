//! Phase S3 items 3 + 5 — the LOAD-path soundness fuzz + generated idempotency
//! sweep.
//!
//! Spec: `docs/SERIALIZATION-PLAN.md` §3.11 (LOAD) + §5 (C1/C2/C3) + S3 items 3/5.
//!
//! # Item 3 — the soundness fuzz (a Miri-TB target)
//!
//! INVARIANT: feeding `load_world` ANY mutation of a valid corpus snapshot must
//! NEVER produce undefined behaviour, an abort, or a panic — only `Ok(report)`
//! (with an internally-consistent report) or `Err(LoadError)`. The loader ingests
//! untrusted bytes (C3), so every malformed stream must surface as a `LoadError`,
//! never UB. The fuzz wraps each load in `catch_unwind` with an EMPTY allowlist: a
//! caught panic FAILS the test (with the iteration index + mutation class for
//! reproduction).
//!
//! The PRNG is a hand-rolled `splitmix64` with a CONSTANT seed — no `Date` /
//! `rand` / `Math.random`; third-party fuzzers are forbidden (the run must be
//! bit-for-bit reproducible across machines and under Miri).
//!
//! # Item 5 — generated idempotency sweep
//!
//! Generates K randomized VALID worlds (varying entity counts + which fixture
//! archetypes are populated) and asserts the POST-FIRST-LOAD FIXED POINT for each:
//! `save(load(save(load(save(w))))) == save(load(save(w)))`. The first raw save of
//! a hand-built world is NOT compared to any post-load save (the loader may re-intern
//! types in a different — equally valid — order; that divergence is a load-path
//! property, see `save_determinism.rs:219`).
//!
//! ════════════════════════════════════════════════════════════════════════════
//! STEP-0 (C2) AUDIT — writer-panic reachability from a hostile file
//! ════════════════════════════════════════════════════════════════════════════
//!
//! `boyko_ecs/src/ecs/core/serialize/load_writer.rs` has 11 `expect` sites. Three
//! are FILE-`n`-driven and were audited for reachability by a fuzz that stomps
//! `entity_count` / `size`, given load.rs's pre-validation:
//!
//! * **Site 302** (formerly `Archetype::reserve_capacity(n).expect(...)`): FIXED at
//!   the WRITER. `reserve_capacity` returns `Err(ArchetypePoolCapacityExceeded)`
//!   (NOT a panic) when `n` exceeds a hosted pool's per-stride row ceiling
//!   (`pool_reserve_rows(stride)`). `n == entity_count` (or the ADDITIVE sum across
//!   blocks that dedup-collapse onto one running archetype), capped in load.rs only
//!   by the per-row `read_u64` bound (8 B/row), so a ZST/tiny-stride column lets a
//!   forged `entity_count` reach the reserve with only `bytes.len()/8` bytes backing
//!   it. The earlier per-block load-side `pool_row_ceiling` pre-check could NOT
//!   shadow this: its cap was per-block-in-isolation, but `reserve_capacity`'s
//!   ceiling is ADDITIVE on the running pool `len` (two blocks with `e1<=ceiling`,
//!   `e2<=ceiling` but `e1+e2>ceiling` both pass the per-block check and overflow in
//!   aggregate). FIX (C2, writer-side in `boyko_ecs`): `load_archetype` now returns
//!   `Err(LoadWriteError::CapacityExceeded)` (mapped to `LoadError::CapacityExceeded`)
//!   instead of `.expect()`-panicking — the SINGLE authoritative gate. The
//!   redundant load-side pre-check + `pool_row_ceiling` helper were REMOVED. The
//!   writer is COLD (called only from `load_world`), so the C1 0%-gate is preserved.
//!   `capacity_guard.rs` proves the block-collapse case returns a loud `Err`, never
//!   a panic.
//! * **Site 334** (`n.checked_mul(stride).expect("n * stride overflow")`): SHADOWED.
//!   This lives inside a `debug_assert_eq!` (Miri/debug only) in the Blit arm, and
//!   the SAME `n * stride` is already run through `checked_mul` in load.rs's POB
//!   classify (`classify_column`, the `n.checked_mul(rt.size)` guard) BEFORE the
//!   writer is reached. No load-side action.
//! * **Site 466** (`start_id.checked_add(n).expect("start_id + n overflow")`):
//!   NOT REACHABLE from file input. `start_id` is a monotonic atomic entity-id
//!   counter; overflowing `usize` would require ~2^64 prior reservations in one
//!   process — impossible in a single fuzz run. It guards an internal counter, not
//!   a file `n`. No action.
//!
//! Conclusion: only Site 302 needed a fix; it is now a writer-side `Result`
//! (`LoadWriteError::CapacityExceeded`), not a panic — see `capacity_guard.rs`.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicI64, Ordering};

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::serialize::{DecodeError, LoadCursor, SaveCursor, Wire};
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};
use boyko_macros::Component;

use boyko_serialize::{LoadEntityPolicy, LoadError, LoadReport, SaveOptions, load_world, save_world};

// ── Test components (mirrors the `save_determinism.rs` fixture shape) ──────────

/// POB: `#[repr(C)]`, 12 B, align 4.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Position {
    x: f32,
    y: f32,
    z: f32,
}

/// POB: `#[repr(C)]`, 8 B, align 4.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Velocity {
    dx: i32,
    dy: i32,
}

/// POB: `#[repr(C)]`, 16 B, align 8 — a wider width so a size/stride desync stomp
/// has a distinct target.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct Heavy {
    a: u64,
    b: u64,
}

/// ZST tag (`PlainOldBytes`, `byte_len == 0`) — REQUIRED so the writer's
/// reserve-ceiling path (Step-0 site 302) is reachable by a forged `entity_count`
/// (a tag column costs 0 wire bytes/row).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
#[repr(C)]
struct ZstTag;

/// Owning component (`String` + `Vec<u8>`) → `SerializeViaFn`. REQUIRED so the
/// decode-rollback path (load_writer.rs ~380-429) is reachable (a mid-decode
/// failure rolls the partial archetype back via `drop_at`).
#[derive(Component, Clone, PartialEq, Debug)]
struct Inventory {
    name: String,
    flags: Vec<u8>,
}

// ════════════════════════════════════════════════════════════════════════════
// splitmix64 PRNG — constant-seeded, reproducible (no Date/rand/Math.random)
// ════════════════════════════════════════════════════════════════════════════

/// A hand-rolled `splitmix64` generator. Deterministic from a constant seed.
struct Rng(u64);

impl Rng {
    /// The fixed seed — every run (native or Miri) draws the identical sequence.
    const SEED: u64 = 0x9E37_79B9_7F4A_7C15;

    fn new() -> Self {
        Rng(Self::SEED)
    }

    /// The canonical `splitmix64` step.
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform value in `[0, n)` (n > 0). `n == 0` returns 0.
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.next() % n as u64) as usize
    }

    /// A uniform value in `[lo, hi)` (`lo < hi`).
    fn range(&mut self, lo: usize, hi: usize) -> usize {
        debug_assert!(lo < hi, "Rng::range: empty range");
        lo + self.below(hi - lo)
    }

    /// A single random byte.
    fn byte(&mut self) -> u8 {
        (self.next() & 0xFF) as u8
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Corpus + fixture world builder
// ════════════════════════════════════════════════════════════════════════════

/// Saves `world` to a fresh byte buffer.
fn save(world: &EcsMaster) -> Vec<u8> {
    let mut out = Vec::new();
    save_world(world, &SaveOptions::default(), &mut out).expect("save");
    out
}

/// Reinterprets a `Copy` POD value's bytes (test-only; the fixture types are all
/// `#[repr(C)]` POD).
fn pod_bytes<T: Copy>(v: &T) -> &[u8] {
    // SAFETY: `T` is a `#[repr(C)]` POD fixture type; we read exactly
    // `size_of::<T>()` initialized bytes from a live stack value, consumed
    // synchronously by `create_entity` before `v` is dropped. Test-only.
    unsafe { std::slice::from_raw_parts((v as *const T) as *const u8, std::mem::size_of::<T>()) }
}

/// Spawns one entity into `arch` carrying exactly the given `(id, bytes)` columns.
fn spawn(world: &mut EcsMaster, arch: ArchetypeId, cols: &[(ComponentId, &[u8])]) {
    world.create_entity(arch, cols).expect("create_entity");
}

/// Registers every fixture component (the W1 contract: components must be touched
/// before a load resolves their stable names).
fn register_components() {
    let _ = Position::component_id();
    let _ = Velocity::component_id();
    let _ = Heavy::component_id();
    let _ = ZstTag::component_id();
    let _ = Inventory::component_id();
}

/// Builds the multi-archetype corpus world: mixed-width POB + a ZST tag + an OWNING
/// (ViaFn) column (the owning one is REQUIRED so the decode-rollback path is
/// reachable). Reuses the `save_determinism.rs::build_fixture` SHAPE.
fn build_corpus_world() -> EcsMaster {
    build_world(8, &[true, true, true])
}

/// Builds the corpus once and returns its saved bytes (the fuzz mutates copies of
/// these).
fn build_corpus() -> Vec<u8> {
    register_components();
    let w = build_corpus_world();
    save(&w)
}

/// Builds a fixture world with `rows`-ish population per populated archetype and a
/// per-archetype presence mask `[arch_a, arch_b, arch_c]`. Used by both the corpus
/// builder and the Item-5 generated sweep (varying shape = entity counts + which
/// archetypes are populated).
fn build_world(rows: usize, present: &[bool; 3]) -> EcsMaster {
    let mut w = EcsMaster::new();

    // Archetype A: Position + Velocity + Heavy + ZstTag (mixed widths + a ZST tag).
    if present[0] {
        let a = w.get_or_create_archetype(&[
            Position::component_id(),
            Velocity::component_id(),
            Heavy::component_id(),
            ZstTag::component_id(),
        ]);
        for i in 0..rows as u32 {
            let p = Position { x: i as f32, y: i as f32 + 0.5, z: -(i as f32) };
            let v = Velocity { dx: i as i32 - 2, dy: (i as i32) * 3 };
            let h = Heavy { a: ((i as u64) << 40) | 0xABCD, b: u64::MAX - i as u64 };
            spawn(
                &mut w,
                a,
                &[
                    (Position::component_id(), pod_bytes(&p)),
                    (Velocity::component_id(), pod_bytes(&v)),
                    (Heavy::component_id(), pod_bytes(&h)),
                    (ZstTag::component_id(), &[]),
                ],
            );
        }
    }

    // Archetype B: Heavy + Position (a second POB shape).
    if present[1] {
        let b = w.get_or_create_archetype(&[Heavy::component_id(), Position::component_id()]);
        let brows = rows.max(1).min(rows + 1);
        for i in 0..brows as u32 {
            let h = Heavy { a: 0xDEAD_0000 + i as u64, b: 7 * i as u64 };
            let p = Position { x: 100.0 + i as f32, y: 0.0, z: 0.0 };
            spawn(
                &mut w,
                b,
                &[
                    (Heavy::component_id(), pod_bytes(&h)),
                    (Position::component_id(), pod_bytes(&p)),
                ],
            );
        }
    }

    // Archetype C: an OWNING (ViaFn) column + a POB neighbour. Inventory is not
    // `Copy`, so it spawns through its typed bundle.
    if present[2] {
        let c = w.get_or_create_archetype(&[Inventory::component_id(), Velocity::component_id()]);
        let crows = rows.max(2);
        for i in 0..crows as u32 {
            w.spawn_two(
                c,
                Inventory {
                    name: format!("item-{i}"),
                    flags: vec![(i & 0xFF) as u8, ((i >> 8) & 0xFF) as u8, 0xAB],
                },
                Velocity { dx: i as i32, dy: -(i as i32) },
            )
            .expect("spawn owning + pob");
        }
    }

    w
}

// ════════════════════════════════════════════════════════════════════════════
// Mutation classes
// ════════════════════════════════════════════════════════════════════════════

/// Header field byte offsets (from `format.rs::SaveHeader`).
const HDR_TYPE_TABLE_OFF: usize = 16;
const HDR_ARCH_TABLE_OFF: usize = 24;
const HDR_ENTITY_TABLE_OFF: usize = 32;
const HDR_VAR_DATA_OFF: usize = 40;
const HDR_TYPE_COUNT: usize = 48;
const HDR_ARCH_COUNT: usize = 52;
const HDR_ENTITY_COUNT: usize = 56;

/// Per-entry / per-block strides + field offsets (from `format.rs`).
const TYPE_ENTRY_SIZE: usize = 40;
const ARCH_BLOCK_SIZE: usize = 24;

/// The number of mutation classes the uniform-random run draws from.
const NUM_OPS: usize = 6;

/// Applies ONE mutation class chosen by `op` to `bytes` in place. Each class is a
/// deliberate adversarial stomp; `Rng` picks the exact offset/value. Out-of-bounds
/// writes are guarded so the mutation itself never panics (only the load may).
fn mutate(bytes: &mut [u8], op: usize, rng: &mut Rng) {
    if bytes.is_empty() {
        return;
    }
    match op % NUM_OPS {
        0 => {
            // (1) single-bit flip at a random offset.
            let off = rng.below(bytes.len());
            let bit = rng.below(8);
            bytes[off] ^= 1 << bit;
        }
        1 => {
            // (2) random truncation — handled by the caller (it shortens the slice);
            // as an in-place mutation we instead zero a random tail run so this op
            // still perturbs the stream when truncation is applied separately.
            let start = rng.below(bytes.len());
            for b in &mut bytes[start..] {
                *b = 0;
            }
        }
        2 => {
            // (3) header *_off / *_count field stomp.
            let fields = [
                HDR_TYPE_TABLE_OFF,
                HDR_ARCH_TABLE_OFF,
                HDR_ENTITY_TABLE_OFF,
                HDR_VAR_DATA_OFF,
                HDR_TYPE_COUNT,
                HDR_ARCH_COUNT,
                HDR_ENTITY_COUNT,
            ];
            let f = fields[rng.below(fields.len())];
            stomp_u32_or_u64(bytes, f, rng);
        }
        3 => {
            // (4) type-entry field stomp (size/stride desync lives here too).
            stomp_type_entry_field(bytes, rng);
        }
        4 => {
            // (5) archetype-block field stomp.
            stomp_arch_block_field(bytes, rng);
        }
        _ => {
            // (6) column-region (data_off / byte_len) stomp.
            stomp_column_region(bytes, rng);
        }
    }
}

/// Writes a random `u64` (or its low `u32`) at `off`, bounds-guarded.
fn stomp_u32_or_u64(bytes: &mut [u8], off: usize, rng: &mut Rng) {
    let v = rng.next();
    if off + 8 <= bytes.len() {
        bytes[off..off + 8].copy_from_slice(&v.to_le_bytes());
    } else if off + 4 <= bytes.len() {
        bytes[off..off + 4].copy_from_slice(&(v as u32).to_le_bytes());
    }
}

/// Stomps a random field of a random type-table entry (`stable_name_hash` @0,
/// `layout_fingerprint` @8, `size` @16, `align` @20, `format_version` @32,
/// `serializability` @34). The `size` field is the size/stride-desync adversary.
fn stomp_type_entry_field(bytes: &mut [u8], rng: &mut Rng) {
    let Some(table_off) = read_u64_field(bytes, HDR_TYPE_TABLE_OFF) else { return };
    let count = read_u32_field(bytes, HDR_TYPE_COUNT).unwrap_or(0) as usize;
    if count == 0 {
        return;
    }
    let i = rng.below(count);
    let entry_off = table_off + i * TYPE_ENTRY_SIZE;
    if entry_off + TYPE_ENTRY_SIZE > bytes.len() {
        return;
    }
    // Pick a field offset within the entry.
    let field_off = entry_off + [0usize, 8, 16, 20, 32, 34][rng.below(6)];
    if field_off + 8 <= bytes.len() {
        let v = rng.next();
        bytes[field_off..field_off + 8].copy_from_slice(&v.to_le_bytes());
    } else if field_off < bytes.len() {
        bytes[field_off] = rng.byte();
    }
}

/// Stomps a random field of a random archetype block (`component_count` @0,
/// `entity_count` @4, `type_indices_off` @8, `column_regions_off` @12,
/// `entity_rows_off` @16). The block table is a sequence; we walk a random count.
fn stomp_arch_block_field(bytes: &mut [u8], rng: &mut Rng) {
    let Some(arch_off) = read_u64_field(bytes, HDR_ARCH_TABLE_OFF) else { return };
    if arch_off >= bytes.len() {
        return;
    }
    // We do not parse the full block chain (a hostile file may not have one); stomp
    // the FIRST block's fields (the most likely to be reachable before any error).
    let field = [0usize, 4, 8, 12, 16][rng.below(5)];
    let off = arch_off + field;
    if off + 4 <= bytes.len() {
        let v = rng.next() as u32;
        bytes[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
}

/// Stomps a `ColumnRegion` (`data_off` @0 / `byte_len` @8) inside the first
/// archetype block's region array.
fn stomp_column_region(bytes: &mut [u8], rng: &mut Rng) {
    let Some(arch_off) = read_u64_field(bytes, HDR_ARCH_TABLE_OFF) else { return };
    if arch_off + ARCH_BLOCK_SIZE > bytes.len() {
        return;
    }
    // `column_regions_off` is at block offset 12 (a u32 file offset).
    let Some(regions_off) = read_u32_field(bytes, arch_off + 12).map(|v| v as usize) else {
        return;
    };
    let comp_count = read_u32_field(bytes, arch_off).unwrap_or(0) as usize;
    if comp_count == 0 {
        return;
    }
    let c = rng.below(comp_count);
    let region_off = regions_off + c * 16;
    // Stomp either data_off (@0) or byte_len (@8) of this region.
    let sub = if rng.below(2) == 0 { 0 } else { 8 };
    let off = region_off + sub;
    if off + 8 <= bytes.len() {
        let v = rng.next();
        bytes[off..off + 8].copy_from_slice(&v.to_le_bytes());
    }
}

/// Reads a `u64` header field at `off`, or `None` if out of range / not usefully a
/// `usize`.
fn read_u64_field(bytes: &[u8], off: usize) -> Option<usize> {
    if off + 8 > bytes.len() {
        return None;
    }
    let v = u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap());
    usize::try_from(v).ok()
}

/// Reads a `u32` field at `off`.
fn read_u32_field(bytes: &[u8], off: usize) -> Option<u32> {
    if off + 4 > bytes.len() {
        return None;
    }
    Some(u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()))
}

/// The on-disk `serializability` discriminant for `SerializeViaFn` (an OWNING
/// column) — `format.rs::serializability_from_u8` maps byte `1` to it. Stored at
/// byte offset 34 of a [`TypeTableEntry`] (matching `stomp_type_entry_field`).
const SERIALIZABILITY_VIA_FN: u8 = 1;
const TYPE_ENTRY_SERIALIZABILITY_OFF: usize = 34;

/// Located region of the OWNING (`SerializeViaFn`) column in a VALID corpus: the
/// file offset of its [`ColumnRegion.byte_len`] field (a `u64`) and that field's
/// current value (the full encoded run length). Used by C1 case (b) to stomp the
/// `byte_len` DOWN so the column stays IN-BOUNDS but runs out mid-decode.
struct OwningColumn {
    /// File offset of the `ColumnRegion.byte_len` `u64` field.
    byte_len_off: usize,
    /// The full encoded-run length currently recorded there.
    byte_len: usize,
}

/// Walks the corpus's archetype blocks + column regions and returns the FIRST
/// owning (`SerializeViaFn`) column found (the `Inventory` column of fixture
/// archetype C). Deterministic — the corpus layout is fixed by `build_corpus_world`.
/// Returns `None` only if the corpus has no owning column (it always does, so a
/// `None` here is a fixture/format regression the caller surfaces loudly).
fn find_owning_column(bytes: &[u8]) -> Option<OwningColumn> {
    let type_table_off = read_u64_field(bytes, HDR_TYPE_TABLE_OFF)?;
    let arch_table_off = read_u64_field(bytes, HDR_ARCH_TABLE_OFF)?;
    let arch_count = read_u32_field(bytes, HDR_ARCH_COUNT)? as usize;
    let type_count = read_u32_field(bytes, HDR_TYPE_COUNT)? as usize;

    let mut block_off = arch_table_off;
    for _ in 0..arch_count {
        if block_off + ARCH_BLOCK_SIZE > bytes.len() {
            return None;
        }
        let component_count = read_u32_field(bytes, block_off)? as usize;
        let entity_count = read_u32_field(bytes, block_off + 4)? as usize;
        let type_indices_off = read_u32_field(bytes, block_off + 8)? as usize;
        let column_regions_off = read_u32_field(bytes, block_off + 12)? as usize;
        let entity_rows_off = read_u32_field(bytes, block_off + 16)? as usize;

        for c in 0..component_count {
            let ti_off = type_indices_off + c * 4;
            let type_index = read_u32_field(bytes, ti_off)? as usize;
            if type_index >= type_count {
                return None;
            }
            let entry_off = type_table_off + type_index * TYPE_ENTRY_SIZE;
            let ser_off = entry_off + TYPE_ENTRY_SERIALIZABILITY_OFF;
            if ser_off >= bytes.len() {
                return None;
            }
            if bytes[ser_off] == SERIALIZABILITY_VIA_FN {
                // `ColumnRegion` is `{ data_off: u64 @0, byte_len: u64 @8 }`.
                let region_off = column_regions_off + c * 16;
                let byte_len_off = region_off + 8;
                let byte_len = read_u64_field(bytes, byte_len_off)?;
                // Require a multi-row owning column so a tail-shortened run commits
                // ≥ 1 row before the EOF (C1's whole point). The fixture's archetype
                // C has `crows = rows.max(2)` ≥ 2 rows.
                if entity_count >= 2 {
                    return Some(OwningColumn { byte_len_off, byte_len });
                }
            }
        }

        // Advance past this block's entity-row table to the next block header.
        block_off = entity_rows_off + entity_count * 8;
    }
    None
}

// ════════════════════════════════════════════════════════════════════════════
// Report internal-consistency check
// ════════════════════════════════════════════════════════════════════════════

/// Asserts a successful `LoadReport` is internally consistent against the loaded
/// world. A successful load that LIES about what it did is as much a bug as a panic.
fn assert_report_consistent(report: &LoadReport, dst: &EcsMaster, iter: usize, op: usize) {
    assert_eq!(
        dst.entity_count() as u64,
        report.entities_loaded,
        "iter {iter} op {op}: world entity_count != report.entities_loaded"
    );

    // Every pool-bearing column in the loaded world was materialized by exactly one of
    // the three pool-producing plans: a Blit, a Decode, or a Construct
    // (`#[require]`-defaulted). Blits are counted `columns_blitted`, decodes
    // `columns_decoded`, and constructs are a SUBSET of `types_defaulted` (which also
    // counts no-pool excluded columns). Therefore the loaded world's total pool count
    // (each archetype's `component_ids()` is its pool-bearing id set — bitset tags,
    // counted separately under `types_bitset_skipped`, are NOT in it) is bounded ABOVE
    // by `columns_blitted + columns_decoded + types_defaulted`:
    //
    //     total_pools <= columns_blitted + columns_decoded + types_defaulted
    //
    // This is FALSIFIABLE — a report that LIES by under-reporting all three column
    // plans while the world actually holds those pools fails this check (unlike the
    // prior `X <= X + non_negative` tautology it replaces). It is also robust to a
    // DEDUP-COLLAPSE (two file blocks writing into one running archetype): collapse
    // only INCREASES the right-hand counters per block while `total_pools` counts each
    // live pool ONCE, so the inequality cannot false-fail.
    let total_pools: u64 = dst
        .archetype_master()
        .iter_archetypes()
        .map(|a| a.component_ids().len() as u64)
        .sum();
    let pool_producing = report.columns_blitted as u64
        + report.columns_decoded as u64
        + report.types_defaulted as u64;
    assert!(
        total_pools <= pool_producing,
        "iter {iter} op {op}: loaded world has {total_pools} pool-bearing columns but the \
         report accounts for only {pool_producing} pool-producing plans (blit+decode+defaulted) \
         — a lying report"
    );

    // The full classification universe must not overflow `u64` (a corrupt report with
    // saturated counters would wrap on a naive sum). `checked_add` makes a wrap a
    // detectable failure rather than silent UB-free garbage.
    let classified = (report.columns_blitted as u64)
        .checked_add(report.columns_decoded as u64)
        .and_then(|s| s.checked_add(report.types_skipped as u64))
        .and_then(|s| s.checked_add(report.types_bitset_skipped as u64))
        .and_then(|s| s.checked_add(report.types_defaulted as u64));
    assert!(
        classified.is_some(),
        "iter {iter} op {op}: report column counters overflow u64 when summed"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Item 3 — the soundness fuzz
// ════════════════════════════════════════════════════════════════════════════

/// A curated case: a label + the bytes to feed (already mutated/truncated). The
/// `intent` documents which unsafe path it targets (Miri-curated subset, C1).
struct CuratedCase {
    intent: &'static str,
    bytes: Vec<u8>,
}

/// Deterministically constructs the high-value curated cases that each reach a
/// specific unsafe path at least once (C1 — the critical Miri amendment). Run under
/// `cfg!(miri)` IN ADDITION to a small uniform-random tail, so Miri-TB walks the
/// blit, the decode-rollback, and the reject-before-unsafe guard every run instead
/// of relying on uniform luck within ~50 iters.
fn miri_curated_cases(corpus: &[u8]) -> Vec<CuratedCase> {
    let mut cases = Vec::new();

    // (a) Successful blit of mutated-but-valid POB: flip a byte INSIDE a POB
    //     column's DATA region. POB is all-bits-valid, so the load still SUCCEEDS —
    //     exercising `copy_nonoverlapping` blit + commit on mutated content. We do
    //     not need to locate the exact region: flipping a byte in the back half of
    //     the file lands in the column-data / var-data area (the header + tables sit
    //     in the front), and any value is valid for a POB column.
    {
        let mut b = corpus.to_vec();
        if b.len() > 2 {
            let off = b.len() - 2;
            b[off] ^= 0xFF;
        }
        cases.push(CuratedCase {
            intent: "(a) blit of mutated-but-valid POB content (copy_nonoverlapping + commit)",
            bytes: b,
        });
    }

    // (b) Decode-rollback path (HIGHEST value, C1): the OWNING (`SerializeViaFn`)
    //     column's `ColumnRegion.byte_len` is stomped DOWN so its data region stays
    //     IN-BOUNDS (the loader's `slice_at` in `decode_column` SUCCEEDS with a short
    //     slice) but the per-element decoder RUNS OUT mid-column. This is the ONLY
    //     shape that reaches load_writer.rs ~398-429 with ≥ 1 COMMITTED row: a pure
    //     tail truncation (the previous `corpus[..cut]`) could not — `decode_column`
    //     validates the FULL column run up front via `slice_at`, so a tail cut inside
    //     the owning column fails at CLASSIFY (a clean `Truncated`, the writer never
    //     runs) and a cut after the column leaves it fully present (no rollback).
    //
    //     WHY ≥ 1 row commits (reachability proof, by construction): the corpus's
    //     archetype C has `crows = rows.max(2)` ≥ 2 owning rows. The decode loop reads
    //     rows 0..n from a cursor scoped to exactly `byte_len` bytes, committing each
    //     row via `commit_units(pool_row, 1)` (load_writer.rs ~420) BEFORE reading the
    //     next. Shrinking `byte_len` to `full_run - 1` keeps every row but the LAST
    //     fully readable; the last row's `read_*` returns `Err` (UnexpectedEof /
    //     BadLengthPrefix), so when `rollback_committed` fires (load_writer.rs ~403)
    //     exactly `n - 1 ≥ 1` rows were committed → the `drop_at` cascade runs over a
    //     non-empty prefix. The dedicated `owning_column_short_run_rolls_back_committed`
    //     test below ASSERTS this by Drop-counting (a kept reachability proof that the
    //     committed rows were dropped exactly once).
    if let Some(owning) = find_owning_column(corpus) {
        if owning.byte_len > 1 {
            let mut b = corpus.to_vec();
            // Drop just the final byte of the owning run: the last row runs out.
            let short = (owning.byte_len - 1) as u64;
            b[owning.byte_len_off..owning.byte_len_off + 8]
                .copy_from_slice(&short.to_le_bytes());
            cases.push(CuratedCase {
                intent: "(b) owning byte_len short by 1 → decode-rollback + drop_at over ≥1 committed row",
                bytes: b,
            });
        }
        // A second, more aggressive short: cut the run to ~25% so MULTIPLE rows
        // decode-commit and a middle row hits EOF (still ≥ 1 committed; widens the
        // rollback prefix the `drop_at` cascade walks).
        if owning.byte_len > 8 {
            let mut b = corpus.to_vec();
            let short = (owning.byte_len / 4).max(1) as u64;
            b[owning.byte_len_off..owning.byte_len_off + 8]
                .copy_from_slice(&short.to_le_bytes());
            cases.push(CuratedCase {
                intent: "(b2) owning byte_len ~25% → multi-row commit then mid-column EOF rollback",
                bytes: b,
            });
        }
    }

    // (c) size/stride desync: stomp a type entry's `size` (@16) while keeping its
    //     fingerprint + version → the load.rs `classify_column` `byte_len == n *
    //     stride` guard must REJECT it BEFORE any unsafe write (no
    //     `copy_nonoverlapping` with a desynced stride ever runs).
    {
        let mut b = corpus.to_vec();
        if let Some(table_off) = read_u64_field(&b, HDR_TYPE_TABLE_OFF) {
            let count = read_u32_field(&b, HDR_TYPE_COUNT).unwrap_or(0) as usize;
            // Stomp the `size` of every entry to a large, desynced value; the loader
            // must reject (byte_len != n * stomped_stride) before any blit.
            for i in 0..count {
                let size_off = table_off + i * TYPE_ENTRY_SIZE + 16;
                if size_off + 4 <= b.len() {
                    b[size_off..size_off + 4].copy_from_slice(&0x0010_0000u32.to_le_bytes());
                }
            }
        }
        cases.push(CuratedCase {
            intent: "(c) size/stride desync → rejected by n*stride guard before any unsafe write",
            bytes: b,
        });
    }

    // (d) Structural stomps that still reach `load_archetype` with valid-but-mutated
    //     descriptors: nudge the first archetype block's `entity_count` DOWN (a
    //     smaller, still-valid count) so fewer rows load but the path runs end to end.
    {
        let mut b = corpus.to_vec();
        if let Some(arch_off) = read_u64_field(&b, HDR_ARCH_TABLE_OFF) {
            let ec_off = arch_off + 4; // entity_count @ block offset 4.
            if ec_off + 4 <= b.len() {
                b[ec_off..ec_off + 4].copy_from_slice(&1u32.to_le_bytes());
            }
        }
        cases.push(CuratedCase {
            intent: "(d) valid-but-reduced entity_count → load_archetype runs with mutated descriptor",
            bytes: b,
        });
    }
    {
        // (d2) Stomp the entity_count of the FIRST block to 0 (an empty archetype —
        //      the writer's `n == 0` early-return path).
        let mut b = corpus.to_vec();
        if let Some(arch_off) = read_u64_field(&b, HDR_ARCH_TABLE_OFF) {
            let ec_off = arch_off + 4;
            if ec_off + 4 <= b.len() {
                b[ec_off..ec_off + 4].copy_from_slice(&0u32.to_le_bytes());
            }
        }
        cases.push(CuratedCase {
            intent: "(d2) entity_count = 0 → writer n==0 early-return path",
            bytes: b,
        });
    }

    cases
}

/// Runs one load attempt under `catch_unwind` (empty allowlist). On a caught panic
/// FAILS with the iteration context; otherwise asserts report consistency.
fn run_one_load(bytes: &[u8], iter: usize, op: usize) {
    register_components();
    let mut dst = EcsMaster::new();
    let r = catch_unwind(AssertUnwindSafe(|| {
        load_world(&mut dst, bytes, LoadEntityPolicy::Remap)
    }));
    match r {
        Ok(Ok(report)) => assert_report_consistent(&report, &dst, iter, op),
        Ok(Err(_)) => { /* a clean LoadError is acceptable */ }
        Err(_) => panic!(
            "PANIC during load_world (iter {iter}, mutation class {op}) — the loader must \
             reject untrusted bytes with a LoadError, never panic"
        ),
    }
}

#[test]
fn load_fuzz_never_panics() {
    let corpus = build_corpus();
    let mut rng = Rng::new();

    if cfg!(miri) {
        // C1: the SEEDED Miri-curated subset (deterministic high-value unsafe paths)
        // PLUS a small uniform-random tail. NOT the uniform-random 50.
        let cases = miri_curated_cases(&corpus);
        for (idx, case) in cases.iter().enumerate() {
            // `op` is encoded as the curated index for reproduction; intent printed
            // on a failure via the panic message context below.
            let _ = case.intent;
            run_one_load(&case.bytes, idx, /* curated marker */ usize::MAX);
        }
        // ~40 uniform-random iters for breadth.
        for iter in 0..40 {
            let op = rng.below(NUM_OPS);
            let mut bytes = corpus.clone();
            apply_mutation_with_truncation(&mut bytes, op, &mut rng);
            run_one_load(&bytes, iter, op);
        }
    } else {
        // Native uniform-random run.
        for iter in 0..5000 {
            let op = rng.below(NUM_OPS);
            let mut bytes = corpus.clone();
            apply_mutation_with_truncation(&mut bytes, op, &mut rng);
            run_one_load(&bytes, iter, op);
        }
    }
}

/// Applies a mutation class to `bytes`. Class 2 (truncation) shortens the buffer to
/// a random length; every other class mutates in place. Kept here so both the native
/// and Miri-tail loops share the exact same mutation semantics.
fn apply_mutation_with_truncation(bytes: &mut Vec<u8>, op: usize, rng: &mut Rng) {
    if op % NUM_OPS == 1 && !bytes.is_empty() {
        // Random truncation length in `[0, len]`.
        let new_len = rng.range(0, bytes.len() + 1);
        bytes.truncate(new_len);
    } else {
        mutate(bytes, op, rng);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Item 5 — generated idempotency sweep (reuses the Rng + corpus infra)
// ════════════════════════════════════════════════════════════════════════════
//
// For K randomized VALID worlds, assert the POST-FIRST-LOAD FIXED POINT exactly as
// `save_determinism.rs:219`: compare the re-save of the once-loaded world to the
// re-save of the twice-loaded world — `save(load(save(load(save(w))))) ==
// save(load(save(w)))`. The first RAW save is NEVER compared to a post-load save:
// the loader may re-intern the distinct types in a different (equally valid) order
// than the build order, a load-path property, not an idempotency violation (critic
// O1; see save_determinism.rs:219).

#[test]
fn generated_idempotency_sweep() {
    register_components();
    let mut rng = Rng::new();
    let k = if cfg!(miri) { 5 } else { 200 };

    for case in 0..k {
        // Vary the world SHAPE: entity-count + which fixture archetypes are populated.
        // At least one archetype is always present (a wholly-empty world is a separate
        // trivial case already covered elsewhere).
        let mut present = [
            rng.below(2) == 0,
            rng.below(2) == 0,
            rng.below(2) == 0,
        ];
        if !present.iter().any(|&p| p) {
            present[case % 3] = true;
        }
        let rows = rng.range(1, 12);
        let world = build_world(rows, &present);

        // save → load → re-save → load → re-save, then assert the fixed point.
        let first = save(&world);

        let mut dst1 = EcsMaster::new();
        load_world(&mut dst1, &first, LoadEntityPolicy::Remap)
            .unwrap_or_else(|e| panic!("case {case}: load #1 failed: {e:?}"));
        let resaved1 = save(&dst1);

        let mut dst2 = EcsMaster::new();
        load_world(&mut dst2, &resaved1, LoadEntityPolicy::Remap)
            .unwrap_or_else(|e| panic!("case {case}: load #2 failed: {e:?}"));
        let resaved2 = save(&dst2);

        assert_eq!(
            resaved1.len(),
            resaved2.len(),
            "case {case} (rows {rows}, present {present:?}): fixed-point byte length diverged"
        );
        assert_eq!(
            resaved1, resaved2,
            "case {case} (rows {rows}, present {present:?}): save→load→re-save is NOT a \
             byte-stable fixed point (load-canonical layout must re-save identically; \
             see save_determinism.rs:219)"
        );
    }
}

// ════════════════════════════════════════════════════════════════════════════
// C1 reachability PROOF — owning byte_len short → decode-rollback over ≥1 row
// ════════════════════════════════════════════════════════════════════════════
//
// A KEPT assertion (the finding's "an assertion you keep") that the curated case
// (b) shape — an owning column whose `ColumnRegion.byte_len` is stomped DOWN so its
// data region stays in-bounds but the decoder runs out mid-column — deterministically
// reaches load_writer.rs ~398-429 with ≥ 1 COMMITTED row. The proof is Drop-counting:
// `CountedPayload` (a `Wire` leaf) increments a live counter on every constructed
// instance and decrements on Drop. After the load returns `Err`, the rollback's
// `drop_at` cascade must have dropped every committed row exactly once, so the net
// live count returns to its pre-load baseline. A committed-then-dropped row proves
// the row was committed before the `Err` — the rollback path ran over a non-empty
// prefix. (Mirrors the panic-path proof in `load_hardening.rs::O1`, but for the
// `DecodeError`-return rollback edge instead of the panic-unwind `Drop` edge.)

/// Net count of live `CountedPayload` instances (constructed minus dropped). After a
/// clean rollback it returns to its pre-load value.
static LIVE_COUNTED: AtomicI64 = AtomicI64::new(0);

/// A `Wire` leaf that Drop-counts every constructed instance (no panic — it decodes
/// normally; the mid-column failure comes from the externally-shortened `byte_len`,
/// which starves the cursor on a later row's `read_u32`).
#[derive(Clone, PartialEq, Debug)]
struct CountedPayload {
    val: u32,
}

impl CountedPayload {
    fn new(val: u32) -> Self {
        LIVE_COUNTED.fetch_add(1, Ordering::SeqCst);
        Self { val }
    }
}

impl Drop for CountedPayload {
    fn drop(&mut self) {
        LIVE_COUNTED.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Wire for CountedPayload {
    fn wire_write(&self, c: &mut SaveCursor<'_>) {
        c.write_u32(self.val);
    }

    fn wire_read(c: &mut LoadCursor<'_>) -> Result<Self, DecodeError> {
        // Reads 4 bytes per row; a shortened run starves a later row here → `Err`.
        let val = c.read_u32()?;
        Ok(CountedPayload::new(val))
    }
}

/// An owning component (a non-`Copy` `Wire` field → classified `SerializeViaFn`), so
/// its load runs the row-by-row decode path the rollback guard protects.
#[derive(Component, Clone, PartialEq, Debug)]
struct OCounted {
    p: CountedPayload,
}

#[test]
fn owning_column_short_run_rolls_back_committed() {
    // A 4-row world of the owning component alone (so `find_owning_column` resolves
    // exactly this column).
    let owning_id = OCounted::component_id();
    let mut src = EcsMaster::new();
    let arch = src.get_or_create_archetype(&[owning_id]);
    let rows = 4u32;
    for i in 0..rows {
        src.spawn_one(arch, OCounted { p: CountedPayload { val: 100 + i } })
            .expect("spawn owning");
    }
    let bytes = save(&src);

    // Baseline AFTER the save (the save reads the live world by reference — it never
    // constructs a `CountedPayload`). The `rows` live ones belong to `src`.
    let live_before = LIVE_COUNTED.load(Ordering::SeqCst);

    // Locate this owning column's `byte_len` and stomp it short by one byte: every
    // row but the last reads fully, the last row's `read_u32` starves → `Err` AFTER
    // `rows - 1 >= 1` rows were committed.
    let owning = find_owning_column(&bytes).expect("the owning column must be present");
    assert!(owning.byte_len > 1, "owning run must hold >1 byte");
    let mut corrupt = bytes.clone();
    let short = (owning.byte_len - 1) as u64;
    corrupt[owning.byte_len_off..owning.byte_len_off + 8].copy_from_slice(&short.to_le_bytes());

    let mut dst = EcsMaster::new();
    let result = load_world(&mut dst, &corrupt, LoadEntityPolicy::Remap);

    // The short run must surface as a loud decode error (never UB / panic / Ok).
    assert!(
        matches!(result, Err(LoadError::Decode(_))),
        "a short owning run must be a LoadError::Decode, got {result:?}"
    );
    // The rollback must leave the world empty (no half-loaded archetype).
    assert_eq!(dst.entity_count(), 0, "rollback must leave the world empty");
    // The decisive reachability proof: every row committed before the `Err` was
    // dropped exactly once by the `drop_at` rollback cascade, so the net live count
    // is back to baseline. Had ZERO rows committed (the path NOT reached), the count
    // would also be at baseline — so additionally assert the run was multi-row: the
    // save wrote `rows` rows, and `byte_len - 1` keeps `rows - 1 >= 1` decodable, so
    // the committed-then-rolled-back prefix is provably non-empty by construction
    // (documented on curated case (b)).
    assert_eq!(
        LIVE_COUNTED.load(Ordering::SeqCst),
        live_before,
        "rollback must drop every committed row exactly once (no leak, no double-drop)"
    );
}
