//! The windowed G-buffer runner (host plan D6, R3).
//!
//! Installed by [`EnginePlugins`](crate::plugins::EnginePlugins) via
//! `App::set_runner`. Owns the whole app lifecycle: the device-singleton boot,
//! the window host boot, the World's GPU residents, `finish()`, the frame
//! loop (token-fenced camera + instance uploads → the production
//! `render_gbuffer_frame`), and the D2 teardown — in that order, by
//! construction.

use boyko_ecs::{App, AppExit};

#[cfg(windows)]
use boyko_ecs::ecs::core::time::FixedTime;

#[cfg(windows)]
use std::time::Instant;

// The OS→ECS input bridge helper (host plan R6) is source-agnostic (pure
// translation + `push_raw`, no FFI, no GPU) so it is defined un-gated and unit-
// testable on every host. `CapturedMsg` (the renderer's drained-message enum)
// and the `boyko_input` translate fns both compile cross-platform.
use boyko_input::{RawInputQueue, translate_win32, translate_win32_raw_mouse};
use boyko_rhi_vulkan::window::CapturedMsg;

#[cfg(windows)]
use boyko_ecs::ecs::core::asset::{AssetPaths, AssetServer, AssetStaging, Assets};
#[cfg(windows)]
use boyko_input::{ButtonState, KeyCode, RawInputEvent};
#[cfg(windows)]
use boyko_render::light_system::{LightTableGeneration, LightTableStaging};
// Read only by the `#[cfg(debug_assertions)]` textured-SDF guard in the frame loop, so a
// release build would see this import as unused.
#[cfg(all(windows, debug_assertions))]
use boyko_render::MATERIAL_FLAG_TEXTURED;
#[cfg(windows)]
use boyko_render::{
    AaConfig, AaMode, BindlessTextureTable, CsmCasterScratch, DdgiCaps, JitterState,
    LightingConfig, Material, MaterialId, MaterialTable, MeshAssetsExt,
    MeshGpu, MeshRenderScratch, OrphanedMeshGpu, OrphanedTextureGpu, RayBackendPolicy, RayCaps,
    RenderEpoch, ResolvedAa, ResolvedCsm, ResolvedShadowAtlas, ResolvedSsao, ResolvedTaa,
    RetiredGpuBuffers,
    RhiContext, SdfEditStaging, ShadowDenoiseConfig, ShadowDenoiseMode, TaaConfig, TaaState,
    TextureAssetsExt, TextureGpu, advance_jitter, collect_sdf_edits, forward_gbuffer_push_from_view,
    gbuffer_push_from_view, gbuffer_push_from_view_jittered, ndc_jitter, retire_deferred_frees,
    upload_atlas_ring,
    upload_camera_ring_sheared, upload_csm_ring, upload_instance_materials, upload_instance_materials_tex,
    upload_instance_models, upload_light_table, upload_material_assets, upload_mesh_assets,
    backfill_vb_geometry_slots,
    upload_motion_cam_ring, upload_pair_out_slot, upload_pair_ring, upload_sdf_edit_list,
    upload_taa_ring, upload_texture_assets, upload_vb_instance_rows,
};
#[cfg(all(windows, feature = "hwrt"))]
use boyko_render::{
    RayBackend, RayBackendConfig, RayGeom, RayWorkload, ResolvedRayShadow, ResolvedShadowDenoise,
    ResolvedTemporalShadow, upload_mesh_ids, upload_ray_shadow_ring, upload_shadow_denoise_ring,
    upload_temporal_shadow_ring,
};
#[cfg(windows)]
#[cfg(windows)]
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
#[cfg(windows)]
use boyko_rhi_vulkan::ffi::VkExtent2D;
#[cfg(windows)]
use boyko_rhi_vulkan::swapchain::{GBUFFER_PUSH_BYTES, GBufferMeshDraw};
#[cfg(windows)]
use boyko_scene::render_caps::MeshHandle;
#[cfg(windows)]
use boyko_scene::ViewUniform;

#[cfg(windows)]
use crate::device::GpuDevice;
#[cfg(windows)]
use crate::host::WindowHost;
#[cfg(windows)]
use crate::light_gate::light_upload_due;
// VG R3 piece 4 rung P4-4: the diagnostic regime, read live per frame beside `HzbConfig` and
// recorded into both artifacts (the probe dump's `[host]` table and the bench summary's regime
// line).
#[cfg(windows)]
use crate::occlusion_force::OcclusionForce;
#[cfg(windows)]
use crate::window_info::{HostFrameStats, WindowInfo};

/// Window description handed from [`EnginePlugins`](crate::plugins::EnginePlugins)
/// to the runner (title + requested client size; `present_mode` etc. arrive
/// with later rungs).
#[derive(Clone, Copy)]
pub(crate) struct WindowDesc {
    /// The window caption.
    pub(crate) title: &'static str,
    /// Requested client-area width in pixels.
    pub(crate) width: u32,
    /// Requested client-area height in pixels.
    pub(crate) height: u32,
    /// SSAA (AA campaign Stage 3): the owner-requested render scale — `0`/`1` mean off
    /// (the DEFAULT, byte-identical to before SSAA existed); v1 honors ONLY `2`. Read
    /// exactly once by [`crate::host::WindowHost::boot`]'s device-capability arming
    /// probe, which is the SOLE authority over whether SSAA actually activates (a
    /// request that cannot fit the device degrades to `Off`, never a panic).
    pub(crate) ssaa_scale: u32,
}

/// The frame clear color — a dark neutral (the empty-gather / background tone).
#[cfg(windows)]
const CLEAR_COLOR: [f32; 4] = [0.05, 0.07, 0.10, 1.0];

/// VB-P1d: the default number of TIMED frames the froxel cull/shade bench collects (past
/// warm-up) when `BOYKO_VB_BENCH_FRAMES` is unset. Mirrors
/// `window_present_gbuffer`'s own `GPU_PASS_COST_FRAMES` R0 harness order of magnitude.
#[cfg(windows)]
const VB_BENCH_DEFAULT_FRAMES: u32 = 220;

/// VB-P1d: the warm-up frames discarded from the front of the bench sample window (shader
/// compile + GPU clock ramp) — mirrors `window_present_gbuffer`'s own `GPU_PASS_COST_WARMUP`.
#[cfg(windows)]
const VB_BENCH_WARMUP: usize = 20;


/// Asset-system rung A1: the boot preallocation budget for
/// [`Assets::<Material>::with_reserved`] — the host does not know a game's material
/// count generically, so this is a practical setup-time default (Principle 5: reserve
/// once so `Assets::add` never reallocates the column mid-setup). `MaterialTable` hard-
/// sizes its device table to the ACTUAL registered count at `boot_seed`, so an under-
/// or over-estimate here costs nothing beyond one `Vec` growth.
#[cfg(windows)]
const MATERIAL_CAPACITY: usize = 256;

