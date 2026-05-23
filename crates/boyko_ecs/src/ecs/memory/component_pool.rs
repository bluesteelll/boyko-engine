use std::alloc::Layout;
use std::any::TypeId;
use std::ptr::NonNull;

use crate::ecs::constants::{
    DEFAULT_CHUNKS_PER_POOL, LARGE_COMPONENTS_PER_CHUNK, MEDIUM_COMPONENTS_PER_CHUNK,
    MEDIUM_COMPONENT_THRESHOLD, SMALL_COMPONENTS_PER_CHUNK, SMALL_COMPONENT_THRESHOLD,
    TINY_COMPONENTS_PER_CHUNK, TINY_COMPONENT_THRESHOLD,
};
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
    /// Reference to the arena for memory allocation.
    arena: NonNull<Arena>,

    /// Buffer for storing components, allocated directly from the arena.
    buffer: NonNull<u8>,

    /// Buffer capacity in bytes.
    buffer_capacity_bytes: usize,

    /// Maximum number of components.
    max_components: usize,

    /// Array of units with direct pointers (always densely packed).
    units: Vec<Unit>,

    /// Chunk metadata.
    pub chunks: Vec<Chunk>,

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

        Self {
            arena: NonNull::from(arena),
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

            ptr as *mut u8
        };

        let unit = Unit::new(component_ptr, buffer_index);
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

        let unit = Unit::new(dst as *mut u8, buffer_index);
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

            self.units[index] = Unit::new(removed_ptr, index);

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

    /// Gets all components in a chunk as raw pointers.
    pub fn get_chunk_component_pointers(&self, chunk_index: usize) -> Option<Vec<*const u8>> {
        if chunk_index >= self.chunks.len() {
            return None;
        }

        let pointers = self
            .units
            .iter()
            .enumerate()
            .filter(|(idx, _)| *idx / self.components_per_chunk == chunk_index)
            .map(|(_, unit)| unit.ptr() as *const u8)
            .collect();

        Some(pointers)
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
}

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
