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
use crate::ecs::core::component::enable::enable_store::{
    EnableColumn, rows_per_page_log2, words_per_page,
};
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

    /// AND-composite enabled-RUN walk over the resolved columns (Decision 5b /
    /// W2). Calls `f(run_start, run_len)` once per maximal contiguous MATCHING
    /// run in `[range_start, range_end)`, where a row matches iff it satisfies
    /// EVERY term (with-set / without-clear) — the chunk-path generalisation of
    /// [`Self::passes`].
    ///
    /// Every yielded run is ⊆ `[range_start, range_end)` (INV-RANGE); `range_end`
    /// is the sole upper terminator, including the all-`u64::MAX` span produced
    /// by an all-`without` / absent-page archetype (C5). A NULL column's
    /// summary/data is NEVER dereferenced (W2): a NULL `with` term forces an
    /// empty composite (no runs); a NULL `without` term contributes all-ones.
    ///
    /// # Safety
    ///
    /// Each non-null `cols[i]` MUST be a column pointer cached by a prior
    /// [`EnableTerms::resolve`] for the archetype the cursor is currently
    /// positioned on, valid for the call (the archetype outlives the call; a
    /// directory regrow runs only inside a `&mut` apply window where no cursor is
    /// live — same contract as [`Self::passes`]). `range_end <= entity_count` for
    /// that archetype.
    pub(crate) unsafe fn for_each_run(
        &self,
        range_start: usize,
        range_end: usize,
        mut f: impl FnMut(usize, usize),
    ) {
        debug_assert!(range_start <= range_end, "for_each_run: range_start > range_end");
        let len = self.len as usize;

        // ── Single-term fast path: delegate to the Decision-4 kernel. ────────
        if len == 1 {
            let want = self.polarity & 1 != 0;
            let col = self.cols[0];
            if col.is_null() {
                // NULL single term: `with` ⇒ no runs; `without` ⇒ one full run.
                if !want && range_end > range_start {
                    f(range_start, range_end - range_start);
                }
                return;
            }
            // SAFETY: `col` is non-null (checked) and the resolved column for the
            //   current archetype per the method contract; `enabled_runs` only
            //   reads its paged words / summary (`Relaxed`), never mutates.
            let runs = unsafe { (*col).enabled_runs(range_start, range_end, !want) };
            for (s, l) in runs {
                f(s, l);
            }
            return;
        }

        // ── Zero terms: vacuously matches every row (one run). ───────────────
        if len == 0 {
            if range_end > range_start {
                f(range_start, range_end - range_start);
            }
            return;
        }

        // ── Multi-term: any NULL `with` term kills the whole composite. ──────
        for i in 0..len {
            let want = (self.polarity >> i) & 1 != 0;
            if want && self.cols[i].is_null() {
                return; // empty composite ⇒ no runs (fast exit).
            }
        }
        // SAFETY: every NULL term remaining is a `without` term (the NULL `with`
        //   case returned above); the composite walk below skips NULL columns'
        //   summary/data entirely (all-ones contribution).
        unsafe { self.for_each_run_composite(range_start, range_end, &mut f) };
    }

    /// Multi-term composite run walk (≥ 2 terms, no NULL `with`). Reads the
    /// per-block composite match-bitmap via the non-flushing single-cursor logic
    /// mirroring [`EnableColumn::enabled_runs`] (INV-COALESCE + INV-RANGE).
    ///
    /// # Safety
    ///
    /// Same as [`Self::for_each_run`]; additionally every NULL column in
    /// `cols[0..len)` MUST be a `without` term (the caller verified no NULL
    /// `with` term remains).
    unsafe fn for_each_run_composite(
        &self,
        range_start: usize,
        range_end: usize,
        f: &mut impl FnMut(usize, usize),
    ) {
        let words_per_page = words_per_page();
        let log2 = rows_per_page_log2();
        let len = self.len as usize;

        let mut next_row = range_start;
        loop {
            // ── Phase 1: find a run start. ───────────────────────────────────
            let run_start = loop {
                if next_row >= range_end {
                    return;
                }
                let p = next_row >> log2;
                let w = (next_row >> 6) & (words_per_page - 1);
                let word_base = (next_row >> 6) << 6;

                // Coarse permit: a 64-row block can contain a match only if every
                // `with` term's summary bit is set AND every `without` term's
                // summary bit may be clear (always permits — clear contributes a
                // MAX match-word). NULL `without` terms touch no summary.
                let mut permit = true;
                for i in 0..len {
                    let want = (self.polarity >> i) & 1 != 0;
                    if want {
                        // `with`: column is non-null here (NULL-with returned).
                        // SAFETY: non-null resolved column per the contract.
                        let s = unsafe { (*self.cols[i]).summary_word(p) };
                        if (s >> w) & 1 == 0 {
                            permit = false;
                            break;
                        }
                    }
                    // `without` term: no summary constraint (clear bits anywhere
                    // contribute matches); skip — never touch a NULL column.
                }
                if !permit {
                    // Whole-page skip (perf): the per-word permit just failed. If
                    // the AND of every `with` term's page summary is 0, NO word in
                    // this 4096-row page can match (the page-level hoist of the
                    // per-word permit) — skip the whole page in one step instead
                    // of 64 per-word probes. Otherwise advance one 64-row word.
                    // `without` terms never gate (a clear bit contributes a match);
                    // a NULL `with` term cannot reach here (the caller returned).
                    let mut page_and = u64::MAX;
                    for i in 0..len {
                        if (self.polarity >> i) & 1 != 0 {
                            // SAFETY: non-null resolved `with` column per the
                            //   contract (same deref as the permit loop above).
                            page_and &= unsafe { (*self.cols[i]).summary_word(p) };
                            if page_and == 0 {
                                break;
                            }
                        }
                    }
                    next_row = if page_and == 0 {
                        ((p + 1) << log2).min(range_end)
                    } else {
                        (word_base + 64).min(range_end)
                    };
                    continue;
                }

                // Mask off bits BELOW the cursor's in-word position so that
                // `trailing_zeros()` cannot return an already-consumed bit in
                // this same word (BUG-1: a stale low bit rewinds the cursor →
                // infinite loop). `range_mask` encodes only the range bounds.
                let bit_in_word = next_row - word_base;
                let cursor_low_mask = if bit_in_word == 0 {
                    u64::MAX
                } else {
                    u64::MAX << bit_in_word
                };
                // SAFETY: the composite reads only non-null columns' data; NULL
                //   `without` columns contribute u64::MAX (no deref).
                let composite = unsafe { self.composite_word(p, w) }
                    & range_mask(word_base, range_start, range_end)
                    & cursor_low_mask;
                if composite == 0 {
                    next_row = (word_base + 64).min(range_end);
                    continue;
                }
                break word_base + composite.trailing_zeros() as usize;
            };

            // ── Phase 2: extend maximally, clamped to range_end (C4). ────────
            let mut cursor = run_start;
            let run_end = loop {
                let p = cursor >> log2;
                let w = (cursor >> 6) & (words_per_page - 1);
                let word_base = (cursor >> 6) << 6;
                let bit_in_word = cursor - word_base;

                // SAFETY: composite reads only non-null columns' data.
                let composite = unsafe { self.composite_word(p, w) }
                    & range_mask(word_base, range_start, range_end);
                let above = composite >> bit_in_word;
                let ones = above.trailing_ones() as usize;
                let block_end = cursor + ones;

                let reached_word_top = block_end > word_base + 63;
                if !reached_word_top || block_end >= range_end {
                    break block_end.min(range_end);
                }
                cursor = word_base + 64;
                if cursor >= range_end {
                    break range_end;
                }
            };

            debug_assert!(
                range_start <= run_start && run_end <= range_end,
                "for_each_run_composite INV-RANGE violated"
            );
            debug_assert!(run_end > run_start, "for_each_run_composite empty run");
            next_row = run_end;
            f(run_start, run_end - run_start);
        }
    }

    /// AND-composite match-bitmap for 64-row block `(p, w)` (W2): per term
    /// `match = want ? data : !data`; NULL `without` term contributes `u64::MAX`
    /// (no load). A NULL `with` term cannot reach here (caller verified).
    ///
    /// # Safety
    ///
    /// Every non-null `cols[i]` is a valid resolved column per the call contract.
    #[inline]
    unsafe fn composite_word(&self, p: usize, w: usize) -> u64 {
        let len = self.len as usize;
        let mut acc = u64::MAX;
        for i in 0..len {
            let want = (self.polarity >> i) & 1 != 0;
            let col = self.cols[i];
            let contribution = if col.is_null() {
                // NULL term reaching here is a `without` term ⇒ every row clear ⇒
                // every row matches ⇒ all-ones; no summary/data load (W2).
                u64::MAX
            } else {
                // SAFETY: non-null resolved column per the contract. `match_word`
                //   loads data word `w` of page `p` (`Relaxed`), absent page ⇒ 0,
                //   and applies the polarity complement.
                unsafe { (*col).match_word(p, w, !want) }
            };
            acc &= contribution;
            if acc == 0 {
                break;
            }
        }
        acc
    }
}

/// First/last partial-word range mask for word base `word_base` (C4): clears
/// bits below `range_start` and bits at/above `range_end`. Whole interior words
/// pass through `u64::MAX`. Shared by the composite run walk.
#[inline]
fn range_mask(word_base: usize, range_start: usize, range_end: usize) -> u64 {
    let mut mask = u64::MAX;
    if range_start > word_base {
        let lo = range_start - word_base;
        debug_assert!(lo < 64, "range_mask low shift out of range");
        mask &= u64::MAX << lo;
    }
    if range_end < word_base + 64 {
        let hi = range_end.saturating_sub(word_base);
        if hi == 0 {
            return 0;
        }
        debug_assert!(hi < 64, "range_mask high shift out of range");
        mask &= u64::MAX >> (64 - hi);
    }
    mask
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
