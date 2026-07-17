//! The per-entity 48-byte model-affine instance column (mesh foundation M3).
//!
//! [`InstanceModelCol`] is the EXACT byte layout the M1/M2 gbuffer vertex shader
//! reads as `StructuredBuffer<InstanceModelCol> instances` at set 0 (binding 0): a
//! 3×4 ROW-MAJOR affine, 12 `f32` = 48 B, laid out as three interleaved
//! `[row.xyz | translation_component]` quads. The M3 bucketed gather
//! ([`crate::mesh_draw`]) scatters this column into a per-mesh contiguous instance
//! ring; the recorder binds that ring once and the VS indexes
//! `instances[base_instance + SV_InstanceID]`.
//!
//! # Why a SIBLING column to [`Gpu3dInstance`](crate::gpu3d_instance::Gpu3dInstance)
//!
//! `Gpu3dInstance` is the S4 GPU-instance record (52 B: `linear_rows` 36 B +
//! `translation` 12 B + `material` 4 B) — its memory layout stores the three linear
//! ROWS contiguously and the translation AFTER them, NOT the interleaved 3×4 the
//! gbuffer VS's `InstanceModelCol` reads. They encode the same affine but with
//! DIFFERENT byte layouts: `Gpu3dInstance` feeds the S4 vertex-stepped instance
//! buffer (`@location` attributes); `InstanceModelCol` feeds the M1/M2 SSBO the VS
//! indexes by `SV_InstanceID`. M3's gather + draw need the SSBO layout, so this is
//! the column it produces — a sibling, not a replacement (the architect's "add a
//! sibling that produces the 48 B model column").
//!
//! # Principle 0
//!
//! The `InstanceModelCol` column IS the instance-ring source — there is NO parallel
//! `std::Vec` mirror. The source of truth is the
//! [`GlobalTransform`](boyko_scene::GlobalTransform) column;
//! [`sync_instance_model_cols`] is the EXPLICIT pack (one affine read + one packed
//! write per visible row, alloc-free), symmetric with
//! [`sync_gpu_3d_instances`](crate::gpu3d_system::sync_gpu_3d_instances).

use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::iters::query::filter_enable::Enabled;
use boyko_macros::Component;
use boyko_scene::GlobalTransform;
use boyko_scene::render_caps::RenderEnabled;
use bytemuck::{Pod, Zeroable};

/// One per-entity 3×4 ROW-MAJOR model affine, byte-identical to the gbuffer VS's
/// `InstanceModelCol`.
///
/// `#[repr(C)]` pins the 48-byte stride the M1/M2 instance SSBO declares. `rows[i]`
/// is `[m3_row_i.x, m3_row_i.y, m3_row_i.z, translation_i]` — the linear row's three
/// components followed by that row's translation component (the row-major 3×4 the VS
/// multiplies a model-space position by). The single row-major ↔ column-major
/// boundary is crossed in the shader (mirroring
/// [`Affine3A::to_mat4`](boyko_math::Affine3A::to_mat4) and the M2 host
/// `instance_affine` mirror).
///
/// # POD / layout
///
/// All `f32`, no padding holes (48 B, a multiple of 4) — required for the
/// `bytemuck::Pod` derive and for the gather's `cast_slice` scatter into the mapped
/// SSBO ring to be sound. The `Component` derive is a pure marker (it only assigns a
/// `ComponentId`).
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct InstanceModelCol {
    /// The three interleaved `[linear_row.xyz | translation_component]` quads (the
    /// 3×4 row-major affine). 48 B.
    pub rows: [[f32; 4]; 3],
}

/// The byte size of one [`InstanceModelCol`] — the M1/M2 instance SSBO stride (48 B).
/// Equals `boyko_rhi_vulkan`'s `GBUFFER_INSTANCE_MODEL_BYTES` (the layout contract is
/// cross-crate, so the size is re-pinned on each side).
pub const INSTANCE_MODEL_COL_BYTES: usize = 48;

// The whole M3 instancing strategy depends on this exact size: the SSBO stride and
// every per-bucket `base_instance` byte offset derive from it. A silent layout change
// (an added field, padding from a non-POD field) must fail the build, not corrupt the
// draw — mirroring `Gpu3dInstance`'s pins.
const _: () = assert!(size_of::<InstanceModelCol>() == INSTANCE_MODEL_COL_BYTES);
const _: () = assert!(align_of::<InstanceModelCol>() == 4);

