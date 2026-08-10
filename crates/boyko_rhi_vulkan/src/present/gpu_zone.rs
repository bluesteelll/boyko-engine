//! Profiling rung 5a — the GPU zone recorder: a ring of query-pool slots, a per-pair witness, and
//! the 2×2 label that says what each pair's number is worth.
//!
//! # What this replaces, and why three collectors existed
//!
//! [`gpu_timing`](super::gpu_timing) holds **three** collectors — `TimestampCollector`,
//! `VbTimestampCollector` (deleted at rung 7), `Sv0TimestampCollector` — identical but for their
//! pass enum and pool
//! width. They are separate because their reader blocks: `VK_QUERY_RESULT_WAIT_BIT` makes
//! `vkGetQueryPoolResults` wait forever on any query its recorder never wrote, so every harness had
//! to arrange, in its own way, to only ever ask about pairs it *knew* were written. A vocabulary
//! per harness is what that arrangement looks like once it is code.
//!
//! Rung 4 removed the block ([`read_query_pool_pairs_available`]). This module is what the absence
//! of the block buys: one recorder, any pass, and *"the recorder never bracketed this"* as an
//! answer rather than a deadlock.
//!
//! [`read_query_pool_pairs_available`]: boyko_rhi::RhiDevice::read_query_pool_pairs_available
//!
//! # Availability is not the witness, and the difference is the whole design
//!
//! Availability answers *"the GPU wrote this query"*. It does **not** answer *"the recorder
//! bracketed this pass"* — a pass that never ran and a pass whose queries never came back are both
//! `available == 0` and mean opposite things.
//!
//! Nor can a duration tell them apart. The tree's own argument, from the collector this replaces:
//! a zero-filled pair *"reads ~0 like a genuinely free pass, and its begin offset is only *usually*
//! the frame's largest — a `TOP_OF_PIPE` stamp recorded last may legally report an EARLY time …,
//! so an offset-position rule is a heuristic, not a proof."* And on this box a genuinely empty
//! bracket does not even read zero — measured at 128 ticks at rung 4 and 96 at rung 5a, so the
//! hardware granularity is the floor under every duration here, whatever its exact step is.
//!
//! So the recorder keeps a host-side witness per pair — begun / ended, written where the `vkCmd*`
//! is — and the label is the 2×2 over (witness, availability):
//!
//! | begun | ended | available | label | meaning |
//! |---|---|---|---|---|
//! | 1 | 1 | yes | [`GpuLabel::Measured`] | a number |
//! | 0 | 0 | – | [`GpuLabel::NotBracketed`] | this leg does not run that pass |
//! | 1 | 0 | – | [`GpuLabel::Torn`] | a recorder bug |
//! | 1 | 1 | no | [`GpuLabel::Lost`] | bracketed, never came back — **no number** |
//!
//! # The witness is marks + ONE seal, not an atomic bitmask
//!
//! `AtomicU128` does not exist on stable or nightly, and a hand-rolled 128-bit atomic is
//! `cmpxchg16b` — a full read-modify-write, not the cheap `Release` store the ordering argument
//! needs. So the marks are plain bytes in an [`UnsafeCell`], written by exactly one thread (the
//! recorder), and the single [`AtomicU32`] `seal` is the one release edge: the recorder stores the
//! frame number after the marks, and retire loads it and compares. That scales to any pair count
//! with no bitmask width wall, and costs a plain byte store instead of an atomic OR per mark.
//!
//! # What rung 5a does NOT have
//!
//! No `CommandWitness` (rung 5b, behind `profiling-census`), no ported VB brackets and no
//! cross-collector A/B (rung 5c), and no write into the ECS `Profiler` — this module produces
//! labelled results and hands them to its caller. Each absent rather than present-and-inert, for
//! the reason the store's own rungs are staged that way: a value nothing can make move is
//! indistinguishable from a measurement of zero.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};

use boyko_rhi::{RhiDevice, TimestampStage};

use crate::device::{DeviceFns, VulkanContext};
use crate::error::VulkanError;
use crate::ffi::VkCommandBuffer;
use crate::rhi_impl::VulkanQueryPool;

/// Pairs one frame slot can bracket — 256 queries, Bevy's `QuerySet` size.
pub const MAX_GPU_PAIRS: usize = 128;

/// Frame slots in the ring.
///
/// **Strictly greater than `FRAMES_IN_FLIGHT = 2`**, which is what makes the host-reset fallback
/// free: a slot that retires without host query reset must wait for an armed frame to record a
/// `vkCmdResetQueryPool` for it, and with four slots against two in flight there is always a clean
/// one. The fallback costs recycle latency, never a stall.
pub const GPU_RING_DEPTH: usize = 4;

const _: () = assert!(
    GPU_RING_DEPTH > super::FRAMES_IN_FLIGHT,
    "a ring no deeper than the frames in flight makes the host-reset fallback a stall"
);

/// Frames a slot is given past its submit-epoch deadline before it is retired as incomplete.
pub const RETIRE_GRACE_FRAMES: u8 = 2;

/// The **second** deadline, counted in ECS frames rather than submit epochs.
///
/// Two horns, and they are independent because the failure modes are. The epoch horn is the tight
/// one in normal running. The frame horn is the one that fires when **submits freeze while frames
/// keep going** — the host loop `continue`s on a 0×0 client after the ECS update and before the
/// record, so a minimised window keeps folding and keeps serving readers while the render epoch
/// stands still. An epoch-only deadline can never fire there, and teardown is never reached because
/// the process is alive.
pub const GPU_FRAME_DEADLINE: u64 =
    GPU_RING_DEPTH as u64 + RETIRE_GRACE_FRAMES as u64 + 2;

/// Queries one slot's pool holds.
pub const QUERIES_PER_SLOT: u32 = (MAX_GPU_PAIRS * 2) as u32;

/// Zone ids reserved to one recorded pass FAMILY.
///
/// # Why bases exist at all, and why they are not the engine zone space
///
/// Rung 5c set the zone id of a ported VB bracket to its `VbTimedPass` slot: honest, because it
/// names the pass, and not a claim to be part of the engine-wide zone space (that space is minted by
/// the schedule, for systems, and these are not systems). With ONE family that is complete.
///
/// Rung 6 ports two more, and slot-alone stops naming a pass: `TimedPass::DdgiUpdate` and
/// `Sv0TimedPass::Marcher` are both slot 0 and **both can be recorded into one frame's slot** —
/// `record_gbuffer` holds the brackets for both. A base per family is the smallest thing that keeps
/// the id a name. It is const-asserted disjoint at the seam that uses it, next to the enum whose
/// width it must exceed, so a family that grows past [`ZONE_FAMILY_WIDTH`] is a **build failure**
/// rather than two families quietly sharing an id.
///
/// It is still not the engine zone space, and the distance is the point: these ids live in one
/// recorder, mean nothing outside it, and rung 7 deletes the enums they are derived from.
pub const ZONE_FAMILY_WIDTH: u16 = 16;

