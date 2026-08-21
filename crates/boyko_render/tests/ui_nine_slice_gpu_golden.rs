//! UI-ADVANCED rung S4 — the NINE-SLICE GPU golden
//! (`docs/UI-PLAN-SPRITES.md` gate G4-3, red mutations M4-b, M4-c1, M4-e).
//!
//! # What this pins, and why it drives the scheduler's own loop
//!
//! The picture here is produced by **`UiUploadSystem::gather_into_staging`** —
//! the in-schedule pack — from an ECS world, and uploaded verbatim from
//! `sys.staged()`. It does NOT hand-pack `UiInstance`s the way the S3 sprite
//! golden does (`ui_sprite_gpu_golden.rs`'s `sprite()` helper): a gate that
//! builds the nine records itself would re-implement the expansion policy and
//! then gate the policy against its own copy of it. The one nine-slice-shaped
//! precedent in the tree is exactly that construction, and this rung's audit
//! rejected it before the code was written.
//!
//! # The scene
//!
//! A 128×128 `R8G8B8A8` offscreen target. One node at logical (16,16) 96×96,
//! carrying:
//!
//! * `UiBackground` — an opaque OLIVE that must end up covering **zero** pixels.
//!   The nine regions tile the node's rect exactly, so any of it in the readback
//!   is a missing slice. The colour collides with no source cell, for a reason
//!   the constant's own doc records.
//! * `UiImage` — the whole of a **3×3 procedural source whose nine texels are
//!   nine DISTINCT colours**, under an **opaque white tint**. Distinct because a
//!   symmetric source makes region assignment unobservable: the natural
//!   corner/edge/centre source is invariant under the full dihedral group, so
//!   all 24 corner permutations would hash identically and M4-e could not fire.
//!   Opaque white because `UiImage`'s default tint is alpha 0 and the pack
//!   premultiplies it into every slice — a defaulted tint would move zero pixels
//!   and disarm both M4-b and M4-e.
//! * `UiNineSlice` — `border_px = [16, 24, 16, 24]`, deliberately ASYMMETRIC:
//!   `[16; 4]` makes `[l, t, r, b]` and `[t, l, b, r]` hash identically, which is
//!   the same class of hidden symmetry one axis over. `border_uv` takes its
//!   equal-thirds `Default`, so the golden authors no new field.
//!
//! `UiSamplerMode::Pixel`. The row's own claim — "each corner samples only its
//! own source cell" — is FALSE under the default `Smooth`: `Smooth` is
//! `Filter::Linear`, and magnifying a 3-texel axis blends into the neighbouring
//! cell past each cell's texel centre. Under `Pixel` it is true as written, and
//! the golden's meaning ("this region came from that cell") stops depending on a
//! filter kernel. Every one of the 96 destination columns maps to a `u` strictly
//! inside its own third, and likewise for the rows, so the image is exactly nine
//! solid rectangles.
//!
//! # CI gate
//!
//! A GPU-less / loader-less / validation-less host makes `VulkanContext::boot`
//! return `Err` and the test skips gracefully (the `ui_rect_gpu_golden`
//! convention). **A skip is not a pass**, and this file says so mechanically:
//! set `BOYKO_UI_GOLDEN_REQUIRE_DEVICE=1` and a skip becomes a failure, so a run
//! that claims to have compared the picture can be made to prove it.

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
    record_ui_rects, ui_render_discovery, BindlessTextureTable, RhiContext, UiOrtho, UiRenderGeneration,
    UiSamplerMode, UiUploadSystem, UI_NINE_SLICE_REGIONS,
};
use boyko_ui::components::{
    ComputedRect, StackIndex, UiBackground, UiImage, UiNineSlice, UiRoot,
};

use common::{assert_ui_golden_image_pin, assert_validation_clean, boot_or_skip};

