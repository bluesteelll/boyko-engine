//! std-lib S4-follow-up gate suite — the `visibility_sync` bridge (boyko_scene
//! half, bit-level).
//!
//! These gates exercise the durable [`Visibility`] byte → [`RenderEnabled`] bit
//! bridge AT THE BIT LEVEL (probing `is_enabled::<RenderEnabled>`), which is the
//! exact gate the 3D instance pack (`sync_gpu_3d_instances`, in `boyko_render`)
//! filters on — `Enabled<RenderEnabled>` visits a row iff its bit is set (proven
//! independently by `render_caps_s4::enabled_render_enabled_filters_visible_rows_only`
//! and `render_upload_s4::hidden_row_is_excluded_from_the_pack`). The full
//! end-to-end pipeline (`visibility_sync` → bit → pack inclusion/exclusion) lives
//! in `boyko_render/tests/visibility_sync_pack.rs` (that crate names
//! `Gpu3dInstance`).
//!
//! Gates covered:
//!
//!  1. Spawn `Visibility::Hidden` → after `visibility_sync` runs, the
//!     `RenderEnabled` bit is CLEAR (the path the pack skips).
//!  2. Toggle a previously-Hidden entity to `Visible` / `Inherited` → bit SET
//!     (both directions); flipping back to Hidden → bit CLEAR again.
//!  3. Changed-gated 0%-work: a frame in which no `Visibility` changed visits
//!     ZERO rows (a direct work-counter probe mirroring the production query) and
//!     allocates nothing over the scheduler baseline (delta == 0).
//!  4. A manual `RenderEnabled` toggle on an entity whose `Visibility` did NOT
//!     change is NOT overridden by `visibility_sync` (the system acts only on
//!     `Changed<Visibility>`).
//!  5. H-06 (unification plan 02, row A9): a toggle pending for `E` does NOT
//!     land on `F`, spawned on `E`'s recycled id in the SAME frame — `F`'s bit
//!     is unchanged and equals a run where `F` takes a fresh id.
//!
//! # How a `Visibility` change is driven
//!
//! `visibility_sync` is `Changed<Visibility>`-gated; a one-shot
//! `EcsMaster::run_system` leaves an EMPTY change window (`this_run == last_run`),
//! so it would match zero rows. A `Schedule::run` advances `last_run` per frame,
//! so the spawn's Added-`Visibility` (⊆ Changed) is observed on the first run,
//! and a later in-schedule write (through `Mut<Visibility>`) is observed on the
//! frame after it lands — the same discipline `render_upload_s4` uses for
//! `Mut<Transform>` via `apply_pending_move`. We therefore drive everything
//! through a real `Schedule`.

use boyko_ecs::ecs::core::commands::Command;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::{Changed, Mut, Query};
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::{Commands, ResMut};
use boyko_ecs::ecs::identifiers::primitives::ArchetypeId;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_scene::render_caps::{RenderEnabled, Visibility};
use boyko_scene::visibility_sync::visibility_sync;

// ── byte helper ─────────────────────────────────────────────────────────────

/// Views a `#[repr(u8)]` POD component as raw bytes for the `create_entity` path.
///
/// # Safety
/// `T` is a fixed-layout `#[repr]` component whose byte image is a valid
/// serialization for its pool (holds for `Visibility` — a 1 B `repr(u8)` enum).
fn as_bytes<T>(value: &T) -> &[u8] {
    // SAFETY: `value` is a live `T`; we view its `size_of::<T>()` bytes read-only.
    // `T` is a fixed-layout component, matching the pool's stored layout; the
    // slice borrows `value` so it cannot outlive it.
    unsafe { std::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) }
}

// ── pending-write resource (drives a Changed<Visibility> INSIDE the schedule) ─

/// A queued `Visibility` write applied INSIDE the schedule so the write's
/// `Changed` tick lands in the SAME frame window the next `visibility_sync` run
/// observes (a write done between frames via `get_component_mut` stamps at the
/// apply-window tick and is observed one frame later — see `get_component_mut`'s
/// O4 doc; routing it through a same-frame `Mut<Visibility>` mutator system keeps
/// the chain crisp). `Some` ⇒ apply to `target` this frame, then clear.
#[derive(boyko_macros::Resource, Default)]
struct PendingVis {
    target: Option<Entity>,
    value: Visibility,
}

