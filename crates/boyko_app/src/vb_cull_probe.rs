//! **VG rung R2d-5 / VG R3 piece 3 step P3-5 — the armed cull readback**
//! (`BOYKO_VB_CULL_READBACK=<path.txt>`).
//!
//! Two things live here, and they are the two halves of one seam:
//!
//! * `VbCullProbe` — the settle → request → drain driver the frame loop threads through its
//!   steady path, a sibling of `crate::hzb_dump::HzbDump` and sharing its two constants;
//! * [`format_vb_cull_probe_line`] — the ONE serializer of a decoded
//!   `crate::gpu_scene::VbCullReadback`, and therefore the only channel from the cull's device
//!   buffers to a test process.
//!
//! (The driver and everything it names are `#[cfg(windows)]`, like their siblings — its only caller
//! is the windowed frame loop — so they are referenced here in code spans rather than intra-doc
//! links, which would not resolve on a docs build for another target. The line format is not gated:
//! it has no device and no OS in it, which is also why its gate needs no GPU.)
//!
//! # Why the driver replaced a bare early return, and what that fixed
//!
//! Until step P3-5 the readback ran on the FIRST presented frame and `return`ed out of the frame
//! loop from inside its own branch. Three things followed, and all three were defects:
//!
//! 1. **Frame 1 is not a converged frame.** The pyramid's boot clear makes it all-zeros at birth, so
//!    a cull reading it on frame 1 provably defers nothing. A fixture capturing that frame would
//!    compare a cull that did nothing against a cull that was off, get agreement, and prove nothing.
//! 2. **The `return` was outside the exit conjunction**, so arming `BOYKO_VB_CULL_READBACK` beside
//!    `BOYKO_HZB_DUMP` exited the process at frame 1: the cull file was written and the pyramid file
//!    **never**. The two captures could not be produced by one process, which is exactly what the
//!    pairing check needs.
//! 3. **The payload had no frame identity at all.** Nothing on the probe line said which frame it
//!    came from, so "the two files describe the same frame" was unstatable.
//!
//! The driver below is `crate::hzb_dump::HzbDump`'s shape, counting the SAME
//! `hzb_dump::SETTLE_FRAMES` and `hzb_dump::DRAIN_FRAMES` — shared constants, not two literals that
//! agree, because the pairing check is a claim about the two probes landing on ONE frame.
//!
//! # The capture is ONE frame, and that is what the request flag buys
//!
//! The staging is boot-owned and per-FIF, and the copies are recorded on exactly the frames whose
//! `GBufferScene::vb_cull_readback` is `Some`. The driver hands that arming out on the request frame
//! alone, so the drained slot still holds the request frame's bytes when it is mapped
//! `DRAIN_FRAMES` presents later — with the copies running every frame, the same slot would have
//! been rewritten twice by then and the drained read would describe a frame the header does not
//! name.
//!
//! The drain is what makes the read safe, and it replaces the `wait_idle` the old block used:
//! `DRAIN_FRAMES (3) > FRAMES_IN_FLIGHT (2)` presented frames after the capture, the frame loop has
//! necessarily re-waited the capture slot's fence, which is the same argument the pyramid dump's own
//! drain rests on.

#[cfg(windows)]
use core::num::NonZeroU32;

#[cfg(windows)]
use crate::hzb_dump::{DRAIN_FRAMES, SETTLE_FRAMES};

/// The settle → request → drain progression. `Settle`/`Drain` count REMAINING presented frames;
/// `Request` keeps re-requesting across `Ok(false)` recreate-skips until a request frame presents.
#[cfg(windows)]
enum ProbeState {
    Settle(u32),
    Request,
    Drain(NonZeroU32),
}

