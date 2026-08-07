//! `Renderer::record_vb`: the on-screen FUSED VisibilityBuffer v1 record body behind
//! [`Renderer::render_gbuffer_frame`] — the `VisibilityBuffer` sibling of
//! [`record_forward`](super::forward), driven through
//! [`VbBarrierSink`](super::super::graph_bridge::VbBarrierSink) via [`Renderer::record_vb_pass`].
//!
//! # v1 SCOPE CUT (mirrors `vb_resolve.comp.hlsl`'s own doc)
//!
//! NO SSAO/DDGI/shadow-denoise/motion-vector/TAA (`cap_vb_v1_consumers` forces every one of
//! those consumers off structurally); NO froxel (VB v1 shades ALL-LIGHTS, mirrors plain
//! `Forward`'s own base compile); NO `interp` (the VB instance ring is a plain CPU `bytemuck`
//! upload, no GPU-side interpolation this rung); NO depth prepass (`mesh_geo_shade_split ==
//! false`, fused only); NO SDF leg (`VisibilityBuffer × {Both, Sdf}` collapses to `Mesh` until
//! R10 — `LegsCollapsedToMeshPreVbSdf`). Shadows (CSM + punctual atlas) ARE in scope —
//! `vb_resolve.comp.hlsl` samples them inline via `shadow_apply.hlsli`.
//!
//! # Why a SEPARATE record body, not a `record_forward` branch
//!
//! Mirrors `record_forward`'s own "own private ResId space, zero edits to Deferred/Forward's
//! reachable code" trade-off — see that file's doc for the rationale. Every duplicated block
//! (light_upload/csm/atlas/present-blit) is a byte-for-byte port of `record_forward`'s
//! counterpart, adapted only for the VB-private barrier sink (`record_vb_pass` vs
//! `record_forward_pass`) and targets.

use core::ptr;

#[cfg(feature = "hwrt")]
use crate::compute::LOCAL_SIZE_X;
use crate::compute::{HZB_BUILD_PUSH_BYTES, HZB_BUILD_TILE, HZB_LEVELS_PER_PASS};
use crate::ffi::*;
use crate::memory::BoundBuffer;
use crate::texture::{MAX_CASCADES, MAX_TEXTURE_LAYERS};

use super::super::frame_driver::Renderer;
use super::super::gpu_timing::VbTimedPass;
use super::super::scene_types::{
    CLUSTER_CULL_HIER_PUSH_BYTES, CLUSTER_CULL_PUSH_BYTES, GBUFFER_PUSH_BASE_INSTANCE_OFFSET,
    GBufferMeshDraw, GBufferScene, HZB_DUMP_HEADER_BYTES, HZB_PYRAMID_POISON, HzbDumpLayout,
    LIGHT_CULL_LOCAL_SIZE_X, MAX_HZB_LEVELS, VB_BATCH_CULL_LOCAL_SIZE_X, VB_BATCH_CULL_PUSH_BYTES,
    VB_BATCH_DESC_STRIDE, VB_CULL_OCC_ARMED, VB_CULL_OCC_FORCE_KEEP, VB_CULL_OCC_FORCE_LATE,
    VB_CULL_PHASE_EARLY, VB_CULL_PHASE_LATE, VB_CULL_UNIFORM_BYTES, VbBatchCullPush, VbBatchDesc,
    VbCullUniform,
};
use super::super::targets::{ForwardTargets, GBufferTargets, VB_CLASSIFY_MAX_MATERIAL_ROWS, VbTargets};
use super::super::{COLOR_SUBRESOURCE_RANGE, SwapchainError};

/// The VB `lit` clear color — byte-identical to `record_forward`'s own `FORWARD_LIT_CLEAR`
/// (`record_gbuffer`'s albedo clear, the marcher's `BACKGROUND` base). VB v1 has no SDF leg
/// (mesh-only, `LegsCollapsedToMeshPreVbSdf` until R10) and paints the sky FIRST (`vb_sky`), so
/// this clear is only ever visible transiently before `vb_sky` overwrites every pixel.
const VB_LIT_CLEAR: [f32; 4] = [0.05, 0.05, 0.1, 1.0];

/// The VB reverse-Z depth CLEAR (Decision 4): `0.0`, the "nothing drawn yet" sentinel — mirrors
/// `record_forward`'s own `FORWARD_DEPTH_CLEAR` (paired with `VK_COMPARE_OP_GREATER`).
const VB_DEPTH_CLEAR: f32 = 0.0;

/// The `vb_id` CLEAR value (Decision 9 / plan §F): `uint2(0xFFFFFFFF, 0)` — the SDF-owned /
/// unwritten-pixel sentinel (`VB_ID_SENTINEL`, `boyko_render::render_path_config` +
/// `vb_pack.hlsli`). A miss reads `instance_id == VB_ID_SENTINEL` and `vb_resolve` writes
/// nothing for that pixel (the sky color already painted by `vb_sky` stands).
const VB_ID_CLEAR: [u32; 4] = [0xFFFF_FFFF, 0, 0, 0];

/// VG rung R2d-4: bit 1 of the `vb_raster.vs.hlsl` push's `use_model_matrix` word — "read this
/// draw's instance indices THROUGH `gVbVisibleInstance` (Set-0 @11) instead of computing them".
///
/// Bit 0 keeps its pre-R2d meaning (`0` = legacy arm, non-zero = instanced arm), which is what
/// makes the word safe to widen: every VS in `crates/boyko_rhi_vulkan/shaders` spells the arm test
/// `pc.use_model_matrix == 0u` and NONE tests `== 1u`, so a set bit 1 cannot select the wrong arm
/// in any of them. The bit is meaningful ONLY beside a set bit 0 — see the recorder's own
/// `debug_assert!`.
pub(crate) const VB_RASTER_FLAG_VISIBLE_INDIRECTION: u32 = 2;

/// Bit 0 of the same word — the pre-R2d ARM selector (`0` = legacy arm, non-zero = instanced arm),
/// minted by `boyko_render::view::forward_view_proj_rows` into `GBufferScene::mvp`'s byte 84.
/// Named here only so the recorder's `debug_assert!` can state its relationship to
/// [`VB_RASTER_FLAG_VISIBLE_INDIRECTION`] without a bare `1`.
const VB_RASTER_FLAG_INSTANCED_ARM: u32 = 1;

/// VG R3 piece 2 step P2-6 — **gate G2's counts, AUTHORED BY THE RECORDER.**
///
/// [`Renderer::record_vb`] fills one of these through an `Option<&mut VbRecordProbe>` parameter
/// when a caller asks for it; `None` on every steady, golden and interactive frame, where the
/// whole probe costs one `Option` check per counted site and records no command.
///
/// # Why these numbers ORIGINATE here rather than being re-derived on the host
///
/// The gate's question is "did the recorder actually record two raster scopes?". A host that
/// re-derives `scopes` from `GBufferScene::vb_occlusion_instances` agrees with itself no matter
/// what this function did — the tautology this campaign has shipped as a gate five times. So the
/// counts are incremented AT the `vkCmd*` calls they count, and the host's own numbers
/// (`draw_batches`, the marked-instance count) travel beside them as an INDEPENDENT cross-check
/// rather than as their source.
///
/// # Why a `&mut` parameter and not a device buffer
///
/// Every count is known on the host at record time. A buffer would add an allocation, a declared
/// pass, a barrier, a fence wait and a decode to move a number that is already in a register —
/// and would change the recorded command stream, which is precisely what this piece claims not
/// to do.
///
/// # What a filled probe CANNOT claim
///
/// That the GPU *executed* the scope. A scope whose every draw carries `instanceCount = 0` has no
/// observable consequence of execution, so no gate in this repository can close that gap; this
/// one stops at "the host recorded it".
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct VbRecordProbe {
    /// Raster scopes (`vkCmdBeginRendering`/`EndRendering` brackets over `vb_id` + `vb_depth`)
    /// recorded this frame: `1` unsplit, `2` on an armed split, and `0` on a frame that records
    /// no mesh raster at all (`VisibilityBuffer × Sdf`, or a non-VB path where `record_vb` never
    /// runs — the probe then stays at its `Default`).
    pub scopes: u32,
    /// `vkCmdDrawIndexedIndirect` calls issued in the LATE scope. `0` unless the split is armed;
    /// otherwise the same `batch_count` bound the early scope and the cull dispatch share.
    pub late_draws: u32,
    /// The sum of `instanceCount` over the late records this frame WROTE. `0` in piece 2 by
    /// construction — the late scope draws nothing — and the number piece 3 turns nonzero.
    pub late_instances: u32,
}

/// VG R3 piece 1 step P1-5: the 72-byte `hzb_build` push constant, mirrored FIELD FOR FIELD from
/// `crates/boyko_rhi_vulkan/shaders/hzb_build.comp.hlsl`'s `HzbBuildPush`.
///
/// The six destination extents are SIX NAMED FIELDS rather than a `[[u32; 2]; 6]`, because that is
/// how the HLSL spells them: a reader can diff this declaration one-for-one against the shader
/// instead of counting subscripts. The layout is the shader's own
/// `OpMemberDecorate ... Offset` sequence — eight `uint2` at 0, 8, …, 56, then two `uint` at 64
/// and 68 — pinned host-side by the `const _` below and shader-side by
/// `tests/hzb_build_spv_sync.rs`.
///
/// `#[repr(C)]` PLUS a hand-written [`Self::to_bytes`], which is the shape
/// `boyko_app/tests/hzb_build_oracle_gate.rs` already ships for this same block: the attribute and
/// the `const` size assert pin the layout, while writing the words out is what makes the byte
/// OFFSETS — the actual contract with the HLSL — reviewable rather than implied.
#[repr(C)]
#[derive(Clone, Copy)]
struct HzbBuildPush {
    /// `S` — the SOURCE depth extent, on EVERY pass.
    ///
    /// ⚠️ NOT `plan.extent_of(0)`. Level 0 is `prev_pow2` of each source axis, so at 1920×1080 the
    /// source is 1920×1080 while level 0 is 1024×1024, and the base map `first(t) = ⌈t·S/P⌉` reads
    /// BOTH. The two coincide only when `S == P` — which is every golden pin's 512×512 — so a
    /// confusion here is invisible at the extents this repository gates and wrong at every other.
    src_extent: [u32; 2],
    /// `E(d-1)`, the level this pass reduces FROM. Read only by a reduce pass (`base_level != 0`);
    /// the base pass is handed level 0's own extent as a well-defined placeholder rather than a
    /// zero that would divide.
    fine_extent: [u32; 2],
    /// `E(d)` — and, on the base pass, `P = prev_pow2(S)` per axis.
    out_extent0: [u32; 2],
    /// `E(d+1)`.
    out_extent1: [u32; 2],
    /// `E(d+2)`.
    out_extent2: [u32; 2],
    /// `E(d+3)`.
    out_extent3: [u32; 2],
    /// `E(d+4)`.
    out_extent4: [u32; 2],
    /// `E(d+5)`.
    out_extent5: [u32; 2],
    /// `d` — this pass's first output level, and the base/reduce variant discriminator
    /// (`0` ⇔ BASE, the shader's one uniform branch).
    base_level: u32,
    /// How many levels THIS pass writes, in `1 ..= HZB_LEVELS_PER_PASS`. Every store in the shader
    /// is guarded by `k < level_count`, which is what makes the padded `gDst` bindings unwritten.
    level_count: u32,
}

const _: () = assert!(
    core::mem::size_of::<HzbBuildPush>() == HZB_BUILD_PUSH_BYTES as usize,
    "VG R3 P1-5: HzbBuildPush must match hzb_build.comp.hlsl's 72-byte push block"
);

impl HzbBuildPush {
    /// Serializes the block to its 72 wire bytes, little-endian, field by field — eighteen `u32`
    /// words in the shader's own member order.
    #[inline]
    fn to_bytes(self) -> [u8; HZB_BUILD_PUSH_BYTES as usize] {
        let words: [u32; HZB_BUILD_PUSH_BYTES as usize / 4] = [
            self.src_extent[0],
            self.src_extent[1],
            self.fine_extent[0],
            self.fine_extent[1],
            self.out_extent0[0],
            self.out_extent0[1],
            self.out_extent1[0],
            self.out_extent1[1],
            self.out_extent2[0],
            self.out_extent2[1],
            self.out_extent3[0],
            self.out_extent3[1],
            self.out_extent4[0],
            self.out_extent4[1],
            self.out_extent5[0],
            self.out_extent5[1],
            self.base_level,
            self.level_count,
        ];
        let mut bytes = [0u8; HZB_BUILD_PUSH_BYTES as usize];
        for (i, w) in words.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        bytes
    }
}

/// VG rung R2d-3: the number of LEADING batches whose OWNED region of the per-instance survivor
/// list (`gVbVisibleInstance`) fits inside `visible_elems` — i.e. the index of the FIRST batch
/// whose `base_instance + instance_count` would run past the end, or `mesh_draw.len()` when none
/// does.
///
/// `visible_elems` must be derived from the ALLOCATION (`BoundBuffer::size / 4`), never from the
/// host constant that sized it — the VB-P1j lesson, which this file already applies at
/// `record_capacity` and at the cull's own `visible_cap`: a capacity carried as a separate word
/// drifts from the buffer it claims to describe and nothing detects it. (Both are cited by
/// IDENTIFIER rather than by line: they are unique in this file, and a line number in a doc
/// comment inside the very file it points into is the one citation form guaranteed to rot on the
/// next edit — as this comment's first draft demonstrated by pointing at unrelated code.)
///
/// # Why a PREFIX is sound — the clamp cannot let a later batch slip through
///
/// `MeshRenderScratch::gather_mixed_into` assigns `base_instance = running` BEFORE it adds that
/// mesh's count (`crates/boyko_render/src/mesh_draw.rs:815-832`: `offsets[m] = running;` … `running
/// += c;`, and the emitted `DrawBatch` carries `base_instance: running` from the same iteration).
/// Bases are therefore NON-DECREASING in batch order, and every emitted batch has `c >= 1`
/// (`counts[m] == 0` zeroes out into `resolved == None` and emits no batch at all — same lines), so
/// they are STRICTLY ASCENDING. `base + count` is likewise non-decreasing, which makes the
/// predicate `base + count > visible_elems` MONOTONE: once it is true for one batch it is true for
/// every later one. The first index that trips it is thus a genuine prefix boundary, not a filter —
/// no batch past it can fit, so none is silently dropped from the middle of the list.
///
/// A clamped-away batch is NOT degraded: it keeps the `VkDrawIndexedIndirectCommand` the host's own
/// transfer fill wrote (the cull never visits it, since the dispatch covers only this prefix) and,
/// from rung R2d-4 on, a CLEAR indirection bit — i.e. exactly pre-R2d rendering for that batch.
///
/// Widened to `usize` on purpose: the two `u32` fields are summed in `usize` so the check itself
/// cannot wrap on a pathological descriptor (a wrapped sum would compare SMALL and admit a batch
/// whose region runs off the end).
pub(crate) fn vb_cull_batch_count_visible_clamp(
    mesh_draw: &[GBufferMeshDraw<'_>],
    visible_elems: usize,
) -> usize {
    mesh_draw
        .iter()
        .position(|b| b.base_instance as usize + b.instance_count as usize > visible_elems)
        .unwrap_or(mesh_draw.len())
}

/// VG rung R2d-5: byte offset of the cull readback staging's COUNT region. The other eight offsets
/// are derived from the region sizes ([`VbCullReadbackLayout`]), because only the first one can be
/// a constant — the rest depend on how large the buffers they follow actually are.
pub(crate) const VB_CULL_READBACK_COUNT_OFFSET: u64 = 0;

/// VG R3 piece 3 step P3-5: the SOURCE allocation sizes the readback staging is packed from — one
/// field per distinct source buffer, never per region.
///
/// `late_visible` and `late_count` are each copied TWICE (once before `vb_cull_late`, once after
/// `vb_raster_late`), and the two copies necessarily have the same length because they name the same
/// allocation. Spelling the sources rather than the regions is what makes that a fact of the type
/// instead of a convention two call sites have to keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VbCullReadbackSources {
    /// `vb_cull_count`'s allocation size.
    pub(crate) count: u64,
    /// `vb_cull_visible`'s allocation size.
    pub(crate) list: u64,
    /// `vb_indirect`'s allocation size.
    pub(crate) records: u64,
    /// `vb_visible_instance`'s allocation size.
    pub(crate) vis: u64,
    /// `vb_late_visible`'s allocation size — copied into the PRE and the POST snapshot alike.
    pub(crate) late_visible: u64,
    /// `vb_late_count`'s allocation size — likewise copied twice.
    pub(crate) late_count: u64,
    /// `vb_indirect_late`'s allocation size (POST snapshot only).
    pub(crate) late_records: u64,
}

/// VG rung R2d-5 / VG R3 piece 3 step P3-5: the NINE region SIZES of the cull readback staging, in
/// staging order — COUNT | LIST | RECORDS | VIS | LATE_CAND | LATE_CNT_PRE | LATE_SURV |
/// LATE_CNT_POST | LATE_REC.
///
/// Every field is the byte size of the ALLOCATION that region copies, capped by whatever the
/// staging still has unassigned. There is deliberately no "remainder" region and no literal size:
/// rung R2c-tail's `rb.size - 16` could not be checked against its source, and the design draft
/// that preceded this rung proposed the literals "8 records" and "32 entries", neither of which can
/// hold the 45-instance, 7-batch corpus the probe exists to observe.
///
/// # Why the last five regions exist even on a frame that copies nothing into them
///
/// The late five are written only on an occlusion-SPLIT frame (the declarator gates their
/// `TRANSFER_READ` accesses on exactly that — `graph_bridge.rs`'s `vb_cull_readback` /
/// `vb_cull_readback_late` blocks), while the layout is computed from the ALLOCATIONS and is
/// therefore the same on every frame. That is deliberate: the host decodes at CONSTANT offsets, so
/// a layout whose region set moved with the frame's arming would read the same bytes as different
/// fields on different frames. An unsplit frame leaves those regions holding the staging's
/// zero prefill (`boyko_app`'s `GpuSceneBundles::boot`) — "no candidates", which is exactly what an
/// unsplit frame has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VbCullReadbackLayout {
    /// Bytes copied from the visible-BATCH counter (`vb_cull_count`).
    pub(crate) count: u64,
    /// Bytes copied from the compacted visible-BATCH list (`vb_cull_visible`).
    pub(crate) list: u64,
    /// Bytes copied from the indirect draw-record array (`vb_indirect`) — the post-cull
    /// `instanceCount` words live at word 1 of each record.
    pub(crate) records: u64,
    /// Bytes copied from the per-INSTANCE survivor list (`vb_visible_instance`).
    pub(crate) vis: u64,
    /// PRE-late snapshot: `vb_late_visible` as the EARLY phase wrote it — the CANDIDATE list. The
    /// only place the candidate set is observable at all, because the late phase compacts the same
    /// region in place (plan A3's corollary).
    pub(crate) late_candidates: u64,
    /// PRE-late snapshot: `vb_late_count` — each batch's `n_defer`, plus the reserved frame slot.
    pub(crate) late_count_pre: u64,
    /// POST-late snapshot: `vb_late_visible` after compaction — the SURVIVOR prefix followed by the
    /// original tail.
    pub(crate) late_survivors: u64,
    /// POST-late snapshot: `vb_late_count` again — the no-clobber clause's second side.
    pub(crate) late_count_post: u64,
    /// POST-late snapshot: `vb_indirect_late`, whose word 1 per record is the `instanceCount` the
    /// LATE cull wrote and the late raster fetches.
    pub(crate) late_records: u64,
}

impl VbCullReadbackLayout {
    /// Destination byte offset of the LIST region — the COUNT region begins the staging at
    /// [`VB_CULL_READBACK_COUNT_OFFSET`], so the LIST begins where it ends.
    pub(crate) const fn list_offset(&self) -> u64 {
        self.count
    }

    /// Destination byte offset of the RECORDS region.
    pub(crate) const fn records_offset(&self) -> u64 {
        self.list_offset() + self.list
    }

    /// Destination byte offset of the VIS region.
    pub(crate) const fn vis_offset(&self) -> u64 {
        self.records_offset() + self.records
    }

    /// Destination byte offset of the PRE-late CANDIDATE region.
    pub(crate) const fn late_candidates_offset(&self) -> u64 {
        self.vis_offset() + self.vis
    }

    /// Destination byte offset of the PRE-late COUNT region.
    pub(crate) const fn late_count_pre_offset(&self) -> u64 {
        self.late_candidates_offset() + self.late_candidates
    }

    /// Destination byte offset of the POST-late SURVIVOR region.
    pub(crate) const fn late_survivors_offset(&self) -> u64 {
        self.late_count_pre_offset() + self.late_count_pre
    }

    /// Destination byte offset of the POST-late COUNT region.
    pub(crate) const fn late_count_post_offset(&self) -> u64 {
        self.late_survivors_offset() + self.late_survivors
    }

    /// Destination byte offset of the POST-late RECORD region.
    pub(crate) const fn late_records_offset(&self) -> u64 {
        self.late_count_post_offset() + self.late_count_post
    }

    /// Total staging bytes the nine regions occupy.
    pub(crate) const fn total(&self) -> u64 {
        self.late_records_offset() + self.late_records
    }

    /// `true` iff every region carries its whole source buffer — i.e. the staging was large enough
    /// and nothing was silently trimmed.
    pub(crate) const fn is_untruncated(&self, src: &VbCullReadbackSources) -> bool {
        self.count == src.count
            && self.list == src.list
            && self.records == src.records
            && self.vis == src.vis
            && self.late_candidates == src.late_visible
            && self.late_count_pre == src.late_count
            && self.late_survivors == src.late_visible
            && self.late_count_post == src.late_count
            && self.late_records == src.late_records
    }
}

/// Packs the cull output allocations into the readback staging.
///
/// Each region takes its SOURCE buffer's size, capped by the staging bytes the regions before it
/// left unassigned. That cap is what makes an overflowing copy unrepresentable rather than merely
/// unlikely: a staging smaller than the sources produces short (or zero-length) trailing
/// regions, which the caller skips, instead of a `vkCmdCopyBuffer` writing past the allocation —
/// undefined with `robustBufferAccess` off and invisible to the validation layers, which do not
/// follow buffer contents.
///
/// The host sizes the staging from the same capacity constants (`boyko_app`'s
/// `VB_CULL_READBACK_BYTES`), so on every real boot nothing is trimmed and the recorder's
/// `debug_assert` on [`VbCullReadbackLayout::is_untruncated`] holds.
pub(crate) fn vb_cull_readback_layout(
    src: &VbCullReadbackSources,
    staging_bytes: u64,
) -> VbCullReadbackLayout {
    // TWO properties, and each rules out a different silent corruption.
    //
    // ALL-OR-NOTHING per region, not `min`: a `min` makes a PARTIAL region representable, so a
    // staging one byte short would yield a 20479-of-20480-byte RECORDS copy and the decode would
    // read a record array truncated mid-record while every length it checks still looked sane.
    //
    // PREFIX, not per-region: once one region does not fit, every LATER one is dropped too, even
    // if it would have fitted in the space the dropped one vacated. Letting a successor slide
    // forward would move its destination offset — and the host decodes at CONSTANT offsets, so a
    // slid region is read as the wrong bytes rather than as missing data. This is the same
    // monotone-prefix discipline `vb_cull_batch_count_visible_clamp` applies to batches, for the
    // same reason: a hole in the middle is undetectable downstream, a truncated tail is not.
    let mut remaining = staging_bytes;
    let mut take = |want: u64| {
        if want <= remaining {
            remaining -= want;
            want
        } else {
            remaining = 0;
            0
        }
    };
    let count = take(src.count);
    let list = take(src.list);
    let records = take(src.records);
    let vis = take(src.vis);
    let late_candidates = take(src.late_visible);
    let late_count_pre = take(src.late_count);
    let late_survivors = take(src.late_visible);
    let late_count_post = take(src.late_count);
    let late_records = take(src.late_records);
    VbCullReadbackLayout {
        count,
        list,
        records,
        vis,
        late_candidates,
        late_count_pre,
        late_survivors,
        late_count_post,
        late_records,
    }
}

/// VG R3 piece 3 step P3-5: reads the seven cull source ALLOCATION sizes off `scene` for
/// frame-in-flight slot `fi`.
///
/// Every size comes from the live [`BoundBuffer::size`] rather than from a host capacity constant —
/// the VB-P1j lesson this file already applies at `record_capacity` and at the cull's `visible_cap`.
/// Both readback snapshots call THIS, so the PRE pass and the POST pass cannot pack against two
/// different layouts: a disagreement there would put the post-late regions at offsets the host does
/// not decode at, and every field would read as plausible nonsense.
///
/// `None` on a scene that carries no cull at all (every non-`VisibilityBuffer` boot).
pub(crate) fn vb_cull_readback_sources(
    scene: &GBufferScene<'_>,
    fi: usize,
) -> Option<VbCullReadbackSources> {
    Some(VbCullReadbackSources {
        count: scene.vb_cull_count?[fi].size,
        list: scene.vb_cull_visible?[fi].size,
        records: scene.vb_indirect?[fi].size,
        vis: scene.vb_visible_instance?[fi].size,
        late_visible: scene.vb_late_visible?[fi].size,
        late_count: scene.vb_late_count?[fi].size,
        late_records: scene.vb_indirect_late?[fi].size,
    })
}

