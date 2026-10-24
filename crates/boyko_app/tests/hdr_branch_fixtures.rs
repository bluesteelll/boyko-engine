//! Render wave PX0 (H3 Scope 8; `px0/cut.md` D-F1..D-F3, PX0-11, PX0-12) — **the HDR branch
//! fixtures, their hot variants, the producer matrix, and the two receipt keys' device gates**.
//!
//! # The fixtures
//!
//! One small scene per tail branch, each with a screen REGION where only that branch runs (the
//! table in [`FIXTURES`]): a centre 32×32 on the object for a surface producer, the top-left 64×64
//! for a background branch. Each region is read from the frame's display-referred image dump —
//! `lit` (RGBA8) before render wave PX1a, `display` since (`lit` is scene-linear RGBA16F there;
//! `BOYKO_IMAGE_DUMP=lit,display`) — or, for the SSAA box, from the host dump BMP
//! (`BOYKO_IMAGE_DUMP` and `BOYKO_HOST_DUMP` are exclusive).
//!
//! - **Constant branches** (`lit = base`, `BACKGROUND_BASE`, `FORWARD_LIT_CLEAR`): every texel of
//!   the region is the display of the scene-linear constant the branch stores
//!   ([`hdr_fixtures::BACKGROUND_SCENE`]) — `quantize8(post_final_eval(c))` within one code, both
//!   UNORM neighbours of the device's value ([`hdr_fixtures::DISPLAY_LSB`]; a rounding mode is not
//!   the spec's).
//! - **Producer branches**: the receipt keys that select them hold the fixture's values, and the
//!   region is not all the background constant's display.
//! - **Hot variants** (every lit-store producer and the SSAA box): an emissive-only surface (black
//!   albedo, no light, no SkyLight, SSAO / DDGI off — rule xvi's zeroed terms) or a uniform sky
//!   (`sky = ground = r`, no directional) at `r = 2.0`; the region equals
//!   `quantize8(post_final_eval(r))` within 1 LSB — the hot oracle is the traced body itself
//!   (`boyko_shaderdsl::post`) — which proves the fixture delivers `r ≥ 2.0` to its region now,
//!   instead of a wrong red at PX1a's test 11b.
//!
//! Every display oracle here evaluates the body under the RUN's own `post_final` words — the
//! capture's receipt `post_word0..7` ([`hdr_fixtures::receipt_post_words`]) — never a spelled
//! curve (render wave PXt, `pxt/cut.md` D-G2's rule): the fixtures run the default configuration,
//! so they follow the engine's default tone curve, whichever one ships.
//!
//! # The producer matrix ([`producer_matrix`], host-tested here)
//!
//! Over the transition-pinned legs' receipts (non-TAA, non-particle) ∪ the fixtures: every branch
//! needs ≥ 1 region and every lit-store producer ≥ 1 hot region; a gap is red. With
//! `BOYKO_PX0_PREMOVE_DIR=<dir>` (GL-H0) the matrix driver reads the dir's pin receipts, runs every
//! fixture into it, and writes `<dir>/producer_matrix.md`. The three `boyko_rhi_vulkan` harness
//! pins (`grand_showcase`, `deferred_sdf_only`, `deferred_mesh_only`) have no runner and no
//! receipt; the matrix lists them as "no receipt".
//!
//! # The receipt keys (PX0-11)
//!
//! [`receipt_light_sky_tracks_the_light_table`] and [`receipt_vb_classified_tracks_the_producer`]
//! are the device red-firsts of `light_sky` / `vb_classified` (red before the keys exist: "no key").
//!
//! Windowed-test conventions: the device drivers are ignored, `--test-threads=1`,
//! `BOYKO_DISABLE_VALIDATION=1`; the device leg (GL-PX0-F) is `-- --ignored --test-threads=1 --skip
//! producer_matrix_over_premove` — the matrix is GL-H0's generator, not a check of this leg.

#[path = "common/graph_receipt.rs"]
mod graph_receipt;
#[path = "common/hdr_fixtures.rs"]
mod hdr_fixtures;
#[path = "common/image_dump.rs"]
mod image_dump;

use std::path::PathBuf;

