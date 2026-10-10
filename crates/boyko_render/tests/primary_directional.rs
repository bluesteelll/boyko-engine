//! G1 of defect R2 (`docs/render/light-table-defects/R2-DESIGN.md`): the host's copy of the shaders'
//! primary-directional rule, `boyko_render::primary_directional_dir`, reads the staged table the way
//! the resolve does — the first row in `[0, l0a_count)` whose masked kind is DIRECTIONAL, its
//! `dir_kind.xyz` bits verbatim — and agrees with the ECS fold that wrote the table.
//!
//! The Deferred marcher's sun is this function's result (`boyko_app`'s `MarcherSun`), so a
//! disagreement here detaches the marcher's shadow from the resolve's sun.
//!
//! | test | checks | mutation that turns it red |
//! |---|---|---|
//! | T1 | one sun posed by its `GlobalTransform` ⇒ the fold's `from_directional` bits, after `light_reconcile` | — |
//! | T2 | the first sun disabled through `LightEnabled` ⇒ the second | — |
//! | T3 | an empty table, sky only, point + spot only ⇒ `None` | M3: return row 0 unconditionally |
//! | T4 | a directional row whose kind word carries bit 16 and slot bits ⇒ `Some` | M1: drop the kind mask |
//! | T5 | `l0a_count = 1`, row 0 SKY, row 1 directional-looking ⇒ `None`; the same with `l0a_count = 2` ⇒ row 1 | M2: scan every row, not `[0, l0a_count)` |
//! | T12 | 1,000 seeded random light sets and enables, folded by the ECS ⇒ the first ENABLED directional in the fold's own query order, or `None` | — |
//!
//! T12's oracle is the ECS QUERY ORDER, not spawn order: it iterates the same
//! `Query<(&DirectionalLight, IsEnabled<LightEnabled>)>` shape `collect_lights` folds, and a third of
//! the suns carry a test marker that puts them in a second archetype, so the two orders differ. That
//! is counted, not assumed: T12 is red unless the first enabled sun in spawn order differs from the
//! query-order oracle in at least [`MIN_ORDER_DIFFERS`] cases.
//!
//! EVERY LIGHT IS ENABLED EXPLICITLY, as it spawns ([`enable`]). The ECS lights here are spawned
//! between `app.update()` calls through world-level `run_system`, and `LightingPlugin`'s seed does
//! not enable every light spawned that way. Measured on this lane (2026-09-19): a sun spawned
//! between updates, after several updates had run, kept its `LightEnabled` bit clear for three
//! more updates, whether it was spawned through `Commands` or `create_entity`; the same sun spawned
//! after a single light-less update, or from a scheduled system, was seeded. Without the explicit
//! enable only the seed's first full scan caught T1's sun, T2 panicked on "two enabled suns", and T2
//! and T12 never ran. This test pins the table parser against the fold's query order, not the seed,
//! so it sets each bit itself. The seed's blind spot is pre-existing and not R2's.
//!
//! SINGLE ECS TEST: T1, T2 and T12 share ONE `#[test]` on ONE `App`, because `LightingPlugin`'s
//! eviction hooks are process-global (see `le_support/common.rs`). T3–T5 build their tables from
//! slices and never archetype a light, so they are free to be separate tests in this binary.

#[path = "le_support/common.rs"]
mod common;

use boyko_ecs::ecs::core::app::App;
use boyko_ecs::ecs::core::entity::entity::Entity;
use boyko_ecs::ecs::core::iters::query::{IsEnabled, Query};
use boyko_ecs::ecs::core::system::Commands;
use boyko_macros::Component;
use boyko_math::{Affine3A, Quat, Vec3};
use boyko_render::light_system::{GPU_LIGHT_BYTES, LIGHT_HEADER_BYTES, LightTableStaging};
use boyko_render::{
    DirectionalLight, DirectionalLightObject, GpuLight, LightEnabled, LightingConfig, MAX_LIGHTS,
    PointLight, SLOT_NONE_FIELD, SkyLight, SpotLight, primary_directional_dir,
    set_light_enabled_now, write_light_table,
};
use boyko_scene::{GlobalTransform, Transform};

/// Byte offset of the header's `l0a_count` word.
const L0A_COUNT_BYTE: usize = 8;
/// Byte offset of a row's kind word (`dir_kind.w`), relative to the row.
const KIND_WORD_BYTE: usize = 12;
/// The constructor argument every ECS sun is spawned with, which `light_reconcile` must overwrite
/// from the pose — so a table that still carried it would be a table built from the wrong input.
const DECOY: [f32; 3] = [0.0, 1.0, 0.0];
/// T12's floor on the cases whose spawn-order oracle differs from its query-order oracle, so the
/// corpus provably tells the two orders apart (a probe of this seed's corpus counted 69 of 1,000).
const MIN_ORDER_DIFFERS: u32 = 20;

