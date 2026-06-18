//! P0a acceptance: runtime-extent + additive-perspective marcher on the RTX 3060
//! oracle (the render-plan PHASE P0 "Gate", gates G3/G4/G5).
//!
//! These fixtures exercise the P0a part-1 (resolution-as-dispatch-dim) and part-2
//! (additive perspective ray-gen) paths of `sdf_depth_composite.hlsl` END-TO-END on
//! real Vulkan — a COMPUTE-ONLY dispatch (no mesh raster / no depth-image copy: the
//! depth region is host-seeded to the far-plane clear so the marcher runs unbounded).
//!
//! - **G3** — a 1920×1080 AND a 1280×720 perspective dispatch are validation- AND
//!   sync-validation-clean, the dispatch completes, and the readback is FINITE (no
//!   NaN/inf) where the camera sees geometry. The storage buffer is sized to the
//!   runtime extent (`DEPTH_BASE + 2*W*H` words ≈ 16.6 MB at 1080p), NOT the 64×64
//!   static buffer.
//! - **G4** — at a small extent (64×64 and 128×128) a perspective dispatch is diffed
//!   per-pixel against [`golden_composite_pixel_ex`] with `CompositeCamera::Perspective`
//!   within ±2/255 — the host mirror (the M1 raw-divide `normalize`) predicts the GPU
//!   on the perspective path.
//! - **G5** — GPU marcher wall-time baselines (a fence-bracketed submit; the RHI
//!   exposes NO timestamp-query pool, so wall-clock around the serialized single-
//!   command-buffer submit is the available oracle — see [`time_dispatch`]) for a
//!   SPARSE (clustered geometry, ~70% empty) and a DENSE (geometry fills frame) scene
//!   at 720p and 1080p, perspective camera. The numbers later phases cite.
//!
//! # The shared one-binding buffer layout (mirrors the shader at runtime extent)
//!
//!   word 0                        : uint edit_count
//!   words [HEADER_BASE=4 ..]      : MAX_SDF_EDITS * SdfEdit (the std430 array)
//!   words [DEPTH_BASE=196 ..]     : W*H f32 mesh depth (host-seeded to CLEAR=1.0)
//!   words [PIXEL_BASE=196+W*H ..] : W*H u32 packed-RGBA output
//!
//! `DEPTH_BASE` is the shader's fixed `HEADER_BASE + MAX_SDF_EDITS*SDF_EDIT_WORDS`
//! (= 196); `pixel_base()` scales with the runtime extent, so the buffer is
//! `DEPTH_BASE + 2*W*H` words. The depth region is seeded to `MESH_DEPTH_CLEAR`
//! (1.0) — leaving it zero would decode as "mesh at t=0" and clip every march.
//!
//! # The oracle
//!
//! Boots with validation enabled (`enable_validation: true`), asserts
//! `debug_state().total() == 0` after each run, and (for G3) confirms the readback is
//! finite. A GPU-less / loader-less host makes `VulkanContext::boot` return `Err`; the
//! test SKIPS with a log — and every non-skip path asserts the body actually ran (the
//! device name is printed and pixels are read back), so a skip is never a silent pass.

use core::ptr::NonNull;
use std::time::Instant;

