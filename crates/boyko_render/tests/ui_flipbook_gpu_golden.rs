//! UI-ADVANCED rung S5 — the FLIPBOOK GPU goldens
//! (`docs/UI-PLAN-SPRITES.md` gates G5-5, G5-9; red mutations M5-b, M5-f, M5-g).
//!
//! # What these pin, and why the picture comes from the scheduler
//!
//! Both pictures are produced by the real loop: a `Schedule` running
//! `ui_sprite_flipbook` ahead of `ui_render_discovery`, then
//! `UiUploadSystem`'s own dispatch — including the D6a per-slot generation gate.
//! Nothing here hand-packs a `UiInstance` or hand-computes a frame UV.
//!
//! **The upload is dispatched EVERY tick, and that is load-bearing.** The gate is
//! "never seen ⇒ repack", so a harness that ticked N times and dispatched ONCE at
//! the end would repack on that single dispatch and pick up whatever index the
//! component happens to hold — which is the right index even when the repaint
//! signal is broken. M5-g (writing `index` through `&mut`, which does not consult
//! ticks) would then pass. Dispatching per tick means the gate has already seen a
//! generation, so only a BUMP can force a repack, and a broken signal freezes the
//! picture at the first frame — which is exactly the failure M5-g describes.
//!
//! # G5-5 samples TWO tick counts, not one
//!
//! One blessed hash at one tick count cannot separate "it animates" from "it
//! renders a fixed picture that happens to be frame 3": an implementation that
//! ignored `elapsed` and packed a constant `index = 3` would reproduce it exactly.
//! Two tick counts, whose expected frames are the ones `ui_s5_sprite_sheet`'s
//! G5-2 pins for the same mode and fps, close that gap.
//!
//! # The scene
//!
//! A 128×128 `R8G8B8A8` offscreen target. One node at logical (16,16) 96×96
//! carrying `UiImage` (an OPAQUE WHITE tint — `UiImage`'s authored default is
//! alpha 0, and a defaulted tint would hash the clear ground and disarm every red
//! here), `UiSpriteSheet`, `UiSpriteAnim` and `UiSpriteCursor`, against a
//! `UiSheetTable` holding one 16×16 sheet: **4×4 frames of 4×4 texels, sixteen
//! MUTUALLY DISTINCT frames**.
//!
//! 4×4 FRAMES of 4×4 TEXELS, not a 4×4-texel image: on a 4×4-texel source each
//! frame is one texel and the half-texel inset is `0.5/4 = 0.125` against a frame
//! extent of `0.25`, so insetting both sides leaves an extent of exactly ZERO —
//! `frac`, `lerp` and the inset would all be no-ops and M5-b would move no pixel.
//! At 16×16 the inset is `1/32` against `0.25`, leaving `0.1875`.
//!
//! Sixteen distinct frames because a hash cannot see an off-by-one between two
//! identical frames — the S4 lesson (a symmetric source makes the assignment
//! unobservable), one axis over.
//!
//! # CI gate
//!
//! A GPU-less / loader-less / validation-less host skips gracefully. **A skip is
//! not a pass**: set `BOYKO_UI_GOLDEN_REQUIRE_DEVICE=1` and a skip becomes a
//! failure. The guard is replicated here rather than inherited, because
//! `boot_or_skip` exits 0 and `BOYKO_UI_GOLDEN_REQUIRE_DEVICE` is not a shared
//! helper.

mod common;

use std::time::Duration;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::{Schedule, ScheduleBuilder};
use boyko_ecs::ecs::core::system::Commands;
use boyko_ecs::ecs::core::time::time::Time;
use boyko_threadpool::ThreadPoolBuilder;

