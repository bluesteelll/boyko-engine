//! VG R3 piece 3, step P3-4 — **gate G-P3-D: the CULL SHADER's occlusion verdict equals
//! `boyko_render::hzb`'s, with no engine involved.**
//!
//! `vb_batch_cull.comp.hlsl`'s `occlusion_reject` is a hand-authored, statement-for-statement mirror
//! of the oracle's `project_aabb → select_texels → occluder_depth → occlusion_verdict` chain. A
//! mirror is a claim, and this file decides it — by dispatching the REAL committed module against
//! this file's own pyramid, its own instance rows, its own bounds and its own readback.
//!
//! # Why it dispatches the whole module rather than a leaf
//!
//! There is no leaf to dispatch: DXC inlines every helper into `%main`, and the module has one entry
//! point. So the gate observes the WHOLE PARTITION — which instances landed in `VbVisibleInstance`
//! and which in `VbLateVisible` — rather than a boolean. That is strictly more than G-P3-D needs for
//! the occlusion verdict, and it is what lets the sentinel corpus gate the FRUSTUM side too.
//!
//! # Why the file lives in `boyko_app`
//!
//! The oracle is `boyko_render::hzb`; the shader and its `.spv` are `boyko_rhi_vulkan`. The
//! dependency runs `boyko_render → boyko_rhi_vulkan`, never the reverse, so a test inside
//! `boyko_rhi_vulkan/tests/` cannot name the oracle. `boyko_app` is one of only two crates that name
//! both. This is verbatim `hzb_build_oracle_gate.rs`'s own placement argument.
//!
//! # NO ENGINE IS INVOLVED, and that is the point
//!
//! This gate creates its own pyramid image, its own twelve-binding set layout, its own pipeline and
//! its own buffers. It does NOT touch `HzbTargets`, `GBufferScene`, `GBufferTargets`, the framegraph
//! or the runner. The ONLY thing it shares with the engine is the compiled `vb_batch_cull.comp.spv`
//! and the host constants that describe its interface — a wiring bug the engine and the gate had in
//! common would otherwise cancel out and the gate would certify it.
//!
//! The boundary corpus additionally builds a THIRTEEN-binding layout over the
//! `-D VB_CULL_DEBUG_PROBE=1` variant of that same source, whose extra `VbCullDebug` @12 is the only
//! way the leaf's own `depth_near` is readable at all. Nothing in the engine binds or dispatches
//! that module; see `docs/SHADER-VARIANT-MANIFEST.md`'s row for it.
//!
//! For the same reason the push block and the 96-byte uniform are HAND-SERIALIZED here, field by
//! field, instead of being transmuted out of `present::scene_types`: the byte offsets ARE the
//! contract with the HLSL, and writing them out is what makes them reviewable.
//!
//! # ⚠️ WHAT THIS GATE CANNOT CLAIM
//!
//! * **That the ENGINE's cull reads the right pyramid, the right ring, the right matrix or the right
//!   extent.** It builds its own everything. That is the engine-level gate's job, and this division
//!   is verbatim the one piece 1 established between G3 (the shader vs the oracle) and G8 (the
//!   engine's pyramid vs the oracle).
//! * **That ALL the arithmetic is bit-identical to the oracle's.** It is not, and it cannot be made
//!   so: the window rect goes through a divide, and Vulkan's precision appendix permits `OpFDiv`
//!   2.5 ULP at 32-bit where Rust's `/` is the IEEE 0.5-ULP one. The P3-4 diagnostic step MEASURED
//!   the consequence over 72 boundary probes — rect, level and all four taps identical on every
//!   probe, `depth_near` (the quotient) apart by up to 1 ULP on 6 of them, in BOTH directions, and
//!   2 verdicts flipped by it. That is why the VERDICT no longer runs through a quotient: it is
//!   `∀i: cz_i < occ · cw_i`, whose every operand is an exactly-mirrored `precise` fold and whose
//!   one operation is a correctly-rounded multiply, so the DECIDING arithmetic agrees by
//!   construction. The census below still measures and prints `depth_near`'s distance because it is
//!   the leaf's most sensitive scalar and therefore the earliest warning of a fold that drifted.
//! * **Anything about the `.spv`'s content.** `vb_batch_cull_spv_sync.rs` owns that.
//! * **Anything at all when no GPU is present.** Every test here SKIPS with an eprintln in that
//!   case, and a skipped gate certifies nothing.
//!
//! # The controls (source corruptions, run by the orchestrator, published either way)
//!
//! | # | corruption | expected |
//! |---|---|---|
//! | D1 | `ceil(hi) - 1` instead of `floor(hi)` in step 3 | RED on the exact-integer-edge extent |
//! | D2 | clamp `level` down to `levels - 1` instead of KEEPing | RED on the truncated-layout case — a FALSE REJECT |
//! | D3 | drop `hzb_msb`'s `v == 0u` guard on ONE axis | RED on the `1 × 1` layout, where single-texel rects are unconditional and the UNSIGNED `max` lets the un-guarded axis win |
//! | D4 | hoist the sentinel guard AFTER the Arvo fold | RED on the sentinel corpus: case (a) is frustum-deleted, case (b) is occlusion-deleted |
//! | D5 | swap the explicit `precise` fold for `dot()` | **report whether the differential moves — a null result IS the finding.** It measures whether this driver reassociates `OpDot`. Do NOT "fix" a null result by keeping `dot()`: the precedent is about what the spec PERMITS |
//! | D6 | drop `precise` from the projection locals | report whether the differential moves. ⚠️ This one has an artifact-level pin behind it — `vb_batch_cull_spv_sync.rs`'s `no_contraction` count MUST move. If it does not, `precise` is not reaching the artifact and THAT is the finding |
//! | D7 | put the verdict back on the quotient (`return depth_near < occ;` in the shipping module, with `depth_near` un-`#ifdef`-ed) | RED on the boundary corpus. This is the corruption the corpus was rebuilt to catch, and it is the state the module was MEASURED in: 2 of 72 probes partitioned differently. A GREEN result here would mean the corpus no longer lands on the boundary — re-derive the plant, do not accept the pass |
//!
//! # A reliance this file inherits and states rather than hides
//!
//! Outputs are `HostVisibleCoherent` storage buffers read straight off their persistent mapping
//! after a fence wait. The RHI's `BarrierStage` has no `HOST` bit, so no availability barrier to the
//! host domain can be recorded; `hzb_build_oracle_gate.rs` relies on exactly the same thing for its
//! readback and ships green. If that reliance ever breaks it breaks BOTH gates at once, loudly (the
//! non-vacuity poison below would survive), not silently.
//!
//! # Run
//!
//! `cargo test -p boyko-app --test hzb_verdict_oracle_gate -- --ignored --nocapture
//! --test-threads=1` with `BOYKO_DISABLE_VALIDATION=1`.

use core::ptr::NonNull;

use boyko_rhi::{
    BarrierAccess, BarrierStage, BindGroupDesc, BindGroupEntry, BindGroupLayoutDesc,
    BindGroupLayoutEntry, BufferDesc, BufferImageCopy, BufferUsage, ComputePipelineDesc,
    DescriptorKind, Format, ImageAspect, ImageBarrierDesc, ImageLayout, ImageSubresourceRange,
    ImageUsage, MemoryLocation, RhiCommandEncoder, RhiDevice, RhiQueue, ShaderStage, TextureDesc,
    TextureDimension,
};
use boyko_rhi_vulkan::compute::{
    VB_BATCH_CULL_LOCAL_SIZE_X, VB_BATCH_CULL_PUSH_BYTES, VB_BATCH_DESC_STRIDE,
    VB_CULL_DEBUG_LAYOUT_BINDINGS, VB_CULL_DEBUG_RECORD_WORDS, VB_CULL_LAYOUT_BINDINGS,
    VB_CULL_UNIFORM_BYTES, vb_batch_cull_debug_spirv, vb_batch_cull_spirv,
};
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::present::{VB_CULL_OCC_ARMED, VB_CULL_OCC_FORCE_KEEP, VB_CULL_OCC_FORCE_LATE};
use boyko_rhi_vulkan::rhi_impl::{ComputePipeline, VulkanBindGroupLayout, VulkanShaderModule};

use boyko_render::csm_caster::arvo_transform;
use boyko_render::frustum::{
    FRUSTUM_PLANE_COUNT, Plane, aabb_outside_frustum, frustum_planes_from_view_proj,
};
use boyko_render::hzb::{
    HzbLayout, KeepReason, OcclusionVerdict, build_pyramid, occlusion_verdict, project_aabb,
    select_texels,
};
use boyko_render::occlusion_marker::VB_INST_FLAG_OCCLUSION_CULLING;

// ==============================================================================================
// The wire layouts, mirrored FIELD FOR FIELD from `vb_batch_cull.comp.hlsl`
// ==============================================================================================

/// `VkDrawIndexedIndirectCommand`'s byte stride — the shader's `DRAW_INDEXED_INDIRECT_STRIDE`.
const DRAW_RECORD_STRIDE: usize = 20;
/// Byte offset of `instanceCount` inside that record — the shader's `INSTANCE_COUNT_OFFSET`.
const INSTANCE_COUNT_OFFSET: usize = 4;
/// `VbInstanceRow`: three `float4` affine rows, `mesh_id` @48, `flags` @52, `uint2 _pad` @56.
const INSTANCE_ROW_BYTES: usize = 64;
/// `MeshLocalBounds`: `float3 bmin` @0, pad @12, `float3 bmax` @16, pad @28.
const MESH_BOUNDS_BYTES: usize = 32;

// The host's own stride constant must agree with the descriptor this file serializes.
const _: () = assert!(VB_BATCH_DESC_STRIDE as usize == 32);

/// The value every readback word is poisoned with before the dispatch.
///
/// It cannot be a legitimate output: every list this gate reads holds a GLOBAL INSTANCE INDEX below
/// `1 << 20`, and every count is below the batch/instance capacities this file allocates. So "the
/// dispatch never ran" and "the shader never wrote this slot" both surface as this exact word
/// instead of as a plausible-looking disagreement.
const READBACK_POISON: u32 = 0xDEAD_BEEF;

/// One instance's row, in the shader's own field order.
#[derive(Clone, Copy, Debug)]
struct TestInstance {
    /// The 3×4 row-major affine as three `[linear_row.xyz | translation]` quads.
    rows: [[f32; 4]; 3],
    mesh_id: u32,
    flags: u32,
}

impl TestInstance {
    /// The identity affine, which every corpus but the sentinel one uses.
    ///
    /// ⚠️ NOT a convenience. The shader folds the local box with `dot(row.xyz, lc) + row.w` while the
    /// host oracle `arvo_transform` folds it as an explicit left sum, and the two may differ by an
    /// ULP for a general affine — a difference this gate is NOT testing and must not be perturbed
    /// by. Under the identity every product is `1 · x` or `0 · x`, so both spellings are EXACT and
    /// the world box the shader tests is bit-identical to the one the oracle is handed. The
    /// randomness lives where the gate is aimed: the view-projection and the bounds.
    fn identity(mesh_id: u32, flags: u32) -> Self {
        Self {
            rows: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
            mesh_id,
            flags,
        }
    }

    fn to_bytes(self) -> [u8; INSTANCE_ROW_BYTES] {
        let mut b = [0u8; INSTANCE_ROW_BYTES];
        for (r, row) in self.rows.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                let o = r * 16 + c * 4;
                b[o..o + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        b[48..52].copy_from_slice(&self.mesh_id.to_le_bytes());
        b[52..56].copy_from_slice(&self.flags.to_le_bytes());
        b
    }
}

/// One mesh's LOCAL-space AABB.
#[derive(Clone, Copy, Debug)]
struct TestBounds {
    bmin: [f32; 3],
    bmax: [f32; 3],
}

impl TestBounds {
    /// `MeshLocalBounds::UNKNOWN` — the INVERTED sentinel every consumer must read as KEEP.
    const UNKNOWN: Self = Self { bmin: [1e30; 3], bmax: [-1e30; 3] };

