//! S35 GPU PROOF (a) — the OrbitCamera RIG drives the on-screen SDF view (RTX
//! 3060 oracle), plus the non-ignored CPU setup asserts.
//!
//! This is the offscreen counterpart of the live windowed example
//! (`examples/orbit_cube_window.rs`): the SAME `OrbitCamera` →
//! `orbit_camera_system` → `propagate_transforms` → `resolve_active_camera` →
//! `ViewUniform` chain drives the marcher's [`CompositePushConstants`] (via
//! `composite_perspective_from_view`), and the SAME asymmetric SDF box (with a
//! marker sphere fused to one corner) is rendered at FOUR yaw angles
//! (0° / 45° / 90° / 135°) into ONE left→right STRIP, so the owner eyeballs a
//! single continuous orbit: the box's silhouette + visible faces change and the
//! corner marker sweeps across as the camera circles it. A box (not a sphere) is
//! the subject precisely because a sphere looks identical from every angle and so
//! cannot show a rotation.
//!
//! # The driving contract (BUG-S35-1)
//!
//! The rig writes the camera's local `Transform` through `Mut<Transform>`
//! (change-tracked); `propagate_transforms` is dirty-gated on that row's
//! `changed_tick`. For the just-written pose to recompose `GlobalTransform` the
//! SAME frame, the three systems run under a real `Schedule` / `App::update`
//! (whose `Schedule::run` promotes the change ticks at frame start) — NOT a bare
//! `run_system` chain. This mirrors `boyko_scene/tests/orbit_camera.rs`'s
//! `rig_pipeline_world` + `frame` driver verbatim (the load-bearing correctness
//! path), per pose.
//!
//! # Why this lives in `boyko_render/tests`
//!
//! `boyko_render` is the only crate that may name BOTH the scene vocabulary
//! (`boyko_scene`: the camera systems → `ViewUniform`) AND the marcher push
//! constants (`boyko_rhi_vulkan`). The marcher buffer-layout helpers
//! (`seed_buffer` / `read_pixels` / `run_marcher` / `unpack_rgb`) are mirrored
//! VERBATIM from `camera_drives_render_gpu.rs`; the no-dep BMP writer
//! (`write_bmp`) from `p7b_world_ui_screenshot.rs`.
//!
//! # Split: CPU setup asserts run in-workflow; the GPU screenshot is `#[ignore]`d
//!
//! The non-ignored `s35_orbit_*_setup` tests drive ONLY the ECS systems on the
//! CPU (no device) and assert the rig drove the pose (BUG-S35-1 guard), looks at
//! the target, and that the oblique view differs materially from the head-on one
//! — so a mis-wired rig fails fast without a GPU boot. The screenshot test is
//! `#[ignore]`d inside `mod gpu` (Vulkan boot can hang a headless run); the
//! orchestrator runs it on the RTX.
//!
//! The owner-run command (one line, RTX 3060):
//!
//! ```text
//! cargo test -p boyko-render --test orbit_camera_drives_render_gpu \
//!   s35_orbit_screenshot -- --ignored --test-threads=1 --nocapture
//! ```
//!
//! Output image: `D:\claude\BoykoEngine\target\screenshots\s35_orbit.bmp`

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Bundle;

use boyko_math::{Vec3, Vec4};

use boyko_scene::{
    ActiveCamera, Camera, GlobalTransform, OrbitCamera, Projection, Transform, ViewUniform,
    orbit_camera_system, propagate_transforms, resolve_active_camera,
};

use core::f32::consts::FRAC_PI_4;

// ════════════════════════════════════════════════════════════════════════════
// Fixed proof-image geometry (the perspective rig + the recognizable SDF scene)
// ════════════════════════════════════════════════════════════════════════════

/// One pose's render width (the composite is `2 * W` wide — pose A | pose B).
const W: u32 = 256;
/// Render height.
const H: u32 = 256;
/// Camera aspect (square per-pose tile so NDC x/y stay symmetric).
const ASPECT: f32 = W as f32 / H as f32;
/// Vertical FOV (45°).
const FOV_Y: f32 = FRAC_PI_4;
const NEAR: f32 = 0.1;
const FAR: f32 = 100.0;

/// The orbit radius — frames the box + corner marker comfortably at 45° FOV across
/// every strip pose.
const ORBIT_DISTANCE: f32 = 6.0;

