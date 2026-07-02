//! GUI P5b — the END-TO-END MSDF TEXT GPU golden (RTX 3060, validation clean).
//!
//! This is the proof that text is GENUINELY rendered through the LIVE P5a present
//! path, not stubbed: a real baked MTSDF atlas is uploaded by `ui_setup`, glyph
//! instances are produced by the SAME `pack_ui_instance(PackInput{ text_uv: Some(uv),
//! .. })` lane the emitter feeds, uploaded through `ui_upload`, re-resolved by
//! `ui_handles` (MF-7), and drawn by the SHARED `record_ui_rects` recorder (the exact
//! recorder the on-screen `record_present_sampled` UI sub-pass uses) into an offscreen
//! `LoadOp::Load` scope, then read back per-texel. The atlas sample + `median3` +
//! `screenPxRange` MSDF branch (`FLAG_TEXT`) and the binding-1 atlas / binding-2 UBO
//! are all exercised on the device — closing the "no call site produces a glyph
//! instance reaching the swapchain" finding.
//!
//! # Why a hand-authored atlas (not the full bake on-device)
//!
//! The bake pipeline's per-texel float field is gated by its own CPU goldens
//! (`boyko_fontbake::tests::gate_goldens`). THIS golden isolates the RUNTIME shader +
//! upload + UV path: it authors an MTSDF atlas with EXACT texel values so every
//! interior/exterior/corner assertion is bit-deterministic and independent of the
//! generator's float behavior. The atlas is a genuine `BakedFont` (`AtlasKind::Mtsdf`,
//! RGBA8 `w*h*4`), travelling the same `ui_setup` → `create_atlas` → staged-copy →
//! `ShaderReadOnlyOptimal` upload as a production atlas.
//!
//! # The scene + the proof (the plan's mandatory G-T4.x gates)
//!
//! Two distinct glyphs are emitted via the text lane into a 64×64 `R8G8B8A8` offscreen
//! target (top-left ortho), each a `text_uv: Some(uv)` instance pointing at a distinct
//! atlas cell:
//! - glyph A (a solid block) at a large quad → its interior texels are foreground.
//! - glyph B (a different footprint) at a separate quad → renders distinctly.
//!
//! Decisive assertions:
//! - **G-T4.1 crisp glyph**: glyph A's interior texel == the foreground color
//!   (median ≥ 0.5 ⇒ coverage 1, premultiplied fg over the CLEAR background); a texel
//!   well OUTSIDE glyph A's quad == the CLEAR color (the glyph did not bleed).
//! - **G-T4.2 median preserves an edge vs an SDF control**: glyph B's atlas carries a
//!   median-high BAND whose three MSDF channels DISAGREE such that `median(r,g,b) ≥ 0.5`
//!   (the feature is KEPT) while the single-channel control (`.a`, a true SDF) is
//!   `< 0.5` (it would be ROUNDED OFF). The rendered band texel is fg — proving the FS
//!   uses the median, the defining MSDF property. (A full-height BAND, not a single
//!   texel, so the assertion is robust to bilinear sub-texel blending.)
//! - **G-T4.4 distinct glyphs**: glyph A and glyph B sample different atlas cells and
//!   render their own footprints (a texel inside A is fg, the matching texel in B's
//!   exterior band is background) — proving the UV lane + the `corner_radius`→`uv`
//!   alias offset are correct (not a constant).
//! - validation messenger == ZERO messages (the GPU-half soundness oracle).
//!
//! # CI gate (graceful skip)
//!
//! A GPU-less / loader-less / validation-less host makes `VulkanContext::boot` return
//! `Err`; the test skips gracefully (mirrors `ui_rect_gpu_golden`). Run the GPU
//! goldens with `--test-threads=1` (windowed/offscreen GPU tests contend otherwise).

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

/// The offscreen image dimensions (matches the rect golden geometry).
const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;
const TEXELS: usize = (WIDTH * HEIGHT) as usize;
const SIZE: u64 = (TEXELS * 4) as u64;