    fn to_bytes(self) -> [u8; MESH_BOUNDS_BYTES] {
        let mut b = [0u8; MESH_BOUNDS_BYTES];
        for (i, v) in self.bmin.iter().enumerate() {
            b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        for (i, v) in self.bmax.iter().enumerate() {
            b[16 + i * 4..16 + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    /// The shader's OUTER guard, `any(bmin > bmax)`, mirrored component for component.
    ///
    /// ⚠️ NOT spelled `!(bmin <= bmax)`. That is the ORACLE's world-space guard inside
    /// `project_aabb`, a different test on different data — and the difference is exactly that a NaN
    /// coordinate is NOT caught by this one. The shader spells both, in the same two places.
    fn is_sentinel(self) -> bool {
        (0..3).any(|i| self.bmin[i] > self.bmax[i])
    }
}

/// One batch's descriptor.
#[derive(Clone, Copy, Debug)]
struct TestBatch {
    base_instance: u32,
    instance_count: u32,
}

impl TestBatch {
    /// The level-1 union AABB this gate always uses: large but FINITE on both corners, so the batch
    /// survives every plane set and the partition observed is the per-INSTANCE decision alone.
    ///
    /// Finite rather than infinite for the shipped module's own recorded reason: `dot(n, p) + d`
    /// against an infinite corner can produce a NaN, and a NaN picks the OTHER operand under
    /// `NMin`/`NMax` rather than propagating.
    const UNION_HALF: f32 = 1.0e18;

    fn to_bytes(self) -> [u8; 32] {
        let mut b = [0u8; 32];
        for i in 0..3 {
            b[i * 4..i * 4 + 4].copy_from_slice(&(-Self::UNION_HALF).to_le_bytes());
            b[16 + i * 4..16 + i * 4 + 4].copy_from_slice(&Self::UNION_HALF.to_le_bytes());
        }
        b[12..16].copy_from_slice(&self.instance_count.to_le_bytes());
        b[28..32].copy_from_slice(&self.base_instance.to_le_bytes());
        b
    }
}

/// The 112-byte `VbBatchCullPush`, serialized in the shader's own member order.
fn push_bytes(
    planes: &[Plane; FRUSTUM_PLANE_COUNT],
    batch_count: u32,
    visible_cap: u32,
    phase: u32,
    occ_flags: u32,
) -> [u8; VB_BATCH_CULL_PUSH_BYTES as usize] {
    let mut b = [0u8; VB_BATCH_CULL_PUSH_BYTES as usize];
    for (p, plane) in planes.iter().enumerate() {
        for (c, v) in plane.iter().enumerate() {
            let o = p * 16 + c * 4;
            b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    b[96..100].copy_from_slice(&batch_count.to_le_bytes());
    b[100..104].copy_from_slice(&visible_cap.to_le_bytes());
    b[104..108].copy_from_slice(&phase.to_le_bytes());
    b[108..112].copy_from_slice(&occ_flags.to_le_bytes());
    b
}

/// The 96-byte `VbCullUniform`, serialized in the shader's own member order.
///
/// `view_proj_rows` is MATH-ROW form — `pv[row][col]`, `clip = pv · world` — which is verbatim what
/// `boyko_render::hzb::project_aabb` takes. The engine performs one byte inversion out of its
/// column-major push storage to get here; this gate never has a column-major form at all, which is
/// deliberate: a transposed matrix still projects, to a systematically wrong rect with every guard
/// silent, so the fewer places the convention is restated the better.
fn uniform_bytes(
    view_proj_rows: &[[f32; 4]; 4],
    src_extent: [u32; 2],
    base_extent: [u32; 2],
    levels: u32,
    frame_index: u32,
) -> [u8; VB_CULL_UNIFORM_BYTES as usize] {
    let mut b = [0u8; VB_CULL_UNIFORM_BYTES as usize];
    for (r, row) in view_proj_rows.iter().enumerate() {
        for (c, v) in row.iter().enumerate() {
            let o = r * 16 + c * 4;
            b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    b[64..68].copy_from_slice(&src_extent[0].to_le_bytes());
    b[68..72].copy_from_slice(&src_extent[1].to_le_bytes());
    b[72..76].copy_from_slice(&base_extent[0].to_le_bytes());
    b[76..80].copy_from_slice(&base_extent[1].to_le_bytes());
    b[80..84].copy_from_slice(&levels.to_le_bytes());
    b[84..88].copy_from_slice(&frame_index.to_le_bytes());
    b
}

// ==============================================================================================
// The matrix, the pattern, and the deterministic RNG
// ==============================================================================================

/// A reverse-Z infinite-far perspective in MATH-ROW form, view space with `+Z` FORWARD.
///
/// ```text
/// row0 = [f / aspect, 0, 0, 0]      cx = (f/aspect) · x
/// row1 = [0,          f, 0, 0]      cy = f · y
/// row2 = [0, 0, 0, near]            cz = near          ⇒  z_ndc = near / z
/// row3 = [0, 0, 1, 0]               cw = z
/// ```
///
/// HAND-BUILT rather than taken from `boyko_render::view::forward_view_proj_rows`, for this gate's
/// governing reason: a wrong matrix builder shared with the engine would cancel out. It is also the
/// simplest matrix in which "put an occluder at depth `d`" is a one-line statement — `z_ndc` is
/// exactly `near / z`, monotonically decreasing in `z`, which is what reverse-Z means.
fn perspective_rows(fov_y_tan: f32, aspect: f32, near: f32) -> [[f32; 4]; 4] {
    let f = 1.0 / fov_y_tan;
    [
        [f / aspect, 0.0, 0.0, 0.0],
        [0.0, f, 0.0, 0.0],
        [0.0, 0.0, 0.0, near],
        [0.0, 0.0, 1.0, 0.0],
    ]
}

/// The deterministic xorshift64 this repository already uses for reproducible corpora (the shape
/// `boyko_render::hzb`'s own property tests carry). No dependency, no clock, reproducible failures.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 32) as u32
    }

    /// A float in `[0, 1)`, exactly representable.
    fn unit(&mut self) -> f32 {
        f32::from(self.next_u32() as u16) / 65536.0
    }

    /// A float in `[lo, hi)`.
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
}

/// A depth pattern in `(0, 1]`, keyed by the extent so a stale or wrong-extent upload cannot pass.
fn depth_pattern(layout: &HzbLayout) -> Vec<f32> {
    let (w, h) = (layout.x().source(), layout.y().source());
    let mut depth = vec![0.0f32; layout.source_len()];
    for y in 0..h {
        for x in 0..w {
            // A 23-bit numerator over `2^23`: exactly representable, 8.4 million distinct values, so
            // two different footprints almost never share a minimum by accident.
            let mut v = x
                .wrapping_mul(0x9E37_79B1)
                ^ y.wrapping_mul(0x85EB_CA77)
                ^ w.wrapping_mul(0xC2B2_AE3D)
                ^ h.wrapping_mul(0x27D4_EB2F);
            v ^= v >> 16;
            v = v.wrapping_mul(0x85EB_CA6B);
            v ^= v >> 13;
            v = v.wrapping_mul(0xC2B2_AE35);
            v ^= v >> 16;
            // The range is `(0.25, 0.75]` EXACTLY, and both ends are load-bearing. Centred so both
            // verdicts are reachable — a box nearer than the pattern KEEPs, one farther REJECTs; a
            // pattern hugging 0 or 1 would make one of the two unobservable and the corpus would be
            // vacuous in exactly one direction. The `0.75` CEILING is what `spread_boxes`' arm 0
            // relies on to be unconditionally NotOccluded, and
            // [`the_depth_pattern_stays_inside_the_band_the_corpora_assume`] pins it.
            let k = (v & 0x003F_FFFF) + 1;
            depth[y as usize * w as usize + x as usize] = 0.25 + (k as f32) / 8_388_608.0;
        }
    }
    depth
}

// ==============================================================================================
// The case, and the partition it is measured against
// ==============================================================================================

/// One dispatch's worth of inputs. Everything the shader can read is here and nothing else exists.
struct CullCase {
    label: String,
    /// The pyramid's layout — its `source()` is also the framebuffer extent the verdict projects
    /// into, exactly as `HzbPlan` ties the two in the engine.
    layout: HzbLayout,
    /// The flat pyramid, in the oracle's own level-major layout.
    pyramid: Vec<f32>,
    view_proj_rows: [[f32; 4]; 4],
    planes: [Plane; FRUSTUM_PLANE_COUNT],
    bounds: Vec<TestBounds>,
    instances: Vec<TestInstance>,
    batches: Vec<TestBatch>,
    occ_flags: u32,
}

/// What the early phase is expected to do with one instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expected {
    /// Frustum-rejected: in NEITHER list.
    Dropped,
    /// Drawn now: appended to `VbVisibleInstance`.
    Early,
    /// Deferred: appended to `VbLateVisible`.
    Late,
}

/// The host oracle for ONE instance's early-phase fate, in the shader's own order.
///
/// This is the whole comparison. It is composed out of the SHIPPED oracles —
/// `csm_caster::arvo_transform`, `frustum::aabb_outside_frustum`, `hzb::occlusion_verdict` — rather
/// than re-derived here, so a bug in this file cannot agree with a bug in the shader.
fn expected_fate(case: &CullCase, g: u32) -> (Expected, Option<OcclusionVerdict>) {
    let inst = case.instances[g as usize];
    let b = case.bounds[inst.mesh_id as usize];
    if b.is_sentinel() {
        // The OUTER guard: unknown bounds are KEPT, tested BEFORE the transform, never
        // frustum-tested and never occlusion-tested. Absence of bounds is not evidence of
        // invisibility.
        return (Expected::Early, None);
    }
    let lc = [
        (b.bmin[0] + b.bmax[0]) * 0.5,
        (b.bmin[1] + b.bmax[1]) * 0.5,
        (b.bmin[2] + b.bmax[2]) * 0.5,
    ];
    let lh = [
        (b.bmax[0] - b.bmin[0]) * 0.5,
        (b.bmax[1] - b.bmin[1]) * 0.5,
        (b.bmax[2] - b.bmin[2]) * 0.5,
    ];
    let (wc, wh) = arvo_transform(&inst.rows, lc, lh);
    let mn = [wc[0] - wh[0], wc[1] - wh[1], wc[2] - wh[2]];
    let mx = [wc[0] + wh[0], wc[1] + wh[1], wc[2] + wh[2]];

    if aabb_outside_frustum(&case.planes, mn, mx) {
        return (Expected::Dropped, None);
    }
    let armed = case.occ_flags & VB_CULL_OCC_ARMED != 0;
    let force_keep = case.occ_flags & VB_CULL_OCC_FORCE_KEEP != 0;
    let force_late = case.occ_flags & VB_CULL_OCC_FORCE_LATE != 0;
    let marked = inst.flags & VB_INST_FLAG_OCCLUSION_CULLING != 0;
    if !(armed && !force_keep && marked) {
        return (Expected::Early, None);
    }
    if force_late {
        return (Expected::Late, None);
    }
    let verdict = occlusion_verdict(&case.layout, &case.pyramid, &case.view_proj_rows, mn, mx);
    let fate = match verdict {
        OcclusionVerdict::Reject => Expected::Late,
        OcclusionVerdict::Keep(_) => Expected::Early,
    };
    (fate, Some(verdict))
}

/// What the GPU actually did, per batch.
struct Partition {
    /// `VbIndirect[b].instanceCount` — the early survivor count.
    early_count: Vec<u32>,
    /// `VbLateCount[b]` — the deferral count.
    late_count: Vec<u32>,
    /// The whole `VbVisibleInstance` allocation, poison and all.
    visible: Vec<u32>,
    /// The whole `VbLateVisible` allocation, poison and all.
    late_visible: Vec<u32>,
    /// `VbLateCount`'s reserved TAIL slot — the frame index the GPU observed in the uniform.
    gpu_frame_index: u32,
    /// The whole `VbCullDebug` allocation, poison and all — EMPTY unless the case ran on the
    /// `-D VB_CULL_DEBUG_PROBE=1` rig, which is the only module that declares the sink.
    debug: Vec<u32>,
}

// ==============================================================================================
// The DIAGNOSTIC sink, mirrored from `vb_batch_cull.comp.hlsl`'s `VB_CULL_DEBUG_PROBE` block
// ==============================================================================================

/// The word the shader writes for a record field the exiting stage never computed.
///
/// `0xFFFFFFFF` is a quiet NaN, so a reader that forgot to branch on the stage gets a NaN rather
/// than a plausible depth. It is DISTINCT from [`READBACK_POISON`], which means the shader never
/// wrote the slot at all — the two failures are different and must not read alike.
const VB_DBG_UNSET: u32 = 0xFFFF_FFFF;

/// The exits of `occlusion_reject`, in its own source order. 1..6 mirror `KeepReason` one for one;
/// 2 and 4 are its single `NonFinite`, split by WHICH finiteness guard fired.
const VB_DBG_STAGE_UNORDERED_BOX: u32 = 1;
const VB_DBG_STAGE_CLIP_NON_FINITE: u32 = 2;
const VB_DBG_STAGE_BEHIND_EYE: u32 = 3;
const VB_DBG_STAGE_NDC_NON_FINITE: u32 = 4;
const VB_DBG_STAGE_EMPTY_RECT: u32 = 5;
const VB_DBG_STAGE_LEVEL_UNAVAIL: u32 = 6;
/// The strict comparison was REACHED — every field of the record is defined.
const VB_DBG_STAGE_VERDICT: u32 = 7;

/// One `VbCullDebug` record, decoded. The shader's own eight words, named.
#[derive(Clone, Copy, Debug)]
struct DebugRecord {
    stage: u32,
    /// `depth_near` AS BITS. Never as an `f32`, because the whole question this record answers is
    /// which bit pattern the shader arrived at.
    depth_near_bits: u32,
    occ_bits: u32,
    level: u32,
    /// `tap_x0, tap_x1, tap_y0, tap_y1` — the SHIFTED texel coordinates of the four taps.
    taps: [u32; 4],
}

impl DebugRecord {
    /// Decodes the record for global instance `g` out of a [`Partition::debug`] readback.
    fn decode(words: &[u32], g: usize) -> Self {
        let o = g * VB_CULL_DEBUG_RECORD_WORDS as usize;
        Self {
            stage: words[o],
            depth_near_bits: words[o + 1],
            occ_bits: words[o + 2],
            level: words[o + 3],
            taps: [words[o + 4], words[o + 5], words[o + 6], words[o + 7]],
        }
    }

    /// The stage's name, or a loud spelling of a word that is not a legal stage at all.
    fn stage_name(self) -> &'static str {
        match self.stage {
            VB_DBG_STAGE_UNORDERED_BOX => "unordered-box",
            VB_DBG_STAGE_CLIP_NON_FINITE => "clip-non-finite",
            VB_DBG_STAGE_BEHIND_EYE => "behind-eye",
            VB_DBG_STAGE_NDC_NON_FINITE => "ndc-non-finite",
            VB_DBG_STAGE_EMPTY_RECT => "empty-rect",
            VB_DBG_STAGE_LEVEL_UNAVAIL => "level-unavailable",
            VB_DBG_STAGE_VERDICT => "VERDICT",
            READBACK_POISON => "!! POISON (the shader never wrote this record) !!",
            _ => "!! not a legal stage !!",
        }
    }
}

/// The IEEE-754 TOTAL ORDER key of a `f32` bit pattern: the signed index of that float in the
/// ordered sequence of all representable values.
///
/// Positive floats are already ordered by their bit pattern; a negative one is ordered by the
/// NEGATION of its magnitude bits, which also collapses `-0.0` and `+0.0` onto the same key — the
/// right reading here, because the shader's own `hzb_conservative_min` is MEASURED to produce a
/// `-0.0` where the oracle produces `+0.0`, and the verdict compares `cz` against `occ · cw`, where
/// the two zeros give products that IEEE compares EQUAL.
fn ordered_key(bits: u32) -> i64 {
    if bits & 0x8000_0000 != 0 {
        -i64::from(bits & 0x7FFF_FFFF)
    } else {
        i64::from(bits)
    }
}

/// How many representable `f32` values separate two bit patterns, or `None` when either is a NaN
/// (including the [`VB_DBG_UNSET`] filler) and the distance therefore means nothing.
fn ulp_distance(a_bits: u32, b_bits: u32) -> Option<u64> {
    if f32::from_bits(a_bits).is_nan() || f32::from_bits(b_bits).is_nan() {
        return None;
    }
    Some((ordered_key(a_bits) - ordered_key(b_bits)).unsigned_abs())
}

// ==============================================================================================
// Device boot + raw-mapping helpers (the `hzb_build_oracle_gate.rs` shapes)
// ==============================================================================================

/// Boots an offscreen context (validation off), or `None` with a SKIP log when no GPU/loader.
///
/// This is the ONLY reason the gate skips. Everything else — a failed allocation, a poisoned slot,
/// a disagreeing verdict — fails the test.
fn boot_or_skip(what: &str) -> Option<VulkanContext> {
    match VulkanContext::boot(InstanceConfig {
        enable_validation: false,
        ..InstanceConfig::default()
    }) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            eprintln!("SKIP hzb_verdict_oracle_gate::{what}: GPU / loader unavailable ({e:?})");
            None
        }
    }
}

/// Writes raw bytes into a host-coherent mapping at `offset`.
///
/// # Safety
///
/// `base` must point at a live host-coherent mapping of at least `offset + src.len()` bytes, and no
/// GPU work touching it may be in flight (the caller has not submitted, or has fence-waited).
unsafe fn write_bytes(base: NonNull<u8>, offset: usize, src: &[u8]) {
    // SAFETY: forwarded verbatim from this function's own contract — `base + offset .. + src.len()`
    // is inside the mapping, and `copy_nonoverlapping` into a mapping the caller owns exclusively
    // cannot overlap `src`, which is a separate host allocation.
    unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), base.as_ptr().add(offset), src.len()) }
}

