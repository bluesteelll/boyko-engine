//! [`ObjMeshLoader`] — the Wavefront `.obj` mesh loader (asset-system rung
//! A3b).

use boyko_ecs::ecs::core::asset::{Asset, AssetError, AssetLoader};
use boyko_math::Vec3;

use crate::mesh::{MeshGpu, Vertex};
use crate::mesh_data::MeshData;
use crate::tangent::generate_tangents;

/// The neutral vertex color OBJ carries no channel for — the flat-shaded
/// gbuffer albedo lane [`Vertex::color`] defaults to.
const DEFAULT_VERTEX_COLOR: [f32; 4] = [0.8, 0.8, 0.8, 1.0];

/// Loads a Wavefront `.obj` mesh into a [`MeshData`] CPU intermediate — a
/// single streaming pass over the text.
///
/// # Grammar
///
/// `v x y z` (position), `vn x y z` (normal), `vt u v` (texcoord — carried into
/// the output [`Vertex::uv`]), `f ...` (a face: 3+ corners, each `v`, `v/vt`,
/// `v/vt/vn`, or `v//vn`). `o` / `g` / `usemtl` / `mtllib` / a blank / a `#`
/// comment / any other unrecognized leading token is skipped, not an error.
///
/// After dedup, [`generate_tangents`](crate::tangent::generate_tangents) runs once
/// over the whole mesh (a post-pass — tangent generation needs the full triangle
/// list). A `.obj` with no `vt` lines leaves every `uv` at `[0.0, 0.0]`, so every
/// tangent falls back to the arbitrary orthonormal case (harmless — unread
/// without a normal map).
///
/// Face-corner indices are 1-based OR negative-relative (O2): a negative
/// index resolves as `pool_len + idx`, where `pool_len` is the relevant pool's
/// (`v`/`vt`/`vn`) element count AS OF this face line in the single streaming
/// pass (not the file's final count).
///
/// A face with more than 3 corners is FAN-triangulated: corners `[0, k, k+1]`
/// for `k` in `1..len-1`. Each unique `(v, vt, vn)` corner triple dedups to
/// one output [`Vertex`] via an indirect sort on the raw (resolved) indices
/// (F-obj — no hashing: see [`dedup_corners`]).
///
/// # Limitations (documented, not bugs)
///
/// - Fan triangulation is correct only for a CONVEX, PLANAR polygon (the
///   common exporter case); a concave N-gon triangulates incorrectly.
/// - A face that omits `vn` for (any of) its corners falls back to ONE flat
///   normal per output triangle (an edge cross-product via [`Vec3::cross`]).
///   The dedup key still uses `-1` for a missing `vn`, so two DIFFERENT faces
///   sharing the exact same `(v, vt)` pair while both omitting `vn` alias
///   onto the SAME output vertex (keeping only the first triangle's flat
///   normal) — sound for the common case (a flat-shaded export gives every
///   face its own unique vertices, as does
///   [`MeshAssetsExt::cube`](crate::mesh_assets::MeshAssetsExt::cube)), not
///   for a smooth-shaded mesh that omits `vn` while sharing vertices across
///   faces with different normals.
pub struct ObjMeshLoader;

impl AssetLoader for ObjMeshLoader {
    type Out = MeshGpu;

