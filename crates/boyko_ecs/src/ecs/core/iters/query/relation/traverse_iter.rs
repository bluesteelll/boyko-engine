//! Transitive / wildcard relation traversal iterators (the QUERY-side
//! wildcard family).
//!
//! All four iterators walk ONLY the existing relationship storage — the FK
//! [`Relationship`](crate::ecs::core::relationship::Relationship) component on
//! the source and the reverse
//! [`RelationshipTarget`](crate::ecs::core::relationship::RelationshipTarget)
//! collection on the target — through the address-stable `get_component`
//! primitive (the same per-hop column lookup
//! [`Toward<R>::next`](crate::ecs::core::component::observers::traversal::Toward)
//! already pays). NO unsafe, NO side index, NO `HashMap` (Principle 0).
//!
//! # Depth + cycle discipline
//!
//! The transitive walks ([`AncestorsIter`] / [`DescendantsIter`]) are bounded
//! by [`MAX_PROPAGATION_DEPTH`] (the existing 1024 cap). When the relation is
//! NOT [`Relationship::ACYCLIC`], a `#[cold]` function-local [`VisitedSet`]
//! (transient scratch — NOT a durable side store) guards revisits so a cyclic
//! graph terminates. When the relation IS acyclic (e.g. `ChildOf`), the
//! visited set const-folds away (`if const { !R::ACYCLIC }`) and the depth cap
//! alone bounds the walk.
//!
//! These iterators are reusable by the later relation-aware observer-broadcast
//! phase (the same wildcard walks back the broadcast frontier needs).

use crate::ecs::constants::MAX_PROPAGATION_DEPTH;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::entity::entity::Entity;
use crate::ecs::core::relationship::{Relationship, RelationshipSourceCollection, RelationshipTarget};

// ── VisitedSet — function-local cold-path revisit guard ─────────────────────

/// Transient, growable, [`Entity`]-id-keyed visited bitset — function-local
/// scratch for the `!ACYCLIC` traversal cold path ONLY.
///
/// This is NOT a durable side store (Principle 0): it is allocated lazily on
/// the cold path of a single `AncestorsIter` / `DescendantsIter` walk and
/// dropped with the iterator. An acyclic relation const-folds it away entirely
/// (`if const { !R::ACYCLIC }`), so the common `ChildOf` walk pays nothing.
///
/// `boyko_utils::BitSet<T>` is a fixed-width integer bitset (≤ `T::BITS` keys),
/// unsuitable for an arbitrary [`Entity`] id space; this growable word-vec
/// bitset is the cold-path equivalent keyed on `EntityId.0`.
#[derive(Default)]
struct VisitedSet {
    /// One bit per entity id; word `i` covers ids `[64*i, 64*i + 64)`.
    words: Vec<u64>,
}

impl VisitedSet {
    /// Marks `id` visited; returns `true` iff it was ALREADY present (a revisit
    /// — the caller stops descending that branch).
    #[cold]
    #[inline(never)]
    fn insert_seen(&mut self, id: usize) -> bool {
        let word = id >> 6;
        let bit = id & 63;
        if word >= self.words.len() {
            self.words.resize(word + 1, 0);
        }
        let mask = 1u64 << bit;
        let already = (self.words[word] & mask) != 0;
        self.words[word] |= mask;
        already
    }
}

// ── TargetsIter — R's single target for a source (0..1) ─────────────────────

/// Yields `R`'s single target for `source` (cardinality 0..1).
///
/// `R` is a single-target foreign key, so this yields at most one entity: the
/// FK target if `source` carries `R`, otherwise nothing. Built on
/// `get_component::<R>(source).map(|r| r.target())` — the same hop
/// [`Toward<R>`](crate::ecs::core::component::observers::traversal::Toward)
/// pays.
pub struct TargetsIter<'w, R: Relationship> {
    world: &'w EcsMaster,
    source: Entity,
    /// `true` once the single target has been yielded (or proven absent).
    done: bool,
    _marker: core::marker::PhantomData<fn() -> R>,
}

impl<'w, R: Relationship> TargetsIter<'w, R> {
    /// Builds the iterator over `source`'s single `R` target.
    #[inline]
    pub fn new(world: &'w EcsMaster, source: Entity) -> Self {
        Self {
            world,
            source,
            done: false,
            _marker: core::marker::PhantomData,
        }
    }
}

impl<R: Relationship> Iterator for TargetsIter<'_, R> {
    type Item = Entity;

    #[inline]
    fn next(&mut self) -> Option<Entity> {
        if self.done {
            return None;
        }
        self.done = true;
        self.world.get_component::<R>(self.source).map(|r| r.target())
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(1))
    }
}

