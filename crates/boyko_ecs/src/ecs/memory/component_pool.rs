use std::alloc::Layout;
use std::any::TypeId;
use std::cell::UnsafeCell;
use std::ptr::NonNull;

use crate::ecs::constants::{
    SIMD_BUFFER_ALIGN, pool_align_up_granule, pool_byte_layout, pool_commit_step,
    pool_reserve_rows,
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
    /// `stride > 0`: `== vm.base()`. Phase 22 D1 (`stride == 0`, tag pools):
    /// a dangling, provenance-free pointer at address
    /// `SIMD_BUFFER_ALIGN.max(align)` — non-null, SIMD-A1-aligned, valid
    /// ONLY for zero-size access (the data sub-region is vacuous).
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

    /// Committed bytes of the data sub-region; granule-aligned, monotonic
    /// (cold-path bookkeeping for `grow_rows`).
    data_committed: usize,

    /// Committed bytes of EACH tick sub-region; granule-aligned, monotonic.
    ticks_committed: usize,

    /// `added` tick sub-region base (`vm.base() + data_len`); WRITE-ONCE.
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

    /// `changed` tick sub-region base (`vm.base() + data_len + tick_len`);
    /// WRITE-ONCE. Same shape and discipline as [`Self::added_base`].
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

        // D1 layout: [data | added_ticks | changed_ticks], all sub-regions
        // granule-aligned; checked arithmetic panics loudly on overflow.
        let stride = component_layout.size();
        let layout = pool_byte_layout(reserve_rows, stride);

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

        // Phase X.A SIMD-A1 invariant (plan §6.4): the data base MUST be
        // SIMD_BUFFER_ALIGN-aligned so callers (`buffer_ptr`,
        // `Query::for_each_chunk` inner loops) can rely on it without
        // re-checking. Asserted against `base` (the reservation base — its
        // >= 4096 alignment guarantee is what this actually verifies; holds
        // trivially post-X.I, see the D10 note above).
        debug_assert!(
            (base.as_ptr() as usize).is_multiple_of(SIMD_BUFFER_ALIGN),
            "SIMD-A1: ComponentPool reservation base {:p} is not SIMD_BUFFER_ALIGN={}-aligned",
            base.as_ptr(),
            SIMD_BUFFER_ALIGN
        );

        // SAFETY (S-TICKBASE): both offsets are in-bounds of the single
        // reservation (`added_off < changed_off < os_len` by the D1 layout
        // math, all checked; `os_len <= isize::MAX` asserted by `reserve`),
        // so each `add` stays inside the one allocated object — ★R1-8: the
        // data region and BOTH tick regions are ONE allocated object, the
        // pool's own reservation. Phase 22 ZST arm: for `stride == 0` the
        // data sub-region is empty (`added_off == 0`, `changed_off ==
        // tick_len`); `buffer` (set below) is a dangling aligned pointer
        // valid only for zero-size access per the Rust reference — both
        // tick bases derive from `base` (the single LIVE reservation, O1),
        // and tick-tick disjointness is unchanged. Alignment:
        // granule(64 KiB)-aligned offsets from a >= 4096-aligned base yield
        // alignment >= 4096 >= 4 = align_of::<UnsafeCell<Tick>>
        // (const-asserted at the top of this file). The bases are derived
        // once and never reassigned (write-once); the reservation ADDRESS
        // is stable for the pool's lifetime, so they remain valid after
        // `vm` moves into the struct below.
        let (added_base, changed_base) = unsafe {
            (
                base.add(layout.added_off).cast::<UnsafeCell<Tick>>(),
                base.add(layout.changed_off).cast::<UnsafeCell<Tick>>(),
            )
        };

        // O1: `buffer` per-arm, LAST — after every live-reservation-derived
        // pointer is already bound.
        let buffer = if stride > 0 {
            base
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

    /// Creates a new pool with the Phase X.I D2 byte-targeted, row-clamped
    /// default ceiling:
    /// `reserve_rows = clamp(POOL_TARGET_DATA_BYTES / stride,
    /// POOL_MIN_ROWS, POOL_MAX_ROWS)`.
    pub fn with_default_sizes(component_id: usize) -> Self {
        let component_size = component_registry::get_component_size(component_id)
            .expect("Component not registered");
        Self::new(component_id, pool_reserve_rows(component_size))
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
    /// (`constants::pool_commit_step`). O(1) in live rows — no bytes
    /// copied, no bytes written; the base pointers never move (in-place
    /// frontier commits on the pool's own reservation).
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
        // recomputed on this cold path instead of stored (D1).
        let layout = pool_byte_layout(self.reserve_rows, stride);

        // GROW1-XI proof 1: n <= reserve_rows => n*stride <=
        // reserve_rows*stride <= data_len, and data_len is a granule
        // multiple => align_up(n*stride, G) <= data_len. The mul cannot
        // overflow: reserve_rows*stride was overflow-checked at construction.
        let needed = pool_align_up_granule(n * stride);
        debug_assert!(
            needed <= layout.data_len,
            "GROW1-XI step 1: needed overruns the data sub-region"
        );
        // GROW1-XI corollary 0a: past both guards `n > committed_rows`, and
        // the clamped case is excluded by the ceiling check, so `needed >
        // data_committed` — the saturating_sub inside `pool_commit_step` is
        // a belt that never actually saturates.
        debug_assert!(
            needed > self.data_committed,
            "GROW1-XI corollary 0a: grow_rows reached the commit path with a satisfied request"
        );

        let step = pool_commit_step(self.data_committed, needed);
        // GROW1-XI proof 2: step >= needed - data_committed by the
        // request-dominant max, and the min(data_len) clamp cannot bite
        // below `needed` (needed <= data_len) => new_d >= needed, and
        // new_d > data_committed strictly (the vm.rs `new > old`
        // debug_assert is unreachable from this caller — GROW1-XI 0b).
        let new_d = (self.data_committed + step).min(layout.data_len);
        // IM-3: grow is Host-only in Phase 4 (`host_vm_mut` unreachable!s on a
        // Device pool, which is never minted). Panics only on genuine OS OOM.
        self.backing.host_vm_mut().commit(self.data_committed, new_d);

        // GROW1-XI proofs 3 + 4: new_d >= needed >= n*stride =>
        // floor(new_d/stride) >= n; the min(reserve_rows) is LOAD-BEARING —
        // granule padding can make floor(data_len/stride) > reserve_rows,
        // and the tick sub-regions are sized for reserve_rows only.
        let rows = (new_d / stride).min(self.reserve_rows);
        debug_assert!(
            rows >= n,
            "GROW1-XI step 3: post-grow committed_rows must cover the request"
        );

        // GROW1-XI proof 5: rows <= reserve_rows => align_up(rows*4, G) <=
        // align_up(reserve_rows*4, G) = tick_len.
        let t_new = pool_align_up_granule(rows * 4);
        debug_assert!(
            t_new <= layout.tick_len,
            "GROW1-XI step 5: tick commit overruns the tick sub-region"
        );
        if t_new > self.ticks_committed {
            // IM-3: Host-only grow (see the data commit above).
            let vm = self.backing.host_vm_mut();
            vm.commit(
                layout.added_off + self.ticks_committed,
                layout.added_off + t_new,
            );
            vm.commit(
                layout.changed_off + self.ticks_committed,
                layout.changed_off + t_new,
            );
            // ★Q6: the frontier field is written only AFTER the commits it
            // describes succeed (panic-coherent on a mid-grow OS OOM).
            self.ticks_committed = t_new;
        }
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
    ///   needed_t)` with `needed_t = align_up(n * 4, G)` — the same
    ///   request-dominant doubling, applied to the tick byte frontier.
    /// * **Z3 (in-bounds)**: `n <= reserve_rows ⇒ n*4 <= reserve_rows*4 ⇒
    ///   needed_t <= tick_len` (tick_len is a granule multiple), and
    ///   `t_new = (ticks_committed + step).min(tick_len)` never overruns
    ///   the sub-region.
    /// * **Z4 (strict growth)**: past the guards `n > committed_rows`. The
    ///   reserve-clamp case is excluded (`committed_rows == reserve_rows`
    ///   would contradict `n <= reserve_rows < n`), so `committed_rows =
    ///   ticks_committed / 4` exactly (granule multiples are divisible by
    ///   4), hence `n*4 > ticks_committed`, hence `needed_t >= n*4 >
    ///   ticks_committed`, and with Z3 `t_new >= needed_t >
    ///   ticks_committed` — BOTH tick commits satisfy the vm `new > old`
    ///   assert.
    /// * **Z5 (sufficiency)**: `committed_rows' = (t_new / 4)
    ///   .min(reserve_rows) >= n`, since `t_new >= needed_t >= n*4` and
    ///   `n <= reserve_rows` (debug-asserted — the GROW1-XI step-3
    ///   analogue). Callers never retry.
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
        // instead of stored (D1), mirroring the stride > 0 body.
        let layout = pool_byte_layout(self.reserve_rows, 0);
        debug_assert_eq!(layout.data_len, 0, "Z1: vacuous data region");
        debug_assert_eq!(layout.added_off, 0, "Z1: added ticks start at the base");

        // Z2: the tick byte frontier drives the policy. The mul cannot
        // overflow: `reserve_rows * 4` was overflow-checked by
        // `pool_byte_layout` at construction and `n <= reserve_rows`.
        let needed_t = pool_align_up_granule(n * 4);
        debug_assert!(
            needed_t <= layout.tick_len,
            "Z3: tick request overruns the tick sub-region"
        );
        debug_assert!(
            needed_t > self.ticks_committed,
            "Z4: grow_rows_zst reached the commit path with a satisfied request"
        );

        let step = pool_commit_step(self.ticks_committed, needed_t);
        let t_new = (self.ticks_committed + step).min(layout.tick_len);
        debug_assert!(
            t_new > self.ticks_committed,
            "Z4: the tick frontier must grow strictly (vm `new > old` precondition)"
        );

        // Z3 + Z4: both ranges are granule-aligned (granule-multiple offsets
        // plus granule-multiple frontiers), strictly growing, and in-bounds
        // of the reservation (`changed_off + tick_len == os_len`). Panics
        // only on genuine OS OOM (same contract as the stride > 0 path).
        // IM-3: Host-only grow (a ZST device pool is never minted in Phase 4).
        let vm = self.backing.host_vm_mut();
        vm.commit(
            layout.added_off + self.ticks_committed,
            layout.added_off + t_new,
        );
        vm.commit(
            layout.changed_off + self.ticks_committed,
            layout.changed_off + t_new,
        );

        // Z5: the min(reserve_rows) is LOAD-BEARING — granule padding can
        // make tick_len / 4 exceed reserve_rows.
        let rows = (t_new / 4).min(self.reserve_rows);
        debug_assert!(
            rows >= n,
            "Z5: post-grow committed_rows must cover the request"
        );

        // Z6: frontier fields written only AFTER both commits succeeded.
        self.ticks_committed = t_new;
        self.committed_rows = rows;
        true
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
    /// * `idx >= self.len` (the slot is uninit and not yet committed); after the
    ///   matching `commit_units(idx, 1)` the slot becomes addressable.
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
    // Geometry note shared by every test below: the fixtures use a 64-byte
    // stride so ONE commit granule (`COMMIT_GRANULE` = 64 KiB of DATA
    // bytes) covers exactly 1024 rows — slab boundaries therefore sit at
    // rows 1024 / 2048 / 4096 under the D4 doubling policy
    // (64 KiB -> 128 KiB -> 256 KiB), and a 4096-row pool spans
    // 4 granules = 3 data-commit events when filled by an `add` loop.
    // ====================================================================

    /// Phase X.I W4 fixture: a 64-byte POD component (8 x u64). `tag` makes
    /// every row distinguishable; `pad` brings the stride to exactly one
    /// 64th of a commit granule (see geometry note above).
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

    /// U-P2 — the address-stability witness (plan §Test matrix).
    ///
    /// Pins THE central Phase X.I soundness claim (Soundness item 1): the
    /// three write-once base pointers (`buffer_ptr`, `added_ticks_ptr`,
    /// `changed_ticks_ptr`) and any previously returned row pointer stay
    /// bit-identical across >= 3 data-commit growth events, and pre-growth
    /// VALUES (component bytes + stamped ticks) remain readable through the
    /// recorded pointers. A 4096-row pool of 64-B rows spans 4 granules
    /// (256 KiB) -> the add loop drives 3 commits: 64 KiB (row 0),
    /// 128 KiB (row 1024), 256 KiB (row 2048).
    ///
    /// Miri: ignored (4096-add loop; the M-XI suite covers the identical
    /// bookkeeping with a small-granule-count geometry under Tree Borrows).
    #[test]
    #[cfg_attr(miri, ignore)]
    fn address_stability_across_three_slab_growths() {
        use crate::ecs::core::change_detection::Tick;

        let mut pool = make_stride64_pool(4096);
        assert_eq!(pool.component_layout().size(), 64, "fixture stride must be 64 B");
        assert_eq!(pool.capacity(), 4096, "D2 mapping: reserve_rows = 1 * 4096");
        assert_eq!(pool.committed_rows(), 0, "D3: zero initial commit");

        // First add: triggers the first 64 KiB commit (1024 rows).
        let v0 = Stride64::new(0xA5A5_0000);
        pool.add_typed(v0).expect("row 0 fits under the ceiling");
        assert_eq!(pool.committed_rows(), 1024, "first commit = one granule = 1024 rows");

        // Record every base pointer + the row-0 pointer BEFORE further growth,
        // and stamp a distinctive pre-growth tick on row 0.
        let base_before = pool.buffer_ptr();
        let row0_before = pool.get_raw(0).expect("row 0 live");
        let added_before = pool.added_ticks_ptr();
        let changed_before = pool.changed_ticks_ptr();
        pool.fill_ticks(0, 1, Tick::new(7));

        // Grow across the remaining slab boundaries via the warm add path:
        // rows 1..4096 cross commits at len = 1024 and len = 2048.
        for i in 1..4096u64 {
            pool.add_typed(Stride64::new(i)).expect("under the 4096-row ceiling");
        }
        assert_eq!(pool.count(), 4096, "all 4096 rows live");
        assert_eq!(pool.committed_rows(), 4096, "frontier reached the full reserve");

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

        // (3) Boundary rows on both sides of each slab edge read back correctly.
        for &row in &[1023usize, 1024, 2047, 2048, 4095] {
            assert_eq!(
                *pool.get_typed::<Stride64>(row).expect("boundary row typed read"),
                Stride64::new(row as u64),
                "row {row} (slab-boundary +/- 1) must hold its written value"
            );
        }
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
        // the ceiling (GROW1-XI step 4): one granule covers 1024 rows of data
        // but the pool may only ever expose 4.
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
    /// Sequence: first add commits the granule -> every never-written slot
    /// in `[1, 1024)` reads `Tick::ZERO`; fill to the 1024-row boundary and
    /// stamp `[0, 1024)`; the 1025th add grows across the boundary ->
    /// pre-grow stamps at rows 1023/1022 survive, the freshly-grown row 1024
    /// reads ZERO until stamped (write-before-read), then reads its stamp,
    /// and the new never-written tail `[1025, 2048)` reads ZERO.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn tick_lockstep_and_jxi_zero_at_slab_boundary() {
        use crate::ecs::core::change_detection::Tick;

        let mut pool = make_stride64_pool(4096);

        // Row 0 commits the first granule (1024 rows of data; the tick
        // granule covers 16384 slots, so ticks are committed well past the
        // data frontier — lockstep by ROWS, saturating by bytes).
        pool.add_typed(Stride64::new(0)).expect("row 0");
        assert_eq!(pool.committed_rows(), 1024);

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
        for i in 1..1024u64 {
            pool.add_typed(Stride64::new(i)).expect("rows 1..1024");
        }
        assert_eq!(pool.count(), 1024);
        assert_eq!(pool.committed_rows(), 1024, "len reached the first-slab frontier");
        pool.fill_ticks(0, 1024, Tick::new(5));

        // Cross the boundary: row 1024 triggers grow_rows(1025) -> 2048 rows.
        pool.add_typed(Stride64::new(1024)).expect("row 1024 grows the pool");
        assert_eq!(pool.committed_rows(), 2048, "doubling: 64 KiB -> 128 KiB = 2048 rows");

        // Pre-grow stamps at [boundary-2, boundary-1] survived the grow.
        // SAFETY: 1022/1023 < count(); &pool shared read, no writer.
        let (t1022, t1023) = unsafe { (pool.read_added_tick(1022), pool.read_added_tick(1023)) };
        assert_eq!(t1022, Tick::new(5), "stamp at boundary-2 survives the grow");
        assert_eq!(t1023, Tick::new(5), "stamp at boundary-1 survives the grow");

        // The freshly-grown-then-added row 1024: never stamped -> ZERO (J-XI
        // on a demand-committed page), then write-before-read round-trips.
        // SAFETY: 1024 < count() == 1025; &pool exclusive in this test.
        let t1024_unstamped = unsafe { pool.read_added_tick(1024) };
        assert_eq!(
            t1024_unstamped,
            Tick::ZERO,
            "a freshly committed, never-stamped tick slot reads ZERO"
        );
        // SAFETY: 1024 < count(); single-threaded test holds exclusive access.
        unsafe {
            pool.write_added_tick(1024, Tick::new(9));
            pool.write_changed_tick(1024, Tick::new(9));
        }
        // SAFETY: as above.
        let (a1024, c1024) = unsafe { (pool.read_added_tick(1024), pool.read_changed_tick(1024)) };
        assert_eq!(a1024, Tick::new(9), "write-before-read: stamped added tick reads back");
        assert_eq!(c1024, Tick::new(9), "write-before-read: stamped changed tick reads back");

        // J-XI (2): the newly committed never-written tail also reads ZERO.
        for i in pool.count()..pool.committed_rows() {
            // SAFETY: i < committed_rows() — documented validity window.
            let (a, c) = unsafe { (*(*added.add(i)).get(), *(*changed.add(i)).get()) };
            assert_eq!(a, Tick::ZERO, "post-grow never-written added slot {i} reads ZERO");
            assert_eq!(c, Tick::ZERO, "post-grow never-written changed slot {i} reads ZERO");
        }
    }

    /// Phase X.I W4 fixture: a 64-byte drop-counting component so the
    /// drop-accounting boundary sits at 1024 rows (one granule).
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

    /// U-P5 — drop-count-exact across a growth boundary. 1500 64-B rows
    /// cross the 1024-row slab boundary (one mid-sequence growth event);
    /// 5 pops + 3 swap_removes drop exactly 8; pool Drop drops exactly the
    /// 1492 survivors. Total == 1500 — every value dropped EXACTLY once,
    /// no uninit `[len, committed_rows)` slot dropped, and the growth event
    /// itself dropped nothing (O(1), zero bytes copied, zero drops).
    #[test]
    #[cfg_attr(miri, ignore)]
    fn drop_count_exact_across_growth_boundary() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        component_registry::register_layout::<DropPad64>(DROP_PAD_ID.0);
        let mut pool = ComponentPool::new(DROP_PAD_ID.0, 4096);

        let counter = Arc::new(AtomicUsize::new(0));
        const M: usize = 1500; // crosses the 1024-row boundary
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

        // One sub-boundary row, one row that crossed the boundary.
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
    #[test]
    fn grow_rows_idempotent_below_frontier() {
        let mut pool = make_stride64_pool(4096);

        // First real grow: request 100 -> one granule -> 1024 rows.
        assert!(pool.grow_rows(100), "grow within the ceiling succeeds");
        assert_eq!(pool.committed_rows(), 1024, "one granule = 1024 rows of 64 B");

        // Idempotent no-op arm, exercised repeatedly at several n values
        // including the n == committed_rows edge and n == 0.
        for _round in 0..2 {
            for &n in &[0usize, 1, 100, 1023, 1024] {
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
        assert!(pool.grow_rows(1025), "grow past the frontier succeeds");
        assert_eq!(pool.committed_rows(), 2048, "doubling: 64 KiB -> 128 KiB");

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

    /// Phase 22 — THE GROW1-ZST growth gate: two successive
    /// `grow_rows_zst` invocations that EACH reach `vm.commit`, with strict
    /// tick-frontier growth (Z4), request coverage (Z5), `data_committed ==
    /// 0` throughout (Z1), and stable base pointers across both commits.
    ///
    /// Geometry: ticks are 4 B/row, so one granule (64 KiB) covers 16,384
    /// rows. `grow(1)` commits the first granule; `grow(20_000)` drives the
    /// request past that frontier (20,000 × 4 B > 64 KiB) — the second call
    /// MUST commit again (doubling: 64 KiB → 128 KiB).
    #[test]
    fn zst_pool_growth_two_successive_commits() {
        use crate::ecs::constants::COMMIT_GRANULE as G;

        let mut pool = make_zst_pool(100_000);
        assert_eq!(pool.committed_rows(), 0, "D3: zero initial commit");
        assert_eq!(pool.ticks_committed, 0);
        assert_eq!(pool.data_committed, 0, "Z1 before any growth");

        let buffer_before = pool.buffer_ptr();
        let added_before = pool.added_ticks_ptr();
        let changed_before = pool.changed_ticks_ptr();

        // First grow: 0 → one granule of ticks (first vm.commit pair).
        assert!(pool.grow_rows(1), "grow within the ceiling succeeds");
        assert_eq!(pool.ticks_committed, G, "first commit = one tick granule");
        assert_eq!(pool.committed_rows(), G / 4, "G/4 = 16,384 rows of 4 B ticks");
        assert_eq!(pool.data_committed, 0, "Z1 after the first commit");

        // Idempotent no-op arm below the frontier: zero state change.
        let frontier = (pool.ticks_committed, pool.committed_rows());
        assert!(pool.grow_rows(100), "no-op grow below the frontier");
        assert_eq!(
            (pool.ticks_committed, pool.committed_rows()),
            frontier,
            "idempotent arm must not move the frontier"
        );

        // Second grow: n = 20,000 > 16,384 — past the first commit step, so
        // grow_rows_zst reaches vm.commit AGAIN (doubling to two granules).
        assert!(pool.grow_rows(20_000), "second grow within the ceiling");
        assert_eq!(pool.ticks_committed, 2 * G, "STRICT frontier growth: G → 2·G (Z4)");
        assert_eq!(pool.committed_rows(), 2 * G / 4, "2G/4 = 32,768 rows");
        assert!(pool.committed_rows() >= 20_000, "Z5: request covered");
        assert_eq!(pool.data_committed, 0, "Z1 after the second commit");

        // Base pointers never move across ZST growth (write-once contract).
        assert_eq!(pool.buffer_ptr(), buffer_before, "dangling data base stable");
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
}
