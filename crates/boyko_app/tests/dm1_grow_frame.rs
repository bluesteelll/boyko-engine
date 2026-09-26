//! DM1 test (5): **the grow frame** (`docs/render/DYNAMIC-MATERIALS-DESIGN-SPACE.md` §7 DM1
//! (a)(5), F3).
//!
//! On the frame a mint crosses the table's power-of-two capacity, the runner creates the grown
//! table and copies ONE full image into it — every row's current authority, built after this
//! frame's edits — instead of this frame's compact runs, which would leave every row the frame did
//! not touch unseeded in the new buffer.
//!
//! **The fixture** (`VisibilityBuffer × Mesh`; every material black-based, so the emissive lane —
//! table-only — carries the colour, critique W5):
//! - boot: the runner's default row 0 plus seven startup mints ⇒ high-water 8 = the boot capacity.
//!   Row 1 is BLUE and never touched again (critique W4: the row a compact-only grow would leave
//!   unseeded); row 2 is GREEN; rows 3..=7 are GRAY. A cube carries row 1 and a cube row 2.
//! - frame [`GROW_AT`], ONE Main system ordered before `VisibilitySet::Read` (so the stager, after
//!   the gather, drains both marks that same frame): row 2 is edited to RED, and the 9th material
//!   (row 8, RED) is minted — high-water 9 grows the table 8 → 16 — and a cube carrying it spawns.
//!
//! **Premises:** the boot high-water is 8; the minted row is 8; row 1 is BLUE and row 2 GREEN over
//! frames 6..=9.
//!
//! **Verdict:**
//! - pixels, every frame in [`VERDICT`]: row 1 BLUE, row 2 RED, row 8 RED;
//! - the sidecar (the recorder's own counts, `mat_pass`/`mat_regions`, and the host's plan,
//!   `mat_full`): frame 0 and frame [`GROW_AT`] each record ONE pass and ONE full-image region;
//!   every other frame records no pass and no region (the idle assert — critique W3).
//!
//! **What fails it.** This test cannot be red on the parent by its own claim (there the host-visible
//! grow re-seeds the table from the authority, and the sidecar has no `mat_*` keys). Its red is shown
//! by mutation: a grow frame that records the compact runs only turns row 1 dark; a grow frame that
//! records the full image AND the runs reports three regions on frame [`GROW_AT`].
//!
//! Windowed-test conventions: ignored by default, `--test-threads=1`, one `#[test]` per file,
//! `BOYKO_DISABLE_VALIDATION=1` for the default leg. The validation leg (`BOYKO_ENABLE_VALIDATION=1`,
//! `BOYKO_DISABLE_VALIDATION` unset) must print zero `[vk-validation]` lines (critique W6: the grow
//! frame's copy into a fresh buffer, and the next frame's repoint).

#![cfg(windows)]

mod dm1_common;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::asset::Handle;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::{GeometryLegs, Material, RenderPath};
use boyko_scene::VisibilitySet;

use dm1_common::{
    BLUE, Capture, GRAY, GREEN, Hue, RED, arm_dump, assert_ran, assert_resolved, build_app, cube_pixel,
    glow_material, spawn_cube, spawn_view,
};

/// The frame whose Main run edits row 2 and mints row 8 (the grow frame).
const GROW_AT: u64 = 10;
const FRAMES: u32 = 16;
const BEFORE: core::ops::RangeInclusive<u32> = 6..=9;
const VERDICT: core::ops::RangeInclusive<u32> = (GROW_AT as u32 + 2)..=(FRAMES - 1);
const BOOT_X: f32 = -2.0;
const EDITED_X: f32 = 0.0;
const MINTED_X: f32 = 2.0;

#[derive(Resource, Default)]
struct Rig {
    cube: Option<MeshHandle>,
    boot_high_water: usize,
    edited: Option<Handle<Material>>,
    minted_row: Option<u32>,
}

fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
    mut rig: ResMut<Rig>,
) {
    let cube = meshes.cube(dev.get(), dm1_common::CUBE);
    let boot = materials.add(glow_material(BLUE));
    let edited = materials.add(glow_material(GREEN));
    for _ in 0..5 {
        materials.add(glow_material(GRAY));
    }
    rig.boot_high_water = materials.high_water();
    spawn_cube(&mut commands, cube, BOOT_X, 0.0, boot.index());
    spawn_cube(&mut commands, cube, EDITED_X, 0.0, edited.index());
    spawn_view(&mut commands);
    rig.cube = Some(cube);
    rig.edited = Some(edited);
}