use boyko_rhi::{
    BufferDesc, BufferUsage, ComputePipelineDesc, MemoryLocation, RhiCommandEncoder, RhiDevice,
    RhiQueue, ShaderStage,
};
use boyko_rhi_vulkan::compute::{
    CAM_MODE_PERSPECTIVE, COMPOSITE_DEPTH_BASE_WORDS, COMPOSITE_PUSH_CONSTANT_BYTES,
    CompositeCamera, CompositePushConstants, LOCAL_SIZE_X, MESH_DEPTH_CLEAR, SdfEdit,
    golden_composite_pixel_ex, sdf_depth_composite_spirv, sdf_op,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// Per-channel tolerance on the packed-RGBA bytes (the rung-8..11 ±2/255 contract):
/// DXC `mad`/`fma` rounding makes a bit-exact match brittle; ±2/255 still proves the
/// lit / mesh / background colors apart (they differ by 100+).
const CHANNEL_TOL: i32 = 2;

/// The shader's fixed depth-region word base (`HEADER_BASE + MAX_SDF_EDITS *
/// SDF_EDIT_WORDS` = 196). Re-pinned here so a desync with the imported const is a
/// build error (it must equal the host `COMPOSITE_DEPTH_BASE_WORDS`).
const DEPTH_BASE: usize = 196;
const _: () = assert!(
    DEPTH_BASE == COMPOSITE_DEPTH_BASE_WORDS,
    "DEPTH_BASE must equal the shader's fixed depth-region base"
);

/// Total `u32` words for a runtime `w × h` extent: header + edit array + a `w*h` f32
/// depth region + a `w*h` u32 pixel region. Matches the shader's
/// `pixel_base() + w*h == DEPTH_BASE + 2*w*h`.
#[inline]
fn buffer_words(w: u32, h: u32) -> usize {
    DEPTH_BASE + 2 * (w as usize) * (h as usize)
}

/// Pixel-region word base at runtime extent (`DEPTH_BASE + w*h`), mirroring the
/// shader's `pixel_base()`.
#[inline]
fn pixel_base_words(w: u32, h: u32) -> usize {
    DEPTH_BASE + (w as usize) * (h as usize)
}

/// `ceil(w*h / LOCAL_SIZE_X)` — the 1D compute dispatch group count for `w × h`.
#[inline]
fn group_count(w: u32, h: u32) -> u32 {
    ((w as u64 * h as u64) as u32).div_ceil(LOCAL_SIZE_X)
}

/// Boots a validation-enabled headless context, or returns `None` (with a SKIP log)
/// when no GPU / loader / validation layer / dynamic-rendering is available.
fn boot_or_skip(test: &str) -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP {test}: validation layer / GPU / dynamicRendering unavailable ({e:?})");
            None
        }
    }
}

/// Asserts the validation messenger (incl. sync-validation, enabled in
/// `InstanceConfig::default`) recorded ZERO messages — the GPU-half oracle.
fn assert_validation_clean(ctx: &VulkanContext, label: &str) {
    let state = ctx
        .debug_state()
        .expect("invariant: validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "{label}: validation/sync-validation reported {} message(s) — see the [vk-validation] log",
        state.total()
    );
}

/// A perspective camera looking down -Z from `+Z`, 60° vertical FOV, aspect = `w/h`.
/// The scene geometry sits near the origin, so this camera sees it centered.
fn forward_camera(w: u32, h: u32) -> CompositePushConstants {
    CompositePushConstants::perspective(
        [0.0, 0.0, 3.0],  // eye on +Z
        [0.0, 0.0, -1.0], // forward toward the origin
        [1.0, 0.0, 0.0],  // right
        [0.0, 1.0, 0.0],  // up
        core::f32::consts::FRAC_PI_3, // 60° vertical FOV
        w,
        h,
    )
}

/// The matching host-side [`CompositeCamera::Perspective`] for [`forward_camera`] at
/// extent `w × h` (same eye/basis/FOV/aspect), so the host golden predicts the GPU.
fn forward_camera_host(w: u32, h: u32) -> CompositeCamera {
    let tan_half_fov = (core::f32::consts::FRAC_PI_3 * 0.5).tan();
    CompositeCamera::Perspective {
        eye: [0.0, 0.0, 3.0],
        forward: [0.0, 0.0, -1.0],
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        tan_half_fov,
        aspect: (w as f32) / (h as f32),
    }
}

/// The G4 correctness scene: the rung-9/10 "crater" CSG (a base sphere with a smaller
/// sphere subtracted) — a recognizable non-trivial field the camera centers on.
fn crater() -> Vec<SdfEdit> {
    vec![
        SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
    ]
}

/// The G5 SPARSE scene: ONE small sphere clustered near the origin, so a wide-FOV
/// camera leaves most of the frame empty (background ~70%+) — the P4/P5 best case.
fn sparse_scene() -> Vec<SdfEdit> {
    vec![SdfEdit::sphere([0.0, 0.0, 0.0], 0.25, sdf_op::UNION, 0.0)]
}

/// The G5 DENSE scene: a large sphere unioned with a large box so the field fills the
/// frame for the forward camera — the honest worst case (~no empty-space prefix).
fn dense_scene() -> Vec<SdfEdit> {
    vec![
        SdfEdit::sphere([0.0, 0.0, 0.0], 1.5, sdf_op::UNION, 0.0),
        SdfEdit::box_shape([0.0, 0.0, 0.0], [1.4, 1.4, 1.4], sdf_op::UNION, 0.0),
    ]
}

