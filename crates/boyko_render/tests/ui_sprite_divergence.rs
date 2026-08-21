//! UI-ADVANCED rung S3 — the DIVERGENT-DESCRIPTOR leg: correctness across many distinct
//! bindless slots, and §10.1's measurement (`docs/UI-PLAN-SPRITES.md`).
//!
//! # Why a separate scene from `ui_sprite_gpu_golden`
//!
//! That golden draws two sprites, far apart, on two slots. Two far-apart quads do not put
//! two different descriptor indices in ONE subgroup, so they cannot exercise the thing
//! risk **SR3** is about: `NonUniformResourceIndex`, the first non-uniform thing in this
//! shader. This scene draws a DENSE grid of 4×4-px quads, so a 32-lane wave straddles
//! several quads and therefore several slots — which is exactly the shape that makes the
//! qualifier load-bearing (red mutation **M3-b**) and the shape §10.1 has to time.
//!
//! **Plan defect recorded here:** M3-b says "run G3-7's 64-slot leg", but the S3 gate
//! table has no G3-7 — it stops at G3-6. The 64-slot leg it means is §10.1's measurement
//! row, which was specified as a MEASUREMENT and not as a gate, so the mutation named a
//! red that the plan never created. This file is that leg, promoted to a gate.
//!
//! # What is asserted, and what is only reported
//!
//! * **Asserted** (the M3-b red vehicle): every one of 256 densely-packed quads sampling
//!   64 distinct slots reads back its OWN slot's color. A dropped
//!   `NonUniformResourceIndex` makes the wave resolve one lane's descriptor for all of
//!   them, so most quads read a neighbour's color.
//! * **Reported, not asserted** (§10.1): GPU-timestamp nanoseconds around the UI pass for
//!   1 / 8 / 64 distinct slots at N ∈ {256, 2048}. No threshold: the point of the number
//!   is whether Model A (a runtime atlas) ever becomes worth reaching for, and that is a
//!   judgement recorded in S7, not a CI gate. A timing assert here would be a flake.

mod common;

use boyko_rhi::enums::TimestampStage;
use boyko_rhi::{
    BarrierAccess, BarrierStage, BufferDesc, BufferImageCopy, BufferUsage, Format, ImageAspect,
    ImageBarrierDesc, ImageLayout, ImageSubresourceRange, ImageUsage, LoadOp, MemoryLocation,
    QueryPoolDesc, RenderArea, RenderingAttachment, RenderingDesc, RhiCommandEncoder, RhiDevice,
    RhiQueue, StoreOp, TextureDesc, TextureDimension,
};
use boyko_render::bindless::create_rgba_texture;
use boyko_render::{
    pack_ui_image_instance, record_ui_rects, BindlessTextureTable, PackInput, RhiContext,
    UiImageInput, UiInstance, UiOrtho, UiSamplerMode,
};
use boyko_rhi_vulkan::texture::VulkanTexture;

use common::{assert_validation_clean, boot_or_skip};

/// The offscreen target. 256×256 holds the whole N=2048 grid without overlap, so the
/// timing legs differ ONLY in how many distinct slots the same quads sample.
const DIM: u32 = 256;
const SIZE: u64 = (DIM as u64) * (DIM as u64) * 4;
/// One quad's edge in px. Small on purpose: a 32-lane wave covers several quads, so the
/// descriptor index genuinely diverges within a wave.
const CELL: u32 = 4;
/// Quads per row.
const COLS: u32 = 32;
/// The distinct textures registered into the table (the widest leg).
const MAX_SLOTS: usize = 64;

const TINT_OPAQUE_WHITE: u32 = 0xFF_FF_FF_FF;

/// How many times the measurement records the UI pass inside ONE timestamp bracket.
///
/// **This constant is the whole reason §10.1 is a measurement rather than noise.** The
/// first run of this leg timed a SINGLE pass and produced 10240 / 13312 / 11264 ns for
/// 1 / 8 / 64 slots — every value an exact multiple of 1024 ns, and 8 slots reading
/// SLOWER than 64, which cannot be true of a divergence cost. The device's timestamp
/// counter advances in a lattice of ~1024 ns, so a single sub-15 µs pass was being
/// measured with a ruler whose smallest mark was a tenth of the thing measured; the
/// "deltas" were two or three lattice steps of scheduling jitter. Repeating the pass
/// inside one bracket multiplies the signal and leaves the lattice where it belongs —
/// far below the difference under test.
const PASS_REPEATS: u32 = 64;

