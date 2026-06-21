//! GUI P5b — the MULTI-SCALE MSDF crispness GPU golden (RTX 3060, validation clean):
//! the plan's **G-T4.3 sharp-corners-at-scale** gate, driven through the LIVE present
//! path (the same `ui_setup` → `pack_ui_instance(text_uv)` → `ui_upload` → `ui_handles`
//! (MF-7) → `record_ui_rects` recorder the on-screen UI sub-pass uses), into an offscreen
//! `LoadOp::Load` target, read back per-texel.
//!
//! # What this proves the shipped single-scale golden does NOT
//!
//! `ui_text_gpu_golden` renders at ONE 16-px quad and proves median-over-SDF (G-T4.2).
//! This golden renders the SAME MSDF glyph cell at a SMALL (16 px) AND a LARGE (96 px)
//! quad and asserts the median-reconstructed feature edge stays CRISP at BOTH:
//!
//! * the median-high band (channels `0.9,0.9,0.1` ⇒ `median = 0.9 ≥ 0.5`, a feature a
//!   single-channel SDF with `.a = 0.1` would ROUND OFF) renders FOREGROUND at 16 px AND
//!   at 96 px — the MSDF feature is scale-invariant (it does not wash out when minified
//!   nor smear when magnified);
//! * the exterior band (`median = 0`) stays BACKGROUND at BOTH scales — the interior /
//!   exterior boundary is a HARD edge at every scale, not a rounded blob (the defining
//!   sharp-corner property the median preserves and `screenPxRange` keeps to ~1–2 device
//!   px of AA regardless of quad size).
//!
//! Sampling the centre column of a full-height authored BAND (not a single texel) keeps
//! each assert robust to bilinear sub-texel blending at both magnification factors.
//!
//! Run the GPU goldens with `--test-threads=1` (offscreen GPU tests contend otherwise).

mod common;

use boyko_rhi::{
    BarrierAccess, BarrierStage, BufferDesc, BufferImageCopy, BufferUsage, Format, ImageAspect,
    ImageBarrierDesc, ImageLayout, ImageSubresourceRange, ImageUsage, LoadOp, MemoryLocation,
    RenderArea, RenderingAttachment, RenderingDesc, RhiCommandEncoder, RhiDevice, RhiQueue,
    StoreOp, TextureDesc, TextureDimension,
};
use boyko_render::{pack_ui_instance, record_ui_rects, PackInput, RhiContext, UiInstance, UiOrtho};

use boyko_fontbake::atlas::{AtlasImage, AtlasKind, AtlasMeta, BakedFont, GlyphMetrics};

use common::{assert_validation_clean, boot_or_skip};

/// A 128×128 offscreen target — large enough to hold a 96-px glyph quad with margin.
const WIDTH: u32 = 128;
const HEIGHT: u32 = 128;
const TEXELS: usize = (WIDTH * HEIGHT) as usize;
const SIZE: u64 = (TEXELS * 4) as u64;

const CLEAR_BYTES: [u8; 4] = [0x11, 0x22, 0x33, 0xFF];
const FG: u32 = 0xFF00_00FF; // opaque RED, straight RGBA8
const FG_BYTES: [u8; 4] = [0xFF, 0x00, 0x00, 0xFF];

/// One 4×4 glyph cell: a TOP median-high band (y∈[0,2)) where `median(0.9,0.9,0.1)=0.9`
/// (kept) but `.a=0.1` (an SDF would drop it), and a BOTTOM exterior band (y∈[2,4),
/// `median=0`). The same cell, sampled by both the small and the large quad.
const ATLAS_W: u32 = 4;
const ATLAS_H: u32 = 4;

fn texel(r: f32, g: f32, b: f32, a: f32) -> [u8; 4] {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [q(r), q(g), q(b), q(a)]
}

