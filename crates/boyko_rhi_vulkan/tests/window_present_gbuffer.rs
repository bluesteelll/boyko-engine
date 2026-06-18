//! Render **P1c GPU gate** (scaffold) — the FIRST IMAGE-BASED HYBRID FRAME ON SCREEN.
//! The P1b shared-depth marcher (a real GPU-rasterized mesh's depth written into a
//! D32_SFLOAT IMAGE, SAMPLED directly by the SDF compute marcher, which STORES the
//! FINAL composite into an R8G8B8A8 ALBEDO storage image) is driven ON SCREEN and the
//! ALBEDO is present-blit (fullscreen-sample) into the windowed swapchain image, with
//! the validation layer as the soundness oracle. On ONE presented frame — before
//! present — the swapchain image is copied back into a host-visible staging buffer and
//! golden-asserted against the host composite truth ([`golden_composite_pixel`]).
//!
//! # The P1c graduation: image-based, NO depth→buffer copy
//!
//! This is the on-screen counterpart of the P1b OFFSCREEN driver
//! (`tests/sdf_gbuffer_hybrid.rs::run_gbuffer_hybrid`). Where the packed on-screen path
//! (`window_present_hybrid.rs`) copies the rasterized depth into a shared buffer
//! (`copy_image_to_buffer(depth)`) and the marcher reads it as a packed buffer, the P1c
//! path SAMPLES the depth IMAGE and STORES the composite into the ALBEDO IMAGE — the
//! per-frame depth→buffer copy is GONE. The single depth
//! `DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL` barrier replaces the packed
//! path's depth copy + its two transfer barriers; the descriptor sets are written ONCE
//! at [`Renderer::render_gbuffer_frame`]'s first sync (NO per-frame
//! `vkUpdateDescriptorSets`).
//!
//! Determinism (INVIOLABLE): the marcher (field eval + ray-gen + lighting + the albedo
//! composite) is BYTE-UNTOUCHED from P1b (the verbatim `sdf_gbuffer_composite_spirv`),
//! so the on-screen image-composite golden equals the packed on-screen composite within
//! `+/-2/255` per channel — the SAME host golden the packed path uses.
//!
//! # 1:1 top-left present (WSI-clamp safe)
//!
//! The composite is rendered at its NATIVE 64×64 size; [`Renderer::render_gbuffer_frame`]
//! present-blits it 1:1 in the swapchain image's TOP-LEFT (it clamps the present
//! viewport/scissor to `min(swapchain_extent, present_extent)`), so the per-texel golden
//! is exact regardless of a WSI `current_extent` clamp, as long as the swapchain is at
//! least 64×64 (the top-left sub-rect fits). The G-buffer + marcher dispatch are sized
//! to the 64×64 composite, NOT the (possibly wider) swapchain extent.
//!
//! # The discriminator texels (picked host-side, BEFORE any GPU run)
//!
//! The same three rung-10/P1b regions: a mesh-occludes-SDF texel (`MESH_COLOR`), an SDF
//! lit texel, and a background texel — each asserted color-close to
//! [`golden_composite_pixel`] within `+/-2/255`, accounting for the swapchain being
//! `B8G8R8A8` (the readback bytes are then BGRA; the golden is RGBA byte order).
//!
//! # SCAFFOLD STATUS — the GPU run is the TESTER's
//!
//! This file compiles + [`Renderer::render_gbuffer_frame`] records the full P1c stream,
//! but the golden GPU assertion is gated behind a graceful boot/WSI/format SKIP and a
//! `#[cfg(windows)]` (it needs a real RTX-3060 windowed device). The tester: run it on
//! the GPU, confirm the presented swapchain image matches the rung-10/P1b hybrid golden
//! within `+/-2/255`, confirm — by recording inspection — that NO
//! `copy_image_to_buffer(depth)` and NO per-frame `vkUpdateDescriptorSets` are in the
//! stream, and confirm validation + sync-validation are clean.

#![cfg(windows)]

use core::ptr::NonNull;
use core::slice;

