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
//! Nothing is kept before L10 C3b (C2a lands the types over an EMPTY store), so today every view
//! is its stream element for element, and every reader moved onto the views reads exactly what
//! it read from the slices before.
//!
//! A manifold view yields [`Manifold`] by value and implements no `Index`: a held manifold is
//! rebuilt from its island's row table, and a reader that mixed a view position with a solver
//! index would otherwise read the wrong manifold silently. The solver's own index space is
//! [`Manifolds::solver_manifolds`](crate::resources::Manifolds::solver_manifolds).

use core::fmt;
use core::iter::FusedIterator;

use crate::manifold::{BodyIndex, Manifold};

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
/// ([`Manifolds::manifolds`](crate::resources::Manifolds::manifolds)). Yields [`Manifold`] by
/// value; no `Index`.
#[derive(Clone, Copy)]
pub struct ManifoldsView<'a> {
    /// The narrowphase's and the SDF stage's stream.
    stream: &'a [Manifold],
}

impl<'a> ManifoldsView<'a> {
    /// The view over `stream`, with nothing kept.
    #[inline]
    pub(crate) fn new(stream: &'a [Manifold]) -> Self {
        Self { stream }
    }

    /// The number of logical manifolds. O(1).
    #[inline]
    pub fn len(&self) -> usize {
        self.stream.len()
    }

    /// Whether the step has no solver manifold.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.stream.is_empty()
    }

    /// The manifolds, by value, in the order the sleep-skip off would emit them.
    #[inline]
    pub fn iter(&self) -> ManifoldsIter<'a> {
        ManifoldsIter { stream: self.stream.iter() }
    }

    /// The manifold at logical position `pos`, or `None` past the end.
    #[inline]
    pub fn get(&self, pos: usize) -> Option<Manifold> {
        self.stream.get(pos).copied()
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

/// The iterator of a [`ManifoldsView`]: manifolds by value, in logical order.
#[derive(Clone, Debug)]
pub struct ManifoldsIter<'a> {
    /// The stream's remaining manifolds.
    stream: core::slice::Iter<'a, Manifold>,
}

impl Iterator for ManifoldsIter<'_> {
    type Item = Manifold;

    #[inline]
    fn next(&mut self) -> Option<Manifold> {
        self.stream.next().copied()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.stream.size_hint()
    }
}

impl DoubleEndedIterator for ManifoldsIter<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<Manifold> {
        self.stream.next_back().copied()
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
}

impl<'a> IslandManifolds<'a> {
    /// The island over the stream indices `stream`, with nothing kept.
    #[inline]
    pub(crate) fn new(stream: &'a [u32]) -> Self {
        Self { stream }
    }

    /// The number of manifolds of the island. O(1).
    #[inline]
    pub fn len(&self) -> usize {
        self.stream.len()
    }

    /// Whether the island holds no manifold.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.stream.is_empty()
    }

    /// The island's handles: its stream indices in ascending order, then its kept handles.
    #[inline]
    pub fn iter(&self) -> IslandIter<'a> {
        IslandIter { stream: self.stream.iter() }
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
}

impl Iterator for IslandIter<'_> {
    type Item = u32;

    #[inline]
    fn next(&mut self) -> Option<u32> {
        self.stream.next().copied()
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.stream.size_hint()
    }
}

impl DoubleEndedIterator for IslandIter<'_> {
    #[inline]
    fn next_back(&mut self) -> Option<u32> {
        self.stream.next_back().copied()
    }
}

impl ExactSizeIterator for IslandIter<'_> {}

impl FusedIterator for IslandIter<'_> {}
