use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
use crate::ecs::core::archetype::generation::ArchetypeGeneration;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::iters::archetype_bit_set::ArchetypeBitSet;
use crate::ecs::core::iters::component_set::ComponentSet;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

/// Persistent archetype-match cache for hot-path query iteration.
///
/// Unlike `Query<'a>` (which rebuilds its archetype list on every construction),
/// `QueryState` is long-lived and caches the result across frames. On the warm
/// path — when no new archetypes have been created — `iter()` costs one pointer
/// load + comparison. The delta update path classifies only newly minted archetypes.
///
/// # Layout rationale
/// `#[repr(C, align(64))]` places the hot fields (`generation`, `matched_ids`)
/// in cache line 0. The three filter masks (192 B) and the dedup bitset (128 B)
/// occupy later cache lines and are only touched on cache misses.
///
/// # Generation and `clear()`
/// `QueryState` stores the generation value at the time of the last sync. If
/// `ArchetypeMaster::clear()` is called while a `QueryState` is alive, its
/// cached IDs refer to recycled slots and must be discarded. Debug builds assert
/// this via `debug_assert!` in `iter()` and `update_archetypes()`.
/// Callers must call `reset()` or drop the `QueryState` before `clear()`.
#[repr(C, align(64))]
pub struct QueryState {
    // Cache line 0 (hot): read on every iter() / update_archetypes() early-exit check.
    generation: ArchetypeGeneration,
    matched_ids: Vec<ArchetypeId>,
    // Lines 1-3 (cold-after-warmup): 3 × 64 B filter masks.
    include: ComponentMask,
    exclude: ComponentMask,
    optional: ComponentMask,
    // Lines 4-5 (coldest): dedup bitset. Touched only when delta > 0.
    matched_archetypes: ArchetypeBitSet,
}

impl QueryState {
    /// Creates a new `QueryState` with the given filter masks.
    ///
    /// The state starts at `FIRST` generation — a call to `update_archetypes`
    /// or `iter` is required before any archetypes are matched.
    pub fn new(include: ComponentMask, exclude: ComponentMask, optional: ComponentMask) -> Self {
        Self {
            generation: ArchetypeGeneration::FIRST,
            matched_ids: Vec::with_capacity(16),
            include,
            exclude,
            optional,
            matched_archetypes: ArchetypeBitSet::new(),
        }
    }

    /// Creates a state that matches archetypes containing all of the given component IDs.
    pub fn with_component_ids(includes: &[ComponentId]) -> Self {
        let mut include = ComponentMask::new();
        for &id in includes {
            include.set(id);
        }
        Self::new(include, ComponentMask::new(), ComponentMask::new())
    }

    /// Creates a type-safe state matching archetypes for the given `ComponentSet`.
    pub fn with<T: ComponentSet>() -> Self {
        Self::with_component_ids(T::component_ids())
    }

    /// Returns an iterator over matched archetypes, updating the cache if needed.
    ///
    /// Warm path (generation unchanged): one load + comparison, then slice walk.
    /// Cold path (new archetypes exist): `update_archetypes` classifies the delta.
    ///
    /// # `clear()` interaction
    /// If `master.clear()` was called since the last `iter()` / `update_archetypes()`
    /// invocation, this state's cache is stale and must not be used. Call `reset()`
    /// first, or drop this state and reconstruct. Debug builds catch the violation
    /// via `debug_assert!`; release builds silently use the stale cache.
    pub fn iter<'a>(&'a mut self, master: &'a ArchetypeMaster) -> QueryStateIter<'a> {
        debug_assert!(
            self.generation <= master.archetype_generation(),
            "QueryState.generation ({:?}) > master.archetype_generation() ({:?}); \
             likely cause: master.clear() was called while this QueryState was alive",
            self.generation,
            master.archetype_generation(),
        );
        if self.generation != master.archetype_generation() {
            self.update_archetypes(master);
        }
        // `iter_cached` is valid because update_archetypes() just synced generation.
        self.iter_cached(master)
    }

