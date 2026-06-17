//! Phase-6 Slice S0 — RUNG 5 acceptance: SAMPLE a texture in a shader via a
//! descriptor set + sampler, proving the full descriptor/sampler surface with a
//! TWO-PASS headless golden. Rendered offscreen + headless via the Vulkan 1.3
//! dynamic-rendering graphics pipeline, proven by a golden image readback, with the
//! validation layer as the soundness oracle.
//!
//! This is the next bounded step beyond rung 4's depth test: it stands up the
//! sampler + bind-group-layout + bind-group + `bind_descriptor_set` surface and the
//! COLOR_ATTACHMENT → SHADER_READ_ONLY_OPTIMAL image-layout transition. There is
//! still NO multi-attachment G-buffer / lighting MATH / on-screen present / SDF.
//!
//! # The two passes + the sampled-read proof
//!
//! - **Pass 1 (render a KNOWN image into the source texture `T`):** clear `T` to a
//!   known CLEAR color, then draw the rung-2 `SV_VertexID` triangle (a known TRIANGLE
//!   color) covering `T`'s centre but NOT its corners. So `T` ends up non-uniform: a
//!   TRIANGLE-colored centre over a CLEAR-colored corner.
//! - **Barrier:** transition `T` COLOR_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL
//!   (the rung-5 net-new transition: FRAGMENT_SHADER stage + SHADER_READ access).
//! - **Pass 2 (SAMPLE `T` into the output texture `O`):** a full-screen triangle
//!   whose fragment shader samples `T` (a COMBINED_IMAGE_SAMPLER bound at set 0 via a
//!   bind group) at the interpolated UV and writes the sampled color into `O`. Because
//!   the full-screen sample is a 1:1 mapping, `O` becomes a copy of `T`.
//! - **Readback:** copy `O` → a host-visible buffer and assert.
//!
//! The decisive assertion: `O`'s CENTRE texel == the TRIANGLE color (the sampled
//! read round-tripped `T`'s painted centre THROUGH the descriptor set), and `O`'s
//! (0,0) CORNER == the CLEAR color (the sampled read also round-tripped `T`'s
//! unpainted corner — proving the sample reads real `T` content, not a uniform
//! fill). The validation messenger == zero messages is the soundness oracle.
//!
//! A uniform-fill source would make the centre/corner indistinguishable, so the
//! non-uniform `T` is what makes "the sampled read genuinely round-trips" provable.
//!
//! # CI gate (graceful skip)
//!
//! A GPU-less / loader-less host, or one without the validation layer / dynamic
//! rendering, makes `VulkanContext::boot` return `Err`; the test skips gracefully
//! (mirrors rungs 1-4).

use core::slice;

use boyko_rhi::enums::{AddressMode, BarrierAccess, BarrierStage, Filter};
use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BufferDesc, BufferImageCopy, BufferUsage,
    Format, GraphicsPipelineDesc, ImageAspect, ImageBarrierDesc, ImageLayout, ImageSubresourceRange,
    ImageUsage, LoadOp, MemoryLocation, PrimitiveTopology, RenderArea, RenderingAttachment,
    RenderingDesc, RhiCommandEncoder, RhiDevice, RhiQueue, SamplerDesc, ShaderStage, StoreOp,
    TextureDesc, TextureDimension, Viewport,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// The offscreen image dimensions. Small but multi-texel so a covered/uncovered
/// boundary is unambiguous.
const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;
const TEXELS: usize = (WIDTH * HEIGHT) as usize;
const SIZE: u64 = (TEXELS * 4) as u64;

/// Pass-1 CLEAR color (R, G, B, A) for R8G8B8A8_UNORM — the source texture `T`'s
/// UNPAINTED corner. Each byte is `byte / 255.0` so the float→byte UNORM conversion
/// is exact, which makes the after-sampling golden assertion exact.
const CLEAR_BYTES: [u8; 4] = [0x11, 0x22, 0x33, 0xFF];

/// The rung-2 triangle's fragment color — opaque RED `(1, 0, 0, 1)` → bytes
/// `0xFF 0x00 0x00 0xFF` (the rung-2 `triangle.fs` is hardcoded to this). It paints
/// `T`'s centre; the sampled `O`'s centre must round-trip to exactly this.
const TRIANGLE_BYTES: [u8; 4] = [0xFF, 0x00, 0x00, 0xFF];

/// Pass-2 CLEAR color for the output texture `O`. Any covered texel is overwritten
/// by the sampled color, so this is only the value an UNCOVERED `O` texel would keep
/// — but the full-screen triangle covers all of `O`, so it never survives. A
/// distinct value (vs both other colors) makes a coverage bug loud.
const OUTPUT_CLEAR_BYTES: [u8; 4] = [0x44, 0x55, 0x66, 0xFF];

/// The CLEAR color as the RGBA floats `begin_rendering` takes.
fn floats(bytes: [u8; 4]) -> [f32; 4] {
    [
        bytes[0] as f32 / 255.0,
        bytes[1] as f32 / 255.0,
        bytes[2] as f32 / 255.0,
        bytes[3] as f32 / 255.0,
    ]
}