/// The cull-readback driver the frame loop threads through its steady path.
#[cfg(windows)]
pub(crate) struct VbCullProbe {
    /// Destination path (the `BOYKO_VB_CULL_READBACK` value), written once. May be EMPTY: the
    /// variable's presence is what `GpuSceneBundles::boot` keys the staging on, so an empty value
    /// still arms the capture and simply prints without writing a file.
    path: String,
    state: ProbeState,
    /// The frame-in-flight slot the request frame used — the slot [`Self::finish`]'s caller decodes,
    /// which is NOT the slot the drained frame is running on.
    request_slot: usize,
    /// The ENGINE frame index the request frame carried (the runner's monotonic per-iteration
    /// counter, the same clock `VbCullUniform::frame_index` is stamped from).
    request_frame_index: u32,
    /// The CAPTURE frame's live `DrawBatch` count, latched because the drained decode happens after
    /// three further gathers.
    request_batch_count: usize,
    /// The CAPTURE frame's per-batch `base_instance` list, latched for the same reason. The bases
    /// live only on the host — the GPU sees them inside a descriptor the probe does not copy — so
    /// without them the payload could only be printed as a flat prefix.
    request_bases: Vec<u32>,
}

#[cfg(windows)]
impl VbCullProbe {
    /// Arms the probe iff `BOYKO_VB_CULL_READBACK` is set (the value is the output path). Cold:
    /// called once before the frame loop.
    ///
    /// The predicate is `is_ok()`, NOT "set and non-empty", because `GpuSceneBundles::boot` creates
    /// the staging on `is_ok()` — a driver with a narrower gate would leave a boot that allocated
    /// the staging with no capture to end the run, i.e. a process that never exits.
    pub(crate) fn from_env() -> Option<Self> {
        let path = std::env::var("BOYKO_VB_CULL_READBACK").ok()?;
        boyko_log::info!(
            boyko_log::Host,
            "BOYKO_VB_CULL_READBACK armed -> {}",
            boyko_log::dsp!(path, 192)
        );
        Some(Self {
            path,
            state: ProbeState::Settle(SETTLE_FRAMES),
            request_slot: 0,
            request_frame_index: 0,
            request_batch_count: 0,
            request_bases: Vec::new(),
        })
    }

    /// The per-frame capture request: `true` on the request frame(s), `false` otherwise.
    ///
    /// `slot` is this frame's frame-in-flight index and `frame_index` its engine frame index; both
    /// are LATCHED here rather than re-read at the drain, because by then the loop is several frames
    /// past the capture and holds neither.
    pub(crate) fn request(&mut self, slot: usize, frame_index: u32) -> bool {
        if !matches!(self.state, ProbeState::Request) {
            return false;
        }
        self.request_slot = slot;
        self.request_frame_index = frame_index;
        true
    }

    /// Advances the settle → request → drain machine after a frame attempt (`presented == true` iff
    /// `render_gbuffer_frame` returned `Ok(true)`). Returns `true` when the drained capture is
    /// host-readable — the caller then decodes [`Self::capture`]'s slot and runs [`Self::finish`].
    pub(crate) fn after_present(&mut self, presented: bool) -> bool {
        if !presented {
            return false;
        }
        match self.state {
            ProbeState::Settle(n) => {
                self.state = if n > 1 { ProbeState::Settle(n - 1) } else { ProbeState::Request };
                false
            }
            ProbeState::Request => {
                self.state = ProbeState::Drain(
                    NonZeroU32::new(DRAIN_FRAMES).expect("invariant: DRAIN_FRAMES > 0"),
                );
                false
            }
            ProbeState::Drain(n) => match NonZeroU32::new(n.get() - 1) {
                Some(left) => {
                    self.state = ProbeState::Drain(left);
                    false
                }
                None => true,
            },
        }
    }

    /// Latches the CAPTURE frame's host-side draw-list shape.
    ///
    /// Called on a request frame only, AFTER the render, because the gather's `base_instance`
    /// prefix sum is what the survivor regions are addressed by and it is not recoverable from the
    /// device payload. A recreate-skip re-enters `Request` next iteration and overwrites this, so
    /// what survives is always the batches of the frame that actually presented the copies.
    pub(crate) fn latch_batches(&mut self, batch_count: usize, bases: Vec<u32>) {
        debug_assert!(
            matches!(self.state, ProbeState::Request),
            "invariant: only a request frame latches its batches"
        );
        self.request_batch_count = batch_count;
        self.request_bases = bases;
    }

