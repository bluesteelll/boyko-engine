//! VB-SV0 plan, rung S1 — the `VisibilityBuffer × Both` fixture with a NON-EMPTY SDF edit list.
//!
//! # Why this file exists
//!
//! Every VB golden shipped so far has an EMPTY SDF edit list — `goldens/PINS.toml`'s `[vb_both]`
//! ("boot-seeded EMPTY (count == 0)") and `[vb_sdf_only]` say so in as many words. SV0's shadow
//! and contact-AO terms are both exactly `1.0` on such a scene, so any byte-identity gate
//! quantified over those pins is VACUOUS: it would go green over an empty selection no matter what
//! SV0 did. This fixture is the non-empty selection, and the S1 oracle is what PROVES it non-empty
//! on the CPU before a line of shader is written.
//!
//! # The scene
//!
//! `vb_both.rs`'s five-sphere `grand_showcase_2mat` scene verbatim — same mesh generation, same
//! five materials, same sun/sky, same camera, same `RenderPathConfig{VisibilityBuffer, Both}` —
//! with ONE addition: a single SDF sphere on the segment from the CENTRE mesh sphere toward the
//! key light. The whole scene, including that body's placement and the derivation behind it, lives
//! in [`sv0_scene`] so the S1 oracle measures exactly what this test renders.
//!
//! The frame is therefore NOT expected to match `[vb_mesh]` / `[vb_both]`: under `legs: Both` a
//! non-empty edit list makes `sdf_forward_march` composite the SDF body's own pixels into `gLit`
//! independently of SV0. That difference is not evidence of anything by itself — it is why S1's
//! gate is a CPU coverage oracle and not "the frame differs".
//!
//! # Sibling
//!
//! [`vb_both_sdf_tex.rs`](vb_both_sdf_tex.rs) is the SAME scene with a textured material, which is
//! what makes the classified `vb_shade_tex` rows constructible for the later SV0 gates.
//!
//! Windowed-test conventions (mirrors `vb_both.rs`): `#[ignore]` (needs a real windowed GPU
//! device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`.
//! `BOYKO_HOST_DUMP=<path.bmp>` arms the `boyko_app::host_dump` screenshot capture; see
//! `goldens/PINS.toml`'s `[vb_both_sdf]` pin (UNBLESSED — seeded `PENDING`).

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::Material;
use boyko_render::{GeometryLegs, MeshAssetsVbExt, MeshGeometryTableSlot, RenderPath, RenderPathConfig};

mod sv0_scene;

/// `vb_both.rs::setup` with the S1 SDF body added — the five-sphere row (registered via
/// [`MeshAssetsVbExt::register_mesh_vb`], falling back to the plain
/// [`MeshAssetsExt::register_mesh`] when the geometry table is not armed), the four flat-colour
/// materials, the sun/sky, the camera, and [`sv0_scene::spawn_sdf_body`]'s single occluder.
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    let (verts, idx) = sv0_scene::scene_sphere_mesh();
    let sphere = match geo_table.0.as_mut() {
        Some(table) => meshes.register_mesh_vb(dev.get(), &verts, &idx, table),
        None => meshes.register_mesh(dev.get(), &verts, &idx),
    };

    let red = materials.add(Material::new([0.72, 0.04, 0.04, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let green = materials.add(Material::new([0.05, 0.46, 0.10, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let gold = materials.add(Material::new([1.0, 0.71, 0.29, 1.0], 1.0, 0.13, 0.5, [0.0; 3], 0));
    let blue = materials.add(Material::new([0.20, 0.38, 0.92, 1.0], 1.0, 0.42, 0.5, [0.0; 3], 0));

    let materials_row: [Option<u16>; sv0_scene::MESH_ROW_COUNT] =
        [None, Some(red.index() as u16), Some(green.index() as u16), Some(gold.index() as u16), Some(blue.index() as u16)];

    // ONE call spawns the row, the SDF occluder, the sun/sky and the camera — the S1 delta against
    // `vb_both.rs` (`collect_sdf_edits` gathers `count == 1` instead of the boot-seeded empty list
    // every other VB pin renders) is INSIDE it and cannot be dropped from here. That is deliberate:
    // a fixture able to omit its own body would leave all four S1 gates green over a pin rendering
    // the empty list. See `sv0_scene::spawn_scene`.
    sv0_scene::spawn_scene(&mut commands, sphere, &materials_row);
}

/// **The `VisibilityBuffer × Both` + non-empty-SDF golden dump** (rung S1's flat fixture).
///
/// `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1`; the
/// orchestrator runs it on the GPU to dump the screenshot and blesses `[vb_both_sdf]` after the
/// owner's visual sign-off.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU to dump the VisibilityBuffer x Both + SDF screenshot"]
fn vb_both_sdf_screenshot_dump() {
    let mut app = App::new();
    let plugins = EnginePlugins::window(
        "boyko_engine vb both sdf",
        sv0_scene::DUMP_EXTENT,
        sv0_scene::DUMP_EXTENT,
    );
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    // Requested AFTER `add_plugins` (which installs `RenderPathPlugin`'s `Deferred` default) so
    // this owner override wins — `vb_both.rs`'s own post-plugins insert, verbatim.
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Both });
    app.run();
}
