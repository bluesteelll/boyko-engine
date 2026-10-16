//! **RD2 / TRANSPARENCY R0a — G17 and the glb material-import gates.**
//!
//! Every fixture is a `.glb` BUILT HERE (zero committed `.glb`). The material JSON blocks are
//! the glTF 2.0 specification's `material` example (the gold factors block) and the six KHR
//! extension READMEs' example blocks, copied verbatim with only factor values edited — so a
//! field name or a placement the parser's author misread would be misread symmetrically in the
//! parser and a hand-written fixture, but not in a block copied from the spec. Two deviations,
//! each forced: the `KHR_materials_ior` and `KHR_materials_specular` README blocks carry a
//! trailing comma (`"ior": 1.4,`), which is not JSON, so it is dropped; `alphaMode`,
//! `alphaCutoff` and `doubleSided` have no example block in the specification, so they are added
//! to the gold block with the property names the specification's "Alpha Coverage" and "Double
//! Sided" sections spell.
//!
//! G17 asserts on the SPAWNED ARCHETYPES — the route markers present on each entity — not on
//! parsed fields, through the real spawn body ([`spawn_glb_scene_with`]) over a host uploader
//! that records what it was given instead of touching a device.

use boyko_ecs::ecs::core::asset::Assets;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::{Commands, ResMut};

use boyko_render::loaders::GlbMeshLoader;
use boyko_render::loaders::glb::{GlbAlphaMode, GlbImage, GlbScene};
use boyko_render::loaders::glb_spawn::{
    GlbSpawnOptions, GlbSpawnReport, GlbUploader, spawn_glb_scene_with,
};
use boyko_render::{
    AlphaMasked, AlphaMode, BlendClass, ColorSpace, Material, MaterialRoute, Translucent, Vertex,
};
use boyko_scene::render_caps::{MaterialHandle, MeshHandle};

// ---------------------------------------------------------------------------------------------
// The container and the shared geometry.
// ---------------------------------------------------------------------------------------------

/// Assembles a `.glb` container around a JSON chunk and a BIN chunk.
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

/// The per-vertex alpha the textureless `MASK` primitive's `COLOR_0` carries.
const VERTEX_ALPHA: f32 = 0.25;

/// One CCW triangle (positions, normals), a 4-component `COLOR_0`, `u16` indices and an
/// 8-byte stand-in image, at the offsets [`accessors_json`] names.
fn bin() -> Vec<u8> {
    let mut b = Vec::new();
    for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        b.extend_from_slice(&v.to_le_bytes()); // 0..36 POSITION
    }
    for v in [0.0f32, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0] {
        b.extend_from_slice(&v.to_le_bytes()); // 36..72 NORMAL
    }
    for _ in 0..3 {
        for v in [1.0f32, 1.0, 1.0, VERTEX_ALPHA] {
            b.extend_from_slice(&v.to_le_bytes()); // 72..120 COLOR_0 (VEC4)
        }
    }
    for i in [0u16, 1, 2] {
        b.extend_from_slice(&i.to_le_bytes()); // 120..126 indices
    }
    b.extend_from_slice(&[0, 0]); // pad to 128
    b.extend_from_slice(b"\x89PNGfake"); // 128..136 the image bytes (never decoded here)
    b
}

fn accessors_json() -> &'static str {
    r#""accessors": [
    {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"},
    {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"},
    {"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC4"},
    {"bufferView": 3, "componentType": 5123, "count": 3, "type": "SCALAR"}
  ],
  "bufferViews": [
    {"buffer": 0, "byteOffset": 0, "byteLength": 36},
    {"buffer": 0, "byteOffset": 36, "byteLength": 36},
    {"buffer": 0, "byteOffset": 72, "byteLength": 48},
    {"buffer": 0, "byteOffset": 120, "byteLength": 6},
    {"buffer": 0, "byteOffset": 128, "byteLength": 8}
  ],
  "images": [{"bufferView": 4, "mimeType": "image/png"}],
  "textures": [{"source": 0}]"#
}

