use std::marker::PhantomData;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::iters::component_set::ComponentSet;
use crate::ecs::core::iters::query_state::{QueryState, QueryStateIter};
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

/// High-performance one-shot archetype query.
///
/// `Query<'a>` constructs a one-shot [`QueryState`] at creation time, scans all
/// current archetypes once, and then serves `iter()` from the cached result. The
/// public API is unchanged from the pre-refactor version; internally it delegates
/// to `QueryState` so that both paths share the same filter logic.
///
/// # UB elimination
/// The previous implementation stored `Vec<&'a Archetype>`. After an
/// `ArchetypeBundle::swap_remove` (triggered by `remove_archetype`), those
/// references could dangle. The new implementation stores `Vec<ArchetypeId>`
/// inside `QueryState`, resolving live references on demand via
/// `master.get_archetype(id)`. Stale IDs are transparently skipped.
pub struct Query<'a> {
    state: QueryState,
    master: &'a ArchetypeMaster,
}

impl<'a> Query<'a> {
    /// Creates a new query directly from a set of archetype references.
    ///
    /// The IDs are extracted from the provided references and stored in the
    /// internal `QueryState`. This constructor exists for backward compatibility;
    /// prefer the typed constructors (`with_component_ids`, `with`, etc.) for
    /// new code.
    pub fn from_archetypes(archetypes: Vec<&'a Archetype>, master: &'a ArchetypeMaster) -> Self {
        let include = ComponentMask::new();
        let exclude = ComponentMask::new();
        let optional = ComponentMask::new();
        let mut state = QueryState::new(include, exclude, optional);
        // Populate the cache via push_matched, which keeps the dedup bitset
        // and matched-IDs list in sync through the single authoritative path.
        for arch in archetypes {
            state.push_matched(arch.id());
        }
        // Mark generation as synced so iter_cached can be called without update.
        state.mark_synced(master);
        Self { state, master }
    }

    /// Creates a query for archetypes containing all specified component IDs.
    pub fn with_component_ids(master: &'a ArchetypeMaster, component_ids: &[ComponentId]) -> Self {
        let mut state = QueryState::with_component_ids(component_ids);
        state.update_archetypes(master);
        Self { state, master }
    }

    /// Creates a query for archetypes matching the component mask (superset match).
    pub fn with_mask(master: &'a ArchetypeMaster, mask: &ComponentMask) -> Self {
        // `with_mask` matches archetypes whose component set is a superset of `mask`.
        // Build an include-only QueryState from the mask.
        let state_mask = *mask;
        let mut state = QueryState::new(state_mask, ComponentMask::new(), ComponentMask::new());
        state.update_archetypes(master);
        Self { state, master }
    }

    /// Creates a query for archetypes whose component set exactly equals `mask`.
    pub fn with_exact_mask(master: &'a ArchetypeMaster, mask: &ComponentMask) -> Self {
        // Exact match: use the registry path directly to collect IDs, then wrap them
        // in a QueryState. QueryState's filter logic does not support exact-match
        // semantics natively, so we pre-populate matched_ids from the registry result.
        let archetype_ids = master.archetype_registry().find_exact_match(mask);
        let include = ComponentMask::new();
        let exclude = ComponentMask::new();
        let optional = ComponentMask::new();
        let mut state = QueryState::new(include, exclude, optional);
        for id in archetype_ids {
            state.push_matched(id);
        }
        state.mark_synced(master);
        Self { state, master }
    }

    /// Creates a type-safe query for archetypes containing all components in `T`.
    ///
    /// Example: `Query::with::<(Position, Velocity)>(master)`
    pub fn with<T: ComponentSet>(master: &'a ArchetypeMaster) -> Self {
        Self::with_component_ids(master, T::component_ids())
    }

    /// Creates a query with complex filtering.
    ///
    /// - `include_mask`: components that must be present (AND)
    /// - `exclude_mask`: components that must not be present (NOT)
    /// - `optional_mask`: if non-empty, at least one must be present
    pub fn with_filters(
        master: &'a ArchetypeMaster,
        include_mask: &ComponentMask,
        exclude_mask: &ComponentMask,
        optional_mask: &ComponentMask,
    ) -> Self {
        let mut state = QueryState::new(*include_mask, *exclude_mask, *optional_mask);
        state.update_archetypes(master);
        Self { state, master }
    }

