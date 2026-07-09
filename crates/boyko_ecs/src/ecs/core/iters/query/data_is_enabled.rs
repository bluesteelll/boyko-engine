//! `IsEnabled<T>` — non-filtering per-row read of an EnableTag bit.
//!
//! The order-preserving twin of [`Option<&T>`](super::data): a [`QueryData`]
//! datum whose `Item<'w>` is `bool`, yielding the EnableTag `T` bit for EVERY
//! row the query already matches, in the identical `iter()` order. It is
//! **non-filtering** — it never drops or reorders a row (only filters / the
//! cull pass touch the archetype id-slice; a datum's `fetch(row)` runs once per
//! already-selected row in the cursor's existing order).
//!
//! # Why a new datum (and not `Option<&T>` or `Enabled<T>`)
//!
//! A bitset EnableTag (`#[component(storage = "bitset")]`) has NO
//! `ComponentPool` / column, so:
//! * `Option<&T>` cannot resolve it (its inner `&T` fetch dereferences a column
//!   that does not exist for the id);
//! * `Enabled<T>` is a per-row FILTER — it DROPS rows, which reindexes any dense
//!   downstream addressing (the determinism-breaking move physics must avoid).
//!
//! `IsEnabled<T>` instead reads the bit from the archetype's `EnableColumn`
//! (the same column-ptr discipline as
//! [`EnableFetch`](super::filter_enable::EnableFetch)) and yields it as a value,
//! never gating the row. This is the reusable kernel primitive any system needs
//! to read an enable bit per row without filtering.
//!
//! # Fetch shape
//!
//! [`IsEnabledFetch`] is byte-identical to
//! [`EnableFetch`](super::filter_enable::EnableFetch): a single
//! `*const EnableColumn` cached per archetype in `set_table_*`, NULL for an
//! archetype with no column for the tag (every row reads `false`).
//!
//! # Access / shape consts
//!
//! * `init_access` — no-op (structural). A bitset id has no `ComponentPool`, so
//!   no sibling `&T` / `&mut T` data param can ever alias it — identical
//!   argument to [`Enabled<T>`](super::filter_enable::Enabled)'s `init_access`.
//! * `matches_component_set` — unconditionally `true` (non-filtering, like
//!   `Option<D>`); the bit is never in any archetype signature mask.
//! * `aggregate_include` — no-op (adds no required bit, like `Option<D>`).
//! * `HAS_DATA_COMPONENT = false` — contributes no positive include bit (it
//!   never requires `T` present), so it does not bound an `Enabled`/`Disabled`
//!   term any more than `Option<&T>` does.

use std::marker::PhantomData;

use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::component::component::Component;
use crate::ecs::core::component::component_registry::{self, StorageKind};
use crate::ecs::core::component::enable::enable_store::EnableColumn;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::identifiers::primitives::ComponentId;

use super::data::{QueryData, ReadOnlyQueryData};

/// Per-system cached state for [`IsEnabled<T>`]: the resolved bitset
/// [`ComponentId`]. Mirrors
/// [`EnabledState`](super::filter_enable::EnabledState).
#[derive(Clone, Copy)]
pub struct IsEnabledState<T> {
    pub(crate) id: ComponentId,
    _marker: PhantomData<fn() -> T>,
}

/// Per-archetype fetch scratch for [`IsEnabled<T>`]: a borrowed pointer to the
/// active archetype's [`EnableColumn`] for the tag, or NULL.
///
/// NULL means the archetype has no allocated column for the tag — every row
/// reads `false` (never toggled). `set_table_*` refreshes the pointer per
/// archetype, exactly like
/// [`EnableFetch`](super::filter_enable::EnableFetch).
pub struct IsEnabledFetch<'w> {
    /// Base pointer to the active archetype's column for the tag, or NULL.
    pub(crate) col: *const EnableColumn,
    /// Ties the fetch lifetime to `'w` (the archetype-pointer lifetime).
    pub(crate) _marker: PhantomData<&'w ()>,
}

