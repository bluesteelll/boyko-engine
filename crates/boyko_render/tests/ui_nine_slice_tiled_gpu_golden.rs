//! UI-ADVANCED rung S5 — the TILED nine-slice GPU goldens
//! (`docs/UI-PLAN-SPRITES.md` gates G5-7, G5-8; red mutation M5-e).
//!
//! # What these pin
//!
//! **G5-7 — `Tile` actually tiles.** G4-3's landed scene verbatim, at
//! `NineSliceMode::Tile`, over a source that can SHOW the difference. Three legs:
//! named probe columns, a blessed hash, and byte-identity of the four CORNER
//! regions against the same scene at `Stretch`.
//!
//! **G5-8 — `Tile` under a sheet stays inside its frame.** The same scene on a
//! node that ALSO carries `UiSpriteSheet`. The instrument is a COLOUR-PALETTE
//! CENSUS, because "which texel was sampled" is not directly observable in a
//! readback: every one of the sheet's sixteen frames gets a palette disjoint from
//! every other frame's, and the assertion is that every non-clear pixel belongs to
//! frame 6's 36.
//!
//! # Why the sources are what they are
//!
//! **G5-7's source is 6×6, and each of its nine 2×2 cells is a 2×2
//! CHECKERBOARD.** A 3×3 source (S4's) cannot distinguish a tile from a stretch at
//! all: each nine-slice region of it is exactly ONE uniform texel under the
//! equal-thirds `border_uv`, and four repeats of a uniform texel are byte-identical
//! to one stretched copy. And a cell painted as two STRIPES arms only one axis: a
//! horizontally-striped cell makes the top-edge probe pair read one value twice
//! under `Tile` as well as under `Stretch`, so that leg passes on a `Tile` that
//! silently fell back. Only an alternation on BOTH axes arms both pairs.
//!
//! **The probes sit in the SECOND repeat, not the first.** In repeat 0 the tiled
//! parameter is `frac(t)` with `t < 1`, where `frac` is the IDENTITY — so a probe
//! there reads the same texel under `Tile` and under M5-e (a UV past `[0,1]`
//! instead of a wrap), and the red would not fire. This is the same trap S-D15
//! found in the mechanism itself, one level down in its own gate.
//!
//! **G5-8's sheet is 24×24 with `inset_uv = (0, 0)`.** NEAREST, so the inset
//! protects against nothing; zero inset makes each frame exactly 6 texels per axis
//! so each nine-slice region is exactly 2×2, which is what makes "every sampled
//! texel lies within that frame's sub-rect" decidable per texel instead of per
//! sub-texel blend.
//!
//! # The limitation this trace does NOT make true, stated rather than gated away
//!
//! Under `UiSamplerMode::Smooth` the hardware's bilinear tap at a TILE SEAM
//! straddles `sub_max → sub_min` and therefore reads one texel outside the
//! sub-rect. `UiSheet::inset_uv` cannot fix it — the inset is on the frame's outer
//! edge and the seam is interior. Both rows here run `Pixel`, where the artifact
//! does not exist; a per-sprite `REPEAT` sampler would fix it and is S7's deferred
//! lever (`flags` bit 4).
//!
//! # CI gate
//!
//! **A skip is not a pass**: `BOYKO_UI_GOLDEN_REQUIRE_DEVICE=1` turns one into a
//! failure. Replicated here rather than inherited (`boot_or_skip` exits 0).

mod common;

use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::system::Commands;
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
    UiRenderGeneration, UiSamplerMode, UiUploadSystem, UI_NINE_SLICE_REGIONS,
};
use boyko_ui::components::{
    ComputedRect, NineSliceMode, StackIndex, UiBackground, UiImage, UiNineSlice, UiRoot,
    UiSpriteSheet,
};
use boyko_ui::sprite::{UiSheet, UiSheetTable};

use common::{assert_ui_golden_image_pin, assert_validation_clean, boot_or_skip};

/// UI-ADVANCED S5 (S-D6): SHA-256 of the full 128×128 RGBA readback of G4-3's
/// scene at `NineSliceMode::Tile`, `UiSamplerMode::Pixel`, over the 6×6
/// checkerboard source.
///
/// Blessed 2026-08-26 on this box (RTX 3060, validation on) and LOOKED AT.
/// NINETEEN distinct colours: the CLEAR ground (7 168 px = 128² − 96²) and the
/// eighteen source values (nine cells × two each), every one of them present. No
/// twentieth colour — `Pixel` is NEAREST and every slice is opaque, so the node's
/// own background contributes ZERO pixels and there is no blend seam.
/// Re-bless: `BOYKO_UI_GOLDEN_BLESS=1`.
const UI_NINE_SLICE_TILED_SHA256: &str =
    "9dc817b60c9ab882e1676086b8c3639b130b6a91578197a7dc69974dbf5ead5a";