/// The windowed runner body (host plan D6): boot → World residents →
/// insert-if-absent `AppExit(false)` → `finish()` → frame loop → D2 teardown.
///
/// Boot failure (no loader / GPU / window) is NOT a panic: the runner logs one
/// line at the binary boundary and returns `AppExit(true)` — the app must exit
/// gracefully on a GPU-less machine.
///
/// The shipped runner does NOT request the validation layer BY DEFAULT (review
/// P1-3): per [`InstanceConfig`]'s contract an ABSENT
/// `VK_LAYER_KHRONOS_validation` fails boot with `ValidationUnavailable` (no
/// silent fallback), which would kill the app on every machine without the
/// Vulkan SDK.
///
/// `BOYKO_ENABLE_VALIDATION` is the opt-in knob that doc deferred to "a later
/// rung", and it turned out to be load-bearing rather than a convenience. The
/// backend gates the layer on `enable_validation && BOYKO_DISABLE_VALIDATION
/// unset` (`device.rs:2350`), so with this flag hardcoded `false` the FIRST
/// conjunct was always false and stripping the env var could not enable
/// anything. `scripts/golden.ps1 -ValidationOn` stripped it and reported
/// "VALIDATION: clean (0 messages)" for all 22 `boyko-app` pins — a gate that
/// could not fail. Measured, not inferred: a deliberately illegal
/// `mip_levels: 12` on a 512x512 image (max 10) was accepted by
/// `vkCreateImage` and drew ZERO validation messages.
///
/// Absent the variable the boot is byte-identical to before.
#[cfg(windows)]
pub(crate) fn run_windowed(app: &mut App, desc: WindowDesc) -> AppExit {
    // ── Boot: the device singleton (plan D2 step 1). ─────────────────────────
    let config = InstanceConfig {
        enable_validation: std::env::var_os("BOYKO_ENABLE_VALIDATION").is_some(),
        windowed: true,
    };
    let ctx = match VulkanContext::boot_singleton(config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("boyko_app: Vulkan device boot failed - exiting ({e:?})");
            return AppExit(true);
        }
    };

    // ── Boot: the window host chain (window → surface → swapchain → renderer
    // → the R3 G-buffer scene bundles at the boot-fixed composite extent).
    let mut host = match WindowHost::boot(ctx, &desc) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("boyko_app: window host boot failed - exiting ({e})");
            // SAFETY: the `boot_singleton` above succeeded and its singleton
            // is still live (the null-swap tripwire would catch a violation);
            // the device is idle (no GPU work was ever submitted); and no
            // `&'static VulkanContext` reference remains in any live structure
            // — the failed boot chain dropped its partial links before
            // returning, nothing was inserted into the World yet, and `ctx` is
            // not used past this statement.
            unsafe { VulkanContext::destroy_singleton() };
            return AppExit(true);
        }
    };

    // Textured-PBR rung T6c (fix, post-review): create the bindless texture-array
    // descriptor set (T4) HERE — immediately after the host boots, BEFORE any World
    // insertion or `app.finish()` — so it can be inserted as a World resident a
    // startup system can `NonSendResMut<BindlessTextureTable>` (T7's texture-loading
    // shape: decode -> register_texture -> slot -> Material::with_textures, run from
    // the user's `setup` startup system, which drains DURING `finish()` below —
    // requesting a resource not yet in the World panics, the exact bug this
    // reordering fixes; see `textured_smoke.rs`).
    //
    // `BindlessTextureTable::new` is FALLIBLE. Doing it at THIS point — before any
    // `insert_non_send_resource`/`insert_resource` call below and before
    // `app.finish()` — keeps its failure unwind MINIMAL and self-contained: nothing
    // has been inserted into the World yet (no World resident to evict) and no
    // plugin's `build()` has run yet (`AssetRefcountPlugin`'s `DeferredFree`/
    // `RenderEpoch` — which the FULL `teardown()` below force-drains via
    // `retire_deferred_frees` — do not exist in the World until `app.finish()`
    // runs). Calling the full `teardown()` here would panic on THOSE missing
    // resources — the same class of bug this whole fix addresses, just shifted to
    // the failure path instead of the happy path. So this failure branch destroys
    // ONLY the host's GPU chain (mirrors `WindowHost::boot`'s own failure branch
    // above, generalized to the now-live `host`) + the device singleton, exactly
    // like the two failure branches above it.
    let bindless_texture_table = match BindlessTextureTable::new(ctx) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("boyko_app: bindless texture table creation failed - exiting ({e:?})");
            // SAFETY: `host` was just booted on `ctx` above; no GPU work was ever
            // submitted (the frame loop has not started, nothing was drawn); no
            // World resident references any host/device resource yet (nothing was
            // inserted); `destroy_host_gpu_chain` waits the device idle (the
            // renderer's `Drop`) before destroying `host`'s explicit RHI resources;
            // no `&'static VulkanContext` reference survives past
            // `destroy_singleton` below.
            unsafe {
                destroy_host_gpu_chain(host, ctx);
                VulkanContext::destroy_singleton();
            }
            return AppExit(true);
        }
    };

    // ── World residents BEFORE `finish()` so the startup one-shots drain WITH
    // the device present (plan D2 step 4 / D6). The GPU handles are NonSend:
    // all GPU access stays runner-thread-only. `Assets<MeshGpu>` starts empty —
    // user startup registers meshes through `GpuDevice` (asset-system rung A2:
    // the mesh records own their GPU buffers, so this table IS the GPU-resident
    // mesh asset table, no separate mirror). `WindowInfo` seeds at
    // the boot client size (its one-frame-stale contract starts post-present).
    app.world_mut()
        .insert_non_send_resource(RhiContext::from_shared(ctx));
    app.world_mut().insert_non_send_resource(GpuDevice(ctx));
    app.world_mut()
        .insert_non_send_resource(Assets::<MeshGpu>::default());
    // Asset-streaming plan F6: the fill-reject orphan teardown queue (Decision 4).
    // NonSend (a `MeshGpu` orphan owns `!Send` RHI buffers) — drained every frame
    // by `retire_deferred_frees` alongside `DeferredFree`, and at shutdown below.
    app.world_mut()
        .insert_non_send_resource(OrphanedMeshGpu::default());
    // Asset-system rung A1: `Assets<Material>` is the CPU authority (a Resource,
    // the same generic asset-kernel table every future asset type shares);
    // `MaterialTable` is its GPU mirror (NonSend — it owns RHI buffers once
    // `boot_seed` runs after `finish()` below, see there for the boot-ordering
    // contract against `sync_gbuffer`). Slot 0 is minted here as the engine default
    // material so a mesh/SDF hit that carries no explicit material id always resolves
    // to a valid row; user startup mints more through `Assets::add`. The render
    // carrier truncates a fresh `Handle`'s `u32` index to `MaterialId`'s 16-bit width
    // at the mint site (`MaterialId::from_handle`) — slot 0's handle always mints
    // `MaterialId::DEFAULT`, asserted below.
    let mut material_assets = Assets::<Material>::with_reserved(MATERIAL_CAPACITY);
    let default_material = material_assets.add(Material::default());
    debug_assert_eq!(
        MaterialId::from_handle(default_material),
        MaterialId::DEFAULT,
        "invariant: the first Assets<Material> row mints MaterialId::DEFAULT"
    );
    // Asset-streaming plan F2: pin slot 0 NEVER-RETIRE. The default material is
    // referenced by every entity that carries no explicit `MaterialHandle`
    // (`MaterialHandle(0)`, potentially every entity in a scene), so its
    // refcount is never a reliable "unused" signal — `dec_ref` reaching zero on
    // this slot must stay `Loaded`, never transition to `Retiring`. There is no
    // equivalent fixed-slot default mesh (a scene that spawns `MeshHandle`
    // always names an explicitly-loaded mesh id), so only the material default
    // is pinned here.
    material_assets.pin(0);
    app.world_mut().insert_resource(material_assets);
    app.world_mut()
        .insert_non_send_resource(MaterialTable::new());
    // Asset-streaming plan F7: the grow-and-defer-old fence queue for a GPU-mirrored
    // SSBO's superseded buffer (the material table, the per-slot instance family).
    // NonSend for the same reason as `OrphanedMeshGpu` — it owns `!Send` RHI buffers.
    // Drained every frame by `retire_deferred_frees` alongside `DeferredFree` +
    // `OrphanedMeshGpu`, and force-drained at shutdown below.
    app.world_mut()
        .insert_non_send_resource(RetiredGpuBuffers::default());

    // Textured-PBR rung T6b: `Assets<TextureGpu>` is the GPU-resident texture asset
    // table (`TextureGpu` owns its `VulkanTexture` directly, mirroring
    // `Assets<MeshGpu>` — see `texture.rs`'s module doc: "no separate mirror").
    // Registered NonSend even though `TextureGpu`'s CURRENT field composition
    // happens to satisfy Rust's auto-`Send` algorithm (its `VulkanTexture` holds no
    // raw-mapped-pointer field, unlike `MeshGpu`'s `BoundBuffer::mapped:
    // Option<NonNull<u8>>`, which is what actually makes `MeshGpu` `!Send`):
    // `texture.rs`'s own module doc already declares this asset non-`Send` BY
    // DESIGN ("owns a non-Send RHI texture"), matching the engine's
    // dispatcher-only device-object policy (`boyko_rhi_vulkan::ffi`'s handle-`Send`
    // SAFETY note: "cross-thread access is governed later by the dispatcher-only
    // NonSendResource model, not a blanket Sync") rather than an accident of
    // today's struct layout that a future field addition (e.g. a host-mapped
    // readback view) could silently flip.
    //
    // Textured-PBR rung T6c (fix, post-review): `bindless_texture_table` was
    // ALREADY created (fallibly) right after the host boot above — its failure
    // already returned before reaching here. Inserted alongside `Assets<TextureGpu>`/
    // `OrphanedTextureGpu` (the rest of the texture-asset resource group) so a
    // startup `setup` system can `NonSendResMut<BindlessTextureTable>` it, exactly
    // like it already does `NonSendResMut<Assets<TextureGpu>>` (T7's texture-loading
    // shape needs BOTH live during `finish()`'s startup-system drain).
    app.world_mut()
        .insert_non_send_resource(bindless_texture_table);
    app.world_mut()
        .insert_non_send_resource(Assets::<TextureGpu>::default());
    // Textured-PBR rung T6b: the F6-style fill-reject orphan teardown queue
    // (mirrors `OrphanedMeshGpu` — a `TextureGpu` orphan owns a live device image
    // + bindless slot with no `Drop` to free them).
    app.world_mut()
        .insert_non_send_resource(OrphanedTextureGpu::default());

    // Asset-system rung A3b: the decode->upload staging queues. `AssetServer`
    // was not wired into `boyko_app` before this rung — it is minted here bare
    // (asset-streaming plan F3: loader dispatch is a compile-time-static
    // `HasLoaders` const-table on `MeshGpu`/`Material` themselves, so there
    // is no runtime registration step). `AssetStaging<A>` is the NonSend
    // handoff queue `AssetServer::load` pushes into and the boot-one-shot
    // `upload_material_assets`/`upload_mesh_assets` drain (run explicitly below,
    // after `finish()`). `AssetPaths<A>` (asset-streaming plan F4) is the
    // HashMap-free path→handle dedup index `AssetServer::load` consults —
    // wired alongside its matching `AssetStaging<A>` since both are per-type
    // arguments the same `load` call threads through. No scene calls `load`
    // yet at this rung, so every queue/index stays empty at boot — zero
    // effect, the wiring is the deliverable.
    let asset_server = AssetServer::new();
    app.world_mut().insert_resource(asset_server);
    app.world_mut()
        .insert_non_send_resource(AssetStaging::<Material>::default());
    app.world_mut()
        .insert_non_send_resource(AssetStaging::<MeshGpu>::default());
    app.world_mut()
        .insert_non_send_resource(AssetStaging::<TextureGpu>::default());
    app.world_mut()
        .insert_non_send_resource(AssetPaths::<Material>::default());
    app.world_mut()
        .insert_non_send_resource(AssetPaths::<MeshGpu>::default());
    app.world_mut()
        .insert_non_send_resource(AssetPaths::<TextureGpu>::default());

    app.world_mut().insert_resource(WindowInfo {
        width: host.window.width(),
        height: host.window.height(),
    });
    // The R4 host probe (WindowInfo-adjacent, same post-present publish step):
    // lets headless smokes assert the light-upload gating + CSM arming decisions.
    app.world_mut().insert_resource(HostFrameStats::default());

    // ── SDFDDGI I2 (plan §3): OVERRIDE the `DdgiPlugin`'s default `DdgiCaps`
    // (storage_ok = true) with the REAL device query now that the device is
    // booted. `resolve_ddgi_grid_gated` reads this to CLAMP the DDGI resolve to
    // DISABLED when the device lacks B10G11R11/RG16F STORAGE — degrading (not
    // fail-fasting) a GI-enabled config on an unsupported device, so the future
    // armed pass never binds a non-storage atlas as a storage image
    // (validation error / device loss). The same `ctx` whose
    // `DdgiAtlas::create` already read these caps to drop the STORAGE bit is
    // reachable here; `insert_resource` REPLACES the plugin's default.
    app.world_mut()
        .insert_resource(DdgiCaps::new(ctx.device_caps().ddgi_storage_ok()));

    // HW-RT rung R1 (plan §3/§9): OVERRIDE the `RayPlugin`'s default `RayCaps`
    // (tier = `Absent`) with the REAL device tier now that the device is booted.
    // `resolve_ray_backend_system` reads this to select the ray backend per
    // workload. In R1 `DeviceCaps::ray_query` is hard-wired `false` (no RT
    // extension requested), so `rt_tier()` returns `Absent` on every device and
    // the resolve stays all-software — this fill is the tested seam R2a inherits
    // (the tier goes live once the real presence+enable query lands), NOT a
    // behavior change. `insert_resource` REPLACES the plugin's default.
    app.world_mut()
        .insert_resource(RayCaps::new(ctx.device_caps().rt_tier()));

    // HW-RT rung 2 (runtime backend toggle): seed the owner's `RayBackendPolicy`
    // force-software knob from `BOYKO_FORCE_SOFTWARE`. A non-empty truthy value
    // ("1"/"true", case-insensitive) DOWNGRADES every hardware cell to Software at
    // the resolve — the host-layer boot knob that (a) makes the forced-software path
    // headlessly runnable for the byte-identity gate and (b) gives the owner a real
    // runtime toggle to flight-check on RT hardware. Unset (the default) keeps the
    // tier's own selection — today's behavior. This is inert on a non-hwrt / non-RT
    // build (no hardware cell to downgrade), but reading/writing the resource is
    // harmless, so it stays un-gated to match the surrounding `RayCaps` override.
    if let Ok(v) = std::env::var("BOYKO_FORCE_SOFTWARE") {
        let force = matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true");
        if force {
            app.world_mut()
                .resource_mut::<RayBackendPolicy>()
                .force_software = true;
            eprintln!(
                "boyko_app: BOYKO_FORCE_SOFTWARE={v} - forcing the SOFTWARE ray-shadow backend"
            );
        }
    }

    // HW-RT rung 3a step 7 (spatial-denoise flight-check knob): seed the author's
    // `ShadowDenoiseConfig` from `BOYKO_SHADOW_DENOISE`. A truthy value ("spatial"/"1"/"true",
    // case-insensitive, trimmed) flips `mode` to `Spatial` — the host-layer boot knob that (a)
    // makes the spatial-denoise path headlessly renderable for the orchestrator's structure /
    // grain gate and (b) gives the owner a real toggle to flight-check on RT hardware. An
    // optional `BOYKO_SHADOW_DENOISE_LEVELS` (a `u32`) A/B-tunes the à-trous level count
    // (clamped `1..=MAX_ATROUS_LEVELS` by `clamped_levels()` at the read site). Unset (the
    // default `mode == None`) keeps every host world byte-identical (the `scene.shadow` gate
    // stays closed). `insert_resource` in `ShadowDenoisePlugin` seeded the config, so this
    // overwrites its fields. Un-gated to match the surrounding `RayCaps`/`BOYKO_FORCE_SOFTWARE`
    // overrides (the resource always exists; on a non-RT device the gate's `tlas_enabled` half
    // keeps the pass off regardless).
    if let Ok(v) = std::env::var("BOYKO_SHADOW_DENOISE") {
        // Rung 3b step 7: the 4-state selector. `spatial`/`1`/`true` keep the Rung-3a SPATIAL (à-trous)
        // path; `temporal` selects the cross-frame reproject (0 à-trous); `both` runs à-trous THEN
        // temporal. Any other value leaves the default `None` (byte-identical).
        let mode = match v.trim().to_ascii_lowercase().as_str() {
            "spatial" | "1" | "true" => Some(ShadowDenoiseMode::Spatial),
            "temporal" => Some(ShadowDenoiseMode::Temporal),
            "both" => Some(ShadowDenoiseMode::Both),
            _ => None,
        };
        if let Some(mode) = mode {
            let levels = std::env::var("BOYKO_SHADOW_DENOISE_LEVELS")
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok());
            let cfg = app.world_mut().resource_mut::<ShadowDenoiseConfig>();
            cfg.mode = mode;
            if let Some(levels) = levels {
                cfg.levels = levels;
            }
            let clamped = cfg.clamped_levels();
            eprintln!(
                "boyko_app: BOYKO_SHADOW_DENOISE={v} - enabling the {mode:?} shadow denoise (à-trous levels={clamped})"
            );
        }
    }

    // Multi-paradigm render-path plan, rung R1 (Decision 1 — the `ssaa_armed` boot-lock
    // precedent): resolve `RenderPathConfig` + the boot-time pre-light-consumer snapshot +
    // the device's VB geometry-table capability into ONE immutable `ResolvedRenderPath`,
    // exactly once, before the frame loop starts (AFTER the `BOYKO_SHADOW_DENOISE` env
    // override above, so the shadow-denoise consumer snapshot reflects the flight-check
    // knob). `world.try_resource` degrades every missing config Resource to its structural
    // "off" default (a host that composes a SUBSET of `EnginePlugins`'s plugins never
    // panics here) — the SAME graceful pattern `ddgi_enabled`/`terminator_wrap` use inside
    // the frame loop. `host.resolved_render_path` is written below and IS read downstream —
    // this comment said "nothing reads it yet (R2 wires the declarator dispatch)" long after R2
    // wired exactly that: `boyko_rhi_vulkan`'s `declare_frame_graph` selects the per-path
    // declarator by matching on the threaded carrier's `path`.
    let render_path_cfg = app
        .world()
        .try_resource::<boyko_render::RenderPathConfig>()
        .copied()
        .unwrap_or_default();
    // Hardware-traced/denoised shadow visibility is armed only on a ray-capable device
    // (`ray_query_enabled()` is unconditionally `false` on a `not(hwrt)` build, so this
    // expression needs no separate `#[cfg]` split) AND the author's `ShadowDenoiseConfig`
    // wanting spatial or temporal denoise — the SAME two conditions `frame_loop`'s
    // `denoise_armed` folds into `scene.shadow`'s per-frame gate, computed here at boot
    // since `ResolvedRenderPath`'s shadow-source arming is itself a boot commitment, never
    // a per-frame re-derivation.
    let hwrt_denoise_or_vis_on = ctx.ray_query_enabled()
        && app
            .world()
            .try_resource::<boyko_render::ShadowDenoiseConfig>()
            .is_some_and(|cfg| cfg.spatial_enabled() || cfg.temporal_enabled());
    let render_path_consumers = boyko_render::RenderPathConsumers {
        ssao_on: app.world().try_resource::<boyko_render::SsaoConfig>().is_some_and(|c| c.enabled()),
        ddgi_on: app.world().try_resource::<boyko_render::DdgiConfig>().is_some_and(|c| c.enabled()),
        shadow_denoise_spatial_on: app
            .world()
            .try_resource::<boyko_render::ShadowDenoiseConfig>()
            .is_some_and(|c| c.spatial_enabled()),
        shadow_temporal_on: app
            .world()
            .try_resource::<boyko_render::ShadowDenoiseConfig>()
            .is_some_and(|c| c.temporal_enabled()),
        // No `SsrConfig` exists yet (out of scope this rung) — the plan §A default.
        ssr_on: false,
        taa_on: app.world().try_resource::<AaConfig>().is_some_and(|c| matches!(c.mode, AaMode::Taa)),
        csm_on: app.world().try_resource::<boyko_render::CsmConfig>().is_some_and(|c| c.enabled()),
        punctual_shadows_on: app
            .world()
            .try_resource::<boyko_render::ShadowConfig>()
            .is_some_and(|c| c.enabled()),
        hwrt_denoise_or_vis_on,
        // No owner-facing SDF-shadow toggle exists yet — mirrors Deferred's current
        // unconditional non-hwrt SDF soft shadow (see `RenderPathConsumers::sdf_shadows_wanted`'s doc).
        sdf_shadows_wanted: true,
        // VB-P1b: the real `LightingConfig::clusters_enabled` toggle (owner-set, DEFAULT
        // `false` — see that field's own doc), the SAME degrade-to-off-on-missing-resource
        // shape every other consumer probe above uses. `froxel_light_cull` itself
        // (`RenderPathConsumers::clusters_wanted`'s doc) additionally requires
        // `path == VisibilityBuffer`, so a Deferred/Forward/ForwardPlus scene that sets
        // `clusters_enabled = true` still resolves unarmed here — VB-only by construction.
        clusters_wanted: app
            .world()
            .try_resource::<LightingConfig>()
            .is_some_and(|c| c.clusters_enabled),
    };
    let render_path_caps = boyko_render::RenderPathDeviceCaps::new(
        ctx.device_caps().storage_buffer_array_non_uniform_indexing_ok,
    );
    let (resolved_render_path, render_path_degrades) =
        boyko_render::resolve_render_path(&render_path_cfg, render_path_consumers, render_path_caps);
    // Warn-once boot diagnostics (mirrors `WindowHost::boot`'s SSAA degrade `eprintln!` —
    // no logging crate is wired into this workspace, so `eprintln!` is the established
    // boot-diagnostic channel). Never a panic — degrade-not-panic by construction.
    for degrade in render_path_degrades.reasons() {
        eprintln!("boyko_app: render path degraded ({degrade:?})");
    }
    host.resolved_render_path = resolved_render_path;
    // OVERRIDE the `RenderPathPlugin`'s default `ResolvedRenderPath` with the real
    // boot-resolved value — the SAME `DdgiCaps`/`RayCaps` post-boot override precedent
    // above (this fn's `DdgiCaps::new(..)`/`RayCaps::new(..)` inserts). Without this, a
    // future `Res<ResolvedRenderPath>` consumer (R2's declarator dispatch) would read the
    // plugin's stale `Deferred + Both` default forever, even when the owner requested (and
    // this boot-lock resolved) something else — the host field above is the per-frame RHI
    // seam's source of truth, but the World Resource must ALSO be genuinely authoritative
    // for any ECS-side reader.
    app.world_mut().insert_resource(resolved_render_path);
    // Rung R9a (plan P2-d): OVERWRITE `SsaoPlugin`'s inert default boot-freeze snapshot with
    // the REAL one — the SAME post-boot override discipline as `resolved_render_path` above.
    // Under a non-Deferred resolved path the pre-light consumer set is boot-committed; the
    // snapshot lets every per-frame `SsaoConfig` reader clamp to the boot truth (warn-once)
    // instead of drifting from the boot-shaped framegraph. Snapshot taken from the SAME
    // Resource the `render_path_consumers` assembly above read.
    let boot_ssao_cfg = app
        .world()
        .try_resource::<boyko_render::SsaoConfig>()
        .copied()
        .unwrap_or_default();
    // Rung R9c: the DDGI CONFIG bit joins the snapshot (the caps fold —
    // `ddgi_storage_ok()` — stays live in the frame loop; only the owner-config half
    // freezes, mirroring `render_path_consumers.ddgi_on`'s own assembly above).
    let boot_ddgi_on = app
        .world()
        .try_resource::<boyko_render::DdgiConfig>()
        .is_some_and(|c| c.enabled());
    app.world_mut().insert_resource(boyko_render::RenderPathFrozenConsumers::new(
        boot_ssao_cfg,
        boot_ddgi_on,
        !matches!(resolved_render_path.path, boyko_render::RenderPath::Deferred),
    ));

    // Multi-paradigm render-path plan, rung R-VBGEO (Decision 0 / Rev-5 streaming
    // invariant): commit `vb_geometry_table` onto `ctx` and construct the
    // `MeshGeometryTableSlot` wrapper resource — BOTH before `app.finish()` (which drains
    // every startup system, incl. a user `setup` that may call
    // `MeshAssetsExt::cube`/`plane`/`register_mesh`) and before the `upload_mesh_assets`
    // boot drain below, so the flag is available at EVERY mesh-registration site,
    // host-authored or streamed, before the FIRST mesh upload (the Rev-5 gate).
    // `VB_IMPLEMENTED` is `true` as of rung R8, so this arm is LIVE: a `VisibilityBuffer x
    // Mesh` resolve on a device with the descriptor-indexing prerequisite builds a real
    // table (and the VB compute pipelines that need its layout) here. Every other boot
    // takes the `false` branch — `MeshGeometryTableSlot(None)`, zero cost, no device calls.
    ctx.set_vb_geometry_table_armed(resolved_render_path.vb_geometry_table);
    let mesh_geometry_table_slot = if resolved_render_path.vb_geometry_table {
        match boyko_render::MeshGeometryTable::new(ctx) {
            Ok(table) => {
                // Multi-paradigm render-path plan, rung R8: `vb_resolve_pipeline`'s Set 2 needs
                // this table's live descriptor-set layout (`GpuSceneBundles::vb_resolve_pipeline`'s
                // doc) — build it HERE, right after the table itself exists, before any frame
                // records against it.
                host.gpu.build_vb_resolve_pipeline(ctx, table.set());
                // VB-P2 classification plan, rung P2a (dark infra): builds the classify/shade
                // pipelines right alongside the fused resolve pipeline — needs the SAME
                // geometry-table Set-2 layout `vb_shade` shares with `vb_resolve`. Nothing
                // declares/records against them yet (`record_vb`/`declare_vb_graph` are
                // untouched this rung).
                host.gpu.build_vb_classify_pipelines(ctx, table.set());
                boyko_render::MeshGeometryTableSlot(Some(table))
            }
            Err(e) => {
                eprintln!(
                    "boyko_app: MeshGeometryTable::new failed ({e:?}) - VB geometry table disabled"
                );
                boyko_render::MeshGeometryTableSlot(None)
            }
        }
    } else {
        boyko_render::MeshGeometryTableSlot(None)
    };
    app.world_mut().insert_non_send_resource(mesh_geometry_table_slot);

    // ── Windowed `AppExit` semantics: insert-IF-ABSENT (plan D6; the legacy
    // headless path keeps its unconditional insert).
    if !app.world().contains_resource::<AppExit>() {
        app.world_mut().insert_resource(AppExit(false));
    }

    app.finish();

    // SSAA (AA campaign Stage 3, C1): resolution is a BOOT COMMITMENT — when the host
    // armed the 2× composite extent (`WindowHost::boot`'s device-capability probe), the
    // ECS mode MUST agree, so a 2× render is never left un-consumed (which would sample
    // the 2× `lit` 1:1 through the unchanged present-blit crop — a cropped top-left
    // quarter, never what an armed boot intends). This is TRUTHFUL, not the sole
    // authority: the per-frame read site below is the backstop LOCK (host-authoritative
    // regardless of what any startup system might have inserted).
    if host.ssaa_armed {
        app.world_mut().insert_resource(AaConfig { mode: AaMode::Ssaa });
    }

    // The R7 SDF edit-list gather — run ONCE here, explicitly (host plan R7, the P0
    // order fix). `collect_sdf_edits` MUST observe every `SdfPrimitive` the user spawns,
    // including those from systems registered via `add_startup_system` AFTER
    // `add_plugins(EnginePlugins)`. Startup systems drain in PUSH order inside `finish()`
    // above, so a plugin-registered startup gather would race (run BEFORE) the user's
    // later `setup` and see zero primitives. Running it HERE — after `finish()` drained
    // ALL startup systems (World fully populated: every spawn applied, the GPU residents
    // inserted before finish) and before the frame loop — makes the gather order-proof
    // and single-site. It sets `SdfEditStaging::dirty` deterministically; the frame
    // loop's first-frame `is_dirty()` block then performs the one-shot upload unchanged.
    app.world_mut().run_system(collect_sdf_edits);

    // Textured-PBR rung T6c (fix, post-review): build the TEXTURED gbuffer producer
    // pipeline (a 2-set layout needing the bindless texture-array table's
    // descriptor-set LAYOUT, which does not exist at `GpuSceneBundles::boot()` time
    // — see that fn's doc). `BindlessTextureTable` was created + inserted into the
    // World BEFORE `finish()` above (so a startup `setup` system could
    // `NonSendResMut<BindlessTextureTable>` it to register textures — see
    // `textured_smoke.rs`); read it back here via a World borrow instead of the old
    // local variable. `&mut host.gpu` (a plain `WindowHost` field, not a World
    // resident) and the World's `&BindlessTextureTable` are disjoint objects, so
    // this borrow cannot conflict with anything the drained startup systems did to
    // the table — `finish()` has already fully run by this point, so the two
    // accesses are SEQUENTIAL (setup's writes, then this read), never concurrent.
    let bindless_texture_table = app.world().non_send_resource::<BindlessTextureTable>();
    host.gpu.build_textured_resources(ctx, bindless_texture_table);

    // Textured-PBR rung TV0 (`RENDER-PARITY-PLAN.md` §2.3): build the `vb_shade` TEXTURED
    // compute pipeline — needs BOTH the Decision-0 geometry table's Set-2 layout (built above,
    // right after `MeshGeometryTable::new`) AND the bindless table's Set-3 layout (just built by
    // `build_textured_resources` above — the LAST of the two dependencies to exist), so this
    // call is the LATEST point either builder could run. `Option`-guarded: a no-op (`vb_shade_tex_pipeline`
    // stays `None`) on any boot without a live geometry table (`MeshGeometryTableSlot(None)` —
    // mirrors `build_vb_resolve_pipeline`/`build_vb_classify_pipelines`'s own gate).
    if let Some(table) =
        app.world().non_send_resource::<boyko_render::MeshGeometryTableSlot>().0.as_ref()
    {
        host.gpu.build_vb_shade_textured_pipeline(ctx, table.set(), bindless_texture_table);
        // Rung R9b: the split pair (`vb_geo` + `vb_shade_split{,_tex}`) — needs the SAME
        // geometry Set-2 layout; the TEXTURED sibling additionally needs the bindless Set-3
        // (`Some` here — `build_textured_resources` just built it above).
        host.gpu.build_vb_split_pipelines(ctx, table.set(), Some(bindless_texture_table));
        // VB-P1b: the ENTIRE froxel light-cull machinery, gated behind the single boot-frozen
        // arm bit `ResolvedRenderPath::froxel_light_cull` — armed iff the booted scene's
        // `LightingConfig::clusters_enabled` is `true` under `RenderPath::VisibilityBuffer`
        // (`RenderPathConsumers::clusters_wanted`'s own doc). Every scene that never opts in
        // leaves `host.gpu.cluster_cull_pipeline`/`cluster_grid`/`light_index`/
        // `light_index_alloc`/`vb_layout0_froxel` all `None`, so every pre-VB-P1b golden stays
        // byte-identical. Needs the SAME geometry Set-2 layout + bindless Set-3 the TEXTURED
        // pipeline above just proved exist.
        //
        // The `ClusterConfig` Resource (VB-P1b-0, `EnginePlugins::build`'s default seed) —
        // NOT a hardcoded local — so the buffer/grid SIZING here and the header's
        // `cluster_packed_dims` (`sync_cluster_light_gate`, reading the SAME resource) can
        // never disagree on dims. `try_resource` degrades to `ClusterConfig::default()` for a
        // host composing a subset of `EnginePlugins` (the SAME graceful pattern every other
        // consumer probe above uses).
        if resolved_render_path.froxel_light_cull {
            let cluster_config = app
                .world()
                .try_resource::<boyko_render::ClusterConfig>()
                .copied()
                .unwrap_or_default();
            // VB-P1e H4: the hierarchical cull ARM selection — a BOOT-TIME-ONLY env toggle (read
            // once, here; no per-frame cost). This is an A/B ARM SELECTOR, not a force-off
            // switch (P1-1, adversarial review): its only sanctioned protocol is an interleaved
            // paired sweep, whose natural operator spelling is `=0`/`=1` — so it parses the
            // VALUE (`parse_hier_cull_env`'s `"0"`/`"1"` grammar), unlike
            // `BOYKO_VB_FROXEL_FORCE_OFF`'s "presence is the trigger" convention
            // (`vb_p1d_cull_shade_bench.rs`'s doc), which that knob can afford because it has
            // only one meaningful "on" state. Unset (the default, every golden/production boot)
            // selects the BASE 64-wide arm — byte-identical to every pre-H4 boot. `=1` selects
            // the `-D HIER=1` 256-wide arm this rung arms (H3 proved on hardware that the two
            // arms emit the same per-froxel sets in the same order); `=0` selects the base arm
            // explicitly (so an A/B sweep's `=0` leg cannot be mistaken for `=1` and silently
            // compare the hierarchical arm against itself — this campaign's recurring
            // reproducibility failure class). Any other value panics loudly rather than
            // guessing.
            // VB-P1e follow-up: the DEFAULT is now the hierarchical arm. Decided by numbers, per
            // this project's standing rule that performance decides a render fork — not deferred
            // to the owner, because nothing about it is a values call:
            //
            // * H4 measured the cull at N=512 at **22.5x** the base arm (paired protocol, 6
            //   consecutive pairs at -95.6% with 0.03pp spread), 26.2x on the collinearity-fixed
            //   rig, and 9.3x on the dense in-frustum rig — the honest lower bound.
            // * There is no low-N penalty to trade against it: at N=8 the hierarchical arm is
            //   **1.4x FASTER** than the base one, which is the outcome the design feared and did
            //   not get.
            // * Byte-identity is proven end to end, not merely in the cull's buffers: H3's device
            //   oracle showed both arms emit the same per-froxel sets in the same order, and H5
            //   re-rendered `vb_mesh_froxel` with the arm on to `fb220ff3...` — the base arm's own
            //   pin, through the whole pipeline to the final frame.
            //
            // What is NOT decided here, and stays the owner's: **retiring** the base arm. It
            // remains fully built, selectable, and the equality oracle's permanent reference —
            // which is also why the knob keeps both directions rather than becoming a
            // hierarchical-only switch.
            let hier_cull = match std::env::var("BOYKO_VB_HIER_CULL") {
                Ok(v) => parse_hier_cull_env(&v),
                Err(std::env::VarError::NotPresent) => true,
                Err(std::env::VarError::NotUnicode(v)) => panic!(
                    "invariant: BOYKO_VB_HIER_CULL must be `0` or `1` (valid UTF-8), got {v:?}"
                ),
            };
            // Self-identifying boot log (P1-1): every bench/host log names the arm actually
            // selected, so a mis-set knob is visible after the fact rather than reading as a
            // silent "no win, no regression" from an accidental self-comparison.
            eprintln!(
                "boyko_app: VB-P1e froxel cull arm = {} (BOYKO_VB_HIER_CULL)",
                if hier_cull {
                    "HIER (-D HIER=1, 256-wide, default)"
                } else {
                    "BASE (64-wide, opt-out)"
                }
            );
            host.gpu.build_froxel_light_cull(
                ctx,
                table.set(),
                bindless_texture_table,
                cluster_config,
                hier_cull,
            );
        }
    }

    // Asset-system rung A3b: drain any decoded-but-not-yet-uploaded assets BEFORE
    // `MaterialTable::boot_seed` below sizes/uploads the device SSBO from
    // `Assets<Material>` — a loaded material must be `fill`ed (and therefore
    // counted in `high_water`) before that seed runs. Materials FIRST (the
    // ordering `boot_seed` requires), then meshes (no ordering dependency on
    // `boot_seed`, but run alongside as one boot-time asset-drain block). This is
    // a BOOT ONE-SHOT, not a per-frame system (keeps the frame loop unchanged):
    // at A3b no scene calls `AssetServer::load`, so both drains are empty and
    // this costs nothing beyond the `staging.is_empty()` check (byte-identical).
    // Textured-PBR T6b adds the texture drain alongside (also empty at boot —
    // dormant until a later rung loads a texture).
    app.world_mut().run_system(upload_material_assets);
    app.world_mut().run_system(upload_mesh_assets);
    app.world_mut().run_system(upload_texture_assets);

    // Multi-paradigm render-path plan: under a VisibilityBuffer boot, back-fill a geometry-table
    // slot for every HOST-AUTHORED mesh (`register_mesh`/`cube`/`plane`, which register with the
    // reserved slot) so ANY scene's meshes are re-fetchable by `vb_resolve` — not just those that
    // used the VB-aware `register_mesh_vb`. A no-op (returns immediately) on every non-VB boot
    // (`MeshGeometryTableSlot(None)`), so Deferred/Forward/ForwardPlus stay byte-identical; meshes
    // that already hold a real slot (streamed / `register_mesh_vb`) are skipped. Runs after the
    // mesh drain above (streamed meshes present) and before the frame loop's first VB resolve.
    app.world_mut().run_system(backfill_vb_geometry_slots);

    // Asset-system rung A1: boot-seed the material table — hard-size + upload the
    // device SSBO from whatever `finish()` drained into `Assets<Material>` (every
    // startup `Assets::add` call already landed, by the SAME order-proof
    // `collect_sdf_edits` relies on above). This MUST run before the frame loop's
    // first frame: `GBufferTargets::sync_gbuffer` (called lazily from inside
    // `render_gbuffer_frame`, never during `WindowHost::boot` or `finish()`) writes
    // the persistent resolve/vocab descriptor sets against `MaterialTable::table()`
    // ONCE and never updates them per-frame, so the table must be live before that
    // first bind.
    //
    // `boot_seed` needs `&Assets<Material>` and `&mut MaterialTable` live at once;
    // both live in the same World, so the resource is taken out (owned) for the
    // duration of the call and reinserted immediately after — a one-time boot cost,
    // never on the per-frame path.
    let material_assets = app
        .world_mut()
        .remove_resource::<Assets<Material>>()
        .expect("invariant: Assets<Material> was inserted before finish()");
    app.world_mut()
        .non_send_resource_mut::<MaterialTable>()
        .boot_seed(&material_assets, ctx);
    app.world_mut().insert_resource(material_assets);

    // A startup-requested exit is honored (plan D6): skip the loop, tear down.
    // BOTH frame-loop exits (normal Escape/close/AppExit AND a terminal render
    // error) return into this ONE teardown + destroy sequence below.
    if !app.world().resource::<AppExit>().0 {
        frame_loop(app, &mut host, ctx);
    }

    teardown(app, host, ctx);

    // Plan D2 step 4 — end the singleton's lifecycle: the LAST statement of
    // the RUNNER's teardown sequence, deliberately OUTSIDE `teardown` itself.
    // `teardown` takes `ctx: &VulkanContext` as a reference PARAMETER, and a
    // reference parameter is PROTECTED for the whole call (the same class the
    // paramless-`destroy_singleton` review P0 pinned): deallocating the
    // referent INSIDE that call would be UB regardless of any use-after.
    // Hoisted here, `teardown` has returned (its protector is gone) and only
    // the runner's own `ctx` LOCAL remains — never used past this statement.
    //
    // SAFETY: the `boot_singleton` above succeeded and its singleton is still
    // live — this is the runner's only destroy on this path (the null-swap
    // tripwire would catch a violation); the device is idle (`teardown`
    // dropped the `Renderer`, whose `Drop` waits idle, and nothing submitted
    // since); and NO `&'static VulkanContext` reference remains in any live
    // structure — `teardown` destroyed the whole host chain (renderer /
    // targets / bundles / swapchain / surface / window) and evicted every
    // World GPU resident (`RhiContext`, `Assets<MeshGpu>`, `GpuDevice`), no
    // protected `&ctx` parameter is in scope, and `ctx` is not used past this
    // statement — so the documented `'static` fiction ends with no surviving
    // reference.
    unsafe { VulkanContext::destroy_singleton() };

    // Plan D2: the post-run App state is pinned — the World is no longer
    // GPU-capable. A violation here means teardown forgot an eviction and a
    // dangling `&'static` survives `destroy_singleton` (a bug, not user error).
    debug_assert!(
        !app.world().contains_non_send_resource::<RhiContext>()
            && !app.world().contains_non_send_resource::<GpuDevice>()
            && !app.world().contains_non_send_resource::<Assets<MeshGpu>>()
            && !app.world().contains_non_send_resource::<MaterialTable>()
            && !app.world().contains_non_send_resource::<Assets<TextureGpu>>()
            && !app.world().contains_non_send_resource::<BindlessTextureTable>(),
        "invariant: the post-run World is GPU-evicted (plan D2 + textured-PBR T6b)"
    );

    AppExit(true)
}

