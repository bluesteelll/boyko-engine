//! Render wave PX1a test 10 (`plan/H4-PX1a-hdr-lit.md` Tests 10; `px1a/cut.md` C4, D-G1..D-G4, D-O1,
//! D-O3): **the post seam's structure, read from the production frame-graph receipt (R).**
//!
//! Each driver boots one configuration with `BOYKO_GRAPH_RECEIPT` armed and requires of the
//! receipt ([`check_post_seam`]):
//!
//! - **`post_final` is declared** on every declarator — Deferred, VisibilityBuffer, Forward and
//!   ForwardPlus (the Forward family's post seam is new);
//! - **its accesses follow `post::POST_FINAL_BINDINGS`** (D-O1): the ONE fetching row `gScene` ⇒ the
//!   first access reads the scene input at `COMPUTE / SHADER_READ / GENERAL` — `taa_hist` under
//!   TAA, `lit_native` under SSAA, `lit` otherwise — and the store row ⇒ the second writes
//!   `display` at `COMPUTE / SHADER_WRITE / GENERAL`; nothing else;
//! - **`display` is seeded at the boundary layout `SHADER_READ_ONLY_OPTIMAL`, FrameLocal**, its seed
//!   naming `FRAGMENT | COMPUTE`, and the frame's last declared access of it (before any image-dump
//!   pass) is `present_sample`'s FRAGMENT read at that layout;
//! - **the order**: `post_final` follows every `lit` producer (`taa_resolve` under TAA,
//!   `ssaa_downsample` under SSAA, `particle_draw` when particles run) and precedes `present_sample`;
//!   the CAS sharpen is a declared `cas` pass between them iff it is armed, reading `display` at
//!   `COMPUTE / GENERAL` (D-G3); under SSAA `ssaa_downsample` writes `lit_native` as a colour
//!   attachment;
//! - **`[config]` carries the frame's `PostFinalGpu`** under `post::POST_FINAL_RECEIPT_KEYS` (D-O3):
//!   an exposure ratio of exactly 1.0 and the dispatch extent — `display`'s extent, the native one
//!   only on a frame that runs the SSAA downsample (D-G1).
//!
//! One driver reads images instead of the receipt (review r2 R2-W1): on Forward under a boot-armed
//! SSAA it dumps `lit` and `display` and requires [`check_presents_the_whole_scene`] — `display`,
//! reduced to the native extent the window shows, depicts the whole scene `lit` holds, not its
//! top-left quarter. The seam check cannot see that: the seam is the same either way.
//!
//! The non-ignored tests run [`check_post_seam`] and [`check_presents_the_whole_scene`] over fixtures
//! they must accept and reject, every run. Windowed-test conventions: the drivers are ignored,
//! `--test-threads=1`, `BOYKO_DISABLE_VALIDATION=1`; each driver sets its own frame budget.

#[path = "common/graph_receipt.rs"]
mod graph_receipt;
#[path = "common/image_dump.rs"]
mod image_dump;

use boyko_app::EnginePlugins;
use boyko_render::{AaConfig, AaMode, RenderPath, SharpenMode, TaaConfig};
use boyko_rhi_vulkan::ffi::{
    VK_ACCESS_SHADER_READ_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_IMAGE_LAYOUT_GENERAL,
    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
    VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
};
use boyko_shaderdsl::post::POST_FINAL_RECEIPT_KEYS;

use graph_receipt::{
    ENV_DDGI, ENV_INTERP, ENV_PARTICLES, ENV_PATH, ENV_TAA, ENV_UPLOAD, OBS_WIN, ObsArming, Receipt,
    assert_obs_resolved, build_obs_app_with, fresh_out, run_worker,
};
use image_dump::{DumpFile, VK_R8G8B8A8_UNORM, VK_R32G32B32A32_SFLOAT, encode, files_of};

/// The worker's AA knob (`off | taa | taa_cas | fxaa | smaa | ssaa`).
const ENV_AA: &str = "BOYKO_PX1A_AA";
/// Frames before the receipt frame.
const SETTLE: u32 = 30;
/// [`check_presents_the_whole_scene`]'s bar: the least rank correlation between `display` and `lit`,
/// both reduced to the native extent. Measured on the device at `6c163cf7` (triage r2,
/// `scratch/tri2/analyze.out`): a whole-scene `display` ranks at 0.997-0.999 against the scene, a
/// top-left-quarter one at -0.35.
const WHOLE_SCENE_RHO_MIN: f64 = 0.9;
/// [`check_presents_the_whole_scene`]'s power premise: the most the scene's top-left quarter, read
/// 1:1, may rank like the whole scene. Above it a quarter-reading `display` could pass the bar.
/// Measured on the OBS Forward scene at `6c163cf7`: -0.35.
const QUARTER_RHO_MAX: f64 = 0.5;

/// One configuration's AA arming.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Aa {
    Off,
    Taa,
    TaaCas,
    Fxaa,
    Smaa,
    Ssaa,
}

