/// Virtual-memory commit granularity (Phase X.F, renamed from
/// `ARENA_COMMIT_GRANULE` when Phase X.J retired the shared Arena): 64 KiB —
/// the Windows reservation granularity, and a multiple of the 4 KiB
/// commit/`mprotect` page size everywhere. Every `VmReservation` length is
/// rounded up to this (`os_len = align_up(len, COMMIT_GRANULE)`) so a
/// frontier commit can never overrun the kernel's page-rounded mapping.
pub const COMMIT_GRANULE: usize = 64 * 1024;

/// Typical CPU cache line size in bytes
/// Used for memory alignment to optimize cache usage
pub const CACHE_LINE_SIZE: usize = 64;

/// Minimum alignment for components (8 bytes)
/// Ensures components have at least this alignment even if their actual type
/// requires less alignment
pub const MIN_ALIGNMENT: usize = 8;

/// Minimum alignment for `ComponentPool` backing buffers.
///
/// Lifted from `align_of::<T>()` to ensure column-start addresses are AVX2-loadable
/// without an unaligned-prologue. Required by `Query::for_each_chunk` SIMD-amenable
/// inner loops (Phase X.A). 32 = AVX2 baseline; AVX-512 (64-byte) is opt-in via a
/// future `SIMD_BUFFER_ALIGN_AVX512` cfg-gated constant if needed.
///
/// See `docs/PHASE-X.A-PLAN.md` §6 for the design rationale and cost analysis.
pub const SIMD_BUFFER_ALIGN: usize = 32;

//
// Component-pool sizing (Phase X.I)
//

/// Target data-region size for a default-sized `ComponentPool` (Phase X.I
/// D2): each pool owns one `VmReservation` laid out
/// `[data | added_ticks | changed_ticks]`, and the default row ceiling is
/// `clamp(POOL_TARGET_DATA_BYTES / stride, POOL_MIN_ROWS, POOL_MAX_ROWS)`
/// (see `pool_reserve_rows`). 1 GiB on the 64-bit OS-syscall arms —
/// virtual address space only (no commit charge until `grow_rows` commits
/// at the frontier), aligned with the Phase X.G 67 M-entity inland ceiling.
///
/// VA budget: 3000 pools (1000 archetypes × 3 pools) reserve ≤ ~3.4 TiB =
/// 2.7% of the 128 TB user VA on Windows/Linux alike. OS-knob note: each
/// pool contributes ≤ 6 VMAs/VADs (3 sub-regions × committed prefix +
/// `PROT_NONE` tail) ⇒ ≤ 18,000 at 3000 pools vs the Linux default
/// `vm.max_map_count` of 65,530 (3.6× headroom) — the one OS limit a
/// pathological embedder could approach.
///
/// Kernel-memory audit F1/F3/F4 amendment: each archetype additionally owns
/// one `VmColumn<EntityId>` (`entity_ids`, F1) reserved at
/// `POOL_MAX_ROWS × 8 B` = **128 MiB of VA** (lazy — nothing materializes
/// until the first push), and each `DenseStore` one `s2e` column (F3) at
/// `reserve_rows × 8 B` (≤ 128 MiB at the ceiling). At 1000 archetypes that
/// is +125 GiB VA — noise against the 3.4 TiB pool budget — plus ≤ 2
/// VMAs/VADs per MATERIALIZED column (committed prefix + `PROT_NONE` tail)
/// ⇒ +2,000, still ≥ 3× headroom under `vm.max_map_count`. Resident floor:
/// one `POOL_MIN_SLAB` (64 KiB) commit per NON-EMPTY column — empty
/// archetypes/stores commit nothing. The dense bookkeeping arrays stay small
/// heap `Vec`s (F4: floor + amortized growth), so none of this eagerly
/// commits resident memory on the syscall arms.
#[cfg(all(not(miri), any(windows, unix), target_pointer_width = "64"))]
pub const POOL_TARGET_DATA_BYTES: usize = 1024 * 1024 * 1024;
/// Fallback-arm target (Miri / wasm32 / 32-bit / exotic): 4 MiB. The
/// fallback `VmReservation::reserve` is an EAGER `alloc_zeroed` of the full
/// `os_len` (commit is a no-op), so the footprint scales per-POOL — worst
/// eager footprint per pool is 4 MiB data + 2 × 1 MiB ticks = 6 MiB
/// (★R1-3). Large strides SHRINK vs the pre-X.I class ceilings (e.g.
/// 192 B → 21,845 rows vs 32,768) — documented, loud-panic at the reserve
/// ceiling; >4 MiB of one large-component pool on wasm/Miri is out of scope.
#[cfg(not(all(not(miri), any(windows, unix), target_pointer_width = "64")))]
pub const POOL_TARGET_DATA_BYTES: usize = 4 * 1024 * 1024;

