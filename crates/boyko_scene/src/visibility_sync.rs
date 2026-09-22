//! The `visibility_sync` system (standard-library Phase S4 follow-up) — the
//! bridge that drives the per-frame [`RenderEnabled`] draw bit from the durable
//! [`Visibility`] byte.
//!
//! # The two surfaces it bridges
//!
//! S4 shipped a **two-surface** visibility model (see
//! [`render_caps`](crate::render_caps)):
//!
//! * [`Visibility`] — a `#[repr(u8)]` per-row byte (`Inherited = 0`,
//!   `Visible = 1`, `Hidden = 2`): the **persisted authoring intent**, the
//!   source of truth that survives serialization.
//! * [`RenderEnabled`] — an `EnableTag` bitset bit: the **O(1) per-frame draw
//!   toggle** the 3D instance pack
//!   ([`sync_gpu_3d_instances`](../../boyko_render/gpu3d_system/fn.sync_gpu_3d_instances.html))
//!   filters on (`Enabled<RenderEnabled>`).
//!
//! In bare S4 the bridge was **manual** — setting `Visibility::Hidden` alone did
//! NOT hide a row; the user also had to call `disable::<RenderEnabled>()`.
//! [`visibility_sync`] closes that gap: a `Changed<Visibility>`-gated system that
//! drives the bit from the byte through **deferred commands**.
//!
//! # Mapping
//!
//! * `Visibility::Hidden`               ⇒ `disable::<RenderEnabled>()`
//! * `Visibility::Visible` / `Inherited` ⇒ `enable::<RenderEnabled>()`
//!
//! # DEFERRED: `InheritedVisibility` propagation is out of scope
//!
//! `Inherited` is treated as **visible at the entity level** here. True
//! parent-effective visibility — propagating a hidden ancestor down the
//! `ChildOf` tree so an `Inherited` child of a `Hidden` parent is itself hidden
//! (Bevy's `InheritedVisibility` / `ViewVisibility` pass) — is a SEPARATE, larger
//! feature and is **explicitly deferred**. This system only reflects each
//! entity's OWN byte; a hierarchical propagation pass would layer on top of it
//! (computing an effective `Visibility` per entity, which this system would then
//! sync) without changing the bridge below.
//!
//! # Why a custom command keyed by the full `Entity` (deviation from `EntityCommands::enable`)
//!
//! `EntityCommands::enable::<T>()` / `disable::<T>()` (and the underlying
//! `EnableTagCommand`) are keyed by a full [`Entity`] (id + generation) — the
//! apply-time `live_inland` resolve rejects a generation mismatch. A read-only
//! query only exposes per-row [`EntityId`]s
//! ([`Query::iter_entities`](boyko_ecs::ecs::core::iters::query::Query::iter_entities)
//! yields `(EntityId, _)`), so the system resolves the handle itself, at
//! enqueue, through the [`Entities`] param (KE2): one slot read per changed row,
//! on the worker. A row the query yields is live for the whole system body
//! (SCH7 — no structural mutation runs while a body does), so the resolved
//! handle carries the generation that is current when the toggle is enqueued.
//! `SetRenderEnabled` carries that `Entity` and delegates at apply to the same
//! public [`EcsMaster::enable`] / [`EcsMaster::disable`] direct API the
//! `EntityCommands` path ultimately calls — the deferred semantics and the
//! dead/stale no-op contract are the kernel's own.
//!
//! ## Hazard H-06 — why the generation must travel with the toggle
//!
//! The first shipped form carried the bare `EntityId` and resolved "whatever is
//! live at that id" at apply (`EcsMaster::get_entity`) — hazard H-06 of the
//! unification plan (02, row A9). A despawn of `E` and a spawn of `F` on `E`'s
//! recycled id in the SAME frame, drained before `E`'s pending toggle, made
//! the toggle land on `F`: a `Hidden` `F` was drawn for a frame. With the
//! generation in the command, that toggle is stale at apply and
//! `EcsMaster::enable` / `disable` drop it in their `live_inland` resolve —
//! the one slot read the apply costs; a dead `E` (a despawn racing an
//! enqueued toggle) is the same silent no-op it always was. The gate is
//! `visibility_sync_gates::toggle_pending_for_a_despawned_entity_does_not_land_on_its_recycled_id`.
//!
//! [`Visibility`]: crate::render_caps::Visibility
//! [`RenderEnabled`]: crate::render_caps::RenderEnabled
//! [`Entity`]: boyko_ecs::ecs::core::entity::entity::Entity
//! [`EntityId`]: boyko_ecs::ecs::identifiers::primitives::EntityId
//! [`Entities`]: boyko_ecs::ecs::core::system::Entities

use boyko_ecs::ecs::core::commands::Command;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::{Changed, Query};
use boyko_ecs::ecs::core::system::{Commands, Entities};

use crate::render_caps::{RenderEnabled, Visibility};