    /// `(frame-in-flight slot, engine frame index)` of the CAPTURE frame — the pair
    /// `GpuSceneBundles::read_vb_cull` takes.
    pub(crate) fn capture(&self) -> (usize, u32) {
        (self.request_slot, self.request_frame_index)
    }

    /// The CAPTURE frame's live `DrawBatch` count.
    pub(crate) fn batch_count(&self) -> usize {
        self.request_batch_count
    }

    /// The CAPTURE frame's per-batch `base_instance` list.
    pub(crate) fn bases(&self) -> &[u32] {
        &self.request_bases
    }

    /// Prints the probe line and writes it to the armed path, consuming the driver (the capture is
    /// one-shot).
    ///
    /// The file is written before the caller's exit so the run's termination is unambiguous evidence
    /// that it is complete. An EMPTY path prints and writes nothing — the same `BOYKO_HOST_DUMP`
    /// shape the previous block used, kept because a run that arms the variable with no value still
    /// allocates the staging and must still terminate.
    pub(crate) fn finish(self, line: &str) {
        // The FILE is the contract — every consumer of this probe reads `self.path`, and none has
        // ever parsed this line off stdout (measured across `crates/*/tests` and `scripts/` at
        // L8b: zero readers). So the record is a convenience for a human watching the run, and
        // bounding it at 1536 B costs that reader nothing the file does not still hold in full.
        // `MAX_RECORD_BYTES` is 2048 including the header and the tag bytes, so a larger bound
        // would make a long batch list REFUSE the record outright instead of truncating it.
        boyko_log::info!(boyko_log::Profiling, "{}", boyko_log::dsp!(line, 1536));
        if self.path.is_empty() {
            return;
        }
        std::fs::write(&self.path, line)
            .expect("invariant: the cull-readback probe must be able to write its path");
    }
}

/// **VG rung R2d-5 / VG R3 piece 3 step P3-5: the cull-probe line's fields**, as plain borrowed
/// slices.
///
/// This module is a TEST SEAM (`#[doc(hidden)] pub` at the `lib.rs` declaration):
/// `format_vb_cull_probe_line` is the only emitter of this line and
/// `vb_inst_cull_scene::parse_probe_line` (in `tests/`) the only reader, so the round-trip gate has
/// to be able to call the real emitter. Taking slices rather than `&VbCullReadback` is what keeps
/// that seam from dragging the crate-private readback type and its nine device regions into the
/// public API.
#[derive(Clone, Copy, Debug)]
pub struct VbCullProbeFields<'a> {
    /// Live `DrawBatch` records this frame (`scene.mesh_draw.len()`).
    pub drawn_batches: usize,
    /// The HOST's `base_instance` per drawn batch, in batch order.
    pub bases: &'a [u32],
    /// GPU counter: batches that passed the level-1 AABB test AND carry at least one survivor.
    pub visible_batches: u32,
    /// GPU `vb_cull_visible` — the compacted visible-BATCH indices.
    pub batch_list: &'a [u32],
    /// GPU `vb_indirect` word 1 per record — the post-cull `instanceCount` the raster FETCHES.
    pub record_instance_counts: &'a [u32],
    /// GPU `vb_visible_instance` — the per-INSTANCE survivor list.
    pub visible_instances: &'a [u32],
    /// PRE-late `vb_late_visible` — the CANDIDATE list.
    pub late_candidates: &'a [u32],
    /// PRE-late `vb_late_count` — `n_defer` per batch.
    pub late_count_pre: &'a [u32],
    /// POST-late `vb_late_visible` — the compacted survivor prefix followed by the candidate tail.
    pub late_survivors: &'a [u32],
    /// POST-late `vb_late_count` — the no-clobber clause's second side.
    pub late_count_post: &'a [u32],
    /// POST-late `vb_indirect_late` word 1 per record — `n_keep` per batch.
    pub late_record_instance_counts: &'a [u32],
    /// The ENGINE frame this capture came from.
    pub frame_index: u32,
    /// The frame index the GPU read out of `VbCullUniform`.
    pub gpu_observed_frame_index: u32,
}