use boyko_shaderdsl::post::{TONEMAP_ACES, TONEMAP_AGX, TONEMAP_MODE_SHIFT, ubo_word};
use graph_receipt::Receipt;
use hdr_fixtures::{
    BACKGROUND_SCENE, Capture, DISPLAY_LSB, Expect, Fixture, FIXTURES, HOT_R, Hot, capture, default_post_words, display8,
    displays, fixture, fixture_worker_main, hot8, receipt_post_words, tonemap_word,
};

/// **THE WORKER** — one fixture (`BOYKO_PX0_FIXTURE=<name>[:hot]`); the body is shared
/// ([`hdr_fixtures::fixture_worker_main`]).
#[test]
#[ignore = "gpu-windowed: the PX0 branch-fixture worker, spawned by the hdr_branch_fixtures drivers; run with --test-threads=1"]
fn hdr_branch_fixture_worker() {
    fixture_worker_main("hdr_branch_fixture_worker");
}

/// This binary's worker, the one every driver here spawns.
const WORKER: &str = "hdr_branch_fixture_worker";

// ===============================================================================================
// The drivers
// ===============================================================================================

/// The checks of one capture against its fixture (`hot` selects the hot oracle). Every oracle is
/// the traced body under the capture's own `post_final` words (its receipt's `post_word0..7`).
fn check(f: &Fixture, hot: bool, c: &Capture) -> Result<(), String> {
    let words = receipt_post_words(&c.receipt);
    let curve = tonemap_word(&words);
    if hot {
        let want = hot8(words, HOT_R);
        if want.iter().any(|&v| v >= 254) {
            return Err(format!(
                "{} (hot): PREMISE: the run's words (tonemap word {curve}) display r = {HOT_R} as {want:?}: a saturated oracle cannot tell {HOT_R} from more",
                f.name
            ));
        }
        return if c.region.iter().all(|p| displays(want, *p)) {
            Ok(())
        } else {
            Err(format!(
                "{} (hot): region radiance below {HOT_R}: expected quantize8(post_final_eval({HOT_R})) under the run's words (tonemap word {curve}) = {want:?} within {DISPLAY_LSB} LSB, first texel {:?}",
                f.name, c.region[0]
            ))
        };
    }
    match f.expect {
        Expect::Constant(k) => {
            let want = display8(words, k);
            match c.region.iter().find(|p| !displays(want, **p)) {
                None => Ok(()),
                Some(p) => Err(format!(
                    "{}: the constant region is not the display of the scene-linear {k:?} under the run's words (tonemap word {curve}): expected {want:?} within {DISPLAY_LSB} LSB, first other texel {p:?}",
                    f.name
                )),
            }
        }
        Expect::Producer(keys) => {
            for (k, v) in keys {
                let got = c.receipt.config(k);
                if got != *v {
                    return Err(format!("{}: PREMISE [config] {k} = {got}, the fixture needs {v}", f.name));
                }
            }
            let background = display8(words, BACKGROUND_SCENE);
            if c.region.iter().all(|p| displays(background, *p)) {
                return Err(format!(
                    "{}: region empty — every texel is the background constant's display under the run's words ({background:?} within {DISPLAY_LSB} LSB)",
                    f.name
                ));
            }
            Ok(())
        }
    }
}

/// One capture's check, its oracle's inputs logged first (the curve the run's words name).
fn check_logged(f: &Fixture, hot: bool, c: &Capture) {
    let words = receipt_post_words(&c.receipt);
    eprintln!(
        "{}{}: the run's post_final tonemap word {}; hot oracle {:?}, background {:?}",
        f.name,
        if hot { " (hot)" } else { "" },
        tonemap_word(&words),
        hot8(words, HOT_R),
        display8(words, BACKGROUND_SCENE)
    );
    if let Err(e) = check(f, hot, c) {
        panic!("{e}");
    }
}

