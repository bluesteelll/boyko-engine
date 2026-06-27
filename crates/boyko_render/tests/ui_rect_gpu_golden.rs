//! GUI P5a Rung 5 — the END-TO-END UI-rect GPU golden (RTX 3060, validation clean).
//!
//! Proves the FULL `boyko_render` UI render path on a real device, not just the CPU
//! pack: `RhiContext::ui_setup` (the owned UI pipeline + bind-group layout + per-FIF
//! host-mapped STORAGE rings) → `pack_ui_instance` → `RhiContext::ui_upload` (memcpy
//! into the current-FIF ring + the POD `UiFramePlan`) → `RhiContext::ui_handles`
//! (the by-`frame_index` re-resolution, MF-7) → the shared, `RhiApi`-generic
//! `record_ui_rects` recorder (the ONE instanced `draw(6, N, 0, 0)` into a fresh
//! `LoadOp::Load` full-extent scope) → readback.
//!
//! This is the test that makes `record_ui_rects` LIVE (it was unexercised before) and
//! that exercises the same `RhiContext` UI capability the on-screen `present_sampled`
//! UI sub-pass uses, through the trait-encoder offscreen path (the concrete-present /
//! trait-test split the recorder is generic for).
//!
//! # The scene + the proof
//!
//! Two packed UI rects are drawn into a 64×64 `R8G8B8A8` offscreen target via the
//! `UiOrtho::for_extent(64,64)` pixel→NDC transform (top-left origin):
//! - instance 0: an opaque RED rect at min (8,8) size 16 → interior centre (16,16),
//! - instance 1: an opaque GREEN rect at min (40,40) size 16 → interior centre (48,48).
//!
//! Both rects are OPAQUE (alpha 255 → the premultiply is identity) with no border and
//! no clip, so each interior texel is the rect's straight color composited over the
//! opaque CLEAR background under the pipeline's premultiplied-alpha blend
//! (`src + dst*(1-src_a)` = `src` where `src_a == 1`). Decisive assertions:
//! - the RED rect's interior centre texel == RED (the VS placed it via the SSBO
//!   min/size, the FS filled it via the SSBO color — both stages off one descriptor),
//! - the GREEN rect's interior centre texel == GREEN (a distinct SSBO record),
//! - an uncovered texel == the CLEAR color (genuine per-instance placement, not a
//!   full-screen fill — `src == 0` there, so the blend keeps `dst` == CLEAR),
//! - the validation messenger == ZERO messages (the GPU-half soundness oracle).
//!
//! # CI gate (graceful skip)
//!
//! A GPU-less / loader-less / validation-less host makes `VulkanContext::boot` return
//! `Err`; the test skips gracefully (mirrors `ssbo_graphics_probe` / `round_trip`).

mod common;

use boyko_rhi::{
    BarrierAccess, BarrierStage, BufferDesc, BufferImageCopy, BufferUsage, Format, ImageAspect,
    ImageBarrierDesc, ImageLayout, ImageSubresourceRange, ImageUsage, LoadOp, MemoryLocation,
    RenderArea, RenderingAttachment, RenderingDesc, RhiCommandEncoder, RhiDevice, RhiQueue,
    StoreOp, TextureDesc, TextureDimension,
};
use boyko_render::{
    pack_ui_instance, record_ui_rects, PackInput, RhiContext, UiInstance, UiOrtho,
};

use common::{assert_validation_clean, boot_or_skip};

/// The offscreen image dimensions — small but multi-texel so a covered/uncovered
/// boundary and the per-instance placement are unambiguous (matches the Rung-0.5
/// probe geometry so the centre-texel expectations carry over).
const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;
const TEXELS: usize = (WIDTH * HEIGHT) as usize;
const SIZE: u64 = (TEXELS * 4) as u64;

/// The offscreen CLEAR color (the texel an uncovered sample keeps). Opaque so the
/// premultiplied-alpha blend over it is deterministic where `src == 0`.
const CLEAR_BYTES: [u8; 4] = [0x11, 0x22, 0x33, 0xFF];
/// Instance 0's straight RGBA8 fill (opaque RED) — top-left.
const RED: u32 = 0xFF00_00FF; // byte0=R=FF, byte3=A=FF
const RED_BYTES: [u8; 4] = [0xFF, 0x00, 0x00, 0xFF];
/// Instance 1's straight RGBA8 fill (opaque GREEN) — bottom-right.
const GREEN: u32 = 0xFF00_FF00; // byte1=G=FF, byte3=A=FF
const GREEN_BYTES: [u8; 4] = [0x00, 0xFF, 0x00, 0xFF];

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