/// Minimum row ceiling for a default-sized pool (Phase X.I D2): 65,536 on
/// the syscall arms — never below the pre-X.I medium-class ceiling. Binds
/// for strides > 16 KiB (VA-only cost; commit stays demand-driven).
#[cfg(all(not(miri), any(windows, unix), target_pointer_width = "64"))]
pub const POOL_MIN_ROWS: usize = 65_536;
/// Fallback-arm minimum row ceiling: 256 — caps the huge-stride eager
/// blowup (256 × 64 KiB = 16 MiB, pathological ≥ 16 KiB strides only).
#[cfg(not(all(not(miri), any(windows, unix), target_pointer_width = "64")))]
pub const POOL_MIN_ROWS: usize = 256;

/// Maximum row ceiling for a default-sized pool (Phase X.I D2): 2^24 =
/// 16,777,216 on the syscall arms. Binds for strides < 64 B (caps the tick
/// sub-regions at 64 MiB each, ALWAYS).
#[cfg(all(not(miri), any(windows, unix), target_pointer_width = "64"))]
pub const POOL_MAX_ROWS: usize = 16_777_216;
/// Fallback-arm maximum row ceiling: 2^18 = 262,144 — exactly the pre-X.I
/// tiny-class ceiling, so every wasm/Miri workload that worked keeps
/// working (★R1-3: all `boyko_demo` strides are ≤ 16 B and land here).
#[cfg(not(all(not(miri), any(windows, unix), target_pointer_width = "64")))]
pub const POOL_MAX_ROWS: usize = 262_144;

// Phase X.I D2: `EntityInland.unit_index` is a `u32`; the row ceiling must
// stay strictly below `u32::MAX` so every dense row index fits (2^24 ≪ 2^32).
const _: () = assert!(POOL_MAX_ROWS < u32::MAX as usize);
// The clamp must be a real interval and the MIN floor must keep `unit_index`
// representable on every arm.
const _: () = assert!(POOL_MIN_ROWS <= POOL_MAX_ROWS && POOL_MIN_ROWS > 0);

/// Minimum pool data-commit slab (Phase X.I D4): 64 KiB = one commit
/// granule — the floor that keeps sparse archetypes cheap (a 1-row
/// archetype commits 3 × 64 KiB per pool, not megabytes). Doubling from
/// here reaches any real population in ≤ a dozen µs-scale events.
pub const POOL_MIN_SLAB: usize = 64 * 1024;

/// Maximum pool data-commit step (Phase X.I D4): 64 MiB — bounds
/// commit-charge overshoot by one slab (the X.F overshoot-honesty bound);
/// one max-step costs ≤ ~50 µs (the Phase X.F B4 envelope). A larger
/// REQUEST is not clamped (the request-dominant `max` in
/// `pool_commit_step` always covers it).
pub const POOL_MAX_SLAB: usize = 64 * 1024 * 1024;

// ── Phase X.I pure sizing / layout math (D1 + D2 + D4) ─────────────────────
//
// Consumed by `ComponentPool` (`memory/component_pool.rs`); kept next to the
// constants they interpret so the policy is auditable in one place. All
// arithmetic is checked — overflow panics loudly at construction time, never
// wraps into a silently-undersized reservation.

/// Checked granule round-up (twin of `vm.rs::checked_align_up`, const form).
pub(crate) const fn pool_align_up_granule(value: usize) -> usize {
    match value.checked_add(COMMIT_GRANULE - 1) {
        Some(v) => v & !(COMMIT_GRANULE - 1),
        None => panic!("pool_align_up_granule: overflow (value too close to usize::MAX)"),
    }
}

/// Phase X.I D2 sizing formula: byte-targeted, row-clamped default ceiling.
///
/// `reserve_rows(stride) = clamp(POOL_TARGET_DATA_BYTES / stride,
/// POOL_MIN_ROWS, POOL_MAX_ROWS)`. Used by
/// `ComponentPool::with_default_sizes` (in-crate) and, cross-crate, by every
/// `ScratchColumn<T>`-backed transient scratch (e.g.
/// `boyko_render::mesh_draw::MeshRenderScratch`) that wants the SAME
/// VA-reservation-class ceiling every other kernel column uses, instead of a
/// bespoke fixed cap — the legacy explicit-ceiling constructor
/// `ComponentPool::new` bypasses the clamp by design (★R1-9).
///
/// Phase 22 D6 (ZST/tag pools): `stride == 0` routes straight to
/// [`POOL_MAX_ROWS`] — row capacity is bounded by the tick sub-regions
/// alone, the same ceiling a 1-byte component hits. The reservation is
/// address space only (zero commit charge): at the ceiling a ZST pool
/// reserves `2 × tick_len` = 2^24 rows × 4 B × 2 regions = **128 MiB of
/// virtual address space per tag pool per hosting archetype** (2 MiB under
/// the cfg-fallback `POOL_MAX_ROWS = 262_144`), with zero resident bytes
/// until rows commit.
pub const fn pool_reserve_rows(stride: usize) -> usize {
    if stride == 0 {
        return POOL_MAX_ROWS;
    }
    let by_bytes = POOL_TARGET_DATA_BYTES / stride;
    if by_bytes < POOL_MIN_ROWS {
        POOL_MIN_ROWS
    } else if by_bytes > POOL_MAX_ROWS {
        POOL_MAX_ROWS
    } else {
        by_bytes
    }
}

