//! Run-condition combinators — [`CombinedSystem`] and [`Not`]
//! (kernel backlog **KE5**, ruling **D5**).
//!
//! `.run_if(a).run_if(b)` already folds to a logical AND across a system's own
//! condition list. What it cannot express is a *disjunction* (`a || b`) or a
//! *negation* (`!a`), because the fold site owns the operator. This module adds
//! the missing shapes as ordinary `System<Out = bool>` values, so a combined
//! condition is stored, initialised, ticked and evaluated by exactly the
//! machinery that already handles a leaf condition — `run_if` takes it through
//! the same `IntoSystem<(), bool, _>` door.
//!
//! # The fold is EAGER — ruling D5
//!
//! [`CombinedSystem`] runs **both** children on every frame it is reached, then
//! combines the two materialised `bool`s. It never short-circuits, and the
//! ground is not (only) that `run_once` mutates on evaluation:
//!
//! > A condition's change-tick window advances only on a frame it actually
//! > runs (`EcsMaster::run_condition` is the single write site, and it advances
//! > `(prev_this_run, this_run]` right before dispatching). A short-circuited
//! > RHS therefore **freezes** its window: it does not observe the frame it was
//! > skipped on, and when it is finally reached its window still opens back at
//! > its last real run — so it reports every change accumulated in between, all
//! > at once. That is a bogus `Changed` burst, and it is silent.
//!
//! Eager folding costs one extra condition body per frame — conditions are
//! evaluated single-threaded at the apply-window barrier, are read-only by
//! contract, and are typically a resource compare. Paying that is the price of
//! a `Changed`/`Added`/`run_once` child meaning the same thing inside a
//! combinator as outside one.
//!
//! Pinned by the five D5 tests in
//! `crates/boyko_ecs/tests/ke5_condition_combinators.rs`, cited there by
//! number.
//!
//! # Access is the union
//!
//! Both children run, so the composite touches everything either child
//! touches. [`CombinedSystem::initialize`] unions the children's declared
//! [`Access`] into its own via [`Access::extend`].
//!
//! **What reads that union is the Phase-16 CR1 read-only contract, not the
//! conflict graph.** `debug_assert_condition_read_only` (`schedule_builder.rs`)
//! is the only consumer of a condition's `Access` in the tree: a composite that
//! declared only the LHS's surface would let an RHS declaring `ResMut<T>` pass
//! that check unseen. `ConflictGraph::build` reads the owning SYSTEM's access,
//! never its conditions', and `EcsMaster::run_condition` evaluates conditions
//! single-threaded at the apply-window barrier under `&mut self` — so there is
//! no concurrency here for the union to guard, and this paragraph used to say
//! there was. `D5(4)` still fails on a narrowed union; only the stake was wrong.
//!
//! # Tick maintenance is forwarded to BOTH children
//!
//! [`System::set_change_ticks`] and [`System::check_change_tick`] are the two
//! tick-maintenance channels, and the composite forwards both to each child.
//! `set_change_ticks` forwards using **each child's own** previous `this_run`
//! as its new `last_run` — the identical derivation `run_condition` performs
//! for a leaf condition — so a child's window means the same thing whether it
//! is nested or not. This is the mechanism the eager fold's correctness rests
//! on; D5(5) pins it directly.
//!
//! # Zero when unused
//!
//! Everything here is generic over the child system types, so a program that
//! never names a combinator monomorphises none of it and emits no code. No
//! existing type gains a field, no existing function gains a branch, and
//! `Schedule` / `SystemConfig` / the condition-eval pass are untouched: a
//! combined condition arrives at `run_if` as just another
//! `IntoSystem<(), bool, _>`.
//!
//! [`Access`]: crate::ecs::core::system::access::Access
//! [`Access::extend`]: crate::ecs::core::system::access::Access::extend
//! [`System::set_change_ticks`]: crate::ecs::core::system::system::System::set_change_ticks
//! [`System::check_change_tick`]: crate::ecs::core::system::system::System::check_change_tick

use std::marker::PhantomData;

use crate::ecs::core::change_detection::{MAX_CHANGE_AGE, Tick};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::access::Access;
use crate::ecs::core::system::into_system::IntoSystem;
use crate::ecs::core::system::system::System;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// The boolean operator a [`CombinedSystem`] folds its two children with.
///
/// A zero-sized type parameter rather than a stored function pointer: the
/// operator is chosen at the type level, so [`combine`](Self::combine) is a
/// direct call the optimiser folds into a single `and` / `or` instruction with
/// no indirect branch. Sealed by convention — the two implementors below are
/// the whole set the grammar (`when (A or B)` / `unless C`) needs.
pub trait CombineOp: Send + Sync + 'static {
    /// Folds the two children's already-materialised verdicts.
    ///
    /// Both arguments are values, never closures: the signature is what makes
    /// short-circuiting unrepresentable at this seam.
    fn combine(lhs: bool, rhs: bool) -> bool;
}