/// Fills `count` 32-bit words of a host-coherent mapping with `value`.
///
/// # Safety
///
/// `base` must point at a live host-coherent mapping of at least `count * 4` bytes, and no GPU work
/// touching it may be in flight.
unsafe fn fill_words(base: NonNull<u8>, count: usize, value: u32) {
    // SAFETY: forwarded verbatim from this function's own contract — every `base + i*4` for
    // `i < count` is inside the mapping, and `write_unaligned` imposes no alignment requirement (a
    // sub-allocated buffer's mapping carries only the block's alignment guarantee).
    unsafe {
        let p = base.as_ptr().cast::<u32>();
        for i in 0..count {
            p.add(i).write_unaligned(value);
        }
    }
}

/// Reads `count` 32-bit words out of a host-coherent mapping.
///
/// # Safety
///
/// `base` must point at a live host-coherent mapping of at least `count * 4` bytes, and no GPU work
/// touching it may be in flight (the caller fence-waited).
unsafe fn read_words(base: NonNull<u8>, count: usize) -> Vec<u32> {
    // SAFETY: forwarded verbatim from this function's own contract — every `base + i*4` for
    // `i < count` is inside the mapping, the bytes are stable, and `read_unaligned` imposes no
    // alignment requirement.
    unsafe {
        let p = base.as_ptr().cast::<u32>();
        (0..count).map(|i| p.add(i).read_unaligned()).collect()
    }
}

// ==============================================================================================
// The rig — the objects that do not depend on the case
// ==============================================================================================

/// The set layout, the shader module and the pipeline, built ONCE per test.
///
/// Hoisted out of [`run_case`] because the random corpus dispatches a hundred-odd cases and a
/// per-case `vkCreateComputePipelines` would dominate its runtime. Nothing case-dependent lives
/// here, which is the property that makes the hoist safe.
struct Rig {
    layout: VulkanBindGroupLayout,
    module: VulkanShaderModule,
    pipeline: ComputePipeline,
    /// True for the `-D VB_CULL_DEBUG_PROBE=1` variant, which declares `VbCullDebug` @12. It is
    /// what makes [`run_case`] allocate, bind and read back the sink; the SHIPPING module declares
    /// no such binding, and binding one to it would be a written descriptor no shader loads.
    debug_records: bool,
}

impl Rig {
    /// The gate's OWN twelve-binding `vb_cull_layout` over the SHIPPING module: eleven COMPUTE
    /// storage buffers and, at @9, one SAMPLED image.
    fn new(ctx: &VulkanContext) -> Self {
        Self::build(ctx, vb_batch_cull_spirv(), VB_CULL_LAYOUT_BINDINGS, false)
    }

    /// The same layout PLUS the `VbCullDebug` storage buffer at @12, over the
    /// `-D VB_CULL_DEBUG_PROBE=1` module.
    ///
    /// The two rigs coexist so the boundary survey can dispatch BOTH over every probe: the
    /// SHIPPING one decides the verdict (nothing about that comparison is delegated to a
    /// diagnostic artifact) and this one supplies the numbers.
    fn debug_probe(ctx: &VulkanContext) -> Self {
        Self::build(ctx, vb_batch_cull_debug_spirv(), VB_CULL_DEBUG_LAYOUT_BINDINGS, true)
    }

    /// Built from the host's arity constants and the `@9` position rather than from a literal, so a
    /// widened layout fails to compile here instead of binding resources to the wrong slots —
    /// `create_bind_group` matches entries to layout bindings POSITIONALLY.
    fn build(
        ctx: &VulkanContext,
        spirv: &'static [u32],
        bindings: u32,
        debug_records: bool,
    ) -> Self {
        const HZB_BINDING: u32 = 9;
        let entries: Vec<BindGroupLayoutEntry> = (0..bindings)
            .map(|binding| BindGroupLayoutEntry {
                binding,
                count: 1,
                kind: if binding == HZB_BINDING {
                    DescriptorKind::SampledImage
                } else {
                    DescriptorKind::StorageBuffer
                },
                stage: ShaderStage::COMPUTE,
            })
            .collect();
        let layout = ctx
            .create_bind_group_layout(&BindGroupLayoutDesc { entries: &entries })
            .unwrap_or_else(|e| panic!("vb_cull set layout ({bindings} bindings): {e:?}"));
        let module = ctx
            .create_shader_module(spirv)
            .unwrap_or_else(|e| panic!("vb_batch_cull shader module: {e:?}"));
        let pipeline = ctx
            .create_compute_pipeline(&ComputePipelineDesc {
                module: &module,
                entry: c"main",
                push_constant_bytes: VB_BATCH_CULL_PUSH_BYTES,
                bind_group_layout: Some(&layout),
                spec_constants: &[],
            })
            .unwrap_or_else(|e| panic!("vb_batch_cull pipeline: {e:?}"));
        Self { layout, module, pipeline, debug_records }
    }

    /// # Safety
    ///
    /// Every object was created on `ctx` by [`Rig::build`] — through [`Rig::new`] or
    /// [`Rig::debug_probe`] — and no submission referencing it is in flight (every [`run_case`]
    /// fence-waited before returning). Each is consumed by value and so destroyed exactly once, in
    /// reverse acquisition order.
    unsafe fn destroy(self, ctx: &VulkanContext) {
        // SAFETY: forwarded verbatim from this function's own contract.
        unsafe {
            ctx.destroy_compute_pipeline(self.pipeline);
            ctx.destroy_shader_module(self.module);
            ctx.destroy_bind_group_layout(self.layout);
        }
    }
}

// ==============================================================================================
// One case: upload, dispatch, read back
// ==============================================================================================

/// Dispatches the EARLY phase of the real `vb_batch_cull` module over `case` and returns what it
/// wrote. Allocates and frees everything it needs per call.
fn run_case(ctx: &VulkanContext, rig: &Rig, case: &CullCase) -> Partition {
    let label = &case.label;
    let batch_count = case.batches.len();
    let instance_count = case.instances.len();
    assert!(batch_count > 0 && instance_count > 0, "[{label}] an empty case proves nothing");

    let levels = case.layout.levels();
    let [base_w, base_h] = case.layout.level_extent(0);
    let [src_w, src_h] = [case.layout.x().source(), case.layout.y().source()];
    assert_eq!(
        case.pyramid.len(),
        case.layout.pyramid_len(),
        "[{label}] the pyramid is the wrong length for its layout"
    );

    // ---- 1) buffers, all HostVisibleCoherent so upload and readback need no staging ------------
    let new_buf = |bytes: u64, what: &str| {
        ctx.create_buffer(&BufferDesc {
            size: bytes,
            usage: BufferUsage::STORAGE,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .unwrap_or_else(|e| panic!("[{label}] {what} ({bytes} B): {e:?}"))
    };
    let map = |b: &_, what: &str| {
        ctx.buffer_mapped_ptr(b).unwrap_or_else(|| panic!("[{label}] {what} is not host-mapped"))
    };

    // `VbLateCount` carries ONE reserved tail slot past the per-batch words: the shader reads its
    // own element count off the descriptor (`GetDimensions`) and stamps the frame index into the
    // LAST element, so the allocation's size is what places that slot.
    let late_count_elems = batch_count + 1;

    let indirect = new_buf((batch_count * DRAW_RECORD_STRIDE) as u64, "VbIndirect");
    let batch_desc = new_buf((batch_count * 32) as u64, "VbBatchDesc");
    let cull_visible = new_buf((batch_count * 4) as u64, "VbCullVisible");
    let cull_count = new_buf(4, "VbCullCount");
    let instances = new_buf((instance_count * INSTANCE_ROW_BYTES) as u64, "gVbInstances");
    let mesh_bounds = new_buf((case.bounds.len() * MESH_BOUNDS_BYTES) as u64, "gMeshBounds");
    let visible_instance = new_buf((instance_count * 4) as u64, "VbVisibleInstance");
    let late_visible = new_buf((instance_count * 4) as u64, "VbLateVisible");
    let cull_uniform = new_buf(u64::from(VB_CULL_UNIFORM_BYTES), "VbCullUni");
    let indirect_late = new_buf((batch_count * DRAW_RECORD_STRIDE) as u64, "VbIndirectLate");
    let late_count = new_buf((late_count_elems * 4) as u64, "VbLateCount");
    let pyramid_staging = ctx
        .create_buffer(&BufferDesc {
            size: (case.pyramid.len() * 4) as u64,
            usage: BufferUsage::TRANSFER_SRC,
            location: MemoryLocation::HostVisibleCoherent,
        })
        .unwrap_or_else(|e| panic!("[{label}] pyramid staging: {e:?}"));
    // The DIAGNOSTIC sink, allocated ONLY for the `-D VB_CULL_DEBUG_PROBE=1` rig: one record per
    // instance SLOT, so the shader's `g_vb_dbg_slot = g` addressing needs no second convention.
    let debug_words =
        if rig.debug_records { instance_count * VB_CULL_DEBUG_RECORD_WORDS as usize } else { 0 };
    let cull_debug = rig.debug_records.then(|| new_buf((debug_words * 4) as u64, "VbCullDebug"));

    // ---- 2) the uploads, and the POISON that makes non-vacuity assertable ----------------------
    //
    // SAFETY: every buffer above was just created host-coherent at the byte size each write below
    // stays inside, every persistent mapping is live, and NO submission has been made yet — so no
    // GPU work touches any of them.
    unsafe {
        // POISONED, not zeroed: a zero here would be a plausible survivor count, so a dispatch that
        // never ran would agree with any batch whose expected early count happens to be zero.
        let p = map(&indirect, "VbIndirect");
        fill_words(p, batch_count * DRAW_RECORD_STRIDE / 4, READBACK_POISON);
        let p = map(&batch_desc, "VbBatchDesc");
        for (b, batch) in case.batches.iter().enumerate() {
            write_bytes(p, b * 32, &batch.to_bytes());
        }
        let p = map(&cull_visible, "VbCullVisible");
        fill_words(p, batch_count, READBACK_POISON);
        // The atomic bump's counter MUST start at zero; nothing else here does.
        fill_words(map(&cull_count, "VbCullCount"), 1, 0);
        let p = map(&instances, "gVbInstances");
        for (g, inst) in case.instances.iter().enumerate() {
            write_bytes(p, g * INSTANCE_ROW_BYTES, &inst.to_bytes());
        }
        let p = map(&mesh_bounds, "gMeshBounds");
        for (m, b) in case.bounds.iter().enumerate() {
            write_bytes(p, m * MESH_BOUNDS_BYTES, &b.to_bytes());
        }
        // Both survivor lists are poisoned WHOLE. Every slot the shader is expected to write is then
        // asserted non-poison below, so "the dispatch never ran" cannot read as "it wrote a prefix".
        fill_words(map(&visible_instance, "VbVisibleInstance"), instance_count, READBACK_POISON);
        fill_words(map(&late_visible, "VbLateVisible"), instance_count, READBACK_POISON);
        fill_words(map(&late_count, "VbLateCount"), late_count_elems, READBACK_POISON);
        fill_words(map(&indirect_late, "VbIndirectLate"), batch_count * DRAW_RECORD_STRIDE / 4, 0);
        write_bytes(
            map(&cull_uniform, "VbCullUni"),
            0,
            &uniform_bytes(
                &case.view_proj_rows,
                [src_w, src_h],
                [base_w, base_h],
                levels,
                GATE_FRAME_INDEX,
            ),
        );
        let p = map(&pyramid_staging, "pyramid staging");
        let words: Vec<u32> = case.pyramid.iter().map(|v| v.to_bits()).collect();
        for (i, w) in words.iter().enumerate() {
            write_bytes(p, i * 4, &w.to_le_bytes());
        }
        // POISONED WHOLE, like the two survivor lists: a record the leaf never reached must read as
        // "the shader never wrote this" rather than as a plausible `depth_near` of zero.
        if let Some(b) = &cull_debug {
            fill_words(map(b, "VbCullDebug"), debug_words, READBACK_POISON);
        }
    }

    // ---- 3) the pyramid image: a REAL Vulkan mip chain, in GENERAL for life -------------------
    //
    // Vulkan's own mip rule (`max(1, base >> k)`) IS the oracle's `level_extent(k)`, so the image's
    // real extents and the layout agree by construction rather than by a second derivation.
    //
    // `SAMPLED` is what makes the `SampledImageAtGeneral` descriptor write legal; `TRANSFER_DST` is
    // how the oracle's pyramid gets in.
    let pyramid = ctx
        .create_texture(&TextureDesc {
            width: base_w,
            height: base_h,
            depth: 1,
            format: Format::R32Sfloat,
            dimension: TextureDimension::D2,
            usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
            array_layers: 1,
            mip_levels: levels,
            view_format: None,
        })
        .unwrap_or_else(|e| panic!("[{label}] pyramid image ({base_w}x{base_h}, {levels}): {e:?}"));

    // Scoped so every `&buffer` the entry list holds is released before the teardown below moves
    // those buffers — the set itself retains its resources by RAW HANDLE, which is what makes the
    // destruction ORDER (and not the borrow) the thing that keeps this sound.
    let set = {
        let mut entries = vec![
            BindGroupEntry::StorageBuffer { buffer: &indirect },
            BindGroupEntry::StorageBuffer { buffer: &batch_desc },
            BindGroupEntry::StorageBuffer { buffer: &cull_visible },
            BindGroupEntry::StorageBuffer { buffer: &cull_count },
            BindGroupEntry::StorageBuffer { buffer: &instances },
            BindGroupEntry::StorageBuffer { buffer: &mesh_bounds },
            BindGroupEntry::StorageBuffer { buffer: &visible_instance },
            BindGroupEntry::StorageBuffer { buffer: &late_visible },
            BindGroupEntry::StorageBuffer { buffer: &cull_uniform },
            // @9 — the pyramid, recorded at `GENERAL` with a NULL sampler. `SampledImage` would
            // record `SHADER_READ_ONLY_OPTIMAL`, a layout this image is never in.
            BindGroupEntry::SampledImageAtGeneral { texture: &pyramid },
            BindGroupEntry::StorageBuffer { buffer: &indirect_late },
            BindGroupEntry::StorageBuffer { buffer: &late_count },
        ];
        // @12, and ONLY on the diagnostic rig, whose layout is the one binding wider.
        if let Some(b) = &cull_debug {
            entries.push(BindGroupEntry::StorageBuffer { buffer: b });
        }
        ctx.create_bind_group(&BindGroupDesc { layout: &rig.layout, entries: &entries })
            .unwrap_or_else(|e| panic!("[{label}] vb_cull descriptor set: {e:?}"))
    };

    // ---- 4) record -----------------------------------------------------------------------------
    let fence = ctx.create_fence(false).unwrap_or_else(|e| panic!("[{label}] fence: {e:?}"));
    let mut encoder =
        ctx.create_command_encoder().unwrap_or_else(|e| panic!("[{label}] encoder: {e:?}"));
    encoder.begin().unwrap_or_else(|e| panic!("[{label}] encoder begin: {e:?}"));

    let whole_pyramid = ImageSubresourceRange {
        aspect: ImageAspect::COLOR,
        base_mip_level: 0,
        level_count: levels,
        base_array_layer: 0,
        layer_count: 1,
    };
    // UNDEFINED → GENERAL once, then the upload AT GENERAL (a legal transfer destination) and one
    // GENERAL → GENERAL visibility barrier. ONE layout for the image's whole life, which is the
    // property the `SampledImageAtGeneral` descriptor records.
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &pyramid,
        src_stage: BarrierStage::TOP_OF_PIPE,
        dst_stage: BarrierStage::TRANSFER,
        src_access: BarrierAccess::NONE,
        dst_access: BarrierAccess::TRANSFER_WRITE,
        old_layout: ImageLayout::Undefined,
        new_layout: ImageLayout::General,
        range: whole_pyramid,
    });
    let mip_regions: Vec<BufferImageCopy> = (0..levels)
        .map(|level| {
            let [lw, lh] = case.layout.level_extent(level);
            BufferImageCopy {
                buffer_offset: (case.layout.level_offset(level) * 4) as u64,
                buffer_row_length: 0,
                buffer_image_height: 0,
                aspect: ImageAspect::COLOR,
                mip_level: level,
                base_array_layer: 0,
                layer_count: 1,
                image_offset_x: 0,
                image_offset_y: 0,
                image_offset_z: 0,
                image_extent_w: lw,
                image_extent_h: lh,
                image_extent_d: 1,
            }
        })
        .collect();
    encoder.copy_buffer_to_image(&pyramid_staging, &pyramid, ImageLayout::General, &mip_regions);
    encoder.image_barrier(&ImageBarrierDesc {
        texture: &pyramid,
        src_stage: BarrierStage::TRANSFER,
        dst_stage: BarrierStage::COMPUTE_SHADER,
        src_access: BarrierAccess::TRANSFER_WRITE,
        dst_access: BarrierAccess::SHADER_READ,
        old_layout: ImageLayout::General,
        new_layout: ImageLayout::General,
        range: whole_pyramid,
    });

    encoder.bind_compute_pipeline(&rig.pipeline);
    encoder.bind_descriptor_set_compute(&set, &rig.pipeline);
    encoder.push_compute_constants(
        &rig.pipeline,
        ShaderStage::COMPUTE,
        0,
        &push_bytes(
            &case.planes,
            batch_count as u32,
            batch_count as u32,
            VB_CULL_PHASE_EARLY,
            case.occ_flags,
        ),
    );
    encoder.dispatch((batch_count as u32).div_ceil(VB_BATCH_CULL_LOCAL_SIZE_X), 1, 1);
    encoder.end().unwrap_or_else(|e| panic!("[{label}] encoder end: {e:?}"));
    ctx.rhi_queue().submit(&encoder, &fence).unwrap_or_else(|e| panic!("[{label}] submit: {e:?}"));
    ctx.wait_fence(&fence, u64::MAX).unwrap_or_else(|e| panic!("[{label}] wait_fence: {e:?}"));

    // ---- 5) read back --------------------------------------------------------------------------
    //
    // SAFETY: every buffer is host-coherent at the byte size each read stays inside, its persistent
    // mapping is live, and the ONE submission that touched them completed (fence-waited above), so
    // the bytes are stable.
    let (record_words, late_count_words, visible, late_visible_words) = unsafe {
        (
            read_words(map(&indirect, "VbIndirect"), batch_count * DRAW_RECORD_STRIDE / 4),
            read_words(map(&late_count, "VbLateCount"), late_count_elems),
            read_words(map(&visible_instance, "VbVisibleInstance"), instance_count),
            read_words(map(&late_visible, "VbLateVisible"), instance_count),
        )
    };
    let early_count: Vec<u32> = (0..batch_count)
        .map(|b| record_words[(b * DRAW_RECORD_STRIDE + INSTANCE_COUNT_OFFSET) / 4])
        .collect();

    // SAFETY: as for the four reads above — the sink is host-coherent at `debug_words * 4` bytes,
    // its mapping is live, and the one submission that wrote it completed.
    let debug = match &cull_debug {
        Some(b) => unsafe { read_words(map(b, "VbCullDebug"), debug_words) },
        None => Vec::new(),
    };

    let out = Partition {
        early_count,
        late_count: late_count_words[..batch_count].to_vec(),
        visible,
        late_visible: late_visible_words,
        gpu_frame_index: late_count_words[late_count_elems - 1],
        debug,
    };

    // ---- 6) teardown, in reverse acquisition order ---------------------------------------------
    //
    // SAFETY: every object below was created on `ctx` in this function and the one submission that
    // referenced them completed (fence-waited above), so none is GPU-referenced. Each is consumed BY
    // VALUE and so destroyed exactly once. The descriptor set goes before the image it was written
    // with (a set retains its view by raw handle) — THE OWNERSHIP RULE,
    // `VUID-vkDestroyImage-image-01000`.
    unsafe {
        ctx.destroy_command_encoder(encoder);
        ctx.destroy_fence(fence);
        ctx.destroy_bind_group(set);
        ctx.destroy_texture(pyramid);
        if let Some(b) = cull_debug {
            ctx.destroy_buffer(b);
        }
        ctx.destroy_buffer(pyramid_staging);
        ctx.destroy_buffer(late_count);
        ctx.destroy_buffer(indirect_late);
        ctx.destroy_buffer(cull_uniform);
        ctx.destroy_buffer(late_visible);
        ctx.destroy_buffer(visible_instance);
        ctx.destroy_buffer(mesh_bounds);
        ctx.destroy_buffer(instances);
        ctx.destroy_buffer(cull_count);
        ctx.destroy_buffer(cull_visible);
        ctx.destroy_buffer(batch_desc);
        ctx.destroy_buffer(indirect);
    }
    out
}

