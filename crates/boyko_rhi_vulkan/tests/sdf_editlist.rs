//! Phase-6 rung-9 acceptance test: sphere-trace an ORDERED SDF EDIT-LIST
//! (multi-primitive CSG) via a compute shader, golden-verified — the first real
//! "CSG on screen" thread (generalizes rung 8's single hardcoded sphere into the
//! SDF-edits model, SDF doc §2-§3).
//!
//! Reuses the PROVEN rung-1 compute + storage-BUFFER path verbatim (see
//! `tests/compute.rs` / `tests/sdf_spheretrace.rs`): one host-visible-coherent
//! `STORAGE` buffer, the fixed Slice-0 compute pipeline layout (binding 0 = one
//! `RWStructuredBuffer<uint>` at COMPUTE + a 4-byte `uint count` push constant).
//! The edit-list reaches the shader PACKED as a header at the front of that SAME
//! single buffer (no second binding): the host writes `edit_count` + the edit
//! array via [`encode_edit_list`], the shader reads them and writes the packed
//! pixels AFTER the header.
//!
//! # The CSG scene + what it proves
//!
//! The edit-list is a base sphere with a smaller sphere SUBTRACTED out of its
//! `+x` side — a recognizable crater/bite, unmistakably NOT a single primitive.
//! The test picks three discriminating texels host-side from the golden:
//!
//! - **the carved texel** — a pixel whose ray HITS the base sphere ALONE but
//!   MISSES after the subtraction (now background). This is the load-bearing CSG
//!   discriminator: it can only be background if the subtraction actually ran.
//! - **a surface texel** — a pixel that HITS the combined surface (lit color),
//!   proving the field is still a solid body, not erased.
//! - **a corner texel** — a guaranteed MISS (background), the baseline.
//!
//! Each is asserted against the host golden ([`golden_editlist_pixel`]) within
//! the same `+/-2/255` per-channel tolerance as rung 8, plus hit!=miss guards.
//!
//! # The oracle (plan §6, mirrored from `sdf_spheretrace.rs`)
//!
//! Boots with validation enabled and asserts `debug_state().total() == 0` after
//! the run. A GPU-less / loader-less / validation-layer-less host makes
//! `VulkanContext::boot` return `Err`; the test skips gracefully.

use core::ptr::NonNull;

use boyko_rhi::{
    BufferDesc, BufferUsage, ComputePipelineDesc, MemoryLocation, RhiCommandEncoder, RhiDevice,
    RhiQueue, ShaderStage,
};

use boyko_rhi_vulkan::compute::{EDITLIST_BUFFER_WORDS, LOCAL_SIZE_X, PIXEL_BASE_WORDS, SDF_IMG_H, SDF_IMG_W, SdfEdit, editlist_pixel_hits, encode_edit_list, sdf_editlist_spirv, sdf_op};
use boyko_rhi_vulkan::goldens::{golden_editlist_pixel};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// Total pixel count (the push constant; the shader bounds `idx < count`).
const PIXELS: u32 = SDF_IMG_W * SDF_IMG_H;

/// Per-channel tolerance on the packed-RGBA bytes (identical to rung 8): DXC
/// `mad`/`fma` rounding makes a bit-exact match brittle; `+/-2/255` still proves
/// the lit CSG surface color while a wrong color misses by ~100+.
const CHANNEL_TOL: i32 = 2;

/// Boots a validation-enabled context, or returns `None` (with a SKIP log) when
/// no GPU / loader / validation layer is available.
fn boot_or_skip(test: &str) -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP {test}: validation layer / GPU unavailable ({e:?})");
            None
        }
    }
}

/// Asserts the validation messenger recorded ZERO messages.
fn assert_validation_clean(ctx: &VulkanContext) {
    let state = ctx
        .debug_state()
        .expect("invariant: validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during the edit-list run — see the [vk-validation] log",
        state.total()
    );
}

/// `ceil(PIXELS / LOCAL_SIZE_X)` — the 1D dispatch group count.
fn group_count_x() -> u32 {
    PIXELS.div_ceil(LOCAL_SIZE_X)
}