/// Texture `k`'s solid color — distinct per `k` in every channel, opaque (so the
/// premultiply and the white-tint modulate are both the identity and the readback texel
/// equals the source texel exactly).
fn slot_color(k: usize) -> [u8; 4] {
    [
        (k * 4 + 1) as u8,
        (200 - k * 2) as u8,
        (k * 3 + 30) as u8,
        0xFF,
    ]
}

/// Quad `i`'s top-left px in the grid.
fn cell_origin(i: u32) -> (u32, u32) {
    ((i % COLS) * CELL, (i / COLS) * CELL)
}

/// `n` sprite quads in a dense grid, quad `i` sampling `slots[i % slots.len()]`.
fn grid_scene(n: u32, slots: &[u32]) -> Vec<UiInstance> {
    (0..n)
        .map(|i| {
            let (x, y) = cell_origin(i);
            pack_ui_image_instance(
                &PackInput {
                    rect: [x as f32, y as f32, CELL as f32, CELL as f32],
                    color: 0,
                    border_color: 0,
                    corner_radius: [0.0; 4],
                    border_width: [0.0; 4],
                    clip: None,
                    text_uv: None,
                    image: Some(UiImageInput {
                        slot: slots[i as usize % slots.len()],
                        uv: [0.0, 0.0, 1.0, 1.0],
                        tint: TINT_OPAQUE_WHITE,
                    }),
                    nine_slice: None,
                },
                1.0,
            )
            .expect("every row carries an image")
        })
        .collect()
}

/// One rendered leg: the readback plus the GPU-timestamp nanoseconds bracketing the UI
/// pass (`TopOfPipe` before `begin_rendering`, `BottomOfPipe` after `end_rendering`).
struct Leg {
    readback: Vec<u8>,
    gpu_ns: f64,
}

