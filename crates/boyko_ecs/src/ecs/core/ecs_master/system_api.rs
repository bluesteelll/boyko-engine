//! Ad-hoc system execution surface on [`EcsMaster`] (mechanical split).
//!
//! `run_system_once` / `run_closure_once` / `run_system` / `run_cached_system`
//! and the `run_condition` helper. Extracted verbatim from `ecs_master.rs`.

use crate::ecs::core::change_detection::Tick;
use crate::ecs::core::system::{
    dispatcher_token::DispatcherToken, into_system::IntoSystem, system::System,
    unsafe_ecs_cell::UnsafeEcsCell,
};
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;

impl EcsMaster {
    // ── System execution (Phase 8a Step 8) ──────────────────────────────────

    /// Runs a single [`System`] once, end-to-end.
    ///
    /// Generic over `S: System` so the caller's system value survives across
    /// calls without virtual dispatch. Sequence:
    ///   1. [`System::initialize`] — idempotent two-phase init (state then
    ///      access surface); subsequent calls short-circuit so cross-call
    ///      `&mut S` reuse is supported.
    ///   2. `DispatcherToken::new` — mints the dispatcher-solo capability
    ///      bound to the `&mut self` borrow scope.
    ///   3. [`System::run_dispatcher`] — invokes the system body. The default
    ///      forwards to [`System::run_unsafe`] via the token's cell, so a CPU
    ///      system is byte-identical to the prior `run_unsafe` path; a
    ///      `GpuCompute` system overrides it to reach its `!Send` resource
    ///      through the token (Phase 5 Option C).
    ///
    /// This is a dispatcher-solo entry point: `&mut self` is exclusive for the
    /// whole call, so `running == 0` at the language level (no worker is live).
    /// Phase 9's scheduler runs the same `run_dispatcher` on its own
    /// dispatcher-solo path.
    ///
    /// [`System`]: crate::ecs::core::system::system::System
    /// [`System::initialize`]: crate::ecs::core::system::system::System::initialize
    /// [`System::run_unsafe`]: crate::ecs::core::system::system::System::run_unsafe
    /// [`System::run_dispatcher`]: crate::ecs::core::system::system::System::run_dispatcher
    pub fn run_system_once<S: System>(&mut self, system: &mut S) -> S::Out {
        system.initialize(self);
        // SAFETY (Option C / S1'): `&mut self` is exclusive for the entire call
        //   ⇒ `running == 0` (no worker is live, no other `run_unsafe` /
        //   `run_dispatcher` in flight on this `EcsMaster`) — exactly the
        //   dispatcher-solo context `DispatcherToken::new` requires. The token
        //   does not outlive the `&mut self` borrow: it is consumed by
        //   `run_dispatcher` on the next line and cannot escape.
        let token = unsafe { DispatcherToken::new(self) };
        // SAFETY (S1'): the token witnesses `running == 0` (it is mintable only
        //   in the dispatcher-solo context above), so no other system body is in
        //   flight on this world.
        unsafe { system.run_dispatcher(token) }
    }

    /// Deprecated alias for [`run_system`](EcsMaster::run_system), retained
    /// for Phase 8a callsite compatibility (W3 turbofish form removed —
    /// the closure's param type now infers from its signature).
    ///
    /// ```ignore
    /// // Phase 8a (W3 turbofish — no longer accepted):
    /// // ecs.run_closure_once::<(Res<A>, ResMut<B>), _, _>(|(a, b)| { /* ... */ });
    ///
    /// // Phase 8c (post Step 5 — closure-annotation form):
    /// ecs.run_closure_once(|(a, b): (Res<A>, ResMut<B>)| { /* ... */ });
    /// ```
    ///
    /// New code should call [`run_system`](EcsMaster::run_system) directly;
    /// `run_closure_once` is preserved as a compatibility shim and may be
    /// removed in Phase 9.
    ///
    /// [`run_system`]: EcsMaster::run_system
    #[inline]
    pub fn run_closure_once<F, M, Out>(&mut self, body: F) -> Out
    where
        F: IntoSystem<(), Out, M>,
        F::System: System<Out = Out>,
    {
        self.run_system(body)
    }

    // ── Phase 8c Step 4: `run_system` / `run_cached_system` ──────────────────

