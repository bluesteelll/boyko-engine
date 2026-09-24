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
//! L10 C3b keeps held islands' manifolds (`held_store.rs`); the pair view stays the stream until
//! L10 C3c withholds pairs from it. With nothing held, every view is its stream element for
//! element.
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
/// ([`ContactPairs::pairs`](crate::resources::ContactPairs::pairs)).
#[derive(Clone, Copy)]
pub struct PairsView<'a> {
    /// The broadphase's stream.
    stream: &'a [(BodyIndex, BodyIndex)],
}

impl<'a> PairsView<'a> {
    /// The view over `stream`, with nothing withheld.
    #[inline]
    pub(crate) fn new(stream: &'a [(BodyIndex, BodyIndex)]) -> Self {
        Self { stream }
    }

    /// The number of logical pairs. O(1).
    #[inline]
    pub fn len(&self) -> usize {
        self.stream.len()
    }

    /// Whether the step has no candidate pair.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.stream.is_empty()
    }

    /// The pairs in `(min, max)` order.
    #[inline]
    pub fn iter(&self) -> PairsIter<'a> {
        PairsIter { stream: self.stream.iter() }
    }

    /// The `k`-th pair in `(min, max)` order, or `None` past the end.
    #[inline]
    pub fn get(&self, k: usize) -> Option<&'a (BodyIndex, BodyIndex)> {
        self.stream.get(k)
    }

    /// Whether `pair` is a candidate pair of the step. O(log n): the view is strictly
    /// increasing (the broadphase's contract).
    #[inline]
    pub fn contains(&self, pair: &(BodyIndex, BodyIndex)) -> bool {
        self.stream.binary_search(pair).is_ok()
    }

    /// The pairs, copied into a `Vec` in `(min, max)` order: the in-crate tests' helper. The
    /// public API has no allocating method (UG-02: no new heap site in library code); a caller
    /// collects [`iter`](Self::iter).
    #[cfg(test)]
    pub(crate) fn to_vec(self) -> Vec<(BodyIndex, BodyIndex)> {
        self.stream.to_vec()
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

/// The iterator of a [`PairsView`], in `(min, max)` order.
#[derive(Clone, Debug)]
pub struct PairsIter<'a> {
    /// The stream's remaining pairs.
    stream: core::slice::Iter<'a, (BodyIndex, BodyIndex)>,
}

impl<'a> Iterator for PairsIter<'a> {
    type Item = &'a (BodyIndex, BodyIndex);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.stream.next()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.stream.size_hint()
    }
}

impl DoubleEndedIterator for PairsIter<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        self.stream.next_back()
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
