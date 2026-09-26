//! The LOGICAL views over a step's contact state (L10, design `04-DESIGN-REV2.md` D6): what
//! [`ContactPairs::pairs`](crate::resources::ContactPairs::pairs),
//! [`Manifolds::manifolds`](crate::resources::Manifolds::manifolds) and
//! [`ConstraintGraph::island`](crate::resources::ConstraintGraph::island) hand a reader.
//!
//! With L10's sleep-skip a held island's pairs skip the narrowphase and its manifolds are kept
//! outside the stream the solver reads, yet every public view must still report them (ruling
//! B1). A view is therefore the stream ⊎ the kept store, merged in the canonical order the
//! sleep-skip off would emit — with no per-step copy: the merge happens on the read path.
//!
//! L10 C3b keeps held islands' manifolds (`held_store.rs`); L10 C3c's tree seam withholds held
//! islands' pairs from the stream (design 04 T3), which the pair view merges back. With nothing
//! held, every view is its stream element for element.
//!
//! A manifold view yields [`Manifold`] by value and implements no `Index`: a held manifold is
//! rebuilt from its island's row table, and a reader that mixed a view position with a solver
//! index would otherwise read the wrong manifold silently. The solver's own index space is
//! [`Manifolds::solver_manifolds`](crate::resources::Manifolds::solver_manifolds).

use core::fmt;
use core::iter::FusedIterator;

use crate::held_store::HeldView;
use crate::manifold::{BodyIndex, Manifold};
use crate::solver::warm_records::ord;

/// The first handle of the kept store: a manifold handle below it is a stream index into
/// [`Manifolds::solver_manifolds`](crate::resources::Manifolds::solver_manifolds), and
/// `HELD_BASE + slot` names kept manifold `slot` of a held island (L10 design 04 D6).
pub const HELD_BASE: u32 = 1 << 31;

/// The logical candidate pairs of a step, `(min, max)`-sorted and strictly increasing
/// ([`ContactPairs::pairs`](crate::resources::ContactPairs::pairs)): the broadphase's stream
/// merged with the pairs the tree broadphase withholds (L10 C3c, design 04 T3). The two are
/// disjoint and each strictly sorted, so the merge is a two-pointer walk with no allocation.
#[derive(Clone, Copy)]
pub struct PairsView<'a> {
    /// The broadphase's stream.
    stream: &'a [(BodyIndex, BodyIndex)],
    /// The pairs withheld from it.
    withheld: &'a [(BodyIndex, BodyIndex)],
}

impl<'a> PairsView<'a> {
    /// The view over `stream` ⊎ `withheld`, both strictly sorted and disjoint.
    #[inline]
    pub(crate) fn new(
        stream: &'a [(BodyIndex, BodyIndex)],
        withheld: &'a [(BodyIndex, BodyIndex)],
    ) -> Self {
        Self { stream, withheld }
    }

    /// The number of logical pairs. O(1).
    #[inline]
    pub fn len(&self) -> usize {
        self.stream.len() + self.withheld.len()
    }

    /// Whether the step has no candidate pair.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The pairs in `(min, max)` order.
    #[inline]
    pub fn iter(&self) -> PairsIter<'a> {
        PairsIter {
            stream: self.stream,
            withheld: self.withheld,
            s: 0,
            s_end: self.stream.len(),
            w: 0,
            w_end: self.withheld.len(),
        }
    }

    /// The `k`-th pair in `(min, max)` order, or `None` past the end. O(log² n) with pairs
    /// withheld (a binary search over the stream, each probe ranking a stream pair among the
    /// withheld ones), O(1) without.
    #[inline]
    pub fn get(&self, k: usize) -> Option<&'a (BodyIndex, BodyIndex)> {
        if self.withheld.is_empty() {
            return self.stream.get(k);
        }
        if k >= self.len() {
            return None;
        }
        let at = |s: usize| s + self.withheld.partition_point(|w| w < &self.stream[s]);
        let s = partition_point(0, self.stream.len(), |s| at(s) < k);
        if s < self.stream.len() && at(s) == k {
            Some(&self.stream[s])
        } else {
            self.withheld.get(k - s)
        }
    }

    /// Whether `pair` is a candidate pair of the step. O(log n): the view is strictly
    /// increasing (the broadphase's contract).
    #[inline]
    pub fn contains(&self, pair: &(BodyIndex, BodyIndex)) -> bool {
        self.stream.binary_search(pair).is_ok() || self.withheld.binary_search(pair).is_ok()
    }

    /// The pairs, copied into a `Vec` in `(min, max)` order: the in-crate tests' helper. The
    /// public API has no allocating method (UG-02: no new heap site in library code); a caller
    /// collects [`iter`](Self::iter).
    #[cfg(test)]
    pub(crate) fn to_vec(self) -> Vec<(BodyIndex, BodyIndex)> {
        self.iter().copied().collect()
    }
}