/// The offscreen CLEAR color (the texel an uncovered/exterior sample keeps). Opaque so
/// the premultiplied-alpha blend over it is deterministic where `src == 0`.
const CLEAR_BYTES: [u8; 4] = [0x11, 0x22, 0x33, 0xFF];
/// The glyph foreground (opaque RED) — STRAIGHT RGBA8, premultiply is identity at A=255.
const FG: u32 = 0xFF00_00FF; // byte0=R=FF, byte3=A=FF
const FG_BYTES: [u8; 4] = [0xFF, 0x00, 0x00, 0xFF];

/// The hand-authored MTSDF atlas dimensions. A 8×4 atlas holds two 4×4 glyph cells
/// side by side (cell A at x∈[0,4), cell B at x∈[4,8)), each large enough that a glyph
/// quad scaled over it samples interior/exterior/corner texels unambiguously.
const ATLAS_W: u32 = 8;
const ATLAS_H: u32 = 4;

/// One MTSDF texel as RGBA8 from per-channel `[0,1]` distances. `.rgb` are the MSDF
/// channels (the FS takes their `median`); `.a` is the true single-channel SDF control
/// (the FS does NOT sample it — it is here only to author a corner where the median and
/// the single-channel control DISAGREE, for G-T4.2 documentation).
fn texel(r: f32, g: f32, b: f32, a: f32) -> [u8; 4] {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [q(r), q(g), q(b), q(a)]
}

/// Builds the hand-authored MTSDF [`BakedFont`].
///
/// Cell A (x∈[0,4)) is a SOLID interior block: every texel has `median(r,g,b) = 1.0`
/// (fully inside), so a glyph quad over it reads foreground everywhere → G-T4.1.
///
/// Cell B (x∈[4,8)) is split into a TOP median-high BAND (y∈[0,2)) and a BOTTOM EXTERIOR
/// band (y∈[2,4)). The top band's channels are authored so `median(r,g,b) ≥ 0.5` (the
/// MSDF KEEPS the feature) while `.a < 0.5` (a single-channel SDF would ROUND it off) —
/// the G-T4.2 median-vs-SDF discriminator. The bottom band is exterior (`median = 0`) so
/// B renders DISTINCTLY from A's solid interior (G-T4.4). Full-height BANDS (not a single
/// texel) keep the per-texel asserts robust to bilinear sub-texel blending.
fn authored_font() -> BakedFont {
    let mut pixels = vec![0u8; (ATLAS_W * ATLAS_H * 4) as usize];
    let mut put = |x: u32, y: u32, t: [u8; 4]| {
        let i = ((y * ATLAS_W + x) * 4) as usize;
        pixels[i..i + 4].copy_from_slice(&t);
    };

    // Cell A: a solid interior block (median == 1 everywhere). a == 1 too.
    for y in 0..4 {
        for x in 0..4 {
            put(x, y, texel(1.0, 1.0, 1.0, 1.0));
        }
    }

    // Cell B top band (y∈[0,2)): the median-vs-SDF discriminator. Two channels high, one
    // low ⇒ median(0.9, 0.9, 0.1) = 0.9 (≥ 0.5, the feature is KEPT by the median), while
    // the single-channel control a = 0.1 (< 0.5, an SDF rounds it off). The FS takes
    // median(rgb), so this band renders foreground — the defining MSDF property.
    for y in 0..2 {
        for x in 4..8 {
            put(x, y, texel(0.9, 0.9, 0.1, 0.1));
        }
    }
    // Cell B bottom band (y∈[2,4)): exterior (median == 0) ⇒ background.
    for y in 2..4 {
        for x in 4..8 {
            put(x, y, texel(0.0, 0.0, 0.0, 0.0));
        }
    }

    // Two glyph slots: slot 0 == cell A, slot 1 == cell B (atlas bounds in TEXELS,
    // [left, bottom, right, top]; plane unused here — the test sets quads directly).
    let glyphs = vec![
        GlyphMetrics {
            advance_em: 0.5,
            plane: [0.0, 0.0, 1.0, 1.0],
            atlas: [0.0, 4.0, 4.0, 0.0], // cell A: x∈[0,4), full height
        },
        GlyphMetrics {
            advance_em: 0.5,
            plane: [0.0, 0.0, 1.0, 1.0],
            atlas: [4.0, 4.0, 8.0, 0.0], // cell B: x∈[4,8), full height
        },
    ];

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
        atlas: AtlasImage {
            width: ATLAS_W,
            height: ATLAS_H,
            pixels,
        },
    }
}