/// Logical AND (`lhs && rhs`), folded eagerly.
///
/// Equivalent in verdict to `.run_if(a).run_if(b)`, which the condition-eval
/// pass already folds with `&=`. It exists as a combinator so an AND can be
/// nested inside an OR (`(a && b) || c`) — the chained form cannot be.
pub struct AndOp;

impl CombineOp for AndOp {
    #[inline]
    fn combine(lhs: bool, rhs: bool) -> bool {
        // `&`, not `&&`: both operands are already values here, but the
        // bitwise form states at the source level that no control flow is
        // involved. The short-circuit that ruling D5 forbids is prevented one
        // layer up — in `run_unsafe`, which runs both children before calling
        // this — and this operator cannot reintroduce it.
        lhs & rhs
    }
}

/// Logical OR (`lhs || rhs`), folded eagerly.
///
/// The shape `run_if` could not express before: a system that runs when
/// **either** condition holds. The RHS still evaluates on a frame the LHS
/// already decided the verdict — see the module docs for why that is the point
/// and not an oversight.
pub struct OrOp;

impl CombineOp for OrOp {
    #[inline]
    fn combine(lhs: bool, rhs: bool) -> bool {
        // `|`, not `||` — see `AndOp::combine`.
        lhs | rhs
    }
}

/// Two run conditions folded by `OP`, evaluated **eagerly** (ruling D5).
///
/// Construct through [`ConditionExt::and`] / [`ConditionExt::or`] rather than
/// by naming the fields; the extension trait converts each side through
/// [`IntoSystem`] so a bare `fn` or closure works as a child.
///
/// ```ignore
/// use boyko_ecs::ecs::core::schedule::{ConditionExt, run_once};
///
/// builder
///     .add_system(spawn_wave)
///     .run_if(run_once.or(arena_is_empty));
/// ```
pub struct CombinedSystem<A, B, OP> {
    /// Left-hand child. Runs first, every reached frame.
    a: A,
    /// Right-hand child. Runs second, every reached frame — including frames
    /// on which the LHS alone already determined the verdict.
    b: B,
    /// The composite's own meta: name, the unioned access, and the tick
    /// snapshot the scheduler writes through [`System::set_change_ticks`].
    meta: SystemMeta,
    /// `true` once [`System::initialize`] has unioned the children's access.
    ///
    /// The scheduler re-`initialize`s conditions before every evaluation (the
    /// FS1 idempotence contract), and the union is 192 B of ORs; the flag keeps
    /// that off the per-frame path. Correctness does not depend on it — OR is
    /// idempotent — only cost does.
    access_unioned: bool,
    /// Binds `OP` without storing it (`OP` is a ZST marker).
    _op: PhantomData<fn() -> OP>,
}

impl<A, B, OP> CombinedSystem<A, B, OP>
where
    A: System<Out = bool>,
    B: System<Out = bool>,
    OP: CombineOp,
{
    /// Wraps two already-converted child systems.
    ///
    /// The meta is seeded with the `for_testing` sentinel tick, exactly as
    /// `FunctionSystem::new` does, because a constructor has no `world`;
    /// [`System::initialize`] re-seeds both ticks from the world's real tick
    /// on first init (Phase 10 Wave D Step 14, Option B).
    pub fn new(a: A, b: B) -> Self {
        Self {
            a,
            b,
            meta: SystemMeta::for_testing(std::any::type_name::<Self>()),
            access_unioned: false,
            _op: PhantomData,
        }
    }

    /// Read-only handle on the left child.
    ///
    /// Exists so the D5 pin tests can assert on a child's tick snapshot and
    /// declared access directly, rather than inferring them from the
    /// composite's observed verdict — D5(4)/D5(5) are explicitly written
    /// against the composed state, so a scheduler that happened to serialise
    /// could not hide a missing entry.
    #[inline]
    pub fn lhs(&self) -> &A {
        &self.a
    }

    /// Read-only handle on the right child. See [`lhs`](Self::lhs).
    #[inline]
    pub fn rhs(&self) -> &B {
        &self.b
    }
}

