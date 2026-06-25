//! Phase-6 Slice S1 — RUNG 6 acceptance: a minimal DEFERRED renderer — a 2-attachment
//! G-buffer (albedo + world normal, MRT) written by a geometry pass, then a full-screen
//! deferred-lighting pass that samples BOTH G-buffer textures and applies ONE hardcoded
//! directional light, written to a final output texture. Rendered offscreen + headless
//! via the Vulkan 1.3 dynamic-rendering graphics surface, proven by a golden image
//! readback, with the validation layer as the soundness oracle.
//!
//! This is the deferred-shading CORE (G-buffer + lighting); it does NOT present to a
//! swapchain (the next rung), and there is no SDF / ECS / physics. It synthesizes
//! rungs 1-5: textures + image barriers (rung 1), a graphics pipeline + draw (rung 2),
//! a vertex buffer + MVP push constant (rung 3), a depth attachment + depth test
//! (rung 4), and descriptor-set + sampler reads (rung 5) — now with MULTIPLE render
//! targets (two color attachments) and TWO sampled bindings.
//!
//! # The two passes
//!
//! - **Geometry pass (MRT):** clear the albedo + normal G-buffer textures, depth-test
//!   against a depth attachment, and draw a quad (two triangles) from a vertex buffer,
//!   transformed by an MVP push constant. The fragment shader writes a known albedo to
//!   SV_Target0 and the packed world normal `(n * 0.5 + 0.5)` to SV_Target1. The quad
//!   faces +Z (normal `(0, 0, 1)`), packed to `(0.5, 0.5, 1.0)`.
//! - **Barrier:** transition BOTH G-buffer textures COLOR_ATTACHMENT_OPTIMAL ->
//!   SHADER_READ_ONLY_OPTIMAL (the lighting pass will SAMPLE them).
//! - **Lighting pass:** a full-screen triangle whose fragment shader samples albedo +
//!   normal (a 2-binding bind group), unpacks the normal, and computes
//!   `lit = albedo * max(dot(N, L), 0) + ambient` for `L = (0, 0, 1)`, `ambient = 0.1`,
//!   writing the lit color into the final output texture `O`.
//! - **Readback:** copy `O` -> a host-visible buffer and assert.
//!
//! # The decisive assertions
//!
//! - `O`'s CENTRE texel (covered by the quad: `N·L = 1`) == `albedo + ambient`
//!   (within a small UNORM-quantization tolerance) — proving the full deferred chain:
//!   the geometry pass wrote albedo + normal, the barrier published them, and the
//!   lighting pass sampled BOTH and applied the light.
//! - `O`'s (0,0) CORNER texel (no geometry: the G-buffer cleared to albedo 0 + a
//!   normal that unpacks to `N = 0`, so `N·L = 0`) == `ambient` — the lighting-pass
//!   background.
//! - The validation messenger recorded ZERO messages (the GPU-half oracle).
//!
//! # CI gate (graceful skip)
//!
//! A GPU-less / loader-less host, or one without the validation layer / dynamic
//! rendering, makes `VulkanContext::boot` return `Err`; the test skips gracefully
//! (mirrors rungs 1-5).

use core::slice;

use boyko_rhi::enums::{AddressMode, BarrierAccess, BarrierStage, DescriptorKind, Filter};
use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc,
    BufferImageCopy, BufferUsage, Format, GraphicsPipelineDesc, ImageAspect, ImageBarrierDesc,
    ImageLayout, ImageSubresourceRange, ImageUsage, LoadOp, MemoryLocation, MipMode,
    PrimitiveTopology, RenderArea, RenderingAttachment, RenderingDesc, RhiCommandEncoder, RhiDevice,
    RhiQueue, SamplerDesc, ShaderStage, StoreOp, TextureDesc, TextureDimension, VertexAttribute,
    VertexBufferLayout, VertexFormat, Viewport,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// The offscreen image dimensions. Small but multi-texel so a covered/uncovered
/// boundary is unambiguous.
const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;
const TEXELS: usize = (WIDTH * HEIGHT) as usize;
const SIZE: u64 = (TEXELS * 4) as u64;

/// The known surface albedo the geometry fragment shader writes to SV_Target0. MUST
/// equal the `ALBEDO` constant in `gbuffer.fs.hlsl` — the lighting golden is computed
/// from it on the host side.
const ALBEDO: [f32; 3] = [0.8, 0.6, 0.4];