/// Seeds the edit-list header (word 0 = count, then the std430 edit array at
/// `HEADER_BASE = 4`) AND the depth region (seeded to `MESH_DEPTH_CLEAR` so the
/// marcher runs UNBOUNDED — a compute-only G3/G4/G5 has no rasterized mesh).
fn seed_buffer(base: NonNull<u8>, edits: &[SdfEdit], w: u32, h: u32) {
    let dst = base.as_ptr().cast::<u32>();
    let n_pixels = (w as usize) * (h as usize);
    // word 0 = edit_count.
    // SAFETY: `dst` is the start of a `buffer_words(w,h)*4`-byte host-coherent mapping
    // (the buffer was created at exactly that size); every index below is < that word
    // count. No GPU work is in flight yet (submit happens after), so the host writes
    // are unsynchronized-safe; `write_unaligned` tolerates the sub-allocated offset.
    unsafe { dst.write_unaligned(edits.len() as u32) };
    // The std430 edit array (12 words / edit, mirroring the shader's load_edit).
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
            // SAFETY: see the function-level note; `off + j < DEPTH_BASE` for the
            // fixed-cap edit array, well within the mapping.
            unsafe { dst.add(off + j).write_unaligned(word) };
        }
    }
    // The depth region: seed every pixel to the far-plane clear (1.0) ⇒ "no mesh".
    let clear_bits = MESH_DEPTH_CLEAR.to_bits();
    for i in 0..n_pixels {
        // SAFETY: `DEPTH_BASE + i` for `i < n_pixels` is the depth region, in-bounds
        // (`DEPTH_BASE + n_pixels == pixel_base_words(w,h)`).
        unsafe { dst.add(DEPTH_BASE + i).write_unaligned(clear_bits) };
    }
}

/// Reads the `w*h` packed-RGBA pixels out of the buffer's PIXEL region (valid only
/// after a fence-waited submit).
fn read_pixels(base: NonNull<u8>, w: u32, h: u32) -> Vec<u32> {
    let n = (w as usize) * (h as usize);
    let pbase = pixel_base_words(w, h);
    let p = base.as_ptr().cast::<u32>();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        // SAFETY: `pbase + i` for `i < n` is the pixel region, in-bounds
        // (`pbase + n == buffer_words(w,h)`). A fence wait preceded this read, so the
        // GPU writes are complete + coherent. Any bit pattern is a valid `u32`.
        out.push(unsafe { p.add(pbase + i).read_unaligned() });
    }
    out
}

/// Records + submits ONE compute-only marcher dispatch into a runtime-sized buffer,
/// fence-waits, and returns `(pixels, gpu_wall_time)`. The wall time brackets the
/// submit→fence-wait of the SINGLE recorded command buffer (the §1b serialized
/// model): with nothing else in flight it is dominated by the marcher's GPU time —
/// the available proxy for a timestamp query (the RHI exposes no query pool).
fn run_marcher(
    ctx: &VulkanContext,
    edits: &[SdfEdit],
    pc: CompositePushConstants,
    w: u32,
    h: u32,
    label: &str,
) -> (Vec<u32>, std::time::Duration) {
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
        let mapped = device
            .buffer_mapped_ptr(&buffer)
            .expect("host-visible buffer is mapped");
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
        })
        .expect("composite compute pipeline (needs an 80-byte compute push range)");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");

    encoder.begin().expect("begin");
    encoder.bind_compute_pipeline(&compute);
    encoder.bind_storage_buffer(&buffer, 0, 0);
    encoder.push_constants(ShaderStage::COMPUTE, 0, pc.as_bytes());
    encoder.dispatch(group_count(w, h), 1, 1);
    encoder.end().expect("end");

    let start = Instant::now();
    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");
    let elapsed = start.elapsed();

    let mapped = device
        .buffer_mapped_ptr(&buffer)
        .expect("host-visible buffer is mapped");
    let pixels = read_pixels(mapped, w, h);
    assert_eq!(pixels.len(), (w as usize) * (h as usize), "{label}: full readback");

    assert_validation_clean(ctx, label);

    // SAFETY: every resource was created on `device` and is destroyed exactly once;
    // the submission completed (fence-waited above), so none is GPU-in-use.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_compute_pipeline(compute);
        device.destroy_shader_module(cs);
        device.destroy_buffer(buffer);
    }

    (pixels, elapsed)
}