/// Base for the ten `record_vb` ids below. Named `VbTimedPass`-derived until rung 7 step 5 deleted
/// that enum; the derivation is now this base plus a literal offset per id.
pub const ZONE_BASE_VB: u16 = 0;
/// Base for `TimedPass`-derived ids — the four software-ray passes in `record_gbuffer`.
pub const ZONE_BASE_GBUFFER: u16 = ZONE_FAMILY_WIDTH;
/// Base for `Sv0TimedPass`-derived ids — the Deferred fine-marcher dispatch.
pub const ZONE_BASE_SV0: u16 = 2 * ZONE_FAMILY_WIDTH;

const _: () = assert!(
    ZONE_BASE_VB + ZONE_FAMILY_WIDTH <= ZONE_BASE_GBUFFER
        && ZONE_BASE_GBUFFER + ZONE_FAMILY_WIDTH <= ZONE_BASE_SV0,
    "zone family ranges overlap, so one id would name two passes"
);

// === The VB family's ten zone ids — profiling rung 7, step 5. ===
//
// These constants are what is LEFT of `VbTimedPass` after rung 7 deleted the collector that enum
// existed to index. The enum's per-variant documentation is migrated here rather than summarised,
// because it is the only written record of WHY each bracket sits where it sits — placement
// decisions taken under VB-P1e H0 and VG R3 piece 4 rung P4-2, each of which cost a measurement.
// What is NOT migrated is every claim about the readback: `VK_QUERY_RESULT_WAIT_BIT`, the pairs
// that had to be written whether or not the frame bracketed them, and the hang that made those
// fills mandatory. `GpuZoneRecorder` reads `WITH_AVAILABILITY` and labels an unwritten pair
// `NotBracketed`, so those sentences describe a hazard that no longer has a mechanism.
//
// They are plain `u16` and not a new enum ON PURPOSE. An enum brings back a hand-maintained
// slot→id table — the exact property D6 rejects — and `zone_begin_stage` below is already keyed by
// ZONE ID, so a second vocabulary would have to be kept in agreement with it. Here the id IS the
// name.

/// VB-P1e H0: the light-cull's alloc-counter `cmd_fill_buffer` plus its graph-derived
/// TRANSFER→COMPUTE barrier — the FIRST HALF of what VB-P1d bracketed as one `LightCull` pair.
///
/// Bracketed even on a frame where the froxel arm is not boot-built (`scene.cluster_cull.is_none()`),
/// reporting near-zero ns then. Exists to attribute §1.2's ~13.9 µs fixed cull cost (fill+barrier vs
/// dispatch ramp) instead of assuming it.
pub const ZONE_VB_CULL_RESET: u16 = ZONE_BASE_VB;
/// VB-P1e H0: the cull dispatch itself (`cluster_cull.comp.hlsl`) — the SECOND HALF of VB-P1d's
/// `LightCull` pair.
pub const ZONE_VB_CULL_DISPATCH: u16 = ZONE_BASE_VB + 1;
/// The `record_vb` lit-producer dispatch — whichever of the THREE mutually-exclusive producers this
/// frame selects: `vb_shade_split` (when `scene.path_vb_split()`, which DISPLACES both others), else
/// `vb_shade` (material-classified, when `scene.vb_use_classified`), else the fused `vb_resolve`.
///
/// Bracketed identically in all three branches — the same "derived barriers + bind + dispatch"
/// extent — so exactly one pair is opened per mesh-leg frame whichever branch runs. VB-P1d's
/// assertion survives as a SCOPE statement (its break-even number is defined against the fused /
/// classified tail), which is all it ever was once the split arm gained its own bracket.
pub const ZONE_VB_SHADE: u16 = ZONE_BASE_VB + 2;
/// VG R3 piece 4 rung P4-2: the LATE indirect-record fill — the host `vkCmdUpdateBuffer` chunks that
/// seed `vb_indirect_late` with `instanceCount = 0`, plus the pass's derived barriers.
///
/// The bracket sits OUTSIDE `if occlusion_split`, so a disarmed frame brackets a block that records
/// nothing and reports a near-zero MEASURED cost. Moving it inside would make the disarmed leg
/// report `FALLBACK` instead — the plan's control (ii), and the reason the placement is stated here
/// rather than left to the recorder's indentation.
pub const ZONE_VB_LATE_UPLOAD: u16 = ZONE_BASE_VB + 3;
/// VG R3 piece 4 rung P4-2: the EARLY batch-cull dispatch — `vb_batch_cull.comp` at `phase = EARLY`,
/// its derived barriers, its descriptor bind and (under `BOYKO_VB_CULL_READBACK`) its pre-snapshot
/// copies. Bracketed outside `if batch_cull_armed` for [`ZONE_VB_LATE_UPLOAD`]'s reason.
pub const ZONE_VB_EARLY_CULL: u16 = ZONE_BASE_VB + 4;
/// VG R3 piece 4 rung P4-2: the EARLY raster scope — `vb_raster`'s derived barriers, the
/// `cmd_begin_rendering`/`cmd_end_rendering` pair and every indirect draw between them. THE pass the
/// occlusion split exists to shrink, so `-Δ5` is the plan's `Saving` term.
pub const ZONE_VB_EARLY_RASTER: u16 = ZONE_BASE_VB + 5;
/// VG R3 piece 4 rung P4-2: the `[hzb_poison, hzb_build_*]` block — bracketed INSIDE
/// `record_hzb_poison_build`, at its first and last statements, so ONE bracket site serves both of
/// that function's mutually-exclusive call sites.
///
/// ⚠️ Its POSITION is leg-dependent (see [`VB_ZONE_COUNT`]'s leg table) and its magnitude is
/// therefore NOT comparable across an armed/disarmed pair. Bracketing inside the function rather
/// than at the call sites is what makes the witness a record of what executed instead of a caller's
/// prediction about a body it cannot see.
pub const ZONE_VB_HZB_BUILD: u16 = ZONE_BASE_VB + 6;
/// VG R3 piece 4 rung P4-2: the LATE batch-cull dispatch — the second `vb_batch_cull.comp` dispatch
/// at `phase = LATE`, reading the pyramid this frame's [`ZONE_VB_HZB_BUILD`] wrote. Bracketed
/// outside `if occlusion_split`.
pub const ZONE_VB_LATE_CULL: u16 = ZONE_BASE_VB + 7;
/// VG R3 piece 4 rung P4-2: the LATE raster scope — the second `begin/endRendering` bracket over the
/// same two views, drawing whatever `instanceCount` the late cull wrote. Bracketed outside
/// `if occlusion_split`, and closed AFTER the host-side probe counter that follows the scope, so the
/// pair covers the whole recorded unit rather than the scope alone.
pub const ZONE_VB_LATE_RASTER: u16 = ZONE_BASE_VB + 8;
/// VG R3 piece 4 rung P4-2: **the run bracket** — opens immediately before [`ZONE_VB_LATE_UPLOAD`]'s
/// begin and closes immediately after [`ZONE_VB_LATE_RASTER`]'s end.
///
/// THE headline interval, and the only aggregate that is migration-immune: all eight stamps
/// `b9 … e9` are `BOTTOM_OF_PIPE`, so the intervals between consecutive ones exactly PARTITION
/// `[t(b9), t(e9)]` — work that migrates between ids 3..8 is zero-sum inside it and cancels in a
/// paired difference of two structurally identical runs. Its span is identical on every leg (unlike
/// ids 2 and 6), which is why a record-order clause is scoped to it.
pub const ZONE_VB_RUN: u16 = ZONE_BASE_VB + 9;

