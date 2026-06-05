use std::alloc::Layout;
use std::any::TypeId;
use std::cell::UnsafeCell;
use std::ptr::NonNull;

use crate::ecs::constants::{
    DEFAULT_CHUNKS_PER_POOL, LARGE_COMPONENTS_PER_CHUNK, MEDIUM_COMPONENTS_PER_CHUNK,
    MEDIUM_COMPONENT_THRESHOLD, SIMD_BUFFER_ALIGN, SMALL_COMPONENTS_PER_CHUNK,
    SMALL_COMPONENT_THRESHOLD, TINY_COMPONENTS_PER_CHUNK, TINY_COMPONENT_THRESHOLD,
};
use crate::ecs::core::change_detection::Tick;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry::{self, DropFn};
use crate::ecs::memory::arena::Arena;
use crate::ecs::memory::chunk::Chunk;

/// Pool of components of a specific type, stored as a dense byte buffer.
///
/// Components live contiguously in `buffer`: row `i` starts at
/// `buffer + i * component_layout.size()`. The rows `[0, self.len)` are fully
/// initialized; slots beyond that are uninitialized arena memory and must never
/// be read or dropped. The row pointer is recomputed on demand via
/// [`ComponentPool::row_ptr`] rather than cached per-row, so the pool holds no
/// parallel pointer array (the cached pointer was always exactly the computed
/// address — see Phase X.B).
pub struct ComponentPool {
    /// Raw provenance pointer to the arena for memory allocation.
    /// Stored as `*const Arena` (raw provenance) to avoid Miri retag UB when
    /// the owning `EcsMaster` constructs multiple pools from the same arena:
    /// using `NonNull::from(&*boxed_arena)` across multiple reborrow sites
    /// creates overlapping `&mut`-derived tags. The raw pointer sidesteps the
    /// Stacked Borrows model entirely (Miri retag fix, Phase 3a).
    /// Reserved for future deallocation / defragmentation support (Phase 3).
    #[allow(dead_code)]
    arena: *const Arena,

    /// Buffer for storing components, allocated directly from the arena.
    buffer: NonNull<u8>,

    /// Buffer capacity in bytes.
    /// Reserved for bounds-checking in future growth/defragmentation code (Phase 3).
    #[allow(dead_code)]
    buffer_capacity_bytes: usize,

    /// Maximum number of components.
    max_components: usize,

    /// Live row count; rows `[0, len)` are initialized and densely packed.
    len: usize,

    /// Chunk metadata.
    chunks: Vec<Chunk>,

    /// Components per chunk.
    components_per_chunk: usize,

    /// Component ID — used to look up layout information.
    component_id: usize,

    /// Component layout (cached from registry for performance).
    component_layout: Layout,

    /// Cached drop_fn for the component type (`None` when `!needs_drop`).
    /// Read on every swap_remove / pop / set_component / Drop.
    drop_fn: Option<DropFn>,

    /// Cached TypeId for debug-only typed-API validation.
    component_type_id: TypeId,

    /// Phase 10 STORE3: per-row "added at" ticks, parallel to the data rows.
    ///
    /// Logical row `i` has its added-at tick at `added_ticks[i]`. The buffer is
    /// sized to `max_components` at pool construction and never reallocates;
    /// slots beyond `self.len` stay at [`Tick::ZERO`] until a future write.
    ///
    /// `UnsafeCell<Tick>` provides interior mutability through a shared
    /// `&self` (used by Wave C `Added<C>::filter_fetch` reads through the
    /// `Fetch<'w>` pointer) while still permitting the Phase 9 scheduler
    /// to declare exclusive write access on a per-`(archetype, component)`
    /// basis (SCH3). Adjacent-row writes from sibling `par_iter` chunks
    /// target distinct memory locations — sound per Rust's abstract
    /// machine even though they share a cache line (Round 2 C3, plan §11.5).
    pub(crate) added_ticks: Box<[UnsafeCell<Tick>]>,

