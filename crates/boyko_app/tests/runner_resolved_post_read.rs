//! Render wave PX1a test 7c (`plan/H4-PX1a-hdr-lit.md` Tests 7c, rev 4 arch OQ1; `px1a/cut.md` D-E5,
//! C2, C8): **the runner reads `ResolvedPost` without panicking, and falls back to exactly what
//! `PostPlugin` would have written.**
//!
//! - The read is a pure function of `Option<&ResolvedPost>` and `Option<&LightingConfig>`
//!   (`boyko_app::post_host::resolved_post_for_frame`): present ⇒ the resource, no report; absent ⇒
//!   `ResolvedPost::from_config` of the config (or of its default) and a `post_plugin_absent`
//!   report — equal field for field to what `resolve_post_policy` writes for the same config in a
//!   world with no camera and no cut.
//! - `post_final_gpu` packs the tonemapper into the mode word at `TONEMAP_MODE_SHIFT`, an exposure
//!   ratio of exactly `1.0` (until PX3) and the dispatch extent.
//! - The runner's source reads the resource only through that function (`try_resource`), never
//!   through a panicking `resource::<ResolvedPost>()`.
//! - `EnginePlugins` composes `PostPlugin`.
//! - Render wave PX1b (`px1b/cut.md` D-R8, D-T3): the sharpen word — `post_sharpen` is
//!   `taa_armed ∧ SharpenMode::Rcas` (a host without `TaaConfig` never sharpens), `with_rcas` sets
//!   exactly the mode word's `POST_MODE_SHARPEN` bit and the `sharpness` word, and the runner calls
//!   each exactly once, `post_sharpen` with its single TAA predicate `taa_armed_now`.
//!
//! The one test that builds `EnginePlugins` spawns nothing; the other world composes only
//! `PostPlugin` (no light hooks), so the binary's process-global registrations cannot collide.

use boyko_app::EnginePlugins;
use boyko_app::post_host::{post_final_gpu, post_sharpen, resolved_post_for_frame, with_rcas};
use boyko_ecs::App;
use boyko_render::light::{LightingConfig, TONEMAP_MODE_SHIFT, Tonemapper};
use boyko_render::post::gpu::{POST_MODE_SHARPEN, PostFinalGpu};
use boyko_render::post::{PostPlugin, ResolvedPost};
use boyko_render::{SharpenMode, TaaConfig};

fn configs() -> [LightingConfig; 3] {
    let d = LightingConfig::default();
    [
        d,
        LightingConfig { exposure: 2.5, tonemapper: Tonemapper::Neutral, ..d },
        LightingConfig { exposure: 0.125, tonemapper: Tonemapper::ReinhardJodie, ..d },
    ]
}

#[test]
fn the_fallback_is_from_config_and_reports_once_per_read() {
    for cfg in configs() {
        let (post, absent) = resolved_post_for_frame(None, Some(&cfg));
        assert!(absent, "a missing ResolvedPost must be reported (post_plugin_absent)");
        assert_eq!(post, ResolvedPost::from_config(&cfg));
        assert_eq!((post.pre_exposure, post.pre_ratio, post.cut), (cfg.exposure, 1.0, 0));
        assert_eq!(post.tonemapper, cfg.tonemapper as u32);
    }
    let (post, absent) = resolved_post_for_frame(None, None);
    assert!(absent);
    assert_eq!(post, ResolvedPost::from_config(&LightingConfig::default()));
}

#[test]
fn a_present_resolved_post_is_read_as_is() {
    let cfg = LightingConfig { exposure: 3.0, ..LightingConfig::default() };
    let written = ResolvedPost { pre_ratio: 1.5, cut: 1, ..ResolvedPost::from_config(&cfg) };
    let (post, absent) = resolved_post_for_frame(Some(&written), Some(&LightingConfig::default()));
    assert!(!absent);
    assert_eq!(post, written);
}

#[test]
fn the_fallback_equals_what_post_plugin_writes_with_no_camera_and_no_cut() {
    for cfg in configs() {
        let mut app = App::new();
        app.insert_resource(cfg);
        app.add_plugins(PostPlugin);
        app.finish();
        for _ in 0..3 {
            app.update();
        }
        let written = *app.world().resource::<ResolvedPost>();
        assert_eq!(resolved_post_for_frame(None, Some(&cfg)).0, written, "config {cfg:?}");
    }
}

