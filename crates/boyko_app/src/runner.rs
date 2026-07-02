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
use std::time::Instant;

#[cfg(windows)]
use boyko_input::{ButtonState, KeyCode, RawInputEvent, translate_win32};
#[cfg(windows)]
use boyko_render::light_system::{LightTableGeneration, LightTableStaging};
#[cfg(windows)]
use boyko_render::{
    CsmCasterScratch, MeshRegistry, MeshRenderScratch, ResolvedCsm, RhiContext,
    gbuffer_push_from_view, upload_camera_ring, upload_csm_ring, upload_instance_models,
    upload_light_table,
};
#[cfg(windows)]
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
#[cfg(windows)]
use boyko_rhi_vulkan::ffi::VkExtent2D;
#[cfg(windows)]
use boyko_rhi_vulkan::swapchain::{GBUFFER_PUSH_BYTES, GBufferMeshDraw};
#[cfg(windows)]
use boyko_rhi_vulkan::window::CapturedMsg;
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
}

/// The frame clear color — a dark neutral (the empty-gather / background tone).
#[cfg(windows)]
const CLEAR_COLOR: [f32; 4] = [0.05, 0.07, 0.10, 1.0];

/// The windowed runner body (host plan D6): boot → World residents →
/// insert-if-absent `AppExit(false)` → `finish()` → frame loop → D2 teardown.
///
/// Boot failure (no loader / GPU / window) is NOT a panic: the runner logs one
/// line at the binary boundary and returns `AppExit(true)` — the app must exit
/// gracefully on a GPU-less machine.
///
/// The shipped runner does NOT request the validation layer (review P1-3): per
/// [`InstanceConfig`]'s contract an ABSENT `VK_LAYER_KHRONOS_validation` fails
/// boot with `ValidationUnavailable` (no silent fallback), which would kill
/// the app on every machine without the Vulkan SDK. The messenger oracle stays
/// a test-harness convention; a debug validation knob arrives with a later
/// rung.
#[cfg(windows)]
pub(crate) fn run_windowed(app: &mut App, desc: WindowDesc) -> AppExit {
    // ── Boot: the device singleton (plan D2 step 1). ─────────────────────────
    let config = InstanceConfig {
        enable_validation: false,
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

    // ── World residents BEFORE `finish()` so the startup one-shots drain WITH
    // the device present (plan D2 step 4 / D6). The GPU handles are NonSend:
    // all GPU access stays runner-thread-only. `MeshRegistry` starts empty —
    // user startup registers meshes through `GpuDevice`. `WindowInfo` seeds at
    // the boot client size (its one-frame-stale contract starts post-present).
    app.world_mut()
        .insert_non_send_resource(RhiContext::from_shared(ctx));
    app.world_mut().insert_non_send_resource(GpuDevice(ctx));
    app.world_mut()
        .insert_non_send_resource(MeshRegistry::new());
    app.world_mut().insert_resource(WindowInfo {
        width: host.window.width(),
        height: host.window.height(),
    });
    // The R4 host probe (WindowInfo-adjacent, same post-present publish step):
    // lets headless smokes assert the light-upload gating + CSM arming decisions.
    app.world_mut().insert_resource(HostFrameStats::default());

    // ── Windowed `AppExit` semantics: insert-IF-ABSENT (plan D6; the legacy
    // headless path keeps its unconditional insert).
    if !app.world().contains_resource::<AppExit>() {
        app.world_mut().insert_resource(AppExit(false));
    }

    app.finish();

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
    // World GPU resident (`RhiContext`, `MeshRegistry`, `GpuDevice`), no
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
            && !app.world().contains_non_send_resource::<MeshRegistry>(),
        "invariant: the post-run World is GPU-evicted (plan D2)"
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

/// The per-frame loop (host plan D6 / the runner-frame table, R3 + R4):
///
/// 1. pump the OS queue + drain input (Escape only until R6);
/// 2. `update_with_delta` — Time → events → Fixed×N → Main (propagation,
///    camera resolve, `sync_instance_model_cols`, `gather_mesh_draws`,
///    `gather_shadow_casters`, light reconcile + collect, the CSM fit);
/// 3. `AppExit` check;
/// 4. `token = wait_frame_in_flight()` — the pacing point + the fence proof;
/// 5. token-typed uploads into slot `token.slot()`: the b5 camera block + the
///    instance-model ring (UNCONDITIONAL, plan D5) + the CSM cascade UBO
///    (unconditional 336 B) + the light staging iff
///    `light_uploaded_gen[s] != LightTableGeneration` (the D5 gate);
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
    let mut last = Instant::now();
    loop {
        // 1. Pump the OS queue; `false` = WM_QUIT (the window closed).
        if !host.window.pump_events() {
            return;
        }

        // Drain the captured input. R3 still discards everything except the
        // Escape key-down exit; the InputPlugin ingest arrives in R6.
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

        // 2. The ECS frame with the real wall delta (Time clamps/scales it).
        let now = Instant::now();
        let dt = now - last;
        last = now;
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

        // 5–7. Uploads + stack scene assembly + render. The draw list reuses
        // the host's parked allocation (0 alloc/frame after warmup); its
        // elements borrow the World's `MeshRegistry` buffers for this frame.
        // The two flags feed the post-present `HostFrameStats` publish (step 8):
        // the light flag is set on the gated branch; the csm flag is assigned
        // exactly once inside the block (definite-initialization, no dead seed).
        let mut frame_light_uploaded = false;
        let frame_csm_armed;
        let mut draws = host.draw_scratch.take();
        let presented = {
            let world = app.world();
            let view = *world.resource::<ViewUniform>();

            // 5a. The 80-byte b5 camera block into slot `s` (plan D7: the
            //     composite extent, not the window extent).
            // SAFETY: `host.gpu.camera_ring[s]` is a live host-visible buffer
            // minted by `GpuSceneBundles::boot` (`RhiDevice::create_buffer`,
            // HostVisibleCoherent — its `mapped`/`size` are the RHI's own, not
            // hand-built) and destroyed only in teardown after the loop; it IS
            // the fenced slot's buffer (`s == token.slot()`), satisfying both
            // upload preconditions.
            unsafe {
                upload_camera_ring(&token, &host.gpu.camera_ring[s], &view, cw, ch);
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

            // 6a. DrawBatch → GBufferMeshDraw: resolve each batch's mesh to its
            //     registry GPU buffers (the showcase's ~8070 conversion, driven
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
            let registry = world.non_send_resource::<MeshRegistry>();
            let casters = world.resource::<CsmCasterScratch>();
            let caster_batches = casters.batches();
            let mut ci = 0usize;
            for b in &scratch.batches {
                while ci < caster_batches.len() && caster_batches[ci].mesh_id < b.mesh_id {
                    ci += 1;
                }
                let casts_shadow =
                    ci < caster_batches.len() && caster_batches[ci].mesh_id == b.mesh_id;
                let mesh = registry.get(MeshHandle(b.mesh_id));
                draws.push(GBufferMeshDraw {
                    vertex_buffer: &mesh.vertex_buffer,
                    index_buffer: &mesh.index_buffer,
                    index_count: b.index_count,
                    index_type: b.index_type.as_i32(),
                    base_instance: b.base_instance,
                    instance_count: b.instance_count,
                    casts_shadow,
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
                gbuffer_push_from_view(&view, cw, ch, instanced)
            } else {
                let mut zeroed = [0u8; GBUFFER_PUSH_BYTES];
                if instanced {
                    // The recorder contract: byte 84 (`use_model_matrix`) MUST
                    // be 1 whenever `mesh_draw` is non-empty.
                    zeroed[84..88].copy_from_slice(&1u32.to_le_bytes());
                }
                zeroed
            };
            let scene = host.gpu.scene(mvp, s, &draws, light_upload, csm_armed.then_some(resolved_csm));

            // 7. Render + present, consuming the token (the host-write window
            //    for slot `s` ends here — R0b).
            // SAFETY: `ctx`/`surface`/`swapchain`/`renderer` share the one
            // pinned device; every `scene` resource is live (owned by
            // `host.gpu` / the World's `MeshRegistry`, both outliving the
            // call); `present_extent` == the composite extent the camera
            // push `count`, `dispatch_group_count_x`, and the G-buffer targets
            // are sized to (all boot-fixed, plan D7); no readback requested.
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
                    None,
                )
            }
        };
        host.draw_scratch.put(draws);

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
        }
    }
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
    // Steps 1+2 — unbundle the host and drop the renderer FIRST: its `Drop`
    // performs the `vkDeviceWaitIdle` (frame_driver.rs), so everything after
    // runs under an idle device. Then destroy the extent-dependent targets and
    // the static scene bundles EXPLICITLY (no `Drop` glue on RHI resources),
    // and only then drop swapchain → surface → window (the surface before the
    // window it borrows).
    let WindowHost {
        renderer,
        frame,
        gpu,
        draw_scratch: _,
        composite_extent: _,
        light_uploaded_gen: _,
        swapchain,
        surface,
        window,
    } = host;
    drop(renderer);
    // SAFETY: the renderer drop above waited the device idle, so no submission
    // references the targets or the scene bundles; `ctx` is the live context
    // they were created on; each is destroyed exactly once (by-value moves).
    unsafe {
        frame.destroy(ctx);
        gpu.destroy(ctx);
    }
    drop(swapchain);
    drop(surface);
    drop(window);

    // Step 3 — EVICT every device-referencing World resident (the runner
    // borrows the App and cannot drop it — explicit eviction is the
    // replacement; plan critic delta A1):
    //   - `RhiContext` (shared mode): its `Drop` runs `destroy_all` (frees
    //     every column/UI resource; the registry teardown wait-idles the
    //     already-idle device — a benign no-op) and NEVER touches the device
    //     lifecycle;
    //   - `MeshRegistry`: its buffers are destroyed through `ctx` under the
    //     step-1 idle (`unsafe destroy` — the registry has no `Drop` glue);
    //   - `GpuDevice`: the last world-resident `&'static` handle — no dangling
    //     `&'static` may remain in a live structure past this point.
    drop(app.world_mut().remove_non_send_resource::<RhiContext>());
    if let Some(mut registry) = app.world_mut().remove_non_send_resource::<MeshRegistry>() {
        // SAFETY: the device is idle (step 1) so no in-flight submit references
        // any mesh buffer; `ctx` is the context they were created on; the
        // registry is destroyed exactly once (just removed from the World).
        unsafe { registry.destroy(ctx) };
    }
    // `GpuDevice` is a plain reference newtype (no `Drop` glue) — removal alone
    // ends its residency; the returned `Option` is discarded.
    let _ = app.world_mut().remove_non_send_resource::<GpuDevice>();

    // Step 4 (`destroy_singleton`) is the CALLER's — see the fn contract: it
    // must run AFTER this fn returns, once `ctx`'s protector is gone.
}
