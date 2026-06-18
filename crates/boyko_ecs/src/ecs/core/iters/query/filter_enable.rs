//! EnableTag query filters: [`Enabled<T>`] / [`Disabled<T>`] (Decision D2 /
//! Step 7).
//!
//! Both are **non-archetypal per-row** [`QueryFilter`]s over the bitset storage
//! backend (`#[component(storage = "bitset")]` / `register_enable_tag`). Unlike
//! the signature-storage filters (`With` / `Without` / `Added` / `Changed`),
//! an EnableTag is NOT part of an archetype's signature mask and has no
//! `ComponentPool`; its bit lives in the archetype's
//! `EnableStore`
//! at `(archetype, row)`. The per-row predicate tests that bit.
//!
//! # Shape (Decision D2)
//!
//! ```text
//! IS_ARCHETYPAL            = false   // activates the per-row filter_fetch branch
//! NEEDS_CHANGE_DETECTION   = false   // no tick-meta path (the _no_meta body is real)
//! CONTAINS_ENABLE_TERM     = true    // (D, F) shape const-assert input (Step 7a)
//! IS_SOLE_SINGLE_ENABLE    = true    // a single leaf — candidate-seedable (amendment A3.3)
//! HAS_POSITIVE_ARCHETYPAL  = false   // an enable term is NOT a positive include bit
//! CONTAINS_CHANGE_DETECTION= false   // not a change-detection term
//! ```
//!
//! # `init_access` — explicit no-op (ENBL-ACCESS-1, structural — Decision C1/D8)
//!
//! Neither filter declares any component access. The precedent is
//! [`Without<C>`](super::filter::Without) (the ONLY other no-op leaf), NOT
//! `With`/`Added`/`Changed` (which all declare a read). See the SAFETY block on
//! each `init_access` for the structural soundness argument.
//!
//! # `Or` rejection (M1)
//!
//! Neither filter implements the sealed
//! `OrComposable` marker, so
//! `Or<(Enabled<A>, ..)>` is a compile error: `Or` folds a non-archetypal
//! per-row test against an archetypal element's unconditional `true`, which
//! would leak disabled rows.
//!
//! # `for_each_chunk` rejection
//!
//! Neither filter implements
//! [`ArchetypalQueryFilter`](super::filter::ArchetypalQueryFilter), so the
//! chunk API (which yields one slice per archetype with no per-row gate)
//! rejects them at the bound.

use std::marker::PhantomData;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::archetype::archetype_master::ArchetypeMaster;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry::{self, StorageKind};
use crate::ecs::core::component::enable::enable_store::EnableColumn;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

use super::filter::QueryFilter;

// ── Per-system state ─────────────────────────────────────────────────────────

/// Per-system cached state for [`Enabled<T>`]: the resolved bitset [`ComponentId`].
#[derive(Clone, Copy)]
pub struct EnabledState<T> {
    pub(crate) id: ComponentId,
    _marker: PhantomData<fn() -> T>,
}

/// Per-system cached state for [`Disabled<T>`]: the resolved bitset [`ComponentId`].
#[derive(Clone, Copy)]
pub struct DisabledState<T> {
    pub(crate) id: ComponentId,
    _marker: PhantomData<fn() -> T>,
}

// ── Per-archetype fetch ──────────────────────────────────────────────────────

/// Per-archetype `Fetch` scratch shared by both enable filters: a borrowed
/// pointer to the active archetype's `EnableColumn` for the tag, or NULL.
///
/// NULL means the archetype has no allocated column for the tag — every row is
/// disabled. `Enabled::filter_fetch` reads NULL as `false`; `Disabled` inverts
/// it to `true` (Decision D2 / amendment A1.1: a no-column row is "disabled").
///
/// `set_table_*` refreshes the pointer per archetype (mirrors the `Added`/
/// `Changed` `tick_base` discipline). The per-page deref happens inside
/// `EnableColumn::test`; the cursor hoists the loop-invariant word load.
pub struct EnableFetch<'w> {
    /// Base pointer to the active archetype's column for the tag. NULL when the
    /// archetype has no column (all rows disabled).
    pub(crate) col: *const EnableColumn,
    /// Ties the fetch lifetime to `'w` (the archetype-pointer lifetime).
    pub(crate) _marker: PhantomData<&'w ()>,
}