use boyko_rhi::{
    BarrierAccess, BarrierStage, BufferDesc, BufferImageCopy, BufferUsage, Format, ImageAspect,
    ImageBarrierDesc, ImageLayout, ImageSubresourceRange, ImageUsage, LoadOp, MemoryLocation,
    RenderArea, RenderingAttachment, RenderingDesc, RhiCommandEncoder, RhiDevice, RhiQueue,
    StoreOp, TextureDesc, TextureDimension,
};
use boyko_render::bindless::create_rgba_texture;
use boyko_render::{
    record_ui_rects, ui_render_discovery, BindlessTextureTable, RhiContext, UiOrtho,
    UiRenderGeneration, UiSamplerMode, UiUploadSystem,
};
use boyko_ui::components::{
    ComputedRect, SpriteAnimMode, StackIndex, UiBackground, UiImage, UiRoot, UiSpriteAnim,
    UiSpriteCursor, UiSpriteSheet,
};
use boyko_ui::sprite::{ui_sprite_flipbook, UiSheet, UiSheetTable};

use common::{assert_ui_golden_image_pin, assert_validation_clean, boot_or_skip};

/// UI-ADVANCED S5 (S-D6): SHA-256 of the full 128×128 RGBA readback after THREE
/// flipbook ticks, at `UiSamplerMode::Pixel`.
///
/// Blessed 2026-08-26 on this box (RTX 3060, validation on) and LOOKED AT. TWO
/// distinct colours, each with an exact pixel count: the CLEAR ground
/// (7 168 px = 128² − 96²) and `FRAME_RGB[3]` (9 216 px = 96²). No third colour:
/// `Pixel` is NEAREST, the sprite covers the node's rect exactly, and the source
/// is opaque under an opaque white tint — so the node's own background
/// contributes ZERO pixels and there is no blend seam.
///
/// The colour is frame **3**, which is what `ui_s5_sprite_sheet`'s G5-2 pins for
/// `Forward` at 10 fps after three 100 ms ticks. Re-bless:
/// `BOYKO_UI_GOLDEN_BLESS=1`.
const UI_FLIPBOOK_FRAME3_SHA256: &str =
    "0fd69179e103d53c24bcd5f55fd5b1701e3ca8ee221cfde171ca94257c26d552";

/// The same readback after SEVEN ticks — frame 7. It exists so the row's claim is
/// "it animates" rather than "it renders one picture": an implementation that
/// ignored `elapsed` and packed a constant index would reproduce ONE of these
/// hashes and not both.
const UI_FLIPBOOK_FRAME7_SHA256: &str =
    "c948b9897458a2238a4b3b0d624c6e3400563d39b63e6135e0a079370590a16a";

const WIDTH: u32 = 128;
const HEIGHT: u32 = 128;
const TEXELS: usize = (WIDTH * HEIGHT) as usize;
const SIZE: u64 = (TEXELS * 4) as u64;

/// The offscreen CLEAR color.
const CLEAR_BYTES: [u8; 4] = [0x11, 0x22, 0x33, 0xFF];

/// The node's own background — opaque, and it must cover ZERO pixels: the sprite
/// covers the node's rect exactly and is opaque, so any of this colour in the
/// readback means the sprite record is missing.
///
/// Chosen to collide with NO frame colour, and that is not fussiness: the S4
/// golden's first spelling of its background constant was byte-identical to one
/// of its source cells, and the "zero background pixels" assertion then counted
/// that cell's pixels and reported a defect on a picture that was correct. The
/// green channel below is `0x80` = 128; every frame's green is `0x14 + 0x0E * f`
/// = 20 + 14f, and 128 − 20 = 108 is not a multiple of 14, so no frame can hit it.
///
/// *(The first spelling of this reason said the greens are "odd for even `f`".
/// They are EVEN for every `f` — 20 and 14 are both even — so parity proves
/// nothing here, and `0x80` is even too. The conclusion was right and the
/// argument was not, which is the shape a reader inherits without checking.)*
const BACKGROUND_OLIVE: u32 = 0xFF_00_80_80;
const BACKGROUND_OLIVE_BYTES: [u8; 4] = [0x80, 0x80, 0x00, 0xFF];

/// An opaque WHITE tint — premultiplied it is `(1,1,1,1)`, so the modulate is the
/// identity and every assertion reads the SOURCE, not the tint.
const TINT_OPAQUE_WHITE: u32 = 0xFF_FF_FF_FF;