/// The additive ambient term the lighting fragment shader applies. MUST equal the
/// `AMBIENT` constant in `deferred_light.fs.hlsl`.
const AMBIENT: f32 = 0.1;

/// The per-channel UNORM-quantization tolerance for the lit golden. The albedo is
/// quantized once into the G-buffer (R8G8B8A8_UNORM), sampled back, lit, and quantized
/// again into `O`; ±3 covers the round-trip + rounding-mode slack while still catching
/// a wrong color / missing light / wrong channel (the golden colors differ by far more
/// than 3 per channel).
const TOL: i32 = 3;

/// The G-buffer clear values. Albedo clears to opaque BLACK, which is what makes an
/// UNCOVERED texel's lit value EXACTLY `ambient`: `lit = albedo * (N·L) + ambient`
/// with `albedo = 0` is `ambient` for ANY N·L. (The normal clears to `(0.5,0.5,0.5)`,
/// which unpacks to `2*128/255 - 1 ≈ 0.004` per axis — NOT exactly 0 — so the corner's
/// exactness rides on the zeroed albedo, not on N·L being zero. A future change to a
/// non-black albedo clear would make the corner depend on the cleared normal.) The
/// depth clears to the far plane `1.0`.
const ALBEDO_CLEAR: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const NORMAL_CLEAR: [f32; 4] = [0.5, 0.5, 0.5, 1.0];

/// The final-output clear color. The full-screen lighting draw covers all of `O`, so
/// this never survives; a distinct value makes a coverage bug loud.
const OUTPUT_CLEAR: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// The MVP byte size (a `float4x4`).
const MVP_BYTES: u32 = 64;

/// One geometry-pass vertex: a `Float32x3` position (offset 0) + a `Float32x3` world
/// normal (offset 12), matching the rung-6 G-buffer vertex layout. `#[repr(C)]` so the
/// field layout is the exact 24-byte stride the layout declares (3 + 3 = 6 f32).
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
}

/// The per-vertex stride the layout declares (3 + 3 floats = 24 bytes).
const VERTEX_STRIDE: u32 = core::mem::size_of::<Vertex>() as u32;
const _: () = assert!(VERTEX_STRIDE == 24, "Vertex must be tightly packed at 24 bytes");

/// A float color/value as the R8G8B8A8_UNORM byte (round-to-nearest), the convention a
/// UNORM attachment uses.
fn to_unorm(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// The deterministic MVP: `diag(0.7, 0.7, 1, 1)` packed column-major (identical to
/// row-major because the matrix is diagonal). It scales the model-space `[-1, 1]` quad
/// to the `[-0.7, 0.7]` covering quad (covers the centre, misses the corners — a
/// bypassed/identity MVP would fill the corner too, which the corner assert catches).
#[rustfmt::skip]
fn mvp_bytes() -> [u8; MVP_BYTES as usize] {
    let m: [f32; 16] = [
        0.7, 0.0, 0.0, 0.0,
        0.0, 0.7, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        0.0, 0.0, 0.0, 1.0,
    ];
    let mut out = [0u8; MVP_BYTES as usize];
    for (i, f) in m.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
    }
    out
}

// --- Committed SPIR-V blobs (a `#[repr(C, align(4))]` byte wrapper re-viewed as a
//     `&[u32]` word stream). The geometry pass uses the new MRT G-buffer shaders; the
//     lighting pass reuses the rung-5 full-screen vertex shader + the new 2-sample
//     lighting fragment shader. ---

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

/// The committed rung-6 G-buffer vertex SPIR-V (`gbuffer.vs.spv`, 916 bytes): position
/// + normal from a vertex buffer, MVP-transformed.
static GBUFFER_VS_SPV: SpirvBlob<916> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/gbuffer.vs.spv"
)));

/// The committed rung-6 G-buffer fragment SPIR-V (`gbuffer.fs.spv`, 768 bytes): writes
/// albedo to SV_Target0 + packed normal to SV_Target1 (MRT).
static GBUFFER_FS_SPV: SpirvBlob<768> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/gbuffer.fs.spv"
)));

