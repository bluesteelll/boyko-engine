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
