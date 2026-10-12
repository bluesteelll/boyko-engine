//! OBS host test 6 (render-wave plan H0 Scope 4, AD-15): the cross-frame wrappers
//! `SeededImage` / `SeededBuffer` — seed coverage checked on the frame's TRAILING SCOPE, the
//! dump-run seed widening with a restore READ, and the initialising-access discipline of the
//! FrameLocal class.
//!
//! # What it asserts
//!
//! 1. **Trailing scope.** A reader-seeded resource whose last reader's stage is outside the seed
//!    fires; an in-frame write at a stage outside a reader seed FOLLOWED by a covered reader is
//!    silent (the `taa_cov` / `csm_trans` / `lit_prev` shapes); a frame that ends on a write under
//!    a reader seed fires (it needs a writer seed), and is silent under one; two unordered
//!    trailing readers need both stages (the `exposure_state` shape); the scope is per mip.
//! 2. **Dump frames.** A reader-seeded image whose frame ends with the dump's `TRANSFER` copy and
//!    its restore READ is silent under the dump-run seed (production ∪ `TRANSFER`); the same frame
//!    with the restore declared as a WRITE fires; under the production seed it fires.
//! 3. **The initialising access.** A FrameLocal wrapper whose first access is `rmw` or `read`
//!    fires, even when that access carries a WRITE bit; `init_attachment` then `rmw` is silent;
//!    every `init_*` kind is accepted as the first access, including a depth attachment.
//! 4. **CrossFrame reads** keep the per-read seed assert of `MaterialTableRes::read`.
//!
//! Every firing case is `cfg(debug_assertions)`: the checks are debug assertions.

use boyko_rhi_vulkan::ffi::{
    VK_ACCESS_COLOR_ATTACHMENT_READ_BIT, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
    VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT,
    VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_TRANSFER_READ_BIT, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
    VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
    VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
    VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT,
};
use boyko_rhi_vulkan::framegraph::{
    FrameGraph, ResSync, SeedContent, SeededBuffer, SeededImage, SubRange,
};

const COMPUTE: u32 = VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT;
const FRAGMENT: u32 = VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT;
const TRANSFER: u32 = VK_PIPELINE_STAGE_TRANSFER_BIT;
const COLOR_OUT: u32 = VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT;
const SRO: i32 = VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL;

fn graph() -> FrameGraph {
    FrameGraph::with_capacity(8, 8, 16)
}

fn readers(stages: u32) -> ResSync {
    ResSync::seeded_readers(stages, VK_ACCESS_SHADER_READ_BIT)
}

/// A FrameLocal single image with a reader seed at `stages`.
fn local_image(g: &mut FrameGraph, name: &'static str, stages: u32) -> SeededImage {
    SeededImage::declare(g, name, readers(stages), 1, SeedContent::FrameLocal, false)
}

/// A FrameLocal single buffer with a reader seed at `stages`.
fn local_buffer(g: &mut FrameGraph, name: &'static str, stages: u32) -> SeededBuffer {
    SeededBuffer::declare(g, name, readers(stages), SeedContent::FrameLocal, false)
}

// --- 1. the trailing scope ---------------------------------------------------------------------

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "SEED COVERAGE violated — the trailing scope of 'fog_sun_far' (mip 0)")]
fn last_reader_outside_the_seed_fires() {
    let mut g = graph();
    let img = local_image(&mut g, "fog_sun_far", COMPUTE);
    g.add_pass("fog_sun_far_depth");
    img.init_storage_full(&mut g, COMPUTE, SubRange::COLOR);
    g.add_pass("a_fragment_reader");
    img.read(&mut g, FRAGMENT, VK_ACCESS_SHADER_READ_BIT, SRO, SubRange::COLOR);
    g.compile();
}