impl Renderer<'_> {
    /// Records the VisibilityBuffer on-screen frame: `light_upload? → csm? → atlas? → vb_sky →
    /// vb_raster → vb_resolve → present-blit` — EXACTLY [`Renderer::declare_vb_graph`]'s
    /// declaration order (the SAME "declare/record order parity" invariant `record_forward`'s
    /// doc explains).
    ///
    /// # The two shapes, and the ONE predicate that picks between them
    ///
    /// VG R3 piece 2 step P2-5 (docs/VG-R3-P2-CAPABILITY-SPLIT-PLAN.md, decisions D4/D6):
    /// [`GBufferScene::path_vb_occlusion_split`] — the SAME method `declare_vb_graph` reads,
    /// evaluated ONCE per frame here — picks the frame's pass chain:
    ///
    /// * **unsplit** (every scene in the tree today, since nothing marks `OcclusionCulling`):
    ///   `… → vb_indirect_upload? → vb_batch_cull? → vb_raster → (classify?) →
    ///   vb_shade | vb_resolve → hzb_poison? → hzb_build_*? → …`, i.e. ZERO recorded commands
    ///   change — no extra pass, no extra barrier, no extra draw;
    /// * **armed split**: `… → vb_indirect_upload? → vb_indirect_late_upload → vb_batch_cull? →
    ///   vb_cull_readback? → vb_raster → hzb_poison? → hzb_build_*? → vb_cull_late →
    ///   vb_raster_late → vb_cull_readback_late? → (classify?) → …` (VG R3 piece 3 step P3-3
    ///   inserted the two `vb_cull_*late` passes).
    ///
    /// The late scope is fully recorded and DRAWS NOTHING: every record it fetches carries
    /// `instanceCount = 0`, both attachments are `LOAD_OP_LOAD`/`STORE_OP_STORE` over the same
    /// `renderArea`, so the framebuffer contents it stores are the ones the early scope stored.
    /// The late CULL is likewise recorded and DECIDES NOTHING: since step P3-4 its phase-1 body is
    /// real, but its loop bound is `VbLateCount[i]`, which the early phase writes only under
    /// `VB_CULL_OCC_ARMED` and reads as `0` without it — so it compacts nothing and stores the same
    /// `instanceCount = 0` the host's `vb_indirect_late_upload` already seeded.
    ///
    /// # Safety
    ///
    /// `cmd` must be recordable (waited free); `image`/`view` must belong to the swapchain image
    /// presented this frame; `scene`'s pipelines/buffers/samplers are live on this device;
    /// `targets`/`forward`/`vb` were synced to `present_extent` (the SAME contract
    /// [`record_gbuffer`](super::gbuffer)'s doc states, restricted to VB's own images/sets).
    /// `extent` is the swapchain extent and governs ONLY the present-blit's clear render-area and
    /// the readback region; a `Some(readback)` buffer is host-visible and ≥ the swapchain image's
    /// (`extent`-sized) byte size. Rung R9d: `accel_fns` is the AS command table for the split's
    /// own per-frame TLAS build (`Some` only on an RT device) — the SAME contract
    /// [`record_gbuffer`](super::gbuffer)'s own `accel_fns` param documents.
    ///
    /// VG-R0 rung R0c: a `Some(vb_id_readback)` buffer is host-visible and ≥
    /// `present_extent.width * present_extent.height * 8` bytes (`R32G32_UINT`, 8 B/texel — the
    /// `vb_id` ring is sized to `present_extent`, NOT `extent`; under armed SSAA that is the 2×
    /// composite, which is what makes the top two ladder rungs reachable at all). `None` on every
    /// steady/golden frame, and an unarmed frame records **zero** extra commands — the byte-
    /// neutrality R0c gate (a) rests on.
    ///
    /// VG R3 piece 1 step P1-6: the pyramid dump's staging is NOT a parameter — it rides on
    /// [`GBufferScene::hzb_dump`](super::super::scene_types::GBufferScene::hzb_dump), because
    /// unlike the census's `vb_id` copy it is a DECLARED framegraph pass and the declarator sees
    /// only the scene. A `Some` there (plus an armed pyramid and a mesh leg) means this frame
    /// copies `vb_depth` and every pyramid mip into a buffer of at least
    /// [`HzbDumpLayout::total_bytes`](super::super::scene_types::HzbDumpLayout::total_bytes) for
    /// this frame's plan and `present_extent`.
    ///
    /// VG R3 piece 2 step P2-6: `probe` is gate G2's recorder-authored count sink
    /// ([`VbRecordProbe`]). `None` on every steady/golden/interactive frame, and a `None` frame
    /// records byte-identical commands — the probe writes host memory only. It is a `&mut`
    /// PARAMETER despite the `&self` receiver precisely so it cannot become recorder state that
    /// survives a frame.
    #[allow(clippy::too_many_arguments)]
    pub(crate) unsafe fn record_vb(
        &self,
        cmd: VkCommandBuffer,
        image: VkImage,
        view: VkImageView,
        extent: VkExtent2D,
        present_extent: VkExtent2D,
        aa_extent: VkExtent2D,
        clear: [f32; 4],
        scene: &GBufferScene<'_>,
        targets: &GBufferTargets,
        forward: &ForwardTargets,
        vb: &VbTargets,
        readback: Option<&BoundBuffer>,
        vb_id_readback: Option<&BoundBuffer>,
        mut probe: Option<&mut VbRecordProbe>,
        #[cfg(feature = "hwrt")] accel_fns: Option<&crate::accel::AccelFns>,
    ) -> Result<(), SwapchainError> {
        let begin = VkCommandBufferBeginInfo {
            s_type: VkStructureType::CommandBufferBeginInfo,
            p_next: ptr::null(),
            flags: VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT,
            p_inheritance_info: ptr::null(),
        };
        // SAFETY: `cmd` is recordable per this fn's contract; `begin` is a fully-initialized
        // one-time-submit begin-info.
        let raw = unsafe { (self.fns.begin_command_buffer)(cmd, &begin) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkBeginCommandBuffer", result));
        }

        let fi = self.frame_index;

        // VB-P1d: reset ALL `2 * VB_PASS_COUNT` bench queries at the frame top — OUTSIDE any
        // render / dynamic-rendering scope (recording is open but no `begin_rendering` has run
        // yet), before the frame's first `write_timestamp`. GATED on `scene.vb_gpu_timing`:
        // `None` (every golden/host/interactive frame) records NOTHING, so the command stream
        // is byte-identical. A TIMESTAMP query is undefined until reset.
        if let Some(tc) = scene.vb_gpu_timing {
            // SAFETY: recording is open; `self.fns` is the live device fn-table; the reset is
            // recorded before any `begin_rendering` (outside a render pass, per
            // `VUID-vkCmdResetQueryPool-renderpass`); `fi` is this present's in-flight slot.
            unsafe { tc.reset_frame(self.fns, cmd, fi) };
        }

        let plan = self.vb_pass_plan.as_ref().expect("invariant: declare_frame_graph ran before record_vb");

        // VG R3 piece 2 step P2-5 (plan D3/D4/D6): THE single source of "this frame records TWO
        // raster scopes", read ONCE — the SAME method `declare_vb_graph` read when it built `plan`,
        // off the SAME scene. Four recorded things key off it (the late fill, the late scope, and
        // which of the two slots the `[hzb_poison, hzb_build_*]` block occupies), and every one of
        // them has a declared counterpart; a second spelling here is how declare/record parity
        // breaks. The `debug_assert!`s below check the agreement rather than assume it.
        let occlusion_split = scene.path_vb_occlusion_split();
        debug_assert_eq!(
            plan.vb_raster_late.is_some(),
            occlusion_split,
            "invariant: declare/record parity — the late raster scope is declared on EXACTLY the \
             predicate that records it"
        );
        debug_assert_eq!(
            plan.vb_indirect_late_upload.is_some(),
            occlusion_split,
            "invariant: declare/record parity — the late indirect upload is declared on EXACTLY \
             the predicate that records it"
        );

        let vertex_offset: VkDeviceSize = 0;

        // === Lighting L0-r0: ASYNC light-table re-upload — byte-for-byte port of
        // `record_forward`'s own `light_upload` block. Recorded ONLY on a dirty frame. ===
        if scene.light_dirty && scene.light_upload_bytes > 0 {
            let light_upload =
                plan.light_upload.expect("invariant: light_dirty ⇒ light_upload pass declared");
            // SAFETY: recording is open; `record_vb_pass` records the graph's derived
            // cross-frame seed-WAR buffer barrier for the "light_upload" pass into `cmd`, ahead
            // of the copy it guards.
            self.record_vb_pass(light_upload, cmd, targets, forward, vb, scene, fi);
            let region = VkBufferCopy { src_offset: 0, dst_offset: 0, size: scene.light_upload_bytes };
            // SAFETY: recording is open; the copy names the live host-coherent staging +
            // device-local table buffers; the copy region spans `[0, light_upload_bytes)` ≤ both
            // buffer sizes (caller contract). `&region` outlives the call.
            unsafe {
                (self.fns.cmd_copy_buffer)(cmd, scene.light_staging.buffer, scene.light_table.buffer, 1, &region);
            }
        }

        // VB-P1e H0: open the CullReset bracket BEFORE the cull's `if let` gate, so it is
        // written EVERY bench-armed frame REGARDLESS of whether the froxel arm itself is
        // boot-built (`scene.cluster_cull.is_none()` on a flat-leg bench boot ⇒ a near-zero-
        // width bracket with no GPU work between begin/end) — the `VK_QUERY_RESULT_WAIT_BIT`
        // readback (`GpuSceneBundles::read_vb_bench_ns`) never blocks on an unwritten query this
        // way, whichever leg (flat vs froxel) this boot resolved. GATED — `None` records
        // nothing. This splits VB-P1d's single `LightCull` bracket into `CullReset` (the
        // alloc-counter fill + its graph-derived TRANSFER→COMPUTE barrier) and `CullDispatch`
        // (the dispatch alone, below), so §1.2's "~13.9 us fixed cost is fill+barrier"
        // hypothesis can be attributed instead of assumed (VB-P1E-HIERARCHICAL-CULL-PLAN.md
        // §8.5, H0). Each of the two new brackets duplicates the cull-wired gate check below
        // rather than sharing one `if let` across both: the HANG WARNING requires BOTH pairs'
        // begin/end to sit outside their OWN gate, exactly as this bracket already did for
        // `LightCull` — a single gate shared between `CullReset`'s end and `CullDispatch`'s
        // begin would leave both unwritten on a flat-leg boot and deadlock the
        // `VK_QUERY_RESULT_WAIT_BIT` readback.
        if let Some(tc) = scene.vb_gpu_timing {
            // SAFETY: recording is open; `self.fns` is the live device fn-table; the pool was
            // reset at the frame top; this write is outside any rendering scope; `fi` is this
            // present's in-flight slot.
            unsafe { tc.write_begin(self.fns, cmd, fi, VbTimedPass::CullReset) };
        }
        // === VB-P1a ("dark infra"): the L1 clustered froxel light-cull RESET — byte-for-byte
        // port of `record_forward`'s own `light_cull` fill+barrier. Recorded ONLY when
        // `scene.cluster_cull.is_some()` (⚠️ default-OFF via the owner's
        // `LightingConfig::clusters_enabled`, NOT hardcoded off — this block does not record on an
        // unarmed boot, and does record on `vb_mesh_froxel`'s) AND the scene wires the cull set
        // (the SAME "4-buffers-Some" gate
        // `declare_vb_graph` uses). ===
        if let (Some(_cull_pipeline), Some(_cull_set), Some(_grid), Some(_index), Some(alloc)) = (
            scene.cluster_cull,
            targets.cull_set.as_ref().map(|s| &s[fi]),
            scene.cluster_grid,
            scene.light_index,
            scene.light_index_alloc,
        ) {
            // (L1-0) Reset the global slice-allocation counter to 0 (a transfer fill), then
            // order the fill before the cull's atomic reads/writes (TRANSFER→COMPUTE). See
            // `record_forward`'s own L1-0 comment for the full rationale.
            // SAFETY: recording is open; `alloc` is a live device-local STORAGE buffer (≥ 4 B,
            // the single u32 counter); `cmd_fill_buffer` zero-fills it (Vulkan 1.0 core). The
            // FILL is GPU work (not a barrier), so it runs unconditionally — only the following
            // barrier is graph-driven.
            unsafe {
                (self.fns.cmd_fill_buffer)(cmd, alloc.buffer, 0, VK_WHOLE_SIZE, 0);
            }
            let light_cull = plan
                .light_cull
                .expect("invariant: cull wired ⇒ light_cull pass declared");
            // SAFETY: recording is open; `record_vb_pass` records the graph's derived
            // TRANSFER→COMPUTE barrier for the "light_cull" pass into `cmd`, ordering the fill's
            // TRANSFER write before the cull's COMPUTE atomics on the GPU timeline.
            self.record_vb_pass(light_cull, cmd, targets, forward, vb, scene, fi);
        }
        // VB-P1e H0: close the CullReset bracket. GATED.
        if let Some(tc) = scene.vb_gpu_timing {
            // SAFETY: recording is open; the pool was reset this frame; `fi` is this slot.
            unsafe { tc.write_end(self.fns, cmd, fi, VbTimedPass::CullReset) };
        }

        // VB-P1e H0: open the CullDispatch bracket — same unconditional-write shape as
        // `CullReset` above, and for the same hang-avoidance reason.
        if let Some(tc) = scene.vb_gpu_timing {
            // SAFETY: recording is open; `self.fns` is the live device fn-table; the pool was
            // reset at the frame top; this write is outside any rendering scope; `fi` is this
            // present's in-flight slot.
            unsafe { tc.write_begin(self.fns, cmd, fi, VbTimedPass::CullDispatch) };
        }
        if let (Some(cull_pipeline), Some(cull_set), Some(_grid), Some(_index), Some(_alloc)) = (
            scene.cluster_cull,
            targets.cull_set.as_ref().map(|s| &s[fi]),
            scene.cluster_grid,
            scene.light_index,
            scene.light_index_alloc,
        ) {
            // (L1-1) Bind the cull pipeline + the cull set (written ONCE at sync_gbuffer), push
            // this arm's own push image, dispatch this arm's own group count (VB-P1e D11/H4):
            // base = `cluster_count` froxels at the 64-wide group + the 16-byte
            // `ClusterCullPush`; hier = `h.groups` groups of 256 + the 24-byte
            // `ClusterCullHierPush`. `scene.cluster_cull_hier` selects BOTH halves together, so
            // the group count can never be paired with the other arm's push range.
            let (cull_groups, push_ptr, push_len) = match scene.cluster_cull_hier.as_ref() {
                Some(h) => (h.groups, h.push.as_ptr(), CLUSTER_CULL_HIER_PUSH_BYTES),
                None => (
                    scene.cluster_count.div_ceil(LIGHT_CULL_LOCAL_SIZE_X),
                    scene.cluster_cull_push.as_ptr(),
                    CLUSTER_CULL_PUSH_BYTES,
                ),
            };
            // SAFETY: recording is open; the cull pipeline + its layout (declaring `cull_layout`
            // at set 0 + a COMPUTE push range sized for the SAME arm) are live on this device
            // (caller contract); the cull set binds the camera UBO + light table + the cluster
            // buffers; the dispatch size and the push image are the SAME `Option` arm (base:
            // `cluster_count` froxels at the 64-wide group + the 16-byte `ClusterCullPush`;
            // hier: `h.groups` groups of 256 + the 24-byte `ClusterCullHierPush`), so the group
            // count can never be paired with the other arm's push range.
            //
            // The two arms' `ClusterGrid[fi]` write bounds are DIFFERENT quantities (P0-1,
            // adversarial review — the two must not be conflated), but as of VB-P1j both are
            // hard-bounded by the ALLOCATION, so neither can write past the end of `ClusterGrid`
            // under any boot/live `ClusterConfig` skew. HIER: `cluster_cull.hlsl`'s `#ifdef HIER`
            // branch guards on `fi < pc.cluster_capacity`, a pushed BOOT-snapshot word minted by
            // `build_froxel_light_cull` from the SAME `ClusterConfig::cluster_count()` binding
            // the `ClusterGrid` buffer itself was allocated from (`gpu_scene/mod.rs`) — a live
            // edit to the `ClusterConfig` Resource cannot move this arm's own write bound, by
            // construction (D11). BASE: the `#else` branch still carries NO `cluster_capacity`
            // push word (its push stays 16 B / 4 words — `z_near`, `z_far`,
            // `max_lights_per_cluster`, `index_list_cap`; VB-P1j deliberately did NOT widen it);
            // it clamps `cluster_count` by `ClusterGrid.GetDimensions()` instead, i.e. by the
            // bound DESCRIPTOR's own element count (SPIR-V `OpArrayLength`). That is the
            // allocation itself rather than a host-side mirror of it, so this arm's bound cannot
            // drift from the buffer even if a push word or a boot snapshot were wrong. Before
            // VB-P1j it bounded only on the LIVE header's `dim_x*dim_y*dim_z`, reaching
            // `min(64*ceil(boot_cc/64), live_cc)` — measured at 16 cells / 128 B past the end
            // for boot 16x9x23 vs live 16x9x24.
            // SCOPE: this bounds THIS dispatch's writes only. The `ClusterGrid` *readers*
            // (vb_resolve/vb_shade/deferred_pbr/forward_opaque) are a separate contract, closed
            // by VB-P1k in the same commit: each disarms its cluster walk (falling back to the
            // in-bounds flat light scan) unless the live grid fits that same `GetDimensions()`
            // bound.
            // `&cull_set.descriptor_set` is a single-element local alive for the call
            // (first_set 0, count 1).
            unsafe {
                (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, cull_pipeline.pipeline);
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    cull_pipeline.layout,
                    0,
                    1,
                    &cull_set.descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_push_constants)(
                    cmd,
                    cull_pipeline.layout,
                    VK_SHADER_STAGE_COMPUTE_BIT,
                    0,
                    push_len,
                    push_ptr.cast(),
                );
                (self.fns.cmd_dispatch)(cmd, cull_groups, 1, 1);
            }
            // (L1-2) The cull's ClusterGrid + LightIndexList writes are made available + visible
            // to `vb_resolve`/`vb_shade`'s reads by the graph: derived at the reader — NOT here.
        }
        // VB-P1e H0: close the CullDispatch bracket. GATED.
        if let Some(tc) = scene.vb_gpu_timing {
            // SAFETY: recording is open; the pool was reset this frame; `fi` is this slot.
            unsafe { tc.write_end(self.fns, cmd, fi, VbTimedPass::CullDispatch) };
        }

        // === CSM cascade DEPTH pass — byte-for-byte port of `record_forward`'s own `csm` block.
        // Recorded ONLY when `scene.csm.is_some()`; runs BEFORE `vb_resolve` (which samples the
        // cascade inline). ===
        if let Some(csm) = &scene.csm {
            let cascade = scene.csm_cascade_texture;
            let active = (csm.active_count as usize).clamp(1, MAX_CASCADES) as u32;
            let csm_pass = plan.csm.expect("invariant: scene.csm.is_some() ⇒ csm pass declared");
            // SAFETY: recording is open; `record_vb_pass` records the graph's derived
            // UNDEFINED→DEPTH barrier-in for the "csm_depth" pass into `cmd`.
            self.record_vb_pass(csm_pass, cmd, targets, forward, vb, scene, fi);

            let cascade_extent = VkExtent2D { width: csm.shadow_dim, height: csm.shadow_dim };
            let csm_area = VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent: cascade_extent };
            let csm_viewport = VkViewport {
                x: 0.0,
                y: 0.0,
                width: cascade_extent.width as f32,
                height: cascade_extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            let mut csm_push = csm.push;
            for c in 0..active {
                csm_push[0..64].copy_from_slice(&csm.cascade_view_proj[c as usize]);
                let csm_depth_attachment = VkRenderingAttachmentInfo {
                    s_type: VkStructureType::RenderingAttachmentInfo,
                    p_next: ptr::null(),
                    image_view: cascade.layer_render_view(c),
                    image_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                    resolve_mode: 0,
                    resolve_image_view: VkImageView::NULL,
                    resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                    load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
                    store_op: VK_ATTACHMENT_STORE_OP_STORE,
                    clear_value: VkClearValue {
                        depth_stencil: VkClearDepthStencilValue { depth: 1.0, stencil: 0 },
                    },
                };
                let csm_rendering = VkRenderingInfo {
                    s_type: VkStructureType::RenderingInfo,
                    p_next: ptr::null(),
                    flags: 0,
                    render_area: csm_area,
                    layer_count: 1,
                    view_mask: 0,
                    color_attachment_count: 0,
                    p_color_attachments: ptr::null(),
                    p_depth_attachment: (&csm_depth_attachment as *const VkRenderingAttachmentInfo).cast(),
                    p_stencil_attachment: ptr::null(),
                };
                // SAFETY: recording is open; `csm_rendering` names the live cascade layer-`c`
                // render view (now DEPTH_ATTACHMENT_OPTIMAL; `c < active <= MAX_CASCADES`),
                // depth-only; the depth-only pipeline + the SAME instance SSBO
                // (`scene.instance_bind_group`) satisfy the depth VS's static `instances`
                // reference; the 88-byte push carries cascade `c`'s `view_proj` +
                // `use_model_matrix == 1`; per caster batch the recorder re-pushes
                // `base_instance` then `draw_indexed` reads that batch's bound vertex+index
                // buffers. Begin/End bracket each cascade.
                unsafe {
                    (self.fns.cmd_begin_rendering)(cmd, &csm_rendering);
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, csm.pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_GRAPHICS,
                        csm.pipeline.layout,
                        0,
                        1,
                        &scene.instance_bind_group.descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_push_constants)(
                        cmd,
                        csm.pipeline.layout,
                        VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
                        0,
                        csm_push.len() as u32,
                        csm_push.as_ptr().cast(),
                    );
                    (self.fns.cmd_set_viewport)(cmd, 0, 1, &csm_viewport);
                    (self.fns.cmd_set_scissor)(cmd, 0, 1, &csm_area);
                    for batch in scene.mesh_draw {
                        if !batch.casts_shadow {
                            continue;
                        }
                        let base = batch.base_instance;
                        csm_push[GBUFFER_PUSH_BASE_INSTANCE_OFFSET as usize
                            ..GBUFFER_PUSH_BASE_INSTANCE_OFFSET as usize + 4]
                            .copy_from_slice(&base.to_le_bytes());
                        (self.fns.cmd_push_constants)(
                            cmd,
                            csm.pipeline.layout,
                            VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
                            GBUFFER_PUSH_BASE_INSTANCE_OFFSET,
                            4,
                            (&base as *const u32).cast(),
                        );
                        (self.fns.cmd_bind_vertex_buffers)(cmd, 0, 1, &batch.vertex_buffer.buffer, &vertex_offset);
                        (self.fns.cmd_bind_index_buffer)(cmd, batch.index_buffer.buffer, 0, batch.index_type);
                        (self.fns.cmd_draw_indexed)(cmd, batch.index_count, batch.instance_count, 0, 0, 0);
                    }
                    (self.fns.cmd_end_rendering)(cmd);
                }
            }
        }

        // === Punctual (spot/point) shadow-atlas DEPTH pass — byte-for-byte port of
        // `record_forward`'s own `atlas` block. Recorded ONLY when `scene.atlas_punctual.is_some()`.
        // ===
        if let Some(atlas_act) = &scene.atlas_punctual {
            let atlas = scene.shadow_atlas_texture;
            let active = (atlas_act.active_layers as usize).clamp(1, MAX_TEXTURE_LAYERS) as u32;
            let atlas_pass =
                plan.atlas.expect("invariant: scene.atlas_punctual.is_some() ⇒ atlas pass declared");
            // SAFETY: recording is open; `record_vb_pass` records the graph's derived
            // UNDEFINED→DEPTH barrier-in for the "atlas_depth" pass into `cmd`.
            self.record_vb_pass(atlas_pass, cmd, targets, forward, vb, scene, fi);

            let atlas_extent = VkExtent2D { width: atlas_act.shadow_dim, height: atlas_act.shadow_dim };
            let atlas_area = VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent: atlas_extent };
            let atlas_viewport = VkViewport {
                x: 0.0,
                y: 0.0,
                width: atlas_extent.width as f32,
                height: atlas_extent.height as f32,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            let mut atlas_push = atlas_act.push;
            let mut bound_point: Option<bool> = None;
            for s in 0..active {
                let is_point = atlas_act.face_is_point[s as usize];
                let face_pipeline = if is_point { atlas_act.point_pipeline } else { atlas_act.pipeline };
                atlas_push[0..64].copy_from_slice(&atlas_act.face_view_proj[s as usize]);
                if is_point {
                    atlas_push[64..80].copy_from_slice(&atlas_act.face_light[s as usize]);
                }
                let atlas_depth_attachment = VkRenderingAttachmentInfo {
                    s_type: VkStructureType::RenderingAttachmentInfo,
                    p_next: ptr::null(),
                    image_view: atlas.layer_render_view(s),
                    image_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                    resolve_mode: 0,
                    resolve_image_view: VkImageView::NULL,
                    resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                    load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
                    store_op: VK_ATTACHMENT_STORE_OP_STORE,
                    clear_value: VkClearValue {
                        depth_stencil: VkClearDepthStencilValue { depth: 1.0, stencil: 0 },
                    },
                };
                let atlas_rendering = VkRenderingInfo {
                    s_type: VkStructureType::RenderingInfo,
                    p_next: ptr::null(),
                    flags: 0,
                    render_area: atlas_area,
                    layer_count: 1,
                    view_mask: 0,
                    color_attachment_count: 0,
                    p_color_attachments: ptr::null(),
                    p_depth_attachment: (&atlas_depth_attachment as *const VkRenderingAttachmentInfo).cast(),
                    p_stencil_attachment: ptr::null(),
                };
                // SAFETY: recording is open; `atlas_rendering` names the live atlas layer-`s`
                // render view (now DEPTH_ATTACHMENT_OPTIMAL; `s < active <= MAX_TEXTURE_LAYERS`),
                // depth-only; the selected SPOT/POINT depth-only pipeline shares the SAME layout
                // as the SAME instance SSBO set 0; the 88-byte push carries slot `s`'s
                // `view_proj` (+ the POINT `cam_eye` lane) + `use_model_matrix == 1`; per caster
                // batch the recorder re-pushes `base_instance` then `draw_indexed` reads that
                // batch's bound vertex+index buffers. Begin/End bracket each slot.
                unsafe {
                    (self.fns.cmd_begin_rendering)(cmd, &atlas_rendering);
                    if bound_point != Some(is_point) {
                        (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, face_pipeline.pipeline);
                        (self.fns.cmd_bind_descriptor_sets)(
                            cmd,
                            VK_PIPELINE_BIND_POINT_GRAPHICS,
                            face_pipeline.layout,
                            0,
                            1,
                            &scene.instance_bind_group.descriptor_set,
                            0,
                            ptr::null(),
                        );
                        bound_point = Some(is_point);
                    }
                    (self.fns.cmd_push_constants)(
                        cmd,
                        face_pipeline.layout,
                        VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
                        0,
                        atlas_push.len() as u32,
                        atlas_push.as_ptr().cast(),
                    );
                    (self.fns.cmd_set_viewport)(cmd, 0, 1, &atlas_viewport);
                    (self.fns.cmd_set_scissor)(cmd, 0, 1, &atlas_area);
                    for batch in scene.mesh_draw {
                        if !batch.casts_shadow {
                            continue;
                        }
                        let base = batch.base_instance;
                        atlas_push[GBUFFER_PUSH_BASE_INSTANCE_OFFSET as usize
                            ..GBUFFER_PUSH_BASE_INSTANCE_OFFSET as usize + 4]
                            .copy_from_slice(&base.to_le_bytes());
                        (self.fns.cmd_push_constants)(
                            cmd,
                            face_pipeline.layout,
                            VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
                            GBUFFER_PUSH_BASE_INSTANCE_OFFSET,
                            4,
                            (&base as *const u32).cast(),
                        );
                        (self.fns.cmd_bind_vertex_buffers)(cmd, 0, 1, &batch.vertex_buffer.buffer, &vertex_offset);
                        (self.fns.cmd_bind_index_buffer)(cmd, batch.index_buffer.buffer, 0, batch.index_type);
                        (self.fns.cmd_draw_indexed)(cmd, batch.index_count, batch.instance_count, 0, 0, 0);
                    }
                    (self.fns.cmd_end_rendering)(cmd);
                }
            }
        }

        // === Pass `vb_sky`: the sky background pass — writes `lit` (COLOR, first-touch). ===
        // SAFETY: recording is open; `record_vb_pass` records the graph's derived
        // UNDEFINED→COLOR_ATTACHMENT_OPTIMAL barrier-in for `lit` (+ the cascade/atlas
        // →SHADER_READ_ONLY_OPTIMAL barriers-out, this pass's declared FRAGMENT reader are
        // actually consumed by `vb_resolve` below, not this pass — see `declare_vb_graph`) for
        // the "vb_sky" pass into `cmd`.
        self.record_vb_pass(plan.vb_sky, cmd, targets, forward, vb, scene, fi);

        let vb_sky_pipeline =
            scene.vb_sky_pipeline.expect("invariant: a VisibilityBuffer-resolved scene always carries vb_sky_pipeline");
        let vb_set0 =
            targets.vb_set0.as_ref().expect("invariant: TargetsProfile::VbMesh ⇒ targets.vb_set0 is built");

        let lit_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: targets.lit[fi].view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue { color: VkClearColorValue { float32: VB_LIT_CLEAR } },
        };
        let sky_rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent: present_extent },
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: 1,
            p_color_attachments: &lit_attachment,
            p_depth_attachment: ptr::null(),
            p_stencil_attachment: ptr::null(),
        };
        let full_viewport = VkViewport {
            x: 0.0,
            y: 0.0,
            width: present_extent.width as f32,
            height: present_extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let full_area = VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent: present_extent };
        // SAFETY: recording is open; `sky_rendering` names the live `lit` view (now
        // COLOR_ATTACHMENT_OPTIMAL); `vb_sky_pipeline` (REUSING the compiled `forward_sky`
        // SPIR-V verbatim against a NEW pipeline object built for `vb_layout0` — declares NO
        // depth attachment, `GBufferScene::vb_sky_pipeline`'s doc); `vb_set0[fi]` is a live
        // descriptor set written once per extent (the sky FS reads only its `Camera`/`LightBuf`
        // subset — the SAME bound-but-unread-subset idiom `forward_sky_pipeline` establishes).
        // `draw(3, 1, 0, 0)` is the `SV_VertexID` fullscreen triangle (no vertex buffer). Begin/
        // End bracket the pass exactly.
        unsafe {
            (self.fns.cmd_begin_rendering)(cmd, &sky_rendering);
            (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, vb_sky_pipeline.pipeline);
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                vb_sky_pipeline.layout,
                0,
                1,
                &vb_set0[fi].descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_set_viewport)(cmd, 0, 1, &full_viewport);
            (self.fns.cmd_set_scissor)(cmd, 0, 1, &full_area);
            (self.fns.cmd_draw)(cmd, 3, 1, 0, 0);
            (self.fns.cmd_end_rendering)(cmd);
        }

        // === Passes `vb_raster` + `vb_resolve` — rung R10: recorded ONLY under `mesh_leg`
        // (a `VisibilityBuffer x Sdf` frame skips both — the Decision-0 geometry table they
        // re-fetch through carries no slot with no mesh leg; see `declare_vb_graph`'s matching
        // gate). `vb_sky`'s `lit` write above stands for the sky; `sdf_forward_march` below
        // composites the SDF field over whatever the mesh raster/resolve left. ===
        if scene.resolved_render_path.mesh_leg {
            // VG rung R2c0: the batch-cull arm — the SAME `Option` predicate `declare_vb_graph`
            // reads, spelled once here so the recorder cannot drift from the declarator. See the
            // dispatch block below for why a narrower recorder gate would be a missing barrier
            // rather than merely a skipped optimisation.
            //
            // VG rung R2d-2 added `vb_mesh_bounds`, and it is the conjunct that can actually be
            // false: `GBufferTargets::sync` cannot build `vb_cull_set` without a geometry table to
            // bind at `vb_cull_layout` @5, so this predicate must go false on exactly the boots
            // where that set goes `None` — otherwise the `.expect()` on it below is reachable.
            let batch_cull_armed = scene.vb_indirect.is_some()
                && scene.vb_batch_desc.is_some()
                && scene.vb_cull_visible.is_some()
                && scene.vb_cull_count.is_some()
                && scene.vb_mesh_bounds.is_some()
                && scene.vb_batch_cull_pipeline.is_some();

            // === Pass `vb_raster`: the mesh id-raster pass (Decision 9) — writes `vb_id` (COLOR,
            // R32G32_UINT) + `vb_depth` (DEPTH, HW reverse-Z, first-touch `GREATER`, write ON). ===
            // === Rung R2a': fill this frame's indirect draw records, BEFORE any render scope. ===
            //
            // `vkCmdUpdateBuffer` is forbidden inside a render pass instance
            // (VUID-vkCmdUpdateBuffer-renderpass), so it lands here rather than beside the draws it
            // feeds. Written in CHUNKS: a single update would need the whole record array on the
            // stack (a cap, and 20 KiB of it), while chunking bounds the stack at one chunk and
            // removes the cap entirely. Each chunk's byte offset is `i * 20`, a multiple of 4, so
            // every update satisfies `VUID-vkCmdUpdateBuffer-dstOffset-00036`.
            // The record array's capacity, taken from the ALLOCATION rather than from the host
            // constant that sized it — the VB-P1j lesson, where a capacity carried as a separate
            // word had drifted from the buffer it claimed to describe and nothing detected it.
            // Hoisted above the fill because the DRAW LOOP needs the same bound: a batch with no
            // record must keep its direct draw rather than fetch past the end of the buffer.
            //
            // `mesh_draw.len() <= record_capacity` holds today by an ARGUMENT (batches ≤ instances,
            // since every batch holds at least one instance, and both arrays are sized to
            // `INSTANCE_CAPACITY`) — not by a check. Rung R2a''s SAFETY comment cited "the debug
            // assert on the draw loop below" for this bound and NO SUCH ASSERT EXISTED. Here it is,
            // with a release-side clamp behind it: an indirect fetch past the end of the allocation
            // would be an out-of-bounds device read that nothing in this repository detects
            // (`robustBufferAccess` is off, and the validation layers do not follow buffer
            // contents), whereas falling back to the direct draw renders the same image.
            let record_capacity = scene
                .vb_indirect
                .map_or(0, |r| (r[fi].size / u64::from(DRAW_INDEXED_INDIRECT_STRIDE)) as usize);
            debug_assert!(
                scene.vb_indirect.is_none() || scene.mesh_draw.len() <= record_capacity,
                "invariant: {} draw batches exceed the {record_capacity}-record indirect allocation",
                scene.mesh_draw.len()
            );

            // === VG rung R2d-3: the cull's batch count, HOISTED above BOTH fills. ===
            //
            // It used to be computed at the dispatch, AFTER the descriptor fill had already chosen
            // its own bound. Now the descriptor fill and the dispatch read the ONE number, so a
            // lane can never be dispatched over a descriptor that was never written (the W1
            // single-value discipline this file already applies to `batch_cull_armed`).
            //
            // Every bound is ALLOCATION-derived, none is a host constant: the record array, the
            // descriptor array, and — new this rung — the per-instance survivor list, whose element
            // count is its own `size / 4`. See `vb_cull_batch_count_visible_clamp`'s doc for why
            // clamping on that last one is a PREFIX (bases are strictly ascending, so the predicate
            // is monotone) and therefore cannot drop a batch out of the middle of the list.
            let desc_capacity = scene
                .vb_batch_desc
                .map_or(0, |d| (d[fi].size / u64::from(VB_BATCH_DESC_STRIDE)) as usize);
            let visible_elems =
                scene.vb_visible_instance.map_or(0, |v| (v[fi].size / 4) as usize);
            // === VG R3 piece 3 step P3-2 (plan D3): the LATE list's size BACKSTOP. ===
            //
            // ASSERTED, never folded into the `.min()` chain above, and the reason is the one
            // `record_capacity_late`'s own assert states for the late RECORD array: a late list
            // SHORTER than the early one would silently drop the tail batches from the late scope,
            // and the fix for that is the build-time const-assert that pins the two ELEMENT COUNTS
            // equal (`boyko_app`'s `VB_LATE_VISIBLE_ELEMS == VB_VISIBLE_INSTANCE_ELEMS`), not a
            // runtime clamp that would make the drop quiet and legal. `.min()`-ing it here would
            // ALSO change `batch_count` on a frame where the two allocations disagreed — i.e. it
            // would silently repair the wiring bug this is here to expose.
            //
            // Derived from the ALLOCATION on both sides (the VB-P1j lesson), so it compares the
            // buffers rather than the constants that were meant to size them.
            debug_assert!(
                scene.vb_late_visible.is_none_or(|l| (l[fi].size / 4) as usize >= visible_elems),
                "invariant: the late candidate/survivor list holds every index the early survivor \
                 list can ({} late elems against {visible_elems} early)",
                scene.vb_late_visible.map_or(0, |l| l[fi].size / 4)
            );
            let batch_count = scene
                .mesh_draw
                .len()
                .min(record_capacity)
                .min(desc_capacity)
                .min(vb_cull_batch_count_visible_clamp(scene.mesh_draw, visible_elems));

            if let Some(indirect) = scene.vb_indirect {
                let upload = plan
                    .vb_indirect_upload
                    .expect("invariant: mesh_leg + vb_indirect => vb_indirect_upload pass declared");
                // SAFETY: recording is open; `record_vb_pass` records the graph's derived
                // barriers-in for the "vb_indirect_upload" pass (a first touch of a frame-private
                // buffer, so typically none) into `cmd`.
                self.record_vb_pass(upload, cmd, targets, forward, vb, scene, fi);

                const CHUNK: usize = 64;
                let stride = u64::from(DRAW_INDEXED_INDIRECT_STRIDE);
                let recorded = &scene.mesh_draw[..scene.mesh_draw.len().min(record_capacity)];
                for (c, batches) in recorded.chunks(CHUNK).enumerate() {
                    let mut records = [VkDrawIndexedIndirectCommand::default(); CHUNK];
                    for (r, batch) in records.iter_mut().zip(batches) {
                        *r = VkDrawIndexedIndirectCommand {
                            index_count: batch.index_count,
                            instance_count: batch.instance_count,
                            first_index: 0,
                            vertex_offset: 0,
                            // ⚠️ MUST stay 0: `drawIndirectFirstInstance` is not enabled on this
                            // device, and the validation layers cannot read buffer CONTENTS, so a
                            // nonzero value here is silent corruption rather than a caught error.
                            // The VS reads `instances[pc.base_instance + SV_InstanceID]`, so the
                            // base travels in the push constant exactly as it did before this rung.
                            //
                            // ⚠️ Rung R2d-4 RAISED THE STAKES on this field. `SV_InstanceID` now
                            // also indexes `gVbVisibleInstance`, whose written region per batch is
                            // exactly `[base_instance, base_instance + instance_count)`. If a
                            // later rung enables the feature and writes a nonzero value here, an
                            // id shifted by it stops being a wrong transform and becomes an
                            // OUT-OF-RANGE read of that SSBO, undefined with `robustBufferAccess`
                            // off. `vb_raster.vs.hlsl`'s header states the same warning from the
                            // shader side and names the `.spv` census that pins the lowering.
                            first_instance: 0,
                        };
                    }
                    debug_assert!(
                        records.iter().all(|r| r.first_instance == 0),
                        "invariant: drawIndirectFirstInstance is VK_FALSE on this device"
                    );
                    let bytes = (batches.len() as u64) * stride;
                    // SAFETY: recording is open and NO render-pass instance is active (the raster's
                    // `cmd_begin_rendering` is below); `indirect[fi]` is a live DEVICE_LOCAL buffer
                    // created with TRANSFER_DST at `INSTANCE_CAPACITY` records, and this write ends
                    // at `(c * CHUNK + batches.len()) * stride`, which the debug assert on the draw
                    // loop below bounds; `records` outlives the call; `bytes` is a multiple of 4 and
                    // at most `CHUNK * 20` = 1280, inside the 65536-byte inline limit.
                    unsafe {
                        (self.fns.cmd_update_buffer)(
                            cmd,
                            indirect[fi].buffer,
                            (c * CHUNK) as u64 * stride,
                            bytes,
                            records.as_ptr().cast(),
                        );
                    }
                }

                // === Rung R2c0: the batch-cull DESCRIPTORS, filled in the SAME transfer pass. ===
                //
                // One 32-byte `VbBatchDesc` per batch, chunked exactly like the records above (64
                // descriptors = 2048 bytes per update, inside the 65536-byte inline limit; every
                // offset is a multiple of 32 and therefore of 4).
                //
                // `instance_count` is the SAME word the record above already carries. That is the
                // rung's whole point: the cull rewrites the record with a value the host had
                // already written, so the frame is byte-identical and the machinery is a NULL
                // CONTROL rather than a change. A batch whose AABB the host could NOT compute
                // keeps the conservative `VbBatchDesc::UNBOUNDED` corners, which survive every
                // plane of the test rung R2c armed — so the reachable error is a wasted draw,
                // never a vanished object.
                if let Some(desc) = scene.vb_batch_desc.filter(|_| batch_cull_armed) {
                    let desc_stride = u64::from(VB_BATCH_DESC_STRIDE);
                    // Rung R2d-3: the SAME `batch_count` the dispatch below covers — bounded by
                    // the record, descriptor AND survivor-list allocations at one site above, so a
                    // dispatched lane can never read a descriptor this loop skipped.
                    let described = &recorded[..batch_count.min(recorded.len())];
                    for (c, batches) in described.chunks(CHUNK).enumerate() {
                        let mut descs = [VbBatchDesc::unbounded(0, 0); CHUNK];
                        for (d, batch) in descs.iter_mut().zip(batches) {
                            // Rung R2c: the batch's real world AABB when the host could compute
                            // one. `None` (mesh not `Loaded`, or the C0 zero-vertex sentinel)
                            // falls back to the UNBOUNDED corners, which survive every plane —
                            // absence of bounds is not evidence of invisibility, and the fallback
                            // is conservative by construction rather than by a branch in the
                            // shader.
                            //
                            // Rung R2d-3: `base_instance` is the SAME prefix-sum offset the raster
                            // pushes per batch, so the shader's survivor region and the VS's
                            // instance bucket are keyed off one host number.
                            *d = match batch.world_aabb {
                                Some((mn, mx)) => VbBatchDesc::bounded(
                                    batch.instance_count,
                                    batch.base_instance,
                                    mn,
                                    mx,
                                ),
                                None => VbBatchDesc::unbounded(
                                    batch.instance_count,
                                    batch.base_instance,
                                ),
                            };
                        }
                        let bytes = (batches.len() as u64) * desc_stride;
                        // SAFETY: recording is open and NO render-pass instance is active (the
                        // raster's `cmd_begin_rendering` is below); `desc[fi]` is a live
                        // DEVICE_LOCAL buffer created with TRANSFER_DST at `INSTANCE_CAPACITY`
                        // descriptors, and this write ends at `(c * CHUNK + batches.len()) *
                        // desc_stride`, which the draw loop's own batch-count assert bounds;
                        // `descs` outlives the call; `bytes` is a multiple of 4 and at most
                        // `CHUNK * 32` = 2048, inside the 65536-byte inline limit.
                        unsafe {
                            (self.fns.cmd_update_buffer)(
                                cmd,
                                desc[fi].buffer,
                                (c * CHUNK) as u64 * desc_stride,
                                bytes,
                                descs.as_ptr().cast(),
                            );
                        }
                    }
                }
            }

            // === VG R3 piece 2 step P2-5 (plan D4): the LATE indirect record fill. ===
            //
            // Recorded HERE — immediately after the early fill and before the cull dispatch, which
            // is EXACTLY `declare_vb_graph`'s position for `vb_indirect_late_upload` (declare/record
            // ORDER parity). Its gate is `path_vb_occlusion_split()` and NOTHING else:
            // `scene.vb_indirect_late` is minted unconditionally on every VB boot, so an extra
            // `is_some()` conjunct would be a dead one — and a recorder gate NARROWER than the
            // declarator's would leave the declared TRANSFER write on a buffer nothing wrote, i.e.
            // the late fetch's dependency derived against a transfer that never happened.
            //
            // THE RECORDS ARE REAL EXCEPT FOR ONE WORD. Each carries the early record's true
            // `index_count`, `first_index`, `vertex_offset` and `first_instance: 0`; only
            // `instance_count` is the inert `0`. An all-zero record would be a placeholder, and
            // piece 3 would then be adding structure rather than flipping a producer — the
            // shipped-inert discipline `vb_batch_cull.comp.hlsl` states for its own two rungs.
            if occlusion_split {
                let late = scene.vb_indirect_late.expect(
                    "invariant: path_vb_occlusion_split ⇒ vb_indirect_late (GpuSceneBundles::boot \
                     mints it unconditionally on every VB boot)",
                );
                let late_upload = plan
                    .vb_indirect_late_upload
                    .expect("invariant: the recorder's late-upload gate is the declarator's, verbatim");
                // SAFETY: recording is open; `record_vb_pass` records the graph's derived
                // barriers-in for the "vb_indirect_late_upload" pass (a first touch of a
                // frame-private buffer, so typically none) into `cmd`, ahead of the updates below.
                self.record_vb_pass(late_upload, cmd, targets, forward, vb, scene, fi);

                const CHUNK: usize = 64;
                let stride = u64::from(DRAW_INDEXED_INDIRECT_STRIDE);
                // The array's capacity from the ALLOCATION, never from the host constant that
                // sized it (the VB-P1j lesson). It bounds nothing that `batch_count` does not
                // already bound — it is asserted, not min-ed, because a late array SHORTER than
                // the early one would silently drop the tail batches from the late scope, and D5's
                // build-time const-assert on the two capacities is what actually prevents that.
                let record_capacity_late = (late[fi].size / stride) as usize;
                debug_assert!(
                    record_capacity_late >= batch_count,
                    "invariant: the late record array holds every batch the late scope draws \
                     ({batch_count} batches into {record_capacity_late} records)"
                );
                // VG R3 piece 3 step P3-2 (plan D3): the second size BACKSTOP — `vb_late_count`
                // carries one `u32` per late RECORD plus one reserved tail slot for the frame index
                // the GPU observed. Compared against the record array beside it rather than against
                // a host constant, for the same VB-P1j reason: this is the pair that must agree,
                // and both numbers come from the allocations. Asserted, not clamped — a short count
                // array is a wiring bug, and clamping would let a batch report a deferral count no
                // slot exists for.
                debug_assert!(
                    scene
                        .vb_late_count
                        .is_none_or(|c| (c[fi].size / 4) as usize > record_capacity_late),
                    "invariant: vb_late_count holds one u32 per late record plus the reserved \
                     frame slot ({} elems against {record_capacity_late} records)",
                    scene.vb_late_count.map_or(0, |c| c[fi].size / 4)
                );

                // ⚠️ BOUNDED BY `batch_count`, the SAME hoisted local the early fill and the cull
                // dispatch read — NOT by `record_capacity_late`. The record array is a PREFIX, not
                // a mask: the late scope records exactly as many empty draws as the early scope
                // records real ones, never the full 1024-record allocation.
                let late_batches = &scene.mesh_draw[..batch_count.min(scene.mesh_draw.len())];
                for (c, batches) in late_batches.chunks(CHUNK).enumerate() {
                    let mut records = [VkDrawIndexedIndirectCommand::default(); CHUNK];
                    for (r, batch) in records.iter_mut().zip(batches) {
                        *r = VkDrawIndexedIndirectCommand {
                            index_count: batch.index_count,
                            // ⚠️ THE ONE INERT WORD, and it is the CONSERVATIVE constant (the two
                            // earlier inert rungs shipped the permissive one) because this scope
                            // must draw NOTHING. Piece 3 replaces the producer of this word — host
                            // `0` becomes the late cull's survivor count — and deletes the
                            // `debug_assert!` below in the same change.
                            instance_count: 0,
                            first_index: 0,
                            vertex_offset: 0,
                            // ⚠️ MUST stay 0, for the SAME reason the early fill states:
                            // `drawIndirectFirstInstance` is not enabled on this device and the
                            // validation layers cannot read buffer CONTENTS, so a nonzero value is
                            // silent corruption rather than a caught error.
                            first_instance: 0,
                        };
                    }
                    debug_assert!(
                        records.iter().all(|r| r.first_instance == 0),
                        "invariant: drawIndirectFirstInstance is VK_FALSE on this device"
                    );
                    debug_assert!(
                        records.iter().all(|r| r.instance_count == 0),
                        "invariant: PIECE 2 ONLY — the late scope draws nothing, so every late \
                         record's instanceCount is the inert 0. DELETE THIS ASSERT IN PIECE 3, \
                         deliberately, in the change that makes the late cull the producer of \
                         this word"
                    );
                    // Gate G2's `late_instances`, summed over the records this chunk WRITES —
                    // `zip(batches)` filled only the first `batches.len()` slots, and the rest of
                    // the fixed-size array is the `Default` the loop never reached. Summed from
                    // the record array rather than from the constant `0` above, so a piece-3 edit
                    // that starts writing a real count is reflected here without touching the
                    // probe.
                    if let Some(p) = probe.as_deref_mut() {
                        p.late_instances +=
                            records[..batches.len()].iter().map(|r| r.instance_count).sum::<u32>();
                    }
                    let bytes = (batches.len() as u64) * stride;
                    // SAFETY: recording is open and NO render-pass instance is active (the early
                    // raster's `cmd_begin_rendering` is below, and `vkCmdUpdateBuffer` is forbidden
                    // inside one per VUID-vkCmdUpdateBuffer-renderpass); `late[fi]` is a live
                    // DEVICE_LOCAL buffer created with TRANSFER_DST at `VB_INDIRECT_LATE_RECORDS`
                    // records, and this write ends at `(c * CHUNK + batches.len()) * stride`, which
                    // the `record_capacity_late >= batch_count` assert above bounds; `records`
                    // outlives the call; `bytes` is a multiple of 4 (the stride is 20) and at most
                    // `CHUNK * 20` = 1280, inside the 65536-byte inline limit; the destination
                    // offset `c * CHUNK * stride` is a multiple of 4, as
                    // VUID-vkCmdUpdateBuffer-dstOffset-00036 requires.
                    unsafe {
                        (self.fns.cmd_update_buffer)(
                            cmd,
                            late[fi].buffer,
                            (c * CHUNK) as u64 * stride,
                            bytes,
                            records.as_ptr().cast(),
                        );
                    }
                }
            }

            // === Rung R2c0: the per-BATCH draw-record cull dispatch. ===
            //
            // ⚠️ This gate is the DECLARATOR's, verbatim — `targets.vb_cull_set` is deliberately
            // NOT part of it, and is `.expect()`ed instead. A recorder gate NARROWER than
            // `declare_vb_graph`'s would skip the dispatch while the graph still believes a
            // COMPUTE wrote `vb_indirect` last, and the `TRANSFER → DRAW_INDIRECT` dependency the
            // upload actually needs would then be derived nowhere. That is a missing barrier, not
            // a wasted one, so the two predicates must be the same predicate. The set's presence
            // is implied: `GBufferTargets::sync` builds it under the same VB gate that produces
            // these scene buffers, and a create FAILURE returns `Err` rather than a silent `None`.
            if batch_cull_armed {
                let pipeline = scene
                    .vb_batch_cull_pipeline
                    .expect("invariant: batch_cull_armed => vb_batch_cull_pipeline");
                let count = scene.vb_cull_count.expect("invariant: batch_cull_armed => vb_cull_count");
                let visible = scene.vb_cull_visible.expect("invariant: batch_cull_armed => vb_cull_visible");
                let cull_set = &targets
                    .vb_cull_set
                    .as_ref()
                    .expect("invariant: batch_cull_armed => targets.vb_cull_set (same VB gate)")[fi];

                // === VG R3 piece 3 step P3-3 (plan D6): the cull's UNIFORM, built ONCE here. ===
                //
                // `VbCullUniform::for_frame` performs the ONE byte inversion out of the raster
                // push's column-major leading 64 bytes into the MATH-ROW form the occlusion leaf
                // will take — the same bytes `boyko_render::frustum::frustum_planes_from_push_bytes`
                // derived `scene.vb_cull_planes` from, so the cull's frustum test and its occlusion
                // test cannot end up on two different matrices (and the TAA path jitters that matrix
                // per frame, which is exactly the drift byte provenance forecloses).
                //
                // UNCONDITIONAL — armed or not, split or not. See `VbCullUniform::for_frame`'s doc
                // for each disarmed field's value and why the alternative (gating the fill) would
                // leave the shader's `levels` read on unwritten allocation contents.
                let cull_uniform = VbCullUniform::for_frame(
                    scene.mvp[0..64]
                        .try_into()
                        .expect("invariant: the raster push's leading 64 bytes are view_proj"),
                    [present_extent.width, present_extent.height],
                    scene.hzb,
                    scene.engine_frame_index,
                );
                let cull_uniform_buf = scene
                    .vb_cull_uniform
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_cull_uniform");

                // Reset the visible counter to 0 (a transfer fill), then order it before the
                // dispatch's atomics — the SAME shape the L1 cull's `light_index_alloc` reset
                // uses. The FILL is GPU work and runs unconditionally within this arm; only the
                // barrier that follows is graph-driven.
                //
                // ⚠️ VG R3 piece 3 step P3-3: THE UNIFORM UPDATE BELONGS ON THIS SIDE OF
                // `record_vb_pass`, AND THAT IS NOT A STYLE CHOICE. A pass's ENTIRE barrier set —
                // including the intra-pass `TRANSFER → COMPUTE` edge derived from its second
                // declared access on this very buffer — is emitted at ONE site, at the pass
                // boundary (`framegraph::graph`'s `PassBarrierRange` + `record::record_pass`). So
                // TRANSFER work belonging to a pass must be recorded BEFORE `record_vb_pass`, or the
                // barrier precedes the write it is supposed to order and the dispatch reads stale
                // bytes.
                //
                // ⚠️ AND THE NEIGHBOURING PASS DOES THE OPPOSITE, CORRECTLY — that is the trap.
                // `vb_indirect_late_upload` above calls `record_vb_pass` FIRST and issues its
                // `cmd_update_buffer` AFTER. Both orders are right, for opposite reasons: that
                // pass's barrier orders THE WRITE ITSELF (a WAW/WAR flush must precede it), while
                // this pass's barrier orders an INTRA-pass edge (the write must precede the
                // barrier). Two adjacent sites, opposite orders, and nothing but this comment says
                // so.
                //
                // ⚠️ NO RUNTIME ASSERT CAN CHECK THIS. `FrameGraph::pass_access_count` is private
                // and no per-pass accessor exists, so the ordering is carried by this comment and by
                // the `frame_index` control the uniform itself provides — `vb_late_count`'s reserved
                // tail slot, which the readback compares against the host's number. A fill on the
                // wrong side would make the dispatch read frame N−FRAMES_IN_FLIGHT's uniform, and on
                // a static fixture that is BIT-IDENTICAL: invisible to every golden, every image
                // gate and every oracle differential.
                //
                // SAFETY: recording is open and no render scope is active. `count[fi]` is a live
                // device-local STORAGE buffer (≥ 4 B, the single u32 counter); `cmd_fill_buffer`
                // zero-fills it (Vulkan 1.0 core). `cull_uniform_buf[fi]` is a live DEVICE_LOCAL
                // buffer created with `TRANSFER_DST` at exactly `VB_CULL_UNIFORM_BYTES`, so the
                // update covers it whole; the destination offset is 0 and the size is 96 — both
                // multiples of 4, as `VUID-vkCmdUpdateBuffer-dstOffset-00036` /
                // `-dataSize-00037` require — and 96 is far inside the 65536-byte inline limit.
                // `cull_uniform` is a local that outlives the call, and `vkCmdUpdateBuffer` is
                // forbidden inside a render-pass instance (VUID-vkCmdUpdateBuffer-renderpass):
                // none is active here, the early raster's `cmd_begin_rendering` is below.
                unsafe {
                    (self.fns.cmd_fill_buffer)(cmd, count[fi].buffer, 0, VK_WHOLE_SIZE, 0);
                    (self.fns.cmd_update_buffer)(
                        cmd,
                        cull_uniform_buf[fi].buffer,
                        0,
                        u64::from(VB_CULL_UNIFORM_BYTES),
                        (&cull_uniform as *const VbCullUniform).cast(),
                    );
                }
                let cull_pass = plan
                    .vb_batch_cull
                    .expect("invariant: batch_cull_armed => vb_batch_cull pass declared");
                // SAFETY: recording is open; `record_vb_pass` records the graph's derived barriers
                // for the "vb_batch_cull" pass into `cmd` — the TRANSFER→COMPUTE ordering of the
                // descriptor upload, this counter fill and (since VG R3 P3-3) the uniform update
                // against the dispatch below, plus that step's split-gated pyramid read.
                self.record_vb_pass(cull_pass, cmd, targets, forward, vb, scene, fi);

                // Bounded by EVERY allocation the cull touches at index `i` — it reads
                // `VbBatchDesc[i]`, writes record `i`, and (rung R2d-3) writes that batch's OWNED
                // region of the survivor list — so the smallest capacity governs the dispatch.
                // Computed ONCE above, beside the fills, rather than re-derived here. The shader's
                // own `i >= pc.batch_count` guard then trims the tail group's lanes — together
                // that is what keeps every device access in bounds with `robustBufferAccess` off.
                //
                // ⚠️ Rung R2d-3 deliberately puts NO capacity guard in the shader for the region
                // write: a clamped-and-dropped region write would leave a survivor slot unwritten
                // while `instanceCount` still reported it, which the rasterizer would then
                // dereference. The host prefix above is the ONLY bound, and it drops the whole
                // batch (record intact, exactly pre-R2d rendering) rather than half of one.
                let dispatched_batches = batch_count as u32;
                // The clamp bound comes from the ALLOCATION, not from a host mirror of it: a push
                // word describing a capacity can drift from the buffer it describes, which is
                // exactly the failure VB-P1j had to close for `ClusterGrid`.
                let visible_cap = (visible[fi].size / 4) as u32;
                // Rung R2c: the six frustum planes the host extracted from the SAME 64 push bytes
                // the raster's VS reads. A frame that carries none pushes `DISARMED_PLANES`, which
                // cannot reject any box — so the cull degrades to rung R2c0's null control exactly,
                // not to something merely similar.
                // === VG R3 piece 3 step P3-3: the occlusion flag word's two host invariants. ===
                //
                // Checked ONCE, at the first site that consumes the word, so the late push below
                // inherits them. Both are properties of the HOST fold, so a violation is a wiring
                // bug in `GpuSceneBundles::scene` rather than a device fact — which is why they are
                // `debug_assert!`s here and not a runtime clamp that would repair the bug quietly.
                debug_assert!(
                    scene.vb_occ_flags & VB_CULL_OCC_ARMED == 0 || scene.hzb.is_some(),
                    "invariant: the ARMED bit claims a pyramid exists to test against; without one \
                     the occlusion leaf would project into `hzb_null` and reject nothing while the \
                     frame paid for the partition"
                );
                debug_assert!(
                    scene.vb_occ_flags & (VB_CULL_OCC_FORCE_LATE | VB_CULL_OCC_FORCE_KEEP)
                        != (VB_CULL_OCC_FORCE_LATE | VB_CULL_OCC_FORCE_KEEP),
                    "invariant: FORCE_LATE (defer everything marked) and FORCE_KEEP (defer nothing) \
                     are opposite controls — both set is a contradiction whose resolution would be \
                     whichever branch the shader tests first"
                );
                let push = VbBatchCullPush {
                    planes: scene.vb_cull_planes.unwrap_or(VbBatchCullPush::DISARMED_PLANES),
                    batch_count: dispatched_batches,
                    visible_cap,
                    // VG R3 piece 3 step P3-3: the EARLY phase. The late dispatch below pushes the
                    // SAME struct with `VB_CULL_PHASE_LATE`, which is the only word that differs.
                    phase: VB_CULL_PHASE_EARLY,
                    // Read by the module since step P3-4 — the `defer` guard, the two list stores
                    // and the pyramid tap's disarmed address mask — and `0` on every configuration
                    // until the P3-6 arming commit folds it, which is what keeps the frame
                    // byte-identical while the machinery is present.
                    occ_flags: scene.vb_occ_flags,
                };
                let groups = dispatched_batches.div_ceil(VB_BATCH_CULL_LOCAL_SIZE_X);
                // SAFETY: recording is open and outside any render scope; `pipeline` + its layout
                // (one COMPUTE set + the shared `COMPUTE_PUSH_CONSTANT_RANGE_BYTES` push range,
                // of which this pass writes `VB_BATCH_CULL_PUSH_BYTES` = 112 since VG R3 P3-3) are
                // live on this device (caller contract). `cull_set` binds TWELVE COMPUTE descriptors
                // against the 12-entry `vb_cull_layout` — eleven storage buffers and, at @9, one
                // SAMPLED image recorded at `GENERAL` — all written once at `GBufferTargets::sync`.
                // The module names ALL TWELVE since VG R3 P3-4, which is the step that gave the five
                // P3-2 added (@7/@8/@9/@10/@11) their loads; @0..@6 have been loaded, stored or
                // atomically updated since rung R2d-6. Between P3-2 and P3-4 the last five were
                // bound-but-unread, which is the legal direction — a WRITTEN descriptor a shader
                // never loads from is never dereferenced, so a bound set may exceed what the module
                // declares. (The reverse is undefined with `robustBufferAccess` off, which is why
                // the set and the layout widened in one commit while the shader lagged.) ⚠️ @9 is
                // `hzb_null` HERE regardless of the HZB arm — `vb_cull_set_hzb` is bound at P3-6 —
                // and the module's own address mask makes that tap `(0, 0, 0)` while
                // `VB_CULL_OCC_ARMED` is clear. The dispatch covers `dispatched_batches` lanes and
                // the shader trims its tail group's out-of-range lanes.
                // `&cull_set.descriptor_set` and `push` are locals alive for the calls.
                unsafe {
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        pipeline.layout,
                        0,
                        1,
                        &cull_set.descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_push_constants)(
                        cmd,
                        pipeline.layout,
                        VK_SHADER_STAGE_COMPUTE_BIT,
                        0,
                        VB_BATCH_CULL_PUSH_BYTES,
                        (&push as *const VbBatchCullPush).cast(),
                    );
                    (self.fns.cmd_dispatch)(cmd, groups, 1, 1);
                }
                // Rung R2c-tail: copy the cull's outputs into host-visible staging so a test can
                // read what the GPU actually decided. ARMED ONLY under `BOYKO_VB_CULL_READBACK`;
                // an unarmed frame records nothing here, which is what keeps the nine pins
                // byte-identical while this probe exists.
                if let Some(rb) = scene.vb_cull_readback {
                    let rb_pass = plan
                        .vb_cull_readback
                        .expect("invariant: vb_cull_readback armed => the readback pass is declared");
                    // SAFETY: recording is open; `record_vb_pass` records the graph's derived
                    // COMPUTE->TRANSFER barrier, making the cull's atomic writes AVAILABLE to the
                    // copies below. Without it the copy could read the counter before the
                    // dispatch's writes landed — and it would usually look right.
                    self.record_vb_pass(rb_pass, cmd, targets, forward, vb, scene, fi);

                    // === VG rung R2d-5: FOUR named regions, every size taken from the ALLOCATION
                    // it copies. VG R3 piece 3 step P3-5 added the PRE-late pair below. ===
                    //
                    // Rung R2c-tail copied the counter and then `rb.size - 16` — a REMAINDER, which
                    // is the one region shape that cannot be checked against its source: it is
                    // whatever the staging has left, so a staging that grew or shrank silently
                    // changed how much of the visible list was read. Each region below names its
                    // source buffer and takes that buffer's own `size`.
                    let indirect = scene
                        .vb_indirect
                        .expect("invariant: batch_cull_armed => vb_indirect");
                    let vis = scene
                        .vb_visible_instance
                        .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_visible_instance");
                    let sources = vb_cull_readback_sources(scene, fi)
                        .expect("invariant: batch_cull_armed => every cull readback source exists");
                    let layout = vb_cull_readback_layout(&sources, rb[fi].size);
                    debug_assert!(
                        layout.is_untruncated(&sources),
                        // `total() <= rb.size` is NOT asserted beside this: it is IMPLIED. If every
                        // region received its whole source then each already fitted the staging
                        // remaining at its turn, so the sum cannot exceed it. Asserting both would
                        // read as two independent checks when one is a restatement of the other.
                        "invariant: the cull readback staging holds all nine regions whole \
                         ({sources:?} = {} into a {}-byte staging)",
                        layout.total(),
                        rb[fi].size
                    );
                    // The PRE-late pair is recorded ONLY on a split frame, and the gate is the
                    // DECLARATOR's verbatim: `graph_bridge.rs`'s `vb_cull_readback` block declares
                    // `vb_late_visible`/`vb_late_count` `TRANSFER_READ` under `if occlusion_split`
                    // and under nothing else. Copying them unconditionally would be an UNDECLARED
                    // transfer read — the P2-7 class this campaign has already shipped once.
                    //
                    // Their REGIONS exist either way (the layout is computed from allocations, not
                    // from this frame's arming), so the host's constant decode offsets do not move
                    // between an unsplit and a split frame.
                    let late_visible = scene
                        .vb_late_visible
                        .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_late_visible");
                    let late_count = scene
                        .vb_late_count
                        .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_late_count");
                    let copies = [
                        (count[fi].buffer, layout.count, VB_CULL_READBACK_COUNT_OFFSET),
                        (visible[fi].buffer, layout.list, layout.list_offset()),
                        (indirect[fi].buffer, layout.records, layout.records_offset()),
                        (vis[fi].buffer, layout.vis, layout.vis_offset()),
                        (
                            late_visible[fi].buffer,
                            if occlusion_split { layout.late_candidates } else { 0 },
                            layout.late_candidates_offset(),
                        ),
                        (
                            late_count[fi].buffer,
                            if occlusion_split { layout.late_count_pre } else { 0 },
                            layout.late_count_pre_offset(),
                        ),
                    ];
                    for (src, size, dst_offset) in copies {
                        // `vkCmdCopyBuffer` forbids a zero-size region
                        // (VUID-VkBufferCopy-size-00112); a region the staging could not hold is
                        // skipped rather than clamped to nothing.
                        if size == 0 {
                            continue;
                        }
                        let region = VkBufferCopy { src_offset: 0, dst_offset, size };
                        // SAFETY: recording is open and outside any render scope. `src` is one of
                        // the six live cull buffers, each created with `TRANSFER_SRC`
                        // (`vb_cull_count`, `vb_cull_visible`, `vb_indirect`,
                        // `vb_visible_instance`, `vb_late_visible` and `vb_late_count` — see
                        // `GpuSceneBundles::boot`), and `rb[fi]` is the live host-visible
                        // `TRANSFER_DST` staging. `vb_cull_readback_layout` computed `size` as
                        // `min(src_size, staging bytes still unassigned)` and assigned the regions
                        // in order, so `dst_offset + size <= rb[fi].size` and `size <= src.size`
                        // both hold for every element of `copies` — the source read and the
                        // destination write are each in bounds. The two late entries additionally
                        // pass `0` (skipped above) on an unsplit frame, which is the only state in
                        // which their `TRANSFER_READ` is undeclared. `region` is a local that
                        // outlives the call.
                        unsafe {
                            (self.fns.cmd_copy_buffer)(cmd, src, rb[fi].buffer, 1, &region);
                        }
                    }
                }

                // The cull's `vb_indirect` write is made visible to the raster's indirect FETCH by
                // the graph: derived at the reader (`vb_raster`'s own `DRAW_INDIRECT` access), not
                // here. Under the probe there is now a TRANSFER read between the two, so that one
                // edge becomes two — `COMPUTE -> TRANSFER` (availability) then
                // `TRANSFER -> DRAW_INDIRECT` (visibility) — which `declare_vb_graph`'s
                // `vb_cull_readback` block records in full. Still derived, still at the reader.
            }

            // SAFETY: recording is open; `record_vb_pass` records the graph's derived
            // UNDEFINED→COLOR_ATTACHMENT_OPTIMAL (`vb_id`) + UNDEFINED→DEPTH_ATTACHMENT_OPTIMAL
            // (`vb_depth`) barriers-in for the "vb_raster" pass into `cmd` — and, since rung R2a',
            // the TRANSFER→DRAW_INDIRECT dependency against the update above.
            self.record_vb_pass(
                plan.vb_raster.expect("invariant: mesh_leg => vb_raster pass declared (declare_vb_graph)"),
                cmd,
                targets,
                forward,
                vb,
                scene,
                fi,
            );

            let vb_raster_pipeline =
                scene.vb_raster_pipeline.expect("invariant: a VisibilityBuffer-resolved scene always carries vb_raster_pipeline");

            let vb_id_attachment = VkRenderingAttachmentInfo {
                s_type: VkStructureType::RenderingAttachmentInfo,
                p_next: ptr::null(),
                image_view: vb.vb_id[fi].view,
                image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                resolve_mode: 0,
                resolve_image_view: VkImageView::NULL,
                resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
                store_op: VK_ATTACHMENT_STORE_OP_STORE,
                clear_value: VkClearValue { color: VkClearColorValue { uint32: VB_ID_CLEAR } },
            };
            let vb_depth_attachment = VkRenderingAttachmentInfo {
                s_type: VkStructureType::RenderingAttachmentInfo,
                p_next: ptr::null(),
                image_view: forward.depth[fi].view,
                image_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                resolve_mode: 0,
                resolve_image_view: VkImageView::NULL,
                resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
                store_op: VK_ATTACHMENT_STORE_OP_STORE,
                clear_value: VkClearValue {
                    depth_stencil: VkClearDepthStencilValue { depth: VB_DEPTH_CLEAR, stencil: 0 },
                },
            };
            let vb_rendering = VkRenderingInfo {
                s_type: VkStructureType::RenderingInfo,
                p_next: ptr::null(),
                flags: 0,
                render_area: full_area,
                layer_count: 1,
                view_mask: 0,
                color_attachment_count: 1,
                p_color_attachments: &vb_id_attachment,
                p_depth_attachment: (&vb_depth_attachment as *const VkRenderingAttachmentInfo).cast(),
                p_stencil_attachment: ptr::null(),
            };
            // === VG rung R2d-4: the per-draw `{ base_instance, flags }` push image. ===
            //
            // The pass-wide push below writes all 88 bytes of `scene.mvp`, whose word at offset 84
            // is the arm selector `boyko_render::view::forward_view_proj_rows` mints (1 when an
            // instanced batch list draws, 0 for a legacy merged draw). Each batch then re-writes
            // the LAST TWO words of that range in ONE call — 8 bytes at
            // `GBUFFER_PUSH_BASE_INSTANCE_OFFSET` (80), ending at exactly `GBUFFER_PUSH_BYTES`
            // (88), which is the whole range this pipeline's layout declares for VERTEX|FRAGMENT
            // at offset 0 (`GraphicsPipelineDesc::push_constant_bytes`, wired to
            // `GBUFFER_PUSH_BYTES` at the pipeline's build site). Both the offset and the size are
            // multiples of 4, as `vkCmdPushConstants` requires.
            //
            // The flags word is read out of `scene.mvp` rather than re-derived, so this recorder
            // cannot disagree with the pass-wide push about which ARM the draw is on — it only
            // ever ADDS a bit.
            const FLAGS_OFFSET: usize = GBUFFER_PUSH_BASE_INSTANCE_OFFSET as usize + 4;
            let base_flags = u32::from_le_bytes([
                scene.mvp[FLAGS_OFFSET],
                scene.mvp[FLAGS_OFFSET + 1],
                scene.mvp[FLAGS_OFFSET + 2],
                scene.mvp[FLAGS_OFFSET + 3],
            ]);

            // SAFETY: recording is open; `vb_rendering` names the live `vb_id` view (now
            // COLOR_ATTACHMENT_OPTIMAL) + the live `vb_depth` (`forward.depth[fi]`, REUSED verbatim)
            // view (now DEPTH_ATTACHMENT_OPTIMAL); `vb_raster_pipeline` (1-set, built against
            // `vb_layout0` — since rung R2d-4 its VS references `instances` @0, `visible_instances`
            // @11 and the push, still a subset of what `vb_set0` binds) + the 88-byte VERTEX push
            // range belong to this device (caller contract); `vb_set0[fi]` is a live descriptor set
            // whose @11 entry is the survivor list the VS now loads from. The per-draw push writes
            // bytes [80, 88) of that 88-byte range. `full_viewport`/`full_area` outlive the
            // bracketed calls; each `DrawBatch`'s per-instance draw reads that batch's bound
            // vertex+index buffers. Begin/End bracket the pass exactly.
            unsafe {
                (self.fns.cmd_begin_rendering)(cmd, &vb_rendering);
                (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, vb_raster_pipeline.pipeline);
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_GRAPHICS,
                    vb_raster_pipeline.layout,
                    0,
                    1,
                    &vb_set0[fi].descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_push_constants)(
                    cmd,
                    vb_raster_pipeline.layout,
                    VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
                    0,
                    scene.mvp.len() as u32,
                    scene.mvp.as_ptr().cast(),
                );
                (self.fns.cmd_set_viewport)(cmd, 0, 1, &full_viewport);
                (self.fns.cmd_set_scissor)(cmd, 0, 1, &full_area);
                for (i, batch) in scene.mesh_draw.iter().enumerate() {
                    // ⚠️ Rung R2d-4: `i < batch_count` is LOAD-BEARING, and it is the RELEASE-path
                    // mechanism — not a debug check. The cull's dispatch covers exactly
                    // `[0, batch_count)` (the SAME hoisted local the descriptor fill and the
                    // dispatch read), and the shader's own tail guard trims lanes past it. A batch
                    // OUTSIDE that range — clamped away by the visible-capacity clamp
                    // (`vb_cull_batch_count_visible_clamp`), by the record/descriptor capacities, or
                    // simply beyond the dispatch — had its region of the survivor list NOT written
                    // this frame. `gVbVisibleInstance` is DEVICE_LOCAL and nothing clears it, so
                    // that region holds undefined device memory on frame 1 and a previous frame's
                    // residue afterwards. Clearing the bit makes the VS evaluate the pre-R2d
                    // expression literally for exactly those batches, which is also why a clamped
                    // batch renders identically rather than merely "close".
                    // The instanced-arm term makes `flags == 2` STRUCTURALLY unreachable rather
                    // than merely asserted. Bit 1 without bit 0 is not a no-op: it makes the VS's
                    // `use_model_matrix == 0u` test false and flips the draw from the legacy arm to
                    // the instanced one. The contract that byte 84 is 1 whenever `mesh_draw` is
                    // non-empty is minted in a DIFFERENT crate (`boyko_app`'s runner), so guarding
                    // it only with a `debug_assert!` here would leave the release path depending on
                    // a promise this file cannot see. Reading bit 0 back out of the word we are
                    // about to push costs one AND and removes the dependency.
                    let indirection = batch_cull_armed
                        && i < batch_count
                        && (base_flags & VB_RASTER_FLAG_INSTANCED_ARM) != 0;
                    let mut flags = base_flags;
                    if indirection {
                        flags |= VB_RASTER_FLAG_VISIBLE_INDIRECTION;
                    }
                    debug_assert!(
                        (flags & VB_RASTER_FLAG_VISIBLE_INDIRECTION) == 0
                            || (flags & VB_RASTER_FLAG_INSTANCED_ARM) != 0,
                        "invariant: the visible-indirection bit is meaningless without the \
                         instanced arm — flags {flags:#x} means the word was assembled wrong, and \
                         the VS would take the instanced arm on a draw that has no instance rows"
                    );
                    let push: [u32; 2] = [batch.base_instance, flags];
                    (self.fns.cmd_push_constants)(
                        cmd,
                        vb_raster_pipeline.layout,
                        VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
                        GBUFFER_PUSH_BASE_INSTANCE_OFFSET,
                        core::mem::size_of_val(&push) as u32,
                        push.as_ptr().cast(),
                    );
                    (self.fns.cmd_bind_vertex_buffers)(cmd, 0, 1, &batch.vertex_buffer.buffer, &vertex_offset);
                    (self.fns.cmd_bind_index_buffer)(cmd, batch.index_buffer.buffer, 0, batch.index_type);
                    // Rung R2a': the indirect seam. The record was filled above with EXACTLY the
                    // arguments the direct call took, so this frame is byte-identical BY
                    // CONSTRUCTION rather than by measurement — the point of the rung is the
                    // TRANSFER→DRAW_INDIRECT dependency and the record path, not a different image.
                    //
                    // `draw_count = 1` is not a choice: `multiDrawIndirect` is not enabled on this
                    // device, so the only legal values are 0 and 1. With `draw_count == 1` the
                    // `stride` argument is unread, and it is passed truthfully anyway.
                    // `i < record_capacity` is the same allocation-derived bound the fill above
                    // used: a batch with no record falls through to the direct arm rather than
                    // fetching a command past the end of the buffer.
                    match scene.vb_indirect.filter(|_| i < record_capacity) {
                        Some(indirect) => (self.fns.cmd_draw_indexed_indirect)(
                            cmd,
                            indirect[fi].buffer,
                            i as u64 * u64::from(DRAW_INDEXED_INDIRECT_STRIDE),
                            1,
                            DRAW_INDEXED_INDIRECT_STRIDE,
                        ),
                        // A boot that failed to build the record buffer still renders, on the
                        // direct path this rung replaced.
                        None => (self.fns.cmd_draw_indexed)(
                            cmd,
                            batch.index_count,
                            batch.instance_count,
                            0,
                            0,
                            0,
                        ),
                    }
                }
                (self.fns.cmd_end_rendering)(cmd);
            }
            // Gate G2's `scopes`, counted AT the bracket that closes the EARLY scope rather than
            // derived from the arming predicate — the difference between a gate and a tautology.
            if let Some(p) = probe.as_deref_mut() {
                p.scopes += 1;
            }

            // === VG R3 piece 2 step P2-5 (plan D6): the `[hzb_poison, hzb_build_*]` block's
            // ARMED-SPLIT slot — BETWEEN the two raster scopes. ===
            //
            // The unit moves WHOLE, at both declare and record: `hzb_poison` is asserted to precede
            // every `hzb_build_*`, and a build that moved without its clear would have the clear
            // ERASE what the dispatches just wrote (dev profile: the declarator's assert fires;
            // release: gate G8 reds at every texel claiming "the build never ran"). One helper
            // records both, so leaving the poison behind is not expressible here.
            //
            // The pyramid must reduce the depth the EARLY scope wrote, which is why the block lands
            // before the late scope rather than after it.
            if occlusion_split {
                self.record_hzb_poison_build(plan, cmd, targets, forward, vb, scene, present_extent, fi);
            }

            // === VG R3 piece 3 step P3-3 (plan D4/D5): the LATE cull dispatch. ===
            //
            // The SECOND dispatch of `vb_batch_cull.comp.hlsl`, differing from the early one in ONE
            // pushed word (`phase`). Recorded HERE — after the pyramid this frame's raster fed, and
            // before the late scope that fetches the count it will write — which is EXACTLY
            // `declare_vb_graph`'s position for `vb_cull_late` (declare/record ORDER parity, the
            // invariant this file and the declarator both treat as load-bearing).
            //
            // FIXED AND HOST-SIZED: `batch_count` lanes, the same number and the same
            // `VB_BATCH_CULL_LOCAL_SIZE_X` divisor the early dispatch uses. The cull runs ONE LANE
            // PER BATCH and `batch_count` is a host number, so no `vkCmdDispatchIndirect` and no
            // `vkCmdDrawIndexedIndirectCount` is needed — neither is in this device's fn table, and
            // the GPU-only quantity (the per-batch candidate count) is a LOOP BOUND inside a lane,
            // never a dispatch size.
            //
            // ⚠️ IT WRITES NOTHING BUT ZEROS UNTIL P3-6, BY CONSTRUCTION AND NOT BY LUCK. The
            // module's phase fork (`if (pc.phase == VB_CULL_PHASE_LATE) { ... return; }`) landed at
            // step P3-3, in the same commit as this dispatch — deliberately, because without it the
            // dispatch would re-run the EARLY body and rewrite `VbVisibleInstance` and every
            // record's `instanceCount` AFTER the early raster had fetched them; on a static scene it
            // would write the same numbers, so no golden could see it. Step P3-4 gave phase 1 its
            // real body, and its loop bound `VbLateCount[i]` is read as `0` while
            // `VB_CULL_OCC_ARMED` is clear — so the compaction runs zero iterations and stores the
            // `instanceCount = 0` the host fill already wrote.
            //
            // ⚠️ THE GATE IS THE DECLARATOR'S, VERBATIM — `occlusion_split`, not
            // `batch_cull_armed`. A recorder gate NARROWER than `declare_vb_graph`'s would leave
            // `vb_indirect_late`'s declared writer as a COMPUTE that never ran, and the
            // `TRANSFER → DRAW_INDIRECT` dependency the host fill actually needs would be derived
            // nowhere: a missing barrier, not a wasted one. The pipeline and the set are
            // `.expect()`ed rather than folded into the gate for the same reason the early dispatch
            // states — and the residual that makes those `.expect()`s reachable at all (a device
            // without `storage_buffer_array_non_uniform_indexing`, where the split can arm while
            // the cull is unwired) is named at the declaration site and closed by step P3-6's
            // `vb_mesh_bounds.is_some()` conjunct.
            //
            // ⚠️ IT BINDS `vb_cull_set`, THE SAME SET THE EARLY DISPATCH BOUND, so @9 is `hzb_null`
            // even on an HZB-armed frame. Binding the REAL pyramid is `vb_cull_set_hzb`'s job at
            // step P3-6. The module HAS tapped @9 since P3-4, and that is safe here for the reason
            // its own `hzb_pyramid_load` states: with `VB_CULL_OCC_ARMED` clear every coordinate and
            // the level are masked to 0, so the address is `(0, 0, 0)` — in range for a 1x1
            // single-mip image whether or not the tap is dynamically reached.
            if occlusion_split {
                let pipeline = scene
                    .vb_batch_cull_pipeline
                    .expect("invariant: path_vb_occlusion_split ⇒ vb_batch_cull_pipeline (the C16 \
                             residual is closed by P3-6's vb_mesh_bounds conjunct)");
                let cull_set = &targets
                    .vb_cull_set
                    .as_ref()
                    .expect("invariant: path_vb_occlusion_split ⇒ targets.vb_cull_set (same VB gate)")[fi];
                let late_cull_pass = plan
                    .vb_cull_late
                    .expect("invariant: the recorder's late-cull gate is the declarator's, verbatim");
                // SAFETY: recording is open and no render scope is active (the early scope's
                // `cmd_end_rendering` is above and the late scope's `cmd_begin_rendering` is below);
                // `record_vb_pass` records the graph's derived barriers for the "vb_cull_late" pass
                // into `cmd` — the pyramid's `hzb_build_{n-1}` → COMPUTE RAW at `GENERAL` (no layout
                // change), the early phase's `vb_late_visible` / `vb_late_count` writes made visible
                // to this dispatch, and this pass's own read→write self-edge on `vb_late_visible`.
                self.record_vb_pass(late_cull_pass, cmd, targets, forward, vb, scene, fi);

                let dispatched_batches = batch_count as u32;
                let late_push = VbBatchCullPush {
                    // Byte-identical to the early push except for `phase`: the late phase re-tests
                    // the SAME instances with the SAME camera, so a second plane set (or a second
                    // capacity) would be a second thing that can drift.
                    planes: scene.vb_cull_planes.unwrap_or(VbBatchCullPush::DISARMED_PLANES),
                    batch_count: dispatched_batches,
                    visible_cap: visible_elems as u32,
                    phase: VB_CULL_PHASE_LATE,
                    occ_flags: scene.vb_occ_flags,
                };
                let late_groups = dispatched_batches.div_ceil(VB_BATCH_CULL_LOCAL_SIZE_X);
                // SAFETY: recording is open and outside any render scope. `pipeline` + its layout
                // are the SAME live objects the early dispatch bound (caller contract), and this
                // pass writes the same `VB_BATCH_CULL_PUSH_BYTES` = 112 prefix of the shared COMPUTE
                // push range at offset 0 — both multiples of 4. `cull_set` binds all TWELVE entries
                // `vb_cull_layout` declares, written once at `GBufferTargets::sync` and untouched
                // since; the module names all twelve since P3-4 and, on this phase, reads the
                // uniform, the batch descriptors and `VbLateCount` before its loop bound stops it.
                // The dispatch covers
                // `dispatched_batches` lanes — the SAME host number the early dispatch used, bounded
                // by the same three allocation-derived capacities — and the shader's own
                // `i >= pc.batch_count` guard trims the tail group. `&cull_set.descriptor_set` and
                // `late_push` are locals alive for the calls.
                unsafe {
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        pipeline.layout,
                        0,
                        1,
                        &cull_set.descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_push_constants)(
                        cmd,
                        pipeline.layout,
                        VK_SHADER_STAGE_COMPUTE_BIT,
                        0,
                        VB_BATCH_CULL_PUSH_BYTES,
                        (&late_push as *const VbBatchCullPush).cast(),
                    );
                    (self.fns.cmd_dispatch)(cmd, late_groups, 1, 1);
                }
            }

            // === VG R3 piece 2 step P2-5 (plan D4): the LATE raster scope. ===
            //
            // A SECOND `begin/endRendering` bracket over the SAME two views and the SAME
            // `renderArea`, `LOAD_OP_LOAD`/`STORE_OP_STORE` on both, drawing `batch_count` indirect
            // draws whose every record carries `instanceCount = 0`. `LOAD_OP_LOAD` yields exactly
            // what the early scope stored, no fragment is produced, and `STORE_OP_STORE` writes the
            // loaded contents back — so the final `vb_id`/`vb_depth` contents are the early scope's,
            // by an argument that needs no numerics and is therefore not subject to the 8-bit
            // golden floor.
            //
            // The binds are REPEATED rather than relied on to survive `vkCmdEndRendering` and the
            // interposed compute dispatches: four commands to remove a subtle dependence on state
            // leakage across a render scope.
            if occlusion_split {
                let late = scene.vb_indirect_late.expect(
                    "invariant: path_vb_occlusion_split ⇒ vb_indirect_late (GpuSceneBundles::boot \
                     mints it unconditionally on every VB boot)",
                );
                let late_pass = plan
                    .vb_raster_late
                    .expect("invariant: the recorder's late-scope gate is the declarator's, verbatim");
                // SAFETY: recording is open and the early scope's `cmd_end_rendering` is above;
                // `record_vb_pass` records this pass's derived barriers into `cmd` — the `vb_id`
                // and `vb_depth` WAWs at their existing layouts (on an HZB-armed frame the depth's
                // `SHADER_READ_ONLY_OPTIMAL → DEPTH_ATTACHMENT_OPTIMAL` return leg), and the
                // `TRANSFER → DRAW_INDIRECT` dependency against the late fill above.
                self.record_vb_pass(late_pass, cmd, targets, forward, vb, scene, fi);

                // ⚠️ `LOAD_OP_LOAD`, not CLEAR, and this is the whole equivalence. A CLEAR here
                // would present only what the late scope drew — nothing. Every other field is the
                // early scope's, spelled OUT rather than functional-update-copied: this FFI struct
                // is deliberately neither `Clone` nor `Copy` (it owns a `p_next` raw pointer), and
                // `..early` would partially MOVE out of a local the early scope's already-submitted
                // `VkRenderingInfo` still points at. The `clear_value` is ignored under
                // `LOAD_OP_LOAD`; it carries the early scope's value so the union has a defined
                // active field rather than arbitrary padding.
                let vb_id_attachment_late = VkRenderingAttachmentInfo {
                    s_type: VkStructureType::RenderingAttachmentInfo,
                    p_next: ptr::null(),
                    image_view: vb.vb_id[fi].view,
                    image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    resolve_mode: 0,
                    resolve_image_view: VkImageView::NULL,
                    resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                    load_op: VK_ATTACHMENT_LOAD_OP_LOAD,
                    store_op: VK_ATTACHMENT_STORE_OP_STORE,
                    clear_value: VkClearValue { color: VkClearColorValue { uint32: VB_ID_CLEAR } },
                };
                let vb_depth_attachment_late = VkRenderingAttachmentInfo {
                    s_type: VkStructureType::RenderingAttachmentInfo,
                    p_next: ptr::null(),
                    image_view: forward.depth[fi].view,
                    image_layout: VK_IMAGE_LAYOUT_DEPTH_ATTACHMENT_OPTIMAL,
                    resolve_mode: 0,
                    resolve_image_view: VkImageView::NULL,
                    resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
                    load_op: VK_ATTACHMENT_LOAD_OP_LOAD,
                    store_op: VK_ATTACHMENT_STORE_OP_STORE,
                    clear_value: VkClearValue {
                        depth_stencil: VkClearDepthStencilValue { depth: VB_DEPTH_CLEAR, stencil: 0 },
                    },
                };
                let vb_rendering_late = VkRenderingInfo {
                    s_type: VkStructureType::RenderingInfo,
                    p_next: ptr::null(),
                    flags: 0,
                    render_area: full_area,
                    layer_count: 1,
                    view_mask: 0,
                    color_attachment_count: 1,
                    p_color_attachments: &vb_id_attachment_late,
                    p_depth_attachment: (&vb_depth_attachment_late as *const VkRenderingAttachmentInfo).cast(),
                    p_stencil_attachment: ptr::null(),
                };
                // The equivalence argument depends on the two scopes covering the SAME region: a
                // narrower late `renderArea` would leave the store undefined outside it. They share
                // one `full_area` local, and this checks that they still do — `VkRect2D` carries no
                // `PartialEq`, so the four fields are compared by hand.
                debug_assert!(
                    vb_rendering_late.render_area.offset.x == vb_rendering.render_area.offset.x
                        && vb_rendering_late.render_area.offset.y == vb_rendering.render_area.offset.y
                        && vb_rendering_late.render_area.extent.width
                            == vb_rendering.render_area.extent.width
                        && vb_rendering_late.render_area.extent.height
                            == vb_rendering.render_area.extent.height,
                    "invariant: the late scope's renderArea equals the early scope's"
                );

                // SAFETY: recording is open and no render scope is active (the early scope's
                // `cmd_end_rendering` is above). `vb_rendering_late` names the SAME live `vb_id`
                // and `vb_depth` views the early scope named, each in the layout the derived
                // barrier just left it in (`COLOR_ATTACHMENT_OPTIMAL` / `DEPTH_ATTACHMENT_OPTIMAL`);
                // both attachments are `LOAD_OP_LOAD`, which requires exactly that the contents be
                // defined — they are, the early scope stored them. `vb_raster_pipeline` and its
                // 88-byte VERTEX|FRAGMENT push range and `vb_set0[fi]` are the same live objects the
                // early scope bound (caller contract). Each per-batch push writes bytes [80, 88) of
                // that range, both offset and size multiples of 4. `vb_rendering_late`,
                // `vb_id_attachment_late`, `vb_depth_attachment_late`, `full_viewport`, `full_area`
                // and `push` are locals alive across the bracketed calls; every `i` is `<
                // batch_count <= record_capacity_late`, so `i * DRAW_INDEXED_INDIRECT_STRIDE + 20`
                // is inside `late[fi]`, whose contents this frame's own `vkCmdUpdateBuffer` wrote
                // above and whose visibility to the indirect FETCH the declared
                // `TRANSFER → DRAW_INDIRECT` edge provides. Begin/End bracket the pass exactly.
                unsafe {
                    (self.fns.cmd_begin_rendering)(cmd, &vb_rendering_late);
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, vb_raster_pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_GRAPHICS,
                        vb_raster_pipeline.layout,
                        0,
                        1,
                        &vb_set0[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_push_constants)(
                        cmd,
                        vb_raster_pipeline.layout,
                        VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
                        0,
                        scene.mvp.len() as u32,
                        scene.mvp.as_ptr().cast(),
                    );
                    (self.fns.cmd_set_viewport)(cmd, 0, 1, &full_viewport);
                    (self.fns.cmd_set_scissor)(cmd, 0, 1, &full_area);
                    for (i, batch) in scene.mesh_draw.iter().enumerate().take(batch_count) {
                        // ⚠️ THE SURVIVOR-INDIRECTION BIT IS CLEAR, and "harmless because the count
                        // is zero" is not the reason. A set bit would name a region of
                        // `gVbVisibleInstance` that NO pass wrote this frame — verbatim the residue
                        // hazard `R2d-REGION-DEFINED` exists to forbid — and DXC is free to lower
                        // the VS's `? :` to an eager load plus an `OpSelect`. Piece 3 sets the bit
                        // in the same change that writes the region.
                        let push: [u32; 2] = [batch.base_instance, base_flags];
                        debug_assert!(
                            (push[1] & VB_RASTER_FLAG_VISIBLE_INDIRECTION) == 0,
                            "invariant: PIECE 2 ONLY — the late scope pushes the survivor \
                             indirection CLEAR, because no pass writes that region this frame"
                        );
                        (self.fns.cmd_push_constants)(
                            cmd,
                            vb_raster_pipeline.layout,
                            VK_SHADER_STAGE_VERTEX_BIT | VK_SHADER_STAGE_FRAGMENT_BIT,
                            GBUFFER_PUSH_BASE_INSTANCE_OFFSET,
                            core::mem::size_of_val(&push) as u32,
                            push.as_ptr().cast(),
                        );
                        (self.fns.cmd_bind_vertex_buffers)(cmd, 0, 1, &batch.vertex_buffer.buffer, &vertex_offset);
                        (self.fns.cmd_bind_index_buffer)(cmd, batch.index_buffer.buffer, 0, batch.index_type);
                        // `draw_count = 1` is not a choice: `multiDrawIndirect` is not enabled on
                        // this device, so the only legal values are 0 and 1. There is no direct-draw
                        // fallback arm here, unlike the early scope: this scope EXISTS to fetch from
                        // the late array, and a direct draw would draw the batch for real.
                        (self.fns.cmd_draw_indexed_indirect)(
                            cmd,
                            late[fi].buffer,
                            i as u64 * u64::from(DRAW_INDEXED_INDIRECT_STRIDE),
                            1,
                            DRAW_INDEXED_INDIRECT_STRIDE,
                        );
                        // Gate G2's `late_draws`, counted PER ISSUED DRAW inside the loop. A count
                        // assigned from `batch_count` after the loop would be green under the
                        // `take(1)` corruption the gate's own red control uses.
                        if let Some(p) = probe.as_deref_mut() {
                            p.late_draws += 1;
                        }
                    }
                    (self.fns.cmd_end_rendering)(cmd);
                }
                if let Some(p) = probe {
                    p.scopes += 1;
                }
            }

            // === VG R3 piece 3 steps P3-3/P3-5 (plan D8): the POST-LATE readback snapshot. ===
            //
            // Recorded at `declare_vb_graph`'s matching position — AFTER the late scope — so pass
            // ORDER parity holds between declarator and recorder. Step P3-3 landed the barriers
            // alone (a declared pass whose barriers are never recorded is a pass whose derived edges
            // the command stream never establishes, and the graph would go on deriving the NEXT
            // pass's edges from a state nothing produced); step P3-5 adds the three COPIES.
            //
            // ⚠️ The copies read the SAME two buffers the PRE snapshot copied, and that is the
            // point: `vb_late_visible` is compacted IN PLACE by the late cull, so the candidate list
            // and the survivor prefix are the same bytes at two different TIMES. Only two snapshots
            // can hold both, which is why plan A5's adjudication needs `late_candidates` (PRE) and
            // `late_survivors` (POST) as separate regions rather than one region read twice.
            //
            // Armed only under `BOYKO_VB_CULL_READBACK` (and only on a split frame), so every
            // golden and interactive boot records nothing here at all.
            if let Some(rb_late_pass) = plan.vb_cull_readback_late {
                debug_assert!(
                    occlusion_split && scene.vb_cull_readback.is_some(),
                    "invariant: the recorder's post-late snapshot gate is the declarator's, verbatim"
                );
                // SAFETY: recording is open and no render scope is active (the late scope's
                // `cmd_end_rendering` is above); `record_vb_pass` records the graph's derived
                // barriers for the "vb_cull_readback_late" pass into `cmd`.
                self.record_vb_pass(rb_late_pass, cmd, targets, forward, vb, scene, fi);

                let rb = scene
                    .vb_cull_readback
                    .expect("invariant: the post-late snapshot pass is declared only when the probe is armed");
                let sources = vb_cull_readback_sources(scene, fi)
                    .expect("invariant: occlusion_split => every cull readback source exists");
                // The SAME derivation the PRE snapshot ran, from the SAME allocations — so the two
                // passes address one layout rather than two that happen to agree.
                let layout = vb_cull_readback_layout(&sources, rb[fi].size);
                debug_assert!(
                    layout.is_untruncated(&sources),
                    "invariant: the cull readback staging holds all nine regions whole \
                     ({sources:?} = {} into a {}-byte staging)",
                    layout.total(),
                    rb[fi].size
                );
                let late_visible = scene
                    .vb_late_visible
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_late_visible");
                let late_count = scene
                    .vb_late_count
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_late_count");
                let late_records = scene
                    .vb_indirect_late
                    .expect("invariant: path_vb_occlusion_split => vb_indirect_late");
                let copies = [
                    (late_visible[fi].buffer, layout.late_survivors, layout.late_survivors_offset()),
                    (late_count[fi].buffer, layout.late_count_post, layout.late_count_post_offset()),
                    (late_records[fi].buffer, layout.late_records, layout.late_records_offset()),
                ];
                for (src, size, dst_offset) in copies {
                    // `vkCmdCopyBuffer` forbids a zero-size region
                    // (VUID-VkBufferCopy-size-00112); a region the staging could not hold is
                    // skipped rather than clamped to nothing.
                    if size == 0 {
                        continue;
                    }
                    let region = VkBufferCopy { src_offset: 0, dst_offset, size };
                    // SAFETY: recording is open and outside any render scope (the late scope ended
                    // above). `src` is one of the three live late-cull buffers, each created with
                    // `TRANSFER_SRC` (`vb_late_visible`, `vb_late_count`, `vb_indirect_late` — see
                    // `GpuSceneBundles::boot`), and `rb[fi]` is the live host-visible
                    // `TRANSFER_DST` staging. `vb_cull_readback_layout` assigned every region as a
                    // whole-or-nothing prefix of the staging, so `dst_offset + size <= rb[fi].size`
                    // and `size <= src.size` both hold. All three `TRANSFER_READ`s are declared on
                    // this pass (`graph_bridge.rs`'s `vb_cull_readback_late` block). `region` is a
                    // local that outlives the call.
                    unsafe {
                        (self.fns.cmd_copy_buffer)(cmd, src, rb[fi].buffer, 1, &region);
                    }
                }
            }

            // === VB-P2 classification plan (docs/VB-P2-CLASSIFICATION-PLAN.md), rung P2c: the
            // classify chain `fill -> count -> scan -> scatter` — populates `gClassify` on real
            // hardware ONLY when the classified path is selected (`scene.vb_use_classified`, plan
            // P1-4) — mirrors `declare_vb_graph`'s matching gate, so a `!vb_use_classified` frame
            // records NONE of these four passes (ZERO classify tax, not merely an unread output as
            // rung P2b left it). `vb_shade` (below) is the sole consumer of `gClassify`. ===
            if scene.vb_use_classified && scene.path_vb_fused() {
                // Pass `vb_classify_fill`: two `vkCmdFillBuffer`s zero `counts[MAX]` and sentinel
                // `group_to_mat[G+MAX]` with `0xFFFFFFFF` (critic P1-1 — decouples correctness from
                // the scan's per-frame loop bound). Word offsets mirror
                // `vb_classify_common.hlsli`'s own sync-pin: `counts_off = 0`, `group_to_mat_off =
                // 4*MAX` (both word offsets, ×4 for bytes); `group_to_mat`'s reserved CAPACITY is
                // `G+MAX` (P1-2 — fixed per extent, not per frame), so the sentinel fill covers the
                // WHOLE reserved region regardless of this frame's live material count.
                // SAFETY: recording is open; `record_vb_pass` records the graph's derived
                // TRANSFER_WRITE producer access for the "vb_classify_fill" pass into `cmd` (the
                // FIRST access on `gclassify` this frame).
                self.record_vb_pass(
                    plan.vb_classify_fill.expect(
                        "invariant: mesh_leg && vb_use_classified => vb_classify_fill pass declared (declare_vb_graph)",
                    ),
                    cmd,
                    targets,
                    forward,
                    vb,
                    scene,
                    fi,
                );
                let gclassify_buf = targets
                    .vb_classify
                    .as_ref()
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries targets.vb_classify")
                    .gclassify[fi]
                    .buffer;
                let counts_bytes: VkDeviceSize = VB_CLASSIFY_MAX_MATERIAL_ROWS * 4;
                let group_to_mat_off_bytes: VkDeviceSize = 4 * VB_CLASSIFY_MAX_MATERIAL_ROWS * 4;
                let group_to_mat_bytes: VkDeviceSize =
                    (scene.dispatch_group_count_x as VkDeviceSize + VB_CLASSIFY_MAX_MATERIAL_ROWS) * 4;
                // SAFETY: recording is open; `gclassify_buf` is the live per-FIF `gClassify` buffer
                // (`STORAGE | TRANSFER_DST`, sized per `VbClassifyTargets::build`'s doc); both fill
                // regions lie within its bounds (`counts` is the buffer's first `MAX*4` bytes;
                // `group_to_mat`'s `[4*MAX*4, 4*MAX*4 + (G+MAX)*4)` range is its own reserved region,
                // per the sync-pin doc above — both fully inside the buffer `VbClassifyTargets::build`
                // allocates).
                unsafe {
                    (self.fns.cmd_fill_buffer)(cmd, gclassify_buf, 0, counts_bytes, 0);
                    (self.fns.cmd_fill_buffer)(
                        cmd,
                        gclassify_buf,
                        group_to_mat_off_bytes,
                        group_to_mat_bytes,
                        0xFFFF_FFFF,
                    );
                }

                // Pass `vb_classify_count`: one thread per composite pixel, `InterlockedAdd
                // (counts[mat], 1)` for every non-SENTINEL pixel's material id.
                // SAFETY: recording is open; `record_vb_pass` records the graph's derived
                // COLOR_ATTACHMENT_OPTIMAL->SHADER_READ_ONLY_OPTIMAL barrier (`vb_id`, the FIRST
                // reader this frame — `vb_shade`'s later same-layout read needs none) + the
                // TRANSFER_WRITE->SHADER_READ|SHADER_WRITE barrier (`gclassify`, chained from
                // `vb_classify_fill`) for the "vb_classify_count" pass.
                self.record_vb_pass(
                    plan.vb_classify_count.expect(
                        "invariant: mesh_leg && vb_use_classified => vb_classify_count pass declared (declare_vb_graph)",
                    ),
                    cmd,
                    targets,
                    forward,
                    vb,
                    scene,
                    fi,
                );
                let vb_classify_count_pipeline = scene
                    .vb_classify_count_pipeline
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_classify_count_pipeline");
                // SAFETY: recording is open; `vb_classify_count_pipeline` (1-set, built against
                // `vb_layout0`) belongs to this device (caller contract); `vb_set0[fi]` is a live
                // descriptor set; `scene.dispatch_group_count_x` covers `present_extent.width *
                // present_extent.height` pixels at the shader's `numthreads(64,1,1)` 1D grid (the
                // SAME grid `vb_shade` dispatches at). The pipeline's push-constant range (4 bytes,
                // declared but unread by this shader — the shared compute-push-range convention this
                // RHI mandates, `DdgiUpdateActivation`'s own doc) is never written; nothing pushes.
                unsafe {
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, vb_classify_count_pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_classify_count_pipeline.layout,
                        0,
                        1,
                        &vb_set0[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
                }

                // Pass `vb_classify_scan`: a single workgroup performing the two chained
                // exclusive-prefix-sum phases (`counts -> offsets`/`cursors`, `gc -> gbase` +
                // `group_to_mat` fill) over the LIVE `[0, material_count)` prefix.
                // SAFETY: recording is open; `record_vb_pass` records the graph's derived
                // SHADER_WRITE->SHADER_READ|SHADER_WRITE barrier (`gclassify`, chained from
                // `vb_classify_count`) for the "vb_classify_scan" pass — P1-3.
                self.record_vb_pass(
                    plan.vb_classify_scan.expect(
                        "invariant: mesh_leg && vb_use_classified => vb_classify_scan pass declared (declare_vb_graph)",
                    ),
                    cmd,
                    targets,
                    forward,
                    vb,
                    scene,
                    fi,
                );
                let vb_classify_scan_pipeline = scene
                    .vb_classify_scan_pipeline
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_classify_scan_pipeline");
                // SAFETY: recording is open; `vb_classify_scan_pipeline`'s 4-byte push constant
                // (`PushConstants { uint material_count; }`, `vb_classify_scan.comp.hlsl`) is
                // written from `scene.vb_classify_material_count` (a plain `u32` local, `'static`
                // for the duration of this call); the pointer is valid and the push call happens
                // before the dispatch reads it.
                unsafe {
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, vb_classify_scan_pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_classify_scan_pipeline.layout,
                        0,
                        1,
                        &vb_set0[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_push_constants)(
                        cmd,
                        vb_classify_scan_pipeline.layout,
                        VK_SHADER_STAGE_COMPUTE_BIT,
                        0,
                        4,
                        (&scene.vb_classify_material_count as *const u32).cast(),
                    );
                    (self.fns.cmd_dispatch)(cmd, 1, 1, 1);
                }

                // Pass `vb_classify_scatter`: one thread per composite pixel, claims a slot in its
                // material's `pixel_list` region and stores the pixel's linear index there.
                // SAFETY: recording is open; `record_vb_pass` records the graph's derived
                // SHADER_WRITE->SHADER_READ|SHADER_WRITE barrier (`gclassify`, chained from
                // `vb_classify_scan`) + the (already-SHADER_READ_ONLY_OPTIMAL, same-layout, no-op)
                // `vb_id` read for the "vb_classify_scatter" pass.
                self.record_vb_pass(
                    plan.vb_classify_scatter.expect(
                        "invariant: mesh_leg && vb_use_classified => vb_classify_scatter pass declared (declare_vb_graph)",
                    ),
                    cmd,
                    targets,
                    forward,
                    vb,
                    scene,
                    fi,
                );
                let vb_classify_scatter_pipeline = scene.vb_classify_scatter_pipeline.expect(
                    "invariant: a VisibilityBuffer-resolved scene always carries vb_classify_scatter_pipeline",
                );
                // SAFETY: recording is open; same contract as `vb_classify_count_pipeline` above —
                // 1-set pipeline, `vb_set0[fi]` live, `dispatch_group_count_x` covers every pixel;
                // this shader also declares no push constant, so nothing pushes.
                unsafe {
                    (self.fns.cmd_bind_pipeline)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_classify_scatter_pipeline.pipeline,
                    );
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_classify_scatter_pipeline.layout,
                        0,
                        1,
                        &vb_set0[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
                }
            }

            // === The `lit`-producer choice (plan P1-4) is THREE-way, not two: the split
            // `vb_shade_split` (below, when `scene.path_vb_split()`) DISPLACES both of the
            // other two — `vb_shade` (material-classified, when `scene.vb_use_classified`) and
            // the fused `vb_resolve` are mutually exclusive WITH EACH OTHER (exactly one of
            // THIS PAIR runs whenever `!scene.path_vb_split()`), but NEITHER runs on a split
            // frame — mirrors `declare_vb_graph`'s matching branch.
            //
            // VB-P1d: this three-way split is why the bench's `VbTimedPass::VbShade` bracket
            // used to be recorded in ONLY the classified/fused arms (below) — a split frame RESET
            // a query pair it then never WROTE, and the `VK_QUERY_RESULT_WAIT_BIT` readback would
            // block forever on it. That hazard was held off by a caller-side precondition alone.
            //
            // It is now closed AT THE RECORDER: the split arm (further down, at the
            // `vb_shade_split` dispatch) carries the SAME `VbTimedPass::VbShade` pair, so the
            // bracket covers whichever of the THREE lit producers a frame selects and exactly one
            // begin/end pair is written per mesh-leg frame in every branch. A precondition on one
            // caller cannot protect a second one; writing the pair in every arm can.
            //
            // `boyko_app::runner`'s VB-P1d block keeps its `!mesh_geo_shade_split` assertion: it
            // is no longer a hang guard (the pair IS written on a split frame now) but a SCOPE
            // statement — VB-P1d's `flat_shade_ns` vs `froxel_total_ns` break-even is defined
            // against the fused/classified tail, and silently admitting a third producer would
            // change what its number means. ===
            if scene.path_vb_split() {
                // Rung R9b: the split DISPLACES the fused lit producer — `vb_shade_split`
                // (recorded in the split arm after this block) is the sole lit producer;
                // neither `vb_shade` nor `vb_resolve` records (mirrors the declarator). Its
                // `VbTimedPass::VbShade` bracket is recorded THERE, not here.
            } else if scene.vb_use_classified {
                // VB-P1d: open the VbShade bracket — this branch is the classified lit
                // producer, mutually exclusive with the fused `vb_resolve` branch below (exactly
                // one of the two ever records per frame), so the SAME `VbTimedPass::VbShade`
                // pair is written by whichever runs. GATED.
                if let Some(tc) = scene.vb_gpu_timing {
                    // SAFETY: recording is open; `self.fns` is the live device fn-table; the
                    // pool was reset at the frame top; `fi` is this present's in-flight slot.
                    unsafe { tc.write_begin(self.fns, cmd, fi, VbTimedPass::VbShade) };
                }
                // === Pass `vb_shade`: the material-classified shading compute pass (VB-P2
                // classification plan rung P2c) — re-fetches geometry via the Decision-0 table
                // (Set 2) for each classify-table pixel, shades, writes `lit` (STORAGE, extending
                // `vb_sky`'s COLOR write, C5). Byte-identical to `vb_resolve`'s own shading tail by
                // construction (plan D3). ===
                // SAFETY: recording is open; `record_vb_pass` records the graph's derived
                // (already-SHADER_READ_ONLY_OPTIMAL, no-op) `vb_id` read + the SHADER_WRITE->
                // SHADER_READ `gclassify` barrier (chained from `vb_classify_scatter`) + the
                // COLOR_ATTACHMENT_OPTIMAL→GENERAL barrier for `lit` (+ the cascade/atlas
                // →SHADER_READ_ONLY_OPTIMAL barriers when armed) for the "vb_shade" pass into `cmd`.
                self.record_vb_pass(
                    plan.vb_shade.expect(
                        "invariant: mesh_leg && vb_use_classified => vb_shade pass declared (declare_vb_graph)",
                    ),
                    cmd,
                    targets,
                    forward,
                    vb,
                    scene,
                    fi,
                );

                // Textured-PBR rung TV0 (`RENDER-PARITY-PLAN.md` §2.3): select the TEXTURED
                // `vb_shade` pipeline + its OWN Set-0 vocabulary set (`vb_set0_tex`, the wider
                // `PerInstanceMaterialTex` ring at b1) when this frame's VB gather bound a
                // non-zero material texture slot (`GBufferScene::vb_tex_active`'s own doc) — the
                // base classified pipeline/set otherwise. Mutually exclusive by construction
                // (`vb_tex_active` implies `vb_use_classified`, since it feeds that selector's
                // OR-in at the `GpuSceneBundles::scene()` assembly seam).
                //
                // VB-P1c closes the VB-P1a scope cut: `textured`/`froxel` are INDEPENDENT axes
                // (`vb_tex_active` never reads `cluster_cull`), so all four combinations select
                // their own pipeline + Set-0 — `vb_set0_tex_froxel` (the TEXTURED+FROXEL combined
                // Set-0) exists exactly for the `(true, true)` cell. ⚠️ **The arm bit is
                // default-OFF, not hardcoded off, and this comment said the latter.**
                // `froxel_light_cull = clusters_wanted && path == VisibilityBuffer`, and
                // `clusters_wanted` is the owner's `LightingConfig::clusters_enabled` (default
                // `false`). So the `(true, false)`/`(false, false)` cells are the only ones a
                // DEFAULT boot reaches — but `vb_mesh_froxel` and `vb_mesh_tex_froxel` set the
                // flag `true`, reach the froxel cells, and are golden-pinned with screenshot
                // dumps. Six sibling comments said the same false thing; VB-P1b armed the cull
                // and the repair landed in the code and in one comment, not in the other seven.
                let textured = scene.vb_tex_active();
                let froxel = scene.cluster_cull.is_some();
                let (vb_shade_pipeline, vb_shade_set0) = match (textured, froxel) {
                    (true, true) => (
                        scene.vb_shade_tex_froxel_pipeline.expect(
                            "invariant: vb_tex_active() && cluster_cull.is_some() => scene.vb_shade_tex_froxel_pipeline is Some",
                        ),
                        targets.vb_set0_tex_froxel.as_ref().expect(
                            "invariant: vb_tex_active() && cluster_cull.is_some() => targets.vb_set0_tex_froxel is built",
                        ),
                    ),
                    (true, false) => (
                        scene.vb_shade_tex_pipeline.expect(
                            "invariant: GBufferScene::vb_tex_active() => scene.vb_shade_tex_pipeline is Some",
                        ),
                        targets.vb_set0_tex.as_ref().expect(
                            "invariant: GBufferScene::vb_tex_active() => targets.vb_set0_tex is built",
                        ),
                    ),
                    (false, true) => (
                        scene.vb_shade_froxel_pipeline.expect(
                            "invariant: scene.cluster_cull.is_some() => scene.vb_shade_froxel_pipeline is Some",
                        ),
                        targets.vb_set0_froxel.as_ref().expect(
                            "invariant: scene.cluster_cull.is_some() => targets.vb_set0_froxel is built",
                        ),
                    ),
                    (false, false) => (
                        scene.vb_shade_pipeline.expect(
                            "invariant: a VisibilityBuffer-resolved scene always carries vb_shade_pipeline",
                        ),
                        vb_set0,
                    ),
                };
                let vb_geometry_set = scene
                    .vb_geometry_set
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_geometry_set");
                // SAFETY: recording is open; `vb_shade_pipeline` (the compute pipeline: Set 0 =
                // `vb_shade_set0[fi]` (`vb_set0[fi]`, or `vb_set0_tex[fi]`/`vb_set0_froxel[fi]`/
                // `vb_set0_tex_froxel[fi]` per the `(textured, froxel)` match above), Set 1 =
                // `forward.set1[fi]` — the Forward-family shadow set REUSED VERBATIM, Set 2 =
                // `vb_geometry_set` — the Decision-0 geometry table, bound directly, no ring, the
                // SAME triple `vb_resolve_pipeline` binds — PLUS, when `textured`, a 4th Set 3 =
                // `scene.bindless_set` — the shared bindless texture-array table, its LAYOUT already
                // baked into the TEXTURED pipeline's `VkPipelineLayout` at boot, mirroring
                // `record_gbuffer`'s own `tex_active` bindless-set bind) belongs to this device
                // (caller contract); `scene.mvp`'s leading 64 bytes are the SAME `view_proj` matrix
                // `vb_resolve.comp.hlsl`'s push constant reads (`vb_shade.comp.hlsl`'s push is the
                // identical 64-byte shape regardless of `-D TEXTURED`, plan D3); `scene.dispatch_group_count_x +
                // scene.vb_classify_material_count` is the D2 over-dispatch (`G +
                // present_material_count` groups — the classify chain's `scan` pass populated
                // `group_to_mat[0..total_groups)` with real material ids and left
                // `[total_groups, G+MAX)` SENTINEL from `fill`; `vb_shade`/`vb_shade_tex` early-outs
                // on every surplus group's SENTINEL read).
                unsafe {
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, vb_shade_pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_shade_pipeline.layout,
                        0,
                        1,
                        &vb_shade_set0[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_shade_pipeline.layout,
                        1,
                        1,
                        &forward.set1[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_shade_pipeline.layout,
                        2,
                        1,
                        &vb_geometry_set.set(),
                        0,
                        ptr::null(),
                    );
                    if textured {
                        let bindless_set = scene
                            .bindless_set
                            .expect("invariant: GBufferScene::vb_tex_active() => scene.bindless_set is Some");
                        (self.fns.cmd_bind_descriptor_sets)(
                            cmd,
                            VK_PIPELINE_BIND_POINT_COMPUTE,
                            vb_shade_pipeline.layout,
                            3,
                            1,
                            &bindless_set,
                            0,
                            ptr::null(),
                        );
                    }
                    (self.fns.cmd_push_constants)(
                        cmd,
                        vb_shade_pipeline.layout,
                        VK_SHADER_STAGE_COMPUTE_BIT,
                        0,
                        64,
                        scene.mvp.as_ptr().cast(),
                    );
                    (self.fns.cmd_dispatch)(
                        cmd,
                        scene.dispatch_group_count_x + scene.vb_classify_material_count,
                        1,
                        1,
                    );
                }
                // VB-P1d: close the VbShade bracket. GATED.
                if let Some(tc) = scene.vb_gpu_timing {
                    // SAFETY: recording is open; the pool was reset this frame; `fi` is this slot.
                    unsafe { tc.write_end(self.fns, cmd, fi, VbTimedPass::VbShade) };
                }
            } else {
                // VB-P1d: open the VbShade bracket — the fused lit producer, mutually exclusive
                // with the classified branch above (see that branch's own VB-P1d comment). GATED.
                if let Some(tc) = scene.vb_gpu_timing {
                    // SAFETY: recording is open; `self.fns` is the live device fn-table; the
                    // pool was reset at the frame top; `fi` is this present's in-flight slot.
                    unsafe { tc.write_begin(self.fns, cmd, fi, VbTimedPass::VbShade) };
                }
                // === Pass `vb_resolve`: the FUSED resolve compute pass (Decision 5) — reads `vb_id`,
                // re-fetches geometry via the Decision-0 table (Set 2), shades, writes `lit` (STORAGE,
                // extending `vb_sky`'s COLOR write, C5). ===
                // SAFETY: recording is open; `record_vb_pass` records the graph's derived
                // COLOR_ATTACHMENT_OPTIMAL→SHADER_READ_ONLY_OPTIMAL barrier for `vb_id` + the
                // COLOR_ATTACHMENT_OPTIMAL→GENERAL barrier for `lit` (+ the cascade/atlas
                // →SHADER_READ_ONLY_OPTIMAL barriers when armed) for the "vb_resolve" pass into `cmd`.
                self.record_vb_pass(
                    plan.vb_resolve.expect(
                        "invariant: mesh_leg && !vb_use_classified => vb_resolve pass declared (declare_vb_graph)",
                    ),
                    cmd,
                    targets,
                    forward,
                    vb,
                    scene,
                    fi,
                );

                // VB-P1a ("dark infra"): select the FROXEL-variant pipeline + its OWN WIDER
                // Set-0 (`vb_set0_froxel`, 11 bindings) when the arm is built
                // (`scene.cluster_cull.is_some()` — ⚠️ default-OFF, not hardcoded off, so this is
                // the base arm on an unarmed boot and the froxel arm on `vb_mesh_froxel`'s),
                // else the base `vb_resolve_pipeline` +
                // `vb_set0` — mutually exclusive by construction (mirrors `vb_shade`'s own
                // `textured` selector immediately above).
                let froxel = scene.cluster_cull.is_some();
                let (vb_resolve_pipeline, vb_resolve_set0) = if froxel {
                    (
                        scene.vb_resolve_froxel_pipeline.expect(
                            "invariant: scene.cluster_cull.is_some() => scene.vb_resolve_froxel_pipeline is Some",
                        ),
                        targets.vb_set0_froxel.as_ref().expect(
                            "invariant: scene.cluster_cull.is_some() => targets.vb_set0_froxel is built",
                        ),
                    )
                } else {
                    (
                        scene.vb_resolve_pipeline.expect(
                            "invariant: a VisibilityBuffer-resolved scene always carries vb_resolve_pipeline",
                        ),
                        vb_set0,
                    )
                };
                let vb_geometry_set = scene
                    .vb_geometry_set
                    .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_geometry_set");
                // SAFETY: recording is open; `vb_resolve_pipeline` (the 3-set compute pipeline: Set 0 =
                // `vb_resolve_set0[fi]` (`vb_set0[fi]` or, when `froxel`, `vb_set0_froxel[fi]`), Set 1 =
                // `forward.set1[fi]` — the Forward-family shadow set REUSED VERBATIM, Set 2 =
                // `vb_geometry_set` — the Decision-0 geometry table, bound directly,
                // no ring) belongs to this device (caller contract); `scene.mvp` is the SAME 88-byte
                // push whose leading 64 bytes are the `view_proj` matrix `vb_resolve.comp.hlsl`'s
                // push constant reads (the SAME matrix `vb_raster.vs.hlsl` used, `GBUFFER_PUSH_BYTES`
                // layout parity); `scene.dispatch_group_count_x` covers `present_extent.width *
                // present_extent.height` pixels at the shader's `numthreads(64,1,1)` 1D grid (the SAME
                // grid the deferred marcher/resolve/`sdf_forward_march` dispatch at).
                unsafe {
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, vb_resolve_pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_resolve_pipeline.layout,
                        0,
                        1,
                        &vb_resolve_set0[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_resolve_pipeline.layout,
                        1,
                        1,
                        &forward.set1[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vb_resolve_pipeline.layout,
                        2,
                        1,
                        &vb_geometry_set.set(),
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_push_constants)(
                        cmd,
                        vb_resolve_pipeline.layout,
                        VK_SHADER_STAGE_COMPUTE_BIT,
                        0,
                        64,
                        scene.mvp.as_ptr().cast(),
                    );
                    (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
                }
                // VB-P1d: close the VbShade bracket. GATED.
                if let Some(tc) = scene.vb_gpu_timing {
                    // SAFETY: recording is open; the pool was reset this frame; `fi` is this slot.
                    unsafe { tc.write_end(self.fns, cmd, fi, VbTimedPass::VbShade) };
                }
            }
        }

        // === VG R3 piece 1 steps P1-8/P1-5 + VG R3 piece 2 step P2-5 (plan D6): the
        // `[hzb_poison, hzb_build_*]` block's UNSPLIT slot — after the `lit` producer, before the
        // split arm's `vb_viewt` PRE-TAIL slot below. ===
        //
        // The position the block has held since piece 1, and the one it keeps on every frame the
        // occlusion split is not armed — which is every scene in this tree today. Recorded in
        // EXACTLY `declare_vb_graph`'s matching slot; the two sites read ONE predicate, so they
        // cannot disagree about which slot this frame uses.
        //
        // ⚠️ An UNARMED frame records ZERO commands here — no clear, no barrier, no dispatch —
        // which is what keeps every golden pin byte-identical.
        if !occlusion_split {
            self.record_hzb_poison_build(plan, cmd, targets, forward, vb, scene, present_extent, fi);
        }

        // === Rung R9b: the SPLIT arm — recorded in EXACTLY `declare_vb_graph`'s order:
        // vb_viewt(pre-tail, iff ssao armed) → vb_geo → ssao gather → à-trous×N →
        // vb_shade_split. `sdf_forward_march` (below) then extends the split shade's `lit`
        // write under `Both`, exactly as it extends the fused producer's. ===
        if scene.path_vb_split() {
            // (1) The gViewT producer's PRE-TAIL slot (the gather's ray-metric depth source).
            if scene.ssao.is_some()
                && let Some(vb_viewt_pass) = plan.viewt_from_depth
            {
                self.record_vb_viewt_dispatch(vb_viewt_pass, cmd, targets, forward, vb, scene, present_extent, fi);
            }

            // (2) Pass `vb_geo` — the thin-aux producer: the first `vb_id` reader under split
            // (derives COLOR→SHADER_READ_ONLY), writes `thin_normal` (first-touch
            // UNDEFINED→GENERAL). SAFETY: recording is open; `record_vb_pass` records those
            // derived barriers for the "vb_geo" pass into `cmd`.
            let geo_pass = plan
                .vb_geo
                .expect("invariant: path_vb_split() => vb_geo pass declared (declare_vb_graph)");
            self.record_vb_pass(geo_pass, cmd, targets, forward, vb, scene, fi);
            // Rung R9d: select the `-D MOTION=1` sibling when the hwrt shadow chain's temporal
            // stage is armed this frame (`vb_geo_mv_active()` — the O1 single predicate
            // `declare_vb_graph`'s conditional `motion_vec` write also reads), else the base
            // pipeline (byte-identical to R9b).
            #[cfg(feature = "hwrt")]
            let vb_geo_pipeline = if scene.vb_geo_mv_active() {
                scene.vb_geo_mv_pipeline.expect(
                    "invariant: vb_geo_mv_active() ⇒ scene.vb_geo_mv_pipeline is Some (build_vb_split_pipelines ran on an RT device)",
                )
            } else {
                scene
                    .vb_geo_pipeline
                    .expect("invariant: a split-armed scene carries vb_geo_pipeline (build_vb_split_pipelines ran)")
            };
            #[cfg(not(feature = "hwrt"))]
            let vb_geo_pipeline = scene
                .vb_geo_pipeline
                .expect("invariant: a split-armed scene carries vb_geo_pipeline (build_vb_split_pipelines ran)");
            let vb_geometry_set = scene
                .vb_geometry_set
                .expect("invariant: a VisibilityBuffer-resolved scene always carries vb_geometry_set");
            let vb_set0 = targets
                .vb_set0
                .as_ref()
                .expect("invariant: a VisibilityBuffer-resolved scene always builds vb_set0");
            let vb_geo_aux_set = targets
                .vb_geo_aux_set
                .as_ref()
                .expect("invariant: a split-armed boot builds vb_geo_aux_set (targets tail)");
            // SAFETY: recording is open; `vb_geo_pipeline` (3-set: Set 0 = `vb_set0[fi]` — the
            // BASE ring, `vb_geo` samples no textures; Set 1 = `vb_geo_aux_set[fi]`; Set 2 =
            // the geometry table) belongs to this device; the 64-byte push is `scene.mvp`'s
            // leading `view_proj` (the `vb_resolve` shape); `dispatch_group_count_x` covers the
            // pixel count at `numthreads(64,1,1)`.
            unsafe {
                (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, vb_geo_pipeline.pipeline);
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    vb_geo_pipeline.layout,
                    0,
                    1,
                    &vb_set0[fi].descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    vb_geo_pipeline.layout,
                    1,
                    1,
                    &vb_geo_aux_set[fi].descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    vb_geo_pipeline.layout,
                    2,
                    1,
                    &vb_geometry_set.set(),
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_push_constants)(
                    cmd,
                    vb_geo_pipeline.layout,
                    VK_SHADER_STAGE_COMPUTE_BIT,
                    0,
                    64,
                    scene.mvp.as_ptr().cast(),
                );
                (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
            }

            // (3) The SSAO gather + à-trous chain (`path_vb_ssao` — the declare-site predicate).
            if scene.path_vb_ssao() {
                let ssao_pass = plan
                    .vb_ssao
                    .expect("invariant: path_vb_ssao() => the VB ssao pass was declared");
                // SAFETY: recording is open; `record_vb_pass` records the thin_normal/viewt
                // store→load + the `ssao` seed-inert first-touch barriers for the "ssao" pass.
                self.record_vb_pass(ssao_pass, cmd, targets, forward, vb, scene, fi);
                let gather_pipeline = scene
                    .ssao_vb_pipeline
                    .expect("invariant: path_vb_ssao() => scene.ssao_vb_pipeline is Some");
                let vb_ssao_set = targets
                    .vb_ssao_set
                    .as_ref()
                    .expect("invariant: a split-armed boot builds vb_ssao_set (targets tail)");
                // SAFETY: recording is open; the VB_THIN gather reads its camera from the UBO
                // @3, so no push constant is recorded (the deferred gather's own discipline);
                // `dispatch_group_count_x` covers the pixel count.
                unsafe {
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, gather_pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        gather_pipeline.layout,
                        0,
                        1,
                        &vb_ssao_set[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
                }

                // The à-trous chain — the deferred record loop MIRRORED (gbuffer.rs's own
                // block, incl. the graceful set-degrade + the 0-||-2..=MAX contract assert);
                // the role-keyed SETS are the SAME core-ring sets (core.viewt/ssao/rings are
                // path-shared), so no VB-specific set plumbing exists here.
                if let Some(activation) = &scene.ssao
                    && activation.atrous_levels > 0
                    && let (
                        Some(read8_set),
                        Some(interior_from0_set),
                        Some(interior_from1_set),
                        Some(write8_from0_set),
                        Some(write8_from1_set),
                    ) = (
                        targets.ssao_atrous_read8_set.as_ref(),
                        targets.ssao_atrous_interior_from0_set.as_ref(),
                        targets.ssao_atrous_interior_from1_set.as_ref(),
                        targets.ssao_atrous_write8_from0_set.as_ref(),
                        targets.ssao_atrous_write8_from1_set.as_ref(),
                    )
                {
                    let read8_pipeline = scene
                        .ssao_atrous_read8_pipeline
                        .expect("invariant: the SSAO à-trous sets built ⇒ the boot read8 pipeline is Some");
                    let interior_pipeline = scene.ssao_atrous_interior_pipeline.expect(
                        "invariant: the SSAO à-trous sets built ⇒ the boot interior pipeline is Some",
                    );
                    let write8_pipeline = scene
                        .ssao_atrous_write8_pipeline
                        .expect("invariant: the SSAO à-trous sets built ⇒ the boot write8 pipeline is Some");
                    let atrous_levels =
                        activation.atrous_levels.min(crate::present::MAX_SSAO_ATROUS_LEVELS);
                    debug_assert!(
                        atrous_levels == 0
                            || (2..=crate::present::MAX_SSAO_ATROUS_LEVELS).contains(&atrous_levels),
                        "invariant: ssao à-trous levels is 0 or 2..=MAX at the RHI boundary; got {atrous_levels}"
                    );
                    for level in 0..atrous_levels {
                        let atrous_pass = plan.ssao_atrous[level as usize].expect(
                            "invariant: level < ssao_atrous_levels ⇒ the VB ssao_atrous[level] declared",
                        );
                        // SAFETY: recording is open; the "ssao_atrous" pass's derived RAW
                        // barriers (gather-write→level-0-read, the ring ping-pong, the last
                        // level's write→shade-read) are recorded via the VB sink.
                        self.record_vb_pass(atrous_pass, cmd, targets, forward, vb, scene, fi);
                        let (pipeline, set) =
                            match crate::present::ssao_atrous_step(level, atrous_levels) {
                                crate::present::AtrousStepRole::Read8 => {
                                    (read8_pipeline, &read8_set[self.frame_index])
                                }
                                crate::present::AtrousStepRole::Interior { in_ring: 0 } => {
                                    (interior_pipeline, &interior_from0_set[self.frame_index])
                                }
                                crate::present::AtrousStepRole::Interior { .. } => {
                                    (interior_pipeline, &interior_from1_set[self.frame_index])
                                }
                                crate::present::AtrousStepRole::Write8 { in_ring: 0 } => {
                                    (write8_pipeline, &write8_from0_set[self.frame_index])
                                }
                                crate::present::AtrousStepRole::Write8 { .. } => {
                                    (write8_pipeline, &write8_from1_set[self.frame_index])
                                }
                            };
                        let step: u32 = 1u32 << level;
                        // SAFETY: recording is open; the selected variant + its shared 4-binding
                        // layout are live; the 4-byte `{ uint step }` push covers the declared
                        // COMPUTE range; `dispatch_group_count_x` covers the pixel count.
                        unsafe {
                            (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline.pipeline);
                            (self.fns.cmd_bind_descriptor_sets)(
                                cmd,
                                VK_PIPELINE_BIND_POINT_COMPUTE,
                                pipeline.layout,
                                0,
                                1,
                                &set.descriptor_set,
                                0,
                                ptr::null(),
                            );
                            (self.fns.cmd_push_constants)(
                                cmd,
                                pipeline.layout,
                                VK_SHADER_STAGE_COMPUTE_BIT,
                                0,
                                4,
                                (&step as *const u32).cast(),
                            );
                            (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
                        }
                    }
                }
            }

            // (3.6, rung R9d) The VB split's hardware shadow chain — TLAS pack/build + the RT
            // soft-shadow VIS pre-pass + the `levels` à-trous passes + the temporal reproject.
            // Recorded in the SAME order `declare_vb_graph` declares them (after the SSAO/
            // à-trous chain, before `ddgi_update`). Reuses the DEFERRED chain's shared pipeline
            // objects (`scene.shadow`'s own `atrous_pipeline`/`temporal_pipeline` — one pipeline
            // object, many per-path callers) against the split's OWN dedicated sets
            // (`thin_normal`/`viewt` instead of the fat `gNormal`/`gViewT`). The tlas pack/build
            // body is `record_gbuffer`'s own port, adapted to `record_vb_pass` for barriers.
            #[cfg(feature = "hwrt")]
            if let (Some(t), Some(fns)) = (scene.tlas.as_ref(), accel_fns) {
                let pack_pass =
                    plan.tlas_pack.expect("invariant: scene.tlas.is_some() ⇒ tlas_pack declared");
                let build_pass =
                    plan.tlas_build.expect("invariant: scene.tlas.is_some() ⇒ tlas_build declared");
                // SAFETY: recording is open; `record_vb_pass` records the "tlas_pack" pass's
                // derived input barriers into `cmd` against the live scene buffers.
                self.record_vb_pass(pack_pass, cmd, targets, forward, vb, scene, fi);
                let groups = t.count.div_ceil(LOCAL_SIZE_X);
                let push = t.count.to_le_bytes();
                // SAFETY: recording is open; the packer pipeline + its layout are live on this
                // device (caller contract); `t.bind_group` binds this frame slot's pack inputs;
                // `groups` covers `t.count` at the 64-wide group; `&t.bind_group.descriptor_set`
                // is a single-element local alive for the call; the push is exactly
                // `BUILD_TLAS_INSTANCES_PUSH_BYTES` (4) at offset 0 and `push` outlives the call.
                unsafe {
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, t.pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        t.pipeline.layout,
                        0,
                        1,
                        &t.bind_group.descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_push_constants)(
                        cmd,
                        t.pipeline.layout,
                        VK_SHADER_STAGE_COMPUTE_BIT,
                        0,
                        crate::compute::BUILD_TLAS_INSTANCES_PUSH_BYTES,
                        push.as_ptr().cast(),
                    );
                    (self.fns.cmd_dispatch)(cmd, groups, 1, 1);
                }
                // SAFETY: recording is open; `record_vb_pass` records the "tlas_build" pass's
                // derived pack-WRITE → build-READ barrier on `tlas_instances`.
                self.record_vb_pass(build_pass, cmd, targets, forward, vb, scene, fi);
                let entry = boyko_rhi::AsBuildEntry {
                    kind: boyko_rhi::AsKind::Tlas,
                    geometry: boyko_rhi::AsGeometryDesc {
                        vertex_data: t.instance_array_addr,
                        index_data: 0,
                        vertex_stride: 0,
                        max_vertex: 0,
                        primitive_count: t.count,
                        index_type: boyko_rhi::AsIndexType::Uint32,
                    },
                    scratch_address: t.scratch_addr,
                };
                // SAFETY: recording is open; `fns` is the live device's AS table (resolved from
                // the RT `ctx`); `entry`'s `vertex_data` (the pack-written instance array) +
                // `scratch_address` + `t.dest.handle` are live, correctly-flagged resources; the
                // pack→build barrier just recorded orders the instance-array write before this
                // build's read; `entry`/`dest` are 1-element slices that outlive the call.
                unsafe {
                    crate::accel::cmd_build_acceleration_structures(
                        fns,
                        cmd,
                        core::slice::from_ref(&entry),
                        &[t.dest],
                    );
                }
                // SAFETY: recording is open; `self.fns` is the live device's core command table.
                // The barrier touches no resource beyond the execution/memory dependency
                // (AS_BUILD stage → COMPUTE_SHADER stage).
                unsafe {
                    crate::accel::cmd_acceleration_structure_barrier(self.fns, cmd);
                }
            }

            #[cfg(feature = "hwrt")]
            if let (Some(sh), Some(vis_set), Some(atrous_sets)) = (
                scene.shadow.as_ref(),
                targets.vb_shadow_vis_set.as_ref(),
                targets.vb_shadow_atrous_sets.as_ref(),
            ) {
                let vis_pipeline = scene.vb_shadow_vis_pipeline.expect(
                    "invariant: scene.shadow.is_some() under the split ⇒ vb_shadow_vis_pipeline is Some",
                );
                let vis_pass = plan
                    .shadow_vis
                    .expect("invariant: scene.shadow.is_some() ⇒ shadow_vis pass declared");
                // SAFETY: recording is open; `record_vb_pass` records the graph's derived input
                // barriers for the "shadow_vis" pass into `cmd`.
                self.record_vb_pass(vis_pass, cmd, targets, forward, vb, scene, fi);
                // SAFETY: recording is open; `vis_pipeline` + its 7-binding layout are live on
                // this device (caller contract); `vis_set[fi]` binds `thin_normal`/`viewt` +
                // `LightTable` + the camera UBO + the TLAS + the `ResolvedRayShadow` UBO +
                // `gShadowVis` (the write target); `dispatch_group_count_x` covers the pixel
                // count; the pipeline's declared 4-byte push is bound-but-unread (no push
                // recorded, mirroring the deferred VIS pass).
                unsafe {
                    (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, vis_pipeline.pipeline);
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        vis_pipeline.layout,
                        0,
                        1,
                        &vis_set[fi].descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
                }

                let atrous_levels =
                    (sh.atrous_levels as usize).min(crate::present::MAX_ATROUS_LEVELS as usize);
                debug_assert!(
                    atrous_sets.len() >= atrous_levels,
                    "invariant: the VB à-trous set array must hold at least `atrous_levels` levels"
                );
                debug_assert_eq!(
                    sh.final_is_vis2,
                    atrous_levels % 2 == 1,
                    "invariant: the VB denoised/temporal bind ring must match the last à-trous parity"
                );
                for (level, level_ring) in atrous_sets.iter().enumerate().take(atrous_levels) {
                    let atrous_pass = plan.shadow_atrous[level].expect(
                        "invariant: level < scene.shadow.atrous_levels ⇒ the VB shadow_atrous[level] declared",
                    );
                    let step: u32 = 1u32 << level;
                    // SAFETY: recording is open; `record_vb_pass` records the "shadow_atrous"
                    // pass's derived RAW barriers on the ping-pong pair into `cmd`.
                    self.record_vb_pass(atrous_pass, cmd, targets, forward, vb, scene, fi);
                    let atrous_set = &level_ring[self.frame_index];
                    // SAFETY: recording is open; the SHARED à-trous pipeline + its 6-binding
                    // layout (the deferred boot object) are live on this device; `atrous_set`
                    // binds `gVisIn`/`gVisOut` (the ping-pong pair) + `thin_normal`/`viewt` + the
                    // `ResolvedShadowDenoise` UBO + the camera UBO; the 4-byte `{ uint step }`
                    // push covers the pipeline's declared COMPUTE range; `dispatch_group_count_x`
                    // covers the pixel count.
                    unsafe {
                        (self.fns.cmd_bind_pipeline)(
                            cmd,
                            VK_PIPELINE_BIND_POINT_COMPUTE,
                            sh.atrous_pipeline.pipeline,
                        );
                        (self.fns.cmd_bind_descriptor_sets)(
                            cmd,
                            VK_PIPELINE_BIND_POINT_COMPUTE,
                            sh.atrous_pipeline.layout,
                            0,
                            1,
                            &atrous_set.descriptor_set,
                            0,
                            ptr::null(),
                        );
                        (self.fns.cmd_push_constants)(
                            cmd,
                            sh.atrous_pipeline.layout,
                            VK_SHADER_STAGE_COMPUTE_BIT,
                            0,
                            4,
                            (&step as *const u32).cast(),
                        );
                        (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
                    }
                }
            }

            #[cfg(feature = "hwrt")]
            if scene.path_vb_hwrt_shadow()
                && scene.temporal_active()
                && let Some(temporal_sets) = targets.vb_shadow_temporal_set.as_ref()
            {
                let sh = scene
                    .shadow
                    .as_ref()
                    .expect("invariant: temporal_active() implies scene.shadow.is_some()");
                let temporal_pipeline = sh.temporal_pipeline.expect(
                    "invariant: temporal_active() + the VB temporal set built implies the temporal pipeline",
                );
                let temporal_pass = plan.shadow_temporal.expect(
                    "invariant: scene.temporal_active() under the split ⇒ shadow_temporal pass declared",
                );
                // SAFETY: recording is open; `record_vb_pass` records the "shadow_temporal"
                // pass's derived input/RAW barriers into `cmd`.
                self.record_vb_pass(temporal_pass, cmd, targets, forward, vb, scene, fi);
                let temporal_set = &temporal_sets[self.frame_index];
                // SAFETY: recording is open; the SHARED temporal pipeline + its 8-binding layout
                // (the deferred boot object) are live on this device; `temporal_set` binds
                // gVisIn/gMotionVec/gViewT/gHistIn/gHistOut/gTemporalOut + the
                // ResolvedTemporalShadow UBO + the camera UBO for `frame_index`;
                // `dispatch_group_count_x` covers the pixel count; the pipeline's declared 4-byte
                // COMPUTE range is bound-but-unread (no push recorded).
                unsafe {
                    (self.fns.cmd_bind_pipeline)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        temporal_pipeline.pipeline,
                    );
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        temporal_pipeline.layout,
                        0,
                        1,
                        &temporal_set.descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
                }
            }

            // (3.5, rung R9c) Pass `ddgi_update` — the probe update under VB (reachable only
            // VB×Both, `path_vb_ddgi()`): the DEFERRED record body mirrored (the single
            // non-ringed 7-binding set; the shader reads all params from the b6 UBO — no push).
            if scene.path_vb_ddgi() {
                let activation = scene
                    .ddgi_update
                    .as_ref()
                    .expect("invariant: path_vb_ddgi() ⇒ scene.ddgi_update.is_some()");
                let ddgi_update_set = targets
                    .ddgi_update_set
                    .as_ref()
                    .expect("invariant: scene.ddgi_update is Some ⇒ GBufferTargets wrote ddgi_update_set");
                let ddgi_pass = plan
                    .ddgi_update
                    .expect("invariant: path_vb_ddgi() ⇒ the VB ddgi_update pass was declared");
                // SAFETY: recording is open; `record_vb_pass` records the derived light/ray/
                // classification reads + the content-preserving SRO→GENERAL atlas transitions.
                self.record_vb_pass(ddgi_pass, cmd, targets, forward, vb, scene, fi);
                // SAFETY: recording is open; the update pipeline + its 7-binding layout are
                // live (caller contract); `dispatch_group_count_x` is the activation's own
                // probe-subset block count; no push (the b6 UBO carries every param).
                unsafe {
                    (self.fns.cmd_bind_pipeline)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        activation.pipeline.pipeline,
                    );
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        activation.pipeline.layout,
                        0,
                        1,
                        &ddgi_update_set.descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_dispatch)(cmd, activation.dispatch_group_count_x, 1, 1);
                }
            }

            // (4) Pass `vb_shade_split` — the split's lit producer (RE-fetch + shade + the
            // unconditional gSsao read + the rung-R9c header-gated DDGI probe sample). Per-frame
            // base/`_tex` pick mirrors the fused
            // `vb_resolve`/`vb_shade_tex` selection (boot-frozen split, per-frame textures).
            let shade_pass = plan
                .vb_shade_split
                .expect("invariant: path_vb_split() => vb_shade_split pass declared");
            // Open the VbShade bracket on the SPLIT lit producer. The three producer arms are
            // mutually exclusive (the `path_vb_split()` early arm above records neither of the
            // other two), so exactly ONE begin/end pair of this slot is written per mesh-leg frame
            // whichever branch runs — which is what keeps the `WAIT_BIT` readback from blocking
            // and what lets one collector serve all three rows. Opened BEFORE `record_vb_pass` so
            // the bracket spans the same "derived barriers + bind + dispatch" extent the
            // classified/fused arms measure; a bracket that started after the barriers would not
            // be comparable to the other two rows. GATED on `scene.vb_gpu_timing`: `None` on every
            // golden/host/interactive frame records NOTHING, so the command stream stays
            // byte-identical to the path that had no bracket here.
            if let Some(tc) = scene.vb_gpu_timing {
                // SAFETY: recording is open; `self.fns` is the live device fn-table; the pool was
                // reset at the frame top (`reset_frame`); `fi` is this present's in-flight slot.
                unsafe { tc.write_begin(self.fns, cmd, fi, VbTimedPass::VbShade) };
            }
            // SAFETY: recording is open; `record_vb_pass` records the `lit`
            // COLOR_ATTACHMENT→GENERAL + cascade/atlas + `ssao` GENERAL-read barriers for the
            // "vb_shade_split" pass into `cmd`.
            self.record_vb_pass(shade_pass, cmd, targets, forward, vb, scene, fi);
            // Rung R9d: extend the base/`_tex` pick to the 2×2 matrix — the `-D HWRT=1` variants
            // when the hwrt shadow chain is armed (`path_vb_hwrt_shadow()`, the SAME predicate
            // the declare-site's conditional denoised-vis read uses), else the software pair
            // exactly as shipped.
            #[cfg(feature = "hwrt")]
            let (shade_pipeline, shade_set0) = if scene.path_vb_hwrt_shadow() {
                // docs/SHADER-VARIANT-MANIFEST.md's `vb_shade_split_*hwrt` reachability note,
                // made mechanical AT the selection site (it previously rested on the boot
                // resolver's predicate alone).
                //
                // What this guards is variant-matrix COMPLETENESS, not a defect in these two
                // rows: `vb_shade_split.comp.hlsl` has no `sdf_soft_shadow` arm in ANY of its
                // four variants, so MESH pixels under VB receive no SDF-cast shadow whichever row
                // is bound — a v1 scope cut, deliberate. Scoped to mesh pixels on purpose: a
                // VB×Both / VB×Sdf frame records the same `sdf_forward_march` compute pass the
                // Forward family uses, and that pass DOES march a soft shadow for the SDF leg's
                // own pixels, so "a VB frame never combines an SDF-march source" is false. The
                // invariant
                // is that the resolver must never RECORD a combination the shipped rows cannot
                // express: `SDF_SOFT_MARCH` armed alongside `HWRT_VIS` would be a shadow source
                // that binds cleanly, raises no validation message, and is silently ignored.
                // The resolver guarantees it today
                // (`ShadowSources::hwrt_vis_excludes_sdf_soft_march`); this catches a carrier
                // that reached the recorder from anywhere else — a hand-built `GBufferScene`
                // fixture, or a future per-frame re-resolve.
                debug_assert!(
                    !scene.shadow_has_sdf_soft_march(),
                    "invariant (Decision 7): the vb_shade_split HWRT variants have no SDF-march \
                     arm, so SDF_SOFT_MARCH must not be armed while they are bound (shadow bits \
                     {:#06b})",
                    scene.resolved_render_path.shadow
                );
                if scene.vb_tex_active() {
                    (
                        scene.vb_shade_split_tex_hwrt_pipeline.expect(
                            "invariant: vb_tex_active() + path_vb_hwrt_shadow() requires vb_shade_split_tex_hwrt_pipeline",
                        ),
                        targets.vb_set0_tex.as_ref().expect(
                            "invariant: GBufferScene::vb_tex_active() => targets.vb_set0_tex is built",
                        ),
                    )
                } else {
                    (
                        scene.vb_shade_split_hwrt_pipeline.expect(
                            "invariant: path_vb_hwrt_shadow() requires vb_shade_split_hwrt_pipeline",
                        ),
                        vb_set0,
                    )
                }
            } else if scene.vb_tex_active() {
                (
                    scene.vb_shade_split_tex_pipeline.expect(
                        "invariant: vb_tex_active() under split requires vb_shade_split_tex_pipeline",
                    ),
                    targets.vb_set0_tex.as_ref().expect(
                        "invariant: GBufferScene::vb_tex_active() => targets.vb_set0_tex is built",
                    ),
                )
            } else {
                (
                    scene.vb_shade_split_pipeline.expect(
                        "invariant: a split-armed scene carries vb_shade_split_pipeline",
                    ),
                    vb_set0,
                )
            };
            #[cfg(not(feature = "hwrt"))]
            let (shade_pipeline, shade_set0) = if scene.vb_tex_active() {
                (
                    scene.vb_shade_split_tex_pipeline.expect(
                        "invariant: vb_tex_active() under split requires vb_shade_split_tex_pipeline",
                    ),
                    targets.vb_set0_tex.as_ref().expect(
                        "invariant: GBufferScene::vb_tex_active() => targets.vb_set0_tex is built",
                    ),
                )
            } else {
                (
                    scene.vb_shade_split_pipeline.expect(
                        "invariant: a split-armed scene carries vb_shade_split_pipeline",
                    ),
                    vb_set0,
                )
            };
            let vb_split_set1 = targets
                .vb_split_set1
                .as_ref()
                .expect("invariant: a split-armed boot builds vb_split_set1 (targets tail)");
            // SAFETY: recording is open; `shade_pipeline` (Set 0 = the base/_tex vb ring, Set 1
            // = `vb_split_set1[fi]` — the shadow vocab + gSsao + the DDGI combined atlases, Set
            // 2 = the geometry table, `_tex` adds Set 3 = the bindless table) belongs to this
            // device; the 64-byte push is the SAME `view_proj`; `dispatch_group_count_x`
            // covers the pixel count.
            unsafe {
                (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, shade_pipeline.pipeline);
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    shade_pipeline.layout,
                    0,
                    1,
                    &shade_set0[fi].descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    shade_pipeline.layout,
                    1,
                    1,
                    &vb_split_set1[fi].descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    shade_pipeline.layout,
                    2,
                    1,
                    &vb_geometry_set.set(),
                    0,
                    ptr::null(),
                );
                if scene.vb_tex_active() {
                    let bindless_set = scene
                        .bindless_set
                        .expect("invariant: GBufferScene::vb_tex_active() => scene.bindless_set is Some");
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        shade_pipeline.layout,
                        3,
                        1,
                        &bindless_set,
                        0,
                        ptr::null(),
                    );
                }
                (self.fns.cmd_push_constants)(
                    cmd,
                    shade_pipeline.layout,
                    VK_SHADER_STAGE_COMPUTE_BIT,
                    0,
                    64,
                    scene.mvp.as_ptr().cast(),
                );
                (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
            }
            // Close the SPLIT lit producer's VbShade bracket. GATED.
            if let Some(tc) = scene.vb_gpu_timing {
                // SAFETY: recording is open; the pool was reset this frame; `fi` is this slot.
                unsafe { tc.write_end(self.fns, cmd, fi, VbTimedPass::VbShade) };
            }
        }

        // === Pass `sdf_forward_march` — rung R10: the fused SDF march-then-shade COMPUTE pass,
        // the SAME body `record_forward` records (that fn's own doc has the full C5 rationale).
        // Recorded ONLY when `scene.path_has_sdf_forward()` holds. Extends the `lit` write above
        // (`vb_resolve`'s GENERAL store under `Both`, or `vb_sky`'s COLOR write under `Sdf`) and
        // marches THIS frame's `lit` pixels the raster/sky did not already paint (a miss writes
        // nothing to `lit` — the sky/mesh color stands, the pass's own doc; the TAA-armed VIEWT
        // variants additionally write the `gViewT` lane for EVERY pixel, exactly once). ===
        if scene.path_has_sdf_forward() {
            let sdf_forward_pass = plan
                .sdf_forward_march
                .expect("invariant: scene.path_has_sdf_forward() => sdf_forward_march pass declared");
            // SAFETY: recording is open; `record_vb_pass` records the graph's derived `lit`
            // ->GENERAL barrier (+ the `vb_depth` DEPTH_ATTACHMENT_OPTIMAL->SHADER_READ_ONLY_OPTIMAL
            // barrier under `mesh_leg`, or the cascade/atlas/light_table producer barriers under
            // `!mesh_leg`) for the "sdf_forward_march" pass into `cmd`.
            self.record_vb_pass(sdf_forward_pass, cmd, targets, forward, vb, scene, fi);

            // Decision 4's ownership gate: the `HAS_MESH` variants need the reverse-Z decode
            // `A`/`B` (`scene.sdf_forward_view_z_a`/`_b`) to bound the march at the mesh surface;
            // the mesh-less variants never read them (`SdfForwardMarchPush::sdf_only`'s doc).
            // TAA-under-VB: `writes_viewt` (the SAME O1 predicate `declare_vb_graph` read to
            // declare this pass's conditional `viewt` write) selects the `VIEWT` gViewT-producing
            // sibling — same layout, same push, plus the binding-13 store.
            let writes_viewt = scene.path_sdf_forward_writes_viewt();
            let (pipeline, push) = if scene.resolved_render_path.mesh_leg {
                let p = if writes_viewt {
                    scene.sdf_forward_march_viewt_pipeline.expect(
                        "invariant: path_sdf_forward_writes_viewt() requires scene.sdf_forward_march_viewt_pipeline",
                    )
                } else {
                    scene.sdf_forward_march_pipeline.expect(
                        "invariant: scene.path_has_sdf_forward() requires scene.sdf_forward_march_pipeline",
                    )
                };
                let push = crate::compute::SdfForwardMarchPush::has_mesh(
                    present_extent.width,
                    present_extent.height,
                    scene.sdf_forward_view_z_a,
                    scene.sdf_forward_view_z_b,
                    scene.light_dir,
                );
                (p, push)
            } else {
                let p = if writes_viewt {
                    scene.sdf_forward_march_sdfonly_viewt_pipeline.expect(
                        "invariant: path_sdf_forward_writes_viewt() requires scene.sdf_forward_march_sdfonly_viewt_pipeline",
                    )
                } else {
                    scene.sdf_forward_march_sdfonly_pipeline.expect(
                        "invariant: scene.path_has_sdf_forward() requires scene.sdf_forward_march_sdfonly_pipeline",
                    )
                };
                let push = crate::compute::SdfForwardMarchPush::sdf_only(
                    present_extent.width,
                    present_extent.height,
                    scene.light_dir,
                );
                (p, push)
            };
            let sdf_forward_set = targets
                .sdf_forward_set
                .as_ref()
                .expect("invariant: scene.path_has_sdf_forward() => targets.sdf_forward_set is built");
            let push_bytes = push.as_bytes();
            // SAFETY: recording is open; `pipeline` (one of the four `{HAS_MESH} x {VIEWT}`
            // compute variants, selected by `mesh_leg` x `writes_viewt`) + its 2-set layout
            // (Set 0 = `sdf_forward_set[fi]`, the SAME
            // path-independent vocab ring the Forward family binds — it references `forward.depth`,
            // which VB reuses as `vb_depth`; Set 1 = `forward.set1[fi]`, the Forward-family shadow
            // set REUSED VERBATIM, the same one `vb_resolve` binds) belong to this device (caller
            // contract); both descriptor sets are live, written once per extent; `push_bytes` is
            // exactly `SDF_FORWARD_MARCH_PUSH_BYTES` (40) at offset 0; `scene.dispatch_group_count_x`
            // covers `present_extent.width * present_extent.height` pixels at the shader's
            // `numthreads(64,1,1)` 1D grid (the SAME grid `vb_resolve`/the deferred marcher use).
            unsafe {
                (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_COMPUTE, pipeline.pipeline);
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    pipeline.layout,
                    0,
                    1,
                    &sdf_forward_set[fi].descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_bind_descriptor_sets)(
                    cmd,
                    VK_PIPELINE_BIND_POINT_COMPUTE,
                    pipeline.layout,
                    1,
                    1,
                    &forward.set1[fi].descriptor_set,
                    0,
                    ptr::null(),
                );
                (self.fns.cmd_push_constants)(
                    cmd,
                    pipeline.layout,
                    VK_SHADER_STAGE_COMPUTE_BIT,
                    0,
                    crate::compute::SDF_FORWARD_MARCH_PUSH_BYTES,
                    push_bytes.as_ptr().cast(),
                );
                (self.fns.cmd_dispatch)(cmd, scene.dispatch_group_count_x, 1, 1);
            }
        }

        // === TAA-under-VB: the `vb_viewt` gViewT-producer dispatch — AFTER every `lit`/depth
        // producer, BEFORE the TAA resolve below that consumes `viewt`. Mirrors
        // `record_gbuffer`'s own `viewt_from_depth` record site token-for-token, adapted to the
        // VB plan/sink + the 16-byte reverse-Z push. ===
        debug_assert_eq!(
            plan.viewt_from_depth.is_some(),
            scene.viewt_from_vb_depth.is_some(),
            "W1: declare/record predicate desync (viewt_from_vb_depth)"
        );
        // Rung R9b: this is the taa-only LATE slot — with the split's SSAO armed the pass was
        // recorded PRE-TAIL (inside the split arm above), matching `declare_vb_graph`'s slot
        // choice (ONE `scene.ssao.is_some()` predicate at both sites).
        if scene.ssao.is_none()
            && let Some(vb_viewt_pass) = plan.viewt_from_depth
        {
            self.record_vb_viewt_dispatch(vb_viewt_pass, cmd, targets, forward, vb, scene, present_extent, fi);
        }

        // === TAA-under-VB: the temporal-resolve (+ RCAS) — recorded HERE, BEFORE
        // `present_sample`'s `lit` GENERAL→SHADER_READ_ONLY transition (TAA is a COMPUTE
        // dispatch whose own graph pass derives that transition out of the `lit` producer — the
        // OPPOSITE ordering the FXAA/SMAA/SSAA fragment passes below use; `record_gbuffer`'s
        // TAA block ordering, ported). The pass barriers are emitted through the VB sink
        // (`record_vb_pass` — VB's OWN ResId table), then the graph-emit-free
        // `record_taa_body`/`record_rcas` bodies run unchanged (both are path-portable). ===
        if let Some(taa) = scene.taa.as_ref()
            && targets.taa_resolve_set.is_some()
        {
            let taa_pass = plan
                .taa_resolve
                .expect("invariant: scene.taa.is_some() ⇒ the VB taa_resolve pass was declared");
            self.record_vb_pass(taa_pass, cmd, targets, forward, vb, scene, fi);
            // SAFETY: recording is open; the taa_resolve pass barriers were just emitted via
            // the VB sink above (record_taa_body's caller contract); `aa_out`/`taa_hist`/
            // `taa_resolve_set` were built by `create()` under the same `scene.taa` that gates
            // this branch.
            unsafe { self.record_taa_body(cmd, targets, taa, scene, fi) };
            if let Some(rcas) = scene.rcas.as_ref()
                && targets.rcas_set.is_some()
            {
                // SAFETY: recording is open; `record_taa_body` (just above) already wrote
                // `taa_resolved[fi]`, leaving it in GENERAL; `taa_resolved`/`aa_out`/`rcas_set`
                // were built by `create()` under the same `scene.rcas` that gates this branch;
                // `present_extent` sizes both (the SAME extent the resolve dispatched over).
                unsafe { self.record_rcas(cmd, targets, rcas, present_extent, scene, fi) };
            }
        }

        // === Present-blit `lit` into the swapchain — byte-for-byte port of `record_forward`'s
        // own tail. ===
        // SAFETY: recording is open; `record_vb_pass` records the graph's derived
        // GENERAL→SHADER_READ_ONLY_OPTIMAL barrier for the "present_sample" pass into `cmd`
        // (with TAA armed, the taa_resolve pass above already left `lit` in SHADER_READ_ONLY —
        // this then derives no further barrier, the deferred precedent).
        self.record_vb_pass(plan.present_sample, cmd, targets, forward, vb, scene, fi);

        // Anti-aliasing resolve (VB). When AA is armed, `sync_gbuffer` rewires `present_set` to
        // sample `aa_out` (path-agnostic), but the FXAA/SMAA/SSAA dispatch used to live ONLY in
        // `record_gbuffer` (Deferred) -- so under VB the resolved `lit` was never anti-aliased into
        // `aa_out`, and the present-blit below sampled a never-written (black) image. Mirror
        // `record_gbuffer`'s AA block verbatim: FXAA/SMAA read `present_extent`; SSAA reads the
        // BOOT-FIXED `aa_extent` (`present_extent` is 2x under SSAA). TAA (VB × Mesh) was ALREADY
        // recorded above (before `present_sample` -- the compute-vs-fragment ordering, see that
        // block's comment), so the fall-through documents it exactly like `record_gbuffer`'s
        // equivalent else. OFF (`aa_out` is `None`) records nothing -> the AA-off VB command
        // stream is byte-identical to before this block existed.
        if targets.aa_out.is_some() {
            if let Some(fxaa) = scene.aa.as_ref() {
                // SAFETY: recording is open; `present_sample` above left `lit` in
                // SHADER_READ_ONLY_OPTIMAL; `aa_out`/`fxaa_set` were built by `create()` under the
                // same `scene.aa` that gates this branch; `present_extent` sizes `aa_out`.
                unsafe { self.record_fxaa(cmd, targets, fxaa, present_extent, fi) };
            } else if let Some(smaa) = scene.smaa.as_ref() {
                // SAFETY: recording is open; `present_sample` above left `lit` in
                // SHADER_READ_ONLY_OPTIMAL; `aa_out`/the SMAA edge/weight targets + the three
                // `smaa_*_set` rings were built by `create()` under the same `scene.smaa` that
                // gates this branch; `present_extent` sizes every SMAA target.
                unsafe { self.record_smaa(cmd, targets, smaa, present_extent, fi) };
            } else if let Some(ssaa) = scene.ssaa.as_ref() {
                debug_assert!(targets.aa_out.is_some() && targets.downsample_set.is_some());
                // SAFETY: recording is open; `present_sample` above left `lit` (the 2x ring) in
                // SHADER_READ_ONLY_OPTIMAL; `aa_out`/`downsample_set` were built by `create()`
                // under the same `scene.ssaa` that gates this branch, sized to the BOOT-FIXED
                // `aa_extent` (NOT `present_extent`, which is 2x under SSAA).
                unsafe { self.record_ssaa(cmd, targets, ssaa, aa_extent, fi) };
            } else {
                // `aa_out.is_some()` with none of aa/smaa/ssaa matched ⇒ TAA is the reason
                // (the four arms are mutually exclusive by construction); `record_taa_body`
                // already ran above (the TAA-under-VB block) — nothing left to do here, the
                // SAME fall-through `record_gbuffer`'s equivalent else documents.
                debug_assert!(
                    scene.taa.is_some(),
                    "invariant: VB aa_out armed but none of aa/smaa/ssaa/taa matched"
                );
            }
        }

        let to_color = VkImageMemoryBarrier {
            s_type: VkStructureType::ImageMemoryBarrier,
            p_next: ptr::null(),
            src_access_mask: 0,
            dst_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
            old_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            new_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
            image,
            subresource_range: COLOR_SUBRESOURCE_RANGE,
        };
        // SAFETY: recording is open; one image barrier on the live swapchain `image`;
        // TOP_OF_PIPE→COLOR_ATTACHMENT_OUTPUT with UNDEFINED→COLOR is the superset-correct
        // acquire→render transition; `&to_color` outlives the call.
        unsafe {
            (self.fns.cmd_pipeline_barrier)(
                cmd,
                VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT,
                VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                0,
                0,
                ptr::null(),
                0,
                ptr::null(),
                1,
                (&to_color as *const VkImageMemoryBarrier).cast(),
            );
        }

        let color_attachment = VkRenderingAttachmentInfo {
            s_type: VkStructureType::RenderingAttachmentInfo,
            p_next: ptr::null(),
            image_view: view,
            image_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
            resolve_mode: 0,
            resolve_image_view: VkImageView::NULL,
            resolve_image_layout: VK_IMAGE_LAYOUT_UNDEFINED,
            load_op: VK_ATTACHMENT_LOAD_OP_CLEAR,
            store_op: VK_ATTACHMENT_STORE_OP_STORE,
            clear_value: VkClearValue { color: VkClearColorValue { float32: clear } },
        };
        let present_rendering = VkRenderingInfo {
            s_type: VkStructureType::RenderingInfo,
            p_next: ptr::null(),
            flags: 0,
            render_area: VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent },
            layer_count: 1,
            view_mask: 0,
            color_attachment_count: 1,
            p_color_attachments: &color_attachment,
            p_depth_attachment: ptr::null(),
            p_stencil_attachment: ptr::null(),
        };
        let blit_extent = VkExtent2D {
            width: extent.width.min(present_extent.width),
            height: extent.height.min(present_extent.height),
        };
        let blit_viewport = VkViewport {
            x: 0.0,
            y: 0.0,
            width: blit_extent.width as f32,
            height: blit_extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let blit_scissor = VkRect2D { offset: VkOffset2D { x: 0, y: 0 }, extent: blit_extent };
        // SAFETY: recording is open; `present_rendering` names the live swapchain `view` (now
        // COLOR_ATTACHMENT_OPTIMAL); dynamic rendering is enabled. `scene.present_pipeline` +
        // its bind-group layout belong to this device and its declared color format equals the
        // swapchain's; `targets.present_set[fi]` binds `lit[fi]` (now SHADER_READ_ONLY_OPTIMAL —
        // the SAME slot `vb_resolve` just wrote), OR `aa_out[fi]` when AA is armed (the AA pass
        // above left it SHADER_READ_ONLY_OPTIMAL) + sampler at set 0; `blit_viewport`/
        // `blit_scissor` outlive the bracketed calls; `draw(3, 1, 0, 0)` is the `SV_VertexID`
        // fullscreen triangle. Begin/End bracket the pass exactly.
        unsafe {
            (self.fns.cmd_begin_rendering)(cmd, &present_rendering);
            (self.fns.cmd_bind_pipeline)(cmd, VK_PIPELINE_BIND_POINT_GRAPHICS, scene.present_pipeline.pipeline);
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_GRAPHICS,
                scene.present_pipeline.layout,
                0,
                1,
                &targets.present_set[fi].descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_set_viewport)(cmd, 0, 1, &blit_viewport);
            (self.fns.cmd_set_scissor)(cmd, 0, 1, &blit_scissor);
            (self.fns.cmd_draw)(cmd, 3, 1, 0, 0);
            (self.fns.cmd_end_rendering)(cmd);
        }

        // === VG-R0 rung R0c: the ARMED density-census copy — `vb_id` → host-visible staging. ===
        //
        // AND-gated on `mesh_leg` as well as on the `Option`, and the second conjunct is load-
        // bearing rather than defensive: a `VisibilityBuffer × Sdf` frame skips `vb_raster`
        // entirely (the gate ~1500 lines above), so this ring slot was never transitioned out of
        // UNDEFINED this frame and holds no raster. Copying it would read undefined memory through
        // a `srcLayout` it is not in. Recording nothing instead leaves the staging at the sentinel
        // prefill its owner wrote, which reduces to `covered_pixels == 0` and reds R0c(c′) — an
        // instrument failure that NAMES ITSELF, rather than a plausible fabricated row.
        //
        // Sited AFTER every `vb_id` reader and BEFORE the swapchain's own present/readback
        // transition, so it needs no restore: the next frame in this ring slot re-enters through
        // `vb_raster`'s UNDEFINED→COLOR_ATTACHMENT_OPTIMAL barrier, which discards contents by
        // definition. An UNARMED frame records ZERO commands here (R0c gate (a)'s byte-neutrality).
        if let Some(census) = vb_id_readback
            && scene.resolved_render_path.mesh_leg
        {
            let vb_id_to_transfer = VkImageMemoryBarrier {
                s_type: VkStructureType::ImageMemoryBarrier,
                p_next: ptr::null(),
                src_access_mask: VK_ACCESS_SHADER_READ_BIT,
                dst_access_mask: VK_ACCESS_TRANSFER_READ_BIT,
                old_layout: VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL,
                new_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                image: vb.vb_id[fi].image,
                subresource_range: COLOR_SUBRESOURCE_RANGE,
            };
            // SAFETY: recording is open; `mesh_leg` guarantees `vb_raster` ran and the graph's
            // own first-reader barrier left this slot SHADER_READ_ONLY_OPTIMAL, so `old_layout`
            // is the layout the image is actually in. The source stage mask is the union of the
            // two shader stages that read `vb_id` (`vb_resolve`/`vb_classify`/`vb_geo` are
            // compute; the split's thin-aux reader is fragment), so every prior read is ordered
            // before the copy. `&vb_id_to_transfer` outlives the call.
            unsafe {
                (self.fns.cmd_pipeline_barrier)(
                    cmd,
                    VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT | VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
                    VK_PIPELINE_STAGE_TRANSFER_BIT,
                    0,
                    0,
                    ptr::null(),
                    0,
                    ptr::null(),
                    1,
                    (&vb_id_to_transfer as *const VkImageMemoryBarrier).cast(),
                );
            }

            // `present_extent`, NOT `extent`: the ring is sized to the composite (`VbTargets::build`
            // takes `GBufferTargets::create`'s `extent`, which IS the composite), and under armed
            // SSAA the composite is 2× native — the route §9.1's grant table takes to the top two
            // ladder rungs.
            let census_region = VkBufferImageCopy {
                buffer_offset: 0,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: VkImageSubresourceLayers {
                    aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                },
                image_offset: VkOffset3D { x: 0, y: 0, z: 0 },
                image_extent: VkExtent3D {
                    width: present_extent.width,
                    height: present_extent.height,
                    depth: 1,
                },
            };
            // SAFETY: recording is open; `vb_id[fi]` is TRANSFER_SRC_OPTIMAL per the barrier
            // above; one full-image tightly-packed color region copies into the live host-visible
            // `census.buffer`, which is ≥ `present_extent.width * present_extent.height * 8` bytes
            // per this fn's contract; `&census_region` outlives the call.
            unsafe {
                (self.fns.cmd_copy_image_to_buffer)(
                    cmd,
                    vb.vb_id[fi].image,
                    VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    census.buffer,
                    1,
                    &census_region,
                );
            }
        }

        // === VG R3 piece 1 step P1-6 (plan §5, gate G8): the ARMED pyramid dump. ===
        //
        // Copies the ENGINE'S OWN depth (mip 0, DEPTH aspect — the source the pyramid reduced)
        // and EVERY mip of the ENGINE'S OWN pyramid into one host-visible buffer, laid out so
        // P1-8's host half can address level `k` by a flat offset. G3 builds its own everything
        // and therefore cannot see a wrong source, a wrong extent, a stale descriptor, a missing
        // barrier or a pass that never ran; this is the readback that can.
        //
        // ⚠️ THE GATE IS THE DECLARATOR'S, VERBATIM (`scene.hzb_dump` + `scene.hzb` + `mesh_leg`),
        // and `targets.hzb`/`plan.hzb_dump` are `.expect()`ed under it rather than made further
        // conjuncts — the same single-predicate discipline the build block above follows. Spelling
        // it identically is what keeps `plan.hzb_dump` an `.expect()` instead of a silent skip: a
        // gate that could be false here while the declarator's was true would turn a declared pass
        // into an unrecorded one, and the only reason that is survivable at all is that this pass
        // is declared LAST.
        //
        // Sited HERE — after every pyramid writer and after the present blit, beside the census
        // copy, before the swapchain's own present/readback transition. The declaration is
        // correspondingly LAST in `declare_vb_graph`, which is the order parity `record_vb_pass`
        // depends on. An UNARMED frame records ZERO commands in this block.
        if let Some(staging) = scene.hzb_dump
            && scene.hzb.is_some()
            && scene.resolved_render_path.mesh_leg
        {
            let hzb = targets
                .hzb
                .as_ref()
                .expect("invariant: scene.hzb armed => targets.hzb (sync_gbuffer's hzb_arm predicate)");

            // THE plan is the bundle's OWN field — the same number the dispatch arithmetic and
            // the descriptor sets were sized from (`HzbTargets::plan`'s rule), so the copy regions
            // cannot name a level the image does not have.
            //
            // The SOURCE extent is `present_extent`, the SAME value the build pushed as
            // `src_extent`: the depth ring is sized to the composite, which under armed SSAA is 2×
            // native. Passing the client extent would copy a quarter of the image the pyramid
            // reduced and the gate would compare against the wrong depth.
            let layout = HzbDumpLayout::new(
                hzb.plan,
                [present_extent.width, present_extent.height],
            );
            let levels = hzb.plan.levels;

            // ⚠️ A RELEASE-LIVE bound, not a `debug_assert!`. The host sized this staging from
            // `GBufferScene::hzb`, while every offset below comes from `HzbTargets::plan`; the two
            // agree on every real boot (the build block above debug-asserts it, and both derive
            // from the same boot-fixed composite extent), but "agree today" is not a bound, and
            // the failure mode of being wrong is a transfer writing past a host-visible allocation
            // — undefined, and invisible to the validation layers, which do not follow buffer
            // contents.
            //
            // A short staging therefore records NOTHING — not even the pass's derived barriers,
            // which is sound precisely because the dump is the LAST declared pass: nothing later
            // in this frame consumes the layout it would have produced, and the ring re-enters
            // next frame through `vb_raster`'s own `UNDEFINED` first touch. And the silence is not
            // silent: the host driver prefills the whole staging with `0xFF` — `f32::NAN` — which
            // neither payload can legitimately contain (a reverse-Z attachment is clamped and
            // cannot hold NaN, and `hzb_build`'s reduce collapses NaN to `-INFINITY`). G8 reds on
            // "no texel is NaN", by name, instead of comparing a truncated pyramid against a full
            // one.
            debug_assert!(
                staging.size >= layout.total_bytes(),
                "invariant: the HZB dump staging is sized by HzbDumpLayout::total_bytes"
            );
            if staging.size >= layout.total_bytes() {
                let dump_pass = plan
                    .hzb_dump
                    .expect("invariant: the recorder's dump gate is the declarator's, verbatim");
                // SAFETY: recording is open; `record_vb_pass` records this pass's derived barriers
                // into `cmd` — the `vb_depth` SHADER_READ_ONLY_OPTIMAL → TRANSFER_SRC_OPTIMAL
                // transition out of its last reader, and the `COMPUTE(SHADER_WRITE) → TRANSFER`
                // flush of the build's stores over mips `[0, levels)`. Without it the copies could
                // read the pyramid before the last dispatch's stores landed, and it would usually
                // look right.
                self.record_vb_pass(dump_pass, cmd, targets, forward, vb, scene, fi);

                // The header, written by the RECORDER rather than by the host: these are the
                // numbers the copies below actually used, and G8 exists to catch a WRONG extent —
                // a host that re-derived the extent it expected and decoded with that could not
                // see one.
                let header = layout.header_words();
                // SAFETY: recording is open and no render-pass instance is active (the present
                // blit's `cmd_end_rendering` is above). `staging.buffer` is the live host-visible
                // TRANSFER_DST dump staging, `>= layout.total_bytes()` per the branch condition,
                // and `HZB_DUMP_HEADER_BYTES` is that layout's leading region, so the write is in
                // bounds. It is a multiple of 4 and 152 bytes, inside the 65536-byte inline limit.
                // `header` is a local that outlives the call.
                unsafe {
                    (self.fns.cmd_update_buffer)(
                        cmd,
                        staging.buffer,
                        0,
                        HZB_DUMP_HEADER_BYTES,
                        header.as_ptr().cast(),
                    );
                }

                let depth_region = VkBufferImageCopy {
                    buffer_offset: layout.depth_offset(),
                    buffer_row_length: 0,
                    buffer_image_height: 0,
                    image_subresource: VkImageSubresourceLayers {
                        aspect_mask: VK_IMAGE_ASPECT_DEPTH_BIT,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    },
                    image_offset: VkOffset3D { x: 0, y: 0, z: 0 },
                    image_extent: VkExtent3D {
                        width: present_extent.width,
                        height: present_extent.height,
                        depth: 1,
                    },
                };
                // SAFETY: recording is open; `forward.depth[fi]` is TRANSFER_SRC_OPTIMAL per the
                // pass's derived barrier above and was created with `TRANSFER_SRC` usage
                // (`ForwardTargets::build`'s `depth_desc`, plan §6); one full-image tightly-packed
                // DEPTH region copies into `[depth_offset, depth_offset + depth_bytes)` of the
                // staging, which `total_bytes` covers and the branch condition bounds;
                // `&depth_region` outlives the call.
                unsafe {
                    (self.fns.cmd_copy_image_to_buffer)(
                        cmd,
                        forward.depth[fi].image,
                        VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                        staging.buffer,
                        1,
                        &depth_region,
                    );
                }

                // ONE region per mip, all in one call. The buffer offsets are
                // `HzbDumpLayout::level_byte_offset`'s — levels back to back, finest first, each
                // row-major — which is `boyko_render::hzb::HzbLayout::level_offset`'s own layout,
                // so P1-8 can compare `build_pyramid`'s flat output against this region word for
                // word.
                let regions: [VkBufferImageCopy; MAX_HZB_LEVELS] = core::array::from_fn(|k| {
                    // Slots at `k >= levels` are PADDING — the copy count below is `levels`, so
                    // Vulkan never reads them. They repeat the last live level rather than
                    // carrying a zero extent, which no VUID admits even in an unread entry a
                    // validation layer might one day walk.
                    let level = (k as u32).min(levels - 1);
                    let [w, h] = hzb.plan.extent_of(level);
                    VkBufferImageCopy {
                        buffer_offset: layout.level_byte_offset(level),
                        buffer_row_length: 0,
                        buffer_image_height: 0,
                        image_subresource: VkImageSubresourceLayers {
                            aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
                            mip_level: level,
                            base_array_layer: 0,
                            layer_count: 1,
                        },
                        image_offset: VkOffset3D { x: 0, y: 0, z: 0 },
                        image_extent: VkExtent3D { width: w, height: h, depth: 1 },
                    }
                });
                // SAFETY: recording is open; the pyramid is `GENERAL` for life and
                // `vkCmdCopyImageToBuffer` accepts that layout, which is why the pass's derived
                // edge needed no transition; the image was created with `TRANSFER_SRC` and
                // `levels` mips (`HzbTargets::build`), so every `mip_level` named is a real mip
                // and every extent is that mip's own; the regions tile
                // `[pyramid_offset, pyramid_offset + pyramid_bytes)` without overlap, inside the
                // staging per the branch condition. `levels <= MAX_HZB_LEVELS` (the plan's own
                // invariant) bounds the region count to the array's length. `regions` is a local
                // that outlives the call.
                unsafe {
                    (self.fns.cmd_copy_image_to_buffer)(
                        cmd,
                        hzb.pyramid.image,
                        VK_IMAGE_LAYOUT_GENERAL,
                        staging.buffer,
                        levels,
                        regions.as_ptr(),
                    );
                }
            }
        }

        match readback {
            None => {
                let to_present = VkImageMemoryBarrier {
                    s_type: VkStructureType::ImageMemoryBarrier,
                    p_next: ptr::null(),
                    src_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    dst_access_mask: 0,
                    old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    new_layout: VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
                    src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: COLOR_SUBRESOURCE_RANGE,
                };
                // SAFETY: recording is open; COLOR_ATTACHMENT_OUTPUT→BOTTOM_OF_PIPE with
                // COLOR→PRESENT makes the blit's writes visible to the present engine;
                // `&to_present` outlives the call.
                unsafe {
                    (self.fns.cmd_pipeline_barrier)(
                        cmd,
                        VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                        VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        (&to_present as *const VkImageMemoryBarrier).cast(),
                    );
                }
            }
            Some(staging) => {
                let to_transfer = VkImageMemoryBarrier {
                    s_type: VkStructureType::ImageMemoryBarrier,
                    p_next: ptr::null(),
                    src_access_mask: VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT,
                    dst_access_mask: VK_ACCESS_TRANSFER_READ_BIT,
                    old_layout: VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL,
                    new_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: COLOR_SUBRESOURCE_RANGE,
                };
                // SAFETY: recording is open; COLOR_ATTACHMENT_OUTPUT→TRANSFER with
                // COLOR→TRANSFER_SRC makes the blit's writes available to the copy;
                // `&to_transfer` outlives the call.
                unsafe {
                    (self.fns.cmd_pipeline_barrier)(
                        cmd,
                        VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT,
                        VK_PIPELINE_STAGE_TRANSFER_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        (&to_transfer as *const VkImageMemoryBarrier).cast(),
                    );
                }

                let region = VkBufferImageCopy {
                    buffer_offset: 0,
                    buffer_row_length: 0,
                    buffer_image_height: 0,
                    image_subresource: VkImageSubresourceLayers {
                        aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    },
                    image_offset: VkOffset3D { x: 0, y: 0, z: 0 },
                    image_extent: VkExtent3D { width: extent.width, height: extent.height, depth: 1 },
                };
                // SAFETY: recording is open; the swapchain image is TRANSFER_SRC_OPTIMAL per the
                // barrier above; one full-image tightly-packed color region copies into the live
                // host-visible `staging.buffer` (≥ the image's byte size per this fn's contract);
                // `&region` outlives the call.
                unsafe {
                    (self.fns.cmd_copy_image_to_buffer)(
                        cmd,
                        image,
                        VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                        staging.buffer,
                        1,
                        &region,
                    );
                }

                let to_present = VkImageMemoryBarrier {
                    s_type: VkStructureType::ImageMemoryBarrier,
                    p_next: ptr::null(),
                    src_access_mask: VK_ACCESS_TRANSFER_READ_BIT,
                    dst_access_mask: 0,
                    old_layout: VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
                    new_layout: VK_IMAGE_LAYOUT_PRESENT_SRC_KHR,
                    src_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    dst_queue_family_index: VK_QUEUE_FAMILY_IGNORED,
                    image,
                    subresource_range: COLOR_SUBRESOURCE_RANGE,
                };
                // SAFETY: recording is open; TRANSFER→BOTTOM_OF_PIPE with TRANSFER_SRC→PRESENT
                // releases the image to the present engine after the readback copy; `&to_present`
                // outlives the call.
                unsafe {
                    (self.fns.cmd_pipeline_barrier)(
                        cmd,
                        VK_PIPELINE_STAGE_TRANSFER_BIT,
                        VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT,
                        0,
                        0,
                        ptr::null(),
                        0,
                        ptr::null(),
                        1,
                        (&to_present as *const VkImageMemoryBarrier).cast(),
                    );
                }
            }
        }

        // SAFETY: recording is open; ending it matches the `begin` above.
        let raw = unsafe { (self.fns.end_command_buffer)(cmd) };
        let result = VkResult::from_raw(raw);
        if !result.is_success() {
            return Err(SwapchainError::VkError("vkEndCommandBuffer", result));
        }
        Ok(())
    }

    /// VG R3 piece 2 step P2-5 (docs/VG-R3-P2-CAPABILITY-SPLIT-PLAN.md, decision D6): the
    /// `[hzb_poison, hzb_build_0 .. hzb_build_{n-1}]` BLOCK's pass-barriers + commands, extracted
    /// so BOTH record slots share one implementation — today's (after the `lit` producer) on an
    /// unsplit frame, and between the two raster scopes on an armed-split one.
    /// `declare_vb_graph` declares the block in exactly one position per frame
    /// ([`GBufferScene::path_vb_occlusion_split`] picks it), and the recorder replays the SAME
    /// body at the matching site — the [`Self::record_vb_viewt_dispatch`] idiom directly below.
    ///
    /// # Why the poison and the builds are ONE function
    ///
    /// `hzb_poison` is asserted to be declared before every `hzb_build_*`, and its clear must
    /// therefore also be RECORDED before every dispatch — otherwise it erases exactly the levels
    /// they just wrote, the dump reads [`HZB_PYRAMID_POISON`] everywhere, and gate G8 reds
    /// claiming "the build never ran". In a dev-profile build (which is what the golden runs use)
    /// the declarator's `debug_assert!` fires first; in a release binary it is compiled out and
    /// only the wrong-looking gate remains. One function is what makes "the block moves whole" a
    /// property of the code rather than of a reviewer.
    ///
    /// # The gates are the DECLARATOR'S, verbatim
    ///
    /// Both conditions below are spelled exactly as `declare_hzb_poison_build`'s inputs are
    /// derived, and `targets.hzb` is `.expect()`ed rather than made an extra conjunct (the
    /// `batch_cull_armed` discipline this file already follows): `sync_gbuffer`'s rebuild
    /// predicate carries `hzb_arm == scene.hzb.is_some()`, so an armed scene ALWAYS has the
    /// bundle and a create failure returns `Err` rather than a silent `None`. A recorder gate
    /// narrower than the declarator's would leave declared writes on the pyramid that no command
    /// performs — and, for the poison specifically, would leave its declared barrier unemitted
    /// while `hzb_build_0`'s own barrier still flushes a `TRANSFER_WRITE` that never happened.
    /// ⚠️ Before VG R3 piece 3 step P3-0 the poison ALSO carried the pyramid's only
    /// `UNDEFINED → GENERAL` transition; `HzbTargets::boot_clear_hzb_pyramid` owns that now (the
    /// image is `GENERAL` from birth), so a skipped poison no longer hands `hzb_build_0` a storage
    /// image in the wrong LAYOUT — it hands it an UNPOISONED one, which is the defect gate G8
    /// exists to refuse.
    ///
    /// # Recording contract
    ///
    /// Recording must be open on `cmd` and NO render-pass instance may be active: both
    /// `vkCmdClearColorImage` and `vkCmdDispatch` are forbidden inside one. Both call sites
    /// record after a `cmd_end_rendering`, and the `unsafe` blocks below cite this.
    #[allow(clippy::too_many_arguments)]
    fn record_hzb_poison_build(
        &self,
        plan: &super::super::graph_bridge::VbPassPlan,
        cmd: VkCommandBuffer,
        targets: &GBufferTargets,
        forward: &ForwardTargets,
        vb: &VbTargets,
        scene: &GBufferScene<'_>,
        present_extent: VkExtent2D,
        fi: usize,
    ) {
        // === VG R3 piece 1 step P1-8 (plan §5/§13, gate G8): the pyramid POISON clear. ===
        //
        // Recorded FIRST — immediately before the first `hzb_build` dispatch below, in EXACTLY
        // `declare_vb_graph`'s position (the declarator asserts the ordering both ways). Every mip
        // is filled with a value the reduce cannot produce, so a level the build fails to write
        // reads `HZB_PYRAMID_POISON` in the dump and G8 reds by name instead of agreeing with the
        // oracle over a field of far-plane zeros — the vacuity step P1-6 measured (89.3% of the
        // `vb_mesh` pyramid is `0.0`; levels 6..9, the second build pass's whole output, entirely
        // so).
        if scene.hzb_dump.is_some() && scene.hzb.is_some() && scene.resolved_render_path.mesh_leg {
            let hzb = targets
                .hzb
                .as_ref()
                .expect("invariant: scene.hzb armed => targets.hzb (sync_gbuffer's hzb_arm predicate)");
            let poison_pass = plan
                .hzb_poison
                .expect("invariant: the recorder's poison gate is the declarator's, verbatim");
            // SAFETY: recording is open; `record_vb_pass` records this pass's derived barrier into
            // `cmd` — the `UNDEFINED → GENERAL` first touch of mips `[0, levels)`, which is what
            // makes the clear below legal on an image the frame has not otherwise touched yet.
            self.record_vb_pass(poison_pass, cmd, targets, forward, vb, scene, fi);

            // `R32_SFLOAT` reads only `float32[0]`; the other three components are ignored for a
            // single-component format and are spelled as the same value rather than left as
            // arbitrary padding.
            let poison = VkClearColorValue { float32: [HZB_PYRAMID_POISON; 4] };
            // ⚠️ `hzb.plan.levels`, NEVER `MAX_HZB_LEVELS`: the capacity is 17 and the image has
            // `levels` mips (`HzbTargets::build` creates it from this same number), so a
            // capacity-wide range would name mips that do not exist at every real render extent.
            let range = VkImageSubresourceRange {
                aspect_mask: VK_IMAGE_ASPECT_COLOR_BIT,
                base_mip_level: 0,
                level_count: hzb.plan.levels,
                base_array_layer: 0,
                layer_count: 1,
            };
            // SAFETY: recording is open and no render-pass instance is active (this fn's own
            // contract — both call sites record after a `cmd_end_rendering`). `hzb.pyramid.image`
            // is the live pyramid, created with `TRANSFER_DST` usage (`HzbTargets::build`'s
            // `pyramid_desc`, which `VUID-vkCmdClearColorImage-image-00002` requires) and a COLOR
            // `R32_SFLOAT` format with no depth/stencil aspect. It is in `GENERAL` — one of the two
            // layouts this command accepts — by the pass's derived first-touch barrier just
            // recorded. The range names mips `[0, levels)` and layer 0 of a `levels`-mip,
            // single-layer image, so it is in bounds. `&poison` and `&range` are fully-initialized
            // locals alive for the call.
            unsafe {
                (self.fns.cmd_clear_color_image)(
                    cmd,
                    hzb.pyramid.image,
                    VK_IMAGE_LAYOUT_GENERAL,
                    &poison,
                    1,
                    &range,
                );
            }
        }

        // === VG R3 piece 1 step P1-5: the HZB depth-pyramid BUILD dispatches. ===
        //
        // Recorded after the mesh raster has written `vb_depth`, in EXACTLY `declare_vb_graph`'s
        // position for the same chain. Declare/record ORDER parity is the invariant this file and
        // the declarator both treat as load-bearing.
        //
        // ⚠️ Under `HzbMode::Off` this records NOTHING — no barrier, no bind, no dispatch — which
        // is what keeps every golden pin byte-identical. The ARMED pins stay byte-identical too,
        // for a different reason: the pyramid is an image nothing samples in pieces 1 and 2 (the
        // cull is piece 3), so the dispatches move no pixel — at EITHER slot.
        if scene.hzb.is_some() && scene.resolved_render_path.mesh_leg {
            let hzb = targets
                .hzb
                .as_ref()
                .expect("invariant: scene.hzb armed => targets.hzb (sync_gbuffer's hzb_arm predicate)");
            // THE plan is the bundle's OWN field, not `scene.hzb`: the descriptor sets, the image's
            // real mip count and this dispatch arithmetic must be sized from ONE number, or a lane
            // can be dispatched over a level whose view was never built (`HzbTargets::plan`'s own
            // doc states the rule; `build` follows it for the sets).
            let hzb_plan = hzb.plan;
            debug_assert_eq!(
                Some(hzb_plan),
                scene.hzb,
                "invariant: the pyramid bundle's plan matches the scene's — both are derived from \
                 present_extent, and `sync_gbuffer` rebuilds the bundle when that changes"
            );
            let pipeline = scene
                .hzb_build_pipeline
                .expect("invariant: scene.hzb armed => scene.hzb_build_pipeline (GpuSceneBundles::boot mints it unconditionally)");

            let levels = hzb_plan.levels;
            let pass_count = levels.div_ceil(HZB_LEVELS_PER_PASS) as usize;
            // The SOURCE extent, `S` — the extent the depth ring was sized to, which is what the
            // pyramid reduces. NOT `hzb_plan.extent_of(0)`: that is `P = prev_pow2(S)` per axis,
            // and the shader's base map `⌈t·S/P⌉` reads BOTH (see `HzbBuildPush::src_extent`).
            let src_extent = [present_extent.width, present_extent.height];

            for p in 0..pass_count {
                let d = p as u32 * HZB_LEVELS_PER_PASS;
                let n = (levels - d).min(HZB_LEVELS_PER_PASS);

                let hzb_pass = plan.hzb_build[p]
                    .expect("invariant: the recorder's pass count equals the declarator's (one plan)");
                // SAFETY: recording is open; `record_vb_pass` records this pass's derived
                // barriers into `cmd` — the raster's DEPTH_ATTACHMENT→SHADER_READ_ONLY transition
                // on `vb_depth` (pass 0), the previous pass's write→read flush on mip `d-1`
                // (passes 1+), and the UNDEFINED→GENERAL first touch of mips `[d, d+n)`.
                self.record_vb_pass(hzb_pass, cmd, targets, forward, vb, scene, fi);

                let set = hzb.sets[fi][p]
                    .as_ref()
                    .expect("invariant: sets[slot][p] is Some for every p < levels.div_ceil(HZB_LEVELS_PER_PASS)");

                // `k >= n` pads with level `d` — a real level of a real mip, matching the view
                // `HzbTargets::build` binds at that destination slot. NEVER zero: the shader
                // divides by these extents before it tests `k < level_count`, and a padded slot
                // must therefore be well-defined rather than merely unwritten.
                let out = |k: u32| hzb_plan.extent_of(if k < n { d + k } else { d });
                let push = HzbBuildPush {
                    src_extent,
                    // `E(d-1)` on a reduce pass; on the BASE pass mip `d-1` does not exist and the
                    // shader's `base_level == 0` arm never reads it, so level 0's own extent goes
                    // in as a well-defined placeholder.
                    fine_extent: hzb_plan.extent_of(d.saturating_sub(1)),
                    out_extent0: out(0),
                    out_extent1: out(1),
                    out_extent2: out(2),
                    out_extent3: out(3),
                    out_extent4: out(4),
                    out_extent5: out(5),
                    base_level: d,
                    level_count: n,
                }
                .to_bytes();

                // THE DISPATCH DIVISOR is this pass's FIRST OUTPUT LEVEL — the one index space in
                // which the base and reduce variants have the same shape (plan §2). One workgroup
                // covers a `HZB_BUILD_TILE`-texel tile of level `d`.
                let [ex, ey] = hzb_plan.extent_of(d);

                // SAFETY: recording is open and outside any render scope (this fn's own contract);
                // `pipeline` + its layout (one COMPUTE set + a `HZB_BUILD_PUSH_BYTES` = 72-byte
                // COMPUTE push range) are live on this device — `GpuSceneBundles::boot` mints both
                // unconditionally and owns them for the device's lifetime. `set` binds all 8
                // entries `hzb_build_layout` declares (SAMPLED `gSrcDepth` @0 — the `[fi]` slot's
                // depth, which is why the sets are ringed — plus the seven single-mip storage views
                // @1..@7), written once when these targets were built (`HzbTargets::build`, which
                // binds a REAL view at every slot including the padded ones) and untouched since.
                // The push writes exactly the declared range at offset 0 from a
                // `[u8; HZB_BUILD_PUSH_BYTES]` local. The dispatch covers `ceil(E(d)/TILE)` groups
                // per axis; lanes past the level extent issue no tap and store nothing (the
                // shader's own boundary rule). `&set.descriptor_set` and `push` are locals alive
                // for the calls.
                unsafe {
                    (self.fns.cmd_bind_pipeline)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        pipeline.pipeline,
                    );
                    (self.fns.cmd_bind_descriptor_sets)(
                        cmd,
                        VK_PIPELINE_BIND_POINT_COMPUTE,
                        pipeline.layout,
                        0,
                        1,
                        &set.descriptor_set,
                        0,
                        ptr::null(),
                    );
                    (self.fns.cmd_push_constants)(
                        cmd,
                        pipeline.layout,
                        VK_SHADER_STAGE_COMPUTE_BIT,
                        0,
                        HZB_BUILD_PUSH_BYTES,
                        push.as_ptr().cast(),
                    );
                    (self.fns.cmd_dispatch)(
                        cmd,
                        ex.div_ceil(HZB_BUILD_TILE),
                        ey.div_ceil(HZB_BUILD_TILE),
                        1,
                    );
                }
            }
        }
    }

    /// Rung R9b: the `vb_viewt` (viewt_from_depth_rz) pass-barriers + dispatch body, extracted
    /// so BOTH record slots (the split's SSAO-armed PRE-TAIL slot and the taa-only LATE slot)
    /// share one implementation — `declare_vb_graph` declares the pass in exactly one position
    /// per frame (`scene.ssao.is_some()` picks it), and the recorder replays the SAME body at
    /// the matching site.
    #[allow(clippy::too_many_arguments)]
    fn record_vb_viewt_dispatch(
        &self,
        vb_viewt_pass: crate::framegraph::PassId,
        cmd: VkCommandBuffer,
        targets: &GBufferTargets,
        forward: &ForwardTargets,
        vb: &VbTargets,
        scene: &GBufferScene<'_>,
        present_extent: VkExtent2D,
        fi: usize,
    ) {
        // SAFETY: recording is open (caller contract); `record_vb_pass` records the graph's
        // derived DEPTH_ATTACHMENT→SHADER_READ_ONLY (vb_depth) + UNDEFINED→GENERAL (viewt)
        // barriers for the "vb_viewt" pass into `cmd`.
        self.record_vb_pass(vb_viewt_pass, cmd, targets, forward, vb, scene, fi);
        let activation = scene.viewt_from_vb_depth.as_ref().expect(
            "invariant: plan.viewt_from_depth.is_some() ⇒ scene.viewt_from_vb_depth.is_some() (W1)",
        );
        let push = crate::compute::ViewtFromDepthRzPush::new(
            present_extent.width,
            present_extent.height,
            activation.view_z_a,
            activation.view_z_b,
        );
        let push_bytes = push.as_bytes();
        let set = &targets.viewt_from_vb_depth_set.as_ref().expect(
            "invariant: scene.viewt_from_vb_depth.is_some() ⇒ create wrote viewt_from_vb_depth_set",
        )[fi];
        let group_x = present_extent.width.div_ceil(8);
        let group_y = present_extent.height.div_ceil(8);
        // SAFETY: recording is open; `activation.pipeline` + its layout (declaring
        // `activation.layout` at set 0 AND the 16-byte COMPUTE push range) are live on this
        // device (caller contract); `set` binds the now-transitioned reverse-Z depth
        // (SHADER_READ, by the `record_vb_pass` call above) + `gViewT` (GENERAL) + the
        // camera-ring slot `fi`; `group_x`/`group_y` cover `present_extent` (the 8×8-tile
        // ceiling of the SAME extent the depth/gViewT images are sized to);
        // `&set.descriptor_set` is a single-element local alive for the call; `push_bytes`
        // is `VIEWT_FROM_DEPTH_RZ_PUSH_BYTES` (16) bytes at offset 0, exactly the declared
        // push range, and the backing `push` local outlives the call.
        unsafe {
            (self.fns.cmd_bind_pipeline)(
                cmd,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                activation.pipeline.pipeline,
            );
            (self.fns.cmd_bind_descriptor_sets)(
                cmd,
                VK_PIPELINE_BIND_POINT_COMPUTE,
                activation.pipeline.layout,
                0,
                1,
                &set.descriptor_set,
                0,
                ptr::null(),
            );
            (self.fns.cmd_push_constants)(
                cmd,
                activation.pipeline.layout,
                VK_SHADER_STAGE_COMPUTE_BIT,
                0,
                crate::compute::VIEWT_FROM_DEPTH_RZ_PUSH_BYTES,
                push_bytes.as_ptr().cast(),
            );
            (self.fns.cmd_dispatch)(cmd, group_x, group_y, 1);
        }
    }
}