/// Puts a sun in a second archetype, so T12's query order differs from its spawn order.
#[derive(Component, Clone, Copy)]
struct OtherArchetype;

// ===============================================================================================
// Slice-built tables (T3–T5)
// ===============================================================================================

/// Folds the four slices into a worst-case-sized buffer and returns it trimmed to the valid bytes.
fn table(dirs: &[DirectionalLight], skies: &[SkyLight], points: &[PointLight], spots: &[SpotLight]) -> Vec<u8> {
    let mut buf = vec![0u8; LIGHT_HEADER_BYTES + (MAX_LIGHTS as usize) * GPU_LIGHT_BYTES];
    let used = write_light_table(&mut buf, dirs, skies, points, spots, &LightingConfig::default());
    buf.truncate(used);
    buf
}

fn word(t: &[u8], at: usize) -> u32 {
    u32::from_ne_bytes(t[at..at + 4].try_into().expect("invariant: 4 bytes"))
}

fn set_word(t: &mut [u8], at: usize, w: u32) {
    t[at..at + 4].copy_from_slice(&w.to_ne_bytes());
}

fn row(i: usize) -> usize {
    LIGHT_HEADER_BYTES + i * GPU_LIGHT_BYTES
}

fn bits(v: [f32; 3]) -> [u32; 3] {
    v.map(f32::to_bits)
}

fn sky() -> SkyLight {
    SkyLight::new([0.2, 0.3, 0.4], [0.05, 0.06, 0.07])
}

fn point() -> PointLight {
    PointLight::new([1.0, 2.0, 3.0], [1.0, 1.0, 1.0], 50.0, 5.0)
}

fn spot() -> SpotLight {
    SpotLight::new([0.0, 2.0, 0.0], [0.0, -1.0, 0.0], [1.0, 1.0, 1.0], 100.0, 5.0, 15.0, 30.0)
}

/// **T3.** No directional row ⇒ `None`, whatever else the table holds.
#[test]
fn a_table_without_a_directional_has_no_primary() {
    assert_eq!(primary_directional_dir(&[]), None, "an empty slice is shorter than its header");
    assert_eq!(primary_directional_dir(&[0u8; 16]), None, "a truncated header");
    assert_eq!(primary_directional_dir(&table(&[], &[], &[], &[])), None, "a header-only table");
    assert_eq!(primary_directional_dir(&table(&[], &[sky()], &[], &[])), None, "sky only");
    assert_eq!(primary_directional_dir(&table(&[], &[], &[point()], &[spot()])), None, "point + spot only");
    assert_eq!(
        primary_directional_dir(&table(&[], &[sky()], &[point()], &[spot()])),
        None,
        "sky + point + spot"
    );
}

/// **T4.** The kind compare is masked: a directional row carrying bit 16 and slot bits above its tag
/// is still directional (the shaders' `light_kind()`).
#[test]
fn flag_bits_above_the_kind_tag_do_not_hide_a_directional() {
    let sun = DirectionalLight::new([0.3, 0.8, -0.5], [1.0, 1.0, 1.0], 2.0);
    let mut t = table(&[sun], &[sky()], &[], &[]);
    let want = GpuLight::from_directional(&sun).dir_kind;
    set_word(&mut t, row(0) + KIND_WORD_BYTE, (1 << 16) | SLOT_NONE_FIELD);
    let got = primary_directional_dir(&t).expect("a flagged directional row is still the primary");
    assert_eq!(bits(got), bits([want[0], want[1], want[2]]));
}

/// **T5.** Only `[0, l0a_count)` is the directional block: a row past it is never the primary, even
/// when its kind word reads DIRECTIONAL; and the scan never reads past the rows the table holds.
#[test]
fn only_the_front_block_is_scanned() {
    // Row 0 = the sky (l0a_count = 1), row 1 = the point, rewritten to look directional.
    let mut t = table(&[], &[sky()], &[point()], &[]);
    assert_eq!(word(&t, L0A_COUNT_BYTE), 1, "the fixture's front block is the sky alone");
    let decoy_dir = [0.25_f32, 0.5, -0.75];
    for (lane, v) in decoy_dir.iter().enumerate() {
        set_word(&mut t, row(1) + lane * 4, v.to_bits());
    }
    set_word(&mut t, row(1) + KIND_WORD_BYTE, 0);
    assert_eq!(primary_directional_dir(&t), None, "row 1 is outside [0, l0a_count = 1)");

    set_word(&mut t, L0A_COUNT_BYTE, 2);
    let got = primary_directional_dir(&t).expect("l0a_count = 2 takes row 1 in");
    assert_eq!(bits(got), bits(decoy_dir));

    // A header that overstates its rows: the scan stops at the table's end instead of reading on.
    set_word(&mut t, L0A_COUNT_BYTE, 1000);
    let two_rows = t.len();
    assert_eq!(primary_directional_dir(&t[..two_rows - 1]).map(bits), None, "row 1 is cut short");
    assert_eq!(primary_directional_dir(&t).map(bits), Some(bits(decoy_dir)));
}