/// The byte index of texel `(x, y)` in the tightly-packed R8G8B8A8 readback.
fn texel_base(x: u32, y: u32) -> usize {
    ((y * WIDTH + x) * 4) as usize
}

/// The CLEAR color as the RGBA floats `begin_rendering` takes.
fn floats(bytes: [u8; 4]) -> [f32; 4] {
    [
        bytes[0] as f32 / 255.0,
        bytes[1] as f32 / 255.0,
        bytes[2] as f32 / 255.0,
        bytes[3] as f32 / 255.0,
    ]
}

/// A normalized UV rect for a glyph cell `(left, top, right, bottom)` in `[0,1]`,
/// matched to `shape::quad_uv`'s ordering: atlas `[left, bottom, right, top]` texels →
/// `(left/aw, top/ah, right/aw, bottom/ah)`. `top` (the smaller texel-Y) maps to the
/// quad corner v=0; `bottom` to v=1 (image-top-origin texels).
fn cell_uv(g: &GlyphMetrics) -> [f32; 4] {
    let aw = ATLAS_W as f32;
    let ah = ATLAS_H as f32;
    [
        g.atlas[0] / aw, // left
        g.atlas[3] / ah, // top (smaller texel-Y → v=0)
        g.atlas[2] / aw, // right
        g.atlas[1] / ah, // bottom (larger texel-Y → v=1)
    ]
}

/// One glyph quad at logical-px `(x, y, w, h)` sampling atlas cell `uv` with `color`
/// — the SAME `pack_ui_instance` text lane (`text_uv: Some(uv)`) the emitter feeds.
fn glyph_quad(x: f32, y: f32, w: f32, h: f32, color: u32, uv: [f32; 4]) -> UiInstance {
    pack_ui_instance(
        &PackInput {
            rect: [x, y, w, h],
            color,
            border_color: 0,
            corner_radius: [0.0; 4],
            border_width: [0.0; 4],
            clip: None,
            text_uv: Some(uv),
        },
        1.0,
    )
}