impl Aa {
    fn word(self) -> &'static str {
        match self {
            Aa::Off => "off",
            Aa::Taa => "taa",
            Aa::TaaCas => "taa_cas",
            Aa::Fxaa => "fxaa",
            Aa::Smaa => "smaa",
            Aa::Ssaa => "ssaa",
        }
    }

    fn parse(w: &str) -> Self {
        match w.trim() {
            "off" => Aa::Off,
            "taa" => Aa::Taa,
            "taa_cas" => Aa::TaaCas,
            "fxaa" => Aa::Fxaa,
            "smaa" => Aa::Smaa,
            "ssaa" => Aa::Ssaa,
            other => panic!("{ENV_AA}={other} is not off|taa|taa_cas|fxaa|smaa|ssaa"),
        }
    }

    /// The pass name and resource the post_final scene read must be.
    fn scene_input(self) -> &'static str {
        match self {
            Aa::Taa | Aa::TaaCas => "taa_hist",
            Aa::Ssaa => "lit_native",
            _ => "lit",
        }
    }
}

// ===============================================================================================
// The check
// ===============================================================================================

/// One `[[access]]` row.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AccessRow {
    pass: u64,
    res: u64,
    stage: u64,
    access: u64,
    layout: i64,
}

fn accesses(r: &Receipt) -> Vec<AccessRow> {
    r.array("access")
        .map(|t| AccessRow {
            pass: t.u64("pass"),
            res: t.u64("res"),
            stage: t.u64("stage"),
            access: t.u64("access"),
            layout: t.i64("layout"),
        })
        .collect()
}

fn pass_index(passes: &[String], name: &str) -> Option<u64> {
    passes.iter().position(|p| p == name).map(|i| i as u64)
}