// ===============================================================================================
// ECS-built tables (T1, T2, T12)
// ===============================================================================================

/// A small xorshift64 — T12's seeded source, no dependency.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// Uniform in `[-1, 1)`.
    fn signed_unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
    }

    /// A random direction, well away from zero length.
    fn direction(&mut self) -> Vec3 {
        loop {
            let v = Vec3::new(self.signed_unit(), self.signed_unit(), self.signed_unit());
            if v.length_squared() > 0.01 {
                return v.normalize();
            }
        }
    }
}

/// Sets a just-spawned light's `LightEnabled` bit, which the seed does not reliably set for a light
/// spawned between updates (see the module doc), and returns the light.
fn enable(app: &mut App, light: Entity) -> Entity {
    set_light_enabled_now(app.world_mut(), light, true);
    light
}

/// Spawns an enabled sun whose `GlobalTransform` aims its `-Z` along `dir` (TO the light), with the
/// [`DECOY`] as the constructor argument, optionally in the second archetype.
fn spawn_sun(app: &mut App, dir: Vec3, other_archetype: bool) -> Entity {
    let pose = Affine3A::look_at_rh(Vec3::ZERO, dir, Vec3::new(0.0, 1.0, 0.0));
    let sun = app.world_mut().run_system(move |mut cmds: Commands| {
        let mut sun = cmds.spawn(DirectionalLightObject {
            transform: Transform { translation: Vec3::ZERO, rotation: Quat::from_mat3(pose.matrix3), scale: Vec3::ONE },
            global: GlobalTransform(pose),
            light: DirectionalLight::new(DECOY, [1.0, 1.0, 1.0], 1.0),
        });
        if other_archetype {
            sun.insert(OtherArchetype);
        }
        sun.id()
    });
    enable(app, sun)
}

fn spawn_sky(app: &mut App) -> Entity {
    let light = app.world_mut().run_system(|mut cmds: Commands| cmds.spawn(sky()).id());
    enable(app, light)
}

fn spawn_point(app: &mut App) -> Entity {
    let light = app.world_mut().run_system(|mut cmds: Commands| cmds.spawn(point()).id());
    enable(app, light)
}

fn spawn_spot(app: &mut App) -> Entity {
    let light = app.world_mut().run_system(|mut cmds: Commands| cmds.spawn(spot()).id());
    enable(app, light)
}

fn despawn_all(app: &mut App, entities: &mut Vec<Entity>) {
    let doomed = std::mem::take(entities);
    app.world_mut().run_system(move |mut cmds: Commands| {
        for e in &doomed {
            cmds.despawn(*e);
        }
    });
}

/// The oracle: the first ENABLED directional in the fold's own query order, as the fold writes it.
fn first_enabled_directional(app: &mut App) -> Option<[f32; 3]> {
    app.world_mut().run_system(|q: Query<(&DirectionalLight, IsEnabled<LightEnabled>)>| {
        q.iter().find_map(|(l, enabled)| {
            enabled.then(|| {
                let d = GpuLight::from_directional(l).dir_kind;
                [d[0], d[1], d[2]]
            })
        })
    })
}

/// The spawn-order counterpart of [`first_enabled_directional`]: the first ENABLED sun of `suns`,
/// taken in the order they were spawned. T12 counts where it differs from the query-order oracle.
fn first_enabled_in_spawn_order(app: &App, suns: &[Entity]) -> Option<[f32; 3]> {
    let world = app.world();
    suns.iter().find_map(|&sun| {
        let light = world.get_component::<DirectionalLight>(sun).filter(|_| world.is_enabled::<LightEnabled>(sun))?;
        let d = GpuLight::from_directional(light).dir_kind;
        Some([d[0], d[1], d[2]])
    })
}

fn staged_primary(app: &App) -> Option<[f32; 3]> {
    app.world().resource::<LightTableStaging>().primary_directional_dir()
}