/// The node's destination rect (logical px == physical px at scale 1.0).
const NODE: [f32; 4] = [16.0, 16.0, 96.0, 96.0];

/// The sheet's grid: 4×4 FRAMES.
const COLS: u16 = 4;
const ROWS: u16 = 4;
/// Each frame is 4×4 TEXELS, so the atlas is 16×16.
const TEXELS_PER_FRAME_AXIS: u32 = 4;
const ATLAS: u32 = COLS as u32 * TEXELS_PER_FRAME_AXIS;
/// A HALF texel of a 16-texel axis, exact in binary FP.
const HALF_TEXEL: f32 = 0.5 / ATLAS as f32;

/// Frame `f`'s colour. Each frame is a SOLID block of its own colour, and the
/// sixteen are mutually distinct by construction (the green channel alone
/// separates them).
fn frame_rgba(f: u32) -> [u8; 4] {
    [
        (0x30 + 0x0B * f) as u8,
        (0x14 + 0x0E * f) as u8,
        (0xC0 - 0x07 * f) as u8,
        0xFF,
    ]
}

/// The 16×16 procedural sheet: sixteen 4×4 solid frames, ROW-MAJOR. Built in Rust
/// and bit-reproducible (S-D5) — `boyko_image` is a decoder only, so a checked-in
/// PNG could not be regenerated by anything the repo owns.
fn sheet_source() -> Vec<u8> {
    let mut pixels = vec![0u8; (ATLAS * ATLAS * 4) as usize];
    for f in 0..(COLS as u32 * ROWS as u32) {
        let col = f % COLS as u32;
        let row = f / COLS as u32;
        let rgba = frame_rgba(f);
        for dy in 0..TEXELS_PER_FRAME_AXIS {
            for dx in 0..TEXELS_PER_FRAME_AXIS {
                let x = col * TEXELS_PER_FRAME_AXIS + dx;
                let y = row * TEXELS_PER_FRAME_AXIS + dy;
                let b = ((y * ATLAS + x) * 4) as usize;
                pixels[b..b + 4].copy_from_slice(&rgba);
            }
        }
    }
    pixels
}

fn floats(bytes: [u8; 4]) -> [f32; 4] {
    [
        bytes[0] as f32 / 255.0,
        bytes[1] as f32 / 255.0,
        bytes[2] as f32 / 255.0,
        bytes[3] as f32 / 255.0,
    ]
}

fn texel_at(out: &[u8], x: u32, y: u32) -> [u8; 4] {
    let b = ((y * WIDTH + x) * 4) as usize;
    [out[b], out[b + 1], out[b + 2], out[b + 3]]
}

/// The sheet this file registers. `inset_uv` is a half texel per axis — its whole
/// stated purpose is bilinear bleed, which G5-9 is the row that can see.
fn sheet(slot: u32, inset: f32) -> UiSheet {
    UiSheet {
        slot,
        cols: COLS,
        rows: ROWS,
        frame_count: COLS * ROWS,
        _pad: [0; 2],
        inset_uv: [inset, inset],
    }
}

/// Builds a world holding the sheet table and one sprite node, optionally
/// ANIMATED, and returns it with a schedule that runs the flipbook AHEAD of
/// discovery.
fn build_world(slot: u32, inset: f32, anim: Option<UiSpriteAnim>, index: u16) -> EcsMaster {
    let mut world = EcsMaster::new();
    world.insert_resource(UiRenderGeneration::default());
    world.insert_resource(Time::default());
    let mut table = UiSheetTable::new();
    table.register(sheet(slot, inset));
    world.insert_resource(table);
    world.run_system(move |mut cmds: Commands| {
        let mut e = cmds.spawn(ComputedRect {
            x: NODE[0],
            y: NODE[1],
            w: NODE[2],
            h: NODE[3],
        });
        e.insert(UiBackground {
            color: BACKGROUND_OLIVE,
            ..UiBackground::default()
        });
        e.insert(StackIndex(0));
        e.insert(UiImage {
            // NOT the sheet's slot and NOT a frame's UV: the sheet overrides both,
            // so a pack that read these through would sample the wrong texture.
            texture: 0,
            uv_min: [0.0, 0.0],
            uv_max: [1.0, 1.0],
            tint: TINT_OPAQUE_WHITE,
        });
        e.insert(UiSpriteSheet { sheet: 0, index });
        if let Some(a) = anim {
            e.insert(a);
            e.insert(CursorBundle {
                c: UiSpriteCursor::default(),
            });
        }
        e.insert(UiRoot);
    });
    world
}

