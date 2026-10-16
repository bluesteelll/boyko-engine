//! Render wave PX0 (H3 Tests; `px0/cut.md` PX0-8; triage r1 W2) — **`post_eval`'s command line**,
//! run through the built binary (`CARGO_BIN_EXE_post_eval`) on synthetic OBSD dumps written the way
//! the recorder writes them (`dump::DumpHeader::to_bytes` + the texels):
//!
//! - [`eval_passes_the_display_its_words_made`] — a display the traced body made under a word set
//!   passes `eval` under those words;
//! - [`eval_floor_admits_the_far_neighbour_and_refuses_any_one_channel_past_the_pair`] — the display
//!   at the other code the UNORM store may return passes; a display with one channel of every pixel
//!   at the closest code outside that pair fails everywhere, for each channel in turn;
//! - [`another_curve_word_fails_on_most_pixels`] — the same display under another tonemap word
//!   fails on > 90 % of its pixels;
//! - [`a_display_that_ignored_exposure_ratio_fails_where_it_is_not_one`] — and its control;
//! - [`a_missing_binding_is_a_named_error`] — exit 2 naming the row, never a zero texture;
//! - [`bound_equals_the_hand_joint_forward_difference`] — `bound --steps 2` against the joint
//!   forward difference computed here, under four word sets: the three curve words at exposure
//!   ratio 1, two of whose maxima sit at a mixed-sign pattern, and ACES at ratio 2, whose maximum
//!   is the all-up pattern alone.
//!
//! The synthetic scene is a B10G11R11 dump, the coarser of the two `lit` formats PX0 weighed.
//! `bound` steps whatever encoding the dump's header names, not `post::LIT_STORAGE_FORMAT`
//! (RGBA16F since PX0-14). Every reference evaluates the traced body ([`post_final_eval`]) — the
//! oracle `post_eval` instantiates — never a re-composition of its stages.

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

/// The display whose channel `c` is the code `pick(c, v)` chooses for the body's value `v = 255·o`
/// (`o` the traced body's output under `w`, unclamped; the product in f64, exact for an f32), with
/// each channel's smallest and largest distance over the frame in LSB, measured as `eval` measures
/// it.
fn display_picked(w: [u32; 8], scene: &[[f32; 4]], pick: impl Fn(usize, f64) -> f64) -> (Vec<u8>, [f64; 3], [f64; 3]) {
    let p = eval_params(w);
    let tex = HostTex2::new(scene, W, H);
    let mut out = Vec::with_capacity((W * H * 4) as usize);
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [0.0f64; 3]);
    for y in 0..H {
        for x in 0..W {
            let o = post_final_eval([x, y], &p, tex);
            for (c, &oc) in o[..3].iter().enumerate() {
                let v = 255.0 * f64::from(oc);
                assert!((0.0..=255.0).contains(&v), "PREMISE: the body's value {oc} at ({x}, {y}) is a display value");
                let code = pick(c, v);
                assert!((0.0..=255.0).contains(&code), "PREMISE: code {code} for v = {v} is a byte");
                let byte = code as u8;
                let d = 255.0 * f64::from((f32::from(byte) / 255.0 - oc).abs());
                lo[c] = lo[c].min(d);
                hi[c] = hi[c].max(d);
                out.push(byte);
            }
            out.push(255);
        }
    }
    (out, lo, hi)
}

/// The integer nearest `v` that round-to-nearest does NOT pick — the other code Vulkan's float →
/// UNORM conversion may return (it returns ONE OF the two integers nearest `v`;
/// `hdr_branch_fixtures.rs`'s `constant8`, triage r1 F3) — and `v` itself when `v` is integral.
fn far_neighbour(v: f64) -> f64 {
    if v.round() == v.floor() { v.ceil() } else { v.floor() }
}

/// One code past [`far_neighbour`], away from the nearest code: `2 − |v − round(v)|` ≥ 1.5 LSB from
/// `v`. Toward the range's interior when `v` is integral; mirrored (`2 + |v − round(v)|`) where it
/// would leave `[0, 255]`.
fn past_far_neighbour(v: f64) -> f64 {
    let n = v.round();
    let s = if v > n {
        1.0
    } else if v < n {
        -1.0
    } else if n < 128.0 {
        1.0
    } else {
        -1.0
    };
    let c = n + 2.0 * s;
    if (0.0..=255.0).contains(&c) { c } else { n - 2.0 * s }
}

/// One code past the NEAREST code, on the side away from [`far_neighbour`]: `1 + |v − round(v)|`
/// from `v`, in (1, 1.5] — the closest code outside the pair the store may return. Where that code
/// would leave `[0, 255]`, or `v` is integral (both `v ± 1` are then exactly one code away, at the
/// floor itself), [`past_far_neighbour`], at least 1.5 codes away.
fn past_nearest(v: f64) -> f64 {
    let n = v.round();
    let c = if v > n { n - 1.0 } else { n + 1.0 };
    if v != n && (0.0..=255.0).contains(&c) { c } else { past_far_neighbour(v) }
}

