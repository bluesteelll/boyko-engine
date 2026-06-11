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
use crate::ecs::memory::arena::Arena;
use crate::ecs::memory::vm::VmReservation;

// Phase X.I D1: the tick sub-regions are sized at `reserve_rows * 4` bytes —
// pinned here so the layout math in `constants::pool_byte_layout` (which uses
// the literal 4) can never drift from the real slot type.
const _: () = assert!(std::mem::size_of::<UnsafeCell<Tick>>() == 4);
const _: () = assert!(std::mem::align_of::<UnsafeCell<Tick>>() == 4);

/// Pool of components of a specific type, stored as a dense byte buffer.
///
/// Components live contiguously in `buffer`: row `i` starts at
/// `buffer + i * component_layout.size()`. The rows `[0, self.len)` are fully
/// initialized; rows `[len, committed_rows)` are committed-but-uninitialized
/// and must never be read or dropped; rows `[committed_rows, reserve_rows)`
/// are reserved address space only (`PROT_NONE` on the syscall arms — a
/// stray touch faults loudly). The row pointer is recomputed on demand via
/// [`ComponentPool::row_ptr`] rather than cached per-row (Phase X.B).
///
/// # Phase X.I — one `VmReservation` per pool, in-place row growth
///
/// Each pool owns ONE virtual-address reservation laid out
/// `[data | added_ticks | changed_ticks]` with granule-aligned, fixed
/// sub-region offsets computed once at construction
/// (`constants::pool_byte_layout`). Growth ([`ComponentPool::grow_rows`])
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
    /// Data sub-region base (== `vm.base()`); WRITE-ONCE (invariant U6 twin).
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

    /// The pool's single reservation. Declared LAST: `Drop::drop`'s body
    /// (the `drop_fn` loop over rows `[0, len)`) runs before field drops,
    /// so the reservation is released strictly after its last use
    /// (release-after-use; M-001 per-arm deallocator carried by
    /// `VmReservation`, V-DROP releases partially committed reservations
    /// in full).
    vm: VmReservation,
}

