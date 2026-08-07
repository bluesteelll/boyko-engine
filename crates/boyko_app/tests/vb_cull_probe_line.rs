//! **VG R3 piece 3 step P3-5 — the cull-probe line's format → parse ROUND TRIP.**
//!
//! `boyko_app::vb_cull_probe::format_vb_cull_probe_line` is the ONLY channel from the cull's device
//! buffers to a test process, and `vb_inst_cull_scene::parse_probe_line` is the ONLY reader of what
//! it writes. Every clause of every GPU gate downstream — the corpus counts, and piece 3's
//! candidate/survivor adjudication — is a statement about numbers that made that crossing intact.
//!
//! This file boots no device and needs none. It is the one gate that can go RED on a grouping bug.
//!
//! # Why it exists: the defect it is the third attempt to stop
//!
//! Plan round 1 shipped `VbRecordProbe::late_cull_dispatches` — a probe field with no serializer.
//! Round 2 fixed that on `vb_probe_dump.rs` and reproduced it here: the two readback snapshots were
//! decoded into `VbCullReadback` and dropped, because the line format was never widened. A field
//! that reaches the host and not the file is a measurement nobody can assert on, and the failure is
//! silent — the reader's `field()` panics naming the MISSING KEY, which reads like a parser bug
//! rather than an emitter that never wrote it.
//!
//! # Both halves are the shipped ones
//!
//! The emitter is called across the crate boundary through `boyko_app`'s `#[doc(hidden)] pub`
//! seam, and the parser is the same module the four `vb_inst_cull_*` gates use. Re-implementing
//! either here would make this a test of a copy — the tautology shape this campaign has shipped as
//! a gate before.
//!
//! # What it CANNOT claim
//!
//! Nothing about the GPU, the cull, the regions the device actually wrote, or whether the decode in
//! `GpuSceneBundles::read_vb_cull` addresses the right bytes. It claims exactly this: for a payload
//! the emitter is handed, the parser recovers it. A wrong REGION would round-trip perfectly.
//!
//! `#![cfg(windows)]` matches every other consumer of the shared fixture module, which pulls in the
//! windowed host types. It is NOT `#[ignore]`d: no device is touched, so it runs in every sweep.

#![cfg(windows)]

use boyko_app::vb_cull_probe::{VbCullProbeFields, format_vb_cull_probe_line};

mod vb_inst_cull_scene;

use vb_inst_cull_scene::parse_probe_line;

/// A whole probe payload in host form — the round trip's SOURCE of truth, so each case states its
/// expectation once and the assertions below compare against that one statement.
struct Payload {
    drawn_batches: usize,
    bases: Vec<u32>,
    visible_batches: u32,
    batch_list: Vec<u32>,
    record_instance_counts: Vec<u32>,
    visible_instances: Vec<u32>,
    late_candidates: Vec<u32>,
    late_count_pre: Vec<u32>,
    late_survivors: Vec<u32>,
    late_count_post: Vec<u32>,
    late_record_instance_counts: Vec<u32>,
    frame_index: u32,
    gpu_observed_frame_index: u32,
}

impl Payload {
    fn format(&self) -> String {
        format_vb_cull_probe_line(&VbCullProbeFields {
            drawn_batches: self.drawn_batches,
            bases: &self.bases,
            visible_batches: self.visible_batches,
            batch_list: &self.batch_list,
            record_instance_counts: &self.record_instance_counts,
            visible_instances: &self.visible_instances,
            late_candidates: &self.late_candidates,
            late_count_pre: &self.late_count_pre,
            late_survivors: &self.late_survivors,
            late_count_post: &self.late_count_post,
            late_record_instance_counts: &self.late_record_instance_counts,
            frame_index: self.frame_index,
            gpu_observed_frame_index: self.gpu_observed_frame_index,
        })
    }

    /// The per-batch groups this payload MEANS, computed here by slicing each region at its own
    /// base with its own length source — independently of the emitter, which is what makes the
    /// comparison a round trip rather than a restatement.
    fn expected_groups(&self, lens: &[u32], data: &[u32], count: usize) -> Vec<(u32, Vec<u32>)> {
        (0..count)
            .map(|b| {
                let base = self.bases[b];
                let lo = base as usize;
                let hi = lo + lens[b] as usize;
                (base, data[lo..hi].to_vec())
            })
            .collect()
    }
}

