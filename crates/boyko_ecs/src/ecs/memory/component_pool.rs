use std::alloc::Layout;
use std::any::TypeId;
use std::cell::UnsafeCell;
use std::ptr::NonNull;

use crate::ecs::constants::{
    COMMIT_PAGE, SIMD_BUFFER_ALIGN, pool_align_up_page, pool_base_stagger, pool_byte_layout,
    pool_commit_step, pool_reserve_rows,
};
use crate::ecs::core::change_detection::Tick;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry::{self, DropFn};
#[cfg(not(miri))]
use crate::ecs::memory::device_column::DeviceColumn;
use crate::ecs::memory::vm::VmReservation;

// Phase X.I D1: the tick sub-regions are sized at `reserve_rows * 4` bytes —
// pinned here so the layout math in `constants::pool_byte_layout` (which uses
// the literal 4) can never drift from the real slot type.
const _: () = assert!(std::mem::size_of::<UnsafeCell<Tick>>() == 4);
const _: () = assert!(std::mem::align_of::<UnsafeCell<Tick>>() == 4);

/// `ticks_committed` sentinel for an UNTRACKED pool — one whose tick
/// sub-regions are reserved but will never be committed
/// ([`ComponentPool::new_untracked`]).
///
/// # Why a sentinel in the existing field rather than a new flag
///
/// `ComponentPool` is size-pinned at 128 B by a const assert below, with no
/// spare byte for a `bool`. A sentinel costs zero bytes AND zero branches:
/// `grow_rows`'s tick-commit guard is already `if t_new > self.ticks_committed`,
/// and `t_new` is bounded by the tick cap `align_up_page(stagger + tick_len)`
/// (itself inside the reservation), so `usize::MAX` makes that guard uniformly
/// false without a single new `if` on the cold path — and without touching the
/// layout, which is what keeps the `debug_assert!(t_new <= tick_cap)` proof
/// step above it TRUE (packing plan obligation 3).
///
/// ⚠ The alternative — zeroing `tick_len` in the layout — is UNSOUND and was
/// rejected with its mechanism: that assert fires BEFORE the commit guard, and
/// with `tick_len == 0` its bound collapses to `align_up_page(stagger)` — 0 at
/// `stagger == 0`, one page otherwise — so `t_new = align_up_page(stagger +
/// rows * 4)` trips it at the first grow of every unstaggered pool and as soon
/// as any other pool outgrows one tick page, on every debug build and every
/// Miri run. Reserved-uncommitted is also the form the approved remediation
/// prescribes verbatim (`docs/ARCH-AUDIT-ECS-DATA-REMEDIATION.md`, "tick
/// sub-regions reserved-uncommitted").
pub(crate) const UNTRACKED_TICKS: usize = usize::MAX;

// Phase 4 Seam 3 (IM-1 / IM-6): the `vm: VmReservation` -> `backing:
// PoolBacking` swap must add ZERO bytes. `PoolBacking::Device(Box<DeviceColumn>)`
// is 8 B (a single `Box`), <= `VmReservation`'s 16 B (host) — so on the host
// build `PoolBacking` stays 16 B (same as `vm`), and `ComponentPool` stays at
// its pre-Phase-4 128 B. Under `#[cfg(miri)]` the `Device` arm is compiled out,
// so `PoolBacking` is a single-variant enum niche-optimized to `VmReservation`'s
// exact layout (24 B + the fallback `Layout` field = 32 B for the field), and
// `ComponentPool` stays at its pre-Phase-4 144 B (P7). Two pins because the
// fallback `VmReservation` carries an extra `Layout` field.
#[cfg(not(miri))]
const _: () = assert!(
    std::mem::size_of::<ComponentPool>() == 128,
    "Phase 4 IM-1: the vm->backing swap must NOT grow ComponentPool (host = 128 B)"
);
#[cfg(miri)]
const _: () = assert!(
    std::mem::size_of::<ComponentPool>() == 144,
    "Phase 4 IM-1: the vm->backing swap must NOT grow ComponentPool (miri = 144 B)"
);

/// Phase 4 Seam 3 (D3, CR-C) — where a [`ComponentPool`]'s rows physically live.
///
/// The Host arm wraps the pre-Phase-4 [`VmReservation`] verbatim (one
/// virtual-address reservation, lazy-committed); the Device arm wraps a boxed
/// [`DeviceColumn`] (a graphics-pure handle + device-side row counters). The
/// three write-once base pointers (`buffer` / `added_base` / `changed_base`)
/// stay TOP-LEVEL `ComponentPool` fields derived from the Host arm in `new`, so
/// the hot [`ComponentPool::row_ptr`] reads `self.buffer` and NEVER matches on
/// `backing` — byte-identical codegen (the 0%-gate, D4).
///
/// `Device` is `#[cfg(not(miri))]` (Phase 4 mints no device pool; Miri cannot
/// run the RHI syscalls), so under Miri this is a single-variant enum
/// niche-optimized to `VmReservation`'s exact layout (P7). The Device payload is
/// boxed (8 B ≤ the 16-B Host arm) so the enum stays ≤ 16 B (IM-1).
pub(crate) enum PoolBacking {
    /// Host-memory backing: the pool's own virtual-address reservation. The
    /// ONLY arm `new` constructs in Phase 4; its `Drop` releases the
    /// reservation (declared last in `ComponentPool`, same slot as the old
    /// `vm`).
    Host(VmReservation),
    /// Device-memory backing (Phase 5 fill). Boxed so `PoolBacking` stays
    /// ≤ 16 B; Phase 4 never constructs it (the residency table is empty until a
    /// GPU component registers, and no production path mints a device pool), so
    /// `#[allow(dead_code)]` until Phase 5 wires the RHI mint. Exercised in tests
    /// via `ComponentPool::make_device_backed_for_test`.
    #[cfg(not(miri))]
    #[allow(dead_code)]
    Device(Box<DeviceColumn>),
}

impl PoolBacking {
    /// Returns `true` iff this is the Device arm (Phase 4: always `false` — no
    /// device pool is minted). Used by the `Drop` CR-C debug-assert.
    #[inline]
    pub(crate) fn is_device(&self) -> bool {
        match self {
            PoolBacking::Host(_) => false,
            #[cfg(not(miri))]
            PoolBacking::Device(_) => true,
        }
    }

    /// Returns `&mut VmReservation` for the Host arm — the grow funnels' commit
    /// accessor (IM-3).
    ///
    /// # Panics
    ///
    /// `unreachable!` on the Device arm: Phase 4 mints no device pool, and growth
    /// is Host-only. Phase 5 replaces the Device arm of `grow_rows` with the RHI
    /// realloc+copy+fence path BEFORE any device pool can exist, so this is
    /// genuinely unreachable until then.
    #[cfg(not(miri))]
    #[inline]
    pub(crate) fn host_vm_mut(&mut self) -> &mut VmReservation {
        match self {
            PoolBacking::Host(vm) => vm,
            PoolBacking::Device(_) => Self::grow_is_host_only(),
        }
    }

    /// Single-variant Miri build: the Device arm is compiled out, so this is an
    /// irrefutable bind with no panic arm.
    #[cfg(miri)]
    #[inline]
    pub(crate) fn host_vm_mut(&mut self) -> &mut VmReservation {
        let PoolBacking::Host(vm) = self;
        vm
    }

    /// The cold, never-taken "growth is Host-only in Phase 4" reject arm,
    /// factored out so `host_vm_mut` stays a one-line match (IM-3).
    #[cfg(not(miri))]
    #[cold]
    #[inline(never)]
    fn grow_is_host_only() -> ! {
        unreachable!("Phase 4 mints no Device pool; ComponentPool grow is Host-only")
    }
}

/// Pool of components of a specific type, stored as a dense byte buffer.
///
/// Components live contiguously in `buffer`: row `i` starts at
/// `buffer + i * component_layout.size()`. The rows `[0, self.len)` are fully
/// initialized; rows `[len, committed_rows)` are committed-but-uninitialized
/// and must never be read or dropped; rows `[committed_rows, reserve_rows)`
/// are reserved address space only (`PROT_NONE` on the syscall arms — a
/// stray touch faults loudly). The row pointer is recomputed on demand via
/// `ComponentPool::row_ptr` rather than cached per-row (Phase X.B).
///
/// # Phase X.I — one `VmReservation` per pool, in-place row growth
///
/// Each pool owns ONE virtual-address reservation laid out
/// `[data | added_ticks | changed_ticks]` with granule-aligned, fixed
/// sub-region offsets computed once at construction
/// (`constants::pool_byte_layout`). Growth (`ComponentPool::grow_rows`)
/// only commits fresh pages at the frontier of the SAME reservation — the
/// three base pointers are write-once, so every previously returned pointer
/// (incl. `Archetype::columns[c].ptr` and the query fetches' tick bases)
/// stays valid for the pool's lifetime. Growth is O(1) in live rows: no
/// bytes are copied, ever.
//
// Not `#[repr(C)]`-pinned (no external offset contract; the hot READ paths
// never load pool fields — Phase X.I D10). Field ORDER groups the warm trio
// (`buffer`, `len`, `committed_rows`).
pub struct ComponentPool {
    /// Data sub-region base; WRITE-ONCE (invariant U6 twin).
    ///
    /// `stride > 0`: `== vm.base() + pool_base_stagger(component_id)` — the
    /// P2-CACHE-FIX leading pad shifts the data sub-region off the bare
    /// reservation base so different columns land in different cache sets.
    /// Phase 22 D1 (`stride == 0`, tag pools): a dangling, provenance-free
    /// pointer at address `SIMD_BUFFER_ALIGN.max(align)` — non-null,
    /// SIMD-A1-aligned, valid ONLY for zero-size access (the data sub-region is
    /// vacuous).
    buffer: NonNull<u8>,

    /// Live row count; rows `[0, len)` are initialized and densely packed.
    /// THE liveness oracle — nothing is ever read at or above it.
    len: usize,

    /// Warm-path capacity comparator: rows `[0, committed_rows)` are
    /// committed read/write. The single compare in `add`/`add_typed`.
    committed_rows: usize,

    /// The reserve ceiling (== [`ComponentPool::capacity`]); immutable
    /// after `new`.
    reserve_rows: usize,

    /// Component layout (cached from registry for performance).
    component_layout: Layout,

    /// Committed bytes of the data sub-region, measured from its absolute
    /// PAGE FLOOR (reservation offset 0), so it includes the stagger pad; a
    /// `COMMIT_PAGE` multiple, monotonic (cold-path bookkeeping for
    /// `grow_rows`, packing plan D2).
    data_committed: usize,

    /// Committed bytes of EACH tick sub-region, measured from that
    /// sub-region's absolute page floor (`added_off - stagger`,
    /// `changed_off - stagger`); a `COMMIT_PAGE` multiple, monotonic.
    ///
    /// [`UNTRACKED_TICKS`] is a SENTINEL, not a byte count: it marks a pool
    /// built by [`ComponentPool::new_untracked`], whose tick sub-regions stay
    /// RESERVED-BUT-NEVER-COMMITTED. The sentinel needs no branch in
    /// `grow_rows` — its commit guard is `t_new > self.ticks_committed`, and
    /// nothing exceeds `usize::MAX`, so the two tick commits are skipped by the
    /// arithmetic that is already there.
    ticks_committed: usize,

    /// `added` tick sub-region base (`vm.base() + stagger + data_len`);
    /// WRITE-ONCE.
    ///
    /// `UnsafeCell<Tick>` is `repr(transparent)` over a 4-byte `u32` whose
    /// every bit pattern is valid, so demand-zero pages read as
    /// [`Tick::ZERO`] (J-XI, ★R1-4 never-written form). It provides interior
    /// mutability through a shared `&self` (used by `Added<C>::filter_fetch`
    /// reads through the `Fetch<'w>` pointer) while still permitting the
    /// Phase 9 scheduler to declare exclusive write access on a
    /// per-`(archetype, component)` basis (SCH3). Adjacent-row writes from
    /// sibling `par_iter` chunks target distinct memory locations — sound
    /// per Rust's abstract machine even though they share a cache line
    /// (Round 2 C3).
    added_base: NonNull<UnsafeCell<Tick>>,

    /// `changed` tick sub-region base
    /// (`vm.base() + stagger + data_len + tick_len`); WRITE-ONCE. Same shape
    /// and discipline as [`Self::added_base`].
    /// Updated by `Mut<T>::deref_mut` and `EcsMaster::set_component_raw`.
    changed_base: NonNull<UnsafeCell<Tick>>,

    /// Component ID — used to look up layout information.
    component_id: usize,

    /// Cached drop_fn for the component type (`None` when `!needs_drop`).
    /// Read on every swap_remove / pop / set_component / Drop.
    drop_fn: Option<DropFn>,

    /// Cached TypeId for debug-only typed-API validation.
    component_type_id: TypeId,

    /// The pool's backing storage (Phase 4 Seam 3 — was `vm: VmReservation`).
    /// Declared LAST: `Drop::drop`'s body (the `drop_fn` loop over rows
    /// `[0, len)`) runs before field drops, so the backing is released strictly
    /// after its last use (release-after-use; M-001 per-arm deallocator carried
    /// by `VmReservation` inside the `Host` arm, V-DROP releases partially
    /// committed reservations in full). For a Device pool (Phase 5) the Box's
    /// drop releases the device column; per CR-C `len == 0` so the `drop_fn`
    /// loop is a no-op.
    backing: PoolBacking,
}

impl ComponentPool {
    /// Creates a new component pool with an EXPLICIT row ceiling.
    ///
    /// # Phase X.I D2 mapping (★R1-9 — binding)
    ///
    /// This explicit-ceiling constructor uses `reserve_rows` EXACTLY — it
    /// deliberately BYPASSES the `POOL_MIN_ROWS`/`POOL_MAX_ROWS` clamp. The
    /// entire pin-test ledger (drop_fn `cap`-row pools, the in-file 256-row
    /// proptests, the dense bench, the X.B identity tests) depends on exact
    /// small ceilings; routing this constructor through the clamp would be
    /// a ledger-wide breakage. (Phase X.J collapsed the historical
    /// `num_chunks × components_per_chunk` parameter pair — `reserve_rows
    /// = n × m` — into this single parameter.)
    ///
    /// Construction performs ONE address-space reservation (no commit
    /// charge, zero resident bytes) and computes the three write-once base
    /// pointers; the first `add`/`reserve_capacity` takes the cold
    /// `grow_rows` path (Phase X.I D3).
    ///
    /// # Panics
    ///
    /// * `reserve_rows == 0` (★R1-5) — the ceiling must be non-zero.
    /// * `component_layout.align() > 4096` — every arm's reservation base
    ///   is only guaranteed 4096-aligned.
    /// * `reserve_rows * stride` overflowing `usize`.
    /// * OS reservation failure (unrecoverable misconfiguration).
    pub fn new(component_id: usize, reserve_rows: usize) -> Self {
        debug_assert!(component_id < 512, "Component ID exceeds maximum allowed");

        // SAFETY: component_id was checked above; caller must have registered
        // the component before constructing a pool (invariant of ComponentPool::new).
        let registry_layout =
            unsafe { component_registry::get_layout_unchecked(component_id) };
        let component_layout = registry_layout.layout();
        let drop_fn = registry_layout.drop_fn;
        let component_type_id = registry_layout.type_id;

        // ★R1-5: loud pool-level assert BEFORE the vm reservation, so a zero
        // ceiling names this constructor instead of panicking inside
        // `VmReservation::reserve(0)` with a vm-internals message.
        assert!(
            reserve_rows > 0,
            "ComponentPool::new: reserve_rows == 0 (the row ceiling must be non-zero)"
        );
        // ★R1-9 ceiling guard: this constructor bypasses the POOL_MAX_ROWS
        // clamp, so it must enforce the row-index representability bound
        // itself — `EntityInland.unit_index` is a `u32` (`archetype.rs`
        // stores `row as u32`; the migration helpers cast likewise), and a
        // ceiling above it would alias wrong rows through the safe API.
        // Strict `<` for symmetry with the `POOL_MAX_ROWS < u32::MAX` const
        // assert in constants.rs: one bound form for both constructors.
        assert!(
            reserve_rows < u32::MAX as usize,
            "ComponentPool::new: reserve_rows = {reserve_rows} exceeds the \
             `EntityInland.unit_index: u32` row-index ceiling \
             (must be < u32::MAX, matching the POOL_MAX_ROWS const assert)"
        );

        // Phase X.I D10: every arm's reservation base is >= 4096-aligned
        // (VirtualAlloc 64 KiB / mmap 4 KiB / fallback `Layout` align 4096 —
        // vm.rs), strictly wider than the old arena bound of 64. Component
        // types aligned beyond a page are unsupported — loud, not silent.
        let element_align = component_layout.align();
        assert!(
            element_align <= 4096,
            "ComponentPool::new: component alignment {element_align} exceeds the \
             4096-byte reservation-base guarantee"
        );

        // P2-CACHE-FIX: stagger this pool's in-reservation base by a
        // component_id-derived, cache-line-multiple leading offset so different
        // columns' element-`i` rows land in different L1/L2 cache sets. Without
        // it every pool's 64 KiB-aligned reservation base puts element `i` of
        // every column in the SAME cache set, so a SoA hot loop sweeping ~24
        // columns (the rigid solver) fights for one 8-way set — a conflict-miss
        // storm that cost the ~40% rigid-solver regression at P2.
        let stagger = pool_base_stagger(component_id);

        // D1 layout: [pad | data | added_ticks | changed_ticks], every
        // sub-region granule-aligned, shifted right by the per-pool stagger;
        // checked arithmetic panics loudly on overflow.
        let stride = component_layout.size();
        let layout = pool_byte_layout(reserve_rows, stride, stagger);

        // D3: eager reserve, ZERO initial commit. `reserve` (zeroed
        // contract — NOT `reserve_unzeroed`): the tick sub-regions rely on
        // never-written-reads-zero (J-XI below), and the fallback arm
        // models the OS zero-fill with `alloc_zeroed`.
        let vm = VmReservation::reserve(layout.os_len);
        // Phase 22 O1 ordering (binding): `base` is the LIVE reservation
        // base; every derived pointer below (the tick bases in particular)
        // MUST come from it. `buffer` is set per-arm LAST, because the
        // ZST arm's buffer is a dangling pointer — deriving tick bases from
        // it would be UB at the first `fill_ticks`.
        let base = vm.base();

        // Phase X.A SIMD-A1 invariant (plan §6.4): the DATA base MUST be
        // SIMD_BUFFER_ALIGN-aligned so callers (`buffer_ptr`,
        // `Query::for_each_chunk` inner loops) can rely on it without
        // re-checking. Post-P2-CACHE-FIX the data base is `reservation_base +
        // stagger` (not the bare reservation base): the >= 4096-aligned base
        // plus a `CACHE_LINE_SIZE`-multiple (hence SIMD_BUFFER_ALIGN-multiple,
        // const-asserted on `POOL_STAGGER_LINES`) stagger stays SIMD-aligned.
        // SAFETY: `layout.data_off == stagger < os_len`, in-bounds of the
        // single reservation, so the `add` stays inside one allocated object.
        let data_base = unsafe { base.add(layout.data_off) };
        debug_assert!(
            (data_base.as_ptr() as usize).is_multiple_of(SIMD_BUFFER_ALIGN),
            "SIMD-A1: ComponentPool data base {:p} (reservation base + stagger {}) is not \
             SIMD_BUFFER_ALIGN={}-aligned",
            data_base.as_ptr(),
            stagger,
            SIMD_BUFFER_ALIGN
        );

        // SAFETY (S-TICKBASE): both offsets are in-bounds of the single
        // reservation (`stagger <= added_off < changed_off < os_len` by the D1
        // layout math, all checked; `os_len <= isize::MAX` asserted by
        // `reserve`), so each `add` stays inside the one allocated object —
        // ★R1-8: the data region and BOTH tick regions are ONE allocated
        // object, the pool's own reservation. P2-CACHE-FIX: every offset
        // (`added_off`/`changed_off`) already includes the leading `stagger`
        // pad (`pool_byte_layout` shifts all three sub-regions right by it),
        // so the tick bases derive correctly from the bare reservation `base`.
        // Phase 22 ZST arm: for `stride == 0` the data sub-region is empty
        // (`added_off == stagger`, `changed_off == stagger + tick_len`);
        // `buffer` (set below) is a dangling aligned pointer valid only for
        // zero-size access per the Rust reference — both tick bases derive from
        // `base` (the single LIVE reservation, O1), and tick-tick disjointness
        // is unchanged. Alignment: granule(64 KiB)-aligned tick offsets plus a
        // `CACHE_LINE_SIZE`-multiple stagger, from a >= 4096-aligned base,
        // yield alignment >= CACHE_LINE_SIZE = 64 >= 4 =
        // align_of::<UnsafeCell<Tick>> (const-asserted at the top of this
        // file). The bases are derived once and never reassigned (write-once);
        // the reservation ADDRESS is stable for the pool's lifetime, so they
        // remain valid after `vm` moves into the struct below.
        let (added_base, changed_base) = unsafe {
            (
                base.add(layout.added_off).cast::<UnsafeCell<Tick>>(),
                base.add(layout.changed_off).cast::<UnsafeCell<Tick>>(),
            )
        };

        // O1: `buffer` per-arm, LAST — after every live-reservation-derived
        // pointer is already bound. P2-CACHE-FIX: the stride > 0 data base is
        // `data_base == base + stagger` (computed above for the SIMD-A1 check),
        // NOT the bare reservation base.
        let buffer = if stride > 0 {
            data_base
        } else {
            // Phase 22 D1/D6: a ZST pool stores no data bytes. The buffer
            // is a dangling, provenance-free pointer at address
            // `SIMD_BUFFER_ALIGN.max(element_align)`: non-null, aligned for
            // the component type AND a multiple of SIMD_BUFFER_ALIGN (the
            // max of two powers of two is a multiple of both; element_align
            // <= 4096 asserted above), so the `buffer_ptr()` /
            // `for_each_chunk` SIMD-A1 alignment contract holds unchanged.
            // Used ONLY for zero-size access (reads/writes/drops of 0 bytes).
            let addr = SIMD_BUFFER_ALIGN.max(element_align);
            // SIMD-A1 belt: the dangling base must satisfy the same
            // alignment contract the live base does.
            debug_assert!(
                addr.is_multiple_of(SIMD_BUFFER_ALIGN),
                "SIMD-A1 belt: ZST dangling buffer address {addr:#x} is not \
                 SIMD_BUFFER_ALIGN={SIMD_BUFFER_ALIGN}-aligned"
            );
            NonNull::new(std::ptr::without_provenance_mut::<u8>(addr))
                .expect("invariant: SIMD_BUFFER_ALIGN.max(align) >= 32 is non-zero")
        };

        // Phase 10 STORE10, re-worded to the Phase X.I J-XI never-written
        // form (★R1-4): every NEVER-WRITTEN tick slot in [0, committed_rows)
        // reads `Tick::ZERO` (vm zero-fill contract; pinned by the U-P6
        // transmute test). Slots VACATED by pop/swap_remove MAY hold a stale
        // live tick — fine: nothing reads at or above `len` (`check_ticks`
        // scans `[0, count())`; fetches index below `entity_count`), and
        // every re-add re-stamps before any read (`fill_ticks` /
        // `write_*_tick` cover `[0, len)`). Do NOT assert all-zero-above-len
        // — it is FALSE after any churn. Write-before-read is the
        // load-bearing property; J-XI is the belt.
        Self {
            buffer,
            len: 0,
            committed_rows: 0,
            reserve_rows,
            component_layout,
            data_committed: 0,
            ticks_committed: 0,
            added_base,
            changed_base,
            component_id,
            drop_fn,
            component_type_id,
            // Phase 4 Seam 3: `new` ALWAYS constructs the Host arm (Phase 4
            // mints no device pool). The three base pointers above were derived
            // from `vm.base()` before this move, so wrapping it here changes
            // nothing about `row_ptr` (which reads `self.buffer`, never
            // `self.backing`).
            backing: PoolBacking::Host(vm),
        }
    }

    /// Creates a pool whose tick sub-regions are RESERVED BUT NEVER COMMITTED —
    /// for raw scratch that carries no change detection.
    ///
    /// Identical to [`new`](Self::new) in layout, in base pointers, and in every
    /// data-path byte; the only difference is that `grow_rows` skips the two
    /// tick commits, so the pool's resident cost is its data region alone.
    ///
    /// # Why this exists
    ///
    /// A `ComponentPool` reservation is `[pad | data | added_ticks |
    /// changed_ticks]` and `grow_rows` commits ALL THREE, each on its own
    /// [`COMMIT_PAGE`](crate::ecs::constants::COMMIT_PAGE) ladder. Two tick
    /// regions at 4 B/row is **8 B of change detection per row on top of the
    /// datum**, so a tracked column costs 3.0x a plain `Vec<u32>`, 2.0x a
    /// `Vec<u64>` and 9.0x a `Vec<bool>` in commit charge — and the first
    /// non-empty grow makes three pages (12 KiB) resident per tracked column
    /// regardless of row count, where an untracked one makes one (4 KiB).
    ///
    /// [`ScratchColumn`] declares it never reads a tick ("There is NO
    /// change-detection tick use — this is raw scratch"), and that claim is
    /// structural rather than aspirational: its entire pool surface is
    /// `push_copy` / `clear_no_drop` / `buffer_ptr` / `buffer_ptr_mut` /
    /// `count` / `capacity` / `component_layout`, and none of those reaches tick
    /// memory. So every tick page such a column commits is paid for and never
    /// read.
    ///
    /// # Contract — the caller must never read or write a tick
    ///
    /// The tick sub-regions of an untracked pool are RESERVED address space with
    /// no backing pages. Touching one is not a logic error that returns a wrong
    /// answer; it is a hard fault. Every tick accessor on this type therefore
    /// carries a `debug_assert!(self.is_tracked())` naming this constructor, so
    /// the misuse is loud in debug and under Miri rather than only in
    /// production. `new_untracked` is for storage whose element type carries its
    /// own liveness (`ScratchColumn`'s refill-every-step discipline), never for
    /// a pool that backs an archetype column.
    ///
    /// # Panics
    /// * everything [`new`](Self::new) panics on, and
    /// * `stride == 0` — a ZST pool's row capacity is TICK-DRIVEN
    ///   (`grow_rows_zst`: `data_committed` is invariantly 0 and the tick
    ///   regions alone bound `committed_rows`), so an untracked ZST pool could
    ///   never grow past zero rows. Refused loudly instead of silently
    ///   capping at 0.
    ///
    /// [`ScratchColumn`]: crate::ecs::core::component::scratch::ScratchColumn
    pub(crate) fn new_untracked(component_id: usize, reserve_rows: usize) -> Self {
        let mut pool = Self::new(component_id, reserve_rows);

        // Phase 22 D6 interaction: the ZST arm derives row capacity from the
        // tick sub-regions, which this constructor refuses to commit. Asserted
        // AFTER `new` so the layout/registry diagnostics fire first and this one
        // reads as the narrower refusal it is.
        assert!(
            pool.component_layout.size() > 0,
            "ComponentPool::new_untracked: a ZST (stride == 0) pool is tick-driven \
             (grow_rows_zst bounds committed_rows by the tick sub-regions alone), \
             so an untracked ZST pool could never grow past zero rows"
        );

        // Sound to flip AFTER construction rather than threading a flag through
        // `new`: construction is one address-space reservation with ZERO commit
        // (D3), so no tick page has been committed yet and the sentinel cannot
        // strand one. Keeping `new` untouched also keeps it byte-identical for
        // every existing caller.
        pool.ticks_committed = UNTRACKED_TICKS;
        pool
    }

    /// `false` iff this pool was built by
    /// [`new_untracked`](Self::new_untracked) — i.e. its tick sub-regions are
    /// reserved but hold no committed pages, so no tick may be read or written.
    #[inline]
    pub(crate) fn is_tracked(&self) -> bool {
        self.ticks_committed != UNTRACKED_TICKS
    }

    /// Creates a new pool with the Phase X.I D2 byte-targeted, row-clamped
    /// default ceiling:
    /// `reserve_rows = clamp(POOL_TARGET_DATA_BYTES / stride,
    /// POOL_MIN_ROWS, POOL_MAX_ROWS)`.
    pub fn with_default_sizes(component_id: usize) -> Self {
        let component_size = component_registry::get_component_size(component_id)
            .expect("Component not registered");
        Self::new(component_id, pool_reserve_rows(component_size))
    }