/// UI-ADVANCED S4 (S-D6): SHA-256 of the full 128×128 RGBA readback in `Pixel`
/// mode. NEW at S4 — the four S2 pins and the S3 sprite pin must NOT move, and
/// they do not: S4 changes what is emitted only for a node carrying
/// `UiNineSlice`, and no other scene in the tree has one.
///
/// Blessed 2026-08-21 on this box (RTX 3060, validation on) and LOOKED AT. TEN
/// distinct colours, every one accounted for by an exact pixel count:
/// the CLEAR ground (7 168 px = 128² − 96²) and the nine source cells at
/// 384 / 1 536 / 384 / 768 / 3 072 / 768 / 384 / 1 536 / 384 px — the outer
/// product of the column widths (16, 64, 16) and the row heights (24, 48, 24).
/// There is NO blend seam and no eleventh colour: `Pixel` is NEAREST, the nine
/// destination rects tile the node's rect on integer pixel boundaries, and every
/// slice is opaque, so the node's own background contributes ZERO pixels.
/// Re-bless: `BOYKO_UI_GOLDEN_BLESS=1`.
const UI_NINE_SLICE_PIXEL_SHA256: &str =
    "b84da469113facd1f13084c184e817d10621b4c692e1c0cb6a70b7c3b724c06e";

const WIDTH: u32 = 128;
const HEIGHT: u32 = 128;
const TEXELS: usize = (WIDTH * HEIGHT) as usize;
const SIZE: u64 = (TEXELS * 4) as u64;

/// The offscreen CLEAR color (the texel an uncovered sample keeps).
const CLEAR_BYTES: [u8; 4] = [0x11, 0x22, 0x33, 0xFF];

/// The node's own background — opaque OLIVE, and it must cover ZERO pixels: the
/// nine regions tile the rect exactly, so background colour in the readback means
/// a slice is missing.
///
/// The colour is chosen to collide with NO source cell, and that is not fussiness:
/// the first spelling of this constant was pure blue, which is byte-identical to
/// [`CELL`]`[2]` (TR). The "zero background pixels" assertion then counted TR's
/// 384 px and reported a missing slice on a picture that was correct — the
/// accounting catching a defect in its own instrument, which is the whole reason
/// this file counts colours instead of trusting the hash.
const BACKGROUND_OLIVE: u32 = 0xFF_00_80_80;
const BACKGROUND_OLIVE_BYTES: [u8; 4] = [0x80, 0x80, 0x00, 0xFF];

/// An opaque WHITE tint — premultiplied it is `(1,1,1,1)`, so the modulate is
/// the identity and every assertion below reads the SOURCE CELL, not the tint.
const TINT_OPAQUE_WHITE: u32 = 0xFF_FF_FF_FF;

/// The node's destination rect, logical px == physical px at scale 1.0.
const NODE: [f32; 4] = [16.0, 16.0, 96.0, 96.0];
/// The destination border, `[l, t, r, b]` — asymmetric on purpose.
const BORDER_PX: [f32; 4] = [16.0, 24.0, 16.0, 24.0];

/// The 3×3 source's nine texels, ROW-MAJOR, all opaque and all distinct.
/// Deliberately not a ramp: no two differ in only one channel, so a swapped pair
/// is a large hash delta as well as a wrong probe.
const CELL: [[u8; 4]; 9] = [
    [0xFF, 0x00, 0x00, 0xFF], // TL — red
    [0x00, 0xFF, 0x00, 0xFF], // T  — green
    [0x00, 0x00, 0xFF, 0xFF], // TR — blue
    [0xFF, 0xFF, 0x00, 0xFF], // L  — yellow
    [0xFF, 0x00, 0xFF, 0xFF], // C  — magenta
    [0x00, 0xFF, 0xFF, 0xFF], // R  — cyan
    [0xFF, 0x80, 0x00, 0xFF], // BL — orange
    [0x80, 0x00, 0xFF, 0xFF], // B  — violet
    [0x40, 0x40, 0x40, 0xFF], // BR — dark grey
];

const REGION: [&str; 9] = ["TL", "T", "TR", "L", "C", "R", "BL", "B", "BR"];