use boyko_rhi::enums::{AddressMode, DescriptorKind, Filter};
use boyko_rhi::{
    BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc, BufferUsage, ComputePipelineDesc,
    Format, GraphicsPipelineDesc, MemoryLocation, PrimitiveTopology, RhiDevice, SamplerDesc,
    ShaderStage, VertexAttribute, VertexBufferLayout, VertexFormat,
};
use boyko_rhi_vulkan::compute::{
    COMPOSITE_PUSH_CONSTANT_BYTES, CompositePushConstants, EDITLIST_BUFFER_WORDS, LOCAL_SIZE_X,
    MESH_COLOR, MESH_DEPTH_CLEAR, SDF_CAMERA_Z, SDF_IMG_H, SDF_IMG_W, SDF_TRACE_T_MAX,
    SDF_VIEW_HALF_EXTENT, SdfEdit, editlist_pixel_hits, encode_edit_list, golden_composite_pixel,
    mesh_depth_for_z, pack_rgba, pixel_world_xy, sdf_gbuffer_composite_spirv, sdf_op,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::ffi::{
    VK_FORMAT_B8G8R8A8_SRGB, VK_FORMAT_B8G8R8A8_UNORM, VK_FORMAT_R8G8B8A8_UNORM, VkExtent2D,
};
use boyko_rhi_vulkan::swapchain::{
    GBUFFER_MVP_BYTES, GBufferFrame, GBufferScene, Renderer, Surface, Swapchain,
};
use boyko_rhi_vulkan::window::Window;

/// The window's client size; the composite is `SDF_IMG_W × SDF_IMG_H` (64×64). The WSI
/// may clamp the swapchain extent wider; the composite present-blits 1:1 in the top-left.
const WIDTH: u32 = SDF_IMG_W;
const HEIGHT: u32 = SDF_IMG_H;

/// Total pixel count (the marcher's dispatch element count; the shader bounds `idx < count`).
const PIXELS: u32 = SDF_IMG_W * SDF_IMG_H;

/// Per-channel tolerance on the packed-RGBA bytes (identical to rung 9/10 / P1b): DXC
/// `mad`/`fma` rounding + the float→UNORM store + the sample round-trip make a bit-exact
/// match brittle; `+/-2/255` still proves the lit SDF / flat mesh / background apart.
const CHANNEL_TOL: i32 = 2;

/// The mesh quad's constant world Z (strictly between the sphere surface and the camera,
/// so the mesh occludes the SDF where they overlap). Mirrors the packed/P1b `MESH_Z`.
const MESH_Z: f32 = 1.0;

/// The mesh quad's world-XY footprint (the left part of the view in x, full y), so the
/// sphere straddles the quad edge. Mirrors the packed/P1b footprint.
const QUAD_X_MIN: f32 = -1.0;
const QUAD_X_MAX: f32 = 0.2;
const QUAD_Y_MIN: f32 = -1.0;
const QUAD_Y_MAX: f32 = 1.0;

/// The depth attachment's CLEAR value (the far plane). Must equal [`MESH_DEPTH_CLEAR`].
const DEPTH_CLEAR: f32 = MESH_DEPTH_CLEAR;

/// The throwaway raster-color format. MUST equal the recorder's
/// `GBUFFER_RASTER_COLOR_FORMAT` (`R8G8B8A8_UNORM`) so the depth-prepass pipeline's
/// declared color format matches the bound throwaway color attachment (W2-b).
const RASTER_COLOR_FORMAT: Format = Format::R8G8B8A8Unorm;

/// One vertex: a `Float32x3` position (offset 0) + a `Float32x4` color (offset 12), the
/// rung-3/4 vertex layout reused. `#[repr(C)]` for the exact 28-byte stride.
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 4],
}

const VERTEX_STRIDE: u32 = core::mem::size_of::<Vertex>() as u32;
const _: () = assert!(VERTEX_STRIDE == 28, "Vertex must be tightly packed at 28 bytes");

/// The MVP byte size (a `float4x4`). Must equal [`GBUFFER_MVP_BYTES`].
const MVP_BYTES: u32 = GBUFFER_MVP_BYTES as u32;

/// A 4-byte-aligned wrapper around a committed SPIR-V byte blob so its address is a
/// valid `*const u32` and it can be re-viewed as a `&[u32]` word stream.
#[repr(C, align(4))]
struct SpirvBlob<const N: usize>([u8; N]);

impl<const N: usize> SpirvBlob<N> {
    /// Re-views the blob as its SPIR-V `u32` word stream.
    fn as_words(&self) -> &[u32] {
        const { assert!(N.is_multiple_of(4), "SPIR-V byte length must be a multiple of 4") };
        // SAFETY: the `align(4)` wrapper makes `self.0`'s address a valid `*const u32`;
        // `N` is a 4-byte multiple (const-asserted); the `&self` borrow keeps the
        // `'static` blob alive for the slice's lifetime; any bit pattern is a valid `u32`.
        unsafe { slice::from_raw_parts(self.0.as_ptr().cast::<u32>(), N / 4) }
    }
}