/// A document with one mesh whose primitive `k` draws `materials[k]`; `vertex_alpha` marks the
/// primitives that read the 4-component `COLOR_0`. `root_extra` is spliced at the root.
fn doc(materials: &[&str], vertex_alpha: &[bool], root_extra: &str) -> Vec<u8> {
    let prims: Vec<String> = (0..materials.len())
        .map(|k| {
            let colour = if vertex_alpha.get(k).copied().unwrap_or(false) { r#", "COLOR_0": 2"# } else { "" };
            format!(r#"{{"attributes": {{"POSITION": 0, "NORMAL": 1{colour}}}, "indices": 3, "material": {k}}}"#)
        })
        .collect();
    let json = format!(
        r#"{{
  "asset": {{"version": "2.0"}},
  "scenes": [{{"nodes": [0]}}],
  "nodes": [{{"mesh": 0}}],
  "meshes": [{{"primitives": [{}]}}],
  "materials": [{}],
  {}{root_extra}
}}"#,
        prims.join(", "),
        materials.join(", "),
        accessors_json()
    );
    glb(&json, &bin())
}

fn decode(bytes: &[u8]) -> GlbScene {
    GlbMeshLoader::decode_scene(bytes).expect("the fixture is a valid glb")
}

// ---------------------------------------------------------------------------------------------
// The material blocks.
// ---------------------------------------------------------------------------------------------

/// glTF 2.0 specification, "Metallic-Roughness Material" example (the gold material), factors
/// edited per material so each row is distinguishable.
fn gold(base: &str, extra: &str) -> String {
    format!(
        r#"{{
            "pbrMetallicRoughness": {{
                "baseColorFactor": [ {base} ],
                "metallicFactor": 1.0,
                "roughnessFactor": 0.0
            }}{extra}
        }}"#
    )
}

/// `KHR_materials_transmission` README example block (`transmissionFactor` edited).
const TRANSMISSION: &str = r#"{
    "extensions": {
       "KHR_materials_transmission": {
         "transmissionFactor": 0.75,
         "transmissionTexture": {
           "index": 0
         }
       }
    }
  }"#;

/// `KHR_materials_volume` and `KHR_materials_dispersion` README example blocks, merged into one
/// `extensions` object (factors edited).
const VOLUME_DISPERSION: &str = r#"{
        "extensions": {
            "KHR_materials_volume": {
                "thicknessFactor": 2.0,
                "attenuationDistance":  0.006,
                "attenuationColor": [ 0.5, 0.25, 0.125 ]
            },
                "KHR_materials_dispersion": {
                    "dispersion": 0.1
                }
        }
    }"#;

/// `KHR_materials_diffuse_transmission` README example block (factors edited).
const DIFFUSE_TRANSMISSION: &str = r#"{
      "extensions": {
        "KHR_materials_diffuse_transmission": {
          "diffuseTransmissionFactor": 0.25,
          "diffuseTransmissionTexture": {
            "index": 0
          },
          "diffuseTransmissionColorFactor": [
            1.0,
            0.9,
            0.85
          ]
        }
      }
    }"#;

/// `KHR_materials_ior` and `KHR_materials_specular` README example blocks (factors edited; the
/// READMEs' trailing commas dropped — they are not JSON).
const IOR_SPECULAR: &str = r#"{
            "extensions": {
                "KHR_materials_ior": {
                    "ior": 1.4
                },
                "KHR_materials_specular": {
                    "specularFactor": 0.7,
                    "specularColorFactor": [0.9, 0.8, 0.7]
                }
            }
        }"#;

