//! Phase-6 Slice S0 — RUNG 2 acceptance: a real TRIANGLE rasterized into an
//! offscreen color attachment via a Vulkan 1.3 dynamic-rendering GRAPHICS PIPELINE,
//! proven by a golden image readback, with the validation layer as the soundness
//! oracle.
//!
//! This is the staged-ladder step 3 (the plan's "Graphics-pipeline + draw-triangle
//! golden"), built on rung 1's offscreen render target + barrier + readback path.
//! The single rung-2 deviation from the plan's literal wording: the triangle's
//! vertices are GENERATED in the vertex shader from `SV_VertexID` (gl_VertexIndex)
//! rather than read from a vertex buffer — the plan explicitly permits this simpler
//! variant for rung 2, so no vertex buffer / `bind_vertex_buffer` is introduced.
//!
//! The flow:
//!
//! 1. Boot a headless, validation-enabled device (Correction #1 routes
//!    `dynamicRendering` into the headless path; the test skips on `Err`).
//! 2. `create_texture` an R8G8B8A8_UNORM 2D color image
//!    (`COLOR_ATTACHMENT | TRANSFER_SRC`).
//! 3. `create_shader_module` for the committed vertex + fragment `.spv`, then
//!    `create_graphics_pipeline` (empty layout, dynamic viewport/scissor, single
//!    color attachment whose declared format MATCHES the rendering scope — the
//!    W2-b contract).
//! 4. Record: `image_barrier` UNDEFINED → COLOR_ATTACHMENT_OPTIMAL →
//!    `begin_rendering` (loadOp = CLEAR to a known color, so uncovered texels keep
//!    the clear) → `bind_graphics_pipeline` + `set_viewport` + `set_scissor` →
//!    `draw(3, 1, 0, 0)` → `end_rendering` → `image_barrier` →
//!    `copy_image_to_buffer`.
//! 5. Submit once, fence-wait, map-read the staging buffer.
//! 6. Assert the CENTRE texel (which the triangle covers) equals the FRAGMENT
//!    colour, AND a CORNER texel (which the triangle does NOT cover) equals the
//!    CLEAR colour — proving the triangle actually RASTERIZED, not just a clear.
//! 7. Assert the validation messenger recorded ZERO messages.
//!
//! # CI gate (graceful skip)
//!
//! A GPU-less / loader-less host, or one without the validation layer / dynamic
//! rendering, makes `VulkanContext::boot` return `Err`; the test skips gracefully
//! (mirrors rung 1).

use core::slice;

use boyko_rhi::{
    BufferDesc, BufferImageCopy, BufferUsage, Format, GraphicsPipelineDesc, ImageAspect,
    ImageBarrierDesc, ImageLayout, ImageSubresourceRange, ImageUsage, LoadOp, MemoryLocation,
    PrimitiveTopology, RenderArea, RenderingAttachment, RenderingDesc, RhiCommandEncoder,
    RhiDevice, RhiQueue, StoreOp, TextureDesc, Viewport,
};
use boyko_rhi::enums::{BarrierAccess, BarrierStage};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// The offscreen image dimensions. Small but multi-texel so a covered/uncovered
/// boundary is unambiguous.
const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;
const TEXELS: usize = (WIDTH * HEIGHT) as usize;
const SIZE: u64 = (TEXELS * 4) as u64;

/// The CLEAR colour bytes (R, G, B, A) for R8G8B8A8_UNORM — the value an UNCOVERED
/// texel keeps (the rung-1 clear). Each byte is `byte / 255.0` so the float→byte
/// UNORM conversion is exact.
const CLEAR_BYTES: [u8; 4] = [0xAA, 0xBB, 0xCC, 0xDD];

/// The FRAGMENT-shader output bytes — opaque red `(1, 0, 0, 1)` → `0xFF 0x00 0x00
/// 0xFF`, the value a COVERED texel takes. Distinct from `CLEAR_BYTES` so a covered
/// vs uncovered texel cannot be confused.
const FRAGMENT_BYTES: [u8; 4] = [0xFF, 0x00, 0x00, 0xFF];

/// The clear colour as the RGBA floats `begin_rendering` takes.
fn clear_floats() -> [f32; 4] {
    [
        CLEAR_BYTES[0] as f32 / 255.0,
        CLEAR_BYTES[1] as f32 / 255.0,
        CLEAR_BYTES[2] as f32 / 255.0,
        CLEAR_BYTES[3] as f32 / 255.0,
    ]
}