/// **THE CASE THE GATE EXISTS FOR: ragged, NON-CONTIGUOUS regions with an empty group.**
///
/// Three drawn batches at bases `0`, `5`, `11`. The gaps at `[3, 5)` and `[8, 11)` are what a frame
/// that skipped a batch whose mesh asset is not `Loaded` produces — `scene.mesh_draw` is a
/// SUBSEQUENCE of the gather's list, so consecutive bases leave holes. The slots inside those holes
/// carry deliberately distinctive values (`9xx`): an emitter that laid the groups out contiguously
/// would print them, and every assertion below would fail naming the number it printed.
///
/// The three grouped lists are sized by THREE DIFFERENT numbers — `vis` by `inst`, `late_cand` by
/// `late_cnt_pre`, `late_surv` by `late_ic` — and none of the three per-batch triples agree, so
/// swapping any two sizers changes the output.
///
/// Batch 2's `late_ic` is `0`: an empty group, which must round-trip as a base with no members
/// rather than vanishing (a vanished group would shift every later group's identity by one).
fn ragged_payload() -> Payload {
    Payload {
        drawn_batches: 3,
        bases: vec![0, 5, 11],
        visible_batches: 3,
        batch_list: vec![0, 1, 2],
        //             b0: 3      b1: 3        b2: 2
        record_instance_counts: vec![3, 3, 2],
        visible_instances: vec![
            0, 1, 2, // batch 0 @ base 0
            900, 901, // owned by nobody
            5, 6, 7, // batch 1 @ base 5
            902, 903, 904, // owned by nobody
            11, 12, // batch 2 @ base 11
        ],
        //             b0: 2      b1: 3        b2: 1
        late_count_pre: vec![2, 3, 1],
        late_count_post: vec![2, 3, 1],
        late_candidates: vec![
            20, 21, // batch 0's 2 candidates @ base 0
            910, 911, 912, // owned by nobody
            25, 26, 27, // batch 1's 3 candidates @ base 5
            913, 914, 915, // owned by nobody
            31, 932, // batch 2's 1 candidate @ base 11 (the second is the untouched tail)
        ],
        //             b0: 1      b1: 2        b2: 0
        late_record_instance_counts: vec![1, 2, 0],
        late_survivors: vec![
            21, 940, // batch 0 kept 1; slot 1 is the untouched candidate tail
            941, 942, 943, // owned by nobody
            26, 27, 944, // batch 1 kept 2
            945, 946, 947, // owned by nobody
            948, 949, // batch 2 kept 0 — the whole region is tail
        ],
        frame_index: 31,
        gpu_observed_frame_index: 31,
    }
}

#[test]
fn a_ragged_non_contiguous_payload_round_trips_through_every_key() {
    let p = ragged_payload();
    let line = p.format();
    let got = parse_probe_line(&line);

    assert_eq!(got.batches, p.drawn_batches, "batches= in {line:?}");
    assert_eq!(got.visible, p.visible_batches, "visible= in {line:?}");
    assert_eq!(got.frame, p.frame_index, "frame= in {line:?}");
    assert_eq!(got.gpu_frame, p.gpu_observed_frame_index, "gpu_frame= in {line:?}");
    assert_eq!(got.list, p.batch_list, "list= in {line:?}");
    assert_eq!(got.inst, p.record_instance_counts, "inst= in {line:?}");
    assert_eq!(got.late_cnt_pre, p.late_count_pre, "late_cnt_pre= in {line:?}");
    assert_eq!(got.late_cnt_post, p.late_count_post, "late_cnt_post= in {line:?}");
    assert_eq!(got.late_ic, p.late_record_instance_counts, "late_ic= in {line:?}");

    // The three GROUPED lists, each against its own independently-sliced expectation.
    assert_eq!(
        got.vis,
        p.expected_groups(&p.record_instance_counts, &p.visible_instances, 3),
        "`vis` must be each batch's region at its OWN base, sized by `inst`. A contiguous layout \
         would print the 9xx slots no batch owns -- got {line:?}"
    );
    assert_eq!(
        got.late_cand,
        p.expected_groups(&p.late_count_pre, &p.late_candidates, 3),
        "`late_cand` must be sized by `late_cnt_pre` (the DEFERRAL count), not by `inst` and not \
         by `late_ic` -- got {line:?}"
    );
    assert_eq!(
        got.late_surv,
        p.expected_groups(&p.late_record_instance_counts, &p.late_survivors, 3),
        "`late_surv` must be sized by `late_ic` (the KEEP count). Sizing it by `late_cnt_pre` \
         would print the untouched candidate tail as if it had survived, which is precisely the \
         reading plan A5's clause 5 forbids -- got {line:?}"
    );
}

