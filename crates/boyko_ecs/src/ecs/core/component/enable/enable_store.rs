//! Paged enable-bit storage: `EnablePage`, `EnableColumn`, `EnableStore` (D1).
//!
//! # Layout (Decision D1)
//!
//! For each `EnableTag` id, each archetype that has it toggled owns a lazily
//! allocated **paged** [`EnableColumn`]: a page directory
//! (`Box<[Option<Box<EnablePage>>]>`) where each [`EnablePage`] is
//! `[AtomicU64; 64]` = 512 B covering 4096 rows. The bit's home is
//! `(archetype, row)`, exactly like component data and Phase-10 tick columns —
//! so it travels correctly through the existing row-copy / swap-remove loop and
//! never leaks across entity recycling.
//!
//! Index arithmetic (row = a pool `unit_index`):
//! - page index    = `row >> 12`        (`row / ROWS_PER_PAGE`)
//! - word in page  = `(row >> 6) & 63`  (`(row / 64) % WORDS_PER_PAGE`)
//! - bit in word   = `row & 63`
//!
//! A column allocates ONLY the pages a toggle touches (first toggle into a
//! 4096-row range = one 512 B page), so no single allocation exceeds one page.
//!
//! # Atomics & ordering (D8 / Multithreading model)
//!
//! Every bit word is an `AtomicU64`. In v1 a toggle requires `&mut EcsMaster`
//! and runs in the structural/apply window where no worker is live, so reads
//! and writes use `Relaxed`: `load(Relaxed)` is a plain `mov`, and there is no
//! concurrent toggler to synchronize against. `AtomicU64` is the lock-free
//! primitive that lets the D7 worker-marking seam add real `Acquire`/`Release`
//! later at zero v1 read cost. Interior mutability through `AtomicU64` is sound
//! under Tree Borrows exactly as Phase-10's `UnsafeCell<Tick>`.
//!
//! [`EnableColumn`]: crate::ecs::core::component::enable::enable_store::EnableColumn

use std::sync::atomic::{AtomicU64, Ordering};

use crate::ecs::identifiers::primitives::ComponentId;

/// `AtomicU64` words per [`EnablePage`]. `64 words * 64 bits = 4096 rows/page`.
pub(crate) const WORDS_PER_PAGE: usize = 64;

/// Rows covered by one [`EnablePage`] (`WORDS_PER_PAGE * 64`). Page boundary.
pub(crate) const ROWS_PER_PAGE: usize = WORDS_PER_PAGE * 64;

/// `log2(ROWS_PER_PAGE)` — shift to turn a row into its page index.
const ROWS_PER_PAGE_LOG2: u32 = 12;

/// Inline capacity of [`SmallList4`] before spilling to the heap.
const SMALL_LIST_INLINE: usize = 4;

// ─────────────────────────────────────────────────────────────────────────────
// EnablePage
// ─────────────────────────────────────────────────────────────────────────────

/// One 512 B page of enable bits: `[AtomicU64; 64]` covering 4096 rows.
///
/// `#[repr(C, align(64))]` pins the page to a single 64 B-cache-line-aligned
/// allocation unit (D1 sub-decision: backing & regrow). The page is heap-boxed
/// and stored behind an `Option<Box<EnablePage>>` directory slot, allocated only
/// on first toggle into its row range.
#[repr(C, align(64))]
pub(crate) struct EnablePage([AtomicU64; WORDS_PER_PAGE]);

// D1 TRIPWIRE: one page is exactly one 512 B allocation unit (64 × 8 B atomics =
// 4096 rows). A future `WORDS_PER_PAGE` edit that breaks the size/align invariant
// fails the build (matching the `ComponentLayout == 56` const-assert convention).
const _: () = assert!(core::mem::size_of::<EnablePage>() == 512);
const _: () = assert!(core::mem::align_of::<EnablePage>() == 64);

impl EnablePage {
    /// Allocates a zeroed page (all bits clear). Used on first toggle into a
    /// page's 4096-row range.
    #[inline]
    fn new_boxed() -> Box<Self> {
        // `AtomicU64` is not `Copy`, so build the array element-wise. The
        // compiler lowers this to a `bzero`/`memset` of the heap allocation.
        Box::new(EnablePage(std::array::from_fn(|_| AtomicU64::new(0))))
    }

    /// Tests the bit for `local_row` (already reduced to `0..ROWS_PER_PAGE`).
    #[inline]
    fn test_local(&self, local_row: usize) -> bool {
        debug_assert!(local_row < ROWS_PER_PAGE, "local row out of page range");
        let word = (local_row >> 6) & (WORDS_PER_PAGE - 1);
        let bit = local_row & 63;
        // Relaxed: no concurrent toggler in v1 (D8); load is a plain `mov`.
        (self.0[word].load(Ordering::Relaxed) >> bit) & 1 == 1
    }

    /// Sets the bit for `local_row` to `value`.
    #[inline]
    fn set_local(&self, local_row: usize, value: bool) {
        debug_assert!(local_row < ROWS_PER_PAGE, "local row out of page range");
        let word = (local_row >> 6) & (WORDS_PER_PAGE - 1);
        let mask = 1u64 << (local_row & 63);
        // Relaxed: &mut-exclusive apply window in v1 (D8). `fetch_or`/`fetch_and`
        // keep the other 63 bits in the word intact (other rows in the block).
        if value {
            self.0[word].fetch_or(mask, Ordering::Relaxed);
        } else {
            self.0[word].fetch_and(!mask, Ordering::Relaxed);
        }
    }

    /// Sets `local_row` to `value`, returning the touched word index plus the
    /// word's post-write occupancy so the owning [`EnableColumn`] can keep its
    /// `summary` slot in lockstep (Decision 2 / C3).
    ///
    /// The post-occupancy is derived from the atomic RMW return value — there is
    /// no extra `load`, even on the clear path: `fetch_or` returns the old word
    /// (OR `mask` reconstructs the new), `fetch_and` likewise (AND `!mask`).
    #[inline]
    fn set_local_reporting(&self, local_row: usize, value: bool) -> (usize, WordState) {
        debug_assert!(local_row < ROWS_PER_PAGE, "local row out of page range");
        let word = (local_row >> 6) & (WORDS_PER_PAGE - 1);
        let mask = 1u64 << (local_row & 63);
        // Relaxed: &mut-exclusive apply window in v1 (D8). The RMW return is the
        // PRE-write word; reconstruct the post-write word without a second load.
        let post = if value {
            self.0[word].fetch_or(mask, Ordering::Relaxed) | mask
        } else {
            self.0[word].fetch_and(!mask, Ordering::Relaxed) & !mask
        };
        (word, if post == 0 { WordState::Empty } else { WordState::NonEmpty })
    }