/// The same scene on a node ALSO carrying `UiSpriteSheet{ index: 6 }`, over the
/// 24×24 sixteen-frame sheet. Blessed 2026-08-26 and LOOKED AT: the CLEAR ground
/// plus values drawn exclusively from frame 6's own 36 (see the census in
/// [`g5_8_tiling_under_a_sheet_stays_inside_the_frame_golden`]).
const UI_TILED_SHEET_SHA256: &str =
    "766f799717cf3a06063b844418391077e7b1fb3e090eb188363afe871f85338f";

const WIDTH: u32 = 128;
const HEIGHT: u32 = 128;
const TEXELS: usize = (WIDTH * HEIGHT) as usize;
const SIZE: u64 = (TEXELS * 4) as u64;

const CLEAR_BYTES: [u8; 4] = [0x11, 0x22, 0x33, 0xFF];

/// The node's own background — opaque OLIVE, and it must cover ZERO pixels. It
/// collides with no source value in either scene: G5-7's values all have
/// `b == 0x40`, G5-8's all have `b == 0x80`, and this one's is `0x00`. (The S4
/// golden's first background constant WAS byte-identical to one of its source
/// cells, and the "zero background pixels" assertion then counted that cell.)
const BACKGROUND_OLIVE: u32 = 0xFF_00_80_80;
const BACKGROUND_OLIVE_BYTES: [u8; 4] = [0x80, 0x80, 0x00, 0xFF];

const TINT_OPAQUE_WHITE: u32 = 0xFF_FF_FF_FF;

/// G4-3's landed scene, verbatim: the node at logical (16,16) 96×96 …
const NODE: [f32; 4] = [16.0, 16.0, 96.0, 96.0];
/// … with its deliberately ASYMMETRIC destination border `[l, t, r, b]`.
///
/// On these numbers S-D15 (3)'s ratio COMPUTES `tiles = (4, 2)`:
/// `64 * (2/3) / ((1/3) * 32) = 4` and `48 * (2/3) / ((1/3) * 48) = 2`. The gate
/// does not assert the 4; `ui_s5_sprite_sheet`'s G5-11 derives it from the same
/// inputs on the CPU, and the probe columns below are a function of it.
const BORDER_PX: [f32; 4] = [16.0, 24.0, 16.0, 24.0];

// ───────────────────────── G5-7's 6x6 checkerboard ─────────────────────────

/// The 6×6 source's two values for cell `k` (`k = cell_row * 3 + cell_col`), as
/// `(A, B)`. Eighteen mutually distinct opaque values, all with `b == 0x40` so
/// none can collide with the background or the clear ground.
fn cell_values(k: u32) -> ([u8; 4], [u8; 4]) {
    (
        [(0x20 + 0x19 * k) as u8, 0x30, 0x40, 0xFF],
        [(0x28 + 0x19 * k) as u8, 0x90, 0x40, 0xFF],
    )
}

/// The 6×6 source: nine 2×2 cells, each a 2×2 CHECKERBOARD of its own two values.
///
/// The checkerboard is what arms BOTH probe pairs. A cell painted as two
/// horizontal stripes leaves the top-edge pair reading one value twice (it varies
/// only with `y`), and two vertical stripes does the same to the left-edge pair —
/// each of which would pass on a `Tile` that fell back to `Stretch`.
fn source_6x6() -> Vec<u8> {
    let mut px = vec![0u8; 6 * 6 * 4];
    for cell_row in 0..3u32 {
        for cell_col in 0..3u32 {
            let (a, b) = cell_values(cell_row * 3 + cell_col);
            for dy in 0..2u32 {
                for dx in 0..2u32 {
                    let v = if (dx + dy).is_multiple_of(2) { a } else { b };
                    let x = cell_col * 2 + dx;
                    let y = cell_row * 2 + dy;
                    let o = ((y * 6 + x) * 4) as usize;
                    px[o..o + 4].copy_from_slice(&v);
                }
            }
        }
    }
    px
}

