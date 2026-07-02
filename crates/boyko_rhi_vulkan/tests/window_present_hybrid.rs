//! Phase-6 Slice S1 — RUNG 11 acceptance: the FIRST HYBRID FRAME ON SCREEN. The
//! rung-10 SDF + MESH HYBRID COMPOSITE (a real GPU-rasterized mesh's depth bounds a
//! GPU SDF sphere-trace, composited into one packed-RGBA buffer) is routed to the
//! windowed swapchain image via a FULLSCREEN-SAMPLE pass and PRESENTED, with the
//! validation layer as the soundness oracle. On ONE presented frame — before
//! present — the swapchain image is copied back into a host-visible staging buffer
//! and golden-asserted against the host composite truth ([`golden_composite_pixel`]).
//!
//! # Route B (texture + fullscreen-sample, NOT a raw buffer→swapchain copy)
//!
//! The composite buffer holds packed `0xAABBGGRR` RGBA. A raw byte copy of those
//! bytes into a `B8G8R8A8_UNORM` swapchain image would swap R/B on screen. Instead
//! the composite pixel region is uploaded (`copy_buffer_to_image`) into an
//! `R8G8B8A8_UNORM` SAMPLED texture; a fullscreen-sample graphics pass samples that
//! texture (normalized float RGBA) and writes it into the swapchain image, so the
//! GPU converts RGBA → the swapchain's format on the attachment write. On-screen
//! colors are therefore correct on ANY swapchain format. This reuses rungs 5 (the
//! sampler/bind-group/fullscreen pipeline) + 2 (the draw) + 7 (the present loop +
//! readback golden) wholesale.
//!
//! # The two fenced submits the test drives
//!
//! 1. **Composite** (the rung-10 shared-depth pass, factored into [`run_composite`]):
//!    raster the quad into a depth attachment → copy depth into the shared buffer →
//!    transfer→compute barrier → SDF sphere-trace bounded by that depth → the
//!    packed-RGBA composite lands in the buffer's PIXEL region.
//! 2. **Present** ([`Renderer::present_sampled`]): upload the pixel region into the
//!    SAMPLED texture → fullscreen-sample it into the acquired swapchain image →
//!    present; on the readback frame, also copy the swapchain image back for the
//!    golden.
//!
//! # The discriminator texels (picked host-side, BEFORE any GPU run)
//!
//! The same three rung-10 regions: a mesh-occludes-SDF texel (`MESH_COLOR`), an SDF
//! texel (the lit color), and a background texel — each asserted color-close to
//! [`golden_composite_pixel`] within `+/-2/255`, accounting for the swapchain being
//! `B8G8R8A8` (the readback bytes are then BGRA; the golden is RGBA byte order).
//!
//! The composite presents 1:1 in the swapchain image's TOP-LEFT 64×64 sub-rect
//! (the WSI may clamp the swapchain extent wider than the requested 64×64). The
//! discriminator texels are at composite coords `(px, py) < 64`, which land in that
//! top-left region, so they are read back from the full swapchain image at
//! `py * live_extent_width + px` (live stride, top-left sub-rect) — making the
//! composite-space golden exact regardless of the WSI extent clamp.
//!
//! # CI gate (graceful skip)
//!
//! Mirrors `window_present_scene.rs`: no window / no Vulkan loader / no GPU / no
//! validation SDK / no WSI / no dynamic rendering → a SKIP. The test is
//! `#[cfg(windows)]`; on other targets it is a trivial pass.

#![cfg(windows)]

use core::ptr::NonNull;
use core::slice;

use boyko_rhi::enums::{AddressMode, BarrierAccess, BarrierStage, DescriptorKind, Filter};
use boyko_rhi::{
    BarrierDesc, BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry,
    BufferBarrier, BufferDesc, BufferImageCopy, BufferUsage, ComputePipelineDesc, DepthAttachment,
    Format, CullMode, GraphicsPipelineDesc, ImageAspect, ImageBarrierDesc, ImageLayout, ImageSubresourceRange,
    ImageUsage, LoadOp, MemoryLocation, MipMode, PrimitiveTopology, RenderArea, RenderingAttachment,
    RenderingDesc, RhiCommandEncoder, RhiDevice, RhiQueue, SamplerDesc, ShaderStage, StoreOp,
    TextureDesc, TextureDimension, VertexAttribute, VertexBufferLayout, VertexFormat, Viewport,
};
use boyko_rhi_vulkan::compute::{COMPOSITE_BUFFER_WORDS, COMPOSITE_DEPTH_BASE_WORDS, COMPOSITE_PIXEL_BASE_WORDS, COMPOSITE_PUSH_CONSTANT_BYTES, CompositePushConstants, LOCAL_SIZE_X, MESH_COLOR, MESH_DEPTH_CLEAR, SDF_CAMERA_Z, SDF_IMG_H, SDF_IMG_W, SDF_TRACE_T_MAX, SDF_VIEW_HALF_EXTENT, SdfEdit, editlist_pixel_hits, encode_edit_list, mesh_depth_for_z, pack_rgba, pixel_world_xy, sdf_depth_composite_spirv, sdf_op};
use boyko_rhi_vulkan::goldens::{golden_composite_pixel};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::ffi::{
    VK_FORMAT_B8G8R8A8_SRGB, VK_FORMAT_B8G8R8A8_UNORM, VK_FORMAT_R8G8B8A8_UNORM, VkExtent2D,
};
use boyko_rhi_vulkan::swapchain::{Renderer, SampledComposite, Surface, Swapchain};
use boyko_rhi_vulkan::window::Window;