/// The receipt's post seam against the configuration (module doc); `display_edge` is the edge of
/// the square `display` the frame dispatches `post_final` over.
fn check_post_seam(r: &Receipt, aa: Aa, particles: bool, display_edge: u32) -> Result<(), String> {
    let passes = r.pass_names();
    let res = r.resources();
    let name_of = |i: u64| res.iter().find(|row| row.index == i).map_or("<none>", |row| row.name.as_str());
    let acc = accesses(r);
    let post = pass_index(&passes, "post_final").ok_or("no pass `post_final`")?;
    let present = pass_index(&passes, "present_sample").ok_or("no pass `present_sample`")?;
    // Render wave PX1a C11 (zero cost off): the test-only NaN injection is never declared on a run
    // without the `inject_nan_lit` force key (these drivers arm none).
    if pass_index(&passes, "test_nan_inject").is_some() {
        return Err("the test-only pass `test_nan_inject` is declared on a run without its force key".to_owned());
    }

    // The bindings' order (D-O1).
    let mine: Vec<&AccessRow> = acc.iter().filter(|a| a.pass == post).collect();
    if mine.len() != 2 {
        return Err(format!("post_final declares {} accesses, POST_FINAL_BINDINGS has two declared rows", mine.len()));
    }
    let want = aa.scene_input();
    let scene = mine[0];
    if name_of(scene.res) != want
        || scene.stage != u64::from(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT)
        || scene.access != u64::from(VK_ACCESS_SHADER_READ_BIT)
        || scene.layout != i64::from(VK_IMAGE_LAYOUT_GENERAL)
    {
        return Err(format!(
            "post_final's first access (gScene) is {:?} on `{}`; expected a COMPUTE SHADER_READ of `{want}` at GENERAL",
            scene,
            name_of(scene.res)
        ));
    }
    let store = mine[1];
    if name_of(store.res) != "display"
        || store.stage != u64::from(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT)
        || store.access != u64::from(VK_ACCESS_SHADER_WRITE_BIT)
        || store.layout != i64::from(VK_IMAGE_LAYOUT_GENERAL)
    {
        return Err(format!("post_final's second access (gDisplay) is {store:?} on `{}`", name_of(store.res)));
    }

    // `display`: FrameLocal, seeded at SHADER_READ_ONLY_OPTIMAL, with FRAGMENT | COMPUTE readers.
    let display = res.iter().find(|row| row.name == "display").ok_or("no resource `display`")?;
    if display.derived_class() != "FrameLocal" || display.seed_layout != i64::from(VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL) {
        return Err(format!("display is {} at seed layout {} (want FrameLocal at SHADER_READ_ONLY_OPTIMAL)", display.derived_class(), display.seed_layout));
    }
    let need = u64::from(VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT | VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT);
    if display.seed_stages() & need != need {
        return Err(format!("display's seed stages {:#x} lack FRAGMENT | COMPUTE", display.seed_stages()));
    }
    let dump_from = pass_index(&passes, "image_dump").unwrap_or(u64::MAX);
    let last = acc.iter().rev().find(|a| a.res == display.index && a.pass < dump_from).ok_or("display is never accessed")?;
    if last.pass != present
        || last.stage != u64::from(VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT)
        || last.layout != i64::from(VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL)
    {
        return Err(format!("display's last access is {last:?} in `{}`, not present_sample's FRAGMENT read at SRO", passes[last.pass as usize]));
    }

    // The order.
    if post > present {
        return Err("post_final is declared after present_sample".to_owned());
    }
    let producers: &[&str] = match aa {
        Aa::Taa | Aa::TaaCas => &["taa_resolve"],
        Aa::Ssaa => &["ssaa_downsample"],
        _ => &[],
    };
    for p in producers.iter().copied().chain(particles.then_some("particle_draw")) {
        let i = pass_index(&passes, p).ok_or_else(|| format!("no pass `{p}` on this configuration"))?;
        if i > post {
            return Err(format!("`{p}` is declared after post_final"));
        }
    }
    let cas = pass_index(&passes, "cas");
    match (aa, cas) {
        (Aa::TaaCas, None) => return Err("TAA + CAS declares no `cas` pass".to_owned()),
        (Aa::TaaCas, Some(c)) => {
            if !(post < c && c < present) {
                return Err("`cas` is not between post_final and present_sample".to_owned());
            }
            let read = acc
                .iter()
                .find(|a| a.pass == c && a.res == display.index)
                .ok_or("`cas` does not read display")?;
            if read.stage != u64::from(VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT) || read.layout != i64::from(VK_IMAGE_LAYOUT_GENERAL) {
                return Err(format!("`cas` reads display as {read:?}, not a COMPUTE read at GENERAL"));
            }
        }
        (_, Some(_)) => return Err(format!("a `cas` pass on a {aa:?} frame")),
        (_, None) => {}
    }
    if aa == Aa::Ssaa {
        let ds = pass_index(&passes, "ssaa_downsample").ok_or("no pass `ssaa_downsample`")?;
        let native = res.iter().find(|row| row.name == "lit_native").ok_or("no resource `lit_native`")?;
        if !acc.iter().any(|a| a.pass == ds && a.res == native.index && a.access & u64::from(VK_ACCESS_SHADER_READ_BIT) == 0) {
            return Err("ssaa_downsample does not write lit_native".to_owned());
        }
    }
    if res.iter().any(|row| row.name == "taa_resolved" || row.name == "aa_out") {
        return Err("taa_resolved / aa_out is a graph resource".to_owned());
    }

    // `[config]`: the frame's PostFinalGpu (D-O3).
    let cfg = r.single("config");
    for k in POST_FINAL_RECEIPT_KEYS {
        if cfg.get(k).is_none() {
            return Err(format!("[config] has no `{k}`"));
        }
    }
    let ratio = f32::from_bits(r.config(POST_FINAL_RECEIPT_KEYS[1]) as u32);
    if ratio != 1.0 {
        return Err(format!("[config] post_word1 (exposure_ratio) = {ratio}, not 1.0 before PX3"));
    }
    let (w, h) = (r.config(POST_FINAL_RECEIPT_KEYS[2]), r.config(POST_FINAL_RECEIPT_KEYS[3]));
    if w != u64::from(display_edge) || h != u64::from(display_edge) {
        return Err(format!("[config] post_word2/3 (extent) = {w}x{h}, not the {display_edge}^2 display"));
    }
    Ok(())
}

// ===============================================================================================
// The whole-scene check (review r2 R2-W1)
// ===============================================================================================

/// Rec. 709 luminance of a decoded texel; a non-finite channel reads as 0 (`hdr_sanitize`'s rule).
fn luma(t: [f32; 4]) -> f64 {
    let c = |v: f32| if v.is_finite() { f64::from(v) } else { 0.0 };
    0.2126 * c(t[0]) + 0.7152 * c(t[1]) + 0.0722 * c(t[2])
}

/// `img`'s luminance box-reduced to `edge`², row-major. `img` must be square and a whole multiple
/// of `edge`.
fn reduced_luma(img: &DumpFile, edge: u32) -> Result<Vec<f64>, String> {
    if img.width != img.height || img.width < edge || img.width % edge != 0 {
        return Err(format!("a {}x{} image is not a whole multiple of the {edge}^2 native extent", img.width, img.height));
    }
    let k = img.width / edge;
    let mut out = vec![0.0f64; (edge * edge) as usize];
    for y in 0..img.height {
        for x in 0..img.width {
            out[((y / k) * edge + x / k) as usize] += luma(img.texel_f32(x, y));
        }
    }
    let n = f64::from(k * k);
    for v in &mut out {
        *v /= n;
    }
    Ok(out)
}

/// The ranks of `v`, ties at their average rank.
fn ranks(v: &[f64]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..v.len()).collect();
    order.sort_by(|&a, &b| v[a].total_cmp(&v[b]));
    let mut out = vec![0.0f64; v.len()];
    let mut i = 0;
    while i < order.len() {
        let mut j = i;
        while j + 1 < order.len() && v[order[j + 1]] == v[order[i]] {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0;
        for &k in &order[i..=j] {
            out[k] = avg;
        }
        i = j + 1;
    }
    out
}