#[cfg(test)]
mod vb_cull_readback_layout_tests {
    use super::{VbCullReadbackSources, vb_cull_readback_layout};

    /// The seven real source sizes on every current boot, spelled here so the exact-fit case pins
    /// the shipped geometry rather than an invented one: counter 16 B, batch list
    /// `INSTANCE_CAPACITY * 4`, records `INSTANCE_CAPACITY * DRAW_INDEXED_INDIRECT_STRIDE`,
    /// survivor list `INSTANCE_CAPACITY * 4`, late list `INSTANCE_CAPACITY * 4`, late counts
    /// `(INSTANCE_CAPACITY + 1) * 4` (the reserved frame slot), late records
    /// `INSTANCE_CAPACITY * DRAW_INDEXED_INDIRECT_STRIDE`.
    const COUNT: u64 = 16;
    const LIST: u64 = 1024 * 4;
    const RECORDS: u64 = 1024 * 20;
    const VIS: u64 = 1024 * 4;
    const LATE_VISIBLE: u64 = 1024 * 4;
    const LATE_COUNT: u64 = 1025 * 4;
    const LATE_RECORDS: u64 = 1024 * 20;

    /// `late_visible` and `late_count` are each copied TWICE — the PRE and the POST snapshot — so
    /// the staging is the seven sources plus a second copy of those two.
    const TOTAL: u64 = COUNT
        + LIST
        + RECORDS
        + VIS
        + 2 * LATE_VISIBLE
        + 2 * LATE_COUNT
        + LATE_RECORDS;