impl InstanceModelCol {
    /// Packs a [`GlobalTransform`]'s affine into the interleaved 3×4 row-major
    /// layout the gbuffer VS reads. The linear ROWS are copied verbatim (no
    /// decomposition); the row-major → column-major transpose is the shader's job.
    #[inline]
    pub fn from_global(g: &GlobalTransform) -> Self {
        let a = g.affine();
        let r = a.matrix3.rows;
        let t = a.translation;
        Self {
            rows: [
                [r[0].x, r[0].y, r[0].z, t.x],
                [r[1].x, r[1].y, r[1].z, t.y],
                [r[2].x, r[2].y, r[2].z, t.z],
            ],
        }
    }
}

/// Packs each visible entity's `GlobalTransform` into its [`InstanceModelCol`] column
/// — one sequential `Affine3A` read + one packed write per row, alloc-free.
///
/// # What "visible" means
///
/// The query is filtered on `Enabled<RenderEnabled>`, so a row whose `RenderEnabled`
/// bit is clear (the `Visibility::Hidden` path) is skipped branch-free at iteration —
/// its `InstanceModelCol` is never refreshed from a moving transform (the same gate
/// [`sync_gpu_3d_instances`](crate::gpu3d_system::sync_gpu_3d_instances) uses).
///
/// # 0%-gate
///
/// A world with no `InstanceModelCol` column yields zero matching archetypes, so the
/// system does zero work — a scene that never opts into M3 instancing pays nothing.
#[allow(clippy::needless_pass_by_value)]
pub fn sync_instance_model_cols(
    mut q: Query<(&GlobalTransform, &mut InstanceModelCol), Enabled<RenderEnabled>>,
) {
    for (g, col) in q.iter_mut() {
        *col = InstanceModelCol::from_global(g);
    }
}

/// The per-entity PREVIOUS-frame model affine — a byte-identical dense sibling of
/// [`InstanceModelCol`], carrying the transform the entity had LAST frame.
///
/// # Why it exists (HW-RT Rung 3b — temporal shadow-vis motion vectors)
///
/// The temporal shadow-vis denoiser reprojects each pixel's shadow term from where its
/// surface *was* last frame. For a moving mesh box the correct per-object motion vector
/// needs the box's PREVIOUS model transform in the raster VS
/// (`prev_world = prev_m3·position_local + prev_t`), computed alongside the current
/// `cur_world` — the deferred domain has neither `SV_InstanceID` nor `position_local`, so
/// mesh motion vectors MUST be generated in the raster pass, and that pass reads this
/// prev-transform column. This sibling is the ECS-native carry of that prev-transform.
///
/// # Principle 0
///
/// The prev-transform is durable per-entity data, so it lives in a dense `ComponentPool`
/// column — NOT a side `std::Vec<PrevModel>` (the SP4-race lesson: a parallel data system
/// glued on the side is the anti-pattern this engine forbids). It is the exact 48-byte
/// layout of [`InstanceModelCol`]: the gbuffer VS reads it as the same
/// `StructuredBuffer<InstanceModelCol>` stride, just from the prev-instance ring instead of
/// the current one.
///
/// # NOT `hwrt`-walled (un-walled by TAA rung D1 — mirrors [`crate::motion_cam`]'s W3 un-wall)
///
/// Was `#[cfg(feature = "hwrt")]`-gated when rung 3b introduced it for the shadow-temporal
/// mesh-MV raster producer. Un-walled (TAA rung D1) so this component + its sync system
/// ([`sync_prev_instance_model_cols`]) are reachable/testable on BOTH legs — a FUTURE
/// per-object TAA reprojection consumer needs this column to exist as a type before it can be
/// wired, exactly as [`crate::motion_cam::MotionCam`] was un-walled ahead of its GPU consumer.
///
/// This is a data-layer-only change, and a NARROWER one than `motion_cam`'s: the GPU producer
/// (`gbuffer_mrt_mv`, the `motion_vec` target, `MotionVecResources` in `boyko_app`) stays
/// `#[cfg(feature = "hwrt")]`-gated, unchanged, AND — unlike `MotionCam`, whose upload fn
/// ([`crate::upload_motion_cam_ring`]) was un-walled alongside it — this type's own upload fn
/// ([`crate::upload_prev_instance_models`]) STAYS `hwrt`-gated: it reads
/// [`crate::MeshRenderScratch::prev_ring`], a SEPARATE `hwrt`-only wall in `mesh_draw.rs` this
/// rung does not touch (see the D1 report for why). No plugin adds this column to any archetype
/// on either leg yet, so the 0%-gate ([`sync_prev_instance_model_cols`]'s own doc) holds
/// byte-identically on BOTH legs: zero matching archetypes ⇒ zero work, regardless of `hwrt`.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct PrevInstanceModelCol {
    /// The three interleaved `[linear_row.xyz | translation_component]` quads — the SAME
    /// 3×4 row-major affine layout as [`InstanceModelCol`], holding LAST frame's transform.
    pub rows: [[f32; 4]; 3],
}