/// Number of distinct cache-line offsets the per-pool base stagger spreads
/// across (P2-CACHE-FIX): 64 lines × 64 B = one 4 KiB page. A modern L1d is
/// 8-way × 64 sets × 64 B = 32 KiB, so stepping the leading offset by one
/// cache line per `component_id` (mod 64) lands consecutive pools' element-`i`
/// rows in 64 *different* L1 sets (and, since the stride is a cache line,
/// 64 different L2 sets too).
const POOL_STAGGER_LINES: usize = 64;

// The stagger granule must preserve the SIMD data-base alignment guarantee:
// each step is a whole `CACHE_LINE_SIZE` (64 B) and the data sub-region starts
// at `stagger`, so the data base is `reservation_base + stagger`. A 64 KiB-
// aligned reservation base plus a 64 B-multiple stagger stays
// SIMD_BUFFER_ALIGN-aligned iff 64 is a multiple of SIMD_BUFFER_ALIGN.
const _: () = assert!(
    CACHE_LINE_SIZE.is_multiple_of(SIMD_BUFFER_ALIGN),
    "POOL_STAGGER_LINES step (CACHE_LINE_SIZE) must preserve SIMD_BUFFER_ALIGN"
);

/// Per-pool leading byte offset that spreads each pool's data (and tick)
/// sub-regions across distinct cache sets (P2-CACHE-FIX).
///
/// Every `ComponentPool` reservation comes from `VirtualAlloc`/`mmap` at the
/// 64 KiB OS reservation granularity, so EVERY pool's base has its low 16 bits
/// zero. A SoA hot loop that sweeps many columns at index `i` (the rigid solver
/// touches ~24 contact columns per contact) then maps element `i` of every
/// column to the SAME L1 set (32 KiB) and L2 set (512 KiB), turning an 8-way
/// set into a conflict-miss storm. Staggering each pool's in-reservation base
/// by `(component_id % 64) × CACHE_LINE_SIZE` returns the heap-`Vec` behavior
/// (scattered offsets ⇒ spread across sets) measured to recover the ~40%
/// rigid-solver regression from the P2 ScratchColumn migration.
///
/// The result is always a multiple of [`CACHE_LINE_SIZE`] (64 B), hence a
/// multiple of [`SIMD_BUFFER_ALIGN`] (the const assert above pins this), so the
/// staggered data base preserves the AVX2 alignment contract. It is strictly
/// less than one page (`< 64 × 64 = 4096`), so it costs at most one extra
/// lazily-committed page per pool.
pub(crate) const fn pool_base_stagger(component_id: usize) -> usize {
    (component_id % POOL_STAGGER_LINES) * CACHE_LINE_SIZE
}

/// Byte layout of a pool's single reservation (Phase X.I D1, P2-CACHE-FIX
/// stagger): `[pad | data | added_ticks | changed_ticks]`, every *sub-region*
/// (data and both ticks) granule-aligned, shifted right by a per-pool leading
/// `stagger` pad.
///
/// ```text
/// stagger     = pool_base_stagger(component_id)   (0 ≤ stagger < 4096)
/// data_off    = stagger                            (leading pad, SIMD-aligned)
/// data_len    = align_up(reserve_rows × stride, G)
/// added_off   = stagger + data_len;   tick_len = align_up(reserve_rows × 4, G)
/// changed_off = stagger + data_len + tick_len
/// os_len      = align_up(stagger + data_len + 2 × tick_len, G)
/// ```
///
/// The `4` is `size_of::<UnsafeCell<Tick>>()` — pinned by a const assert in
/// `component_pool.rs` and the U-P6 transmute test.
///
/// P2-CACHE-FIX: `stagger` (a per-pool, [`pool_base_stagger`]-derived,
/// `CACHE_LINE_SIZE`-multiple leading pad) shifts ALL three sub-regions right
/// so different columns' element-`i` rows land in different cache sets. It is a
/// multiple of [`SIMD_BUFFER_ALIGN`] (the const assert on `POOL_STAGGER_LINES`
/// pins this), and a granule base plus a 64 B-multiple stagger keeps the data
/// base SIMD-aligned. `os_len` is re-rounded UP to a granule so the frontier
/// commit can never overrun the kernel's page-rounded mapping (the pad pushes
/// the tail off the granule boundary it had at `stagger == 0`).
///
/// Phase 22 D6 (ZST/tag pools): for `stride == 0` the data sub-region is
/// vacuous (`[stagger, stagger)`) — `data_bytes = 0`, `data_len = align_up(0)
/// = 0`, `added_off = stagger`, `changed_off = stagger + tick_len`, `os_len =
/// align_up(stagger + 2 × tick_len, G)`. The two tick regions remain disjoint
/// (`[stagger, stagger+tick_len)` / `[stagger+tick_len, stagger+2·tick_len)`):
/// the ★R1-8 disjointness proof becomes vacuous for data-vs-tick and is
/// unchanged for tick-vs-tick.
pub(crate) struct PoolByteLayout {
    /// Per-pool leading pad (also `data_off`); `CACHE_LINE_SIZE`-multiple,
    /// `< 4096` (P2-CACHE-FIX). The data sub-region starts here, not at 0.
    pub(crate) data_off: usize,
    /// Granule-aligned data sub-region length.
    pub(crate) data_len: usize,
    /// Granule-aligned length of EACH tick sub-region.
    pub(crate) tick_len: usize,
    /// Offset of the `added` tick sub-region (== `data_off + data_len`).
    pub(crate) added_off: usize,
    /// Offset of the `changed` tick sub-region
    /// (== `data_off + data_len + tick_len`).
    pub(crate) changed_off: usize,
    /// Total reservation length
    /// (`align_up(data_off + data_len + 2 × tick_len, G)`).
    pub(crate) os_len: usize,
}