/// The value of source texel `(row, col)` of the 6×6 checkerboard — the same
/// arithmetic [`source_6x6`] paints with, so a probe expectation names a TEXEL
/// rather than repeating a colour literal.
fn src_texel(row: u32, col: u32) -> [u8; 4] {
    let (a, b) = cell_values((row / 2) * 3 + (col / 2));
    if ((row % 2) + (col % 2)).is_multiple_of(2) {
        a
    } else {
        b
    }
}

/// G5-7's four named probes, `(x, y, source_row, source_col)`.
///
/// Derived from the scene, NOT read back at bless time:
///
/// * **Top edge** — destination x 32..96 (64 px) at `tiles_x = 4`, so each repeat
///   is 16 px and each of the region's two source texels is 8 px. Repeat 1 spans
///   x 48..64, so `x = 52` is its first source texel (col 2) and `x = 60` its
///   second (col 3). `y = 20` is the region's first source row (row 0: the top
///   band is 24 px over 2 texels, 12 px each). Under `Stretch` both x-probes fall
///   in source col 2, so the pair AGREES. Under M5-e (`t = local_uv * tiles`, no
///   wrap) they land in cols 4 and 5 — outside the region's own source entirely.
/// * **Left edge** — destination y 40..88 (48 px) at `tiles_y = 2`, so each repeat
///   is 24 px and each source texel 12 px. Repeat 1 spans y 64..88, so `y = 70` is
///   source row 2 and `y = 82` is row 3. `x = 20` is the region's first source col
///   (col 0). Under `Stretch` both y-probes fall in source row 3 (the pair
///   AGREES); under M5-e they land in rows 4 and 5.
const PROBES: [(u32, u32, u32, u32); 4] = [
    (52, 20, 0, 2),
    (60, 20, 0, 3),
    (20, 70, 2, 0),
    (20, 82, 3, 0),
];

// ───────────────────────── G5-8's 24x24 sheet ──────────────────────────────

const SHEET_COLS: u16 = 4;
const SHEET_ROWS: u16 = 4;
const SHEET_FRAME_TEXELS: u32 = 6;
const SHEET_ATLAS: u32 = SHEET_COLS as u32 * SHEET_FRAME_TEXELS;
/// The frame the sheet node names.
const SHEET_FRAME: u16 = 6;

/// The 576 mutually distinct sheet values: `(frame, texel)` ↦ a colour whose RED
/// channel identifies the FRAME and whose GREEN identifies the texel within it.
///
/// That structure is what makes the census decidable per pixel AND makes its
/// failure legible: an escaped sample reports which frame it escaped into.
fn sheet_value(frame: u32, texel: u32) -> [u8; 4] {
    [
        (0x10 + 0x0E * frame) as u8,
        (0x04 + 0x07 * texel) as u8,
        0x80,
        0xFF,
    ]
}

/// The 24×24 sheet: 4×4 frames of 6×6 texels, all 576 values distinct.
fn sheet_source() -> Vec<u8> {
    let mut px = vec![0u8; (SHEET_ATLAS * SHEET_ATLAS * 4) as usize];
    for f in 0..(SHEET_COLS as u32 * SHEET_ROWS as u32) {
        let fc = f % SHEET_COLS as u32;
        let fr = f / SHEET_COLS as u32;
        for t in 0..(SHEET_FRAME_TEXELS * SHEET_FRAME_TEXELS) {
            let dx = t % SHEET_FRAME_TEXELS;
            let dy = t / SHEET_FRAME_TEXELS;
            let x = fc * SHEET_FRAME_TEXELS + dx;
            let y = fr * SHEET_FRAME_TEXELS + dy;
            let o = ((y * SHEET_ATLAS + x) * 4) as usize;
            px[o..o + 4].copy_from_slice(&sheet_value(f, t));
        }
    }
    px
}

// ───────────────────────── shared plumbing ─────────────────────────────────

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