    /// Commits one sub-region's frontier step on the page-floor ladder
    /// (packing plan D2): the reservation bytes `[floor + old, floor + new)`.
    ///
    /// `floor` is the sub-region's absolute PAGE FLOOR, `offset - stagger`:
    /// `0` for data, `data_len` for `added`, `data_len + tick_len` for
    /// `changed`. Every sub-region offset is `≡ stagger (mod COMMIT_PAGE)` and
    /// `stagger < POOL_STAGGER_SPAN <= COMMIT_PAGE` (const-asserted), so the
    /// floor is a page multiple and the stagger pad is committed with the
    /// sub-region's first page. `old` / `new` are frontiers measured from that
    /// floor: page multiples by induction over the ladder (`pool_commit_step`
    /// of page multiples, capped at a page multiple), `new > old`. The OS is
    /// therefore told exactly `new - old` bytes — no rounding and no overshoot,
    /// where the granule ladder's `align_down`/`align_up` pair committed a whole
    /// extra granule per sub-region whenever the stagger was non-zero.
    ///
    /// The name predates the page floors and is kept: UG-15 leg (2) pins this
    /// body by exact symbol name (P29-2).
    ///
    /// IM-3: grow is Host-only in Phase 4 — the caller funnels through
    /// `host_vm_mut`, which `unreachable!`s on a Device pool (never minted).
    #[cold]
    #[inline(never)]
    fn commit_subregion(&mut self, floor: usize, old: usize, new: usize) {
        // D2 obligation 5 (strict growth; `VmReservation::commit`'s `new > old`).
        debug_assert!(new > old, "commit_subregion: empty or backwards range");
        // D2 obligation 6 (page alignment of both ends).
        debug_assert!(
            floor.is_multiple_of(COMMIT_PAGE)
                && old.is_multiple_of(COMMIT_PAGE)
                && new.is_multiple_of(COMMIT_PAGE),
            "commit_subregion: floor {floor} + [{old}, {new}) is not page-aligned \
             (D2 obligation 6)"
        );
        self.backing.host_vm_mut().commit(floor + old, floor + new);
    }

    /// Phase X.I D4 — the single cold growth funnel: ensures rows `[0, n)`
    /// are committed read/write (data + both tick sub-regions in lockstep,
    /// by rows).
    ///
    /// Returns `true` when `committed_rows >= n` on exit — callers never
    /// retry (GROW1-XI sufficiency). Returns `false` iff
    /// `n > reserve_rows` (the ceiling) with ZERO state change.
    /// `n <= committed_rows` is an idempotent no-op (★R1-1): zero
    /// syscalls, zero state change — `Archetype::reserve_capacity` Phase B
    /// may call this unconditionally.
    ///
    /// Growth policy: data-region byte doubling clamped to
    /// `[POOL_MIN_SLAB, POOL_MAX_SLAB]`, request-dominant
    /// (`constants::pool_commit_step`), on absolute page floors (packing plan
    /// D2): every frontier is measured from its sub-region's page floor and
    /// steps by whole `COMMIT_PAGE`s, so a one-row column commits exactly one
    /// page per sub-region. O(1) in live rows — no bytes copied, no bytes
    /// written; the base pointers never move (in-place frontier commits on the
    /// pool's own reservation).
    ///
    /// # Panics
    ///
    /// `VmReservation::commit` failure (commit charge / overcommit
    /// exhaustion) — genuine OS OOM; the world is poisoned (same recovery
    /// contract as the `Component::drop` panic policy: discard the
    /// `EcsMaster`).
    #[cold]
    #[inline(never)]
    pub(crate) fn grow_rows(&mut self, n: usize) -> bool {
        if n > self.reserve_rows {
            return false; // ceiling; ZERO state change
        }
        if n <= self.committed_rows {
            return true; // ★R1-1 idempotent no-op; ZERO syscalls, ZERO state change
        }

        // Phase 22 D6: ZST (tag) pools have a vacuous data region — row
        // capacity is tick-driven. The stride > 0 body below divides by
        // stride and commits the data region, both meaningless at stride 0;
        // branch out before touching either (the stride > 0 path below
        // stays byte-identical to its pre-Phase-22 form).
        if self.component_layout.size() == 0 {
            return self.grow_rows_zst(n);
        }

        let stride = self.component_layout.size();
        // The sub-region geometry is a pure function of immutable fields —
        // recomputed on this cold path instead of stored (D1). P2-CACHE-FIX:
        // the SAME stagger `new` used MUST be recomputed here (from the stored
        // `component_id`) so every commit offset matches the construction-time
        // layout — `pool_byte_layout` is the single source of truth for both.
        let stagger = pool_base_stagger(self.component_id);
        let layout = pool_byte_layout(self.reserve_rows, stride, stagger);

        // Packing plan D2 — absolute page floors. Each sub-region's floor is
        // its offset minus the stagger (data 0, added `data_len`, changed
        // `data_len + tick_len`), and every frontier field is measured from its
        // floor, so it includes the stagger pad. `data_len` and `tick_len` are
        // granule multiples and `stagger < COMMIT_PAGE`, so each cap below is
        // the sub-region length at `stagger == 0` and length + one page
        // otherwise.
        let added_floor = layout.data_len;
        let changed_floor = layout.data_len + layout.tick_len;
        let data_cap = pool_align_up_page(stagger + layout.data_len);
        let tick_cap = pool_align_up_page(stagger + layout.tick_len);

        // GROW1-XI proof 1 / D2 obligation 2: n <= reserve_rows => stagger +
        // n*stride <= stagger + data_len => needed <= data_cap, and data_cap
        // <= data_len + COMMIT_PAGE <= added_floor + tick_len: in bounds. The
        // mul cannot overflow: reserve_rows*stride was overflow-checked at
        // construction.
        let needed = pool_align_up_page(stagger + n * stride);
        debug_assert!(
            needed <= data_cap,
            "GROW1-XI step 1 / D2 obligation 2: needed overruns the data cap"
        );
        // GROW1-XI corollary 0a / D2 obligation 5: past both guards `n >
        // committed_rows`, and the clamped case is excluded by the ceiling
        // check. Before the first grow `data_committed == 0 < needed`; after
        // it `committed_rows == (data_committed - stagger) / stride` unclamped,
        // so `stagger + n*stride > data_committed`, and `needed` (a page
        // round-up) exceeds the page multiple `data_committed` — the
        // saturating_sub inside `pool_commit_step` never actually saturates.
        debug_assert!(
            needed > self.data_committed,
            "GROW1-XI corollary 0a: grow_rows reached the commit path with a satisfied request"
        );

        let step = pool_commit_step(self.data_committed, needed);
        // GROW1-XI proof 2 / D2 obligations 2, 5, 6: step >= needed -
        // data_committed by the request-dominant max, and the min(data_cap)
        // clamp cannot bite below `needed` (needed <= data_cap) => new_d >=
        // needed > data_committed strictly (GROW1-XI 0b). Every term is a page
        // multiple, so new_d is one.
        let new_d = (self.data_committed + step).min(data_cap);
        // `commit_subregion` panics only on genuine OS OOM.
        self.commit_subregion(0, self.data_committed, new_d);

        // GROW1-XI proofs 3 + 4 / D2 obligation 1: new_d >= needed >= stagger
        // + n*stride => floor((new_d - stagger) / stride) >= n. No underflow:
        // new_d >= COMMIT_PAGE > stagger. The min(reserve_rows) is
        // LOAD-BEARING — page padding can make the quotient exceed
        // reserve_rows, and the tick sub-regions are sized for reserve_rows
        // only.
        let rows = ((new_d - stagger) / stride).min(self.reserve_rows);
        debug_assert!(
            rows >= n,
            "GROW1-XI step 3: post-grow committed_rows must cover the request"
        );
        // D2 obligation 11 (coverage twin of obligation 1): every exposed row
        // lies inside committed data pages — the direction `row_ptr` relies on.
        debug_assert!(
            stagger + rows * stride <= new_d,
            "D2 obligation 11: committed rows overrun the committed data pages"
        );

        // GROW1-XI proof 5 / D2 obligation 3: rows <= reserve_rows =>
        // stagger + rows*4 <= stagger + tick_len => t_new <= tick_cap. The
        // bound is the cap, not `tick_len`: at stagger > 0 and full capacity
        // t_new legitimately reaches tick_len + COMMIT_PAGE.
        let t_new = pool_align_up_page(stagger + rows * 4);
        debug_assert!(
            t_new <= tick_cap,
            "GROW1-XI step 5 / D2 obligation 3: tick commit overruns the tick cap"
        );
        // D2 obligation 4: the `changed` interval ends inside the reservation
        // (`os_len` holds two whole tick sub-regions plus a granule of slack
        // at stagger > 0; exactly two at stagger == 0).
        debug_assert!(
            changed_floor + t_new <= layout.os_len,
            "D2 obligation 4: the changed-tick commit overruns the reservation"
        );
        if t_new > self.ticks_committed {
            self.commit_subregion(added_floor, self.ticks_committed, t_new);
            self.commit_subregion(changed_floor, self.ticks_committed, t_new);
            // ★Q6: the frontier field is written only AFTER the commits it
            // describes succeed (panic-coherent on a mid-grow OS OOM).
            self.ticks_committed = t_new;
        }
        // D2 obligation 12 (coverage twin of obligation 3): a tracked pool's
        // exposed rows lie inside committed tick pages. An untracked pool
        // never touches a tick, and its sentinel skipped both commits above.
        debug_assert!(
            !self.is_tracked() || stagger + rows * 4 <= self.ticks_committed,
            "D2 obligation 12: committed rows overrun the committed tick pages"
        );
        self.data_committed = new_d;
        self.committed_rows = rows;
        true
    }

    /// Phase 22 D6 — tick-region-driven growth for ZST (tag) pools.
    ///
    /// `#[cold]` sibling of [`grow_rows`](Self::grow_rows), reached ONLY
    /// through its early `stride == 0` branch AFTER the ceiling and
    /// idempotency guards: on entry `committed_rows < n <= reserve_rows`
    /// holds (debug-asserted).
    ///
    /// # GROW1-ZST proof chain (Z1–Z6)
    ///
    /// * **Z1 (driver)**: for `stride == 0`, row capacity is bounded by the
    ///   tick sub-regions alone; `data_committed` is invariantly 0 forever
    ///   (debug-asserted below) and `vm.commit` is NEVER called on the
    ///   (vacuous) data region — the vm `new > old` assert on a data commit
    ///   and the stride>0 path's `new_d / stride` division are structurally
    ///   unreachable.
    /// * **Z2 (policy)**: reuses `pool_commit_step(ticks_committed,
    ///   needed_t)` with `needed_t = align_up_page(stagger + n * 4)` — the
    ///   same request-dominant doubling, applied to the tick byte frontier,
    ///   which (packing plan D2) is measured from each tick sub-region's
    ///   absolute page floor: `added` at 0 (the data region has length 0),
    ///   `changed` at `tick_len`.
    /// * **Z3 (in-bounds)**: `n <= reserve_rows ⇒ stagger + n*4 <= stagger +
    ///   tick_len ⇒ needed_t <= tick_cap = align_up_page(stagger +
    ///   tick_len)`, and `t_new = (ticks_committed + step).min(tick_cap)`
    ///   never overruns it; `tick_len + tick_cap <= os_len` (obligation 4).
    /// * **Z4 (strict growth)**: past the guards `n > committed_rows`. The
    ///   reserve-clamp case is excluded (`committed_rows == reserve_rows`
    ///   would contradict `n <= reserve_rows < n`). Before the first grow
    ///   `ticks_committed == 0 < COMMIT_PAGE <= needed_t`; after it
    ///   `committed_rows = (ticks_committed - stagger) / 4` exactly (a page
    ///   multiple minus a 64-B multiple is divisible by 4), hence `stagger +
    ///   n*4 > ticks_committed`, hence `needed_t > ticks_committed` (a page
    ///   round-up past a page multiple), and with Z3 `t_new >= needed_t >
    ///   ticks_committed` — BOTH tick commits satisfy the vm `new > old`
    ///   assert.
    /// * **Z5 (sufficiency)**: `committed_rows' = ((t_new - stagger) / 4)
    ///   .min(reserve_rows) >= n`, since `t_new >= needed_t >= stagger + n*4`
    ///   and `n <= reserve_rows` (debug-asserted — the GROW1-XI step-3
    ///   analogue). No underflow: `t_new >= COMMIT_PAGE > stagger`. Callers
    ///   never retry. Its coverage twin `stagger + rows*4 <= t_new` is
    ///   debug-asserted too.
    /// * **Z6 (panic coherence)**: `ticks_committed` and `committed_rows`
    ///   are written only AFTER both commits succeed (★Q6 pattern
    ///   preserved — a mid-grow OS OOM leaves the frontier fields
    ///   describing only committed pages).
    #[cold]
    #[inline(never)]
    fn grow_rows_zst(&mut self, n: usize) -> bool {
        debug_assert_eq!(
            self.component_layout.size(),
            0,
            "grow_rows_zst: reachable only for stride == 0 pools"
        );
        // Z1: the data region of a ZST pool is vacuous and never committed.
        debug_assert_eq!(
            self.data_committed, 0,
            "Z1: a ZST pool must never commit data bytes"
        );
        // Entry contract established by grow_rows' two guards.
        debug_assert!(
            self.committed_rows < n && n <= self.reserve_rows,
            "grow_rows_zst: entry contract (committed_rows < n <= reserve_rows) violated"
        );

        // Pure function of immutable fields — recomputed on this cold path
        // instead of stored (D1), mirroring the stride > 0 body. P2-CACHE-FIX:
        // the SAME stagger `new` used (from the stored `component_id`).
        let stagger = pool_base_stagger(self.component_id);
        let layout = pool_byte_layout(self.reserve_rows, 0, stagger);
        debug_assert_eq!(layout.data_len, 0, "Z1: vacuous data region");
        debug_assert_eq!(
            layout.added_off, stagger,
            "Z1: added ticks start at the stagger pad (vacuous data region)"
        );

        // D2 page floors of the two tick sub-regions (see Z2) and the tick cap.
        let changed_floor = layout.tick_len;
        let tick_cap = pool_align_up_page(stagger + layout.tick_len);

        // Z2: the tick byte frontier drives the policy. The mul cannot
        // overflow: `reserve_rows * 4` was overflow-checked by
        // `pool_byte_layout` at construction and `n <= reserve_rows`.
        let needed_t = pool_align_up_page(stagger + n * 4);
        debug_assert!(
            needed_t <= tick_cap,
            "Z3: tick request overruns the tick cap"
        );
        debug_assert!(
            needed_t > self.ticks_committed,
            "Z4: grow_rows_zst reached the commit path with a satisfied request"
        );

        let step = pool_commit_step(self.ticks_committed, needed_t);
        let t_new = (self.ticks_committed + step).min(tick_cap);
        debug_assert!(
            t_new > self.ticks_committed,
            "Z4: the tick frontier must grow strictly (vm `new > old` precondition)"
        );
        debug_assert!(
            changed_floor + t_new <= layout.os_len,
            "Z3 / D2 obligation 4: the changed-tick commit overruns the reservation"
        );

        // Z3 + Z4: both tick frontiers are strictly growing and in-bounds of
        // the reservation. Panics only on genuine OS OOM (same contract as
        // the stride > 0 path).
        self.commit_subregion(0, self.ticks_committed, t_new);
        self.commit_subregion(changed_floor, self.ticks_committed, t_new);

        // Z5: the min(reserve_rows) is LOAD-BEARING — page padding can make
        // the quotient exceed reserve_rows.
        let rows = ((t_new - stagger) / 4).min(self.reserve_rows);
        debug_assert!(
            rows >= n,
            "Z5: post-grow committed_rows must cover the request"
        );
        // Z5's coverage twin: every exposed row's tick slots lie inside the
        // committed tick pages.
        debug_assert!(
            stagger + rows * 4 <= t_new,
            "Z5 coverage: committed rows overrun the committed tick pages"
        );

        // Z6: frontier fields written only AFTER both commits succeeded.
        self.ticks_committed = t_new;
        self.committed_rows = rows;
        true
    }

    /// Test-only memory model of this pool (packing plan D7, obligation 10):
    /// the byte length of the UNION of the three committed intervals, each read
    /// at its sub-region's absolute page floor (`sub-region offset - stagger`):
    ///
    /// * data: `[0, data_committed)`;
    /// * added: `[data_len, data_len + ticks_committed)`;
    /// * changed: `[data_len + tick_len, data_len + tick_len + ticks_committed)`.
    ///
    /// Both tick intervals are empty on an untracked pool, and the data
    /// interval is empty on a ZST pool.
    ///
    /// The union, not the sum `data_committed + 2 * ticks_committed`: adjacent
    /// intervals share a page wherever the earlier region's ladder has reached
    /// its cap, and which boundaries that happens at depends on the geometry.
    /// The OS-truth tests (`commit_floor_tests`) hold this model to the pages
    /// the kernel actually reports, so a model that merely restates the
    /// arithmetic cannot pass them.
    #[cfg(test)]
    pub(crate) fn committed_bytes(&self) -> usize {
        let layout = pool_byte_layout(
            self.reserve_rows,
            self.component_layout.size(),
            pool_base_stagger(self.component_id),
        );
        let ticks = if self.is_tracked() { self.ticks_committed } else { 0 };
        let (dl, tl) = (layout.data_len, layout.tick_len);
        // Already sorted by start: 0 <= data_len <= data_len + tick_len.
        let intervals = [
            (0, self.data_committed),
            (dl, dl + ticks),
            (dl + tl, dl + tl + ticks),
        ];
        let mut total = 0;
        let mut open: Option<(usize, usize)> = None;
        for (start, end) in intervals {
            if end <= start {
                continue;
            }
            open = match open {
                Some((s, e)) if start <= e => Some((s, e.max(end))),
                Some((s, e)) => {
                    total += e - s;
                    Some((start, end))
                }
                None => Some((start, end)),
            };
        }
        if let Some((s, e)) = open {
            total += e - s;
        }
        total
    }

    /// Byte pointer for row `idx`, computed from the stable reservation base.
    ///
    /// Phase 22 D6: for `stride == 0` (tag pools) every row returns the
    /// dangling aligned base — valid because only zero-size reads, writes,
    /// and drops ever go through it.
    ///
    /// # Safety
    /// * `idx < self.committed_rows` (the slot lies inside the committed
    ///   prefix of the data sub-region); reads of LIVE data additionally
    ///   require `idx < self.len`.
    /// * Valid for `self.component_layout.size()` bytes.
    #[inline]
    unsafe fn row_ptr(&self, idx: usize) -> *mut u8 {
        debug_assert!(idx < self.committed_rows, "row_ptr: idx out of committed bounds");
        // SAFETY: stride > 0 — idx < committed_rows <= reserve_rows ⇒
        //   idx*stride + stride <= reserve_rows*stride <= data_len, so the
        //   element span lies inside the data sub-region of the pool's OWN
        //   reservation, within committed (read/write) pages. Provenance
        //   derives from `self.buffer` via one `add` — and the data region
        //   plus BOTH tick regions are ONE allocated object (★R1-8: a
        //   single `VmReservation` per pool). The base is write-once in
        //   `new` and never moves: Phase X.I growth only commits fresh
        //   pages at the frontier of the SAME reservation; previously
        //   returned pointers are never remapped or relocated.
        //   stride == 0 (Phase 22) — the offset is `idx * 0 == 0` and
        //   `add(0)` is valid for ANY pointer, so every row yields the
        //   dangling, T-aligned base, which is valid exclusively for
        //   zero-size access (the only access a ZST pool ever performs).
        unsafe { self.buffer.as_ptr().add(idx * self.component_layout.size()) }
    }

    /// Adds a component to the pool via raw byte slice.
    ///
    /// The caller must ensure `component_bytes` contains a valid, initialized
    /// representation of the pool's registered type.
    ///
    /// Returns the slot index on success, `None` when the pool's reserve
    /// ceiling (`reserve_rows`) is exhausted — committed capacity below the
    /// ceiling grows inline via the cold [`grow_rows`](Self::grow_rows)
    /// path (Phase X.I D5).
    #[doc(hidden)]
    pub fn add(&mut self, component_bytes: &[u8]) -> Option<usize> {
        debug_assert_eq!(
            component_bytes.len(),
            self.component_layout.size(),
            "Component size mismatch: expected {}, got {}",
            self.component_layout.size(),
            component_bytes.len()
        );

        // ★R1-2 (binding single-compare shape): ONE warm compare, not taken
        // on the hot path. The reserve-ceiling check lives INSIDE the cold
        // `grow_rows` (its first guard) — an explicit warm ceiling compare
        // here would be redundant. `None` therefore still means
        // reserve-ceiling exhaustion, now >= 16x further out (Phase X.I D2).
        if self.len >= self.committed_rows && !self.grow_rows(self.len + 1) {
            return None;
        }

        let buffer_index = self.len;

        // SAFETY: buffer_index < committed_rows (grown above if needed), so
        // `row_ptr` yields a pointer to a committed slot inside the pool's
        // reservation. The source and destination do not overlap (source is
        // caller memory, destination is the pool's reservation). The row is
        // uninitialised until this write; `self.len += 1` below marks it live.
        unsafe {
            std::ptr::copy_nonoverlapping(
                component_bytes.as_ptr(),
                self.row_ptr(buffer_index),
                self.component_layout.size(),
            );
        }

        self.len += 1;

        Some(buffer_index)
    }

    /// Type-checked append. Consumes `value` by move into the pool's slot.
    ///
    /// # Returns
    /// - `Some(slot_index)` on success.
    /// - `None` when the reserve ceiling (`reserve_rows`) is exhausted —
    ///   committed capacity below the ceiling grows inline (Phase X.I D5).
    ///   `value` drops normally at the caller's scope exit — the pool is
    ///   not modified and no slot is allocated.
    ///
    /// # Panics (debug only)
    /// `debug_assert!` if `TypeId::of::<T>()` does not match the pool's
    /// registered type.
    #[inline]
    pub fn add_typed<T: Component>(&mut self, value: T) -> Option<usize> {
        debug_assert_eq!(
            self.component_type_id,
            TypeId::of::<T>(),
            "ComponentPool typed API: T = {} does not match pool's registered type",
            std::any::type_name::<T>()
        );

        // ★R1-2 binding single-compare shape — see `add` for the rationale.
        if self.len >= self.committed_rows && !self.grow_rows(self.len + 1) {
            return None; // value drops at scope exit
        }

        let buffer_index = self.len;

        // SAFETY:
        // - buffer_index < committed_rows (grown above if needed), so
        //   `row_ptr` yields a pointer to a committed slot inside the pool's
        //   reservation.
        // - The slot is aligned to align_of::<T>(): buffer base is aligned to
        //   component_layout.align(); per the Rust Reference §"Type Layout",
        //   size_of::<T>() is a multiple of align_of::<T>() for every Sized T,
        //   so the stride preserves alignment.
        // - The slot is exclusively owned (&mut self); no aliasing.
        // - ptr::write consumes `value` by move; the local binding ceases to
        //   exist after this call — no scope-exit drop.
        unsafe { core::ptr::write(self.row_ptr(buffer_index).cast::<T>(), value) };

        self.len += 1;

        Some(buffer_index)
    }

    /// Inlined typed append for a Copy/POD element (the [`ScratchColumn`] fast
    /// path).
    ///
    /// Mirrors [`add_typed`](Self::add_typed) but bounds `T: Copy` (no
    /// `Component`), and is `#[inline]` so it lowers into the caller's hot build
    /// loop as a typed store — unlike the type-erased, non-inlined byte
    /// [`add`](Self::add). The single warm compare + cold grow is byte-identical
    /// to `add` / `add_typed`.
    ///
    /// # Returns
    /// - `Some(slot_index)` on success.
    /// - `None` when the reserve ceiling (`reserve_rows`) is exhausted — the
    ///   pool is not modified.
    ///
    /// # Panics (debug only)
    /// `debug_assert!` if `TypeId::of::<T>()` does not match the pool's
    /// registered type.
    ///
    /// [`ScratchColumn`]: crate::ecs::core::component::scratch::ScratchColumn
    #[inline]
    pub(crate) fn push_copy<T: Copy + 'static>(&mut self, value: T) -> Option<usize> {
        debug_assert_eq!(
            self.component_type_id,
            TypeId::of::<T>(),
            "ComponentPool::push_copy: T = {} does not match pool's registered type",
            std::any::type_name::<T>()
        );

        // ★R1-2 binding single-compare shape — see `add` for the rationale.
        if self.len >= self.committed_rows && !self.grow_rows(self.len + 1) {
            return None;
        }

        let buffer_index = self.len;

        // SAFETY:
        // - buffer_index < committed_rows (grown above if needed), so `row_ptr`
        //   yields a pointer to a committed (read/write) slot inside the pool's
        //   reservation.
        // - The slot is aligned to align_of::<T>(): the buffer base is aligned
        //   to component_layout.align() (>= align_of::<T>(), debug-asserted by
        //   `ScratchColumn::new`), and stride is a multiple of that alignment.
        // - The slot is exclusively owned (&mut self); no aliasing.
        // - ptr::write consumes `value` by move (a bitwise copy for `T: Copy`);
        //   `self.len += 1` below marks the slot live. The written bytes equal
        //   the `copy_nonoverlapping` of `value`'s bytes that `add` performs.
        unsafe { core::ptr::write(self.row_ptr(buffer_index).cast::<T>(), value) };

        self.len += 1;