// SAFETY (S1): `CombinedSystem` owns two `System` children (`Send + Sync +
//   'static` by that trait's own bound), a `SystemMeta` (`Send + Sync` by
//   composition), a `bool`, and a phantom — all `Send + Sync + 'static`.
//   `run_unsafe` performs no world access of its own; it forwards the caller's
//   `UnsafeEcsCell` to each child in turn, sequentially and never
//   concurrently, so S1 holds for each forwarded call exactly when it holds
//   for the composite's own caller. The two calls do not overlap: the first
//   returns a `bool` before the second begins, so no two child `run_unsafe`
//   invocations are ever in flight on the same world.
unsafe impl<A, B, OP> System for CombinedSystem<A, B, OP>
where
    A: System<Out = bool>,
    B: System<Out = bool>,
    OP: CombineOp,
{
    type Out = bool;

    #[inline]
    fn name(&self) -> &'static str {
        self.meta.name()
    }

    #[inline]
    fn access(&self) -> &Access {
        self.meta.access()
    }

    fn initialize(&mut self, world: &mut EcsMaster) {
        // Children first: their `Access` is populated by their own
        // `init_access` chain, so the union below has to run after.
        self.a.initialize(world);
        self.b.initialize(world);

        if self.access_unioned {
            return;
        }

        // D5(4) — the composite's access is the union of the children's.
        self.meta.access.extend(self.a.access());
        self.meta.access.extend(self.b.access());

        // A child that needs the dispatcher makes the composite need it too;
        // the flag is a disjunction over the params, and the composite's params
        // are the union of its children's.
        if self.a.meta().requires_dispatcher() || self.b.meta().requires_dispatcher() {
            self.meta.mark_requires_dispatcher();
        }

        // Mirror `FunctionSystem::initialize`'s world-aware tick re-seed
        // (Phase 10 Wave D Step 14 / Option B): replace the `for_testing`
        // sentinel with `current - MAX_CHANGE_AGE` so the composite's own
        // first observation window has the late-added-system semantic. The
        // children re-seed themselves inside their own `initialize` above.
        let current = world.current_tick();
        let seed = Tick::new(current.get().wrapping_sub(MAX_CHANGE_AGE));
        self.meta.last_run = seed;
        self.meta.this_run = seed;

        self.access_unioned = true;
    }

    /// Runs **both** children, then folds — ruling D5.
    ///
    /// # Safety
    ///
    /// **S1** — as for any [`System`]: no other `run_unsafe` may be in flight
    /// on this world. The forwarded child calls inherit that guarantee from
    /// the composite's caller and are strictly sequential.
    #[inline]
    unsafe fn run_unsafe(&mut self, world: UnsafeEcsCell<'_>) -> bool {
        // D5(1) — both children evaluate, unconditionally, in declaration
        // order. Two separate `let` bindings rather than one expression: `&&`
        // / `||` in an expression would short-circuit, and this is the exact
        // line the ruling is about.
        //
        // SAFETY (S1): the caller guarantees no other `System::run_unsafe` is
        //   in flight on this world; this call and the next do not overlap
        //   (the first completes before the second is issued). `UnsafeEcsCell`
        //   is `Copy`, so handing the same cell to both children mints no new
        //   aliasing capability — it is the caller's one capability, used
        //   twice in sequence.
        let lhs = unsafe { self.a.run_unsafe(world) };
        // SAFETY (S1): as above; the LHS call has returned.
        let rhs = unsafe { self.b.run_unsafe(world) };
        OP::combine(lhs, rhs)
    }

    #[inline]
    fn is_gpu(&self) -> bool {
        // Honest disjunction rather than a hardcoded `false`. A GPU child in a
        // run condition is an API misuse (conditions are read-only and run at
        // the barrier), but reporting `false` over one would strip the
        // dispatcher-solo discipline the child was relying on.
        self.a.is_gpu() || self.b.is_gpu()
    }

    #[inline]
    fn apply(&mut self, world: &mut EcsMaster) {
        // Forwarded for completeness as a `System`. Run conditions never reach
        // here — the condition-eval pass calls `run_unsafe` only, and a
        // condition's deferred work is documented as dropped (`SystemConfig::run_if`).
        self.a.apply(world);
        self.b.apply(world);
    }

    #[inline]
    fn meta(&self) -> &SystemMeta {
        &self.meta
    }

    fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick) {
        self.meta.last_run = last_run;
        self.meta.this_run = this_run;

        // D5(5) — forwarded to BOTH children, and derived per child the way
        // `EcsMaster::run_condition` derives it for a leaf: the new `last_run`
        // is that child's OWN previous `this_run`, not the composite's
        // `last_run`. In steady state (both run every reached frame) the two
        // coincide; the per-child derivation is what keeps them coinciding
        // after a frame on which the composite itself was not reached.
        let a_prev = self.a.meta().this_run();
        self.a.set_change_ticks(a_prev, this_run);
        let b_prev = self.b.meta().this_run();
        self.b.set_change_ticks(b_prev, this_run);
    }

    fn check_change_tick(&mut self, current: Tick) {
        self.meta.last_run = self.meta.last_run.check_tick(current);
        self.meta.this_run = self.meta.this_run.check_tick(current);
        // The children hold their own snapshots and are invisible to
        // `Schedule::check_change_ticks` (which walks the boxed condition, not
        // its interior), so the composite is the only thing that can clamp
        // them. Missing this would let a child's `last_run` age past
        // MAX_CHANGE_AGE and silently flip `Tick::is_newer_than`.
        self.a.check_change_tick(current);
        self.b.check_change_tick(current);
    }

    #[inline]
    fn set_zone(&mut self, zone: u16) {
        self.meta.zone = zone;
    }
}

