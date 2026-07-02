//! S3 GPU GATE — the ENGINE camera drives the GPU render (RTX 3060 oracle).
//!
//! This is the end-to-end proof that the S3 view path is the EXECUTED seam, not
//! dead exports: a real ECS world resolves [`resolve_active_camera`] into a
//! [`ViewUniform`], the `boyko_render::view` bridge turns that one view into the
//! marcher's [`CompositePushConstants`], the GPU marches an SDF scene, and the
//! pixels are read back. Moving the ECS [`Camera`] entity (pose A → pose B) and
//! re-resolving must change the rendered image CONSISTENTLY with the move — a
//! known scene point lands on the pixel the bridged camera predicts.
//!
//! # Why this lives in `boyko_render/tests`
//!
//! It is the only crate that may name BOTH the scene vocabulary (`boyko_scene`)
//! and the marcher push-constant struct (`boyko_rhi_vulkan`); the low-level
//! backend must not depend upward on the scene crate. The marcher buffer-layout
//! helpers (`seed_buffer` / `read_pixels` / `run_marcher`) are test-local in
//! `boyko_rhi_vulkan/tests`, so they are mirrored here verbatim and fed the
//! BRIDGED push constants.
//!
//! Run single-threaded with validation: `cargo test -p boyko-render
//! --test camera_drives_render_gpu --release -- --test-threads=1 --nocapture`.

mod common;

use core::ptr::NonNull;

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Bundle;

use boyko_rhi::{
    BufferDesc, BufferUsage, ComputePipelineDesc, MemoryLocation, RhiCommandEncoder, RhiDevice,
    RhiQueue, ShaderStage,
};
use boyko_rhi_vulkan::compute::{
    COMPOSITE_DEPTH_BASE_WORDS, COMPOSITE_PUSH_CONSTANT_BYTES, CAM_MODE_PERSPECTIVE, CompositeCamera,
    CompositePushConstants, LOCAL_SIZE_X, MESH_DEPTH_CLEAR, SdfEdit,
    sdf_depth_composite_spirv, sdf_op,
};
// The CPU golden oracle (audit W3/R-2 split out of `compute`): behind the `goldens`
// cargo feature, ON for this crate's test targets via the dev-dependency below.
use boyko_rhi_vulkan::goldens::golden_composite_pixel_ex;
use boyko_rhi_vulkan::device::VulkanContext;

use boyko_render::composite_perspective_from_view;

use boyko_scene::{
    ActiveCamera, Camera, GlobalTransform, Projection, ViewUniform, resolve_active_camera,
};

use boyko_math::{Affine3A, Mat3, Vec3};

use common::{assert_validation_clean, boot_or_skip};

use core::f32::consts::FRAC_PI_3;

// ── Marcher buffer layout (mirrors boyko_rhi_vulkan/tests/sdf_perspective_resolution.rs) ──

const DEPTH_BASE: usize = 196;
const _: () = assert!(DEPTH_BASE == COMPOSITE_DEPTH_BASE_WORDS);

#[inline]
fn buffer_words(w: u32, h: u32) -> usize {
    DEPTH_BASE + 2 * (w as usize) * (h as usize)
}

#[inline]
fn pixel_base_words(w: u32, h: u32) -> usize {
    DEPTH_BASE + (w as usize) * (h as usize)
}

#[inline]
fn group_count(w: u32, h: u32) -> u32 {
    ((w as u64 * h as u64) as u32).div_ceil(LOCAL_SIZE_X)
}

/// A single small sphere at the origin — a compact target the camera centers on,
/// leaving most of the frame empty so a camera MOVE visibly shifts where it lands.
fn sphere_scene() -> Vec<SdfEdit> {
    vec![SdfEdit::sphere([0.0, 0.0, 0.0], 0.35, sdf_op::UNION, 0.0)]
}