/// The orbit/look-at target: the box sits at the origin, so the rig keeps it
/// centered as it sweeps around.
const TARGET: [f32; 3] = [0.0, 0.0, 0.0];

/// The hero primitive: an ASYMMETRIC box (a distinct half-extent per axis) centered
/// at the origin. Unlike a sphere — which looks identical from every angle and so
/// cannot show an orbit — a box's silhouette + visible faces change as the camera
/// sweeps, making the rotation legible.
const BOX_HALF: [f32; 3] = [1.4, 0.7, 1.0];

/// A small marker sphere SMOOTH-fused to one corner of the box (the `+X+Y+Z`
/// corner) — a recognizable feature to track around the orbit, removing any
/// orientation ambiguity.
const MARKER_POS: [f32; 3] = [1.4, 0.7, 1.0];
const MARKER_R: f32 = 0.55;
/// Smooth-union blend radius fusing the marker into the box (a soft corner bump,
/// not a detached ball).
const MARKER_SMOOTH: f32 = 0.45;

/// A fixed mild downward tilt for every strip pose (≈10°): enough to read the top
/// face (depth cue) while keeping world-vertical edges very nearly parallel (no
/// harsh keystoning).
const STRIP_PITCH: f32 = 0.18;

/// The orbit STRIP: FOUR yaw angles (0°, 45°, 90°, 135°) at the fixed pitch,
/// rendered left→right so the continuous rotation of the SAME box reads
/// unmistakably as a single camera orbit (not several different scenes).
const POSES: [(f32, f32); 4] = [
    (0.0, STRIP_PITCH),
    (core::f32::consts::FRAC_PI_4, STRIP_PITCH),
    (core::f32::consts::FRAC_PI_2, STRIP_PITCH),
    (3.0 * core::f32::consts::FRAC_PI_4, STRIP_PITCH),
];

/// The head-on pose the CPU asserts use (strip column 0).
const POSE_A: (f32, f32) = POSES[0];
/// The most-oblique pose the CPU asserts use (strip column 3).
const POSE_B: (f32, f32) = POSES[3];

/// Element-wise float tolerance for the CPU setup asserts.
const EPS: f32 = 1.0e-3;

/// A fixed per-update delta (keeps `Instant::now` jitter out of the frame
/// driver — the established timed-vehicle discipline).
const FIXED_DELTA: Duration = Duration::from_millis(16);

// ════════════════════════════════════════════════════════════════════════════
// The rig camera ECS vehicle (mirrors boyko_scene/tests/orbit_camera.rs)
// ════════════════════════════════════════════════════════════════════════════

/// The rig camera's full spawn bundle: the EXPLICIT 5-component list (O2) —
/// selection metadata, projection, the rig, and BOTH pose columns
/// (`propagate_transforms`'s archetype gate needs both present).
#[derive(Bundle)]
struct RigCameraBundle {
    camera: Camera,
    projection: Projection,
    rig: OrbitCamera,
    transform: Transform,
    global: GlobalTransform,
}

/// A perspective projection matching the proof-image aspect / FOV.
fn perspective() -> Projection {
    Projection::Perspective {
        fov_y: FOV_Y,
        aspect: ASPECT,
        near: NEAR,
        far: FAR,
    }
}

/// A FULL-pipeline App: the camera resources seeded AND the three rig systems
/// registered into the `Main` schedule with the §8 ordering edges
/// (`orbit_camera_system.before(propagate)`,
/// `resolve_active_camera.after(propagate)`). `App::update` runs one frame at a
/// single promoted world tick — the production vehicle (cf. `CameraPlugin`).
/// Cloned from `orbit_camera::rig_pipeline_world`. NOT finished (the caller
/// spawns first, then `update`).
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

/// Advances the App one full frame (`Schedule::run`: bump the world tick, run
/// all registered systems in order at the promoted tick — the BUG-S35-1
/// correctness path).
#[inline]
fn frame(app: &mut App) {
    app.update_with_delta(FIXED_DELTA);
}

