//! S35 GPU PROOF (b) — a live WINDOWED orbit: the `OrbitCamera` rig drives the
//! on-screen `ViewUniform`, which drives a CUBE rendered to the swapchain.
//!
//! The owner runs this and watches a colored cube spin (the camera orbits it).
//! Each frame the rig's `yaw` advances, the REAL `orbit_camera_system` →
//! `propagate_transforms` → `resolve_active_camera` chain (under a real
//! `App::update` / `Schedule::run` — the BUG-S35-1 correctness path) resolves a
//! fresh `ViewUniform`, `view_proj_columns(view.view_proj)` is flattened
//! COLUMN-MAJOR into the 64-byte MVP push constant, and `Scene::set_mvp` hands it
//! to the renderer's per-frame `cmd_push_constants` re-push. Model = identity
//! (the cube sits at the origin), so the MVP is exactly the engine `view_proj`.
//!
//! ALSO: on ONE frame at a KNOWN orbit angle it requests the swapchain-image
//! readback and writes `target/screenshots/s35_orbit_window.bmp`, so the
//! orchestrator can verify the windowed path produced the right oblique view
//! without watching the live window.
//!
//! # Layering
//!
//! This lives in `boyko_render` — the only crate that may name BOTH the windowed
//! `Renderer`/`Scene`/`Surface`/`Swapchain`/`Window` (`boyko_rhi_vulkan`) AND the
//! camera systems → `ViewUniform` (`boyko_scene`). The windowed present recipe +
//! the reused rung-3/4 MVP shaders + `swapchain_format_to_rhi` + the no-dep BMP
//! writer are mirrored from `boyko_rhi_vulkan/tests/window_present_scene.rs` +
//! `boyko_render/tests/p7b_world_ui_screenshot.rs` (an example cannot reach a
//! `tests/` helper, so the small ones are duplicated here).
//!
//! # Graceful skip
//!
//! `#[cfg(windows)]`; on a host with no window / no Vulkan / no surface / no
//! swapchain it prints a SKIP and returns (never hard-fails on a headless box).
//!
//! Owner-run command:
//!
//! ```text
//! cargo run -p boyko-render --example orbit_cube_window --release
//! ```
//!
//! Readback image: `D:\claude\BoykoEngine\target\screenshots\s35_orbit_window.bmp`

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use core::slice;

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Bundle;

