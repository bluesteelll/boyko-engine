//! Split from `data.rs` (mechanical move; see `super` for the shared
//! `QueryData` / `ReadOnlyQueryData` traits and imports).

use super::*;

// ── Option<D> (task #9 — non-filtering optional data) ──────────────────────
//
// `Option<D>` yields `Some(D::Item)` for archetypes that contain `D`'s
// component(s) and `None` for those that do not — WITHOUT filtering the
// archetype out. This is the inverse of every leaf: `matches_component_set`
// is unconditionally `true` (the archetype is admitted either way), and the
// per-archetype `set_table_*` GATES the inner forward on whether the inner
// `D` actually matches (Decision 1).

/// Per-archetype fetch scratch for `Option<D>: QueryData`.
///
/// Holds the inner `D::Fetch<'w>` plus a `matches` flag computed in
/// `set_table_*` (`true` iff the active archetype contains `D`'s
/// component(s)). When `matches` is `false`, `inner` stays at its
/// `D::init_fetch` NULL-init value and is NEVER read (`fetch` returns `None`).
///
/// `Copy` / `Clone` are implemented manually so the auto-derive does not
/// require `D::Fetch<'w>: Copy` via an unwanted blanket bound (it already is
/// `Copy` per the `QueryData::Fetch: Copy` bound, but the manual impls mirror
/// `ReadFetch` and keep the derive heuristics out of the picture).
pub struct OptionFetch<'w, D: QueryData> {
    /// Inner fetch. Valid only when `matches` is `true`; otherwise the
    /// NULL-init value from `D::init_fetch` (never dereferenced).
    pub(crate) inner: D::Fetch<'w>,
    /// `true` iff the active archetype contains `D`'s component(s).
    pub(crate) matches: bool,
}

impl<D: QueryData> Clone for OptionFetch<'_, D> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<D: QueryData> Copy for OptionFetch<'_, D> {}

// SAFETY (QD1-QD4):
//   - QD1: `init_access` forwards to `D::init_access`, declaring `D`'s exact
//     read/write surface (Decision 8 — conservative, correct; still trips
//     B0002 for `(&mut A, Option<&A>)`).
//   - QD2: `init_fetch` produces `(D::init_fetch (NULL), matches = false)`;
//     `set_table_*` overwrites `matches` and, when `matches == true`, the
//     inner via the gated forward, before any `fetch` call.
//   - QD3: `OptionFetch<'w, D>` carries `D::Fetch<'w>`, so the inner's
//     lifetime invariants ride `'w`.
//   - QD4: each `set_table_*` variant forwards to the matching inner variant
//     (readonly→readonly, mut→mut, no_meta→no_meta); the inner's own QD4
//     backstop-panic is preserved (NEVER gated away — only the FORWARD is
//     gated on `matches`).
unsafe impl<D: QueryData> QueryData for Option<D> {
    type State = D::State;
    type Fetch<'w> = OptionFetch<'w, D>;
    type Item<'w> = Option<D::Item<'w>>;

    const IS_READ_ONLY: bool = D::IS_READ_ONLY;
    // The inner participates in change detection iff `D` does; the dispatcher
    // routes via the same `if const { D::NCD || F::NCD }` const-fold.
    const NEEDS_CHANGE_DETECTION: bool = D::NEEDS_CHANGE_DETECTION;
    // Non-filtering: `Option<D>` contributes NO positive include bit (it never
    // requires `D`'s component present), so it is NOT a bounding data
    // component for an `Enabled`/`Disabled` term.
    const HAS_DATA_COMPONENT: bool = false;
    // `matches_component_set` is unconditionally `true` ⇒ nothing to trim.
    const REQUIRES_POST_FILTER_TRIM: bool = false;
    // Dense plan D3: forward the inner's dense-ness so the cursor resolves the
    // inner's `DenseStore` pointer (otherwise `Option<&Dense>`'s inner `fetch`
    // would deref a NULL store). W1 (None-on-absence): `Option<&Dense>` yields
    // `Some(&val)` for a present member and `None` for an absent one — the
    // correct `Option` semantics. The per-row membership is the inner's
    // `dense_row_passes` (≡ `slot_of(entity).is_some()`), checked inside
    // `fetch` (NOT via `Self::dense_row_passes`, which stays the default `true`
    // so `Option` never SKIPS a row — it maps an absent member to `None`).
    const HAS_DENSE: bool = D::HAS_DENSE;
    // Relation-DSL join: forward the inner's relation-ness so the cursor
    // resolves the inner's world cell (gated internally by `D::HAS_RELATED`).
    const HAS_RELATED: bool = D::HAS_RELATED;