impl fmt::Debug for PairsView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl PartialEq for PairsView<'_> {
    /// Element-wise, in order.
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl Eq for PairsView<'_> {}

impl PartialEq<[(BodyIndex, BodyIndex)]> for PairsView<'_> {
    /// Element-wise, in order.
    fn eq(&self, other: &[(BodyIndex, BodyIndex)]) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl PartialEq<&[(BodyIndex, BodyIndex)]> for PairsView<'_> {
    /// Element-wise, in order.
    fn eq(&self, other: &&[(BodyIndex, BodyIndex)]) -> bool {
        *self == **other
    }
}

impl<const N: usize> PartialEq<[(BodyIndex, BodyIndex); N]> for PairsView<'_> {
    /// Element-wise, in order.
    fn eq(&self, other: &[(BodyIndex, BodyIndex); N]) -> bool {
        *self == other[..]
    }
}

impl<const N: usize> PartialEq<&[(BodyIndex, BodyIndex); N]> for PairsView<'_> {
    /// Element-wise, in order.
    fn eq(&self, other: &&[(BodyIndex, BodyIndex); N]) -> bool {
        *self == other[..]
    }
}

impl PartialEq<Vec<(BodyIndex, BodyIndex)>> for PairsView<'_> {
    /// Element-wise, in order.
    fn eq(&self, other: &Vec<(BodyIndex, BodyIndex)>) -> bool {
        *self == other[..]
    }
}

impl<'a> IntoIterator for PairsView<'a> {
    type Item = &'a (BodyIndex, BodyIndex);
    type IntoIter = PairsIter<'a>;

    #[inline]
    fn into_iter(self) -> PairsIter<'a> {
        self.iter()
    }
}

impl<'a> IntoIterator for &PairsView<'a> {
    type Item = &'a (BodyIndex, BodyIndex);
    type IntoIter = PairsIter<'a>;

    #[inline]
    fn into_iter(self) -> PairsIter<'a> {
        self.iter()
    }
}

/// The iterator of a [`PairsView`], in `(min, max)` order: the stream's `[s, s_end)` merged with
/// the withheld pairs' `[w, w_end)`, from both ends.
#[derive(Clone, Debug)]
pub struct PairsIter<'a> {
    /// The stream.
    stream: &'a [(BodyIndex, BodyIndex)],
    /// The withheld pairs.
    withheld: &'a [(BodyIndex, BodyIndex)],
    /// The stream's front cursor.
    s: usize,
    /// The stream's back cursor (one past).
    s_end: usize,
    /// The withheld pairs' front cursor.
    w: usize,
    /// The withheld pairs' back cursor (one past).
    w_end: usize,
}

impl<'a> Iterator for PairsIter<'a> {
    type Item = &'a (BodyIndex, BodyIndex);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let take_withheld = self.w < self.w_end
            && (self.s == self.s_end || self.withheld[self.w] < self.stream[self.s]);
        if take_withheld {
            self.w += 1;
            Some(&self.withheld[self.w - 1])
        } else if self.s < self.s_end {
            self.s += 1;
            Some(&self.stream[self.s - 1])
        } else {
            None
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = (self.s_end - self.s) + (self.w_end - self.w);
        (n, Some(n))
    }
}