/// The shader's `VB_CULL_PHASE_EARLY`. Every corpus here dispatches phase 0, because phase 0 is the
/// phase that PARTITIONS — phase 1 re-runs the identical leaf over the phase-0 output, so a leaf
/// disagreement is observable in phase 0 alone and a phase-1 dispatch would add the compaction's
/// own questions to a gate aimed at the verdict.
const VB_CULL_PHASE_EARLY: u32 = 0;

/// The frame index this gate stamps into every uniform. Any value works; a recognisable one makes a
/// failure dump readable.
const GATE_FRAME_INDEX: u32 = 0x00C0_FFEE;

// ==============================================================================================
// The adjudication
// ==============================================================================================

/// Per-`KeepReason` observation counts, plus the rejects. Printed by every corpus so a class that is
/// never reached is VISIBLE rather than assumed.
#[derive(Default, Debug)]
struct VerdictCensus {
    unknown_bounds: usize,
    behind_eye: usize,
    non_finite: usize,
    empty_rect: usize,
    level_unavailable: usize,
    not_occluded: usize,
    reject: usize,
    /// Instances the FRUSTUM dropped before the occlusion test ran.
    frustum_dropped: usize,
    /// Instances the occlusion test never saw because they carry no marker or the split is disarmed.
    untested: usize,
}

impl VerdictCensus {
    fn record(&mut self, fate: Expected, verdict: Option<OcclusionVerdict>) {
        match verdict {
            Some(OcclusionVerdict::Reject) => self.reject += 1,
            Some(OcclusionVerdict::Keep(KeepReason::UnknownBounds)) => self.unknown_bounds += 1,
            Some(OcclusionVerdict::Keep(KeepReason::BehindEye)) => self.behind_eye += 1,
            Some(OcclusionVerdict::Keep(KeepReason::NonFinite)) => self.non_finite += 1,
            Some(OcclusionVerdict::Keep(KeepReason::EmptyRect)) => self.empty_rect += 1,
            Some(OcclusionVerdict::Keep(KeepReason::LevelUnavailable)) => {
                self.level_unavailable += 1;
            }
            Some(OcclusionVerdict::Keep(KeepReason::NotOccluded)) => self.not_occluded += 1,
            None if fate == Expected::Dropped => self.frustum_dropped += 1,
            None => self.untested += 1,
        }
    }
}

/// Compares one case's GPU partition against the host oracle, elementwise and IN ORDER, and folds
/// its verdict classes into `census`.
///
/// # What "elementwise and in order" buys, and why a count comparison would not
///
/// Both lists are region-addressed and written by a single lane in ascending `j`, so their ORDER is
/// a property of the algorithm, not an accident. Comparing lengths alone would pass a cursor bug
/// that wrote the right number of the wrong indices — the exact miss an earlier draft of the
/// engine-level gate shipped.
fn adjudicate(case: &CullCase, got: &Partition, census: &mut VerdictCensus) {
    let label = &case.label;
    for (b, batch) in case.batches.iter().enumerate() {
        let base = batch.base_instance as usize;
        let mut want_early: Vec<u32> = Vec::new();
        let mut want_late: Vec<u32> = Vec::new();
        for j in 0..batch.instance_count {
            let g = batch.base_instance + j;
            let (fate, verdict) = expected_fate(case, g);
            census.record(fate, verdict);
            match fate {
                Expected::Dropped => {}
                Expected::Early => want_early.push(g),
                Expected::Late => want_late.push(g),
            }
        }

        assert_eq!(
            got.early_count[b] as usize,
            want_early.len(),
            "[{label}] batch {b}: the record's instanceCount is {} but the oracle keeps {} of {} \
             instances early. (deferred: gpu {} / oracle {})",
            got.early_count[b],
            want_early.len(),
            batch.instance_count,
            got.late_count[b],
            want_late.len()
        );
        assert_eq!(
            got.late_count[b] as usize,
            want_late.len(),
            "[{label}] batch {b}: VbLateCount is {} but the oracle defers {} of {} instances",
            got.late_count[b],
            want_late.len(),
            batch.instance_count
        );

        let gpu_early = &got.visible[base..base + want_early.len()];
        let gpu_late = &got.late_visible[base..base + want_late.len()];
        for (slot, (&gpu, &want)) in gpu_early.iter().zip(want_early.iter()).enumerate() {
            assert_ne!(
                gpu, READBACK_POISON,
                "[{label}] batch {b} early slot {slot} still holds the readback POISON — the \
                 shader never wrote it, so the count above agreed by accident"
            );
            assert_eq!(
                gpu, want,
                "[{label}] batch {b} early slot {slot}: the GPU kept global instance {gpu}, the \
                 oracle kept {want}. Both lists hold GLOBAL indices in ascending order, so this is \
                 a partition disagreement and not an ordering convention"
            );
        }
        for (slot, (&gpu, &want)) in gpu_late.iter().zip(want_late.iter()).enumerate() {
            assert_ne!(
                gpu, READBACK_POISON,
                "[{label}] batch {b} late slot {slot} still holds the readback POISON"
            );
            assert_eq!(
                gpu, want,
                "[{label}] batch {b} late slot {slot}: the GPU deferred global instance {gpu}, the \
                 oracle defers {want}"
            );
        }

        // THE PARTITION PROPERTY ITSELF, asserted rather than inferred from the two lists agreeing:
        // an early reject must never REMOVE an instance, only move it. `k + n_defer` is exactly the
        // frustum-survivor count.
        let survivors = (0..batch.instance_count)
            .filter(|j| expected_fate(case, batch.base_instance + j).0 != Expected::Dropped)
            .count();
        assert_eq!(
            got.early_count[b] as usize + got.late_count[b] as usize,
            survivors,
            "[{label}] batch {b}: INVARIANT VG-P3-RECOVERY — the early phase must PARTITION the \
             frustum survivors, never remove one. k + n_defer = {} + {} against {survivors} \
             survivors",
            got.early_count[b],
            got.late_count[b]
        );
    }
}

/// The tail-slot control: the shader stamped the frame index it actually read out of the uniform.
///
/// Asserted on every corpus, because it is what proves the module read THIS dispatch's uniform block
/// rather than whatever the allocation happened to hold — and the same instrument the engine-level
/// record-order control depends on.
fn assert_frame_stamp(case: &CullCase, got: &Partition) {
    assert_eq!(
        got.gpu_frame_index, GATE_FRAME_INDEX,
        "[{}] the GPU stamped frame index 0x{:08X} into VbLateCount's reserved tail slot, not the \
         0x{GATE_FRAME_INDEX:08X} this dispatch's uniform carries. Either the uniform was not read, \
         or the tail slot the shader derives from the buffer's own element count is not the slot \
         this gate reads.",
        case.label,
        got.gpu_frame_index
    );
}

// ==============================================================================================
// The SURVEY — corpus 3's own adjudication, which visits every probe before it asserts
// ==============================================================================================

/// The GPU's fate for a case with exactly ONE batch of exactly ONE instance, read off the same two
/// counters [`adjudicate`] reads and cross-checked against the survivor lists.
///
/// Separate from [`adjudicate`] and NOT a relaxation of it: it decides the same partition from the
/// same words, and every structural property that is not the question under study — the readback
/// poison, the impossible "both lists" state, and INVARIANT VG-P3-RECOVERY — still panics on the
/// spot. Only the VERDICT comparison is deferred to the end of the corpus, because that is the one
/// this file is trying to characterise rather than merely detect.
fn single_instance_gpu_fate(case: &CullCase, got: &Partition) -> Expected {
    let label = &case.label;
    debug_assert_eq!(case.instances.len(), 1, "the survey decodes ONE instance");
    let (early, late) = (got.early_count[0], got.late_count[0]);
    // Widened before the sum: an UNWRITTEN counter still holds `READBACK_POISON`, and `u32 + u32`
    // would then be an overflow panic that names arithmetic instead of naming the missing write.
    assert!(
        u64::from(early) + u64::from(late) <= 1,
        "[{label}] the GPU reports {early} early and {late} late survivors for a ONE-instance \
         batch (0x{:08X} is the readback POISON — that value means the shader never wrote the \
         word). INVARIANT VG-P3-RECOVERY is a partition, so the two cannot exceed the batch's own \
         instance count",
        READBACK_POISON
    );
    match (early, late) {
        (1, 0) => {
            assert_ne!(
                got.visible[0], READBACK_POISON,
                "[{label}] the record counts one early survivor but the survivor slot still holds \
                 the readback POISON — the count agreed by accident"
            );
            assert_eq!(got.visible[0], 0, "[{label}] the early slot holds a wrong global index");
            Expected::Early
        }
        (0, 1) => {
            assert_ne!(
                got.late_visible[0], READBACK_POISON,
                "[{label}] the record counts one deferral but the late slot still holds the \
                 readback POISON"
            );
            assert_eq!(got.late_visible[0], 0, "[{label}] the late slot has a wrong global index");
            Expected::Late
        }
        (0, 0) => Expected::Dropped,
        _ => unreachable!("the assertion above bounds `early + late` at 1"),
    }
}