    /// Loads the raw data word `word` (`0..WORDS_PER_PAGE`) for the run walk.
    /// Used by [`EnabledRuns`] / `EnableTermCols::for_each_run` to build the
    /// match-bitmap of a 64-row block.
    #[inline]
    fn data_word(&self, word: usize) -> u64 {
        debug_assert!(word < WORDS_PER_PAGE, "data word out of page range");
        // Relaxed: no concurrent toggler in v1 (D8); load is a plain `mov`.
        self.0[word].load(Ordering::Relaxed)
    }
}

/// Post-write occupancy of a touched 64-bit data word, reported by
/// [`EnablePage::set_local_reporting`] so the column can maintain its summary
/// (Decision 2). Storage-less; lowered to a single bool comparison.
#[derive(Clone, Copy, PartialEq, Eq)]
enum WordState {
    /// The word is all-zero after the write (no live bit in this 64-row block).
    Empty,
    /// The word has ≥ 1 set bit after the write.
    NonEmpty,
}

// ─────────────────────────────────────────────────────────────────────────────
// EnableColumn
// ─────────────────────────────────────────────────────────────────────────────

/// A lazily-paged, row-indexed enable-bit column for one `(archetype, tag)`.
///
/// The page directory has one slot per 4096-row block; a [`EnablePage`] is
/// allocated only when a row in its range is first toggled (D1: caps any single
/// allocation at 512 B). The directory is regrown when the owning pool's
/// reserved row count crosses a page boundary (the regrow re-`Box`es the 16 B
/// directory entries and moves the existing `Box<EnablePage>` pointers — no page
/// data is copied).
#[repr(C)]
pub(crate) struct EnableColumn {
    /// Page directory: `pages[row >> 12]` is the page for that row (or `None` if
    /// no row in its range has been toggled yet).
    pages: Box<[Option<Box<EnablePage>>]>,
    /// Word-occupancy summary (Decision 1/3): one [`AtomicU64`] per directory
    /// slot. `summary[p]` bit `w` (`0..WORDS_PER_PAGE`) is set iff data word `w`
    /// of page `p` has ≥ 1 set bit; an absent page contributes a 0 word.
    ///
    /// INVARIANT: `summary.len() == pages.len()` ALWAYS — the two grow in
    /// lockstep (`new` / `ensure_directory`), so `summary[p]` is a valid in-range
    /// slot for every directory slot, including `None` pages (correctly 0). The
    /// run walk reads it to skip fully-non-matching 64-row words / 4096-row pages
    /// without touching page data.
    summary: Box<[AtomicU64]>,
}

/// Number of 4096-row pages needed to cover `rows` rows (≥ 1, so a fresh column
/// always has at least one directory slot).
#[inline]
const fn pages_for_rows(rows: usize) -> usize {
    // ceil(rows / ROWS_PER_PAGE), with a floor of 1 directory slot.
    let pages = rows.div_ceil(ROWS_PER_PAGE);
    if pages == 0 { 1 } else { pages }
}

impl EnableColumn {
    /// Creates an empty column whose directory covers `reserve_rows` rows. No
    /// page is allocated until the first toggle (`get_or_alloc_page`).
    #[inline]
    pub(crate) fn new(reserve_rows: usize) -> Self {
        let len = pages_for_rows(reserve_rows);
        let mut dir: Vec<Option<Box<EnablePage>>> = Vec::with_capacity(len);
        dir.resize_with(len, || None);
        // Summary grows in lockstep with the directory; a fresh slot is all-zero
        // (no page ⇒ no live bit), matching "never allocate to store a 0".
        let mut summary: Vec<AtomicU64> = Vec::with_capacity(len);
        summary.resize_with(len, || AtomicU64::new(0));
        EnableColumn {
            pages: dir.into_boxed_slice(),
            summary: summary.into_boxed_slice(),
        }
    }

    /// Tests the enable bit for `row`. A `None` page (never toggled in that
    /// 4096-row range) reads as `false` (all-disabled). Hot read path.
    #[inline]
    pub(crate) fn test(&self, row: usize) -> bool {
        let page_idx = row >> ROWS_PER_PAGE_LOG2;
        match self.pages.get(page_idx) {
            // No directory slot or no page yet ⇒ the bit is clear.
            Some(Some(page)) => page.test_local(row & (ROWS_PER_PAGE - 1)),
            _ => false,
        }
    }

    /// Ensures the directory has a slot for `page_idx`, regrowing it if the
    /// owning pool grew past the current capacity (D1 sub-decision: regrow moves
    /// `Box` pointers, no page-data copy).
    #[cold]
    fn ensure_directory(&mut self, page_idx: usize) {
        if page_idx < self.pages.len() {
            return;
        }
        let new_len = page_idx + 1;
        let old = std::mem::take(&mut self.pages);
        let mut dir: Vec<Option<Box<EnablePage>>> = old.into_vec();
        dir.resize_with(new_len, || None);
        self.pages = dir.into_boxed_slice();
        // Lockstep regrow (Decision 3): keep `summary.len() == pages.len()`. New
        // tail slots are 0 (their pages are absent ⇒ no live bit).
        let old_summary = std::mem::take(&mut self.summary);
        let mut summary: Vec<AtomicU64> = old_summary.into_vec();
        summary.resize_with(new_len, || AtomicU64::new(0));
        self.summary = summary.into_boxed_slice();
        debug_assert_eq!(
            self.summary.len(),
            self.pages.len(),
            "summary/pages length divergence after ensure_directory (Decision 3 INV)"
        );
    }

    /// Returns the page for `page_idx`, allocating a zeroed 512 B page on first
    /// touch (the cold path). The directory is regrown first if needed.
    #[cold]
    fn get_or_alloc_page(&mut self, page_idx: usize) -> &EnablePage {
        self.ensure_directory(page_idx);
        let slot = &mut self.pages[page_idx];
        if slot.is_none() {
            *slot = Some(EnablePage::new_boxed());
        }
        // `slot` was just ensured `Some`; the invariant is local to this fn.
        slot.as_deref()
            .expect("invariant: page slot set immediately above")
    }