/// The non-Windows runner arm: windowing is Windows-first (`boyko_rhi_vulkan`
/// D8 — the XCB/Wayland arm lands when Linux on-screen is first targeted), so
/// the windowed runner exits gracefully, mirroring the boot-failure path.
#[cfg(not(windows))]
pub(crate) fn run_windowed(_app: &mut App, _desc: WindowDesc) -> AppExit {
    eprintln!("boyko_app: windowing is not supported on this platform - exiting");
    AppExit(true)
}

/// Parses `BOYKO_VB_HIER_CULL`'s `"0"`/`"1"` grammar (VB-P1e H4, P1-1 adversarial review):
/// this knob is an A/B ARM SELECTOR, not a force-off switch, so it must read the VALUE, not
/// merely the env var's presence — an operator's natural `=0` spelling under the old
/// "presence is the trigger" convention would silently select the hierarchical arm, comparing
/// it against itself in a paired sweep. Any spelling other than the two accepted digits panics
/// loudly rather than guessing, mirroring `vb_p1d_cull_shade_bench.rs`'s own `parse_grid_spec`
/// discipline.
#[cfg(windows)]
fn parse_hier_cull_env(spec: &str) -> bool {
    match spec {
        "0" => false,
        "1" => true,
        other => panic!(
            "invariant: BOYKO_VB_HIER_CULL must be `0` (base arm) or `1` (hierarchical arm), got `{other}`"
        ),
    }
}

/// The OS→ECS input bridge (host plan R6, Decision 1): translate one drained
/// [`CapturedMsg`] and push the resulting [`RawInputEvent`] into the World's
/// [`RawInputQueue`].
///
/// A [`CapturedMsg::Raw`] triple is mapped by `translate_win32` (an unmapped
/// message yields `None` and is dropped); a [`CapturedMsg::RawMouse`] delta by
/// `translate_win32_raw_mouse`. The runner does NOT `begin_frame` / drain /
/// `apply` here — those stay in `update_action_state` (on `Main`), which folds
/// this queue into the frame's [`PhysicalInput`](boyko_input::PhysicalInput)
/// snapshot. Pure (no FFI, no GPU), so it is unit-testable on every host.
#[inline]
fn ingest_captured(queue: &mut RawInputQueue, captured: CapturedMsg) {
    match captured {
        CapturedMsg::Raw { msg, wparam, lparam } => {
            if let Some(ev) = translate_win32(msg, wparam, lparam) {
                queue.push_raw(ev);
            }
        }
        CapturedMsg::RawMouse { dx, dy } => {
            queue.push_raw(translate_win32_raw_mouse(dx, dy));
        }
    }
}

