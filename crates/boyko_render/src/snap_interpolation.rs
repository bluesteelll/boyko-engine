//! Host plan D4 (R5) — the interpolation SNAP / teleport seam.
//!
//! Interpolation lerps a body's drawn pose between its two most recent fixed
//! poses (`GpuTransform3D::prev` → `curr`). A TELEPORT — a discontinuous pose
//! jump (respawn, camera cut, a portal) — must NOT be lerped: a frame that
//! lerps across the jump draws the body streaking through the gap (the "zero
//! streak" the player must never see). The fix is to collapse `prev = curr` for
//! that one frame so `mix(prev, curr, alpha) == curr` at every `alpha`.
//!
//! # The two mechanisms (host plan D4, pinned)
//!
//! * [`SnapInterpolation`] — a per-entity `EnableTag` bit that flags a body
//!   whose pose jumped this frame. It is ENABLED by
//!   [`TeleportCommandsExt::teleport_to`] (alongside the `Transform` write) and
//!   DISABLED by [`snap_apply`] (a per-row bit toggle — no archetype migration).
//! * [`snap_apply`] — a Main-schedule system (`.before(gather_mesh_draws)`)
//!   that, for every enabled body, recomputes the collapsed pair from its live
//!   `Transform` (`curr = prev = from(Transform)` — the zero streak) and issues
//!   the deferred `disable::<SnapInterpolation>()` so the snap lasts exactly one
//!   frame.
//!
//! # Storage: a real `EnableTag` (the kernel now supports dense × enable)
//!
//! `SnapInterpolation` is a bitset `EnableTag` (`#[component(storage =
//! "bitset")]`), filtered with `Enabled<SnapInterpolation>`. The pair it filters
//! ([`GpuTransform3D`]) is a DENSE component; historically a
//! `Query<&mut GpuTransform3D, Enabled<Tag>>` silently yielded ZERO rows because
//! the kernel candidate seed treated a dense INCLUDE term and an enable term as
//! mutually exclusive. That kernel hole is now CLOSED (the dense × enable-query
//! feature, `state.rs` dense-seed recull + `dense_iter` compile-reject), so the
//! intended `EnableTag` design lands: teleporting a body is an O(1) per-row bit
//! flip with NO archetype migration (the `RenderEnabled` precedent, lifted to
//! interpolation).
//!
//! # Mechanism attribution (the critic pin)
//!
//! A `Commands` enable issued during Fixed substep `k` flushes at that substep's
//! END, so the same-substep [`pack_gpu_transforms`](crate::gpu_transform_pack)
//! does NOT observe the bit — the pack's snap branch (`IsEnabled<SnapInterpolation>`
//! is `true`) therefore covers only bits enabled in EARLIER substeps/frames. The
//! last-substep teleport is handled by `snap_apply` ALONE: it runs in Main, AFTER
//! the whole fixed loop settled and BEFORE the gather, so a teleport enqueued on
//! the final substep is flushed (Main starts with a drain) and observed by
//! `snap_apply`, which recomputes `curr = prev = from(Transform)` from the live
//! pose BEFORE the gather reads the pair. `snap_apply` reads `&Transform` (not the
//! packed `curr`), so it is self-sufficient regardless of the pack timing — even
//! when `curr` is stale (the last-substep / Main-issued / 0-substep teleport). The
//! two paths are complementary, not redundant — do not fold them.

use boyko_ecs::ecs::core::commands::Command;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::filter_enable::Enabled;
use boyko_ecs::ecs::core::system::{Commands, EntityCommands};
use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_macros::Component;
use boyko_scene::Transform;

use crate::gpu_transform3d::{GpuTransform3D, TrsPacked};

/// The per-entity teleport marker — an `EnableTag` bit that, while ENABLED on a
/// body, tells [`snap_apply`] to collapse that body's interpolation pair to
/// `prev = curr` (no lerp across the pose jump) for one frame.
///
/// A bitset `EnableTag` (`#[component(storage = "bitset")]` — the
/// [`RenderEnabled`](boyko_scene::render_caps::RenderEnabled) shape): it has no
/// `ComponentPool` and is NOT part of any archetype signature, so
/// enabling/disabling it is O(1) with no archetype migration, no structural
/// generation bump, and no per-row bytes. The interpolation pair
/// ([`GpuTransform3D`]) is a DENSE component, and the
/// kernel NOW supports an enable-term filter over a dense-include query (the
/// dense × enable-query feature), so `snap_apply`'s
/// `Query<(&Transform, &mut GpuTransform3D), Enabled<SnapInterpolation>>` and the
/// pack's `IsEnabled<SnapInterpolation>` per-row read resolve correctly.
#[derive(Component, Clone, Copy, Debug)]
#[component(storage = "bitset")]
pub struct SnapInterpolation;

