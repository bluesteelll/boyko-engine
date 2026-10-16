//! Render wave PX0 (H3 Tests; `px0/cut.md` PX0-8; triage r1 W2) — **`post_eval`'s command line**,
//! run through the built binary (`CARGO_BIN_EXE_post_eval`) on synthetic OBSD dumps written the way
//! the recorder writes them (`dump::DumpHeader::to_bytes` + the texels):
//!
//! - [`eval_passes_the_display_its_words_made`] — a display the traced body made under a word set
//!   passes `eval` under those words;
//! - [`another_curve_word_fails_on_most_pixels`] — the same display under another tonemap word
//!   fails on > 90 % of its pixels;
//! - [`a_display_that_ignored_exposure_ratio_fails_where_it_is_not_one`] — and its control;
//! - [`a_missing_binding_is_a_named_error`] — exit 2 naming the row, never a zero texture;
//! - [`bound_equals_the_hand_joint_forward_difference`] — `bound --steps 2` against the joint
//!   forward difference computed here.
//!
//! The scene is stored through B10G11R11 (`post::LIT_STORAGE_FORMAT`), so `bound` steps the shipped
//! encoding. Every reference evaluates the traced body ([`post_final_eval`]) — the oracle `post_eval`
//! instantiates — never a re-composition of its stages.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use boyko_shaderdsl::cf::HostTex2;
use boyko_shaderdsl::dump::DumpHeader;
use boyko_shaderdsl::pixfmt::{self, Rounding};
use boyko_shaderdsl::post::{self, eval_params, post_final_eval};

/// The synthetic frame's extent.
const W: u32 = 16;
const H: u32 = 8;
/// `bound`'s step count in the H3 case (`order_noise_steps = 2`).
const STEPS: i32 = 2;

/// A fresh scratch directory for one test (tests run in parallel; each owns its own).
fn scratch(test: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("post_eval_cli").join(test);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("invariant: CARGO_TARGET_TMPDIR accepts a subdirectory");
    d
}

/// Writes an OBSD image dump of `w × h` texels of `format`.
fn write_dump(path: &Path, format: u32, w: u32, h: u32, texels: &[u8]) {
    let tb = pixfmt::texel_bytes(format).expect("invariant: a decodable format");
    assert_eq!(texels.len(), (w * h * tb) as usize, "invariant: one texel per pixel");
    let header = DumpHeader { format, width: w, height: h, mip: 0, layers: 1, frame: 0, texel_bytes: tb };
    let mut bytes = header.to_bytes().to_vec();
    bytes.extend_from_slice(texels);
    std::fs::write(path, bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

/// The scene: three distinct geometric HDR ramps (one per channel, so a curve that mixes channels
/// sees unequal inputs), stored through B10G11R11 with nearest-even rounding. Returns the dump's
/// words and the decoded texels the body reads.
fn scene() -> (Vec<u32>, Vec<[f32; 4]>) {
    let n = W * H;
    let ramp = |lo: f32, hi: f32, t: f32| lo * (hi / lo).powf(t);
    let words: Vec<u32> = (0..n)
        .map(|i| {
            let t = i as f32 / (n - 1) as f32;
            let rgb = [ramp(0.05, 12.0, t), ramp(0.08, 6.0, 1.0 - t), ramp(0.03, 9.0, (3.0 * t).fract())];
            pixfmt::pack_b10g11r11(rgb, Rounding::NearestEven)
        })
        .collect();
    let texels = words
        .iter()
        .map(|w| {
            let [r, g, b] = pixfmt::unpack_b10g11r11(*w);
            [r, g, b, 1.0]
        })
        .collect();
    (words, texels)
}

/// The eight `PostFinalGpu` words for a tonemap `mode` and an exposure `ratio`, at the frame's extent.
fn words(mode: u32, ratio: f32) -> [u32; 8] {
    let mut w = [0u32; 8];
    w[post::ubo_word("mode_words")] = mode << post::TONEMAP_MODE_SHIFT;
    w[post::ubo_word("exposure_ratio")] = ratio.to_bits();
    w[post::ubo_word("extent")] = W;
    w[post::ubo_word("extent") + 1] = H;
    w
}

/// The 8-bit display the traced body makes from `scene` under `w` (round to nearest per channel).
fn display(w: [u32; 8], scene: &[[f32; 4]]) -> Vec<u8> {
    let p = eval_params(w);
    let tex = HostTex2::new(scene, W, H);
    let mut out = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let o = post_final_eval([x, y], &p, tex);
            out.extend(o[..3].iter().map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8));
            out.push(255);
        }
    }
    out
}