/// Spearman's rank correlation of `a` and `b` (equal lengths); 0 when either is constant.
fn spearman(a: &[f64], b: &[f64]) -> f64 {
    let (ra, rb) = (ranks(a), ranks(b));
    let n = ra.len() as f64;
    let (ma, mb) = (ra.iter().sum::<f64>() / n, rb.iter().sum::<f64>() / n);
    let (mut sab, mut saa, mut sbb) = (0.0f64, 0.0f64, 0.0f64);
    for (x, y) in ra.iter().zip(&rb) {
        let (dx, dy) = (x - ma, y - mb);
        sab += dx * dy;
        saa += dx * dx;
        sbb += dy * dy;
    }
    if saa == 0.0 || sbb == 0.0 { 0.0 } else { sab / (saa * sbb).sqrt() }
}

/// Review r2 R2-W1: on a boot-armed 2x SSAA, the `display` the window shows depicts the WHOLE scene
/// the 2x `lit` holds. Returns the rank correlation it measured.
///
/// - **Premise:** `lit` is the 2x composite of the `native`² window; otherwise the boot SSAA did not
///   arm and the run is vacuous.
/// - **Power:** the scene's top-left quarter, read 1:1 at `native`², ranks like the whole scene
///   (`lit` box-reduced 2:1) at most [`QUARTER_RHO_MAX`]; otherwise a `display` holding that
///   quarter could pass.
/// - **The check:** `display`, box-reduced to `native`² (2:1 when it is the composite, 1:1 when it
///   is native), ranks like the whole scene at least [`WHOLE_SCENE_RHO_MIN`]. Rank correlation of
///   luminance, because the tonemap and the OETF between `lit` and `display` are monotone; the
///   reduction makes the check blind to the route `display` takes (a native `display` from a
///   downsampled scene, or a composite one the present minifies).
fn check_presents_the_whole_scene(lit: &DumpFile, display: &DumpFile, native: u32) -> Result<f64, String> {
    if (lit.width, lit.height) != (2 * native, 2 * native) {
        return Err(format!(
            "lit is {}x{}, not the 2x composite of the {native}^2 window: the boot SSAA did not arm, so the run is vacuous",
            lit.width, lit.height
        ));
    }
    let whole = reduced_luma(lit, native)?;
    let quarter: Vec<f64> = (0..native).flat_map(|y| (0..native).map(move |x| (x, y))).map(|(x, y)| luma(lit.texel_f32(x, y))).collect();
    let quarter_rho = spearman(&quarter, &whole);
    if quarter_rho > QUARTER_RHO_MAX {
        return Err(format!(
            "the scene's top-left quarter ranks like the whole scene ({quarter_rho:.4} > {QUARTER_RHO_MAX}): the check has no power on it"
        ));
    }
    let shown = reduced_luma(display, native)?;
    let rho = spearman(&shown, &whole);
    if rho < WHOLE_SCENE_RHO_MIN {
        return Err(format!(
            "display ({}x{}) reduced to {native}^2 does not depict the whole scene: rank correlation {rho:.4} with lit's 2:1 reduction (< {WHOLE_SCENE_RHO_MIN}), {:.4} with lit's top-left quarter",
            display.width,
            display.height,
            spearman(&shown, &quarter)
        ));
    }
    Ok(rho)
}

// ===============================================================================================
// Host fixtures (not ignored): the check accepts a correct seam and rejects each broken one
// ===============================================================================================

const GENERAL: i64 = 1;
const SRO: i64 = 5;
const CS: u64 = 0x800;
const FS: u64 = 0x80;

/// A receipt with resources `lit` (0), `taa_hist` (1), `display` (2) and the given passes /
/// accesses `(pass, res, stage, access, layout)`.
fn fixture_receipt(passes: &[&str], accesses: &[(u64, u64, u64, u64, i64)], post_words: bool) -> Receipt {
    let mut t = String::from("schema_version = 1\n\n[config]\npath = 0\n");
    if post_words {
        t.push_str(&format!(
            "post_word0 = 0\npost_word1 = {}\npost_word2 = {OBS_WIN}\npost_word3 = {OBS_WIN}\npost_word4 = 0\npost_word5 = 0\npost_word6 = 0\npost_word7 = 0\n",
            1.0f32.to_bits()
        ));
    }
    for (i, n) in ["lit", "taa_hist", "display"].iter().enumerate() {
        let (seeded, layout, vis) = match *n {
            "display" => ("false", SRO, FS | CS),
            "taa_hist" => ("true", GENERAL, CS),
            _ => ("false", 0, 0),
        };
        t.push_str(&format!(
            "\n[[resource]]\nindex = {i}\nname = \"{n}\"\nkind = \"image\"\nmips = 1\nseeded = {seeded}\nseed_layout = {layout}\nseed_flush_stages = 0\nseed_flush_access = 0\nseed_visible_stages = {vis}\nseed_visible_access = 32\n"
        ));
    }
    for (i, p) in passes.iter().enumerate() {
        t.push_str(&format!("\n[[pass]]\nindex = {i}\nname = \"{p}\"\naccesses = 0\nimg_barriers = 0\nbuf_barriers = 0\n"));
    }
    for (p, r, s, a, l) in accesses {
        t.push_str(&format!("\n[[access]]\npass = {p}\nres = {r}\nstage = {s}\naccess = {a}\nlayout = {l}\nbase_mip = 0\nmip_count = 1\n"));
    }
    Receipt::parse(&t).expect("invariant: the fixture parses")
}

