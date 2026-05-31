//! [`Schedule`] — the runnable artefact produced by
//! [`ScheduleBuilder::build`].
//!
//! See Phase 9 plan §5.4 (dispatcher loop) + §5.4.5.1 (canonical
//! happens-before diagram) + §5.4.5.2 (canonical apply-window drain) +
//! §2.5 EXC1 (exclusive system SAFETY). Wave 5 Step 12 lands the
//! apply-window executor body; Step 13 wires exclusive systems on the same
//! dispatch path; Step 14 leaves the sync-point analyzer as a documented
//! pass-through (the per-system apply call in the apply window already
//! flushes `Commands` correctly under SCH7).
//!
//! # Loop rhythm (plan §5.4.5.1 / Round 3 W-NEW-1)
//!
//! ```text
//! loop:
//!   1. apply_window_drain — if gate fires (pending == running > 0 OR
//!      pending > 0 && running == 0), pop every completion serially and
//!      run `apply(&mut world)` for each. `&mut world` is the dispatcher's
//!      own exclusive borrow — no worker cell aliases it because the gate
//!      proves every dispatched system has reported back.
//!   2. if completed == n → return.
//!   3. mint a fresh `UnsafeEcsCell` from `&mut world` (per-round; plan
//!      §5.4 Round 2 O3). The cell lives for this dispatch round.
//!   4. try_dispatch_ready — for each system with pred_remaining[i] == 0,
//!      not running, not completed, no conflict against running:
//!        - exclusive: run inline on the dispatcher (EXC1) since the
//!          conflict bits force running == 0 anyway.
//!        - concurrent: scope.spawn — worker runs run_unsafe(cell_copy),
//!          pushes completion + fetch_add(pending_apply, Release).
//!   5. if no dispatch happened this round AND something is still running,
//!      park_timeout for at most 100 µs. The completing worker unparks via
//!      ScopeShared::waker; the timeout is the backstop.
//! ```
//!
//! [`ScheduleBuilder::build`]: super::schedule_builder::ScheduleBuilder::build

use std::sync::Arc;
use std::sync::atomic::Ordering;
#[cfg(not(miri))]
use std::time::Duration;

use boyko_threadpool::{Scope, ThreadPool};
use fixedbitset::FixedBitSet;

use crate::ecs::core::change_detection::run_check_ticks_scan;
use crate::ecs::core::component::hooks::scope::DeferredScopeGuard;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::schedule::bitset_intersects::bitset_intersects;
use crate::ecs::core::schedule::conflict_graph::{ConflictGraph, SystemIndex};
use crate::ecs::core::schedule::executor_scratch::ExecutorScratch;
use crate::ecs::core::schedule::system_box::{BoolSystem, SystemBox};
use crate::ecs::core::schedule::system_set::SystemSetId;
use crate::ecs::core::state::StateEntry;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// Park timeout used between consecutive dispatch rounds when at least one
/// system is still running but nothing new is dispatchable.
///
/// The completing worker unparks the dispatcher via `ScopeShared::waker`;
/// the timeout is the backstop for the case where the wake-up raced ahead
/// of the dispatcher's `park_timeout` call. 100 µs matches plan §5.4.5.1.
#[cfg(not(miri))]
const PARK_TIMEOUT: Duration = Duration::from_micros(100);

/// Built schedule — a snapshot of `n` systems, their conflict graph, and
/// the executor scratch state. Constructed exclusively by
/// [`ScheduleBuilder::build`]; mutated only through [`Schedule::run`]
/// (which advances the executor scratch per frame).
///
/// # Field order
///
/// `pool → systems → conflict_graph → executor_scratch`. `pool` and
/// `systems` are touched once per frame at the outer install boundary;
/// `conflict_graph` is read many times per dispatch round (read-only);
/// `executor_scratch` is the hottest field — `running`, `completed`,
/// `pred_remaining`, and `completion_queue` all sit in the dispatcher's L1
/// for the duration of a frame.
///
/// # Lifetime
///
/// `Schedule` owns its own `Arc<ThreadPool>` clone — Phase 9 Q3 / SCH4
/// require the pool to outlive every running frame. The `Arc` is borrowed
/// as `&*self.pool` inside `run` to avoid a per-frame `Arc::clone`.
///
/// [`ScheduleBuilder::build`]: super::schedule_builder::ScheduleBuilder::build
pub struct Schedule {
    /// Pool handle. Cloned from `ScheduleBuilder::pool` during build.
    pub(crate) pool: Arc<ThreadPool>,

    /// Systems in topological order. Stored as a `Vec` so the elements'
    /// heap addresses are stable across frames — the executor leans on
    /// this stability to mint raw pointers per dispatch (see
    /// [`try_dispatch_ready`](Schedule::try_dispatch_ready)).
    pub(crate) systems: Vec<SystemBox>,

    /// Conflict bitsets + ordering DAG predecessor/successor lists.
    /// Read-only throughout the executor's lifetime.
    pub(crate) conflict_graph: ConflictGraph,

    /// Per-frame scratch reused across `Schedule::run` calls.
    pub(crate) executor_scratch: ExecutorScratch,

    // ── Phase 16 — run conditions (touched only via the gated Step 1.5) ──────
    /// `has_condition[i]` set iff system `i` has ANY own condition OR is a
    /// member of ANY conditioned (gating) set. THE 0%-GATE (see
    /// `PHASE-16-PLAN.md` §4): all-zero when no `.run_if` anywhere, so the
    /// executor's condition-eval branch is predicted-not-taken across the
    /// whole run.
    pub(crate) has_condition: FixedBitSet,

    /// Per-system own conditions, indexed by post-topo `SystemIndex` (permuted
    /// alongside `systems` at build, §2.5). `system_conditions[i]` is empty
    /// unless system `i` carried `.run_if`. `len() == systems.len()`.
    pub(crate) system_conditions: Vec<Vec<BoolSystem>>,

    /// Gating sets per system, indexed by post-topo `SystemIndex`: the
    /// transitive sets system `i` belongs to that carry at least one
    /// condition. Empty for systems in no conditioned set.
    /// `len() == systems.len()`.
    pub(crate) system_gating_sets: Vec<Box<[SystemSetId]>>,

    /// Flat set-condition table. The per-frame cached result lives in
    /// `ExecutorScratch::set_cond_*` (indexed by the dense `slot` here), NOT
    /// here. Multiple rows for one `set_id` fold to an AND (§7.1).
    pub(crate) set_conditions: Vec<SetConditionEntry>,

    // ── Phase 17 — state transitions (gated, off the executor hot path) ──────
    /// Type-erased registry of state transitions, one [`StateEntry`] per
    /// registered `State<S>` (`PHASE-17-PLAN.md` §4.2). EMPTY for a no-state
    /// schedule ⇒ the once-per-frame state pass early-outs on a single
    /// `is_empty()` compare (THE 0%-gate, §6.3), the twin of `has_condition`.
    ///
    /// Appended as the **LAST** field (M3): every pre-existing field keeps its
    /// exact offset, so the hot prefix
    /// (`pool → systems → conflict_graph → executor_scratch → has_condition`)
    /// documented above is byte-for-byte unchanged.
    pub(crate) state_entries: Vec<StateEntry>,
}

/// One set-condition row (Phase 16, `PHASE-16-PLAN.md` §2.4). `slot` is the
/// dense index into `ExecutorScratch::{set_cond_evaluated, set_cond_result}`
/// used to memoize the per-frame verdict (a set condition runs exactly once
/// per frame regardless of how many members it gates).
pub(crate) struct SetConditionEntry {
    /// The set this condition gates. Multiple rows may share a `set_id`.
    pub(crate) set_id: SystemSetId,
    /// The erased read-only predicate. Initialized at build.
    pub(crate) condition: BoolSystem,
    /// Dense index into the per-frame memo bitsets in `ExecutorScratch`.
    pub(crate) slot: u16,
}

