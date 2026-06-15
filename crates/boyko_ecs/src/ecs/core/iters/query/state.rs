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
use crate::ecs::core::iters::query::term_list::TermScratch;
use crate::ecs::core::iters::query_state::QueryState;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::identifiers::primitives::ComponentId;

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
    /// Phase 22.1 Area A: cold 16-byte tail (two `AtomicPtr`) memoising the
    /// term-prefiltered id list per epoch. NEVER touched by the no-terms
    /// paths — term-bearing driver entries resolve once through it via
    /// [`TermScratch::resolve_term_filtered`], reclaiming the retired list at
    /// the slot-exclusive mint funnels. Auto `Send + Sync`; soundness carried
    /// by the protocol P1–P4 (see [`term_list`](super::term_list)).
    pub(crate) term_scratch: TermScratch,
    /// EnableTag O2 (amendment A4.2) — last-observed `enable_generation`.
    ///
    /// Present in EVERY monomorphization but READ/WRITTEN only on the
    /// `CONTAINS_ENABLE_TERM` (enable-bearing) paths — both the positive-term
    /// cull and the candidate-seeded global scan share this one slot (the two
    /// shapes are const-disjoint write sites; A4.2 honest field-reuse). A
    /// column-alloc first-toggle into a new archetype bumps
    /// `ArchetypeMaster::enable_generation`, and the next `update` re-culls /
    /// re-seeds. The 8-byte tail is never touched by non-enable queries (the
    /// 0%-gate: `update`'s warm path is gated by `if const { has_enable_term }`,
    /// so this load is not even emitted into a non-enable monomorphization).
    last_observed_enable_generation: u64,
    _marker: PhantomData<fn() -> (D, F)>,
}

impl<D: QueryData, F: QueryFilter> QueryDataState<D, F> {
    /// EnableTag — `true` iff `F` contributes an `Enabled`/`Disabled` term.
    ///
    /// Gates the entire enable machinery (the O2 `enable_generation` check, the
    /// cull, the candidate seed). For a non-enable `(D, F)` this is `false`,
    /// so every `if const { Self::HAS_ENABLE_TERM }` branch const-folds OUT —
    /// the warm-path `update` is byte-identical to pre-EnableTag (the 0%-gate).
    const HAS_ENABLE_TERM: bool = F::CONTAINS_ENABLE_TERM;

    /// EnableTag amendment A3.2 — `true` iff `(D, F)` is the candidate-seedable
    /// SOLE single enable shape: a single `Enabled<A>` / `Disabled<A>` leaf
    /// (`IS_SOLE_SINGLE_ENABLE`) with NO positive bound (no data component, no
    /// `With`). For this shape `new`/`update` seed the matched set from the
    /// bounded `EnablePresence[A]` candidate snapshot instead of the
    /// `1..gen` sweep. `false` for every other shape (positive-term enable,
    /// non-enable, or a rejected enable-tuple — which never compiles).
    const IS_CANDIDATE_SEEDED: bool =
        F::IS_SOLE_SINGLE_ENABLE && !D::HAS_DATA_COMPONENT && !F::HAS_POSITIVE_ARCHETYPAL;

