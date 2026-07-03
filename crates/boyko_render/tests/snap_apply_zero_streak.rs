//! Host plan R5 — the SNAP zero-streak proof (the last-substep-teleport class),
//! on the real `EnableTag` path.
//!
//! `snap_apply` runs in Main, AFTER the whole fixed loop settled and BEFORE the
//! gather. A teleport enqueued on the FINAL fixed substep flushes at Main's drain
//! and is observed by `snap_apply`, which recomputes `curr = prev = from(Transform)`
//! from the LIVE pose (no lerp across the pose jump — the zero streak) and issues a
//! deferred `disable::<SnapInterpolation>()` so the snap lasts exactly one frame.
//!
//! Two things this pins that the pre-migration table-tag version could not:
//! * `SnapInterpolation` is a bitset `EnableTag` — the flag is a per-row bit
//!   (`Enabled<SnapInterpolation>` filter), toggled with `enable` / `disable`, over
//!   the DENSE `GpuTransform3D` pair (the kernel dense × enable-query feature).
//! * `snap_apply` reads `&Transform` (the P1 fix): the collapse is self-sufficient
//!   even when `curr` is STALE (a teleport that never ran a pack — the last-substep /
//!   Main-issued / 0-substep case). The real-teleport chain test exercises exactly
//!   that stale-`curr` path.

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::filter_enable::Enabled;
use boyko_ecs::ecs::core::system::{Commands, EntityCommands};

use boyko_macros::Bundle;
use boyko_math::Vec3;

use boyko_render::snap_interpolation::snap_apply;
use boyko_render::{
    GpuTransform3D, SnapInterpolation, TeleportCommandsExt, TrsPacked, pack_gpu_transforms,
};

use boyko_scene::transform::Transform;

use bytemuck::bytes_of;

/// A `(Transform, GpuTransform3D)` spawn payload — the table pose alongside the
/// dense interpolation pair (the pack's `(&Transform, &mut GpuTransform3D)` shape).
/// The `SnapInterpolation` bit is NOT part of the bundle (a bitset EnableTag is
/// toggled, not inserted), so the flagged shape uses the same bundle + an `enable`.
#[derive(Bundle)]
struct PairBundle {
    transform: Transform,
    pair: GpuTransform3D,
}

/// Spawns one FLAGGED entity carrying a table `Transform`, the dense
/// `GpuTransform3D` (whose `prev` and `curr` DIFFER — a body mid-motion whose naive
/// lerp would streak), and an ENABLED `SnapInterpolation` bit. Returns its `Entity`.
///
/// `curr` is seeded at x=50 (the post-teleport pose) so the collapse is a no-op on
/// `curr` but a real change on `prev` — the same-`curr`-and-`Transform` case.
fn spawn_flagged_moving(world: &mut EcsMaster) -> Entity {
    // prev at the origin, curr jumped far away — the "teleport" endpoints.
    let prev = TrsPacked::from_transform(&Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)));
    let curr = TrsPacked::from_transform(&Transform::from_translation(Vec3::new(50.0, 0.0, 0.0)));
    let pair = GpuTransform3D { prev, curr };
    let transform = Transform::from_translation(Vec3::new(50.0, 0.0, 0.0));

    let sink: std::sync::Arc<std::sync::Mutex<Option<Entity>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let probe = std::sync::Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(PairBundle { transform, pair })
            .enable::<SnapInterpolation>()
            .id();
        *probe.lock().expect("probe") = Some(e);
    });
    sink.lock().expect("probe").expect("spawn handle")
}

/// Reads the sole dense pair (every test spawns exactly one).
fn read_pair(world: &mut EcsMaster) -> GpuTransform3D {
    let view = world.query::<&GpuTransform3D, ()>();
    let mut it = view.dense_iter();
    let (_e, pair) = it.next().expect("exactly one dense pair exists");
    *pair
}

/// Counts the ENABLED-bit rows over the dense pair — the `snap_apply` shape. Uses
/// the FILTERED `.iter()` (NOT `dense_iter`, which bypasses the enable term — the
/// dense fast driver strides the store ignoring the filter).
fn enabled_count(world: &mut EcsMaster) -> usize {
    world
        .query::<&GpuTransform3D, Enabled<SnapInterpolation>>()
        .iter()
        .count()
}

