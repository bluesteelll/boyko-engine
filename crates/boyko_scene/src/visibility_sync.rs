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
//! # Why a custom by-id command (deviation from `EntityCommands::enable`)
//!
//! `EntityCommands::enable::<T>()` / `disable::<T>()` (and the underlying
//! `EnableTagCommand`) are keyed by a full
//! [`Entity`](boyko_ecs::ecs::core::entity::entity::Entity) (id + generation) — the
//! apply-time `live_inland` resolve rejects a generation mismatch. A read-only
//! query only exposes per-row [`EntityId`]s
//! ([`Query::iter_entities`](boyko_ecs::ecs::core::iters::query::Query::iter_entities)
//! yields `(EntityId, _)`; there is no `QueryData for Entity` and no
//! world-resolving `SystemParam`), so the system cannot reconstruct the correct
//! generation at enqueue time. [`SetRenderEnabledById`] therefore carries the
//! `EntityId` and resolves the live full `Entity` at apply (under `&mut
//! EcsMaster`, where the current generation is authoritative) via
//! [`EcsMaster::get_entity`], then delegates to the same public
//! [`EcsMaster::enable`] / [`EcsMaster::disable`] direct API the
//! `EntityCommands` path ultimately calls — preserving the deferred semantics and
//! the dead/stale no-op contract.
//!
//! [`Visibility`]: crate::render_caps::Visibility
//! [`RenderEnabled`]: crate::render_caps::RenderEnabled

use boyko_ecs::ecs::core::commands::Command;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::{Changed, Query};
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::identifiers::primitives::EntityId;

use crate::render_caps::{RenderEnabled, Visibility};

/// Deferred toggle of the [`RenderEnabled`] bit, keyed by [`EntityId`].
///
/// Enqueued by [`visibility_sync`] and flushed by the command queue under
/// exclusive `&mut EcsMaster`. The full [`Entity`](boyko_ecs::ecs::core::entity::entity::Entity)
/// (with the authoritative current generation) is resolved at apply via
/// [`EcsMaster::get_entity`]; a dead / stale id is a silent no-op (a despawn may
/// legitimately race an enqueued toggle within the same frame — the same
/// contract as the kernel's `EnableTagCommand`).
///
/// # Layout
///
/// ```text
/// +0  : id: EntityId   (8 B — usize on 64-bit)
/// +8  : value: bool    (1 B — true = enable, false = disable)
/// ```
#[repr(C)]
struct SetRenderEnabledById {
    /// The row's entity id, read from the matched archetype's entity-id column.
    id: EntityId,
    /// `true` ⇒ enable (`Visible` / `Inherited`); `false` ⇒ disable (`Hidden`).
    value: bool,
}

impl Command for SetRenderEnabledById {
    fn apply(self, world: &mut EcsMaster) {
        // Resolve the live full `Entity` (current generation) at apply time; a
        // dead / stale id ⇒ silent no-op. The bit op resolves the row via the
        // live inland inside `enable` / `disable` (never a captured enqueue-time
        // row), so a swap-remove that moved another entity is honored.
        let Some(entity) = world.get_entity(self.id) else {
            return;
        };
        if self.value {
            world.enable::<RenderEnabled>(entity);
        } else {
            world.disable::<RenderEnabled>(entity);
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
/// # Add-order contract (cross-schedule ordering vs. the render pack)
///
/// The toggle is **deferred**: the bit flips at the next command-apply window,
/// after this system's body returns. For a `Hidden` entity to be excluded from
/// the pack the frame after its byte changes, `visibility_sync` (and its apply
/// window) must run BEFORE
/// [`sync_gpu_3d_instances`](../../boyko_render/gpu3d_system/fn.sync_gpu_3d_instances.html),
/// which filters on `Enabled<RenderEnabled>`. That edge cannot be expressed in
/// `boyko_scene` (the pack system's `SystemKey` lives in `boyko_render`'s
/// `Render3dPlugin`), so — exactly as `Render3dPlugin` / `LightingPlugin`
/// document — **add `Render3dPlugin` together with `TransformPlugin` or
/// `CameraPlugin`** so the host schedule runs propagation + this sync first and
/// the pack last. The `Changed`-driven gate makes a loose one-frame stagger
/// self-correcting (a missed toggle re-fires the frame after the byte changes).
///
/// Within `boyko_scene` the system is registered `.after(propagate_transforms)`
/// (see `TransformPlugin` / `CameraPlugin`) to keep the documented per-frame
/// chain coherent — it has no data dependency on propagation (distinct columns),
/// but ordering it after propagation and before the pack keeps the
/// authoring-intent → effective-pose → GPU-pack order intuitive.
#[allow(clippy::needless_pass_by_value)]
pub fn visibility_sync(
    mut commands: Commands,
    q: Query<&Visibility, Changed<Visibility>>,
) {
    for (id, vis) in q.iter_entities() {
        // `Inherited` is treated as visible at the entity level (hierarchical
        // propagation is deferred — see the module docs).
        let value = !matches!(vis, Visibility::Hidden);
        commands.add(SetRenderEnabledById { id, value });
    }
}