/// Computes the D1 layout with the P2-CACHE-FIX leading stagger and checked
/// arithmetic (overflow panics loudly). `stagger` is the single source of
/// truth used by BOTH `ComponentPool::new` and `ComponentPool::grow_rows`
/// (callers pass `pool_base_stagger(component_id)`), so their commit offsets
/// can never drift.
pub(crate) const fn pool_byte_layout(
    reserve_rows: usize,
    stride: usize,
    stagger: usize,
) -> PoolByteLayout {
    // Belt assert — `ComponentPool::new` fires the loud constructor-naming
    // asserts (★R1-5) before reaching this math. `stride == 0` is a VALID
    // input (Phase 22 D6): the math degrades to the vacuous-data-region
    // layout documented on `PoolByteLayout` with no further branching.
    assert!(reserve_rows > 0, "pool_byte_layout: reserve_rows must be non-zero");
    // P2-CACHE-FIX: the stagger is a SIMD-aligned leading pad strictly below
    // one page; a value above it would defeat the per-page cost bound and
    // signal a caller computing it outside `pool_base_stagger`.
    assert!(
        stagger < 4096 && stagger.is_multiple_of(SIMD_BUFFER_ALIGN),
        "pool_byte_layout: stagger must be SIMD-aligned and < one page"
    );

    let data_bytes = match reserve_rows.checked_mul(stride) {
        Some(v) => v,
        None => panic!("pool_byte_layout: reserve_rows * stride overflows usize"),
    };
    let data_len = pool_align_up_granule(data_bytes);

    // 4 == size_of::<UnsafeCell<Tick>>() (const-asserted in component_pool.rs).
    let tick_bytes = match reserve_rows.checked_mul(4) {
        Some(v) => v,
        None => panic!("pool_byte_layout: reserve_rows * 4 overflows usize"),
    };
    let tick_len = pool_align_up_granule(tick_bytes);

    // The data sub-region starts AFTER the leading stagger pad; every tick
    // offset shifts by `stagger` accordingly.
    let data_off = stagger;
    let added_off = match data_off.checked_add(data_len) {
        Some(v) => v,
        None => panic!("pool_byte_layout: data_off + data_len overflows usize"),
    };
    let changed_off = match added_off.checked_add(tick_len) {
        Some(v) => v,
        None => panic!("pool_byte_layout: added_off + tick_len overflows usize"),
    };
    let tail = match changed_off.checked_add(tick_len) {
        Some(v) => v,
        None => panic!("pool_byte_layout: changed_off + tick_len overflows usize"),
    };
    // The stagger pushes the tail off the granule boundary it had at
    // `stagger == 0`, so re-round UP to a granule (twin of the COMMIT_GRANULE
    // contract: every reservation length is granule-rounded so a frontier
    // commit never overruns the kernel mapping). `pool_align_up_granule`
    // panics loudly on overflow.
    let os_len = pool_align_up_granule(tail);

    PoolByteLayout {
        data_off,
        data_len,
        tick_len,
        added_off,
        changed_off,
        os_len,
    }
}

/// Phase X.I D4 commit-step policy: data-region byte doubling clamped to
/// `[POOL_MIN_SLAB, POOL_MAX_SLAB]`, request-dominant.
///
/// `step = clamp(data_committed, MIN_SLAB, MAX_SLAB)
///           .max(needed.saturating_sub(data_committed))`
///
/// The `saturating_sub` is a belt: `grow_rows` proves `needed >
/// data_committed` before calling (GROW1-XI corollary 0a), and
/// `grow_rows_zst` proves the tick analogue `needed_t > ticks_committed`
/// (Z4), so the sub never actually saturates on either real path.
pub(crate) const fn pool_commit_step(data_committed: usize, needed: usize) -> usize {
    let doubling = if data_committed < POOL_MIN_SLAB {
        POOL_MIN_SLAB
    } else if data_committed > POOL_MAX_SLAB {
        POOL_MAX_SLAB
    } else {
        data_committed
    };
    let request = needed.saturating_sub(data_committed);
    if request > doubling { request } else { doubling }
}