/// A teleport flagged this frame (`SnapInterpolation` enabled) is collapsed to
/// `curr = prev = from(Transform)` by `snap_apply` — the following gather reads a
/// zero-streak pair — and the bit is disabled for the next frame (the deferred
/// one-shot disable).
#[test]
fn snap_apply_collapses_prev_to_curr_and_clears_the_bit() {
    let mut world = EcsMaster::new();
    let entity = spawn_flagged_moving(&mut world);

    // Precondition: the pair genuinely streaks before the snap (prev != curr).
    let before = read_pair(&mut world);
    assert_ne!(
        bytes_of(&before.prev),
        bytes_of(&before.curr),
        "precondition: the body is mid-motion (prev != curr — a naive lerp streaks)"
    );
    // Precondition: the snap bit is enabled before snap_apply (the teleport landed
    // it — the last-substep teleport whose enable flushed at Main's drain).
    assert_eq!(enabled_count(&mut world), 1, "precondition: the body's SnapInterpolation bit is enabled");

    // snap_apply (Main, pre-gather): collapse curr = prev = from(Transform) AND
    // enqueue the deferred disable. `run_system` flushes the deferred queue on return.
    world.run_system(snap_apply);

    // The zero-streak: the following gather reads prev == curr (mix(prev, curr,
    // alpha) == curr at every alpha — the body draws at the post-teleport pose).
    let after = read_pair(&mut world);
    assert_eq!(
        bytes_of(&after.prev),
        bytes_of(&after.curr),
        "snap_apply collapsed the pair to prev == curr (the zero streak)"
    );
    // And curr equals the LIVE Transform pose (x=50) — snap_apply recomputes from
    // &Transform, not the stale packed curr (the P1 fix; here they coincide).
    let want = TrsPacked::from_transform(&Transform::from_translation(Vec3::new(50.0, 0.0, 0.0)));
    assert_eq!(bytes_of(&after.curr), bytes_of(&want), "curr is the live Transform pose");

    // The bit cleared: the snap lasted exactly one frame (the deferred disable
    // applied when run_system flushed). The still-live entity's pair is intact.
    assert_eq!(enabled_count(&mut world), 0, "snap_apply's deferred disable cleared the bit for next frame");
    let _ = entity;
    // The dense pair survived (a bit disable is a per-row toggle — no archetype
    // migration, the dense column is untouched).
    assert_eq!(
        world.query::<&GpuTransform3D, ()>().dense_iter().count(),
        1,
        "the interpolation pair survives the bit disable (dense column untouched)"
    );
}

/// THE last-substep-teleport chain (the P1 witness): a teleport issued from a Main
/// system on a 0-substep frame writes the new `Transform` + enables the bit, but NO
/// pack runs to refresh `curr` — so `curr` is STALE at snap_apply. The collapse must
/// STILL land the pair at the post-teleport pose because `snap_apply` reads
/// `&Transform`, not `curr`. Chain: spawn interp body (curr stale at origin) →
/// `teleport_to(x=42)` from Main → drain → snap_apply → assert prev == curr ==
/// from(x=42) (NOT the stale origin).
#[test]
fn last_substep_teleport_collapses_to_live_transform_not_stale_curr() {
    let mut world = EcsMaster::new();

    // Spawn a still body at the origin: Transform = origin, pair seeded prev == curr
    // == origin. No SnapInterpolation bit yet (a normal interpolated body).
    let sink: std::sync::Arc<std::sync::Mutex<Option<Entity>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let probe = std::sync::Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let origin = Transform::from_translation(Vec3::new(0.0, 0.0, 0.0));
        let pair = GpuTransform3D::from_transform(&origin);
        let e = cmds.spawn(PairBundle { transform: origin, pair }).id();
        *probe.lock().expect("probe") = Some(e);
    });
    let entity = sink.lock().expect("probe").expect("spawn handle");

    // The stale-curr setup: `curr` is at the origin, and NO pack runs after the
    // teleport (the 0-substep frame). So a `prev = curr` collapse would draw the
    // body at the OLD origin — the exact P1 regression the &Transform read fixes.
    let before = read_pair(&mut world);
    assert_eq!(before.curr.pos[0], 0.0, "precondition: curr is stale at the origin");

    // Teleport from a Main system (the last-substep / Main-issued class): write the
    // new Transform (x=42) AND enable the snap bit in one deferred window. No pack
    // runs — curr stays at the origin.
    const TELEPORT_X: f32 = 42.0;
    world.run_system(move |mut cmds: Commands| {
        let mut ec: EntityCommands = cmds.entity(entity);
        ec.teleport_to(Transform::from_translation(Vec3::new(TELEPORT_X, 0.0, 0.0)));
    });

    // The bit is now enabled (the teleport's enable drained); curr is STILL stale.
    assert_eq!(enabled_count(&mut world), 1, "the teleport enabled the snap bit");
    let mid = read_pair(&mut world);
    assert_eq!(mid.curr.pos[0], 0.0, "curr is STILL stale (no pack ran on the 0-substep frame)");

    // snap_apply: recompute from the LIVE Transform (x=42), NOT the stale curr.
    world.run_system(snap_apply);

    let after = read_pair(&mut world);
    let want = TrsPacked::from_transform(&Transform::from_translation(Vec3::new(TELEPORT_X, 0.0, 0.0)));
    assert_eq!(
        bytes_of(&after.prev),
        bytes_of(&after.curr),
        "the collapse is a zero streak (prev == curr)"
    );
    assert_eq!(
        bytes_of(&after.curr),
        bytes_of(&want),
        "P1: the pair collapsed to the LIVE Transform (x=42), NOT the stale curr (origin)"
    );
    assert_eq!(enabled_count(&mut world), 0, "the one-shot snap disabled the bit");
}

