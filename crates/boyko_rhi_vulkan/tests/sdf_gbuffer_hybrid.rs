//! Render **P1b GPU gate** (scaffold) — the OFFSCREEN image-based SDF + mesh hybrid
//! composite writing an MRT G-buffer, the image-based rewrite of the rung-10
//! packed-buffer marcher (`sdf_mesh_hybrid_depth.rs`'s `run_hybrid`).
//!
//! # What this proves (the P1b milestone)
//!
//! `run_gbuffer_hybrid` records the §15.1 shared-depth seam WITHOUT the per-frame
//! depth→buffer copy: a real GPU-rasterized quad's depth is written into a D32_SFLOAT
//! IMAGE, transitioned `DEPTH_ATTACHMENT_OPTIMAL -> SHADER_READ_ONLY_OPTIMAL` (a
//! SINGLE depth barrier, DEPTH aspect, `LATE_FRAGMENT_TESTS` src — replacing the old
//! copy + its two transfer barriers), and SAMPLED directly by the marcher compute
//! shader (`Texture2D<float>.Load`). The marcher STORES its color into an
//! `R8G8B8A8_UNORM` STORAGE image (the ALBEDO G-buffer target), plus additive
//! normal/material targets, through the P1a multi-resource descriptor *vocabulary* set
//! (written ONCE at create — NO per-frame `vkUpdateDescriptorSets`). The ALBEDO image
//! is read back and asserted against [`golden_composite_pixel_ex`] within `+/-2/255`
//! per channel — the SAME host golden the packed path uses, UNCHANGED.
//!
//! Determinism (INVIOLABLE): the field eval + ray-gen + lighting are byte-identical to
//! the packed marcher (a verbatim shader cut); only the depth SOURCE (a sampled image)
//! and the color SINK (a storage image) change. The float-to-UNORM albedo store vs the
//! host `pack_rgba` rounding is absorbed by the rung-10 `+/-2/255` tolerance.
//!
//! # SCAFFOLD STATUS — the GPU run is the TESTER's
//!
//! This file compiles + `run_gbuffer_hybrid` records the full P1b stream, but the
//! golden GPU assertion `p1b_gbuffer_hybrid_matches_golden` is gated behind `#[ignore]`
//! because it needs a real RTX-3060 device. The tester: (1) un-`#[ignore]` it, (2) run
//! it on the GPU, (3) confirm the readback ALBEDO matches the rung-10 hybrid golden
//! (crater_csg / box_csg / smooth_union + mesh-occludes-SDF) within `+/-2/255`, (4)
//! confirm — by recording inspection — that the `copy_image_to_buffer(depth)` + its two
//! barriers are ABSENT from the stream (the deletion target), and (5) confirm
//! validation + sync-validation are clean.

use core::ptr::NonNull;
use core::slice;