/// Which side of the strict comparison a planted `occ` puts a box on.
///
/// Decided from the ORACLE's OWN per-corner clip pairs (`ScreenRect::corner_zw`) and the SAME
/// product the verdict forms, so the classification is a reading of the shipped predicate rather
/// than a restatement of the plant's intent. The three variants are the three-sided non-vacuity the
/// boundary corpus has to prove.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoundarySide {
    /// Some corner is strictly IN FRONT of `occ` (`cz > occ · cw`): KEEP, with room to spare. An
    /// implementation that got the comparison backwards fails here.
    StrictKeep,
    /// No corner is in front, and at least one meets `occ · cw` EXACTLY. **KEEP, and this is the
    /// only arm that decides `<` against `<=`** — a `<=` implementation REJECTS exactly this probe.
    ExactTie,
    /// Every corner is strictly behind `occ`: REJECT. Without this arm the corpus never observes a
    /// rejection at all.
    StrictReject,
}

/// Classifies a planted `occ` against the eight `(clip.z, clip.w)` pairs the oracle projected.
///
/// The product is spelled `occ * cw` — the operand order `ScreenRect::behind_occluder` and
/// `occlusion_reject`'s step 6 both use. Multiplication is commutative in IEEE-754 including the
/// rounding, so the order is a readability choice here; it is written to match anyway, because the
/// day it stops matching is the day the three files stop being one predicate.
///
/// A `NaN` `occ` — which this corpus never plants — would fall through both comparisons and be
/// reported `StrictReject` while the oracle KEEPs. That mismatch is caught on the spot by the
/// caller's classification assertion rather than being read as a rejection.
fn boundary_side(corner_zw: &[[f32; 2]; 8], occ: f32) -> BoundarySide {
    let mut tie = false;
    for &[cz, cw] in corner_zw {
        let bound = occ * cw;
        if cz > bound {
            return BoundarySide::StrictKeep;
        }
        if cz == bound {
            tie = true;
        }
    }
    if tie { BoundarySide::ExactTie } else { BoundarySide::StrictReject }
}

/// One boundary probe, fully observed: what the host oracle computed, what the shader computed, and
/// what each of them decided.
struct ProbeObservation {
    label: String,
    /// `ScreenRect::depth_near` from `boyko_render::hzb::project_aabb`. ⚠️ REPORTED, not the
    /// verdict's operand — the census measures it, the partition no longer depends on it.
    host_depth_near: f32,
    /// The `occ` this arm actually planted into every texel of every level.
    planted_occ: f32,
    /// Which side of the strict comparison the ORACLE's own numbers put that plant on.
    side: BoundarySide,
    /// The `-D VB_CULL_DEBUG_PROBE=1` module's own record for this instance.
    gpu: DebugRecord,
    host_fate: Expected,
    gpu_fate: Expected,
}

impl ProbeObservation {
    /// `Some(distance)` when both `depth_near`s are comparable numbers, `None` when the shader
    /// exited before folding one (or filled the field with [`VB_DBG_UNSET`]).
    fn depth_near_ulps(&self) -> Option<u64> {
        ulp_distance(self.gpu.depth_near_bits, self.host_depth_near.to_bits())
    }

    /// One line of the survey table. Every float is printed AS BITS beside its value, because the
    /// question is which bit pattern each side arrived at and a decimal rendering of a 1-ULP
    /// difference prints identically on both sides.
    fn line(&self) -> String {
        let dn_gpu = f32::from_bits(self.gpu.depth_near_bits);
        let ulps = match self.depth_near_ulps() {
            Some(n) => format!("{n}"),
            None => "n/a".into(),
        };
        format!(
            "[{}] stage={} side={:?} | host dn=0x{:08X} ({:e}) gpu dn=0x{:08X} ({:e}) ulps={} | \
             planted occ=0x{:08X} gpu occ=0x{:08X} | level={} taps={:?} | host={:?} gpu={:?}{}",
            self.label,
            self.gpu.stage_name(),
            self.side,
            self.host_depth_near.to_bits(),
            self.host_depth_near,
            self.gpu.depth_near_bits,
            dn_gpu,
            ulps,
            self.planted_occ.to_bits(),
            self.gpu.occ_bits,
            self.gpu.level,
            self.gpu.taps,
            self.host_fate,
            self.gpu_fate,
            if self.host_fate == self.gpu_fate { "" } else { "   <<< VERDICTS DISAGREE" }
        )
    }
}

/// The `depth_near` census over a whole corpus of probes — the measurement the boundary corpus
/// existed to make possible and could not make.
///
/// ⚠️ SINCE THE VERDICT WENT DIVISION-FREE THIS CENSUS NO LONGER MEASURES THE DECISION. `depth_near`
/// is the quotient the verdict used to fold, and its 1-ULP spread across 72 probes — in BOTH
/// directions, so a rounding difference and not a bias — is what forced the reformulation: Vulkan
/// permits `OpFDiv` 2.5 ULP at 32-bit while Rust's divide is correctly rounded, so no qualification
/// could have closed it. It is still measured and printed because it is the leaf's most sensitive
/// scalar and therefore the first place a FUTURE drift in the fold shows up — a drift in `cz`/`cw`
/// (which DO decide) would move it too.
#[derive(Default, Debug)]
struct DepthNearCensus {
    /// Probes where both sides produced a comparable number.
    compared: usize,
    /// …of which the two bit patterns are EQUAL.
    identical: usize,
    /// …of which the shader's is BELOW the oracle's. Under the old quotient verdict this was the
    /// GEOMETRY-DELETING direction; it is now a diagnostic of the divide alone.
    gpu_below: usize,
    /// …and above, which under the old verdict cost a wasted draw and nothing else.
    gpu_above: usize,
    /// The largest distance observed, in representable `f32` steps.
    max_ulps: u64,
    /// Probes where the shader exited before folding a `depth_near` at all, so there is nothing to
    /// compare. Counted rather than dropped: a corpus that compared NOTHING must not read as a
    /// corpus that compared everything and agreed.
    incomparable: usize,
}

impl DepthNearCensus {
    fn record(&mut self, o: &ProbeObservation) {
        match o.depth_near_ulps() {
            None => self.incomparable += 1,
            Some(n) => {
                self.compared += 1;
                self.max_ulps = self.max_ulps.max(n);
                let gpu = ordered_key(o.gpu.depth_near_bits);
                let host = ordered_key(o.host_depth_near.to_bits());
                match gpu.cmp(&host) {
                    core::cmp::Ordering::Equal => self.identical += 1,
                    core::cmp::Ordering::Less => self.gpu_below += 1,
                    core::cmp::Ordering::Greater => self.gpu_above += 1,
                }
            }
        }
    }

    /// The SIGN of the divergence in words, which is the half of the census that says whether the
    /// difference can delete geometry.
    fn direction(&self) -> &'static str {
        match (self.gpu_below, self.gpu_above) {
            (0, 0) => "none (every comparable probe is bit-identical)",
            (_, 0) => "the SHADER IS BELOW the oracle — the GEOMETRY-DELETING direction",
            (0, _) => "the shader is ABOVE the oracle — the wasted-draw direction",
            _ => "BOTH directions occur, so it is a rounding difference and not a systematic bias",
        }
    }
}

// ==============================================================================================
// Corpus construction
// ==============================================================================================

/// The DISARMED plane set: every plane `(0,0,0,0)`, so `dist + radius == 0.0` and the `< 0.0`
/// rejection never fires. Mirrors `VbBatchCullPush::DISARMED_PLANES`.
///
/// Used by every corpus but the sentinel one, so that the partition observed is the OCCLUSION
/// decision alone — a frustum reject and an occlusion reject are different fates and mixing them
/// would blunt the message a failure carries.
const DISARMED_PLANES: [Plane; FRUSTUM_PLANE_COUNT] = [[0.0; 4]; FRUSTUM_PLANE_COUNT];

/// One mesh whose LOCAL box is the world box wanted, placed by the identity affine.
fn box_at(centre: [f32; 3], half: [f32; 3]) -> TestBounds {
    TestBounds {
        bmin: [centre[0] - half[0], centre[1] - half[1], centre[2] - half[2]],
        bmax: [centre[0] + half[0], centre[1] + half[1], centre[2] + half[2]],
    }
}

/// Builds a case whose instances are one-per-mesh, identity-affine and all MARKED, split evenly into
/// `batches` batches.
fn case_from_boxes(
    label: String,
    layout: HzbLayout,
    pyramid: Vec<f32>,
    view_proj_rows: [[f32; 4]; 4],
    boxes: Vec<TestBounds>,
    batches: usize,
    occ_flags: u32,
) -> CullCase {
    let n = boxes.len();
    assert_eq!(n % batches, 0, "the corpus must divide evenly into batches");
    let per = (n / batches) as u32;
    let instances: Vec<TestInstance> = (0..n)
        .map(|m| TestInstance::identity(m as u32, VB_INST_FLAG_OCCLUSION_CULLING))
        .collect();
    let batch_descs = (0..batches)
        .map(|b| TestBatch { base_instance: b as u32 * per, instance_count: per })
        .collect();
    CullCase {
        label,
        layout,
        pyramid,
        view_proj_rows,
        planes: DISARMED_PLANES,
        bounds: boxes,
        instances,
        batches: batch_descs,
        occ_flags,
    }
}

/// A spread of view-space boxes that reaches every verdict class the projection can produce.
///
/// Each arm names the class it exists to reach; the census assertion in the random corpus is what
/// says the arms actually fired, because a corpus that never reaches `BehindEye` proves nothing
/// about the `cw <= 0` guard.
fn spread_boxes(rng: &mut Rng, n: usize, near: f32) -> Vec<TestBounds> {
    (0..n)
        .map(|i| match i % 8 {
            // HARD AGAINST THE NEAR PLANE and centred on screen: `depth_near = near / z` is
            // `≈ 0.917`, above [`depth_pattern`]'s maximum of `0.75`, for EVERY `near`. This arm is
            // the `NotOccluded` GUARANTEE — the census assertion below would otherwise depend on a
            // random draw, and a corpus whose coverage depends on luck is a corpus that goes red for
            // the wrong reason.
            0 => box_at([0.0, 0.0, near * 1.1], [near * 0.01, near * 0.01, near * 0.01]),
            // NEAR-ish and on-screen, with the placement randomised: the same class, reached the
            // hard way, so the guaranteed arm above is not the only evidence for it.
            1 => box_at(
                [rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(near * 1.5, near * 3.0)],
                [0.05, 0.05, 0.05],
            ),
            // FAR and on-screen: `depth_near = near / z` is tiny, well under the pattern's floor, so
            // these REJECT. The arm that makes the gate non-vacuous in the deleting direction.
            2 | 3 => box_at(
                [rng.range(-20.0, 20.0), rng.range(-20.0, 20.0), rng.range(200.0, 4000.0)],
                [0.5, 0.5, 0.5],
            ),
            // STRADDLING the eye plane: some corner has `cw <= 0`. The `BehindEye` arm.
            4 => box_at([rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), 0.0], [1.0, 1.0, 1.0]),
            // OFF-SCREEN far to one side, but in front: the rect clamps away. The `EmptyRect` arm.
            5 => box_at([rng.range(400.0, 800.0), 0.0, rng.range(1.0, 4.0)], [0.1, 0.1, 0.1]),
            // A tiny `w` with a large `x`: the post-divide window coordinate overflows to infinity
            // even though the clip coordinates are finite. The `NonFinite` arm, and the reason the
            // oracle checks finiteness TWICE.
            6 => box_at([1.0e30, 1.0e30, 1.0e-30], [1.0e29, 1.0e29, 1.0e-31]),
            // An INFINITE local extent: `lh` is infinite, `wh` folds to a NaN through the `0 · inf`
            // terms of the identity affine, and the world box is neither ordered nor inverted. The
            // `UnknownBounds` arm — reached through the ORACLE's world-space guard, NOT through the
            // outer sentinel guard, which this box does not trip (`bmin < bmax` componentwise).
            _ => TestBounds { bmin: [-f32::INFINITY; 3], bmax: [f32::INFINITY; 3] },
        })
        .collect()
}

// ==============================================================================================
// CORPUS 1 — the oracle's own extents
// ==============================================================================================

/// VG R3 P3-4 GATE G-P3-D, corpus 1: **the extents the oracle's own 26 tests pin.**
///
/// `7×3` (odd on both axes, `prev_pow2` bites on both), `8×16` (`S == P`, so the base map degenerates
/// to the identity and a base-map bug is ABSENT — this row isolates the rest), `1×1` (every rect is
/// a single texel, which is what makes `hzb_msb`'s zero guard unconditional — control D3's home),
/// `511×1023` and `1920×1080` (a non-identity `prev_pow2` mapping on BOTH axes).
///
/// Four of the five are non-power-of-two, which is the configuration Bevy ships its whole occlusion
/// feature as *experimental* for. This engine's `prev_pow2` level 0 makes the source→texel mapping
/// non-identity, and **that mapping is where the field's bugs live**.
///
/// ⚠️ What this corpus CANNOT claim: anything about a level the selector cannot reach. On a COMPLETE
/// chain `level >= levels` is unreachable by construction, so `LevelUnavailable` is the truncated
/// corpus's job, not this one's.
#[test]
#[ignore = "live dispatch gate (GPU + --nocapture --test-threads=1); the orchestrator runs it"]
fn cull_shader_verdict_eq_oracle_on_the_pinned_extents() {
    let Some(ctx) = boot_or_skip("pinned_extents") else {
        return;
    };
    println!("hzb_verdict_oracle_gate (extents) on: {}", ctx.device_name());
    let rig = Rig::new(&ctx);
    let mut census = VerdictCensus::default();

    for (w, h, why) in [
        (7u32, 3u32, "odd on both axes; prev_pow2 bites on both (P = 4x2)"),
        (8, 16, "S == P on both axes: the base map is the identity, isolating the selector"),
        (1, 1, "one texel: every rect is single-texel, so the firstbithigh(0) guard is forced"),
        (511, 1023, "P = 256x512, a long chain with a clamped axis mid-way"),
        (1920, 1080, "the real render extent; S and P differ on BOTH axes"),
    ] {
        let layout = HzbLayout::new(w, h).unwrap_or_else(|e| panic!("{w}x{h} is not legal: {e:?}"));
        let depth = depth_pattern(&layout);
        let mut pyramid = vec![0.0f32; layout.pyramid_len()];
        build_pyramid(&layout, &depth, &mut pyramid);

        let near = 0.1f32;
        let vp = perspective_rows(0.5, w as f32 / h as f32, near);
        let mut rng = Rng::new(0x5EED_0001 ^ (u64::from(w) << 32) ^ u64::from(h));
        let boxes = spread_boxes(&mut rng, 256, near);
        let case = case_from_boxes(
            format!("{w}x{h} ({why})"),
            layout,
            pyramid,
            vp,
            boxes,
            8,
            VB_CULL_OCC_ARMED,
        );
        let got = run_case(&ctx, &rig, &case);
        assert_frame_stamp(&case, &got);
        adjudicate(&case, &got, &mut census);
        println!("  [{}] 256 instances in 8 batches: partition agrees", case.label);
    }

    println!("hzb_verdict_oracle_gate corpus 1 verdict census: {census:?}");
    assert!(
        census.reject > 0 && census.not_occluded > 0,
        "corpus 1 observed {} rejects and {} not-occluded keeps — a corpus that reaches only one \
         side of the verdict cannot distinguish a correct cull from one that answers a constant. \
         {census:?}",
        census.reject,
        census.not_occluded
    );

    // SAFETY: `rig` was created on `ctx` above and every `run_case` fence-waited before returning,
    // so no submission referencing the pipeline, module or layout is in flight.
    unsafe { rig.destroy(&ctx) };
}

