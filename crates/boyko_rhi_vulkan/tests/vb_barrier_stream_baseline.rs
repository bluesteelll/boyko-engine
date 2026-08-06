//! VG R3 piece 2, step **P2-4** — the BASELINE barrier-stream pin for the VisibilityBuffer
//! frame, on the UNMODIFIED declarator, over the four UNSPLIT rows of gate G4's matrix
//! (`docs/VG-R3-P2-CAPABILITY-SPLIT-PLAN.md`, "G4 — the derived barrier stream, per
//! CONFIGURATION, asserted FIELD-BY-FIELD").
//!
//! # Why this file exists BEFORE the split does
//!
//! `docs/VG-R3-P1-PYRAMID-PLAN.md` states the discipline this step obeys: *"Authoring them
//! after the change would certify the new behaviour instead of the old one."* Every expectation
//! here is measured against today's declaration shape, so P2-5's diff is measured against a
//! baseline nobody could have tuned to it.
//!
//! # Why it is the ONLY gate that can see a missing barrier
//!
//! Step P2-0 was executed and RESOLVED (the plan's "P2-0 RESOLVED" section): a genuine missing
//! barrier — `hzb_build_p`'s read of mip `d - 1`, the only declared read of that mip, deleted
//! while the dispatch that reads it stayed — produced **19 validation messages (the unchanged
//! baseline), no `SYNC-HAZARD-*`, and a byte-identical golden image**. Synchronization
//! validation is not live on this machine. So neither the golden pins nor the validation leg
//! can see a barrier defect in this piece, and the field-level assertions below are not
//! belt-and-braces — they are the whole of the coverage.
//!
//! A COUNT would not have been enough, and that is the round-1 finding this file is written
//! against: the read-declared/write-undeclared defect yields the SAME barrier count and differs
//! only in `src_stage`/`src_access`. [`a_dropped_writer_keeps_every_count_and_moves_only_fields`]
//! demonstrates exactly that, today, on this replica.
//!
//! # What is pinned
//!
//! For each row: the whole compiled stream — `img_barriers()`, `buf_barriers()` AND
//! `pass_barriers()` — element for element and FIELD for field, in declaration order. That is
//! the only shape that catches a reordering, a widened `subresource`, or a barrier re-attributed
//! to another pass, all three of which leave counts and memberships intact. Piece 2's D6 moves a
//! whole pass block between two scopes, so per-pass attribution is a first-class part of the
//! baseline rather than a bonus.
//!
//! # What this pin CANNOT claim
//!
//! * **Nothing about `declare_vb_graph` ITSELF.** This is a hand-written REPLICA of it.
//!   `declare_vb_graph` is `pub(crate)` on a `Renderer` no test constructs (its only references
//!   are its definition in `present/graph_bridge.rs` and the `3 =>` arm of
//!   `declare_frame_graph`'s dispatch; every other hit is a doc comment), so no integration test
//!   can call it. The tree's existing framegraph pin says the same about its own subject,
//!   verbatim: *"**Nothing about `declare_deferred_graph` ITSELF.** This is a hand-written
//!   REPLICA of it"* (`tests/framegraph_gbuffer_equiv.rs`). This file proves the framegraph
//!   DERIVES this stream from a declaration shaped like the one `declare_vb_graph` writes — not
//!   that `declare_vb_graph` writes that shape.
//! * **What keeps the replica in step with the declarator** is therefore named here, because
//!   "read the declarator carefully" is not a mechanism:
//!   1. Every arming predicate below is taken from a NAMED production predicate rather than
//!      re-invented — `GBufferScene::path_vb_ssao` / `path_vb_split` / `path_vb_fused` /
//!      `path_has_sdf_forward` / `path_sdf_forward_writes_viewt` (`present/scene_types.rs`), and
//!      the host arming site for `viewt_from_vb_depth` (`boyko_app/src/gpu_scene/mod.rs`, the
//!      `(!sdf_leg && Taa) || (mesh_geo_shade_split && ssao)` disjunction). Renaming or
//!      re-gating one of those is a grep away from this file.
//!   2. Every span and count that the declarator derives from a constant is taken from THAT
//!      constant here — `MAX_CASCADES`, `MAX_TEXTURE_LAYERS`, `HZB_LEVELS_PER_PASS`,
//!      `MAX_HZB_PASSES` — never a literal, so a constant that moves moves this pin with it.
//!   3. `declare_vb_graph`'s own `debug_assert`s run in production on every dev-profile golden
//!      run (`scripts/golden.ps1` carries no `--release`): the `hzb_pyramid`-is-the-last-image
//!      assert, the `vb_indirect_late`-is-the-last-buffer assert, `poison < build`,
//!      `build < dump`, and `poison.is_some() == dump.is_some()`. Those constrain the real
//!      declarator where this replica cannot reach it.
//!   4. G2 (step P2-6) contributes a scope count that originates in the RECORDER. Replica pin,
//!      production asserts and recorder count are the evidence together; none of them alone is.
//! * **Nothing about recording.** The pin stops at the derived plan; how `record_all` batches it
//!   into `vkCmdPipelineBarrier` array calls is a different question.
//! * **Nothing about pixels, and nothing about soundness.** A stream can be pinned and wrong.
//!   This says "this is what the machine derives, and it has not moved", which is exactly the
//!   claim P2-5's diff needs and no more.
//! * **Nothing about the occlusion split**, which does not exist yet. No row here records a
//!   second raster scope; G4's four split rows (S1..S4) are authored at P2-5.
//! * **Nothing about the `hwrt` image/buffer tail.** The `hwrt` arm of `declare_vb_graph`
//!   appends six images and one buffer that NO pass in these four configurations names (the VB
//!   hardware shadow chain needs `split && scene.shadow`, and the rows below hold `shadow` off),
//!   so they route zero barriers by the declarator's own rule and are not modelled. This replica
//!   builds its OWN local frame, so its absolute `ResId` numbering is its own and the omission
//!   cannot shift a pinned index.
//!
//! Pure CPU: no device, no `dxc`, no window. It cannot SKIP.

use std::fmt::Write as _;

use boyko_rhi_vulkan::compute::{HZB_LEVELS_PER_PASS, MAX_HZB_PASSES};
use boyko_rhi_vulkan::ffi::{
    VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
    VK_ACCESS_INDIRECT_COMMAND_READ_BIT, VK_ACCESS_SHADER_READ_BIT, VK_ACCESS_SHADER_WRITE_BIT,
    VK_ACCESS_TRANSFER_READ_BIT, VK_ACCESS_TRANSFER_WRITE_BIT, VK_IMAGE_ASPECT_COLOR_BIT,
    VK_IMAGE_ASPECT_DEPTH_BIT, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
    VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_GENERAL,
    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
    VK_IMAGE_LAYOUT_UNDEFINED, VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
    VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT, VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
    VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
    VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
};
use boyko_rhi_vulkan::framegraph::{
    BufBarrier, FrameGraph, ImgBarrier, PassBarrierRange, ResId, ResSync, SubRange,
};
use boyko_rhi_vulkan::texture::{MAX_CASCADES, MAX_TEXTURE_LAYERS};

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Constants mirroring the declarator's own
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// `declare_vb_graph`'s own `FRAG` local — the depth-attachment stage pair.
const FRAG: u32 =
    VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT;

/// The read|write access the batch cull declares on its atomic counter (`declare_vb_graph`'s own
/// `RW` local inside the `vb_batch_cull` arm).
const RW: u32 = VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT;

/// The HZB level count the shipping 512×512 pin resolves — the number step P1-4's corruption
/// control reported from the engine (`sets built = 4, pass_count = 2, levels = 10`) and the one
/// `compile_derives_the_hzb_build_chain_at_a_real_extent` is written at. G4's U2 row names it.
const HZB_LEVELS: u32 = 10;

/// `HZB_BUILD_PASS_NAMES` in `present/graph_bridge.rs`, which is private to that module.
/// `FrameGraph::add_pass` takes a `&'static str`, so one literal per slot IS the mechanism
/// there and here; sized by [`MAX_HZB_PASSES`] so a capacity change is a compile error rather
/// than an index panic.
const HZB_BUILD_PASS_NAMES: [&str; MAX_HZB_PASSES] = ["hzb_build_0", "hzb_build_1", "hzb_build_2"];

/// A single-layer COLOR span over mips `[base, base + count)` — `graph_bridge.rs`'s private
/// `hzb_mips` helper. `SubRange::color_mips` cannot express it (it pins `base_mip: 0`, while a
/// reduce pass reads mip `d - 1`).
const fn hzb_mips(base: u32, count: u32) -> SubRange {
    SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: base, mip_count: count, base_layer: 0, layer_count: 1 }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The matrix
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// One row of G4's matrix, in the UNSPLIT half. The knobs are exactly the columns G4 varies;
/// everything else is held FIXED across all four rows (see [`declare_vb_unsplit_frame`]), so a
/// row-to-row difference in the pinned stream is attributable to the row's own column.
///
/// The occlusion split itself has no field here **on purpose**: it does not exist at P2-4, and a
/// knob for it would be a place for P2-5 to be tuned into this baseline.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct VbUnsplitRow {
    /// G4's row label, used in every failure message so a divergence names WHICH configuration
    /// moved rather than "the stream drifted".
    id: &'static str,
    /// `scene.hzb` — `Some(levels)` arms `hzb_build_*`; `None` is the `HzbMode::Off` 0%-gate,
    /// where the pyramid `ResId` is still declared and named by no pass.
    hzb_levels: Option<u32>,
    /// `scene.hzb_dump` — arms the `hzb_poison` + `hzb_dump` pair (ONE predicate, asserted equal
    /// in the declarator). Requires `hzb_levels`, exactly as `scene.hzb.filter(|_| mesh_leg)`
    /// does in production.
    hzb_dump: bool,
    /// `scene.ssao` — arms the rung-R9b geo/shade split (`path_vb_ssao()` is
    /// `mesh_geo_shade_split && ssao.is_some()`, and SSAO under VB reads the split's `thin_normal`
    /// lane), and with it the `vb_viewt` PRE-TAIL slot: the host arms `viewt_from_vb_depth` on
    /// `(mesh_geo_shade_split && ssao_variant.is_some())` as well as on the marcher-less TAA
    /// case. Row U4 is the only one that sets it.
    ssao: bool,
    /// `resolved_render_path.sdf_leg` — `VisibilityBuffer × Both`, which arms
    /// `sdf_forward_march`. Its `mesh_leg` arm is the fourth `vb_depth` reader the D6 block move
    /// re-sources, which is why U4 carries it.
    sdf_leg: bool,
    /// **RED CONTROL ONLY.** Drops `vb_batch_cull`'s declared `vb_visible_instance` write while
    /// leaving `vb_raster`'s read of it in place — the write-undeclared defect class B2 named.
    /// Every pinned row below holds this `false`; only
    /// [`a_dropped_writer_keeps_every_count_and_moves_only_fields`] sets it.
    red_control_drop_cull_survivor_write: bool,
}

