//! [`EnginePlugins`] — the host composition plugin (host plan D1/D6, R3).
//!
//! Composes the engine's windowed frame stack: the scene plugins (transform
//! propagation + camera resolution + visibility bridge via `CameraPlugin`,
//! the S4 3D pack via `Render3dPlugin`), the R3 mesh-draw pack + gather, the
//! D4 `FixedSet` ordering seam, and the windowed G-buffer runner.

use boyko_ecs::ecs::core::app::CoreSchedule;
use boyko_ecs::{App, Plugin};
use boyko_render::instance_model::sync_instance_model_cols;
// HW-RT rung 3b: the prev-frame model-affine copy system (temporal motion vectors), ordered
// `.before` the affine pack below. `not(hwrt)` never compiles it.
#[cfg(feature = "hwrt")]
use boyko_render::instance_model::sync_prev_instance_model_cols;
// HW-RT rung 3b step 5a: the persisted previous-frame camera view-proj (the motion-vector camera
// carry) — a `Resource` singleton the runner `advance`s each temporal frame. `not(hwrt)` never
// compiles it.
#[cfg(feature = "hwrt")]
use boyko_render::MotionCamState;
use boyko_render::light_system::LightTableStaging;
use boyko_render::{
    AssetRefcountPlugin, ClusterConfig, CsmCasterScratch, CsmFitSet, CsmPlugin, CsmResolveSet,
    LightCollectSet, LightingConfig, LightingPlugin, MeshRenderScratch, RayPlugin, Render3dPlugin,
    RenderPathPlugin, SdfPlugin, ShadowAtlasPlugin, ShadowDenoisePlugin, SsaoPlugin,
    add_gpu_transform_pack, gather_mesh_draws, gather_shadow_casters, reduce_caster_bounds,
    snap_apply, sync_cluster_light_gate, sync_csm_light_gate, sync_punctual_light_gate,
    sync_ssao_light_gate, sync_sv0_light_gate,
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
    /// SSAA (AA campaign Stage 3): the owner-requested render scale, default `0`
    /// (off — byte-identical to before SSAA existed). Set via [`Self::with_ssaa_scale`];
    /// `build` also honors `BOYKO_AA=ssaa` (the owner-eval channel) when this stays at
    /// its default. Only `2` is ever honored past `build` — the host's device-capability
    /// probe is the sole arming authority (see `crate::host::WindowHost::boot`).
    ssaa_scale: u32,
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
            ssaa_scale: 0,
        }
    }

    /// Requests SSAA (AA campaign Stage 3) at the given render scale — v1 honors ONLY
    /// `2` (2× per axis); any other value is clamped to off by the host's boot-time
    /// device-capability probe (`WindowHost::boot`), which is the sole arming authority
    /// (dims + VRAM budget) and NEVER panics on a device that cannot fit the request.
    #[inline]
    pub fn with_ssaa_scale(mut self, scale: u32) -> Self {
        self.ssaa_scale = scale;
        self
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

        // Asset-streaming plan F2: the refcount lifetime pipeline. Inserts
        // `RefcountDeltas`/`DeferredFree` and registers `apply_refcount_deltas`
        // (no ordering edge needed yet — see that system's doc). The `Assets<
        // MeshGpu>`/`Assets<Material>` resources it reads are inserted by
        // `runner::run_windowed` before the frame loop starts, well after this
        // `build()` call, so add-order here does not matter.
        app.add_plugin(AssetRefcountPlugin);

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
        // Render P7-Q2: `SsaoPlugin` — the SSAO quality config substrate. UNLIKE its old
        // config-only state, the windowed host now boots the SSAO pipeline/layout
        // (`gpu_scene::GpuSceneBundles::boot`) and arms `GBufferScene::ssao` from the
        // resolved selection (`boyko_app::runner`'s per-frame `World` read — the same
        // `try_resource` pattern `ResolvedAa` uses), so this is a LIVE consumer, mirroring
        // `ShadowDenoisePlugin`/`AaPlugin` below. The default `SsaoQuality::Off` keeps
        // every host world byte-identical (`scene.ssao` stays `None`, the resolve's
        // `ssao_mode` header gate stays 0), so composing it unconditionally is safe.
        app.insert_resource(LightTableStaging::default());
        app.insert_resource(LightingConfig::default());
        // VB-P1b-0: `ClusterConfig` is seeded HERE (mirrors `LightingConfig` immediately
        // above), not by `LightingPlugin`/any render-path plugin — it bridges the L1
        // froxel-cull grid/near/far parameters into the light-header pack via the
        // `sync_cluster_light_gate` bridge below, the SAME "composing app seeds it"
        // precedent `LightingConfig` itself follows. Default `16x9x24` dims / `0.1..50.0`
        // near/far (`ClusterConfig::default()`) — inert until a scene ALSO sets
        // `LightingConfig::clusters_enabled = true` (the 0%-gate: the sync gate zeros the
        // header's cluster lane whenever that bit is off, regardless of these dims).
        app.insert_resource(ClusterConfig::default());
        app.add_plugin(LightingPlugin);
        app.add_plugin(SsaoPlugin);
        app.add_plugin(CsmPlugin);
        // ShadowAtlasPlugin (the punctual host rung) — the spot/point analogue of CsmPlugin:
        // it seeds the owner-set `ShadowConfig` (default DISABLED — the 0%-gate; overwrite it
        // AFTER `add_plugins` to enable punctual shadows) + the derived `ResolvedShadowAtlas`,
        // and registers the cold `resolve_shadow_atlas` fit. Added HERE (after Csm, alongside
        // Camera/Lighting) so add-order places `resolve_shadow_atlas` before the host-closure
        // `sync_punctual_light_gate` reads its `mode_word` — the SAME cross-plugin add-order
        // discipline CsmPlugin documents (a `.after(resolve_shadow_atlas)` edge is not
        // expressible across plugins; a loose one-frame stagger off cold owner state is
        // self-correcting, and the default DISABLED config gates the whole path off).
        app.add_plugin(ShadowAtlasPlugin);

        // HW-RT rung R1 — the dormant unified ray / acceleration-structure seam.
        // RayPlugin seeds the derived `RayBackendConfig` carrier (default DISABLED —
        // every cell Software) + its `RayCaps` device-tier input (default `Absent`)
        // and schedules the cold `resolve_ray_backend_system` under `RayResolveSet`.
        // Dormant: the resolve is all-software for every tier, no pass reads the
        // config, and `RayResolveSet` has no command-recording consumer, so the
        // command stream is byte-identical. The runner OVERRIDES `RayCaps` at device
        // boot with the real `DeviceCaps::rt_tier()` query (still `Absent` in R1), at
        // the same site it fills `DdgiCaps`.
        app.add_plugin(RayPlugin);

        // HW-RT rung 3a — the spatial (à-trous) RT soft-shadow DENOISE config substrate.
        // `ShadowDenoisePlugin` inserts the author-set `ShadowDenoiseConfig` (default
        // `mode == None` — the 0%-gate) + its derived `ResolvedShadowDenoise` companion and
        // schedules the cold `resolve_shadow_denoise_policy` single-writer. Unlike step 1 (no
        // live pass) the host now has the denoise pass wired, so composing it here makes the
        // knob LIVE: the per-frame `scene.shadow` gate (gpu_scene::scene) reads
        // `ShadowDenoiseConfig::enabled()`; the à-trous UBO upload reads `ResolvedShadowDenoise`.
        // The default `None` keeps every host world byte-identical (the gate stays closed) — safe
        // to compose unconditionally, and the `BOYKO_SHADOW_DENOISE` boot knob flips it to
        // `Spatial` for a headless flight-check.
        app.add_plugin(ShadowDenoisePlugin);

        // Anti-aliasing Stage 1 — unlike `SsaoPlugin` above (deliberately NOT composed: the
        // windowed host has no SSAO pipeline/targets yet, so composing it would ship a
        // silently-dead knob), `AaPlugin` HAS a live consumer (the FXAA post-process pass
        // wired into `gpu_scene::scene`/`record_gbuffer`) — mirrors `ShadowDenoisePlugin`'s
        // live wiring. Injects `resolve_aa_policy` (reads `AaConfig`, writes `ResolvedAa`;
        // no render-resource conflict with any other system → scheduled independently).
        // The default `AaMode::Off` keeps every host world byte-identical (`scene.aa` stays
        // `None`), so composing it unconditionally is safe.
        app.add_plugin(boyko_render::AaPlugin);

        // Multi-paradigm render-path plan, rung R1 — `RenderPathPlugin`: seeds the owner-set
        // `RenderPathConfig` (default `Deferred + Both`, the byte-identity anchor) + its derived
        // `ResolvedRenderPath`. UNLIKE `AaPlugin`/`SsaoPlugin` above it registers NO per-frame
        // system (Decision 1 — path/legs are a ONE-TIME boot commitment, never re-derived per
        // frame); `boyko_app::runner` calls `resolve_render_path` directly at boot and overrides
        // this plugin's default, the SAME `DdgiCaps`/`RayCaps` override precedent. R1 is
        // dead-but-threaded: nothing downstream reads the resolved carrier yet.
        app.add_plugin(RenderPathPlugin);

        // Dev/test launch seam: `BOYKO_RENDER_PATH` / `BOYKO_GEOMETRY_LEGS` override the
        // `Deferred + Both` anchor `RenderPathPlugin` just seeded, so `scripts/run-scene.ps1` can
        // launch ANY windowed example in ANY paradigm without editing the scene. Runs DURING
        // `build()`, BEFORE any scene's own post-`add_plugins` `insert_resource(RenderPathConfig)`
        // (e.g. a golden test's explicit config), so an explicit choice still wins — a stray env
        // var never clobbers a pinned golden. `None` (both env vars unset — the golden-run case)
        // leaves the byte-identity anchor untouched.
        if let Some(cfg) = render_path_config_from_env() {
            app.insert_resource(cfg);
        }

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
        // HW-RT rung 3b step 5a: the persisted prev-frame camera view-proj (the motion-vector
        // camera carry). Inserted so the runner's `advance` (temporal frames only) finds it; a
        // `None` seed yields `prev == cur` on the first temporal frame (zero motion). Dormant until
        // the temporal denoiser is on (0%-gate). `not(hwrt)` never inserts it.
        #[cfg(feature = "hwrt")]
        app.insert_resource(MotionCamState::default());
        app.add_systems_cfg(|b| {
            let pack = b.add_system(sync_instance_model_cols).key();
            // HW-RT rung 3b: `prev := curr` MUST run BEFORE the affine pack refreshes `curr`
            // from this frame's moving `GlobalTransform`, so a mesh's motion vector is this
            // frame's true per-object displacement (else `prev == curr`, zero motion, every
            // box ghosts under its own motion). Dormant until a scene carries the
            // `PrevInstanceModelCol` column (0%-gate).
            #[cfg(feature = "hwrt")]
            b.add_system(sync_prev_instance_model_cols).before(pack);
            let casters = b.add_system(gather_shadow_casters).after(pack).key();
            b.add_system(sync_csm_light_gate).after(casters);
            // CSM auto-fit plan (`docs/CSM-AUTOFIT-PLAN.md`) rung C5: `reduce_caster_bounds`
            // is the UNWIRED EXPORTED API `CsmPlugin` deliberately does not register (mirrors
            // `gather_shadow_casters` itself) — this is the app that co-registers it. `.after
            // (casters)` folds THIS frame's finished gather output, not last frame's scratch
            // (D7 — `CsmCasterScratch` is single-writer, `gather_shadow_casters` owns it).
            // `.in_set(CsmFitSet)` gives `resolve_csm_cascades` (which joins `CsmResolveSet` in
            // `CsmPlugin`, csm_plugin.rs:76) something to order against below. Without this
            // registration `CsmCasterBounds` stays the `EMPTY` seed `CsmPlugin` inserts, so
            // every `CsmFitMode` renders as `Fixed` (D7/T15) — never a panic, always a no-op.
            b.add_system(reduce_caster_bounds).after(casters).in_set(CsmFitSet);
            // `CsmFitSet → CsmResolveSet`: `resolve_csm_cascades` must observe THIS frame's
            // folded bounds, not a one-frame-stale value (D11 — no accepted stagger, unlike
            // the cold-owner-state cross-plugin staggers documented elsewhere in this file).
            // Declared HERE (not inside `CsmPlugin`) because this closure is the first one that
            // gives `CsmFitSet` a member; a `configure_set` inside `CsmPlugin` alone would warn
            // W1501 (memberless set) in a bare-`CsmPlugin` world (D11). `App::add_systems_cfg`
            // threads the SAME Main builder through every closure/plugin (app.rs:313-319), so
            // this edge resolves against `CsmFitSet`'s membership above and `CsmResolveSet`'s
            // membership in `CsmPlugin` regardless of registration order.
            b.configure_set(CsmResolveSet).after(CsmFitSet);
            // The punctual header-gate ⇄ depth-pass lock-step (mirrors the csm sync): after the
            // SAME caster gather so the gate's caster predicate is THIS frame's. It reads
            // `ResolvedShadowAtlas.mode_word` (written by `resolve_shadow_atlas` in
            // ShadowAtlasPlugin, ordered earlier by add-order) — the resolve→sync edge follows the
            // same cross-plugin add-order discipline as csm (self-correcting under a one-frame lag,
            // gated off by the default DISABLED ShadowConfig).
            b.add_system(sync_punctual_light_gate).after(casters);
            // Render P7-Q2: the SSAO header-gate bridge — mirrors `sync_csm_light_gate`/
            // `sync_punctual_light_gate`'s cross-plugin registration (it bridges
            // `SsaoPlugin`'s `SsaoConfig` and `LightingPlugin`'s `LightingConfig`), but
            // reads `SsaoConfig` directly (no `ResolvedSsao`/caster dependency — mirrors
            // `sync_ddgi_light_gate`'s shape), so it carries no ordering edge here.
            b.add_system(sync_ssao_light_gate);
            // VB-SV0 rung S4: the SDF-on-mesh header-gate bridge — reads the boot-committed
            // `ResolvedRenderPath`, RESOLVES `LightingConfig`'s two SV0 request bits against
            // `vb_sdf_mesh_armable()`, and publishes the pair into the `_armed` fields the header
            // packer reads. Same cross-plugin registration rationale as the gates above (it
            // bridges `RenderPathPlugin`'s resolved carrier and `LightingPlugin`'s
            // `LightingConfig`). The resolve is monotone downward — `request && capability` — so
            // a world that never sets the request is untouched.
            //
            // It DOES carry an ordering edge (code-review P2-b), for the reason
            // `sync_cluster_light_gate` below carries one and `sync_ssao_light_gate` above does
            // not. Those two publish a bit that FOLLOWS an owner-set config, so an unordered fold
            // packs a value one frame late and self-corrects. This gate publishes the result of a
            // CAPABILITY resolve against a request the owner may set at any time: run after the
            // fold, the first armed frame packs the PRE-resolve `_armed` pair — the wrong state,
            // not a late one. `[vb_both_sdf]`-shaped fixtures dump a small fixed number of
            // frames, so "one frame" is a frame that can be the measured one. `LightCollectSet`
            // is the same by-name cross-plugin seam `sync_cluster_light_gate` uses (see its own
            // comment below for why `collect_lights`' `SystemKey` is not nameable here).
            b.add_system(sync_sv0_light_gate).before_set(LightCollectSet);
            // VB-P1b-0: the L1 cluster header-gate bridge — reads `ClusterConfig` directly (no
            // caster/resolved-carrier dependency, the SAME "no edge" shape `sync_ssao_light_gate`
            // above carries for ITS OWN inputs). `ClusterConfig` is seeded by THIS fn (mirrors
            // `LightingConfig` itself), so this bridge belongs alongside the other
            // `sync_*_light_gate`s in this SAME closure rather than inside `LightingPlugin`/any
            // render-path plugin.
            //
            // UNLIKE the sibling gates, this one DOES carry an explicit `.before_set` edge
            // (code-review C1): `sync_csm_light_gate`/`sync_ssao_light_gate` feed the fold with
            // only a SCALAR HEADER BIT, so a one-frame-stale read is merely a wrong bit (benign,
            // self-correcting). This gate feeds a GPU BUFFER INDEX (`cluster_packed_dims`): on the
            // very first frame `clusters_enabled` goes `true`, an unordered fold could pack
            // `clusters_enabled=1` with STALE/ZERO dims (this gate hasn't run yet that frame), and
            // the froxel resolve's `cluster_z_slice`/`cluster_linear_index` would then underflow to
            // an out-of-bounds `ClusterGrid` index. That WAS real GPU UB with
            // `robust_buffer_access` disabled (`device.rs`); as of VB-P1k all four `ClusterGrid`
            // readers reject a zero-dims (or over-capacity) header and fall back to the in-bounds
            // flat light scan, so the residue is a one-frame LIGHTING artefact rather than a
            // device fault — this edge is now a correctness edge, not the only line against UB,
            // and it stays for that reason. `.before_set(LightCollectSet)` is the SAME cross-plugin
            // by-name seam `resolve_shadow_atlas`/`PunctualResolveSet` uses (`collect_lights`'s
            // `SystemKey` is a closure-local in `LightingPlugin::build`, invisible here) — see
            // `LightCollectSet`'s own doc.
            b.add_system(sync_cluster_light_gate).before_set(LightCollectSet);
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

        // SSAA (AA campaign Stage 3): the explicit builder wins when `>= 2`; otherwise
        // `BOYKO_AA=ssaa` (the owner-eval channel, same env family the AA framework
        // already reserves) requests the v1 default of `2`. Any other value the host
        // does not honor (only `2` arms — see `WindowHost::boot`), so passing it through
        // unclamped here is harmless: the host's device-capability probe is the sole
        // arming authority.
        let ssaa_scale = if self.ssaa_scale >= 2 {
            self.ssaa_scale
        } else if std::env::var("BOYKO_AA").as_deref() == Ok("ssaa") {
            2
        } else {
            0
        };
        let desc = WindowDesc {
            title: self.title,
            width: self.width,
            height: self.height,
            ssaa_scale,
        };
        app.set_runner(Box::new(move |app: &mut App| {
            runner::run_windowed(app, desc)
        }));
    }

    fn name(&self) -> &'static str {
        "boyko_app::EnginePlugins"
    }
}