    /// The shape-assert body (amendment A3 / Step 7a). Called from BOTH the
    /// codegen-time trigger (`new`'s inline `const {}` block — fires under
    /// `build` / `test` / any codegen) AND the check-time trigger
    /// ([`Self::assert_query_shape`] in a `const ITEM` context — fires under a
    /// metadata-only `cargo check`, the mode `trybuild` `compile_fail` runs).
    /// Two triggers are required: a generic-fn `const {}` block is evaluated
    /// only at codegen, while `trybuild` checks — neither alone covers every
    /// build path (the Phase-12.5 "const must be in a forcing context" lesson,
    /// verified against this toolchain).
    ///
    /// * `_C2` (NARROWED — amendment A3.2): an `Enabled`/`Disabled` term is
    ///   admitted only when bounded by a positive term (a data component or
    ///   `With<_>`) OR it is a SOLE single leaf (`IS_SOLE_SINGLE_ENABLE` —
    ///   candidate-seeded). An enable-tuple with no positive term is rejected
    ///   (no multi-tag resolver in v1).
    /// * `_C3` (KEPT verbatim — amendment A3.4): an enable term cannot be
    ///   combined with `Added`/`Changed` in one query.
    const fn eval_shape_asserts() {
        // _C2 (narrowed): positive-bounded OR sole-single-leaf.
        assert!(
            !F::CONTAINS_ENABLE_TERM
                || D::HAS_DATA_COMPONENT
                || F::HAS_POSITIVE_ARCHETYPAL
                || F::IS_SOLE_SINGLE_ENABLE,
            "`Enabled<T>`/`Disabled<T>` with no positive term is supported ONLY as a \
             SINGLE sole term (`Query<(), Enabled<A>>`). A tuple of enable terms without \
             a positive archetypal term is not bounded in v1 — add `With<_>` / a data \
             component, or split into separate single-term queries."
        );
        // _C3 (kept): enable XOR change-detection in one query.
        assert!(
            !(F::CONTAINS_ENABLE_TERM && F::CONTAINS_CHANGE_DETECTION),
            "an `Enabled<T>`/`Disabled<T>` term cannot be combined with `Added`/`Changed` \
             in one query: point lookups apply the enable bit but not change-detection, \
             which would silently mislead. Split into two queries."
        );
    }

    /// The check-time shape-assert trigger (amendment A3 / Step 7a). A `pub`
    /// `const fn` so an external `compile_fail` test can force the asserts in a
    /// `const ITEM` context:
    ///
    /// ```ignore
    /// const _: () = QueryDataState::<&P, (Changed<P>, Enabled<A>)>::assert_query_shape();
    /// ```
    ///
    /// A `const fn` call inside a `const _: () = ...` item is eagerly
    /// const-evaluated even under a metadata-only `cargo check` — unlike a
    /// generic-fn `const {}` block, which fires only at codegen. This is the only
    /// trigger that catches a misuse under `trybuild`'s `compile_fail` (which
    /// runs `cargo check`).
    pub const fn assert_query_shape() {
        Self::eval_shape_asserts();
    }

    /// Resolves the tag id of a SOLE single enable term from `filter_state`
    /// (amendment A2.1). Called ONLY under `if const { Self::IS_CANDIDATE_SEEDED }`,
    /// so for any non-sole-enable `F` the trait's `unreachable!()` backstop is
    /// never emitted into a reachable path.
    #[inline]
    fn sole_enable_tag(&self) -> ComponentId {
        <F as QueryFilter>::sole_enable_tag_id(&self.filter_state)
    }

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
        // Force-evaluate the per-(D, F) shape asserts (amendment A3 / Step 7a).
        // This inline `const {}` block fires at CODEGEN of this monomorphization
        // (build / test). The `check`-time trigger for `trybuild` compile_fail
        // is the public `Self::assert_query_shape()` in a `const ITEM` context.
        const { Self::eval_shape_asserts() };

        let data_state = <D as QueryData>::init_state(world);
        let filter_state = <F as QueryFilter>::init_state(world);

        let mut include = ComponentMask::new();
        let mut exclude = ComponentMask::new();
        let optional = ComponentMask::new();

        <D as QueryData>::aggregate_include(&data_state, &mut include);
        <F as QueryFilter>::aggregate_include(&filter_state, &mut include);
        <F as QueryFilter>::aggregate_exclude(&filter_state, &mut exclude);

        let mut archetype_state = QueryState::new(include, exclude, optional);