impl Schedule {
    /// Runs one frame of the schedule.
    ///
    /// Executes every registered system exactly once (SCH6) under the
    /// apply-window aliasing contract (SCH7). The dispatcher thread runs
    /// this function; worker threads pick up dispatched system bodies via
    /// the [`Scope::spawn`] mechanism. Returns only after every system has
    /// both run and applied.
    ///
    /// # Panics
    ///
    /// * Re-raises the first panic observed by any worker (TPN9 / SCH11)
    ///   on the dispatcher thread, surfaced through `Scope::Drop`.
    /// * `debug_assert!`s `SystemBox::is_exclusive == access().is_universal()`
    ///   for every system (plan §13.6 SCH15).
    ///
    /// [`Scope::spawn`]: boyko_threadpool::Scope::spawn
    pub fn run(&mut self, world: &mut EcsMaster) {
        // SCH15 (Round 2 C9 / OQ-4) — confirm the build-time cache still
        // matches the system's declared access. A future refactor that
        // mutates `Access` after build would desync; catching it here is
        // load-bearing for the exclusive-system gate inside the loop.
        debug_assert!(
            self.systems
                .iter()
                .all(|sb| sb.is_exclusive == sb.system.access().is_universal()),
            "invariant SCH15: SystemBox::is_exclusive desynced from access().is_universal()"
        );

        self.executor_scratch.reset_for_frame(&self.conflict_graph);

        // Phase 10 Wave D Step 13 — frame-start change-detection tick bump
        // (plan §4.5 / PHASE9.1). One `fetch_add(Relaxed)` per
        // `Schedule::run`; the returned value is the new `this_run`
        // published to every system below.
        let this_run = world.bump_change_tick();

        // Phase 10 Wave D Step 13 — conditional wraparound clamp scan
        // (plan §2.7 WRAP1-WRAP2). `should_run_check_ticks` fires roughly
        // every `CHECK_TICK_THRESHOLD` frames ≈ ~100 days at 60 FPS; the
        // hot-path cost is a single u32 compare per `Schedule::run`.
        if world.should_run_check_ticks() {
            run_check_ticks_scan(world);
            world.set_last_check_tick(this_run);
        }

        // Phase 10 Wave D Step 13 — per-system tick snapshot dispatch
        // (plan §2.6 SCT4 / PHASE9.2). Each system's PREVIOUS `this_run`
        // becomes its new `last_run`; its new `this_run` is the
        // dispatcher-wide value just bumped. This is the SINGLE write
        // site for both ticks per frame — `System::set_change_ticks` has
        // no default body so every impl must declare it (plan §5.4-bis).
        //
        // The write happens here (before the empty-schedule short-circuit
        // and before the executor loop) so that workers spawned later in
        // this same frame observe consistent tick state through
        // `&SystemMeta` captured by Query / SystemChangeTick. The
        // dispatcher's sequential write happens-before every worker spawn
        // (plan §8.2).
        for sys_box in self.systems.iter_mut() {
            let prev_this_run = sys_box.system.meta().this_run();
            sys_box.system.set_change_ticks(prev_this_run, this_run);
        }

        // Phase 16.1 (B-1 fix) — extend the per-frame tick snapshot dispatch to
        // run conditions. A condition is an ordinary `System<Out = bool>`; like
        // any system it carries a `SystemMeta` with `last_run` / `this_run`
        // ticks. `run_condition` (`ecs_master.rs`) calls `initialize` (FS1
        // no-op after build) + `run_unsafe` but NOT `set_change_ticks`, so
        // without this loop a condition's ticks stay frozen at the `initialize`
        // sentinel (`current - MAX_CHANGE_AGE`) forever — every per-row tick
        // then reads as "changed since last_run" and a `Changed<T>` / `Added<T>`
        // / `Ref<T>` condition silently reports ALWAYS-TRUE. Bumping the
        // condition ticks here, with the SAME `this_run` as the systems, makes
        // tick-based conditions observe the correct `(last_run, this_run]`
        // window and fire only when the data actually changed.
        //
        // This stays on the cold once-per-frame path. The hot dispatch loop
        // (`try_dispatch_ready`) and the 0%-gate (`has_condition.is_clear()`)
        // are untouched: a condition-free schedule has empty
        // `system_conditions` / `set_conditions`, so both loops below are no-ops
        // and add only two `is_empty()`-equivalent length checks per frame.
        for own_conds in self.system_conditions.iter_mut() {
            for cond in own_conds.iter_mut() {
                let prev_this_run = cond.meta().this_run();
                cond.set_change_ticks(prev_this_run, this_run);
            }
        }
        for entry in self.set_conditions.iter_mut() {
            let prev_this_run = entry.condition.meta().this_run();
            entry.condition.set_change_ticks(prev_this_run, this_run);
        }

        // Phase 17 — state transition pass. THE 0%-GATE: a no-state schedule has
        // `state_entries` empty ⇒ one `is_empty()` compare, predicted-not-taken.
        // Runs BEFORE the executor loop so `evaluate_ready_conditions` (Step 1.5)
        // observes the freshly-written record the SAME frame. Reuses the
        // frame-start `this_run` (no second bump). Holds the dispatcher's unique
        // `&mut world` directly — `pool.install` is not entered yet, no worker
        // exists ⇒ trivially race-free, no cell, no `unsafe`.
        if !self.state_entries.is_empty() {
            // `this_run` is the frame-start `Tick`; the erased apply pointers
            // take the raw `u32` counter (it stamps `recorded_tick`, not a
            // change-detection window).
            self.run_state_transitions(world, this_run.get());
        }

        if self.systems.is_empty() {
            return;
        }

        // Round 3 Q3: borrow the pool by reference; no per-frame Arc::clone.
        // We pull the Arc out of `self` first so the install closure can
        // borrow `self` mutably without conflicting with the pool borrow.
        let pool_arc: Arc<ThreadPool> = Arc::clone(&self.pool);
        let pool: &ThreadPool = &pool_arc;

        // The outer `install` sets the dispatcher's TLS `ACTIVE_POOL`
        // pointer for the duration of the closure — `par_iter` from a
        // system body (Wave 6) will find the ambient pool through it.
        // Round 2 W6 still applies: the closure here runs the executor
        // main loop and may spawn arbitrarily many scopes via the inner
        // `scope` argument.
        //
        // The `install` body borrows `self` and `world` for the duration
        // of the install scope, which equals `'scope` from the borrow
        // checker's standpoint. The executor's `'scope` (the cell + spawn
        // captures) coincides with the install closure's lifetime.
        pool.install(|scope| {
            self.executor_main_loop(world, scope);
        });
        drop(pool_arc);

        debug_assert_eq!(
            self.executor_scratch.pending_apply.load(Ordering::Relaxed),
            0,
            "invariant SCH6: pending_apply must be 0 at end of frame"
        );
    }

    /// Runs each registered state's transition apply once per frame
    /// (`PHASE-17-PLAN.md` §6.1). Cold: at most `state_entries.len()` (~4)
    /// monomorphised applies, each a handful of slab reads/writes to cached
    /// `ResourceId`s. Gated by `state_entries.is_empty()` at the call site (THE
    /// 0%-gate, §7), so a no-state schedule never reaches here.
    ///
    /// `fire_initial` is read from each entry's `pending_initial` flag and then
    /// cleared, so the synthesized `none → initial` transition (D7) fires on the
    /// FIRST `Schedule::run` only. Per-entry ⇒ a state shared by two schedules
    /// fires its initial once per schedule.
    ///
    /// Holds the dispatcher's unique `&mut EcsMaster` (the call site runs before
    /// `pool.install`, so no worker is live) — no cell, no `unsafe`.
    #[cold]
    fn run_state_transitions(&mut self, world: &mut EcsMaster, this_run: u32) {
        // §9 invariant: the pass stamps `recorded_tick` with the frame-start
        // `this_run`, so it must match the world's current tick exactly (no
        // second bump between the bump site and here).
        debug_assert!(
            this_run == world.current_tick().get(),
            "invariant: state pass `this_run` must equal the world's current tick"
        );
        for entry in self.state_entries.iter_mut() {
            let fire_initial = entry.pending_initial;
            entry.pending_initial = false;
            (entry.apply)(world, this_run, fire_initial);
            debug_assert!(
                !entry.pending_initial,
                "invariant: pending_initial must be cleared after the first run"
            );
        }
    }

    /// Number of systems in the schedule (post-topological-sort).
    #[inline]
    pub fn len(&self) -> usize {
        self.systems.len()
    }