    /// Sets the enable bit for `row` to `value`, allocating the page on first
    /// touch. `reserve_rows` is the owning pool's reserved row count, used to
    /// regrow the directory if `row` lies beyond the current capacity.
    pub(crate) fn set(&mut self, row: usize, value: bool, reserve_rows: usize) {
        debug_assert!(
            row < reserve_rows,
            "enable-bit row {row} >= reserve_rows {reserve_rows} (D1 inv 2)"
        );
        let page_idx = row >> ROWS_PER_PAGE_LOG2;
        let page = self.get_or_alloc_page(page_idx);
        let (word, post) = page.set_local_reporting(row & (ROWS_PER_PAGE - 1), value);
        // Summary maintenance (Decision 2 / C3). `get_or_alloc_page` ensured the
        // directory (and thus `summary[page_idx]`) is in range — `set` always
        // allocates the page, even for a clear.
        let bit = 1u64 << word;
        if value {
            // C3(b): a set is UNCONDITIONAL — `fetch_or(mask)|mask != 0` always
            // (mask != 0), so the word is non-empty; no predicted branch.
            self.summary[page_idx].fetch_or(bit, Ordering::Relaxed);
        } else if let WordState::Empty = post {
            // A clear empties the summary bit only when the whole word emptied.
            self.summary[page_idx].fetch_and(!bit, Ordering::Relaxed);
        }
    }

    /// True if no page has been allocated yet (the column holds no live bits).
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.pages.iter().all(Option::is_none)
    }

    /// Swap-remove fix-up for a vacated `(removed, last)` row pair.
    ///
    /// Mirrors the component-byte `swap_remove`: the entity formerly at `last`
    /// is moving into `removed`'s slot, so `removed` must inherit `last`'s bit
    /// and `last` must be cleared.
    ///
    /// # Ordering (C2 / C4 — READ-first)
    ///
    /// The bit at `last` is **read before any write** (step 1), so writing
    /// `removed` (step 2) cannot corrupt the value when `removed == last - 1` or
    /// when both fall in the same word; the final `clear(last)` (step 3) then
    /// vacates the popped slot. On `removed == last` (the last live row was
    /// removed) the net effect is a single `clear(last)`.
    pub(crate) fn swap_remove_bit(&mut self, removed: usize, last: usize, reserve_rows: usize) {
        debug_assert!(
            removed <= last && last < reserve_rows,
            "swap_remove_bit({removed}, {last}) out of range (reserve_rows {reserve_rows})"
        );
        // (1) READ last's bit BEFORE any write (D1 inv 6 / C2-r5).
        let bit = self.test(last);
        // (2) Write removed = last's bit. Allocates the target page if needed
        //     (the bit may move into a not-yet-toggled page range).
        if bit {
            self.set(removed, true, reserve_rows);
        } else {
            // Clear only if a page exists there; never allocate to write a zero.
            self.clear_if_present(removed);
        }
        // (3) Clear the popped slot.
        self.clear_if_present(last);
    }

    /// Clears `row`'s bit only if its page is already allocated. Avoids
    /// allocating a page just to store a zero (a clear into a never-toggled
    /// range is already the default).
    #[inline]
    fn clear_if_present(&mut self, row: usize) {
        let page_idx = row >> ROWS_PER_PAGE_LOG2;
        // C3(a): the summary update fires ONLY inside the `Some(Some(page))`
        // branch. A clear into an absent page touches no bit, and that page's
        // summary slot is already 0 (lockstep) — nothing to do.
        if let Some(Some(page)) = self.pages.get(page_idx) {
            let (word, post) = page.set_local_reporting(row & (ROWS_PER_PAGE - 1), false);
            if let WordState::Empty = post {
                // The page existed ⇒ `summary[page_idx]` is a valid in-range slot
                // (index lockstep). Clear the word bit iff the word emptied.
                self.summary[page_idx].fetch_and(!(1u64 << word), Ordering::Relaxed);
            }
        }
    }

    /// Recomputes `summary[p]` from page `p`'s live bits and asserts it equals
    /// the stored summary, for every directory slot (C2 oracle).
    ///
    /// An absent page must have a 0 summary slot; a present page's summary bit
    /// `w` must equal `data_word(w) != 0`. Called after a structural mutation
    /// completes (e.g. the whole `swap_remove_bit`) to catch a desync on either
    /// touched page of a cross-page swap. Compiled out in release.
    #[cfg(debug_assertions)]
    pub(crate) fn debug_assert_column_summary_consistent(&self) {
        debug_assert_eq!(
            self.summary.len(),
            self.pages.len(),
            "summary/pages length divergence (Decision 3 INV)"
        );
        for (p, slot) in self.pages.iter().enumerate() {
            let stored = self.summary[p].load(Ordering::Relaxed);
            let expected = match slot {
                Some(page) => {
                    let mut bits = 0u64;
                    for w in 0..WORDS_PER_PAGE {
                        if page.data_word(w) != 0 {
                            bits |= 1u64 << w;
                        }
                    }
                    bits
                }
                None => 0,
            };
            debug_assert_eq!(
                stored, expected,
                "summary desync at page {p}: stored {stored:#x} != recomputed {expected:#x}"
            );
        }
    }

    /// Returns an enabled-RUN iterator over MATCHING rows in
    /// `[range_start, range_end)` (Decision 4 / W1 / C4).
    ///
    /// `invert == false` (Enabled / `with_enabled`): a row matches iff its bit is
    /// SET. `invert == true` (Disabled / `without_enabled`): a row matches iff its
    /// bit is CLEAR — an ABSENT page is then one continuous matching span.
    ///
    /// Every yielded `(start, len)` run is ⊆ `[range_start, range_end)`
    /// (INV-RANGE) and maximal (INV-COALESCE: a run terminates only at the first
    /// non-matching row or at `range_end`, crossing word and page boundaries
    /// without flushing). A fully-matching multi-page range over `[0, n)` yields
    /// exactly one run `(0, n)`, byte-identical to a no-filter single chunk.
    #[inline]
    pub(crate) fn enabled_runs(
        &self,
        range_start: usize,
        range_end: usize,
        invert: bool,
    ) -> EnabledRuns<'_> {
        debug_assert!(range_start <= range_end, "enabled_runs: range_start > range_end");
        EnabledRuns {
            col: self,
            next_row: range_start,
            range_start,
            range_end,
            invert,
        }
    }

    /// The match-bitmap of 64-row block `(p, w)` for the run walk.
    ///
    /// `invert == false`: the raw data word (set bit = matching). `invert ==
    /// true`: the complement (clear bit = matching), so an absent page yields
    /// `u64::MAX` (all-matching). NEVER allocates; an absent page reads 0 data.
    #[inline]
    pub(crate) fn match_word(&self, page_idx: usize, word: usize, invert: bool) -> u64 {
        let data = match self.pages.get(page_idx) {
            Some(Some(page)) => page.data_word(word),
            _ => 0,
        };
        if invert { !data } else { data }
    }

    /// The summary word for directory slot `page_idx` (bit `w` set ⇒ data word
    /// `w` non-empty). An out-of-range / absent slot reads 0. Only meaningful for
    /// the `invert == false` coarse skip (set-bit occupancy).
    #[inline]
    pub(crate) fn summary_word(&self, page_idx: usize) -> u64 {
        match self.summary.get(page_idx) {
            Some(s) => s.load(Ordering::Relaxed),
            None => 0,
        }
    }
}