/// The committed rung-3 vertex SPIR-V (`triangle_mvp.vs.spv`), reused for the prepass.
static MVP_VS_SPV: SpirvBlob<916> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/triangle_mvp.vs.spv"
)));

/// The committed rung-3 fragment SPIR-V (`triangle_mvp.fs.spv`), reused.
static MVP_FS_SPV: SpirvBlob<368> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/triangle_mvp.fs.spv"
)));

/// The committed rung-5 fullscreen vertex SPIR-V (`fullscreen_sample.vs.spv`): a
/// fullscreen triangle generating positions + UVs from `SV_VertexID`.
static SAMPLE_VS_SPV: SpirvBlob<744> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/fullscreen_sample.vs.spv"
)));

/// The committed rung-5 fullscreen fragment SPIR-V (`fullscreen_sample.fs.spv`): samples
/// the bound `Texture2D` + `SamplerState` at the UV and outputs it.
static SAMPLE_FS_SPV: SpirvBlob<764> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/fullscreen_sample.fs.spv"
)));

/// `ceil(PIXELS / LOCAL_SIZE_X)` — the 1D compute dispatch group count.
fn group_count_x() -> u32 {
    PIXELS.div_ceil(LOCAL_SIZE_X)
}

/// Maps the swapchain's `i32` `VkFormat` to "readback bytes are BGRA" (skips an
/// unsupported / SRGB format). Identical to the packed on-screen test.
fn swapchain_readback_is_bgra(vk_format: i32) -> Option<bool> {
    match vk_format {
        f if f == VK_FORMAT_B8G8R8A8_UNORM => Some(true),
        f if f == VK_FORMAT_R8G8B8A8_UNORM => Some(false),
        _ => None,
    }
}

/// The orthographic MVP for the rung-3 vertex shader, uploaded COLUMN-MAJOR (the
/// VERIFIED transpose). Maps a fronto-parallel world vertex so the stored depth is
/// `(CAM_Z - worldZ) / T_MAX`. Mirrors the packed/P1b `ortho_mvp_bytes`.
#[rustfmt::skip]
fn ortho_mvp_bytes() -> [u8; MVP_BYTES as usize] {
    let h = SDF_VIEW_HALF_EXTENT;
    let tmax = SDF_TRACE_T_MAX;
    let cam = SDF_CAMERA_Z;
    let mt: [f32; 16] = [
        1.0 / h, 0.0,      0.0,          0.0,
        0.0,     -1.0 / h, 0.0,          0.0,
        0.0,     0.0,      -1.0 / tmax,  0.0,
        0.0,     0.0,      cam / tmax,   1.0,
    ];
    let mut out = [0u8; MVP_BYTES as usize];
    for (i, f) in mt.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
    }
    out
}

/// The mesh quad as two triangles spanning the world-XY footprint at world Z [`MESH_Z`].
fn quad_vertices() -> [Vertex; 6] {
    let z = MESH_Z;
    let c = [1.0_f32, 1.0, 1.0, 1.0];
    let bl = Vertex { position: [QUAD_X_MIN, QUAD_Y_MIN, z], color: c };
    let br = Vertex { position: [QUAD_X_MAX, QUAD_Y_MIN, z], color: c };
    let tr = Vertex { position: [QUAD_X_MAX, QUAD_Y_MAX, z], color: c };
    let tl = Vertex { position: [QUAD_X_MIN, QUAD_Y_MAX, z], color: c };
    [bl, br, tr, bl, tr, tl]
}

/// Whether pixel `(px, py)`'s orthographic ray passes through the mesh quad footprint
/// (the rasterizer's covered-pixel set, host-computable from the SAME camera mapping).
fn mesh_covers_pixel(px: u32, py: u32) -> bool {
    let [x, y] = pixel_world_xy(px, py);
    (QUAD_X_MIN..=QUAD_X_MAX).contains(&x) && (QUAD_Y_MIN..=QUAD_Y_MAX).contains(&y)
}

/// The per-pixel mesh depth the GPU is expected to produce: the constant inside the
/// quad, the clear value outside.
fn expected_mesh_depth(px: u32, py: u32) -> f32 {
    if mesh_covers_pixel(px, py) {
        mesh_depth_for_z(MESH_Z)
    } else {
        DEPTH_CLEAR
    }
}