use boyko_rhi::{
    BufferDesc, BufferUsage, Format, GraphicsPipelineDesc, MemoryLocation, PrimitiveTopology,
    RhiDevice, VertexAttribute, VertexBufferLayout, VertexFormat,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::ffi::{
    VK_FORMAT_B8G8R8A8_SRGB, VK_FORMAT_B8G8R8A8_UNORM, VK_FORMAT_R8G8B8A8_SRGB,
    VK_FORMAT_R8G8B8A8_UNORM,
};
use boyko_rhi_vulkan::swapchain::{Renderer, SCENE_MVP_BYTES, Scene, Surface, Swapchain};
use boyko_rhi_vulkan::window::Window;

use boyko_render::view_proj_columns;

use boyko_scene::{
    ActiveCamera, Camera, GlobalTransform, OrbitCamera, Projection, Transform, ViewUniform,
    orbit_camera_system, propagate_transforms, resolve_active_camera,
};

use core::f32::consts::FRAC_PI_4;

// ════════════════════════════════════════════════════════════════════════════
// Window / camera / orbit constants
// ════════════════════════════════════════════════════════════════════════════

/// The window's client size.
const WIDTH: u32 = 512;
const HEIGHT: u32 = 512;

/// Camera vertical FOV (45°) and clip planes.
const FOV_Y: f32 = FRAC_PI_4;
const ASPECT: f32 = WIDTH as f32 / HEIGHT as f32;
const NEAR: f32 = 0.1;
const FAR: f32 = 100.0;

/// The cube sits at the origin; the rig orbits THIS point at this radius. A larger
/// radius (telephoto-ish) keeps the perspective mild so the cube reads as a clean
/// box rather than a wide-angle bulge.
const TARGET: [f32; 3] = [0.0, 0.0, 0.0];
const ORBIT_DISTANCE: f32 = 7.0;
/// A LEVEL camera (pitch 0): the orbit sweeps the cube's equator so world-vertical
/// edges (‖ +Y) stay PERFECTLY PARALLEL on screen (no keystoning) — a camera tilt
/// would converge them toward the vertical vanishing point. We still read full 3D
/// from the front + side faces meeting at the receding vertical edge.
const ORBIT_PITCH: f32 = 0.0;
/// Per-frame yaw advance (radians) — ~0.03 rad/frame ≈ a full orbit every ~210
/// frames.
const YAW_STEP: f32 = 0.03;
/// Total frames to render before a clean teardown (≈ a couple of full orbits).
const TOTAL_FRAMES: u32 = 240;

/// The frame on which to request the swapchain readback → BMP. Chosen at a KNOWN
/// orbit angle (yaw = `READBACK_FRAME * YAW_STEP`, an oblique view) so the
/// orchestrator can cross-verify the windowed path off-line.
const READBACK_FRAME: u32 = 20;

/// A fixed per-update delta — keeps `Instant::now` jitter out of the frame
/// driver (the engine never reads wall-clock here; the rig advance is explicit).
const FIXED_DELTA: Duration = Duration::from_millis(16);

/// The clear color bytes (R, G, B, A) — a dark slate background.
const CLEAR_BYTES: [u8; 4] = [0x12, 0x14, 0x1A, 0xFF];

// ════════════════════════════════════════════════════════════════════════════
// The cube mesh (8 corners → 12 triangles → 36 vertices; 6 distinctly-colored
// faces so rotation is obvious). The rung-3/4 28-byte Vertex layout.
// ════════════════════════════════════════════════════════════════════════════

/// One vertex: a `Float32x3` position (offset 0) + a `Float32x4` color (offset
/// 12) — the rung-3/4 vertex layout (28-byte stride). `#[repr(C)]` for the exact
/// stride. Mirrors `window_present_scene::Vertex`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 4],
}

const VERTEX_STRIDE: u32 = core::mem::size_of::<Vertex>() as u32;
const _: () = assert!(VERTEX_STRIDE == 28, "Vertex must be tightly packed at 28 bytes");

/// Builds a unit cube `[-h, h]³` as 36 vertices (6 faces × 2 triangles × 3
/// verts), each face a distinct opaque color so the orbit visibly reveals
/// different faces. The model is centered at the origin (so MVP model = identity
/// and the cube tracks `TARGET`). Winding is CCW-front for each outward face; the
/// pipeline does no back-face cull (the rung-3/4 pipeline), so winding is purely
/// cosmetic here.
fn cube_vertices() -> [Vertex; 36] {
    const H: f32 = 0.9;
    // 8 corners.
    let c = [
        [-H, -H, -H], // 0
        [H, -H, -H],  // 1
        [H, H, -H],   // 2
        [-H, H, -H],  // 3
        [-H, -H, H],  // 4
        [H, -H, H],   // 5
        [H, H, H],    // 6
        [-H, H, H],   // 7
    ];
    // Per-face colors (R,G,B,A): +X red, −X green, +Y blue, −Y yellow,
    // +Z magenta, −Z cyan.
    let red = [1.0, 0.2, 0.2, 1.0];
    let green = [0.2, 1.0, 0.2, 1.0];
    let blue = [0.3, 0.4, 1.0, 1.0];
    let yellow = [1.0, 0.9, 0.2, 1.0];
    let magenta = [1.0, 0.3, 1.0, 1.0];
    let cyan = [0.2, 1.0, 1.0, 1.0];

    // Each face: two CCW triangles (viewed from outside).
    let faces: [([usize; 4], [f32; 4]); 6] = [
        ([1, 2, 6, 5], red),     // +X
        ([4, 7, 3, 0], green),   // −X
        ([3, 7, 6, 2], blue),    // +Y
        ([4, 0, 1, 5], yellow),  // −Y
        ([5, 6, 7, 4], magenta), // +Z
        ([0, 3, 2, 1], cyan),    // −Z
    ];

    let mut verts = [Vertex { position: [0.0; 3], color: [0.0; 4] }; 36];
    let mut vi = 0usize;
    for (quad, color) in faces {
        // Two triangles: (a,b,c) and (a,c,d).
        for &idx in &[quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]] {
            verts[vi] = Vertex { position: c[idx], color };
            vi += 1;
        }
    }
    debug_assert!(vi == 36, "cube emits exactly 36 vertices");
    verts
}

