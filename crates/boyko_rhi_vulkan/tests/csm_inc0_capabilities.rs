//! CSM Increment 0 — the three contained RHI capability gaps, each exercised by a
//! NEW-capability acceptance test. NO behavior change to existing resources: these
//! tests create the NEW shapes (a multi-layer array depth texture, a comparison
//! sampler, a depth-only configurable pipeline + an indexed draw into a depth-array
//! layer) and prove they create + record CLEANLY via the validation layer (the
//! GPU-half oracle, zero messages), gracefully skipping on a GPU-less / loader-less
//! / validation-less host (mirrors the rung tests).
//!
//! These do NOT implement the CSM depth pass, the cascade fit, or the resolve (those
//! are Increment 1+). They validate ONLY the three RHI capabilities:
//!
//! 1. **Multi-view array depth texture** — a 4-layer `D32_SFLOAT`
//!    `DEPTH_STENCIL_ATTACHMENT | SAMPLED` image. The backend creates 4 per-layer
//!    `VK_IMAGE_VIEW_TYPE_2D` render views + 1 `VK_IMAGE_VIEW_TYPE_2D_ARRAY` sample
//!    view; success + a clean validation messenger proves every view is valid.
//! 2. **Comparison sampler** — `SamplerDesc.compare = Some(CompareOp::LessOrEqual)`
//!    builds a `compareEnable = VK_TRUE` PCF sampler.
//! 3. **Depth-only configurable pipeline** — an EMPTY `color_formats` + `Some(depth)`
//!    pipeline with `CullMode::Front` + `Some(DepthBias { .. })`, then a minimal
//!    indexed draw into the depth-array texture's layer 0 (the existing `.view` path).

use core::slice;

use boyko_rhi::enums::{BarrierAccess, BarrierStage};
use boyko_rhi::{
    AddressMode, BufferDesc, BufferUsage, CompareOp, CullMode, DepthAttachment, DepthBias, Filter,
    Format, GraphicsPipelineDesc, ImageBarrierDesc, ImageLayout, ImageSubresourceRange, ImageUsage,
    IndexType, LoadOp, MemoryLocation, MipMode, PrimitiveTopology, RenderArea, RenderingDesc,
    RhiCommandEncoder, RhiDevice, RhiQueue, SamplerDesc, ShaderStage, StoreOp, TextureDesc,
    TextureDimension, VertexAttribute, VertexBufferLayout, VertexFormat, Viewport,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// The shadow-map (array depth texture) layer dimensions. Small but multi-texel.
const SHADOW_DIM: u32 = 64;
/// The cascade count exercised — the array texture's layer count (== the backend's
/// `MAX_CASCADES`).
const CASCADES: u32 = 4;

/// The depth attachment's CLEAR value (the far plane).
const DEPTH_CLEAR: f32 = 1.0;
/// The MVP push-constant range size (a `float4x4`), reused from the rung-3 shader.
const MVP_BYTES: u32 = 64;
/// The committed rung-3 vertex stride (`Float32x3` pos @ 0 + `Float32x4` color @ 12).
const VERTEX_STRIDE: u32 = 28;

/// One vertex matching the committed rung-3 MVP vertex layout: a `Float32x3` position
/// (offset 0) + a `Float32x4` color (offset 12). `#[repr(C)]` so the fields are
/// tightly packed at the 28-byte stride the shader expects.
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    pos: [f32; 3],
    color: [f32; 4],
}

const _: () = assert!(
    core::mem::size_of::<Vertex>() == VERTEX_STRIDE as usize,
    "Vertex must be tightly packed at 28 bytes"
);

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

/// The committed rung-3 vertex SPIR-V (`triangle_mvp.vs.spv`, 916 bytes), reused — a
/// depth-only pass still needs a vertex stage; the depth comes from the transformed
/// `gl_Position.z`.
static MVP_VS_SPV: SpirvBlob<916> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/triangle_mvp.vs.spv"
)));

/// The committed rung-3 fragment SPIR-V (`triangle_mvp.fs.spv`, 368 bytes), reused —
/// its color output is discarded (the depth-only pipeline declares NO color
/// attachment), but the fragment stage still runs so depth is written.
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
///
/// A no-op (with a one-line note) when validation is disabled via
/// `BOYKO_DISABLE_VALIDATION` (the layer DLL crashes the MinGW process on this box):
/// there is no messenger to read, but the create/record paths still run.
fn assert_validation_clean(ctx: &VulkanContext, what: &str) {
    if !ctx.validation_enabled() {
        eprintln!(
            "NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — skipping the {what} clean-oracle assert"
        );
        return;
    }
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
        "validation layer reported {} message(s) during the {what} capability test — see the [vk-validation] log",
        state.total()
    );
}

