//! **VG-R0 rung R0b — the `.glb` decoder gate** (plan §3.3).
//!
//! Every fixture is BUILT BY THIS TEST rather than committed, so the corpus
//! payload (fetched + gitignored) is not a precondition for exercising the
//! decoder, and each refusal is provoked by changing exactly one thing about an
//! otherwise-valid file. That pairing is what makes a refusal a demonstration
//! rather than an assertion: the same bytes decode when the one field is legal.

use boyko_ecs::ecs::core::asset::AssetLoader;
use boyko_render::loaders::GlbMeshLoader;

/// Assembles a `.glb` container around a JSON chunk and a BIN chunk.
fn glb(json: &str, bin: &[u8]) -> Vec<u8> {
    let mut j = json.as_bytes().to_vec();
    while !j.len().is_multiple_of(4) {
        j.push(b' '); // JSON chunks pad with spaces, BIN with zeros.
    }
    let mut b = bin.to_vec();
    while !b.len().is_multiple_of(4) {
        b.push(0);
    }
    let total = 12 + 8 + j.len() + if b.is_empty() { 0 } else { 8 + b.len() };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&0x4674_6C67u32.to_le_bytes()); // "glTF"
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(j.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes()); // "JSON"
    out.extend_from_slice(&j);
    if !b.is_empty() {
        out.extend_from_slice(&(b.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004E_4942u32.to_le_bytes()); // "BIN\0"
        out.extend_from_slice(&b);
    }
    out
}

/// One triangle: three positions, three normals, three `u16` indices.
fn triangle_bin() -> Vec<u8> {
    let mut bin = Vec::new();
    let pos: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    for v in pos {
        bin.extend_from_slice(&v.to_le_bytes());
    }
    let nrm: [f32; 9] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
    for v in nrm {
        bin.extend_from_slice(&v.to_le_bytes());
    }
    for i in [0u16, 1, 2] {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    bin
}

/// Byte offsets inside [`triangle_bin`].
const POS_OFF: usize = 0;
const NRM_OFF: usize = 36;
const IDX_OFF: usize = 72;

/// A valid single-triangle document, with `extra` splice points for the mutations.
fn triangle_json(prim_extra: &str, root_extra: &str) -> String {
    format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scenes": [{{"nodes": [0]}}],
  "nodes": [{{"mesh": 0}}],
  "meshes": [{{"primitives": [{{
      "attributes": {{"POSITION": 0, "NORMAL": 1}},
      "indices": 2{prim_extra}
  }}]}}],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
    {{"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"}},
    {{"bufferView": 2, "componentType": 5123, "count": 3, "type": "SCALAR"}}
  ],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": {POS_OFF}, "byteLength": 36}},
    {{"buffer": 0, "byteOffset": {NRM_OFF}, "byteLength": 36}},
    {{"buffer": 0, "byteOffset": {IDX_OFF}, "byteLength": 6}}
  ],
  "buffers": [{{"byteLength": 78}}]{root_extra}
}}"#
    )
}

fn valid() -> Vec<u8> {
    glb(&triangle_json("", ""), &triangle_bin())
}

#[test]
fn a_minimal_indexed_triangle_decodes() {
    let mesh = GlbMeshLoader::decode(&valid()).expect("a legal one-primitive .glb must decode");
    assert_eq!(mesh.vertices.len(), 3, "three vertices");
    assert_eq!(mesh.indices, vec![0, 1, 2], "indices verbatim");
    assert_eq!(mesh.vertices[1].position, [1.0, 0.0, 0.0], "POSITION is read in order");
    assert_eq!(mesh.vertices[0].normal, [0.0, 0.0, 1.0], "NORMAL is read");
    // No COLOR_0 in the fixture: the neutral default, not zeros (which would be black).
    assert_eq!(mesh.vertices[0].color, [0.8, 0.8, 0.8, 1.0], "COLOR_0 default");
    // No TANGENT: the engine's post-pass ran, so the basis is not left at zero.
    assert!(
        mesh.vertices.iter().any(|v| v.tangent[..3] != [0.0, 0.0, 0.0]),
        "a missing TANGENT must run generate_tangents, not ship a zero basis"
    );
}

