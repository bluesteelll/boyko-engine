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
    out.extend_from_slice(b"glTF"); // magic, written as BYTES: a hex constant here once repeated the decoder's own typo
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(j.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
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
            "a scene graph reaching one node twice",
            glb(
                &triangle_json("", "").replace(
                    r#""nodes": [{"mesh": 0}]"#,
                    r#""nodes": [{"children": [2]}, {"children": [2]}, {"mesh": 0}]"#,
                )
                .replace(r#""scenes": [{"nodes": [0]}]"#, r#""scenes": [{"nodes": [0, 1]}]"#),
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

/// A node transform is BAKED, not refused. Rev 33 refused any non-identity TRS; every real
/// single-mesh asset carries one, so the restriction refused essentially all real content.
/// Ignoring a transform is the defect — applying it is the fix.
#[test]
fn a_node_transform_is_baked_into_model_space() {
    let bin = triangle_bin();

    // Pure translation: positions move, normals do not.
    let moved = GlbMeshLoader::decode(&glb(
        &triangle_json("", "").replace(r#"{"mesh": 0}"#, r#"{"mesh": 0, "translation": [0, 5, 0]}"#),
        &bin,
    ))
    .expect("a translated node must DECODE, not be refused");
    assert_eq!(moved.vertices[0].position, [0.0, 5.0, 0.0], "translation is applied");
    assert_eq!(moved.vertices[1].position, [1.0, 5.0, 0.0], "applied to every vertex");
    assert_eq!(moved.vertices[0].normal, [0.0, 0.0, 1.0], "a translation cannot rotate a normal");

    // Non-uniform scale: the normal takes the INVERSE TRANSPOSE, not the matrix. Scaling x by 2
    // and leaving z alone must leave a +Z normal at +Z, and stretch x positions.
    let scaled = GlbMeshLoader::decode(&glb(
        &triangle_json("", "").replace(r#"{"mesh": 0}"#, r#"{"mesh": 0, "scale": [2, 1, 1]}"#),
        &bin,
    ))
    .expect("a scaled node must decode");
    assert_eq!(scaled.vertices[1].position, [2.0, 0.0, 0.0], "scale is applied to positions");
    assert_eq!(scaled.vertices[0].normal, [0.0, 0.0, 1.0], "+Z normal survives an x-only scale");

    // An explicit matrix takes the same path.
    let m = GlbMeshLoader::decode(&glb(
        &triangle_json("", "").replace(
            r#"{"mesh": 0}"#,
            r#"{"mesh": 0, "matrix": [3,0,0,0, 0,3,0,0, 0,0,3,0, 1,2,3,1]}"#,
        ),
        &bin,
    ))
    .expect("an explicit matrix must decode");
    assert_eq!(m.vertices[0].position, [1.0, 2.0, 3.0], "column-major translation column");
    assert_eq!(m.vertices[1].position, [4.0, 2.0, 3.0], "uniform scale then translate");
    assert_eq!(m.vertices[0].normal, [0.0, 0.0, 1.0], "uniform scale leaves the normal direction");

    // And the identity path is untouched.
    let plain = GlbMeshLoader::decode(&valid()).expect("identity still decodes");
    assert_eq!(plain.vertices[1].position, [1.0, 0.0, 0.0]);
}

/// A 90-degree rotation about +X maps +Z to +Y. This is the case a naive "multiply the normal by
/// the matrix" would still get right, and it is here so the rotation path is not left unasserted.
#[test]
fn a_rotation_moves_both_positions_and_normals() {
    let s = (0.5f32).sqrt(); // sin(45) = cos(45); quaternion for a 90-degree X rotation
    let json = triangle_json("", "").replace(
        r#"{"mesh": 0}"#,
        &format!(r#"{{"mesh": 0, "rotation": [{s}, 0, 0, {s}]}}"#),
    );
    let m = GlbMeshLoader::decode(&glb(&json, &triangle_bin())).expect("a rotated node decodes");
    let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
    // (0,1,0) -> (0,0,1)
    assert!(near(m.vertices[2].position[2], 1.0), "+Y position rotates to +Z, got {:?}", m.vertices[2].position);
    // normal (0,0,1) -> (0,-1,0)
    assert!(near(m.vertices[0].normal[1], -1.0), "+Z normal rotates to -Y, got {:?}", m.vertices[0].normal);
}

/// **The decoder against REAL content**, not synthetic fixtures.
///
/// Decodes every `.glb` present under `assets/vg_corpus/` — the gitignored corpus payload — and
/// skips, naming itself, when none is there. This is the test that settled the Rev 33 restriction:
/// every real single-mesh asset probed carries a node transform, so "refuse any non-identity TRS"
/// refused essentially all real content, and only baking makes the corpus ingestible.
#[test]
fn every_real_corpus_glb_decodes() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/vg_corpus");
    let mut found = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "glb") {
                found.push(p);
            }
        }
    }

    if found.is_empty() {
        eprintln!(
            "SKIP: no .glb under assets/vg_corpus/ (the payload is gitignored). Run \
             scripts/fetch_corpus.ps1. The synthetic-fixture tests above ran and are unaffected."
        );
        return;
    }

    found.sort();
    for path in &found {
        let bytes = std::fs::read(path).expect("a listed .glb is readable");
        let name = path.file_name().unwrap().to_string_lossy();
        let mesh = GlbMeshLoader::decode(&bytes)
            .unwrap_or_else(|e| panic!("RED: real asset {name} failed to decode: {e:?}"));
        assert!(!mesh.vertices.is_empty(), "{name}: decoded zero vertices");
        assert_eq!(mesh.indices.len() % 3, 0, "{name}: index count is not a multiple of 3");
        assert!(
            mesh.indices.iter().all(|i| (*i as usize) < mesh.vertices.len()),
            "{name}: an index is out of range"
        );
        // A real asset must not come out of the decoder collapsed at the origin: that is what a
        // dropped or mis-transposed transform would look like.
        let spread = mesh
            .vertices
            .iter()
            .map(|v| v.position[0].abs() + v.position[1].abs() + v.position[2].abs())
            .fold(0.0f32, f32::max);
        assert!(spread > 0.0, "{name}: every vertex sits at the origin");
        eprintln!(
            "real asset {name}: {} vertices, {} triangles, max |pos| lane sum {spread:.3}",
            mesh.vertices.len(),
            mesh.indices.len() / 3
        );
    }
}

/// Two ROOT mesh nodes concatenate rather than being refused, each with its own transform baked.
///
/// This is the Rev 38 line: neither act places one mesh RELATIVE to another, so both are decoding.
/// Composing a parent transform with a child's does place them relative to one another, which is
/// why a hierarchy stays refused (asserted in the refusal list above).
#[test]
fn root_mesh_nodes_concatenate_with_each_transform_baked() {
    let json = triangle_json("", "")
        .replace(
            r#""nodes": [{"mesh": 0}]"#,
            r#""nodes": [{"mesh": 0}, {"mesh": 0, "translation": [10, 0, 0]}]"#,
        )
        // The SCENE decides what is rendered: a node the scene does not name is not in the
        // asset, and the decoder honours that rather than baking every mesh-bearing node it finds.
        .replace(r#""scenes": [{"nodes": [0]}]"#, r#""scenes": [{"nodes": [0, 1]}]"#);
    let m = GlbMeshLoader::decode(&glb(&json, &triangle_bin())).expect("two root nodes decode");
    assert_eq!(m.vertices.len(), 6, "both instances contribute their vertices");
    assert_eq!(m.indices.len(), 6, "and their triangles");
    assert_eq!(m.vertices[0].position, [0.0, 0.0, 0.0], "the first is untransformed");
    assert_eq!(m.vertices[3].position, [10.0, 0.0, 0.0], "the second carries its own translation");
    assert_eq!(
        &m.indices[3..],
        &[3, 4, 5],
        "the second instance's indices are OFFSET by the first's vertex count — un-offset indices          would silently re-draw the first triangle twice"
    );
}

/// A node hierarchy is COMPOSED, not refused: the parent's transform multiplies the child's, and
/// the result is still the geometry THIS FILE describes in its own space. Placing assets relative
/// to one another is the census's job; nothing here does it.
#[test]
fn a_node_hierarchy_composes_parent_and_child_transforms() {
    let json = triangle_json("", "")
        .replace(
            r#""nodes": [{"mesh": 0}]"#,
            r#""nodes": [{"children": [1], "translation": [100, 0, 0]}, {"mesh": 0, "translation": [0, 7, 0]}]"#,
        );
    let m = GlbMeshLoader::decode(&glb(&json, &triangle_bin())).expect("a hierarchy decodes");
    assert_eq!(m.vertices.len(), 3, "one mesh, reached once");
    assert_eq!(
        m.vertices[0].position,
        [100.0, 7.0, 0.0],
        "parent translation COMPOSED with the child's, not either one alone"
    );
    // Order matters: parent-then-child, not child-then-parent, is only visible under scale.
    let scaled = triangle_json("", "").replace(
        r#""nodes": [{"mesh": 0}]"#,
        r#""nodes": [{"children": [1], "scale": [2, 2, 2]}, {"mesh": 0, "translation": [3, 0, 0]}]"#,
    );
    let s = GlbMeshLoader::decode(&glb(&scaled, &triangle_bin())).expect("scaled hierarchy decodes");
    assert_eq!(
        s.vertices[0].position,
        [6.0, 0.0, 0.0],
        "the parent's scale must apply TO the child's translation (6, not 3)"
    );
}

/// A mesh node the SCENE does not name is not part of the asset, and must not be baked in.
#[test]
fn a_node_outside_the_scene_is_not_decoded() {
    let json = triangle_json("", "").replace(
        r#""nodes": [{"mesh": 0}]"#,
        r#""nodes": [{"mesh": 0}, {"mesh": 0, "translation": [10, 0, 0]}]"#,
    ); // scene still names only node 0
    let m = GlbMeshLoader::decode(&glb(&json, &triangle_bin())).expect("decodes");
    assert_eq!(
        m.vertices.len(),
        3,
        "only the scene's own node contributes — an off-scene node would silently add geometry          the manifest's published triangle count does not describe"
    );
}