/// The base-sphere SDF scene (one union sphere, origin, r=0.5) — the recognizable SDF
/// body the mesh occludes (the packed/P1b `sphere_scene`).
fn sphere_scene() -> Vec<SdfEdit> {
    vec![SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0)]
}

/// Writes `words` `u32`s into a buffer's persistent host-coherent mapping (the CPU seeds
/// the edit-list header before submit).
fn write_words(base: NonNull<u8>, words: &[u32]) {
    let dst = base.as_ptr().cast::<u32>();
    for (i, &w) in words.iter().enumerate() {
        // SAFETY: the buffer is at least `words.len() * 4` bytes inside the persistent
        // host-coherent mapping; `dst + i` for `i < words.len()` is in-bounds. No GPU
        // work is in flight yet (the present loop follows), so the host write is
        // unsynchronized-safe. `write_unaligned` tolerates the sub-allocated offset.
        unsafe { dst.add(i).write_unaligned(w) };
    }
}

/// Splits a packed `0xAABBGGRR` into `[r, g, b]` (the low three bytes).
fn unpack_rgb(packed: u32) -> [i32; 3] {
    [
        (packed & 0xFF) as i32,
        ((packed >> 8) & 0xFF) as i32,
        ((packed >> 16) & 0xFF) as i32,
    ]
}

/// Decodes one readback texel into `[r, g, b]`, applying the swapchain channel order.
fn readback_rgb(texel: [u8; 4], is_bgra: bool) -> [i32; 3] {
    if is_bgra {
        [texel[2] as i32, texel[1] as i32, texel[0] as i32]
    } else {
        [texel[0] as i32, texel[1] as i32, texel[2] as i32]
    }
}

/// `true` if a readback texel agrees with a golden packed `0xAABBGGRR` within
/// `CHANNEL_TOL` per RGB channel (swapchain-byte-order-aware).
fn readback_close(texel: [u8; 4], golden: u32, is_bgra: bool) -> bool {
    let g = readback_rgb(texel, is_bgra);
    let w = unpack_rgb(golden);
    (0..3).all(|c| (g[c] - w[c]).abs() <= CHANNEL_TOL)
}

/// Asserts a readback texel agrees with a golden packed color (swapchain-order-aware).
fn assert_readback_close(texel: [u8; 4], golden: u32, is_bgra: bool, label: &str) {
    assert!(
        readback_close(texel, golden, is_bgra),
        "{label}: readback {texel:02x?} (bgra={is_bgra}) != golden {golden:#010x} -> {:?} within +/-{CHANNEL_TOL}",
        unpack_rgb(golden),
    );
}

/// `true` if two golden packed colors agree within `CHANNEL_TOL` per RGB channel.
fn goldens_close(a: u32, b: u32) -> bool {
    let x = unpack_rgb(a);
    let y = unpack_rgb(b);
    (0..3).all(|c| (x[c] - y[c]).abs() <= CHANNEL_TOL)
}

/// The byte index of texel `(x, y)` in a tightly-packed 4-byte/texel readback of a
/// `w`-wide image.
fn texel_base(x: u32, y: u32, w: u32) -> usize {
    ((y * w + x) * 4) as usize
}

/// Scans for the first pixel matching `pred(sphere_hit, mesh_covered)`.
fn find_texel(edits: &[SdfEdit], pred: impl Fn(bool, bool) -> bool) -> Option<(u32, u32)> {
    for py in 0..SDF_IMG_H {
        for px in 0..SDF_IMG_W {
            let hit = editlist_pixel_hits(edits, px, py);
            let covered = mesh_covers_pixel(px, py);
            if pred(hit, covered) {
                return Some((px, py));
            }
        }
    }
    None
}

