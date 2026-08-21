//! UI-ADVANCED rung S3 — the SPRITE GPU golden (`docs/UI-PLAN-SPRITES.md` gates G3-1,
//! G3-3, G3-5's device half, and the offscreen half of G3-6).
//!
//! This is the test that makes the textured lane LIVE end to end on a real device:
//! `BindlessTextureTable` → `create_rgba_texture` (S-D5's procedural source) →
//! `register` (the slot) → `pack_ui_image_instance` (the slot into `flags` bits 20..31) →
//! `ui_setup(font: None, bindless: Some(..))` → `ui_upload` → `record_ui_rects` (which
//! binds set 1 through the generic `bind_descriptor_set_at` verb S-D3 added) → readback.
//!
//! # The scene + the proof
//!
//! Two SPRITE quads into a 64×64 `R8G8B8A8` offscreen target, on TWO DISTINCT bindless
//! slots — distinct on purpose, because one slot proves nothing about the field that
//! selects it:
//!
//! * slot A at (8,8) 16×16 — an 8×8 procedural CHECKERBOARD: a 2×2 arrangement of 4×4
//!   blocks, light / dark / dark / light. Four decisive texels, one per block, sampled
//!   well inside it: the two light ones and the two dark ones. A shader that dropped the
//!   UV lerp, flipped v, or sampled a different texture cannot reproduce all four.
//! * slot B at (40,40) 16×16 — a solid GREEN texture. Its interior is the assertion that
//!   the SLOT FIELD, not the draw order, chooses the texture (red mutation M3-a writes
//!   the slot into bits 16..27, which makes the shader read slot 0 — the table's reserved
//!   MAGENTA error texture — for both quads).
//!
//! plus: an uncovered texel keeps the CLEAR color, the validation messenger reports ZERO
//! messages, and the whole readback carries an S-D6 SHA-256 image pin.
//!
//! # Both sampler modes (S-D4)
//!
//! The four probe texels land at UVs whose LINEAR footprint lies wholly inside one
//! checker block, so `Smooth` and `Pixel` must produce the SAME four values — which is
//! what lets one scene gate both modes and prove `Pixel` (NEAREST) is actually reachable.
//! It is reachable only because the UI owns its sprite sampler instead of inheriting the
//! bindless set's immutable trilinear/anisotropic/REPEAT one (S-D4's whole reason).
//!
//! # `font: None` is not incidental
//!
//! This golden passes NO font (gate G3-3): a sprite-only UI must boot. Before S3
//! `ui_setup` demanded a `&BakedFont` unconditionally and this test could not have been
//! written. Red mutation M3-d removes D8e's default fill and this boot fails.
//!
//! # CI gate (graceful skip)
//!
//! A GPU-less / loader-less / validation-less host makes `VulkanContext::boot` return
//! `Err`; the test skips gracefully (the `ui_rect_gpu_golden` convention).

mod common;

use boyko_rhi::{
    BarrierAccess, BarrierStage, BufferDesc, BufferImageCopy, BufferUsage, Format, ImageAspect,
    ImageBarrierDesc, ImageLayout, ImageSubresourceRange, ImageUsage, LoadOp, MemoryLocation,
    RenderArea, RenderingAttachment, RenderingDesc, RhiCommandEncoder, RhiDevice, RhiQueue,
    StoreOp, TextureDesc, TextureDimension,
};
use boyko_render::bindless::create_rgba_texture;
use boyko_render::{
    pack_ui_image_instance, record_ui_rects, BindlessTextureTable, PackInput, RhiContext,
    UiImageInput, UiInstance, UiOrtho, UiSamplerMode,
};

use common::{assert_ui_golden_image_pin, assert_validation_clean, boot_or_skip};

/// UI-ADVANCED S3 (S-D6): SHA-256 of the full 64×64 RGBA readback in `Smooth` mode. This
/// golden is NEW at S3, so its hash is blessed here for the first time — unlike the four
/// S2 pins, which must NOT move (gate G3-2). Re-bless: `BOYKO_UI_GOLDEN_BLESS=1`.
///
/// Blessed 2026-08-21 on this box (RTX 3060, validation on) and LOOKED AT: eight distinct
/// colors, all accounted for — the CLEAR ground (3584 px), the green quad (256 px), the
/// checker's light and dark blocks (98 px each), and a ONE-PIXEL blend seam along each
/// block boundary (199/88, and 171/116 at the centre cross where both axes blend). That
/// seam is `Smooth`'s LINEAR filter doing exactly what it should at `uv == 0.5`, which
/// lands on the boundary between texels 3 and 4; the sibling `Pixel` test has a hard edge
/// there instead, which is why the two modes share texel probes but not an image pin.
const UI_SPRITE_SMOOTH_SHA256: &str =
    "7dd1b855628d29965f770d1226877afd2af5fb12e7a73b49a8b20b9dd3fad768";