/// Words per [`EnablePage`] — re-exported `pub(crate)` so the multi-term
/// composite run walk (`EnableTermCols::for_each_run`) shares the page geometry.
pub(crate) const fn words_per_page() -> usize {
    WORDS_PER_PAGE
}

/// `log2(ROWS_PER_PAGE)` — `pub(crate)` for the composite run walk's page math.
pub(crate) const fn rows_per_page_log2() -> u32 {
    ROWS_PER_PAGE_LOG2
}

// ─────────────────────────────────────────────────────────────────────────────
// EnabledRuns — enabled-run iterator (Decision 4 / W1 / C4)
// ─────────────────────────────────────────────────────────────────────────────

/// Iterator over maximal contiguous MATCHING runs of one [`EnableColumn`] in
/// `[range_start, range_end)` (Decision 4). Yields `(start, len)`.
///
/// A single monotonic cursor (`next_row`) scans a match-bitmap stream and does
/// NOT restart per word or per page: a run is extended across word AND page
/// boundaries without flushing, terminating only at the first non-matching row
/// or at `range_end` (INV-COALESCE). Every yielded run is ⊆
/// `[range_start, range_end)` (INV-RANGE). The `summary` is consulted ONLY to
/// fast-forward over fully-non-matching words / pages while SEARCHING for a run
/// start (`invert == false`); it is never used to terminate an in-progress run.
pub(crate) struct EnabledRuns<'a> {
    col: &'a EnableColumn,
    /// Next row to consider; monotonic, crosses words/pages freely.
    next_row: usize,
    /// Lower bound carried for the INV-RANGE debug assert.
    range_start: usize,
    /// Exclusive upper bound — the SOLE hard terminator (C4/C5).
    range_end: usize,
    /// `true` = Disabled / `without_enabled` (matching = clear bit).
    invert: bool,
}

impl EnabledRuns<'_> {
    /// First/last partial-word range mask for word `(p, w)`: bits below the run's
    /// `range_start` (when this is the start word) and bits at/above `range_end`
    /// (when this is the end word) are cleared. Whole interior words pass through
    /// `u64::MAX` (no masking). C4: applied BEFORE any `trailing_*`.
    #[inline]
    fn range_mask(&self, word_base: usize) -> u64 {
        let mut mask = u64::MAX;
        // Clear low bits before `range_start` (only affects the first word).
        if self.range_start > word_base {
            let lo = self.range_start - word_base;
            // lo is in 1..=63 here (>=64 would mean word_base+64 <= range_start,
            // i.e. this word is entirely before the range — never scanned).
            debug_assert!(lo < 64, "range_mask low shift out of range");
            mask &= u64::MAX << lo;
        }
        // Clear high bits at/above `range_end` (only affects the last word).
        if self.range_end < word_base + 64 {
            let hi = self.range_end.saturating_sub(word_base);
            // hi in 0..=63: keep bits [0, hi).
            if hi == 0 {
                return 0;
            }
            debug_assert!(hi < 64, "range_mask high shift out of range");
            mask &= u64::MAX >> (64 - hi);
        }
        mask
    }
}