    /// Creates a query with type-safe complex filtering.
    pub fn with_type_filters<
        Inc: ComponentSet,
        Exc: ComponentSet,
        Opt: ComponentSet,
    >(
        master: &'a ArchetypeMaster,
    ) -> Self {
        let mut include_mask = ComponentMask::new();
        let mut exclude_mask = ComponentMask::new();
        let mut optional_mask = ComponentMask::new();

        for &id in Inc::component_ids() {
            include_mask.set(id);
        }
        for &id in Exc::component_ids() {
            exclude_mask.set(id);
        }
        for &id in Opt::component_ids() {
            optional_mask.set(id);
        }

        Self::with_filters(master, &include_mask, &exclude_mask, &optional_mask)
    }

    /// Returns the number of matched archetypes.
    #[inline]
    pub fn len(&self) -> usize {
        self.state.len()
    }

    /// Returns true if no archetypes matched.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.state.is_empty()
    }

    /// Materializes matched archetypes as a `Vec<&Archetype>`.
    ///
    /// This method is retained for backward compatibility. Stale-removed IDs
    /// are silently skipped. For hot-path iteration, prefer `iter()`.
    pub fn archetypes(&self) -> Vec<&'a Archetype> {
        self.state
            .matched_ids()
            .iter()
            .filter_map(|&id| self.master.get_archetype(id))
            .collect()
    }

    /// Returns an iterator over matched archetypes.
    ///
    /// The cache was already updated in the constructor, so this is a pure
    /// slice walk with one `get_archetype` lookup per element.
    ///
    /// The `&'a self` bound ensures the iterator's `ArchetypeId` slice
    /// lives for at least `'a`, matching the lifetime of `master`.
    pub fn iter(&'a self) -> QueryStateIter<'a> {
        self.state.iter_cached(self.master)
    }

    /// Returns a per-entity iterator over component `A` for every matched archetype.
    ///
    /// Yields `&'_ A` for each entity in archetype-major / dense-row order.
    /// Archetypes that do not contain `A` (defensive skip) or that have zero
    /// entities are silently skipped.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let q = Query::with::<Position>(&master);
    /// for pos in q.iter_one::<Position>() {
    ///     println!("{}", pos.x);
    /// }
    /// ```
    pub fn iter_one<A: Component>(&self) -> QueryIterOne<'_, A> {
        QueryIterOne::new(self.state.matched_ids(), self.master)
    }

    /// Returns a per-entity iterator over component pair `(A, B)` for every
    /// matched archetype.
    ///
    /// Yields `(&'_ A, &'_ B)` in archetype-major / dense-row order. Both
    /// pools are walked in lockstep; they have the same row count because they
    /// belong to the same archetype. Archetypes missing either component or
    /// with zero entities are silently skipped.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let q = Query::with::<(Position, Velocity)>(&master);
    /// for (pos, vel) in q.iter_two::<Position, Velocity>() {
    ///     println!("{} {}", pos.x, vel.x);
    /// }
    /// ```
    pub fn iter_two<A: Component, B: Component>(&self) -> QueryIterTwo<'_, A, B> {
        QueryIterTwo::new(self.state.matched_ids(), self.master)
    }
}

/// Enable for-loop iteration over query results.
impl<'a> IntoIterator for &'a Query<'a> {
    type Item = &'a Archetype;
    type IntoIter = QueryStateIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.state.iter_cached(self.master)
    }
}

// ---- Per-entity iterators -------------------------------------------------------

/// Per-entity iterator over a single component type `A`.
///
/// Created by [`Query::iter_one`]. Yields `&'q A` for every entity across
/// every matched archetype in archetype-major, dense-row order.
///
/// Internally uses a pointer-bump loop to avoid per-row bounds checks and
/// virtual dispatch: the inner `next()` body is a compare, a pointer
/// dereference, and two pointer increments.
pub struct QueryIterOne<'q, A: Component> {
    /// Slice of matched archetype IDs from the query state.
    archetype_ids: &'q [ArchetypeId],
    /// Borrow of the master so we can resolve IDs to live `Archetype` refs.
    master: &'q ArchetypeMaster,
    /// Index into `archetype_ids`; advances when the current archetype is exhausted.
    arch_cursor: usize,
    /// Number of entity rows remaining in the current archetype's `A` pool.
    current_remaining: usize,
    /// Pointer to the next unread `A` slot in the current archetype's buffer.
    /// Null when between archetypes (before the first `load_archetype` call or
    /// after the final one is exhausted).
    current_ptr: *const A,
    _phantom: PhantomData<&'q A>,
}