    #[inline]
    fn init_state(world: &mut EcsMaster) -> Self::State {
        D::init_state(world)
    }

    fn init_access(state: &Self::State, access_set: &mut FilteredAccessSet) {
        // Decision 8: forward — declares `D`'s read/write surface so
        // `(&mut A, Option<&A>)` and `AnyOf<(&mut A, …)>` still trip B0002.
        D::init_access(state, access_set);
    }

    #[inline]
    fn matches_component_set(_state: &Self::State, _mask: &ComponentMask) -> bool {
        // Non-filtering: the archetype is admitted whether or not it contains
        // `D`. The per-archetype `matches` flag (computed in `set_table_*`)
        // decides Some vs None per row, NOT archetype membership.
        true
    }

    #[inline]
    fn aggregate_include(_state: &Self::State, _include: &mut ComponentMask) {
        // No-op: `Option<D>` adds no required bit (Decision: do NOT populate
        // `include` — that would WRONGLY require `D`'s component present).
    }

    #[inline]
    fn init_fetch<'w>(state: &Self::State) -> Self::Fetch<'w> {
        OptionFetch {
            inner: D::init_fetch(state),
            matches: false,
        }
    }

    #[inline]
    unsafe fn resolve_dense<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
    ) {
        // Dense plan D3: forward into the inner so its `DenseStore` pointer is
        // resolved (gated internally by `const { D::HAS_DENSE }`).
        // SAFETY (D3): the `world` cell is `Copy`, forwarded by value to
        //   preserve provenance; the inner gates its body on its own dense-ness.
        unsafe { D::resolve_dense(&mut fetch.inner, state, world); }
    }

    #[inline]
    unsafe fn resolve_related<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        world: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell<'w>,
    ) {
        // Relation-DSL join: forward into the inner so its world cell is cached
        // (gated internally by `const { D::HAS_RELATED }`).
        // SAFETY (relation join): the `world` cell is `Copy`, forwarded by value
        //   to preserve provenance; the inner gates its body on its own
        //   relation-ness.
        unsafe { D::resolve_related(&mut fetch.inner, state, world); }
    }

    #[inline]
    unsafe fn set_table_readonly<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
        meta: &'_ SystemMeta,
    ) {
        // SAFETY (Decision 1, QD3, QD4): `archetype` is a live `*const
        //   Archetype` for `'w` (caller contract). `matches` re-derives the
        //   inner predicate from the archetype's own mask. When `matches ==
        //   true`, `D::matches_component_set` held ⇒ every column `D` reads is
        //   non-null ⇒ the forwarded `D::set_table_readonly`'s QD1/QD3 + its
        //   internal `debug_assert!(!ptr.is_null())` hold; the inner's QD4
        //   readonly backstop-panic (for a write-inner) is reached only if a
        //   custom impl falsely claimed `ReadOnlyQueryData for Option<&mut T>`
        //   — preserved verbatim. When `matches == false`, the forward is
        //   skipped and `fetch.inner` stays at its NULL-init value, NEVER read
        //   (`fetch` returns `None`).
        let matches = D::matches_component_set(state, unsafe { (*archetype).component_mask() });
        if matches {
            unsafe { D::set_table_readonly(&mut fetch.inner, state, archetype, meta) };
        }
        fetch.matches = matches;
    }

    #[inline]
    unsafe fn set_table_mut<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
        meta: &'_ SystemMeta,
    ) {
        // SAFETY (Decision 1, QD1, QD3, QD4): same gate as
        //   `set_table_readonly` with the strictly-stronger caller guarantee
        //   that `archetype` carries write-capable provenance. When `matches
        //   == true`, the forwarded `D::set_table_mut` consumes that
        //   provenance for `D`'s columns. When `false`, the inner stays
        //   NULL-init and is never read. The mask read uses a shared reborrow
        //   (`component_mask` needs no write provenance).
        let matches =
            D::matches_component_set(state, unsafe { (*archetype).component_mask() });
        if matches {
            unsafe { D::set_table_mut(&mut fetch.inner, state, archetype, meta) };
        }
        fetch.matches = matches;
    }

    #[inline]
    unsafe fn set_table_readonly_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *const Archetype,
    ) {
        // SAFETY (Decision 1/2, QD3, QD4): identical gate to
        //   `set_table_readonly` minus the unused `meta`. For an NCD=false
        //   inner (`&T`) this forwards to the inner's real meta-free body
        //   (Decision 2 row 1). For an NCD=true inner (`Ref<T>`) the forward
        //   reaches the inner's `#[cold]` no-meta panic — UNREACHABLE, because
        //   `Option<Ref<T>>::NCD = true` routes the driver
        //   (iter.rs:298 `if const { D::NCD || F::NCD }`) to the meta path
        //   (Decision 2 note b). The inner's QD4 readonly backstop on a
        //   write-inner is preserved verbatim.
        let matches = D::matches_component_set(state, unsafe { (*archetype).component_mask() });
        if matches {
            unsafe { D::set_table_readonly_no_meta(&mut fetch.inner, state, archetype) };
        }
        fetch.matches = matches;
    }

    #[inline]
    unsafe fn set_table_mut_no_meta<'w>(
        fetch: &mut Self::Fetch<'w>,
        state: &Self::State,
        archetype: *mut Archetype,
    ) {
        // SAFETY (Decision 1/2, QD1, QD3, QD4): same gate, write-capable
        //   `archetype`, meta-free. For an NCD=false inner (`&mut T`) this
        //   forwards to the inner's real meta-free body (Decision 2 row 2).
        //   For an NCD=true inner (`Mut<T>`) the forward reaches the inner's
        //   `#[cold]` no-meta panic — UNREACHABLE because `Option<Mut<T>>::NCD
        //   = true` routes through the meta path.
        let matches =
            D::matches_component_set(state, unsafe { (*archetype).component_mask() });
        if matches {
            unsafe { D::set_table_mut_no_meta(&mut fetch.inner, state, archetype) };
        }
        fetch.matches = matches;
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        // SAFETY (Decision 1, QD2, QD3): when `fetch.matches`, the inner was
        //   initialised by the gated `set_table_*` forward (caller called
        //   `set_table_*` before any `fetch`), so `D::fetch`'s contract holds
        //   (`row < entity_count`, inner bases non-null). When `!fetch.matches`
        //   the inner is the NULL-init value — NOT read here (we return
        //   `None`).
        if fetch.matches {
            // Dense plan D3 (W1 — None-on-absence): for a dense inner, the
            // archetype-level `matches` is unconditionally `true` (a dense
            // component is signature-excluded). The REAL membership oracle is
            // the per-row dense slot lookup. Gate the inner forward on the
            // inner's own `dense_row_passes` (≡ `slot_of(entity).is_some()`):
            // an entity that lacks the dense member yields `None`, the correct
            // `Option` semantics. For a table inner this const-folds OUT — the
            // unconditional `Some(D::fetch)` path is restored (0%-gate).
            if const { D::HAS_DENSE } {
                // SAFETY (D3): the inner's `set_table_*` forward cached
                //   `entity_ids` and `resolve_dense` cached the `DenseStore`
                //   pointer (both run for `matches == true` before any
                //   `fetch`); `row < entity_count` (caller contract). A NULL
                //   store / absent slot ⟹ `dense_row_passes` returns `false`
                //   ⟹ we never call `D::fetch` (no NULL/missing-slot deref).
                if unsafe { D::dense_row_passes(&fetch.inner, row) } {
                    Some(unsafe { D::fetch(&fetch.inner, row) })
                } else {
                    None
                }
            } else {
                Some(unsafe { D::fetch(&fetch.inner, row) })
            }
        } else {
            None
        }
    }
}

// SAFETY: `Option<D>` performs no writes when `D` does not — `IS_READ_ONLY =
//   D::IS_READ_ONLY`, and `D: ReadOnlyQueryData` guarantees `D::IS_READ_ONLY
//   = true`. So `Option<&mut T>` / `Option<Mut<T>>` are rejected from
//   `iter()` / `par_iter()` (only `iter_mut()` admits them).
unsafe impl<D: ReadOnlyQueryData> ReadOnlyQueryData for Option<D> {}