impl Iterator for EnabledRuns<'_> {
    type Item = (usize, usize);

    // Left un-annotated (W1 minor): a single call site per driver; LLVM decides.
    fn next(&mut self) -> Option<(usize, usize)> {
        // ── Phase 1: find a run start (skip non-matching rows). ──────────────
        let run_start = loop {
            if self.next_row >= self.range_end {
                return None;
            }
            let p = self.next_row >> ROWS_PER_PAGE_LOG2;
            let w = (self.next_row >> 6) & (WORDS_PER_PAGE - 1);
            let word_base = (self.next_row >> 6) << 6;

            // Coarse fast-forward over fully-non-matching set-content (invert ==
            // false only — an absent/all-zero page is all-MATCHING when inverted,
            // handled by the per-word scan below, never skipped here).
            if !self.invert {
                let summary = self.col.summary_word(p);
                if summary == 0 {
                    // Whole page empty ⇒ no set bit anywhere in it.
                    self.next_row = ((p + 1) << ROWS_PER_PAGE_LOG2).min(self.range_end);
                    continue;
                }
                if (summary >> w) & 1 == 0 {
                    // This 64-row word has no set bit.
                    self.next_row = (word_base + 64).min(self.range_end);
                    continue;
                }
            }

            // Mask off bits BELOW the cursor's in-word position so that
            // `trailing_zeros()` cannot return an already-consumed bit in this
            // same word (BUG-1: a stale low bit rewinds the cursor → infinite
            // loop). The range mask alone encodes `range_start`/`range_end`, not
            // the cursor, which can stop mid-word past an earlier matching bit.
            let bit_in_word = self.next_row - word_base;
            let cursor_low_mask = if bit_in_word == 0 {
                u64::MAX
            } else {
                u64::MAX << bit_in_word
            };
            let m =
                self.col.match_word(p, w, self.invert) & self.range_mask(word_base) & cursor_low_mask;
            if m == 0 {
                // No match in this (range-masked) word at/after the cursor —
                // advance to the next word.
                self.next_row = (word_base + 64).min(self.range_end);
                continue;
            }
            // First matching bit at/after the cursor within this word.
            break word_base + m.trailing_zeros() as usize;
        };

        // ── Phase 2: extend the run maximally, clamped to range_end (C4). ────
        let mut cursor = run_start;
        let run_end = loop {
            let p = cursor >> ROWS_PER_PAGE_LOG2;
            let w = (cursor >> 6) & (WORDS_PER_PAGE - 1);
            let word_base = (cursor >> 6) << 6;
            let bit_in_word = cursor - word_base;

            let m = self.col.match_word(p, w, self.invert) & self.range_mask(word_base);
            // Matching bits at/above the cursor bit, packed to bit 0.
            let above = m >> bit_in_word;
            // Length of the contiguous matching block starting at `cursor`.
            let ones = above.trailing_ones() as usize;
            let block_end = cursor + ones;

            // The run stops inside this word unless it consumed every bit up to
            // the word's top (bit 63) AND the word's top row is < range_end.
            let word_top_row = word_base + 63;
            let reached_word_top = block_end > word_top_row;
            if !reached_word_top || block_end >= self.range_end {
                break block_end.min(self.range_end);
            }
            // Continue into the next word (word w+1, or word 0 of page p+1).
            cursor = word_base + 64;
            if cursor >= self.range_end {
                break self.range_end;
            }
        };

        debug_assert!(
            self.range_start <= run_start && run_end <= self.range_end,
            "EnabledRuns INV-RANGE violated: run [{run_start}, {run_end}) \
             not in [{}, {})",
            self.range_start,
            self.range_end
        );
        debug_assert!(run_end > run_start, "EnabledRuns yielded an empty run");
        self.next_row = run_end;
        Some((run_start, run_end - run_start))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// EnableStore
// ─────────────────────────────────────────────────────────────────────────────

/// All enable-bit columns owned by one archetype (parallel to its component
/// pools). Stored as an inline-4 small list of `(ComponentId, EnableColumn)`:
/// the dominant access is enumerate-all-allocated-columns (the swap-remove and
/// migration paths), not O(1) point lookup, and most archetypes use few tags.
#[repr(C)]
pub(crate) struct EnableStore {
    columns: SmallList4<(ComponentId, EnableColumn)>,
}

impl Default for EnableStore {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl EnableStore {
    /// Creates an empty store (no columns, no allocation).
    #[inline]
    pub(crate) fn new() -> Self {
        EnableStore {
            columns: SmallList4::new(),
        }
    }

    /// Returns the column for `cid` (a ≤4 linear scan), or `None` if this
    /// archetype has never toggled that tag.
    #[inline]
    pub(crate) fn column(&self, cid: ComponentId) -> Option<&EnableColumn> {
        self.columns
            .iter()
            .find(|(id, _)| *id == cid)
            .map(|(_, col)| col)
    }

    /// Returns a mutable column for `cid`, creating it (and noting the directory
    /// for `reserve_rows`) on first touch. The cold first-touch path is where
    /// the caller (Step 4/5) records `note_column_alloc` + the
    /// `enable_generation` bump (O2); this method only owns the storage.
    pub(crate) fn get_or_alloc_column(
        &mut self,
        cid: ComponentId,
        reserve_rows: usize,
    ) -> &mut EnableColumn {
        let idx = self.columns.iter().position(|(id, _)| *id == cid);
        match idx {
            Some(i) => &mut self.columns.get_mut(i).1,
            None => {
                self.push_new_column(cid, reserve_rows);
                // The column was pushed to the end.
                let last = self.columns.len() - 1;
                &mut self.columns.get_mut(last).1
            }
        }
    }

    /// Cold helper: appends a freshly-created column for `cid`.
    #[cold]
    fn push_new_column(&mut self, cid: ComponentId, reserve_rows: usize) {
        self.columns
            .push((cid, EnableColumn::new(reserve_rows)));
    }

    /// Applies the swap-remove fix-up to **every** allocated column for the
    /// vacated `(removed, last)` row pair (O1: fires once per structural op).
    ///
    /// Post-condition (debug): the vacated `removed` row holds `last`'s former
    /// bit and `last`'s bit is now clear, for each column.
    pub(crate) fn swap_remove_row(&mut self, removed: usize, last: usize, reserve_rows: usize) {
        for i in 0..self.columns.len() {
            let col = &mut self.columns.get_mut(i).1;
            // Capture the source bit BEFORE the fix-up for the post-condition
            // assert (compiled out in release).
            #[cfg(debug_assertions)]
            let expected = col.test(last);
            col.swap_remove_bit(removed, last, reserve_rows);
            #[cfg(debug_assertions)]
            {
                if removed != last {
                    debug_assert_eq!(
                        col.test(removed),
                        expected,
                        "swap_remove_row: removed row did not inherit last's bit"
                    );
                }
                debug_assert!(
                    !col.test(last),
                    "swap_remove_row: last row's bit not cleared"
                );
                // C2 oracle: the whole `swap_remove_bit` (set + up to two clears,
                // possibly across two pages) has completed; the summary must now
                // match the live bits on EVERY page (both touched pages of a
                // cross-page swap are covered by the full-column walk).
                col.debug_assert_column_summary_consistent();
            }
        }
    }

    /// Phase-1 migration READ (C4 / W3-r6): snapshots every allocated column's
    /// bit at `row` into `out` as **owned `(ComponentId, bool)` `Copy` values**.
    ///
    /// # Borrow-free invariant (W3-r6 / D1 inv 6)
    ///
    /// The snapshot stores plain `bool`s, never a reference into a source
    /// column. `out` therefore does not borrow `self` after this call returns,
    /// so its contents survive a later `swap_remove_row` that mutates the very
    /// columns just read — structurally it cannot be the NEW-1 dangling-slice
    /// class. `out` is cleared first so callers may reuse a scratch buffer.
    pub(crate) fn read_row_bits(&self, row: usize, out: &mut SmallList4<(ComponentId, bool)>) {
        out.clear();
        for (cid, col) in self.columns.iter() {
            out.push((*cid, col.test(row)));
        }
    }

    /// Phase-2 migration WRITE (C4): sets the target column's bit for `cid` at
    /// `row`, allocating the column/page on first touch. The cold column-create
    /// path is where the caller records `note_column_alloc` + the
    /// `enable_generation` bump (O2).
    #[inline]
    pub(crate) fn write_row_bit(
        &mut self,
        cid: ComponentId,
        row: usize,
        bit: bool,
        reserve_rows: usize,
    ) {
        // Never allocate a column or page just to write a clear bit (the default
        // for a never-toggled target is already 0).
        if !bit {
            if let Some(col) = self.column_mut(cid) {
                col.set(row, false, reserve_rows);
            }
            return;
        }
        self.get_or_alloc_column(cid, reserve_rows)
            .set(row, true, reserve_rows);
    }

    /// Mutable column lookup without allocating (≤4 scan).
    #[inline]
    fn column_mut(&mut self, cid: ComponentId) -> Option<&mut EnableColumn> {
        let idx = self.columns.iter().position(|(id, _)| *id == cid)?;
        Some(&mut self.columns.get_mut(idx).1)
    }

    /// True if this archetype owns no enable columns.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// Number of allocated columns (diagnostics / tests).
    #[inline]
    pub(crate) fn column_count(&self) -> usize {
        self.columns.len()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SmallList4 — inline-4 + heap spill
// ─────────────────────────────────────────────────────────────────────────────

/// A minimal small-vector: stores up to [`SMALL_LIST_INLINE`] elements inline,
/// spilling the entire contents to a heap `Vec` once the fifth is pushed.
///
/// Used where the element count is almost always ≤4 (enable columns per
/// archetype; enable bits snapshotted during migration) and the dominant
/// operation is push-then-enumerate. Not a general-purpose container: it
/// supports push, indexed access, in-order iteration, length, and clear — which
/// is all the enable-store paths need.
pub(crate) enum SmallList4<T> {
    /// Up to 4 elements stored inline.
    Inline { items: [Option<T>; SMALL_LIST_INLINE], len: usize },
    /// Spilled to the heap once the inline capacity was exceeded.
    Spilled(Vec<T>),
}

impl<T> SmallList4<T> {
    /// Creates an empty inline list.
    #[inline]
    pub(crate) fn new() -> Self {
        SmallList4::Inline {
            items: [const { None }; SMALL_LIST_INLINE],
            len: 0,
        }
    }

    /// Number of elements.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        match self {
            SmallList4::Inline { len, .. } => *len,
            SmallList4::Spilled(v) => v.len(),
        }
    }

    /// True if empty.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Appends `value`, spilling to the heap if the inline capacity is full.
    pub(crate) fn push(&mut self, value: T) {
        match self {
            SmallList4::Inline { items, len } => {
                if *len < SMALL_LIST_INLINE {
                    items[*len] = Some(value);
                    *len += 1;
                } else {
                    // Spill: move all inline elements into a Vec, then push.
                    let mut v = Vec::with_capacity(SMALL_LIST_INLINE * 2);
                    for slot in items.iter_mut() {
                        v.push(slot.take().expect("invariant: inline slots [0..len) are Some"));
                    }
                    v.push(value);
                    *self = SmallList4::Spilled(v);
                }
            }
            SmallList4::Spilled(v) => v.push(value),
        }
    }

    /// Returns a shared reference to the element at `index`.
    ///
    /// # Panics
    /// Panics if `index >= len()`.
    #[inline]
    pub(crate) fn get(&self, index: usize) -> &T {
        match self {
            SmallList4::Inline { items, len } => {
                debug_assert!(index < *len, "SmallList4 index {index} out of bounds (len {len})");
                items[index]
                    .as_ref()
                    .expect("invariant: inline slots [0..len) are Some")
            }
            SmallList4::Spilled(v) => &v[index],
        }
    }

    /// Returns a mutable reference to the element at `index`.
    ///
    /// # Panics
    /// Panics if `index >= len()`.
    #[inline]
    pub(crate) fn get_mut(&mut self, index: usize) -> &mut T {
        match self {
            SmallList4::Inline { items, len } => {
                debug_assert!(index < *len, "SmallList4 index {index} out of bounds (len {len})");
                items[index]
                    .as_mut()
                    .expect("invariant: inline slots [0..len) are Some")
            }
            SmallList4::Spilled(v) => &mut v[index],
        }
    }

    /// Finds the index of the first element matching `pred`.
    #[inline]
    pub(crate) fn position<P: FnMut(&T) -> bool>(&self, pred: P) -> Option<usize> {
        self.iter().position(pred)
    }

    /// Iterates the elements in insertion order.
    #[inline]
    pub(crate) fn iter(&self) -> SmallList4Iter<'_, T> {
        SmallList4Iter { list: self, pos: 0 }
    }

    /// Removes all elements, resetting to the inline representation.
    #[inline]
    pub(crate) fn clear(&mut self) {
        *self = SmallList4::new();
    }
}

/// In-order iterator over [`SmallList4`].
pub(crate) struct SmallList4Iter<'a, T> {
    list: &'a SmallList4<T>,
    pos: usize,
}

