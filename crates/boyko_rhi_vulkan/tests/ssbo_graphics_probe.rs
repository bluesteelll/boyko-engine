//! GUI P5a RUNG 0.5 (de-risk) — prove the never-before-exercised combination: a
//! GRAPHICS pipeline binding a STORAGE buffer (set 0, binding 0) visible at
//! VERTEX|FRAGMENT, indexed by `SV_InstanceID`, drawing a vertexless instanced quad
//! whose per-instance transform is read IN THE VERTEX STAGE.
//!
//! Every prior `StorageBuffer` bind in the engine is on the COMPUTE bind point; no
//! graphics pipeline has ever been created with `bind_group_layout: Some(StorageBuffer)`
//! and `enums.rs` documents "COMPUTE is the only stage the foundation uses;
//! VERTEX/FRAGMENT are seam." This test isolates a backend stage-flag / descriptor-type
//! mismatch BEFORE the SDF/blend complexity of the full UI pipeline (plan Rung 0.5,
//! Decision 2). If this faults, the fallback is vertex-buffer instancing (a larger RHI
//! change) and P5a must STOP and re-plan.
//!
//! # The scene + the proof
//!
//! Two instances of a unit quad are placed into a 64×64 `R8G8B8A8` offscreen target via
//! a 2D pixel→NDC ortho (top-left origin). Instance 0 is a RED rect in the top-left
//! quadrant; instance 1 is a GREEN rect in the bottom-right quadrant. Both the quad
//! placement (VS reads `min_px`/`size_px`) and the fill color (FS reads `color`) come
//! from the SAME bound `StorageBuffer<RungInstance>` — so a correct readback proves the
//! SSBO is read in BOTH stages off one VERTEX|FRAGMENT-visible descriptor.
//!
//! Decisive assertions:
//! - the top-left-quadrant centre texel == RED (VS placed inst 0 there; FS colored it),
//! - the bottom-right-quadrant centre texel == GREEN (the second instance),
//! - a texel covered by NEITHER rect == the CLEAR color (real per-instance placement,
//!   not a full-screen fill),
//! - the validation messenger == ZERO messages (the GPU-half soundness oracle).
//!
//! # CI gate (graceful skip)
//!
//! A GPU-less / loader-less host, or one without validation / dynamic rendering, makes
//! `VulkanContext::boot` return `Err`; the test skips gracefully (mirrors graphics_*).

use core::slice;

use boyko_rhi::enums::{BarrierAccess, BarrierStage, DescriptorKind};
use boyko_rhi::{
    BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc, BindGroupLayoutEntry, BufferDesc,
    BufferImageCopy, BufferUsage, Format, CullMode, GraphicsPipelineDesc, ImageAspect, ImageBarrierDesc,
    ImageLayout, ImageSubresourceRange, ImageUsage, LoadOp, MemoryLocation, PrimitiveTopology,
    RenderArea, RenderingAttachment, RenderingDesc, RhiCommandEncoder, RhiDevice, RhiQueue,
    ShaderStage, StoreOp, TextureDesc, TextureDimension, Viewport,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

/// The offscreen image dimensions. Small but multi-texel so a covered/uncovered
/// boundary and the per-quadrant placement are unambiguous.
const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;
const TEXELS: usize = (WIDTH * HEIGHT) as usize;
const SIZE: u64 = (TEXELS * 4) as u64;

/// The offscreen CLEAR color (the texel a covered-by-NEITHER-rect sample keeps).
const CLEAR_BYTES: [u8; 4] = [0x11, 0x22, 0x33, 0xFF];
/// Instance 0's fill (opaque RED) — top-left quadrant.
const RED_BYTES: [u8; 4] = [0xFF, 0x00, 0x00, 0xFF];
/// Instance 1's fill (opaque GREEN) — bottom-right quadrant.
const GREEN_BYTES: [u8; 4] = [0x00, 0xFF, 0x00, 0xFF];

/// The per-instance record the shader's `StructuredBuffer<RungInstance>` reads.
/// `#[repr(C)]` std430-compatible BY BYTE IMAGE: `min_px` @0, `size_px` @8,
/// `color` @16, size 32 (no internal/tail pad). Rust lays `[f32; 4]` at align 4
/// (not the HLSL `float4` 16-align), but the field BYTE OFFSETS equal std430's
/// (the array stride the shader sees is 32, multiple of 16), so the raw byte copy
/// into the SSBO is layout-correct — the std430 contract is per-field-offset, not
/// Rust's whole-struct align.
#[repr(C)]
#[derive(Clone, Copy)]
struct RungInstance {
    min_px: [f32; 2],
    size_px: [f32; 2],
    color: [f32; 4],
}

const RUNG_INSTANCE_SIZE: usize = 32;
const _: () = assert!(size_of::<RungInstance>() == RUNG_INSTANCE_SIZE);
const _: () = assert!(core::mem::offset_of!(RungInstance, min_px) == 0);
const _: () = assert!(core::mem::offset_of!(RungInstance, size_px) == 8);
const _: () = assert!(core::mem::offset_of!(RungInstance, color) == 16);

/// The 2D pixel→NDC ortho push-constant block (`scale`, `translate`), 16 bytes.
/// Top-left origin via the negative-y scale, mirroring the HLSL `Ortho`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Ortho {
    scale: [f32; 2],
    translate: [f32; 2],
}

