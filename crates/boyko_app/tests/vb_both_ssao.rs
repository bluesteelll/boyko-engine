//! VB-SV0 DP6 rung DP6-0 (`docs/VB-SV0-DP6-DESIGN.md`, N2) — the **split** `VisibilityBuffer ×
//! Both` fixture with a NON-EMPTY SDF edit list.
//!
//! # Why this file exists
//!
//! DP6's cost table has four cells and two boot classes, and until this file only one of them was
//! bootable:
//!
//! | boot class | producer chain | fixture before this file |
//! |---|---|---|
//! | **fused** | `vb_resolve` (+ the dedicated `sdf_mesh_shadow` prepass when SV0 arms) | [`vb_both_sdf`](vb_both_sdf.rs) |
//! | **already split** | `vb_geo` → SSAO → à-trous → `vb_shade_split` (+ the same prepass) | **none** |
//!
//! `[vb_mesh_ssao]` is the split fixture, but it is `VisibilityBuffer × Mesh` — no SDF leg, so
//! `SDF_SOFT_MARCH` never arms and SV0 can never arm on it either (design Q5). `[vb_both_sdf]` has
//! the SDF leg and a non-empty edit list, but arms no pre-light consumer, so it resolves
//! `mesh_geo_shade_split == false` and is fused. The split-and-SDF-armed combination — the boot
//! class DP6 changes most — had no boot at all.
//!
//! This fixture is that boot: `[vb_both_sdf]`'s scene verbatim (so the edit list is the SAME
//! `count == 1` selection the S1 oracle measures, and the frames are comparable across boot
//! classes) plus `SsaoConfig::High`, which is what arms the split.
//!
//! # UNPINNED, deliberately, and DP6c is what pins it
//!
//! There is no `[vb_both_ssao]` row in `goldens/PINS.toml` yet. DP6-0 mints the *instrument*
//! (`ZONE_VB_GEO`) and records baselines on an unmodified producer; the frame this fixture renders
//! does not move at this rung, so a pin taken here would pin a byte string nothing in this rung can
//! change. DP6c — where the producer moves and the frame's byte-identity becomes the claim — adds
//! the pin.
//!
//! # The `BOYKO_SDF_MESH` arm
//!
//! Verbatim from [`vb_both_sdf`](vb_both_sdf.rs): unset ⇒ both request bits stay false ⇒ SV0
//! disarmed (**arm C**, the production split boot); `on`/`shadow`/`ao` arm the request on this
//! exact scene, which is what makes the split+SV0-armed baseline cell measurable at all.
//!
//! Windowed-test conventions (mirrors `vb_both_sdf.rs`): `#[ignore]` (needs a real windowed GPU
//! device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`.
//! `BOYKO_HOST_DUMP=<path.bmp>` arms the `boyko_app::host_dump` screenshot capture;
//! `BOYKO_VB_ZONE=1` + `BOYKO_PROFILE_ARTIFACT=<path.toml>` arm the measurement channel this rung
//! reads.

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::Material;
use boyko_render::{
    GeometryLegs, MeshAssetsVbExt, MeshGeometryTableSlot, RenderPath, RenderPathConfig, SsaoConfig,
    SsaoQuality,
};

mod sv0_scene;

/// `vb_both_sdf.rs::setup`, verbatim — the five-sphere row, the four flat-colour materials, the
/// sun/sky, the camera and [`sv0_scene::spawn_sdf_body`]'s single occluder.
///
/// Copied rather than shared because the delta between this fixture and `[vb_both_sdf]` must be
/// EXACTLY the `SsaoConfig` insert below: a shared setup that later grew a knob would change both
/// boot classes at once and the paired cost table would stop comparing two boots of one scene.
/// The scene itself is already shared — through [`sv0_scene`], which is where the parts that must
/// not drift live.
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

    let materials_row: [Option<u16>; sv0_scene::MESH_ROW_COUNT] = [
        None,
        Some(red.index() as u16),
        Some(green.index() as u16),
        Some(gold.index() as u16),
        Some(blue.index() as u16),
    ];

    sv0_scene::spawn_scene(&mut commands, sphere, &materials_row);
}

/// **The `VisibilityBuffer × Both` + SSAO + non-empty-SDF boot** (DP6-0's split baseline cell).
///
/// `SsaoConfig::High` arms `mesh_geo_shade_split` at the boot resolver, so the frame runs
/// `vb_geo → ssao(VB_THIN) → à-trous×3 → vb_shade_split` — and with `BOYKO_SDF_MESH` set, the
/// dedicated `sdf_mesh_shadow` prepass runs beside them. That is the *"today, split"* row of DP6's
/// cost table: `ZONE_VB_GEO + ZONE_VB_SHADE + ZONE_VB_SDF_MESH`.
///
/// `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1` and
/// `--test-threads=1`.
#[test]
#[ignore = "needs a real windowed GPU device; the orchestrator runs it on the GPU for the DP6-0 split baseline cells"]
fn vb_both_ssao_screenshot_dump() {
    let mut app = App::new();
    let plugins = EnginePlugins::window(
        "boyko_engine vb both ssao",
        sv0_scene::DUMP_EXTENT,
        sv0_scene::DUMP_EXTENT,
    );
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    // Both inserted AFTER `add_plugins` (which installs `RenderPathPlugin`'s `Deferred` default) so
    // these owner overrides win — `vb_both_sdf.rs` / `vb_mesh_ssao.rs`'s own post-plugins pattern.
    app.insert_resource(RenderPathConfig {
        path: RenderPath::VisibilityBuffer,
        legs: GeometryLegs::Both,
    });
    // THE ONE DELTA against `[vb_both_sdf]`. `atrous_levels: 3` matches `[vb_mesh_ssao]`, so the
    // split chain this boots is the one that pin already renders and blesses.
    app.insert_resource(SsaoConfig { quality: SsaoQuality::High, atrous_levels: 3 });
    // The env-gated SV0 arm — `vb_both_sdf.rs`'s block verbatim. Unset ⇒ requests stay false ⇒ arm
    // C (the production split boot, SV0 disarmed); `on|shadow|ao` arms the request bits.
    {
        // DP6a: `host` is a FOURTH accepted value and is deliberately in neither pattern below —
        // it arms the boot-side `sdf_mesh_term_wanted` (read from the env at `runner`'s boot seam)
        // and leaves both REQUEST bits false, which is measurement arm B (SV0 variant bound, mode
        // 0). On THIS fixture the split is already armed by SSAO, so `host` changes only the
        // pipeline pick, never the leg class.
        let sdf_mesh = std::env::var("BOYKO_SDF_MESH").unwrap_or_default();
        app.insert_resource(boyko_render::LightingConfig {
            vb_sdf_mesh_shadow: matches!(sdf_mesh.as_str(), "on" | "shadow"),
            vb_sdf_mesh_ao: matches!(sdf_mesh.as_str(), "on" | "ao"),
            ..boyko_render::LightingConfig::default()
        });
    }
    app.run();
}