/// Renders `instances` through the real UI capability and returns the readback + the UI
/// pass's GPU duration. `readback_wanted == false` still copies (the copy is outside the
/// timestamp bracket either way) — one code path, so the timed work is identical.
fn render_leg(rhi: &mut RhiContext, instances: &[UiInstance], repeats: u32) -> Leg {
    debug_assert!(repeats > 0, "invariant: a timed bracket records at least one pass");
    let ortho = UiOrtho::for_extent(DIM, DIM);
    // SAFETY: setup-time write — nothing has been submitted against the per-FIF rings, so
    // slot 0 is free to host-write unfenced.
    let token = unsafe { boyko_rhi_vulkan::swapchain::FrameWriteToken::forge_unfenced(0) };
    let plan = rhi.ui_upload(instances, ortho, &token).expect("ui_upload");
    let (pipeline, bind_group) = rhi.ui_handles(plan.frame_index).expect("ui_handles");
    let sprite_group = rhi.ui_sprite_group().expect("ui_sprite_group");

    let device = rhi.context();
    let queue = device.rhi_queue();

    let output = device
        .create_texture(&TextureDesc {
            width: DIM,
            height: DIM,
            depth: 1,
            format: Format::R8G8B8A8Unorm,
            dimension: TextureDimension::D2,
            usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_SRC,
            array_layers: 1,
            mip_levels: 1,
            view_format: None,
        })
        .expect("offscreen output");
    let staging = device
        .create_buffer(&BufferDesc {
            size: SIZE,
            usage: BufferUsage::TRANSFER_DST,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .expect("readback staging");
    let queries = device
        .create_query_pool(&QueryPoolDesc { count: 2 })
        .expect("a 2-query TIMESTAMP pool (one begin/end pair)");
    let fence = device.create_fence(false).expect("fence");
    let mut encoder = device.create_command_encoder().expect("encoder");
    let full = RenderArea {
        x: 0,
        y: 0,
        width: DIM,
        height: DIM,
    };

    encoder.begin().expect("begin");
    // A TIMESTAMP query is UNDEFINED until reset, and the reset must be recorded OUTSIDE
    // any rendering scope (`VUID-vkCmdResetQueryPool-renderpass`).
    encoder.reset_query_pool(&queries, 0, 2);
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
        clear_color: [0.0, 0.0, 0.0, 1.0],
    }];
    encoder.begin_rendering(&RenderingDesc {
        render_area: full,
        colors: &clear_attachment,
        depth: None,
    });
    encoder.end_rendering();

    // ── The timed bracket: the UI pass alone (the clear above is outside it). ──
    encoder.write_timestamp(&queries, TimestampStage::TopOfPipe, 0);
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
    // SAFETY: recording is open inside a `begin_rendering(LoadOp::Load)` scope whose color
    // format matches the pipeline's, at `full`; `pipeline`/`bind_group`/`sprite_group` are
    // the live handles `ui_setup` built and `ui_upload` filled for `plan.frame_index`; every
    // slot the instances name was registered into the caller's live table.
    for _ in 0..repeats {
        unsafe {
            record_ui_rects(&mut encoder, &full, &plan, pipeline, bind_group, sprite_group);
        }
    }
    encoder.end_rendering();
    encoder.write_timestamp(&queries, TimestampStage::BottomOfPipe, 1);

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
        image_extent_w: DIM,
        image_extent_h: DIM,
        image_extent_d: 1,
    }];
    encoder.copy_image_to_buffer(&output, ImageLayout::TransferSrcOptimal, &staging, &regions);
    encoder.end().expect("end");
    queue.submit(&encoder, &fence).expect("submit");
    device.wait_fence(&fence, u64::MAX).expect("wait_fence");

    let mut scratch = [0u64; 2];
    let mut out_ns = [0f64; 1];
    device
        .read_query_pool_ns(&queries, 1, &mut scratch, &mut out_ns)
        .expect("read the timestamp pair");

    let dst_ptr = device
        .buffer_mapped_ptr(&staging)
        .expect("staging is mapped");
    let mut readback = vec![0u8; SIZE as usize];
    // SAFETY: `dst_ptr` points at `SIZE` mapped host-coherent bytes; the fence wait above
    // ordered this read after the draw + copy; `readback` is a distinct allocation.
    unsafe {
        core::ptr::copy_nonoverlapping(dst_ptr.as_ptr(), readback.as_mut_ptr(), SIZE as usize);
    }

    // SAFETY: each transient was created on `device`, its GPU work is fence-waited, and
    // each is moved by value ⇒ destroyed exactly once.
    unsafe {
        device.destroy_command_encoder(encoder);
        device.destroy_fence(fence);
        device.destroy_query_pool(queries);
        device.destroy_buffer(staging);
        device.destroy_texture(output);
    }
    Leg {
        readback,
        gpu_ns: out_ns[0] / repeats as f64,
    }
}

/// Boots, builds the table + `MAX_SLOTS` distinct solid textures, and runs `body` with the
/// issued slots. Returns `false` on a device-less / validation-less host.
fn with_many_slots(test: &str, body: impl FnOnce(&mut RhiContext, &[u32])) -> bool {
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
    let mut table = BindlessTextureTable::new(rhi.context()).expect("bindless table");
    let mut textures: Vec<VulkanTexture> = Vec::with_capacity(MAX_SLOTS);
    let mut slots: Vec<u32> = Vec::with_capacity(MAX_SLOTS);
    for k in 0..MAX_SLOTS {
        let t = create_rgba_texture(rhi.context(), 2, 2, &slot_color(k).repeat(4))
            .expect("procedural solid texture (S-D5)");
        slots.push(table.register(rhi.context(), t.view()));
        textures.push(t);
    }

    rhi.ui_setup(
        Format::R8G8B8A8Unorm,
        boyko_render::ui_rect_vs_spirv(),
        boyko_render::ui_rect_fs_spirv(),
        2048,
        None,
        UiSamplerMode::Smooth,
        Some(table.set()),
    )
    .expect("ui_setup (sprite-only, shared bindless table)");

    body(&mut rhi, &slots);

    assert_validation_clean(rhi.context());

    rhi.destroy_all();
    let device = rhi.context();
    let _ = device.wait_idle();
    for t in textures {
        // SAFETY: created on `device` by `create_rgba_texture`, the device is drained, and
        // each is moved by value ⇒ destroyed exactly once.
        unsafe { device.destroy_texture(t) };
    }
    table.destroy(rhi.context());
    drop(rhi);
    true
}