// The prev-instance ring stride MUST equal the current instance ring stride (48 B): the
// gbuffer VS indexes both by `base_instance + SV_InstanceID`, so a layout divergence would
// desynchronise the two rings and reproject to a wrong surface point.
const _: () = assert!(size_of::<PrevInstanceModelCol>() == INSTANCE_MODEL_COL_BYTES);
const _: () = assert!(align_of::<PrevInstanceModelCol>() == 4);

/// Copies each visible entity's CURRENT [`InstanceModelCol`] into its
/// [`PrevInstanceModelCol`] — one sequential 48-byte column-to-column copy per row,
/// alloc-free, branch-free (the `Enabled<RenderEnabled>` filter is a structural skip).
///
/// # Ordering — MUST run `.before(sync_instance_model_cols)`
///
/// This captures `prev := curr` BEFORE [`sync_instance_model_cols`] refreshes `curr` from
/// this frame's moving [`GlobalTransform`]. The plugin pins the `.before` edge. With it, at
/// frame N: `prev` holds frame N−1's transform (the value `curr` still carries at frame
/// start) and `curr` is then overwritten with frame N's — so the motion vector `cur − prev`
/// is exactly this frame's per-object displacement. Reordering it AFTER the refresh would
/// make `prev == curr` (zero motion, every box ghosts under its own motion — the exact
/// class the denoiser must fix).
///
/// # 0%-gate
///
/// A world with no [`PrevInstanceModelCol`] column yields zero matching archetypes ⇒ zero
/// work: a scene that never opts into temporal motion vectors pays nothing. Holds on BOTH
/// legs (this system is un-walled from `hwrt` — see the type's own doc).
#[allow(clippy::needless_pass_by_value)]
pub fn sync_prev_instance_model_cols(
    mut q: Query<(&InstanceModelCol, &mut PrevInstanceModelCol), Enabled<RenderEnabled>>,
) {
    for (cur, prev) in q.iter_mut() {
        prev.rows = cur.rows;
    }
}

/// Multi-paradigm render-path plan, rung R-VBGEO (plan §Data structures) — the
/// `VisibilityBuffer` path's OWN instance row: [`InstanceModelCol`]'s 48-byte 3×4
/// row-major affine (byte-identical leading bytes, offset 0..48) plus an appended
/// `mesh_id: u32` lane (offset 48, Decision 0's geometry-table slot) padded to a
/// 64-byte std430-stable stride.
///
/// A VB-path-CONDITIONAL row shape, NOT a widening of [`InstanceModelCol`] itself:
/// Deferred/Forward keep the 48-byte column EXACTLY (byte-identity — this type is
/// never read by any pipeline those paths bind). The VB compute fetch
/// (`vb_geom_fetch.hlsli`, R8) needs `mesh_id` PER INSTANCE (not per-draw/push-constant)
/// because a VB shading pass holds only `(instance_id, triangle_id)` per pixel with no
/// per-draw binding — see Decision 0.
///
/// # Reachable as of rung R8
///
/// [`crate::mesh_draw::MeshRenderScratch::sync_vb_instance_ring`] builds a ring of these rows
/// (from the SAME `ring`/`mesh_ids` gather output [`InstanceModelCol`]'s own scatter
/// populates) on a `VisibilityBuffer`-resolved boot; `boyko_render::upload::
/// upload_vb_instance_rows` uploads it into `GpuSceneBundles::vb_instance_rings`. Deferred/
/// Forward/ForwardPlus never construct this ring (Principle 1: the boot-resolved path selects
/// WHICH gather/upload pair runs, never a per-instance branch), pinned by the offset
/// const-asserts below.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct VbInstanceRow {
    /// The SAME interleaved `[linear_row.xyz | translation_component]` quads as
    /// [`InstanceModelCol::rows`] — byte-identical leading 48 bytes (offset 0..48).
    pub affine: [[f32; 4]; 3],
    /// The Decision-0 geometry-table slot (this instance's mesh's `mesh_id`) — the key
    /// the VB compute fetch resolves `gMeshIndices[]`/`gMeshVerts[]`/`gMeshMeta[]`
    /// through. Offset 48.
    pub mesh_id: u32,
    /// Pads the row to a 64-byte std430-stable stride — unused, always zero. Offset 52.
    pub _pad: [u32; 3],
}

