//! **RD6 / D-DS device gates — glTF `doubleSided` on the opaque rail** (render wave RD6, defect
//! D-DS, commit D-4; the cut's §3 D-4 list plus critique W5's per-family arms).
//!
//! glTF 2.0 "Double Sided": "When this value is false, back-face culling is enabled, i.e., only
//! front-facing triangles are rendered. When this value is true, back-face culling is disabled,
//! triangles are rendered from both sides, and the normal vectors of the back-facing triangles
//! are negated".
//!
//! Every gate boots the `common/dds_scene.rs` worker once per arm, dumps `lit` and the path's depth
//! image (`depth` / `forward_depth` / `vb_depth`) with `BOYKO_IMAGE_DUMP`, and compares arms pixel
//! for pixel over a host-computed footprint:
//!
//! 1. `dds_back_face_lit_*` — a double-sided card seen from its authored BACK is lit like its
//!    front over the 2-px-eroded footprint (`A ≥ 32/255`), with the sun along the view direction
//!    and ambient + emission zeroed (rule xvi). Every channel of every pixel matches in one of two
//!    arms ([`front_back_match`]): the scene-linear `lit` radiances within the producer's own
//!    front/back difference plus one binary16 store, or `post_final`'s 8-bit displays of the two
//!    within one code — the pre-PX1a gate, whose `lit` was that display. Red before the fix: the
//!    back shades with its front normal, `N·L < 0`, dark.
//! 2. `dds_single_sided_back_culled_*` — a single-sided card seen from its back, a backdrop 2 units
//!    behind: the depth over the card's footprint equals the no-card run's (the card is culled).
//! 3. `dds_double_sided_caster_*` / `dds_double_sided_point_caster_*` — a horizontal double-sided
//!    card with its authored front AWAY from the light still casts on the floor (the lit ratio in
//!    the host-predicted umbra, over pixels where the floor is visible in both runs, ≤ 0.5); the
//!    single-sided controls keep HEAD's light-facing caster set (K1): toward the light casts,
//!    away does not — except the sun on a `hwrt` Deferred boot, whose traced shadow rays are
//!    facing-agnostic (DS-D8), so there the away card casts too.
//! 4. `dds_mirrored_cube_*` — the built-in cube at scale `(−1, 1, 1)` renders the pixels of the
//!    unmirrored one (its X mirror is the same geometric cube). Green before the fix (culling is
//!    off); the regression guard of a mirrored instance turned inside-out.
//! 5. `dds_mirrored_card_lit_*` — test 1's back arm on an X-mirrored instance (an IN-PLANE axis, so
//!    the card's normal is unchanged and only its winding reverses).
//! 6. `dds_forwardplus_prepass_parity` — ForwardPlus's depth prepass and its EQUAL colour pass cull
//!    the same faces: the image over a culled card equals the no-card run's.
//!
//! W5 (one failing gate per pipeline family): the TEX family (`dds_back_face_lit_deferred_textured`),
//! the ForwardPlus `Back` member (`dds_single_sided_back_culled_forwardplus`), the point-light
//! caster family (`dds_double_sided_point_caster_*`), and under `hwrt` the temporal denoiser's two
//! camera families. A frame with a non-default material takes MVPM, so
//! `dds_back_face_lit_deferred_denoised` (its double-sided card) gates MVPM only; the MV family
//! (an all-default frame) is gated by `dds_single_sided_back_culled_deferred_denoised` (its `Back`
//! member) and `dds_mirrored_cube_deferred_denoised` (its `Front` member) — triage r1 T-MV.
//!
//! Windowed-test conventions: ignored by default, `--test-threads=1`, `BOYKO_DISABLE_VALIDATION=1`,
//! `BOYKO_WINDOW_FRAMES` set (each driver also sets a per-worker backstop).

#[path = "common/dds_scene.rs"]
mod dds_scene;
#[path = "common/graph_receipt.rs"]
mod graph_receipt;
#[path = "common/image_dump.rs"]
mod image_dump;

use boyko_app::post_host::post_final_gpu;
use boyko_render::post::ResolvedPost;
use boyko_render::{LightingConfig, RenderPath};
use boyko_shaderdsl::cf::{EvalCf, HostTex2};
use boyko_shaderdsl::post::{PostParams, eval_params, post_final_eval, ubo_word};

use dds_scene::{
    DdsArming, ENV_BACKDROP, ENV_DS, ENV_EMISSIVE, ENV_FACING, ENV_LIGHT, ENV_MIRROR, ENV_NOCARD,
    ENV_PATH, ENV_SCENE, ENV_TEX, EMPTY_PIXEL, Rect, WIN, assert_dds_resolved, backdrop_band, build_dds_app,
    card_mask, shadow_rect,
};
use graph_receipt::{fresh_out, run_worker};
use image_dump::{DumpFile, VK_D32_SFLOAT, files_of};