/// Spawns a rig camera at `(yaw, pitch)` and returns its live handle.
fn spawn_rig_camera(world: &mut EcsMaster, yaw: f32, pitch: f32) -> Entity {
    let rig = OrbitCamera::new(TARGET, ORBIT_DISTANCE, yaw, pitch);
    let sink: Arc<Mutex<Option<Entity>>> = Arc::new(Mutex::new(None));
    let probe = Arc::clone(&sink);
    world.run_system(move |mut cmds: Commands| {
        let e = cmds
            .spawn(RigCameraBundle {
                camera: Camera::DEFAULT,
                projection: perspective(),
                rig,
                transform: Transform::IDENTITY,
                global: GlobalTransform::IDENTITY,
            })
            .id();
        *probe.lock().expect("probe lock") = Some(e);
    });
    let e = sink.lock().expect("probe lock").expect("spawn produced a handle");
    assert!(world.has_entity(e), "spawned rig camera is live after the apply window");
    e
}

/// Builds a fresh pipeline App, spawns the rig at `(yaw, pitch)`, runs ONE frame,
/// and returns `(App, camera_entity, resolved ViewUniform)`. A fresh world per
/// pose keeps the two renders fully independent (no cross-pose state).
fn resolve_pose(yaw: f32, pitch: f32) -> (App, Entity, ViewUniform) {
    let mut app = rig_pipeline_world();
    let e = spawn_rig_camera(app.world_mut(), yaw, pitch);
    frame(&mut app);
    let view = *app.world().resource::<ViewUniform>();
    (app, e, view)
}

/// Reads the entity's propagated world translation (the orbit eye).
fn eye_of(app: &App, e: Entity) -> Vec3 {
    app.world()
        .get_component::<GlobalTransform>(e)
        .expect("camera has GlobalTransform")
        .affine()
        .translation
}

// ════════════════════════════════════════════════════════════════════════════
// Geometry helpers (the analytic orbit eye + NDC projection check)
// ════════════════════════════════════════════════════════════════════════════

/// The analytic orbit eye for `(yaw, pitch)` at `ORBIT_DISTANCE` — the §6.3
/// formula `eye = target + dist * (cp*sy, sp, cp*cy)`. The CPU setup assert
/// compares the rig-DRIVEN `GlobalTransform.translation` against this, proving
/// the rig (not a stale identity pose) drove the camera (BUG-S35-1).
fn analytic_eye(yaw: f32, pitch: f32) -> Vec3 {
    let (sp, cp) = pitch.sin_cos();
    let (sy, cy) = yaw.sin_cos();
    let t = Vec3::new(TARGET[0], TARGET[1], TARGET[2]);
    t + Vec3::new(ORBIT_DISTANCE * cp * sy, ORBIT_DISTANCE * sp, ORBIT_DISTANCE * cp * cy)
}

#[track_caller]
fn approx(a: f32, b: f32, what: &str) {
    assert!((a - b).abs() <= EPS, "{what}: expected {b}, got {a} (|Δ|={})", (a - b).abs());
}