/// The byte index of texel `(x, y)` in the tightly-packed R8G8B8A8 readback.
fn texel_base(x: u32, y: u32) -> usize {
    ((y * WIDTH + x) * 4) as usize
}

/// One opaque, border-less, clip-less rect at logical-px `(x, y, w, h)` with straight
/// `color`. `scale_factor == 1.0`, so logical px == physical px in this golden.
fn opaque_rect(x: f32, y: f32, w: f32, h: f32, color: u32) -> UiInstance {
    pack_ui_instance(
        &PackInput {
            rect: [x, y, w, h],
            color,
            border_color: 0,
            corner_radius: [0.0; 4],
            border_width: [0.0; 4],
            clip: None,
            text_uv: None,
        },
        1.0,
    )
}

/// A minimal 1×1 MTSDF `BakedFont` so `ui_setup` can build its 3-binding bind-group
/// (the atlas binding is always present — GUI P5b Decision T4-C). This rect-only
/// golden emits no glyphs, so the atlas content is irrelevant; it just must exist.
fn tiny_font() -> boyko_fontbake::atlas::BakedFont {
    use boyko_fontbake::atlas::{AtlasImage, AtlasKind, AtlasMeta, BakedFont};
    BakedFont {
        meta: AtlasMeta {
            distance_range_texels: 6.0,
            pixels_per_em: 48.0,
            atlas_w: 1,
            atlas_h: 1,
            ascender_em: 0.8,
            descender_em: -0.2,
            line_gap_em: 0.0,
            kind: AtlasKind::Mtsdf,
        },
        glyphs: Vec::new(),
        cmap: Vec::new(),
        kern: Vec::new(),
        atlas: AtlasImage {
            width: 1,
            height: 1,
            pixels: vec![0u8; 4],
        },
    }
}