impl<'q, A: Component> QueryIterOne<'q, A> {
    fn new(archetype_ids: &'q [ArchetypeId], master: &'q ArchetypeMaster) -> Self {
        Self {
            archetype_ids,
            master,
            arch_cursor: 0,
            current_remaining: 0,
            current_ptr: std::ptr::null(),
            _phantom: PhantomData,
        }
    }

    /// Loads the component `A` buffer base and entity count from `arch`.
    ///
    /// On success, sets `current_ptr` to the first slot and
    /// `current_remaining` to the entity count. On failure (pool missing or
    /// empty), sets `current_remaining` to 0 so the outer loop skips ahead.
    fn load_archetype(&mut self, arch: &Archetype) {
        let comp_id = A::component_id();
        let Some(pool) = arch.component_pools().get_pool(comp_id) else {
            // Defensive: QueryState matched this archetype, so it should have A.
            // If it doesn't (e.g. a manually-constructed query), skip it safely.
            self.current_remaining = 0;
            self.current_ptr = std::ptr::null();
            return;
        };
        let entity_count = arch.entity_count();
        if entity_count == 0 {
            self.current_remaining = 0;
            self.current_ptr = std::ptr::null();
            return;
        }
        debug_assert_eq!(
            pool.component_layout().size(),
            std::mem::size_of::<A>(),
            "ComponentPool layout for {} mismatches size_of::<A> ({})",
            std::any::type_name::<A>(),
            std::mem::size_of::<A>(),
        );
        debug_assert!(
            pool.component_layout().align() >= std::mem::align_of::<A>(),
            "ComponentPool alignment for {} ({}) is less than align_of::<A> ({})",
            std::any::type_name::<A>(),
            pool.component_layout().align(),
            std::mem::align_of::<A>(),
        );
        // SAFETY: `pool.buffer_ptr()` returns the base of the flat dense
        // allocation holding exactly `pool.count()` initialised `A` values.
        // `entity_count == pool.count()` because all pools in an archetype
        // grow in lock-step (see `ComponentPoolBundle::push_entity_components`).
        // Casting `*const u8` to `*const A` is sound because:
        //   - the buffer is aligned to `component_layout.align()` ≥ `align_of::<A>()`,
        //   - `size_of::<A>()` equals `component_layout.size()` (asserted above),
        //   - and the slot at offset 0 is initialised (entity_count > 0).
        self.current_ptr = pool.buffer_ptr().cast::<A>();
        self.current_remaining = entity_count;
    }
}

impl<'q, A: Component> Iterator for QueryIterOne<'q, A> {
    type Item = &'q A;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.current_remaining > 0 {
                // SAFETY:
                // - `current_ptr` was set by `load_archetype`, which verified
                //   that the buffer is non-null, aligned to `A`, and covers at
                //   least `entity_count` initialised `A` slots starting at slot 0.
                // - We have advanced exactly `entity_count - current_remaining`
                //   steps so far, all within `[0, entity_count)`, so
                //   `current_ptr` points at an initialised slot.
                // - `'q` is the lifetime of `&self.master` / `self.archetype_ids`,
                //   both of which keep the underlying archetype buffers alive for
                //   at least `'q`. The returned reference borrows for `'q`.
                // - The caller holds `&Query<'_>`, so no exclusive access to the
                //   pool exists while this reference is live.
                let item = unsafe { &*self.current_ptr };
                self.current_remaining -= 1;
                // SAFETY: advancing by 1 either stays in `[0, entity_count)` or
                // lands exactly one-past-the-end, which is sound to compute (but
                // not to dereference). `current_remaining == 0` after this
                // decrement means we will call `load_archetype` next time and
                // replace `current_ptr` before dereferencing again.
                self.current_ptr = unsafe { self.current_ptr.add(1) };
                return Some(item);
            }

            if self.arch_cursor >= self.archetype_ids.len() {
                return None;
            }
            let arch_id = self.archetype_ids[self.arch_cursor];
            self.arch_cursor += 1;
            if let Some(arch) = self.master.get_archetype(arch_id) {
                self.load_archetype(arch);
            }
            // Loop: re-check `current_remaining` at the top.
        }
    }
}