/// The byte size of one [`VbInstanceRow`] — the VB-path instance SSBO's per-instance
/// stride (64 B: [`InstanceModelCol`]'s 48-byte affine + a `uint` `mesh_id` + a
/// 12-byte pad to the next std430 lane).
pub const VB_INSTANCE_ROW_BYTES: usize = 64;

const _: () = assert!(
    size_of::<VbInstanceRow>() == VB_INSTANCE_ROW_BYTES,
    "VbInstanceRow must be 64 bytes (InstanceModelCol's 48-byte affine + a mesh_id uint, padded)"
);
const _: () = assert!(align_of::<VbInstanceRow>() == 4);
const _: () = assert!(core::mem::offset_of!(VbInstanceRow, affine) == 0);
const _: () = assert!(core::mem::offset_of!(VbInstanceRow, mesh_id) == 48);
// The leading 48 bytes MUST byte-match `InstanceModelCol` — Deferred/Forward read
// exactly that layout; the VB path reads the SAME leading bytes plus the appended lane.
const _: () = assert!(core::mem::offset_of!(VbInstanceRow, affine) == core::mem::offset_of!(InstanceModelCol, rows));

impl VbInstanceRow {
    /// Packs an [`InstanceModelCol`] (the already-computed 3×4 affine) plus its
    /// resolved `mesh_id` into the VB-path row shape — the "second packing fn selected
    /// at boot" Principle 1 calls for (a future VB gather, R8/R9, calls this instead of
    /// writing `InstanceModelCol` directly; no per-instance path branch is needed
    /// since the boot-resolved path selects WHICH gather runs, not a per-row check).
    #[inline]
    pub const fn from_model_col(model: &InstanceModelCol, mesh_id: u32) -> Self {
        Self { affine: model.rows, mesh_id, _pad: [0; 3] }
    }
}

#[cfg(test)]
mod vb_instance_row_tests {
    use super::*;

    #[test]
    fn from_model_col_copies_the_affine_and_mesh_id_verbatim() {
        let model = InstanceModelCol {
            rows: [[1.0, 2.0, 3.0, 4.0], [5.0, 6.0, 7.0, 8.0], [9.0, 10.0, 11.0, 12.0]],
        };
        let row = VbInstanceRow::from_model_col(&model, 42);
        assert_eq!(row.affine, model.rows);
        assert_eq!(row.mesh_id, 42);
        assert_eq!(row._pad, [0, 0, 0]);
    }

    #[test]
    fn vb_instance_row_leading_bytes_match_instance_model_col_byte_for_byte() {
        let model = InstanceModelCol {
            rows: [[1.0, 2.0, 3.0, 4.0], [5.0, 6.0, 7.0, 8.0], [9.0, 10.0, 11.0, 12.0]],
        };
        let row = VbInstanceRow::from_model_col(&model, 7);
        let model_bytes = bytemuck::bytes_of(&model);
        let row_bytes = bytemuck::bytes_of(&row);
        assert_eq!(&row_bytes[0..INSTANCE_MODEL_COL_BYTES], model_bytes);
    }
}