/// **U1** — split off, HZB off, dump off, SSAO off, `VB × Mesh`. The shipping baseline: nothing
/// about the split leaks into the unarmed path.
const U1: VbUnsplitRow = VbUnsplitRow {
    id: "U1 (split off, HZB off, dump off, SSAO off, VB×Mesh)",
    hzb_levels: None,
    hzb_dump: false,
    ssao: false,
    sdf_leg: false,
    red_control_drop_cull_survivor_write: false,
};

/// **U2** — split off, HZB armed, dump off, SSAO off, `VB × Mesh`. Today's `vb_mesh_hzb` shape,
/// including the three pyramid barriers at `levels = 10`.
const U2: VbUnsplitRow = VbUnsplitRow {
    id: "U2 (split off, HZB armed, dump off, SSAO off, VB×Mesh)",
    hzb_levels: Some(HZB_LEVELS),
    hzb_dump: false,
    ssao: false,
    sdf_leg: false,
    red_control_drop_cull_survivor_write: false,
};

/// **U3** — split off, HZB armed, dump ON, SSAO off, `VB × Mesh`. G5's own path: `hzb_poison`'s
/// `UNDEFINED → GENERAL` first touch, and `hzb_dump`'s `vb_depth` source — the source P2-5
/// re-sources.
const U3: VbUnsplitRow = VbUnsplitRow {
    id: "U3 (split off, HZB armed, dump ON, SSAO off, VB×Mesh)",
    hzb_levels: Some(HZB_LEVELS),
    hzb_dump: true,
    ssao: false,
    sdf_leg: false,
    red_control_drop_cull_survivor_write: false,
};

/// **U4** — split off, HZB armed, dump off, SSAO ON, `VB × Both`. The other re-sourced
/// `vb_depth` readers: the `vb_viewt` PRE-TAIL slot and `sdf_forward_march`'s mesh arm.
const U4: VbUnsplitRow = VbUnsplitRow {
    id: "U4 (split off, HZB armed, dump off, SSAO ON, VB×Both)",
    hzb_levels: Some(HZB_LEVELS),
    hzb_dump: false,
    ssao: true,
    sdf_leg: true,
    red_control_drop_cull_survivor_write: false,
};

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The replica
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A compiled replica frame plus the handles the named-hazard assertions read.
///
/// Only the `ResId`s an assertion names are carried; the rest are declared as `_`-prefixed
/// locals in [`declare_vb_unsplit_frame`], where the underscore records "declared for `ResId`
/// fidelity, named by no pass in these rows".
struct VbFrame {
    /// The compiled graph.
    g: FrameGraph,
    /// The pass names in `add_pass` order, recorded AT the `add_pass` call so a failure report
    /// cannot mislabel a pass. (`FrameGraph` exposes no pass-name accessor: `pass_barriers()`
    /// returns bare index ranges.) A separate hand-maintained table would be a second thing that
    /// can disagree with the frame — here there is only one.
    pass_names: Vec<&'static str>,
    /// The row this frame was built from, for failure messages.
    row: VbUnsplitRow,
    lit: ResId,
    vb_id: ResId,
    vb_depth: ResId,
    viewt: ResId,
    hzb_pyramid: ResId,
    light_table: ResId,
    vb_instance_ring: ResId,
    vb_indirect: ResId,
    vb_visible_instance: ResId,
    /// P2-3's late record array — declared, named by NO pass at this step. Every row asserts it
    /// routes zero barriers; that is the structural form of "nothing about the split leaks into
    /// the unarmed path", and it is the assertion P2-5 must flip.
    vb_indirect_late: ResId,
}