// ==============================================================================================
// CORPUS 2 — the random corpus
// ==============================================================================================

/// How many `(matrix, AABB)` pairs the random corpus covers. The matrix varies per CASE (it lives in
/// the uniform block, one per dispatch) and the AABB per instance, so the product is the pair count.
const RANDOM_MATRICES: usize = 128;
/// AABBs per matrix. `128 × 1024 = 131 072`, comfortably past the 100 000 the plan asks for.
const RANDOM_BOXES: usize = 1024;

/// VG R3 P3-4 GATE G-P3-D, corpus 2: **131 072 `(matrix, AABB)` pairs, and every `KeepReason` class
/// OBSERVED at least once.**
///
/// The extent is small (`64 × 48`) on purpose: corpus 1 owns extent variety, and this corpus owns
/// volume. What varies here is the projection — field of view, aspect and near plane — and the boxes,
/// which straddle the near plane, sit wholly behind the eye, run off-screen and overflow the divide.
///
/// ⚠️ The observation counts are PRINTED and the "every class fired" assertion is what makes them
/// load-bearing. A corpus that never reaches `BehindEye` proves nothing about the `cw <= 0` guard,
/// and this campaign has shipped gates that were green because they never reached the case.
///
/// ⚠️ What it CANNOT claim: that these classes are reachable IN THE ENGINE. Several of them are
/// constructed degeneracies — an infinite local extent, a `1e-30` view-space depth — that a real
/// gather would not produce. They are here because the SHADER must agree with the ORACLE on them,
/// not because the engine will meet them.
#[test]
#[ignore = "live dispatch gate (GPU + --nocapture --test-threads=1); the orchestrator runs it"]
fn cull_shader_verdict_eq_oracle_over_the_random_corpus() {
    let Some(ctx) = boot_or_skip("random_corpus") else {
        return;
    };
    println!("hzb_verdict_oracle_gate (random) on: {}", ctx.device_name());
    let rig = Rig::new(&ctx);
    let mut census = VerdictCensus::default();

    let layout = HzbLayout::new(64, 48).expect("invariant: 64x48 is a legal HZB source");
    let depth = depth_pattern(&layout);
    let mut pyramid = vec![0.0f32; layout.pyramid_len()];
    build_pyramid(&layout, &depth, &mut pyramid);

    let mut rng = Rng::new(0x5EED_0002);
    for m in 0..RANDOM_MATRICES {
        let near = rng.range(0.01, 1.0);
        let vp = perspective_rows(rng.range(0.2, 1.6), rng.range(0.5, 2.5), near);
        let boxes = spread_boxes(&mut rng, RANDOM_BOXES, near);
        let case = case_from_boxes(
            format!("random matrix {m}"),
            layout,
            pyramid.clone(),
            vp,
            boxes,
            16,
            VB_CULL_OCC_ARMED,
        );
        let got = run_case(&ctx, &rig, &case);
        assert_frame_stamp(&case, &got);
        adjudicate(&case, &got, &mut census);
    }

    println!(
        "hzb_verdict_oracle_gate corpus 2: {} pairs, verdict census {census:?}",
        RANDOM_MATRICES * RANDOM_BOXES
    );
    // Every class the PROJECTION can produce. `LevelUnavailable` is deliberately absent from this
    // list: on a complete chain the selector cannot reach it, and asserting it here would be a
    // requirement no correct implementation could satisfy. The truncated corpus owns it.
    for (n, what) in [
        (census.reject, "Reject"),
        (census.not_occluded, "Keep(NotOccluded)"),
        (census.behind_eye, "Keep(BehindEye)"),
        (census.non_finite, "Keep(NonFinite)"),
        (census.empty_rect, "Keep(EmptyRect)"),
        (census.unknown_bounds, "Keep(UnknownBounds)"),
    ] {
        assert!(
            n > 0,
            "the random corpus NEVER reached `{what}`, so it says nothing about the guard that \
             produces it. Fix the corpus, not the assertion. {census:?}"
        );
    }

    // SAFETY: as in corpus 1 — every `run_case` fence-waited before returning.
    unsafe { rig.destroy(&ctx) };
}

// ==============================================================================================
// CORPUS 3 — the constructed boundary corpus, and the truncated layout
// ==============================================================================================

/// VG R3 P3-4 GATE G-P3-D, corpus 3: **the EXACT TIE of the reject predicate, and one representable
/// step to either side of it — plus the truncated layout that makes `LevelUnavailable` reachable.**
///
/// This is the only place the `<` versus `<=` difference is decidable, and it is the one difference
/// that DELETES geometry: the soundness chain `occ ≤ D[p] ≤ d_i(p) ≤ depth_near` admits equality, so
/// `<=` would reject a visible instance.
///
/// # THE PLANT IS DERIVED AGAINST THE PREDICATE THAT DECIDES, WHICH IS NO LONGER A QUOTIENT
///
/// The verdict is `∀i: cz_i < occ · cw_i` (see `boyko_render::hzb`'s module header for why the
/// quotient form could not be made to agree with the shader). Its boundary is therefore
/// `cz_i == occ · cw_i`, and planting `occ` at the oracle's `depth_near` — the old construction —
/// would land ONE ROUNDING away from it rather than ON it.
///
/// So the boundary is CONSTRUCTED, exactly, out of the gate's own matrix rather than searched for:
///
/// * `perspective_rows` has `row2 = [0, 0, 0, near]` and `row3 = [0, 0, 1, 0]`, so for every corner
///   the fold `r0·x + r1·y + r2·z + r3` collapses to `cz = near` and `cw = z` — EXACTLY, with no
///   rounding at all, which is also why the 72-probe measurement found the shader's `cz`/`cw`
///   bit-identical to the oracle's while only the quotient moved.
/// * Every instance is the identity affine, so a box whose near face is at `z = near · 2^k` has
///   `cw_min = near · 2^k` exactly — scaling a float by a power of two is exact.
/// * Then `occ = 2^-k` gives `occ · cw_min = near` EXACTLY, again by exponent arithmetic alone.
///   `cz = near`, so `cz < occ · cw_min` is FALSE by an EQUALITY: **the exact tie, and a KEEP**. The
///   far corners have larger `cw`, hence strictly larger products, so nothing else spoils it.
/// * `next_below(2^-k)` scales the product to `near · (1 − 2⁻²⁴)`, which rounds to the float BELOW
///   `near` (the deficit is 0.8 ulp, and `near = 0.1` is not a power of two so the binade does not
///   change): `cz > occ · cw` ⇒ **strictly KEEP**.
/// * `next_above(2^-k)` scales it to `near · (1 + 2⁻²³)` = `near` plus 1.6 ulp, which rounds UP:
///   every corner is strictly behind ⇒ **REJECT**.
///
/// Nothing above depends on a search succeeding, so the three-sided non-vacuity below is a property
/// of the construction and not of a lucky draw. The counters still bind it, because a construction
/// that is right on paper and wrong in the fixture is this campaign's most frequent defect — and
/// each arm is CLASSIFIED by [`boundary_side`] from the ORACLE's own projected pairs, never from the
/// arm's label.
///
/// One instance per case, so two probes can never share a texel and interfere. Each case also plants
/// the SAME value into every level, so the answer does not depend on which level the shader's own
/// selector picked — a deliberate weakening on that one axis, because corpus 1 is what pins the
/// selector and this corpus is aimed at the comparison.
///
/// # THIS CORPUS SURVEYS; IT DOES NOT ABORT ON THE FIRST DISAGREEMENT
///
/// Every other corpus adjudicates through [`adjudicate`], which panics on the first mismatch — the
/// right shape for a corpus whose job is to be green. This one's job is to CHARACTERISE, and a
/// gate that dies on probe 0 of 24 reports a symptom while withholding its frequency, its size and
/// its sign. So it visits every probe, records the row, and asserts afterwards. Nothing is
/// relaxed: the structural properties still panic on the spot
/// ([`single_instance_gpu_fate`]), the non-vacuity counters still bind, and the final assertion
/// still fails while ANY verdict disagreement exists.
///
/// # …and it dispatches BOTH cull modules over every probe
///
/// The verdict comes from the SHIPPING `vb_batch_cull.comp.spv`; the numbers come from the
/// `-D VB_CULL_DEBUG_PROBE=1` variant, which is the only module that declares the `VbCullDebug`
/// sink. Their partitions are asserted EQUAL on every probe, so the diagnostic artifact is a
/// measured proxy rather than an assumed one — the two are one source and one preprocessed fold,
/// but "compiled from the same text" is not "computes the same thing" and this corpus is the place
/// that difference would show.
///
/// ⚠️ What it STILL does not claim: that the shader's `depth_near` is bit-identical to the
/// oracle's. It is MEASURED not to be — 6 of 72 probes apart by 1 ULP, in both directions — and that
/// measurement is the reason the verdict stopped reading it. The distance keeps being printed as a
/// drift detector on the fold, and it is deliberately NOT asserted: asserting bit-identity on a
/// quotient would be asserting that a conforming `OpFDiv` is wrong. What the corpus DECIDES remains
/// the verdict: the two must partition every probe the same way.
#[test]
#[ignore = "live dispatch gate (GPU + --nocapture --test-threads=1); the orchestrator runs it"]
fn cull_shader_verdict_eq_oracle_at_the_strict_boundary() {
    let Some(ctx) = boot_or_skip("boundary") else {
        return;
    };
    println!("hzb_verdict_oracle_gate (boundary) on: {}", ctx.device_name());
    let rig = Rig::new(&ctx);
    let dbg_rig = Rig::debug_probe(&ctx);
    let mut census = VerdictCensus::default();

    let near = 0.1f32;
    let mut planted_tie = 0usize;
    let mut planted_keep = 0usize;
    let mut planted_reject = 0usize;
    let mut observations: Vec<ProbeObservation> = Vec::new();

    for (w, h) in [(64u32, 48u32), (7, 3), (1920, 1080)] {
        let layout = HzbLayout::new(w, h).expect("invariant: a legal HZB source");
        let vp = perspective_rows(0.5, w as f32 / h as f32, near);
        let mut rng = Rng::new(0x5EED_0003 ^ (u64::from(w) << 24));

        for probe in 0..8u32 {
            // The near face at `near · 2^k`, k = 1..=8 — EXACT, and the whole reason `occ = 2^-k`
            // is an exact tie. Built as an explicit `TestBounds` rather than through `box_at`,
            // because `centre − half` would have to be exact too and that is one accident away.
            let k = probe + 1;
            let z_near = near * (1u32 << k) as f32;
            // Everything transverse SCALES with the near face, so the screen footprint is roughly
            // constant in k and lands well inside the frame on all three extents: with
            // `fov_y_tan = 0.5` the widest |ndc| any corner reaches is `2 · 0.3 = 0.6 < 1`.
            let hx = z_near * 0.05;
            let cx = rng.range(-0.25, 0.25) * z_near;
            let cy = rng.range(-0.25, 0.25) * z_near;
            let bx = TestBounds {
                bmin: [cx - hx, cy - hx, z_near],
                bmax: [cx + hx, cy + hx, z_near * 1.25],
            };
            // The oracle's own projection supplies the clip pairs the plant is classified against.
            // If it cannot project this box the probe is not a boundary probe at all, so it is
            // skipped rather than asserted — and the counters below make a corpus that skipped
            // everything impossible to miss.
            let Ok(rect) = project_aabb(&vp, [w, h], bx.bmin, bx.bmax) else {
                continue;
            };
            if select_texels(&layout, &rect).is_err() {
                continue;
            }
            let dn = rect.depth_near;
            // `2^-k`, exact for every k in range, so `occ_tie · (near · 2^k)` is `near` with no
            // rounding whatsoever — the equality the strict `<` has to keep.
            let occ_tie = 1.0f32 / (1u32 << k) as f32;
            for (which, occ) in [
                ("exact tie", occ_tie),
                ("one ULP below", next_below(occ_tie)),
                ("one ULP above", next_above(occ_tie)),
            ] {
                // A FLAT pyramid at `occ`: every texel of every level carries it, so the verdict
                // cannot depend on which level the shader selected.
                let pyramid = vec![occ; layout.pyramid_len()];
                let case = CullCase {
                    label: format!("{w}x{h} boundary probe {probe} ({which})"),
                    layout,
                    pyramid,
                    view_proj_rows: vp,
                    planes: DISARMED_PLANES,
                    bounds: vec![bx],
                    instances: vec![TestInstance::identity(0, VB_INST_FLAG_OCCLUSION_CULLING)],
                    batches: vec![TestBatch { base_instance: 0, instance_count: 1 }],
                    occ_flags: VB_CULL_OCC_ARMED,
                };
                let want = occlusion_verdict(&case.layout, &case.pyramid, &vp, bx.bmin, bx.bmax);
                // WHICH SIDE the plant actually landed on, read off the ORACLE's own projected
                // pairs with the verdict's own product — never off the arm's label, because the
                // label is the intent and the classification has to be the fact.
                let side = boundary_side(&rect.corner_zw, occ);
                match want {
                    OcclusionVerdict::Reject => planted_reject += 1,
                    OcclusionVerdict::Keep(KeepReason::NotOccluded) => match side {
                        BoundarySide::ExactTie => planted_tie += 1,
                        BoundarySide::StrictKeep => planted_keep += 1,
                        // Unreachable: `StrictReject` is exactly the oracle's `Reject`. Asserted
                        // below rather than assumed here.
                        BoundarySide::StrictReject => {}
                    },
                    // The plant landed somewhere the projection guards catch first; not a boundary
                    // probe. Counted by the census, not by the three totals below.
                    OcclusionVerdict::Keep(_) => {}
                }
                // The classification and the ORACLE must agree, or one of them is reading a
                // different `occ` than the other — which is precisely what a non-flat pyramid, a
                // wrong `pyramid_len` or a NaN policy change would look like. `occluder_depth` over
                // a pyramid filled with one value must return that value.
                let reached_verdict = matches!(
                    want,
                    OcclusionVerdict::Reject | OcclusionVerdict::Keep(KeepReason::NotOccluded)
                );
                if reached_verdict {
                    assert_eq!(
                        side == BoundarySide::StrictReject,
                        want == OcclusionVerdict::Reject,
                        "[{}] the plant classifies as {side:?} against the oracle's own corner \
                         pairs, but `occlusion_verdict` answered {want:?}. The two read the SAME \
                         predicate over the SAME numbers, so they can only disagree if the `occ` \
                         that reached the verdict is not the {occ:e} this arm planted into every \
                         texel",
                        case.label
                    );
                }
                // The SHIPPING module decides the verdict…
                let got = run_case(&ctx, &rig, &case);
                assert_frame_stamp(&case, &got);
                // …and the diagnostic variant supplies the intermediates, over the SAME inputs.
                let dbg = run_case(&ctx, &dbg_rig, &case);
                assert_frame_stamp(&case, &dbg);
                assert_eq!(
                    (dbg.early_count[0], dbg.late_count[0]),
                    (got.early_count[0], got.late_count[0]),
                    "[{}] the `-D VB_CULL_DEBUG_PROBE=1` module partitioned this probe DIFFERENTLY \
                     from the shipping one (debug {}/{} vs shipping {}/{} early/late). The two are \
                     one source and one preprocessed fold, so the diagnostic artifact has drifted \
                     and every number it reports below is about a module that is not the one \
                     shipping — fix that before reading any of them.",
                    case.label,
                    dbg.early_count[0],
                    dbg.late_count[0],
                    got.early_count[0],
                    got.late_count[0]
                );

                let (host_fate, verdict) = expected_fate(&case, 0);
                census.record(host_fate, verdict);
                let gpu_fate = single_instance_gpu_fate(&case, &got);
                let rec = DebugRecord::decode(&dbg.debug, 0);
                // NON-VACUITY OF THE SINK ITSELF: a record still holding the poison means the leaf
                // never reached ANY of its exits for this instance, which cannot happen for an
                // ARMED, MARKED, known-bounds instance — so it would mean the sink is not wired,
                // and every "agreement" below would be an agreement between two silences.
                assert_ne!(
                    rec.stage, READBACK_POISON,
                    "[{}] the VbCullDebug record still holds the readback POISON: the diagnostic \
                     module never wrote it. The sink is not reaching the artifact, or @12 is not \
                     the binding this gate wrote. {rec:?}",
                    case.label
                );
                assert!(
                    (VB_DBG_STAGE_UNORDERED_BOX..=VB_DBG_STAGE_VERDICT).contains(&rec.stage),
                    "[{}] the VbCullDebug record's stage word is {}, which is not one of the \
                     leaf's seven exits — the host mirror of the record layout and the shader's \
                     have drifted. {rec:?}",
                    case.label,
                    rec.stage
                );
                // A record that says VERDICT must be COMPLETE. `occ` is read out of the pyramid,
                // which this corpus fills with finite positive depths, so the `VB_DBG_UNSET` filler
                // is not a value it can legitimately hold — an `occ` still reading as the filler
                // would mean the two record layouts have drifted by a word, which shifts every
                // field and would otherwise be read as a wrong VALUE rather than a wrong OFFSET.
                if rec.stage == VB_DBG_STAGE_VERDICT {
                    assert_ne!(
                        rec.occ_bits, VB_DBG_UNSET,
                        "[{}] the VbCullDebug record reached the verdict but its `occ` word is the \
                         UNSET filler. {rec:?}",
                        case.label
                    );
                }

                observations.push(ProbeObservation {
                    label: case.label.clone(),
                    host_depth_near: dn,
                    planted_occ: occ,
                    side,
                    gpu: rec,
                    host_fate,
                    gpu_fate,
                });
            }
        }
    }

    // THE SURVEY, printed BEFORE any assertion below can abort — so a red run publishes the whole
    // table and not just the row that happened to fail first.
    println!("--- hzb_verdict_oracle_gate corpus 3: {} boundary probes ---", observations.len());
    for o in &observations {
        println!("  {}", o.line());
    }
    let mut dn_census = DepthNearCensus::default();
    for o in &observations {
        dn_census.record(o);
    }
    let diverged: Vec<&ProbeObservation> =
        observations.iter().filter(|o| o.host_fate != o.gpu_fate).collect();
    println!(
        "  depth_near census: {dn_census:?}\n  direction: {}\n  verdict disagreements: {} of {}",
        dn_census.direction(),
        diverged.len(),
        observations.len()
    );

    println!(
        "hzb_verdict_oracle_gate corpus 3: {planted_tie} EXACT-TIE KEEP probes, {planted_keep} \
         strict KEEP probes, {planted_reject} strict REJECT probes, census {census:?}"
    );
    // NON-VACUITY, THREE-SIDED. Without these the corpus could have skipped every probe (or planted
    // only rejects) and still reported success — the exact shape of the vacuous gate this campaign
    // keeps catching. Each side is counted from [`boundary_side`]'s reading of the ORACLE's own
    // corner pairs, so "the arm was labelled `exact tie`" is never what satisfies it.
    assert!(
        planted_tie > 0,
        "no probe landed on an EXACT TIE (`cz == occ · cw` with no corner in front of it), so the \
         strictness of `<` was never exercised and a `<=` implementation would pass this corpus. \
         The tie is CONSTRUCTED — near face at `near · 2^k`, `occ = 2^-k`, both exact — so zero \
         here means the construction no longer produces what it derives, not that the draw was \
         unlucky"
    );
    assert!(
        planted_keep > 0,
        "no probe landed STRICTLY on the keep side (some corner in front of `occ`), so an \
         implementation that got the comparison backwards — rejecting whenever anything is in \
         front — would pass this corpus on the tie arm alone"
    );
    assert!(
        planted_reject > 0,
        "no probe landed on the `∀i: cz < occ · cw ⇒ REJECT` side, so the corpus never observed a \
         rejection at all"
    );
    // …and of the SURVEY, which would otherwise report a clean census over zero rows.
    assert!(
        dn_census.compared > 0,
        "not one probe produced a comparable `depth_near` on both sides ({dn_census:?}), so the \
         census below is a statement about nothing. Either every probe exited the leaf early or \
         the sink is recording the wrong field"
    );

    // THE DECISION, deferred to here so that the census above is complete when it fires.
    assert!(
        diverged.is_empty(),
        "THE SHADER AND THE ORACLE PARTITION {} OF {} BOUNDARY PROBES DIFFERENTLY.\n  \
         depth_near: {} of {} comparable probes are bit-identical, {} of them differ; the largest \
         distance is {} ULP and the direction is {}.\n  The diverging probes:\n{}\n  \
         Read this together with the table above. The verdict is `∀i: cz_i < occ · cw_i` — one \
         correctly-rounded multiply over operands both sides fold identically — and every arm \
         plants `occ` AT the exact tie or one representable step from it. So a disagreement here \
         is NOT the 1-ULP quotient difference the `depth_near` census reports (that quantity no \
         longer decides): it is a disagreement about `cz`, `cw`, `occ` or the comparison itself. \
         Check the `side` column against each row's fate before suspecting the rounding.",
        diverged.len(),
        observations.len(),
        dn_census.identical,
        dn_census.compared,
        dn_census.compared - dn_census.identical,
        dn_census.max_ulps,
        dn_census.direction(),
        diverged.iter().map(|o| format!("    {}", o.line())).collect::<Vec<_>>().join("\n")
    );

    // === The TRUNCATED layout, which is the only way `LevelUnavailable` is reachable. ===
    //
    // On a COMPLETE chain `levels = msb(max(base)) + 1` while the selector can only ask for
    // `msb(base - 1) = log2(base) - 1`, so `level >= levels` never fires. A truncated pyramid — the
    // shape `HzbLayout::truncated` exists for — makes it fire, and the escape must be KEEP, NEVER a
    // clamp down to `levels - 1`: a finer level samples a strict SUBSET of the rect's footprint, so
    // `occ` could only come out too large and reject a VISIBLE instance. Control D2 is exactly that
    // clamp.
    let trunc = HzbLayout::truncated(256, 256, 2).expect("invariant: a 2-level 256x256 chain");
    let depth = depth_pattern(&trunc);
    let mut pyramid = vec![0.0f32; trunc.pyramid_len()];
    build_pyramid(&trunc, &depth, &mut pyramid);
    let vp = perspective_rows(0.5, 1.0, near);
    let mut rng = Rng::new(0x5EED_0004);
    // BIG boxes, so the screen rect spans many texels and the selector asks for a coarse level the
    // two-level chain does not have.
    let boxes: Vec<TestBounds> = (0..64)
        .map(|_| {
            box_at([rng.range(-0.5, 0.5), rng.range(-0.5, 0.5), rng.range(2.0, 6.0)], [1.5, 1.5, 0.2])
        })
        .collect();
    let case = case_from_boxes(
        "256x256 TRUNCATED to 2 levels".into(),
        trunc,
        pyramid,
        vp,
        boxes,
        8,
        VB_CULL_OCC_ARMED,
    );
    let mut trunc_census = VerdictCensus::default();
    let got = run_case(&ctx, &rig, &case);
    assert_frame_stamp(&case, &got);
    adjudicate(&case, &got, &mut trunc_census);
    println!("hzb_verdict_oracle_gate corpus 3 (truncated): {trunc_census:?}");
    assert!(
        trunc_census.level_unavailable > 0,
        "the truncated case never reached `Keep(LevelUnavailable)`, so the one escape that must NOT \
         clamp down was never exercised. {trunc_census:?}"
    );

    // SAFETY: as in corpus 1 — every `run_case` fence-waited before returning. BOTH rigs were
    // created on `ctx` by `Rig::build` and each is consumed by value, so each is destroyed once.
    unsafe {
        dbg_rig.destroy(&ctx);
        rig.destroy(&ctx);
    }
}

