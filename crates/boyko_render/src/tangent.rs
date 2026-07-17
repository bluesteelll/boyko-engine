//! Per-vertex tangent generation (Lengyel's method) — a load-time, ONE-SHOT pass
//! (Principle 1: never per-frame) that derives [`Vertex::tangent`] from a mesh's
//! already-final `position` / `normal` / `uv` fields.
//!
//! Consumers: the host-authored `cube`/`plane` primitives
//! ([`mesh_assets`](crate::mesh_assets)) and the `.obj` loader's post-dedup pass
//! ([`ObjMeshLoader`](crate::loaders::ObjMeshLoader)) — both run this exactly once,
//! at mesh-build time, never in a per-frame path.

use boyko_math::Vec3;

use crate::mesh::Vertex;

/// The UV-determinant degenerate-triangle guard, mirroring the gbuffer VS's own
/// `DET_EPS` discipline (`gbuffer_mrt.vs.hlsl`) for a near-singular basis: a
/// triangle whose UV area is at or below this contributes no tangent/bitangent
/// (its corners rely on the per-vertex arbitrary-tangent fallback instead).
const DET_EPS: f32 = 1e-8;

/// The 3D-geometric degenerate-triangle guard: the squared magnitude of a
/// triangle's edge-cross-product (`|edge1 × edge2|² = (2·area)²`) at or below this
/// marks a triangle with (near-)zero WORLD area. This is a DISTINCT degeneracy from
/// [`DET_EPS`] above: a UV-sphere's poles are fans of duplicated vertices sharing one
/// 3D position, so a pole-fan triangle has a zero-length edge (zero 3D area) yet a
/// perfectly HEALTHY UV determinant (the two pole corners carry different `u`) — the
/// `DET_EPS` check never fires, but the triangle injects a spurious bitangent that
/// poisons the pole-ring basis (dark/smeared pole shading). Rejecting it by genuine
/// 3D degeneracy mirrors Bevy's UV-sphere builder, which simply omits these triangles.
/// The threshold is far below any well-formed mesh triangle at unit-ish scale (a thin
/// but valid pole-adjacent triangle has `|edge1 × edge2|² ~ 1e-4`), so only the
/// exact-zero-area duplicated-vertex case is caught.
const GEO_AREA_EPS_SQ: f32 = 1e-12;

