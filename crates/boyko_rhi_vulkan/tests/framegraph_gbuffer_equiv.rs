//! Step 1b equivalence gate: the frame graph, declared over the FULL (maximal-
//! permutation) on-screen G-buffer frame, must auto-derive a barrier set that is
//! a SOUND SUPERSET of `swapchain::record_gbuffer`'s hand-authored barriers —
//! same per-resource layout trajectories, every producer→consumer hazard covered,
//! and no more barriers than the hand path (minimality, C6).
//!
//! This is a pure-CPU diff: the graph does NOT drive the GPU in Step 1b, so this
//! runs on any machine (no `#[ignore]`, no Vulkan device). It is the reference
//! the live hand path is measured against before Step 1f deletes it.

use std::fmt::Write as _;

use boyko_rhi_vulkan::ffi::{
    VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
    VK_ACCESS_INDIRECT_COMMAND_READ_BIT, VK_ACCESS_SHADER_READ_BIT, VK_ACCESS_SHADER_WRITE_BIT,
    VK_ACCESS_TRANSFER_WRITE_BIT, VK_IMAGE_ASPECT_COLOR_BIT, VK_IMAGE_ASPECT_DEPTH_BIT,
    VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
    VK_IMAGE_LAYOUT_GENERAL, VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, VK_IMAGE_LAYOUT_UNDEFINED,
    VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
    VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
    VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
    VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
};
use boyko_rhi_vulkan::framegraph::{
    BufBarrier, FrameGraph, ImgBarrier, PassBarrierRange, ResId, ResSync, SubRange,
};

/// Cascade / atlas layer counts, pinned to the scene's clamped `csm.active_count`
/// and atlas `active_layers` (record_gbuffer clamps `[1, MAX_CASCADES]` /
/// `[1, MAX_TEXTURE_LAYERS]`). W3: the derived `SubRange.layer_count` tracks these;
/// if the shipping scene runs a different count, drive these from the same const.
const CASCADE_LAYERS: u32 = 4;
const ATLAS_LAYERS: u32 = 6;

const FRAG: u32 = VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT;
const RW: u32 = VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT;

/// Declare the maximal G-buffer frame (raster → light-upload → coarse-cull →
/// marcher → ssao → light-cull → csm-depth → atlas-depth → resolve → present),
/// exactly mirroring the accesses `record_gbuffer` performs with EVERY optional
/// pass wired. Returns the compiled graph + the resource handles for assertions.
struct Frame {
    g: FrameGraph,
    albedo: ResId,
    normal: ResId,
    material: ResId,
    depth: ResId,
    viewt: ResId,
    lit: ResId,
    ssao: ResId,
    cascade: ResId,
    atlas: ResId,
    swapchain: ResId,
    light_table: ResId,
    tiles: ResId,
    grid: ResId,
    index: ResId,
    alloc: ResId,
}