    /// Build a one-shot system from any function `F: SystemParamFunction<M>`
    /// (via [`IntoSystem`]), run it once, flush its deferred buffers, and
    /// discard.
    ///
    /// The function is moved in; if you want to amortise the state init
    /// across many invocations, use [`run_cached_system`] with a pre-built
    /// [`FunctionSystem`] hoisted outside your loop. Per-call `run_system`
    /// rebuilds the system on every call (≈ 1 µs cold init + ≤ 30 ns
    /// dispatch + closure body + apply — see plan §1.2 first-call row).
    ///
    /// # Example
    ///
    /// ```ignore
    /// ecs.run_system(|res: Res<MyResource>| {
    ///     println!("{}", res.0);
    /// });
    /// ```
    ///
    /// # Borrow-checker enforced invariants (S1, APP4)
    ///
    /// `&mut self` is exclusive for the entire call; no other `System` can
    /// be in flight on the same world, and no `apply` re-entry into
    /// `run_system` / `run_cached_system` / `run_system_once` is reachable
    /// (Rust's borrow checker rejects the nested `&mut`).
    ///
    /// [`IntoSystem`]: crate::ecs::core::system::into_system::IntoSystem
    /// [`FunctionSystem`]: crate::ecs::core::system::function_system::FunctionSystem
    /// [`run_cached_system`]: EcsMaster::run_cached_system
    pub fn run_system<F, M, Out>(&mut self, system: F) -> Out
    where
        F: IntoSystem<(), Out, M>,
        F::System: System<Out = Out>,
    {
        let mut sys = F::into_system(system);
        self.run_cached_system(&mut sys)
    }

    /// Run a pre-built [`System`] once, flushing its deferred buffers.
    ///
    /// Sequence (plan §17 / §9.5):
    ///   1. [`System::initialize`] — idempotent (FS1). Re-running the same
    ///      cached system pays the init cost only on the first call.
    ///   2. `UnsafeEcsCell::new_mutable` — mints the write-capable cell
    ///      bound to the `&mut self` borrow scope.
    ///   3. [`System::run_unsafe`] — body execution under invariant S1.
    ///   4. [`System::apply`] — flushes per-`SystemParam` deferred buffers
    ///      (e.g. `Commands<'s>`'s [`CommandQueue`]) under `&mut self`.
    ///      APP1' — safe method; APP4 — must not re-enter the runner.
    ///
    /// Phase 9's scheduler will replace this method with a multi-system
    /// runner that resolves aliasing via the [`Access`] conflict graph; for
    /// now `&mut EcsMaster` enforces the S1 invariant trivially.
    ///
    /// [`System`]: crate::ecs::core::system::system::System
    /// [`System::initialize`]: crate::ecs::core::system::system::System::initialize
    /// [`System::run_unsafe`]: crate::ecs::core::system::system::System::run_unsafe
    /// [`System::apply`]: crate::ecs::core::system::system::System::apply
    /// [`CommandQueue`]: crate::ecs::core::commands::command_queue::CommandQueue
    /// [`Access`]: crate::ecs::core::system::access::Access
    pub fn run_cached_system<S>(&mut self, system: &mut S) -> S::Out
    where
        S: System,
    {
        system.initialize(self);
        // SAFETY (U_C1): `cell` does not outlive the `&mut self` borrow — it
        //   is consumed by `run_unsafe` on the next line and cannot escape.
        let cell = unsafe { UnsafeEcsCell::new_mutable(self) };
        // SAFETY (S1): `&mut self` is exclusive for the entire call ⇒ no
        //   other `System::run_unsafe` is in flight on this `EcsMaster`.
        //   The Phase 9 scheduler will replace this trivial enforcement
        //   with the `Access` conflict graph.
        let out = unsafe { system.run_unsafe(cell) };
        // APP1' (Round 3 / O3'): `apply` is a SAFE method; the borrow
        //   checker (still holding `&mut self`) prevents re-entry per APP4.
        system.apply(self);
        // NEW-2: drain the world-resident deferred-hook queue so commands a
        // hook/observer enqueued during `apply` (via `DeferredEcsMaster`) are
        // actually applied. This mirrors `Schedule::run`'s apply-window barrier
        // drain (schedule.rs:560 / :889); without it the single-system runner
        // silently loses nested deferred commands. The drain is depth-0-gated
        // (TLS via `hooks::scope`) and `run_cached_system` is a top-level
        // `&mut self` entry at depth 0 — same self-draining discipline the
        // direct-API methods (`create_entity` / `delete_entity`) use.
        self.drain_deferred_hook_queue();
        out
    }