// ── SourcesIter — all sources of R for a target ─────────────────────────────

/// Yields every source whose `R` foreign key points at `target`, by reading
/// the target's reverse [`RelationshipTarget`] collection (`R::Target`).
///
/// Reuses the existing reverse index (the
/// [`RelationshipSourceCollection`]); NO scan over all entities. The reverse
/// collection is resolved ONCE at construction (O2 fix) and held by reference
/// for the iterator's lifetime, then read by O(1) index per step
/// (`collection().get(i)` — O(1) for the v1 `Vec` backing). This is sound: the
/// iterator borrows `&'w EcsMaster`, so the `&'w R::Target` it holds cannot be
/// invalidated for its lifetime (a read-only traversal never takes a `&mut`
/// into storage). It mirrors `DescendantsIter::push_sources` (one lookup per
/// node, then a tight index loop) instead of paying a full random-access
/// `get_component` lookup per yielded source.
pub struct SourcesIter<'w, R: Relationship> {
    /// The reverse collection resolved ONCE at construction. `None` when the
    /// target carries no `R::Target` (no source ever pointed here) — the
    /// iterator is then empty.
    rev: Option<&'w <R::Target as RelationshipTarget>::Collection>,
    /// Next index into the reverse collection.
    index: usize,
    /// Length of the reverse collection (read once with `rev`; immutable for
    /// the iterator's life — `&EcsMaster`).
    len: usize,
    _marker: core::marker::PhantomData<fn() -> R>,
}

impl<'w, R: Relationship> SourcesIter<'w, R> {
    /// Builds the iterator over every source of `R` pointing at `target`.
    #[inline]
    pub fn new(world: &'w EcsMaster, target: Entity) -> Self {
        // O2 fix: resolve the reverse collection ONCE here (one `get_component`
        // lookup) and hold it by reference. The `&'w R::Target` lifetime is
        // tied to `&'w EcsMaster`, so the borrow is valid for the whole walk and
        // each `next` is a plain O(1) slice index. An absent `R::Target` yields
        // an empty iterator.
        let rev = world.get_component::<R::Target>(target).map(|r| r.collection());
        let len = rev.map(RelationshipSourceCollection::len).unwrap_or(0);
        Self {
            rev,
            index: 0,
            len,
            _marker: core::marker::PhantomData,
        }
    }
}

impl<R: Relationship> Iterator for SourcesIter<'_, R> {
    type Item = Entity;

    #[inline]
    fn next(&mut self) -> Option<Entity> {
        if self.index >= self.len {
            return None;
        }
        let i = self.index;
        self.index += 1;
        // O(1) index into the once-resolved collection (no per-step lookup).
        // `rev` is `Some` whenever `len > 0`, so the index is in range.
        self.rev.and_then(|c| c.get(i))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.len.saturating_sub(self.index);
        (0, Some(remaining))
    }
}

// ── AncestorsIter — walk e -> R.target -> ... up the chain ──────────────────

/// Walks UP the `R` chain: `e -> R.target -> (R.target).target -> ...`, yielding
/// each target in turn (NOT `e` itself).
///
/// Reuses the [`Toward<R>::next`](crate::ecs::core::component::observers::traversal::Toward)
/// hop shape. Bounded by [`MAX_PROPAGATION_DEPTH`]. For a non-`ACYCLIC`
/// relation a `#[cold]` [`VisitedSet`] terminates a cycle; for an `ACYCLIC`
/// relation (e.g. `ChildOf`) the visited set const-folds away.
pub struct AncestorsIter<'w, R: Relationship> {
    world: &'w EcsMaster,
    /// The node whose `R` target is yielded next; `None` once the walk stops.
    current: Option<Entity>,
    /// Hops taken so far (the depth-cap counter).
    depth: usize,
    /// Cold-path revisit guard (allocated only for `!R::ACYCLIC`).
    visited: VisitedSet,
    _marker: core::marker::PhantomData<fn() -> R>,
}

impl<'w, R: Relationship> AncestorsIter<'w, R> {
    /// Builds the upward walk from `e` (the first yield is `e`'s `R` target).
    #[inline]
    pub fn new(world: &'w EcsMaster, e: Entity) -> Self {
        let mut visited = VisitedSet::default();
        // Seed the start node into the visited set so a self-link (`R(self)`)
        // terminates immediately on the `!ACYCLIC` path.
        if const { !R::ACYCLIC } {
            visited.insert_seen(e.id().0);
        }
        Self {
            world,
            current: Some(e),
            depth: 0,
            visited,
            _marker: core::marker::PhantomData,
        }
    }
}