impl Clone for EnableFetch<'_> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for EnableFetch<'_> {}

/// Caches the archetype's [`EnableColumn`] pointer (or NULL) into `fetch`.
///
/// # Safety
///
/// `archetype` MUST be a live `*const Archetype` for the fetch lifetime `'w`
/// with provenance from `UnsafeEcsCell::archetype_ptr(id)` (the `set_table_*`
/// caller contract). The returned pointer borrows the archetype's `EnableStore`
/// and is re-read by every `set_table_*` (so a directory regrow under a `&mut`
/// apply window — where no fetch is live — never leaves it dangling).
#[inline]
unsafe fn cache_column(col: &mut *const EnableColumn, id: ComponentId, archetype: *const Archetype) {
    // SAFETY: per the caller contract `archetype` is a live `*const Archetype`
    //   for `'w`; the shared reborrow is confined to this call. `enable_column_ptr`
    //   reads the archetype's `EnableStore` (an `&self` scan ≤ 4) and returns a
    //   borrowed `*const EnableColumn` (or NULL) whose provenance is the stable,
    //   interior-mutable column storage — valid for `'w` because the archetype
    //   outlives the fetch.
    let archetype_ref: &Archetype = unsafe { &*archetype };
    *col = archetype_ref.enable_column_ptr(id);
}

// ── Enabled<T> ───────────────────────────────────────────────────────────────

/// Per-row filter: matches rows whose EnableTag `T` bit is **set**
/// (Decision D2). Non-archetypal — the per-row `filter_fetch` tests the bit at
/// `(archetype, row)` via the archetype's `EnableStore`.
///
/// `Enabled<T>` requires a positive archetypal term to bound iteration, OR is a
/// sole single term (candidate-seeded — amendment A3) — enforced at the
/// `(D, F)` construction seam (Step 7a). It cannot appear in `Or<>` (M1),
/// cannot be combined with `Added`/`Changed` in one query (C3), and cannot be
/// used with `for_each_chunk`.
///
/// To read `T`'s data, do NOT pair `Enabled<T>` with `&T` — a bitset tag has no
/// `ComponentPool`. `Enabled<T>` only gates rows; pair it with the real data
/// components you want to read, e.g. `Query<&Pos, Enabled<Stunned>>`.
pub struct Enabled<T: Component> {
    _marker: PhantomData<fn() -> T>,
}