/// Enough frames for a spawn to leave the seed's `Added` window (the seed re-enables a light still
/// inside it — `light_enabled_toggle.rs`), so a toggle after this sticks.
fn settle(app: &mut App) {
    for _ in 0..3 {
        app.update();
    }
}

/// **T1, T2, T12** — see the module doc for why they share one test.
#[test]
fn the_primary_is_the_first_enabled_directional_the_fold_wrote() {
    let mut app = common::lighting_app();
    app.finish();
    let mut live: Vec<Entity> = Vec::new();

    // T1: the table's primary is the fold's bits for the pose-derived direction, not the decoy.
    let aim = Vec3::new(-0.4, 0.78, 0.48).normalize();
    live.push(spawn_sun(&mut app, aim, false));
    settle(&mut app);
    let want = first_enabled_directional(&mut app).expect("T1: one enabled sun");
    let got = staged_primary(&app).expect("T1: the staged table has a primary");
    assert_eq!(bits(got), bits(want), "T1: the staged primary is the fold's from_directional bits");
    let dot = got[0] * aim.x + got[1] * aim.y + got[2] * aim.z;
    assert!(dot > 0.9999, "T1: the primary {got:?} is the pose's direction {aim:?}, not the decoy {DECOY:?}");
    despawn_all(&mut app, &mut live);
    app.update();
    assert_eq!(staged_primary(&app), None, "T1: the despawned sun leaves no primary");

    // T2: the first sun disabled ⇒ the second.
    let first = spawn_sun(&mut app, Vec3::new(0.6, 0.8, 0.0), false);
    let second_dir = Vec3::new(0.0, 0.8, -0.6);
    let second = spawn_sun(&mut app, second_dir, false);
    live.extend([first, second]);
    settle(&mut app);
    let both = staged_primary(&app).expect("T2: two enabled suns");
    assert!(both[0] > 0.5, "T2: with both enabled the primary is the first sun, got {both:?}");
    set_light_enabled_now(app.world_mut(), first, false);
    app.update();
    let got = staged_primary(&app).expect("T2: the second sun is still enabled");
    assert_eq!(bits(got), bits(first_enabled_directional(&mut app).expect("T2: oracle")));
    assert!(got[2] < -0.5, "T2: the primary is the second sun {second_dir:?}, got {got:?}");
    despawn_all(&mut app, &mut live);
    app.update();

    // T12: random light sets and enables, folded by the ECS.
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    let (mut with_primary, mut without_primary, mut disabled_first) = (0u32, 0u32, 0u32);
    let mut order_differs = 0u32;
    let mut suns: Vec<Entity> = Vec::new();
    for case in 0..1000u32 {
        suns.clear();
        for _ in 0..rng.below(4) {
            let dir = rng.direction();
            let other = rng.below(3) == 0;
            let sun = spawn_sun(&mut app, dir, other);
            suns.push(sun);
            live.push(sun);
        }
        for _ in 0..rng.below(2) {
            live.push(spawn_sky(&mut app));
        }
        for _ in 0..rng.below(3) {
            live.push(spawn_point(&mut app));
        }
        for _ in 0..rng.below(3) {
            live.push(spawn_spot(&mut app));
        }
        settle(&mut app);
        let first_before = first_enabled_directional(&mut app);
        for &e in &live {
            if rng.below(3) == 0 {
                set_light_enabled_now(app.world_mut(), e, false);
            }
        }
        app.update();

        let want = first_enabled_directional(&mut app);
        let got = staged_primary(&app);
        assert_eq!(
            got.map(bits),
            want.map(bits),
            "T12 case {case}: the staged primary is not the first enabled directional in query order"
        );
        match want {
            Some(_) => with_primary += 1,
            None => without_primary += 1,
        }
        if first_before.is_some() && first_before.map(bits) != want.map(bits) {
            disabled_first += 1;
        }
        if first_enabled_in_spawn_order(&app, &suns).map(bits) != want.map(bits) {
            order_differs += 1;
        }
        despawn_all(&mut app, &mut live);
        app.update();
    }
    // Not vacuous: the corpus exercised both outcomes, the disabled-first shape T2 pins, and enough
    // cases where spawn order and query order name different suns that a spawn-order oracle is red.
    assert!(
        with_primary >= 100 && without_primary >= 100 && disabled_first >= 50 && order_differs >= MIN_ORDER_DIFFERS,
        "T12's corpus is too narrow: {with_primary} cases with a primary, {without_primary} without, \
         {disabled_first} where disabling changed the primary, {order_differs} where the first enabled \
         sun in spawn order is not the first in query order (want >= {MIN_ORDER_DIFFERS})"
    );
}