    /// Run a type-erased read-only **run condition** once on `&mut self`,
    /// returning its `bool` verdict (Phase 16, `PHASE-16-PLAN.md` §5.1).
    ///
    /// Mirrors the [`run_cached_system`](Self::run_cached_system) sequence
    /// but takes a `?Sized` `dyn System<Out = bool>` receiver (so it accepts
    /// a `&mut BoolSystem` via `Box::as_mut`) and DELIBERATELY OMITS the
    /// `apply` step:
    ///
    /// 1. [`System::initialize`] — idempotent (FS1); already ran at build,
    ///    so this is a no-op every frame.
    /// 2. `UnsafeEcsCell::new_mutable` — write-capable cell bound to the
    ///    `&mut self` borrow scope.
    /// 3. [`System::run_unsafe`] — the predicate body; returns the `bool`.
    ///
    /// # No `apply` (orchestrator decision, §0-P6a)
    ///
    /// Conditions are pure read-only predicates. A condition that uses
    /// `Commands` / `EventWriter` is a documented logic error; its deferred
    /// commands are DROPPED here (never flushed mid-eval-pass) rather than
    /// applied — flushing structural mutations between two conditions in the
    /// same eval pass would let the second condition observe a half-applied
    /// world. The read-only contract is `debug_assert!`ed at build
    /// (`schedule_builder.rs` Step 1).
    ///
    /// # Change-detection ticks (Phase 16.1)
    ///
    /// This method advances the condition's `(last_run, this_run]` snapshot via
    /// [`System::set_change_ticks`] — but ONLY here, on a frame the condition is
    /// actually evaluated (Bevy "since-last-actual-run" parity). `last_run`
    /// becomes the condition's PREVIOUS `this_run` (frozen across every frame it
    /// was skipped), and `this_run` becomes the caller's frame-start tick
    /// (`Schedule::frame_this_run`). A condition dormant for N frames (gated by
    /// a false set/state condition, or whose members are blocked by
    /// `pred_remaining`) therefore resumes observing ALL changes since its last
    /// actual run — not just since the last frame — so a `Changed<T>` /
    /// `Added<T>` / `Ref<T>` condition no longer silently misses dormant changes
    /// (nor reports always-true). For a condition evaluated every frame the
    /// window is identical to the old frame-start bump.
    ///
    /// [`System::set_change_ticks`]: crate::ecs::core::system::system::System::set_change_ticks
    ///
    /// # Caller precondition
    ///
    /// The dispatcher holds the unique `&mut EcsMaster`, recovered at the
    /// apply-window boundary where `running.count_ones() == 0` — so no
    /// worker holds a live cell copy (the S1 contract). The only call site is
    /// `Schedule::evaluate_ready_conditions` / `set_gate`.
    ///
    /// [`System`]: crate::ecs::core::system::system::System
    /// [`System::initialize`]: crate::ecs::core::system::system::System::initialize
    /// [`System::run_unsafe`]: crate::ecs::core::system::system::System::run_unsafe
    /// [`UnsafeEcsCell::new_mutable`]: crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell::new_mutable
    pub(crate) fn run_condition(
        &mut self,
        condition: &mut dyn System<Out = bool>,
        this_run: Tick,
    ) -> bool {
        // FS1 no-op after build — conditions are initialized once in
        // `ScheduleBuilder::try_build` Step 1, so their `Access` + `Local`
        // state are already live before the first frame.
        condition.initialize(self);
        // Phase 16.1 (Gap #1): advance the condition's tick snapshot ONLY now,
        // on a frame it is actually evaluated. `prev` is the condition's
        // PREVIOUS `this_run` (frozen across skipped frames); the new `this_run`
        // is the dispatcher's `frame_this_run`. This is the single write site
        // for a condition's ticks — there is NO frame-start condition bump.
        let prev = condition.meta().this_run();
        condition.set_change_ticks(prev, this_run);
        // SAFETY (S1 / Phase 16 CR2): `&mut self` is the dispatcher's unique
        //   exclusive borrow on the world, recovered at the apply-window
        //   boundary where `running == 0` (caller-checked) ⇒ no worker holds
        //   a cell copy. `cell` is consumed by `run_unsafe` on the next line
        //   and cannot escape, so no aliasing `UnsafeEcsCell` is minted.
        let cell = unsafe { UnsafeEcsCell::new_mutable(self) };
        // SAFETY (S1): as above — no other `System::run_unsafe` is in flight
        //   on this `EcsMaster` (single-threaded eval at the barrier). The
        //   cell does not outlive this statement.
        unsafe { condition.run_unsafe(cell) }
    }

}