/// The marker the drivers set.
const ENV_WORKER: &str = "BOYKO_DDS_WORKER";
/// The capture's settle window.
const SETTLE: u32 = 12;
/// `1/255` of scene-linear `lit` radiance.
///
/// Since render wave PX1a (`7a4204b1`) `lit` is RGBA16F radiance; before it `lit` held the
/// display-referred `OETF(ACES(radiance))`, and this constant was one display code. The premise
/// floors (`32·LSB`, `8·LSB`) and the equality gates of tests 4 and 6 read it in radiance. Those
/// gates compare two runs of the same surface under the same light. There `LSB` is 8 binary16
/// ULPs in `[0.5, 1)`: looser than one store, tighter than one display code. Tests 1 and 5 gate
/// through [`front_back_match`] instead.
const LSB: f32 = 1.0 / 255.0;

/// The display arm of [`front_back_match`]: `post_final`'s 8-bit displays of the front and back
/// texels within this many codes per channel. It is the pre-PX1a gate `|A − B| ≤ 1/255`, kept in
/// the unit it was set in (ruling 81 (a), the K12 suites' `lit_matches` form).
const DISPLAY_CODES: u8 = 1;

/// The gNormal octahedral codes (RG8 UNORM) of the textured card's front and back.
///
/// The fixture's normal texel `(128, 128, 255)` (`dds_scene.rs`'s `card_material`) is not flat:
/// 8-bit UNORM cannot store 127.5, so it decodes to `n_ts = normalize(u, u, 1)` with
/// `u = 2·128/255 − 1 = +1/255`. The back arm's 180° turn about Y keeps the bitangent, and the
/// glTF flip then negates the whole shading normal. The bitangent tilt is therefore mirrored: the
/// front shades `≈ (u, −u, 1)` and the back `≈ (u, u, 1)` — a bumped sheet seen from behind. Their
/// octahedral coordinates are `(127.996, 127.004)` and `(127.996, 127.996)` codes. Both lie in
/// `[127, 128]`, so either rounding Vulkan's float→UNORM conversion allows leaves them at most one
/// code apart along world Y.
const TEX_FRONT_OCT: (u8, u8) = (128, 127);
/// See [`TEX_FRONT_OCT`].
const TEX_BACK_OCT: (u8, u8) = (128, 128);

/// The sup of `|∂lobe/∂n_y|` for each `deferred_pbr.hlsl` directional-resolve lobe that reads the
/// textured card's tilt: diffuse `N·L`, the GGX direct lobe (`pbr_lighting.hlsli`) and the sun-disc
/// env lobe along `reflect(−v, n)` (exponent `2/α² − 2`).
///
/// The sun `SUN_VIEW = (0.1, 0.25, 1)` has `l_y = 0.2414`. Each sup is taken over the card mask's
/// 4489 view rays and over the segment between the two decoded normals, at the fixture's constants:
/// albedo 0.8, metallic 0, roughness fp16(0.6) through gPbr, reflectance 0.5 (f0 0.04),
/// illuminance 2.8, exposure 1. Evaluated on the unrounded producer formulas (PX1a merge triage
/// R1, `px1a/merge/triage_reds.md` §1.2): derived from the producer's math, not from a measured
/// delta (ruling 81 (c)). Their sum bounds the total gradient by the triangle inequality; the
/// joint sup is 0.71121, so the bound is nearly tight.
const TEX_LOBE_SUP: [f32; 3] = [0.174_84, 0.183_03, 0.356_36];

/// **The D-DS WORKER** — the `common/dds_scene.rs` scene the driver's env names.
#[test]
#[ignore = "gpu-windowed: the RD6 D-DS worker, spawned by the dds_* drivers; run with --test-threads=1"]
fn dds_worker() {
    if std::env::var_os(ENV_WORKER).is_none() {
        eprintln!("dds_worker: {ENV_WORKER} is unset -- SKIPPED (spawned by its drivers).");
        return;
    }
    let arming = DdsArming::from_env();
    let mut app = build_dds_app("boyko_app RD6 D-DS worker", arming);
    app.run();
    assert_dds_resolved(&app, arming);
}

/// One arm's captures.
struct Capture {
    lit: DumpFile,
    depth: DumpFile,
}