impl Clone for IsEnabledFetch<'_> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for IsEnabledFetch<'_> {}

/// Caches the archetype's [`EnableColumn`] pointer (or NULL) into `col`.
///
/// # Safety
///
/// `archetype` MUST be a live `*const Archetype` for the fetch lifetime `'w`
/// with provenance from `UnsafeEcsCell::archetype_ptr(id)` (the `set_table_*`
/// caller contract). The returned pointer borrows the archetype's `EnableStore`
/// and is re-read by every `set_table_*` (so a directory regrow under a `&mut`
/// apply window — where no fetch is live — never leaves it dangling). Mirrors
/// `filter_enable::cache_column`.
#[inline]
unsafe fn cache_column(col: &mut *const EnableColumn, id: ComponentId, archetype: *const Archetype) {
    // SAFETY: per the caller contract `archetype` is a live `*const Archetype`
    //   for `'w`; the shared reborrow is confined to this call.
    //   `enable_column_ptr` reads the archetype's `EnableStore` (an `&self`
    //   scan ≤ 4) and returns a borrowed `*const EnableColumn` (or NULL) whose
    //   provenance is the stable, interior-mutable column storage — valid for
    //   `'w` because the archetype outlives the fetch.
    let archetype_ref: &Archetype = unsafe { &*archetype };
    *col = archetype_ref.enable_column_ptr(id);
}

/// Non-filtering per-row read of EnableTag `T`'s bit. `Item<'w> = bool`.
///
/// Yields the bit for EVERY row the query already matches, in the identical
/// `iter()` order — it NEVER drops or reorders a row (the order-preserving twin
/// of [`Option<&T>`](super::data)). See the module docs for the full rationale.
///
/// ```ignore
/// // Read an enable bit per row WITHOUT filtering:
/// for (pos, awake) in query.iter() {  // Query<(&Pos, IsEnabled<Awake>)>
///     if awake { /* ... */ }
/// }
/// ```
pub struct IsEnabled<T: Component> {
    _marker: PhantomData<fn() -> T>,
}

// SAFETY (QD1-QD4):
//   - QD1: `init_state` caches `T::component_id()`; `init_access` declares NO
//     access (structural — a bitset id has no `ComponentPool`, so no sibling
//     `&T`/`&mut T` data param can alias it; identical argument to
//     `Enabled<T>::init_access`). The `fetch` reads ONLY the enable bit, not a
//     component column.
//   - QD2: `init_fetch` sets `col = null`; `set_table_*` overwrites it (or NULL
//     for a no-column archetype) before any `fetch` call.
//   - QD3: `Fetch<'w>` lifetime is `'w` via `PhantomData<&'w ()>`; the cached
//     `*const EnableColumn` is scoped to the archetype minted for `'w`.
//   - QD4: read-only data — both `set_table_*` variants share the same body
//     (the enable bit is read shared regardless of the archetype pointer's
//     mutability).
unsafe impl<T: Component> QueryData for IsEnabled<T> {
    type State = IsEnabledState<T>;
    type Fetch<'w> = IsEnabledFetch<'w>;
    type Item<'w> = bool;

    const IS_READ_ONLY: bool = true;
    // The enable bit is not a per-row tick; no change-detection meta needed.
    const NEEDS_CHANGE_DETECTION: bool = false;
    // Non-filtering: contributes NO positive include bit (it never requires `T`
    // present), exactly like `Option<&T>`. So it is NOT a bounding data
    // component for an `Enabled`/`Disabled` term.
    const HAS_DATA_COMPONENT: bool = false;
    // `matches_component_set` is unconditionally `true` ⇒ nothing to trim.
    const REQUIRES_POST_FILTER_TRIM: bool = false;

