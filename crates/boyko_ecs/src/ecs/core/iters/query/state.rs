//! `QueryDataState<D, F>` — per-system Query state cache.
//!
//! Wraps the Phase 5c `QueryState` archetype-match cache with the Phase 8b
//! per-system `D::State` / `F::State` triple and a post-filter pass that
//! enforces `Or<F>` semantics (and any future non-mask predicate).
//!
//! See §6 of `docs/PHASE-8B-QUERY-DSL-PLAN.md` for the full design.

use std::marker::PhantomData;

use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::iters::query::data::QueryData;
use crate::ecs::core::iters::query::filter::QueryFilter;
use crate::ecs::core::iters::query_state::QueryState;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;

/// Per-system state cache for a `Query<D, F>`.
///
/// Bundles three pieces:
/// * `archetype_state` — the Phase 5c [`QueryState`] archetype-match cache
///   (include/exclude/optional mask plus the `matched_ids` / dedup bitset
///   pair) populated by `update_archetypes`.
/// * `data_state` — `D::State`: per-`QueryData` cached metadata (resolved
///   [`ComponentId`](crate::ecs::identifiers::primitives::ComponentId)s).
/// * `filter_state` — `F::State`: per-`QueryFilter` cached metadata.
///
/// # INVARIANT (Phase 8b POST_FILTER) — M1
///
/// The pair `archetype_state.matched_ids: Vec<ArchetypeId>` and
/// `archetype_state.matched_archetypes: ArchetypeBitSet` MUST stay
/// synchronised: for every `id` in `matched_ids` the bit `id.0` MUST be set
/// in the bitset, and `bitset.popcount() == matched_ids.len()`.
///
/// Mutation paths that preserve the invariant:
/// * [`QueryState::update_archetypes`] — adds via `push_matched`, which sets
///   the bit before pushing the id.
/// * [`QueryState::remove_matched_at`] — `swap_remove`s the id and clears
///   the corresponding bit in the bitset.
///
/// [`Self::assert_dual_invariant`] is invoked at the tail of
/// [`Self::post_filter_matched`] in debug builds to surface any future
/// regression in either mutator.
pub struct QueryDataState<D: QueryData, F: QueryFilter> {
    pub(crate) archetype_state: QueryState,
    pub(crate) data_state: D::State,
    pub(crate) filter_state: F::State,
    _marker: PhantomData<fn() -> (D, F)>,
}

impl<D: QueryData, F: QueryFilter> QueryDataState<D, F> {
    /// Builds a fresh `QueryDataState` for `world`.
    ///
    /// Steps:
    /// 1. Allocate `D::State` and `F::State`.
    /// 2. Aggregate `include` from `D` and `F`, `exclude` from `F`. `optional`
    ///    is unused by Phase 8b — `With<C>` already contributes to `include`,
    ///    and `Or<F>` is enforced exclusively by the post-filter pass.
    /// 3. Construct the inner [`QueryState`] and sync it against the live
    ///    archetype set via `update_archetypes`.
    /// 4. Apply [`Self::post_filter_matched`] to drop any archetype that the
    ///    mask aggregation accepted but `D` / `F`'s `matches_component_set`
    ///    rejects (the `Or<F>` case).
    ///
    /// This is a cold path — called once per `(system, world)` pair at
    /// system registration. The cost is dominated by step 4 for `Or<F>`
    /// queries (see §6.4 of the Phase 8b plan).
    pub fn new(world: &mut EcsMaster) -> Self {
        let data_state = <D as QueryData>::init_state(world);
        let filter_state = <F as QueryFilter>::init_state(world);

        let mut include = ComponentMask::new();
        let mut exclude = ComponentMask::new();
        let optional = ComponentMask::new();

        <D as QueryData>::aggregate_include(&data_state, &mut include);
        <F as QueryFilter>::aggregate_include(&filter_state, &mut include);
        <F as QueryFilter>::aggregate_exclude(&filter_state, &mut exclude);

        let mut archetype_state = QueryState::new(include, exclude, optional);

        archetype_state.update_archetypes(world.archetype_master());
        Self::post_filter_matched(
            &mut archetype_state,
            &data_state,
            &filter_state,
            world.archetype_master(),
        );

        Self {
            archetype_state,
            data_state,
            filter_state,
            _marker: PhantomData,
        }
    }