/// Boots the worker with `env` on `path` and returns its `lit` and depth captures.
fn capture(path: RenderPath, env: &[(&str, &str)], tag: &str) -> Capture {
    let depth_name = DdsArming::depth_name(path);
    let dir = fresh_out(&format!("dds_{}_{tag}", DdsArming::path_word(path)));
    std::fs::create_dir_all(&dir).expect("invariant: the temp dir accepts a subdirectory");
    let mut all: Vec<(&str, String)> = vec![
        (ENV_WORKER, "1".to_owned()),
        (ENV_PATH, DdsArming::path_word(path).to_owned()),
        ("BOYKO_IMAGE_DUMP", format!("lit,{depth_name}")),
        ("BOYKO_IMAGE_DUMP_PATH", dir.display().to_string()),
        ("BOYKO_IMAGE_DUMP_SETTLE", SETTLE.to_string()),
        ("BOYKO_WINDOW_FRAMES", (SETTLE + 30).to_string()),
    ];
    all.extend(env.iter().map(|&(k, v)| (k, v.to_owned())));
    run_worker("dds_worker", &all, false);
    let load = |name: &str| {
        let files = files_of(&dir, name);
        assert_eq!(files.len(), 1, "one `{name}` capture expected ({tag}): {files:?}");
        let d = DumpFile::load(&files[0].1);
        assert_eq!((d.width, d.height), (WIN, WIN), "the `{name}` dump's extent ({tag})");
        d
    };
    let depth = load(depth_name);
    assert_eq!(depth.format, VK_D32_SFLOAT, "the depth dump is D32 ({tag})");
    Capture { lit: load("lit"), depth }
}

fn rgb(c: &Capture, x: u32, y: u32) -> [f32; 3] {
    let t = c.lit.texel_f32(x, y);
    [t[0], t[1], t[2]]
}

fn mean_rgb(c: &Capture, r: Rect) -> [f32; 3] {
    let mut s = [0.0f64; 3];
    let mut n = 0.0f64;
    for (x, y) in r.pixels() {
        let p = rgb(c, x, y);
        for k in 0..3 {
            s[k] += f64::from(p[k]);
        }
        n += 1.0;
    }
    s.map(|v| (v / n) as f32)
}

/// The largest per-channel difference between two captures over `r`, with its pixel.
fn max_diff(a: &Capture, b: &Capture, r: Rect) -> (f32, (u32, u32)) {
    let mut worst = (0.0f32, (r.x0, r.y0));
    for (x, y) in r.pixels() {
        let (pa, pb) = (rgb(a, x, y), rgb(b, x, y));
        for k in 0..3 {
            let d = (pa[k] - pb[k]).abs();
            if d > worst.0 {
                worst = (d, (x, y));
            }
        }
    }
    worst
}

/// `base` followed by `extra` (an arm's own env, then the variant's).
fn with_extra<'a>(extra: &[(&'a str, &'a str)], base: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    base.iter().chain(extra).copied().collect()
}

// --- 1 / 5: a double-sided back face is lit like its front --------------------------------------

/// `deferred_pbr.hlsl`'s `oct_decode` of the gNormal codes `(cx, cy)` (UNORM: `code / 255`).
fn oct_decode((cx, cy): (u8, u8)) -> [f32; 3] {
    let e = [f32::from(cx) / 255.0 * 2.0 - 1.0, f32::from(cy) / 255.0 * 2.0 - 1.0];
    let mut n = [e[0], e[1], 1.0 - e[0].abs() - e[1].abs()];
    let t = (-n[2]).clamp(0.0, 1.0);
    n[0] += if n[0] >= 0.0 { -t } else { t };
    n[1] += if n[1] >= 0.0 { -t } else { t };
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    n.map(|c| c / len)
}

/// The radiance by which the textured arm's front and back legitimately differ before the `lit`
/// stores: `|Δn| · Σ TEX_LOBE_SUP`, with `|Δn|` the distance between the decoded
/// [`TEX_FRONT_OCT`] and [`TEX_BACK_OCT`] normals (0.007905). That is 0.005646. With the one
/// binary16 ULP [`front_back_match`] adds, the bound is 0.006134 at lit `[0.5, 1)` (1.56 × 1/255).
fn tex_front_back_asymmetry() -> f32 {
    let (f, b) = (oct_decode(TEX_FRONT_OCT), oct_decode(TEX_BACK_OCT));
    let dn = ((f[0] - b[0]).powi(2) + (f[1] - b[1]).powi(2) + (f[2] - b[2]).powi(2)).sqrt();
    dn * TEX_LOBE_SUP.iter().sum::<f32>()
}

