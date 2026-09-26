//! DM1 red-first test (4): live defect D-2, the `PerInstanceMaterial` ring's falling edge.
//!
//! The ring is uploaded only on a frame whose gather saw a non-default material
//! (`MeshRenderScratch::any_non_default_material`), but VB and Forward read it EVERY frame. So once
//! the last non-default instance goes away, both in-flight slots keep their last bytes, and an
//! instance the gather renumbers into a stale index draws a material that is no longer there.
//!
//! **The fixture.** Four default-material cubes (explicit `MaterialHandle(0)`, so every instance
//! lives in ONE archetype and ONE mesh bucket — the ring order is then the archetype's row order).
//! At frame [`SPAWN_X_AT`] a RED cube `X` is spawned: it takes ring index 4, and the flag rises. At
//! frame [`SWAP_AT`] `X` is despawned and a default cube `D4` is spawned at `X`'s position: `X` was
//! the archetype's last row, so `D4` takes the same row and the same ring index 4, and the flag
//! falls.
//!
//! **Premises:** the sample at `X`'s position is the background, stable over frames 0..=4 (nothing
//! is drawn there yet), and RED, stable over frames 6..=9 (`X` renders its own material); the
//! control cube `D0` is never RED, and on every verdict frame it is Neutral and more than
//! [`DRAWN_MARGIN`] away from that background (the control is drawn).
//!
//! **Verdict:** from frame `SWAP_AT + FRAMES_IN_FLIGHT` on, `D4` renders material 0: its sample
//! equals `D0`'s in the SAME frame within [`MATCH_TOL`] per channel (design §7 DM1 test (4):
//! "every instance must render material 0"). RED on a tree without the F7 upload rule by
//! derivation (RESEARCH §1.11 D-2), and shown red on the parent before the fix lands.
//!
//! **Why a positive verdict, not "not RED".** With DM1 in, a stale id stops reading RED: `X` is the
//! departed material's last carrier, so its refcount turns the row `Retiring` (freed at `epoch +
//! RETIRE_DELAY`), and the stager uploads every non-`Loaded` row as zeros — a stale id reads
//! black. A "not RED" verdict therefore passed a runner that shared one high-water record between
//! both ring slots (slot 1 kept `X`'s id and drew black on every later slot-1 frame), and caught
//! the full D-2 revert on one frame only (DM1 triage r1, T1). Equality with the in-frame control
//! rejects the departed RED row, a zeroed row, an out-of-range id and an undrawn `D4` alike.

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::system::{Res, ResMut};
use boyko_macros::Resource;
use boyko_render::{GeometryLegs, Material, RenderPath};

use super::{
    Capture, Hue, RED, arm_dump, assert_ran, assert_resolved, build_app, cube_pixel, glow_material,
    spawn_view,
};

/// The frame whose Main run spawns the non-default instance `X`.
pub const SPAWN_X_AT: u64 = 5;
/// The frame whose Main run despawns `X` and spawns the default `D4` in its place.
pub const SWAP_AT: u64 = 10;
/// Frames the burst captures, from frame 0.
pub const FRAMES: u32 = 16;
/// The frames the verdict reads: `SWAP_AT + FRAMES_IN_FLIGHT (2)` onward.
pub const VERDICT: core::ops::RangeInclusive<u32> = 12..=(FRAMES - 1);
/// The per-channel tolerance within which `D4`'s sample must equal `D0`'s in the same frame. Both
/// read `[165, 165, 166]` on VB and Forward (difference 0); every failure class sits more than 100
/// away on some channel — the departed RED row `[244, 53, 48]`, a zeroed row `[0, 0, 0]`, the
/// undrawn background `[46, 46, 50]`.
pub const MATCH_TOL: u8 = 8;
/// The least per-channel distance between `D0` and the background at `X`'s position (the
/// [`Hue::of`] channel margin): without it, "`D4` equals `D0`" could be met by a `D4` that is not
/// drawn at all.
pub const DRAWN_MARGIN: u8 = 40;
const _: () = assert!(DRAWN_MARGIN > MATCH_TOL, "an undrawn D4 must fail the D0 comparison");

const DEFAULTS: [(f32, f32); 4] = [(-2.0, 1.3), (0.0, 1.3), (2.0, 1.3), (-2.0, -1.3)];
const X_POS: (f32, f32) = (1.2, -1.3);

#[derive(Resource, Default)]
pub struct Rig {
    cube: Option<MeshHandle>,
    x_row: u32,
    x_entity: Option<Entity>,
}

fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    dev: NonSendRes<GpuDevice>,
    mut rig: ResMut<Rig>,
) {
    let cube = meshes.cube(dev.get(), super::CUBE);
    for (x, y) in DEFAULTS {
        super::spawn_cube(&mut commands, cube, x, y, 0);
    }
    rig.x_row = materials.add(glow_material(RED)).index();
    rig.cube = Some(cube);
    spawn_view(&mut commands);
}