    const SRC: VbCullReadbackSources = VbCullReadbackSources {
        count: COUNT,
        list: LIST,
        records: RECORDS,
        vis: VIS,
        late_visible: LATE_VISIBLE,
        late_count: LATE_COUNT,
        late_records: LATE_RECORDS,
    };

    #[test]
    fn the_shipped_staging_fits_all_nine_regions_at_their_documented_offsets() {
        let l = vb_cull_readback_layout(&SRC, TOTAL);
        assert_eq!((l.count, l.list, l.records, l.vis), (COUNT, LIST, RECORDS, VIS));
        assert_eq!(
            (l.late_candidates, l.late_count_pre, l.late_survivors, l.late_count_post, l.late_records),
            (LATE_VISIBLE, LATE_COUNT, LATE_VISIBLE, LATE_COUNT, LATE_RECORDS)
        );
        assert_eq!(l.list_offset(), 16);
        assert_eq!(l.records_offset(), 16 + 4096);
        assert_eq!(l.vis_offset(), 16 + 4096 + 20480);
        assert_eq!(l.late_candidates_offset(), 16 + 4096 + 20480 + 4096);
        assert_eq!(l.late_count_pre_offset(), 16 + 4096 + 20480 + 4096 + 4096);
        assert_eq!(l.late_survivors_offset(), 16 + 4096 + 20480 + 4096 + 4096 + 4100);
        assert_eq!(l.late_count_post_offset(), 16 + 4096 + 20480 + 4096 + 4096 + 4100 + 4096);
        assert_eq!(
            l.late_records_offset(),
            16 + 4096 + 20480 + 4096 + 4096 + 4100 + 4096 + 4100
        );
        assert_eq!(l.total(), 65560, "the staging the host allocates must equal what is packed");
        assert!(l.is_untruncated(&SRC));
    }

