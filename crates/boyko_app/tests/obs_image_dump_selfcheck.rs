//! **OBS (D) — the image dump's self-check** (render-wave plan H0 Scope 6, device test 3).
//!
//! The drivers spawn [`obs_dump_worker`] (this binary, `--exact`) on the OBS scene with one
//! instrument armed per run, and read what it wrote:
//!
//! - `obs_image_dump_lit_equals_host_dump`: a Deferred frame with AA off and a material edited
//!   every frame (so neighbouring frames differ — a ring slot off by one is visible): the
//!   display-referred image equals the same frame's `BOYKO_HOST_DUMP` BMP texel for texel, channel
//!   order mapped. Two runs, because the two instruments are exclusive per run. Since render wave
//!   PX1a that image is `display` (`lit` is scene-linear RGBA16F), so the dump run captures
//!   `lit,display`: `display` equals the host dump exactly, and `lit` — the ring the resolve wrote —
//!   displays as that same frame through `post_final_eval` within one 8-bit step (the oracle's
//!   transcendental tolerance; a wrong ring slot is a whole frame's material edit away).
//! - `obs_image_dump_taa_hist_burst4`: TAA on, `taa_hist` dumped on 4 consecutive frames — four
//!   files, consecutive frame indices, finite RGBA16F history. The worker is a debug build, so the
//!   (P) seeded-layout invariant ran on every dump frame and stayed silent (the copy at `GENERAL`
//!   leaves the pool in its boundary layout).
//! - `obs_image_dump_seed_widening` (critique W6): both runs edit a material every frame, so
//!   `material_table` is declared on the receipt frame. The receipt of the dump run must carry the
//!   row with `TRANSFER` in its seed; the receipt of a plain run, the production seed. The row's
//!   PRESENCE is asserted first, so neither half can pass by the row being absent.
//! - `obs_image_dump_validation_clean` (critique O7): the dump worker with validation ON (the
//!   parent's `BOYKO_DISABLE_VALIDATION` removed for the child, implicit layers disabled) copies
//!   every Deferred name; the worker reads the validation ledger after teardown and requires it
//!   clean with a live messenger.
//!
//! The non-ignored tests run the decoders over synthetic files of every supported format.
//!
//! Windowed-test conventions: the device tests are ignored by default, `--test-threads=1`,
//! `BOYKO_DISABLE_VALIDATION=1` (except the validation driver's child), `BOYKO_WINDOW_FRAMES` set.

#[path = "common/graph_receipt.rs"]
mod graph_receipt;
#[path = "common/image_dump.rs"]
mod image_dump;

use std::path::Path;

use graph_receipt::{
    ENV_DDGI, ENV_INTERP, ENV_PARTICLES, ENV_PATH, ENV_TAA, ENV_UPLOAD, ObsArming, Receipt, assert_obs_resolved,
    build_obs_app, fresh_out, run_worker,
};
use image_dump::{DumpFile, encode, f16_to_f32, files_of};

/// The marker the drivers set: the worker runs only when spawned by one.
const ENV_WORKER: &str = "BOYKO_OBS_DUMP_WORKER";
/// Set by the validation driver: the worker checks the validation ledger after teardown.
const ENV_LEDGER: &str = "BOYKO_OBS_LEDGER";
/// The settle window both instruments use, so they capture the same presented frame.
const SETTLE: u32 = 30;

/// `VK_PIPELINE_STAGE_TRANSFER_BIT`.
const STAGE_TRANSFER: u64 = 0x1000;
/// `VK_ACCESS_TRANSFER_READ_BIT`.
const ACCESS_TRANSFER_READ: u64 = 0x800;

// ===============================================================================================
// Host fixtures (not ignored)
// ===============================================================================================