/// The window's client size. The composite image is `SDF_IMG_W × SDF_IMG_H` (64×64);
/// the window is opened at the same size. The WSI may CLAMP the swapchain extent
/// wider/taller than the requested size (a driver-minimum surface extent); the
/// composite is presented at its NATIVE 64×64 size in the TOP-LEFT of the swapchain
/// image (a 1:1 mapping, no scaling — [`Renderer::present_sampled`] clamps the present
/// viewport/scissor to `min(swapchain_extent, texture_extent)`), so the per-texel
/// golden is exact regardless of the clamp, as long as the swapchain is AT LEAST 64×64
/// (the top-left 64×64 sub-rect fits).
const WIDTH: u32 = SDF_IMG_W;
const HEIGHT: u32 = SDF_IMG_H;

/// Total pixel count (the compute push constant; the shader bounds `idx < count`).
const PIXELS: u32 = SDF_IMG_W * SDF_IMG_H;

/// Per-channel tolerance on the packed-RGBA bytes (identical to rung 9/10): DXC
/// `mad`/`fma` rounding plus the float→UNORM sample round-trip make a bit-exact match
/// brittle; `+/-2/255` still proves the lit SDF surface / flat mesh / background
/// colors apart (they differ by 100+).
const CHANNEL_TOL: i32 = 2;

/// The mesh quad's constant world Z (rung-10: strictly between the sphere surface and
/// the camera, so the mesh occludes the SDF where they overlap).
const MESH_Z: f32 = 1.0;

/// The mesh quad's world-XY footprint corners (rung-10: the left ~60% of the view in
/// x, full y), straddling the sphere so there are mesh-occludes-SDF / SDF-only /
/// mesh-on-background / background pixels.
const QUAD_X_MIN: f32 = -1.0;
const QUAD_X_MAX: f32 = 0.2;
const QUAD_Y_MIN: f32 = -1.0;
const QUAD_Y_MAX: f32 = 1.0;

/// The depth attachment's CLEAR value (the far plane). Must equal [`MESH_DEPTH_CLEAR`].
const DEPTH_CLEAR: f32 = MESH_DEPTH_CLEAR;

/// A throwaway color-attachment format for the mesh raster pass (its result is never
/// read; only the DEPTH is consumed).
const RASTER_COLOR_FORMAT: Format = Format::R8G8B8A8Unorm;

/// One vertex: a `Float32x3` position (offset 0) + a `Float32x4` color (offset 12),
/// the rung-3/4 vertex layout reused. `#[repr(C)]` so the field layout is the exact
/// 28-byte stride the layout declares.
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 4],
}

const VERTEX_STRIDE: u32 = core::mem::size_of::<Vertex>() as u32;
const _: () = assert!(VERTEX_STRIDE == 28, "Vertex must be tightly packed at 28 bytes");

/// The MVP byte size (a `float4x4`).
const MVP_BYTES: u32 = 64;

// --- Committed SPIR-V (a `#[repr(C, align(4))]` byte wrapper re-viewed as a `&[u32]`
//     word stream). The mesh raster reuses rung-3 MVP vertex+fragment; the composite
//     reuses the rung-10 compute; the present reuses the rung-5 fullscreen sample. ---

/// A 4-byte-aligned wrapper around a committed SPIR-V byte blob so its address is a
/// valid `*const u32` and it can be re-viewed as a `&[u32]` word stream.
#[repr(C, align(4))]
struct SpirvBlob<const N: usize>([u8; N]);

impl<const N: usize> SpirvBlob<N> {
    /// Re-views the blob as its SPIR-V `u32` word stream.
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
static MVP_VS_SPV: SpirvBlob<916> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/triangle_mvp.vs.spv"
)));

/// The committed rung-3 fragment SPIR-V (`triangle_mvp.fs.spv`, 368 bytes), reused.
static MVP_FS_SPV: SpirvBlob<368> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/triangle_mvp.fs.spv"
)));