/// Flattens the engine view-proj into the 64-byte MVP push constant in the layout
/// the reused rung-3/4 vertex shader expects: **COLUMN-MAJOR** (column 0's 4 floats
/// first, …). `triangle_mvp.vs.hlsl` declares `float4x4 mvp` and computes
/// `mul(pc.mvp, float4(pos, 1))` = `M · v`. DXC compiles the `float4x4` to SPIR-V
/// with the default COLUMN-MAJOR matrix packing (the Vulkan/SPIR-V convention), so
/// the shader reconstructs `M` from a column-major byte stream — uploading
/// `view_proj`'s columns in order makes `M == view_proj` and the transform exact.
///
/// `view_proj_columns` returns `[[f32;4];4]` with each inner array already a COLUMN
/// of the engine `Mat4` (`cols[c] = (m0c, m1c, m2c, m3c)`, a pure field copy of
/// `Mat4::cols`, no transpose), so writing the inner arrays in order yields exactly
/// that column-major stream. `view_proj` IS the MVP because the cube's model
/// transform is identity (centered at the origin).
///
/// Verified empirically on the RTX: a head-on capture (yaw≈0) renders the `+Z`
/// magenta face as a centered square, and an oblique capture renders a coherent
/// 3-face cube — a transposed (row-major) upload instead collapses the perspective
/// to a degenerate sliver. (`window_present_scene`'s golden only ever fed a DIAGONAL
/// MVP, which is transpose-invariant, so this rig path is the first non-symmetric
/// exerciser of the byte order.)
fn mvp_bytes_from_view(view: &ViewUniform) -> [u8; SCENE_MVP_BYTES] {
    let cols = view_proj_columns(view.view_proj); // cols[c] = column c = [m0c, m1c, m2c, m3c]
    let mut out = [0u8; SCENE_MVP_BYTES];
    let mut byte = 0usize;
    for col in cols {
        for f in col {
            out[byte..byte + 4].copy_from_slice(&f.to_le_bytes());
            byte += 4;
        }
    }
    out
}

// --- Committed rung-3/4 SPIR-V, reused unchanged (depth from the transformed
//     `gl_Position.z`, so the MVP vertex+fragment shaders suffice). Mirrors
//     window_present_scene's blob loading. ---

/// A 4-byte-aligned wrapper around a committed SPIR-V byte blob so its address is
/// a valid `*const u32` and it can be re-viewed as a `&[u32]` word stream.
#[repr(C, align(4))]
struct SpirvBlob<const N: usize>([u8; N]);

impl<const N: usize> SpirvBlob<N> {
    fn as_words(&self) -> &[u32] {
        const { assert!(N.is_multiple_of(4), "SPIR-V byte length must be a multiple of 4") };
        // SAFETY: the `align(4)` wrapper makes `self.0`'s address a valid `*const
        // u32`; `N` is a 4-byte multiple (const-asserted above), so the blob is
        // exactly `N / 4` whole `u32` words; the `&self` borrow keeps the `'static`
        // blob alive for the slice's lifetime; any bit pattern is a valid `u32`.
        unsafe { slice::from_raw_parts(self.0.as_ptr().cast::<u32>(), N / 4) }
    }
}