/// Splits a packed `0xAABBGGRR` into `[r, g, b]` (the low three bytes).
fn unpack_rgb(packed: u32) -> [i32; 3] {
    [
        (packed & 0xFF) as i32,
        ((packed >> 8) & 0xFF) as i32,
        ((packed >> 16) & 0xFF) as i32,
    ]
}

/// `true` iff the alpha channel is a valid packed pixel (`0xFF`) and the color is a
/// finite, in-range byte triple — a packed `u32` is always finite, so the real check
/// is the marcher actually wrote this word (alpha 0xFF) rather than leaving it zero.
fn is_written_pixel(packed: u32) -> bool {
    (packed >> 24) == 0xFF
}

// ===========================================================================
// G3 — 1080p + 720p perspective dispatch: validation/sync-validation clean +
//      finite (written) readback. The buffer is runtime-sized (NOT 64×64).
// ===========================================================================

/// G3 (1080p). A 1920×1080 perspective marcher dispatch into a ~16.6 MB runtime
/// buffer is validation- AND sync-validation-clean, completes, and every pixel is a
/// WRITTEN packed pixel (alpha 0xFF — finite by construction; not a left-zero word).
#[test]
fn perspective_1080p_dispatch_is_validation_clean_and_finite() {
    let Some(ctx) = boot_or_skip("perspective_1080p_dispatch") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    assert!(ctx.validation_enabled(), "validation must be active");

    let (w, h) = (1920u32, 1080u32);
    let edits = crater();
    let pc = forward_camera(w, h);
    assert_eq!(pc.camera_mode, CAM_MODE_PERSPECTIVE);
    assert_eq!(pc.count, w * h, "count must be the full pixel total");

    let (pixels, dt) = run_marcher(&ctx, &edits, pc, w, h, "G3-1080p");
    println!(
        "G3-1080p: {}x{} = {} px dispatched, wall {:?}",
        w,
        h,
        pixels.len(),
        dt
    );

    let written = pixels.iter().filter(|&&p| is_written_pixel(p)).count();
    assert_eq!(
        written,
        pixels.len(),
        "every 1080p pixel must be a finite written pixel (alpha 0xFF); {} were left unwritten",
        pixels.len() - written
    );
}

/// G3 (720p). A 1280×720 perspective marcher dispatch is validation/sync-validation
/// clean, completes, and every pixel is finite/written.
#[test]
fn perspective_720p_dispatch_is_validation_clean_and_finite() {
    let Some(ctx) = boot_or_skip("perspective_720p_dispatch") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    let (w, h) = (1280u32, 720u32);
    let edits = crater();
    let pc = forward_camera(w, h);

    let (pixels, dt) = run_marcher(&ctx, &edits, pc, w, h, "G3-720p");
    println!("G3-720p: {}x{} = {} px, wall {:?}", w, h, pixels.len(), dt);

    let written = pixels.iter().filter(|&&p| is_written_pixel(p)).count();
    assert_eq!(written, pixels.len(), "every 720p pixel must be finite/written");
}

// ===========================================================================
// G4 — small-N perspective host-vs-GPU correctness (±2/255). The host mirror
//      (`golden_composite_pixel_ex` + `CompositeCamera::Perspective`) predicts the
//      GPU per pixel on the perspective ray-gen path.
// ===========================================================================