use boyko_rhi::enums::{BarrierAccess, BarrierStage};
use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc,
    BufferImageCopy, BufferUsage, ComputePipelineDesc, DepthAttachment, DescriptorKind, Format,
    GraphicsPipelineDesc, ImageAspect, ImageBarrierDesc, ImageLayout, ImageSubresourceRange,
    ImageUsage, LoadOp, MemoryLocation, PrimitiveTopology, RenderArea, RenderingAttachment,
    RenderingDesc, RhiCommandEncoder, RhiDevice, RhiQueue, SamplerDesc, ShaderStage, StoreOp,
    TextureDesc, TextureDimension, VertexAttribute, VertexBufferLayout, VertexFormat, Viewport,
};
use boyko_rhi_vulkan::compute::{
    COMPOSITE_PUSH_CONSTANT_BYTES, CompositeCamera, CompositePushConstants, LOCAL_SIZE_X,
    MESH_COLOR, MESH_DEPTH_CLEAR, SDF_CAMERA_Z, SDF_IMG_H, SDF_IMG_W, SDF_TRACE_T_MAX,
    SDF_VIEW_HALF_EXTENT, SdfEdit, EDITLIST_BUFFER_WORDS, editlist_pixel_hits, encode_edit_list,
    golden_composite_pixel_ex, mesh_depth_for_z, pixel_world_xy, sdf_gbuffer_composite_spirv,
    sdf_op,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// Total pixel count (the compute UBO `count`; the shader bounds `idx < count`).
const PIXELS: u32 = SDF_IMG_W * SDF_IMG_H;

/// R8G8B8A8 ALBEDO readback byte size.
const READBACK_BYTES: u64 = (PIXELS as u64) * 4;

/// Per-channel tolerance on the packed-RGBA bytes (identical to rung 9/10): DXC
/// `mad`/`fma` rounding + the float→UNORM store quantization make a bit-exact match
/// brittle; `+/-2/255` still proves the lit SDF surface / flat mesh / background colors
/// apart (they differ by 100+).
const CHANNEL_TOL: i32 = 2;

/// The mesh quad's constant world Z (chosen strictly between the sphere surface and the
/// camera so the mesh occludes the SDF over the sphere). Mirrors `run_hybrid`'s `MESH_Z`.
const MESH_Z: f32 = 1.0;

/// The mesh quad's world-XY footprint (the left part of the view in x, full y), so the
/// sphere straddles the quad edge — yielding texels over BOTH / sphere-only / quad-only
/// / neither. Mirrors `run_hybrid`'s footprint.
const QUAD_X_MIN: f32 = -1.0;
const QUAD_X_MAX: f32 = 0.2;
const QUAD_Y_MIN: f32 = -1.0;
const QUAD_Y_MAX: f32 = 1.0;

/// The depth attachment's CLEAR value (the far plane; an uncovered pixel keeps it,
/// decoded as "no mesh"). Must equal [`MESH_DEPTH_CLEAR`].
const DEPTH_CLEAR: f32 = MESH_DEPTH_CLEAR;

/// A throwaway color-attachment format for the mesh raster pass (the graphics pipeline
/// requires a non-empty `color_formats`; only the DEPTH result is consumed).
const COLOR_FORMAT: Format = Format::R8G8B8A8Unorm;

/// The G-buffer color format (albedo / normal / material): `R8G8B8A8_UNORM`, the
/// STORAGE-image store target whose support the [`DeviceCaps`] boot fail-fast asserts.
const GBUFFER_FORMAT: Format = Format::R8G8B8A8Unorm;

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

/// The MVP byte size (a `float4x4`).
const MVP_BYTES: u32 = 64;

/// A 4-byte-aligned wrapper around a committed SPIR-V byte blob.
#[repr(C, align(4))]
struct SpirvBlob<const N: usize>([u8; N]);

impl<const N: usize> SpirvBlob<N> {
    fn as_words(&self) -> &[u32] {
        const { assert!(N.is_multiple_of(4), "SPIR-V byte length must be a multiple of 4") };
        // SAFETY: the `align(4)` wrapper makes `self.0`'s address a valid `*const u32`;
        // `N` is a 4-byte multiple (const-asserted); the `&self` borrow keeps the
        // `'static` blob alive for the slice's lifetime; any bit pattern is a valid `u32`.
        unsafe { slice::from_raw_parts(self.0.as_ptr().cast::<u32>(), N / 4) }
    }
}

/// The committed rung-3 vertex SPIR-V (`triangle_mvp.vs.spv`), reused for the raster.
static MVP_VS_SPV: SpirvBlob<916> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/triangle_mvp.vs.spv"
)));

/// The committed rung-3 fragment SPIR-V (`triangle_mvp.fs.spv`), reused.
static MVP_FS_SPV: SpirvBlob<368> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/triangle_mvp.fs.spv"
)));

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

/// Asserts the validation messenger recorded ZERO messages (the GPU-half oracle).
fn assert_validation_clean(ctx: &VulkanContext) {
    let state = ctx
        .debug_state()
        .expect("invariant: validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during the P1b G-buffer hybrid run — see the [vk-validation] log",
        state.total()
    );
}

/// `ceil(PIXELS / LOCAL_SIZE_X)` — the 1D compute dispatch group count.
fn group_count_x() -> u32 {
    PIXELS.div_ceil(LOCAL_SIZE_X)
}