/// The committed rung-5 full-screen vertex SPIR-V (`fullscreen_sample.vs.spv`, 744
/// bytes): a full-screen triangle generating positions + UVs from `SV_VertexID`.
static SAMPLE_VS_SPV: SpirvBlob<744> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/fullscreen_sample.vs.spv"
)));

/// The committed rung-5 full-screen fragment SPIR-V (`fullscreen_sample.fs.spv`, 764
/// bytes): samples the bound `Texture2D`+`SamplerState` at the UV and outputs it.
static SAMPLE_FS_SPV: SpirvBlob<764> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/fullscreen_sample.fs.spv"
)));

/// `ceil(PIXELS / LOCAL_SIZE_X)` — the 1D compute dispatch group count.
fn group_count_x() -> u32 {
    PIXELS.div_ceil(LOCAL_SIZE_X)
}

/// Maps the swapchain's `i32` `VkFormat` to the boolean "readback bytes are BGRA"
/// (and skips an unsupported/SRGB format). `B8G8R8A8_UNORM` → `true` (the readback
/// texel is `[B, G, R, A]`); `R8G8B8A8_UNORM` → `false` (`[R, G, B, A]`). Only the
/// two UNORM formats `pick_surface_format` selects have a host-decodable byte order
/// here; an SRGB swapchain (which would gamma-encode the sampled floats) is skipped.
fn swapchain_readback_is_bgra(vk_format: i32) -> Option<bool> {
    match vk_format {
        f if f == VK_FORMAT_B8G8R8A8_UNORM => Some(true),
        f if f == VK_FORMAT_R8G8B8A8_UNORM => Some(false),
        _ => None,
    }
}

/// The orthographic MVP for the rung-3 vertex shader (rung-10's `ortho_mvp_bytes`,
/// uploaded COLUMN-MAJOR so the GPU evaluates `clip = M * p`). Maps a fronto-parallel
/// world vertex to clip space so the stored depth equals `t / T_MAX` with
/// `t = CAM_Z - worldZ`. See `sdf_mesh_hybrid_depth.rs` for the full derivation.
#[rustfmt::skip]
fn ortho_mvp_bytes() -> [u8; MVP_BYTES as usize] {
    let h = SDF_VIEW_HALF_EXTENT;
    let tmax = SDF_TRACE_T_MAX;
    let cam = SDF_CAMERA_Z;
    // Mᵀ in row-major upload order: mt[r*4 + c] = M[c][r]. Each group of 4 is a
    // COLUMN of `M` (so the only off-diagonal term `CAM_Z/T_MAX` is the 15th float).
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

/// The mesh quad as two triangles (six vertices) at constant world Z [`MESH_Z`]
/// (rung-10's `quad_vertices`). The per-vertex color is arbitrary (unused by the
/// composite).
fn quad_vertices() -> [Vertex; 6] {
    let z = MESH_Z;
    let c = [1.0_f32, 1.0, 1.0, 1.0];
    let bl = Vertex { position: [QUAD_X_MIN, QUAD_Y_MIN, z], color: c };
    let br = Vertex { position: [QUAD_X_MAX, QUAD_Y_MIN, z], color: c };
    let tr = Vertex { position: [QUAD_X_MAX, QUAD_Y_MAX, z], color: c };
    let tl = Vertex { position: [QUAD_X_MIN, QUAD_Y_MAX, z], color: c };
    [bl, br, tr, bl, tr, tl]
}

/// Whether pixel `(px, py)`'s orthographic ray passes through the mesh quad's
/// world-XY footprint (rung-10's `mesh_covers_pixel` — exactly the rasterizer's
/// covered-pixel set, host-computable from the SAME camera mapping the golden uses).
fn mesh_covers_pixel(px: u32, py: u32) -> bool {
    let [x, y] = pixel_world_xy(px, py);
    (QUAD_X_MIN..=QUAD_X_MAX).contains(&x) && (QUAD_Y_MIN..=QUAD_Y_MAX).contains(&y)
}

/// The per-pixel mesh DEPTH the GPU is expected to produce: the constant
/// [`mesh_depth_for_z`]`(MESH_Z)` inside the quad footprint, the clear value outside.
fn expected_mesh_depth(px: u32, py: u32) -> f32 {
    if mesh_covers_pixel(px, py) {
        mesh_depth_for_z(MESH_Z)
    } else {
        DEPTH_CLEAR
    }
}

/// The base-sphere SDF scene (one union sphere, origin, r=0.5) — the rung-9/10
/// `base_only` field, the recognizable SDF body the mesh occludes.
fn sphere_scene() -> Vec<SdfEdit> {
    vec![SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0)]
}

