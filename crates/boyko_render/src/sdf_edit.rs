//! The R7 SDF instance path — the ECS-native authoring of the marcher's edit list.
//!
//! v1 = a few STATIC boot-time SDF primitives spawned as ECS entities, direct-marched
//! by the existing analytic marcher (NO brick bake). The scene edit list lives in the
//! ECS's own storage (principle 0): the per-entity [`SdfPrimitive`] component is the ONE
//! owner; [`SdfEditStaging`] is a REUSED gather scratch (like
//! [`LightTableStaging`](crate::light_system::LightTableStaging) — NOT a durable
//! side-store). [`collect_sdf_edits`] runs ONCE in the STARTUP schedule (after the
//! startup spawns populate the World), walking `Query<&SdfPrimitive>` into the inline
//! `[SdfEdit; MAX_SDF_EDITS]` scratch. The host then encodes + uploads that scratch into
//! the marcher's binding-0 edit-list SSBO exactly once, on the first frame, under the
//! `FrameWriteToken` (see [`upload_sdf_edit_list`](crate::upload::upload_sdf_edit_list)).
//!
//! # Why boyko_render (not boyko_scene / boyko_app)
//!
//! [`boyko_sdf_math`] is a true `no_std` leaf and cannot derive `Component`;
//! `boyko_scene`'s charter forbids render/SDF types; `boyko_app` must not define
//! per-entity GPU-data paths. `boyko_render` is the D1 data bridge that already owns
//! per-entity GPU-data components (`InstanceModelCol`, `GpuTransform3D`), so the
//! `SdfEdit`-carrying component + its gather belong here.
//!
//! # Scope (v1)
//!
//! Boot-static: the gather runs once, the write is one-shot, the frame loop adds
//! NOTHING (0 per-frame cost). DEFERRED (a separate campaign): dynamic per-frame edits
//! (ring-ify the edit list + a generation gate), Transform-driven position, the
//! streaming brick clipmap accelerator, and raising `MAX_SDF_EDITS`.

use boyko_ecs::ecs::core::app::{App, Plugin};
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_macros::{Component, Resource};

pub use boyko_sdf_math::{MAX_SDF_EDITS, SdfEdit, sdf_kind, sdf_op};

/// A single SDF primitive authored as an ECS entity — the R7 SDF-instance-path
/// component. Its PRESENCE routes the entity into the marcher's edit list
/// ([`collect_sdf_edits`] gathers `Query<&SdfPrimitive>`); an entity without it is not
/// an SDF occluder (capability = component presence).
///
/// `#[repr(transparent)]` over the 48-byte std430 [`SdfEdit`] — the component carries
/// the exact bytes the marcher's binding-0 edit-list SSBO reads, so the gather is a
/// pure copy (no repack). Author with the `SdfEdit` constructors, e.g.
/// `SdfPrimitive(SdfEdit::sphere([x, y, z], r, sdf_op::UNION, 0.0))`.
///
/// v1 stores WORLD-SPACE position in the `SdfEdit` directly (no `Transform` read);
/// Transform-driven position is a deferred campaign.
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug)]
pub struct SdfPrimitive(pub SdfEdit);

/// The reused SDF edit-list gather scratch (principle 0 — NOT a durable side-store).
///
/// `edits` is a fixed-capacity `[SdfEdit; MAX_SDF_EDITS]` (768 B inline — no allocation,
/// no `Vec`), refilled in place by [`collect_sdf_edits`]. `count` is the number of live
/// edits (`<= MAX_SDF_EDITS`); `dirty` is set when the gather produced any edit and
/// cleared by [`Self::mark_uploaded`] once the host has written the encoded list.
///
/// The authoritative store is the [`SdfPrimitive`] column; this scratch is a transient
/// staging buffer the host reads once to seed the marcher's SSBO.
#[derive(Resource)]
pub struct SdfEditStaging {
    /// The gathered edits (only `edits[..count]` are live). Inline fixed-capacity —
    /// the whole `[SdfEdit; MAX_SDF_EDITS]` is 768 B, never heap-allocated.
    edits: [SdfEdit; MAX_SDF_EDITS],
    /// Number of live edits in `edits` (`<= MAX_SDF_EDITS`).
    count: u32,
    /// Set by the gather when `count > 0`; cleared by [`Self::mark_uploaded`] after the
    /// host writes the encoded list into the marcher's SSBO.
    dirty: bool,
}