/// The hand-authored single-cell MTSDF font (the median-vs-SDF discriminator).
fn authored_font() -> BakedFont {
    let mut pixels = vec![0u8; (ATLAS_W * ATLAS_H * 4) as usize];
    let mut put = |x: u32, y: u32, t: [u8; 4]| {
        let i = ((y * ATLAS_W + x) * 4) as usize;
        pixels[i..i + 4].copy_from_slice(&t);
    };
    for y in 0..2 {
        for x in 0..4 {
            put(x, y, texel(0.9, 0.9, 0.1, 0.1)); // median 0.9 ≥ 0.5; SDF control 0.1 < 0.5
        }
    }
    for y in 2..4 {
        for x in 0..4 {
            put(x, y, texel(0.0, 0.0, 0.0, 0.0)); // exterior
        }
    }

    let glyphs = vec![GlyphMetrics {
        advance_em: 0.5,
        plane: [0.0, 0.0, 1.0, 1.0],
        atlas: [0.0, 4.0, 4.0, 0.0], // the whole cell, [left, bottom, right, top] texels
    }];

    BakedFont {
        meta: AtlasMeta {
            distance_range_texels: 6.0,
            pixels_per_em: 48.0,
            atlas_w: ATLAS_W,
            atlas_h: ATLAS_H,
            ascender_em: 0.8,
            descender_em: -0.2,
            line_gap_em: 0.0,
            kind: AtlasKind::Mtsdf,
        },
        glyphs,
        cmap: Vec::new(),
        kern: Vec::new(),
        atlas: AtlasImage { width: ATLAS_W, height: ATLAS_H, pixels },
    }
}

fn texel_base(x: u32, y: u32) -> usize {
    ((y * WIDTH + x) * 4) as usize
}

fn floats(bytes: [u8; 4]) -> [f32; 4] {
    [
        bytes[0] as f32 / 255.0,
        bytes[1] as f32 / 255.0,
        bytes[2] as f32 / 255.0,
        bytes[3] as f32 / 255.0,
    ]
}

/// The normalized UV rect for the single cell `(left, top, right, bottom)` in `[0,1]`
/// (matching `shape::quad_uv`'s ordering: `top` = smaller texel-Y → quad v=0).
fn cell_uv(g: &GlyphMetrics) -> [f32; 4] {
    let aw = ATLAS_W as f32;
    let ah = ATLAS_H as f32;
    [g.atlas[0] / aw, g.atlas[3] / ah, g.atlas[2] / aw, g.atlas[1] / ah]
}

/// One glyph quad at `(x, y, w, h)` sampling `uv` — the SAME `pack_ui_instance` text lane.
fn glyph_quad(x: f32, y: f32, w: f32, h: f32, uv: [f32; 4]) -> UiInstance {
    pack_ui_instance(
        &PackInput {
            rect: [x, y, w, h],
            color: FG,
            border_color: 0,
            corner_radius: [0.0; 4],
            border_width: [0.0; 4],
            clip: None,
            text_uv: Some(uv),
        },
        1.0,
    )
}