/// Writes `words` `u32`s into a buffer's persistent host-coherent mapping (valid
/// before the submit — the CPU seeds the edit-list header here).
fn write_words(base: NonNull<u8>, words: &[u32]) {
    let dst = base.as_ptr().cast::<u32>();
    for (i, &w) in words.iter().enumerate() {
        // SAFETY: the buffer is `EDITLIST_BUFFER_WORDS * 4` bytes inside the
        // persistent host-coherent mapping; `dst + i` for `i < words.len() <=
        // EDITLIST_BUFFER_WORDS` is in-bounds. No GPU work is in flight yet (the
        // submit happens after this), so the host write is unsynchronized-safe.
        // `write_unaligned` tolerates the sub-allocated offset's alignment.
        unsafe { dst.add(i).write_unaligned(w) };
    }
}

/// Reads `PIXELS` packed-RGBA `u32`s from the buffer's PIXEL region (after the
/// edit-list header), valid only after a fence-waited submit.
fn read_pixels(base: NonNull<u8>) -> Vec<u32> {
    let n = PIXELS as usize;
    let mut out = Vec::with_capacity(n);
    let base = base.as_ptr().cast::<u32>();
    for i in 0..n {
        // SAFETY: the buffer is `EDITLIST_BUFFER_WORDS * 4` bytes inside the
        // persistent host-coherent mapping; `PIXEL_BASE_WORDS + i` for `i < n` is
        // in-bounds (`PIXEL_BASE_WORDS + n == EDITLIST_BUFFER_WORDS`). A fence
        // wait preceded this read, so the GPU writes are complete + coherent.
        let v = unsafe { base.add(PIXEL_BASE_WORDS + i).read_unaligned() };
        out.push(v);
    }
    out
}

/// Splits a packed `0xAABBGGRR` into `[r, g, b]` (the low three bytes).
fn unpack_rgb(packed: u32) -> [i32; 3] {
    [
        (packed & 0xFF) as i32,
        ((packed >> 8) & 0xFF) as i32,
        ((packed >> 16) & 0xFF) as i32,
    ]
}

/// Asserts two packed colors agree within `CHANNEL_TOL` per RGB channel.
fn assert_color_close(got: u32, want: u32, label: &str) {
    let g = unpack_rgb(got);
    let w = unpack_rgb(want);
    for c in 0..3 {
        assert!(
            (g[c] - w[c]).abs() <= CHANNEL_TOL,
            "{label}: channel {c} off by {} (got {:#010x} -> {:?}, want {:#010x} -> {:?}, tol {CHANNEL_TOL})",
            (g[c] - w[c]).abs(),
            got,
            g,
            want,
            w,
        );
    }
}

/// Dispatches the rung-9 SDF edit-list compute over `edits` on `ctx` and returns
/// the `PIXELS`-long packed-RGBA readback. Mirrors the `crater()` flow exactly:
/// one host-visible storage buffer, the packed-header seed via [`encode_edit_list`],
/// the fixed Slice-0 one-binding compute pipeline, one fenced submit, readback.
/// Asserts the validation messenger stays clean, then destroys every resource.
fn run_editlist(ctx: &VulkanContext, edits: &[SdfEdit]) -> Vec<u32> {
    let device: &VulkanContext = ctx;
    let queue = ctx.rhi_queue();

    let buffer = device
        .create_buffer(&BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("storage buffer");

    {
        let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
        encode_edit_list(&mut header, edits);
        let mapped = device
            .buffer_mapped_ptr(&buffer)
            .expect("host-visible buffer is mapped");
        write_words(mapped, &header);
    }

    let module = device
        .create_shader_module(sdf_editlist_spirv())
        .expect("sdf_editlist shader module");
    let pipeline = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            push_constant_bytes: 4,
            bind_group_layout: None,
        })
        .expect("sdf_editlist compute pipeline");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");

    encoder.begin().expect("begin");
    encoder.bind_compute_pipeline(&pipeline);
    encoder.bind_storage_buffer(&buffer, 0, 0);
    encoder.push_constants(ShaderStage::COMPUTE, 0, &PIXELS.to_ne_bytes());
    encoder.dispatch(group_count_x(), 1, 1);
    encoder.end().expect("end");

    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    let mapped = device
        .buffer_mapped_ptr(&buffer)
        .expect("host-visible buffer is mapped");
    let out = read_pixels(mapped);
    assert_eq!(out.len(), PIXELS as usize);

    assert_validation_clean(ctx);

    // SAFETY: every resource below was created on `device` and is destroyed
    // exactly once; the last submission completed (fence-waited above), so none is
    // in use by the GPU.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_compute_pipeline(pipeline);
        device.destroy_shader_module(module);
        device.destroy_buffer(buffer);
    }

    out
}