    /// Classifies any archetypes created since the last sync against the filter.
    ///
    /// This is the delta update path. IDs already in `matched_archetypes` are
    /// skipped in O(1) via the dedup bitset; only truly new IDs are tested.
    pub fn update_archetypes(&mut self, master: &ArchetypeMaster) {
        let current = master.archetype_generation();
        debug_assert!(
            self.generation <= current,
            "QueryState.generation ({:?}) > master.archetype_generation() ({:?}); \
             master.clear() was called without resetting this QueryState",
            self.generation,
            current,
        );
        if self.generation == current {
            return;
        }

        // Iterate all archetype IDs from 1..current.get() and skip already-seen ones.
        // A future optimization (master.archetypes_since(n) slice) would eliminate the
        // full sweep, but requires bundle index tracking. The dedup bitset makes the
        // per-seen-id work O(1), so total cost is O(new ids).
        for id in 1..current.get() {
            if !self.matched_archetypes.contains(id)
                && let Some(arch) = master.get_archetype(ArchetypeId(id))
            {
                let mask = arch.component_mask();
                if self.matches(mask) {
                    self.matched_archetypes.insert(id);
                    self.matched_ids.push(ArchetypeId(id));
                }
                // Unmatched IDs are not inserted into matched_archetypes.
                // Archetype component sets are immutable post-creation, so
                // the same filter applied again will still not match. The loop
                // bound is O(current.get()) per call, bounded by MAX_ARCHETYPES.
            }
        }
        self.generation = current;
    }

    /// Tests whether a component mask satisfies the query filters.
    ///
    /// - include: every bit in `self.include` must be set in `mask`.
    /// - exclude: no bit in `self.exclude` may be set in `mask`.
    /// - optional: if non-empty, at least one bit must overlap with `mask`.
    #[inline]
    fn matches(&self, mask: &ComponentMask) -> bool {
        // `include.is_subset(mask)` — every include bit is present in mask.
        let include_ok = self.include.is_subset(mask);
        // `mask.intersects(exclude)` — mask shares a bit with exclude → reject.
        let exclude_ok = !mask.intersects(&self.exclude);
        // optional empty → no additional constraint.
        let optional_ok = self.optional.is_empty() || mask.intersects(&self.optional);
        include_ok && exclude_ok && optional_ok
    }

    /// Returns the number of matched archetypes.
    #[inline]
    pub fn len(&self) -> usize {
        self.matched_ids.len()
    }

    /// Returns true if no archetypes are matched.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.matched_ids.is_empty()
    }

    /// Returns a slice of all matched archetype IDs.
    #[inline]
    pub fn matched_ids(&self) -> &[ArchetypeId] {
        &self.matched_ids
    }

    /// Resets the cache, returning the state to its initial condition.
    ///
    /// After `reset()`, the next `iter()` or `update_archetypes()` will
    /// re-scan all archetypes from scratch.
    pub fn reset(&mut self) {
        self.matched_ids.clear();
        self.matched_archetypes.clear_all();
        self.generation = ArchetypeGeneration::FIRST;
    }

    /// Iterates the cached matched IDs without re-checking the generation.
    ///
    /// Requires that `update_archetypes` has already been called and
    /// `self.generation == master.archetype_generation()`.
    #[inline]
    pub(crate) fn iter_cached<'a>(&'a self, master: &'a ArchetypeMaster) -> QueryStateIter<'a> {
        QueryStateIter {
            master,
            ids: self.matched_ids.iter(),
        }
    }

    /// Inserts an archetype ID into both the dedup bitset and the matched-IDs
    /// list, if it has not already been recorded.
    ///
    /// This is the single authoritative mutation path for adding a matched
    /// archetype; both internal structures are always updated together,
    /// preventing silent desync if new internal state is added in the future.
    #[inline]
    pub(crate) fn push_matched(&mut self, id: ArchetypeId) {
        if !self.matched_archetypes.contains(id.0) {
            self.matched_archetypes.insert(id.0);
            self.matched_ids.push(id);
        }
    }

    /// Marks this state as synced with `master`'s current generation.
    ///
    /// Call once after manually pre-populating the cache via `push_matched`
    /// (e.g., in `Query::from_archetypes` or `Query::with_exact_mask`) to
    /// prevent a redundant `update_archetypes` sweep on the next `iter()`.
    #[inline]
    pub(crate) fn mark_synced(&mut self, master: &ArchetypeMaster) {
        self.generation = master.archetype_generation();
    }
}