/// Computes a per-vertex tangent (unit `xyz` + handedness `w`) for `vertices` from
/// `indices` (a flat `Uint32` triangle list) via Lengyel's method:
///
/// 1. Per triangle, solve `T`/`B` from the two edge vectors and their UV deltas
///    (`r = 1 / (s1*t2 - s2*t1)`) and accumulate into each of its 3 corners.
/// 2. Per vertex, Gram-Schmidt-orthogonalize the accumulated tangent against the
///    vertex's own `normal` (`T' = normalize(T - N * dot(N, T))`), then derive the
///    handedness `w = sign(dot(cross(N, T'), B))`.
///
/// A triangle whose UV area is degenerate (`|det| < DET_EPS`, e.g. a UV-less
/// mesh where every `uv` is `[0.0, 0.0]`) OR whose 3D area is degenerate
/// (`|edge1 × edge2|² < GEO_AREA_EPS_SQ`, e.g. a UV-sphere pole-fan triangle with
/// a zero-length edge) contributes nothing. A vertex that ends up with NO
/// contribution (every owning triangle degenerate, or an isolated vertex — e.g. a
/// whole UV-less mesh) falls back to an arbitrary orthonormal tangent derived from
/// its normal alone: harmless, since it is unread by every pipeline that has no
/// normal map to sample.
///
/// # Panics (debug only)
/// `debug_assert!`s `indices.len()` is a multiple of 3 and every index is in
/// range for `vertices` — a malformed triangle list is a caller bug (the mesh
/// generators / the OBJ loader's dedup pass never produce one).
pub fn generate_tangents(vertices: &mut [Vertex], indices: &[u32]) {
    debug_assert!(
        indices.len().is_multiple_of(3),
        "invariant: a triangle-list index buffer's length is a multiple of 3"
    );

    let mut tan_accum = vec![Vec3::ZERO; vertices.len()];
    let mut bitan_accum = vec![Vec3::ZERO; vertices.len()];

    for tri in indices.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let corners = [i0, i1, i2];
        for &i in &corners {
            debug_assert!(i < vertices.len(), "invariant: triangle index in range for vertices");
        }

        let p0 = position_of(&vertices[i0]);
        let p1 = position_of(&vertices[i1]);
        let p2 = position_of(&vertices[i2]);
        let uv0 = vertices[i0].uv;
        let uv1 = vertices[i1].uv;
        let uv2 = vertices[i2].uv;

        let edge1 = p1 - p0;
        let edge2 = p2 - p0;
        let delta_u1 = uv1[0] - uv0[0];
        let delta_v1 = uv1[1] - uv0[1];
        let delta_u2 = uv2[0] - uv0[0];
        let delta_v2 = uv2[1] - uv0[1];

        let det = delta_u1 * delta_v2 - delta_u2 * delta_v1;
        if det.abs() < DET_EPS {
            continue;
        }
        // Reject a triangle with (near-)zero 3D AREA even when its UV determinant is
        // healthy — the UV-sphere pole-fan case (see `GEO_AREA_EPS_SQ`). Without this,
        // a zero-area pole triangle contributes a spurious bitangent that corrupts the
        // pole-ring handedness/basis and darkens/smears the shaded pole.
        if edge1.cross(edge2).length_squared() < GEO_AREA_EPS_SQ {
            continue;
        }
        let r = det.recip();
        let t = (edge1 * delta_v2 - edge2 * delta_v1) * r;
        let b = (edge2 * delta_u1 - edge1 * delta_u2) * r;

        for &i in &corners {
            tan_accum[i] = tan_accum[i] + t;
            bitan_accum[i] = bitan_accum[i] + b;
        }
    }

    for (i, vertex) in vertices.iter_mut().enumerate() {
        let n = normal_of(vertex);
        let ortho = tan_accum[i] - n * n.dot(tan_accum[i]);
        // Mirrors `Vec3::normalize`'s own degenerate guard: a zero (or every-
        // triangle-degenerate) accumulation, or one that landed exactly parallel
        // to `n`, cannot be turned into a tangent direction.
        vertex.tangent = if ortho.length_squared() <= f32::MIN_POSITIVE {
            arbitrary_tangent(n)
        } else {
            let t = ortho.normalize();
            let w = if n.cross(t).dot(bitan_accum[i]) < 0.0 { -1.0 } else { 1.0 };
            [t.x, t.y, t.z, w]
        };
    }
}

#[inline]
fn position_of(v: &Vertex) -> Vec3 {
    Vec3::new(v.position[0], v.position[1], v.position[2])
}

#[inline]
fn normal_of(v: &Vertex) -> Vec3 {
    Vec3::new(v.normal[0], v.normal[1], v.normal[2])
}