fn drive(stats: Res<HostFrameStats>, mut rig: ResMut<Rig>, mut materials: ResMut<Assets<Material>>, mut commands: Commands) {
    if stats.frames != GROW_AT {
        return;
    }
    let edited = rig.edited.expect("invariant: setup minted the edited row");
    materials.get_mut(edited).expect("invariant: the edited row is live").gpu.emissive =
        [RED[0], RED[1], RED[2], 0.0];
    let row = materials.add(glow_material(RED)).index();
    rig.minted_row = Some(row);
    let cube = rig.cube.expect("invariant: setup registered the cube");
    spawn_cube(&mut commands, cube, MINTED_X, 0.0, row);
}

/// The sidecar's `key` on frame `k`, as an integer — a premise failure if the line lacks it.
fn count(cap: &Capture, k: u32, key: &str) -> u32 {
    let f = cap.frame(k);
    f.field(key)
        .unwrap_or_else(|| panic!("PREMISE: the frame-{k} state line carries no `{key}=` (the runner's DM1 upload counters)"))
        .parse()
        .unwrap_or_else(|e| panic!("frame {k}: `{key}=` is not an integer ({e})"))
}

#[test]
#[ignore = "gpu-windowed: needs a windowed GPU device; DM1 test (5), the grow frame, run with --test-threads=1"]
fn dm1_grow_frame() {
    let dump = arm_dump("dm1_grow_frame", FRAMES);
    let mut app = build_app("boyko_app DM1 grow frame", RenderPath::VisibilityBuffer, GeometryLegs::Mesh);
    app.insert_resource(Rig::default());
    app.add_startup_system(setup);
    app.add_systems_cfg(|b| {
        b.add_system(drive).before_set(VisibilitySet::Read);
    });
    app.run();
    assert_ran(&app);
    assert_resolved(&app, RenderPath::VisibilityBuffer, false);

    let rig = app.world().resource::<Rig>();
    assert_eq!(rig.boot_high_water, 8, "PREMISE: the boot high-water (the boot table capacity) is not 8");
    assert_eq!(rig.minted_row, Some(8), "PREMISE: the grow-frame mint did not land on row 8");

    let cap = Capture::load(&dump);
    let boot = cube_pixel(BOOT_X, 0.0);
    let edited = cube_pixel(EDITED_X, 0.0);
    let minted = cube_pixel(MINTED_X, 0.0);
    for (name, px, want) in [("row 1 (boot)", boot, Hue::Blue), ("row 2 (to be edited)", edited, Hue::Green)] {
        let before = cap.assert_stable(name, px, BEFORE);
        assert_eq!(Hue::of(before), want, "PREMISE: {name} at {px:?} is {before:?} before the grow frame");
    }

    let samples = [
        ("row 1 (boot, untouched on the grow frame)", boot, Hue::Blue),
        ("row 2 (edited on the grow frame)", edited, Hue::Red),
        ("row 8 (minted on the grow frame, 8 -> 16)", minted, Hue::Red),
    ];
    let mut failures = Vec::new();
    for (name, px, want) in samples {
        eprintln!("DM1(5) {name} at {px:?}:{}", cap.trace(px, 0..=FRAMES - 1));
        for k in VERDICT {
            let v = cap.frame(k).image.rgb(px);
            if Hue::of(v) != want {
                failures.push(format!("frame {k}: {name} at {px:?} is {v:?} ({:?}), want {want:?}", Hue::of(v)));
            }
        }
    }

    for k in 0..FRAMES {
        let (pass, regions, full) = (count(&cap, k, "mat_pass"), count(&cap, k, "mat_regions"), count(&cap, k, "mat_full"));
        eprintln!("DM1(5) frame {k}: mat_pass={pass} mat_regions={regions} mat_full={full}");
        let want = if k == 0 || u64::from(k) == GROW_AT { (1, 1, 1) } else { (0, 0, 0) };
        if (pass, regions, full) != want {
            failures.push(format!(
                "frame {k}: mat_pass={pass} mat_regions={regions} mat_full={full}, want {want:?} — {}",
                if want.0 == 1 {
                    "frame 0 and the grow frame copy exactly one full-image region"
                } else {
                    "an idle frame records no material_upload pass and no region"
                }
            ));
        }
    }
    assert!(failures.is_empty(), "DM1 test (5), the grow frame:\n{}", failures.join("\n"));
}
