//! VG rungs R2a′/R2c0: the derived barrier chain on `vb_indirect`, asserted from the sync
//! algebra rather than eyeballed in the recorder.
//!
//! `docs/MESHLET-VIRTUAL-GEOMETRY-RESEARCH.md`'s R2 row exists to de-risk *"cull-pass declaration,
//! compaction, indirect barriers and count buffers"*. Three of those four are structural and are
//! covered by the goldens plus the `.spv` gates. **The barriers are not** — a missing or
//! mis-sourced buffer dependency is invisible to a golden (the frame usually still looks right),
//! invisible to the validation layers (they do not track buffer hazards), and invisible to
//! `robustBufferAccess` (it is off on this device). This file is the gate for that fourth one.
//!
//! `declare_vb_graph` declares exactly three accesses on the `vb_indirect` ResId, in this order:
//!
//! | pass | stage | access |
//! |---|---|---|
//! | `vb_indirect_upload` | `TRANSFER` | `TRANSFER_WRITE` (the inline `vkCmdUpdateBuffer`) |
//! | `vb_batch_cull` | `COMPUTE_SHADER` | `SHADER_WRITE` (the `instanceCount` rewrite) |
//! | `vb_raster` | `DRAW_INDIRECT` | `INDIRECT_COMMAND_READ` (the indirect fetch) |
//!
//! The chain those three must produce is a **WAW** (upload → cull) followed by a **RAW**
//! (cull → raster). Both are derived by [`transition`] from the declarations alone; nothing in the
//! recorder writes a barrier by hand. This test replays that exact sequence against one `ResSync`
//! and pins every field of both derived transitions.
//!
//! Runs unconditionally — pure algebra, no device, no `dxc`, so it cannot SKIP.

use boyko_rhi_vulkan::ffi::{
    VK_ACCESS_INDIRECT_COMMAND_READ_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_ACCESS_TRANSFER_WRITE_BIT,
    VK_IMAGE_LAYOUT_UNDEFINED, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
    VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT,
};
use boyko_rhi_vulkan::framegraph::ResSync;
use boyko_rhi_vulkan::framegraph::sync::transition;

/// A buffer keeps the `UNDEFINED` layout sentinel forever — spelled once so no step of the replay
/// can accidentally introduce a layout change and manufacture a barrier that the real graph would
/// not emit.
const BUF: i32 = VK_IMAGE_LAYOUT_UNDEFINED;

/// The full rung-R2c0 chain: upload (first touch, silent) → cull (WAW) → raster (RAW).
#[test]
fn vb_indirect_chain_is_waw_then_raw() {
    // `declare_vb_graph` seeds `vb_indirect` with `add_buffer` (undefined) — it is per-FIF, so a
    // sibling in-flight frame touches a DIFFERENT slot and there is no cross-frame hazard to seed.
    let mut s = ResSync::undefined();

    // (1) `vb_indirect_upload`. A first-touch BUFFER write has no hazard yet, so NO barrier — but
    // it must still leave a pending flush, which is what makes step (2) a real dependency rather
    // than a bare execution edge.
    let upload = transition(&mut s, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT, BUF);
    assert!(
        upload.is_none(),
        "the upload is the first touch of a frame-private buffer and must emit no barrier; got \
         {upload:?}"
    );
    assert_eq!(
        s.flush_access, VK_ACCESS_TRANSFER_WRITE_BIT,
        "invariant: the silent first-touch write MUST still record its pending flush — without it \
         every later reader derives `src_access = 0`, which is a stale-read hazard that emits a \
         barrier looking entirely correct"
    );

    // (2) `vb_batch_cull` — WAW. The cull overwrites word 1 of each record the transfer just
    // wrote; same bytes, so the outcome is benign, but the ordering hazard is real and must be
    // barriered.
    let cull = transition(&mut s, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, BUF)
        .expect("invariant: the cull's write must order after the upload's (WAW)");
    assert_eq!(cull.src_stage, VK_PIPELINE_STAGE_TRANSFER_BIT);
    assert_eq!(cull.src_access, VK_ACCESS_TRANSFER_WRITE_BIT);
    assert_eq!(cull.dst_stage, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT);
    assert_eq!(cull.dst_access, VK_ACCESS_SHADER_WRITE_BIT);

    // (3) `vb_raster` — RAW, and the source is now the CULL, not the upload. That re-sourcing is
    // the whole structural change rung R2c0 makes to R2a''s seam: the graph tracks the LAST
    // writer, so inserting a pass between producer and consumer moves the dependency without
    // anyone editing the consumer's declaration.
    let raster = transition(
        &mut s,
        VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
        BUF,
    )
    .expect("invariant: the indirect fetch must order after the cull's write (RAW)");
    assert_eq!(
        raster.src_stage, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        "the indirect fetch is sourced on the wrong producer — with the cull declared, the last \
         writer of `vb_indirect` is COMPUTE, not TRANSFER"
    );
    assert_eq!(raster.src_access, VK_ACCESS_SHADER_WRITE_BIT);
    assert_eq!(raster.dst_stage, VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT);
    assert_eq!(
        raster.dst_access, VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
        "invariant: the consumer side is the INDIRECT FETCH access, not a shader read — R1 \
         narrowed `GpuStage::Indirect`'s read arm to exactly this bit"
    );
}

