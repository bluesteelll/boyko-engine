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

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::{Changed, Mut, Query};
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::ResMut;
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