/// The per-frame loop (host plan D6 / the runner-frame table, R3 + R4 + R6):
///
/// 1. pump the OS queue + drain input: WITH an `InputPlugin` (a World
///    [`RawInputQueue`]) each drained message is bridged into the queue (R6,
///    Decision 1) for `update_action_state` to fold; WITHOUT one, a lightweight
///    inline Escape scan sets `AppExit` (the `room.rs` / `clear.rs` fallback);
/// 2. `update_with_delta` — Time → events → Fixed×N → Main (propagation,
///    camera resolve, `fly_camera_system`, `sync_instance_model_cols`,
///    `gather_mesh_draws`, `gather_shadow_casters`, light reconcile + collect,
///    the CSM fit);
/// 3. `AppExit` check;
/// 4. `token = wait_frame_in_flight()` — the pacing point + the fence proof;
/// 5. token-typed uploads into slot `token.slot()`: the b5 camera block + the
///    instance-model ring (UNCONDITIONAL, plan D5) + the interpolation-pair ring
///    (UNCONDITIONAL, plan D5/R5) + the CSM cascade UBO (unconditional 336 B) +
///    the light staging iff `light_uploaded_gen[s] != LightTableGeneration`
///    (the D5 gate);
/// 6. assemble `GBufferScene` on the stack (draw list from the gather output,
///    `casts_shadow` from the caster gather, `csm` armed on "fitted sun AND
///    live casters" — the `sync_csm_light_gate` predicate);
/// 7. `render_gbuffer_frame(token, ..)` — consumes the token; `Ok(false)` ⇒
///    recreate-skip, `Err` ⇒ exit;
/// 8. `refresh_size()`; write `WindowInfo` + `HostFrameStats` (one-frame-stale
///    contract).
///
/// Simulation fully precedes the fence wait (CPU/GPU overlap). A minimized
/// window (0×0 client) skips 4–7 and keeps pumping.
#[cfg(windows)]
fn frame_loop(app: &mut App, host: &mut WindowHost, ctx: &'static VulkanContext) {
    let (cw, ch) = host.composite_extent;
    let present_extent = VkExtent2D {
        width: cw,
        height: ch,
    };
    // The env-gated frame dump (`BOYKO_HOST_DUMP` — the host's diagnostic /
    // owner-eval channel, see `host_dump`). `None` on the steady path.
    let mut dump = crate::host_dump::HostDump::from_env(host.swapchain.format());
    // VG-R0 rung R0c: the env-gated density census (`BOYKO_VG_CENSUS`). `None` on the steady path,
    // and independent of `dump` above — the two read different images at different extents.
    let mut census = crate::vg_census_dump::VgCensusDump::from_env();
    // VG R3 piece 1 step P1-6: the env-gated HZB pyramid dump (`BOYKO_HZB_DUMP`) — gate G8's
    // recording seam. `None` on the steady path, and independent of both drivers above: it copies
    // the depth ring and the pyramid, neither of which the other two touch.
    let mut hzb_dump = crate::hzb_dump::HzbDump::from_env();
    // VG R3 piece 2 step P2-6: the env-gated VB RECORDING probe (`BOYKO_VB_PROBE`) — gate G2's
    // seam. `None` on the steady path, and independent of the three drivers above: it copies no
    // resource at all, it hands the recorder a host-memory count sink for one settled frame.
    let mut vb_probe = crate::vb_probe_dump::VbProbeDump::from_env();
    // VG R3 piece 3 step P3-5: the env-gated cull READBACK capture (`BOYKO_VB_CULL_READBACK`).
    // `None` on the steady path. Until this step the readback ran on the FIRST presented frame and
    // `return`ed from inside its own branch, which is why arming it beside `BOYKO_HZB_DUMP` wrote
    // the cull file and never the pyramid one; it now settles, requests and drains on the SAME
    // schedule as `hzb_dump` and exits through the conjunction below with the other four.
    let mut vb_cull_probe = crate::vb_cull_probe::VbCullProbe::from_env();
    // The staging and the driver read the SAME variable at two sites (`GpuSceneBundles::boot` mints
    // the buffer, this drives the capture). A boot that allocated the staging with no driver to end
    // the run would spin forever; a driver with no staging would drain and decode nothing.
    debug_assert_eq!(
        vb_cull_probe.is_some(),
        host.gpu.vb_cull_readback.is_some(),
        "invariant: the cull-probe driver and its staging are armed by one variable"
    );
    // Automated-run frame cap (the `BOYKO_WIN_HIDDEN` companion, and the app-level
    // twin of `window_present_gbuffer`'s own `BOYKO_WINDOW_FRAMES`): `BOYKO_WINDOW_FRAMES=<n>`
    // bounds this loop to `n` iterations, then exits cleanly. Without it an
    // orchestrated run of an `app.run()` scene that forgot `BOYKO_HOST_DUMP` (whose
    // capture is the usual exit) spins FOREVER — a window that "never closes by
    // itself", or worse an invisible spinner under `BOYKO_WIN_HIDDEN`. Unset
    // (every interactive/owner run): unbounded, exactly as before.
    let max_frames: Option<u64> = std::env::var("BOYKO_WINDOW_FRAMES")
        .ok()
        .and_then(|s| s.parse().ok());
    let mut frames_run: u64 = 0;
    let mut last = Instant::now();
    // SDFDDGI I2 (arm): a monotonically-incrementing frame index feeding the probe-update UBO's
    // round-robin `frame_index` (which subset updates this frame). Wraps at u32::MAX (benign — the
    // subset phase is `frame_index % subset_n`).
    let mut frame_index: u32 = 0;

    // Profiling rung 5c: leg B, and since rung 7 step 6c the ONLY GPU timing leg in the engine.
    // All three old collectors are deleted, so there is no A/B left to keep apart and `boot` reads
    // exactly one knob — `BOYKO_VB_ZONE`. What the exclusivity assert protected was the case where
    // a boot armed both legs and an operator read the result as a comparison it never made.
    let vb_zone = host.gpu.vb_zone_armed();
    // Its own timed-frame budget, on the SAME knob shape and the SAME default as the bench's, so
    // the two legs of one A/B run the same number of frames without the driver stating it twice.
    let vb_zone_frames: u32 = if vb_zone {
        std::env::var("BOYKO_VB_BENCH_FRAMES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(VB_BENCH_DEFAULT_FRAMES)
            .max(1)
    } else {
        0
    };
    // Preallocated ONCE (Principle 5): ~9 KiB of readback scratch the retire path fills every
    // frame. A per-frame temporary of that size is exactly the allocation this engine removes.
    let mut vb_zone_scratch = boyko_rhi_vulkan::present::gpu_zone::RetireScratch::new();
    let mut vb_zone_seen: u32 = 0;
    let mut vb_zone_pairs_measured: u64 = 0;
    let mut vb_zone_pairs_lost: u64 = 0;
    let mut vb_zone_pairs_torn: u64 = 0;
    let mut vb_zone_pairs_unbracketed: u64 = 0;
    // Profiling rung 7: the reducer that fills the measurement artifact. Built only when the zone
    // leg is armed AND a path was named, so a run without `BOYKO_PROFILE_ARTIFACT` accumulates
    // nothing -- the artifact is opt-in for the same reason the collectors are.
    let vb_zone_artifact_path: Option<std::path::PathBuf> =
        std::env::var_os("BOYKO_PROFILE_ARTIFACT").map(std::path::PathBuf::from);
    // THE STALENESS DISCRIMINATOR, chosen by the parent BEFORE this process started. See
    // `profiling::artifact`'s Decision 4: it is the only header field a parent can predict.
    // Profiling rung 7: the ZONE leg's own regime census. The bench leg has carried
    // `vb_bench_force_seen`/`vb_bench_mode_seen` since P4-4 for its printed provenance line; the
    // artifact needs the same observation, and it cannot be derived from `ResolvedRenderPath`
    // because P4-4 made the regime a LIVE Resource (`artifact.rs`'s Decision 7). Accumulated from
    // the SAME per-frame values the scene was assembled from, so the census describes the frames
    // this window timed rather than the state at exit.
    let mut vb_zone_force_seen: u8 = 0;
    let mut vb_zone_mode_seen: u8 = 0;
    let vb_zone_run_token: String =
        std::env::var("BOYKO_PROFILE_RUN_TOKEN").unwrap_or_default();
    let mut vb_zone_reducer = (vb_zone && vb_zone_artifact_path.is_some()).then(|| {
        crate::profiling::reduce::WindowReducer::new(f64::from(ctx.device_caps().timestamp_period))
    });
    // Preallocated ONCE at their final capacity (Principle 5) — the bench never reallocates
    // once the loop starts. VB-P1e H0 split the single `cull_ns` sample into
    // `cull_reset_ns`/`cull_dispatch_ns` (the `CullReset`/`CullDispatch` bracket pair) so the
    // fixed-cost hypothesis in `VB-P1E-HIERARCHICAL-CULL-PLAN.md` §1.2 can be attributed.
    // VG R3 piece 4 rung P4-4: the SET of distinct occlusion regimes observed across the TIMED
    // frames, as two bitmasks (bit `k` = the variant at index `k` was seen). Rung P4-4 turned the
    // regime from a boot-time env read into a live Resource, and the boot read's second rationale
    // was that a mid-run knob makes "which regime produced this capture?" unanswerable. The answer
    // is RECORDED, never asserted: a flip shows as `n_distinct > 1` in the summary and the harness
    // rejects that worker, instead of averaging two regimes and attributing them to one.
    //
    // Two `u8`s, updated with one shift-or per timed frame. No allocation, no per-frame I/O.


    loop {
        // 1. Pump the OS queue; `false` = WM_QUIT (the window closed).
        if !host.window.pump_events() {
            return;
        }

        // The automated-run frame cap (counted at the loop TOP, so even a
        // minimized-window pump-only spin terminates). Unset ⇒ `None` ⇒ inert.
        if let Some(cap) = max_frames {
            if frames_run >= cap {
                return;
            }
            frames_run += 1;
        }

        // Step 1: drain the captured OS input (host plan R6, Decision 1). WITH an
        // `InputPlugin` (a World `RawInputQueue`), bridge every drained message
        // into the queue — `update_action_state` on `Main` folds it into this
        // frame's `PhysicalInput` snapshot, and the ECS-native `quit_on_action`
        // (in `FlyCameraPlugin`) sets `AppExit` on `FlyAction::Quit`; the runner
        // stays input-agnostic (its step-3 `AppExit` check handles it). WITHOUT a
        // queue (an input-free scene: `room.rs` / `clear.rs`), keep a lightweight
        // inline Escape scan → return. The two paths are non-redundant: the ECS
        // path when input is present, the inline scan only when it is absent.
        if app.world().contains_resource::<RawInputQueue>() {
            let queue = app.world_mut().resource_mut::<RawInputQueue>();
            // Reborrow `queue` per call: the `FnMut` closure runs once per drained
            // message, so it cannot move the `&mut` out.
            host.window
                .drain_input(|captured| ingest_captured(&mut *queue, captured));
        } else {
            let mut escape = false;
            host.window.drain_input(|captured| {
                if let CapturedMsg::Raw { msg, wparam, lparam } = captured
                    && let Some(RawInputEvent::Key {
                        code: KeyCode::Escape,
                        state: ButtonState::Pressed,
                        ..
                    }) = translate_win32(msg, wparam, lparam)
                {
                    escape = true;
                }
            });
            if escape {
                return;
            }
        }

        // 2. The ECS frame with the real wall delta (Time clamps/scales it).
        let now = Instant::now();
        let dt = now - last;
        last = now;
        // Asset-streaming plan F6 Decision 1/2: publish the fence clock BEFORE the
        // ECS frame runs, so `apply_refcount_deltas` (inside `update_with_delta`)
        // stamps every newly-`Retiring` row's `retire_frame` from THIS frame's
        // epoch — read before this frame's own submit, matching the fence-gate
        // proof (`retire_deferred_frees`'s doc).
        *app.world_mut().resource_mut::<RenderEpoch>() = RenderEpoch(host.renderer.submission_epoch());
        app.update_with_delta(dt);

        // 3. `AppExit` check — after the frame completes, before the present.
        if app.world().resource::<AppExit>().0 {
            return;
        }

        // Minimized window (0×0 client): skip the fence + uploads + render,
        // keep pumping (plan runner-frame note).
        if host.window.width() == 0 || host.window.height() == 0 {
            host.window.refresh_size();
            continue;
        }

        // 4. Mint the frame-write token — the slot fence wait that makes the
        //    per-slot mapped writes below race-free (the `80bf033` class).
        let token = match host.renderer.wait_frame_in_flight() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("boyko_app: frame fence wait failed - exiting ({e:?})");
                return;
            }
        };
        let s = token.slot();

        // 4.5. Asset-streaming plan F6 Decision 2: the fence-gated deferred-free
        // drain — MUST run after `wait_frame_in_flight` above (the fence-gate
        // proof's precondition) and before any per-frame mesh-handle resolve
        // below (the mesh buffer-ptr resolve, the hwrt TLAS gather). Reads the
        // SAME epoch just published above `app.update_with_delta` (no new GPU
        // submit occurred between the two reads).
        retire_deferred_frees(
            app.world_mut(),
            ctx,
            host.renderer.submission_epoch(),
            &mut host.retire_scratch,
        );

        // 4.6/4.7 pre-checks. Asset-streaming plan F7 review W1 (MUST-FIX): read only
        // `Copy` scalars through CHEAP, allocation-free resource accessors
        // (`resource`/`non_send_resource[_mut]` deref through the slab, `#[inline]`) —
        // deciding WHETHER either GPU mirror needs to grow BEFORE paying for any NonSend
        // take-out/reinsert. `NonSendResources::insert`/`remove` heap-(de)allocate the
        // box and are `#[cold]` (setup-only APIs in the kernel) — correct to pay on the
        // rare grow frame, wrong to pay every frame on the golden/steady-state path.
        let material_high_water = app.world().resource::<Assets<Material>>().high_water();
        let material_grow_needed = app
            .world()
            .non_send_resource::<MaterialTable>()
            .needs_grow(material_high_water);
        let instance_needed = app.world().resource::<MeshRenderScratch>().instance_count() as u32;
        let instance_grow_needed = host.gpu.needs_instance_grow(instance_needed, s);

        // 4.6/4.7 grow (rare path only). `MaterialTable::grow_if_needed` needs `&Assets<
        // Material>` (a Send Resource) + `&mut MaterialTable` + `&mut
        // RetiredGpuBuffers` (both NonSend) live at once — the safe resource facade
        // cannot split a live `&World` borrow across Send/NonSend storage, so
        // `MaterialTable`/`RetiredGpuBuffers` are taken out (owned) for the call's
        // duration and reinserted right after (the same boot `Assets<Material>`
        // take-out/reinsert idiom above) — but ONLY inside this `if`, so a non-growing
        // frame never executes it. `RetiredGpuBuffers` is taken out ONCE and shared by
        // both grows (never twice per frame).
        if material_grow_needed || instance_grow_needed {
            let mut retired = app
                .world_mut()
                .remove_non_send_resource::<RetiredGpuBuffers>()
                .expect("invariant: RetiredGpuBuffers inserted at boot");

            if material_grow_needed {
                let mut material_table = app
                    .world_mut()
                    .remove_non_send_resource::<MaterialTable>()
                    .expect("invariant: MaterialTable inserted at boot");
                {
                    let material_assets = app.world().resource::<Assets<Material>>();
                    material_table.grow_if_needed(
                        material_assets,
                        ctx,
                        &mut retired,
                        host.renderer.submission_epoch(),
                    );
                }
                app.world_mut().insert_non_send_resource(material_table);
            }

            if instance_grow_needed {
                // SAFETY: `token` proves slot `token.slot()`'s fence was waited THIS
                // frame (the `wait_frame_in_flight` call above) — every set this may
                // repoint is non-pending.
                unsafe {
                    host.gpu.grow_instance_family_if_needed(
                        instance_needed,
                        ctx,
                        &token,
                        &mut retired,
                        host.renderer.submission_epoch(),
                    );
                }
            }

            app.world_mut().insert_non_send_resource(retired);
        }

        // 4.6 repoint (every frame, cheap). FIX-E (F7 §8, load-bearing): gated ONLY on
        // `rebind_pending`, never on `material_grow_needed` THIS frame or on dirty state
        // — a slot left lagging by a PRIOR frame's grow must still be repointed before
        // its set is recorded (invariant (b) of the FIF-rebind proof). Cheap: only
        // `non_send_resource[_mut]` derefs, no take-out/reinsert.
        if host.frame.targets_ready() {
            let rebind = app
                .world_mut()
                .non_send_resource_mut::<MaterialTable>()
                .take_rebind_pending(s);
            if rebind {
                let material_table = app.world().non_send_resource::<MaterialTable>();
                // SAFETY: `s == token.slot()` fenced this frame (`wait_frame_in_flight`
                // above) — its sets are non-pending; `table()` reads the CURRENT
                // (possibly just-grown) buffer.
                unsafe {
                    host.frame.repoint_material_table(ctx, s, material_table.table());
                }
            }

            // Asset-streaming plan F7-hwrt (task#11): the AS-handle counterpart of the
            // material repoint above — gated ONLY on `tlas_accel_rebind_pending[s]`
            // (`core::mem::take` clears it), NEVER on "grew this frame", so a slot left
            // lagging by a PRIOR frame's TLAS grow still converges (the SAME FIX-E
            // discipline `MaterialTable::rebind_pending` uses).
            #[cfg(feature = "hwrt")]
            if core::mem::take(&mut host.gpu.tlas_accel_rebind_pending[s]) {
                // SAFETY: `s == token.slot()` fenced this frame (`wait_frame_in_flight`
                // above) — its sets are non-pending; `current_tlas_accel(s)` reads the
                // CURRENT (just-grown) persistent TLAS.
                unsafe {
                    host.frame.repoint_tlas_accel(ctx, s, host.gpu.current_tlas_accel(s));
                }
            }
        }

        // 5-pre. The R7 SDF edit list — the ONE-SHOT boot-static write (host plan R7).
        // The explicit post-`finish()` `collect_sdf_edits` (run in `run_windowed` above,
        // after ALL startup spawns drained) gathered every `SdfPrimitive` into
        // `SdfEditStaging` and set `dirty` iff any SDF primitive was spawned. On the FIRST
        // frame, encode + write the marcher's binding-0 edit-list SSBO ONCE (before the
        // first `render_gbuffer_frame` below — same frame, uploads precede render), then
        // `mark_uploaded()` so `is_dirty()` stays false: the frame loop touches the SDF
        // path never again (0 per-frame cost, v1 boot-static).
        //
        // Deliberate deviation (v1): the design's post-boot-spawn debug_assert is replaced
        // by this `is_dirty()` one-shot gate. A re-query to catch a post-boot spawn would
        // perturb change-detection ticks (running a system stamps ticks), and v1 scope is
        // boot-static — so a post-boot `SdfPrimitive` spawn is silently ignored (the scope
        // line; the dynamic per-frame edit path is a deferred campaign).
        //
        // Textured-PBR rung T5 (D6): the textured path is mesh-only in v1 — the deferred
        // resolve cannot distinguish an SDF-marched pixel from a mesh-rasterized one, so a
        // TEXTURED material bound to an SDF primitive would silently misbehave once a later
        // rung wires texture sampling into the resolve. `collect_sdf_edits`
        // (`boyko_render::sdf_edit`) itself has NO `Assets<Material>` access — adding one
        // would panic every headless SDF-gather harness that does not insert that resource
        // (this kernel has no `Option<Res<R>>` SystemParam; `Res<T>::get_param` panics on a
        // missing resource) — so this frame-loop site (which already holds BOTH the
        // gathered edits and the world's `Assets<Material>`) is the nearest place this check
        // can run without perturbing any existing caller. Debug-only, compiled out in
        // release; the whole block runs at most once (the first `is_dirty()` frame).
        #[cfg(debug_assertions)]
        {
            let staging = app.world().resource::<SdfEditStaging>();
            if staging.is_dirty() {
                let material_assets = app.world().resource::<Assets<Material>>();
                for edit in staging.edits() {
                    let id = MaterialId::from_center_w_bits(edit.center[3]);
                    let textured = material_assets
                        .get_by_index(u32::from(id.index()))
                        .is_some_and(|m| m.gpu.mrr[3].to_bits() & MATERIAL_FLAG_TEXTURED != 0);
                    debug_assert!(
                        !textured,
                        "SDF primitive carries material id {} which is TEXTURED — the \
                         textured path is mesh-only in v1 (the deferred resolve cannot \
                         distinguish an SDF-marched pixel from a mesh-rasterized one)",
                        id.index()
                    );
                }
            }
        }
        {
            let staging = app.world_mut().resource_mut::<SdfEditStaging>();
            if staging.is_dirty() {
                // SAFETY: `host.gpu.edit_list()` is a live host-visible buffer minted by
                // `GpuSceneBundles::boot` (`RhiDevice::create_buffer`, HostVisibleCoherent
                // — its `mapped`/`size` are the RHI's own) and destroyed only in teardown
                // after the loop. This is the ONE-SHOT boot-static write, run under the
                // fenced token BEFORE the first marcher dispatch reads the buffer, so no
                // in-flight GPU read of a non-empty edit list is racing it (the SSBO was
                // boot-seeded EMPTY; nothing else rewrites this single shared buffer).
                unsafe {
                    upload_sdf_edit_list(&token, host.gpu.edit_list(), staging.edits());
                }
                staging.mark_uploaded();
            }
        }

        // 5–7. Uploads + stack scene assembly + render. The draw list reuses
        // the host's parked allocation (0 alloc/frame after warmup); its
        // elements borrow the World's `Assets<MeshGpu>` buffers for this frame.
        // The two flags feed the post-present `HostFrameStats` publish (step 8):
        // the light flag is set on the gated branch; the csm flag is assigned
        // exactly once inside the block (definite-initialization, no dead seed).
        let mut frame_light_uploaded = false;
        let frame_csm_armed;
        // The punctual-armed probe (the punctual host rung): assigned exactly once inside the
        // block (definite-initialization, no dead seed), mirroring `frame_csm_armed`.
        let frame_punctual_armed;
        // The interp-armed probe (host plan R5): set when this frame's pair gather
        // produced instances (the pair ring was uploaded + `scene.interp` armed).
        let frame_interp_armed;
        // VG R3 piece 2 step P2-6: this frame's marked-instance count, carried OUT of the render
        // block so gate G2's artifact can record the host's own view beside the recorder's. Read
        // from the SAME `scratch.occlusion_instances()` call that threads it into the scene, so
        // the file cannot report a number the frame did not use.
        //
        // DEFINITE-INITIALIZED, not seeded. An earlier draft wrote `= 0` and argued the seed was
        // the truthful answer on a minimized (0x0) window, where the block below skips scene
        // assembly. That argument is false and the compiler says so: every path that READS this
        // reads it inside the block that assigns it, so the seed was dead — `unused_assignments`
        // fired on it. Leaving the declaration bare makes the property a build error rather than a
        // comment: if a future edit adds a read on the skip path, this stops compiling instead of
        // silently reporting a zero the frame never computed. Not `mut` either — with the seed gone
        // the binding is assigned exactly once, which is the shape the value actually has.
        let frame_occlusion_instances: u32;
        // VG R3 piece 4 rung P4-4: THIS frame's occlusion regime, as the HOST saw it — carried out
        // of the render block for two readers that are not the renderer: the record probe's
        // `[host]` table (which is compared against the RECORDER's own stamped word, so the
        // artifact holds two independent derivations rather than one site agreeing with itself)
        // and the bench summary's `VB-P4 regime` line.
        //
        // DEFINITE-INITIALIZED, not seeded, for the reason the binding above states: every read is
        // downstream of the block that assigns them, so a seed would be dead and
        // `unused_assignments` — a `-D warnings` gate — would say so.
        let frame_occ_mode: boyko_render::OcclusionMode;
        let frame_occ_force: OcclusionForce;
        // VG R3 piece 3 step P3-5: `true` on the ONE frame the cull-readback probe captures.
        //
        // DEFINITE-INITIALIZED for verbatim the reason the binding above is, and it is not a style
        // choice: the block below assigns it on every path that reaches it, so a `= false` seed
        // would be dead and `unused_assignments` — a `-D warnings` gate — would say so. The
        // minimized (0×0 client) iteration never gets here at all; it `continue`s far above.
        let vb_cull_capture: bool;
        // The dump's readback request (cold; `None` without the env knob). The
        // returned borrow holds `dump` until the render call consumes it.
        let readback = match dump.as_mut() {
            Some(d) => d.request(ctx, host.swapchain.extent()),
            None => None,
        };
        // The census's own request. `present_extent` (the COMPOSITE), not the swapchain extent:
        // the `vb_id` ring is sized to the composite, which under armed SSAA is 2x the client
        // area — the route the plan's §9.1 grant table takes to the top two ladder rungs.
        let vb_id_readback = match census.as_mut() {
            Some(c) => c.request(ctx, present_extent),
            None => None,
        };
        // Anti-aliasing Stage 4 (TAA W2): the resolved mode read early — BEFORE the render block
        // borrows `&World` immutably — so the jitter advance below can take `&mut World`.
        // `try_resource` degrades to `Off` on a host that never composes `AaPlugin` (mirrors
        // `resolved_aa_mode`'s later, fuller read for `host.gpu.scene`; SSAA's host-lock does not
        // apply to TAA, so this narrower read is sufficient here).
        // TAA-under-VB: AND-gated on `ResolvedRenderPath::taa_supported()` — the SINGLE
        // predicate the resolver cap and `GpuSceneBundles::scene`'s degrade also read, so the
        // jitter advance, `TaaState`, the MotionCam advance, the taa-UBO uploads AND the C1
        // basis shear are all path-coherent from this one site. This also silently fixes the
        // pre-existing Forward+Taa wobble where the shear armed with no accumulator (no pin
        // ever exercised that combination).
        let taa_armed_now = app
            .world()
            .try_resource::<ResolvedAa>()
            .map(|r| r.mode)
            .unwrap_or_default()
            == AaMode::Taa
            && host.resolved_render_path.taa_supported();
        // Anti-aliasing Stage 4 (TAA W5): the history-reset transition detection — captured
        // BEFORE `advance_jitter` (below) overwrites `JitterState.armed`, so `taa_was_armed` is
        // exactly "was Taa armed LAST frame". `!taa_was_armed` catches a transition INTO `Taa`;
        // `extent_changed` catches a resize (`taa_hist`'s allocated shape just changed under
        // `sync_gbuffer`, so the sibling parity slot's prior contents are meaningless at the new
        // resolution). Either forces `TaaState::mark_reset()`, so the resolve replaces rather
        // than blends its first post-transition frame (mirrors the shadow-temporal denoiser's I5
        // disocclusion fallback). `TaaState::advance()` then runs every frame regardless of arm
        // state (cheap, cold — a checked-and-cleared flag, not a sticky one), consuming this
        // frame's reset bit into `taa_reset_flag` for `host.gpu.scene`'s `taa_reset` param below.
        let taa_was_armed = app.world().try_resource::<JitterState>().is_some_and(|j| j.armed);
        let mut taa_reset_flag = false;
        if let Some(taa_state) = app.world_mut().try_resource_mut::<TaaState>() {
            let extent_changed = taa_state.extent_changed(cw, ch);
            if taa_armed_now && (!taa_was_armed || extent_changed) {
                taa_state.mark_reset();
            }
            taa_reset_flag = taa_state.advance();
        }
        // TAA W2: advance the jitter phase ONCE per frame (cold) — structurally gated on
        // `taa_armed_now` (`advance_jitter`'s `armed` param): an armed frame cycles `phase`
        // through `HALTON_8`; a disarmed frame freezes `phase` and clears `JitterState.armed`,
        // so `ndc_jitter` returns the exact-zero offset below (OFF byte-identity). Read via the
        // fallible `try_resource_mut` — CONSISTENT with the `taa_armed_now` guard above (M1): a
        // host that never composes `AaPlugin` has no `JitterState` AND `taa_armed_now == false`,
        // so skipping the advance (freezing the phase) is exactly correct — never a panic.
        if let Some(jitter) = app.world_mut().try_resource_mut::<JitterState>() {
            advance_jitter(jitter, taa_armed_now);
        }
        // HW-RT Rung 3b step 5a: advance the MESH motion-vector camera pair BEFORE the render
        // block (the `advance` needs `&mut World` to persist this frame's `cur` as next frame's
        // `prev`; the render block borrows `&World`). Computed ONLY when temporal is on AND the
        // MV ring exists (an RT + storage device) — else `None`, and the runner uploads nothing +
        // the recorder takes the base 3-MRT raster (byte-identical). `marcher_view_proj_rows` is
        // the marcher-aligned proj·view the shaders reproject against (I-O1 majorness pin).
        #[cfg(feature = "hwrt")]
        let mv_cam = {
            let temporal_on = app
                .world()
                .resource::<boyko_render::ShadowDenoiseConfig>()
                .temporal_enabled();
            if temporal_on && host.gpu.motion_vec_slots(0).is_some() {
                let view = *app.world().resource::<ViewUniform>();
                let cur = boyko_render::marcher_view_proj_rows(&view, cw, ch);
                Some(
                    app.world_mut()
                        .resource_mut::<boyko_render::MotionCamState>()
                        .advance(cur),
                )
            } else {
                None
            }
        };
        // Anti-aliasing Stage 4 (TAA W5): TAA's OWN `MotionCam` pair — REUSES `mv_cam`'s result
        // when the hwrt mesh-shadow MV producer already advanced `MotionCamState` THIS frame
        // (same camera, same frame ⇒ same pair; a second `advance()` call would double-consume
        // the ONE-call-per-frame contract and corrupt `prev` — see `TaaActivation`'s "why a
        // dedicated ring" doc). Otherwise (mv_cam absent — `not(hwrt)`, or hwrt with temporal
        // off) advances `MotionCamState` itself, exactly once. `None` when TAA is not armed this
        // frame (the byte-identical 0%-gate: no advance beyond the one above, no upload).
        let taa_motion_cam: Option<boyko_render::MotionCam> = if taa_armed_now {
            #[cfg(feature = "hwrt")]
            {
                if let Some(cam) = mv_cam {
                    Some(cam)
                } else {
                    let view = *app.world().resource::<ViewUniform>();
                    let cur = boyko_render::marcher_view_proj_rows(&view, cw, ch);
                    Some(
                        app.world_mut()
                            .resource_mut::<boyko_render::MotionCamState>()
                            .advance(cur),
                    )
                }
            }
            #[cfg(not(feature = "hwrt"))]
            {
                let view = *app.world().resource::<ViewUniform>();
                let cur = boyko_render::marcher_view_proj_rows(&view, cw, ch);
                Some(app.world_mut().resource_mut::<boyko_render::MotionCamState>().advance(cur))
            }
        } else {
            None
        };
        // Multi-paradigm render-path plan, rung R8 (Decision 0): builds the VB-path instance
        // ring from the SAME gather `gather_mesh_draws` already populated this frame (a PARALLEL
        // fold over `ring`/`mesh_ids`, no second ECS query — `sync_vb_instance_ring_system`'s
        // doc). Run via `World::run_system` (the SAME one-shot idiom `upload_material_assets`/
        // `upload_mesh_assets` use at boot) so the scheduler's disjoint-borrow machinery — not a
        // manual `World::resource_mut`/`World::non_send_resource` pair — resolves the
        // NonSend-asset-table-vs-Resource split. Gated on the boot-resolved path (Decision 1:
        // never a per-frame path branch on FRAMEGRAPH SHAPE, but this is a plain data-prep step,
        // not a shape change) — a Deferred/Forward boot pays zero cost here.
        if host.resolved_render_path.path == boyko_render::RenderPath::VisibilityBuffer {
            app.world_mut().run_system(boyko_render::sync_vb_instance_ring_system);
        }


        let mut draws = host.draw_scratch.take();
        let presented = {
            let world = app.world();
            let view = *world.resource::<ViewUniform>();

            // TAA rung C1: the b5 camera-basis shear -- gated on `TaaConfig::jitter_scope ==
            // RasterAndBasis` AND `taa_armed_now` (the structural skip: `None` on EITHER being
            // false, never a computed `Some([0.0, 0.0])` -- see
            // `composite_perspective_from_view_sheared`'s doc for why the latter is not an
            // equivalent substitute). Reuses the SAME `JitterState`/`ndc_jitter` the raster
            // gbuffer push reads below (I2: the marcher and raster must sample the exact same
            // final-NDC sub-pixel position) -- a second read of the same cold, pure Resource,
            // not a second jitter phase; `JitterState` is guaranteed present whenever
            // `TaaConfig` is (both inserted together by `AaPlugin::build`).
            let basis_shear: Option<[f32; 2]> = if taa_armed_now
                && world.try_resource::<TaaConfig>().is_some_and(|c| c.basis_shear_enabled())
            {
                let jitter_state = *world.resource::<JitterState>();
                let j = ndc_jitter(&jitter_state, cw, ch);
                Some([j.jx, j.jy])
            } else {
                None
            };

            // 5a. The 80-byte b5 camera block into slot `s` (plan D7: the
            //     composite extent, not the window extent).
            // SAFETY: `host.gpu.camera_ring[s]` is a live host-visible buffer
            // minted by `GpuSceneBundles::boot` (`RhiDevice::create_buffer`,
            // HostVisibleCoherent — its `mapped`/`size` are the RHI's own, not
            // hand-built) and destroyed only in teardown after the loop; it IS
            // the fenced slot's buffer (`s == token.slot()`), satisfying both
            // upload preconditions.
            unsafe {
                upload_camera_ring_sheared(
                    &token,
                    &host.gpu.camera_ring[s],
                    &view,
                    cw,
                    ch,
                    basis_shear,
                );
            }

            // 5b. The gathered instance-model ring into slot `s` —
            //     UNCONDITIONAL every frame (plan D5).
            let scratch = world.resource::<MeshRenderScratch>();
            // SAFETY: `host.gpu.instance_rings[s]` — same provenance contract
            // as the camera slot above (boot-minted, live until teardown, the
            // fenced slot `s == token.slot()`).
            unsafe {
                upload_instance_models(&token, &host.gpu.instance_rings[s], scratch);
            }

            // 5b'. Multi-paradigm render-path plan, rung R8 (Decision 0): the gathered VB-path
            //      instance ring — gated on the boot-resolved path (`sync_vb_instance_ring_system`
            //      above already skipped populating `scratch.vb_ring` on any other boot, so
            //      `upload_vb_instance_rows` itself would just upload zero bytes there anyway;
            //      the explicit gate documents intent and avoids the dead upload call).
            if host.resolved_render_path.path == boyko_render::RenderPath::VisibilityBuffer {
                // SAFETY: `host.gpu.vb_instance_rings[s]` — same provenance contract as
                // `instance_rings[s]` above (boot-minted, live until teardown, the fenced slot
                // `s == token.slot()`).
                unsafe {
                    upload_vb_instance_rows(&token, &host.gpu.vb_instance_rings[s], scratch);
                }
            }

            // 5b''. Asset-streaming plan F8: the gathered per-instance material-id lane —
            //       gated on `any_non_default_material` (Principle 1: a default frame does
            //       ZERO material-upload work).
            if scratch.any_non_default_material() {
                // SAFETY: `host.gpu.pm_instance_material_rings[s]` — same provenance
                // contract as `instance_rings[s]` above (boot-minted or F8-grown in
                // lockstep, live until teardown, the fenced slot `s == token.slot()`).
                unsafe {
                    upload_instance_materials(&token, &host.gpu.pm_instance_material_rings[s], scratch);
                }
            }

            // 5b'''. Textured-PBR T6c: the gathered per-instance TEXTURED material payload
            //        lane — gated on `any_textured_material` (Principle 1: a non-textured
            //        frame does ZERO material-upload work). `tex_instance_material_slot`
            //        returns `None` if the TEXTURED pipeline never got built (bindless table
            //        create failure), in which case there is nothing to upload into.
            if scratch.any_textured_material()
                && let Some(tex_slot) = host.gpu.tex_instance_material_slot(s)
            {
                // SAFETY: `tex_slot` — same provenance contract as `pm_instance_material_rings[s]`
                // above (boot-minted, live until teardown, the fenced slot `s == token.slot()`).
                unsafe {
                    upload_instance_materials_tex(&token, tex_slot, scratch);
                }
            }

            // 5b'. The gathered interpolation PAIR ring + OUT-SLOT lane into slot
            //      `s` (refined-B) — UNCONDITIONAL every frame (plan D5; the
            //      fingerprint gate was KILLED). Both are empty on a pure-static
            //      scene (no interpolated body): then `interp_count == 0` arms no
            //      interp pass (byte-identical to interp OFF). The DYNAMIC count is
            //      this frame's dispatch bound + the scene arming key; the interp
            //      compute reads the pair @0 + out-slot @1 and scatters into the
            //      SHARED instance ring @2 (uploaded whole in 5b above).
            let interp_count = scratch.dynamic_count() as u32;
            frame_interp_armed = interp_count > 0;
            // SAFETY: `host.gpu.interp_pair_slot(s)` / `interp_out_slot_slot(s)` —
            // same provenance contract as the instance slot above (boot-minted at
            // INSTANCE_CAPACITY, live until teardown, the fenced slot
            // `s == token.slot()`); the interp compute reads the same slots @0/@1,
            // the sibling frame binds the other slot.
            unsafe {
                upload_pair_ring(&token, host.gpu.interp_pair_slot(s), scratch);
                upload_pair_out_slot(&token, host.gpu.interp_out_slot_slot(s), scratch);
            }

            // 5b''. HW-RT rung R2a-3: the gathered per-instance MESH-ID (BLAS-index) lane
            //       into slot `s` — UNCONDITIONAL on an RT device (the pack path mirror of
            //       5b). The TLAS packer reads it at binding 1 (`MeshIds[i]`) to resolve each
            //       instance's BLAS address; an empty gather writes nothing (then no TLAS this
            //       frame). Skipped entirely on a non-RT device (`mesh_id_slot` is `None`).
            #[cfg(feature = "hwrt")]
            if ctx.ray_query_enabled()
                && let Some(mesh_id_slot) = host.gpu.mesh_id_slot(s)
            {
                // SAFETY: `mesh_id_slot` — same provenance contract as the instance slot above
                // (boot-minted at INSTANCE_CAPACITY on the RT device, live until teardown, the
                // fenced slot `s == token.slot()`); the packer reads the same slot at binding 1,
                // the sibling frame binds the other slot.
                unsafe {
                    upload_mesh_ids(&token, mesh_id_slot, scratch);
                }
            }

            // 5b'''. HW-RT Rung 3b step 5a: the MESH motion-vector uploads into slot `s` — the
            //        gathered PREV-instance ring (index-aligned with the current ring) + the
            //        MotionCam view-proj pair. GATED on `mv_cam` being `Some` (temporal-on AND the
            //        MV ring exists — the SAME gate the recorder binds the MV pipeline under), so a
            //        temporal-OFF frame (or a non-RT / non-storage device) writes NOTHING and the
            //        base 3-MRT raster draws (byte-identical). `motion_vec_slots(s)` returns the
            //        FENCED slot's prev-instance ring + motion-cam UBO; the MV bind group binds the
            //        SAME slots @1/@2.
            #[cfg(feature = "hwrt")]
            if let Some(cam) = mv_cam.as_ref()
                && let Some((prev_slot, cam_slot)) = host.gpu.motion_vec_slots(s)
            {
                // SAFETY: `prev_slot` / `cam_slot` — same provenance contract as the instance slot
                // above (boot-minted at INSTANCE_CAPACITY / MOTION_CAM_UBO_BYTES on the RT+storage
                // device under the MV gate, live until teardown, the fenced slot `s == token.slot()`);
                // the MV VS reads the same slots @1/@2, the sibling frame binds the other slot.
                unsafe {
                    boyko_render::upload_prev_instance_models(&token, prev_slot, scratch);
                    boyko_render::upload_motion_cam_ring(&token, cam_slot, cam);
                }
            }

            // 5c. The light staging into slot `s` — GEN-GATED (plan D5, R4):
            //     rewritten only when this slot's uploaded generation lags the
            //     writer-side `LightTableGeneration` (`collect_lights` bumps it
            //     once per actual staged rewrite). The staging is a per-slot
            //     RING: a single instance rewritten on frame N+1 would race
            //     frame N's still-in-flight recorded staging→table copy (the
            //     host-write-vs-GPU-transfer-read class); slot `s`'s buffer is
            //     only read by slot-`s` frames, whose fence the token proves.
            let generation = world.resource::<LightTableGeneration>().0;
            let light_upload = if light_upload_due(&mut host.light_uploaded_gen, s, generation)
            {
                let staged = world.resource::<LightTableStaging>();
                let bytes = staged.bytes();
                // SAFETY: `host.gpu.light_staging[s]` — same provenance
                // contract as the camera slot above (boot-minted at the full
                // table capacity, live until teardown, the fenced slot
                // `s == token.slot()`); `bytes` is the staging resource's own
                // preallocated scratch, sized <= that same capacity.
                unsafe {
                    upload_light_table(&token, &host.gpu.light_staging[s], bytes);
                }
                frame_light_uploaded = true;
                Some(bytes.len() as u64)
            } else {
                None
            };

            // 5d. The CSM cascade UBO into slot `s` — UNCONDITIONAL every
            //     frame (336 B; the fit is recomputed from the live camera by
            //     `resolve_csm_cascades`, so a boot-seed would go stale — see
            //     `upload_csm_ring`'s rationale). A DISABLED selection uploads
            //     as all-zero, the bound-but-unread OFF state.
            let resolved_csm = world.resource::<ResolvedCsm>();
            // SAFETY: the cascade UBO ring slot — same provenance contract as
            // the camera slot above (boot-minted at RESOLVED_CSM_BYTES, live
            // until teardown, the fenced slot `s == token.slot()`).
            unsafe {
                upload_csm_ring(&token, host.gpu.csm_ubo_slot(s), resolved_csm);
            }

            // 5d'. The punctual shadow-atlas UBO into slot `s` — UNCONDITIONAL
            //      every frame (1296 B; the fit is camera-dependent — the
            //      `spot_priority` top-K shifts with the camera — so a boot-seed
            //      would go stale, see `upload_atlas_ring`'s rationale). A DISABLED
            //      selection uploads as all-zero, the bound-but-unread OFF state.
            let resolved_atlas = world.resource::<ResolvedShadowAtlas>();
            // SAFETY: the atlas UBO ring slot — same provenance contract as the
            // cascade slot above (boot-minted at RESOLVED_SHADOW_ATLAS_BYTES, live
            // until teardown, the fenced slot `s == token.slot()`).
            unsafe {
                upload_atlas_ring(&token, host.gpu.atlas_ubo_slot(s), resolved_atlas);
            }

            // 5d''. HW-RT rung 1b/3b: the HWRT soft-shadow-params UBO into slot `s` —
            //       UNCONDITIONAL every HWRT frame (20 B; `resolve_ray_shadow_system`
            //       re-derives the 16-byte resolved mirror from the author
            //       `RayShadowConfig`, so a boot-seed would go stale on a retune, see
            //       `upload_ray_shadow_ring`). The rung-3b `frame_index` seed rides
            //       along in the SAME upload (the runner's own monotonic counter, hot
            //       per-frame — not resolve-derived) so the shadow ray's cone rotation
            //       advances by the golden angle every frame, giving the temporal
            //       shadow denoiser something to average. GATED on an RT device
            //       (`ray_query_enabled`) — the SAME gate that mints the ring in
            //       `GpuSceneBundles::boot`, so an unminted slot is never uploaded; a
            //       software-only build pays zero (the whole block is
            //       `#[cfg(feature = "hwrt")]`).
            #[cfg(feature = "hwrt")]
            if ctx.ray_query_enabled() {
                let resolved_ray_shadow = world.resource::<ResolvedRayShadow>();
                // SAFETY: the HWRT shadow-params UBO ring slot — same provenance
                // contract as the cascade slot above (boot-minted at
                // RAY_SHADOW_UBO_BYTES (32 B, room for the 20 B written) on the RT
                // device under this same gate, live until teardown, the fenced slot
                // `s == token.slot()`).
                unsafe {
                    upload_ray_shadow_ring(
                        &token,
                        host.gpu.ray_shadow_ubo_slot(s),
                        resolved_ray_shadow,
                        frame_index,
                    );
                }

                // 5d'''. HW-RT rung 3a step 7: the à-trous edge-stop UBO (`sigma_z`/`sigma_n`)
                //        into the renderer's `shadow_denoise_ubo[s]` — the per-level à-trous sets
                //        bind slot `s` @4. `resolve_shadow_denoise_policy` re-derives it from the
                //        author `ShadowDenoiseConfig` each frame (a boot-seed would go stale on a
                //        retune, mirroring `upload_ray_shadow_ring`). `shadow_denoise_ubo_slot`
                //        is `None` until the first frame syncs the targets (frame 0) OR on a
                //        device lacking RG16 storage (`shadow_denoise_storage_ok()`) — in both the
                //        denoise pass is not recorded, so the (absent) slot is never read. GATED on
                //        `ray_query_enabled()` — the SAME gate that mints the ring in
                //        `GBufferTargets::build_shadow_denoise_sets`.
                if let Some(denoise_slot) = host.frame.shadow_denoise_ubo_slot(s) {
                    let resolved_denoise = world.resource::<ResolvedShadowDenoise>();
                    // SAFETY: `denoise_slot` is the renderer's `shadow_denoise_ubo[s]` — a live
                    // host-coherent >= RESOLVED_SHADOW_DENOISE_BYTES UNIFORM buffer minted under
                    // this same `ray_query_enabled()` gate, live until the targets are torn down
                    // (device-idle). The FENCED slot `s == token.slot()`: the borrowed
                    // `FrameWriteToken` proves this slot's in-flight fence was waited THIS frame
                    // (the previous occupant's à-trous reads retired; the sibling frame binds the
                    // other slot) — the same borrow-is-fence-proof shape as `upload_ray_shadow_ring`.
                    unsafe {
                        upload_shadow_denoise_ring(&token, denoise_slot, resolved_denoise);
                    }
                }

                // 5d''''. HW-RT Rung 3b step 6: the temporal reproject scalars
                //         (`feedback_max`/`feedback_min`/`variance_gamma`/`depth_tol`) into the
                //         renderer's `temporal_shadow_ubo[s]` — the temporal set binds slot `s` @6.
                //         `resolve_temporal_shadow_policy` re-derives it from the author
                //         `ShadowDenoiseConfig` each frame (a boot-seed would go stale on a retune,
                //         mirroring the à-trous upload above). `temporal_shadow_ubo_slot` is `None`
                //         until the targets sync (frame 0) OR when the temporal denoise is not armed
                //         (the ring was never minted) — in both the temporal pass is not recorded, so
                //         the (absent) slot is never read.
                if let Some(temporal_slot) = host.frame.temporal_shadow_ubo_slot(s) {
                    let resolved_temporal = world.resource::<ResolvedTemporalShadow>();
                    // SAFETY: `temporal_slot` is the renderer's `temporal_shadow_ubo[s]` — a live
                    // host-coherent >= RESOLVED_TEMPORAL_SHADOW_BYTES UNIFORM buffer minted under this
                    // same `ray_query_enabled()` gate (in `build_shadow_temporal_sets`), live until the
                    // targets are torn down (device-idle). The FENCED slot `s == token.slot()`: the
                    // borrowed `FrameWriteToken` proves this slot's in-flight fence was waited THIS
                    // frame (the previous occupant's temporal reproject retired; the sibling frame binds
                    // the other slot) — the same borrow-is-fence-proof shape as `upload_shadow_denoise_ring`.
                    unsafe {
                        upload_temporal_shadow_ring(&token, temporal_slot, resolved_temporal);
                    }
                }
            }

            // 5d'''''. Anti-aliasing Stage 4 (TAA W5): the resolve's tunables UBO
            // (`ResolvedTaa`) + its DEDICATED `MotionCam` UBO — into the renderer's
            // `taa_ubo[s]`/`taa_motion_cam_ubo[s]`. NOT `hwrt`-gated (mirrors the à-trous/
            // temporal uploads above, minus the feature gate). `taa_ubo_slot`/
            // `taa_motion_cam_ubo_slot` are `None` until the targets sync (frame 0) OR TAA is
            // not armed (the rings were never minted) — in both the resolve is not recorded, so
            // the (absent) slots are never read. `taa_motion_cam` is `None` on the SAME
            // `!taa_armed_now` condition (computed above, before this block), so the two `if
            // let`s below either both fire or both stay silent.
            if let Some(taa_ubo_slot) = host.frame.taa_ubo_slot(s) {
                let resolved_taa = world.resource::<ResolvedTaa>();
                // SAFETY: `taa_ubo_slot` is the renderer's `taa_ubo[s]` — a live host-coherent
                // >= RESOLVED_TAA_BYTES UNIFORM buffer minted when `scene.taa` first armed (in
                // `build_taa_resolve_set`), live until the targets are torn down (device-idle).
                // The FENCED slot `s == token.slot()`: the borrowed `FrameWriteToken` proves
                // this slot's in-flight fence was waited THIS frame (the previous occupant's
                // resolve retired; the sibling frame binds the other slot) — the same
                // borrow-is-fence-proof shape as `upload_shadow_denoise_ring`.
                unsafe {
                    upload_taa_ring(&token, taa_ubo_slot, resolved_taa);
                }
            }
            if let Some(mc_slot) = host.frame.taa_motion_cam_ubo_slot(s)
                && let Some(cam) = taa_motion_cam.as_ref()
            {
                // SAFETY: `mc_slot` is the renderer's `taa_motion_cam_ubo[s]` — same provenance
                // contract as `taa_ubo_slot` above (minted alongside it in
                // `build_taa_resolve_set`, live until teardown, the fenced slot `s ==
                // token.slot()`).
                unsafe {
                    upload_motion_cam_ring(&token, mc_slot, cam);
                }
            }

            // 6a. DrawBatch → GBufferMeshDraw: resolve each batch's mesh to its
            //     asset-table GPU buffers (the showcase's ~8070 conversion, driven
            //     by the ECS gather instead of a test-built list). R4:
            //     `casts_shadow` is driven by the PRODUCTION `ShadowCaster`
            //     gather — a mesh whose batch appears in `CsmCasterScratch`
            //     casts; a receiver-only mesh (no `ShadowCaster` row) does not
            //     stamp itself into the cascades. Both batch lists are emitted
            //     in ascending `mesh_id` order, so one merge walk (zero alloc)
            //     resolves the flag. NOTE the recorder's per-BATCH granularity:
            //     a mesh with ANY caster instance casts with ALL its visible
            //     instances (mixed caster/receiver instances of one mesh are
            //     not separable on this path — split the mesh id if needed).
            let mesh_assets = world.non_send_resource::<Assets<MeshGpu>>();
            // Asset-system rung A1: the World-owned material GPU mirror, threaded into
            // `scene()` below so `GBufferScene.material_table` binds its device SSBO
            // instead of a boot-owned buffer.
            let material_table = world.non_send_resource::<MaterialTable>();
            let casters = world.resource::<CsmCasterScratch>();
            let caster_batches = casters.batches();
            // VG rung R2c: the instance ring the per-batch AABB fold reads. Bound ONCE outside
            // the loop — every batch indexes the same slice by its own `base_instance`.
            let instance_ring = scratch.ring.as_read_slice();
            let mut ci = 0usize;
            for b in scratch.batches.as_read_slice() {
                while ci < caster_batches.len() && caster_batches[ci].mesh_id < b.mesh_id {
                    ci += 1;
                }
                let casts_shadow =
                    ci < caster_batches.len() && caster_batches[ci].mesh_id == b.mesh_id;
                // INVARIANT (asset-streaming plan F6 FIX-2): this resolve must NEVER
                // dereference a non-Loaded slot. `scratch.batches` was gathered earlier
                // THIS frame's `app.update_with_delta`, but `retire_deferred_frees` (run
                // just before this block, host plan step 4.5) may have since retired a
                // mesh whose carrier `validate_asset_refs` (a best-effort net, not a hard
                // guarantee) failed to disable in time — `try_get` + a graceful skip
                // makes that safe by construction, independent of validation timing.
                let Some(mesh) = mesh_assets.try_get(MeshHandle(b.mesh_id)) else {
                    continue;
                };
                draws.push(GBufferMeshDraw {
                    vertex_buffer: &mesh.vertex_buffer,
                    index_buffer: &mesh.index_buffer,
                    index_count: b.index_count,
                    index_type: b.index_type.as_i32(),
                    base_instance: b.base_instance,
                    instance_count: b.instance_count,
                    casts_shadow,
                    // VG rung R2c: this batch's world AABB, folded from the SAME instance ring the
                    // raster draws and through the SAME Arvo transform the CSM caster fit uses
                    // (`arvo_transform`, shared so the two can never drift). `None` when the fold
                    // declines — a zero-instance batch or the C0 zero-vertex sentinel — and the
                    // recorder then writes the UNBOUNDED corners, which survive every plane.
                    world_aabb: boyko_render::csm_caster::batch_world_aabb(
                        b,
                        instance_ring,
                        (mesh.local_min, mesh.local_max),
                    ),
                });
            }

            // 6a'. The cascade depth-pass arming predicate (R4): a fitted sun
            //      (`ResolvedCsm.csm_mode_word == 1` — CsmConfig enabled AND a
            //      DirectionalLight exists) AND live caster batches. This is
            //      the SAME predicate `sync_csm_light_gate` drives the light-
            //      header csm gate with, so the resolve samples the cascades
            //      only on frame streams where this depth pass transitioned
            //      the cascade texture (capability = presence: no sun or no
            //      casters ⇒ None ⇒ no depth pass recorded at all).
            let csm_armed = resolved_csm.csm_mode_word == 1 && casters.batch_count() > 0;
            frame_csm_armed = csm_armed;
            #[cfg(debug_assertions)]
            if csm_armed {
                // The host cascade texture is boot-fixed at CSM_SHADOW_DIM; a
                // diverging owner-set resolution would skew the fit's
                // texel_size against the real map.
                debug_assert_eq!(
                    world.resource::<boyko_render::CsmConfig>().resolution,
                    crate::gpu_scene::CSM_SHADOW_DIM,
                    "invariant: CsmConfig.resolution matches the host cascade texture"
                );
            }

            // 6a''. The punctual (spot/point) depth-pass arming predicate: a fitted
            //       atlas (`ResolvedShadowAtlas.mode_word == 1` — ShadowConfig
            //       enabled AND at least one `CastsPunctualShadow` light got a slot)
            //       AND live caster batches. The casters are the SAME
            //       `CsmCasterScratch` the cascade arm reads — a `ShadowCaster` mesh
            //       casts into BOTH the cascade array and the punctual atlas, so one
            //       gather feeds both gates. This is the SAME predicate
            //       `sync_punctual_light_gate` drives the light-header punctual bit
            //       with, so the resolve samples the atlas only on frame streams where
            //       this depth pass transitioned the atlas texture.
            let punctual_armed = resolved_atlas.mode_word == 1 && casters.batch_count() > 0;
            frame_punctual_armed = punctual_armed;

            // 6b. The raster push from the SAME resolved view the camera upload
            //     used (the marcher/raster screen alignment is by construction);
            //     `use_model_matrix == 1` iff the instanced batch list draws.
            //     A world with NO perspective camera resolved (the identity
            //     `ViewUniform`, `fov_y == 0` — e.g. a camera-less scene) gets a
            //     ZEROED view_proj: every raster vertex lands at `clip.w == 0`
            //     and is clipped away, so the frame presents the background —
            //     a valid camera-less frame, never a panic (the perspective
            //     bridge is documented perspective-only).
            let instanced = !draws.is_empty();
            let mvp = if view.fov_y > 0.0 {
                // The composite extent (P1-1: BOTH pushes derive their aspect
                // from `(cw, ch)` — the authored Projection aspect is not
                // consulted by the windowed host's pushes).
                //
                // Multi-paradigm render-path plan, rung R4b-b (widened to `ForwardPlus` at rung
                // R5, widened to `VisibilityBuffer` at rung R8 — the bug this comment now
                // documents): a Forward-family-OR-VB-resolved boot builds its push from
                // `forward_gbuffer_push_from_view` (the reverse-Z projection,
                // `boyko_render::view::forward_view_proj_rows`) instead of the Deferred
                // `gbuffer_push_from_view` — a cold, boot-resolved host-side branch (Decision 1:
                // the paths are mutually exclusive per boot, `host.resolved_render_path` never
                // changes mid-run). `vb_raster.vs.hlsl` reads this SAME `scene.mvp` push as its
                // `pc.view_proj` and its pipeline is built `VK_COMPARE_OP_GREATER` against a
                // `0.0` clear (Decision 4, HW reverse-Z) — feeding it the Deferred custom-linear
                // matrix instead produces depth values inconsistent with that reverse-Z GREATER
                // test, failing the depth test for every fragment (zero `vb_id` writes, the
                // rung-R8 GPU regression: the dumped frame degenerated to the sky-only pin
                // because `vb_resolve` saw the sentinel-cleared `vb_id` everywhere).
                //
                // TAA-under-VB: with `taa_armed_now` (structurally true ONLY where
                // `ResolvedRenderPath::taa_supported()` holds — Deferred or VB, any legs) this
                // arm now builds the JITTERED reverse-Z push
                // (`forward_gbuffer_push_from_view_jittered`, rows 0/1 only — the reverse-Z
                // z-row stays byte-untouched, the R8 hard rule above). `vb_resolve`/`vb_shade`
                // re-fetch geometry through this SAME push, so raster sampling and geometry
                // reconstruction stay at the same sub-pixel offset by construction. Under
                // Forward/Forward+ `taa_armed_now` is structurally false (`taa_supported()`),
                // so those paths still never jitter.
                //
                // TAA W2: the STRUCTURAL OFF-skip — a TAA-off Deferred frame calls the plain
                // (non-jittered) fn, not `_jittered` with a zero offset (`no *0.0`, per the
                // byte-identity discipline). `taa_armed_now` was read + `JitterState` advanced
                // BEFORE this block (see above); `world.resource::<JitterState>()` reads the
                // SAME already-advanced phase this frame's jitter derives from.
                if matches!(
                    host.resolved_render_path.path,
                    boyko_render::RenderPath::Forward
                        | boyko_render::RenderPath::ForwardPlus
                        | boyko_render::RenderPath::VisibilityBuffer
                ) {
                    if taa_armed_now {
                        let jitter_state = *world.resource::<JitterState>();
                        let jitter = ndc_jitter(&jitter_state, cw, ch);
                        boyko_render::view::forward_gbuffer_push_from_view_jittered(
                            &view, cw, ch, instanced, jitter,
                        )
                    } else {
                        forward_gbuffer_push_from_view(&view, cw, ch, instanced)
                    }
                } else if taa_armed_now {
                    let jitter_state = *world.resource::<JitterState>();
                    let jitter = ndc_jitter(&jitter_state, cw, ch);
                    gbuffer_push_from_view_jittered(&view, cw, ch, instanced, jitter)
                } else {
                    gbuffer_push_from_view(&view, cw, ch, instanced)
                }
            } else {
                let mut zeroed = [0u8; GBUFFER_PUSH_BYTES];
                if instanced {
                    // The recorder contract: byte 84 (`use_model_matrix`) MUST
                    // be 1 whenever `mesh_draw` is non-empty.
                    zeroed[84..88].copy_from_slice(&1u32.to_le_bytes());
                }
                zeroed
            };

            // Multi-paradigm render-path plan, rung R-SDFFWD: the `sdf_forward_march`
            // `HAS_MESH` push's reverse-Z decode `A`/`B`
            // (`boyko_render::view::forward_view_z_coeffs`), precomputed from the SAME
            // `view.near`/`view.far` the `forward_gbuffer_push_from_view` arm above used to
            // ENCODE the mesh depth this frame (`forward_view_proj_rows`'s own derivation, REUSED
            // verbatim by `vb_raster` — the mvp arm was widened to `VisibilityBuffer` at rung R8) —
            // the algebraic inverse the compute pass's `HAS_MESH` variant applies to the SAMPLED
            // depth.
            //
            // Rung R10: the gate is the SEMANTIC "the `HAS_MESH` march will dispatch this frame"
            // predicate — `sdf_forward_marched && mesh_leg` — NOT a per-path `matches!`. That
            // predicate is exactly what `record_forward`/`record_vb` read to select the `HAS_MESH`
            // pipeline variant (the mesh-less `sdf_only` variant never reads A/B), so it folds in
            // `VisibilityBuffer × Both` (previously excluded — the R8 `matches!`-widening lesson:
            // a hardcoded `Forward | ForwardPlus` here fed VB×Both a degenerate `A = B = 0` decode)
            // WITHOUT re-enumerating paths. Don't-care (`0.0, 0.0`) off it: the `sdf_only` variant
            // ignores them, and a Deferred / camera-less frame never builds this pass at all
            // (`SdfForwardMarchPush::sdf_only`'s doc).
            let (sdf_forward_view_z_a, sdf_forward_view_z_b) = if view.fov_y > 0.0
                && host.resolved_render_path.sdf_forward_marched
                && host.resolved_render_path.mesh_leg
            {
                boyko_render::view::forward_view_z_coeffs(view.near, view.far)
            } else {
                (0.0, 0.0)
            };
            // TAA-under-VB: the `viewt_from_depth_rz` push's reverse-Z decode pair — the SAME
            // `forward_view_z_coeffs` single source as the march pair above, but gated on the
            // TAA-under-VB arm (VB × Mesh never marches, so the pair above stays `(0.0, 0.0)`
            // don't-care exactly when this pass needs real coefficients). `taa_armed_now`
            // already folds `taa_supported()`; the activation resolves to `None` in `scene()`
            // under Deferred AND under the SDF-carrying VB legs (where the VIEWT-variant
            // marcher owns the gViewT lane and reads the march pair above instead), making the
            // pair don't-care there.
            // Rung R9b: the split's SSAO arms `vb_viewt` TAA-independently (the pre-tail slot),
            // so the coefficient pair must be REAL whenever either consumer is armed — the
            // SAME freeze-clamped `ResolvedSsao` the scene() arming reads.
            let vb_split_ssao_armed_now = host.resolved_render_path.mesh_geo_shade_split
                && world.try_resource::<boyko_render::ResolvedSsao>().is_some_and(|r| r.variant.is_some());
            let (vb_viewt_view_z_a, vb_viewt_view_z_b) =
                if view.fov_y > 0.0 && (taa_armed_now || vb_split_ssao_armed_now) {
                    boyko_render::view::forward_view_z_coeffs(view.near, view.far)
                } else {
                    (0.0, 0.0)
                };
            // The lerp alpha (host plan R5): `overstep_fraction()` in [0, 1),
            // sampled in Main AFTER the fixed loop settled (a mid-catch-up read
            // could see overstep >= timestep; the value saturates at
            // 1.0.next_down()). Refreshed EVERY frame via the 8-byte interp push
            // even when the pairs are not re-uploaded. `FixedTime` is inserted at
            // `finish()` (insert-if-absent), so it is always present in the loop.
            let overstep = world.resource::<FixedTime>().overstep_fraction();
            // SDFDDGI I2 (arm): GI is ON when the world carries an ENABLED `DdgiConfig` (the host's
            // config path — the owner/test inserts `DdgiConfig { ddgi_indirect: true, .. }`) AND the
            // device supports B10G11R11/RG16F STORAGE (plan §3 degrade — the atlas was created WITHOUT
            // the STORAGE bit on an unsupported device, so binding it as a storage image would fault;
            // clamp to OFF). Absent (the default host that never composes `DdgiPlugin`), GI is OFF →
            // `ddgi_update = None`, the byte-identical 0%-gate. Even when ON the render stays
            // byte-identical this rung (I3 wires the resolve sample; the atlas is written-but-unread).
            // Rung R9c: the CONFIG half routes through the boot-freeze clamp (warn-once no-op
            // under a non-Deferred path — the SAME `effective_ssao_config` discipline); the
            // device-caps fold stays live.
            let ddgi_cfg_on = world
                .try_resource::<boyko_render::DdgiConfig>()
                .is_some_and(|cfg| cfg.enabled());
            let ddgi_cfg_on = match world.try_resource::<boyko_render::RenderPathFrozenConsumers>() {
                Some(f) => boyko_render::effective_ddgi_enabled(ddgi_cfg_on, f),
                None => ddgi_cfg_on,
            };
            let ddgi_enabled = ctx.device_caps().ddgi_storage_ok() && ddgi_cfg_on;
            // HW-RT rung R2a-3: TLAS arming — hwrt + an RT device + a non-empty gather. On an RT
            // device, first sync the frame-invariant BLAS-address table (a no-op unless the mesh
            // asset table's `install_epoch` advanced — asset-streaming plan F6: gated on install,
            // not row-count growth, so a fence-gated retire+reuse cannot leave a stale, freed BLAS
            // address behind), then arm the per-frame pack + build. On a non-RT device (or hwrt OFF)
            // `tlas_enabled` is `false` → the byte-identical OFF path (no pack, no build, no barrier).
            #[cfg(feature = "hwrt")]
            let tlas_enabled = {
                // Rung 2: fold the config-arbiter read into the TLAS gate. The
                // owner's runtime force-software knob (`RayBackendPolicy`) flows
                // through `resolve_ray_backend_system` into the mesh-shadow cell;
                // when it resolves to `Software` the TLAS is DISARMED here, so the
                // deferred resolve's `scene.tlas.is_some()`-gated `hwrt_triple` falls
                // back to the pure-software shadow path (no pack, no build, no
                // barrier) — a runtime backend flip with zero pipeline rebuild.
                let backend_hw = world
                    .resource::<RayBackendConfig>()
                    .table[RayWorkload::Shadow as usize][RayGeom::Mesh as usize]
                    == RayBackend::HardwareTri;
                // A `HardwareTri` cell can only survive the resolve on an RT device
                // (the `Weak`/`Strong` tier fit routes it, `Absent` stays software),
                // so a hardware cell implies `ray_query_enabled()`.
                debug_assert!(
                    !backend_hw || ctx.ray_query_enabled(),
                    "invariant: a HardwareTri mesh-shadow cell implies an RT device (ray_query_enabled)"
                );
                // The frame-invariant BLAS-address sync runs under `ray_query_enabled()`
                // REGARDLESS of `backend_hw` (boot-time setup; a no-op unless the mesh
                // table's `install_epoch` advanced — F6). Only the per-frame TLAS BUILD
                // is gated by `tlas_enabled`.
                if ctx.ray_query_enabled() {
                    host.gpu.sync_tlas_blas_addr(ctx, mesh_assets);
                }
                ctx.ray_query_enabled() && backend_hw && scratch.instance_count() > 0
            };
            // HW-RT rung 3a/3b step 7: read the author's denoise mode from the world
            // (`ShadowDenoisePlugin` inserts the config). `denoise_armed = spatial || temporal` is the
            // widened arm gate (Rung 3b: Temporal-only still arms the VIS pass + the temporal sets);
            // `atrous_levels = spatial ? clamped_levels() (>=1) : 0` (Temporal-only runs 0 à-trous, so
            // the raw VIS feeds the temporal reproject); `temporal_enabled` is the structural
            // `mode ∈ {Temporal, Both}` predicate (the MESH-MV raster + the temporal pass gate). The
            // OTHER three `scene.shadow` gate conditions (`backend == HardwareTri`, `tlas_nonempty`,
            // `has_primary_directional`) are threaded via `tlas_enabled` + `csm_armed` inside `scene()`.
            // Default `mode == None` ⇒ `denoise_armed == false` ⇒ `scene.shadow == None` ⇒ byte-identical.
            #[cfg(feature = "hwrt")]
            let (denoise_armed, atrous_levels, temporal_enabled) = {
                let cfg = world.resource::<boyko_render::ShadowDenoiseConfig>();
                let spatial = cfg.spatial_enabled();
                let temporal = cfg.temporal_enabled();
                let atrous_levels = if spatial { cfg.clamped_levels() } else { 0 };
                (spatial || temporal, atrous_levels, temporal)
            };
            // Render terminator-softening: `true` iff the world carries a `LightingConfig`
            // resource with `terminator_softening > 0` — the SAME `world.try_resource` pattern
            // `ddgi_enabled` above uses. Absent (the default host that never sets it) or `0.0`
            // (the plugin-inserted default), `terminator_wrap` is `false` → `scene()` binds the
            // base resolve pipeline → byte-identical (the 0%-gate).
            let terminator_wrap = world
                .try_resource::<LightingConfig>()
                .is_some_and(|cfg| cfg.terminator_softening > 0.0);
            // Anti-aliasing Stage 1/2: read the resolved AA mode (`AaPlugin`'s
            // `resolve_aa_policy` is the single writer). No `#[cfg(hwrt)]` — AA is
            // feature-independent, unlike `denoise_armed` above. The SAME `try_resource`
            // pattern `terminator_wrap`/`ddgi_enabled` use: a host that omits `AaPlugin`
            // degrades to the default `Off` rather than panicking. Default/absent `Off` ⇒
            // `scene.aa == None` ⇒ byte-identical (the 0%-gate).
            let resolved_aa_mode = world
                .try_resource::<ResolvedAa>()
                .map(|r| r.mode)
                .unwrap_or_default();
            // TAA rung T3: read the owner-set RCAS sharpen mode + strength straight off
            // `TaaConfig` (NOT a `Resolved*` derived carrier — `TaaConfig::rcas_sharpness`'s own
            // doc: it is host-read at record time, never folded into the `ResolvedTaa` UBO). The
            // SAME `try_resource` pattern `resolved_aa_mode` uses: a host that omits `AaPlugin`
            // degrades to `TaaConfig::default()`'s `SharpenMode::None` rather than panicking.
            // Default/absent `None` ⇒ `scene.rcas == None` ⇒ byte-identical (the 0%-gate).
            let resolved_taa_config = world.try_resource::<TaaConfig>().copied().unwrap_or_default();
            // Render P7-Q2: read the resolved SSAO selection (`SsaoPlugin`'s
            // `resolve_ssao_policy` is the single writer). The SAME `try_resource` pattern
            // `resolved_aa_mode` uses: a host that omits `SsaoPlugin` degrades to `None`
            // (no variant) rather than panicking. Default/absent `None` ⇒ `scene.ssao ==
            // None` ⇒ byte-identical (the 0%-gate; the resolve's `ssao_mode` header gate is
            // armed separately by `boyko_render::sync_ssao_light_gate`, in lock-step with
            // `SsaoConfig`).
            let ssao_variant = world.try_resource::<ResolvedSsao>().and_then(|r| r.variant);
            // The SSAO edge-avoiding à-trous denoise chain: the resolved, ALREADY-CLAMPED pass
            // count (`ResolvedSsao::atrous_levels` — `0` or `2..=MAX_SSAO_ATROUS_LEVELS`; forced
            // to `0` by `resolve_ssao` whenever `ssao_variant` is `None`, so the two can never
            // disagree). The SAME `try_resource` pattern `ssao_variant` uses: a host that omits
            // `SsaoPlugin` degrades to `0` (no à-trous dispatch) rather than panicking.
            let ssao_atrous_levels = world
                .try_resource::<ResolvedSsao>()
                .map_or(0, |r| r.atrous_levels);
            // VG R3 piece 1 step P1-2: the depth-pyramid arming + its DERIVED shape, in ONE call.
            // The SAME `try_resource` pattern `ssao_variant`/`resolved_aa_mode` use — a host that
            // omits `HzbPlugin` degrades to "no pyramid" rather than panicking — and the ONLY site
            // in the tree that calls `boyko_render::hzb::HzbLayout` on a frame path (plan §4: one
            // implementation of `prev_pow2`/`msb`/`level_extent`, the backend derives nothing).
            //
            // Sized to `present_extent`, the COMPOSITE extent `GBufferTargets::create` allocates
            // every other per-extent target at — the extent whose depth attachment the pyramid
            // reduces, not the window's live client size. Default/absent `HzbMode::Off` ⇒ `None`
            // ⇒ `scene.hzb == None` ⇒ no image, no per-mip views, no build passes (the 0%-gate).
            //
            // VG R3 piece 4 rung P4-4: the CONSUMER knob joins the call, and the plan is `Some`
            // iff a producer asks OR a consumer needs. Read LIVE here beside `HzbConfig`, not
            // frozen: the split has no light-header term to drift out of lock-step with, and a
            // flip that changes whether a pyramid exists moves `hzb_arm` and takes the targets
            // recreate route (`hzb_config.rs`'s repaired "why not RenderPathFrozenConsumers" doc).
            let occ_config = world.try_resource::<boyko_render::OcclusionConfig>().copied();
            // The DIAGNOSTIC verdict override, read the same way and inert without the arming
            // above (`occlusion_arm_for` returns `None`, so the force word never reaches the
            // scene). `None` — a host that inserts no `OcclusionForce` — IS `OcclusionForce::None`,
            // which is every shipping run and every golden.
            let occ_force = world.try_resource::<OcclusionForce>().copied().unwrap_or_default();
            frame_occ_mode = occ_config.map_or(boyko_render::OcclusionMode::Off, |c| c.mode);
            frame_occ_force = occ_force;
            let hzb_plan = crate::hzb_plan::hzb_plan_for(
                world.try_resource::<boyko_render::HzbConfig>().copied(),
                occ_config,
                present_extent.width,
                present_extent.height,
            );
            // VG R3 piece 4 rung P4-4: the OWNER's arming for THIS frame — presence is the arming,
            // the payload is the forced verdict. One call, one site, threaded into the scene below
            // exactly as `hzb_plan` is.
            let vb_occlusion_arm = crate::occlusion_arm::occlusion_arm_for(occ_config, occ_force);
            // VG R3 piece 1 step P1-6: the dump's own request, sited HERE because it needs
            // `hzb_plan` — the staging is sized from the plan's per-level extents, not from the
            // extent alone, so it cannot be requested beside the census's at the top of the frame.
            // `present_extent` (the COMPOSITE) for the same reason the census uses it: that is the
            // extent the depth ring is built at and the extent the recorder's copy region names.
            // `None` on every non-probe frame, and also on a probe run whose pyramid is disarmed —
            // there is nothing to dump then, and the driver stays in `Request` rather than
            // draining a staging nothing was copied into.
            let hzb_dump_staging = match hzb_dump.as_mut() {
                Some(d) => d.request(ctx, present_extent, hzb_plan),
                None => None,
            };
            // VG R3 piece 3 step P3-5: the cull readback's own request, sited beside the pyramid
            // dump's so the two probes ask on the SAME frame. That is what makes the pairing check a
            // statement about ONE frame: both drivers count the same `SETTLE_FRAMES` of presented
            // frames from the same start, so on an armed-together run they enter `Request` together.
            //
            // Unlike the dump's, this request hands out no buffer — the staging is boot-owned and
            // per-FIF. What it hands out is the ARMING: `scene.vb_cull_readback` is `Some` on this
            // frame only, so the copies run once and the drained slot still holds this frame's bytes.
            vb_cull_capture = vb_cull_probe.as_mut().is_some_and(|p| p.request(s, frame_index));
            // SSAA (AA campaign Stage 3, C1) — the HOST-AUTHORITATIVE LOCK: resolution is
            // a boot commitment (`WindowHost::boot`'s device-capability probe), so the
            // per-frame mode MUST agree with it, never the reverse. `host.ssaa_armed` ⇒
            // FORCE `Ssaa` regardless of what the ECS resolved (the post-`finish()`
            // resident insertion above already makes this the truthful common case; this
            // debug_assert catches any later system that stomped `AaConfig` back to
            // something else). `!host.ssaa_armed` ⇒ any `Ssaa` the ECS resolved (e.g. a
            // stale `AaConfig` from a prior boot's serialized state, or a test that
            // requests SSAA on an unarmed host) DEGRADES to `Off` — the 2× render was
            // never committed, so consuming it would sample uninitialized/OOB `.Load`
            // coords in the downsample shader. Off/Fxaa/Smaa pass through unchanged.
            let aa_mode = if host.ssaa_armed {
                debug_assert!(
                    matches!(resolved_aa_mode, AaMode::Ssaa),
                    "invariant: SSAA boot-armed but ResolvedAa={resolved_aa_mode:?}"
                );
                AaMode::Ssaa
            } else if matches!(resolved_aa_mode, AaMode::Ssaa) {
                AaMode::Off
            } else {
                resolved_aa_mode
            };
            // Multi-paradigm render-path plan, rung R8: the live Decision-0 geometry table's
            // Set, read from `World` here (the ONE per-frame `World` read `scene()` itself
            // avoids by taking this as a plain param — this fn's own doc). `None` on every boot
            // that never armed the table (`MeshGeometryTableSlot(None)` — every non-VB boot, or a
            // VB boot whose device lacks the descriptor-indexing prerequisite).
            // Virtual-geometry ladder, rung R2d-2: the SAME slot yields the table's `gMeshBounds[]`
            // buffer. Read through ONE borrow of the resource so the two cannot come from
            // different table generations — `vb_geometry_set` and `vb_mesh_bounds` are `Some`
            // together or `None` together, which is the property `vb_cull_set`'s gate rests on.
            let vb_geometry_table =
                world.non_send_resource::<boyko_render::MeshGeometryTableSlot>().0.as_ref();
            let vb_geometry_set = vb_geometry_table.map(boyko_render::MeshGeometryTable::set);
            let vb_mesh_bounds =
                vb_geometry_table.map(boyko_render::MeshGeometryTable::bounds_buffer);
            // VB-P1e D11: `ClusterConfig` dims are a BOOT commitment — `build_froxel_light_cull`
            // sized every L1 buffer (and, on the HIER arm, the pushed `cluster_dims_packed`) from
            // the dims read at boot; a live edit to the `ClusterConfig` Resource behind this
            // dispatch's back cannot move the cull's OWN writes (D11), but the `ClusterGrid`
            // *consumers* (`vb_resolve`/`vb_shade`/`deferred_pbr`/`forward_opaque`) still index
            // with the LIVE header dims (VB-P1k, pre-existing, not closed here) — so a boot/live
            // skew is a frame-level safety gap this assert cannot fix, only catch. The cheapest
            // tripwire against an owner system stomping the Resource after boot; release builds
            // do not pay for it.
            //
            // Gated on `host.gpu.cluster_cull_armed()`, NOT `resolved_render_path.froxel_light_cull`
            // (P1-2, adversarial review): `froxel_light_cull` is strictly WIDER than the
            // condition that actually WROTE `cluster_boot_packed_dims` — `build_froxel_light_cull`
            // additionally requires a live `MeshGeometryTableSlot` (`runner.rs`'s own boot call
            // site), which `froxel_light_cull` does not. A `VisibilityBuffer` + `GeometryLegs::Sdf`
            // boot (a shipped golden matrix cell) or any boot where `MeshGeometryTable::new`
            // degrades to `None` would otherwise leave the snapshot at its zeroed default while
            // the live `ClusterConfig` Resource is non-zero, panicking this assert every frame.
            if host.gpu.cluster_cull_armed() {
                debug_assert_eq!(
                    world.try_resource::<boyko_render::ClusterConfig>().map(|c| c.packed_dims()),
                    Some(host.gpu.cluster_boot_packed_dims()),
                    "invariant: ClusterConfig dims are a boot commitment (cull buffers are boot-sized)"
                );
            }
            // VG R3 piece 2 steps P2-3/P2-6: read ONCE, here, and used twice below — threaded
            // into the scene (where it is the split's structural conjunct) and carried out to
            // gate G2's artifact. Two calls would be two chances to read a different scratch.
            frame_occlusion_instances = scratch.occlusion_instances();
            // Profiling rung 5c: claim this frame's zone ring slot BEFORE `scene` reads it — a
            // `&mut` call that must end before `scene` hands out shared borrows for the frame. The
            // epoch is read into a local first for the same borrow reason. A no-op unless leg B is
            // armed, so it sits outside any predicate that could drift from `vb_zone_for_frame`'s.
            let vb_zone_epoch = host.renderer.submission_epoch();
            host.gpu.open_vb_zone_frame(frame_index, vb_zone_epoch, u64::from(frame_index));
            let scene = host.gpu.scene(
                mvp,
                s,
                &draws,
                light_upload,
                csm_armed.then_some(resolved_csm),
                punctual_armed.then_some(resolved_atlas),
                interp_count,
                overstep,
                ddgi_enabled,
                frame_index,
                #[cfg(feature = "hwrt")]
                tlas_enabled,
                #[cfg(feature = "hwrt")]
                denoise_armed,
                #[cfg(feature = "hwrt")]
                atrous_levels,
                #[cfg(feature = "hwrt")]
                temporal_enabled,
                material_table,
                // Asset-streaming plan F8: the per-frame PER_INSTANCE_MATERIAL pipeline gate,
                // read from the SAME `scratch` the instance-model upload above just read.
                scratch.any_non_default_material(),
                // Textured-PBR T6c: the per-frame TEXTURED pipeline gate, read from the SAME
                // `scratch` — `gather_mesh_draws` always runs `gather_material_tex_into` right
                // after the affine gather (mesh_draw.rs), so this flag is fresh every frame.
                scratch.any_textured_material(),
                terminator_wrap,
                aa_mode,
                taa_reset_flag,
                resolved_taa_config.sharpen,
                resolved_taa_config.rcas_sharpness,
                ssao_variant,
                ssao_atrous_levels,
                // Multi-paradigm render-path plan, rung R1: the boot-committed selection
                // (Decision 1 — a HOST field, never a per-frame `World` read, mirroring
                // `host.ssaa_armed`'s threading). `scene()` converts it into the plain-POD
                // `ResolvedRenderPathGpu`.
                //
                // No longer dead-but-threaded (this comment claimed that through rung R8): the
                // mirror's `path` / `mesh_leg` / `sdf_leg` / `sdf_forward_marched` /
                // `needs_depth_prepass` / `mesh_geo_shade_split` fields drive the declarator and
                // recorder dispatch in `boyko_rhi_vulkan::present`. `shadow` is read by ONE
                // assertion (`record_vb`'s `vb_shade_split_*hwrt` exclusion check) and by nothing
                // that selects a pass — see `ResolvedRenderPathGpu::shadow`'s own doc for why
                // that is deliberate. The remaining mirror fields have no reader at all,
                // `vb_geometry_table` / `froxel_light_cull` included: those two ARE read, but off
                // THIS value above (the pre-conversion `boyko_render` carrier), never off the
                // mirror — each reaches the RHI by its own boot route.
                host.resolved_render_path,
                // Multi-paradigm render-path plan, rung R-SDFFWD: the `sdf_forward_march`
                // `HAS_MESH` push's reverse-Z decode A/B, computed above at the SAME site `mvp`
                // is built (the SAME `view.near`/`view.far` `forward_gbuffer_push_from_view`
                // used).
                sdf_forward_view_z_a,
                sdf_forward_view_z_b,
                // TAA-under-VB: the `viewt_from_depth_rz` reverse-Z pair, computed above at the
                // SAME `view.near`/`view.far` site (single-source discipline).
                vb_viewt_view_z_a,
                vb_viewt_view_z_b,
                vb_geometry_set,
                vb_mesh_bounds,
                // VG R3 piece 1 step P1-2: this frame's pyramid plan (the single `HzbLayout` call
                // above). `None` on the default `HzbMode::Off` — the 0%-gate.
                hzb_plan,
                // VG R3 piece 1 step P1-6: the dump staging, requested just above from the SAME
                // `hzb_plan` and the SAME `present_extent` this call threads — one site, so the
                // staging's size and the recorder's copy regions cannot disagree.
                hzb_dump_staging,
                // VG R3 piece 2 step P2-3: this frame's occlusion-capable instance count, read
                // from the SAME `scratch` the two material gates above are — the MAIN
                // `MeshRenderScratch`, never `CsmCasterScratch.0`, which runs the identical fold
                // on its own caster-filtered scratch and is therefore redundant, never
                // authoritative. Reading it off anything but this scratch would be a SECOND
                // frame-level predicate that can disagree with the first.
                frame_occlusion_instances,
                // VG R3 piece 4 rung P4-4: this frame's OWNER arming, from the single
                // `occlusion_arm_for` call above — `None` on the default `OcclusionMode::Off`,
                // which is every committed pin but the four that arm the split.
                vb_occlusion_arm,
                // VG R3 piece 3 step P3-5: this frame's cull-readback arming, from the request just
                // above — `false` on every frame but the one the probe captures.
                vb_cull_capture,
                ctx,
            );

            // 6. Asset-streaming plan F7 §6/§8 (invariant W1-b): the fenced slot must
            // be repointed before its descriptor sets are recorded/bound below —
            // guards the FIF-rebind proof's invariant (b), not merely documents it.
            debug_assert!(
                !host.frame.targets_ready() || !material_table.rebind_pending(s),
                "invariant (F7 W1-b): the fenced slot must be repointed before its set is recorded"
            );

            // 7. Render + present, consuming the token (the host-write window
            //    for slot `s` ends here — R0b).
            // SAFETY: `ctx`/`surface`/`swapchain`/`renderer` share the one
            // pinned device; every `scene` resource is live (owned by
            // `host.gpu` / the World's `Assets<MeshGpu>` / `MaterialTable`, all
            // outliving the call); `present_extent` == the composite extent the camera
            // push `count`, `dispatch_group_count_x`, and the G-buffer targets
            // are sized to (all boot-fixed, plan D7); `aa_extent` == `host.native_extent`
            // (boot-fixed, SSAA's `aa_out` size — see `render_gbuffer_frame`'s doc: it
            // MUST stay boot-fixed, not the live window size, for the same resize-
            // invariance `present_extent` already has); a `Some(readback)` is the dump's
            // host-visible staging, sized to the current swapchain extent by
            // `HostDump::request` (`None` on the steady path); a `Some(vb_id_readback)` is the
            // VG-R0 census's own host-visible staging, sized by `VgCensusDump::request` to
            // `present_extent * 8 B` — the COMPOSITE extent the `vb_id` ring is built at, which is
            // the extent the recorder's copy region names (`None` on the steady path).
            let aa_extent = VkExtent2D {
                width: host.native_extent.0,
                height: host.native_extent.1,
            };
            unsafe {
                host.renderer.render_gbuffer_frame(
                    token,
                    ctx,
                    &host.surface,
                    &mut host.swapchain,
                    &scene,
                    &mut host.frame,
                    host.window.width(),
                    host.window.height(),
                    CLEAR_COLOR,
                    present_extent,
                    aa_extent,
                    readback,
                    vb_id_readback,
                    // VG R3 piece 2 step P2-6: gate G2's count sink, `Some` on ONE settled frame
                    // of a `BOYKO_VB_PROBE` run and `None` on every other frame of every other
                    // run. The recorder writes host memory through it and records no command for
                    // it, which is what keeps an armed run's command stream identical to a
                    // steady one's.
                    vb_probe.as_mut().and_then(crate::vb_probe_dump::VbProbeDump::request),
                )
            }
        };
        // VG-R0 R0c: this frame's SUBMITTED triangle count — the `submitted_per_covered_pixel`
        // report-only numerator, read from the draw list BEFORE the scratch is returned. Gated on
        // the census being armed, so the steady path pays one `Option` check and no iteration.
        let submitted_tris: u64 = if census.is_some() {
            draws
                .iter()
                .map(|d| u64::from(d.index_count / 3) * u64::from(d.instance_count))
                .sum()
        } else {
            0
        };
        // VG rung R2c-tail: captured BEFORE `draws` is handed back to the scratch — the cull
        // readback's line reports it beside the GPU's own visible count, and gate G2's probe
        // artifact carries it as the host's independent cross-check.
        let draw_batch_count = draws.len();
        // VG rung R2d-5: the per-batch BASES, likewise captured before the scratch takes `draws`
        // back. The survivor list is REGION-addressed — batch `b` owns
        // `[base_instance(b), base_instance(b) + instanceCount(b))` and writes nowhere else — and
        // the base lives only on the host (the GPU sees it inside a descriptor the probe does not
        // copy). Without it the readback could only print a FLAT prefix, which interleaves real
        // survivors with slots no batch owns: this loop's own `try_get` skip above makes
        // `scene.mesh_draw` a SUBSEQUENCE of the gather, so the bases can leave gaps.
        //
        // VG R3 piece 3 step P3-5: LATCHED INTO THE DRIVER rather than kept in a loop local. The
        // payload is decoded `DRAIN_FRAMES` presented frames after this one, and by then `draws` has
        // been returned to the scratch and re-gathered three times. A local would therefore describe
        // the DRAIN frame's batches while the bytes describe the CAPTURE frame's — identical on a
        // static fixture and wrong on anything that moves, which is the class of agreement this
        // campaign has shipped as a gate before.
        //
        // Gated on the capture frame, so no other frame of any run allocates here.
        if vb_cull_capture
            && let Some(p) = vb_cull_probe.as_mut()
        {
            p.latch_batches(draw_batch_count, draws.iter().map(|d| d.base_instance).collect());
        }
        host.draw_scratch.put(draws);

        let presented_ok = matches!(&presented, Ok(true));
        match presented {
            // Presented normally, or the swapchain was (re)created this call
            // (frame skipped; the size refresh below feeds the next attempt) —
            // plan D7: a resize recreates the swapchain, the blit clamps.
            Ok(_) => {}
            // Terminal: the renderer must not be reused after a post-acquire
            // failure (`frame_driver` contract) — exit and tear down.
            Err(e) => {
                eprintln!("boyko_app: terminal render error - exiting ({e:?})");
                return;
            }
        }

        // VB-P1e H0: accumulate this frame's per-pass bench samples — read ONLY on a frame that
        // actually presented (a resize-skip records no new `record_vb` work this iteration).
        // GATED on `vb_bench`; dead code on every non-bench run. VG R3 piece 4 rung P4-1: one row
        // per VB zone id, plus the frame's structural label from the recorder's witness.
        // Profiling rung 5c: the command census, printed by BOTH A/B legs from the SAME place and
        // in the SAME format. It is what G10's witness clause compares — "same number of bracket
        // timestamps, each at the same position in the recorded stream" — and it has no vocabulary,
        // so no `pass -> zone` table exists to be written wrong.
        //
        // The counters are zero unless `boyko_rhi_vulkan/profiling-census` is on: without it the
        // ~200 increments at `vb.rs`'s record sites compile to nothing. The line prints anyway, and
        // says `stream_pos=0`, because a gate that silently saw no line could not tell a census
        // build from a run that never reached a frame.
        if vb_zone
            && presented_ok
            && let Some(w) = host.gpu.vb_census()
        {
            use core::fmt::Write as _;
            let mut positions = String::with_capacity(160);
            for (k, p) in w.stamp_positions().enumerate() {
                let sep = if k > 0 { "," } else { "" };
                let _ = write!(positions, "{sep}{p}");
            }
            // Profiling rung 7c: WHAT was stamped, beside WHERE. A position is blind to the
            // pipeline stage, and that blindness is what let the zone recorder open seven of the
            // ten VB brackets at `TOP_OF_PIPE` where the collector opened them at `BOTTOM_OF_PIPE`
            // — same commands, same positions, a different quantity — with G10 green throughout.
            // `T`/`B` rather than the enum's `Debug`, so the list stays one short token per stamp.
            let mut stages = String::with_capacity(80);
            for (k, s) in w.stamp_stages().enumerate() {
                let sep = if k > 0 { "," } else { "" };
                let c = match s {
                    boyko_rhi::TimestampStage::TopOfPipe => 'T',
                    boyko_rhi::TimestampStage::BottomOfPipe => 'B',
                };
                let _ = write!(stages, "{sep}{c}");
            }
            println!(
                "VB-CENSUS leg={} frame={frame_index} stream_pos={} profiling_cmds={} \
                 resets={} stamps={} repairs={} pairs={} positions=[{positions}] stages=[{stages}]",
                // Rung 7 deleted the collector leg, so the only two legs left are the
                // zone recorder and the R0 gbuffer collector.
                if vb_zone { "zone" } else { "gbuf" },
                w.stream_pos(),
                w.profiling_cmds(),
                w.query_resets(),
                w.timestamps(),
                w.repairs(),
                w.recorded_pairs(),
            );
        }

        // Profiling rung 5c: poll the zone ring and report every retired frame. The recorder is
        // NON-BLOCKING by construction (G2a's const-assert), so a frame whose results are not back
        // yet is simply not retired this iteration — there is no wait here and no `wait_idle`, the
        // difference from the bench leg directly below.
        if vb_zone && presented_ok {
            vb_zone_force_seen |= 1u8 << frame_occ_force.slot();
            vb_zone_mode_seen |= 1u8 << (frame_occ_mode as u32);
            let epoch = host.renderer.submission_epoch();
            let now = u64::from(frame_index);
            let (mut measured, mut lost, mut torn, mut unbracketed) = (0u64, 0u64, 0u64, 0u64);
            let mut retired_frames = 0u32;
            host.gpu.retire_vb_zone(ctx, epoch, now, &mut vb_zone_scratch, |frame, pairs| {
                retired_frames += 1;
                // Profiling rung 6: the ZONE IDS, printed. They are `family base + pass slot`, and
                // printing them is what lets a gate assert that a Deferred frame's brackets landed
                // in the gbuffer/SV0 ranges rather than colliding with VB's at slot 0 — the whole
                // reason the bases exist. A count alone cannot say that.
                {
                    use core::fmt::Write as _;
                    let mut zones = String::with_capacity(64);
                    for (k, p) in pairs.iter().enumerate() {
                        let sep = if k > 0 { "," } else { "" };
                        let _ = write!(zones, "{sep}{}", p.zone);
                    }
                    println!("VB-ZONE zones frame={} ids=[{zones}]", frame.frame);
                }
                if let Some(r) = vb_zone_reducer.as_mut() {
                    r.observe_frame(pairs);
                }
                for p in pairs {
                    match p.label {
                        boyko_rhi_vulkan::present::gpu_zone::GpuLabel::Measured => measured += 1,
                        boyko_rhi_vulkan::present::gpu_zone::GpuLabel::Lost => lost += 1,
                        boyko_rhi_vulkan::present::gpu_zone::GpuLabel::Torn => torn += 1,
                        boyko_rhi_vulkan::present::gpu_zone::GpuLabel::NotBracketed => {
                            unbracketed += 1;
                        }
                    }
                }
                println!(
                    "VB-ZONE retired frame={} pairs={} cause={:?} lost={} torn={}",
                    frame.frame, frame.pairs, frame.cause, frame.lost, frame.torn
                );
            });
            vb_zone_pairs_measured += measured;
            vb_zone_pairs_lost += lost;
            vb_zone_pairs_torn += torn;
            vb_zone_pairs_unbracketed += unbracketed;
            vb_zone_seen += retired_frames;
            if vb_zone_seen >= VB_BENCH_WARMUP as u32 + vb_zone_frames {
                // Teardown's own clause: frames stop here, so neither deadline horn can fire and
                // the last `GPU_RING_DEPTH` slots would otherwise be dropped silently.
                host.gpu.flush_vb_zone(ctx, &mut vb_zone_scratch, |frame, _pairs| {
                    println!(
                        "VB-ZONE flushed frame={} pairs={} cause={:?}",
                        frame.frame, frame.pairs, frame.cause
                    );
                });
                println!(
                    "VB-ZONE summary frames={vb_zone_seen} measured={vb_zone_pairs_measured} \
                     lost={vb_zone_pairs_lost} torn={vb_zone_pairs_torn} \
                     not_bracketed={vb_zone_pairs_unbracketed}"
                );
                // Profiling rung 7: the measurement artifact, written ONCE at the end of the window
                // and OFF-FRAME — the last thing this run does before it exits. The reducer has no
                // console form, which is what lets rung 7 delete the printed channel, so this write
                // is the only way its numbers leave the process.
                if let (Some(r), Some(path)) = (vb_zone_reducer.take(), vb_zone_artifact_path.as_ref())
                {
                    let frames = r.frames();
                    let (zones, census) = r.finish();
                    let session = boyko_diag::clock::session_id();
                    let art = crate::profiling::artifact::Artifact {
                        header: crate::profiling::artifact::ArtifactHeader {
                            schema_version: crate::profiling::artifact::ARTIFACT_SCHEMA_VERSION,
                            session_lo: session.0,
                            session_hi: session.1,
                            run_token: vb_zone_run_token.clone(),
                            // Rung 7c: the tag is TWO halves split by who can know it. This one is
                            // DERIVED from the WHOLE boot-resolved path — `path × legs` alone let
                            // the froxel-cull leg share a tag with the flat leg, which are the two
                            // sides of the floor experiment (`artifact.rs`'s Decision 5).
                            workload_tag: crate::profiling::artifact::config_tag(
                                &host.resolved_render_path,
                            ),
                            // And this one is DECLARED, because nothing here can derive a light
                            // count or a scene. Empty when nobody said, which `floor_source`
                            // refuses — the strict option, owner's call.
                            content_tag: std::env::var("BOYKO_PROFILE_WORKLOAD")
                                .unwrap_or_default(),
                            instrument: if ctx.device_caps().timestamps_usable() {
                                crate::profiling::artifact::Instrument::Live
                            } else {
                                crate::profiling::artifact::Instrument::NoTimestamps
                            },
                            precision_decimals: crate::profiling::artifact::PRECISION_DECIMALS,
                            // The SET, not the value at exit — rendered by the same helper the
                            // printed provenance line uses, so the two spellings cannot drift.
                            regimes: word_set(&OcclusionForce::ALL, vb_zone_force_seen, |f| {
                                f.as_str()
                            }),
                            modes: word_set(
                                &boyko_render::OcclusionMode::ALL,
                                vb_zone_mode_seen,
                                |m| m.as_str(),
                            ),
                            regime_n_distinct: vb_zone_force_seen.count_ones(),
                        },
                        zones,
                        census,
                    };
                    match art.write(path) {
                        Ok(()) => println!(
                            "VB-ZONE artifact path={} frames={frames} zones={} measured={}",
                            path.display(),
                            art.zones.len(),
                            art.census.measured
                        ),
                        // LOUD, and not a panic: the measurement is over by the time this runs, and
                        // a run that rendered correctly should not be reported as a crash because a
                        // path was unwritable. The gate that reads the file reds on its absence.
                        Err(e) => {
                            eprintln!("VB-ZONE artifact: could not write {}: {e}", path.display());
                        }
                    }
                }
                return;
            }
        }

        // Diagnostic dump (cold): advance settle → request → drain; once the
        // drained readback is host-readable, print the frame-stream state the
        // GPU actually consumed beside the image, write it, and exit the loop.
        let dump_ready = match dump.as_mut() {
            Some(d) => d.after_present(presented_ok),
            None => false,
        };
        if dump_ready {
            dump_diagnostics(app);
            dump.take()
                .expect("invariant: the dump just reported ready")
                .finish(ctx);
        }

        // VG-R0 rung R0c (cold): the same settle → request → drain progression for the density
        // census. The drain is what makes the host read of the per-FIF `vb_id` ring safe — the
        // readback frame's slot fence is re-waited before the staging is mapped.
        let census_ready = match census.as_mut() {
            Some(c) => c.after_present(presented_ok),
            None => false,
        };
        if census_ready {
            let cx = crate::vg_census_dump::CensusContext {
                submitted_tris,
                ssaa_armed: host.ssaa_armed,
                ssaa_scale: host.ssaa_scale,
                native_extent: host.native_extent,
                // Recorded, not assumed: a boot that resolved away from VB, or away from the mesh
                // leg, records NO copy, and the row must say so rather than let a sentinel-only
                // readback look like an empty scene.
                vb_mesh_leg: matches!(
                    host.resolved_render_path.path,
                    boyko_render::RenderPath::VisibilityBuffer
                ) && host.resolved_render_path.mesh_leg,
            };
            census
                .take()
                .expect("invariant: the census just reported ready")
                .finish(ctx, &cx);
        }

        // VG R3 piece 1 step P1-6 (cold): the same settle → request → drain progression for the
        // pyramid dump. The drain is what makes the host read safe — `vb_depth` is a per-FIF RING
        // (the pyramid is not), so the dump frame's slot fence must be re-waited before the
        // staging is mapped.
        let hzb_dump_ready = match hzb_dump.as_mut() {
            Some(d) => d.after_present(presented_ok),
            None => false,
        };
        if hzb_dump_ready {
            hzb_dump
                .take()
                .expect("invariant: the HZB dump just reported ready")
                .finish(ctx);
        }

        // VG R3 piece 2 step P2-6 (cold): gate G2's recording probe. Settle → probe, with NO
        // drain — it copies no device resource, so its counts are complete the moment the record
        // body returned (`vb_probe_dump`'s own doc states why the sibling drains do not apply).
        let vb_probe_ready = match vb_probe.as_mut() {
            Some(p) => p.after_present(presented_ok),
            None => false,
        };
        if vb_probe_ready {
            let cx = crate::vb_probe_dump::VbProbeContext {
                draw_batches: draw_batch_count as u32,
                occlusion_instances: frame_occlusion_instances,
                // Recorded, not assumed: a device that fails the VB capability probe degrades to
                // `Deferred` and `record_vb` never runs, so a zero `scopes` would be an
                // INSTRUMENT failure. The gate reads these two before it reads any count.
                vb_path: matches!(
                    host.resolved_render_path.path,
                    boyko_render::RenderPath::VisibilityBuffer
                ),
                mesh_leg: host.resolved_render_path.mesh_leg,
                // VG R3 piece 4 rung P4-4: the HOST's own view of this frame's regime, written
                // into `[host]` beside the RECORDER's stamped `[probe] occ_flags`. The two are
                // derived at different sites, so a gate comparing them compares two derivations —
                // `vb_probe_dump`'s stated design principle, applied to provenance instead of
                // counts. It is what makes a live `OcclusionForce` legible from the artifact
                // instead of asserted to have held still.
                occ_mode: frame_occ_mode,
                occ_force: frame_occ_force,
            };
            vb_probe
                .take()
                .expect("invariant: the VB record probe just reported ready")
                .finish(&cx);
        }

        // VG rung R2c-tail / R2d-5, converted at VG R3 piece 3 step P3-5 (cold): the same settle →
        // request → drain progression for the cull readback.
        //
        // This is the gate the goldens cannot be: every pinned scene is entirely on-screen, so a
        // cull that rejects nothing renders the same image as a correct one. The visible COUNT is
        // the observable that separates them, and it is the first consumer the compaction buffers
        // have had since rung R2c0 built them.
        //
        // The DRAIN is what replaced the `wait_idle` this block used until P3-5: the copies ran on
        // the capture frame alone, and `DRAIN_FRAMES (3) > FRAMES_IN_FLIGHT (2)` presented frames
        // later the loop has necessarily re-waited that frame's slot fence. That is the same
        // argument the pyramid dump's drain rests on, and it is what lets the two captures come from
        // ONE frame instead of one of them ending the process before the other ran.
        let vb_cull_ready = match vb_cull_probe.as_mut() {
            Some(p) => p.after_present(presented_ok),
            None => false,
        };
        if vb_cull_ready {
            let probe = vb_cull_probe
                .take()
                .expect("invariant: the cull readback just reported ready");
            let (capture_slot, capture_frame) = probe.capture();
            // `read_vb_cull` is `None` only when the staging is absent, and the boot-time
            // `debug_assert_eq!` above states that a run with this driver has one. `expect` rather
            // than an `if let`: silently skipping here would leave the driver taken and the run
            // exiting with no file, which reads exactly like a scene that produced nothing.
            let rb = host
                .gpu
                .read_vb_cull(capture_slot, capture_frame)
                .expect("invariant: the cull-probe driver and its staging are armed by one variable");
            let line = crate::vb_cull_probe::format_vb_cull_probe_line(
                &crate::vb_cull_probe::VbCullProbeFields {
                    drawn_batches: probe.batch_count(),
                    bases: probe.bases(),
                    visible_batches: rb.visible_batches,
                    batch_list: &rb.batch_list,
                    record_instance_counts: &rb.record_instance_counts,
                    visible_instances: &rb.visible_instances,
                    late_candidates: &rb.late_candidates,
                    late_count_pre: &rb.late_count_pre,
                    late_survivors: &rb.late_survivors,
                    late_count_post: &rb.late_count_post,
                    late_record_instance_counts: &rb.late_record_instance_counts,
                    frame_index: rb.frame_index,
                    gpu_observed_frame_index: rb.gpu_observed_frame_index,
                },
            );
            probe.finish(&line);
        }

        // Exit once EVERY armed capture has completed. Each driver `take()`s itself on completion,
        // so `is_none()` reads "not armed, or already finished".
        //
        // ⚠️ The conjunction is load-bearing, not tidiness. All five drivers settle for the same
        // 30 frames (four of them then drain for 3 more), so with several armed they report ready
        // on the SAME frame — and a `return` inside the first branch would exit before the others
        // ever ran. The later capture would silently produce no file, which is indistinguishable
        // from one that ran and found nothing: a skip that does not name itself.
        //
        // VG R3 piece 3 step P3-5 folded the FIFTH driver in, and that was a defect being fixed
        // rather than a symmetry being tidied: the cull readback used to `return` from its own
        // branch on the FIRST presented frame, so `BOYKO_VB_CULL_READBACK` beside `BOYKO_HZB_DUMP`
        // exited at frame 1 with the cull file written and the pyramid file never. The pairing check
        // that compares the two captures' frame index could not be run at all until this line.
        if (dump_ready || census_ready || hzb_dump_ready || vb_probe_ready || vb_cull_ready)
            && dump.is_none()
            && census.is_none()
            && hzb_dump.is_none()
            && vb_probe.is_none()
            && vb_cull_probe.is_none()
        {
            return;
        }

        // 8. Re-observe the client size and publish it: Main readers observe
        //    the PREVIOUS frame's size (the `WindowInfo` one-frame-stale
        //    contract, documented on the type). The R4 host probe shares the
        //    publish step (three integer stores — zero alloc).
        host.window.refresh_size();
        *app.world_mut().resource_mut::<WindowInfo>() = WindowInfo {
            width: host.window.width(),
            height: host.window.height(),
        };
        {
            let stats = app.world_mut().resource_mut::<HostFrameStats>();
            stats.frames += 1;
            stats.light_uploads += u64::from(frame_light_uploaded);
            stats.csm_armed_frames += u64::from(frame_csm_armed);
            stats.punctual_armed_frames += u64::from(frame_punctual_armed);
            stats.interp_armed_frames += u64::from(frame_interp_armed);
        }
        // SDFDDGI I2 (arm): advance the round-robin frame index (wraps benignly at u32::MAX).
        frame_index = frame_index.wrapping_add(1);
    }
}