#[test]
fn decoders_read_every_format_fixture() {
    // RGBA8 and BGRA8.
    let f = DumpFile::parse(&encode(37, 2, 1, 7, 4, &[1, 2, 3, 4, 250, 251, 252, 253])).expect("rgba8");
    assert_eq!((f.frame, f.rgba8(1, 0)), (7, [250, 251, 252, 253]));
    let f = DumpFile::parse(&encode(44, 1, 1, 0, 4, &[10, 20, 30, 40])).expect("bgra8");
    assert_eq!(f.rgba8(0, 0), [30, 20, 10, 40]);
    // Rows are tightly packed: texel (x, y) is at (y * width + x) * 4 — a pitch error reds row 1.
    let rows: Vec<u8> = (0u8..16).collect();
    let f = DumpFile::parse(&encode(37, 2, 2, 0, 4, &rows)).expect("rgba8 2x2");
    assert_eq!((f.rgba8(1, 0), f.rgba8(0, 1), f.rgba8(1, 1)), ([4, 5, 6, 7], [8, 9, 10, 11], [12, 13, 14, 15]));
    // RGBA16F: 1.0, -2.0, 0.5, 65504, and the smallest subnormal.
    let halves: [u16; 8] = [0x3C00, 0xC000, 0x3800, 0x7BFF, 0x0001, 0x0000, 0x8000, 0x3555];
    let bytes: Vec<u8> = halves.iter().flat_map(|h| h.to_le_bytes()).collect();
    let f = DumpFile::parse(&encode(97, 2, 1, 0, 8, &bytes)).expect("rgba16f");
    assert_eq!(f.texel_f32(0, 0), [1.0, -2.0, 0.5, 65504.0]);
    let t = f.texel_f32(1, 0);
    assert_eq!(t[0], 2.0f32.powi(-24), "the smallest subnormal half");
    assert_eq!((t[1], t[2].to_bits()), (0.0, (-0.0f32).to_bits()));
    assert!((t[3] - 0.333_252).abs() < 1e-6, "0x3555 is 0.333252: {}", t[3]);
    assert!(f16_to_f32(0x7C00).is_infinite() && f16_to_f32(0x7E00).is_nan());
    // R32F, D32, RGBA32F.
    let f = DumpFile::parse(&encode(100, 1, 1, 0, 4, &1.25f32.to_le_bytes())).expect("r32f");
    assert_eq!(f.texel_f32(0, 0), [1.25, 0.0, 0.0, 1.0]);
    let f = DumpFile::parse(&encode(126, 1, 1, 0, 4, &0.070_312_5f32.to_le_bytes())).expect("d32");
    assert_eq!(f.depth(0, 0), 0.070_312_5);
    let v: Vec<u8> = [1.0f32, 2.0, 3.0, 4.0].iter().flat_map(|x| x.to_le_bytes()).collect();
    let f = DumpFile::parse(&encode(109, 1, 1, 0, 16, &v)).expect("rgba32f");
    assert_eq!(f.texel_f32(0, 0), [1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn a_malformed_dump_is_red() {
    assert!(DumpFile::parse(&[0u8; 16]).is_err(), "short");
    assert!(DumpFile::parse(&encode(37, 2, 2, 0, 4, &[0; 12])).is_err(), "truncated texels");
    assert!(DumpFile::parse(&encode(999, 1, 1, 0, 4, &[0; 4])).is_err(), "an unknown format");
    assert!(DumpFile::parse(&encode(97, 1, 1, 0, 4, &[0; 4])).is_err(), "a texel size the format contradicts");
    let mut bad = encode(37, 1, 1, 0, 4, &[0; 4]);
    bad[0] ^= 1;
    assert!(DumpFile::parse(&bad).is_err(), "bad magic");
}

// ===============================================================================================
// The worker
// ===============================================================================================

/// **The OBS dump WORKER** — one OBS-scene boot with whatever instrument its driver armed.
#[test]
#[ignore = "gpu-windowed: the OBS image-dump worker, spawned by the obs_image_dump_* drivers; run with --test-threads=1"]
fn obs_dump_worker() {
    if std::env::var_os(ENV_WORKER).is_none() {
        eprintln!("obs_dump_worker: {ENV_WORKER} is unset -- SKIPPED (spawned by its drivers).");
        return;
    }
    let arming = ObsArming::from_env();
    let mut app = build_obs_app("boyko_app OBS dump worker", arming);
    app.run();
    if std::env::var_os(ENV_LEDGER).is_some() {
        let after = boyko_rhi_vulkan::debug::validation_ledger();
        eprintln!("obs_dump_worker: validation ledger after teardown {after:?}");
        assert!(after.messengers_created == 1, "DEAD ORACLE: no validation messenger in this process ({after:?})");
        assert!(
            after.errors == 0 && after.validation_warnings == 0 && after.general_errors == 0,
            "VALIDATION NOT CLEAN on the image-dump run: {after:?} — the [vk-validation] lines above name each VUID"
        );
    }
    assert_obs_resolved(&app, arming);
}

// ===============================================================================================
// The drivers
// ===============================================================================================

/// The Deferred OBS scene's env, AA as asked, no particles, interpolation or DDGI (each makes
/// two runs differ), a material edited every frame.
fn deferred_env(taa: bool) -> Vec<(&'static str, String)> {
    vec![
        (ENV_WORKER, "1".to_owned()),
        (ENV_PATH, "deferred".to_owned()),
        (ENV_TAA, if taa { "1" } else { "0" }.to_owned()),
        (ENV_DDGI, "0".to_owned()),
        (ENV_INTERP, "0".to_owned()),
        (ENV_PARTICLES, "0".to_owned()),
        (ENV_UPLOAD, "1".to_owned()),
        ("BOYKO_WINDOW_FRAMES", (SETTLE + 60).to_string()),
    ]
}

fn with(mut env: Vec<(&'static str, String)>, more: &[(&'static str, String)]) -> Vec<(&'static str, String)> {
    env.extend_from_slice(more);
    env
}

/// A fresh, empty dump directory.
fn dump_dir(name: &str) -> std::path::PathBuf {
    let dir = fresh_out(name);
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    dir
}

/// The host dump's 32-bpp bottom-up BMP, as `(width, height, rgba at (x, y-from-top))`.
fn read_bmp(path: &Path) -> (u32, u32, Vec<u8>) {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("no host dump at {} ({e})", path.display()));
    assert!(b.len() >= 54 && &b[..2] == b"BM", "{} is not a BMP", path.display());
    let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().expect("invariant: 4 bytes"));
    let (off, w) = (u32_at(10) as usize, u32_at(18));
    let h = i32::from_le_bytes(b[22..26].try_into().expect("invariant: 4 bytes"));
    assert!(h > 0, "the host dump writes bottom-up rows");
    let h = h as u32;
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let o = off + (((h - 1 - y) * w + x) * 4) as usize;
            let d = ((y * w + x) * 4) as usize;
            rgba[d..d + 4].copy_from_slice(&[b[o + 2], b[o + 1], b[o], b[o + 3]]);
        }
    }
    (w, h, rgba)
}