/// Parses the `BOYKO_RENDER_PATH` / `BOYKO_GEOMETRY_LEGS` dev/test launch env vars into a
/// [`boyko_render::RenderPathConfig`] override (Multi-paradigm render-path plan — the launcher
/// seam `scripts/run-scene.ps1` drives).
///
/// Returns `None` when BOTH are unset (the common/golden case), so [`EnginePlugins::build`] leaves
/// `RenderPathPlugin`'s `Deferred + Both` byte-identity anchor exactly as seeded. When EITHER is
/// set, the unset axis keeps its anchor value (path→`Deferred`, legs→`Both`). Values are
/// case-insensitive and accept friendly aliases; an unrecognized value falls back to that axis's
/// anchor with an `eprintln` diagnostic (never a panic — a mistyped var must not crash the app).
fn render_path_config_from_env() -> Option<boyko_render::RenderPathConfig> {
    use boyko_render::{GeometryLegs, RenderPath};

    let path_var = std::env::var("BOYKO_RENDER_PATH").ok();
    let legs_var = std::env::var("BOYKO_GEOMETRY_LEGS").ok();
    if path_var.is_none() && legs_var.is_none() {
        return None;
    }

    let path = match path_var.as_deref().map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        None | Some("deferred") => RenderPath::Deferred,
        Some("forward") => RenderPath::Forward,
        Some("forwardplus" | "forward+" | "forward_plus" | "clustered") => RenderPath::ForwardPlus,
        Some("visibilitybuffer" | "vb" | "visibility_buffer" | "visbuffer") => {
            RenderPath::VisibilityBuffer
        }
        Some(other) => {
            eprintln!(
                "[boyko] BOYKO_RENDER_PATH='{other}' unrecognized -> Deferred (valid: deferred|forward|forwardplus|vb)"
            );
            RenderPath::Deferred
        }
    };

    let legs = match legs_var.as_deref().map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        None | Some("both") => GeometryLegs::Both,
        Some("mesh") => GeometryLegs::Mesh,
        Some("sdf") => GeometryLegs::Sdf,
        Some(other) => {
            eprintln!("[boyko] BOYKO_GEOMETRY_LEGS='{other}' unrecognized -> Both (valid: both|mesh|sdf)");
            GeometryLegs::Both
        }
    };

    eprintln!("[boyko] render-path override from env: {path:?} x {legs:?}");
    Some(boyko_render::RenderPathConfig { path, legs })
}
