use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
use crate::ecs::core::archetype::generation::ArchetypeGeneration;
use crate::ecs::core::component::component_mask::ComponentMask;
use crate::ecs::core::component::component_registry::{
    GPU_COMPONENT_SET_WORDS, gpu_component_set_word,
};
use crate::ecs::core::iters::archetype_bit_set::ArchetypeBitSet;
use crate::ecs::core::iters::component_set::ComponentSet;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

/// Persistent archetype-match cache for hot-path query iteration.
///
/// Unlike `LegacyQuery<'a>` (which rebuilds its archetype list on every construction),
/// `QueryState` is long-lived and caches the result across frames. On the warm
/// path — when no new archetypes have been created — `iter()` costs one pointer
/// load + comparison. The delta update path classifies only newly minted archetypes.
///
/// # Layout rationale
/// `#[repr(C, align(64))]` places the hot fields (`generation`, `matched_ids`)
/// in cache line 0. The three filter masks (192 B) and the dedup bitset (128 B)
/// occupy later cache lines and are only touched on cache misses.
///
/// # Generation pair and `clear()` / `remove_archetype()`
///
/// `QueryState` snapshots both `archetype_generation` (bumped on every
/// `create_archetype`) and `structural_generation` (bumped on every
/// `remove_archetype` and on `clear()`). On the next `iter()`:
///
/// - `structural_generation` mismatched → drop the dedup bitset + matched_ids,
///   re-classify every live archetype from scratch. This is the load-bearing
///   piece of the ArchetypeId-ABA fix: without it, a recycled `ArchetypeId`
///   (e.g. after `clear()` resets `next_archetype_id` to 1) would be skipped
///   by the stale dedup bitset and silently absent from query results.
/// - Only `archetype_generation` mismatched → delta-add path: skip already-seen
///   ids via the bitset, classify only the new ids. Preserves the warm-path
///   ~21x speedup over rebuilding `LegacyQuery` from scratch.
///
/// A `QueryState` is therefore safe to keep alive across `clear()` and
/// `remove_archetype()` calls — no manual `reset()` is required for
/// correctness. `reset()` remains available for callers that want to drop
/// capacity or rebuild the filter explicitly.
#[repr(C, align(64))]
pub struct QueryState {
    // Cache line 0 (hot): read on every iter() / update_archetypes() early-exit check.
    generation: ArchetypeGeneration,
    /// Last observed `master.structural_generation()` — bumps on every
    /// archetype removal or `clear()`. A mismatch forces a full rebuild
    /// instead of the cheap delta-add path, eliminating the ArchetypeId-ABA
    /// hazard documented on `ArchetypeMaster::structural_generation`.
    structural_generation: ArchetypeGeneration,
    /// # Phase 22 D4 — the `_pre_terms` module boundary
    ///
    /// This is the SHARED, term-agnostic archetype-match cache. Dynamic-tag
    /// terms (`with_tag` / `without_tag`) are per-view state applied at each
    /// driver's archetype-transition point and NEVER mutate this cache (the
    /// QS1 dual-structure invariant stays intact — the cache is shared across
    /// all instances of a `(D, F)` query type). Every accessor that exposes
    /// this list carries the `_pre_terms` suffix, so a consumer outside this
    /// module cannot read the matched list without consciously typing
    /// `_pre_terms`; inside this module the private field is touched only by
    /// the QS1 cache-maintenance code, which is pre-terms by definition.
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
    ///
    /// # Phase 5 W1 — CPU query over a Gpu component (release-present panic)
    ///
    /// If `include` names ANY component whose `residency_class` is
    /// [`ResidencyKind::Gpu`](crate::ecs::core::component::component_registry::ResidencyKind::Gpu),
    /// this panics LOUDLY. A device component is absent from the CPU surface
    /// (its archetype is GPU-resident and skipped at collection time), so such a
    /// query would silently match nothing — a footgun caught here, at build, not
    /// in the hot loop. Only `include` is checked (`exclude` / `optional` are
    /// harmless — they never produce a device read). The check is the single
    /// funnel for every typed / legacy query construction. 0%-gate: a CPU-only
    /// world leaves `GPU_COMPONENT_SET` all-zero, so the intersect is `0` and the
    /// branch predicts not-taken; the collection loop is untouched.
    ///
    /// ## W1 ordering contract (FIX-4)
    ///
    /// The W1 diagnostic is a ONE-SHOT read of `GPU_COMPONENT_SET`, so it is
    /// guaranteed to fire only when every `Gpu` component is classified BEFORE
    /// any `QueryState` naming it is constructed. Typed queries satisfy this by
    /// construction (`init_state` resolves `component_id()`, which installs the
    /// residency class and sets the bit, before any scan). The raw-mask paths
    /// `Query::with_mask` /
    /// `Query::with_filters` copy a caller-built mask WITHOUT resolving ids, so a
    /// `Gpu` component classified AFTER such a `QueryState` is built would be
    /// missed — raw-mask callers must honor the same registration-before-query
    /// ordering. A missed W1 is NOT a soundness hole: `update_archetypes`'
    /// `is_gpu_resident()` skip is stamped at archetype mint (timing-independent),
    /// so soundness is upheld unconditionally — a missed W1 degrades only to
    /// "silently matches nothing", never UB.
    pub fn new(include: ComponentMask, exclude: ComponentMask, optional: ComponentMask) -> Self {
        // W1 footgun: a CPU query naming a Gpu component is a build-time error.
        // `MAX_COMPONENTS / 64` words of bitwise-AND, cold; non-zero ⇒ a named
        // include bit is a Gpu component.
        for w in 0..GPU_COMPONENT_SET_WORDS {
            if include.block(w).value() & gpu_component_set_word(w) != 0 {
                cpu_query_over_gpu_component_panic();
            }
        }
        Self {
            generation: ArchetypeGeneration::FIRST,
            structural_generation: ArchetypeGeneration::FIRST,
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
    /// # Phase 22 D4 — pre-terms
    /// Iterates the raw matched list **term-agnostically**; reserved for the
    /// legacy surface, benches, and cache maintenance. Dynamic-tag terms are
    /// applied by the typed drivers only (the D4 funnel).
    ///
    /// # `clear()` / `remove_archetype()` interaction
    /// Safe across both. The structural_generation mismatch detected here
    /// triggers a full rebuild inside `update_archetypes`, which correctly
    /// handles recycled `ArchetypeId`s by re-classifying the live archetype
    /// set against this state's filter.
    pub fn iter_pre_terms<'a>(&'a mut self, master: &'a ArchetypeMaster) -> QueryStateIter<'a> {
        debug_assert!(
            self.generation <= master.archetype_generation(),
            "QueryState.generation ({:?}) > master.archetype_generation() ({:?}); \
             likely cause: master.clear() was called while this QueryState was alive",
            self.generation,
            master.archetype_generation(),
        );
        debug_assert!(
            self.structural_generation <= master.structural_generation(),
            "QueryState.structural_generation ({:?}) > master.structural_generation() ({:?})",
            self.structural_generation,
            master.structural_generation(),
        );
        if self.structural_generation != master.structural_generation()
            || self.generation != master.archetype_generation()
        {
            self.update_archetypes(master);
        }
        // `iter_cached_pre_terms` is valid because update_archetypes() just
        // synced both gens.
        self.iter_cached_pre_terms(master)
    }

    /// Brings the cache in sync with the master's current archetype set.
    ///
    /// Two paths:
    ///
    /// 1. **Structural change** (`structural_generation` bumped — archetypes
    ///    were removed, or `clear()` was called): full rebuild. The cached
    ///    bitset + matched_ids are dropped, then every live archetype ID is
    ///    re-classified against the filter. This is the load-bearing piece of
    ///    the ArchetypeId-ABA fix — without it, a freshly created archetype
    ///    reusing a recycled ID would be skipped by the dedup bitset and
    ///    invisibly absent from query results.
    ///
    /// 2. **Creation-only delta** (`generation` bumped, `structural_generation`
    ///    unchanged): delta-add. IDs already recorded in `matched_archetypes`
    ///    are skipped in O(1); only truly new IDs are tested against the
    ///    filter. This is the original warm-ish path that preserves the
    ///    "create many, read many" benchmark profile (~21x speedup over
    ///    rebuilding `LegacyQuery` from scratch).
    pub fn update_archetypes(&mut self, master: &ArchetypeMaster) {
        let current_gen = master.archetype_generation();
        let current_struct = master.structural_generation();
        debug_assert!(
            self.generation <= current_gen,
            "QueryState.generation ({:?}) > master.archetype_generation() ({:?}); \
             master.clear() was called without resetting this QueryState",
            self.generation,
            current_gen,
        );
        debug_assert!(
            self.structural_generation <= current_struct,
            "QueryState.structural_generation ({:?}) > master.structural_generation() ({:?})",
            self.structural_generation,
            current_struct,
        );

        if self.structural_generation != current_struct {
            // Structural change: drop the dedup bitset and matched_ids; rebuild
            // from scratch. `matched_archetypes.clear_all()` zeroes the 128 B
            // bitset; `matched_ids.clear()` drops length without releasing
            // capacity, keeping the next iter's push amortised O(1).
            self.matched_archetypes.clear_all();
            self.matched_ids.clear();
        } else if self.generation == current_gen {
            // No change at all — caller raced us; nothing to do.
            return;
        }

        // Iterate all archetype IDs from 1..current_gen.get() and skip
        // already-seen ones via the bitset. After a structural rebuild the
        // bitset is empty, so every live ID is freshly classified. After a
        // creation-only delta the bitset preserves prior decisions, so only
        // new IDs reach the `matches(mask)` check.
        //
        // The loop bound is O(current_gen.get()) per call. For a delta path
        // most iterations short-circuit on the bitset hit; for a structural
        // rebuild every live ID is tested.
        for id in 1..current_gen.get() {
            if !self.matched_archetypes.contains(id)
                && let Some(arch) = master.get_archetype(ArchetypeId(id))
            {
                // Phase 5 D7 / MF-4 — SOUNDNESS PREREQUISITE: a CPU `Query` must
                // never collect a GPU-resident archetype. Its component pools are
                // device-backed and their inline columns are nulled (C1), so
                // `row_ptr` / `for_each_chunk` on a device pool is never reached.
                // A CPU-only world never sets the bit, so this is one
                // `test`/`jz` on the already-loaded `flags` `u16` that predicts
                // not-taken (the 0%-gate). The W1 check at `new` already rejects
                // a query that NAMES a Gpu component, so this skip only fires for
                // a query whose include set is disjoint from the GPU archetype.
                if arch.flags.is_gpu_resident() {
                    continue;
                }
                let mask = arch.component_mask();
                if self.matches(mask) {
                    self.matched_archetypes.insert(id);
                    self.matched_ids.push(ArchetypeId(id));
                }
                // Unmatched IDs stay out of the bitset. The archetype's
                // component mask is immutable post-creation, so the same
                // filter applied again will not match — there's no value in
                // remembering negative classifications.
            }
        }
        self.generation = current_gen;
        self.structural_generation = current_struct;
    }

    /// Seeds the matched set directly from a bounded candidate archetype
    /// bitset — the EnableTag candidate-seeded global scan (amendment A1.3 /
    /// Step 7a).
    ///
    /// This is the rebuild primitive for a SOLE single `Enabled<A>` /
    /// `Disabled<A>` query (empty include mask, no positive data term). It
    /// **replaces** [`Self::update_archetypes`]'s `1..current_gen` sweep for
    /// this shape — the candidate bitset (`EnablePresence[A]` snapshot) is the
    /// membership predicate, so `self.matches(mask)` is never called.
    ///
    /// # Boundedness (the M2/C2 resolution)
    ///
    /// The walk is popcount-bounded: [`ArchetypeBitSet::for_each_set_bit`]
    /// visits exactly `candidates.popcount()` ids, never `MAX_ARCHETYPES` and
    /// never `current_gen`. `EnablePresence[A]` is structurally `<=` the live
    /// archetype count, so this never degrades into a full-world scan on ANY
    /// path (delta-add OR the structural full-clear below).
    ///
    /// # Two paths (mirroring `update_archetypes`)
    ///
    /// * **Structural change** (`structural_generation` bumped — archetype
    ///   removal / `clear()`): drop the dual structure, then re-push from the
    ///   candidate snapshot. This purges recycled/removed ids exactly like
    ///   `update_archetypes`'s ABA rebuild (amendment A1.5), still
    ///   popcount-bounded.
    /// * **Creation/enable delta** (no structural change): delta-add only the
    ///   newly-present candidates the dedup bitset has not yet recorded.
    ///
    /// Liveness is intersected via `master.get_archetype(id)`: a stale
    /// candidate id whose archetype was removed is skipped (identical to
    /// `update_archetypes`). `push_matched` preserves the QS1 dual-structure
    /// invariant by construction.
    pub(crate) fn seed_from_candidates(
        &mut self,
        candidates: &ArchetypeBitSet,
        master: &ArchetypeMaster,
    ) {
        let current_gen = master.archetype_generation();
        let current_struct = master.structural_generation();
        debug_assert!(
            self.generation <= current_gen,
            "QueryState.generation ({:?}) > master.archetype_generation() ({:?})",
            self.generation,
            current_gen,
        );
        debug_assert!(
            self.structural_generation <= current_struct,
            "QueryState.structural_generation ({:?}) > master.structural_generation() ({:?})",
            self.structural_generation,
            current_struct,
        );

        if self.structural_generation != current_struct {
            // Structural change ⇒ full clear, then re-push from the (bounded)
            // candidate snapshot. Capacity is retained for amortised O(1).
            self.matched_archetypes.clear_all();
            self.matched_ids.clear();
        }

        // Popcount-bounded walk over the candidate bitset. `push_matched`
        // O(1)-skips ids already recorded (the delta-add fast path), so a
        // re-seed with an unchanged candidate set is cheap.
        candidates.for_each_set_bit(|id| {
            let arch_id = ArchetypeId(id);
            if !self.matched_archetypes.contains(id)
                && let Some(arch) = master.get_archetype(arch_id)
            {
                // Phase 5 W2 — a GPU-pure archetype is structurally absent from
                // any `EnablePresence[A]` candidate set: EnableTags are always
                // `StorageKind::Bitset` / `ResidencyKind::Cpu`, so a GPU-pure
                // archetype (all-components-Gpu, C2) carries no enable tag and
                // never enters a candidate bitset. A `continue` here would
                // SILENTLY mask a regression that smuggled a GPU archetype into a
                // candidate set; assert it cannot happen instead. Reuses the
                // existing `get_archetype` Some-binding (zero extra lookup).
                debug_assert!(
                    !arch.flags.is_gpu_resident(),
                    "seed_from_candidates: a GPU-resident archetype must never appear \
                     in an EnableTag candidate set (EnableTags are always Cpu)"
                );
                self.push_matched(arch_id);
            }
        });

        self.generation = current_gen;
        self.structural_generation = current_struct;
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

    /// Returns the number of matched archetypes — **pre-terms** (Phase 22
    /// D4): dynamic-tag terms are per-view and not visible at this layer.
    #[inline]
    pub(crate) fn len_pre_terms(&self) -> usize {
        self.matched_ids.len()
    }

    /// Returns true if no archetypes are matched — **pre-terms** (Phase 22
    /// D4): dynamic-tag terms are per-view and not visible at this layer.
    #[inline]
    pub(crate) fn is_empty_pre_terms(&self) -> bool {
        self.matched_ids.is_empty()
    }

    /// Returns a slice of all matched archetype IDs — **pre-terms** (Phase 22
    /// D4): the raw shared cache, before any per-view dynamic-tag term is
    /// applied. Every driver that consumes this slice MUST apply
    /// `archetype_passes_tag_terms` at its archetype-transition point.
    #[inline]
    pub fn matched_ids_pre_terms(&self) -> &[ArchetypeId] {
        &self.matched_ids
    }

    /// `true` iff the include mask is empty — no table positive bound (Dense
    /// plan D3). A dense-include query with an empty include mask would
    /// otherwise full-scan the world, so the dense-seed path bounds it via
    /// `seed_from_candidates` instead.
    #[inline]
    pub(crate) fn is_empty_include(&self) -> bool {
        self.include.is_empty()
    }

    /// Resets the cache, returning the state to its initial condition.
    ///
    /// After `reset()`, the next `iter()` or `update_archetypes()` will
    /// re-scan all archetypes from scratch.
    pub fn reset(&mut self) {
        self.matched_ids.clear();
        self.matched_archetypes.clear_all();
        self.generation = ArchetypeGeneration::FIRST;
        self.structural_generation = ArchetypeGeneration::FIRST;
    }

    /// Iterates the cached matched IDs without re-checking the generation —
    /// **pre-terms** (Phase 22 D4): term-agnostic raw-cache walk, reserved
    /// for the legacy surface and cache maintenance.
    ///
    /// Requires that `update_archetypes` has already been called and
    /// `self.generation == master.archetype_generation()`.
    #[inline]
    pub(crate) fn iter_cached_pre_terms<'a>(
        &'a self,
        master: &'a ArchetypeMaster,
    ) -> QueryStateIter<'a> {
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

    /// Marks this state as synced with `master`'s current generation pair.
    ///
    /// Call once after manually pre-populating the cache via `push_matched`
    /// (e.g., in `LegacyQuery::from_archetypes` or `LegacyQuery::with_exact_mask`) to
    /// prevent a redundant `update_archetypes` sweep on the next `iter()`.
    ///
    /// Stamps BOTH `generation` and `structural_generation` — otherwise the
    /// first `iter()` would observe a structural mismatch and rebuild,
    /// silently discarding the just-pushed cache contents.
    #[inline]
    pub(crate) fn mark_synced(&mut self, master: &ArchetypeMaster) {
        self.generation = master.archetype_generation();
        self.structural_generation = master.structural_generation();
    }

    // --- Phase 8b Step 5 helpers ---
    //
    // The five accessors below are consumed by `QueryDataState<D, F>`
    // (`iters/query/state.rs`, Phase 8b Step 6). They are `pub(crate)` only
    // — external callers see the public `matched_ids`, `iter`, etc.

    /// Exposes the matched-ids vector for in-place mutation by
    /// `QueryDataState::post_filter_matched` (Phase 8b Step 6).
    ///
    /// # Phase 8b QS1 invariant
    /// `matched_ids` and `matched_archetypes` bitset stay synchronized via
    /// `remove_matched_at` (the only intended mutator). Direct mutation
    /// through this accessor MUST also clear the corresponding bit on
    /// `matched_archetypes`; otherwise QS1 is violated and
    /// `QueryDataState::assert_dual_invariant` will detect the desync in
    /// debug builds.
    ///
    /// Currently consumed only by the `assert_dual_invariant` synthetic-
    /// violation test in `iters/query/state.rs`; future Phase 8b steps may
    /// route additional mutation paths through this accessor (e.g. tick
    /// filters in Phase 10). The `dead_code` allow keeps non-test builds
    /// clippy-clean without dropping the published `pub(crate)` API.
    ///
    /// Phase 22 D4: renamed `_pre_terms` for sweep completeness — this is the
    /// QS1 cache-maintenance writer (delta-update), pre-terms by definition,
    /// and its return derefs to a readable slice.
    #[allow(dead_code)]
    #[inline]
    pub(crate) fn matched_ids_pre_terms_mut(&mut self) -> &mut Vec<ArchetypeId> {
        &mut self.matched_ids
    }

    /// Removes the matched id at dense index `dense_idx` (via `swap_remove`
    /// on `matched_ids`) AND clears the corresponding bit on
    /// `matched_archetypes`. The single safe paired mutator that maintains
    /// the M1/QS1 dual-structure invariant.
    ///
    /// # Panics
    /// Panics if `dense_idx >= matched_ids.len()` (propagated from
    /// `Vec::swap_remove`).
    #[inline]
    pub(crate) fn remove_matched_at(&mut self, dense_idx: usize) {
        let removed_id = self.matched_ids.swap_remove(dense_idx);
        self.matched_archetypes.remove(removed_id.0);
    }

    /// Read-only accessor for the dedup bitset; consumed by
    /// `QueryDataState::assert_dual_invariant` (Phase 8b Step 6) to verify
    /// the M1/QS1 dual-structure invariant.
    ///
    /// `#[allow(dead_code)]` is kept because `assert_dual_invariant` is
    /// `#[cfg(debug_assertions)]`-only — release builds compile away the
    /// only consumer and would otherwise warn.
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn matched_archetypes_bitset(&self) -> &ArchetypeBitSet {
        &self.matched_archetypes
    }

    /// Snapshot of the last-observed master archetype generation. Used by
    /// `QueryDataState::update` to detect that `update_archetypes` synced
    /// the cache against a newly-created archetype.
    #[inline]
    pub(crate) fn last_observed_archetype_generation(&self) -> ArchetypeGeneration {
        self.generation
    }

    /// Snapshot of the last-observed master structural generation. Used by
    /// `QueryDataState::update` to detect that `update_archetypes` rebuilt
    /// the cache after a removal or `clear()`.
    #[inline]
    pub(crate) fn last_observed_structural_generation(&self) -> ArchetypeGeneration {
        self.structural_generation
    }

    /// `true` when this state's cached generations match `master`'s current
    /// generations — i.e. `update_archetypes` has been run against the live
    /// archetype set and no churn has occurred since.
    ///
    /// Phase 22.1 Area A (O1): the term prefilter's cold rebuild arm
    /// (`TermScratch::rebuild_publish`) `debug_assert!`s this so any entry
    /// point that resolves terms without a prior `QueryDataState::update` is
    /// caught — a stale memo would otherwise persist a wrong filtered list
    /// until the next generation bump.
    #[inline]
    pub(crate) fn generations_synced(&self, master: &ArchetypeMaster) -> bool {
        self.generation == master.archetype_generation()
            && self.structural_generation == master.structural_generation()
    }
}

/// Phase 5 W1 — cold reject for a CPU query that NAMES a `Gpu` component in its
/// `include` mask (resolve OQ4: release-present panic, matching the residency
/// reject at archetype mint).
///
/// `#[cold] #[inline(never)]` so the diagnostic stays out of line and the
/// `QueryState::new` intersect check carries only the never-taken branch (the
/// collection loop is untouched — the 0%-gate).
#[cold]
#[inline(never)]
fn cpu_query_over_gpu_component_panic() -> ! {
    panic!(
        "a CPU Query names a ResidencyKind::Gpu component in its `include` set: a \
         device component is absent from the CPU surface (its archetype is \
         GPU-resident and skipped at query collection), so this query would \
         silently match nothing — drive the device column from a GPU-compute \
         system (`.gpu()`) instead, not a CPU Query"
    );
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

    fn setup() -> ArchetypeMaster {
        register_components();
        ArchetypeMaster::new()
    }

    // --- test 700: empty state yields nothing ---

    #[test]
    fn t700_empty_state_iter_yields_nothing() {
        let master = setup();
        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);
        let count = state.iter_pre_terms(&master).count();
        assert_eq!(count, 0);
    }

    // --- test 701: single archetype matched after update ---

    #[test]
    fn t701_single_archetype_match_after_update() {
        let mut master = setup();
        master.create_archetype(&[Pos::component_id(), Vel::component_id()]);

        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);
        state.update_archetypes(&master);

        assert_eq!(state.len_pre_terms(), 1);
        let arch = state.iter_pre_terms(&master).next().expect("one archetype expected");
        assert!(arch.has_component_id(Pos::component_id()));
    }