    #[inline]
    fn init_state(_world: &mut EcsMaster) -> Self::State {
        let id = T::component_id();
        debug_assert_eq!(
            component_registry::storage_kind(id.0),
            StorageKind::Bitset,
            "IsEnabled<{}>: id {} is not a bitset enable tag — use \
             #[component(storage = \"bitset\")] / register_enable_tag",
            std::any::type_name::<T>(),
            id.0,
        );
        IsEnabledState { id, _marker: PhantomData }
    }

    #[inline]
    fn init_access(_state: &Self::State, _access_set: &mut FilteredAccessSet) {
        // No-op (structural) — see `Enabled<T>::init_access` (filter_enable.rs)
        // for the full argument: a bitset id has no `ComponentPool`, so a
        // sibling `&T`/`&mut T` data access on the id is STRUCTURALLY IMPOSSIBLE
        // ⇒ nothing for the aliasing detector to serialize against ⇒ a no-op is
        // correct. Precedent: `Without<C>` (the only other no-op leaf).
    }

    #[inline]
    fn matches_component_set(
        _state: &Self::State,
        _mask: &crate::ecs::core::component::component_mask::ComponentMask,
    ) -> bool {
        // Non-filtering: the archetype is admitted whether or not the bit is set
        // for any of its rows (the bit is never in the signature mask). The
        // per-row value is delivered by `fetch`, not by archetype membership.
        true
    }

    #[inline]
    fn aggregate_include(
        _state: &Self::State,
        _include: &mut crate::ecs::core::component::component_mask::ComponentMask,
    ) {
        // No-op: adds no required bit (a bitset id is signature-excluded, and
        // requiring it would wrongly match nothing). Mirrors `Option<D>`.
    }

    #[inline]
    fn init_fetch<'w>(_state: &Self::State) -> Self::Fetch<'w> {
        IsEnabledFetch { col: std::ptr::null(), _marker: PhantomData }
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
        _meta: &'_ SystemMeta,
    ) {
        // SAFETY (QD3): forwarded to `cache_column` — `archetype` carries the
        //   caller's read-only provenance for `'w`; the only per-archetype state
        //   is the column pointer and `meta` carries nothing this datum reads.
        unsafe { cache_column(&mut fetch.col, state.id, archetype) }
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        _meta: &'_ SystemMeta,
    ) {
        // SAFETY (QD3, QD4): the enable bit is read shared regardless of the
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
        // NCD = false: the meta-free variant carries the real body.
        // SAFETY (QD3): `archetype` carries the caller's read-only provenance.
        unsafe { cache_column(&mut fetch.col, state.id, archetype) }
    }

    #[inline]
    unsafe fn set_table_mut_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    ) {
        // SAFETY (QD3, QD4): shared reborrow of a write-capable pointer for a
        //   shared read of the enable bit.
        unsafe { cache_column(&mut fetch.col, state.id, archetype as *const _) }
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        if fetch.col.is_null() {
            // No column for this archetype ⇒ every row reads disabled.
            return false;
        }
        // SAFETY (QD2, QD3): `fetch.col` is the borrowed column pointer cached
        //   by the prior `set_table_*` for this archetype; it is non-null
        //   (checked above) and valid for `'w` (the archetype outlives the
        //   fetch; a directory regrow runs only inside a `&mut` apply window
        //   where no fetch is live). `EnableColumn::test` does the paged deref +
        //   word load (`Relaxed`) + bit test, reading `false` for a
        //   never-toggled page; `row < entity_count()` per the QD2/QD3 contract.
        //   Mirrors `Enabled<T>::filter_fetch` (filter_enable.rs).
        let column: &EnableColumn = unsafe { &*fetch.col };
        column.test(row)
    }
}

// SAFETY: `IsEnabled<T>` reads ONLY the enable bit and yields a `bool` by value;
//   it performs no writes and `IS_READ_ONLY = true`.
unsafe impl<T: Component> ReadOnlyQueryData for IsEnabled<T> {}
