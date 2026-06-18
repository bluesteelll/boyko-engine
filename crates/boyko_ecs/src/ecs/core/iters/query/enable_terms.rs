//! EnableTag Step 9 — runtime **per-row** dynamic enable-bit query terms.
//!
//! `EnableTerms` is the per-view term list populated by
//! `Query::with_enabled` / `Query::without_enabled` (and the `QueryView`
//! mirrors). It is the dynamic twin of the typed
//! [`Enabled<T>`](super::filter_enable::Enabled) /
//! [`Disabled<T>`](super::filter_enable::Disabled) filters: each term is a
//! **per-row enable-bit test** at `(archetype, row)`, NOT an archetype-level
//! signature test like the Phase-22 `TagTerms`.
//!
//! # Cost contract (the 0%-gate — Decision D2 / Step 9)
//!
//! A query with no dynamic enable term gates the per-row scan behind a single
//! `EnableTerms::is_empty` (`len == 0`) branch — when no term is set, the
//! cursors never load the resolved-column scratch and never run the bit loop.
//!
//! This is NOT a free const-gate. Unlike the Phase-22.1
//! `TagTerms` gate (archetype-level — it leaves
//! ZERO term code in the row loop) and unlike the Wave-3 `filter_fetch`
//! `if !const { F::IS_ARCHETYPAL }` guard (const-folded away entirely), an
//! enable term is a genuine per-row predicate, so the `is_empty()` guard stays
//! inside the row loop as a RUNTIME branch. It is loop-invariant
//! (`enable_terms` is never written during iteration), so the compiler hoists
//! the `len` load and the residual is one predicted-not-taken branch —
//! bench-verified flat (the `query_iter` / `par_iter` gate benches show no
//! measurable change vs. a no-enable build).
//!
//! # Per-archetype resolution
//!
//! Unlike `TagTerms` (archetype-level, resolved once at the driver entry into
//! a `&[ArchetypeId]` slice), an enable term is a per-row predicate, so the
//! driver caches one `*const EnableColumn` per term **per archetype
//! transition** (`EnableTermCols`) — exactly like the typed
//! [`EnableFetch`](super::filter_enable) `set_table_*` cold path. The per-row
//! `EnableTermCols::passes` then tests the bit (a NULL column reads as
//! disabled, so a `without_enabled` term keeps it — mirroring
//! [`Disabled<T>`](super::filter_enable::Disabled)).

use crate::ecs::constants::MAX_ENABLE_TERMS;
use crate::ecs::core::archetype::archetype::Archetype;
use crate::ecs::core::component::component_registry::EnableTagId;
use crate::ecs::core::component::enable::enable_store::EnableColumn;
use crate::ecs::identifiers::primitives::ComponentId;

/// Runtime per-row enable terms (EnableTag Step 9).
///
/// Carried by BOTH `Query<D, F>` (SystemParam) and `QueryView<D, F>` (direct
/// API); never stored in the shared interned `QueryState` (QS1 stays
/// term-agnostic — these are per-view, like [`TagTerms`](super::tag_terms)).
/// `polarity` bit `i` is `1` for a `with_enabled` term (bit must be SET) and
/// `0` for a `without_enabled` term (bit must be CLEAR) over `ids[i]`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EnableTerms {
    /// Term tag ids; only `[0, len)` is meaningful.
    ids: [ComponentId; MAX_ENABLE_TERMS],
    /// Bit `i`: `1` = with_enabled (bit must be set), `0` = without_enabled
    /// (bit must be clear).
    polarity: u8,
    /// Number of live terms; `<= MAX_ENABLE_TERMS`.
    len: u8,
}

impl EnableTerms {
    /// The no-terms value — the state of every freshly minted query view. The
    /// universal case; the cursors take the byte-identical fast path for it.
    pub(crate) const EMPTY: Self = Self {
        // Slot 0 is a placeholder, never read: only `[0, len)` is meaningful.
        ids: [ComponentId(0); MAX_ENABLE_TERMS],
        polarity: 0,
        len: 0,
    };

    /// `true` when no terms are set — the byte-identical fast-path gate every
    /// driver checks. The whole per-row enable scan is unreachable when this
    /// returns `true` (the 0%-gate).
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Number of live terms.
    #[inline]
    #[allow(dead_code)] // symmetry with is_empty; consumed by tests / debug asserts
    pub(crate) fn len(&self) -> usize {
        self.len as usize
    }

    /// Appends a `with_enabled` term (the row's `tag` bit must be SET).
    ///
    /// # Panics
    /// Loud release-active panic past [`MAX_ENABLE_TERMS`] terms (setup-time,
    /// cold — C2 dynamic enforcement).
    #[inline]
    pub(crate) fn push_with(&mut self, tag: EnableTagId) {
        self.push(tag.component_id(), true);
    }

    /// Appends a `without_enabled` term (the row's `tag` bit must be CLEAR).
    ///
    /// # Panics
    /// Loud release-active panic past [`MAX_ENABLE_TERMS`] terms (setup-time,
    /// cold — C2 dynamic enforcement).
    #[inline]
    pub(crate) fn push_without(&mut self, tag: EnableTagId) {
        self.push(tag.component_id(), false);
    }