#[test]
#[ignore = "gpu-windowed: OBS device test 3, the lit dump against the same frame's host dump (two obs_dump_worker runs); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn obs_image_dump_lit_equals_host_dump() {
    let dir = dump_dir("dump_lit_eq");
    run_worker(
        "obs_dump_worker",
        &with(deferred_env(false), &[
            ("BOYKO_IMAGE_DUMP", "lit,display".to_owned()),
            ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
            ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
        ]),
        false,
    );
    let bmp = fresh_out("dump_lit_eq_host.bmp");
    run_worker(
        "obs_dump_worker",
        &with(deferred_env(false), &[
            ("BOYKO_HOST_DUMP", bmp.display().to_string()),
            ("BOYKO_HOST_DUMP_SETTLE", SETTLE.to_string()),
        ]),
        false,
    );
    let files = files_of(&dir, "display");
    assert_eq!(files.len(), 1, "one display capture expected, found {files:?}");
    let display = DumpFile::load(&files[0].1);
    let (bw, bh, host) = read_bmp(&bmp);
    assert!(
        bw >= display.width && bh >= display.height,
        "the swapchain {bw}x{bh} is smaller than display {}x{}",
        display.width,
        display.height
    );
    let mut diffs = 0u64;
    let mut first = None;
    for y in 0..display.height {
        for x in 0..display.width {
            let d = ((y * bw + x) * 4) as usize;
            let want = [host[d], host[d + 1], host[d + 2]];
            let got = display.rgba8(x, y);
            if [got[0], got[1], got[2]] != want {
                diffs += 1;
                first.get_or_insert((x, y, got, want));
            }
        }
    }
    eprintln!(
        "display dump frame {} {}x{} vs host dump {bw}x{bh}: {diffs} differing texels",
        display.frame, display.width, display.height
    );
    assert_eq!(
        diffs, 0,
        "the display dump differs from the same frame's presented image in {diffs} texels (first at {first:?} as \
         (x, y, dumped rgba, presented rgb)) — a wrong row pitch or channel order"
    );

    // The `lit` ring slot: the same frame's scene-linear `lit`, displayed through the traced
    // `post_final` body (ACES, exposure ratio 1 — the OBS scene's defaults), is `display`.
    let files = files_of(&dir, "lit");
    assert_eq!(files.len(), 1, "one lit capture expected, found {files:?}");
    let lit = DumpFile::load(&files[0].1);
    assert_eq!((lit.frame, lit.width, lit.height), (display.frame, display.width, display.height), "lit and display: one frame, one extent");
    let mut words = [0u32; 8];
    words[boyko_shaderdsl::post::ubo_word("exposure_ratio")] = 1.0f32.to_bits();
    words[boyko_shaderdsl::post::ubo_word("extent")] = 1;
    words[boyko_shaderdsl::post::ubo_word("extent") + 1] = 1;
    let params = boyko_shaderdsl::post::eval_params(words);
    let mut off = 0u64;
    let mut worst = (0u8, (0, 0));
    for y in 0..lit.height {
        for x in 0..lit.width {
            let texel = [lit.texel_f32(x, y)];
            let o = boyko_shaderdsl::post::post_final_eval([0, 0], &params, boyko_shaderdsl::cf::HostTex2::new(&texel, 1, 1));
            let shown = display.rgba8(x, y);
            for (&v, &s) in o[..3].iter().zip(&shown[..3]) {
                let want = (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                let d = want.abs_diff(s);
                if d > worst.0 {
                    worst = (d, (x, y));
                }
                if d > 1 {
                    off += 1;
                }
            }
        }
    }
    eprintln!("lit through post_final_eval vs display: worst {} LSB at {:?}, {off} channels above 1 LSB", worst.0, worst.1);
    assert_eq!(off, 0, "the lit dump is not the frame display shows (worst {} LSB at {:?}) — a wrong ring slot", worst.0, worst.1);
}