/// Renders a set-of-variants bitmask as a comma-separated word list, in the variants' own order.
///
/// `seen` bit `k` selects `variants[k]`, which is the accumulation site's contract
/// (`OcclusionForce::slot()` is a pinned bijection onto that range;
/// [`boyko_render::OcclusionMode`] is `#[repr(u32)]` with `ALL` in discriminant order, pinned by
/// its own test). An EMPTY set prints `-` rather than an empty bracket pair, so a summary from a
/// run whose warm-up consumed every frame is legible as "nothing observed" instead of as a
/// truncated line.
///
/// `#[cold]`: called twice, once per process, from the summary print.
#[cfg(windows)]
#[cold]
#[inline(never)]
fn word_set<T: Copy>(variants: &[T], seen: u8, word: impl Fn(T) -> &'static str) -> String {
    let mut out = String::new();
    for (k, v) in variants.iter().enumerate() {
        if seen & (1u8 << k) != 0 {
            if !out.is_empty() {
                out.push(',');
            }
            out.push_str(word(*v));
        }
    }
    if out.is_empty() { "-".to_string() } else { out }
}




/// One-shot frame-stream diagnostics printed beside the dump image — the
/// values the GPU consumed on the captured frame stream: the live CSM
/// selection, the staged light-table header + rows, and the host stats.
#[cfg(windows)]
#[cold]
#[inline(never)]
fn dump_diagnostics(app: &App) {
    let world = app.world();

    let csm = world.resource::<ResolvedCsm>();
    eprintln!(
        "boyko_app: dump csm_mode={} active={}",
        csm.csm_mode_word, csm.active_count
    );
    for (c, data) in csm
        .cascades
        .iter()
        .enumerate()
        .take(csm.active_count as usize)
    {
        let m = &data.view_proj;
        eprintln!(
            "boyko_app: dump cascade[{c}] split_far={:.3} texel={:.4} col3=[{:.3}, {:.3}, {:.3}, {:.3}]",
            data.split_far, data.texel_size, m[3][0], m[3][1], m[3][2], m[3][3]
        );
    }

    // The punctual shadow-atlas selection consumed on the captured frame stream (mirrors the
    // cascade dump): the mode word, active layer count, the per-slot spot/point tag, and each
    // active face's `light_pos`/`inv_range` (POINT lanes; benign for SPOT).
    let atlas = world.resource::<ResolvedShadowAtlas>();
    eprintln!(
        "boyko_app: dump atlas mode={} active_layers={} face_point_mask={:#06x}",
        atlas.mode_word, atlas.active_layers, atlas.face_point_mask
    );
    for (s, face) in atlas
        .faces
        .iter()
        .enumerate()
        .take(atlas.active_layers as usize)
    {
        let is_point = (atlas.face_point_mask >> s) & 1 != 0;
        let kind = if is_point { "point" } else { "spot " };
        eprintln!(
            "boyko_app: dump atlas[{s}] kind={kind} light_pos=[{:.2}, {:.2}, {:.2}] inv_range={:.4}",
            face.light_pos[0], face.light_pos[1], face.light_pos[2], face.inv_range
        );
    }

    let staged = world.resource::<LightTableStaging>();
    let bytes = staged.bytes();
    let word = |i: usize| -> u32 {
        u32::from_le_bytes(
            bytes[i * 4..i * 4 + 4]
                .try_into()
                .expect("invariant: the staged table holds whole words"),
        )
    };
    let lane = |i: usize| f32::from_bits(word(i));
    // Header: counts_exposure (words 0..4), the word-7 gate lane, and the sky
    // lanes (words 4..12).
    eprintln!(
        "boyko_app: dump header counts_exposure=[{:.3}, {:.3}, {:.3}, {:.3}] word7={:#010x}",
        lane(0), lane(1), lane(2), lane(3), word(7)
    );
    // 16 header words, then 12-word GpuLight rows: dir_kind | pos_range | color_cone.
    let rows = (bytes.len().saturating_sub(64)) / 48;
    for r in 0..rows {
        let base = 16 + r * 12;
        eprintln!(
            "boyko_app: dump light[{r}] dir_kind=[{:.3}, {:.3}, {:.3}, {:.3}] pos_range=[{:.2}, {:.2}, {:.2}, {:.2}] color_cone=[{:.2}, {:.2}, {:.2}, {:.2}]",
            lane(base), lane(base + 1), lane(base + 2), lane(base + 3),
            lane(base + 4), lane(base + 5), lane(base + 6), lane(base + 7),
            lane(base + 8), lane(base + 9), lane(base + 10), lane(base + 11)
        );
    }

    let stats = world.resource::<HostFrameStats>();
    eprintln!(
        "boyko_app: dump stats frames={} light_uploads={} csm_armed={} punctual_armed={} interp_armed={}",
        stats.frames,
        stats.light_uploads,
        stats.csm_armed_frames,
        stats.punctual_armed_frames,
        stats.interp_armed_frames
    );
}