/// Writes `words` `u32`s into a buffer's persistent host-coherent mapping (the CPU
/// seeds the edit-list header before submit).
fn write_words(base: NonNull<u8>, words: &[u32]) {
    let dst = base.as_ptr().cast::<u32>();
    for (i, &w) in words.iter().enumerate() {
        // SAFETY: the buffer is `COMPOSITE_BUFFER_WORDS * 4` bytes inside the
        // persistent host-coherent mapping; `dst + i` for `i < words.len() <=
        // COMPOSITE_BUFFER_WORDS` is in-bounds. No GPU work is in flight yet (the
        // submit happens after this), so the host write is unsynchronized-safe.
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

/// Decodes one readback texel (`[c0, c1, c2, c3]`) into `[r, g, b]`, applying the
/// swapchain channel order: BGRA reads `[B, G, R, A]` (R = `c2`, G = `c1`, B = `c0`);
/// RGBA reads `[R, G, B, A]` (R = `c0`, G = `c1`, B = `c2`).
fn readback_rgb(texel: [u8; 4], is_bgra: bool) -> [i32; 3] {
    if is_bgra {
        [texel[2] as i32, texel[1] as i32, texel[0] as i32]
    } else {
        [texel[0] as i32, texel[1] as i32, texel[2] as i32]
    }
}

/// `true` if a readback texel agrees with a golden packed `0xAABBGGRR` within
/// `CHANNEL_TOL` per RGB channel (accounting for the swapchain byte order).
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

/// Scans for the first pixel matching `pred(sphere_hit, mesh_covered)` (rung-10's
/// `find_texel`).
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

/// Runs the rung-10 shared-depth hybrid composite in ONE fenced submit, leaving the
/// packed-RGBA composite in `buffer`'s PIXEL region. The flow is identical to
/// `sdf_mesh_hybrid_depth.rs::run_hybrid` (raster quad → copy depth into the shared
/// buffer → transfer→compute barrier → SDF sphere-trace bounded by that depth) — the
/// rung-10 test + asserts are untouched; this is a parallel driver that KEEPS the
/// composite buffer (the caller presents it) rather than destroying everything.
fn run_composite(device: &VulkanContext, edits: &[SdfEdit], buffer: &boyko_rhi_vulkan::memory::BoundBuffer) {
    let queue = device.rhi_queue();

    // Seed the edit-list header BEFORE submit (depth + pixel regions are GPU-written).
    {
        let mut header = vec![0u32; COMPOSITE_BUFFER_WORDS];
        encode_edit_list(&mut header, edits);
        let mapped = device
            .buffer_mapped_ptr(buffer)
            .expect("host-visible composite buffer is mapped");
        write_words(mapped, &header);
    }

    // The depth image (D32_SFLOAT): DEPTH_STENCIL_ATTACHMENT (rasterize into it) |
    // TRANSFER_SRC (copy into the shared buffer).
    let depth = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: Format::D32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::TRANSFER_SRC,
            array_layers: 1,
        })
        .expect("offscreen depth texture");

    // A throwaway color attachment (the pipeline needs a non-empty color set).
    let color = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: RASTER_COLOR_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::COLOR_ATTACHMENT,
            array_layers: 1,
        })
        .expect("throwaway color texture");

    // The quad vertex buffer (host-visible).
    let vertices = quad_vertices();
    let vertex_bytes = core::mem::size_of_val(&vertices) as u64;
    let vertex_buffer = device
        .create_buffer(&BufferDesc {
            size: vertex_bytes,
            usage: BufferUsage::VERTEX,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible vertex buffer");
    let vb_ptr = device
        .buffer_mapped_ptr(&vertex_buffer)
        .expect("host-visible vertex buffer is mapped");
    // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes (the
    // buffer was created at exactly that size); `vertices` is a distinct,
    // non-overlapping stack array of `vertex_bytes` bytes; the write completes before
    // any submit references the buffer (host-coherent: no flush).
    unsafe {
        core::ptr::copy_nonoverlapping(
            vertices.as_ptr().cast::<u8>(),
            vb_ptr.as_ptr(),
            vertex_bytes as usize,
        );
    }

    let vs = device
        .create_shader_module(MVP_VS_SPV.as_words())
        .expect("vertex shader module");
    let fs = device
        .create_shader_module(MVP_FS_SPV.as_words())
        .expect("fragment shader module");
    let cs = device
        .create_shader_module(sdf_depth_composite_spirv())
        .expect("composite compute shader module");

    let attributes = [
        VertexAttribute { location: 0, offset: 0, format: VertexFormat::Float32x3 },
        VertexAttribute { location: 1, offset: 12, format: VertexFormat::Float32x4 },
    ];
    let gfx = device
        .create_graphics_pipeline(&GraphicsPipelineDesc {
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
            blend: None,
            cull_mode: CullMode::None,
            depth_bias: None,
        })
        .expect("depth-testing graphics pipeline");

    let compute = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &cs,
            entry: c"main",
            // P0a: the marcher's push block is now the extent/camera struct (80 B).
            // The golden invocation pushes extent (64,64) + ORTHO → bit-exact rays.
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: None,
        })
        .expect("composite compute pipeline");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");

    encoder.begin().expect("begin");

    // Mesh raster pass: clear depth to the far plane, rasterize the quad.
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &color,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::ColorAttachmentOptimal,
        range: ImageSubresourceRange::COLOR,
    });
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &depth,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::EARLY_FRAGMENT_TESTS | BarrierStage::LATE_FRAGMENT_TESTS,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::DepthAttachmentOptimal,
        range: ImageSubresourceRange::DEPTH,
    });

    let full = RenderArea { x: 0, y: 0, width: SDF_IMG_W, height: SDF_IMG_H };
    let color_attachment = [RenderingAttachment {
        texture: &color,
        layout: ImageLayout::ColorAttachmentOptimal,
        load_op: LoadOp::Clear,
        store_op: StoreOp::Store,
        clear_color: [0.0, 0.0, 0.0, 1.0],
    }];
    encoder.begin_rendering(&RenderingDesc {
        render_area: full,
        colors: &color_attachment,
        depth: Some(DepthAttachment {
            texture: &depth,
            layout: ImageLayout::DepthAttachmentOptimal,
            load_op: LoadOp::Clear,
            store_op: StoreOp::Store,
            clear_depth: DEPTH_CLEAR,
        }),
    });
    encoder.bind_graphics_pipeline(&gfx);
    encoder.push_graphics_constants(&gfx, ShaderStage::VERTEX, 0, &ortho_mvp_bytes());
    encoder.bind_vertex_buffer(&vertex_buffer, 0, 0);
    encoder.set_viewport(&Viewport {
        x: 0.0,
        y: 0.0,
        width: SDF_IMG_W as f32,
        height: SDF_IMG_H as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    });
    encoder.set_scissor(&full);
    encoder.draw(6, 1, 0, 0); // two triangles = the quad
    encoder.end_rendering();

    // Depth → shared buffer: transition for the copy source, then copy the DEPTH
    // aspect into the buffer's DEPTH region.
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &depth,
        src_stage: BarrierStage::EARLY_FRAGMENT_TESTS | BarrierStage::LATE_FRAGMENT_TESTS,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE,
        dst_access: BarrierAccess::TRANSFER_READ,
        old_layout: ImageLayout::DepthAttachmentOptimal,
        new_layout: ImageLayout::TransferSrcOptimal,
        range: ImageSubresourceRange::DEPTH,
    });
    let depth_regions = [BufferImageCopy {
        buffer_offset: (COMPOSITE_DEPTH_BASE_WORDS as u64) * 4,
        buffer_row_length: 0,
        buffer_image_height: 0,
        aspect: ImageAspect::DEPTH,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
        image_offset_x: 0,
        image_offset_y: 0,
        image_offset_z: 0,
        image_extent_w: SDF_IMG_W,
        image_extent_h: SDF_IMG_H,
        image_extent_d: 1,
    }];
    encoder.copy_image_to_buffer(&depth, ImageLayout::TransferSrcOptimal, buffer, &depth_regions);

    // The load-bearing transfer → compute hazard over the shared buffer.
    encoder.pipeline_barrier(&BarrierDesc {
        src_stage: BarrierStage::TRANSFER,
        dst_stage: BarrierStage::COMPUTE_SHADER,
        buffers: &[BufferBarrier {
            buffer,
            src_access: BarrierAccess::TRANSFER_WRITE,
            dst_access: BarrierAccess::SHADER_READ,
        }],
    });

    // SDF composite compute pass: march bounded by the shared mesh depth.
    // P0a: push the full extent/camera block at the golden 64×64 ORTHO extent (same
    // extent → same rays → bit-exact pixels). `count` stays at offset 0 == PIXELS.
    let pc = CompositePushConstants::ortho(SDF_IMG_W, SDF_IMG_H);
    debug_assert_eq!(pc.count, PIXELS);
    encoder.bind_compute_pipeline(&compute);
    encoder.bind_storage_buffer(buffer, 0, 0);
    encoder.push_constants(ShaderStage::COMPUTE, 0, pc.as_bytes());
    encoder.dispatch(group_count_x(), 1, 1);

    encoder.end().expect("end");

    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    // SAFETY: every resource below was created on `device` and is destroyed exactly
    // once; the composite submission completed (fence-waited above), so none is in
    // use by the GPU. The composite `buffer` itself is NOT destroyed here — the
    // caller presents it and tears it down later.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_compute_pipeline(compute);
        device.destroy_graphics_pipeline(gfx);
        device.destroy_shader_module(cs);
        device.destroy_shader_module(fs);
        device.destroy_shader_module(vs);
        device.destroy_buffer(vertex_buffer);
        device.destroy_texture(color);
        device.destroy_texture(depth);
    }
}