/// The CONTROL that makes the case above able to fail.
///
/// A payload whose regions ARE contiguous cannot distinguish a base-addressed emitter from a
/// position-addressed one — the two produce identical bytes. This test measures that, so the
/// ragged fixture's teeth are a demonstrated property rather than an asserted intention: if a
/// future edit "simplified" `ragged_payload` into contiguous regions, the gate above would keep
/// passing while detecting nothing, and this test is the record of why that must not happen.
#[test]
fn a_contiguous_payload_cannot_tell_the_two_layouts_apart() {
    let p = ragged_payload();

    // The same batch lengths with the SURVIVOR list laid out with no gaps: `bases` is then exactly
    // the running sum of `inst`, which is the one arrangement in which a positional layout and a
    // base-addressed one cannot be told apart.
    //
    // ⚠️ It holds for `vis` ONLY, and that is a fact about the data rather than a shortcut here: the
    // late lists are addressed by the SAME bases but sized by SMALLER counts, so they are never
    // positionally contiguous. The late arrays below are sized to keep every slice in range; no
    // assertion reads them.
    let contiguous = Payload {
        bases: vec![0, 3, 6],
        visible_instances: vec![0, 1, 2, 5, 6, 7, 11, 12],
        late_candidates: vec![20, 21, 0, 25, 26, 27, 31, 0],
        late_survivors: vec![21, 0, 0, 26, 27, 0, 0, 0],
        ..ragged_payload()
    };
    let got = parse_probe_line(&contiguous.format());

    // Position-addressed slicing: batch `b` starts at the running sum of the previous lengths.
    let positional = |lens: &[u32], data: &[u32]| -> Vec<(u32, Vec<u32>)> {
        let mut running = 0usize;
        let mut out = Vec::new();
        for (b, &base) in contiguous.bases.iter().enumerate() {
            let len = lens[b] as usize;
            out.push((base, data[running..running + len].to_vec()));
            running += len;
        }
        out
    };
    assert_eq!(
        got.vis,
        positional(&contiguous.record_instance_counts, &contiguous.visible_instances),
        "on a CONTIGUOUS payload the two layouts must agree -- if they did not, the ragged case \
         above would be testing something other than the grouping rule"
    );

    // And on the ragged payload they must NOT agree, which is the property the gate rests on.
    let ragged = parse_probe_line(&p.format());
    let ragged_positional = {
        let mut running = 0usize;
        let mut out = Vec::new();
        for (b, &base) in p.bases.iter().enumerate() {
            let len = p.record_instance_counts[b] as usize;
            out.push((base, p.visible_instances[running..running + len].to_vec()));
            running += len;
        }
        out
    };
    assert_ne!(
        ragged.vis, ragged_positional,
        "the ragged fixture must SEPARATE the base-addressed layout from the positional one; if \
         these agree the fixture has lost its gaps and the gate is vacuous"
    );
}

/// Every group empty, on every grouped list — the shape an UNSPLIT frame produces, which is every
/// probe run in the tree (no committed `BOYKO_VB_CULL_READBACK` fixture marks `OcclusionCulling`,
/// so none of them arms the split even after the P3-6 arming commit) and therefore the shape the
/// existing corpus gates will actually see.
///
/// It must round-trip as "three groups, each with no members", never as "no groups": the group
/// COUNT is what carries the batch identity, and a parser that dropped empties would renumber the
/// batches.
#[test]
fn an_all_empty_late_payload_round_trips_as_groups_with_no_members() {
    let p = Payload {
        drawn_batches: 3,
        bases: vec![0, 5, 11],
        visible_batches: 3,
        batch_list: vec![0, 1, 2],
        record_instance_counts: vec![3, 3, 2],
        visible_instances: vec![0, 1, 2, 900, 901, 5, 6, 7, 902, 903, 904, 11, 12],
        late_candidates: vec![0; 13],
        late_count_pre: vec![0, 0, 0],
        late_survivors: vec![0; 13],
        late_count_post: vec![0, 0, 0],
        late_record_instance_counts: vec![0, 0, 0],
        frame_index: 31,
        gpu_observed_frame_index: 0,
    };
    let line = p.format();
    let got = parse_probe_line(&line);

    assert_eq!(got.late_cnt_pre, vec![0, 0, 0], "in {line:?}");
    assert_eq!(got.late_ic, vec![0, 0, 0], "in {line:?}");
    assert_eq!(
        got.late_cand,
        vec![(0, vec![]), (5, vec![]), (11, vec![])],
        "three EMPTY groups, one per drawn batch -- a parser that dropped them would renumber the \
         batches, and a gate reading `late_cand[1]` would then be reading batch 2 -- got {line:?}"
    );
    assert_eq!(
        got.late_surv,
        vec![(0, vec![]), (5, vec![]), (11, vec![])],
        "in {line:?}"
    );
    // The `vis` half of the SAME line is non-empty, so this case is not the degenerate "everything
    // is empty" one that would pass under a formatter that emitted nothing at all.
    assert_eq!(got.vis.len(), 3, "in {line:?}");
    assert_eq!(got.vis[2], (11, vec![11, 12]), "in {line:?}");
}