    /// `true` iff the schedule has no systems.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.systems.is_empty()
    }

    /// Main executor loop. See module docs for the loop rhythm.
    ///
    /// The `'scope` lifetime on `scope` ties every spawned closure to the
    /// outer `install` frame — `Scope::Drop` blocks until every worker
    /// task completes, so the raw pointers captured by the spawned
    /// closures (`system_ptr`, `completion_queue_ptr`, `pending_apply_ptr`)
    /// remain valid for the closure's entire body.
    ///
    /// # Cell lifetime and the apply window reborrow
    ///
    /// `cell` is minted ONCE at the top of this function from the
    /// caller's `&'scope mut EcsMaster`. Its lifetime is `'scope`, which
    /// is what `scope.spawn`'s `'scope` capture bound requires. Per-round
    /// access to `&mut EcsMaster` (for `apply_window_drain`) is recovered
    /// via `unsafe { cell.world_mut() }` — the cell carries write-capable
    /// provenance, and the apply-window barrier (SCH7) guarantees no
    /// concurrent worker holds a cell copy at the moment of the reborrow
    /// (the gate `pending == running.count_ones()` proves it).
    ///
    /// This matches plan §5.4.5.1's per-round-mint intent: the LOGICAL
    /// cell is "refreshed" each iteration via the `world_mut` reborrow,
    /// even though the Rust-level value is the same. The reborrow is the
    /// canonical happens-before edge that synchronises the dispatcher's
    /// read with every worker's release `fetch_add` (the Acquire load on
    /// `pending_apply` immediately above is the matching barrier).
    fn executor_main_loop<'scope>(
        &mut self,
        world: &'scope mut EcsMaster,
        scope: &Scope<'scope>,
    ) {
        let n = self.systems.len();

        // SAFETY (U_C1, SEND1/SEND3 — Phase 9 §9.2):
        //   `world` is borrowed `&'scope mut` for the install frame; the
        //   cell carries `'scope`. Workers receive `Copy`s of the cell.
        //   Aliasing across cell copies is enforced by the conflict graph
        //   (SCH3) + apply-window barrier (SCH7) — the dispatcher only
        //   recovers `&mut EcsMaster` via `cell.world_mut()` inside
        //   `apply_window_drain`, which is gated on `running == pending`,
        //   guaranteeing no worker still holds a cell copy.
        let cell: UnsafeEcsCell<'scope> = unsafe { UnsafeEcsCell::new_mutable(world) };

        loop {
            // === Step 1: apply window drain (plan §5.4.5.1 gate). ===
            //
            // Monotonicity note (Round 3 W-NEW-4): the combined check
            // `pending == running.count_ones()` is monotone within one
            // outer iteration — both counters only increase during a
            // round, and the dispatcher mutates `running` only inside
            // `try_dispatch_ready` which has already returned by this
            // point. Staleness from reading the two values separately
            // is bounded to one loop iteration; the next iteration sees
            // the stable state. No data race exists: `pending` is
            // atomic; `running` is dispatcher-owned.
            //
            // Acquire ordering on `pending` synchronises-with every
            // worker's Release `fetch_add` (plan §5.4.5.1 diagram), so
            // by the time we proceed into `apply_window_drain` the
            // worker's writes to component bytes are visible to the
            // dispatcher.
            let pending = self.executor_scratch.pending_apply.load(Ordering::Acquire);
            let running = self.executor_scratch.running.count_ones(..);
            if pending > 0 && (pending == running || running == 0) {
                // SAFETY (SCH7 apply window): the gate above proved every
                //   dispatched system has reported completion. No worker
                //   holds the cell at this moment — every worker that
                //   received a cell copy has run past the `fetch_add`
                //   that we just Acquire-loaded. The `world_mut` reborrow
                //   is therefore the exclusive borrow on the world for
                //   the duration of `apply_window_drain`.
                let world_mut: &mut EcsMaster = unsafe { cell.world_mut() };
                self.apply_window_drain(world_mut);
            }

            // === Step 1.5 (Phase 16): evaluate conditions for newly-ready
            //     systems (PHASE-16-PLAN.md §3.2). ===
            //
            // 0%-GATE (§4): skip the whole pass when no `.run_if` exists.
            // `is_clear()` on a ≤16-word bitset is a few ORs; the branch is
            // predicted-not-taken for a condition-free schedule, and
            // `try_dispatch_ready` (Step 3) stays byte-identical.
            if !self.has_condition.is_clear() {
                // Race-freedom (§0-P1 / Proof a): evaluate ONLY when no worker
                // is live. The apply-window gate above either drained every
                // dispatched worker (so `running` is back to 0) OR `running`
                // was already 0. We require `running.count_ones() == 0` before
                // touching the cell as `&mut` — the SAME precondition the
                // inline-exclusive path uses (EXC2). When workers are still in
                // flight we defer condition eval to a later iteration (the
                // dispatcher parks below and wakes on completion).
                if self.executor_scratch.running.count_ones(..) == 0 {
                    // SAFETY (SCH7 / Phase 16 CR2): `running == 0` ⇒ every
                    //   previously dispatched worker has completed AND been
                    //   drained (the apply window above popped them, clearing
                    //   each `running` bit at schedule.rs:331). No worker holds
                    //   a cell copy. This reborrow is the exclusive `&mut
                    //   EcsMaster` for the duration of condition eval, which
                    //   never spawns and never retains a cell-derived borrow
                    //   (`run_condition` consumes its cell and returns a bool).
                    let world_mut: &mut EcsMaster = unsafe { cell.world_mut() };
                    self.evaluate_ready_conditions(world_mut);
                }
            }

            // === Step 2: termination check. ===
            if self.executor_scratch.completed.count_ones(..) == n {
                return;
            }

            // === Step 3+4: dispatch ready systems. ===
            //
            // The cell is COPIED into each spawn; the same `'scope`-lifetimed
            // cell value is shared (per-round logical "refresh" is enforced
            // by the SCH7 barrier and the per-round Acquire load above,
            // not by re-minting the cell value).
            let dispatched = self.try_dispatch_ready(scope, cell);

            // === Step 5: backoff. ===
            //
            // If nothing dispatched but something is running, park until
            // a worker unparks us (the last-completer pattern in
            // `ScopeShared::pending`). The 100 µs timeout is the backstop
            // for the case where the wake-up raced ahead of our park
            // call — a benign no-op spin.
            if dispatched == 0 && self.executor_scratch.running.count_ones(..) > 0 {
                // Under Miri the scheduler is cooperative and does not advance
                // other threads across a `park_timeout` the way it does across
                // an explicit yield; without this, the dispatcher spins this
                // branch forever while the workers never get scheduled
                // (livelock). Yield so Miri can run the workers to completion.
                // Compiles to nothing natively (same discipline as
                // `boyko_threadpool::scope::join_workers_until_drained`).
                #[cfg(miri)]
                std::thread::yield_now();
                #[cfg(not(miri))]
                std::thread::park_timeout(PARK_TIMEOUT);
            }
        }
    }

    /// Drain the completion queue and apply every completed system.
    ///
    /// Precondition (plan §13.6): the apply-window gate fired at the call
    /// site. The dispatcher owns `&mut world` exclusively for the duration
    /// of this function — no worker holds a cell-mediated borrow because
    /// every worker that received the current round's cell has already
    /// pushed its completion and incremented `pending_apply` before this
    /// function reads the count.
    fn apply_window_drain(&mut self, world: &mut EcsMaster) {
        let target = self
            .executor_scratch
            .pending_apply
            .load(Ordering::Acquire);

        let mut drained = 0usize;
        while drained < target {
            // `pending_apply` (the `target` above) is incremented by the worker
            // *after* its `completion_queue.push` (see the completion path), and
            // this drain reads `target` with `Acquire`, which synchronizes-with
            // that `Release` `fetch_add` — so a counted completion is always
            // visible to `pop()` here on real hardware (the `None` arm is
            // unreachable natively; it compiles to a never-taken retry, zero
            // steady-state cost). Under Miri's cooperative scheduler the worker's
            // push may not yet be observable on this step (and `ArrayQueue::pop`'s
            // own internal `Backoff` spin has no Miri yield, being third-party),
            // so yield to let the worker run instead of spinning — without this,
            // the dispatcher livelocks here on the mandatory drain path.
            let idx = match self.executor_scratch.completion_queue.pop() {
                Some(idx) => idx,
                None => {
                    #[cfg(miri)]
                    std::thread::yield_now();
                    continue;
                }
            };
            let i = idx.0 as usize;

            self.executor_scratch.running.set(i, false);

            // SAFETY (SCH7 / apply window):
            //   - Workers are fully drained for the current round; the
            //     gate `pending == running.count_ones()` guaranteed every
            //     dispatched system has pushed its completion.
            //   - The dispatcher holds `&mut world` exclusively (it is
            //     `&mut self`'s caller borrow) — the `apply` call is a
            //     safe method whose signature already encodes that.
            //
            // Phase 14a §8 P1: hold the RAII `DeferredScopeGuard` ACROSS the
            // `apply` call. There is NO schedule-level `catch_unwind` around
            // this site — the only catch is inside `CommandQueue::apply`. If a
            // command panic propagates up through here, the guard's `Drop`
            // decrements the depth during unwind (no leak). The guard touches
            // only the thread-local depth (it caches no pointer), so it does
            // not freeze `world` for the `apply` reborrow.
            {
                let scope = DeferredScopeGuard::enter();
                self.systems[i].system.apply(world);
                drop(scope);
            }
            // Depth is 0 here: drain the deferred-hook queue (Q-A1 outermost
            // owner). A command's own hooks ran at depth >= 1 and only enqueued;
            // they apply now. Re-entrant hook appends are picked up by the
            // drain's `while !is_empty()` loop.
            world.drain_deferred_hook_queue();

            self.executor_scratch.completed.insert(i);

            // Decrement each ordered successor. The plain decrement is
            // sound under Round 3 O-NEW-2 (dispatcher-sole-mutator); no
            // worker reads `pred_remaining`.
            for &successor in self.conflict_graph.successors[i].iter() {
                let s = successor.0 as usize;
                debug_assert!(
                    self.executor_scratch.pred_remaining[s] > 0,
                    "invariant SCH13: pred_remaining must not underflow (system {})",
                    s,
                );
                self.executor_scratch.pred_remaining[s] -= 1;
            }

            drained += 1;
        }

        // Release matches the worker's `fetch_add(Release)` — but here we
        // only `fetch_sub` what we observed; Relaxed is correct because
        // the dispatcher's subsequent operations are sequenced behind a
        // `&mut self` borrow.
        self.executor_scratch
            .pending_apply
            .fetch_sub(target, Ordering::Relaxed);
    }

    /// Evaluate conditions for every conditioned system that is newly ready
    /// (`pred_remaining == 0`, not running, not completed) and whose
    /// conditions have NOT yet been folded this frame. A system whose folded
    /// gate is `false` is marked completed and its successors decremented —
    /// exactly as if it had run and applied, but WITHOUT running the body,
    /// spawning a worker, or bumping `pending_apply` (PHASE-16-PLAN.md §3.3).
    ///
    /// # Precondition
    ///
    /// `running.count_ones() == 0` (caller-checked in Step 1.5). The
    /// dispatcher holds the unique `&mut EcsMaster`; no worker is live, so
    /// `run_condition` may read the world race-free (Proof a).
    ///
    /// # Cascade
    ///
    /// Skipping system `i` lowers its successors' `pred_remaining`. Because
    /// this is a `for i in 0..n` pass in topo order and skip-decrements only
    /// LOWER counts, any successor `s` (necessarily `s > i` in topo order)
    /// that thereby reaches `pred_remaining == 0` is reached LATER in the SAME
    /// pass with the updated count — so a contiguous all-skip chain settles in
    /// one forward pass. A successor made ready by a REAL completion is handled
    /// on the next loop iteration (after the next `apply_window_drain`).
    fn evaluate_ready_conditions(&mut self, world: &mut EcsMaster) {
        let n = self.systems.len();

        for i in 0..n {
            // Reuse the EXACT ready predicate from `try_dispatch_ready`
            // (minus the conflict check — conflicts gate concurrent dispatch,
            // not single-threaded condition eval).
            if self.executor_scratch.completed.contains(i) {
                continue;
            }
            // `running` is all-zero here (caller precondition), but keep the
            // check to mirror the dispatch predicate exactly.
            if self.executor_scratch.running.contains(i) {
                continue;
            }
            if self.executor_scratch.pred_remaining[i] != 0 {
                continue;
            }
            if !self.has_condition.contains(i) {
                continue;
            }
            if self.executor_scratch.cond_evaluated.contains(i) {
                continue;
            }

            self.executor_scratch.cond_evaluated.insert(i);

            // EAGER FOLD (§6): run ALL own conditions + all gating-set
            // conditions, AND the results. NO short-circuit — every condition
            // body runs so stateful conditions (e.g. `run_once`) advance their
            // `Local` every frame they are reached. `should_run &= r` makes the
            // "no break" intent unambiguous (bitwise AND over materialized
            // bools, never a control-flow short-circuit).
            let mut should_run = true;

            // Own conditions. Index by position so the `&mut
            // self.system_conditions[i][k]` borrow is released before the next
            // iteration; `world` is a disjoint parameter (not a field of self).
            let own_len = self.system_conditions[i].len();
            for k in 0..own_len {
                let cond = self.system_conditions[i][k].as_mut();
                let r = world.run_condition(cond);
                should_run &= r;
            }

            // Gating-set conditions (memoized per frame). Snapshot the set ids
            // first so the `&self.system_gating_sets[i]` borrow does not span
            // the `&mut self` reborrow inside `set_gate`.
            let gating_len = self.system_gating_sets[i].len();
            for g in 0..gating_len {
                let set_id = self.system_gating_sets[i][g];
                should_run &= self.set_gate(world, set_id);
            }

            if !should_run {
                // SKIP: mark completed + decrement successors, WITHOUT
                // body / apply / spawn / pending bump.
                self.mark_skipped(i);
            }
            // If `should_run == true`, do nothing — `try_dispatch_ready` picks
            // the system up normally this same loop iteration (Step 3).
        }
    }

    /// Mark system `i` as skipped: set `completed` + decrement successors'
    /// `pred_remaining`. Mirrors the apply-window completion tail
    /// (`schedule.rs:359-372`) MINUS run / apply / queue (PHASE-16-PLAN.md §3.3).
    #[inline]
    fn mark_skipped(&mut self, i: usize) {
        self.executor_scratch.completed.insert(i);
        // Decrement successors — IDENTICAL to the apply-window path
        // (schedule.rs:364-372). A skip is invisible to the apply window's
        // `target`/`drained` accounting (it never pushes a completion).
        for &successor in self.conflict_graph.successors[i].iter() {
            let s = successor.0 as usize;
            debug_assert!(
                self.executor_scratch.pred_remaining[s] > 0,
                "invariant SCH13 (Phase 16): pred_remaining must not underflow on skip (system {})",
                s,
            );
            self.executor_scratch.pred_remaining[s] -= 1;
        }
    }

    /// Memoized set-condition gate (PHASE-16-PLAN.md §7.1). Returns the AND of
    /// every set-condition row for `set_id`; each row's body runs at most ONCE
    /// per frame (the first ready member that depends on it triggers the run;
    /// subsequent members read the cache).
    ///
    /// # Borrow note (R9)
    ///
    /// Indexes `self.set_conditions` by position rather than holding an
    /// iterator, so the `&mut self.set_conditions[k].condition` borrow (passed
    /// to `run_condition`) is released before the disjoint `&mut
    /// self.executor_scratch` memo write. `world` is a parameter, not a field.
    fn set_gate(&mut self, world: &mut EcsMaster, set_id: SystemSetId) -> bool {
        let mut acc = true;
        let rows = self.set_conditions.len();
        for k in 0..rows {
            if self.set_conditions[k].set_id != set_id {
                continue;
            }
            let slot = self.set_conditions[k].slot as usize;
            let r = if self.executor_scratch.set_cond_evaluated.contains(slot) {
                self.executor_scratch.set_cond_result.contains(slot)
            } else {
                // EAGER (§6): run the set-condition body once this frame.
                let cond = self.set_conditions[k].condition.as_mut();
                let v = world.run_condition(cond);
                self.executor_scratch.set_cond_evaluated.insert(slot);
                self.executor_scratch.set_cond_result.set(slot, v);
                v
            };
            acc &= r; // eager AND across a set's own conditions
        }
        acc
    }

    /// Find and dispatch every system that is ready this round.
    ///
    /// Returns the number of systems dispatched (incl. exclusive systems
    /// run inline). The caller uses the count to decide whether to park.
    ///
    /// # Borrow-checker dance
    ///
    /// The naive shape "iterate over `self.systems`, conditionally call
    /// `scope.spawn` with `&mut self.systems[i].system`" runs into the
    /// borrow checker's prohibition on holding `&mut self` while passing
    /// a closure (with a `'scope` upper bound on its captures) to
    /// `scope.spawn`. We sidestep it by:
    ///
    /// 1. Collecting the dispatchable indices into a small scratch vec
    ///    (the count is bounded by the system count, ≤ 1024).
    /// 2. For exclusive systems, running them inline on the dispatcher
    ///    inside the same loop — no spawn aliasing.
    /// 3. For concurrent systems, lifting raw pointers
    ///    (`*mut Box<dyn System>`, `*const ArrayQueue`, `*const AtomicUsize`)
    ///    from `&mut self` *before* the closure capture. The pointers are
    ///    `Send` (we wrap them in a small `SpawnPointers` `Copy` struct
    ///    with `unsafe impl Send`) and they remain valid for the entire
    ///    `'scope` window because:
    ///      - `self.systems` is a `Vec<SystemBox>` — the box pointers
    ///        do not move across frames; only the inner `Box<dyn System>`
    ///        is consumed by `run_unsafe(&mut self, ...)`.
    ///      - `self.executor_scratch.completion_queue` and
    ///        `self.executor_scratch.pending_apply` are fields of `self`
    ///        which outlives the `'scope` borrow (Scope::Drop blocks
    ///        until the spawn completes; we're still inside
    ///        `executor_main_loop` which holds `&'scope mut self`).
    ///      - The `ConflictGraph::conflict_bits` already-disjoint-by-graph
    ///        invariant (SCH3) guarantees no two concurrently-dispatched
    ///        systems alias the same `system_ptr` — each ptr points to a
    ///        distinct slot in `self.systems` and the conflict bits prevent
    ///        two parallel systems from sharing any worker resource that
    ///        would alias.
    ///
    /// The pointer escape is documented in the spawn's SAFETY block.
    fn try_dispatch_ready<'scope>(
        &mut self,
        scope: &Scope<'scope>,
        cell: UnsafeEcsCell<'scope>,
    ) -> usize {
        let n = self.systems.len();
        let running_count = self.executor_scratch.running.count_ones(..);

        // Cheap scratch — bounded by the number of systems (≤ 1024).
        // The scratch is a stack-local Vec because we cannot easily reuse
        // the `executor_scratch.ready_scratch` bitset and at the same time
        // iterate it while mutating `self.executor_scratch.running` below.
        //
        // Two buckets so exclusive systems can short-circuit when
        // `running_count > 0`. Allocation cost is dominated by the dispatch
        // path itself (~120 ns/spawn per plan §10.5); negligible.
        let mut exclusive_to_run: Vec<SystemIndex> = Vec::new();
        let mut to_spawn: Vec<SystemIndex> = Vec::new();

        for i in 0..n {
            if self.executor_scratch.completed.contains(i) {
                continue;
            }
            if self.executor_scratch.running.contains(i) {
                continue;
            }
            if self.executor_scratch.pred_remaining[i] != 0 {
                continue;
            }

            // Conflict check: does this system's conflict bitset intersect
            // the current `running` set? `bitset_intersects` is the
            // AVX2-fast-path helper from Wave 4 Step 10.
            if bitset_intersects(
                &self.conflict_graph.conflict_bits[i],
                &self.executor_scratch.running,
            ) {
                continue;
            }

            if self.systems[i].is_exclusive {
                // EXC2: exclusive systems require `running == 0` at
                // dispatch time. If anything is running we defer to the
                // next round (the apply window will drain and the next
                // call to this function will reconsider).
                if running_count > 0 {
                    continue;
                }
                exclusive_to_run.push(SystemIndex(i as u16));
                // An exclusive system, once accepted, blocks everything
                // else in this round (its conflict bits intersect every
                // running system after it). Stop scanning to preserve the
                // single-exclusive-per-round invariant.
                break;
            }

            to_spawn.push(SystemIndex(i as u16));
            // Mark running NOW so subsequent indices in this loop see the
            // updated running set for their conflict check.
            self.executor_scratch.running.insert(i);
        }

        debug_assert!(
            // Plan §13.6 W7 post-condition (relaxed form): every spawned
            // system has running[i] set; nothing in `to_spawn` is also
            // in `completed`.
            to_spawn.iter().all(|idx| {
                let i = idx.0 as usize;
                self.executor_scratch.running.contains(i)
                    && !self.executor_scratch.completed.contains(i)
            }),
            "find_ready post-condition: dispatched systems must be marked running and not completed"
        );

        let mut dispatched = 0usize;

        // === Exclusive path (plan §2.5 EXC1, Step 13). ===
        for idx in &exclusive_to_run {
            let i = idx.0 as usize;
            self.executor_scratch.running.insert(i);

            // SAFETY (EXC1 — Round 3 W-NEW-5):
            //   - Universal access ⇒ conflict graph forces `running == 0`
            //     before dispatch. We checked `running_count == 0` above
            //     before pushing to `exclusive_to_run`; nothing was spawned
            //     in this call.
            //   - `cell.world_mut()` reborrows `&mut EcsMaster` from the
            //     cell minted in the current dispatch round from the
            //     dispatcher's own &mut world. No other reference exists.
            //   - The exclusive system body must not retain any
            //     cell-derived borrow past return. The dispatcher
            //     reborrows via the same cell for `apply` immediately
            //     below; a stashed pointer would alias the apply
            //     reborrow.
            //   - Calling `run_unsafe(cell)` is safe under S1 because no
            //     worker is running.
            let world_ref: &mut EcsMaster = unsafe { cell.world_mut() };
            // Use the `&mut self.systems[i].system` borrow ONLY locally;
            // it does not escape because we are not in a spawn closure
            // here. The cell carries write capability; reborrowing
            // `world_ref` for `apply` after `run_unsafe` returns is fine
            // because `run_unsafe` consumed its `cell` argument by value
            // and the cell's lifetime is `'scope >= 'fn-body`.
            unsafe {
                self.systems[i].system.run_unsafe(cell);
            }
            // Apply runs inline (no completion-queue round-trip). The
            // reborrow is via the same cell, which is rooted in the same
            // `&mut world` provenance; no aliasing.
            //
            // Phase 14a §8 P1: bracket `apply` with the RAII guard (same
            // panic-safety reasoning as the concurrent site — no schedule-level
            // catch_unwind), then drain at depth 0.
            {
                let scope = DeferredScopeGuard::enter();
                self.systems[i].system.apply(world_ref);
                drop(scope);
            }
            world_ref.drain_deferred_hook_queue();
            self.executor_scratch.running.set(i, false);
            self.executor_scratch.completed.insert(i);

            // Update successors' pred_remaining.
            for &succ in self.conflict_graph.successors[i].iter() {
                let s = succ.0 as usize;
                debug_assert!(
                    self.executor_scratch.pred_remaining[s] > 0,
                    "invariant SCH13: pred_remaining must not underflow (system {})",
                    s,
                );
                self.executor_scratch.pred_remaining[s] -= 1;
            }
            dispatched += 1;
        }

        // If we ran an exclusive system inline, do not spawn any
        // concurrent systems in this call — the apply window already
        // ran for the exclusive; the next outer iteration will pick up
        // any newly-ready successors. (Also: `to_spawn` will be empty
        // because the `break` above abandoned the scan, but the explicit
        // early return makes intent clearer.)
        if !exclusive_to_run.is_empty() {
            return dispatched;
        }

        // === Concurrent path (Step 12). ===
        //
        // Lift the raw pointers ONCE; the closures capture the `Copy`
        // wrapper (SpawnPointers) by value. The pointer values remain
        // valid for the duration of the `'scope` borrow because:
        //   * `self.systems` is `Vec<SystemBox>` — its heap buffer's
        //     address is stable across frames and no system is added /
        //     removed mid-run (SCH1).
        //   * `self.executor_scratch.completion_queue` and
        //     `pending_apply` are direct fields of `self`. `self` is
        //     `&'scope mut` for the duration of `executor_main_loop`;
        //     spawned closures end before that borrow ends because
        //     `Scope::Drop` blocks until all spawns complete.
        if to_spawn.is_empty() {
            return dispatched;
        }

        // Pointer set captured by each spawn closure. Pull these out of
        // `self` once; we will not re-borrow `self.executor_scratch`
        // until the closures have been packaged (each closure only
        // dereferences pointers it received by value, not via `&self`).
        let systems_ptr: *mut SystemBox = self.systems.as_mut_ptr();
        let completion_queue_ptr: *const crossbeam_queue::ArrayQueue<SystemIndex> =
            &self.executor_scratch.completion_queue as *const _;
        // `CachePadded<T>::deref` yields `&T` — take a raw pointer to the
        // inner atomic so we don't have to bind a temporary reference.
        let pending_apply_ptr: *const std::sync::atomic::AtomicUsize =
            &*self.executor_scratch.pending_apply as *const _;

        for idx in to_spawn {
            let ptrs = SpawnPointers {
                systems: systems_ptr,
                completion_queue: completion_queue_ptr,
                pending_apply: pending_apply_ptr,
            };
            let sys_idx = idx;
            let cell_copy = cell;

            // SAFETY (S1, SCH3, SCH7, SEND1/SEND3 — plan §5.4):
            //   - `systems_ptr.add(sys_idx.0)` references a `SystemBox`
            //     whose `system` field is exclusively owned by this task
            //     for its duration: `running[sys_idx]` is set and the
            //     conflict graph ensures no other concurrent dispatch
            //     picks the same index. Two concurrent systems may both
            //     alias the `systems` Vec at different indices — that is
            //     not UB because the Box<dyn System>'s exclusively reside
            //     in disjoint vec slots.
            //   - `cell_copy` is a Copy of the cell minted in the current
            //     round from `&mut world`; conflict bits enforce
            //     non-aliasing across cell copies (SCH3); the apply
            //     window (SCH7) guarantees the dispatcher does not
            //     reborrow `&mut world` while this task is alive.
            //   - `completion_queue_ptr` and `pending_apply_ptr` are
            //     pointers to atomic / lock-free fields of `self`; the
            //     dispatcher does not drop or move them while `'scope`
            //     is alive (Scope::Drop holds the dispatcher).
            //   - Cell ↑Send (SEND3); SystemBox ↑Send via Box<dyn System
            //     + Send + Sync + 'static>; atomic / ArrayQueue ↑Send.
            scope.spawn(move || {
                // Allocation discipline (ALLOC1 / ALLOC6): set the TLS
                // flag so that any arena allocation inside the body
                // trips the `Arena::allocate_*` debug assertion.
                let _alloc_guard = boyko_threadpool::InSystemRunGuard::enter();

                // SAFETY (S1 / SCH3): see outer SAFETY block. The cell
                //   copy carries write-capable provenance; aliasing is
                //   enforced upstream by the conflict graph. Method
                //   receivers force whole-`ptrs` capture (sidestepping
                //   Rust 2021 disjoint capture which would otherwise
                //   reduce the closure's `Send` bound to per-field
                //   `*const T` Send-ness).
                unsafe {
                    let system_slot = ptrs.system_slot(sys_idx.0 as usize);
                    (*system_slot).system.run_unsafe(cell_copy);
                }
                // Drop the guard BEFORE publishing completion so a
                // dispatcher that observes `pending_apply == running`
                // cannot still find a worker inside the system body
                // (closing the SP1' window for the future force_alloc
                // CI mode).
                drop(_alloc_guard);

                // SAFETY: completion_queue_ptr / pending_apply_ptr were
                //   minted from references to fields of `self`; `self`
                //   outlives the spawn (Scope::Drop). The push is
                //   infallible because capacity ≥ system_count (SCH6
                //   guarantees one push per system per frame).
                unsafe {
                    ptrs.completion_queue()
                        .push(sys_idx)
                        .expect("invariant SCH6: completion_queue cap ≥ system_count");
                    // Release pairs with the dispatcher's Acquire load
                    // on `pending_apply` (plan §5.4.5.1 diagram). Every
                    // byte the body wrote becomes visible to the
                    // dispatcher before it reads pending == target.
                    ptrs.pending_apply().fetch_add(1, Ordering::Release);
                }
            });

            dispatched += 1;
        }

        dispatched
    }
}

