//! Phase-6 Slice S0 — RUNG 4 acceptance: a DEPTH attachment + DEPTH TEST proving
//! occlusion — two overlapping triangles at different Z sort correctly by depth,
//! NOT by draw order. Rendered offscreen + headless via the Vulkan 1.3
//! dynamic-rendering graphics pipeline, proven by a golden image readback, with the
//! validation layer as the soundness oracle.
//!
//! This is the next bounded step beyond rung 3's single MVP triangle: a depth
//! `VkImage` (`D32_SFLOAT`, DEPTH-aspect view) is created + transitioned UNDEFINED →
//! DEPTH_ATTACHMENT_OPTIMAL, bound as `begin_rendering`'s depth attachment (loadOp =
//! CLEAR to depth = 1.0, the far plane), and a depth-testing pipeline
//! (`depthTestEnable`/`depthWriteEnable`, compareOp = LESS) decides per-fragment
//! which triangle wins. There are still NO descriptor sets / samplers / textures /
//! G-buffer / present / SDF.
//!
//! # The two triangles + the depth proof
//!
//! Both triangles use the SAME diagonal MVP `diag(0.7, 0.7, 1, 1)` (symmetric, so
//! the 16 packed floats are storage-convention-insensitive — see the rung-3 MVP
//! note) and the SAME x/y model footprint `(0, -1), (1, 1), (-1, 1)`, so both cover
//! the image centre and miss the corners. They differ ONLY in their model-space Z:
//! `mul(mvp, float4(pos, 1))` makes clip-space `z == pos.z` (the mvp z-row is
//! `[0, 0, 1, 0]`), and with `w == 1` + viewport depth `[0, 1]` the written depth IS
//! that Z. So:
//!
//! - NEAR triangle: `z = 0.25` → depth 0.25, colored GREEN.
//! - FAR triangle:  `z = 0.75` → depth 0.75, colored RED.
//!
//! Two variants prove it is depth, not paint order:
//!
//! - **Variant A (far first, then near):** draw FAR (red, 0.75) then NEAR (green,
//!   0.25). The centre must be GREEN — the nearer fragment wins because 0.25 < 0.75.
//!   (A no-depth pipeline would already give green here purely by paint order, so
//!   this variant alone is not conclusive.)
//! - **Variant B (near first, then far):** draw NEAR (green, 0.25) then FAR (red,
//!   0.75). The centre must STILL be GREEN — the FAR fragment is REJECTED by the
//!   depth test (0.75 < 0.25 is false), so it does NOT overwrite the nearer green.
//!   WITHOUT depth this variant would paint RED last and the centre would be red —
//!   so a green centre here is the decisive depth proof.
//!
//! Both variants also assert an uncovered CORNER == the clear color and the
//! validation messenger == zero messages.
//!
//! # CI gate (graceful skip)
//!
//! A GPU-less / loader-less host, or one without the validation layer / dynamic
//! rendering, makes `VulkanContext::boot` return `Err`; the test skips gracefully
//! (mirrors rungs 1-3).

use core::slice;