#[test]
fn post_final_gpu_packs_the_tonemapper_a_unit_ratio_and_the_extent() {
    for cfg in configs() {
        let post = ResolvedPost::from_config(&cfg);
        assert_eq!(
            post_final_gpu(&post, [640, 360]),
            PostFinalGpu {
                mode_words: (cfg.tonemapper as u32) << TONEMAP_MODE_SHIFT,
                exposure_ratio: 1.0,
                extent: [640, 360],
                sharpness: 0.0,
                _r: [0; 3],
            }
        );
    }
}

#[test]
fn the_runner_reads_resolved_post_only_through_try_resource() {
    let runner = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/runner.rs"))
        .expect("invariant: the runner source is readable");
    let code: String = runner.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    // The panicking read is the METHOD `.resource::<…>` — the bare `resource::<ResolvedPost>` is also
    // the tail of the required `try_resource::<ResolvedPost>`, so matching it alone could never pass
    // (the C2 form of this census; corrected at C8, where the runner first reads the resource).
    assert!(!compact.contains(".resource::<ResolvedPost>"), "the runner reads ResolvedPost through a panicking resource()");
    assert!(
        !compact.contains(".resource::<boyko_render::post::ResolvedPost>"),
        "the runner reads ResolvedPost through resource()"
    );
    assert_eq!(compact.matches("resolved_post_for_frame(").count(), 1, "the runner does not read ResolvedPost through post_host");
    assert!(compact.contains("try_resource::<ResolvedPost>") || compact.contains("try_resource::<boyko_render::post::ResolvedPost>"));
}

#[test]
fn engine_plugins_compose_post_plugin() {
    let mut app = App::new();
    app.add_plugins(EnginePlugins::window("px1a-post-plugin-composition", 64, 64));
    app.finish();
    assert!(app.world().try_resource::<ResolvedPost>().is_some(), "EnginePlugins does not compose PostPlugin");
}

#[test]
fn post_sharpen_is_taa_armed_and_rcas() {
    // `rcas_sharpness` 1 and 0 map to RCAS's con 1 and 0.25 exactly (FSR 2's remap, 2^-(2 - 2s)).
    let strong = TaaConfig { sharpen: SharpenMode::Rcas, rcas_sharpness: 1.0, ..TaaConfig::default() };
    let mild = TaaConfig { rcas_sharpness: 0.0, ..strong };
    let off = TaaConfig { rcas_sharpness: 1.0, ..TaaConfig::default() };
    assert_eq!(off.sharpen, SharpenMode::None, "PREMISE: the default config does not sharpen");
    assert_eq!(post_sharpen(true, Some(&strong)), Some(1.0), "TAA armed + Rcas sharpens at con 1");
    assert_eq!(post_sharpen(true, Some(&mild)), Some(0.25), "TAA armed + Rcas sharpens at con 0.25");
    assert_eq!(post_sharpen(false, Some(&strong)), None, "a frame without TAA armed never sharpens");
    assert_eq!(post_sharpen(true, Some(&off)), None, "SharpenMode::None never sharpens");
    assert_eq!(post_sharpen(false, Some(&off)), None);
    assert_eq!(post_sharpen(true, None), None, "a host without AaPlugin (no TaaConfig) never sharpens");
    assert_eq!(post_sharpen(false, None), None);
}

#[test]
fn with_rcas_sets_the_bit_and_the_sharpness_word_only() {
    for cfg in configs() {
        let base = post_final_gpu(&ResolvedPost::from_config(&cfg), [640, 360]);
        assert_eq!(base.mode_words & POST_MODE_SHARPEN, 0, "PREMISE: post_final_gpu does not sharpen");
        assert_eq!(with_rcas(base, None), base, "no con, no change");
        let s = with_rcas(base, Some(0.5));
        assert_eq!(s.mode_words, base.mode_words | POST_MODE_SHARPEN, "the sharpen bit, the tonemapper kept");
        assert_eq!(s.sharpness.to_bits(), 0.5f32.to_bits(), "the sharpness word carries con");
        assert_eq!(PostFinalGpu { mode_words: base.mode_words, sharpness: base.sharpness, ..s }, base, "nothing else moves");
    }
}

#[test]
fn the_runner_reads_the_sharpen_word_once_through_post_host() {
    let runner = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/runner.rs"))
        .expect("invariant: the runner source is readable");
    let code: String = runner.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(compact.matches("post_sharpen(").count(), 1, "the runner must compute the sharpen word exactly once, through post_host");
    assert_eq!(compact.matches("with_rcas(").count(), 1, "the runner must arm the post_final words exactly once, through post_host");
    assert!(compact.contains("post_sharpen(taa_armed_now,"), "the sharpen word must read the runner's single TAA predicate");
}