    /// THE PROPERTY THAT MADE `min` WRONG: a region either carries its whole source or is absent.
    /// A partial copy would truncate a record array mid-record while every length the decode reads
    /// still looked sane, and nothing downstream could detect it.
    #[test]
    fn a_region_that_does_not_fit_whole_is_dropped_rather_than_truncated() {
        // One byte short of the full packing: the LATE RECORD region cannot fit, so it must be 0 —
        // never 20479.
        let l = vb_cull_readback_layout(&SRC, TOTAL - 1);
        assert_eq!(l.late_records, 0, "a short trailing region must be dropped, not truncated");
        assert_eq!((l.count, l.list, l.records, l.vis), (COUNT, LIST, RECORDS, VIS));
        assert!(!l.is_untruncated(&SRC), "and the truncation must be visible");

        // One byte short of RECORDS. VIS would FIT in the space RECORDS vacated (4096 <= 20479),
        // and letting it slide there is the defect this case exists to forbid: the host decodes at
        // constant offsets, so a slid region is read as the WRONG BYTES rather than as missing
        // data. The packing is a prefix, so VIS and every late region drop too.
        let l = vb_cull_readback_layout(&SRC, COUNT + LIST + RECORDS - 1);
        assert_eq!(
            (l.records, l.vis, l.late_candidates, l.late_count_pre),
            (0, 0, 0, 0),
            "a dropped region must drop every successor, not let one slide forward into its space"
        );
        assert_eq!(
            (l.late_survivors, l.late_count_post, l.late_records),
            (0, 0, 0),
            "and the prefix rule reaches the POST snapshot as well — the two snapshots are one \
             packing, not two"
        );
    }