        Some(buffer_index)
    }

    /// O(1) logical clear for a drop-free (Copy/POD) pool: resets `len` to 0
    /// without per-element work. The committed pages stay resident (the reuse
    /// contract — the next refill writes over them).
    ///
    /// Only sound when the element needs no drop: leaving `[0, old_len)`
    /// un-dropped would leak or double-free a non-`Copy` `T`. The
    /// `debug_assert!(self.drop_fn.is_none())` enforces that — the
    /// [`ScratchColumn`] contract is `T: Copy` ⇒ `!needs_drop` ⇒
    /// `drop_fn == None`.
    ///
    /// [`ScratchColumn`]: crate::ecs::core::component::scratch::ScratchColumn
    #[inline]
    pub(crate) fn clear_no_drop(&mut self) {
        debug_assert!(
            self.drop_fn.is_none(),
            "clear_no_drop on a pool with a drop_fn (would leak undropped rows)"
        );
        self.len = 0;
    }

    /// O(1) logical shrink for a drop-free (Copy/POD) pool: lowers `len` to
    /// `new_len` without per-element work, leaving the committed pages resident.
    /// A `new_len` at or above the current one is a no-op (`Vec::truncate`
    /// semantics), so a caller need not compare first.
    ///
    /// Sound for exactly the reason [`clear_no_drop`](Self::clear_no_drop) is:
    /// dropping the tail is a no-op only when the element needs no drop, which
    /// the [`ScratchColumn`] contract (`T: Copy`) guarantees and the
    /// `debug_assert!` enforces.
    ///
    /// The rows above `new_len` keep their bytes and are re-exposed verbatim by a
    /// later `push` / `resize` back over them — that is the reuse contract, not a
    /// leak, and the same one `clear_no_drop` relies on.
    ///
    /// [`ScratchColumn`]: crate::ecs::core::component::scratch::ScratchColumn
    /// Sets the live row count of a drop-free (Copy/POD) pool to `new_len`,
    /// asserting the rows are committed.
    ///
    /// The write-back half of a cached-frontier refill: a caller that has already
    /// written rows `[old_len, new_len)` through the pool's stable base publishes
    /// the frontier with this, instead of paying a field store per element.
    ///
    /// # Safety contract (upheld by the caller, debug-asserted here)
    /// Rows `[0, new_len)` must all be INITIALISED. Raising `len` over rows that
    /// were never written would expose uninitialised memory as live `T`.
    #[inline]
    pub(crate) fn set_len_no_drop(&mut self, new_len: usize) {
        debug_assert!(
            self.drop_fn.is_none(),
            "set_len_no_drop on a pool with a drop_fn"
        );
        debug_assert!(
            new_len <= self.committed_rows,
            "set_len_no_drop past the committed frontier"
        );
        self.len = new_len;
    }

    #[inline]
    pub(crate) fn truncate_no_drop(&mut self, new_len: usize) {
        debug_assert!(
            self.drop_fn.is_none(),
            "truncate_no_drop on a pool with a drop_fn (would leak undropped rows)"
        );
        if new_len < self.len {
            self.len = new_len;
        }
    }

    /// Extends a Copy/POD pool to `new_len` rows, writing `value` into every slot
    /// from the current frontier up to it. Shorter or equal `new_len` is a no-op
    /// (the caller wanting a shrink uses [`truncate_no_drop`](Self::truncate_no_drop)).
    ///
    /// This exists rather than a `push_copy` loop because the loop pays the
    /// `len >= committed_rows` compare and a potential cold `grow_rows` call PER
    /// ELEMENT, while the span is known up front: one grow covers the whole
    /// extension and the fill becomes a constant-stride typed store loop.
    ///
    /// # Returns
    /// - `true` on success.
    /// - `false` when the reserve ceiling (`reserve_rows`) is exhausted — the pool
    ///   is NOT modified (`grow_rows` is zero-state-change on the ceiling path).
    ///
    /// # Panics (debug only)
    /// `debug_assert!` if `TypeId::of::<T>()` does not match the pool's registered
    /// type, or if the pool has a `drop_fn`.
    #[inline]
    pub(crate) fn extend_fill_copy<T: Copy + 'static>(&mut self, new_len: usize, value: T) -> bool {
        debug_assert_eq!(
            self.component_type_id,
            TypeId::of::<T>(),
            "ComponentPool::extend_fill_copy: T = {} does not match pool's registered type",
            std::any::type_name::<T>()
        );
        debug_assert!(
            self.drop_fn.is_none(),
            "extend_fill_copy on a pool with a drop_fn (the fill would overwrite live rows              without dropping them)"
        );
        if new_len <= self.len {
            return true;
        }
        if new_len > self.committed_rows && !self.grow_rows(new_len) {
            return false;
        }
        for idx in self.len..new_len {
            // SAFETY: mirrors `push_copy`'s per-slot write, hoisting its grow check
            // out of the loop.
            // - `idx < new_len <= committed_rows` (grown above if needed), so
            //   `row_ptr` yields a pointer to a committed (read/write) slot inside
            //   the pool's reservation.
            // - The slot is aligned to `align_of::<T>()`: the buffer base is aligned
            //   to `component_layout.align()` (>= `align_of::<T>()`, debug-asserted
            //   by `ScratchColumn::new`) and stride is a multiple of that alignment.
            // - The slot is exclusively owned (`&mut self`); no aliasing, and no
            //   `&[T]` / `&mut [T]` over the span exists yet because `len` is only
            //   raised AFTER every slot has been written.
            // - `ptr::write` initialises the slot from a `Copy` value, so the rows
            //   `[old_len, new_len)` are valid `T` before they become live.
            unsafe { core::ptr::write(self.row_ptr(idx).cast::<T>(), value) };
        }
        self.len = new_len;
        true
    }

    /// Removes the last component from the pool, invoking drop glue if needed.
    pub fn pop(&mut self) -> bool {
        if self.len == 0 {
            return false;
        }

        let last_index = self.len - 1;

        // SAFETY:
        // - last_index < self.len, so `row_ptr` addresses a slot written by a
        //   prior add/add_typed (initialized).
        // - We hold &mut self → exclusive access, no aliasing.
        // - After drop_fn, the slot is logically uninitialized; `self.len -= 1`
        //   below removes it from the live range so it becomes unreachable.
        unsafe {
            if let Some(drop_fn) = self.drop_fn {
                drop_fn(self.row_ptr(last_index));
            }
        }

        self.len -= 1;

        true
    }

    /// Returns the index of the last component in the pool.
    ///
    /// Useful when determining what will be affected by a `swap_remove`.
    #[inline]
    pub fn last_index(&self) -> Option<usize> {
        if self.len == 0 {
            None
        } else {
            Some(self.len - 1)
        }
    }

    /// Removes a component by index using swap_remove to maintain dense storage.
    ///
    /// The component at `index` is dropped via the registered drop glue before
    /// the last component is memcpy'd into its slot.
    pub fn swap_remove(&mut self, index: usize) -> bool {
        if index >= self.len {
            return false;
        }

        let last_index = self.len - 1;

        if index != last_index {
            // SAFETY:
            // - index < self.len and last_index < self.len, so both `row_ptr`
            //   results address slots written by a prior add/add_typed
            //   (initialized).
            // - We hold &mut self → exclusive access. Non-overlap: for
            //   stride > 0 the two slots are distinct stride multiples
            //   (index != last_index); for stride == 0 (Phase 22 tag pools)
            //   this is a ZERO-byte copy between equal dangling pointers —
            //   trivially non-overlapping and explicitly allowed
            //   (`copy_nonoverlapping` with count 0 imposes no validity
            //   requirements beyond alignment, which the dangling base
            //   satisfies by construction).
            // - After drop_fn, the slot at `index` is logically uninitialized;
            //   the copy_nonoverlapping below overwrites it with last's bytes,
            //   restoring the invariant.
            // - PANIC CAVEAT: if T::drop panics, the slot at `index` is
            //   uninitialized while self.len still includes it. Per the
            //   Component trait panic policy this is a logic bug in the user's
            //   Drop impl; the pool is considered poisoned.
            unsafe {
                let removed_ptr = self.row_ptr(index);
                if let Some(drop_fn) = self.drop_fn {
                    drop_fn(removed_ptr);
                }
                std::ptr::copy_nonoverlapping(
                    self.row_ptr(last_index),
                    removed_ptr,
                    self.component_layout.size(),
                );
            }

            // Phase 10 STORE5: swap tick slots in lockstep with the data
            // buffer. The last row's ticks move into the vacated slot so
            // row `index` continues to carry the moved entity's lifecycle
            // history. No tick is dropped here — `Tick` is `Copy`.
            //
            // SAFETY: `index != last_index` (checked above) and both
            // indices are `< self.len <= committed_rows`, so both slots lie
            // in the committed prefix of each tick sub-region.
            // `&mut self` gives exclusive access to the tick sub-regions;
            // no concurrent reader exists per Phase 9 SCH3.
            unsafe {
                debug_assert!(
                    self.is_tracked(),
                    "ComponentPool::swap_remove (tick swap): tick access on an UNTRACKED pool \
                     (new_untracked) - the tick sub-regions are reserved, NOT committed"
                );
                let added = self.added_base.as_ptr();
                let changed = self.changed_base.as_ptr();
                *(*added.add(index)).get() = *(*added.add(last_index)).get();
                *(*changed.add(index)).get() = *(*changed.add(last_index)).get();
            }
        } else {
            // Removing the last element: drop in place, no memcpy.
            //
            // SAFETY: index == last_index < self.len, so `row_ptr` addresses a
            // slot written by a prior add/add_typed (initialized). Exclusive
            // access via &mut self. `self.len -= 1` below removes it from the
            // live range so the slot becomes unreachable.
            unsafe {
                if let Some(drop_fn) = self.drop_fn {
                    drop_fn(self.row_ptr(index));
                }
            }
        }

        self.len -= 1;
        true
    }

    /// Gets a pointer to a component by index.
    pub fn get_raw(&self, index: usize) -> Option<*const u8> {
        if index >= self.len {
            return None;
        }
        // SAFETY: index < self.len ⇒ within the live, initialized range; the
        // slot was written by a prior add/add_typed.
        Some(unsafe { self.row_ptr(index).cast_const() })
    }

    /// Gets a mutable pointer to a component by index.
    pub fn get_raw_mut(&mut self, index: usize) -> Option<*mut u8> {
        if index >= self.len {
            return None;
        }
        // SAFETY: index < self.len ⇒ within the live, initialized range; the
        // slot was written by a prior add/add_typed.
        Some(unsafe { self.row_ptr(index) })
    }

    /// Type-checked shared read.
    ///
    /// A typed wrapper over [`get_raw`](ComponentPool::get_raw) that asserts
    /// the caller's `T` matches the pool's registered type before casting. This
    /// surfaces registry-mismatch bugs at the read boundary rather than
    /// silently producing a mis-typed reference (defense-in-depth for audit C-004).
    ///
    /// # Returns
    /// - `Some(&T)` if `index < self.count()`.
    /// - `None` if `index` is out of bounds.
    ///
    /// # Panics (debug only)
    /// `debug_assert!` fires if `TypeId::of::<T>()` does not match the pool's
    /// registered type — surfaces caller bugs at the read boundary instead of
    /// producing a mis-typed reference (audit C-004).
    #[inline]
    pub fn get_typed<T: Component>(&self, index: usize) -> Option<&T> {
        debug_assert_eq!(
            self.component_type_id,
            TypeId::of::<T>(),
            "ComponentPool typed read: T = {} does not match pool's registered type",
            std::any::type_name::<T>()
        );
        let ptr = self.get_raw(index)?;
        // SAFETY:
        // - `get_raw` returns `Some(ptr)` only when `index < self.len`,
        //   meaning the slot was populated via `add` / `add_typed` and has not
        //   been removed. All such slots are fully initialized.
        // - The pool allocates its buffer aligned to `component_layout.align()`,
        //   which equals `align_of::<T>()` because `TypeId::of::<T>()` matches
        //   the registered type (asserted by `debug_assert_eq!` above). Each
        //   slot offset is a multiple of `size_of::<T>()`, which is itself a
        //   multiple of `align_of::<T>()` per the Rust Reference §"Type Layout".
        // - `&self` guarantees no concurrent mutable access for the lifetime of
        //   the returned reference.
        Some(unsafe { &*ptr.cast::<T>() })
    }

    /// Type-checked exclusive read.
    ///
    /// A typed wrapper over [`get_raw_mut`](ComponentPool::get_raw_mut) that
    /// asserts the caller's `T` matches the pool's registered type before
    /// casting. Same defense-in-depth rationale as [`get_typed`](ComponentPool::get_typed).
    ///
    /// # Returns
    /// - `Some(&mut T)` if `index < self.count()`.
    /// - `None` if `index` is out of bounds.
    ///
    /// # Panics (debug only)
    /// Same TypeId mismatch check as `get_typed`.
    #[inline]
    pub fn get_mut_typed<T: Component>(&mut self, index: usize) -> Option<&mut T> {
        debug_assert_eq!(
            self.component_type_id,
            TypeId::of::<T>(),
            "ComponentPool typed mut read: T = {} does not match pool's registered type",
            std::any::type_name::<T>()
        );
        let ptr = self.get_raw_mut(index)?;
        // SAFETY:
        // - `get_raw_mut` returns `Some(ptr)` only when `index < self.len`,
        //   meaning the slot is fully initialized.
        // - Alignment matches `align_of::<T>()` per the same reasoning as
        //   `get_typed`: the TypeId `debug_assert_eq!` above confirms `T` is the
        //   pool's registered type, so `component_layout.align() == align_of::<T>()`.
        // - `&mut self` provides exclusive ownership of the pool; no other
        //   reference to this slot exists for the lifetime of the return value.
        Some(unsafe { &mut *ptr.cast::<T>() })
    }

    /// Overwrites the component at `index` with `component_bytes` (raw API).
    ///
    /// Invokes drop glue on the existing value before overwriting.
    ///
    /// # Safety contract (raw API)
    /// The caller is responsible for ensuring that `component_bytes` is a
    /// valid, initialized representation of the pool's registered type. If
    /// the bytes are not of type `T`, the future read or drop of the slot
    /// is undefined behavior — this is the pre-existing raw-API contract.
    ///
    /// # Panic safety
    ///
    /// If the existing component's `Drop` impl panics during the internal
    /// `drop_fn` call, the slot at `index` becomes logically uninitialized
    /// while `self.len` still includes it. Any subsequent operation on the
    /// pool that touches this slot is undefined behavior.
    ///
    /// Per the engine-wide policy (see `Component` trait `# Panic safety`):
    /// `Component::drop` must not panic. If a panicking `Drop` is unavoidable,
    /// the recovery contract is: discard the entire `EcsMaster`.
    #[doc(hidden)]
    pub fn set_component(&mut self, index: usize, component_bytes: &[u8]) -> bool {
        debug_assert_eq!(
            component_bytes.len(),
            self.component_layout.size(),
            "Component size mismatch: expected {}, got {}",
            self.component_layout.size(),
            component_bytes.len()
        );

        if index >= self.len {
            return false;
        }

        // SAFETY:
        // - index < self.len (checked); the slot is live and initialized.
        // - row_ptr is aligned to the pool's component type (pool allocation
        //   invariant).
        // - Exclusive access via &mut self; no aliasing.
        // - drop_fn drops the existing value; copy_nonoverlapping writes the
        //   new bytes. Both halves use the same slot as destination/source
        //   respectively. They are sequenced (drop then write), so there is no
        //   overlap issue.
        // - If component_bytes is not the correct type representation, the new
        //   slot contents are UB on subsequent typed access — raw API caller's
        //   responsibility (unchanged from pre-existing contract).
        // - Source (caller memory) and destination (pool slot) do not overlap.
        unsafe {
            let ptr = self.row_ptr(index);
            if let Some(drop_fn) = self.drop_fn {
                drop_fn(ptr);
            }
            std::ptr::copy_nonoverlapping(
                component_bytes.as_ptr(),
                ptr,
                self.component_layout.size(),
            );
        }

        true
    }

    /// Type-checked in-place overwrite: drops the existing component at
    /// `index` (invoking drop glue if registered), then moves `value` into
    /// the same slot.
    ///
    /// The slot index is preserved, so any external mapping
    /// (e.g. `EntityInland.unit_index`) remains valid.
    ///
    /// # Returns
    /// - `true` on success.
    /// - `false` if `index >= self.len`. `value` drops normally at scope exit
    ///   — the pool is not modified.
    ///
    /// # Panic safety
    /// **This method is NOT panic-safe.** If the existing component's `Drop`
    /// impl panics during the internal drop_fn call, the slot at `index`
    /// becomes logically uninitialized while `self.len` still includes it.
    /// Any subsequent operation on the pool that touches this slot is
    /// undefined behavior.
    ///
    /// This matches the engine-wide policy in the `Component` trait docs:
    /// **`Component::drop` must not panic.** If a panicking `Drop` is
    /// unavoidable in your application, the recovery contract is: **do not
    /// touch the affected `EcsMaster` again — drop it entirely**.
    ///
    /// # Panics (debug only)
    /// `debug_assert!` on `TypeId` mismatch.
    #[inline]
    pub fn set_component_typed<T: Component>(&mut self, index: usize, value: T) -> bool {
        debug_assert_eq!(
            self.component_type_id,
            TypeId::of::<T>(),
            "ComponentPool typed API: T = {} does not match pool's registered type",
            std::any::type_name::<T>()
        );

        if index >= self.len {
            return false; // value drops at scope exit
        }

        // SAFETY:
        // - index < self.len (checked); the slot is live and initialized.
        // - row_ptr came from pool allocation; aligned to align_of::<T>() (pool
        //   allocation invariant: buffer aligned to component_layout.align(), and
        //   stride is a multiple of that alignment).
        // - Exclusive access via &mut self.
        // - PANIC CAVEAT: see method-level Panic safety rustdoc above. If
        //   T::Drop panics, the slot is left uninitialized. Caller upholds the
        //   engine contract; if violated, pool is poisoned per the documented
        //   recovery policy.
        // - ptr::write is nounwind (core intrinsic); consumes `value` by move —
        //   the local binding ceases to exist after this call.
        unsafe {
            let ptr = self.row_ptr(index);
            if let Some(drop_fn) = self.drop_fn {
                drop_fn(ptr);
            }
            core::ptr::write(ptr.cast::<T>(), value);
        }

        true
    }

    /// Gets the number of active components.
    #[inline]
    pub fn count(&self) -> usize {
        self.len
    }

    /// Gets the pool's row ceiling (`reserve_rows`) — the bound exhaustion
    /// is measured against (Phase X.I D6; the X.F precedent: capacity =
    /// reserve). Committed capacity below the ceiling grows on demand and
    /// is reported by [`Self::committed_rows`].
    #[inline]
    pub fn capacity(&self) -> usize {
        self.reserve_rows
    }

    /// Phase X.I D6: rows currently committed read/write — the growth
    /// frontier (diagnostics/tests).
    /// Invariant: `count() <= committed_rows() <= capacity()`.
    #[inline]
    pub fn committed_rows(&self) -> usize {
        self.committed_rows
    }

    /// Gets the component ID.
    #[inline]
    pub fn component_id(&self) -> usize {
        self.component_id
    }

    /// Gets the component layout.
    #[inline]
    pub fn component_layout(&self) -> Layout {
        self.component_layout
    }

    /// Returns the base pointer of the flat component buffer.
    ///
    /// The buffer holds `self.count()` initialised components at stride
    /// `self.component_layout().size()`. Slot `i` starts at
    /// `buffer_ptr().add(i * size)` and is valid for `size` bytes.
    ///
    /// # Alignment invariant (Phase X.A SIMD-A1)
    ///
    /// The returned pointer is guaranteed to be aligned to at least
    /// `max(align_of::<T>(), SIMD_BUFFER_ALIGN)`. For all component types
    /// `T` with `align_of::<T>() <= SIMD_BUFFER_ALIGN`, this is
    /// [`SIMD_BUFFER_ALIGN`]
    /// = 32 bytes — sufficient for AVX2 aligned 256-bit loads from the
    /// column start.
    ///
    /// This eliminates the cross-cache-line load penalty on archetype row 0
    /// (Intel Optimization Manual §3.6) that the previous `align_of::<T>()`
    /// alignment incurred for small-aligned types such as `f32`.
    ///
    /// Per-row alignment beyond `align_of::<T>()` is **not** guaranteed: for
    /// non-power-of-2-sized `T` (e.g. `struct Foo([f32; 3])`, 12 B), interior
    /// rows are aligned only to `align_of::<T>()`. Users emitting explicit
    /// SIMD loads must use unaligned-load intrinsics (`_mm256_loadu_ps`) or
    /// rely on LLVM autovectorisation, which handles unaligned interior rows
    /// correctly.
    ///
    /// See `docs/PHASE-X.A-PLAN.md` §6.3 for the full alignment story and the
    /// Bevy PR #6161 `Vec3` soundness postmortem that motivated rejecting
    /// per-row alignment promises.
    ///
    /// # Safety contract for callers
    ///
    /// Callers must ensure:
    /// 1. The index used to compute an offset is less than `self.count()`,
    ///    so the slot at that offset was written by `add` / `add_typed` and
    ///    is fully initialised.
    /// 2. The type `T` cast from the returned pointer matches the pool's
    ///    registered type (`component_layout().size() == size_of::<T>()` and
    ///    `component_layout().align() >= align_of::<T>()`). Use
    ///    `debug_assert_eq!` on both invariants at the call site.
    /// 3. No exclusive (`&mut`) access to the pool exists for the duration
    ///    of the reference derived from this pointer.
    #[inline]
    pub fn buffer_ptr(&self) -> *const u8 {
        // SAFETY: `NonNull::as_ptr` is always non-null. Casting to `*const u8`
        // drops mutability but the pointer provenance is preserved. This method
        // only returns the base; dereferencing individual slots is the caller's
        // responsibility (see safety contract in the doc comment above).
        self.buffer.as_ptr().cast_const()
    }

    /// Returns the WRITE-CAPABLE base pointer of the flat component buffer
    /// (Decision 4 / O1). The mutable counterpart of [`Self::buffer_ptr`].
    ///
    /// The base is **write-once and vm-reservation-stable** (Phase X.I): set
    /// in `new`, never moved — growth only commits fresh pages at the frontier
    /// of the SAME reservation, so a previously returned base stays valid
    /// across `grow_rows`. The typed batch-write path
    /// ([`ColumnPtr`](crate::ecs::core::bundle::ColumnPtr)) reads this base
    /// once per batch under a single `&mut` borrow of the pool bundle, then
    /// writes through it row-by-row without re-borrowing the pool.
    ///
    /// # Safety contract for callers
    ///
    /// Mirrors [`Self::buffer_ptr`] for write access:
    /// 1. A row index `r` used to compute `base.add(r * stride)` must satisfy
    ///    `r < self.committed_rows()` (the slot lies inside the committed
    ///    prefix). Reads of LIVE data additionally require `r < self.count()`.
    /// 2. The type `T` cast from a derived pointer must match the pool's
    ///    registered type (`component_layout().size() == size_of::<T>()` and
    ///    `component_layout().align() >= align_of::<T>()`).
    /// 3. The caller holds exclusive (`&mut`) access to the pool for the
    ///    duration of every write derived from this pointer (the typed batch
    ///    path resolves under `&mut`, ends that borrow, then writes through the
    ///    raw base while no other access path to the pool is live — W2).
    #[inline]
    pub fn buffer_ptr_mut(&mut self) -> *mut u8 {
        // SAFETY: `NonNull::as_ptr` is always non-null; the returned `*mut u8`
        // carries the pool's write-capable provenance. Dereferencing
        // individual slots is the caller's responsibility (see the safety
        // contract above; the write-once-stable-base promise is the Phase X.I
        // invariant documented on the `buffer` field).
        self.buffer.as_ptr()
    }

    /// `true` when the pool's reserve ceiling is exhausted
    /// (`len >= reserve_rows`). Phase X.I D5: committed capacity below the
    /// ceiling grows on demand and does NOT count as full.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.len >= self.reserve_rows
    }

    /// Rows remaining below the reserve ceiling (`reserve_rows - len`).
    #[inline]
    pub fn remaining_capacity(&self) -> usize {
        self.reserve_rows - self.len
    }

    // ── Phase 11 — Unit-pointer accessors + no-drop scaffolding ─────────────
    //
    // Wave E Step 12 (plan §7.2 / Round 3 C-N2). The migration paths in
    // `commands/migration_helpers.rs` need to read the raw row pointer
    // for row `idx` so they can build a `&[u8]` retained-bytes slice
    // *before* swapping the row out via `swap_remove_index_no_drop`. The
    // existing `get_raw` returns `Option<*const u8>` but with a non-trivial
    // borrow check signature; `unit_ptr` is the trivial inline alias used
    // exclusively by migration callers.

    /// Returns the raw row pointer for row `idx`. Panics in debug if
    /// `idx >= self.count()`.
    ///
    /// Used by Phase 11 archetype migrations to read source-row bytes
    /// before they are swap-removed (plan §7.2 retained-bytes extraction).
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn unit_ptr(&self, idx: usize) -> *const u8 {
        debug_assert!(idx < self.len, "unit_ptr: idx out of bounds");
        // SAFETY: idx < self.len ⇒ within the live, initialized range; the slot
        // was written by a prior add/add_typed.
        unsafe { self.row_ptr(idx).cast_const() }
    }

    /// Phase 11 W-N1 defensive check (plan §7.4): returns whether `idx`
    /// is a live row in this pool. Used by
    /// [`crate::ecs::core::component::component_pool_bundle::ComponentPoolBundle::has_pool`]
    /// and the `apply_replace_in_place` debug_assert site.
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn has_row(&self, idx: usize) -> bool {
        idx < self.len
    }

    /// Runs the registered `drop_fn` on the slot at `idx`. Logically
    /// uninitialises the bytes (the next `write_at` or
    /// `swap_remove_index_no_drop` rewrites them).
    ///
    /// # Safety (plan §7.3, C5)
    ///
    /// * `idx < self.count()` — debug-asserted.
    /// * Caller holds exclusive access via `&mut self`.
    /// * Caller will follow up with `write_at(idx, ...)` (replace-in-place)
    ///   or `swap_remove_index_no_drop(idx)` (migration); otherwise the
    ///   pool's `count()` continues to claim the slot as live, leading to
    ///   read-of-uninit on next access.
    #[allow(dead_code)]
    pub(crate) unsafe fn drop_at(&mut self, idx: usize) {
        debug_assert!(idx < self.len, "drop_at: idx out of bounds");
        if let Some(drop_fn) = self.drop_fn {
            // SAFETY: `idx < self.len` (debug-asserted) ⇒ the slot was written
            //   by a prior `add` / `add_typed` and contains a valid `T`.
            //   `&mut self` ⇒ exclusive access; the registered `drop_fn` is
            //   `unsafe fn(*mut u8)` (= `drop_in_place::<T>` under the hood via
            //   `register_layout::<T>`).
            unsafe { drop_fn(self.row_ptr(idx)) };
        }
    }

    /// Writes `bytes` into the slot at `idx`. The slot MUST be logically
    /// uninitialised (just after `drop_at`) — caller responsibility.
    ///
    /// # Safety (plan §7.4)
    ///
    /// * `idx < self.count()` — debug-asserted.
    /// * `bytes.len() == self.component_layout().size()` — debug-asserted.
    /// * The bytes form a valid representation of the pool's registered
    ///   type. (Mirrors the existing `set_component` raw-API contract.)
    /// * Caller holds exclusive access via `&mut self`.
    #[allow(dead_code)]
    pub(crate) unsafe fn write_at(&mut self, idx: usize, bytes: &[u8]) {
        debug_assert!(idx < self.len, "write_at: idx out of bounds");
        debug_assert_eq!(
            bytes.len(),
            self.component_layout.size(),
            "write_at: bytes.len() != layout.size()"
        );
        // SAFETY (mirrors `set_component`):
        //   * `idx < self.len` — slot is reachable.
        //   * `&mut self` ⇒ exclusive access.
        //   * Source (`bytes` — caller memory) and destination (the pool's
        //     reservation) are disjoint allocations; `copy_nonoverlapping`
        //     is sound (★R1-8: "disjoint" here is caller bytes vs pool, NOT
        //     intra-pool — data + tick regions share one allocated object).
        //   * Slot is logically uninit (caller contract); no drop runs.
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.row_ptr(idx),
                self.component_layout.size(),
            );
        }
    }

    /// Moves the value at `idx` out of the pool via a typed `ptr::read`,
    /// WITHOUT invoking the registered `drop_fn` (asset-streaming plan F1).
    ///
    /// The slot becomes logically uninitialised — exactly [`Self::drop_at`]'s
    /// post-condition — but instead of running drop glue in place and
    /// discarding the value, this moves it out for the caller to own. This is
    /// the primitive [`Assets::remove`](crate::ecs::core::asset::assets::Assets::remove)
    /// / [`Assets::add`](crate::ecs::core::asset::assets::Assets::add)'s
    /// slot-reuse path build on: a row that was `take_at`'d is exactly as
    /// "logically dead" as one that was `drop_at`'d, so [`Self::write_at`]
    /// may immediately overwrite it.
    ///
    /// # Safety
    ///
    /// * `idx < self.count()` — the slot must be live (initialised by a
    ///   prior `add` / `add_typed` / `write_at`) — debug-asserted.
    /// * `T` must match the pool's registered type exactly (size, alignment,
    ///   `TypeId`) — debug-asserted.
    /// * Caller holds exclusive access via `&mut self`.
    /// * Caller does not read or drop this slot again until it is rewritten
    ///   (`write_at`) — ownership of the value has moved to the caller via
    ///   the returned `T`; the pool's bookkeeping (`len`) is unchanged, so the
    ///   caller is responsible for whatever occupancy tracking (a `live`
    ///   bitmap, a free-list) makes this exactly-once (never re-read,
    ///   never re-dropped).
    #[allow(dead_code)]
    pub(crate) unsafe fn take_at<T: 'static>(&mut self, idx: usize) -> T {
        debug_assert!(idx < self.len, "take_at: idx out of bounds");
        debug_assert_eq!(
            self.component_type_id,
            TypeId::of::<T>(),
            "ComponentPool::take_at: T = {} does not match pool's registered type",
            std::any::type_name::<T>()
        );
        // SAFETY: `idx < self.len` (debug-asserted) ⇒ the slot was written by
        //   a prior `add` / `add_typed` / `write_at` and holds a valid,
        //   initialized `T` of the pool's registered type (the `TypeId`
        //   debug_assert above). `&mut self` ⇒ exclusive access. `ptr::read`
        //   performs a bitwise move-out WITHOUT running `T::drop` — unlike
        //   `drop_at`, which runs `drop_fn` in place and returns nothing. The
        //   caller now owns the returned value; the slot's bytes are a
        //   "moved-from" `T` (still readable as raw bytes, but no longer a
        //   valid live `T` per Rust's move semantics) until `write_at`
        //   rewrites them.
        unsafe { core::ptr::read(self.row_ptr(idx).cast::<T>()) }
    }

    /// Swap-removes row `idx` for byte storage + tick storage. NO
    /// `drop_fn` invocation on either source or last slot (W-N2 tightening
    /// of plan §7.2).
    ///
    /// Mirrors the existing [`Self::swap_remove`] flow over the dense byte
    /// buffer but skips drop.
    ///
    /// # Safety (plan §7.2)
    ///
    /// * `idx < self.count()` — debug-asserted.
    /// * Caller has ensured the source-row bytes were moved-out or
    ///   explicitly dropped (per the `move_out_entity` PRECONDITION).
    /// * Caller holds exclusive access via `&mut self`.
    #[allow(dead_code)]
    pub(crate) unsafe fn swap_remove_index_no_drop(&mut self, idx: usize) {
        debug_assert!(
            idx < self.len,
            "swap_remove_index_no_drop: idx out of bounds"
        );
        let last_index = self.len - 1;

        if idx != last_index {
            // SAFETY (mirrors existing `swap_remove` semantics minus the
            // drop):
            //   * idx < self.len and last_index < self.len, so both `row_ptr`
            //     results are valid committed-slot pointers produced by prior
            //     `add` / `add_typed`.
            //   * Non-overlapping: `idx != last_index`; for stride > 0 each
            //     slot is `component_layout.size()` bytes at distinct stride
            //     multiples of the same data sub-region — distinct row
            //     indices guarantee non-overlap. For stride == 0 (Phase 22
            //     tag pools) this is a zero-byte copy between equal dangling
            //     pointers — trivially non-overlapping and allowed.
            //   * W-N2: NO `drop_fn` invocation on either slot. Caller
            //     has already moved or dropped the bytes per the
            //     `move_out_entity` contract.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    self.row_ptr(last_index),
                    self.row_ptr(idx),
                    self.component_layout.size(),
                );
            }

            // Tick swap — mirrors the existing `swap_remove` block.
            // SAFETY: idx != last_index, both < self.len <= committed_rows
            //   (committed prefix of each tick sub-region).
            //   `&mut self` ⇒ exclusive access to the tick sub-regions;
            //   no concurrent reader exists per Phase 9 SCH3.
            unsafe {
                debug_assert!(
                    self.is_tracked(),
                    "ComponentPool::swap_remove (tick swap): tick access on an UNTRACKED pool \
                     (new_untracked) - the tick sub-regions are reserved, NOT committed"
                );
                let added = self.added_base.as_ptr();
                let changed = self.changed_base.as_ptr();
                *(*added.add(idx)).get() = *(*added.add(last_index)).get();
                *(*changed.add(idx)).get() = *(*changed.add(last_index)).get();
            }
        }
        // (idx == last_index): just decrement. No byte/tick movement needed.

        self.len -= 1;
    }

    /// Pops the last row without invoking `drop_fn` (plan §7.2 / C5).
    /// Used by [`crate::ecs::core::archetype::archetype::Archetype::move_out_entity`]
    /// when `removed_unit_index == last_unit_index`.
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn pop_entity_no_drop(&mut self) {
        debug_assert!(self.len != 0, "pop_entity_no_drop: pool empty");
        // W-N2: NO `drop_fn` invocation.
        self.len -= 1;
    }

    // ── Phase 10 STORE3 — tick accessors (Phase X.I: sub-region re-base) ────
    //
    // The per-row tick storage lives in the pool's OWN reservation as two
    // fixed sub-regions (`added_base` / `changed_base` — see the D1 field
    // docs). The bases are write-once and vm-reservation-stable: growth
    // commits fresh pages in place and never moves them — a STRICTLY
    // stronger promise than the old STORE2 "Box never reallocates" wording.

    /// Returns the base pointer of the per-row `added` tick sub-region.
    ///
    /// The pointer is valid for `self.committed_rows()` readable
    /// `UnsafeCell<Tick>` slots and stays stable for the pool's lifetime
    /// (write-once vm-reservation base — Phase X.I). `Added<C>::set_table_*`
    /// caches this base pointer in its `Fetch<'w>` and indexes per-row
    /// below `entity_count`.
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn added_ticks_ptr(&self) -> *const UnsafeCell<Tick> {
        debug_assert!(
            self.is_tracked(),
            "ComponentPool::added_ticks_ptr: tick access on an UNTRACKED pool \
             (new_untracked) - the tick sub-regions are reserved, NOT committed"
        );
        self.added_base.as_ptr().cast_const()
    }

    /// Returns the base pointer of the per-row `changed` tick sub-region.
    ///
    /// Same shape and lifetime contract as [`Self::added_ticks_ptr`].
    /// `Changed<C>::set_table_*` and `Mut<T>::deref_mut` both reach the
    /// sub-region through this pointer.
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn changed_ticks_ptr(&self) -> *const UnsafeCell<Tick> {
        debug_assert!(
            self.is_tracked(),
            "ComponentPool::changed_ticks_ptr: tick access on an UNTRACKED pool \
             (new_untracked) - the tick sub-regions are reserved, NOT committed"
        );
        self.changed_base.as_ptr().cast_const()
    }

    /// Writes the `added` tick for row `index`.
    ///
    /// Called on entity insertion (`Archetype::create_entity` → bundle
    /// push) with the world's current tick.
    ///
    /// # Safety
    ///
    /// * `index < self.count()` — the slot must be live (initialised by
    ///   a prior `add` / `add_typed`), which also places it inside the
    ///   committed prefix (`count() <= committed_rows`).
    /// * The caller holds exclusive write access to this `(archetype,
    ///   component)` per Phase 9 SCH3 (the scheduler's conflict graph
    ///   guarantees no concurrent reader of the same slot exists).
    #[inline]
    pub(crate) unsafe fn write_added_tick(&self, index: usize, tick: Tick) {
        debug_assert!(
            self.is_tracked(),
            "ComponentPool::write_added_tick: tick access on an UNTRACKED pool \
             (new_untracked) - the tick sub-regions are reserved, NOT committed"
        );
        debug_assert!(index < self.committed_rows);
        // SAFETY: caller asserts `index < self.count() <= committed_rows`,
        // so the slot lies in the committed prefix of the `added` tick
        // sub-region (in-bounds of the pool's reservation by the D1 layout
        // math), and Phase 9 SCH3 exclusivity on this `(archetype,
        // component)`. `UnsafeCell::get()` produces a `*mut Tick` to a
        // distinct memory location per row — adjacent-row writes from
        // sibling `par_iter` chunks are sound per Rust's abstract machine
        // (Round 2 C3).
        unsafe {
            *(*self.added_base.as_ptr().add(index)).get() = tick;
        }
    }

    /// Writes the `changed` tick for row `index`.
    ///
    /// Called on entity insertion (alongside [`Self::write_added_tick`]),
    /// on `set_component`, and on `Mut<T>::deref_mut` (Wave C). The plan
    /// §2.4 INIT3 path threads `current_tick` from
    /// `EcsMaster::create_entity`.
    ///
    /// # Safety
    ///
    /// Same conditions as [`Self::write_added_tick`].
    #[inline]
    pub(crate) unsafe fn write_changed_tick(&self, index: usize, tick: Tick) {
        debug_assert!(
            self.is_tracked(),
            "ComponentPool::write_changed_tick: tick access on an UNTRACKED pool \
             (new_untracked) - the tick sub-regions are reserved, NOT committed"
        );
        debug_assert!(index < self.committed_rows);
        // SAFETY: caller asserts `index < self.count() <= committed_rows`
        // (committed prefix of the `changed` tick sub-region) and Phase 9
        // SCH3 exclusivity. Per-row `UnsafeCell<Tick>` is a distinct memory
        // location (Round 2 C3).
        unsafe {
            *(*self.changed_base.as_ptr().add(index)).get() = tick;
        }
    }

    /// Reads the `added` tick for row `index`.
    ///
    /// # Safety
    ///
    /// * `index < self.count()`.
    /// * The caller holds at least shared access to this `(archetype,
    ///   component)` per Phase 9 SCH3 — no concurrent writer is active.
    #[allow(dead_code)]
    #[inline]
    pub(crate) unsafe fn read_added_tick(&self, index: usize) -> Tick {
        debug_assert!(
            self.is_tracked(),
            "ComponentPool::read_added_tick: tick access on an UNTRACKED pool \
             (new_untracked) - the tick sub-regions are reserved, NOT committed"
        );
        debug_assert!(index < self.committed_rows);
        // SAFETY: caller asserts `index < self.count() <= committed_rows`
        // (committed prefix of the `added` tick sub-region) and Phase 9
        // SCH3 (at least shared access — no writer). The dereferenced
        // value is `Copy`.
        unsafe { *(*self.added_base.as_ptr().add(index)).get() }
    }

    /// Reads the `changed` tick for row `index`.
    ///
    /// # Safety
    ///
    /// Same conditions as [`Self::read_added_tick`].
    #[allow(dead_code)]
    #[inline]
    pub(crate) unsafe fn read_changed_tick(&self, index: usize) -> Tick {
        debug_assert!(
            self.is_tracked(),
            "ComponentPool::read_changed_tick: tick access on an UNTRACKED pool \
             (new_untracked) - the tick sub-regions are reserved, NOT committed"
        );
        debug_assert!(index < self.committed_rows);
        // SAFETY: caller asserts `index < self.count() <= committed_rows`
        // (committed prefix of the `changed` tick sub-region) and Phase 9
        // SCH3 (at least shared access — no writer).
        unsafe { *(*self.changed_base.as_ptr().add(index)).get() }
    }

    /// Dense plan D4: copies the `(added, changed)` tick pair from row `src` to
    /// row `dst`, leaving `src`'s ticks unchanged.
    ///
    /// Used by [`DenseStore::compact`](crate::ecs::core::component::dense::DenseStore)
    /// to keep each live slot's change-detection ticks travelling with its data
    /// when the slot is relocated down to its canonical index — the tick storage
    /// is slot-indexed (parallel to the data rows), so a data relocation that did
    /// not also move the ticks would leave the compacted slot reading a stale
    /// tick (a change-detection correctness bug).
    ///
    /// # Safety
    ///
    /// * `src < self.committed_rows` and `dst < self.committed_rows`.
    /// * Caller holds exclusive access via `&mut self`.
    #[allow(dead_code)]
    #[inline]
    pub(crate) unsafe fn move_ticks(&mut self, src: usize, dst: usize) {
        debug_assert!(
            self.is_tracked(),
            "ComponentPool::move_ticks: tick access on an UNTRACKED pool \
             (new_untracked) - the tick sub-regions are reserved, NOT committed"
        );
        debug_assert!(src < self.committed_rows);
        debug_assert!(dst < self.committed_rows);
        // SAFETY: both indices are `< committed_rows` (debug-asserted), so both
        //   tick slots lie in the committed prefix of each tick sub-region.
        //   `&mut self` ⇒ exclusive access; `Tick` is `Copy` (a plain `u32`).
        unsafe {
            let added = self.added_base.as_ptr();
            let changed = self.changed_base.as_ptr();
            *(*added.add(dst)).get() = *(*added.add(src)).get();
            *(*changed.add(dst)).get() = *(*changed.add(src)).get();
        }
    }

    // ── Phase 12.5 Opt-A2 — batch reserve / write accessors (C-N1) ──────────
    //
    // §5.6 of the spawn-optimisations plan. The batch path reserves
    // capacity, writes payload bytes directly into pre-validated pool
    // slots, then commits the rows (advancing `len`) and stamps
    // `(added, changed)` ticks in tight loops. All accessors are
    // `pub(crate)` — consumed exclusively by `Archetype::reserve_capacity`,
    // `SpawnBatchCommand::apply`, and
    // `ComponentPoolBundle::commit_units_batch` / `fill_ticks_batch`.

    /// Phase 12.5 Opt-A2 (C-N1) / Phase X.I D5: returns `true` iff `n` more
    /// rows fit under the reserve ceiling (`count + n <= reserve_rows`).
    ///
    /// Cheap inline check used by `Archetype::reserve_capacity` Phase A to
    /// pre-validate the entire bundle before any pool is mutated (two-phase
    /// contract; mirrors `can_push_entity_components`). This is a pure
    /// CEILING check — committed capacity is grown later by Phase B's
    /// unconditional `grow_rows` calls.
    #[inline]
    pub(crate) fn can_reserve(&self, n: usize) -> bool {
        self.len
            .checked_add(n)
            .is_some_and(|end| end <= self.reserve_rows)
    }

    /// Phase 12.5 Opt-A2 (C-N1): returns `(current_count, reserve_rows)`
    /// for diagnostic / error-reporting paths
    /// (`EcsError::ArchetypePoolCapacityExceeded` — the second element is
    /// the pool's reserve ceiling in rows).
    #[inline]
    pub(crate) fn len_for_reserve(&self) -> (usize, usize) {
        (self.len, self.reserve_rows)
    }

    /// Phase 12.5 Opt-A2 (SBO13 / §5.6): writes `bytes` into the slot at
    /// `idx` WITHOUT advancing `len`, WITHOUT capacity checks, and
    /// WITHOUT invoking any drop (the slot is logically uninit).
    ///
    /// The batch path uses this for every row in `[start_row, start_row + n)`
    /// after `reserve_capacity` has grown + validated the range and before
    /// `commit_units` advances `self.len`. Slot bookkeeping (`len`) is
    /// deferred to [`Self::commit_units`].
    ///
    /// # Safety
    ///
    /// * `idx < committed_rows` — caller pre-grew via
    ///   `Archetype::reserve_capacity` (Phase X.I: Phase B committed the
    ///   rows).
    /// * `idx >= self.len` (i.e. the slot is uninit and not yet committed).
    ///   After the matching `commit_units(start_row, n)` call the slot
    ///   becomes addressable.
    /// * `bytes.len() == self.component_layout().size()` — debug-asserted.
    /// * `bytes` forms a valid representation of the pool's registered
    ///   type (raw-API contract identical to `write_at`).
    /// * Caller holds exclusive `&mut self` access.
    #[inline]
    pub(crate) unsafe fn write_at_unchecked_initialized(
        &mut self,
        idx: usize,
        bytes: &[u8],
    ) {
        debug_assert!(
            idx < self.committed_rows,
            "write_at_unchecked_initialized: idx {} >= committed_rows {} \
             (callers pre-grow via reserve_capacity)",
            idx,
            self.committed_rows
        );
        debug_assert_eq!(
            bytes.len(),
            self.component_layout.size(),
            "write_at_unchecked_initialized: bytes.len() != layout.size()"
        );
        // SAFETY (mirrors `add` / `write_at`):
        //   * `idx < committed_rows` ⇒ `row_ptr` addresses a committed slot
        //     within the pool's reservation (this slot is not yet live).
        //   * Source (caller stack) and destination (the pool's
        //     reservation) live in disjoint allocations;
        //     `copy_nonoverlapping` is sound.
        //   * `&mut self` ⇒ exclusive access.
        //   * The slot is logically uninit by the caller's pre-reserve
        //     contract; no drop runs.
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.row_ptr(idx),
                self.component_layout.size(),
            );
        }
    }

    /// Required components (Feature 1, D5): runs the capture-free `ctor` to
    /// construct one value of this pool's registered type directly into the
    /// reserved-but-uncommitted slot `idx`. Mirrors
    /// [`Self::write_at_unchecked_initialized`] but materializes the value via a
    /// constructor function pointer instead of a memcpy — used by the
    /// constructor pass for a required component that the user bundle does not
    /// supply.
    ///
    /// # Safety
    ///
    /// * `idx < committed_rows` — caller pre-grew via
    ///   `Archetype::reserve_capacity` (Phase X.I committed the rows).
    /// * The slot at `idx` holds NO LIVE VALUE. Two shapes satisfy this, and the
    ///   bullet named only the first until KE14 D5:
    ///   1. `idx == self.len` — a fresh frontier slot, made addressable by the
    ///      matching `commit_units(idx, 1)` afterwards; or
    ///   2. `idx < self.len` where the previous tenant was already `drop_at`-ed
    ///      and the slot not re-committed — `DenseStore`'s LIFO free-slot reuse,
    ///      a legitimate second caller whose slot is LOGICALLY uninitialised
    ///      without being POSITIONALLY past `len`.
    ///
    ///   `ctor`'s `ptr::write` drops nothing, so a live value at `idx` would be
    ///   LEAKED, not double-dropped: this requirement is about the leak and
    ///   about the caller's own `len` / free-list bookkeeping, not about memory
    ///   safety. The `debug_assert` below checks the bound that IS a safety
    ///   condition (`idx < committed_rows`).
    /// * `ctor` constructs a value whose layout matches this pool's registered
    ///   type — guaranteed by the registry: `ctor` came from `REQUIRES_ALL`
    ///   keyed by this column's `ComponentId`, the same id this pool was created
    ///   for.
    /// * Caller holds exclusive `&mut self` access.
    #[inline]
    pub(crate) unsafe fn construct_at_uninitialized(
        &mut self,
        idx: usize,
        ctor: crate::ecs::core::component::component_registry::RequiredCtor,
    ) {
        debug_assert!(
            idx < self.committed_rows,
            "construct_at_uninitialized: idx {} >= committed_rows {} \
             (callers pre-grow via reserve_capacity)",
            idx,
            self.committed_rows
        );
        // SAFETY:
        //   * `idx < committed_rows` ⇒ `row_ptr` addresses a committed slot
        //     within the pool's reservation (this slot is not yet live).
        //   * `ctor` writes exactly one value of this pool's registered type
        //     into `dst` (the registry pairs the ctor with this column's id),
        //     and `ptr::write` (inside the derive-generated ctor) does not drop
        //     the uninit destination.
        //   * `&mut self` ⇒ exclusive access; the slot is logically uninit by
        //     the caller's pre-reserve contract, so no drop runs.
        unsafe {
            ctor(self.row_ptr(idx));
        }
    }

    /// Feature 3 (clone): returns the raw `*mut u8` of the reserved (committed-
    /// capacity, not-yet-live) row `idx`, for a caller that must WRITE THROUGH the
    /// pointer (a per-element `CloneFn`) rather than memcpy a `&[u8]`. Sibling of
    /// [`Self::write_at_unchecked_initialized`] / [`Self::construct_at_uninitialized`]
    /// — same pre-reserve contract, different write mechanism.
    ///
    /// # Safety
    /// * `idx < committed_rows` — the caller pre-grew via
    ///   `Archetype::reserve_capacity` (Phase X.I committed the rows).
    /// * The slot is logically uninit (not yet committed via `commit_units`); the
    ///   returned pointer is aligned to the pool's registered type and valid for
    ///   `component_layout().size()` bytes.
    /// * The caller holds exclusive `&mut self` access and writes exactly one value
    ///   of the registered type into the slot (without dropping the uninit prior
    ///   contents), then `commit_units(idx, 1)`.
    #[inline]
    pub(crate) unsafe fn reserved_row_ptr(&mut self, idx: usize) -> *mut u8 {
        debug_assert!(
            idx < self.committed_rows,
            "reserved_row_ptr: idx {} >= committed_rows {} (callers pre-grow via \
             reserve_capacity)",
            idx,
            self.committed_rows
        );
        // SAFETY: `idx < committed_rows` ⇒ `row_ptr` addresses a committed slot
        //   within the pool's reservation (this slot is not yet live). `&mut self`
        //   gives exclusive write access; the returned pointer carries the pool's
        //   own reservation provenance.
        unsafe { self.row_ptr(idx) }
    }

    /// Phase 12.5 Opt-A2 (§5.6): commits `n` rows starting at `start_row`
    /// (advancing `self.len`) after the batch path has written every row's
    /// bytes via [`Self::write_at_unchecked_initialized`].
    ///
    /// Pre: `start_row == self.count()` (the rows must land contiguously
    /// at the tail) and the caller pre-grew committed capacity via
    /// `Archetype::reserve_capacity`. Phase X.I D7 deleted the chunk
    /// bookkeeping loop — committing a batch is a single guarded length
    /// bump.
    #[inline]
    pub(crate) fn commit_units(&mut self, start_row: usize, count: usize) {
        // Defense-in-depth: callers (`SpawnBatchCommand::apply`)
        // early-return on `n == 0`, but the method must stay total.
        if count == 0 {
            return;
        }
        debug_assert_eq!(
            start_row,
            self.len,
            "commit_units: start_row {} != current count {} (rows must extend the tail)",
            start_row,
            self.len
        );
        debug_assert!(
            start_row + count <= self.committed_rows,
            "commit_units: range past committed_rows (callers pre-grow via reserve_capacity)"
        );

        // The per-row bytes were already written by the caller's
        // `write_at_unchecked_initialized` calls into the dense buffer
        // (rows `[start_row, start_row + count)`, which the debug_assert above
        // proves equals `[len, len + count)`). With the parallel `Vec<Unit>`
        // (Phase X.B) and the chunk dirty marks (Phase X.I) both removed,
        // committing the batch is a single length bump — the rows are now
        // addressable via `row_ptr`.
        self.len += count;
    }

    /// Phase 12.5 Opt-A2 (§5.6 / STORE4): writes `tick` into both the
    /// `added` and `changed` tick slots for every row in
    /// `[start_row, start_row + count)`.
    ///
    /// Vectorisable: the sub-regions are dense `UnsafeCell<Tick>` arrays
    /// and `UnsafeCell<Tick>` is `#[repr(transparent)]` over `Tick`
    /// (4 B `u32`). The compiler lowers the inner loop to a
    /// SIMD-friendly streaming write.
    ///
    /// Phase 12.6 — `#[inline]` so the count=1 caller
    /// (`SpawnAtCommand::apply`) inlines the body and the compiler folds
    /// the loop down to two unchecked-cell stores.
    #[inline]
    pub(crate) fn fill_ticks(&mut self, start_row: usize, count: usize, tick: Tick) {
        debug_assert!(
            self.is_tracked(),
            "ComponentPool::fill_ticks: tick access on an UNTRACKED pool \
             (new_untracked) - the tick sub-regions are reserved, NOT committed"
        );
        // Defense-in-depth: skip the entire body on a zero-count call.
        // Mirrors the `commit_units` guard above; keeps the public API
        // total even for callers that have not pre-filtered `n == 0`.
        if count == 0 {
            return;
        }
        debug_assert!(
            start_row + count <= self.committed_rows,
            "fill_ticks: range past committed_rows (callers pre-grow via reserve_capacity)"
        );
        // SAFETY (STORE4 + SCH3):
        //   * Range `[start_row, start_row + count)` is within the committed
        //     prefix of both tick sub-regions (debug-asserted above; callers
        //     pre-grew via `reserve_capacity` / inline `grow_rows`).
        //   * `&mut self` ⇒ exclusive write access; per-row `UnsafeCell<Tick>`
        //     is a distinct memory location per Rust's abstract machine.
        unsafe {
            let added_base = self.added_base.as_ptr();
            let changed_base = self.changed_base.as_ptr();
            for i in 0..count {
                *(*added_base.add(start_row + i)).get() = tick;
                *(*changed_base.add(start_row + i)).get() = tick;
            }
        }
    }

    /// Flips a freshly-constructed (empty, `len == 0`) Host pool's backing to a
    /// `Device` arm wrapping `handle` (Phase 5 MF-2 / O1).
    ///
    /// This is the real device-mint primitive that `boyko_render` reaches through
    /// the archetype-level funnel [`Archetype::make_component_device_backed`]
    /// (Wave B). It is `#[cfg(not(miri))]` (aligned with [`DeviceColumn`], NOT
    /// `cfg(test)`): Miri cannot run the RHI syscalls a real device column needs,
    /// and `PoolBacking::Device` is compiled out under Miri.
    ///
    /// # O1 — release data-loss guard
    ///
    /// Carries a **release** `assert!(self.len == 0)`: switching a populated pool
    /// to `Device` would silently leak the live Host rows (overwriting `backing`
    /// drops the Host `VmReservation`). A device pool then keeps Host `len == 0`
    /// for life (CR-C), so the assert is a partition-integrity guard, not a
    /// hot-path check.
    ///
    /// **Dangling Host bases (FIX-8 / PB-2).** Overwriting `backing` releases the
    /// Host `VmReservation`, so the three write-once base pointers `buffer`,
    /// `added_base`, and `changed_base` become DANGLING after this flip — they
    /// are NOT cleared. Soundness rests on two guards (C1 / SEND10): (1) the
    /// Query-path skip (`update_archetypes` skips GPU-resident archetypes so
    /// `row_ptr` on a device pool is never reached), and (2) the archetype-level
    /// funnel NULLs `columns[cid]` so every direct-access null-check returns
    /// `None`/`false`/skip. With `len == 0` no base-reading accessor
    /// (`row_ptr`, the tick-fill loops, `Drop`'s per-row walk) ever dereferences
    /// a base pointer regardless. A future maintainer MUST NOT add a post-flip
    /// read of `buffer` / `added_base` / `changed_base`.
    ///
    /// `#[allow(dead_code)]`: the production caller is `boyko_render`'s device
    /// mint (Wave B), not yet in tree — same Phase-5 forward-seam discipline as
    /// the `DeviceColumn` accessors. Exercised in tests.
    #[cfg(not(miri))]
    #[allow(dead_code)]
    pub(crate) fn make_device_backed(&mut self, handle: u64) {
        // O1: release-present data-loss guard — only an empty pool may flip.
        assert_eq!(
            self.len, 0,
            "make_device_backed: only an empty pool may switch to Device backing (CR-C / O1)"
        );
        use crate::ecs::memory::device_column::{DeviceColumn, DeviceColumnHandle};
        self.backing = PoolBacking::Device(Box::new(DeviceColumn::new(DeviceColumnHandle(handle))));
        // CR-C post-condition: a device pool keeps Host `len == 0` for life.
        debug_assert!(
            self.backing.is_device() && self.len == 0,
            "make_device_backed post-condition: backing is Device with Host len == 0"
        );
    }

    /// Test-only thin wrapper over [`Self::make_device_backed`] (Phase 4 Seam 3
    /// CR-C coverage), kept so existing tests keep their call name.
    #[cfg(all(test, not(miri)))]
    pub(crate) fn make_device_backed_for_test(&mut self, handle: u64) {
        self.make_device_backed(handle);
    }

    /// Overwrites the device handle on a `Device`-backed pool (Phase 5 MF-2/3).
    ///
    /// Called by `boyko_render`'s `grow_column` after it reallocs the device
    /// column and mints a NEW handle. **MF-3:** this mutates ONLY the boxed
    /// [`DeviceColumn`]'s handle — DISTINCT from the write-once `buffer` /
    /// `added_base` / `changed_base` (which dangle after the device flip) — so it
    /// violates no base-pointer invariant, and it does NOT call
    /// `grow_rows` / `host_vm_mut` (the `unreachable!` Host-only grow arm stays
    /// unreachable). A no-op on a Host pool (defensive — `boyko_render` only calls
    /// it on a device pool); the device grow keeps Host `len == 0` (debug-asserted).
    ///
    /// `#[allow(dead_code)]`: consumed by `boyko_render`'s `grow_column` (Wave B),
    /// not yet in tree — the Phase-5 forward-seam discipline.
    #[cfg(not(miri))]
    #[allow(dead_code)]
    pub(crate) fn set_device_handle(&mut self, handle: crate::ecs::memory::device_column::DeviceColumnHandle) {
        debug_assert!(
            self.len == 0,
            "set_device_handle: a Device pool keeps Host len == 0 (CR-C); got len = {}",
            self.len
        );
        if let PoolBacking::Device(dc) = &mut self.backing {
            dc.set_handle(handle);
        }
    }

    /// Returns the device handle of a `Device`-backed pool, or `None` for a Host
    /// pool (Phase 5 MF-2/3).
    ///
    /// `#[allow(dead_code)]`: read by `boyko_render`'s frame-path resolve (Wave
    /// B), not yet in tree — the Phase-5 forward-seam discipline.
    #[cfg(not(miri))]
    #[allow(dead_code)]
    pub(crate) fn device_handle(&self) -> Option<crate::ecs::memory::device_column::DeviceColumnHandle> {
        match &self.backing {
            PoolBacking::Host(_) => None,
            PoolBacking::Device(dc) => Some(dc.handle()),
        }
    }
}