#[track_caller]
fn vec3_approx(a: Vec3, b: Vec3, what: &str) {
    assert!(
        (a.x - b.x).abs() <= EPS && (a.y - b.y).abs() <= EPS && (a.z - b.z).abs() <= EPS,
        "{what}: expected {b:?}, got {a:?}"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// No-dep BMP writer (cloned verbatim from p7b_world_ui_screenshot::write_bmp)
// ════════════════════════════════════════════════════════════════════════════

/// Writes `rgba` (`w*h*4` tightly-packed R8G8B8A8) as a dependency-free 32bpp
/// BGRA BMP. Top-down via a NEGATIVE `biHeight`, so the in-memory top-left texel
/// is the image top-left — NO row flip. The single channel swap (RGBA → BGRA) is
/// here ONLY. Cloned verbatim from `p7b_world_ui_screenshot::write_bmp`.
fn write_bmp(path: &Path, rgba: &[u8], w: u32, h: u32) -> std::io::Result<()> {
    debug_assert_eq!(rgba.len(), (w * h * 4) as usize, "invariant: BMP body is w*h*4 bytes");
    let pixel_bytes = w * h * 4;
    let pixel_offset: u32 = 54; // 14-byte file header + 40-byte info header.
    let file_size = pixel_offset + pixel_bytes;

    let mut buf = Vec::with_capacity(file_size as usize);
    // --- BITMAPFILEHEADER (14 bytes) ---
    buf.extend_from_slice(b"BM");
    buf.extend_from_slice(&file_size.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes()); // reserved1
    buf.extend_from_slice(&0u16.to_le_bytes()); // reserved2
    buf.extend_from_slice(&pixel_offset.to_le_bytes());
    // --- BITMAPINFOHEADER (40 bytes) ---
    buf.extend_from_slice(&40u32.to_le_bytes()); // biSize
    buf.extend_from_slice(&(w as i32).to_le_bytes()); // biWidth
    buf.extend_from_slice(&(-(h as i32)).to_le_bytes()); // biHeight (negative => top-down)
    buf.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    buf.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    buf.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    buf.extend_from_slice(&pixel_bytes.to_le_bytes()); // biSizeImage
    buf.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    buf.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    buf.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    buf.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant
    // --- pixel data: RGBA -> BGRA (the ONLY channel swap; no row flip) ---
    for px in rgba.chunks_exact(4) {
        buf.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &buf)
}

/// The screenshot output path under the workspace target dir.
fn screenshot_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("target")
        .join("screenshots")
        .join("s35_orbit.bmp")
}

// ════════════════════════════════════════════════════════════════════════════
// CPU setup-validation tests (run in-workflow; NOT ignored — no GPU device)
// ════════════════════════════════════════════════════════════════════════════

/// The rig DROVE the camera pose: after one `App::update` at the OBLIQUE pose the
/// camera's `GlobalTransform.translation` equals the analytic orbit eye (NOT the
/// spawn-identity origin). This is the BUG-S35-1 guard: a rig that wrote
/// `Transform` without stamping the change tick would leave `GlobalTransform`
/// stuck at identity and the eye at the origin.
#[test]
fn s35_orbit_rig_drives_pose_setup() {
    let (yaw, pitch) = POSE_B;
    let (app, e, _view) = resolve_pose(yaw, pitch);
    vec3_approx(
        eye_of(&app, e),
        analytic_eye(yaw, pitch),
        "oblique pose: rig-driven GlobalTransform.translation == the analytic orbit eye",
    );
    // Anti-vacuity: the driven eye is NOT the spawn-identity origin (the failure
    // mode BUG-S35-1 produced).
    assert!(
        eye_of(&app, e).length() > 1.0,
        "the rig moved the eye off the origin (not the stale identity pose)"
    );
}

/// The rig LOOKS AT the target: at the oblique pose the resolved
/// `ViewUniform.view_proj` projects `TARGET` to NDC ≈ origin (screen center).
#[test]
fn s35_orbit_looks_at_target_setup() {
    let (yaw, pitch) = POSE_B;
    let (_app, _e, view) = resolve_pose(yaw, pitch);
    let target = Vec4::from_vec3(Vec3::new(TARGET[0], TARGET[1], TARGET[2]), 1.0);
    let clip = view.view_proj.mul_vec4(target);
    assert!(clip.w.abs() > EPS, "target in front of camera (w != 0): w={}", clip.w);
    approx(clip.x / clip.w, 0.0, "oblique pose: target NDC x ≈ 0 (looks at target)");
    approx(clip.y / clip.w, 0.0, "oblique pose: target NDC y ≈ 0 (looks at target)");
}

/// The rig CHANGED the view: the oblique `view_proj` differs materially from the
/// head-on one (the orbit actually rotated what the camera sees — the premise of
/// the GPU screenshot, guarded on CPU).
#[test]
fn s35_orbit_oblique_differs_from_head_on_setup() {
    let (_app_a, _ea, view_a) = resolve_pose(POSE_A.0, POSE_A.1);
    let (_app_b, _eb, view_b) = resolve_pose(POSE_B.0, POSE_B.1);

    // The eyes moved apart (the orbit swept around the target).
    let eye_a = view_a.camera_pos;
    let eye_b = view_b.camera_pos;
    let eye_shift = ((eye_a.x - eye_b.x).powi(2)
        + (eye_a.y - eye_b.y).powi(2)
        + (eye_a.z - eye_b.z).powi(2))
    .sqrt();
    assert!(eye_shift > 1.0, "the orbit moved the eye between poses (|Δeye|={eye_shift})");

    // The view matrices differ materially (a basis-only no-op would leave them
    // equal). Compare the max per-element |Δ| across the 4 columns.
    let mut max_delta = 0.0f32;
    for j in 0..4 {
        let a = view_a.view_proj.cols[j];
        let b = view_b.view_proj.cols[j];
        for d in [a.x - b.x, a.y - b.y, a.z - b.z, a.w - b.w] {
            max_delta = max_delta.max(d.abs());
        }
    }
    assert!(max_delta > 0.1, "the rig rotated the view (max |Δ view_proj| = {max_delta})");
}

// ════════════════════════════════════════════════════════════════════════════
// The GPU screenshot test (#[ignore]) — owner-run on the RTX
// ════════════════════════════════════════════════════════════════════════════

#[cfg(not(miri))]
mod gpu {
    use super::*;

    use core::ptr::NonNull;

    use boyko_rhi::{
        BufferDesc, BufferUsage, ComputePipelineDesc, MemoryLocation, RhiCommandEncoder, RhiDevice,
        RhiQueue, ShaderStage,
    };
    use boyko_rhi_vulkan::compute::{
        COMPOSITE_DEPTH_BASE_WORDS, COMPOSITE_PUSH_CONSTANT_BYTES, CompositePushConstants,
        LOCAL_SIZE_X, MESH_DEPTH_CLEAR, SdfEdit, sdf_depth_composite_spirv, sdf_op,
    };
    use boyko_rhi_vulkan::device::VulkanContext;

    use boyko_render::composite_perspective_from_view;

    use common::{assert_validation_clean, boot_or_skip};

    // ── Marcher buffer layout (mirrors camera_drives_render_gpu.rs verbatim) ──

    const DEPTH_BASE: usize = 196;
    const _: () = assert!(DEPTH_BASE == COMPOSITE_DEPTH_BASE_WORDS);

    fn buffer_words(w: u32, h: u32) -> usize {
        DEPTH_BASE + 2 * (w as usize) * (h as usize)
    }

    fn pixel_base_words(w: u32, h: u32) -> usize {
        DEPTH_BASE + (w as usize) * (h as usize)
    }

    fn group_count(w: u32, h: u32) -> u32 {
        ((w as u64 * h as u64) as u32).div_ceil(LOCAL_SIZE_X)
    }

    /// The hero scene: an asymmetric box with a marker sphere SMOOTH-unioned to its
    /// `+X+Y+Z` corner, folded into one edit list. The box seeds the field (hard);
    /// the marker unions with a `MARKER_SMOOTH` blend (a soft corner bump that makes
    /// the orientation trackable as the camera orbits).
    fn box_scene() -> Vec<SdfEdit> {
        vec![
            SdfEdit::box_shape(TARGET, BOX_HALF, sdf_op::UNION, 0.0),
            SdfEdit::sphere(MARKER_POS, MARKER_R, sdf_op::UNION, MARKER_SMOOTH),
        ]
    }

    fn seed_buffer(base: NonNull<u8>, edits: &[SdfEdit], w: u32, h: u32) {
        let dst = base.as_ptr().cast::<u32>();
        let n_pixels = (w as usize) * (h as usize);
        // SAFETY: `dst` is the start of a `buffer_words(w,h)*4`-byte host-coherent
        // mapping (the buffer was created at exactly that size); every index written
        // is < that word count. No GPU work is in flight (submit happens after);
        // `write_unaligned` tolerates the sub-allocated offset.
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
            // SAFETY: `pbase + i` for `i < n` is the pixel region, in-bounds; a fence
            // wait preceded this read so GPU writes are complete + coherent.
            out.push(unsafe { p.add(pbase + i).read_unaligned() });
        }
        out
    }

    /// Records + submits ONE compute-only marcher dispatch driven by `pc`,
    /// fence-waits, asserts validation-clean, and returns the readback pixels.
    /// Cloned from `camera_drives_render_gpu::run_marcher`.
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

        // SAFETY: every resource was created on `device` and is destroyed exactly
        // once; the submission completed (fence-waited above), so none is GPU-in-use.
        unsafe {
            device.destroy_command_encoder(encoder);
            device.destroy_fence(fence);
            device.destroy_compute_pipeline(compute);
            device.destroy_shader_module(cs);
            device.destroy_buffer(buffer);
        }

        pixels
    }

    /// Unpacks a marcher pixel `(0x00BBGGRR)` to `[r, g, b]` (0..=255). Cloned
    /// from `camera_drives_render_gpu::unpack_rgb` (u8 lanes for the composite).
    fn unpack_rgb(packed: u32) -> [u8; 3] {
        [(packed & 0xFF) as u8, ((packed >> 8) & 0xFF) as u8, ((packed >> 16) & 0xFF) as u8]
    }

    /// Blits a `W*H` RGBA tile into the `2*W`-wide composite at column offset
    /// `x_off` (one pose per tile).
    fn blit_tile(dst: &mut [u8], dst_w: u32, x_off: u32, tile_rgb: &[u32]) {
        for py in 0..H {
            for px in 0..W {
                let rgb = unpack_rgb(tile_rgb[(py * W + px) as usize]);
                let di = (((py * dst_w) + x_off + px) * 4) as usize;
                dst[di] = rgb[0];
                dst[di + 1] = rgb[1];
                dst[di + 2] = rgb[2];
                dst[di + 3] = 255;
            }
        }
    }

    /// Renders the SDF scene at one orbit pose and returns the packed `W*H` tile.
    /// Drives the REAL rig → propagate → resolve chain through `App::update`
    /// (BUG-S35-1 path), then bridges the resolved view to the marcher.
    fn render_pose(ctx: &VulkanContext, yaw: f32, pitch: f32, label: &str) -> Vec<u32> {
        let (_app, _e, view) = resolve_pose(yaw, pitch);
        let pc = composite_perspective_from_view(&view, W, H);
        let edits = box_scene();
        run_marcher(ctx, &edits, pc, W, H, label)
    }

    /// The owner-eval screenshot: drives the rig at TWO orbit poses (head-on |
    /// oblique), renders each SDF tile through the live `ViewUniform`, composites
    /// them side-by-side, asserts the two tiles DIFFER (the orbit moved the view),
    /// and writes the BMP. `#[ignore]`d — Vulkan boot can hang a headless run; the
    /// orchestrator runs it on the RTX (see the module header).
    #[test]
    #[ignore = "boots Vulkan on the GPU; owner-run on the RTX (see module header)"]
    fn s35_orbit_screenshot() {
        let Some(ctx) = boot_or_skip("s35_orbit_screenshot") else {
            return;
        };
        println!("Vulkan device (validation on): {}", ctx.device_name());
        if !ctx.validation_enabled() {
        // The box-level BOYKO_DISABLE_VALIDATION escape hatch (the validation layer is
        // crash-prone on some machines) removes the layer this gate exists to exercise -
        // SKIP, mirroring the no-device SKIP convention, instead of failing the suite.
        assert!(
            std::env::var_os("BOYKO_DISABLE_VALIDATION").is_some(),
            "validation must be active when enable_validation is set and the escape hatch is absent"
        );
        eprintln!("SKIP: validation disabled (BOYKO_DISABLE_VALIDATION)");
        return;
    }

        // FOUR strip poses → four tiles. Each drives the FULL rig pipeline for ITS
        // pose (the same box, the camera swept to a new yaw).
        let tiles: Vec<Vec<u32>> = POSES
            .iter()
            .enumerate()
            .map(|(i, &(yaw, pitch))| render_pose(&ctx, yaw, pitch, &format!("pose{i}")))
            .collect();

        // The orbit changed the render: the first and last columns differ in a
        // non-trivial pixel count (a rig that did not drive the view would render
        // identical tiles).
        let diff = tiles[0].iter().zip(tiles[POSES.len() - 1].iter()).filter(|(a, b)| a != b).count();
        println!("s35_orbit strip: {diff} px differ between column 0 and column {}", POSES.len() - 1);
        assert!(
            diff > (W as usize * H as usize) / 100,
            "the orbit must change the render (only {diff} px differed)"
        );

        // Composite the four tiles left→right (the orbit strip: yaw 0° | 45° | 90° | 135°).
        let comp_w = POSES.len() as u32 * W;
        let mut composite = vec![0u8; (comp_w * H * 4) as usize];
        for (i, tile) in tiles.iter().enumerate() {
            blit_tile(&mut composite, comp_w, i as u32 * W, tile);
        }

        let path = screenshot_path();
        write_bmp(&path, &composite, comp_w, H).expect("write the S35 orbit strip BMP");
        let abs = std::fs::canonicalize(&path).unwrap_or(path);
        println!("S35 orbit strip written: {} ({}x{})", abs.display(), comp_w, H);
    }
}