/// How many zone ids the VB family uses — the count [`super::passes`]'s witness masks are sized by.
///
/// # Record order is LEG-DEPENDENT, and two ids are the ones that move
///
/// | leg | order of BEGIN stamps |
/// |---|---|
/// | armed split | `0 1` ‖ `9b 3 4 5 6 7 8 9e` ‖ `2` |
/// | disarmed | `0 1` ‖ `9b 3 4 5 7 8 9e` ‖ `2` ‖ `6` |
///
/// [`ZONE_VB_HZB_BUILD`] moves because `record_hzb_poison_build` has two mutually-exclusive call
/// sites on opposite sides of the lit producer; [`ZONE_VB_SHADE`] moves between its own three
/// producer arms. The `9b … 9e` span is identical on every leg.
///
/// This table is why `TsWitness::pair_of` REMEMBERS each id's pair index instead of deriving it
/// from the count of lower-numbered opens: the ids do not open in increasing order, so the
/// derivation gives `VbRun` pair 8 where it is 2.
pub const VB_ZONE_COUNT: u16 = 10;

// The per-frame witness masks in `passes::vb` are `u16`, one bit per id, so the family may not
// outgrow that width without widening them too — and it may not outgrow its base spacing either.
const _: () = assert!(
    VB_ZONE_COUNT <= 16 && VB_ZONE_COUNT <= ZONE_FAMILY_WIDTH,
    "the VB witness masks are u16 — one bit per zone id — and the family must fit its base spacing"
);

// === The gbuffer and SV0 families — profiling rung 7, step 6c. ===
//
// What is left of `TimedPass` and `Sv0TimedPass`, on the terms the VB block above records. Both
// families open at `TOP_OF_PIPE`, which is what their collectors' `write_begin` always did.

/// HW-RT rung R0: the DDGI probe-update dispatch.
pub const ZONE_GBUF_DDGI_UPDATE: u16 = ZONE_BASE_GBUFFER;
/// HW-RT rung R0: the deferred resolve dispatch, INCLUDING its inline SDF soft-shadow march — R0
/// brackets passes, not shader sections.
pub const ZONE_GBUF_DEFERRED_RESOLVE: u16 = ZONE_BASE_GBUFFER + 1;
/// HW-RT rung R0: the CSM cascade depth pass. Mesh-leg-owned, so a `Deferred × Sdf` frame never
/// records it — under the zone recorder that is a `NotBracketed` label rather than a hang.
pub const ZONE_GBUF_CSM_DEPTH: u16 = ZONE_BASE_GBUFFER + 2;
/// HW-RT rung R0: the punctual spot/point atlas depth pass. Mesh-leg-owned, like the cascade above.
pub const ZONE_GBUF_PUNCTUAL_DEPTH: u16 = ZONE_BASE_GBUFFER + 3;

/// How many zone ids the gbuffer family uses.
pub const GBUF_ZONE_COUNT: u16 = 4;

/// VB-SV0 rung S1.5: the Deferred fine-marcher dispatch (`sdf_gbuffer_composite.hlsl`).
///
/// Bracketed inside the recorder's `if let Some(marcher_pass)` arm, i.e. on exactly the frames the
/// dispatch is recorded. ⚠️ A render path that does not dispatch the marcher therefore leaves this
/// pair UNWRITTEN. That used to be a caller precondition, because the collector's readback waited on
/// it forever; the zone recorder labels it and moves on, which is what the port bought.
pub const ZONE_SV0_MARCHER: u16 = ZONE_BASE_SV0;

/// How many zone ids the SV0 family uses.
pub const SV0_ZONE_COUNT: u16 = 1;

// One assert per family and not a conjunction: the widths are independent, so an `&&` would let the
// wider one imply the narrower and clippy is right to call the second conjunct dead.
const _: () = assert!(
    GBUF_ZONE_COUNT <= ZONE_FAMILY_WIDTH,
    "the gbuffer family no longer fits its reserved zone-id range"
);
const _: () = assert!(
    SV0_ZONE_COUNT <= ZONE_FAMILY_WIDTH,
    "the SV0 family no longer fits its reserved zone-id range"
);

/// One past the highest zone id any family can mint — the width a zone-KEYED array needs.
///
/// Three families at [`ZONE_FAMILY_WIDTH`] apiece. Derived rather than written, so a fourth family
/// widens every zone-keyed array by changing one base.
pub const ZONE_ID_SPAN: usize = (ZONE_BASE_SV0 + ZONE_FAMILY_WIDTH) as usize;

const _: () = assert!(
    ZONE_ID_SPAN > (ZONE_BASE_SV0 + SV0_ZONE_COUNT - 1) as usize,
    "a zone-keyed array sized by ZONE_ID_SPAN must hold the highest id any family mints"
);