/// The M3-b red vehicle: 256 dense 4×4 quads over 64 DISTINCT slots, every quad asserted
/// to read back its OWN slot's color.
///
/// A wave covers several of these quads, so the descriptor index is genuinely non-uniform
/// across it. Dropping `NonUniformResourceIndex` from the eDSL leaf makes the compiler
/// free to resolve ONE lane's index for the whole wave, and the quads that did not supply
/// that lane read a neighbour's color — which is what this loop sees, per quad, by name.
#[test]
fn ui_sprite_divergent_slots_each_quad_samples_its_own_texture() {
    let ran = with_many_slots(
        "ui_sprite_divergent_slots_each_quad_samples_its_own_texture",
        |rhi, slots| {
            const N: u32 = 256;
            let leg = render_leg(rhi, &grid_scene(N, slots), 1);
            let mut wrong = 0usize;
            let mut first_wrong = None;
            for i in 0..N {
                let (x, y) = cell_origin(i);
                // A texel well inside the quad (the source is a uniform 2×2, so LINEAR
                // filtering of it is exact everywhere, and CELL == 4 leaves no doubt).
                let b = (((y + 1) * DIM + (x + 1)) * 4) as usize;
                let got = [
                    leg.readback[b],
                    leg.readback[b + 1],
                    leg.readback[b + 2],
                    leg.readback[b + 3],
                ];
                let want = slot_color(i as usize % slots.len());
                if got != want {
                    wrong += 1;
                    first_wrong.get_or_insert((i, got, want));
                }
            }
            assert_eq!(
                wrong,
                0,
                "{wrong} of {N} densely-packed quads sampled the WRONG slot's texture — \
                 the descriptor index diverges within a wave here, so this is what \
                 `NonUniformResourceIndex` is load-bearing for (SR3, red mutation M3-b). \
                 First: {first_wrong:?}"
            );
            println!(
                "M3-b vehicle: {N} quads x {} distinct slots, all correct; UI pass {:.1} us",
                slots.len(),
                leg.gpu_ns / 1000.0
            );
        },
    );
    if !ran {
        eprintln!("SKIP: no device / no validation layer");
    }
}

/// §10.1 — D2's `NonUniformResourceIndex` divergence, measured rather than argued.
///
/// The SAME N quads over 1 / 8 / 64 distinct slots, GPU-timestamped around the UI pass, at
/// N ∈ {256, 2048}. Nothing is asserted about the numbers: the decision they inform (is
/// Model A — a runtime atlas — worth reaching for?) is recorded in S7 either way, and
/// `UiImage { texture, uv_min, uv_max }` already describes an atlas tile and a bindless
/// slot equally well, which is why the deferral is honest rather than hopeful.
///
/// The instrument's own resolution is reported beside the numbers: a delta smaller than
/// the timestamp lattice's step is not a small effect, it is no measurement.
#[test]
fn ui_sprite_slot_divergence_measurement_10_1() {
    let ran = with_many_slots("ui_sprite_slot_divergence_measurement_10_1", |rhi, slots| {
        // A warm leg first: the first submit of a fresh pipeline pays one-time costs that
        // belong to no leg.
        let _ = render_leg(rhi, &grid_scene(256, &slots[..1]), PASS_REPEATS);

        println!(
            "§10.1 — UI pass GPU ns/pass (median of 5, {PASS_REPEATS} passes per timestamp              bracket so the device's ~1024 ns timestamp lattice is <0.1% of each figure),              same quads over 1 / 8 / 64 distinct bindless slots:"
        );
        for n in [256u32, 2048u32] {
            let mut row = Vec::new();
            for k in [1usize, 8, 64] {
                // Median of five: a single GPU timestamp on a sub-microsecond pass is
                // dominated by scheduling noise.
                let mut samples: Vec<f64> = (0..5)
                    .map(|_| render_leg(rhi, &grid_scene(n, &slots[..k]), PASS_REPEATS).gpu_ns)
                    .collect();
                samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaN from a tick count"));
                row.push((k, samples[2]));
            }
            let base = row[0].1;
            let cells: Vec<String> = row
                .iter()
                .map(|(k, ns)| {
                    let delta = if base > 0.0 {
                        format!("{:+.1}%", (ns - base) / base * 100.0)
                    } else {
                        "n/a".to_string()
                    };
                    format!("{k:>2} slots: {:>8.1} ns ({delta})", ns)
                })
                .collect();
            println!("  N={n:<5} {}", cells.join("   |   "));
        }
    });
    if !ran {
        eprintln!("SKIP: no device / no validation layer");
    }
}