/// An arbitrary tangent orthogonal to `n`, for a vertex whose UV gave no usable
/// direction. Picks a helper axis not (nearly) parallel to `n` so the cross
/// product is never degenerate, then normalizes it; handedness is fixed at `+1`
/// (arbitrary — no UV means no consumer reads it as a normal-map basis).
#[inline]
fn arbitrary_tangent(n: Vec3) -> [f32; 4] {
    let helper = if n.x.abs() < 0.9 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::new(0.0, 1.0, 0.0) };
    let t = helper.cross(n).normalize();
    [t.x, t.y, t.z, 1.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    fn assert_orthonormal(vertex: &Vertex) {
        let t = Vec3::new(vertex.tangent[0], vertex.tangent[1], vertex.tangent[2]);
        let n = Vec3::new(vertex.normal[0], vertex.normal[1], vertex.normal[2]);
        assert!((t.length() - 1.0).abs() < EPS, "tangent must be unit length, got {t:?}");
        assert!(t.dot(n).abs() < EPS, "tangent must be orthogonal to normal, got dot={}", t.dot(n));
    }

    /// A single flat triangle with a straightforward axis-aligned UV: the
    /// textbook Lengyel case, `T` along `+X`, `B` along `+Y`, positive handedness.
    #[test]
    fn single_triangle_axis_aligned_uv_yields_expected_tangent() {
        let mut vertices = vec![
            Vertex { uv: [0.0, 0.0], ..Vertex::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0; 4]) },
            Vertex { uv: [1.0, 0.0], ..Vertex::new([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0; 4]) },
            Vertex { uv: [0.0, 1.0], ..Vertex::new([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0; 4]) },
        ];
        generate_tangents(&mut vertices, &[0, 1, 2]);
        for v in &vertices {
            assert_orthonormal(v);
            assert!((v.tangent[0] - 1.0).abs() < EPS, "expected T = (1,0,0), got {:?}", v.tangent);
            assert!(v.tangent[1].abs() < EPS && v.tangent[2].abs() < EPS);
            assert!((v.tangent[3] - 1.0).abs() < EPS, "expected +1 handedness, got {}", v.tangent[3]);
        }
    }

    /// The SAME triangle geometry, but the UV is mirrored across the `u` axis
    /// (`uv1.u` negated) — the common "mirrored UV island" case. The tangent
    /// direction flips and the handedness sign flips to `-1`.
    #[test]
    fn mirrored_uv_flips_handedness_sign() {
        let mut vertices = vec![
            Vertex { uv: [0.0, 0.0], ..Vertex::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0; 4]) },
            Vertex { uv: [-1.0, 0.0], ..Vertex::new([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0; 4]) },
            Vertex { uv: [0.0, 1.0], ..Vertex::new([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0; 4]) },
        ];
        generate_tangents(&mut vertices, &[0, 1, 2]);
        for v in &vertices {
            assert_orthonormal(v);
            assert!((v.tangent[0] + 1.0).abs() < EPS, "expected T = (-1,0,0), got {:?}", v.tangent);
            assert!((v.tangent[3] + 1.0).abs() < EPS, "expected -1 handedness, got {}", v.tangent[3]);
        }
    }

    /// A vertex whose STORED normal is tilted away from its triangle's flat
    /// geometric normal (the smooth-shading case) exercises the Gram-Schmidt
    /// re-orthogonalization step for real (a trivial planar mesh never needs it —
    /// the raw accumulated tangent is already `⟂ N`).
    #[test]
    fn gram_schmidt_reorthogonalizes_against_a_tilted_stored_normal() {
        let tilted_n = Vec3::new(1.0, 0.0, 1.0).normalize();
        let mut vertices = vec![
            Vertex {
                normal: [tilted_n.x, tilted_n.y, tilted_n.z],
                uv: [0.0, 0.0],
                ..Vertex::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [1.0; 4])
            },
            Vertex { uv: [1.0, 0.0], ..Vertex::new([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0; 4]) },
            Vertex { uv: [0.0, 1.0], ..Vertex::new([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0; 4]) },
        ];
        generate_tangents(&mut vertices, &[0, 1, 2]);
        assert_orthonormal(&vertices[0]);
        // The un-tilted corners keep the trivial (already-orthogonal) case.
        assert_orthonormal(&vertices[1]);
        assert_orthonormal(&vertices[2]);
    }

    /// Every UV is `[0.0, 0.0]` (a UV-less mesh, or `Vertex::new`'s default) — every
    /// triangle's UV determinant is exactly zero, so every vertex falls back to the
    /// arbitrary orthonormal tangent. Still `⟂ N`, still unit length.
    #[test]
    fn degenerate_uv_falls_back_to_arbitrary_orthonormal_tangent() {
        let mut vertices = vec![
            Vertex::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0; 4]),
            Vertex::new([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0; 4]),
            Vertex::new([0.0, 1.0, 0.0], [0.0, 1.0, 0.0], [1.0; 4]),
        ];
        generate_tangents(&mut vertices, &[0, 1, 2]);
        for v in &vertices {
            assert_orthonormal(v);
        }
    }

    /// A UV-sphere pole-fan triangle: two corners share the SAME 3D position (the
    /// pole's duplicated vertices) but carry DIFFERENT `u` — so its UV determinant is
    /// healthy (the `DET_EPS` guard does not fire) yet its 3D area is exactly zero. It
    /// must contribute NOTHING (the `GEO_AREA_EPS_SQ` guard), else it injects a spurious
    /// bitangent. Here the two pole corners own no other triangle, so they must fall back
    /// to the arbitrary orthonormal tangent — proving the degenerate triangle was skipped
    /// (had it contributed, its non-zero bitangent would have set a real, non-fallback
    /// basis and the tangent would depend on it).
    #[test]
    fn zero_area_pole_fan_triangle_contributes_nothing() {
        let pole_n = [0.0, 1.0, 0.0];
        let mut vertices = vec![
            // Two coincident "pole" vertices at the same position, different u.
            Vertex { uv: [0.0, 0.0], ..Vertex::new([0.0, 1.0, 0.0], pole_n, [1.0; 4]) },
            Vertex { uv: [0.25, 0.0], ..Vertex::new([0.0, 1.0, 0.0], pole_n, [1.0; 4]) },
            // A distinct ring vertex.
            Vertex { uv: [0.0, 0.1], ..Vertex::new([0.1, 0.9, 0.0], [1.0, 0.0, 0.0], [1.0; 4]) },
        ];
        // Triangle (pole0, pole1, ring): edge (pole1 - pole0) is exactly zero -> zero 3D area.
        generate_tangents(&mut vertices, &[0, 1, 2]);
        // Both pole vertices own ONLY this degenerate triangle -> arbitrary fallback,
        // still orthonormal, never NaN.
        for v in &vertices[0..2] {
            assert_orthonormal(v);
            let expected = arbitrary_tangent(Vec3::new(0.0, 1.0, 0.0));
            assert!(
                (0..4).all(|k| (v.tangent[k] - expected[k]).abs() < EPS),
                "pole vertex must take the arbitrary-tangent fallback (degenerate triangle skipped), got {:?}",
                v.tangent
            );
        }
    }

    /// An isolated vertex (present in `vertices` but referenced by no triangle in
    /// `indices`) also falls back cleanly — no panic, still orthonormal.
    #[test]
    fn vertex_owned_by_no_triangle_falls_back_cleanly() {
        let mut vertices = vec![
            Vertex { uv: [0.0, 0.0], ..Vertex::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0; 4]) },
            Vertex { uv: [1.0, 0.0], ..Vertex::new([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0; 4]) },
            Vertex { uv: [0.0, 1.0], ..Vertex::new([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0; 4]) },
            Vertex::new([5.0, 5.0, 5.0], [1.0, 0.0, 0.0], [1.0; 4]),
        ];
        generate_tangents(&mut vertices, &[0, 1, 2]);
        assert_orthonormal(&vertices[3]);
    }

    /// [`mesh_assets::cube_geometry`](crate::mesh_assets::cube_geometry)'s `+X`
    /// face (planar, axis-aligned UV): the tangent generation is exactly analytic
    /// here (no degenerate branch is ever taken for a well-formed planar quad) —
    /// `T = (0,1,0)`, positive handedness, for all 4 face vertices.
    #[test]
    fn cube_plus_x_face_tangent_is_analytic() {
        let (vertices, _) = crate::mesh_assets::cube_geometry(2.0);
        for v in &vertices[0..4] {
            assert_orthonormal(v);
            assert!((v.tangent[0]).abs() < EPS, "expected Tx=0, got {:?}", v.tangent);
            assert!((v.tangent[1] - 1.0).abs() < EPS, "expected Ty=1, got {:?}", v.tangent);
            assert!((v.tangent[2]).abs() < EPS, "expected Tz=0, got {:?}", v.tangent);
            assert!((v.tangent[3] - 1.0).abs() < EPS, "expected +1 handedness, got {}", v.tangent[3]);
        }
    }

    /// [`mesh_assets::plane_geometry`](crate::mesh_assets::plane_geometry) is one
    /// flat quad (`N = +Y`): every vertex must agree on the same tangent basis
    /// (`T` along `+X`, orthonormal to `N`) — the analytic-quad guarantee.
    #[test]
    fn plane_geometry_tangent_is_uniform_and_orthonormal() {
        let (vertices, _) = crate::mesh_assets::plane_geometry(4.0);
        for v in &vertices {
            assert_orthonormal(v);
            assert!((v.tangent[0] - 1.0).abs() < EPS, "expected Tx=1, got {:?}", v.tangent);
            assert!(v.tangent[1].abs() < EPS && v.tangent[2].abs() < EPS);
        }
    }
}