fn drive(name: &str) {
    let f = fixture(name);
    let cold = capture(WORKER, f, false, None);
    check_logged(f, false, &cold);
    if f.hot != Hot::None {
        let hot = capture(WORKER, f, true, None);
        check_logged(f, true, &hot);
    }
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (deferred surface + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_deferred_surface() {
    drive("deferred_surface");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (deferred SkyLight sky + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_deferred_sky() {
    drive("deferred_sky");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (deferred lit = base); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_deferred_base() {
    drive("deferred_base");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (forward_opaque + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_forward_opaque() {
    drive("forward_opaque");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (forward_opaque_froxel + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_forward_opaque_froxel() {
    drive("forward_opaque_froxel");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (forward_sky with SkyLight + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_forward_sky() {
    drive("forward_sky");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (forward_sky BACKGROUND_BASE); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_forward_sky_base() {
    drive("forward_sky_base");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (FORWARD_LIT_CLEAR); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_forward_lit_clear() {
    drive("forward_lit_clear");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (vb_sky + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_vb_sky() {
    drive("vb_sky");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (sdf_forward_march on Forward + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_sdf_forward_march_forward() {
    drive("sdf_forward_march_forward");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (sdf_forward_march on VB + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_sdf_forward_march_vb() {
    drive("sdf_forward_march_vb");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (vb_resolve + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_vb_resolve() {
    drive("vb_resolve");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (vb_shade, classified + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_vb_shade() {
    drive("vb_shade");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (vb_shade_split + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_vb_shade_split() {
    drive("vb_shade_split");
}

#[test]
#[ignore = "gpu-windowed: PX0 branch fixture (the SSAA box, host dump + hot); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn fixture_ssaa_downsample() {
    drive("ssaa_downsample");
}

/// **PX0-11's `light_sky` gate**: Forward × Mesh without a SkyLight — `light_sky = 0` while the
/// sky PIPELINE is still built (`forward_sky = 1`, OBS's old proxy); with one — `light_sky = 1`.
#[test]
#[ignore = "gpu-windowed: PX0-11 receipt gate for light_sky (spawns hdr_branch_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn receipt_light_sky_tracks_the_light_table() {
    let without = capture(WORKER, fixture("forward_sky_base"), false, None);
    assert_eq!(without.receipt.config("forward_sky"), 1, "PREMISE: the sky pipeline is built on every Forward boot");
    assert_eq!(without.receipt.config("light_sky"), 0, "a table with no SkyLight must read light_sky = 0");
    let with = capture(WORKER, fixture("forward_sky"), false, None);
    assert_eq!(with.receipt.config("light_sky"), 1, "a table with a SkyLight must read light_sky = 1");
}

/// **PX0-11's `vb_classified` gate**: the classified producer (`BOYKO_VB_FORCE_CLASSIFIED`, the
/// producer selector's own knob) reads 1, the fused one 0.
#[test]
#[ignore = "gpu-windowed: PX0-11 receipt gate for vb_classified (spawns hdr_branch_fixture_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn receipt_vb_classified_tracks_the_producer() {
    let fused = capture(WORKER, fixture("vb_resolve"), false, None);
    assert_eq!(fused.receipt.config("vb_classified"), 0, "the fused producer must read vb_classified = 0");
    let classified = capture(WORKER, fixture("vb_shade"), false, None);
    assert_eq!(classified.receipt.config("vb_classified"), 1, "the classified producer must read vb_classified = 1");
}

// ===============================================================================================
// The producer matrix
// ===============================================================================================

/// The lit-store producers (each needs a hot region).
const LIT_STORE: [&str; 12] = [
    "deferred_surface",
    "deferred_sky",
    "forward_opaque",
    "forward_opaque_froxel",
    "forward_sky",
    "vb_sky",
    "sdf_forward_march_forward",
    "sdf_forward_march_vb",
    "vb_resolve",
    "vb_shade",
    "vb_shade_split",
    "ssaa_downsample",
];

/// The `boyko_rhi_vulkan` harness pins: no runner, no receipt.
const NO_RECEIPT_PINS: [&str; 3] = ["grand_showcase", "deferred_sdf_only", "deferred_mesh_only"];

/// The branches a transition-pinned leg's `[config]` exercises (the recorder's own predicates).
fn branches_of(cfg: &dyn Fn(&str) -> u64) -> Vec<&'static str> {
    let mut b = Vec::new();
    let mesh = cfg("mesh_leg") == 1;
    match cfg("path") {
        0 => {
            if mesh {
                b.push("deferred_surface");
            }
            b.push(if cfg("light_sky") == 1 { "deferred_sky" } else { "deferred_base" });
            if cfg("ssaa") == 1 {
                b.push("ssaa_downsample");
            }
        }
        1 | 2 => {
            if mesh {
                b.push(if cfg("path_is_forward_plus") == 1 { "forward_opaque_froxel" } else { "forward_opaque" });
                b.push(if cfg("light_sky") == 1 { "forward_sky" } else { "forward_sky_base" });
            } else {
                b.push("forward_lit_clear");
            }
            if cfg("path_has_sdf_forward") == 1 {
                b.push("sdf_forward_march_forward");
            }
        }
        _ => {
            b.push(if cfg("light_sky") == 1 { "vb_sky" } else { "forward_sky_base" });
            if cfg("path_has_sdf_forward") == 1 {
                b.push("sdf_forward_march_vb");
            }
            if mesh {
                b.push(if cfg("path_vb_split") == 1 {
                    "vb_shade_split"
                } else if cfg("vb_classified") == 1 {
                    "vb_shade"
                } else {
                    "vb_resolve"
                });
            }
        }
    }
    b
}

/// One input of the matrix: a leg (a pin's receipt) or a fixture (with whether its hot region
/// passed).
struct MatrixInput<'a> {
    name: String,
    cfg: Box<dyn Fn(&str) -> u64 + 'a>,
    fixture: Option<(&'static str, bool)>,
}

/// The matrix: per branch, the legs and fixtures covering it; red when a branch has no region or a
/// lit-store producer no hot region.
fn producer_matrix(inputs: &[MatrixInput<'_>]) -> Result<String, Vec<String>> {
    let mut out = String::from("| branch | transition-pinned legs | fixture | hot region |\n|---|---|---|---|\n");
    let mut bad = Vec::new();
    for f in FIXTURES {
        let legs: Vec<&str> = inputs
            .iter()
            .filter(|i| i.fixture.is_none())
            .filter(|i| (i.cfg)("taa") == 0 && (i.cfg)("path_has_particles") == 0)
            .filter(|i| branches_of(&*i.cfg).contains(&f.name))
            .map(|i| i.name.as_str())
            .collect();
        let fix = inputs.iter().find(|i| i.fixture.is_some_and(|(n, _)| n == f.name));
        let hot = fix.is_some_and(|i| i.fixture.is_some_and(|(_, h)| h));
        if legs.is_empty() && fix.is_none() {
            bad.push(format!("branch `{}` is reached by no transition-pinned leg and no fixture", f.name));
        }
        if LIT_STORE.contains(&f.name) && !hot {
            bad.push(format!("lit-store producer `{}` has no hot region (r >= {HOT_R})", f.name));
        }
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            f.name,
            if legs.is_empty() { "—".to_owned() } else { legs.join(", ") },
            if fix.is_some() { "yes" } else { "—" },
            if hot { "yes" } else { "—" }
        ));
    }
    out.push_str(&format!("\nNo receipt (boyko_rhi_vulkan harness pins, no runner): {}\n", NO_RECEIPT_PINS.join(", ")));
    if bad.is_empty() { Ok(out) } else { Err(bad) }
}

/// **The matrix over GL-H0's pre-move set** (`BOYKO_PX0_PREMOVE_DIR`): every fixture (both
/// variants) dumped into the dir, every pin receipt in it read, `producer_matrix.md` written.
///
/// A `generator`, not a device check (triage r1 W7): it is GL-H0 step 5, it writes an artifact,
/// and without GL-H0's directory it cannot pass, so no device leg may sweep it in. A per-binary
/// `-- --ignored` run (GL-PX0-F) names it out with `--skip producer_matrix_over_premove`.
#[test]
#[ignore = "generator: PX0 GL-H0 step 5, the producer matrix over GL-H0's pre-move dir (BOYKO_PX0_PREMOVE_DIR; renders every fixture on a windowed GPU, writes producer_matrix.md); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn producer_matrix_over_premove() {
    let Ok(dir) = std::env::var("BOYKO_PX0_PREMOVE_DIR") else {
        panic!("BOYKO_PX0_PREMOVE_DIR is unset: name GL-H0's pre-move dir");
    };
    let dir = PathBuf::from(dir);
    let mut inputs: Vec<MatrixInput<'static>> = Vec::new();
    for f in FIXTURES {
        let cold = capture(WORKER, &f, false, Some(&dir));
        check(&f, false, &cold).unwrap_or_else(|e| panic!("{e}"));
        let hot = f.hot != Hot::None && {
            let h = capture(WORKER, &f, true, Some(&dir));
            check(&f, true, &h).unwrap_or_else(|e| panic!("{e}"));
            true
        };
        let r = cold.receipt;
        inputs.push(MatrixInput { name: f.name.to_owned(), cfg: Box::new(move |k| r.config(k)), fixture: Some((f.name, hot)) });
    }
    for e in std::fs::read_dir(&dir).expect("the premove dir is readable").flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_owned();
        if !name.ends_with(".toml") || name.starts_with("fixture_") {
            continue;
        }
        let r = Receipt::load(&p);
        inputs.push(MatrixInput { name, cfg: Box::new(move |k| r.config(k)), fixture: None });
    }
    match producer_matrix(&inputs) {
        Ok(md) => std::fs::write(dir.join("producer_matrix.md"), md).expect("write producer_matrix.md"),
        Err(bad) => panic!("producer matrix:\n{}", bad.join("\n")),
    }
}

/// The matrix on synthetic receipts, both directions: complete fixtures pass; a missing hot region
/// and a branch with no region are each named.
#[test]
fn the_matrix_reds_a_gap_and_passes_full_coverage() {
    let full: Vec<MatrixInput<'static>> = FIXTURES
        .iter()
        .map(|f| MatrixInput { name: f.name.to_owned(), cfg: Box::new(|_| 0), fixture: Some((f.name, f.hot != Hot::None)) })
        .collect();
    assert!(producer_matrix(&full).is_ok(), "{:?}", producer_matrix(&full).err());

    let cold_only: Vec<MatrixInput<'static>> = FIXTURES
        .iter()
        .map(|f| MatrixInput { name: f.name.to_owned(), cfg: Box::new(|_| 0), fixture: Some((f.name, f.name != "vb_shade" && f.hot != Hot::None)) })
        .collect();
    let e = producer_matrix(&cold_only).expect_err("a producer with no hot region must red");
    assert!(e.iter().any(|m| m.contains("`vb_shade` has no hot region")), "{e:?}");

    let missing: Vec<MatrixInput<'static>> = FIXTURES
        .iter()
        .filter(|f| f.name != "forward_lit_clear")
        .map(|f| MatrixInput { name: f.name.to_owned(), cfg: Box::new(|_| 0), fixture: Some((f.name, f.hot != Hot::None)) })
        .collect();
    let e = producer_matrix(&missing).expect_err("a branch with no region must red");
    assert!(e.iter().any(|m| m.contains("`forward_lit_clear` is reached by no")), "{e:?}");

    // A transition-pinned leg covers a branch without a fixture: a Forward × Sdf receipt.
    let mut leg_only: Vec<MatrixInput<'static>> = FIXTURES
        .iter()
        .filter(|f| f.name != "forward_lit_clear")
        .map(|f| MatrixInput { name: f.name.to_owned(), cfg: Box::new(|_| 0), fixture: Some((f.name, f.hot != Hot::None)) })
        .collect();
    leg_only.push(MatrixInput {
        name: "sdf_forward_only".to_owned(),
        cfg: Box::new(|k| u64::from(k == "path")),
        fixture: None,
    });
    assert!(producer_matrix(&leg_only).is_ok(), "{:?}", producer_matrix(&leg_only).err());
}

/// The hot oracle is hot: `r = 2.0` through the default curve's tail is above display mid-grey and
/// below white, and a cold `r = 0.5` differs from it (the "region radiance below 2.0" red a cold
/// fixture gets). The oracle follows the curve its words name: ACES Hill and AgX display `r = 2.0`
/// differently.
#[test]
fn the_hot_oracle_separates_hot_from_cold() {
    let words = default_post_words();
    let hot = hot8(words, HOT_R);
    let cold = hot8(words, 0.5);
    println!(
        "the default's words (tonemap word {}): hot {hot:?}, cold {cold:?}, background {:?}",
        tonemap_word(&words),
        display8(words, BACKGROUND_SCENE)
    );
    assert!(hot.iter().all(|v| (200..255).contains(v)), "the default curve at r = {HOT_R} through the OETF: {hot:?}");
    assert!(hot.iter().zip(&cold).all(|(h, c)| h.abs_diff(*c) > 1), "{hot:?} vs {cold:?}");
    let under = |curve: u32| {
        let mut w = words;
        w[ubo_word("mode_words")] = curve << TONEMAP_MODE_SHIFT;
        hot8(w, HOT_R)
    };
    assert_ne!(under(TONEMAP_ACES), under(TONEMAP_AGX), "hot8 does not read the tonemap word of its words");
}

/// A host [`Capture`] whose region is `n` copies of `texel`, with a `[config]` of `keys` and the
/// default configuration's `post_word0..7` (what a fixture frame records).
fn synthetic_capture(texel: [u8; 3], n: usize, keys: &[(&str, u64)]) -> Capture {
    let mut text = String::from("[config]\n");
    for (k, v) in keys {
        text.push_str(&format!("{k} = {v}\n"));
    }
    for (i, w) in default_post_words().iter().enumerate() {
        text.push_str(&format!("post_word{i} = {w}\n"));
    }
    Capture { region: vec![texel; n], receipt: Receipt::parse(&text).expect("invariant: a synthetic receipt parses") }
}

/// `p` moved by `d` codes per channel.
fn shifted(p: [u8; 3], d: [i16; 3]) -> [u8; 3] {
    core::array::from_fn(|k| u8::try_from(i16::from(p[k]) + d[k]).expect("invariant: the probe stays inside 0..=255"))
}

/// **A constant region is judged by the spec's conversion, not by round-half-up** (triage r1 F3):
/// Vulkan's float → UNORM conversion returns either integer nearest the device's value (the RTX
/// 3060 wrote the pre-PX1a blue `0.1 · 255 = 25.5000004` as 25, which a round-half-up oracle
/// refused). Since PX1a that value is `post_final`'s display of the stored constant, so both
/// neighbours of the oracle's rounded code pass and a code two steps off does not. The codes are
/// the default curve's display of the constant ([`display8`]), never spelled.
#[test]
fn a_constant_region_admits_both_spec_neighbours() {
    let words = default_post_words();
    for name in ["deferred_base", "forward_sky_base", "forward_lit_clear"] {
        let f = fixture(name);
        let Expect::Constant(k) = f.expect else { panic!("{name} is a constant fixture") };
        let want = display8(words, k);
        assert!(want.iter().all(|v| (2..=253).contains(v)), "{name}: {want:?} leaves the ±2 probes no room");
        for d in [[0, 0, 0], [1, 1, -1], [-1, 0, 1]] {
            let texel = shifted(want, d);
            let c = synthetic_capture(texel, 64, &[]);
            assert_eq!(check(f, false, &c), Ok(()), "{name}: {texel:?} is a permitted conversion of {want:?}");
        }
        for d in [[0, 0, -2], [2, 0, 0], [0, -2, 1]] {
            let texel = shifted(want, d);
            let c = synthetic_capture(texel, 64, &[]);
            assert!(check(f, false, &c).is_err(), "{name}: {texel:?} is no conversion of {want:?}");
        }
    }
}

/// **A producer region showing only the background is empty, under the same conversion** (triage
/// r1 F3): a guard narrower than the device's conversion let a producer fixture whose region showed
/// nothing but background pass it. One producer texel in the region clears the guard.
#[test]
fn a_background_only_producer_region_is_empty() {
    let f = fixture("forward_sky");
    let keys = [("path_is_forward", 1), ("light_sky", 1)];
    let background = display8(default_post_words(), BACKGROUND_SCENE);
    for texel in [background, shifted(background, [1, 0, -1])] {
        let e = check(f, false, &synthetic_capture(texel, 64, &keys)).expect_err("a background-only region must red");
        assert!(e.contains("region empty"), "{texel:?}: {e}");
    }
    let mut lit = synthetic_capture(background, 64, &keys);
    lit.region[7] = background.map(|v| v.wrapping_add(128));
    assert_eq!(check(f, false, &lit), Ok(()));
}

/// Every fixture name is unique and every lit-store producer has a hot fixture.
#[test]
fn every_lit_store_producer_has_a_hot_fixture() {
    for (i, f) in FIXTURES.iter().enumerate() {
        assert!(FIXTURES[i + 1..].iter().all(|g| g.name != f.name), "duplicate fixture {}", f.name);
    }
    for p in LIT_STORE {
        assert!(FIXTURES.iter().any(|f| f.name == p && f.hot != Hot::None), "{p} has no hot variant");
    }
}