fn good_off() -> Receipt {
    fixture_receipt(
        &["resolve", "post_final", "present_sample"],
        &[(0, 0, CS, 0x40, GENERAL), (1, 0, CS, 0x20, GENERAL), (1, 2, CS, 0x40, GENERAL), (2, 2, FS, 0x20, SRO)],
        true,
    )
}

#[test]
fn the_seam_check_accepts_a_correct_frame() {
    check_post_seam(&good_off(), Aa::Off, false, OBS_WIN).expect("a correct AA-off frame");
    let taa = fixture_receipt(
        &["resolve", "taa_resolve", "post_final", "present_sample"],
        &[(1, 1, CS, 0x40, GENERAL), (2, 1, CS, 0x20, GENERAL), (2, 2, CS, 0x40, GENERAL), (3, 2, FS, 0x20, SRO)],
        true,
    );
    check_post_seam(&taa, Aa::Taa, false, OBS_WIN).expect("a correct TAA frame");
}

#[test]
fn the_seam_check_rejects_each_broken_frame() {
    let red = |r: Receipt, aa: Aa| check_post_seam(&r, aa, false, OBS_WIN).expect_err("a broken seam must red");
    assert!(red(fixture_receipt(&["resolve", "present_sample"], &[], true), Aa::Off).contains("no pass `post_final`"));
    // The TAA frame's post_final reading `lit` (the accumulation bypassed).
    let bypass = fixture_receipt(
        &["resolve", "taa_resolve", "post_final", "present_sample"],
        &[(1, 1, CS, 0x40, GENERAL), (2, 0, CS, 0x20, GENERAL), (2, 2, CS, 0x40, GENERAL), (3, 2, FS, 0x20, SRO)],
        true,
    );
    assert!(red(bypass, Aa::Taa).contains("expected a COMPUTE SHADER_READ of `taa_hist`"));
    // The scene read at SHADER_READ_ONLY_OPTIMAL (a sampled bind), not GENERAL.
    let sro = fixture_receipt(
        &["resolve", "post_final", "present_sample"],
        &[(1, 0, CS, 0x20, SRO), (1, 2, CS, 0x40, GENERAL), (2, 2, FS, 0x20, SRO)],
        true,
    );
    assert!(red(sro, Aa::Off).contains("first access"));
    // display left at GENERAL (no present_sample read).
    let stranded = fixture_receipt(&["resolve", "post_final", "present_sample"], &[(1, 0, CS, 0x20, GENERAL), (1, 2, CS, 0x40, GENERAL)], true);
    assert!(red(stranded, Aa::Off).contains("last access"));
    // No receipt words.
    let wordless = fixture_receipt(
        &["resolve", "post_final", "present_sample"],
        &[(1, 0, CS, 0x20, GENERAL), (1, 2, CS, 0x40, GENERAL), (2, 2, FS, 0x20, SRO)],
        false,
    );
    assert!(red(wordless, Aa::Off).contains("post_word0"));
    // A `cas` pass on a frame that did not arm it.
    let cas = fixture_receipt(
        &["resolve", "post_final", "cas", "present_sample"],
        &[(1, 0, CS, 0x20, GENERAL), (1, 2, CS, 0x40, GENERAL), (2, 2, CS, 0x20, GENERAL), (3, 2, FS, 0x20, SRO)],
        true,
    );
    assert!(red(cas, Aa::Off).contains("`cas` pass"));
    // C11: the test-only NaN injection declared on a production run (no force key armed).
    let injected = fixture_receipt(
        &["resolve", "test_nan_inject", "post_final", "present_sample"],
        &[(2, 0, CS, 0x20, GENERAL), (2, 2, CS, 0x40, GENERAL), (3, 2, FS, 0x20, SRO)],
        true,
    );
    assert!(red(injected, Aa::Off).contains("test_nan_inject"));
}

/// The whole-scene fixtures' native edge.
const FIX_EDGE: u32 = 16;

/// A synthetic `RGBA32F` `lit` of edge `edge` whose radiance is `f(x, y)`.
fn fixture_lit(edge: u32, f: impl Fn(u32, u32) -> f32) -> DumpFile {
    let mut texels = Vec::with_capacity((edge * edge * 16) as usize);
    for y in 0..edge {
        for x in 0..edge {
            let v = f(x, y);
            for c in [v, v, v, 1.0] {
                texels.extend_from_slice(&c.to_le_bytes());
            }
        }
    }
    DumpFile::parse(&encode(VK_R32G32B32A32_SFLOAT, edge, edge, SETTLE, 16, &texels)).expect("invariant: the fixture parses")
}