/// Raw-pointer bundle captured by each spawn closure in
/// [`Schedule::try_dispatch_ready`]. The pointers reference fields of the
/// owning `Schedule`; their validity for the closure's lifetime is
/// established by the surrounding SAFETY block (see
/// `try_dispatch_ready`'s spawn invocation).
///
/// # Disjoint-capture sidestep
///
/// Rust 2021 disjoint capture (RFC 2229) makes a `move ||` closure
/// capture *individual fields* it touches, not the whole struct. The
/// per-field `*const T` does not implement `Send`, so a closure that
/// touched two fields directly would be `!Send`. To force whole-struct
/// capture, every field access is mediated by an `&self` method —
/// disjoint capture cannot decompose a borrow of the receiver across
/// fields, so the closure captures the whole `SpawnPointers` value.
///
/// `Copy` so the closure captures the value rather than a borrow; `Send`
/// so it can cross the worker boundary.
#[derive(Clone, Copy)]
struct SpawnPointers {
    /// Base pointer into `Schedule::systems`. Index by `SystemIndex.0`.
    systems: *mut SystemBox,
    /// Pointer to the `Schedule::executor_scratch.completion_queue` field.
    completion_queue: *const crossbeam_queue::ArrayQueue<SystemIndex>,
    /// Pointer to the `Schedule::executor_scratch.pending_apply` atomic.
    pending_apply: *const std::sync::atomic::AtomicUsize,
}