/// Deferred toggle of the [`RenderEnabled`] bit, keyed by the full
/// [`Entity`] (id + generation) that was live when it was enqueued.
///
/// Enqueued by [`visibility_sync`] and flushed by the command queue under
/// exclusive `&mut EcsMaster`. At apply the handle is resolved by
/// [`EcsMaster::enable`] / [`EcsMaster::disable`] themselves (`live_inland`: one
/// slot read, null and generation checked): a dead entity, or an id that a
/// later spawn re-occupied under a bumped generation (H-06), no longer matches
/// and the toggle is dropped — the same contract as the kernel's
/// `EnableTagCommand`. A despawn may legitimately race an enqueued toggle
/// within the same frame.
///
/// # Layout
///
/// ```text
/// +0  : entity: Entity (16 B — usize id + u32 generation + pad)
/// +16 : value: bool    (1 B — true = enable, false = disable)
/// +24 : end (padded)
/// ```
#[repr(C)]
struct SetRenderEnabled {
    /// The row's entity, resolved from the matched archetype's entity-id
    /// column through [`Entities`] at enqueue.
    entity: Entity,
    /// `true` ⇒ enable (`Visible` / `Inherited`); `false` ⇒ disable (`Hidden`).
    value: bool,
}

impl Command for SetRenderEnabled {
    fn apply(self, world: &mut EcsMaster) {
        // No resolve of our own: `enable` / `disable` read the slot once and
        // drop a dead or generation-stale handle there. The bit op resolves
        // the row via the live inland (never a captured enqueue-time row), so a
        // swap-remove that moved another entity is honored.
        if self.value {
            world.enable::<RenderEnabled>(self.entity);
        } else {
            world.disable::<RenderEnabled>(self.entity);
        }
    }
}

/// Drives the [`RenderEnabled`] draw bit from the durable [`Visibility`] byte —
/// the S4-follow-up bridge by which `Visibility::Hidden` actually hides.
///
/// `Changed<Visibility>`-gated: the system visits ONLY rows whose `Visibility`
/// was added or mutated since it last ran, so a frame in which no `Visibility`
/// changed does **zero per-entity work** (no command churn, no allocation) —
/// the 0%-overhead property. On spawn, the freshly-added `Visibility` is
/// `Changed`, so a new entity's bit is reconciled to its byte on the first run
/// after spawn (no explicit toggle needed).
///
/// Because the gate is `Changed<Visibility>`, an explicit manual
/// `enable`/`disable::<RenderEnabled>()` on an entity whose `Visibility` did NOT
/// change is left untouched — the system does not fight a manual override.
///
/// # Ordering contract (vs. every `RenderEnabled` reader)
///
/// The toggle is **deferred**: the bit flips at the next command-apply window,
/// after this system's body returns. For the bit a reader sees to be THIS frame's,
/// `visibility_sync` (and its apply window) must run BEFORE every system that
/// filters on `Enabled<RenderEnabled>` — the instance packs
/// ([`sync_gpu_3d_instances`](../../boyko_render/gpu3d_system/fn.sync_gpu_3d_instances.html)
/// among them) and the mesh and shadow-caster gathers.
/// Their `SystemKey`s live in `boyko_render`'s plugins, so the edges are pinned by
/// name: this system joins [`VisibilitySet::Sync`](crate::sets::VisibilitySet::Sync),
/// the readers join [`VisibilitySet::Read`](crate::sets::VisibilitySet::Read), and the
/// asset-ref validation — which neither reads nor writes this bit since the
/// asset-validate prerequisites, but writes the `RenderStale` / `MaterialStale` bits
/// the gathers ALSO filter on, through a command of its own — joins
/// [`VisibilitySet::Validate`](crate::sets::VisibilitySet::Validate). The composing host
/// configures `Read.after(Sync)`, `Read.after(Validate)` and `Validate.after(Sync)`
/// (`boyko_app::EnginePlugins` does; the last edge carries no data today and stays as
/// the declared phase order, pending the host owner's ruling).
/// The `Changed` gate does NOT make a missing edge self-correcting on a
/// spawn: a reader that runs first sees the bit clear on the entity's first frame,
/// and in the shipped host that was a frame 0 with no meshes drawn.
///
/// Within `boyko_scene` the system is registered `.after(propagate_transforms)`
/// (see `TransformPlugin` / `CameraPlugin`) to keep the documented per-frame
/// chain coherent — it has no data dependency on propagation (distinct columns),
/// but ordering it after propagation and before the pack keeps the
/// authoring-intent → effective-pose → GPU-pack order intuitive.
#[allow(clippy::needless_pass_by_value)]
pub fn visibility_sync(
    mut commands: Commands,
    entities: Entities,
    q: Query<&Visibility, Changed<Visibility>>,
) {
    for (id, vis) in q.iter_entities() {
        // A row the query yields is live for the whole body (SCH7), so the
        // resolve cannot miss; the handle it returns carries the CURRENT
        // generation, which is what lets apply tell this entity from a later
        // occupant of the same id (H-06).
        let entity = entities.get(id);
        debug_assert!(
            entity.is_some(),
            "invariant SCH7: a row yielded by the query resolves to a live entity"
        );
        let Some(entity) = entity else { continue };
        // `Inherited` is treated as visible at the entity level (hierarchical
        // propagation is deferred — see the module docs).
        let value = !matches!(vis, Visibility::Hidden);
        commands.add(SetRenderEnabled { entity, value });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the layout the struct doc and the A9 cost statement rely on:
    /// `Entity` (16 B) + `bool`, `#[repr(C)]` ⇒ 24 B, 8-aligned — 8 B more per
    /// toggle than the 16 B by-id form it replaced.
    #[test]
    fn set_render_enabled_is_24_bytes_8_aligned() {
        assert_eq!(
            size_of::<SetRenderEnabled>(),
            24,
            "Entity (16 B) + bool (1 B), repr(C), padded to the 8 B alignment"
        );
        assert_eq!(
            align_of::<SetRenderEnabled>(),
            8,
            "alignment follows Entity's usize id"
        );
    }
}