        // EnableTag amendment A2.2 — the candidate-seeded branch. `IS_CANDIDATE_SEEDED`
        // is a const, so for every non-sole-enable `(D, F)` the whole `if const`
        // collapses to the `else` arm at monomorphization — the seed code,
        // `snapshot_present`, and `sole_enable_tag_id` are NEVER emitted there.
        // The non-enable path is byte-identical to pre-EnableTag (the 0%-gate).
        let mut last_observed_enable_generation = 0u64;
        if const { Self::IS_CANDIDATE_SEEDED } {
            // Bounded candidate snapshot (popcount-walked by seed_from_candidates).
            let tag = <F as QueryFilter>::sole_enable_tag_id(&filter_state);
            let candidates = world.archetype_master().enable_presence().snapshot_present(tag);
            archetype_state.seed_from_candidates(&candidates, world.archetype_master());
            last_observed_enable_generation = world.archetype_master().enable_generation();
            // No `post_filter_matched`: the include mask is empty and the
            // candidate bitset IS the membership predicate (nothing to trim).
        } else {
            archetype_state.update_archetypes(world.archetype_master());
            Self::post_filter_matched(
                &mut archetype_state,
                &data_state,
                &filter_state,
                world.archetype_master(),
            );
            // Positive-term enable shapes seed the O2 generation here too.
            if const { Self::HAS_ENABLE_TERM } {
                Self::cull_enable_archetypes(&mut archetype_state, &filter_state, world.archetype_master());
                last_observed_enable_generation = world.archetype_master().enable_generation();
            }
        }