// SAFETY (SEND10 — Phase 9 §2.4, §9.1, §11.3 + Phase 10 STORE3 / Round 2 C3
// + Phase X.I growth):
//
// `ComponentPool` becomes `Send + Sync` under the Phase 9 contract:
//
//   - Pool reads (component access on the Query iteration path) take
//     non-overlapping byte ranges between parallel systems, enforced by the
//     scheduler's `ConflictGraph` (SCH3) on the declared `Access` surface.
//     Two concurrently running systems never hold mutable references into the
//     same `ComponentPool` byte range.
//   - Pool growth (`grow_rows` — frontier commits on the pool's OWN
//     reservation) is plain `&mut self` field mutation, reachable only
//     through `&mut` paths: the owner's direct API, or the apply window
//     where SCH7 guarantees zero workers in flight. The commit syscalls are
//     not global-allocator calls, so the ALLOC1 TLS guard does not see them
//     — the `&mut` exclusivity IS the guard (SEND10 bullet 3, realized by
//     Phase X.I). Because the base pointers never move, column/tick bases
//     captured by earlier fetches stay valid across growth; no concurrent
//     reader can observe a half-grown pool (no `&self` read path loads the
//     frontier fields).
//   - `len` / `committed_rows` stay plain `usize`: legal via the `&mut`
//     exclusivity above, NOT via address stability (the X.G D7 wording
//     discipline). Mutations occur only on `&mut self` paths (`add`, `pop`,
//     `swap_remove`, `set_component`, `grow_rows`); the dispatcher
//     serialises these under the apply window. Worker reads use `&self`
//     entry points (`get_raw`, `buffer_ptr`, `count`).
//   - Phase 10 / X.I: the per-row tick storage lives in two sub-regions of
//     the pool's reservation, exposed as `UnsafeCell<Tick>` slots.
//     `UnsafeCell<Tick>` is `!Sync` on its own, but the pool exposes the
//     cells only through unsafe accessors (`write_added_tick`,
//     `write_changed_tick`, `read_added_tick`, `read_changed_tick`) whose
//     contract requires the caller hold the SCH3 exclusivity for writes
//     (or shared access for reads). Each `UnsafeCell<Tick>` is a distinct
//     memory location per Rust's abstract machine — adjacent-row writes
//     from `par_iter` chunks on the same cache line are sound (Round 2 C3
//     / Rustonomicon §"Data Races and Race Conditions"). The MESI
//     cache-line ping-pong is a perf cost, not UB.
//   - Phase 4 Seam 3 (IM-5) + Phase 5 (C1): the `backing: PoolBacking` field is
//     `Send + Sync` in both arms. `Host(VmReservation)` carries the same
//     `NonNull<u8>` + `usize` the old `vm` field did (the discipline above is
//     unchanged). `Device(Box<DeviceColumn>)` carries a `Copy` POD `u64`
//     `DeviceColumnHandle` + two `usize` counters: the handle is never a
//     pointer the CPU dereferences, the device backing is never touched
//     concurrently by CPU code, and a device pool keeps Host `len == 0` (CR-C)
//     so there is no Host-side aliasing. `DeviceColumn: Send + Sync` is
//     independently witnessed by `device_column::_assert`.
//
//     After `make_device_backed` flips a Host pool to `Device`, the freed Host
//     `VmReservation`'s three write-once base pointers (`buffer` / `added_base`
//     / `changed_base`) DANGLE but stay non-null. They are proven CPU-unreachable
//     by BOTH Phase-5 guards (C1, the device-mint contract):
//       (1) the QUERY-PATH SKIP — `QueryState::update_archetypes` skips every
//           `is_gpu_resident()` archetype at collection time, so the hot
//           `row_ptr` / `for_each_chunk` readers never see a device pool; AND
//       (2) the DIRECT-ACCESS NULL-COLUMN — the archetype-level funnel
//           `Archetype::make_component_device_backed` NULLs `columns[cid]`, so
//           every direct reader's existing null-check returns `None`/`false`/skip.
//     Enumerated direct readers, ALL covered by guard (2):
//     `EcsMaster::get_component_raw{,_mut}`, `get_component{,_mut}`,
//     `set_component_raw` (via the mut path), `has_component`,
//     `get_components_raw{,_mut}`. `query_entities` reads no `columns[].ptr`
//     (it exposes only entity handles). With `len == 0` no base-reading accessor
//     dereferences a base pointer regardless of the guards.
unsafe impl Send for ComponentPool {}
unsafe impl Sync for ComponentPool {}