    const EXTENSIONS: &'static [&'static str] = &["obj"];

    fn decode(bytes: &[u8]) -> Result<<Self::Out as Asset>::Cpu, AssetError> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| decode_error("obj file is not valid UTF-8".to_owned()))?;

        let mut positions: Vec<[f32; 3]> = Vec::new();
        let mut normals: Vec<[f32; 3]> = Vec::new();
        let mut uvs: Vec<[f32; 2]> = Vec::new();
        let mut corners: Vec<CornerRecord> = Vec::new();

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut fields = line.split_ascii_whitespace();
            let Some(tag) = fields.next() else { continue };
            let rest: Vec<&str> = fields.collect();

            match tag {
                "v" => positions.push(parse_floats3(&rest)?),
                "vn" => normals.push(parse_floats3(&rest)?),
                "vt" => uvs.push(parse_floats2(&rest)?),
                "f" => decode_face(&rest, &positions, &normals, &uvs, &mut corners)?,
                // "o" / "g" / "usemtl" / "mtllib" / unknown leading tokens: skipped.
                _ => {}
            }
        }

        if corners.is_empty() {
            return Err(decode_error("obj file has no faces".to_owned()));
        }

        let (mut vertices, indices) = dedup_corners(corners);
        generate_tangents(&mut vertices, &indices);
        Ok(MeshData { vertices, indices })
    }
}

/// One resolved OBJ face corner: 0-based indices into `positions` and,
/// optionally, `uvs` / `normals`.
#[derive(Clone, Copy)]
struct Corner {
    v: u32,
    vt: Option<u32>,
    vn: Option<u32>,
}

/// One face-corner's dedup key + realized [`Vertex`] payload, collected (in
/// face-corner emission order, across the whole file) by [`decode_face`]
/// ahead of the sort-dedup pass in [`dedup_corners`].
#[derive(Clone, Copy)]
struct CornerRecord {
    /// `(v, vt, vn)` with a missing `vt`/`vn` encoded as `-1` — the same key
    /// shape the retired `HashMap<(i32,i32,i32), u32>` dedup used.
    key: (i32, i32, i32),
    vertex: Vertex,
}

/// Resolves one `f ...` line's corners, fan-triangulates, and appends one
/// [`CornerRecord`] per output triangle-corner to `out` (deferring dedup to
/// [`dedup_corners`], run once after the whole file is streamed). See the
/// module doc's grammar / triangulation / limitations sections.
fn decode_face(
    fields: &[&str],
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    out: &mut Vec<CornerRecord>,
) -> Result<(), AssetError> {
    if fields.len() < 3 {
        return Err(decode_error(format!("a face needs at least 3 corners, found {}", fields.len())));
    }

    let face_corners = fields
        .iter()
        .map(|tok| parse_corner(tok, positions.len(), uvs.len(), normals.len()))
        .collect::<Result<Vec<Corner>, AssetError>>()?;

    // A face provides `vn` for every corner or none at all — the flat-normal
    // fallback computes one normal per output triangle when any corner omits it.
    let has_vn = face_corners.iter().all(|c| c.vn.is_some());

    for k in 1..face_corners.len() - 1 {
        let tri = [face_corners[0], face_corners[k], face_corners[k + 1]];
        let flat_normal = (!has_vn).then(|| {
            face_normal(positions[tri[0].v as usize], positions[tri[1].v as usize], positions[tri[2].v as usize])
        });

        for corner in tri {
            let key = (corner.v as i32, corner.vt.map_or(-1, |i| i as i32), corner.vn.map_or(-1, |i| i as i32));
            let normal = match corner.vn {
                Some(i) => normals[i as usize],
                None => flat_normal.expect("invariant: has_vn is false whenever a corner's vn is None"),
            };
            let uv = corner.vt.map_or([0.0, 0.0], |i| uvs[i as usize]);
            let mut vertex = Vertex::new(positions[corner.v as usize], normal, DEFAULT_VERTEX_COLOR);
            vertex.uv = uv;
            out.push(CornerRecord { key, vertex });
        }
    }
    Ok(())
}