// SAFETY (QF1, QF2, QF3):
//   - QF1: `IS_ARCHETYPAL = false`; `filter_fetch` performs a real per-row
//     bit test (NULL-column short-circuit to `false`).
//   - QF2: `init_access` declares NOTHING (ENBL-ACCESS-1 — structural).
//   - QF3: `Fetch<'w>` holds a `*const EnableColumn` scoped to `'w` via
//     `PhantomData<&'w ()>`, refreshed by every `set_table_*` before any
//     `filter_fetch`.
unsafe impl<T: Component> QueryFilter for Enabled<T> {
    type State = EnabledState<T>;
    type Fetch<'w> = EnableFetch<'w>;
    const IS_ARCHETYPAL: bool = false;
    const NEEDS_CHANGE_DETECTION: bool = false;
    // EnableTag D2 / C2: an enable term — the (D, F) shape const-asserts (Step
    // 7a) require it to be bounded by a positive term or to be a sole leaf.
    const CONTAINS_ENABLE_TERM: bool = true;
    // Amendment A3.3: a single Enabled<T> leaf is candidate-seedable.
    const IS_SOLE_SINGLE_ENABLE: bool = true;
    // HAS_POSITIVE_ARCHETYPAL stays default `false`: an enable term contributes
    // NO positive include bit (matches_component_set is unconditional `true`;
    // the cull is a separate pass). CONTAINS_CHANGE_DETECTION stays default
    // `false`.

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        let id = T::component_id();
        debug_assert_eq!(
            component_registry::storage_kind(id.0),
            StorageKind::Bitset,
            "Enabled<{}>: id {} is not a bitset enable tag — use \
             #[component(storage = \"bitset\")] / register_enable_tag",
            std::any::type_name::<T>(),
            id.0,
        );
        EnabledState { id, _marker: PhantomData }
    }

    #[inline]
    fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {
        // ENBL-ACCESS-1: Enable filters declare NO component access in v1.
        //
        // PRECEDENT: this mirrors `Without<C>` (filter.rs — the ONLY other
        // no-op leaf), NOT `With<C>`/`Added<C>`/`Changed<C>` (which DECLARE a
        // read).
        //
        // WHY `With`/`Added`/`Changed` declare a conservative read: a sibling
        // `&mut C` data param CAN exist in the same system (C is in the
        // signature and has a `ComponentPool` the sibling writes), so the
        // intra-system aliasing detector must serialize the filter's logical
        // read of C's lifecycle against that `&mut C`.
        //
        // WHY an EnableTag does NOT: Decision D5 filters the bitset id OUT of
        // every archetype signature and gives it NO `ComponentPool`. Therefore
        // NO `&C`/`&mut C` data param can ever resolve against this id (there
        // is no column to fetch) — a sibling data access on the id is
        // STRUCTURALLY IMPOSSIBLE. With no possible sibling, there is nothing
        // for the aliasing detector to serialize against, exactly as for
        // `Without<C>`'s absence-inspection. Declaring add_component_read(id)
        // would be WRONG: it would manufacture a false conflict with an
        // unrelated system and imply a change-detected read contract the
        // backend does not honor. (D7 worker-marking is the ONLY place that
        // adds an access declaration — a new EnableWrite category, see D8.)
    }

    #[inline]
    fn matches_component_set(
        _state: &Self::State,
        _mask: &crate::ecs::core::component::component_mask::ComponentMask,
    ) -> bool {
        // D2: the per-archetype presence verdict is delivered by the SEPARATE
        // cull pass (Step 7a), not by the signature mask — a bitset id is never
        // in the mask. Always `true` here.
        true
    }

    #[inline]
    fn sole_enable_tag_id(state: &Self::State) -> ComponentId {
        // Amendment A2.1: overrides the trait's unreachable backstop. Called
        // only under `if const { IS_CANDIDATE_SEEDED }` (Step 7a).
        state.id
    }

    #[inline]
    fn enable_cull_keeps_archetype(
        state: &Self::State,
        master: &ArchetypeMaster,
        arch: ArchetypeId,
    ) -> bool {
        // Decision 2: keep `arch` iff it owns an allocated column for the tag.
        // A no-column archetype has every row disabled (A1.1) ⇒ NO `Enabled<T>`
        // row can survive ⇒ drop it. Uses the O(1) presence oracle (one pointer
        // load + one word load + one bit test) rather than minting an
        // `&Archetype` — the oracle is the module's documented cull consumer.
        master.enable_presence().contains(state.id, arch)
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
        EnableFetch { col: std::ptr::null(), _marker: PhantomData }
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
        _meta: &'_ SystemMeta,
    ) {
        // SAFETY (QF3): forwarded to the meta-free body — `NEEDS_CHANGE_DETECTION
        //   = false`, so the column pointer is the only per-archetype state and
        //   `meta` carries nothing this filter reads. `archetype` carries the
        //   caller's read-only provenance.
        unsafe { cache_column(&mut fetch.col, state.id, archetype) }
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        _meta: &'_ SystemMeta,
    ) {
        // SAFETY (QF3): the enable bit is read shared regardless of the
        //   archetype pointer's mutability; reborrow as `*const` and cache. No
        //   write-capable provenance is consumed.
        unsafe { cache_column(&mut fetch.col, state.id, archetype as *const _) }
    }

    #[inline]
    unsafe fn set_table_readonly_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
    ) {
        // W4: the meta-free variant carries the REAL body for this NCD = false
        // filter (the NCD6 const-fold dispatch routes here).
        // SAFETY (QF3): `archetype` carries the caller's read-only provenance.
        unsafe { cache_column(&mut fetch.col, state.id, archetype) }
    }

    #[inline]
    unsafe fn set_table_mut_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    ) {
        // SAFETY (QF3): shared reborrow of a write-capable pointer for a shared
        //   read of the enable bit.
        unsafe { cache_column(&mut fetch.col, state.id, archetype as *const _) }
    }

    #[inline]
    unsafe fn filter_fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
        if fetch.col.is_null() {
            // No column for this archetype ⇒ every row disabled (D2).
            return false;
        }
        // SAFETY (QF3): `fetch.col` is the borrowed column pointer cached by the
        //   prior `set_table_*` for this archetype; it is non-null (checked
        //   above) and valid for `'w` (the archetype outlives the fetch; a
        //   directory regrow runs only inside a `&mut` apply window where no
        //   fetch is live). `EnableColumn::test` does the paged deref + word
        //   load + bit test (`Relaxed`), reading `false` for a never-toggled
        //   page; `row` is in range per the QF3 contract.
        let column: &EnableColumn = unsafe { &*fetch.col };
        column.test(row)
    }
}