impl Default for SdfEditStaging {
    #[inline]
    fn default() -> Self {
        // An inert zero-radius union sphere fills every unused slot (never read past
        // `count`); an empty scratch matches the boot's empty edit-list seed exactly.
        let placeholder = SdfEdit::sphere([0.0, 0.0, 0.0], 0.0, sdf_op::UNION, 0.0);
        Self { edits: [placeholder; MAX_SDF_EDITS], count: 0, dirty: false }
    }
}

impl SdfEditStaging {
    /// The live edit slice (`edits[..count]`) — exactly what the host encodes into the
    /// marcher's binding-0 SSBO via [`encode_edit_list`](boyko_rhi_vulkan::compute::encode_edit_list).
    #[inline]
    pub fn edits(&self) -> &[SdfEdit] {
        &self.edits[..self.count as usize]
    }

    /// Whether a gathered-but-not-yet-uploaded edit list is pending (the host's
    /// first-frame one-shot write is gated on this).
    #[inline]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clears the dirty flag after the host has written the encoded list — the one-shot
    /// boot-static upload runs exactly once (v1: no re-gather, no ring).
    #[inline]
    pub fn mark_uploaded(&mut self) {
        self.dirty = false;
    }
}

/// Gathers every [`SdfPrimitive`] entity into the reused [`SdfEditStaging`] scratch —
/// the one-shot STARTUP-schedule gather (v1). Clears the count, walks the column, and
/// clamps to [`MAX_SDF_EDITS`] (excess primitives are silently dropped, matching the
/// shader's edit-count clamp); `dirty` is set iff any edit was gathered.
///
/// Runs ONCE (registered in the startup schedule by [`SdfPlugin`]), after the startup
/// spawns have populated the World — so the host's first-frame write sees the full boot
/// scene. v1 does NOT re-run per frame (the frame loop adds no SDF cost).
#[allow(clippy::needless_pass_by_value)]
pub fn collect_sdf_edits(edits: Query<&SdfPrimitive>, mut staging: ResMut<SdfEditStaging>) {
    let mut count = 0usize;
    for row in edits.iter() {
        if count < MAX_SDF_EDITS {
            staging.edits[count] = row.0;
            count += 1;
        }
    }
    debug_assert!(
        count <= MAX_SDF_EDITS,
        "invariant: gathered SDF edit count {count} exceeds MAX_SDF_EDITS {MAX_SDF_EDITS}"
    );
    staging.count = count as u32;
    staging.dirty = count > 0;
}

/// Composes the R7 SDF instance path: inserts the [`SdfEditStaging`] gather scratch.
///
/// # Why the gather is NOT registered as a startup system (the P0 order fix)
///
/// [`collect_sdf_edits`] must observe EVERY `SdfPrimitive` the user spawns — including
/// the ones spawned by systems registered via `App::add_startup_system` AFTER
/// `add_plugins(EnginePlugins)` runs. Startup systems drain in PUSH order in
/// `App::finish`, so a gather registered here (inside `SdfPlugin::build`, during
/// `add_plugins`) would run BEFORE the user's later `add_startup_system(setup)` — it
/// would see zero primitives, never mark the staging dirty, and nothing would upload
/// (the sphere would not render). Registration order is user-visible and fragile.
///
/// The host therefore runs [`collect_sdf_edits`] ONCE explicitly, AFTER `app.finish()`
/// drains ALL startup systems (so every `SdfPrimitive` is spawned) and BEFORE the frame
/// loop — an order-proof, single-site gather (see the runner). This plugin only inserts
/// the staging resource; the gather is the host's to schedule.
#[derive(Default)]
pub struct SdfPlugin;

impl Plugin for SdfPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(SdfEditStaging::default());
    }

    fn name(&self) -> &'static str {
        "boyko_render::SdfPlugin"
    }
}
