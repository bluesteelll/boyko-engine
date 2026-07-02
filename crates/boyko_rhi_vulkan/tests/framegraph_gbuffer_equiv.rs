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
};
use boyko_rhi_vulkan::framegraph::{BufBarrier, FrameGraph, ImgBarrier, ResId, SubRange};

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

    // Images.
    let albedo = g.add_image("albedo");
    let normal = g.add_image("normal");
    let material = g.add_image("material");
    let depth = g.add_image("depth");
    let viewt = g.add_image("viewt");
    let lit = g.add_image("lit");
    let ssao = g.add_image("ssao");
    let cascade = g.add_image("cascade");
    let atlas = g.add_image("atlas");
    // The WSI swapchain image (acquired UNDEFINED, presented PRESENT_SRC_KHR) — a
    // first-class graph resource, so the acquire→render→present transition is
    // owned + verified like any other (C2).
    let swapchain = g.add_image("swapchain");
    // Buffers.
    let light_table = g.add_buffer("light_table");
    let tiles = g.add_buffer("tiles");
    let grid = g.add_buffer("grid");
    let index = g.add_buffer("index");
    let alloc = g.add_buffer("alloc");

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

    // --- CSM cascade layered: UNDEFINED→DEPTH (4 layers), DEPTH→SHADER_READ_ONLY (4 layers) ---
    assert!(
        has_img(img, f.cascade, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, FRAG, 0, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, SubRange::depth_layers(CASCADE_LAYERS)),
        "cascade layered depth-in missing"
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
}

#[test]
fn graph_matches_hand_path_barrier_count_exactly() {
    let f = build_maximal_frame();
    let img = f.g.img_barriers().len();
    let buf = f.g.buf_barriers().len();

    // Derived by ENUMERATING every `cmd_pipeline_barrier`-emitted image/buffer
    // barrier in `record_gbuffer` for the maximal-live permutation (readback=None):
    //   IMAGE (23): color-in ×3, depth-in ×1, depth→sampled ×1, color→general ×3,
    //     lit/viewt/ssao→general ×3, store→load ×4, ssao store→load ×1,
    //     csm-in ×1, csm→sampled ×1, atlas-in ×1, atlas→sampled ×1, lit→sampled ×1,
    //     swapchain acquire→color ×1, swapchain color→present ×1.
    //   BUFFER (5): light-upload ×1, coarse-tiles ×1, alloc-reset ×1, cull→resolve ×2.
    // The graph derives EXACTLY these (equality, not `≤` — the hand path is already
    // barrier-minimal; the graph's win is auto-derivation + correctness + the
    // history-rotation/aliasing it enables, NOT fewer barriers).
    const HAND_IMAGE_BARRIERS: usize = 23;
    const HAND_BUFFER_BARRIERS: usize = 5;
    assert_eq!(img, HAND_IMAGE_BARRIERS, "image barrier count diverged from the hand path");
    assert_eq!(buf, HAND_BUFFER_BARRIERS, "buffer barrier count diverged from the hand path");
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
/// derived barriers by (src,dst) stage pair into batched array calls. Measure the
/// resulting call count — it must not exceed the hand path's post-1a array-call
/// count. The honest result is PARITY (18 == 18): the graph fuses some batches the
/// hand path splits (cascade+atlas→sampled; light_table+alloc) and splits some the
/// hand path fuses (lit/viewt/ssao→general placed at each true first-use), netting
/// zero. The graph's win is auto-derivation + correctness + enabling history-
/// rotation/aliasing — NOT fewer calls than hand-tuned sync1 batching.
#[test]
fn record_step_call_count_is_parity_with_hand_path() {
    let f = build_maximal_frame();
    let mut sink = CountSink::default();
    f.g.record_all(&mut sink);
    let total = sink.img_calls + sink.buf_calls;
    // The post-1a hand path emits 18 array-form `vkCmdPipelineBarrier` calls for
    // this maximal-live permutation (see graph_matches_hand_path_barrier_count).
    const HAND_ARRAY_CALLS: usize = 18;
    assert_eq!(
        total, HAND_ARRAY_CALLS,
        "graph record call count diverged from the hand path's array-call count"
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
