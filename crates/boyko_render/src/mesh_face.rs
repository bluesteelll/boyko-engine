//! RD6 / D-DS — the face class of one mesh instance, and the winding census.
//!
//! # The convention (glTF's)
//!
//! A triangle's FRONT is its counter-clockwise side by the right-hand rule, in a right-handed
//! world (glTF 2.0: "the determinant of the node's global transform defines the winding order
//! … if the determinant is a positive value, the front-facing triangles use the counterclockwise
//! order"). The camera passes rasterise exactly that as front-facing: every pipeline's front face
//! is `VK_FRONT_FACE_COUNTER_CLOCKWISE`, the camera basis is `right × up = −forward`, the
//! projections put `clip.y = −up·` and the viewport height is positive. The built-in meshes, the
//! glb importer and every engine fixture follow it.
//!
//! # The face class
//!
//! [`mesh_face_bits`] derives the two
//! [`MESH_FACE_*`](boyko_rhi::enums::MESH_FACE_DOUBLE_SIDED) bits of one instance:
//! `DOUBLE_SIDED` from its material row (`Material::double_sided`), `MIRRORED` from the sign of
//! its model transform's determinant ([`model_det`] over the instance's `InstanceModelCol`,
//! which the instance pack refreshes from `GlobalTransform` for every visible row, static and
//! interpolated alike — so the camera and the caster gathers derive the SAME class from the SAME
//! column). Nothing is stored beyond the per-frame gather lanes: the material column and the
//! instance's model column are the sources of truth.
//!
//! # The winding census
//!
//! [`winding_census`] compares each triangle's geometric normal (its winding) with the mean of
//! its three vertex normals. It detects content that disagrees with the convention — a mesh wound
//! clockwise for its own normals renders inside-out once single-sided back faces are culled, and
//! lights the wrong side of a double-sided one. It never re-winds: glTF normals are free
//! attributes, and a re-wind would change which side reference viewers flip.

use boyko_rhi::enums::{MESH_FACE_DOUBLE_SIDED, MESH_FACE_MIRRORED};

use crate::instance_model::InstanceModelCol;
use crate::mesh::Vertex;

/// The determinant of `model`'s linear 3×3 (`rows[i][0..3]`) — its sign is the instance's
/// handedness: negative ⇔ the transform mirrors, reversing every triangle's screen winding.
#[inline]
pub fn model_det(model: &InstanceModelCol) -> f32 {
    let [r0, r1, r2] = model.rows;
    r0[0] * (r1[1] * r2[2] - r1[2] * r2[1]) - r0[1] * (r1[0] * r2[2] - r1[2] * r2[0])
        + r0[2] * (r1[0] * r2[1] - r1[1] * r2[0])
}

/// The face bits of one instance: [`MESH_FACE_DOUBLE_SIDED`] iff `double_sided`,
/// [`MESH_FACE_MIRRORED`] iff the model's determinant is negative (`det < 0`; a zero, `−0.0` or
/// NaN determinant is not mirrored — a degenerate instance rasterises nothing either way).
///
/// Branch-free: two `bool` multiplies, the determinant's 14 flops on the 48 B the gather
/// already reads.
#[inline]
pub fn mesh_face_bits(double_sided: bool, model: &InstanceModelCol) -> u32 {
    u32::from(double_sided) * MESH_FACE_DOUBLE_SIDED + u32::from(model_det(model) < 0.0) * MESH_FACE_MIRRORED
}

/// How many triangles of a mesh agree with the counter-clockwise convention.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindingCensus {
    /// Triangles whose geometric normal points to the side of their mean vertex normal.
    pub agree: u32,
    /// Triangles whose geometric normal points away from their mean vertex normal — wound
    /// clockwise for their authored normals.
    pub disagree: u32,
    /// Triangles the census cannot judge: an out-of-range index, zero area, a zero mean normal,
    /// or a geometric normal within `|cos| <` [`WINDING_INDETERMINATE_COS`] of perpendicular to
    /// the mean normal (an up-normal foliage card, for one).
    pub indeterminate: u32,
}

impl WindingCensus {
    /// `true` when more triangles disagree with the convention than agree — the mesh is wound
    /// clockwise for its normals.
    #[inline]
    pub fn mostly_clockwise(&self) -> bool {
        self.disagree > self.agree
    }
}

