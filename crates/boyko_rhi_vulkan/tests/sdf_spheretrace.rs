//! Phase-6 rung-8 acceptance test: sphere-trace ONE analytic SDF primitive via a
//! compute shader, golden-verified — the first real "SDF on screen" thread.
//!
//! Reuses the PROVEN rung-1 compute + storage-BUFFER path verbatim (see
//! `tests/compute.rs`): one host-visible-coherent `STORAGE` buffer of `W*H`
//! `u32`s, the fixed Slice-0 compute pipeline layout (binding 0 = one
//! `RWStructuredBuffer<uint>` at COMPUTE + a 4-byte `uint count` push constant),
//! record begin → bind pipeline → bind buffer → push `W*H` → dispatch
//! `ceil(W*H/64)` → end, submit + fence-wait, read back the persistent mapping.
//! NO new descriptor plumbing, NO storage image — exactly the rung-1 contract.
//!
//! # What it proves
//!
//! The `shaders/sdf_spheretrace.hlsl` compute shader sphere-traces one hardcoded
//! analytic sphere (`sdf(p) = length(p) - 0.5`) under a deterministic
//! orthographic camera looking down -Z, lights each hit (Lambert + ambient from
//! one directional light), and packs the RGBA into a `u32` per pixel. The test:
//!
//! - asserts the CENTER pixel (its ray HITS the sphere) equals the host-side lit
//!   golden ([`golden_sdf_pixel`]) within a small per-channel tolerance, and
//! - asserts a CORNER pixel (its ray MISSES) equals the background color.
//!
//! The hit-center + miss-corner pair proves the sphere-trace actually rendered a
//! sphere (a constant fill could not produce two distinct, geometry-correct
//! colors). The host golden mirrors the shader's camera/sphere/light math
//! exactly (one source of truth in `compute.rs`).
//!
//! # The oracle (plan §6, mirrored from `compute.rs`)
//!
//! Boots with validation enabled and asserts `debug_state().total() == 0` after
//! the run — a validation WARNING/ERROR FAILS the test (the soundness oracle
//! that substitutes for Miri on the raw-FFI path). The golden diff is the second
//! oracle.
//!
//! # CI gate (graceful skip)
//!
//! A GPU-less / loader-less / validation-layer-less host makes
//! `VulkanContext::boot` return `Err`; the test skips gracefully (mirrors the
//! compute tests).

use core::ptr::NonNull;

use boyko_rhi::{
    BufferDesc, BufferUsage, ComputePipelineDesc, MemoryLocation, RhiCommandEncoder, RhiDevice,
    RhiQueue, ShaderStage,
};

use boyko_rhi_vulkan::compute::{LOCAL_SIZE_X, SDF_IMG_H, SDF_IMG_W, sdf_pixel_hits, sdf_spheretrace_spirv};
use boyko_rhi_vulkan::goldens::{golden_sdf_pixel};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// Total pixel count (the storage buffer holds one packed-RGBA `u32` per pixel).
const PIXELS: u32 = SDF_IMG_W * SDF_IMG_H;

/// Per-channel tolerance on the packed-RGBA bytes. DXC `mad`/`fma` rounding and
/// the `*255 + 0.5` round-to-nearest make a bit-exact match brittle across
/// drivers; ±2 / 255 still proves the lit sphere color (and a constant fill or a
/// background-colored hit would miss by far more).
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
    if !ctx.validation_enabled() {
        assert!(
            std::env::var_os("BOYKO_DISABLE_VALIDATION").is_some(),
            "validation must be active when enable_validation is set and the escape hatch is absent"
        );
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) - messenger oracle skipped");
        return;
    }
    let state = ctx
        .debug_state()
        .expect("invariant: validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during the SDF run — see the [vk-validation] log",
        state.total()
    );
}

/// `ceil(PIXELS / LOCAL_SIZE_X)` — the 1D dispatch group count.
fn group_count_x() -> u32 {
    PIXELS.div_ceil(LOCAL_SIZE_X)
}