/// The logical negation of a run condition — `unless C`.
///
/// Runs its child every reached frame (there is nothing to short-circuit, but
/// the tick-forwarding half of ruling D5 applies identically) and inverts the
/// verdict.
pub struct Not<A> {
    /// The negated child.
    a: A,
    /// The composite's own meta; access is the child's access verbatim.
    meta: SystemMeta,
    /// `true` once `initialize` has copied the child's access in. See
    /// [`CombinedSystem::access_unioned`].
    access_unioned: bool,
}

impl<A> Not<A>
where
    A: System<Out = bool>,
{
    /// Wraps an already-converted child system. Prefer the free function
    /// [`not`], which accepts anything `IntoSystem<(), bool, _>`.
    pub fn new(a: A) -> Self {
        Self {
            a,
            meta: SystemMeta::for_testing(std::any::type_name::<Self>()),
            access_unioned: false,
        }
    }

    /// Read-only handle on the negated child — the D5(4)/D5(5) assertions
    /// reach the child's access and tick snapshot through it.
    #[inline]
    pub fn inner(&self) -> &A {
        &self.a
    }
}

// SAFETY (S1): `Not<A>` owns one `System` child (`Send + Sync + 'static` by
//   that trait's bound), a `SystemMeta` and a `bool`. `run_unsafe` forwards
//   the caller's cell to the single child and inverts the result; S1 holds for
//   the forwarded call exactly when it holds for the composite's caller.
unsafe impl<A> System for Not<A>
where
    A: System<Out = bool>,
{
    type Out = bool;

    #[inline]
    fn name(&self) -> &'static str {
        self.meta.name()
    }

    #[inline]
    fn access(&self) -> &Access {
        self.meta.access()
    }

    fn initialize(&mut self, world: &mut EcsMaster) {
        self.a.initialize(world);
        if self.access_unioned {
            return;
        }
        self.meta.access.extend(self.a.access());
        if self.a.meta().requires_dispatcher() {
            self.meta.mark_requires_dispatcher();
        }
        let current = world.current_tick();
        let seed = Tick::new(current.get().wrapping_sub(MAX_CHANGE_AGE));
        self.meta.last_run = seed;
        self.meta.this_run = seed;
        self.access_unioned = true;
    }

    /// # Safety
    ///
    /// **S1** — as for any [`System`]; the forwarded child call inherits the
    /// caller's guarantee.
    #[inline]
    unsafe fn run_unsafe(&mut self, world: UnsafeEcsCell<'_>) -> bool {
        // SAFETY (S1): the caller guarantees no other `System::run_unsafe` is
        //   in flight on this world; this is the sole forwarded call.
        !unsafe { self.a.run_unsafe(world) }
    }

    #[inline]
    fn is_gpu(&self) -> bool {
        self.a.is_gpu()
    }

    #[inline]
    fn apply(&mut self, world: &mut EcsMaster) {
        self.a.apply(world);
    }

    #[inline]
    fn meta(&self) -> &SystemMeta {
        &self.meta
    }

    fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick) {
        self.meta.last_run = last_run;
        self.meta.this_run = this_run;
        let prev = self.a.meta().this_run();
        self.a.set_change_ticks(prev, this_run);
    }

    fn check_change_tick(&mut self, current: Tick) {
        self.meta.last_run = self.meta.last_run.check_tick(current);
        self.meta.this_run = self.meta.this_run.check_tick(current);
        self.a.check_change_tick(current);
    }

    #[inline]
    fn set_zone(&mut self, zone: u16) {
        self.meta.zone = zone;
    }
}

