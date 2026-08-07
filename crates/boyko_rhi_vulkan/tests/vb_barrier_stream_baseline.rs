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
//! # ⚠️ THE THIRD RE-PIN: VG R3 piece 3 step P3-8, and the debt it pays — one DOCUMENTED gap and one that was NOT
//!
//! Step P3-8 closes two divergences between this replica and `declare_vb_graph`. **Both were opened
//! by earlier steps of the same piece; only ONE of them was written down**, and the difference is
//! worth recording because it decides how a reader should treat a green run of this file.
//!
//! 1. **`vb_raster_late`'s two new VERTEX reads** (all four `S*` rows). P3-6 armed the cull, so the
//!    late scope's VS really does read `vb_instance_ring` and `vb_late_visible`. The gap was
//!    recorded in block caps at its own declaration site the moment it was opened, with the shape of
//!    the missing edge named. Closing it adds **exactly one** buffer barrier per split row —
//!    `vb_late_visible`, `COMPUTE_SHADER(SHADER_WRITE) → VERTEX_SHADER(SHADER_READ)` — because the
//!    ring read is already visible at that stage and access and `sync::transition` returns `None`
//!    for it.
//! 2. **`hzb_dump_depth_early`** (row `S3` only). P3-7 added the pass to production **and recorded
//!    nothing here**, so between P3-7 and P3-8 the S3 baseline pinned a stream production had
//!    stopped producing — a replica short one whole PASS, which also shifts every later index. It
//!    adds one image barrier, moves TWO FIELDS of `vb_raster_late`'s depth WAW (`old_layout` and
//!    `src_stage`, with the COUNT unchanged — the class this file exists to catch), and makes the
//!    "the split adds three passes" arithmetic row-dependent for the first time.
//!
//! **Both deltas were DERIVED from `sync::transition` and the declaration order, not regenerated.**
//! [`dump_vb_split_barrier_streams`]'s own doc states why that mattered here: a baseline authored
//! after the change certifies the new behaviour, and re-running the generator would have made the
//! replica agree with production BY CONSTRUCTION — the "gate that certifies the defect" this
//! campaign has already recorded once. Each derivation is written beside the row it produced.
//!
//! ⚠️ **The lesson, stated where the next author will hit it:** a divergence recorded only in a plan
//! is a divergence that will be forgotten. Divergence 1 survived three steps because it was written
//! at its own site in block caps; divergence 2 was invisible for one step because it was not.
//!
//! # VG R3 piece 4 step P4-5 — the TWO PROBE-ON rows, and why they are a DELTA and not two more pins
//!
//! All eight PINNED rows hold `scene.vb_cull_readback` OFF, while the gates that decide whether the
//! occlusion split WORKS — `boyko_app/tests/vb_occ_mixed.rs`'s G-P3-B/C — run with
//! `BOYKO_VB_CULL_READBACK` ARMED. So until this step the barrier stream a *gate* executes was
//! modelled by nothing, and a defect there makes a gate LIE (a readback of stale bytes) rather than
//! merely making a diagnostic wrong. The asymmetry is stated from the other side at
//! `vb_occ_mixed.rs:40-43`.
//!
//! [`P1`] and [`P3`] are [`S1`] and [`S3`] with `probe: true`. They are deliberately **NOT** pinned
//! field by field — [`assert_row_is_pinned`] is never called on them, and its
//! `vb_indirect_late`-routes-TWO-barriers clause would red on them by construction, because the
//! probe appends a THIRD (see [`probe_delta_expectation`]). What is asserted instead is the
//! DIFFERENCE against their PROBE-OFF twins, which already carry the whole-stream pin:
//! [`probe_on_appends_the_readback_reads_and_resources_four_readers`].
//!
//! **The delta was DERIVED from the two declaration sites (`graph_bridge.rs`'s `vb_cull_readback`
//! and `vb_cull_readback_late` blocks) and from `framegraph/sync.rs`'s `transition`, never
//! regenerated** — [`dump_vb_split_barrier_streams`]'s doc states why that matters, and
//! [`probe_delta_expectation`] carries the derivation beside every row it produced.
//!
//! ⚠️ **A PLAN PREDICTION THAT THE TREE REFUTES, recorded here because a divergence recorded only
//! in a plan is forgotten.** `docs/VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md`'s P4-5 entry predicts
//! *"the re-sourcing of `vb_raster_late`'s indirect fetch from `COMPUTE(SHADER_WRITE) →
//! INDIRECT_COMMAND_READ` into `COMPUTE → TRANSFER` + `TRANSFER → INDIRECT_COMMAND_READ`"*. The
//! tree does NOT do that, and refuses to on purpose: `vb_cull_readback_late` is declared AFTER
//! `vb_raster_late`, and `graph_bridge.rs`'s own comment at that site says the alternative siting
//! *"would RE-SOURCE that fetch … Sited here it does not"*. The shipping
//! `vb_indirect_late_upload → vb_cull_late → vb_raster_late` chain is therefore field-identical with
//! and without the probe — which this step asserts rather than assumes. The re-sourcing the probe
//! DOES cause is on the EARLY pass and reaches four other readers; see [`probe_delta_expectation`].
//!
//! # The SECOND re-pin: VG R3 piece 3 step P3-3, and the ONE place the plan's own prediction is wrong
//!
//! P3-3 declares the late cull and the occlusion split's three buffers. Its plan entry says it moves
//! the **S-rows** — and it does — but it ALSO moves all four **U-rows**, which that entry does not
//! say, and the reason is a contradiction inside the plan's own D8: the section opens with *"New
//! accesses on `vb_batch_cull`, ALL gated on `occlusion_split` so an unsplit frame's declared set …
//! is bit-unchanged"* and then names the exception two paragraphs later — *"the uniform's pair is
//! the ONE exception to the gating: it is declared and written on every frame the cull runs"*. Both
//! cannot hold. The exception is the specific statement and the one D6 argues for (the module reads
//! `levels` out of that buffer, so a gated fill leaves it unwritten on a disarmed boot), so the
//! exception wins and the opening sentence is false for the U-rows.
//!
//! **The delta, DERIVED before it was measured, and it is total.** `vb_cull_uniform` is appended
//! LAST among the buffers, so no existing `ResId` moves and no pinned barrier's `res` field changes.
//! Inside `vb_batch_cull` its first access is a first-touch TRANSFER write (no barrier — nothing to
//! order against) and its second is a COMPUTE read, which derives exactly ONE
//! `TRANSFER(TRANSFER_WRITE) → COMPUTE(SHADER_READ)` buffer barrier — the same shape `vb_cull_count`'s
//! own fill already derives in that pass, and it joins that pass's existing `TRANSFER → COMPUTE`
//! group. So every row gains ONE `BufBarrier` element and every `PassBarrierRange` from
//! `vb_batch_cull` onward shifts by one. The S-rows additionally gain the split-gated early accesses
//! and the whole `vb_cull_late` pass.
//!
//! **What is NOT moved by it: any pixel.** `vb_cull_uniform` is read by no shader until step P3-4,
//! and a barrier cannot move a pixel it orders nothing against, so the 26 golden pins are
//! byte-identical. What moved is this file, which is the point of this file.
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
//! ## FOUR NAMED assertion SITES moved by P3-3's TWO-PRODUCER chain, and what they now state
//!
//! Four SITES, seven distinct failing tests and six on either leg — the first site is the helper all
//! four `S*` whole-stream pins call, and the last two are the debug and release legs of one control.
//! (A FIFTH site moved with P3-3's rows, from a DIFFERENT declaration; it has its own section
//! below, and was found only after this one was written.)
//!
//! ONE declaration moves all four: `vb_cull_late` writes `vb_indirect_late`. That buffer had exactly
//! ONE declared producer — the host's `vb_indirect_late_upload` — and now has TWO, so the chain the
//! stream derives across it is TWO barriers where it was one. This is plan D8's four-link chain; a
//! PROBE-OFF matrix (every PINNED row here) derives its first three links, and since VG R3 piece 4
//! step P4-5 the two unpinned probe rows derive the FOURTH — the post-late snapshot's TRANSFER read
//! — as an appended third barrier that leaves these two field-identical:
//!
//! ```text
//! vb_indirect_late_upload  TRANSFER(TRANSFER_WRITE) ──WAW──▶ vb_cull_late    COMPUTE(SHADER_WRITE)
//! vb_cull_late             COMPUTE(SHADER_WRITE)    ──RAW──▶ vb_raster_late  DRAW_INDIRECT(INDIRECT_COMMAND_READ)
//! ```
//!
//! * [`assert_row_is_pinned`]'s split branch said *"expected exactly one"* and now says TWO — and it
//!   no longer stops at the count, because after P3-3 a count alone cannot see the defect class the
//!   assertion exists for (spelled out two paragraphs down).
//! * [`s1_pins_the_late_boundary_barriers_field_by_field`] asserted the `TRANSFER_WRITE →
//!   INDIRECT_COMMAND_READ` edge. **That edge no longer exists** — the fetch is sourced from the
//!   cull. Its successor asserts BOTH links, and the first of them is now the ONLY place in the
//!   entire derived stream where the host upload is observable at all.
//! * `a_dropped_late_upload_write_keeps_the_count_and_moves_only_fields` is RENAMED to
//!   [`a_dropped_late_upload_write_deletes_the_upload_to_cull_waw`], because a name is a claim and
//!   that claim INVERTED. With a second producer, dropping the upload's write no longer re-sources
//!   the fetch: it DELETES the WAW and leaves the fetch barrier field-identical. The defect became
//!   count-VISIBLE and field-INVISIBLE — the exact opposite of the shape the R1 control was written
//!   to demonstrate — and the control now asserts that inversion instead of the old one.
//! * `the_dropped_late_upload_write_now_trips_the_framegraph_guard` **can no longer fire**: with a
//!   second declared producer, dropping the upload leaves the fetch preceded by a declared write, so
//!   `compile`'s provenance guard has nothing to catch. It is replaced by
//!   [`the_dropped_early_survivor_write_trips_the_guard_through_the_split_read`]; the disposition,
//!   including what the replacement does NOT cover, is argued at that test.
//!
//! **What the re-pinned assertions still catch — the property the originals were load-bearing
//! for.** Deleting any ONE of the three links leaves exactly ONE barrier: without the upload's write
//! the cull's store is a first touch (no barrier) and only the fetch's RAW survives; without the
//! cull's write the upload's fill is the first touch and only the fetch's RAW survives; without the
//! fetch only the WAW survives. So a missing half still REDS on the count, which is the original
//! claim carried over verbatim at the new number. And the one shape a count CANNOT see — the late
//! cull's access declared as a READ rather than a write — leaves the count at two and moves
//! `src_access` to `0` on the second barrier. That is why the re-pin asserts all four `(stage,
//! access)` pairs and not merely the number.
//!
//! ## The FIFTH site: P3-3's EARLY pyramid READ, which re-sources the pyramid's first WRITE
//!
//! A second P3-3 declaration moves a named-hazard assertion, and it is unrelated to the record
//! array: `vb_batch_cull` gains a READ of the whole pyramid, gated on `split &&
//! hzb_levels.is_some()`. It DECLARES plan D1's early-predicate input — the pyramid **as the
//! previous frame left it** — ahead of the leaf that consumes it (P3-4). So on a SPLIT row that
//! read, not `hzb_build_0`, is the frame's first pyramid access.
//!
//! The consequence is one field on one barrier, and plan D2's own hazard table names both halves of
//! it: the **cross-frame RAW** the P3-0 seed exists for is discharged at that READ, and the build's
//! write behind it becomes a **WAR** — `src_access: 0`, an execution-only dependency, since a read
//! has no memory to make available. `s2_pins_the_depth_round_trip_across_the_moved_block`'s tail
//! asserted that write as a seed FLUSH and is re-pinned at the test, with what it now catches and
//! the two ways it is weaker. The MERGE is unmoved and stated on a wider span than before: the
//! build's six mips still fold into ONE barrier over `[0, 6)` and the early read's ten fold into one
//! over `[0, 10)`.
//!
//! **The U-rows are NOT affected by this one** — the gate is on `split`, so an unsplit frame
//! declares no early pyramid read, `hzb_build_0`'s write stays the frame's first pyramid access,
//! and it still flushes the seed. Verified against the regenerated arrays rather than inferred from
//! the gate: the FIRST `hzb_pyramid` element of each unsplit row still reads
//! `(COMPUTE, SHADER_WRITE, GENERAL → GENERAL)` — `U2_EXPECTED_IMG[10]` and `U4_EXPECTED_IMG[6]`
//! (`hzb_build_0`'s write), `U3_EXPECTED_IMG[9]` (the poison clear) — which is the seed-flush shape
//! P3-0 pinned, so `u2`'s and `u3`'s assertions are untouched. (Their LATER pyramid elements are
//! sourced from the clear or from each other and always were; what P3-3 does move on those rows is
//! one BUFFER barrier, argued above.)
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
    /// `scene.vb_cull_readback` (the `BOYKO_VB_CULL_READBACK` boot knob) — arms the
    /// `vb_cull_readback` pass before `vb_raster` and, on a split row, `vb_cull_readback_late`
    /// after `vb_raster_late`.
    ///
    /// ⚠️ **A FIXED BASE until VG R3 piece 4 step P4-5, a COLUMN since.** All EIGHT pinned rows
    /// hold it `false` and their twelve measured expectation arrays are untouched by this step —
    /// an unarmed boot declares no pass at all, so a `true` reaching any pinned row would show up
    /// as a whole-stream divergence first. Only [`P1`] and [`P3`] set it, and they are asserted as
    /// a DELTA against [`S1`]/[`S3`] rather than pinned (see the module doc's P4-5 section).
    probe: bool,
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
    /// while leaving `vb_raster_late`'s indirect fetch in place, on the resource piece 2 adds.
    ///
    /// ⚠️ **Its defect class CHANGED at VG R3 P3-3.** While the upload was the buffer's only
    /// producer this was the read-declared/write-undeclared shape: same barrier count, two moved
    /// source fields. `vb_cull_late` is now a second producer, so the fetch stays correctly sourced
    /// from the cull and what the drop removes is the upload→cull WAW — one whole barrier, and the
    /// only trace the host fill leaves in the stream. Every pinned row holds this `false`; only
    /// [`a_dropped_late_upload_write_deletes_the_upload_to_cull_waw`] sets it.
    red_control_drop_late_upload_write: bool,
    /// **RED CONTROL ONLY (VG R3 piece 3 step P3-3).** Drops `vb_batch_cull`'s declared
    /// `vb_late_visible` WRITE while leaving `vb_cull_late`'s READ of it in place.
    ///
    /// Like `red_control_drop_cull_late_count_write` this fires `compile`'s P2-8 provenance guard
    /// rather than moving fields — but it is the only control that reaches the guard THROUGH the
    /// read half of a read-then-write pair, which is the property plan D8 pays a self-WAR edge for
    /// and which nothing else in this file demonstrates. Every pinned row holds this `false`; only
    /// [`the_dropped_early_survivor_write_trips_the_guard_through_the_split_read`] sets it.
    red_control_drop_cull_late_visible_write: bool,
    /// **RED CONTROL ONLY (VG R3 piece 3 step P3-3, plan G-P3-F's F4).** Drops `vb_batch_cull`'s
    /// declared `vb_late_count` WRITE while leaving `vb_cull_late`'s read of it in place.
    ///
    /// Unlike the two controls above this one does NOT move fields — it fires `compile`'s P2-8
    /// provenance guard, because `vb_late_count`'s first touch is that write and the read then
    /// becomes a first-touch read of a bare `add_buffer`. It is the ONE new buffer in piece 3 the
    /// guard can protect (`vb_indirect_late`'s first touch is the host upload's TRANSFER write, and
    /// a write is never tested), so this control is what demonstrates the coverage exists rather
    /// than asserting it. Every pinned row holds this `false`; only
    /// [`the_dropped_late_count_write_now_trips_the_framegraph_guard`] sets it.
    red_control_drop_cull_late_count_write: bool,
}