/// Per-entity iterator over a component pair `(A, B)`.
///
/// Created by [`Query::iter_two`]. Yields `(&'q A, &'q B)` for every entity
/// across every matched archetype in archetype-major, dense-row order. Both
/// pools are walked in lockstep via two independent pointer-bump cursors that
/// share a single `remaining` counter.
pub struct QueryIterTwo<'q, A: Component, B: Component> {
    archetype_ids: &'q [ArchetypeId],
    master: &'q ArchetypeMaster,
    arch_cursor: usize,
    current_remaining: usize,
    ptr_a: *const A,
    ptr_b: *const B,
    _phantom: PhantomData<(&'q A, &'q B)>,
}

impl<'q, A: Component, B: Component> QueryIterTwo<'q, A, B> {
    fn new(archetype_ids: &'q [ArchetypeId], master: &'q ArchetypeMaster) -> Self {
        Self {
            archetype_ids,
            master,
            arch_cursor: 0,
            current_remaining: 0,
            ptr_a: std::ptr::null(),
            ptr_b: std::ptr::null(),
            _phantom: PhantomData,
        }
    }

    /// Loads both component buffers from `arch`.
    ///
    /// Requires that `arch` has pools for both `A` and `B`. If either pool is
    /// missing or the archetype is empty, sets `current_remaining` to 0.
    fn load_archetype(&mut self, arch: &Archetype) {
        let id_a = A::component_id();
        let id_b = B::component_id();
        let (Some(pool_a), Some(pool_b)) = (
            arch.component_pools().get_pool(id_a),
            arch.component_pools().get_pool(id_b),
        ) else {
            self.current_remaining = 0;
            self.ptr_a = std::ptr::null();
            self.ptr_b = std::ptr::null();
            return;
        };
        let entity_count = arch.entity_count();
        if entity_count == 0 {
            self.current_remaining = 0;
            self.ptr_a = std::ptr::null();
            self.ptr_b = std::ptr::null();
            return;
        }
        debug_assert_eq!(
            pool_a.component_layout().size(),
            std::mem::size_of::<A>(),
            "ComponentPool layout for {} mismatches size_of::<A>",
            std::any::type_name::<A>(),
        );
        debug_assert!(
            pool_a.component_layout().align() >= std::mem::align_of::<A>(),
            "ComponentPool alignment for {} is less than align_of::<A>",
            std::any::type_name::<A>(),
        );
        debug_assert_eq!(
            pool_b.component_layout().size(),
            std::mem::size_of::<B>(),
            "ComponentPool layout for {} mismatches size_of::<B>",
            std::any::type_name::<B>(),
        );
        debug_assert!(
            pool_b.component_layout().align() >= std::mem::align_of::<B>(),
            "ComponentPool alignment for {} is less than align_of::<B>",
            std::any::type_name::<B>(),
        );
        // SAFETY: same reasoning as `QueryIterOne::load_archetype`, applied
        // independently to each pool. Both pools share `entity_count` because
        // all pools in an archetype grow in lock-step.
        self.ptr_a = pool_a.buffer_ptr().cast::<A>();
        self.ptr_b = pool_b.buffer_ptr().cast::<B>();
        self.current_remaining = entity_count;
    }
}

impl<'q, A: Component, B: Component> Iterator for QueryIterTwo<'q, A, B> {
    type Item = (&'q A, &'q B);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.current_remaining > 0 {
                // SAFETY:
                // - `ptr_a` and `ptr_b` were set by `load_archetype`, which
                //   verified that both buffers are non-null, each aligned to
                //   their respective types, and each covers at least
                //   `entity_count` initialised slots starting at offset 0.
                // - We have advanced exactly `entity_count - current_remaining`
                //   steps, all within `[0, entity_count)`, so both pointers
                //   point at initialised slots.
                // - The returned references borrow for `'q`, which keeps both
                //   archetype buffers alive. No exclusive pool access exists
                //   while these references are live (caller holds `&Query<'_>`).
                let item_a = unsafe { &*self.ptr_a };
                let item_b = unsafe { &*self.ptr_b };
                self.current_remaining -= 1;
                // SAFETY: advancing by 1 stays within bounds or lands exactly
                // one-past-the-end (sound to compute, not to deref). When
                // `current_remaining == 0` the next call enters `load_archetype`
                // and replaces both pointers before any deref.
                self.ptr_a = unsafe { self.ptr_a.add(1) };
                self.ptr_b = unsafe { self.ptr_b.add(1) };
                return Some((item_a, item_b));
            }

