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
//! # Rung R2d-3 widened the cull's declaration by two, and both are gated here
//!
//! | ResId | pass | stage | access |
//! |---|---|---|---|
//! | `vb_instance_ring` | `vb_batch_cull` | `COMPUTE_SHADER` | `SHADER_READ` (bound at @4 since R2d-2, declared nowhere until now) |
//! | `vb_visible_instance` | `vb_batch_cull` | `COMPUTE_SHADER` | `SHADER_WRITE` (the per-INSTANCE survivor region) |
//!
//! Each gets a chain test plus the sensitivity control the file's own
//! [`dropping_the_cull_moves_the_derived_source_back_to_transfer`] shape establishes — an
//! assertion that cannot tell the declared world from the undeclared one is not a gate.
//!
//! Runs unconditionally — pure algebra, no device, no `dxc`, so it cannot SKIP.

use boyko_rhi_vulkan::ffi::{
    VK_ACCESS_INDIRECT_COMMAND_READ_BIT, VK_ACCESS_SHADER_READ_BIT, VK_ACCESS_SHADER_WRITE_BIT,
    VK_ACCESS_TRANSFER_WRITE_BIT, VK_IMAGE_LAYOUT_UNDEFINED, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
    VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT,
    VK_PIPELINE_STAGE_VERTEX_SHADER_BIT,
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

/// VG rung R2d-3, access (1 of 2): `vb_instance_ring`'s COMPUTE read, declared on `vb_batch_cull`.
///
/// The ring was BOUND to the cull set at rung R2d-2 (`vb_cull_layout` @4) while its only declared
/// access in this whole graph stayed `vb_raster`'s VERTEX read — the omission round 1's critique
/// found. Declare/record parity is what this file exists to gate: an access the recorder performs
/// and the declarator omits is a dependency derived NOWHERE, and a buffer hazard is invisible to a
/// golden, to the validation layers and to `robustBufferAccess` (off on this device).
///
/// With the read declared, the raster's own read no longer derives off a bare first touch: it
/// orders after the CULL's read (a visibility extension — an execution edge, `src_access == 0`,
/// because the source is a read and there is no memory to make available).
#[test]
fn vb_instance_ring_raster_read_orders_after_the_culls_read() {
    // `declare_vb_graph` seeds `vb_instance_ring` with `add_buffer` (undefined) — per-FIF.
    let mut s = ResSync::undefined();

    // (1) `vb_batch_cull`'s COMPUTE read, new this rung: the FIRST touch of the ring this frame.
    let cull = transition(&mut s, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, BUF)
        .expect("invariant: a first-touch READ takes an execution edge off TOP_OF_PIPE");
    assert_eq!(cull.src_stage, boyko_rhi_vulkan::ffi::VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT);
    assert_eq!(
        cull.src_access, 0,
        "invariant: there is no pending write on the ring, so there is no memory to make available \
         — a nonzero src_access here would be a fabricated dependency"
    );
    assert_eq!(cull.dst_stage, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT);
    assert_eq!(cull.dst_access, VK_ACCESS_SHADER_READ_BIT);

    // (2) `vb_raster`'s VERTEX read. Same ring, different stage, so the visibility must be
    // EXTENDED to VERTEX — and the source is now the cull, not the top of the pipe.
    let raster =
        transition(&mut s, VK_PIPELINE_STAGE_VERTEX_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, BUF)
            .expect("invariant: the VERTEX read is not yet visible — the cull's read covered COMPUTE");
    assert_eq!(
        raster.src_stage, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        "the raster's ring read is sourced on the wrong producer — with the cull's read declared, \
         the last toucher of `vb_instance_ring` is COMPUTE, not the top of the pipe"
    );
    assert_eq!(raster.src_access, 0, "read → read carries no availability operation");
    assert_eq!(raster.dst_stage, VK_PIPELINE_STAGE_VERTEX_SHADER_BIT);
    assert_eq!(raster.dst_access, VK_ACCESS_SHADER_READ_BIT);
}

/// SENSITIVITY CONTROL for the test above, in the shape
/// [`dropping_the_cull_moves_the_derived_source_back_to_transfer`] establishes: replay the graph as
/// it stood BEFORE rung R2d-3 declared the cull's ring read, and show the raster's read derives a
/// DIFFERENT source. Without this, the assertion above could be passing because `transition`
/// reports a plausible constant rather than because the new declaration is actually there.
#[test]
fn dropping_the_culls_ring_read_leaves_the_raster_on_a_first_touch() {
    let mut s = ResSync::undefined();
    let raster =
        transition(&mut s, VK_PIPELINE_STAGE_VERTEX_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, BUF)
            .expect("invariant: R2d-2's own chain — the raster's read is the ring's first touch");
    assert_eq!(raster.src_stage, boyko_rhi_vulkan::ffi::VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT);
    assert_ne!(
        raster.src_stage, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
        "the two chains derive the SAME source, so the sibling test cannot distinguish a declared \
         cull read from a missing one and its central assertion is vacuous"
    );
}

/// VG rung R2d-3, access (2 of 2): `vb_visible_instance`'s COMPUTE WRITE, declared on
/// `vb_batch_cull` — the per-INSTANCE survivor list the cull now fills.
///
/// It is the buffer's ONLY declared access this rung (no shader reads it yet), so the shape to pin
/// is the one the first test pins for `vb_indirect`'s upload: a first-touch buffer write emits NO
/// barrier, but it MUST leave a pending flush. That flush is not bookkeeping — it is what makes
/// rung R2d-4's raster read a real RAW instead of a bare execution edge with `src_access = 0`,
/// which is a stale-read hazard that emits a barrier looking entirely correct.
#[test]
fn vb_visible_instance_write_is_silent_but_leaves_a_pending_flush() {
    let mut s = ResSync::undefined();

    let write =
        transition(&mut s, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT, VK_ACCESS_SHADER_WRITE_BIT, BUF);
    assert!(
        write.is_none(),
        "the cull's region write is the first touch of a frame-private buffer and must emit no \
         barrier; got {write:?}"
    );
    assert_eq!(
        s.flush_access, VK_ACCESS_SHADER_WRITE_BIT,
        "invariant: the silent first-touch write MUST still record its pending flush — the arming \
         rung's reader derives its RAW from exactly this"
    );
    assert_eq!(s.flush_stages, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT);

    // FORWARD-LOOKING, and labelled as such: no pass declares this read yet. Replaying it here is
    // what shows the state left behind is a PRODUCER state — the property the write is declared for
    // in the first place, and the one that would silently be missing if the declaration were
    // dropped.
    let future_reader =
        transition(&mut s, VK_PIPELINE_STAGE_VERTEX_SHADER_BIT, VK_ACCESS_SHADER_READ_BIT, BUF)
            .expect("invariant: a reader of the survivor list must order after the cull's write");
    assert_eq!(future_reader.src_stage, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT);
    assert_eq!(
        future_reader.src_access, VK_ACCESS_SHADER_WRITE_BIT,
        "invariant: the RAW must make the cull's WRITE available — `src_access = 0` here would be \
         an execution edge that orders the stages while leaving the data unflushed"
    );
}

/// The counter's own chain, which is a DIFFERENT shape and is easy to get wrong: the
/// `vkCmdFillBuffer` reset and the cull's atomics are declared on the SAME pass, so the derived
/// `TRANSFER → COMPUTE` dependency is intra-pass. It is the `light_cull`/`light_index_alloc` shape
/// verbatim, and it is what stops the atomics from racing the zero-fill.
#[test]
fn vb_cull_count_orders_the_atomics_after_the_reset() {
    const RW: u32 = VK_ACCESS_SHADER_READ_BIT | VK_ACCESS_SHADER_WRITE_BIT;
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
