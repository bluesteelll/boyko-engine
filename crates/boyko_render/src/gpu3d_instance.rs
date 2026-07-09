//! The 3D GPU instance component (standard-library Phase S4).
//!
//! [`Gpu3dInstance`] is the 3D analogue of the demo's 2D `GpuInstance`: one
//! per-entity record packed for the instance vertex buffer. It is a SEPARATE
//! component (its own const-asserted layout + WGSL attribute contract) because a
//! 3D world pose (a 3×3 linear part + a translation) does not fit the 2D record's
//! 24 B — the 24 B `GpuInstance` is byte-frozen and untouched.
//!
//! # Principle 0
//!
//! The `Gpu3dInstance` column IS the GPU instance buffer source — there is NO
//! parallel `std::Vec` mirror. The source of truth is a DIFFERENT column
//! ([`GlobalTransform`](boyko_scene::GlobalTransform)), so
//! [`sync_gpu_3d_instances`](crate::gpu3d_system::sync_gpu_3d_instances) is an
//! EXPLICIT pack system (one affine read + one packed write per visible row,
//! alloc-free) — a transform/write, not a zero-copy reinterpret. The zero-copy
//! step is the COLUMN → GPU upload: the contiguous `Gpu3dInstance` column is
//! `bytemuck::cast_slice`d straight into the vertex buffer (the consuming
//! renderer owns that upload, mirroring the demo's `upload_instances`).

use boyko_macros::Component;
use bytemuck::{Pod, Zeroable};

/// One 3D instanced object on the GPU.
///
/// `#[repr(C)]` pins the field order so the WGSL instance-buffer attribute
/// offsets stay in lockstep with this layout. The linear part is stored as the
/// three ROWS of [`Affine3A::matrix3`](boyko_math::Affine3A) verbatim (a direct
/// read — no decomposition); `boyko_math`'s `Mat3` is row-major, and the single
/// row-major ↔ column-major boundary is crossed in the shader (mirroring
/// [`Affine3A::to_mat4`](boyko_math::Affine3A::to_mat4)).
///
/// # WGSL attribute contract (instance-stepped, base location `N`)
///
/// * `linear_rows` — 3 × `vec3<f32>` at `@location(N)`, `@location(N+1)`,
///   `@location(N+2)`: the `Affine3A.matrix3` ROWS (row-major). The shader builds
///   the column-major model matrix as `mat3(col0, col1, col2)` where
///   `col_j = (rows[0][j], rows[1][j], rows[2][j])`.
/// * `translation` — `vec3<f32>` at `@location(N+3)`: the world position.
/// * `material` — `u32` at `@location(N+4)`: low 16 bits = the
///   [`MaterialHandle`](boyko_scene::MaterialHandle); high 16 bits = pad (0).
///
/// # Interpolation
///
/// v1 ships WITHOUT GPU-side `mix(prev, pos, alpha)` (no `prev_pose` lane, so no
/// second prev-shuffle writer — the 2D Phase-20.1 single-writer interpolation is
/// untouched and needs no reconciliation). 3D interpolation is deferred, not
/// dropped.
///
/// # POD / layout
///
/// The fields are all `f32`-aligned with no padding holes (52 B, a multiple of
/// 4), which is required for the `bytemuck::Pod` derive and for the `cast_slice`
/// upload to be sound. The `Component` derive is a pure marker (it only assigns a
/// `ComponentId`), so the column stays a valid GPU instance array.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct Gpu3dInstance {
    /// The `Affine3A.matrix3` rows (row-major), copied verbatim — no
    /// decomposition. `linear_rows[i]` is row `i` of the world linear part. 36 B.
    pub linear_rows: [[f32; 3]; 3],
    /// The world translation. 12 B.
    pub translation: [f32; 3],
    /// Packed material: low 16 bits = the material handle; high 16 bits = pad (0).
    /// 4 B.
    pub material: u32,
}

/// The expected size of [`Gpu3dInstance`] in bytes (36 + 12 + 4 = 52 B, no
/// padding).
pub const GPU3D_INSTANCE_SIZE: usize = 52;

// The whole 3D-instancing strategy depends on this exact size / alignment: the
// vertex-buffer stride and every WGSL attribute offset derive from it. A silent
// layout change (an added field, padding from a non-POD field) must fail the
// build, not corrupt the draw — mirroring the demo's 24 B `GpuInstance` pins.
const _: () = assert!(size_of::<Gpu3dInstance>() == GPU3D_INSTANCE_SIZE);
const _: () = assert!(align_of::<Gpu3dInstance>() == 4);