// --- Committed SPIR-V blobs (a `#[repr(C, align(4))]` byte wrapper re-viewed as a
//     `&[u32]` word stream). Pass 1 reuses the rung-2 `SV_VertexID` triangle (a known
//     solid color, no vertex buffer); pass 2 uses the new full-screen sample shaders. ---

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

/// The committed rung-2 vertex SPIR-V (`triangle.vs.spv`, 700 bytes), reused for the
/// pass-1 source draw.
static TRI_VS_SPV: SpirvBlob<700> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/triangle.vs.spv"
)));

/// The committed rung-2 fragment SPIR-V (`triangle.fs.spv`, 336 bytes), reused: it
/// outputs the known opaque-red [`TRIANGLE_BYTES`].
static TRI_FS_SPV: SpirvBlob<336> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/triangle.fs.spv"
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
        "validation layer reported {} message(s) during the two-pass sampled draw — see the [vk-validation] log",
        state.total()
    );
}

/// The byte index of texel `(x, y)` in the tightly-packed R8G8B8A8 readback.
fn texel_base(x: u32, y: u32) -> usize {
    ((y * WIDTH + x) * 4) as usize
}

/// The full-surface render area + viewport scissor reused by both passes.
fn full_area() -> RenderArea {
    RenderArea {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    }
}

/// Renders the two-pass scene (pass 1 paints the source `T`; pass 2 samples `T` into
/// `O` through a bind group) and returns `O`'s readback bytes.
fn render_sampled(device: &VulkanContext) -> Vec<u8> {
    let queue = device.rhi_queue();

    // The source texture `T`: COLOR_ATTACHMENT (pass-1 render target) + SAMPLED
    // (pass-2 read). Its format MUST equal both pipelines' `color_format` (W2-b).
    let source = device
        .create_texture(&TextureDesc {
            width: WIDTH,
            height: HEIGHT,
            depth: 1,
            format: Format::R8G8B8A8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::SAMPLED,
        })
        .expect("source texture T (COLOR_ATTACHMENT | SAMPLED)");

    // The output texture `O`: COLOR_ATTACHMENT (pass-2 render target) + TRANSFER_SRC
    // (golden readback).
    let output = device
        .create_texture(&TextureDesc {
            width: WIDTH,
            height: HEIGHT,
            depth: 1,
            format: Format::R8G8B8A8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_SRC,
        })
        .expect("output texture O (COLOR_ATTACHMENT | TRANSFER_SRC)");

    // Pass-1 pipeline: the rung-2 vertex-buffer-less triangle, no depth, no
    // descriptors. `color_format` == T's format (W2-b).
    let tri_vs = device
        .create_shader_module(TRI_VS_SPV.as_words())
        .expect("pass-1 vertex shader module");
    let tri_fs = device
        .create_shader_module(TRI_FS_SPV.as_words())
        .expect("pass-1 fragment shader module");
    let source_pipeline = device
        .create_graphics_pipeline(&GraphicsPipelineDesc {
            vertex_module: &tri_vs,
            vertex_entry: c"main",
            fragment_module: &tri_fs,
            fragment_entry: c"main",
            color_formats: &[Format::R8G8B8A8Unorm],
            depth_format: None,
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: None,
            push_constant_bytes: 0,
            bind_group_layout: None,
        })
        .expect("pass-1 source pipeline");

    // The sampler + bind-group layout (one COMBINED_IMAGE_SAMPLER @ set0/binding0,
    // FRAGMENT stage). The sampler is the deterministic 1:1 default
    // (NEAREST + CLAMP_TO_EDGE).
    let sampler = device
        .create_sampler(&SamplerDesc {
            mag_filter: Filter::Nearest,
            min_filter: Filter::Nearest,
            address_mode: AddressMode::ClampToEdge,
        })
        .expect("sampler");
    let bind_group_layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc {
            stage: ShaderStage::FRAGMENT,
            binding_count: 1,
        })
        .expect("bind-group layout");

    // Pass-2 pipeline: the full-screen sampler, no vertex buffer, no depth, ONE
    // bind-group layout at set 0 (the combined-image-sampler). `color_format` == O's
    // format (W2-b).
    let sample_vs = device
        .create_shader_module(SAMPLE_VS_SPV.as_words())
        .expect("pass-2 vertex shader module");
    let sample_fs = device
        .create_shader_module(SAMPLE_FS_SPV.as_words())
        .expect("pass-2 fragment shader module");
    let sample_pipeline = device
        .create_graphics_pipeline(&GraphicsPipelineDesc {
            vertex_module: &sample_vs,
            vertex_entry: c"main",
            fragment_module: &sample_fs,
            fragment_entry: c"main",
            color_formats: &[Format::R8G8B8A8Unorm],
            depth_format: None,
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: None,
            push_constant_bytes: 0,
            bind_group_layout: Some(&bind_group_layout),
        })
        .expect("pass-2 sample pipeline");

    // The bind group binding (T's view, sampler) into the layout's binding 0. T is
    // sampled in SHADER_READ_ONLY_OPTIMAL (transitioned below before pass 2).
    let bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &bind_group_layout,
            entries: &[BindGroupEntry {
                texture: &source,
                sampler: &sampler,
            }],
        })
        .expect("bind group (T, sampler)");

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
    let viewport = Viewport {
        x: 0.0,
        y: 0.0,
        width: WIDTH as f32,
        height: HEIGHT as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    };

    encoder.begin().expect("begin");

    // --- Pass 1: paint T (UNDEFINED → COLOR_ATTACHMENT_OPTIMAL, clear + triangle). ---
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &source,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::ColorAttachmentOptimal,
        range: ImageSubresourceRange::COLOR,
    });
    let source_attachment = [RenderingAttachment {
        texture: &source,
        layout: ImageLayout::ColorAttachmentOptimal,
        load_op: LoadOp::Clear,
        store_op: StoreOp::Store,
        clear_color: floats(CLEAR_BYTES),
    }];
    encoder.begin_rendering(&RenderingDesc {
        render_area: full,
        colors: &source_attachment,
        depth: None,
    });
    encoder.bind_graphics_pipeline(&source_pipeline);
    encoder.set_viewport(&viewport);
    encoder.set_scissor(&full);
    encoder.draw(3, 1, 0, 0);
    encoder.end_rendering();

    // --- Barrier: T COLOR_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL (the rung-5
    //     net-new transition: the pass-2 fragment shader will SAMPLE T). The src scope
    //     is the pass-1 color write; the dst scope is the pass-2 fragment-shader read. ---
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &source,
        src_stage: BarrierStage::COLOR_ATTACHMENT_OUTPUT,
        dst_stage: BarrierStage::FRAGMENT_SHADER,
        src_access: BarrierAccess::COLOR_ATTACHMENT_WRITE,
        dst_access: BarrierAccess::SHADER_READ,
        old_layout: ImageLayout::ColorAttachmentOptimal,
        new_layout: ImageLayout::ShaderReadOnlyOptimal,
        range: ImageSubresourceRange::COLOR,
    });

    // --- Pass 2: sample T into O (UNDEFINED → COLOR_ATTACHMENT_OPTIMAL, full-screen
    //     sampling draw). ---
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
        clear_color: floats(OUTPUT_CLEAR_BYTES),
    }];
    encoder.begin_rendering(&RenderingDesc {
        render_area: full,
        colors: &output_attachment,
        depth: None,
    });
    encoder.bind_graphics_pipeline(&sample_pipeline);
    encoder.bind_descriptor_set(&bind_group, &sample_pipeline);
    encoder.set_viewport(&viewport);
    encoder.set_scissor(&full);
    encoder.draw(3, 1, 0, 0);
    encoder.end_rendering();

    // --- O: COLOR_ATTACHMENT_OPTIMAL → TRANSFER_SRC_OPTIMAL for the readback. ---
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
    // dependency order (encoder/fence/buffer, then the bind group + its layout +
    // sampler, then the pipelines + modules, then the textures).
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_buffer(staging);
        device.destroy_bind_group(bind_group);
        device.destroy_graphics_pipeline(sample_pipeline);
        device.destroy_shader_module(sample_fs);
        device.destroy_shader_module(sample_vs);
        device.destroy_bind_group_layout(bind_group_layout);
        device.destroy_sampler(sampler);
        device.destroy_graphics_pipeline(source_pipeline);
        device.destroy_shader_module(tri_fs);
        device.destroy_shader_module(tri_vs);
        device.destroy_texture(output);
        device.destroy_texture(source);
    }

    out
}

