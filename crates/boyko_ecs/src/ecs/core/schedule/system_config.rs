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
use crate::ecs::core::schedule::system_box::BoolSystem;
use crate::ecs::core::schedule::system_set::SystemSet;
use crate::ecs::core::system::into_system::IntoSystem;
use crate::ecs::core::system::system::System;

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

    /// Attaches a **run condition** to this system (Phase 16). The system's
    /// body runs in a frame only if every attached condition returns `true`.
    ///
    /// A condition is any `impl IntoSystem<(), bool, M>` — e.g. a
    /// `fn() -> bool`, `fn(Res<R>) -> bool`, or the built-in
    /// [`run_once`](crate::ecs::core::schedule::run_once).
    ///
    /// # Eager AND (no short-circuit)
    ///
    /// Multiple `.run_if(a).run_if(b)` accumulate and fold to a logical AND.
    /// EVERY condition is evaluated every frame (the fold never
    /// short-circuits), so a stateful condition like `run_once` advances its
    /// own `Local` even when an earlier condition already returned `false`.
    /// See `PHASE-16-PLAN.md` §6.
    ///
    /// # Read-only requirement
    ///
    /// A condition MUST be read-only — it must declare no component / resource
    /// writes. This is `debug_assert!`ed at build (`PHASE-16-PLAN.md` §8.5).
    /// Conditions are evaluated single-threaded at the apply-window barrier,
    /// so a write-declaring condition is sound (it holds the exclusive `&mut`)
    /// but is an API misuse; do not use `Commands` / `EventWriter` in a
    /// condition (its deferred work is dropped, never applied).
    ///
    /// # Change-detection conditions (Phase 16.1, B-1 — now supported)
    ///
    /// A condition using change detection (`Changed<T>` / `Added<T>` /
    /// `Ref<T>`) works correctly: [`Schedule::run`] bumps every condition's
    /// `(last_run, this_run)` tick snapshot at frame start with the same
    /// `this_run` as the systems (mirroring the per-system dispatch), so the
    /// condition observes the proper `(last_run, this_run]` window and fires
    /// only when the data actually changed.
    ///
    /// On the FIRST frame a condition observes every pre-existing tick as
    /// "changed since last run" (the standard late-added-system semantic — the
    /// initial `last_run` is `current - MAX_CHANGE_AGE`), identical to how a
    /// system's `Changed<T>` query behaves on its first run.
    ///
    /// [`Schedule::run`]: crate::ecs::core::schedule::schedule::Schedule::run
    #[inline]
    pub fn run_if<C, M>(self, condition: C) -> Self
    where
        C: IntoSystem<(), bool, M>,
        C::System: System<Out = bool> + 'static,
    {
        let sys = C::into_system(condition);
        let boxed: BoolSystem = Box::new(sys);
        self.builder.descriptors[self.key.0].conditions.push(boxed);
        self
    }

    /// Marks this system as a **GPU-compute** system (Phase 5 MF-1).
    ///
    /// Sets the descriptor's `is_gpu` flag, which the build-time `SystemKind`
    /// resolution ([`ScheduleBuilder::build`]) reads to classify the system as
    /// [`SystemKind::GpuCompute`](crate::ecs::core::system::system_kind::SystemKind::GpuCompute)
    /// — a marker carve-out that is NOT derived from access. A `GpuCompute`
    /// system runs dispatcher-solo at the apply-window barrier (`running == 0`),
    /// the sound site for recording/submitting through the `!Send` RHI.
    ///
    /// # Why an explicit marker (not derived)
    ///
    /// `is_gpu` cannot be inferred from access or `requires_dispatcher`: the
    /// latter is also raised by every `NonSendResMut` CPU system, so deriving
    /// from it would mis-mark them `GpuCompute`. The marker is the single source
    /// of truth (MF-1, rejected-alternative rationale).
    ///
    /// # 0%-gate
    ///
    /// Kind resolution is build-time/cold. A schedule that never calls `.gpu()`
    /// leaves every descriptor at the default `is_gpu = false`, so the resolution
    /// is byte-identical to the previous `is_exclusive` derivation.
    ///
    /// [`ScheduleBuilder::build`]: super::schedule_builder::ScheduleBuilder::build
    #[inline]
    pub fn gpu(self) -> Self {
        self.builder.descriptors[self.key.0].is_gpu = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::ecs::core::schedule::schedule_builder::ScheduleBuilder;
    use boyko_threadpool::ThreadPoolBuilder;
    use std::sync::Arc;

    fn serial_builder() -> ScheduleBuilder {
        let pool = ThreadPoolBuilder::new().num_threads(1).build();
        ScheduleBuilder::new(pool)
    }

    /// `.run_if(cond)` pushes exactly one `BoolSystem` onto the referenced
    /// descriptor's `conditions` vec. (Plan §10 `run_if_stores_condition`.)
    #[test]
    fn run_if_pushes_one_condition() {
        let mut builder = serial_builder();
        let key = builder.add_system(|| {}).run_if(|| true).key();
        assert_eq!(
            builder.descriptors[key.0].conditions.len(),
            1,
            "a single .run_if stores one condition"
        );
    }

    /// `.run_if(a).run_if(b)` accumulates BOTH conditions on the descriptor
    /// (they fold to an AND at eval; storage is additive).
    #[test]
    fn chained_run_if_accumulates_conditions() {
        let mut builder = serial_builder();
        let key = builder
            .add_system(|| {})
            .run_if(|| true)
            .run_if(|| false)
            .key();
        assert_eq!(
            builder.descriptors[key.0].conditions.len(),
            2,
            "two chained .run_if calls store two conditions"
        );
    }

    /// An `add_system` with no `.run_if` leaves `conditions` empty — the
    /// 0%-gate precondition at the descriptor level.
    #[test]
    fn no_run_if_leaves_conditions_empty() {
        let mut builder = serial_builder();
        let key = builder.add_system(|| {}).key();
        let _ = Arc::clone(&builder.pool); // touch pool to keep it alive in scope
        assert!(
            builder.descriptors[key.0].conditions.is_empty(),
            "a system with no .run_if has zero conditions"
        );
    }
}