        Self {
            archetype_state,
            data_state,
            filter_state,
            term_scratch: TermScratch::new(),
            last_observed_enable_generation,
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
            // Phase 22 D4: pre-terms by design — this is QS1 cache
            // maintenance over the SHARED archetype-match cache; dynamic-tag
            // terms are per-view and must never affect (or read through)
            // the shared cache.
            let ids = archetype_state.matched_ids_pre_terms();
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
        // Phase 22 D4: pre-terms by design — QS1 dual-structure verification
        // runs against the shared term-agnostic cache.
        let ids = archetype_state.matched_ids_pre_terms();
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
        // EnableTag candidate-seeded global scan (amendment A4.2). Const-folded:
        // emitted ONLY for the sole-single-enable shape; every other `(D, F)`
        // skips straight to the path below.
        if const { Self::IS_CANDIDATE_SEEDED } {
            let cur_enable_gen = master.enable_generation();
            let pre_struct = self.archetype_state.last_observed_structural_generation();
            if self.last_observed_enable_generation != cur_enable_gen
                || pre_struct != master.structural_generation()
            {
                // Re-snapshot + re-seed. `seed_from_candidates` does its own
                // structural-mismatch full clear (popcount-bounded — A1.5).
                let tag = self.sole_enable_tag();
                let candidates = master.enable_presence().snapshot_present(tag);
                self.archetype_state.seed_from_candidates(&candidates, master);
                self.last_observed_enable_generation = cur_enable_gen;
            }
            return;
        }

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
            // Positive-term enable shapes re-cull after a structural rebuild.
            if const { Self::HAS_ENABLE_TERM } {
                Self::cull_enable_archetypes(&mut self.archetype_state, &self.filter_state, master);
                self.last_observed_enable_generation = master.enable_generation();
            }
        } else if const { Self::HAS_ENABLE_TERM } {
            // No archetype churn, but a column-alloc first-toggle may have
            // bumped enable_generation (a new archetype gained the tag's
            // column). Re-cull over the bounded matched set if so. Gated by
            // `has_enable_term` ⇒ the load is not emitted for non-enable queries
            // (the 0%-gate warm path stays byte-identical above).
            let cur_enable_gen = master.enable_generation();
            if self.last_observed_enable_generation != cur_enable_gen {
                // A column-alloc cannot retroactively change which archetypes
                // satisfy the positive term, so the matched set already contains
                // every candidate; re-running the cull narrows it to the
                // now-present ones. (Bounded over matched_ids — C2.)
                Self::cull_enable_archetypes(&mut self.archetype_state, &self.filter_state, master);
                self.last_observed_enable_generation = cur_enable_gen;
            }
        }
    }

    /// EnableTag positive-term presence cull (plan D2 / Step 7a) — narrows the
    /// already-bounded matched set to archetypes present for the enable tag.
    ///
    /// Runs ONLY for `CONTAINS_ENABLE_TERM` shapes, over `matched_ids` (bounded
    /// by the required positive term — C2), NEVER an empty-include full scan.
    ///
    /// # Current scope (FLAGGED — see report)
    ///
    /// A correct coarse cull must (a) resolve the enable tag id and (b) know the
    /// term's polarity (`Enabled` culls no-column archetypes; `Disabled` must
    /// NOT — amendment A1.1). The v1 `QueryFilter` trait exposes neither a
    /// tuple-enable tag resolver nor a polarity/`cull_keeps_archetype` hook
    /// (`sole_enable_tag_id` resolves only a SOLE single leaf). Applying a
    /// present-cull without the polarity discriminator would wrongly drop the
    /// no-column archetypes a positive-term `Disabled<A>` must visit.
    ///
    /// Therefore this pass is a CORRECTNESS-PRESERVING NO-OP in v1: per-row
    /// `filter_fetch` (Step 7) already delivers exact results; the cull is a
    /// pure optimization. It is structurally wired (the `update` re-cull
    /// trigger, the O2 generation stamp) so a Step-7 follow-up that adds a
    /// `QueryFilter::enable_cull_keeps_archetype(state, presence, arch)` hook
    /// (default: keep all) can drop in the per-archetype verdict here with no
    /// further changes to `new` / `update`.
    #[inline]
    fn cull_enable_archetypes(
        _archetype_state: &mut QueryState,
        _filter_state: &F::State,
        _master: &ArchetypeMaster,
    ) {
        // NO-OP in v1 — see method doc (the polarity/tag resolver gap). The
        // wiring (this call site, the O2 generation stamp) is the seam for the
        // Step-7 follow-up hook.
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
    use crate::ecs::core::iters::query::filter::{Or, With};
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
            state.archetype_state.matched_ids_pre_terms(),
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
        let snapshot: Vec<_> = state.archetype_state.matched_ids_pre_terms().to_vec();

        // First update: cache is already in sync — no churn, no rebuild.
        state.update(ecs.archetype_master());
        assert_eq!(state.archetype_state.matched_ids_pre_terms(), snapshot.as_slice());

        // Second update: still no churn.
        state.update(ecs.archetype_master());
        assert_eq!(state.archetype_state.matched_ids_pre_terms(), snapshot.as_slice());
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
            state.archetype_state.matched_ids_pre_terms().is_empty(),
            "Or<(With<A>, With<B>)> against {{C, D}} archetypes must yield empty matches; got {:?}",
            state.archetype_state.matched_ids_pre_terms(),
        );

        // Sanity: adding a CompA archetype makes the Or filter pick it up.
        let with_a = ecs.create_archetype(&[COMP_A]);
        let mut state = state;
        state.update(ecs.archetype_master());
        assert_eq!(
            state.archetype_state.matched_ids_pre_terms(),
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
    ///
    /// Gated on `debug_assertions` because the tested method
    /// `assert_dual_invariant` is `#[cfg(debug_assertions)]`-only and would
    /// not exist in release-mode test builds.
    #[cfg(debug_assertions)]
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
        let ids = state.archetype_state.matched_ids_pre_terms();
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
        state.archetype_state.matched_ids_pre_terms_mut().push(real);

        QueryDataState::<&CompA, ()>::assert_dual_invariant(&state.archetype_state);
    }
}

/// EnableTag Wave 3 / Step 7a — the candidate-seeded global scan, the cull
/// seam, and the O2 invalidation, exercised at the `(D, F)` seam.
///
/// Component ids lazy-mint via `register_new::<T>()` + `OnceLock` (the Step-7
/// fixture pattern), so they never collide with the contested fixed-id blocks
/// in the shared lib-test process. The tag types are classified
/// `StorageKind::Bitset` at runtime (mirroring the Wave-5 derive).
#[cfg(test)]
mod enable_global_scan {
    use std::sync::OnceLock;