/// Renders the two-instance UI scene through the real `RhiContext` UI capability +
/// the `record_ui_rects` recorder, into an offscreen target, and returns the readback.
fn render_ui_golden(rhi: &mut RhiContext) -> Vec<u8> {
    // --- 1. SETUP: build the owned UI pipeline + per-FIF rings for R8G8B8A8. ---
    let font = tiny_font();
    rhi.ui_setup(
        Format::R8G8B8A8Unorm,
        boyko_render::ui_rect_vs_spirv(),
        boyko_render::ui_rect_fs_spirv(),
        // A tiny initial ring (2 rows) so this golden also crosses the grow path
        // (3 instances > 2) on a second frame if extended; here 2 instances fit.
        2,
        &font,
    )
    .expect("ui_setup (UI pipeline + bind-group layout + per-FIF rings)");

    // --- 2. PACK + UPLOAD: two opaque rects into frame-in-flight slot 0. ---
    let instances = [
        opaque_rect(8.0, 8.0, 16.0, 16.0, RED),
        opaque_rect(40.0, 40.0, 16.0, 16.0, GREEN),
    ];
    let ortho = UiOrtho::for_extent(WIDTH, HEIGHT);
    let frame_index = 0usize;
    let plan = rhi
        .ui_upload(&instances, ortho, frame_index)
        .expect("ui_upload (memcpy into the current-FIF ring + POD UiFramePlan)");
    assert_eq!(plan.instance_count, 2, "two instances uploaded");
    assert_eq!(plan.frame_index, frame_index, "the plan carries the slot index");

    // --- 3. RE-RESOLVE the current-frame handles by frame_index (MF-7). ---
    let (pipeline, bind_group) = rhi
        .ui_handles(plan.frame_index)
        .expect("ui_handles after ui_setup");

    // --- 4. RECORD: an offscreen target, then `record_ui_rects` into a fresh
    //        LoadOp::Load full-extent scope (the recorder's contract), then readback. ---
    let device = rhi.context();
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
        })
        .expect("offscreen output texture (COLOR_ATTACHMENT | TRANSFER_SRC)");

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

    encoder.begin().expect("begin");

    // UNDEFINED → COLOR_ATTACHMENT.
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

    // CLEAR pass: paint the opaque CLEAR background (the swapchain "composite" the
    // on-screen path would have rendered first). Closed immediately, so the UI pass
    // below opens its OWN LoadOp::Load scope — exactly the Decision-9 contract.
    let clear_attachment = [RenderingAttachment {
        texture: &output,
        layout: ImageLayout::ColorAttachmentOptimal,
        load_op: LoadOp::Clear,
        store_op: StoreOp::Store,
        clear_color: floats(CLEAR_BYTES),
    }];
    encoder.begin_rendering(&RenderingDesc {
        render_area: full,
        colors: &clear_attachment,
        depth: None,
    });
    encoder.end_rendering();

    // UI pass: a FRESH LoadOp::Load full-extent scope (preserve the CLEAR), then the
    // shared recorder records the one instanced draw. This is the exact sequence the
    // on-screen `record_present_sampled` UI sub-pass performs (a concrete-handle copy
    // of this trait-generic recorder).
    let ui_attachment = [RenderingAttachment {
        texture: &output,
        layout: ImageLayout::ColorAttachmentOptimal,
        load_op: LoadOp::Load,
        store_op: StoreOp::Store,
        clear_color: [0.0; 4],
    }];
    encoder.begin_rendering(&RenderingDesc {
        render_area: full,
        colors: &ui_attachment,
        depth: None,
    });
    // SAFETY: recording is open inside a `begin_rendering(LoadOp::Load)` scope whose
    // single color attachment's format (`R8G8B8A8Unorm`) equals the UI pipeline's
    // `color_formats[0]` (passed to `ui_setup`), at `full`; `pipeline`/`bind_group`
    // are the live, current-frame-re-resolved (MF-7) UI handles whose backing ring
    // holds `plan.instance_count` valid records uploaded for `plan.frame_index`
    // above; the pipeline declares the UI bind-group layout at set 0 and a 16-byte
    // VERTEX-stage push range (`UiOrtho`). The recorder pushes `plan.ortho` (VERTEX),
    // sets the full-extent viewport+scissor, and records one `draw(6, N, 0, 0)`.
    unsafe {
        record_ui_rects(&mut encoder, &full, &plan, pipeline, bind_group);
    }
    encoder.end_rendering();

    // COLOR → TRANSFER_SRC, then copy the image to the staging buffer.
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
    // SAFETY: `dst_ptr` points to `SIZE` mapped host-coherent bytes; the fence wait
    // above ordered this read after the draw + copy completed; reading `SIZE` bytes is
    // in-bounds; `out` is a distinct, non-overlapping allocation.
    unsafe {
        core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), SIZE as usize);
    }

    // Teardown the transient offscreen resources (the fence above ordered their last
    // GPU use). The UI capability + device are owned by `rhi` (torn down by the
    // caller's `destroy_all` / `Drop`).
    // SAFETY: each was created on `device`, its GPU work completed (fence-waited), and
    // each is destroyed exactly once here.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_buffer(staging);
        device.destroy_texture(output);
    }

    out
}

#[test]
fn ui_rects_render_through_the_full_render_path_golden() {
    let Some(ctx) = boot_or_skip("ui_rects_render_through_the_full_render_path_golden") else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    assert!(ctx.validation_enabled(), "validation must be active");

    let mut rhi = RhiContext::new(ctx);
    let out = render_ui_golden(&mut rhi);

    // Instance 0: RED rect at min (8,8) size 16 → interior centre texel (16,16).
    let red = texel_base(16, 16);
    let red_texel = [out[red], out[red + 1], out[red + 2], out[red + 3]];
    assert_eq!(
        red_texel, RED_BYTES,
        "instance 0's interior must be RED (the VS placed it via the SSBO min/size, the FS filled it via the SSBO color): got {red_texel:02x?}"
    );

    // Instance 1: GREEN rect at min (40,40) size 16 → interior centre texel (48,48).
    let green = texel_base(48, 48);
    let green_texel = [out[green], out[green + 1], out[green + 2], out[green + 3]];
    assert_eq!(
        green_texel, GREEN_BYTES,
        "instance 1's interior must be GREEN (a distinct SSBO record drives a distinct quad + color): got {green_texel:02x?}"
    );

    // A texel covered by NEITHER rect keeps the CLEAR color (genuine per-instance
    // placement under the LoadOp::Load UI pass, not a full-screen fill).
    let bg = texel_base(60, 4);
    let bg_texel = [out[bg], out[bg + 1], out[bg + 2], out[bg + 3]];
    assert_eq!(
        bg_texel, CLEAR_BYTES,
        "an uncovered texel must keep the CLEAR color (the UI pass loaded, did not clear): got {bg_texel:02x?}"
    );

    assert_validation_clean(rhi.context());

    rhi.destroy_all();
    drop(rhi);
}