/// The G17 material set, in primitive order, with each one's expected route.
fn g17_materials() -> Vec<(String, MaterialRoute, &'static str)> {
    vec![
        (gold("1.000, 0.766, 0.336, 1.0", ""), MaterialRoute::Opaque, "OPAQUE (absent alphaMode)"),
        (
            gold("0.9, 0.1, 0.1, 1.0", r#", "alphaMode": "MASK", "alphaCutoff": 0.3"#),
            MaterialRoute::Masked,
            "MASK(0.3)",
        ),
        (gold("0.1, 0.9, 0.1, 0.5", r#", "alphaMode": "BLEND""#), MaterialRoute::Translucent, "BLEND"),
        (
            gold("0.1, 0.1, 0.9, 1.0", r#", "alphaMode": "OPAQUE", "doubleSided": true"#),
            MaterialRoute::Opaque,
            "OPAQUE doubleSided",
        ),
        (TRANSMISSION.to_string(), MaterialRoute::Translucent, "transmission > 0"),
        (r#"{ "alphaMode": "MASK" }"#.to_string(), MaterialRoute::Masked, "textureless MASK"),
        (VOLUME_DISPERSION.to_string(), MaterialRoute::Opaque, "volume + dispersion (no transmission)"),
        (DIFFUSE_TRANSMISSION.to_string(), MaterialRoute::Opaque, "diffuse transmission"),
        (IOR_SPECULAR.to_string(), MaterialRoute::Opaque, "ior + specular"),
    ]
}

/// Primitive 5 (the textureless `MASK`) reads the 4-component `COLOR_0`.
const VERTEX_ALPHA_PRIM: usize = 5;

fn g17_doc() -> Vec<u8> {
    let mats = g17_materials();
    let blocks: Vec<&str> = mats.iter().map(|(m, _, _)| m.as_str()).collect();
    let mut va = vec![false; blocks.len()];
    va[VERTEX_ALPHA_PRIM] = true;
    doc(&blocks, &va, "")
}

// ---------------------------------------------------------------------------------------------
// The host spawn harness.
// ---------------------------------------------------------------------------------------------

/// Records every upload; hands out mesh ids in upload order and a distinct slot per
/// (image, colour space): `100 + 2·image + (sRGB ? 0 : 1)`.
#[derive(Default)]
struct HostUploader {
    meshes: Vec<Vec<Vertex>>,
    images: Vec<(usize, ColorSpace)>,
}

fn host_slot(image: usize, space: ColorSpace) -> u32 {
    100 + 2 * image as u32 + u32::from(space != ColorSpace::Srgb)
}

impl GlbUploader for HostUploader {
    fn upload_mesh(&mut self, vertices: &[Vertex], _indices: &[u32]) -> MeshHandle {
        self.meshes.push(vertices.to_vec());
        MeshHandle(self.meshes.len() as u32 - 1)
    }

    fn upload_image(&mut self, image: &GlbImage, color_space: ColorSpace) -> Option<u32> {
        // The fixture has one image; its index is recovered from the stand-in bytes' position.
        assert_eq!(image.bytes, b"\x89PNGfake", "the uploader is handed the image's own bytes");
        self.images.push((0, color_space));
        Some(host_slot(0, color_space))
    }
}

/// One spawned drawable: its mesh id, material handle and route markers.
#[derive(Clone, Copy, Debug)]
struct Spawned {
    mesh: u32,
    material: u16,
    translucent: bool,
    masked: bool,
}

struct SpawnRun {
    report: GlbSpawnReport,
    uploader: HostUploader,
    spawned: Vec<Spawned>,
    ecs: EcsMaster,
}

fn spawn(scene: GlbScene) -> SpawnRun {
    let mut ecs = EcsMaster::new();
    let mut materials = Assets::<Material>::with_reserved(16);
    let default = materials.add(Material::default());
    materials.pin(default.index());
    ecs.insert_resource(materials);
    let _ = MeshHandle::component_id();
    let _ = MaterialHandle::component_id();

    let mut scene = Some(scene);
    let (report, uploader) = ecs.run_system(
        move |mut cmds: Commands, mut mats: ResMut<Assets<Material>>| -> (GlbSpawnReport, HostUploader) {
            let scene = scene.take().expect("the spawn system runs once");
            let mut up = HostUploader::default();
            let report = spawn_glb_scene_with(&scene, &mut cmds, &mut mats, &mut up, GlbSpawnOptions::default());
            (report, up)
        },
    );
    let spawned = ecs.run_system(
        |q: Query<(&MeshHandle, &MaterialHandle, Option<&Translucent>, Option<&AlphaMasked>)>| -> Vec<Spawned> {
            q.iter()
                .map(|(mesh, mat, t, m)| Spawned {
                    mesh: mesh.0,
                    material: mat.0,
                    translucent: t.is_some(),
                    masked: m.is_some(),
                })
                .collect()
        },
    );
    SpawnRun { report, uploader, spawned, ecs }
}

impl SpawnRun {
    /// The entity drawing primitive `k` (mesh ids are handed out in primitive order).
    fn part(&self, k: usize) -> Option<Spawned> {
        self.spawned.iter().copied().find(|s| s.mesh == k as u32)
    }

    fn material(&self, k: usize) -> Material {
        let s = self.part(k).unwrap_or_else(|| panic!("primitive {k} was not spawned"));
        *self
            .ecs
            .resource::<Assets<Material>>()
            .get_by_index(u32::from(s.material))
            .unwrap_or_else(|| panic!("primitive {k}'s MaterialHandle({}) resolves to no row", s.material))
    }
}

// ---------------------------------------------------------------------------------------------
// G17.
// ---------------------------------------------------------------------------------------------

/// G17: each primitive's entity carries exactly the route marker its glTF material implies —
/// `BLEND` and `transmission > 0` ⇒ `Translucent`, `MASK` ⇒ `AlphaMasked`, `OPAQUE` ⇒ neither.
#[test]
fn g17_the_spawned_archetypes_carry_the_route_markers() {
    let run = spawn(decode(&g17_doc()));
    let mats = g17_materials();

    // BLEND first: the defect this rung fixes is a BLEND primitive on the opaque rail.
    let blend = run.part(2);
    assert!(
        blend.is_some_and(|s| s.translucent && !s.masked),
        "RED: the BLEND primitive spawned without `Translucent` (spawned: {blend:?})"
    );
    for (k, (_, route, what)) in mats.iter().enumerate() {
        let s = run.part(k);
        let (want_t, want_m) = match route {
            MaterialRoute::Opaque => (false, false),
            MaterialRoute::Masked => (false, true),
            MaterialRoute::Translucent => (true, false),
        };
        assert!(
            s.is_some_and(|s| s.translucent == want_t && s.masked == want_m),
            "RED: primitive {k} ({what}) must spawn with Translucent={want_t}, AlphaMasked={want_m}; \
             spawned: {s:?}"
        );
    }
    assert_eq!(run.spawned.len(), mats.len(), "one entity per primitive");
    assert_eq!(run.report.entities, mats.len() as u32, "the report counts every entity");
    assert_eq!(
        (run.report.opaque, run.report.masked, run.report.translucent),
        (5, 2, 2),
        "the report's per-route counts match the spawned markers"
    );
}

/// G17: the parsed alpha cutoff, `doubleSided` and the KHR factors and slots reach the
/// `Material` row the entity references, and the vertex alpha reaches `Vertex.color.w`.
#[test]
fn g17_alpha_double_sided_and_khr_data_reach_the_material() {
    let run = spawn(decode(&g17_doc()));

    assert_eq!(run.material(0).alpha, AlphaMode::Opaque);
    assert!(!run.material(0).double_sided, "doubleSided defaults to false");
    assert_eq!(run.material(1).alpha, AlphaMode::Mask { cutoff: 0.3 }, "MASK keeps its alphaCutoff");
    assert_eq!(run.material(2).alpha, AlphaMode::Blend(BlendClass::PremulOver), "BLEND is premultiplied over");
    assert!(run.material(3).double_sided, "doubleSided: true reaches Material");
    assert_eq!(run.material(5).alpha, AlphaMode::Mask { cutoff: 0.5 }, "an absent alphaCutoff is 0.5");

    let t = run.material(4).x;
    assert_eq!(t.transmission, 0.75);
    assert_eq!(t.transmission_tex, host_slot(0, ColorSpace::Linear), "transmissionTexture is linear data");

    let v = run.material(6).x;
    assert_eq!((v.thickness, v.attenuation_distance), (2.0, 0.006));
    assert_eq!(v.attenuation_color, [0.5, 0.25, 0.125]);
    assert_eq!(v.dispersion, 0.1);
    assert_eq!(v.transmission, 0.0, "volume alone carries no transmission");

    let d = run.material(7).x;
    assert_eq!(d.diffuse_transmission, 0.25);
    assert_eq!(d.diffuse_transmission_color, [1.0, 0.9, 0.85]);
    assert_eq!(d.diffuse_transmission_tex, host_slot(0, ColorSpace::Linear), "dt strength is linear data");

    let s = run.material(8).x;
    assert_eq!((s.ior, s.specular), (1.4, 0.7));
    assert_eq!(s.specular_color, [0.9, 0.8, 0.7]);

    let verts = run.uploader.meshes.get(VERTEX_ALPHA_PRIM).expect("the vertex-alpha primitive was uploaded");
    assert!(
        verts.iter().all(|v| v.color[3] == VERTEX_ALPHA),
        "COLOR_0.a reaches Vertex.color.w: {:?}",
        verts.iter().map(|v| v.color).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------------------------
// Parse tests (GlbMaterial).
// ---------------------------------------------------------------------------------------------

/// Every new field at its glTF default when the document says nothing.
#[test]
fn parse_defaults_when_absent() {
    let scene = decode(&doc(&[&gold("1.0, 1.0, 1.0, 1.0", "")], &[], ""));
    let m = scene.materials[0];
    assert_eq!(m.alpha_mode, GlbAlphaMode::Opaque);
    assert_eq!(m.alpha_cutoff, 0.5, "alphaCutoff defaults to 0.5, not 0");
    assert!(!m.double_sided, "doubleSided defaults to false");
    let k = m.khr;
    assert_eq!((k.transmission, k.ior, k.thickness), (0.0, 1.5, 0.0));
    assert_eq!(k.attenuation_distance, f32::INFINITY);
    assert_eq!(k.attenuation_color, [1.0; 3]);
    assert_eq!((k.specular, k.specular_color), (1.0, [1.0; 3]));
    assert_eq!((k.dispersion, k.diffuse_transmission), (0.0, 0.0));
    assert_eq!(k.diffuse_transmission_color, [1.0; 3]);
    assert_eq!(
        (k.transmission_tex, k.thickness_tex, k.diffuse_transmission_tex, k.diffuse_transmission_color_tex),
        (None, None, None, None)
    );
}

/// `alphaMode`, `alphaCutoff` and `doubleSided` are read.
#[test]
fn parse_alpha_mode_cutoff_and_double_sided() {
    let scene = decode(&g17_doc());
    let m = &scene.materials;
    assert_eq!(m[1].alpha_mode, GlbAlphaMode::Mask, "MASK parsed");
    assert_eq!(m[1].alpha_cutoff, 0.3, "RED: alpha_cutoff 0.3 expected, got {}", m[1].alpha_cutoff);
    assert_eq!(m[2].alpha_mode, GlbAlphaMode::Blend, "BLEND parsed");
    assert_eq!(m[3].alpha_mode, GlbAlphaMode::Opaque, "explicit OPAQUE parsed");
    assert!(m[3].double_sided, "doubleSided parsed");
    assert_eq!(m[5].alpha_cutoff, 0.5, "MASK without alphaCutoff keeps the 0.5 default");
}

/// The KHR factors and texture references are read from each README block.
#[test]
fn parse_khr_factors_and_texture_slots() {
    let scene = decode(&g17_doc());
    let m = &scene.materials;
    assert_eq!(m[4].khr.transmission, 0.75, "RED: transmissionFactor not parsed");
    assert_eq!(m[4].khr.transmission_tex, Some(0), "transmissionTexture → image 0");
    assert_eq!((m[6].khr.thickness, m[6].khr.attenuation_distance), (2.0, 0.006));
    assert_eq!(m[6].khr.attenuation_color, [0.5, 0.25, 0.125]);
    assert_eq!(m[6].khr.dispersion, 0.1);
    assert_eq!(m[7].khr.diffuse_transmission, 0.25);
    assert_eq!(m[7].khr.diffuse_transmission_color, [1.0, 0.9, 0.85]);
    assert_eq!(m[7].khr.diffuse_transmission_tex, Some(0));
    assert_eq!((m[8].khr.ior, m[8].khr.specular), (1.4, 0.7));
    assert_eq!(m[8].khr.specular_color, [0.9, 0.8, 0.7]);
}

/// The two texture-only KHR slots (thickness, diffuse-transmission colour) are read.
#[test]
fn parse_thickness_and_dt_colour_textures() {
    let mat = r#"{
        "extensions": {
            "KHR_materials_volume": { "thicknessFactor": 1.0, "thicknessTexture": { "index": 0 } },
            "KHR_materials_diffuse_transmission": { "diffuseTransmissionColorTexture": { "index": 0 } }
        }
    }"#;
    let scene = decode(&doc(&[mat], &[], ""));
    assert_eq!(scene.materials[0].khr.thickness_tex, Some(0));
    assert_eq!(scene.materials[0].khr.diffuse_transmission_color_tex, Some(0));
}

// ---------------------------------------------------------------------------------------------
// extensionsRequired.
// ---------------------------------------------------------------------------------------------

const KHR_MATERIAL_NAMES: [&str; 6] = [
    "KHR_materials_transmission",
    "KHR_materials_ior",
    "KHR_materials_volume",
    "KHR_materials_specular",
    "KHR_materials_dispersion",
    "KHR_materials_diffuse_transmission",
];

/// Each of the six KHR material extensions may be REQUIRED: the decoder imports them, so a file
/// that requires one is decodable.
#[test]
fn extensions_required_accepts_the_six_khr_material_names() {
    for name in KHR_MATERIAL_NAMES {
        let extra = format!(r#", "extensionsUsed": ["{name}"], "extensionsRequired": ["{name}"]"#);
        let r = GlbMeshLoader::decode_scene(&doc(&[&gold("1.0, 1.0, 1.0, 1.0", "")], &[], &extra));
        assert!(r.is_ok(), "RED: {name} refused: {:?}", r.err());
    }
    let all = KHR_MATERIAL_NAMES.map(|n| format!("\"{n}\"")).join(", ");
    let extra = format!(r#", "extensionsRequired": [{all}]"#);
    assert!(
        GlbMeshLoader::decode_scene(&doc(&[&gold("1.0, 1.0, 1.0, 1.0", "")], &[], &extra)).is_ok(),
        "RED: the six names together refused"
    );
}

/// An extension the decoder does not implement stays a refusal, alone or beside an accepted one.
#[test]
fn extensions_required_still_refuses_an_unknown_extension() {
    for list in [r#"["KHR_draco_mesh_compression"]"#, r#"["KHR_materials_ior", "EXT_meshopt_compression"]"#] {
        let extra = format!(r#", "extensionsRequired": {list}"#);
        assert!(
            GlbMeshLoader::decode_scene(&doc(&[&gold("1.0, 1.0, 1.0, 1.0", "")], &[], &extra)).is_err(),
            "RED: {list} was accepted — the refusal of an unimplemented required extension is lost"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// The four counters (AD-21: counted, never silent) — each with its control.
// ---------------------------------------------------------------------------------------------

/// A KHR texture reference with `texCoord ≠ 0` is read as `texCoord` 0 and counted.
#[test]
fn counter_khr_texcoord1_ignored() {
    let with = |tc: u32| {
        format!(
            r#"{{ "extensions": {{ "KHR_materials_transmission": {{
                "transmissionFactor": 1.0, "transmissionTexture": {{ "index": 0, "texCoord": {tc} }} }} }} }}"#
        )
    };
    let run = spawn(decode(&doc(&[&with(1)], &[], "")));
    assert_eq!(run.report.khr_texcoord1_ignored, 1, "RED: a texCoord 1 KHR reference was not counted");
    assert_eq!(run.material(0).x.transmission_tex, host_slot(0, ColorSpace::Linear), "still bound, as texCoord 0");
    let control = spawn(decode(&doc(&[&with(0)], &[], "")));
    assert_eq!(control.report.khr_texcoord1_ignored, 0, "texCoord 0 is not counted");
}

/// `KHR_materials_specular`'s textures are not imported, and counted.
#[test]
fn counter_specular_textures_ignored() {
    let with = r#"{ "extensions": { "KHR_materials_specular": {
        "specularFactor": 0.5, "specularTexture": { "index": 0 }, "specularColorTexture": { "index": 0 } } } }"#;
    let run = spawn(decode(&doc(&[with], &[], "")));
    assert_eq!(run.report.specular_textures_ignored, 2, "RED: specular textures dropped silently");
    let control = spawn(decode(&doc(&[IOR_SPECULAR], &[], "")));
    assert_eq!(control.report.specular_textures_ignored, 0, "factors alone are not counted");
}

/// An image used both as sRGB colour and as linear data is uploaded once per space, counted.
#[test]
fn counter_dual_colour_space_images() {
    let dual = r#"{
        "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } },
        "extensions": { "KHR_materials_transmission": {
            "transmissionFactor": 1.0, "transmissionTexture": { "index": 0 } } }
    }"#;
    let run = spawn(decode(&doc(&[dual], &[], "")));
    assert_eq!(run.report.dual_colour_space_images, 1, "RED: a dual-use image was not counted");
    assert_eq!(run.report.images_uploaded, 2, "uploaded once per colour space");
    let m = run.material(0);
    assert_eq!(m.textures.albedo, host_slot(0, ColorSpace::Srgb), "base colour samples the sRGB upload");
    assert_eq!(m.x.transmission_tex, host_slot(0, ColorSpace::Linear), "transmission samples the linear upload");

    let control = spawn(decode(&doc(&[TRANSMISSION], &[], "")));
    assert_eq!(control.report.dual_colour_space_images, 0, "a single-space image is not counted");
    assert_eq!(control.report.images_uploaded, 1);
}

/// `dispersion > 0` without a volume (`thickness = 0`) is counted.
#[test]
fn counter_dispersion_without_volume() {
    let bare = r#"{ "extensions": { "KHR_materials_dispersion": { "dispersion": 0.1 } } }"#;
    let run = spawn(decode(&doc(&[bare], &[], "")));
    assert_eq!(run.report.dispersion_without_volume, 1, "RED: dispersion without a volume was not counted");
    let control = spawn(decode(&doc(&[VOLUME_DISPERSION], &[], "")));
    assert_eq!(control.report.dispersion_without_volume, 0, "dispersion inside a volume is not counted");
}

/// `route()` is the 09-10 §1.1 rule on the CPU row: diffuse transmission alone keeps the alpha
/// route; transmission makes any alpha mode translucent.
#[test]
fn route_rule_on_material_rows() {
    let mut m = Material::default();
    assert_eq!(m.route(), MaterialRoute::Opaque);
    m.x.diffuse_transmission = 1.0;
    assert_eq!(m.route(), MaterialRoute::Opaque, "diffuse transmission alone keeps the opaque route");
    m.alpha = AlphaMode::Mask { cutoff: 0.5 };
    assert_eq!(m.route(), MaterialRoute::Masked, "RED: MASK must route Masked");
    m.x.transmission = 0.01;
    assert_eq!(m.route(), MaterialRoute::Translucent, "transmission > 0 routes translucent over MASK");
    m.x.transmission = 0.0;
    m.alpha = AlphaMode::Blend(BlendClass::Additive);
    assert_eq!(m.route(), MaterialRoute::Translucent, "every blend class routes translucent");
}
