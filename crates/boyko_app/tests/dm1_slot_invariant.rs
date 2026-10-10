//! DM1 red-first test (2): **the staging-slot invariant, with a pinned row layout**
//! (`docs/render/DYNAMIC-MATERIALS-DESIGN-SPACE.md` F1, §7 DM1 (a)(2)).
//!
//! *Every byte range copied from staging slot `s` in frame N must have been written into slot `s`
//! in frame N.* A row edited in frame N−1 was written only into the OTHER slot, so a copy that
//! reads slot `s` over a range covering that row replays whatever slot `s` held two frames ago.
//!
//! **The layout** (rows `a < b < c`, minted at boot, one cube each, all booted with a dim NEUTRAL
//! emissive over a black base):
//! - frame [`N`]−2 sets `b` GREEN (the stale value `B0`, written into slot `x`);
//! - frame [`N`]−1 sets `b` BLUE (the live value `B1`, written into slot `1−x`);
//! - frame [`N`] sets `a` and `c` RED and leaves `b` alone (slot `x` again).
//!
//! A `[min_row, max_row]` copy out of a row-mirrored slot `x` at frame N spans `b` and writes the
//! stale GREEN over the live BLUE. A test with one row per frame could not see that (min equals
//! max); this layout makes the naive coalescing visible.
//!
//! **Premises:** the boot resolved `VisibilityBuffer × Mesh`; all three samples are NEUTRAL and
//! stable over frames 6..=9; the sidecar's `slot=` alternates over frames N−2, N−1, N.
//!
//! **Verdict:** on every frame in [`VERDICT`], `a` and `c` are RED and `b` is BLUE. A GREEN `b` is
//! the slot-invariant violation; a NEUTRAL `b` is an edit that never reached the GPU at all.
//!
//! Windowed-test conventions: ignored by default (needs a windowed GPU device), `--test-threads=1`, one
//! `#[test]` per file (the process-global hook rule — `asset_streaming_f7_grow_headless.rs`'s
//! module doc), `BOYKO_DISABLE_VALIDATION=1` for the default leg.

#![cfg(windows)]

mod dm1_common;

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::asset::Handle;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::{GeometryLegs, Material, RenderPath};

use dm1_common::{
    BLUE, Capture, GRAY, GREEN, Hue, RED, arm_dump, assert_ran, assert_resolved, build_app,
    cube_pixel, glow_material, spawn_cube, spawn_view,
};

/// Frame `N`: the frame that edits `a` and `c` only.
const N: u64 = 12;
const FRAMES: u32 = 17;
const VERDICT: core::ops::RangeInclusive<u32> = (N as u32 + 2)..=(FRAMES - 1);
const XS: [f32; 3] = [-2.0, 0.0, 2.0];

#[derive(Resource, Default)]
struct Rows(Option<[Handle<Material>; 3]>);

fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
    mut rows: ResMut<Rows>,
) {
    let cube = meshes.cube(dev.get(), dm1_common::CUBE);
    let hs = [
        materials.add(glow_material(GRAY)),
        materials.add(glow_material(GRAY)),
        materials.add(glow_material(GRAY)),
    ];
    assert!(hs[0].index() < hs[1].index() && hs[1].index() < hs[2].index(), "PREMISE: rows a < b < c");
    for (h, x) in hs.iter().zip(XS) {
        spawn_cube(&mut commands, cube, x, 0.0, h.index());
    }
    spawn_view(&mut commands);
    rows.0 = Some(hs);
}

fn set(materials: &mut Assets<Material>, h: Handle<Material>, e: [f32; 3]) {
    materials.get_mut(h).expect("invariant: the edited row is live").gpu.emissive = [e[0], e[1], e[2], 0.0];
}

fn edit(stats: Res<HostFrameStats>, rows: Res<Rows>, mut materials: ResMut<Assets<Material>>) {
    let [a, b, c] = rows.0.expect("invariant: setup minted the rows");
    match stats.frames {
        f if f == N - 2 => set(&mut materials, b, GREEN),
        f if f == N - 1 => set(&mut materials, b, BLUE),
        f if f == N => {
            set(&mut materials, a, RED);
            set(&mut materials, c, RED);
        }
        _ => {}
    }
}

#[test]
#[ignore = "gpu-windowed: needs a windowed GPU device; DM1 red-first gate (2), run with --test-threads=1"]
fn dm1_slot_invariant() {
    let dump = arm_dump("dm1_slot_invariant", FRAMES);
    let mut app = build_app("boyko_app DM1 slot invariant", RenderPath::VisibilityBuffer, GeometryLegs::Mesh);
    app.insert_resource(Rows::default());
    app.add_startup_system(setup);
    app.add_systems(edit);
    app.run();
    assert_ran(&app);
    assert_resolved(&app, RenderPath::VisibilityBuffer, false);

    let cap = Capture::load(&dump);
    let names = ["a", "b", "c"];
    let pxs = XS.map(|x| cube_pixel(x, 0.0));
    for (name, px) in names.iter().zip(pxs) {
        eprintln!("DM1(2) `{name}` at {px:?}:{}", cap.trace(px, 0..=FRAMES - 1));
        let before = cap.assert_stable(name, px, 6..=9);
        assert_eq!(Hue::of(before), Hue::Neutral, "PREMISE: `{name}` boots {before:?}, not NEUTRAL");
    }
    let (sx, sy, sz) = (cap.frame(N as u32 - 2).slot, cap.frame(N as u32 - 1).slot, cap.frame(N as u32).slot);
    assert!(
        sx == sz && sx != sy,
        "PREMISE: the slots over frames N-2, N-1, N are {sx}, {sy}, {sz} — not x, 1-x, x"
    );

    let want = [Hue::Red, Hue::Blue, Hue::Red];
    let mut failures = Vec::new();
    for k in VERDICT {
        for ((name, px), w) in names.iter().zip(pxs).zip(want) {
            let v = cap.frame(k).image.rgb(px);
            let got = Hue::of(v);
            if got != w {
                let why = match (*name, got) {
                    ("b", Hue::Green) => "the STALE value B0: a copy read slot x over row b (slot-invariant violation)",
                    (_, Hue::Neutral) => "the BOOT value: the edit never reached the GPU",
                    _ => "neither the edit nor a known failure shape",
                };
                failures.push(format!("frame {k}: `{name}` at {px:?} is {v:?} ({got:?}), want {w:?} — {why}"));
            }
        }
    }
    assert!(failures.is_empty(), "DM1 test (2): the slot invariant does not hold:\n{}", failures.join("\n"));
}