#[test]
#[ignore = "gpu-windowed: OBS device test 3, a 4-frame taa_hist burst under TAA (obs_dump_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn obs_image_dump_taa_hist_burst4() {
    let dir = dump_dir("dump_taa_burst4");
    run_worker(
        "obs_dump_worker",
        &with(deferred_env(true), &[
            ("BOYKO_IMAGE_DUMP", "taa_hist".to_owned()),
            ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
            ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
            ("BOYKO_IMAGE_DUMP_FRAMES", "4".to_owned()),
        ]),
        false,
    );
    let files = files_of(&dir, "taa_hist");
    let frames: Vec<u32> = files.iter().map(|f| f.0).collect();
    assert_eq!(files.len(), 4, "a 4-frame burst writes 4 taa_hist files: {frames:?}");
    assert!(frames.windows(2).all(|w| w[1] == w[0] + 1), "the burst's frames are consecutive: {frames:?}");
    for (frame, path) in &files {
        let d = DumpFile::load(path);
        assert_eq!((d.format, d.frame), (image_dump::VK_R16G16B16A16_SFLOAT, *frame), "{}", path.display());
        let (mut lit, mut bad) = (0u64, 0u64);
        for y in 0..d.height {
            for x in 0..d.width {
                let t = d.texel_f32(x, y);
                if t.iter().any(|c| !c.is_finite()) {
                    bad += 1;
                } else if t[0] + t[1] + t[2] > 0.0 {
                    lit += 1;
                }
            }
        }
        eprintln!("taa_hist frame {frame}: {lit} non-black texels, {bad} non-finite");
        assert_eq!(bad, 0, "frame {frame}: the history holds non-finite texels");
        assert!(lit > 0, "frame {frame}: the history is black — the copy read nothing the resolve wrote");
    }
}