impl DoubleEndedIterator for PairsIter<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        let take_withheld = self.w < self.w_end
            && (self.s == self.s_end || self.withheld[self.w_end - 1] > self.stream[self.s_end - 1]);
        if take_withheld {
            self.w_end -= 1;
            Some(&self.withheld[self.w_end])
        } else if self.s < self.s_end {
            self.s_end -= 1;
            Some(&self.stream[self.s_end])
        } else {
            None
        }
    }
}

impl ExactSizeIterator for PairsIter<'_> {}

impl FusedIterator for PairsIter<'_> {}

/// The logical solver manifolds of a step, in the order the sleep-skip off would emit them
/// ([`Manifolds::manifolds`](crate::resources::Manifolds::manifolds)): the stream merged with the
/// held store's kept manifolds by their canonical ordinal (body-body pairs by `(a, b)`, then SDF
/// contacts by row). Yields [`Manifold`] by value; no `Index`.
#[derive(Clone, Copy)]
pub struct ManifoldsView<'a> {
    /// The narrowphase's and the SDF stage's stream, sorted by ordinal.
    stream: &'a [Manifold],
    /// The held store (its `order` is sorted by ordinal).
    held: HeldView<'a>,
}

/// The canonical ordinal of a stream manifold (the stream is sorted by it).
#[inline]
fn ordinal(m: &Manifold) -> u64 {
    ord(m.body_a.0, m.body_b.0)
}

/// The first index of `lo..hi` for which `pred` is false (the range is partitioned by `pred`).
#[inline]
fn partition_point(mut lo: usize, mut hi: usize, pred: impl Fn(usize) -> bool) -> usize {
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if pred(mid) {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

impl<'a> ManifoldsView<'a> {
    /// The view over `stream` and the held store `held`.
    #[inline]
    pub(crate) fn new(stream: &'a [Manifold], held: HeldView<'a>) -> Self {
        Self { stream, held }
    }

    /// The number of logical manifolds. O(1).
    #[inline]
    pub fn len(&self) -> usize {
        self.stream.len() + self.held.order().len()
    }

    /// Whether the step has no solver manifold.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The manifolds, by value, in the order the sleep-skip off would emit them (a two-pointer
    /// merge; no allocation).
    #[inline]
    pub fn iter(&self) -> ManifoldsIter<'a> {
        ManifoldsIter {
            stream: self.stream,
            held: self.held,
            s: 0,
            s_end: self.stream.len(),
            o: 0,
            o_end: self.held.order().len(),
        }
    }

    /// The manifold at logical position `pos`, or `None` past the end. O(log² n): a binary search
    /// over the stream, each probe ranking the stream manifold among the kept ones.
    #[inline]
    pub fn get(&self, pos: usize) -> Option<Manifold> {
        if pos >= self.len() {
            return None;
        }
        let order = self.held.order();
        if order.is_empty() {
            return self.stream.get(pos).copied();
        }
        let at = |s: usize| s + self.held.rank_below(ordinal(&self.stream[s]));
        let s = partition_point(0, self.stream.len(), |s| at(s) < pos);
        if s < self.stream.len() && at(s) == pos {
            Some(self.stream[s])
        } else {
            order.get(pos - s).map(|&slot| self.held.manifold(slot))
        }
    }
}

impl fmt::Debug for ManifoldsView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<'a> IntoIterator for ManifoldsView<'a> {
    type Item = Manifold;
    type IntoIter = ManifoldsIter<'a>;

    #[inline]
    fn into_iter(self) -> ManifoldsIter<'a> {
        self.iter()
    }
}

impl<'a> IntoIterator for &ManifoldsView<'a> {
    type Item = Manifold;
    type IntoIter = ManifoldsIter<'a>;

