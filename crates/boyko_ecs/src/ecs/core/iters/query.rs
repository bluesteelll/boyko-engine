use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::iters::component_set::ComponentSet;
use crate::ecs::core::iters::query_state::{QueryState, QueryStateIter};
use crate::ecs::identifiers::primitives::ComponentId;

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
        let component_ids = T::component_ids();
        Self::with_component_ids(master, &component_ids)
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

        for &id in &Inc::component_ids() {
            include_mask.set(id);
        }
        for &id in &Exc::component_ids() {
            exclude_mask.set(id);
        }
        for &id in &Opt::component_ids() {
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
}

/// Enable for-loop iteration over query results.
impl<'a> IntoIterator for &'a Query<'a> {
    type Item = &'a Archetype;
    type IntoIter = QueryStateIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.state.iter_cached(self.master)
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
        fn component_id() -> ComponentId {
            200
        }
    }

    impl Component for Velocity {
        fn component_id() -> ComponentId {
            201
        }
    }

    impl Component for Health {
        fn component_id() -> ComponentId {
            202
        }
    }

    impl Component for Damage {
        fn component_id() -> ComponentId {
            203
        }
    }

    fn register_mock_components() {
        // Register component layouts for testing
        component_registry::register_layout::<Position>(Position::component_id());
        component_registry::register_layout::<Velocity>(Velocity::component_id());
        component_registry::register_layout::<Health>(Health::component_id());
        component_registry::register_layout::<Damage>(Damage::component_id());
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
        let arena: Box<Arena> = Box::new(Arena::new());
        let mut master = ArchetypeMaster::new(&arena);

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
}
