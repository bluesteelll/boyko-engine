//! VG R3 piece 2 — **gate G4**: the derived barrier stream for the VisibilityBuffer frame, per
//! CONFIGURATION, asserted FIELD BY FIELD (`docs/VG-R3-P2-CAPABILITY-SPLIT-PLAN.md`, "G4").
//!
//! Eight rows, authored in two steps and in this order on purpose:
//!
//! * **U1..U4 — step P2-4**, on the UNMODIFIED declarator, before the split existed.
//! * **S1..S4 — step P2-6**, on the declarator P2-5 changed.
//!
//! # Why the halves were authored in that order, and why they must never be re-measured together
//!
//! `docs/VG-R3-P1-PYRAMID-PLAN.md` states the discipline: *"Authoring them after the change would
//! certify the new behaviour instead of the old one."* The four `U*` expectations were measured
//! against a declaration shape nobody could have tuned to the split, and the split's diff is
//! measured against them. They have two generators for that reason
//! ([`dump_vb_unsplit_barrier_streams`] / [`dump_vb_split_barrier_streams`]): "re-measure the
//! split rows" must not be able to silently also re-measure the rows they are compared against.
//!
//! # The ONE deliberate re-pin, and what makes it a re-pin rather than a re-authoring
//!
//! VG R3 piece 3 step P3-0 flipped `hzb_pyramid`'s framegraph SEED from `ResSync::undefined()` to
//! `seeded_writer_at_layout(GENERAL, COMPUTE_SHADER, SHADER_WRITE)` — the cross-frame decision D2
//! of `docs/VG-R3-P3-CULL-INTEGRATION-PLAN.md` argues, landed alone precisely because it is the
//! one change in that piece that moves an existing stream. Ten pinned rows moved with it: every
//! `hzb_pyramid` barrier that had been a FIRST TOUCH (U2 `[10]`/`[12]`, U3 `[9]`, U4 `[6]`/`[8]`,
//! S2 `[6]`/`[8]`, S3 `[5]`, S4 `[6]`/`[8]`).
//!
//! The delta was DERIVED FROM `sync::transition` BEFORE it was measured, and it is total: with the
//! seed carrying a pending flush at a real layout, the first access finds `layout_change == false`
//! and `flush_access != 0`, so it still emits a barrier (`need` is true either way) but sources it
//! from the seed's writer. `src_stage: TOP_OF_PIPE → COMPUTE_SHADER`, `src_access: 0 →
//! SHADER_WRITE`, `old_layout: UNDEFINED → GENERAL`; `dst_*`, `new_layout`, every subresource, the
//! ORDER and every COUNT unmoved, on every row. Anything else that moves is a real finding, not
//! this re-pin — which is the whole reason the prediction is written down here instead of the
//! generator's output being pasted in and trusted.
//!
//! **Three NAMED-HAZARD assertions moved with those rows**, and they are re-pinned rather than
//! deleted for the same reason: each asserted a FIRST-TOUCH property whose subject the seed
//! removed. [`u2_pins_the_pyramid_chain_and_the_depth_handoff`],
//! [`u3_pins_the_poison_whole_chain_waw_and_the_dump_layout_pair`] (RENAMED — it was
//! `u3_pins_the_poison_first_touch_…`, and a name is a claim) and
//! [`s2_pins_the_depth_round_trip_across_the_moved_block`] each carry the argument in their own
//! doc, including what they now catch and the ONE way each is weaker than what it replaced. The
//! MERGE each of them asserts — six mips into one barrier, ten into one — is unaffected by the
//! seed and is stated more sharply than before: a seeded chain still folds, and if it stopped
//! folding that would be a finding about `compile`, not something to re-pin around.
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
//!   4. G2 (step P2-6, `boyko_app/tests/vb_occ_split_gate.rs`) contributes a scope count that
//!      originates in the RECORDER. Replica pin, production asserts and recorder count are the
//!      evidence together; none of them alone is.
//! * **Nothing about recording.** The pin stops at the derived plan; how `record_all` batches it
//!   into `vkCmdPipelineBarrier` array calls is a different question, and whether `record_vb`
//!   records the scope at all is gate G2's.
//! * **Nothing about pixels, and nothing about soundness.** A stream can be pinned and wrong.
//!   This says "this is what the machine derives, and it has not moved".
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

/// One row of G4's matrix. The knobs are exactly the columns G4 varies; everything else is held
/// FIXED across all eight rows (see [`declare_vb_frame`]), so a row-to-row difference in the
/// pinned stream is attributable to the row's own column.
///
/// ⚠️ At P2-4 this type was `VbRow` and carried no `split` field **on purpose** — the split
/// did not exist, and a knob for it would have been a place for P2-5 to be tuned into the
/// baseline. P2-5 landed; P2-6 adds the field and the four `S*` rows. **The four `U*` rows and
/// their twelve measured expectation arrays are UNTOUCHED**, which is itself an assertion: an
/// unsplit frame must derive the stream that was pinned before the split existed, and `split:
/// false` reaching any declaration below would show up there first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct VbRow {
    /// G4's row label, used in every failure message so a divergence names WHICH configuration
    /// moved rather than "the stream drifted".
    id: &'static str,
    /// `GBufferScene::path_vb_occlusion_split()` — arms `vb_indirect_late_upload`, `vb_raster_late`
    /// and the `[hzb_poison, hzb_build_*]` block's EARLY slot (plan D4/D6). False on every scene in
    /// the tree today, since nothing carries `OcclusionCulling`.
    split: bool,
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
    /// **RED CONTROL ONLY (G4's R1).** Drops `vb_indirect_late_upload`'s declared TRANSFER write
    /// while leaving `vb_raster_late`'s indirect fetch in place — the SAME defect class, on the
    /// resource piece 2 actually adds. Every pinned row holds this `false`; only
    /// [`a_dropped_late_upload_write_keeps_the_count_and_moves_only_fields`] sets it.
    red_control_drop_late_upload_write: bool,
}

/// **U1** — split off, HZB off, dump off, SSAO off, `VB × Mesh`. The shipping baseline: nothing
/// about the split leaks into the unarmed path.
const U1: VbRow = VbRow {
    id: "U1 (split off, HZB off, dump off, SSAO off, VB×Mesh)",
    split: false,
    hzb_levels: None,
    hzb_dump: false,
    ssao: false,
    sdf_leg: false,
    red_control_drop_cull_survivor_write: false,
    red_control_drop_late_upload_write: false,
};

/// **U2** — split off, HZB armed, dump off, SSAO off, `VB × Mesh`. Today's `vb_mesh_hzb` shape,
/// including the three pyramid barriers at `levels = 10`.
const U2: VbRow = VbRow {
    id: "U2 (split off, HZB armed, dump off, SSAO off, VB×Mesh)",
    hzb_levels: Some(HZB_LEVELS),
    ..U1
};

/// **U3** — split off, HZB armed, dump ON, SSAO off, `VB × Mesh`. G5's own path: `hzb_poison`'s
/// one whole-chain `GENERAL → GENERAL` clear barrier, and `hzb_dump`'s `vb_depth` source — the
/// source P2-5 re-sources.
///
/// ⚠️ This line read *"`hzb_poison`'s `UNDEFINED → GENERAL` first touch"* until VG R3 P3-0, whose
/// seed removed the first touch (see the module doc's re-pin section and
/// [`u3_pins_the_poison_whole_chain_waw_and_the_dump_layout_pair`]).
const U3: VbRow = VbRow {
    id: "U3 (split off, HZB armed, dump ON, SSAO off, VB×Mesh)",
    hzb_dump: true,
    ..U2
};

/// **U4** — split off, HZB armed, dump off, SSAO ON, `VB × Both`. The other re-sourced
/// `vb_depth` readers: the `vb_viewt` PRE-TAIL slot and `sdf_forward_march`'s mesh arm.
const U4: VbRow = VbRow {
    id: "U4 (split off, HZB armed, dump off, SSAO ON, VB×Both)",
    ssao: true,
    sdf_leg: true,
    ..U2
};

/// **S1** (VG R3 piece 2 step P2-6) — split **ON**, HZB off, dump off, SSAO off, `VB × Mesh`.
///
/// The three new barriers at the late scope's boundary, and the row where they stand alone:
/// `vb_id` WAW, `vb_depth` WAW, and `vb_indirect_late`'s `TRANSFER_WRITE → INDIRECT_COMMAND_READ`.
/// ⚠️ The COUNT of three is not evidence — the read-declared/write-undeclared defect yields three
/// as well, differing only in `src_stage`/`src_access`. See
/// [`s1_pins_the_late_boundary_barriers_field_by_field`] and the R1 control.
const S1: VbRow = VbRow {
    id: "S1 (split ON, HZB off, dump off, SSAO off, VB×Mesh)",
    split: true,
    ..U1
};

/// **S2** — split ON, HZB armed, dump off, SSAO off, `VB × Mesh`. The depth ROUND TRIP across the
/// moved poison+build block: `DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL` into
/// `hzb_build_0`, then back into `vb_raster_late` — neither of them a first touch, because a first
/// touch here would license DISCARDING the early scope's depth.
const S2: VbRow = VbRow {
    id: "S2 (split ON, HZB armed, dump off, SSAO off, VB×Mesh)",
    split: true,
    ..U2
};

/// **S3** — split ON, HZB armed, dump ON, SSAO off, `VB × Mesh`. **Mandatory**: `hzb_dump` is one
/// of the four `vb_depth` readers the block move re-sources, and it is gate G5's own path.
const S3: VbRow = VbRow {
    id: "S3 (split ON, HZB armed, dump ON, SSAO off, VB×Mesh)",
    split: true,
    ..U3
};