/// A `Bundle` wrapper so `Commands::insert` can take the DENSE cursor (the
/// `dense_d2_routing::T4DenseBundle` idiom).
#[derive(boyko_macros::Bundle)]
struct CursorBundle {
    c: UiSpriteCursor,
}

fn flipbook_schedule(world: &mut EcsMaster) -> Schedule {
    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    let discovery = b.add_system(ui_render_discovery).key();
    b.add_system(ui_sprite_flipbook).before(discovery);
    b.build(world)
}

/// Ticks the world `ticks` times at `dt`, DISPATCHING THE UPLOAD EVERY TICK (see
/// the module doc), and returns the system with its staging box holding whatever
/// the last dispatch left there.
fn run_ticks(world: &mut EcsMaster, ticks: usize, dt: Duration) -> UiUploadSystem {
    let mut schedule = flipbook_schedule(world);
    let mut sys = UiUploadSystem::new(1.0);
    // Settle the spawn first: it is itself a change to every pack input.
    for _ in 0..3 {
        schedule.run(world);
        world.run_system_once(&mut sys);
    }
    for _ in 0..ticks {
        world.resource_mut::<Time>().advance_with(dt);
        schedule.run(world);
        world.run_system_once(&mut sys);
    }
    sys
}

/// Renders whatever `sys` staged and returns the readback.
fn render(
    rhi: &mut RhiContext,
    table: &BindlessTextureTable,
    sys: &UiUploadSystem,
    mode: UiSamplerMode,
) -> Vec<u8> {
    rhi.ui_setup(
        Format::R8G8B8A8Unorm,
        boyko_render::ui_rect_vs_spirv(),
        boyko_render::ui_rect_fs_spirv(),
        4,
        None,
        mode,
        Some(table.set()),
    )
    .expect("ui_setup with NO font and a bindless table");

    let instances = sys.staged();
    assert_eq!(
        instances.len(),
        2,
        "the node stages its background plus ONE image record — `UiSpriteSheet` changes \
         what that record SAMPLES, never how many records exist"
    );

    let ortho = UiOrtho::for_extent(WIDTH, HEIGHT);
    // SAFETY: the per-FIF rings were just created by `ui_setup`; nothing was ever
    // submitted against them, so slot 0 is free to host-write unfenced.
    let token = unsafe { boyko_rhi_vulkan::swapchain::FrameWriteToken::forge_unfenced(0) };
    let plan = rhi
        .ui_upload(instances, ortho, &token)
        .expect("ui_upload (memcpy into the current-FIF ring + POD UiFramePlan)");

    let (pipeline, bind_group) = rhi
        .ui_handles(plan.frame_index)
        .expect("ui_handles after ui_setup");
    let sprite_group = rhi
        .ui_sprite_group()
        .expect("ui_sprite_group after ui_setup — set 1 is never absent once setup ran");

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
    // the live, current-frame-re-resolved UI handles whose backing ring holds
    // `plan.instance_count` valid records uploaded for `plan.frame_index` above;
    // `sprite_group` is the set-1 group that same `ui_setup` built the pipeline layout's
    // index 1 from, and the texture it indexes is `table`'s, alive for this whole call.
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

/// Boots, builds the table + the 16×16 procedural sheet, and runs `body`.
/// Returns `false` when the host has no device / no validation layer.
fn with_sheet(test: &str, body: impl FnOnce(&mut RhiContext, &BindlessTextureTable, u32)) -> bool {
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

    let source = create_rgba_texture(rhi.context(), ATLAS, ATLAS, &sheet_source())
        .expect("the procedural 16x16 sixteen-frame sheet uploads (S-D5)");
    let slot = table.register(rhi.context(), source.view());
    assert_ne!(slot, 0, "slot 0 is the reserved magenta error slot and is never issued");

    body(&mut rhi, &table, slot);

    assert_validation_clean(rhi.context());

    rhi.destroy_all();
    let device = rhi.context();
    let _ = device.wait_idle();
    // SAFETY: the texture was created on `device` by `create_rgba_texture`, the device
    // is drained, and it is moved by value ⇒ destroyed exactly once.
    unsafe {
        device.destroy_texture(source);
    }
    table.destroy(rhi.context());
    drop(rhi);
    true
}

/// Every pixel is the CLEAR ground or frame `f`'s colour — nothing else. Returns
/// the frame-colour pixel count.
fn assert_solid_frame(out: &[u8], f: u32) -> u32 {
    let want = frame_rgba(f);
    let mut frame_px = 0u32;
    let mut clear_px = 0u32;
    let mut background_px = 0u32;
    let mut other_px = 0u32;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            match texel_at(out, x, y) {
                t if t == want => frame_px += 1,
                t if t == CLEAR_BYTES => clear_px += 1,
                t if t == BACKGROUND_OLIVE_BYTES => background_px += 1,
                _ => other_px += 1,
            }
        }
    }
    assert_eq!(
        background_px, 0,
        "the node's own BACKGROUND must be completely covered: the sprite covers the \
         node's rect exactly and is opaque"
    );
    assert_eq!(
        other_px, 0,
        "TWO colours and no third: `Pixel` is NEAREST and every destination column maps \
         strictly inside frame {f}'s own texels, so no blend and no neighbouring frame \
         can appear"
    );
    assert_eq!(
        clear_px,
        WIDTH * HEIGHT - NODE[2] as u32 * NODE[3] as u32,
        "the clear ground is the target minus the node's rect, exactly"
    );
    assert_eq!(
        frame_px,
        NODE[2] as u32 * NODE[3] as u32,
        "frame {f} covers the node's rect, exactly"
    );
    frame_px
}