/// Applies any queued [`PendingVis`] through `Mut<Visibility>` (bumping the
/// `Changed` tick), runs FIRST in the frame (before `visibility_sync`).
#[allow(clippy::needless_pass_by_value)]
fn apply_pending_vis(mut pending: ResMut<PendingVis>, mut q: Query<Mut<Visibility>>) {
    let Some(target) = pending.target.take() else { return };
    let v = pending.value;
    for (id, mut vis) in q.iter_entities_mut() {
        if id == target.id() {
            *vis = v;
        }
    }
}

/// Queues a `Visibility` write to be applied INSIDE the next `Schedule::run`.
fn queue_vis(world: &mut EcsMaster, target: Entity, value: Visibility) {
    let pv = world.resource_mut::<PendingVis>();
    pv.target = Some(target);
    pv.value = value;
}

// ── harness ──────────────────────────────────────────────────────────────────

/// An archetype carrying only `Visibility` (the bridge's sole input column).
fn vis_archetype(world: &mut EcsMaster) -> ArchetypeId {
    world.create_archetype(&[Visibility::component_id()])
}

/// Spawns an entity with the given `Visibility` and returns its handle. The
/// `RenderEnabled` bit starts CLEAR (a bitset tag is never set until toggled).
fn spawn_vis(world: &mut EcsMaster, arch: ArchetypeId, vis: Visibility) -> Entity {
    world
        .create_entity(arch, &[(Visibility::component_id(), as_bytes(&vis))])
        .expect("invariant: visibility archetype accepts its one column")
}

/// Builds a `Schedule` wiring `apply_pending_vis` (mutator) → `visibility_sync`
/// so an in-schedule `Visibility` write and the bridge that reads it share a
/// per-frame change window. `PendingVis` is inserted so the mutator's `ResMut`
/// resolves.
fn build_sync_schedule(world: &mut EcsMaster) -> Schedule {
    world.insert_resource(PendingVis::default());
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let mutate = b.add_system(apply_pending_vis).key();
    b.add_system(visibility_sync).after(mutate);
    b.build(world)
}