/// The `material_table` row of a receipt — its presence asserted first (critique W6).
fn material_table_seed(r: &Receipt, run: &str) -> (u64, u64) {
    let rows: Vec<_> = r.resources().into_iter().filter(|x| x.name == "material_table").collect();
    assert_eq!(
        rows.len(),
        1,
        "PREMISE ({run}): the receipt frame must declare material_table (the worker edits a material \
         every frame) — found {} rows",
        rows.len()
    );
    (rows[0].seed_visible_stages, rows[0].seed_visible_access)
}

#[test]
#[ignore = "gpu-windowed: OBS device test 3, the dump-run seed widening against a production receipt (two obs_dump_worker runs); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn obs_image_dump_seed_widening() {
    let dir = dump_dir("dump_seed_widening");
    let dumped = fresh_out("dump_seed_widening_dump.toml");
    run_worker(
        "obs_dump_worker",
        &with(deferred_env(false), &[
            ("BOYKO_IMAGE_DUMP", "lit".to_owned()),
            ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
            ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
            ("BOYKO_GRAPH_RECEIPT", dumped.display().to_string()),
            ("BOYKO_GRAPH_RECEIPT_SETTLE", SETTLE.to_string()),
        ]),
        false,
    );
    let plain = fresh_out("dump_seed_widening_plain.toml");
    run_worker(
        "obs_dump_worker",
        &with(deferred_env(false), &[
            ("BOYKO_GRAPH_RECEIPT", plain.display().to_string()),
            ("BOYKO_GRAPH_RECEIPT_SETTLE", SETTLE.to_string()),
        ]),
        false,
    );
    let d = Receipt::load(&dumped);
    let p = Receipt::load(&plain);
    assert!(
        d.pass_names().iter().any(|n| n == "image_dump"),
        "PREMISE: the dump run's receipt frame is a dump frame (it declares image_dump): {:?}",
        d.pass_names()
    );
    let (d_stages, d_access) = material_table_seed(&d, "dump run");
    let (p_stages, p_access) = material_table_seed(&p, "plain run");
    eprintln!("material_table seed: dump run stages {d_stages:#x} access {d_access:#x}; plain run {p_stages:#x} / {p_access:#x}");
    assert!(
        d_stages & STAGE_TRANSFER != 0 && d_access & ACCESS_TRANSFER_READ != 0,
        "the dump run's material_table seed lacks the dump's TRANSFER read (stages {d_stages:#x}, access \
         {d_access:#x}) — frame N+1's first write would not wait for frame N's dump copy"
    );
    assert!(
        p_stages & STAGE_TRANSFER == 0 && p_access & ACCESS_TRANSFER_READ == 0,
        "the plain run's material_table seed carries TRANSFER (stages {p_stages:#x}, access {p_access:#x}) — \
         the widening leaked into a run without the instrument"
    );
    assert_eq!(
        (d_stages & !STAGE_TRANSFER, d_access & !ACCESS_TRANSFER_READ),
        (p_stages, p_access),
        "the widening must ADD the dump's read to the production seed, nothing else"
    );
}

#[test]
#[ignore = "gpu-windowed: OBS device test 3's validation-on leg, the dump worker under the validation layer (obs_dump_worker); run with --test-threads=1"]
fn obs_image_dump_validation_clean() {
    let dir = dump_dir("dump_validation");
    run_worker(
        "obs_dump_worker",
        &with(deferred_env(true), &[
            ("BOYKO_IMAGE_DUMP", "depth,viewt,lit,taa_hist".to_owned()),
            ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
            ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
            ("BOYKO_IMAGE_DUMP_FRAMES", "2".to_owned()),
            (ENV_LEDGER, "1".to_owned()),
            ("VK_LOADER_LAYERS_DISABLE", "~implicit~".to_owned()),
        ]),
        true,
    );
    for name in ["depth", "viewt", "lit", "taa_hist"] {
        let files = files_of(&dir, name);
        assert_eq!(files.len(), 2, "the validated run copied `{name}` on both burst frames: {files:?}");
    }
}