/// The `|cos|` below which [`winding_census`] calls a triangle indeterminate.
pub const WINDING_INDETERMINATE_COS: f32 = 0.05;

/// Counts how `indices`' triangles over `vertices` agree with the convention (see the module
/// doc). Load-time / debug-time only: one pass, no allocation. A trailing partial triangle is
/// ignored.
pub fn winding_census(vertices: &[Vertex], indices: &[u32]) -> WindingCensus {
    let mut out = WindingCensus::default();
    for &[i0, i1, i2] in indices.as_chunks::<3>().0 {
        let (Some(a), Some(b), Some(c)) =
            (vertices.get(i0 as usize), vertices.get(i1 as usize), vertices.get(i2 as usize))
        else {
            out.indeterminate += 1;
            continue;
        };
        let g = cross(sub(b.position, a.position), sub(c.position, a.position));
        let n = [
            a.normal[0] + b.normal[0] + c.normal[0],
            a.normal[1] + b.normal[1] + c.normal[1],
            a.normal[2] + b.normal[2] + c.normal[2],
        ];
        let (gl, nl) = (dot(g, g).sqrt(), dot(n, n).sqrt());
        let cos = if gl > 0.0 && nl > 0.0 { dot(g, n) / (gl * nl) } else { f32::NAN };
        if cos.is_nan() || cos.abs() < WINDING_INDETERMINATE_COS {
            out.indeterminate += 1;
        } else if cos > 0.0 {
            out.agree += 1;
        } else {
            out.disagree += 1;
        }
    }
    out
}

#[inline]
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