    /// Phase 10 STORE3: per-row "last changed at" ticks, parallel to the data rows.
    ///
    /// Same shape and discipline as [`added_ticks`]. Updated by
    /// `Mut<T>::deref_mut` (Wave C) and by `EcsMaster::set_component_raw`
    /// (Wave D follow-up; out of Wave B scope).
    pub(crate) changed_ticks: Box<[UnsafeCell<Tick>]>,
}

impl ComponentPool {
    /// Creates a new component pool with direct memory allocation.
    pub fn new(
        arena: &Arena,
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

        let max_components = num_chunks * components_per_chunk;
        let buffer_capacity_bytes = max_components * component_layout.size();

        // Phase X.A SIMD-A1 (plan §6.2): lift the buffer alignment from the raw
        // `align_of::<T>()` (which can be as low as 1 byte) to at least
        // `SIMD_BUFFER_ALIGN = 32` so that column-start addresses are AVX2-loadable
        // without an unaligned-prologue. For component types whose alignment
        // already exceeds 32 (rare; e.g. `#[repr(align(64))]`), we honour the
        // stricter requirement via `max`. The cost is at most one alignment-gap
        // per pool (<= 31 B) on the arena side; see plan §6.2 for the bound.
        let element_align = component_layout.align();
        let buffer_align = element_align.max(SIMD_BUFFER_ALIGN);

        // SAFETY: size and alignment are both valid Layout inputs:
        // - `buffer_capacity_bytes` is a product of registry-validated sizes;
        // - `buffer_align` is the max of two power-of-2 alignments
        //   (`component_layout.align()` is a power of 2 by Layout invariant;
        //   `SIMD_BUFFER_ALIGN = 32` is `2^5`), so the result is itself a
        //   power of 2 and a valid alignment per `Layout::from_size_align`.
        let buffer_layout = unsafe {
            Layout::from_size_align_unchecked(buffer_capacity_bytes, buffer_align)
        };

        let buffer = arena.allocate_layout(buffer_layout);

        // Phase X.A SIMD-A1 invariant (plan §6.4): `ComponentPool::new` allocates
        // with `align = max(align_of::<T>(), SIMD_BUFFER_ALIGN)`, so the returned
        // base MUST be SIMD_BUFFER_ALIGN-aligned. Asserted at pool construction so
        // callers (`buffer_ptr`, future `Query::for_each_chunk` inner loops) can
        // rely on the invariant without re-checking.
        debug_assert!(
            (buffer.as_ptr() as usize).is_multiple_of(SIMD_BUFFER_ALIGN),
            "SIMD-A1: ComponentPool buffer ptr {:p} is not SIMD_BUFFER_ALIGN={}-aligned",
            buffer.as_ptr(),
            SIMD_BUFFER_ALIGN
        );

        let mut chunks = Vec::with_capacity(num_chunks);
        for i in 0..num_chunks {
            let start_index = i * components_per_chunk;
            chunks.push(Chunk::new(start_index, components_per_chunk));
        }

        // Phase 10 STORE10: per-row tick buffers zero-initialised at pool
        // construction. Slots above `self.len` are never read as
        // meaningful comparands — `SystemMeta::new` sets `last_run =
        // current_tick - MAX_CHANGE_AGE`, so any post-init write is
        // observable. The buffers are global-allocator `Box<[_]>` per
        // STORE2 (not arena-resident): they grow with `max_components`
        // and stand outside the arena's free-list discipline.
        let added_ticks: Box<[UnsafeCell<Tick>]> = (0..max_components)
            .map(|_| UnsafeCell::new(Tick::ZERO))
            .collect();
        let changed_ticks: Box<[UnsafeCell<Tick>]> = (0..max_components)
            .map(|_| UnsafeCell::new(Tick::ZERO))
            .collect();

        Self {
            // SAFETY: `arena` is a shared reference valid for the lifetime of
            // the owning `EcsMaster`; converting to a raw pointer is lossless
            // (non-null, correct provenance). The field is only stored for
            // future deallocation; it is never dereferenced inside ComponentPool.
            arena: &raw const *arena,
            buffer,
            buffer_capacity_bytes,
            max_components,
            len: 0,
            chunks,
            components_per_chunk,
            component_id,
            component_layout,
            drop_fn,
            component_type_id,
            added_ticks,
            changed_ticks,
        }
    }