//
// Archetype and entity configuration
//

/// Default virtual-address reservation for the entity-metadata store
/// (`InlandStore`, Phase X.G): 1 GiB = 67,108,864 16-byte `EntityInland`
/// slots on the 64-bit OS-syscall arms — the per-pool component-data
/// ceilings (`POOL_TARGET_DATA_BYTES`) exhaust long before 67 M real
/// entities. Reservation is address space only (no commit charge, no
/// resident pages). Tooling note: large `PROT_NONE`/`PAGE_NOACCESS`
/// reservations show up in ASan/valgrind-class tooling as *address space*,
/// not memory.
#[cfg(all(not(miri), any(windows, unix), target_pointer_width = "64"))]
pub const DEFAULT_INLAND_RESERVE: usize = 1024 * 1024 * 1024;
/// Fallback default (Miri / wasm32 / exotic / 32-bit): the reservation is
/// eagerly allocated ZEROED from the global allocator, so it must stay small.
/// 16 MiB = 1,048,576 entity slots — no Miri/wasm workload approaches that.
/// Native wasm32 cost: one eager zeroed 16 MiB per world, accepted because
/// the shipping wasm demo creates exactly one world (Phase X.G R2-W3).
#[cfg(not(all(not(miri), any(windows, unix), target_pointer_width = "64")))]
pub const DEFAULT_INLAND_RESERVE: usize = 16 * 1024 * 1024;

/// Smallest commit slab for the entity-metadata store (Phase X.G D2):
/// 256 KiB = 16,384 slots — covers the 9192-slot first request (1000 +
/// MAX_BATCH_HINT) in a single event. Granule (`COMMIT_GRANULE`)
/// multiple.
pub const INLAND_MIN_SLAB: usize = 256 * 1024;

/// Largest geometric commit step for the entity-metadata store (Phase X.G
/// D2): 16 MiB = 1,048,576 slots — one max-step covers a 1 M-entity world;
/// bounds commit-charge overshoot by one slab.
pub const INLAND_MAX_SLAB: usize = 16 * 1024 * 1024;

//
// Event dispatch configuration
//

/// Maximum number of worker threads that can send events concurrently.
/// Controls the number of per-type writer lanes in `EventBuffer<E>`.
pub const MAX_EVENT_THREADS: u32 = 64;

/// Maximum events per lane per frame in `EventBuffer<E>`.
/// Bounds the per-lane write buffer allocation at preregister time.
pub const MAX_EVENT_CAPACITY: u32 = 16384;

//
// Enable-tag (bitset storage) configuration
//

/// Maximum number of dynamic enable-tag terms (`with_enabled` /
/// `without_enabled`) a single query may carry (EnableTag plan, D2).
///
/// Bounds the inline `EnableTerms` stack struct in the query cursor so dynamic
/// enable filtering never spills to the heap on the hot path. Typed
/// `Enabled<T>` / `Disabled<T>` terms do not consume this budget — only the
/// runtime-added dynamic terms do.
pub const MAX_ENABLE_TERMS: usize = 8;

//
// Observer propagation configuration (Feature 2)
//

/// Maximum number of `ChildOf` hops a single custom-trigger propagation walk may
/// take before a debug tripwire fires (Feature 2 D5).
///
/// A `ChildOf` cycle (A→B→…→A) would loop the bubble walk indefinitely. Cycles
/// are a documented footgun (only direct self-reference is guarded in the
/// hierarchy), so this is a `debug_assert!` cap — free in release, catches a
/// malformed hierarchy in debug. Sized far above any realistic scene depth.
pub const MAX_PROPAGATION_DEPTH: usize = 1024;

//
// Deferred-hook drain backstop (Relations v1, W4 / C1)
//

/// Hard upper bound on the number of re-entrant drain *turns* a single outermost
/// `drain_deferred_hook_queue` may take before a `#[cold]` runaway backstop
/// aborts the drain (Relations v1, W4 / C1).
///
/// One turn = one `apply_via_raw_twin` batch; a re-entrant hook-enqueued command
/// (link / unlink / `LINKED_DESPAWN` cascade) produces the NEXT turn. A *cyclic*
/// `LINKED_DESPAWN` graph already terminates naturally — a re-entered despawn of
/// an already-freed entity is a clean generation-checked no-op in
/// `delete_entity_core`, so each cyclic entity is despawned exactly once and the
/// live set strictly shrinks per real despawn. This bound is therefore a BLUNT
/// backstop against a *pathological* non-terminating re-enqueue (a future
/// relation that resurrects entities, or a malformed hook) — NOT the primary
/// cycle-termination mechanism. It is a cross-level bound on the FLAT drain queue
/// (where the cascade actually recurses), not a per-hook depth count: the broken
/// per-hook RAII guard could never accumulate across the flat queue (every
/// cascade level fired at depth 1).
///
/// Sized far above any realistic per-drain turn count: a legitimate cascade takes
/// turns proportional to the tree/graph DEPTH (bounded by `MAX_PROPAGATION_DEPTH`
/// in well-formed scenes), never to the entity COUNT (a wide despawn of M
/// children is still one cascade level = a bounded turn count). The backstop only
/// fires for an unbounded re-enqueue that would otherwise hang.
pub const MAX_HOOK_DRAIN_TURNS: usize = 1 << 24;