/// `taa_cov` (blended draws at `COLOR_ATTACHMENT_OUTPUT`, seed `COMPUTE`), `csm_trans`
/// (`LATE_FRAGMENT_TESTS` depth + colour draw, seed `COMPUTE | FRAGMENT`) and `lit_prev` (a copy
/// at `TRANSFER`, seed `FRAGMENT | COMPUTE`): each writes outside its reader seed IN the frame,
/// and is followed by a covered reader — no cross-frame gap, so the check is silent.
#[test]
fn in_frame_write_outside_reader_seed_then_covered_reader_is_silent() {
    let mut g = graph();
    let taa_cov = local_image(&mut g, "taa_cov", COMPUTE);
    let csm_trans = local_image(&mut g, "csm_trans", COMPUTE | FRAGMENT);
    let lit_prev = local_image(&mut g, "lit_prev", FRAGMENT | COMPUTE);

    g.add_pass("cov_draws");
    taa_cov.init_attachment(
        &mut g,
        COLOR_OUT,
        VK_ACCESS_COLOR_ATTACHMENT_READ_BIT | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        SubRange::COLOR,
    );
    g.add_pass("csm_trans_draw");
    csm_trans.init_attachment(
        &mut g,
        VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT | COLOR_OUT,
        VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        SubRange::COLOR,
    );
    g.add_pass("scene_color_copy");
    lit_prev.init_copy_full(&mut g, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, SubRange::COLOR);

    g.add_pass("taa_resolve");
    taa_cov.read(&mut g, COMPUTE, VK_ACCESS_SHADER_READ_BIT, SRO, SubRange::COLOR);
    g.add_pass("lit_producers");
    csm_trans.read(&mut g, FRAGMENT, VK_ACCESS_SHADER_READ_BIT, SRO, SubRange::COLOR);
    lit_prev.read(&mut g, FRAGMENT, VK_ACCESS_SHADER_READ_BIT, SRO, SubRange::COLOR);
    g.compile();
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "'counts' (mip 0) ENDS THE FRAME ON A WRITE")]
fn a_frame_ending_on_a_write_under_a_reader_seed_fires() {
    let mut g = graph();
    let buf = local_buffer(&mut g, "counts", COMPUTE);
    g.add_pass("count");
    buf.init_storage_full(&mut g, COMPUTE);
    g.compile();
}

#[test]
fn a_frame_ending_on_a_write_under_a_writer_seed_is_silent() {
    let mut g = graph();
    let buf = SeededBuffer::declare(
        &mut g,
        "alloc",
        ResSync::seeded_writer(COMPUTE, VK_ACCESS_SHADER_WRITE_BIT),
        SeedContent::CrossFrame,
        false,
    );
    g.add_pass("cull");
    buf.write(&mut g, COMPUTE, VK_ACCESS_SHADER_WRITE_BIT);
    g.compile();
}

/// The `exposure_state` shape: the adapt dispatch and the readback copy both read the frame's
/// last write, unordered with each other — the seed must name BOTH.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "SEED COVERAGE violated — the trailing scope of 'exposure' (mip 0)")]
fn two_unordered_trailing_readers_need_both_stages() {
    let mut g = graph();
    let buf = local_buffer(&mut g, "exposure", COMPUTE);
    g.add_pass("exposure_clear");
    buf.init_clear(&mut g);
    g.add_pass("exposure_adapt");
    buf.read(&mut g, COMPUTE, VK_ACCESS_SHADER_READ_BIT);
    g.add_pass("exposure_readback");
    buf.read(&mut g, TRANSFER, VK_ACCESS_TRANSFER_READ_BIT);
    g.compile();
}

#[test]
fn two_unordered_trailing_readers_covered_by_both_stages_are_silent() {
    let mut g = graph();
    let buf = local_buffer(&mut g, "exposure", COMPUTE | TRANSFER);
    g.add_pass("exposure_clear");
    buf.init_clear(&mut g);
    g.add_pass("exposure_adapt");
    buf.read(&mut g, COMPUTE, VK_ACCESS_SHADER_READ_BIT);
    g.add_pass("exposure_readback");
    buf.read(&mut g, TRANSFER, VK_ACCESS_TRANSFER_READ_BIT);
    g.compile();
}