impl Drop for ComponentPool {
    // PANIC POLICY:
    // Each `drop_fn(ptr)` call may panic if the user's `T::drop` panics.
    // Per the `Component` trait's `# Panic safety` doc-section, this is
    // forbidden by contract. If it happens during normal teardown, the panic
    // propagates to the caller and any remaining slots in this pool leak —
    // their Drop is not invoked because the loop aborts on first panic.
    // If it happens during stack unwinding (a second panic), the Rust runtime
    // aborts the process.
    //
    // We deliberately do NOT wrap each call in `catch_unwind`:
    //   - cost: ~20-30 ns per slot × thousands of slots × pools per master
    //     = measurable teardown delay for a contractually impossible event;
    //   - benefit: marginal — a user who violates the contract has already
    //     exhibited a logic bug.
    fn drop(&mut self) {
        // CR-C: a Device pool keeps Host `self.len == 0` for life — the device
        // row count lives in `PoolBacking::Device(..).device_len`. So the
        // `drop_fn` loop over `[0, len)` below is a no-op for a device pool and
        // never `drop_in_place`s over uninitialized / device-resident bytes.
        // Device teardown is `DeviceColumn::drop` (Phase 5: RHI release),
        // reached via the boxed `Device` arm's field drop, NOT the CPU `drop_fn`.
        debug_assert!(
            !self.backing.is_device() || self.len == 0,
            "CR-C: a Device ComponentPool must keep Host len == 0 (len = {})",
            self.len
        );
        if let Some(drop_fn) = self.drop_fn {
            // SAFETY:
            // - Rows `[0, self.len)` are all live and initialized per the pool's
            //   invariant (every slot up to `self.len` was written by add or
            //   add_typed before `self.len` was incremented), and
            //   `len <= committed_rows` keeps every `row_ptr(row)` inside
            //   committed read/write pages.
            // - Each `row_ptr(row)` points at a properly-aligned, T-sized,
            //   T-typed slot (pool construction invariant); `row < len`
            //   satisfies `row_ptr`'s safety contract.
            // - We have exclusive access (Drop receives &mut self).
            // - drop_fn matches the signature unsafe fn(*mut u8) and calls
            //   drop_in_place::<T> which is valid for these initialized slots.
            // - Phase 22 (stride == 0): a ZST with a Drop impl is legal
            //   (`needs_drop` true). Every `row_ptr(row)` is the dangling
            //   aligned base; `drop_in_place::<T>` for a ZST reads no bytes,
            //   so one call per logical live row at the shared dangling
            //   address is sound — `len` bounds the call count exactly.
            for row in 0..self.len {
                unsafe { drop_fn(self.row_ptr(row)) }
            }
        }
        // The backing itself is released by the `backing` field's Drop
        // (declared LAST in the struct): this body runs BEFORE field drops, so
        // release happens strictly after the last use. For `Host` the
        // `VmReservation` arm releases the reservation (V-DROP releases partially
        // committed reservations in full; M-001 per-arm deallocator lives inside
        // `VmReservation`); for `Device` (Phase 5) the boxed `DeviceColumn`'s
        // drop releases the device column.
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::ComponentPool;
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::identifiers::primitives::ComponentId;

    // ID allocation (no collision with integration test files or other unit tests):
    //   component_registry unit tests: 450..466, 498, 499
    //   drop_fn integration:           200..207
    //   drop_safety integration:       480..481
    //   typed-read tests below:        220..223
    //   Phase X.B dense-equivalence tests below: 224..226
    //   Phase X.I growth tests below:  226, 227
    //   Phase 22 ZST (tag) pool tests below: 228, 229
    //   packing-plan commit-floor oracle (`commit_floor_tests` below):
    //     127..=130, 162, 176, 188..=193, 230..=293, 318, 319, 382..=385,
    //     447..=449 — the census and the reasons are in that module's header
    const POS_ID: ComponentId = ComponentId(220);
    const VEL_ID: ComponentId = ComponentId(221);
    const OTHER_ID: ComponentId = ComponentId(222);
    const F32_WRAP_ID: ComponentId = ComponentId(223);
    // Phase X.B: a u64-payload component for the dense-pointer + oracle tests
    // (a stride that is a clean power-of-2 makes the `buffer + i*stride`
    // address arithmetic in `dense_equivalence` trivially auditable).
    const U64_ID: ComponentId = ComponentId(224);
    // Phase X.B: a drop-counting component for `drop_count_exact`.
    const DROPPER_ID: ComponentId = ComponentId(225);

    // ---- component type definitions ------------------------------------------------

    #[repr(C)]
    struct Position {
        x: f32,
        y: f32,
        z: f32,
    }

    #[repr(C)]
    struct Velocity {
        vx: f32,
        vy: f32,
        vz: f32,
    }

    /// A distinct type used solely for the TypeId-mismatch panic test.
    #[repr(C)]
    struct OtherComponent {
        val: u64,
    }

    /// Phase X.A SIMD-A1 fixture: a small-aligned (`align_of::<F32Wrap>() = 4`)
    /// component used to exercise the SIMD-buffer-alignment lift. The wrapper
    /// is `#[repr(transparent)]` over `f32`, so its alignment is exactly
    /// `align_of::<f32>() = 4` — far below `SIMD_BUFFER_ALIGN = 32`. The
    /// alignment-lift path must round the buffer alignment up to 32; without
    /// the lift, the buffer would be only 4-byte-aligned.
    #[repr(transparent)]
    struct F32Wrap(#[allow(dead_code)] f32);

    // ---- Component impls (mirrors what #[derive(Component)] generates) -------------

    impl Component for Position {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| {
                component_registry::register_layout::<Position>(POS_ID.0);
                POS_ID
            })
        }
    }