// ── Phase X.I W1 — U-P1 sizing/slab table tests + U-P6 Tick::ZERO pin ──────

#[cfg(test)]
mod tests {
    use std::cell::UnsafeCell;

    use super::*;
    use crate::ecs::core::change_detection::Tick;

    const G: usize = COMMIT_GRANULE;
    const KIB: usize = 1024;
    const MIB: usize = 1024 * 1024;

    /// U-P1 — D2 class-ceiling table on the syscall arms (1 GiB target),
    /// including both clamp edges: 64 B is EXACTLY the MAX-clamp boundary
    /// (2^30 / 2^6 = 2^24 = `POOL_MAX_ROWS`) and 16 KiB is EXACTLY the
    /// MIN-clamp boundary (2^30 / 2^14 = 2^16 = `POOL_MIN_ROWS`).
    #[cfg(all(not(miri), any(windows, unix), target_pointer_width = "64"))]
    #[test]
    fn pool_reserve_rows_syscall_arm_table() {
        assert_eq!(pool_reserve_rows(12), POOL_MAX_ROWS, "12 B: row cap binds");
        assert_eq!(pool_reserve_rows(64), POOL_MAX_ROWS, "64 B: exact MAX edge");
        assert_eq!(pool_reserve_rows(65), 16_519_104, "65 B: just past the MAX edge");
        assert_eq!(pool_reserve_rows(192), 5_592_405, "192 B class");
        assert_eq!(pool_reserve_rows(256), 4_194_304, "256 B class");
        assert_eq!(pool_reserve_rows(1024), 1_048_576, "1 KiB class");
        assert_eq!(pool_reserve_rows(4096), 262_144, "4 KiB class");
        assert_eq!(pool_reserve_rows(16 * KIB), POOL_MIN_ROWS, "16 KiB: exact MIN edge");
        assert_eq!(pool_reserve_rows(32 * KIB), POOL_MIN_ROWS, "32 KiB: MIN floor binds");
    }

    /// U-P1 — D2 fallback-arm constants (★R1-3, 4 MiB target): 16 B is
    /// EXACTLY the MAX-clamp boundary (2^22 / 2^4 = 2^18 = `POOL_MAX_ROWS`,
    /// today's tiny-class ceiling); 16 KiB is EXACTLY the MIN-clamp
    /// boundary (2^22 / 2^14 = 2^8 = `POOL_MIN_ROWS`).
    #[cfg(not(all(not(miri), any(windows, unix), target_pointer_width = "64")))]
    #[test]
    fn pool_reserve_rows_fallback_arm_table() {
        assert_eq!(pool_reserve_rows(8), POOL_MAX_ROWS, "8 B: row cap binds");
        assert_eq!(pool_reserve_rows(16), POOL_MAX_ROWS, "16 B: exact MAX edge");
        assert_eq!(pool_reserve_rows(32), 131_072, "32 B: pre-X.I small ceiling");
        assert_eq!(pool_reserve_rows(64), 65_536, "64 B: pre-X.I medium ceiling");
        assert_eq!(pool_reserve_rows(192), 21_845, "192 B: documented shrink");
        assert_eq!(pool_reserve_rows(1024), 4_096, "1 KiB: documented shrink");
        assert_eq!(pool_reserve_rows(16 * KIB), POOL_MIN_ROWS, "16 KiB: exact MIN edge");
        assert_eq!(pool_reserve_rows(32 * KIB), POOL_MIN_ROWS, "32 KiB: MIN floor binds");
    }