fn seed_buffer(base: NonNull<u8>, edits: &[SdfEdit], w: u32, h: u32) {
    let dst = base.as_ptr().cast::<u32>();
    let n_pixels = (w as usize) * (h as usize);
    // SAFETY: `dst` is the start of a `buffer_words(w,h)*4`-byte host-coherent mapping
    // (the buffer was created at exactly that size); every index written is < that word
    // count. No GPU work is in flight (submit happens after); `write_unaligned`
    // tolerates the sub-allocated offset.
    unsafe { dst.write_unaligned(edits.len() as u32) };
    for (i, e) in edits.iter().enumerate() {
        let off = 4 + i * 12;
        let words = [
            e.center[0].to_bits(),
            e.center[1].to_bits(),
            e.center[2].to_bits(),
            e.center[3].to_bits(),
            e.params[0].to_bits(),
            e.params[1].to_bits(),
            e.params[2].to_bits(),
            e.params[3].to_bits(),
            e.kind,
            e.op,
            e.smoothness.to_bits(),
            e._pad,
        ];
        for (j, &word) in words.iter().enumerate() {
            // SAFETY: `off + j < DEPTH_BASE` for the fixed-cap edit array, in-bounds.
            unsafe { dst.add(off + j).write_unaligned(word) };
        }
    }
    let clear_bits = MESH_DEPTH_CLEAR.to_bits();
    for i in 0..n_pixels {
        // SAFETY: `DEPTH_BASE + i` for `i < n_pixels` is the depth region, in-bounds.
        unsafe { dst.add(DEPTH_BASE + i).write_unaligned(clear_bits) };
    }
}

fn read_pixels(base: NonNull<u8>, w: u32, h: u32) -> Vec<u32> {
    let n = (w as usize) * (h as usize);
    let pbase = pixel_base_words(w, h);
    let p = base.as_ptr().cast::<u32>();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        // SAFETY: `pbase + i` for `i < n` is the pixel region, in-bounds; a fence wait
        // preceded this read so GPU writes are complete + coherent.
        out.push(unsafe { p.add(pbase + i).read_unaligned() });
    }
    out
}

/// Records + submits ONE compute-only marcher dispatch driven by `pc`, fence-waits,
/// asserts validation-clean, and returns the readback pixels.
fn run_marcher(
    ctx: &VulkanContext,
    edits: &[SdfEdit],
    pc: CompositePushConstants,
    w: u32,
    h: u32,
    label: &str,
) -> Vec<u32> {
    let device: &VulkanContext = ctx;
    let queue = ctx.rhi_queue();

    let buffer = device
        .create_buffer(&BufferDesc {
            size: (buffer_words(w, h) as u64) * 4,
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("runtime-extent shared storage buffer");

    {
        let mapped = device.buffer_mapped_ptr(&buffer).expect("host-visible buffer mapped");
        seed_buffer(mapped, edits, w, h);
    }

    let cs = device
        .create_shader_module(sdf_depth_composite_spirv())
        .expect("composite compute shader module");
    let compute = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &cs,
            entry: c"main",
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: None,
        })
        .expect("composite compute pipeline");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");

    encoder.begin().expect("begin");
    encoder.bind_compute_pipeline(&compute);
    encoder.bind_storage_buffer(&buffer, 0, 0);
    encoder.push_constants(ShaderStage::COMPUTE, 0, pc.as_bytes());
    encoder.dispatch(group_count(w, h), 1, 1);
    encoder.end().expect("end");

    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    let mapped = device.buffer_mapped_ptr(&buffer).expect("host-visible buffer mapped");
    let pixels = read_pixels(mapped, w, h);
    assert_eq!(pixels.len(), (w as usize) * (h as usize), "{label}: full readback");

    assert_validation_clean(ctx);

    // SAFETY: every resource was created on `device` and is destroyed exactly once;
    // the submission completed (fence-waited above), so none is GPU-in-use.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_compute_pipeline(compute);
        device.destroy_shader_module(cs);
        device.destroy_buffer(buffer);
    }

    pixels
}

// ── The ECS camera vehicle ──

#[derive(Bundle)]
struct CameraBundle {
    camera: Camera,
    projection: Projection,
    global: GlobalTransform,
}

/// A camera world with the resources seeded but no schedule (the resolver is run
/// on demand). Mirrors `CameraPlugin::build`'s resource seeding.
fn camera_app() -> App {
    let mut app = App::new();
    app.insert_resource(ActiveCamera::default());
    app.insert_resource(ViewUniform::default());
    app.finish();
    app
}

/// Spawns a perspective camera at `eye` (identity rotation, looking down -Z) and
/// returns its live handle.
fn spawn_camera_at(world: &mut EcsMaster, eye: Vec3) -> Entity {
    use std::sync::{Arc, Mutex};
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    // Capture the `Copy` components and build the bundle INSIDE the closure so the
    // closure stays `FnMut` (`run_system` requires it) — a moved-out bundle would
    // make it `FnOnce`.
    let camera = Camera::DEFAULT;
    let projection = Projection::Perspective {
        fov_y: FRAC_PI_3,
        aspect: 1.0,
        near: 0.1,
        far: 100.0,
    };
    let global = GlobalTransform(Affine3A {
        matrix3: Mat3::IDENTITY,
        translation: eye,
    });
    world.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(CameraBundle {
                camera,
                projection,
                global,
            })
            .id();
        *probe.lock().expect("probe lock") = Some(e);
    });
    let e = sink.lock().expect("probe lock").expect("spawn handle");
    assert!(world.has_entity(e), "spawned camera is live");
    e
}