impl SpawnPointers {
    /// Returns the `SystemBox` slot for the given system index.
    ///
    /// # Safety
    /// * The pointer must point inside the owning `Schedule::systems`
    ///   Vec, which is `'scope`-alive (Scope::Drop blocks).
    /// * No concurrent worker may alias the same `SystemBox` (enforced
    ///   by the conflict graph + `running` bitset; see SCH3).
    #[inline]
    unsafe fn system_slot(&self, idx: usize) -> *mut SystemBox {
        // SAFETY (S1 + SCH3): forwarded to the caller; see method doc.
        unsafe { self.systems.add(idx) }
    }

    /// Returns the completion queue reference.
    ///
    /// # Safety
    /// The pointer must reference a live `ArrayQueue` field of the
    /// owning `Schedule::executor_scratch`. The schedule outlives the
    /// closure (Scope::Drop).
    #[inline]
    unsafe fn completion_queue(&self) -> &crossbeam_queue::ArrayQueue<SystemIndex> {
        // SAFETY: forwarded to the caller; see method doc.
        unsafe { &*self.completion_queue }
    }

    /// Returns the pending-apply atomic reference.
    ///
    /// # Safety
    /// The pointer must reference a live `AtomicUsize` field of the
    /// owning `Schedule::executor_scratch`. The schedule outlives the
    /// closure (Scope::Drop).
    #[inline]
    unsafe fn pending_apply(&self) -> &std::sync::atomic::AtomicUsize {
        // SAFETY: forwarded to the caller; see method doc.
        unsafe { &*self.pending_apply }
    }
}

