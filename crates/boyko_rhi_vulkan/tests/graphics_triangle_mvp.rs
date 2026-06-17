//! Phase-6 Slice S0 — RUNG 3 acceptance: a real triangle read from a VERTEX BUFFER
//! and positioned by an MVP PUSH CONSTANT, rasterized into an offscreen color
//! attachment via the Vulkan 1.3 dynamic-rendering graphics pipeline, proven by a
//! golden image readback, with the validation layer as the soundness oracle.
//!
//! This is the next bounded step beyond rung 2's in-shader (`SV_VertexID`) triangle:
//! the geometry now comes from a device buffer (position + per-vertex color) and is
//! transformed by a 4x4 MVP matrix supplied as a `VERTEX`-stage push constant. There
//! is still NO depth attachment (rung 4) and NO descriptor sets / samplers.
//!
//! The flow:
//!
//! 1. Boot a headless, validation-enabled device (the test skips on `Err`).
//! 2. `create_texture` an R8G8B8A8_UNORM 2D color image
//!    (`COLOR_ATTACHMENT | TRANSFER_SRC`).
//! 3. Upload the vertex buffer: a host-visible `VERTEX`-usage buffer written
//!    directly through its mapped pointer (the buffer-create + host-coherent write
//!    path the compute/readback tests use; a host-visible vertex buffer needs no
//!    staging `copy_buffer`).
//! 4. `create_graphics_pipeline` with the rung-3 vertex layout (binding stride 28:
//!    position `Float32x3` @ 0 + color `Float32x4` @ 12) + a 64-byte `VERTEX` push
//!    range (the MVP `float4x4`).
//! 5. Record: `image_barrier` UNDEFINED → COLOR_ATTACHMENT_OPTIMAL →
//!    `begin_rendering` (loadOp = CLEAR) → `bind_graphics_pipeline` +
//!    `push_graphics_constants(MVP)` + `bind_vertex_buffer` + dynamic
//!    viewport/scissor → `draw(3, 1, 0, 0)` → `end_rendering` → `image_barrier`
//!    COLOR → TRANSFER_SRC → `copy_image_to_buffer`.
//! 6. Submit once, fence-wait, map-read the staging buffer.
//! 7. Assert the CENTRE texel (which the MVP-transformed triangle covers) equals the
//!    per-vertex color, AND a CORNER texel (uncovered) equals the CLEAR color.
//! 8. Assert the validation messenger recorded ZERO messages.
//!
//! # The MVP (deterministic + transpose-robust)
//!
//! Model-space vertices: `(0, -1, 0)`, `(1, 1, 0)`, `(-1, 1, 0)` (a triangle
//! spanning the model square `[-1, 1]`). The MVP is the DIAGONAL matrix
//! `diag(0.7, 0.7, 1, 1)`, so `mul(mvp, float4(pos, 1))` maps the vertices to NDC
//! `(0, -0.7)`, `(0.7, 0.7)`, `(-0.7, 0.7)` — exactly rung 2's covering triangle
//! (covers the centre, misses the corners). A diagonal matrix is symmetric, so the
//! 16 packed floats are identical whether DXC stores the `float4x4` row- or
//! column-major: the golden is insensitive to the matrix-storage convention, and a
//! BYPASSED MVP (e.g. identity left in by a wiring bug) would map the vertices to the
//! full `[-1, 1]` NDC and fill the corner too — which the corner==clear assert
//! catches. The transform is therefore genuinely exercised, not cosmetic.
//!
//! # CI gate (graceful skip)
//!
//! A GPU-less / loader-less host, or one without the validation layer / dynamic
//! rendering, makes `VulkanContext::boot` return `Err`; the test skips gracefully
//! (mirrors rungs 1-2).

use core::slice;

use boyko_rhi::enums::{BarrierAccess, BarrierStage};
use boyko_rhi::{
    BufferDesc, BufferImageCopy, BufferUsage, Format, GraphicsPipelineDesc, ImageAspect,
    ImageBarrierDesc, ImageLayout, ImageSubresourceRange, ImageUsage, LoadOp, MemoryLocation,
    PrimitiveTopology, RenderArea, RenderingAttachment, RenderingDesc, RhiCommandEncoder,
    RhiDevice, RhiQueue, ShaderStage, StoreOp, TextureDesc, VertexAttribute, VertexBufferLayout,
    VertexFormat, Viewport,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// The offscreen image dimensions. Small but multi-texel so a covered/uncovered
/// boundary is unambiguous.
const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;
const TEXELS: usize = (WIDTH * HEIGHT) as usize;
const SIZE: u64 = (TEXELS * 4) as u64;

/// The CLEAR color bytes (R, G, B, A) for R8G8B8A8_UNORM — the value an UNCOVERED
/// texel keeps. Each byte is `byte / 255.0` so the float→byte UNORM conversion is
/// exact.
const CLEAR_BYTES: [u8; 4] = [0xAA, 0xBB, 0xCC, 0xDD];

/// The per-vertex color bytes a COVERED texel takes — opaque green `(0, 1, 0, 1)` →
/// `0x00 0xFF 0x00 0xFF`. Distinct from `CLEAR_BYTES` so covered vs uncovered cannot
/// be confused, and distinct from rung-2's red so a stale-pipeline mixup is visible.
const VERTEX_COLOR_BYTES: [u8; 4] = [0x00, 0xFF, 0x00, 0xFF];

/// One vertex: a `Float32x3` position (offset 0) + a `Float32x4` color (offset 12),
/// matching the rung-3 vertex layout. `#[repr(C)]` so the field layout is the exact
/// 28-byte stride the layout declares (3 + 4 = 7 f32, tightly packed).
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 3],
    color: [f32; 4],
}