/// The zero-streak snap: for every body whose [`SnapInterpolation`] bit is
/// ENABLED, recompute its interpolation pair to `curr = prev = from(Transform)`
/// (no lerp across the pose jump) and issue the deferred
/// `disable::<SnapInterpolation>()` so the snap lasts exactly this one frame.
///
/// Runs in the Main schedule, `.before(gather_mesh_draws)`, so the collapsed
/// pair is what the gather reads (and hence what the GPU lerp sees) THIS frame —
/// the teleporting body draws at its live `Transform` pose with no streak, at
/// every `alpha`.
///
/// # Why read `&Transform` (not the packed `curr`) — the P1 fix
///
/// The old collapse wrote `prev = curr`. But `curr` is STALE when a teleport
/// flushes at Main's drain without a subsequent pack: the last fixed substep, a
/// Main-issued teleport, or a 0-substep frame all leave `curr` at the PRE-teleport
/// pose (the pack that would refresh it runs in Fixed, which did not run — or ran
/// before the enable landed). Collapsing to the stale `curr` would draw the body
/// at its OLD pose. Recomputing from the live `Transform`
/// (`TrsPacked::from_transform`) makes the collapse self-sufficient regardless of
/// pack timing — the drawn pose is always the post-teleport pose.
///
/// # The dense × enable-term composition
///
/// The query pairs a DENSE component (`&mut GpuTransform3D`) with the
/// [`Enabled<SnapInterpolation>`] enable filter. The kernel's dense-seed recull
/// bounds the driver to enable-column-bearing archetypes and the per-row
/// `filter_fetch` trims disabled rows, so the enabled dense rows iterate correctly.
/// `iter_entities_mut` is an archetype-walking cursor (NOT the dense `.get()` fast
/// path), so the per-row enable bit is honored and no dense null-deref is possible.
///
/// # Deferred disable timing (the pinned mechanism)
///
/// The `disable` is a deferred command — the bit clears at the next command-apply
/// window (after this system returns). So the SAME frame's gather still sees
/// `prev == curr` (this system already wrote it), and the NEXT frame's
/// [`pack_gpu_transforms`](crate::gpu_transform_pack) shuffle resumes normally
/// (the bit is clear by then). If the body teleports again, `teleport_to`
/// re-enables the bit and the cycle repeats.
///
/// # 0%-gate
///
/// A world where no body is enabled (the steady state) has an all-clear enable
/// column, so the system yields zero matching rows — zero per-entity work, no
/// command churn.
#[allow(clippy::needless_pass_by_value)]
pub fn snap_apply(
    mut commands: Commands,
    mut q: Query<(&Transform, &mut GpuTransform3D), Enabled<SnapInterpolation>>,
) {
    for (id, (transform, pair)) in q.iter_entities_mut() {
        // Recompute the collapsed pair from the LIVE Transform (the P1 fix):
        // curr = prev = from(Transform), so mix(prev, curr, alpha) == curr for
        // every alpha — the drawn pose is the post-teleport pose, no streak,
        // regardless of whether the pack refreshed `curr` this frame.
        let packed = TrsPacked::from_transform(transform);
        pair.curr = packed;
        pair.prev = packed;
        // Clear the bit for next frame (deferred; the same-frame gather already
        // reads the collapsed pair this system just wrote).
        commands.add(DisableSnapById { id });
    }
}

/// A deferred `disable::<SnapInterpolation>` keyed by [`EntityId`], mirroring
/// `boyko_scene`'s `SetRenderEnabledById` pattern (resolve the live full `Entity`
/// at apply time; a dead / stale id is a silent no-op — a despawn may legitimately
/// race the enqueued disable within the frame).
struct DisableSnapById {
    /// The flagged row's entity id, read from the matched archetype's id column.
    id: EntityId,
}

impl Command for DisableSnapById {
    fn apply(self, world: &mut EcsMaster) {
        // Resolve the live full `Entity` (current generation) at apply time; a
        // dead / stale id ⇒ silent no-op. `disable` clears the enable bit via the
        // live inland (never a captured enqueue-time row), so a swap-remove that
        // moved another entity is honored. This is the same `EntityId`-keyed
        // resolve-at-apply pattern `boyko_scene`'s `SetRenderEnabledById` uses (the
        // id column has no generation; the full `Entity` is the only safe key).
        let Some(entity) = world.get_entity(self.id) else {
            return;
        };
        world.disable::<SnapInterpolation>(entity);
    }
}

/// Deferred-command sugar for a teleport (host plan D4): the FIRST `EntityCommands`
/// extension trait in the repo. It must name both [`Transform`] (`boyko_scene`) and
/// [`SnapInterpolation`] (`boyko_render`), which the ECS core cannot depend up on —
/// hence an extension trait defined HERE (the crate that names both), not an
/// inherent `boyko_ecs` method.
pub trait TeleportCommandsExt {
    /// Teleports the entity to `transform`: writes the new [`Transform`] AND
    /// ENABLES [`SnapInterpolation`] in ONE deferred command window, so the pair's
    /// `prev = curr` collapse ([`snap_apply`]) lands the SAME frame the pose jumps
    /// — the body appears at the new pose with no interpolation streak.
    ///
    /// Both effects enqueue on the same [`EntityCommands`] queue and drain in one
    /// apply window, so a teleport never leaves the pose written without the snap
    /// bit (which would streak) or the bit set without the pose (a spurious snap).
    /// Chainable (`&mut self -> &mut Self`).
    fn teleport_to(&mut self, transform: Transform) -> &mut Self;
}

impl TeleportCommandsExt for EntityCommands<'_, '_> {
    #[inline]
    fn teleport_to(&mut self, transform: Transform) -> &mut Self {
        // The deferred Transform write (an InsertCommand — Transform is a
        // Component/Bundle) THEN the deferred enable of the snap bit: both land in
        // the same drain window, so the pose and the snap bit are never split.
        self.insert(transform);
        self.enable::<SnapInterpolation>();
        self
    }
}

// Layout pin (house style): an EnableTag carries no data — its bit is the whole
// datum. A silent widening would break the zero-sized-tag contract.
const _: () = assert!(size_of::<SnapInterpolation>() == 0);
