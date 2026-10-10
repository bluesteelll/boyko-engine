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
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

/// Per-system state cache for a `Query<D, F>`.
///
/// Bundles three pieces:
/// * `archetype_state` — the Phase 5c [`QueryState`] archetype-match cache
///   (include/exclude/optional mask plus the `matched_ids` / dedup bitset
///   pair) populated by `update_archetypes`.
/// * `data_state` — `D::State`: per-`QueryData` cached metadata (resolved
///   [`ComponentId`]s).
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
/// * `QueryState::remove_matched_at` — `swap_remove`s the id and clears
///   the corresponding bit in the bitset.
///
/// `Self::assert_dual_invariant` is invoked at the tail of
/// `Self::post_filter_matched` in debug builds to surface any future
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
    /// EnableTag candidate-seeded O2 (amendment A4.2) — last-observed
    /// `enable_generation` (Relaxed) for the SOLE single-enable candidate path.
    ///
    /// Present in EVERY monomorphization but READ/WRITTEN only on the
    /// `IS_CANDIDATE_SEEDED` (sole single-enable) paths. A column-alloc
    /// first-toggle into a new archetype bumps `ArchetypeMaster::enable_generation`,
    /// and the next `update` re-seeds. The 8-byte tail is never touched by
    /// non-enable queries (the 0%-gate: `update`'s candidate branch is gated by
    /// `if const { Self::IS_CANDIDATE_SEEDED }`, so this load is not even emitted
    /// into a non-candidate monomorphization).
    last_observed_enable_generation: u64,
    /// EnableTag positive-term cull (Decision 1, Model B) — the recomputed
    /// culled id list plus its `EnablePresence::epoch()` invalidation stamp.
    ///
    /// READ/WRITTEN only on the positive-term `CONTAINS_ENABLE_TERM` paths
    /// (`HAS_ENABLE_TERM && !IS_CANDIDATE_SEEDED`). `Vec::new()` is alloc-free,
    /// so for every other `(D, F)` this is a zero-capacity Vec constructed once
    /// and never read — every access is `const { Self::HAS_ENABLE_TERM }`-gated
    /// (the 0%-gate). A plain `Vec` mutated only under `&mut QueryDataState`
    /// (`new` builds it locally; `update`'s recull writes it): NO interior
    /// mutability, NO raw-pointer caching (Decision 8 — unlike `term_scratch`).
    enable_cull: EnableCull,
    _marker: PhantomData<fn() -> (D, F)>,
}

/// EnableTag positive-term cull state (Decision 1, Model B). A cold tail on
/// [`QueryDataState`]; see that field's doc for the gating contract.
pub(crate) struct EnableCull {
    /// `matched_ids` minus enable-rejected archetypes — the driver id list for a
    /// positive-term enable query. Recomputed wholesale from the full
    /// `matched_ids` on each invalidation (Model B never mutates `matched_ids`).
    culled_ids: Vec<ArchetypeId>,
    /// Invalidation stamp = [`EnablePresence::epoch()`](crate::ecs::core::component::enable::enable_presence::EnablePresence::epoch)
    /// (Acquire, Decision 4) at the last recull. A change means a column was
    /// allocated, so the cull verdict may have moved ⇒ re-cull.
    last_observed_enable_epoch: u64,
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
    /// task #9 / Decision 4 — `!D::REQUIRES_POST_FILTER_TRIM` keeps an
    /// `AnyOf<…>` data term OUT of the candidate-seed fast path: its ≥1-member
    /// OR-trim lives ONLY in `post_filter_matched`, which the candidate-seed
    /// branch skips. Without this term `Query<AnyOf<(&A, &B)>, Enabled<C>>`
    /// would visit a C-present archetype lacking both A and B and yield a
    /// `(None, None)` row — a contract violation. The default-`false` const
    /// keeps the formula identical for every non-`AnyOf` data term (0%-gate).
    const IS_CANDIDATE_SEEDED: bool = F::IS_SOLE_SINGLE_ENABLE
        && !D::HAS_DATA_COMPONENT
        && !F::HAS_POSITIVE_ARCHETYPAL
        && !D::REQUIRES_POST_FILTER_TRIM;

    /// Dense plan D3 — `true` iff `(D, F)` carries a dense INCLUDE term
    /// (`&Dense` / `&mut Dense` / `With<Dense>`). When set AND the table include
    /// mask is empty at runtime, `new`/`update` seed the matched set from the
    /// union of the dense terms' `DenseStore::arch_presence` (bounded), instead
    /// of the empty-include full archetype scan. The per-row `dense_row_passes`
    /// / dense `filter_fetch` is the exact membership trim regardless of the
    /// seed (the seed only bounds the candidate set). Mutually exclusive with
    /// the enable candidate seed (no query mixes a dense include with an enable
    /// term in D3). `false` for a no-dense query (the 0%-gate — the seed branch
    /// const-folds OUT).
    const HAS_DENSE_INCLUDE: bool = D::HAS_DENSE_INCLUDE || F::HAS_DENSE_INCLUDE;

    /// Dense-enable plan D1 — `true` iff `(D, F)` combines a dense INCLUDE term
    /// with an enable term (`Enabled`/`Disabled`). This is the shape the
    /// zero-row "compile-but-lie" bug lived in: a dense-seeded query
    /// (`HAS_DENSE_INCLUDE`) that also carries a per-row enable predicate
    /// (`HAS_ENABLE_TERM`).
    ///
    /// It is DISJOINT from [`Self::IS_CANDIDATE_SEEDED`]: the candidate seed
    /// requires `!HAS_DATA_COMPONENT`, but a `&Dense`/`&mut Dense` include has
    /// `HAS_DATA_COMPONENT = true`, and a sole `Query<(), Enabled<Tag>>` carries
    /// no dense include. For every non-dense OR non-enable `(D, F)` this const
    /// folds to `false`, so the D2/D3 recull branches are dead-code-eliminated —
    /// the 0%-gate stays byte-identical (const-asserted in `shape_consts_classification`
    /// and the D6 dense-enable suite). Whether the query is REALLY dense-seeded
    /// (vs table-seeded) additionally requires `is_empty_include()` at runtime
    /// (see [`Self::use_dense_seed`]) — this const is the compile-time upper
    /// bound of the dense-enable shape family (D6 shape table).
    // Outside `#[cfg(test)]` this const has exactly ONE consumer — the
    // `#[cfg(debug_assertions)]` desync guard in `enable_driver_ids` — so a release
    // build sees it as dead. That is the shape, not a defect: it is a compile-time
    // classification whose whole job is to switch a debug-only invariant check on.
    // Without this the release `-D warnings` leg reds on a const the debug leg needs.
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    const IS_DENSE_ENABLE: bool = Self::HAS_DENSE_INCLUDE && Self::HAS_ENABLE_TERM;

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
    /// 4. Apply `Self::post_filter_matched` to drop any archetype that the
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
        // Decision 8 / W1: `self` does not exist yet, so the positive-term cull
        // recomputes into a LOCAL Vec moved into the struct literal below. Empty
        // for every non-positive-term shape (alloc-free).
        let mut culled_ids: Vec<ArchetypeId> = Vec::new();
        let mut last_observed_enable_epoch = 0u64;
        if const { Self::IS_CANDIDATE_SEEDED } {
            // Bounded candidate snapshot (popcount-walked by seed_from_candidates).
            let tag = <F as QueryFilter>::sole_enable_tag_id(&filter_state);
            let candidates = world.archetype_master().enable_presence().snapshot_present(tag);
            archetype_state.seed_from_candidates(&candidates, world.archetype_master());
            last_observed_enable_generation = world.archetype_master().enable_generation();
            // No `post_filter_matched`: the include mask is empty and the
            // candidate bitset IS the membership predicate (nothing to trim).
        } else if (const { Self::HAS_DENSE_INCLUDE }) && archetype_state.is_empty_include() {
            // Dense plan D3 dense-seed path: a dense INCLUDE term with NO table
            // positive bound (empty include mask) would otherwise full-scan the
            // world. Seed the candidate archetypes from the union of the dense
            // terms' `arch_presence` (bounded). The per-row `dense_row_passes`
            // / dense `filter_fetch` is the exact membership trim; the seed is a
            // conservative over-approximation (false positives trimmed per-row).
            Self::dense_seed(&data_state, &filter_state, &mut archetype_state, world);
            // Dense-enable plan D2 — bound the driver to enable-kept archetypes
            // over the dense-seeded candidate set. Gated by `HAS_ENABLE_TERM` so
            // a dense-only query (`Query<&Dense, Changed<Dense>>` etc.) never
            // emits this recull (the 0%-gate): the const folds the branch out and
            // `culled_ids` stays the empty `Vec` built below. `recull` uses
            // `F::enable_cull_keeps_archetype`, which encodes the polarity (D4):
            // `Enabled` tightens to column-bearing archetypes, `Disabled` keeps
            // all (A1.1 — a no-column dense archetype is all-disabled and must
            // NOT be dropped). The per-row `filter_fetch` is the exact trim.
            if const { Self::HAS_ENABLE_TERM } {
                Self::recull(
                    archetype_state.matched_ids_pre_terms(),
                    &filter_state,
                    world.archetype_master(),
                    &mut culled_ids,
                );
                last_observed_enable_epoch =
                    world.archetype_master().enable_presence().epoch();
            }
        } else {
            archetype_state.update_archetypes(world.archetype_master());
            Self::post_filter_matched(
                &mut archetype_state,
                &data_state,
                &filter_state,
                world.archetype_master(),
            );
            // Positive-term enable shapes recompute the cull list + stamp its
            // epoch here (Decision 3/4). `IS_CANDIDATE_SEEDED` is false in this
            // arm, so `HAS_ENABLE_TERM` here is exactly the positive-term shape.
            if const { Self::HAS_ENABLE_TERM } {
                Self::recull(
                    archetype_state.matched_ids_pre_terms(),
                    &filter_state,
                    world.archetype_master(),
                    &mut culled_ids,
                );
                last_observed_enable_epoch =
                    world.archetype_master().enable_presence().epoch();
            }
        }

