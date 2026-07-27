//! VB-SV0 plan, rung S1 — the TEXTURED `VisibilityBuffer × Both` fixture with a NON-EMPTY SDF
//! edit list.
//!
//! # Why a second fixture
//!
//! The later SV0 gates enumerate rows over the classified VB tails, and three of them are TEXTURED
//! rows. A textured row cannot be armed from the shipping textured pins: those are `legs: Mesh`,
//! where SV0 is structurally unarmable. Nothing in the tree was simultaneously textured, `Both`,
//! and carrying a non-empty SDF edit list — so those rows had no constructible fixture at all.
//! This file is it.
//!
//! # The scene
//!
//! [`vb_both_sdf.rs`](vb_both_sdf.rs)'s scene — the SAME five-sphere row, the SAME sun/sky, the
//! SAME camera, the SAME single SDF occluder, all from the shared [`sv0_scene`] module — with the
//! flat-colour materials swapped for the real textured material grafted from `vb_mesh_tex.rs`
//! (`load_material_folder` + [`Material::with_textures`], the only constructor that can set
//! `MATERIAL_FLAG_TEXTURED`). That flag OR-reduces over the mesh draws into `vb_tex_active`, which
//! auto-selects the classified `vb_shade`/`vb_shade_tex` pipeline — no `RenderPathConfig` field
//! and no env knob is involved, which is exactly why this is a clone-and-graft and not new
//! plumbing.
//!
//! # The ONE deliberate delta from `vb_mesh_tex.rs`'s material row
//!
//! `vb_mesh_tex.rs` leaves the MIDDLE sphere (index 2) untextured as a visible contrast baseline.
//! Here the middle sphere is the SDF body's anchor ([`sv0_scene::SDF_ANCHOR_INDEX`]) and therefore
//! the only sphere that carries shadowed and contact-AO pixels. Leaving it untextured would route
//! every one of those pixels through the plain `vb_shade` row and leave the `vb_shade_tex` row
//! carrying no SV0-affected pixel at all — a textured fixture whose textured half is vacuous, i.e.
//! the very defect the two-fixture split exists to prevent. The untextured contrast sphere is
//! therefore moved to the LAST index, which keeps both classified rows populated while putting the
//! SV0-affected pixels in the textured one.
//!
//! Nothing couples the material to the SDF gather (`collect_sdf_edits` is a pure
//! `Query<&SdfPrimitive>` walk) and the SDF surface cannot pick up this material (it reads
//! `base_color` only, and `SdfEdit::sphere` leaves material lane 0), so the swap changes the mesh
//! shading path and nothing else.
//!
//! Windowed-test conventions (mirrors `vb_mesh_tex.rs`): `#[ignore]` (needs a real windowed GPU
//! device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`.
//! `BOYKO_HOST_DUMP=<path.bmp>` arms the `boyko_app::host_dump` screenshot capture; see
//! `goldens/PINS.toml`'s `[vb_both_sdf_tex]` pin (UNBLESSED — seeded `PENDING`).

#![cfg(windows)]

use std::path::PathBuf;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::{
    BindlessTextureTable, GeometryLegs, Material, MaterialGpu, MeshAssetsVbExt,
    MeshGeometryTableSlot, RenderPath, RenderPathConfig, TextureGpu, load_material_folder,
};

mod sv0_scene;

/// The env var that overrides the default texture folder — `vb_mesh_tex.rs`'s
/// `BOYKO_PBR_TEXTURE_DIR` convention, kept so the two textured VB fixtures are driven the same
/// way.
const TEXTURE_DIR_ENV: &str = "BOYKO_PBR_TEXTURE_DIR";
/// The default texture folder — the committed `synth_bumps` oracle set, resolved relative to this
/// crate's manifest at compile time (repo-relative regardless of the test binary's working
/// directory). Same path `vb_mesh_tex.rs` compiles in.
const DEFAULT_TEXTURE_DIR: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pbr_fixtures/synth_bumps");

/// [`vb_both_sdf.rs`](vb_both_sdf.rs)'s scene with the textured material grafted in: the shared
/// [`sv0_scene`] row / sun / sky / camera / SDF body, and one `MATERIAL_FLAG_TEXTURED` material
/// shared by four instances (the multi-instance-same-material coverage the classified pipeline's
/// uniform-bindless-index invariant wants).
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut textures: NonSendResMut<Assets<TextureGpu>>,
    mut bindless: NonSendResMut<BindlessTextureTable>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    let (verts, idx) = sv0_scene::scene_sphere_mesh();
    let sphere = match geo_table.0.as_mut() {
        Some(table) => meshes.register_mesh_vb(dev.get(), &verts, &idx, table),
        None => meshes.register_mesh(dev.get(), &verts, &idx),
    };

    let texture_dir = std::env::var(TEXTURE_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_TEXTURE_DIR));
    println!("vb_both_sdf_tex: reading textures from {}", texture_dir.display());
    let material_textures =
        load_material_folder(&mut textures, dev.get(), &mut bindless, &texture_dir);
    // A silent fall back to slot 0 would still set MATERIAL_FLAG_TEXTURED, i.e. the classified row
    // would be selected while sampling the fallback texel — a textured pin that proves nothing.
    // Printing the resolved slots is what makes that visible in the dump run's log.
    println!(
        "vb_both_sdf_tex: resolved slots — albedo={} normal={} metal_rough={} ao={} emissive={} \
         (0 = fallback)",
        material_textures.albedo,
        material_textures.normal,
        material_textures.metal_rough,
        material_textures.ao,
        material_textures.emissive
    );
    let textured_mat = materials.add(Material::with_textures(
        MaterialGpu::new([1.0, 1.0, 1.0, 1.0], 0.0, 0.5, 0.5, [0.0; 3], 0),
        material_textures,
    ));

    // The untextured contrast sphere is the LAST one, NOT the middle one — see this file's doc for
    // why the SDF body's anchor sphere has to be textured.
    let textured = Some(textured_mat.index() as u16);
    let materials_row: [Option<u16>; sv0_scene::MESH_ROW_COUNT] =
        [textured, textured, textured, textured, None];
    debug_assert!(
        materials_row[sv0_scene::SDF_ANCHOR_INDEX].is_some(),
        "invariant: the SDF body's anchor sphere must carry the textured material, or every \
         SV0-affected pixel lands in the untextured classified row"
    );

    // ONE call spawns the row, the SDF occluder, the sun/sky and the camera — see
    // `sv0_scene::spawn_scene` for why this fixture cannot omit the body.
    sv0_scene::spawn_scene(&mut commands, sphere, &materials_row);
}

/// **The TEXTURED `VisibilityBuffer × Both` + non-empty-SDF golden dump** (rung S1's textured
/// fixture — the vehicle for the classified `vb_shade_tex` rows).
///
/// `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot and blesses `[vb_both_sdf_tex]` after
/// the owner's visual sign-off.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU to dump the TEXTURED VisibilityBuffer x Both + SDF screenshot"]
fn vb_both_sdf_tex_screenshot_dump() {
    let mut app = App::new();
    let plugins = EnginePlugins::window(
        "boyko_engine vb both sdf textured",
        sv0_scene::DUMP_EXTENT,
        sv0_scene::DUMP_EXTENT,
    );
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    // Requested AFTER `add_plugins` (which installs `RenderPathPlugin`'s `Deferred` default) so
    // this owner override wins — `vb_both.rs` / `vb_mesh_tex.rs`'s own post-plugins insert.
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Both });
    app.run();
}