    use super::*;
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry::{self, StorageKind};
    use crate::ecs::core::entity::entity::Entity;
    use crate::ecs::core::iters::query::filter::{Changed, With};
    use crate::ecs::core::iters::query::filter_enable::{Disabled, Enabled};
    use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

    // ── Lazy-mint fixtures ───────────────────────────────────────────────────

    /// Bitset enable tag `A`.
    #[repr(C)]
    struct TagA;
    impl Component for TagA {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| ComponentId(component_registry::register_new::<TagA>()))
        }
    }
    /// Bitset enable tag `B` (the enable-tuple compile-fail twin).
    #[repr(C)]
    struct TagB;
    impl Component for TagB {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| ComponentId(component_registry::register_new::<TagB>()))
        }
    }
    /// Data component `P`.
    #[repr(C)]
    struct P {
        v: u32,
    }
    impl Component for P {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| ComponentId(component_registry::register_new::<P>()))
        }
    }

    /// Classifies `A` / `B` as bitset enable tags (the Wave-5 derive's runtime
    /// effect). Idempotent.
    fn register() {
        component_registry::set_storage_kind(TagA::component_id().0, StorageKind::Bitset);
        component_registry::set_storage_kind(TagB::component_id().0, StorageKind::Bitset);
    }

    /// Mints a fresh, distinct data component id (P-sized layout). Each call
    /// yields a unique signature element so callers can build genuinely distinct
    /// archetypes (`create_archetype(&[P, P])` would collapse duplicate ids).
    fn fresh_marker() -> ComponentId {
        ComponentId(component_registry::register_new::<P>())
    }

    /// Spawns one entity into `arch`, supplying a P-sized payload for EVERY
    /// component id in `comps` (`create_entity` requires bytes for all of the
    /// archetype's components). `comps` must equal the archetype's component set.
    fn spawn_into(ecs: &mut EcsMaster, arch: ArchetypeId, comps: &[ComponentId], v: u32) -> Entity {
        let p = P { v };
        // SAFETY (test): `p` outlives the borrow; byte view of a `#[repr(C)]` POD.
        // Every component here is P-sized (`P` itself + `fresh_marker` ids, which
        // `register_new::<P>` registered with P's layout), so one payload fits all.
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &p as *const P as *const u8,
                core::mem::size_of::<P>(),
            )
        };
        let payload: Vec<(ComponentId, &[u8])> = comps.iter().map(|&c| (c, bytes)).collect();
        ecs.create_entity(arch, &payload).expect("spawn must succeed")
    }

    /// Spawns one `P { v }` entity into a single-`[P]` archetype.
    fn spawn_p(ecs: &mut EcsMaster, arch: ArchetypeId, v: u32) -> Entity {
        spawn_into(ecs, arch, &[P::component_id()], v)
    }

    // ── Sole Disabled<A> bounded to present-A archetypes ──────────────────────

    /// `Query<(), Disabled<A>>` enumerates EXACTLY the disabled rows across
    /// multiple present-A archetypes, and is BOUNDED to present-for-A
    /// archetypes (it does NOT enumerate the world).
    #[test]
    fn sole_disabled_enumerates_disabled_rows_bounded_to_present() {
        register();
        let mut ecs = EcsMaster::new();
        // arch1 [P]: gains an A-column (e0 enabled, e1 clear within the column).
        let arch1 = ecs.create_archetype(&[P::component_id()]);
        let e0 = spawn_p(&mut ecs, arch1, 10);
        let _e1 = spawn_p(&mut ecs, arch1, 11);
        // arch2 [P, marker]: a distinct archetype that also gains an A-column
        // (e2 clear, e3 enabled). A fresh marker id gives a distinct signature.
        let marker = fresh_marker();
        let arch2_comps = [P::component_id(), marker];
        let arch2 = ecs.create_archetype(&arch2_comps);
        let _e2 = spawn_into(&mut ecs, arch2, &arch2_comps, 20);
        let e3 = spawn_into(&mut ecs, arch2, &arch2_comps, 21);
        // arch_absent [P, marker2]: NEVER gains an A-column ⇒ NOT a candidate.
        let marker2 = fresh_marker();
        let absent_comps = [P::component_id(), marker2];
        let arch_absent = ecs.create_archetype(&absent_comps);
        let _e_absent = spawn_into(&mut ecs, arch_absent, &absent_comps, 99);

        ecs.enable::<TagA>(e0); // arch1 present-A; e0 set, e1 clear.
        ecs.enable::<TagA>(e3); // arch2 present-A; e3 set, e2 clear.

        let state = QueryDataState::<(), Disabled<TagA>>::new(&mut ecs);
        let mut matched: Vec<_> = state.archetype_state.matched_ids_pre_terms().to_vec();
        matched.sort_unstable_by_key(|a| a.0);
        let mut expected = vec![arch1, arch2];
        expected.sort_unstable_by_key(|a| a.0);
        assert_eq!(
            matched, expected,
            "sole Disabled<A> candidate set = present-A archetypes only (arch_absent excluded)"
        );

        // Row-level: the two A-clear rows (e1 in arch1, e2 in arch2), bounded to
        // present-A; the no-column arch_absent row (99) is NOT visited.
        let view = ecs.query::<(), Disabled<TagA>>();
        assert_eq!(view.iter().count(), 2, "exactly the two A-clear rows in present-A archetypes");
    }

    // ── A1.1 two-shape coherence (MANDATORY) ──────────────────────────────────

    /// Amendment A1.1: a no-A-column archetype holding `e: P`.
    /// `Query<&P, Disabled<A>>` VISITS `e` (filter_fetch no-column = disabled);
    /// `Query<(), Disabled<A>>` does NOT (candidate set = present-A only). The
    /// two shapes answer different questions.
    #[test]
    fn two_shape_disabled_coherence_no_column_archetype() {
        register();
        let mut ecs = EcsMaster::new();
        // A present-A archetype so the candidate set is non-empty for the sole shape.
        let present = ecs.create_archetype(&[P::component_id()]);
        let pe = spawn_p(&mut ecs, present, 1);
        ecs.enable::<TagA>(pe); // allocate present's A-column; pe enabled.

        // A SECOND, distinct archetype with NO A-column, holding `e: P`.
        let marker = fresh_marker();
        let no_col_comps = [P::component_id(), marker];
        let no_col = ecs.create_archetype(&no_col_comps);
        let _e = spawn_into(&mut ecs, no_col, &no_col_comps, 42);

        // Positive-term shape: visits the no-A-column row (no-column ⇒ disabled).
        let pos_view = ecs.query::<&P, Disabled<TagA>>();
        let mut pos: Vec<u32> = pos_view.iter().map(|p: &P| p.v).collect();
        pos.sort_unstable();
        // pe is enabled ⇒ excluded; the no-column row (42) is disabled ⇒ visited.
        assert_eq!(pos, vec![42], "positive-term Disabled visits the no-A-column row");

        // Sole shape: candidate set = present-A only ⇒ no_col is NOT a candidate.
        let sole_state = QueryDataState::<(), Disabled<TagA>>::new(&mut ecs);
        let matched = sole_state.archetype_state.matched_ids_pre_terms();
        assert_eq!(
            matched,
            &[present],
            "sole Disabled<A> enumerates only present-A archetypes (no_col excluded)"
        );
        let sole_view = ecs.query::<(), Disabled<TagA>>();
        // Within `present`, pe is enabled ⇒ 0 disabled rows; no_col not visited.
        assert_eq!(
            sole_view.iter().count(),
            0,
            "sole Disabled<A> does NOT visit the no-A-column row e=42"
        );
    }

    // ── Sole Enabled<A> bounded to present-A ──────────────────────────────────

    /// `Query<(), Enabled<A>>` enumerates enabled rows, bounded to present-A.
    #[test]
    fn sole_enabled_enumerates_enabled_rows_bounded_to_present() {
        register();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[P::component_id()]);
        let e0 = spawn_p(&mut ecs, arch, 1);
        let _e1 = spawn_p(&mut ecs, arch, 2);
        let e2 = spawn_p(&mut ecs, arch, 3);
        // A distinct non-present archetype that must NOT be enumerated.
        let marker = fresh_marker();
        let absent_comps = [P::component_id(), marker];
        let absent = ecs.create_archetype(&absent_comps);
        let _ea = spawn_into(&mut ecs, absent, &absent_comps, 99);

        ecs.enable::<TagA>(e0);
        ecs.enable::<TagA>(e2);

        let state = QueryDataState::<(), Enabled<TagA>>::new(&mut ecs);
        assert_eq!(
            state.archetype_state.matched_ids_pre_terms(),
            &[arch],
            "sole Enabled<A> candidate set = present-A only (absent excluded)"
        );

        let view = ecs.query::<(), Enabled<TagA>>();
        assert_eq!(view.iter().count(), 2, "exactly the two A-enabled rows");
    }

    // ── Boundedness over many archetypes (cull-to-K) ──────────────────────────

    /// A many-archetype world where only K hold an A-column: the sole-Enabled
    /// candidate set is bounded to K, never N. Proves the popcount-bounded scan
    /// never degrades into a full-world sweep (the M2 hazard).
    #[test]
    fn sole_enabled_bounded_to_k_present_of_many() {
        register();
        let mut ecs = EcsMaster::new();
        // N distinct archetypes, each `[P, marker_i]` (a fresh marker component
        // per i gives a distinct signature). Only every 13th gains an A-column.
        // `register_new::<P>()` mints a fresh valid id AND registers a P-sized
        // layout in one call (so the marker can carry a payload row).
        const N: usize = 40;
        let mut present_archs = Vec::new();
        for i in 0..N {
            let marker = fresh_marker();
            let comps = [P::component_id(), marker];
            let arch = ecs.create_archetype(&comps);
            let e = spawn_into(&mut ecs, arch, &comps, i as u32);
            if i % 13 == 0 {
                ecs.enable::<TagA>(e); // i = 0, 13, 26, 39 ⇒ 4 present
                present_archs.push(arch);
            }
        }

        let state = QueryDataState::<(), Enabled<TagA>>::new(&mut ecs);
        let mut matched: Vec<_> = state.archetype_state.matched_ids_pre_terms().to_vec();
        matched.sort_unstable_by_key(|a| a.0);
        present_archs.sort_unstable_by_key(|a| a.0);
        assert_eq!(
            matched, present_archs,
            "candidate set = exactly the K={} present-A archetypes, never N={N}",
            present_archs.len(),
        );
        assert!(
            matched.len() < N,
            "matched ({}) bounded by K, strictly < N ({N})",
            matched.len(),
        );
    }

    // ── O2: first-toggle-into-a-new-archetype re-seed ─────────────────────────

    /// Build + cache `Query<(), Enabled<A>>`, iterate (empty), enable A in a
    /// not-yet-present archetype, then update + iterate again ⇒ the new row is
    /// visited. Proves the candidate path's `enable_generation` invalidation.
    #[test]
    fn o2_first_toggle_into_new_archetype_reseeds() {
        register();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[P::component_id()]);
        let e0 = spawn_p(&mut ecs, arch, 7);

        // No A toggled yet ⇒ empty candidate set.
        let mut state = QueryDataState::<(), Enabled<TagA>>::new(&mut ecs);
        assert!(
            state.archetype_state.matched_ids_pre_terms().is_empty(),
            "no present-A archetype yet ⇒ empty matched set"
        );

        // First toggle of A into `arch` ⇒ allocates its column, bumps
        // enable_generation.
        ecs.enable::<TagA>(e0);

        state.update(ecs.archetype_master());
        assert_eq!(
            state.archetype_state.matched_ids_pre_terms(),
            &[arch],
            "O2: enable_generation bump re-seeds the candidate set"
        );
    }

    /// O2 + removal: remove a candidate archetype ⇒ absent, no panic (A1.5/A4.4).
    #[test]
    fn o2_removal_of_candidate_archetype_is_purged() {
        register();
        let mut ecs = EcsMaster::new();
        let keep = ecs.create_archetype(&[P::component_id()]);
        let marker = fresh_marker();
        let drop_comps = [P::component_id(), marker];
        let drop_arch = ecs.create_archetype(&drop_comps);
        let ek = spawn_p(&mut ecs, keep, 1);
        let ed = spawn_into(&mut ecs, drop_arch, &drop_comps, 2);
        ecs.enable::<TagA>(ek);
        ecs.enable::<TagA>(ed); // allocate drop_arch's A-column too.

        let mut state = QueryDataState::<(), Enabled<TagA>>::new(&mut ecs);
        let mut matched: Vec<_> = state.archetype_state.matched_ids_pre_terms().to_vec();
        matched.sort_unstable_by_key(|a| a.0);
        let mut expected = vec![keep, drop_arch];
        expected.sort_unstable_by_key(|a| a.0);
        assert_eq!(matched, expected, "both present-A archetypes seeded");

        // Remove drop_arch (bumps structural_generation + clears its presence bits).
        assert!(ecs.archetype_master_mut().remove_archetype(drop_arch));

        state.update(ecs.archetype_master());
        assert_eq!(
            state.archetype_state.matched_ids_pre_terms(),
            &[keep],
            "removed candidate archetype is purged on the structural re-seed"
        );
    }

    // ── 0%-gate: normal queries still compile (no false-positive asserts) ──────

    /// A normal `Query<&P, With<TagB-as-data>>` and `Query<&P, Changed<P>>`
    /// still construct (the const-asserts don't false-positive). `IS_CANDIDATE_SEEDED`
    /// is `false` for both, so they take the unchanged `update_archetypes` path.
    #[test]
    fn zero_gate_normal_queries_construct() {
        register();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[P::component_id()]);
        let _e = spawn_p(&mut ecs, arch, 1);

        let with_state = QueryDataState::<&P, With<P>>::new(&mut ecs);
        const {
            assert!(!QueryDataState::<&P, With<P>>::IS_CANDIDATE_SEEDED);
            assert!(!QueryDataState::<&P, With<P>>::HAS_ENABLE_TERM);
        }
        let _ = with_state;

        let changed_state = QueryDataState::<&P, Changed<P>>::new(&mut ecs);
        const {
            assert!(!QueryDataState::<&P, Changed<P>>::IS_CANDIDATE_SEEDED);
            assert!(!QueryDataState::<&P, Changed<P>>::HAS_ENABLE_TERM);
        }
        let _ = changed_state;
    }

    /// Shape-const sanity: the candidate-seeded classification is exactly the
    /// sole-single-enable + no-positive-bound shape.
    #[test]
    fn shape_consts_classification() {
        const {
            assert!(QueryDataState::<(), Enabled<TagA>>::IS_CANDIDATE_SEEDED);
            assert!(QueryDataState::<(), Disabled<TagA>>::IS_CANDIDATE_SEEDED);
            assert!(QueryDataState::<(), Enabled<TagA>>::HAS_ENABLE_TERM);
            // Positive-term enable ⇒ NOT candidate-seeded, but IS an enable shape.
            assert!(!QueryDataState::<&P, Enabled<TagA>>::IS_CANDIDATE_SEEDED);
            assert!(QueryDataState::<&P, Enabled<TagA>>::HAS_ENABLE_TERM);
            // With-bounded enable ⇒ NOT candidate-seeded.
            assert!(!QueryDataState::<(), (With<P>, Enabled<TagA>)>::IS_CANDIDATE_SEEDED);
        }
    }
}
