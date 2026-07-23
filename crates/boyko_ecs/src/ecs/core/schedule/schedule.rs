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

use std::mem;
use std::sync::Arc;
use std::sync::atomic::Ordering;
#[cfg(not(miri))]
use std::time::Duration;

use boyko_threadpool::{Scope, ThreadPool};
use fixedbitset::FixedBitSet;

use crate::ecs::core::change_detection::{Tick, run_check_ticks_scan};
use crate::ecs::core::component::hooks::scope::DeferredScopeGuard;
use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::schedule::bitset_intersects::bitset_intersects;
use crate::ecs::core::schedule::conflict_graph::{ConflictGraph, SystemIndex};
use crate::ecs::core::schedule::executor_scratch::{CompletionCell, ExecutorScratch};
use crate::ecs::core::schedule::system_box::{BoolSystem, SystemBox};
use crate::ecs::core::schedule::system_set::SystemSetId;
use crate::ecs::core::state::StateEntry;
use crate::ecs::core::system::dispatcher_token::DispatcherToken;
use crate::ecs::core::system::gpu_intent::{GpuAccessIntent, GpuStage};
use crate::ecs::core::system::system_kind::SystemKind;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;
use crate::ecs::identifiers::primitives::WorldId;

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
/// `executor_scratch` is the hottest field — `running`, `completed`, and
/// `pred_remaining` all sit in the dispatcher's L1 for the duration of a
/// frame (the cross-thread `completion` channel is now out-of-line behind a
/// `NonNull`, Phase 9.3c, so it no longer shares those lines).
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
    /// The last pointer-bearing field. Every pre-existing field keeps its exact
    /// offset, so the cross-thread hot prefix
    /// (`pool → systems → conflict_graph → executor_scratch → has_condition`)
    /// documented above is byte-for-byte unchanged.
    pub(crate) state_entries: Vec<StateEntry>,

    // ── Phase 16.1 — tick-aware run conditions (W2) ──────────────────────────
    /// Frame-start `this_run`, set ONCE at the top of [`Schedule::run`] (= the
    /// first per-frame `bump_change_tick`). Single source of truth for the
    /// gated-system dispatch stamp (C1) and the condition eval-site checkpoint
    /// (Gap #1) — both must use the SAME value the every-frame systems were
    /// stamped with, so a reached-this-frame gated system/condition observes
    /// the identical `(last_run, this_run]` window (#56 coupling:
    /// `frame_this_run == current_tick() - 1` after the apply-window bump).
    ///
    /// NOT `world.current_tick()` at use time — that reads `this_run + 1` once
    /// the #56 apply-window bump has fired. Appended as the **LAST** field
    /// (M3): a pointer-free trailing scalar, so it changes no pre-existing
    /// offset and stays out of the cross-thread pointer prefix above.
    pub(crate) frame_this_run: Tick,

    // ── Phase 21 — world binding (H2) ────────────────────────────────────────
    /// The [`WorldId`] of the world this schedule was built on, recorded at
    /// [`ScheduleBuilder::try_build`]. [`Schedule::run`] release-panics on a
    /// mismatch (Bevy parity): per-world caches held by the systems
    /// (`EventReaderState`'s `NonNull<EventBuffer<E>>`, `QueryState`'s
    /// generation snapshots) are valid ONLY against the build world, so a
    /// cross-world `run` is a latent use-after-free / aliasing surface —
    /// closed loudly at the single entry point. Appended after
    /// `frame_this_run` (same M3 discipline): a pointer-free trailing scalar,
    /// no pre-existing offset changes.
    ///
    /// [`ScheduleBuilder::try_build`]: super::schedule_builder::ScheduleBuilder::try_build
    pub(crate) world_id: WorldId,
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
    /// # World binding (Phase 21 H2)
    ///
    /// A `Schedule` is bound to the world it was built on
    /// ([`ScheduleBuilder::build`] records the world's [`WorldId`]). Passing
    /// any other world panics in release builds — the systems' per-world
    /// caches (event-buffer pointers, query-state generations) are only valid
    /// against the build world.
    ///
    /// # Caller obligation (NSND-THREAD — Phase 5 Option C)
    ///
    /// If the world holds any `!Send` resource, `run` MUST be called on that
    /// slab's owning thread (the thread of the first
    /// [`EcsMaster::insert_non_send_resource`]). A `GpuCompute` / `CpuExclusive`
    /// system reaches a `!Send` resource on the dispatcher-solo path (the
    /// `DispatcherToken` / NonSend `SystemParam` projections), which is sound
    /// ONLY when the dispatcher thread is that owning thread. Workers never touch
    /// the `!Send` slab (the token/cell-accessor surface is dispatcher-only). The
    /// M2 debug tripwire catches a wrong-thread `run`.
    ///
    /// [`EcsMaster::insert_non_send_resource`]: crate::ecs::core::ecs_master::ecs_master::EcsMaster::insert_non_send_resource
    ///
    /// # Panics
    ///
    /// * `boyko-B9101` if `world` is not the world this schedule was built on
    ///   (Phase 21 world-binding gate).
    /// * Re-raises the first panic observed by any worker (TPN9 / SCH11)
    ///   on the dispatcher thread, surfaced through `Scope::Drop`.
    /// * `debug_assert!`s the SCH15 equality `(SystemBox::kind ==
    ///   CpuExclusive) == access().is_universal()` for every CPU system
    ///   (plan §13.6 SCH15 / Phase 4 CR-B); `GpuCompute` is the marker-set
    ///   carve-out excluded from the equality.
    ///
    /// [`Scope::spawn`]: boyko_threadpool::Scope::spawn
    /// [`ScheduleBuilder::build`]: super::schedule_builder::ScheduleBuilder::build
    pub fn run(&mut self, world: &mut EcsMaster) {
        // Phase 21 (H2) — world-binding gate. One u64 compare per run,
        // predicted-not-taken; the panic body is out-of-line (#[cold]).
        // RELEASE-level (Bevy parity): a cross-world run would dereference
        // per-world cached pointers (EventReaderState's NonNull, QueryState
        // generations) against the wrong world — a UAF surface, so it must
        // fail loudly, not just in debug. This single compare amends the
        // P20-B1(a) byte-identity gate; re-verified within ±2% on the
        // 50-system bench (see PHASE-21-RESULTS.md).
        if world.world_id() != self.world_id {
            schedule_world_mismatch_panic(self.world_id, world.world_id());
        }

        // SCH15 (Round 2 C9 / OQ-4 / Phase 4 CR-B) — confirm the build-time
        // `kind` cache still matches the system's declared access. A future
        // refactor that mutates `Access` after build would desync; catching
        // it here is load-bearing for the dispatcher-only gate inside the
        // loop.
        //
        // CR-B form: the invariant stays an EQUALITY on the CpuExclusive
        // axis — `(kind == CpuExclusive) <==> access().is_universal()` — for
        // every CPU system. `GpuCompute` is the one marker-set carve-out
        // that carries NO access constraint (Phase-5-scheduled), so it is
        // excluded from the equality (a GpuCompute system may declare any
        // access). Excluding it is sound because the resolution at
        // `ScheduleBuilder::build` sets `GpuCompute` BEFORE the universal
        // check, so a `GpuCompute` `kind` can never be a desynced
        // `CpuExclusive`.
        debug_assert!(
            self.systems.iter().all(|sb| {
                use crate::ecs::core::system::system_kind::SystemKind;
                if sb.kind == SystemKind::GpuCompute {
                    // Marker carve-out: no access constraint.
                    true
                } else {
                    (sb.kind == SystemKind::CpuExclusive)
                        == sb.system.access().is_universal()
                }
            }),
            "invariant SCH15: SystemBox::kind desynced from access().is_universal() \
             on the CpuExclusive axis"
        );

        self.executor_scratch.reset_for_frame(&self.conflict_graph);

        // Phase 10 Wave D Step 13 — frame-start change-detection tick bump
        // (plan §4.5 / PHASE9.1). The FIRST of ~2 bumps per `Schedule::run`:
        // this one publishes the new `this_run` to every system, condition,
        // and the state pass below; the SECOND (Bug #56) fires at the
        // apply-window, after this `this_run` was captured and before any
        // deferred command drains, so deferred-added components land at
        // `this_run + 1` and are observed by Added<T>/Changed<T> exactly once
        // on the next RUN OF THIS SCHEDULE (Phase 20: under a multi-schedule
        // App each `Schedule::run` carries its own window, so "frame" is the
        // wrong unit). Both are `fetch_add(Relaxed)`; only this first value
        // is read here.
        let this_run = world.bump_change_tick();

        // Phase 16.1 (W2): publish the frame-start `this_run` as the single
        // source of truth for the gated-system dispatch stamp (C1) and the
        // condition eval-site checkpoint (Gap #1). Both stamp sites read this
        // field, never `world.current_tick()` (which becomes `this_run + 1`
        // after the #56 apply-window bump below).
        self.frame_this_run = this_run;

        // Phase 10 Wave D Step 13 — conditional wraparound clamp scan
        // (plan §2.7 WRAP1-WRAP2). `should_run_check_ticks` fires roughly
        // every `CHECK_TICK_THRESHOLD` frames ≈ ~100 days at 60 FPS; the
        // hot-path cost is a single u32 compare per `Schedule::run`.
        if world.should_run_check_ticks() {
            run_check_ticks_scan(world);
            // Phase 16.1 (Gap #2): the per-row pool scan above does NOT touch
            // any system's or condition's `last_run`/`this_run`. Once Phase
            // 16.1 advances those ticks only on a frame the system/condition
            // runs (C1 + Gap #1), a dormant span > `MAX_CHANGE_AGE` could flip
            // `Tick::is_newer_than`; clamp them here under the SAME `this_run`
            // gate (no drift vs `set_last_check_tick`).
            self.check_change_ticks(this_run);
            world.set_last_check_tick(this_run);
        }

        // Phase 10 Wave D Step 13 / Phase 16.1 C1 — per-system tick snapshot
        // dispatch (plan §2.6 SCT4 / PHASE9.2). Each system's PREVIOUS
        // `this_run` becomes its new `last_run`; its new `this_run` is the
        // dispatcher-wide value just bumped. `System::set_change_ticks` has no
        // default body so every impl must declare it (plan §5.4-bis).
        //
        // Phase 16.1 C1: ONLY systems with `has_condition[i]` CLEAR are stamped
        // here. They run every frame, so "advance every frame" ≡ "advance when
        // run" — byte-identical to the old unconditional loop on the plain
        // (no-`.run_if`) path, where `has_condition` is all-zero and the
        // `contains(i)` branch is uniformly not-taken (THE 0%-gate, W1). A
        // GATED system (`has_condition[i]` set) is stamped instead at its
        // DISPATCH site (`try_dispatch_ready` / the inline-exclusive path),
        // immediately before it runs, so a frame it is SKIPPED leaves its ticks
        // FROZEN — on resume its `Changed<T>` body queries then observe the
        // full dormant window (C1) instead of an empty one.
        //
        // The write happens here (before the empty-schedule short-circuit and
        // before the executor loop) so that workers spawned later in this same
        // frame observe consistent tick state through `&SystemMeta` captured by
        // Query / SystemChangeTick; the dispatcher's sequential write
        // happens-before every worker spawn (plan §8.2). The gated-system
        // dispatch stamp preserves that edge: it is a dispatcher-sequential
        // `&mut` write sequenced before that system's own `scope.spawn`.
        for (i, sys_box) in self.systems.iter_mut().enumerate() {
            if self.has_condition.contains(i) {
                continue;
            }
            let prev_this_run = sys_box.system.meta().this_run();
            sys_box.system.set_change_ticks(prev_this_run, this_run);
        }

        // Phase 16.1 (Gap #1) — condition ticks are NOT bumped at frame start.
        // A condition is an ordinary `System<Out = bool>` with its own
        // `SystemMeta` ticks; `run_condition` (`ecs_master.rs`) now advances
        // `(last_run, this_run]` itself, but ONLY on a frame the condition is
        // actually evaluated (Bevy "since-last-actual-run" parity). A condition
        // dormant for N frames (gated by a false set/state condition or by a
        // `pred_remaining`-blocked member) therefore resumes observing ALL
        // changes since its last actual run, not just since the last frame — so
        // it no longer silently misses dormant changes. For a condition
        // evaluated every frame the window is identical to the old frame-start
        // bump (`prev` == last frame's `this_run`), so the every-frame case is
        // behavior-preserving.

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

        // Bug #56: apply-window change-tick bump. Deferred command applies
        // (SpawnAt/Insert/migration/spawn_batch) stamp added/changed by reading
        // current_tick() at apply time; bumping once here — after systems',
        // conditions', and the state pass's frame-start `this_run` were captured,
        // before any deferred drain — lands those stamps at `this_run + 1`,
        // strictly between this run's reader window and the next's, so
        // Added<T>/Changed<T> observe a deferred-added component exactly once on
        // the following run of this schedule (Bevy's ApplyDeferred-sync-point
        // analogue; Phase 20 wording — "frame" ⇒ "run of that schedule"). The
        // result is intentionally discarded: only the side effect on change_tick
        // matters; systems'/conditions'/state ticks remain pinned to `this_run`.
        let _apply_window_tick = world.bump_change_tick();
        debug_assert_eq!(
            world.current_tick().get(),
            this_run.get().wrapping_add(1),
            "Bug#56: apply-window tick must be exactly one past the frame-start this_run \
             (exactly one extra bump per frame)"
        );

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
            // SAFETY (Phase 9.3c): post-`install`, every worker has joined via
            //   `Scope::Drop`; this single-threaded read mints a transient cell
            //   over the owning `NonNull` (non-retagging `as_ptr` lineage; no
            //   `Box` place named). Kept INSIDE `debug_assert_eq!` so it elides
            //   in release (no extra load on the frame-end path).
            unsafe { CompletionCell::new(self.executor_scratch.completion) }
                .pending_load(Ordering::Relaxed),
            0,
            "invariant SCH6: pending_apply must be 0 at end of frame"
        );
    }

    /// Phase 16.1 (Gap #2) — clamp every system's and condition's tick snapshot
    /// against `current`, mirroring the per-row pool scan in
    /// [`run_check_ticks_scan`].
    ///
    /// Bevy's `Schedule::check_change_ticks` clamps `systems` +
    /// `system_conditions` + `set_conditions`; before Phase 16.1 the
    /// unconditional frame-start bumps refreshed every tick each frame, masking
    /// the hole, but once C1 + Gap #1 advance a gated system's / a dormant
    /// condition's `last_run` only on a frame it runs, an un-refreshed
    /// `last_run` could drift past `MAX_CHANGE_AGE` and flip
    /// [`Tick::is_newer_than`]. This pulls each one back to the oldest
    /// still-valid tick via [`System::check_change_tick`].
    ///
    /// Cold: called only inside the `should_run_check_ticks` block (≈ every
    /// `CHECK_TICK_THRESHOLD` frames), right after `run_check_ticks_scan` and
    /// before `pool.install`, so the dispatcher's `&mut self` is exclusive — no
    /// worker is live, no cell, no `unsafe`.
    ///
    /// [`run_check_ticks_scan`]: crate::ecs::core::change_detection::run_check_ticks_scan
    /// [`Tick::is_newer_than`]: crate::ecs::core::change_detection::Tick::is_newer_than
    /// [`System::check_change_tick`]: crate::ecs::core::system::system::System::check_change_tick
    ///
    /// Phase 20 D8: `pub(crate)` so `App::check_ticks_all_schedules` (the
    /// margin-aware all-schedule pass) can clamp BOTH schedules' system /
    /// condition ticks from the frame driver. The body is unchanged; the
    /// internal call site above stays as the standalone-single-schedule belt.
    #[cold]
    #[inline(never)]
    pub(crate) fn check_change_ticks(&mut self, current: Tick) {
        for sys_box in self.systems.iter_mut() {
            sys_box.system.check_change_tick(current);
        }
        for own_conds in self.system_conditions.iter_mut() {
            for cond in own_conds.iter_mut() {
                cond.check_change_tick(current);
            }
        }
        for entry in self.set_conditions.iter_mut() {
            entry.condition.check_change_tick(current);
        }
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
    /// task completes, so the captures held by the spawned closures (the
    /// `systems` raw pointer into the `Schedule::systems` Vec heap buffer and
    /// the `completion` [`CompletionCell`] over the channel's own heap
    /// allocation) remain valid for the closure's entire body.
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

        // Phase 9.3c: mint the completion cell ONCE from the owning `NonNull`
        // (a Copy read of the pointer — it does NOT borrow `self`, so it
        // survives across the `&mut self` dispatch calls below). All completion
        // access — dispatcher AND workers — flows through this cell's single
        // non-retagging `NonNull::as_ptr` lineage; the heap channel's pointee is
        // never reborrowed as a `&CompletionChannel` under `&mut self` again
        // this frame (which would re-protect the heap and re-introduce the bug).
        //
        // SAFETY: the channel is owned by `self.executor_scratch`, which is
        //   borrowed `&mut` for this whole call, so the pointee outlives the
        //   cell's lifetime (bounded by the spawns, which `Scope::Drop` joins
        //   before this frame returns).
        let completion = unsafe { CompletionCell::new(self.executor_scratch.completion) };

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
            let pending = completion.pending_load(Ordering::Acquire);
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
                self.apply_window_drain(world_mut, completion);
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
            let dispatched = self.try_dispatch_ready(scope, cell, completion);

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
    fn apply_window_drain(&mut self, world: &mut EcsMaster, completion: CompletionCell<'_>) {
        let target = completion.pending_load(Ordering::Acquire);

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
            let idx = match completion.pop() {
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
        completion.pending_fetch_sub(target, Ordering::Relaxed);
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

        // Phase 16.1 (Gap #1): snapshot the frame-start tick into a `Copy`
        // local so passing it to `run_condition` does not borrow `self` while
        // the `&mut self.system_conditions[i][k]` condition borrow is live.
        let this_run = self.frame_this_run;

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
                let r = world.run_condition(cond, this_run);
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
        // Phase 16.1 (Gap #1): snapshot before the `&mut self.set_conditions[k]
        // .condition` borrow below (same borrow-conflict avoidance as
        // `evaluate_ready_conditions`).
        let this_run = self.frame_this_run;
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
                let v = world.run_condition(cond, this_run);
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
    /// 3. For concurrent systems, lifting the `systems` raw pointer
    ///    (`*mut SystemBox`) from `&mut self` *before* the closure capture and
    ///    pairing it with the `completion` [`CompletionCell`] (minted once in
    ///    `executor_main_loop`) inside a small `SpawnPointers` `Copy` struct
    ///    (`unsafe impl Send`). Both remain valid for the entire `'scope`
    ///    window because:
    ///      - `self.systems` is a `Vec<SystemBox>` whose heap buffer is a
    ///        SEPARATE allocation (outside the `Schedule` allocation, so no
    ///        `&mut self` protector covers it); the buffer does not move across
    ///        frames; only the inner `Box<dyn System>` is consumed by
    ///        `run_unsafe(&mut self, ...)`.
    ///      - The `completion` cell points at the `CompletionChannel`'s OWN
    ///        heap allocation (Phase 9.3c), reached only through the cell's
    ///        non-retagging `NonNull::as_ptr` lineage — never through a
    ///        `&mut self` reborrow. The channel is owned by
    ///        `self.executor_scratch`, which outlives the `'scope` borrow
    ///        (Scope::Drop blocks until the spawn completes; we are still inside
    ///        `executor_main_loop`, which holds `&'scope mut self`).
    ///      - The `ConflictGraph::conflict_bits` already-disjoint-by-graph
    ///        invariant (SCH3) guarantees no two concurrently-dispatched
    ///        systems alias the same `systems` slot — each points to a distinct
    ///        index in `self.systems` and the conflict bits prevent two
    ///        parallel systems from sharing any worker resource that would
    ///        alias.
    ///
    /// The pointer escape is documented in the spawn's SAFETY block.
    fn try_dispatch_ready<'scope>(
        &mut self,
        scope: &Scope<'scope>,
        cell: UnsafeEcsCell<'scope>,
        completion: CompletionCell<'scope>,
    ) -> usize {
        let n = self.systems.len();

        // Reusable per-round scratch (preallocated once in `ExecutorScratch::new`,
        // capacity ≤ system_count). `mem::take` lends the buffers out so the loop
        // can fill them while still borrowing `self.executor_scratch.running`
        // (split borrow); each is restored (drained, allocation intact) before
        // every return. This replaces the previous two `Vec::new()` per dispatch
        // round — the executor's only hot-path allocation.
        //
        // Two buckets so dispatcher-only systems can short-circuit when the
        // LIVE running set is non-empty (FIX-3 — the snapshot would be stale by
        // the time a later index is examined).
        let mut exclusive_to_run = mem::take(&mut self.executor_scratch.exclusive_to_run);
        let mut to_spawn = mem::take(&mut self.executor_scratch.to_spawn);
        debug_assert!(
            exclusive_to_run.is_empty() && to_spawn.is_empty(),
            "dispatch scratch must be drained and restored by the previous round",
        );

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

            if self.systems[i].kind.runs_on_dispatcher() {
                // EXC2 (Phase 4 FIX-3 / SCH15-C1): dispatcher-only systems
                // (CpuExclusive AND Phase-4 GpuCompute) require `running == 0` at
                // dispatch time. Gate on the LIVE running set, NOT the
                // `running_count` snapshot captured at function entry: concurrent
                // systems are inserted into `running` earlier in THIS loop
                // (below), so the snapshot can be stale and would otherwise let a
                // dispatcher-only system be co-dispatched with a concurrent one
                // in the same round. For `CpuExclusive` the conflict graph also
                // serializes (universal access ⇒ `bitset_intersects` above already
                // rejects), so this is belt-and-suspenders; for `GpuCompute`
                // (non-universal, marker-only) the conflict graph CANNOT enforce
                // solo-ness, so the live check is the ONLY guard. (GpuCompute is
                // not creatable end-to-end in Phase 4 — no builder API; the
                // end-to-end GpuCompute scheduling test is a Phase-5 obligation,
                // finding X5.) If anything is running we defer to the next round
                // (the apply window drains, and the next call reconsiders).
                if self.executor_scratch.running.count_ones(..) > 0 {
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
            // FIX-3 / SCH15: a dispatcher-only system is accepted into
            // `exclusive_to_run` ONLY when the LIVE running set is empty (the
            // EXC2 gate above), and the `break` after the push guarantees no
            // concurrent system was inserted into `running` afterwards in this
            // call. So at this point the running set still holds nothing — the
            // dispatcher-solo invariant the inline `run_unsafe` below relies on.
            debug_assert!(
                self.executor_scratch.running.count_ones(..) == 0,
                "EXC2/FIX-3: a dispatcher-only system must dispatch solo (running == 0)",
            );
            self.executor_scratch.running.insert(i);

            // Phase 16.1 C1 — gated-system dispatch stamp (inline-exclusive
            // path). A system with `has_condition[i]` set was NOT stamped at
            // frame start (its ticks stay frozen across skipped frames); stamp
            // it here, immediately before `run_unsafe`, so on a frame it runs
            // its body queries observe the full dormant `(last_run, this_run]`
            // window. Same thread, program order ⇒ the stamp is visible to the
            // run below. The `has_condition[i]`-clear case was already stamped
            // at frame start, so skip it (no double-advance).
            if self.has_condition.contains(i) {
                let prev = self.systems[i].system.meta().this_run();
                self.systems[i]
                    .system
                    .set_change_ticks(prev, self.frame_this_run);
            }

            // SAFETY (EXC1 — Round 3 W-NEW-5; FIX-3; Phase 5 Option C):
            //   - For `CpuExclusive`, universal access ⇒ the conflict graph
            //     forces `running == 0` before dispatch. Independently, the EXC2
            //     gate checks the LIVE `running.count_ones()` (NOT a stale
            //     snapshot) before pushing to `exclusive_to_run`, and the `break`
            //     after the push means nothing was spawned in this call — the
            //     `debug_assert!(running == 0)` at the top of this loop pins it.
            //     For `GpuCompute` (marker-only, non-universal) the live EXC2
            //     check is the sole solo-ness guard.
            //   - `cell.world_mut()` reborrows `&mut EcsMaster` from the
            //     cell minted in the current dispatch round from the
            //     dispatcher's own &mut world. No other reference exists.
            //   - We mint a `DispatcherToken` from that SAME reborrow and call
            //     `run_dispatcher` (not `run_unsafe`). The token is the Option-C
            //     capability for reaching `!Send` resources; minting it here is
            //     sound because the EXC2 gate above guarantees `running == 0`
            //     (no worker holds an aliasing cell — the token's `new`
            //     contract). The default `run_dispatcher` forwards to
            //     `run_unsafe` via the cell, so every CPU system is
            //     byte-identical; only a `GpuCompute` system overrides it.
            //   - The token + the `&mut EcsMaster` reborrow it carries do not
            //     escape: `run_dispatcher` consumes the token by value, and the
            //     dispatcher reborrows via the same cell for `apply` only after
            //     the token's borrow has ended. A stashed pointer would alias
            //     the apply reborrow — the system body must not retain one.
            //   - Calling `run_dispatcher(token)` is safe under S1' because no
            //     worker is running.
            {
                // SAFETY (Option C / S1'): EXC2 guarantees `running == 0`, so the
                //   dispatcher is solo — exactly the context `DispatcherToken::new`
                //   requires. The reborrow's `'scope` lifetime outlives the call.
                let world_ref: &mut EcsMaster = unsafe { cell.world_mut() };
                let token = unsafe { DispatcherToken::new(world_ref) };
                unsafe {
                    self.systems[i].system.run_dispatcher(token);
                }
            }
            // Reborrow for `apply` AFTER the token's borrow has ended (it was
            // consumed by `run_dispatcher` above). The cell carries write
            // capability; this is the same `&mut world` provenance, no aliasing.
            //
            // SAFETY (U_C3, S1): `running == 0` (EXC2), so no worker aliases the
            //   reborrow; the cell was minted via `new_mutable` from the
            //   dispatcher's own `&mut world`. The prior token's borrow ended
            //   when `run_dispatcher` consumed it, so this reborrow is the sole
            //   live `&mut EcsMaster`.
            let world_ref: &mut EcsMaster = unsafe { cell.world_mut() };
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
            // Restore both scratch buffers (drained, allocation reused) before
            // returning so the next round `mem::take`s them empty.
            exclusive_to_run.clear();
            self.executor_scratch.exclusive_to_run = exclusive_to_run;
            self.executor_scratch.to_spawn = to_spawn;
            return dispatched;
        }
        // Past this point `exclusive_to_run` is empty; restore it now so the
        // remaining exits only need to hand `to_spawn` back.
        self.executor_scratch.exclusive_to_run = exclusive_to_run;

        // === Concurrent path (Step 12). ===
        //
        // Each closure captures the `Copy` `SpawnPointers` by value. Its
        // captures remain valid for the `'scope` borrow because:
        //   * `self.systems` is `Vec<SystemBox>` — its heap buffer (a SEPARATE
        //     allocation, outside the `Schedule` allocation) is address-stable
        //     across frames and no system is added/removed mid-run (SCH1).
        //   * `completion` is a `CompletionCell` over the `CompletionChannel`'s
        //     OWN heap allocation (Phase 9.3c), owned by `self.executor_scratch`
        //     for the `&'scope mut` duration of `executor_main_loop`; spawned
        //     closures end before that borrow ends (`Scope::Drop` blocks until
        //     all spawns complete).
        if to_spawn.is_empty() {
            self.executor_scratch.to_spawn = to_spawn;
            return dispatched;
        }

        // Phase 16.1 C1 — gated-system dispatch stamp (concurrent path),
        // OQ-R2-1 resolved to the PRE-PASS form. Stamp every gated index in
        // `to_spawn` BEFORE minting `systems_ptr` below: a system with
        // `has_condition[i]` set was not stamped at frame start (its ticks stay
        // frozen across skipped frames), so on a frame it runs we advance its
        // `(last_run, this_run]` window here, immediately before dispatch.
        //
        // Why a separate pre-pass and not inside the spawn loop: a fresh
        // `&mut self.systems[i]` taken AFTER `systems_ptr = self.systems
        // .as_mut_ptr()` (and while earlier-iteration workers already hold that
        // raw pointer) would, under Tree Borrows, invalidate `systems_ptr`'s
        // provenance — a worker's later `*system_slot` deref would then be
        // use-after-invalidation UB. Hoisting all `&mut` stamps before the raw
        // lift keeps every stamp sequenced before the pointer is created and
        // before any `scope.spawn`, so the happens-before edge (dispatcher
        // stamp → that system's worker Fetch) and the 0%-gate are unchanged.
        // Pure ordering: same writes, same values, no new aliasing.
        for idx in &to_spawn {
            let i = idx.0 as usize;
            if self.has_condition.contains(i) {
                let prev = self.systems[i].system.meta().this_run();
                self.systems[i]
                    .system
                    .set_change_ticks(prev, self.frame_this_run);
            }
        }

        // Phase 9.3c: only `systems` is lifted as a raw pointer here — it
        // targets the `Schedule::systems` Vec's SEPARATE heap buffer (already
        // outside the `Schedule` allocation, so no `&mut self` protector covers
        // it). The completion-channel pointers are GONE: workers now reach the
        // queue/atomic through the `completion` `CompletionCell` (its own heap
        // allocation), captured by value into each spawn below.
        let systems_ptr: *mut SystemBox = self.systems.as_mut_ptr();

        // Drain (not consume-by-value) so the buffer's heap allocation survives
        // to be restored below for reuse next round.
        for idx in to_spawn.drain(..) {
            let ptrs = SpawnPointers {
                systems: systems_ptr,
                completion,
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
            //   - `completion` is a `CompletionCell` over the channel's OWN
            //     heap allocation (Phase 9.3c); the dispatcher does not drop or
            //     move it while `'scope` is alive (Scope::Drop holds the
            //     dispatcher), and every access is through the cell's
            //     non-retagging `NonNull::as_ptr` lineage — so the worker push
            //     is no longer a foreign write under the `&mut self` protector.
            //   - Cell ↑Send (SEND3); SystemBox ↑Send via Box<dyn System
            //     + Send + Sync + 'static>; `CompletionCell` ↑Send.
            scope.spawn(move || {
                // Allocation discipline (ALLOC1 / ALLOC6): set the TLS
                // flag so allocation-restricted paths (event send/read,
                // Time access) can debug_assert their context.
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

                // Phase 9.3c: publish completion through the `CompletionCell`.
                // `push` / `pending_fetch_add` are SAFE interior-mutable `&self`
                // ops; the only `unsafe` is inside `CompletionCell::channel`,
                // discharged by the cell's own contract (heap allocation
                // OUTSIDE the `Schedule` allocation, so no `&mut self` protector
                // covers it; non-retagging `as_ptr`; pointee live for the spawn
                // via Scope::Drop). The push is infallible because capacity ≥
                // system_count (SCH6: one push per system per frame).
                ptrs.completion
                    .push(sys_idx)
                    .expect("invariant SCH6: completion_queue cap ≥ system_count");
                // Release pairs with the dispatcher's Acquire load on `pending`
                // (plan §5.4.5.1 diagram). Every byte the body wrote becomes
                // visible to the dispatcher before it reads pending == target.
                ptrs.completion.pending_fetch_add(Ordering::Release);
            });

            dispatched += 1;
        }

        // `to_spawn` was drained above; restore the empty buffer for reuse.
        self.executor_scratch.to_spawn = to_spawn;

        dispatched
    }

    /// Yields the GPU barrier-lowering inputs for this schedule (Phase 5 MF-6).
    ///
    /// Walks the conflict graph's directed `successors` edges and yields one
    /// [`GpuBarrierEdge`] per `producer → consumer` edge whose CONSUMER is a
    /// `SystemKind::GpuCompute` system. Each edge carries the producer's and
    /// consumer's [`GpuAccessIntent`] (a CPU producer with no declared intent
    /// yields an empty `Compute`-stage intent — it touches no device column).
    /// `boyko_render`'s `lower_barriers` consumes this iterator to emit precise
    /// Vulkan buffer barriers; the abstract-to-`Vk*` lowering lives entirely in
    /// `boyko_render` (no graphics type crosses this seam).
    ///
    /// # O2 — the `u32` indices are TRANSIENT
    ///
    /// `GpuBarrierEdge::{producer, consumer}` are a build-time `SystemIndex`
    /// projection valid ONLY against THIS `Schedule` and consumed in the same
    /// build pass by `lower_barriers`. They are NOT durable — **never persist a
    /// `u32` past the build pass.** The durable barrier key is the stable
    /// `(ArchetypeId, ComponentId)` (MF-7), which `lower_barriers` derives from
    /// the intent's device-column touches.
    ///
    /// Cold (schedule build time); never on the per-frame run path.
    pub fn gpu_barrier_inputs(&self) -> impl Iterator<Item = GpuBarrierEdge> + '_ {
        // An empty Compute-stage intent stands in for a producer / consumer that
        // declares none (a CPU producer touches no device column).
        let empty_intent = GpuAccessIntent::new(GpuStage::Compute);
        self.conflict_graph
            .successors
            .iter()
            .enumerate()
            .flat_map(move |(producer, succs)| {
                let producer = producer as u32;
                let empty_intent = empty_intent.clone();
                succs.iter().filter_map(move |&consumer_idx| {
                    let consumer = consumer_idx.0 as usize;
                    if self.systems[consumer].kind != SystemKind::GpuCompute {
                        return None;
                    }
                    let producer_intent = self.systems[producer as usize]
                        .system
                        .meta()
                        .gpu_intent()
                        .cloned()
                        .unwrap_or_else(|| empty_intent.clone());
                    let consumer_intent = self.systems[consumer]
                        .system
                        .meta()
                        .gpu_intent()
                        .cloned()
                        .unwrap_or_else(|| empty_intent.clone());
                    Some(GpuBarrierEdge {
                        producer,
                        consumer: consumer_idx.0 as u32,
                        producer_intent,
                        consumer_intent,
                    })
                })
            })
    }
}

/// One directed producer→consumer GPU barrier-lowering input (Phase 5 MF-6).
///
/// Yielded by [`Schedule::gpu_barrier_inputs`]; consumed by `boyko_render`'s
/// `lower_barriers` to emit a precise Vulkan buffer barrier. A PUBLIC POD that
/// leaks NO internal type — it exposes only `u32` system indices and the public
/// [`GpuAccessIntent`].
///
/// # O2 — `producer` / `consumer` are TRANSIENT
///
/// The `u32` indices are a build-time `SystemIndex` projection valid ONLY against
/// the producing `Schedule`, consumed in the same build pass. They are NOT
/// durable — **never persist a `u32` past the build pass.** The durable barrier
/// key is the stable `(ArchetypeId, ComponentId)` (MF-7), derived by
/// `lower_barriers` from the intents' device-column touches.
#[derive(Clone, Debug)]
pub struct GpuBarrierEdge {
    /// Transient build-time index of the producer system (a `SystemIndex.0`
    /// projection — see the O2 note).
    pub producer: u32,
    /// Transient build-time index of the consumer system. The consumer is always
    /// a `SystemKind::GpuCompute` system (the filter predicate).
    pub consumer: u32,
    /// The producer's declared GPU access intent (an empty `Compute`-stage intent
    /// when the producer is a CPU system touching no device column).
    pub producer_intent: GpuAccessIntent,
    /// The consumer's declared GPU access intent (the GPU-compute system's
    /// device-column touches that drive the barrier's `dst` masks).
    pub consumer_intent: GpuAccessIntent,
}

/// Bundle captured by each spawn closure in
/// [`Schedule::try_dispatch_ready`]. `systems` references a slot in the owning
/// `Schedule::systems` Vec (a SEPARATE heap buffer); `completion` is a Copy
/// cell over the completion channel's OWN heap allocation (Phase 9.3c). Their
/// validity for the closure's lifetime is established by the surrounding SAFETY
/// block (see `try_dispatch_ready`'s spawn invocation).
///
/// # Disjoint-capture sidestep
///
/// Rust 2021 disjoint capture (RFC 2229) makes a `move ||` closure capture the
/// *individual paths* it touches. The `systems: *mut SystemBox` does not
/// implement `Send`, so a closure that captured it as a bare field would be
/// `!Send`. `systems` is therefore reached ONLY through the `&self`
/// `system_slot` method, whose receiver borrows the whole `SpawnPointers`, so
/// the closure captures the entire value and the `unsafe impl Send` applies.
/// (`completion` is `Send` on its own, so accessing it directly is harmless;
/// the whole-capture forced by `system_slot` covers it anyway.)
///
/// `Copy` so the `move` closure captures the value rather than a borrow; `Send`
/// so it can cross the worker boundary.
#[derive(Clone, Copy)]
struct SpawnPointers<'a> {
    /// Base pointer into `Schedule::systems` (a SEPARATE Vec heap buffer,
    /// already outside the `Schedule` allocation). Index by `SystemIndex.0`.
    systems: *mut SystemBox,
    /// Copy cell over the completion channel's OWN heap allocation (Phase
    /// 9.3c). Reached only through the cell's non-retagging accessors.
    completion: CompletionCell<'a>,
}

impl<'a> SpawnPointers<'a> {
    /// Returns the `SystemBox` slot for the given system index.
    ///
    /// # Safety
    /// * The pointer must point inside the owning `Schedule::systems`
    ///   Vec, which is `'scope`-alive (Scope::Drop blocks).
    /// * No concurrent worker may alias the same `SystemBox` (enforced
    ///   by the conflict graph + `running` bitset; see SCH3).
    ///
    /// Kept as an `&self` method so the spawn closure's receiver borrow
    /// captures the WHOLE `SpawnPointers`, keeping the `!Send`
    /// `*mut SystemBox` behind the struct's `unsafe impl Send` (Phase 9.3c:
    /// `completion` is `Send` on its own, but `systems` still needs this).
    #[inline]
    unsafe fn system_slot(&self, idx: usize) -> *mut SystemBox {
        // SAFETY (S1 + SCH3): forwarded to the caller; see method doc.
        unsafe { self.systems.add(idx) }
    }
}

/// Phase 21 (H2) — cold panic site for the [`Schedule::run`] world-binding
/// gate. Out-of-line so the hot run-entry path carries only the compare +
/// never-taken branch.
#[cold]
#[inline(never)]
fn schedule_world_mismatch_panic(built: WorldId, got: WorldId) -> ! {
    panic!(
        "boyko-B9101: Schedule::run called with a different world than the one it was \
         built on (built on {built}, got {got}) — a Schedule is bound to the world it \
         was built on; build a separate Schedule per world"
    );
}

// SAFETY (SEND1 / SEND3 — Phase 9 §9.2, updated Phase 9.3c):
//
// `SpawnPointers<'a>` carries `systems: *mut SystemBox` (`!Send` by default)
// plus `completion: CompletionCell<'a>` (already `Send`). We hand-mark the
// whole bundle `Send` because it escapes into a `scope.spawn` closure that runs
// on a worker thread. The members:
//
//   - `systems: *mut SystemBox` — references a slot inside the parent
//     `Schedule::systems` Vec's separate heap buffer. That buffer lives for
//     `'scope` (Scope::Drop blocks before the surrounding `&mut Schedule`
//     ends). Multiple workers may hold pointers to *different* indices
//     concurrently; the conflict graph (SCH3) guarantees no two concurrent
//     workers point to the same index.
//
//   - `completion: CompletionCell<'a>` — a Copy cell over the completion
//     channel's own heap allocation; already `Send` (its own SAFETY block
//     covers the `Sync` channel interior — `ArrayQueue` + `AtomicUsize`).
//
// `Sync` is unnecessary — the bundle is captured by value into the `move`
// closure (Copy), not shared via reference.
unsafe impl<'a> Send for SpawnPointers<'a> {}

#[cfg(test)]
mod tests {
    // Test-only observation channel: `Arc<Mutex<Vec<u8>>>` is the execution-order
    // log the scheduler assertions read back from worker threads — harness state,
    // never engine data. Compiled out of every shipping build.
    #![allow(clippy::disallowed_types)]

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
        fn check_change_tick(&mut self, current: crate::ecs::core::change_detection::Tick) {
            self.meta.last_run = self.meta.last_run.check_tick(current);
            self.meta.this_run = self.meta.this_run.check_tick(current);
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
            // Pre-initialise so SystemBox::new resolves CpuExclusive.
            let mut world_pre = EcsMaster::new();
            sys.initialize(&mut world_pre);
            let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
            let system_box = SystemBox::new(boxed);
            assert_eq!(
                system_box.kind,
                crate::ecs::core::system::system_kind::SystemKind::CpuExclusive,
                "ExclusiveFunctionSystem must resolve SystemKind::CpuExclusive"
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
            fn check_change_tick(&mut self, current: crate::ecs::core::change_detection::Tick) {
                self.meta.last_run = self.meta.last_run.check_tick(current);
                self.meta.this_run = self.meta.this_run.check_tick(current);
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
        // The skip must NOT touch pending / queue (it is invisible to the
        // apply-window accounting).
        // SAFETY (Phase 9.3c): single-threaded test, no worker exists; the cell
        //   reads the channel through the non-retagging `as_ptr` lineage.
        let completion =
            unsafe { CompletionCell::new(schedule.executor_scratch.completion) };
        assert_eq!(
            completion.pending_load(Ordering::Relaxed),
            0,
            "a skip never bumps pending_apply"
        );
        assert!(
            completion.queue_is_empty(),
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
        fn check_change_tick(&mut self, current: crate::ecs::core::change_detection::Tick) {
            self.meta.last_run = self.meta.last_run.check_tick(current);
            self.meta.this_run = self.meta.this_run.check_tick(current);
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

        // Frame 1: `this_run` is the FRAME-START tick bumped at the top of `run`.
        // Bug #56 adds a SECOND (apply-window) bump per frame, so the world's
        // end-of-frame `current_tick()` is `frame_start_this_run + 1`. The
        // condition's `this_run` is pinned to the frame-start value, hence one
        // behind the world's current tick (NOT equal — the pre-#56 coupling).
        schedule.run(&mut world);
        let this_f1 = this_seen.load(Ordering::Relaxed);
        assert_eq!(
            this_f1,
            world.current_tick().get().wrapping_sub(1),
            "frame-1 condition this_run must equal the frame-start tick, one behind the \
             world's apply-window-bumped current tick (Bug #56: 2 bumps/frame)"
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

    // ── Phase 16.1 — Gap #1 / C1 dormancy + Gap #2 wraparound clamp ───────────
    //
    // The integration suite (`tests/phase16_1_dormant.rs`) proves the REACHABLE
    // public-API behavior (the C1 system-body dormancy end-to-end, plus the
    // every-frame eval-site checkpoint). These in-crate unit tests reach what the
    // external crate cannot: (a) a CONDITION genuinely NOT evaluated for several
    // frames (boyko's eager fold means no public single-schedule topology leaves
    // a reachable system's condition unevaluated — OQ-1 — so the only way to
    // drive a dormant condition is to call `run_condition` selectively, which is
    // `pub(crate)`), and (b) `Schedule::check_change_ticks` (a private method) on
    // a dormant condition AND a dormant system past `CHECK_TICK_THRESHOLD`
    // (518_400_000 — unreachable by running real frames).

    use crate::ecs::core::change_detection::{CHECK_TICK_THRESHOLD, MAX_CHANGE_AGE, Tick};

    /// Gap #1 MECHANISM. A condition advances its `(last_run, this_run]` window
    /// ONLY on a frame it is actually evaluated (`run_condition`), so a condition
    /// dormant (not evaluated) for several `bump_change_tick` advances resumes
    /// with its `last_run` FROZEN at its last actual evaluation — and therefore
    /// its resume window spans EVERY tick that elapsed while it was dormant.
    ///
    /// This is the unit twin of the (unreachable-via-public-API) "state/set-gated
    /// `Changed<T>` condition resumes and sees dormant changes" integration case.
    /// A change stamped at a dormant-frame tick `T_mut` satisfies the resumed
    /// window iff `last_run` stayed frozen; the assert pins exactly that, and
    /// pins that a pre-fix unconditional frame-start bump (which would have
    /// advanced `last_run` to the last dormant frame) would have MISSED it.
    #[test]
    fn dormant_condition_resume_window_spans_skipped_ticks() {
        let mut world = EcsMaster::new();

        let last_seen = Arc::new(AtomicU32::new(0));
        let this_seen = Arc::new(AtomicU32::new(0));
        let mut cond = TickRecordingCondition {
            meta: SystemMeta::for_testing("dormant_cond"),
            last_run_seen: Arc::clone(&last_seen),
            this_run_seen: Arc::clone(&this_seen),
        };
        cond.initialize(&mut world);

        // Frame 1 — the condition IS evaluated. `prev` is the `for_testing`
        // sentinel; the new `this_run` is T1.
        let t1 = world.bump_change_tick();
        world.run_condition(&mut cond, t1);
        let this_after_f1 = this_seen.load(Ordering::Relaxed);
        assert_eq!(this_after_f1, t1.get(), "frame 1: condition this_run == T1");

        // Frames 2..=4 — the condition is DORMANT (NOT evaluated). The world tick
        // still advances each "frame"; a mutation lands at T_mut (frame 3).
        let _t2 = world.bump_change_tick();
        let t_mut = world.bump_change_tick(); // frame 3: a change is stamped here
        let _t4 = world.bump_change_tick();

        // Frame 5 — the condition is evaluated again. Its `last_run` MUST be the
        // FROZEN T1 (its last actual evaluation), NOT T4.
        let t5 = world.bump_change_tick();
        world.run_condition(&mut cond, t5);
        let last_after_f5 = last_seen.load(Ordering::Relaxed);
        let this_after_f5 = this_seen.load(Ordering::Relaxed);
        assert_eq!(
            last_after_f5,
            t1.get(),
            "Gap #1: a dormant condition's last_run stays FROZEN at its last actual \
             evaluation (T1), NOT advanced to the last dormant frame (T4)"
        );
        assert_eq!(this_after_f5, t5.get(), "frame 5: condition this_run == T5");

        // The resume window (T1, T5] therefore SPANS the dormant-frame mutation
        // at T_mut. A pre-fix frame-start bump would have produced last_run==T4,
        // and (T4, T5] does NOT contain T_mut.
        assert!(
            Tick::new(t_mut.get()).is_newer_than(Tick::new(last_after_f5), Tick::new(this_after_f5)),
            "the frozen resume window (T1, T5] must contain the dormant mutation T_mut \
             (this is the change a Changed<T> condition would observe on resume)"
        );
        assert!(
            !Tick::new(t_mut.get()).is_newer_than(Tick::new(_t4.get()), Tick::new(t5.get())),
            "regression net: a pre-fix last_run==T4 window (T4, T5] would have MISSED T_mut"
        );
    }

    /// A system that records the `(last_run, this_run)` its body would observe.
    /// Mirrors `TickRecordingCondition` but for `Out = ()` so it can model a
    /// gated SYSTEM body (C1).
    struct TickRecordingSystem {
        meta: SystemMeta,
        last_run_seen: Arc<AtomicU32>,
        this_run_seen: Arc<AtomicU32>,
    }

    // SAFETY (S1): reads only its own meta + writes foreign atomics; never
    //   touches the world cell.
    unsafe impl System for TickRecordingSystem {
        type Out = ();
        fn name(&self) -> &'static str {
            self.meta.name()
        }
        fn access(&self) -> &Access {
            self.meta.access()
        }
        fn initialize(&mut self, _w: &mut EcsMaster) {}
        unsafe fn run_unsafe(&mut self, _w: UnsafeEcsCell<'_>) {
            self.last_run_seen
                .store(self.meta.last_run().get(), Ordering::Relaxed);
            self.this_run_seen
                .store(self.meta.this_run().get(), Ordering::Relaxed);
        }
        fn meta(&self) -> &SystemMeta {
            &self.meta
        }
        fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick) {
            self.meta.last_run = last_run;
            self.meta.this_run = this_run;
        }
        fn check_change_tick(&mut self, current: Tick) {
            self.meta.last_run = self.meta.last_run.check_tick(current);
            self.meta.this_run = self.meta.this_run.check_tick(current);
        }
    }

    /// C1 MECHANISM (unit). A GATED system is stamped at its DISPATCH site (only
    /// on a frame it runs); a skipped frame leaves its ticks FROZEN. So a gated
    /// system run on frame 1, skipped frames 2..=4 (NOT stamped), run again on
    /// frame 5, observes a body window whose `last_run` is the FROZEN frame-1
    /// `this_run` — spanning every dormant tick.
    ///
    /// This drives the dispatch-stamp path directly through a real single-system
    /// `Schedule::run` with a gate that flips, asserting the resumed body's
    /// recorded `last_run` is frame 1's `this_run`, not frame 4's.
    #[test]
    fn gated_system_body_window_frozen_while_skipped() {
        // Use a real schedule so the actual C1 dispatch-stamp path runs. The
        // gate is a `run_once`-style external flip via a recorded condition that
        // returns the current value of a shared bool.
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);

        let last_seen = Arc::new(AtomicU32::new(0));
        let this_seen = Arc::new(AtomicU32::new(0));

        // Register the gated system through the descriptor directly (the test
        // harness style), then attach a flip-gate condition.
        {
            use crate::ecs::core::schedule::system_box::SystemBox;
            use crate::ecs::core::schedule::system_descriptor::SystemDescriptor;
            let sys = TickRecordingSystem {
                meta: SystemMeta::for_testing("gated_body"),
                last_run_seen: Arc::clone(&last_seen),
                this_run_seen: Arc::clone(&this_seen),
            };
            let boxed: Box<dyn System<Out = ()>> = Box::new(sys);
            builder
                .descriptors
                .push(SystemDescriptor::new(SystemBox::new(boxed)));
        }

        // Gate condition: returns the shared flag's current value. When false the
        // system is skipped (NOT stamped); when true it dispatches (stamped).
        let gate = Arc::new(std::sync::atomic::AtomicBool::new(true));
        {
            let gate_cl = Arc::clone(&gate);
            let cond = ProbeBoolSystem {
                meta: SystemMeta::for_testing("flip_gate"),
                verdict: gate_cl,
            };
            let boxed: Box<dyn System<Out = bool>> = Box::new(cond);
            builder.descriptors[0].conditions.push(boxed);
        }

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        // Frame 1: gate true ⇒ system dispatches ⇒ stamped. Capture frame-1
        // this_run (the dispatch-stamp value == frame_this_run).
        schedule.run(&mut world);
        let this_f1 = this_seen.load(Ordering::Relaxed);
        assert_ne!(this_f1, 0, "frame 1: gated system ran and recorded a non-zero this_run");

        // Frames 2..=4: gate false ⇒ system SKIPPED ⇒ NOT stamped (ticks frozen).
        gate.store(false, Ordering::Relaxed);
        schedule.run(&mut world);
        schedule.run(&mut world);
        schedule.run(&mut world);
        // The recorded values are unchanged (the body never ran on 2..=4).
        assert_eq!(
            this_seen.load(Ordering::Relaxed),
            this_f1,
            "frames 2..=4: skipped ⇒ the body never ran ⇒ no new recording"
        );

        // Frame 5: gate true ⇒ system dispatches again ⇒ stamped. Its body window
        // `last_run` MUST be the FROZEN frame-1 this_run (C1), proving a skipped
        // span did not advance it. Pre-fix the unconditional frame-start bump
        // would have made last_run == frame-4 this_run.
        gate.store(true, Ordering::Relaxed);
        schedule.run(&mut world);
        let last_f5 = last_seen.load(Ordering::Relaxed);
        let this_f5 = this_seen.load(Ordering::Relaxed);
        assert_eq!(
            last_f5, this_f1,
            "C1: a gated system's body last_run on resume equals its LAST RUN frame (frame 1), \
             NOT the last skipped frame — its ticks stayed frozen while skipped"
        );
        assert!(
            this_f5 > this_f1,
            "frame 5: this_run advanced past frame 1 ({this_f1} -> {this_f5})"
        );
    }

    /// Gap #2 (wraparound) — `Schedule::check_change_ticks` clamps a DORMANT
    /// condition's stale `last_run`/`this_run` so a span > `MAX_CHANGE_AGE` does
    /// not flip `Tick::is_newer_than`. Seed a condition with an ancient `last_run`
    /// (age > `MAX_CHANGE_AGE` relative to a `current` past `CHECK_TICK_THRESHOLD`)
    /// and assert the clamp pulls it back to `current - MAX_CHANGE_AGE`.
    #[test]
    fn check_change_ticks_clamps_dormant_condition() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        push_system(&mut builder, "gated", Access::new(), || {});
        {
            // A plain condition whose meta we will stale-seed.
            let cond = ProbeBoolSystem {
                meta: SystemMeta::for_testing("stale_cond"),
                verdict: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            };
            let boxed: Box<dyn System<Out = bool>> = Box::new(cond);
            builder.descriptors[0].conditions.push(boxed);
        }

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        // `current` past the threshold; seed a condition window whose age
        // EXCEEDS MAX_CHANGE_AGE (so the clamp actually fires — `check_tick` is a
        // no-op for in-range ticks). age = MAX_CHANGE_AGE + 100.
        let current = Tick::new(CHECK_TICK_THRESHOLD.wrapping_add(1000));
        let ancient = Tick::new(current.get().wrapping_sub(MAX_CHANGE_AGE).wrapping_sub(100));
        schedule.system_conditions[0][0].set_change_ticks(ancient, ancient);

        schedule.check_change_ticks(current);

        let clamped = schedule.system_conditions[0][0].meta();
        let expected = Tick::new(current.get().wrapping_sub(MAX_CHANGE_AGE));
        assert_eq!(
            clamped.last_run(),
            expected,
            "check_change_ticks clamps a dormant condition's stale last_run to \
             current - MAX_CHANGE_AGE"
        );
        assert_eq!(
            clamped.this_run(),
            expected,
            "check_change_ticks clamps a dormant condition's stale this_run too (OQ-4)"
        );
        // Post-clamp the age is bounded ⇒ is_newer_than no longer false-positives
        // a clamped tick as "newer than current".
        let age = current.get().wrapping_sub(clamped.last_run().get());
        assert!(age <= MAX_CHANGE_AGE, "post-clamp age must be <= MAX_CHANGE_AGE (was {age})");
    }

    /// Gap #2 (wraparound) — `Schedule::check_change_ticks` clamps a DORMANT
    /// SYSTEM's stale tick snapshot. This is the hole C1 specifically opens: once
    /// a gated system's `last_run` advances only when it runs, a long dormant span
    /// can age it past `MAX_CHANGE_AGE`. The clamp must pull it back so the
    /// resumed body's window is well-formed.
    #[test]
    fn check_change_ticks_clamps_dormant_system() {
        let pool = fresh_pool();
        let mut builder = ScheduleBuilder::new(pool);
        push_system(&mut builder, "dormant", Access::new(), || {});

        let mut world = EcsMaster::new();
        let mut schedule = builder.build(&mut world);

        // Seed a system window aged past MAX_CHANGE_AGE so the clamp fires.
        let current = Tick::new(CHECK_TICK_THRESHOLD.wrapping_add(7));
        let ancient = Tick::new(current.get().wrapping_sub(MAX_CHANGE_AGE).wrapping_sub(5));
        schedule.systems[0].system.set_change_ticks(ancient, ancient);

        schedule.check_change_ticks(current);

        let clamped = schedule.systems[0].system.meta();
        let expected = Tick::new(current.get().wrapping_sub(MAX_CHANGE_AGE));
        assert_eq!(
            clamped.last_run(),
            expected,
            "check_change_ticks clamps a dormant SYSTEM's stale last_run \
             (the hole C1 opens) to current - MAX_CHANGE_AGE"
        );
        assert_eq!(clamped.this_run(), expected, "and clamps this_run too");
    }

    /// Gap #2 PROPERTY — for ANY dormant span up to
    /// `CHECK_TICK_THRESHOLD + MAX_CHANGE_AGE` over BOTH a system's and a
    /// condition's `last_run`, after one `check_change_ticks` clamp the resumed
    /// window (a) never reports a change older than the last actual run as "newer"
    /// and (b) never false-positives a clamped tick. Mirrors the
    /// `phase10_wraparound_property.rs` style but exercises the schedule-level
    /// clamp entry point (private to this crate).
    #[test]
    fn prop_check_change_ticks_no_false_positive_after_clamp() {
        use proptest::prelude::*;
        use proptest::test_runner::{Config, TestRunner};

        let mut runner = TestRunner::new(Config {
            cases: 256,
            ..Config::default()
        });

        runner
            .run(
                &(
                    any::<u32>(),
                    0u32..=(CHECK_TICK_THRESHOLD.wrapping_add(MAX_CHANGE_AGE)),
                ),
                |(current_raw, dormant_span)| {
                    let current = Tick::new(current_raw);
                    // The system/condition last ran `dormant_span` ticks ago.
                    let last_actual = Tick::new(current_raw.wrapping_sub(dormant_span));

                    let pool = ThreadPoolBuilder::new().num_threads(1).build();
                    let mut builder = ScheduleBuilder::new(pool);
                    push_system(&mut builder, "p", Access::new(), || {});
                    {
                        let cond = ProbeBoolSystem {
                            meta: SystemMeta::for_testing("p_cond"),
                            verdict: Arc::new(std::sync::atomic::AtomicBool::new(true)),
                        };
                        let boxed: Box<dyn System<Out = bool>> = Box::new(cond);
                        builder.descriptors[0].conditions.push(boxed);
                    }
                    let mut world = EcsMaster::new();
                    let mut schedule = builder.build(&mut world);

                    schedule.systems[0]
                        .system
                        .set_change_ticks(last_actual, last_actual);
                    schedule.system_conditions[0][0]
                        .set_change_ticks(last_actual, last_actual);

                    schedule.check_change_ticks(current);

                    for meta in [
                        schedule.systems[0].system.meta(),
                        schedule.system_conditions[0][0].meta(),
                    ] {
                        // (b) the clamped last_run has bounded age ⇒ no
                        // false-positive "newer than current".
                        let age = current.get().wrapping_sub(meta.last_run().get());
                        prop_assert!(
                            age <= MAX_CHANGE_AGE,
                            "post-clamp age {} exceeds MAX_CHANGE_AGE (span={})",
                            age,
                            dormant_span
                        );
                        // A clamped tick must NOT report itself as strictly newer
                        // than `current` under a degenerate (current, current]
                        // window — the clamp keeps the relation well-formed.
                        prop_assert!(
                            !meta.last_run().is_newer_than(current, current),
                            "a clamped last_run must not false-positive as newer than current"
                        );
                    }
                    Ok(())
                },
            )
            .expect("property: check_change_ticks clamp holds for all dormant spans");
    }

    /// A test-only `System<Out = bool>` returning a shared `AtomicBool`'s value
    /// (a flip-gate), with the mandatory tick hooks. Used to drive a gated
    /// system's skip/run across frames and to stand in as a clampable condition.
    struct ProbeBoolSystem {
        meta: SystemMeta,
        verdict: Arc<std::sync::atomic::AtomicBool>,
    }

    // SAFETY (S1): reads only a foreign atomic; never touches the world cell.
    unsafe impl System for ProbeBoolSystem {
        type Out = bool;
        fn name(&self) -> &'static str {
            self.meta.name()
        }
        fn access(&self) -> &Access {
            self.meta.access()
        }
        fn initialize(&mut self, _w: &mut EcsMaster) {}
        unsafe fn run_unsafe(&mut self, _w: UnsafeEcsCell<'_>) -> bool {
            self.verdict.load(Ordering::Relaxed)
        }
        fn meta(&self) -> &SystemMeta {
            &self.meta
        }
        fn set_change_ticks(&mut self, last_run: Tick, this_run: Tick) {
            self.meta.last_run = last_run;
            self.meta.this_run = this_run;
        }
        fn check_change_tick(&mut self, current: Tick) {
            self.meta.last_run = self.meta.last_run.check_tick(current);
            self.meta.this_run = self.meta.this_run.check_tick(current);
        }
    }
}
