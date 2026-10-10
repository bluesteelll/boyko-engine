//! Defect R2b (`docs/render/light-table-defects/R2-DESIGN.md`, "Sibling defect R2b"): the CSM fit
//! and the light table name the SAME sun. `resolve_csm_cascades` fits the first ENABLED
//! `DirectionalLight` in query order; `collect_lights` folds the enabled directionals first, so the
//! table's primary directional (`LightTableStaging::primary_directional_dir` — the row the resolve
//! lights with and the Deferred marcher shadows toward) is that same light. Before R2b the fit took
//! the first sun UNFILTERED, so a disabled first sun had cascades fitted to a light nothing lit.
//!
//! # The check
//!
//! Three posed suns across two archetypes — A and C plain, B carrying a test marker — so the query
//! order (archetype by archetype) is not the spawn order; that is asserted, not assumed. For each of
//! the 8 enable masks over {A, B, C} one frame is run, and in THAT frame:
//! - no enabled sun ⇒ `ResolvedCsm::DISABLED`, and the table has no primary;
//! - otherwise the sun the fit took is identified by value — the one sun whose
//!   `resolve_csm(cfg, view, direction, CsmFit::NONE)` equals the published `ResolvedCsm` (the three
//!   fits are asserted pairwise distinct and live first, so the match names exactly one sun) — and it
//!   must be the first ENABLED sun in query order, and its table encoding
//!   (`GpuLight::from_directional`'s `dir_kind.xyz`) must equal the table's primary, bit for bit.
//!
//! The oracle is the ECS QUERY ORDER, iterated through the same
//! `Query<(&DirectionalLight, IsEnabled<LightEnabled>)>` shape both systems read. The masks with the
//! query's first sun disabled (four of the eight, the all-off mask among them) are red on the
//! pre-R2b tree.
//!
//! # The spawn frame
//!
//! Before the sweep, sun A is spawned alone and ONE frame is run: the frame on which
//! `LightingPlugin`'s seed first sets A's `LightEnabled` bit (a row the seed has not reached reads
//! disabled) and `collect_lights`, ordered after the seed, folds A into the table. The fit must
//! take A on that same frame, and it does only because `resolve_csm_cascades` is ordered after the
//! seed. `LightingPlugin` puts the seed in `LightSeedSet` and `CsmPlugin` puts the fit in
//! `CsmResolveSet`; neither declares the edge between them, because an edge naming a memberless set
//! warns `boyko-W1501` in a world that holds only one of the two plugins. `boyko_app`'s
//! `EnginePlugins` declares `CsmResolveSet.after(LightSeedSet)`, and a world composing the two
//! plugins without `boyko_app` declares it itself, as this one does. Without that line the fit runs
//! first and publishes `ResolvedCsm::DISABLED` while the table already lights with A: measured in
//! 300 of 300 runs of a `LightingPlugin` + `CsmPlugin` world before the edge existed, and this case
//! goes red that way when the line is removed.
//!
//! A is built with the exact direction `light_reconcile` derives from its pose, so the reconcile has
//! nothing to rewrite, and that is asserted. It keeps the case about the seed alone:
//! `light_reconcile` is not ordered before the fit either (the one-frame stagger `CsmPlugin`'s doc
//! accepts). Built with the [`DECOY`] instead, A passed 30 of 30 runs with the edge (the reconcile
//! happened to run before the fit every time), but that order comes from the executor, not from an
//! edge, and this case must not rest on it.
//!
//! # What it does not cover
//!
//! A sun added after the first frame, which the seed reaches only through its `Added` scan. B and C
//! are spawned that way, between frames through a world-level `run_system`, but the sweep sets every
//! bit itself, so nothing here depends on the seed reaching them. Whether it does is an open question
//! of its own (`docs/OPEN-QUESTIONS.md`, 2026-09-19).
//!
//! SINGLE-TEST BINARY: `LightingPlugin`'s eviction hooks are process-global
//! (`le_support/common.rs`).

#[path = "le_support/common.rs"]
mod common;

use core::f32::consts::FRAC_PI_3;

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::{IsEnabled, Query};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Component;
use boyko_math::{Affine3A, Quat, Vec3};
use boyko_render::light_system::LightTableStaging;
use boyko_render::{
    CsmConfig, CsmFit, CsmPlugin, CsmResolveSet, DirectionalLight, DirectionalLightObject, GpuLight,
    LightEnabled, LightSeedSet, ResolvedCsm, resolve_csm, set_light_enabled_now,
};
use boyko_scene::{GlobalTransform, Projection, Transform, ViewUniform};

/// The constructor argument B and C are spawned with, which `light_reconcile` overwrites from the
/// pose — so a fit or a row that still carried it would be built from the wrong input.
const DECOY: [f32; 3] = [0.0, 1.0, 0.0];