    /// U-P1 — D1 layout math at `stagger == 0` (the pre-P2-CACHE-FIX
    /// geometry): every sub-region granule-aligned, offsets disjoint and in
    /// order, `os_len = data_len + 2 × tick_len`.
    #[test]
    fn pool_byte_layout_granule_aligned_and_disjoint() {
        // Single-granule pool (the D2-mapped test geometry: 256 × 16 B).
        let l = pool_byte_layout(256, 16, 0);
        assert_eq!(l.data_off, 0, "stagger == 0 ⇒ data starts at the base");
        assert_eq!(l.data_len, G, "4096 B of data rounds up to one granule");
        assert_eq!(l.tick_len, G, "1024 B of ticks rounds up to one granule");
        assert_eq!(l.added_off, G);
        assert_eq!(l.changed_off, 2 * G);
        assert_eq!(l.os_len, 3 * G);

        // Multi-granule pool: 65,536 × 192 B = 12 MiB data (already a
        // granule multiple), 256 KiB ticks.
        let l2 = pool_byte_layout(65_536, 192, 0);
        assert_eq!(l2.data_off, 0);
        assert_eq!(l2.data_len, 12 * MIB);
        assert_eq!(l2.tick_len, 256 * KIB);
        assert_eq!(l2.added_off, l2.data_len);
        assert_eq!(l2.changed_off, l2.data_len + l2.tick_len);
        assert_eq!(l2.os_len, l2.data_len + 2 * l2.tick_len);
        assert!(l2.data_len.is_multiple_of(G) && l2.tick_len.is_multiple_of(G));

        // Non-multiple byte counts round UP (never down).
        let l3 = pool_byte_layout(100, 12, 0);
        assert_eq!(l3.data_len, G, "1200 B rounds up to one granule");
        assert_eq!(l3.tick_len, G, "400 B rounds up to one granule");
        assert!(l3.data_len >= 100 * 12 && l3.tick_len >= 100 * 4);
    }

    /// P2-CACHE-FIX — D1 layout math with a non-zero per-pool stagger: every
    /// offset shifts right by exactly `stagger`, the data base stays
    /// SIMD-aligned, the sub-regions stay disjoint and in-bounds, and `os_len`
    /// is re-rounded UP to a granule (the pad pushes the tail off the boundary
    /// it had at `stagger == 0`).
    #[test]
    fn pool_byte_layout_stagger_shifts_all_subregions() {
        // A representative stagger: component_id 3 ⇒ 3 × 64 = 192 B.
        let stagger = pool_base_stagger(3);
        assert_eq!(stagger, 192, "component_id 3 ⇒ 3 cache lines");
        assert!(stagger.is_multiple_of(SIMD_BUFFER_ALIGN), "stagger SIMD-aligned");

        // Single-granule pool (256 × 16 B) at this stagger.
        let l = pool_byte_layout(256, 16, stagger);
        assert_eq!(l.data_off, stagger, "data starts at the stagger pad");
        assert_eq!(l.data_len, G, "data length is stagger-independent");
        assert_eq!(l.tick_len, G, "tick length is stagger-independent");
        assert_eq!(l.added_off, stagger + G, "added ticks shift by stagger");
        assert_eq!(l.changed_off, stagger + 2 * G, "changed ticks shift by stagger");
        // os_len re-rounds UP: stagger + 3 G is NOT a granule multiple.
        assert_eq!(l.os_len, 4 * G, "os_len rounds the staggered tail up to a granule");
        assert!(l.os_len.is_multiple_of(G), "os_len stays granule-aligned");

        // Disjointness + in-bounds: data ⊂ [stagger, added_off),
        // added ⊂ [added_off, changed_off), changed ⊂ [changed_off, os_len).
        assert!(l.data_off + l.data_len <= l.added_off, "data before added ticks");
        assert!(l.added_off + l.tick_len <= l.changed_off, "added before changed");
        assert!(l.changed_off + l.tick_len <= l.os_len, "changed within reservation");

        // The maximal stagger (component_id 63 ⇒ 63 × 64 = 4032 B) is still
        // strictly below one page and SIMD-aligned.
        let max_stagger = pool_base_stagger(63);
        assert_eq!(max_stagger, 4032);
        assert!(max_stagger < 4096 && max_stagger.is_multiple_of(SIMD_BUFFER_ALIGN));
        // component_id 64 wraps back to 0 (mod 64).
        assert_eq!(pool_base_stagger(64), 0, "stagger wraps mod 64");
    }

