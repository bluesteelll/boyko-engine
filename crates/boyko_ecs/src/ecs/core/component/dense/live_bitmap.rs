//! A growable, word-array per-slot liveness bitmap — the [`DenseStore`]'s own
//! bookkeeping (Dense plan, W3).
//!
//! None of the existing engine bitsets fit the per-slot liveness role: the
//! `boyko_utils::BitSet` family is a single fixed-width integer (≤ 128 bits),
//! `BitSet256` is fixed 256, and `ArchetypeBitSet` is fixed 1024. The dense
//! column's slot space is sized by `reserve_rows` (up to `< u32::MAX`), so the
//! liveness oracle must be a heap-backed array of 64-bit words that grows with
//! the column.
//!
//! The raw words pointer is what [`DenseSolveView`] holds (`live: *const u64`)
//! to run its `debug_assert!(is_live(slot))` guard in `row_ptr` — a read-only
//! liveness oracle, never mutated through the solve view.
//!
//! [`DenseStore`]: super::dense_store::DenseStore
//! [`DenseSolveView`]: super::views::DenseSolveView

/// Number of slots addressed by one `u64` word.
const BITS_PER_WORD: usize = 64;

/// Heap-backed per-slot liveness bitmap, addressed `bit i -> words[i >> 6]` at
/// bit `i & 63`.
///
/// This is the dense storage's own structural bookkeeping (the legitimate L1
/// `std::Vec` exception called out by the Dense plan). Word 0 carries slots
/// `0..=63`, word 1 carries slots `64..=127`, and so on. The array grows by
/// whole words as the column's slot space grows; cleared bits read back zero.
pub(crate) struct LiveBitmap {
    /// Little-endian word array. `len() * 64` is the addressable slot ceiling;
    /// growth appends zero words so a freshly addressable slot reads as dead.
    words: Vec<u64>,
}

impl LiveBitmap {
    /// Creates a bitmap sized to address at least `slots` slots without a
    /// further allocation. All bits start clear (dead).
    #[inline]
    pub(crate) fn with_capacity(slots: usize) -> Self {
        let n_words = slots.div_ceil(BITS_PER_WORD);
        Self {
            words: vec![0u64; n_words],
        }
    }

    /// Ensures `slot` is addressable, appending zero words if needed.
    ///
    /// Cold relative to `set`/`clear`: only the first touch of a fresh word
    /// range pays the (amortized) `Vec` growth.
    #[inline]
    fn ensure_addressable(&mut self, slot: usize) {
        let word = slot >> 6;
        if word >= self.words.len() {
            self.words.resize(word + 1, 0u64);
        }
    }

    /// Marks `slot` live (sets its bit), growing the word array if `slot` lies
    /// beyond the current addressable range.
    #[inline]
    pub(crate) fn set(&mut self, slot: usize) {
        self.ensure_addressable(slot);
        self.words[slot >> 6] |= 1u64 << (slot & 63);
    }

    /// Marks `slot` dead (clears its bit).
    ///
    /// `slot` must already be addressable (it was set live before being
    /// cleared) — debug-asserted.
    #[inline]
    pub(crate) fn clear(&mut self, slot: usize) {
        debug_assert!(
            (slot >> 6) < self.words.len(),
            "LiveBitmap::clear: slot {slot} beyond addressable range"
        );
        self.words[slot >> 6] &= !(1u64 << (slot & 63));
    }

    /// Returns `true` iff `slot` is live. Slots beyond the addressable range
    /// read as dead (never set).
    #[inline]
    pub(crate) fn test(&self, slot: usize) -> bool {
        let word = slot >> 6;
        if word >= self.words.len() {
            return false;
        }
        (self.words[word] >> (slot & 63)) & 1 == 1
    }

    /// Clears every bit in O(words) without releasing the backing allocation.
    /// Used by `compact()` to rebuild liveness from the canonical order.
    #[inline]
    pub(crate) fn clear_all(&mut self) {
        self.words.iter_mut().for_each(|w| *w = 0);
    }

    /// Returns the raw words base pointer — the read-only liveness oracle the
    /// [`DenseSolveView`] caches (`live: *const u64`).
    ///
    /// [`DenseSolveView`]: super::views::DenseSolveView
    #[inline]
    pub(crate) fn words_ptr(&self) -> *const u64 {
        self.words.as_ptr()
    }

    /// Number of addressable words. Multiplied by 64 it is the slot ceiling the
    /// solve view bounds-checks against alongside the column length.
    #[inline]
    pub(crate) fn word_count(&self) -> usize {
        self.words.len()
    }

    /// Reads the liveness of `slot` through a raw words pointer.
    ///
    /// The shared liveness primitive between [`LiveBitmap::test`] (owned, safe)
    /// and [`DenseSolveView::is_live`] (cached pointer, `unsafe`): both lower to
    /// the identical word-load + bit-test so the debug_assert in the solve
    /// view's `row_ptr` is the same oracle the store's structural ops maintain.
    ///
    /// # Safety
    /// * `words` points at the live `LiveBitmap`'s array and `slot >> 6 <
    ///   word_count` — the caller (the solve view) guarantees both: the view's
    ///   lifetime `'a` borrows the store (so the array is not freed or
    ///   reallocated for the view's life), and the slot is bounded by the
    ///   column length the view also caches.
    ///
    /// [`DenseSolveView::is_live`]: super::views::DenseSolveView::is_live
    #[inline]
    pub(crate) unsafe fn test_raw(words: *const u64, slot: usize) -> bool {
        // SAFETY: the caller guarantees `slot >> 6 < word_count` and that
        // `words` is the live array base, so `add(slot >> 6)` lands inside the
        // `Vec<u64>` allocation and the read is of an initialized word.
        let w = unsafe { *words.add(slot >> 6) };
        (w >> (slot & 63)) & 1 == 1
    }
}