/// Resolves the minimal unique vertex set from `corners` (collected in
/// face-corner emission order) via an indirect sort on the dedup key: sort a
/// permutation of corner indices by `key`, then walk the sorted runs of equal
/// keys, emitting one dense vertex per run and scattering its id back into an
/// index buffer sized to the original corner order.
///
/// Produces a vertex/index pair GEOMETRICALLY EQUIVALENT to the retired
/// `HashMap<(i32,i32,i32), u32>` corner-key dedup — the same unique-vertex set
/// and topology (one vertex per distinct corner key) — though the vertex
/// NUMBERING (and thus the integer values in the index buffer) differs: this
/// numbers vertices in sorted-key order, the `HashMap` numbered them in
/// first-emission order. Nothing downstream depends on absolute vertex
/// numbering, so the two meshes are interchangeable. First-emission still wins
/// on a key collision (matching `HashMap::entry(..).or_insert_with(..)`, which
/// never overwrites an existing key — see the module doc's flat-normal-fallback
/// limitation for why that matters) — with an `O(n log n)` sort replacing
/// hashing (F-obj). This is a cold, one-shot decode path; the sort also yields
/// the smallest possible vertex buffer.
fn dedup_corners(corners: Vec<CornerRecord>) -> (Vec<Vertex>, Vec<u32>) {
    let mut order: Vec<u32> = (0..corners.len() as u32).collect();
    order.sort_unstable_by_key(|&i| corners[i as usize].key);

    let mut vertices: Vec<Vertex> = Vec::with_capacity(corners.len());
    let mut indices: Vec<u32> = vec![0; corners.len()];

    let mut i = 0;
    while i < order.len() {
        let key = corners[order[i] as usize].key;
        let run_start = i;
        i += 1;
        while i < order.len() && corners[order[i] as usize].key == key {
            i += 1;
        }

        let run = &order[run_start..i];
        // First-emission wins on a key collision, matching the retired
        // HashMap's `or_insert_with` (never overwrites an existing entry).
        let first = *run.iter().min().expect("invariant: a sort run is never empty");
        let new_id = vertices.len() as u32;
        vertices.push(corners[first as usize].vertex);
        for &orig in run {
            indices[orig as usize] = new_id;
        }
    }

    (vertices, indices)
}

/// Parses one face-corner token (`v`, `v/vt`, `v/vt/vn`, or `v//vn`),
/// resolving each 1-based / negative-relative index to 0-based (O2 — see the
/// module doc's grammar section for the exact rule).
fn parse_corner(token: &str, position_count: usize, uv_count: usize, normal_count: usize) -> Result<Corner, AssetError> {
    let mut parts = token.split('/');
    let v_raw = parts.next().filter(|s| !s.is_empty()).ok_or_else(|| malformed_corner(token))?;
    let vt_raw = parts.next();
    let vn_raw = parts.next();
    if parts.next().is_some() {
        return Err(malformed_corner(token));
    }

    let v = resolve_index(v_raw, position_count).ok_or_else(|| out_of_range_corner(token))?;
    let vt = match vt_raw {
        Some(raw) if !raw.is_empty() => {
            Some(resolve_index(raw, uv_count).ok_or_else(|| out_of_range_corner(token))?)
        }
        _ => None,
    };
    let vn = match vn_raw {
        Some(raw) if !raw.is_empty() => {
            Some(resolve_index(raw, normal_count).ok_or_else(|| out_of_range_corner(token))?)
        }
        _ => None,
    };

    Ok(Corner { v, vt, vn })
}

/// Resolves a 1-based OR negative-relative OBJ index against a pool of
/// `count` elements (O2: a negative index resolves as `count + idx`,
/// 0-based). Returns `None` if `raw` does not parse as an integer, or the
/// resolved 0-based index falls outside `0..count`.
#[inline]
fn resolve_index(raw: &str, count: usize) -> Option<u32> {
    let idx: i64 = raw.parse().ok()?;
    let resolved = if idx < 0 { count as i64 + idx } else { idx - 1 };
    if resolved < 0 || resolved as usize >= count {
        return None;
    }
    Some(resolved as u32)
}