impl<'a, T> Iterator for SmallList4Iter<'a, T> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos < self.list.len() {
            let item = self.list.get(self.pos);
            self.pos += 1;
            Some(item)
        } else {
            None
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.list.len() - self.pos;
        (remaining, Some(remaining))
    }
}

impl<'a, T> ExactSizeIterator for SmallList4Iter<'a, T> {}

#[cfg(test)]
mod tests {
    use super::*;

    fn cid(n: usize) -> ComponentId {
        ComponentId(n)
    }

    // ── EnablePage / size pins ───────────────────────────────────────────────

    #[test]
    fn enable_page_is_512_bytes() {
        assert_eq!(std::mem::size_of::<EnablePage>(), 512, "page must be exactly 512 B (D1)");
        assert_eq!(std::mem::align_of::<EnablePage>(), 64, "page must be cache-line aligned");
        assert_eq!(ROWS_PER_PAGE, 4096);
        assert_eq!(WORDS_PER_PAGE, 64);
    }

    #[test]
    fn fresh_page_reads_all_clear() {
        let page = EnablePage::new_boxed();
        for row in [0usize, 1, 63, 64, 4095] {
            assert!(!page.test_local(row), "fresh page must read clear at {row}");
        }
    }

    #[test]
    fn page_set_and_clear_roundtrip() {
        let page = EnablePage::new_boxed();
        page.set_local(0, true);
        page.set_local(63, true);
        page.set_local(64, true);
        page.set_local(4095, true);
        assert!(page.test_local(0));
        assert!(page.test_local(63));
        assert!(page.test_local(64));
        assert!(page.test_local(4095));
        // Neighbours stay clear (per-bit isolation in the word).
        assert!(!page.test_local(1));
        assert!(!page.test_local(62));
        assert!(!page.test_local(65));
        assert!(!page.test_local(4094));

        page.set_local(63, false);
        assert!(!page.test_local(63));
        assert!(page.test_local(0), "clearing bit 63 must not disturb bit 0");
    }

    // ── EnableColumn ─────────────────────────────────────────────────────────

    #[test]
    fn column_lazy_no_page_until_toggle() {
        let mut col = EnableColumn::new(4096);
        assert!(col.is_empty(), "no page until first toggle");
        assert!(!col.test(0));
        assert!(!col.test(4095));
        col.set(10, true, 4096);
        assert!(!col.is_empty());
        assert!(col.test(10));
    }