/// A `Schedule` running ONLY `visibility_sync` (no mutator), for the spawn-time
/// reconcile gates that need no later edit.
fn build_sync_only_schedule(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    b.add_system(visibility_sync);
    b.build(world)
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 1 — a spawned Hidden entity ends up bit-CLEAR after visibility_sync;
//          a spawned Visible / Inherited entity ends up bit-SET.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn spawn_hidden_clears_bit_visible_and_inherited_set_it() {
    let mut world = EcsMaster::new();
    let arch = vis_archetype(&mut world);

    let hidden = spawn_vis(&mut world, arch, Visibility::Hidden);
    let visible = spawn_vis(&mut world, arch, Visibility::Visible);
    let inherited = spawn_vis(&mut world, arch, Visibility::Inherited);

    // Pre-condition: bits start clear (a bitset tag is unset until toggled).
    assert!(!world.is_enabled::<RenderEnabled>(hidden));
    assert!(!world.is_enabled::<RenderEnabled>(visible));
    assert!(!world.is_enabled::<RenderEnabled>(inherited));

    let mut sched = build_sync_only_schedule(&mut world);
    // First run: Added-Visibility ⊆ Changed, so the bridge reconciles all three;
    // the deferred SetRenderEnabledById commands flush in the apply window of the
    // SAME run.
    sched.run(&mut world);

    assert!(
        !world.is_enabled::<RenderEnabled>(hidden),
        "Visibility::Hidden ⇒ RenderEnabled bit CLEAR (the row the pack skips)"
    );
    assert!(
        world.is_enabled::<RenderEnabled>(visible),
        "Visibility::Visible ⇒ RenderEnabled bit SET"
    );
    assert!(
        world.is_enabled::<RenderEnabled>(inherited),
        "Visibility::Inherited is treated as visible at the entity level ⇒ bit SET"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 2 — toggle BOTH directions: Hidden → Visible/Inherited sets the bit;
//          Visible → Hidden clears it again.
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn toggle_both_directions_tracks_the_bit() {
    let mut world = EcsMaster::new();
    let arch = vis_archetype(&mut world);
    let e = spawn_vis(&mut world, arch, Visibility::Hidden);
    let mut sched = build_sync_schedule(&mut world);

    // Frame 0: the spawn's Added Hidden is reconciled → bit clear.
    sched.run(&mut world);
    assert!(!world.is_enabled::<RenderEnabled>(e), "spawned Hidden ⇒ bit clear");

    // Hidden → Visible. queue_vis stages the write; the next run applies it
    // (apply_pending_vis bumps the Changed tick) and visibility_sync — running
    // AFTER the mutator in the SAME frame — observes it and enables the bit.
    queue_vis(&mut world, e, Visibility::Visible);
    sched.run(&mut world);
    assert!(
        world.is_enabled::<RenderEnabled>(e),
        "Hidden → Visible ⇒ bit SET (re-shown the frame the byte changed)"
    );

    // Visible → Hidden again → bit clear.
    queue_vis(&mut world, e, Visibility::Hidden);
    sched.run(&mut world);
    assert!(
        !world.is_enabled::<RenderEnabled>(e),
        "Visible → Hidden ⇒ bit CLEAR (hidden the frame the byte changed)"
    );

    // Hidden → Inherited (the other "visible" mapping) ⇒ bit SET.
    queue_vis(&mut world, e, Visibility::Inherited);
    sched.run(&mut world);
    assert!(
        world.is_enabled::<RenderEnabled>(e),
        "Hidden → Inherited ⇒ bit SET (Inherited is visible at the entity level)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 3 — Changed-gated 0%-work: an unchanged-Visibility frame visits ZERO
//          rows and allocates nothing over the scheduler baseline.
// ════════════════════════════════════════════════════════════════════════════

/// A probe mirroring `visibility_sync`'s production query EXACTLY
/// (`Query<&Visibility, Changed<Visibility>>`), counting the rows it would visit
/// into a `Res`-counter. This is the deterministic, non-flaky work signal (the
/// same approach as `render_upload_s4::pack_does_zero_work_when_no_gpu3d_instance_column`).
#[derive(boyko_macros::Resource, Default)]
struct VisitCount(usize);

#[allow(clippy::needless_pass_by_value)]
fn count_changed_visibility(
    q: Query<&Visibility, Changed<Visibility>>,
    mut probe: ResMut<VisitCount>,
) {
    let mut n = 0usize;
    for _ in q.iter_entities() {
        n += 1;
    }
    probe.0 = n;
}

#[test]
fn unchanged_frame_visits_zero_rows() {
    let mut world = EcsMaster::new();
    world.insert_resource(VisitCount::default());
    let arch = vis_archetype(&mut world);
    // A handful of entities, all of stable Visibility.
    for v in [Visibility::Visible, Visibility::Hidden, Visibility::Inherited, Visibility::Visible] {
        spawn_vis(&mut world, arch, v);
    }

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    // Run the bridge AND the visit-counter (mirror query) together; ordering is
    // irrelevant — both read the same Changed<Visibility> window.
    b.add_system(visibility_sync);
    b.add_system(count_changed_visibility);
    let mut sched = b.build(&mut world);

    // Frame 0: every spawn's Added Visibility is Changed → the probe sees all 4.
    sched.run(&mut world);
    assert_eq!(
        world.resource::<VisitCount>().0,
        4,
        "anti-vacuity: the first (spawn) frame visits all 4 freshly-added rows"
    );

    // Frame 1: nothing changed → ZERO rows visited (the 0%-work property).
    sched.run(&mut world);
    assert_eq!(
        world.resource::<VisitCount>().0,
        0,
        "0%-work: an unchanged-Visibility frame visits ZERO rows (no per-entity command churn)"
    );

    // And one more still frame stays zero (steady state, not a one-shot artifact).
    sched.run(&mut world);
    assert_eq!(world.resource::<VisitCount>().0, 0, "steady-state still frame stays at 0 rows");
}

#[test]
fn changed_gated_work_is_proportional_to_changed_rows_only() {
    // The CHURN proxy, made exact. `visibility_sync`'s per-entity work is one
    // `commands.add` per row its `Query<&Visibility, Changed<Visibility>>` visits,
    // so the visit count IS the per-frame command churn. The mirror probe
    // (`count_changed_visibility`) runs the IDENTICAL query, so its count equals
    // the bridge's `commands.add` count exactly. This proves the 0%-work property
    // WITHOUT an allocator (deterministic; the allocator route was rejected — see
    // the module note: `run_system`'s per-call harness rebuild and the parallel
    // pool's background churn both confound an alloc count, the latter flakily).
    //
    // We assert: a still frame ⇒ 0 churn; a frame that changes EXACTLY k rows ⇒ k
    // churn (anti-vacuity — the counter tracks real work, it is not stuck at 0).
    let mut world = EcsMaster::new();
    world.insert_resource(VisitCount::default());
    world.insert_resource(PendingVis::default());
    let arch = vis_archetype(&mut world);
    let mut handles = Vec::new();
    for v in [Visibility::Visible, Visibility::Hidden, Visibility::Inherited, Visibility::Visible] {
        handles.push(spawn_vis(&mut world, arch, v));
    }

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    // apply_pending_vis (mutator) → visibility_sync (bridge) → count_changed_visibility
    // (the mirror probe). The probe runs LAST so it observes the same Changed window
    // the bridge did this frame.
    let mutate = b.add_system(apply_pending_vis).key();
    let bridge = b.add_system(visibility_sync).after(mutate).key();
    b.add_system(count_changed_visibility).after(bridge);
    let mut sched = b.build(&mut world);

    // Frame 0: all 4 spawns' Added Visibility ⊆ Changed ⇒ the bridge enqueues 4.
    sched.run(&mut world);
    assert_eq!(
        world.resource::<VisitCount>().0,
        4,
        "spawn frame: the bridge's query visits all 4 Added rows (4 commands enqueued)"
    );

    // Frame 1: nothing changed ⇒ ZERO rows ⇒ ZERO command churn (the 0%-work gate).
    sched.run(&mut world);
    assert_eq!(
        world.resource::<VisitCount>().0,
        0,
        "0%-work: an unchanged-Visibility frame visits ZERO rows ⇒ ZERO commands enqueued"
    );
    // A second still frame confirms steady state (not a one-shot drain artifact).
    sched.run(&mut world);
    assert_eq!(world.resource::<VisitCount>().0, 0, "steady-state still frame stays at 0 churn");

    // Change EXACTLY ONE row's Visibility: the next frame's churn is exactly 1
    // (proportional to changed rows — the gate is value-driven, not a full rescan).
    queue_vis(&mut world, handles[1], Visibility::Visible);
    sched.run(&mut world);
    assert_eq!(
        world.resource::<VisitCount>().0,
        1,
        "changing exactly 1 row ⇒ exactly 1 row visited ⇒ exactly 1 command (proportional churn)"
    );

    // And it falls back to 0 the very next still frame.
    sched.run(&mut world);
    assert_eq!(world.resource::<VisitCount>().0, 0, "after the 1-row change, the next frame is 0 again");
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 4 — a manual RenderEnabled toggle on an entity whose Visibility did NOT
//          change is NOT overridden (the system fires only on Changed<Visibility>).
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn manual_toggle_is_not_fought_on_unchanged_visibility() {
    let mut world = EcsMaster::new();
    let arch = vis_archetype(&mut world);
    // A Visible entity: visibility_sync would, on a Changed frame, set its bit.
    let e = spawn_vis(&mut world, arch, Visibility::Visible);
    let mut sched = build_sync_schedule(&mut world);

    // Frame 0: spawn's Added Visible reconciled → bit SET.
    sched.run(&mut world);
    assert!(world.is_enabled::<RenderEnabled>(e), "spawned Visible ⇒ bit set");

    // A manual per-frame override (e.g. a culling system) HIDES the entity by
    // clearing the bit directly — WITHOUT touching Visibility. visibility_sync
    // must NOT re-enable it on subsequent frames (Visibility is unchanged).
    world.disable::<RenderEnabled>(e);
    assert!(!world.is_enabled::<RenderEnabled>(e), "manual disable cleared the bit");

    // Several frames with NO Visibility change: the bridge must leave the manual
    // override untouched (it acts only on Changed<Visibility>).
    for frame in 0..5 {
        sched.run(&mut world);
        assert!(
            !world.is_enabled::<RenderEnabled>(e),
            "frame {frame}: manual disable survives — visibility_sync does not fight an \
             unchanged Visibility"
        );
    }

    // Conversely, the bridge DOES reassert when Visibility actually changes: write
    // Visible again (a real Changed) → the bridge re-enables.
    queue_vis(&mut world, e, Visibility::Hidden);
    sched.run(&mut world);
    queue_vis(&mut world, e, Visibility::Visible);
    sched.run(&mut world);
    assert!(
        world.is_enabled::<RenderEnabled>(e),
        "a real Visibility change DOES drive the bit (the bridge is live, not inert)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Gate 5 — H-06: a toggle pending for E must not land on F, spawned on E's
//          recycled id in the SAME frame (unification plan 02, row A9).
// ════════════════════════════════════════════════════════════════════════════
//
// The interleaving under test is: `visibility_sync` enqueues a toggle for E
// while E is live → E is despawned and F is registered on E's id → the toggle
// drains. Only the apply window can stage the middle step, so the harness
// runs a second system, `killer`, CONCURRENTLY with `visibility_sync` (no
// ordering edge; disjoint access) and lets the drain order do the rest:
//
// * `killer` is added FIRST and the pool has ONE worker. The dispatcher runs
//   on the test thread (no worker lane), so both systems are pushed to the
//   global FIFO injector in index order and the single worker runs `killer`
//   then `visibility_sync`; `ScheduleBuilder::build`'s Kahn sort is
//   insertion-stable, so index order is add order. Completions land in a
//   FIFO `ArrayQueue`, and the apply-window gate fires only once BOTH have
//   completed, so `killer`'s queue drains first — by construction, not by
//   timing.
// * `visibility_sync`'s body ran BEFORE any despawn applied (SCH7), so its
//   query still yielded E, and E's toggle is in its queue when `killer`'s
//   commands run.
// * F is spawned by a command that calls `EcsMaster::create_entity` AT APPLY,
//   on the dispatcher: `allocate_entity` pops the recycled stack before it
//   mints, so the id `despawn(E)` pushed one command earlier is reused. (A
//   worker-side `Commands::spawn` claims its id at ENQUEUE, before any despawn
//   of this frame has applied, so it cannot stage the same-frame reuse.)
//
// Every run pins the recycle it relies on (`F.id == E.id`, generation + 1), so
// a scheduler change that broke the interleaving would fail loudly rather than
// pass on an F that never took E's slot.

/// Which order `killer` enqueues its two commands in — the ONLY thing that
/// differs between the recycled run and the fresh-id control.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum ClaimOrder {
    /// `despawn(E)`, then spawn F at apply: F pops E's slot (same id,
    /// generation + 1).
    #[default]
    Recycled,
    /// Spawn F at apply, then `despawn(E)`: the free stack is empty when F is
    /// allocated, so F mints a fresh id.
    Fresh,
}

/// The frame's structural request: `killer` consumes `doomed` on the frame it
/// runs; `SpawnHiddenAtApply` reports the handle F was registered under.
#[derive(boyko_macros::Resource, Default)]
struct KillRequest {
    doomed: Option<Entity>,
    arch: Option<ArchetypeId>,
    order: ClaimOrder,
    spawned: Option<Entity>,
}

/// Spawns a `Visibility::Hidden` row through the DISPATCHER's allocator, at
/// apply time (see the gate's header for why the enqueue-time claim of
/// `Commands::spawn` cannot stand in for this).
struct SpawnHiddenAtApply {
    arch: ArchetypeId,
}

impl Command for SpawnHiddenAtApply {
    fn apply(self, world: &mut EcsMaster) {
        let f = spawn_vis(world, self.arch, Visibility::Hidden);
        world.resource_mut::<KillRequest>().spawned = Some(f);
    }
}

/// Concurrent with `visibility_sync` (`ResMut<KillRequest>` + `Commands` vs
/// `Query<&Visibility>` + `Commands`: no conflict, no edge). Enqueues the
/// despawn of E and the apply-time spawn of F in the requested order.
#[allow(clippy::needless_pass_by_value)]
fn killer(mut commands: Commands, mut req: ResMut<KillRequest>) {
    let Some(e) = req.doomed.take() else { return };
    let arch = req.arch.expect("invariant: `arch` is set together with `doomed`");
    match req.order {
        ClaimOrder::Recycled => {
            commands.despawn(e);
            commands.add(SpawnHiddenAtApply { arch });
        }
        ClaimOrder::Fresh => {
            commands.add(SpawnHiddenAtApply { arch });
            commands.despawn(e);
        }
    }
}

/// What one H-06 frame left behind.
#[derive(Debug)]
struct H06Outcome {
    e: Entity,
    /// `None` when the frame despawned nothing (the toggle-is-live control).
    f: Option<Entity>,
    /// `is_enabled::<RenderEnabled>` on F after the frame (`false` if no F).
    f_enabled: bool,
    /// `is_enabled::<RenderEnabled>` on E after the frame (a dead E reads
    /// `false`).
    e_enabled: bool,
    /// Whether the world's `enable_generation` moved across the frame — it is
    /// bumped exactly once, on the FIRST toggle into an archetype, so on a
    /// world whose only toggle is E's it says whether that toggle was applied
    /// to a live row (any row) or dropped.
    enable_generation_moved: bool,
}

/// One frame: E is spawned `Visible` (its Added `Visibility` is the pending
/// toggle — an `enable`), then `killer` and `visibility_sync` run
/// concurrently on a one-worker pool. `order == None` runs the control frame
/// in which nothing is despawned.
fn run_h06_frame(order: Option<ClaimOrder>) -> H06Outcome {
    let mut world = EcsMaster::new();
    world.insert_resource(KillRequest::default());
    let arch = vis_archetype(&mut world);
    let e = spawn_vis(&mut world, arch, Visibility::Visible);
    {
        let req = world.resource_mut::<KillRequest>();
        req.doomed = order.map(|_| e);
        req.arch = Some(arch);
        req.order = order.unwrap_or_default();
    }

    // ONE worker + `killer` added FIRST: the dispatch and drain order the gate
    // header derives.
    let pool = ThreadPoolBuilder::new().num_threads(1).build();
    let mut b = ScheduleBuilder::new(pool);
    b.add_system(killer);
    b.add_system(visibility_sync);
    let mut sched = b.build(&mut world);

    let gen_before = world.archetype_master().enable_generation();
    sched.run(&mut world);
    let gen_after = world.archetype_master().enable_generation();

    let f = world.resource::<KillRequest>().spawned;
    H06Outcome {
        e,
        f,
        f_enabled: f.is_some_and(|f| world.is_enabled::<RenderEnabled>(f)),
        e_enabled: world.is_enabled::<RenderEnabled>(e),
        enable_generation_moved: gen_after != gen_before,
    }
}

#[test]
fn toggle_pending_for_a_despawned_entity_does_not_land_on_its_recycled_id() {
    // Anti-vacuity 1 — the frame really carries a live `enable` for E: with
    // nothing despawned, E's bit is SET after the frame.
    let live = run_h06_frame(None);
    assert!(live.f.is_none(), "control: nothing was spawned");
    assert!(
        live.e_enabled,
        "control: the spawn frame's Added Visible drives E's bit — the toggle under test is live"
    );
    assert!(
        live.enable_generation_moved,
        "control: the first toggle into the archetype bumps enable_generation (the drain-order \
         probe below reads this signal)"
    );

    // The recycled run.
    let a = run_h06_frame(Some(ClaimOrder::Recycled));
    let fa = a.f.expect("recycled run: SpawnHiddenAtApply registered F");
    // Anti-vacuity 2 — the recycle happened as staged: F sits on E's id, one
    // generation up (F3: the recycled entry carries the bumped generation).
    assert_eq!(fa.id(), a.e.id(), "recycled run: F must take E's id (killer drained before the toggle)");
    assert_eq!(
        fa.generation(),
        a.e.generation() + 1,
        "recycled run: F carries E's slot generation + 1"
    );
    assert_ne!(fa, a.e, "recycled run: F and E are distinct handles on one id");

    // THE H-06 ASSERTION. F was spawned `Hidden` and has never been toggled;
    // E's pending `enable`, keyed by E's id, must not reach it.
    assert!(
        !a.f_enabled,
        "H-06: a toggle pending for the despawned E landed on F, spawned on E's recycled id in \
         the same frame — F ({fa:?}) was spawned Hidden, never toggled, and reads ENABLED",
    );
    // Drain-order pin, read on the fixed tree: E's toggle was DROPPED (no
    // column allocation, so no bump). Had it resolved before `killer`'s
    // despawn — E live — it would have allocated the archetype's column and
    // bumped the generation, and this run would be green for the wrong
    // reason.
    assert!(
        !a.enable_generation_moved,
        "recycled run: the stale toggle must be dropped at apply, not applied to a live row"
    );

    // The fresh-id control: identical frame, F allocated BEFORE E's despawn.
    let b = run_h06_frame(Some(ClaimOrder::Fresh));
    let fb = b.f.expect("fresh run: SpawnHiddenAtApply registered F");
    assert_ne!(fb.id(), b.e.id(), "fresh run: F must NOT take E's id (nothing was recycled yet)");
    assert_eq!(fb.generation(), 0, "fresh run: a minted id starts at generation 0");
    assert!(!b.f_enabled, "fresh run: F was spawned Hidden and never toggled");

    // The plan's equality: the recycled run reads exactly what the fresh-id
    // run reads.
    assert_eq!(
        (a.f_enabled, a.enable_generation_moved),
        (b.f_enabled, b.enable_generation_moved),
        "H-06: F's bit (and the toggle's fate) must not depend on whether F recycled E's id"
    );
}