/// Uploads the composite buffer's packed-RGBA PIXEL region into the SAMPLED
/// `R8G8B8A8_UNORM` texture exactly ONCE, in its own fenced submit, leaving the
/// texture in `SHADER_READ_ONLY_OPTIMAL`. After this the present loop only samples the
/// texture (a pure FRAGMENT_SHADER read), so frames-in-flight may sample it
/// concurrently with no write-after-read hazard.
///
/// The copy uses the TEXTURE's `SDF_IMG_W x SDF_IMG_H` (64x64) extent — the composite
/// source size — NOT the swapchain extent: the source pixel region holds exactly
/// `PIXELS` u32s and the texture is created at 64x64, so the copy can never over-read
/// the buffer nor over-write the texture regardless of the WSI swapchain extent.
fn upload_composite_to_texture(
    device: &VulkanContext,
    source: &boyko_rhi_vulkan::memory::BoundBuffer,
    texture: &boyko_rhi_vulkan::texture::VulkanTexture,
) {
    let queue = device.rhi_queue();
    let fence = device.create_fence(false).expect("upload fence");
    let mut encoder = device.create_command_encoder().expect("upload command encoder");

    encoder.begin().expect("begin upload");

    // UNDEFINED -> TRANSFER_DST (the prior contents are discarded; this is the first
    // use of the texture).
    encoder.image_barrier(&ImageBarrierDesc {
        texture,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::TRANSFER_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::TransferDstOptimal,
        range: ImageSubresourceRange::COLOR,
    });

    // Copy the packed-RGBA pixel region (one u32/texel, tightly packed) into the
    // texture using the TEXTURE's 64x64 extent.
    let regions = [BufferImageCopy {
        buffer_offset: (COMPOSITE_PIXEL_BASE_WORDS as u64) * 4,
        buffer_row_length: 0,
        buffer_image_height: 0,
        aspect: ImageAspect::COLOR,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
        image_offset_x: 0,
        image_offset_y: 0,
        image_offset_z: 0,
        image_extent_w: SDF_IMG_W,
        image_extent_h: SDF_IMG_H,
        image_extent_d: 1,
    }];
    encoder.copy_buffer_to_image(source, texture, ImageLayout::TransferDstOptimal, &regions);

    // TRANSFER_DST -> SHADER_READ_ONLY (the copy WROTE at TRANSFER; the present loop
    // READS at FRAGMENT_SHADER). The texture stays in this layout permanently.
    encoder.image_barrier(&ImageBarrierDesc {
        texture,
        src_stage: BarrierStage::TRANSFER,
        dst_stage: BarrierStage::FRAGMENT_SHADER,
        src_access: BarrierAccess::TRANSFER_WRITE,
        dst_access: BarrierAccess::SHADER_READ,
        old_layout: ImageLayout::TransferDstOptimal,
        new_layout: ImageLayout::ShaderReadOnlyOptimal,
        range: ImageSubresourceRange::COLOR,
    });

    encoder.end().expect("end upload");

    queue.submit(&encoder, &fence).expect("submit upload");
    device.wait_fence(&fence, u64::MAX).expect("wait upload fence");

    // SAFETY: the upload submission completed (fence-waited above), so neither the
    // encoder nor the fence is in use; each was created on `device` and is destroyed
    // exactly once. The `source` buffer + `texture` are owned + torn down by the
    // caller.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
    }
}

