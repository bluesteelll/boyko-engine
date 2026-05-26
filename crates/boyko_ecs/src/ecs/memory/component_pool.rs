use std::alloc::Layout;
use std::any::TypeId;
use std::cell::UnsafeCell;
use std::ptr::NonNull;

use crate::ecs::constants::{
    DEFAULT_CHUNKS_PER_POOL, LARGE_COMPONENTS_PER_CHUNK, MEDIUM_COMPONENTS_PER_CHUNK,
    MEDIUM_COMPONENT_THRESHOLD, SMALL_COMPONENTS_PER_CHUNK, SMALL_COMPONENT_THRESHOLD,
    TINY_COMPONENTS_PER_CHUNK, TINY_COMPONENT_THRESHOLD,
};
use crate::ecs::core::change_detection::Tick;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry::{self, DropFn};
use crate::ecs::memory::arena::Arena;
use crate::ecs::memory::chunk::Chunk;
use crate::ecs::memory::id_unit::Unit;

/// Pool of components of a specific type with direct pointers.
///
/// All slots in `[0, units.len())` are fully initialized. Slots beyond that
/// are uninitialized arena memory and must never be read or dropped.
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

    /// Array of units with direct pointers (always densely packed).
    units: Vec<Unit>,

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

    /// Phase 10 STORE3: per-row "added at" ticks, parallel to `units`.
    ///
    /// Logical row `i` (= `units[i]`) has its added-at tick at
    /// `added_ticks[i]`. The buffer is sized to `max_components` at pool
    /// construction and never reallocates; slots beyond `units.len()`
    /// stay at [`Tick::ZERO`] until a future write.
    ///
    /// `UnsafeCell<Tick>` provides interior mutability through a shared
    /// `&self` (used by Wave C `Added<C>::filter_fetch` reads through the
    /// `Fetch<'w>` pointer) while still permitting the Phase 9 scheduler
    /// to declare exclusive write access on a per-`(archetype, component)`
    /// basis (SCH3). Adjacent-row writes from sibling `par_iter` chunks
    /// target distinct memory locations — sound per Rust's abstract
    /// machine even though they share a cache line (Round 2 C3, plan §11.5).
    pub(crate) added_ticks: Box<[UnsafeCell<Tick>]>,

    /// Phase 10 STORE3: per-row "last changed at" ticks, parallel to `units`.
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

        // SAFETY: size and alignment come from a registered ComponentLayout,
        // which was produced by size_of::<T>() / align_of::<T>() — always valid.
        let buffer_layout = unsafe {
            Layout::from_size_align_unchecked(buffer_capacity_bytes, component_layout.align())
        };

        let buffer = arena.allocate_layout(buffer_layout);

        let mut chunks = Vec::with_capacity(num_chunks);
        for i in 0..num_chunks {
            let start_index = i * components_per_chunk;
            chunks.push(Chunk::new(start_index, components_per_chunk));
        }

        // Phase 10 STORE10: per-row tick buffers zero-initialised at pool
        // construction. Slots above `units.len()` are never read as
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
            units: Vec::with_capacity(max_components),
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

        if self.units.len() >= self.max_components {
            return None;
        }

        let buffer_index = self.units.len();

        let component_ptr = unsafe {
            let ptr = self.buffer
                .as_ptr()
                .add(buffer_index * self.component_layout.size());

            // SAFETY: buffer_index < max_components (checked above); the
            // destination is within the pool allocation. The source and
            // destination do not overlap (source is caller memory, destination
            // is arena memory).
            std::ptr::copy_nonoverlapping(
                component_bytes.as_ptr(),
                ptr,
                self.component_layout.size(),
            );

            ptr
        };

        let unit = Unit::new(component_ptr);
        let chunk_index = buffer_index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_index) {
            chunk.mark_dirty();
        }
        self.units.push(unit);

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

        if self.units.len() >= self.max_components {
            return None; // value drops at scope exit
        }

        let buffer_index = self.units.len();

        // SAFETY: buffer_index < max_components (just checked); the buffer
        // covers max_components * size_of::<T>() bytes starting at buffer base.
        let dst = unsafe {
            self.buffer
                .as_ptr()
                .add(buffer_index * self.component_layout.size())
        };

        // SAFETY:
        // - dst is within the pool's allocation (buffer_index < max_components).
        // - dst is aligned to align_of::<T>(): buffer base is aligned to
        //   component_layout.align(); per the Rust Reference §"Type Layout",
        //   size_of::<T>() is a multiple of align_of::<T>() for every Sized T,
        //   so the stride preserves alignment.
        // - dst is exclusively owned (&mut self); no aliasing.
        // - ptr::write consumes `value` by move; the local binding ceases to
        //   exist after this call — no scope-exit drop.
        unsafe { core::ptr::write(dst.cast::<T>(), value) };

        let unit = Unit::new(dst);
        let chunk_index = buffer_index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_index) {
            chunk.mark_dirty();
        }
        self.units.push(unit);

        Some(buffer_index)
    }

    /// Removes the last component from the pool, invoking drop glue if needed.
    pub fn pop(&mut self) -> bool {
        if self.units.is_empty() {
            return false;
        }

        let last_index = self.units.len() - 1;
        let last_ptr = self.units[last_index].ptr();

        // SAFETY:
        // - last_ptr came from a prior add/add_typed (initialized slot).
        // - We hold &mut self → exclusive access, no aliasing.
        // - After drop_fn, the slot is logically uninitialized; units.pop()
        //   removes the index entry so the slot becomes unreachable.
        unsafe {
            if let Some(drop_fn) = self.drop_fn {
                drop_fn(last_ptr);
            }
        }

        let chunk_index = last_index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_index) {
            chunk.mark_dirty();
        }
        self.units.pop();

        true
    }

    /// Returns the index of the last component in the pool.
    ///
    /// Useful when determining what will be affected by a `swap_remove`.
    #[inline]
    pub fn last_index(&self) -> Option<usize> {
        if self.units.is_empty() {
            None
        } else {
            Some(self.units.len() - 1)
        }
    }

    /// Removes a component by index using swap_remove to maintain dense storage.
    ///
    /// The component at `index` is dropped via the registered drop glue before
    /// the last component is memcpy'd into its slot.
    pub fn swap_remove(&mut self, index: usize) -> bool {
        if index >= self.units.len() {
            return false;
        }

        let last_index = self.units.len() - 1;

        if index != last_index {
            let removed_ptr = self.units[index].ptr();
            let last_ptr = self.units[last_index].ptr();

            // SAFETY:
            // - removed_ptr came from a prior add/add_typed (initialized slot).
            // - We hold &mut self → exclusive access; removed_ptr and last_ptr
            //   are separate non-overlapping slots (index != last_index, stride
            //   is size_of::<T>() which is > 0 (ZSTs rejected at pool construction)).
            // - After drop_fn, the slot at `index` is logically uninitialized;
            //   the copy_nonoverlapping below overwrites it with last's bytes,
            //   restoring the invariant.
            // - PANIC CAVEAT: if T::drop panics, the slot at `index` is
            //   uninitialized while units.len() still includes it. Per the
            //   Component trait panic policy this is a logic bug in the user's
            //   Drop impl; the pool is considered poisoned.
            unsafe {
                if let Some(drop_fn) = self.drop_fn {
                    drop_fn(removed_ptr);
                }
                // SAFETY: separate, non-overlapping slots; component_layout.size()
                // bytes each; pool allocation is a flat array.
                std::ptr::copy_nonoverlapping(
                    last_ptr,
                    removed_ptr,
                    self.component_layout.size(),
                );
            }

            self.units[index] = Unit::new(removed_ptr);

            // Phase 10 STORE5: swap tick slots in lockstep with the data
            // buffer. The last row's ticks move into the vacated slot so
            // row `index` continues to carry the moved entity's lifecycle
            // history. No tick is dropped here — `Tick` is `Copy`.
            //
            // SAFETY: `index != last_index` (checked above) and both
            // indices are `< self.units.len()` (the removal precondition
            // guards `index`, and `last_index = self.units.len() - 1`).
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
            let removed_ptr = self.units[index].ptr();

            // SAFETY: removed_ptr came from a prior add/add_typed (initialized).
            // Exclusive access via &mut self. units.pop() below removes the
            // entry so the slot becomes unreachable.
            unsafe {
                if let Some(drop_fn) = self.drop_fn {
                    drop_fn(removed_ptr);
                }
            }

            let chunk_idx = index / self.components_per_chunk;
            if let Some(chunk) = self.chunks.get_mut(chunk_idx) {
                chunk.mark_dirty();
            }
        }

        self.units.pop();
        true
    }

    /// Gets a pointer to a component by index.
    pub fn get_raw(&self, index: usize) -> Option<*const u8> {
        if index >= self.units.len() {
            return None;
        }
        Some(self.units[index].ptr())
    }

    /// Gets a mutable pointer to a component by index.
    pub fn get_raw_mut(&mut self, index: usize) -> Option<*mut u8> {
        if index >= self.units.len() {
            return None;
        }
        let chunk_index = index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_index) {
            chunk.mark_dirty();
        }
        Some(self.units[index].ptr())
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
        // - `get_raw` returns `Some(ptr)` only when `index < self.units.len()`,
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
        // - `get_raw_mut` returns `Some(ptr)` only when `index < self.units.len()`,
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
    /// while `self.units.len()` still includes it. Any subsequent operation
    /// on the pool that touches this slot is undefined behavior.
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

        if index >= self.units.len() {
            return false;
        }

        let ptr = self.units[index].ptr();

        // SAFETY:
        // - index < units.len() (checked); units[index] is a live, initialized slot.
        // - ptr is aligned to the pool's component type (pool allocation invariant).
        // - Exclusive access via &mut self; no aliasing.
        // - drop_fn drops the existing value; copy_nonoverlapping writes the
        //   new bytes. Both halves use ptr as the destination/source respectively.
        //   They are sequenced (drop then write), so there is no overlap issue.
        // - If component_bytes is not the correct type representation, the new
        //   slot contents are UB on subsequent typed access — raw API caller's
        //   responsibility (unchanged from pre-existing contract).
        unsafe {
            if let Some(drop_fn) = self.drop_fn {
                drop_fn(ptr);
            }
            // SAFETY: size verified by debug_assert; pool allocation is aligned
            // to component type; source and destination do not overlap (source
            // is caller memory, destination is arena memory).
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
    /// - `false` if `index >= self.units.len()`. `value` drops normally at
    ///   scope exit — the pool is not modified.
    ///
    /// # Panic safety
    /// **This method is NOT panic-safe.** If the existing component's `Drop`
    /// impl panics during the internal drop_fn call, the slot at `index`
    /// becomes logically uninitialized while `self.units.len()` still includes
    /// it. Any subsequent operation on the pool that touches this slot is
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

        if index >= self.units.len() {
            return false; // value drops at scope exit
        }

        let ptr = self.units[index].ptr();

        // SAFETY:
        // - index < units.len() (checked); units[index] is a live, initialized slot.
        // - ptr came from pool allocation; aligned to align_of::<T>() (pool
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

    /// Returns the slice of [`Unit`]s that physically live in `chunk_index`.
    ///
    /// Preferred over the old `get_chunk_component_pointers` API because it is
    /// O(1) and zero-alloc: units belonging to a single chunk occupy the
    /// contiguous range `[start, end)` inside the dense `units` Vec, so no
    /// iteration or heap allocation is needed.  Callers that need raw pointers
    /// can extract them at the call site via `unit.ptr()`.
    ///
    /// Returns `&[]` when `chunk_index` is past the last chunk that has any
    /// live units (including a fully-empty pool).
    #[inline]
    pub fn chunk_units(&self, chunk_index: usize) -> &[Unit] {
        debug_assert!(self.components_per_chunk > 0, "invariant: components_per_chunk > 0");

        let start = chunk_index * self.components_per_chunk;
        if start >= self.units.len() {
            return &[];
        }
        let end = (start + self.components_per_chunk).min(self.units.len());

        // SAFETY: start < self.units.len() (checked above) and
        // end <= self.units.len() (clamped by .min()). Both bounds are within
        // the initialised region [0, units.len()), so the slice is valid.
        &self.units[start..end]
    }

    /// Gets the number of active components.
    #[inline]
    pub fn count(&self) -> usize {
        self.units.len()
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
        self.units.len() >= self.max_components
    }

    /// Gets the remaining capacity.
    #[inline]
    pub fn remaining_capacity(&self) -> usize {
        self.max_components - self.units.len()
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
        debug_assert!(idx < self.units.len(), "unit_ptr: idx out of bounds");
        self.units[idx].ptr().cast_const()
    }

    /// Phase 11 W-N1 defensive check (plan §7.4): returns whether `idx`
    /// is a live row in this pool. Used by
    /// [`crate::ecs::core::component::component_pool_bundle::ComponentPoolBundle::has_pool`]
    /// and the `apply_replace_in_place` debug_assert site.
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn has_row(&self, idx: usize) -> bool {
        idx < self.units.len()
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
        debug_assert!(idx < self.units.len(), "drop_at: idx out of bounds");
        if let Some(drop_fn) = self.drop_fn {
            // SAFETY: `idx < self.units.len()` (debug-asserted) ⇒ the slot
            //   was written by a prior `add` / `add_typed` and contains a
            //   valid `T`. `&mut self` ⇒ exclusive access; the registered
            //   `drop_fn` is `unsafe fn(*mut u8)` (= `drop_in_place::<T>`
            //   under the hood via `register_layout::<T>`).
            unsafe { drop_fn(self.units[idx].ptr()) };
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
        debug_assert!(idx < self.units.len(), "write_at: idx out of bounds");
        debug_assert_eq!(
            bytes.len(),
            self.component_layout.size(),
            "write_at: bytes.len() != layout.size()"
        );
        // SAFETY (mirrors `set_component`):
        //   * `idx < self.units.len()` — slot is reachable.
        //   * `&mut self` ⇒ exclusive access.
        //   * Source (`bytes`) and destination (arena slot) are disjoint
        //     allocations; `copy_nonoverlapping` is sound.
        //   * Slot is logically uninit (caller contract); no drop runs.
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.units[idx].ptr(),
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
    /// Mirrors the existing [`Self::swap_remove`] flow over the chunked +
    /// Unit-pointer storage but skips drop.
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
            idx < self.units.len(),
            "swap_remove_index_no_drop: idx out of bounds"
        );
        let last_index = self.units.len() - 1;

        if idx != last_index {
            let removed_ptr = self.units[idx].ptr();
            let last_ptr = self.units[last_index].ptr();

            // SAFETY (mirrors existing `swap_remove` semantics minus the
            // drop):
            //   * `removed_ptr` and `last_ptr` are valid arena pointers
            //     produced by prior `add` / `add_typed`.
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
                    last_ptr,
                    removed_ptr,
                    self.component_layout.size(),
                );
            }

            // Refresh the unit's pointer (preserves the invariant that
            // `self.units[idx].ptr()` addresses the bytes for row idx).
            self.units[idx] = crate::ecs::memory::id_unit::Unit::new(removed_ptr);

            // Tick swap — mirrors the existing `swap_remove` block.
            // SAFETY: idx != last_index, both < self.units.len().
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
        // (idx == last_index): just pop. No byte/tick movement needed.

        self.units.pop();
    }

    /// Pops the last row without invoking `drop_fn` (plan §7.2 / C5).
    /// Used by [`crate::ecs::core::archetype::archetype::Archetype::move_out_entity`]
    /// when `removed_unit_index == last_unit_index`.
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn pop_entity_no_drop(&mut self) {
        debug_assert!(!self.units.is_empty(), "pop_entity_no_drop: pool empty");
        let last_index = self.units.len() - 1;
        let chunk_idx = last_index / self.components_per_chunk;
        if let Some(chunk) = self.chunks.get_mut(chunk_idx) {
            chunk.mark_dirty();
        }
        // W-N2: NO `drop_fn` invocation.
        self.units.pop();
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
    // slots, then commits `units` and stamps `(added, changed)` ticks in
    // tight loops. All accessors are `pub(crate)` — consumed exclusively
    // by `Archetype::reserve_capacity`, `SpawnBatchCommand::apply`, and
    // `ComponentPoolBundle::commit_units_batch` / `fill_ticks_batch`.

    /// Phase 12.5 Opt-A2 (C-N1): returns `true` iff this pool can reserve
    /// `n` more rows (i.e. `count + n ≤ max_components`).
    ///
    /// Cheap inline check used by `Archetype::reserve_capacity` to
    /// pre-validate the entire bundle before any pool is mutated
    /// (two-phase commit; mirrors `can_push_entity_components`).
    #[inline]
    pub(crate) fn can_reserve(&self, n: usize) -> bool {
        self.units
            .len()
            .checked_add(n)
            .is_some_and(|end| end <= self.max_components)
    }

    /// Phase 12.5 Opt-A2 (C-N1): returns `(current_count, max_components)`
    /// for diagnostic / error-reporting paths (`EcsError::ArchetypePoolCapacityExceeded`).
    #[inline]
    pub(crate) fn len_for_reserve(&self) -> (usize, usize) {
        (self.units.len(), self.max_components)
    }

    /// Phase 12.5 Opt-A2 (SBO13 / §5.6): writes `bytes` into the slot at
    /// `idx` WITHOUT touching `units`, WITHOUT capacity checks, and
    /// WITHOUT invoking any drop (the slot is logically uninit).
    ///
    /// The batch path uses this for every row in `[start_row, start_row + n)`
    /// after `reserve_capacity` has validated the range and before
    /// `commit_units` extends `units.len()`. Slot bookkeeping (units +
    /// chunk dirty mark) is deferred to [`Self::commit_units`].
    ///
    /// # Safety
    ///
    /// * `idx < max_components` — caller pre-validated via `can_reserve`
    ///   plus `reserve_capacity`'s archetype-level guard.
    /// * `idx >= units.len()` (i.e. the slot is uninit and not yet
    ///   committed). After the matching `commit_units(start_row, n)` call
    ///   the slot becomes addressable.
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
        //   * `idx < max_components` ⇒ destination is within the pool
        //     allocation.
        //   * Source (caller stack) and destination (arena slot) live in
        //     disjoint allocations; `copy_nonoverlapping` is sound.
        //   * `&mut self` ⇒ exclusive access.
        //   * The slot is logically uninit by the caller's pre-reserve
        //     contract; no drop runs.
        unsafe {
            let dst = self
                .buffer
                .as_ptr()
                .add(idx * self.component_layout.size());
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                dst,
                self.component_layout.size(),
            );
        }
    }

    /// Phase 12.5 Opt-A2 (§5.6): commits `n` rows starting at `start_row`
    /// into `units` after the batch path has written every row's bytes via
    /// [`Self::write_at_unchecked_initialized`].
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
            self.units.len(),
            "commit_units: start_row {} != current count {} (rows must extend the tail)",
            start_row,
            self.units.len()
        );
        debug_assert!(
            start_row + count <= self.max_components,
            "commit_units: range past max_components"
        );

        // Fused reserve + raw-pointer-write + set_len. Each
        // `Unit { ptr: buffer.add(i * stride) }` is a `#[repr(transparent)]`
        // wrapper around a `*mut u8`, so the inner loop is a strided
        // pointer increment — the compiler vectorises it.
        self.units.reserve(count);
        let stride = self.component_layout.size();
        // SAFETY (commit_units / SBO13):
        //   * `self.units.reserve(count)` above guarantees capacity for
        //     `count` more `Unit` writes past the current `len()`.
        //   * `start_row == self.units.len()` (debug-asserted), so the
        //     written range is exactly `[len, len + count)` — uninitialised
        //     slots inside the existing capacity; no aliasing with
        //     reachable `&Unit` views.
        //   * `start_row + count <= max_components` ⇒ every computed
        //     `base.add(i * stride)` lies inside the arena buffer
        //     allocation, matching the `write_at_unchecked_initialized`
        //     writes performed by the caller for the same slots.
        //   * `Unit` is `#[repr(transparent)]` over `*mut u8` so the raw
        //     write deposits a valid `Unit` representation.
        unsafe {
            let base = self.buffer.as_ptr();
            let units_ptr = self.units.as_mut_ptr();
            for i in 0..count {
                let component_ptr = base.add((start_row + i) * stride);
                std::ptr::write(units_ptr.add(start_row + i), Unit::new(component_ptr));
            }
            self.units.set_len(start_row + count);
        }

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
//   - `Vec<Unit>` and `Vec<Chunk>` mutations occur only on `&mut self` paths
//     (`add`, `pop`, `swap_remove`, `set_component`); the dispatcher
//     serialises these under the apply window. Worker reads use `&self`
//     entry points (`get_raw`, `chunk_units`, `buffer_ptr`, `count`).
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
            // - units[0..len] are all live and initialized per the pool's
            //   invariant (every slot up to units.len() was written by add or
            //   add_typed before being tracked in `units`).
            // - Each unit's ptr() points at a properly-aligned, T-sized,
            //   T-typed allocation (pool construction invariant).
            // - We have exclusive access (Drop receives &mut self).
            // - drop_fn matches the signature unsafe fn(*mut u8) and calls
            //   drop_in_place::<T> which is valid for these initialized slots.
            for unit in &self.units {
                unsafe { drop_fn(unit.ptr()) }
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
    const POS_ID: ComponentId = ComponentId(220);
    const VEL_ID: ComponentId = ComponentId(221);
    const OTHER_ID: ComponentId = ComponentId(222);

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

    // ---- helpers -------------------------------------------------------------------

    fn register_all() {
        component_registry::register_layout::<Position>(POS_ID.0);
        component_registry::register_layout::<Velocity>(VEL_ID.0);
        component_registry::register_layout::<OtherComponent>(OTHER_ID.0);
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
}