// ── Disabled<T> ──────────────────────────────────────────────────────────────

/// Per-row filter: matches rows whose EnableTag `T` bit is **NOT set**
/// (Decision D2). The polarity-inverted twin of [`Enabled<T>`].
///
/// Per amendment A1.1, a row in an archetype with **no column / no page** for
/// `T` reads as disabled ⇒ `Disabled<T>` returns `true` for it. As a
/// positive-term query (`Query<&D, Disabled<A>>`) it therefore visits every
/// D-row whose A-bit is clear, including no-A-column archetypes. (A sole
/// `Query<(), Disabled<A>>` enumerates only present-A archetypes — the
/// candidate-seeded path, Step 7a; the two shapes answer different questions.)
///
/// `Disabled<T>` shares every shape constant and constraint with `Enabled<T>`
/// (positive-term-or-sole, no `Or`, no `Added`/`Changed` mix, no
/// `for_each_chunk`).
pub struct Disabled<T: Component> {
    _marker: PhantomData<fn() -> T>,
}

// SAFETY (QF1, QF2, QF3): identical to `Enabled<T>` except `filter_fetch` is
//   inverted (NULL column / clear bit ⇒ `true`). See the `Enabled<T>` impl for
//   the full reasoning; the only behavioural delta is the predicate polarity.
unsafe impl<T: Component> QueryFilter for Disabled<T> {
    type State = DisabledState<T>;
    type Fetch<'w> = EnableFetch<'w>;
    const IS_ARCHETYPAL: bool = false;
    const NEEDS_CHANGE_DETECTION: bool = false;
    const CONTAINS_ENABLE_TERM: bool = true;
    const IS_SOLE_SINGLE_ENABLE: bool = true;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        let id = T::component_id();
        debug_assert_eq!(
            component_registry::storage_kind(id.0),
            StorageKind::Bitset,
            "Disabled<{}>: id {} is not a bitset enable tag — use \
             #[component(storage = \"bitset\")] / register_enable_tag",
            std::any::type_name::<T>(),
            id.0,
        );
        DisabledState { id, _marker: PhantomData }
    }

    #[inline]
    fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {
        // ENBL-ACCESS-1: no-op, structurally justified — see `Enabled<T>`'s
        // `init_access` for the full argument (a bitset id has no
        // `ComponentPool`, so a sibling data access is structurally impossible
        // ⇒ nothing to serialize ⇒ no-op is correct; precedent `Without<C>`).
    }

    #[inline]
    fn matches_component_set(
        _state: &Self::State,
        _mask: &crate::ecs::core::component::component_mask::ComponentMask,
    ) -> bool {
        true
    }

    #[inline]
    fn sole_enable_tag_id(state: &Self::State) -> ComponentId {
        state.id
    }

    #[inline]
    fn enable_cull_keeps_archetype(
        _state: &Self::State,
        _master: &ArchetypeMaster,
        _arch: ArchetypeId,
    ) -> bool {
        // Decision 2 / A1.1 — EXPLICIT `true`, NOT inherited: a `Disabled<T>`
        // term MUST NOT cull. A no-column archetype has every row "disabled"
        // (no bit ⇒ clear ⇒ matches `Disabled<T>`), so dropping no-column
        // archetypes here would wrongly hide the very rows a positive-term
        // `Query<&D, Disabled<T>>` is meant to visit. The presence cull is a
        // no-op for the disabled polarity; correctness rests on per-row
        // `filter_fetch` (NULL column ⇒ `true`).
        true
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
        EnableFetch { col: std::ptr::null(), _marker: PhantomData }
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
        _meta: &'_ SystemMeta,
    ) {
        // SAFETY (QF3): see `Enabled::set_table_readonly`.
        unsafe { cache_column(&mut fetch.col, state.id, archetype) }
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        _meta: &'_ SystemMeta,
    ) {
        // SAFETY (QF3): see `Enabled::set_table_mut`.
        unsafe { cache_column(&mut fetch.col, state.id, archetype as *const _) }
    }

    #[inline]
    unsafe fn set_table_readonly_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
    ) {
        // W4: real body — see `Enabled::set_table_readonly_no_meta`.
        // SAFETY (QF3): `archetype` carries the caller's read-only provenance.
        unsafe { cache_column(&mut fetch.col, state.id, archetype) }
    }

    #[inline]
    unsafe fn set_table_mut_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    ) {
        // SAFETY (QF3): shared reborrow for a shared enable-bit read.
        unsafe { cache_column(&mut fetch.col, state.id, archetype as *const _) }
    }

    #[inline]
    unsafe fn filter_fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> bool {
        if fetch.col.is_null() {
            // Amendment A1.1: no column ⇒ the row is disabled ⇒ `true`.
            return true;
        }
        // SAFETY (QF3): identical to `Enabled::filter_fetch` (non-null column
        //   cached for this archetype, valid for `'w`, `row` in range); the
        //   only delta is the inverted predicate.
        let column: &EnableColumn = unsafe { &*fetch.col };
        !column.test(row)
    }
}

