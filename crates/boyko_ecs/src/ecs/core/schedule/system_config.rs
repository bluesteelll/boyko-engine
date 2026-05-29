//! [`SystemConfig`] — fluent chaining handle returned by
//! [`ScheduleBuilder::add_system`].
//!
//! See Phase 9 plan §5.6. Wave 4 Step 9 ships the minimum surface the
//! topological-sort + cycle-detection tests in `schedule_builder` exercise:
//! `.before(other)`, `.after(other)`, `.chain(other)` register ordering
//! edges against pre-existing systems by their [`SystemKey`]; `.in_set`
//! records set membership for the (Wave 5 Step 14) expansion pass.
//!
//! # Why `SystemKey` rather than `IntoSystem<...>`
//!
//! Plan §5.6 sketches `pub fn before<F, M>(self, other: F)`. That
//! generic-over-`IntoSystem` form requires either (a) eagerly adding the
//! referenced system on every chained call (Bevy-style implicit
//! registration), which complicates ordering semantics, or (b) keying by
//! `TypeId` (which fails on closures that have anonymous types). Wave 4
//! Step 9 takes the simpler tactic: store handles returned by
//! `add_system` and reference them by value. Wave 7 can re-introduce
//! the generic form via a `SystemConfigExt` trait once the macro layer
//! (`#[derive(SystemSet)]`) lands.
//!
//! [`ScheduleBuilder::add_system`]: super::schedule_builder::ScheduleBuilder::add_system

use crate::ecs::core::schedule::ordering::{OrderingEdge, SetOrderEdge, SystemKey};
use crate::ecs::core::schedule::schedule_builder::ScheduleBuilder;
use crate::ecs::core::schedule::system_set::SystemSet;

/// Fluent chaining handle returned by
/// [`ScheduleBuilder::add_system`].
///
/// Each chained call appends an [`OrderingEdge`] (or a set-membership
/// hint) to the descriptor referenced by `key` inside `builder`. The
/// handle is single-use per `add_system` invocation — `self` is consumed
/// and returned so the user pattern stays
/// `builder.add_system(foo).before(bar).after(baz)`.
///
/// [`ScheduleBuilder::add_system`]: super::schedule_builder::ScheduleBuilder::add_system
pub struct SystemConfig<'a> {
    /// Borrow of the owning builder; the lifetime ties chaining to a
    /// single `add_system` call.
    pub(crate) builder: &'a mut ScheduleBuilder,

    /// Pre-build handle for the system this chain references.
    pub(crate) key: SystemKey,
}

impl<'a> SystemConfig<'a> {
    /// Returns the [`SystemKey`] of the system this chain operates on.
    /// Used by tests + by user code that needs to forward the handle
    /// into a sibling `.before(...)` / `.after(...)` call.
    #[inline]
    pub fn key(&self) -> SystemKey {
        self.key
    }

    /// Records "this system must run before `other`". Encoded as an
    /// [`OrderingEdge::Before`] on the receiver-side descriptor.
    ///
    /// Equivalent to `other_config.after(self.key())` — the edge ends up
    /// on whichever descriptor carries it; `build` deduplicates by
    /// flattening both directions into a single edge list.
    #[inline]
    pub fn before(self, other: SystemKey) -> Self {
        let edge = OrderingEdge::Before(self.key, other);
        self.builder.descriptors[self.key.0]
            .ordering_hints
            .push(edge);
        self
    }

    /// Records "this system must run after `other`". Encoded as
    /// [`OrderingEdge::After`].
    #[inline]
    pub fn after(self, other: SystemKey) -> Self {
        let edge = OrderingEdge::After(self.key, other);
        self.builder.descriptors[self.key.0]
            .ordering_hints
            .push(edge);
        self
    }

    /// Records a strict serial order between this system and `other`
    /// (this → other). Same DAG edge as `before` but a distinct
    /// [`OrderingEdge`] variant for diagnostics.
    #[inline]
    pub fn chain(self, other: SystemKey) -> Self {
        let edge = OrderingEdge::ChainConsecutive(self.key, other);
        self.builder.descriptors[self.key.0]
            .ordering_hints
            .push(edge);
        self
    }

    /// Records membership in `set`. The set is interned (or looked up) by
    /// its `(TypeId, discriminant)` key, so subsequent `.in_set(SameSet)`
    /// calls — and `configure_set(SameSet)` / `before_set(SameSet)` —
    /// resolve to the **same** [`SystemSetId`] (Phase 15 §13-P1).
    ///
    /// Membership alone contributes no ordering edge; the set's expanded
    /// edges arise from set-level ordering (`before_set` / `configure_set`)
    /// over the transitive members computed at `build` (D1/D3).
    #[inline]
    pub fn in_set<S: SystemSet>(self, set: S) -> Self {
        let set_id = self.builder.set_id_of_value(set);
        self.builder.descriptors[self.key.0].sets.push(set_id);
        self.builder.descriptors[self.key.0]
            .ordering_hints
            .push(OrderingEdge::InSet(self.key, set_id));
        // The set bookkeeping is mirrored on the builder so the build-time
        // set-expansion pass doesn't have to walk every descriptor — keep
        // both in sync.
        self.builder.set_members.entry(set_id).or_default().push(self.key);
        self
    }

    /// Records "this system runs before every (current + nested) member of
    /// `set`". Recorded as a builder-level [`SetOrderEdge`]; expanded into
    /// `X → member` edges at `build` over the transitive membership.
    #[inline]
    pub fn before_set<S: SystemSet>(self, set: S) -> Self {
        let set_id = self.builder.set_id_of_value(set);
        self.builder
            .set_ordering
            .push(SetOrderEdge::SystemBeforeSet(self.key, set_id));
        self
    }

    /// Records "this system runs after every member of `set`". Recorded as
    /// a builder-level [`SetOrderEdge`]; expanded into `member → X` edges at
    /// `build`.
    #[inline]
    pub fn after_set<S: SystemSet>(self, set: S) -> Self {
        let set_id = self.builder.set_id_of_value(set);
        self.builder
            .set_ordering
            .push(SetOrderEdge::SystemAfterSet(self.key, set_id));
        self
    }
}
