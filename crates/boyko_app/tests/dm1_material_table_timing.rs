//! DM1 (c) — **the material table's memory-kind A/B instrument** (the DM1 cut's §5; design §7
//! DM1 (c), F3). NOT a gate and never a CI leg: the orchestrator's quiet-window queue builds this
//! binary at the tip that keeps the table host-visible (A) and at the tip that makes it
//! device-local (B), and runs each under `BOYKO_VB_ZONE=1` with `BOYKO_PROFILE_ARTIFACT`, in A B B A
//! order. The runner's own zone leg does all the timing and writes the artifact; this file only
//! builds a scene that reads the material table heavily and drives the edits the legs ask for.
//!
//! # Knobs (environment)
//!
//! - `BOYKO_DM1_PATH` — `vb` (default; `VisibilityBuffer × Mesh`, the `vb_resolve` reader) or
//!   `deferred` (`Deferred × Mesh`, the `deferred_pbr` resolve reader).
//! - `BOYKO_DM1_RES` — `WxH`, default `1920x1080`. The run asserts the achieved client area equals
//!   it: a display that clamps the window makes the row "not measured", never a number at a
//!   different size.
//! - `BOYKO_DM1_EDIT_ROWS` — edit this many material rows (1..=n) EVERY frame, default `0` (the
//!   idle table). The 100-row leg of §5.
//! - `BOYKO_DM1_GROW_AT` — on this frame mint one material past the boot capacity, so the table
//!   grows and that frame copies the full image (the grow-frame leg of §5).
//!
//! The run length is the runner's: `BOYKO_VB_ZONE=1` stops after `BOYKO_VB_BENCH_FRAMES` timed
//! frames, and `BOYKO_WINDOW_FRAMES=n` caps any run. One of the two must be set — a harness that
//! could open a window and never close it is refused at the top.
//!
//! # The scene
//!
//! [`MATERIALS`] distinct material rows on a [`GRID_X`]×[`GRID_Y`] grid of cubes filling the view,
//! every cube its own row, so every shaded pixel fetches `Materials[id]` from a row other pixels do
//! not share.

#![cfg(windows)]

mod dm1_common;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::asset::Handle;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::{GeometryLegs, Material, RenderPath, RenderPathConfig};

use dm1_common::{glow_material, spawn_cube, spawn_view};

/// Material rows minted at startup (plus the runner's default row 0).
const MATERIALS: usize = 256;
const GRID_X: usize = 16;
const GRID_Y: usize = 16;
const SPACING: f32 = 0.34;

#[derive(Resource, Default)]
struct Rig {
    cube: Option<MeshHandle>,
    rows: Vec<Handle<Material>>,
    edit_rows: usize,
    grow_at: Option<u64>,
    grew: bool,
}

fn env_var(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.trim().is_empty())
}

fn requested_res() -> (u32, u32) {
    let Some(v) = env_var("BOYKO_DM1_RES") else {
        return (1920, 1080);
    };
    let (w, h) = v
        .split_once(['x', 'X'])
        .unwrap_or_else(|| panic!("BOYKO_DM1_RES={v}: expected WxH, e.g. 2560x1440"));
    let parse = |s: &str| s.trim().parse::<u32>().unwrap_or_else(|e| panic!("BOYKO_DM1_RES={v}: {e}"));
    (parse(w), parse(h))
}

fn requested_path() -> RenderPath {
    match env_var("BOYKO_DM1_PATH").as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("vb") => RenderPath::VisibilityBuffer,
        Some("deferred") => RenderPath::Deferred,
        Some(other) => panic!("BOYKO_DM1_PATH={other}: expected `vb` or `deferred`"),
    }
}

fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
    mut rig: ResMut<Rig>,
) {
    let cube = meshes.cube(dev.get(), SPACING * 0.9);
    rig.rows = (0..MATERIALS)
        .map(|i| {
            let t = i as f32 / MATERIALS as f32;
            materials.add(glow_material([t, 1.0 - t, 0.5 * t]))
        })
        .collect();
    for gy in 0..GRID_Y {
        for gx in 0..GRID_X {
            let i = gy * GRID_X + gx;
            let x = (gx as f32 - (GRID_X as f32 - 1.0) * 0.5) * SPACING;
            let y = (gy as f32 - (GRID_Y as f32 - 1.0) * 0.5) * SPACING;
            spawn_cube(&mut commands, cube, x, y, rig.rows[i % MATERIALS].index());
        }
    }
    spawn_view(&mut commands);
    rig.cube = Some(cube);
}

fn drive(stats: Res<HostFrameStats>, mut rig: ResMut<Rig>, mut materials: ResMut<Assets<Material>>) {
    let phase = (stats.frames % 2) as f32;
    for &h in rig.rows.iter().take(rig.edit_rows) {
        if let Some(m) = materials.get_mut(h) {
            m.gpu.emissive[3] = phase;
        }
    }
    if !rig.grew && rig.grow_at == Some(stats.frames) {
        materials.add(glow_material([1.0, 1.0, 1.0]));
        rig.grew = true;
    }
}

#[test]
#[ignore = "gpu-windowed: needs a windowed GPU device; the DM1 (c) memory-kind A/B instrument, run only by the quiet-window queue (see the module doc) with --test-threads=1"]
fn dm1_material_table_timing() {
    assert!(
        env_var("BOYKO_VB_ZONE").is_some() || env_var("BOYKO_WINDOW_FRAMES").is_some(),
        "set BOYKO_VB_ZONE=1 (+ BOYKO_VB_BENCH_FRAMES) or BOYKO_WINDOW_FRAMES=<n>: without either the \
         window never closes"
    );
    let (w, h) = requested_res();
    let path = requested_path();
    let edit_rows = env_var("BOYKO_DM1_EDIT_ROWS").map_or(0, |v| {
        v.trim().parse::<usize>().unwrap_or_else(|e| panic!("BOYKO_DM1_EDIT_ROWS={v}: {e}"))
    });
    assert!(edit_rows <= MATERIALS, "BOYKO_DM1_EDIT_ROWS={edit_rows} exceeds the {MATERIALS} minted rows");
    let grow_at = env_var("BOYKO_DM1_GROW_AT")
        .map(|v| v.trim().parse::<u64>().unwrap_or_else(|e| panic!("BOYKO_DM1_GROW_AT={v}: {e}")));

    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("boyko_app DM1 material table timing", w, h));
    app.insert_resource(RenderPathConfig { path, legs: GeometryLegs::Mesh });
    app.insert_resource(Rig { edit_rows, grow_at, ..Rig::default() });
    app.add_startup_system(setup);
    app.add_systems(drive);
    app.run();

    let info = *app.world().resource::<WindowInfo>();
    assert_eq!(
        (info.width, info.height),
        (w, h),
        "the window came up {}x{}, not the requested {w}x{h}: this row is NOT MEASURED (a clamped \
         display), never a number at a different size",
        info.width,
        info.height
    );
    let stats = *app.world().resource::<HostFrameStats>();
    assert!(stats.frames > 0, "the runner presented no frame");
    if let Some(at) = grow_at {
        assert!(app.world().resource::<Rig>().grew, "BOYKO_DM1_GROW_AT={at}: the run ended before the grow frame");
    }
    eprintln!(
        "DM1 timing: path={path:?} res={w}x{h} edit_rows={edit_rows} grow_at={grow_at:?} frames={}",
        stats.frames
    );
}
