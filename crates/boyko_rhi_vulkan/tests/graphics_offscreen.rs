//! Phase-6 Slice S0 — RUNG 1 acceptance: the smallest real graphics-surface
//! thread. A headless offscreen CLEAR via Vulkan 1.3 dynamic rendering, proven by
//! a golden image readback, with the validation layer as the soundness oracle.
//!
//! The flow (no graphics pipeline, no draw — those are rung 2+):
//!
//! 1. Boot a headless, validation-enabled device. Correction #1 routes the
//!    `dynamicRendering` feature into this headless path, so `begin_rendering`
//!    does not fault; Correction #2 fails the boot fast with a clear error if the
//!    GPU lacks the feature (the test skips on `Err`).
//! 2. `create_texture` an R8G8B8A8_UNORM 2D color image
//!    (`COLOR_ATTACHMENT | TRANSFER_SRC`).
//! 3. Record: `image_barrier` UNDEFINED → COLOR_ATTACHMENT_OPTIMAL →
//!    `begin_rendering` (loadOp = CLEAR to a known color) → `end_rendering` →
//!    `image_barrier` COLOR_ATTACHMENT_OPTIMAL → TRANSFER_SRC_OPTIMAL →
//!    `copy_image_to_buffer` into a host-visible staging buffer.
//! 4. Submit once, fence-wait, map-read the staging buffer, and assert EVERY texel
//!    equals the clear color.
//! 5. Assert the validation messenger recorded ZERO messages — the soundness
//!    oracle for the GPU half (Miri cannot check raw driver FFI).
//!
//! # CI gate (graceful skip)
//!
//! A GPU-less / loader-less host, or one without the SDK's validation layer (or
//! without `dynamicRendering`), makes `VulkanContext::boot` return `Err`; the test
//! skips gracefully.

use boyko_rhi::{
    BufferDesc, BufferImageCopy, BufferUsage, Format, ImageAspect, ImageBarrierDesc, ImageLayout,
    ImageSubresourceRange, ImageUsage, LoadOp, MemoryLocation, RenderArea, RenderingAttachment,
    RenderingDesc, RhiCommandEncoder, RhiDevice, RhiQueue, StoreOp, TextureDesc,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};

use boyko_rhi::enums::{BarrierAccess, BarrierStage};

/// The offscreen image dimensions. Small but multi-texel so a partial / stale
/// clear would mismatch loudly.
const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;
/// Texel count + byte size (R8G8B8A8 = 4 bytes/texel).
const TEXELS: usize = (WIDTH * HEIGHT) as usize;
const SIZE: u64 = (TEXELS * 4) as u64;

/// The known clear color, per byte (R, G, B, A) for R8G8B8A8_UNORM. Each byte is
/// expressed as `byte / 255.0` so the UNORM float→byte conversion is exact (the
/// golden readback compares the exact bytes `0xAA 0xBB 0xCC 0xDD`). Mirrors the
/// plan's `0x_AA_BB_CC_DD` suggestion.
const CLEAR_BYTES: [u8; 4] = [0xAA, 0xBB, 0xCC, 0xDD];

/// The clear color as the RGBA floats `begin_rendering` takes.
fn clear_floats() -> [f32; 4] {
    [
        CLEAR_BYTES[0] as f32 / 255.0,
        CLEAR_BYTES[1] as f32 / 255.0,
        CLEAR_BYTES[2] as f32 / 255.0,
        CLEAR_BYTES[3] as f32 / 255.0,
    ]
}

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
        "validation layer reported {} message(s) during the offscreen clear — see the [vk-validation] log",
        state.total()
    );
}

#[test]
fn offscreen_clear_golden_round_trip() {
    let Some(ctx) = boot_or_skip("offscreen_clear_golden_round_trip") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    assert!(ctx.validation_enabled(), "validation must be active");

    let device: &VulkanContext = &ctx;
    let queue = ctx.rhi_queue();

    // The offscreen color image: cleared as a color attachment, then read back as a
    // transfer source.
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

    // The host-visible staging buffer the image is read back into.
    let staging = device
        .create_buffer(&BufferDesc {
            size: SIZE,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible readback staging buffer");

    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");

    // Record the whole rung-1 thread.
    encoder.begin().expect("begin");

    // UNDEFINED → COLOR_ATTACHMENT_OPTIMAL (TOP_OF_PIPE → COLOR_ATTACHMENT_OUTPUT),
    // the acquire→render transition (abstracted from `swapchain.rs::record_clear`).
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

    // Clear the whole image to the known color via dynamic rendering.
    let attachment = [RenderingAttachment {
        texture: &color,
        layout: ImageLayout::ColorAttachmentOptimal,
        load_op: LoadOp::Clear,
        store_op: StoreOp::Store,
        clear_color: clear_floats(),
    }];
    encoder.begin_rendering(&RenderingDesc {
        render_area: RenderArea {
            x: 0,
            y: 0,
            width: WIDTH,
            height: HEIGHT,
        },
        colors: &attachment,
        depth: None,
    });
    encoder.end_rendering();

    // COLOR_ATTACHMENT_OPTIMAL → TRANSFER_SRC_OPTIMAL so the readback copy can read
    // the cleared image (COLOR_ATTACHMENT_OUTPUT write → TRANSFER read).
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

    // Copy the whole image → the staging buffer (tightly packed).
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

    // Submit once + fence-wait; the cleared bytes can only reach the staging buffer
    // through the real GPU clear + copy.
    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    // Golden: read back the staging buffer and assert EVERY texel equals the clear.
    let dst_ptr = device
        .buffer_mapped_ptr(&staging)
        .expect("host-visible staging buffer is mapped");
    // SAFETY: `dst_ptr` points to `SIZE` mapped host-coherent bytes; a fence wait
    // preceded this read, so the GPU clear + copy are complete + coherent; reading
    // `SIZE` bytes is in-bounds; `out` is a distinct, non-overlapping allocation.
    let mut out = vec![0u8; SIZE as usize];
    unsafe {
        core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), SIZE as usize);
    }
    for texel in 0..TEXELS {
        let base = texel * 4;
        let got = [out[base], out[base + 1], out[base + 2], out[base + 3]];
        assert_eq!(
            got, CLEAR_BYTES,
            "texel {texel} mismatched after offscreen clear: got {got:02x?}, want {CLEAR_BYTES:02x?}"
        );
    }

    // The oracle: a clean run records zero validation messages.
    assert_validation_clean(&ctx);

    // Teardown. The encoder's last submission completed (fence-waited above), so
    // destroying everything is sound; reverse-ish order.
    // SAFETY: each resource was created on `device`, its GPU work has completed (the
    // fence was waited), and each is destroyed exactly once here.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_buffer(staging);
        device.destroy_texture(color);
    }
    drop(ctx);
}