const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;
const TEXELS: usize = (WIDTH * HEIGHT) as usize;
const SIZE: u64 = (TEXELS * 4) as u64;

/// The offscreen CLEAR color (the texel an uncovered sample keeps).
const CLEAR_BYTES: [u8; 4] = [0x11, 0x22, 0x33, 0xFF];
/// The checkerboard's LIGHT block — opaque white, so the premultiply is the identity and
/// the readback texel equals the source texel exactly.
const LIGHT: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
/// The checkerboard's DARK block.
const DARK: [u8; 4] = [0x20, 0x20, 0x20, 0xFF];
/// The second sprite's solid fill (opaque GREEN).
const GREEN: [u8; 4] = [0x00, 0xFF, 0x00, 0xFF];
/// An opaque WHITE tint — premultiplied it is `(1,1,1,1)`, so the modulate is the
/// identity and every assertion below reads the TEXTURE, not the tint.
const TINT_OPAQUE_WHITE: u32 = 0xFF_FF_FF_FF;

fn floats(bytes: [u8; 4]) -> [f32; 4] {
    [
        bytes[0] as f32 / 255.0,
        bytes[1] as f32 / 255.0,
        bytes[2] as f32 / 255.0,
        bytes[3] as f32 / 255.0,
    ]
}

fn texel_base(x: u32, y: u32) -> usize {
    ((y * WIDTH + x) * 4) as usize
}

fn texel_at(out: &[u8], x: u32, y: u32) -> [u8; 4] {
    let b = texel_base(x, y);
    [out[b], out[b + 1], out[b + 2], out[b + 3]]
}

/// S-D5: the procedural 8×8 checkerboard — a 2×2 arrangement of 4×4 blocks, built in
/// Rust and bit-reproducible, so the image pin has a referent nothing outside this repo
/// can move. `boyko_image` is a decoder only, so a checked-in PNG could not be
/// regenerated by anything the repo owns.
fn checkerboard_8x8() -> Vec<u8> {
    let mut pixels = Vec::with_capacity(8 * 8 * 4);
    for y in 0..8u32 {
        for x in 0..8u32 {
            let block_light = ((x / 4) + (y / 4)) % 2 == 0;
            pixels.extend_from_slice(if block_light { &LIGHT } else { &DARK });
        }
    }
    pixels
}

/// One SPRITE record covering `(x, y, w, h)` (logical px == physical px at scale 1.0),
/// sampling the whole of bindless `slot` under an opaque white tint.
fn sprite(x: f32, y: f32, w: f32, h: f32, slot: u32) -> UiInstance {
    pack_ui_image_instance(
        &PackInput {
            rect: [x, y, w, h],
            color: 0,
            border_color: 0,
            corner_radius: [0.0; 4],
            border_width: [0.0; 4],
            clip: None,
            text_uv: None,
            image: Some(UiImageInput {
                slot,
                uv: [0.0, 0.0, 1.0, 1.0],
                tint: TINT_OPAQUE_WHITE,
            }),
            nine_slice: None,
        },
        1.0,
    )
    .expect("a PackInput carrying an image emits a sprite record")
}