/// The per-vertex stride the layout declares (3 + 4 floats = 28 bytes).
const VERTEX_STRIDE: u32 = core::mem::size_of::<Vertex>() as u32;
const _: () = assert!(VERTEX_STRIDE == 28, "Vertex must be tightly packed at 28 bytes");

/// The MVP byte size (a `float4x4`).
const MVP_BYTES: u32 = 64;

/// The clear color as the RGBA floats `begin_rendering` takes.
fn clear_floats() -> [f32; 4] {
    [
        CLEAR_BYTES[0] as f32 / 255.0,
        CLEAR_BYTES[1] as f32 / 255.0,
        CLEAR_BYTES[2] as f32 / 255.0,
        CLEAR_BYTES[3] as f32 / 255.0,
    ]
}

/// The per-vertex color as the RGBA floats stored in each vertex.
fn vertex_color_floats() -> [f32; 4] {
    [
        VERTEX_COLOR_BYTES[0] as f32 / 255.0,
        VERTEX_COLOR_BYTES[1] as f32 / 255.0,
        VERTEX_COLOR_BYTES[2] as f32 / 255.0,
        VERTEX_COLOR_BYTES[3] as f32 / 255.0,
    ]
}

/// The deterministic MVP: `diag(0.7, 0.7, 1, 1)` packed column-major (identical to
/// row-major because the matrix is diagonal — see the module-level MVP note). It
/// scales the model-space `[-1, 1]` triangle to the `[-0.7, 0.7]` covering triangle.
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

// --- Committed rung-3 SPIR-V, embedded at compile time (the proven shader pattern:
//     a `#[repr(C, align(4))]` blob re-viewed as a `&[u32]` word stream). Authored as
//     HLSL → DXC → committed `.spv`; see the `shaders/triangle_mvp.*.hlsl` headers
//     for the exact DXC invocation. ---

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

/// The committed rung-3 vertex SPIR-V (`triangle_mvp.vs.spv`, 916 bytes).
static MVP_VS_SPV: SpirvBlob<916> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/triangle_mvp.vs.spv"
)));

/// The committed rung-3 fragment SPIR-V (`triangle_mvp.fs.spv`, 368 bytes).
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
        .expect("validation enabled => a debug-messenger state is present");
    assert_eq!(
        state.total(),
        0,
        "validation layer reported {} message(s) during the MVP triangle draw — see the [vk-validation] log",
        state.total()
    );
}

/// The byte index of texel `(x, y)` in the tightly-packed R8G8B8A8 readback.
fn texel_base(x: u32, y: u32) -> usize {
    ((y * WIDTH + x) * 4) as usize
}