    #[test]
    fn column_atomic_read_write_roundtrip() {
        let mut col = EnableColumn::new(8192);
        col.set(0, true, 8192);
        col.set(100, true, 8192);
        assert!(col.test(0));
        assert!(col.test(100));
        assert!(!col.test(1));
        col.set(0, false, 8192);
        assert!(!col.test(0));
        assert!(col.test(100));
    }

    #[test]
    fn column_page_boundary_toggle_4095_vs_4096() {
        // Row 4095 lives in page 0, row 4096 in page 1 — exercises the directory
        // index split (D1 page boundary).
        let mut col = EnableColumn::new(8192);
        col.set(4095, true, 8192);
        assert!(col.test(4095));
        assert!(!col.test(4096), "row 4096 is a different page, must be clear");

        col.set(4096, true, 8192);
        assert!(col.test(4096));
        assert!(col.test(4095), "toggling page 1 must not disturb page 0");

        col.set(4095, false, 8192);
        assert!(!col.test(4095));
        assert!(col.test(4096));
    }

    #[test]
    fn degenerate_large_archetype_page_alloc_le_512() {
        // A column reserving > 4096 rows must still allocate at most one 512 B
        // page per touched range (no 32 KB whole-archetype single alloc).
        let rows = 100_000usize;
        let mut col = EnableColumn::new(rows);
        // Touch one row deep in the column.
        col.set(99_999, true, rows);
        assert!(col.test(99_999));
        // Exactly one page allocated (only the touched 4096-row range).
        let allocated = (0..pages_for_rows(rows))
            .filter(|&p| {
                // a present page reads back the toggled bit only in its range
                let base = p * ROWS_PER_PAGE;
                col.pages[p].is_some() && base <= 99_999
            })
            .count();
        assert_eq!(allocated, 1, "only the touched page is allocated");
        // The page itself is 512 B (the size pin already asserts this).
        assert_eq!(std::mem::size_of::<EnablePage>(), 512);
    }

    #[test]
    fn swap_remove_bit_read_first_oracle_swapped() {
        // Oracle: removed inherits last's bit, last is cleared, READ-first.
        // Case A: last set, removed clear -> removed becomes set.
        let mut col = EnableColumn::new(4096);
        col.set(7, true, 4096); // last = 7 set
        // removed = 3 (clear)
        col.swap_remove_bit(3, 7, 4096);
        assert!(col.test(3), "removed must inherit last's set bit");
        assert!(!col.test(7), "last must be cleared");

        // Case B: last clear, removed set -> removed becomes clear.
        let mut col = EnableColumn::new(4096);
        col.set(3, true, 4096); // removed currently set
        col.swap_remove_bit(3, 7, 4096); // last(7) is clear
        assert!(!col.test(3), "removed must take last's clear bit (overwrite)");
        assert!(!col.test(7));

        // Case C: adjacent removed == last - 1, both set bits in same word —
        // READ-first guarantees correctness.
        let mut col = EnableColumn::new(4096);
        col.set(6, false, 4096);
        col.set(7, true, 4096);
        col.swap_remove_bit(6, 7, 4096);
        assert!(col.test(6), "adjacent: removed inherits last");
        assert!(!col.test(7));
    }

    #[test]
    fn swap_remove_bit_pop_branch_removed_equals_last() {
        // O1-r7 Last/pop: removed == last -> single clear(last).
        let mut col = EnableColumn::new(4096);
        col.set(5, true, 4096);
        col.swap_remove_bit(5, 5, 4096);
        assert!(!col.test(5), "pop branch clears the popped bit");
    }

    #[test]
    fn swap_remove_bit_cross_page() {
        // last in page 1, removed in page 0 — fix-up must allocate the target
        // page and move the bit across pages.
        let mut col = EnableColumn::new(8192);
        col.set(5000, true, 8192); // last in page 1
        col.swap_remove_bit(10, 5000, 8192); // removed in page 0
        assert!(col.test(10), "bit moved into page 0");
        assert!(!col.test(5000));
    }

    // ── EnableStore ──────────────────────────────────────────────────────────

    #[test]
    fn store_empty_by_default() {
        let store = EnableStore::new();
        assert!(store.is_empty());
        assert_eq!(store.column_count(), 0);
        assert!(store.column(cid(0)).is_none());
    }

    #[test]
    fn store_get_or_alloc_then_lookup() {
        let mut store = EnableStore::new();
        store.get_or_alloc_column(cid(3), 4096).set(2, true, 4096);
        assert_eq!(store.column_count(), 1);
        assert!(store.column(cid(3)).unwrap().test(2));
        assert!(store.column(cid(4)).is_none());
        // Re-alloc returns the same column (no duplicate).
        store.get_or_alloc_column(cid(3), 4096).set(5, true, 4096);
        assert_eq!(store.column_count(), 1);
        assert!(store.column(cid(3)).unwrap().test(2));
        assert!(store.column(cid(3)).unwrap().test(5));
    }

    #[test]
    fn store_write_row_bit_clear_never_allocates() {
        let mut store = EnableStore::new();
        // Writing a clear bit to a non-existent column must not allocate one.
        store.write_row_bit(cid(9), 0, false, 4096);
        assert_eq!(store.column_count(), 0, "clear write must not allocate a column");
        // Writing a set bit allocates.
        store.write_row_bit(cid(9), 0, true, 4096);
        assert_eq!(store.column_count(), 1);
        assert!(store.column(cid(9)).unwrap().test(0));
    }

    #[test]
    fn store_read_row_bits_borrow_free_snapshot() {
        // W3-r6: the snapshot is owned (ComponentId, bool) Copy data; it must
        // survive a later mutation of the very columns it read.
        let mut store = EnableStore::new();
        store.get_or_alloc_column(cid(1), 4096).set(10, true, 4096);
        store.get_or_alloc_column(cid(2), 4096).set(10, false, 4096);

        let mut scratch: SmallList4<(ComponentId, bool)> = SmallList4::new();
        store.read_row_bits(10, &mut scratch);

        // Now mutate the source columns (the migration phase-3 swap analogue).
        store.swap_remove_row(10, 10, 4096);

        // The scratch snapshot is unaffected — it owns its bools.
        let snapshot: Vec<(ComponentId, bool)> = scratch.iter().copied().collect();
        assert!(snapshot.contains(&(cid(1), true)));
        assert!(snapshot.contains(&(cid(2), false)));
        // The source bit at row 10 was cleared by swap_remove_row.
        assert!(!store.column(cid(1)).unwrap().test(10));
    }