const _: () = assert!(size_of::<Ortho>() == 16);

/// The CLEAR color as the RGBA floats `begin_rendering` takes (each byte / 255 is
/// exact for an R8G8B8A8_UNORM round-trip).
fn floats(bytes: [u8; 4]) -> [f32; 4] {
    [
        bytes[0] as f32 / 255.0,
        bytes[1] as f32 / 255.0,
        bytes[2] as f32 / 255.0,
        bytes[3] as f32 / 255.0,
    ]
}

/// Pixel→NDC ortho for a `(w, h)` target, TOP-LEFT pixel origin. Vulkan NDC is
/// y-DOWN (NDC y = -1 is the top framebuffer row), so a top-left origin needs a
/// POSITIVE y scale: `(0,0)→(-1,-1)` (top-left), `(w,h)→(+1,+1)` (bottom-right).
/// (The GL/`-2/h`,`+1` formula would land pixel-row 0 at the framebuffer bottom —
/// confirmed by the Rung-0.5 GPU oracle.)
fn ortho_for(w: u32, h: u32) -> Ortho {
    Ortho {
        scale: [2.0 / w as f32, 2.0 / h as f32],
        translate: [-1.0, -1.0],
    }
}

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

/// The committed Rung-0.5 vertex SPIR-V (`ssbo_quad.vs.spv`): vertexless quad +
/// VERTEX-stage SSBO read by `SV_InstanceID` + the ortho push constant.
static QUAD_VS_SPV: SpirvBlob<1808> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/ssbo_quad.vs.spv"
)));

/// The committed Rung-0.5 fragment SPIR-V (`ssbo_quad.fs.spv`): FRAGMENT-stage SSBO
/// read by the interpolated `SV_InstanceID`, outputs the record's color.
static QUAD_FS_SPV: SpirvBlob<904> = SpirvBlob(*include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shaders/ssbo_quad.fs.spv"
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
        "validation layer reported {} message(s) during the SSBO-graphics probe — see the [vk-validation] log",
        state.total()
    );
}

/// The byte index of texel `(x, y)` in the tightly-packed R8G8B8A8 readback.
fn texel_base(x: u32, y: u32) -> usize {
    ((y * WIDTH + x) * 4) as usize
}