/// The committed rung-3 vertex SPIR-V (`triangle_mvp.vs.spv`, 916 bytes), reused.
/// The shader is in the `boyko_rhi_vulkan` crate's `shaders/` dir (the backend
/// owns the committed scene shaders).
static MVP_VS_SPV: SpirvBlob<916> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../boyko_rhi_vulkan/shaders/triangle_mvp.vs.spv"
)));

/// The committed rung-3 fragment SPIR-V (`triangle_mvp.fs.spv`, 368 bytes).
static MVP_FS_SPV: SpirvBlob<368> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../boyko_rhi_vulkan/shaders/triangle_mvp.fs.spv"
)));

/// Maps the swapchain's `i32` `VkFormat` to a `boyko_rhi::Format` (the pipeline's
/// declared color attachment must equal the swapchain image). An unexpected
/// format returns `None` → a graceful SKIP. Cloned from
/// `window_present_scene::swapchain_format_to_rhi`.
fn swapchain_format_to_rhi(vk_format: i32) -> Option<Format> {
    match vk_format {
        f if f == VK_FORMAT_B8G8R8A8_UNORM => Some(Format::B8G8R8A8Unorm),
        f if f == VK_FORMAT_R8G8B8A8_UNORM => Some(Format::R8G8B8A8Unorm),
        f if f == VK_FORMAT_B8G8R8A8_SRGB => Some(Format::B8G8R8A8Srgb),
        f if f == VK_FORMAT_R8G8B8A8_SRGB => Some(Format::R8G8B8A8Srgb),
        _ => None,
    }
}

/// A byte color → the RGBA floats passed as a clear.
fn floats(bytes: [u8; 4]) -> [f32; 4] {
    [
        bytes[0] as f32 / 255.0,
        bytes[1] as f32 / 255.0,
        bytes[2] as f32 / 255.0,
        bytes[3] as f32 / 255.0,
    ]
}

// ════════════════════════════════════════════════════════════════════════════
// The rig camera ECS vehicle (mirrors boyko_scene/tests/orbit_camera.rs)
// ════════════════════════════════════════════════════════════════════════════

/// The rig camera's full spawn bundle (the explicit 5-component list — O2).
#[derive(Bundle)]
struct RigCameraBundle {
    camera: Camera,
    projection: Projection,
    rig: OrbitCamera,
    transform: Transform,
    global: GlobalTransform,
}

/// A FULL-pipeline App with the camera resources + the three rig systems wired
/// `orbit.before(propagate)`, `resolve.after(propagate)` — the production vehicle
/// (cf. `CameraPlugin`). Cloned from `orbit_camera::rig_pipeline_world`.
fn rig_pipeline_world() -> App {
    let mut app = App::new();
    app.insert_resource(ActiveCamera::default());
    app.insert_resource(ViewUniform::default());
    app.add_systems_cfg(|b| {
        let propagate = b.add_system(propagate_transforms).key();
        b.add_system(orbit_camera_system).before(propagate);
        b.add_system(resolve_active_camera).after(propagate);
    });
    app
}

/// Spawns the rig camera at `(yaw, pitch)` and returns its live handle.
fn spawn_rig_camera(world: &mut EcsMaster, yaw: f32, pitch: f32) -> Entity {
    let rig = OrbitCamera::new(TARGET, ORBIT_DISTANCE, yaw, pitch);
    let projection = Projection::Perspective {
        fov_y: FOV_Y,
        aspect: ASPECT,
        near: NEAR,
        far: FAR,
    };
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(RigCameraBundle {
                camera: Camera::DEFAULT,
                projection,
                rig,
                transform: Transform::IDENTITY,
                global: GlobalTransform::IDENTITY,
            })
            .id();
        *probe.lock().expect("probe lock") = Some(e);
    });
    let e = sink.lock().expect("probe lock").expect("spawn handle");
    assert!(world.has_entity(e), "spawned rig camera is live");
    e
}