/// Renders the SAME cell at a small + a large quad through the live path, returns the
/// readback.
fn render_multiscale(rhi: &mut RhiContext) -> Vec<u8> {
    let font = authored_font();
    let uv = cell_uv(&font.glyphs[0]);

    rhi.ui_setup(
        Format::R8G8B8A8Unorm,
        boyko_render::ui_rect_vs_spirv(),
        boyko_render::ui_rect_fs_spirv(),
        4,
        &font,
    )
    .expect("ui_setup (UI pipeline + atlas upload + per-FIF rings)");

    // Small quad: 16×16 at (8,8). Large quad: 96×96 at (24,24). Both sample the SAME
    // cell; the top half (v∈[0,0.5)) is the median-high band, the bottom half exterior.
    let instances = [
        glyph_quad(8.0, 8.0, 16.0, 16.0, uv),
        glyph_quad(24.0, 24.0, 96.0, 96.0, uv),
    ];
    let ortho = UiOrtho::for_extent(WIDTH, HEIGHT);
    let plan = rhi
        .ui_upload(&instances, ortho, 0)
        .expect("ui_upload (memcpy into the FIF ring + POD UiFramePlan)");
    assert_eq!(plan.instance_count, 2, "two glyph instances (small + large) uploaded");

    let (pipeline, bind_group) = rhi.ui_handles(plan.frame_index).expect("ui_handles after ui_setup");

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
        })
        .expect("offscreen output texture");
    let staging = device
        .create_buffer(&BufferDesc {
            size: SIZE,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("host-visible readback staging buffer");
    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("command encoder");
    let full = RenderArea { x: 0, y: 0, width: WIDTH, height: HEIGHT };

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
    let clear_attachment = [RenderingAttachment {
        texture: &output,
        layout: ImageLayout::ColorAttachmentOptimal,
        load_op: LoadOp::Clear,
        store_op: StoreOp::Store,
        clear_color: floats(CLEAR_BYTES),
    }];
    encoder.begin_rendering(&RenderingDesc { render_area: full, colors: &clear_attachment, depth: None });
    encoder.end_rendering();

    let ui_attachment = [RenderingAttachment {
        texture: &output,
        layout: ImageLayout::ColorAttachmentOptimal,
        load_op: LoadOp::Load,
        store_op: StoreOp::Store,
        clear_color: [0.0; 4],
    }];
    encoder.begin_rendering(&RenderingDesc { render_area: full, colors: &ui_attachment, depth: None });
    // SAFETY: recording is open in a `begin_rendering(LoadOp::Load)` scope whose single
    // color attachment format (`R8G8B8A8Unorm`) equals the UI pipeline's color_formats[0]
    // at `full`; `pipeline`/`bind_group` are the live current-frame (MF-7) UI handles
    // whose ring holds `plan.instance_count` valid records uploaded for `plan.frame_index`.
    unsafe {
        record_ui_rects(&mut encoder, &full, &plan, pipeline, bind_group);
    }
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

    let dst_ptr = device.buffer_mapped_ptr(&staging).expect("host-visible staging buffer is mapped");
    let mut out = vec![0u8; SIZE as usize];
    // SAFETY: `dst_ptr` points to `SIZE` mapped host-coherent bytes; the fence wait
    // ordered this read after the draw + copy; `out` is a distinct `SIZE`-byte allocation.
    unsafe {
        core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), SIZE as usize);
    }
    // SAFETY: each transient resource was created on `device`, its GPU work completed
    // (fence-waited), and each is destroyed exactly once here.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_buffer(staging);
        device.destroy_texture(output);
    }
    out
}

#[test]
fn ui_text_msdf_corner_stays_crisp_at_small_and_large_scale_golden() {
    let Some(ctx) = boot_or_skip("ui_text_msdf_corner_stays_crisp_at_small_and_large_scale_golden")
    else {
        return;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    assert!(ctx.validation_enabled(), "validation must be active");

    let mut rhi = RhiContext::new(ctx);
    let out = render_multiscale(&mut rhi);
    let at = |x: u32, y: u32| -> [u8; 4] {
        let b = texel_base(x, y);
        [out[b], out[b + 1], out[b + 2], out[b + 3]]
    };

    // ── SMALL quad (8,8)–(24,24), 16 px. Top half y∈[8,16) ⇒ atlas v∈[0,0.5) (median
    //    band); bottom half y∈[16,24) ⇒ v∈[0.5,1) (exterior). Centre-x 16 avoids the
    //    cell's left/right bilinear edge.
    assert_eq!(
        at(16, 11),
        FG_BYTES,
        "G-T4.3 small: the MSDF median feature must be FOREGROUND at 16 px (median 0.9 ≥ 0.5; \
         an SDF control 0.1 would round it off): got {:02x?}",
        at(16, 11)
    );
    assert_eq!(
        at(16, 21),
        CLEAR_BYTES,
        "G-T4.3 small: the exterior band must stay BACKGROUND at 16 px (a HARD interior/exterior \
         edge, not a rounded blob): got {:02x?}",
        at(16, 21)
    );

    // ── LARGE quad (24,24)–(120,120), 96 px. Top half y∈[24,72) ⇒ v∈[0,0.5) (median
    //    band); bottom half y∈[72,120) ⇒ v∈[0.5,1) (exterior). Centre-x 72.
    assert_eq!(
        at(72, 44),
        FG_BYTES,
        "G-T4.3 large: the SAME MSDF median feature must be FOREGROUND at 96 px (scale-invariant; \
         it does not wash out when magnified 6×): got {:02x?}",
        at(72, 44)
    );
    assert_eq!(
        at(72, 100),
        CLEAR_BYTES,
        "G-T4.3 large: the exterior band must stay BACKGROUND at 96 px (the interior/exterior \
         boundary is a HARD edge at every scale — the defining sharp-corner MSDF property): got {:02x?}",
        at(72, 100)
    );

    // The GPU-half soundness oracle: zero validation messages across the multi-scale draw.
    assert_validation_clean(rhi.context());

    rhi.destroy_all();
    drop(rhi);
}