/// SENSITIVITY CONTROL. The assertions above are only worth something if they can tell the two
/// worlds apart — with the cull declared and without it.
///
/// This replays R2a''s chain (upload → raster, no cull) and shows the derived source is TRANSFER,
/// the value the test above forbids. Without this, `vb_indirect_chain_is_waw_then_raw` could be
/// passing because `transition` reports a plausible constant rather than because it tracks the
/// last writer.
#[test]
fn dropping_the_cull_moves_the_derived_source_back_to_transfer() {
    let mut s = ResSync::undefined();
    let _ = transition(&mut s, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT, BUF);
    let raster = transition(
        &mut s,
        VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT,
        VK_ACCESS_INDIRECT_COMMAND_READ_BIT,
        BUF,
    )
    .expect("invariant: R2a''s own chain — the indirect fetch orders after the transfer fill");
    assert_eq!(raster.src_stage, VK_PIPELINE_STAGE_TRANSFER_BIT);
    assert_eq!(raster.src_access, VK_ACCESS_TRANSFER_WRITE_BIT);
    assert_ne!(
        raster.src_stage, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        "the two chains derive the SAME source, so the sibling test cannot distinguish a declared \
         cull from a missing one and its central assertion is vacuous"
    );
}

/// The counter's own chain, which is a DIFFERENT shape and is easy to get wrong: the
/// `vkCmdFillBuffer` reset and the cull's atomics are declared on the SAME pass, so the derived
/// `TRANSFER → COMPUTE` dependency is intra-pass. It is the `light_cull`/`light_index_alloc` shape
/// verbatim, and it is what stops the atomics from racing the zero-fill.
#[test]
fn vb_cull_count_orders_the_atomics_after_the_reset() {
    const RW: u32 = boyko_rhi_vulkan::ffi::VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT;
    let mut s = ResSync::undefined();

    let fill = transition(&mut s, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_ACCESS_TRANSFER_WRITE_BIT, BUF);
    assert!(fill.is_none(), "the counter's zero-fill is its first touch this frame");

    let atomics = transition(&mut s, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, RW, BUF)
        .expect("invariant: the InterlockedAdd must order after the zero-fill");
    assert_eq!(atomics.src_stage, VK_PIPELINE_STAGE_TRANSFER_BIT);
    assert_eq!(atomics.src_access, VK_ACCESS_TRANSFER_WRITE_BIT);
    assert_eq!(atomics.dst_stage, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT);
    assert_eq!(
        atomics.dst_access, RW,
        "invariant: the atomics declare READ|WRITE — a WRITE-only declaration would leave the \
         read half of the read-modify-write unordered against the fill"
    );
}