    /// Creates a new pool with optimal sizes for the given component type.
    pub fn with_default_sizes(arena: &Arena, component_id: usize) -> Self {
        let component_size = component_registry::get_component_size(component_id)
            .expect("Component not registered");

        let components_per_chunk = Self::get_optimal_chunk_capacity(component_size);
        Self::new(arena, component_id, DEFAULT_CHUNKS_PER_POOL, components_per_chunk)
    }

    /// Determines the optimal number of components per chunk based on size.
    fn get_optimal_chunk_capacity(component_size: usize) -> usize {
        if component_size <= TINY_COMPONENT_THRESHOLD {
            TINY_COMPONENTS_PER_CHUNK
        } else if component_size <= SMALL_COMPONENT_THRESHOLD {
            SMALL_COMPONENTS_PER_CHUNK
        } else if component_size <= MEDIUM_COMPONENT_THRESHOLD {
            MEDIUM_COMPONENTS_PER_CHUNK
        } else {
            LARGE_COMPONENTS_PER_CHUNK
        }
    }

    /// Byte pointer for row `idx`, computed from the stable arena base.
    ///
    /// # Safety
    /// * `idx < self.max_components` (the slot lies inside the buffer allocation);
    ///   reads of LIVE data additionally require `idx < self.len`.
    /// * Valid for `self.component_layout.size()` bytes.
    #[inline]
    unsafe fn row_ptr(&self, idx: usize) -> *mut u8 {
        debug_assert!(idx < self.max_components, "row_ptr: idx out of buffer bounds");
        // SAFETY: idx < max_components ⇒ idx*stride + stride <= max_components*stride
        //   == buffer_capacity_bytes, so the element span is inside the single arena
        //   allocation backing `self.buffer`. Provenance derives from `self.buffer`
        //   via one `add` (the same address the deleted `Unit.ptr` cached). The base
        //   is write-once in `new` and never reallocated (fixed arena capacity).
        unsafe { self.buffer.as_ptr().add(idx * self.component_layout.size()) }
    }

    /// Adds a component to the pool via raw byte slice.
    ///
    /// The caller must ensure `component_bytes` contains a valid, initialized
    /// representation of the pool's registered type.
    ///
    /// Returns the slot index on success, `None` if the pool is full.
    #[doc(hidden)]
    pub fn add(&mut self, component_bytes: &[u8]) -> Option<usize> {
        debug_assert_eq!(
            component_bytes.len(),
            self.component_layout.size(),
            "Component size mismatch: expected {}, got {}",
            self.component_layout.size(),
            component_bytes.len()
        );

        if self.len >= self.max_components {
            return None;
        }

        let buffer_index = self.len;

        // SAFETY: buffer_index < max_components (checked above), so
        // `row_ptr` yields a pointer to a slot inside the pool allocation.
        // The source and destination do not overlap (source is caller
        // memory, destination is arena memory). The row is uninitialised
        // until this write; `self.len += 1` below marks it live.
        unsafe {
            std::ptr::copy_nonoverlapping(
                component_bytes.as_ptr(),
                self.row_ptr(buffer_index),
                self.component_layout.size(),
            );
        }

        let chunk_index = buffer_index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_index) {
            chunk.mark_dirty();
        }
        self.len += 1;

