//! IBL1 R1 (`ibl1/cut.md` C9, D-P2e) — **the environment light in the light fold, end to end**:
//! the ONE-SKY-row rule (D20) through `collect_lights` on a real `App`.
//!
//! The pure rule (`plan_env_row`) is unit-tested in `light_system.rs`; this file drives the
//! system the way a frame does — the plugin's unarmed default, the boot override, the
//! `Changed<EnvironmentLight>` probe and the removal hook:
//!
//! 1. **Unarmed byte identity.** An `EnvironmentLight` on an UNARMED world — and on a world with no
//!    `ResolvedEnvironment` at all (a `LightingPlugin`-only host) — leaves the folded table bytes
//!    equal to the same world without it.
//! 2. **Armed: exactly one sky row.** Arming (the runner's post-boot override + `LightTableDirty`)
//!    folds one SKY row flagged `LIGHT_FLAG_ENV`, and the co-present `SkyLight` writes nothing:
//!    `l0a_count` = directionals + 1.
//! 3. **Change.** An intensity edit is seen by the `Changed` probe: the table is rewritten
//!    (`LightTableGeneration` bumps) and the row's lanes scale.
//! 4. **Removal.** Despawning the environment entity marks `LightTableDirty` (the removal hook),
//!    and the next fold carries the `SkyLight` row again — the unarmed bytes exactly.
//!
//! SINGLE-TEST BINARY (the `light_generation.rs` rule): `LightingPlugin` registers process-global
//! hooks, so a second `App` with it in this process would panic `AlreadyArchetyped`. The phases
//! above therefore run in sequence on one `App`.

#[path = "le_support/common.rs"]
mod common;

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::component::component::Component;
use boyko_render::environment::{EnvironmentLight, EnvironmentPlugin, ResolvedEnvironment};
use boyko_render::light::{GpuLight, LIGHT_FLAG_ENV, LIGHT_KIND_SKY, LightTableDirty, SkyLight};
use boyko_render::light_system::{LIGHT_HEADER_BYTES, LightTableGeneration, LightTableStaging};

/// The staged table's bytes.
fn table(app: &App) -> Vec<u8> {
    app.world().resource::<LightTableStaging>().bytes().to_vec()
}

/// Row `i` of the staged table (after the header).
fn row(app: &App, i: usize) -> GpuLight {
    let bytes = table(app);
    let at = LIGHT_HEADER_BYTES + i * size_of::<GpuLight>();
    assert!(bytes.len() >= at + size_of::<GpuLight>(), "the table holds row {i}");
    let mut words = [0f32; 12];
    for (k, w) in words.iter_mut().enumerate() {
        let b = at + 4 * k;
        *w = f32::from_le_bytes([bytes[b], bytes[b + 1], bytes[b + 2], bytes[b + 3]]);
    }
    GpuLight {
        dir_kind: [words[0], words[1], words[2], words[3]],
        pos_range: [words[4], words[5], words[6], words[7]],
        color_cone: [words[8], words[9], words[10], words[11]],
    }
}

fn generation(app: &App) -> u64 {
    app.world().resource::<LightTableGeneration>().0
}

fn mark_dirty(app: &mut App) {
    app.world_mut().resource_mut::<LightTableDirty>().0 = true;
}

fn settle(app: &mut App) {
    for _ in 0..3 {
        app.update();
    }
}

#[test]
fn environment_light_folds_one_sky_row_only_when_armed() {
    let mut app = common::lighting_app();
    app.add_plugins(EnvironmentPlugin);
    app.finish();

    let world = app.world_mut();
    let _sun = common::spawn_dir_light(world, [0.0, 1.0, 0.0]);
    let sky_arch = world.create_archetype(&[SkyLight::component_id()]);
    let sky = SkyLight::new([0.9, 0.85, 0.8], [0.2, 0.18, 0.16]);
    world.spawn_one(sky_arch, sky).expect("the SkyLight spawns");
    settle(&mut app);
    let without_env = table(&app);
    assert_eq!(common::l0a_count(&app), 2, "anti-vacuity: a directional + the SkyLight row");

    // 1a. Unarmed (the plugin's default carrier): the EnvironmentLight is ignored, bytes unchanged.
    let env_arch = app.world_mut().create_archetype(&[EnvironmentLight::component_id()]);
    let env = app
        .world_mut()
        .spawn_one(env_arch, EnvironmentLight::new(1.0, 0.5))
        .expect("the EnvironmentLight spawns");
    settle(&mut app);
    assert_eq!(table(&app), without_env, "an unarmed boot folds the same bytes with or without an EnvironmentLight");

    // 1b. No carrier at all (a LightingPlugin-only host): unarmed too, no panic.
    let carrier = app.world_mut().remove_resource::<ResolvedEnvironment>();
    assert!(carrier.is_some(), "EnvironmentPlugin inserted the unarmed carrier");
    mark_dirty(&mut app);
    settle(&mut app);
    assert_eq!(table(&app), without_env, "a world without ResolvedEnvironment is unarmed");

    // 2. Armed: the runner's post-boot override + LightTableDirty.
    let armed = ResolvedEnvironment {
        armed: true,
        tile: 256,
        levels: 7,
        sky_lane: [0.32, 0.31, 0.30],
        ground_lane: [0.05, 0.05, 0.04],
    };
    app.world_mut().insert_resource(armed);
    mark_dirty(&mut app);
    settle(&mut app);
    assert_eq!(common::l0a_count(&app), 2, "exactly ONE sky row: the SkyLight writes nothing");
    let r = row(&app, 1);
    assert_eq!(r.dir_kind[3].to_bits(), LIGHT_KIND_SKY | LIGHT_FLAG_ENV, "the row is the flagged SKY row");
    assert_eq!(r.dir_kind[0], 1.0, "intensity lane");
    assert!((r.dir_kind[1] - 0.5f32.cos()).abs() < 1e-6 && (r.dir_kind[2] - 0.5f32.sin()).abs() < 1e-6, "yaw lanes");
    assert_eq!([r.color_cone[0], r.color_cone[1], r.color_cone[2]], armed.sky_lane, "sky lane × 1");
    assert_eq!([r.pos_range[0], r.pos_range[1], r.pos_range[2]], armed.ground_lane, "ground lane × 1");
    assert_eq!(r.pos_range[3], 6.0, "L1 rides the range lane");
    assert_eq!(r.color_cone[3], 256.0, "the tile rides the cone lane");
    assert!(app.world().resource::<LightTableStaging>().sky_present(), "table_has_sky sees the flagged row");

    // 3. Change: the Changed<EnvironmentLight> probe rewrites the table.
    let before = generation(&app);
    app.world_mut().get_component_mut::<EnvironmentLight>(env).expect("the light is live").intensity = 2.0;
    app.update();
    assert!(generation(&app) > before, "an intensity edit rewrites the table");
    let r = row(&app, 1);
    assert_eq!(r.dir_kind[0], 2.0);
    assert_eq!(r.color_cone[0], armed.sky_lane[0] * 2.0, "the lanes scale with the intensity");

    // 4. Removal: the hook marks the table dirty and the SkyLight row comes back.
    assert!(app.world_mut().delete_entity(env), "the environment entity despawns");
    settle(&mut app);
    assert_eq!(table(&app), without_env, "after the removal the SkyLight row is back, byte for byte");
}