/// Renders the two-sprite scene through the real UI capability with the shared bindless
/// table bound at set 1, and returns the readback.
fn render_sprite_golden(
    rhi: &mut RhiContext,
    table: &BindlessTextureTable,
    slot_checker: u32,
    slot_green: u32,
    mode: UiSamplerMode,
) -> Vec<u8> {
    // --- 1. SETUP: NO font (G3-3) + the shared bindless table at set 1. ---
    rhi.ui_setup(
        Format::R8G8B8A8Unorm,
        boyko_render::ui_rect_vs_spirv(),
        boyko_render::ui_rect_fs_spirv(),
        4,
        None,
        mode,
        Some(table.set()),
    )
    .expect("ui_setup with NO font and a bindless table (G3-3: a sprite-only UI boots)");

    // --- 2. PACK + UPLOAD: two sprite records on two distinct slots. ---
    let instances = [
        sprite(8.0, 8.0, 16.0, 16.0, slot_checker),
        sprite(40.0, 40.0, 16.0, 16.0, slot_green),
    ];
    let ortho = UiOrtho::for_extent(WIDTH, HEIGHT);
    // SAFETY: the per-FIF rings were just created by `ui_setup`; nothing was ever
    // submitted against them, so slot 0 is free to host-write unfenced.
    let token = unsafe { boyko_rhi_vulkan::swapchain::FrameWriteToken::forge_unfenced(0) };
    let plan = rhi
        .ui_upload(&instances, ortho, &token)
        .expect("ui_upload (memcpy into the current-FIF ring + POD UiFramePlan)");
    assert_eq!(plan.instance_count, 2, "two sprite instances uploaded");

    // --- 3. RE-RESOLVE the current-frame handles (MF-7) + the set-1 sprite group. ---
    let (pipeline, bind_group) = rhi
        .ui_handles(plan.frame_index)
        .expect("ui_handles after ui_setup");
    let sprite_group = rhi
        .ui_sprite_group()
        .expect("ui_sprite_group after ui_setup — set 1 is never absent once setup ran");

    // --- 4. RECORD into an offscreen target, then read back. ---
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
            mip_levels: 1,
            view_format: None,
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
    encoder.begin_rendering(&RenderingDesc {
        render_area: full,
        colors: &clear_attachment,
        depth: None,
    });
    encoder.end_rendering();

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
    // `color_formats[0]` (passed to `ui_setup`), at `full`; `pipeline`/`bind_group` are
    // the live, current-frame-re-resolved (MF-7) UI handles whose backing ring holds
    // `plan.instance_count` valid records uploaded for `plan.frame_index` above;
    // `sprite_group` is the set-1 group that same `ui_setup` built the pipeline layout's
    // index 1 from, and the textures it indexes are `table`'s, alive for this whole call.
    unsafe {
        record_ui_rects(&mut encoder, &full, &plan, pipeline, bind_group, sprite_group);
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
    // SAFETY: `dst_ptr` points to `SIZE` mapped host-coherent bytes; the fence wait above
    // ordered this read after the draw + copy completed; reading `SIZE` bytes is
    // in-bounds; `out` is a distinct, non-overlapping allocation.
    unsafe {
        core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), out.as_mut_ptr(), SIZE as usize);
    }

    // SAFETY: each transient was created on `device`, its GPU work completed
    // (fence-waited), and each is destroyed exactly once here.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_buffer(staging);
        device.destroy_texture(output);
    }
    out
}

/// The four checker probes + the second sprite's interior + an uncovered texel. Shared by
/// both sampler modes: the probe UVs' LINEAR footprints lie wholly inside one block, so
/// `Smooth` and `Pixel` MUST agree here — and a mode that silently fell back to the other
/// would still be caught by the image pin, which does not agree.
fn assert_sprite_scene(out: &[u8], mode: UiSamplerMode) {
    // The checker quad spans px (8..24, 8..24); uv = (px - 8) / 16; texel = uv * 8.
    // Probe one texel well inside each 4×4 block.
    assert_eq!(
        texel_at(out, 10, 10),
        LIGHT,
        "{mode:?}: the checker's top-left block is LIGHT (the sprite sampled its texture \
         through the UV lerp of the quad corner)"
    );
    assert_eq!(
        texel_at(out, 20, 10),
        DARK,
        "{mode:?}: the checker's top-RIGHT block is DARK — u actually varies across the quad"
    );
    assert_eq!(
        texel_at(out, 10, 20),
        DARK,
        "{mode:?}: the checker's BOTTOM-left block is DARK — v actually varies, and in the \
         top-left-origin direction (a flipped v swaps this with the probe above)"
    );
    assert_eq!(
        texel_at(out, 20, 20),
        LIGHT,
        "{mode:?}: the checker's bottom-right block is LIGHT"
    );
    assert_eq!(
        texel_at(out, 48, 48),
        GREEN,
        "{mode:?}: the SECOND sprite samples its OWN slot — the slot field in flags bits \
         20..31 selects the texture (M3-a writes it into bits 16..27 and both quads go \
         MAGENTA, the table's reserved error texture at slot 0)"
    );
    assert_eq!(
        texel_at(out, 60, 4),
        CLEAR_BYTES,
        "{mode:?}: an uncovered texel keeps the CLEAR color (genuine per-instance \
         placement under the LoadOp::Load UI pass, not a full-screen fill)"
    );
}