use boyko_rhi::enums::{BarrierAccess, BarrierStage};
use boyko_rhi::{
    BufferDesc, BufferImageCopy, BufferUsage, DepthAttachment, Format, CullMode, GraphicsPipelineDesc,
    ImageAspect, ImageBarrierDesc, ImageLayout, ImageSubresourceRange, ImageUsage, LoadOp,
    MemoryLocation, PrimitiveTopology, RenderArea, RenderingAttachment, RenderingDesc,
    RhiCommandEncoder, RhiDevice, RhiQueue, ShaderStage, StoreOp, TextureDesc, TextureDimension,
    VertexAttribute, VertexBufferLayout, VertexFormat, Viewport,
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

/// The NEAR triangle's per-vertex color — opaque green `(0, 1, 0, 1)`. The depth
/// test must let it WIN at the overlap centre regardless of draw order.
const NEAR_COLOR_BYTES: [u8; 4] = [0x00, 0xFF, 0x00, 0xFF];

/// The FAR triangle's per-vertex color — opaque red `(1, 0, 0, 1)`. The depth test
/// must REJECT it at the overlap centre (it is behind the near triangle).
const FAR_COLOR_BYTES: [u8; 4] = [0xFF, 0x00, 0x00, 0xFF];

/// The NEAR triangle's model-space (and clip-space) Z → the written depth 0.25.
const NEAR_Z: f32 = 0.25;
/// The FAR triangle's model-space (and clip-space) Z → the written depth 0.75.
const FAR_Z: f32 = 0.75;
/// The depth attachment's CLEAR value (the far plane; a LESS test always passes for
/// the first fragment since both 0.25 and 0.75 are < 1.0).
const DEPTH_CLEAR: f32 = 1.0;

/// One vertex: a `Float32x3` position (offset 0) + a `Float32x4` color (offset 12),
/// matching the rung-3 vertex layout (reused unchanged). `#[repr(C)]` so the field
/// layout is the exact 28-byte stride the layout declares.
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

/// A byte color → the RGBA floats stored in each vertex.
fn color_floats(bytes: [u8; 4]) -> [f32; 4] {
    [
        bytes[0] as f32 / 255.0,
        bytes[1] as f32 / 255.0,
        bytes[2] as f32 / 255.0,
        bytes[3] as f32 / 255.0,
    ]
}

/// The deterministic MVP: `diag(0.7, 0.7, 1, 1)` packed (identical row-/column-major
/// because diagonal — see the module MVP note). Scales the model `[-1, 1]` x/y to
/// the `[-0.7, 0.7]` covering footprint and passes Z through unchanged (z-row
/// `[0, 0, 1, 0]`), so a vertex's model Z becomes its written depth.
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

/// The three vertices of a centre-covering triangle at the given Z + color.
fn triangle(z: f32, color: [u8; 4]) -> [Vertex; 3] {
    let c = color_floats(color);
    [
        Vertex { position: [0.0, -1.0, z], color: c },
        Vertex { position: [1.0, 1.0, z], color: c },
        Vertex { position: [-1.0, 1.0, z], color: c },
    ]
}

// --- Committed rung-3 SPIR-V, reused unchanged: the depth comes from the
//     transformed `gl_Position.z`, so the MVP vertex+fragment shaders suffice with
//     NO new shader. Embedded at compile time (a `#[repr(C, align(4))]` blob
//     re-viewed as a `&[u32]` word stream). ---

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
/// box): there is no messenger to read, but the PIXEL goldens still run.
fn assert_validation_clean(ctx: &VulkanContext, variant: &str) {
    if !ctx.validation_enabled() {
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — skipping the {variant} clean-oracle assert");
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
        "validation layer reported {} message(s) during the depth-test {variant} draw — see the [vk-validation] log",
        state.total()
    );
}

/// The byte index of texel `(x, y)` in the tightly-packed R8G8B8A8 readback.
fn texel_base(x: u32, y: u32) -> usize {
    ((y * WIDTH + x) * 4) as usize
}

/// Renders the two triangles in the order `[first, second]` and returns the readback
/// bytes. Both triangles are passed as one combined six-vertex buffer; the draw order
/// is the order they appear in `vertices` (front three = first drawn, back three =
/// second drawn), so depth — not array order — must decide the overlap.
fn render_two_triangles(
    device: &VulkanContext,
    vertices: &[Vertex; 6],
) -> Vec<u8> {
    let queue = device.rhi_queue();

    // The offscreen color image. Its format MUST equal the pipeline's `color_format`.
    let color = device
        .create_texture(&TextureDesc {
            width: WIDTH,
            height: HEIGHT,
            depth: 1,
            format: Format::R8G8B8A8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_SRC,
            array_layers: 1,
        })
        .expect("offscreen color texture");

    // The depth image (D32_SFLOAT, DEPTH_STENCIL_ATTACHMENT). Its format MUST equal
    // the pipeline's declared `depth_format` (the W2-b contract). The DEPTH-aspect
    // view is derived from the usage inside `create_texture`.
    let depth = device
        .create_texture(&TextureDesc {
            width: WIDTH,
            height: HEIGHT,
            depth: 1,
            format: Format::D32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT,
            array_layers: 1,
        })
        .expect("offscreen depth texture");

    // The combined six-vertex buffer (two triangles), host-visible.
    let vertex_bytes = core::mem::size_of_val(vertices) as u64;
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

    let vs = device
        .create_shader_module(MVP_VS_SPV.as_words())
        .expect("vertex shader module");
    let fs = device
        .create_shader_module(MVP_FS_SPV.as_words())
        .expect("fragment shader module");

    // The rung-4 depth-testing pipeline: rung-3 vertex layout + a 64-byte VERTEX push
    // range (the MVP), PLUS a declared `depth_format` that enables depth test/write +
    // compareOp LESS + `depthAttachmentFormat`. Both attachment formats equal the
    // images' formats (W2-b).
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
            color_formats: &[Format::R8G8B8A8Unorm],
            depth_format: Some(Format::D32Sfloat),
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: Some(VertexBufferLayout {
                stride: VERTEX_STRIDE,
                attributes: &attributes,
            }),
            push_constant_bytes: MVP_BYTES,
            // Rung 4 binds no descriptor sets (the rung-5 bind-group-layout seam).
            bind_group_layout: None,
            blend: None,
            cull_mode: CullMode::None,
            depth_bias: None,
        })
        .expect("depth-testing graphics pipeline");

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

    // Color: UNDEFINED → COLOR_ATTACHMENT_OPTIMAL.
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
    // Depth: UNDEFINED → DEPTH_ATTACHMENT_OPTIMAL (the early/late fragment-test
    // stage + depth-write access, DEPTH aspect range).
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
    encoder.set_viewport(&Viewport {
        x: 0.0,
        y: 0.0,
        width: WIDTH as f32,
        height: HEIGHT as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    });
    encoder.set_scissor(&full);
    // First triangle (vertices 0..3), then second triangle (vertices 3..6). The
    // depth test — not this order — must decide the overlap.
    encoder.draw(3, 1, 0, 0);
    encoder.draw(3, 1, 3, 0);
    encoder.end_rendering();

    // Color: COLOR_ATTACHMENT_OPTIMAL → TRANSFER_SRC_OPTIMAL for the readback.
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
    let mut out = vec![0u8; SIZE as usize];
    // SAFETY: `dst_ptr` points to `SIZE` mapped host-coherent bytes; a fence wait
    // preceded this read, so the GPU draws + copy are complete + coherent; reading
    // `SIZE` bytes is in-bounds; `out` is a distinct, non-overlapping allocation.
    unsafe {
        core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), SIZE as usize);
    }

    // Teardown. The encoder's last submission completed (fence-waited above).
    // SAFETY: each resource was created on `device`, its GPU work has completed (the
    // fence was waited), and each is destroyed exactly once here. The depth texture
    // is torn down like the color one (`destroy_texture` frees view → image → memory).
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_buffer(staging);
        device.destroy_graphics_pipeline(pipeline);
        device.destroy_shader_module(fs);
        device.destroy_shader_module(vs);
        device.destroy_buffer(vertex_buffer);
        device.destroy_texture(depth);
        device.destroy_texture(color);
    }

    out
}