/// A single-batch payload — the shape `[vb_mesh]`'s five spheres produce (one `MeshHandle`, one
/// `DrawBatch`) — so the round trip is exercised at the arity where a `|` separator does not appear
/// at all.
#[test]
fn a_single_batch_payload_round_trips_without_a_separator() {
    let p = Payload {
        drawn_batches: 1,
        bases: vec![0],
        visible_batches: 1,
        batch_list: vec![0],
        record_instance_counts: vec![5],
        visible_instances: vec![0, 1, 2, 3, 4],
        late_candidates: vec![0, 1, 2, 3, 4],
        late_count_pre: vec![2],
        late_survivors: vec![1, 3, 2, 3, 4],
        late_count_post: vec![2],
        late_record_instance_counts: vec![2],
        frame_index: 33,
        gpu_observed_frame_index: 33,
    };
    let line = p.format();
    assert!(!line.contains('|'), "a one-batch line has no group separator: {line:?}");
    let got = parse_probe_line(&line);
    assert_eq!(got.batches, 1);
    assert_eq!(got.vis, vec![(0, vec![0, 1, 2, 3, 4])], "in {line:?}");
    assert_eq!(got.late_cand, vec![(0, vec![0, 1])], "in {line:?}");
    assert_eq!(got.late_surv, vec![(0, vec![1, 3])], "in {line:?}");
}

/// A payload with ZERO drawn batches round-trips as empty lists and empty group sets — the state a
/// frame whose every mesh asset is still loading produces. It must not panic and must not lose the
/// two scalars, which are the only evidence such a run leaves.
#[test]
fn a_zero_batch_payload_round_trips_as_empty_lists() {
    let p = Payload {
        drawn_batches: 0,
        bases: vec![],
        visible_batches: 0,
        batch_list: vec![],
        record_instance_counts: vec![],
        visible_instances: vec![],
        late_candidates: vec![],
        late_count_pre: vec![],
        late_survivors: vec![],
        late_count_post: vec![],
        late_record_instance_counts: vec![],
        frame_index: 30,
        gpu_observed_frame_index: 0,
    };
    let line = p.format();
    let got = parse_probe_line(&line);
    assert_eq!(got.batches, 0, "in {line:?}");
    assert_eq!(got.frame, 30, "the frame index survives an empty payload -- {line:?}");
    assert!(got.vis.is_empty() && got.late_cand.is_empty() && got.late_surv.is_empty());
    assert!(got.inst.is_empty() && got.late_cnt_pre.is_empty() && got.late_ic.is_empty());
}

/// The fixture's own teeth: the ragged payload must actually BE ragged, and its three length
/// sources must actually DIFFER. Asserted rather than eyeballed, because both properties are what
/// the grouping assertions detect with and a "tidying" edit could remove either while leaving every
/// test above green.
#[test]
fn the_ragged_fixture_is_ragged_and_its_three_sizers_disagree() {
    let p = ragged_payload();
    for b in 1..p.bases.len() {
        let prev_end = p.bases[b - 1] + p.record_instance_counts[b - 1];
        assert!(
            p.bases[b] > prev_end,
            "batch {b}'s base ({}) must leave a GAP after batch {}'s region (ends at {prev_end}); \
             without gaps a positional layout is indistinguishable from a base-addressed one",
            p.bases[b],
            b - 1
        );
    }
    assert_ne!(
        p.record_instance_counts, p.late_count_pre,
        "`inst` and `late_cnt_pre` must differ, or sizing `late_cand` by the wrong one is invisible"
    );
    assert_ne!(
        p.late_count_pre, p.late_record_instance_counts,
        "`late_cnt_pre` and `late_ic` must differ, or sizing `late_surv` by the wrong one is \
         invisible -- and this is the pair plan A5's clause 5 turns on"
    );
    assert!(
        p.late_record_instance_counts.contains(&0),
        "one batch must keep NOTHING, so the empty-group case is exercised on the ragged fixture \
         too and not only in its own test"
    );
}