        Some(buffer_index)
    }

    /// Type-checked append. Consumes `value` by move into the pool's slot.
    ///
    /// # Returns
    /// - `Some(slot_index)` on success.
    /// - `None` if pool is at capacity. `value` drops normally at the
    ///   caller's scope exit — the pool is not modified and no slot is
    ///   allocated.
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

        if self.len >= self.max_components {
            return None; // value drops at scope exit
        }

        let buffer_index = self.len;

        // SAFETY:
        // - buffer_index < max_components (just checked), so `row_ptr` yields a
        //   pointer within the pool's allocation.
        // - The slot is aligned to align_of::<T>(): buffer base is aligned to
        //   component_layout.align(); per the Rust Reference §"Type Layout",
        //   size_of::<T>() is a multiple of align_of::<T>() for every Sized T,
        //   so the stride preserves alignment.
        // - The slot is exclusively owned (&mut self); no aliasing.
        // - ptr::write consumes `value` by move; the local binding ceases to
        //   exist after this call — no scope-exit drop.
        unsafe { core::ptr::write(self.row_ptr(buffer_index).cast::<T>(), value) };

        let chunk_index = buffer_index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_index) {
            chunk.mark_dirty();
        }
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

        let chunk_index = last_index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_index) {
            chunk.mark_dirty();
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
            // indices are `< self.len` (the removal precondition guards
            // `index`, and `last_index = self.len - 1`).
            // `&mut self` gives exclusive access to the tick buffers;
            // no concurrent reader exists per Phase 9 SCH3.
            unsafe {
                let added_last = *self.added_ticks[last_index].get();
                let changed_last = *self.changed_ticks[last_index].get();
                *self.added_ticks[index].get() = added_last;
                *self.changed_ticks[index].get() = changed_last;
            }

            let chunk_idx = index / self.components_per_chunk;
            if let Some(chunk) = self.chunks.get_mut(chunk_idx) {
                chunk.mark_dirty();
            }

            let last_chunk_idx = last_index / self.components_per_chunk;
            if let Some(chunk) = self.chunks.get_mut(last_chunk_idx) {
                chunk.mark_dirty();
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

            let chunk_idx = index / self.components_per_chunk;
            if let Some(chunk) = self.chunks.get_mut(chunk_idx) {
                chunk.mark_dirty();
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
        let chunk_index = index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_index) {
            chunk.mark_dirty();
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
        // - Source (caller memory) and destination (arena slot) do not overlap.
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

        let chunk_idx = index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_idx) {
            chunk.mark_dirty();
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

        let chunk_idx = index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_idx) {
            chunk.mark_dirty();
        }
        true
    }

    /// Gets the number of active components.
    #[inline]
    pub fn count(&self) -> usize {
        self.len
    }

    /// Gets the total pool capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.max_components
    }

    /// Gets the number of chunks.
    #[inline]
    pub fn chunks_count(&self) -> usize {
        self.chunks.len()
    }

    /// Returns the chunk metadata slice. Pool internals manage chunk state;
    /// external code should not mutate chunks directly.
    #[inline]
    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
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

    /// Checks if the pool is full.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.len >= self.max_components
    }

    /// Gets the remaining capacity.
    #[inline]
    pub fn remaining_capacity(&self) -> usize {
        self.max_components - self.len
    }

    // ── Phase 11 — Unit-pointer accessors + no-drop scaffolding ─────────────
    //
    // Wave E Step 12 (plan §7.2 / Round 3 C-N2). The migration paths in
    // `commands/migration_helpers.rs` need to read the raw arena pointer
    // for row `idx` so they can build a `&[u8]` retained-bytes slice
    // *before* swapping the row out via `swap_remove_index_no_drop`. The
    // existing `get_raw` returns `Option<*const u8>` but with a non-trivial
    // borrow check signature; `unit_ptr` is the trivial inline alias used
    // exclusively by migration callers.

    /// Returns the raw arena pointer for row `idx`. Panics in debug if
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
        let chunk_idx = idx / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_idx) {
            chunk.mark_dirty();
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
        //   * Source (`bytes`) and destination (arena slot) are disjoint
        //     allocations; `copy_nonoverlapping` is sound.
        //   * Slot is logically uninit (caller contract); no drop runs.
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.row_ptr(idx),
                self.component_layout.size(),
            );
        }
        let chunk_idx = idx / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_idx) {
            chunk.mark_dirty();
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
            //     results are valid arena pointers produced by prior
            //     `add` / `add_typed`.
            //   * Non-overlapping: `idx != last_index`; each slot is
            //     `component_layout.size()` bytes. They may live in
            //     different chunks (large pools span multiple), but
            //     `copy_nonoverlapping` does not require same allocation
            //     — only non-overlap.
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
            // SAFETY: idx != last_index, both < self.len.
            //   `&mut self` ⇒ exclusive access to the tick buffers;
            //   no concurrent reader exists per Phase 9 SCH3.
            unsafe {
                let added_last = *self.added_ticks[last_index].get();
                let changed_last = *self.changed_ticks[last_index].get();
                *self.added_ticks[idx].get() = added_last;
                *self.changed_ticks[idx].get() = changed_last;
            }

            let chunk_idx = idx / self.components_per_chunk;
            if let Some(chunk) = self.chunks.get_mut(chunk_idx) {
                chunk.mark_dirty();
            }
            let last_chunk_idx = last_index / self.components_per_chunk;
            if let Some(chunk) = self.chunks.get_mut(last_chunk_idx) {
                chunk.mark_dirty();
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
        let last_index = self.len - 1;
        let chunk_idx = last_index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_idx) {
            chunk.mark_dirty();
        }
        // W-N2: NO `drop_fn` invocation.
        self.len -= 1;
    }

    // ── Phase 10 STORE3 — tick buffer accessors ─────────────────────────────
    //
    // Wave B Step 5 lands the accessors. Wave C consumers (`Added<C>`,
    // `Changed<C>`, `Mut<T>::deref_mut`) wire the per-row tick reads
    // and the deref-time tick writes. Until Wave C lands, the four
    // accessors below are unused; the `dead_code` allow is removed
    // when Wave C ships.

    /// Returns a base pointer to the per-row `added_ticks` buffer.
    ///
    /// The pointer is valid for `self.capacity()` `UnsafeCell<Tick>`
    /// slots and stays alive for the pool's lifetime (`Box<[_]>` —
    /// never reallocated post-construction). Wave C `Added<C>::set_table_*`
    /// caches this base pointer in its `Fetch<'w>` and indexes per-row.
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn added_ticks_ptr(&self) -> *const UnsafeCell<Tick> {
        self.added_ticks.as_ptr()
    }

    /// Returns a base pointer to the per-row `changed_ticks` buffer.
    ///
    /// Same shape and lifetime contract as [`Self::added_ticks_ptr`].
    /// Wave C `Changed<C>::set_table_*` and `Mut<T>::deref_mut` both
    /// reach the buffer through this pointer.
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn changed_ticks_ptr(&self) -> *const UnsafeCell<Tick> {
        self.changed_ticks.as_ptr()
    }

    /// Writes the `added` tick for row `index`.
    ///
    /// Called on entity insertion (`Archetype::create_entity` → bundle
    /// push) with the world's current tick.
    ///
    /// # Safety
    ///
    /// * `index < self.count()` — the slot must be live (initialised by
    ///   a prior `add` / `add_typed`).
    /// * The caller holds exclusive write access to this `(archetype,
    ///   component)` per Phase 9 SCH3 (the scheduler's conflict graph
    ///   guarantees no concurrent reader of the same slot exists).
    #[inline]
    pub(crate) unsafe fn write_added_tick(&self, index: usize, tick: Tick) {
        debug_assert!(index < self.added_ticks.len());
        // SAFETY: caller asserts `index < self.count() <= self.added_ticks.len()`
        // and Phase 9 SCH3 exclusivity on this `(archetype, component)`.
        // `UnsafeCell::get()` produces a `*mut Tick` to a distinct memory
        // location per row — adjacent-row writes from sibling `par_iter`
        // chunks are sound per Rust's abstract machine (Round 2 C3).
        unsafe {
            *self.added_ticks.get_unchecked(index).get() = tick;
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
        debug_assert!(index < self.changed_ticks.len());
        // SAFETY: caller asserts `index < self.count() <= self.changed_ticks.len()`
        // and Phase 9 SCH3 exclusivity. Per-row `UnsafeCell<Tick>` is a
        // distinct memory location (Round 2 C3).
        unsafe {
            *self.changed_ticks.get_unchecked(index).get() = tick;
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
        debug_assert!(index < self.added_ticks.len());
        // SAFETY: caller asserts `index < self.count() <= self.added_ticks.len()`
        // and Phase 9 SCH3 (at least shared access — no writer). The
        // dereferenced value is `Copy`.
        unsafe { *self.added_ticks.get_unchecked(index).get() }
    }

    /// Reads the `changed` tick for row `index`.
    ///
    /// # Safety
    ///
    /// Same conditions as [`Self::read_added_tick`].
    #[allow(dead_code)]
    #[inline]
    pub(crate) unsafe fn read_changed_tick(&self, index: usize) -> Tick {
        debug_assert!(index < self.changed_ticks.len());
        // SAFETY: caller asserts `index < self.count() <= self.changed_ticks.len()`
        // and Phase 9 SCH3 (at least shared access — no writer).
        unsafe { *self.changed_ticks.get_unchecked(index).get() }
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

    /// Phase 12.5 Opt-A2 (C-N1): returns `true` iff this pool can reserve
    /// `n` more rows (i.e. `count + n ≤ max_components`).
    ///
    /// Cheap inline check used by `Archetype::reserve_capacity` to
    /// pre-validate the entire bundle before any pool is mutated
    /// (two-phase commit; mirrors `can_push_entity_components`).
    #[inline]
    pub(crate) fn can_reserve(&self, n: usize) -> bool {
        self.len
            .checked_add(n)
            .is_some_and(|end| end <= self.max_components)
    }

    /// Phase 12.5 Opt-A2 (C-N1): returns `(current_count, max_components)`
    /// for diagnostic / error-reporting paths (`EcsError::ArchetypePoolCapacityExceeded`).
    #[inline]
    pub(crate) fn len_for_reserve(&self) -> (usize, usize) {
        (self.len, self.max_components)
    }

    /// Phase 12.5 Opt-A2 (SBO13 / §5.6): writes `bytes` into the slot at
    /// `idx` WITHOUT advancing `len`, WITHOUT capacity checks, and
    /// WITHOUT invoking any drop (the slot is logically uninit).
    ///
    /// The batch path uses this for every row in `[start_row, start_row + n)`
    /// after `reserve_capacity` has validated the range and before
    /// `commit_units` advances `self.len`. Slot bookkeeping (`len` +
    /// chunk dirty mark) is deferred to [`Self::commit_units`].
    ///
    /// # Safety
    ///
    /// * `idx < max_components` — caller pre-validated via `can_reserve`
    ///   plus `reserve_capacity`'s archetype-level guard.
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
            idx < self.max_components,
            "write_at_unchecked_initialized: idx {} >= max_components {}",
            idx,
            self.max_components
        );
        debug_assert_eq!(
            bytes.len(),
            self.component_layout.size(),
            "write_at_unchecked_initialized: bytes.len() != layout.size()"
        );
        // SAFETY (mirrors `add` / `write_at`):
        //   * `idx < max_components` ⇒ `row_ptr` addresses a slot within the
        //     pool allocation (the buffer-bounds branch of `row_ptr`'s
        //     contract; this slot is not yet live).
        //   * Source (caller stack) and destination (arena slot) live in
        //     disjoint allocations; `copy_nonoverlapping` is sound.
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
    /// at the tail). Chunk dirty marks are stamped for every chunk the
    /// range touches.
    ///
    /// # Phase 12.6 inline hint
    ///
    /// `#[inline]` so the count=1 caller (`SpawnAtCommand::apply`) inlines
    /// the body and the compiler folds away the per-row constants —
    /// `count == 1` collapses the loop, the chunk range becomes a single
    /// `first_chunk = last_chunk` iteration, and the `get_unchecked_mut`
    /// path below eliminates the `Vec::get_mut` `Option` branch.
    #[inline]
    pub(crate) fn commit_units(&mut self, start_row: usize, count: usize) {
        // Defense-in-depth: `count == 0` would underflow the
        // `(start_row + count - 1) / components_per_chunk` expression
        // below. Callers (`SpawnBatchCommand::apply`) early-return on
        // `n == 0`, but the public method must still be safe to call.
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
            start_row + count <= self.max_components,
            "commit_units: range past max_components"
        );

        // The per-row bytes were already written by the caller's
        // `write_at_unchecked_initialized` calls into the dense buffer
        // (rows `[start_row, start_row + count)`, which the debug_assert above
        // proves equals `[len, len + count)`). With the parallel `Vec<Unit>`
        // removed (Phase X.B), committing the batch is a single length bump —
        // the rows are now addressable via `row_ptr`.
        self.len += count;

        // Mark every touched chunk dirty (range may span multiple).
        //
        // Phase 12.6: `chunks: Vec<Chunk>` is sized to `num_chunks` at
        // pool construction (see `ComponentPool::new` — `chunks.push` in
        // an init loop) and is NEVER mutated thereafter. `start_row +
        // count <= max_components` (debug-asserted above) implies
        // `last_chunk < num_chunks == chunks.len()`, so the `get_mut`
        // bounds check is provably-true on every call — use
        // `get_unchecked_mut` to eliminate the `Option` branch.
        let first_chunk = start_row / self.components_per_chunk;
        let last_chunk = (start_row + count - 1) / self.components_per_chunk;
        debug_assert!(
            last_chunk < self.chunks.len(),
            "commit_units: chunk index {} >= chunks.len() {} (pool construction invariant)",
            last_chunk,
            self.chunks.len()
        );
        // SAFETY:
        //   * `chunks` is fixed-size at pool construction (`chunks.push`
        //     in init loop, never extended).
        //   * `last_chunk < chunks.len()` by the debug-asserted invariant
        //     above (precondition: `start_row + count <= max_components
        //     == num_chunks * components_per_chunk`).
        //   * `&mut self` ⇒ exclusive access; no concurrent reader of
        //     `self.chunks`.
        unsafe {
            for chunk_idx in first_chunk..=last_chunk {
                self.chunks.get_unchecked_mut(chunk_idx).mark_dirty();
            }
        }
    }

    /// Phase 12.5 Opt-A2 (§5.6 / STORE4): writes `tick` into both
    /// `added_ticks[i]` and `changed_ticks[i]` for every `i` in
    /// `[start_row, start_row + count)`.
    ///
    /// Vectorisable: the buffers are `Box<[UnsafeCell<Tick>]>` and
    /// `UnsafeCell<Tick>` is `#[repr(transparent)]` over `Tick`
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
            start_row + count <= self.added_ticks.len(),
            "fill_ticks: range past added_ticks buffer"
        );
        debug_assert!(
            start_row + count <= self.changed_ticks.len(),
            "fill_ticks: range past changed_ticks buffer"
        );
        // SAFETY (STORE4 + SCH3):
        //   * Range `[start_row, start_row + count)` is in-bounds for both
        //     tick buffers (debug-asserted above).
        //   * `&mut self` ⇒ exclusive write access; per-row `UnsafeCell<Tick>`
        //     is a distinct memory location per Rust's abstract machine.
        unsafe {
            let added_base = self.added_ticks.as_ptr();
            let changed_base = self.changed_ticks.as_ptr();
            for i in 0..count {
                *(*added_base.add(start_row + i)).get() = tick;
                *(*changed_base.add(start_row + i)).get() = tick;
            }
        }
    }
}