/// Iterator over matched archetypes produced by `QueryState::iter`.
///
/// Skips stale-removed IDs: if an archetype was removed from the master
/// after this `QueryState` was last synced, `master.get_archetype(id)`
/// returns `None` and the ID is transparently skipped.
pub struct QueryStateIter<'a> {
    master: &'a ArchetypeMaster,
    ids: std::slice::Iter<'a, ArchetypeId>,
}

impl<'a> Iterator for QueryStateIter<'a> {
    type Item = &'a Archetype;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        // find_map skips stale-removed IDs (get_archetype returns None for removed IDs).
        self.ids.by_ref().find_map(|&id| self.master.get_archetype(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::memory::arena::Arena;

    // Component IDs 490-493: reserved for query_state tests.
    // Note: IDs 700-799 were originally specified in the plan roadmap note, but
    // MAX_COMPONENTS = 512 caps valid IDs at 511. Range 490-493 is confirmed free
    // (450-465 = component_registry; 470-471 = query_iter bench; 480-481 = swap_remove).
    #[repr(C)]
    struct Pos(u32);
    #[repr(C)]
    struct Vel(u32);
    #[repr(C)]
    struct Health(u32);
    #[repr(C)]
    struct Damage(u32);

    impl Component for Pos {
        fn component_id() -> ComponentId { ComponentId(490) }
    }
    impl Component for Vel {
        fn component_id() -> ComponentId { ComponentId(491) }
    }
    impl Component for Health {
        fn component_id() -> ComponentId { ComponentId(492) }
    }
    impl Component for Damage {
        fn component_id() -> ComponentId { ComponentId(493) }
    }

    fn register_components() {
        component_registry::register_layout::<Pos>(Pos::component_id().0);
        component_registry::register_layout::<Vel>(Vel::component_id().0);
        component_registry::register_layout::<Health>(Health::component_id().0);
        component_registry::register_layout::<Damage>(Damage::component_id().0);
    }

    fn setup() -> (ArchetypeMaster, Box<Arena>) {
        register_components();
        let arena = Box::new(Arena::new());
        // Mint the raw arena pointer from the Box's inner representation without
        // creating a `&Arena` reference. See `EcsMaster::new` for the full
        // rationale (Phase 3a Miri retag fix).
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

    // --- test 700: empty state yields nothing ---

    #[test]
    fn t700_empty_state_iter_yields_nothing() {
        let (master, _arena) = setup();
        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);
        let count = state.iter(&master).count();
        assert_eq!(count, 0);
    }

    // --- test 701: single archetype matched after update ---

    #[test]
    fn t701_single_archetype_match_after_update() {
        let (mut master, _arena) = setup();
        master.create_archetype(&[Pos::component_id(), Vel::component_id()]);

        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);
        state.update_archetypes(&master);

        assert_eq!(state.len(), 1);
        let arch = state.iter(&master).next().expect("one archetype expected");
        assert!(arch.has_component_id(Pos::component_id()));
    }

    // --- test 702: update_archetypes is idempotent ---

    #[test]
    fn t702_update_idempotent() {
        let (mut master, _arena) = setup();
        master.create_archetype(&[Pos::component_id()]);

        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);
        state.update_archetypes(&master);
        let len_first = state.len();

        state.update_archetypes(&master);
        assert_eq!(state.len(), len_first, "second update must be a no-op");
    }

    // --- test 703: delta update classifies only new archetypes ---

    #[test]
    fn t703_delta_update_only_classifies_new_archetypes() {
        let (mut master, _arena) = setup();
        master.create_archetype(&[Pos::component_id(), Vel::component_id()]);
        master.create_archetype(&[Pos::component_id(), Health::component_id()]);

        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);
        state.update_archetypes(&master);
        assert_eq!(state.len(), 2, "both Pos+Vel and Pos+Health should match");