#[test]
fn vertex_buffer_mvp_triangle_golden_round_trip() {
    let Some(ctx) = boot_or_skip("vertex_buffer_mvp_triangle_golden_round_trip") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    assert!(ctx.validation_enabled(), "validation must be active");

    let device: &VulkanContext = &ctx;
    let queue = ctx.rhi_queue();

    // The offscreen color image. Its format MUST equal the pipeline's declared
    // `color_format` (the W2-b contract).
    let color = device
        .create_texture(&TextureDesc {
            width: WIDTH,
            height: HEIGHT,
            depth: 1,
            format: Format::R8G8B8A8Unorm,
            dimension: boyko_rhi::TextureDimension::D2,
            usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_SRC,
        })
        .expect("offscreen color texture");

    // The vertex buffer: three vertices (model-space triangle + per-vertex color),
    // host-visible so the data is written directly through its mapped pointer.
    let col = vertex_color_floats();
    let vertices = [
        Vertex { position: [0.0, -1.0, 0.0], color: col },
        Vertex { position: [1.0, 1.0, 0.0], color: col },
        Vertex { position: [-1.0, 1.0, 0.0], color: col },
    ];
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
    // any submit references the buffer (host-coherent: no explicit flush needed).
    unsafe {
        core::ptr::copy_nonoverlapping(
            vertices.as_ptr().cast::<u8>(),
            vb_ptr.as_ptr(),
            vertex_bytes as usize,
        );
    }

    // The committed vertex + fragment shader modules.
    let vs = device
        .create_shader_module(MVP_VS_SPV.as_words())
        .expect("vertex shader module");
    let fs = device
        .create_shader_module(MVP_FS_SPV.as_words())
        .expect("fragment shader module");

    // The rung-3 graphics pipeline: vertex layout (position + color) + a 64-byte
    // VERTEX push range (the MVP). `color_format` equals the attachment's format.
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
    let pipeline = device
        .create_graphics_pipeline(&GraphicsPipelineDesc {
            vertex_module: &vs,
            vertex_entry: c"main",
            fragment_module: &fs,
            fragment_entry: c"main",
            color_format: Format::R8G8B8A8Unorm,
            depth_format: None,
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: Some(VertexBufferLayout {
                stride: VERTEX_STRIDE,
                attributes: &attributes,
            }),
            push_constant_bytes: MVP_BYTES,
            // Rung 3 binds no descriptor sets (the rung-5 bind-group-layout seam).
            bind_group_layout: None,
        })
        .expect("graphics pipeline");

    let staging = device
        .create_buffer(&BufferDesc {
            size: SIZE,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible readback staging buffer");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");

    encoder.begin().expect("begin");

    // UNDEFINED → COLOR_ATTACHMENT_OPTIMAL (the acquire→render transition).
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

    let attachment = [RenderingAttachment {
        texture: &color,
        layout: ImageLayout::ColorAttachmentOptimal,
        load_op: LoadOp::Clear,
        store_op: StoreOp::Store,
        clear_color: clear_floats(),
    }];
    let full = RenderArea {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    };
    encoder.begin_rendering(&RenderingDesc {
        render_area: full,
        colors: &attachment,
        depth: None,
    });
    encoder.bind_graphics_pipeline(&pipeline);
    // Push the MVP to the pipeline's VERTEX-stage range, then bind the vertex buffer.
    encoder.push_graphics_constants(&pipeline, ShaderStage::VERTEX, 0, &mvp_bytes());
    encoder.bind_vertex_buffer(&vertex_buffer, 0, 0);
    encoder.set_viewport(&Viewport {
        x: 0.0,
        y: 0.0,
        width: WIDTH as f32,
        height: HEIGHT as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    });
    encoder.set_scissor(&full);
    encoder.draw(3, 1, 0, 0);
    encoder.end_rendering();

    // COLOR_ATTACHMENT_OPTIMAL → TRANSFER_SRC_OPTIMAL for the readback copy.
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &color,
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
    encoder.copy_image_to_buffer(&color, ImageLayout::TransferSrcOptimal, &staging, &regions);

    encoder.end().expect("end");

    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    // Read back the staging buffer.
    let dst_ptr = device
        .buffer_mapped_ptr(&staging)
        .expect("host-visible staging buffer is mapped");
    // SAFETY: `dst_ptr` points to `SIZE` mapped host-coherent bytes; a fence wait
    // preceded this read, so the GPU draw + copy are complete + coherent; reading
    // `SIZE` bytes is in-bounds; `out` is a distinct, non-overlapping allocation.
    let mut out = vec![0u8; SIZE as usize];
    unsafe {
        core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), SIZE as usize);
    }

    // The CENTRE texel: the MVP-transformed triangle (NDC (0,-0.7),(0.7,0.7),
    // (-0.7,0.7)) covers the image centre, so this texel must equal the per-vertex
    // color — proving the vertex buffer + MVP transform genuinely rasterized.
    let centre = texel_base(WIDTH / 2, HEIGHT / 2);
    let centre_texel = [
        out[centre],
        out[centre + 1],
        out[centre + 2],
        out[centre + 3],
    ];
    assert_eq!(
        centre_texel, VERTEX_COLOR_BYTES,
        "centre texel must be the per-vertex color (MVP triangle covers it): got {centre_texel:02x?}, want {VERTEX_COLOR_BYTES:02x?}"
    );

    // The (0, 0) CORNER texel: the triangle does NOT reach the corners (a bypassed /
    // identity MVP would map the model triangle to full [-1,1] NDC and fill it), so
    // this texel must keep the CLEAR color — the covered-vs-clear pair proves a real
    // shape AND that the MVP scale was applied.
    let corner = texel_base(0, 0);
    let corner_texel = [
        out[corner],
        out[corner + 1],
        out[corner + 2],
        out[corner + 3],
    ];
    assert_eq!(
        corner_texel, CLEAR_BYTES,
        "corner texel must be the clear color (MVP triangle does NOT cover it): got {corner_texel:02x?}, want {CLEAR_BYTES:02x?}"
    );

    // The oracle: a clean run records zero validation messages.
    assert_validation_clean(&ctx);

    // Teardown. The encoder's last submission completed (fence-waited above).
    // SAFETY: each resource was created on `device`, its GPU work has completed (the
    // fence was waited), and each is destroyed exactly once here.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_buffer(staging);
        device.destroy_graphics_pipeline(pipeline);
        device.destroy_shader_module(fs);
        device.destroy_shader_module(vs);
        device.destroy_buffer(vertex_buffer);
        device.destroy_texture(color);
    }
    drop(ctx);
}