/// The D2 teardown's steps 1+2 — unbundle the host and drop the renderer FIRST: its
/// `Drop` performs the `vkDeviceWaitIdle` (frame_driver.rs), so everything after runs
/// under an idle device. Then destroy the extent-dependent targets and the static
/// scene bundles EXPLICITLY (no `Drop` glue on RHI resources), and only then drop
/// swapchain → surface → window (the surface before the window it borrows).
///
/// Touches NO World resident — factored out of [`teardown`] (textured-PBR T6c, fix
/// post-review) so a pre-`finish()` boot failure (`BindlessTextureTable::new`'s `Err`
/// branch in `run_windowed`) can reuse this EXACT host-destroy sequence without
/// pulling in `teardown`'s step 3 (which touches World residents / plugin resources
/// that do not exist yet before `finish()` runs).
///
/// # Safety
/// `ctx` is the live context every resource in `host` was created on; no submission
/// is in flight past this call (the renderer's `Drop`, first, waits the device idle).
#[cfg(windows)]
unsafe fn destroy_host_gpu_chain(host: WindowHost, ctx: &VulkanContext) {
    let WindowHost {
        renderer,
        frame,
        gpu,
        draw_scratch: _,
        retire_scratch: _,
        composite_extent: _,
        ssaa_armed: _,
        ssaa_scale: _,
        native_extent: _,
        resolved_render_path: _,
        light_uploaded_gen: _,
        swapchain,
        surface,
        window,
    } = host;
    drop(renderer);
    // SAFETY: the renderer drop above waited the device idle, so no submission
    // references the targets or the scene bundles; per this fn's own contract `ctx`
    // is the live context they were created on; each is destroyed exactly once
    // (by-value moves).
    unsafe {
        frame.destroy(ctx);
        gpu.destroy(ctx);
    }
    drop(swapchain);
    drop(surface);
    drop(window);
}