/// Per MIP: level 1 is read at `FRAGMENT` (outside the seed) and level 0 is written AFTER it. No
/// in-frame barrier orders a read of level 1 before a write of level 0 (different subresources),
/// so level 1's trailing scope is the read — a per-resource walk would wrongly stop at the write.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "SEED COVERAGE violated — the trailing scope of 'chain' (mip 1)")]
fn the_trailing_scope_is_per_mip() {
    let mut g = graph();
    let chain = SeededImage::declare(
        &mut g,
        "chain",
        ResSync::seeded_writer(COMPUTE, VK_ACCESS_SHADER_WRITE_BIT),
        2,
        SeedContent::FrameLocal,
        false,
    );
    g.add_pass("build");
    chain.init_storage_full(&mut g, COMPUTE, SubRange::color_mips(2));
    g.add_pass("sample_level_1");
    chain.read(
        &mut g,
        FRAGMENT,
        VK_ACCESS_SHADER_READ_BIT,
        SRO,
        SubRange { base_mip: 1, mip_count: 1, ..SubRange::color_mips(2) },
    );
    g.add_pass("rewrite_level_0");
    chain.rmw(
        &mut g,
        COMPUTE,
        VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
        boyko_rhi_vulkan::ffi::VK_IMAGE_LAYOUT_GENERAL,
        SubRange::COLOR,
    );
    g.compile();
}

// --- 2. dump frames ----------------------------------------------------------------------------

/// The `display` shape: written by a full-extent compute pass, sampled by the present blit at its
/// boundary layout, then — on a dump frame — copied at `TRANSFER_SRC_OPTIMAL` and restored by a
/// READ at the boundary layout. `restore_write` declares the restore as a write instead.
fn dump_frame(dump_widen: bool, restore_write: bool) {
    let mut g = graph();
    let seed = ResSync::seeded_readers_at_layout(SRO, FRAGMENT, VK_ACCESS_SHADER_READ_BIT);
    let display =
        SeededImage::declare(&mut g, "display", seed, 1, SeedContent::FrameLocal, dump_widen);
    g.add_pass("post_final");
    display.init_storage_full(&mut g, COMPUTE, SubRange::COLOR);
    g.add_pass("present_sample");
    display.read(&mut g, FRAGMENT, VK_ACCESS_SHADER_READ_BIT, SRO, SubRange::COLOR);
    g.add_pass("image_dump");
    display.read(
        &mut g,
        TRANSFER,
        VK_ACCESS_TRANSFER_READ_BIT,
        VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        SubRange::COLOR,
    );
    if restore_write {
        display.rmw(&mut g, FRAGMENT, VK_ACCESS_SHADER_WRITE_BIT, SRO, SubRange::COLOR);
    } else {
        display.read(&mut g, FRAGMENT, VK_ACCESS_SHADER_READ_BIT, SRO, SubRange::COLOR);
    }
    g.compile();
}

#[test]
fn dump_frame_copy_and_restore_read_under_the_dump_run_seed_is_silent() {
    dump_frame(true, false);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "'display' (mip 0) ENDS THE FRAME ON A WRITE")]
fn dump_frame_restore_declared_as_a_write_fires() {
    dump_frame(true, true);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "SEED COVERAGE violated — the trailing scope of 'display' (mip 0)")]
fn dump_frame_under_the_production_seed_fires() {
    dump_frame(false, false);
}

// --- 3. the initialising access ----------------------------------------------------------------

/// The `LOAD`-ing blend: `READ | WRITE` passes `compile`'s unwritten-read guard (it carries a
/// write bit), so only the wrapper's discipline can see that it accumulates across frames.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "INITIALISING ACCESS missing — FrameLocal 'taa_cov' is touched by rmw")]
fn rmw_first_fires_even_with_write_bit() {
    let mut g = graph();
    let img = local_image(&mut g, "taa_cov", COLOR_OUT);
    g.add_pass("cov_draws");
    img.rmw(
        &mut g,
        COLOR_OUT,
        VK_ACCESS_COLOR_ATTACHMENT_READ_BIT | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        SubRange::COLOR,
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "INITIALISING ACCESS missing — FrameLocal 'fog_media_acc' is touched by read")]
fn read_first_fires() {
    let mut g = graph();
    let buf = local_buffer(&mut g, "fog_media_acc", COMPUTE);
    g.add_pass("fog_inject");
    buf.read(&mut g, COMPUTE, VK_ACCESS_SHADER_READ_BIT);
}

#[test]
fn init_attachment_then_rmw_is_silent() {
    let mut g = graph();
    let img = local_image(&mut g, "taa_cov", COLOR_OUT | COMPUTE);
    g.add_pass("cov_init");
    img.init_attachment(
        &mut g,
        COLOR_OUT,
        VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        SubRange::COLOR,
    );
    g.add_pass("cov_draws");
    img.rmw(
        &mut g,
        COLOR_OUT,
        VK_ACCESS_COLOR_ATTACHMENT_READ_BIT | VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        SubRange::COLOR,
    );
    g.add_pass("taa_resolve");
    img.read(&mut g, COMPUTE, VK_ACCESS_SHADER_READ_BIT, SRO, SubRange::COLOR);
    g.compile();
}

