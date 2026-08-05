//! Step 1b equivalence gate: the frame graph, declared over the FULL (maximal-
//! permutation) on-screen G-buffer frame, must auto-derive a barrier set that is
//! a SOUND SUPERSET of `swapchain::record_gbuffer`'s hand-authored barriers —
//! same per-resource layout trajectories, every producer→consumer hazard covered,
//! and no more barriers than the hand path (minimality, C6).
//!
//! This is a pure-CPU diff: the graph does NOT drive the GPU in Step 1b, so this
//! runs on any machine (no `#[ignore]`, no Vulkan device). It is the reference
//! the live hand path is measured against before Step 1f deletes it.

use boyko_rhi_vulkan::ffi::{
    VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
    VK_ACCESS_SHADER_READ_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_TRANSFER_WRITE_BIT,
    VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
    VK_IMAGE_LAYOUT_GENERAL, VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, VK_IMAGE_LAYOUT_UNDEFINED,
    VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT,
    VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
    VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT,
    VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
};
use boyko_rhi_vulkan::framegraph::{BufBarrier, FrameGraph, ImgBarrier, ResId, ResSync, SubRange};

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
    // input. Both FIF-ringed / frame-private ⇒ `add_buffer` (undefined seed), so no cross-frame
    // ordering — only the intra-frame COMPUTE→VERTEX RAW is derived. ResIds are declared BEFORE
    // the interp pass (which accesses them), mirroring `declare_deferred_graph`. This test builds
    // its OWN local frame (not `declare_deferred_graph`), so the absolute ResId numbering does not
    // affect it — the note keeps the cross-reference accurate.
    let interp_pairs = g.add_buffer("interp_pairs");
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
// `compile()`'s DEBUG-ONLY unwritten-transient-image-read authoring guard.
//
// A mis-authored pass that READS a transient (non-seeded) image with no prior
// producer/seed would otherwise silently derive a hazard-free `TOP_OF_PIPE`
// barrier — a whole class of authoring regressions going uncaught. `compile`
// tracks a per-resource written-or-seeded bit and `debug_assert!`s it holds
// before a non-seeded transient IMAGE's first read.
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
/// written down — the sibling at `compile_panics_when_span_and_layout_both_vary` carries both the
/// attribute and this exact rationale — so what was missing was not the knowledge but the gate on
/// two tests that were added later.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "reads transient image")]
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

// ===========================================================================
// `compile()`'s DEBUG-ONLY `INVARIANT HZB-SUBRESOURCE-UNIFORM` guard (VG R3 S2).
//
// The sync state machine keys one `ResSync` per `ResId` and never reads a
// `SubRange`, so a single tracked layout can misdescribe a subresource exactly
// when a resource's accesses vary in BOTH the mip/layer span AND the layout.
// The guard latches one bit per axis and fires only on the conjunction. These
// tests pin both verdicts — a guard never shown RED is not a guard — and pin
// that the shipped layered declarations (CSM cascades, the punctual atlas, the
// DDGI probe atlases: one span, several layouts) are on the permitted side.
// ===========================================================================

/// `SubRange::color_mips` is a whole-chain COLOR range at layer 0 — the shape a
/// mip-pyramid access declares. Pinned because the guard's span comparison and
/// the emitted barrier both read these four fields verbatim.
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

/// Both axes vary on one resource (whole chain at GENERAL, then mip 0 alone at
/// SHADER_READ_ONLY_OPTIMAL) — the tracked layout now misdescribes mips 1..n, so
/// the guard must fire. This is the exact shape an HZB build/consume pair takes.
///
/// `cfg(debug_assertions)`: the guard IS a `debug_assert!`, so in a release test
/// binary `compile` correctly does not panic and a `should_panic` test would report
/// a failure that is not one. CI runs a debug × release matrix, so the debug leg
/// still gates this.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "HZB-SUBRESOURCE-UNIFORM")]
fn compile_panics_when_span_and_layout_both_vary() {
    let mut g = FrameGraph::with_capacity(4, 4, 4);
    let pyramid = g.add_image("pyramid");

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
}

