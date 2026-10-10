//! `record` — lower a compiled [`FrameGraph`] barrier plan into the minimum number
//! of batched **sync1** `vkCmdPipelineBarrier` calls.
//!
//! A sync1 pipeline barrier carries ONE `(srcStageMask, dstStageMask)` for the
//! whole call but an ARRAY of image + buffer barriers. So the derived barriers of
//! a pass batch into one call PER distinct `(src_stage, dst_stage)` pair; distinct
//! stage pairs cannot merge. [`FrameGraph::record_pass`] performs that grouping
//! **allocation-free** (stack scratch) and drives a [`BarrierSink`] once per group.
//!
//! The sink is the seam between the backend-agnostic derivation (here) and the
//! `Vk*` recording: the REAL sink (swapchain, Step 1c) resolves each
//! [`ImgBarrier::res`] → the current physical `VkImage`, builds the
//! `VkImageMemoryBarrier` array, and calls the `cmd_pipeline_barrier` fn pointer;
//! a TEST sink just counts calls (the C6 call-count measurement, GPU-free).

use super::graph::FrameGraph;
use super::ids::PassId;
use super::sync::{BufBarrier, ImgBarrier};

/// The stack-scratch CHUNK size of the record grouping. The densest real pass
/// boundary (the marcher's input transitions) has 5 barriers; 16 is generous
/// headroom keeping the grouping alloc-free. NOT a hard cap: a pass exceeding it
/// is processed in chunks of 16 (each grouped independently — sound: the same
/// barriers are recorded, at worst one extra `vkCmdPipelineBarrier` per stage
/// pair spanning a chunk boundary). The `debug_assert` still flags the headroom
/// breach so the constant gets raised deliberately (audit B-010: the guard must
/// not be debug-only — `declare_deferred_graph` grows two files away).
pub const MAX_PASS_BARRIERS: usize = 16;

/// A backend sink that RECORDS one batched sync1 `vkCmdPipelineBarrier` per call.
///
/// Each call receives a group of barriers sharing ONE `(src_stage, dst_stage)`.
/// The real impl resolves `res` → the physical handle and records; a test impl
/// counts. Kept object-safe-free (generic at the call site, no `dyn`) so the hot
/// record path has zero virtual dispatch.
pub trait BarrierSink {
    /// Record one image-barrier batch (all sharing `src_stage`/`dst_stage`).
    fn image_barriers(&mut self, src_stage: u32, dst_stage: u32, group: &[ImgBarrier]);
    /// Record one buffer-barrier batch (all sharing `src_stage`/`dst_stage`).
    fn buffer_barriers(&mut self, src_stage: u32, dst_stage: u32, group: &[BufBarrier]);
}

impl FrameGraph {
    /// Emit pass `p`'s derived barriers as the MINIMUM number of sync1 array-form
    /// calls: one per distinct `(src_stage, dst_stage)`. Alloc-free (stack scratch).
    ///
    /// (Image and buffer groups sharing a stage pair are emitted as separate calls
    /// here; fusing them into one mixed call is a sound future micro-opt — the
    /// hand path also records image and buffer barriers separately.)
    pub fn record_pass<S: BarrierSink>(&self, p: PassId, sink: &mut S) {
        let r = self.pass_barriers()[p.index()];
        let img = &self.img_barriers()[r.img_begin as usize..(r.img_begin + r.img_count) as usize];
        emit_img_groups(img, sink);
        let buf = &self.buf_barriers()[r.buf_begin as usize..(r.buf_begin + r.buf_count) as usize];
        emit_buf_groups(buf, sink);
    }

    /// Record every pass's barriers in execution order (the whole frame). The sink
    /// interleaves them with the pass GPU work at record time (Step 1c).
    pub fn record_all<S: BarrierSink>(&self, sink: &mut S) {
        for p in 0..self.pass_barriers().len() {
            self.record_pass(PassId(p as u16), sink);
        }
    }
}

fn emit_img_groups<S: BarrierSink>(bs: &[ImgBarrier], sink: &mut S) {
    debug_assert!(
        bs.len() <= MAX_PASS_BARRIERS,
        "pass image barriers ({}) exceed MAX_PASS_BARRIERS — raise the constant",
        bs.len()
    );
    // Release-safe bound (B-010): chunk an oversized pass instead of indexing the
    // stack arrays out of range (the debug_assert above is compiled out).
    if bs.len() > MAX_PASS_BARRIERS {
        for chunk in bs.chunks(MAX_PASS_BARRIERS) {
            emit_img_groups(chunk, sink);
        }
        return;
    }
    let mut done = [false; MAX_PASS_BARRIERS];
    let mut i = 0;
    while i < bs.len() {
        if done[i] {
            i += 1;
            continue;
        }
        let (ss, ds) = (bs[i].src_stage, bs[i].dst_stage);
        let mut group = [bs[i]; MAX_PASS_BARRIERS];
        let mut n = 0;
        let mut j = i;
        while j < bs.len() {
            if !done[j] && bs[j].src_stage == ss && bs[j].dst_stage == ds {
                group[n] = bs[j];
                n += 1;
                done[j] = true;
            }
            j += 1;
        }
        sink.image_barriers(ss, ds, &group[..n]);
        i += 1;
    }
}

fn emit_buf_groups<S: BarrierSink>(bs: &[BufBarrier], sink: &mut S) {
    debug_assert!(
        bs.len() <= MAX_PASS_BARRIERS,
        "pass buffer barriers ({}) exceed MAX_PASS_BARRIERS — raise the constant",
        bs.len()
    );
    // Release-safe bound (B-010): chunk an oversized pass instead of indexing the
    // stack arrays out of range (the debug_assert above is compiled out).
    if bs.len() > MAX_PASS_BARRIERS {
        for chunk in bs.chunks(MAX_PASS_BARRIERS) {
            emit_buf_groups(chunk, sink);
        }
        return;
    }
    let mut done = [false; MAX_PASS_BARRIERS];
    let mut i = 0;
    while i < bs.len() {
        if done[i] {
            i += 1;
            continue;
        }
        let (ss, ds) = (bs[i].src_stage, bs[i].dst_stage);
        let mut group = [bs[i]; MAX_PASS_BARRIERS];
        let mut n = 0;
        let mut j = i;
        while j < bs.len() {
            if !done[j] && bs[j].src_stage == ss && bs[j].dst_stage == ds {
                group[n] = bs[j];
                n += 1;
                done[j] = true;
            }
            j += 1;
        }
        sink.buffer_barriers(ss, ds, &group[..n]);
        i += 1;
    }
}