/// Asserts the centre texel equals `want` and the (0,0) corner equals the clear.
fn assert_centre_and_corner(out: &[u8], want: [u8; 4], variant: &str) {
    let centre = texel_base(WIDTH / 2, HEIGHT / 2);
    let centre_texel = [out[centre], out[centre + 1], out[centre + 2], out[centre + 3]];
    assert_eq!(
        centre_texel, want,
        "[{variant}] overlap centre must be {want:02x?} (the nearer triangle wins the depth test): got {centre_texel:02x?}"
    );

    let corner = texel_base(0, 0);
    let corner_texel = [out[corner], out[corner + 1], out[corner + 2], out[corner + 3]];
    assert_eq!(
        corner_texel, CLEAR_BYTES,
        "[{variant}] corner texel must be the clear color (neither triangle covers it): got {corner_texel:02x?}, want {CLEAR_BYTES:02x?}"
    );
}

#[test]
fn depth_test_far_then_near_overlap_golden() {
    let Some(ctx) = boot_or_skip("depth_test_far_then_near_overlap_golden") else {
        return;
    };
    println!("Vulkan device: {}", ctx.device_name());
    // Pixel golden: runs with or without validation (the clean-oracle assert
    // self-gates when validation is disabled via BOYKO_DISABLE_VALIDATION).
    if !ctx.validation_enabled() {
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — depth golden still runs");
    }

    let device: &VulkanContext = &ctx;

    // Variant A: draw FAR (red, 0.75) FIRST, then NEAR (green, 0.25). The nearer
    // green must win the overlap (0.25 < 0.75). Combined buffer: far then near.
    let far = triangle(FAR_Z, FAR_COLOR_BYTES);
    let near = triangle(NEAR_Z, NEAR_COLOR_BYTES);
    let vertices = [far[0], far[1], far[2], near[0], near[1], near[2]];

    let out = render_two_triangles(device, &vertices);
    assert_centre_and_corner(&out, NEAR_COLOR_BYTES, "far-then-near");
    assert_validation_clean(&ctx, "far-then-near");

    drop(ctx);
}

#[test]
fn depth_test_near_then_far_overlap_golden() {
    let Some(ctx) = boot_or_skip("depth_test_near_then_far_overlap_golden") else {
        return;
    };
    println!("Vulkan device: {}", ctx.device_name());
    // Pixel golden: runs with or without validation (the clean-oracle assert
    // self-gates when validation is disabled via BOYKO_DISABLE_VALIDATION).
    if !ctx.validation_enabled() {
        eprintln!("NOTE: validation disabled (BOYKO_DISABLE_VALIDATION) — depth golden still runs");
    }

    let device: &VulkanContext = &ctx;

    // Variant B (the decisive one): draw NEAR (green, 0.25) FIRST, then FAR (red,
    // 0.75). WITHOUT depth, red would paint last and the centre would be RED. WITH
    // depth, the far red fragment is REJECTED (0.75 < 0.25 is false), so the centre
    // stays GREEN — proving it is the depth test, not paint order, that decides.
    let near = triangle(NEAR_Z, NEAR_COLOR_BYTES);
    let far = triangle(FAR_Z, FAR_COLOR_BYTES);
    let vertices = [near[0], near[1], near[2], far[0], far[1], far[2]];

    let out = render_two_triangles(device, &vertices);
    assert_centre_and_corner(&out, NEAR_COLOR_BYTES, "near-then-far");
    assert_validation_clean(&ctx, "near-then-far");

    drop(ctx);
}