/// Every `init_*` kind opens a frame, on images and buffers — including `init_attachment` on a
/// DEPTH attachment (`fog_sun_far`'s CLEAR, rev 5).
#[test]
fn every_init_kind_is_accepted_as_the_first_access() {
    let mut g = graph();
    let depth = local_image(&mut g, "fog_sun_far", COMPUTE);
    let cleared = local_image(&mut g, "cleared", COMPUTE);
    let copied = local_image(&mut g, "copied", COMPUTE);
    let stored = local_image(&mut g, "stored", COMPUTE);
    let buf_cleared = local_buffer(&mut g, "buf_cleared", COMPUTE);
    let buf_copied = local_buffer(&mut g, "buf_copied", COMPUTE);
    let buf_stored = local_buffer(&mut g, "buf_stored", COMPUTE);

    g.add_pass("inits");
    depth.init_attachment(
        &mut g,
        VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        SubRange::DEPTH,
    );
    cleared.init_clear(&mut g, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, SubRange::COLOR);
    copied.init_copy_full(&mut g, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, SubRange::COLOR);
    stored.init_storage_full(&mut g, COMPUTE, SubRange::COLOR);
    buf_cleared.init_clear(&mut g);
    buf_copied.init_copy_full(&mut g);
    buf_stored.init_storage_full(&mut g, COMPUTE);

    g.add_pass("readers");
    depth.read(&mut g, COMPUTE, VK_ACCESS_SHADER_READ_BIT, SRO, SubRange::DEPTH);
    for img in [cleared, copied, stored] {
        img.read(&mut g, COMPUTE, VK_ACCESS_SHADER_READ_BIT, SRO, SubRange::COLOR);
    }
    for buf in [buf_cleared, buf_copied, buf_stored] {
        buf.read(&mut g, COMPUTE, VK_ACCESS_SHADER_READ_BIT);
    }
    g.compile();
}

// --- 4. CrossFrame reads -----------------------------------------------------------------------

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "a reader of 'material_table' at stage 0x80 lies outside its cross-frame seed 0x800")]
fn cross_frame_read_outside_the_seed_fires_per_read() {
    let mut g = graph();
    let table = SeededBuffer::declare(
        &mut g,
        "material_table",
        readers(COMPUTE),
        SeedContent::CrossFrame,
        false,
    );
    g.add_pass("material_upload");
    table.write(&mut g, TRANSFER, boyko_rhi_vulkan::ffi::VK_ACCESS_TRANSFER_WRITE_BIT);
    g.add_pass("forward_opaque");
    table.read(&mut g, FRAGMENT, VK_ACCESS_SHADER_READ_BIT);
}

/// The production `material_table` shape (F7): a CrossFrame buffer written by the upload copy and
/// read by the lit producers inside its seed — silent; and the dump widening adds `TRANSFER` to
/// the seed's stages.
#[test]
fn material_table_shape_is_silent_and_dump_widening_adds_transfer() {
    let mut g = graph();
    let table = SeededBuffer::declare(
        &mut g,
        "material_table",
        readers(COMPUTE),
        SeedContent::CrossFrame,
        false,
    );
    g.add_pass("material_upload");
    table.write(&mut g, TRANSFER, boyko_rhi_vulkan::ffi::VK_ACCESS_TRANSFER_WRITE_BIT);
    g.add_pass("deferred_pbr");
    table.read(&mut g, COMPUTE, VK_ACCESS_SHADER_READ_BIT);
    g.compile();
    assert_eq!(table.seed_stages(), COMPUTE);
    assert_eq!(g.res_seed(table.id()).visible_stages, COMPUTE);

    let mut g = graph();
    let widened = SeededBuffer::declare(
        &mut g,
        "material_table",
        readers(COMPUTE),
        SeedContent::CrossFrame,
        true,
    );
    assert_eq!(widened.seed_stages(), COMPUTE | TRANSFER);
    assert_eq!(g.res_seed(widened.id()).visible_access & VK_ACCESS_TRANSFER_READ_BIT, VK_ACCESS_TRANSFER_READ_BIT);
}