/// One binary16 step at the magnitude of `x`, the ULP of the RGBA16F `lit` store: `2^(e − 10)` in
/// the binade `[2^e, 2^(e+1))`, the subnormal step `2^−24` below `2^−14`.
fn f16_ulp(x: f32) -> f32 {
    let a = x.abs();
    let e = if a < f32::from_bits(0x3880_0000) { -14 } else { (((a.to_bits() >> 23) & 0xFF) as i32 - 127).min(15) };
    f32::from_bits(((e - 10 + 127) as u32) << 23)
}

/// The `post_final` parameters of a D-DS frame. The scene's `LightingConfig` differs from the
/// default only in its sky terms (`dds_scene::build_dds_app_sized`), which `post_final` does not
/// read. The default's tonemapper (Khronos PBR Neutral since render wave PXt) and
/// `post_final_gpu`'s unit exposure ratio are therefore the frame's.
fn display_params() -> PostParams<EvalCf> {
    let gpu = post_final_gpu(&ResolvedPost::from_config(&LightingConfig::default()), [1, 1]);
    let mut words = [0u32; 8];
    words[ubo_word("mode_words")] = gpu.mode_words;
    words[ubo_word("exposure_ratio")] = gpu.exposure_ratio.to_bits();
    words[ubo_word("extent")] = 1;
    words[ubo_word("extent") + 1] = 1;
    eval_params(words)
}

/// `post_final`'s display of one `lit` texel ([`post_final_eval`], the generated body), quantised
/// like the RGBA8 UNORM `display` store.
fn display8(params: &PostParams<EvalCf>, c: [f32; 3]) -> [u8; 3] {
    let texel = [[c[0], c[1], c[2], 1.0]];
    let o = post_final_eval([0, 0], params, HostTex2::new(&texel, 1, 1));
    [o[0], o[1], o[2]].map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
}

/// How a front and a back capture compare in [`front_back_match`]'s two arms.
struct FrontBack {
    /// Pixels with a channel outside both arms, or a non-finite texel.
    mismatched: usize,
    /// The largest per-channel `|A − B|`, its pixel, and the radiance arm's bound there.
    radiance: (f32, (u32, u32), f32),
    /// The largest per-channel display-code difference and its pixel.
    display: (u8, (u32, u32)),
}

/// Tests 1 and 5's comparison of the front `a` and the back `b` over `r`. A channel matches in
/// either arm (ruling 81 (a)):
///
/// - **radiance**: `|A − B| ≤ asymmetry + f16_ulp(max(A, B))`. `asymmetry` is what the producer's
///   front and back legitimately differ by before the stores (0 when both sides shade the same
///   normal; [`tex_front_back_asymmetry`] for the textured card). The ULP covers the two
///   truncating binary16 stores (`goldens::RGBA16F_ROUNDING`'s store class `Z`): each errs by
///   less than one ULP of its own texel, in the same direction.
/// - **display**: `post_final`'s 8-bit displays of the two within [`DISPLAY_CODES`].
///
/// A non-finite texel matches in neither arm.
fn front_back_match(a: &Capture, b: &Capture, r: Rect, asymmetry: f32) -> FrontBack {
    let params = display_params();
    // `NEG_INFINITY`: the first texel records its bound even when every difference is 0.
    let mut out =
        FrontBack { mismatched: 0, radiance: (f32::NEG_INFINITY, (r.x0, r.y0), 0.0), display: (0, (r.x0, r.y0)) };
    for (x, y) in r.pixels() {
        let (pa, pb) = (rgb(a, x, y), rgb(b, x, y));
        if pa.iter().chain(&pb).any(|c| !c.is_finite()) {
            out.mismatched += 1;
            continue;
        }
        let (da, db) = (display8(&params, pa), display8(&params, pb));
        let mut outside = false;
        for k in 0..3 {
            let d = (pa[k] - pb[k]).abs();
            let bound = asymmetry + f16_ulp(pa[k].max(pb[k]));
            let codes = da[k].abs_diff(db[k]);
            if d > out.radiance.0 {
                out.radiance = (d, (x, y), bound);
            }
            if codes > out.display.0 {
                out.display = (codes, (x, y));
            }
            outside |= d > bound && codes > DISPLAY_CODES;
        }
        out.mismatched += usize::from(outside);
    }
    out
}