/// Builds the one-node world and runs Phase 1 of the upload seam against it — the
/// SAME loop the scheduler runs, driven device-free through `run_system_once`.
///
/// `sheet` is `Some` for G5-8: the node then additionally carries `UiSpriteSheet`,
/// and the gather substitutes the frame's sub-rect into the image inputs BEFORE
/// `border_uv` cuts it — so the nine sub-rects live inside frame 6 and the tile
/// counts are unchanged, because the sub-rect extent cancels out of the ratio.
fn stage(slot: u32, mode: NineSliceMode, sheet: Option<UiSheet>) -> UiUploadSystem {
    let mut world = EcsMaster::new();
    world.insert_resource(UiRenderGeneration::default());
    let has_sheet = sheet.is_some();
    if let Some(s) = sheet {
        let mut table = UiSheetTable::new();
        table.register(s);
        world.insert_resource(table);
    }
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
            texture: slot,
            uv_min: [0.0, 0.0],
            uv_max: [1.0, 1.0],
            tint: TINT_OPAQUE_WHITE,
        });
        e.insert(UiNineSlice {
            border_px: BORDER_PX,
            mode,
            // `border_uv` takes its equal-thirds Default, exactly as G4-3 does —
            // right for a 3-cell axis, and the zero-configuration case an author
            // gets.
            ..UiNineSlice::default()
        });
        if has_sheet {
            e.insert(UiSpriteSheet {
                sheet: 0,
                index: SHEET_FRAME,
            });
        }
        e.insert(UiRoot);
    });

    let pool = ThreadPoolBuilder::new().num_threads(2).build();
    let mut b = ScheduleBuilder::new(pool);
    b.add_system(ui_render_discovery);
    let mut schedule = b.build(&mut world);

    let mut sys = UiUploadSystem::new(1.0);
    let mut settled = 0;
    for _ in 0..8 {
        let before = world.resource::<UiRenderGeneration>().generation;
        schedule.run(&mut world);
        if world.resource::<UiRenderGeneration>().generation == before {
            settled += 1;
            if settled == 2 {
                break;
            }
        } else {
            settled = 0;
        }
    }
    assert_eq!(settled, 2, "discovery must go quiet after the spawn settles");
    world.run_system_once(&mut sys);
    sys
}