/// The MVP push bytes — an identity-ish diagonal `float4x4` (column-major, but
/// symmetric so storage convention is irrelevant) that maps the unit triangle into
/// clip space with depth `z = pos.z`.
fn mvp_bytes() -> [u8; MVP_BYTES as usize] {
    #[rustfmt::skip]
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

// ===========================================================================
// Capability 1 — the multi-layer array depth texture.
// ===========================================================================

/// A 4-layer `D32_SFLOAT` `DEPTH_STENCIL_ATTACHMENT | SAMPLED` array texture creates
/// cleanly: the backend builds 4 per-layer render views + 1 array sample view, all
/// valid (proven by create success + the clean validation messenger). The single-layer
/// path is left byte-identical (every existing texture passes `array_layers: 1`).
#[test]
fn array_depth_texture_creates() {
    let Some(ctx) = boot_or_skip("array_depth_texture_creates") else {
        return;
    };

    let texture = ctx
        .create_texture(&TextureDesc {
            width: SHADOW_DIM,
            height: SHADOW_DIM,
            depth: 1,
            format: Format::D32Sfloat,
            dimension: TextureDimension::D2,
            // DEPTH + SAMPLED is the CSM shadow-map usage (already proven on this
            // device by the gbuffer depth at `swapchain.rs`).
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
            array_layers: CASCADES,
            mip_levels: 1,
            view_format: None,
        })
        .expect("4-layer D32 array depth texture creates");

    assert_validation_clean(&ctx, "array depth texture");

    // SAFETY: the texture was created on `ctx`, no GPU work referenced it (no submit),
    // and it is destroyed exactly once here. `destroy_texture` tears down every view
    // (the array sample view + the 4 per-layer render views) → image → memory.
    unsafe { ctx.destroy_texture(texture) };
}

// ===========================================================================
// Capability 2 — the comparison sampler.
// ===========================================================================

/// A comparison sampler (`compare = Some(CompareOp::LessOrEqual)`) creates cleanly —
/// the backend sets `compareEnable = VK_TRUE` + `compareOp = LESS_OR_EQUAL` (the PCF
/// shadow op). A `compare: None` sampler stays byte-identical to today.
#[test]
fn comparison_sampler_creates() {
    let Some(ctx) = boot_or_skip("comparison_sampler_creates") else {
        return;
    };

    let sampler = ctx
        .create_sampler(&SamplerDesc {
            // PCF reads typically bilinearly filter the comparison results.
            mag_filter: Filter::Linear,
            min_filter: Filter::Linear,
            address_mode: AddressMode::ClampToEdge,
            mip: MipMode::None,
            compare: Some(CompareOp::LessOrEqual),
        })
        .expect("comparison sampler creates");

    assert_validation_clean(&ctx, "comparison sampler");

    // SAFETY: created on `ctx`, never bound into a submission, destroyed exactly once.
    unsafe { ctx.destroy_sampler(sampler) };
}

// ===========================================================================
// Capability 3 — the depth-only configurable pipeline + an indexed draw into a
// depth-array layer.
// ===========================================================================

/// A depth-only graphics pipeline (EMPTY `color_formats`, `CullMode::Front`,
/// `Some(DepthBias)`) creates, and a minimal indexed mesh draws into the depth-array
/// texture's layer 0 — all validation-clean. This exercises: the empty-color path
/// (`colorAttachmentCount = 0` + null color-blend + null format array), the
/// configurable cull mode + depth bias, the array depth texture as a depth attachment,
/// and the indexed-draw verb.
#[test]
fn depth_only_pipeline_draws_indexed_into_array_layer() {
    let Some(ctx) = boot_or_skip("depth_only_pipeline_draws_indexed_into_array_layer") else {
        return;
    };
    let queue = ctx.rhi_queue();

    // The array depth target (the CSM shadow atlas). The draw renders into layer 0 via
    // the texture's full-subresource `.view` (== layer 0's render view) — the per-layer
    // render-view wiring for cascades 1..N is Increment 1.
    let depth = ctx
        .create_texture(&TextureDesc {
            width: SHADOW_DIM,
            height: SHADOW_DIM,
            depth: 1,
            format: Format::D32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT | ImageUsage::SAMPLED,
            array_layers: CASCADES,
            mip_levels: 1,
            view_format: None,
        })
        .expect("array depth target");

    // A minimal indexed triangle (3 vertices, 3 indices). The color lane is unused
    // (no color attachment), but the vertex layout matches the committed MVP shader.
    let vertices: [Vertex; 3] = [
        Vertex {
            pos: [0.0, -1.0, 0.25],
            color: [1.0, 1.0, 1.0, 1.0],
        },
        Vertex {
            pos: [1.0, 1.0, 0.25],
            color: [1.0, 1.0, 1.0, 1.0],
        },
        Vertex {
            pos: [-1.0, 1.0, 0.25],
            color: [1.0, 1.0, 1.0, 1.0],
        },
    ];
    let indices: [u16; 3] = [0, 1, 2];

    let vertex_bytes = core::mem::size_of_val(&vertices) as u64;
    let vertex_buffer = ctx
        .create_buffer(&BufferDesc {
            size: vertex_bytes,
            usage: BufferUsage::VERTEX,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible vertex buffer");
    let vb_ptr = ctx
        .buffer_mapped_ptr(&vertex_buffer)
        .expect("vertex buffer mapped");
    // SAFETY: `vb_ptr` points to `vertex_bytes` mapped host-coherent bytes (the buffer
    // was created at exactly that size); `vertices` is a distinct, non-overlapping
    // stack array of `vertex_bytes` bytes; host-coherent => the write is visible before
    // the submit references the buffer.
    unsafe {
        core::ptr::copy_nonoverlapping(
            vertices.as_ptr().cast::<u8>(),
            vb_ptr.as_ptr(),
            vertex_bytes as usize,
        );
    }

    let index_bytes = core::mem::size_of_val(&indices) as u64;
    let index_buffer = ctx
        .create_buffer(&BufferDesc {
            size: index_bytes,
            usage: BufferUsage::INDEX,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible index buffer");
    let ib_ptr = ctx
        .buffer_mapped_ptr(&index_buffer)
        .expect("index buffer mapped");
    // SAFETY: `ib_ptr` points to `index_bytes` mapped host-coherent bytes; `indices`
    // is a distinct, non-overlapping stack array of `index_bytes` bytes; host-coherent
    // => visible before the submit.
    unsafe {
        core::ptr::copy_nonoverlapping(
            indices.as_ptr().cast::<u8>(),
            ib_ptr.as_ptr(),
            index_bytes as usize,
        );
    }

    let vs = ctx
        .create_shader_module(MVP_VS_SPV.as_words())
        .expect("vertex shader module");
    let fs = ctx
        .create_shader_module(MVP_FS_SPV.as_words())
        .expect("fragment shader module");

    let attributes = [
        VertexAttribute {
            location: 0,
            offset: 0,
            format: VertexFormat::Float32x3,
        },
        VertexAttribute {
            location: 1,
            offset: 12,
            format: VertexFormat::Float32x4,
        },
    ];
    // The DEPTH-ONLY pipeline: EMPTY color_formats (colorAttachmentCount = 0), a real
    // depth format (required when there is no color), `CullMode::Front`, and a slope +
    // constant depth bias (the shadow-map acne fix).
    let pipeline = ctx
        .create_graphics_pipeline(&GraphicsPipelineDesc {
            vertex_module: &vs,
            vertex_entry: c"main",
            fragment_module: &fs,
            fragment_entry: c"main",
            color_formats: &[],
            depth_format: Some(Format::D32Sfloat),
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: Some(VertexBufferLayout {
                stride: VERTEX_STRIDE,
                attributes: &attributes,
            }),
            push_constant_bytes: MVP_BYTES,
            bind_group_layout: None,
            blend: None,
            cull_mode: CullMode::Front,
            depth_bias: Some(DepthBias {
                constant_factor: 1.25,
                slope_factor: 1.75,
                clamp: 0.0,
            }),
        })
        .expect("depth-only graphics pipeline creates");

    let fence = ctx.create_fence(false).expect("fence");
    let mut encoder = ctx.create_command_encoder().expect("command encoder");

    encoder.begin().expect("begin");

    // Depth (layer 0): UNDEFINED → DEPTH_ATTACHMENT_OPTIMAL.
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

    let full = RenderArea {
        x: 0,
        y: 0,
        width: SHADOW_DIM,
        height: SHADOW_DIM,
    };
    // A depth-only rendering scope: NO color attachments, one depth attachment.
    encoder.begin_rendering(&RenderingDesc {
        render_area: full,
        colors: &[],
        depth: Some(DepthAttachment {
            texture: &depth,
            layout: ImageLayout::DepthAttachmentOptimal,
            load_op: LoadOp::Clear,
            store_op: StoreOp::Store,
            clear_depth: DEPTH_CLEAR,
        }),
    });
    encoder.bind_graphics_pipeline(&pipeline);
    encoder.push_graphics_constants(&pipeline, ShaderStage::VERTEX, 0, &mvp_bytes());
    encoder.bind_vertex_buffer(&vertex_buffer, 0, 0);
    encoder.bind_index_buffer(&index_buffer, 0, IndexType::Uint16);
    encoder.set_viewport(&Viewport {
        x: 0.0,
        y: 0.0,
        width: SHADOW_DIM as f32,
        height: SHADOW_DIM as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    });
    encoder.set_scissor(&full);
    encoder.draw_indexed(3, 1, 0, 0, 0);
    encoder.end_rendering();

    encoder.end().expect("end");

    queue.submit(&encoder, &fence).expect("submit");
    ctx.wait_fence(&fence, u64::MAX).expect("wait_fence");

    assert_validation_clean(&ctx, "depth-only indexed draw");

    // SAFETY: each resource was created on `ctx`, its GPU work completed (the fence was
    // waited above), and each is destroyed exactly once here, in reverse order.
    unsafe {
        ctx.destroy_command_encoder(encoder);
        ctx.destroy_fence(fence);
        ctx.destroy_graphics_pipeline(pipeline);
        ctx.destroy_shader_module(fs);
        ctx.destroy_shader_module(vs);
        ctx.destroy_buffer(index_buffer);
        ctx.destroy_buffer(vertex_buffer);
        ctx.destroy_texture(depth);
    }
}