        // Add a third archetype and delta-update
        master.create_archetype(&[Pos::component_id(), Damage::component_id()]);
        state.update_archetypes(&master);
        assert_eq!(state.len(), 3, "third archetype must be picked up on delta");
    }

    // --- test 704: include/exclude/optional filter semantics ---

    #[test]
    fn t704_include_exclude_optional_combinations() {
        let (mut master, _arena) = setup();
        // 5 archetypes matching query.rs::test_complex_filtering setup
        master.create_archetype(&[Pos::component_id()]);
        master.create_archetype(&[Pos::component_id(), Vel::component_id()]);
        master.create_archetype(&[Health::component_id()]);
        master.create_archetype(&[Pos::component_id(), Health::component_id()]);
        master.create_archetype(&[Pos::component_id(), Vel::component_id(), Health::component_id()]);

        let mut include = ComponentMask::new();
        include.set(Pos::component_id());

        let mut exclude = ComponentMask::new();
        exclude.set(Damage::component_id());

        let mut optional = ComponentMask::new();
        optional.set(Vel::component_id());
        optional.set(Health::component_id());

        let mut state = QueryState::new(include, exclude, optional);
        state.update_archetypes(&master);

        // Expected: [Pos+Vel], [Pos+Health], [Pos+Vel+Health] → 3
        // [Pos] alone fails optional (no Vel or Health)
        // [Health] alone fails include (no Pos)
        assert_eq!(
            state.len(),
            3,
            "filter must match same 3 archetypes as Query::test_complex_filtering"
        );
        for arch in state.iter(&master) {
            assert!(arch.has_component_id(Pos::component_id()), "must have Pos");
            assert!(!arch.has_component_id(Damage::component_id()), "must not have Damage");
            assert!(
                arch.has_component_id(Vel::component_id())
                    || arch.has_component_id(Health::component_id()),
                "must have Vel or Health"
            );
        }
    }

    // --- test 705: dedup bitset and matched_ids consistency ---

    #[test]
    fn t705_dual_structure_consistency() {
        let (mut master, _arena) = setup();
        master.create_archetype(&[Pos::component_id()]);
        master.create_archetype(&[Vel::component_id()]);
        master.create_archetype(&[Pos::component_id(), Vel::component_id()]);
        master.create_archetype(&[Health::component_id()]);
        master.create_archetype(&[Pos::component_id(), Health::component_id()]);

        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);
        state.update_archetypes(&master);

        // For every ID in matched_ids, the bitset must say contains=true
        for &id in state.matched_ids() {
            assert!(
                state.matched_archetypes.contains(id.0),
                "id {} in matched_ids but not in bitset",
                id
            );
        }

        // For every ID NOT in matched_ids, the bitset should NOT say contains=true
        // (IDs that exist but don't match Pos).
        let vel_only_id = master
            .find_archetypes_with_components(&[Vel::component_id()])
            .into_iter()
            .find(|&id| {
                let arch = master.get_archetype(id).unwrap();
                !arch.has_component_id(Pos::component_id())
            });
        if let Some(id) = vel_only_id {
            assert!(
                !state.matched_ids().contains(&id),
                "vel-only archetype must not be in matched_ids"
            );
        }
    }

    // --- test 706: reset clears the cache ---

    #[test]
    fn t706_reset_clears_cache() {
        let (mut master, _arena) = setup();
        master.create_archetype(&[Pos::component_id()]);
        master.create_archetype(&[Pos::component_id(), Vel::component_id()]);

        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);
        state.update_archetypes(&master);
        assert_eq!(state.len(), 2, "two archetypes before reset");

        state.reset();
        assert_eq!(state.len(), 0, "len must be 0 after reset");
        assert!(state.is_empty(), "is_empty must be true after reset");

        // After reset the bitset must also be clean: re-run update and verify no dups
        state.update_archetypes(&master);
        assert_eq!(state.len(), 2, "must re-match after reset+update");
    }

    // --- test 707: stale id after remove is skipped ---

    #[test]
    fn t707_stale_id_after_remove_is_skipped() {
        let (mut master, _arena) = setup();
        master.create_archetype(&[Pos::component_id()]);
        let id_to_remove =
            master.create_archetype(&[Pos::component_id(), Vel::component_id()]);
        master.create_archetype(&[Pos::component_id(), Health::component_id()]);

        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);
        state.update_archetypes(&master);
        assert_eq!(state.len(), 3);

        // Remove one archetype from master
        master.remove_archetype(id_to_remove);

        // iter() must skip the now-missing id (get_archetype returns None)
        let count = state.iter(&master).count();
        assert_eq!(count, 2, "stale removed id must be skipped during iteration");
    }
}