/// Declare and compile one UNSPLIT row of G4's matrix, mirroring `declare_vb_graph`'s
/// declaration order pass for pass and access for access.
///
/// # The knobs this replica holds FIXED, and why each is a fixed base rather than a column
///
/// | knob | value | why |
/// |---|---|---|
/// | `mesh_leg` | on | every G4 row is a mesh frame; a `VB × Sdf` frame declares neither raster |
/// | `light_dirty && light_upload_bytes > 0` | on | the frame-1 shape; it is what makes the `light_table` WAR seed and the upload→shade RAW appear at all, and it is constant across the matrix |
/// | `scene.cluster_cull` (L1 froxel) | off | the owner default (`LightingConfig::clusters_enabled == false`); the armed boot is its own golden pin, not a G4 row |
/// | `scene.csm` / `scene.atlas_punctual` | on | the shipping shadow vocabulary, constant across the matrix |
/// | `scene.vb_indirect` / the R2c batch cull | on | armed on the shipping VB pins; the `vb_indirect` upload→cull→raster chain is exactly what P2-5's `vb_indirect_late` mirrors, so the baseline must contain it |
/// | `scene.vb_cull_readback` | off | a probe; an unarmed boot declares no pass at all |
/// | `vb_use_classified` | off (FUSED `vb_resolve`) | the classify chain is orthogonal to every resource the occlusion split touches, and the split arm (U4) displaces it anyway |
/// | `scene.taa` | off | TAA arms a second `gViewT` producer schedule that G4 does not vary |
/// | DDGI (`path_vb_ddgi`) | off | GI-off is the byte-identity discipline the tree already pins |
/// | `scene.shadow` (hwrt chain) | off | see the module doc: the `hwrt` tail is unnamed by every pass here |
/// | SSAO à-trous levels | 0 | a legal clamped value (the storage-degrade path, `ssao_atrous_levels == 0 \|\| (2..=MAX)`), and the ladder touches no `vb_depth` reader U4 exists to pin |
fn declare_vb_unsplit_frame(row: VbUnsplitRow) -> VbFrame {
    // Sized past the mip-weighted state total (29 resources, of which the pyramid contributes
    // `HZB_LEVELS` entries) so a declare pass performs no reallocation.
    let mut g = FrameGraph::with_capacity(48, 24, 128);
    let mut pass_names: Vec<&'static str> = Vec::with_capacity(24);
    // `declare_vb_graph`'s own first statement. Redundant on a graph this fn just built, kept so
    // the replica's shape is the declarator's — the real one re-declares into a REUSED graph.
    g.reset();

    // Begin a pass AND record its name in ONE statement, so the frame and the label list
    // cannot drift apart — the failure mode a separately maintained pass-name table has.
    macro_rules! pass {
        ($name:expr) => {{
            let name: &'static str = $name;
            pass_names.push(name);
            g.add_pass(name)
        }};
    }

    // ---- Images, in `declare_vb_graph`'s FIXED local ResId order ----------------------------
    // The seeds are the declarator's verbatim: `cascade`/`atlas` are non-ringed and end their
    // frame consumed by a COMPUTE read (VB's shading consumer is a COMPUTE pass, not a fragment
    // shader — the P1-1 seed-stage rule); the TAA parity pair is the write/read-sibling
    // WAR/RAW pair; `ssao` carries the deferred declarator's `undefined()` seed; the DDGI
    // atlases are persistent accumulators living in SHADER_READ_ONLY_OPTIMAL between updates.
    let lit = g.add_image("lit");
    let vb_id = g.add_image("vb_id");
    let vb_depth = g.add_image("vb_depth");
    let cascade = g.add_image_seeded(
        "cascade",
        ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
    );
    let atlas = g.add_image_seeded(
        "atlas",
        ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
    );
    let viewt = g.add_image("viewt");
    let _taa_hist = g.add_image_seeded(
        "taa_hist",
        ResSync::seeded_readers_at_layout(
            VK_IMAGE_LAYOUT_GENERAL,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        ),
    );
    let _taa_hist_read = g.add_image_seeded(
        "taa_hist_read",
        ResSync::seeded_writer_at_layout(
            VK_IMAGE_LAYOUT_GENERAL,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
        ),
    );
    let thin_normal = g.add_image("thin_normal");
    let ssao_img = g.add_image_seeded("ssao", ResSync::undefined());
    let _ssao_ring_a = g.add_image("ssao_ring_a");
    let _ssao_ring_b = g.add_image("ssao_ring_b");
    let _ddgi_irr = g.add_image_seeded(
        "ddgi_irr",
        ResSync::seeded_readers_at_layout(
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        ),
    );
    let _ddgi_depth = g.add_image_seeded(
        "ddgi_depth",
        ResSync::seeded_readers_at_layout(
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        ),
    );
    // The pyramid is declared LAST among images in both `cfg` arms, with `plan.levels` mips when
    // armed and the minimum `1` when not — `add_image_mipped` rejects `0`, and the disarmed value
    // is sound precisely because no access names the ResId then.
    let hzb_pyramid = g.add_image_mipped("hzb_pyramid", row.hzb_levels.unwrap_or(1), ResSync::undefined());

    // ---- Buffers, in the declarator's FIXED order ------------------------------------------
    let light_table = g.add_buffer_seeded(
        "light_table",
        ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
    );
    let vb_instance_ring = g.add_buffer("vb_instance_ring");
    let _gclassify = g.add_buffer("gclassify");
    let _ddgi_classification = g.add_buffer_seeded(
        "ddgi_classification",
        ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
    );
    let _ddgi_ray_table = g.add_buffer_seeded(
        "ddgi_ray_table",
        ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
    );
    let _cluster_grid = g.add_buffer_seeded(
        "cluster_grid",
        ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
    );
    let _light_index = g.add_buffer_seeded(
        "light_index",
        ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
    );
    let _light_index_alloc = g.add_buffer_seeded(
        "light_index_alloc",
        ResSync::seeded_writer(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT),
    );
    let vb_indirect = g.add_buffer("vb_indirect");
    let vb_batch_desc = g.add_buffer("vb_batch_desc");
    let vb_cull_visible = g.add_buffer("vb_cull_visible");
    let vb_cull_count = g.add_buffer("vb_cull_count");
    let vb_visible_instance = g.add_buffer("vb_visible_instance");
    // P2-3's append. Declared, named by no pass — the `hzb_pyramid` shape one screen up.
    let vb_indirect_late = g.add_buffer("vb_indirect_late");

    // ---- Passes, in declaration (execution) order -------------------------------------------

    // `light_upload` — held armed across the whole matrix (see the fixed-base table).
    pass!("light_upload");
    g.buffer_access(light_table, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT);

    // `light_cull` — the L1 froxel cull, held OFF across the matrix.

    // `csm_depth` / `atlas_depth`: the FULL `MAX_CASCADES` / `MAX_TEXTURE_LAYERS` arrays, the
    // 09600 whole-view shape every declarator uses.
    pass!("csm_depth");
    g.image_access(
        cascade,
        FRAG,
        VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        SubRange::depth_layers(MAX_CASCADES as u32),
    );
    pass!("atlas_depth");
    g.image_access(
        atlas,
        FRAG,
        VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        SubRange::depth_layers(MAX_TEXTURE_LAYERS as u32),
    );

    // `vb_sky` (always present): the first touch of `lit` as a COLOR attachment.
    pass!("vb_sky");
    g.image_access(
        lit,
        VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        SubRange::COLOR,
    );

    // `vb_indirect_upload` — the inline `vkCmdUpdateBuffer` that fills this frame's draw records
    // and, since R2c0, the `VbBatchDesc` array the cull reads. Its own pass, so the graph DERIVES
    // the TRANSFER → COMPUTE / TRANSFER → DRAW_INDIRECT dependencies against the consumers below.
    pass!("vb_indirect_upload");
    g.buffer_access(vb_indirect, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT);
    g.buffer_access(vb_batch_desc, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT);

    // `vb_batch_cull` — the counter's `vkCmdFillBuffer` reset and the atomics that follow it in
    // ONE pass (the intra-pass TRANSFER → COMPUTE shape `light_cull` uses for its own allocator),
    // then the descriptor read, the `instanceCount` rewrite, the compacted list, and R2d-3's two
    // per-INSTANCE accesses.
    pass!("vb_batch_cull");
    g.buffer_access(vb_cull_count, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT);
    g.buffer_access(vb_cull_count, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW);
    g.buffer_access(vb_batch_desc, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    g.buffer_access(vb_indirect, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT);
    g.buffer_access(vb_cull_visible, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT);
    g.buffer_access(vb_instance_ring, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    if !row.red_control_drop_cull_survivor_write {
        g.buffer_access(
            vb_visible_instance,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
        );
    }

    // `vb_cull_readback` — the probe, held OFF across the matrix.

    // `vb_raster` — the early scope, and the only one that exists at P2-4.
    pass!("vb_raster");
    g.buffer_access(
        vb_indirect,
        VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    );
    g.image_access(
        vb_id,
        VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        SubRange::COLOR,
    );
    g.image_access(
        vb_depth,
        FRAG,
        VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        SubRange::DEPTH,
    );
    g.buffer_access(vb_instance_ring, VK_PIPELINE_STAGE_VERTEX_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    g.buffer_access(
        vb_visible_instance,
        VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
    );

    // The classify chain is skipped: `use_classified` is off across the matrix, and the R9b split
    // (row U4) displaces it regardless (`path_vb_fused()` is false under the split).

    // The FUSED `lit` producer. Under the R9b split NEITHER `vb_resolve` nor `vb_shade` runs —
    // `vb_shade_split`, declared further down, is the producer.
    let split = row.ssao;
    if !split {
        pass!("vb_resolve");
        g.image_access(
            vb_id,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::COLOR,
        );
        g.image_access(
            lit,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            SubRange::COLOR,
        );
        g.buffer_access(vb_instance_ring, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
        // Gated on `light_upload.is_some()` in the declarator — armed across this matrix.
        g.buffer_access(light_table, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
        // UNCONDITIONAL + FULL-ARRAY (09600): the shader statically references both always-bound
        // Set-1 shadow maps.
        g.image_access(
            cascade,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::depth_layers(MAX_CASCADES as u32),
        );
        g.image_access(
            atlas,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::depth_layers(MAX_TEXTURE_LAYERS as u32),
        );
    }

    // ---- The pyramid POISON + BUILD block, at TODAY'S slot ----------------------------------
    // This is the block D6 moves between the two raster scopes on an armed-split frame. Its
    // position here — after the `lit` producer, before the `vb_viewt` PRE-TAIL slot — is
    // precisely what P2-5's diff is measured against, and it is why `pass_barriers()` is part of
    // this pin rather than an extra.
    if let Some(levels) = row.hzb_levels {
        if row.hzb_dump {
            pass!("hzb_poison");
            g.image_access(
                hzb_pyramid,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                hzb_mips(0, levels),
            );
        }

        let pass_count = levels.div_ceil(HZB_LEVELS_PER_PASS) as usize;
        assert!(
            pass_count <= MAX_HZB_PASSES,
            "the row's level count needs {pass_count} passes, more than MAX_HZB_PASSES"
        );
        // Iterated by NAME rather than by index: the declarator's own loop is
        // `for p in 0..pass_count { g.add_pass(HZB_BUILD_PASS_NAMES[p]) }`, and the replica must
        // walk the same names in the same order. `.take(pass_count)` is what keeps the two in
        // step — the array is `MAX_HZB_PASSES` long (a CAPACITY) while `pass_count` is the live
        // span, the distinction `MAX_HZB_LEVELS`' own doc warns about.
        for (p, pass_name) in HZB_BUILD_PASS_NAMES.iter().enumerate().take(pass_count) {
            let d = p as u32 * HZB_LEVELS_PER_PASS;
            let n = (levels - d).min(HZB_LEVELS_PER_PASS);
            pass!(*pass_name);
            if p == 0 {
                // The SOURCE depth, at the same (stage, access, layout, aspect) shape
                // `vb_viewt` / `sdf_forward_march` declare. THIS is the access that derives
                // `vb_depth`'s DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL transition
                // today, and every later same-layout read then needs none.
                g.image_access(
                    vb_depth,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                    SubRange::DEPTH,
                );
            } else {
                g.image_access(
                    hzb_pyramid,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                    VK_ACCESS_SHADER_READ_BIT,
                    VK_IMAGE_LAYOUT_GENERAL,
                    hzb_mips(d - 1, 1),
                );
            }
            g.image_access(
                hzb_pyramid,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                hzb_mips(d, n),
            );
        }
    }

    // ---- The rung-R9b split arm -------------------------------------------------------------
    // `vb_viewt` PRE-TAIL slot: with the split's SSAO armed the gViewT producer must run BEFORE
    // the gather, so ONE `ssao.is_some()` predicate picks this slot over the LATE one (the
    // accesses are IDENTICAL in both; only the position differs). Reachable here because the
    // host arms `viewt_from_vb_depth` on `mesh_geo_shade_split && ssao_variant.is_some()`.
    if row.ssao {
        pass!("vb_viewt");
        g.image_access(
            vb_depth,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::DEPTH,
        );
        g.image_access(
            viewt,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            SubRange::COLOR,
        );
    }
    if split {
        // `vb_geo` — the split's thin-aux producer and the FIRST `vb_id` reader under split.
        pass!("vb_geo");
        g.image_access(
            vb_id,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::COLOR,
        );
        g.buffer_access(vb_instance_ring, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
        g.image_access(
            thin_normal,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            SubRange::COLOR,
        );

        // The SSAO gather (`path_vb_ssao()`), with the à-trous ladder at 0 levels.
        pass!("ssao");
        for &res in &[thin_normal, viewt] {
            g.image_access(
                res,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                SubRange::COLOR,
            );
        }
        g.image_access(
            ssao_img,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            SubRange::COLOR,
        );

        // `vb_shade_split` — the split's `lit` producer. Reads `ssao` UNCONDITIONALLY (the 09600
        // stable-descriptor discipline); the DDGI atlas reads are gated on an update this matrix
        // holds off.
        pass!("vb_shade_split");
        g.image_access(
            vb_id,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::COLOR,
        );
        g.image_access(
            lit,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            SubRange::COLOR,
        );
        g.buffer_access(vb_instance_ring, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
        g.buffer_access(light_table, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
        g.image_access(
            cascade,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::depth_layers(MAX_CASCADES as u32),
        );
        g.image_access(
            atlas,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::depth_layers(MAX_TEXTURE_LAYERS as u32),
        );
        g.image_access(
            ssao_img,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            SubRange::COLOR,
        );
    }

    // `sdf_forward_march` — the fused SDF march-then-shade pass. Under `Both` its `lit` write
    // extends the previous producer's GENERAL write (COMPUTE→COMPUTE WAW, no layout change), and
    // under `mesh_leg` it READS `vb_depth` — the fourth reader the D6 block move re-sources.
    // `path_sdf_forward_writes_viewt()` is `has_sdf && taa.is_some()`, so with TAA off across
    // this matrix the marcher declares no `viewt` write.
    if row.sdf_leg {
        pass!("sdf_forward_march");
        g.image_access(
            lit,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            SubRange::COLOR,
        );
        g.image_access(
            vb_depth,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::DEPTH,
        );
    }

    // The `vb_viewt` LATE slot is gated `viewt_from_vb_depth.is_some() && ssao.is_none()`, and
    // this matrix arms `viewt_from_vb_depth` only through the SSAO disjunct, so the late slot is
    // unreachable in every row. `taa_resolve` is gated on `scene.taa`, held off.

    // `present_sample`: `lit` → SHADER_READ_ONLY_OPTIMAL for the present blit's FRAGMENT sample.
    pass!("present_sample");
    g.image_access(
        lit,
        VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
        VK_ACCESS_SHADER_READ_BIT,
        VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        SubRange::COLOR,
    );

    // `hzb_dump` — declared LAST in the whole graph, so it observes the FINISHED pyramid.
    if row.hzb_dump
        && let Some(levels) = row.hzb_levels
    {
        pass!("hzb_dump");
        g.image_access(
            vb_depth,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_ACCESS_TRANSFER_READ_BIT,
            VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
            SubRange::DEPTH,
        );
        g.image_access(
            hzb_pyramid,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_ACCESS_TRANSFER_READ_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            hzb_mips(0, levels),
        );
    }

    g.compile();
    VbFrame {
        g,
        pass_names,
        row,
        lit,
        vb_id,
        vb_depth,
        viewt,
        hzb_pyramid,
        light_table,
        vb_instance_ring,
        vb_indirect,
        vb_visible_instance,
        vb_indirect_late,
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Membership + census helpers
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Does the derived image stream contain a barrier with EXACTLY these fields?
///
/// `#[allow(clippy::too_many_arguments)]`: the argument list IS the assertion — every field of
/// `ImgBarrier` except the ones the caller is asking about. Grouping them into a struct would
/// just move the same eight values behind a constructor and make each call site longer.
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

/// Does the derived buffer stream contain a barrier with EXACTLY these fields?
fn has_buf(bs: &[BufBarrier], res: ResId, ss: u32, ds: u32, sa: u32, da: u32) -> bool {
    bs.iter()
        .any(|b| b.res == res && b.src_stage == ss && b.dst_stage == ds && b.src_access == sa && b.dst_access == da)
}

/// Every derived image barrier naming `res`, in stream order.
fn img_on(bs: &[ImgBarrier], res: ResId) -> Vec<&ImgBarrier> {
    bs.iter().filter(|b| b.res == res).collect()
}

/// Every derived buffer barrier naming `res`, in stream order.
fn buf_on(bs: &[BufBarrier], res: ResId) -> Vec<&BufBarrier> {
    bs.iter().filter(|b| b.res == res).collect()
}

/// The index of the first element that differs, or of the first index past the shorter stream
/// when the lengths differ; `None` when the two are equal.
fn first_divergence<T: PartialEq>(actual: &[T], expected: &[T]) -> Option<usize> {
    if let Some(i) = actual.iter().zip(expected.iter()).position(|(a, e)| a != e) {
        return Some(i);
    }
    (actual.len() != expected.len()).then_some(actual.len().min(expected.len()))
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The generator: measured, never predicted
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Single-BIT `VkPipelineStageFlags` → constant name, ascending bit order.
///
/// ONLY constants `use`d at the top of this file may appear here: the dumper emits these names
/// verbatim into text that must COMPILE when pasted, and this table is the compiler's own
/// witness of that — a name not in scope fails to build the table, not the paste.
const STAGE_BITS: &[(u32, &str)] = &[
    (VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, "VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT"),
    (VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT, "VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT"),
    (VK_PIPELINE_STAGE_VERTEX_SHADER_BIT, "VK_PIPELINE_STAGE_VERTEX_SHADER_BIT"),
    (VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT, "VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT"),
    (VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT, "VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT"),
    (VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT, "VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT"),
    (VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT, "VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT"),
    (VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, "VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT"),
    (VK_PIPELINE_STAGE_TRANSFER_BIT, "VK_PIPELINE_STAGE_TRANSFER_BIT"),
];

/// Single-BIT `VkAccessFlags` → constant name, ascending bit order (see [`STAGE_BITS`]).
const ACCESS_BITS: &[(u32, &str)] = &[
    (VK_ACCESS_INDIRECT_COMMAND_READ_BIT, "VK_ACCESS_INDIRECT_COMMAND_READ_BIT"),
    (VK_ACCESS_SHADER_READ_BIT, "VK_ACCESS_SHADER_READ_BIT"),
    (VK_ACCESS_SHADER_WRITE_BIT, "VK_ACCESS_SHADER_WRITE_BIT"),
    (VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT, "VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT"),
    (VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT, "VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT"),
    (VK_ACCESS_TRANSFER_READ_BIT, "VK_ACCESS_TRANSFER_READ_BIT"),
    (VK_ACCESS_TRANSFER_WRITE_BIT, "VK_ACCESS_TRANSFER_WRITE_BIT"),
];

/// Single-BIT `VkImageAspectFlags` → constant name (see [`STAGE_BITS`]).
const ASPECT_BITS: &[(u32, &str)] = &[
    (VK_IMAGE_ASPECT_COLOR_BIT, "VK_IMAGE_ASPECT_COLOR_BIT"),
    (VK_IMAGE_ASPECT_DEPTH_BIT, "VK_IMAGE_ASPECT_DEPTH_BIT"),
];

/// `VkImageLayout` value → constant name. Layouts are enum-valued, not a bit set, so this is an
/// exact-match table (see [`STAGE_BITS`]).
const LAYOUT_VALUES: &[(i32, &str)] = &[
    (VK_IMAGE_LAYOUT_UNDEFINED, "VK_IMAGE_LAYOUT_UNDEFINED"),
    (VK_IMAGE_LAYOUT_GENERAL, "VK_IMAGE_LAYOUT_GENERAL"),
    (VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, "VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL"),
    (VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, "VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL"),
    (VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, "VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL"),
    (VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, "VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL"),
];

/// A stage/access/aspect mask as a Rust expression: the `|`-joined names of the known bits, plus
/// a `0x…` literal for any bit this file has no name for, and a bare `0` for an empty mask.
///
/// The hex tail is what keeps the emitter HONEST: an unknown bit is printed rather than dropped,
/// so a pasted expectation still equals the value it was measured from.
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

/// A `VkImageLayout` as a Rust expression: the constant name if this file has one in scope, else
/// the raw literal (which still compiles, and still equals what was measured).
fn layout_expr(layout: i32) -> String {
    LAYOUT_VALUES
        .iter()
        .find(|&&(value, _)| value == layout)
        .map_or_else(|| layout.to_string(), |&(_, name)| name.to_string())
}

/// The resource's declared debug name, quoted — or a loud marker when the `ResId` is outside the
/// frame's declared range.
///
/// `FrameGraph::res_name` PANICS on an out-of-range `ResId`, and the divergence reports label the
/// EXPECTED side too, whose `ResId` comes from a hand-pasted literal. A panic while formatting a
/// failure message would replace the diagnosis with its own noise. `vb_indirect_late` is the LAST
/// resource [`declare_vb_unsplit_frame`] declares, so its index is the bound.
fn res_label(f: &VbFrame, res: ResId) -> String {
    if res.index() <= f.vb_indirect_late.index() {
        format!("{:?}", f.g.res_name(res))
    } else {
        format!("<ResId {} is outside this frame's {} resources>", res.0, f.vb_indirect_late.index() + 1)
    }
}

/// The pass name at `index`, or a loud marker when the pin outran the recorded names (same
/// reasoning as [`res_label`]: a formatter must not panic while reporting someone else's failure).
fn pass_label(f: &VbFrame, index: usize) -> String {
    f.pass_names
        .get(index)
        .map_or_else(|| format!("<no name for pass {index}>"), |name| format!("{name:?}"))
}

/// One derived [`ImgBarrier`] as a copy-pasteable Rust struct literal, with the resource name as
/// a trailing comment on the `res` line so a human diff of two dumps reads as prose.
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

/// One [`PassBarrierRange`] as a copy-pasteable Rust struct literal, labelled with its pass name.
fn pass_range_source(r: &PassBarrierRange, label: &str, index: usize) -> String {
    format!(
        "    PassBarrierRange {{ img_begin: {}, img_count: {}, buf_begin: {}, buf_count: {} }}, // [{index}] {label}\n",
        r.img_begin, r.img_count, r.buf_begin, r.buf_count,
    )
}

/// Print one row's whole compiled stream as the three expectation constants, ready to paste.
fn dump_row(row: VbUnsplitRow, prefix: &str) {
    let f = declare_vb_unsplit_frame(row);
    let img = f.g.img_barriers();
    let buf = f.g.buf_barriers();
    let passes = f.g.pass_barriers();

    println!("// ---- {} ----", row.id);
    println!("// {} image barriers, {} buffer barriers, {} pass ranges.", img.len(), buf.len(), passes.len());
    println!("const {prefix}_EXPECTED_IMG: &[ImgBarrier] = &[");
    for (i, b) in img.iter().enumerate() {
        print!("{}", img_barrier_source(b, &res_label(&f, b.res), i));
    }
    println!("];");
    println!("const {prefix}_EXPECTED_BUF: &[BufBarrier] = &[");
    for (i, b) in buf.iter().enumerate() {
        print!("{}", buf_barrier_source(b, &res_label(&f, b.res), i));
    }
    println!("];");
    println!("const {prefix}_EXPECTED_PASS: &[PassBarrierRange] = &[");
    for (i, r) in passes.iter().enumerate() {
        print!("{}", pass_range_source(r, &pass_label(&f, i), i));
    }
    println!("];");
    println!();
}

/// **GENERATOR, not a gate** — prints all four rows' compiled streams as the twelve expectation
/// constants below, ready to paste.
///
/// ```text
/// cargo test -p boyko_rhi_vulkan --test vb_barrier_stream_baseline \
///     dump_vb_unsplit_barrier_streams -- --ignored --nocapture
/// ```
///
/// `#[ignore]` because it asserts nothing: it exists so the pins' expectations are MEASURED off a
/// compile rather than predicted by whoever writes the pin. Predicting a barrier stream and then
/// confirming it is a gate wearing a prediction's clothes — the values are read off `compile()`
/// and pasted, and thereafter they say what the graph DOES.
///
/// `--nocapture` is not optional: without it libtest swallows the output and the run looks like a
/// silent pass.
#[test]
#[ignore = "generator, not a gate: prints the four baselines as Rust source; the orchestrator runs it"]
fn dump_vb_unsplit_barrier_streams() {
    println!("// ===== BEGIN dump_vb_unsplit_barrier_streams =====");
    println!("// Replace each `const U?_EXPECTED_…` array in tests/vb_barrier_stream_baseline.rs");
    println!("// (each currently holds one TBD_* sentinel) with the matching block below, KEEPING");
    println!("// the `///` doc comment already above it.");
    println!();
    dump_row(U1, "U1");
    dump_row(U2, "U2");
    dump_row(U3, "U3");
    dump_row(U4, "U4");
    println!("// ===== END dump_vb_unsplit_barrier_streams =====");
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The baselines
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// **A barrier that has not been MEASURED yet.**
///
/// Every field is its type's MAX: `ResId(u16::MAX)` names no resource in any frame graph,
/// `u32::MAX` is not a stage/access mask the frame can form, and `i32::MAX` is not a
/// `VkImageLayout`. So it cannot be mistaken for a plausible barrier and cannot satisfy any
/// comparison in the pins — an unfilled baseline fails loudly rather than asserting something
/// convenient.
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
const TBD_BUF_BARRIER: BufBarrier =
    BufBarrier { res: ResId(u16::MAX), src_stage: u32::MAX, dst_stage: u32::MAX, src_access: u32::MAX, dst_access: u32::MAX };

/// The per-pass-range unfilled sentinel — see [`TBD_IMG_BARRIER`]. `u32::MAX` begins no slice
/// into an arena the frame can fill.
const TBD_PASS_RANGE: PassBarrierRange =
    PassBarrierRange { img_begin: u32::MAX, img_count: u32::MAX, buf_begin: u32::MAX, buf_count: u32::MAX };

/// **U1's UNFILLED image baseline.** Read off [`dump_vb_unsplit_barrier_streams`] and pasted; a
/// stream derived by hand from the state machine, or by calling `compile()` a second time inside
/// the pin, would assert only that the code equals itself — both sides would move together under
/// exactly the change this pin exists to catch. Once filled these are MEASURED: do NOT edit them
/// to make a failing run green.
const U1_EXPECTED_IMG: &[ImgBarrier] = &[
    ImgBarrier {
        res: ResId(3), // [0] "cascade"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 4 },
    },
    ImgBarrier {
        res: ResId(4), // [1] "atlas"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 16 },
    },
    ImgBarrier {
        res: ResId(0), // [2] "lit"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [3] "vb_id"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [4] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [5] "vb_id"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [6] "lit"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(3), // [7] "cascade"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 4 },
    },
    ImgBarrier {
        res: ResId(4), // [8] "atlas"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 16 },
    },
    ImgBarrier {
        res: ResId(0), // [9] "lit"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
];
/// U1's UNFILLED buffer baseline — see [`U1_EXPECTED_IMG`].
const U1_EXPECTED_BUF: &[BufBarrier] = &[
    BufBarrier {
        res: ResId(15), // [0] "light_table"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_TRANSFER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(26), // [1] "vb_cull_count"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(24), // [2] "vb_batch_desc"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(23), // [3] "vb_indirect"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(16), // [4] "vb_instance_ring"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(23), // [5] "vb_indirect"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    BufBarrier {
        res: ResId(16), // [6] "vb_instance_ring"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(27), // [7] "vb_visible_instance"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(15), // [8] "light_table"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
];
/// U1's UNFILLED per-pass attribution baseline — see [`U1_EXPECTED_IMG`].
const U1_EXPECTED_PASS: &[PassBarrierRange] = &[
    PassBarrierRange { img_begin: 0, img_count: 0, buf_begin: 0, buf_count: 1 }, // [0] "light_upload"
    PassBarrierRange { img_begin: 0, img_count: 1, buf_begin: 1, buf_count: 0 }, // [1] "csm_depth"
    PassBarrierRange { img_begin: 1, img_count: 1, buf_begin: 1, buf_count: 0 }, // [2] "atlas_depth"
    PassBarrierRange { img_begin: 2, img_count: 1, buf_begin: 1, buf_count: 0 }, // [3] "vb_sky"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 0 }, // [4] "vb_indirect_upload"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 4 }, // [5] "vb_batch_cull"
    PassBarrierRange { img_begin: 3, img_count: 2, buf_begin: 5, buf_count: 3 }, // [6] "vb_raster"
    PassBarrierRange { img_begin: 5, img_count: 4, buf_begin: 8, buf_count: 1 }, // [7] "vb_resolve"
    PassBarrierRange { img_begin: 9, img_count: 1, buf_begin: 9, buf_count: 0 }, // [8] "present_sample"
];

/// U2's UNFILLED image baseline — see [`U1_EXPECTED_IMG`].
const U2_EXPECTED_IMG: &[ImgBarrier] = &[
    ImgBarrier {
        res: ResId(3), // [0] "cascade"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 4 },
    },
    ImgBarrier {
        res: ResId(4), // [1] "atlas"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 16 },
    },
    ImgBarrier {
        res: ResId(0), // [2] "lit"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [3] "vb_id"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [4] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [5] "vb_id"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [6] "lit"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(3), // [7] "cascade"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 4 },
    },
    ImgBarrier {
        res: ResId(4), // [8] "atlas"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 16 },
    },
    ImgBarrier {
        res: ResId(2), // [9] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [10] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 6, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [11] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 5, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [12] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 6, mip_count: 4, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [13] "lit"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
];
/// U2's UNFILLED buffer baseline — see [`U1_EXPECTED_IMG`].
const U2_EXPECTED_BUF: &[BufBarrier] = &[
    BufBarrier {
        res: ResId(15), // [0] "light_table"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_TRANSFER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(26), // [1] "vb_cull_count"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(24), // [2] "vb_batch_desc"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(23), // [3] "vb_indirect"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(16), // [4] "vb_instance_ring"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(23), // [5] "vb_indirect"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    BufBarrier {
        res: ResId(16), // [6] "vb_instance_ring"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(27), // [7] "vb_visible_instance"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(15), // [8] "light_table"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
];
/// U2's UNFILLED per-pass attribution baseline — see [`U1_EXPECTED_IMG`].
const U2_EXPECTED_PASS: &[PassBarrierRange] = &[
    PassBarrierRange { img_begin: 0, img_count: 0, buf_begin: 0, buf_count: 1 }, // [0] "light_upload"
    PassBarrierRange { img_begin: 0, img_count: 1, buf_begin: 1, buf_count: 0 }, // [1] "csm_depth"
    PassBarrierRange { img_begin: 1, img_count: 1, buf_begin: 1, buf_count: 0 }, // [2] "atlas_depth"
    PassBarrierRange { img_begin: 2, img_count: 1, buf_begin: 1, buf_count: 0 }, // [3] "vb_sky"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 0 }, // [4] "vb_indirect_upload"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 4 }, // [5] "vb_batch_cull"
    PassBarrierRange { img_begin: 3, img_count: 2, buf_begin: 5, buf_count: 3 }, // [6] "vb_raster"
    PassBarrierRange { img_begin: 5, img_count: 4, buf_begin: 8, buf_count: 1 }, // [7] "vb_resolve"
    PassBarrierRange { img_begin: 9, img_count: 2, buf_begin: 9, buf_count: 0 }, // [8] "hzb_build_0"
    PassBarrierRange { img_begin: 11, img_count: 2, buf_begin: 9, buf_count: 0 }, // [9] "hzb_build_1"
    PassBarrierRange { img_begin: 13, img_count: 1, buf_begin: 9, buf_count: 0 }, // [10] "present_sample"
];

/// U3's UNFILLED image baseline — see [`U1_EXPECTED_IMG`].
const U3_EXPECTED_IMG: &[ImgBarrier] = &[
    ImgBarrier {
        res: ResId(3), // [0] "cascade"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 4 },
    },
    ImgBarrier {
        res: ResId(4), // [1] "atlas"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 16 },
    },
    ImgBarrier {
        res: ResId(0), // [2] "lit"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [3] "vb_id"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [4] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [5] "vb_id"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [6] "lit"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(3), // [7] "cascade"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 4 },
    },
    ImgBarrier {
        res: ResId(4), // [8] "atlas"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 16 },
    },
    ImgBarrier {
        res: ResId(14), // [9] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 10, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [10] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [11] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 6, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [12] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 5, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [13] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 6, mip_count: 4, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [14] "lit"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [15] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_TRANSFER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [16] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_TRANSFER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 5, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [17] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_TRANSFER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 5, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [18] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_TRANSFER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 6, mip_count: 4, base_layer: 0, layer_count: 1 },
    },
];
/// U3's UNFILLED buffer baseline — see [`U1_EXPECTED_IMG`].
const U3_EXPECTED_BUF: &[BufBarrier] = &[
    BufBarrier {
        res: ResId(15), // [0] "light_table"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_TRANSFER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(26), // [1] "vb_cull_count"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(24), // [2] "vb_batch_desc"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(23), // [3] "vb_indirect"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(16), // [4] "vb_instance_ring"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(23), // [5] "vb_indirect"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    BufBarrier {
        res: ResId(16), // [6] "vb_instance_ring"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(27), // [7] "vb_visible_instance"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(15), // [8] "light_table"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
];
/// U3's UNFILLED per-pass attribution baseline — see [`U1_EXPECTED_IMG`].
const U3_EXPECTED_PASS: &[PassBarrierRange] = &[
    PassBarrierRange { img_begin: 0, img_count: 0, buf_begin: 0, buf_count: 1 }, // [0] "light_upload"
    PassBarrierRange { img_begin: 0, img_count: 1, buf_begin: 1, buf_count: 0 }, // [1] "csm_depth"
    PassBarrierRange { img_begin: 1, img_count: 1, buf_begin: 1, buf_count: 0 }, // [2] "atlas_depth"
    PassBarrierRange { img_begin: 2, img_count: 1, buf_begin: 1, buf_count: 0 }, // [3] "vb_sky"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 0 }, // [4] "vb_indirect_upload"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 4 }, // [5] "vb_batch_cull"
    PassBarrierRange { img_begin: 3, img_count: 2, buf_begin: 5, buf_count: 3 }, // [6] "vb_raster"
    PassBarrierRange { img_begin: 5, img_count: 4, buf_begin: 8, buf_count: 1 }, // [7] "vb_resolve"
    PassBarrierRange { img_begin: 9, img_count: 1, buf_begin: 9, buf_count: 0 }, // [8] "hzb_poison"
    PassBarrierRange { img_begin: 10, img_count: 2, buf_begin: 9, buf_count: 0 }, // [9] "hzb_build_0"
    PassBarrierRange { img_begin: 12, img_count: 2, buf_begin: 9, buf_count: 0 }, // [10] "hzb_build_1"
    PassBarrierRange { img_begin: 14, img_count: 1, buf_begin: 9, buf_count: 0 }, // [11] "present_sample"
    PassBarrierRange { img_begin: 15, img_count: 4, buf_begin: 9, buf_count: 0 }, // [12] "hzb_dump"
];

/// U4's UNFILLED image baseline — see [`U1_EXPECTED_IMG`].
const U4_EXPECTED_IMG: &[ImgBarrier] = &[
    ImgBarrier {
        res: ResId(3), // [0] "cascade"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 4 },
    },
    ImgBarrier {
        res: ResId(4), // [1] "atlas"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 16 },
    },
    ImgBarrier {
        res: ResId(0), // [2] "lit"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [3] "vb_id"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [4] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [5] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [6] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 6, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [7] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 5, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [8] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 6, mip_count: 4, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(5), // [9] "viewt"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [10] "vb_id"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(8), // [11] "thin_normal"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(8), // [12] "thin_normal"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(5), // [13] "viewt"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(9), // [14] "ssao"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [15] "lit"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(3), // [16] "cascade"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 4 },
    },
    ImgBarrier {
        res: ResId(4), // [17] "atlas"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 16 },
    },
    ImgBarrier {
        res: ResId(9), // [18] "ssao"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [19] "lit"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [20] "lit"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
];
/// U4's UNFILLED buffer baseline — see [`U1_EXPECTED_IMG`].
const U4_EXPECTED_BUF: &[BufBarrier] = &[
    BufBarrier {
        res: ResId(15), // [0] "light_table"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_TRANSFER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(26), // [1] "vb_cull_count"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(24), // [2] "vb_batch_desc"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(23), // [3] "vb_indirect"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(16), // [4] "vb_instance_ring"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(23), // [5] "vb_indirect"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    BufBarrier {
        res: ResId(16), // [6] "vb_instance_ring"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(27), // [7] "vb_visible_instance"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(15), // [8] "light_table"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
];
/// U4's UNFILLED per-pass attribution baseline — see [`U1_EXPECTED_IMG`].
const U4_EXPECTED_PASS: &[PassBarrierRange] = &[
    PassBarrierRange { img_begin: 0, img_count: 0, buf_begin: 0, buf_count: 1 }, // [0] "light_upload"
    PassBarrierRange { img_begin: 0, img_count: 1, buf_begin: 1, buf_count: 0 }, // [1] "csm_depth"
    PassBarrierRange { img_begin: 1, img_count: 1, buf_begin: 1, buf_count: 0 }, // [2] "atlas_depth"
    PassBarrierRange { img_begin: 2, img_count: 1, buf_begin: 1, buf_count: 0 }, // [3] "vb_sky"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 0 }, // [4] "vb_indirect_upload"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 4 }, // [5] "vb_batch_cull"
    PassBarrierRange { img_begin: 3, img_count: 2, buf_begin: 5, buf_count: 3 }, // [6] "vb_raster"
    PassBarrierRange { img_begin: 5, img_count: 2, buf_begin: 8, buf_count: 0 }, // [7] "hzb_build_0"
    PassBarrierRange { img_begin: 7, img_count: 2, buf_begin: 8, buf_count: 0 }, // [8] "hzb_build_1"
    PassBarrierRange { img_begin: 9, img_count: 1, buf_begin: 8, buf_count: 0 }, // [9] "vb_viewt"
    PassBarrierRange { img_begin: 10, img_count: 2, buf_begin: 8, buf_count: 0 }, // [10] "vb_geo"
    PassBarrierRange { img_begin: 12, img_count: 3, buf_begin: 8, buf_count: 0 }, // [11] "ssao"
    PassBarrierRange { img_begin: 15, img_count: 4, buf_begin: 8, buf_count: 1 }, // [12] "vb_shade_split"
    PassBarrierRange { img_begin: 19, img_count: 1, buf_begin: 9, buf_count: 0 }, // [13] "sdf_forward_march"
    PassBarrierRange { img_begin: 20, img_count: 1, buf_begin: 9, buf_count: 0 }, // [14] "present_sample"
];

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Divergence reporting
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// The names of the [`ImgBarrier`] fields that differ.
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
    if a.subresource != e.subresource {
        d.push("subresource");
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

/// One [`ImgBarrier`], field by field, each mask as BOTH its raw value and its `VK_*` name — the
/// raw value because a name table is a claim about the value, and a failure report is the wrong
/// place to trust one.
fn describe_img(f: &VbFrame, b: &ImgBarrier) -> String {
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
        // `saturating_add`: a formatter must not panic while reporting someone else's failure,
        // and the PINNED side of a report is a hand-pasted literal this file cannot bound.
        sub.base_mip.saturating_add(sub.mip_count),
        sub.base_layer,
        sub.base_layer.saturating_add(sub.layer_count),
    );
    s
}

/// One [`BufBarrier`], field by field (see [`describe_img`]).
fn describe_buf(f: &VbFrame, b: &BufBarrier) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "    res        = ResId({}) {}", b.res.0, res_label(f, b.res));
    let _ = writeln!(s, "    src_stage  = 0x{:08X}  {}", b.src_stage, mask_expr(b.src_stage, STAGE_BITS));
    let _ = writeln!(s, "    dst_stage  = 0x{:08X}  {}", b.dst_stage, mask_expr(b.dst_stage, STAGE_BITS));
    let _ = writeln!(s, "    src_access = 0x{:08X}  {}", b.src_access, mask_expr(b.src_access, ACCESS_BITS));
    let _ = writeln!(s, "    dst_access = 0x{:08X}  {}", b.dst_access, mask_expr(b.dst_access, ACCESS_BITS));
    s
}

/// One [`PassBarrierRange`], with its pass name.
fn describe_pass(f: &VbFrame, r: &PassBarrierRange, index: usize) -> String {
    format!(
        "    pass {} img [{}, {}) buf [{}, {})\n",
        pass_label(f, index),
        r.img_begin,
        // `saturating_add` — see [`describe_img`].
        r.img_begin.saturating_add(r.img_count),
        r.buf_begin,
        r.buf_begin.saturating_add(r.buf_count),
    )
}

/// The shared head of every divergence report: WHICH configuration moved, what diverged, where,
/// and how to act on it.
fn divergence_header(f: &VbFrame, kind: &str, index: usize, actual_len: usize, expected_len: usize) -> String {
    format!(
        "configuration {}: the compiled {kind} stream diverged from the pinned baseline at index \
         {index} (compiled {actual_len} entries, pinned {expected_len}).\n\
         This is the VG R3 P2-4 BASELINE, measured on the UNMODIFIED declarator BEFORE the \
         occlusion split existed. Synchronization validation is NOT live on this machine (the \
         plan's P2-0 RESOLVED measurement: a genuine missing barrier emitted no message and \
         changed no pixel), so this pin is the ONLY thing that can see a barrier defect here. If \
         you believe the new stream is correct, re-run `dump_vb_unsplit_barrier_streams` and \
         justify EVERY changed line — do not paste over the pin to make this green.\n",
        f.row.id
    )
}

/// The full image-stream divergence report: the first differing index, which fields moved, and
/// both sides field by field with the resource NAME on each.
fn img_divergence_report(f: &VbFrame, actual: &[ImgBarrier], expected: &[ImgBarrier], i: usize) -> String {
    let mut s = divergence_header(f, "IMAGE barrier", i, actual.len(), expected.len());
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
fn buf_divergence_report(f: &VbFrame, actual: &[BufBarrier], expected: &[BufBarrier], i: usize) -> String {
    let mut s = divergence_header(f, "BUFFER barrier", i, actual.len(), expected.len());
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

/// The full per-pass-range divergence report (see [`img_divergence_report`]). A difference here
/// with IDENTICAL barrier arrays means the barriers were RE-ATTRIBUTED to other passes — the same
/// stream recorded at different points in the frame, which is exactly what D6's block move does.
fn pass_divergence_report(f: &VbFrame, actual: &[PassBarrierRange], expected: &[PassBarrierRange], i: usize) -> String {
    let mut s = divergence_header(f, "PASS barrier-range", i, actual.len(), expected.len());
    match (actual.get(i), expected.get(i)) {
        (Some(a), Some(e)) => {
            let _ = write!(s, "  COMPILED [{i}]:\n{}", describe_pass(f, a, i));
            let _ = write!(s, "  PINNED   [{i}]:\n{}", describe_pass(f, e, i));
        }
        (Some(a), None) => {
            let _ = writeln!(s, "  the pinned stream ENDS here; the compiled one continues with:");
            let _ = write!(s, "  COMPILED [{i}]:\n{}", describe_pass(f, a, i));
        }
        (None, Some(e)) => {
            let _ = writeln!(s, "  the compiled stream ENDS here; the pin still expects:");
            let _ = write!(s, "  PINNED   [{i}]:\n{}", describe_pass(f, e, i));
        }
        (None, None) => {
            let _ = writeln!(s, "  index is past BOTH streams — bug in `first_divergence`.");
        }
    }
    s
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The four pins
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Assert one row's whole compiled stream against its pinned baseline, element for element and
/// field for field, across images, buffers AND per-pass attribution.
///
/// Also asserts the one property every UNSPLIT row shares: `vb_indirect_late` — P2-3's late
/// record array, declared and named by no pass — routes ZERO barriers. That is the structural
/// form of "nothing about the split leaks into the unarmed path", and it is the assertion P2-5
/// must flip on its own four rows while leaving these four untouched.
fn assert_row_is_pinned(
    row: VbUnsplitRow,
    expected_img: &[ImgBarrier],
    expected_buf: &[BufBarrier],
    expected_pass: &[PassBarrierRange],
) {
    // FIRST, so an unfilled baseline reports ITSELF instead of a divergence at index 0 against a
    // sentinel.
    let unfilled = expected_img.contains(&TBD_IMG_BARRIER)
        || expected_buf.contains(&TBD_BUF_BARRIER)
        || expected_pass.contains(&TBD_PASS_RANGE);
    assert!(
        !unfilled,
        "configuration {}: the barrier-stream baseline is the UNFILLED PLACEHOLDER. Run the \
         generator and paste its output over the twelve `const U?_EXPECTED_…` arrays in this \
         file:\n    \
         cargo test -p boyko_rhi_vulkan --test vb_barrier_stream_baseline \
         dump_vb_unsplit_barrier_streams -- --ignored --nocapture\n\
         (The values are MEASURED off `compile()`, never predicted — see `U1_EXPECTED_IMG`'s \
         doc. This must be done BEFORE step P2-5 exists: a baseline authored after the change \
         certifies the new behaviour instead of the old one.)",
        row.id
    );

    let f = declare_vb_unsplit_frame(row);
    let img = f.g.img_barriers();
    let buf = f.g.buf_barriers();
    let passes = f.g.pass_barriers();

    // The pass-name list labels every report below; if it has drifted from the frame the labels
    // lie, so check it before trusting anything they say.
    assert_eq!(
        passes.len(),
        f.pass_names.len(),
        "configuration {}: {} pass names recorded but {} passes compiled — every failure report \
         below would mislabel its pass",
        row.id,
        f.pass_names.len(),
        passes.len()
    );

    assert!(
        img_on(img, f.hzb_pyramid).is_empty() || row.hzb_levels.is_some(),
        "configuration {}: the pyramid ResId is named by a pass on an HZB-OFF frame",
        row.id
    );
    assert!(
        buf_on(buf, f.vb_indirect_late).is_empty(),
        "configuration {}: `vb_indirect_late` routed a barrier at P2-4, where NO pass declares an \
         access on it. The late record array is declared so the sink slot has a ResId and the \
         drift assert has something to measure; its first access arrives with P2-5's \
         `vb_indirect_late_upload`.",
        row.id
    );

    if let Some(i) = first_divergence(img, expected_img) {
        panic!("{}", img_divergence_report(&f, img, expected_img, i));
    }
    if let Some(i) = first_divergence(buf, expected_buf) {
        panic!("{}", buf_divergence_report(&f, buf, expected_buf, i));
    }
    if let Some(i) = first_divergence(passes, expected_pass) {
        panic!("{}", pass_divergence_report(&f, passes, expected_pass, i));
    }
}

/// **G4 row U1** — split off, HZB off, dump off, SSAO off, `VB × Mesh`: the shipping baseline.
#[test]
fn u1_shipping_baseline_stream_is_pinned() {
    assert_row_is_pinned(U1, U1_EXPECTED_IMG, U1_EXPECTED_BUF, U1_EXPECTED_PASS);
}

/// **G4 row U2** — split off, HZB armed, dump off, SSAO off, `VB × Mesh`: today's `vb_mesh_hzb`
/// shape.
#[test]
fn u2_hzb_armed_stream_is_pinned() {
    assert_row_is_pinned(U2, U2_EXPECTED_IMG, U2_EXPECTED_BUF, U2_EXPECTED_PASS);
}

/// **G4 row U3** — split off, HZB armed, dump ON, SSAO off, `VB × Mesh`: G5's own path.
#[test]
fn u3_hzb_dump_stream_is_pinned() {
    assert_row_is_pinned(U3, U3_EXPECTED_IMG, U3_EXPECTED_BUF, U3_EXPECTED_PASS);
}

/// **G4 row U4** — split off, HZB armed, dump off, SSAO ON, `VB × Both`: the other re-sourced
/// `vb_depth` readers.
#[test]
fn u4_ssao_and_sdf_leg_stream_is_pinned() {
    assert_row_is_pinned(U4, U4_EXPECTED_IMG, U4_EXPECTED_BUF, U4_EXPECTED_PASS);
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Named hazards — what each row exists to catch, spelled as a property
// ─────────────────────────────────────────────────────────────────────────────────────────────
//
// These are membership assertions on individual barriers, and they are a SUPERSET of nothing:
// the whole-stream pins above already contain them. They earn their place as DIAGNOSIS — when a
// stream moves, these say WHICH property broke instead of "index 17 differs" — and as the
// written form of the claim each row is in the matrix for. Every value asserted here is one the
// declarator, the plan, or an existing pin states in words; the values nobody has stated are
// left to the measured whole-stream baselines rather than predicted here.

/// **U1's claim.** The early indirect chain — upload(TRANSFER) → cull(COMPUTE) WAW, then
/// cull(COMPUTE) → raster(DRAW_INDIRECT) RAW — plus the raster's two attachment first touches
/// and the survivor list's COMPUTE → VERTEX RAW.
///
/// The two `vb_indirect` transitions are the same pair `tests/vb_indirect_barrier_chain.rs` pins
/// from the sync algebra; here they are pinned as they appear in a whole VB frame, which is the
/// thing P2-5 adds a second, parallel copy of on `vb_indirect_late`.
#[test]
fn u1_pins_the_early_indirect_chain_and_the_raster_first_touches() {
    let f = declare_vb_unsplit_frame(U1);
    let img = f.g.img_barriers();
    let buf = f.g.buf_barriers();

    assert!(
        has_buf(
            buf,
            f.vb_indirect,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
        ),
        "the upload → cull WAW on `vb_indirect` is missing: the cull overwrites word 1 of every \
         record the transfer just wrote"
    );
    assert!(
        has_buf(
            buf,
            f.vb_indirect,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
        ),
        "the cull → raster RAW on `vb_indirect` is missing or mis-sourced: with the cull declared \
         the last writer is COMPUTE, not TRANSFER, and the consumer side is the INDIRECT FETCH"
    );
    assert!(
        has_buf(
            buf,
            f.vb_visible_instance,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        ),
        "the survivor list's COMPUTE(WRITE) → VERTEX(READ) RAW is missing: `src_access = 0` here \
         would order the stages while leaving the cull's stores unflushed — a stale read behind a \
         barrier that looks entirely correct"
    );
    assert!(
        has_buf(
            buf,
            f.vb_instance_ring,
            VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            0,
            VK_ACCESS_SHADER_READ_BIT,
        ),
        "the cull's first-touch READ edge on the instance ring is missing"
    );
    assert!(
        has_buf(
            buf,
            f.vb_instance_ring,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
            0,
            VK_ACCESS_SHADER_READ_BIT,
        ),
        "the raster's ring read must extend the CULL's read visibility to VERTEX (read → read \
         carries no availability operation, so `src_access` is 0)"
    );
    assert!(
        has_buf(
            buf,
            f.light_table,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            0,
            VK_ACCESS_TRANSFER_WRITE_BIT,
        ),
        "the light table's cross-frame WAR seed (sibling shade reads → this upload) is missing"
    );
    assert!(
        has_buf(
            buf,
            f.light_table,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        ),
        "the light-table upload → shade RAW is missing"
    );

    assert!(
        has_img(
            img,
            f.vb_id,
            VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
            VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
            0,
            VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            VK_IMAGE_LAYOUT_UNDEFINED,
            VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            SubRange::COLOR,
        ),
        "`vb_id`'s raster first touch is missing"
    );
    assert!(
        has_img(
            img,
            f.vb_depth,
            VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
            FRAG,
            0,
            VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
            VK_IMAGE_LAYOUT_UNDEFINED,
            VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
            SubRange::DEPTH,
        ),
        "`vb_depth`'s raster first touch is missing"
    );
    assert!(
        has_img(
            img,
            f.vb_id,
            VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::COLOR,
        ),
        "`vb_id`'s COLOR_ATTACHMENT → SHADER_READ_ONLY hand-off into the lit producer is missing"
    );
    assert!(
        has_img(
            img,
            f.lit,
            VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            VK_IMAGE_LAYOUT_GENERAL,
            SubRange::COLOR,
        ),
        "`lit`'s sky(COLOR) → resolve(GENERAL) hand-off is missing"
    );
    assert!(
        has_img(
            img,
            f.lit,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::COLOR,
        ),
        "`lit`'s GENERAL → SHADER_READ_ONLY transition into the present sample is missing"
    );

    assert!(
        img_on(img, f.hzb_pyramid).is_empty(),
        "HZB is OFF in U1, so no pass names the pyramid and it must route ZERO barriers — that is \
         the 0%-gate the disarmed `add_image_mipped(.., 1, ..)` declaration rests on"
    );
}

/// **U2's claim.** The pyramid build chain at `levels = 10`: pass 0's merged first touch over
/// mips `[0, 6)`, pass 1's RAW over mip 5 ALONE, and pass 1's first touch over `[6, 10)` — the
/// three barriers `compile_derives_the_hzb_build_chain_at_a_real_extent` measured in isolation,
/// here inside a whole VB frame. Plus the `vb_depth` hand-off `hzb_build_0` derives out of the
/// raster.
#[test]
fn u2_pins_the_pyramid_chain_and_the_depth_handoff() {
    let f = declare_vb_unsplit_frame(U2);
    let img = f.g.img_barriers();

    assert!(
        has_img(
            img,
            f.vb_depth,
            FRAG,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
            VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
            SubRange::DEPTH,
        ),
        "the raster → `hzb_build_0` depth hand-off is missing. This is the barrier P2-5 keeps but \
         re-positions: with the poison+build block moved between the two raster scopes it is \
         derived at the same access, and the LATE raster then transitions the depth back"
    );
    assert!(
        has_img(
            img,
            f.hzb_pyramid,
            VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            0,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_UNDEFINED,
            VK_IMAGE_LAYOUT_GENERAL,
            hzb_mips(0, HZB_LEVELS_PER_PASS),
        ),
        "pass 0's six mips are all in the same state, so they must MERGE into ONE first-touch \
         barrier over [0, 6)"
    );
    assert!(
        has_img(
            img,
            f.hzb_pyramid,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_ACCESS_SHADER_READ_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            VK_IMAGE_LAYOUT_GENERAL,
            hzb_mips(HZB_LEVELS_PER_PASS - 1, 1),
        ),
        "the reduce pass's read of mip 5 must be a RAW flush over mip 5 ALONE — the per-(ResId, \
         mip) property P1-5a shipped"
    );
    assert!(
        has_img(
            img,
            f.hzb_pyramid,
            VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            0,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_UNDEFINED,
            VK_IMAGE_LAYOUT_GENERAL,
            hzb_mips(HZB_LEVELS_PER_PASS, HZB_LEVELS - HZB_LEVELS_PER_PASS),
        ),
        "mips 6..9 are a FIRST TOUCH and must transition out of UNDEFINED — the per-ResId machine \
         derived `old_layout == new_layout == GENERAL` here while the dispatch wrote them through \
         GENERAL storage descriptors"
    );
    assert_eq!(
        img_on(img, f.hzb_pyramid).len(),
        3,
        "an undumped, unpoisoned pyramid derives EXACTLY the three build-chain barriers"
    );
}

/// **U3's claim.** The poison's `UNDEFINED → GENERAL` first touch over the whole chain, the WAW
/// it turns `hzb_build_0`'s write into, and the layout pair `hzb_dump` derives on `vb_depth`.
///
/// ⚠️ The dump's `src_stage`/`src_access` are deliberately NOT asserted here: they are precisely
/// the fields P2-5 changes (the plan's S3 row — from the "already SHADER_READ_ONLY,
/// execution-only" arm to a real RAW flush out of the late raster), so writing them by hand would
/// be a prediction wearing a gate's clothes. The measured whole-stream baseline carries them.
#[test]
fn u3_pins_the_poison_first_touch_and_the_dump_layout_pair() {
    let f = declare_vb_unsplit_frame(U3);
    let img = f.g.img_barriers();

    assert!(
        has_img(
            img,
            f.hzb_pyramid,
            VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            0,
            VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_IMAGE_LAYOUT_UNDEFINED,
            VK_IMAGE_LAYOUT_GENERAL,
            hzb_mips(0, HZB_LEVELS),
        ),
        "the poison clear's UNDEFINED → GENERAL first touch over ALL {} mips is missing. GENERAL \
         is one of the two layouts `vkCmdClearColorImage` accepts and it is the layout the pyramid \
         holds for life, so no extra transition may appear anywhere",
        HZB_LEVELS
    );
    assert!(
        has_img(
            img,
            f.hzb_pyramid,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            VK_IMAGE_LAYOUT_GENERAL,
            hzb_mips(0, HZB_LEVELS_PER_PASS),
        ),
        "on a poisoned frame `hzb_build_0` must derive a real WAW flush (TRANSFER_WRITE → \
         SHADER_WRITE) instead of the first touch it derives on an undumped frame"
    );

    let depth = img_on(img, f.vb_depth);
    let dump = depth
        .iter()
        .find(|b| b.new_layout == VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL)
        .expect("invariant: the dump's depth copy must derive a transition into TRANSFER_SRC_OPTIMAL");
    assert_eq!(
        dump.old_layout, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        "on an UNSPLIT armed frame the dump finds `vb_depth` where `hzb_build_0`'s read left it. \
         An UNDEFINED oldLayout here would license discarding the depth the pyramid was built from"
    );
    assert_eq!(dump.dst_stage, VK_PIPELINE_STAGE_TRANSFER_BIT);
    assert_eq!(dump.dst_access, VK_ACCESS_TRANSFER_READ_BIT);
    assert_eq!(dump.subresource, SubRange::DEPTH);
}

/// **U4's claim, and it is a claim about a barrier that is NOT there.** On an unsplit frame
/// `vb_depth` carries EXACTLY two barriers — the raster's first touch and `hzb_build_0`'s
/// hand-off — and the two later readers this row exists for, the `vb_viewt` PRE-TAIL slot and
/// `sdf_forward_march`'s mesh arm, derive NONE: the image is already in
/// `SHADER_READ_ONLY_OPTIMAL` with COMPUTE visibility, so a same-layout same-stage read needs
/// nothing.
///
/// That absence is exactly what P2-5 must move. With the poison+build block relocated between the
/// scopes, the last toucher of `vb_depth` becomes the LATE raster with a pending write, and every
/// one of these readers changes character to a real RAW flush plus a layout transition. A gate
/// that only asserted the barriers that exist today would go green on both worlds.
#[test]
fn u4_pins_the_absent_barriers_on_the_later_depth_readers() {
    let f = declare_vb_unsplit_frame(U4);
    let img = f.g.img_barriers();

    let depth = img_on(img, f.vb_depth);
    assert_eq!(
        depth.len(),
        2,
        "`vb_depth` must carry exactly TWO barriers on an unsplit HZB-armed frame (the raster \
         first touch and the `hzb_build_0` hand-off); the `vb_viewt` PRE-TAIL read and \
         `sdf_forward_march`'s mesh-arm read must derive NONE.\n\
         ⚠️ If this reds the FIRST time it is run — i.e. before P2-5 exists — the finding is not \
         in this file: the declarator states the property in words twice (*\"every later \
         same-layout read then needs none\"*, and the dump's *\"on every armed frame that is \
         SHADER_READ_ONLY_OPTIMAL, since hzb_build_0 itself reads it there\"*), and a third \
         barrier here would falsify BOTH. Escalate the count rather than relaxing it.\n\
         Got: {depth:#?}"
    );
    assert_eq!(depth[0].old_layout, VK_IMAGE_LAYOUT_UNDEFINED);
    assert_eq!(depth[0].new_layout, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL);
    assert_eq!(depth[1].src_stage, FRAG);
    assert_eq!(depth[1].src_access, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT);
    assert_eq!(depth[1].dst_stage, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT);
    assert_eq!(depth[1].dst_access, VK_ACCESS_SHADER_READ_BIT);
    assert_eq!(depth[1].old_layout, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL);
    assert_eq!(depth[1].new_layout, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL);

    assert!(
        has_img(
            img,
            f.viewt,
            VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            0,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_UNDEFINED,
            VK_IMAGE_LAYOUT_GENERAL,
            SubRange::COLOR,
        ),
        "the PRE-TAIL `vb_viewt` pass is the frame's sole gViewT producer here, so its write must \
         be `viewt`'s UNDEFINED → GENERAL first touch"
    );
    assert!(
        has_img(
            img,
            f.lit,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            VK_IMAGE_LAYOUT_GENERAL,
            SubRange::COLOR,
        ),
        "under `Both` the marcher's `lit` write extends the split shade's GENERAL write — a \
         COMPUTE→COMPUTE WAW with NO layout change"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The red control
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// **THE CONTROL, and it is the one that decides whether this whole file is worth anything.**
///
/// The defect class piece 2 can actually ship is *read declared, write undeclared*: a consumer
/// whose producer nobody told the graph about. B2 established that it yields the SAME barrier
/// COUNT as the correct implementation and differs only in `src_stage`/`src_access` — so a
/// count-based gate goes **green on the defective frame and red on the correct one**, which is
/// how G4 was specified in round 1 and why it was rewritten to assert fields.
///
/// This reproduces that class TODAY, on the unmodified declarator, by dropping ONE declaration
/// from the replica: `vb_batch_cull`'s `buffer_access(vb_visible_instance, COMPUTE_SHADER,
/// SHADER_WRITE)`. The survivor list then has a declared READER (`vb_raster`'s VERTEX read) and
/// no declared WRITER, which is the exact shape `vb_indirect_late` would have if P2-5 declared
/// `vb_raster_late`'s indirect fetch without `vb_indirect_late_upload`'s transfer write.
///
/// # Which edit reds the four pins
///
/// Deleting that one `buffer_access` — in this replica, or in `declare_vb_graph`'s `vb_batch_cull`
/// arm, which is what the replica mirrors. The four whole-stream pins go RED on the FIELDS of one
/// buffer barrier; the image stream, both counts and the per-pass attribution are untouched.
///
/// # Why the pin's own assertion shape is what catches it
///
/// The two assertions below are the discriminator: the counts are asserted EQUAL (so a
/// count/attribution gate is demonstrably green on the defect) and the streams are asserted
/// DIFFERENT, with the corrupted barrier's source read out as `(TOP_OF_PIPE, 0)` — an execution
/// edge that orders the stages while leaving the cull's stores unflushed. On this machine nothing
/// else would notice: `robustBufferAccess` is off, the validation layers do not track buffer
/// hazards, synchronization validation is not live (P2-0), and the survivor list is currently the
/// identity, so even a stale read paints the right picture.
#[test]
fn a_dropped_writer_keeps_every_count_and_moves_only_fields() {
    let faithful = declare_vb_unsplit_frame(U1);
    let corrupt = declare_vb_unsplit_frame(VbUnsplitRow {
        id: "U1 + RED CONTROL (cull's survivor-list write undeclared)",
        red_control_drop_cull_survivor_write: true,
        ..U1
    });

    let f_img = faithful.g.img_barriers();
    let c_img = corrupt.g.img_barriers();
    let f_buf = faithful.g.buf_barriers();
    let c_buf = corrupt.g.buf_barriers();

    // (1) A COUNT gate is GREEN on the defect — image stream identical, buffer count identical,
    // per-pass attribution identical.
    assert_eq!(
        f_img, c_img,
        "the control must isolate ONE buffer declaration; the image stream moving too would mean \
         it is testing something else"
    );
    assert_eq!(
        f_buf.len(),
        c_buf.len(),
        "the whole point of this control is that the defect PRESERVES the barrier count: a \
         first-touch read emits one barrier exactly as a RAW does. If the counts differ, the \
         control no longer demonstrates the class B2 named"
    );
    assert_eq!(
        faithful.g.pass_barriers(),
        corrupt.g.pass_barriers(),
        "per-pass attribution must be identical too — the defect moves no barrier to another pass"
    );

    // (2) A FIELD gate is RED on the defect.
    assert_ne!(
        f_buf, c_buf,
        "the two buffer streams are IDENTICAL, so this file cannot tell a declared writer from a \
         missing one and every whole-stream pin above is vacuous on the class it exists for"
    );

    let faithful_survivor = buf_on(f_buf, faithful.vb_visible_instance);
    let corrupt_survivor = buf_on(c_buf, corrupt.vb_visible_instance);
    assert_eq!(faithful_survivor.len(), 1, "the survivor list carries exactly one barrier when correct");
    assert_eq!(corrupt_survivor.len(), 1, "…and exactly one when its writer is undeclared — the same count");
    assert_eq!(
        (faithful_survivor[0].src_stage, faithful_survivor[0].src_access),
        (VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT),
        "correct: the VS read is a RAW that makes the cull's stores AVAILABLE"
    );
    assert_eq!(
        (corrupt_survivor[0].src_stage, corrupt_survivor[0].src_access),
        (VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, 0),
        "defective: with no declared writer the read takes the first-touch arm — an execution-only \
         edge with nothing to flush. This is the exact fingerprint of the P2-5 defect B2 named \
         (`vb_indirect_late` fetched with its transfer write undeclared), and it is invisible to \
         every other gate on this machine"
    );
    assert_eq!(
        (corrupt_survivor[0].dst_stage, corrupt_survivor[0].dst_access),
        (faithful_survivor[0].dst_stage, faithful_survivor[0].dst_access),
        "the CONSUMER side is unchanged by the defect — which is why only the source fields can \
         discriminate, and why a gate that checks anything less than fields cannot"
    );
}

/// `compile()` is idempotent and a re-declared frame reproduces its own stream — so a divergence
/// reported by the pins above is a change in the DECLARATION, never a stale arena.
///
/// Both legs matter: recompiling one graph catches a `compile` that accumulates, and rebuilding a
/// second frame catches a `reset` that leaks (`res_shape`/`res_state_total` are a prefix sum, and
/// the pyramid is the mipped resource that would index past the end if one survived).
#[test]
fn every_row_is_reproducible_and_compile_is_idempotent() {
    for row in [U1, U2, U3, U4] {
        let mut f = declare_vb_unsplit_frame(row);
        let first: Vec<ImgBarrier> = f.g.img_barriers().to_vec();
        let first_buf: Vec<BufBarrier> = f.g.buf_barriers().to_vec();
        f.g.compile();
        assert_eq!(f.g.img_barriers(), first.as_slice(), "configuration {}: compile not idempotent", row.id);
        assert_eq!(f.g.buf_barriers(), first_buf.as_slice(), "configuration {}: compile not idempotent", row.id);

        let again = declare_vb_unsplit_frame(row);
        assert_eq!(again.g.img_barriers(), first.as_slice(), "configuration {}: rebuild diverged", row.id);
        assert_eq!(again.g.buf_barriers(), first_buf.as_slice(), "configuration {}: rebuild diverged", row.id);
    }
}

/// The four rows must be four DIFFERENT frames. A matrix whose rows compile to the same stream
/// pins one configuration four times and reports it as coverage — the vacuity mode this campaign
/// has shipped five times.
///
/// Stated as a property of the compiled streams rather than of the config structs, because it is
/// the streams the pins compare.
#[test]
fn the_four_rows_are_four_distinct_configurations() {
    /// One row's compiled streams, owned so all four can be compared pairwise.
    struct RowStream {
        row: VbUnsplitRow,
        img: Vec<ImgBarrier>,
        buf: Vec<BufBarrier>,
        passes: Vec<PassBarrierRange>,
        pass_count: usize,
    }

    let frames: Vec<RowStream> = [U1, U2, U3, U4]
        .into_iter()
        .map(|row| {
            let f = declare_vb_unsplit_frame(row);
            RowStream {
                row,
                img: f.g.img_barriers().to_vec(),
                buf: f.g.buf_barriers().to_vec(),
                passes: f.g.pass_barriers().to_vec(),
                pass_count: f.pass_names.len(),
            }
        })
        .collect();

    for (i, a) in frames.iter().enumerate() {
        for b in frames.iter().skip(i + 1) {
            assert!(
                a.img != b.img || a.buf != b.buf || a.passes != b.passes,
                "configurations {} and {} compile to the SAME barrier stream — one of them is not \
                 exercising the arm it was added for",
                a.row.id,
                b.row.id
            );
        }
    }

    // The pass counts a reader can check against the declarator by eye, so a silently unarmed
    // knob (an SSAO row that declared no `vb_geo`, an HZB row that declared no build pass) is
    // caught here rather than being absorbed into a pasted baseline.
    assert_eq!(
        frames[0].pass_count, 9,
        "U1: light_upload, csm_depth, atlas_depth, vb_sky, vb_indirect_upload, vb_batch_cull, \
         vb_raster, vb_resolve, present_sample"
    );
    assert_eq!(frames[1].pass_count, 11, "U2 adds hzb_build_0 and hzb_build_1");
    assert_eq!(frames[2].pass_count, 13, "U3 adds hzb_poison and hzb_dump on top of U2");
    assert_eq!(
        frames[3].pass_count, 15,
        "U4 drops `vb_resolve` for the split trio (vb_geo, ssao, vb_shade_split) and adds \
         `vb_viewt` (pre-tail) and `sdf_forward_march` on top of U2"
    );
}

/// `PassId` is strictly monotonic in declaration order and `compile()` does not reorder, so the
/// declare-order invariants `declare_vb_graph` asserts in production hold in this replica too —
/// and the one D6 will move is asserted here BEFORE it moves.
#[test]
fn declare_order_invariants_hold_in_the_replica() {
    let f = declare_vb_unsplit_frame(U3);
    let index_of = |name: &str| -> Option<usize> { f.pass_names.iter().position(|n| *n == name) };

    let poison = index_of("hzb_poison").expect("invariant: U3 arms the poison");
    let build0 = index_of("hzb_build_0").expect("invariant: U3 arms the build chain");
    let dump = index_of("hzb_dump").expect("invariant: U3 arms the dump");
    let raster = index_of("vb_raster").expect("invariant: every row declares the early raster");

    assert!(poison < build0, "the poison must be declared BEFORE every build pass, or it erases what the build wrote");
    assert!(build0 < dump, "the dump must observe a FINISHED pyramid");
    assert!(
        raster < build0,
        "the pyramid reduces the depth THIS FRAME'S raster wrote; a build declared first would \
         read an unwritten transient"
    );
    // The property P2-5 changes: today the whole poison+build block sits AFTER the lit producer.
    // D6 moves it whole to immediately after the early raster, and the pin above measures the
    // per-pass attribution that move rewrites.
    let lit_producer = index_of("vb_resolve").expect("invariant: U3 is a fused frame");
    assert!(
        lit_producer < poison,
        "at P2-4 the poison+build block is declared AFTER the lit producer. When this assertion \
         reverses, D6's block move has landed and the four baselines above must be re-measured \
         for the SPLIT rows only — the unsplit rows keep this order by the same single predicate"
    );
}