        Self {
            archetype_state,
            data_state,
            filter_state,
            term_scratch: TermScratch::new(),
            last_observed_enable_generation,
            enable_cull: EnableCull {
                culled_ids,
                last_observed_enable_epoch,
            },
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
            // (Decision 5 structural-rebuild branch: `matched_ids` was rebuilt,
            // so the cull must recompute from the fresh set.)
            if const { Self::HAS_ENABLE_TERM } {
                Self::recull(
                    self.archetype_state.matched_ids_pre_terms(),
                    &self.filter_state,
                    master,
                    &mut self.enable_cull.culled_ids,
                );
                self.enable_cull.last_observed_enable_epoch =
                    master.enable_presence().epoch();
            }
        } else if const { Self::HAS_ENABLE_TERM } {
            // No archetype churn, but a column-alloc first-toggle may have
            // bumped the presence epoch (a new archetype gained the tag's
            // column). Re-cull over the bounded matched set if so. Gated by
            // `has_enable_term` ⇒ the load is not emitted for non-enable queries
            // (the 0%-gate warm path stays byte-identical above).
            //
            // Decision 4: the stamp is `EnablePresence::epoch()` (Acquire,
            // purpose-built) NOT `enable_generation()` (Relaxed) — the Acquire
            // trigger pairs with the Acquire `contains` oracle the cull reads.
            //
            // Decision 5 (re-add invariant): this warm-only branch is reached
            // iff both archetype + structural generations are unchanged ⇒
            // `matched_ids` is identical to its value at the last full recompute
            // ⇒ any archetype that newly satisfies the cull (gained a column)
            // was ALREADY in `matched_ids` (the positive term is enable-toggle
            // independent, and Model B never removes from `matched_ids`).
            // Recompute-from-`matched_ids` therefore re-adds it.
            //
            // CROSS-MODULE INVARIANT (O1): `epoch()` is bumped ONLY by
            // `note_column_alloc` (column-additions over an unchanged
            // `matched_ids`), so this warm-only branch deliberately handles ONLY
            // that case. A presence-bit CLEAR (archetype removal via
            // `EnablePresence::clear_archetype`) does NOT bump `epoch()` — it is
            // ALWAYS accompanied by a `structural_generation` bump, which routes
            // `update` into the structural-rebuild branch above. That branch
            // reculls from a fresh `matched_ids` (already lacking the removed id),
            // so the cleared bit needs no epoch signal here. Correctness rests on
            // this pairing (clear ⇒ structural bump); breaking it would silently
            // strand a stale id in `culled_ids`.
            let cur_epoch = master.enable_presence().epoch();
            if self.enable_cull.last_observed_enable_epoch != cur_epoch {
                Self::recull(
                    self.archetype_state.matched_ids_pre_terms(),
                    &self.filter_state,
                    master,
                    &mut self.enable_cull.culled_ids,
                );
                self.enable_cull.last_observed_enable_epoch = cur_epoch;
            }
        }
    }

    /// EnableTag positive-term presence cull (Decision 3, Model B) — recomputes
    /// `out` as `matched` minus the archetypes the typed enable term proves
    /// row-empty.
    ///
    /// Runs ONLY for positive-term `CONTAINS_ENABLE_TERM` shapes, over
    /// `matched_ids` (bounded by the required positive term — C2), NEVER an
    /// empty-include full scan. `out` is recomputed wholesale (`clear` +
    /// `extend` over a filtered `Copy` iterator) so the warm-only re-cull
    /// re-adds any archetype that newly gained a column (Decision 5: the
    /// re-add-by-construction invariant).
    ///
    /// Per-archetype verdict = [`QueryFilter::enable_cull_keeps_archetype`]:
    /// `Enabled<T>` keeps iff present (O(1) oracle); `Disabled<T>` keeps all
    /// (A1.1); an AND-tuple keeps iff every member keeps. No new `unsafe` —
    /// single-threaded `new`/`update`, reads the Acquire presence oracle, writes
    /// a plain Vec (Decision 3/8).
    fn recull(
        matched: &[ArchetypeId],
        filter_state: &F::State,
        master: &ArchetypeMaster,
        out: &mut Vec<ArchetypeId>,
    ) {
        out.clear();
        out.extend(
            matched
                .iter()
                .copied()
                .filter(|&a| F::enable_cull_keeps_archetype(filter_state, master, a)),
        );
    }

    /// Dense plan D3 — builds the candidate bitset from the union of `(D, F)`'s
    /// dense INCLUDE terms' `arch_presence` and seeds `archetype_state`.
    ///
    /// Called ONLY when `Self::HAS_DENSE_INCLUDE && include.is_empty()` — a
    /// dense include with no table positive bound. The seed is a CONSERVATIVE
    /// over-approximation (false positives are trimmed per-row by
    /// `dense_row_passes` / dense `filter_fetch`); `seed_from_candidates` is
    /// popcount-bounded and handles a structural rebuild internally.
    fn dense_seed(
        data_state: &D::State,
        filter_state: &F::State,
        archetype_state: &mut QueryState,
        world: &mut EcsMaster,
    ) {
        let mut candidates =
            crate::ecs::core::iters::archetype_bit_set::ArchetypeBitSet::new();
        {
            let registry = world.dense_registry();
            <D as QueryData>::dense_include_candidates(data_state, registry, &mut candidates);
            <F as QueryFilter>::dense_include_candidates(filter_state, registry, &mut candidates);
        }
        archetype_state.seed_from_candidates(&candidates, world.archetype_master());
    }

    /// Dense plan D3 — driver-entry refresh that routes a dense-include query
    /// through [`Self::dense_update`] and every other shape through
    /// [`Self::update`]. The single funnel for callers that hold both the
    /// `ArchetypeMaster` and the `DenseRegistry` (the `QueryView` path and the
    /// `Query` SystemParam path). Const-folded: a no-dense `(D, F)` skips the
    /// dense branch entirely and the `registry` argument is unused (the 0%-gate).
    #[inline]
    pub(crate) fn update_with_world(
        &mut self,
        master: &ArchetypeMaster,
        registry: &crate::ecs::core::component::dense::DenseRegistry,
    ) {
        if self.use_dense_seed() {
            self.dense_update(master, registry);
        } else {
            self.update(master);
        }
    }

    /// Dense plan D3 — the driver-entry re-seed for a dense-include query.
    ///
    /// Mirrors [`Self::update`] but for the dense-seed shape: it takes the dense
    /// `registry` (the world cell side) alongside `master` because the dense
    /// `arch_presence` lives in the `DenseRegistry`, not the `ArchetypeMaster`.
    /// `seed_from_candidates` dedup-skips already-matched ids (cheap re-seed) and
    /// does its own structural-mismatch full clear, so a later dense insert into
    /// a previously-unseeded archetype (which does NOT bump the archetype
    /// generation) is still picked up — the candidate bitset is rebuilt from the
    /// live `arch_presence` on every call.
    pub(crate) fn dense_update(
        &mut self,
        master: &ArchetypeMaster,
        registry: &crate::ecs::core::component::dense::DenseRegistry,
    ) {
        // Dense-enable plan D3 — snapshot the pre-reseed dense-generation signals
        // BEFORE `seed_from_candidates` overwrites them, so the enable recull can
        // be gated on "did the seeded set actually move?" (below). Two signals:
        //   * structural generation — a removal / ABA rebuild bumps it (and clears
        //     the affected presence bits, so the reseed drops the removed ids);
        //   * matched-id count — a dense-insert into a not-yet-seeded archetype
        //     delta-adds WITHOUT a structural bump, so it only shows up as a
        //     length increase.
        // Together they detect every membership change the reseed can produce.
        // Emitted ONLY for the enable shape (the `HAS_ENABLE_TERM` gate below
        // consumes them); a dense-only query const-folds the snapshot out.
        let pre_struct = self.archetype_state.last_observed_structural_generation();
        let pre_len = self.archetype_state.matched_ids_pre_terms().len();

        let mut candidates =
            crate::ecs::core::iters::archetype_bit_set::ArchetypeBitSet::new();
        <D as QueryData>::dense_include_candidates(&self.data_state, registry, &mut candidates);
        <F as QueryFilter>::dense_include_candidates(&self.filter_state, registry, &mut candidates);
        // The reseed of `matched_ids` is UNCONDITIONAL (dense inserts, archetype
        // churn, and removals must always be fresh); only the enable recull below
        // is gated.
        self.archetype_state.seed_from_candidates(&candidates, master);

        // Dense-enable plan D3 — re-home the positive-term enable recull onto the
        // dense refresh path (the dense shape bypasses `update()` entirely, so its
        // epoch-gated recull is unreachable there — the invalidation half of the
        // fix). Gated by `HAS_ENABLE_TERM` (the 0%-gate: a dense-only query never
        // emits this block).
        if const { Self::HAS_ENABLE_TERM } {
            // The recull GATE (REQUIRED, not optional — critic O1): `dense_update`
            // runs on EVERY per-frame query resolution, so an unconditional recull
            // would be a per-frame O(matched archetypes) `enable_cull_keeps_archetype`
            // scan — a regression vs the table path's epoch-gated warm branch.
            //   * `dense_generation_changed` — the seeded set grew (a new dense
            //     archetype, `pre_len != post_len`) OR a structural rebuild
            //     removed one (`pre_struct != master.structural_generation()`,
            //     which also covers a same-len removal+add). Either changes the
            //     cull membership ⇒ recull.
            //   * `epoch()` moved — `note_column_alloc` (a dense archetype gained
            //     the tag column for the first time) bumped it (Acquire, Decision 4).
            //     This term is load-bearing: a first-column alloc bumps NEITHER
            //     generation, so `dense_generation_changed` alone would miss it.
            // A pure enable-toggle of an existing row trips NEITHER term (its
            // archetype membership + column presence are unchanged; the flipped
            // bit is reflected solely by the per-row `filter_fetch` at iteration),
            // so the gate correctly skips it.
            let post_len = self.archetype_state.matched_ids_pre_terms().len();
            let dense_generation_changed = pre_len != post_len
                || pre_struct != self.archetype_state.last_observed_structural_generation();
            let cur_epoch = master.enable_presence().epoch();
            if dense_generation_changed
                || cur_epoch != self.enable_cull.last_observed_enable_epoch
            {
                Self::recull(
                    self.archetype_state.matched_ids_pre_terms(),
                    &self.filter_state,
                    master,
                    &mut self.enable_cull.culled_ids,
                );
                self.enable_cull.last_observed_enable_epoch = cur_epoch;
            }
        }
    }

    /// Dense plan D3 — `true` iff `update` should route through the dense-seed
    /// path. A `const` so the driver entry can `if const { … }`-gate the call
    /// (the registry mint folds out for a non-dense query — the 0%-gate). The
    /// runtime `include.is_empty()` check distinguishes a dense include that has
    /// a table positive bound (the include mask scan already bounds it) from one
    /// that does not (the seed bounds it).
    #[inline]
    pub(crate) fn use_dense_seed(&self) -> bool {
        (const { Self::HAS_DENSE_INCLUDE }) && self.archetype_state.is_empty_include()
    }

    /// The id slice a positive-term enable query's drivers walk (Decision 3).
    ///
    /// Three const-disjoint arms, all `const`-folded at monomorphization:
    /// * non-enable `F` (`!HAS_ENABLE_TERM`): the shared `matched_ids_pre_terms()`
    ///   load, byte-identical to pre-EnableTag (the 0%-gate — this method folds
    ///   to a single field load that the caller's no-terms arm already emitted).
    /// * candidate-seeded sole enable (`IS_CANDIDATE_SEEDED`): the seed already
    ///   bounded `matched_ids` to present-for-tag archetypes — no separate cull
    ///   list, so the pre-terms slice IS the driver set.
    /// * positive-term enable (`HAS_ENABLE_TERM && !IS_CANDIDATE_SEEDED`): the
    ///   recomputed [`EnableCull::culled_ids`].
    ///
    /// Routed by both `Query::driver_ids` and `QueryView::driver_ids` (and the
    /// count/is_empty no-terms arms — Decision 6) on the no-terms fast path.
    #[inline]
    pub(crate) fn enable_driver_ids(&self) -> &[ArchetypeId] {
        if const { Self::HAS_ENABLE_TERM } {
            if const { Self::IS_CANDIDATE_SEEDED } {
                self.archetype_state.matched_ids_pre_terms()
            } else {
                // Dense-enable plan D4 — for the IS_DENSE_ENABLE shape `culled_ids`
                // is now the driver (D2/D3 populate it). Debug-assert the two
                // invariants the fix rests on (mirroring `qs1_after_cull`):
                //   1. the driver IS `culled_ids` (not the raw `matched_ids`);
                //   2. `culled_ids ⊆ matched_ids` — the cull is a strict subset,
                //      never a desynced superset. Model B never mutates
                //      `matched_ids`, so a culled id that is NOT in `matched_ids`
                //      would signal a stranded/stale driver id (the exact desync
                //      `qs1_after_cull` guards for the table shape).
                #[cfg(debug_assertions)]
                if const { Self::IS_DENSE_ENABLE } {
                    let matched = self.archetype_state.matched_archetypes_bitset();
                    for id in &self.enable_cull.culled_ids {
                        debug_assert!(
                            matched.contains(id.0),
                            "dense-enable D4: culled driver id {} is not in matched_ids \
                             (culled_ids ⊄ matched_ids — desynced driver)",
                            id.0,
                        );
                    }
                }
                &self.enable_cull.culled_ids
            }
        } else {
            self.archetype_state.matched_ids_pre_terms()
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

    // ── Positive-term archetype cull (task #5, ENABLE-CULL-PLAN Decisions 1–7) ──
    //
    // These exercise the Model-B `recull` + `enable_driver_ids` routing for the
    // POSITIVE-TERM enable shape `Query<&D, Enabled<A>>` (`HAS_ENABLE_TERM &&
    // !IS_CANDIDATE_SEEDED`) — distinct from the candidate-seeded sole-term path
    // tested above. They reuse the parent fixtures (`register`, `spawn_p`,
    // `spawn_into`, `fresh_marker`, `TagA`/`TagB`/`P`) via `use super::*`.

    /// Decision 5 #1 (THE load-bearing re-add test): a positive-term
    /// `Query<&P, Enabled<A>>` over a world where archetype X has `P` but NO
    /// A-column culls X (0 rows visited). After `enable::<A>` on a row IN X (no
    /// structural churn — `set_enable_bit` allocates the column + bumps the
    /// `EnablePresence` epoch but never the structural/archetype generation), a
    /// fresh `update`+iter MUST re-add X and visit that row. Proves Model B
    /// re-adds an archetype that gained a column later (the recompute reads the
    /// untouched `matched_ids`, which always contained X).
    #[test]
    fn cull_then_enable_readds() {
        register();
        let mut ecs = EcsMaster::new();
        // Archetype X: holds `P`, never an A-column at first.
        let x = ecs.create_archetype(&[P::component_id()]);
        let ex = spawn_p(&mut ecs, x, 100);

        // Drive the state directly so we control update timing precisely.
        let mut state = QueryDataState::<&P, Enabled<TagA>>::new(&mut ecs);
        // No A-column anywhere ⇒ X is culled out of the driver ids on `new`.
        assert!(
            state.enable_driver_ids().is_empty(),
            "no A-column ⇒ X culled ⇒ empty driver ids; got {:?}",
            state.enable_driver_ids(),
        );

        // The view confirms 0 rows before the enable.
        {
            let pre = ecs.query::<&P, Enabled<TagA>>();
            assert_eq!(pre.iter().count(), 0, "no enabled rows before enable::<A>");
        }

        // Enable A on the row in X — allocates X's A-column, bumps the
        // EnablePresence epoch, but performs NO structural migration.
        ecs.enable::<TagA>(ex);

        // Re-cull: the epoch moved ⇒ recompute from the (unchanged) matched_ids
        // ⇒ X re-enters the driver set.
        state.update(ecs.archetype_master());
        assert_eq!(
            state.enable_driver_ids(),
            &[x],
            "Model B re-adds X after it gains an A-column (re-add invariant)",
        );

        // And the public view now visits ex.
        let post = ecs.query::<&P, Enabled<TagA>>();
        let got: Vec<u32> = post.iter().map(|p: &P| p.v).collect();
        assert_eq!(got, vec![100], "the newly-enabled row in X is now visited");
    }

    /// Decision 5 #2: create a NEW archetype Y AND `enable::<A>` into it between
    /// two updates. Y's appearance bumps the structural/archetype generation, so
    /// the structural-rebuild branch of `update` runs (not the warm-only epoch
    /// branch). The rebuild must re-cull, surfacing Y's enabled row.
    #[test]
    fn enable_into_new_archetype_interleaved() {
        register();
        let mut ecs = EcsMaster::new();
        // A present-A archetype so the first cull is non-empty.
        let base = ecs.create_archetype(&[P::component_id()]);
        let eb = spawn_p(&mut ecs, base, 1);
        ecs.enable::<TagA>(eb);

        let mut state = QueryDataState::<&P, Enabled<TagA>>::new(&mut ecs);
        assert_eq!(state.enable_driver_ids(), &[base], "base present-A on new");

        // Create a brand-new archetype Y (distinct signature) AND enable A in it
        // — both a structural churn (new archetype) and an epoch bump.
        let marker = fresh_marker();
        let y_comps = [P::component_id(), marker];
        let y = ecs.create_archetype(&y_comps);
        let ey = spawn_into(&mut ecs, y, &y_comps, 2);
        ecs.enable::<TagA>(ey);

        state.update(ecs.archetype_master());
        let mut ids: Vec<_> = state.enable_driver_ids().to_vec();
        ids.sort_unstable_by_key(|a| a.0);
        let mut expected = vec![base, y];
        expected.sort_unstable_by_key(|a| a.0);
        assert_eq!(
            ids, expected,
            "structural-rebuild branch re-culls ⇒ Y present-A appears",
        );

        let view = ecs.query::<&P, Enabled<TagA>>();
        let mut got: Vec<u32> = view.iter().map(|p: &P| p.v).collect();
        got.sort_unstable();
        assert_eq!(got, vec![1, 2], "both enabled rows visited (base + Y)");
    }

    /// Decision 5 #3: `culled_ids` is filled in `new` (NOT empty until the first
    /// `update`). Over a world with K present-A archetypes, the positive-term
    /// query's driver set equals exactly those K on the FIRST query — without any
    /// explicit `update` between `new` and the read.
    #[test]
    fn new_populates_culled_ids() {
        register();
        let mut ecs = EcsMaster::new();
        // K = 3 present-A archetypes + 2 no-A-column archetypes, all holding P.
        let mut present = Vec::new();
        for i in 0..3u32 {
            let marker = fresh_marker();
            let comps = [P::component_id(), marker];
            let arch = ecs.create_archetype(&comps);
            let e = spawn_into(&mut ecs, arch, &comps, i);
            ecs.enable::<TagA>(e);
            present.push(arch);
        }
        for i in 0..2u32 {
            let marker = fresh_marker();
            let comps = [P::component_id(), marker];
            let arch = ecs.create_archetype(&comps);
            let _e = spawn_into(&mut ecs, arch, &comps, 100 + i);
            // No enable ⇒ no A-column ⇒ must be culled.
        }

        // No `update` call between `new` and the read.
        let state = QueryDataState::<&P, Enabled<TagA>>::new(&mut ecs);
        let mut ids: Vec<_> = state.enable_driver_ids().to_vec();
        ids.sort_unstable_by_key(|a| a.0);
        present.sort_unstable_by_key(|a| a.0);
        assert_eq!(
            ids, present,
            "culled_ids filled in `new` = exactly the K present-A archetypes",
        );
    }

    /// Amendment A1.1 (disabled_does_not_cull): a positive-term
    /// `Query<&P, Disabled<A>>` over a world with a no-A-column archetype Z MUST
    /// visit Z's rows (no column ⇒ every row "disabled" ⇒ match). The
    /// `Disabled<T>` cull verdict is an explicit `true` (no cull), so Z stays in
    /// the driver set.
    #[test]
    fn disabled_does_not_cull() {
        register();
        let mut ecs = EcsMaster::new();
        // Z: holds P, never an A-column ⇒ every row disabled.
        let z = ecs.create_archetype(&[P::component_id()]);
        let _e0 = spawn_p(&mut ecs, z, 30);
        let _e1 = spawn_p(&mut ecs, z, 31);

        let state = QueryDataState::<&P, Disabled<TagA>>::new(&mut ecs);
        assert_eq!(
            state.enable_driver_ids(),
            &[z],
            "Disabled<A> MUST NOT cull the no-A-column archetype Z",
        );

        let view = ecs.query::<&P, Disabled<TagA>>();
        let mut got: Vec<u32> = view.iter().map(|p: &P| p.v).collect();
        got.sort_unstable();
        assert_eq!(got, vec![30, 31], "all of Z's rows visited (no-column = disabled)");
    }

    /// Tuple cull `(With<P>, Enabled<A>)`: the AND-composed cull drops a
    /// no-A-column archetype that HAS `P` but keeps a with-A-column one. Verifies
    /// the tuple macro AND-folds `enable_cull_keeps_archetype` (drop iff some
    /// member proves the archetype row-empty for the enable term).
    #[test]
    fn tuple_cull() {
        register();
        let mut ecs = EcsMaster::new();
        // with_col: holds P, gains an A-column (e_on enabled).
        let with_col = ecs.create_archetype(&[P::component_id()]);
        let e_on = spawn_p(&mut ecs, with_col, 1);
        ecs.enable::<TagA>(e_on);
        // no_col: a DISTINCT archetype that has P (so With<P> matches) but never
        // an A-column ⇒ the Enabled<A> member culls it.
        let marker = fresh_marker();
        let no_col_comps = [P::component_id(), marker];
        let no_col = ecs.create_archetype(&no_col_comps);
        let _e_off = spawn_into(&mut ecs, no_col, &no_col_comps, 2);

        let state = QueryDataState::<&P, (With<P>, Enabled<TagA>)>::new(&mut ecs);
        assert_eq!(
            state.enable_driver_ids(),
            &[with_col],
            "(With<P>, Enabled<A>) culls the no-A-column archetype, keeps with_col",
        );

        let view = ecs.query::<&P, (With<P>, Enabled<TagA>)>();
        let got: Vec<u32> = view.iter().map(|p: &P| p.v).collect();
        assert_eq!(got, vec![1], "only the with-A-column enabled row is visited");
    }

    /// Decision 7 (dynamic_term_mixed): a typed `Query<&P, Enabled<A>>` combined
    /// with a dynamic `with_enabled(B)` / `without_enabled(B)` term still matches
    /// a brute-force per-row oracle over a culled world. The typed cull only
    /// drops archetypes with ZERO typed-A rows, so it can never hide a row a
    /// dynamic term would surface.
    #[test]
    fn dynamic_term_mixed() {
        use crate::ecs::core::component::component_registry::EnableTagId;
        register();
        let mut ecs = EcsMaster::new();

        // arch1 [P]: gains an A-column. e0 (A on, B on), e1 (A on, B off).
        let arch1 = ecs.create_archetype(&[P::component_id()]);
        let e0 = spawn_p(&mut ecs, arch1, 10);
        let e1 = spawn_p(&mut ecs, arch1, 11);
        // arch2 [P, marker]: a distinct present-A archetype. e2 (A on, B on).
        let marker = fresh_marker();
        let a2 = [P::component_id(), marker];
        let arch2 = ecs.create_archetype(&a2);
        let e2 = spawn_into(&mut ecs, arch2, &a2, 12);
        // arch3 [P, marker2]: NO A-column (culled). e3 (B on) — must never
        // surface under `Enabled<A>` regardless of the dynamic B term.
        let marker2 = fresh_marker();
        let a3 = [P::component_id(), marker2];
        let arch3 = ecs.create_archetype(&a3);
        let e3 = spawn_into(&mut ecs, arch3, &a3, 13);

        // Toggle A (typed term) and B (dynamic term).
        ecs.enable::<TagA>(e0);
        ecs.enable::<TagA>(e1);
        ecs.enable::<TagA>(e2);
        ecs.enable::<TagB>(e0);
        ecs.enable::<TagB>(e2);
        ecs.enable::<TagB>(e3);

        let b_tag = EnableTagId(TagB::component_id());

        // Oracle: a row matches `Enabled<A> AND with_enabled(B)` iff A-set && B-set.
        // Over our world that is exactly {e0, e2}. e1 has A but not B; e3 has B
        // but no A-column (culled) ⇒ excluded.
        let with_view = ecs.query::<&P, Enabled<TagA>>().with_enabled(b_tag);
        let mut got_with: Vec<u32> = with_view.iter().map(|p: &P| p.v).collect();
        got_with.sort_unstable();
        assert_eq!(
            got_with,
            vec![10, 12],
            "Enabled<A> AND with_enabled(B): only A&&B rows (cull keeps no extra/loses none)",
        );

        // Oracle: `Enabled<A> AND without_enabled(B)` ⇒ A-set && B-clear ⇒ {e1}.
        let without_view = ecs.query::<&P, Enabled<TagA>>().without_enabled(b_tag);
        let got_without: Vec<u32> = without_view.iter().map(|p: &P| p.v).collect();
        assert_eq!(
            got_without,
            vec![11],
            "Enabled<A> AND without_enabled(B): only the A-set, B-clear row",
        );

        // Sanity: e3 (B-set but in a culled no-A-column archetype) never appears.
        assert!(
            !got_with.contains(&13) && !got_without.contains(&13),
            "the culled no-A-column row e3 is invisible to both dynamic polarities",
        );
        let _ = arch1; // silence unused in case of refactor
        let _ = arch2;
        let _ = arch3;
    }

    /// get_iter_agree: `QueryView::get` on an entity in a no-A-column archetype
    /// with `Query<&P, Enabled<A>>` returns `None` (per-row exact), proving `get`
    /// and `iter` agree at the row level even though `get` uses the per-row
    /// bitset test while `iter` walks the culled archetype set.
    #[test]
    fn get_iter_agree() {
        register();
        let mut ecs = EcsMaster::new();
        // present: a with-A-column archetype; e_on enabled.
        let present = ecs.create_archetype(&[P::component_id()]);
        let e_on = spawn_p(&mut ecs, present, 1);
        ecs.enable::<TagA>(e_on);
        // no_col: a distinct no-A-column archetype holding e_off.
        let marker = fresh_marker();
        let comps = [P::component_id(), marker];
        let no_col = ecs.create_archetype(&comps);
        let e_off = spawn_into(&mut ecs, no_col, &comps, 2);

        let view = ecs.query::<&P, Enabled<TagA>>();
        // get on the no-A-column row ⇒ None (per-row enable test fails).
        assert!(
            view.get(e_off).is_none(),
            "get on a no-A-column row returns None (agrees with cull/iter)",
        );
        // get on the enabled row ⇒ Some.
        assert!(
            view.get(e_on).is_some(),
            "get on the enabled row returns Some",
        );
        // iter never yields e_off's value either.
        let got: Vec<u32> = view.iter().map(|p: &P| p.v).collect();
        assert_eq!(got, vec![1], "iter visits only the enabled row — agrees with get");
    }

    /// QS1 after cull: the dual invariant (matched_ids ⇔ dedup bitset bijection)
    /// still holds after a positive-term cull. Model B never mutates
    /// `matched_ids`, so the cull cannot desync QS1 — the bitset/popcount track
    /// the FULL matched set, while `culled_ids` is the separate driver list.
    #[cfg(debug_assertions)]
    #[test]
    fn qs1_after_cull() {
        register();
        let mut ecs = EcsMaster::new();
        // Two with-A-column archetypes + one no-A-column ⇒ matched_ids has all 3
        // (the positive term &P matches every one), culled_ids has only the 2.
        let a1 = ecs.create_archetype(&[P::component_id()]);
        let e1 = spawn_p(&mut ecs, a1, 1);
        ecs.enable::<TagA>(e1);
        let m2 = fresh_marker();
        let c2 = [P::component_id(), m2];
        let a2 = ecs.create_archetype(&c2);
        let e2 = spawn_into(&mut ecs, a2, &c2, 2);
        ecs.enable::<TagA>(e2);
        let m3 = fresh_marker();
        let c3 = [P::component_id(), m3];
        let a3 = ecs.create_archetype(&c3);
        let _e3 = spawn_into(&mut ecs, a3, &c3, 3); // no A-column ⇒ culled

        let state = QueryDataState::<&P, Enabled<TagA>>::new(&mut ecs);

        // QS1 holds against the FULL matched set (untouched by the cull).
        QueryDataState::<&P, Enabled<TagA>>::assert_dual_invariant(&state.archetype_state);
        let bitset = state.archetype_state.matched_archetypes_bitset();
        let matched = state.archetype_state.matched_ids_pre_terms();
        assert_eq!(
            matched.len(),
            3,
            "matched_ids holds all 3 &P archetypes (Model B leaves it untouched)",
        );
        assert_eq!(
            bitset.popcount() as usize,
            matched.len(),
            "popcount == matched_ids len after the cull (QS1 bijection intact)",
        );
        for id in matched {
            assert!(
                bitset.contains(id.0),
                "matched id {} set in the dedup bitset post-cull",
                id.0,
            );
        }
        // The cull is a STRICT subset of matched_ids (2 of 3), not a mutation.
        assert_eq!(
            state.enable_driver_ids().len(),
            2,
            "culled driver set = 2 present-A archetypes ⊂ the 3 matched_ids",
        );
    }
}

/// Dense-enable plan (D0–D6) — the dense-INCLUDE × enable-term query support.
///
/// The regression witness for the "compile-but-lie" zero-row bug: a query that
/// combines a **dense-stored** component term (`&Dense` / `&mut Dense`, an empty
/// table include) with an **enable term** (`Enabled`/`Disabled`). Every fixture
/// that means to exercise the dense-seed path asserts `state.use_dense_seed() ==
/// true` (critic W2) — otherwise it silently routes through the `update()` table
/// recull and leaves D2/D3 unverified.
///
/// Fixtures lazy-mint component ids (`register_new::<T>()` + `OnceLock`) and
/// classify the dense component `StorageKind::Dense` + the tag `StorageKind::Bitset`
/// at runtime, matching `enable_global_scan`.
#[cfg(test)]
mod dense_enable {
    use std::sync::OnceLock;

    use super::*;
    use crate::ecs::core::component::component::Component;
    use crate::ecs::core::component::component_registry::{self, StorageKind};
    use crate::ecs::core::entity::entity::Entity;
    use crate::ecs::core::iters::query::data::AnyOf;
    use crate::ecs::core::iters::query::filter::{Changed, With};
    use crate::ecs::core::iters::query::filter_enable::{Disabled, Enabled};
    use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

    // ── Fixtures ─────────────────────────────────────────────────────────────

    /// A dense-stored payload. `STORAGE_IS_DENSE = true` at the TYPE
    /// level gives `HAS_DENSE_INCLUDE` on the query data; the runtime
    /// `set_storage_kind(_, Dense)` in `register()` routes its inserts to the
    /// `DenseStore` (via `partition_dense_components`).
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Dn {
        v: u32,
    }
    impl Component for Dn {
        const STORAGE_IS_DENSE: bool = true;
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| ComponentId(component_registry::register_new::<Dn>()))
        }
    }

    /// A plain TABLE component `B` (the mixed `(&Dn, &B)` / `AnyOf` sibling).
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct B {
        w: u32,
    }
    impl Component for B {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| ComponentId(component_registry::register_new::<B>()))
        }
    }

    /// Bitset enable tag.
    #[repr(C)]
    struct Tag;
    impl Component for Tag {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| ComponentId(component_registry::register_new::<Tag>()))
        }
    }

    /// Classifies `Dn` dense + `Tag` bitset (the derive's runtime effect).
    /// Idempotent (`set_storage_kind` is a write-once-then-idempotent register).
    fn register() {
        component_registry::set_storage_kind(Dn::component_id().0, StorageKind::Dense);
        component_registry::set_storage_kind(B::component_id().0, StorageKind::Table);
        component_registry::set_storage_kind(Tag::component_id().0, StorageKind::Bitset);
    }

    /// Mints a fresh, distinct table marker id (P-sized layout) so callers can
    /// build genuinely distinct archetypes.
    fn fresh_marker() -> ComponentId {
        ComponentId(component_registry::register_new::<Dn>())
    }

    /// Spawns one entity into `arch`, supplying a `Dn`-sized payload for every id
    /// in `payload_ids` (which must cover the archetype's table components PLUS
    /// any dense ids to be routed to the `DenseStore`). Returns the `Entity`.
    fn spawn(
        ecs: &mut EcsMaster,
        arch: ArchetypeId,
        payload_ids: &[ComponentId],
        v: u32,
    ) -> Entity {
        let d = Dn { v };
        // SAFETY (test): `d` outlives the borrow; byte view of a `#[repr(C)]` POD.
        // Every id here is `Dn`-sized (`Dn`/`B`/`fresh_marker`, all registered via
        // `register_new::<Dn>()`), so one payload fits all.
        let bytes = unsafe {
            core::slice::from_raw_parts(&d as *const Dn as *const u8, core::mem::size_of::<Dn>())
        };
        let payload: Vec<(ComponentId, &[u8])> =
            payload_ids.iter().map(|&c| (c, bytes)).collect();
        ecs.create_entity(arch, &payload).expect("spawn must succeed")
    }

    /// Collects the `v` values a `Query<&mut Dn, F>` iterates (via the public view
    /// + `iter_mut` archetype cursor, which enforces the per-row enable bit).
    fn iter_enabled_vs(ecs: &mut EcsMaster) -> Vec<u32> {
        let mut view = ecs.query::<&mut Dn, Enabled<Tag>>();
        let mut got: Vec<u32> = view.iter_mut().map(|d: &mut Dn| d.v).collect();
        got.sort_unstable();
        got
    }

    /// Collects the `v` values a `Query<&mut Dn, Disabled<Tag>>` iterates.
    fn iter_disabled_vs(ecs: &mut EcsMaster) -> Vec<u32> {
        let mut view = ecs.query::<&mut Dn, Disabled<Tag>>();
        let mut got: Vec<u32> = view.iter_mut().map(|d: &mut Dn| d.v).collect();
        got.sort_unstable();
        got
    }

    // ── Test #7: const classification (0%-gate) ──────────────────────────────

    /// `IS_DENSE_ENABLE` is `true` EXACTLY for the dense+enable shape, `false`
    /// for dense-only, enable-only, and plain queries. `IS_CANDIDATE_SEEDED`
    /// stays `false` for the dense+enable shape (it is not the sole-enable shape).
    /// Plain `Query<&P, With<P>>` is unaffected (the 0%-gate const-assert).
    #[test]
    fn const_classification() {
        const {
            // Dense + enable ⇒ IS_DENSE_ENABLE, and NOT candidate-seeded.
            assert!(QueryDataState::<&mut Dn, Enabled<Tag>>::IS_DENSE_ENABLE);
            assert!(QueryDataState::<&mut Dn, Disabled<Tag>>::IS_DENSE_ENABLE);
            assert!(QueryDataState::<&Dn, Enabled<Tag>>::IS_DENSE_ENABLE);
            assert!(!QueryDataState::<&mut Dn, Enabled<Tag>>::IS_CANDIDATE_SEEDED);
            assert!(QueryDataState::<&mut Dn, Enabled<Tag>>::HAS_ENABLE_TERM);
            assert!(QueryDataState::<&mut Dn, Enabled<Tag>>::HAS_DENSE_INCLUDE);

            // Dense-only ⇒ NOT dense-enable, NOT enable at all.
            assert!(!QueryDataState::<&mut Dn, Changed<Dn>>::IS_DENSE_ENABLE);
            assert!(!QueryDataState::<&mut Dn, Changed<Dn>>::HAS_ENABLE_TERM);
            assert!(QueryDataState::<&mut Dn, ()>::HAS_DENSE_INCLUDE);
            assert!(!QueryDataState::<&mut Dn, ()>::IS_DENSE_ENABLE);

            // Enable-only (sole) ⇒ NOT dense-enable (no dense include).
            assert!(!QueryDataState::<(), Enabled<Tag>>::IS_DENSE_ENABLE);
            assert!(QueryDataState::<(), Enabled<Tag>>::IS_CANDIDATE_SEEDED);

            // Plain query ⇒ untouched (the 0%-gate).
            assert!(!QueryDataState::<&B, With<B>>::IS_DENSE_ENABLE);
            assert!(!QueryDataState::<&B, With<B>>::HAS_DENSE_INCLUDE);
            assert!(!QueryDataState::<&B, With<B>>::HAS_ENABLE_TERM);
        }
    }

    // ── Test #1: positive behavioral, Enabled (the zero-row regression) ──────

    /// `Query<&mut Dn, Enabled<Tag>>` over a world with some-enabled /
    /// all-enabled / all-disabled dense rows yields EXACTLY the enabled dense
    /// rows. This is the direct witness for the zero-row bug (pre-fix the dense
    /// seed left `culled_ids` empty ⇒ zero driver archetypes ⇒ zero rows).
    #[test]
    fn dense_enable_yields_only_enabled_rows() {
        register();
        let mut ecs = EcsMaster::new();
        // arch1 [Dn-dense]: e0 enabled, e1 disabled (some-enabled).
        let arch1 = ecs.create_archetype(&[]);
        let e0 = spawn(&mut ecs, arch1, &[Dn::component_id()], 10);
        let _e1 = spawn(&mut ecs, arch1, &[Dn::component_id()], 11);
        // arch2 [Dn, marker]: e2 enabled, e3 enabled (all-enabled).
        let marker = fresh_marker();
        let arch2 = ecs.create_archetype(&[marker]);
        let e2 = spawn(&mut ecs, arch2, &[marker, Dn::component_id()], 20);
        let e3 = spawn(&mut ecs, arch2, &[marker, Dn::component_id()], 21);
        // arch3 [Dn, marker2]: e4 disabled, e5 disabled (all-disabled).
        let marker2 = fresh_marker();
        let arch3 = ecs.create_archetype(&[marker2]);
        let _e4 = spawn(&mut ecs, arch3, &[marker2, Dn::component_id()], 30);
        let _e5 = spawn(&mut ecs, arch3, &[marker2, Dn::component_id()], 31);

        ecs.enable::<Tag>(e0);
        ecs.enable::<Tag>(e2);
        ecs.enable::<Tag>(e3);

        // W2: the shape MUST route through the dense-seed path.
        let state = QueryDataState::<&mut Dn, Enabled<Tag>>::new(&mut ecs);
        assert!(
            state.use_dense_seed(),
            "Query<&mut Dn, Enabled<Tag>> must dense-seed (else D2/D3 unverified)"
        );

        let got = iter_enabled_vs(&mut ecs);
        assert_eq!(
            got,
            vec![10, 20, 21],
            "only the enabled dense rows are visited (e1/e4/e5 disabled, excluded)"
        );
    }

    // ── Test #2: polarity, Disabled (the A1.1 no-column trap) ────────────────

    /// `Query<&mut Dn, Disabled<Tag>>` yields the disabled dense rows INCLUDING
    /// rows in dense archetypes that never had the tag column. A no-column dense
    /// archetype is all-disabled (A1.1), so an up-front presence intersection
    /// would false-empty this — the reason the fix uses `recull` (which keeps all
    /// archetypes for the `Disabled` polarity) not an intersection.
    #[test]
    fn dense_disabled_includes_no_column_archetypes() {
        register();
        let mut ecs = EcsMaster::new();
        // present [Dn]: gains a Tag column; e_on enabled, e_off disabled.
        let present = ecs.create_archetype(&[]);
        let e_on = spawn(&mut ecs, present, &[Dn::component_id()], 1);
        let _e_off = spawn(&mut ecs, present, &[Dn::component_id()], 2);
        ecs.enable::<Tag>(e_on);
        // no_col [Dn, marker]: NEVER gains a Tag column ⇒ every row disabled.
        let marker = fresh_marker();
        let no_col = ecs.create_archetype(&[marker]);
        let _e = spawn(&mut ecs, no_col, &[marker, Dn::component_id()], 42);

        let state = QueryDataState::<&mut Dn, Disabled<Tag>>::new(&mut ecs);
        assert!(state.use_dense_seed(), "Disabled dense shape must dense-seed");

        let got = iter_disabled_vs(&mut ecs);
        assert_eq!(
            got,
            vec![2, 42],
            "disabled rows include the no-Tag-column archetype (42) — A1.1 trap"
        );
    }

    // ── Test #3: per-row toggle (exact path, NOT recull) ─────────────────────

    /// In a column-present dense archetype, disable then re-enable a row → it
    /// leaves then re-enters the result across `update`s. `contains` is
    /// column-presence, so the archetype stays in `culled_ids` throughout and
    /// ONLY `filter_fetch` changes. A regression that dropped per-row enforcement
    /// (leaving only the coarse archetype cull) would keep the disabled row
    /// visible and fail here.
    #[test]
    fn dense_per_row_toggle_exact_trim() {
        register();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[]);
        let e0 = spawn(&mut ecs, arch, &[Dn::component_id()], 7);
        let e1 = spawn(&mut ecs, arch, &[Dn::component_id()], 8);
        ecs.enable::<Tag>(e0);
        ecs.enable::<Tag>(e1);

        assert_eq!(iter_enabled_vs(&mut ecs), vec![7, 8], "both enabled initially");

        // Disable e1 (a pure per-row toggle — no column-alloc, no structural churn).
        ecs.disable::<Tag>(e1);
        assert_eq!(
            iter_enabled_vs(&mut ecs),
            vec![7],
            "disabled row leaves the result (per-row filter_fetch)"
        );

        // Re-enable e1.
        ecs.enable::<Tag>(e1);
        assert_eq!(
            iter_enabled_vs(&mut ecs),
            vec![7, 8],
            "re-enabled row re-enters (per-row exact trim over the dense-seeded driver)"
        );
    }

    // ── Test #4: re-seed on dense-insert (the dense_generation_changed gate) ──

    /// Insert the dense component into a NEW archetype that has the tag → it is
    /// picked up on the next resolve. Exercises `dense_update`'s unconditional
    /// reseed of `matched_ids` + the gated recull (the `dense_generation_changed`
    /// term: the seeded set grew).
    #[test]
    fn dense_reseed_on_new_archetype() {
        register();
        let mut ecs = EcsMaster::new();
        // Base present-Tag dense archetype so the first resolve is non-empty.
        let base = ecs.create_archetype(&[]);
        let eb = spawn(&mut ecs, base, &[Dn::component_id()], 1);
        ecs.enable::<Tag>(eb);
        assert_eq!(iter_enabled_vs(&mut ecs), vec![1], "base enabled row seen");

        // A brand-new dense archetype with an enabled row (distinct signature).
        let marker = fresh_marker();
        let y = ecs.create_archetype(&[marker]);
        let ey = spawn(&mut ecs, y, &[marker, Dn::component_id()], 2);
        ecs.enable::<Tag>(ey);

        assert_eq!(
            iter_enabled_vs(&mut ecs),
            vec![1, 2],
            "dense_update reseed + recull surfaces the new archetype's enabled row"
        );
    }

    // ── Test #5: column-alloc epoch (the D3 recull invalidation) ─────────────

    /// A dense archetype gains the Tag column for the FIRST time mid-run
    /// (`enable::<Tag>` on a row in a previously-all-disabled dense archetype →
    /// `note_column_alloc` bumps the presence epoch WITHOUT any structural /
    /// dense-generation change). The epoch-gated recull re-adds it exactly once.
    /// This is the test that fails if the recull is gated on the dense generation
    /// ALONE (the epoch term is load-bearing — a first-column alloc bumps neither
    /// generation nor the matched-id count).
    #[test]
    fn dense_column_alloc_epoch_readds() {
        register();
        let mut ecs = EcsMaster::new();
        // x [Dn]: holds a dense row, NO Tag column at first.
        let x = ecs.create_archetype(&[]);
        let ex = spawn(&mut ecs, x, &[Dn::component_id()], 100);

        // Drive the state directly so we control resolve timing precisely.
        let mut state = QueryDataState::<&mut Dn, Enabled<Tag>>::new(&mut ecs);
        assert!(state.use_dense_seed(), "must dense-seed");
        // No Tag column anywhere ⇒ x is culled out of the driver on `new`.
        assert!(
            state.enable_driver_ids().is_empty(),
            "no Tag column ⇒ x culled ⇒ empty driver; got {:?}",
            state.enable_driver_ids(),
        );

        // Enable Tag on the row in x — allocates x's Tag column, bumps the
        // EnablePresence epoch, NO structural / dense-generation change.
        ecs.enable::<Tag>(ex);

        // dense_update path (the dense shape's per-frame resolve). The epoch moved
        // ⇒ the gated recull re-adds x. Two simultaneous SHARED borrows of `ecs`
        // (master + dense registry) are legal — neither is `&mut`.
        {
            let master = ecs.archetype_master();
            let registry = ecs.dense_registry();
            state.update_with_world(master, registry);
        }
        assert_eq!(
            state.enable_driver_ids(),
            &[x],
            "epoch-gated recull re-adds x after it gains a Tag column (D3)",
        );

        // And the public view now visits the newly-enabled row.
        assert_eq!(
            iter_enabled_vs(&mut ecs),
            vec![100],
            "the newly-enabled dense row in x is visited",
        );
    }

    // ── Test #6: get/iter agreement (critic W3) ──────────────────────────────

    /// For a dense+enable query, `get(entity)` on an enabled dense entity DECIDES
    /// membership by the same per-row enable predicate as `iter`: the enabled
    /// entity is admitted (would be `Some`), the disabled one rejected (`None`),
    /// and the admitted set equals the `iter` set. `get` routes through
    /// `matched_archetypes_bitset` + `query_view_enable_passes` — a DIFFERENT path
    /// than `culled_ids` — so it needs its own witness (critic W3).
    ///
    /// PRE-EXISTING BUG (out of D0–D6 scope, flagged for the reviewer): the FINAL
    /// step of `QueryView::get`/`get_mut` — `D::fetch` for a DENSE `D` — reads
    /// `fetch.dense`, which `get` NEVER resolves (only the iter cursors call
    /// `resolve_dense`; `get` skips it). So the dense `fetch` arm null-derefs for
    /// ANY dense `get`, independent of enable. This surfaced ONLY now because the
    /// D2/D3 fix first made a dense+enable query reach a non-empty result. To keep
    /// this planner-scope test green without depending on the broken `get`-fetch,
    /// we witness the enable DECISION via `matched_archetypes_bitset()` +
    /// `query_view_enable_passes` (the exact predicate `get` uses BEFORE the
    /// broken fetch) and confirm the admitted set equals the `iter` set. Fixing
    /// `get`'s dense fetch (a one-line `resolve_dense` mirror) is a separate,
    /// reviewer-triaged repair — NOT folded here (scope discipline).
    #[test]
    fn dense_get_iter_agree() {
        use crate::ecs::core::iters::query::filter_enable::query_view_enable_passes;

        register();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[]);
        let e_on = spawn(&mut ecs, arch, &[Dn::component_id()], 1);
        let e_off = spawn(&mut ecs, arch, &[Dn::component_id()], 2);
        ecs.enable::<Tag>(e_on);

        // The dense+enable `&Dn` query must route through the dense-seed.
        let state = QueryDataState::<&Dn, Enabled<Tag>>::new(&mut ecs);
        assert!(state.use_dense_seed(), "read-only dense+enable must dense-seed");

        // The `get`-decision predicate: membership (matched bitset) + per-row
        // enable (`query_view_enable_passes`) — exactly what `get` tests before
        // the (pre-existing-broken) dense fetch. Both entities share `arch`, which
        // must be a driver (present-Tag column).
        let master = ecs.archetype_master();
        let arch_ptr = master.get_archetype(arch).expect("arch live") as *const _;
        let bitset = state.archetype_state.matched_archetypes_bitset();
        assert!(bitset.contains(arch.0), "arch is a matched driver for the get path");

        // e_on's row is admitted; e_off's row is rejected — by the SAME per-row
        // predicate `iter` uses. Rows are unit_index order: e_on=row 0, e_off=row 1.
        // SAFETY (test): `arch_ptr` is the live matched archetype; rows 0/1 exist.
        let on_passes =
            unsafe { query_view_enable_passes::<Enabled<Tag>>(&state.filter_state, arch_ptr, 0) };
        let off_passes =
            unsafe { query_view_enable_passes::<Enabled<Tag>>(&state.filter_state, arch_ptr, 1) };
        assert!(on_passes, "get-decision admits the enabled dense row (would be Some)");
        assert!(!off_passes, "get-decision rejects the disabled dense row (would be None)");
        let _ = (e_on, e_off);
        drop(state);

        // The iter set equals the admitted (get-decision-Some) set ({e_on}).
        let view = ecs.query::<&Dn, Enabled<Tag>>();
        let got: Vec<u32> = view.iter().map(|d: &Dn| d.v).collect();
        assert_eq!(
            got,
            vec![1],
            "iter visits only the enabled dense row — agrees with the get decision",
        );
    }

    // ── Test #8 companion: positive iter_mut of the D0-rejected query ─────────

    /// Positive companion to the D0 `dense_iter_mut` compile-reject: the SAME
    /// `Query<&mut Dn, Enabled<Tag>>` that CANNOT use `dense_iter_mut()` iterates
    /// correctly via `iter_mut()` (the archetype-walking cursor). A write through
    /// the yielded `&mut` lands only on enabled rows.
    #[test]
    fn dense_enable_iter_mut_positive_companion() {
        register();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[]);
        let e_on = spawn(&mut ecs, arch, &[Dn::component_id()], 5);
        let _e_off = spawn(&mut ecs, arch, &[Dn::component_id()], 6);
        ecs.enable::<Tag>(e_on);

        // iter_mut over the dense+enable query: write +100 to every visited row.
        {
            let mut view = ecs.query::<&mut Dn, Enabled<Tag>>();
            for d in view.iter_mut() {
                let d: &mut Dn = d;
                d.v += 100;
            }
        }

        // Only the enabled row was mutated (105); the disabled row is untouched (6).
        let mut all: Vec<u32> = {
            let mut view = ecs.query::<&mut Dn, ()>();
            view.iter_mut().map(|d: &mut Dn| d.v).collect()
        };
        all.sort_unstable();
        assert_eq!(
            all,
            vec![6, 105],
            "iter_mut writes land only on the enabled row (disabled row untouched)"
        );
    }

    // ── D6 pre-existing rows: smoke assertions ───────────────────────────────

    /// D6 row `Query<&Dn, (With<B>, Enabled<Tag>)>` — the `With<B>` include bit
    /// routes it to the TABLE path (`use_dense_seed() == false`), where the
    /// pre-existing positive-term recull already works. Smoke-assert it yields
    /// correct rows today (the fix's completeness depends on it).
    #[test]
    fn d6_with_table_plus_enable_table_path() {
        register();
        let mut ecs = EcsMaster::new();
        // arch [B(table), Dn(dense)]: e_on enabled, e_off disabled.
        let arch = ecs.create_archetype(&[B::component_id()]);
        let e_on = spawn(&mut ecs, arch, &[B::component_id(), Dn::component_id()], 1);
        let _e_off = spawn(&mut ecs, arch, &[B::component_id(), Dn::component_id()], 2);
        ecs.enable::<Tag>(e_on);

        // With<B> gives a table include bit ⇒ NOT dense-seeded (table path).
        let state = QueryDataState::<&Dn, (With<B>, Enabled<Tag>)>::new(&mut ecs);
        assert!(
            !state.use_dense_seed(),
            "With<B> include bit routes this to the table path, not the dense seed"
        );

        let view = ecs.query::<&Dn, (With<B>, Enabled<Tag>)>();
        let got: Vec<u32> = view.iter().map(|d: &Dn| d.v).collect();
        assert_eq!(got, vec![1], "table-path positive-term recull yields the enabled row");
    }

    /// D6 row `Query<(&Dn, &B), Enabled<Tag>>` — the `&B` term sets a table
    /// include bit ⇒ table path. Pre-existing positive-term recull. Smoke.
    #[test]
    fn d6_dense_and_table_plus_enable_table_path() {
        register();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[B::component_id()]);
        let e_on = spawn(&mut ecs, arch, &[B::component_id(), Dn::component_id()], 10);
        let _e_off = spawn(&mut ecs, arch, &[B::component_id(), Dn::component_id()], 11);
        ecs.enable::<Tag>(e_on);

        let state = QueryDataState::<(&Dn, &B), Enabled<Tag>>::new(&mut ecs);
        assert!(
            !state.use_dense_seed(),
            "&B include bit routes (&Dn, &B) to the table path"
        );

        let view = ecs.query::<(&Dn, &B), Enabled<Tag>>();
        let got: Vec<u32> = view.iter().map(|(d, _b): (&Dn, &B)| d.v).collect();
        assert_eq!(got, vec![10], "(&Dn, &B) + Enabled yields the enabled row");
    }

    // ── Test #9: AnyOf<(&Dn, &B)> + Enabled<Tag> (D6 open row) ────────────────

    /// The D6 genuine open row: `Query<AnyOf<(&Dn, &B)>, Enabled<Tag>>`. `AnyOf`
    /// has `HAS_DENSE = true` but `HAS_DENSE_INCLUDE = false` (its members are an
    /// OR, not a required include) + `REQUIRES_POST_FILTER_TRIM = true`, so
    /// `IS_DENSE_ENABLE == false` and it is NOT candidate-seeded → it takes the
    /// `else` table branch, which reculls; the cursor also runs the per-member
    /// OR-trim. This test determines whether that interaction yields correct rows.
    ///
    /// OUTCOME (developer-verified): the query yields exactly the enabled rows
    /// that satisfy the `AnyOf` OR-trim, so it is IN-SCOPE and supported (no
    /// shape-assert reject added). See the report for the analysis.
    #[test]
    fn anyof_dense_plus_enable_yields_correct_rows() {
        register();
        let mut ecs = EcsMaster::new();
        // arch_both [B, Dn]: has both members. e0 enabled, e1 disabled.
        let arch_both = ecs.create_archetype(&[B::component_id()]);
        let e0 = spawn(&mut ecs, arch_both, &[B::component_id(), Dn::component_id()], 10);
        let _e1 = spawn(&mut ecs, arch_both, &[B::component_id(), Dn::component_id()], 11);
        // arch_dn [Dn, marker]: has ONLY Dn (of the AnyOf members). e2 enabled.
        let marker = fresh_marker();
        let arch_dn = ecs.create_archetype(&[marker]);
        let e2 = spawn(&mut ecs, arch_dn, &[marker, Dn::component_id()], 20);
        // arch_b [B, marker2]: has ONLY B. e3 enabled.
        let marker2 = fresh_marker();
        let arch_b = ecs.create_archetype(&[B::component_id(), marker2]);
        let e3 = spawn(&mut ecs, arch_b, &[B::component_id(), marker2], 30);

        ecs.enable::<Tag>(e0);
        ecs.enable::<Tag>(e2);
        ecs.enable::<Tag>(e3);

        // Classification witness: AnyOf-dense + enable is NOT the dense-enable
        // shape (HAS_DENSE_INCLUDE = false for AnyOf) ⇒ table path. `const {}` so
        // the classification is proven at compile time (and clippy does not flag a
        // runtime assert on a const).
        const {
            assert!(!QueryDataState::<AnyOf<(&Dn, &B)>, Enabled<Tag>>::IS_DENSE_ENABLE);
        }
        let state = QueryDataState::<AnyOf<(&Dn, &B)>, Enabled<Tag>>::new(&mut ecs);
        assert!(
            !state.use_dense_seed(),
            "AnyOf routes through the table path (REQUIRES_POST_FILTER_TRIM)"
        );

        // Behavioral: enabled AND (has Dn OR has B). e0 (both, on), e2 (Dn, on),
        // e3 (B, on) all qualify; e1 (both, off) is disabled ⇒ excluded.
        let view = ecs.query::<AnyOf<(&Dn, &B)>, Enabled<Tag>>();
        let mut got: Vec<u32> = view
            .iter()
            .map(|(dn, b): (Option<&Dn>, Option<&B>)| match (dn, b) {
                (Some(d), _) => d.v,
                (None, Some(bb)) => bb.w,
                (None, None) => unreachable!("AnyOf yields at least one member"),
            })
            .collect();
        got.sort_unstable();
        assert_eq!(
            got,
            vec![10, 20, 30],
            "AnyOf + Enabled: exactly the enabled rows with ≥1 member (e1 disabled excluded)"
        );
        let _ = (arch_both, arch_dn, arch_b);
    }
}