/// The centre texel of each destination region, in target pixels. AUTHORED from
/// the scene: columns span x 16..32 / 32..96 / 96..112 and rows span
/// y 16..40 / 40..88 / 88..112.
const PROBE_CENTRE: [(u32, u32); 9] = [
    (24, 28),
    (64, 28),
    (104, 28),
    (24, 64),
    (64, 64),
    (104, 64),
    (24, 100),
    (64, 100),
    (104, 100),
];

/// The 3×3 source, built in Rust and bit-reproducible (S-D5) — `boyko_image` is
/// a decoder only, so a checked-in PNG could not be regenerated by anything the
/// repo owns.
fn source_3x3() -> Vec<u8> {
    let mut pixels = Vec::with_capacity(9 * 4);
    for c in CELL {
        pixels.extend_from_slice(&c);
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

/// Builds the one-node world and runs Phase 1 of the upload seam against it,
/// returning the system with its staging box filled — the SAME loop the
/// scheduler runs, driven device-free through `EcsMaster::run_system_once`.
fn stage_nine_sliced_node(slot: u32) -> UiUploadSystem {
    let mut world = EcsMaster::new();
    world.insert_resource(UiRenderGeneration::default());
    world.run_system(move |mut cmds: Commands| {
        let mut e = cmds.spawn(ComputedRect {
            x: NODE[0],
            y: NODE[1],
            w: NODE[2],
            h: NODE[3],
        });
        e.insert(UiBackground { color: BACKGROUND_OLIVE, ..UiBackground::default() });
        e.insert(StackIndex(0));
        e.insert(UiImage {
            texture: slot,
            uv_min: [0.0, 0.0],
            uv_max: [1.0, 1.0],
            tint: TINT_OPAQUE_WHITE,
        });
        e.insert(UiNineSlice {
            border_px: BORDER_PX,
            // `border_uv` takes its equal-thirds Default — exactly right for a
            // 3×3 source, and the zero-configuration case an author gets.
            ..UiNineSlice::default()
        });
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

/// Renders the staged records through the real UI capability with the shared
/// bindless table bound at set 1, and returns the readback.
fn render_nine_slice_golden(
    rhi: &mut RhiContext,
    table: &BindlessTextureTable,
    slot: u32,
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

    // --- The records come from the SCHEDULER'S OWN PACK, not from this file. ---
    let sys = stage_nine_sliced_node(slot);
    let instances = sys.staged();
    assert_eq!(
        instances.len(),
        1 + UI_NINE_SLICE_REGIONS as usize,
        "the node stages its background plus every region and NO whole-rect image \
         record (S-D12 (1)) — the count is DERIVED, never a literal"
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
    // the live, current-frame-re-resolved (MF-7) UI handles whose backing ring holds
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

/// The nine region probes + the boundary probes + the accounting.
fn assert_nine_slice_scene(out: &[u8]) {
    // 1. Every region shows its OWN source cell. This is what M4-e permutes.
    for r in 0..9 {
        let (x, y) = PROBE_CENTRE[r];
        assert_eq!(
            texel_at(out, x, y),
            CELL[r],
            "region {} samples its own source cell — a permuted source assignment shows \
             up here as a neighbour's colour",
            REGION[r]
        );
    }

    // 2. The corners are exactly `border_px`, NOT a fraction of the rect. This is
    //    what M4-b breaks: a proportional corner at 96×96 from a 3×3 source is
    //    32×32, which swallows the pixels probed below.
    assert_eq!(
        texel_at(out, 31, 39),
        CELL[0],
        "the LAST pixel of the TL corner (x 16..31, y 16..39) is still TL — the corner \
         is 16×24, the authored `border_px`"
    );
    assert_eq!(
        texel_at(out, 32, 40),
        CELL[4],
        "…and the NEXT pixel is already the CENTRE. A proportional corner would be \
         32×32 and this pixel would still be TL"
    );
    assert_eq!(
        texel_at(out, 96, 88),
        CELL[8],
        "the BR corner starts at (96,88) — `border_px[2]`/`[3]` measured from the far \
         edge, which is where the [l,t,r,b] side order becomes observable"
    );
    assert_eq!(
        texel_at(out, 95, 87),
        CELL[4],
        "…and the pixel before it is still the centre"
    );

    // 3. An uncovered texel keeps the CLEAR colour — genuine per-instance
    //    placement under the LoadOp::Load UI pass, not a full-screen fill.
    assert_eq!(texel_at(out, 4, 4), CLEAR_BYTES, "outside the node: CLEAR");
    assert_eq!(texel_at(out, 120, 120), CLEAR_BYTES, "outside the node: CLEAR");

    // 4. THE ACCOUNTING. Ten colours, each with the pixel count the geometry
    //    predicts, and ZERO pixels of the node's own background — the nine
    //    regions tile the rect exactly and every slice is opaque, so any blue is
    //    a missing slice and any eleventh colour is a blend that `Pixel` must not
    //    produce.
    let widths = [16u32, 64, 16];
    let heights = [24u32, 48, 24];
    let mut counted = 0u32;
    for r in 0..9 {
        let expect = widths[r % 3] * heights[r / 3];
        let mut n = 0u32;
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                if texel_at(out, x, y) == CELL[r] {
                    n += 1;
                }
            }
        }
        assert_eq!(
            n, expect,
            "region {} must cover exactly {expect} px (columns 16/64/16 × rows 24/48/24)",
            REGION[r]
        );
        counted += n;
    }
    let mut clear = 0u32;
    let mut background = 0u32;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            match texel_at(out, x, y) {
                t if t == CLEAR_BYTES => clear += 1,
                t if t == BACKGROUND_OLIVE_BYTES => background += 1,
                _ => {}
            }
        }
    }
    assert_eq!(
        background, 0,
        "the node's own BACKGROUND must be completely covered: the nine regions tile its \
         rect and every slice is opaque"
    );
    assert_eq!(
        clear,
        WIDTH * HEIGHT - NODE[2] as u32 * NODE[3] as u32,
        "the clear ground is the target minus the node's rect, exactly"
    );
    assert_eq!(
        counted + clear,
        WIDTH * HEIGHT,
        "TEN colours and no eleventh: every texel in the readback is either the clear \
         ground or one of the nine source cells (a blend seam would show up here)"
    );
}

/// Boots, builds the table + the 3×3 procedural source, and runs `body` with the
/// registered slot. Returns `false` when the host has no device / no validation
/// layer.
fn with_nine_slice_table(test: &str, body: impl FnOnce(&mut RhiContext, &BindlessTextureTable, u32)) -> bool {
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

    let source = create_rgba_texture(rhi.context(), 3, 3, &source_3x3())
        .expect("the procedural 3x3 nine-cell source uploads (S-D5)");
    let slot = table.register(rhi.context(), source.view());
    assert_ne!(slot, 0, "slot 0 is the reserved magenta error slot and is never issued");

    body(&mut rhi, &table, slot);

    assert_validation_clean(rhi.context());

    // Teardown order: the UI capability first (it BORROWS the table's descriptor
    // set and must stop naming it before the table frees its pool), then the
    // texture the table indexes, then the table, then the device.
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

/// G4-3: slicing preserves corners — the picture the scheduler's own pack
/// produced, with the S-D6 image pin.
#[test]
fn ui_nine_slice_preserves_corners_golden() {
    let ran = with_nine_slice_table(
        "ui_nine_slice_preserves_corners_golden",
        |rhi, table, slot| {
            let out = render_nine_slice_golden(rhi, table, slot, UiSamplerMode::Pixel);
            assert_nine_slice_scene(&out);
            assert_ui_golden_image_pin(
                "ui_nine_slice_gpu_golden",
                &out,
                WIDTH,
                HEIGHT,
                UI_NINE_SLICE_PIXEL_SHA256,
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