/// The orthographic MVP for the rung-3 vertex shader, uploaded COLUMN-MAJOR (the
/// VERIFIED transpose — see `run_hybrid`'s `ortho_mvp_bytes`). Maps a fronto-parallel
/// world vertex so the stored depth is `(CAM_Z - worldZ) / T_MAX`.
#[rustfmt::skip]
fn ortho_mvp_bytes() -> [u8; MVP_BYTES as usize] {
    let h = SDF_VIEW_HALF_EXTENT;
    let tmax = SDF_TRACE_T_MAX;
    let cam = SDF_CAMERA_Z;
    // Mᵀ in row-major upload order: mt[r*4 + c] = M[c][r] (each group of 4 is a COLUMN
    // of M); the only off-diagonal term `CAM_Z/T_MAX` lives at mt[14].
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

/// The per-pixel mesh depth the GPU is expected to produce (the host model for the
/// golden's `mesh_depth` input): the constant inside the quad, the clear outside.
fn expected_mesh_depth(px: u32, py: u32) -> f32 {
    if mesh_covers_pixel(px, py) {
        mesh_depth_for_z(MESH_Z)
    } else {
        DEPTH_CLEAR
    }
}

/// Writes `words` `u32`s into a buffer's persistent host-coherent mapping (valid before
/// the submit — the CPU seeds the edit-list header / the UBO here).
fn write_words(base: NonNull<u8>, words: &[u32]) {
    let dst = base.as_ptr().cast::<u32>();
    for (i, &w) in words.iter().enumerate() {
        // SAFETY: the buffer is at least `words.len() * 4` bytes inside the persistent
        // host-coherent mapping; `dst + i` for `i < words.len()` is in-bounds. No GPU
        // work is in flight yet (the submit follows), so the host write is
        // unsynchronized-safe. `write_unaligned` tolerates the sub-allocated offset.
        unsafe { dst.add(i).write_unaligned(w) };
    }
}

/// Splits a packed `0xAABBGGRR` into `[r, g, b]` (the low three bytes).
fn unpack_packed_rgb(packed: u32) -> [i32; 3] {
    [
        (packed & 0xFF) as i32,
        ((packed >> 8) & 0xFF) as i32,
        ((packed >> 16) & 0xFF) as i32,
    ]
}

/// Splits an R8G8B8A8 readback texel's first three bytes into `[r, g, b]`.
fn unpack_texel_rgb(rgba: &[u8]) -> [i32; 3] {
    [rgba[0] as i32, rgba[1] as i32, rgba[2] as i32]
}

/// `true` if a readback texel agrees with a packed golden within `CHANNEL_TOL`/channel.
fn texel_close(got: [i32; 3], want_packed: u32) -> bool {
    let w = unpack_packed_rgb(want_packed);
    (0..3).all(|c| (got[c] - w[c]).abs() <= CHANNEL_TOL)
}

/// Records + submits the full P1b OFFSCREEN G-buffer hybrid composite in ONE command
/// buffer / ONE fenced submit, returning the readback ALBEDO storage image as `PIXELS`
/// R8G8B8A8 texels (4 bytes each). The flow — the §15.1 seam with NO depth→buffer copy:
///
///   raster quad → D32 depth IMAGE → barrier depth DEPTH_ATTACHMENT→SHADER_READ_ONLY
///   (one barrier) → barrier the 3 G-buffer images UNDEFINED→GENERAL → bind the
///   vocabulary set {SSBO edit-list, SAMPLED depth, STORAGE albedo/normal/material,
///   UNIFORM camera} + the marcher → dispatch (the marcher SAMPLES the depth image) →
///   barrier albedo GENERAL→TRANSFER_SRC → copy_image_to_buffer(albedo) into readback.
///
/// There is NO `copy_image_to_buffer(depth)` and NO transfer→compute buffer barrier:
/// the single depth `DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL` barrier
/// replaces the old copy + its two barriers. The vocabulary set is written ONCE at
/// `create_bind_group` — there is no per-frame `vkUpdateDescriptorSets`.
fn run_gbuffer_hybrid(ctx: &VulkanContext, edits: &[SdfEdit]) -> Vec<u8> {
    let device: &VulkanContext = ctx;
    let queue = ctx.rhi_queue();

    // --- The edit-list StorageBuffer (binding 0), seeded with the packed header. The
    // P1b shader only READS the rung-9 header + edit array (`Buf[0..PIXEL_BASE_WORDS]`,
    // i.e. `Buf[0..196]`); the depth/pixel regions are no longer used by this path.
    // We deliberately OVER-ALLOCATE to the full `EDITLIST_BUFFER_WORDS` (which still
    // includes the now-unused pixel region) rather than trimming to `PIXEL_BASE_WORDS`:
    // `encode_edit_list` debug-asserts `buf.len() >= EDITLIST_BUFFER_WORDS`, so reusing
    // the shared const keeps the host encoder and the buffer in lock-step and avoids a
    // size desync. The extra words are simply never touched. ---
    let buffer = device
        .create_buffer(&BufferDesc {
            size: (EDITLIST_BUFFER_WORDS as u64) * 4,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("edit-list storage buffer");
    {
        let mut header = vec![0u32; EDITLIST_BUFFER_WORDS];
        encode_edit_list(&mut header, edits);
        let mapped = device
            .buffer_mapped_ptr(&buffer)
            .expect("host-visible buffer is mapped");
        write_words(mapped, &header);
    }

    // --- The camera/extent UNIFORM buffer (binding 5), written ONCE at setup (NOT a
    // per-frame push). At the golden 64×64 ORTHO extent it drives bit-exact rays. ---
    let camera_uniform = device
        .create_buffer(&BufferDesc {
            size: COMPOSITE_PUSH_CONSTANT_BYTES as u64,
            usage: BufferUsage::UNIFORM,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("camera uniform buffer");
    {
        let pc = CompositePushConstants::ortho(SDF_IMG_W, SDF_IMG_H);
        debug_assert_eq!(pc.count, PIXELS);
        let mapped = device
            .buffer_mapped_ptr(&camera_uniform)
            .expect("host-visible uniform buffer is mapped");
        // `as_bytes()` is the same 80-byte POD the packed path pushed; write it once.
        let bytes = pc.as_bytes();
        // SAFETY: `mapped` points to `COMPOSITE_PUSH_CONSTANT_BYTES` mapped
        // host-coherent bytes; `bytes` is exactly that many bytes; no GPU work is in
        // flight yet (submit follows), so the write is unsynchronized-safe.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.as_ptr(), bytes.len());
        }
    }

    // --- The depth IMAGE (D32_SFLOAT): DEPTH_STENCIL_ATTACHMENT (rasterize into it) |
    // SAMPLED (the marcher samples it directly — NO copy). A DEPTH_STENCIL_ATTACHMENT
    // usage gives the texture a DEPTH-aspect view, exactly what the marcher's
    // `Texture2D<float>.Load` samples. ---
    let depth = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: Format::D32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
        })
        .expect("offscreen depth texture (sampled)");

    // A throwaway color attachment for the raster pass (never read back).
    let color = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: COLOR_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::COLOR_ATTACHMENT,
        })
        .expect("throwaway color texture");

    // --- The MRT G-buffer STORAGE images (albedo + normal + material): STORAGE for the
    // compute store; albedo also TRANSFER_SRC so the golden readback can copy it out. ---
    let albedo = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: GBUFFER_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE | ImageUsage::TRANSFER_SRC,
        })
        .expect("G-buffer albedo storage image");
    let normal = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: GBUFFER_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE,
        })
        .expect("G-buffer normal storage image");
    let material = device
        .create_texture(&TextureDesc {
            width: SDF_IMG_W,
            height: SDF_IMG_H,
            depth: 1,
            format: GBUFFER_FORMAT,
            dimension: TextureDimension::D2,
            usage: ImageUsage::STORAGE,
        })
        .expect("G-buffer material storage image");

    // The depth is SAMPLED via `.Load` (OpImageFetch, no sampler), but the RHI
    // `BindGroupEntry::SampledImage` requires a sampler handle; a nearest/clamp sampler
    // is created and bound (it is ignored by an unfiltered fetch).
    let sampler = device
        .create_sampler(&SamplerDesc::default())
        .expect("depth sampler (ignored by .Load)");

    // The readback buffer for the ALBEDO image.
    let readback = device
        .create_buffer(&BufferDesc {
            size: READBACK_BYTES,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible readback buffer");

    // The quad vertex buffer.
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
    // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes; `vertices`
    // is a distinct stack array of `vertex_bytes` bytes; the write completes before any
    // submit references the buffer (host-coherent: no flush).
    unsafe {
        core::ptr::copy_nonoverlapping(
            vertices.as_ptr().cast::<u8>(),
            vb_ptr.as_ptr(),
            vertex_bytes as usize,
        );
    }

    // --- Modules: the rung-3 mesh-raster pair + the P1b G-buffer marcher (compute). ---
    let vs = device
        .create_shader_module(MVP_VS_SPV.as_words())
        .expect("vertex shader module");
    let fs = device
        .create_shader_module(MVP_FS_SPV.as_words())
        .expect("fragment shader module");
    let cs = device
        .create_shader_module(sdf_gbuffer_composite_spirv())
        .expect("P1b G-buffer marcher compute shader module");

    // The depth-testing graphics pipeline (rung-3 vertex layout + 64-byte VERTEX MVP
    // push + a declared depth_format).
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
            color_formats: &[COLOR_FORMAT],
            depth_format: Some(Format::D32Sfloat),
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: Some(VertexBufferLayout {
                stride: VERTEX_STRIDE,
                attributes: &attributes,
            }),
            push_constant_bytes: MVP_BYTES,
            bind_group_layout: None,
        })
        .expect("depth-testing graphics pipeline");

    // --- The P1b vocabulary set: { SSBO edit-list @0, SAMPLED depth @1, STORAGE
    // albedo @2, STORAGE normal @3, STORAGE material @4, UNIFORM camera @5 }. ---
    let layout_entries = [
        BindGroupLayoutEntry { binding: 0, count: 1, kind: DescriptorKind::StorageBuffer, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 1, count: 1, kind: DescriptorKind::SampledImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 2, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 3, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 4, count: 1, kind: DescriptorKind::StorageImage, stage: ShaderStage::COMPUTE },
        BindGroupLayoutEntry { binding: 5, count: 1, kind: DescriptorKind::UniformBuffer, stage: ShaderStage::COMPUTE },
    ];
    let bind_layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc { entries: &layout_entries })
        .expect("P1b vocabulary bind-group layout");
    let compute = device
        .create_compute_pipeline(&ComputePipelineDesc {
            module: &cs,
            entry: c"main",
            // The dedicated vocabulary layout declares the shared compute push range; the
            // marcher takes NO push (the camera is the UBO @ binding 5), so nothing is
            // pushed against it. `COMPOSITE_PUSH_CONSTANT_BYTES` keeps the create-time
            // "non-empty multiple of 4 within the shared range" contract.
            push_constant_bytes: COMPOSITE_PUSH_CONSTANT_BYTES,
            bind_group_layout: Some(&bind_layout),
        })
        .expect("P1b G-buffer marcher compute pipeline");
    // The vocabulary bind group, written ONCE at create (NO per-frame update).
    let bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &bind_layout,
            entries: &[
                BindGroupEntry::StorageBuffer { buffer: &buffer },
                BindGroupEntry::SampledImage { texture: &depth, sampler: &sampler },
                BindGroupEntry::StorageImage { texture: &albedo },
                BindGroupEntry::StorageImage { texture: &normal },
                BindGroupEntry::StorageImage { texture: &material },
                BindGroupEntry::UniformBuffer { buffer: &camera_uniform },
            ],
        })
        .expect("P1b vocabulary bind group");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");

    encoder.begin().expect("begin");

    // --- Mesh raster pass: clear depth to the far plane, rasterize the quad. ---
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

    // --- THE single depth dual-use barrier: DEPTH_ATTACHMENT_OPTIMAL →
    // SHADER_READ_ONLY_OPTIMAL. The depth WRITES happen at LATE_FRAGMENT_TESTS; the
    // marcher SAMPLES at COMPUTE_SHADER. This one barrier (DEPTH aspect,
    // LATE_FRAGMENT_TESTS src) makes the depth-write available + visible to the
    // shader-read and transitions the layout for sampling. It REPLACES the rung-10
    // depth→buffer copy + its two transfer barriers — there is NO copy_image_to_buffer
    // of the depth here. ---
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &depth,
        src_stage: BarrierStage::EARLY_FRAGMENT_TESTS | BarrierStage::LATE_FRAGMENT_TESTS,
        dst_stage: BarrierStage::COMPUTE_SHADER,
        src_access: BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE,
        dst_access: BarrierAccess::SHADER_READ,
        old_layout: ImageLayout::DepthAttachmentOptimal,
        new_layout: ImageLayout::ShaderReadOnlyOptimal,
        range: ImageSubresourceRange::DEPTH,
    });

    // --- The 3 G-buffer storage images: UNDEFINED → GENERAL (a compute store). ---
    for tex in [&albedo, &normal, &material] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::TOP_OF_PIPE,
            dst_stage: BarrierStage::COMPUTE_SHADER,
            src_access: BarrierAccess::NONE,
            dst_access: BarrierAccess::SHADER_WRITE,
            old_layout: ImageLayout::Undefined,
            new_layout: ImageLayout::General,
            range: ImageSubresourceRange::COLOR,
        });
    }

    // --- SDF marcher compute pass: SAMPLE the depth image, STORE the G-buffer. The
    // vocabulary set is bound against the pipeline's OWN dedicated layout via
    // `bind_descriptor_set_compute`; no `bind_storage_buffer`, so the encoder's fixed
    // single-set rebind is skipped and no push is recorded (the camera is the UBO). ---
    encoder.bind_compute_pipeline(&compute);
    encoder.bind_descriptor_set_compute(&bind_group, &compute);
    encoder.dispatch(group_count_x(), 1, 1);

    // --- ALBEDO: GENERAL → TRANSFER_SRC_OPTIMAL for the readback copy. ---
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &albedo,
        src_stage: BarrierStage::COMPUTE_SHADER,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::SHADER_WRITE,
        dst_access: BarrierAccess::TRANSFER_READ,
        old_layout: ImageLayout::General,
        new_layout: ImageLayout::TransferSrcOptimal,
        range: ImageSubresourceRange::COLOR,
    });

    let regions = [BufferImageCopy {
        buffer_offset: 0,
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
    encoder.copy_image_to_buffer(&albedo, ImageLayout::TransferSrcOptimal, &readback, &regions);

    encoder.end().expect("end");

    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    // Read back the ALBEDO R8G8B8A8 bytes.
    let dst_ptr = device
        .buffer_mapped_ptr(&readback)
        .expect("host-visible readback buffer is mapped");
    let mut out = vec![0u8; READBACK_BYTES as usize];
    // SAFETY: `dst_ptr` points to `READBACK_BYTES` mapped host-coherent bytes; a fence
    // wait preceded this read, so the GPU store + copy are complete + coherent; reading
    // `READBACK_BYTES` bytes is in-bounds; `out` is a distinct allocation.
    unsafe {
        core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), READBACK_BYTES as usize);
    }

    assert_validation_clean(ctx);

    // SAFETY: every resource was created on `device`; the last submission completed
    // (fence-waited above), so none is in use; each is destroyed exactly once.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_bind_group(bind_group);
        device.destroy_compute_pipeline(compute);
        device.destroy_bind_group_layout(bind_layout);
        device.destroy_graphics_pipeline(gfx);
        device.destroy_shader_module(cs);
        device.destroy_shader_module(fs);
        device.destroy_shader_module(vs);
        device.destroy_buffer(vertex_buffer);
        device.destroy_buffer(readback);
        device.destroy_sampler(sampler);
        device.destroy_texture(material);
        device.destroy_texture(normal);
        device.destroy_texture(albedo);
        device.destroy_texture(color);
        device.destroy_texture(depth);
        device.destroy_buffer(camera_uniform);
        device.destroy_buffer(buffer);
    }

    out
}