fn drive(stats: Res<HostFrameStats>, mut rig: ResMut<Rig>, mut commands: Commands) {
    let cube = rig.cube.expect("invariant: setup registered the cube");
    let row = u16::try_from(rig.x_row).expect("invariant: a DM1 row fits 16 bits");
    if stats.frames == SPAWN_X_AT {
        let e = commands
            .spawn(MeshBundle {
                material: MaterialHandle(row),
                ..MeshBundle::new(cube, Transform::from_translation(Vec3::new(X_POS.0, X_POS.1, 0.0)))
            })
            .id();
        rig.x_entity = Some(e);
    } else if stats.frames == SWAP_AT {
        let e = rig.x_entity.take().expect("invariant: X was spawned at SPAWN_X_AT");
        commands.entity(e).despawn();
        super::spawn_cube(&mut commands, cube, X_POS.0, X_POS.1, 0);
    }
}

/// Runs test (4) on `path` (mesh leg only) and asserts the verdict.
pub fn run(test: &str, path: RenderPath) {
    let dump = arm_dump(test, FRAMES);
    let mut app = build_app("boyko_app DM1 PerInstanceMaterial falling edge", path, GeometryLegs::Mesh);
    app.insert_resource(Rig::default());
    app.add_startup_system(setup);
    app.add_systems(drive);
    app.run();
    assert_ran(&app);
    assert_resolved(&app, path, false);

    let cap = Capture::load(&dump);
    let x_px = cube_pixel(X_POS.0, X_POS.1);
    let d0_px = cube_pixel(DEFAULTS[0].0, DEFAULTS[0].1);
    eprintln!("DM1(4) {path:?} X/D4 sample at {x_px:?}:{}", cap.trace(x_px, 0..=FRAMES - 1));
    eprintln!("DM1(4) {path:?} D0 control at {d0_px:?}:{}", cap.trace(d0_px, 0..=FRAMES - 1));

    let background = cap.assert_stable("background at X", x_px, 0..=4);
    let before = cap.assert_stable("X", x_px, 6..=9);
    assert_eq!(
        Hue::of(before),
        Hue::Red,
        "PREMISE: X renders {before:?} over frames 6..=9, not its own RED material"
    );
    for k in 0..FRAMES {
        let v = cap.frame(k).image.rgb(d0_px);
        assert_ne!(Hue::of(v), Hue::Red, "PREMISE: the default control cube D0 is RED at frame {k} ({v:?})");
    }
    for k in VERDICT {
        let d0 = cap.frame(k).image.rgb(d0_px);
        assert_eq!(
            Hue::of(d0),
            Hue::Neutral,
            "PREMISE: the default control cube D0 renders {d0:?} at frame {k}, not a neutral material-0 surface"
        );
        assert!(
            channel_distance(d0, background) > DRAWN_MARGIN,
            "PREMISE: D0 renders {d0:?} at frame {k}, within {DRAWN_MARGIN} of the background \
             {background:?} — an undrawn control cannot tell a drawn D4 from none"
        );
    }

    let mut failures = Vec::new();
    for k in VERDICT {
        let f = cap.frame(k);
        let d4 = f.image.rgb(x_px);
        let d0 = f.image.rgb(d0_px);
        if channel_distance(d4, d0) > MATCH_TOL {
            failures.push(format!(
                "frame {k} slot {}: D4 renders {d4:?} ({:?}), the D0 control {d0:?} — {}",
                f.slot,
                Hue::of(d4),
                diagnose(d4, background)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "DM1 test (4) / D-2 on {path:?}: after the last non-default instance left at frame {SWAP_AT}, \
         the renumbered default instance D4 does not render material 0 (the D0 control's colour \
         within {MATCH_TOL} per channel):\n{}",
        failures.join("\n")
    );
}

/// The largest per-channel difference between two texels.
fn channel_distance(a: [u8; 3], b: [u8; 3]) -> u8 {
    a.iter().zip(b).map(|(x, y)| x.abs_diff(y)).max().expect("invariant: three channels")
}

/// Names the failing class of a `D4` sample for the verdict message — a diagnostic only; the
/// verdict is the D0 comparison.
fn diagnose(d4: [u8; 3], background: [u8; 3]) -> &'static str {
    if Hue::of(d4) == Hue::Red {
        "X's departed RED material, read through a stale PerInstanceMaterial id"
    } else if channel_distance(d4, background) <= MATCH_TOL {
        "the background: D4 is not drawn"
    } else if Hue::of(d4) == Hue::Dark {
        "a zeroed material row: a stale id into the departed material's retired row, or an id past \
         the table"
    } else {
        "not material 0"
    }
}
