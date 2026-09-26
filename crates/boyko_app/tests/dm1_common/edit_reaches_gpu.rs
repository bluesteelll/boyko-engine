//! DM1 red-first test (1): an authored material edit made after boot reaches the GPU table on
//! every render path, for a mesh pixel AND an SDF pixel.
//!
//! The scene holds one cube (material `M`) and one SDF sphere (material `S`), both booted with a
//! GREEN emissive over a black base. At frame [`EDIT_AT`] a Main system sets both emissives to RED
//! through `Assets::get_mut` — the ordinary gameplay edit, nothing else. On a tree whose table
//! never re-uploads after boot (the defect RESEARCH §1.2 records: emissive is stale on every
//! path), both samples stay GREEN forever; with Tier 1 wired, both turn RED.
//!
//! **Premises** (a failure is a fixture bug, not a verdict): the boot resolved the asked path with
//! both legs on; both samples are GREEN and bit-stable over frames 6..=9 (so the sample is our
//! geometry and nothing else moves it); every frame a clause names was captured.
//!
//! **Verdict:** both samples are RED on every frame in [`VERDICT`].
//!
//! **The idle assert** (DM1 (c), critique W3), from the sidecar's recorder-site counts: frame 0
//! records ONE `material_upload` pass copying ONE full-image region; the edit frame records one pass
//! and at least one region; every other frame records NO pass and NO region — a pass declared with
//! an empty run list still counts, so a graph that paid the upload's barriers on an idle frame is
//! red here. The edit system runs before `VisibilitySet::Read`, so the stager (after the gather)
//! drains the edit on [`EDIT_AT`] itself rather than on whichever frame the executor's packing
//! chose.

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::asset::Handle;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::{GeometryLegs, Material, RenderPath};
use boyko_scene::VisibilitySet;

use super::{
    Capture, GREEN, Hue, RED, arm_dump, assert_ran, assert_resolved, build_app, cube_pixel,
    glow_material, sdf_pixel, spawn_cube, spawn_sdf_sphere, spawn_view,
};

/// The frame whose Main run makes the edit (`HostFrameStats::frames` == the sidecar's `frame=`).
pub const EDIT_AT: u64 = 10;
/// Frames the burst captures, from frame 0.
pub const FRAMES: u32 = 16;
/// The frames the "before" premise reads.
pub const BEFORE: core::ops::RangeInclusive<u32> = 6..=9;
/// The frames the verdict reads — two past the edit, so an implementation that lands the copy one
/// frame late still passes; one that never lands it cannot.
pub const VERDICT: core::ops::RangeInclusive<u32> = 12..=(FRAMES - 1);

const MESH_X: f32 = -1.2;
const SDF_X: f32 = 1.2;

/// The two material handles the edit targets, minted at startup.
#[derive(Resource, Default)]
pub struct EditRows(Option<(Handle<Material>, Handle<Material>)>);

fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
    mut rows: ResMut<EditRows>,
) {
    let cube = meshes.cube(dev.get(), super::CUBE);
    let m = materials.add(glow_material(GREEN));
    let s = materials.add(glow_material(GREEN));
    spawn_cube(&mut commands, cube, MESH_X, 0.0, m.index());
    spawn_sdf_sphere(&mut commands, SDF_X, 0.0, s.index());
    spawn_view(&mut commands);
    rows.0 = Some((m, s));
}

fn edit(stats: Res<HostFrameStats>, rows: Res<EditRows>, mut materials: ResMut<Assets<Material>>) {
    if stats.frames != EDIT_AT {
        return;
    }
    let (m, s) = rows.0.expect("invariant: setup minted the rows before the first frame");
    for h in [m, s] {
        let mat = materials.get_mut(h).expect("invariant: the edited row is live");
        mat.gpu.emissive = [RED[0], RED[1], RED[2], 0.0];
    }
}

/// Runs test (1) on `path` (both legs) and asserts the verdict.
pub fn run(test: &str, path: RenderPath) {
    let dump = arm_dump(test, FRAMES);
    let mut app = build_app("boyko_app DM1 edit reaches gpu", path, GeometryLegs::Both);
    app.insert_resource(EditRows::default());
    app.add_startup_system(setup);
    app.add_systems_cfg(|b| {
        b.add_system(edit).before_set(VisibilitySet::Read);
    });
    app.run();
    assert_ran(&app);
    assert_resolved(&app, path, true);

    let cap = Capture::load(&dump);
    let mesh = cube_pixel(MESH_X, 0.0);
    let sdf = sdf_pixel(SDF_X, 0.0);
    let mut failures = Vec::new();
    for (name, px) in [("mesh", mesh), ("sdf", sdf)] {
        let before = cap.assert_stable(name, px, BEFORE);
        assert_eq!(
            Hue::of(before),
            Hue::Green,
            "PREMISE: sample `{name}` at {px:?} is {before:?} before the edit, not the GREEN boot \
             emissive — the pixel is not the fixture's geometry{}",
            cap.trace(px, 0..=FRAMES - 1)
        );
        for k in VERDICT {
            let v = cap.frame(k).image.rgb(px);
            if Hue::of(v) != Hue::Red {
                failures.push(format!(
                    "{path:?} `{name}` at {px:?}: frame {k} is {v:?} ({:?}); the edit at frame \
                     {EDIT_AT} set RED",
                    Hue::of(v)
                ));
            }
        }
        eprintln!("DM1(1) {path:?} `{name}` at {px:?}:{}", cap.trace(px, 0..=FRAMES - 1));
    }
    for k in 0..FRAMES {
        let f = cap.frame(k);
        let count = |key: &str| -> u32 {
            f.field(key)
                .unwrap_or_else(|| panic!("PREMISE: the frame-{k} state line carries no `{key}=` (the runner's DM1 upload counters)"))
                .parse()
                .unwrap_or_else(|e| panic!("frame {k}: `{key}=` is not an integer ({e})"))
        };
        let (pass, regions, full) = (count("mat_pass"), count("mat_regions"), count("mat_full"));
        eprintln!("DM1(1) {path:?} frame {k}: mat_pass={pass} mat_regions={regions} mat_full={full}");
        let ok = match u64::from(k) {
            0 => (pass, regions, full) == (1, 1, 1),
            EDIT_AT => pass == 1 && regions >= 1 && full == 0,
            _ => (pass, regions) == (0, 0),
        };
        if !ok {
            failures.push(format!(
                "{path:?} frame {k}: mat_pass={pass} mat_regions={regions} mat_full={full} — frame 0 copies one \
                 full-image region, frame {EDIT_AT} the compact edit runs, and every other frame nothing"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "DM1 test (1) on {path:?}: {} failures — a sample that is not RED is an edit made through \
         Assets::get_mut that never reached the GPU table; a `mat_*` line is an upload recorded where \
         none was due, or missing where one was:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