/// **S4** — split ON, HZB armed, dump off, SSAO ON, `VB × Both`. The other two re-sourced
/// readers: the `vb_viewt` PRE-TAIL slot and `sdf_forward_march`'s mesh arm.
const S4: VbRow = VbRow {
    id: "S4 (split ON, HZB armed, dump off, SSAO ON, VB×Both)",
    split: true,
    ..U4
};

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The replica
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A compiled replica frame plus the handles the named-hazard assertions read.
///
/// Only the `ResId`s an assertion names are carried; the rest are declared as `_`-prefixed
/// locals in [`declare_vb_frame`], where the underscore records "declared for `ResId`
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
    row: VbRow,
    lit: ResId,
    vb_id: ResId,
    vb_depth: ResId,
    viewt: ResId,
    hzb_pyramid: ResId,
    light_table: ResId,
    vb_instance_ring: ResId,
    vb_indirect: ResId,
    vb_visible_instance: ResId,
    /// P2-3's late record array. On an UNSPLIT row it is declared and named by NO pass, and the
    /// row asserts it routes ZERO barriers — the structural form of "nothing about the split leaks
    /// into the unarmed path". On a SPLIT row it carries exactly one:
    /// `vb_indirect_late_upload`'s TRANSFER write → `vb_raster_late`'s indirect fetch.
    vb_indirect_late: ResId,
}