/// Diffs every pixel of a GPU perspective dispatch at `(w, h)` against the host
/// golden mirror within ±2/255, and asserts a non-trivial fraction of pixels HIT the
/// lit surface (anti-vacuity: the camera actually sees the crater, not a blank frame).
fn assert_perspective_matches_host(ctx: &VulkanContext, w: u32, h: u32, label: &str) {
    let edits = crater();
    let pc = forward_camera(w, h);
    let host_cam = forward_camera_host(w, h);

    let (pixels, _dt) = run_marcher(ctx, &edits, pc, w, h, label);

    let mut max_delta = 0i32;
    let mut worst = (0u32, 0u32, 0u32, 0u32);
    let mut hits = 0usize;
    for py in 0..h {
        for px in 0..w {
            let idx = (py * w + px) as usize;
            let got = pixels[idx];
            // No mesh in a compute-only run: the depth region is the clear sentinel.
            let want = golden_composite_pixel_ex(&edits, MESH_DEPTH_CLEAR, px, py, w, h, host_cam);
            let g = unpack_rgb(got);
            let wv = unpack_rgb(want);
            // A "hit" pixel is the warm lit color (high red, low blue); track for
            // anti-vacuity. BACKGROUND is (13,13,26)-ish ⇒ low red.
            if g[0] > 60 {
                hits += 1;
            }
            for c in 0..3 {
                let d = (g[c] - wv[c]).abs();
                if d > max_delta {
                    max_delta = d;
                    worst = (px, py, got, want);
                }
            }
        }
    }
    println!(
        "{label}: {}x{} max per-channel delta = {}/255 (worst px ({},{}) got {:#010x} want {:#010x}); lit pixels {}",
        w, h, max_delta, worst.0, worst.1, worst.2, worst.3, hits
    );
    assert!(
        max_delta <= CHANNEL_TOL,
        "{label}: host mirror diverged from GPU by {}/255 (> {}/255) at px ({},{}): got {:#010x} want {:#010x}",
        max_delta,
        CHANNEL_TOL,
        worst.0,
        worst.1,
        worst.2,
        worst.3
    );
    assert!(
        hits > (w as usize * h as usize) / 200,
        "{label}: anti-vacuity — the perspective camera must SEE the crater (only {hits} lit pixels)"
    );
}

/// G4 (64×64). Perspective GPU output equals the host mirror within ±2/255.
#[test]
fn perspective_64x64_matches_host_mirror() {
    let Some(ctx) = boot_or_skip("perspective_64x64_matches_host") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    assert_perspective_matches_host(&ctx, 64, 64, "G4-64x64");
}

/// G4 (128×128). Perspective GPU output equals the host mirror within ±2/255 at a
/// non-square-of-64 extent (exercises the `idx%w`/`idx/w` extent reconstruction).
#[test]
fn perspective_128x128_matches_host_mirror() {
    let Some(ctx) = boot_or_skip("perspective_128x128_matches_host") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    assert_perspective_matches_host(&ctx, 128, 128, "G4-128x128");
}

// ===========================================================================
// G5 — GPU marcher wall-time baselines: SPARSE vs DENSE × 720p / 1080p,
//      perspective camera. (NO timestamp-query pool in the RHI: a fence-bracketed
//      submit wall-time is the available proxy — documented in `run_marcher`.)
// ===========================================================================

/// G5. Records the 4 baselines (sparse/dense × 720p/1080p) and prints a table. The
/// edit-count band is NOTED: the shader caps `MAX_SDF_EDITS = 16u`, so the scenes use
/// 1–2 edits (the ≤16 "tech-demo floor"); P0b/P1 widen the cap to the 256–4096 band.
#[test]
fn perspective_gpu_time_baselines() {
    let Some(ctx) = boot_or_skip("perspective_gpu_time_baselines") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());

    // Warm-up: the first submit pays one-time driver/pipeline costs; discard it so the
    // baselines reflect steady marcher time.
    let _ = run_marcher(&ctx, &sparse_scene(), forward_camera(1280, 720), 1280, 720, "G5-warmup");

    let runs = [
        ("sparse", sparse_scene(), 1280u32, 720u32),
        ("sparse", sparse_scene(), 1920, 1080),
        ("dense", dense_scene(), 1280, 720),
        ("dense", dense_scene(), 1920, 1080),
    ];

    println!("=== G5 GPU marcher wall-time baselines (perspective, 1-2 edits cap=16) ===");
    println!("| scene  | resolution | best-of-3 wall (median submit→fence) |");
    println!("|--------|------------|--------------------------------------|");
    for (name, edits, w, h) in runs {
        // Best-of-3 to suppress scheduler/driver jitter on the wall-clock proxy.
        let mut times = Vec::new();
        for _ in 0..3 {
            let (_px, dt) = run_marcher(&ctx, &edits, forward_camera(w, h), w, h, "G5");
            times.push(dt);
        }
        times.sort();
        let median = times[1];
        println!("| {:<6} | {:>4}x{:<5} | {:>10.3?} (min {:.3?}) |", name, w, h, median, times[0]);
    }
    println!("NOTE: wall-clock proxy (no RHI timestamp-query pool); MAX_SDF_EDITS=16 cap — record band is the ≤16 floor.");
}