/// Each of these changes ONE thing about the file above and must be refused. The
/// pairing with [`a_minimal_indexed_triangle_decodes`] is the demonstration.
#[test]
fn every_out_of_subset_document_is_refused() {
    let bin = triangle_bin();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("non-triangle mode", glb(&triangle_json(r#", "mode": 1"#, ""), &bin)),
        ("morph targets", glb(&triangle_json(r#", "targets": [{"POSITION": 0}]"#, ""), &bin)),
        (
            "non-indexed primitive",
            glb(
                &triangle_json("", "").replace(r#""indices": 2"#, r#""indices_removed": 2"#),
                &bin,
            ),
        ),
        (
            "required extension (Draco/meshopt)",
            glb(&triangle_json("", r#", "extensionsRequired": ["KHR_draco_mesh_compression"]"#), &bin),
        ),
        ("animation", glb(&triangle_json("", r#", "animations": [{"channels": []}]"#), &bin)),
        ("skins", glb(&triangle_json("", r#", "skins": [{"joints": [0]}]"#), &bin)),
        (
            "sparse accessor",
            glb(
                &triangle_json("", "").replace(
                    r#"{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}"#,
                    r#"{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "sparse": {"count": 1}}"#,
                ),
                &bin,
            ),
        ),
        (
            "node with a non-identity translation",
            glb(
                &triangle_json("", "")
                    .replace(r#"{"mesh": 0}"#, r#"{"mesh": 0, "translation": [0, 5, 0]}"#),
                &bin,
            ),
        ),
        (
            "node with a non-identity matrix",
            glb(
                &triangle_json("", "").replace(
                    r#"{"mesh": 0}"#,
                    r#"{"mesh": 0, "matrix": [2,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1]}"#,
                ),
                &bin,
            ),
        ),
        (
            "a node hierarchy",
            glb(
                &triangle_json("", "")
                    .replace(r#""nodes": [{"mesh": 0}]"#, r#""nodes": [{"children": [1]}, {"mesh": 0}]"#),
                &bin,
            ),
        ),
        (
            "u8 indices",
            glb(
                &triangle_json("", "").replace(
                    r#"{"bufferView": 2, "componentType": 5123, "count": 3, "type": "SCALAR"}"#,
                    r#"{"bufferView": 2, "componentType": 5121, "count": 3, "type": "SCALAR"}"#,
                ),
                &bin,
            ),
        ),
        (
            "a missing NORMAL",
            glb(
                &triangle_json("", "").replace(r#", "NORMAL": 1"#, ""),
                &bin,
            ),
        ),
    ];

    for (what, bytes) in cases {
        let r = GlbMeshLoader::decode(&bytes);
        assert!(
            r.is_err(),
            "RED: `{what}` was ACCEPTED. §3.3's subset is a scope cut whose whole point is that \
             an unsupported document fails loudly — a partial mesh silently accepted is a census \
             measuring a different scene than the manifest describes."
        );
    }
}

/// A truncated or corrupt container must fail rather than read out of bounds.
#[test]
fn malformed_containers_are_refused_without_reading_out_of_bounds() {
    assert!(GlbMeshLoader::decode(b"").is_err(), "empty");
    assert!(GlbMeshLoader::decode(b"not a glb at all").is_err(), "bad magic");

    let mut wrong_version = valid();
    wrong_version[4..8].copy_from_slice(&1u32.to_le_bytes());
    assert!(GlbMeshLoader::decode(&wrong_version).is_err(), "container version 1");

    // A header claiming more bytes than the file holds.
    let mut overlong = valid();
    let n = overlong.len() as u32;
    overlong[8..12].copy_from_slice(&(n + 4096).to_le_bytes());
    assert!(GlbMeshLoader::decode(&overlong).is_err(), "over-declared length");

    // An accessor that reads past the BIN chunk.
    let past_end = glb(
        &triangle_json("", "").replace(r#""count": 3, "type": "VEC3"}"#, r#""count": 9999, "type": "VEC3"}"#),
        &triangle_bin(),
    );
    assert!(GlbMeshLoader::decode(&past_end).is_err(), "accessor past the BIN chunk");
}

/// The index range is validated against the vertex count, because an out-of-range
/// index is a GPU-side out-of-bounds read this engine cannot see (no
/// `robustBufferAccess`, validation off on the pins).
#[test]
fn an_out_of_range_index_is_refused() {
    let mut bin = triangle_bin();
    bin[IDX_OFF..IDX_OFF + 2].copy_from_slice(&7u16.to_le_bytes());
    let r = GlbMeshLoader::decode(&glb(&triangle_json("", ""), &bin));
    assert!(r.is_err(), "RED: an index of 7 into a 3-vertex mesh was accepted");
}