/// **The BEGIN stage of a zone, keyed by ZONE ID.** Rung 7's home for the table that lived on
/// `VbTimedPass::begin_stage`.
///
/// The enum is deleted with its collector, and `ZONE_BASE_VB + slot` is the only vocabulary left
/// afterwards — so the table has to move BEFORE the deletion, not be re-derived after it. Rung 7c
/// is why: the port carried the brackets and not their stages, and seven VB passes measured a
/// different quantity under a green `G10` for five commits. Re-deriving this under deadline
/// pressure is that failure's exact shape.
///
/// `BOTTOM_OF_PIPE` for the seven P4-2 partitioning brackets (`ZONE_BASE_VB + 3 ..= +9`), because a
/// bottom stamp is a prefix-completion time and consecutive ones exactly partition their span.
/// `TOP_OF_PIPE` everywhere else: VB slots 0..2 keep it for compatibility with published VB-P1d
/// numbers, and both gbuffer families' collectors always opened there.
///
/// # What gates this table now — stated, because the answer changed
///
/// While `VbTimedPass` existed, `G10`'s stage clause WAS the gate: leg A read the enum, leg B read
/// this, and the two were compared stamp for stamp on every steady frame — 26 frames, 520
/// timestamps, all identical. Rung 7 step 5 deleted leg A, so that comparison has no second side
/// and the gate is gone with it. Nothing measures this table against an independent copy any more,
/// **because there is no independent copy left to measure against.**
///
/// What replaces it is weaker and is named as weaker: the `const` block below pins each of the ten
/// VB ids to the stage it had when both tables agreed. That catches a row edited by hand; it cannot
/// catch a bracket moved to a site where the other stage is the right one. That question is a
/// measurement, and after this rung it is rung 8's.
#[must_use]
pub const fn zone_begin_stage(zone: u16) -> TimestampStage {
    // Written against the NAMED ids rather than `3..=9`: the range is the P4-2 partitioning span,
    // and spelling it as two endpoints that move with the constants is what keeps it that span when
    // the family grows. No `>= ZONE_BASE_VB` guard is possible — that base is 0, so the comparison
    // is tautological on a `u16` and clippy refuses it; the other families' bases (gbuffer's 16,
    // SV0's 32) fall outside this range on their own.
    if matches!(zone, ZONE_VB_LATE_UPLOAD..=ZONE_VB_RUN) {
        TimestampStage::BottomOfPipe
    } else {
        TimestampStage::TopOfPipe
    }
}

// The stage each VB id carried while `VbTimedPass::begin_stage` was the second, independently
// written copy of this table and `G10` compared the two stamp for stamp. Pinned id by id rather
// than as a range so that a row cannot be changed without changing the line that states it.
const _: () = {
    const fn tops(zone: u16) -> bool {
        matches!(zone_begin_stage(zone), TimestampStage::TopOfPipe)
    }
    const fn bottoms(zone: u16) -> bool {
        matches!(zone_begin_stage(zone), TimestampStage::BottomOfPipe)
    }
    assert!(
        tops(ZONE_VB_CULL_RESET)
            && tops(ZONE_VB_CULL_DISPATCH)
            && tops(ZONE_VB_SHADE)
            && bottoms(ZONE_VB_LATE_UPLOAD)
            && bottoms(ZONE_VB_EARLY_CULL)
            && bottoms(ZONE_VB_EARLY_RASTER)
            && bottoms(ZONE_VB_HZB_BUILD)
            && bottoms(ZONE_VB_LATE_CULL)
            && bottoms(ZONE_VB_LATE_RASTER)
            && bottoms(ZONE_VB_RUN),
        "a VB zone's BEGIN stage differs from the one G10 measured while both tables existed"
    );
    // The other two families opened at TOP on both legs, per their collectors' `write_begin`.
    assert!(
        tops(ZONE_BASE_GBUFFER) && tops(ZONE_BASE_SV0),
        "the gbuffer and SV0 families open at TOP_OF_PIPE"
    );
};

/// The witness bit set when a pair's BEGIN timestamp is recorded.
const MARK_BEGUN: u8 = 1 << 0;
/// The witness bit set when a pair's END timestamp is recorded.
const MARK_ENDED: u8 = 1 << 1;

/// A slot's `seal` value before the recorder has sealed it.
///
/// `u32::MAX` rather than `0`, because `0` is a real frame number — the first one. A sentinel that
/// collides with a legal value makes "never sealed" and "sealed at frame 0" the same state.
const SEAL_UNSEALED: u32 = u32::MAX;

/// What a retired pair's numbers are worth.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GpuLabel {
    /// Bracketed and available. `begin_ticks` / `dur_ticks` are a measurement.
    Measured = 0,
    /// The recorder never bracketed this pair — this leg does not run that pass. **Not** an error,
    /// and the reason the witness exists: a duration cannot say this.
    #[default]
    NotBracketed = 1,
    /// Begun and never ended. A recorder bug, counted rather than printed per pair.
    Torn = 2,
    /// Bracketed, but the queries never became available before the deadline. **No number**, and
    /// the state the blocking design could not express — it hung instead.
    Lost = 3,
}

/// One retired pair.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PairResult {
    /// The zone the recorder opened this pair for. Meaningless when the label is
    /// [`GpuLabel::NotBracketed`].
    pub zone: u16,
    /// What the two numbers below are worth.
    pub label: GpuLabel,
    /// The GPU clock at the pair's BEGIN stamp, masked to the device's valid bits. Zero for any
    /// label other than [`GpuLabel::Measured`].
    pub begin_ticks: u64,
    /// The pair's duration in GPU ticks. Zero for any label other than [`GpuLabel::Measured`].
    pub dur_ticks: u64,
}

/// Why a slot retired.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RetireCause {
    /// Every bracketed pair came back. The frame is complete.
    Complete,
    /// The submit-epoch deadline fired: the GPU has moved `FRAMES_IN_FLIGHT` submits past this
    /// slot's, and the grace frames are spent.
    EpochDeadline,
    /// The frame deadline fired: submits froze while frames kept going.
    FrameDeadline,
    /// A host-side flush at teardown.
    Flushed,
}

/// One retired frame slot, as its consumer sees it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RetiredFrame {
    /// The frame number the recorder opened this slot for.
    pub frame: u32,
    /// Pairs the recorder allocated. `results[..pairs]` are the ones to read.
    pub pairs: u16,
    /// Why it retired.
    pub cause: RetireCause,
    /// Pairs labelled [`GpuLabel::Lost`] — bracketed and never returned.
    pub lost: u16,
    /// Pairs labelled [`GpuLabel::Torn`] — begun and never ended.
    pub torn: u16,
}