/// Reads `PIXELS` packed-RGBA `u32`s from a buffer's persistent host-coherent
/// mapping (valid only after a fence-waited submit). Mirrors `compute.rs`'s
/// `read_back`.
fn read_back(base: NonNull<u8>) -> Vec<u32> {
    let n = PIXELS as usize;
    let mut out = Vec::with_capacity(n);
    let base = base.as_ptr().cast::<u32>();
    for i in 0..n {
        // SAFETY: the buffer is `PIXELS * 4` bytes inside the persistent
        // host-coherent mapping; `base + i` for `i < n` is in-bounds; a fence
        // wait preceded this read, so the GPU writes are complete + coherent.
        // `read_unaligned` tolerates the sub-allocated offset's alignment.
        let v = unsafe { base.add(i).read_unaligned() };
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

/// Rung 8 — sphere-trace one analytic sphere; the center pixel HITS (lit color),
/// a corner pixel MISSES (background).
#[test]
fn sdf_spheretrace_hit_center_miss_corner() {
    let Some(ctx) = boot_or_skip("sdf_spheretrace_hit_center_miss_corner") else {
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

    let device: &VulkanContext = &ctx;
    let queue = ctx.rhi_queue();

    // One storage buffer of W*H packed-RGBA u32s, host-visible+coherent so the
    // CPU can read the result back directly (the rung-1 readback path).
    let buffer = device
        .create_buffer(&BufferDesc {
            size: (PIXELS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("storage buffer");

    // Compile the SDF compute module + build the pipeline on the shared Slice-0
    // layout (one storage binding + a 4-byte push range — push_constant_bytes:4).
    let module = device
        .create_shader_module(sdf_spheretrace_spirv())
        .expect("sdf_spheretrace shader module");
    let pipeline = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &module,
            entry: c"main",
            push_constant_bytes: 4,
            bind_group_layout: None,
        })
        .expect("sdf_spheretrace compute pipeline");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");

    // Record: begin → bind pipeline → bind storage buffer → push pixel count →
    // dispatch ceil(W*H/64) → end (the rung-1 recording shape verbatim).
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
    let out = read_back(mapped);
    assert_eq!(out.len(), PIXELS as usize);

    // Pick a guaranteed-HIT pixel (the image center) and a guaranteed-MISS pixel
    // (the (0,0) corner) host-side, so the assertion is independent of any GPU
    // run. The host golden mirrors the shader's camera/sphere math.
    let cx = SDF_IMG_W / 2;
    let cy = SDF_IMG_H / 2;
    assert!(
        sdf_pixel_hits(cx, cy),
        "invariant: the center pixel ({cx},{cy}) must HIT the sphere — golden is miscomputed"
    );
    assert!(
        !sdf_pixel_hits(0, 0),
        "invariant: the (0,0) corner pixel must MISS the sphere — golden is miscomputed"
    );

    let center_idx = (cy * SDF_IMG_W + cx) as usize;
    let center_got = out[center_idx];
    let center_want = golden_sdf_pixel(cx, cy);
    assert_color_close(center_got, center_want, "center (HIT, lit sphere)");

    let corner_idx = 0usize; // pixel (0,0)
    let corner_got = out[corner_idx];
    let corner_want = golden_sdf_pixel(0, 0);
    assert_color_close(corner_got, corner_want, "corner (MISS, background)");

    // Sanity: the hit and miss colors must actually differ (a constant fill would
    // make them equal). This is the "rendered a sphere, not a flat color" guard.
    assert_ne!(
        center_want, corner_want,
        "invariant: the lit-sphere and background goldens must differ"
    );
    assert_ne!(
        unpack_rgb(center_got),
        unpack_rgb(corner_got),
        "the GPU center (hit) and corner (miss) pixels must differ — a constant fill would not"
    );

    // The oracle: a clean run records zero validation messages.
    assert_validation_clean(&ctx);

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
    drop(ctx);
}