/// The base-sphere-only edit-list (one union sphere) — the "before subtraction"
/// field used to find the carved discriminator texel.
fn base_only() -> Vec<SdfEdit> {
    vec![SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0)]
}

/// The CSG edit-list: the base sphere (center origin, r=0.5) with a smaller
/// sphere SUBTRACTED through its `+x` body (center (0.3, 0, 0), r=0.35) — a
/// through-hole/bite, recognizably NOT a single primitive. The subtract sphere
/// sits ON the body axis (z=0) so it carves the full material column at the
/// carved texels (a ray that hit the base surface there now exits to background);
/// a near-surface bite (z near the front) would only dent the surface, not open a
/// background hole. ~60 texels are carved and ~750 still hit — wide margin for the
/// host-side discriminator scan below.
fn crater() -> Vec<SdfEdit> {
    vec![
        SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
    ]
}

/// Finds a "carved" texel: a pixel that HITS the base sphere alone but MISSES the
/// CSG field after the subtraction. This is the load-bearing CSG discriminator.
fn find_carved_texel(base: &[SdfEdit], csg: &[SdfEdit]) -> Option<(u32, u32)> {
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            if editlist_pixel_hits(base, px, py) && !editlist_pixel_hits(csg, px, py) {
                return Some((px, py));
            }
        }
    }
    None
}

/// Finds a "surface" texel: a pixel that HITS the CSG field (a remaining solid
/// surface). The center is preferred when it qualifies.
fn find_surface_texel(csg: &[SdfEdit]) -> Option<(u32, u32)> {
    let cx = SDF_IMG_W / 2;
    let cy = SDF_IMG_H / 2;
    if editlist_pixel_hits(csg, cx, cy) {
        return Some((cx, cy));
    }
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            if editlist_pixel_hits(csg, px, py) {
                return Some((px, py));
            }
        }
    }
    None
}

/// Rung 9 — sphere-trace an ordered SDF edit-list (base sphere minus a smaller
/// sphere). The carved texel HITS the base alone but MISSES the CSG (background);
/// a surface texel HITS the CSG (lit); a corner MISSES.
#[test]
fn sdf_editlist_crater_csg() {
    let Some(ctx) = boot_or_skip("sdf_editlist_crater_csg") else {
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

    let base = base_only();
    let csg = crater();

    // Pick the discriminating texels host-side BEFORE any GPU run, so the
    // assertions are independent of the GPU and prove the CSG is geometry-correct.
    let (carve_px, carve_py) =
        find_carved_texel(&base, &csg).expect("invariant: the crater must remove at least one texel that the base sphere hit");
    let (surf_px, surf_py) =
        find_surface_texel(&csg).expect("invariant: the CSG body must still hit at least one texel");
    let (corner_px, corner_py) = (0u32, 0u32);
    assert!(
        !editlist_pixel_hits(&csg, corner_px, corner_py),
        "invariant: the (0,0) corner must MISS the CSG field"
    );

    let out = run_editlist(&ctx, &csg);

    let idx = |px: u32, py: u32| (py * SDF_IMG_W + px) as usize;

    // The carved texel: GPU MISSES (background) — the CSG subtraction ran.
    let carve_got = out[idx(carve_px, carve_py)];
    let carve_want = golden_editlist_pixel(&csg, carve_px, carve_py);
    assert_color_close(
        carve_got,
        carve_want,
        "carved (hits base alone, MISSES after subtraction)",
    );

    // The surface texel: GPU HITS (lit CSG surface).
    let surf_got = out[idx(surf_px, surf_py)];
    let surf_want = golden_editlist_pixel(&csg, surf_px, surf_py);
    assert_color_close(surf_got, surf_want, "surface (HIT, lit CSG body)");

    // The corner texel: GPU MISSES (background).
    let corner_got = out[idx(corner_px, corner_py)];
    let corner_want = golden_editlist_pixel(&csg, corner_px, corner_py);
    assert_color_close(corner_got, corner_want, "corner (MISS, background)");

    // The carved (now-background) and surface (lit) goldens MUST differ — the
    // proof that the subtraction carved a hole rather than leaving a solid sphere.
    assert_ne!(
        carve_want, surf_want,
        "invariant: the carved-background and lit-surface goldens must differ (the CSG bite is real)"
    );
    assert_ne!(
        unpack_rgb(carve_got),
        unpack_rgb(surf_got),
        "the GPU carved (miss) and surface (hit) pixels must differ — proving the subtraction ran"
    );
    // The carved and corner are both background — they must agree (both missed).
    assert_color_close(carve_got, corner_got, "carved vs corner (both background)");

    drop(ctx);
}

/// A box at the origin with half-extents 0.5 — the BOX primitive in isolation.
fn box_at_origin() -> Vec<SdfEdit> {
    vec![SdfEdit::box_shape(
        [0.0, 0.0, 0.0],
        [0.5, 0.5, 0.5],
        sdf_op::UNION,
        0.0,
    )]
}

/// A sphere at the origin with the SAME radius (0.5) as the box's half-extent —
/// the box-vs-sphere guard scene. An inscribed sphere of this radius is wholly
/// contained in the box, so a texel over a box corner/edge region (radius from
/// the view-plane center > 0.5) HITS the box but MISSES this sphere.
fn inscribed_sphere() -> Vec<SdfEdit> {
    vec![SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0)]
}