/// A synthetic `RGBA8` `display` of edge `edge`: a Reinhard tonemap of `radiance(x, y)`.
fn fixture_display(edge: u32, radiance: impl Fn(u32, u32) -> f32) -> DumpFile {
    let mut texels = Vec::with_capacity((edge * edge * 4) as usize);
    for y in 0..edge {
        for x in 0..edge {
            let v = radiance(x, y);
            let code = (255.0 * v / (1.0 + v)).round() as u8;
            texels.extend_from_slice(&[code, code, code, 255]);
        }
    }
    DumpFile::parse(&encode(VK_R8G8B8A8_UNORM, edge, edge, SETTLE, 4, &texels)).expect("invariant: the fixture parses")
}

/// The fixtures' scene at the 2x composite: radiance = the distance to the image centre, so the
/// top-left quarter (a corner falling toward the centre) does not rank like the whole (a bowl).
fn radial(x: u32, y: u32) -> f32 {
    let c = FIX_EDGE as f32;
    (x as f32 + 0.5 - c).hypot(y as f32 + 0.5 - c)
}

/// [`radial`] box-reduced 2:1 to the native edge.
fn radial_native(x: u32, y: u32) -> f32 {
    0.25 * (radial(2 * x, 2 * y) + radial(2 * x + 1, 2 * y) + radial(2 * x, 2 * y + 1) + radial(2 * x + 1, 2 * y + 1))
}

#[test]
fn the_whole_scene_check_accepts_a_display_of_the_whole_scene() {
    let lit = fixture_lit(2 * FIX_EDGE, radial);
    // The composite-sized display the present minifies, and the native one a downsample feeds.
    for display in [fixture_display(2 * FIX_EDGE, radial), fixture_display(FIX_EDGE, radial_native)] {
        let rho = check_presents_the_whole_scene(&lit, &display, FIX_EDGE)
            .unwrap_or_else(|e| panic!("a {}^2 display of the whole scene: {e}", display.width));
        assert!(rho > 0.99, "a {}^2 display of the whole scene ranks at {rho}", display.width);
    }
}

#[test]
fn the_whole_scene_check_rejects_a_quarter_a_vacuous_run_and_a_powerless_scene() {
    let lit = fixture_lit(2 * FIX_EDGE, radial);
    // R2-W1 itself: a native display read 1:1 from the 2x lit — its top-left quarter.
    let quarter = fixture_display(FIX_EDGE, radial);
    let e = check_presents_the_whole_scene(&lit, &quarter, FIX_EDGE).expect_err("a top-left quarter must red");
    assert!(e.contains("does not depict the whole scene"), "{e}");
    // The boot SSAA did not arm: lit is native.
    let native_lit = fixture_lit(FIX_EDGE, radial_native);
    let e = check_presents_the_whole_scene(&native_lit, &fixture_display(FIX_EDGE, radial_native), FIX_EDGE)
        .expect_err("a native lit must red");
    assert!(e.contains("vacuous"), "{e}");
    // A self-similar scene (a ramp): its top-left quarter ranks like the whole, so no power.
    let ramp = |x: u32, y: u32| (x + 3 * y) as f32;
    let e = check_presents_the_whole_scene(&fixture_lit(2 * FIX_EDGE, ramp), &fixture_display(FIX_EDGE, ramp), FIX_EDGE)
        .expect_err("a self-similar scene must red");
    assert!(e.contains("no power"), "{e}");
    // A display that is not a whole multiple of the window.
    let odd = fixture_display(FIX_EDGE + 1, radial);
    let e = check_presents_the_whole_scene(&lit, &odd, FIX_EDGE).expect_err("a misfit display must red");
    assert!(e.contains("not a whole multiple"), "{e}");
}

// ===============================================================================================
// The worker and the drivers
// ===============================================================================================

/// **THE WORKER** — one configuration (`BOYKO_OBS_PATH`, `BOYKO_PX1A_AA`), receipt armed.
#[test]
#[ignore = "gpu-windowed: the PX1a receipt worker, spawned by the px1a_graph_receipt drivers; run with --test-threads=1"]
fn px1a_receipt_worker() {
    let Ok(aa) = std::env::var(ENV_AA) else {
        eprintln!("px1a_receipt_worker: {ENV_AA} is unset -- SKIPPED (spawned by its drivers).");
        return;
    };
    let aa = Aa::parse(&aa);
    let arming = ObsArming::from_env();
    let plugins = EnginePlugins::window("boyko_engine px1a receipt", OBS_WIN, OBS_WIN);
    let mut app = build_obs_app_with(if aa == Aa::Ssaa { plugins.with_ssaa_scale(2) } else { plugins }, arming);
    match aa {
        Aa::Off | Aa::Ssaa => {
            app.insert_resource(AaConfig { mode: AaMode::Off });
        }
        Aa::Taa => {
            app.insert_resource(AaConfig { mode: AaMode::Taa });
        }
        Aa::TaaCas => {
            app.insert_resource(AaConfig { mode: AaMode::Taa });
            app.insert_resource(TaaConfig { sharpen: SharpenMode::Rcas, ..TaaConfig::default() });
        }
        Aa::Fxaa => {
            app.insert_resource(AaConfig { mode: AaMode::Fxaa });
        }
        Aa::Smaa => {
            app.insert_resource(AaConfig { mode: AaMode::Smaa });
        }
    }
    app.run();
    assert_obs_resolved(&app, arming);
}