    #[inline]
    fn into_iter(self) -> ManifoldsIter<'a> {
        self.iter()
    }
}

/// The iterator of a [`ManifoldsView`]: manifolds by value, in logical order — the stream's
/// `[s, s_end)` merged with the kept `order[o, o_end)` by ordinal, from both ends.
#[derive(Clone, Debug)]
pub struct ManifoldsIter<'a> {
    /// The stream.
    stream: &'a [Manifold],
    /// The held store.
    held: HeldView<'a>,
    /// The stream's front cursor.
    s: usize,
    /// The stream's back cursor (one past).
    s_end: usize,
    /// The kept order's front cursor.
    o: usize,
    /// The kept order's back cursor (one past).
    o_end: usize,
}

impl Iterator for ManifoldsIter<'_> {
    type Item = Manifold;

    #[inline]
    fn next(&mut self) -> Option<Manifold> {
        let order = self.held.order();
        let take_stream = if self.s == self.s_end {
            false
        } else if self.o == self.o_end {
            true
        } else {
            ordinal(&self.stream[self.s]) < self.held.ordinal(order[self.o])
        };
        if take_stream {
            self.s += 1;
            Some(self.stream[self.s - 1])
        } else if self.o < self.o_end {
            self.o += 1;
            Some(self.held.manifold(order[self.o - 1]))
        } else {
            None
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = (self.s_end - self.s) + (self.o_end - self.o);
        (n, Some(n))
    }
}

impl DoubleEndedIterator for ManifoldsIter<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<Manifold> {
        let order = self.held.order();
        let take_stream = if self.s == self.s_end {
            false
        } else if self.o == self.o_end {
            true
        } else {
            ordinal(&self.stream[self.s_end - 1]) > self.held.ordinal(order[self.o_end - 1])
        };
        if take_stream {
            self.s_end -= 1;
            Some(self.stream[self.s_end])
        } else if self.o < self.o_end {
            self.o_end -= 1;
            Some(self.held.manifold(order[self.o_end]))
        } else {
            None
        }
    }
}

impl ExactSizeIterator for ManifoldsIter<'_> {}

impl FusedIterator for ManifoldsIter<'_> {}

/// The manifolds of one island, as handles
/// ([`ConstraintGraph::island`](crate::resources::ConstraintGraph::island)): stream indices
/// below [`HELD_BASE`], and `HELD_BASE + slot` for the manifolds kept for a held island.
#[derive(Clone, Copy)]
pub struct IslandManifolds<'a> {
    /// The island's stream indices, ascending.
    stream: &'a [u32],
    /// The first kept slot of the island's held record (unread when `held_len == 0`).
    held_start: u32,
    /// The island's kept manifolds.
    held_len: u32,
}

impl<'a> IslandManifolds<'a> {
    /// The island over the stream indices `stream` and the kept slots
    /// `held_start .. held_start + held_len`.
    #[inline]
    pub(crate) fn new(stream: &'a [u32], held_start: u32, held_len: u32) -> Self {
        Self { stream, held_start, held_len }
    }

    /// The number of manifolds of the island. O(1).
    #[inline]
    pub fn len(&self) -> usize {
        self.stream.len() + self.held_len as usize
    }

    /// Whether the island holds no manifold.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The island's handles: its stream indices in ascending order, then its kept handles.
    #[inline]
    pub fn iter(&self) -> IslandIter<'a> {
        let base = HELD_BASE + self.held_start;
        IslandIter { stream: self.stream.iter(), held: base..base + self.held_len }
    }
}

impl fmt::Debug for IslandManifolds<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<'a> IntoIterator for IslandManifolds<'a> {
    type Item = u32;
    type IntoIter = IslandIter<'a>;

    #[inline]
    fn into_iter(self) -> IslandIter<'a> {
        self.iter()
    }
}

impl<'a> IntoIterator for &IslandManifolds<'a> {
    type Item = u32;
    type IntoIter = IslandIter<'a>;

    #[inline]
    fn into_iter(self) -> IslandIter<'a> {
        self.iter()
    }
}

