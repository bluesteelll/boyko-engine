//! **RD6 / D-DS — H5: a mirrored glb placement keeps the counter-clockwise front.**
//!
//! glTF 2.0: "the determinant of the node's global transform defines the winding order … if the
//! determinant is a positive value, the front-facing triangles use the counterclockwise order"
//! — so a node with a negative determinant authors CLOCKWISE fronts. The decoder bakes every
//! placement into model space and the engine renders counter-clockwise fronts, so a mirrored
//! placement must re-wind its triangles (swap each triangle's 2nd and 3rd index) and negate its
//! tangent handedness at bake. A positive-determinant placement changes nothing.
//!
//! Every fixture is built here (zero committed `.glb`): one triangle, counter-clockwise for its
//! `+Z` normal, with a `TANGENT` of handedness `+1`.

use boyko_ecs::ecs::core::asset::Assets;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::system::{Commands, ResMut};

use boyko_render::loaders::GlbMeshLoader;
use boyko_render::loaders::glb::{GlbImage, GlbScene};
use boyko_render::loaders::glb_spawn::{GlbSpawnOptions, GlbSpawnReport, GlbUploader, spawn_glb_scene_with};
use boyko_render::mesh_face::winding_census;
use boyko_render::{ColorSpace, Material, Vertex};
use boyko_scene::render_caps::{MaterialHandle, MeshHandle};

fn glb(json: &str, bin: &[u8]) -> Vec<u8> {
    let mut j = json.as_bytes().to_vec();
    while !j.len().is_multiple_of(4) {
        j.push(b' ');
    }
    let mut b = bin.to_vec();
    while !b.len().is_multiple_of(4) {
        b.push(0);
    }
    let total = 12 + 8 + j.len() + 8 + b.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(j.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&j);
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&b);
    out
}

/// One triangle: positions (counter-clockwise about `+Z`), `+Z` normals, `+X` tangents with
/// handedness `+1`, `u16` indices.
fn bin() -> Vec<u8> {
    let mut b = Vec::new();
    for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        b.extend_from_slice(&v.to_le_bytes()); // 0..36 POSITION
    }
    for _ in 0..3 {
        for v in [0.0f32, 0.0, 1.0] {
            b.extend_from_slice(&v.to_le_bytes()); // 36..72 NORMAL
        }
    }
    for _ in 0..3 {
        for v in [1.0f32, 0.0, 0.0, 1.0] {
            b.extend_from_slice(&v.to_le_bytes()); // 72..120 TANGENT
        }
    }
    for i in [0u16, 1, 2] {
        b.extend_from_slice(&i.to_le_bytes()); // 120..126 indices
    }
    b
}

/// The document, with `node` spliced as the one node's body.
fn doc(node: &str) -> Vec<u8> {
    let json = format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scenes": [{{"nodes": [0]}}],
  "nodes": [{{"mesh": 0{node}}}],
  "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0, "NORMAL": 1, "TANGENT": 2}}, "indices": 3}}]}}],
  "accessors": [
    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
    {{"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"}},
    {{"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC4"}},
    {{"bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR"}}
  ],
  "bufferViews": [
    {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
    {{"buffer": 0, "byteOffset": 36, "byteLength": 36}},
    {{"buffer": 0, "byteOffset": 72, "byteLength": 48}},
    {{"buffer": 0, "byteOffset": 120, "byteLength": 6}}
  ],
  "buffers": [{{"byteLength": 128}}]
}}"#
    );
    glb(&json, &bin())
}

fn decode(node: &str) -> GlbScene {
    GlbMeshLoader::decode_scene(&doc(node)).expect("the fixture is a valid glb")
}