/// `A` = the double-sided card's front toward the camera; `B` = its back (optionally mirrored).
/// `asymmetry` is the producer's legitimate front/back radiance difference ([`front_back_match`]).
fn back_face_lit(path: RenderPath, extra: &[(&str, &str)], mirrored_b: bool, asymmetry: f32) {
    let mask = card_mask();
    let base: Vec<(&str, &str)> = [(ENV_SCENE, "card"), (ENV_DS, "1")].into_iter().chain(extra.iter().copied()).collect();
    let a = capture(path, &base, "front");
    let mut b_env = base.clone();
    b_env.push((ENV_FACING, "back"));
    if mirrored_b {
        b_env.push((ENV_MIRROR, "1"));
    }
    let b = capture(path, &b_env, if mirrored_b { "back_mirrored" } else { "back" });

    // The "different red": a double-sided back face must be RASTERISED (its depth written).
    let clear = b.depth.depth(EMPTY_PIXEL.0, EMPTY_PIXEL.1);
    let undrawn = mask.pixels().filter(|&(x, y)| b.depth.depth(x, y).to_bits() == clear.to_bits()).count();
    assert_eq!(undrawn, 0, "DIFFERENT RED on {path:?}: {undrawn} px of the double-sided card's back are not drawn — a double-sided back face is culled (the cull table is wrong)");

    let (ma, mb) = (mean_rgb(&a, mask), mean_rgb(&b, mask));
    eprintln!("{path:?}: front mean {ma:?}, back{} mean {mb:?}", if mirrored_b { " (mirrored)" } else { "" });
    assert!(ma.iter().all(|&c| c >= 32.0 * LSB), "PREMISE on {path:?}: the lit front is not lit ({ma:?})");
    let m = front_back_match(&a, &b, mask, asymmetry);
    let ((d, at, bound), (codes, codes_at)) = (m.radiance, m.display);
    eprintln!(
        "{path:?}: max |A-B| {d} at {at:?} (radiance bound there {bound}), max display delta {codes} code(s) at {codes_at:?}"
    );
    assert_eq!(
        m.mismatched,
        0,
        "RED on {path:?}: the double-sided card's back{} is not lit like its front — {} of {} px outside both arms: \
         max |A-B| {d} at {at:?} against the radiance bound {bound} there, max display delta {codes} code(s) at \
         {codes_at:?} against {DISPLAY_CODES} (front mean {ma:?}, back mean {mb:?}: the back shades with its \
         un-negated normal)",
        if mirrored_b { " on a mirrored instance" } else { "" },
        m.mismatched,
        mask.pixels().count()
    );
}