/// Advances the rig's `yaw` in place (the windowed-example animation shape: a
/// one-shot `Query<&mut OrbitCamera>` mutates the rig field; the next
/// `App::update` re-derives the pose). Mirrors `orbit_camera::set_rig`.
fn advance_yaw(world: &mut EcsMaster, target: Entity, new_yaw: f32) {
    world.run_system(move |mut q: Query<&mut OrbitCamera>| {
        for (id, r) in q.iter_entities_mut() {
            if id == target.id() {
                r.yaw = new_yaw;
            }
        }
    });
}

// ════════════════════════════════════════════════════════════════════════════
// Inlined harness helpers (an example cannot reach the tests/ `common` module)
// ════════════════════════════════════════════════════════════════════════════

/// Boots a validation-enabled WINDOWED context, or returns `None` (with a SKIP
/// log). Inlined from `tests/common::boot_or_skip`, set `windowed: true` (the
/// present path needs the swapchain device extension).
fn boot_windowed_or_skip() -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        windowed: true,
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP orbit_cube_window: windowed Vulkan unavailable ({e:?})");
            None
        }
    }
}

/// Writes `rgba` (`w*h*4` R8G8B8A8) as a dependency-free 32bpp BGRA BMP
/// (top-down, no row flip). Cloned verbatim from
/// `p7b_world_ui_screenshot::write_bmp`.
fn write_bmp(path: &Path, rgba: &[u8], w: u32, h: u32) -> std::io::Result<()> {
    debug_assert_eq!(rgba.len(), (w * h * 4) as usize, "invariant: BMP body is w*h*4 bytes");
    let pixel_bytes = w * h * 4;
    let pixel_offset: u32 = 54;
    let file_size = pixel_offset + pixel_bytes;

    let mut buf = Vec::with_capacity(file_size as usize);
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&file_size.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&pixel_offset.to_le_bytes());
    buf.extend_from_slice(&40u32.to_le_bytes());
    buf.extend_from_slice(&(w as i32).to_le_bytes());
    buf.extend_from_slice(&(-(h as i32)).to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&32u16.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&pixel_bytes.to_le_bytes());
    buf.extend_from_slice(&0i32.to_le_bytes());
    buf.extend_from_slice(&0i32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    for px in rgba.chunks_exact(4) {
        buf.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &buf)
}

/// The readback BMP output path under the workspace target dir.
fn screenshot_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("target")
        .join("screenshots")
        .join("s35_orbit_window.bmp")
}

// ════════════════════════════════════════════════════════════════════════════
// main — the live windowed orbit
// ════════════════════════════════════════════════════════════════════════════