/// Renders the two-instance SSBO-graphics scene and returns the readback bytes.
fn render_probe(device: &VulkanContext) -> Vec<u8> {
    let queue = device.rhi_queue();

    let output = device
        .create_texture(&TextureDesc {
            width: WIDTH,
            height: HEIGHT,
            depth: 1,
            format: Format::R8G8B8A8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_SRC,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("output texture O (COLOR_ATTACHMENT | TRANSFER_SRC)");

    // The two per-instance records: a top-left RED rect, a bottom-right GREEN rect.
    let instances = [
        RungInstance {
            min_px: [8.0, 8.0],
            size_px: [16.0, 16.0],
            color: floats(RED_BYTES),
        },
        RungInstance {
            min_px: [40.0, 40.0],
            size_px: [16.0, 16.0],
            color: floats(GREEN_BYTES),
        },
    ];
    let instance_count = instances.len() as u32;
    let instances_bytes = instance_count as u64 * RUNG_INSTANCE_SIZE as u64;

    // A host-visible STORAGE buffer holding the records (the de-risk uses one upload;
    // the full P5a path persistently maps a grow-only ring per frame-in-flight).
    let instance_buffer = device
        .create_buffer(&BufferDesc {
            size: instances_bytes,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible instance STORAGE buffer");
    let map = device
        .buffer_mapped_ptr(&instance_buffer)
        .expect("host-visible STORAGE buffer is mapped");
    // SAFETY: `map` points to `instances_bytes` mapped host-coherent bytes; the
    // `instances` array is `#[repr(C)]` POD (f32 only, const-asserted 32 B/16-align,
    // no padding), so its byte image is a valid initialized `[u8]` of exactly
    // `instances_bytes`; the write is in-bounds and non-overlapping; no submission
    // reads the buffer until the queue submit below.
    unsafe {
        core::ptr::copy_nonoverlapping(
            instances.as_ptr().cast::<u8>(),
            map.as_ptr(),
            instances_bytes as usize,
        );
    }

    // The bind-group layout: ONE STORAGE_BUFFER @ set0/binding0, visible at
    // VERTEX|FRAGMENT — the never-exercised combination this rung proves.
    let bind_group_layout = device
        .create_bind_group_layout(&BindGroupLayoutDesc {
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                count: 1,
                kind: DescriptorKind::StorageBuffer,
                stage: ShaderStage::VERTEX | ShaderStage::FRAGMENT,
            }],
        })
        .expect("bind-group layout (StorageBuffer @ VERTEX|FRAGMENT)");

    let bind_group = device
        .create_bind_group(&BindGroupDesc {
            layout: &bind_group_layout,
            entries: &[BindGroupEntry::StorageBuffer {
                buffer: &instance_buffer,
            }],
        })
        .expect("bind group (instance STORAGE buffer)");

    let vs = device
        .create_shader_module(QUAD_VS_SPV.as_words())
        .expect("vertex shader module");
    let fs = device
        .create_shader_module(QUAD_FS_SPV.as_words())
        .expect("fragment shader module");

    // The graphics pipeline: vertexless (`vertex_layout: None`), a VERTEX-stage 16-byte
    // ortho push range, and the SSBO bind-group layout at set 0.
    let pipeline = device
        .create_graphics_pipeline(&GraphicsPipelineDesc {
            vertex_module: &vs,
            vertex_entry: c"main",
            fragment_module: &fs,
            fragment_entry: c"main",
            color_formats: &[Format::R8G8B8A8Unorm],
            depth_format: None,
            topology: PrimitiveTopology::TriangleList,
            vertex_layout: None,
            push_constant_bytes: size_of::<Ortho>() as u32,
            bind_group_layout: Some(&bind_group_layout),
            blend: None,
            cull_mode: CullMode::None,
            depth_bias: None,
        })
        .expect("SSBO-graphics pipeline");

    let staging = device
        .create_buffer(&BufferDesc {
            size: SIZE,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible readback staging buffer");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");
    let full = RenderArea {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    };
    let viewport = Viewport {
        x: 0.0,
        y: 0.0,
        width: WIDTH as f32,
        height: HEIGHT as f32,
        min_depth: 0.0,
        max_depth: 1.0,
    };
    let ortho = ortho_for(WIDTH, HEIGHT);
    // SAFETY (push bytes): `Ortho` is `#[repr(C)]` POD (f32 only, 16 B), so its byte
    // image is a valid `[u8; 16]`; the slice is not retained past the push record.
    let ortho_bytes: &[u8] = unsafe {
        slice::from_raw_parts((&ortho as *const Ortho).cast::<u8>(), size_of::<Ortho>())
    };

    encoder.begin().expect("begin");

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
        clear_color: floats(CLEAR_BYTES),
    }];
    encoder.begin_rendering(&RenderingDesc {
        render_area: full,
        colors: &output_attachment,
        depth: None,
    });
    encoder.bind_graphics_pipeline(&pipeline);
    encoder.bind_descriptor_set(&bind_group, &pipeline);
    // The ortho is read in the VERTEX stage only (the FS reads only the SSBO color),
    // so a VERTEX-stage push against the pipeline's VERTEX push range is correct here.
    encoder.push_graphics_constants(&pipeline, ShaderStage::VERTEX, 0, ortho_bytes);
    encoder.set_viewport(&viewport);
    encoder.set_scissor(&full);
    encoder.draw(6, instance_count, 0, 0);
    encoder.end_rendering();

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

    let dst_ptr = device
        .buffer_mapped_ptr(&staging)
        .expect("host-visible staging buffer is mapped");
    let mut out = vec![0u8; SIZE as usize];
    // SAFETY: `dst_ptr` points to `SIZE` mapped host-coherent bytes; a fence wait
    // preceded this read, so the GPU draw + copy are complete + coherent; reading
    // `SIZE` bytes is in-bounds; `out` is a distinct, non-overlapping allocation.
    unsafe {
        core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), SIZE as usize);
    }

    // Teardown in reverse dependency order; the encoder's last submission completed
    // (fence-waited above).
    // SAFETY: each resource was created on `device`, its GPU work has completed (the
    // fence was waited), and each is destroyed exactly once here.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_buffer(staging);
        device.destroy_graphics_pipeline(pipeline);
        device.destroy_shader_module(fs);
        device.destroy_shader_module(vs);
        device.destroy_bind_group(bind_group);
        device.destroy_bind_group_layout(bind_group_layout);
        device.destroy_buffer(instance_buffer);
        device.destroy_texture(output);
    }

    out
}