impl<R: Relationship> Iterator for AncestorsIter<'_, R> {
    type Item = Entity;

    #[inline]
    fn next(&mut self) -> Option<Entity> {
        let node = self.current?;
        // Depth cap: `debug_assert!` in debug, hard stop in release.
        debug_assert!(
            self.depth <= MAX_PROPAGATION_DEPTH,
            "AncestorsIter exceeded MAX_PROPAGATION_DEPTH"
        );
        if self.depth >= MAX_PROPAGATION_DEPTH {
            self.current = None;
            return None;
        }
        // One hop up: `node -> R.target`.
        let Some(parent) = self.world.get_component::<R>(node).map(|r| r.target()) else {
            self.current = None;
            return None;
        };
        self.depth += 1;
        // `!ACYCLIC`: stop if the parent was already seen (cycle guard).
        if const { !R::ACYCLIC } && self.visited.insert_seen(parent.id().0) {
            self.current = None;
            return None;
        }
        self.current = Some(parent);
        Some(parent)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(MAX_PROPAGATION_DEPTH.saturating_sub(self.depth)))
    }
}

// ── DescendantsIter — explicit-stack DFS over reverse collections ───────────

/// Walks DOWN the `R` chain from `root` via explicit-stack DFS over the reverse
/// [`RelationshipTarget`] collections, yielding each descendant source (NOT
/// `root` itself).
///
/// Each pop reads the node's `R::Target` reverse collection and pushes its
/// sources. Bounded by [`MAX_PROPAGATION_DEPTH`] (per-node depth tag); for a
/// non-`ACYCLIC` relation a `#[cold]` [`VisitedSet`] keeps each node visited at
/// most once (so the total work is ≤ C·N under the guard), const-folded away
/// for an `ACYCLIC` relation.
///
/// The DFS stack is function-local transient scratch (NOT a durable side
/// store) — it lives with the iterator and holds `(node, depth)` frontier
/// entries.
pub struct DescendantsIter<'w, R: Relationship> {
    world: &'w EcsMaster,
    /// DFS frontier of `(node, depth)` entries still to expand.
    stack: Vec<(Entity, usize)>,
    /// Cold-path revisit guard (allocated only for `!R::ACYCLIC`).
    visited: VisitedSet,
    _marker: core::marker::PhantomData<fn() -> R>,
}

impl<'w, R: Relationship> DescendantsIter<'w, R> {
    /// Builds the downward DFS from `root` (the first yields are `root`'s direct
    /// sources).
    #[inline]
    pub fn new(world: &'w EcsMaster, root: Entity) -> Self {
        let mut visited = VisitedSet::default();
        if const { !R::ACYCLIC } {
            visited.insert_seen(root.id().0);
        }
        // Seed the frontier with `root`'s direct sources (depth 1).
        let mut stack = Vec::new();
        Self::push_sources(world, root, 1, &mut stack);
        Self {
            world,
            stack,
            visited,
            _marker: core::marker::PhantomData,
        }
    }

    /// Pushes every source of `R` pointing at `node` onto `stack` at `depth`.
    /// Reuses the reverse collection by O(1) index (`Vec` backing), so this is
    /// O(children).
    #[inline]
    fn push_sources(world: &EcsMaster, node: Entity, depth: usize, stack: &mut Vec<(Entity, usize)>) {
        let Some(rev) = world.get_component::<R::Target>(node) else {
            return;
        };
        let collection = rev.collection();
        let len = collection.len();
        for i in 0..len {
            if let Some(source) = collection.get(i) {
                stack.push((source, depth));
            }
        }
    }
}

impl<R: Relationship> Iterator for DescendantsIter<'_, R> {
    type Item = Entity;

    #[inline]
    fn next(&mut self) -> Option<Entity> {
        while let Some((node, depth)) = self.stack.pop() {
            debug_assert!(
                depth <= MAX_PROPAGATION_DEPTH,
                "DescendantsIter exceeded MAX_PROPAGATION_DEPTH"
            );
            // `!ACYCLIC`: skip a node already visited (the ≤ C·N guarantee).
            if const { !R::ACYCLIC } && self.visited.insert_seen(node.id().0) {
                continue;
            }
            // Depth cap: do not expand below the cap, but still YIELD the node
            // (it is a valid descendant at `depth <= cap`).
            if depth < MAX_PROPAGATION_DEPTH {
                Self::push_sources(self.world, node, depth + 1, &mut self.stack);
            }
            return Some(node);
        }
        None
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        // At least the frontier already discovered; no cheap upper bound.
        (0, None)
    }
}