// NOTE (M1): `Enabled<T>` / `Disabled<T>` deliberately do NOT implement
//   `super::filter::OrComposable`, so `Or<(Enabled<A>, ..)>` is a compile
//   error. They also do NOT implement
//   `super::filter::ArchetypalQueryFilter`, so `for_each_chunk` rejects them.

// ── Point-lookup typed enable test (C3-r5 / Step 9) ──────────────────────────

/// Applies the typed enable filter `F` to a single in-hand `(archetype, row)`
/// for the `QueryView::get` / `get_mut` point-lookup path (Decision C3-r5 /
/// Step 9).
///
/// Today `get` / `get_mut` apply only the archetype-level matched-set
/// membership check, never the per-row `filter_fetch` — so a typed
/// `Enabled<T>` / `Disabled<T>` term would be silently ignored (the C3
/// "compile-but-lie" bug). This helper closes that gap by reusing the same
/// generic `init_fetch` → `set_table_readonly_no_meta` → `filter_fetch`
/// pipeline the cursors run, applied once to the in-hand row.
///
/// The whole helper is gated behind `const { F::CONTAINS_ENABLE_TERM }` by the
/// caller, so it is emitted ONLY for enable-bearing filters — the no-enable
/// point lookup is byte-identical to today (the 0%-gate). It is correct for
/// every `F`:
/// * Archetypal-only `F` (`With` / `Without` / `Or`) has
///   `IS_ARCHETYPAL = true`, so `filter_fetch` returns `true` unconditionally
///   (already enforced at the matched-set membership check) — but those `F`
///   have `CONTAINS_ENABLE_TERM = false` and never reach here.
/// * `Enabled<T>` / `Disabled<T>` (and AND-tuples containing them) run the real
///   per-row bit test.
/// * `Added` / `Changed` cannot be mixed with an enable term (C3-r7 compile
///   reject), so any `F` reaching here has `NEEDS_CHANGE_DETECTION = false`;
///   the meta-free `set_table_readonly_no_meta` route is correct and never hits
///   the `_no_meta` backstop. Change-detection filters on their own are NOT
///   applied by point lookups (BUG-ENABLE-PRE-2, documented).
///
/// # Safety
///
/// `archetype` MUST be a live `*const Archetype` for the duration of this call
/// (the caller's slab-stable pointer), with the archetype already confirmed to
/// be in the query's matched set. `row` MUST be in range
/// (`row < archetype.entity_count()`).
#[inline]
pub(crate) unsafe fn query_view_enable_passes<F: QueryFilter>(
    filter_state: &F::State,
    archetype: *const Archetype,
    row: usize,
) -> bool {
    let mut fetch = <F as QueryFilter>::init_fetch(filter_state);
    // SAFETY (ENBL-PT): `archetype` is the caller's live, matched, slab-stable
    //   `*const Archetype`. Any `F` reaching here has
    //   `NEEDS_CHANGE_DETECTION = false` (C3-r7 forbids the enable+change mix),
    //   so the meta-free table refresh is the correct, panic-free route — it
    //   caches the enable column pointer (or NULL) for this archetype into
    //   `fetch`.
    unsafe {
        <F as QueryFilter>::set_table_readonly_no_meta(&mut fetch, filter_state, archetype);
    }
    // SAFETY (ENBL-PT): `set_table_readonly_no_meta` cached the per-archetype
    //   fetch above; `row < entity_count()` per the caller contract. For an
    //   enable filter this tests the bit at `(archetype, row)` (Enabled: set;
    //   Disabled: clear) — identical to the cursor's per-row gate.
    unsafe { <F as QueryFilter>::filter_fetch(&fetch, row) }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::*;
    use crate::ecs::core::component::component_mask::ComponentMask;
    use crate::ecs::core::iters::query::filter::{Changed, Or, QueryFilter, With, Without};
    use crate::ecs::identifiers::primitives::ComponentId;

    // Test components lazy-mint their ids via `register_new` (the same path the
    // `#[derive(Component)]` macro uses externally), so they never collide with
    // the fixed-id blocks other test binaries reserve in the shared lib-test
    // process. The `#[derive(Component)]` macro emits `boyko_ecs::..` paths that
    // do not resolve inside the crate itself, so the impls are hand-written —
    // mirroring `enable_tag_api.rs` / `migration_helpers.rs` test fixtures. The
    // tag types (`A`, `B`) are classified `StorageKind::Bitset` at runtime in
    // `register_components`.
    #[repr(C)]
    struct A;
    impl Component for A {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| ComponentId(component_registry::register_new::<A>()))
        }
    }
    #[repr(C)]
    struct B;
    impl Component for B {
        fn component_id() -> ComponentId {
            static ID: OnceLock<ComponentId> = OnceLock::new();
            *ID.get_or_init(|| ComponentId(component_registry::register_new::<B>()))
        }
    }
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

    // ── Golden if-const matrix (D2 shape) ────────────────────────────────────

    /// `Enabled`/`Disabled` are non-archetypal, change-detection-free per-row
    /// filters that ARE enable terms and ARE sole-single-enable leaves, are NOT
    /// positive archetypal terms, and are NOT change-detection terms.
    ///
    /// `const { assert!(..) }` so the golden matrix is a per-monomorphisation
    /// compile-time guarantee — the same const-eval mechanism Step 7a's shape
    /// asserts consume.
    #[test]
    fn golden_shape_consts() {
        const {
            assert!(!<Enabled<A> as QueryFilter>::IS_ARCHETYPAL);
            assert!(!<Enabled<A> as QueryFilter>::NEEDS_CHANGE_DETECTION);
            assert!(<Enabled<A> as QueryFilter>::CONTAINS_ENABLE_TERM);
            assert!(<Enabled<A> as QueryFilter>::IS_SOLE_SINGLE_ENABLE);
            assert!(!<Enabled<A> as QueryFilter>::HAS_POSITIVE_ARCHETYPAL);
            assert!(!<Enabled<A> as QueryFilter>::CONTAINS_CHANGE_DETECTION);

            assert!(!<Disabled<A> as QueryFilter>::IS_ARCHETYPAL);
            assert!(!<Disabled<A> as QueryFilter>::NEEDS_CHANGE_DETECTION);
            assert!(<Disabled<A> as QueryFilter>::CONTAINS_ENABLE_TERM);
            assert!(<Disabled<A> as QueryFilter>::IS_SOLE_SINGLE_ENABLE);
            assert!(!<Disabled<A> as QueryFilter>::HAS_POSITIVE_ARCHETYPAL);
            assert!(!<Disabled<A> as QueryFilter>::CONTAINS_CHANGE_DETECTION);
        }
    }

    /// An AND-tuple OR-folds the shape consts: `(With<P>, Enabled<A>)` has a
    /// positive archetypal term AND an enable term, but is NOT a single leaf.
    #[test]
    fn golden_shape_consts_and_tuple() {
        const {
            // With<P> contributes the positive archetypal term; Enabled<A> the
            // enable term; a tuple is never a single enable leaf; non-archetypal
            // because Enabled<A> is.
            assert!(<(With<P>, Enabled<A>) as QueryFilter>::HAS_POSITIVE_ARCHETYPAL);
            assert!(<(With<P>, Enabled<A>) as QueryFilter>::CONTAINS_ENABLE_TERM);
            assert!(!<(With<P>, Enabled<A>) as QueryFilter>::IS_SOLE_SINGLE_ENABLE);
            assert!(!<(With<P>, Enabled<A>) as QueryFilter>::CONTAINS_CHANGE_DETECTION);
            assert!(!<(With<P>, Enabled<A>) as QueryFilter>::IS_ARCHETYPAL);
        }
    }

    /// `(Changed<B>, Enabled<A>)` reports BOTH a change-detection term AND an
    /// enable term — the C3 const-assert input that Step 7a will reject.
    #[test]
    fn golden_shape_consts_change_plus_enable() {
        const {
            assert!(<(Changed<B>, Enabled<A>) as QueryFilter>::CONTAINS_ENABLE_TERM);
            assert!(<(Changed<B>, Enabled<A>) as QueryFilter>::CONTAINS_CHANGE_DETECTION);
            assert!(!<(Changed<B>, Enabled<A>) as QueryFilter>::IS_SOLE_SINGLE_ENABLE);
        }
    }

    /// Existing filters keep their defaults: no enable term, no false-positive
    /// positive-archetypal classification on `Without`.
    #[test]
    fn golden_shape_consts_existing_filters() {
        const {
            assert!(!<() as QueryFilter>::CONTAINS_ENABLE_TERM);
            assert!(!<() as QueryFilter>::HAS_POSITIVE_ARCHETYPAL);
            assert!(!<() as QueryFilter>::IS_SOLE_SINGLE_ENABLE);

            assert!(<With<P> as QueryFilter>::HAS_POSITIVE_ARCHETYPAL);
            assert!(!<With<P> as QueryFilter>::CONTAINS_ENABLE_TERM);

            assert!(!<Without<P> as QueryFilter>::HAS_POSITIVE_ARCHETYPAL);
            assert!(!<Without<P> as QueryFilter>::CONTAINS_ENABLE_TERM);

            assert!(<Changed<B> as QueryFilter>::CONTAINS_CHANGE_DETECTION);
            assert!(!<Changed<B> as QueryFilter>::CONTAINS_ENABLE_TERM);

            // An Or of existing composable filters still compiles and folds —
            // the M1 regression guard (the OrComposable bound did not break
            // existing Or filters).
            assert!(<Or<(With<P>, Changed<B>)> as QueryFilter>::CONTAINS_CHANGE_DETECTION);
            assert!(!<Or<(With<P>, Changed<B>)> as QueryFilter>::CONTAINS_ENABLE_TERM);
        }
    }

    /// `matches_component_set` is unconditionally `true` for both filters (the
    /// presence verdict is the separate cull pass — Step 7a).
    #[test]
    fn matches_component_set_is_unconditional() {
        let empty = ComponentMask::new();
        let id = A::component_id();
        let es = EnabledState::<A> { id, _marker: PhantomData };
        let ds = DisabledState::<A> { id, _marker: PhantomData };
        assert!(<Enabled<A> as QueryFilter>::matches_component_set(&es, &empty));
        assert!(<Disabled<A> as QueryFilter>::matches_component_set(&ds, &empty));
    }

    /// `sole_enable_tag_id` returns the resolved tag id (overrides the trait's
    /// unreachable backstop — amendment A2.1).
    #[test]
    fn sole_enable_tag_id_returns_state_id() {
        let a_id = A::component_id();
        let b_id = B::component_id();
        let es = EnabledState::<A> { id: a_id, _marker: PhantomData };
        let ds = DisabledState::<B> { id: b_id, _marker: PhantomData };
        assert_eq!(<Enabled<A> as QueryFilter>::sole_enable_tag_id(&es), a_id);
        assert_eq!(<Disabled<B> as QueryFilter>::sole_enable_tag_id(&ds), b_id);
    }

    // ── Typed iteration behaviour (in-crate: needs `set_storage_kind`) ────────

    /// Classifies the tag types `A`/`B` as `StorageKind::Bitset`. The derived
    /// `component_id()` lazily registers each type's layout on first call, so no
    /// explicit `register_layout` is needed (Step-10's macro will additionally
    /// emit the `set_storage_kind` call these tests make by hand).
    fn register_components() {
        component_registry::set_storage_kind(A::component_id().0, StorageKind::Bitset);
        component_registry::set_storage_kind(B::component_id().0, StorageKind::Bitset);
    }

    /// Spawns one `P { v }` entity into `arch` (a `[P]` archetype) and returns
    /// the [`Entity`].
    fn spawn_p(
        ecs: &mut EcsMaster,
        arch: crate::ecs::identifiers::primitives::ArchetypeId,
        v: u32,
    ) -> crate::ecs::core::entity::entity::Entity {
        let p = P { v };
        // SAFETY (test): `p` outlives the borrow; byte view of a #[repr(C)] POD.
        let bytes = unsafe {
            core::slice::from_raw_parts(
                &p as *const P as *const u8,
                core::mem::size_of::<P>(),
            )
        };
        ecs.create_entity(arch, &[(P::component_id(), bytes)])
            .expect("spawn must succeed")
    }

    /// `Query<&P, Enabled<A>>` yields ONLY the rows whose `A` bit is set. The
    /// per-row `filter_fetch` drives this even before Step 7a's cull pass lands
    /// (correctness comes from `filter_fetch`; the cull is an optimization).
    #[test]
    fn query_enabled_yields_only_enabled_rows() {
        register_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[P::component_id()]);
        let e0 = spawn_p(&mut ecs, arch, 10);
        let _e1 = spawn_p(&mut ecs, arch, 11);
        let e2 = spawn_p(&mut ecs, arch, 12);

        // Enable A on e0 and e2 only.
        ecs.enable::<A>(e0);
        ecs.enable::<A>(e2);

        let view = ecs.query::<&P, Enabled<A>>();
        let mut got: Vec<u32> = view.iter().map(|p: &P| p.v).collect();
        got.sort_unstable();
        assert_eq!(got, vec![10, 12], "only A-enabled rows are visited");
    }

    /// `Query<&P, Disabled<A>>` yields the complement — rows whose `A` bit is
    /// clear, INCLUDING rows in an archetype that has an A-column but a clear
    /// bit (amendment A1.1 per-row inversion).
    #[test]
    fn query_disabled_yields_complement() {
        register_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[P::component_id()]);
        let e0 = spawn_p(&mut ecs, arch, 20);
        let _e1 = spawn_p(&mut ecs, arch, 21);
        let e2 = spawn_p(&mut ecs, arch, 22);

        // Allocate the A-column (enable e0, e2) so the archetype HAS a column;
        // e1 remains clear within that present column.
        ecs.enable::<A>(e0);
        ecs.enable::<A>(e2);

        let view = ecs.query::<&P, Disabled<A>>();
        let got: Vec<u32> = view.iter().map(|p: &P| p.v).collect();
        assert_eq!(got, vec![21], "only the A-clear row (within a present column)");
    }

    /// Amendment A1.1: a positive-term `Query<&P, Disabled<A>>` over an
    /// archetype with NO A-column reports EVERY row as disabled (no-column ⇒
    /// `filter_fetch` returns `true`). The user named the positive set `&P`.
    #[test]
    fn query_disabled_no_column_archetype_visits_all_rows() {
        register_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[P::component_id()]);
        let _e0 = spawn_p(&mut ecs, arch, 30);
        let _e1 = spawn_p(&mut ecs, arch, 31);
        // No `enable::<A>` call ⇒ the archetype never allocates an A-column.

        let view = ecs.query::<&P, Disabled<A>>();
        let mut got: Vec<u32> = view.iter().map(|p: &P| p.v).collect();
        got.sort_unstable();
        assert_eq!(
            got,
            vec![30, 31],
            "no A-column ⇒ every row is disabled (A1.1 per-row inversion)"
        );
    }

    /// `Disabled<A>` in an AND-tuple with a positive `With` term: only the
    /// A-clear rows of the bounded archetype are visited, including the
    /// no-column-row=disabled semantics.
    #[test]
    fn query_disabled_and_tuple_with_positive_term() {
        register_components();
        let mut ecs = EcsMaster::new();
        let arch = ecs.create_archetype(&[P::component_id()]);
        let e0 = spawn_p(&mut ecs, arch, 40);
        let _e1 = spawn_p(&mut ecs, arch, 41);

        ecs.enable::<A>(e0); // e0 enabled, e1 clear (within the present column)

        // `(With<P>, Disabled<A>)` — With<P> is the positive bound; Disabled<A>
        // is the per-row inverted gate.
        let view = ecs.query::<&P, (With<P>, Disabled<A>)>();
        let got: Vec<u32> = view.iter().map(|p: &P| p.v).collect();
        assert_eq!(got, vec![41], "AND-tuple Disabled keeps only the A-clear row");
    }
}