/// The caller-owned working set [`GpuZoneRecorder::retire`] and [`GpuZoneRecorder::flush`] read
/// and write.
///
/// One struct rather than five parameters, and it is not only tidiness: these five buffers are
/// **one object with one lifetime** — the host allocates it once beside the recorder and reuses it
/// every frame, because a retire that allocated would be a profiler allocating on the frame path.
/// Passing them separately invited a caller to hand in five buffers of five different lengths, and
/// the verb's length contract would then be five preconditions instead of one type.
///
/// ~9.3 KiB. Hold ONE per recorder; do not build it on the stack per call.
pub struct RetireScratch {
    /// Staging for the non-blocking read: value + availability per query, two queries per pair.
    raw: [u64; MAX_GPU_PAIRS * 4],
    begin_ticks: [u64; MAX_GPU_PAIRS],
    dur_ticks: [u64; MAX_GPU_PAIRS],
    available: [u8; MAX_GPU_PAIRS],
    results: [PairResult; MAX_GPU_PAIRS],
}

impl Default for RetireScratch {
    fn default() -> RetireScratch {
        RetireScratch::new()
    }
}

impl RetireScratch {
    /// A zeroed working set.
    #[must_use]
    pub fn new() -> RetireScratch {
        RetireScratch {
            raw: [0; MAX_GPU_PAIRS * 4],
            begin_ticks: [0; MAX_GPU_PAIRS],
            dur_ticks: [0; MAX_GPU_PAIRS],
            available: [0; MAX_GPU_PAIRS],
            results: [PairResult::default(); MAX_GPU_PAIRS],
        }
    }
}

/// One frame's worth of bracketing state.
///
/// `#[repr(C)]` because the marks/seal adjacency is the ordering argument's subject, not an
/// incidental layout.
#[repr(C)]
struct FrameSlot {
    /// Per-pair witness bits. Plain bytes in a cell: exactly one thread writes them, and the
    /// [`Self::seal`] store is the release edge that publishes all of them at once.
    marks: UnsafeCell<[u8; MAX_GPU_PAIRS]>,
    /// Pair → zone id, written beside the mark by the same single thread and published by the
    /// same seal.
    zone_of: UnsafeCell<[u16; MAX_GPU_PAIRS]>,
    /// **The one release edge.** Holds the frame number once the marks are complete, and
    /// [`SEAL_UNSEALED`] otherwise.
    seal: AtomicU32,
    /// The bump allocator. Atomic because pairs are allocated during recording, through `&self`.
    used_pairs: AtomicU16,
    /// The frame this slot is recording.
    frame: u32,
    /// The render epoch at record time — horn 1's input.
    submit_epoch: u64,
    /// The ECS frame counter at record time — horn 2's input.
    record_frame: u64,
    /// Grace frames left after the epoch deadline first fires.
    grace: u8,
    /// Set when the slot retired without a host query reset, so its pool still holds the previous
    /// frame's results. Slot recycling refuses this slot until an armed frame records a
    /// `vkCmdResetQueryPool` for it.
    ///
    /// Atomic for one reason: **every recording verb takes `&self`**, because recording mutates GPU
    /// query memory rather than this struct, and the reset is a recording verb like the others. A
    /// `bool` here would have made `record_reset` the single `&mut self` exception, and a caller
    /// that holds the recorder shared for a frame's recording could not then call it at the frame
    /// top — which is the one place it belongs.
    needs_cmd_reset: AtomicBool,
    /// Whether this slot is recording or awaiting retire.
    in_flight: bool,
}

impl FrameSlot {
    const fn new() -> FrameSlot {
        FrameSlot {
            marks: UnsafeCell::new([0u8; MAX_GPU_PAIRS]),
            zone_of: UnsafeCell::new([0u16; MAX_GPU_PAIRS]),
            seal: AtomicU32::new(SEAL_UNSEALED),
            used_pairs: AtomicU16::new(0),
            frame: 0,
            submit_epoch: 0,
            record_frame: 0,
            grace: RETIRE_GRACE_FRAMES,
            needs_cmd_reset: AtomicBool::new(false),
            in_flight: false,
        }
    }
}

// SAFETY (`Sync` for `FrameSlot`, whose `UnsafeCell` fields make it `!Sync` by default):
//   (a) SINGLE PRODUCER. `marks` and `zone_of` are written ONLY by `alloc_pair` / `mark_begun` /
//       `mark_ended`, which the recorder calls from the one thread recording that frame's command
//       buffer. The engine records a frame on a single thread; two threads recording one slot
//       would be a recorder bug the witness itself would then report as `Torn`.
//   (b) ONE RELEASE EDGE. Every read of those cells is in `retire`, and it happens only after
//       `seal.load(Acquire)` observed the frame number that `seal.store(Release)` published after
//       the last mark write. That pairing is what makes the plain byte stores visible, and it is
//       why the seal is the ONLY atomic here.
//   (c) EXCLUSIVITY AT RETIRE. `retire` takes `&mut self` on the recorder, so no recording call
//       can be in flight against the same slot while it reads.
unsafe impl Sync for FrameSlot {}

/// The GPU zone recorder: `GPU_RING_DEPTH` slots, each with its own query pool.
pub struct GpuZoneRecorder {
    pools: [VulkanQueryPool; GPU_RING_DEPTH],
    slots: [FrameSlot; GPU_RING_DEPTH],
    /// The slot the next `open_frame` will try.
    next: usize,
}

impl GpuZoneRecorder {
    /// Takes ownership of one query pool per slot. Each must hold at least [`QUERIES_PER_SLOT`]
    /// queries.
    #[must_use]
    pub fn new(pools: [VulkanQueryPool; GPU_RING_DEPTH]) -> GpuZoneRecorder {
        GpuZoneRecorder {
            pools,
            slots: core::array::from_fn(|_| FrameSlot::new()),
            next: 0,
        }
    }

    /// The pool backing `slot`.
    #[must_use]
    pub fn pool(&self, slot: usize) -> &VulkanQueryPool {
        &self.pools[slot]
    }

    /// Pairs `slot` has allocated so far.
    #[must_use]
    pub fn used_pairs(&self, slot: usize) -> u16 {
        self.slots[slot].used_pairs.load(Ordering::Relaxed)
    }

    /// Whether `slot` is awaiting retire.
    #[must_use]
    pub fn in_flight(&self, slot: usize) -> bool {
        self.slots[slot].in_flight
    }