/// The next representable `f32` above `v`, by bit pattern. `v` is finite and positive here.
fn next_above(v: f32) -> f32 {
    if v.is_nan() || v == f32::INFINITY {
        return v;
    }
    if v == 0.0 {
        return f32::from_bits(1);
    }
    if v > 0.0 { f32::from_bits(v.to_bits() + 1) } else { f32::from_bits(v.to_bits() - 1) }
}

/// The next representable `f32` below `v`, by bit pattern.
fn next_below(v: f32) -> f32 {
    if v.is_nan() || v == f32::NEG_INFINITY {
        return v;
    }
    if v == 0.0 {
        return f32::from_bits(0x8000_0001);
    }
    if v > 0.0 { f32::from_bits(v.to_bits() - 1) } else { f32::from_bits(v.to_bits() + 1) }
}

// ==============================================================================================
// CORPUS 4 — the sentinel corpus
// ==============================================================================================

/// VG R3 P3-4 GATE G-P3-D, corpus 4: **`MeshLocalBounds::UNKNOWN` must land in the EARLY survivor
/// list — never deferred, never dropped — under BOTH a normal affine and an exactly-zero one.**
///
/// This is the corpus that gates the ORDER of the sentinel guard, and it is the only one that runs
/// with REAL frustum planes, because one of the two failure modes is a FRUSTUM deletion:
///
/// * **sentinel + a normal affine.** `MeshLocalBounds::UNKNOWN` is `min = +1e30, max = -1e30`, so
///   `lh = -1e30`; folded through a normal affine the world box is INVERTED, giving
///   `radius = dot(|n|, h)` large NEGATIVE, so `dist + radius < 0` on the very FIRST plane and the
///   instance is frustum-REJECTED. It would land in NEITHER list — and VG-P3-RECOVERY does not
///   cover it, because recovery is about the deferred set.
/// * **sentinel + an exactly-zero linear part** (`Transform::from_scale(Vec3::ZERO)` is an unguarded
///   public `const fn`). Every `wh[r] = dot(|row.xyz|, lh)` is `0 · -1e30 = 0`, so `mn == mx == wc`:
///   a world-space `any(mn > mx)` is FALSE, the collapsed POINT survives the frustum, and it is
///   trivially occluded by anything. It would be DEFERRED, and then deleted by the late phase.
///
/// Both are the geometry-deleting direction, and both are invisible to any image gate on a corpus
/// where no mesh carries the sentinel. Control D4 — hoist the guard after the Arvo fold — must turn
/// this corpus red, one failure mode per instance.
///
/// ⚠️ Reachability, stated because "invisible today" is not "impossible":
/// `MeshGeometryTable`'s own doc says *"A slot that is never registered keeps the
/// `MeshLocalBounds::UNKNOWN` prefill"*, and `VB_GEOMETRY_RESERVED_SLOT` is exactly such a slot. What
/// keeps the sentinel rare on the committed corpus is that the gather EXCLUDES non-resolvable meshes
/// entirely — a mesh that IS registered and never received a VB geometry slot resolves to the
/// reserved slot and DOES reach the ring, carrying the prefill.
///
/// ⚠️ What it CANNOT claim: that the engine's gather produces such a row today. It uploads its own
/// bounds; the reachability argument above is a claim about the shipped code, gated by nothing.
#[test]
#[ignore = "live dispatch gate (GPU + --nocapture --test-threads=1); the orchestrator runs it"]
fn the_unknown_bounds_sentinel_is_drawn_early_under_every_affine() {
    let Some(ctx) = boot_or_skip("sentinel") else {
        return;
    };
    println!("hzb_verdict_oracle_gate (sentinel) on: {}", ctx.device_name());
    let rig = Rig::new(&ctx);

    let (w, h) = (256u32, 256u32);
    let near = 0.1f32;
    let layout = HzbLayout::new(w, h).expect("invariant: 256x256 is a legal HZB source");
    // A FLAT pyramid at `SENTINEL_OCC`, chosen so all three depths this fixture uses land on the
    // side the corpus needs, with `z_ndc = near / z` under this projection:
    //
    //   * the near box at `z ≈ 1.7`  ⇒ `depth_near ≈ 0.0588 > 0.05` ⇒ **KEEP**  (non-vacuity: the
    //     partition does not defer everything);
    //   * the far box at `z ≈ 39.5`  ⇒ `depth_near ≈ 0.0025 < 0.05` ⇒ **REJECT** (non-vacuity: it
    //     defers something);
    //   * the COLLAPSED POINT the zero-affine sentinel would produce, at the instance's translation
    //     `z = 3` ⇒ `depth_near ≈ 0.0333 < 0.05` ⇒ **REJECT** — which is what makes control D4's
    //     failure mode (b) an occlusion DELETION rather than a coincidence that happens to survive.
    const SENTINEL_OCC: f32 = 0.05;
    let pyramid = vec![SENTINEL_OCC; layout.pyramid_len()];
    let vp = perspective_rows(0.5, 1.0, near);
    let planes = frustum_planes_from_view_proj(&vp);

    // Three meshes: the sentinel (shared by the two sentinel instances), one box the pyramid does
    // NOT occlude and one it does. The last two are the corpus's own non-vacuity — without them a
    // shader that deferred NOTHING at all would satisfy the sentinel clause trivially.
    let bounds = vec![
        TestBounds::UNKNOWN,
        box_at([0.0, 0.0, 2.0], [0.3, 0.3, 0.3]),
        box_at([0.0, 0.0, 40.0], [0.5, 0.5, 0.5]),
    ];
    let instances = vec![
        // (a) the sentinel under a NORMAL affine — a rotation-free scale-and-translate.
        TestInstance {
            rows: [[2.0, 0.0, 0.0, 1.0], [0.0, 3.0, 0.0, -2.0], [0.0, 0.0, 1.5, 8.0]],
            mesh_id: 0,
            flags: VB_INST_FLAG_OCCLUSION_CULLING,
        },
        // (b) the sentinel under an EXACTLY ZERO linear part.
        TestInstance {
            rows: [[0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 3.0]],
            mesh_id: 0,
            flags: VB_INST_FLAG_OCCLUSION_CULLING,
        },
        // (c) a real box the pyramid does NOT occlude, and (d) one it does.
        TestInstance::identity(1, VB_INST_FLAG_OCCLUSION_CULLING),
        TestInstance::identity(2, VB_INST_FLAG_OCCLUSION_CULLING),
    ];
    // The FIXTURE PRECONDITION for control D4's failure mode (b), asserted rather than assumed: the
    // point at instance (b)'s translation must actually be occluded by this pyramid. If it were not,
    // hoisting the sentinel guard would leave (b) in the early list and the control would report a
    // false GREEN on the half of B4 it exists to prove.
    {
        let t = TestBounds { bmin: [0.0, 0.0, 3.0], bmax: [0.0, 0.0, 3.0] };
        assert_eq!(
            occlusion_verdict(&layout, &pyramid, &vp, t.bmin, t.bmax),
            OcclusionVerdict::Reject,
            "the collapsed point a zero-linear-part sentinel folds to is NOT occluded by this \
             fixture's pyramid, so control D4's failure mode (b) could not fire here"
        );
    }
    let case = CullCase {
        label: "sentinel corpus (REAL planes)".into(),
        layout,
        pyramid,
        view_proj_rows: vp,
        planes,
        bounds,
        instances,
        batches: vec![TestBatch { base_instance: 0, instance_count: 4 }],
        occ_flags: VB_CULL_OCC_ARMED,
    };

    // The oracle's own answer for the two sentinel instances, asserted on the HOST first: if the
    // fixture did not put them in the EARLY list by construction, the GPU comparison below would be
    // asserting nothing.
    for g in 0..2u32 {
        let (fate, verdict) = expected_fate(&case, g);
        assert_eq!(
            fate,
            Expected::Early,
            "the FIXTURE is wrong: sentinel instance {g} is not EARLY on the host oracle either \
             ({verdict:?}), so the GPU comparison would prove nothing"
        );
    }
    // …and the non-vacuity of the corpus: instance 3 must actually be deferred, or "the sentinel was
    // not deferred" is satisfied by a shader that defers nothing.
    assert_eq!(
        expected_fate(&case, 3).0,
        Expected::Late,
        "the FIXTURE is wrong: the occluded box is not deferred by the oracle, so this corpus \
         cannot tell a working partition from one that never defers"
    );

    let got = run_case(&ctx, &rig, &case);
    assert_frame_stamp(&case, &got);
    let mut census = VerdictCensus::default();
    adjudicate(&case, &got, &mut census);

    // Stated AGAIN, directly and by name, so a failure reads as "the sentinel was deleted" rather
    // than as one differing slot inside the general adjudication.
    let early = &got.visible[..got.early_count[0] as usize];
    for g in 0..2u32 {
        assert!(
            early.contains(&g),
            "SENTINEL INSTANCE {g} IS NOT IN THE EARLY SURVIVOR LIST. early={early:?} \
             late={:?} — an UNKNOWN-bounds instance must be DRAWN, never frustum-tested and never \
             occlusion-tested. Absence of bounds is not evidence of invisibility, and the order of \
             the guard is what makes that true.",
            &got.late_visible[..got.late_count[0] as usize]
        );
    }
    println!("hzb_verdict_oracle_gate corpus 4: sentinel drawn early under both affines; {census:?}");

    // SAFETY: as in corpus 1 — `run_case` fence-waited before returning.
    unsafe { rig.destroy(&ctx) };
}

