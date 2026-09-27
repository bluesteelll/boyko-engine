//! DM1 red-first test (3): **live defect D-1, the headroom mint**
//! (`docs/render/DYNAMIC-MATERIALS-DESIGN-SPACE.md` §10.2 D-1, §7 DM1 (a)(3); RESEARCH §1.11).
//!
//! `MaterialTable::grow_if_needed` returns early whenever the high-water mark fits the current
//! capacity, a grow rounds the capacity up to a power of two, and it seeds only the rows `Loaded`
//! at that moment. Nothing else writes the table after boot. So a material minted — or streamed in
//! by `fill` — into the headroom `[need, capacity)` after a grow keeps an all-zero GPU row.
//!
//! **The fixture** (`VisibilityBuffer × Mesh`, where base colour AND emissive both come from the
//! table, so a zero row is black on every channel — the Deferred per-instance raster lane would
//! carry a base colour past a zero row, which is why this gate is pinned to VB):
//! - boot: the runner's default row 0 plus four startup mints ⇒ high-water 5 = the boot capacity;
//! - frame [`MINT_6_AT`]: the 6th material (row 5, RED) is minted and drawn — high-water 6 grows
//!   the table to 8 and re-seeds it (the POSITIVE CONTROL: this row renders on every tree);
//! - frame [`MINT_7_AT`]: the 7th material (row 6, RED) is minted into the headroom and drawn;
//! - frame [`RESERVE_AT`]: row 7 is `reserve`d; frame [`FILL_AT`]: it is `fill`ed RED and drawn.
//!
//! **Premises:** the boot high-water is 5; the three minted rows are 5, 6 and 7.
//!
//! **Verdict:** on every frame in [`VERDICT`], all three cubes are RED. A DARK row 6 or 7 is D-1.
//! A wrong row 5 on any tree is a separate grow defect, named as such.
//!
//! Windowed-test conventions: ignored by default, `--test-threads=1`, one `#[test]` per file,
//! `BOYKO_DISABLE_VALIDATION=1` for the default leg.

#![cfg(windows)]

mod dm1_common;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::asset::Handle;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::{GeometryLegs, Material, RenderPath};

use dm1_common::{
    Capture, GRAY, Hue, RED, arm_dump, assert_ran, assert_resolved, build_app, cube_pixel,
    glow_material, spawn_cube, spawn_view,
};

const MINT_6_AT: u64 = 10;
const MINT_7_AT: u64 = 12;
const RESERVE_AT: u64 = 13;
const FILL_AT: u64 = 14;
const FRAMES: u32 = 19;
const VERDICT: core::ops::RangeInclusive<u32> = (FILL_AT as u32 + 2)..=(FRAMES - 1);
const XS: [f32; 3] = [-2.0, 0.0, 2.0];

#[derive(Resource, Default)]
struct Rig {
    cube: Option<MeshHandle>,
    boot_high_water: usize,
    rows: [u32; 3],
    reserved: Option<Handle<Material>>,
}

fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
    mut rig: ResMut<Rig>,
) {
    rig.cube = Some(meshes.cube(dev.get(), dm1_common::CUBE));
    for _ in 0..4 {
        materials.add(glow_material(GRAY));
    }
    rig.boot_high_water = materials.high_water();
    spawn_view(&mut commands);
}

fn drive(stats: Res<HostFrameStats>, mut rig: ResMut<Rig>, mut materials: ResMut<Assets<Material>>, mut commands: Commands) {
    let cube = rig.cube.expect("invariant: setup registered the cube");
    match stats.frames {
        MINT_6_AT => {
            let row = materials.add(glow_material(RED)).index();
            rig.rows[0] = row;
            spawn_cube(&mut commands, cube, XS[0], 0.0, row);
        }
        MINT_7_AT => {
            let row = materials.add(glow_material(RED)).index();
            rig.rows[1] = row;
            spawn_cube(&mut commands, cube, XS[1], 0.0, row);
        }
        RESERVE_AT => {
            let h = materials.reserve();
            rig.rows[2] = h.index();
            rig.reserved = Some(h);
        }
        FILL_AT => {
            let h = rig.reserved.take().expect("invariant: reserved at RESERVE_AT");
            if materials.fill(h, glow_material(RED)).is_err() {
                panic!("PREMISE: fill of the reserved row {} was rejected", h.index());
            }
            spawn_cube(&mut commands, cube, XS[2], 0.0, h.index());
        }
        _ => {}
    }
}

#[test]
#[ignore = "gpu-windowed: needs a windowed GPU device; DM1 red-first gate (3) / D-1, run with --test-threads=1"]
fn dm1_headroom_mint() {
    let dump = arm_dump("dm1_headroom_mint", FRAMES);
    let mut app = build_app("boyko_app DM1 headroom mint", RenderPath::VisibilityBuffer, GeometryLegs::Mesh);
    app.insert_resource(Rig::default());
    app.add_startup_system(setup);
    app.add_systems(drive);
    app.run();
    assert_ran(&app);
    assert_resolved(&app, RenderPath::VisibilityBuffer, false);

    let rig = app.world().resource::<Rig>();
    assert_eq!(rig.boot_high_water, 5, "PREMISE: the boot high-water (the boot table capacity) is not 5");
    assert_eq!(rig.rows, [5, 6, 7], "PREMISE: the minted rows are not 5, 6, 7");

    let cap = Capture::load(&dump);
    let labels = [
        "row 5 (6th mint, grows 5 -> 8: POSITIVE CONTROL)",
        "row 6 (7th mint, into the headroom)",
        "row 7 (reserve + fill, into the headroom)",
    ];
    let pxs = XS.map(|x| cube_pixel(x, 0.0));
    let mut failures = Vec::new();
    for (i, (label, px)) in labels.iter().zip(pxs).enumerate() {
        eprintln!("DM1(3) {label} at {px:?}:{}", cap.trace(px, 0..=FRAMES - 1));
        for k in VERDICT {
            let v = cap.frame(k).image.rgb(px);
            if Hue::of(v) != Hue::Red {
                // Row 5 is seeded by the grow itself on every tree, so a wrong row 5 is not D-1: the
                // grown table's bytes never reached this path's shaders — a separate grow defect.
                let why = match (i, Hue::of(v)) {
                    (0, _) => "the POSITIVE CONTROL failed: a separate grow defect, not D-1",
                    (_, Hue::Dark) => "an all-zero GPU row (D-1)",
                    _ => "not the minted RED",
                };
                failures.push(format!("frame {k}: {label} at {px:?} is {v:?} ({:?}) — {why}", Hue::of(v)));
            }
        }
    }
    assert!(failures.is_empty(), "DM1 test (3) / D-1: a headroom row never reached the GPU:\n{}", failures.join("\n"));
}