impl ComponentPool {
    /// Creates a new component pool with an EXPLICIT row ceiling.
    ///
    /// # Phase X.I D2 mapping (★R1-9 — binding)
    ///
    /// This legacy constructor produces `reserve_rows = num_chunks *
    /// components_per_chunk` EXACTLY — it deliberately BYPASSES the
    /// `POOL_MIN_ROWS`/`POOL_MAX_ROWS` clamp. The entire pin-test ledger
    /// (drop_fn `1 × cap` pools, the in-file `4 × 64` proptests, the dense
    /// bench, the X.B identity tests) depends on exact small ceilings;
    /// routing this constructor through the clamp would be a ledger-wide
    /// breakage. The first two parameter names are historical (the chunk
    /// machinery was deleted by Phase X.I D7); a rename is filed for X.J.
    ///
    /// Construction performs ONE address-space reservation (no commit
    /// charge, zero resident bytes) and computes the three write-once base
    /// pointers; the first `add`/`reserve_capacity` takes the cold
    /// [`grow_rows`](Self::grow_rows) path (Phase X.I D3).
    ///
    /// # Panics
    ///
    /// * `reserve_rows == 0` (★R1-5) — the ceiling must be non-zero.
    /// * `component_layout.align() > 4096` — every arm's reservation base
    ///   is only guaranteed 4096-aligned.
    /// * `reserve_rows * stride` (or `num_chunks * components_per_chunk`)
    ///   overflowing `usize`.
    /// * OS reservation failure (unrecoverable misconfiguration).
    pub fn new(
        _arena: &Arena,
        component_id: usize,
        num_chunks: usize,
        components_per_chunk: usize,
    ) -> Self {
        debug_assert!(component_id < 512, "Component ID exceeds maximum allowed");

        // SAFETY: component_id was checked above; caller must have registered
        // the component before constructing a pool (invariant of ComponentPool::new).
        let registry_layout =
            unsafe { component_registry::get_layout_unchecked(component_id) };
        let component_layout = registry_layout.layout();
        let drop_fn = registry_layout.drop_fn;
        let component_type_id = registry_layout.type_id;

        debug_assert!(
            component_layout.size() > 0,
            "ComponentPool does not support zero-sized components (component_id = {}); \
             ZST registration is a Phase 2 enhancement",
            component_id
        );

        // D2 mapping (★R1-9): reserve_rows = n * m EXACTLY, clamp bypassed.
        let reserve_rows = num_chunks
            .checked_mul(components_per_chunk)
            .expect("ComponentPool::new: num_chunks * components_per_chunk overflows usize");
        // ★R1-5: loud pool-level assert BEFORE the vm reservation, so a zero
        // ceiling names this constructor instead of panicking inside
        // `VmReservation::reserve(0)` with a vm-internals message.
        assert!(
            reserve_rows > 0,
            "ComponentPool::new: reserve_rows == 0 \
             (num_chunks * components_per_chunk must be non-zero)"
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
            "ComponentPool::new: reserve_rows = {reserve_rows} \
             (num_chunks * components_per_chunk) exceeds the \
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
        let buffer = vm.base();

        // Phase X.A SIMD-A1 invariant (plan §6.4): the data base MUST be
        // SIMD_BUFFER_ALIGN-aligned so callers (`buffer_ptr`,
        // `Query::for_each_chunk` inner loops) can rely on it without
        // re-checking. Holds trivially post-X.I: every arm's reservation
        // base is >= 4096-aligned (see the D10 note above).
        debug_assert!(
            (buffer.as_ptr() as usize).is_multiple_of(SIMD_BUFFER_ALIGN),
            "SIMD-A1: ComponentPool buffer ptr {:p} is not SIMD_BUFFER_ALIGN={}-aligned",
            buffer.as_ptr(),
            SIMD_BUFFER_ALIGN
        );

        // SAFETY (S-TICKBASE): both offsets are in-bounds of the single
        // reservation (`added_off < changed_off < os_len` by the D1 layout
        // math, all checked; `os_len <= isize::MAX` asserted by `reserve`),
        // so each `add` stays inside the one allocated object — ★R1-8: the
        // data region and BOTH tick regions are ONE allocated object, the
        // pool's own reservation. Alignment: granule(64 KiB)-aligned
        // offsets from a >= 4096-aligned base yield alignment >= 4096 >= 4
        // = align_of::<UnsafeCell<Tick>> (const-asserted at the top of this
        // file). The bases are derived once and never reassigned
        // (write-once); the reservation ADDRESS is stable for the pool's
        // lifetime, so they remain valid after `vm` moves into the struct
        // below.
        let (added_base, changed_base) = unsafe {
            (
                buffer.add(layout.added_off).cast::<UnsafeCell<Tick>>(),
                buffer.add(layout.changed_off).cast::<UnsafeCell<Tick>>(),
            )
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
            vm,
        }
    }

    /// Creates a new pool with the Phase X.I D2 byte-targeted, row-clamped
    /// default ceiling:
    /// `reserve_rows = clamp(POOL_TARGET_DATA_BYTES / stride,
    /// POOL_MIN_ROWS, POOL_MAX_ROWS)`.
    pub fn with_default_sizes(_arena: &Arena, component_id: usize) -> Self {
        let component_size = component_registry::get_component_size(component_id)
            .expect("Component not registered");
        Self::new(_arena, component_id, 1, pool_reserve_rows(component_size))
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
        self.vm.commit(self.data_committed, new_d); // panics only on genuine OS OOM

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
            self.vm.commit(
                layout.added_off + self.ticks_committed,
                layout.added_off + t_new,
            );
            self.vm.commit(
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

    /// Byte pointer for row `idx`, computed from the stable reservation base.
    ///
    /// # Safety
    /// * `idx < self.committed_rows` (the slot lies inside the committed
    ///   prefix of the data sub-region); reads of LIVE data additionally
    ///   require `idx < self.len`.
    /// * Valid for `self.component_layout.size()` bytes.
    #[inline]
    unsafe fn row_ptr(&self, idx: usize) -> *mut u8 {
        debug_assert!(idx < self.committed_rows, "row_ptr: idx out of committed bounds");
        // SAFETY: idx < committed_rows <= reserve_rows ⇒ idx*stride + stride
        //   <= reserve_rows*stride <= data_len, so the element span lies
        //   inside the data sub-region of the pool's OWN reservation, within
        //   committed (read/write) pages. Provenance derives from
        //   `self.buffer` via one `add` — and the data region plus BOTH tick
        //   regions are ONE allocated object (★R1-8: a single
        //   `VmReservation` per pool). The base is write-once in `new` and
        //   never moves: Phase X.I growth only commits fresh pages at the
        //   frontier of the SAME reservation; previously returned pointers
        //   are never remapped or relocated.
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
            // - We hold &mut self → exclusive access; the two slots are
            //   non-overlapping (index != last_index, stride is
            //   component_layout.size() which is > 0 — ZSTs rejected at pool
            //   construction).
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
    /// is measured against (Phase X.I D6; the X.F precedent:
    /// `Arena::capacity()` = reserve). Committed capacity below the ceiling
    /// grows on demand and is reported by [`Self::committed_rows`].
    #[inline]
    pub fn capacity(&self) -> usize {
        self.reserve_rows
    }

    /// Phase X.I D6: rows currently committed read/write — the growth
    /// frontier (diagnostics/tests; the mirror of `Arena::committed()`).
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
    /// [`SIMD_BUFFER_ALIGN`](crate::ecs::constants::SIMD_BUFFER_ALIGN)
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
            //   * Non-overlapping: `idx != last_index`; each slot is
            //     `component_layout.size()` bytes at distinct stride
            //     multiples of the same data sub-region —
            //     `copy_nonoverlapping` requires only non-overlap, which
            //     distinct row indices guarantee.
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

    // ── Phase 12.5 Opt-A2 — batch reserve / write accessors (C-N1) ──────────
    //
    // §5.6 of the spawn-optimisations plan. The batch path reserves
    // capacity, writes payload bytes directly into pre-validated arena
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
            for row in 0..self.len {
                unsafe { drop_fn(self.row_ptr(row)) }
            }
        }
        // The reservation itself is released by the `vm` field's Drop
        // (declared LAST in the struct): this body runs BEFORE field drops,
        // so release happens strictly after the last use. V-DROP releases
        // partially committed reservations in full; M-001 per-arm
        // deallocator lives inside `VmReservation`.
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::ComponentPool;
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::identifiers::primitives::ComponentId;
    use crate::ecs::memory::arena::Arena;

    // ID allocation (no collision with integration test files or other unit tests):
    //   component_registry unit tests: 450..466, 498, 499
    //   drop_fn integration:           200..207
    //   drop_safety integration:       480..481
    //   typed-read tests below:        220..223
    //   Phase X.B dense-equivalence tests below: 224..226
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

    fn make_position_pool(arena: &Arena, cap: usize) -> ComponentPool {
        register_all();
        ComponentPool::new(arena, POS_ID.0, 1, cap)
    }

    // ---- tests (audit C-004 typed read wrappers) -----------------------------------

    /// `get_typed` must return the exact field values that were inserted via `add_typed`.
    #[test]
    fn get_typed_returns_inserted_value() {
        register_all();
        let arena = Arena::new();
        let mut pool = make_position_pool(&arena, 4);

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
        let arena = Arena::new();
        let mut pool = make_position_pool(&arena, 4);

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
        let arena = Arena::new();
        let pool = make_position_pool(&arena, 4);

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
        let arena = Arena::new();
        // Pool is registered for `Position` (POS_ID).
        let mut pool = ComponentPool::new(&arena, POS_ID.0, 1, 4);

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
    /// Phase X.I note: the original X.A scenario (an arena cursor left
    /// misaligned by a preceding pool, corrected by the constructor's
    /// alignment lift) is dead — pools no longer allocate from the arena.
    /// Post-X.I the assertion is trivially true: every arm's reservation
    /// base is >= 4096-aligned (VirtualAlloc 64 KiB / mmap 4 KiB / fallback
    /// `Layout` align 4096), far above `SIMD_BUFFER_ALIGN = 32`. The test
    /// is kept as a TRIPWIRE for the SIMD-A1 contract: if a future storage
    /// change ever hands out a buffer base below 32-byte alignment, this
    /// fails loudly. The `_prefix` pool below is the historical
    /// non-tautology fixture, retained unchanged (test logic frozen).
    #[test]
    fn buffer_ptr_is_simd_aligned() {
        use crate::ecs::constants::SIMD_BUFFER_ALIGN;

        register_all();
        let arena = Arena::new();

        // Historical X.A fixture: pre-X.I this pool left the shared arena
        // cursor at a 16-mod-32 offset so the next pool's buffer would be
        // misaligned without the constructor's alignment lift. Post-X.I
        // every pool owns its reservation, so this no longer influences the
        // F32Wrap pool's base — retained to keep the test logic frozen.
        let _prefix = ComponentPool::new(&arena, POS_ID.0, 1, 4);

        // Constructor arguments mirror the rest of the test module:
        // `(arena, component_id, num_chunks, components_per_chunk)`. Using
        // the real `ComponentPool::new` keeps the tripwire wired to the
        // production base-pointer derivation.
        let pool = ComponentPool::new(&arena, F32_WRAP_ID.0, 1, 4);

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

    fn make_u64_pool(arena: &Arena, num_chunks: usize, per_chunk: usize) -> ComponentPool {
        component_registry::register_layout::<U64Pair>(U64_ID.0);
        ComponentPool::new(arena, U64_ID.0, num_chunks, per_chunk)
    }

    /// Phase X.B core proof: after a mixed `add` + `swap_remove(mid)` + `add`
    /// sequence, every live row `i` satisfies
    /// `get_raw(i) == buffer_ptr() + i * stride` AND the round-tripped value
    /// matches a dense `Vec` oracle maintained with the same swap_remove rule.
    /// This is the exact identity the deleted `Unit.ptr()` cache used to hold.
    #[test]
    fn dense_equivalence() {
        let arena = Arena::new();
        // Multiple chunks so a swap can move a value across a chunk boundary
        // (4 chunks × 4 = 16 slots); proves row_ptr spans the whole buffer.
        let mut pool = make_u64_pool(&arena, 4, 4);
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
        let arena = Arena::new();
        let mut pool = make_u64_pool(&arena, 1, 16);

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
        let arena = Arena::new();
        // Capacity 16, only 6 live → 10 uninit slots that must NOT be dropped.
        let mut pool = ComponentPool::new(&arena, DROPPER_ID.0, 1, 16);

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
        use crate::ecs::memory::arena::Arena;
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

                let arena = Arena::new();
                // 4 chunks × 64 = 256 capacity — comfortably above the 200-op cap.
                let mut pool = ComponentPool::new(&arena, U64_ID.0, 4, 64);
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
        let arena = Arena::new();
        // 4 chunks × 4 = 16 slots: a mid-row swap can move the last row across
        // a chunk boundary, exercising row_ptr over the whole buffer.
        let mut pool = make_u64_pool(&arena, 4, 4);
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
        use crate::ecs::memory::arena::Arena;
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

                let arena = Arena::new();
                // 4 chunks × 64 = 256 capacity > the 200-op cap.
                let mut pool = ComponentPool::new(&arena, U64_ID.0, 4, 64);
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
        let arena = Arena::new();
        // Capacity 16, 8 live → 8 uninit slots that must NOT be dropped.
        let mut pool = ComponentPool::new(&arena, DROPPER_ID.0, 1, 16);

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

    /// Phase X.I U-P1 (★R1-5) — a zero row ceiling (`num_chunks *
    /// components_per_chunk == 0`, reachable via the D2 constructor
    /// mapping) must hit the loud pool-level construction assert that
    /// names the constructor — NOT a `VmReservation::reserve(0)` panic
    /// with a vm-internals message.
    #[test]
    #[should_panic(expected = "ComponentPool::new: reserve_rows == 0")]
    fn zero_ceiling_construction_panics_loudly() {
        register_all();
        let arena = Arena::new();
        let _ = ComponentPool::new(&arena, POS_ID.0, 0, 16);
    }
}