/// The D2 teardown steps 1–3 — a named, ordered sequence; every step is
/// load-bearing. EVICTS everything; it does NOT end the singleton.
///
/// # Contract: the CALLER destroys the singleton AFTER this returns
///
/// `ctx` is a reference PARAMETER — protected for the whole call (the same
/// Stacked/Tree-Borrows class the paramless `destroy_singleton` signature
/// exists for): calling `VulkanContext::destroy_singleton()` inside this fn
/// would deallocate a protected referent, UB regardless of any use-after. The
/// runner therefore calls `destroy_singleton` as its OWN last statement, after
/// this fn (and its protector) has returned. Postcondition on return: the
/// device is idle, every host RHI resource is destroyed, and no `&'static
/// VulkanContext` remains in any live structure — exactly the destroy
/// precondition. `ctx` here is only the destroy route for the host's explicit
/// (non-`Drop`) RHI resources.
#[cfg(windows)]
fn teardown(app: &mut App, host: WindowHost, ctx: &VulkanContext) {
    // Steps 1+2 — see `destroy_host_gpu_chain`'s doc.
    // SAFETY: `host` was booted on `ctx` and no submission is in flight past this
    // call (the frame loop's own per-frame fence wait / the renderer's `Drop`
    // inside `destroy_host_gpu_chain` guarantees this) — the same contract every
    // prior caller of this inlined sequence relied on.
    unsafe { destroy_host_gpu_chain(host, ctx) };

    // Step 3 — EVICT every device-referencing World resident (the runner
    // borrows the App and cannot drop it — explicit eviction is the
    // replacement; plan critic delta A1):
    //   - `RhiContext` (shared mode): its `Drop` runs `destroy_all` (frees
    //     every column/UI resource; the registry teardown wait-idles the
    //     already-idle device — a benign no-op) and NEVER touches the device
    //     lifecycle;
    //   - `Assets<MeshGpu>` / `MaterialTable`: their buffers are destroyed through
    //     `ctx` under the step-1 idle (`unsafe destroy` — neither has `Drop` glue);
    //   - `GpuDevice`: the last world-resident `&'static` handle — no dangling
    //     `&'static` may remain in a live structure past this point.
    drop(app.world_mut().remove_non_send_resource::<RhiContext>());
    // Asset-streaming plan F6: force-drain every residual `DeferredFree` entry +
    // `OrphanedMeshGpu` orphan BEFORE the whole-table teardown below. The device
    // is idle (step 1), which trivially satisfies (and exceeds) the per-resource
    // fence precondition every other F6 destroy relies on, so `epoch = u64::MAX`
    // forces every entry past its `retire_frame` gate regardless of how recently
    // it was enqueued. This is REQUIRED, not cosmetic: `Assets::iter` (which
    // `MeshAssetsExt::destroy` below collects its handles from) skips a
    // `Retiring` row (F2: it is occupied-but-unreusable, not `Loaded`), so an
    // un-drained `Retiring` mesh's device buffers would otherwise leak at
    // shutdown.
    retire_deferred_frees(app.world_mut(), ctx, u64::MAX, &mut Vec::new());
    if let Some(mut mesh_assets) = app.world_mut().remove_non_send_resource::<Assets<MeshGpu>>() {
        // SAFETY: the device is idle (step 1) so no in-flight submit references
        // any mesh buffer; `ctx` is the context they were created on; the
        // table is destroyed exactly once (just removed from the World).
        unsafe { mesh_assets.destroy(ctx) };
    }
    // Asset-system rung A1: `MaterialTable`'s table + staging ring are destroyed the
    // SAME way, under the SAME step-1 idle contract. `Assets<Material>` (the CPU
    // authority) holds no GPU handle, so it needs no explicit eviction here — it
    // drops normally along with the rest of the World.
    if let Some(mut material_table) = app.world_mut().remove_non_send_resource::<MaterialTable>() {
        // SAFETY: the device is idle (step 1) so no in-flight submit references
        // the material table or its staging ring; `ctx` is the context they were
        // created on; the table is destroyed exactly once (just removed from
        // the World).
        unsafe { material_table.destroy(ctx) };
    }
    // Asset-streaming plan F7 O1: force-drain every residual `RetiredGpuBuffers`
    // entry under the SAME step-1 device-idle contract as the F6 queues above. The
    // `retire_deferred_frees(..., u64::MAX, ..)` call above already emptied this
    // queue (every `retire_frame <= u64::MAX`) — this drain is UNCONDITIONAL and
    // does not depend on that coincidence, mirroring the explicit teardown drain
    // every other F6/F7 GPU-teardown queue gets.
    if let Some(mut retired) = app.world_mut().remove_non_send_resource::<RetiredGpuBuffers>() {
        // SAFETY: the device is idle (step 1) so no in-flight submission references
        // any queued buffer; `ctx` is the context every entry was created on.
        unsafe { retired.drain_all(ctx) };
    }
    // Textured-PBR rung T6b (openQ2): `Assets<TextureGpu>::destroy` — frees each
    // registered `VulkanTexture` and returns its bindless slot — MUST run BEFORE
    // `BindlessTextureTable::destroy` (which frees the descriptor set + the
    // magenta error texture): a slot returned mid-destroy still needs a LIVE
    // table to return it TO. Both under the SAME step-1 device-idle contract as
    // every other teardown call above. Textured-PBR rung T6c (fix, post-review):
    // `BindlessTextureTable::new`'s fallible creation now runs BEFORE `finish()`
    // (`run_windowed`), with its OWN minimal unwind on `Err` (`destroy_host_gpu_chain`
    // + `destroy_singleton`, no World touch) — that failure path never reaches this
    // fn anymore, so by the time `teardown` runs, both tables are ALWAYS present.
    //
    // Grooming fix (item C): the removal order is `bindless_table` OUTER,
    // `texture_assets` INNER — never a `remove::<A>() && let remove::<B>()` let-chain.
    // A let-chain TAKES the first resource unconditionally, and if the second `remove`
    // is `None` the chain's body never runs — the first value then silently drops at
    // the end of the statement with NO `destroy` call (a real leak: `TextureGpu` owns
    // device images/views that only `ctx.destroy_texture` frees, never a bare Rust
    // `Drop`). Checking `bindless_table` FIRST means a missing table leaves
    // `texture_assets` untouched in the World (never taken, so nothing to leak here);
    // a present table but missing assets still gets destroyed via the `else` arm.
    // Every combination this fn can observe either destroys exactly what it removed,
    // or removes nothing.
    if let Some(mut bindless_table) = app.world_mut().remove_non_send_resource::<BindlessTextureTable>() {
        if let Some(mut texture_assets) =
            app.world_mut().remove_non_send_resource::<Assets<TextureGpu>>()
        {
            // SAFETY: the device is idle (step 1) so no in-flight submit
            // references any texture image or the bindless descriptor set;
            // `ctx` is the context both were created on; each table is
            // destroyed exactly once (just removed from the World).
            unsafe { texture_assets.destroy(ctx, &mut bindless_table) };
            bindless_table.destroy(ctx);
        } else {
            // No `Assets<TextureGpu>` was taken (defensive path — see above), so
            // there is nothing to return a bindless slot for; the table itself was
            // already removed from the World and must still be destroyed here.
            bindless_table.destroy(ctx);
        }
    }
    // `GpuDevice` is a plain reference newtype (no `Drop` glue) — removal alone
    // ends its residency; the returned `Option` is discarded.
    let _ = app.world_mut().remove_non_send_resource::<GpuDevice>();

    // Step 4 (`destroy_singleton`) is the CALLER's — see the fn contract: it
    // must run AFTER this fn returns, once `ctx`'s protector is gone.
}