// ==============================================================================================
// The HOST-side controls — no GPU, so they cannot skip
// ==============================================================================================

/// FIXTURE CONTROL for [`expected_fate`] and [`TestBounds::is_sentinel`], and it is not decorative.
///
/// Every GPU corpus above compares against this file's own oracle composition. If that composition
/// were wrong in the SAME direction as a shader bug, the gate would certify the bug. These arms fix
/// the composition's behaviour on cases whose answers are known without a device.
///
/// Runs unconditionally — no GPU, so it cannot SKIP the way the corpora do.
#[test]
fn the_host_oracle_composition_answers_the_known_cases() {
    let layout = HzbLayout::new(64, 48).expect("invariant: 64x48 is a legal HZB source");
    let vp = perspective_rows(0.5, 64.0 / 48.0, 0.1);

    // The sentinel is caught by the OUTER guard, `any(bmin > bmax)` — component-wise, and NOT by
    // `!(bmin <= bmax)`. The distinction matters: a NaN coordinate is neither, and the shader spells
    // both guards in different places for that reason.
    assert!(TestBounds::UNKNOWN.is_sentinel(), "the UNKNOWN prefill must read as the sentinel");
    assert!(
        !TestBounds { bmin: [0.0; 3], bmax: [1.0; 3] }.is_sentinel(),
        "an ordered box must not read as the sentinel"
    );
    assert!(
        !TestBounds { bmin: [f32::NAN; 3], bmax: [1.0; 3] }.is_sentinel(),
        "a NaN box must NOT trip the outer guard — `NaN > x` is false. It is the ORACLE's own \
         world-space `!(min <= max)` that catches it, one guard later, and conflating the two is \
         what an earlier draft of this rung got wrong"
    );

    // A sentinel instance is EARLY whatever the planes and whatever the pyramid say.
    let case = CullCase {
        label: "host control".into(),
        layout,
        // An all-near pyramid: the strongest occluder there is, so "EARLY" cannot be an accident of
        // the depth values.
        pyramid: vec![1.0f32; layout.pyramid_len()],
        view_proj_rows: vp,
        planes: frustum_planes_from_view_proj(&vp),
        bounds: vec![TestBounds::UNKNOWN, box_at([0.0, 0.0, 40.0], [0.5, 0.5, 0.5])],
        instances: vec![
            TestInstance::identity(0, VB_INST_FLAG_OCCLUSION_CULLING),
            TestInstance::identity(1, VB_INST_FLAG_OCCLUSION_CULLING),
            // The SAME box as instance 1, but UNMARKED: the capability is structural, so its absence
            // must be a skip rather than a runtime `false`.
            TestInstance::identity(1, 0),
        ],
        batches: vec![TestBatch { base_instance: 0, instance_count: 3 }],
        occ_flags: VB_CULL_OCC_ARMED,
    };
    assert_eq!(expected_fate(&case, 0).0, Expected::Early, "the sentinel must be drawn early");
    assert_eq!(expected_fate(&case, 1).0, Expected::Late, "a fully occluded MARKED box defers");
    assert_eq!(
        expected_fate(&case, 2).0,
        Expected::Early,
        "an UNMARKED box is never occlusion-tested, however occluded it is"
    );

    // FORCE_KEEP defers nothing; FORCE_LATE defers every marked survivor. Both are controls the
    // engine arms through the same word, and getting either backwards would invert a gate.
    let mut forced = CullCase { occ_flags: VB_CULL_OCC_ARMED | VB_CULL_OCC_FORCE_KEEP, ..case };
    assert_eq!(expected_fate(&forced, 1).0, Expected::Early, "FORCE_KEEP must defer NOTHING");
    forced.occ_flags = VB_CULL_OCC_ARMED | VB_CULL_OCC_FORCE_LATE;
    assert_eq!(
        expected_fate(&forced, 1).0,
        Expected::Late,
        "FORCE_LATE must defer every MARKED survivor whatever the verdict says"
    );
    assert_eq!(
        expected_fate(&forced, 2).0,
        Expected::Early,
        "FORCE_LATE must still not touch an UNMARKED instance"
    );
    forced.occ_flags = 0;
    assert_eq!(
        expected_fate(&forced, 1).0,
        Expected::Early,
        "DISARMED must defer nothing at all — the partition degrades to the pre-P3-4 loop exactly"
    );
}

/// FIXTURE CONTROL for [`depth_pattern`]'s band, which two corpus assertions silently depend on.
///
/// `spread_boxes`' arm 0 is the GUARANTEE that `Keep(NotOccluded)` is observed, and the guarantee is
/// arithmetic, not luck: that box's `depth_near` is `≈ 0.917` for every `near`, which beats the
/// pattern only while the pattern's ceiling stays at `0.75`. Widen the band and the corpora go red
/// for a reason that has nothing to do with the shader — so the band is pinned here, on the HOST,
/// where the failure names itself.
///
/// Runs unconditionally — no GPU, so it cannot SKIP.
#[test]
fn the_depth_pattern_stays_inside_the_band_the_corpora_assume() {
    for (w, h) in [(64u32, 48u32), (7, 3), (1, 1), (1920, 1080)] {
        let layout = HzbLayout::new(w, h).expect("invariant: a legal HZB source");
        let depth = depth_pattern(&layout);
        assert_eq!(depth.len(), layout.source_len(), "[{w}x{h}] the pattern is the wrong length");
        let lo = depth.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = depth.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(
            lo > 0.25 && hi <= 0.75,
            "[{w}x{h}] the depth pattern spans [{lo}, {hi}], outside the (0.25, 0.75] band. \
             `spread_boxes`' guaranteed NotOccluded arm needs the 0.75 ceiling and the Reject arms \
             need the 0.25 floor; a widened band makes the corpora red for a reason unrelated to \
             the shader"
        );
    }
    // …and the pattern must actually VARY, or `build_pyramid` would fold a constant and a
    // wrong-footprint bug would be invisible.
    let layout = HzbLayout::new(64, 48).expect("invariant: 64x48 is a legal HZB source");
    let depth = depth_pattern(&layout);
    let distinct = depth.iter().map(|v| v.to_bits()).collect::<std::collections::BTreeSet<_>>();
    assert!(
        distinct.len() > depth.len() / 2,
        "the depth pattern has only {} distinct values over {} pixels — two different footprints \
         would then share a minimum by accident and a wrong-footprint bug would read as agreement",
        distinct.len(),
        depth.len()
    );
}

/// FIXTURE CONTROL for the two wire serializers, on the ONE property a reviewer cannot check by
/// reading them: that the byte offsets match the host's own size constants.
///
/// A push block or uniform one field short is not a compile error anywhere — it is a shader reading
/// whatever follows in the range, silently.
#[test]
fn the_wire_blocks_match_the_host_size_constants() {
    let push = push_bytes(&DISARMED_PLANES, 3, 4, VB_CULL_PHASE_EARLY, VB_CULL_OCC_ARMED);
    assert_eq!(
        push.len(),
        VB_BATCH_CULL_PUSH_BYTES as usize,
        "the hand-serialized push block is not `VB_BATCH_CULL_PUSH_BYTES` long"
    );
    // The four scalars live past the six planes, at 96/100/104/108. Read them back to fix the
    // offsets, because a transposed pair here would swap `phase` with `occ_flags` and turn every
    // early dispatch into a late one.
    assert_eq!(u32::from_le_bytes(push[96..100].try_into().unwrap()), 3, "batch_count @96");
    assert_eq!(u32::from_le_bytes(push[100..104].try_into().unwrap()), 4, "visible_cap @100");
    assert_eq!(u32::from_le_bytes(push[104..108].try_into().unwrap()), 0, "phase @104");
    assert_eq!(
        u32::from_le_bytes(push[108..112].try_into().unwrap()),
        VB_CULL_OCC_ARMED,
        "occ_flags @108"
    );

    let rows = [[1.0, 2.0, 3.0, 4.0], [5.0; 4], [6.0; 4], [7.0; 4]];
    let uni = uniform_bytes(&rows, [11, 22], [8, 16], 5, GATE_FRAME_INDEX);
    assert_eq!(
        uni.len(),
        VB_CULL_UNIFORM_BYTES as usize,
        "the hand-serialized uniform is not `VB_CULL_UNIFORM_BYTES` long"
    );
    // Row-major, `pv[row][col]`: element `(0,1)` is at byte 4, NOT at byte 16. A transposed matrix
    // still projects — to a systematically wrong rect on every instance, with every guard silent.
    assert_eq!(f32::from_le_bytes(uni[4..8].try_into().unwrap()), 2.0, "view_proj_rows[0][1] @4");
    assert_eq!(f32::from_le_bytes(uni[16..20].try_into().unwrap()), 5.0, "view_proj_rows[1][0] @16");
    assert_eq!(u32::from_le_bytes(uni[64..68].try_into().unwrap()), 11, "src_extent.x @64");
    assert_eq!(u32::from_le_bytes(uni[72..76].try_into().unwrap()), 8, "base_extent.x @72");
    assert_eq!(u32::from_le_bytes(uni[80..84].try_into().unwrap()), 5, "levels @80");
    assert_eq!(
        u32::from_le_bytes(uni[84..88].try_into().unwrap()),
        GATE_FRAME_INDEX,
        "frame_index @84"
    );

    // The instance row's two named lanes sit in the 16-byte tail beside `mesh_id`; P3-4 is the first
    // code to read `flags`, so a wrong offset here would read the affine's last row as a flag word.
    let row = TestInstance::identity(0x1234, VB_INST_FLAG_OCCLUSION_CULLING).to_bytes();
    assert_eq!(u32::from_le_bytes(row[48..52].try_into().unwrap()), 0x1234, "mesh_id @48");
    assert_eq!(
        u32::from_le_bytes(row[52..56].try_into().unwrap()),
        VB_INST_FLAG_OCCLUSION_CULLING,
        "flags @52"
    );
    assert_eq!(f32::from_le_bytes(row[0..4].try_into().unwrap()), 1.0, "affine row0.x @0");
    assert_eq!(f32::from_le_bytes(row[20..24].try_into().unwrap()), 1.0, "affine row1.y @20");

    let b = TestBounds { bmin: [-1.0, -2.0, -3.0], bmax: [4.0, 5.0, 6.0] }.to_bytes();
    assert_eq!(f32::from_le_bytes(b[0..4].try_into().unwrap()), -1.0, "bmin.x @0");
    assert_eq!(f32::from_le_bytes(b[16..20].try_into().unwrap()), 4.0, "bmax.x @16");

    let d = TestBatch { base_instance: 7, instance_count: 9 }.to_bytes();
    assert_eq!(u32::from_le_bytes(d[12..16].try_into().unwrap()), 9, "instance_count @12");
    assert_eq!(u32::from_le_bytes(d[28..32].try_into().unwrap()), 7, "base_instance @28");
}