/// Sets the camera entity's world pose (eye position; identity rotation).
fn move_camera_to(world: &mut EcsMaster, cam: Entity, eye: Vec3) {
    world.run_system(move |mut q: boyko_ecs::ecs::core::iters::query::Query<&mut GlobalTransform>| {
        for (id, g) in q.iter_entities_mut() {
            if id == cam.id() {
                g.0.translation = eye;
            }
        }
    });
}

/// The host-mirror [`CompositeCamera`] that corresponds to a bridged
/// [`CompositePushConstants`] — same eye/basis/FOV/aspect — so the CPU golden
/// predicts the GPU pixel-for-pixel. Built from the SAME `ViewUniform` lanes the
/// bridge consumed, proving the GPU saw the engine camera.
fn host_camera_from_view(view: &ViewUniform, w: u32, h: u32) -> CompositeCamera {
    let tan_half_fov = (view.fov_y * 0.5).tan();
    CompositeCamera::Perspective {
        eye: [view.camera_pos.x, view.camera_pos.y, view.camera_pos.z],
        forward: [view.cam_forward.x, view.cam_forward.y, view.cam_forward.z],
        right: [view.cam_right.x, view.cam_right.y, view.cam_right.z],
        up: [view.cam_up.x, view.cam_up.y, view.cam_up.z],
        tan_half_fov,
        aspect: (w as f32) / (h as f32),
    }
}

fn unpack_rgb(packed: u32) -> [i32; 3] {
    [
        (packed & 0xFF) as i32,
        ((packed >> 8) & 0xFF) as i32,
        ((packed >> 16) & 0xFF) as i32,
    ]
}

/// A pixel is a "hit" (the lit sphere) when its red channel is high; background is
/// the dark `(13,13,26)`-ish color.
fn is_hit(packed: u32) -> bool {
    unpack_rgb(packed)[0] > 60
}

/// The centroid (mean px, py) of the hit pixels — the screen location the camera
/// places the sphere. Returns `None` if nothing was hit.
fn hit_centroid(pixels: &[u32], w: u32, h: u32) -> Option<(f32, f32)> {
    let mut sx = 0.0f64;
    let mut sy = 0.0f64;
    let mut n = 0u64;
    for py in 0..h {
        for px in 0..w {
            if is_hit(pixels[(py * w + px) as usize]) {
                sx += px as f64;
                sy += py as f64;
                n += 1;
            }
        }
    }
    (n > 0).then(|| ((sx / n as f64) as f32, (sy / n as f64) as f32))
}

/// Resolves the active camera and bridges it to the marcher push constants.
fn resolve_and_bridge(app: &mut App, w: u32, h: u32) -> (ViewUniform, CompositePushConstants) {
    app.world_mut().run_system(resolve_active_camera);
    let view = *app.world().resource::<ViewUniform>();
    let pc = composite_perspective_from_view(&view, w, h);
    (view, pc)
}