/// The rung-9/10 "crater" CSG scene (base sphere minus a smaller sphere).
fn crater() -> Vec<SdfEdit> {
    vec![
        SdfEdit::sphere([0.0, 0.0, 0.0], 0.5, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.3, 0.0, 0.0], 0.35, sdf_op::SUBTRACT, 0.0),
    ]
}

/// A box CSG scene (a box unioned, exercising the box primitive + the mesh occlusion).
fn box_csg() -> Vec<SdfEdit> {
    vec![SdfEdit::box_shape([0.0, 0.0, 0.0], [0.4, 0.4, 0.4], sdf_op::UNION, 0.0)]
}

/// A smooth-union scene (two spheres blended), exercising the smooth-min path.
fn smooth_union() -> Vec<SdfEdit> {
    vec![
        SdfEdit::sphere([-0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.0),
        SdfEdit::sphere([0.25, 0.0, 0.0], 0.35, sdf_op::UNION, 0.15),
    ]
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

/// **P1b GPU gate (TESTER):** the OFFSCREEN image-based G-buffer hybrid composite
/// reproduces the rung-10 hybrid golden by reading back the ALBEDO STORAGE image, with
/// NO depth→buffer copy in the recorded stream.
///
/// For each scene (crater_csg / box_csg / smooth_union): every readback ALBEDO texel
/// must match [`golden_composite_pixel_ex`] (fed the SAME per-pixel
/// [`expected_mesh_depth`] the GPU rasterizes) within `+/-2/255` per channel, plus the
/// four discriminator texels (mesh-occludes-SDF / SDF-only / mesh-only / background) and
/// `assert_validation_clean`. The set is written ONCE at create — NO per-frame
/// `vkUpdateDescriptorSets`. The depth `copy_image_to_buffer` + its two barriers are
/// ABSENT (confirm by recording inspection / the validation-clean sync check).
#[test]
fn p1b_gbuffer_hybrid_matches_golden() {
    let Some(ctx) = boot_or_skip("p1b_gbuffer_hybrid_matches_golden") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    assert!(ctx.validation_enabled(), "validation must be active");
    let caps = ctx.device_caps();
    assert!(
        caps.gbuffer_storage_format_ok,
        "a booted context must support STORAGE_IMAGE on the G-buffer format"
    );

    for (name, edits) in [
        ("crater_csg", crater()),
        ("box_csg", box_csg()),
        ("smooth_union", smooth_union()),
    ] {
        // The four discriminator texels, picked host-side BEFORE the GPU run.
        let a = find_texel(&edits, |hit, covered| hit && covered);
        let b = find_texel(&edits, |hit, covered| hit && !covered);
        let c = find_texel(&edits, |hit, covered| !hit && covered);
        let d = find_texel(&edits, |hit, covered| !hit && !covered);

        let albedo = run_gbuffer_hybrid(&ctx, &edits);
        assert_eq!(albedo.len(), READBACK_BYTES as usize);

        let texel = |px: u32, py: u32| -> &[u8] {
            let base = ((py * SDF_IMG_W + px) as usize) * 4;
            &albedo[base..base + 4]
        };

        // Whole-image golden scan: each ALBEDO texel within +/-2/255 of the host golden,
        // fed the per-pixel mesh depth the GPU rasterizes.
        let mut max_delta = 0i32;
        for py in 0..SDF_IMG_H {
            for px in 0..SDF_IMG_W {
                let got = unpack_texel_rgb(texel(px, py));
                let md = expected_mesh_depth(px, py);
                let want = golden_composite_pixel_ex(
                    &edits,
                    md,
                    px,
                    py,
                    SDF_IMG_W,
                    SDF_IMG_H,
                    CompositeCamera::Ortho,
                );
                let w = unpack_packed_rgb(want);
                for ch in 0..3 {
                    let dd = (got[ch] - w[ch]).abs();
                    if dd > max_delta {
                        max_delta = dd;
                    }
                }
                assert!(
                    texel_close(got, want),
                    "[{name}] albedo texel ({px},{py}) mismatch: got {got:?}, want {w:?} (tol {CHANNEL_TOL}, max so far {max_delta})"
                );
            }
        }
        println!("[{name}] P1b G-buffer albedo: max per-channel delta = {max_delta}/255 (tol {CHANNEL_TOL})");

        // Texel A (sphere ∧ quad) → MESH_COLOR (mesh occludes the SDF — the load-bearing
        // occlusion proof, only correct if the sampled depth actually clipped the march).
        if let Some((ax, ay)) = a {
            let got = unpack_texel_rgb(texel(ax, ay));
            assert!(
                texel_close(got, boyko_rhi_vulkan::compute::pack_rgba(MESH_COLOR)),
                "[{name}] texel A ({ax},{ay}) must be MESH_COLOR (mesh occludes SDF), got {got:?}"
            );
        }
        // Texel D (background) — distinct from texel A (mesh), proving the marcher ran a
        // field, not a constant fill.
        if let (Some((ax, ay)), Some((dx, dy))) = (a, d) {
            let av = unpack_texel_rgb(texel(ax, ay));
            let dv = unpack_texel_rgb(texel(dx, dy));
            assert!(
                !(0..3).all(|ch| (av[ch] - dv[ch]).abs() <= CHANNEL_TOL),
                "[{name}] texel A {av:?} (mesh) must differ from texel D {dv:?} (background)"
            );
        }
        let _ = (b, c); // B/C are exercised by the whole-image scan above.
    }
}