#[test]
#[ignore = "gpu-windowed: RD6 test 1 on Deferred (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_back_face_lit_deferred() {
    back_face_lit(RenderPath::Deferred, &[], false, 0.0);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 1 on Forward (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_back_face_lit_forward() {
    back_face_lit(RenderPath::Forward, &[], false, 0.0);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 1 on ForwardPlus (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_back_face_lit_forwardplus() {
    back_face_lit(RenderPath::ForwardPlus, &[], false, 0.0);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 1 on VisibilityBuffer (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_back_face_lit_vb() {
    back_face_lit(RenderPath::VisibilityBuffer, &[], false, 0.0);
}

/// W5: the TEX family (a textured double-sided material: an albedo and the fixture's near-flat
/// normal map, see [`TEX_FRONT_OCT`]). Its front and back shade normals one octahedral code apart,
/// so the radiance arm allows [`tex_front_back_asymmetry`]. The arm gates the family's routing (its
/// face bit). It does not gate the order of the TBN block and the final-normal flip: a back flipped
/// before the TBN block mirrors the tangent tilt instead and is also one code from the front (PX1a
/// merge triage R1, host model).
#[test]
#[ignore = "gpu-windowed: RD6 test 1 on Deferred, textured (the gbuffer_mrt_tex family; spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_back_face_lit_deferred_textured() {
    back_face_lit(RenderPath::Deferred, &[(ENV_TEX, "1")], false, tex_front_back_asymmetry());
}

/// W5: the MV / MVPM families — the temporal shadow denoiser routes the Deferred camera raster
/// through the motion-vector pipelines (`hwrt` builds on a ray-query device).
#[cfg(feature = "hwrt")]
#[test]
#[ignore = "feature+gpu-windowed+gpu-cap: RD6 test 1 on Deferred under the temporal denoiser (the MV/MVPM families; needs ray query; spawns dds_worker); run with --features hwrt, --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_back_face_lit_deferred_denoised() {
    back_face_lit(RenderPath::Deferred, &[(dds_scene::ENV_DENOISE, "1")], false, 0.0);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 5 on Deferred (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_mirrored_card_lit_deferred() {
    back_face_lit(RenderPath::Deferred, &[], true, 0.0);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 5 on Forward (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_mirrored_card_lit_forward() {
    back_face_lit(RenderPath::Forward, &[], true, 0.0);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 5 on VisibilityBuffer (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_mirrored_card_lit_vb() {
    back_face_lit(RenderPath::VisibilityBuffer, &[], true, 0.0);
}

// --- 2 / 6: a single-sided back face is culled ---------------------------------------------------

/// The single-sided card's back toward the camera with a backdrop, against the no-card run
/// (`extra` arms both runs).
fn single_sided_back_culled(path: RenderPath, extra: &[(&str, &str)]) {
    let mask = card_mask();
    let band = backdrop_band();
    let with = capture(path, &with_extra(extra, &[(ENV_SCENE, "card"), (ENV_FACING, "back"), (ENV_BACKDROP, "1")]), "card");
    let without = capture(path, &with_extra(extra, &[(ENV_SCENE, "card"), (ENV_BACKDROP, "1"), (ENV_NOCARD, "1")]), "nocard");

    let clear = with.depth.depth(EMPTY_PIXEL.0, EMPTY_PIXEL.1);
    let backdrop_missing = band.pixels().filter(|&(x, y)| with.depth.depth(x, y).to_bits() == clear.to_bits()).count();
    assert_eq!(backdrop_missing, 0, "DIFFERENT RED on {path:?}: {backdrop_missing} backdrop px are not drawn — FRONT faces are culled (the cull sign is wrong)");

    let differing = mask.pixels().filter(|&(x, y)| with.depth.depth(x, y).to_bits() != without.depth.depth(x, y).to_bits()).count();
    let total = mask.pixels().count();
    assert_eq!(
        differing, 0,
        "RED on {path:?}: {differing} of {total} px over the single-sided card's footprint hold the CARD's depth — \
         its back face was rasterised instead of culled"
    );
}

#[test]
#[ignore = "gpu-windowed: RD6 test 2 on Deferred (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_single_sided_back_culled_deferred() {
    single_sided_back_culled(RenderPath::Deferred, &[]);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 2 on Forward (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_single_sided_back_culled_forward() {
    single_sided_back_culled(RenderPath::Forward, &[]);
}

/// W5: the ForwardPlus `Back` member (the depth prepass) has its own gate.
#[test]
#[ignore = "gpu-windowed: RD6 test 2 on ForwardPlus (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_single_sided_back_culled_forwardplus() {
    single_sided_back_culled(RenderPath::ForwardPlus, &[]);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 2 on VisibilityBuffer (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_single_sided_back_culled_vb() {
    single_sided_back_culled(RenderPath::VisibilityBuffer, &[]);
}

/// Triage r1 T-MV: the MV family's `Back` member. Every material of this scene is the pinned
/// default, so under the temporal denoiser the Deferred camera raster takes the MV family
/// (`raster_pipeline_mvpm` is armed only when some instance carries a non-default material, and
/// the recorder prefers it), which `dds_back_face_lit_deferred_denoised`'s double-sided card never
/// reaches. Measured red on `raster_mv: None` (the loop falls back to the unculled `None`
/// pipeline) and on the MV family built with `cull_mode: None` for every member.
#[cfg(feature = "hwrt")]
#[test]
#[ignore = "feature+gpu-windowed+gpu-cap: RD6 test 2 on Deferred under the temporal denoiser (the MV family's Back member; needs ray query; spawns dds_worker); run with --features hwrt, --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_single_sided_back_culled_deferred_denoised() {
    single_sided_back_culled(RenderPath::Deferred, &[(dds_scene::ENV_DENOISE, "1")]);
}

/// Test 6: ForwardPlus prepass parity — the image over a culled single-sided card (an EMISSIVE
/// one, so a drawn card is never confused with an undrawn pixel) equals the no-card run's: the
/// prepass and the EQUAL colour pass culled the same face. A prepass that keeps the face while the
/// colour pass culls it leaves undrawn pixels there.
#[test]
#[ignore = "gpu-windowed: RD6 test 6, ForwardPlus prepass parity (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_forwardplus_prepass_parity() {
    let path = RenderPath::ForwardPlus;
    let mask = card_mask();
    let with = capture(path, &[(ENV_SCENE, "card"), (ENV_FACING, "back"), (ENV_BACKDROP, "1"), (ENV_EMISSIVE, "1")], "card");
    let without = capture(path, &[(ENV_SCENE, "card"), (ENV_BACKDROP, "1"), (ENV_NOCARD, "1")], "nocard");
    let backdrop = mean_rgb(&without, mask);
    assert!(backdrop.iter().any(|&c| c >= 8.0 * LSB), "PREMISE: the backdrop behind the card is lit ({backdrop:?})");
    let (d, at) = max_diff(&with, &without, mask);
    assert!(
        d <= LSB,
        "RED: over the single-sided card's footprint ForwardPlus does not show the backdrop — max |with - without| \
         {d} at {at:?} (the card was drawn, or the prepass and the colour pass culled different faces)"
    );
}

// --- 3: casters cull by the flag -----------------------------------------------------------------

/// The lit ratio over the predicted umbra, restricted to pixels where the floor shows in both runs.
fn shadow_ratio(run: &Capture, reference: &Capture, rect: Rect) -> (f64, usize) {
    let luma = |p: [f32; 3]| f64::from(0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]);
    let (mut num, mut den, mut n) = (0.0f64, 0.0f64, 0usize);
    for (x, y) in rect.pixels() {
        if run.depth.depth(x, y).to_bits() != reference.depth.depth(x, y).to_bits() {
            continue; // the card itself covers this pixel
        }
        num += luma(rgb(run, x, y));
        den += luma(rgb(reference, x, y));
        n += 1;
    }
    (if den > 0.0 { num / den } else { f64::NAN }, n)
}

/// The double-sided card faces AWAY from the light and must cast; the single-sided controls keep
/// HEAD's caster set (K1).
fn double_sided_caster(path: RenderPath, light: &str) {
    let point = light == "point";
    let rect = shadow_rect(point);
    let caster = |extra: &[(&str, &str)], tag: &str| {
        let mut env = vec![(ENV_SCENE, "caster"), (ENV_LIGHT, light)];
        env.extend_from_slice(extra);
        capture(path, &env, tag)
    };
    let reference = caster(&[(ENV_NOCARD, "1")], &format!("{light}_nocard"));
    let ratio = |c: &Capture, what: &str| {
        let (r, n) = shadow_ratio(c, &reference, rect);
        assert!(n >= 40, "VACUOUS on {path:?} ({light}): only {n} floor px of the umbra {rect:?} are visible for {what}");
        eprintln!("{path:?} ({light}): {what}: ratio {r:.3} over {n} px");
        r
    };

    let ss_toward = caster(&[], &format!("{light}_ss_toward"));
    let r = ratio(&ss_toward, "single-sided, front toward the light");
    assert!(r <= 0.5, "CONTROL on {path:?} ({light}): a single-sided card facing the light must cast (ratio {r:.3})");
    let ss_away = caster(&[(ENV_FACING, "back")], &format!("{light}_ss_away"));
    let r = ratio(&ss_away, "single-sided, front away from the light");
    // DS-D8 (a stated narrowing): on a `hwrt` Deferred boot the sun's mesh shadow is also traced
    // (`deferred_pbr.hlsl`'s `FORCE_OPAQUE` ray query) and RT shadow rays treat every triangle as
    // two-sided, so there the away-facing single-sided card DOES occlude and K1's control inverts.
    // The raster caster members this test is about are then gated by the software leg.
    if cfg!(feature = "hwrt") && path == RenderPath::Deferred && !point {
        assert!(
            r <= 0.5,
            "CONTROL on {path:?} ({light}, hwrt): RT shadow rays are facing-agnostic (DS-D8), so a single-sided card \
             facing away must still cast here (ratio {r:.3})"
        );
    } else {
        assert!(
            r >= 0.95,
            "CONTROL on {path:?} ({light}): a single-sided card facing away must not cast (K1; ratio {r:.3})"
        );
    }
    let ds_away = caster(&[(ENV_FACING, "back"), (ENV_DS, "1")], &format!("{light}_ds_away"));
    let r = ratio(&ds_away, "double-sided, front away from the light");
    assert!(
        r <= 0.5,
        "RED on {path:?} ({light}): a double-sided card whose authored front faces away from the light casts no \
         shadow (ratio {r:.3}; the caster pass culled it as a single-sided face)"
    );
}

#[test]
#[ignore = "gpu-windowed: RD6 test 3 on Deferred, sun (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_double_sided_caster_deferred() {
    double_sided_caster(RenderPath::Deferred, "sun");
}

#[test]
#[ignore = "gpu-windowed: RD6 test 3 on Forward, sun (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_double_sided_caster_forward() {
    double_sided_caster(RenderPath::Forward, "sun");
}

#[test]
#[ignore = "gpu-windowed: RD6 test 3 on VisibilityBuffer, sun (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_double_sided_caster_vb() {
    double_sided_caster(RenderPath::VisibilityBuffer, "sun");
}

/// W5: the point-light caster family (`point_depth_pipeline`), per path.
#[test]
#[ignore = "gpu-windowed: RD6 test 3 on Deferred, point light (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_double_sided_point_caster_deferred() {
    double_sided_caster(RenderPath::Deferred, "point");
}

#[test]
#[ignore = "gpu-windowed: RD6 test 3 on Forward, point light (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_double_sided_point_caster_forward() {
    double_sided_caster(RenderPath::Forward, "point");
}

#[test]
#[ignore = "gpu-windowed: RD6 test 3 on VisibilityBuffer, point light (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_double_sided_point_caster_vb() {
    double_sided_caster(RenderPath::VisibilityBuffer, "point");
}

// --- 4: a mirrored instance keeps its facing ------------------------------------------------------

/// `extra` arms both runs.
fn mirrored_cube(path: RenderPath, extra: &[(&str, &str)]) {
    let a = capture(path, &with_extra(extra, &[(ENV_SCENE, "cube")]), "cube");
    let b = capture(path, &with_extra(extra, &[(ENV_SCENE, "cube"), (ENV_MIRROR, "1")]), "cube_mirrored");
    let clear = a.depth.depth(EMPTY_PIXEL.0, EMPTY_PIXEL.1);
    let covered = |x: u32, y: u32| a.depth.depth(x, y).to_bits() != clear.to_bits();
    // The cube mask, eroded by 2 px (every pixel within 2 px is the cube's too).
    let mut n = 0;
    let mut worst = (0.0f32, (0, 0));
    for y in 2..WIN - 2 {
        for x in 2..WIN - 2 {
            if !(x - 2..=x + 2).all(|i| (y - 2..=y + 2).all(|j| covered(i, j))) {
                continue;
            }
            n += 1;
            let (pa, pb) = (rgb(&a, x, y), rgb(&b, x, y));
            for k in 0..3 {
                let d = (pa[k] - pb[k]).abs();
                if d > worst.0 {
                    worst = (d, (x, y));
                }
            }
        }
    }
    assert!(n >= 500, "VACUOUS on {path:?}: the cube covers only {n} eroded px");
    assert!(
        worst.0 <= LSB,
        "RED on {path:?}: the X-mirrored cube does not render the unmirrored cube's pixels — max |A-B| {} at {:?} \
         over {n} px (a mirrored instance turned inside-out: its cull or flip ignores the mirror)",
        worst.0,
        worst.1
    );
}

#[test]
#[ignore = "gpu-windowed: RD6 test 4 on Deferred (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_mirrored_cube_deferred() {
    mirrored_cube(RenderPath::Deferred, &[]);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 4 on Forward (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_mirrored_cube_forward() {
    mirrored_cube(RenderPath::Forward, &[]);
}

#[test]
#[ignore = "gpu-windowed: RD6 test 4 on VisibilityBuffer (spawns dds_worker); run with --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_mirrored_cube_vb() {
    mirrored_cube(RenderPath::VisibilityBuffer, &[]);
}

/// Triage r1 T-MV: the MV family's `Front` member (a mirrored single-sided batch). The cube is the
/// pinned default material, so under the temporal denoiser it is drawn by the MV family (see
/// `dds_single_sided_back_culled_deferred_denoised`). Measured red on the MV family built with
/// `cull_mode: Back` for every member (the mirrored cube's member culls its visible faces).
#[cfg(feature = "hwrt")]
#[test]
#[ignore = "feature+gpu-windowed+gpu-cap: RD6 test 4 on Deferred under the temporal denoiser (the MV family's Front member; needs ray query; spawns dds_worker); run with --features hwrt, --test-threads=1 and BOYKO_DISABLE_VALIDATION=1"]
fn dds_mirrored_cube_deferred_denoised() {
    mirrored_cube(RenderPath::Deferred, &[(dds_scene::ENV_DENOISE, "1")]);
}

/// The fixture geometry the gates read, checked on every host: the card mask sits inside the
/// window, the backdrop band beside it, the predicted umbras inside the window.
#[test]
fn the_dds_windows_sit_where_the_gates_need_them() {
    let card = card_mask();
    assert!(card.x0 > 8 && card.x1 < WIN - 8 && card.y0 > 8 && card.y1 < WIN - 8, "card {card:?}");
    let band = backdrop_band();
    assert!(band.x1 + 4 < card.x0 && band.x0 > 0, "band {band:?} vs card {card:?}");
    for point in [false, true] {
        let s = shadow_rect(point);
        assert!(s.x0 > 4 && s.x1 < WIN - 4 && s.y0 > 4 && s.y1 < WIN - 4, "umbra {s:?} (point={point})");
        assert!(s.pixels().count() >= 80, "umbra {s:?} (point={point}) is too small to judge");
    }
}