    /// Trims `archetype_state.matched_ids` by re-applying
    /// `D::matches_component_set` AND `F::matches_component_set` to each id.
    ///
    /// Worst-case complexity: O(matched_ids.len() × (D-arity + F-arity)). For
    /// `Query<(), Or<F>>` (empty include + Or filter) the inner `QueryState`'s
    /// include mask is empty, so `update_archetypes` matches every live
    /// archetype; this method then scans them all and rejects the ones that
    /// fail the Or predicate. Cost is paid per `update` call (i.e. per
    /// generation bump), not per `iter()`.
    ///
    /// # Borrow-then-drop pattern
    ///
    /// The slice borrow ends at each loop iteration boundary so that
    /// `archetype_state.remove_matched_at` (which takes `&mut self`) can be
    /// called without conflict.
    fn post_filter_matched(
        archetype_state: &mut QueryState,
        data_state: &D::State,
        filter_state: &F::State,
        master: &ArchetypeMaster,
    ) {
        let mut idx = 0;
        loop {
            let ids = archetype_state.matched_ids();
            if idx >= ids.len() {
                break;
            }
            let id = ids[idx];
            let pass = master.get_archetype(id).is_some_and(|arch| {
                let mask = arch.component_mask();
                <D as QueryData>::matches_component_set(data_state, mask)
                    && <F as QueryFilter>::matches_component_set(filter_state, mask)
            });
            if pass {
                idx += 1;
            } else {
                archetype_state.remove_matched_at(idx);
                // idx unchanged — the swapped-in element at this slot still
                // needs to be checked.
            }
        }

        // M1: verify the dual-structure invariant after every mutation pass.
        #[cfg(debug_assertions)]
        Self::assert_dual_invariant(archetype_state);
    }

    /// M1 — debug-only invariant check: `matched_ids` and the
    /// `matched_archetypes` bitset are mutually consistent.
    ///
    /// Two conditions:
    /// * Every `id` in `matched_ids` has its `id.0` bit set in the bitset.
    /// * `bitset.popcount() as usize == matched_ids.len()` — bijection.
    #[cfg(debug_assertions)]
    fn assert_dual_invariant(archetype_state: &QueryState) {
        let ids = archetype_state.matched_ids();
        let bitset = archetype_state.matched_archetypes_bitset();
        for id in ids {
            debug_assert!(
                bitset.contains(id.0),
                "QS1 violation: id {} in matched_ids but bit not set in bitset",
                id.0,
            );
        }
        // `popcount()` returns `u32`; cast to `usize` so the comparison is
        // typed against `matched_ids.len()` without sign-extension surprises.
        debug_assert_eq!(
            bitset.popcount() as usize,
            ids.len(),
            "QS1 violation: bitset popcount {} != matched_ids.len() {}",
            bitset.popcount(),
            ids.len(),
        );
    }

    /// Brings the cache in sync with `master`'s current archetype set.
    ///
    /// Snapshot semantics: read the last-observed generations BEFORE calling
    /// `update_archetypes`, then compare against the master's post-update
    /// values. If either generation moved, the matched-id set may have grown
    /// or been rebuilt; re-run the post-filter to enforce the non-mask
    /// predicates (`Or<F>` and Phase 10+ filters).
    ///
    /// If both generations are unchanged, `update_archetypes` is a no-op and
    /// the post-filter is skipped — the warm-path `update` is therefore a
    /// pair of generation loads + comparisons.
    pub fn update(&mut self, master: &ArchetypeMaster) {
        let pre_gen = self.archetype_state.last_observed_archetype_generation();
        let pre_struct = self.archetype_state.last_observed_structural_generation();
        self.archetype_state.update_archetypes(master);
        if pre_gen != master.archetype_generation()
            || pre_struct != master.structural_generation()
        {
            Self::post_filter_matched(
                &mut self.archetype_state,
                &self.data_state,
                &self.filter_state,
                master,
            );
        }
    }