/// The suns' names, by spawn index, for the failure messages.
const NAMES: [char; 3] = ['A', 'B', 'C'];

/// Puts a sun in a second archetype, so the query order differs from the spawn order.
#[derive(Component, Clone, Copy)]
struct OtherArchetype;

/// A perspective camera at `(0, 2, 0)` looking down `-Z`: 60° vertical FOV, 16:9, near 0.1,
/// far 1000 — the view the fit reads (`fov_y != 0`, so the fit is not short-circuited).
fn camera_view() -> ViewUniform {
    let eye = Vec3::new(0.0, 2.0, 0.0);
    let world = Affine3A::look_at_rh(eye, eye + Vec3::new(0.0, 0.0, -1.0), Vec3::new(0.0, 1.0, 0.0));
    let proj = Projection::Perspective { fov_y: FRAC_PI_3, aspect: 16.0 / 9.0, near: 0.1, far: 1000.0 };
    ViewUniform::from_camera(world, proj)
}

/// A pose whose `-Z` aims along `dir` (TO the light).
fn sun_pose(dir: Vec3) -> Affine3A {
    Affine3A::look_at_rh(Vec3::ZERO, dir, Vec3::new(0.0, 1.0, 0.0))
}

/// The direction `light_reconcile` writes for `pose`, computed with its own expression
/// (`to_light_dir`: the pose's world `-Z`, normalized), so the bits match what it would write.
fn reconciled_dir(pose: Affine3A) -> [f32; 3] {
    let d = pose.transform_vector(Vec3::new(0.0, 0.0, -1.0)).normalize();
    [d.x, d.y, d.z]
}

/// Spawns `light` with its `Transform` and `GlobalTransform` both at `pose`, optionally in the
/// second archetype.
fn spawn_posed(app: &mut App, pose: Affine3A, light: DirectionalLight, other_archetype: bool) -> Entity {
    app.world_mut().run_system(move |mut cmds: Commands| {
        let mut sun = cmds.spawn(DirectionalLightObject {
            transform: Transform { translation: Vec3::ZERO, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE },
            global: GlobalTransform(pose),
            light,
        });
        if other_archetype {
            sun.insert(OtherArchetype);
        }
        sun.id()
    })
}

/// Spawns a sun whose `GlobalTransform` aims it along `dir` (TO the light), with the [`DECOY`] as
/// the constructor argument, optionally in the second archetype.
fn spawn_sun(app: &mut App, dir: Vec3, other_archetype: bool) -> Entity {
    spawn_posed(app, sun_pose(dir), DirectionalLight::new(DECOY, [1.0, 1.0, 1.0], 1.0), other_archetype)
}

/// Enough frames for the spawns to leave the seed's `Added` window (the seed re-enables a light
/// still inside it), so every enable set after this sticks.
fn settle(app: &mut App) {
    for _ in 0..3 {
        app.update();
    }
}

/// The suns in the fits' own query order: each one's spawn index into `suns`, its current
/// component, and its `LightEnabled` bit.
fn query_order(app: &mut App, suns: [Entity; 3]) -> Vec<(usize, DirectionalLight, bool)> {
    app.world_mut().run_system(move |q: Query<(&DirectionalLight, IsEnabled<LightEnabled>)>| {
        q.iter_entities()
            .map(|(id, (light, enabled))| {
                let at = suns
                    .iter()
                    .position(|sun| sun.id() == id)
                    .expect("invariant: the three test suns are the only directional lights");
                (at, *light, enabled)
            })
            .collect()
    })
}

fn bits(v: [f32; 3]) -> [u32; 3] {
    v.map(f32::to_bits)
}

/// Spawns sun A alone, runs ONE frame, and checks that the fit and the table already agree on it —
/// see "The spawn frame" in the module doc. Returns A.
fn spawn_frame_case(app: &mut App, cfg: &CsmConfig, view: &ViewUniform) -> Entity {
    let pose = sun_pose(Vec3::new(0.6, 0.8, 0.0));
    let dir = reconciled_dir(pose);
    let a = spawn_posed(app, pose, DirectionalLight { direction: dir, color: [1.0; 3], illuminance: 1.0 }, false);
    let want = resolve_csm(cfg, view, dir, CsmFit::NONE);
    assert_eq!(want.csm_mode_word, 1, "control: sun A fits live cascades");

    app.update();

    let (light, enabled) = app
        .world_mut()
        .run_system(move |q: Query<(&DirectionalLight, IsEnabled<LightEnabled>)>| {
            q.iter_entities().find(|&(id, _)| id == a.id()).map(|(_, (light, enabled))| (*light, enabled))
        })
        .expect("invariant: sun A is live");
    assert!(enabled, "spawn frame: premise: the light seed enabled sun A on its first frame");
    assert_eq!(
        bits(light.direction),
        bits(dir),
        "spawn frame: premise: `light_reconcile` rewrote sun A's direction, so this case would measure \
         the reconcile's own stagger rather than the seed edge"
    );
    let row = GpuLight::from_directional(&light).dir_kind;
    let primary = app.world().resource::<LightTableStaging>().primary_directional_dir();
    assert_eq!(
        primary.map(bits),
        Some(bits([row[0], row[1], row[2]])),
        "spawn frame: premise: the table's primary is sun A on the frame A is first enabled"
    );
    assert_eq!(
        *app.world().resource::<ResolvedCsm>(),
        want,
        "spawn frame: the table already lights with sun A, but the fit did not take it (DISABLED means \
         `resolve_csm_cascades` ran before the light seed enabled A: the `LightSeedSet → CsmResolveSet` \
         edge is missing)"
    );
    a
}