/// Boots, builds the table + the two procedural textures, and runs `body` with the two
/// registered slots. Returns `false` when the host has no device / no validation layer.
fn with_sprite_table(test: &str, body: impl FnOnce(&mut RhiContext, &BindlessTextureTable, u32, u32)) -> bool {
    let Some(ctx) = boot_or_skip(test) else {
        return false;
    };
    println!("Vulkan device (validation on): {}", ctx.device_name());
    if !ctx.validation_enabled() {
        assert!(
            std::env::var_os("BOYKO_DISABLE_VALIDATION").is_some(),
            "validation must be active when enable_validation is set and the escape hatch is absent"
        );
        eprintln!("SKIP {test}: validation disabled (BOYKO_DISABLE_VALIDATION)");
        return false;
    }

    let mut rhi = RhiContext::new(ctx);
    let mut table =
        BindlessTextureTable::new(rhi.context()).expect("the bindless texture table boots");

    let checker = create_rgba_texture(rhi.context(), 8, 8, &checkerboard_8x8())
        .expect("the procedural 8x8 checkerboard uploads (S-D5)");
    let green = create_rgba_texture(rhi.context(), 2, 2, &GREEN.repeat(4))
        .expect("the solid green source uploads");
    let slot_checker = table.register(rhi.context(), checker.view());
    let slot_green = table.register(rhi.context(), green.view());
    assert_ne!(
        slot_checker, slot_green,
        "the two sprites must sit on DISTINCT slots, or the slot field proves nothing"
    );
    assert!(
        slot_checker != 0 && slot_green != 0,
        "slot 0 is the reserved magenta error slot and is never issued"
    );

    body(&mut rhi, &table, slot_checker, slot_green);

    assert_validation_clean(rhi.context());

    // Teardown order: the UI capability first (it BORROWS the table's descriptor set and
    // must stop naming it before the table frees its pool), then the textures the table
    // indexes, then the table, then the device.
    rhi.destroy_all();
    let device = rhi.context();
    let _ = device.wait_idle();
    // SAFETY: both textures were created on `device` by `create_rgba_texture`, the device
    // is drained, and each is moved by value ⇒ destroyed exactly once.
    unsafe {
        device.destroy_texture(checker);
        device.destroy_texture(green);
    }
    table.destroy(rhi.context());
    drop(rhi);
    true
}

/// G3-1 + G3-3 + the offscreen half of G3-6: a sprite renders, on a font-less boot,
/// through the generic recorder's set-1 bind — with the S-D6 image pin.
#[test]
fn ui_sprite_renders_through_the_bindless_lane_golden() {
    let ran = with_sprite_table(
        "ui_sprite_renders_through_the_bindless_lane_golden",
        |rhi, table, slot_checker, slot_green| {
            let out = render_sprite_golden(
                rhi,
                table,
                slot_checker,
                slot_green,
                UiSamplerMode::Smooth,
            );
            assert_sprite_scene(&out, UiSamplerMode::Smooth);
            assert_ui_golden_image_pin(
                "ui_sprite_gpu_golden",
                &out,
                WIDTH,
                HEIGHT,
                UI_SPRITE_SMOOTH_SHA256,
            );
        },
    );
    if !ran {
        eprintln!("SKIP: no device / no validation layer");
    }
}

/// S-D4's reason, made a test: `UiSamplerMode::Pixel` is REACHABLE, and it renders the
/// same decisive texels. Under the shared bindless sampler (immutable LINEAR + 16×
/// anisotropic + REPEAT) this mode could not exist at all — which is the cost of D2 that
/// S-D4 pays rather than discovers.
///
/// It carries no image pin: the AA-free probes above are what this mode is being tested
/// for, and pinning a second whole image would double the re-bless surface of every
/// future sprite rung for no extra falsifying power.
#[test]
fn ui_sprite_pixel_sampler_mode_is_reachable_and_samples_the_same_texels() {
    let ran = with_sprite_table(
        "ui_sprite_pixel_sampler_mode_is_reachable_and_samples_the_same_texels",
        |rhi, table, slot_checker, slot_green| {
            let out =
                render_sprite_golden(rhi, table, slot_checker, slot_green, UiSamplerMode::Pixel);
            assert_sprite_scene(&out, UiSamplerMode::Pixel);
        },
    );
    if !ran {
        eprintln!("SKIP: no device / no validation layer");
    }
}