/// The committed rung-5 full-screen vertex SPIR-V (`fullscreen_sample.vs.spv`, 744
/// bytes), reused for the lighting pass (full-screen triangle + UVs from SV_VertexID).
static LIGHT_VS_SPV: SpirvBlob<744> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/fullscreen_sample.vs.spv"
)));

/// The committed rung-6 deferred-lighting fragment SPIR-V (`deferred_light.fs.spv`,
/// 1432 bytes): samples albedo + normal (2 bindings) and applies the directional light.
static LIGHT_FS_SPV: SpirvBlob<1432> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/deferred_light.fs.spv"
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
///
/// A no-op (with a one-line note) when validation is disabled via
/// `BOYKO_DISABLE_VALIDATION` (the layer DLL crashes the MinGW process on this
/// box): there is no messenger to read, but the PIXEL golden still runs.
fn assert_validation_clean(ctx: &VulkanContext) {
    if !ctx.validation_enabled() {
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — skipping the clean-oracle assert");
        return;
    }
    let state = ctx
        .debug_state()
        .expect("validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during the deferred render — see the [vk-validation] log",
        state.total()
    );
}

/// The byte index of texel `(x, y)` in the tightly-packed R8G8B8A8 readback.
fn texel_base(x: u32, y: u32) -> usize {
    ((y * WIDTH + x) * 4) as usize
}

/// Asserts texel `got` is within [`TOL`] per RGB channel of `want`, with an exact A.
fn assert_texel_close(got: [u8; 4], want: [u8; 4], label: &str) {
    for c in 0..3 {
        let diff = (got[c] as i32 - want[c] as i32).abs();
        assert!(
            diff <= TOL,
            "{label}: channel {c} off by {diff} (> {TOL}): got {got:02x?}, want ~{want:02x?}"
        );
    }
    assert_eq!(got[3], want[3], "{label}: alpha must match exactly: got {got:02x?}, want {want:02x?}");
}

/// The full-surface render area reused by both passes.
fn full_area() -> RenderArea {
    RenderArea {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    }
}

/// The full-surface viewport reused by both passes.
fn full_viewport() -> Viewport {
    Viewport {
        x: 0.0,
        y: 0.0,
        width: WIDTH as f32,
        height: HEIGHT as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    }
}

