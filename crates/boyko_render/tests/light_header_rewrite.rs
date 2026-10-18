//! Render wave PX1a tests 8a + 8c (`plan/H4-PX1a-hdr-lit.md` Tests 8, rev 3 arch W8 / perf O11;
//! `px1a/cut.md` D-E4, C2, C8): **a header-only change reaches the staged light table the same
//! frame, as a header-only rewrite.**
//!
//! At HEAD a runtime change of `LightingConfig::exposure` never reaches the GPU until a light
//! changes: `collect_lights`' rebuild gate reads only the light components and `LightTableDirty`.
//! After PX1a the exposure reaches the header through `ResolvedPost` (`PostPlugin`, composed by
//! `le_support::lighting_app`), and a change of any header input rewrites the 64-B header ALONE:
//!
//! - **(8a)** staged bytes `[0, 64)` change, the rows `[64, used)` do not; `LightHeaderGeneration`
//!   bumps, `LightTableGeneration` does NOT (its D5 meaning — "a rows rewrite" — and its readers
//!   are unchanged); an idle frame bumps neither.
//! - **(8c)** the composed world: the change is visible in the staged header the SAME frame.
//! - **The pixel-neutral pin (C8):** for the pinned config the staged header is a literal word
//!   vector — the bytes the producers read at C7 — so PX1a's ECS commit moves no pixel.
//!
//! SINGLE-TEST BINARY (`le_support/common.rs`'s isolation rule): this file's one `#[test]` adds
//! `LightingPlugin` and then spawns a light.

#[path = "le_support/common.rs"]
mod common;

use boyko_ecs::ecs::core::app::App;
use boyko_render::light::LightingConfig;
use boyko_render::light_system::{LIGHT_HEADER_BYTES, LightHeaderGeneration, LightTableGeneration, LightTableStaging};

fn staged(app: &App) -> Vec<u8> {
    app.world().resource::<LightTableStaging>().bytes().to_vec()
}

fn words(bytes: &[u8]) -> Vec<u32> {
    bytes[..LIGHT_HEADER_BYTES].as_chunks::<4>().0.iter().map(|w| u32::from_le_bytes(*w)).collect()
}

/// The staged header for one enabled directional light under the default config at `exposure`:
/// `[light_count, exposure, l0a_count, point_spot_count | sky_diffuse rgb, word 7 | sky_spec rgb,
/// word 11 | the cluster lane]`.
fn pinned_header(exposure: f32) -> Vec<u32> {
    let d = LightingConfig::default();
    vec![
        1,
        exposure.to_bits(),
        1,
        0,
        d.sky_diffuse[0].to_bits(),
        d.sky_diffuse[1].to_bits(),
        d.sky_diffuse[2].to_bits(),
        0,
        d.sky_spec[0].to_bits(),
        d.sky_spec[1].to_bits(),
        d.sky_spec[2].to_bits(),
        0,
        0,
        0,
        0,
        0,
    ]
}

fn rows_gen(app: &App) -> u64 {
    app.world().resource::<LightTableGeneration>().0
}

fn header_gen(app: &App) -> Option<u64> {
    app.world().try_resource::<LightHeaderGeneration>().map(|g| g.0)
}

#[test]
fn a_header_only_change_rewrites_the_staged_header_alone_the_same_frame() {
    let mut app = common::lighting_app();
    app.finish();
    common::spawn_dir_light(app.world_mut(), [0.0, -1.0, 0.0]);
    for _ in 0..4 {
        app.update();
    }

    let before = staged(&app);
    assert!(before.len() > LIGHT_HEADER_BYTES, "the table holds the directional row");
    assert_eq!(words(&before), pinned_header(1.0), "the default staged header (the pixel-neutral pin)");
    let (rows0, header0) = (rows_gen(&app), header_gen(&app));

    app.world_mut().resource_mut::<LightingConfig>().exposure = 2.0;
    app.update();

    let after = staged(&app);
    let w1 = f32::from_bits(words(&after)[1]);
    assert_eq!(w1, 2.0, "staged header word 1 = {w1} after setting exposure 2.0 (no rebuild)");
    let header0 = header0.expect("LightingPlugin inserts LightHeaderGeneration beside LightTableGeneration");
    assert_eq!(words(&after), pinned_header(2.0), "only the exposure word moved in the header");
    assert_eq!(after[LIGHT_HEADER_BYTES..], before[LIGHT_HEADER_BYTES..], "a header-only change rewrote the rows");
    assert_eq!(rows_gen(&app), rows0, "LightTableGeneration bumped on a header-only change");
    assert_eq!(header_gen(&app), Some(header0 + 1), "LightHeaderGeneration did not bump exactly once");

    app.update();
    app.update();
    assert_eq!(rows_gen(&app), rows0, "an idle frame bumped LightTableGeneration");
    assert_eq!(header_gen(&app), Some(header0 + 1), "an idle frame bumped LightHeaderGeneration");
    assert_eq!(staged(&app), after, "an idle frame rewrote the staging");
}