// SAFETY (SEND10 — Phase 9 §2.4, §9.1, §11.3 + Phase 10 STORE3 / Round 2 C3):
//
// `ComponentPool` becomes `Send + Sync` under the Phase 9 contract:
//
//   - Pool reads (component access on the Query iteration path) take
//     non-overlapping byte ranges between parallel systems, enforced by the
//     scheduler's `ConflictGraph` (SCH3) on the declared `Access` surface.
//     Two concurrently running systems never hold mutable references into the
//     same `ComponentPool` byte range.
//   - The `arena: *const Arena` field is treated as opaque inside the pool
//     (only the pool's own `new` ever dereferences it; that path runs at
//     archetype creation, which is dispatcher-only via `ArchetypeMaster::
//     create_archetype` under the apply window — §9.4 audit row 1).
//   - Pool growth / extension (any path that may invoke `arena.allocate_*`)
//     is restricted to the apply window by the ALLOC1 discipline; no
//     concurrent reader can observe a half-grown pool.
//   - `len: usize` and `Vec<Chunk>` mutations occur only on `&mut self` paths
//     (`add`, `pop`, `swap_remove`, `set_component`); the dispatcher
//     serialises these under the apply window. Worker reads use `&self`
//     entry points (`get_raw`, `buffer_ptr`, `count`).
//   - Phase 10: the `added_ticks` / `changed_ticks` buffers are
//     `Box<[UnsafeCell<Tick>]>`. `UnsafeCell<Tick>` is `!Sync` on its own,
//     but the pool exposes the cells only through unsafe accessors
//     (`write_added_tick`, `write_changed_tick`, `read_added_tick`,
//     `read_changed_tick`) whose contract requires the caller hold the
//     SCH3 exclusivity for writes (or shared access for reads). Each
//     `UnsafeCell<Tick>` is a distinct memory location per Rust's abstract
//     machine — adjacent-row writes from `par_iter` chunks on the same
//     cache line are sound (Round 2 C3 / Rustonomicon §"Data Races and
//     Race Conditions"). The MESI cache-line ping-pong is a perf cost,
//     not UB.
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
            //   add_typed before `self.len` was incremented).
            // - Each `row_ptr(row)` points at a properly-aligned, T-sized,
            //   T-typed allocation (pool construction invariant); `row < len`
            //   satisfies `row_ptr`'s safety contract.
            // - We have exclusive access (Drop receives &mut self).
            // - drop_fn matches the signature unsafe fn(*mut u8) and calls
            //   drop_in_place::<T> which is valid for these initialized slots.
            for row in 0..self.len {
                unsafe { drop_fn(self.row_ptr(row)) }
            }
        }
        // Arena memory release happens via Arena::Drop (M-001).
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
    /// The fixture component `F32Wrap` is `#[repr(transparent)]` over `f32`,
    /// giving `align_of::<F32Wrap>() = 4` — well below
    /// `SIMD_BUFFER_ALIGN = 32`. Without the alignment lift in
    /// `ComponentPool::new`, the buffer pointer would only be 4-byte-aligned;
    /// the lift rounds the buffer alignment up to 32. This test gates the
    /// entire Phase X.A Wave 1 — if it fails, the SIMD alignment story is
    /// broken at the arena layer.
    ///
    /// To prove the assertion is non-tautological, the test deliberately
    /// pre-allocates a `Position` pool (48-byte buffer at align 4) so that
    /// the arena cursor sits at an offset of `48 mod 32 = 16` from the
    /// 64-byte-aligned arena base before the `F32Wrap` pool is constructed.
    /// Without the lift, the `F32Wrap` pool's buffer (align 4) would land at
    /// that 16-mod-32 offset and the assertion would fail; with the lift
    /// (align = max(4, 32) = 32), the arena's `allocate_aligned` advances the
    /// cursor to the next 32-byte boundary before placing the buffer.
    #[test]
    fn buffer_ptr_is_simd_aligned() {
        use crate::ecs::constants::SIMD_BUFFER_ALIGN;

        register_all();
        let arena = Arena::new();

        // Pre-allocate a non-SIMD-aligned-sized chunk so the arena cursor
        // is misaligned relative to SIMD_BUFFER_ALIGN before the test pool
        // is constructed. Position is 12 B with align 4; the pool's buffer
        // (1 chunk × 4 components × 12 B = 48 B) consumes the first 48 B of
        // the arena. 48 mod 32 = 16, so the next free byte is at offset 16
        // from the 64-aligned arena base — i.e. NOT 32-aligned.
        let _prefix = ComponentPool::new(&arena, POS_ID.0, 1, 4);

        // Constructor arguments mirror the rest of the test module:
        // `(arena, component_id, num_chunks, components_per_chunk)`. Using
        // the real `ComponentPool::new` rather than replicating its
        // alignment logic — that would make the assertion tautological.
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
    /// `[len, max_components)` slots.
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
    /// `[len, max_components)` slots dropped (which would over-count).
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
    ///       uninitialised `[len, max_components)` slots;
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
}