    impl Component for Velocity {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| {
                component_registry::register_layout::<Velocity>(VEL_ID.0);
                VEL_ID
            })
        }
    }

    impl Component for OtherComponent {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| {
                component_registry::register_layout::<OtherComponent>(OTHER_ID.0);
                OTHER_ID
            })
        }
    }

    impl Component for F32Wrap {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| {
                component_registry::register_layout::<F32Wrap>(F32_WRAP_ID.0);
                F32_WRAP_ID
            })
        }
    }

    // ---- helpers -------------------------------------------------------------------

    fn register_all() {
        component_registry::register_layout::<Position>(POS_ID.0);
        component_registry::register_layout::<Velocity>(VEL_ID.0);
        component_registry::register_layout::<OtherComponent>(OTHER_ID.0);
        component_registry::register_layout::<F32Wrap>(F32_WRAP_ID.0);
    }

    fn make_position_pool(cap: usize) -> ComponentPool {
        register_all();
        ComponentPool::new(POS_ID.0, cap)
    }

    // ---- tests (audit C-004 typed read wrappers) -----------------------------------

    /// `get_typed` must return the exact field values that were inserted via `add_typed`.
    #[test]
    fn get_typed_returns_inserted_value() {
        register_all();
        let mut pool = make_position_pool(4);

        let index = pool
            .add_typed(Position { x: 1.0, y: 2.0, z: 3.0 })
            .expect("pool has capacity for 1 element");

        let got = pool.get_typed::<Position>(index).expect("index 0 must be in bounds");
        assert_eq!(got.x, 1.0, "x must round-trip through the pool");
        assert_eq!(got.y, 2.0, "y must round-trip through the pool");
        assert_eq!(got.z, 3.0, "z must round-trip through the pool");
    }

    /// `get_mut_typed` must allow in-place mutation; the updated value must be
    /// visible via a subsequent `get_typed` call.
    #[test]
    fn get_mut_typed_round_trip() {
        register_all();
        let mut pool = make_position_pool(4);

        let index = pool
            .add_typed(Position { x: 0.0, y: 0.0, z: 0.0 })
            .expect("pool has capacity for 1 element");

        // Mutate in place.
        pool.get_mut_typed::<Position>(index)
            .expect("index 0 must be in bounds")
            .x = 99.0;

        // Re-read and confirm the mutation is visible.
        let got = pool.get_typed::<Position>(index).expect("index 0 must still be in bounds");
        assert_eq!(got.x, 99.0, "x must reflect the in-place mutation");
    }

    /// `get_typed` on an out-of-bounds index must return `None` without panicking.
    /// (The TypeId check is on the type parameter, not the bounds — bounds are
    /// handled by `get_raw` which returns `None`.)
    #[test]
    fn get_typed_out_of_bounds_returns_none() {
        register_all();
        let pool = make_position_pool(4);

        // Pool is empty; index 0 is out of bounds.
        assert!(
            pool.get_typed::<Position>(0).is_none(),
            "get_typed on empty pool must return None"
        );
    }

    /// Passing a type whose `TypeId` does not match the pool's registered type
    /// must fire a `debug_assert` in debug builds.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "does not match pool's registered type")]
    fn get_typed_wrong_type_panics_in_debug() {
        register_all();
        // Pool is registered for `Position` (POS_ID).
        let mut pool = ComponentPool::new(POS_ID.0, 4);

        // Insert a valid Position so that index 0 exists.
        pool.add_typed(Position { x: 1.0, y: 2.0, z: 3.0 })
            .expect("pool must accept first element");

        // Attempt to read as `OtherComponent` — TypeId mismatch must fire debug_assert.
        let _ = pool.get_typed::<OtherComponent>(0);
    }

    /// Phase X.A SIMD-A1 (plan §6.2, §12 Step 1A): every `ComponentPool`
    /// backing buffer must start on a `SIMD_BUFFER_ALIGN`-aligned address so
    /// that `Query::for_each_chunk`'s inner loops can emit AVX2 aligned loads
    /// from the column base without an unaligned-prologue.
    ///
    /// Phase X.I note: the original X.A scenario (a shared-arena cursor
    /// left misaligned by a preceding pool, corrected by the constructor's
    /// alignment lift) is dead — pools own their reservations (the shared
    /// Arena itself was retired in Phase X.J). Post-X.I the assertion is
    /// trivially true: every arm's reservation base is >= 4096-aligned
    /// (VirtualAlloc 64 KiB / mmap 4 KiB / fallback `Layout` align 4096),
    /// far above `SIMD_BUFFER_ALIGN = 32`. The test is kept as a TRIPWIRE
    /// for the SIMD-A1 contract: if a future storage change ever hands out
    /// a buffer base below 32-byte alignment, this fails loudly. The
    /// `_prefix` pool below is the historical non-tautology fixture,
    /// retained unchanged (test logic frozen).
    #[test]
    fn buffer_ptr_is_simd_aligned() {
        use crate::ecs::constants::SIMD_BUFFER_ALIGN;

        register_all();

        // Historical X.A fixture: pre-X.I this pool left the shared arena
        // cursor at a 16-mod-32 offset so the next pool's buffer would be
        // misaligned without the constructor's alignment lift. Post-X.I
        // every pool owns its reservation, so this no longer influences the
        // F32Wrap pool's base — retained to keep the test logic frozen.
        let _prefix = ComponentPool::new(POS_ID.0, 4);

        // Using the real `ComponentPool::new` keeps the tripwire wired to
        // the production base-pointer derivation.
        let pool = ComponentPool::new(F32_WRAP_ID.0, 4);

        let ptr = pool.buffer_ptr() as usize;
        assert!(
            ptr.is_multiple_of(SIMD_BUFFER_ALIGN),
            "ComponentPool<F32Wrap> buffer ptr {:#x} must be SIMD_BUFFER_ALIGN={}-byte aligned \
             for AVX2 column loads (Phase X.A SIMD-A1); offset = {}",
            ptr,
            SIMD_BUFFER_ALIGN,
            ptr % SIMD_BUFFER_ALIGN,
        );
    }

    // ====================================================================
    // Phase X.B — dense `Vec<Unit>` elimination: behavior-equivalence proofs.
    //
    // These tests pin the central refactor claim:
    //   `(the deleted Unit at row i).ptr()  ≡  buffer_ptr() + i * stride`
    // i.e. the row pointer that `ComponentPool` now *computes* on demand
    // (`row_ptr`) is byte-for-byte the address the parallel `Vec<Unit>`
    // used to cache. Every test below drives only the public / pub(crate)
    // surface — `add_typed` / `get_raw` / `get_typed` / `swap_remove` /
    // `pop` / `count` / `buffer_ptr` — so they verify observable behavior,
    // not internal representation.
    // ====================================================================

    /// A 16-byte component whose two fields make a moved value distinguishable
    /// from its destination slot. Used by the dense / swap / oracle tests.
    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Debug)]
    struct U64Pair {
        a: u64,
        b: u64,
    }

    impl Component for U64Pair {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| {
                component_registry::register_layout::<U64Pair>(U64_ID.0);
                U64_ID
            })
        }
    }

    fn make_u64_pool(reserve_rows: usize) -> ComponentPool {
        component_registry::register_layout::<U64Pair>(U64_ID.0);
        ComponentPool::new(U64_ID.0, reserve_rows)
    }

    /// Phase X.B core proof: after a mixed `add` + `swap_remove(mid)` + `add`
    /// sequence, every live row `i` satisfies
    /// `get_raw(i) == buffer_ptr() + i * stride` AND the round-tripped value
    /// matches a dense `Vec` oracle maintained with the same swap_remove rule.
    /// This is the exact identity the deleted `Unit.ptr()` cache used to hold.
    #[test]
    fn dense_equivalence() {
        // 16 slots; a mid-row swap moves a value across the whole buffer,
        // proving row_ptr spans it.
        let mut pool = make_u64_pool(16);
        let stride = pool.component_layout().size();
        assert_eq!(stride, 16, "U64Pair stride must be 16 for this test");

        // Mirror oracle: a dense Vec maintained with the same swap_remove rule.
        let mut oracle: Vec<U64Pair> = Vec::new();

        // Phase 1: add 10 distinguishable values.
        for i in 0..10u64 {
            let v = U64Pair { a: i, b: 1000 + i };
            pool.add_typed(v).expect("pool has capacity for 16");
            oracle.push(v);
        }

        // Phase 2: swap_remove a middle index (forces a cross-row memcpy).
        let mid = 3;
        assert!(pool.swap_remove(mid), "swap_remove(mid) in bounds");
        oracle.swap_remove(mid);

        // Phase 3: add 2 more after the hole was filled.
        for i in 100..102u64 {
            let v = U64Pair { a: i, b: 2000 + i };
            pool.add_typed(v).expect("pool still has capacity");
            oracle.push(v);
        }

        assert_eq!(
            pool.count(),
            oracle.len(),
            "pool count must track the oracle length after the mixed sequence"
        );

        let base = pool.buffer_ptr() as usize;
        // `i` indexes the pool row (`get_raw(i)`), the `i*stride` address math, AND
        // the oracle — a genuine multi-index loop where the range form is clearest.
        #[allow(clippy::needless_range_loop)]
        for i in 0..pool.count() {
            // (1) ADDRESS identity: the computed row pointer equals the address
            //     the deleted Unit.ptr() would have held: buffer + i*stride.
            let raw = pool.get_raw(i).expect("row i is live") as usize;
            assert_eq!(
                raw,
                base + i * stride,
                "row {} pointer must equal buffer_ptr() + {}*{} (row_ptr ≡ Unit.ptr())",
                i,
                i,
                stride
            );

            // (2) VALUE identity: the bytes at that computed address round-trip
            //     to the oracle's value, proving the address points at the
            //     right live datum (not merely an in-bounds address).
            let got = pool.get_typed::<U64Pair>(i).expect("row i typed read");
            assert_eq!(
                *got, oracle[i],
                "row {} value must match the dense Vec oracle after swap_remove",
                i
            );
        }
    }

    /// `swap_remove(k)` on a middle index must: drop the hole's value, move the
    /// previously-last value into row `k`, decrement count, and leave every
    /// other live row byte-unchanged.
    #[test]
    fn swap_remove_moves_last_value_into_hole() {
        let mut pool = make_u64_pool(16);

        const N: u64 = 8;
        for i in 0..N {
            pool.add_typed(U64Pair { a: i, b: 10 + i })
                .expect("capacity 16 holds 8");
        }

        let last_val = *pool
            .get_typed::<U64Pair>((N - 1) as usize)
            .expect("last row live");
        let k = 2usize;
        let untouched_lo = *pool.get_typed::<U64Pair>(0).expect("row 0 live");
        let untouched_hi = *pool.get_typed::<U64Pair>(4).expect("row 4 live");

        assert!(pool.swap_remove(k), "swap_remove(2) in bounds");

        assert_eq!(
            pool.count(),
            (N - 1) as usize,
            "count must decrement by exactly one"
        );
        assert_eq!(
            *pool.get_typed::<U64Pair>(k).expect("hole now holds moved value"),
            last_val,
            "the previously-last value must now be readable at the hole index k"
        );
        // Rows outside k (and below the new len) must be byte-identical.
        assert_eq!(
            *pool.get_typed::<U64Pair>(0).expect("row 0 still live"),
            untouched_lo,
            "row 0 (in [0,k)) must be unchanged by swap_remove(k)"
        );
        assert_eq!(
            *pool.get_typed::<U64Pair>(4).expect("row 4 still live"),
            untouched_hi,
            "row 4 (in (k, last)) must be unchanged by swap_remove(k)"
        );
    }

    /// A drop-counting component to prove the new `Drop` loop `0..len` drops
    /// every live row exactly once and never touches the uninitialised
    /// `[len, committed_rows)` slots.
    #[repr(C)]
    struct Dropper {
        counter: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Drop for Dropper {
        fn drop(&mut self) {
            self.counter
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    impl Component for Dropper {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| {
                component_registry::register_layout::<Dropper>(DROPPER_ID.0);
                DROPPER_ID
            })
        }
    }

    /// Add M rows into a pool with spare capacity, `swap_remove` one
    /// (counter == 1), then drop the pool: counter must equal M — each
    /// remaining live row dropped exactly once, and NONE of the uninitialised
    /// `[len, committed_rows)` slots dropped (which would over-count).
    #[test]
    fn drop_count_exact() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        component_registry::register_layout::<Dropper>(DROPPER_ID.0);
        // Capacity 16, only 6 live → 10 uninit slots that must NOT be dropped.
        let mut pool = ComponentPool::new(DROPPER_ID.0, 16);

        let counter = Arc::new(AtomicUsize::new(0));
        const M: usize = 6;
        for _ in 0..M {
            pool.add_typed(Dropper {
                counter: Arc::clone(&counter),
            })
            .expect("capacity 16 holds 6");
        }
        assert_eq!(
            counter.load(Ordering::Relaxed),
            0,
            "no drops before any removal"
        );

        // swap_remove a middle row → exactly one drop of the removed value.
        assert!(pool.swap_remove(2), "swap_remove(2) in bounds");
        assert_eq!(
            counter.load(Ordering::Relaxed),
            1,
            "swap_remove must drop exactly the removed component"
        );

        // Drop the pool: the remaining M-1 live rows drop, total == M.
        drop(pool);
        assert_eq!(
            counter.load(Ordering::Relaxed),
            M,
            "pool Drop must drop each remaining live row exactly once \
             (total {M}); the uninit [len, max) slots must NOT be dropped"
        );
    }

    /// proptest oracle: drive a generated stream of `add` / `swap_remove` /
    /// `pop` ops against a `Vec<U64Pair>` reference. After every op, assert
    /// `count()` matches and every live row's value matches the oracle (whose
    /// `swap_remove` mirrors the pool's last-into-hole rule). This is the
    /// strongest evidence the *computed* row pointers behave identically to the
    /// deleted cached pointers across an arbitrary op sequence.
    mod oracle {
        use super::{U64Pair, U64_ID};
        use crate::ecs::core::component::component::Component as _;
        use crate::ecs::core::component::component_registry;
        use crate::ecs::memory::component_pool::ComponentPool;
        use proptest::prelude::*;

        #[derive(Clone, Debug)]
        enum Op {
            Add(u64),
            SwapRemove(usize),
            Pop,
        }

        fn op_strategy() -> impl Strategy<Value = Op> {
            prop_oneof![
                any::<u64>().prop_map(Op::Add),
                any::<usize>().prop_map(Op::SwapRemove),
                Just(Op::Pop),
            ]
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(64))]
            #[test]
            fn pool_matches_vec_oracle(ops in proptest::collection::vec(op_strategy(), 1..200)) {
                // Force registration before pool construction.
                let _ = U64Pair::component_id();
                component_registry::register_layout::<U64Pair>(U64_ID.0);

                // 256-row capacity — comfortably above the 200-op cap.
                let mut pool = ComponentPool::new(U64_ID.0, 256);
                let mut oracle: Vec<U64Pair> = Vec::new();

                for op in ops {
                    match op {
                        Op::Add(seed) => {
                            // Skip adds once the pool is full (the pool returns
                            // None; the oracle must mirror by not pushing).
                            if pool.count() < pool.capacity() {
                                let v = U64Pair { a: seed, b: seed ^ 0xA5A5_A5A5_A5A5_A5A5 };
                                let idx = pool.add_typed(v);
                                prop_assert_eq!(idx, Some(oracle.len()));
                                oracle.push(v);
                            }
                        }
                        Op::SwapRemove(raw_idx) => {
                            if oracle.is_empty() {
                                // Out-of-bounds remove must be a no-op (returns false).
                                prop_assert!(!pool.swap_remove(0));
                            } else {
                                let idx = raw_idx % oracle.len();
                                prop_assert!(pool.swap_remove(idx));
                                oracle.swap_remove(idx);
                            }
                        }
                        Op::Pop => {
                            let popped = pool.pop();
                            prop_assert_eq!(popped, oracle.pop().is_some());
                        }
                    }

                    // Invariant after every op: count + every live row's value.
                    prop_assert_eq!(pool.count(), oracle.len());
                    // multi-index: pool row (`get_typed(i)`) + oracle, by the same `i`.
                    #[allow(clippy::needless_range_loop)]
                    for i in 0..oracle.len() {
                        let got = pool.get_typed::<U64Pair>(i)
                            .expect("live row must read back");
                        prop_assert_eq!(*got, oracle[i],
                            "row value mismatch vs oracle at index {}", i);
                    }
                }
            }
        }
    }

    // ====================================================================
    // Phase X.B — the three spec-named behavior-equivalence GATES.
    //
    // These complement (do not replace) the dev-authored `dense_equivalence`
    // / `swap_remove_moves_last_value_into_hole` / `drop_count_exact` /
    // `oracle::pool_matches_vec_oracle` tests above by tightening them to the
    // exact contract the task brief enumerates:
    //   * Gate 1 asserts the oracle + address identity AFTER EVERY op across a
    //     multi-swap_remove interleaving (not just at the end);
    //   * Gate 2 adds `set_component(i, v)` to the proptest op alphabet;
    //   * Gate 3 adds the `swap_remove_index_no_drop` ZERO-drop assertion.
    // All three drive only the public / pub(crate) surface — the computed
    // `row_ptr` is never named, so they verify observable behavior.
    // ====================================================================

    /// Raw little-endian byte view of a `U64Pair` for the `add` / `set_component`
    /// raw-API paths.
    fn u64pair_bytes(p: &U64Pair) -> &[u8] {
        // SAFETY: `U64Pair` is `#[repr(C)]` POD (two `u64`); the slice spans
        // exactly `size_of::<U64Pair>()` initialized bytes.
        unsafe {
            std::slice::from_raw_parts(
                (p as *const U64Pair).cast::<u8>(),
                std::mem::size_of::<U64Pair>(),
            )
        }
    }

    /// Asserts the full substitution + value identity for every live row of
    /// `pool` against the dense `oracle`:
    ///   (1) `get_raw(i)` address == `buffer_ptr() + i * stride` (the deleted
    ///       `Unit.ptr()` identity), and
    ///   (2) `get_typed::<U64Pair>(i)` == `oracle[i]` (the moved-value identity).
    fn assert_pool_matches_oracle(pool: &ComponentPool, oracle: &[U64Pair], stride: usize) {
        assert_eq!(
            pool.count(),
            oracle.len(),
            "count must equal the oracle length"
        );
        let base = pool.buffer_ptr() as usize;
        // multi-index: pool row (`get_raw(i)`) + `i*stride` address + oracle, same `i`.
        #[allow(clippy::needless_range_loop)]
        for i in 0..oracle.len() {
            let raw = pool.get_raw(i).expect("live row i must yield a raw ptr") as usize;
            assert_eq!(
                raw,
                base + i * stride,
                "row {} address must equal buffer_ptr() + {}*{} (row_ptr ≡ Unit.ptr())",
                i,
                i,
                stride
            );
            let got = pool.get_typed::<U64Pair>(i).expect("live row i typed read");
            assert_eq!(*got, oracle[i], "row {} value must match the oracle", i);
        }
    }

    /// GATE 1 — `dense_equivalence_after_swap_remove`.
    ///
    /// Drives the exact brief sequence: add several rows, `swap_remove` a
    /// MIDDLE row, add more, `swap_remove` again — and after EVERY structural
    /// op asserts both the address identity and the value identity against a
    /// `Vec` oracle maintained with the same last-into-hole semantics. This
    /// proves the computed-pointer mapping equals the old stored-pointer
    /// mapping across an interleaving, not merely at a single terminal state.
    #[test]
    fn dense_equivalence_after_swap_remove() {
        // 16 slots: a mid-row swap exercises row_ptr over the whole buffer.
        let mut pool = make_u64_pool(16);
        let stride = pool.component_layout().size();
        assert_eq!(stride, 16, "U64Pair stride must be 16 for the address-identity math");

        let mut oracle: Vec<U64Pair> = Vec::new();

        // add 6 distinct rows; check after each.
        for i in 0..6u64 {
            let v = U64Pair { a: i, b: 0xF00D_0000 + i };
            let idx = pool.add_typed(v).expect("capacity 16 holds 6");
            oracle.push(v);
            assert_eq!(idx, oracle.len() - 1, "add must return the tail index");
            assert_pool_matches_oracle(&pool, &oracle, stride);
        }

        // swap_remove a MIDDLE row (index 2 of 0..6) — a real last-into-hole memcpy.
        assert!(pool.swap_remove(2), "swap_remove(2) in bounds");
        oracle.swap_remove(2);
        assert_pool_matches_oracle(&pool, &oracle, stride);

        // add 3 more after the hole was back-filled; check after each.
        for i in 100..103u64 {
            let v = U64Pair { a: i, b: 0xBEEF_0000 + i };
            pool.add_typed(v).expect("capacity 16 holds the regrowth");
            oracle.push(v);
            assert_pool_matches_oracle(&pool, &oracle, stride);
        }

        // swap_remove AGAIN at a different middle index (1 of the new 0..8).
        assert!(pool.swap_remove(1), "second swap_remove(1) in bounds");
        oracle.swap_remove(1);
        assert_pool_matches_oracle(&pool, &oracle, stride);

        // Drain via swap_remove(0) to empty; the identity must hold at every step
        // including the final single-row (trivial last-row) removal.
        while !oracle.is_empty() {
            assert!(pool.swap_remove(0), "swap_remove(0) while non-empty");
            oracle.swap_remove(0);
            assert_pool_matches_oracle(&pool, &oracle, stride);
        }
        assert_eq!(pool.count(), 0, "pool drained to empty");
    }

    /// GATE 2 — `proptest_pool_vs_vec_oracle`.
    ///
    /// A `proptest` over the op alphabet {`add`, `swap_remove(i)`, `pop`,
    /// `set_component(i, v)`} against a `Vec<U64Pair>` reference oracle (same
    /// last-into-hole `swap_remove` rule). After EVERY op: `count()` matches and
    /// every live row's value matches the oracle. This is the strongest evidence
    /// the computed pointers behave identically across arbitrary interleavings,
    /// and it adds the in-place-overwrite (`set_component`) path the dev oracle
    /// omitted. 64 cases bound runtime.
    mod gate2 {
        use super::{U64Pair, U64_ID, u64pair_bytes};
        use crate::ecs::core::component::component::Component as _;
        use crate::ecs::core::component::component_registry;
        use crate::ecs::memory::component_pool::ComponentPool;
        use proptest::prelude::*;

        #[derive(Clone, Debug)]
        enum Op {
            Add(u64),
            SwapRemove(usize),
            Pop,
            SetComponent(usize, u64),
        }

        fn op_strategy() -> impl Strategy<Value = Op> {
            prop_oneof![
                any::<u64>().prop_map(Op::Add),
                any::<usize>().prop_map(Op::SwapRemove),
                Just(Op::Pop),
                (any::<usize>(), any::<u64>())
                    .prop_map(|(i, v)| Op::SetComponent(i, v)),
            ]
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(64))]
            #[test]
            fn proptest_pool_vs_vec_oracle(
                ops in proptest::collection::vec(op_strategy(), 1..200)
            ) {
                let _ = U64Pair::component_id();
                component_registry::register_layout::<U64Pair>(U64_ID.0);

                // 256-row capacity > the 200-op cap.
                let mut pool = ComponentPool::new(U64_ID.0, 256);
                let mut oracle: Vec<U64Pair> = Vec::new();

                for op in ops {
                    match op {
                        Op::Add(seed) => {
                            if pool.count() < pool.capacity() {
                                let v = U64Pair { a: seed, b: !seed };
                                let idx = pool.add_typed(v);
                                prop_assert_eq!(idx, Some(oracle.len()));
                                oracle.push(v);
                            }
                        }
                        Op::SwapRemove(raw_idx) => {
                            if oracle.is_empty() {
                                prop_assert!(!pool.swap_remove(0));
                            } else {
                                let idx = raw_idx % oracle.len();
                                prop_assert!(pool.swap_remove(idx));
                                oracle.swap_remove(idx);
                            }
                        }
                        Op::Pop => {
                            let popped = pool.pop();
                            prop_assert_eq!(popped, oracle.pop().is_some());
                        }
                        Op::SetComponent(raw_idx, seed) => {
                            // set_component is the in-place overwrite path; it
                            // must mirror exactly into the oracle's same slot.
                            if oracle.is_empty() {
                                let v = U64Pair { a: seed, b: seed };
                                prop_assert!(!pool.set_component(0, u64pair_bytes(&v)));
                            } else {
                                let idx = raw_idx % oracle.len();
                                let v = U64Pair { a: seed, b: seed.rotate_left(32) };
                                prop_assert!(pool.set_component(idx, u64pair_bytes(&v)));
                                oracle[idx] = v;
                            }
                        }
                    }

                    prop_assert_eq!(pool.count(), oracle.len());
                    // multi-index: pool row (`get_typed(i)`) + oracle, by the same `i`.
                    #[allow(clippy::needless_range_loop)]
                    for i in 0..oracle.len() {
                        let got = pool.get_typed::<U64Pair>(i)
                            .expect("live row must read back");
                        prop_assert_eq!(*got, oracle[i],
                            "row value mismatch vs oracle at index {}", i);
                    }
                }
            }
        }
    }

    /// GATE 3 — `drop_count_exactly_once`.
    ///
    /// Pins the three drop-accounting contracts the `Drop { for row in 0..len }`
    /// loop and the two swap-remove variants must honour:
    ///   (a) pool `Drop` drops each LIVE row exactly once and NEVER the
    ///       uninitialised `[len, committed_rows)` slots;
    ///   (b) `swap_remove` (the drop variant) drops the removed row exactly once;
    ///   (c) `swap_remove_index_no_drop` drops ZERO (the migration path that
    ///       has already moved the bytes out).
    #[test]
    fn drop_count_exactly_once() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        component_registry::register_layout::<Dropper>(DROPPER_ID.0);
        // Capacity 16, 8 live → 8 uninit slots that must NOT be dropped.
        let mut pool = ComponentPool::new(DROPPER_ID.0, 16);

        let counter = Arc::new(AtomicUsize::new(0));
        const M: usize = 8;
        for _ in 0..M {
            pool.add_typed(Dropper { counter: Arc::clone(&counter) })
                .expect("capacity 16 holds 8");
        }
        assert_eq!(counter.load(Ordering::Relaxed), 0, "no drops before any removal");

        // (b) swap_remove (drop variant) on a middle row → exactly one drop.
        assert!(pool.swap_remove(3), "swap_remove(3) in bounds");
        assert_eq!(
            counter.load(Ordering::Relaxed),
            1,
            "swap_remove(drop) must drop exactly the removed row once"
        );

        // (c) swap_remove_index_no_drop on a middle row → ZERO additional drops.
        // The bytes are NOT moved out here (this is a white-box drop-accounting
        // probe, not a real migration), so the moved Arc is intentionally
        // leaked by the no-drop semantics — we account for it below so the
        // process-exit drop bookkeeping stays balanced.
        let live_before = pool.count();
        // SAFETY: idx 2 < pool.count() (== 7 here); we hold &mut pool. The
        // no-drop contract requires the caller to have moved/dropped the source
        // bytes — this probe deliberately exercises the ZERO-drop path, so we
        // compensate the leaked Arc strong-count after the pool is gone.
        unsafe { pool.swap_remove_index_no_drop(2) };
        assert_eq!(
            pool.count(),
            live_before - 1,
            "swap_remove_index_no_drop must still decrement count"
        );
        assert_eq!(
            counter.load(Ordering::Relaxed),
            1,
            "swap_remove_index_no_drop must drop ZERO (count stays at the prior 1)"
        );

        // (a) Drop the pool: the remaining live rows each drop exactly once.
        // After swap_remove(drop) (−1 live, +1 dropped) and
        // swap_remove_index_no_drop (−1 live, +0 dropped), 6 rows are live.
        // The no-drop variant overwrote row 2 with the moved row's bytes WITHOUT
        // dropping row 2's original Arc, so that one Arc strong-count is leaked
        // by design of the probe; total observed drops at pool Drop = 1 + 6 = 7.
        let live_at_drop = pool.count();
        assert_eq!(live_at_drop, M - 2, "two rows removed → M-2 live");
        drop(pool);
        assert_eq!(
            counter.load(Ordering::Relaxed),
            1 + live_at_drop,
            "pool Drop must drop each of the {live_at_drop} remaining live rows \
             exactly once (total = 1 swap_remove + {live_at_drop} live); the \
             uninit [len, max) slots must NOT be dropped"
        );
    }

    /// Phase X.I U-P1 (★R1-5) — a zero row ceiling must hit the loud
    /// pool-level construction assert that names the constructor — NOT a
    /// `VmReservation::reserve(0)` panic with a vm-internals message.
    #[test]
    #[should_panic(expected = "ComponentPool::new: reserve_rows == 0")]
    fn zero_ceiling_construction_panics_loudly() {
        register_all();
        let _ = ComponentPool::new(POS_ID.0, 0);
    }

    // ====================================================================
    // Phase X.I W4 — the growth test matrix (U-P2 … U-P5, U-P8).
    //
    // Geometry note shared by every test below (re-derived for the packing
    // plan's page ladder, S2): the fixtures use a 64-byte stride, and the
    // ladder commits DATA from the sub-region's absolute page floor, so the
    // committed-row frontier after committing `d` data bytes is
    // `(d - σ) / 64` with `σ = pool_base_stagger(id)`. For `Stride64` (id 226,
    // σ = 2176) on the 4 KiB page the one-row-at-a-time frontiers are 30, 94,
    // 222, 478, 990, 2014, 4062 and then the 4096-row ceiling: a 4096-row
    // pool's `add` loop takes 8 data-commit events (it took 3 on the 64 KiB
    // granule, whose frontiers sat at 1024 / 2048 / 4096 for every σ).
    // ====================================================================

    /// Phase X.I W4 fixture: a 64-byte POD component (8 x u64). `tag` makes
    /// every row distinguishable; `pad` brings the stride to exactly one
    /// cache line (see geometry note above).
    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Debug)]
    struct Stride64 {
        tag: u64,
        pad: [u64; 7],
    }

    impl Stride64 {
        fn new(tag: u64) -> Self {
            Self { tag, pad: [tag ^ 0xDEAD_BEEF_CAFE_F00D; 7] }
        }
    }

    const STRIDE64_ID: ComponentId = ComponentId(226);

    impl Component for Stride64 {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| {
                component_registry::register_layout::<Stride64>(STRIDE64_ID.0);
                STRIDE64_ID
            })
        }
    }

    fn make_stride64_pool(cap: usize) -> ComponentPool {
        component_registry::register_layout::<Stride64>(STRIDE64_ID.0);
        ComponentPool::new(STRIDE64_ID.0, cap)
    }

    /// The committed-row frontiers a one-row-at-a-time fill of a
    /// `make_stride64_pool(cap)` pool passes through (packing plan D2): the
    /// data ladder doubles from one `COMMIT_PAGE` measured from the absolute
    /// page floor, so rung `k` exposes `(COMMIT_PAGE * 2^k - σ) / 64` rows,
    /// until the ceiling clamps. `2^k` pages stay below `POOL_MAX_SLAB` for
    /// every `cap` these tests use.
    fn stride64_rungs(cap: usize) -> Vec<usize> {
        use crate::ecs::constants::{COMMIT_PAGE, pool_base_stagger};

        let sigma = pool_base_stagger(STRIDE64_ID.0);
        let mut rungs = Vec::new();
        let mut data = COMMIT_PAGE;
        loop {
            let rows = ((data - sigma) / 64).min(cap);
            rungs.push(rows);
            if rows == cap {
                return rungs;
            }
            data *= 2;
        }
    }

    /// U-P2 — the address-stability witness (plan §Test matrix), and the
    /// packing plan's G5 stagger-placement pin.
    ///
    /// Pins THE central Phase X.I soundness claim (Soundness item 1): the
    /// three write-once base pointers (`buffer_ptr`, `added_ticks_ptr`,
    /// `changed_ticks_ptr`) and any previously returned row pointer stay
    /// bit-identical across every data-commit growth event, and pre-growth
    /// VALUES (component bytes + stamped ticks) remain readable through the
    /// recorded pointers. A 4096-row pool of 64-B rows filled by an add loop
    /// passes the 8 frontiers of the geometry note (the name predates the page
    /// ladder, under which the 64 KiB granule's 3 events became 8).
    ///
    /// G5: before the first grow and after the last, each base sits at
    /// `pool_base_stagger(id)` within its commit page. A page-floor commit that
    /// ever moved a base (instead of committing below it) would read 0 here;
    /// nothing else in the tree pins a pool's actual base offset.
    ///
    /// Miri: ignored (4096-add loop; the M-XI suite covers the identical
    /// bookkeeping with a small-granule-count geometry under Tree Borrows).
    #[test]
    #[cfg_attr(miri, ignore = "miri-slow: a 4096-add loop; the M-XI suite (miri_pool_growth.rs) pins the identical bookkeeping at small-granule geometry")]
    fn address_stability_across_three_slab_growths() {
        use crate::ecs::constants::{COMMIT_PAGE, pool_base_stagger};
        use crate::ecs::core::change_detection::Tick;

        let mut pool = make_stride64_pool(4096);
        assert_eq!(pool.component_layout().size(), 64, "fixture stride must be 64 B");
        assert_eq!(pool.capacity(), 4096, "D2 mapping: reserve_rows = 1 * 4096");
        assert_eq!(pool.committed_rows(), 0, "D3: zero initial commit");

        // G5 before the first grow: every base carries the stagger.
        let sigma = pool_base_stagger(STRIDE64_ID.0);
        assert_eq!(sigma, 2176, "id 226 ⇒ σ = 34 × 64 (the geometry note's numbers)");
        let stagger_placement = |p: &ComponentPool| {
            [
                p.buffer_ptr() as usize % COMMIT_PAGE,
                p.added_ticks_ptr() as usize % COMMIT_PAGE,
                p.changed_ticks_ptr() as usize % COMMIT_PAGE,
            ]
        };
        assert_eq!(stagger_placement(&pool), [sigma; 3], "G5 before the first grow");

        // The frontiers the add loop must pass through (geometry note).
        let rungs = stride64_rungs(4096);
        #[cfg(target_arch = "x86_64")]
        assert_eq!(rungs, [30, 94, 222, 478, 990, 2014, 4062, 4096]);

        // First add: the first data page — (COMMIT_PAGE - σ) / 64 rows (was
        // one 64 KiB granule = 1024 rows).
        let v0 = Stride64::new(0xA5A5_0000);
        pool.add_typed(v0).expect("row 0 fits under the ceiling");
        assert_eq!(pool.committed_rows(), rungs[0], "first commit = one page from the floor");

        // Record every base pointer + the row-0 pointer BEFORE further growth,
        // and stamp a distinctive pre-growth tick on row 0.
        let base_before = pool.buffer_ptr();
        let row0_before = pool.get_raw(0).expect("row 0 live");
        let added_before = pool.added_ticks_ptr();
        let changed_before = pool.changed_ticks_ptr();
        pool.fill_ticks(0, 1, Tick::new(7));

        // Grow across every remaining rung via the warm add path, recording
        // each frontier the pool actually passes.
        let mut seen = vec![pool.committed_rows()];
        for i in 1..4096u64 {
            pool.add_typed(Stride64::new(i)).expect("under the 4096-row ceiling");
            if pool.committed_rows() != *seen.last().expect("seeded above") {
                seen.push(pool.committed_rows());
            }
        }
        assert_eq!(pool.count(), 4096, "all 4096 rows live");
        assert_eq!(pool.committed_rows(), 4096, "frontier reached the full reserve");
        assert_eq!(seen, rungs, "the add loop passes exactly the ladder's frontiers");
        assert!(seen.len() >= 8, "the plan's G5 asks for >= 8 growth events");

        // (1) POINTER identity: all recorded pointers are bit-identical —
        // growth never remapped or relocated anything.
        assert_eq!(pool.buffer_ptr(), base_before, "data base must not move across growth");
        assert_eq!(
            pool.get_raw(0).expect("row 0 still live"),
            row0_before,
            "row-0 pointer must not move across growth"
        );
        assert_eq!(pool.added_ticks_ptr(), added_before, "added tick base must not move");
        assert_eq!(pool.changed_ticks_ptr(), changed_before, "changed tick base must not move");

        // (2) VALUE identity through the OLD pointers: pre-growth bytes and
        // ticks are still readable and unchanged.
        assert_eq!(
            *pool.get_typed::<Stride64>(0).expect("row 0 typed read"),
            v0,
            "pre-growth row-0 value must survive 3 growth events untouched"
        );
        // SAFETY: 0 < count() <= committed_rows; &pool is the only borrow.
        let (t_add, t_chg) = unsafe { (pool.read_added_tick(0), pool.read_changed_tick(0)) };
        assert_eq!(t_add, Tick::new(7), "pre-growth added tick survives growth");
        assert_eq!(t_chg, Tick::new(7), "pre-growth changed tick survives growth");

        // (3) Boundary rows on both sides of each commit edge read back
        // correctly: `f - 1` is the last row of one commit, `f` the first of
        // the next (x86_64: 29/30, 93/94, …, 4061/4062), plus the last row.
        let edges = rungs[..rungs.len() - 1].iter().flat_map(|&f| [f - 1, f]);
        for row in edges.chain([4095]) {
            assert_eq!(
                *pool.get_typed::<Stride64>(row).expect("boundary row typed read"),
                Stride64::new(row as u64),
                "row {row} (commit-boundary +/- 1) must hold its written value"
            );
        }

        // G5 after the last grow.
        assert_eq!(stagger_placement(&pool), [sigma; 3], "G5 after the last grow");
    }

    /// U-P3 — reserve-ceiling exhaustion leaves the pool state EXACTLY
    /// unchanged (Soundness item 6: "ceiling exhaustion -> None, ZERO state
    /// change"). A tiny D2-mapped `1 x 4` pool is filled to its ceiling;
    /// the rejected 5th add must not move `count` / `committed_rows` /
    /// `capacity` / the base pointer, and `can_reserve(1)` must be false.
    #[test]
    fn ceiling_exhaustion_rejects_add_with_zero_state_change() {
        let mut pool = make_stride64_pool(4);

        for i in 0..4u64 {
            pool.add_typed(Stride64::new(i)).expect("rows 0..4 fit under the ceiling");
        }
        // min(rows_from_committed_bytes, reserve_rows) clamps the frontier to
        // the ceiling (GROW1-XI step 4): the first data page covers
        // (4096 - 2176) / 64 = 30 rows but the pool may only ever expose 4.
        assert_eq!(pool.committed_rows(), 4, "frontier clamps to the 4-row ceiling");
        assert!(pool.is_full(), "len == reserve_rows is the ceiling");

        let before = (
            pool.count(),
            pool.committed_rows(),
            pool.capacity(),
            pool.buffer_ptr() as usize,
            pool.remaining_capacity(),
        );

        assert_eq!(
            pool.add_typed(Stride64::new(99)),
            None,
            "add past the reserve ceiling must return None"
        );

        let after = (
            pool.count(),
            pool.committed_rows(),
            pool.capacity(),
            pool.buffer_ptr() as usize,
            pool.remaining_capacity(),
        );
        assert_eq!(
            before, after,
            "a rejected add must leave (count, committed_rows, capacity, base, remaining) \
             EXACTLY unchanged"
        );
        assert!(!pool.can_reserve(1), "can_reserve(1) is false at the ceiling");
        assert_eq!(pool.remaining_capacity(), 0, "no rows remain below the ceiling");
        // The 4 live values are untouched by the rejected add.
        for i in 0..4u64 {
            assert_eq!(
                *pool.get_typed::<Stride64>(i as usize).expect("live row"),
                Stride64::new(i),
                "row {i} value unchanged after the rejected add"
            );
        }
    }

    /// U-P4 — tick lockstep + the J-XI never-written invariant at a slab
    /// boundary +/- 1 (Soundness item 4, ★R1-4 never-written form).
    ///
    /// Witness strategy (per the W4 brief): J-XI is read through the pool's
    /// raw tick-base pointers, whose DOCUMENTED contract
    /// (`added_ticks_ptr`: "valid for `self.committed_rows()` readable
    /// `UnsafeCell<Tick>` slots") explicitly permits reads of
    /// never-written slots inside `[len, committed_rows)` — no contract
    /// violation, no new accessors. The `read_added_tick`-style accessors
    /// (whose contract requires `index < count()`) are used only below `len`.
    ///
    /// Sequence (packing plan S2: re-derived on the page ladder, from
    /// `stride64_rungs`; x86_64 numbers in brackets): first add commits the
    /// first data page -> every never-written slot in `[1, r0)` reads
    /// `Tick::ZERO` [r0 = 30]; fill to the `r0` boundary and stamp `[0, r0)`;
    /// the next add grows the DATA across the boundary [to 94] -> pre-grow
    /// stamps at rows `r0 - 1`/`r0 - 2` survive, the freshly-grown row `r0`
    /// reads ZERO until stamped (write-before-read), then reads its stamp, and
    /// the new never-written tail reads ZERO. Then the fill continues to the
    /// first rung whose rows outgrow the first TICK page [478 -> 990, ticks
    /// 4096 -> 8192 B]: stamps below it survive, and the never-written tail,
    /// which now reaches into the freshly committed tick page, reads ZERO.
    /// (On the 64 KiB granule the one crossing was 1024 -> 2048 and the tick
    /// granule never moved.)
    #[test]
    #[cfg_attr(miri, ignore = "miri-slow: ~1000 adds + raw tick scans on both sides of two boundaries; the tick bases are pub(crate), so no tests/ Miri suite can reach this surface")]
    fn tick_lockstep_and_jxi_zero_at_slab_boundary() {
        use crate::ecs::constants::{COMMIT_PAGE, pool_base_stagger};
        use crate::ecs::core::change_detection::Tick;

        let mut pool = make_stride64_pool(4096);
        let rungs = stride64_rungs(4096);
        let (r0, r1) = (rungs[0], rungs[1]);
        #[cfg(target_arch = "x86_64")]
        assert_eq!((r0, r1), (30, 94));

        // Row 0 commits the first data page (r0 rows); the first tick page
        // covers every one of them (σ + 4·r0 <= COMMIT_PAGE) — lockstep by
        // ROWS, saturating by bytes.
        pool.add_typed(Stride64::new(0)).expect("row 0");
        assert_eq!(pool.committed_rows(), r0);
        assert_eq!(pool.ticks_committed, COMMIT_PAGE, "one tick page per tick sub-region");

        // J-XI (1): every never-written tick slot in [len, committed_rows)
        // reads Tick::ZERO through the raw base pointers.
        let added = pool.added_ticks_ptr();
        let changed = pool.changed_ticks_ptr();
        for i in pool.count()..pool.committed_rows() {
            // SAFETY: i < committed_rows() — inside the accessors' documented
            // validity window; shared read of a never-written UnsafeCell slot
            // with no concurrent writer (&pool only).
            let (a, c) = unsafe { (*(*added.add(i)).get(), *(*changed.add(i)).get()) };
            assert_eq!(a, Tick::ZERO, "never-written added slot {i} must read ZERO (J-XI)");
            assert_eq!(c, Tick::ZERO, "never-written changed slot {i} must read ZERO (J-XI)");
        }

        // Fill to the boundary and stamp every live row.
        for i in 1..r0 as u64 {
            pool.add_typed(Stride64::new(i)).expect("rows 1..r0");
        }
        assert_eq!(pool.count(), r0);
        assert_eq!(pool.committed_rows(), r0, "len reached the first-page frontier");
        pool.fill_ticks(0, r0, Tick::new(5));

        // Cross the boundary: row r0 triggers grow_rows(r0 + 1) -> r1 rows.
        pool.add_typed(Stride64::new(r0 as u64)).expect("row r0 grows the pool");
        assert_eq!(pool.committed_rows(), r1, "doubling: one data page -> two");

        // Pre-grow stamps at [boundary-2, boundary-1] survived the grow.
        // SAFETY: r0-2 / r0-1 < count(); &pool shared read, no writer.
        let (t_m2, t_m1) = unsafe { (pool.read_added_tick(r0 - 2), pool.read_added_tick(r0 - 1)) };
        assert_eq!(t_m2, Tick::new(5), "stamp at boundary-2 survives the grow");
        assert_eq!(t_m1, Tick::new(5), "stamp at boundary-1 survives the grow");

        // The freshly-grown-then-added row r0: never stamped -> ZERO (J-XI
        // on a demand-committed page), then write-before-read round-trips.
        // SAFETY: r0 < count() == r0 + 1; &pool exclusive in this test.
        let t_unstamped = unsafe { pool.read_added_tick(r0) };
        assert_eq!(
            t_unstamped,
            Tick::ZERO,
            "a freshly committed, never-stamped tick slot reads ZERO"
        );
        // SAFETY: r0 < count(); single-threaded test holds exclusive access.
        unsafe {
            pool.write_added_tick(r0, Tick::new(9));
            pool.write_changed_tick(r0, Tick::new(9));
        }
        // SAFETY: as above.
        let (a_r0, c_r0) = unsafe { (pool.read_added_tick(r0), pool.read_changed_tick(r0)) };
        assert_eq!(a_r0, Tick::new(9), "write-before-read: stamped added tick reads back");
        assert_eq!(c_r0, Tick::new(9), "write-before-read: stamped changed tick reads back");

        // J-XI (2): the newly committed never-written tail also reads ZERO.
        for i in pool.count()..pool.committed_rows() {
            // SAFETY: i < committed_rows() — documented validity window.
            let (a, c) = unsafe { (*(*added.add(i)).get(), *(*changed.add(i)).get()) };
            assert_eq!(a, Tick::ZERO, "post-grow never-written added slot {i} reads ZERO");
            assert_eq!(c, Tick::ZERO, "post-grow never-written changed slot {i} reads ZERO");
        }

        // The TICK-page boundary: fill to the frontier of the last rung whose
        // rows the first tick page still covers, stamp, then take one more add.
        let sigma = pool_base_stagger(STRIDE64_ID.0);
        let tick_edge = *rungs
            .iter()
            .rev()
            .find(|&&f| sigma + 4 * f <= COMMIT_PAGE)
            .expect("the first rung fits one tick page");
        let next = rungs[rungs.iter().position(|&f| f == tick_edge).expect("a rung") + 1];
        #[cfg(target_arch = "x86_64")]
        assert_eq!((tick_edge, next), (478, 990));
        while pool.count() < tick_edge {
            let i = pool.count() as u64;
            pool.add_typed(Stride64::new(i)).expect("under the 4096-row ceiling");
        }
        assert_eq!(pool.committed_rows(), tick_edge);
        assert_eq!(pool.ticks_committed, COMMIT_PAGE, "still one tick page");
        pool.fill_ticks(0, tick_edge, Tick::new(6));
        pool.add_typed(Stride64::new(tick_edge as u64)).expect("the tick-page crossing");
        assert_eq!(pool.committed_rows(), next);
        assert_eq!(
            pool.ticks_committed,
            (sigma + 4 * next).next_multiple_of(COMMIT_PAGE),
            "the grow committed the tick pages the new rows need"
        );
        assert!(pool.ticks_committed > COMMIT_PAGE, "a second tick page was committed");
        // SAFETY: tick_edge - 1 < count(); &pool shared read, no writer.
        let t_edge = unsafe { pool.read_changed_tick(tick_edge - 1) };
        assert_eq!(t_edge, Tick::new(6), "the last stamp below the tick-page edge survives");
        for i in pool.count()..pool.committed_rows() {
            // SAFETY: i < committed_rows() — documented validity window.
            let (a, c) = unsafe { (*(*added.add(i)).get(), *(*changed.add(i)).get()) };
            assert_eq!(a, Tick::ZERO, "never-written added slot {i} past the tick edge reads ZERO");
            assert_eq!(c, Tick::ZERO, "never-written changed slot {i} past the tick edge reads ZERO");
        }
    }

    /// Phase X.I W4 fixture: a 64-byte drop-counting component. At id 227
    /// (σ = 2240) its page-ladder frontiers are 29, 93, 221, 477, 989, 2013
    /// rows (they sat at 1024 rows, one granule, on the granule ladder).
    #[repr(C)]
    struct DropPad64 {
        counter: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        pad: [u64; 7],
    }

    impl Drop for DropPad64 {
        fn drop(&mut self) {
            self.counter
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    const DROP_PAD_ID: ComponentId = ComponentId(227);

    impl Component for DropPad64 {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| {
                component_registry::register_layout::<DropPad64>(DROP_PAD_ID.0);
                DROP_PAD_ID
            })
        }
    }

    /// U-P5 — drop-count-exact across growth boundaries. 1500 64-B rows
    /// cross five commit boundaries (six growth events, the last to 2013
    /// rows); 5 pops + 3 swap_removes drop exactly 8; pool Drop drops exactly the
    /// 1492 survivors. Total == 1500 — every value dropped EXACTLY once,
    /// no uninit `[len, committed_rows)` slot dropped, and the growth event
    /// itself dropped nothing (O(1), zero bytes copied, zero drops).
    #[test]
    #[cfg_attr(miri, ignore = "miri-slow: 1500 Arc-carrying rows dropped one by one; M-XI miri_drop_count_exact_across_boundary pins the same count at 70 rows")]
    fn drop_count_exact_across_growth_boundary() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        component_registry::register_layout::<DropPad64>(DROP_PAD_ID.0);
        let mut pool = ComponentPool::new(DROP_PAD_ID.0, 4096);

        let counter = Arc::new(AtomicUsize::new(0));
        const M: usize = 1500; // crosses the 989-row boundary, among others
        for _ in 0..M {
            pool.add_typed(DropPad64 {
                counter: Arc::clone(&counter),
                pad: [0; 7],
            })
            .expect("1500 rows fit under the 4096 ceiling");
        }
        assert_eq!(
            counter.load(Ordering::Relaxed),
            0,
            "growth events must drop NOTHING (adds crossed a slab boundary)"
        );
        assert!(pool.committed_rows() >= M, "the boundary crossing grew the frontier");

        for _ in 0..5 {
            assert!(pool.pop(), "pop while non-empty");
        }
        assert_eq!(counter.load(Ordering::Relaxed), 5, "5 pops drop exactly 5");

        // One row of the first commit, one row past the 989-row boundary.
        assert!(pool.swap_remove(10), "swap_remove(10) in bounds");
        assert!(pool.swap_remove(1100), "swap_remove(1100) in bounds");
        assert!(pool.swap_remove(0), "swap_remove(0) in bounds");
        assert_eq!(counter.load(Ordering::Relaxed), 8, "3 swap_removes drop exactly 3");

        let live = pool.count();
        assert_eq!(live, M - 8, "1500 - 5 pops - 3 swap_removes live");
        drop(pool);
        assert_eq!(
            counter.load(Ordering::Relaxed),
            M,
            "pool Drop drops each of the {live} survivors exactly once \
             (total {M}); uninit [len, committed_rows) slots must NOT drop"
        );
    }

    /// U-P8 — `grow_rows` idempotence (★R1-1 / GROW1-XI proof 0).
    ///
    /// `grow_rows(n <= committed_rows)` must return `true` with
    /// `committed_rows` EXACTLY unchanged (zero syscalls is not observable
    /// here; the frontier scalar is the witness), repeatedly; a real grow
    /// must still work after the no-ops; and the ceiling arm must return
    /// `false` with zero state change. This is the guard that makes
    /// `Archetype::reserve_capacity` Phase B's UNCONDITIONAL `grow_rows`
    /// calls legal (the critic-Round-1 CRITICAL fix).
    ///
    /// Packing plan S2 re-derivation (x86_64 numbers in brackets): the first
    /// grow is request-dominant, `align_up_page(σ + 100 * 64)` of data from the
    /// floor [12 288 B -> (12 288 - 2176) / 64 = 158 rows; was one granule =
    /// 1024 rows], and the real grow after the no-ops is the doubling arm
    /// [24 576 B -> 350 rows; was 64 KiB -> 128 KiB = 2048 rows].
    #[test]
    fn grow_rows_idempotent_below_frontier() {
        use crate::ecs::constants::{COMMIT_PAGE, pool_base_stagger};

        let mut pool = make_stride64_pool(4096);
        let sigma = pool_base_stagger(STRIDE64_ID.0);
        let first_data = (sigma + 100 * 64).next_multiple_of(COMMIT_PAGE);
        let first_rows = (first_data - sigma) / 64;
        let doubled_rows = (2 * first_data - sigma) / 64;
        #[cfg(target_arch = "x86_64")]
        assert_eq!((first_rows, doubled_rows), (158, 350));

        // First real grow: request 100 -> first_rows.
        assert!(pool.grow_rows(100), "grow within the ceiling succeeds");
        assert_eq!(pool.committed_rows(), first_rows, "request-dominant first commit");

        // Idempotent no-op arm, exercised repeatedly at several n values
        // including the n == committed_rows edge and n == 0.
        for _round in 0..2 {
            for n in [0usize, 1, 100, first_rows - 1, first_rows] {
                let before = pool.committed_rows();
                assert!(
                    pool.grow_rows(n),
                    "grow_rows({n}) with n <= committed_rows must return true"
                );
                assert_eq!(
                    pool.committed_rows(),
                    before,
                    "grow_rows({n}) no-op arm must leave committed_rows EXACTLY unchanged"
                );
            }
        }

        // A real grow still works after the no-ops (the early-out must not
        // have corrupted the frontier bookkeeping).
        assert!(pool.grow_rows(first_rows + 1), "grow past the frontier succeeds");
        assert_eq!(pool.committed_rows(), doubled_rows, "doubling: the data frontier x2");

        // Ceiling arm: false, ZERO state change.
        let before = pool.committed_rows();
        assert!(!pool.grow_rows(4097), "grow past reserve_rows must return false");
        assert_eq!(
            pool.committed_rows(),
            before,
            "a rejected (ceiling) grow must leave committed_rows EXACTLY unchanged"
        );
        assert_eq!(pool.capacity(), 4096, "the ceiling itself never moves");
    }

    // ====================================================================
    // Phase 22 D6 — positive ZST (tag) pool coverage.
    //
    // Replaces the retired `tests/drop_fn.rs` ZST-rejection test: size 0 is
    // now a valid, distinct pool layout (vacuous data region, dangling
    // SIMD-A1-aligned buffer, tick-driven GROW1-ZST growth). The tests pin
    // add/swap_remove/pop semantics, tick stamping + swap lockstep,
    // Drop-impl-ZST teardown accounting, the with_default_sizes routing,
    // and the double-commit growth gate.
    // ====================================================================

    /// A data-less tag (no Drop impl ⇒ `drop_fn == None`).
    struct ZstTag;

    const ZST_TAG_ID: ComponentId = ComponentId(228);

    impl Component for ZstTag {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| {
                component_registry::register_layout::<ZstTag>(ZST_TAG_ID.0);
                ZST_TAG_ID
            })
        }
    }

    fn make_zst_pool(reserve_rows: usize) -> ComponentPool {
        component_registry::register_layout::<ZstTag>(ZST_TAG_ID.0);
        ComponentPool::new(ZST_TAG_ID.0, reserve_rows)
    }

    /// Phase 22 — construction + add/swap_remove/pop on a ZST pool, with
    /// the dangling-base address identity: every live row reads back at the
    /// SIMD-A1-aligned dangling base (`row_ptr ≡ buffer` at stride 0).
    #[test]
    fn zst_pool_add_swap_remove_pop() {
        use crate::ecs::constants::SIMD_BUFFER_ALIGN;

        let mut pool = make_zst_pool(8);
        assert_eq!(pool.component_layout().size(), 0, "fixture must be a ZST");
        assert_eq!(pool.capacity(), 8);
        assert_eq!(pool.committed_rows(), 0, "D3: zero initial commit");

        // The dangling base: SIMD_BUFFER_ALIGN.max(align_of::<ZstTag>())
        // == 32 for an align-1 tag (the per-arm address the plan pins).
        let base = pool.buffer_ptr() as usize;
        assert_eq!(base, SIMD_BUFFER_ALIGN, "align-1 ZST dangling base sits at 32");

        // add: tail indices + count tracking; the first add takes the
        // grow_rows → grow_rows_zst path (early-branch integration).
        for i in 0..5usize {
            assert_eq!(pool.add_typed(ZstTag), Some(i), "add returns the tail index");
        }
        assert_eq!(pool.count(), 5);
        assert!(pool.committed_rows() >= 5, "first add grew the tick frontier");
        assert_eq!(pool.data_committed, 0, "Z1: no data bytes ever committed");

        // Every live row's pointer is the dangling base (idx * 0 == 0) and
        // typed ZST reads succeed (a &ZST at a dangling aligned address is
        // a valid reference).
        for i in 0..5 {
            assert_eq!(pool.get_raw(i).expect("live row") as usize, base);
            assert!(pool.get_typed::<ZstTag>(i).is_some(), "typed ZST read");
        }
        assert!(pool.get_raw(5).is_none(), "out-of-bounds read is None");

        // swap_remove a middle row: 0-byte copy + tick lockstep.
        assert!(pool.swap_remove(1), "swap_remove(1) in bounds");
        assert_eq!(pool.count(), 4);

        // pop the tail.
        assert!(pool.pop(), "pop while non-empty");
        assert_eq!(pool.count(), 3);

        // Drain; empty-pool ops are no-ops.
        while pool.count() > 0 {
            assert!(pool.swap_remove(0));
        }
        assert!(!pool.swap_remove(0), "swap_remove on an empty pool is a no-op");
        assert!(!pool.pop(), "pop on an empty pool is a no-op");
        assert_eq!(pool.data_committed, 0, "Z1 holds across the whole sequence");
    }

    /// Phase 22 — tick stamping on a ZST pool: `fill_ticks` and the
    /// single-row writers round-trip, and `swap_remove` moves the LAST
    /// row's ticks into the vacated slot (lockstep) exactly as for data
    /// pools.
    #[test]
    fn zst_pool_tick_stamping_and_swap_lockstep() {
        use crate::ecs::core::change_detection::Tick;

        let mut pool = make_zst_pool(16);
        for _ in 0..4 {
            pool.add_typed(ZstTag).expect("under the 16-row ceiling");
        }

        // Bulk-stamp all rows, then over-stamp the last row distinctly.
        pool.fill_ticks(0, 4, Tick::new(10));
        // SAFETY: 3 < count() == 4; this test holds exclusive access.
        unsafe {
            pool.write_added_tick(3, Tick::new(99));
            pool.write_changed_tick(3, Tick::new(99));
        }

        // SAFETY: 0 < count(); shared reads, no concurrent writer.
        let (a0, c0) = unsafe { (pool.read_added_tick(0), pool.read_changed_tick(0)) };
        assert_eq!(a0, Tick::new(10), "bulk-stamped added tick reads back");
        assert_eq!(c0, Tick::new(10), "bulk-stamped changed tick reads back");

        // swap_remove(0): row 3's ticks (99) must move into slot 0.
        assert!(pool.swap_remove(0));
        // SAFETY: 0 < count() == 3; shared reads, no concurrent writer.
        let (a0_after, c0_after) =
            unsafe { (pool.read_added_tick(0), pool.read_changed_tick(0)) };
        assert_eq!(a0_after, Tick::new(99), "tick lockstep: last row's added tick moved");
        assert_eq!(c0_after, Tick::new(99), "tick lockstep: last row's changed tick moved");
    }

    /// A ZST WITH a Drop impl (`needs_drop` true ⇒ `drop_fn` Some). The
    /// counter is a static — a counting FIELD would make the type
    /// non-zero-sized — so this fixture is used by exactly ONE test.
    struct ZstDropTag;

    static ZST_DROP_COUNT: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    impl Drop for ZstDropTag {
        fn drop(&mut self) {
            ZST_DROP_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    const ZST_DROP_ID: ComponentId = ComponentId(229);

    impl Component for ZstDropTag {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| {
                component_registry::register_layout::<ZstDropTag>(ZST_DROP_ID.0);
                ZST_DROP_ID
            })
        }
    }

    /// Phase 22 — Drop-impl-ZST teardown: `drop_in_place::<ZST>` at the
    /// dangling base reads no bytes; swap_remove / pop / pool-Drop must
    /// each account exactly one drop per logical row.
    #[test]
    fn zst_pool_drop_impl_teardown_counts_exactly() {
        use std::sync::atomic::Ordering;

        assert_eq!(std::mem::size_of::<ZstDropTag>(), 0, "fixture must be a ZST");
        assert!(std::mem::needs_drop::<ZstDropTag>(), "fixture must carry drop glue");

        component_registry::register_layout::<ZstDropTag>(ZST_DROP_ID.0);
        let mut pool = ComponentPool::new(ZST_DROP_ID.0, 16);

        let base = ZST_DROP_COUNT.load(Ordering::Relaxed);
        const M: usize = 6;
        for _ in 0..M {
            pool.add_typed(ZstDropTag).expect("under the 16-row ceiling");
        }
        assert_eq!(
            ZST_DROP_COUNT.load(Ordering::Relaxed) - base,
            0,
            "adds (by-move) must not drop"
        );

        assert!(pool.swap_remove(2), "swap_remove(2) in bounds");
        assert_eq!(
            ZST_DROP_COUNT.load(Ordering::Relaxed) - base,
            1,
            "swap_remove drops the removed ZST exactly once"
        );

        assert!(pool.pop(), "pop while non-empty");
        assert_eq!(
            ZST_DROP_COUNT.load(Ordering::Relaxed) - base,
            2,
            "pop drops the tail ZST exactly once"
        );

        let live = pool.count();
        assert_eq!(live, M - 2, "two rows removed → M-2 live");
        drop(pool);
        assert_eq!(
            ZST_DROP_COUNT.load(Ordering::Relaxed) - base,
            2 + live,
            "pool Drop drops each remaining logical row exactly once \
             (one drop_in_place per row at the shared dangling base)"
        );
    }

    /// Phase 22 — `with_default_sizes` on a ZST routes through the
    /// `pool_reserve_rows` ZST arm: ceiling == POOL_MAX_ROWS (tick-bounded,
    /// address-space-only reservation, zero commit).
    #[test]
    fn zst_pool_default_sizes_route_to_max_rows() {
        use crate::ecs::constants::POOL_MAX_ROWS;

        component_registry::register_layout::<ZstTag>(ZST_TAG_ID.0);
        let pool = ComponentPool::with_default_sizes(ZST_TAG_ID.0);
        assert_eq!(pool.capacity(), POOL_MAX_ROWS, "D6: tick-bounded ceiling");
        assert_eq!(pool.committed_rows(), 0, "zero commit at construction");
        assert_eq!(pool.data_committed, 0, "Z1 at construction");
    }

    /// Phase 22 — THE GROW1-ZST growth gate: successive `grow_rows_zst`
    /// invocations that EACH reach `vm.commit`, with strict tick-frontier
    /// growth (Z4), request coverage (Z5), `data_committed == 0` throughout
    /// (Z1), and stable base pointers across every commit.
    ///
    /// Geometry (packing plan S2 re-derivation; x86_64 numbers in brackets):
    /// ticks are 4 B/row and the tick frontier is measured from the absolute
    /// page floor, so `grow(1)` commits one tick page, `(COMMIT_PAGE - σ) / 4`
    /// rows [4096 B, 448 rows at σ = 2304; was one granule, 16 384 rows]. The
    /// second call asks for one row past it: the DOUBLING arm [8192 B, 1472
    /// rows]. The third asks for 20 000 rows: the REQUEST-DOMINANT arm
    /// [`align_up_page(σ + 80 000)` = 86 016 B, 20 928 rows; the granule
    /// version's second call, 20 000, doubled to 128 KiB = 32 768 rows].
    #[test]
    fn zst_pool_growth_two_successive_commits() {
        use crate::ecs::constants::{COMMIT_PAGE as P, pool_base_stagger};

        let mut pool = make_zst_pool(100_000);
        assert_eq!(pool.committed_rows(), 0, "D3: zero initial commit");
        assert_eq!(pool.ticks_committed, 0);
        assert_eq!(pool.data_committed, 0, "Z1 before any growth");
        let sigma = pool_base_stagger(ZST_TAG_ID.0);

        let buffer_before = pool.buffer_ptr();
        let added_before = pool.added_ticks_ptr();
        let changed_before = pool.changed_ticks_ptr();

        // First grow: 0 → one tick page (first vm.commit pair).
        assert!(pool.grow_rows(1), "grow within the ceiling succeeds");
        assert_eq!(pool.ticks_committed, P, "first commit = one tick page");
        assert_eq!(pool.committed_rows(), (P - sigma) / 4, "(P - σ) / 4 rows of 4 B ticks");
        #[cfg(target_arch = "x86_64")]
        assert_eq!(pool.committed_rows(), 448);
        assert_eq!(pool.data_committed, 0, "Z1 after the first commit");

        // Idempotent no-op arm below the frontier: zero state change.
        let frontier = (pool.ticks_committed, pool.committed_rows());
        assert!(pool.grow_rows(100), "no-op grow below the frontier");
        assert_eq!(
            (pool.ticks_committed, pool.committed_rows()),
            frontier,
            "idempotent arm must not move the frontier"
        );

        // Second grow: one row past the frontier — grow_rows_zst reaches
        // vm.commit AGAIN (the doubling arm: one tick page -> two).
        let n2 = pool.committed_rows() + 1;
        assert!(pool.grow_rows(n2), "second grow within the ceiling");
        assert_eq!(pool.ticks_committed, 2 * P, "STRICT frontier growth: P → 2·P (Z4)");
        assert_eq!(pool.committed_rows(), (2 * P - sigma) / 4, "(2P - σ) / 4 rows");
        assert!(pool.committed_rows() >= n2, "Z5: request covered");
        assert_eq!(pool.data_committed, 0, "Z1 after the second commit");

        // Third grow: n = 20,000 is past twice the frontier — the
        // request-dominant arm (one event covers the whole request).
        assert!(pool.grow_rows(20_000), "third grow within the ceiling");
        let t3 = (sigma + 20_000 * 4).next_multiple_of(P);
        assert_eq!(pool.ticks_committed, t3, "request-dominant: align_up_page(σ + 4n)");
        assert_eq!(pool.committed_rows(), (t3 - sigma) / 4, "(t - σ) / 4 rows");
        #[cfg(target_arch = "x86_64")]
        assert_eq!((pool.ticks_committed, pool.committed_rows()), (86_016, 20_928));
        assert!(pool.committed_rows() >= 20_000, "Z5: request covered");
        assert_eq!(pool.data_committed, 0, "Z1 after the third commit");

        // Base pointers never move across ZST growth (write-once contract).
        assert_eq!(pool.buffer_ptr(), buffer_before, "dangling data base stable");
        assert_eq!(pool.added_ticks_ptr() as usize % P, sigma, "G5: the tick base carries σ");
        assert_eq!(pool.added_ticks_ptr(), added_before, "added tick base stable");
        assert_eq!(pool.changed_ticks_ptr(), changed_before, "changed tick base stable");

        // Ceiling arm: false, zero state change (the shared grow_rows guard).
        let before = (pool.ticks_committed, pool.committed_rows());
        assert!(!pool.grow_rows(100_001), "grow past reserve_rows must fail");
        assert_eq!(
            (pool.ticks_committed, pool.committed_rows()),
            before,
            "rejected grow leaves the frontier untouched"
        );
    }

    // ----- Untracked backing: reserved-but-never-committed tick sub-regions -----

    /// THE untracked-backing gate: `new_untracked` commits the DATA sub-region
    /// exactly as a tracked pool does, and NEVER commits either tick
    /// sub-region.
    ///
    /// The TRACKED TWIN is the load-bearing half of this test, not decoration.
    /// An untracked-only assertion would pass just as happily if the new
    /// constructor had also broken data growth — "ticks never move" is trivially
    /// true of a pool that commits nothing at all. Comparing the two frontiers
    /// row for row is what pins the change to the tick axis alone.
    #[test]
    fn untracked_pool_commits_data_but_never_ticks() {
        use super::UNTRACKED_TICKS;

        component_registry::register_layout::<U64Pair>(U64_ID.0);
        let mut tracked = make_u64_pool(100_000);
        let mut untracked = ComponentPool::new_untracked(U64_ID.0, 100_000);

        assert!(tracked.is_tracked(), "the twin must be tracked");
        assert!(!untracked.is_tracked(), "new_untracked ⇒ !is_tracked");
        assert_eq!(
            untracked.ticks_committed, UNTRACKED_TICKS,
            "the sentinel is installed at construction"
        );
        assert_eq!(untracked.data_committed, 0, "D3: zero commit at construction");
        assert_eq!(untracked.committed_rows(), 0, "no rows before the first grow");

        let base_before = untracked.buffer_ptr();

        // Three growths crossing at least one commit step each way.
        for n in [1usize, 5_000, 40_000] {
            assert!(tracked.grow_rows(n), "tracked grow within the ceiling (n = {n})");
            assert!(untracked.grow_rows(n), "untracked grow within the ceiling (n = {n})");

            assert_eq!(
                untracked.ticks_committed, UNTRACKED_TICKS,
                "the tick frontier must NEVER move on an untracked pool (n = {n})"
            );
            assert_eq!(
                (untracked.data_committed, untracked.committed_rows()),
                (tracked.data_committed, tracked.committed_rows()),
                "data growth must be IDENTICAL to the tracked twin (n = {n})"
            );
            assert!(
                untracked.committed_rows() >= n,
                "the request must be covered (n = {n})"
            );
        }

        // Anti-vacuity: if the twin never committed a tick either, the equality
        // above compares two equally-empty pools and proves nothing.
        assert!(
            tracked.ticks_committed > 0,
            "anti-vacuity: the TRACKED twin must actually have committed ticks, \
             or the comparison above is vacuous"
        );

        assert_eq!(
            untracked.buffer_ptr(),
            base_before,
            "the data base stays address-stable across untracked growth"
        );
    }

    /// `new_untracked` REFUSES a ZST, loudly, at construction.
    ///
    /// `grow_rows_zst` derives row capacity from the tick sub-regions alone
    /// (`data_committed` is invariantly 0 on that path), so an untracked ZST
    /// pool could never grow past zero rows. Silently capping at 0 rows is
    /// exactly the "green from emptiness" shape this tree keeps catching, so the
    /// constructor refuses instead.
    #[test]
    #[should_panic(expected = "a ZST (stride == 0) pool is tick-driven")]
    fn untracked_pool_refuses_a_zst() {
        component_registry::register_layout::<ZstTag>(ZST_TAG_ID.0);
        let _ = ComponentPool::new_untracked(ZST_TAG_ID.0, 1_024);
    }

    /// Reading a tick from an untracked pool is refused by the guard rather than
    /// faulting on reserved-uncommitted pages.
    ///
    /// This is the test that gives the ten `debug_assert!(self.is_tracked())`
    /// guards their value: without it they are unexercised prose, and a future
    /// edit could delete them all and stay green.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "tick access on an UNTRACKED pool")]
    fn untracked_pool_refuses_tick_access() {
        component_registry::register_layout::<U64Pair>(U64_ID.0);
        let mut pool = ComponentPool::new_untracked(U64_ID.0, 1_024);
        assert!(pool.grow_rows(1), "grow so the DATA row exists");
        // SAFETY: index 0 < committed_rows after the grow above; the call is
        // expected to trip the untracked guard before any tick is touched.
        let _ = unsafe { pool.read_added_tick(0) };
    }

    // ----- Phase 4 Seam 3: PoolBacking -----

    /// A `PoolBacking::Host` pool round-trips add + `row_ptr` byte-identically
    /// to the pre-Phase-4 behavior (the swap is a no-op for the Host arm).
    #[test]
    fn host_backing_round_trips_add_and_row_ptr() {
        let mut pool = make_u64_pool(8);

        let r0 = U64Pair { a: 1, b: 2 };
        let r1 = U64Pair { a: 3, b: 4 };
        let i0 = pool
            .add(u64pair_bytes(&r0))
            .expect("capacity 8 holds the first row");
        let i1 = pool
            .add(u64pair_bytes(&r1))
            .expect("capacity 8 holds the second row");
        assert_eq!((i0, i1), (0, 1), "dense row indices");

        // Read back through the same row_ptr the hot path uses.
        // SAFETY: rows 0 and 1 are live (just added) and < committed_rows; each
        // slot holds a properly-aligned, initialized U64Pair.
        let (v0, v1) = unsafe {
            (
                *pool.row_ptr(0).cast::<U64Pair>(),
                *pool.row_ptr(1).cast::<U64Pair>(),
            )
        };
        assert_eq!((v0, v1), (r0, r1), "Host backing round-trips the bytes");
    }

    /// CR-C: a `Device`-backed pool keeps Host `len == 0` for life, so its `Drop`
    /// runs the CPU `drop_fn` ZERO times even for a `needs_drop` layout — device
    /// teardown is the boxed `DeviceColumn`'s drop, never the CPU `drop_fn`.
    #[cfg(not(miri))]
    #[test]
    fn device_pool_host_len_stays_zero_on_drop() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        component_registry::register_layout::<Dropper>(DROPPER_ID.0);
        // A needs_drop layout: if the swap ever let the CPU drop_fn run over a
        // device pool, the counter would tick.
        let mut pool = ComponentPool::new(DROPPER_ID.0, 16);
        assert_eq!(pool.len, 0, "fresh pool has no live rows");

        // Switch the empty pool to the stub Device backing (CR-C: len == 0).
        pool.make_device_backed_for_test(0xDEAD_BEEF);
        assert!(pool.backing.is_device(), "backing switched to Device");
        assert_eq!(pool.len, 0, "Device pool keeps Host len == 0 (CR-C)");

        let counter = Arc::new(AtomicUsize::new(0));
        // We do NOT add any row through the Host path (a device pool's rows live
        // device-side; Host len stays 0). Keep `counter` referenced so the test
        // type is exercised; assert the drop_fn never fires.
        let _probe = Dropper {
            counter: Arc::clone(&counter),
        };
        assert_eq!(counter.load(Ordering::Relaxed), 0, "no drop before teardown");

        drop(pool); // Device pool: drop_fn loop is a no-op (len == 0).
        // `_probe` is still alive here; only its own scope-exit drop fires (==1),
        // never the pool's drop_fn — prove the pool contributed ZERO drops.
        drop(_probe);
        assert_eq!(
            counter.load(Ordering::Relaxed),
            1,
            "only the local probe dropped (==1); the Device pool's CPU drop_fn \
             ran 0 times (CR-C: Host len == 0)"
        );
    }

    // ====================================================================
    // Packing plan S0 — the commit-floor oracle
    // (`docs/ecs/POOL-SUBGRANULAR-PACKING-PLAN.md`, gates G1, G2, G3, G7).
    //
    // The `cfg` below is a recorded "not compiled here", never a pass (plan
    // G2): on the fallback arm (Miri, wasm, 32-bit) `VmReservation::reserve`
    // is an eager `alloc_zeroed` of the whole `os_len` and `commit` is a
    // no-op, so no commit floor exists there to measure. The ladder
    // bookkeeping itself is audited under Miri by `tests/miri_pool_growth.rs`
    // and `tests/miri_phase22.rs`.
    // ====================================================================
    #[cfg(all(not(miri), any(windows, unix), target_pointer_width = "64"))]
    mod commit_floor_tests {
        //! Commit-floor oracle for `ComponentPool` (packing plan S0).
        //!
        //! * **G1** — the in-process model `ComponentPool::committed_bytes`,
        //!   pinned by exact equality against the plan's closed form ("The
        //!   floor, derived"), never by a bound.
        //! * **G2** — anti-vacuity: one more grow moves the model by exactly one
        //!   page, and an idempotent grow moves it by exactly zero.
        //! * **G3** — OS truth: the bytes the kernel reports committed inside
        //!   one pool's reservation (`VirtualQuery` on Windows,
        //!   `/proc/self/maps` on Linux) equal the model after every growth
        //!   event of every geometry in the plan's obligation-10 table, so the
        //!   model cannot pass by restating the arithmetic it models.
        //! * **G7** — the grow-event count of a one-row-at-a-time fill to 1 M
        //!   rows, pinned exactly (an equality also catches a ladder widened
        //!   to ×4, which a `<=` bound cannot).
        //! * **G8** — every growth event of 178 pools (two request sequences
        //!   for each of 89 `(stride, σ)` cases) against a test-local D2
        //!   ladder derived from the plan's formulas, the model, and the OS.
        //!
        //! # Component ids
        //!
        //! One registry per process, one layout per slot: a fixed id that a
        //! derive mint later lands on panics loudly in `register_new`, and a
        //! fixed id another test pins to a different type panics in
        //! `register_layout`. Every id below was checked against the lib test
        //! binary's census of fixed ids (`register_layout` and `ComponentId(N)`
        //! in `src/**`, bands 100-109, 150-159, 200-209, 220-229, 300-359,
        //! 400-511 taken) and sits above the derive-mint range, which
        //! `ecs_master`'s own fixed ids at 100..=102 already bound from above.
        //! The stagger `σ = (id % 64) * 64` is what each id is chosen for:
        //!
        //! | ids | layout | σ | used by |
        //! |---|---|---|---|
        //! | 230..=293 | `Floor40`, 40 B | one id per residue, all 64 | G1 (233..=248, 240 untracked), G2, G3, G7 (256 σ 0, 281 σ 1600, 255 σ 4032) |
        //! | 128 / 127 | `Floor1`, 1 B | 0 / 4032 | G1 tiny strides |
        //! | 192 / 191 | `Floor2`, 2 B | 0 / 4032 | G1 tiny strides |
        //! | 224, 226, 223, 228 | the `tests` fixtures (16 B, 64 B, 4 B, ZST) | 2048, 2176, 1984, 2304 | G3 rows 2-5 and the ZST tick-cap row; G8 |
        //! | 129 | `Floor1` | 64 | G8 |
        //! | 193, 190 | `F32Wrap`, 4 B | 64, 3968 | G8 |
        //! | 222, 385, 189 | `OtherComponent`, 8 B | 1920, 64, 3904 | G8 |
        //! | 449, 318 | `U64Pair`, 16 B | 64, 3968 | G8 |
        //! | 382 | `Stride64`, 64 B | 3968 | G8 |
        //! | 319, 188, 130 | `Floor256` | 4032, 3840, 128 | G8 |
        //! | 383, 176, 384 | `Floor1K` | 4032, 3072, 0 | G8 |
        //! | 447, 448, 162 | `Floor4K` | 4032, 0, 2176 | G8 |
        //!
        //! G8 cannot hold every stride at every id (critique W1: 512 slots, one
        //! layout each), so it samples σ per stride: all 64 residues for
        //! stride 40, and for the others the values that stride's arithmetic
        //! turns on — both phases of the tick-driven strides 1 and 2; near-0
        //! and near-4032 for the strides whose first data page never
        //! overflows; and for 256, 1024 and 4096 a σ where `σ + stride`
        //! crosses a page (4032, and 2176 at 4096), one where it fits exactly
        //! (3840, 3072, 0), and one more (128, 0, 2176).
        //!
        //! A σ-independent assertion takes its expectation from the plan's
        //! formula in `COMMIT_PAGE`; the x86_64 literal the plan quotes is
        //! pinned beside it, so the number the documents cite is the number
        //! this gate checks.

        #[cfg(windows)]
        use core::mem::MaybeUninit;
        use core::ops::RangeInclusive;

        use super::{
            F32_WRAP_ID, F32Wrap, OtherComponent, Stride64, U64Pair, ZST_TAG_ID, ZstTag,
            make_stride64_pool, make_u64_pool, make_zst_pool,
        };
        use crate::ecs::constants::{
            COMMIT_GRANULE, COMMIT_PAGE, POOL_MAX_ROWS, POOL_MAX_SLAB, pool_base_stagger,
            pool_reserve_rows,
        };
        use crate::ecs::core::component::component_registry;
        use crate::ecs::identifiers::primitives::EntityId;
        use crate::ecs::memory::component_pool::{ComponentPool, PoolBacking};
        use crate::ecs::memory::vm::VmReservation;
        use crate::ecs::memory::vm_column::VmColumn;

        // Layout-only fixtures: registered for their size and never
        // constructed, because the oracle grows pools with `grow_rows` and
        // never writes a row.
        #[allow(dead_code)]
        #[repr(C)]
        struct Floor40([u64; 5]);
        #[allow(dead_code)]
        #[repr(C)]
        struct Floor1(u8);
        #[allow(dead_code)]
        #[repr(C)]
        struct Floor2(u16);
        #[allow(dead_code)]
        #[repr(C)]
        struct Floor256([u64; 32]);
        #[allow(dead_code)]
        #[repr(C)]
        struct Floor1K([u64; 128]);
        #[allow(dead_code)]
        #[repr(C)]
        struct Floor4K([u64; 512]);

        /// One `Floor40` id per stagger residue.
        const FLOOR40_BAND: RangeInclusive<usize> = 230..=293;
        /// G1's sixteen σ != 0 columns (σ = 2624 ..= 3584).
        const G1_IDS: RangeInclusive<usize> = 233..=248;
        const SIGMA_0: usize = 256;
        const SIGMA_1600: usize = 281;
        const SIGMA_4032: usize = 255;
        const UNTRACKED_40: usize = 240;
        const STRIDE1_SIGMA_0: usize = 128;
        const STRIDE1_SIGMA_4032: usize = 127;
        const STRIDE2_SIGMA_0: usize = 192;
        const STRIDE2_SIGMA_4032: usize = 191;

        /// A default-sized pool (`with_default_sizes`: the 16-column fixture
        /// must be default-sized, plan S0) for `T` registered at `id`.
        fn default_pool<T: 'static>(id: usize) -> ComponentPool {
            component_registry::register_layout::<T>(id);
            ComponentPool::with_default_sizes(id)
        }

        fn floor40_pool(id: usize) -> ComponentPool {
            assert!(FLOOR40_BAND.contains(&id), "Floor40 ids live in FLOOR40_BAND (id {id})");
            default_pool::<Floor40>(id)
        }

        /// "The floor, derived" (packing plan): `(committed_rows,
        /// committed bytes)` after the FIRST grow to one row, from `σ`, the
        /// stride and the tracked flag alone. `stride == 0` is a tracked ZST
        /// (two tick regions, no data), valid while `σ + 4 <= COMMIT_PAGE`.
        fn closed_form_floor(sigma: usize, stride: usize, tracked: bool) -> (usize, usize) {
            let p = COMMIT_PAGE;
            if stride == 0 {
                return ((p - sigma) / 4, 2 * p);
            }
            let data_pages = (sigma + stride).div_ceil(p);
            let rows = (data_pages * p - sigma) / stride;
            if !tracked {
                return (rows, data_pages * p);
            }
            let tick_pages = (sigma + 4 * rows).div_ceil(p);
            (rows, (data_pages + 2 * tick_pages) * p)
        }

        /// The SUM of the three frontiers — what the union model is not.
        /// `sum - committed_bytes()` is the number of shared boundary pages.
        fn frontier_sum(pool: &ComponentPool) -> usize {
            let ticks = if pool.is_tracked() { pool.ticks_committed } else { 0 };
            pool.data_committed + 2 * ticks
        }

        /// Walks `pool` up its commit ladder exactly as a one-row-at-a-time
        /// `add` loop does — `add` grows only when `count == committed_rows`,
        /// and then asks for `committed_rows + 1` — until the frontier covers
        /// `target`, calling `after_each` after every growth event. Returns the
        /// number of events.
        fn climb(
            pool: &mut ComponentPool,
            target: usize,
            mut after_each: impl FnMut(&ComponentPool, usize),
        ) -> usize {
            let mut events = 0;
            while pool.committed_rows() < target {
                let n = pool.committed_rows() + 1;
                assert!(pool.grow_rows(n), "grow_rows({n}) below the ceiling must succeed");
                events += 1;
                after_each(pool, events);
            }
            events
        }

        fn host_vm(pool: &ComponentPool) -> &VmReservation {
            match &pool.backing {
                PoolBacking::Host(vm) => vm,
                PoolBacking::Device(_) => unreachable!("commit-floor fixtures are host pools"),
            }
        }

        /// Bytes the kernel reports committed inside `pool`'s reservation:
        /// a `VirtualQuery` walk over `[base, base + os_len)` summing the
        /// `MEM_COMMIT` regions, clipped to the reservation.
        #[cfg(windows)]
        fn os_committed_bytes(pool: &ComponentPool) -> usize {
            use core::ffi::c_void;

            /// `MEMORY_BASIC_INFORMATION` on Win64. Every field is declared so
            /// the layout is the kernel's; the walk reads three of them.
            #[allow(dead_code)]
            #[repr(C)]
            struct MemoryBasicInformation {
                base_address: *mut c_void,
                allocation_base: *mut c_void,
                allocation_protect: u32,
                partition_id: u16,
                region_size: usize,
                state: u32,
                protect: u32,
                kind: u32,
            }
            const _: () = assert!(size_of::<MemoryBasicInformation>() == 48);
            const MEM_COMMIT: u32 = 0x1000;

            // SAFETY: the signature matches kernel32's `VirtualQuery` on Win64
            // exactly (LPCVOID -> *const c_void, PMEMORY_BASIC_INFORMATION ->
            // *mut MemoryBasicInformation, SIZE_T -> usize); kernel32 is
            // linked transitively by std.
            unsafe extern "system" {
                fn VirtualQuery(
                    address: *const c_void,
                    buffer: *mut MemoryBasicInformation,
                    length: usize,
                ) -> usize;
            }

            let vm = host_vm(pool);
            let start = vm.base().as_ptr() as usize;
            let end = start + vm.os_len();
            let mut at = start;
            let mut committed = 0;
            while at < end {
                let mut info = MaybeUninit::<MemoryBasicInformation>::uninit();
                // SAFETY: `at` lies inside this pool's live reservation
                // `[start, end)`; `info` is a writable buffer of exactly the
                // length passed; the call only writes into that buffer.
                let written = unsafe {
                    VirtualQuery(
                        at as *const c_void,
                        info.as_mut_ptr(),
                        size_of::<MemoryBasicInformation>(),
                    )
                };
                assert_eq!(
                    written,
                    size_of::<MemoryBasicInformation>(),
                    "VirtualQuery failed at {at:#x}"
                );
                // SAFETY: a return value equal to the buffer length means the
                // kernel wrote every field of the struct.
                let info = unsafe { info.assume_init() };
                let region_end = (info.base_address as usize + info.region_size).min(end);
                if info.state == MEM_COMMIT {
                    committed += region_end - at;
                }
                at = region_end;
            }
            committed
        }

        /// Bytes mapped read/write inside `pool`'s reservation according to
        /// `/proc/self/maps`: the `rw` extents clipped to `[base, base +
        /// os_len)`. A monotone prefix of equal-protection `mprotect`s merges
        /// into one VMA, and a VMA can merge across the reservation's edge
        /// with an unrelated mapping, hence the clip.
        #[cfg(target_os = "linux")]
        fn os_committed_bytes(pool: &ComponentPool) -> usize {
            let vm = host_vm(pool);
            let start = vm.base().as_ptr() as usize;
            let end = start + vm.os_len();
            let maps = std::fs::read_to_string("/proc/self/maps")
                .expect("/proc/self/maps is readable on Linux");
            let mut committed = 0;
            for line in maps.lines() {
                let mut fields = line.split_whitespace();
                let (Some(range), Some(perms)) = (fields.next(), fields.next()) else {
                    continue;
                };
                let Some((lo, hi)) = range.split_once('-') else {
                    continue;
                };
                let lo = usize::from_str_radix(lo, 16).expect("maps range start is hex");
                let hi = usize::from_str_radix(hi, 16).expect("maps range end is hex");
                let (lo, hi) = (lo.max(start), hi.min(end));
                if lo < hi && perms.starts_with("rw") {
                    committed += hi - lo;
                }
            }
            committed
        }

        // The OS-truth arm exists on Windows and Linux only: the syscall arm
        // of `vm.rs` is `unix`, but `/proc/self/maps` is Linux's, so on every
        // other unix target the G3 tests below are not compiled — a recorded
        // absence (this comment), never a vacuous green.
        #[cfg(any(windows, target_os = "linux"))]
        fn assert_os_truth(pool: &ComponentPool, what: &str) {
            assert_eq!(
                os_committed_bytes(pool),
                pool.committed_bytes(),
                "G3 OS truth vs the D7 model ({what}; committed_rows = {}, data_committed = {}, \
                 ticks_committed = {})",
                pool.committed_rows(),
                pool.data_committed,
                pool.ticks_committed
            );
        }

        // ---- G1 -------------------------------------------------------------

        /// G1 — sixteen default-sized 40 B tracked columns with σ != 0, one
        /// row each: three pages per column (data, added, changed), 196 608 B
        /// on x86_64. The granule ladder read 3 145 728 B here: 16 × 3 ×
        /// 64 KiB of sub-region-relative frontiers.
        #[test]
        fn floor_of_sixteen_small_tracked_columns() {
            let mut total = 0;
            for id in G1_IDS {
                assert_ne!(pool_base_stagger(id), 0, "G1 needs σ != 0 (id {id})");
                let mut pool = floor40_pool(id);
                assert!(pool.grow_rows(1));
                total += pool.committed_bytes();
            }
            assert_eq!(total, 16 * 3 * COMMIT_PAGE, "G1: sixteen σ != 0 columns, one row each");
            #[cfg(target_arch = "x86_64")]
            assert_eq!(total, 196_608);
        }

        /// G1 — the floor is the same three pages at σ = 0 and at σ = 4032:
        /// the page floor absorbs the stagger pad (plan D2, D4). Only the row
        /// count differs: `(COMMIT_PAGE - σ) / 40`.
        #[test]
        fn floor_is_three_pages_at_both_stagger_extremes() {
            for (id, sigma) in [(SIGMA_0, 0), (SIGMA_4032, 4032)] {
                assert_eq!(pool_base_stagger(id), sigma);
                let mut pool = floor40_pool(id);
                assert!(pool.grow_rows(1));
                assert_eq!(
                    (pool.committed_rows(), pool.committed_bytes()),
                    closed_form_floor(sigma, 40, true),
                    "G1: σ = {sigma}"
                );
                assert_eq!(pool.committed_bytes(), 3 * COMMIT_PAGE, "G1: σ = {sigma}");
            }
            #[cfg(target_arch = "x86_64")]
            assert_eq!(
                [closed_form_floor(0, 40, true), closed_form_floor(4032, 40, true)],
                [(102, 12_288), (1, 12_288)]
            );
        }

        /// G1 — an untracked column (`ScratchColumn`'s backing) costs its data
        /// page alone.
        #[test]
        fn floor_of_an_untracked_column_is_its_data_page() {
            component_registry::register_layout::<Floor40>(UNTRACKED_40);
            let mut pool = ComponentPool::new_untracked(UNTRACKED_40, pool_reserve_rows(40));
            assert!(pool.grow_rows(1));
            assert_eq!(
                (pool.committed_rows(), pool.committed_bytes()),
                closed_form_floor(pool_base_stagger(UNTRACKED_40), 40, false)
            );
            assert_eq!(pool.committed_bytes(), COMMIT_PAGE);
        }

        /// G1 — a tracked ZST (tag) column costs its two tick pages.
        #[test]
        fn floor_of_a_tracked_zst_column_is_two_tick_pages() {
            let mut pool = default_pool::<ZstTag>(ZST_TAG_ID.0);
            assert!(pool.grow_rows(1));
            let sigma = pool_base_stagger(ZST_TAG_ID.0);
            assert_eq!(
                (pool.committed_rows(), pool.committed_bytes()),
                closed_form_floor(sigma, 0, true)
            );
            assert_eq!(pool.committed_bytes(), 2 * COMMIT_PAGE);
            #[cfg(target_arch = "x86_64")]
            assert_eq!(pool.committed_rows(), 448, "(4096 - 2304) / 4");
        }

        /// G1 — a four-column 40 B table plus its `VmColumn<EntityId>`
        /// (OPEN-QUESTIONS F2's unit): 4 × 3 pages + 1 page.
        #[test]
        fn floor_of_a_four_column_table_and_its_entity_column() {
            let mut total = 0;
            for id in G1_IDS.take(4) {
                let mut pool = floor40_pool(id);
                assert!(pool.grow_rows(1));
                total += pool.committed_bytes();
            }
            let mut entity_ids: VmColumn<EntityId> =
                VmColumn::new("commit_floor_tests.entity_ids", POOL_MAX_ROWS);
            entity_ids.push(EntityId(0));
            total += entity_ids.committed_elems() * size_of::<EntityId>();
            assert_eq!(total, 13 * COMMIT_PAGE, "G1: 4 × 3 pages + one entity-id page");
            #[cfg(target_arch = "x86_64")]
            assert_eq!(total, 53_248);
        }

        /// G1 — 1 B and 2 B columns: the tick pages drive the floor (plan
        /// D6), phase-dependent on σ, so the closed form pins them.
        #[test]
        fn tiny_stride_floors_are_driven_by_the_tick_pages() {
            let cases = [
                (STRIDE1_SIGMA_0, 1),
                (STRIDE1_SIGMA_4032, 1),
                (STRIDE2_SIGMA_0, 2),
                (STRIDE2_SIGMA_4032, 2),
            ];
            let mut floors = [0usize; 4];
            for (k, (id, stride)) in cases.into_iter().enumerate() {
                let mut pool = if stride == 1 {
                    default_pool::<Floor1>(id)
                } else {
                    default_pool::<Floor2>(id)
                };
                assert!(pool.grow_rows(1));
                let expected = closed_form_floor(pool_base_stagger(id), stride, true);
                assert_eq!(
                    (pool.committed_rows(), pool.committed_bytes()),
                    expected,
                    "G1: stride {stride}, id {id}"
                );
                floors[k] = expected.1;
            }
            #[cfg(target_arch = "x86_64")]
            assert_eq!(floors, [36_864, 20_480, 20_480, 20_480]);
        }

        // ---- G2 -------------------------------------------------------------

        /// G2 (a) — a model stuck at a constant is red: the second rung of a
        /// σ = 0 column moves the model by exactly one page (the data page
        /// doubles; the tick pages already cover the new rows).
        #[test]
        fn one_more_grow_moves_the_model_by_exactly_one_page() {
            let mut pool = floor40_pool(SIGMA_0);
            assert!(pool.grow_rows(1));
            let (rows, bytes) = (pool.committed_rows(), pool.committed_bytes());
            assert!(pool.grow_rows(rows + 1));
            assert_eq!(pool.committed_bytes() - bytes, COMMIT_PAGE, "G2(a): one more rung");
            assert_eq!(pool.committed_rows(), 2 * COMMIT_PAGE / 40, "G2(a): σ = 0 rows");
        }

        /// G2 (b) — a model that double-counts is red: an idempotent grow at
        /// or below the frontier moves nothing.
        #[test]
        fn idempotent_grow_below_the_frontier_moves_the_model_by_zero() {
            let mut pool = floor40_pool(SIGMA_1600);
            assert!(pool.grow_rows(1));
            let state = |p: &ComponentPool| {
                (p.committed_rows(), p.committed_bytes(), p.data_committed, p.ticks_committed)
            };
            let before = state(&pool);
            for n in [0, 1, before.0] {
                assert!(pool.grow_rows(n));
                assert_eq!(state(&pool), before, "G2(b): grow_rows({n}) is a no-op");
            }
        }

        // ---- G3 -------------------------------------------------------------

        /// G3 — the floor of a σ != 0 column, as the kernel sees it. The
        /// granule ladder's `align_down`/`align_up` pair committed two granules
        /// per sub-region here (393 216 B against a 196 608 B model); only
        /// this arm can see that overshoot.
        #[cfg(any(windows, target_os = "linux"))]
        #[test]
        fn os_truth_equals_the_model_at_the_floor_with_nonzero_stagger() {
            let mut pool = floor40_pool(SIGMA_1600);
            assert!(pool.grow_rows(1));
            assert_os_truth(&pool, "floor, σ = 1600");
            assert_eq!(pool.committed_bytes(), 3 * COMMIT_PAGE);
        }

        /// G3 — the σ = 0 twin: page floors coincide with the sub-region
        /// offsets, so no overshoot existed here even on the granule ladder.
        #[cfg(any(windows, target_os = "linux"))]
        #[test]
        fn os_truth_equals_the_model_at_the_floor_with_zero_stagger() {
            let mut pool = floor40_pool(SIGMA_0);
            assert!(pool.grow_rows(1));
            assert_os_truth(&pool, "floor, σ = 0");
        }

        /// G3 obligation-10 row 2 — `make_stride64_pool(4096)` (σ = 2176) to
        /// full capacity: the data ladder reaches its cap `data_len + P` by
        /// request, so the data interval shares the `added` floor page.
        #[cfg(any(windows, target_os = "linux"))]
        #[test]
        fn os_truth_row2_stride64_to_full_capacity() {
            let mut pool = make_stride64_pool(4096);
            climb(&mut pool, 4096, |p, k| assert_os_truth(p, &format!("row 2, event {k}")));
            assert_eq!(pool.committed_rows(), 4096);
            assert_eq!(frontier_sum(&pool) - pool.committed_bytes(), COMMIT_PAGE, "row 2");
            #[cfg(target_arch = "x86_64")]
            assert_eq!((pool.committed_bytes(), frontier_sum(&pool)), (303_104, 307_200));
        }

        /// G3 obligation-10 row 3 — `make_u64_pool(256)` (σ = 2048) to full
        /// capacity: neither cap is reached, so the union is the sum.
        #[cfg(any(windows, target_os = "linux"))]
        #[test]
        fn os_truth_row3_u64_to_full_capacity() {
            let mut pool = make_u64_pool(256);
            climb(&mut pool, 256, |p, k| assert_os_truth(p, &format!("row 3, event {k}")));
            assert_eq!(pool.committed_rows(), 256);
            assert_eq!(frontier_sum(&pool), pool.committed_bytes(), "row 3");
            #[cfg(target_arch = "x86_64")]
            assert_eq!(pool.committed_bytes(), 16_384);
        }

        /// G3 obligation-10 row 4 — `pool_byte_layout(3072, 64, 2176)` climbed
        /// one rung at a time to `n = 2015`: the doubling from `2G` overshoots
        /// onto the data cap `3G + P` BEFORE full capacity (route (ii)). The
        /// climb is load-bearing: one `grow_rows(2015)` from empty lands at
        /// 135 168 B, where union == sum, and would pass for the wrong reason.
        #[cfg(any(windows, target_os = "linux"))]
        #[test]
        fn os_truth_row4_doubling_overshoots_onto_the_data_cap() {
            let mut pool = make_stride64_pool(3072);
            climb(&mut pool, 2015, |p, k| assert_os_truth(p, &format!("row 4, event {k}")));
            assert_eq!(pool.committed_rows(), 3072, "row 4: the overshoot reaches the ceiling");
            assert_eq!(
                frontier_sum(&pool) - pool.committed_bytes(),
                COMMIT_PAGE,
                "row 4: the data/added boundary page must be shared (anti-vacuity)"
            );
            #[cfg(target_arch = "x86_64")]
            assert_eq!((pool.committed_bytes(), frontier_sum(&pool)), (229_376, 233_472));
        }

        /// G3 obligation-10 row 5 — `pool_byte_layout(16384, 4, 1984)` to full
        /// capacity: both caps reached, two shared pages — the cheap twin of
        /// row 1's both-caps case.
        #[cfg(any(windows, target_os = "linux"))]
        #[test]
        fn os_truth_row5_both_caps_to_full_capacity() {
            component_registry::register_layout::<F32Wrap>(F32_WRAP_ID.0);
            let mut pool = ComponentPool::new(F32_WRAP_ID.0, 16_384);
            climb(&mut pool, 16_384, |p, k| assert_os_truth(p, &format!("row 5, event {k}")));
            assert_eq!(pool.committed_rows(), 16_384);
            assert_eq!(frontier_sum(&pool) - pool.committed_bytes(), 2 * COMMIT_PAGE, "row 5");
            #[cfg(target_arch = "x86_64")]
            assert_eq!((pool.committed_bytes(), frontier_sum(&pool)), (200_704, 208_896));
        }

        /// G3 — a tracked ZST pool (σ = 2304) driven to its tick cap
        /// `align_up_page(σ + tick_len)`, the ZST form of obligation 3's error
        /// class: the `added` interval shares the `changed` floor page.
        #[cfg(any(windows, target_os = "linux"))]
        #[test]
        fn os_truth_zst_to_the_tick_cap() {
            let mut pool = make_zst_pool(16_384);
            climb(&mut pool, 16_384, |p, k| assert_os_truth(p, &format!("ZST, event {k}")));
            assert_eq!(pool.committed_rows(), 16_384);
            assert_eq!(frontier_sum(&pool) - pool.committed_bytes(), COMMIT_PAGE, "ZST tick cap");
            #[cfg(target_arch = "x86_64")]
            assert_eq!(pool.committed_bytes(), 135_168);
        }

        /// G3 obligation-10 row 1 — the default 40 B pool (σ = 1600) at its full
        /// 2^24-row capacity in one request: both caps reached, two shared
        /// pages. Ignored for its commit charge, not its time: ≈ 768 MiB of
        /// process-wide Windows commit in a binary that runs other tests in
        /// parallel. Row 5 gates the same both-caps shape on every run.
        #[cfg(any(windows, target_os = "linux"))]
        #[test]
        #[ignore = "solo: commits ~768 MiB (a default 40 B pool at its full 2^24-row capacity), process-wide commit charge; run alone: `cargo test -p boyko-ecs --lib ecs::memory::component_pool::tests::commit_floor_tests::os_truth_row1_default_pool_at_full_capacity -- --ignored --exact --test-threads=1`"]
        fn os_truth_row1_default_pool_at_full_capacity() {
            let mut pool = floor40_pool(SIGMA_1600);
            assert!(pool.grow_rows(POOL_MAX_ROWS));
            assert_eq!(pool.committed_rows(), POOL_MAX_ROWS);
            assert_os_truth(&pool, "row 1, full capacity");
            assert_eq!(frontier_sum(&pool) - pool.committed_bytes(), 2 * COMMIT_PAGE, "row 1");
            #[cfg(target_arch = "x86_64")]
            assert_eq!(pool.committed_bytes(), 805_310_464);
        }

        // ---- G7 -------------------------------------------------------------

        /// G7 — a one-row-at-a-time fill of a 40 B column to 1 M rows takes
        /// exactly 15 growth events on the page ladder (11 on the granule
        /// ladder), for σ = 0, 1600 and 4032 alike: `4e7 + σ < 2^26`. The
        /// plan's bound is `<= log2(1M·40 / COMMIT_PAGE) + 2 == 16`; the pin is
        /// the equality.
        #[test]
        fn grow_event_count_to_a_million_rows() {
            for id in [SIGMA_0, SIGMA_1600, SIGMA_4032] {
                let mut pool = floor40_pool(id);
                let events = climb(&mut pool, 1_000_000, |_, _| {});
                #[cfg(target_arch = "x86_64")]
                assert_eq!(events, 15, "G7: σ = {}", pool_base_stagger(id));
                assert!(events <= 16, "G7: the plan's bound (σ = {})", pool_base_stagger(id));
            }
        }

        /// G7 — a batch request is ONE event whatever the quantum:
        /// `pool_commit_step` is request-dominant.
        #[test]
        fn a_batch_request_is_one_event() {
            let mut pool = floor40_pool(SIGMA_1600);
            assert!(pool.grow_rows(1_000_000));
            assert!(pool.committed_rows() >= 1_000_000, "G7: one request covers the batch");
        }

        // ---- G8 -------------------------------------------------------------

        /// G8's `(id, stride)` sample beyond the stride-40 band (the module
        /// header's table says what each σ is chosen for).
        const G8_CASES: [(usize, usize); 25] = [
            (128, 1),
            (127, 1),
            (129, 1),
            (192, 2),
            (191, 2),
            (223, 4),
            (193, 4),
            (190, 4),
            (222, 8),
            (385, 8),
            (189, 8),
            (224, 16),
            (449, 16),
            (318, 16),
            (226, 64),
            (382, 64),
            (319, 256),
            (188, 256),
            (130, 256),
            (383, 1024),
            (176, 1024),
            (384, 1024),
            (447, 4096),
            (448, 4096),
            (162, 4096),
        ];

        /// Every G8 pool's explicit row ceiling: `n` ranges over `1..=10_000`.
        const G8_RESERVE: usize = 10_000;

        fn register_stride(id: usize, stride: usize) {
            match stride {
                1 => component_registry::register_layout::<Floor1>(id),
                2 => component_registry::register_layout::<Floor2>(id),
                4 => component_registry::register_layout::<F32Wrap>(id),
                8 => component_registry::register_layout::<OtherComponent>(id),
                16 => component_registry::register_layout::<U64Pair>(id),
                40 => component_registry::register_layout::<Floor40>(id),
                64 => component_registry::register_layout::<Stride64>(id),
                256 => component_registry::register_layout::<Floor256>(id),
                1024 => component_registry::register_layout::<Floor1K>(id),
                4096 => component_registry::register_layout::<Floor4K>(id),
                _ => unreachable!("no G8 fixture of stride {stride}"),
            }
            assert_eq!(
                component_registry::get_component_size(id),
                Some(stride),
                "G8 fixture at id {id}"
            );
        }

        /// G8's oracle: the D2 ladder re-derived from `(σ, stride,
        /// reserve_rows)` with the plan's formulas, independently of
        /// `grow_rows`. Its doubling floor is `COMMIT_PAGE` itself, not
        /// `POOL_MIN_SLAB`, so the recorded G2(c) mutation of that constant
        /// cannot carry the oracle along with the code.
        struct Ladder {
            sigma: usize,
            stride: usize,
            reserve_rows: usize,
            data_len: usize,
            tick_len: usize,
            data: usize,
            ticks: usize,
            rows: usize,
        }

        impl Ladder {
            fn new(sigma: usize, stride: usize, reserve_rows: usize) -> Self {
                Self {
                    sigma,
                    stride,
                    reserve_rows,
                    data_len: (reserve_rows * stride).next_multiple_of(COMMIT_GRANULE),
                    tick_len: (reserve_rows * 4).next_multiple_of(COMMIT_GRANULE),
                    data: 0,
                    ticks: 0,
                    rows: 0,
                }
            }

            fn grow(&mut self, n: usize) {
                if n > self.reserve_rows || n <= self.rows {
                    return;
                }
                let page = COMMIT_PAGE;
                let needed = (self.sigma + n * self.stride).next_multiple_of(page);
                let step = self.data.clamp(page, POOL_MAX_SLAB).max(needed - self.data);
                let data_cap = (self.sigma + self.data_len).next_multiple_of(page);
                self.data = (self.data + step).min(data_cap);
                self.rows = ((self.data - self.sigma) / self.stride).min(self.reserve_rows);
                self.ticks = self.ticks.max((self.sigma + 4 * self.rows).next_multiple_of(page));
            }

            /// The union of the three intervals, computed as the sum minus the
            /// two boundary overlaps (the data interval ends at most one page
            /// past `data_len`, so it never reaches the `changed` floor) — a
            /// different computation from `committed_bytes()`'s merge.
            fn union_bytes(&self) -> usize {
                let data_added = self.data.saturating_sub(self.data_len).min(self.ticks);
                let added_changed = self.ticks.saturating_sub(self.tick_len);
                self.data + 2 * self.ticks - data_added - added_changed
            }
        }

        /// One G8 step: grow both the pool and the oracle to `n`, then hold
        /// the pool to the oracle, to the model, and to the OS.
        fn grow_and_check(pool: &mut ComponentPool, ladder: &mut Ladder, n: usize, what: &str) {
            assert!(pool.grow_rows(n), "G8 {what}: grow_rows({n}) within the ceiling");
            ladder.grow(n);
            assert_eq!(
                (pool.committed_rows(), pool.data_committed, pool.ticks_committed),
                (ladder.rows, ladder.data, ladder.ticks),
                "G8 {what}, n = {n}: (committed_rows, data_committed, ticks_committed) vs the \
                 D2 ladder"
            );
            assert!(pool.committed_rows() >= n, "G8 {what}, n = {n}: the request is covered");
            assert!(
                pool.data_committed.is_multiple_of(COMMIT_PAGE)
                    && pool.ticks_committed.is_multiple_of(COMMIT_PAGE),
                "G8 {what}, n = {n}: frontier fields are page multiples"
            );
            assert_eq!(pool.committed_bytes(), ladder.union_bytes(), "G8 {what}, n = {n}: model");
            let os_len = host_vm(pool).os_len();
            assert!(
                ladder.sigma + pool.committed_rows() * ladder.stride <= os_len,
                "G8 {what}, n = {n}: row_ptr(rows - 1) + stride stays inside the reservation"
            );
            assert!(
                ladder.data_len + ladder.tick_len + pool.ticks_committed <= os_len,
                "G8 {what}, n = {n}: the changed-tick commit stays inside the reservation"
            );
            #[cfg(any(windows, target_os = "linux"))]
            assert_os_truth(pool, &format!("G8 {what}, n = {n}"));
        }

        /// G8 — for every sampled `(stride, σ)`: one pool climbed one rung at
        /// a time from the first row to its 10 000-row ceiling, and one fresh
        /// pool driven by an ascending pseudo-random request sequence
        /// (requests below the frontier included). After every request the
        /// frontier fields equal the oracle's, `committed_rows >= n`, both
        /// frontiers are page multiples, the model equals the oracle's union
        /// and the OS, and nothing commits past `os_len`; at `n = 1` the model
        /// equals the closed form of "The floor, derived".
        #[test]
        fn property_every_grow_matches_the_d2_ladder_and_the_os() {
            let mut checked = 0;
            for (id, stride) in FLOOR40_BAND.map(|id| (id, 40)).chain(G8_CASES) {
                register_stride(id, stride);
                let sigma = pool_base_stagger(id);
                let what = format!("stride {stride}, σ {sigma} (id {id})");

                let mut pool = ComponentPool::new(id, G8_RESERVE);
                let mut ladder = Ladder::new(sigma, stride, G8_RESERVE);
                grow_and_check(&mut pool, &mut ladder, 1, &what);
                assert_eq!(
                    (pool.committed_rows(), pool.committed_bytes()),
                    closed_form_floor(sigma, stride, true),
                    "G8 {what}: the floor's closed form at n = 1"
                );
                while pool.committed_rows() < G8_RESERVE {
                    let n = pool.committed_rows() + 1;
                    grow_and_check(&mut pool, &mut ladder, n, &what);
                }

                let mut pool = ComponentPool::new(id, G8_RESERVE);
                let mut ladder = Ladder::new(sigma, stride, G8_RESERVE);
                let mut state = ((id as u64) << 16) | stride as u64;
                let mut n = 0;
                while n < G8_RESERVE {
                    // Knuth's MMIX LCG; the high bits pick the next increment.
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    n = (n + 1 + (state >> 33) as usize % 1_500).min(G8_RESERVE);
                    grow_and_check(&mut pool, &mut ladder, n, &what);
                }
                checked += 2;
            }
            assert_eq!(checked, 2 * (64 + G8_CASES.len()), "G8: every sampled case ran");
        }
    }
}
