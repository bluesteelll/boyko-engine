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
/// (see [`pool_reserve_rows`]). 1 GiB on the 64-bit OS-syscall arms —
/// virtual address space only (no commit charge until `grow_rows` commits
/// at the frontier), aligned with the Phase X.G 67 M-entity inland ceiling.
///
/// VA budget: 3000 pools (1000 archetypes × 3 pools) reserve ≤ ~3.4 TiB =
/// 2.7% of the 128 TB user VA on Windows/Linux alike. OS-knob note: each
/// pool contributes ≤ 6 VMAs/VADs (3 sub-regions × committed prefix +
/// `PROT_NONE` tail) ⇒ ≤ 18,000 at 3000 pools vs the Linux default
/// `vm.max_map_count` of 65,530 (3.6× headroom) — the one OS limit a
/// pathological embedder could approach.
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
/// [`pool_commit_step`] always covers it).
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
/// `ComponentPool::with_default_sizes` ONLY — the legacy explicit-ceiling
/// constructor `ComponentPool::new` bypasses the clamp by design (★R1-9).
pub(crate) const fn pool_reserve_rows(stride: usize) -> usize {
    assert!(
        stride > 0,
        "pool_reserve_rows: zero-sized components are not supported"
    );
    let by_bytes = POOL_TARGET_DATA_BYTES / stride;
    if by_bytes < POOL_MIN_ROWS {
        POOL_MIN_ROWS
    } else if by_bytes > POOL_MAX_ROWS {
        POOL_MAX_ROWS
    } else {
        by_bytes
    }
}

/// Byte layout of a pool's single reservation (Phase X.I D1):
/// `[data | added_ticks | changed_ticks]`, every sub-region granule-aligned.
///
/// ```text
/// data_off    = 0
/// data_len    = align_up(reserve_rows × stride, G)
/// added_off   = data_len;            tick_len = align_up(reserve_rows × 4, G)
/// changed_off = data_len + tick_len
/// os_len      = data_len + 2 × tick_len
/// ```
///
/// The `4` is `size_of::<UnsafeCell<Tick>>()` — pinned by a const assert in
/// `component_pool.rs` and the U-P6 transmute test.
pub(crate) struct PoolByteLayout {
    /// Granule-aligned data sub-region length (also `added_off`).
    pub(crate) data_len: usize,
    /// Granule-aligned length of EACH tick sub-region.
    pub(crate) tick_len: usize,
    /// Offset of the `added` tick sub-region (== `data_len`).
    pub(crate) added_off: usize,
    /// Offset of the `changed` tick sub-region (== `data_len + tick_len`).
    pub(crate) changed_off: usize,
    /// Total reservation length (`data_len + 2 × tick_len`).
    pub(crate) os_len: usize,
}

/// Computes the D1 layout with checked arithmetic (overflow panics loudly).
pub(crate) const fn pool_byte_layout(reserve_rows: usize, stride: usize) -> PoolByteLayout {
    // Belt asserts — `ComponentPool::new` fires the loud constructor-naming
    // asserts (★R1-5) before reaching this math.
    assert!(reserve_rows > 0, "pool_byte_layout: reserve_rows must be non-zero");
    assert!(
        stride > 0,
        "pool_byte_layout: zero-sized components are not supported"
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

    let added_off = data_len;
    let changed_off = match data_len.checked_add(tick_len) {
        Some(v) => v,
        None => panic!("pool_byte_layout: data_len + tick_len overflows usize"),
    };
    let os_len = match changed_off.checked_add(tick_len) {
        Some(v) => v,
        None => panic!("pool_byte_layout: os_len overflows usize"),
    };

    PoolByteLayout {
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
/// data_committed` before calling (GROW1-XI corollary 0a), so the sub never
/// actually saturates on the real path.
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

    /// U-P1 — D1 layout math: every sub-region granule-aligned, offsets
    /// disjoint and in order, `os_len = data_len + 2 × tick_len`.
    #[test]
    fn pool_byte_layout_granule_aligned_and_disjoint() {
        // Single-granule pool (the D2-mapped test geometry: 256 × 16 B).
        let l = pool_byte_layout(256, 16);
        assert_eq!(l.data_len, G, "4096 B of data rounds up to one granule");
        assert_eq!(l.tick_len, G, "1024 B of ticks rounds up to one granule");
        assert_eq!(l.added_off, G);
        assert_eq!(l.changed_off, 2 * G);
        assert_eq!(l.os_len, 3 * G);

        // Multi-granule pool: 65,536 × 192 B = 12 MiB data (already a
        // granule multiple), 256 KiB ticks.
        let l2 = pool_byte_layout(65_536, 192);
        assert_eq!(l2.data_len, 12 * MIB);
        assert_eq!(l2.tick_len, 256 * KIB);
        assert_eq!(l2.added_off, l2.data_len);
        assert_eq!(l2.changed_off, l2.data_len + l2.tick_len);
        assert_eq!(l2.os_len, l2.data_len + 2 * l2.tick_len);
        assert!(l2.data_len.is_multiple_of(G) && l2.tick_len.is_multiple_of(G));

        // Non-multiple byte counts round UP (never down).
        let l3 = pool_byte_layout(100, 12);
        assert_eq!(l3.data_len, G, "1200 B rounds up to one granule");
        assert_eq!(l3.tick_len, G, "400 B rounds up to one granule");
        assert!(l3.data_len >= 100 * 12 && l3.tick_len >= 100 * 4);
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