    #[test]
    fn store_read_row_bits_clears_scratch_for_reuse() {
        let mut store = EnableStore::new();
        store.get_or_alloc_column(cid(1), 4096).set(0, true, 4096);
        let mut scratch: SmallList4<(ComponentId, bool)> = SmallList4::new();
        scratch.push((cid(99), true)); // stale content
        store.read_row_bits(0, &mut scratch);
        let snap: Vec<_> = scratch.iter().copied().collect();
        assert_eq!(snap, vec![(cid(1), true)], "scratch must be cleared first");
    }

    #[test]
    fn store_swap_remove_row_all_columns() {
        let mut store = EnableStore::new();
        store.get_or_alloc_column(cid(1), 4096).set(7, true, 4096);
        store.get_or_alloc_column(cid(2), 4096).set(3, true, 4096); // at removed
        // removed=3, last=7: col1 (last set) -> 3 set, 7 clear; col2 (last
        // clear) -> 3 clear, 7 clear.
        store.swap_remove_row(3, 7, 4096);
        assert!(store.column(cid(1)).unwrap().test(3));
        assert!(!store.column(cid(1)).unwrap().test(7));
        assert!(!store.column(cid(2)).unwrap().test(3));
        assert!(!store.column(cid(2)).unwrap().test(7));
    }

    // ── SmallList4 ───────────────────────────────────────────────────────────

    #[test]
    fn small_list_inline_then_spill() {
        let mut list: SmallList4<u32> = SmallList4::new();
        assert!(list.is_empty());
        for i in 0..4 {
            list.push(i);
        }
        assert!(matches!(list, SmallList4::Inline { .. }), "4 elements stay inline");
        assert_eq!(list.len(), 4);
        list.push(4);
        assert!(matches!(list, SmallList4::Spilled(_)), "5th element spills");
        assert_eq!(list.len(), 5);
        for i in 0..5 {
            assert_eq!(*list.get(i as usize), i);
        }
        let collected: Vec<u32> = list.iter().copied().collect();
        assert_eq!(collected, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn small_list_position_and_clear() {
        let mut list: SmallList4<(u32, u32)> = SmallList4::new();
        list.push((1, 10));
        list.push((2, 20));
        assert_eq!(list.position(|(k, _)| *k == 2), Some(1));
        assert_eq!(list.position(|(k, _)| *k == 9), None);
        list.clear();
        assert!(list.is_empty());
        assert!(matches!(list, SmallList4::Inline { .. }), "clear resets to inline");
    }

    // ── EnabledRuns run-extraction (Decision 4) ──────────────────────────────

    /// Oracle: collect MATCHING rows via per-row `test`, expand the runs, assert
    /// they equal the bit-by-bit truth AND every run is bounded by range_end.
    fn runs_oracle(col: &EnableColumn, range_start: usize, range_end: usize, invert: bool) -> Vec<usize> {
        let mut rows = Vec::new();
        // Cap the run count so an EnabledRuns non-advancing (infinite-loop) bug is
        // reported as a clear assertion rather than an OOM crash. A correct run
        // walk yields at most `range_end - range_start` runs (1-row runs worst
        // case), so this cap never trips on correct code.
        let cap = (range_end - range_start) + 2;
        let mut prev_end = range_start;
        for (n, (s, l)) in col.enabled_runs(range_start, range_end, invert).enumerate() {
            assert!(n < cap, "EnabledRuns yielded > {cap} runs — non-advancing cursor (infinite loop)");
            assert!(s >= range_start && s + l <= range_end, "run [{s},{}) out of [{range_start},{range_end})", s + l);
            assert!(l > 0 && l <= range_end - range_start, "run len {l} absurd (range {range_start}..{range_end})");
            assert!(s >= prev_end, "runs not monotonic: run starts at {s} but previous ended at {prev_end}");
            prev_end = s + l;
            for r in s..s + l {
                rows.push(r);
            }
        }
        rows
    }

    fn expected_rows(col: &EnableColumn, range_start: usize, range_end: usize, invert: bool) -> Vec<usize> {
        (range_start..range_end).filter(|&r| col.test(r) != invert).collect()
    }

    #[test]
    fn enabled_runs_sparse_one_percent_bounded() {
        let n = 13_000usize;
        let mut col = EnableColumn::new(n);
        // splitmix64-ish deterministic 1% pattern.
        let mut x = 0xDEAD_BEEFu64;
        for r in 0..n {
            x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            if z.is_multiple_of(100) {
                col.set(r, true, n);
            }
        }
        let got = runs_oracle(&col, 0, n, false);
        let want = expected_rows(&col, 0, n, false);
        assert_eq!(got, want, "with_enabled sparse runs must equal bit truth");
    }

    #[test]
    fn enabled_runs_alternating_multipage_bounded() {
        let n = 8_192usize;
        let mut col = EnableColumn::new(n);
        for r in (0..n).step_by(2) {
            col.set(r, true, n);
        }
        let got = runs_oracle(&col, 0, n, false);
        let want = expected_rows(&col, 0, n, false);
        assert_eq!(got, want);
    }

    #[test]
    fn enabled_runs_invert_sparse_bounded() {
        let n = 13_000usize;
        let mut col = EnableColumn::new(n);
        // 99% set so the invert (without) complement is ~1% sparse.
        let mut x = 0x1234_5678u64;
        for r in 0..n {
            x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z ^= z >> 31;
            if !z.is_multiple_of(100) {
                col.set(r, true, n);
            }
        }
        let got = runs_oracle(&col, 0, n, true);
        let want = expected_rows(&col, 0, n, true);
        assert_eq!(got, want, "without_enabled runs must equal cleared-bit truth");
    }

    #[test]
    fn enabled_runs_not_multiple_of_64_last_word_clamped() {
        // n = 13000 = 203*64 + 8: the last word is partial (rows 12992..13000).
        let n = 13_000usize;
        let mut col = EnableColumn::new(n);
        col.set(12_995, true, n); // in the partial last word, < range_end
        col.set(12_999, true, n); // the very last live row
        let got = runs_oracle(&col, 0, n, false);
        assert_eq!(got, vec![12_995, 12_999]);
    }

    #[test]
    fn small_list_get_mut() {
        let mut list: SmallList4<u32> = SmallList4::new();
        list.push(5);
        *list.get_mut(0) = 9;
        assert_eq!(*list.get(0), 9);
    }
}