/// Renders the deferred scene (geometry MRT pass -> barrier -> lighting pass) and
/// returns the final output texture `O`'s readback bytes.
fn render_deferred(device: &VulkanContext) -> Vec<u8> {
    let queue = device.rhi_queue();

    // --- The G-buffer attachments: albedo + normal (COLOR_ATTACHMENT for the geometry
    //     pass, SAMPLED for the lighting pass). Both R8G8B8A8_UNORM. ---
    let albedo_tex = device
        .create_texture(&TextureDesc {
            width: WIDTH,
            height: HEIGHT,
            depth: 1,
            format: Format::R8G8B8A8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
        })
        .expect("G-buffer albedo texture");
    let normal_tex = device
        .create_texture(&TextureDesc {
            width: WIDTH,
            height: HEIGHT,
            depth: 1,
            format: Format::R8G8B8A8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
        })
        .expect("G-buffer normal texture");

    // The depth attachment for the geometry pass (D32_SFLOAT).
    let depth_tex = device
        .create_texture(&TextureDesc {
            width: WIDTH,
            height: HEIGHT,
            depth: 1,
            format: Format::D32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT,
        })
        .expect("G-buffer depth texture");

    // The final output texture `O`: COLOR_ATTACHMENT (lighting render target) +
    // TRANSFER_SRC (golden readback).
    let output = device
        .create_texture(&TextureDesc {
            width: WIDTH,
            height: HEIGHT,
            depth: 1,
            format: Format::R8G8B8A8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_SRC,
        })
        .expect("final output texture O");

    // --- The geometry-pass vertex buffer: a quad (two triangles) facing +Z. Model-
    //     space `[-1, 1]^2`; normal `(0, 0, 1)` for every vertex. Host-visible. ---
    let n = [0.0_f32, 0.0, 1.0];
    let v00 = Vertex { position: [-1.0, -1.0, 0.0], normal: n };
    let v10 = Vertex { position: [1.0, -1.0, 0.0], normal: n };
    let v11 = Vertex { position: [1.0, 1.0, 0.0], normal: n };
    let v01 = Vertex { position: [-1.0, 1.0, 0.0], normal: n };
    // Two CCW triangles covering the quad.
    let vertices = [v00, v10, v11, v00, v11, v01];
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
    // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes (the buffer
    // was created at exactly that size); `vertices` is a distinct, non-overlapping stack
    // array of `vertex_bytes` bytes; the write completes before any submit references the
    // buffer (host-coherent: no explicit flush needed).
    unsafe {
        core::ptr::copy_nonoverlapping(
            vertices.as_ptr().cast::<u8>(),
            vb_ptr.as_ptr(),
            vertex_bytes as usize,
        );
    }

    // --- The geometry (MRT) pipeline: vertex layout (position + normal) + a 64-byte
    //     VERTEX push range (MVP) + TWO color formats (albedo + normal) + a depth
    //     format. The two `color_formats` MUST equal the two G-buffer attachment
    //     formats and the count (W2-b). ---
    let gbuf_vs = device
        .create_shader_module(GBUFFER_VS_SPV.as_words())
        .expect("geometry vertex shader module");
    let gbuf_fs = device
        .create_shader_module(GBUFFER_FS_SPV.as_words())
        .expect("geometry fragment shader module");
    let geom_attributes = [
        VertexAttribute {
            location: 0,
            offset: 0,
            format: VertexFormat::Float32x3,
        },
        VertexAttribute {
            location: 1,
            offset: 12,
            format: VertexFormat::Float32x3,
        },
    ];
    let geometry_pipeline = device
        .create_graphics_pipeline(&GraphicsPipelineDesc {
            vertex_module: &gbuf_vs,
            vertex_entry: c"main",
            fragment_module: &gbuf_fs,
            fragment_entry: c"main",
            // MRT: two color attachments (albedo @ 0, normal @ 1).
            color_formats: &[Format::R8G8B8A8Unorm, Format::R8G8B8A8Unorm],
            depth_format: Some(Format::D32Sfloat),
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: Some(VertexBufferLayout {
                stride: VERTEX_STRIDE,
                attributes: &geom_attributes,
            }),
            push_constant_bytes: MVP_BYTES,
            bind_group_layout: None,
            blend: None,
        })
        .expect("geometry MRT pipeline");

    // --- The lighting pass: a sampler + a 2-binding bind-group layout (albedo + normal)
    //     + the full-screen lighting pipeline (single color attachment = O's format). ---
    let sampler = device
        .create_sampler(&SamplerDesc {
            mag_filter: Filter::Nearest,
            min_filter: Filter::Nearest,
            address_mode: AddressMode::ClampToEdge,
            mip: MipMode::None,
        })
        .expect("sampler");
    let light_layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc {
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    count: 1,
                    kind: DescriptorKind::CombinedImageSampler,
                    stage: ShaderStage::FRAGMENT,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    count: 1,
                    kind: DescriptorKind::CombinedImageSampler,
                    stage: ShaderStage::FRAGMENT,
                },
            ],
        })
        .expect("2-binding bind-group layout");

    let light_vs = device
        .create_shader_module(LIGHT_VS_SPV.as_words())
        .expect("lighting vertex shader module");
    let light_fs = device
        .create_shader_module(LIGHT_FS_SPV.as_words())
        .expect("lighting fragment shader module");
    let lighting_pipeline = device
        .create_graphics_pipeline(&GraphicsPipelineDesc {
            vertex_module: &light_vs,
            vertex_entry: c"main",
            fragment_module: &light_fs,
            fragment_entry: c"main",
            color_formats: &[Format::R8G8B8A8Unorm],
            depth_format: None,
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: None,
            push_constant_bytes: 0,
            bind_group_layout: Some(&light_layout),
            blend: None,
        })
        .expect("deferred lighting pipeline");

    // The bind group binds the two G-buffer views (albedo @ binding 0, normal @
    // binding 1) + the shared sampler. Both textures are sampled in
    // SHADER_READ_ONLY_OPTIMAL (transitioned below before the lighting pass).
    let light_bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &light_layout,
            entries: &[
                BindGroupEntry::CombinedImage {
                    texture: &albedo_tex,
                    sampler: &sampler,
                },
                BindGroupEntry::CombinedImage {
                    texture: &normal_tex,
                    sampler: &sampler,
                },
            ],
        })
        .expect("2-binding bind group (albedo, normal)");

    let staging = device
        .create_buffer(&BufferDesc {
            size: SIZE,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible readback staging buffer");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");
    let full = full_area();
    let viewport = full_viewport();

    encoder.begin().expect("begin");

    // --- Geometry pass: transition albedo + normal -> COLOR_ATTACHMENT, depth ->
    //     DEPTH_ATTACHMENT, then draw the quad into the 2-attachment G-buffer. ---
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &albedo_tex,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::ColorAttachmentOptimal,
        range: ImageSubresourceRange::COLOR,
    });
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &normal_tex,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::ColorAttachmentOptimal,
        range: ImageSubresourceRange::COLOR,
    });
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &depth_tex,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::EARLY_FRAGMENT_TESTS | BarrierStage::LATE_FRAGMENT_TESTS,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::DEPTH_STENCIL_ATTACHMENT_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::DepthAttachmentOptimal,
        range: ImageSubresourceRange::DEPTH,
    });

    let gbuffer_colors = [
        RenderingAttachment {
            texture: &albedo_tex,
            layout: ImageLayout::ColorAttachmentOptimal,
            load_op: LoadOp::Clear,
            store_op: StoreOp::Store,
            clear_color: ALBEDO_CLEAR,
        },
        RenderingAttachment {
            texture: &normal_tex,
            layout: ImageLayout::ColorAttachmentOptimal,
            load_op: LoadOp::Clear,
            store_op: StoreOp::Store,
            clear_color: NORMAL_CLEAR,
        },
    ];
    encoder.begin_rendering(&RenderingDesc {
        render_area: full,
        colors: &gbuffer_colors,
        depth: Some(boyko_rhi::DepthAttachment {
            texture: &depth_tex,
            layout: ImageLayout::DepthAttachmentOptimal,
            load_op: LoadOp::Clear,
            store_op: StoreOp::Store,
            clear_depth: 1.0,
        }),
    });
    encoder.bind_graphics_pipeline(&geometry_pipeline);
    encoder.push_graphics_constants(&geometry_pipeline, ShaderStage::VERTEX, 0, &mvp_bytes());
    encoder.bind_vertex_buffer(&vertex_buffer, 0, 0);
    encoder.set_viewport(&viewport);
    encoder.set_scissor(&full);
    encoder.draw(6, 1, 0, 0);
    encoder.end_rendering();

    // --- Barrier: BOTH G-buffer color textures COLOR_ATTACHMENT_OPTIMAL ->
    //     SHADER_READ_ONLY_OPTIMAL (the lighting pass SAMPLES them). ---
    for tex in [&albedo_tex, &normal_tex] {
        encoder.image_barrier(&ImageBarrierDesc {
            texture: tex,
            src_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
            dst_stage: BarrierStage::FRAGMENT_SHADER,
            src_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
            dst_access: BarrierAccess::SHADER_READ,
            old_layout: ImageLayout::ColorAttachmentOptimal,
            new_layout: ImageLayout::ShaderReadOnlyOptimal,
            range: ImageSubresourceRange::COLOR,
        });
    }

    // --- Lighting pass: transition O -> COLOR_ATTACHMENT, full-screen sample + light. ---
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &output,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::ColorAttachmentOptimal,
        range: ImageSubresourceRange::COLOR,
    });
    let output_attachment = [RenderingAttachment {
        texture: &output,
        layout: ImageLayout::ColorAttachmentOptimal,
        load_op: LoadOp::Clear,
        store_op: StoreOp::Store,
        clear_color: OUTPUT_CLEAR,
    }];
    encoder.begin_rendering(&RenderingDesc {
        render_area: full,
        colors: &output_attachment,
        depth: None,
    });
    encoder.bind_graphics_pipeline(&lighting_pipeline);
    encoder.bind_descriptor_set(&light_bind_group, &lighting_pipeline);
    encoder.set_viewport(&viewport);
    encoder.set_scissor(&full);
    encoder.draw(3, 1, 0, 0);
    encoder.end_rendering();

    // --- O: COLOR_ATTACHMENT_OPTIMAL -> TRANSFER_SRC_OPTIMAL for the readback. ---
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &output,
        src_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
        dst_access: BarrierAccess::TRANSFER_READ,
        old_layout: ImageLayout::ColorAttachmentOptimal,
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
        image_extent_w: WIDTH,
        image_extent_h: HEIGHT,
        image_extent_d: 1,
    }];
    encoder.copy_image_to_buffer(&output, ImageLayout::TransferSrcOptimal, &staging, &regions);

    encoder.end().expect("end");

    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    // Read back the staging buffer.
    let dst_ptr = device
        .buffer_mapped_ptr(&staging)
        .expect("host-visible staging buffer is mapped");
    let mut out = vec![0u8; SIZE as usize];
    // SAFETY: `dst_ptr` points to `SIZE` mapped host-coherent bytes; a fence wait
    // preceded this read, so the GPU draws + copy are complete + coherent; reading
    // `SIZE` bytes is in-bounds; `out` is a distinct, non-overlapping allocation.
    unsafe {
        core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), SIZE as usize);
    }

    // Teardown. The encoder's last submission completed (fence-waited above).
    // SAFETY: each resource was created on `device`, its GPU work has completed (the
    // fence was waited), and each is destroyed exactly once here, in reverse
    // dependency order (encoder/fence/buffer, then the lighting bind group + its layout
    // + sampler, then both pipelines + their modules, then the vertex buffer + textures).
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_buffer(staging);
        device.destroy_bind_group(light_bind_group);
        device.destroy_graphics_pipeline(lighting_pipeline);
        device.destroy_shader_module(light_fs);
        device.destroy_shader_module(light_vs);
        device.destroy_bind_group_layout(light_layout);
        device.destroy_sampler(sampler);
        device.destroy_graphics_pipeline(geometry_pipeline);
        device.destroy_shader_module(gbuf_fs);
        device.destroy_shader_module(gbuf_vs);
        device.destroy_buffer(vertex_buffer);
        device.destroy_texture(output);
        device.destroy_texture(depth_tex);
        device.destroy_texture(normal_tex);
        device.destroy_texture(albedo_tex);
    }

    out
}