    /// Declares the read/write access surface of `D` and `F` to the active
    /// system's [`FilteredAccessSet`]. Surfaces intra-system aliasing
    /// conflicts as `boyko-B0002` panics (cold path).
    pub fn init_access(&self, access_set: &mut FilteredAccessSet) {
        <D as QueryData>::init_access(&self.data_state, access_set);
        <F as QueryFilter>::init_access(&self.filter_state, access_set);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry;
    use crate::ecs::core::iters::query::filter::{Or, With, Without};
    use crate::ecs::identifiers::primitives::ComponentId;

    // Component slots reserved for Phase 8b Step 6 unit tests. Chosen to
    // avoid collisions with the existing allocations elsewhere in the
    // crate:
    //   * 400-417 — archetype.rs
    //   * 200-203 — legacy_query.rs
    //   * 480-482 — archetype_bundle miri tests
    //   * 490-493 — query_state.rs
    //   * 495-497 — component_set.rs
    //   * 503-504 — query/data.rs
    //   * 510      — resource_registry CompThenRes
    // MAX_COMPONENTS = 512 caps valid ids at 511; the plan called for
    // 511-513 but 512+ are invalid, so we use 506-509 (free range).
    const COMP_A: ComponentId = ComponentId(506);
    const COMP_B: ComponentId = ComponentId(507);
    const COMP_C: ComponentId = ComponentId(508);
    const COMP_D: ComponentId = ComponentId(509);

    #[repr(C)]
    struct CompA(#[allow(dead_code)] u32);
    #[repr(C)]
    struct CompB(#[allow(dead_code)] u32);
    #[repr(C)]
    struct CompC(#[allow(dead_code)] u32);
    #[repr(C)]
    struct CompD(#[allow(dead_code)] u32);

    impl Component for CompA {
        fn component_id() -> ComponentId {
            COMP_A
        }
    }
    impl Component for CompB {
        fn component_id() -> ComponentId {
            COMP_B
        }
    }
    impl Component for CompC {
        fn component_id() -> ComponentId {
            COMP_C
        }
    }
    impl Component for CompD {
        fn component_id() -> ComponentId {
            COMP_D
        }
    }

    /// Idempotent registry priming for the four test components.
    fn register_test_components() {
        component_registry::register_layout::<CompA>(COMP_A.0);
        component_registry::register_layout::<CompB>(COMP_B.0);
        component_registry::register_layout::<CompC>(COMP_C.0);
        component_registry::register_layout::<CompD>(COMP_D.0);
    }

    /// `QueryDataState::new` populates the inner archetype_state from the
    /// world's live archetypes.
    #[test]
    fn new_populates_archetype_state() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        // Two archetypes — one matches `&CompA`, the other does not.
        let matching = ecs.create_archetype(&[COMP_A]);
        let _non_matching = ecs.create_archetype(&[COMP_B]);

        let state = QueryDataState::<&CompA, ()>::new(&mut ecs);

        assert_eq!(
            state.archetype_state.matched_ids(),
            &[matching],
            "new must match exactly the CompA archetype",
        );
    }

    /// Two consecutive `update` calls without intervening archetype churn:
    /// the second observes no generation change and triggers no
    /// post_filter pass. We verify this indirectly via the dual-invariant
    /// (no panic) and a stable matched-id snapshot.
    #[test]
    fn update_short_circuits_warm() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        ecs.create_archetype(&[COMP_A]);
        ecs.create_archetype(&[COMP_A, COMP_B]);

        let mut state = QueryDataState::<&CompA, ()>::new(&mut ecs);
        let snapshot: Vec<_> = state.archetype_state.matched_ids().to_vec();

        // First update: cache is already in sync — no churn, no rebuild.
        state.update(ecs.archetype_master());
        assert_eq!(state.archetype_state.matched_ids(), snapshot.as_slice());

        // Second update: still no churn.
        state.update(ecs.archetype_master());
        assert_eq!(state.archetype_state.matched_ids(), snapshot.as_slice());
    }

    /// `Query<(), Or<(With<A>, With<B>)>>` with no archetype matching either
    /// `A` or `B`: post_filter drops every entry from the initial
    /// "include-mask is empty so match all" set, leaving `matched_ids`
    /// empty.
    #[test]
    fn post_filter_drops_or_misses() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        // Two archetypes — neither contains CompA or CompB.
        ecs.create_archetype(&[COMP_C]);
        ecs.create_archetype(&[COMP_D]);

        let state =
            QueryDataState::<(), Or<(With<CompA>, With<CompB>)>>::new(&mut ecs);

        assert!(
            state.archetype_state.matched_ids().is_empty(),
            "Or<(With<A>, With<B>)> against {{C, D}} archetypes must yield empty matches; got {:?}",
            state.archetype_state.matched_ids(),
        );

        // Sanity: adding a CompA archetype makes the Or filter pick it up.
        let with_a = ecs.create_archetype(&[COMP_A]);
        let mut state = state;
        state.update(ecs.archetype_master());
        assert_eq!(
            state.archetype_state.matched_ids(),
            &[with_a],
            "Or filter must accept a CompA-bearing archetype after update",
        );
    }

    /// `init_access` forwards to both `D::init_access` and `F::init_access`.
    /// We instantiate a `QueryDataState<&CompA, With<CompB>>` and check the
    /// resulting `FilteredAccessSet` carries the read bit for both A and B.
    #[test]
    fn init_access_forwards_to_data_and_filter() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let state = QueryDataState::<&CompA, With<CompB>>::new(&mut ecs);

        let mut access_set = FilteredAccessSet::new();
        state.init_access(&mut access_set);

        // A sibling write on either component must conflict, proving the
        // read bits landed.
        let combined = access_set.combined();
        let mut writer_a = crate::ecs::core::system::access::Access::new();
        writer_a.add_component_write(COMP_A);
        assert!(
            combined.conflicts_with(&writer_a),
            "init_access must declare a read of CompA (D side)",
        );
        let mut writer_b = crate::ecs::core::system::access::Access::new();
        writer_b.add_component_write(COMP_B);
        assert!(
            combined.conflicts_with(&writer_b),
            "init_access must declare a read of CompB (F side)",
        );
    }

    /// After `new`, the dual invariant holds: every id in `matched_ids` is
    /// set in the bitset and `popcount == len`.
    #[test]
    fn assert_dual_invariant_passes_on_consistent_state() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        ecs.create_archetype(&[COMP_A]);
        ecs.create_archetype(&[COMP_A, COMP_B]);
        ecs.create_archetype(&[COMP_A, COMP_C]);

        let state = QueryDataState::<&CompA, ()>::new(&mut ecs);

        // No panic + manually re-check the conditions.
        QueryDataState::<&CompA, ()>::assert_dual_invariant(&state.archetype_state);
        let bitset = state.archetype_state.matched_archetypes_bitset();
        let ids = state.archetype_state.matched_ids();
        assert_eq!(ids.len(), 3, "three CompA archetypes matched");
        assert_eq!(
            bitset.popcount() as usize,
            ids.len(),
            "popcount must match matched_ids len",
        );
        for id in ids {
            assert!(
                bitset.contains(id.0),
                "id {} must be set in the dedup bitset",
                id.0,
            );
        }
    }

    /// Synthetic violation: push an extra id into `matched_ids_mut` without
    /// updating the bitset, then expect `assert_dual_invariant` to fire in
    /// debug builds.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "QS1 violation")]
    fn assert_dual_invariant_detects_violation() {
        register_test_components();
        let mut ecs = EcsMaster::new();
        let real = ecs.create_archetype(&[COMP_A]);

        let mut state = QueryDataState::<&CompA, ()>::new(&mut ecs);
        // Corrupt the invariant by pushing a duplicate id directly into the
        // matched_ids vector. The bitset bit for `real` is already set, so
        // this creates a 2-vs-1 popcount mismatch that
        // `assert_dual_invariant` MUST detect.
        state.archetype_state.matched_ids_mut().push(real);

        QueryDataState::<&CompA, ()>::assert_dual_invariant(&state.archetype_state);
    }
}