/// Writes the scene dump and the display `display_words` makes; returns their paths.
fn fixture(dir: &Path, display_words: [u32; 8]) -> (PathBuf, PathBuf) {
    let (packed, texels) = scene();
    let scene_path = dir.join("scene.bin");
    let bytes: Vec<u8> = packed.iter().flat_map(|w| w.to_le_bytes()).collect();
    write_dump(&scene_path, pixfmt::VK_FORMAT_B10G11R11_UFLOAT_PACK32, W, H, &bytes);
    let display_path = dir.join("display.bin");
    write_dump(&display_path, pixfmt::VK_FORMAT_R8G8B8A8_UNORM, W, H, &display(display_words, &texels));
    (scene_path, display_path)
}

fn words_arg(w: [u32; 8]) -> String {
    w.map(|v| v.to_string()).join(",")
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_post_eval"))
        .args(args)
        .output()
        .expect("invariant: the post_eval binary spawns")
}

/// `post_eval eval` with the scene bound and the display named.
fn eval(w: [u32; 8], scene: &Path, display: &Path) -> Output {
    let bind = format!("gScene={}", scene.display());
    run(&["eval", "--words", &words_arg(w), "--bind", &bind, "--display", &display.display().to_string()])
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// The `failing_frac=` field of an `eval` summary line.
fn failing_frac(o: &Output) -> f64 {
    let s = String::from_utf8_lossy(&o.stdout);
    let at = s.find("failing_frac=").unwrap_or_else(|| panic!("no failing_frac in: {s}"));
    let rest = &s[at + "failing_frac=".len()..];
    rest[..rest.find(')').expect("failing_frac=<x>)")].parse().expect("a number")
}

#[test]
fn eval_passes_the_display_its_words_made() {
    let dir = scratch("eval_passes");
    let w = words(post::TONEMAP_ACES, 1.0);
    let (scene, display) = fixture(&dir, w);
    let o = eval(w, &scene, &display);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).contains(&format!("{} pixels, 0 over their bound", W * H)), "{}", text(&o));
}

#[test]
fn another_curve_word_fails_on_most_pixels() {
    let dir = scratch("another_curve");
    let (scene, display) = fixture(&dir, words(post::TONEMAP_ACES, 1.0));
    for other in [post::TONEMAP_NEUTRAL, post::TONEMAP_REINHARD_JODIE] {
        let o = eval(words(other, 1.0), &scene, &display);
        assert_eq!(o.status.code(), Some(1), "curve word {other}: {}", text(&o));
        let frac = failing_frac(&o);
        assert!(frac > 0.9, "curve word {other} fails on only {frac} of the pixels: {}", text(&o));
    }
}

#[test]
fn a_display_that_ignored_exposure_ratio_fails_where_it_is_not_one() {
    let dir = scratch("exposure_ratio");
    let honest = words(post::TONEMAP_ACES, 2.0);
    // The display a body that dropped the ratio would make: the ratio-1 frame.
    let (scene, ignored) = fixture(&dir, words(post::TONEMAP_ACES, 1.0));
    let o = eval(honest, &scene, &ignored);
    assert_eq!(o.status.code(), Some(1), "ignoring a ratio of 2 must fail: {}", text(&o));
    assert!(failing_frac(&o) > 0.5, "{}", text(&o));
    // The control: the display that applied it passes under the same words.
    let control = dir.join("display_ratio2.bin");
    write_dump(&control, pixfmt::VK_FORMAT_R8G8B8A8_UNORM, W, H, &display(honest, &scene_texels()));
    let o = eval(honest, &scene, &control);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
}