#[test]
fn deferred_gbuffer_lighting_golden() {
    let Some(ctx) = boot_or_skip("deferred_gbuffer_lighting_golden") else {
        return;
    };
    println!("Vulkan device: {}", ctx.device_name());
    // Pixel golden: runs with or without validation (the clean-oracle assert
    // self-gates when validation is disabled via BOYKO_DISABLE_VALIDATION).
    if !ctx.validation_enabled() {
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — deferred golden still runs");
    }

    let device: &VulkanContext = &ctx;
    let out = render_deferred(device);

    // The CENTRE texel is covered by the MVP-scaled quad (N·L = 1), so its lit color is
    // `albedo * 1 + ambient`. Compute the expected from the SAME albedo + ambient the
    // shaders use; the albedo is first quantized into the G-buffer, so reproduce that
    // round-trip on the host for an accurate golden (a small TOL covers rounding slack).
    let want_centre = {
        let mut px = [0u8; 4];
        for c in 0..3 {
            // Albedo quantized into the G-buffer, sampled back, lit, re-quantized.
            let gbuf_albedo = to_unorm(ALBEDO[c]) as f32 / 255.0;
            let lit = gbuf_albedo * 1.0 + AMBIENT;
            px[c] = to_unorm(lit);
        }
        px[3] = 0xFF;
        px
    };
    let centre = texel_base(WIDTH / 2, HEIGHT / 2);
    let centre_texel = [
        out[centre],
        out[centre + 1],
        out[centre + 2],
        out[centre + 3],
    ];
    assert_texel_close(
        centre_texel,
        want_centre,
        "centre must be the LIT quad color (albedo*N·L + ambient): the deferred chain (geometry MRT -> barrier -> 2-sample lighting) round-tripped",
    );

    // The (0, 0) CORNER is NOT covered by the quad: the G-buffer there holds albedo 0 +
    // a normal that unpacks to N = 0 (so N·L = 0), so the lit value is `0 + ambient`.
    let want_corner = {
        let amb = to_unorm(AMBIENT);
        [amb, amb, amb, 0xFF]
    };
    let corner = texel_base(0, 0);
    let corner_texel = [
        out[corner],
        out[corner + 1],
        out[corner + 2],
        out[corner + 3],
    ];
    assert_texel_close(
        corner_texel,
        want_corner,
        "corner must be the lighting-pass background (ambient only: no geometry, so N·L = 0)",
    );

    assert_validation_clean(&ctx);

    drop(ctx);
}
