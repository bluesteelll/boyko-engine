//! The 3D GPU instance pack system (standard-library Phase S4).
//!
//! [`sync_gpu_3d_instances`] packs each visible entity's
//! [`GlobalTransform`](boyko_scene::GlobalTransform) (the source of truth) plus
//! its [`MaterialHandle`](boyko_scene::MaterialHandle) into its
//! [`Gpu3dInstance`](crate::gpu3d_instance::Gpu3dInstance) column. This is the
//! EXPLICIT pack step (a transform/write); the zero-copy part is the COLUMN → GPU
//! upload that the consuming renderer performs via `for_each_chunk` +
//! `bytemuck::cast_slice` (mirroring the demo's `upload_instances`).

use boyko_ecs::ecs::core::iters::query::filter_enable::Enabled;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_scene::render_caps::RenderEnabled;
use boyko_scene::{GlobalTransform, MaterialHandle};

use crate::gpu3d_instance::Gpu3dInstance;

/// Packs each visible entity's `GlobalTransform` + `MaterialHandle` into its
/// `Gpu3dInstance` column — one sequential `Affine3A` read + one packed write per
/// row, alloc-free.
///
/// # What "visible" means here
///
/// The query is filtered on `Enabled<RenderEnabled>`, so a row whose
/// `RenderEnabled` bit is clear (the path `Visibility::Hidden` takes — see
/// [`RenderEnabled`](boyko_scene::RenderEnabled)) is skipped branch-free at
/// iteration: a Hidden row's `Gpu3dInstance` is NEVER refreshed from a moving
/// transform. (Whether a Hidden row is also excluded from the *draw* is the
/// consuming renderer's cull/draw-count policy, out of S4 scope — the column-walk
/// upload reads the whole column. S4 owns the pack + the column.)
///
/// # Layout boundary
///
/// The `matrix3` ROWS are copied verbatim (a direct read — no decomposition);
/// the row-major → column-major transpose is deferred to the shader (the single
/// convention boundary, mirroring `Affine3A::to_mat4`).
///
/// # 0%-gate
///
/// A world with no `Gpu3dInstance` column yields zero matching archetypes, so the
/// system does zero work — a scene that never opts into 3D instancing pays
/// nothing.
#[allow(clippy::needless_pass_by_value)]
pub fn sync_gpu_3d_instances(
    mut q: Query<
        (&GlobalTransform, &MaterialHandle, &mut Gpu3dInstance),
        Enabled<RenderEnabled>,
    >,
) {
    for (g, mat, inst) in q.iter_mut() {
        let a = g.affine();
        let r = a.matrix3.rows;
        inst.linear_rows = [
            [r[0].x, r[0].y, r[0].z],
            [r[1].x, r[1].y, r[1].z],
            [r[2].x, r[2].y, r[2].z],
        ];
        inst.translation = [a.translation.x, a.translation.y, a.translation.z];
        // Low 16 bits = the material handle; high 16 bits stay 0 (pad).
        inst.material = u32::from(mat.0);
    }
}