/// H5: a `(−1, 1, 1)` node re-winds the baked triangle so it agrees with its (mirrored) normal,
/// negates the tangent handedness, and says so.
#[test]
fn a_mirrored_placement_is_rewound_with_its_handedness_negated() {
    let scene = decode(r#", "scale": [-1, 1, 1]"#);
    let part = &scene.parts[0];
    let census = winding_census(&part.mesh.vertices, &part.mesh.indices);
    assert_eq!(
        (census.agree, census.disagree),
        (1, 0),
        "RED: the mirrored bake must keep the counter-clockwise front (census {census:?}, indices {:?})",
        part.mesh.indices
    );
    assert_eq!(part.mesh.indices, vec![0, 2, 1], "the 2nd and 3rd index swap; the provoking vertex stays");
    assert!(
        part.mesh.vertices.iter().all(|v| v.tangent[3] == -1.0),
        "the tangent handedness is negated: {:?}",
        part.mesh.vertices.iter().map(|v| v.tangent).collect::<Vec<_>>()
    );
    assert!(part.mirrored_rewound, "the part records the re-wind");
}

/// H5's control: identity and rotation-only placements change no index and no handedness — the
/// decoded `MeshData` equals the frozen expectation.
#[test]
fn a_positive_determinant_placement_is_untouched() {
    for node in ["", r#", "rotation": [0, 0.7071068, 0, 0.7071068]"#, r#", "scale": [2, 3, 4]"#] {
        let scene = decode(node);
        let part = &scene.parts[0];
        assert_eq!(part.mesh.indices, vec![0, 1, 2], "node `{node}`: indices untouched");
        assert!(part.mesh.vertices.iter().all(|v| v.tangent[3] == 1.0), "node `{node}`: handedness untouched");
        assert!(!part.mirrored_rewound, "node `{node}`: not a mirror");
        let census = winding_census(&part.mesh.vertices, &part.mesh.indices);
        assert_eq!((census.agree, census.disagree), (1, 0), "node `{node}`: still counter-clockwise");
    }
    let identity = decode("");
    let expect = [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let got: Vec<[f32; 3]> = identity.parts[0].mesh.vertices.iter().map(|v| v.position).collect();
    assert_eq!(got, expect, "the identity placement's positions are the file's");
}

/// Records uploads only.
#[derive(Default)]
struct HostUploader {
    meshes: u32,
}

impl GlbUploader for HostUploader {
    fn upload_mesh(&mut self, _vertices: &[Vertex], _indices: &[u32]) -> MeshHandle {
        self.meshes += 1;
        MeshHandle(self.meshes - 1)
    }

    fn upload_image(&mut self, _image: &GlbImage, _color_space: ColorSpace) -> Option<u32> {
        None
    }
}

fn spawn(scene: GlbScene) -> GlbSpawnReport {
    let mut ecs = EcsMaster::new();
    let mut materials = Assets::<Material>::with_reserved(4);
    let default = materials.add(Material::default());
    materials.pin(default.index());
    ecs.insert_resource(materials);
    let _ = MeshHandle::component_id();
    let _ = MaterialHandle::component_id();
    let mut scene = Some(scene);
    ecs.run_system(move |mut cmds: Commands, mut mats: ResMut<Assets<Material>>| -> GlbSpawnReport {
        let scene = scene.take().expect("the spawn system runs once");
        spawn_glb_scene_with(&scene, &mut cmds, &mut mats, &mut HostUploader::default(), GlbSpawnOptions::default())
    })
}

/// H5: the spawn report counts the mirrored re-wind; the winding census counts a primitive that
/// still disagrees after bake (here: the authored triangle wound against its normals).
#[test]
fn the_spawn_report_counts_mirrors_and_disagreeing_primitives() {
    let report = spawn(decode(r#", "scale": [-1, 1, 1]"#));
    assert_eq!(report.mirrored_placements_rewound, 1, "RED: the mirrored placement was not counted");
    assert_eq!(report.winding_disagrees_primitives, 0, "the re-wound primitive agrees with its normals");

    let control = spawn(decode(""));
    assert_eq!((control.mirrored_placements_rewound, control.winding_disagrees_primitives), (0, 0));

    // A file whose normals point against its winding: flip the stored normals to −Z.
    let mut scene = decode("");
    for v in &mut scene.parts[0].mesh.vertices {
        v.normal = [0.0, 0.0, -1.0];
    }
    let report = spawn(scene);
    assert_eq!(report.winding_disagrees_primitives, 1, "RED: a clockwise-for-its-normals primitive was not counted");
}