/// Comma-joins a `u32` slice.
fn join(v: &[u32]) -> String {
    v.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

/// Renders `count` per-batch groups as `base:members`, pipe-separated.
///
/// # Why this is PER BATCH, and why each group carries its own base
///
/// Batch `b` owns exactly `[base(b), base(b) + len(b))` of `data` and writes nowhere else
/// (`vb_batch_cull.comp.hlsl`'s INVARIANT R2d-REGION-DISJOINT for the early list, VG-P3-LATE-REGION
/// for the late one). **The regions need NOT be contiguous**: the frame loop skips batches whose
/// mesh asset is not `Loaded`, so `scene.mesh_draw` is a SUBSEQUENCE of the gather's list and
/// consecutive bases can leave gaps. A flat prefix of the buffer would therefore interleave real
/// entries with slots no batch owns — which is why each group carries its own base rather than being
/// POSITIONED by it, and why a reader can reconstruct the regions from the line alone.
///
/// `lens` is the per-batch LENGTH source and is deliberately a parameter: the early list is sized by
/// `vb_indirect[b].instanceCount`, the candidate list by `late_count_pre[b]` and the survivor list by
/// `vb_indirect_late[b].instanceCount`. Three different numbers, one grouping rule.
///
/// Both ends are clamped to `data.len()`, so a length word the GPU never wrote (or one a corruption
/// made absurd) truncates the printed group instead of panicking inside a diagnostic.
fn per_batch_groups(bases: &[u32], lens: &[u32], data: &[u32], count: usize) -> String {
    let mut groups: Vec<String> = Vec::with_capacity(count);
    for (b, &base) in bases.iter().take(count).enumerate() {
        let lo = (base as usize).min(data.len());
        let len = lens.get(b).copied().unwrap_or(0) as usize;
        let hi = lo.saturating_add(len).min(data.len());
        groups.push(format!("{base}:{}", join(&data[lo..hi])));
    }
    groups.join("|")
}

/// **VG rung R2d-5 / VG R3 piece 3 step P3-5: the cull-probe line.**
///
/// | field | source | meaning |
/// |---|---|---|
/// | `batches` | host | live `DrawBatch` records this frame (`scene.mesh_draw.len()`) |
/// | `visible` | GPU counter | batches that passed the level-1 AABB test AND carry at least one survivor |
/// | `frame` | host | the ENGINE frame this capture came from (the settle → request → drain request frame) |
/// | `gpu_frame` | GPU `vb_late_count` tail | the frame index the CULL read out of `VbCullUniform` — plan D6's control F-M4a |
/// | `list` | GPU `vb_cull_visible` | the compacted visible-BATCH indices, `visible` of them |
/// | `inst` | GPU `vb_indirect` | word 1 of each record — the post-cull `instanceCount` the rasterizer FETCHES, one per drawn batch, in batch order |
/// | `vis` | GPU `vb_visible_instance` | `base:members` groups, pipe-separated, one per drawn batch |
/// | `late_cnt_pre` | GPU `vb_late_count` (PRE) | `n_defer` per drawn batch — the DEFERRAL count the early phase wrote |
/// | `late_cnt_post` | GPU `vb_late_count` (POST) | the same words re-read after the late cull; a difference is a clobber |
/// | `late_ic` | GPU `vb_indirect_late` (POST) | word 1 of each late record — `n_keep`, whose only producer is the late cull |
/// | `late_cand` | GPU `vb_late_visible` (PRE) | `base:members` groups sized by `late_cnt_pre` — the CANDIDATE list |
/// | `late_surv` | GPU `vb_late_visible` (POST) | `base:members` groups sized by `late_ic` — the compacted SURVIVORS |
///
/// # The two late instance lists are the same allocation at two TIMES
///
/// The late phase compacts `vb_late_visible` in place, so after it the region holds
/// `kept[0..n_keep)` followed by the ORIGINAL entries at `[n_keep, n_defer)` — a multiset that is
/// **not** the candidate set. `late_cand` is therefore the only observation of the candidate domain,
/// and it exists because the PRE snapshot is copied before `vb_cull_late` runs. Sizing `late_surv`
/// by `late_ic` rather than by `late_cnt_pre` is what stops the untouched tail from being printed as
/// if it had survived.
///
/// # Every list is sized by a number, and the numbers are not interchangeable
///
/// `vis` by `inst`, `late_cand` by `late_cnt_pre`, `late_surv` by `late_ic`. Using one for another
/// is precisely the grouping bug the round-trip gate exists to red on, so they are passed to
/// [`per_batch_groups`] explicitly rather than defaulted.
///
/// The line's length is a property of the SCENE (batches and their instance counts), never of the
/// allocation: nothing here iterates a capacity.
///
/// `#[cold]`/`#[inline(never)]`: a once-per-process diagnostic, never on the hot path.
#[cold]
#[inline(never)]
pub fn format_vb_cull_probe_line(f: &VbCullProbeFields<'_>) -> String {
    // The counter can EXCEED the list's capacity — the shader counts a dropped entry so a trimmed
    // list is detectable — so the printable prefix is the minimum of the two.
    let listed = (f.visible_batches as usize).min(f.batch_list.len());
    let list = join(&f.batch_list[..listed]);

    // One record per DRAWN batch. Each count array spans the whole allocation, so the frame's own
    // batch count is what bounds it.
    let recorded = f.drawn_batches.min(f.record_instance_counts.len());
    let inst = join(&f.record_instance_counts[..recorded]);
    let vis = per_batch_groups(f.bases, f.record_instance_counts, f.visible_instances, recorded);

    let late_recorded = f.drawn_batches.min(f.late_record_instance_counts.len());
    let late_ic = join(&f.late_record_instance_counts[..late_recorded]);
    let cnt_pre = join(&f.late_count_pre[..f.drawn_batches.min(f.late_count_pre.len())]);
    let cnt_post = join(&f.late_count_post[..f.drawn_batches.min(f.late_count_post.len())]);
    let late_cand = per_batch_groups(f.bases, f.late_count_pre, f.late_candidates, recorded);
    let late_surv =
        per_batch_groups(f.bases, f.late_record_instance_counts, f.late_survivors, late_recorded);

    let batches = f.drawn_batches;
    let visible = f.visible_batches;
    let frame = f.frame_index;
    let gpu_frame = f.gpu_observed_frame_index;
    format!(
        "VB_CULL_READBACK batches={batches} visible={visible} frame={frame} gpu_frame={gpu_frame} \
         list=[{list}] inst=[{inst}] vis=[{vis}] late_cnt_pre=[{cnt_pre}] \
         late_cnt_post=[{cnt_post}] late_ic=[{late_ic}] late_cand=[{late_cand}] \
         late_surv=[{late_surv}]"
    )
}

#[cfg(test)]
mod tests {
    use super::{VbCullProbeFields, format_vb_cull_probe_line, per_batch_groups};

    /// A payload whose per-batch regions are RAGGED and NON-CONTIGUOUS, which is the shape the
    /// grouping rule exists for.
    ///
    /// Three drawn batches at bases `0`, `4`, `9` — gaps at `[2,4)` and `[7,9)`, exactly what a
    /// frame that skipped a batch whose mesh is not `Loaded` produces. Slots `2,3` and `7,8` hold
    /// `9xx` values NO batch owns; a formatter that laid the groups out contiguously would print
    /// them.
    fn ragged() -> ([u32; 3], [u32; 3], [u32; 12]) {
        let bases = [0u32, 4, 9];
        let lens = [2u32, 3, 1];
        // 0,1 | (2,3 owned by nobody) | 4,5,6 | (7,8 owned by nobody) | 9
        let data = [0u32, 1, 900, 901, 4, 5, 6, 902, 903, 9, 904, 905];
        (bases, lens, data)
    }

    #[test]
    fn groups_carry_their_own_base_and_skip_the_slots_no_batch_owns() {
        let (bases, lens, data) = ragged();
        assert_eq!(per_batch_groups(&bases, &lens, &data, 3), "0:0,1|4:4,5,6|9:9");
    }

    /// The control that makes the test above able to fail: a formatter that assumed the regions
    /// were CONTIGUOUS — laying batch `b` out at the running sum of the previous lengths instead of
    /// at its own base — would print the slots no batch owns. Spelled here so the difference is a
    /// measured one rather than an asserted intention.
    #[test]
    fn a_contiguous_layout_would_print_different_bytes() {
        let (bases, lens, data) = ragged();
        let mut running = 0usize;
        let mut naive: Vec<String> = Vec::new();
        for (b, &base) in bases.iter().enumerate() {
            let len = lens[b] as usize;
            let members: Vec<String> =
                data[running..running + len].iter().map(u32::to_string).collect();
            naive.push(format!("{base}:{}", members.join(",")));
            running += len;
        }
        assert_eq!(
            naive.join("|"),
            "0:0,1|4:900,901,4|9:5",
            "the naive layout's own output, pinned so the disagreement below is measured"
        );
        assert_ne!(
            per_batch_groups(&bases, &lens, &data, 3),
            naive.join("|"),
            "a contiguous-region assumption must produce DIFFERENT bytes on a ragged payload -- if \
             these agreed, the round-trip gate could not red on a grouping bug"
        );
    }

    #[test]
    fn an_empty_group_is_a_base_with_no_members() {
        let bases = [0u32, 4];
        let lens = [0u32, 2];
        let data = [7u32, 8, 9, 10, 11, 12];
        assert_eq!(per_batch_groups(&bases, &lens, &data, 2), "0:|4:11,12");
    }

    #[test]
    fn a_length_past_the_end_truncates_rather_than_panics() {
        let bases = [0u32];
        let lens = [u32::MAX];
        let data = [1u32, 2, 3];
        assert_eq!(per_batch_groups(&bases, &lens, &data, 1), "0:1,2,3");
    }

    /// A base past the end yields an EMPTY group rather than a panic — a diagnostic must not be the
    /// thing that ends the process it is diagnosing.
    #[test]
    fn a_base_past_the_end_yields_an_empty_group() {
        let bases = [99u32];
        let lens = [4u32];
        let data = [1u32, 2, 3];
        assert_eq!(per_batch_groups(&bases, &lens, &data, 1), "99:");
    }

    // ── VG rung R2d-5's PER-BATCH decode tests, moved verbatim from `runner.rs` at step P3-5 when
    //    the formatter did. They exist because no GPU gate in that rung can pin the decode: on
    //    every shipped fixture the survivor list is the identity and the bases are contiguous, so a
    //    correct per-batch decode and the forbidden flat-prefix decode emit the SAME string — the
    //    narrow fixture prints `0:0,1,2|3:3,4,5` either way. The distinguishing input is a scene
    //    with NON-CONTIGUOUS bases and UNEQUAL per-batch counts, which the runtime produces
    //    whenever a batch's mesh asset is not `Loaded` (the gather still assigned it a region;
    //    `scene.mesh_draw` skips it), and which only a synthetic case can exercise here. ──

    /// Bases with a HOLE between them and counts that differ, so every wrong decode is visible:
    /// batch 0 owns `[0, 2)`, batch 1 owns `[5, 8)`, and slots 2..5 belong to a batch that was
    /// gathered but not drawn.
    ///
    /// The late fields carry the payload an UNSPLIT frame produces — zero counts and an untouched
    /// list — so these three cases stay statements about the early decode alone. That is also what
    /// every committed `BOYKO_VB_CULL_READBACK` fixture produces, since none of them marks
    /// `OcclusionCulling`.
    fn readback_line(vis: &[u32]) -> String {
        format_vb_cull_probe_line(&VbCullProbeFields {
            drawn_batches: 2,
            bases: &[0, 5],
            visible_batches: 2,
            batch_list: &[0, 1, 0, 0],
            record_instance_counts: &[2, 3, 0, 0],
            visible_instances: vis,
            late_candidates: &[0; 10],
            late_count_pre: &[0, 0, 0, 0],
            late_survivors: &[0; 10],
            late_count_post: &[0, 0, 0, 0],
            late_record_instance_counts: &[0, 0, 0, 0],
            frame_index: 31,
            gpu_observed_frame_index: 0,
        })
    }

    const READBACK_VIS: [u32; 10] = [10, 11, 900, 901, 902, 20, 21, 22, 903, 904];

    #[test]
    fn each_group_is_read_from_its_own_base_not_from_a_running_cursor() {
        let line = readback_line(&READBACK_VIS);
        assert!(
            line.contains("vis=[0:10,11|5:20,21,22]"),
            "the decode must select [base, base+count) per batch. A flat prefix would print \
             `0:10,11|5:900,901,902` (a running cursor) or splice the hole's contents into the \
             second group. Got: {line}"
        );
        assert!(
            !line.contains("900") && !line.contains("904"),
            "slots owned by no drawn batch must never appear: {line}"
        );
    }

    #[test]
    fn a_group_is_bounded_by_its_own_record_word_not_by_the_next_base() {
        // Batch 1's count is 3 while the gap to the end of the list is 5. Bounding by the next
        // base (or by the allocation) would print two extra slots the rasterizer never reads.
        let line = readback_line(&READBACK_VIS);
        let group = line.split('|').nth(1).expect("invariant: two groups are printed");
        assert!(group.starts_with("5:20,21,22]"), "group must stop at base+count: {group}");
    }

    #[test]
    fn a_truncated_region_clamps_silently_rather_than_panicking() {
        // A truncated VIS region (staging too small) leaves fewer slots than the bases name.
        let line = readback_line(&READBACK_VIS[..1]);
        assert!(line.contains("vis=[0:10|5:]"), "clamping must be silent and total: {line}");
    }

    /// The whole line, pinned byte for byte on a payload where **every one of the three grouped
    /// lists is sized by a DIFFERENT number** — `vis` by `inst`, `late_cand` by `late_cnt_pre`,
    /// `late_surv` by `late_ic`. Swapping any two of those sizers changes this string.
    #[test]
    fn the_line_pins_all_twelve_keys() {
        let bases = [0u32, 4];
        let inst = [2u32, 3];
        let vis = [10u32, 11, 900, 901, 14, 15, 16];
        let cnt_pre = [3u32, 1, 0];
        let cnt_post = [3u32, 1, 0];
        let late_ic = [1u32, 0];
        let cand = [20u32, 21, 22, 902, 24, 903, 904];
        let surv = [30u32, 905, 906, 907, 34, 908, 909];
        let line = format_vb_cull_probe_line(&VbCullProbeFields {
            drawn_batches: 2,
            bases: &bases,
            visible_batches: 2,
            batch_list: &[0, 1, 777],
            record_instance_counts: &inst,
            visible_instances: &vis,
            late_candidates: &cand,
            late_count_pre: &cnt_pre,
            late_survivors: &surv,
            late_count_post: &cnt_post,
            late_record_instance_counts: &late_ic,
            frame_index: 31,
            gpu_observed_frame_index: 31,
        });
        assert_eq!(
            line,
            "VB_CULL_READBACK batches=2 visible=2 frame=31 gpu_frame=31 list=[0,1] inst=[2,3] \
             vis=[0:10,11|4:14,15,16] late_cnt_pre=[3,1] late_cnt_post=[3,1] late_ic=[1,0] \
             late_cand=[0:20,21,22|4:24] late_surv=[0:30|4:]"
        );
    }

    /// `visible` may EXCEED the list's capacity (the shader counts a dropped entry so a trimmed list
    /// is detectable), and the printed prefix must be the minimum of the two rather than a slice
    /// that panics.
    #[test]
    fn a_counter_past_the_list_capacity_prints_the_whole_list() {
        let bases = [0u32];
        let line = format_vb_cull_probe_line(&VbCullProbeFields {
            drawn_batches: 1,
            bases: &bases,
            visible_batches: 9,
            batch_list: &[0, 1],
            record_instance_counts: &[1],
            visible_instances: &[5],
            late_candidates: &[],
            late_count_pre: &[0],
            late_survivors: &[],
            late_count_post: &[0],
            late_record_instance_counts: &[0],
            frame_index: 3,
            gpu_observed_frame_index: 0,
        });
        assert!(line.contains("visible=9 "), "the COUNTER is reported verbatim: {line}");
        assert!(line.contains("list=[0,1] "), "and the LIST is clamped to what exists: {line}");
    }
}