/// The violation need not involve the FIRST access: here access 2 differs from
/// access 1 only in layout and access 3 only in span, so no single pair differs in
/// both — yet the resource as a whole varies on both axes and the tracked layout is
/// still a lie. Pins that the guard accumulates per axis instead of comparing each
/// access pairwise against the first (which would pass this and miss the bug).
///
/// `cfg(debug_assertions)` for the same reason as the test above.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "HZB-SUBRESOURCE-UNIFORM")]
fn compile_panics_when_the_variation_straddles_three_accesses() {
    let mut g = FrameGraph::with_capacity(4, 4, 4);
    let pyramid = g.add_image("pyramid");

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
}

/// ⚠️ THIS TEST ONCE ASSERTED THE OPPOSITE, and the reversal is the point of the rung.
///
/// Its first version was named `compile_allows_a_varying_span_at_a_uniform_layout` and it
/// PASSED: the guard's original condition permitted a varying span as long as every access
/// declared one layout, on the reasoning that "one layout is true of every subresource".
/// That reasoning is false. A uniform DECLARED layout does not give a uniform ACTUAL one —
/// only the union of spans that actually appeared in an emitted barrier has been
/// transitioned, and every other subresource is still in the image's start layout.
///
/// Reverse the two passes below and the old guard still passed while the frame was UB: the
/// subset-first order transitions mips [0,1) only, then the superset access emits a barrier
/// over [0,4) claiming `oldLayout = GENERAL` for mips 1..3, which are UNDEFINED
/// (VUID-VkImageMemoryBarrier-oldLayout-01197). A guard whose verdict depends on
/// declaration order is not a guard, so the condition was strengthened to span-uniformity
/// and this fixture became the RED case.
///
/// It is also the exact shape a per-mip HZB build chain wants — write mip k, read mip k-1 —
/// which is why the panic message points at per-subresource sync state rather than at
/// making the declarations agree by hand. Tripping here is the intended way to discover
/// that work.
///
/// `cfg(debug_assertions)`: the guard IS a `debug_assert!` — see
/// `compile_panics_on_unwritten_transient_image_read` for the measurement that showed both of
/// these were failing CI's release leg without it.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "HZB-SUBRESOURCE-UNIFORM")]
fn compile_panics_when_one_resource_declares_two_spans() {
    let mut g = FrameGraph::with_capacity(4, 4, 4);
    let pyramid = g.add_image("pyramid");

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

/// The witness is per-COMPILE scratch, not accumulated state. A `FrameGraph` is reused
/// every frame, so a witness surviving `reset` + re-declare would judge this frame's
/// accesses against LAST frame's resource: here graph A pins ResId 0 at
/// `(COLOR, COLOR_ATTACHMENT_OPTIMAL)`, and graph B's first access (`color_mips(4)` at
/// GENERAL) differs from it on BOTH axes — a leak would latch both bits and fire on a
/// declaration that is in fact uniform-span. Silence is the discriminating outcome.
#[test]
fn subresource_guard_does_not_leak_across_reset_and_recompile() {
    let mut g = FrameGraph::with_capacity(4, 4, 8);

    // Graph A — ResId 0 is a plain single-mip color attachment.
    let a = g.add_image("attachment");
    g.add_pass("raster");
    g.image_access(
        a,
        VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        SubRange::COLOR,
    );
    g.compile();

    // Graph B — ResId 0 is now a whole-chain pyramid at two layouts: one span, so
    // permitted. Only a leaked witness from graph A could make this fire.
    g.reset();
    let pyramid = g.add_image("pyramid");
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

    // Two hazards: the first-touch UNDEFINED→GENERAL write, then the RAW read.
    assert_eq!(g.img_barriers().len(), 2);
}