/// Negates a run condition — the `unless C` front.
///
/// ```ignore
/// use boyko_ecs::ecs::core::schedule::not;
///
/// builder.add_system(idle_chatter).run_if(not(in_combat));
/// ```
#[inline]
pub fn not<C, M>(condition: C) -> Not<C::System>
where
    C: IntoSystem<(), bool, M>,
    C::System: System<Out = bool>,
{
    Not::new(C::into_system(condition))
}

/// Fluent combinators on anything that can become a run condition.
///
/// Blanket-implemented for every `IntoSystem<(), bool, M>`, so a bare `fn`, a
/// closure, or an already-built `System<Out = bool>` all gain `.and(..)` /
/// `.or(..)`. The result is itself a `System<Out = bool>`, hence a valid
/// argument to a further `.and` / `.or` / [`not`] and to
/// [`SystemConfig::run_if`](super::system_config::SystemConfig::run_if).
///
/// ```ignore
/// use boyko_ecs::ecs::core::schedule::{ConditionExt, not, run_once};
///
/// // (a && b) || !c  — a shape the chained `.run_if(a).run_if(b)` fold
/// // cannot express, because that fold owns the operator.
/// builder
///     .add_system(sys)
///     .run_if(a.and(b).or(not(c)));
/// ```
pub trait ConditionExt<M>: IntoSystem<(), bool, M> + Sized {
    /// Eager logical AND with `other`.
    ///
    /// Both sides run every reached frame — the same rule the OR carries, for
    /// the same reason: a skipped side's change-tick window would freeze.
    fn and<C, M2>(self, other: C) -> CombinedSystem<Self::System, C::System, AndOp>
    where
        C: IntoSystem<(), bool, M2>,
        Self::System: System<Out = bool>,
        C::System: System<Out = bool>,
    {
        CombinedSystem::new(Self::into_system(self), C::into_system(other))
    }

    /// Eager logical OR with `other` — the `when (A or B)` front.
    ///
    /// The RHS runs even on a frame the LHS already returned `true`. That is
    /// ruling D5, not an oversight; see the module docs.
    fn or<C, M2>(self, other: C) -> CombinedSystem<Self::System, C::System, OrOp>
    where
        C: IntoSystem<(), bool, M2>,
        Self::System: System<Out = bool>,
        C::System: System<Out = bool>,
    {
        CombinedSystem::new(Self::into_system(self), C::into_system(other))
    }
}

impl<T, M> ConditionExt<M> for T where T: IntoSystem<(), bool, M> {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The operator ZSTs fold as advertised. A unit-level pin so a typo in
    /// `AndOp` / `OrOp` cannot hide behind the (much larger) behavioural
    /// tests in `tests/ke5_condition_combinators.rs`.
    #[test]
    fn combine_ops_have_the_boolean_truth_tables() {
        assert!(!AndOp::combine(false, false));
        assert!(!AndOp::combine(false, true));
        assert!(!AndOp::combine(true, false));
        assert!(AndOp::combine(true, true));

        assert!(!OrOp::combine(false, false));
        assert!(OrOp::combine(false, true));
        assert!(OrOp::combine(true, false));
        assert!(OrOp::combine(true, true));
    }

    /// `Access::extend` is a set union in both directions — the primitive
    /// D5(4) rests on. Asserted here rather than only through a composite so a
    /// broken union cannot be mistaken for a broken forwarding.
    #[test]
    fn access_extend_unions_both_sides() {
        use crate::ecs::identifiers::primitives::{ComponentId, ResourceId};

        let mut lhs = Access::new();
        lhs.add_component_read(ComponentId(3));
        lhs.add_resource_write(ResourceId(1));

        let mut rhs = Access::new();
        rhs.add_component_write(ComponentId(9));
        rhs.add_resource_read(ResourceId(7));

        lhs.extend(&rhs);

        assert!(lhs.component_reads.contains(ComponentId(3)), "lhs read kept");
        assert!(
            lhs.component_writes.contains(ComponentId(9)),
            "rhs write absorbed"
        );
        assert!(lhs.resource_writes.get(1), "lhs resource write kept");
        assert!(lhs.resource_reads.get(7), "rhs resource read absorbed");
    }
}