fn scene_texels() -> Vec<[f32; 4]> {
    scene().1
}

#[test]
fn a_missing_binding_is_a_named_error() {
    let dir = scratch("missing_binding");
    let w = words(post::TONEMAP_ACES, 1.0);
    let (_, display) = fixture(&dir, w);
    let named = "binding `gScene` has no dump: pass --bind gScene=<dump.bin>";
    let o = run(&["eval", "--words", &words_arg(w), "--display", &display.display().to_string()]);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains(named), "{}", text(&o));
    let o = run(&["bound", "--words", &words_arg(w), "--steps", &STEPS.to_string()]);
    assert_eq!(o.status.code(), Some(2), "{}", text(&o));
    assert!(text(&o).contains(named), "{}", text(&o));
}

/// The joint forward difference at `±steps` format steps, by hand: per pixel, the eight channel
/// sign patterns applied to the CENTRE texel together (the footprint at
/// `POST_FINAL_TAP_RADIUS = 0`), the largest display change over the three channels, maximised over
/// pixels and patterns. `channels` restricts which channels a pattern perturbs (all three is the
/// joint bound; one is the non-joint variant the test proves it can tell apart).
fn hand_bound(w: [u32; 8], scene: &[[f32; 4]], steps: i32, channels: usize) -> f64 {
    let p = eval_params(w);
    let mut worst = 0.0f64;
    for y in 0..H {
        for x in 0..W {
            let i = (y * W + x) as usize;
            let c0 = scene[i];
            let base = post_final_eval([x, y], &p, HostTex2::new(scene, W, H));
            for pattern in 0..8u32 {
                let mut s = scene.to_vec();
                for c in 0..channels {
                    let k = if (pattern >> c) & 1 == 1 { steps } else { -steps };
                    if let Ok(v) = pixfmt::step_by(pixfmt::B10G11R11_CHANNELS[c], c0[c], k) {
                        s[i][c] = v;
                    }
                }
                let v = post_final_eval([x, y], &p, HostTex2::new(&s, W, H));
                for c in 0..3 {
                    worst = worst.max(255.0 * f64::from((v[c] - base[c]).abs()));
                }
            }
        }
    }
    worst
}

#[test]
fn bound_equals_the_hand_joint_forward_difference() {
    assert_eq!(
        post::POST_FINAL_TAP_RADIUS,
        0,
        "the hand difference below perturbs the centre texel only; a wider footprint (PX1b) extends it"
    );
    let dir = scratch("bound");
    let w = words(post::TONEMAP_ACES, 1.0);
    let (scene, _) = fixture(&dir, w);
    let bind = format!("gScene={}", scene.display());
    let o = run(&["bound", "--words", &words_arg(w), "--bind", &bind, "--steps", &STEPS.to_string()]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let out = String::from_utf8_lossy(&o.stdout);
    let cli: f64 = out
        .trim()
        .strip_prefix("max_delta_bound=")
        .unwrap_or_else(|| panic!("no max_delta_bound line: {out}"))
        .parse()
        .expect("a number");
    let texels = scene_texels();
    let joint = hand_bound(w, &texels, STEPS, 3);
    let one_channel = hand_bound(w, &texels, STEPS, 1);
    // Non-vacuity: two steps move the display, and the joint bound is not the one-channel one, so
    // a `bound` that perturbed one channel would be told apart here.
    assert!(joint > 0.5, "two B10G11R11 steps must move the display: {joint}");
    assert!((joint - one_channel).abs() > 1e-3, "joint {joint} vs one channel {one_channel}");
    // `bound` prints six decimals.
    assert!((cli - joint).abs() <= 1e-6, "post_eval bound {cli} != the hand joint forward difference {joint}");
}
