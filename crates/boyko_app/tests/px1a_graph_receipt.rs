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
//!   an exposure ratio of exactly 1.0 and the dispatch extent.
//!
//! The non-ignored tests run [`check_post_seam`] over fixtures it must accept and reject, every run.
//! Windowed-test conventions: the drivers are ignored, `--test-threads=1`,
//! `BOYKO_DISABLE_VALIDATION=1`; each driver sets its own frame budget.

#[path = "common/graph_receipt.rs"]
mod graph_receipt;

use boyko_app::EnginePlugins;
use boyko_render::{AaConfig, AaMode, RenderPath, SharpenMode, TaaConfig};
use boyko_rhi_vulkan::ffi::{
    VK_ACCESS_SHADER_READ_BIT, VK_ACCESS_SHADER_WRITE_BIT, VK_IMAGE_LAYOUT_GENERAL,
    VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL, VK_PIPELINE_STAGE_COMPUTE_SHADER_BIT,
    VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT,
};
use boyko_shaderdsl::post::POST_FINAL_RECEIPT_KEYS;

use graph_receipt::{
    ENV_PATH, ENV_TAA, OBS_WIN, ObsArming, Receipt, assert_obs_resolved, build_obs_app_with, fresh_out,
    run_worker,
};

/// The worker's AA knob (`off | taa | taa_cas | fxaa | smaa | ssaa`).
const ENV_AA: &str = "BOYKO_PX1A_AA";
/// Frames before the receipt frame.
const SETTLE: u32 = 30;

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

/// The receipt's post seam against the configuration (module doc).
fn check_post_seam(r: &Receipt, aa: Aa, particles: bool) -> Result<(), String> {
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
    if w != u64::from(OBS_WIN) || h != u64::from(OBS_WIN) {
        return Err(format!("[config] post_word2/3 (extent) = {w}x{h}, not the {OBS_WIN}^2 display"));
    }
    Ok(())
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
    check_post_seam(&good_off(), Aa::Off, false).expect("a correct AA-off frame");
    let taa = fixture_receipt(
        &["resolve", "taa_resolve", "post_final", "present_sample"],
        &[(1, 1, CS, 0x40, GENERAL), (2, 1, CS, 0x20, GENERAL), (2, 2, CS, 0x40, GENERAL), (3, 2, FS, 0x20, SRO)],
        true,
    );
    check_post_seam(&taa, Aa::Taa, false).expect("a correct TAA frame");
}

#[test]
fn the_seam_check_rejects_each_broken_frame() {
    let red = |r: Receipt, aa: Aa| check_post_seam(&r, aa, false).expect_err("a broken seam must red");
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
    if let Err(e) = check_post_seam(&r, aa, particles) {
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