    /// Whether `slot`'s pool still needs a recorded reset before it can be reused.
    #[must_use]
    pub fn needs_cmd_reset(&self, slot: usize) -> bool {
        self.slots[slot].needs_cmd_reset.load(Ordering::Relaxed)
    }

    /// Claim a slot for `frame`, or `None` when every slot is still in flight.
    ///
    /// `None` is a real outcome, not an error: it means the GPU is more than `GPU_RING_DEPTH`
    /// frames behind, and the honest response is to record no zones this frame rather than to
    /// overwrite a slot whose results have not been read.
    pub fn open_frame(
        &mut self,
        frame: u32,
        submit_epoch: u64,
        record_frame: u64,
    ) -> Option<usize> {
        for step in 0..GPU_RING_DEPTH {
            let idx = (self.next + step) % GPU_RING_DEPTH;
            if self.slots[idx].in_flight || self.needs_cmd_reset(idx) {
                continue;
            }
            let slot = &mut self.slots[idx];
            slot.frame = frame;
            slot.submit_epoch = submit_epoch;
            slot.record_frame = record_frame;
            slot.grace = RETIRE_GRACE_FRAMES;
            slot.in_flight = true;
            slot.used_pairs.store(0, Ordering::Relaxed);
            slot.seal.store(SEAL_UNSEALED, Ordering::Relaxed);
            // The marks must start clean, or a pair this frame never allocates would inherit the
            // previous frame's `begun` bit and retire as `Torn` — a recorder bug reported against
            // a recorder that did nothing.
            //
            // SAFETY: `&mut self` is exclusive, so no recording call can be reading these cells.
            unsafe {
                (*slot.marks.get()).fill(0);
            }
            self.next = (idx + 1) % GPU_RING_DEPTH;
            return Some(idx);
        }
        None
    }

    /// Allocate one pair in `slot` for `zone`, or `None` when the slot is full.
    ///
    /// Takes `&self`: recording runs against a shared borrow, exactly as the collector this
    /// replaces does, because writing a timestamp mutates GPU query memory rather than this
    /// struct.
    pub fn alloc_pair(&self, slot: usize, zone: u16) -> Option<u16> {
        let s = &self.slots[slot];
        let pair = s.used_pairs.fetch_add(1, Ordering::Relaxed);
        if pair as usize >= MAX_GPU_PAIRS {
            // Undo, so a slot that overflows once does not keep climbing and wrap the counter.
            s.used_pairs.store(MAX_GPU_PAIRS as u16, Ordering::Relaxed);
            return None;
        }
        // SAFETY: single producer (clause (a) of the `Sync` impl); `pair < MAX_GPU_PAIRS`, tested
        //   above. Published by the `seal` store, read only after the matching `Acquire`.
        unsafe {
            (*s.zone_of.get())[pair as usize] = zone;
        }
        Some(pair)
    }

    /// Record `slot`'s pool reset at the frame top.
    ///
    /// # Safety
    ///
    /// `cmd` must be a live command buffer in the recording state, outside any render or
    /// dynamic-rendering scope (`VUID-vkCmdResetQueryPool-renderpass`), and `fns` must be this
    /// device's function table.
    pub unsafe fn record_reset(&self, fns: &DeviceFns, cmd: VkCommandBuffer, slot: usize) {
        let pool = &self.pools[slot];
        // SAFETY: the caller's contract, plus `pool.pool` is a live pool of `QUERIES_PER_SLOT`
        //   queries created on this device.
        unsafe {
            (fns.cmd_reset_query_pool)(cmd, pool.pool, 0, QUERIES_PER_SLOT);
        }
        // The recorded reset is what clears the fallback flag: after this command executes the
        // pool is clean, so the slot may be claimed again.
        self.slots[slot].needs_cmd_reset.store(false, Ordering::Relaxed);
    }

    /// Record `pair`'s BEGIN stamp at `stage` and witness it. **Returns the stage it wrote at**,
    /// so the caller's census records what this function did rather than what the caller expected.
    ///
    /// # The recorder does not choose the stage, and rung 7c is why
    ///
    /// This hardcoded [`TimestampStage::TopOfPipe`] until rung 7c. The generic reading is
    /// defensible — [`TimestampStage`]'s own doc says a bracket *"opens at `TopOfPipe`"* — but it
    /// is not universal, and the brackets rung 5c ported through here are the counterexample:
    /// `VbTimedPass::begin_stage` opens the seven P4-2 passes at `BOTTOM_OF_PIPE` precisely so that
    /// consecutive stamps are prefix-completion times and their intervals **partition** the run.
    /// A `TOP_OF_PIPE` open destroys that property, silently, while changing no command count and
    /// no stream position — so `G10` stayed green across it for five commits.
    ///
    /// The stage is a property of the BRACKET, and the bracket belongs to the caller. A recorder
    /// that picks one is a recorder that can be wrong about a pass it has never heard of.
    ///
    /// # Safety
    ///
    /// As [`Self::record_reset`], and `pair` must have come from [`Self::alloc_pair`] on this
    /// slot.
    pub unsafe fn record_begin(
        &self,
        fns: &DeviceFns,
        cmd: VkCommandBuffer,
        slot: usize,
        pair: u16,
        stage: TimestampStage,
    ) -> TimestampStage {
        // SAFETY: caller contract.
        unsafe {
            self.write_timestamp(fns, cmd, slot, u32::from(pair) * 2, stage);
            self.set_mark(slot, pair, MARK_BEGUN);
        }
        stage
    }

    /// Record `pair`'s END stamp and witness it. **Returns the stage it wrote at.**
    ///
    /// `BOTTOM_OF_PIPE` is not a caller choice here, unlike [`Self::record_begin`]'s: a close that
    /// fired before the bracketed work retired would measure a prefix of it, and no pass in this
    /// tree closes at any other stage. Returned rather than assumed so the census witnesses it from
    /// the same place both legs do.
    ///
    /// # Safety
    ///
    /// As [`Self::record_begin`].
    pub unsafe fn record_end(
        &self,
        fns: &DeviceFns,
        cmd: VkCommandBuffer,
        slot: usize,
        pair: u16,
    ) -> TimestampStage {
        // SAFETY: caller contract.
        unsafe {
            self.write_timestamp(
                fns,
                cmd,
                slot,
                u32::from(pair) * 2 + 1,
                TimestampStage::BottomOfPipe,
            );
            self.set_mark(slot, pair, MARK_ENDED);
        }
        TimestampStage::BottomOfPipe
    }