/// **G5-5** — it animates ON THE GPU, at two tick counts whose frames the CPU
/// gate independently pins.
#[test]
fn g5_5_the_flipbook_animates_on_the_gpu_golden() {
    let ran = with_sheet("g5_5_the_flipbook_animates_on_the_gpu_golden", |rhi, table, slot| {
        const FPS: f32 = 10.0;
        let step = Duration::from_millis(100);
        let anim = UiSpriteAnim {
            first: 0,
            last: 15,
            fps: FPS,
            mode: SpriteAnimMode::Forward,
            repeats: 0,
            _pad: [0; 2],
        };

        // Three ticks ⇒ frame 3 (what G5-2 pins for Forward at 10 fps).
        let mut world = build_world(slot, HALF_TEXEL, Some(anim), 0);
        let sys = run_ticks(&mut world, 3, step);
        let out3 = render(rhi, table, &sys, UiSamplerMode::Pixel);
        assert_solid_frame(&out3, 3);
        assert_ui_golden_image_pin(
            "ui_flipbook_frame3",
            &out3,
            WIDTH,
            HEIGHT,
            UI_FLIPBOOK_FRAME3_SHA256,
        );

        // Seven ticks ⇒ frame 7. A different picture, from the same scene and the
        // same code — which is the difference between "it animates" and "it draws
        // a fixed frame 3".
        let mut world = build_world(slot, HALF_TEXEL, Some(anim), 0);
        let sys = run_ticks(&mut world, 7, step);
        let out7 = render(rhi, table, &sys, UiSamplerMode::Pixel);
        assert_solid_frame(&out7, 7);
        assert_ui_golden_image_pin(
            "ui_flipbook_frame7",
            &out7,
            WIDTH,
            HEIGHT,
            UI_FLIPBOOK_FRAME7_SHA256,
        );

        assert_ne!(
            out3, out7,
            "the two tick counts must render DIFFERENT pictures — an implementation that \
             ignored `elapsed` and packed a constant index would reproduce one hash and \
             not both, and this is the assertion that says so without a hash"
        );
    });
    if !ran {
        eprintln!("SKIP: no device / no validation layer");
        assert!(
            std::env::var_os("BOYKO_UI_GOLDEN_REQUIRE_DEVICE").is_none(),
            "BOYKO_UI_GOLDEN_REQUIRE_DEVICE is set: this run demanded the device leg and \
             the leg SKIPPED. A skip is not a pass — the picture was never compared."
        );
    }
}