fn drive(path: RenderPath, aa: Aa) {
    drive_expecting(path, aa, aa);
}

/// [`drive`] with the worker armed for `aa` and the seam checked against `seam` — the two differ
/// where the path degrades the AA mode the host armed (Forward under a boot SSAA). `display` is
/// native only on a frame that runs the SSAA downsample (D-G1: native under SSAA, the composite
/// otherwise): a path that degrades the boot SSAA keeps `display` at the 2x composite, the extent
/// of the `lit` `post_final` reads 1:1 (review r2 R2-W1).
fn drive_expecting(path: RenderPath, aa: Aa, seam: Aa) {
    let display_edge = if aa == Aa::Ssaa && seam != Aa::Ssaa { 2 * OBS_WIN } else { OBS_WIN };
    let tag = format!("px1a_receipt_{}_{}", ObsArming::path_word(path), aa.word());
    let out = fresh_out(&format!("{tag}.toml"));
    let taa = matches!(aa, Aa::Taa | Aa::TaaCas);
    run_worker(
        "px1a_receipt_worker",
        &[
            (ENV_PATH, ObsArming::path_word(path).to_owned()),
            (ENV_TAA, if taa { "1" } else { "0" }.to_owned()),
            (ENV_AA, aa.word().to_owned()),
            ("BOYKO_GRAPH_RECEIPT", out.display().to_string()),
            ("BOYKO_GRAPH_RECEIPT_SETTLE", SETTLE.to_string()),
            ("BOYKO_WINDOW_FRAMES", (SETTLE + 20).to_string()),
        ],
        false,
    );
    let r = Receipt::load(&out);
    let particles = r.config("path_has_particles") == 1;
    if let Err(e) = check_post_seam(&r, seam, particles, display_edge) {
        panic!("{tag}: {e}");
    }
}

#[test]
#[ignore = "gpu-windowed: PX1a test 10, the post seam on Deferred, AA off (spawns px1a_receipt_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_receipt_deferred_off() {
    drive(RenderPath::Deferred, Aa::Off);
}

#[test]
#[ignore = "gpu-windowed: PX1a test 10, the post seam on Deferred, TAA (spawns px1a_receipt_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_receipt_deferred_taa() {
    drive(RenderPath::Deferred, Aa::Taa);
}

#[test]
#[ignore = "gpu-windowed: PX1a test 10, the post seam on Deferred, TAA + CAS (spawns px1a_receipt_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_receipt_deferred_taa_cas() {
    drive(RenderPath::Deferred, Aa::TaaCas);
}

#[test]
#[ignore = "gpu-windowed: PX1a test 10, the post seam on Deferred, FXAA (spawns px1a_receipt_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_receipt_deferred_fxaa() {
    drive(RenderPath::Deferred, Aa::Fxaa);
}

#[test]
#[ignore = "gpu-windowed: PX1a test 10, the post seam on Deferred, SMAA (spawns px1a_receipt_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_receipt_deferred_smaa() {
    drive(RenderPath::Deferred, Aa::Smaa);
}

#[test]
#[ignore = "gpu-windowed: PX1a test 10, the post seam on Deferred, SSAA (spawns px1a_receipt_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_receipt_deferred_ssaa() {
    drive(RenderPath::Deferred, Aa::Ssaa);
}

#[test]
#[ignore = "gpu-windowed: PX1a test 10, the post seam on VisibilityBuffer, AA off (spawns px1a_receipt_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_receipt_vb_off() {
    drive(RenderPath::VisibilityBuffer, Aa::Off);
}

#[test]
#[ignore = "gpu-windowed: PX1a test 10, the post seam on VisibilityBuffer, TAA (spawns px1a_receipt_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_receipt_vb_taa() {
    drive(RenderPath::VisibilityBuffer, Aa::Taa);
}

#[test]
#[ignore = "gpu-windowed: PX1a test 10, the post seam on VisibilityBuffer, TAA + CAS (spawns px1a_receipt_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_receipt_vb_taa_cas() {
    drive(RenderPath::VisibilityBuffer, Aa::TaaCas);
}