fn main() {
    // --- Open the window first (the surface borrows its HWND/HINSTANCE). ---
    let mut window = match Window::open("boyko_render orbit cube", WIDTH, HEIGHT) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("SKIP orbit_cube_window: cannot open a window ({e:?})");
            return;
        }
    };

    let Some(ctx) = boot_windowed_or_skip() else {
        return;
    };
    println!("Vulkan device (windowed, validation on): {}", ctx.device_name());

    // SAFETY: `window` outlives the surface (dropped after it below); its
    // HWND/HINSTANCE are live for the surface's lifetime.
    let surface = match unsafe { Surface::new(&ctx, window.hinstance(), window.hwnd()) } {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP orbit_cube_window: surface creation failed ({e:?})");
            return;
        }
    };

    let mut swapchain = match Swapchain::new(&ctx, &surface, window.width(), window.height()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP orbit_cube_window: swapchain creation failed ({e:?})");
            return;
        }
    };

    let Some(color_format) = swapchain_format_to_rhi(swapchain.format()) else {
        eprintln!(
            "SKIP orbit_cube_window: swapchain format {} has no basic-slice Format variant",
            swapchain.format()
        );
        return;
    };

    let mut renderer =
        Renderer::new(&ctx, &surface, &swapchain).expect("renderer (command pool + sync) creation");

    // --- Build the cube vertex buffer (host-visible, written once). ---
    let vertices = cube_vertices();
    let vertex_bytes = core::mem::size_of_val(&vertices) as u64;
    let vertex_buffer = RhiDevice::create_buffer(
        &ctx,
        &BufferDesc {
            size: vertex_bytes,
            usage: BufferUsage::VERTEX,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible vertex buffer");
    let vb_ptr =
        RhiDevice::buffer_mapped_ptr(&ctx, &vertex_buffer).expect("host-visible vertex buffer mapped");
    // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes (the
    // buffer was created at exactly that size); `vertices` is a distinct,
    // non-overlapping stack array of `vertex_bytes` bytes; the write completes
    // before any submit references the buffer (host-coherent: no flush needed).
    unsafe {
        core::ptr::copy_nonoverlapping(
            vertices.as_ptr().cast::<u8>(),
            vb_ptr.as_ptr(),
            vertex_bytes as usize,
        );
    }

    let vs = RhiDevice::create_shader_module(&ctx, MVP_VS_SPV.as_words()).expect("vertex shader");
    let fs = RhiDevice::create_shader_module(&ctx, MVP_FS_SPV.as_words()).expect("fragment shader");

    let attributes = [
        VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 1, offset: 12, format: VertexFormat::Float32x4 },
    ];
    let pipeline = RhiDevice::create_graphics_pipeline(
        &ctx,
        &GraphicsPipelineDesc {
            vertex_module: &vs,
            vertex_entry: c"main",
            fragment_module: &fs,
            fragment_entry: c"main",
            color_formats: &[color_format],
            depth_format: Some(Format::D32Sfloat),
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: Some(VertexBufferLayout {
                stride: VERTEX_STRIDE,
                attributes: &attributes,
            }),
            push_constant_bytes: SCENE_MVP_BYTES as u32,
            bind_group_layout: None,
            blend: None,
        },
    )
    .expect("depth-testing graphics pipeline (swapchain color format)");

    // The shader modules are consumed by pipeline creation; destroy them now.
    // SAFETY: both modules were created on `ctx` above and are no longer needed
    // once the pipeline is created (the pipeline holds its own compiled stages);
    // each is destroyed exactly once.
    unsafe {
        RhiDevice::destroy_shader_module(&ctx, fs);
        RhiDevice::destroy_shader_module(&ctx, vs);
    }

    // --- Build the ECS rig App + the initial MVP from frame-0's view. ---
    let mut app = rig_pipeline_world();
    let cam = spawn_rig_camera(app.world_mut(), 0.0, ORBIT_PITCH);
    app.update_with_delta(FIXED_DELTA); // frame 0 → resolve the initial ViewUniform.
    let initial_mvp = mvp_bytes_from_view(app.world().resource::<ViewUniform>());

    let mut scene = Scene::new(pipeline, vertex_buffer, 36, initial_mvp);

    // --- A host-visible staging buffer for the one readback frame. ---
    let staging_size = (swapchain.extent().width * swapchain.extent().height * 4) as u64;
    let staging = RhiDevice::create_buffer(
        &ctx,
        &BufferDesc {
            size: staging_size,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible readback staging buffer");
    let alloc_extent = swapchain.extent();

    // --- The orbit loop: advance yaw → App::update → set_mvp → render+present. ---
    let clear = floats(CLEAR_BYTES);
    let mut readback_done = false;
    let mut readback_extent = swapchain.extent();
    for frame_i in 0..TOTAL_FRAMES {
        window.pump_events();
        window.refresh_size();

        // Advance the rig and re-resolve the view for THIS frame.
        let yaw = frame_i as f32 * YAW_STEP;
        advance_yaw(app.world_mut(), cam, yaw);
        app.update_with_delta(FIXED_DELTA);

        // Bridge the freshly-resolved view → the 64-byte column-major MVP.
        let mvp = mvp_bytes_from_view(app.world().resource::<ViewUniform>());
        scene.set_mvp(mvp);

        // Request the readback on the KNOWN-angle frame, only while the live extent
        // still matches the staging-buffer size (a resize simply skips the capture;
        // the present still runs).
        let live = swapchain.extent();
        let extent_stable = live.width == alloc_extent.width && live.height == alloc_extent.height;
        let want_readback = frame_i == READBACK_FRAME && !readback_done && extent_stable;
        let rb = if want_readback { Some(&staging) } else { None };

        // SAFETY: `ctx`/`surface`/`swapchain`/`scene` are live and created on the
        // same device as `renderer`; a `Some(rb)` staging buffer is host-visible and
        // `staging_size` (>= one swapchain image) bytes; the renderer syncs the
        // scene's depth image to the swapchain extent internally and re-pushes
        // `scene.mvp` (just set) to the vertex stage this frame.
        let presented = unsafe {
            renderer.render_scene_frame(
                &ctx,
                &surface,
                &mut swapchain,
                &mut scene,
                window.width(),
                window.height(),
                clear,
                rb,
            )
        }
        .unwrap_or_else(|e| panic!("scene frame {frame_i} failed: {e:?}"));

        if want_readback && presented {
            readback_done = true;
            readback_extent = swapchain.extent();
        }
    }

    // --- Write the known-angle readback BMP (if a capture frame presented). ---
    if readback_done {
        let w = readback_extent.width;
        let h = readback_extent.height;
        let dst_ptr =
            RhiDevice::buffer_mapped_ptr(&ctx, &staging).expect("host-visible staging buffer mapped");
        let byte_count = (w * h * 4) as usize;
        let mut out = vec![0u8; byte_count];
        // SAFETY: `dst_ptr` points to `staging_size` (>= `byte_count`) mapped
        // host-coherent bytes; the readback frame's submit completed before this read
        // (the renderer fence-waits each frame slot at the START of the NEXT
        // `render_scene_frame`, and more frames followed the capture frame, so its
        // copy is complete + coherent); `out` is a distinct, non-overlapping alloc.
        unsafe {
            core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), byte_count);
        }
        // The swapchain image is commonly B8G8R8A8 (the readback bytes are then
        // B,G,R,A), but `write_bmp` expects R,G,B,A. Swap R/B for a BGRA swapchain so
        // the BMP matches the colors the LIVE window already presented correctly
        // (the GPU honors the image format on present; only this raw-byte readback
        // needs the channel fix). `window_present_scene` sidesteps this by using
        // R==B colors; the cube's distinct red/blue faces expose the order.
        let fmt = swapchain.format();
        if fmt == VK_FORMAT_B8G8R8A8_UNORM || fmt == VK_FORMAT_B8G8R8A8_SRGB {
            for px in out.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
        let path = screenshot_path();
        write_bmp(&path, &out, w, h).expect("write the orbit-cube readback BMP");
        let abs = std::fs::canonicalize(&path).unwrap_or(path);
        let known_yaw = READBACK_FRAME as f32 * YAW_STEP;
        println!(
            "orbited {TOTAL_FRAMES} frames, wrote {} (readback at yaw={known_yaw:.3} rad, pitch={ORBIT_PITCH:.3})",
            abs.display()
        );
    } else {
        println!("orbited {TOTAL_FRAMES} frames (no readback frame presented — swapchain kept recreating)");
    }

    // --- Validation oracle: a clean orbit records zero validation messages. ---
    if let Some(state) = ctx.debug_state()
        && state.total() != 0
    {
        eprintln!(
            "WARNING: validation reported {} message(s) during the orbit — see [vk-validation]",
            state.total()
        );
    }

    // --- Clean reverse-order teardown (renderer waits idle → scene → staging →
    //     swapchain → surface → window). ---
    drop(renderer);
    // SAFETY: the renderer was dropped above (its `Drop` waits the device idle), so
    // no submission references the scene's resources; `ctx` is still alive; each
    // scene resource + the staging buffer is destroyed exactly once.
    unsafe {
        scene.destroy(&ctx);
        RhiDevice::destroy_buffer(&ctx, staging);
    }
    drop(swapchain);
    drop(surface);
    drop(ctx);
    drop(window);
}