// ════════════════════════════════════════════════════════════════════════════
// GATE — moving the ECS camera changes the GPU render consistently with the move
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn ecs_camera_move_drives_gpu_render() {
    let Some(ctx) = boot_or_skip("ecs_camera_move_drives_gpu_render") else {
        eprintln!("GPU UNAVAILABLE — camera_drives_render not exercised");
        return;
    };
    println!("[ecs_camera_move_drives_gpu_render] device (validation on): {}", ctx.device_name());
    if !ctx.validation_enabled() {
        // The box-level BOYKO_DISABLE_VALIDATION escape hatch (the validation layer is
        // crash-prone on some machines) removes the layer this gate exists to exercise —
        // SKIP, mirroring the no-device SKIP convention, instead of failing the suite.
        assert!(
            std::env::var_os("BOYKO_DISABLE_VALIDATION").is_some(),
            "validation must be active when enable_validation is set and the escape hatch is absent"
        );
        eprintln!("SKIP ecs_camera_move_drives_gpu_render: validation disabled (BOYKO_DISABLE_VALIDATION)");
        return;
    }

    let (w, h) = (128u32, 128u32);
    let edits = sphere_scene();

    let mut app = camera_app();

    // ── POSE A: camera on +Z, looking at the origin head-on ⇒ sphere centered. ──
    let cam = spawn_camera_at(app.world_mut(), Vec3::new(0.0, 0.0, 3.0));
    let (view_a, pc_a) = resolve_and_bridge(&mut app, w, h);
    assert_eq!(pc_a.camera_mode, CAM_MODE_PERSPECTIVE);
    assert_eq!([view_a.camera_pos.x, view_a.camera_pos.y, view_a.camera_pos.z], [0.0, 0.0, 3.0]);

    let pixels_a = run_marcher(&ctx, &edits, pc_a, w, h, "poseA");
    let centroid_a = hit_centroid(&pixels_a, w, h).expect("pose A must SEE the sphere");
    println!("pose A: eye {:?}, sphere centroid {:?}", view_a.camera_pos, centroid_a);

    // ── POSE B: shift the camera +X (eye moves right) but keep looking down -Z.
    //    The sphere stays at the origin, so it must shift LEFT on screen. ──
    move_camera_to(app.world_mut(), cam, Vec3::new(1.0, 0.0, 3.0));
    let (view_b, pc_b) = resolve_and_bridge(&mut app, w, h);
    assert_eq!([view_b.camera_pos.x, view_b.camera_pos.y, view_b.camera_pos.z], [1.0, 0.0, 3.0]);

    let pixels_b = run_marcher(&ctx, &edits, pc_b, w, h, "poseB");
    let centroid_b = hit_centroid(&pixels_b, w, h).expect("pose B must still SEE the sphere");
    println!("pose B: eye {:?}, sphere centroid {:?}", view_b.camera_pos, centroid_b);

    // (1) The render CHANGED: the two frames differ in a non-trivial number of pixels.
    let diff = pixels_a.iter().zip(pixels_b.iter()).filter(|(a, b)| a != b).count();
    assert!(
        diff > (w as usize * h as usize) / 100,
        "moving the camera must change the render (only {diff} px differed)"
    );

    // (2) The change is CONSISTENT with the move: eye moved +X, the centered sphere
    //     must shift LEFT (smaller px) on screen. A camera that did NOT consume the
    //     engine view would leave the centroid put.
    assert!(
        centroid_b.0 < centroid_a.0 - 2.0,
        "eye +X must shift the sphere LEFT: centroid_a.x = {}, centroid_b.x = {}",
        centroid_a.0,
        centroid_b.0
    );

    // (3) The KNOWN scene point (sphere center, world origin) projects to the pixel
    //     the bridged camera predicts — for BOTH poses, via the host mirror golden.
    //     This is the "a known scene point projects to the expected pixel" assertion:
    //     the host golden derives from the SAME ViewUniform lanes, and the GPU agrees.
    assert_gpu_matches_bridged_host(&pixels_a, &edits, &view_a, w, h, "poseA");
    assert_gpu_matches_bridged_host(&pixels_b, &edits, &view_b, w, h, "poseB");
}

/// Asserts every GPU pixel equals the host golden built from the BRIDGED camera
/// (within ±2/255), and that the camera actually saw the sphere (anti-vacuity).
/// Because the host golden's ray-gen is fed the bridged `ViewUniform`'s eye/basis,
/// agreement proves the GPU rendered FROM the engine camera, not a hardcoded view.
fn assert_gpu_matches_bridged_host(
    pixels: &[u32],
    edits: &[SdfEdit],
    view: &ViewUniform,
    w: u32,
    h: u32,
    label: &str,
) {
    const CHANNEL_TOL: i32 = 2;
    let host_cam = host_camera_from_view(view, w, h);
    let mut max_delta = 0i32;
    let mut worst = (0u32, 0u32, 0u32, 0u32);
    let mut hits = 0usize;
    for py in 0..h {
        for px in 0..w {
            let idx = (py * w + px) as usize;
            let got = pixels[idx];
            let want = golden_composite_pixel_ex(edits, MESH_DEPTH_CLEAR, px, py, w, h, host_cam);
            if is_hit(got) {
                hits += 1;
            }
            let g = unpack_rgb(got);
            let wv = unpack_rgb(want);
            for c in 0..3 {
                let d = (g[c] - wv[c]).abs();
                if d > max_delta {
                    max_delta = d;
                    worst = (px, py, got, want);
                }
            }
        }
    }
    println!("{label}: max per-channel delta = {}/255, lit pixels {}", max_delta, hits);
    assert!(
        max_delta <= CHANNEL_TOL,
        "{label}: GPU diverged from the bridged-camera host golden by {}/255 at px ({},{}): got {:#010x} want {:#010x}",
        max_delta,
        worst.0,
        worst.1,
        worst.2,
        worst.3
    );
    assert!(
        hits > (w as usize * h as usize) / 400,
        "{label}: anti-vacuity — the bridged camera must SEE the sphere (only {hits} lit px)"
    );
}