/// Spawns a body with an explicit prev/curr history + the table `Transform` at
/// `transform_x`, optionally with the `SnapInterpolation` bit ENABLED.
fn spawn_history_body(
    world: &mut EcsMaster,
    prev_x: f32,
    curr_x: f32,
    transform_x: f32,
    snap: bool,
) {
    let prev = TrsPacked::from_transform(&Transform::from_translation(Vec3::new(prev_x, 0.0, 0.0)));
    let curr = TrsPacked::from_transform(&Transform::from_translation(Vec3::new(curr_x, 0.0, 0.0)));
    let pair = GpuTransform3D { prev, curr };
    let transform = Transform::from_translation(Vec3::new(transform_x, 0.0, 0.0));
    world.run_system(move |mut cmds: Commands| {
        let mut e = cmds.spawn(PairBundle { transform, pair });
        if snap {
            e.enable::<SnapInterpolation>();
        }
    });
}

/// The per-row-branching `pack_gpu_transforms` split by the `SnapInterpolation`
/// bit: a flagged body gets `curr = prev = new` (the snap branch — for a bit that
/// persisted across substeps), an unflagged body gets the normal `prev = old curr;
/// curr = new` shuffle. The pack reads the enable STATE per row via `IsEnabled`.
#[test]
fn pack_snaps_flagged_body_and_shuffles_the_rest() {
    let mut world = EcsMaster::new();
    // Flagged: prev=0, old curr=50, Transform=50 (a teleport whose bit persisted).
    spawn_history_body(&mut world, 0.0, 50.0, 50.0, true);
    // Unflagged: prev=1, old curr=2, Transform=9 (a normal moving body).
    spawn_history_body(&mut world, 1.0, 2.0, 9.0, false);

    world.run_system(pack_gpu_transforms);

    // Read both bodies via the enable filter to identify which is flagged (the
    // flagged one is the sole row `Enabled<SnapInterpolation>` sees).
    let flagged_curr_x = {
        let q = world.query::<&GpuTransform3D, Enabled<SnapInterpolation>>();
        let mut it = q.iter();
        let p = it.next().expect("one flagged body");
        assert!(it.next().is_none(), "exactly one flagged body");
        p.curr.pos[0]
    };
    assert_eq!(flagged_curr_x, 50.0, "the x=50 body is the flagged one");

    // All rows via the dense fast path (both survive); pick each by curr.pos.x.
    let mut rows: Vec<GpuTransform3D> =
        world.query::<&GpuTransform3D, ()>().dense_iter().map(|(_e, p)| *p).collect();
    rows.sort_by(|a, b| a.curr.pos[0].partial_cmp(&b.curr.pos[0]).unwrap());

    // The unflagged body (curr x = 9, the smaller): prev = its OLD curr (x = 2),
    // curr = new Transform (x = 9) — the normal shuffle, prev != curr.
    let np = rows[0];
    assert_eq!(np.prev.pos[0], 2.0, "the shuffle branch set prev = old curr");
    assert_eq!(np.curr.pos[0], 9.0, "the shuffle branch wrote curr = new Transform");
    assert_ne!(
        bytes_of(&np.prev),
        bytes_of(&np.curr),
        "the unflagged body keeps a real interpolation span (prev != curr)"
    );

    // The flagged body (curr x = 50): curr == prev (the snap branch collapsed it to
    // the current Transform pose).
    let fp = rows[1];
    assert_eq!(
        bytes_of(&fp.prev),
        bytes_of(&fp.curr),
        "the snap branch collapsed the flagged body to prev == curr"
    );
    assert_eq!(fp.curr.pos[0], 50.0, "the flagged body's curr is the current Transform pose");
}

/// A frame with NO flagged body does zero work — `snap_apply` collapses nothing
/// and clears nothing (the 0%-gate; a still, non-teleporting body keeps its
/// interpolation pair untouched).
#[test]
fn snap_apply_leaves_unflagged_bodies_untouched() {
    let mut world = EcsMaster::new();
    // An UNFLAGGED moving body (prev=1, curr=2, no SnapInterpolation bit).
    spawn_history_body(&mut world, 1.0, 2.0, 9.0, false);

    let before = read_pair(&mut world);
    world.run_system(snap_apply);
    let after = read_pair(&mut world);

    assert_eq!(
        bytes_of(&before),
        bytes_of(&after),
        "an unflagged body's pair is untouched by snap_apply (the 0%-gate)"
    );
    assert_ne!(
        bytes_of(&after.prev),
        bytes_of(&after.curr),
        "the unflagged body still streaks (snap did not fire) — interpolation stays ON"
    );
}
