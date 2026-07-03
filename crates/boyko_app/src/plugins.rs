//! [`EnginePlugins`] — the host composition plugin (host plan D1/D6, R3).
//!
//! Composes the engine's windowed frame stack: the scene plugins (transform
//! propagation + camera resolution + visibility bridge via `CameraPlugin`,
//! the S4 3D pack via `Render3dPlugin`), the R3 mesh-draw pack + gather, the
//! D4 `FixedSet` ordering seam, and the windowed G-buffer runner.

use boyko_ecs::ecs::core::app::CoreSchedule;
use boyko_ecs::{App, Plugin};
use boyko_render::instance_model::sync_instance_model_cols;
use boyko_render::light_system::LightTableStaging;
use boyko_render::{
    CsmCasterScratch, CsmPlugin, LightingConfig, LightingPlugin, MeshRenderScratch,
    Render3dPlugin, SdfPlugin, add_gpu_transform_pack, gather_mesh_draws, gather_shadow_casters,
    snap_apply, sync_csm_light_gate,
};
use boyko_scene::{CameraPlugin, FixedSet};

use crate::runner::{self, WindowDesc};

/// The engine host plugin: composes the scene/render frame systems, wires the
/// D4 `FixedSet` ordering seam, opens a window, and installs the windowed
/// G-buffer runner (device-singleton boot, token-fenced uploads, the
/// production `render_gbuffer_frame`, D2 teardown) via `App::set_runner`.
///
/// # Composition (add-order discipline)
///
/// `EnginePlugins` adds [`CameraPlugin`] (which owns `propagate_transforms` +
/// `resolve_active_camera` + `visibility_sync` with their ordering edges) and
/// [`Render3dPlugin`], then registers the R3 mesh path —
/// `sync_instance_model_cols` → `gather_mesh_draws` (edge-ordered) — AFTER
/// them. The propagation → pack edge cannot be expressed explicitly
/// (`propagate_transforms`'s `SystemKey` is only obtainable inside
/// `CameraPlugin`'s own builder closure), so it is pinned by the documented
/// cross-crate ADD-ORDER contract — and unlike the `Changed`-gated systems
/// that contract usually covers, `sync_instance_model_cols` is UNCONDITIONAL:
/// a wrong order would be a PERMANENT one-frame pose lag, not a
/// self-correcting stagger. The add-order here IS the pin; do not reorder.
/// Do NOT also add `CameraPlugin` / `TransformPlugin` / `Render3dPlugin` /
/// `LightingPlugin` / `CsmPlugin` yourself — a duplicate plugin panics.
///
/// # Lighting (host plan R4)
///
/// `EnginePlugins` composes [`LightingPlugin`] (light reconcile + table
/// collection + the eviction hooks — so no light component may be archetyped
/// before this plugin is added; spawn lights from startup systems) and
/// [`CsmPlugin`] (the owner-set [`CsmConfig`](boyko_render::CsmConfig), default
/// DISABLED — overwrite it after `add_plugins` to enable sun shadows). Entities
/// carrying `ShadowCaster` cast into the cascades; receiver-only meshes (floors,
/// walls) simply omit the marker. The runner uploads the reconciled light table
/// through the D5 generation protocol and arms the cascade depth pass when a
/// fitted sun and live casters exist.
///
/// # The D4 seam + interpolation (host plan R5)
///
/// Wires `FixedSet::Snapshot.after(FixedSet::Gameplay)` in `CoreSchedule::Fixed`
/// and joins `pack_gpu_transforms` to `FixedSet::Snapshot` — put Fixed gameplay
/// `.in_set(FixedSet::Gameplay)` and the per-substep prev/curr shuffle observes
/// the substep's FINAL pose (no one-substep lag). The Main-schedule
/// `snap_apply` → `gather_mesh_draws` unified path feeds the runner's interp
/// pre-pass; a body opts into interpolation by carrying `GpuTransform3D`, and
/// teleports it with [`teleport_to`](boyko_render::TeleportCommandsExt::teleport_to)
/// (which snaps `prev = curr` for one frame — no streak).
///
/// # Windowed host v1 = PERSPECTIVE cameras only
///
/// The host's camera/raster pushes are the perspective marcher convention. An
/// Orthographic active camera carries the `fov_y == 0` sentinel
/// ([`Projection::fov_y`](boyko_scene::Projection::fov_y)) and DEGRADES to a
/// background-only frame (the marcher takes its frozen ORTHO fixture path and
/// the raster push is zeroed — nothing draws, nothing panics). The sentinel
/// is kept deliberately; an ortho windowed path is a later rung.
///
/// ```no_run
/// use boyko_app::prelude::*;
///
/// let mut app = App::new();
/// app.add_plugins(EnginePlugins::window("my game", 800, 600));
/// app.run();
/// ```
pub struct EnginePlugins {
    /// The window caption.
    title: &'static str,
    /// Requested client-area width in pixels.
    width: u32,
    /// Requested client-area height in pixels.
    height: u32,
}