    /// Publish `slot`'s marks. **Called once, after the last bracket of the frame is recorded.**
    ///
    /// This is the release edge the whole witness rests on: every plain byte store above becomes
    /// visible to `retire`'s `Acquire` load through this one `Release` store.
    pub fn seal(&self, slot: usize) {
        let s = &self.slots[slot];
        s.seal.store(s.frame, Ordering::Release);
    }

    /// The one `vkCmdWriteTimestamp` site.
    ///
    /// # Safety
    ///
    /// `cmd` live and recording; `query < QUERIES_PER_SLOT`.
    unsafe fn write_timestamp(
        &self,
        fns: &DeviceFns,
        cmd: VkCommandBuffer,
        slot: usize,
        query: u32,
        stage: TimestampStage,
    ) {
        debug_assert!(query < QUERIES_PER_SLOT, "invariant: a query index fits the slot's pool");
        let vk_stage = match stage {
            TimestampStage::TopOfPipe => crate::ffi::VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
            TimestampStage::BottomOfPipe => crate::ffi::VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
        };
        // SAFETY: caller contract; `pool.pool` is a live TIMESTAMP pool on this device and
        //   `query` is inside it (asserted above).
        unsafe {
            (fns.cmd_write_timestamp)(cmd, vk_stage, self.pools[slot].pool, query);
        }
    }

    /// Set one witness bit.
    ///
    /// # Safety
    ///
    /// Single producer — see clause (a) of the `Sync` impl.
    unsafe fn set_mark(&self, slot: usize, pair: u16, bit: u8) {
        debug_assert!((pair as usize) < MAX_GPU_PAIRS, "invariant: a pair index fits the slot");
        // SAFETY: caller contract, plus the bound asserted above. The store is plain because the
        //   `seal` store publishes it.
        unsafe {
            (*self.slots[slot].marks.get())[pair as usize] |= bit;
        }
    }

    /// Retire every slot that can, calling `sink` once per retired slot.
    ///
    /// # The two horns, and the decrement that used to wrap
    ///
    /// Horn 1 is the submit epoch: once the host has observed `submit_epoch + FRAMES_IN_FLIGHT`,
    /// this slot's last possible submit is GPU-complete by the ring's own fence discipline. Horn 2
    /// is the frame counter, for the case where submits freeze and frames do not.
    ///
    /// **The grace decrement lives INSIDE horn 1's arm and is guarded.** An earlier form read
    /// `else if epoch_ok && grace == 0 { retire } else { grace -= 1 }`, so a slot whose epoch
    /// condition was *false* with `grace` already 0 executed `0u8 - 1`: a debug panic, or in
    /// release a wrap to 255 that silently restarts the deadline for another 255 frames.
    ///
    /// `scratch` must hold `4 * pairs` words and the three out slices `pairs` each — the rung-4
    /// verb's contract.
    pub fn retire(
        &mut self,
        device: &VulkanContext,
        render_epoch: u64,
        frame_now: u64,
        scratch: &mut RetireScratch,
        mut sink: impl FnMut(RetiredFrame, &[PairResult]),
    ) -> Result<(), VulkanError> {
        for idx in 0..GPU_RING_DEPTH {
            if !self.slots[idx].in_flight {
                continue;
            }
            let pairs = self.slots[idx].used_pairs.load(Ordering::Relaxed).min(MAX_GPU_PAIRS as u16);
            if pairs == 0 {
                // A slot that bracketed nothing has nothing to poll and nothing to report. It is
                // released rather than held to a deadline it can never meet.
                self.close_slot(device, idx);
                continue;
            }

            device.read_query_pool_pairs_available(
                &self.pools[idx],
                u32::from(pairs),
                &mut scratch.raw,
                &mut scratch.begin_ticks,
                &mut scratch.dur_ticks,
                &mut scratch.available,
            )?;

            let all_bracketed_available = self.every_bracketed_pair_available(idx, pairs, &scratch.available);
            let cause = if all_bracketed_available {
                RetireCause::Complete
            } else if render_epoch >= self.slots[idx].submit_epoch + super::FRAMES_IN_FLIGHT as u64 {
                // HORN 1. The decrement is here, guarded, and `continue`s without retiring.
                if self.slots[idx].grace > 0 {
                    self.slots[idx].grace -= 1;
                    continue;
                }
                RetireCause::EpochDeadline
            } else if frame_now.saturating_sub(self.slots[idx].record_frame) > GPU_FRAME_DEADLINE {
                // HORN 2, independent of horn 1 by construction.
                RetireCause::FrameDeadline
            } else {
                continue;
            };

            let retired = self.label_slot(idx, pairs, cause, scratch);
            sink(retired, &scratch.results[..pairs as usize]);
            self.close_slot(device, idx);
        }
        Ok(())
    }

    /// Whether every pair the recorder actually bracketed came back.
    ///
    /// Reads the marks under the seal: an unsealed slot is treated as having no witness, so
    /// nothing is claimed bracketed and the slot retires on a deadline rather than pretending to
    /// be complete.
    fn every_bracketed_pair_available(&self, idx: usize, pairs: u16, available: &[u8]) -> bool {
        if self.slots[idx].seal.load(Ordering::Acquire) != self.slots[idx].frame {
            return false;
        }
        // SAFETY: the `Acquire` load above observed the recorder's `Release` seal, so every mark
        //   write happens-before this read; `&mut self` at the call site excludes a concurrent
        //   recorder (clause (c)).
        let marks = unsafe { &*self.slots[idx].marks.get() };
        for pair in 0..pairs as usize {
            let bracketed = marks[pair] & (MARK_BEGUN | MARK_ENDED) == MARK_BEGUN | MARK_ENDED;
            if bracketed && available[pair] == 0 {
                return false;
            }
        }
        true
    }