#[inline]
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_assets::{cube_geometry, plane_geometry};

    /// An affine with linear part `m` (rows) and a translation that must not matter.
    fn model(m: [[f32; 3]; 3]) -> InstanceModelCol {
        InstanceModelCol {
            rows: [
                [m[0][0], m[0][1], m[0][2], 3.0],
                [m[1][0], m[1][1], m[1][2], -2.0],
                [m[2][0], m[2][1], m[2][2], 7.0],
            ],
        }
    }

    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    const MIRROR_X: [[f32; 3]; 3] = [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    /// A rotation of 30° about Y times a mirror in X, scaled by 2 — a rotated mirror.
    fn rotated_mirror() -> [[f32; 3]; 3] {
        let (s, c) = 30f32.to_radians().sin_cos();
        let r = [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]];
        let mut out = [[0.0; 3]; 3];
        for (i, row) in out.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = 2.0 * r[i][j] * MIRROR_X[j][j];
            }
        }
        out
    }

    /// H3: the bits of every (double-sided × handedness) combination.
    #[test]
    fn face_bits_double_sided_and_mirror() {
        let id = model(IDENTITY);
        let mirror = model(MIRROR_X);
        let rot_mirror = model(rotated_mirror());
        assert!(model_det(&rot_mirror) < 0.0, "fixture: the rotated mirror has a negative determinant");
        assert_eq!(mesh_face_bits(false, &id), 0, "single-sided, not mirrored: class 0");
        assert_eq!(
            mesh_face_bits(true, &id),
            MESH_FACE_DOUBLE_SIDED,
            "RED: a double-sided material must set MESH_FACE_DOUBLE_SIDED"
        );
        assert_eq!(mesh_face_bits(false, &mirror), MESH_FACE_MIRRORED, "RED: det < 0 must set MESH_FACE_MIRRORED");
        assert_eq!(mesh_face_bits(true, &mirror), MESH_FACE_DOUBLE_SIDED | MESH_FACE_MIRRORED);
        assert_eq!(mesh_face_bits(false, &rot_mirror), MESH_FACE_MIRRORED, "a rotated mirror is mirrored");
    }

    /// H3: the determinant's edge cases — only a strictly negative determinant mirrors.
    #[test]
    fn face_bits_determinant_edge_cases() {
        let flat = model([[1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]]);
        assert_eq!(model_det(&flat), 0.0);
        assert_eq!(mesh_face_bits(false, &flat), 0, "det = 0 is not mirrored");
        let neg_zero = model([[-1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]]);
        assert!(model_det(&neg_zero) == 0.0, "fixture: det is ±0");
        assert_eq!(mesh_face_bits(false, &neg_zero), 0, "det = −0 is not mirrored");
        let nan = model([[f32::NAN, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        assert!(model_det(&nan).is_nan());
        assert_eq!(mesh_face_bits(false, &nan), 0, "a NaN determinant is not mirrored");
        let tiny = model([[-1e-10, 0.0, 0.0], [0.0, 1e-10, 0.0], [0.0, 0.0, 1e-10]]);
        assert!(model_det(&tiny) < 0.0, "fixture: a tiny negative determinant stays negative in f32");
        assert_eq!(
            mesh_face_bits(false, &tiny),
            MESH_FACE_MIRRORED,
            "RED: a tiny (1e-30) negative determinant is still a mirror"
        );
        assert_eq!(mesh_face_bits(true, &nan), MESH_FACE_DOUBLE_SIDED, "RED: double-sided survives a NaN model");
    }

    /// H3: the built-ins already follow the convention — every triangle agrees.
    #[test]
    fn winding_census_built_ins_agree() {
        let (v, i) = cube_geometry(1.0);
        assert_eq!(winding_census(&v, &i), WindingCensus { agree: 12, disagree: 0, indeterminate: 0 });
        let (v, i) = plane_geometry(2.0);
        assert_eq!(winding_census(&v, &i), WindingCensus { agree: 2, disagree: 0, indeterminate: 0 });
    }

    /// The `[a, b, a+1] [a+1, b, b+1]` UV-sphere pattern HEAD's test fixtures carry
    /// (`boyko_app/tests/common/mod.rs`), indices only — the census documents that it winds
    /// INWARD for its outward normals.
    fn clockwise_sphere(stacks: u32, slices: u32) -> (Vec<Vertex>, Vec<u32>) {
        let pi = core::f32::consts::PI;
        let mut verts = Vec::new();
        for i in 0..=stacks {
            let (sp, cp) = ((i as f32 / stacks as f32) * pi).sin_cos();
            for j in 0..=slices {
                let (st, ct) = ((j as f32 / slices as f32) * (2.0 * pi)).sin_cos();
                let n = [sp * ct, cp, sp * st];
                verts.push(Vertex::new(n, n, [1.0; 4]));
            }
        }
        let stride = slices + 1;
        let mut idx = Vec::new();
        for i in 0..stacks {
            for j in 0..slices {
                let (a, b) = (i * stride + j, (i + 1) * stride + j);
                if i != 0 {
                    idx.extend_from_slice(&[a, b, a + 1]);
                }
                if i != stacks - 1 {
                    idx.extend_from_slice(&[a + 1, b, b + 1]);
                }
            }
        }
        (verts, idx)
    }

    /// H3: the HEAD fixture pattern is clockwise for its normals — every triangle disagrees —
    /// and swapping each triangle's 2nd and 3rd index (D-3's re-wind) makes every one agree.
    #[test]
    fn winding_census_head_sphere_pattern_disagrees() {
        let (v, mut i) = clockwise_sphere(8, 12);
        let tris = (i.len() / 3) as u32;
        let census = winding_census(&v, &i);
        assert_eq!(census, WindingCensus { agree: 0, disagree: tris, indeterminate: 0 });
        assert!(census.mostly_clockwise());
        for t in i.as_chunks_mut::<3>().0 {
            t.swap(1, 2);
        }
        assert_eq!(winding_census(&v, &i), WindingCensus { agree: tris, disagree: 0, indeterminate: 0 });
    }

    /// H3: a vertical card whose normals point UP (a foliage card's lighting trick) is
    /// indeterminate, not a disagreement — the census stays quiet on it.
    #[test]
    fn winding_census_up_normal_card_is_indeterminate() {
        let up = [0.0, 1.0, 0.0];
        let v = [
            Vertex::new([-1.0, 0.0, 0.0], up, [1.0; 4]),
            Vertex::new([1.0, 0.0, 0.0], up, [1.0; 4]),
            Vertex::new([1.0, 2.0, 0.0], up, [1.0; 4]),
            Vertex::new([-1.0, 2.0, 0.0], up, [1.0; 4]),
        ];
        let census = winding_census(&v, &[0, 1, 2, 0, 2, 3]);
        assert_eq!(census, WindingCensus { agree: 0, disagree: 0, indeterminate: 2 });
        assert!(!census.mostly_clockwise());
        assert_eq!(winding_census(&v, &[0, 1, 9]).indeterminate, 1, "an out-of-range index is indeterminate");
    }
}