/// A flat per-triangle normal (edge cross-product), used when a face
/// provides no `vn` for any corner.
#[inline]
fn face_normal(p0: [f32; 3], p1: [f32; 3], p2: [f32; 3]) -> [f32; 3] {
    let a = Vec3::new(p0[0], p0[1], p0[2]);
    let b = Vec3::new(p1[0], p1[1], p1[2]);
    let c = Vec3::new(p2[0], p2[1], p2[2]);
    let n = (b - a).cross(c - a).normalize();
    [n.x, n.y, n.z]
}

/// Parses exactly 3 whitespace-separated `f32`s (a `v` / `vn` line's payload).
fn parse_floats3(fields: &[&str]) -> Result<[f32; 3], AssetError> {
    if fields.len() < 3 {
        return Err(decode_error(format!("expected 3 floats, found {}", fields.len())));
    }
    Ok([parse_f32(fields[0])?, parse_f32(fields[1])?, parse_f32(fields[2])?])
}

/// Parses exactly 2 whitespace-separated `f32`s (a `vt` line's payload).
fn parse_floats2(fields: &[&str]) -> Result<[f32; 2], AssetError> {
    if fields.len() < 2 {
        return Err(decode_error(format!("expected 2 floats, found {}", fields.len())));
    }
    Ok([parse_f32(fields[0])?, parse_f32(fields[1])?])
}

#[inline]
fn parse_f32(field: &str) -> Result<f32, AssetError> {
    field.parse().map_err(|_| decode_error(format!("invalid float '{field}'")))
}

/// Builds an [`AssetError::Decode`] for a malformed `.obj` file — the
/// parser's sole error path.
#[cold]
#[inline(never)]
fn decode_error(msg: String) -> AssetError {
    AssetError::Decode(msg)
}

#[cold]
#[inline(never)]
fn malformed_corner(token: &str) -> AssetError {
    decode_error(format!("malformed face corner '{token}'"))
}