    /// The PRE pair fits but the POST triple does not. The two snapshots must not be able to
    /// disagree about how much was packed: a staging that held the candidate list but not the
    /// survivor list would make plan A5's `S_b == K_b` compare a real list against a zero region.
    #[test]
    fn a_staging_that_holds_only_the_pre_snapshot_drops_the_whole_post_snapshot() {
        let pre = COUNT + LIST + RECORDS + VIS + LATE_VISIBLE + LATE_COUNT;
        let l = vb_cull_readback_layout(&SRC, pre);
        assert_eq!((l.late_candidates, l.late_count_pre), (LATE_VISIBLE, LATE_COUNT));
        assert_eq!((l.late_survivors, l.late_count_post, l.late_records), (0, 0, 0));
        assert!(!l.is_untruncated(&SRC));
    }

    #[test]
    fn a_staging_too_small_for_even_the_counter_packs_nothing() {
        let l = vb_cull_readback_layout(&SRC, COUNT - 1);
        assert_eq!((l.count, l.list, l.records, l.vis), (0, 0, 0, 0));
        assert_eq!(l.total(), 0);
        assert!(!l.is_untruncated(&SRC));
    }

    #[test]
    fn a_zero_sized_source_is_not_a_truncation() {
        // An absent optional buffer reports size 0; the layout must call that untruncated, or the
        // recorder's debug_assert would fire on a legitimately empty source.
        let src = VbCullReadbackSources { records: 0, ..SRC };
        let l = vb_cull_readback_layout(&src, TOTAL);
        assert_eq!(l.records, 0);
        assert_eq!(l.vis_offset(), COUNT + LIST);
        assert!(l.is_untruncated(&src));
    }
}