    /// Phase 22 D6 — `stride == 0` (ZST/tag pool) layout math: the data
    /// sub-region is vacuous and the reservation degenerates to exactly the
    /// two tick regions.
    #[test]
    fn pool_byte_layout_zst_vacuous_data_region() {
        // The `align_up(0) == 0` pin — the keystone that collapses the data
        // region: `data_len = align_up(reserve_rows × 0) = align_up(0) = 0`.
        assert_eq!(
            pool_align_up_granule(0),
            0,
            "align_up(0) must be 0 (vacuous data region keystone)"
        );

        // Small geometry at stagger == 0: 256 rows of 4 B ticks round up to
        // one granule each.
        let l = pool_byte_layout(256, 0, 0);
        assert_eq!(l.data_off, 0, "stagger == 0 ⇒ data starts at the base");
        assert_eq!(l.data_len, 0, "ZST pool has no data bytes");
        assert_eq!(l.added_off, 0, "added ticks start at the reservation base");
        assert_eq!(l.tick_len, G, "256 × 4 B rounds up to one granule");
        assert_eq!(l.changed_off, l.tick_len, "changed region follows added directly");
        assert_eq!(l.os_len, 2 * l.tick_len, "overall span is exactly 2 × tick_len");

        // Ceiling geometry: the D6 VA-budget shape (2 × tick_len at
        // POOL_MAX_ROWS — 128 MiB on the syscall arms, 2 MiB on the fallback).
        let l2 = pool_byte_layout(POOL_MAX_ROWS, 0, 0);
        assert_eq!(l2.data_len, 0, "vacuous data region at the row ceiling");
        assert_eq!(l2.added_off, 0);
        assert_eq!(l2.tick_len, pool_align_up_granule(POOL_MAX_ROWS * 4));
        assert_eq!(l2.changed_off, l2.tick_len);
        assert_eq!(l2.os_len, 2 * l2.tick_len, "span == 2 × tick_len at the ceiling");
        assert!(l2.tick_len.is_multiple_of(G), "tick region stays granule-aligned");

        // P2-CACHE-FIX: a staggered ZST pool shifts BOTH tick regions right
        // by `stagger` and re-rounds the tail up to a granule. The two tick
        // regions stay disjoint (data-vs-tick is vacuous, tick-vs-tick holds).
        let stagger = pool_base_stagger(5); // 5 × 64 = 320 B
        let l3 = pool_byte_layout(256, 0, stagger);
        assert_eq!(l3.data_off, stagger);
        assert_eq!(l3.data_len, 0, "ZST pool still has no data bytes");
        assert_eq!(l3.added_off, stagger, "added ticks start at the stagger pad");
        assert_eq!(l3.changed_off, stagger + l3.tick_len, "changed follows added");
        assert_eq!(l3.os_len, 3 * G, "stagger + 2 G rounds up to one extra granule");
        assert!(l3.os_len.is_multiple_of(G));
        assert!(l3.added_off + l3.tick_len <= l3.changed_off, "tick-tick disjoint");
        assert!(l3.changed_off + l3.tick_len <= l3.os_len, "changed in-bounds");
    }

    /// Phase 22 D6 — the `pool_reserve_rows` ZST arm routes to
    /// `POOL_MAX_ROWS`: rows are bounded by the tick sub-regions only, the
    /// same ceiling a 1-byte component hits.
    #[test]
    fn pool_reserve_rows_zst_routes_to_max_rows() {
        assert_eq!(
            pool_reserve_rows(0),
            POOL_MAX_ROWS,
            "stride 0: tick-bounded row ceiling"
        );
    }

    /// U-P1 — D4 step policy table: MIN floor, in-band doubling, MAX clamp,
    /// request-dominant, and the saturating belt.
    #[test]
    fn pool_commit_step_policy_table() {
        // Fresh pool (data_committed = 0): the MIN_SLAB floor.
        assert_eq!(pool_commit_step(0, G), POOL_MIN_SLAB, "first grow = one granule");
        // In-band doubling: step equals the committed size.
        assert_eq!(pool_commit_step(4 * G, 5 * G), 4 * G, "doubling inside the band");
        assert_eq!(pool_commit_step(8 * MIB, 8 * MIB + G), 8 * MIB, "doubling at 8 MiB");
        // MAX clamp: doubling never exceeds POOL_MAX_SLAB.
        assert_eq!(
            pool_commit_step(128 * MIB, 128 * MIB + G),
            POOL_MAX_SLAB,
            "doubling clamps to MAX_SLAB"
        );
        // Request-dominant: a large request overrides the doubling step.
        assert_eq!(
            pool_commit_step(G, 100 * G),
            99 * G,
            "request larger than the doubling step wins"
        );
        assert_eq!(
            pool_commit_step(128 * MIB, 256 * MIB),
            128 * MIB,
            "request larger than MAX_SLAB is NOT clamped (one request = one event)"
        );
        // Saturating belt: a satisfied request (never reached on the real
        // path — grow_rows early-outs) degrades to the doubling step.
        assert_eq!(pool_commit_step(2 * G, G), 2 * G, "saturating_sub belt");
    }

    /// U-P6 — Tick::ZERO transmute pin (the X.G U-S1 pattern): demand-zero
    /// pages ARE valid `Tick::ZERO` slots. `Tick` is `repr(transparent)`
    /// over `u32` (4 B, no padding, every bit pattern valid), and
    /// `UnsafeCell` adds no layout on top.
    #[test]
    fn tick_zero_is_all_zero_bytes() {
        // SAFETY: `Tick` is `repr(transparent)` over `u32` — exactly 4 value
        // bytes, no padding; transmuting to a byte array reads all of them.
        let bytes: [u8; 4] = unsafe { std::mem::transmute(Tick::ZERO) };
        assert_eq!(bytes, [0u8; 4], "Tick::ZERO must be all-zero bytes (J-XI keystone)");
        assert_eq!(size_of::<UnsafeCell<Tick>>(), 4, "tick slot stride pin");
        assert_eq!(align_of::<UnsafeCell<Tick>>(), 4, "tick slot align pin");
    }
}