/// Renders whatever `sys` staged, at `Pixel`, and returns the readback.
fn render(rhi: &mut RhiContext, table: &BindlessTextureTable, sys: &UiUploadSystem) -> Vec<u8> {
    rhi.ui_setup(
        Format::R8G8B8A8Unorm,
        boyko_render::ui_rect_vs_spirv(),
        boyko_render::ui_rect_fs_spirv(),
        4,
        None,
        UiSamplerMode::Pixel,
        Some(table.set()),
    )
    .expect("ui_setup with NO font and a bindless table");

    let instances = sys.staged();
    assert_eq!(
        instances.len(),
        1 + UI_NINE_SLICE_REGIONS as usize,
        "the node stages its background plus every region and NO whole-rect image record \
         (S-D12 (1)) — the count is DERIVED, never a literal"
    );

    let ortho = UiOrtho::for_extent(WIDTH, HEIGHT);
    // SAFETY: the per-FIF rings were just created by `ui_setup`; nothing was ever
    // submitted against them, so slot 0 is free to host-write unfenced.
    let token = unsafe { boyko_rhi_vulkan::swapchain::FrameWriteToken::forge_unfenced(0) };
    let plan = rhi
        .ui_upload(instances, ortho, &token)
        .expect("ui_upload (memcpy into the current-FIF ring + POD UiFramePlan)");
    assert_eq!(
        plan.instance_count as usize,
        instances.len(),
        "every staged record was uploaded"
    );

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

/// Boots, builds the table and one procedural source, and runs `body`.
fn with_source(
    test: &str,
    extent: u32,
    pixels: Vec<u8>,
    body: impl FnOnce(&mut RhiContext, &BindlessTextureTable, u32),
) -> bool {
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

    let source = create_rgba_texture(rhi.context(), extent, extent, &pixels)
        .expect("the procedural source uploads (S-D5)");
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

/// The four CORNER regions' pixel spans, `(x0, x1, y0, y1)` exclusive-end, from
/// `NODE` and `BORDER_PX` — columns 16 / 64 / 16 and rows 24 / 48 / 24.
const CORNERS: [(u32, u32, u32, u32); 4] = [
    (16, 32, 16, 40),   // TL
    (96, 112, 16, 40),  // TR
    (16, 32, 88, 112),  // BL
    (96, 112, 88, 112), // BR
];
const CORNER_NAME: [&str; 4] = ["TL", "TR", "BL", "BR"];

/// **G5-7** — `Tile` actually tiles.
#[test]
fn g5_7_tile_actually_tiles_golden() {
    let ran = with_source(
        "g5_7_tile_actually_tiles_golden",
        6,
        source_6x6(),
        |rhi, table, slot| {
            let tiled = render(rhi, table, &stage(slot, NineSliceMode::Tile, None));
            let stretched = render(rhi, table, &stage(slot, NineSliceMode::Stretch, None));

            // (1) THE PROBES. Each is a pixel whose value is a function of the
            //     repeat count — the thing this row exists to prove. Both pairs sit
            //     in the SECOND repeat, where `frac` is not the identity.
            for &(x, y, row, col) in &PROBES {
                assert_eq!(
                    texel_at(&tiled, x, y),
                    src_texel(row, col),
                    "TILED probe ({x},{y}) must be source texel ({row},{col}). Under M5-e \
                     (a UV past [0,1] instead of a wrap) this probe leaves the region's own \
                     source entirely"
                );
            }
            // …and the SAME probes under `Stretch`: each PAIR agrees, because one
            // stretched copy puts both members of a pair in the same source texel.
            assert_eq!(
                texel_at(&stretched, PROBES[0].0, PROBES[0].1),
                texel_at(&stretched, PROBES[1].0, PROBES[1].1),
                "the top-edge probe pair AGREES under Stretch — which is what makes their \
                 differing under Tile evidence of tiling rather than of anything else"
            );
            assert_eq!(
                texel_at(&stretched, PROBES[2].0, PROBES[2].1),
                texel_at(&stretched, PROBES[3].0, PROBES[3].1),
                "…and so does the left-edge pair"
            );
            assert_ne!(
                texel_at(&tiled, PROBES[0].0, PROBES[0].1),
                texel_at(&tiled, PROBES[1].0, PROBES[1].1),
                "the top-edge pair DIFFERS under Tile"
            );
            assert_ne!(
                texel_at(&tiled, PROBES[2].0, PROBES[2].1),
                texel_at(&tiled, PROBES[3].0, PROBES[3].1),
                "…and so does the left-edge pair"
            );

            // (2) THE CORNERS. `FLAG_TILED` is set only when a count exceeds 1, and
            //     a corner is always 1x1 — so a corner sub-quad packs BYTE-IDENTICALLY
            //     to its Stretch record and must render identically. This is a
            //     comparison with THIS scene at Stretch, not with G4-3 (whose source
            //     is 3x3).
            for (c, &(x0, x1, y0, y1)) in CORNERS.iter().enumerate() {
                for y in y0..y1 {
                    for x in x0..x1 {
                        assert_eq!(
                            texel_at(&tiled, x, y),
                            texel_at(&stretched, x, y),
                            "corner {} pixel ({x},{y}): a 1x1 region carries no tile bits at \
                             all, so Tile and Stretch must be the same picture there",
                            CORNER_NAME[c]
                        );
                    }
                }
            }

            // (3) THE ACCOUNTING. Nineteen colours: the clear ground and the
            //     eighteen source values, every one present, none missing, and no
            //     twentieth (a blend seam would be one).
            let mut clear = 0u32;
            let mut background = 0u32;
            let mut other = 0u32;
            let mut seen = [0u32; 18];
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let t = texel_at(&tiled, x, y);
                    if t == CLEAR_BYTES {
                        clear += 1;
                        continue;
                    }
                    if t == BACKGROUND_OLIVE_BYTES {
                        background += 1;
                        continue;
                    }
                    let mut hit = false;
                    for k in 0..9u32 {
                        let (a, b) = cell_values(k);
                        if t == a {
                            seen[(k * 2) as usize] += 1;
                            hit = true;
                        } else if t == b {
                            seen[(k * 2 + 1) as usize] += 1;
                            hit = true;
                        }
                    }
                    if !hit {
                        other += 1;
                    }
                }
            }
            assert_eq!(
                background, 0,
                "the node's own BACKGROUND must be completely covered: the nine regions tile \
                 its rect and every slice is opaque"
            );
            assert_eq!(
                other, 0,
                "no twentieth colour: `Pixel` is NEAREST and every slice is opaque, so a \
                 blend seam cannot appear"
            );
            assert_eq!(
                clear,
                WIDTH * HEIGHT - NODE[2] as u32 * NODE[3] as u32,
                "the clear ground is the target minus the node's rect, exactly"
            );
            for (i, n) in seen.iter().enumerate() {
                assert!(
                    *n > 0,
                    "source value {i} of 18 is MISSING from the readback — every cell shows \
                     both of its checkerboard values under Tile as well as under Stretch"
                );
            }
            assert_eq!(
                seen.iter().sum::<u32>() + clear,
                WIDTH * HEIGHT,
                "every texel in the readback is either the clear ground or one of the \
                 eighteen source values"
            );

            assert_ui_golden_image_pin(
                "ui_nine_slice_tiled",
                &tiled,
                WIDTH,
                HEIGHT,
                UI_NINE_SLICE_TILED_SHA256,
            );
            assert_ne!(
                tiled, stretched,
                "Tile and Stretch must render DIFFERENT pictures — the assertion that says \
                 so without a hash, and the one a `Tile` that fell back to `Stretch` fails"
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

/// **G5-8** — `Tile` under a sheet stays inside its frame.
///
/// The assertion S-D7 could not make, because it FORBADE the combination instead:
/// `frac`-in-sub-rect makes it true by construction, and this row is what proves
/// the construction.
#[test]
fn g5_8_tiling_under_a_sheet_stays_inside_the_frame_golden() {
    let ran = with_source(
        "g5_8_tiling_under_a_sheet_stays_inside_the_frame_golden",
        SHEET_ATLAS,
        sheet_source(),
        |rhi, table, slot| {
            let sheet = UiSheet {
                slot,
                cols: SHEET_COLS,
                rows: SHEET_ROWS,
                frame_count: SHEET_COLS * SHEET_ROWS,
                _pad: [0; 2],
                // ZERO inset: NEAREST has no tap to bleed, and a zero inset makes
                // each frame exactly 6 texels per axis so each nine-slice region is
                // exactly 2x2 — which is what makes "every sampled texel" decidable
                // per texel instead of per sub-texel blend.
                inset_uv: [0.0, 0.0],
            };
            let out = render(rhi, table, &stage(slot, NineSliceMode::Tile, Some(sheet)));

            // THE CENSUS. Every non-clear pixel must be one of frame 6's own 36
            // values. The red channel identifies the frame, so an escaped sample
            // reports WHICH neighbour it escaped into rather than merely being
            // different.
            let frame = SHEET_FRAME as u32;
            let mut clear = 0u32;
            let mut background = 0u32;
            let mut mine = 0u32;
            let mut escaped: Vec<(u32, u32, u32)> = Vec::new();
            let mut seen = [0u32; 36];
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let t = texel_at(&out, x, y);
                    if t == CLEAR_BYTES {
                        clear += 1;
                        continue;
                    }
                    if t == BACKGROUND_OLIVE_BYTES {
                        background += 1;
                        continue;
                    }
                    let mut hit = None;
                    'search: for f in 0..(SHEET_COLS as u32 * SHEET_ROWS as u32) {
                        for i in 0..(SHEET_FRAME_TEXELS * SHEET_FRAME_TEXELS) {
                            if t == sheet_value(f, i) {
                                hit = Some((f, i));
                                break 'search;
                            }
                        }
                    }
                    match hit {
                        Some((f, i)) if f == frame => {
                            mine += 1;
                            seen[i as usize] += 1;
                        }
                        Some((f, _)) => {
                            if escaped.len() < 8 {
                                escaped.push((x, y, f));
                            }
                        }
                        None => {
                            if escaped.len() < 8 {
                                escaped.push((x, y, u32::MAX));
                            }
                        }
                    }
                }
            }
            assert!(
                escaped.is_empty(),
                "a sampled texel ESCAPED frame {frame}. `frac` wraps the quad PARAMETER \
                 inside the record's own sub-rect, so no repeat count can leave the frame; \
                 under M5-e the top edge's UV sweeps four frame-widths past `sub_min` and \
                 lands in the neighbours. First offenders (x, y, frame; {} = no palette \
                 match at all): {escaped:?}",
                u32::MAX
            );
            assert_eq!(
                background, 0,
                "the node's own BACKGROUND must be completely covered"
            );
            assert_eq!(
                clear,
                WIDTH * HEIGHT - NODE[2] as u32 * NODE[3] as u32,
                "the clear ground is the target minus the node's rect, exactly"
            );
            assert_eq!(
                mine + clear,
                WIDTH * HEIGHT,
                "every texel is the clear ground or one of frame {frame}'s 36"
            );
            // Not merely "inside the frame": the tiling must actually SHOW the
            // frame. A degenerate picture that painted one texel everywhere would
            // satisfy the census above and nothing else.
            let distinct = seen.iter().filter(|n| **n > 0).count();
            assert_eq!(
                distinct,
                (SHEET_FRAME_TEXELS * SHEET_FRAME_TEXELS) as usize,
                "ALL 36 of frame {frame}'s texels must appear — MEASURED at bless time, not \
                 a floor: each of the nine regions is exactly 2x2 texels of the frame and \
                 the tiling repeats every one of them. A picture that showed one texel \
                 everywhere would pass the containment census while proving nothing"
            );

            assert_ui_golden_image_pin(
                "ui_tiled_sheet",
                &out,
                WIDTH,
                HEIGHT,
                UI_TILED_SHEET_SHA256,
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
