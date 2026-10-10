//! DM1 live defect D-3: a material-table grow must repoint EVERY descriptor set that binds the
//! table, on every render path.
//!
//! `MaterialTable::grow_if_needed` replaces the table buffer and retires the old one at
//! `epoch + RETIRE_DELAY`; `GBufferFrame::repoint_material_table` then rewrites the material
//! binding of every per-slot set `GBufferTargets::material_set_rings` enumerates. When that list
//! held only the Deferred sets, VB (its Set-0 family) and Forward (its Set 0), and the SDF leg of
//! both (`sdf_forward_set`), kept reading the superseded buffer: out of bounds for every row past
//! its capacity, and a destroyed buffer once the retire lands (`VUID-vkCmdDispatch-None-08114`).
//!
//! **The fixture** (legs `Both`; every material black-based, so the emissive lane — table-only —
//! carries the colour):
//! - boot: the runner's default row 0, a BLUE row 1 and three GRAY rows ⇒ high-water 5 = the boot
//!   table capacity. A cube carries row 1 (the boot-row control).
//! - an SDF sphere carrying row 5 is spawned at startup, before row 5 exists: the SDF edit list is
//!   boot-static, so a sphere cannot join it later. Until the mint it reads past the boot table,
//!   which proves nothing and is not read.
//! - frame [`MINT_AT`]: the 6th material (row 5, RED) is minted and a cube carrying it is spawned.
//!   High-water 6 grows the table 5 → 8, and the grow seeds row 5 into the new buffer.
//!
//! **Premises:** the boot high-water is 5; the minted row is 5; the boot-row cube is BLUE and
//! stable over frames 6..=9.
//!
//! **Verdict:** on every frame in [`VERDICT`], the row-5 cube and the row-5 sphere are RED (the
//! grown buffer reached the path's mesh and SDF sets) and the row-1 cube is still BLUE.

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::{GeometryLegs, Material, RenderPath};

use super::{
    BLUE, Capture, GRAY, Hue, RED, arm_dump, assert_ran, assert_resolved, build_app, cube_pixel,
    glow_material, sdf_pixel, spawn_cube, spawn_sdf_sphere, spawn_view,
};

/// The frame whose Main run mints row 5 and spawns its cube (the grow frame).
pub const MINT_AT: u64 = 10;
/// Frames the burst captures, from frame 0 — past `MINT_AT + RETIRE_DELAY`, so the superseded
/// buffer has been destroyed well before the last verdict frame.
pub const FRAMES: u32 = 18;
/// The verdict frames: two past the grow, so both in-flight slots have been repointed.
pub const VERDICT: core::ops::RangeInclusive<u32> = (MINT_AT as u32 + 2)..=(FRAMES - 1);

/// The row the grow seeds (the first row past the boot capacity).
const GROWN_ROW: u32 = 5;
const GROWN_CUBE: (f32, f32) = (-2.0, 0.0);
const BOOT_CUBE: (f32, f32) = (0.0, 0.0);
const GROWN_SDF: (f32, f32) = (2.0, 0.0);

#[derive(Resource, Default)]
pub struct Rig {
    cube: Option<MeshHandle>,
    boot_high_water: usize,
    boot_row: u32,
    minted_row: Option<u32>,
}

fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
    mut rig: ResMut<Rig>,
) {
    let cube = meshes.cube(dev.get(), super::CUBE);
    rig.boot_row = materials.add(glow_material(BLUE)).index();
    for _ in 0..3 {
        materials.add(glow_material(GRAY));
    }
    rig.boot_high_water = materials.high_water();
    spawn_cube(&mut commands, cube, BOOT_CUBE.0, BOOT_CUBE.1, rig.boot_row);
    spawn_sdf_sphere(&mut commands, GROWN_SDF.0, GROWN_SDF.1, GROWN_ROW);
    spawn_view(&mut commands);
    rig.cube = Some(cube);
}

fn drive(stats: Res<HostFrameStats>, mut rig: ResMut<Rig>, mut materials: ResMut<Assets<Material>>, mut commands: Commands) {
    if stats.frames != MINT_AT {
        return;
    }
    let cube = rig.cube.expect("invariant: setup registered the cube");
    let row = materials.add(glow_material(RED)).index();
    rig.minted_row = Some(row);
    spawn_cube(&mut commands, cube, GROWN_CUBE.0, GROWN_CUBE.1, row);
}

/// Runs the D-3 gate on `path` (both legs) and asserts the verdict.
pub fn run(test: &str, path: RenderPath) {
    let dump = arm_dump(test, FRAMES);
    let mut app = build_app("boyko_app DM1 grow repoint", path, GeometryLegs::Both);
    app.insert_resource(Rig::default());
    app.add_startup_system(setup);
    app.add_systems(drive);
    app.run();
    assert_ran(&app);
    assert_resolved(&app, path, true);

    let rig = app.world().resource::<Rig>();
    assert_eq!(rig.boot_high_water, 5, "PREMISE: the boot high-water (the boot table capacity) is not 5");
    assert_eq!(rig.boot_row, 1, "PREMISE: the BLUE boot row is not row 1");
    assert_eq!(rig.minted_row, Some(GROWN_ROW), "PREMISE: the grow-frame mint did not land on row {GROWN_ROW}");

    let cap = Capture::load(&dump);
    let grown_cube = cube_pixel(GROWN_CUBE.0, GROWN_CUBE.1);
    let boot_cube = cube_pixel(BOOT_CUBE.0, BOOT_CUBE.1);
    let grown_sdf = sdf_pixel(GROWN_SDF.0, GROWN_SDF.1);

    let before = cap.assert_stable("boot cube", boot_cube, 6..=9);
    assert_eq!(
        Hue::of(before),
        Hue::Blue,
        "PREMISE: the row-1 cube is {before:?} before the grow, not its BLUE boot emissive{}",
        cap.trace(boot_cube, 0..=FRAMES - 1)
    );

    let samples = [
        ("row-5 cube (minted on the grow frame)", grown_cube, Hue::Red),
        ("row-5 SDF sphere", grown_sdf, Hue::Red),
        ("row-1 cube (boot row)", boot_cube, Hue::Blue),
    ];
    let mut failures = Vec::new();
    for (name, px, want) in samples {
        eprintln!("DM1 D-3 {path:?} {name} at {px:?}:{}", cap.trace(px, 0..=FRAMES - 1));
        for k in VERDICT {
            let v = cap.frame(k).image.rgb(px);
            if Hue::of(v) != want {
                failures.push(format!(
                    "frame {k} slot {}: {name} at {px:?} is {v:?} ({:?}), want {want:?}",
                    cap.frame(k).slot,
                    Hue::of(v)
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "DM1 D-3 on {path:?}: after the material-table grow at frame {MINT_AT}, a set on this path \
         still reads the superseded table:\n{}",
        failures.join("\n")
    );
}