/// **U1** — split off, HZB off, dump off, SSAO off, `VB × Mesh`. The shipping baseline: nothing
/// about the split leaks into the unarmed path.
const U1: VbRow = VbRow {
    id: "U1 (split off, HZB off, dump off, SSAO off, VB×Mesh)",
    split: false,
    hzb_levels: None,
    hzb_dump: false,
    probe: false,
    ssao: false,
    sdf_leg: false,
    red_control_drop_cull_survivor_write: false,
    red_control_drop_late_upload_write: false,
    red_control_drop_cull_late_count_write: false,
    red_control_drop_cull_late_visible_write: false,
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
/// The new barriers at the late scope's boundary, and the row where they stand alone: `vb_id` WAW,
/// `vb_depth` WAW, and — since VG R3 piece 3 step P3-3 made `vb_cull_late` a SECOND producer of the
/// record array — `vb_indirect_late`'s TWO links, `TRANSFER_WRITE → SHADER_WRITE` (the host fill
/// flushed to the cull's store) and `SHADER_WRITE → INDIRECT_COMMAND_READ` (that store made
/// available to the fetch). ⚠️ The COUNT is not the whole evidence: an access declared as a READ
/// where the chain needs a WRITE keeps every count and moves only `src_access`. See
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
///
/// ⚠️ Since VG R3 piece 3 step P3-8 it is also the ONLY row that declares `hzb_dump_depth_early`
/// (plan D10) — the pass P3-7 added to production and did not add here. It is therefore the only row
/// on which the split adds FOUR passes rather than three, and the only one whose `vb_depth` carries
/// TWO transitions into `TRANSFER_SRC_OPTIMAL`.
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

/// **P1** (VG R3 piece 4 step P4-5) — [`S1`] with the READBACK PROBE armed. Not pinned; compared
/// against `S1` as a delta.
///
/// `S1` is the twin that isolates the probe's effect on the split's own three buffers with no HZB
/// in the frame: `vb_cull_late` still runs (its gate is the split alone — only its pyramid read
/// carries the `hzb_levels` conjunct), so every buffer edge the probe perturbs is present here.
const P1: VbRow = VbRow {
    id: "P1 (S1 + readback PROBE: split ON, HZB off, dump off, SSAO off, VB×Mesh)",
    probe: true,
    ..S1
};

/// **P3** (VG R3 piece 4 step P4-5) — [`S3`] with the READBACK PROBE armed. Not pinned; compared
/// against `S3` as a delta.
///
/// The second twin, and it is `S3` rather than `S2`/`S4` because `S3` is the row carrying the most
/// IMAGE traffic of the four (the poison, the ten-level build chain, `hzb_dump_depth_early` and the
/// frame-end dump), which is what makes the "the probe moves NO image barrier" half of the delta
/// worth executing twice. The BUFFER half is asserted identical to `P1`'s — the probe declares no
/// image access and `S1`/`S3` differ only in image accesses — and that row-independence is itself
/// asserted rather than assumed.
const P3: VbRow = VbRow {
    id: "P3 (S3 + readback PROBE: split ON, HZB armed, dump ON, SSAO off, VB×Mesh)",
    probe: true,
    ..S3
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
    /// The cull's atomic counter. Carried since VG R3 piece 4 step P4-5, whose delta names it:
    /// PROBE-OFF it routes exactly the intra-pass `TRANSFER → COMPUTE` reset edge, PROBE-ON it
    /// gains the snapshot's `COMPUTE(SHADER_WRITE) → TRANSFER(TRANSFER_READ)` flush.
    vb_cull_count: ResId,
    /// The cull's compacted BATCH list. Carried since VG R3 piece 4 step P4-5: it is the one
    /// resource in the whole matrix that routes ZERO barriers on all eight pinned rows and exactly
    /// one on a probe row, which makes it the delta's cleanest single-edge witness.
    vb_cull_visible: ResId,
    /// P2-3's late record array. On an UNSPLIT row it is declared and named by NO pass, and the
    /// row asserts it routes ZERO barriers — the structural form of "nothing about the split leaks
    /// into the unarmed path". On a SPLIT row it carries — since VG R3 piece 3 step P3-3 — the
    /// four-link chain `vb_indirect_late_upload` (TRANSFER write) → `vb_cull_late` (COMPUTE write,
    /// piece 2's obligation 1) → `vb_raster_late` (indirect fetch), plus the post-late snapshot's
    /// TRANSFER read on a PROBE-ON frame.
    vb_indirect_late: ResId,
    /// VG R3 piece 3 step P3-3: the occlusion split's candidate/survivor list. Unnamed by any pass
    /// on an unsplit row; on a split row it carries the early phase's write, `vb_cull_late`'s
    /// read-then-write pair (declared as TWO calls so the P2-8 provenance guard can test the read
    /// half) and therefore the self-WAR execution-only edge that split costs.
    ///
    /// ⚠️ Through VG R3 piece 3 this field was carried `#[allow(dead_code)]`, read by NO assertion:
    /// the two split buffers were covered by name only through
    /// `red_control_drop_cull_late_visible_write` / `red_control_drop_cull_late_count_write`, which
    /// fire inside the builder. Piece 4 step P4-5 reads it — the probe's snapshot re-sources both of
    /// this buffer's `vb_cull_late` edges and appends a third — so the allow is gone.
    vb_late_visible: ResId,
    /// VG R3 piece 3 step P3-3: per-batch `n_defer` plus the reserved frame slot. The ONE new
    /// buffer in this piece whose first touch is a COMPUTE WRITE, which is what makes the P2-8
    /// provenance guard live on it — see
    /// [`the_dropped_late_count_write_now_trips_the_framegraph_guard`].
    ///
    /// Read by an assertion since VG R3 piece 4 step P4-5: it is the buffer whose POST-late
    /// TRANSFER read derives NO barrier at all, the one row of the probe delta a count-only
    /// expectation would have got wrong.
    vb_late_count: ResId,
    /// VG R3 piece 3 step P3-3: the cull's non-push inputs. ⚠️ The ONE resource this step adds that
    /// an UNSPLIT row also carries a barrier for: its `TRANSFER_WRITE → SHADER_READ` pair is
    /// declared on EVERY frame the cull runs (plan D6), so it moves the U-rows as well as the
    /// S-rows.
    vb_cull_uniform: ResId,
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
/// | `scene.vb_cull_readback` | off on the eight PINNED rows | ⚠️ NO LONGER A FIXED BASE: VG R3 piece 4 step P4-5 made it the `VbRow::probe` COLUMN, because the gates that decide the split run with it ARMED. The eight pinned rows still hold it off, so their measured arrays are untouched |
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
    // P2-3's append. Declared, named by no pass on an unsplit row — the `hzb_pyramid` shape one
    // screen up.
    let vb_indirect_late = g.add_buffer("vb_indirect_late");
    // VG R3 piece 3 step P3-3's append: the occlusion split's trio, LAST and in the declarator's
    // order. All three are BARE `add_buffer` — after P2-8 that spelling is the provenance claim,
    // and each has an in-graph producer on every frame it is read.
    let vb_late_visible = g.add_buffer("vb_late_visible");
    let vb_late_count = g.add_buffer("vb_late_count");
    let vb_cull_uniform = g.add_buffer("vb_cull_uniform");

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
    // ⚠️ THIS DECLARATION IS WHAT ORDERS THE HOST FILL OF THE FOUR RECORD WORDS `vb_cull_late` DOES
    // NOT WRITE. Until VG R3 P3-3 it also sourced the indirect fetch, and dropping it cost the
    // stream no barrier and no count, only two fields. With the cull declared as a second producer
    // the fetch is sourced from the cull instead, so dropping this line now DELETES one whole
    // barrier — the upload→cull WAW, the only trace of the fill in the derived stream — while the
    // fetch's barrier stays field-identical. That is what
    // `red_control_drop_late_upload_write` reproduces.
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
    // VG R3 piece 3 step P3-3 (plan D6): the cull's UNIFORM — a `vkCmdUpdateBuffer` (TRANSFER) and
    // the dispatch's read (COMPUTE) inside THIS pass, the same intra-pass shape the counter fill
    // above uses.
    //
    // ⚠️ UNCONDITIONAL, the ONE access P3-3 adds that is not gated on the split — because the
    // module's `level >= levels ⇒ Keep` early-out reads `levels` out of this buffer, so a gated fill
    // would leave that read on unwritten allocation contents on a disarmed boot. It is therefore
    // also the one access that moves the U-rows: every row in this matrix gains exactly one
    // `TRANSFER(TRANSFER_WRITE) → COMPUTE(SHADER_READ)` buffer barrier inside `vb_batch_cull`.
    g.buffer_access(vb_cull_uniform, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT);
    g.buffer_access(vb_cull_uniform, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT);
    // VG R3 piece 3 step P3-3 (plan D8): the EARLY phase's three split-only accesses.
    //
    // The pyramid read is the EARLY predicate's input — the pyramid AS THE PREVIOUS FRAME LEFT IT,
    // since this frame's build has not run yet — which is the cross-frame RAW the P3-0 writer seed
    // exists to order. It carries the extra `hzb_levels.is_some()` conjunct the declarator carries,
    // and for the declarator's reason: `path_vb_occlusion_split()` does not imply
    // `scene.hzb.is_some()` until step P3-6, so row S1 (split ON, HZB off) is a real configuration
    // in which the pyramid image does not exist.
    if row.split {
        if let Some(levels) = row.hzb_levels {
            g.image_access(
                hzb_pyramid,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                hzb_mips(0, levels),
            );
        }
        // ⚠️ `red_control_drop_cull_late_visible_write` drops THIS line and nothing else, which
        // turns `vb_cull_late`'s READ of the survivor list below into a first-touch read. It fires
        // only because that read is declared as its OWN call — see
        // [`the_dropped_early_survivor_write_trips_the_guard_through_the_split_read`].
        if !row.red_control_drop_cull_late_visible_write {
            g.buffer_access(
                vb_late_visible,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
        }
        // ⚠️ `red_control_drop_cull_late_count_write` drops THIS line and nothing else. It is the
        // one buffer piece 3 adds whose first touch is a COMPUTE WRITE, so dropping it turns
        // `vb_cull_late`'s read below into a first-touch read of a bare `add_buffer` and the P2-8
        // provenance guard fires — a `debug_assert!`, not a moved field. See
        // [`the_dropped_late_count_write_now_trips_the_framegraph_guard`].
        if !row.red_control_drop_cull_late_count_write {
            g.buffer_access(
                vb_late_count,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
            );
        }
    }

    // `vb_cull_readback` — the PRE-late snapshot (rung R2c-tail + VG R3 piece 3 step P3-3),
    // MODELLED since VG R3 piece 4 step P4-5. Held OFF on all eight pinned rows; only `P1`/`P3`
    // arm it.
    //
    // Its production gate is `batch_cull_armed && scene.vb_cull_readback.is_some()`, and
    // `batch_cull_armed` is a fixed base of this matrix (see the table above), so `row.probe` alone
    // is that predicate here. Its POSITION — between `vb_batch_cull` and `vb_raster` — is the whole
    // of its cost: four of its six reads sit between a COMPUTE write and a later reader, so they
    // RE-SOURCE that reader's barrier from the write to this pass. The declarator says the same at
    // its own site ("BOTH are also read LATER in the frame by `vb_raster` … this TRANSFER read
    // RE-SOURCES their barriers").
    //
    // The last two are gated on the SPLIT as well as on the probe, exactly as production gates
    // them: this pass sits before `vb_cull_late`, so what it observes is the CANDIDATE list as the
    // early phase wrote it, the only point in the frame at which that multiset is observable.
    if row.probe {
        pass!("vb_cull_readback");
        g.buffer_access(vb_cull_count, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_READ_BIT);
        g.buffer_access(vb_cull_visible, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_READ_BIT);
        g.buffer_access(vb_indirect, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_READ_BIT);
        g.buffer_access(
            vb_visible_instance,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_ACCESS_TRANSFER_READ_BIT,
        );
        if row.split {
            g.buffer_access(
                vb_late_visible,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_READ_BIT,
            );
            g.buffer_access(
                vb_late_count,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_READ_BIT,
            );
        }
    }

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

        // `hzb_dump_depth_early` (VG R3 piece 3 step P3-7, plan D10) — the EARLY-DEPTH dump copy,
        // added to this replica at step P3-8.
        //
        // ⚠️ **P3-7 ADDED IT TO PRODUCTION AND NOT HERE, AND RECORDED NOTHING.** The `vb_raster_late`
        // divergence below was written down in block caps at its own site when it was opened; this
        // one was not, so between P3-7 and P3-8 the S3 baseline pinned a stream that production had
        // stopped producing — a replica silently one PASS short, which is a strictly worse state
        // than the documented one-barrier gap because a missing PASS also shifts every later index.
        //
        // ONE access, the SAME shape the end-of-frame `hzb_dump` declares on the same image, and the
        // POSITION is the claim: after the last `hzb_build_*` so what it copies is exactly what they
        // reduced, and before `vb_raster_late` so nothing has drawn into the depth again. Both
        // neighbours are asserted by `declare_order_invariants_hold_in_the_replica`.
        //
        // The gate is `occlusion_split && hzb_dump_armed` and nothing else — the declarator's, with
        // the `hzb_levels` conjunct that `row.hzb_dump` carries here because the `hzb_dump` pass
        // itself takes it.
        if row.hzb_dump && row.hzb_levels.is_some() {
            pass!("hzb_dump_depth_early");
            g.image_access(
                vb_depth,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_READ_BIT,
                VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                SubRange::DEPTH,
            );
        }

        // `vb_cull_late` (VG R3 piece 3 step P3-3, plan D4/D5/D8) — the SECOND dispatch of the cull
        // module, declared after the last `hzb_build_*` and before `vb_raster_late`: it reads the
        // pyramid THIS frame's build wrote and writes the `instanceCount` the late scope fetches.
        //
        // The access list is deliberately ASYMMETRIC with `vb_batch_cull`'s, and the rule is the one
        // the declarator states: a not-taken LOAD may still issue (DXC may lower a `? :` to an eager
        // load plus an `OpSelect`), but a compiler may not introduce a STORE the source does not
        // perform. `pc.phase` is a push constant, uniform across the dispatch. So every LOAD either
        // phase can issue is declared on BOTH passes, while `vb_indirect_late`'s store is declared
        // HERE only and `vb_indirect`/`vb_cull_visible`/`vb_cull_count`/`vb_visible_instance`'s
        // stores are declared on `vb_batch_cull` only.
        //
        // ⚠️ `vb_late_visible` IS TWO CALLS — read, then write — never one combined
        // `SHADER_READ|SHADER_WRITE`. A combined access is `is_write`, so `compile`'s provenance
        // guard would never test the read half. The cost of the split is a SECOND, execution-only
        // self-WAR edge on this pass, which is a new PINNED row rather than a hidden one.
        pass!("vb_cull_late");
        g.buffer_access(
            vb_batch_desc,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        );
        g.buffer_access(
            vb_instance_ring,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        );
        g.buffer_access(
            vb_cull_uniform,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        );
        if let Some(levels) = row.hzb_levels {
            g.image_access(
                hzb_pyramid,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_READ_BIT,
                VK_IMAGE_LAYOUT_GENERAL,
                hzb_mips(0, levels),
            );
        }
        g.buffer_access(
            vb_late_count,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        );
        g.buffer_access(
            vb_late_visible,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        );
        g.buffer_access(
            vb_late_visible,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
        );
        // Piece 2's obligation 1, discharged: `vb_indirect_late`'s declared writer moves from
        // `(TRANSFER, TRANSFER_WRITE)` to `(COMPUTE_SHADER, SHADER_WRITE)` — and the writer that
        // changes is THIS pass, never `vb_batch_cull`, which does not touch the record array at all.
        g.buffer_access(
            vb_indirect_late,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
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
        // ⚠️⚠️ **DIVERGENCE OPENED AT VG R3 PIECE 3 STEP P3-6, CLOSED HERE AT P3-8.** Through piece 2
        // and steps P3-1..P3-5 this pass declared exactly three accesses, and the VS's
        // `vb_instance_ring` / `vb_visible_instance` reads were deliberately absent because every
        // late record carried `instanceCount = 0`: zero vertex invocations, neither read performed,
        // and declaring them would have declared an access the recorder does not make. Step P3-6
        // ARMED the cull, so `declare_vb_graph` declares TWO MORE accesses here —
        // `vb_instance_ring` at VERTEX and **`vb_late_visible`** at VERTEX (not
        // `vb_visible_instance`: the late scope binds `vb_set0_late`, which is `vb_set0` with @11
        // changed, leaving `vb_raster.vs.hlsl` byte-unchanged).
        //
        // ⚠️ **The two rows below were DERIVED from the declarations, never regenerated by re-running
        // `dump_vb_split_barrier_streams`.** That generator's own doc states the authoring-order
        // discipline: *a baseline authored after the change certifies the new behaviour*. Regenerating
        // here would have made the replica agree with production BY CONSTRUCTION — the "gate that
        // certifies the defect" this campaign has already recorded once. What the derivation says,
        // from `sync::transition`:
        //
        //  * `vb_instance_ring` — `vb_raster` already read it at VERTEX/SHADER_READ and nothing has
        //    written it since, so `layout_change` is false, `flush_access` is 0, and both
        //    `stage & !visible_stages` and `access & !visible_access` are 0 ⇒ `need == false` ⇒
        //    **NO BARRIER**. The declaration still matters (it is what makes that visibility a
        //    declared fact rather than an accident), but it routes nothing.
        //  * `vb_late_visible` — `vb_cull_late`'s read-then-write pair ends in a WRITE, so a pending
        //    COMPUTE/SHADER_WRITE flush is outstanding ⇒ the RAW arm ⇒ **ONE barrier**,
        //    `COMPUTE_SHADER(SHADER_WRITE) → VERTEX_SHADER(SHADER_READ)`.
        pass!("vb_raster_late");
        g.buffer_access(
            vb_indirect_late,
            VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
            VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
        );
        g.buffer_access(
            vb_instance_ring,
            VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        );
        g.buffer_access(
            vb_late_visible,
            VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
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

        // `vb_cull_readback_late` (VG R3 piece 3 step P3-3, plan D8) — the POST-late snapshot,
        // MODELLED since VG R3 piece 4 step P4-5. Gated on `occlusion_split && probe`, which is
        // why it lives inside this block while its pre-late sibling above carries its own gate.
        //
        // ⚠️ THE POSITION IS THE DECISION, and it is the reason the shipping chain is field-
        // identical with and without the probe. Sited BETWEEN `vb_cull_late`'s COMPUTE write and
        // `vb_raster_late`'s DRAW_INDIRECT fetch it would re-source that fetch, exactly as the
        // pre-late pass re-sources four other readers; sited HERE the late raster only READS these
        // three buffers, so the probe appends edges and moves none of the shipping ones. The
        // declarator argues it in those words at its own site; this replica is where the claim is
        // EXECUTED (`probe_on_appends_the_readback_reads_and_resources_four_readers`).
        if row.probe {
            pass!("vb_cull_readback_late");
            g.buffer_access(
                vb_late_visible,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_READ_BIT,
            );
            g.buffer_access(
                vb_late_count,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_READ_BIT,
            );
            g.buffer_access(
                vb_indirect_late,
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_READ_BIT,
            );
        }
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
        vb_cull_count,
        vb_cull_visible,
        vb_indirect_late,
        vb_late_visible,
        vb_late_count,
        vb_cull_uniform,
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
/// failure message would replace the diagnosis with its own noise. `vb_cull_uniform` is the LAST
/// resource [`declare_vb_frame`] declares (it was `vb_indirect_late` until VG R3 piece 3 step
/// P3-3 appended the split's trio), so its index is the bound.
fn res_label(f: &VbFrame, res: ResId) -> String {
    if res.index() <= f.vb_cull_uniform.index() {
        format!("{:?}", f.g.res_name(res))
    } else {
        format!("<ResId {} is outside this frame's {} resources>", res.0, f.vb_cull_uniform.index() + 1)
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
///
/// ⚠️ It has been run TWICE since the four `U*` rows were authored, and both times the delta was
/// DERIVED first and written into this module's doc before the generator was invoked: VG R3 P3-0's
/// pyramid seed, and VG R3 P3-3's unconditional `vb_cull_uniform` pair. Re-running it for any other
/// reason re-measures the baselines the split is compared against, which is the one thing the
/// two-generator split exists to prevent — so if the prediction is not already written down, do not
/// run this.
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
        res: ResId(31), // [5] "vb_cull_uniform"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(23), // [6] "vb_indirect"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    BufBarrier {
        res: ResId(16), // [7] "vb_instance_ring"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(27), // [8] "vb_visible_instance"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(15), // [9] "light_table"
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
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 5 }, // [5] "vb_batch_cull"
    PassBarrierRange { img_begin: 3, img_count: 2, buf_begin: 6, buf_count: 3 }, // [6] "vb_raster"
    PassBarrierRange { img_begin: 5, img_count: 4, buf_begin: 9, buf_count: 1 }, // [7] "vb_resolve"
    PassBarrierRange { img_begin: 9, img_count: 1, buf_begin: 10, buf_count: 0 }, // [8] "present_sample"
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
        res: ResId(31), // [5] "vb_cull_uniform"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(23), // [6] "vb_indirect"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    BufBarrier {
        res: ResId(16), // [7] "vb_instance_ring"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(27), // [8] "vb_visible_instance"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(15), // [9] "light_table"
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
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 5 }, // [5] "vb_batch_cull"
    PassBarrierRange { img_begin: 3, img_count: 2, buf_begin: 6, buf_count: 3 }, // [6] "vb_raster"
    PassBarrierRange { img_begin: 5, img_count: 4, buf_begin: 9, buf_count: 1 }, // [7] "vb_resolve"
    PassBarrierRange { img_begin: 9, img_count: 2, buf_begin: 10, buf_count: 0 }, // [8] "hzb_build_0"
    PassBarrierRange { img_begin: 11, img_count: 2, buf_begin: 10, buf_count: 0 }, // [9] "hzb_build_1"
    PassBarrierRange { img_begin: 13, img_count: 1, buf_begin: 10, buf_count: 0 }, // [10] "present_sample"
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
        res: ResId(31), // [5] "vb_cull_uniform"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(23), // [6] "vb_indirect"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    BufBarrier {
        res: ResId(16), // [7] "vb_instance_ring"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(27), // [8] "vb_visible_instance"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(15), // [9] "light_table"
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
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 5 }, // [5] "vb_batch_cull"
    PassBarrierRange { img_begin: 3, img_count: 2, buf_begin: 6, buf_count: 3 }, // [6] "vb_raster"
    PassBarrierRange { img_begin: 5, img_count: 4, buf_begin: 9, buf_count: 1 }, // [7] "vb_resolve"
    PassBarrierRange { img_begin: 9, img_count: 1, buf_begin: 10, buf_count: 0 }, // [8] "hzb_poison"
    PassBarrierRange { img_begin: 10, img_count: 2, buf_begin: 10, buf_count: 0 }, // [9] "hzb_build_0"
    PassBarrierRange { img_begin: 12, img_count: 2, buf_begin: 10, buf_count: 0 }, // [10] "hzb_build_1"
    PassBarrierRange { img_begin: 14, img_count: 1, buf_begin: 10, buf_count: 0 }, // [11] "present_sample"
    PassBarrierRange { img_begin: 15, img_count: 4, buf_begin: 10, buf_count: 0 }, // [12] "hzb_dump"
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
        res: ResId(31), // [5] "vb_cull_uniform"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(23), // [6] "vb_indirect"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    BufBarrier {
        res: ResId(16), // [7] "vb_instance_ring"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(27), // [8] "vb_visible_instance"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(15), // [9] "light_table"
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
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 5 }, // [5] "vb_batch_cull"
    PassBarrierRange { img_begin: 3, img_count: 2, buf_begin: 6, buf_count: 3 }, // [6] "vb_raster"
    PassBarrierRange { img_begin: 5, img_count: 2, buf_begin: 9, buf_count: 0 }, // [7] "hzb_build_0"
    PassBarrierRange { img_begin: 7, img_count: 2, buf_begin: 9, buf_count: 0 }, // [8] "hzb_build_1"
    PassBarrierRange { img_begin: 9, img_count: 1, buf_begin: 9, buf_count: 0 }, // [9] "vb_viewt"
    PassBarrierRange { img_begin: 10, img_count: 2, buf_begin: 9, buf_count: 0 }, // [10] "vb_geo"
    PassBarrierRange { img_begin: 12, img_count: 3, buf_begin: 9, buf_count: 0 }, // [11] "ssao"
    PassBarrierRange { img_begin: 15, img_count: 4, buf_begin: 9, buf_count: 1 }, // [12] "vb_shade_split"
    PassBarrierRange { img_begin: 19, img_count: 1, buf_begin: 10, buf_count: 0 }, // [13] "sdf_forward_march"
    PassBarrierRange { img_begin: 20, img_count: 1, buf_begin: 10, buf_count: 0 }, // [14] "present_sample"
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
        res: ResId(31), // [5] "vb_cull_uniform"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(23), // [6] "vb_indirect"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    BufBarrier {
        res: ResId(16), // [7] "vb_instance_ring"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(27), // [8] "vb_visible_instance"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(30), // [9] "vb_late_count"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(29), // [10] "vb_late_visible"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(29), // [11] "vb_late_visible"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(28), // [12] "vb_indirect_late"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(28), // [13] "vb_indirect_late"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    // VG R3 piece 3 step P3-8: `vb_raster_late`'s `vb_late_visible` VERTEX read. DERIVED from the
    // declarations, not regenerated: `vb_cull_late` leaves a pending COMPUTE/SHADER_WRITE flush on
    // this buffer (its read-then-write pair ends in the write), so the next access takes
    // `sync::transition`'s RAW arm — `src = (flush_stages, flush_access)` — and the dst is the
    // VERTEX/SHADER_READ this pass declares. The sibling `vb_instance_ring` read declared beside it
    // adds NO barrier: `vb_raster` already read that buffer at VERTEX/SHADER_READ and nothing wrote
    // it since, so `stage & !visible_stages` and `access & !visible_access` are both zero and
    // `need` is false.
    BufBarrier {
        res: ResId(29), // [14] "vb_late_visible"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(15), // [15] "light_table"
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
    PassBarrierRange { img_begin: 3, img_count: 0, buf_begin: 1, buf_count: 5 }, // [6] "vb_batch_cull"
    PassBarrierRange { img_begin: 3, img_count: 2, buf_begin: 6, buf_count: 3 }, // [7] "vb_raster"
    PassBarrierRange { img_begin: 5, img_count: 0, buf_begin: 9, buf_count: 4 }, // [8] "vb_cull_late"
    PassBarrierRange { img_begin: 5, img_count: 2, buf_begin: 13, buf_count: 2 }, // [9] "vb_raster_late"
    PassBarrierRange { img_begin: 7, img_count: 4, buf_begin: 15, buf_count: 1 }, // [10] "vb_resolve"
    PassBarrierRange { img_begin: 11, img_count: 1, buf_begin: 16, buf_count: 0 }, // [11] "present_sample"
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
        res: ResId(14), // [3] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 10, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [4] "vb_id"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [5] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
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
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
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
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 6, mip_count: 4, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [10] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 5, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [11] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 6, mip_count: 4, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [12] "vb_id"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [13] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [14] "vb_id"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
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
        res: ResId(0), // [18] "lit"
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
        res: ResId(31), // [5] "vb_cull_uniform"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(23), // [6] "vb_indirect"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    BufBarrier {
        res: ResId(16), // [7] "vb_instance_ring"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(27), // [8] "vb_visible_instance"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(30), // [9] "vb_late_count"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(29), // [10] "vb_late_visible"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(29), // [11] "vb_late_visible"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(28), // [12] "vb_indirect_late"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(28), // [13] "vb_indirect_late"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    // VG R3 piece 3 step P3-8: `vb_raster_late`'s `vb_late_visible` VERTEX read. DERIVED from the
    // declarations, not regenerated: `vb_cull_late` leaves a pending COMPUTE/SHADER_WRITE flush on
    // this buffer (its read-then-write pair ends in the write), so the next access takes
    // `sync::transition`'s RAW arm — `src = (flush_stages, flush_access)` — and the dst is the
    // VERTEX/SHADER_READ this pass declares. The sibling `vb_instance_ring` read declared beside it
    // adds NO barrier: `vb_raster` already read that buffer at VERTEX/SHADER_READ and nothing wrote
    // it since, so `stage & !visible_stages` and `access & !visible_access` are both zero and
    // `need` is false.
    BufBarrier {
        res: ResId(29), // [14] "vb_late_visible"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(15), // [15] "light_table"
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
    PassBarrierRange { img_begin: 3, img_count: 1, buf_begin: 1, buf_count: 5 }, // [6] "vb_batch_cull"
    PassBarrierRange { img_begin: 4, img_count: 2, buf_begin: 6, buf_count: 3 }, // [7] "vb_raster"
    PassBarrierRange { img_begin: 6, img_count: 2, buf_begin: 9, buf_count: 0 }, // [8] "hzb_build_0"
    PassBarrierRange { img_begin: 8, img_count: 2, buf_begin: 9, buf_count: 0 }, // [9] "hzb_build_1"
    PassBarrierRange { img_begin: 10, img_count: 2, buf_begin: 9, buf_count: 4 }, // [10] "vb_cull_late"
    PassBarrierRange { img_begin: 12, img_count: 2, buf_begin: 13, buf_count: 2 }, // [11] "vb_raster_late"
    PassBarrierRange { img_begin: 14, img_count: 4, buf_begin: 15, buf_count: 1 }, // [12] "vb_resolve"
    PassBarrierRange { img_begin: 18, img_count: 1, buf_begin: 16, buf_count: 0 }, // [13] "present_sample"
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
        res: ResId(14), // [3] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 10, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [4] "vb_id"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [5] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [6] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 10, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [7] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [8] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 6, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [9] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 5, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [10] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 6, mip_count: 4, base_layer: 0, layer_count: 1 },
    },
    // VG R3 piece 3 step P3-8: `hzb_dump_depth_early`'s ONE access. DERIVED, not regenerated.
    // `hzb_build_0` read `vb_depth` at COMPUTE/SHADER_READ and left `SHADER_READ_ONLY_OPTIMAL` with
    // NO pending flush (a read clears it) and `visible = {COMPUTE, SHADER_READ}`. This access
    // changes the layout, so `need` is true; `flush_access == 0` and `visible_stages != 0` select
    // `sync::transition`'s WAR / visibility-extend arm, whose src is `(visible_stages, 0)` — an
    // EXECUTION dependency on the prior readers, with no memory to make available because the src
    // is a read.
    ImgBarrier {
        res: ResId(2), // [11] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_TRANSFER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [12] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 5, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [13] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 6, mip_count: 4, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [14] "vb_id"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    // ⚠️ VG R3 piece 3 step P3-8 MOVES TWO FIELDS OF THIS BARRIER, and the move is the whole point
    // of pinning fields rather than counts. `hzb_dump_depth_early` above now sits between
    // `hzb_build_0`'s read and this write, so the depth's state when the late scope claims it is
    // `TRANSFER_SRC_OPTIMAL` with `visible_stages = {COMPUTE, TRANSFER}` — the old_layout was
    // `SHADER_READ_ONLY_OPTIMAL` and the src_stage was `COMPUTE_SHADER` alone. The COUNT is
    // unchanged, which is exactly the class `a_dropped_writer_keeps_every_count_and_moves_only
    // _fields` exists for.
    ImgBarrier {
        res: ResId(2), // [15] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT | VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [16] "vb_id"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [17] "lit"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(3), // [18] "cascade"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 4 },
    },
    ImgBarrier {
        res: ResId(4), // [19] "atlas"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 16 },
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
    // UNMOVED by step P3-8, and that is itself derived rather than hoped for: `vb_raster_late`'s
    // depth WRITE resets the visibility accumulator, so the frame-end dump's source is the late
    // scope's own flush whether or not an early copy happened earlier in the frame.
    ImgBarrier {
        res: ResId(2), // [21] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_TRANSFER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [22] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_TRANSFER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 10, base_layer: 0, layer_count: 1 },
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
        res: ResId(31), // [5] "vb_cull_uniform"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(23), // [6] "vb_indirect"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    BufBarrier {
        res: ResId(16), // [7] "vb_instance_ring"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(27), // [8] "vb_visible_instance"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(30), // [9] "vb_late_count"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(29), // [10] "vb_late_visible"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(29), // [11] "vb_late_visible"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(28), // [12] "vb_indirect_late"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(28), // [13] "vb_indirect_late"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    // VG R3 piece 3 step P3-8: `vb_raster_late`'s `vb_late_visible` VERTEX read. DERIVED from the
    // declarations, not regenerated: `vb_cull_late` leaves a pending COMPUTE/SHADER_WRITE flush on
    // this buffer (its read-then-write pair ends in the write), so the next access takes
    // `sync::transition`'s RAW arm — `src = (flush_stages, flush_access)` — and the dst is the
    // VERTEX/SHADER_READ this pass declares. The sibling `vb_instance_ring` read declared beside it
    // adds NO barrier: `vb_raster` already read that buffer at VERTEX/SHADER_READ and nothing wrote
    // it since, so `stage & !visible_stages` and `access & !visible_access` are both zero and
    // `need` is false.
    BufBarrier {
        res: ResId(29), // [14] "vb_late_visible"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(15), // [15] "light_table"
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
    PassBarrierRange { img_begin: 3, img_count: 1, buf_begin: 1, buf_count: 5 }, // [6] "vb_batch_cull"
    PassBarrierRange { img_begin: 4, img_count: 2, buf_begin: 6, buf_count: 3 }, // [7] "vb_raster"
    PassBarrierRange { img_begin: 6, img_count: 1, buf_begin: 9, buf_count: 0 }, // [8] "hzb_poison"
    PassBarrierRange { img_begin: 7, img_count: 2, buf_begin: 9, buf_count: 0 }, // [9] "hzb_build_0"
    PassBarrierRange { img_begin: 9, img_count: 2, buf_begin: 9, buf_count: 0 }, // [10] "hzb_build_1"
    // VG R3 piece 3 step P3-8: the pass P3-7 added to production and did NOT add here. It routes
    // exactly ONE image barrier and no buffer barriers.
    PassBarrierRange { img_begin: 11, img_count: 1, buf_begin: 9, buf_count: 0 }, // [11] "hzb_dump_depth_early"
    PassBarrierRange { img_begin: 12, img_count: 2, buf_begin: 9, buf_count: 4 }, // [12] "vb_cull_late"
    PassBarrierRange { img_begin: 14, img_count: 2, buf_begin: 13, buf_count: 2 }, // [13] "vb_raster_late"
    PassBarrierRange { img_begin: 16, img_count: 4, buf_begin: 15, buf_count: 1 }, // [14] "vb_resolve"
    PassBarrierRange { img_begin: 20, img_count: 1, buf_begin: 16, buf_count: 0 }, // [15] "present_sample"
    PassBarrierRange { img_begin: 21, img_count: 2, buf_begin: 16, buf_count: 0 }, // [16] "hzb_dump"
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
        res: ResId(14), // [3] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 10, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [4] "vb_id"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [5] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
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
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
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
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 6, mip_count: 4, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [10] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 5, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(14), // [11] "hzb_pyramid"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 6, mip_count: 4, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [12] "vb_id"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [13] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(2), // [14] "vb_depth"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(5), // [15] "viewt"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(1), // [16] "vb_id"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(8), // [17] "thin_normal"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(8), // [18] "thin_normal"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(5), // [19] "viewt"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(9), // [20] "ssao"
        src_stage: VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [21] "lit"
        src_stage: VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(3), // [22] "cascade"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 4 },
    },
    ImgBarrier {
        res: ResId(4), // [23] "atlas"
        src_stage: VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT | VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
        new_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_DEPTH_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 16 },
    },
    ImgBarrier {
        res: ResId(9), // [24] "ssao"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [25] "lit"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
        old_layout: VK_IMAGE_LAYOUT_GENERAL,
        new_layout: VK_IMAGE_LAYOUT_GENERAL,
        subresource: SubRange { aspect: VK_IMAGE_ASPECT_COLOR_BIT, base_mip: 0, mip_count: 1, base_layer: 0, layer_count: 1 },
    },
    ImgBarrier {
        res: ResId(0), // [26] "lit"
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
        res: ResId(31), // [5] "vb_cull_uniform"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(23), // [6] "vb_indirect"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    BufBarrier {
        res: ResId(16), // [7] "vb_instance_ring"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(27), // [8] "vb_visible_instance"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(30), // [9] "vb_late_count"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(29), // [10] "vb_late_visible"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(29), // [11] "vb_late_visible"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: 0,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(28), // [12] "vb_indirect_late"
        src_stage: VK_PIPELINE_STAGE_TRANSFER_BIT,
        dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        src_access: VK_ACCESS_TRANSFER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_WRITE_BIT,
    },
    BufBarrier {
        res: ResId(28), // [13] "vb_indirect_late"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
    },
    // VG R3 piece 3 step P3-8: `vb_raster_late`'s `vb_late_visible` VERTEX read. DERIVED from the
    // declarations, not regenerated: `vb_cull_late` leaves a pending COMPUTE/SHADER_WRITE flush on
    // this buffer (its read-then-write pair ends in the write), so the next access takes
    // `sync::transition`'s RAW arm — `src = (flush_stages, flush_access)` — and the dst is the
    // VERTEX/SHADER_READ this pass declares. The sibling `vb_instance_ring` read declared beside it
    // adds NO barrier: `vb_raster` already read that buffer at VERTEX/SHADER_READ and nothing wrote
    // it since, so `stage & !visible_stages` and `access & !visible_access` are both zero and
    // `need` is false.
    BufBarrier {
        res: ResId(29), // [14] "vb_late_visible"
        src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        dst_stage: VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
        src_access: VK_ACCESS_SHADER_WRITE_BIT,
        dst_access: VK_ACCESS_SHADER_READ_BIT,
    },
    BufBarrier {
        res: ResId(15), // [15] "light_table"
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
    PassBarrierRange { img_begin: 3, img_count: 1, buf_begin: 1, buf_count: 5 }, // [6] "vb_batch_cull"
    PassBarrierRange { img_begin: 4, img_count: 2, buf_begin: 6, buf_count: 3 }, // [7] "vb_raster"
    PassBarrierRange { img_begin: 6, img_count: 2, buf_begin: 9, buf_count: 0 }, // [8] "hzb_build_0"
    PassBarrierRange { img_begin: 8, img_count: 2, buf_begin: 9, buf_count: 0 }, // [9] "hzb_build_1"
    PassBarrierRange { img_begin: 10, img_count: 2, buf_begin: 9, buf_count: 4 }, // [10] "vb_cull_late"
    PassBarrierRange { img_begin: 12, img_count: 2, buf_begin: 13, buf_count: 2 }, // [11] "vb_raster_late"
    PassBarrierRange { img_begin: 14, img_count: 2, buf_begin: 15, buf_count: 0 }, // [12] "vb_viewt"
    PassBarrierRange { img_begin: 16, img_count: 2, buf_begin: 15, buf_count: 0 }, // [13] "vb_geo"
    PassBarrierRange { img_begin: 18, img_count: 3, buf_begin: 15, buf_count: 0 }, // [14] "ssao"
    PassBarrierRange { img_begin: 21, img_count: 4, buf_begin: 15, buf_count: 1 }, // [15] "vb_shade_split"
    PassBarrierRange { img_begin: 25, img_count: 1, buf_begin: 16, buf_count: 0 }, // [16] "sdf_forward_march"
    PassBarrierRange { img_begin: 26, img_count: 1, buf_begin: 16, buf_count: 0 }, // [17] "present_sample"
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
/// into the unarmed path") and, on a split row, the TWO-link chain across its two declared
/// producers.
///
/// # RE-PINNED at VG R3 P3-3, and why the count is no longer the whole assertion
///
/// This said *"EXACTLY ONE on a split row … here only its existence is claimed"*. The number moved
/// because `vb_cull_late` became a second declared producer of the record array (plan D8): the host
/// upload no longer sources the indirect fetch, the cull does, and the upload now sources the cull.
/// Two producers, two barriers.
///
/// The original claim — *"zero means one of the two halves is undeclared, which is a MISSING
/// barrier that nothing else on this machine can see"* — is PRESERVED at the new number, because
/// deleting any one of the three links leaves exactly ONE barrier (a first-touch buffer write emits
/// none, `framegraph/sync.rs`'s `transition`). What the count can NOT see is an access declared
/// with the wrong ACCESS MASK: declare the late cull's `vb_indirect_late` access as a
/// `SHADER_READ` and the count is still two, with `src_access = 0` on the second — an execution-only
/// edge that orders the stages and flushes nothing. So the four `(stage, access)` pairs are asserted
/// here as well. They are a restatement of two elements of the whole-stream baseline below, and
/// they earn it the way every named hazard in this file does: they say WHICH property broke.
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
    let late = buf_on(buf, f.vb_indirect_late);
    let late_barriers = late.len();
    if row.split {
        assert_eq!(
            late_barriers, 2,
            "configuration {}: `vb_indirect_late` routed {late_barriers} barriers on an ARMED \
             SPLIT, expected exactly TWO — the links either side of `vb_cull_late`'s COMPUTE write, \
             which is the second declared producer plan D8 adds to the host upload. ONE means a \
             link of the chain is undeclared, and each way of getting there is a MISSING barrier: \
             without `vb_indirect_late_upload`'s TRANSFER write the cull's store is a first touch \
             and the host fill is never made available; without `vb_cull_late`'s SHADER_WRITE the \
             fetch is sourced from the host upload and piece 2's obligation 1 is undischarged; \
             without `vb_raster_late`'s INDIRECT_COMMAND_READ the record array is written and never \
             ordered against its consumer. ZERO means the whole chain is gone. Nothing else on this \
             machine can see any of them (P2-0: a genuine missing barrier produced the unchanged \
             19-message validation baseline and a byte-identical golden).",
            row.id
        );
        assert_eq!(
            (late[0].src_stage, late[0].src_access, late[0].dst_stage, late[0].dst_access),
            (
                VK_PIPELINE_STAGE_TRANSFER_BIT,
                VK_ACCESS_TRANSFER_WRITE_BIT,
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT
            ),
            "configuration {}: `vb_indirect_late`'s FIRST barrier must be the host fill flushed to \
             the late cull's store — `TRANSFER(TRANSFER_WRITE) → COMPUTE_SHADER(SHADER_WRITE)`, the \
             WAW between the two declared producers. A `dst_access` that is not a WRITE means \
             `vb_cull_late` declares a READ where it stores `instanceCount`, which keeps the count \
             at two and is invisible to every other gate.\nGot: {:#?}",
            row.id,
            late[0]
        );
        assert_eq!(
            (late[1].src_stage, late[1].src_access, late[1].dst_stage, late[1].dst_access),
            (
                VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
                VK_ACCESS_SHADER_WRITE_BIT,
                VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
                VK_ACCESS_INDIRECT_COMMAND_READ_BIT
            ),
            "configuration {}: `vb_indirect_late`'s SECOND barrier must be that store made \
             available to the indirect FETCH — `COMPUTE_SHADER(SHADER_WRITE) → \
             DRAW_INDIRECT(INDIRECT_COMMAND_READ)`. `src_access = 0` here is the fingerprint of a \
             producer declared as something other than a write: an execution-only edge that orders \
             the stages while leaving the `instanceCount` store unflushed, so the fetch may read \
             the host's `0` and the late scope draws nothing FOREVER. A `src_stage` of TRANSFER \
             means the cull's write is missing and the fetch fell back to the host upload.\n\
             Got: {:#?}",
            row.id,
            late[1]
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

/// **G4 row S1** — split ON, HZB off, dump off, SSAO off, `VB × Mesh`: the new barriers at the late
/// scope's boundary, with nothing else in the frame to hide behind.
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

/// **S1's claim, and it is the load-bearing one of the whole piece.** The barriers at the late
/// scope's boundary, asserted FIELD BY FIELD.
///
/// ⚠️ **A bare count is not the claim, and asserting only a count would certify the defect.** An
/// access declared with the wrong mask — a READ where the chain needs a WRITE — yields the same
/// count, differing only in `src_stage` / `src_access`. Round 1 specified this gate as a count,
/// which would have gone RED on the correct implementation and GREEN on the defective one.
///
/// # RE-PINNED at VG R3 P3-3, and the ONE claim that had to be replaced rather than renumbered
///
/// This asserted `vb_indirect_late`'s `TRANSFER(TRANSFER_WRITE) →
/// DRAW_INDIRECT(INDIRECT_COMMAND_READ)` edge — the host upload flushed straight to the fetch.
/// **That edge no longer exists.** `vb_cull_late` writes the record array (plan D8), so the fetch is
/// sourced from the CULL and the upload is sourced to the cull; one barrier became two and neither
/// is the one this test named.
///
/// **What the successor catches, and why the first link is the sharper of the two.** After P3-3 the
/// upload→cull WAW is the ONLY place in the entire derived stream where `vb_indirect_late_upload`'s
/// existence is observable at all: delete that declaration and the fetch is still correctly sourced
/// from the cull, so a gate that looked only at the fetch would be GREEN while the host fill of the
/// four record words the cull does not write (`indexCount`, `firstIndex`, `vertexOffset`,
/// `firstInstance`) is neither available to the fetch nor ordered against the cull's own store —
/// which can therefore be clobbered by a `vkCmdUpdateBuffer` that has not yet run. That is
/// [`a_dropped_late_upload_write_deletes_the_upload_to_cull_waw`]'s subject, measured on this
/// replica.
///
/// ⚠️ **Weaker in one way, stated:** the old assertion's `(TOP_OF_PIPE, 0)` fingerprint proved "no
/// producer was declared at all". The first link's source can no longer take that value on a
/// well-formed frame — the upload is the buffer's first touch, so an undeclared upload deletes the
/// barrier instead of corrupting it. The count assertion above is what carries that half now.
///
/// The two attachment WAWs are asserted **not to come from `UNDEFINED`**, because a first touch
/// there would license the driver to DISCARD what the early scope wrote — which is the equivalence
/// (`LOAD_OP_LOAD` yields what the early scope stored) the whole piece rests on. P3-3 moves neither.
#[test]
fn s1_pins_the_late_boundary_barriers_field_by_field() {
    let f = declare_vb_frame(S1);
    let img = f.g.img_barriers();
    let buf = f.g.buf_barriers();

    let late = buf_on(buf, f.vb_indirect_late);
    assert_eq!(
        late.len(),
        2,
        "the late record array carries exactly TWO barriers on a split frame — one per declared \
         producer boundary. A count of one means a link is undeclared; see \
         `assert_row_is_pinned` for which deletion yields which.\nGot: {late:#?}"
    );
    assert!(
        has_buf(
            buf,
            f.vb_indirect_late,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_TRANSFER_WRITE_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
        ),
        "the late record array's TRANSFER_WRITE → SHADER_WRITE edge is missing. This is the ONLY \
         barrier in the whole frame that records that `vb_indirect_late_upload` exists: after P3-3 \
         the indirect fetch is sourced from `vb_cull_late`, so dropping the upload's declaration \
         leaves the fetch looking healthy while the host's `vkCmdUpdateBuffer` fill of the four \
         record words the cull does NOT write is neither ordered against the cull's store nor made \
         available to the fetch — on frame 1, against freshly allocated device memory, with \
         `robustBufferAccess` OFF. Nothing else in this repository can see that: it changes no \
         pixel and emits no validation message (measured — the plan's P2-0 table).\n\
         Got: {late:#?}"
    );
    assert!(
        has_buf(
            buf,
            f.vb_indirect_late,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
        ),
        "the late record array's SHADER_WRITE → INDIRECT_COMMAND_READ edge is missing or \
         mis-sourced. A `src_stage` of TRANSFER means `vb_cull_late`'s store of `instanceCount` is \
         undeclared and the fetch still hangs off the host upload — piece 2's obligation 1 \
         undischarged, and the barrier COUNT is unchanged by it. A `src_access` of 0 means the cull \
         declares a non-write access: the stages are ordered and the store is never flushed.\n\
         Got: {late:#?}"
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
/// recorded.
///
/// P3-0's tail assertion said `hzb_build_0` was still the first pyramid writer and that the seed
/// was therefore what sourced ITS write. The seed still sources the frame's first pyramid access —
/// but on a SPLIT row that access is no longer the build. See the next section, which re-pins it.
///
/// # RE-PINNED AGAIN at VG R3 P3-3 — the build's first write is a WAR now, and the seed is carried
/// by the barrier AHEAD of it
///
/// P3-3 gave the EARLY cull a declared READ of the whole pyramid, gated on `split &&
/// hzb_levels.is_some()`. It DECLARES the early predicate's INPUT — the pyramid **as the previous
/// frame left it**, before this frame's build overwrites it (plan D1) — so on a split row it, and
/// not `hzb_build_0`, is the frame's first pyramid access. Two consequences, and plan D2's hazard
/// table predicts both by name:
///
/// * The **cross-frame RAW** — frame N's last `hzb_build_*` write → frame N+1's early cull read —
///   is discharged AT THAT READ. It is the barrier that carries the seed now:
///   `(COMPUTE, SHADER_WRITE) → (COMPUTE, SHADER_READ)`, `GENERAL → GENERAL`, over all ten mips in
///   ONE run.
/// * `hzb_build_0`'s write then finds `flush_access == 0` (the read cleared it) and
///   `visible_stages == COMPUTE`, so `sync::transition` takes its **WAR** arm (`sync.rs:376-379`)
///   and derives an EXECUTION-only dependency: `src_access` is `0` BY DESIGN, because a read has
///   no memory to make available. This is the barrier D2 argues the cross-frame WAR away with —
///   frame N's late-cull read → frame N+1's pyramid write is "subsumed", it says, because frame
///   N+1 derives an intra-frame WAR against its OWN `vb_batch_cull` read and a barrier recorded
///   outside a render pass orders against everything earlier in submission order on the single
///   queue. That barrier is this one, and here it is observed rather than argued.
///
/// **The MERGE survives, and is now asserted on a wider span as well.** The build's six mips are
/// all in ONE state — the state the early read left them in — and still fold into ONE barrier over
/// `[0, 6)`; the early read's ten fold into one over `[0, 10)`. Unfolded, this row's pyramid would
/// carry 10 + 6 + 1 + 4 + 9 = 30 barriers instead of SIX, so the census below is a second,
/// independent statement of the same P1-5a property.
///
/// # What the two re-pinned assertions catch
///
/// * **A lost or reverted seed** — `(TOP_OF_PIPE, 0, UNDEFINED → GENERAL)` on the early read. On a
///   split row that read is the first access, so it is the ONLY access the seed can source, and
///   this is where the D2 claim lives here.
/// * **A DROPPED early pyramid read** — the declaration disappearing, or being re-gated off the
///   split. Both assertions red on it: the census falls to five, and the build's write reverts to
///   the seed FLUSH the unsplit rows carry.
/// * **A broken merge**, on both halves — the run's own `mip_count` and the census.
/// * **A re-based or narrowed span**, on either the read or the build.
/// * **The two ordered the wrong way round** — which is why they are asserted BY POSITION. D1's
///   whole premise is that the early predicate sees the PREVIOUS frame's pyramid.
///
/// ⚠️ **What that dropped read would COST is not yet a soundness defect, and saying otherwise
/// would be a claim about a shader that does not exist.** `vb_batch_cull.comp.hlsl`'s occlusion
/// leaf lands at step P3-4; today neither phase samples the pyramid, and P3-3 declares the access
/// AHEAD of its consumer on purpose. So this pair currently gates the DECLARATION — that the graph
/// models the early phase's input, in the shape D2's cross-frame argument assumes — and becomes a
/// gate on a live stale-read hazard the moment P3-4 arms the leaf. That is the same order this
/// file's other P3-3 rows are in, and it is why the pair is worth pinning now rather than after.
///
/// # Where this is WEAKER than what it replaced — stated, not left to be discovered
///
/// ⚠️ **The build-write assertion no longer sees the seed at all.** Its `src` is derived from THIS
/// frame's early read, so a seed reverted to `undefined()` leaves it byte-identical and only the
/// read assertion moves. The unsplit rows keep the old witness
/// (`u2_pins_the_pyramid_chain_and_the_depth_handoff`), which is why they were left alone: the
/// early cull declares no pyramid read there, so `hzb_build_0`'s write is still their frame's
/// first pyramid access and still flushes the seed. (Their IMAGE streams are unmoved by P3-3;
/// what P3-3 moves on them is one buffer barrier, argued in the module doc.)
///
/// ⚠️ `src_access: 0` is CORRECT here but proves less than a flush does: it says only that SOME
/// prior COMPUTE read of these mips is ordered ahead of the write, not that it was the early
/// cull's. Nothing else on this row reads them before the build — the census is what says so —
/// but this line alone does not separate the two.
///
/// ⚠️ And still weaker in the one way `u2`'s doc spells out: `UNDEFINED` proved "untouched this
/// frame", `GENERAL → GENERAL` does not.
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

    // ---- The pyramid, RE-PINNED at VG R3 P3-3 — the argument is in this test's doc ------------
    //
    // Asserted BY POSITION, because "the early cull's read comes BEFORE this frame's build" IS the
    // claim: a membership test goes green on a stream that ordered the two the other way round,
    // and that order is the one plan D1 forbids.
    let pyr = img_on(img, f.hzb_pyramid);
    assert_eq!(
        pyr.len(),
        6,
        "an undumped ARMED-SPLIT frame derives SIX pyramid barriers: the early cull's read over the \
         whole chain, `hzb_build_0`'s merged write over [0, 6), pass 1's RAW over mip 5 ALONE, pass \
         1's merged write over [6, 10), and the late cull's read — which arrives as TWO runs, \
         [0, 5) and [6, 10), because mip 5 is already visible to a COMPUTE read from pass 1's own \
         reduce. This count IS the merge stated as a census: unfolded, the same accesses would \
         derive 10 + 6 + 1 + 4 + 9 = 30.\nGot: {pyr:#?}"
    );
    assert_eq!(
        *pyr[0],
        ImgBarrier {
            res: f.hzb_pyramid,
            src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            src_access: VK_ACCESS_SHADER_WRITE_BIT,
            dst_access: VK_ACCESS_SHADER_READ_BIT,
            old_layout: VK_IMAGE_LAYOUT_GENERAL,
            new_layout: VK_IMAGE_LAYOUT_GENERAL,
            subresource: hzb_mips(0, HZB_LEVELS),
        },
        "on a split row the EARLY cull's read is the frame's FIRST pyramid access, and it is the \
         barrier that discharges plan D2's cross-frame RAW: frame N's last `hzb_build_*` write made \
         AVAILABLE to frame N+1's early predicate, which by D1 tests against the pyramid as the \
         PREVIOUS frame left it.\n\
         `(TOP_OF_PIPE, 0, UNDEFINED)` means the SEED was lost — the read is then unordered against \
         the sibling in-flight frame's still-pipelined write of this NON-RINGED image, with a \
         licensed content discard on top. A barrier missing here entirely means the read was \
         DROPPED or re-gated off the split, which leaves the cross-frame RAW discharged at the \
         BUILD — after the phase that is supposed to consume it. And the ten mips must arrive as \
         ONE run"
    );
    assert_eq!(
        *pyr[1],
        ImgBarrier {
            res: f.hzb_pyramid,
            src_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            dst_stage: VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            src_access: 0,
            dst_access: VK_ACCESS_SHADER_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_GENERAL,
            new_layout: VK_IMAGE_LAYOUT_GENERAL,
            subresource: hzb_mips(0, HZB_LEVELS_PER_PASS),
        },
        "`hzb_build_0`'s write must be a WAR against the read above — an EXECUTION-only dependency, \
         `src_access = 0`, because a read has nothing to make available. That is how plan D2's \
         cross-frame WAR is subsumed: this same barrier orders the write after frame N's late-cull \
         read through single-queue submission order.\n\
         A `src_access` of SHADER_WRITE here means the frame's first pyramid access is the BUILD \
         again — the early cull's read is undeclared and the early predicate has no ordered input; \
         the row has reverted to the unsplit shape \
         `u2_pins_the_pyramid_chain_and_the_depth_handoff` pins.\n\
         Its six mips are all left in ONE state by the read above, so they must still MERGE into \
         ONE barrier over [0, 6) — six single-mip barriers here is the per-mip state machine having \
         stopped folding, the whole of what P1-5a shipped. GENERAL → GENERAL is layout-PRESERVING: \
         `UNDEFINED` would license discarding a pyramid that now carries across frames (plan D2)"
    );
}

/// **S3's claim.** `hzb_dump`'s `vb_depth` source has CHANGED CHARACTER, and this is the asserted
/// -correct value rather than a regression — and, since VG R3 piece 3 step P3-8, the fact that a
/// dumped split frame now takes **TWO** depth copies whose sources are DIFFERENT.
///
/// On an unsplit armed frame the dump finds the depth already in `SHADER_READ_ONLY_OPTIMAL` where
/// `hzb_build_0` left it (`u3_pins_the_poison_whole_chain_waw_and_the_dump_layout_pair` pins that).
/// With the block moved between the scopes, the last toucher is `vb_raster_late` with a PENDING
/// WRITE, so the dump's transition becomes a real RAW flush out of the depth attachment. That is
/// strictly stronger than the execution-only edge it replaces — and it is why the declarator's own
/// comment ("on every armed frame that is `SHADER_READ_ONLY_OPTIMAL`, since `hzb_build_0` itself
/// reads it there") had to be corrected in P2-5 rather than left standing.
///
/// # ⚠️ P3-8: the two copies must be told apart BY SOURCE, and this test used to pick the wrong one
///
/// Step P3-7 added `hzb_dump_depth_early` to production — a SECOND `TRANSFER_SRC_OPTIMAL` transition
/// on the same image, earlier in the frame — and this test selected `find(new_layout ==
/// TRANSFER_SRC_OPTIMAL)`, i.e. the FIRST. Once the replica models the pass, that predicate returns
/// the EARLY copy, whose source is `(COMPUTE, 0)` — the exact value the assertion below calls the
/// defect. Both are now selected explicitly and pinned separately, which is also the stronger claim:
/// the pair says the two copies observe DIFFERENT states of the image, which is the whole reason
/// there are two of them.
#[test]
fn s3_pins_the_re_sourced_hzb_dump_depth_read() {
    let f = declare_vb_frame(S3);
    let img = f.g.img_barriers();

    let depth = img_on(img, f.vb_depth);
    let copies: Vec<_> = depth
        .iter()
        .filter(|b| b.new_layout == VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL)
        .collect();
    assert_eq!(
        copies.len(),
        2,
        "a DUMPED SPLIT frame copies the depth TWICE — `hzb_dump_depth_early` between the scopes \
         (plan D10) and `hzb_dump` at frame end — so `vb_depth` must carry exactly two transitions \
         into TRANSFER_SRC_OPTIMAL. ONE means the early copy is not declared, and then the dump's \
         `flags` word claims an early region the graph never ordered a copy into.\nGot: {depth:#?}"
    );

    // The EARLY copy: it observes the depth exactly as `hzb_build_0` read it. Its source is an
    // EXECUTION dependency on that read (`src_access = 0`) because a read has nothing to make
    // available — and `SHADER_READ_ONLY_OPTIMAL` as the old layout is what says the copy happens
    // BEFORE the late scope has claimed the attachment again.
    assert_eq!(
        (copies[0].src_stage, copies[0].src_access),
        (VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, 0),
        "the EARLY depth copy must be sourced from `hzb_build_0`'s read. A `(FRAG, \
         DEPTH_STENCIL_ATTACHMENT_WRITE)` source here would mean it is declared AFTER the late \
         raster — and then it copies the FINAL depth while the header still calls it the early one, \
         which is control E2's defect arriving through the graph instead of through the writer"
    );
    assert_eq!(
        (copies[0].old_layout, copies[0].new_layout),
        (VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL),
    );
    assert_eq!(copies[0].dst_stage, VK_PIPELINE_STAGE_TRANSFER_BIT);
    assert_eq!(copies[0].dst_access, VK_ACCESS_TRANSFER_READ_BIT);
    assert_eq!(copies[0].subresource, SubRange::DEPTH);

    // The FRAME-END copy: the late raster wrote the depth after the early copy, so this one is a
    // real RAW flush out of the depth attachment.
    let dump = copies[1];
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

    // ---- The RETURN leg the early copy re-sources (VG R3 piece 3 step P3-8) --------------------
    //
    // `vb_raster_late`'s depth WAW now comes out of TRANSFER_SRC_OPTIMAL with the TRANSFER stage
    // folded into its source, where before P3-7 it came out of SHADER_READ_ONLY_OPTIMAL. The COUNT
    // is unchanged by that, which is exactly why this file pins FIELDS.
    let ret = depth
        .iter()
        .find(|b| b.new_layout == VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL && b.old_layout != VK_IMAGE_LAYOUT_UNDEFINED)
        .expect("invariant: the late raster claims the depth attachment back");
    assert_eq!(
        (ret.old_layout, ret.src_stage, ret.src_access),
        (
            VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT | VK_PIPELINE_STAGE_TRANSFER_BIT,
            0
        ),
        "the late scope must claim the depth back from where the EARLY DUMP COPY left it, ordered \
         against BOTH prior readers (the build's COMPUTE read and the copy's TRANSFER read). A \
         `SHADER_READ_ONLY_OPTIMAL` old layout with a bare COMPUTE source is the pre-P3-7 shape — \
         it means the early copy is not declared on this row"
    );
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

/// **G4's R1 red control** (VG R3 piece 2 step P2-6, RE-PINNED at piece 3 step P3-3) — the defect on
/// the resource piece 2 adds, and the one the plan names.
///
/// Drop `vb_indirect_late_upload`'s declared `buffer_access(TRANSFER, TRANSFER_WRITE)` while
/// leaving `vb_raster_late`'s indirect fetch in place.
///
/// # The claim INVERTED at P3-3, which is why the name changed with it
///
/// This test was `a_dropped_late_upload_write_keeps_the_count_and_moves_only_fields`, and that name
/// is now false in both halves. While the upload was `vb_indirect_late`'s ONLY declared producer the
/// drop was the read-declared/write-undeclared class: the fetch fell to the first-touch arm, the
/// count was preserved and `src_stage`/`src_access` moved to `(TOP_OF_PIPE, 0)`. `vb_cull_late` is
/// now a second producer (plan D8), and it sits BETWEEN the upload and the fetch, so:
///
/// * the fetch is sourced from the CULL and is **field-identical** with and without the defect — a
///   gate that inspects the fetch is GREEN on it;
/// * the cull's store becomes `vb_indirect_late`'s first touch, and a first-touch buffer write emits
///   NO barrier (`framegraph/sync.rs`'s `transition`), so the upload→cull WAW is simply **deleted**
///   — the count drops from two to one.
///
/// So the discriminators swapped ends: what used to be visible only in fields is now visible only in
/// the count. Both halves are asserted below in that new order, because the pair is the claim — a
/// count that moved with unchanged fields is a DELETED barrier, and this file's whole-stream pins
/// see it as a shifted `PassBarrierRange` rather than as a differing barrier.
///
/// # Why this body is no longer RELEASE-ONLY, and what that costs
///
/// P2-8 re-cut the framegraph's unwritten-read backstop on declared PROVENANCE, and while the upload
/// was the sole producer that made this corrupt frame uncompilable in a dev-profile build — hence
/// the `#[cfg(not(debug_assertions))]` this test carried and the `should_panic` sibling that held
/// the debug leg. **P3-3 retires that coverage**: the guard tests reads against "was anything
/// declared to write this before", `vb_cull_late`'s write satisfies it, and the fetch is no longer a
/// first-touch read. The frame compiles on both legs now, so this test runs on both legs — which is
/// strictly more coverage than it had, and is also the honest signal that the stronger property is
/// gone. Nothing replaces it; plan D8 says so in as many words ("after piece 3, `vb_indirect_late`'s
/// provenance is covered by nothing"), and
/// [`the_dropped_early_survivor_write_trips_the_guard_through_the_split_read`] takes the vacated
/// debug-leg slot on a DIFFERENT resource rather than pretending to fill this one.
///
/// When this control was first written nothing else on this machine could see the defect, and that
/// is still true: it changes no pixel (`instanceCount = 0` draws nothing either way) and emits no
/// validation message (measured: a genuine missing barrier produced the unchanged 19-message
/// baseline and no `SYNC-HAZARD`).
#[test]
fn a_dropped_late_upload_write_deletes_the_upload_to_cull_waw() {
    let faithful = declare_vb_frame(S1);
    let corrupt = declare_vb_frame(VbRow {
        id: "S1 + RED CONTROL R1 (late upload's transfer write undeclared)",
        red_control_drop_late_upload_write: true,
        ..S1
    });

    let f_buf = faithful.g.buf_barriers();
    let c_buf = corrupt.g.buf_barriers();

    // The control must isolate ONE buffer declaration.
    assert_eq!(
        faithful.g.img_barriers(),
        corrupt.g.img_barriers(),
        "the image stream moving too would mean this control is testing something else"
    );

    // (1) A COUNT gate is RED on the defect — the half that used to be green.
    let faithful_late = buf_on(f_buf, faithful.vb_indirect_late);
    let corrupt_late = buf_on(c_buf, corrupt.vb_indirect_late);
    assert_eq!(
        faithful_late.len(),
        2,
        "the correct frame carries the two-link chain across `vb_cull_late`; if it does not, this \
         control's premise moved and its conclusion means nothing.\nGot: {faithful_late:#?}"
    );
    assert_eq!(
        corrupt_late.len(),
        1,
        "with the upload's write undeclared the cull's store is a FIRST TOUCH, which emits no \
         barrier — so the upload→cull WAW is deleted rather than mis-sourced.\n\
         Got: {corrupt_late:#?}"
    );
    assert_eq!(
        f_buf.len(),
        c_buf.len() + 1,
        "exactly one barrier is lost frame-wide: the defect neither adds nor moves any other"
    );

    // The deleted barrier is attributed to `vb_cull_late` — derived from the two frames rather than
    // pinned as a literal, so a change to the late cull's other accesses does not fake this claim.
    let cull_late = faithful
        .pass_names
        .iter()
        .position(|n| *n == "vb_cull_late")
        .expect("invariant: a split row declares the late cull");
    assert_eq!(
        faithful.pass_names, corrupt.pass_names,
        "the control drops an ACCESS, never a pass — the two frames must declare the same passes \
         or the index below labels different ones"
    );
    assert_eq!(
        corrupt.g.pass_barriers()[cull_late].buf_count + 1,
        faithful.g.pass_barriers()[cull_late].buf_count,
        "the lost barrier belongs to `vb_cull_late`: it is the flush of the host fill INTO the \
         cull's store, so its absence is a hazard at that pass and not at the raster"
    );

    // (2) A FIELD gate on the surviving barrier is GREEN on the defect — the half that used to be
    // red, and the reason (1) is not cosmetic.
    assert_eq!(
        corrupt_late[0], faithful_late[1],
        "the indirect FETCH's barrier is field-identical with and without the defect, because \
         `vb_cull_late` sources it either way. Any gate that inspects the fetch — this file's own \
         predecessor assertion included — is GREEN on a missing host-fill ordering, and the host \
         fill supplies the four record words the cull does not write"
    );
}

/// **The REPLACEMENT for `the_dropped_late_upload_write_now_trips_the_framegraph_guard`**, which VG
/// R3 P3-3 made unable to fire — and the demonstration that plan D8's read-then-write SPLIT buys the
/// coverage it is charged for.
///
/// # Why the test this replaces cannot fire, and why it was not renumbered
///
/// That control dropped `vb_indirect_late_upload`'s TRANSFER write and asserted that `compile`'s
/// P2-8 provenance guard rejected the frame. The guard fires on a READ of a resource nothing
/// declared a write to; `vb_raster_late`'s indirect fetch was that read while the upload was the
/// buffer's only producer. P3-3 gives `vb_indirect_late` a SECOND producer — `vb_cull_late` — which
/// is declared BEFORE the fetch, so the fetch is no longer a first-touch read and dropping the
/// upload leaves nothing for the guard to catch. There is no number to change: the property is gone.
/// Plan D8 states the consequence without softening it — *"after piece 3, `vb_indirect_late`'s
/// provenance is covered by nothing"* — and the bounded fix (P2-7's `is_write || res_written ||
/// res_seeded` for both kinds, plus the 14-site `add_buffer` audit) is a framegraph-core change that
/// piece 3 does not take. The derived-stream half of that control survives, on both legs, at
/// [`a_dropped_late_upload_write_deletes_the_upload_to_cull_waw`].
///
/// # What this replacement catches
///
/// Drop `vb_batch_cull`'s declared `buffer_access(vb_late_visible, COMPUTE, SHADER_WRITE)` — the
/// EARLY phase's write of the candidate list — while leaving `vb_cull_late`'s read of it in place.
/// `vb_late_visible` is a bare `add_buffer` (the provenance claim: this graph writes it every frame
/// it is read) and that early write is its first touch, so the late cull's read becomes a
/// first-touch read and the guard fires, naming both the pass and the buffer.
///
/// **This is not the `vb_late_count` control with a different resource.** `vb_cull_late` declares
/// `vb_late_visible` as TWO calls — `SHADER_READ`, then `SHADER_WRITE` — precisely so the guard can
/// test the read half, and D8 pays a second self-WAR execution-only edge for it. Under a combined
/// `SHADER_READ | SHADER_WRITE` the access would be `is_write`, the guard would never test it, and
/// this control would go GREEN (i.e. fail to panic) while the early producer was undeclared. That
/// "simplification" is a one-line edit nothing else in this tree would notice, and this is the test
/// that notices it. `the_dropped_late_count_write_now_trips_the_framegraph_guard` cannot: its
/// consumer access is a plain read and would be unaffected by the merge.
///
/// # What it does NOT catch
///
/// Nothing about `vb_indirect_late`. It is a different resource, and the coverage P3-3 retired there
/// is retired; this control does not restore it and must not be read as doing so. Nor anything about
/// `declare_vb_graph`: it fires on the REPLICA, which is what a `cargo test` can reach, and P2-7
/// measured that a replica pin is green when the PRODUCTION declarator loses an access. What makes
/// the property hold in production is that the real declarator runs the SAME `compile` on the same
/// declaration shape in every dev-profile run — and every golden run is one.
///
/// The `expected` substring names the PASS and the BUFFER, so neither an unrelated panic nor the
/// same guard firing on a different resource can satisfy this test.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "pass 'vb_cull_late' reads UNWRITTEN transient buffer 'vb_late_visible'")]
fn the_dropped_early_survivor_write_trips_the_guard_through_the_split_read() {
    let _ = declare_vb_frame(VbRow {
        id: "S1 + RED CONTROL (early cull's vb_late_visible write undeclared)",
        red_control_drop_cull_late_visible_write: true,
        ..S1
    });
}

/// **G-P3-F's F4 red control** (VG R3 piece 3 step P3-3) — the ONE new provenance coverage this
/// step buys, demonstrated rather than claimed.
///
/// Drop `vb_batch_cull`'s declared `buffer_access(vb_late_count, COMPUTE, SHADER_WRITE)` while
/// leaving `vb_cull_late`'s `SHADER_READ` of it in place. `vb_late_count` is a bare `add_buffer`
/// (plan D3/D8: it HAS an in-graph producer every frame it is read), and that write is its FIRST
/// TOUCH, so the read becomes a first-touch read and `compile`'s P2-8 provenance guard fires.
///
/// # Why this control, and not one on `vb_late_visible` or `vb_indirect_late`
///
/// * `vb_indirect_late`'s first touch is `vb_indirect_late_upload`'s TRANSFER write, and the guard
///   tests `is_write || res_written` — a WRITE is never tested. After P3-3 the next declaration in
///   its list is `vb_cull_late`'s write, so **`vb_indirect_late`'s provenance is covered by
///   nothing**, and that sentence is the plan's own (D8). This control does not close it.
/// * `vb_late_visible` is covered too, but only BECAUSE `vb_cull_late` declares its read and its
///   write as TWO calls. Under a combined `SHADER_READ|SHADER_WRITE` the access would be `is_write`
///   and the read half would never be tested — which is the reason the declarator splits it and
///   pays a self-WAR edge for the privilege. That sentence was a CLAIM until the P3-3 re-pin gave
///   it a fixture: [`the_dropped_early_survivor_write_trips_the_guard_through_the_split_read`] is
///   the one that goes green if the two calls are ever merged, and this control cannot — its
///   consumer access is a plain read, unaffected by the merge.
///
/// # What this control CANNOT claim
///
/// Nothing about `declare_vb_graph`. This fires on the REPLICA, which is what a `cargo test` can
/// reach; what makes the property hold in production is that the real declarator runs the SAME
/// `compile` on the same declaration shape, in every dev-profile run — and every golden run is one.
/// P2-7 measured that a replica pin is green when the PRODUCTION declarator loses an access.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "reads UNWRITTEN transient buffer")]
fn the_dropped_late_count_write_now_trips_the_framegraph_guard() {
    let _ = declare_vb_frame(VbRow {
        id: "S1 + RED CONTROL F4 (early cull's vb_late_count write undeclared)",
        red_control_drop_cull_late_count_write: true,
        ..S1
    });
}

/// `compile()` is idempotent and a re-declared frame reproduces its own stream — so a divergence
/// reported by the pins above is a change in the DECLARATION, never a stale arena.
///
/// Both legs matter: recompiling one graph catches a `compile` that accumulates, and rebuilding a
/// second frame catches a `reset` that leaks (`res_shape`/`res_state_total` are a prefix sum, and
/// the pyramid is the mipped resource that would index past the end if one survived).
///
/// The two PROBE-ON rows join the loop at VG R3 piece 4 step P4-5 rather than being left out of a
/// test whose name says "every row": the delta they are asserted through is a comparison of two
/// compiled streams, so "the stream a row compiles to is a function of the row" is a premise of it.
#[test]
fn every_row_is_reproducible_and_compile_is_idempotent() {
    for row in [U1, U2, U3, U4, S1, S2, S3, S4, P1, P3] {
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
    // The split adds EXACTLY THREE passes — `vb_indirect_late_upload`, `vb_cull_late` and
    // `vb_raster_late` — to each row it arms, and MOVES the poison+build block rather than
    // duplicating it. A `+4` or `+5` here would mean the block was declared at both slots, which is
    // how a "moved" block quietly becomes a second one.
    //
    // ⚠️ It was `+2` until VG R3 piece 3 step P3-3 added the late cull. The number is DERIVED — one
    // `pass!` call, added under the same `row.split` arm as the other two — so a `+2` surviving this
    // step would mean the late cull was never declared, which is the state in which
    // `vb_indirect_late`'s writer never moved to COMPUTE and piece 2's obligation 1 is undischarged.
    // The `vb_cull_readback_late` pass is NOT counted here: the probe is off across this matrix.
    //
    // ⚠️ VG R3 piece 3 step P3-8 makes the number ROW-DEPENDENT for the first time: a row that arms
    // the DUMP as well as the split gains a FOURTH pass, `hzb_dump_depth_early` (plan D10, added to
    // production at step P3-7 and to this replica at P3-8). The expectation is DERIVED from the
    // row's own `hzb_dump` flag rather than hard-coded per row, so a future row that arms the dump
    // inherits the right number instead of the number S3 happened to have.
    for (u, s) in [(0usize, 4usize), (1, 5), (2, 6), (3, 7)] {
        let added = if frames[s].row.hzb_dump { 4 } else { 3 };
        assert_eq!(
            frames[s].pass_count,
            frames[u].pass_count + added,
            "{} declares {} passes against {}'s {} — the split adds exactly {added} (the late \
             upload, the late cull, the late raster{}) and MOVES the poison+build block, never \
             duplicates it",
            frames[s].row.id,
            frames[s].pass_count,
            frames[u].row.id,
            frames[u].pass_count,
            if frames[s].row.hzb_dump { " and the early-depth dump copy" } else { "" }
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
                // VG R3 piece 3 step P3-3: the late cull's two order invariants, the replica half
                // of the `debug_assert!`s `declare_vb_graph` now carries. Both are load-bearing in
                // OPPOSITE directions — declared before the builds it would test the PREVIOUS
                // frame's pyramid (which is the EARLY phase's predicate, not this one's); declared
                // after the late raster its COMPUTE write would not be the indirect fetch's source
                // and the derived edge would order nothing.
                let late_cull =
                    index_of("vb_cull_late").expect("invariant: a split row declares the late cull");
                assert!(
                    build0 < late_cull && late_cull < late,
                    "S3: the armed order is `hzb_build_* → vb_cull_late → vb_raster_late`. Got \
                     build0={build0}, late_cull={late_cull}, late={late}"
                );
                // VG R3 piece 3 step P3-8: `hzb_dump_depth_early`'s POSITION IS ITS CORRECTNESS
                // (plan D10). After the last build, so what it copies is exactly what the builds
                // reduced; before the late raster, so nothing has drawn into the depth again. The
                // production declarator carries both as `debug_assert!`s; this is the replica half,
                // reachable from a plain `cargo test`.
                let early_dump = index_of("hzb_dump_depth_early")
                    .expect("invariant: a split row that arms the dump declares the early copy");
                assert!(
                    build0 < early_dump && early_dump < late,
                    "S3: the early-depth copy must sit between the last `hzb_build_*` and \
                     `vb_raster_late`. Declared BEFORE the builds it would copy a depth they had \
                     not read yet; declared AFTER the late raster it would copy the FINAL depth \
                     while the header's flag still calls it the early one — and G-P3-E's clause 3 \
                     would then compare the pyramid against a rebuild from the same bytes it was \
                     built from, i.e. green by construction. Got build0={build0}, \
                     early_dump={early_dump}, late={late}"
                );
                assert!(
                    early_dump < dump,
                    "S3: the EARLY copy must precede the FRAME-END one, or the two regions of the \
                     dump file hold each other's bytes. Got early_dump={early_dump}, dump={dump}"
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

// ─────────────────────────────────────────────────────────────────────────────────────────────
// VG R3 piece 4 step P4-5 — the PROBE-ON delta
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// One derived buffer edge, spelled `src → dst` in the order a derivation reads it.
///
/// [`BufBarrier`]'s own field order interleaves the two halves (`res, src_stage, dst_stage,
/// src_access, dst_access`), while a derivation is written as a pair of `(stage, access)`
/// endpoints. An expectation that reads differently from the argument it was derived from is an
/// expectation with a transposition waiting in it, so this constructor takes the endpoints paired.
const fn edge(
    res: ResId,
    src_stage: u32,
    src_access: u32,
    dst_stage: u32,
    dst_access: u32,
) -> BufBarrier {
    BufBarrier { res, src_stage, dst_stage, src_access, dst_access }
}

/// The MULTISET difference `a \ b`, in `a`'s order: every element of `a` for which no *unconsumed*
/// equal element remains in `b`.
///
/// A multiset rather than a set, because two barriers with identical fields on the same resource
/// are a real possibility in a derived stream and a set difference would let one silently cancel
/// the other — reporting "no change" across an edit that deleted one of a pair.
fn multiset_minus(a: &[BufBarrier], b: &[BufBarrier]) -> Vec<BufBarrier> {
    let mut pool: Vec<BufBarrier> = b.to_vec();
    let mut out: Vec<BufBarrier> = Vec::new();
    for x in a {
        match pool.iter().position(|y| y == x) {
            // `swap_remove`: the pool is a bag, not a sequence — only membership counts.
            Some(i) => {
                pool.swap_remove(i);
            }
            None => out.push(*x),
        }
    }
    out
}

/// Render a derived-barrier list for a failure report, field by field (see [`describe_buf`]).
fn describe_buf_list(f: &VbFrame, bs: &[BufBarrier]) -> String {
    if bs.is_empty() {
        return "    (none)\n".to_string();
    }
    let mut s = String::new();
    for (i, b) in bs.iter().enumerate() {
        let _ = writeln!(s, "  [{i}]:");
        s.push_str(&describe_buf(f, b));
    }
    s
}

/// The PROBE-ON delta, returned as `(added, removed)` in COMPILED-STREAM ORDER.
///
/// # DERIVED from the declarations and `framegraph/sync.rs`'s `transition`, never regenerated
///
/// [`dump_vb_split_barrier_streams`]'s doc states the discipline this obeys: *a baseline authored
/// after the change certifies the new behaviour*, and this file's generators print streams as Rust
/// source, so pasting one would make the replica agree with production BY CONSTRUCTION. Every row
/// below is derived beside the declaration that produced it.
///
/// The probe declares NINE accesses across two passes, all of them `(TRANSFER, TRANSFER_READ)` on
/// buffers, and every buffer it names is written earlier in the frame, so `transition` never takes
/// a first-touch arm and `compile`'s P2-8 provenance guard is silent. `layout_change` is false
/// throughout — `buffer_access` passes the UNDEFINED sentinel a buffer never leaves.
///
/// ## The SIX pre-late reads (`vb_cull_readback`, declared between `vb_batch_cull` and `vb_raster`)
///
/// Each of the six finds `flush_access != 0` — the pass immediately before wrote all six buffers —
/// so each takes the RAW arm and sources `(flush_stages, flush_access)`. Five of those writes are
/// `vb_batch_cull`'s own COMPUTE stores; `vb_cull_count`'s is the `RW` access, whose pending flush
/// is `access & WRITE_ACCESS_MASK` = `SHADER_WRITE` alone. So all six are the SAME shape,
/// `COMPUTE_SHADER(SHADER_WRITE) → TRANSFER(TRANSFER_READ)`, and they differ only in `res`.
///
/// ## The FOUR re-sourcings that pass causes, which are the delta's real content
///
/// A read CLEARS the pending flush and accumulates `(stage, access)` into the visible pair. So the
/// next reader of each of those buffers no longer finds a flush: it finds `visible_stages ==
/// TRANSFER`, its own stage is not covered, and it takes the WAR/visibility-extend arm —
/// `(visible_stages, 0)`. Four readers sit downstream and move:
///
/// * `vb_indirect` at `vb_raster` — `COMPUTE(SHADER_WRITE) → DRAW_INDIRECT(INDIRECT_COMMAND_READ)`
///   becomes `TRANSFER(0) → DRAW_INDIRECT(INDIRECT_COMMAND_READ)`;
/// * `vb_visible_instance` at `vb_raster` — the same move to `TRANSFER(0) → VERTEX(SHADER_READ)`;
/// * `vb_late_count` and `vb_late_visible` at `vb_cull_late` — to `TRANSFER(0) → COMPUTE(SHADER_READ)`.
///
/// The count is unchanged on all four; TWO FIELDS move. That is the class this file exists for:
/// `src_access` going to `0` is an execution-only edge, and it is CORRECT here only because the
/// availability was already discharged by the TRANSFER read that consumed the flush.
///
/// ## The FIFTH move, which is a WIDENING and is easy to miss
///
/// `vb_cull_late` READS `vb_late_visible` and then WRITES it (two declared calls, so the P2-8 guard
/// can test the read half). That write takes the WAR arm on `visible_stages`, and `visible_stages`
/// is accumulated MONOTONICALLY across reads — so with the snapshot's TRANSFER read in front of the
/// pass's COMPUTE read, the self-WAR's `src_stage` is `TRANSFER | COMPUTE_SHADER` where PROBE-OFF
/// it is `COMPUTE_SHADER` alone. One field, one bit, and it is sound in the conservative direction:
/// the compaction's stores must indeed be ordered after the snapshot's copy.
///
/// ## The THREE post-late reads (`vb_cull_readback_late`, declared after `vb_raster_late`)
///
/// * `vb_late_visible` — last touched by `vb_raster_late`'s VERTEX read, so no flush is pending and
///   TRANSFER is not among the visible stages ⇒ the visibility-extend arm ⇒
///   `VERTEX_SHADER(0) → TRANSFER(TRANSFER_READ)`.
/// * `vb_late_count` — **NO BARRIER**. The pre-late snapshot already made `(TRANSFER,
///   TRANSFER_READ)` visible, `vb_cull_late` only READ it since, and a read does not re-arm a
///   flush. So `need` is false on all four terms and the access is FREE. Nine declared accesses,
///   EIGHT derived barriers: the one row of this derivation that a count-only expectation would
///   have got wrong.
/// * `vb_indirect_late` — last touched by the late raster's indirect fetch ⇒
///   `DRAW_INDIRECT(0) → TRANSFER(TRANSFER_READ)`.
///
/// ## What does NOT move, and it is the half the plan predicted backwards
///
/// The shipping chain `vb_indirect_late_upload → vb_cull_late → vb_raster_late` is FIELD-IDENTICAL
/// with and without the probe, because `vb_cull_readback_late` is declared AFTER the late raster
/// rather than between the cull and the fetch. See the module doc's P4-5 section for the plan
/// sentence this refutes and the declarator comment that predicted it.
///
/// `f` is either frame: `declare_vb_frame` declares every resource unconditionally and before any
/// pass, so `ResId` numbering is independent of the row (asserted at the call site).
fn probe_delta_expectation(f: &VbFrame) -> (Vec<BufBarrier>, Vec<BufBarrier>) {
    let added = vec![
        // ---- `vb_cull_readback`, six RAW flushes of the pass immediately before ----
        edge(
            f.vb_cull_count,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_ACCESS_TRANSFER_READ_BIT,
        ),
        edge(
            f.vb_cull_visible,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_ACCESS_TRANSFER_READ_BIT,
        ),
        edge(
            f.vb_indirect,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_ACCESS_TRANSFER_READ_BIT,
        ),
        edge(
            f.vb_visible_instance,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_ACCESS_TRANSFER_READ_BIT,
        ),
        edge(
            f.vb_late_visible,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_ACCESS_TRANSFER_READ_BIT,
        ),
        edge(
            f.vb_late_count,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_ACCESS_TRANSFER_READ_BIT,
        ),
        // ---- `vb_raster`, its two readers re-sourced onto the snapshot ----
        edge(
            f.vb_indirect,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            0,
            VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
            VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
        ),
        edge(
            f.vb_visible_instance,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            0,
            VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        ),
        // ---- `vb_cull_late`, its two reads re-sourced and its self-WAR widened ----
        // `vb_late_count` precedes `vb_late_visible` here, in the declarator's own access order.
        edge(
            f.vb_late_count,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            0,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        ),
        edge(
            f.vb_late_visible,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            0,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        ),
        edge(
            f.vb_late_visible,
            VK_PIPELINE_STAGE_TRANSFER_BIT | VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            0,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
        ),
        // ---- `vb_cull_readback_late`, two visibility extensions and one FREE access ----
        edge(
            f.vb_late_visible,
            VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
            0,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_ACCESS_TRANSFER_READ_BIT,
        ),
        edge(
            f.vb_indirect_late,
            VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
            0,
            VK_PIPELINE_STAGE_TRANSFER_BIT,
            VK_ACCESS_TRANSFER_READ_BIT,
        ),
    ];

    // The PROBE-OFF shapes the five moves above replace, in PROBE-OFF stream order. Each is a
    // barrier the twin row's whole-stream pin asserts by index, so this list is also the statement
    // of WHICH pinned elements the probe perturbs — the rest are untouched.
    let removed = vec![
        edge(
            f.vb_indirect,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
            VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
        ),
        edge(
            f.vb_visible_instance,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        ),
        edge(
            f.vb_late_count,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        ),
        edge(
            f.vb_late_visible,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_READ_BIT,
        ),
        edge(
            f.vb_late_visible,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            0,
            VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
            VK_ACCESS_SHADER_WRITE_BIT,
        ),
    ];

    (added, removed)
}

/// **VG R3 piece 4 step P4-5, disposition (c3)** — the PROBE-ON rows, asserted as a DELTA against
/// their PROBE-OFF twins.
///
/// # The debt this pays
///
/// G-P3-B/C — the gates that decide whether the occlusion split culls anything — run with
/// `BOYKO_VB_CULL_READBACK` armed, and their verdict is read out of buffers this probe copies. A
/// defect in the probe's barrier stream therefore makes a GATE lie: it reads stale bytes and
/// reports them as the GPU's answer. Every pinned row above holds the probe OFF, so until this step
/// that stream was modelled by nothing at all.
///
/// # What is asserted, and why a delta rather than two more field-by-field rows
///
/// The twin already carries the whole-stream pin, so re-pinning ~40 barriers to state 18 would put
/// the interesting ones inside two arrays nobody diffs. Asserted here:
///
/// 1. the IMAGE stream is byte-identical — the probe declares no image access, so anything moving
///    there means the probe branch touched a resource it has no business touching;
/// 2. the pass list gains EXACTLY `vb_cull_readback` immediately before `vb_raster` and
///    `vb_cull_readback_late` immediately after `vb_raster_late` — both positions are correctness,
///    not layout (the second one is the whole reason the shipping chain does not move);
/// 3. the BUFFER multiset difference, both directions, against [`probe_delta_expectation`];
/// 4. the shipping `vb_indirect_late` chain's first two links are FIELD-identical to the twin's,
///    with the snapshot appended as a third — the executable form of the declarator's own claim;
/// 5. the delta is the SAME on both rows, which is the derivation's own prediction (the probe
///    declares only buffer accesses; `S1` and `S3` differ only in image accesses).
///
/// [`assert_row_is_pinned`] is deliberately NOT called on `P1`/`P3`: its split branch asserts
/// `vb_indirect_late` routes exactly TWO barriers, and clause 4 above is precisely the statement
/// that a probe row routes THREE.
///
/// # The RED control
///
/// Delete the `vb_late_visible` `TRANSFER_READ` from `declare_vb_frame`'s `vb_cull_readback` branch
/// — one access — and this test reds in three places at once: the added list loses the pre-late
/// snapshot's flush, the `vb_late_visible`/self-WAR re-sourcings revert to their PROBE-OFF shapes
/// so the removed list empties by two, and the twin comparison in clause 5 still agrees (both rows
/// break identically), which is what makes clause 5 a claim about the derivation rather than a
/// self-check.
///
/// # What this CANNOT claim
///
/// Nothing about `declare_vb_graph`, for the reason stated in the module doc: this is a hand-written
/// REPLICA, and this campaign has MEASURED that a replica cannot see a missing barrier in the real
/// recorder (P2-0: a genuine deletion left the golden byte-identical and the validation baseline
/// unchanged at 19 messages). It covers one class — a future edit that changes the probe branch's
/// DECLARED accesses, or their position, without noticing what it does to four other readers.
#[test]
fn probe_on_appends_the_readback_reads_and_resources_four_readers() {
    let mut deltas: Vec<(VbRow, Vec<BufBarrier>, Vec<BufBarrier>)> = Vec::new();

    for (off_row, on_row) in [(S1, P1), (S3, P3)] {
        let f_off = declare_vb_frame(off_row);
        let f_on = declare_vb_frame(on_row);

        // The two frames must NUMBER their resources identically, or comparing their streams
        // element-wise compares different buffers. True by construction — every `add_image` /
        // `add_buffer` in `declare_vb_frame` runs unconditionally, before any pass — and checked
        // here so the construction cannot quietly stop holding.
        assert_eq!(
            (
                f_off.vb_indirect,
                f_off.vb_visible_instance,
                f_off.vb_cull_count,
                f_off.vb_cull_visible,
                f_off.vb_late_visible,
                f_off.vb_late_count,
                f_off.vb_indirect_late,
            ),
            (
                f_on.vb_indirect,
                f_on.vb_visible_instance,
                f_on.vb_cull_count,
                f_on.vb_cull_visible,
                f_on.vb_late_visible,
                f_on.vb_late_count,
                f_on.vb_indirect_late,
            ),
            "{} vs {}: the probe declares no RESOURCE, so the two rows must number them identically",
            off_row.id,
            on_row.id
        );

        // (1) The probe declares no IMAGE access, so not one image barrier may move — including the
        // ten-level pyramid chain and the depth round trip `P3` carries.
        assert_eq!(
            f_on.g.img_barriers(),
            f_off.g.img_barriers(),
            "{}: the readback probe moved an IMAGE barrier. It declares only buffer accesses, so \
             either the probe branch names an image or an image access was re-ordered behind it",
            on_row.id
        );

        // (2) The two passes and their POSITIONS, derived from the two declaration sites rather
        // than from the compiled result.
        let mut want_names = f_off.pass_names.clone();
        let raster = want_names
            .iter()
            .position(|n| *n == "vb_raster")
            .expect("invariant: every row declares the early raster");
        want_names.insert(raster, "vb_cull_readback");
        let raster_late = want_names
            .iter()
            .position(|n| *n == "vb_raster_late")
            .expect("invariant: a split row declares the late raster");
        want_names.insert(raster_late + 1, "vb_cull_readback_late");
        assert_eq!(
            f_on.pass_names, want_names,
            "{}: the probe must add EXACTLY `vb_cull_readback` immediately before `vb_raster` and \
             `vb_cull_readback_late` immediately after `vb_raster_late`. Both positions are \
             correctness: the first sits between the cull's stores and their readers, which is what \
             re-sources four barriers; the second sits AFTER the late raster, which is the only \
             reason the shipping `vb_indirect_late` chain does not move",
            on_row.id
        );

        // (3) The buffer delta, both directions, against the derivation.
        let (want_added, want_removed) = probe_delta_expectation(&f_on);
        let added = multiset_minus(f_on.g.buf_barriers(), f_off.g.buf_barriers());
        let removed = multiset_minus(f_off.g.buf_barriers(), f_on.g.buf_barriers());
        assert_eq!(
            added,
            want_added,
            "{}: the barriers the probe ADDS diverged from the derivation.\nDERIVED:\n{}\nCOMPILED:\n{}\n\
             Every entry is argued at `probe_delta_expectation`; a divergence is a real finding \
             about the declaration, not a baseline to re-measure.",
            on_row.id,
            describe_buf_list(&f_on, &want_added),
            describe_buf_list(&f_on, &added)
        );
        assert_eq!(
            removed,
            want_removed,
            "{}: the PROBE-OFF barriers the probe REPLACES diverged from the derivation.\n\
             DERIVED:\n{}\nCOMPILED:\n{}\n\
             An EMPTY list here would mean the probe perturbs nothing that was pinned, which \
             contradicts its position between the cull's stores and their readers.",
            on_row.id,
            describe_buf_list(&f_off, &want_removed),
            describe_buf_list(&f_off, &removed)
        );

        // (4) The shipping chain, field-identical, with the snapshot appended as a THIRD link.
        let chain_off = buf_on(f_off.g.buf_barriers(), f_off.vb_indirect_late);
        let chain_on = buf_on(f_on.g.buf_barriers(), f_on.vb_indirect_late);
        assert_eq!(
            (chain_off.len(), chain_on.len()),
            (2, 3),
            "{}: `vb_indirect_late` must route TWO barriers PROBE-OFF (the upload→cull WAW and the \
             cull→fetch RAW) and THREE PROBE-ON (plus the post-late snapshot's TRANSFER read). This \
             is why `assert_row_is_pinned`, whose split branch demands exactly two, is not called \
             on a probe row",
            on_row.id
        );
        assert_eq!(
            (*chain_on[0], *chain_on[1]),
            (*chain_off[0], *chain_off[1]),
            "{}: the SHIPPING chain `vb_indirect_late_upload → vb_cull_late → vb_raster_late` must \
             be FIELD-IDENTICAL with and without the probe. `graph_bridge.rs` argues exactly this \
             where it declares `vb_cull_readback_late` AFTER the late raster — sited between the \
             cull's write and the fetch it WOULD re-source the fetch. A divergence here means that \
             siting changed, and with it what the four `S*` pins certify",
            on_row.id
        );

        deltas.push((on_row, added, removed));
    }

    // (5) The delta is ROW-INDEPENDENT, which is the derivation's own prediction rather than an
    // observation: the probe declares only buffer accesses, and `S1`/`S3` differ only in image
    // accesses (the pyramid chain, the poison, the two depth copies). If this reds while (3) passes
    // on both rows, one of the probe's accesses has acquired an HZB-dependent gate.
    let [(row_a, added_a, removed_a), (row_b, added_b, removed_b)] = &deltas[..] else {
        panic!("invariant: the loop above pushes exactly two deltas");
    };
    assert_eq!(
        (added_a, removed_a),
        (added_b, removed_b),
        "{} and {} must derive the SAME buffer delta — the probe declares no image access, and \
         those two rows differ only in image accesses",
        row_a.id,
        row_b.id
    );
}