#[test]
fn windowed_gbuffer_composite_present_is_validation_clean_and_renders_composite() {
    // Open the window first — the surface borrows its HWND/HINSTANCE and must be
    // destroyed before it.
    let mut window = match Window::open("boyko_rhi_vulkan gbuffer window", WIDTH, HEIGHT) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("SKIP windowed_gbuffer_present: cannot open a window ({e:?})");
            return;
        }
    };

    let ctx = match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        windowed: true,
    }) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP windowed_gbuffer_present: windowed Vulkan unavailable ({e:?})");
            return;
        }
    };
    println!("Vulkan device (windowed, validation on): {}", ctx.device_name());
    assert!(ctx.validation_enabled(), "validation must be active");
    let caps = ctx.device_caps();
    assert!(
        caps.gbuffer_storage_format_ok,
        "a booted context must support STORAGE_IMAGE on the G-buffer format"
    );

    // SAFETY: `window` outlives the surface (dropped after it below); its
    // HWND/HINSTANCE are live for the surface's lifetime.
    let surface = match unsafe { Surface::new(&ctx, window.hinstance(), window.hwnd()) } {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP windowed_gbuffer_present: surface creation failed ({e:?})");
            return;
        }
    };

    let mut swapchain = match Swapchain::new(&ctx, &surface, window.width(), window.height()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP windowed_gbuffer_present: swapchain creation failed ({e:?})");
            return;
        }
    };
    assert!(swapchain.image_count() >= 1, "swapchain must expose >= 1 image");
    println!(
        "swapchain: {} images, extent {}x{}, format {}",
        swapchain.image_count(),
        swapchain.extent().width,
        swapchain.extent().height,
        swapchain.format()
    );

    // The composite present-blits 1:1 in the swapchain image's TOP-LEFT 64×64 sub-rect.
    // A WSI clamp WIDER than 64×64 is fine (the rest stays clear); SMALLER than 64 in
    // either dimension clips the composite → a graceful SKIP (the rung-11 handling).
    if swapchain.extent().width < SDF_IMG_W || swapchain.extent().height < SDF_IMG_H {
        eprintln!(
            "SKIP windowed_gbuffer_present: swapchain extent {}x{} is smaller than the {}x{} \
             composite, so the top-left 1:1 sub-rect does not fit",
            swapchain.extent().width,
            swapchain.extent().height,
            SDF_IMG_W,
            SDF_IMG_H
        );
        return;
    }

    let Some(is_bgra) = swapchain_readback_is_bgra(swapchain.format()) else {
        eprintln!(
            "SKIP windowed_gbuffer_present: swapchain format {} (e.g. {VK_FORMAT_B8G8R8A8_SRGB} SRGB) \
             has no host-decodable UNORM byte order",
            swapchain.format()
        );
        return;
    };

    // The present-blit pipeline declares the swapchain's color format (W2-b).
    let Some(swap_color_format) = (match swapchain.format() {
        f if f == VK_FORMAT_B8G8R8A8_UNORM => Some(Format::B8G8R8A8Unorm),
        f if f == VK_FORMAT_R8G8B8A8_UNORM => Some(Format::R8G8B8A8Unorm),
        _ => None,
    }) else {
        eprintln!("SKIP windowed_gbuffer_present: swapchain format has no basic-slice Format variant");
        return;
    };

    let mut renderer =
        Renderer::new(&ctx, &surface, &swapchain).expect("renderer (command pool + sync) creation");

    let device: &VulkanContext = &ctx;
    let sdf = sphere_scene();

    // --- Pick the three discriminator texels host-side, BEFORE any GPU run. ---
    let (ax, ay) = find_texel(&sdf, |hit, covered| hit && covered)
        .expect("invariant: some pixel must be over BOTH the sphere and the quad (mesh-occludes-SDF)");
    let (bx, by) = find_texel(&sdf, |hit, covered| hit && !covered)
        .expect("invariant: some pixel must be over the sphere but NOT the quad (SDF)");
    let (dx, dy) = find_texel(&sdf, |hit, covered| !hit && !covered)
        .expect("invariant: some pixel must be over neither (background)");

    let depth_at = |px, py| expected_mesh_depth(px, py);
    let a_want = golden_composite_pixel(&sdf, depth_at(ax, ay), ax, ay);
    let b_want = golden_composite_pixel(&sdf, depth_at(bx, by), bx, by);
    let d_want = golden_composite_pixel(&sdf, depth_at(dx, dy), dx, dy);

    let mesh_packed = pack_rgba(MESH_COLOR);
    assert!(
        goldens_close(a_want, mesh_packed),
        "invariant: the mesh-occludes-SDF golden must equal MESH_COLOR"
    );
    assert!(
        !goldens_close(a_want, b_want),
        "invariant: MESH_COLOR and the SDF lit color must differ beyond +/-{CHANNEL_TOL}"
    );
    assert!(
        !goldens_close(a_want, d_want),
        "invariant: MESH_COLOR and BACKGROUND must differ beyond +/-{CHANNEL_TOL}"
    );
    assert!(
        !goldens_close(b_want, d_want),
        "invariant: the SDF lit color and BACKGROUND must differ beyond +/-{CHANNEL_TOL}"
    );

    // === Build the P1c on-screen G-buffer scene's STATIC inputs (the GBufferScene). ===

    // The edit-list StorageBuffer (binding 0), host-seeded ONCE. Over-allocated to the
    // full `EDITLIST_BUFFER_WORDS` (the encoder debug-asserts that size); the marcher
    // only reads the header + edit array.
    let edit_list = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("edit-list storage buffer");
    {
        let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
        encode_edit_list(&mut header, &sdf);
        let mapped = RhiDevice::buffer_mapped_ptr(device, &edit_list)
            .expect("host-visible edit-list buffer is mapped");
        write_words(mapped, &header);
    }

    // The camera/extent UNIFORM buffer (binding 5), host-seeded ONCE at the golden 64×64
    // ORTHO extent (the composite size — NOT the swapchain extent) for bit-exact rays.
    let camera_uniform = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: COMPOSITE_PUSH_CONSTANT_BYTES as u64,
            usage: BufferUsage::UNIFORM,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("camera uniform buffer");
    {
        let pc = CompositePushConstants::ortho(SDF_IMG_W, SDF_IMG_H);
        assert_eq!(pc.count, PIXELS);
        let mapped = RhiDevice::buffer_mapped_ptr(device, &camera_uniform)
            .expect("host-visible uniform buffer is mapped");
        let bytes = pc.as_bytes();
        // SAFETY: `mapped` points to `COMPOSITE_PUSH_CONSTANT_BYTES` mapped host-coherent
        // bytes; `bytes` is exactly that many bytes; no GPU work is in flight yet (the
        // present loop follows), so the write is unsynchronized-safe.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
        }
    }

    // The mesh quad's vertex buffer (host-visible).
    let vertices = quad_vertices();
    let vertex_bytes = core::mem::size_of_val(&vertices) as u64;
    let vertex_buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: vertex_bytes,
            usage: BufferUsage::VERTEX,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible vertex buffer");
    {
        let vb_ptr = RhiDevice::buffer_mapped_ptr(device, &vertex_buffer)
            .expect("host-visible vertex buffer is mapped");
        // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes;
        // `vertices` is a distinct stack array of `vertex_bytes` bytes; the write
        // completes before any submit references the buffer (host-coherent: no flush).
        unsafe {
            core::ptr::copy_nonoverlapping(
                vertices.as_ptr().cast::<u8>(),
                vb_ptr.as_ptr(),
                vertex_bytes as usize,
            );
        }
    }

    // The depth sampler (bound at vocab binding 1; ignored by the marcher's `.Load`).
    let depth_sampler = RhiDevice::create_sampler(device, &SamplerDesc::default())
        .expect("depth sampler (ignored by .Load)");
    // The present-blit sampler (nearest/clamp for a 1:1 albedo sample).
    let present_sampler = RhiDevice::create_sampler(
        device,
        &SamplerDesc {
            mag_filter: Filter::Nearest,
            min_filter: Filter::Nearest,
            address_mode: AddressMode::ClampToEdge,
        },
    )
    .expect("present nearest/clamp sampler");

    // The depth-prepass graphics pipeline (rung-3 vertex layout + 64-byte VERTEX MVP +
    // the throwaway color format + a declared depth format).
    let vs = RhiDevice::create_shader_module(device, MVP_VS_SPV.as_words())
        .expect("prepass vertex shader module");
    let fs = RhiDevice::create_shader_module(device, MVP_FS_SPV.as_words())
        .expect("prepass fragment shader module");
    let attributes = [
        VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 1, offset: 12, format: VertexFormat::Float32x4 },
    ];
    let raster_pipeline = RhiDevice::create_graphics_pipeline(
        device,
        &GraphicsPipelineDesc {
            vertex_module: &vs,
            vertex_entry: c"main",
            fragment_module: &fs,
            fragment_entry: c"main",
            color_formats: &[RASTER_COLOR_FORMAT],
            depth_format: Some(Format::D32Sfloat),
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: Some(VertexBufferLayout {
                stride: VERTEX_STRIDE,
                attributes: &attributes,
            }),
            push_constant_bytes: MVP_BYTES,
            bind_group_layout: None,
        },
    )
    .expect("depth-prepass graphics pipeline");

    // The P1b marcher: the vocabulary bind-group layout + the compute pipeline.
    let cs = RhiDevice::create_shader_module(device, sdf_gbuffer_composite_spirv())
        .expect("P1b G-buffer marcher compute shader module");
    let vocab_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::SampledImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
    ];
    let vocab_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc { entries: &vocab_entries },
    )
    .expect("P1b vocabulary bind-group layout");
    let marcher = RhiDevice::create_compute_pipeline(
        device,
        &ComputePipelineDesc {
            module: &cs,
            entry: c"main",
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&vocab_layout),
        },
    )
    .expect("P1b G-buffer marcher compute pipeline");

    // The present-blit: one COMBINED_IMAGE_SAMPLER layout + the fullscreen-sample pipeline.
    let present_layout = RhiDevice::create_bind_group_layout(
        device,
        &BindGroupLayoutDesc {
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                count: 1,
                kind: DescriptorKind::CombinedImageSampler,
                stage: ShaderStage::FRAGMENT,
            }],
        },
    )
    .expect("present-blit bind-group layout (one COMBINED_IMAGE_SAMPLER)");
    let sample_vs = RhiDevice::create_shader_module(device, SAMPLE_VS_SPV.as_words())
        .expect("fullscreen vertex shader module");
    let sample_fs = RhiDevice::create_shader_module(device, SAMPLE_FS_SPV.as_words())
        .expect("fullscreen fragment shader module");
    let present_pipeline = RhiDevice::create_graphics_pipeline(
        device,
        &GraphicsPipelineDesc {
            vertex_module: &sample_vs,
            vertex_entry: c"main",
            fragment_module: &sample_fs,
            fragment_entry: c"main",
            color_formats: &[swap_color_format],
            depth_format: None,
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: None,
            push_constant_bytes: 0,
            bind_group_layout: Some(&present_layout),
        },
    )
    .expect("present-blit fullscreen-sample pipeline (swapchain color format)");

    // The shader modules are consumed by pipeline creation; destroy them now.
    // SAFETY: every module was created on `ctx` above + is no longer needed once its
    // pipeline is created (the pipeline holds its own compiled code); each is destroyed
    // exactly once.
    unsafe {
        RhiDevice::destroy_shader_module(device, sample_fs);
        RhiDevice::destroy_shader_module(device, sample_vs);
        RhiDevice::destroy_shader_module(device, cs);
        RhiDevice::destroy_shader_module(device, fs);
        RhiDevice::destroy_shader_module(device, vs);
    }

    let mvp = ortho_mvp_bytes();
    let scene = GBufferScene {
        raster_pipeline: &raster_pipeline,
        vertex_buffer: &vertex_buffer,
        vertex_count: vertices.len() as u32,
        mvp,
        marcher: &marcher,
        vocab_layout: &vocab_layout,
        edit_list: &edit_list,
        camera_uniform: &camera_uniform,
        depth_sampler: &depth_sampler,
        present_pipeline: &present_pipeline,
        present_layout: &present_layout,
        present_sampler: &present_sampler,
        dispatch_group_count_x: group_count_x(),
    };

    // The composite's native size — drives the G-buffer alloc + the 1:1 top-left present.
    let present_extent = VkExtent2D { width: SDF_IMG_W, height: SDF_IMG_H };

    // A host-visible staging buffer sized for one full swapchain image (4 B/texel).
    let staging_size = (swapchain.extent().width * swapchain.extent().height * 4) as u64;
    let staging = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: staging_size,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("host-visible readback staging buffer");
    let alloc_extent = swapchain.extent();

    let mut frame = GBufferFrame::new();

    // --- Present the image-based composite for a handful of frames; request the
    //     swapchain-image readback on ONE presented frame. ---
    let clear = [0.0_f32, 0.0, 0.0, 1.0];
    let mut readback_done = false;
    let mut readback_extent = swapchain.extent();
    for i in 0..5u32 {
        window.pump_events();
        window.refresh_size();

        // Request the readback on a single steady frame, only while the live extent
        // still matches the staging-buffer size (a resize simply skips the golden).
        let live = swapchain.extent();
        let extent_stable = live.width == alloc_extent.width && live.height == alloc_extent.height;
        let want_readback = i == 2 && !readback_done && extent_stable;
        let rb = if want_readback { Some(&staging) } else { None };

        // SAFETY: `ctx`/`surface`/`swapchain` are live + created on the same device as
        // `renderer`; every `scene` resource is live on this device; `edit_list` /
        // `camera_uniform` were host-seeded once + are never written again (the marcher
        // only reads them); `frame`'s targets are synced to `present_extent` by the call;
        // `scene.dispatch_group_count_x` + the camera UBO's `count` were sized to the
        // composite extent; a `Some(rb)` staging buffer is host-visible + `staging_size`
        // (>= one swapchain image) bytes.
        let presented = unsafe {
            renderer.render_gbuffer_frame(
                &ctx,
                &surface,
                &mut swapchain,
                &scene,
                &mut frame,
                window.width(),
                window.height(),
                clear,
                present_extent,
                rb,
            )
        }
        .unwrap_or_else(|e| panic!("gbuffer present frame {i} failed: {e:?}"));

        if want_readback && presented {
            readback_done = true;
            readback_extent = swapchain.extent();
        }
    }

    // The oracle: a clean windowed image-based present records zero validation messages.
    let state = ctx
        .debug_state()
        .expect("validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during the windowed G-buffer present — \
         see the [vk-validation] log",
        state.total()
    );

    // The golden: if a readback frame presented, the three discriminator texels must
    // match the host composite truth (swapchain byte-order-aware) — PROVING the
    // image-based composite reached the swapchain image with correct colors, equal to
    // the packed on-screen composite within +/-2/255.
    if readback_done {
        let w = readback_extent.width;
        let h = readback_extent.height;
        let dst_ptr = RhiDevice::buffer_mapped_ptr(device, &staging)
            .expect("host-visible staging buffer is mapped");
        let byte_count = (w * h * 4) as usize;
        let mut out = vec![0u8; byte_count];
        // SAFETY: `dst_ptr` points to `staging_size` (>= `byte_count`) mapped
        // host-coherent bytes; the readback frame's submit completed before this read
        // (the renderer fence-waits the frame slot at the START of each subsequent
        // `render_gbuffer_frame`, and frames followed frame 2, so frame 2's copy is
        // complete + coherent); `out` is a distinct, non-overlapping alloc.
        unsafe {
            core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), byte_count);
        }

        let read_texel = |px: u32, py: u32| -> [u8; 4] {
            let b = texel_base(px, py, w);
            [out[b], out[b + 1], out[b + 2], out[b + 3]]
        };

        let a_got = read_texel(ax, ay);
        let b_got = read_texel(bx, by);
        let d_got = read_texel(dx, dy);

        assert_readback_close(a_got, a_want, is_bgra, "mesh-occludes-SDF texel (MESH_COLOR)");
        assert_readback_close(b_got, b_want, is_bgra, "SDF texel (lit color)");
        assert_readback_close(d_got, d_want, is_bgra, "background texel");

        // The occlusion actually changed the on-screen pixel: the mesh-occludes-SDF
        // texel and the SDF texel are BOTH over the sphere, yet must differ.
        let a_rgb = readback_rgb(a_got, is_bgra);
        let b_rgb = readback_rgb(b_got, is_bgra);
        assert!(
            (0..3).any(|c| (a_rgb[c] - b_rgb[c]).abs() > CHANNEL_TOL),
            "the on-screen mesh-occluded texel {a_got:02x?} and the SDF-visible texel {b_got:02x?} \
             (both over the sphere) must differ — proving the image-based hybrid composite (not a clear) reached the screen"
        );
    } else {
        eprintln!(
            "NOTE windowed_gbuffer_present: no readback frame presented (swapchain kept recreating); \
             validation was still asserted clean across all frames"
        );
    }

    // Clean reverse-order teardown: renderer (waits idle) → the per-extent G-buffer
    // frame → the scene's static resources → swapchain → surface → window.
    drop(renderer);
    // SAFETY: the renderer was dropped above (its `Drop` waits the device idle), so no
    // submission references these resources; `ctx` is still alive; each is destroyed
    // exactly once, in reverse dependency order.
    unsafe {
        frame.destroy(&ctx);
        RhiDevice::destroy_buffer(device, staging);
        RhiDevice::destroy_graphics_pipeline(device, present_pipeline);
        RhiDevice::destroy_bind_group_layout(device, present_layout);
        RhiDevice::destroy_compute_pipeline(device, marcher);
        RhiDevice::destroy_bind_group_layout(device, vocab_layout);
        RhiDevice::destroy_graphics_pipeline(device, raster_pipeline);
        RhiDevice::destroy_sampler(device, present_sampler);
        RhiDevice::destroy_sampler(device, depth_sampler);
        RhiDevice::destroy_buffer(device, vertex_buffer);
        RhiDevice::destroy_buffer(device, camera_uniform);
        RhiDevice::destroy_buffer(device, edit_list);
    }
    drop(swapchain);
    drop(surface);
    drop(ctx);
    drop(window);
}