#[cfg(test)]
mod vb_cull_clamp_tests {
    use super::{GBufferMeshDraw, vb_cull_batch_count_visible_clamp};
    use crate::memory::BoundBuffer;

    /// A device-inert `BoundBuffer` — every handle field is null and nothing is mapped. The clamp
    /// reads only `base_instance`/`instance_count`, so the two buffer references a
    /// `GBufferMeshDraw` carries are pure padding here (the `fake_targets` idiom `targets.rs`'s own
    /// unit tests use).
    fn null_buffer() -> BoundBuffer {
        BoundBuffer {
            buffer: crate::ffi::VkBuffer::NULL,
            offset: 0,
            size: 0,
            mapped: None,
            block: 0,
        }
    }

    /// Builds the batch list from `(base_instance, instance_count)` pairs, borrowing ONE inert
    /// buffer for every batch's vertex/index slots.
    fn batches<'a>(buf: &'a BoundBuffer, spec: &[(u32, u32)]) -> Vec<GBufferMeshDraw<'a>> {
        spec.iter()
            .map(|&(base_instance, instance_count)| GBufferMeshDraw {
                vertex_buffer: buf,
                index_buffer: buf,
                index_count: 3,
                index_type: 0,
                base_instance,
                instance_count,
                casts_shadow: true,
                world_aabb: None,
            })
            .collect()
    }

    /// The ordinary case: every batch's region fits, so the clamp is the identity and NOTHING is
    /// dropped. This is the shape every golden-pinned scene takes, and it is what makes rung R2d-3
    /// inert on them.
    #[test]
    fn a_list_that_fits_is_not_clamped() {
        let buf = null_buffer();
        // Bases are the running prefix sum: 0, 4, 6 — regions [0,4), [4,6), [6,13).
        let m = batches(&buf, &[(0, 4), (4, 2), (6, 7)]);
        assert_eq!(vb_cull_batch_count_visible_clamp(&m, 1024), 3);
        assert_eq!(vb_cull_batch_count_visible_clamp(&[], 1024), 0, "an empty list clamps to 0");
    }

    /// THE BOUNDARY: the last batch ENDS exactly at the capacity. `base + count == visible_elems`
    /// is the last legal region (the slots written are `base ..= visible_elems - 1`), so the
    /// predicate must be `>` and not `>=` — an off-by-one here silently drops the final batch of
    /// every perfectly-sized frame, which renders correctly and is therefore invisible to a golden.
    #[test]
    fn a_batch_that_exactly_fills_the_capacity_survives() {
        let buf = null_buffer();
        let m = batches(&buf, &[(0, 4), (4, 2), (6, 7)]);
        assert_eq!(vb_cull_batch_count_visible_clamp(&m, 13), 3, "region [6,13) fits in 13 slots");
        assert_eq!(
            vb_cull_batch_count_visible_clamp(&m, 12),
            2,
            "one slot short: the last batch must be clamped away whole"
        );
    }

    /// The PREFIX property, stated as the thing that could actually go wrong: the clamp must return
    /// a boundary, never a filtered subset. Every batch below the returned index fits and every
    /// batch at or above it does NOT — checked exhaustively against the returned index rather than
    /// against a hand-copied expectation, so the assertion is about the property.
    #[test]
    fn the_clamp_is_a_prefix_boundary_not_a_filter() {
        let buf = null_buffer();
        // A deliberately lumpy list — a big batch in the middle, so a "filter" implementation
        // (skip the batch that does not fit, keep the smaller ones after it) would return a
        // DIFFERENT count and be caught here.
        let m = batches(&buf, &[(0, 2), (2, 1), (3, 100), (103, 1), (104, 1)]);
        for cap in [0_usize, 1, 2, 3, 4, 50, 102, 103, 104, 105, 4096] {
            let n = vb_cull_batch_count_visible_clamp(&m, cap);
            for (i, b) in m.iter().enumerate() {
                let fits = b.base_instance as usize + b.instance_count as usize <= cap;
                assert_eq!(
                    i < n,
                    fits,
                    "cap {cap}: batch {i} (base {}, count {}) is on the wrong side of the boundary \
                     {n} - the clamp filtered instead of truncating",
                    b.base_instance,
                    b.instance_count
                );
            }
        }
    }

    /// MONOTONICITY, which is what makes the prefix argument sound rather than merely observed:
    /// `gather_mixed_into` reads `base_instance = running` BEFORE `running += c`
    /// (`boyko_render/src/mesh_draw.rs:815-832`), so `base + count` is non-decreasing across the
    /// list and the predicate "does not fit" can never go back to false once it is true.
    ///
    /// Pinned on the GATHER'S OWN arithmetic — the running prefix sum is recomputed here rather
    /// than assumed — so this test fails if that invariant is ever broken upstream.
    #[test]
    fn ends_are_non_decreasing_so_the_predicate_is_monotone() {
        let buf = null_buffer();
        let counts = [4_u32, 1, 7, 2, 9, 1];
        let mut running = 0_u32;
        let spec: Vec<(u32, u32)> = counts
            .iter()
            .map(|&c| {
                let base = running;
                running += c;
                (base, c)
            })
            .collect();
        let m = batches(&buf, &spec);

        let ends: Vec<usize> =
            m.iter().map(|b| b.base_instance as usize + b.instance_count as usize).collect();
        assert!(
            ends.windows(2).all(|w| w[0] <= w[1]),
            "the gather's prefix sum must produce non-decreasing region ends; got {ends:?}"
        );

        // …and the clamp is therefore monotone in the capacity: a larger allocation never admits
        // FEWER batches.
        let mut prev = 0_usize;
        for cap in 0..=(running as usize + 2) {
            let n = vb_cull_batch_count_visible_clamp(&m, cap);
            assert!(n >= prev, "cap {cap}: the admitted prefix shrank from {prev} to {n}");
            prev = n;
        }
        assert_eq!(
            vb_cull_batch_count_visible_clamp(&m, running as usize),
            m.len(),
            "an allocation sized to the total instance count admits every batch"
        );
    }

    /// A zero-element survivor list (an unwired `vb_visible_instance`, which the recorder maps to
    /// `0`) admits NOTHING — the cull then dispatches zero lanes and every record keeps the value
    /// the host's own transfer fill wrote. Degrading to "no cull" rather than to "cull with an
    /// out-of-bounds region write" is the whole point of deriving the bound from the allocation.
    #[test]
    fn a_zero_element_list_admits_no_batch() {
        let buf = null_buffer();
        let m = batches(&buf, &[(0, 1), (1, 1)]);
        assert_eq!(vb_cull_batch_count_visible_clamp(&m, 0), 0);
    }
}