fn build_maximal_frame() -> Frame {
    let mut g = FrameGraph::with_capacity(16, 16, 64);

    // Images. MIRRORS `declare_deferred_graph`: ringed resources start undefined;
    // the SINGLE-INSTANCE cascade/atlas are seeded with the sibling in-flight
    // frame's end-of-frame consumer scopes (the cross-frame WAR fix, B-002/B-003).
    let albedo = g.add_image("albedo");
    let normal = g.add_image("normal");
    let material = g.add_image("material");
    let depth = g.add_image("depth");
    let viewt = g.add_image("viewt");
    let lit = g.add_image("lit");
    let ssao = g.add_image("ssao");
    let cascade = g.add_image_seeded(
        "cascade",
        ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
    );
    let atlas = g.add_image_seeded(
        "atlas",
        ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
    );
    // The WSI swapchain image (acquired UNDEFINED, presented PRESENT_SRC_KHR) — a
    // first-class graph resource, so the acquire→render→present transition is
    // owned + verified like any other (C2). Per-image acquire/present semaphores
    // handle its cross-frame ordering — NOT seeded.
    let swapchain = g.add_image("swapchain");
    // Buffers — all single instances shared by both in-flight frames (seeded,
    // mirroring `declare_deferred_graph`): light_table/tiles/grid/index end their
    // frame consumed by a COMPUTE read; `alloc` ends on the cull's undrained
    // atomic writes (writer seed → full memory dependency for the next reset).
    let light_table = g.add_buffer_seeded(
        "light_table",
        ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
    );
    let tiles = g.add_buffer_seeded(
        "tiles",
        ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
    );
    let grid = g.add_buffer_seeded(
        "grid",
        ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
    );
    let index = g.add_buffer_seeded(
        "index",
        ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
    );
    let alloc = g.add_buffer_seeded(
        "alloc",
        ResSync::seeded_writer(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT),
    );

    // Pass A — raster the 3-MRT G-buffer + depth (record_gbuffer barriers 0/1).
    g.add_pass("raster");
    for &c in &[albedo, normal, material] {
        g.image_access(
            c,
            VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
            VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            SubRange::COLOR,
        );
    }
    g.image_access(
        depth,
        FRAG,
        VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        SubRange::DEPTH,
    );

    // Lighting L0-r0 — async light-table re-upload (copy) then read (barrier).
    g.add_pass("light_upload");
    g.buffer_access(light_table, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT);

    // Render P0 — coarse cull: samples depth (transitions it), writes tiles.
    g.add_pass("coarse_cull");
    g.image_access(
        depth,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
        VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        SubRange::DEPTH,
    );
    g.buffer_access(tiles, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT);

    // Pass B — the marcher: reads depth + tiles, read|writes the attributes + gViewT.
    g.add_pass("marcher");
    g.image_access(
        depth,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
        VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        SubRange::DEPTH,
    );
    g.buffer_access(tiles, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    for &c in &[albedo, normal, material, viewt] {
        g.image_access(c, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR);
    }

    // Render P7 — SSAO: reads normal/material/viewt, writes ssao.
    g.add_pass("ssao");
    for &c in &[normal, material, viewt] {
        g.image_access(c, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR);
    }
    g.image_access(ssao, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR);

    // Lighting L1 — clustered cull: resets alloc (transfer), reads table, writes grid/index.
    g.add_pass("light_cull");
    g.buffer_access(alloc, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT);
    g.buffer_access(alloc, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW);
    g.buffer_access(light_table, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    g.buffer_access(grid, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT);
    g.buffer_access(index, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT);

    // CSM cascade depth pass + spot atlas depth pass (layered, W3 — counts pinned
    // to the scene's clamped active_count via CASCADE_LAYERS / ATLAS_LAYERS).
    g.add_pass("csm_depth");
    g.image_access(cascade, FRAG, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, SubRange::depth_layers(CASCADE_LAYERS));
    g.add_pass("atlas_depth");
    g.image_access(atlas, FRAG, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, SubRange::depth_layers(ATLAS_LAYERS));

    // Resolve — reads everything, writes lit.
    g.add_pass("resolve");
    for &c in &[albedo, normal, material, viewt, ssao] {
        g.image_access(c, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR);
    }
    g.buffer_access(grid, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    g.buffer_access(index, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    g.buffer_access(light_table, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    g.image_access(cascade, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::depth_layers(CASCADE_LAYERS));
    g.image_access(atlas, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::depth_layers(ATLAS_LAYERS));
    g.image_access(lit, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR);

    // Present pass C — FRAGMENT-samples lit (record_gbuffer's `lit_to_sampled`:
    // COMPUTE→FRAGMENT, GENERAL→SHADER_READ_ONLY) and WRITES the acquired swapchain
    // image as a COLOR attachment (record_gbuffer barrier 7: swapchain UNDEFINED→
    // COLOR_ATTACHMENT_OPTIMAL, TOP_OF_PIPE→COLOR_OUT).
    g.add_pass("present_draw");
    g.image_access(lit, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::COLOR);
    g.image_access(swapchain, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, SubRange::COLOR);

    // Post-draw swapchain transition (record_gbuffer barrier 9, steady path):
    // COLOR_ATTACHMENT_OPTIMAL → PRESENT_SRC_KHR, COLOR_OUT→BOTTOM_OF_PIPE, dst_access=0.
    g.add_pass("present_transition");
    g.image_access(swapchain, VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT, 0, VK_IMAGE_LAYOUT_PRESENT_SRC_KHR, SubRange::COLOR);

    g.compile();
    Frame { g, albedo, normal, material, depth, viewt, lit, ssao, cascade, atlas, swapchain, light_table, tiles, grid, index, alloc }
}

#[allow(clippy::too_many_arguments)]
fn has_img(bs: &[ImgBarrier], res: ResId, ss: u32, ds: u32, sa: u32, da: u32, ol: i32, nl: i32, sub: SubRange) -> bool {
    bs.iter().any(|b| {
        b.res == res
            && b.src_stage == ss
            && b.dst_stage == ds
            && b.src_access == sa
            && b.dst_access == da
            && b.old_layout == ol
            && b.new_layout == nl
            && b.subresource == sub
    })
}

fn has_buf(bs: &[BufBarrier], res: ResId, ss: u32, ds: u32, sa: u32, da: u32) -> bool {
    bs.iter()
        .any(|b| b.res == res && b.src_stage == ss && b.dst_stage == ds && b.src_access == sa && b.dst_access == da)
}

#[test]
fn graph_reproduces_gbuffer_layout_trajectories() {
    let f = build_maximal_frame();
    let g = &f.g;
    // Every resource ends the frame in the layout the hand path leaves it — read
    // from the ground-truth compiled sync state (W1: not reconstructed from barriers).
    assert_eq!(g.resolved_layout(f.albedo), VK_IMAGE_LAYOUT_GENERAL, "albedo");
    assert_eq!(g.resolved_layout(f.normal), VK_IMAGE_LAYOUT_GENERAL, "normal");
    assert_eq!(g.resolved_layout(f.material), VK_IMAGE_LAYOUT_GENERAL, "material");
    assert_eq!(g.resolved_layout(f.viewt), VK_IMAGE_LAYOUT_GENERAL, "viewt");
    assert_eq!(g.resolved_layout(f.ssao), VK_IMAGE_LAYOUT_GENERAL, "ssao");
    assert_eq!(g.resolved_layout(f.depth), VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, "depth");
    assert_eq!(g.resolved_layout(f.cascade), VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, "cascade");
    assert_eq!(g.resolved_layout(f.atlas), VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, "atlas");
    assert_eq!(g.resolved_layout(f.lit), VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, "lit");
    assert_eq!(g.resolved_layout(f.swapchain), VK_IMAGE_LAYOUT_PRESENT_SRC_KHR, "swapchain");
}

#[test]
fn graph_covers_every_gbuffer_producer_consumer_hazard() {
    let f = build_maximal_frame();
    let img = f.g.img_barriers();
    let buf = f.g.buf_barriers();

    // --- albedo: UNDEFINED→COLOR (raster in), COLOR→GENERAL (marcher), store→load (resolve) ---
    assert!(
        has_img(img, f.albedo, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, 0, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, SubRange::COLOR),
        "albedo raster barrier-in missing"
    );
    assert!(
        has_img(img, f.albedo, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, RW, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR),
        "albedo color→general hand-off missing"
    );
    assert!(
        has_img(img, f.albedo, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_GENERAL, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR),
        "albedo store→load missing"
    );

    // --- depth: UNDEFINED→DEPTH (raster), DEPTH→SHADER_READ_ONLY (cull/marcher) ---
    assert!(
        has_img(img, f.depth, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, FRAG, 0, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, SubRange::DEPTH),
        "depth raster barrier-in missing"
    );
    assert!(
        has_img(img, f.depth, FRAG, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::DEPTH),
        "depth→sampled missing"
    );

    // --- viewt: UNDEFINED→GENERAL first touch (marcher), store→load (ssao) ---
    assert!(
        has_img(img, f.viewt, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, 0, RW, VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR),
        "viewt first-touch general missing"
    );

    // --- W2: the marcher→SSAO store→load for normal/material/viewt (the subtle
    // RW-write→read flush that diverges positionally from the hand path's eager
    // batch — the graph places it at the SSAO consumer). SHADER_WRITE→SHADER_READ,
    // GENERAL→GENERAL, COMPUTE→COMPUTE, for each of the three SSAO inputs. ---
    for &c in &[f.normal, f.material, f.viewt] {
        assert!(
            has_img(img, c, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_GENERAL, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR),
            "marcher→SSAO store→load missing for a G-buffer attribute"
        );
    }

    // --- swapchain (WSI): UNDEFINED→COLOR (acquire→present draw, barrier 7),
    // COLOR→PRESENT_SRC_KHR (post-draw, barrier 9: dst_access=0, →BOTTOM_OF_PIPE). ---
    assert!(
        has_img(img, f.swapchain, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, 0, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, SubRange::COLOR),
        "swapchain acquire→color missing"
    );
    assert!(
        has_img(img, f.swapchain, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, 0, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_PRESENT_SRC_KHR, SubRange::COLOR),
        "swapchain color→present missing"
    );

    // --- lit: UNDEFINED→GENERAL (resolve write), GENERAL→TRANSFER_SRC (present) ---
    assert!(
        has_img(img, f.lit, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, 0, VK_ACCESS_SHADER_WRITE_BIT, VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR),
        "lit resolve-write general missing"
    );
    assert!(
        has_img(img, f.lit, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_GENERAL, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::COLOR),
        "lit→present-blit fragment-sample missing"
    );

    // --- CSM cascade layered: UNDEFINED→DEPTH (4 layers), DEPTH→SHADER_READ_ONLY (4 layers).
    // The depth-in src is COMPUTE (not TOP_OF_PIPE): the cascade is a SINGLE image shared by
    // both in-flight frames, so its re-render must order after the SIBLING frame's resolve
    // reads — the cross-frame WAR seed supplies that src (B-003). Layout still UNDEFINED
    // (content discarded); only the ordering strengthened. ---
    assert!(
        has_img(img, f.cascade, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, FRAG, 0, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, SubRange::depth_layers(CASCADE_LAYERS)),
        "cascade layered depth-in missing (must carry the cross-frame WAR src = COMPUTE)"
    );
    assert!(
        has_img(img, f.cascade, FRAG, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::depth_layers(CASCADE_LAYERS)),
        "cascade layered →sampled missing"
    );

    // --- buffers: tiles (cull→marcher), light_table (transfer→compute), grid (cull→resolve) ---
    assert!(
        has_buf(buf, f.tiles, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT),
        "tiles cull→marcher missing"
    );
    assert!(
        has_buf(buf, f.light_table, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT),
        "light_table upload→read missing"
    );
    assert!(
        has_buf(buf, f.grid, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT),
        "grid cull→resolve missing"
    );
    assert!(
        has_buf(buf, f.alloc, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT, RW),
        "alloc reset→cull missing"
    );
    // The `index` buffer is symmetric with `grid`.
    assert!(
        has_buf(buf, f.index, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT),
        "index cull→resolve missing"
    );

    // --- The CROSS-FRAME seeds (B-002): each single-instance buffer's FIRST write must
    // order after the sibling in-flight frame's end-of-frame accesses. WAR seeds emit an
    // execution-only src (src_access 0, src_stage = the sibling readers); the alloc WRITER
    // seed emits the full memory dependency (its sibling write is undrained). ---
    assert!(
        has_buf(buf, f.light_table, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT, 0, VK_ACCESS_TRANSFER_WRITE_BIT),
        "light_table cross-frame WAR (sibling resolve reads → this upload) missing"
    );
    assert!(
        has_buf(buf, f.tiles, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, 0, VK_ACCESS_SHADER_WRITE_BIT),
        "tiles cross-frame WAR (sibling marcher reads → this cull write) missing"
    );
    assert!(
        has_buf(buf, f.alloc, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_TRANSFER_WRITE_BIT),
        "alloc cross-frame WAW (sibling cull atomics → this reset) missing"
    );
}

#[test]
fn graph_matches_hand_path_barrier_count_exactly() {
    let f = build_maximal_frame();
    let img = f.g.img_barriers().len();
    let buf = f.g.buf_barriers().len();

    // Derived by ENUMERATING every `cmd_pipeline_barrier`-emitted image/buffer
    // barrier in `record_gbuffer` (pre-1f) for the maximal-live permutation:
    //   IMAGE (23): color-in ×3, depth-in ×1, depth→sampled ×1, color→general ×3,
    //     lit/viewt/ssao→general ×3, store→load ×4, ssao store→load ×1,
    //     csm-in ×1, csm→sampled ×1, atlas-in ×1, atlas→sampled ×1, lit→sampled ×1,
    //     swapchain acquire→color ×1, swapchain color→present ×1.
    //   BUFFER: the hand path emitted 5 (light-upload ×1, coarse-tiles ×1,
    //     alloc-reset ×1, cull→resolve ×2) and LACKED the cross-frame ordering on
    //     the single-instance buffers — the audited B-002 WAR race. The seeded
    //     graph adds exactly the 5 missing first-write barriers (light_table /
    //     tiles / grid / index WAR + alloc WAW), a deliberate sound SUPERSET:
    //     5 hand + 5 cross-frame = 10.
    // Image count is unchanged by the seeds (the cascade/atlas first-touch
    // barriers already existed; only their src stage strengthened TOP→COMPUTE).
    const HAND_IMAGE_BARRIERS: usize = 23;
    const BUFFER_BARRIERS_WITH_CROSS_FRAME: usize = 10;
    assert_eq!(img, HAND_IMAGE_BARRIERS, "image barrier count diverged from the hand path");
    assert_eq!(
        buf, BUFFER_BARRIERS_WITH_CROSS_FRAME,
        "buffer barrier count diverged (5 hand-parity + 5 cross-frame seeds)"
    );
}

/// Counts the sync1 `vkCmdPipelineBarrier` array-CALLS the graph's record step
/// would emit (one per distinct (src,dst) stage pair per pass).
#[derive(Default)]
struct CountSink {
    img_calls: usize,
    buf_calls: usize,
}
impl boyko_rhi_vulkan::framegraph::BarrierSink for CountSink {
    fn image_barriers(&mut self, _s: u32, _d: u32, _g: &[ImgBarrier]) {
        self.img_calls += 1;
    }
    fn buffer_barriers(&mut self, _s: u32, _d: u32, _g: &[BufBarrier]) {
        self.buf_calls += 1;
    }
}

/// C6 (honest count quantification): the graph's record step groups each pass's
/// derived barriers by (src,dst) stage pair into batched array calls. The hand
/// path emitted 18 array calls; against those 18 the graph is call-for-call
/// PARITY (it fuses some batches the hand path split and splits some it fused,
/// netting zero). The cross-frame seeds (B-002) then add exactly FOUR calls the
/// hand path never recorded — the fixed race: the light_upload WAR (+1), the
/// coarse tiles WAR (+1), the cull grid+index WAR group (+1), and the alloc WAW
/// (+1) — 18 + 4 = 22, each one a real missing ordering, not grouping loss.
#[test]
fn record_step_call_count_is_pinned() {
    let f = build_maximal_frame();
    let mut sink = CountSink::default();
    f.g.record_all(&mut sink);
    let total = sink.img_calls + sink.buf_calls;
    const HAND_ARRAY_CALLS_PLUS_CROSS_FRAME: usize = 18 + 4;
    assert_eq!(
        total, HAND_ARRAY_CALLS_PLUS_CROSS_FRAME,
        "graph record call count diverged (18 hand-parity + 4 cross-frame WAR/WAW groups)"
    );
}

/// W5: the OFF path. With every optional pass disabled (no cull/ssao/L1/CSM/
/// atlas/light-upload), the CORE raster→marcher→resolve→present barriers must be
/// byte-for-byte the SAME as in the maximal frame — optional passes are purely
/// additive, they never perturb a neighbour's layout trajectory.
#[test]
fn optional_passes_are_additive_core_barriers_unperturbed() {
    let mut g = FrameGraph::with_capacity(8, 8, 32);
    let albedo = g.add_image("albedo");
    let normal = g.add_image("normal");
    let material = g.add_image("material");
    let depth = g.add_image("depth");
    let viewt = g.add_image("viewt");
    let lit = g.add_image("lit");

    g.add_pass("raster");
    for &c in &[albedo, normal, material] {
        g.image_access(c, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, SubRange::COLOR);
    }
    g.image_access(depth, FRAG, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, SubRange::DEPTH);

    g.add_pass("marcher");
    g.image_access(depth, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::DEPTH);
    for &c in &[albedo, normal, material, viewt] {
        g.image_access(c, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR);
    }

    g.add_pass("resolve");
    for &c in &[albedo, normal, material, viewt] {
        g.image_access(c, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR);
    }
    g.image_access(lit, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR);

    g.add_pass("present_blit");
    g.image_access(lit, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::COLOR);

    g.compile();
    let img = g.img_barriers();

    // The exact same core barriers the maximal frame derives for these resources.
    assert!(has_img(img, albedo, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, 0, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, SubRange::COLOR));
    assert!(has_img(img, albedo, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, RW, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR));
    assert!(has_img(img, albedo, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_GENERAL, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR));
    // depth→sampled is now triggered by the marcher (no coarse cull), identical barrier.
    assert!(has_img(img, depth, FRAG, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::DEPTH));

    // No optional-resource barriers exist (they were never declared): pure core.
    // albedo/normal/material = 3 each, depth = 2, viewt = 2, lit = 2 → 15.
    assert_eq!(g.buf_barriers().len(), 0, "OFF path emits zero buffer barriers");
    assert_eq!(img.len(), 15, "minimal core barrier count");
}

#[test]
fn compile_is_idempotent_and_reset_reuses_capacity() {
    let mut f = build_maximal_frame();
    let first: Vec<_> = f.g.img_barriers().to_vec();
    // Recompiling the same declarations yields the identical barrier set.
    f.g.compile();
    assert_eq!(f.g.img_barriers(), first.as_slice(), "compile not idempotent");

    // A fresh frame (reset + re-declare) reproduces the same plan — no stale SoA.
    let f2 = build_maximal_frame();
    assert_eq!(f2.g.img_barriers(), first.as_slice(), "reset/rebuild diverged");
}

/// Pillar B B3 refined-B: the interp-ON path is PURELY ADDITIVE, and the SHARED instance
/// ring (`interp_model_out`, `declare_deferred_graph`'s ResId 20 after the SDFDDGI I2 buffer
/// reshuffle — DDGI classification/ray-table took ResIds 16/17, so the interp trio shifted to
/// 18/19/20) that the interp compute writes is read at VERTEX by THREE passes — the raster
/// G-buffer pass AND the CSM cascade depth pass AND the punctual atlas depth pass, all binding
/// the same physical instance SSBO (`scene.instance_bind_group`; see `gbuffer.rs` — the
/// csm/atlas VS "binds the SAME instance SSBO the main pass binds"). The graph derives EXACTLY
/// ONE COMPUTE→VERTEX RAW barrier for that ring, at the FIRST reader (the raster pass), and that
/// single barrier covers all three readers by Vulkan memory-dependency semantics (a memory
/// dependency makes the interp write available/visible to every subsequent same-stage access;
/// the later csm/atlas VERTEX reads need no re-barrier). The refined-B topology faithful to
/// `declare_deferred_graph`:
///   - `interp` pass: `buffer_access(interp_model_out, COMPUTE, WRITE)` — the compute write
///     of the ring's dynamic slots (the `add_pass("interp")` block in graph_bridge.rs);
///   - `raster` pass: `buffer_access(interp_model_out, VERTEX, READ)` — the ONLY declared read
///     of the ring; this is where the graph derives the single COMPUTE→VERTEX RAW (the
///     interp-gated `buffer_access` in the `raster` pass block in graph_bridge.rs);
///   - `csm_depth` / `atlas_depth` passes: declare ONLY their layered depth `image_access`
///     (the `add_pass("csm_depth")` / `add_pass("atlas_depth")` blocks in graph_bridge.rs) —
///     they DO NOT declare a `buffer_access` on the
///     ring; their VS reads of the same physical buffer are covered by the raster barrier plus
///     recording order (interp → raster barrier recorded → … → csm → atlas draw; gbuffer.rs
///     L217/L894/L1086). Modeling the graph exactly means NOT declaring csm/atlas ring reads.
///
/// So the graph derives EXACTLY TWO new buffer barriers, both on the interp SSBOs, NONE on the
/// core resources:
///   1. the `interp_pairs` first-touch visibility barrier (`TOP_OF_PIPE → COMPUTE`, no src
///      access) — a benign execution-only barrier making the freshly host-written,
///      frame-private pair slot visible to the interp compute read (the buffer analogue of
///      an image's first-touch UNDEFINED→layout; costs nothing since host-coherent writes
///      are already ordered by the submit);
///   2. the `interp_model_out` COMPUTE(WRITE)→VERTEX(READ) RAW at the raster (the first ring
///      reader) — the load-bearing barrier ordering the interp write before ALL THREE VS reads
///      (raster + csm + atlas). It is derived ONCE, not once-per-reader.
///
/// The interp SSBOs are `add_buffer` (undefined, frame-private), so there is NO cross-frame
/// WAR/WAW seed; the CORE raster/marcher/csm/atlas/resolve barriers are byte-unperturbed. The
/// pin here is "EXACTLY ONE COMPUTE→VERTEX barrier for the shared ring across the whole frame"
/// (proving no redundant re-barrier at the csm/atlas readers), NOT "a barrier per reader".
#[test]
fn interp_prepass_adds_exactly_one_shared_ring_compute_to_vertex_barrier() {
    const RW_ATTR: u32 = VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT;

    let mut g = FrameGraph::with_capacity(8, 8, 32);
    // The core images the raster/marcher/csm/atlas/resolve/present touch. cascade/atlas are the
    // layered shadow-depth targets the csm_depth / atlas_depth passes write (as in the maximal
    // frame); they are `add_image` here (this pin isolates the interp arm, so no cross-frame
    // seed is modeled — the raster core barriers are the invariant under test).
    let albedo = g.add_image("albedo");
    let normal = g.add_image("normal");
    let material = g.add_image("material");
    let depth = g.add_image("depth");
    let viewt = g.add_image("viewt");
    let lit = g.add_image("lit");
    let cascade = g.add_image("cascade");
    let atlas = g.add_image("atlas");
    // The B3 interp SSBOs. `interp_model_out` (ResId 20 in `declare_deferred_graph` after the
    // SDFDDGI I2 buffer reshuffle — DDGI classification/ray-table occupy 16/17, so the interp
    // trio is 18/19/20) is the SHARED instance ring; `interp_pairs` is the host-written pair
    // input. Both FIF-ringed / frame-private ⇒ an `undefined()` start state, so no cross-frame
    // ordering — only the intra-frame COMPUTE→VERTEX RAW is derived. ResIds are declared BEFORE
    // the interp pass (which accesses them), mirroring `declare_deferred_graph`. This test builds
    // its OWN local frame (not `declare_deferred_graph`), so the absolute ResId numbering does not
    // affect it — the note keeps the cross-reference accurate.
    //
    // VG R3 P2-8: `interp_pairs` takes the declarator's `add_buffer_seeded(.., undefined())`
    // spelling, because it is HOST-filled and the `interp` pass below only READS it — a bare
    // `add_buffer` now asserts an in-graph producer. `interp_model_out` stays bare: `interp`
    // WRITES it. The seed value is unchanged, so the barrier counts asserted below are unmoved.
    let interp_pairs = g.add_buffer_seeded("interp_pairs", ResSync::undefined());
    let interp_model_out = g.add_buffer("interp_model_out");

    // Pass `interp` — runs FIRST: reads pairs (first touch), writes the shared ring.
    g.add_pass("interp");
    g.buffer_access(interp_pairs, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    g.buffer_access(interp_model_out, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT);

    // Pass `raster` — the FIRST ring reader: reads `interp_model_out` at VERTEX (the VS indexes
    // `instances[...]`), then the usual 3-MRT + depth writes. The graph derives the ONE
    // COMPUTE→VERTEX RAW here.
    g.add_pass("raster");
    g.buffer_access(interp_model_out, VK_PIPELINE_STAGE_VERTEX_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    for &c in &[albedo, normal, material] {
        g.image_access(c, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, SubRange::COLOR);
    }
    g.image_access(depth, FRAG, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, SubRange::DEPTH);

    // Pass `marcher` (the core tail).
    g.add_pass("marcher");
    g.image_access(depth, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::DEPTH);
    for &c in &[albedo, normal, material, viewt] {
        g.image_access(c, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW_ATTR, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR);
    }

    // Pass `csm_depth` — the SECOND ring reader. It binds the SAME physical instance ring at its
    // VS (gbuffer.rs L962-963: "the SAME instance SSBO the main pass binds"), but in the real
    // graph it declares ONLY its layered depth write — NOT a `buffer_access` on the ring — and
    // relies on the raster barrier + recording order (raster barrier is recorded before csm
    // draws). Mirror that exactly: no ring `buffer_access` here, only the depth `image_access`.
    g.add_pass("csm_depth");
    g.image_access(cascade, FRAG, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, SubRange::depth_layers(CASCADE_LAYERS));

    // Pass `atlas_depth` — the THIRD ring reader. Same as csm_depth: binds the shared ring VS,
    // declares ONLY its layered depth write, covered by the single raster barrier + order.
    g.add_pass("atlas_depth");
    g.image_access(atlas, FRAG, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, SubRange::depth_layers(ATLAS_LAYERS));

    // Pass `resolve` → `present_blit`. resolve reads the layered shadow maps (→sampled).
    g.add_pass("resolve");
    for &c in &[albedo, normal, material, viewt] {
        g.image_access(c, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR);
    }
    g.image_access(cascade, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::depth_layers(CASCADE_LAYERS));
    g.image_access(atlas, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::depth_layers(ATLAS_LAYERS));
    g.image_access(lit, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_IMAGE_LAYOUT_GENERAL, SubRange::COLOR);
    g.add_pass("present_blit");
    g.image_access(lit, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, SubRange::COLOR);

    g.compile();
    let buf = g.buf_barriers();

    // (2) The load-bearing barrier: the shared-ring COMPUTE(WRITE) → VERTEX(READ) RAW, derived
    // at the raster (the first reader). This ONE barrier orders the interp write before ALL
    // THREE VS reads (raster + csm + atlas).
    assert!(
        has_buf(
            buf,
            interp_model_out,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        ),
        "interp_model_out COMPUTE→VERTEX RAW barrier (the shared ring write → first VS read) missing"
    );
    // EXACTLY ONE COMPUTE→VERTEX barrier for the shared ring across the WHOLE frame — the
    // refined-B pin. The csm_depth/atlas_depth passes declare no ring `buffer_access` (they rely
    // on this barrier + recording order), and even if a graph erroneously re-declared the ring
    // read there, the memory dependency at the first reader already makes the write visible to
    // every subsequent VERTEX access, so a second barrier would be redundant. Count is 1.
    let ring_compute_to_vertex = buf
        .iter()
        .filter(|b| {
            b.res == interp_model_out
                && b.src_stage == VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT
                && b.dst_stage == VK_PIPELINE_STAGE_VERTEX_SHADER_BIT
        })
        .count();
    assert_eq!(
        ring_compute_to_vertex, 1,
        "shared ring must derive EXACTLY ONE COMPUTE→VERTEX barrier (at the first reader), \
         covering all three VS readers by Vulkan memory dependency — NOT one per reader"
    );
    // (1) The `interp_pairs` first-touch visibility barrier: TOP_OF_PIPE → COMPUTE, no src
    // access (a benign execution-only barrier on the freshly host-written, frame-private slot).
    assert!(
        has_buf(
            buf,
            interp_pairs,
            VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            0,
            VK_ACCESS_SHADER_READ_BIT,
        ),
        "interp pairs first-touch TOP_OF_PIPE→COMPUTE visibility barrier missing"
    );
    // EXACTLY two buffer barriers total — both on the interp SSBOs; the frame-private (undefined)
    // seed adds no cross-frame WAR/WAW, and no core buffer is touched. The three ring readers
    // (raster + csm + atlas) collapse to the SINGLE ring RAW asserted above.
    assert_eq!(
        buf.len(),
        2,
        "interp adds exactly TWO buffer barriers (pairs first-touch + the ONE shared-ring RAW)"
    );
}

/// HW-RT rung R2a-3: the TLAS pack + build passes (with interp on) derive EXACTLY the two
/// declared barriers on the ring and the instance array — (1) interp(COMPUTE-WRITE) →
/// pack(COMPUTE-READ) RAW on the shared instance ring at the pack (the first COMPUTE reader), and
/// (2) pack(COMPUTE-WRITE) → build(AS_BUILD-READ) RAW on `tlas_instances` at the build — and no
/// core-resource barrier perturbation. This builds its OWN local frame (not
/// `declare_deferred_graph`), so the absolute ResId numbering is irrelevant; it isolates the
/// pack/build barrier derivation.
#[cfg(feature = "hwrt")]
#[test]
fn tlas_pack_build_derives_two_buffer_barriers() {
    use boyko_rhi_vulkan::ffi::VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR;

    let mut g = FrameGraph::with_capacity(8, 8, 32);
    let albedo = g.add_image("albedo");
    let normal = g.add_image("normal");
    let material = g.add_image("material");
    let depth = g.add_image("depth");
    // The shared instance ring + the R2a-3 instance array. Both FIF-ringed / frame-private ⇒
    // `add_buffer` (undefined seed), so no cross-frame ordering — only the intra-frame RAWs.
    let interp_model_out = g.add_buffer("interp_model_out");
    let tlas_instances = g.add_buffer("tlas_instances");

    // Pass `interp` — writes the shared ring.
    g.add_pass("interp");
    g.buffer_access(interp_model_out, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT);

    // Pass `tlas_pack` — reads the ring (COMPUTE, the interp→pack RAW's first reader) + writes
    // the instance array (COMPUTE).
    g.add_pass("tlas_pack");
    g.buffer_access(interp_model_out, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    g.buffer_access(tlas_instances, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT);

    // Pass `tlas_build` — reads the instance array at the AS-build stage (the pack→build RAW).
    g.add_pass("tlas_build");
    g.buffer_access(tlas_instances, VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR, VK_ACCESS_SHADER_READ_BIT);

    // Pass `raster` — the core tail (no ring read here; this pin isolates the pack/build arms).
    g.add_pass("raster");
    for &c in &[albedo, normal, material] {
        g.image_access(c, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, SubRange::COLOR);
    }
    g.image_access(depth, FRAG, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, SubRange::DEPTH);

    g.compile();
    let buf = g.buf_barriers();

    // (1) interp(COMPUTE-WRITE) → pack(COMPUTE-READ) RAW on the shared ring, at the pack.
    assert!(
        has_buf(
            buf,
            interp_model_out,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        ),
        "interp_model_out COMPUTE(WRITE)→COMPUTE(READ) RAW at the pack missing"
    );
    // (2) pack(COMPUTE-WRITE) → build(AS_BUILD-READ) RAW on the instance array, at the build.
    assert!(
        has_buf(
            buf,
            tlas_instances,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_PIPELINE_STAGE_ACCELERATION_STRUCTURE_BUILD_BIT_KHR,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        ),
        "tlas_instances COMPUTE(WRITE)→AS_BUILD(READ) RAW at the build missing"
    );
    // EXACTLY two buffer barriers total — the ring RAW at the pack + the instance-array RAW at
    // the build; the frame-private (undefined) seeds add no cross-frame hazard, no core buffer is
    // touched.
    assert_eq!(
        buf.len(),
        2,
        "tlas pack+build derive exactly TWO buffer barriers (ring RAW at pack + array RAW at build)"
    );
}

/// HW-RT rung R2a-3: the tlas-OFF path adds ZERO new barriers — no pass declares a
/// `buffer_access` on `tlas_instances`, so the graph routes zero barriers naming it (the
/// `optional_passes_are_additive` invariant for the R2a-3 resource).
#[cfg(feature = "hwrt")]
#[test]
fn tlas_off_path_zero_new_barriers() {
    let mut g = FrameGraph::with_capacity(8, 8, 32);
    let albedo = g.add_image("albedo");
    let normal = g.add_image("normal");
    let material = g.add_image("material");
    let depth = g.add_image("depth");
    // `tlas_instances` declared (fixed ResId, as in `declare_deferred_graph`) but NEVER accessed
    // (tlas off ⇒ no pack/build pass).
    let _tlas_instances = g.add_buffer("tlas_instances");

    g.add_pass("raster");
    for &c in &[albedo, normal, material] {
        g.image_access(c, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, SubRange::COLOR);
    }
    g.image_access(depth, FRAG, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, SubRange::DEPTH);

    g.compile();
    assert_eq!(
        g.buf_barriers().len(),
        0,
        "tlas-OFF path (no pack/build access on tlas_instances) emits zero buffer barriers"
    );
}

/// HW-RT rung R2a-3 (RISK-2 regression pin): a FAITHFUL MIRROR of `declare_deferred_graph`'s
/// buffer declaration order under `hwrt` + interp ON + tlas ON, asserting the exact ResId → sink
/// slot mapping the [`GbufferBarrierSink`](graph_bridge) resolves by (`sink_slot = ResId - 18`,
/// the FRAMEGRAPH_IMAGE_COUNT offset under hwrt after Rung 3a's two shadow-vis + Rung 3b's three
/// temporal images + Rung 3b C1/H2's `shadow_temporal_hist_read` sibling/read image + textured-PBR
/// T6a's `pbr`). This pins `tlas_instances` to ResId 25 → slot 7 and the SHIFTED interp trio to
/// 26/27/28 → slots 8/9/10, so a future ResId insertion cannot silently route a barrier to the
/// wrong physical buffer (we have no validation layer on this box). The +7 absolute-ResId shift is
/// the intended consequence of the denoise images + `pbr`; the SINK SLOTS stay put because the
/// offset re-bases by the same const.
///
/// The mirror declares 18 placeholder IMAGES first (the ResId 0..17 the real graph consumes under
/// hwrt, so the buffers start at ResId 18), then the buffers in `declare_deferred_graph`'s EXACT
/// order: light_table..alloc (18..22), ddgi_classification/ray_table (23/24), tlas_instances (25,
/// unconditional under hwrt), then the interp trio (26/27/28). The sink `buffers` array positions
/// (graph_bridge.rs `record_graph_pass`) MUST match this: slot 5=ddgi_class, 6=ddgi_ray,
/// 7=tlas_instances, 8=interp_pairs, 9=interp_out_slot, 10=interp_model_out.
#[cfg(feature = "hwrt")]
#[test]
fn hwrt_resid_18_sink_slot_mapping_pinned() {
    // The sink's fixed image count (graph_bridge.rs `FRAMEGRAPH_IMAGE_COUNT`, `pub(crate)` — mirror
    // its value here; if it changes the real sink offset changes too and this pin must be revisited).
    // Rung 3a bumped it to 13 (shadow_vis/shadow_vis2 at ResId 11/12); Rung 3b to 16 (motion_vec /
    // shadow_temporal_hist / temporal_out at ResId 13/14/15); Rung 3b C1/H2 to 17
    // (shadow_temporal_hist_read — the cross-frame sibling/READ image — at ResId 16); textured-PBR
    // T6a to 18 (`pbr` — the `gPbr` deferred-resolve MRT lane — declared LAST, at ResId 17).
    const IMAGE_COUNT: usize = 18;
    let sink_slot = |r: ResId| r.index() - IMAGE_COUNT;

    let mut g = FrameGraph::with_capacity(20, 8, 32);
    // 18 placeholder images (ResIds 0..=17) under hwrt, matching the real graph's image span so the
    // buffers begin at ResId 18 exactly as in `declare_deferred_graph` (Rung 3a added shadow_vis /
    // shadow_vis2, Rung 3b added motion_vec / shadow_temporal_hist / temporal_out, Rung 3b C1/H2
    // added shadow_temporal_hist_read, textured-PBR T6a added `pbr` — all LAST in the image block,
    // before the first add_buffer).
    for name in [
        "albedo", "normal", "material", "depth", "viewt", "lit", "ssao", "cascade", "atlas",
        "ddgi_irr", "ddgi_depth", "shadow_vis", "shadow_vis2", "motion_vec", "shadow_temporal_hist",
        "temporal_out", "shadow_temporal_hist_read", "pbr",
    ] {
        g.add_image(name);
    }
    // Buffers in `declare_deferred_graph`'s EXACT order.
    let light_table = g.add_buffer("light_table");
    let _tiles = g.add_buffer("tiles");
    let _grid = g.add_buffer("grid");
    let _index = g.add_buffer("index");
    let alloc = g.add_buffer("alloc");
    let ddgi_classification = g.add_buffer("ddgi_classification");
    let ddgi_ray_table = g.add_buffer("ddgi_ray_table");
    // R2a-3: tlas_instances declared UNCONDITIONALLY under hwrt, BEFORE the conditional interp trio.
    let tlas_instances = g.add_buffer("tlas_instances");
    // Interp trio (interp ON), shifted by the tlas_instances insertion.
    let interp_pairs = g.add_buffer("interp_pairs");
    let interp_out_slot = g.add_buffer("interp_out_slot");
    let interp_model_out = g.add_buffer("interp_model_out");

    // The absolute ResIds the real graph assigns under hwrt+interp+tlas — all shifted +7 by the
    // Rung 3a shadow-vis (11/12) + Rung 3b temporal (13/14/15) + Rung 3b C1/H2
    // shadow_temporal_hist_read (16) + textured-PBR T6a `pbr` (17) images, while the SINK SLOTS
    // below stay put (the point).
    assert_eq!(light_table.index(), 18, "light_table ResId");
    assert_eq!(alloc.index(), 22, "alloc ResId");
    assert_eq!(ddgi_classification.index(), 23, "ddgi_classification ResId");
    assert_eq!(ddgi_ray_table.index(), 24, "ddgi_ray_table ResId");
    assert_eq!(tlas_instances.index(), 25, "tlas_instances ResId 25 (fixed under hwrt after 7 denoise+pbr images)");
    assert_eq!(interp_pairs.index(), 26, "interp trio shifts to 26 under hwrt+denoise+pbr images");
    assert_eq!(interp_out_slot.index(), 27, "interp_out_slot ResId under hwrt+denoise+pbr images");
    assert_eq!(interp_model_out.index(), 28, "interp_model_out ResId under hwrt+denoise+pbr images");

    // The sink slot each ResId resolves to (`sink.buffers[ResId - 18]` in `record_graph_pass`) —
    // UNCHANGED by the +7 image shift because the offset re-bases by the same FRAMEGRAPH_IMAGE_COUNT.
    assert_eq!(sink_slot(ddgi_classification), 5, "ddgi_classification → sink slot 5");
    assert_eq!(sink_slot(ddgi_ray_table), 6, "ddgi_ray_table → sink slot 6");
    assert_eq!(sink_slot(tlas_instances), 7, "tlas_instances → sink slot 7 (the R2a-3 pin)");
    assert_eq!(sink_slot(interp_pairs), 8, "interp_pairs → sink slot 8 (shifted)");
    assert_eq!(sink_slot(interp_out_slot), 9, "interp_out_slot → sink slot 9 (shifted)");
    assert_eq!(sink_slot(interp_model_out), 10, "interp_model_out → sink slot 10 (shifted)");
    // The hwrt sink `buffers` array is [VkBuffer; 11] (slots 0..=10) — the tlas_instances slot 7
    // is inside it, and the shifted interp trio (8/9/10) are the last three slots.
    assert_eq!(sink_slot(interp_model_out), 10, "the hwrt sink array's last slot index is 10");
}

// ===========================================================================
// `compile()`'s DEBUG-ONLY unwritten-transient-read authoring guard.
//
// A mis-authored pass that READS a transient (non-seeded) resource with no prior
// producer/seed would otherwise silently derive a hazard-free `TOP_OF_PIPE`
// barrier — a whole class of authoring regressions going uncaught. `compile`
// tracks a per-`(ResId, mip)` written-or-seeded bit and `debug_assert!`s it holds
// before a non-seeded transient resource's first read.
//
// VG R3 P2-8: the guard covers BUFFERS as well as images. It used to read
// `!is_image || ..`, and P2-7 measured what that cost — deleting
// `vb_indirect_late_upload`'s declared TRANSFER_WRITE from `declare_vb_graph`,
// while the recorder still filled the buffer and `vb_raster_late` still fetched
// from it, left the golden, the recorder probe, validation AND the barrier-stream
// pin all GREEN. The discriminator is now the DECLARED PROVENANCE (`add_buffer`
// versus `add_buffer_seeded`), not the resource kind: a host-filled buffer
// legitimately has no in-graph producer, which is why dropping the kind test
// blanket-wise (measured) reds `interp_pairs`.
// ===========================================================================

/// A pass reads a transient image that no prior pass wrote and that was never
/// declared via `add_image_seeded` — the guard must fire.
///
/// `cfg(debug_assertions)`: the guard IS a `debug_assert!`, so in a release test binary `compile`
/// correctly does not panic and a `should_panic` test reports a failure that is not one. CI runs a
/// debug × release matrix, so the debug leg still gates this.
///
/// ⚠️ This attribute was MISSING, and its absence was measured rather than reasoned about: CI runs
/// `cargo test --workspace --all-targets --release` (`.github/workflows/ci.yml:62`, `:103`), so
/// this test and its neighbour below have been FAILING the release leg. The rule was already
/// written down on the subresource-guard fixtures that carried both the attribute and this exact
/// rationale — so what was missing was not the knowledge but the gate on two tests that were added
/// later. (Those fixtures no longer need the attribute: VG R3 P1-5a turned them into assertions on
/// the DERIVED BARRIERS, which are release-live. This one still does — it gates a `debug_assert!`.)
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "reads UNWRITTEN transient image")]
fn compile_panics_on_unwritten_transient_image_read() {
    let mut g = FrameGraph::with_capacity(4, 4, 4);
    let orphan = g.add_image("orphan");

    g.add_pass("consumer_only");
    g.image_access(
        orphan,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
        VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        SubRange::COLOR,
    );

    g.compile();
}

/// A correctly-authored producer→consumer pair (a prior pass writes the image
/// before any pass reads it) must NOT trip the guard.
#[test]
fn compile_does_not_panic_when_producer_writes_before_read() {
    let mut g = FrameGraph::with_capacity(4, 4, 4);
    let img = g.add_image("produced");

    g.add_pass("producer");
    g.image_access(
        img,
        VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        SubRange::COLOR,
    );

    g.add_pass("consumer");
    g.image_access(
        img,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
        VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        SubRange::COLOR,
    );

    g.compile();
}

/// A NON-RINGED, content-persistent seeded image (mirroring the shadow-temporal
/// history pool / DDGI atlas: `add_image_seeded` + `seeded_writer_at_layout`,
/// the sibling in-flight frame's undrained write) read FIRST, with no in-frame
/// producer, must NOT trip the guard — cross-frame content is intentional for
/// a seeded resource.
#[test]
fn compile_does_not_panic_on_seeded_resource_read_first() {
    let mut g = FrameGraph::with_capacity(4, 4, 4);
    let history = g.add_image_seeded(
        "shadow_temporal_hist_read",
        ResSync::seeded_writer_at_layout(
            VK_IMAGE_LAYOUT_GENERAL,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
        ),
    );

    g.add_pass("reader_only");
    g.image_access(
        history,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
        VK_IMAGE_LAYOUT_GENERAL,
        SubRange::COLOR,
    );

    g.compile();
}

/// VG R3 P2-8 — THE BUFFER ARM, and the exact shape P2-7 measured every gate blind to.
///
/// A pass declares an indirect FETCH from a transient buffer that no pass declared a write
/// to, and that was not declared with `add_buffer_seeded`. This is `vb_raster_late` reading
/// `vb_indirect_late` with `vb_indirect_late_upload`'s TRANSFER_WRITE deleted — the
/// production defect that left the `[vb_occ_split]` golden, the recorder probe, validation
/// and the barrier-stream pin ALL GREEN. The guard must fire.
///
/// `cfg(debug_assertions)` for the same reason its image sibling carries it: the guard IS a
/// `debug_assert!`, so in a release test binary `compile` correctly does not panic and a
/// `should_panic` test would report a failure that is not one. CI runs a debug × release
/// matrix, so the debug leg gates this.
///
/// The `expected` substring names the BUFFER arm specifically — the image arm's message says
/// `transient image`, so neither fixture can be satisfied by the other's fire, and no
/// unrelated panic (a bounds check, an arena assert) carries this phrase either.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "reads UNWRITTEN transient buffer")]
fn compile_panics_on_unwritten_transient_buffer_read() {
    let mut g = FrameGraph::with_capacity(4, 4, 4);
    let indirect = g.add_buffer("vb_indirect_late");

    g.add_pass("vb_raster_late");
    g.buffer_access(
        indirect,
        VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    );

    g.compile();
}

/// The same two-pass graph with the declared writer PRESENT — the shipped
/// `vb_indirect_late_upload` → `vb_raster_late` pair — must not trip the guard.
///
/// It asserts the DERIVED BARRIER rather than merely "no panic", which is what makes it
/// non-vacuous: a fixture that only demanded silence would still pass if the guard were
/// deleted outright. The pinned edge is `TRANSFER/TRANSFER_WRITE → DRAW_INDIRECT/
/// INDIRECT_COMMAND_READ`, i.e. a src half that makes the fill AVAILABLE — the precise field
/// the defect replaces with `(TOP_OF_PIPE, 0)`. Silence and a real barrier, together.
#[test]
fn compile_does_not_panic_when_a_pass_writes_the_buffer_before_the_read() {
    let mut g = FrameGraph::with_capacity(4, 4, 4);
    let indirect = g.add_buffer("vb_indirect_late");

    g.add_pass("vb_indirect_late_upload");
    g.buffer_access(indirect, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT);

    g.add_pass("vb_raster_late");
    g.buffer_access(
        indirect,
        VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    );

    g.compile();

    assert!(
        has_buf(
            g.buf_barriers(),
            indirect,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
            VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
        ),
        "a declared writer must derive the TRANSFER(WRITE)→DRAW_INDIRECT(FETCH) RAW that makes \
         the fill available — the exact src half the unwritten-read defect drops to (TOP_OF_PIPE, 0)"
    );
    assert_eq!(
        g.buf_barriers().len(),
        1,
        "exactly one buffer barrier: the first-touch WRITE needs none (nothing to order against), \
         the read needs the RAW"
    );
}

/// A buffer whose content comes from OUTSIDE the graph — the host-filled instance ring
/// (`interp_pairs`, `vb_instance_ring`) — declared with `add_buffer_seeded` and READ FIRST,
/// with no in-graph producer, must NOT trip the guard. This is the case that proved the
/// exemption load-bearing: dropping the guard's kind test blanket-wise reds it.
///
/// Non-vacuous on the axis that matters for the three production conversions P2-8 made: it
/// pins the derived edge FIELD BY FIELD as `(TOP_OF_PIPE, 0) → (COMPUTE, SHADER_READ)` —
/// verbatim what the bare `add_buffer` spelling derived before the conversion, since the seed
/// VALUE is `ResSync::undefined()` either way. So this fixture asserts the conversion is inert
/// on the barrier stream, not merely that it silences an assert; a seed that actually changed
/// the start state would move `src_stage` here and fail.
#[test]
fn compile_does_not_panic_on_seeded_buffer_read_first() {
    let mut g = FrameGraph::with_capacity(4, 4, 4);
    let ring = g.add_buffer_seeded("vb_instance_ring", ResSync::undefined());

    g.add_pass("vb_batch_cull");
    g.buffer_access(ring, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);

    g.compile();

    assert!(
        has_buf(
            g.buf_barriers(),
            ring,
            VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            0,
            VK_ACCESS_SHADER_READ_BIT,
        ),
        "an `undefined()`-seeded buffer must still derive the SAME first-touch \
         TOP_OF_PIPE→COMPUTE edge the bare `add_buffer` derived — the seeded bit is a \
         provenance declaration, not a state change"
    );
    assert_eq!(
        g.buf_barriers().len(),
        1,
        "the seed adds no barrier of its own"
    );
}

// ===========================================================================
// The MIP axis (VG R3 P1-5a): tracked, not asserted — and the DEBUG-ONLY
// `INVARIANT SUBRESOURCE-LAYER-UNIFORM` guard that is what remains.
//
// The sync state machine keys one `ResSync` per `(ResId, mip)`, so accesses that
// name different mips of one image are tracked separately and there is no single
// layout for them to disagree about: the three fixtures below USED to assert a
// panic on exactly that shape and now assert the derived barrier list instead.
// Layers are still uniform-by-requirement — one `ResSync` block covers all of a
// resource's layers — so that axis keeps its guard, and these tests pin that the
// shipped layered declarations (CSM cascades, the punctual atlas, the DDGI probe
// atlases: one layer span, several layouts) are on the permitted side.
// ===========================================================================

/// `SubRange::color_mips` is a whole-chain COLOR range at layer 0 — the shape a
/// mip-pyramid access declares. Pinned because `image_access`'s release-live range check reads
/// `base_mip`/`mip_count`, the layer guard reads `base_layer`/`layer_count`, and the derived
/// barrier carries the aspect and the layer span verbatim.
#[test]
fn subrange_color_mips_spans_the_whole_chain_at_one_layer() {
    let m = SubRange::color_mips(5);
    assert_eq!(m.aspect, SubRange::COLOR.aspect, "color_mips must use the COLOR aspect");
    assert_eq!(m.base_mip, 0);
    assert_eq!(m.mip_count, 5);
    assert_eq!(m.base_layer, 0);
    assert_eq!(m.layer_count, 1);
    // `mip_count = 1` degenerates to the existing single-mip COLOR range, so the
    // constructor is a strict generalization and cannot perturb existing declarations.
    assert_eq!(SubRange::color_mips(1), SubRange::COLOR);
}

/// ⚠️ THIS TEST ONCE ASSERTED A PANIC. It is the same reversal, on a second fixture, that
/// `compile_allows_two_mip_spans_on_one_resource` records in full — read that one for the
/// discriminator between "the machine answers the question" and "the condition was widened".
///
/// The declaration is unchanged in substance: a whole 4-level chain written at GENERAL, then
/// mip 0 ALONE sampled at SHADER_READ_ONLY_OPTIMAL — the exact shape an HZB build/consume pair
/// takes. Under the per-`ResId` machine one tracked layout would have claimed
/// SHADER_READ_ONLY_OPTIMAL for mips 1..3, which nothing had transitioned, so
/// `INVARIANT HZB-SUBRESOURCE-UNIFORM` fired. Under the `(ResId, mip)` machine the mips hold
/// their own states, and THAT is what is asserted here: two barriers, the second covering mip
/// 0 alone rather than the chain, and `resolved_layout_mip` reporting two different layouts on
/// one image.
///
/// No `cfg(debug_assertions)`: the assertions are on the DERIVED barriers, which are
/// release-live, so this now gates both legs of CI's matrix instead of only the debug one.
#[test]
fn compile_tracks_a_distinct_layout_per_mip() {
    let mut g = FrameGraph::with_capacity(4, 4, 4);
    let pyramid = g.add_image_mipped("pyramid", 4, ResSync::undefined());

    g.add_pass("pyramid_build");
    g.image_access(
        pyramid,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_WRITE_BIT,
        VK_IMAGE_LAYOUT_GENERAL,
        SubRange::color_mips(4),
    );

    g.add_pass("pyramid_sample_mip0");
    g.image_access(
        pyramid,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
        VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        SubRange::COLOR,
    );

    g.compile();

    let img = g.img_barriers();
    assert_eq!(img.len(), 2, "one first-touch write barrier + one RAW on mip 0");
    // The build: all four mips are in the same (fresh) state, derive the same transition, and
    // MERGE into the single whole-chain barrier the per-ResId machine also emitted.
    assert_eq!(
        img[0],
        ImgBarrier {
            res: pyramid,
            src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
            dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            src_access: 0,
            dst_access: VK_ACCESS_SHADER_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: VK_IMAGE_LAYOUT_GENERAL,
            subresource: SubRange::color_mips(4),
        },
        "the uniform chain must still merge into ONE barrier over [0, 4)"
    );
    // The consumer: mip 0 ALONE — the span it declared, not the chain. This is the widening
    // that used to be unrepresentable and is the reason the old guard existed.
    assert_eq!(
        img[1],
        ImgBarrier {
            res: pyramid,
            src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            src_access: VK_ACCESS_SHADER_WRITE_BIT,
            dst_access: VK_ACCESS_SHADER_READ_BIT,
            old_layout: VK_IMAGE_LAYOUT_GENERAL,
            new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            subresource: SubRange::COLOR,
        },
        "the mip-0 read must transition mip 0 ONLY, from ITS layout"
    );
    // The state the barriers were derived from: one image, two layouts.
    assert_eq!(
        g.resolved_layout_mip(pyramid, 0),
        VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        "mip 0 was sampled last"
    );
    for m in 1..4 {
        assert_eq!(
            g.resolved_layout_mip(pyramid, m),
            VK_IMAGE_LAYOUT_GENERAL,
            "mip {m} was never sampled and must still be GENERAL"
        );
    }
}

/// ⚠️ THIS TEST ONCE ASSERTED A PANIC (see `compile_allows_two_mip_spans_on_one_resource` for
/// the full record of the two reversals this axis has taken).
///
/// Its shape is the one no PAIRWISE comparison catches: access 2 differs from access 1 only in
/// layout, access 3 only in span, so no single pair differs on both axes while the resource as
/// a whole varies on both. Under the per-`ResId` machine that made the tracked layout a lie and
/// the accumulating guard fired. Under the `(ResId, mip)` machine the third access moves mip 0
/// back to GENERAL while mips 1..3 stay SHADER_READ_ONLY_OPTIMAL — three barriers, the last one
/// over mip 0 alone — and that trajectory is what is pinned here.
///
/// The pin that matters is barrier [2]'s span: a machine that widened it to the chain would
/// transition mips 1..3 out of the layout their last reader left them in, which is precisely
/// the bug the old assert was standing in for.
#[test]
fn compile_derives_per_mip_barriers_when_the_variation_straddles_three_accesses() {
    let mut g = FrameGraph::with_capacity(4, 4, 4);
    let pyramid = g.add_image_mipped("pyramid", 4, ResSync::undefined());

    g.add_pass("build_whole_chain");
    g.image_access(
        pyramid,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_WRITE_BIT,
        VK_IMAGE_LAYOUT_GENERAL,
        SubRange::color_mips(4),
    );

    g.add_pass("read_whole_chain_other_layout");
    g.image_access(
        pyramid,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
        VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        SubRange::color_mips(4),
    );

    g.add_pass("read_mip0_first_layout");
    g.image_access(
        pyramid,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
        VK_IMAGE_LAYOUT_GENERAL,
        SubRange::COLOR,
    );

    g.compile();

    let img = g.img_barriers();
    assert_eq!(img.len(), 3, "first-touch write, whole-chain RAW, then mip 0's layout flip");
    assert_eq!(
        img[0],
        ImgBarrier {
            res: pyramid,
            src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
            dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            src_access: 0,
            dst_access: VK_ACCESS_SHADER_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: VK_IMAGE_LAYOUT_GENERAL,
            subresource: SubRange::color_mips(4),
        },
        "the uniform chain's first touch must merge into ONE barrier"
    );
    assert_eq!(
        img[1],
        ImgBarrier {
            res: pyramid,
            src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            src_access: VK_ACCESS_SHADER_WRITE_BIT,
            dst_access: VK_ACCESS_SHADER_READ_BIT,
            old_layout: VK_IMAGE_LAYOUT_GENERAL,
            new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            subresource: SubRange::color_mips(4),
        },
        "all four mips are still in step, so the RAW must merge back into ONE barrier"
    );
    // src_access 0: the mips' pending write was already flushed by barrier [1], so this is the
    // WAR/execution-only dependency on the prior readers — a layout flip, not a memory one.
    assert_eq!(
        img[2],
        ImgBarrier {
            res: pyramid,
            src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            src_access: 0,
            dst_access: VK_ACCESS_SHADER_READ_BIT,
            old_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            new_layout: VK_IMAGE_LAYOUT_GENERAL,
            subresource: SubRange::COLOR,
        },
        "mip 0's layout flip must cover mip 0 ONLY — widening it would drag mips 1..3 out of \
         the layout their reader left them in"
    );
    assert_eq!(g.resolved_layout_mip(pyramid, 0), VK_IMAGE_LAYOUT_GENERAL, "mip 0 read last at GENERAL");
    for m in 1..4 {
        assert_eq!(
            g.resolved_layout_mip(pyramid, m),
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            "mip {m} was last read at SHADER_READ_ONLY_OPTIMAL"
        );
    }
}

/// ⚠️ THIS FIXTURE HAS REVERSED ITS VERDICT TWICE, and a reader who sees only the latest flip
/// cannot tell it apart from the thing `graph.rs` forbids by name ("never to relax the
/// condition until it goes quiet"). So both reversals are recorded here, with the
/// discriminator.
///
/// **Reversal 1 — GREEN → RED, and it STRENGTHENED an unsound guard.** The first version was
/// named `compile_allows_a_varying_span_at_a_uniform_layout` and it PASSED: the guard's
/// original condition permitted a varying span as long as every access declared one layout, on
/// the reasoning that "one layout is true of every subresource". That reasoning is false. A
/// uniform DECLARED layout does not give a uniform ACTUAL one — only the union of spans that
/// actually appeared in an emitted barrier has been transitioned, and every other subresource
/// is still in the image's start layout. Reverse the two passes below and the old guard still
/// passed while the frame was UB: the subset-first order transitions mips [0,1) only, then the
/// superset access emits a barrier over [0,4) claiming `oldLayout = GENERAL` for mips 1..3,
/// which are UNDEFINED (VUID-VkImageMemoryBarrier-oldLayout-01197). A guard whose verdict
/// depends on declaration order is not a guard, so the condition was strengthened to
/// span-uniformity and this fixture became the RED case.
///
/// **Reversal 2 — RED → GREEN, and it REMOVED THE GUARD'S NEED on this axis by tracking it.**
/// The panic message reversal 1 installed pointed at the fix by name: "give this resource
/// per-subresource sync state, not … make the declarations agree by hand", and the comment
/// beside it called this assert the TRIGGER for that work. VG R3 P1-5a did the work. Sync state
/// is now keyed `(ResId, mip)` — one `ResSync` per level, located by `ResShape::state_base` —
/// so mips 1..3 keep their own layouts and nothing can misdescribe them.
///
/// **The discriminator**, because both "build the machine" and "widen the condition" end with
/// this fixture green: the assertions below are on the DERIVED BARRIERS, not on the absence of
/// a panic. A widened condition would leave the mip-0 read emitting the old whole-chain
/// barrier under one tracked layout; this test requires `mip_count == 1` on it, and its
/// siblings require two different layouts to coexist on one image. Green here is a claim about
/// what the machine derives, which is not a claim silence could make.
///
/// No `cfg(debug_assertions)`: derived barriers are release-live, so both legs gate it now.
#[test]
fn compile_allows_two_mip_spans_on_one_resource() {
    let mut g = FrameGraph::with_capacity(4, 4, 4);
    let pyramid = g.add_image_mipped("pyramid", 4, ResSync::undefined());

    g.add_pass("build_whole_chain");
    g.image_access(
        pyramid,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_WRITE_BIT,
        VK_IMAGE_LAYOUT_GENERAL,
        SubRange::color_mips(4),
    );

    g.add_pass("read_mip0");
    g.image_access(
        pyramid,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
        VK_IMAGE_LAYOUT_GENERAL,
        SubRange::COLOR,
    );

    g.compile();

    let img = g.img_barriers();
    assert_eq!(img.len(), 2, "the whole-chain first touch, then the mip-0 RAW");
    assert_eq!(
        img[0],
        ImgBarrier {
            res: pyramid,
            src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
            dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            src_access: 0,
            dst_access: VK_ACCESS_SHADER_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: VK_IMAGE_LAYOUT_GENERAL,
            subresource: SubRange::color_mips(4),
        },
        "the whole-chain write must merge into ONE barrier over [0, 4)"
    );
    // No layout change here (GENERAL → GENERAL): a pure RAW flush of the write to mip 0. Its
    // span is [0, 1) — the SUBSET the reader declared, not the chain the writer did.
    assert_eq!(
        img[1],
        ImgBarrier {
            res: pyramid,
            src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            src_access: VK_ACCESS_SHADER_WRITE_BIT,
            dst_access: VK_ACCESS_SHADER_READ_BIT,
            old_layout: VK_IMAGE_LAYOUT_GENERAL,
            new_layout: VK_IMAGE_LAYOUT_GENERAL,
            subresource: SubRange::COLOR,
        },
        "the mip-0 read's barrier must cover mip 0 ONLY"
    );
    for m in 0..4 {
        assert_eq!(
            g.resolved_layout_mip(pyramid, m),
            VK_IMAGE_LAYOUT_GENERAL,
            "every mip ends GENERAL here; only mip 0's PENDING WRITE was flushed"
        );
    }
}

/// ⚠️ **THE MEASURED BUG, asserted.** This is the real HZB build shape at a 512×512 render
/// extent — `levels = 10`, two passes — and it is the exact graph that could not be declared
/// before VG R3 P1-5a.
///
/// Pass 0 writes mips `[0, 6)`. Pass 1 reads mip 5 (which pass 0 wrote) and writes mips `[6, 10)`.
///
/// **What the OLD per-`ResId` machine derived, traced in release before the change:** pass 0's
/// first touch transitions `[0, 6)` only, so mips 6..9 are still `UNDEFINED` — but `state` records
/// one layout for the whole `ResId`, so it believes the image is `GENERAL`. Pass 1's write then
/// finds `layout_change == false` and emits a barrier with `old_layout == new_layout == GENERAL`.
/// **Mips 6..9 are never transitioned**, while the dispatch writes them through storage descriptors
/// declared `GENERAL`. Reachable at every extent with `prev_pow2(max(W, H)) >= 64`, invisible to
/// every golden pin, and a well-formed barrier as far as the validation layers can see.
///
/// The third assertion below — `old_layout: UNDEFINED` on the `[6, 4)` span — is that bug, stated
/// as the thing the machine must now get right. Nothing else in this file would catch it.
#[test]
fn compile_derives_the_hzb_build_chain_at_a_real_extent() {
    // `levels = 10`, `HZB_LEVELS_PER_PASS = 6`, so `pass_count = 2` — the numbers step P1-4's own
    // corruption control reported from the engine (`sets built = 4, pass_count = 2, levels = 10`).
    let mut g = FrameGraph::with_capacity(4, 4, 8);
    let pyramid = g.add_image_mipped("hzb", 10, ResSync::undefined());

    let mip = |base_mip: u32, mip_count: u32| SubRange {
        aspect: VK_IMAGE_ASPECT_COLOR_BIT,
        base_mip,
        mip_count,
        base_layer: 0,
        layer_count: 1,
    };

    g.add_pass("hzb_build_0");
    g.image_access(
        pyramid,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_WRITE_BIT,
        VK_IMAGE_LAYOUT_GENERAL,
        mip(0, 6),
    );

    g.add_pass("hzb_build_1");
    // The reduce pass reads mip `d - 1` …
    g.image_access(
        pyramid,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
        VK_IMAGE_LAYOUT_GENERAL,
        mip(5, 1),
    );
    // … and writes mips `[d, d + n)` — the SECOND span on the same ResId in the same pass, which
    // is what `INVARIANT HZB-SUBRESOURCE-UNIFORM` used to refuse by name.
    g.image_access(
        pyramid,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_WRITE_BIT,
        VK_IMAGE_LAYOUT_GENERAL,
        mip(6, 4),
    );

    g.compile();

    let img = g.img_barriers();
    assert_eq!(img.len(), 3, "first touch of [0,6), the mip-5 RAW, then the first touch of [6,10)");
    assert_eq!(
        img[0],
        ImgBarrier {
            res: pyramid,
            src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
            dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            src_access: 0,
            dst_access: VK_ACCESS_SHADER_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: VK_IMAGE_LAYOUT_GENERAL,
            subresource: mip(0, 6),
        },
        "pass 0's six mips are all in the same state, so they MERGE into one barrier"
    );
    assert_eq!(
        img[1],
        ImgBarrier {
            res: pyramid,
            src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            src_access: VK_ACCESS_SHADER_WRITE_BIT,
            dst_access: VK_ACCESS_SHADER_READ_BIT,
            old_layout: VK_IMAGE_LAYOUT_GENERAL,
            new_layout: VK_IMAGE_LAYOUT_GENERAL,
            subresource: mip(5, 1),
        },
        "the reduce pass's read of mip 5 is a RAW flush over mip 5 ALONE"
    );
    assert_eq!(
        img[2],
        ImgBarrier {
            res: pyramid,
            src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
            dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            src_access: 0,
            dst_access: VK_ACCESS_SHADER_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: VK_IMAGE_LAYOUT_GENERAL,
            subresource: mip(6, 4),
        },
        "⚠️ THE BUG: mips 6..9 are a FIRST TOUCH and must transition out of UNDEFINED. The old \
         per-ResId machine derived `old_layout == new_layout == GENERAL` here — no transition at \
         all — while the dispatch wrote them through GENERAL storage descriptors"
    );

    // Stated separately because it is the property, not the barrier: every mip the two passes
    // wrote must END in GENERAL, and the machine must be able to answer PER MIP at all.
    for m in 0..10 {
        assert_eq!(
            g.resolved_layout_mip(pyramid, m),
            VK_IMAGE_LAYOUT_GENERAL,
            "mip {m} must end GENERAL — the whole chain was written by one of the two passes"
        );
    }
}

/// Layouts vary but every access declares the SAME layered span — the CSM cascade /
/// punctual atlas shape shipped today (`depth_layers(N)` written at
/// DEPTH_ATTACHMENT_OPTIMAL, then read at SHADER_READ_ONLY_OPTIMAL). One span means
/// one subresource set, so the tracked layout describes it exactly and the guard must
/// stay silent. This is the regression pin for the whole live layered corpus.
#[test]
fn compile_allows_a_varying_layout_at_a_uniform_span() {
    let mut g = FrameGraph::with_capacity(4, 4, 4);
    let cascade = g.add_image_seeded(
        "cascade",
        ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
    );

    g.add_pass("csm_depth");
    g.image_access(
        cascade,
        FRAG,
        VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        SubRange::depth_layers(CASCADE_LAYERS),
    );

    g.add_pass("resolve");
    g.image_access(
        cascade,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
        VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        SubRange::depth_layers(CASCADE_LAYERS),
    );

    g.compile();
}

/// `reset` + re-declare must leave NOTHING of the previous frame behind. A `FrameGraph` is
/// reused every frame, so this fixture declares two different graphs into one `FrameGraph` and
/// pins the second, on the two arenas whose leak is silent:
///
/// 1. **The layer witness** (`res_sub_witness`), per-COMPILE scratch. Graph A pins ResId 0 at
///    a 4-LAYER depth span; graph B's ResId 0 declares a 1-layer color chain. A witness
///    surviving the reset would compare graph B's layer span against graph A's, latch
///    `layers_varied`, and fire `INVARIANT SUBRESOURCE-LAYER-UNIFORM` on a declaration that is
///    in fact layer-uniform. Silence is the discriminating outcome.
///    ⚠️ Graph A used to declare a single-LAYER color attachment, which discriminated on the
///    MIP axis. VG R3 P1-5a made mips TRACKED rather than asserted, so a mip-only difference no
///    longer reaches the witness at all and that spelling would have been vacuous — it would
///    have passed whether the witness leaked or not. The layer span is what the guard still
///    judges, so that is what graph A varies.
/// 2. **The shape arena** (`res_shape` + `res_state_total`), which P1-5a added and which is a
///    prefix sum — the failure mode `reset` is written against. Graph B is MIPPED, so if
///    `res_state_total` survived the reset its pyramid would be handed `state_base = 1` while
///    `compile` sizes the state arena to 4 entries, and mip 3 would index past the end. The
///    `res_state_total()` assertion below names that directly instead of leaving it to a
///    panic-with-no-diagnosis.
#[test]
fn subresource_guard_does_not_leak_across_reset_and_recompile() {
    let mut g = FrameGraph::with_capacity(4, 4, 8);

    // Graph A — ResId 0 is a single-mip, FOUR-LAYER depth target (the CSM cascade shape).
    let a = g.add_image("cascade");
    g.add_pass("csm_depth");
    g.image_access(
        a,
        FRAG,
        VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        SubRange::depth_layers(CASCADE_LAYERS),
    );
    g.compile();

    // Graph B — ResId 0 is now a MIPPED, single-layer pyramid at two layouts: one layer span,
    // so permitted. Only a leaked witness from graph A could make this fire, and only a leaked
    // prefix sum could make it index the wrong entries.
    g.reset();
    let pyramid = g.add_image_mipped("pyramid", 4, ResSync::undefined());
    assert_eq!(
        g.res_state_total(),
        4,
        "reset must clear `res_state_total` with `res_shape`: the pyramid is the FIRST resource \
         of this graph, so the four entries it declares are the whole arena"
    );
    g.add_pass("build_whole_chain");
    g.image_access(
        pyramid,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_WRITE_BIT,
        VK_IMAGE_LAYOUT_GENERAL,
        SubRange::color_mips(4),
    );
    g.add_pass("read_whole_chain");
    g.image_access(
        pyramid,
        VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
        VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        SubRange::color_mips(4),
    );
    g.compile();

    // Two hazards: the first-touch UNDEFINED→GENERAL write, then the RAW read. Both merge over
    // the whole chain, because every mip of a freshly declared pyramid moves in step.
    let img = g.img_barriers();
    assert_eq!(img.len(), 2);
    assert_eq!(
        img[0],
        ImgBarrier {
            res: pyramid,
            src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
            dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            src_access: 0,
            dst_access: VK_ACCESS_SHADER_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: VK_IMAGE_LAYOUT_GENERAL,
            subresource: SubRange::color_mips(4),
        },
        "a stale `state_base` that happened to stay IN BOUNDS would show up here as the wrong \
         `old_layout` — the failure the length check alone cannot see"
    );
    assert_eq!(
        img[1],
        ImgBarrier {
            res: pyramid,
            src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            src_access: VK_ACCESS_SHADER_WRITE_BIT,
            dst_access: VK_ACCESS_SHADER_READ_BIT,
            old_layout: VK_IMAGE_LAYOUT_GENERAL,
            new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            subresource: SubRange::color_mips(4),
        }
    );
}

// ===========================================================================
// VG R3 P1-5a commit C1 — the BASELINE barrier-stream pin.
//
// Step P1-5a re-keys the framegraph's sync state from `ResId` to `(ResId, mip)`.
// Its central claim is that every path that exists TODAY keeps emitting a
// BYTE-IDENTICAL barrier stream — not "an equivalent one". Nothing above can
// check that: the count pin cannot see a reordering, the membership pins cannot
// see a redundant barrier, and no golden image can see either (a barrier stream
// differs from a correct one in ways an 8-bit framebuffer never renders — a
// missing hazard has to both materialise on this machine's scheduler AND survive
// quantisation before a pixel moves).
//
// So the stream is pinned FIRST, on the UNMODIFIED tree. Authoring this pin after
// the re-key would certify the NEW behaviour under the old name, which is the
// false-fresh trap: the numbers would agree with the code because they were read
// off it.
//
// This section adds no behaviour. It is a generator, a pin, and the tables both
// share.
// ===========================================================================

/// The passes [`build_maximal_frame`] declares, in `add_pass` order.
///
/// `FrameGraph` exposes no pass-name accessor (`pass_barriers()` returns bare index
/// ranges), so the dumper and the divergence report read pass names from here. The pin
/// asserts this table's length against `pass_barriers().len()`, so a pass added to the
/// frame without a row here fails loudly instead of silently mislabelling every later
/// entry in a failure report.
const MAXIMAL_FRAME_PASS_NAMES: &[&str] = &[
    "raster",
    "light_upload",
    "coarse_cull",
    "marcher",
    "ssao",
    "light_cull",
    "csm_depth",
    "atlas_depth",
    "resolve",
    "present_draw",
    "present_transition",
];

/// Single-BIT `VkPipelineStageFlags` → the constant name, in ascending bit order.
///
/// ONLY constants `use`d at the top of this file may appear here: the dumper emits these
/// names verbatim into text that must COMPILE when pasted, and the table is the compiler's
/// own witness of that — a name not in scope fails to build this table, not the paste.
/// Ascending order makes a multi-bit mask render the same way every run, so a diff of two
/// dumps is a diff of the stream and not of the formatter.
const STAGE_BITS: &[(u32, &str)] = &[
    (VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, "VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT"),
    (VK_PIPELINE_STAGE_VERTEX_SHADER_BIT, "VK_PIPELINE_STAGE_VERTEX_SHADER_BIT"),
    (VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, "VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT"),
    (VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT, "VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT"),
    (VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT, "VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT"),
    (VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, "VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT"),
    (VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, "VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT"),
    (VK_PIPELINE_STAGE_TRANSFER_BIT, "VK_PIPELINE_STAGE_TRANSFER_BIT"),
    (VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT, "VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT"),
];

/// Single-BIT `VkAccessFlags` → the constant name, in ascending bit order (see [`STAGE_BITS`]).
const ACCESS_BITS: &[(u32, &str)] = &[
    (VK_ACCESS_SHADER_READ_BIT, "VK_ACCESS_SHADER_READ_BIT"),
    (VK_ACCESS_SHADER_WRITE_BIT, "VK_ACCESS_SHADER_WRITE_BIT"),
    (VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, "VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT"),
    (VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, "VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT"),
    (VK_ACCESS_TRANSFER_WRITE_BIT, "VK_ACCESS_TRANSFER_WRITE_BIT"),
];

/// Single-BIT `VkImageAspectFlags` → the constant name (see [`STAGE_BITS`]).
const ASPECT_BITS: &[(u32, &str)] = &[
    (VK_IMAGE_ASPECT_COLOR_BIT, "VK_IMAGE_ASPECT_COLOR_BIT"),
    (VK_IMAGE_ASPECT_DEPTH_BIT, "VK_IMAGE_ASPECT_DEPTH_BIT"),
];

/// `VkImageLayout` value → the constant name (see [`STAGE_BITS`]). Layouts are enum-valued,
/// not a bit set, so this is an exact-match table.
const LAYOUT_VALUES: &[(i32, &str)] = &[
    (VK_IMAGE_LAYOUT_UNDEFINED, "VK_IMAGE_LAYOUT_UNDEFINED"),
    (VK_IMAGE_LAYOUT_GENERAL, "VK_IMAGE_LAYOUT_GENERAL"),
    (VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, "VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL"),
    (VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, "VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL"),
    (VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, "VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL"),
    (VK_IMAGE_LAYOUT_PRESENT_SRC_KHR, "VK_IMAGE_LAYOUT_PRESENT_SRC_KHR"),
];

/// A stage/access/aspect mask as a Rust expression: the `|`-joined names of the known bits,
/// plus a `0x…` literal for any bit this file has no name for, and a bare `0` for an empty
/// mask (`dst_access: 0` on the present transition, `src_access: 0` on every WAR/first touch).
///
/// The hex tail is what keeps the emitter HONEST: an unknown bit is printed rather than
/// dropped, so a pasted expectation still equals the value it was measured from.
fn mask_expr(mask: u32, table: &[(u32, &str)]) -> String {
    if mask == 0 {
        return "0".to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    let mut rest = mask;
    for &(bit, name) in table {
        if rest & bit != 0 {
            parts.push(name.to_string());
            rest &= !bit;
        }
    }
    if rest != 0 {
        parts.push(format!("0x{rest:X}"));
    }
    parts.join(" | ")
}

/// A `VkImageLayout` as a Rust expression: the constant name if this file has one in scope,
/// else the raw literal (which still compiles, and still equals what was measured).
fn layout_expr(layout: i32) -> String {
    LAYOUT_VALUES
        .iter()
        .find(|&&(value, _)| value == layout)
        .map_or_else(|| layout.to_string(), |&(_, name)| name.to_string())
}

/// The resource's declared debug name, quoted — or a loud marker when the `ResId` is outside
/// the frame's declared range.
///
/// `FrameGraph::res_name` PANICS on an out-of-range `ResId`, and the divergence report below
/// labels the EXPECTED side too, whose `ResId` comes from a hand-pasted literal. A panic
/// while formatting a failure message would replace the diagnosis with its own noise.
/// `alloc` is the LAST resource [`build_maximal_frame`] declares, so its index is the bound.
fn res_label(f: &Frame, res: ResId) -> String {
    if res.index() <= f.alloc.index() {
        format!("{:?}", f.g.res_name(res))
    } else {
        format!("<ResId {} is outside this frame's {} resources>", res.0, f.alloc.index() + 1)
    }
}

/// One derived [`ImgBarrier`] as a copy-pasteable Rust struct literal, with the resource name
/// as a trailing comment on the `res` line so a human diff of two dumps reads as prose.
fn img_barrier_source(b: &ImgBarrier, label: &str, index: usize) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "    ImgBarrier {{");
    let _ = writeln!(s, "        res: ResId({}), // [{index}] {label}", b.res.0);
    let _ = writeln!(s, "        src_stage: {},", mask_expr(b.src_stage, STAGE_BITS));
    let _ = writeln!(s, "        dst_stage: {},", mask_expr(b.dst_stage, STAGE_BITS));
    let _ = writeln!(s, "        src_access: {},", mask_expr(b.src_access, ACCESS_BITS));
    let _ = writeln!(s, "        dst_access: {},", mask_expr(b.dst_access, ACCESS_BITS));
    let _ = writeln!(s, "        old_layout: {},", layout_expr(b.old_layout));
    let _ = writeln!(s, "        new_layout: {},", layout_expr(b.new_layout));
    let sub = b.subresource;
    let _ = writeln!(
        s,
        "        subresource: SubRange {{ aspect: {}, base_mip: {}, mip_count: {}, base_layer: {}, layer_count: {} }},",
        mask_expr(sub.aspect, ASPECT_BITS),
        sub.base_mip,
        sub.mip_count,
        sub.base_layer,
        sub.layer_count,
    );
    let _ = writeln!(s, "    }},");
    s
}

/// One derived [`BufBarrier`] as a copy-pasteable Rust struct literal (see
/// [`img_barrier_source`]). Buffers carry no layout and no subresource.
fn buf_barrier_source(b: &BufBarrier, label: &str, index: usize) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "    BufBarrier {{");
    let _ = writeln!(s, "        res: ResId({}), // [{index}] {label}", b.res.0);
    let _ = writeln!(s, "        src_stage: {},", mask_expr(b.src_stage, STAGE_BITS));
    let _ = writeln!(s, "        dst_stage: {},", mask_expr(b.dst_stage, STAGE_BITS));
    let _ = writeln!(s, "        src_access: {},", mask_expr(b.src_access, ACCESS_BITS));
    let _ = writeln!(s, "        dst_access: {},", mask_expr(b.dst_access, ACCESS_BITS));
    let _ = writeln!(s, "    }},");
    s
}

/// One [`PassBarrierRange`] as a copy-pasteable Rust struct literal, labelled with the pass
/// name from [`MAXIMAL_FRAME_PASS_NAMES`].
fn pass_range_source(r: &PassBarrierRange, label: &str, index: usize) -> String {
    format!(
        "    PassBarrierRange {{ img_begin: {}, img_count: {}, buf_begin: {}, buf_count: {} }}, // [{index}] {label}\n",
        r.img_begin, r.img_count, r.buf_begin, r.buf_count,
    )
}

/// The pass name at `index`, or a loud marker when the pin outran
/// [`MAXIMAL_FRAME_PASS_NAMES`] (same reasoning as [`res_label`]: a formatter must not panic
/// while reporting someone else's failure).
fn pass_label(index: usize) -> String {
    MAXIMAL_FRAME_PASS_NAMES
        .get(index)
        .map_or_else(|| format!("<no name for pass {index}>"), |name| format!("{name:?}"))
}

/// **GENERATOR, not a gate** — prints the whole compiled stream of
/// [`build_maximal_frame`] as the three expectation constants below, ready to paste.
///
/// ```text
/// cargo test -p boyko_rhi_vulkan --test framegraph_gbuffer_equiv \
///     dump_maximal_frame_barrier_stream -- --ignored --nocapture
/// ```
///
/// `#[ignore]` because it asserts nothing: it exists so the pin's expectation is MEASURED
/// off a compile rather than predicted by whoever writes the pin. Predicting a barrier
/// stream and then confirming it is a gate wearing a prediction's clothes — the values are
/// read off `compile()` and pasted, and thereafter they say what the graph DOES.
///
/// `--nocapture` is not optional: without it libtest swallows the output and the run looks
/// like a silent pass.
#[test]
#[ignore = "generator, not a gate: prints the pin below as Rust source; the orchestrator runs it"]
fn dump_maximal_frame_barrier_stream() {
    let f = build_maximal_frame();
    let img = f.g.img_barriers();
    let buf = f.g.buf_barriers();
    let passes = f.g.pass_barriers();

    println!("// ===== BEGIN dump_maximal_frame_barrier_stream =====");
    println!(
        "// {} image barriers, {} buffer barriers, {} pass ranges.",
        img.len(),
        buf.len(),
        passes.len()
    );
    println!("// Replace each `const EXPECTED_…` array in framegraph_gbuffer_equiv.rs (each");
    println!("// currently holds one TBD_* sentinel) with the matching block below, KEEPING");
    println!("// the `///` doc comment already above it.");
    println!();

    println!("const EXPECTED_IMG_BARRIERS: &[ImgBarrier] = &[");
    for (i, b) in img.iter().enumerate() {
        print!("{}", img_barrier_source(b, &res_label(&f, b.res), i));
    }
    println!("];");
    println!();

    println!("const EXPECTED_BUF_BARRIERS: &[BufBarrier] = &[");
    for (i, b) in buf.iter().enumerate() {
        print!("{}", buf_barrier_source(b, &res_label(&f, b.res), i));
    }
    println!("];");
    println!();

    println!("const EXPECTED_PASS_BARRIERS: &[PassBarrierRange] = &[");
    for (i, r) in passes.iter().enumerate() {
        print!("{}", pass_range_source(r, &pass_label(i), i));
    }
    println!("];");
    println!("// ===== END dump_maximal_frame_barrier_stream =====");
}

/// **A barrier that has not been MEASURED yet** — the `CENSUS_TBD` shape of
/// `vb_batch_cull_spv_sync.rs`, carried from a scalar to a struct.
///
/// Every field is its type's MAX: `ResId(u16::MAX)` names no resource in any frame graph
/// (the arena is debug-asserted below `u16::MAX`), `u32::MAX` is not a stage/access mask the
/// frame can form, and `i32::MAX` is not a `VkImageLayout`. So it cannot be mistaken for a
/// plausible barrier and cannot satisfy any comparison in the pin — an unfilled baseline
/// fails loudly rather than asserting something convenient, and even with the pin's explicit
/// placeholder guard removed the divergence report would name it (`res_label` renders the
/// `ResId` as out-of-range).
const TBD_IMG_BARRIER: ImgBarrier = ImgBarrier {
    res: ResId(u16::MAX),
    src_stage: u32::MAX,
    dst_stage: u32::MAX,
    src_access: u32::MAX,
    dst_access: u32::MAX,
    old_layout: i32::MAX,
    new_layout: i32::MAX,
    subresource: SubRange {
        aspect: u32::MAX,
        base_mip: u32::MAX,
        mip_count: u32::MAX,
        base_layer: u32::MAX,
        layer_count: u32::MAX,
    },
};

/// The buffer-barrier unfilled sentinel — see [`TBD_IMG_BARRIER`].
const TBD_BUF_BARRIER: BufBarrier = BufBarrier {
    res: ResId(u16::MAX),
    src_stage: u32::MAX,
    dst_stage: u32::MAX,
    src_access: u32::MAX,
    dst_access: u32::MAX,
};

/// The per-pass-range unfilled sentinel — see [`TBD_IMG_BARRIER`]. `u32::MAX` begins no
/// slice into an arena the frame can fill.
const TBD_PASS_RANGE: PassBarrierRange = PassBarrierRange {
    img_begin: u32::MAX,
    img_count: u32::MAX,
    buf_begin: u32::MAX,
    buf_count: u32::MAX,
};

/// **The UNFILLED image-barrier expectation.**
///
/// # Why a placeholder and not a prediction
///
/// The values are read off `dump_maximal_frame_barrier_stream` and pasted. A stream derived
/// by hand from the state machine, or by calling `compile()` a second time inside the pin,
/// asserts only that the code equals itself: both sides would move together under exactly
/// the change this pin exists to catch.
///
/// Once filled these are MEASURED, and the rule the census pins state applies here too — do
/// NOT edit these literals to make a failing run green. They say what the graph emits, and a
/// change in them is a change in the frame's synchronisation.
const EXPECTED_IMG_BARRIERS: &[ImgBarrier] = &[
    ImgBarrier {
        res: ResId(0), // [0] "albedo"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [1] "normal"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [2] "material"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(3), // [3] "depth"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(3), // [4] "depth"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [5] "albedo"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [6] "normal"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [7] "material"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(4), // [8] "viewt"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [9] "normal"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [10] "material"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(4), // [11] "viewt"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(6), // [12] "ssao"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(7), // [13] "cascade"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 4 },
    },
    ImgBarrier {
        res: ResId(8), // [14] "atlas"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 6 },
    },
    ImgBarrier {
        res: ResId(0), // [15] "albedo"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(6), // [16] "ssao"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(7), // [17] "cascade"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 4 },
    },
    ImgBarrier {
        res: ResId(8), // [18] "atlas"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 6 },
    },
    ImgBarrier {
        res: ResId(5), // [19] "lit"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(5), // [20] "lit"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(9), // [21] "swapchain"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(9), // [22] "swapchain"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: 0,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
];

/// **The UNFILLED buffer-barrier expectation** — see [`EXPECTED_IMG_BARRIERS`].
const EXPECTED_BUF_BARRIERS: &[BufBarrier] = &[
    BufBarrier {
        res: ResId(10), // [0] "light_table"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_TRANSFER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(11), // [1] "tiles"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(11), // [2] "tiles"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(14), // [3] "alloc"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_TRANSFER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(14), // [4] "alloc"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(10), // [5] "light_table"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(12), // [6] "grid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(13), // [7] "index"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(12), // [8] "grid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(13), // [9] "index"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
];

/// **The UNFILLED per-pass range expectation** — see [`EXPECTED_IMG_BARRIERS`].
///
/// This is the array a count pin cannot substitute for: the totals can be right while the
/// barriers sit in front of the WRONG passes, which is precisely the failure mode of a
/// re-keyed state machine that flushes a hazard one pass early or late.
const EXPECTED_PASS_BARRIERS: &[PassBarrierRange] = &[
    PassBarrierRange { img_begin: 0, img_count: 4, buf_begin: 0, buf_count: 0 }, // [0] "raster"
    PassBarrierRange { img_begin: 4, img_count: 0, buf_begin: 0, buf_count: 1 }, // [1] "light_upload"
    PassBarrierRange { img_begin: 4, img_count: 1, buf_begin: 1, buf_count: 1 }, // [2] "coarse_cull"
    PassBarrierRange { img_begin: 5, img_count: 4, buf_begin: 2, buf_count: 1 }, // [3] "marcher"
    PassBarrierRange { img_begin: 9, img_count: 4, buf_begin: 3, buf_count: 0 }, // [4] "ssao"
    PassBarrierRange { img_begin: 13, img_count: 0, buf_begin: 3, buf_count: 5 }, // [5] "light_cull"
    PassBarrierRange { img_begin: 13, img_count: 1, buf_begin: 8, buf_count: 0 }, // [6] "csm_depth"
    PassBarrierRange { img_begin: 14, img_count: 1, buf_begin: 8, buf_count: 0 }, // [7] "atlas_depth"
    PassBarrierRange { img_begin: 15, img_count: 5, buf_begin: 8, buf_count: 2 }, // [8] "resolve"
    PassBarrierRange { img_begin: 20, img_count: 2, buf_begin: 10, buf_count: 0 }, // [9] "present_draw"
    PassBarrierRange { img_begin: 22, img_count: 1, buf_begin: 10, buf_count: 0 }, // [10] "present_transition"
];

/// The first index at which two streams differ, INCLUDING a length difference (reported at
/// the end of the shorter one). `None` iff the two are element-for-element equal.
fn first_divergence<T: PartialEq>(actual: &[T], expected: &[T]) -> Option<usize> {
    if let Some(i) = actual.iter().zip(expected.iter()).position(|(a, e)| a != e) {
        return Some(i);
    }
    if actual.len() != expected.len() {
        return Some(actual.len().min(expected.len()));
    }
    None
}

/// The names of the [`ImgBarrier`] fields that differ, so a failure says WHICH axis moved
/// (a widened `subresource` and a reordered stream read very differently, and the reader
/// should not have to diff eight lines by eye to tell them apart).
fn img_field_diffs(a: &ImgBarrier, e: &ImgBarrier) -> String {
    let mut d: Vec<&str> = Vec::new();
    if a.res != e.res {
        d.push("res");
    }
    if a.src_stage != e.src_stage {
        d.push("src_stage");
    }
    if a.dst_stage != e.dst_stage {
        d.push("dst_stage");
    }
    if a.src_access != e.src_access {
        d.push("src_access");
    }
    if a.dst_access != e.dst_access {
        d.push("dst_access");
    }
    if a.old_layout != e.old_layout {
        d.push("old_layout");
    }
    if a.new_layout != e.new_layout {
        d.push("new_layout");
    }
    if a.subresource.aspect != e.subresource.aspect {
        d.push("subresource.aspect");
    }
    if a.subresource.base_mip != e.subresource.base_mip {
        d.push("subresource.base_mip");
    }
    if a.subresource.mip_count != e.subresource.mip_count {
        d.push("subresource.mip_count");
    }
    if a.subresource.base_layer != e.subresource.base_layer {
        d.push("subresource.base_layer");
    }
    if a.subresource.layer_count != e.subresource.layer_count {
        d.push("subresource.layer_count");
    }
    d.join(", ")
}

/// The names of the [`BufBarrier`] fields that differ (see [`img_field_diffs`]).
fn buf_field_diffs(a: &BufBarrier, e: &BufBarrier) -> String {
    let mut d: Vec<&str> = Vec::new();
    if a.res != e.res {
        d.push("res");
    }
    if a.src_stage != e.src_stage {
        d.push("src_stage");
    }
    if a.dst_stage != e.dst_stage {
        d.push("dst_stage");
    }
    if a.src_access != e.src_access {
        d.push("src_access");
    }
    if a.dst_access != e.dst_access {
        d.push("dst_access");
    }
    d.join(", ")
}

/// One [`ImgBarrier`], field by field, each mask as BOTH its raw value and its `VK_*` name.
///
/// The raw value is there because a name table is a claim about the value, and a failure
/// report is the wrong place to trust one.
fn describe_img(f: &Frame, b: &ImgBarrier) -> String {
    let sub = b.subresource;
    let mut s = String::new();
    let _ = writeln!(s, "    res         = ResId({}) {}", b.res.0, res_label(f, b.res));
    let _ = writeln!(s, "    src_stage   = 0x{:08X}  {}", b.src_stage, mask_expr(b.src_stage, STAGE_BITS));
    let _ = writeln!(s, "    dst_stage   = 0x{:08X}  {}", b.dst_stage, mask_expr(b.dst_stage, STAGE_BITS));
    let _ = writeln!(s, "    src_access  = 0x{:08X}  {}", b.src_access, mask_expr(b.src_access, ACCESS_BITS));
    let _ = writeln!(s, "    dst_access  = 0x{:08X}  {}", b.dst_access, mask_expr(b.dst_access, ACCESS_BITS));
    let _ = writeln!(s, "    old_layout  = {:<11} {}", b.old_layout, layout_expr(b.old_layout));
    let _ = writeln!(s, "    new_layout  = {:<11} {}", b.new_layout, layout_expr(b.new_layout));
    let _ = writeln!(
        s,
        "    subresource = aspect 0x{:X} {}, mips [{}, {}), layers [{}, {})",
        sub.aspect,
        mask_expr(sub.aspect, ASPECT_BITS),
        sub.base_mip,
        sub.base_mip + sub.mip_count,
        sub.base_layer,
        sub.base_layer + sub.layer_count,
    );
    s
}

/// One [`BufBarrier`], field by field (see [`describe_img`]).
fn describe_buf(f: &Frame, b: &BufBarrier) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "    res        = ResId({}) {}", b.res.0, res_label(f, b.res));
    let _ = writeln!(s, "    src_stage  = 0x{:08X}  {}", b.src_stage, mask_expr(b.src_stage, STAGE_BITS));
    let _ = writeln!(s, "    dst_stage  = 0x{:08X}  {}", b.dst_stage, mask_expr(b.dst_stage, STAGE_BITS));
    let _ = writeln!(s, "    src_access = 0x{:08X}  {}", b.src_access, mask_expr(b.src_access, ACCESS_BITS));
    let _ = writeln!(s, "    dst_access = 0x{:08X}  {}", b.dst_access, mask_expr(b.dst_access, ACCESS_BITS));
    s
}

/// One [`PassBarrierRange`], field by field, with the pass name.
fn describe_pass(r: &PassBarrierRange, index: usize) -> String {
    format!(
        "    pass {} img [{}, {}) buf [{}, {})\n",
        pass_label(index),
        r.img_begin,
        r.img_begin + r.img_count,
        r.buf_begin,
        r.buf_begin + r.buf_count,
    )
}

/// The shared head of every divergence report: what diverged, where, and how to act on it.
fn divergence_header(kind: &str, index: usize, actual_len: usize, expected_len: usize) -> String {
    format!(
        "the compiled {kind} stream of `build_maximal_frame` diverged from the pinned \
         baseline at index {index} (compiled {actual_len} entries, pinned {expected_len}).\n\
         This pin is the VG R3 P1-5a BASELINE: it was measured on the per-ResId state \
         machine BEFORE the (ResId, mip) re-key, and P1-5a's claim is that it does not \
         move. If you believe the new stream is correct, re-run \
         `dump_maximal_frame_barrier_stream` and justify EVERY changed line — do not paste \
         over the pin to make this green.\n"
    )
}

/// The full image-stream divergence report: the first differing index, which fields moved,
/// and both sides field-by-field with the resource NAME on each.
fn img_divergence_report(f: &Frame, actual: &[ImgBarrier], expected: &[ImgBarrier], i: usize) -> String {
    let mut s = divergence_header("IMAGE barrier", i, actual.len(), expected.len());
    match (actual.get(i), expected.get(i)) {
        (Some(a), Some(e)) => {
            let _ = writeln!(s, "  fields that differ: {}", img_field_diffs(a, e));
            let _ = writeln!(s, "  COMPILED [{i}]:\n{}", describe_img(f, a));
            let _ = writeln!(s, "  PINNED   [{i}]:\n{}", describe_img(f, e));
        }
        (Some(a), None) => {
            let _ = writeln!(s, "  the pinned stream ENDS here; the compiled one continues with:");
            let _ = writeln!(s, "  COMPILED [{i}]:\n{}", describe_img(f, a));
        }
        (None, Some(e)) => {
            let _ = writeln!(s, "  the compiled stream ENDS here; the pin still expects:");
            let _ = writeln!(s, "  PINNED   [{i}]:\n{}", describe_img(f, e));
        }
        (None, None) => {
            let _ = writeln!(s, "  index is past BOTH streams — bug in `first_divergence`.");
        }
    }
    s
}

/// The full buffer-stream divergence report (see [`img_divergence_report`]).
fn buf_divergence_report(f: &Frame, actual: &[BufBarrier], expected: &[BufBarrier], i: usize) -> String {
    let mut s = divergence_header("BUFFER barrier", i, actual.len(), expected.len());
    match (actual.get(i), expected.get(i)) {
        (Some(a), Some(e)) => {
            let _ = writeln!(s, "  fields that differ: {}", buf_field_diffs(a, e));
            let _ = writeln!(s, "  COMPILED [{i}]:\n{}", describe_buf(f, a));
            let _ = writeln!(s, "  PINNED   [{i}]:\n{}", describe_buf(f, e));
        }
        (Some(a), None) => {
            let _ = writeln!(s, "  the pinned stream ENDS here; the compiled one continues with:");
            let _ = writeln!(s, "  COMPILED [{i}]:\n{}", describe_buf(f, a));
        }
        (None, Some(e)) => {
            let _ = writeln!(s, "  the compiled stream ENDS here; the pin still expects:");
            let _ = writeln!(s, "  PINNED   [{i}]:\n{}", describe_buf(f, e));
        }
        (None, None) => {
            let _ = writeln!(s, "  index is past BOTH streams — bug in `first_divergence`.");
        }
    }
    s
}

/// The full per-pass-range divergence report (see [`img_divergence_report`]). A difference
/// here with IDENTICAL barrier arrays means the barriers were RE-ATTRIBUTED to other passes —
/// the same stream recorded at different points in the frame.
fn pass_divergence_report(actual: &[PassBarrierRange], expected: &[PassBarrierRange], i: usize) -> String {
    let mut s = divergence_header("PASS barrier-range", i, actual.len(), expected.len());
    match (actual.get(i), expected.get(i)) {
        (Some(a), Some(e)) => {
            let _ = write!(s, "  COMPILED [{i}]:\n{}", describe_pass(a, i));
            let _ = write!(s, "  PINNED   [{i}]:\n{}", describe_pass(e, i));
        }
        (Some(a), None) => {
            let _ = writeln!(s, "  the pinned stream ENDS here; the compiled one continues with:");
            let _ = write!(s, "  COMPILED [{i}]:\n{}", describe_pass(a, i));
        }
        (None, Some(e)) => {
            let _ = writeln!(s, "  the compiled stream ENDS here; the pin still expects:");
            let _ = write!(s, "  PINNED   [{i}]:\n{}", describe_pass(e, i));
        }
        (None, None) => {
            let _ = writeln!(s, "  index is past BOTH streams — bug in `first_divergence`.");
        }
    }
    s
}

/// **VG R3 P1-5a BASELINE**: the compiled barrier stream of [`build_maximal_frame`] equals
/// the pinned one ELEMENT FOR ELEMENT AND FIELD FOR FIELD, in order, across
/// `img_barriers()`, `buf_barriers()` AND `pass_barriers()`.
///
/// # What this pin CAN claim
///
/// That the DEFERRED replica frame — the maximal-permutation G-buffer path this file has
/// modelled since Step 1b, mirroring `declare_deferred_graph` — derives the exact same
/// barrier stream, in the exact same order, attributed to the exact same passes, as it did
/// on the per-`ResId` state machine before P1-5a. It is a strict superset of the count pin
/// (`graph_matches_hand_path_barrier_count_exactly`) and the membership pins
/// (`graph_covers_every_gbuffer_producer_consumer_hazard`): it is the only test here that
/// catches a REORDERING, a WIDENED `subresource`, or a barrier moved to another pass — all
/// three of which leave both counts and memberships intact.
///
/// # What this pin CANNOT claim
///
/// * **Nothing about the VB or the FORWARD declarators.** `declare_vb_graph` and
///   `declare_forward_graph` (`present/graph_bridge.rs`) have NO barrier-level test at all —
///   not this one, not a count, not a membership set. Whatever P1-5a does to their streams is
///   unmeasured by this file, and "the framegraph tests are green" says nothing about them.
///   Their coverage today is a rendered golden, which sees a barrier bug only if the hazard
///   materialises on this machine and survives to 8 bits.
/// * **Nothing about `declare_deferred_graph` ITSELF.** This is a hand-written REPLICA of it,
///   in this file, and the two can drift; the replica is what is pinned.
/// * **Nothing about recording.** The pin stops at the derived plan. How `record_all` batches
///   it into `vkCmdPipelineBarrier` array calls is `record_step_call_count_is_pinned`'s
///   subject, and that pin is a COUNT.
/// * **Nothing about soundness.** A stream can be pinned and wrong. This says "unchanged",
///   which is exactly the claim P1-5a needs and no more.
#[test]
fn maximal_frame_barrier_stream_is_pinned() {
    // FIRST, so an unfilled baseline reports ITSELF instead of a divergence at index 0
    // against a sentinel.
    let unfilled = EXPECTED_IMG_BARRIERS.contains(&TBD_IMG_BARRIER)
        || EXPECTED_BUF_BARRIERS.contains(&TBD_BUF_BARRIER)
        || EXPECTED_PASS_BARRIERS.contains(&TBD_PASS_RANGE);
    assert!(
        !unfilled,
        "the barrier-stream baseline is the UNFILLED PLACEHOLDER. Run \
         `dump_maximal_frame_barrier_stream` and paste its output over the three \
         `const EXPECTED_…` arrays in this file:\n    \
         cargo test -p boyko_rhi_vulkan --test framegraph_gbuffer_equiv \
         dump_maximal_frame_barrier_stream -- --ignored --nocapture\n\
         (The values are MEASURED off `compile()`, never predicted — see \
         `EXPECTED_IMG_BARRIERS`'s doc.)"
    );

    let f = build_maximal_frame();
    let img = f.g.img_barriers();
    let buf = f.g.buf_barriers();
    let passes = f.g.pass_barriers();

    // The pass-name table labels every report below; if it has drifted from the frame, the
    // labels lie, so check it before trusting anything they say.
    assert_eq!(
        passes.len(),
        MAXIMAL_FRAME_PASS_NAMES.len(),
        "MAXIMAL_FRAME_PASS_NAMES has {} rows but `build_maximal_frame` declares {} passes — \
         add the missing name(s) in `add_pass` order, or every failure report below \
         mislabels its pass",
        MAXIMAL_FRAME_PASS_NAMES.len(),
        passes.len()
    );

    if let Some(i) = first_divergence(img, EXPECTED_IMG_BARRIERS) {
        panic!("{}", img_divergence_report(&f, img, EXPECTED_IMG_BARRIERS, i));
    }
    if let Some(i) = first_divergence(buf, EXPECTED_BUF_BARRIERS) {
        panic!("{}", buf_divergence_report(&f, buf, EXPECTED_BUF_BARRIERS, i));
    }
    if let Some(i) = first_divergence(passes, EXPECTED_PASS_BARRIERS) {
        panic!("{}", pass_divergence_report(passes, EXPECTED_PASS_BARRIERS, i));
    }
}