    /// Apply the 2×2 label to every pair of `idx`.
    fn label_slot(
        &self,
        idx: usize,
        pairs: u16,
        cause: RetireCause,
        scratch: &mut RetireScratch,
    ) -> RetiredFrame {
        let sealed = self.slots[idx].seal.load(Ordering::Acquire) == self.slots[idx].frame;
        // An unsealed slot has no witness at all, so every pair reads `NOT_BRACKETED` rather than
        // borrowing marks the recorder may still be writing. That is the conservative direction:
        // it declines to report numbers, where the other direction would report them unlabelled.
        let zeroed_marks = [0u8; MAX_GPU_PAIRS];
        let zeroed_zones = [0u16; MAX_GPU_PAIRS];
        // SAFETY: `sealed` means the `Acquire` load above paired with the recorder's `Release`
        //   store, so the marks and zones are fully published; `&mut self` at the call site
        //   excludes a concurrent recorder.
        let (marks, zones) = if sealed {
            unsafe { (&*self.slots[idx].marks.get(), &*self.slots[idx].zone_of.get()) }
        } else {
            (&zeroed_marks, &zeroed_zones)
        };

        let (mut lost, mut torn) = (0u16, 0u16);
        for pair in 0..pairs as usize {
            let m = marks[pair];
            let begun = m & MARK_BEGUN != 0;
            let ended = m & MARK_ENDED != 0;
            let avail = scratch.available[pair] != 0;
            let label = match (begun, ended, avail) {
                (true, true, true) => GpuLabel::Measured,
                (true, true, false) => {
                    lost += 1;
                    GpuLabel::Lost
                }
                (true, false, _) => {
                    torn += 1;
                    GpuLabel::Torn
                }
                (false, _, _) => GpuLabel::NotBracketed,
            };
            scratch.results[pair] = PairResult {
                zone: zones[pair],
                label,
                begin_ticks: if label == GpuLabel::Measured { scratch.begin_ticks[pair] } else { 0 },
                dur_ticks: if label == GpuLabel::Measured { scratch.dur_ticks[pair] } else { 0 },
            };
        }

        RetiredFrame { frame: self.slots[idx].frame, pairs, cause, lost, torn }
    }

    /// Release a slot, resetting its pool on the host when the device allows it.
    fn close_slot(&mut self, device: &VulkanContext, idx: usize) {
        self.slots[idx].in_flight = false;
        self.slots[idx].seal.store(SEAL_UNSEALED, Ordering::Relaxed);
        if device.host_query_reset_supported()
            && device.reset_query_pool_host(&self.pools[idx], 0, QUERIES_PER_SLOT).is_ok()
        {
            self.slots[idx].needs_cmd_reset.store(false, Ordering::Relaxed);
        } else {
            // The fully specified fallback: the slot is unavailable until an armed frame records a
            // `vkCmdResetQueryPool` for it. With `GPU_RING_DEPTH > FRAMES_IN_FLIGHT` there is
            // always a clean slot, so this costs one frame of recycle latency and never a stall.
            self.slots[idx].needs_cmd_reset.store(true, Ordering::Relaxed);
        }
    }

    /// Force-retire every in-flight slot as incomplete — the teardown path.
    ///
    /// Frames stop at shutdown and on device-lost, so neither horn can ever fire; without this the
    /// last `GPU_RING_DEPTH` slots would be dropped silently, which is the loss a profiler exists
    /// to report rather than to commit.
    pub fn flush(
        &mut self,
        device: &VulkanContext,
        scratch: &mut RetireScratch,
        mut sink: impl FnMut(RetiredFrame, &[PairResult]),
    ) -> Result<(), VulkanError> {
        for idx in 0..GPU_RING_DEPTH {
            if !self.slots[idx].in_flight {
                continue;
            }
            let pairs = self.slots[idx].used_pairs.load(Ordering::Relaxed).min(MAX_GPU_PAIRS as u16);
            if pairs == 0 {
                self.close_slot(device, idx);
                continue;
            }
            device.read_query_pool_pairs_available(
                &self.pools[idx],
                u32::from(pairs),
                &mut scratch.raw,
                &mut scratch.begin_ticks,
                &mut scratch.dur_ticks,
                &mut scratch.available,
            )?;
            let retired = self.label_slot(idx, pairs, RetireCause::Flushed, scratch);
            sink(retired, &scratch.results[..pairs as usize]);
            self.close_slot(device, idx);
        }
        Ok(())
    }

    /// Hand the pools back for destruction. The recorder owns them; this is how a caller gets them
    /// out without a `Drop` that would need a device reference it does not hold.
    #[must_use]
    pub fn into_pools(self) -> [VulkanQueryPool; GPU_RING_DEPTH] {
        self.pools
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 2×2 is a table, and a table can be asserted without a GPU. Every one of the four rows,
    /// plus the two the corpus does not spell (`(0,1,*)` — ended without begun) which fall to
    /// `NOT_BRACKETED` because `begun` is the witness's first question.
    #[test]
    fn the_label_table_is_the_one_the_corpus_states() {
        let rows: [(bool, bool, bool, GpuLabel); 6] = [
            (true, true, true, GpuLabel::Measured),
            (false, false, false, GpuLabel::NotBracketed),
            (false, false, true, GpuLabel::NotBracketed),
            (true, false, true, GpuLabel::Torn),
            (true, false, false, GpuLabel::Torn),
            (true, true, false, GpuLabel::Lost),
        ];
        for (begun, ended, avail, want) in rows {
            let got = match (begun, ended, avail) {
                (true, true, true) => GpuLabel::Measured,
                (true, true, false) => GpuLabel::Lost,
                (true, false, _) => GpuLabel::Torn,
                (false, _, _) => GpuLabel::NotBracketed,
            };
            assert_eq!(got, want, "({begun}, {ended}, {avail}) mislabelled");
        }
    }

    /// The default label is `NOT_BRACKETED`, not `MEASURED`.
    ///
    /// A zeroed `PairResult` is what a caller's uninitialised buffer holds, and the difference
    /// between the two defaults is the difference between "no claim" and "a measurement of zero" —
    /// which this campaign has already paid for once.
    #[test]
    fn a_zeroed_result_claims_nothing() {
        let r = PairResult::default();
        assert_eq!(r.label, GpuLabel::NotBracketed);
        assert_eq!(r.dur_ticks, 0);
        assert_eq!(r.begin_ticks, 0);
    }

    /// The frame deadline is the corpus's arithmetic, not a round number chosen here.
    #[test]
    fn the_frame_deadline_is_derived_from_the_ring_and_the_grace() {
        assert_eq!(GPU_FRAME_DEADLINE, 8);
        assert_eq!(QUERIES_PER_SLOT, 256);
        // The ring-depth relation is already a `const _: () = assert!(...)` at module scope — a
        // BUILD failure, which is strictly stronger than a test. Restating it here as a runtime
        // assertion would be a second statement of one fact, and clippy is right to say so.
    }

    /// `SEAL_UNSEALED` must not collide with a legal frame number — and frame 0 is legal.
    #[test]
    fn the_unsealed_sentinel_is_not_a_frame_number_a_session_reaches() {
        assert_eq!(SEAL_UNSEALED, u32::MAX);
        assert_ne!(SEAL_UNSEALED, 0, "frame 0 is the FIRST frame, so 0 cannot mean 'never sealed'");
    }
}
