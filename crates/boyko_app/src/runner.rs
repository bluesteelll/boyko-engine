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
use boyko_input::{ButtonState, KeyCode, RawInputEvent};
#[cfg(windows)]
use boyko_render::light_system::{LightTableGeneration, LightTableStaging};
#[cfg(windows)]
use boyko_render::{
    CsmCasterScratch, DdgiCaps, MeshRegistry, MeshRenderScratch, RayCaps, ResolvedCsm,
    ResolvedShadowAtlas, RhiContext, SdfEditStaging, collect_sdf_edits, gbuffer_push_from_view,
    upload_atlas_ring, upload_camera_ring, upload_csm_ring, upload_instance_models,
    upload_light_table, upload_pair_out_slot, upload_pair_ring, upload_sdf_edit_list,
};
#[cfg(all(windows, feature = "hwrt"))]
use boyko_render::{ResolvedRayShadow, upload_mesh_ids, upload_ray_shadow_ring};
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

    // ── Windowed `AppExit` semantics: insert-IF-ABSENT (plan D6; the legacy
    // headless path keeps its unconditional insert).
    if !app.world().contains_resource::<AppExit>() {
        app.world_mut().insert_resource(AppExit(false));
    }

    app.finish();

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
    let mut last = Instant::now();
    // SDFDDGI I2 (arm): a monotonically-incrementing frame index feeding the probe-update UBO's
    // round-robin `frame_index` (which subset updates this frame). Wraps at u32::MAX (benign — the
    // subset phase is `frame_index % subset_n`).
    let mut frame_index: u32 = 0;
    loop {
        // 1. Pump the OS queue; `false` = WM_QUIT (the window closed).
        if !host.window.pump_events() {
            return;
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
        // elements borrow the World's `MeshRegistry` buffers for this frame.
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
        // The dump's readback request (cold; `None` without the env knob). The
        // returned borrow holds `dump` until the render call consumes it.
        let readback = match dump.as_mut() {
            Some(d) => d.request(ctx, host.swapchain.extent()),
            None => None,
        };
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

            // 5d''. HW-RT rung 1b: the HWRT soft-shadow-params UBO into slot `s` —
            //       UNCONDITIONAL every HWRT frame (16 B; `resolve_ray_shadow_system`
            //       re-derives it from the author `RayShadowConfig`, so a boot-seed
            //       would go stale on a retune, see `upload_ray_shadow_ring`). GATED on
            //       an RT device (`ray_query_enabled`) — the SAME gate that mints the
            //       ring in `GpuSceneBundles::boot`, so an unminted slot is never
            //       uploaded; a software-only build pays zero (the whole block is
            //       `#[cfg(feature = "hwrt")]`).
            #[cfg(feature = "hwrt")]
            if ctx.ray_query_enabled() {
                let resolved_ray_shadow = world.resource::<ResolvedRayShadow>();
                // SAFETY: the HWRT shadow-params UBO ring slot — same provenance
                // contract as the cascade slot above (boot-minted at
                // RESOLVED_RAY_SHADOW_BYTES on the RT device under this same gate, live
                // until teardown, the fenced slot `s == token.slot()`).
                unsafe {
                    upload_ray_shadow_ring(
                        &token,
                        host.gpu.ray_shadow_ubo_slot(s),
                        resolved_ray_shadow,
                    );
                }
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
            let ddgi_enabled = ctx.device_caps().ddgi_storage_ok()
                && world
                    .try_resource::<boyko_render::DdgiConfig>()
                    .is_some_and(|cfg| cfg.enabled());
            // HW-RT rung R2a-3: TLAS arming — hwrt + an RT device + a non-empty gather. On an RT
            // device, first sync the frame-invariant BLAS-address table (a no-op unless the mesh
            // registry's `blas_generation` advanced — a BLAS never moves), then arm the per-frame
            // pack + build. On a non-RT device (or hwrt OFF) `tlas_enabled` is `false` → the
            // byte-identical OFF path (no pack, no build, no barrier).
            #[cfg(feature = "hwrt")]
            let tlas_enabled = {
                let on = ctx.ray_query_enabled() && scratch.instance_count() > 0;
                if ctx.ray_query_enabled() {
                    host.gpu.sync_tlas_blas_addr(ctx, registry);
                }
                on
            };
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
                ctx,
            );

            // 7. Render + present, consuming the token (the host-write window
            //    for slot `s` ends here — R0b).
            // SAFETY: `ctx`/`surface`/`swapchain`/`renderer` share the one
            // pinned device; every `scene` resource is live (owned by
            // `host.gpu` / the World's `MeshRegistry`, both outliving the
            // call); `present_extent` == the composite extent the camera
            // push `count`, `dispatch_group_count_x`, and the G-buffer targets
            // are sized to (all boot-fixed, plan D7); a `Some(readback)` is
            // the dump's host-visible staging, sized to the current swapchain
            // extent by `HostDump::request` (`None` on the steady path).
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
                    readback,
                )
            }
        };
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