    fn push(&mut self, tag: ComponentId, with: bool) {
        let idx = self.len as usize;
        if idx >= MAX_ENABLE_TERMS {
            enable_terms_overflow_panic();
        }
        self.ids[idx] = tag;
        if with {
            self.polarity |= 1 << idx;
        }
        self.len += 1;
    }

    /// Resolves every term's [`EnableColumn`] pointer for `archetype` into an
    /// [`EnableTermCols`] scratch. Called ONCE per archetype transition by the
    /// drivers (the cold per-archetype path), mirroring the typed
    /// [`EnableFetch`](super::filter_enable) `set_table_*` discipline.
    ///
    /// `archetype` is borrowed `&self` only — a scan of the `EnableStore`
    /// (≤ 4) per term — so the resolved pointers are valid for as long as the
    /// archetype outlives the returned scratch (the cursor's `'q`).
    #[inline]
    pub(crate) fn resolve(&self, archetype: &Archetype) -> EnableTermCols {
        let mut cols = EnableTermCols {
            cols: [core::ptr::null(); MAX_ENABLE_TERMS],
            polarity: self.polarity,
            len: self.len,
        };
        let len = self.len as usize;
        for i in 0..len {
            cols.cols[i] = archetype.enable_column_ptr(self.ids[i]);
        }
        cols
    }
}

/// Per-archetype resolved enable-term columns (EnableTag Step 9).
///
/// A NULL `cols[i]` means the archetype has no allocated column for term `i`'s
/// tag — every row reads disabled (clear). [`Self::passes`] therefore keeps a
/// `without_enabled` row (clear bit ⇒ matches) and drops a `with_enabled` row
/// (clear bit ⇒ does not match) — identical polarity to the typed
/// [`Enabled<T>`](super::filter_enable::Enabled) /
/// [`Disabled<T>`](super::filter_enable::Disabled) `filter_fetch`.
#[derive(Clone, Copy)]
pub(crate) struct EnableTermCols {
    /// Per-term resolved column pointer (NULL = no column for this archetype).
    cols: [*const EnableColumn; MAX_ENABLE_TERMS],
    /// Mirror of [`EnableTerms::polarity`] (term `i`: `1` = with, `0` = without).
    polarity: u8,
    /// Number of live terms; `<= MAX_ENABLE_TERMS`.
    len: u8,
}

impl EnableTermCols {
    /// The empty scratch — every cursor's initial value before the first
    /// archetype transition (and the only value a no-enable-term cursor ever
    /// holds). `passes` short-circuits `true` for it (`len == 0`).
    pub(crate) const EMPTY: Self = Self {
        cols: [core::ptr::null(); MAX_ENABLE_TERMS],
        polarity: 0,
        len: 0,
    };

    /// `true` iff `row` satisfies EVERY resolved term (with-set / without-clear).
    ///
    /// # Safety
    ///
    /// Each non-null `cols[i]` MUST be a column pointer cached by a prior
    /// [`EnableTerms::resolve`] for the archetype the cursor is currently
    /// positioned on, valid for the cursor lifetime (the archetype outlives
    /// the cursor; a directory regrow runs only inside a `&mut` apply window
    /// where no cursor is live — same contract as the typed `EnableFetch`).
    /// `row` MUST be in range for that archetype (`row < entity_count()`).
    #[inline]
    pub(crate) unsafe fn passes(&self, row: usize) -> bool {
        let len = self.len as usize;
        for i in 0..len {
            let want = (self.polarity >> i) & 1 != 0;
            let col = self.cols[i];
            let is_set = if col.is_null() {
                // No column for this archetype ⇒ every row disabled (clear).
                false
            } else {
                // SAFETY: per the method contract `col` is the borrowed column
                //   pointer cached by `EnableTerms::resolve` for the current
                //   archetype; it is non-null (checked) and valid for the
                //   cursor lifetime. `EnableColumn::test` does the paged deref
                //   + word load (`Relaxed`) + bit test, reading `false` for a
                //   never-toggled page. `row` is in range per the contract —
                //   mirrors the Wave-3 `Enabled<T>::filter_fetch` SAFETY.
                unsafe { (*col).test(row) }
            };
            if is_set != want {
                return false;
            }
        }
        true
    }
}

/// Cold panic site for the >[`MAX_ENABLE_TERMS`] overflow — loud,
/// release-active, fired at term-add time (setup), never on the iteration hot
/// path (C2 dynamic enforcement: the bounding requirement that cannot be
/// const-checked for dynamic terms is enforced here).
#[cold]
#[inline(never)]
fn enable_terms_overflow_panic() -> ! {
    panic!(
        "with_enabled/without_enabled: more than MAX_ENABLE_TERMS = {MAX_ENABLE_TERMS} dynamic \
         enable terms on a single query; raise the constant if the use case is real"
    )
}