/// The fit's sun is the light table's primary directional, on the frame the sun is spawned and on
/// every enable mask over three suns in two archetypes — see the module doc.
#[test]
fn the_csm_fit_takes_the_light_tables_primary_sun() {
    let mut app = common::lighting_app();
    app.add_plugin(CsmPlugin);
    // The seed → fit edge `boyko_app`'s `EnginePlugins` declares; see "The spawn frame".
    app.add_systems_cfg(|b| {
        b.configure_set(CsmResolveSet).after(LightSeedSet);
    });
    let cfg = CsmConfig { cascade_count: 3, ..CsmConfig::default() };
    app.insert_resource(cfg);
    let view = camera_view();
    app.insert_resource(view);
    app.finish();

    let a = spawn_frame_case(&mut app, &cfg, &view);
    let suns = [
        a,
        spawn_sun(&mut app, Vec3::new(0.0, 0.8, -0.6), true),
        spawn_sun(&mut app, Vec3::new(-0.48, 0.6, 0.64), false),
    ];
    settle(&mut app);

    let order = query_order(&mut app, suns);
    let spawn_indices: Vec<usize> = order.iter().map(|&(at, _, _)| at).collect();
    assert_eq!(spawn_indices.len(), 3, "precondition: all three suns are live");
    assert_ne!(
        spawn_indices,
        [0, 1, 2],
        "precondition: the query order differs from the spawn order (B sits in a second archetype)"
    );
    let fits: Vec<ResolvedCsm> =
        order.iter().map(|(_, light, _)| resolve_csm(&cfg, &view, light.direction, CsmFit::NONE)).collect();
    for (i, fit) in fits.iter().enumerate() {
        assert_eq!(fit.csm_mode_word, 1, "control: sun {} fits live cascades", NAMES[order[i].0]);
        assert_ne!(order[i].1.direction, DECOY, "precondition: `light_reconcile` posed sun {}", NAMES[order[i].0]);
        for (j, other) in fits.iter().enumerate().skip(i + 1) {
            assert_ne!(
                fit,
                other,
                "precondition: suns {} and {} fit different cascades, so a fit names its sun",
                NAMES[order[i].0],
                NAMES[order[j].0]
            );
        }
    }

    for mask in 0u32..8 {
        for (i, &sun) in suns.iter().enumerate() {
            set_light_enabled_now(app.world_mut(), sun, mask & (1 << i) != 0);
        }
        app.update();

        let order = query_order(&mut app, suns);
        let resolved = *app.world().resource::<ResolvedCsm>();
        let primary = app.world().resource::<LightTableStaging>().primary_directional_dir();
        let enabled: String =
            order.iter().filter(|(_, _, on)| *on).map(|&(at, _, _)| NAMES[at]).collect();

        let Some(&(want, want_light, _)) = order.iter().find(|(_, _, on)| *on) else {
            assert_eq!(primary, None, "mask {mask:03b}: no enabled sun leaves no primary in the table");
            assert_eq!(
                resolved,
                ResolvedCsm::DISABLED,
                "mask {mask:03b}: no enabled sun, but the fit published cascades for a disabled one"
            );
            continue;
        };

        let taken: String = order
            .iter()
            .filter(|(_, light, _)| resolve_csm(&cfg, &view, light.direction, CsmFit::NONE) == resolved)
            .map(|&(at, _, _)| NAMES[at])
            .collect();
        assert_eq!(
            taken,
            NAMES[want].to_string(),
            "mask {mask:03b} (enabled, in query order: {enabled}): the fit took sun {taken:?}, \
             but the first enabled sun in query order is {}",
            NAMES[want]
        );

        let primary = primary.unwrap_or_else(|| panic!("mask {mask:03b}: an enabled sun leaves a primary"));
        let row = GpuLight::from_directional(&want_light).dir_kind;
        assert_eq!(
            bits(primary),
            bits([row[0], row[1], row[2]]),
            "mask {mask:03b}: the table's primary is not sun {}'s row, so the fit and the table disagree",
            NAMES[want]
        );
    }
}