// SAFETY (SEND1 / SEND3 — Phase 9 §9.2):
//
// `SpawnPointers` carries three raw pointers; each is `!Send` by default.
// We hand-mark `Send` because the bundle escapes into a `scope.spawn`
// closure that runs on a worker thread. The pointees:
//
//   - `systems: *mut SystemBox` — references a slot inside the parent
//     `Schedule::systems` Vec. The Vec lives for the duration of `'scope`
//     (Scope::Drop blocks before the surrounding `&mut Schedule` ends).
//     Multiple workers may hold pointers to *different* indices in the
//     same Vec concurrently; the conflict graph (SCH3) guarantees no two
//     concurrent workers point to the same index.
//
//   - `completion_queue: *const ArrayQueue<SystemIndex>` — references a
//     field of `Schedule::executor_scratch`. Same lifetime argument;
//     ArrayQueue is Send + Sync per crossbeam-queue's documentation.
//
//   - `pending_apply: *const AtomicUsize` — same lifetime argument;
//     AtomicUsize is Send + Sync.
//
// `Sync` is unnecessary — the bundle is captured by value into the
// `move` closure, not shared via reference.
unsafe impl Send for SpawnPointers {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

    use boyko_threadpool::ThreadPoolBuilder;

    use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
    use crate::ecs::core::schedule::schedule_builder::ScheduleBuilder;
    use crate::ecs::core::system::access::Access;
    use crate::ecs::core::system::exclusive_function_system::ExclusiveFunctionSystem;
    use crate::ecs::core::system::system::System;
    use crate::ecs::core::system::system_meta::SystemMeta;
    use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
    use crate::ecs::identifiers::primitives::ResourceId;