#[test]
fn ssbo_read_in_graphics_pipeline_by_instance_id_golden() {
    let Some(ctx) = boot_or_skip("ssbo_read_in_graphics_pipeline_by_instance_id_golden") else {
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
    let out = render_probe(device);

    // Instance 0: RED rect at min (8,8) size 16 → centre texel (16,16).
    let red = texel_base(16, 16);
    let red_texel = [out[red], out[red + 1], out[red + 2], out[red + 3]];
    assert_eq!(
        red_texel, RED_BYTES,
        "instance 0's centre must be RED (VS placed it via the SSBO min/size, FS colored it via the SSBO color): got {red_texel:02x?}"
    );

    // Instance 1: GREEN rect at min (40,40) size 16 → centre texel (48,48).
    let green = texel_base(48, 48);
    let green_texel = [out[green], out[green + 1], out[green + 2], out[green + 3]];
    assert_eq!(
        green_texel, GREEN_BYTES,
        "instance 1's centre must be GREEN (the second SSBO record drives a distinct quad + color): got {green_texel:02x?}"
    );

    // A texel covered by NEITHER rect (the top-right corner) keeps the CLEAR color —
    // proves genuine per-instance placement, not a full-screen fill.
    let bg = texel_base(60, 4);
    let bg_texel = [out[bg], out[bg + 1], out[bg + 2], out[bg + 3]];
    assert_eq!(
        bg_texel, CLEAR_BYTES,
        "an uncovered texel must keep the CLEAR color (the SSBO min/size genuinely bound the quad): got {bg_texel:02x?}"
    );

    assert_validation_clean(&ctx);

    drop(ctx);
}