/// **G5-9** — `inset_uv` is protecting something, and the instrument is a NAMED
/// PROBE rather than a hash.
///
/// The field's stated purpose is "half-texel inset against bilinear bleed", which
/// is INERT under NEAREST — so it gets its own `Smooth` row, and G5-5 (which runs
/// `Pixel`) cannot fire M5-b's second half.
///
/// # The two probes, and the two values each of them can take
///
/// A STATIC node at frame 6 (`col = 2`, `row = 1`), so the row pins a filter
/// property and not a flipbook one.
///
/// * **Left edge, `(16, 60)`.** The destination spans x 16..112, so the leftmost
///   pixel centre is `local_uv.x = 0.5/96`. WITH the inset, `u = 0.53125 +
///   0.00098 = 0.53223`, i.e. texel `8.516` — between texel 8's centre and texel
///   9's, both of which are frame 6's own (frame 6 owns texels 8..11). The tap
///   therefore reads frame 6's colour EXACTLY. WITHOUT the inset, `u = 0.5 +
///   0.0013 = 0.50130`, i.e. texel `8.021` — which is `0.479` of a texel BELOW
///   texel 8's centre, so the tap is ~48 % texel 7: **frame 5's** last column.
/// * **Top edge, `(60, 16)`.** The same arithmetic on `v`, whose out-of-frame
///   neighbour is **frame 2** (the cell directly above frame 6).
///
/// Both are large, codable differences, and neither is visible to a hash of an
/// 8-bit image at a tolerance a hash could express — which is why this row probes.
#[test]
fn g5_9_the_frame_inset_keeps_the_bilinear_tap_inside_its_frame() {
    let ran = with_sheet(
        "g5_9_the_frame_inset_keeps_the_bilinear_tap_inside_its_frame",
        |rhi, table, slot| {
            const FRAME: u16 = 6;
            let want = frame_rgba(FRAME as u32);
            let neighbour_left = frame_rgba(5);
            let neighbour_above = frame_rgba(2);

            let mut world = build_world(slot, HALF_TEXEL, None, FRAME);
            let sys = run_ticks(&mut world, 0, Duration::ZERO);
            let out = render(rhi, table, &sys, UiSamplerMode::Smooth);

            let left = texel_at(&out, 16, 60);
            let top = texel_at(&out, 60, 16);
            assert_eq!(
                left, want,
                "the LEFT edge probe must be frame {FRAME}'s own colour. Without the \
                 half-texel inset the bilinear tap sits 0.479 of a texel below texel 8's \
                 centre and takes ~48 % of frame 5 ({neighbour_left:?}) — got {left:?}"
            );
            assert_eq!(
                top, want,
                "the TOP edge probe must be frame {FRAME}'s own colour. Without the inset \
                 it takes ~48 % of frame 2 ({neighbour_above:?}) — got {top:?}"
            );

            // The interior is unaffected either way — stated so the row cannot be
            // satisfied by a picture that is uniformly frame 6 for the wrong reason
            // (e.g. a sheet that resolved to nothing and drew the whole atlas).
            assert_eq!(
                texel_at(&out, 64, 64),
                want,
                "the node's interior is frame {FRAME}"
            );
        },
    );
    if !ran {
        eprintln!("SKIP: no device / no validation layer");
        assert!(
            std::env::var_os("BOYKO_UI_GOLDEN_REQUIRE_DEVICE").is_none(),
            "BOYKO_UI_GOLDEN_REQUIRE_DEVICE is set: this run demanded the device leg and \
             the leg SKIPPED. A skip is not a pass — the picture was never compared."
        );
    }
}