    /// Test-only `System` that runs a `Fn() + Send + Sync` body inside
    /// `run_unsafe` and records the call.
    struct ProbeSystem<F: Fn() + Send + Sync + 'static> {
        meta: SystemMeta,
        body: F,
    }

    // SAFETY (S1): `run_unsafe` runs the user closure; the closure does
    //   not touch the cell, so the trait contract is vacuous.
    unsafe impl<F: Fn() + Send + Sync + 'static> System for ProbeSystem<F> {
        type Out = ();
        fn name(&self) -> &'static str {
            self.meta.name()
        }
        fn access(&self) -> &Access {
            self.meta.access()
        }
        fn initialize(&mut self, _world: &mut EcsMaster) {}
        unsafe fn run_unsafe(&mut self, _world: UnsafeEcsCell<'_>) -> Self::Out {
            (self.body)();
        }
        fn meta(&self) -> &SystemMeta {
            &self.meta
        }
        fn set_change_ticks(
            &mut self,
            last_run: crate::ecs::core::change_detection::Tick,
            this_run: crate::ecs::core::change_detection::Tick,
        ) {
            self.meta.last_run = last_run;
            self.meta.this_run = this_run;
        }
    }

    fn fresh_pool() -> Arc<ThreadPool> {
        ThreadPoolBuilder::new().num_threads(2).build()
    }

    fn push_system<F>(
        builder: &mut ScheduleBuilder,
        name: &'static str,
        access: Access,
        body: F,
    ) where
        F: Fn() + Send + Sync + 'static,
    {
        use crate::ecs::core::schedule::system_box::SystemBox;
        use crate::ecs::core::schedule::system_descriptor::SystemDescriptor;
        let mut meta = SystemMeta::for_testing(name);
        meta.access = access;
        let sys = ProbeSystem { meta, body };
        let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
        let system_box = SystemBox::new(boxed);
        builder
            .descriptors
            .push(SystemDescriptor::new(system_box));
    }

    /// Step 12 — a single registered system runs exactly once and returns
    /// cleanly from `Schedule::run`.
    #[test]
    fn single_system_runs_once() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_cl = Arc::clone(&counter);
        push_system(&mut builder, "single", Access::new(), move || {
            counter_cl.fetch_add(1, Ordering::Relaxed);
        });

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);
        schedule.run(&mut world);

        assert_eq!(counter.load(Ordering::Relaxed), 1);
        // Running again starts a fresh frame; SCH6 mandates one body
        // call per frame, so we expect 2 after the second invocation.
        schedule.run(&mut world);
        assert_eq!(counter.load(Ordering::Relaxed), 2);
    }

    /// Step 12 — two independent systems both run in a single frame.
    /// They may run in parallel or serially; the test only asserts the
    /// invariant that each system body fires exactly once per frame.
    #[test]
    fn two_independent_systems_run() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        let counter_a = Arc::new(AtomicUsize::new(0));
        let counter_b = Arc::new(AtomicUsize::new(0));
        let a_cl = Arc::clone(&counter_a);
        let b_cl = Arc::clone(&counter_b);

        // Disjoint resource access — no conflict bit; runs are
        // dispatchable concurrently.
        let mut access_a = Access::new();
        access_a.add_resource_write(ResourceId(0));
        let mut access_b = Access::new();
        access_b.add_resource_write(ResourceId(1));

        push_system(&mut builder, "a", access_a, move || {
            a_cl.fetch_add(1, Ordering::Relaxed);
        });
        push_system(&mut builder, "b", access_b, move || {
            b_cl.fetch_add(1, Ordering::Relaxed);
        });

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);
        schedule.run(&mut world);

        assert_eq!(counter_a.load(Ordering::Relaxed), 1);
        assert_eq!(counter_b.load(Ordering::Relaxed), 1);
    }

    /// Step 12 — two systems writing the same resource share a conflict
    /// bit; the executor must serialize them. We assert non-overlap via
    /// a shared in-flight counter that would witness simultaneity.
    #[test]
    fn conflicting_systems_serialize() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);

        let in_flight = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));

        for name in ["a", "b"] {
            let in_flight_cl = Arc::clone(&in_flight);
            let max_seen_cl = Arc::clone(&max_seen);
            let mut access = Access::new();
            access.add_resource_write(ResourceId(42));
            push_system(&mut builder, name, access, move || {
                let now = in_flight_cl.fetch_add(1, Ordering::AcqRel) + 1;
                // Capture the high-water mark.
                let mut cur = max_seen_cl.load(Ordering::Acquire);
                while now > cur {
                    match max_seen_cl.compare_exchange_weak(
                        cur,
                        now,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => break,
                        Err(v) => cur = v,
                    }
                }
                std::thread::sleep(std::time::Duration::from_micros(100));
                in_flight_cl.fetch_sub(1, Ordering::AcqRel);
            });
        }

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);
        schedule.run(&mut world);

        assert_eq!(
            max_seen.load(Ordering::Relaxed),
            1,
            "conflicting systems must not overlap in time"
        );
    }

    /// Step 13 — an exclusive system runs inline on the dispatcher; the
    /// surrounding non-exclusive systems also run. The ordering must place
    /// the exclusive system between the others (in the topological order
    /// they were registered).
    #[test]
    fn exclusive_system_blocks_others() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);

        let log: Arc<std::sync::Mutex<Vec<u8>>> = Arc::new(std::sync::Mutex::new(Vec::new()));

        // pre-system: writes resource 1
        {
            let log_cl = Arc::clone(&log);
            let mut access = Access::new();
            access.add_resource_write(ResourceId(1));
            push_system(&mut builder, "pre", access, move || {
                log_cl.lock().unwrap().push(1);
            });
        }

        // exclusive system — universal access via ExclusiveFunctionSystem.
        // We register it through the descriptor directly so we don't need
        // the SystemConfig fluent layer.
        {
            use crate::ecs::core::schedule::system_box::SystemBox;
            use crate::ecs::core::schedule::system_descriptor::SystemDescriptor;
            let log_cl = Arc::clone(&log);
            let body = move |_w: &mut EcsMaster| {
                log_cl.lock().unwrap().push(2);
            };
            let mut sys = ExclusiveFunctionSystem::new(body);
            // Pre-initialise so SystemBox::new caches is_exclusive=true.
            let mut world_pre = EcsMaster::new();
            sys.initialize(&mut world_pre);
            let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
            let system_box = SystemBox::new(boxed);
            assert!(
                system_box.is_exclusive,
                "ExclusiveFunctionSystem must populate is_exclusive=true"
            );
            builder
                .descriptors
                .push(SystemDescriptor::new(system_box));
        }

        // post-system: writes resource 1 (same as pre to ensure ordering
        // via `.before` is unnecessary — the exclusive system blocks
        // everyone anyway, and topological order falls back to insertion
        // order for systems that have no DAG edges).
        {
            let log_cl = Arc::clone(&log);
            let mut access = Access::new();
            access.add_resource_write(ResourceId(1));
            push_system(&mut builder, "post", access, move || {
                log_cl.lock().unwrap().push(3);
            });
        }

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);
        schedule.run(&mut world);

        let final_log = log.lock().unwrap().clone();
        // SCH9: order between unordered, non-conflicting systems is
        // unspecified — but pre and post BOTH conflict with the exclusive
        // (universal) one and with each other (resource 1 write/write).
        // Topological tie-break is insertion order (Kahn FIFO), so we
        // expect `[1, 2, 3]`.
        assert_eq!(final_log, vec![1, 2, 3], "expected pre, exclusive, post order");
    }

    /// Step 12 — apply window barrier prevents UB during deferred
    /// `Commands`-style mutations. We don't have Commands wired into the
    /// test harness here, but we approximate the contract: a system body
    /// only writes through the cell (no `&mut world` aliasing); the
    /// `apply` hook must see the body's writes via the Acquire/Release
    /// pair on `pending_apply`. We instrument a system whose `apply`
    /// reads a counter that the body wrote, and assert the read sees the
    /// body's value (1, not 0) every time.
    #[test]
    fn apply_window_sees_body_writes() {
        use std::sync::atomic::AtomicU64;
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);

        let shared = Arc::new(AtomicU64::new(0));
        let apply_observed = Arc::new(AtomicU64::new(0));

        struct ApplyProbe {
            meta: SystemMeta,
            body_target: Arc<AtomicU64>,
            apply_observed: Arc<AtomicU64>,
        }
        // SAFETY (S1): the system only writes a foreign atomic; no world
        //   access through the cell.
        unsafe impl System for ApplyProbe {
            type Out = ();
            fn name(&self) -> &'static str {
                self.meta.name()
            }
            fn access(&self) -> &Access {
                self.meta.access()
            }
            fn initialize(&mut self, _w: &mut EcsMaster) {}
            unsafe fn run_unsafe(&mut self, _w: UnsafeEcsCell<'_>) -> Self::Out {
                self.body_target.store(1, Ordering::Relaxed);
            }
            fn apply(&mut self, _w: &mut EcsMaster) {
                let seen = self.body_target.load(Ordering::Relaxed);
                self.apply_observed.store(seen, Ordering::Relaxed);
            }
            fn meta(&self) -> &SystemMeta {
                &self.meta
            }
            fn set_change_ticks(
                &mut self,
                last_run: crate::ecs::core::change_detection::Tick,
                this_run: crate::ecs::core::change_detection::Tick,
            ) {
                self.meta.last_run = last_run;
                self.meta.this_run = this_run;
            }
        }

        {
            use crate::ecs::core::schedule::system_box::SystemBox;
            use crate::ecs::core::schedule::system_descriptor::SystemDescriptor;
            let sys = ApplyProbe {
                meta: SystemMeta::for_testing("apply_probe"),
                body_target: Arc::clone(&shared),
                apply_observed: Arc::clone(&apply_observed),
            };
            let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
            builder
                .descriptors
                .push(SystemDescriptor::new(SystemBox::new(boxed)));
        }

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);
        schedule.run(&mut world);

        assert_eq!(
            apply_observed.load(Ordering::Relaxed),
            1,
            "apply() must observe the body's write through the Release/Acquire pair"
        );
    }

    // ── Phase 16 — mark_skipped mechanics ────────────────────────────────────

    /// `mark_skipped(i)` marks system `i` completed AND decrements every ordered
    /// successor's `pred_remaining` — IDENTICAL to the apply-window completion
    /// tail minus the body/apply/queue. Construct a 2-system DAG `a → b`, seed
    /// the scratch for a frame, skip `a`, and assert `completed[a]` is set and
    /// `pred_remaining[b]` dropped from 1 to 0. (Plan §10
    /// `mark_skipped_decrements_successors`.)
    #[test]
    fn mark_skipped_marks_completed_and_decrements_successor() {
        use crate::ecs::core::schedule::ordering::{OrderingEdge, SystemKey};

        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        let a_idx = 0usize;
        let b_idx = 1usize;
        push_system(&mut builder, "a", Access::new(), || {});
        push_system(&mut builder, "b", Access::new(), || {});
        // a -> b: stored on a's descriptor.
        builder.descriptors[a_idx]
            .ordering_hints
            .push(OrderingEdge::Before(SystemKey(a_idx), SystemKey(b_idx)));

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        // Topo order is a, b (the edge forces it). Seed the per-frame scratch:
        // pred_remaining[b] starts at 1 (one predecessor, a).
        schedule
            .executor_scratch
            .reset_for_frame(&schedule.conflict_graph);
        assert_eq!(
            schedule.executor_scratch.pred_remaining[1], 1,
            "precondition: b has one predecessor (a)"
        );

        schedule.mark_skipped(0);

        assert!(
            schedule.executor_scratch.completed.contains(0),
            "mark_skipped sets completed[a]"
        );
        assert_eq!(
            schedule.executor_scratch.pred_remaining[1], 0,
            "mark_skipped decrements b's pred_remaining (1 -> 0)"
        );
        // The skip must NOT touch pending_apply / completion_queue (it is
        // invisible to the apply-window accounting).
        assert_eq!(
            schedule
                .executor_scratch
                .pending_apply
                .load(Ordering::Relaxed),
            0,
            "a skip never bumps pending_apply"
        );
        assert!(
            schedule.executor_scratch.completion_queue.is_empty(),
            "a skip never pushes a completion"
        );
    }

    /// A conditionless schedule has an all-zero `has_condition` bitset (the
    /// 0%-gate), so `executor_main_loop`'s Step 1.5 branch is never taken. Unit
    /// confirmation at the `Schedule` level (complements the builder-level
    /// `has_condition_clear_when_no_run_if`).
    #[test]
    fn has_condition_clear_for_plain_schedule() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        push_system(&mut builder, "a", Access::new(), || {});
        push_system(&mut builder, "b", Access::new(), || {});

        let mut world = EcsMaster::new();
        let schedule = builder.build(&mut world);
        assert!(
            schedule.has_condition.is_clear(),
            "no .run_if ⇒ has_condition all-zero ⇒ Step 1.5 predicted-not-taken"
        );
    }

    // ── Phase 16.1 (B-1) — run-condition tick threading ──────────────────────

    /// A `BoolSystem` condition that records its `SystemMeta` tick snapshot on
    /// every `run_unsafe` call. Used to prove `Schedule::run` now bumps a
    /// condition's `(last_run, this_run)` per frame (B-1 fix) instead of leaving
    /// them frozen at the `initialize` sentinel.
    struct TickRecordingCondition {
        meta: SystemMeta,
        last_run_seen: Arc<AtomicU32>,
        this_run_seen: Arc<AtomicU32>,
    }

    // SAFETY (S1): `run_unsafe` reads only this system's own meta and writes
    //   foreign atomics; it never touches the world cell, so the aliasing
    //   contract is vacuous.
    unsafe impl System for TickRecordingCondition {
        type Out = bool;
        fn name(&self) -> &'static str {
            self.meta.name()
        }
        fn access(&self) -> &Access {
            self.meta.access()
        }
        fn initialize(&mut self, _w: &mut EcsMaster) {}
        unsafe fn run_unsafe(&mut self, _w: UnsafeEcsCell<'_>) -> bool {
            self.last_run_seen
                .store(self.meta.last_run().get(), Ordering::Relaxed);
            self.this_run_seen
                .store(self.meta.this_run().get(), Ordering::Relaxed);
            true // keep the gated system ready so the body dispatches
        }
        fn meta(&self) -> &SystemMeta {
            &self.meta
        }
        fn set_change_ticks(
            &mut self,
            last_run: crate::ecs::core::change_detection::Tick,
            this_run: crate::ecs::core::change_detection::Tick,
        ) {
            self.meta.last_run = last_run;
            self.meta.this_run = this_run;
        }
    }

    /// **B-1 regression.** Before the fix, `run_condition` ran a condition
    /// without `set_change_ticks`, so its meta ticks stayed pinned to the
    /// `initialize` sentinel forever — a `Changed<T>`/`Added<T>`/`Ref<T>`
    /// condition then read every row as changed (silently always-true).
    ///
    /// This locks the mechanism that fixes that: `Schedule::run` bumps every
    /// condition's tick snapshot per frame with the SAME `this_run` as the
    /// systems. We attach a tick-recording condition and assert that across
    /// consecutive frames its observed `this_run` ADVANCES (and equals the
    /// world's current tick), and that `last_run` tracks the previous frame's
    /// `this_run`. If the per-condition bump loop is removed, `this_run` stays
    /// frozen at the sentinel and this test fails.
    #[test]
    fn run_condition_ticks_advance_per_frame() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);

        // One ordinary system carrying one tick-recording condition.
        let last_seen = Arc::new(AtomicU32::new(0));
        let this_seen = Arc::new(AtomicU32::new(0));
        push_system(&mut builder, "gated", Access::new(), || {});
        {
            let cond = TickRecordingCondition {
                meta: SystemMeta::for_testing("tick_probe_condition"),
                last_run_seen: Arc::clone(&last_seen),
                this_run_seen: Arc::clone(&this_seen),
            };
            let boxed: Box<dyn System<Out = bool>> = Box::new(cond);
            builder.descriptors[0].conditions.push(boxed);
        }

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        // Frame 1: `this_run` is the tick bumped at the top of `run`. It must
        // equal the world's post-bump current tick and be non-zero.
        schedule.run(&mut world);
        let this_f1 = this_seen.load(Ordering::Relaxed);
        assert_eq!(
            this_f1,
            world.current_tick().get(),
            "frame-1 condition this_run must equal the world's bumped current tick"
        );
        assert_ne!(this_f1, 0, "frame-1 condition this_run must be non-zero (was bumped)");

        // Frame 2: `this_run` advances; `last_run` becomes the previous frame's
        // `this_run` — the SAME contract a system body's ticks follow. A frozen
        // (always-true) condition would report an unchanged `this_run` here.
        schedule.run(&mut world);
        let this_f2 = this_seen.load(Ordering::Relaxed);
        let last_f2 = last_seen.load(Ordering::Relaxed);
        assert!(
            this_f2 > this_f1,
            "condition this_run must advance frame-to-frame ({this_f1} -> {this_f2})"
        );
        assert_eq!(
            last_f2, this_f1,
            "condition last_run on frame 2 must equal frame 1's this_run \
             (the per-frame (last_run, this_run] window)"
        );
    }
}