/// Two unit-radius-ish spheres separated along x, the second UNION'd with a
/// non-zero `smoothness` so a polynomial smooth-min fillet bridges the gap. The
/// fillet ADDS material the hard union (smoothness=0) leaves as empty space.
fn smooth_pair() -> Vec<SdfEdit> {
    vec![
        SdfEdit::sphere([-0.35, 0.0, 0.0], 0.25, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.35, 0.0, 0.0], 0.25, sdf_op::UNION, 0.5),
    ]
}

/// The SAME pair as [`smooth_pair`] but with `smoothness = 0` — the hard-union
/// guard scene. The two spheres do not touch, so the gap between them is empty
/// (background); the smooth-vs-hard discriminator texel lives in that gap.
fn hard_pair() -> Vec<SdfEdit> {
    vec![
        SdfEdit::sphere([-0.35, 0.0, 0.0], 0.25, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.35, 0.0, 0.0], 0.25, sdf_op::UNION, 0.0),
    ]
}

/// Rung 9 — the BOX primitive on the GPU, golden-verified and DISTINGUISHED from
/// a sphere. A box at the origin (half-extents 0.5) is sphere-traced; the
/// discriminating texel (16, 16) maps to view-plane (-0.484, +0.484), radius
/// ~0.685 from center. Both |x| and |y| are < 0.5, so the ray sits over the
/// box's flat +z face near a corner and HITS the box — but radius 0.685 > 0.5,
/// so a sphere of radius 0.5 (the box's inscribed sphere) MISSES that same texel.
/// This is the box-vs-sphere guard: if `sd_box` had a transposed `q.x`/`q.y` or
/// collapsed to a sphere, the GPU box pixel would not match the box golden there.
#[test]
fn box_csg_golden() {
    let Some(ctx) = boot_or_skip("box_csg_golden") else {
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

    let box_scene = box_at_origin();
    let sphere_scene = inscribed_sphere();

    // The box-vs-sphere discriminator, picked host-side BEFORE any GPU run.
    let (bx, by) = (16u32, 16u32);
    assert!(
        editlist_pixel_hits(&box_scene, bx, by),
        "invariant: the box must HIT the corner-region texel ({bx},{by})"
    );
    assert!(
        !editlist_pixel_hits(&sphere_scene, bx, by),
        "guard: a same-half-extent sphere must MISS ({bx},{by}) — a box corner extends past the inscribed sphere"
    );
    // The box-hit and sphere-miss goldens at this texel MUST differ (lit box face
    // vs background): the proof the texel genuinely separates a box from a sphere.
    let box_want = golden_editlist_pixel(&box_scene, bx, by);
    let sphere_want = golden_editlist_pixel(&sphere_scene, bx, by);
    assert_ne!(
        box_want, sphere_want,
        "invariant: the box golden (lit face) and the sphere golden (background) must differ at the discriminator"
    );

    // A guaranteed-miss baseline: the (0,0) corner is outside the box footprint.
    let (corner_px, corner_py) = (0u32, 0u32);
    assert!(
        !editlist_pixel_hits(&box_scene, corner_px, corner_py),
        "invariant: the (0,0) corner must MISS the box field"
    );

    let out = run_editlist(&ctx, &box_scene);
    let idx = |px: u32, py: u32| (py * SDF_IMG_W + px) as usize;

    // The discriminating texel: GPU HITS the box (lit face) and matches the box
    // golden — NOT the sphere golden (which is background there).
    let box_got = out[idx(bx, by)];
    assert_color_close(box_got, box_want, "box corner-region (HIT box, MISS sphere)");
    assert!(
        !{
            let g = unpack_rgb(box_got);
            let w = unpack_rgb(sphere_want);
            (0..3).all(|c| (g[c] - w[c]).abs() <= CHANNEL_TOL)
        },
        "the GPU box pixel must NOT match the sphere (background) golden at ({bx},{by}) — proving a box, not a sphere"
    );

    // The corner texel: GPU MISSES (background).
    let corner_got = out[idx(corner_px, corner_py)];
    let corner_want = golden_editlist_pixel(&box_scene, corner_px, corner_py);
    assert_color_close(corner_got, corner_want, "corner (MISS, background)");

    drop(ctx);
}

/// Rung 9 — the SMOOTH-MIN (k > 0) op on the GPU, golden-verified and
/// DISTINGUISHED from a hard union. Two non-touching spheres (centers +/-0.35,
/// r=0.25) are UNION'd, the second with `smoothness = 0.5`. The polynomial
/// smooth-min bulges a fillet of material into the gap that a hard union
/// (smoothness=0) leaves empty. The discriminating texel (28, 28) sits in that
/// gap: it HITS the smooth scene (a lit fillet surface) but MISSES the hard scene
/// (background). If `smin`/`smax` had a sign error or a wrong blend, the GPU
/// smooth pixel would not match the smooth golden there.
#[test]
fn smooth_union_golden() {
    let Some(ctx) = boot_or_skip("smooth_union_golden") else {
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

    let smooth = smooth_pair();
    let hard = hard_pair();

    // The smooth-vs-hard discriminator (in the fillet/gap), picked host-side.
    let (fx, fy) = (28u32, 28u32);
    assert!(
        editlist_pixel_hits(&smooth, fx, fy),
        "invariant: the smooth-min fillet must HIT the gap texel ({fx},{fy})"
    );
    assert!(
        !editlist_pixel_hits(&hard, fx, fy),
        "guard: the hard union (smoothness=0) must MISS ({fx},{fy}) — the gap is empty without the fillet"
    );
    // The smooth fillet color and the hard background MUST differ beyond the
    // tolerance: the proof the fillet genuinely adds visible material.
    let smooth_want = golden_editlist_pixel(&smooth, fx, fy);
    let hard_want = golden_editlist_pixel(&hard, fx, fy);
    assert!(
        !{
            let g = unpack_rgb(smooth_want);
            let w = unpack_rgb(hard_want);
            (0..3).all(|c| (g[c] - w[c]).abs() <= CHANNEL_TOL)
        },
        "invariant: the smooth fillet golden and the hard-union (background) golden must differ beyond +/-{CHANNEL_TOL} at the discriminator"
    );

    // A guaranteed-miss baseline: the (0,0) corner is outside both fields.
    let (corner_px, corner_py) = (0u32, 0u32);
    assert!(
        !editlist_pixel_hits(&smooth, corner_px, corner_py),
        "invariant: the (0,0) corner must MISS the smooth field"
    );

    let out = run_editlist(&ctx, &smooth);
    let idx = |px: u32, py: u32| (py * SDF_IMG_W + px) as usize;

    // The fillet texel: GPU HITS (lit smooth surface) and matches the smooth
    // golden — which is measurably different from the hard-union background.
    let smooth_got = out[idx(fx, fy)];
    assert_color_close(smooth_got, smooth_want, "smooth fillet (HIT smooth, MISS hard)");
    assert!(
        !{
            let g = unpack_rgb(smooth_got);
            let w = unpack_rgb(hard_want);
            (0..3).all(|c| (g[c] - w[c]).abs() <= CHANNEL_TOL)
        },
        "the GPU smooth pixel must NOT match the hard-union (background) golden at ({fx},{fy}) — proving smooth-min added the fillet"
    );

    // The corner texel: GPU MISSES (background).
    let corner_got = out[idx(corner_px, corner_py)];
    let corner_want = golden_editlist_pixel(&smooth, corner_px, corner_py);
    assert_color_close(corner_got, corner_want, "corner (MISS, background)");

    drop(ctx);
}