impl EnginePlugins {
    /// A windowed host with the given caption and requested client size.
    ///
    /// The composite (render) extent is fixed at boot from the ACTUAL client
    /// size the window comes up at (plan D7); a later window resize recreates
    /// the swapchain only and the present blit clamps. BOTH per-frame camera
    /// pushes (the marcher's b5 block and the raster `view_proj`) derive their
    /// aspect from that boot-fixed composite extent — the authored
    /// `Projection` aspect is NOT consulted by the windowed host's pushes (it
    /// still shapes `ViewUniform::view_proj` for non-host consumers), so the
    /// two can never diverge even when the OS adjusts the client size.
    /// Dynamic aspect/extent tracking is v2.
    #[inline]
    pub fn window(title: &'static str, width: u32, height: u32) -> Self {
        Self {
            title,
            width,
            height,
        }
    }
}

impl Plugin for EnginePlugins {
    /// Composes the frame systems + the D4 seam, then installs the windowed
    /// runner. `App::run` hands the runner control BEFORE `finish()`; the
    /// runner owns the app lifecycle from there (its own `finish()` call,
    /// `AppExit` policy, and teardown — see `runner.rs`).
    fn build(&self, app: &mut App) {
        // Scene stack: propagation + camera resolve + visibility bridge
        // (CameraPlugin SUPERSEDES TransformPlugin — adding both would
        // double-register propagation), then the S4 3D instance pack.
        app.add_plugin(CameraPlugin);
        app.add_plugin(Render3dPlugin);

        // The R4 lighting stack. LightingPlugin registers the light eviction
        // hooks as its FIRST action, inheriting its registration-first
        // invariant: no light component may be archetyped before
        // `EnginePlugins` is added (spawn lights from startup systems — they
        // drain after `finish()`, well past this build). The staging + config
        // inserts mirror the production wiring the lighting suite pins
        // (`le_support::lighting_app`); `LightTableGeneration` /
        // `LightTableDirty` are inserted by the plugin itself. CsmPlugin seeds
        // the owner-set `CsmConfig` (default DISABLED — the 0%-gate; overwrite
        // it AFTER `add_plugins` to enable sun shadows) + the derived
        // `ResolvedCsm` its per-frame camera-fit policy writes. Add-order
        // contract honored: LightingPlugin lands together with CameraPlugin
        // (propagation before reconcile), CsmPlugin after both (camera resolve
        // + sun reconcile before the cascade fit) — all Changed-gated, so the
        // cross-plugin stagger is self-correcting per their type-level docs.
        //
        // SSAO is deliberately NOT composed: `SsaoPlugin` is config-only (no
        // GPU cost at boot), but the windowed host creates no SSAO pipeline /
        // targets yet, so composing it would ship a silently-dead
        // `SsaoConfig` knob. It lands together with the host SSAO pass.
        app.insert_resource(LightTableStaging::default());
        app.insert_resource(LightingConfig::default());
        app.add_plugin(LightingPlugin);
        app.add_plugin(CsmPlugin);

        // The R7 SDF instance path (composed by DEFAULT): inserts the
        // `SdfEditStaging` gather scratch and registers the one-shot startup
        // `collect_sdf_edits` gather. An entity carrying `SdfPrimitive` is direct-
        // marched into the shared G-buffer; a scene with NO `SdfPrimitive` gathers
        // zero edits, so the marcher's edit list stays the empty boot seed (the
        // 0%-gate — byte-identical to pre-R7). The runner performs the one-shot
        // boot-static edit-list upload on the first frame under the write token.
        app.add_plugin(SdfPlugin);

        // The R3 mesh path: pack GlobalTransform → InstanceModelCol, then
        // bucket the visible instances into the reused MeshRenderScratch the
        // runner uploads from. The pack → gather edge is explicit; the
        // propagation → pack edge is the ADD-ORDER pin above (the pack is
        // UNCONDITIONAL, so a wrong order would be a permanent one-frame pose
        // lag — see the type-level Composition doc).
        //
        // R4 adds the caster half in the SAME closure so its edges are
        // expressible: `gather_shadow_casters` (the `With<ShadowCaster>`
        // production gather) runs after the pack, and `sync_csm_light_gate`
        // (the header-gate ⇄ depth-pass lock-step) after the caster gather, so
        // the gate's caster predicate is THIS frame's.
        // R5 adds the INTERPOLATION Main system `snap_apply` (the zero-streak
        // collapse for teleported bodies) in the SAME closure. Refined-B unifies
        // the two former gathers into ONE `gather_mesh_draws` over ALL drawables
        // (static + interpolated), so `snap_apply` must run BEFORE it: the collapsed
        // `curr == prev` a teleport lands is what the unified gather reads into the
        // pair lanes THIS frame. The single gather runs `.after(pack)` (the affine
        // pack — add-order cross-schedule note above) AND `.after(snap)`; it emits
        // ONE batch list + ONE ring, recording each interpolated row's pair +
        // out-slot, so the runner arms interp only when `dynamic_count() > 0`.
        app.insert_resource(MeshRenderScratch::default());
        app.insert_resource(CsmCasterScratch::default());
        app.add_systems_cfg(|b| {
            let pack = b.add_system(sync_instance_model_cols).key();
            let casters = b.add_system(gather_shadow_casters).after(pack).key();
            b.add_system(sync_csm_light_gate).after(casters);
            // The unified gather runs after BOTH the affine pack and the snap
            // collapse (snap-before-gather is load-bearing — the gather reads the
            // collapsed pair).
            let snap = b.add_system(snap_apply).key();
            b.add_system(gather_mesh_draws).after(pack).after(snap);
        });

        // The D4 ordering seam: engine Fixed snapshots run AFTER user Fixed
        // gameplay, pinned BY NAME (no topological accident). R5 makes the seam
        // REAL — `pack_gpu_transforms` joins `FixedSet::Snapshot` (retiring the
        // memberless-set W1501 warning): its `.in_set(Snapshot)` membership +
        // the `configure_set(Snapshot).after(Gameplay)` edge pin it AFTER every
        // user Fixed gameplay system (which joins `FixedSet::Gameplay`), so the
        // prev/curr shuffle observes the substep's FINAL pose (no one-substep
        // lag). The engine composes no physics here, so there is no
        // `sync_body_to_transform` key to name — the set-level edge is the whole
        // ordering contract for the windowed host.
        app.add_systems_cfg_in(CoreSchedule::Fixed, |b| {
            b.configure_set(FixedSet::Snapshot).after(FixedSet::Gameplay);
            add_gpu_transform_pack(b).in_set(FixedSet::Snapshot);
        });

        let desc = WindowDesc {
            title: self.title,
            width: self.width,
            height: self.height,
        };
        app.set_runner(Box::new(move |app: &mut App| {
            runner::run_windowed(app, desc)
        }));
    }

    fn name(&self) -> &'static str {
        "boyko_app::EnginePlugins"
    }
}