/// Renders the two-glyph text scene through the real `RhiContext` UI capability + the
/// `record_ui_rects` recorder (the live present recorder) into an offscreen target,
/// returning the readback.
fn render_text_golden(rhi: &mut RhiContext) -> Vec<u8> {
    let font = authored_font();
    let uv_a = cell_uv(&font.glyphs[0]);
    let uv_b = cell_uv(&font.glyphs[1]);

    // SETUP: build the UI pipeline + rings + upload the authored MTSDF atlas.
    rhi.ui_setup(
        Format::R8G8B8A8Unorm,
        boyko_render::ui_rect_vs_spirv(),
        boyko_render::ui_rect_fs_spirv(),
        4,
        &font,
    )
    .expect("ui_setup (UI pipeline + atlas upload + per-FIF rings)");

    // Two glyph quads via the text lane:
    //   - glyph A at (8,8) 16×16  → solid interior; centre (16,16) is fg.
    //   - glyph B at (40,8) 16×16 → top half median-high (fg), bottom half exterior (bg).
    // B's UV-v grows downward: screen y∈[8,16) samples atlas v∈[0,0.5) (the median-high
    // band), y∈[16,24) samples v∈[0.5,1) (the exterior band).
    let instances = [
        glyph_quad(8.0, 8.0, 16.0, 16.0, FG, uv_a),
        glyph_quad(40.0, 8.0, 16.0, 16.0, FG, uv_b),
    ];
    let ortho = UiOrtho::for_extent(WIDTH, HEIGHT);
    // SAFETY: the per-FIF rings were just created by `ui_setup`; nothing was ever
    // submitted against them, so slot 0 is free to host-write unfenced.
    let token = unsafe { boyko_rhi_vulkan::swapchain::FrameWriteToken::forge_unfenced(0) };
    let plan = rhi
        .ui_upload(&instances, ortho, &token)
        .expect("ui_upload (memcpy into the FIF ring + POD UiFramePlan)");
    assert_eq!(plan.instance_count, 2, "two glyph instances uploaded");

    let (pipeline, bind_group) = rhi
        .ui_handles(plan.frame_index)
        .expect("ui_handles after ui_setup");

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
    let full = RenderArea {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
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

    // CLEAR pass: paint the opaque background, then close it so the UI pass opens its
    // own LoadOp::Load scope (the Decision-9 contract).
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

    // UI pass: a FRESH LoadOp::Load full-extent scope, then the shared recorder records
    // the one instanced draw (text + rects share the SAME draw — Decision T4-G).
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
    // `color_formats[0]`, at `full`; `pipeline`/`bind_group` are the live current-frame
    // (MF-7) UI handles whose ring holds `plan.instance_count` valid records uploaded
    // for `plan.frame_index` above; the pipeline declares the UI bind-group layout
    // (binding 0 SSBO, binding 1 atlas, binding 2 UBO) and a 16-byte VERTEX push range.
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

    let dst_ptr = device
        .buffer_mapped_ptr(&staging)
        .expect("host-visible staging buffer is mapped");
    let mut out = vec![0u8; SIZE as usize];
    // SAFETY: `dst_ptr` points to `SIZE` mapped host-coherent bytes; the fence wait
    // above ordered this read after the draw + copy completed; `out` is a distinct,
    // non-overlapping allocation of `SIZE` bytes.
    unsafe {
        core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), SIZE as usize);
    }

    // Teardown the transient offscreen resources (the fence ordered their last GPU use).
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
fn ui_text_renders_msdf_glyphs_through_the_full_render_path_golden() {
    let Some(ctx) = boot_or_skip("ui_text_renders_msdf_glyphs_through_the_full_render_path_golden")
    else {
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

    let mut rhi = RhiContext::new(ctx);
    let out = render_text_golden(&mut rhi);
    let at = |x: u32, y: u32| -> [u8; 4] {
        let b = texel_base(x, y);
        [out[b], out[b + 1], out[b + 2], out[b + 3]]
    };

    // G-T4.1 crisp glyph: glyph A's interior centre (16,16) == foreground (median 1 ⇒
    // coverage 1 ⇒ premultiplied fg over the opaque background).
    assert_eq!(
        at(16, 16),
        FG_BYTES,
        "G-T4.1: glyph A interior must be foreground (MSDF median ≥ 0.5 ⇒ full coverage): got {:02x?}",
        at(16, 16)
    );

    // G-T4.1 no-bleed: a texel well outside glyph A's quad (x∈[8,24), y∈[8,24)) keeps
    // the CLEAR background — the glyph quad placed by the SSBO min/size did not bleed.
    assert_eq!(
        at(2, 2),
        CLEAR_BYTES,
        "G-T4.1: a texel outside every glyph quad must keep the CLEAR background: got {:02x?}",
        at(2, 2)
    );

    // G-T4.2 median preserves a feature vs an SDF control: glyph B's TOP band (screen
    // y∈[8,16) ⇒ atlas v∈[0,0.5)) samples the authored median-high band where
    // median(0.9,0.9,0.1)=0.9 (≥0.5, KEPT) but the single-channel control a=0.1 (<0.5,
    // would be ROUNDED OFF). The FS uses the median, so the band renders foreground —
    // the defining MSDF property. (48,11) is centre-x (no cell-edge bilinear bleed),
    // mid-top-band.
    assert_eq!(
        at(48, 11),
        FG_BYTES,
        "G-T4.2: glyph B's median-preserved band must be foreground (median 0.9 ≥ 0.5; \
         a single-channel SDF control a=0.1 would round it off): got {:02x?}",
        at(48, 11)
    );

    // G-T4.4 distinct glyphs: glyph B's BOTTOM band (screen y∈[16,24) ⇒ atlas v∈[0.5,1))
    // is EXTERIOR (median 0), so (48,20) keeps the background — A and B sample different
    // atlas cells and render their OWN footprints (the UV lane + the corner_radius→uv
    // alias offset are correct, not a constant; B's footprint ≠ A's solid interior).
    assert_eq!(
        at(48, 20),
        CLEAR_BYTES,
        "G-T4.4: glyph B's exterior band must keep the background (distinct from A's \
         solid interior — the UV lane selects a distinct atlas cell/region): got {:02x?}",
        at(48, 20)
    );

    // The GPU-half soundness oracle: zero validation messages across the text draw.
    assert_validation_clean(rhi.context());

    rhi.destroy_all();
    drop(rhi);
}