#[test]
#[ignore = "gpu-windowed: PX1a test 10, the post seam on VisibilityBuffer, FXAA (spawns px1a_receipt_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_receipt_vb_fxaa() {
    drive(RenderPath::VisibilityBuffer, Aa::Fxaa);
}

#[test]
#[ignore = "gpu-windowed: PX1a test 10, the post seam on VisibilityBuffer, SMAA (spawns px1a_receipt_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_receipt_vb_smaa() {
    drive(RenderPath::VisibilityBuffer, Aa::Smaa);
}

#[test]
#[ignore = "gpu-windowed: PX1a test 10, the post seam on VisibilityBuffer, SSAA (spawns px1a_receipt_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_receipt_vb_ssaa() {
    drive(RenderPath::VisibilityBuffer, Aa::Ssaa);
}

#[test]
#[ignore = "gpu-windowed: PX1a test 10, the post seam on Forward, both legs (spawns px1a_receipt_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_receipt_forward_both() {
    drive(RenderPath::Forward, Aa::Off);
}

#[test]
#[ignore = "gpu-windowed: PX1a test 10, the post seam on ForwardPlus, mesh (spawns px1a_receipt_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_receipt_forwardplus_mesh() {
    drive(RenderPath::ForwardPlus, Aa::Off);
}

/// Review r1 W2, r2 R2-W1: Forward under a boot-armed SSAA. The host fixes the 2x composite at boot
/// (`with_ssaa_scale(2)`: `lit` is `present_extent` = 2x native) while Forward degrades the SSAA
/// pass (`post_process_aa_supported`), so the seam is the AA-off seam: `post_final` reads `lit`
/// 1:1, and `display` must be at that same 2x composite — the receipt's dispatch extent is
/// `2 * OBS_WIN` (at the native extent `display` held the scene's top-left quarter, R2-W1). The
/// worker's debug build also checks the dispatch extent against the UBO's
/// (`record_post_final`'s `debug_assert_eq!`), so a recorder and a host that disagree panic the
/// worker on its first frame. (That check is a debug assertion: it has its power only in a
/// debug-assertions test build, the default `cargo test` profile; the receipt's extent is checked
/// in every build.)
#[test]
#[ignore = "gpu-windowed: PX1a review r1 W2, the post seam on Forward under a boot-armed SSAA (spawns px1a_receipt_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_receipt_forward_boot_ssaa() {
    drive_expecting(RenderPath::Forward, Aa::Ssaa, Aa::Off);
}

/// Review r2 R2-W1: what the window shows on Forward under a boot-armed SSAA. The worker dumps `lit`
/// (the 2x composite) and `display` on one frame, and [`check_presents_the_whole_scene`] requires
/// `display`, reduced to the native window, to depict the whole scene. A `display` at the native
/// extent that `post_final` filled 1:1 from the 2x `lit` holds only the scene's top-left quarter,
/// which the window then shows magnified 2x (rank correlation -0.35 at `6c163cf7`). Particles,
/// interpolation, DDGI and the material edit are off: one static frame.
#[test]
#[ignore = "gpu-windowed: PX1a review r2 R2-W1, Forward under a boot-armed SSAA presents the whole scene (spawns px1a_receipt_worker); run with BOYKO_DISABLE_VALIDATION=1 --test-threads=1"]
fn px1a_forward_boot_ssaa_display_holds_the_whole_scene() {
    let tag = "px1a_forward_boot_ssaa_whole_scene";
    let dir = fresh_out(tag);
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    run_worker(
        "px1a_receipt_worker",
        &[
            (ENV_PATH, ObsArming::path_word(RenderPath::Forward).to_owned()),
            (ENV_TAA, "0".to_owned()),
            (ENV_AA, Aa::Ssaa.word().to_owned()),
            (ENV_PARTICLES, "0".to_owned()),
            (ENV_INTERP, "0".to_owned()),
            (ENV_DDGI, "0".to_owned()),
            (ENV_UPLOAD, "0".to_owned()),
            ("BOYKO_WINDOW_FRAMES", (SETTLE + 20).to_string()),
            ("BOYKO_IMAGE_DUMP", "lit,display".to_owned()),
            ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
            ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
        ],
        false,
    );
    let lits = files_of(&dir, "lit");
    let displays = files_of(&dir, "display");
    let (frame, lit_path) = lits.first().unwrap_or_else(|| panic!("{tag}: no `lit` capture in {}", dir.display()));
    let display_path = displays
        .iter()
        .find(|(f, _)| f == frame)
        .map(|(_, p)| p)
        .unwrap_or_else(|| panic!("{tag}: no `display` capture of frame {frame}: {displays:?}"));
    let (lit, display) = (DumpFile::load(lit_path), DumpFile::load(display_path));
    let rho = check_presents_the_whole_scene(&lit, &display, OBS_WIN).unwrap_or_else(|e| panic!("{tag}: {e}"));
    eprintln!("{tag}: display {}x{}, lit {}x{}, rank correlation with the whole scene {rho:.4}", display.width, display.height, lit.width, lit.height);
}