#[cfg(test)]
mod tests {
    //! R6 runner-bridge unit tests (host plan R6, Decision 1 / Test 5 + Test 6).
    //!
    //! The OS→ECS bridge helper `ingest_captured` is pure (translate +
    //! `push_raw`), so it is exercised here over SYNTHETIC `CapturedMsg`s — no
    //! window, no device — asserting the drained messages land in the queue via
    //! `translate_win32*`, and that a warm ingest allocates nothing.

    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    use boyko_input::win32::{WM_KEYDOWN, WM_KEYUP};
    use boyko_input::{ButtonState, KeyCode, RawInputEvent};

    use super::*;

    /// Builds a `WM_KEY*` `lParam`: OEM scancode in bits 16..=23 (mirrors the
    /// `boyko_input` I6 translate gate's own helper).
    fn key_lparam(scancode: u8) -> isize {
        (scancode as isize) << 16
    }

    /// Escape's OEM scancode (`0x01`) and W's (`0x11`) — from the canonical table.
    const SC_ESCAPE: u8 = 0x01;
    const SC_W: u8 = 0x11;

    /// Test 5: a `Raw` KeyDown, a `Raw` KeyUp, and a `RawMouse` delta all land in
    /// the queue via `translate_win32` / `translate_win32_raw_mouse`, in order.
    #[test]
    fn ingest_captured_bridges_raw_and_raw_mouse() {
        let mut queue = RawInputQueue::with_capacity(16);
        queue.begin_frame();

        ingest_captured(&mut queue, CapturedMsg::Raw { msg: WM_KEYDOWN, wparam: 0, lparam: key_lparam(SC_W) });
        ingest_captured(&mut queue, CapturedMsg::RawMouse { dx: 7, dy: -3 });
        ingest_captured(&mut queue, CapturedMsg::Raw { msg: WM_KEYUP, wparam: 0, lparam: key_lparam(SC_ESCAPE) });

        assert_eq!(queue.len(), 3, "three translated events landed in FIFO order");
        assert_eq!(
            queue.pop(),
            Some(RawInputEvent::Key { code: KeyCode::KeyW, state: ButtonState::Pressed, repeat: false }),
            "first: W key-down"
        );
        assert_eq!(
            queue.pop(),
            Some(RawInputEvent::MouseMotion { dx: 7.0, dy: -3.0 }),
            "second: the raw-mouse delta (i32 → f64)"
        );
        assert_eq!(
            queue.pop(),
            Some(RawInputEvent::Key { code: KeyCode::Escape, state: ButtonState::Released, repeat: false }),
            "third: Escape key-up"
        );
    }

    /// An unmapped `Raw` message translates to `None` and is silently dropped (no
    /// push, no panic).
    #[test]
    fn ingest_captured_drops_unmapped_messages() {
        let mut queue = RawInputQueue::with_capacity(16);
        // `WM_NULL` (0x0000) is not a mapped input message.
        ingest_captured(&mut queue, CapturedMsg::Raw { msg: 0x0000, wparam: 0, lparam: 0 });
        assert_eq!(queue.len(), 0, "an unmapped message pushes nothing");
    }

    // --- Test 6 (bridge half): a warm ingest allocates nothing. ---
    //
    // The counting allocator is process-global, but the tests in this binary run
    // in PARALLEL threads — a global counting flag would fold sibling threads'
    // allocations into this test's window (a flaky over-count). Counting is
    // therefore THREAD-LOCAL: only the measuring thread's allocations count, so
    // the assertion is robust to concurrent siblings without `--test-threads=1`.

    thread_local! {
        static COUNTING: Cell<bool> = const { Cell::new(false) };
        static ACQUISITIONS: Cell<usize> = const { Cell::new(0) };
    }

    struct CountingAlloc;

    #[inline]
    fn note_alloc() {
        // `try_with`: during thread teardown the TLS may be gone — then skip
        // (we are never measuring at that point).
        let _ = COUNTING.try_with(|c| {
            if c.get() {
                let _ = ACQUISITIONS.try_with(|a| a.set(a.get() + 1));
            }
        });
    }

    // SAFETY: pure delegation to `System` with a thread-local counter side-effect;
    // the layout/pointer contracts are forwarded unchanged.
    unsafe impl GlobalAlloc for CountingAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            note_alloc();
            // SAFETY: forwarded verbatim to the system allocator.
            unsafe { System.alloc(layout) }
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            note_alloc();
            // SAFETY: forwarded verbatim to the system allocator.
            unsafe { System.alloc_zeroed(layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            note_alloc();
            // SAFETY: forwarded verbatim to the system allocator.
            unsafe { System.realloc(ptr, layout, new_size) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            // SAFETY: forwarded verbatim to the system allocator.
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    #[global_allocator]
    static ALLOC: CountingAlloc = CountingAlloc;

    /// A warm bridge (queue preallocated) ingesting a mixed key + mouse batch
    /// allocates ZERO heap (`push_raw` writes the existing ring; the pure
    /// translate is stack POD).
    #[test]
    fn ingest_captured_is_alloc_free_when_warm() {
        let mut queue = RawInputQueue::with_capacity(64);
        // Warm the queue outside the counting window.
        for _ in 0..8 {
            queue.begin_frame();
            ingest_captured(&mut queue, CapturedMsg::RawMouse { dx: 1, dy: 1 });
            while queue.pop().is_some() {}
        }

        ACQUISITIONS.with(|a| a.set(0));
        COUNTING.with(|c| c.set(true));
        queue.begin_frame();
        ingest_captured(&mut queue, CapturedMsg::Raw { msg: WM_KEYDOWN, wparam: 0, lparam: key_lparam(SC_W) });
        ingest_captured(&mut queue, CapturedMsg::RawMouse { dx: 4, dy: -2 });
        ingest_captured(&mut queue, CapturedMsg::Raw { msg: WM_KEYUP, wparam: 0, lparam: key_lparam(SC_W) });
        while queue.pop().is_some() {}
        COUNTING.with(|c| c.set(false));

        assert_eq!(
            ACQUISITIONS.with(|a| a.get()),
            0,
            "a warm ingest+drain must not allocate"
        );
    }
}