/// The `[hzb_poison, hzb_build_0 .. hzb_build_{n-1}]` block, declared WHOLE — the replica's mirror
/// of `graph_bridge.rs`'s `declare_hzb_poison_build`.
///
/// It is one function for the reason the production one is: the block has two slots (plan D6) and
/// **moving the builds without their clear is not expressible** here either. `PassId` is strictly
/// monotonic in declare order and `compile()` does not reorder, so a build declared ahead of its
/// poison would have the clear ERASE what the dispatches wrote — the defect the declarator's
/// `poison < build` assert exists to refuse.
///
/// `names` is threaded beside `g` rather than captured, because the caller's `pass!` macro owns
/// both and a closure over them would borrow twice.
fn declare_hzb_poison_build_replica(
    g: &mut FrameGraph,
    names: &mut Vec<&'static str>,
    hzb_levels: Option<u32>,
    dump_armed: bool,
    hzb_pyramid: ResId,
    vb_depth: ResId,
) {
    // The poison arms on EXACTLY `hzb_levels.is_some() && dump_armed` — the dump pass's own
    // predicate, verbatim: a frame that is poisoned is always a frame that is dumped.
    if let (Some(levels), true) = (hzb_levels, dump_armed) {
        names.push("hzb_poison");
        g.add_pass("hzb_poison");
        g.image_access(
            hzb_pyramid,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            hzb_mips(0, levels),
        );
    }

    let Some(levels) = hzb_levels else {
        return;
    };
    let pass_count = levels.div_ceil(HZB_LEVELS_PER_PASS) as usize;
    assert!(
        pass_count <= MAX_HZB_PASSES,
        "the row's level count needs {pass_count} passes, more than MAX_HZB_PASSES"
    );
    // Iterated by NAME rather than by index: the declarator's own loop is
    // `for p in 0..pass_count { g.add_pass(HZB_BUILD_PASS_NAMES[p]) }`, and the replica must walk
    // the same names in the same order. `.take(pass_count)` is what keeps the two in step — the
    // array is `MAX_HZB_PASSES` long (a CAPACITY) while `pass_count` is the live span, the
    // distinction `MAX_HZB_LEVELS`' own doc warns about.
    for (p, pass_name) in HZB_BUILD_PASS_NAMES.iter().enumerate().take(pass_count) {
        let d = p as u32 * HZB_LEVELS_PER_PASS;
        let n = (levels - d).min(HZB_LEVELS_PER_PASS);
        names.push(*pass_name);
        g.add_pass(pass_name);
        if p == 0 {
            // The SOURCE depth, at the same (stage, access, layout, aspect) shape `vb_viewt` /
            // `sdf_forward_march` declare. At the UNSPLIT slot THIS is the access that derives
            // `vb_depth`'s DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL transition, and
            // every later same-layout read then needs none. At the ARMED-SPLIT slot "later" stops
            // meaning "for the rest of the frame": `vb_raster_late` writes the depth again
            // immediately after this block, so every downstream reader is re-sourced from a real
            // RAW flush plus a preserving layout transition.
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

/// Declare and compile one row of G4's matrix, mirroring `declare_vb_graph`'s declaration order
/// pass for pass and access for access.
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
fn declare_vb_frame(row: VbRow) -> VbFrame {
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
    //
    // VG R3 piece 3 step P3-0 flipped the SEED from `ResSync::undefined()` to the cross-frame
    // writer form the declarator now uses (plan D2's two-residual argument, stated in full at the
    // production call site). The replica must mirror it or this pin certifies a stream nothing
    // derives. What it MOVES, derived from `sync::transition` BEFORE it was measured: every
    // `hzb_pyramid` barrier that was a FIRST TOUCH — `src_stage = TOP_OF_PIPE`, `src_access = 0`,
    // `old_layout = UNDEFINED` — becomes `src_stage = COMPUTE_SHADER`, `src_access = SHADER_WRITE`,
    // `old_layout = GENERAL` (the seed's pending flush is now the src, and the layout no longer
    // changes). Counts, order, `dst_*`, `new_layout` and every subresource are UNMOVED, on every
    // row, because `need` is true either way and the state advance is identical. A disarmed row is
    // untouched entirely: no pass names the ResId, so it routes zero barriers whatever the seed.
    let hzb_pyramid = g.add_image_mipped(
        "hzb_pyramid",
        row.hzb_levels.unwrap_or(1),
        ResSync::seeded_writer_at_layout(
            VK_IMAGE_LAYOUT_GENERAL,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
        ),
    );

    // ---- Buffers, in the declarator's FIXED order ------------------------------------------
    let light_table = g.add_buffer_seeded(
        "light_table",
        ResSync::seeded_readers(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT),
    );
    // VG R3 P2-8: the ring is `add_buffer_seeded(.., undefined())`, mirroring the declarator —
    // VB v1 has no `interp` pass, so the ring is HOST-scattered and every declared access to it
    // in this replica (and in `declare_vb_graph`) is a READ. The seed VALUE is `undefined()`,
    // the same state the bare declarator used, so every pinned row below is unmoved; what the
    // spelling buys is that `compile`'s unwritten-read guard, which now covers buffers, does not
    // fire on a resource whose content legitimately comes from outside the graph. `gclassify`
    // stays bare here for the same reason it does in the declarator (its producer is in-graph);
    // no pass in this replica names it at all.
    let vb_instance_ring = g.add_buffer_seeded("vb_instance_ring", ResSync::undefined());
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

    // `vb_indirect_late_upload` (P2-5) — the LATE record array's `vkCmdUpdateBuffer`, in its own
    // pass because its gate is `path_vb_occlusion_split()` while the upload above is gated on
    // `scene.vb_indirect.is_some()`; folding two predicates onto one pass is how a pass ends up
    // declaring an access the recorder does not perform.
    //
    // ⚠️ THIS DECLARATION IS THE DIFFERENCE BETWEEN "DRAWS NOTHING" AND "DRAWS WHATEVER WAS IN
    // FRESHLY ALLOCATED DEVICE MEMORY" — and dropping it costs the stream NO barrier and NO count,
    // only two fields. That is what `red_control_drop_late_upload_write` reproduces.
    if row.split {
        pass!("vb_indirect_late_upload");
        if !row.red_control_drop_late_upload_write {
            g.buffer_access(
                vb_indirect_late,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_WRITE_BIT,
            );
        }
    }

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

    // `vb_raster` — the EARLY scope. On a split row `vb_raster_late` follows it below, past the
    // poison+build block; on an unsplit row it is the frame's only raster scope.
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

    // ---- The poison+build block's ARMED-SPLIT slot, and the LATE raster scope (P2-5, D6/D4) ----
    //
    // The pyramid must reduce the depth the EARLY scope wrote, which fixes the armed order as
    // `vb_raster → hzb_poison → hzb_build_* → vb_raster_late`. ONE predicate picks the block's
    // slot at both declare and record; the accesses are identical in both slots and only the
    // position differs (the `vb_viewt` PRE-TAIL/LATE idiom).
    if row.split {
        declare_hzb_poison_build_replica(
            &mut g,
            &mut pass_names,
            row.hzb_levels,
            row.hzb_dump,
            hzb_pyramid,
            vb_depth,
        );

        // `vb_raster_late` — THREE accesses, each load-bearing:
        //  * `vb_indirect_late` at DRAW_INDIRECT/INDIRECT_COMMAND_READ — the consumer half of the
        //    transfer write above; either half alone derives a barrier that is WRONG rather than
        //    absent.
        //  * `vb_id` as a COLOR write at `COLOR_ATTACHMENT_OPTIMAL` — a WAW against the early
        //    scope's store, at the layout the early scope left it in. NOT `UNDEFINED`: a first
        //    touch would license DISCARDING what the early scope wrote, which is the equivalence
        //    this whole piece rests on.
        //  * `vb_depth` as a DEPTH write at `DEPTH_ATTACHMENT_OPTIMAL` — the same WAW, and on an
        //    HZB-armed row the RETURN half of the round trip through `hzb_build_0`'s read.
        //
        // Deliberately NOT declared (and P2-5's declarator says so in the same words): the VS's
        // `vb_instance_ring` and `vb_visible_instance` reads. Every late record carries
        // `instanceCount = 0`, so this scope issues zero vertex invocations and performs neither
        // read; declaring them would declare an access the recorder does not perform.
        pass!("vb_raster_late");
        g.buffer_access(
            vb_indirect_late,
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
    }

    // The classify chain is skipped: `use_classified` is off across the matrix, and the R9b split
    // (row U4) displaces it regardless (`path_vb_fused()` is false under the split).

    // The FUSED `lit` producer. Under the R9b split NEITHER `vb_resolve` nor `vb_shade` runs —
    // `vb_shade_split`, declared further down, is the producer.
    // The rung-R9b GEO/SHADE split (NOT the occlusion split — `row.split` is that one; the two
    // are independent and S4 arms both).
    let geo_shade_split = row.ssao;
    if !geo_shade_split {
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

    // ---- The pyramid POISON + BUILD block, at the UNSPLIT slot -------------------------------
    // After the `lit` producer, before the `vb_viewt` PRE-TAIL slot: the position the block has
    // held since piece 1, and where it stays on every frame the occlusion split is not armed —
    // which is every scene in this tree today. The four `U*` baselines were measured with it
    // here, so `!row.split` keeping them byte-identical is part of what P2-6 asserts.
    if !row.split {
        declare_hzb_poison_build_replica(
            &mut g,
            &mut pass_names,
            row.hzb_levels,
            row.hzb_dump,
            hzb_pyramid,
            vb_depth,
        );
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
    if geo_shade_split {
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
/// resource [`declare_vb_frame`] declares, so its index is the bound.
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
fn dump_row(row: VbRow, prefix: &str) {
    let f = declare_vb_frame(row);
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

/// **GENERATOR, not a gate** — the UNSPLIT half: prints U1..U4's compiled streams as the twelve
/// `U?_EXPECTED_…` constants below, ready to paste.
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

/// **GENERATOR, not a gate** — the SPLIT half (VG R3 piece 2 step P2-6), printing S1..S4's
/// compiled streams as the twelve `S?_EXPECTED_…` constants, ready to paste.
///
/// ```text
/// cargo test -p boyko_rhi_vulkan --test vb_barrier_stream_baseline \
///     dump_vb_split_barrier_streams -- --ignored --nocapture
/// ```
///
/// A SECOND generator rather than four more rows in the first one: the unsplit baselines were
/// measured on the UNMODIFIED declarator at P2-4 and must never be re-measured casually (that is
/// the whole authoring-order discipline — a baseline authored after the change certifies the new
/// behaviour). Two generators keep "re-measure the split rows" from silently also re-measuring the
/// four rows the split is being compared against.
#[test]
#[ignore = "generator, not a gate: prints the four SPLIT streams as Rust source; the orchestrator runs it"]
fn dump_vb_split_barrier_streams() {
    println!("// ===== BEGIN dump_vb_split_barrier_streams =====");
    println!("// Replace each `const S?_EXPECTED_…` array in tests/vb_barrier_stream_baseline.rs");
    println!("// (each currently holds one TBD_* sentinel) with the matching block below, KEEPING");
    println!("// the `///` doc comment already above it. Do NOT touch the U? arrays.");
    println!();
    dump_row(S1, "S1");
    dump_row(S2, "S2");
    dump_row(S3, "S3");
    dump_row(S4, "S4");
    println!("// ===== END dump_vb_split_barrier_streams =====");
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
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
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
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
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
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
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
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
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
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
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

/// **S1's image baseline** (VG R3 piece 2 step P2-6). Read off [`dump_vb_split_barrier_streams`]
/// and pasted — MEASURED off `compile()`, never predicted, for the reason [`U1_EXPECTED_IMG`]
/// states. Unlike the `U*` arrays, these four rows are measured on the declarator P2-5 already
/// changed, so what they pin is "this is what the split derives", checked against the four
/// UNSPLIT rows that were pinned before it existed.
const S1_EXPECTED_IMG: &[ImgBarrier] = &[
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
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [6] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [7] "vb_id"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [8] "lit"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(3), // [9] "cascade"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 4 },
    },
    ImgBarrier {
        res: ResId(4), // [10] "atlas"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 16 },
    },
    ImgBarrier {
        res: ResId(0), // [11] "lit"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
];
/// S1's buffer baseline — see [`S1_EXPECTED_IMG`].
const S1_EXPECTED_BUF: &[BufBarrier] = &[
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
        res: ResId(28), // [8] "vb_indirect_late"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    BufBarrier {
        res: ResId(15), // [9] "light_table"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
];
/// S1's per-pass attribution baseline — see [`S1_EXPECTED_IMG`].
const S1_EXPECTED_PASS: &[PassBarrierRange] = &[
    PassBarrierRange { img_begin: 0, img_count: 0, buf_begin: 0, buf_count: 1 }, // [0] "light_upload"
    PassBarrierRange { img_begin: 0, img_count: 1, buf_begin: 1, buf_count: 0 }, // [1] "csm_depth"
    PassBarrierRange { img_begin: 1, img_count: 1, buf_begin: 1, buf_count: 0 }, // [2] "atlas_depth"
    PassBarrierRange { img_begin: 2, img_count: 1, buf_begin: 1, buf_count: 0 }, // [3] "vb_sky"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 0 }, // [4] "vb_indirect_upload"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 0 }, // [5] "vb_indirect_late_upload"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 4 }, // [6] "vb_batch_cull"
    PassBarrierRange { img_begin: 3, img_count: 2, buf_begin: 5, buf_count: 3 }, // [7] "vb_raster"
    PassBarrierRange { img_begin: 5, img_count: 2, buf_begin: 8, buf_count: 1 }, // [8] "vb_raster_late"
    PassBarrierRange { img_begin: 7, img_count: 4, buf_begin: 9, buf_count: 1 }, // [9] "vb_resolve"
    PassBarrierRange { img_begin: 11, img_count: 1, buf_begin: 10, buf_count: 0 }, // [10] "present_sample"
];

/// S2's image baseline — see [`S1_EXPECTED_IMG`].
const S2_EXPECTED_IMG: &[ImgBarrier] = &[
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
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
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
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 6, mip_count: 4, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [9] "vb_id"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [10] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [11] "vb_id"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [12] "lit"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(3), // [13] "cascade"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 4 },
    },
    ImgBarrier {
        res: ResId(4), // [14] "atlas"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 16 },
    },
    ImgBarrier {
        res: ResId(0), // [15] "lit"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
];
/// S2's buffer baseline — see [`S1_EXPECTED_IMG`].
const S2_EXPECTED_BUF: &[BufBarrier] = &[
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
        res: ResId(28), // [8] "vb_indirect_late"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    BufBarrier {
        res: ResId(15), // [9] "light_table"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
];
/// S2's per-pass attribution baseline — see [`S1_EXPECTED_IMG`].
const S2_EXPECTED_PASS: &[PassBarrierRange] = &[
    PassBarrierRange { img_begin: 0, img_count: 0, buf_begin: 0, buf_count: 1 }, // [0] "light_upload"
    PassBarrierRange { img_begin: 0, img_count: 1, buf_begin: 1, buf_count: 0 }, // [1] "csm_depth"
    PassBarrierRange { img_begin: 1, img_count: 1, buf_begin: 1, buf_count: 0 }, // [2] "atlas_depth"
    PassBarrierRange { img_begin: 2, img_count: 1, buf_begin: 1, buf_count: 0 }, // [3] "vb_sky"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 0 }, // [4] "vb_indirect_upload"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 0 }, // [5] "vb_indirect_late_upload"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 4 }, // [6] "vb_batch_cull"
    PassBarrierRange { img_begin: 3, img_count: 2, buf_begin: 5, buf_count: 3 }, // [7] "vb_raster"
    PassBarrierRange { img_begin: 5, img_count: 2, buf_begin: 8, buf_count: 0 }, // [8] "hzb_build_0"
    PassBarrierRange { img_begin: 7, img_count: 2, buf_begin: 8, buf_count: 0 }, // [9] "hzb_build_1"
    PassBarrierRange { img_begin: 9, img_count: 2, buf_begin: 8, buf_count: 1 }, // [10] "vb_raster_late"
    PassBarrierRange { img_begin: 11, img_count: 4, buf_begin: 9, buf_count: 1 }, // [11] "vb_resolve"
    PassBarrierRange { img_begin: 15, img_count: 1, buf_begin: 10, buf_count: 0 }, // [12] "present_sample"
];

/// S3's image baseline — see [`S1_EXPECTED_IMG`].
const S3_EXPECTED_IMG: &[ImgBarrier] = &[
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
        res: ResId(14), // [5] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 10, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [6] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [7] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 6, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [8] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 5, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [9] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 6, mip_count: 4, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [10] "vb_id"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [11] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [12] "vb_id"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [13] "lit"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(3), // [14] "cascade"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 4 },
    },
    ImgBarrier {
        res: ResId(4), // [15] "atlas"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 16 },
    },
    ImgBarrier {
        res: ResId(0), // [16] "lit"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [17] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_TRANSFER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [18] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_TRANSFER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 5, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [19] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_TRANSFER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 5, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [20] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_TRANSFER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 6, mip_count: 4, base_layer: 0, layer_count: 1 },
    },
];
/// S3's buffer baseline — see [`S1_EXPECTED_IMG`].
const S3_EXPECTED_BUF: &[BufBarrier] = &[
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
        res: ResId(28), // [8] "vb_indirect_late"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    BufBarrier {
        res: ResId(15), // [9] "light_table"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
];
/// S3's per-pass attribution baseline — see [`S1_EXPECTED_IMG`].
const S3_EXPECTED_PASS: &[PassBarrierRange] = &[
    PassBarrierRange { img_begin: 0, img_count: 0, buf_begin: 0, buf_count: 1 }, // [0] "light_upload"
    PassBarrierRange { img_begin: 0, img_count: 1, buf_begin: 1, buf_count: 0 }, // [1] "csm_depth"
    PassBarrierRange { img_begin: 1, img_count: 1, buf_begin: 1, buf_count: 0 }, // [2] "atlas_depth"
    PassBarrierRange { img_begin: 2, img_count: 1, buf_begin: 1, buf_count: 0 }, // [3] "vb_sky"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 0 }, // [4] "vb_indirect_upload"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 0 }, // [5] "vb_indirect_late_upload"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 4 }, // [6] "vb_batch_cull"
    PassBarrierRange { img_begin: 3, img_count: 2, buf_begin: 5, buf_count: 3 }, // [7] "vb_raster"
    PassBarrierRange { img_begin: 5, img_count: 1, buf_begin: 8, buf_count: 0 }, // [8] "hzb_poison"
    PassBarrierRange { img_begin: 6, img_count: 2, buf_begin: 8, buf_count: 0 }, // [9] "hzb_build_0"
    PassBarrierRange { img_begin: 8, img_count: 2, buf_begin: 8, buf_count: 0 }, // [10] "hzb_build_1"
    PassBarrierRange { img_begin: 10, img_count: 2, buf_begin: 8, buf_count: 1 }, // [11] "vb_raster_late"
    PassBarrierRange { img_begin: 12, img_count: 4, buf_begin: 9, buf_count: 1 }, // [12] "vb_resolve"
    PassBarrierRange { img_begin: 16, img_count: 1, buf_begin: 10, buf_count: 0 }, // [13] "present_sample"
    PassBarrierRange { img_begin: 17, img_count: 4, buf_begin: 10, buf_count: 0 }, // [14] "hzb_dump"
];

/// S4's image baseline — see [`S1_EXPECTED_IMG`].
const S4_EXPECTED_IMG: &[ImgBarrier] = &[
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
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
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
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 6, mip_count: 4, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [9] "vb_id"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [10] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [11] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(5), // [12] "viewt"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [13] "vb_id"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(8), // [14] "thin_normal"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(8), // [15] "thin_normal"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(5), // [16] "viewt"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(9), // [17] "ssao"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [18] "lit"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(3), // [19] "cascade"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 4 },
    },
    ImgBarrier {
        res: ResId(4), // [20] "atlas"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 16 },
    },
    ImgBarrier {
        res: ResId(9), // [21] "ssao"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [22] "lit"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [23] "lit"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
];
/// S4's buffer baseline — see [`S1_EXPECTED_IMG`].
const S4_EXPECTED_BUF: &[BufBarrier] = &[
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
        res: ResId(28), // [8] "vb_indirect_late"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    BufBarrier {
        res: ResId(15), // [9] "light_table"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
];
/// S4's per-pass attribution baseline — see [`S1_EXPECTED_IMG`].
const S4_EXPECTED_PASS: &[PassBarrierRange] = &[
    PassBarrierRange { img_begin: 0, img_count: 0, buf_begin: 0, buf_count: 1 }, // [0] "light_upload"
    PassBarrierRange { img_begin: 0, img_count: 1, buf_begin: 1, buf_count: 0 }, // [1] "csm_depth"
    PassBarrierRange { img_begin: 1, img_count: 1, buf_begin: 1, buf_count: 0 }, // [2] "atlas_depth"
    PassBarrierRange { img_begin: 2, img_count: 1, buf_begin: 1, buf_count: 0 }, // [3] "vb_sky"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 0 }, // [4] "vb_indirect_upload"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 0 }, // [5] "vb_indirect_late_upload"
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 4 }, // [6] "vb_batch_cull"
    PassBarrierRange { img_begin: 3, img_count: 2, buf_begin: 5, buf_count: 3 }, // [7] "vb_raster"
    PassBarrierRange { img_begin: 5, img_count: 2, buf_begin: 8, buf_count: 0 }, // [8] "hzb_build_0"
    PassBarrierRange { img_begin: 7, img_count: 2, buf_begin: 8, buf_count: 0 }, // [9] "hzb_build_1"
    PassBarrierRange { img_begin: 9, img_count: 2, buf_begin: 8, buf_count: 1 }, // [10] "vb_raster_late"
    PassBarrierRange { img_begin: 11, img_count: 2, buf_begin: 9, buf_count: 0 }, // [11] "vb_viewt"
    PassBarrierRange { img_begin: 13, img_count: 2, buf_begin: 9, buf_count: 0 }, // [12] "vb_geo"
    PassBarrierRange { img_begin: 15, img_count: 3, buf_begin: 9, buf_count: 0 }, // [13] "ssao"
    PassBarrierRange { img_begin: 18, img_count: 4, buf_begin: 9, buf_count: 1 }, // [14] "vb_shade_split"
    PassBarrierRange { img_begin: 22, img_count: 1, buf_begin: 10, buf_count: 0 }, // [15] "sdf_forward_march"
    PassBarrierRange { img_begin: 23, img_count: 1, buf_begin: 10, buf_count: 0 }, // [16] "present_sample"
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
         The UNSPLIT (`U*`) baselines were measured on the UNMODIFIED declarator BEFORE the \
         occlusion split existed; the SPLIT (`S*`) ones on the declarator P2-5 changed. \
         Synchronization validation is NOT live on this machine (the plan's P2-0 RESOLVED \
         measurement: a genuine missing barrier emitted no message and changed no pixel), so this \
         pin is the ONLY thing that can see a barrier defect here. If you believe the new stream \
         is correct, re-run the generator for THIS half ({}) and justify EVERY changed line — do \
         not paste over the pin to make this green.\n",
        f.row.id,
        if f.row.split { "dump_vb_split_barrier_streams" } else { "dump_vb_unsplit_barrier_streams" }
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
// The eight pins
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Assert one row's whole compiled stream against its pinned baseline, element for element and
/// field for field, across images, buffers AND per-pass attribution.
///
/// Also asserts the one property that separates the two halves of the matrix: `vb_indirect_late`
/// routes ZERO barriers on an UNSPLIT row (the structural form of "nothing about the split leaks
/// into the unarmed path") and EXACTLY ONE on a split row. The field values of that one are
/// [`s1_pins_the_late_boundary_barriers_field_by_field`]'s subject; here only its existence is
/// claimed, because this is the assertion whose two halves must not be confusable.
fn assert_row_is_pinned(
    row: VbRow,
    expected_img: &[ImgBarrier],
    expected_buf: &[BufBarrier],
    expected_pass: &[PassBarrierRange],
) {
    // FIRST, so an unfilled baseline reports ITSELF instead of a divergence at index 0 against a
    // sentinel.
    let unfilled = expected_img.contains(&TBD_IMG_BARRIER)
        || expected_buf.contains(&TBD_BUF_BARRIER)
        || expected_pass.contains(&TBD_PASS_RANGE);
    let generator = if row.split {
        "dump_vb_split_barrier_streams"
    } else {
        "dump_vb_unsplit_barrier_streams"
    };
    assert!(
        !unfilled,
        "configuration {}: the barrier-stream baseline is the UNFILLED PLACEHOLDER. Run the \
         generator for THIS half of the matrix and paste its output over the twelve \
         `const {}?_EXPECTED_…` arrays in this file:\n    \
         cargo test -p boyko_rhi_vulkan --test vb_barrier_stream_baseline \
         {generator} -- --ignored --nocapture\n\
         (The values are MEASURED off `compile()`, never predicted — see `U1_EXPECTED_IMG`'s \
         doc. ⚠️ Paste ONLY the arrays for the half you generated: the four UNSPLIT baselines were \
         measured on the declarator BEFORE P2-5 changed it, and re-measuring them now would \
         certify the new behaviour instead of the old one.)",
        row.id,
        if row.split { "S" } else { "U" }
    );

    let f = declare_vb_frame(row);
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
    let late_barriers = buf_on(buf, f.vb_indirect_late).len();
    if row.split {
        assert_eq!(
            late_barriers, 1,
            "configuration {}: `vb_indirect_late` routed {late_barriers} barriers on an ARMED \
             SPLIT, expected exactly one — `vb_indirect_late_upload`'s TRANSFER write flushed to \
             `vb_raster_late`'s indirect FETCH. Zero means one of the two halves is undeclared, \
             which is a MISSING barrier that nothing else on this machine can see.",
            row.id
        );
    } else {
        assert_eq!(
            late_barriers, 0,
            "configuration {}: `vb_indirect_late` routed {late_barriers} barriers on an UNSPLIT \
             frame, where NO pass declares an access on it. The late record array is declared so \
             the sink slot has a ResId and the drift assert has something to measure; its accesses \
             exist only under `path_vb_occlusion_split()`.",
            row.id
        );
    }

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

/// **G4 row S1** — split ON, HZB off, dump off, SSAO off, `VB × Mesh`: the three new barriers at
/// the late scope's boundary, with nothing else in the frame to hide behind.
#[test]
fn s1_split_boundary_stream_is_pinned() {
    assert_row_is_pinned(S1, S1_EXPECTED_IMG, S1_EXPECTED_BUF, S1_EXPECTED_PASS);
}

/// **G4 row S2** — split ON, HZB armed, dump off, SSAO off, `VB × Mesh`: the depth round trip
/// across the moved poison+build block, and the pyramid's own three barriers unchanged in content
/// and MOVED in position (which is what `pass_barriers()` is in this pin for).
#[test]
fn s2_split_with_hzb_stream_is_pinned() {
    assert_row_is_pinned(S2, S2_EXPECTED_IMG, S2_EXPECTED_BUF, S2_EXPECTED_PASS);
}

/// **G4 row S3** — split ON, HZB armed, dump ON, SSAO off, `VB × Mesh`: gate G5's own path, where
/// `hzb_dump`'s `vb_depth` source is re-sourced by the block move.
#[test]
fn s3_split_with_hzb_dump_stream_is_pinned() {
    assert_row_is_pinned(S3, S3_EXPECTED_IMG, S3_EXPECTED_BUF, S3_EXPECTED_PASS);
}

/// **G4 row S4** — split ON, HZB armed, dump off, SSAO ON, `VB × Both`: the `vb_viewt` PRE-TAIL
/// slot and `sdf_forward_march`'s mesh arm, the other two re-sourced `vb_depth` readers.
#[test]
fn s4_split_with_ssao_and_sdf_leg_stream_is_pinned() {
    assert_row_is_pinned(S4, S4_EXPECTED_IMG, S4_EXPECTED_BUF, S4_EXPECTED_PASS);
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
    let f = declare_vb_frame(U1);
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

/// **U2's claim.** The pyramid build chain at `levels = 10`: pass 0's ONE merged barrier over
/// mips `[0, 6)`, pass 1's RAW over mip 5 ALONE, and pass 1's ONE merged barrier over `[6, 10)` —
/// the three barriers `compile_derives_the_hzb_build_chain_at_a_real_extent` measured in
/// isolation, here inside a whole VB frame. Plus the `vb_depth` hand-off `hzb_build_0` derives out
/// of the raster.
///
/// # RE-PINNED at VG R3 P3-0 — what changed, and why
///
/// Two of the assertions below read `(TOP_OF_PIPE, 0, UNDEFINED → GENERAL)` until P3-0: a FIRST
/// TOUCH. **There is no first touch on this resource any more**, so the two are re-pinned against
/// the machine's own new answer rather than deleted — piece 1's census precedent, where a rung
/// that armed a decision re-pinned the gate and wrote down the reason.
///
/// P3-0 flipped `hzb_pyramid`'s framegraph SEED from `ResSync::undefined()` to
/// `seeded_writer_at_layout(GENERAL, COMPUTE_SHADER, SHADER_WRITE)` (plan D2, argued in full at
/// the `add_image_mipped` call site in `present/graph_bridge.rs`). `sync::transition` then finds
/// `layout_change == false` and `flush_access != 0` on the frame's first access, so it takes the
/// **flush** branch instead of the first-touch one: `src` is the seed's pending cross-frame write,
/// and `old_layout` is the layout the image is already in. What makes the `GENERAL`-from-birth
/// claim TRUE is a real encoder — `HzbTargets::boot_clear_hzb_pyramid` clears every mip and lands
/// the image in `GENERAL` before any frame is recorded, once per targets generation — so
/// `oldLayout = GENERAL` DESCRIBES the image rather than asserting something about it. `dst_*`,
/// `new_layout`, every subresource span, the ORDER and the COUNT are unmoved, which is why only
/// the `src_*`/`old_layout` half of two assertions is touched.
///
/// # THE MERGE IS THE PROPERTY HERE, and the seed did not cost it
///
/// Pass 0's six mips all sit in ONE state at the access — before P3-0 the fresh `UNDEFINED` one,
/// now the seeded one — so `compile` derives one identical `Trans` per mip and folds them into a
/// single `MipRun`. That fold is what P1-5a shipped a whole step for, and a seeded resource must
/// still get it. It does, on both spans: `[0, 6)` and `[6, 10)` each arrive as ONE barrier.
///
/// # What each assertion catches NOW
///
/// * **A broken merge** — if the per-mip machine stopped folding, pass 0 would emit six
///   single-mip barriers and pass 1 four, no barrier would carry `mip_count == 6` or `4`, and the
///   census at the bottom would read 11 instead of 3. Both halves of that red independently.
/// * **A lost or reverted seed** — `(TOP_OF_PIPE, 0)` as the source means the frame's first
///   pyramid write carries NO dependency on the sibling in-flight frame's still-pipelined write of
///   the same NON-RINGED image, and `UNDEFINED` as the old layout licenses the driver to DISCARD
///   content the pyramid now carries across frames. D2 rules out both; this is what refuses them.
/// * **A moved span** — a widened or re-based `hzb_mips` on either build pass.
///
/// # Where this is WEAKER than what it replaced — stated, not left to be discovered
///
/// `(TOP_OF_PIPE, 0, UNDEFINED)` was a fingerprint of *"nothing in this frame has touched these
/// mips"*. `(COMPUTE, SHADER_WRITE, GENERAL → GENERAL)` is not: an earlier IN-FRAME COMPUTE write
/// of the same mips would derive byte-identical fields. Nothing on this row does that — the census
/// below is what says so, since a fourth pyramid writer would move the count — but taken alone
/// these two assertions no longer separate "sourced from the cross-frame seed" from "sourced from
/// an earlier in-frame COMPUTE write". The whole-stream pin's ORDER and per-pass attribution are
/// what still do.
///
/// The expected values are spelled as literal `VK_*` constants rather than read back from the
/// replica's seed ON PURPOSE. An assertion that took its `src` from the same constant the
/// declaration uses would follow a future seed edit silently — and a seed reverting to
/// `undefined()` is exactly the defect the bullet above exists to catch.
#[test]
fn u2_pins_the_pyramid_chain_and_the_depth_handoff() {
    let f = declare_vb_frame(U2);
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
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            VK_IMAGE_LAYOUT_GENERAL,
            hzb_mips(0, HZB_LEVELS_PER_PASS),
        ),
        "pass 0's six mips are all in the same state — the SEED's, since P3-0 — so they must MERGE \
         into ONE barrier over [0, 6). Six single-mip barriers here is the per-mip state machine \
         having stopped folding, which is the whole of what P1-5a shipped; the census below reads \
         11 instead of 3 in that case.\n\
         The source is the seed's pending cross-frame write, NOT `(TOP_OF_PIPE, 0)`: this frame's \
         first pyramid write must be ordered after the sibling in-flight frame's still-pipelined \
         one on this NON-RINGED image. And GENERAL → GENERAL is layout-PRESERVING — `UNDEFINED` \
         would license discarding content the pyramid now carries across frames (plan D2)"
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
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            VK_IMAGE_LAYOUT_GENERAL,
            hzb_mips(HZB_LEVELS_PER_PASS, HZB_LEVELS - HZB_LEVELS_PER_PASS),
        ),
        "mips 6..9 are still untouched when pass 1 reaches them, so they still carry the SEED's \
         state — all four alike — and must MERGE into ONE barrier over [6, 10). Before P3-0 this \
         run was a first touch out of UNDEFINED; the seed removed the first touch, not the run"
    );
    assert_eq!(
        img_on(img, f.hzb_pyramid).len(),
        3,
        "an undumped, unpoisoned pyramid derives EXACTLY the three build-chain barriers. This \
         count IS the merge stated as a census: unfolded, the same accesses would derive 6 + 1 + 4 \
         = 11. The seed moved no count — it re-sources barriers that were already being emitted"
    );
}

/// **U3's claim.** The poison clear's ONE whole-chain barrier over all ten mips, the WAW it turns
/// `hzb_build_0`'s write into, and the layout pair `hzb_dump` derives on `vb_depth`.
///
/// ⚠️ The dump's `src_stage`/`src_access` are deliberately NOT asserted here: they are precisely
/// the fields P2-5 changes (the plan's S3 row — from the "already SHADER_READ_ONLY,
/// execution-only" arm to a real RAW flush out of the late raster), so writing them by hand would
/// be a prediction wearing a gate's clothes. The measured whole-stream baseline carries them.
///
/// # RENAMED and RE-PINNED at VG R3 P3-0 — what changed, and why
///
/// This was `u3_pins_the_poison_first_touch_and_the_dump_layout_pair`. **The poison is not a
/// first touch any more, so the name could not stay** — a name is a claim like any other. P3-0
/// flipped `hzb_pyramid`'s framegraph SEED from `ResSync::undefined()` to
/// `seeded_writer_at_layout(GENERAL, COMPUTE_SHADER, SHADER_WRITE)` (plan D2), so
/// `sync::transition` finds `layout_change == false` and `flush_access != 0` at the clear and
/// takes the **flush** branch: the clear now flushes the SEED's pending cross-frame write
/// (`COMPUTE/SHADER_WRITE`) into its own `TRANSFER/TRANSFER_WRITE`, at `GENERAL → GENERAL`. The
/// `GENERAL`-from-birth premise is made true by a real encoder, not by assertion —
/// `HzbTargets::boot_clear_hzb_pyramid` clears every mip and leaves the image in `GENERAL` before
/// any frame is recorded, once per targets generation. `dst_*`, `new_layout`, the subresource and
/// the position in the stream are unmoved, so ONLY the first assertion's `src_*`/`old_layout` half
/// moves. Everything below it — `hzb_build_0`'s WAW and the dump's four pinned fields — keeps its
/// measured values; the WAW's MESSAGE is corrected, because it described the undumped frame as
/// deriving a first touch and that sentence is now false.
///
/// # THE MERGE, and it is the widest instance of it in this file
///
/// The clear declares `hzb_mips(0, 10)`, and all ten mips are in the same (seeded) state, so
/// `compile` must fold them into ONE barrier over the whole chain. Ten single-mip barriers here
/// is the per-mip machine having stopped merging — the P1-5a property — and the assertion reds,
/// because no barrier would then carry `mip_count == 10`.
///
/// # What the re-pinned assertion catches, and where it is weaker
///
/// It still refuses a narrowed or re-based clear span, a clear at the wrong stage/access, and an
/// unmerged chain. It newly refuses a LOST seed: `(TOP_OF_PIPE, 0, UNDEFINED)` would mean the
/// clear is unordered against the sibling in-flight frame's still-pipelined pyramid write on this
/// NON-RINGED image, with a licensed content discard on top.
///
/// ⚠️ **Weaker in one way, stated rather than left to be found:** `UNDEFINED` used to prove the
/// clear was the frame's first pyramid toucher. `GENERAL → GENERAL` does not — an in-frame COMPUTE
/// write ahead of the clear would derive the same `src`. What still orders the clear ahead of the
/// builds is the declarator's release-live `poison < build` assert and the whole-stream pin's
/// per-pass attribution, not this line.
#[test]
fn u3_pins_the_poison_whole_chain_waw_and_the_dump_layout_pair() {
    let f = declare_vb_frame(U3);
    let img = f.g.img_barriers();

    assert!(
        has_img(
            img,
            f.hzb_pyramid,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            VK_IMAGE_LAYOUT_GENERAL,
            hzb_mips(0, HZB_LEVELS),
        ),
        "the poison clear's ONE whole-chain barrier over ALL {HZB_LEVELS} mips is missing. Every \
         one of them is in the same (seeded) state, so they must MERGE into a single barrier — \
         {HZB_LEVELS} single-mip barriers here is the per-mip state machine having stopped \
         folding.\n\
         GENERAL is one of the two layouts `vkCmdClearColorImage` accepts and it is the layout the \
         pyramid holds FOR LIFE, so GENERAL → GENERAL is the only legal pair and no extra \
         transition may appear anywhere. Since P3-0 the src is the SEED's pending cross-frame \
         write: `(TOP_OF_PIPE, 0, UNDEFINED)` here would mean the clear is unordered against the \
         sibling in-flight frame's still-pipelined pyramid write, with a licensed content discard \
         on top (plan D2)"
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
        "on a poisoned frame `hzb_build_0` must derive its WAW flush out of the CLEAR \
         (TRANSFER_WRITE → SHADER_WRITE), not out of the seed. On an undumped frame the same \
         access sources `COMPUTE/SHADER_WRITE` instead — the seed's pending write, since P3-0 \
         removed the first touch that used to stand there \
         (`u2_pins_the_pyramid_chain_and_the_depth_handoff`). A `COMPUTE` src on a POISONED frame \
         means the clear was not modelled as this run's producer"
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
/// That absence is exactly what the D6 block move changes, and
/// [`s4_pins_the_re_sourced_later_depth_readers`] is its other half: with the block relocated
/// between the scopes, the last toucher of `vb_depth` becomes the LATE raster with a pending
/// write, and these readers change character to a real RAW flush plus a layout transition. A gate
/// that only asserted the barriers that exist on one of the two frames would go green on both.
#[test]
fn u4_pins_the_absent_barriers_on_the_later_depth_readers() {
    let f = declare_vb_frame(U4);
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

/// **S1's claim, and it is the load-bearing one of the whole piece.** The THREE barriers at the
/// late scope's boundary, asserted FIELD BY FIELD.
///
/// ⚠️ **The count of three is not the claim, and asserting it would certify the defect.** The
/// read-declared/write-undeclared variant of `vb_indirect_late` yields three as well, differing
/// only in `src_stage` / `src_access` — round 1 specified this gate as a count, which would have
/// gone RED on the correct implementation and GREEN on the defective one.
/// [`a_dropped_late_upload_write_keeps_the_count_and_moves_only_fields`] demonstrates exactly that
/// on this replica.
///
/// The two attachment WAWs are asserted **not to come from `UNDEFINED`**, because a first touch
/// there would license the driver to DISCARD what the early scope wrote — which is the equivalence
/// (`LOAD_OP_LOAD` yields what the early scope stored) the whole piece rests on.
#[test]
fn s1_pins_the_late_boundary_barriers_field_by_field() {
    let f = declare_vb_frame(S1);
    let img = f.g.img_barriers();
    let buf = f.g.buf_barriers();

    assert!(
        has_buf(
            buf,
            f.vb_indirect_late,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
            VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
        ),
        "the late record array's TRANSFER_WRITE → INDIRECT_COMMAND_READ edge is missing or \
         mis-sourced. `(TOP_OF_PIPE, 0)` here is the fingerprint of an UNDECLARED writer: the \
         `vkCmdUpdateBuffer` fill would be neither available nor visible to the indirect fetch, \
         and on frame 1 the scope that must draw NOTHING would fetch freshly allocated device \
         memory — arbitrary `instanceCount`, arbitrary `firstInstance`, `robustBufferAccess` OFF. \
         Nothing else in this repository can see that: it changes no pixel and emits no \
         validation message (measured — the plan's P2-0 table).\n\
         Got: {:#?}",
        buf_on(buf, f.vb_indirect_late)
    );

    let id = img_on(img, f.vb_id);
    let waw = id
        .iter()
        .find(|b| {
            b.src_stage == VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT
                && b.dst_stage == VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT
        })
        .unwrap_or_else(|| {
            panic!(
                "no COLOR_ATTACHMENT_OUTPUT → COLOR_ATTACHMENT_OUTPUT WAW on `vb_id`: the late \
                 scope writes the same attachment the early scope wrote, and the graph must order \
                 the two.\nGot: {id:#?}"
            )
        });
    assert_eq!(waw.src_access, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT);
    assert_eq!(waw.dst_access, VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT);
    assert_eq!(
        (waw.old_layout, waw.new_layout),
        (VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL),
        "the late scope's `vb_id` WAW must be layout-PRESERVING. `UNDEFINED → …` licenses \
         discarding the early scope's contents, and `LOAD_OP_LOAD` would then load garbage"
    );

    let depth = img_on(img, f.vb_depth);
    assert_eq!(
        depth.len(),
        2,
        "with HZB off, `vb_depth` carries exactly TWO barriers on a split frame: the early \
         raster's first touch and the late raster's WAW.\nGot: {depth:#?}"
    );
    assert_eq!(depth[0].old_layout, VK_IMAGE_LAYOUT_UNDEFINED, "the early scope's first touch");
    assert_eq!(
        (depth[1].src_stage, depth[1].dst_stage),
        (FRAG, FRAG),
        "the late scope's depth WAW is FRAG → FRAG"
    );
    assert_eq!(depth[1].src_access, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT);
    assert_eq!(depth[1].dst_access, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT);
    assert_eq!(
        (depth[1].old_layout, depth[1].new_layout),
        (VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL),
        "layout-PRESERVING, for the same reason as `vb_id`'s: a first touch here would discard the \
         depth the early scope tested against"
    );
}

/// **S2's claim.** The depth ROUND TRIP across the moved poison+build block, and that NEITHER leg
/// of it is a first touch.
///
/// `DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY_OPTIMAL` (into `hzb_build_0`, which now sits
/// between the scopes) and `SHADER_READ_ONLY_OPTIMAL → DEPTH_ATTACHMENT_OPTIMAL` (back into
/// `vb_raster_late`). Both content-preserving: an `UNDEFINED` on the return leg is round-1 blocker
/// 4's failure — it would discard the early scope's depth after the pyramid was built from it.
///
/// # RE-PINNED at VG R3 P3-0 — and ONLY the tail assertion moved
///
/// The round trip is `vb_depth`'s, and `vb_depth` is not the resource P3-0 seeded: all three of
/// its barriers, all three counts and both layout pairs are byte-unmoved, so the claim this test
/// is named for is untouched and is still asserted in full. What moved is the LAST assertion, the
/// one about the pyramid: it read `(TOP_OF_PIPE, 0, UNDEFINED → GENERAL)` — a FIRST TOUCH — and
/// **there is no first touch on `hzb_pyramid` any more.** P3-0 flipped its framegraph SEED from
/// `ResSync::undefined()` to `seeded_writer_at_layout(GENERAL, COMPUTE_SHADER, SHADER_WRITE)`
/// (plan D2), so `sync::transition` takes the **flush** branch on the first access — `src` is the
/// seed's pending cross-frame write, `old_layout` is the layout the image already holds. The
/// `GENERAL`-from-birth premise is made true by `HzbTargets::boot_clear_hzb_pyramid`, a real
/// encoder + submit + fence that clears every mip once per targets generation before any frame is
/// recorded. `dst_*`, `new_layout`, the subresource span and the position are unmoved.
///
/// **The claim the tail assertion makes is unchanged in substance and still load-bearing**: the
/// poison+build block moved WHOLE, so `hzb_build_0` is still the first pyramid writer, its six
/// mips are still all in one state, and they must still MERGE into ONE barrier over `[0, 6)`. Only
/// the state they are all in has a name now. It reds on: an unmerged chain (six single-mip
/// barriers, none carrying `mip_count == 6`), a block that moved without its clear, a re-based
/// span, and a lost seed — `(TOP_OF_PIPE, 0, UNDEFINED)` would leave this frame's first pyramid
/// write unordered against the sibling in-flight frame's, on a NON-RINGED image, with a licensed
/// content discard on top.
///
/// ⚠️ Weaker in the one way `u2`'s doc spells out: `UNDEFINED` proved "untouched this frame",
/// `GENERAL → GENERAL` does not.
#[test]
fn s2_pins_the_depth_round_trip_across_the_moved_block() {
    let f = declare_vb_frame(S2);
    let img = f.g.img_barriers();

    let depth = img_on(img, f.vb_depth);
    assert_eq!(
        depth.len(),
        3,
        "on a split HZB-armed frame `vb_depth` carries THREE barriers: the early raster's first \
         touch, the hand-off into `hzb_build_0`, and the RETURN into `vb_raster_late`.\n\
         Got: {depth:#?}"
    );
    assert_eq!(depth[0].old_layout, VK_IMAGE_LAYOUT_UNDEFINED);
    assert_eq!(depth[0].new_layout, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL);

    assert_eq!(
        (depth[1].old_layout, depth[1].new_layout),
        (VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL),
        "the hand-off into the pyramid build"
    );
    assert_eq!(depth[1].src_access, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT);
    assert_eq!(depth[1].dst_access, VK_ACCESS_SHADER_READ_BIT);

    assert_eq!(
        (depth[2].old_layout, depth[2].new_layout),
        (VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL),
        "the RETURN leg into `vb_raster_late`. ⚠️ `UNDEFINED` as the old layout here would discard \
         the very depth the pyramid was just built from — round-1 blocker 4's failure, and one \
         that no golden could see, because the late scope writes no fragment"
    );
    assert_eq!(depth[2].dst_stage, FRAG);
    assert_eq!(depth[2].dst_access, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT);

    // The pyramid's own three barriers are unchanged in CONTENT by the move; their POSITION is
    // what changed, and `pass_barriers()` in the whole-stream pin is what measures that. P3-0's
    // seed re-sourced them at BOTH slots alike, which is why U2's and S2's pyramid rows stay
    // field-for-field identical to each other and differ only in where they sit in the stream.
    assert!(
        has_img(
            img,
            f.hzb_pyramid,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_IMAGE_LAYOUT_GENERAL,
            VK_IMAGE_LAYOUT_GENERAL,
            hzb_mips(0, HZB_LEVELS_PER_PASS),
        ),
        "the pyramid's first WRITE must be unchanged by the slot move: the block moved WHOLE, so \
         `hzb_build_0` is still the first pyramid writer on an undumped frame, and its six mips — \
         all in the SEED's state — must still MERGE into ONE barrier over [0, 6).\n\
         Since P3-0 that write flushes the seed's pending cross-frame write rather than being a \
         first touch; `(TOP_OF_PIPE, 0, UNDEFINED)` here means the seed was lost, which leaves \
         this frame unordered against the sibling's pyramid write and licenses a content discard \
         (plan D2)"
    );
}

/// **S3's claim.** `hzb_dump`'s `vb_depth` source has CHANGED CHARACTER, and this is the asserted
/// -correct value rather than a regression.
///
/// On an unsplit armed frame the dump finds the depth already in `SHADER_READ_ONLY_OPTIMAL` where
/// `hzb_build_0` left it (`u3_pins_the_poison_whole_chain_waw_and_the_dump_layout_pair` pins that).
/// With the block moved between the scopes, the last toucher is `vb_raster_late` with a PENDING
/// WRITE, so the dump's transition becomes a real RAW flush out of the depth attachment. That is
/// strictly stronger than the execution-only edge it replaces — and it is why the declarator's own
/// comment ("on every armed frame that is `SHADER_READ_ONLY_OPTIMAL`, since `hzb_build_0` itself
/// reads it there") had to be corrected in P2-5 rather than left standing.
#[test]
fn s3_pins_the_re_sourced_hzb_dump_depth_read() {
    let f = declare_vb_frame(S3);
    let img = f.g.img_barriers();

    let depth = img_on(img, f.vb_depth);
    let dump = depth
        .iter()
        .find(|b| b.new_layout == VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL)
        .expect("invariant: the dump's depth copy derives a transition into TRANSFER_SRC_OPTIMAL");
    assert_eq!(
        (dump.src_stage, dump.src_access),
        (FRAG, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT),
        "on an ARMED-SPLIT frame the dump must flush the LATE raster's depth write. A \
         `(COMPUTE, 0)` source here would mean the graph still believes `hzb_build_0` was the last \
         toucher — i.e. the block did not move, or the late scope's depth access is undeclared"
    );
    assert_eq!(
        dump.old_layout, VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        "…and it finds the image where the late raster left it, not where the build did"
    );
    assert_eq!(dump.dst_stage, VK_PIPELINE_STAGE_TRANSFER_BIT);
    assert_eq!(dump.dst_access, VK_ACCESS_TRANSFER_READ_BIT);
    assert_eq!(dump.subresource, SubRange::DEPTH);
}

/// **S4's claim.** The same character change on the other two re-sourced readers — the `vb_viewt`
/// PRE-TAIL slot and `sdf_forward_march`'s mesh arm.
///
/// U4 pins that these two derive NO barrier on an unsplit frame. Here each derives one, out of the
/// late raster's pending depth write. The PAIR of tests is the claim: neither alone distinguishes
/// "the readers were re-sourced" from "the matrix row does not exercise them".
#[test]
fn s4_pins_the_re_sourced_later_depth_readers() {
    let f = declare_vb_frame(S4);
    let img = f.g.img_barriers();

    let depth = img_on(img, f.vb_depth);
    assert_eq!(
        depth.len(),
        4,
        "on a split `VB × Both` SSAO frame `vb_depth` carries FOUR barriers: the early raster's \
         first touch, the hand-off into `hzb_build_0`, the RETURN into `vb_raster_late`, and ONE \
         re-sourced read shared by the `vb_viewt` PRE-TAIL slot and `sdf_forward_march` (the \
         second reader needs none once the first has transitioned the image).\nGot: {depth:#?}"
    );
    assert_eq!(
        (depth[3].src_stage, depth[3].src_access),
        (FRAG, VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT),
        "the first post-late-scope reader must flush the LATE raster's write — on an UNSPLIT frame \
         it derived NOTHING at all (see `u4_pins_the_absent_barriers_on_the_later_depth_readers`), \
         which is exactly the modelling change D6 makes"
    );
    assert_eq!(
        (depth[3].old_layout, depth[3].new_layout),
        (VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL),
        "…with a preserving transition back into the layout a COMPUTE read wants"
    );
    assert_eq!(depth[3].dst_stage, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT);
    assert_eq!(depth[3].dst_access, VK_ACCESS_SHADER_READ_BIT);
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
/// # Which edit reds the eight pins
///
/// Deleting that one `buffer_access` — in this replica, or in `declare_vb_graph`'s `vb_batch_cull`
/// arm, which is what the replica mirrors. All eight whole-stream pins go RED on the FIELDS of one
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
///
/// # Why this body is now RELEASE-ONLY, and what runs in its place in debug
///
/// VG R3 P2-8 re-cut `graph.rs`'s unwritten-read guard from resource KIND to declared PROVENANCE,
/// so a bare `add_buffer` with a declared reader and no declared writer is a `debug_assert!` fire.
/// The corrupt frame below can therefore no longer be COMPILED in a dev-profile build — which is
/// the improvement, not an obstacle. The claim this body makes (a count gate is green on the
/// defect, a field gate is red) is a statement about the derived STREAM and stays pinned on the
/// release leg, where the guard is compiled out; the debug leg gets
/// `the_dropped_survivor_write_now_trips_the_framegraph_guard` below, which asserts the stronger
/// property that the declaration is rejected outright. CI runs both legs.
#[cfg(not(debug_assertions))]
#[test]
fn a_dropped_writer_keeps_every_count_and_moves_only_fields() {
    let faithful = declare_vb_frame(U1);
    let corrupt = declare_vb_frame(VbRow {
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

/// **The DEBUG-leg half of the control above** (VG R3 P2-8): the same corrupt frame, and the
/// claim is now that it cannot be compiled at all.
///
/// `vb_visible_instance` is declared with a bare `add_buffer` — the provenance declaration that
/// says "this graph writes it" — so a frame declaring `vb_raster`'s VERTEX read of it with the
/// cull's `SHADER_WRITE` dropped trips `compile`'s unwritten-read `debug_assert!`. Before P2-8 the
/// guard carried a `!is_image` term and this shape was waved through in every build, which is what
/// made the release-leg sibling's field assertions the ONLY thing in the tree that could see it.
///
/// The `expected` substring names the BUFFER arm, so an unrelated panic — a bounds check, the
/// layer-uniform invariant, the mip range check — cannot satisfy this test.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "reads UNWRITTEN transient buffer")]
fn the_dropped_survivor_write_now_trips_the_framegraph_guard() {
    let _ = declare_vb_frame(VbRow {
        id: "U1 + RED CONTROL (cull's survivor-list write undeclared)",
        red_control_drop_cull_survivor_write: true,
        ..U1
    });
}

/// **G4's R1 red control** (VG R3 piece 2 step P2-6) — the SAME defect class as the control above,
/// on the resource piece 2 actually adds, and the one the plan names.
///
/// Drop `vb_indirect_late_upload`'s declared `buffer_access(TRANSFER, TRANSFER_WRITE)` while
/// leaving `vb_raster_late`'s indirect fetch in place. The stream keeps **exactly three barriers at
/// the late boundary** and keeps every per-pass attribution; only `src_stage`/`src_access` on ONE
/// buffer barrier move, to `(TOP_OF_PIPE, 0)`.
///
/// That is the whole reason G4 asserts fields: as round 1 specified it — "exactly two/three new
/// barriers" — this gate would have gone **red on the correct implementation and green on the
/// defective one**. And when this control was written nothing else on this machine could see the
/// defect: it changes no pixel (`instanceCount = 0` draws nothing either way), emits no validation
/// message (measured: a genuine missing barrier produced the unchanged 19-message baseline and no
/// `SYNC-HAZARD`), and the framegraph's own unwritten-read backstop was image-only (`!is_image ||
/// ..`) and debug-only by construction.
///
/// # What VG R3 P2-8 changed, and why this body is now RELEASE-ONLY
///
/// P2-8 re-cut that backstop on declared PROVENANCE rather than kind, so a bare `add_buffer` with
/// a declared reader and no declared writer now fires a `debug_assert!` — the corrupt frame below
/// cannot be compiled in a dev-profile build. The STREAM claim (count gate green, field gate red)
/// is a statement about the derived barriers and stays pinned here, on the release leg where the
/// guard is compiled out. The debug leg gets
/// `the_dropped_late_upload_write_now_trips_the_framegraph_guard` below, which asserts the
/// stronger property. CI runs both legs.
#[cfg(not(debug_assertions))]
#[test]
fn a_dropped_late_upload_write_keeps_the_count_and_moves_only_fields() {
    let faithful = declare_vb_frame(S1);
    let corrupt = declare_vb_frame(VbRow {
        id: "S1 + RED CONTROL R1 (late upload's transfer write undeclared)",
        red_control_drop_late_upload_write: true,
        ..S1
    });

    let f_buf = faithful.g.buf_barriers();
    let c_buf = corrupt.g.buf_barriers();

    // (1) A COUNT gate is GREEN on the defect.
    assert_eq!(
        faithful.g.img_barriers(),
        corrupt.g.img_barriers(),
        "the control must isolate ONE buffer declaration; the image stream moving too would mean \
         it is testing something else"
    );
    assert_eq!(
        f_buf.len(),
        c_buf.len(),
        "the point of this control is that the defect PRESERVES the barrier count: a first-touch \
         read emits one barrier exactly as a RAW does"
    );
    assert_eq!(
        faithful.g.pass_barriers(),
        corrupt.g.pass_barriers(),
        "per-pass attribution is identical too — the defect moves no barrier to another pass, so a \
         gate on attribution is green on it as well"
    );

    // (2) A FIELD gate is RED on the defect.
    let faithful_late = buf_on(f_buf, faithful.vb_indirect_late);
    let corrupt_late = buf_on(c_buf, corrupt.vb_indirect_late);
    assert_eq!(faithful_late.len(), 1, "the late record array carries exactly one barrier when correct");
    assert_eq!(corrupt_late.len(), 1, "…and exactly one when its writer is undeclared — the SAME count");
    assert_eq!(
        (faithful_late[0].src_stage, faithful_late[0].src_access),
        (VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT),
        "correct: the indirect FETCH is a RAW that makes the `vkCmdUpdateBuffer` fill AVAILABLE"
    );
    assert_eq!(
        (corrupt_late[0].src_stage, corrupt_late[0].src_access),
        (VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, 0),
        "defective: with no declared writer the fetch takes the first-touch arm — an execution-only \
         edge that orders the stages while leaving the update neither available nor visible. On \
         frame 1, against freshly allocated DEVICE_LOCAL memory, the scope this whole piece claims \
         draws nothing DRAWS"
    );
    assert_eq!(
        (corrupt_late[0].dst_stage, corrupt_late[0].dst_access),
        (faithful_late[0].dst_stage, faithful_late[0].dst_access),
        "the CONSUMER side is unchanged by the defect — which is why only the source fields can \
         discriminate it"
    );
}

/// **The DEBUG-leg half of G4's R1 red control** (VG R3 P2-8), and the closing of the P2-7 hole
/// this whole step exists for.
///
/// P2-7 EXECUTED exactly this corruption against the PRODUCTION declarator — deleting
/// `vb_indirect_late_upload`'s declared `buffer_access(vb_indirect_late, TRANSFER,
/// TRANSFER_WRITE)` in `declare_vb_graph` while the recorder still filled the buffer and
/// `vb_raster_late` still fetched from it — and measured all four gates GREEN: the
/// `[vb_occ_split]` golden, the recorder probe, validation, and this very barrier-stream pin
/// (green because it is a hand-written REPLICA and cannot see the declarator changing shape).
/// P2-8 makes the framegraph itself reject it: `vb_indirect_late` is a bare `add_buffer`, so a
/// declared indirect fetch with no declared writer is a `debug_assert!` fire in every dev-profile
/// run — and every golden run is one (`scripts/golden.ps1` shells a bare `cargo test`).
///
/// This fixture asserts it on the REPLICA, which is what a `cargo test` can reach; the production
/// declarator takes the same guard through the same `compile`.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "reads UNWRITTEN transient buffer")]
fn the_dropped_late_upload_write_now_trips_the_framegraph_guard() {
    let _ = declare_vb_frame(VbRow {
        id: "S1 + RED CONTROL R1 (late upload's transfer write undeclared)",
        red_control_drop_late_upload_write: true,
        ..S1
    });
}

/// `compile()` is idempotent and a re-declared frame reproduces its own stream — so a divergence
/// reported by the pins above is a change in the DECLARATION, never a stale arena.
///
/// Both legs matter: recompiling one graph catches a `compile` that accumulates, and rebuilding a
/// second frame catches a `reset` that leaks (`res_shape`/`res_state_total` are a prefix sum, and
/// the pyramid is the mipped resource that would index past the end if one survived).
#[test]
fn every_row_is_reproducible_and_compile_is_idempotent() {
    for row in [U1, U2, U3, U4, S1, S2, S3, S4] {
        let mut f = declare_vb_frame(row);
        let first: Vec<ImgBarrier> = f.g.img_barriers().to_vec();
        let first_buf: Vec<BufBarrier> = f.g.buf_barriers().to_vec();
        f.g.compile();
        assert_eq!(f.g.img_barriers(), first.as_slice(), "configuration {}: compile not idempotent", row.id);
        assert_eq!(f.g.buf_barriers(), first_buf.as_slice(), "configuration {}: compile not idempotent", row.id);

        let again = declare_vb_frame(row);
        assert_eq!(again.g.img_barriers(), first.as_slice(), "configuration {}: rebuild diverged", row.id);
        assert_eq!(again.g.buf_barriers(), first_buf.as_slice(), "configuration {}: rebuild diverged", row.id);
    }
}

/// The eight rows must be eight DIFFERENT frames. A matrix whose rows compile to the same stream
/// pins one configuration eight times and reports it as coverage — the vacuity mode this campaign
/// has shipped five times.
///
/// Stated as a property of the compiled streams rather than of the config structs, because it is
/// the streams the pins compare. The `U*`/`S*` pairs are the load-bearing comparisons here: if
/// `S1` compiled to `U1`'s stream, the split would be declaring nothing and every split-row pin
/// below would be certifying the unsplit frame.
#[test]
fn the_eight_rows_are_eight_distinct_configurations() {
    /// One row's compiled streams, owned so all eight can be compared pairwise.
    struct RowStream {
        row: VbRow,
        img: Vec<ImgBarrier>,
        buf: Vec<BufBarrier>,
        passes: Vec<PassBarrierRange>,
        pass_count: usize,
    }

    let frames: Vec<RowStream> = [U1, U2, U3, U4, S1, S2, S3, S4]
        .into_iter()
        .map(|row| {
            let f = declare_vb_frame(row);
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
    // The split adds EXACTLY TWO passes — `vb_indirect_late_upload` and `vb_raster_late` — to each
    // row it arms, and MOVES the poison+build block rather than duplicating it. A `+3` or `+4`
    // here would mean the block was declared at both slots, which is how a "moved" block quietly
    // becomes a second one.
    for (u, s) in [(0usize, 4usize), (1, 5), (2, 6), (3, 7)] {
        assert_eq!(
            frames[s].pass_count,
            frames[u].pass_count + 2,
            "{} declares {} passes against {}'s {} — the split adds exactly two (the late upload \
             and the late raster) and MOVES the poison+build block, never duplicates it",
            frames[s].row.id,
            frames[s].pass_count,
            frames[u].row.id,
            frames[u].pass_count
        );
    }
}

/// `PassId` is strictly monotonic in declaration order and `compile()` does not reorder, so the
/// declare-order invariants `declare_vb_graph` asserts in production hold in this replica too —
/// on BOTH slots of the poison+build block.
///
/// The production asserts are `debug_assert!`s inside a `pub(crate)` fn no test can call, and they
/// run in the dev-profile builds every golden and gate run uses. These are the same properties,
/// asserted where a plain `cargo test` reaches them.
#[test]
fn declare_order_invariants_hold_in_the_replica() {
    for row in [U3, S3] {
        let f = declare_vb_frame(row);
        let index_of =
            |name: &str| -> Option<usize> { f.pass_names.iter().position(|n| *n == name) };

        let poison = index_of("hzb_poison").expect("invariant: the row arms the poison");
        let build0 = index_of("hzb_build_0").expect("invariant: the row arms the build chain");
        let dump = index_of("hzb_dump").expect("invariant: the row arms the dump");
        let raster = index_of("vb_raster").expect("invariant: every row declares the early raster");

        assert!(
            poison < build0,
            "{}: the poison must be declared BEFORE every build pass, or it erases what the build \
             wrote. This is the assert whose violation D6 makes possible — moving the builds \
             without their clear — and the one production carries as a `debug_assert!`",
            row.id
        );
        assert!(build0 < dump, "{}: the dump must observe a FINISHED pyramid", row.id);
        assert!(
            raster < build0,
            "{}: the pyramid reduces the depth THIS FRAME'S raster wrote; a build declared first \
             would read an unwritten transient",
            row.id
        );

        match (row.split, index_of("vb_raster_late"), index_of("vb_indirect_late_upload")) {
            (false, None, None) => {
                // The UNSPLIT slot: the block sits AFTER the `lit` producer, exactly where it has
                // been since piece 1. This is the order the four `U*` baselines were measured at.
                let lit_producer =
                    index_of("vb_resolve").expect("invariant: U3 is a fused frame");
                assert!(
                    lit_producer < poison,
                    "U3: on an unsplit frame the poison+build block is declared AFTER the lit \
                     producer. If this reverses, the block moved on a frame that did not arm the \
                     split — and the four measured `U*` baselines are then pinning a shape the \
                     shipping path no longer has"
                );
            }
            (true, Some(late), Some(late_upload)) => {
                // The ARMED-SPLIT slot: between the two scopes, so the pyramid reduces the EARLY
                // scope's depth and the late scope can (from piece 3) consult it.
                assert!(
                    raster < poison && build0 < late,
                    "S3: the armed order is `vb_raster → hzb_poison → hzb_build_* → \
                     vb_raster_late`. Got raster={raster}, poison={poison}, build0={build0}, \
                     late={late}"
                );
                assert!(
                    late_upload < late,
                    "S3: the late indirect upload must be declared BEFORE the late raster fetches \
                     from it — otherwise the derived dependency runs the wrong way. Got \
                     upload={late_upload}, late={late}"
                );
                let lit_producer =
                    index_of("vb_resolve").expect("invariant: S3 is a fused frame");
                assert!(
                    late < lit_producer,
                    "S3: the late scope must write `vb_id` BEFORE the `lit` producer reads it, or \
                     (from piece 3) the late geometry is never shaded. Got late={late}, \
                     lit={lit_producer}"
                );
            }
            (split, late, upload) => panic!(
                "{}: split={split} but vb_raster_late={late:?} and \
                 vb_indirect_late_upload={upload:?} — the two late passes are declared on EXACTLY \
                 the split predicate, together or not at all",
                row.id
            ),
        }
    }
}