// --- Committed triangle SPIR-V, embedded at compile time (the proven shader
//     pattern: a `#[repr(C, align(4))]` blob re-viewed as a `&[u32]` word stream).
//     Authored as HLSL → DXC → committed `.spv`; see the `shaders/triangle.*.hlsl`
//     header comments for the exact DXC invocation. ---

/// A 4-byte-aligned wrapper around a committed SPIR-V byte blob so its address is a
/// valid `*const u32` and it can be re-viewed as a `&[u32]` word stream (a bare
/// `include_bytes!` is only `align(1)`; SPIR-V requires 4-byte word alignment).
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

/// The committed vertex SPIR-V (`triangle.vs.spv`, 700 bytes).
static TRIANGLE_VS_SPV: SpirvBlob<700> =
    SpirvBlob(*include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/triangle.vs.spv")));

/// The committed fragment SPIR-V (`triangle.fs.spv`, 336 bytes).
static TRIANGLE_FS_SPV: SpirvBlob<336> =
    SpirvBlob(*include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/triangle.fs.spv")));

/// Boots a validation-enabled headless context, or returns `None` (with a SKIP
/// log) when no GPU / loader / validation layer / dynamic-rendering is available.
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
        "validation layer reported {} message(s) during the triangle draw — see the [vk-validation] log",
        state.total()
    );
}

/// The byte index of texel `(x, y)` in the tightly-packed R8G8B8A8 readback.
fn texel_base(x: u32, y: u32) -> usize {
    ((y * WIDTH + x) * 4) as usize
}

#[test]
fn triangle_draw_golden_round_trip() {
    let Some(ctx) = boot_or_skip("triangle_draw_golden_round_trip") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    assert!(ctx.validation_enabled(), "validation must be active");

    let device: &VulkanContext = &ctx;
    let queue = ctx.rhi_queue();

    // The offscreen color image: drawn into as a color attachment, read back as a
    // transfer source. Its format MUST equal the pipeline's declared `color_format`.
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

    // The committed vertex + fragment shader modules.
    let vs = device
        .create_shader_module(TRIANGLE_VS_SPV.as_words())
        .expect("vertex shader module");
    let fs = device
        .create_shader_module(TRIANGLE_FS_SPV.as_words())
        .expect("fragment shader module");

    // The rung-2 graphics pipeline. `color_format` equals the attachment's format
    // (the W2-b draw-time contract); dynamic viewport/scissor; no vertex buffer.
    let pipeline = device
        .create_graphics_pipeline(&GraphicsPipelineDesc {
            vertex_module: &vs,
            vertex_entry: c"main",
            fragment_module: &fs,
            fragment_entry: c"main",
            color_format: Format::R8G8B8A8Unorm,
            topology: PrimitiveTopology::TriangleList,
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

    // Record the whole rung-2 thread.
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

    // Begin rendering with a CLEAR load (so the uncovered texels keep the clear
    // colour), then bind the pipeline + dynamic state and draw one triangle.
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
    });
    encoder.bind_graphics_pipeline(&pipeline);
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

    // The CENTRE texel: the triangle (NDC (0,-0.7),(0.7,0.7),(-0.7,0.7)) covers the
    // image centre, so this texel must equal the FRAGMENT colour — proving real
    // rasterization, not a bare clear.
    let centre = texel_base(WIDTH / 2, HEIGHT / 2);
    let centre_texel = [
        out[centre],
        out[centre + 1],
        out[centre + 2],
        out[centre + 3],
    ];
    assert_eq!(
        centre_texel, FRAGMENT_BYTES,
        "centre texel must be the fragment colour (triangle covers it): got {centre_texel:02x?}, want {FRAGMENT_BYTES:02x?}"
    );

    // The (0, 0) CORNER texel: the triangle does NOT reach the corners, so this
    // texel must keep the CLEAR colour — proving the triangle did not fill the
    // whole surface (a clear-only bug would make this also equal the clear, so the
    // covered-vs-clear PAIR is what proves the draw rasterized a real shape).
    let corner = texel_base(0, 0);
    let corner_texel = [
        out[corner],
        out[corner + 1],
        out[corner + 2],
        out[corner + 3],
    ];
    assert_eq!(
        corner_texel, CLEAR_BYTES,
        "corner texel must be the clear colour (triangle does NOT cover it): got {corner_texel:02x?}, want {CLEAR_BYTES:02x?}"
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
        device.destroy_texture(color);
    }
    drop(ctx);
}