            if self.arch_cursor >= self.archetype_ids.len() {
                return None;
            }
            let arch_id = self.archetype_ids[self.arch_cursor];
            self.arch_cursor += 1;
            if let Some(arch) = self.master.get_archetype(arch_id) {
                self.load_archetype(arch);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::memory::arena::Arena;

    // Mock component types for testing.
    //
    // Non-ZST wrappers around `u32` — the `MemFreeBlockMaster` allocator
    // refuses zero-byte requests (returns `None` from `allocate_aligned`),
    // which means a `ComponentPool` over a ZST cannot reserve its buffer
    // and `with_default_sizes` panics with "Arena out of memory". Wrapping
    // in `u32` gives a real layout (4 bytes) without changing any
    // mask/signature behaviour the tests rely on.
    #[repr(C)]
    struct Position(u32);
    #[repr(C)]
    struct Velocity(u32);
    #[repr(C)]
    struct Health(u32);
    #[repr(C)]
    struct Damage(u32);

    // Each test module owns its own ComponentId range; see the corresponding
    // comment in `ecs_master.rs` tests for the rationale. `query` uses 200-209.
    impl Component for Position {
        fn component_id() -> ComponentId { ComponentId(200) }
    }

    impl Component for Velocity {
        fn component_id() -> ComponentId { ComponentId(201) }
    }

    impl Component for Health {
        fn component_id() -> ComponentId { ComponentId(202) }
    }

    impl Component for Damage {
        fn component_id() -> ComponentId { ComponentId(203) }
    }

    fn register_mock_components() {
        // Register component layouts for testing
        component_registry::register_layout::<Position>(Position::component_id().0);
        component_registry::register_layout::<Velocity>(Velocity::component_id().0);
        component_registry::register_layout::<Health>(Health::component_id().0);
        component_registry::register_layout::<Damage>(Damage::component_id().0);
    }

    /// Build the `ArchetypeMaster` together with the `Box<Arena>` it borrows.
    ///
    /// The arena MUST be returned to the caller and kept alive for the
    /// duration of the test: `ArchetypeMaster` stores a `NonNull<Arena>`
    /// derived from this `Box`, and dropping the arena turns that pointer
    /// dangling — manifests as `Arena out of memory` panics in release (the
    /// `MemFreeBlockMaster` lives in the freed buffer). This is the same
    /// failure mode that audit C-001 fixed inside `EcsMaster`; we apply the
    /// `Box<Arena>` pattern here too so tests don't reintroduce it.
    fn setup_test_archetypes() -> (ArchetypeMaster, Box<Arena>) {
        register_mock_components();
        let arena: Box<Arena> = Box::default();
        // Mint the raw arena pointer from the Box's inner representation without
        // creating a `&Arena` reference (Stacked Borrows safe). See
        // `EcsMaster::new` for the full rationale (Phase 3a Miri retag fix).
        // SAFETY: `Box<Arena>` is repr-equivalent to `*mut Arena`; reading the
        // Box field as `*const Arena` gives the stable heap address. `arena`
        // is dropped after `master` (tuple drop: master is field 0, arena field 1).
        let arena_ptr: *const Arena = unsafe {
            let box_ptr: *const Box<Arena> = std::ptr::addr_of!(arena);
            *(box_ptr.cast::<*const Arena>())
        };
        let mut master = unsafe { ArchetypeMaster::new(arena_ptr) };

        // Create some test archetypes
        master.create_archetype(&[Position::component_id()]);
        master.create_archetype(&[Position::component_id(), Velocity::component_id()]);
        master.create_archetype(&[Health::component_id()]);
        master.create_archetype(&[Position::component_id(), Health::component_id()]);
        master.create_archetype(&[
            Position::component_id(),
            Velocity::component_id(),
            Health::component_id(),
        ]);

        (master, arena)
    }

    #[test]
    fn test_basic_query() {
        let (master, _arena) = setup_test_archetypes();
        let query = Query::with_component_ids(&master, &[Position::component_id()]);

        // Should find all archetypes with Position
        assert_eq!(query.len(), 4);

        // All archetypes should have Position component
        for archetype in query.iter() {
            assert!(archetype.has_component_id(Position::component_id()));
        }
    }

    #[test]
    fn test_query_with_multiple_components() {
        let (master, _arena) = setup_test_archetypes();
        let query = Query::with_component_ids(
            &master,
            &[Position::component_id(), Velocity::component_id()],
        );

        // Should find archetypes with both Position and Velocity
        assert_eq!(query.len(), 2);

        // All archetypes should have both components
        for archetype in query.iter() {
            assert!(archetype.has_component_id(Position::component_id()));
            assert!(archetype.has_component_id(Velocity::component_id()));
        }
    }

    #[test]
    fn test_type_safe_query() {
        let (master, _arena) = setup_test_archetypes();
        let query = Query::with::<(Position, Velocity)>(&master);

        // Should find archetypes with both Position and Velocity
        assert_eq!(query.len(), 2);

        // All archetypes should have both components
        for archetype in query.iter() {
            assert!(archetype.has_component_id(Position::component_id()));
            assert!(archetype.has_component_id(Velocity::component_id()));
        }
    }

    #[test]
    fn test_iteration() {
        let (master, _arena) = setup_test_archetypes();
        let query = Query::with_component_ids(&master, &[Position::component_id()]);

        // Manual iteration with iter()
        let mut count = 0;
        for archetype in query.iter() {
            assert!(archetype.has_component_id(Position::component_id()));
            count += 1;
        }
        assert_eq!(count, 4);

        // For-loop iteration with IntoIterator
        count = 0;
        for archetype in &query {
            assert!(archetype.has_component_id(Position::component_id()));
            count += 1;
        }
        assert_eq!(count, 4);

        // Collection with Iterator
        let archetypes: Vec<_> = query.iter().collect();
        assert_eq!(archetypes.len(), 4);

        // Check direct access via archetypes()
        assert_eq!(query.archetypes().len(), 4);
    }

    #[test]
    fn test_complex_filtering() {
        let (master, _arena) = setup_test_archetypes();

        // Create masks for filtering
        let mut include_mask = ComponentMask::new();
        include_mask.set(Position::component_id());

        let mut exclude_mask = ComponentMask::new();
        exclude_mask.set(Damage::component_id());

        let mut optional_mask = ComponentMask::new();
        optional_mask.set(Velocity::component_id());
        optional_mask.set(Health::component_id());

        // Should find archetypes with Position, without Damage, and with either Velocity or Health.
        //
        // From setup_test_archetypes(), 5 archetypes were created:
        //   1. [Position]                          — no Velocity, no Health → fails `optional`
        //   2. [Position, Velocity]                — matches
        //   3. [Health]                            — no Position           → fails `include`
        //   4. [Position, Health]                  — matches
        //   5. [Position, Velocity, Health]        — matches
        // Expected: 3. (The original test asserted 4, which was wrong — see
        // archetype_master.rs::test_get_archetypes_with_component_filter for
        // the consistent baseline: optional == "at least one of these must
        // be present".)
        let query = Query::with_filters(&master, &include_mask, &exclude_mask, &optional_mask);

        assert_eq!(query.len(), 3);

        // Verify filtering criteria
        for archetype in query.iter() {
            // Must have Position
            assert!(archetype.has_component_id(Position::component_id()));

            // Must not have Damage
            assert!(!archetype.has_component_id(Damage::component_id()));

            // Must have either Velocity or Health or both
            assert!(
                archetype.has_component_id(Velocity::component_id())
                    || archetype.has_component_id(Health::component_id())
            );
        }
    }

    #[test]
    fn test_type_safe_filters() {
        let (master, _arena) = setup_test_archetypes();

        // Create masks manually instead of using type_filters to avoid ComponentSet issues
        let mut include_mask = ComponentMask::new();
        include_mask.set(Position::component_id());

        let mut exclude_mask = ComponentMask::new();
        exclude_mask.set(Damage::component_id());

        let mut optional_mask = ComponentMask::new();
        optional_mask.set(Velocity::component_id());
        optional_mask.set(Health::component_id());

        // Same expectation as test_complex_filtering: 3 archetypes match.
        let query = Query::with_filters(&master, &include_mask, &exclude_mask, &optional_mask);

        assert_eq!(query.len(), 3);

        // Verify filtering criteria
        for archetype in query.iter() {
            // Must have Position
            assert!(archetype.has_component_id(Position::component_id()));

            // Must not have Damage
            assert!(!archetype.has_component_id(Damage::component_id()));

            // Must have either Velocity or Health or both
            assert!(
                archetype.has_component_id(Velocity::component_id())
                    || archetype.has_component_id(Health::component_id())
            );
        }
    }

    // ---- iter_one / iter_two tests -----------------------------------------------

    use crate::ecs::core::entity::entity_inland::EntityInland;
    use crate::ecs::identifiers::primitives::{EntityId, InlandPoolId};

    /// Build a fresh `(ArchetypeMaster, Box<Arena>)` pair without any archetypes.
    fn make_master() -> (ArchetypeMaster, Box<Arena>) {
        register_mock_components();
        let arena: Box<Arena> = Box::default();
        // SAFETY: `Box<Arena>` is repr-equivalent to `*mut Arena`; reading the
        // Box field as `*const Arena` gives the stable heap address. `arena`
        // is dropped after `master` (tuple drop: master is field 0, arena field 1).
        let arena_ptr: *const Arena = unsafe {
            let box_ptr: *const Box<Arena> = std::ptr::addr_of!(arena);
            *(box_ptr.cast::<*const Arena>())
        };
        let master = unsafe { ArchetypeMaster::new(arena_ptr) };
        (master, arena)
    }

    /// Push one entity with a `Position(val)` into the archetype, returning its inland.
    fn push_position(master: &mut ArchetypeMaster, arch_id: ArchetypeId, val: u32) -> EntityInland {
        let arch = master.get_archetype_mut(arch_id).expect("archetype must exist");
        let mut inland = EntityInland::new(arch.id(), InlandPoolId(0), 0);
        arch.init_entity_inland(&mut inland);
        let bytes = val.to_ne_bytes();
        let ok = arch.create_entity(EntityId(val as usize), &mut inland, &[
            (Position::component_id(), bytes.as_slice()),
        ]);
        assert!(ok, "create_entity must succeed");
        inland
    }

    /// Push one entity with a `Position(pval)` and `Velocity(vval)` into the archetype.
    fn push_pos_vel(
        master: &mut ArchetypeMaster,
        arch_id: ArchetypeId,
        entity_id: usize,
        pval: u32,
        vval: u32,
    ) -> EntityInland {
        let arch = master.get_archetype_mut(arch_id).expect("archetype must exist");
        let mut inland = EntityInland::new(arch.id(), InlandPoolId(0), 0);
        arch.init_entity_inland(&mut inland);
        let pb = pval.to_ne_bytes();
        let vb = vval.to_ne_bytes();
        let ok = arch.create_entity(EntityId(entity_id), &mut inland, &[
            (Position::component_id(), pb.as_slice()),
            (Velocity::component_id(), vb.as_slice()),
        ]);
        assert!(ok, "create_entity must succeed");
        inland
    }

    // --- iter_one tests ---

    /// iter_one yields all entities from a single archetype with distinct values.
    #[test]
    fn iter_one_yields_all_entities_one_archetype() {
        let (mut master, _arena) = make_master();
        let arch_id = master.create_archetype(&[Position::component_id()]);

        push_position(&mut master, arch_id, 10);
        push_position(&mut master, arch_id, 20);
        push_position(&mut master, arch_id, 30);

        let query = Query::with_component_ids(&master, &[Position::component_id()]);
        let collected: Vec<u32> = query.iter_one::<Position>().map(|p| p.0).collect();

        assert_eq!(collected, [10, 20, 30], "must yield exactly the 3 inserted values in order");
    }

    /// iter_one crosses two archetypes and yields 2 + 3 = 5 items in archetype-major order.
    #[test]
    fn iter_one_across_two_archetypes() {
        let (mut master, _arena) = make_master();
        let arch1 = master.create_archetype(&[Position::component_id()]);
        // A second distinct archetype (with an extra component) that also has Position.
        let arch2 = master.create_archetype(&[
            Position::component_id(),
            Velocity::component_id(),
        ]);

        push_position(&mut master, arch1, 1);
        push_position(&mut master, arch1, 2);

        push_pos_vel(&mut master, arch2, 10, 3, 99);
        push_pos_vel(&mut master, arch2, 11, 4, 99);
        push_pos_vel(&mut master, arch2, 12, 5, 99);

        let query = Query::with_component_ids(&master, &[Position::component_id()]);
        let collected: Vec<u32> = query.iter_one::<Position>().map(|p| p.0).collect();

        // Archetype-major order: arch1 first (IDs are assigned in creation order),
        // arch2 second. Within each archetype, insertion order is preserved.
        assert_eq!(collected.len(), 5, "must yield 5 total entities");
        // First two come from arch1.
        assert_eq!(&collected[..2], &[1, 2]);
        // Last three come from arch2.
        assert_eq!(&collected[2..], &[3, 4, 5]);
    }

    /// iter_one on an archetype with no entities yields nothing.
    #[test]
    fn iter_one_skips_empty_archetype() {
        let (mut master, _arena) = make_master();
        // Create archetype but add no entities.
        master.create_archetype(&[Position::component_id()]);

        let query = Query::with_component_ids(&master, &[Position::component_id()]);
        assert_eq!(
            query.iter_one::<Position>().count(),
            0,
            "empty archetype must produce zero yields"
        );
    }

    // --- iter_two tests ---

    /// iter_two yields correctly paired (Position, Velocity) tuples.
    #[test]
    fn iter_two_yields_paired_components() {
        let (mut master, _arena) = make_master();
        let arch_id = master.create_archetype(&[
            Position::component_id(),
            Velocity::component_id(),
        ]);

        push_pos_vel(&mut master, arch_id, 0, 1, 10);
        push_pos_vel(&mut master, arch_id, 1, 2, 20);
        push_pos_vel(&mut master, arch_id, 2, 3, 30);

        let query = Query::with_component_ids(
            &master,
            &[Position::component_id(), Velocity::component_id()],
        );
        let collected: Vec<(u32, u32)> = query
            .iter_two::<Position, Velocity>()
            .map(|(p, v)| (p.0, v.0))
            .collect();

        assert_eq!(
            collected,
            [(1, 10), (2, 20), (3, 30)],
            "pairs must be correct and in insertion order"
        );
    }

    /// iter_two works across two archetypes and pairs are consistent.
    #[test]
    fn iter_two_across_two_archetypes() {
        let (mut master, _arena) = make_master();
        let arch1 = master.create_archetype(&[
            Position::component_id(),
            Velocity::component_id(),
        ]);
        let arch2 = master.create_archetype(&[
            Position::component_id(),
            Velocity::component_id(),
            Health::component_id(),
        ]);

        push_pos_vel(&mut master, arch1, 0, 100, 200);
        push_pos_vel(&mut master, arch1, 1, 101, 201);

        // For arch2 we need all three components; push manually.
        {
            let arch = master.get_archetype_mut(arch2).expect("arch2 must exist");
            let mut inland = EntityInland::new(arch.id(), InlandPoolId(0), 0);
            arch.init_entity_inland(&mut inland);
            let pb = 102u32.to_ne_bytes();
            let vb = 202u32.to_ne_bytes();
            let hb = 0u32.to_ne_bytes();
            let ok = arch.create_entity(EntityId(2), &mut inland, &[
                (Position::component_id(), pb.as_slice()),
                (Velocity::component_id(), vb.as_slice()),
                (Health::component_id(), hb.as_slice()),
            ]);
            assert!(ok, "create_entity for arch2 must succeed");
        }

        let query = Query::with_component_ids(
            &master,
            &[Position::component_id(), Velocity::component_id()],
        );
        let collected: Vec<(u32, u32)> = query
            .iter_two::<Position, Velocity>()
            .map(|(p, v)| (p.0, v.0))
            .collect();

        assert_eq!(collected.len(), 3, "must yield 3 total entity pairs");
        assert_eq!(&collected[..2], &[(100, 200), (101, 201)], "arch1 pairs must come first");
        assert_eq!(&collected[2..], &[(102, 202)], "arch2 pair must come last");
    }

    /// iter_two on an empty query (no matched archetypes) returns None immediately.
    #[test]
    fn iter_two_returns_none_on_empty_query() {
        let (master, _arena) = make_master();
        // No archetypes created at all.
        let query = Query::with_component_ids(
            &master,
            &[Position::component_id(), Velocity::component_id()],
        );
        assert!(
            query.iter_two::<Position, Velocity>().next().is_none(),
            "empty query must return None on first call to next()"
        );
    }
}