#[test]
fn sampled_texture_round_trips_through_descriptor_set_golden() {
    let Some(ctx) = boot_or_skip("sampled_texture_round_trips_through_descriptor_set_golden") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    assert!(ctx.validation_enabled(), "validation must be active");

    let device: &VulkanContext = &ctx;
    let out = render_sampled(device);

    // The decisive assertion: O's CENTRE texel == the TRIANGLE color, proving the
    // sampled read round-tripped T's painted centre THROUGH the descriptor set.
    let centre = texel_base(WIDTH / 2, HEIGHT / 2);
    let centre_texel = [out[centre], out[centre + 1], out[centre + 2], out[centre + 3]];
    assert_eq!(
        centre_texel, TRIANGLE_BYTES,
        "O's centre must be the TRIANGLE color sampled from T (the descriptor-set read round-trips): got {centre_texel:02x?}, want {TRIANGLE_BYTES:02x?}"
    );

    // O's (0,0) CORNER == T's CLEAR color: the sampled read also round-trips T's
    // UNPAINTED corner (the triangle does not cover it), proving the sample reads
    // real, non-uniform T content — not a constant fill or O's own clear color.
    let corner = texel_base(0, 0);
    let corner_texel = [out[corner], out[corner + 1], out[corner + 2], out[corner + 3]];
    assert_eq!(
        corner_texel, CLEAR_BYTES,
        "O's corner must be T's CLEAR color sampled from T (proves a genuine sampled read of non-uniform T, not O's clear {OUTPUT_CLEAR_BYTES:02x?}): got {corner_texel:02x?}, want {CLEAR_BYTES:02x?}"
    );

    assert_validation_clean(&ctx);

    drop(ctx);
}