#[test]
fn windowed_hybrid_composite_present_is_validation_clean_and_renders_composite() {
    // Open the window first — the surface borrows its HWND/HINSTANCE and must be
    // destroyed before it.
    let mut window = match Window::open("boyko_rhi_vulkan hybrid window", WIDTH, HEIGHT) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("SKIP windowed_hybrid_present: cannot open a window ({e:?})");
            return;
        }
    };

    let ctx = match VulkanContext::boot(InstanceConfig {
        enable_validation: true,
        windowed: true,
    }) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("SKIP windowed_hybrid_present: windowed Vulkan unavailable ({e:?})");
            return;
        }
    };
    println!("Vulkan device (windowed, validation on): {}", ctx.device_name());
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

    // SAFETY: `window` outlives the surface (dropped after it below); its
    // HWND/HINSTANCE are live for the surface's lifetime.
    let surface = match unsafe { Surface::new(&ctx, window.hinstance(), window.hwnd()) } {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP windowed_hybrid_present: surface creation failed ({e:?})");
            return;
        }
    };

    let mut swapchain = match Swapchain::new(&ctx, &surface, window.width(), window.height()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("SKIP windowed_hybrid_present: swapchain creation failed ({e:?})");
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

    // The composite presents 1:1 in the swapchain image's TOP-LEFT 64×64 sub-rect; the
    // golden texels are at composite coords < 64. The WSI may clamp the swapchain extent
    // WIDER/taller than the requested 64×64 (that is fine — the rest stays clear), but
    // if it clamps SMALLER than 64 in either dimension the top-left 64×64 composite no
    // longer fits and the discriminator texels would be clipped → a graceful SKIP.
    if swapchain.extent().width < SDF_IMG_W || swapchain.extent().height < SDF_IMG_H {
        eprintln!(
            "SKIP windowed_hybrid_present: swapchain extent {}x{} is smaller than the {}x{} \
             composite, so the top-left 1:1 sub-rect does not fit",
            swapchain.extent().width,
            swapchain.extent().height,
            SDF_IMG_W,
            SDF_IMG_H
        );
        return;
    }

    // The readback byte order depends on the swapchain format; an SRGB/other format
    // is a graceful SKIP (no host-decodable UNORM byte order).
    let Some(is_bgra) = swapchain_readback_is_bgra(swapchain.format()) else {
        eprintln!(
            "SKIP windowed_hybrid_present: swapchain format {} (e.g. {VK_FORMAT_B8G8R8A8_SRGB} SRGB) \
             has no host-decodable UNORM byte order",
            swapchain.format()
        );
        return;
    };

    // The fullscreen-sample pipeline declares the swapchain's color format (W2-b).
    let Some(swap_color_format) = (match swapchain.format() {
        f if f == VK_FORMAT_B8G8R8A8_UNORM => Some(Format::B8G8R8A8Unorm),
        f if f == VK_FORMAT_R8G8B8A8_UNORM => Some(Format::R8G8B8A8Unorm),
        _ => None,
    }) else {
        eprintln!("SKIP windowed_hybrid_present: swapchain format has no basic-slice Format variant");
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

    // Pairwise-distinct invariant: the three regions must differ beyond the tolerance
    // so they are unambiguous on screen.
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

    // --- Submit 1: produce the hybrid composite into a persistent buffer. ---
    let composite_buffer = RhiDevice::create_buffer(
        device,
        &BufferDesc {
            size: (COMPOSITE_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE | BufferUsage::TRANSFER_DST | BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        },
    )
    .expect("persistent composite storage buffer");
    run_composite(device, &sdf, &composite_buffer);

    // --- Present resources: the SAMPLED texture + sampler + bind group + the
    //     fullscreen-sample pipeline (rung-5 pattern), sized to the composite. ---
    let composite_texture = RhiDevice::create_texture(
        device,
        &TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: Format::R8G8B8A8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
            array_layers: 1,
        },
    )
    .expect("SAMPLED composite texture (R8G8B8A8_UNORM)");

    let sampler = RhiDevice::create_sampler(
        device,
        &SamplerDesc {
            mag_filter: Filter::Nearest,
            min_filter: Filter::Nearest,
            address_mode: AddressMode::ClampToEdge,
            mip: MipMode::None,
            compare: None,
        },
    )
    .expect("nearest/clamp sampler");

    let bind_group_layout = RhiDevice::create_bind_group_layout(
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
    .expect("bind-group layout (one COMBINED_IMAGE_SAMPLER)");

    let sample_vs = RhiDevice::create_shader_module(device, SAMPLE_VS_SPV.as_words())
        .expect("fullscreen vertex shader module");
    let sample_fs = RhiDevice::create_shader_module(device, SAMPLE_FS_SPV.as_words())
        .expect("fullscreen fragment shader module");
    let fullscreen_pipeline = RhiDevice::create_graphics_pipeline(
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
            bind_group_layout: Some(&bind_group_layout),
            blend: None,
            cull_mode: CullMode::None,
            depth_bias: None,
        },
    )
    .expect("fullscreen-sample pipeline (swapchain color format)");

    // The shader modules are consumed by pipeline creation; destroy them now.
    // SAFETY: both modules were created on `ctx` above and are no longer needed once
    // the pipeline is created (the pipeline holds its own compiled code); each is
    // destroyed exactly once.
    unsafe {
        RhiDevice::destroy_shader_module(device, sample_fs);
        RhiDevice::destroy_shader_module(device, sample_vs);
    }

    let bind_group = RhiDevice::create_bind_group(
        device,
        &BindGroupDesc {
            layout: &bind_group_layout,
            entries: &[BindGroupEntry::CombinedImage {
                texture: &composite_texture,
                sampler: &sampler,
            }],
        },
    )
    .expect("bind group (composite texture, sampler)");

    // --- One-time upload: composite buffer pixel region -> the SAMPLED texture. ---
    // The composite is STATIC across the present loop, so it is uploaded into the
    // texture EXACTLY ONCE here (its own fenced submit) and left in
    // SHADER_READ_ONLY_OPTIMAL. The present loop then only SAMPLES it — multiple
    // frames-in-flight reading a read-only texture is sound (no write-after-read). The
    // copy uses the TEXTURE's 64x64 extent (the composite source size), never the
    // swapchain extent, so it can never read past the source pixel region nor write
    // past the texture even if the WSI clamps the swapchain extent.
    upload_composite_to_texture(device, &composite_buffer, &composite_texture);

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

    let composite = SampledComposite {
        texture: &composite_texture,
        sampler: &sampler,
        bind_group: &bind_group,
        pipeline: &fullscreen_pipeline,
        // The TEXTURE's own 64×64 size (the composite source), NOT the swapchain
        // extent: the present pass clamps its viewport/scissor to this so the composite
        // is drawn 1:1 in the swapchain image's top-left.
        texture_extent: VkExtent2D { width: SDF_IMG_W, height: SDF_IMG_H },
    };

    // --- Submit 2..: present the composite for a handful of frames; request the
    //     swapchain-image readback on ONE presented frame. ---
    let clear = [0.0_f32, 0.0, 0.0, 1.0];
    let mut readback_done = false;
    let mut readback_extent = swapchain.extent();
    for i in 0..5u32 {
        window.pump_events();
        window.refresh_size();

        // Request the readback on a single steady frame, only while the live extent
        // still matches the staging-buffer size (a resize simply skips the golden; the
        // present still runs).
        let live = swapchain.extent();
        let extent_stable = live.width == alloc_extent.width && live.height == alloc_extent.height;
        let want_readback = i == 2 && !readback_done && extent_stable;
        let rb = if want_readback { Some(&staging) } else { None };

        let token = renderer
            .wait_frame_in_flight()
            .expect("invariant: the frame slot fence wait precedes the submit");
        // SAFETY: `surface`/`swapchain` are live and created on the same device as
        // `renderer`; every `composite` resource is live on this device, the texture
        // was uploaded once and is resident in SHADER_READ_ONLY_OPTIMAL (the present
        // path only samples it), and the pipeline's color format equals the swapchain
        // format; a `Some(rb)` staging buffer is host-visible and `staging_size` (>=
        // one swapchain image) bytes.
        let presented = unsafe {
            renderer.present_sampled(
                token,
                &surface,
                &mut swapchain,
                &composite,
                window.width(),
                window.height(),
                clear,
                rb,
                None,
            )
        }
        .unwrap_or_else(|e| panic!("hybrid present frame {i} failed: {e:?}"));

        if want_readback && presented {
            readback_done = true;
            readback_extent = swapchain.extent();
        }
    }

    // The oracle: a clean windowed hybrid present records zero validation messages.
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
        .expect("validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during the windowed hybrid present — \
         see the [vk-validation] log",
        state.total()
    );

    // The golden: if a readback frame presented, the three discriminator texels must
    // match the host composite truth (swapchain byte-order-aware) — PROVING the hybrid
    // composite reached the swapchain image with correct colors.
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
        // `present_sampled`, and frames followed frame 2, so frame 2's copy is complete
        // + coherent); `out` is a distinct, non-overlapping alloc.
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
             (both over the sphere) must differ — proving the hybrid composite (not a clear) reached the screen"
        );
    } else {
        eprintln!(
            "NOTE windowed_hybrid_present: no readback frame presented (swapchain kept recreating); \
             validation was still asserted clean across all frames"
        );
    }

    // Clean reverse-order teardown: renderer (waits idle) → present resources →
    // composite buffer → swapchain → surface → window.
    drop(renderer);
    // SAFETY: the renderer was dropped above (its `Drop` waits the device idle), so no
    // submission references these resources; `ctx` is still alive; each is destroyed
    // exactly once, in reverse dependency order (bind group + layout + sampler, the
    // pipeline, the texture, the staging + composite buffers).
    unsafe {
        RhiDevice::destroy_buffer(device, staging);
        RhiDevice::destroy_bind_group(device, bind_group);
        RhiDevice::destroy_graphics_pipeline(device, fullscreen_pipeline);
        RhiDevice::destroy_bind_group_layout(device, bind_group_layout);
        RhiDevice::destroy_sampler(device, sampler);
        RhiDevice::destroy_texture(device, composite_texture);
        RhiDevice::destroy_buffer(device, composite_buffer);
    }
    drop(swapchain);
    drop(surface);
    drop(ctx);
    drop(window);
}