#[cold]
#[inline(never)]
fn out_of_range_corner(token: &str) -> AssetError {
    decode_error(format!("index out of range in face corner '{token}'"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    type Bits3 = (u32, u32, u32);

    fn bits3(v: [f32; 3]) -> Bits3 {
        (v[0].to_bits(), v[1].to_bits(), v[2].to_bits())
    }

    /// Extracts the decoded mesh's UNORDERED triangle set: each triangle is a
    /// sorted `[(position, normal); 3]` bit-pattern triple, so dedup / vertex
    /// / triangle array order does not affect equality — only the GEOMETRY
    /// (which triangles, with which normals, exist) is compared.
    fn triangle_set(mesh: &MeshData) -> HashSet<[(Bits3, Bits3); 3]> {
        mesh.indices
            .chunks_exact(3)
            .map(|tri| {
                let mut verts: [(Bits3, Bits3); 3] = std::array::from_fn(|i| {
                    let v = mesh.vertices[tri[i] as usize];
                    (bits3(v.position), bits3(v.normal))
                });
                verts.sort_unstable();
                verts
            })
            .collect()
    }

    /// A well-formed unit cube (`v` + `vn` + `f v//vn` quads) decodes to a
    /// triangle SET geometry-equivalent to `MeshAssetsExt::cube`'s own face
    /// table (mirrored here — same per-face outward normal + 4-corner data,
    /// same `(0,1,2)`+`(0,2,3)` triangulation the fan rule produces for a
    /// quad).
    #[test]
    fn decode_unit_cube_is_geometry_equivalent_to_mesh_assets_cube() {
        let h = 0.5f32;
        let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
            ([1.0, 0.0, 0.0], [[h, -h, -h], [h, h, -h], [h, h, h], [h, -h, h]]),
            ([-1.0, 0.0, 0.0], [[-h, -h, h], [-h, h, h], [-h, h, -h], [-h, -h, -h]]),
            ([0.0, 1.0, 0.0], [[-h, h, -h], [-h, h, h], [h, h, h], [h, h, -h]]),
            ([0.0, -1.0, 0.0], [[-h, -h, h], [-h, -h, -h], [h, -h, -h], [h, -h, h]]),
            ([0.0, 0.0, 1.0], [[-h, -h, h], [h, -h, h], [h, h, h], [-h, h, h]]),
            ([0.0, 0.0, -1.0], [[h, -h, -h], [-h, -h, -h], [-h, h, -h], [h, h, -h]]),
        ];

        let mut obj = String::new();
        for (_, corners) in &faces {
            for c in corners {
                obj.push_str(&format!("v {} {} {}\n", c[0], c[1], c[2]));
            }
        }
        for (normal, _) in &faces {
            obj.push_str(&format!("vn {} {} {}\n", normal[0], normal[1], normal[2]));
        }
        for (f, _) in faces.iter().enumerate() {
            let base = f * 4 + 1; // 1-based vertex index
            let n = f + 1; // 1-based normal index
            obj.push_str(&format!("f {base}//{n} {}//{n} {}//{n} {}//{n}\n", base + 1, base + 2, base + 3));
        }

        let mesh = ObjMeshLoader::decode(obj.as_bytes()).expect("a well-formed cube obj must decode");

        let mut expected = HashSet::new();
        for (normal, corners) in &faces {
            for &(a, b, c) in &[(0usize, 1usize, 2usize), (0, 2, 3)] {
                let mut tri = [
                    (bits3(corners[a]), bits3(*normal)),
                    (bits3(corners[b]), bits3(*normal)),
                    (bits3(corners[c]), bits3(*normal)),
                ];
                tri.sort_unstable();
                expected.insert(tri);
            }
        }

        assert_eq!(mesh.vertices.len(), 24, "24 unique (position, normal) corners, matching cube()'s dedup shape");
        assert_eq!(mesh.indices.len(), 36, "6 faces * 2 triangles * 3 indices");
        assert_eq!(triangle_set(&mesh), expected, "decoded triangle geometry must match MeshAssetsExt::cube()'s data");
    }

    /// A single quad face fan-triangulates into exactly 2 triangles.
    #[test]
    fn decode_quad_face_triangulates_to_two_triangles() {
        let obj = "\
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
vn 0 0 1
f 1//1 2//1 3//1 4//1
";
        let mesh = ObjMeshLoader::decode(obj.as_bytes()).expect("a quad face must decode");
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices.len(), 6, "one quad fan-triangulates to 2 triangles (6 indices)");
    }

    /// Both `f v//vn` and `f v/vt/vn` corner forms parse (a corner with no `vt`
    /// leaves `Vertex::uv` at `[0.0, 0.0]`; neither form breaks decode).
    #[test]
    fn decode_accepts_v_slash_slash_vn_and_v_slash_vt_slash_vn_forms() {
        let obj = "\
v 0 0 0
v 1 0 0
v 0 1 0
vt 0 0
vt 1 0
vt 0 1
vn 0 0 1
f 1//1 2//1 3//1
f 1/1/1 2/2/1 3/3/1
";
        let mesh = ObjMeshLoader::decode(obj.as_bytes()).expect("both corner forms must decode");
        assert_eq!(mesh.indices.len(), 6, "two triangular faces = 6 indices");
    }

    /// A negative face index resolves relative to the pool size at that
    /// point in the stream (O2): `-1` on the last-declared vertex.
    #[test]
    fn decode_negative_index_resolves_relative_to_current_pool() {
        let obj = "\
v 0 0 0
v 1 0 0
v 0 1 0
f -3 -2 -1
";
        let mesh = ObjMeshLoader::decode(obj.as_bytes()).expect("a negative-indexed face must decode");
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.indices.len(), 3);
        let positions: Vec<[f32; 3]> = mesh.indices.iter().map(|&i| mesh.vertices[i as usize].position).collect();
        assert_eq!(positions, vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
    }

    /// A face with no `vn` at all falls back to a flat per-triangle normal.
    #[test]
    fn decode_missing_vn_computes_a_flat_normal() {
        let obj = "\
v 0 0 0
v 1 0 0
v 0 1 0
f 1 2 3
";
        let mesh = ObjMeshLoader::decode(obj.as_bytes()).expect("a face without vn must still decode");
        assert_eq!(mesh.vertices.len(), 3);
        for v in &mesh.vertices {
            // (1,0,0)-(0,0,0) cross (0,1,0)-(0,0,0) = +Z, already unit length.
            assert!((v.normal[2] - 1.0).abs() < 1e-6, "expected a +Z flat normal, got {:?}", v.normal);
            assert!(v.normal[0].abs() < 1e-6 && v.normal[1].abs() < 1e-6);
        }
    }

    /// Two vn-less faces share corners (same `v`, no `vt`/`vn`) but yield
    /// DIFFERENT flat normals; the deduped shared vertex must keep the
    /// FIRST-emitted face's normal. This locks `dedup_corners`'s `.min()`
    /// first-emission-wins semantics — the one behavior a `run[0]`/`.max()`
    /// regression could silently break (the retired `HashMap::or_insert_with`
    /// never overwrote, so first-seen won). Every other decode test has
    /// identical payloads on colliding keys, so only this one distinguishes
    /// first- from last-emission selection.
    #[test]
    fn decode_flat_normal_collision_keeps_the_first_emitted_face_normal() {
        // Triangle A (emitted first): v1,v2,v3 -> flat normal +Z.
        // Triangle B (second):        v1,v3,v4 -> flat normal +X.
        // v1 (0,0,0) and v3 (0,1,0) are shared corners; both must keep A's +Z.
        let obj = "\
v 0 0 0
v 1 0 0
v 0 1 0
v 0 0 1
f 1 2 3
f 1 3 4
";
        let mesh = ObjMeshLoader::decode(obj.as_bytes()).expect("must decode");
        assert_eq!(mesh.vertices.len(), 4, "v1,v2,v3,v4 are 4 distinct corner keys");
        let normal_at = |pos: [f32; 3]| {
            mesh.vertices
                .iter()
                .find(|v| v.position == pos)
                .unwrap_or_else(|| panic!("vertex at {pos:?} must be present"))
                .normal
        };
        // Shared v1 and v3 keep triangle A's +Z (first-emission wins), NOT B's +X.
        for shared in [[0.0f32, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            let n = normal_at(shared);
            assert!(
                (n[2] - 1.0).abs() < 1e-6 && n[0].abs() < 1e-6 && n[1].abs() < 1e-6,
                "shared corner {shared:?} must keep the first-emitted face A's +Z normal, got {n:?}"
            );
        }
        // v4 (only in face B) carries B's own +X flat normal.
        let n4 = normal_at([0.0, 0.0, 1.0]);
        assert!((n4[0] - 1.0).abs() < 1e-6, "v4 (B-only) carries face B's +X normal, got {n4:?}");
    }

    /// An empty file has no faces — `Decode`.
    #[test]
    fn decode_empty_file_is_decode_error() {
        let result = ObjMeshLoader::decode(b"");
        assert!(matches!(result, Err(AssetError::Decode(_))), "got {result:?}");
    }

    /// Vertices with no `f` lines — `Decode` (no faces).
    #[test]
    fn decode_no_faces_is_decode_error() {
        let result = ObjMeshLoader::decode(b"v 0 0 0\nv 1 0 0\nv 0 1 0\n");
        assert!(matches!(result, Err(AssetError::Decode(_))), "got {result:?}");
    }

    /// A face index outside the declared vertex pool — `Decode`.
    #[test]
    fn decode_out_of_range_index_is_decode_error() {
        let result = ObjMeshLoader::decode(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 9\n");
        assert!(matches!(result, Err(AssetError::Decode(_))), "got {result:?}");
    }
}