/// **`eval`'s rounding floor is one display code, to within a thousandth of a code** (triage r2 G2,
/// r3 W1). The float → UNORM store may return either integer nearest the body's value, so the
/// floor must admit the OTHER one and refuse every code outside that pair:
///
/// - the display at the other code ([`far_neighbour`]) passes. Its largest channel distance is the
///   LOWER pin (0.9996 LSB on this scene): a floor below it, less that pixel's OETF term, reds;
/// - three displays, one per channel, put THAT channel of every pixel at the closest code outside
///   the pair ([`past_nearest`]) and the other two at the nearest code (at most half a code away).
///   Each fails on every pixel. `eval` fails a pixel when ANY channel is over its bound, so each
///   pixel's verdict is its deciding channel's alone, and the smallest deciding distance over the
///   three displays is the UPPER pin (1.00041 LSB, red, on this scene): a floor at or above it,
///   less that pixel's OETF term, passes a pixel and reds here — and so does an `eval` that skips a
///   channel, whose display then passes outright.
///
/// So the floor is pinned to about [0.9996, 1.0004) LSB, and the premises hold both pins within
/// 1e-3 LSB of 1. (A display with EVERY channel one code past the far neighbour decides each pixel
/// by its largest distance; it pinned only [0.9996, 1.616) and passed a skipped channel.) E0's OETF
/// term, at most 1.3e-4 LSB, is smaller than the margin by which these displays clear the floor,
/// so `post_eval.rs`'s unit tests pin it, not a display; no row here is filtered.
#[test]
fn eval_floor_admits_the_far_neighbour_and_refuses_any_one_channel_past_the_pair() {
    let dir = scratch("eval_floor");
    let w = words(post::TONEMAP_ACES, 1.0);
    let (scene, _) = fixture(&dir, w);
    let texels = scene_texels();

    let (far, _, far_hi) = display_picked(w, &texels, |_, v| far_neighbour(v));
    let far_hi = far_hi.iter().copied().fold(0.0f64, f64::max);
    assert!(far_hi < 1.0 && far_hi > 1.0 - 1e-3, "PREMISE: the far display comes within 1e-3 code of the floor: max {far_hi}");
    let far_path = dir.join("display_far.bin");
    write_dump(&far_path, pixfmt::VK_FORMAT_R8G8B8A8_UNORM, W, H, &far);
    let o = eval(w, &scene, &far_path);
    assert_eq!(o.status.code(), Some(0), "the other permitted code (up to {far_hi:.4} LSB) must pass: {}", text(&o));
    assert!(text(&o).contains(&format!("{} pixels, 0 over their bound", W * H)), "{}", text(&o));

    let mut deciding_lo = f64::INFINITY;
    for (c, channel) in ["red", "green", "blue"].into_iter().enumerate() {
        let (past, lo, hi) = display_picked(w, &texels, |k, v| if k == c { past_nearest(v) } else { v.round() });
        for k in (0..3).filter(|&k| k != c) {
            assert!(hi[k] <= 0.5 + 1e-3, "PREMISE: [{channel}] channel {k} sits at the nearest code: max {}", hi[k]);
        }
        // 2e-4 is the unit tests' bound on E0's OETF term: past it, the floor alone decides.
        assert!(lo[c] > 1.0 + 2e-4, "PREMISE: [{channel}] every deciding value is outside the pair, past the OETF term: min {}", lo[c]);
        deciding_lo = deciding_lo.min(lo[c]);
        let past_path = dir.join(format!("display_past_{channel}.bin"));
        write_dump(&past_path, pixfmt::VK_FORMAT_R8G8B8A8_UNORM, W, H, &past);
        let o = eval(w, &scene, &past_path);
        assert_eq!(o.status.code(), Some(1), "[{channel}] one channel past the pair (>= {:.6} LSB) must fail: {}", lo[c], text(&o));
        assert_eq!(failing_frac(&o), 1.0, "[{channel}] every pixel must fail on its {channel} channel: {}", text(&o));
    }
    assert!(deciding_lo < 1.0 + 1e-3, "PREMISE: a deciding channel comes within 1e-3 code of the floor: min {deciding_lo}");
    eprintln!("eval's rounding floor pinned to [{far_hi:.6}, {deciding_lo:.6}) LSB, each end less its pixel's OETF term");
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

/// The joint forward difference at `±steps` format steps, by hand, PER SIGN PATTERN: for each of
/// the eight channel sign patterns (bit `c` set = channel `c` up) applied to the CENTRE texel (the
/// footprint at `POST_FINAL_TAP_RADIUS = 0`), the largest display change over the three channels,
/// maximised over pixels. `bound` is the maximum over the eight. `channels` restricts which
/// channels a pattern perturbs (all three is the joint bound; one is the non-joint variant the
/// test proves it can tell apart).
fn hand_patterns(w: [u32; 8], scene: &[[f32; 4]], steps: i32, channels: usize) -> [f64; 8] {
    let p = eval_params(w);
    let mut per = [0.0f64; 8];
    for y in 0..H {
        for x in 0..W {
            let i = (y * W + x) as usize;
            let c0 = scene[i];
            let base = post_final_eval([x, y], &p, HostTex2::new(scene, W, H));
            for (pattern, worst) in per.iter_mut().enumerate() {
                let mut s = scene.to_vec();
                for c in 0..channels {
                    let k = if (pattern >> c) & 1 == 1 { steps } else { -steps };
                    if let Ok(v) = pixfmt::step_by(pixfmt::B10G11R11_CHANNELS[c], c0[c], k) {
                        s[i][c] = v;
                    }
                }
                let v = post_final_eval([x, y], &p, HostTex2::new(&s, W, H));
                for c in 0..3 {
                    *worst = worst.max(255.0 * f64::from((v[c] - base[c]).abs()));
                }
            }
        }
    }
    per
}

/// Where a word set's per-pattern maximum must sit on this scene, so a `bound` that skips that
/// pattern prints low and is told apart from the hand difference.
#[derive(Clone, Copy)]
enum Argmax {
    /// No constraint.
    Any,
    /// A MIXED-sign pattern — neither all-down (0) nor all-up (7) — by more than 1e-3 LSB over
    /// both (triage r2 G1).
    Mixed,
    /// The all-up pattern (7) alone, by more than 1e-3 LSB over every other (triage r3 W2).
    AllUpAlone,
}

/// The word sets `bound` is checked under: (name, curve word, exposure ratio, where the maximum
/// sits). Under ACES at ratio 1 the maximum on this scene is pattern 0, so that set alone cannot see
/// a `bound` that stops enumerating the mixed patterns; under Neutral and Reinhard-Jodie it is mixed
/// (patterns 2/3 and 4), and a uniform-sign-only `bound` would print 34 % and 24 % low. Under ACES
/// at ratio 2 it is pattern 7 alone (by 0.0149 LSB), so a `bound` that enumerates `0..7` — the
/// off-by-one that drops the all-up pattern — prints the second-largest pattern's maximum instead.
const BOUND_CASES: [(&str, u32, f32, Argmax); 4] = [
    ("aces", post::TONEMAP_ACES, 1.0, Argmax::Any),
    ("neutral", post::TONEMAP_NEUTRAL, 1.0, Argmax::Mixed),
    ("reinhard_jodie", post::TONEMAP_REINHARD_JODIE, 1.0, Argmax::Mixed),
    ("aces_ratio2", post::TONEMAP_ACES, 2.0, Argmax::AllUpAlone),
];

#[test]
fn bound_equals_the_hand_joint_forward_difference() {
    assert_eq!(
        post::POST_FINAL_TAP_RADIUS,
        0,
        "the hand difference below perturbs the centre texel only; a wider footprint (PX1b) extends it"
    );
    let dir = scratch("bound");
    let (scene, _) = fixture(&dir, words(post::TONEMAP_ACES, 1.0));
    let bind = format!("gScene={}", scene.display());
    let texels = scene_texels();
    let max = |per: [f64; 8]| per.iter().copied().fold(0.0f64, f64::max);
    for (name, mode, ratio, argmax) in BOUND_CASES {
        let w = words(mode, ratio);
        let o = run(&["bound", "--words", &words_arg(w), "--bind", &bind, "--steps", &STEPS.to_string()]);
        assert_eq!(o.status.code(), Some(0), "[{name}] {}", text(&o));
        let out = String::from_utf8_lossy(&o.stdout);
        let cli: f64 = out
            .trim()
            .strip_prefix("max_delta_bound=")
            .unwrap_or_else(|| panic!("[{name}] no max_delta_bound line: {out}"))
            .parse()
            .expect("a number");
        let per = hand_patterns(w, &texels, STEPS, 3);
        let joint = max(per);
        let one_channel = max(hand_patterns(w, &texels, STEPS, 1));
        // Non-vacuity: two steps move the display, and the joint bound is not the one-channel one, so
        // a `bound` that perturbed one channel would be told apart here.
        assert!(joint > 0.5, "[{name}] two B10G11R11 steps must move the display: {joint}");
        assert!((joint - one_channel).abs() > 1e-3, "[{name}] joint {joint} vs one channel {one_channel}");
        // ... and where the set's maximum is pinned to a pattern, a `bound` that skips it would be
        // told apart too: a mixed-sign maximum against one restricted to the uniform-sign patterns
        // (or to pattern 0), the all-up maximum against one that stops at pattern 6.
        match argmax {
            Argmax::Any => {}
            Argmax::Mixed => {
                let uniform = per[0].max(per[7]);
                assert!(
                    joint - uniform > 1e-3,
                    "[{name}] the maximum must sit at a mixed-sign pattern: per-pattern maxima {per:?}"
                );
            }
            Argmax::AllUpAlone => {
                let others = per[..7].iter().copied().fold(0.0f64, f64::max);
                assert!(
                    per[7] - others > 1e-3,
                    "[{name}] the maximum must sit at the all-up pattern alone: per-pattern maxima {per:?}"
                );
            }
        }
        // `bound` prints six decimals.
        assert!((cli - joint).abs() <= 1e-6, "[{name}] post_eval bound {cli} != the hand joint forward difference {joint}");
    }
}