    // --- test 702: update_archetypes is idempotent ---

    #[test]
    fn t702_update_idempotent() {
        let mut master = setup();
        master.create_archetype(&[Pos::component_id()]);

        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);
        state.update_archetypes(&master);
        let len_first = state.len_pre_terms();

        state.update_archetypes(&master);
        assert_eq!(state.len_pre_terms(), len_first, "second update must be a no-op");
    }

    // --- test 703: delta update classifies only new archetypes ---

    #[test]
    fn t703_delta_update_only_classifies_new_archetypes() {
        let mut master = setup();
        master.create_archetype(&[Pos::component_id(), Vel::component_id()]);
        master.create_archetype(&[Pos::component_id(), Health::component_id()]);

        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);
        state.update_archetypes(&master);
        assert_eq!(state.len_pre_terms(), 2, "both Pos+Vel and Pos+Health should match");

        // Add a third archetype and delta-update
        master.create_archetype(&[Pos::component_id(), Damage::component_id()]);
        state.update_archetypes(&master);
        assert_eq!(state.len_pre_terms(), 3, "third archetype must be picked up on delta");
    }

    // --- test 704: include/exclude/optional filter semantics ---

    #[test]
    fn t704_include_exclude_optional_combinations() {
        let mut master = setup();
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
            state.len_pre_terms(),
            3,
            "filter must match same 3 archetypes as LegacyQuery::test_complex_filtering"
        );
        for arch in state.iter_pre_terms(&master) {
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
        let mut master = setup();
        master.create_archetype(&[Pos::component_id()]);
        master.create_archetype(&[Vel::component_id()]);
        master.create_archetype(&[Pos::component_id(), Vel::component_id()]);
        master.create_archetype(&[Health::component_id()]);
        master.create_archetype(&[Pos::component_id(), Health::component_id()]);

        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);
        state.update_archetypes(&master);

        // For every ID in matched_ids, the bitset must say contains=true
        for &id in state.matched_ids_pre_terms() {
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
                !state.matched_ids_pre_terms().contains(&id),
                "vel-only archetype must not be in matched_ids"
            );
        }
    }

    // --- test 706: reset clears the cache ---

    #[test]
    fn t706_reset_clears_cache() {
        let mut master = setup();
        master.create_archetype(&[Pos::component_id()]);
        master.create_archetype(&[Pos::component_id(), Vel::component_id()]);

        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);
        state.update_archetypes(&master);
        assert_eq!(state.len_pre_terms(), 2, "two archetypes before reset");

        state.reset();
        assert_eq!(state.len_pre_terms(), 0, "len must be 0 after reset");
        assert!(state.is_empty_pre_terms(), "is_empty must be true after reset");

        // After reset the bitset must also be clean: re-run update and verify no dups
        state.update_archetypes(&master);
        assert_eq!(state.len_pre_terms(), 2, "must re-match after reset+update");
    }

    // --- test 707: stale id after remove is skipped ---

    #[test]
    fn t707_stale_id_after_remove_is_skipped() {
        let mut master = setup();
        master.create_archetype(&[Pos::component_id()]);
        let id_to_remove =
            master.create_archetype(&[Pos::component_id(), Vel::component_id()]);
        master.create_archetype(&[Pos::component_id(), Health::component_id()]);

        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);
        state.update_archetypes(&master);
        assert_eq!(state.len_pre_terms(), 3);

        // Remove one archetype from master
        master.remove_archetype(id_to_remove);

        // iter() must skip the now-missing id (get_archetype returns None)
        let count = state.iter_pre_terms(&master).count();
        assert_eq!(count, 2, "stale removed id must be skipped during iteration");
    }

    // --- ABA-prevention via structural_generation ---

    /// Regression for the ArchetypeId-ABA hazard. Without `structural_generation`
    /// bump + full-rebuild path, the following sequence used to leave a stale
    /// id=1 entry in `matched_ids`. After a `clear()` + `create_archetype`
    /// recycling that id with an UNRELATED component set, the query would
    /// silently include the unrelated archetype in its results.
    ///
    /// With the dual-generation fix, the `iter()` after the recycle observes a
    /// `structural_generation` mismatch, drops the dedup bitset, and rebuilds
    /// — correctly classifying the recycled id by its current component mask.
    #[test]
    fn aba_recycled_archetype_id_after_clear_does_not_leak_into_query() {
        let mut master = setup();

        // Phase 1: create a Pos archetype, query for Pos, observe the match.
        let pos_id_v1 = master.create_archetype(&[Pos::component_id()]);
        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);
        let matched_v1: Vec<_> = state.iter_pre_terms(&master).map(|a| a.id()).collect();
        assert_eq!(matched_v1, vec![pos_id_v1], "phase 1: Pos archetype matches");

        // Phase 2: clear master, then create an UNRELATED archetype that will
        // recycle the same numeric id (clear resets next_archetype_id to 1).
        master.clear();
        let vel_id = master.create_archetype(&[Vel::component_id()]);
        assert_eq!(vel_id, pos_id_v1, "precondition: cleared master recycles id=1");

        // Phase 3: iterate the stale QueryState. The ABA hazard would yield
        // the Vel archetype as if it matched the Pos filter. With the fix,
        // the structural_generation mismatch forces a full rebuild and the
        // re-classification rejects Vel for not having Pos.
        let matched_v2: Vec<_> = state.iter_pre_terms(&master).map(|a| a.id()).collect();
        assert!(
            matched_v2.is_empty(),
            "ABA: recycled id with unrelated mask MUST NOT be surfaced by a stale QueryState; \
             got {:?}",
            matched_v2
        );
    }

    // --- Phase 8b Step 5 helpers ---

    #[test]
    fn remove_matched_at_clears_bit_and_swap_removes() {
        let mut master = setup();
        // Three archetypes that all match the Pos filter; matched_ids ends up
        // populated with three distinct ids in insertion order.
        let id_a = master.create_archetype(&[Pos::component_id()]);
        let id_b = master.create_archetype(&[Pos::component_id(), Vel::component_id()]);
        let id_c = master.create_archetype(&[Pos::component_id(), Health::component_id()]);

        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);
        state.update_archetypes(&master);
        assert_eq!(state.matched_ids_pre_terms(), &[id_a, id_b, id_c], "precondition: three matches in insertion order");
        assert_eq!(state.matched_archetypes_bitset().popcount(), 3, "precondition: bitset popcount == 3");

        // Remove the middle id. swap_remove semantics: last (id_c) moves to slot 1.
        state.remove_matched_at(1);

        // (a) length dropped by exactly one.
        assert_eq!(state.matched_ids_pre_terms().len(), 2, "len must decrement by 1");
        // (b) swap_remove behaviour: id_c (was last) now sits at index 1; id_a unchanged at 0.
        assert_eq!(state.matched_ids_pre_terms()[0], id_a, "slot 0 must be unchanged");
        assert_eq!(state.matched_ids_pre_terms()[1], id_c, "swap_remove must move last element to vacated slot");
        // (c) bitset bit for the removed id (id_b) is cleared; the survivors stay set.
        assert!(!state.matched_archetypes_bitset().contains(id_b.0), "removed id bit must be cleared");
        assert!(state.matched_archetypes_bitset().contains(id_a.0), "survivor id_a bit must remain set");
        assert!(state.matched_archetypes_bitset().contains(id_c.0), "survivor id_c bit must remain set");
        assert_eq!(state.matched_archetypes_bitset().popcount(), 2, "popcount must match new len");
    }

    #[test]
    fn last_observed_generations_return_current_snapshot() {
        let mut master = setup();
        master.create_archetype(&[Pos::component_id()]);

        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);
        // Before any sync: state still carries FIRST.
        assert_eq!(state.last_observed_archetype_generation(), ArchetypeGeneration::FIRST);
        assert_eq!(state.last_observed_structural_generation(), ArchetypeGeneration::FIRST);

        state.update_archetypes(&master);
        // After update: snapshots must equal what update wrote into the fields,
        // which themselves must equal the master's current generation pair.
        assert_eq!(
            state.last_observed_archetype_generation(),
            master.archetype_generation(),
            "archetype-generation snapshot must match master after sync",
        );
        assert_eq!(
            state.last_observed_structural_generation(),
            master.structural_generation(),
            "structural-generation snapshot must match master after sync",
        );

        // Mutating master (new archetype) must NOT auto-update the state's snapshot —
        // the accessors return the last-observed values, not live master values.
        let pre_arch_gen = state.last_observed_archetype_generation();
        master.create_archetype(&[Vel::component_id()]);
        assert_eq!(
            state.last_observed_archetype_generation(),
            pre_arch_gen,
            "snapshot must not change without an explicit update_archetypes call",
        );
        assert_ne!(
            state.last_observed_archetype_generation(),
            master.archetype_generation(),
            "master must have advanced past the stored snapshot",
        );
    }

    /// Mirror of the above without the `clear()` step — verifies that a plain
    /// `remove_archetype` between iterations correctly evicts the dead id
    /// from `matched_ids`, not just from the get_archetype skip path.
    #[test]
    fn remove_archetype_purges_dead_id_from_matched_ids_via_rebuild() {
        let mut master = setup();
        let id_a = master.create_archetype(&[Pos::component_id()]);
        let id_b = master.create_archetype(&[Pos::component_id(), Vel::component_id()]);
        let mut state = QueryState::with_component_ids(&[Pos::component_id()]);

        let v1: Vec<_> = state.iter_pre_terms(&master).map(|a| a.id()).collect();
        assert_eq!(v1.len(), 2, "phase 1: both Pos archetypes match");
        assert!(v1.contains(&id_a) && v1.contains(&id_b));

        master.remove_archetype(id_a);

        let v2: Vec<_> = state.iter_pre_terms(&master).map(|a| a.id()).collect();
        assert_eq!(v2, vec![id_b], "phase 2: removed id_a is gone, id_b remains");

        // Crucially, matched_ids itself was rebuilt — not just filtered during
        // iteration. Length is exactly 1, not 2-with-skip.
        assert_eq!(state.len_pre_terms(), 1, "matched_ids must be physically purged, not skip-on-iter");
    }

    // ----- Phase 5 (MF-4 / D7) — GPU-resident archetype query skip + W1 -----
    //
    // Component ids 346/347 reserved for the GPU-residency query tests (the free
    // 345-399 gap; 345 is taken by archetype.rs's residency tests, the
    // RESIDENCY_CLASS / LAYOUTS tables are process-global across the test binary).
    // Both are GPU-pure, so a `[346, 347]` archetype is GPU-resident (C2).
    #[repr(C)]
    struct GpuA(u32);
    #[repr(C)]
    struct GpuB(u32);

    const GPU_A_ID: ComponentId = ComponentId(346);
    const GPU_B_ID: ComponentId = ComponentId(347);

    fn register_gpu_components() {
        use crate::ecs::core::component::component_registry::ResidencyKind;
        component_registry::register_layout::<GpuA>(GPU_A_ID.0);
        component_registry::register_layout::<GpuB>(GPU_B_ID.0);
        component_registry::set_residency_class(GPU_A_ID.0, ResidencyKind::Gpu);
        component_registry::set_residency_class(GPU_B_ID.0, ResidencyKind::Gpu);
    }

    /// A GPU-pure (`GPU_RESIDENT`) archetype is SKIPPED by the CPU query
    /// collection — `update_archetypes` never adds it to `matched_ids`, so
    /// `row_ptr` on its device pools is never reached (the C1 / D7 soundness
    /// prerequisite). An empty-filter `QueryState` matches every CPU archetype
    /// but never the GPU-resident one.
    #[test]
    fn t720_gpu_resident_archetype_is_skipped_by_query() {
        let mut master = setup();
        register_gpu_components();

        // A CPU archetype + a GPU-pure archetype.
        let cpu_id = master.create_archetype(&[Pos::component_id()]);
        let gpu_id = master.create_archetype(&[GPU_A_ID, GPU_B_ID]);
        assert!(
            master
                .get_archetype(gpu_id)
                .expect("gpu archetype exists")
                .flags
                .is_gpu_resident(),
            "the [GpuA, GpuB] archetype must be GPU-resident (C2: all-components-Gpu)"
        );

        // An empty-filter state matches every archetype EXCEPT the GPU-resident
        // one (the skip). It does NOT name any Gpu component, so W1 does not fire.
        let mut state = QueryState::new(
            ComponentMask::new(),
            ComponentMask::new(),
            ComponentMask::new(),
        );
        state.update_archetypes(&master);

        let ids: Vec<_> = state.iter_pre_terms(&master).map(|a| a.id()).collect();
        assert!(ids.contains(&cpu_id), "the CPU archetype must be matched");
        assert!(
            !ids.contains(&gpu_id),
            "the GPU-resident archetype must be SKIPPED at collection (MF-4 / D7)"
        );
    }

    /// W1 footgun: constructing a `QueryState` whose `include` NAMES a
    /// `ResidencyKind::Gpu` component is a release-present panic — a device
    /// component is absent from the CPU surface, so the query would silently
    /// match nothing.
    #[test]
    #[should_panic(expected = "names a ResidencyKind::Gpu component")]
    fn t721_cpu_query_naming_a_gpu_component_panics() {
        register_gpu_components();
        let mut include = ComponentMask::new();
        include.set(GPU_A_ID);
        let _ = QueryState::new(include, ComponentMask::new(), ComponentMask::new());
    }

    /// An `exclude` mask naming a Gpu component is HARMLESS — only `include`
    /// triggers the W1 panic (exclude/optional never produce a device read).
    #[test]
    fn t722_exclude_naming_a_gpu_component_is_harmless() {
        register_gpu_components();
        let mut exclude = ComponentMask::new();
        exclude.set(GPU_A_ID);
        // Must NOT panic.
        let _ = QueryState::new(ComponentMask::new(), exclude, ComponentMask::new());
    }
}