/// The iterator of an [`IslandManifolds`]: handles by value.
#[derive(Clone, Debug)]
pub struct IslandIter<'a> {
    /// The island's remaining stream indices.
    stream: core::slice::Iter<'a, u32>,
    /// The island's remaining kept handles.
    held: core::ops::Range<u32>,
}

impl Iterator for IslandIter<'_> {
    type Item = u32;

    #[inline]
    fn next(&mut self) -> Option<u32> {
        self.stream.next().copied().or_else(|| self.held.next())
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.stream.len() + self.held.len();
        (n, Some(n))
    }
}

impl DoubleEndedIterator for IslandIter<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<u32> {
        self.held.next_back().or_else(|| self.stream.next_back().copied())
    }
}

impl ExactSizeIterator for IslandIter<'_> {}

impl FusedIterator for IslandIter<'_> {}

#[cfg(test)]
mod tests {
    //! The logical pair view against a merged `Vec` (L10 C3c, design 04 T3): `len`, `iter`,
    //! `next_back`, a walk that mixes both ends, `get` and `contains`, over a stream and a
    //! withheld list that interleave.

    use proptest::prelude::*;

    use super::PairsView;
    use crate::manifold::BodyIndex;

    type Pair = (BodyIndex, BodyIndex);

    /// The largest body count drawn: `12 · 11 / 2 = 66` pairs, one pick each.
    const MAX_BODIES: u32 = 12;
    /// One pick per pair of [`MAX_BODIES`] bodies.
    const MAX_PAIRS: usize = 66;

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 256,
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        /// Every pair `(a, b)`, `a < b < n`, is absent, in the stream or withheld (`pick` 0, 1,
        /// 2); the view must be the sorted union, from either end and by position.
        #[test]
        fn the_pair_view_is_the_merge_of_the_stream_and_the_withheld_pairs(
            n in 2u32..=MAX_BODIES,
            picks in prop::collection::vec(0u8..3, MAX_PAIRS),
            ends in prop::collection::vec(any::<bool>(), MAX_PAIRS + 1),
        ) {
            let all = (0..n).flat_map(|a| (a + 1..n).map(move |b| (BodyIndex(a), BodyIndex(b))));
            let (mut stream, mut withheld, mut want, mut absent) =
                (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for (pair, &pick) in all.zip(&picks) {
                match pick {
                    1 => stream.push(pair),
                    2 => withheld.push(pair),
                    _ => absent.push(pair),
                }
                if pick != 0 {
                    want.push(pair);
                }
            }
            let view = PairsView::new(&stream, &withheld);
            prop_assert_eq!(view.len(), want.len());
            prop_assert_eq!(view.is_empty(), want.is_empty());
            prop_assert_eq!(view.iter().copied().collect::<Vec<Pair>>(), want.clone());
            let mut back: Vec<Pair> = view.iter().rev().copied().collect();
            back.reverse();
            prop_assert_eq!(back, want.clone());

            // Both ends at once: the two cursors of each list must meet, never cross.
            let mut it = view.iter();
            let (mut lo, mut hi) = (0usize, want.len());
            for &front in &ends {
                prop_assert_eq!(it.len(), hi - lo);
                let got = if front { it.next() } else { it.next_back() }.copied();
                if lo == hi {
                    prop_assert_eq!(got, None);
                    break;
                }
                let expected = if front {
                    lo += 1;
                    want[lo - 1]
                } else {
                    hi -= 1;
                    want[hi]
                };
                prop_assert_eq!(got, Some(expected));
            }

            for (k, pair) in want.iter().enumerate() {
                prop_assert_eq!(view.get(k), Some(pair), "get({})", k);
            }
            prop_assert_eq!(view.get(want.len()), None);
            prop_assert!(want.iter().all(|p| view.contains(p)));
            prop_assert!(!absent.iter().any(|p| view.contains(p)));
            prop_assert_eq!(view.to_vec(), want);
        }
    }
}
