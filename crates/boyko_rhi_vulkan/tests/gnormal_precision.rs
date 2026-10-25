//! IBL1 P1 — the Deferred G-buffer normal's precision and layout (verify M1; `ibl1/cut.md` §1.1, §3
//! C1, the critique's C1 / W4).
//!
//! The Deferred G-buffer stored its octahedral normal in the RG lanes of an `R8G8B8A8_UNORM` image:
//! 0.34° mean / 0.95° max error, which steps every polished highlight (verify R12 / R13). IBL1 P1
//! moves `gNormal` to `R16G16_UNORM` (oct16) and the 16-bit material id into the two alpha lanes
//! no shader read (`gAlbedo.A` = low byte, `gMaterial.A` = high byte). These gates pin that layout
//! at every writer and reader, on the HLSL text, on the committed SPIR-V and on the Rust bind side:
//!
//! * **T-N1** [`gnormal_oct_angular_error_bound`] — the shipped oct encode → the `GNORMAL_FORMAT`
//!   quantizer (round-to-nearest AND truncation: this device rounds the `.5` tie down, settle E15) →
//!   the shipped decode, over 2²¹ directions plus the axes and diagonals.
//! * **T-N2** [`gnormal_highlight_step_metric`] — the visible defect: the high-pass energy of the
//!   showcase's GGX highlight shoulder under quantized normals against exact normals.
//! * **T-N3** [`gbuffer_material_id_lanes_round_trip`] — the resolve decodes the id from the two
//!   alpha lanes, and every id round-trips through them.
//! * **T-N4** [`gnormal_layout_census`] — every Deferred `gNormal` declaration is `rg16` /
//!   `float2`, every `gThinNormal` stays `rgba8` (D-P1b), no store writes a 4-lane normal, and every
//!   `gAlbedo` / `gMaterial` writer stores its id byte in the alpha lane.
//! * **T-N6** [`gnormal_slot_spirv_formats`] — the committed `.spv`: every variable named `gNormal`
//!   is declared with `GNORMAL_FORMAT`'s SPIR-V format and every `gThinNormal` with
//!   `VB_THIN_NORMAL_FORMAT`'s, over exactly the expected module sets (a new reader is red until it
//!   is listed). Critique C1: the hwrt à-trous serves the Deferred chain (`gNormal`) and the VB
//!   split (`thin_normal`), so it ships as two modules, one per bound format.
//! * **T-N7** [`vb_split_atrous_binds_the_thin_twin`] — the Rust bind / record side of the same
//!   split: the VB à-trous sets bind `thin_normal` and the VB recorder dispatches the thin twin; the
//!   Deferred sets bind `normal` and its recorder dispatches the base module.
//! * **T-N8** [`gbuffer_harnesses_name_the_shared_formats`] — critique W4 for the device harnesses:
//!   no harness spells a G-buffer format of its own; their MRT pipelines name
//!   `GBUFFER_MRT_COLOR_FORMATS`. (The production pipelines are classified by `lit_format_census`.)
//!
//! T-N5 (the `R16G16_UNORM` boot row) lives beside its table in `src/device.rs`.
//!
//! Host-only: no device, nothing ignored.

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};

use boyko_rhi::Format;
use boyko_rhi_vulkan::present::{GNORMAL_FORMAT, VB_THIN_NORMAL_FORMAT};
use boyko_shaderdsl::cf::EvalCf;
use boyko_shaderdsl::oct::{oct_decode_body, oct_encode_body};
use boyko_shaderdsl::pack::pack_material_id_ba_body;

// ---- shared helpers --------------------------------------------------------------------------

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workspace_root() -> PathBuf {
    crate_dir().join("..").join("..")
}

fn shaders_dir() -> PathBuf {
    crate_dir().join("shaders")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("invariant: {} is readable UTF-8: {e}", path.display()))
}

/// `text` without `//` line comments and `/* */` block comments (newlines kept). String literals
/// are kept: the HLSL format qualifiers this census reads are strings.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let line = rest.find("//");
        let block = rest.find("/*");
        match (line, block) {
            (Some(l), b) if b.is_none_or(|b| l < b) => {
                out.push_str(&rest[..l]);
                rest = rest[l..].find('\n').map_or("", |k| &rest[l + k..]);
            }
            (_, Some(b)) => {
                out.push_str(&rest[..b]);
                let after = &rest[b + 2..];
                let end = after.find("*/").map_or(after.len(), |k| k + 2);
                out.extend(after[..end].chars().filter(|&c| c == '\n'));
                rest = &after[end..];
            }
            _ => {
                out.push_str(rest);
                rest = "";
            }
        }
    }
    out
}

/// Bits per oct channel of the normal stored in `format`.
fn oct_bits(format: Format) -> u32 {
    match format {
        Format::R16G16Unorm => 16,
        Format::R8G8B8A8Unorm => 8,
        other => panic!("invariant: no oct quantizer model for {other:?}"),
    }
}

/// The shipped encode: the eDSL body the HLSL `oct_encode` is spliced from, on the `f32` backend.
fn oct_encode(n: [f32; 3]) -> [f32; 2] {
    let ret = Cell::new([0.0_f32; 2]);
    let _ = oct_encode_body::<EvalCf>(n, &ret);
    ret.get()
}

/// The shipped decode (the eDSL body) plus the `normalize` tail the HLSL wraps around it.
fn oct_decode(e: [f32; 2]) -> [f32; 3] {
    let [x, y, z] = oct_decode_body::<EvalCf>(e[0], e[1]);
    let len = (x * x + y * y + z * z).sqrt();
    [x / len, y / len, z / len]
}

/// How the device converts a float to a UNORM code.
#[derive(Clone, Copy, Debug)]
enum Rounding {
    /// `floor(x·m + 0.5)` — the D3D reference rule.
    Nearest,
    /// `floor(x·m)` — the worst case of a device that rounds the tie (and more) down.
    Truncate,
}

/// The UNORM code of `x` at `bits` per channel, read back as the shader sees it (`code / m`).
fn unorm_round_trip(x: f32, bits: u32, rounding: Rounding) -> f32 {
    let m = f64::from((1u32 << bits) - 1);
    let scaled = f64::from(x.clamp(0.0, 1.0)) * m;
    let code = match rounding {
        Rounding::Nearest => (scaled + 0.5).floor(),
        Rounding::Truncate => scaled.floor(),
    }
    .min(m);
    (code / m) as f32
}

/// `n` through the G-buffer: encode, quantize at `bits`, decode.
fn store_normal(n: [f32; 3], bits: u32, rounding: Rounding) -> [f32; 3] {
    let e = oct_encode(n);
    oct_decode([unorm_round_trip(e[0], bits, rounding), unorm_round_trip(e[1], bits, rounding)])
}

/// The angle between `a` and `b` in degrees, as `atan2(|a×b|, a·b)` in f64 (insensitive to the
/// operands' f32 norm error, which `acos` would amplify to ~0.02°).
fn angle_deg(a: [f32; 3], b: [f32; 3]) -> f64 {
    let a = a.map(f64::from);
    let b = b.map(f64::from);
    let c = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let cross = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
    let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    cross.atan2(dot).to_degrees()
}

/// SplitMix64: a fixed, platform-independent stream for the direction set.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A uniform `[0, 1)` f64 from the stream.
fn unit01(state: &mut u64) -> f64 {
    (splitmix64(state) >> 11) as f64 / (1u64 << 53) as f64
}

// ---- T-N1 ------------------------------------------------------------------------------------

/// Directions in the T-N1 sweep (2²¹, plus the axes and diagonals).
const T_N1_DIRECTIONS: usize = 1 << 21;

/// T-N1 bounds, degrees, per rounding model: `(mean, max)`. Derived from a numpy model of the same
/// pipeline (`D:/tmp/render-orch/ibl1/scratch/tn1_oct_error.py`, f32 encode / decode, 2²¹
/// directions): oct16 nearest 0.00131 / 0.00372, truncation 0.00261 / 0.00741 — each bound ≈ 1.5×
/// the mean and ≈ 1.35× the max (critique OQ1: a margin, not 13 %). oct8 measures 0.336 / 0.953
/// (nearest; verify R12: 0.335 / 0.942) and 0.671 / 1.906 (truncation): red by > 150×.
const T_N1_BOUNDS: [(Rounding, f64, f64); 2] = [(Rounding::Nearest, 0.002, 0.005), (Rounding::Truncate, 0.004, 0.010)];

#[test]
fn gnormal_oct_angular_error_bound() {
    let bits = oct_bits(GNORMAL_FORMAT);
    let mut dirs: Vec<[f32; 3]> = Vec::with_capacity(T_N1_DIRECTIONS + 14);
    let mut state = 0x1B1_0F00D_u64;
    for _ in 0..T_N1_DIRECTIONS {
        let z = 2.0 * unit01(&mut state) - 1.0;
        let phi = core::f64::consts::TAU * unit01(&mut state);
        let r = (1.0 - z * z).max(0.0).sqrt();
        dirs.push([(r * phi.cos()) as f32, (r * phi.sin()) as f32, z as f32]);
    }
    for axis in 0..3 {
        for sign in [1.0_f32, -1.0] {
            let mut d = [0.0_f32; 3];
            d[axis] = sign;
            dirs.push(d);
        }
    }
    let k = 1.0 / 3.0_f32.sqrt();
    for sx in [k, -k] {
        for sy in [k, -k] {
            for sz in [k, -k] {
                dirs.push([sx, sy, sz]);
            }
        }
    }

    let mut report = Vec::new();
    let mut breaches = Vec::new();
    for (rounding, mean_bound, max_bound) in T_N1_BOUNDS {
        let mut sum = 0.0_f64;
        let mut max = 0.0_f64;
        let mut worst = [0.0_f32; 3];
        for &d in &dirs {
            let err = angle_deg(store_normal(d, bits, rounding), d);
            sum += err;
            if err > max {
                max = err;
                worst = d;
            }
        }
        let mean = sum / dirs.len() as f64;
        report.push(format!("{bits}-bit {rounding:?}: mean {mean:.5}° max {max:.5}° (at {worst:?})"));
        if mean > mean_bound || max > max_bound {
            breaches.push(format!("{rounding:?}: mean {mean:.5}° (≤ {mean_bound}) max {max:.5}° (≤ {max_bound})"));
        }
    }
    println!("T-N1 over {} directions, GNORMAL_FORMAT = {GNORMAL_FORMAT:?}:\n  {}", dirs.len(), report.join("\n  "));
    assert!(
        breaches.is_empty(),
        "the G-buffer normal ({GNORMAL_FORMAT:?}, {bits} bits per oct channel) exceeds the angular bound:\n  {}",
        breaches.join("\n  ")
    );
}

// ---- T-N2 ------------------------------------------------------------------------------------

/// The showcase camera (`crates/boyko_app/tests/pbr_material_showcase.rs`): eye, target, vertical
/// fov, and the square window this metric renders.
const EYE: [f64; 3] = [0.0, 1.6, 4.2];
const TARGET: [f64; 3] = [0.0, 1.0, 0.0];
const FOV_Y_DEG: f64 = 45.0;
const WINDOW: u32 = 1280;
/// The showcase sphere: centre and radius.
const SPHERE_C: [f64; 3] = [0.0, 1.05, 0.0];
const SPHERE_R: f64 = 1.0;
/// The showcase sun (direction TO the light, normalized below) and its intensity.
const SUN_DIR: [f64; 3] = [-0.55, 0.30, 0.42];
const SUN_E: f64 = 5.5;
/// The polished gold's GGX alpha (roughness 0.18).
const ALPHA: f64 = 0.0324;
/// The crop around the highlight.
const CROP: u32 = 256;
/// The shoulder band, as a fraction of the crop's exact peak.
const BAND: (f64, f64) = (0.2, 0.8);
/// The high-pass box width. A 3×3 box keeps the quantization steps (an oct8 step spans ≈ 2.6 px of
/// this sphere's normal gradient) and rejects the highlight's own curvature: with the cut's 15-px
/// box the exact image's own residual dominates (the highlight is ≈ 30 px wide at α 0.0324), and
/// oct8 measured only 2.04× its exact energy (`scratch/tn2b.py`); at 3×3 oct8 measures 28× and
/// oct16 1.02×.
const HIGH_PASS_BOX: usize = 3;
/// The allowed ratio of the quantized shoulder's high-pass energy to the exact one's.
const MAX_STEP_RATIO: f64 = 2.0;

fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalize3(a: [f64; 3]) -> [f64; 3] {
    let l = dot3(a, a).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}

/// The sun's direct GGX specular (D · V_SmithGGXCorrelated · NoL · E, Fresnel 1: the metric needs
/// the highlight's shape, not the shipped BRDF's bits) at window pixel `(px, py)`; `bits` quantizes
/// the normal through the G-buffer (`None` = the exact analytic normal). `0` off the sphere.
fn highlight(px: u32, py: u32, bits: Option<u32>) -> f64 {
    let fwd = normalize3(sub3(TARGET, EYE));
    let right = normalize3(cross3(fwd, [0.0, 1.0, 0.0]));
    let up = cross3(right, fwd);
    let t = (FOV_Y_DEG.to_radians() * 0.5).tan();
    let u = ((f64::from(px) + 0.5) / f64::from(WINDOW) * 2.0 - 1.0) * t;
    let v = (1.0 - (f64::from(py) + 0.5) / f64::from(WINDOW) * 2.0) * t;
    let rd = normalize3([
        fwd[0] + u * right[0] + v * up[0],
        fwd[1] + u * right[1] + v * up[1],
        fwd[2] + u * right[2] + v * up[2],
    ]);
    let oc = sub3(EYE, SPHERE_C);
    let b = dot3(rd, oc);
    let disc = b * b - (dot3(oc, oc) - SPHERE_R * SPHERE_R);
    if disc <= 0.0 {
        return 0.0;
    }
    let hit = -b - disc.sqrt();
    let p = [EYE[0] + rd[0] * hit, EYE[1] + rd[1] * hit, EYE[2] + rd[2] * hit];
    let mut n = normalize3(sub3(p, SPHERE_C));
    if let Some(bits) = bits {
        n = store_normal(n.map(|c| c as f32), bits, Rounding::Nearest).map(f64::from);
    }
    let l = normalize3(SUN_DIR);
    let view = [-rd[0], -rd[1], -rd[2]];
    let h = normalize3([l[0] + view[0], l[1] + view[1], l[2] + view[2]]);
    let nol = dot3(n, l).clamp(0.0, 1.0);
    let nov = dot3(n, view).clamp(1e-4, 1.0);
    let noh = dot3(n, h).clamp(0.0, 1.0);
    let a2 = ALPHA * ALPHA;
    let dd = noh * noh * (a2 - 1.0) + 1.0;
    let d = a2 / (core::f64::consts::PI * dd * dd);
    let vis = 0.5 / (nol * (nov * nov * (1.0 - a2) + a2).sqrt() + nov * (nol * nol * (1.0 - a2) + a2).sqrt() + 1e-12);
    d * vis * nol * SUN_E
}

/// `img - box(img)` (edge-clamped `HIGH_PASS_BOX`² mean) on a `CROP`² image.
fn high_pass(img: &[f64]) -> Vec<f64> {
    let s = CROP as usize;
    let r = (HIGH_PASS_BOX / 2) as isize;
    let at = |x: isize, y: isize| img[(y.clamp(0, s as isize - 1) as usize) * s + x.clamp(0, s as isize - 1) as usize];
    let mut out = vec![0.0; s * s];
    for y in 0..s as isize {
        for x in 0..s as isize {
            let mut sum = 0.0;
            for dy in -r..=r {
                for dx in -r..=r {
                    sum += at(x + dx, y + dy);
                }
            }
            out[y as usize * s + x as usize] = at(x, y) - sum / (HIGH_PASS_BOX * HIGH_PASS_BOX) as f64;
        }
    }
    out
}

/// The standard deviation of `hp` over the pixels `band` selects.
fn band_std(hp: &[f64], band: &[bool]) -> f64 {
    let vals: Vec<f64> = hp.iter().zip(band).filter(|(_, b)| **b).map(|(v, _)| *v).collect();
    let mean = vals.iter().sum::<f64>() / vals.len() as f64;
    (vals.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / vals.len() as f64).sqrt()
}

#[test]
fn gnormal_highlight_step_metric() {
    let bits = oct_bits(GNORMAL_FORMAT);
    // Locate the highlight peak on a coarse grid, then crop around it.
    let (mut cx, mut cy, mut peak) = (0u32, 0u32, 0.0_f64);
    for py in (0..WINDOW).step_by(4) {
        for px in (0..WINDOW).step_by(4) {
            let i = highlight(px, py, None);
            if i > peak {
                (cx, cy, peak) = (px, py, i);
            }
        }
    }
    assert!(peak > 0.0, "the crop search found no highlight on the sphere");
    let (x0, y0) = (cx.saturating_sub(CROP / 2), cy.saturating_sub(CROP / 2));
    let render = |bits: Option<u32>| -> Vec<f64> {
        (0..CROP).flat_map(|y| (0..CROP).map(move |x| highlight(x0 + x, y0 + y, bits))).collect()
    };
    let exact = render(None);
    let quant = render(Some(bits));
    let i_max = exact.iter().copied().fold(0.0, f64::max);
    let exact: Vec<f64> = exact.iter().map(|v| v / i_max).collect();
    let quant: Vec<f64> = quant.iter().map(|v| v / i_max).collect();
    let band: Vec<bool> = exact.iter().map(|&v| (BAND.0..=BAND.1).contains(&v)).collect();
    let band_px = band.iter().filter(|b| **b).count();
    assert!(band_px >= 200, "the shoulder band holds only {band_px} px (crop centre ({cx}, {cy})): the crop missed the highlight");
    let hp_exact = band_std(&high_pass(&exact), &band);
    let hp_quant = band_std(&high_pass(&quant), &band);
    let ratio = hp_quant / hp_exact;
    println!(
        "T-N2: {GNORMAL_FORMAT:?} ({bits} bits): crop centre ({cx}, {cy}), peak {i_max:.3}, band {band_px} px, \
         high-pass std exact {hp_exact:.6} quantized {hp_quant:.6}, ratio {ratio:.3}"
    );
    assert!(
        hp_quant <= MAX_STEP_RATIO * hp_exact + 1e-4,
        "the highlight shoulder steps under the {bits}-bit G-buffer normal: high-pass {hp_quant:.6} vs exact \
         {hp_exact:.6} (ratio {ratio:.2} > {MAX_STEP_RATIO})"
    );
}

// ---- T-N3 ------------------------------------------------------------------------------------

/// The expression the resolve's `uint <var> = (uint)(<expr> * 255.0 + 0.5);` decode reads.
fn id_decode_source(resolve: &str, var: &str) -> Option<String> {
    let head = format!("uint {var} = (uint)(");
    let start = resolve.find(&head)? + head.len();
    let end = resolve[start..].find(" * 255.0 + 0.5)")?;
    Some(resolve[start..start + end].trim().to_owned())
}

#[test]
fn gbuffer_material_id_lanes_round_trip() {
    let resolve = strip_comments(&read(&shaders_dir().join("deferred_pbr.hlsl")));
    let lo = id_decode_source(&resolve, "id_lo");
    let hi = id_decode_source(&resolve, "id_hi");
    assert_eq!(
        (lo.as_deref(), hi.as_deref()),
        (Some("albedo_texel.a"), Some("material_texel.a")),
        "deferred_pbr.hlsl must decode the material id from gAlbedo.A (low byte) and gMaterial.A (high byte)"
    );
    assert!(resolve.contains("uint mat_id = id_lo | (id_hi << 8);"), "the resolve's id assembly moved");

    for id in [0u32, 1, 255, 256, 0x1234, 0xFFFF] {
        let ret = Cell::new([0.0_f32; 2]);
        let _ = pack_material_id_ba_body::<EvalCf>(id, &ret);
        // The writer's two lanes through the RGBA8 store, read back as the resolve's floats.
        let [a_lo, a_hi] = ret.get().map(|v| unorm_round_trip(v, 8, Rounding::Nearest));
        let decode = |x: f32| (x * 255.0 + 0.5) as u32;
        assert_eq!(decode(a_lo) | (decode(a_hi) << 8), id, "material id {id:#06x} does not round-trip");
    }
}

// ---- T-N4 ------------------------------------------------------------------------------------

/// The `.hlsl` sources that declare the Deferred `gNormal` (one declaration each).
const GNORMAL_SOURCES: [&str; 7] = [
    "deferred_pbr.hlsl",
    "sdf_gbuffer_composite.hlsl",
    "sdf_ssao.comp.hlsl",
    "sdf_ssao_high.comp.hlsl",
    "sdf_ssao_low.comp.hlsl",
    "sdf_ssao_medium.comp.hlsl",
    "shadow_atrous.comp.hlsl",
];

/// The sources that declare the VB split's `gThinNormal` (one declaration each).
const THIN_NORMAL_SOURCES: [&str; 7] = [
    "sdf_ssao.comp.hlsl",
    "sdf_ssao_high.comp.hlsl",
    "sdf_ssao_low.comp.hlsl",
    "sdf_ssao_medium.comp.hlsl",
    "shadow_atrous.comp.hlsl",
    "vb_geo.comp.hlsl",
    "vb_shadow_vis.comp.hlsl",
];

/// Every `.hlsl` / `.hlsli` under `shaders/`, by file name, comments stripped, in a stable order.
fn hlsl_sources() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = fs::read_dir(shaders_dir())
        .expect("invariant: shaders/ is readable")
        .map(|e| e.expect("invariant: a readable entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "hlsl" || x == "hlsli"))
        .map(|p| {
            let name = p.file_name().expect("invariant: a file name").to_string_lossy().into_owned();
            (name, strip_comments(&read(&p)))
        })
        .collect();
    out.sort();
    out
}

/// Every `[[vk::image_format("<q>")]] RWTexture2D<<t>> <var>` declaration in `text`, as `(q, t)`
/// (`q` empty when the line carries no format pin).
fn storage_decls(text: &str, var: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(at) = line.find("RWTexture2D<") else { continue };
        let after = &line[at + "RWTexture2D<".len()..];
        let Some(close) = after.find('>') else { continue };
        let ident: String = after[close + 1..].trim_start().chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if ident != var {
            continue;
        }
        let qualifier = line
            .find("image_format(\"")
            .map(|q| {
                let s = &line[q + "image_format(\"".len()..];
                s[..s.find('"').unwrap_or(s.len())].to_owned()
            })
            .unwrap_or_default();
        out.push((qualifier, after[..close].trim().to_owned()));
    }
    out
}

/// Every `<lhs> = ...;` assignment in `text`: `Ok(top-level arguments)` when the right-hand side is a
/// `<ctor>(...)` constructor, `Err(the right-hand side's first line)` otherwise.
fn stores(text: &str, lhs: &str, ctor: &str) -> Vec<Result<Vec<String>, String>> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(off) = text[from..].find(lhs) {
        let at = from + off + lhs.len();
        from = at;
        let rest = &text[at..];
        // `lhs` names an image (`gAlbedo[...]`) or a struct field (`output.albedo`).
        let rest = if lhs.ends_with('[') { rest.find(']').map_or("", |k| &rest[k + 1..]) } else { rest };
        let rest = rest.trim_start();
        let Some(rhs) = rest.strip_prefix('=') else { continue };
        let rhs = rhs.trim_start();
        let Some(args_start) = rhs.strip_prefix(ctor).and_then(|r| r.strip_prefix('(')) else {
            out.push(Err(rhs.lines().next().unwrap_or("").trim().to_owned()));
            continue;
        };
        let mut depth = 1usize;
        let mut args = Vec::new();
        let mut cur = String::new();
        for c in args_start.chars() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                ',' if depth == 1 => {
                    args.push(cur.trim().to_owned());
                    cur.clear();
                    continue;
                }
                _ => {}
            }
            cur.push(c);
        }
        args.push(cur.trim().to_owned());
        out.push(Ok(args));
    }
    out
}

#[test]
fn gnormal_layout_census() {
    let sources = hlsl_sources();
    assert!(sources.len() > 40, "anti-vacuity: only {} shader sources read", sources.len());
    let mut breaches = Vec::new();

    // (a) every Deferred gNormal declaration is the oct16 RG16 pin; (b) every gThinNormal is RGBA8.
    for (var, expected, (q, t)) in
        [("gNormal", &GNORMAL_SOURCES[..], ("rg16", "float2")), ("gThinNormal", &THIN_NORMAL_SOURCES[..], ("rgba8", "float4"))]
    {
        let mut declaring = Vec::new();
        for (name, text) in &sources {
            let decls = storage_decls(text, var);
            if decls.is_empty() {
                continue;
            }
            declaring.push(name.as_str());
            for (dq, dt) in &decls {
                if dq != q || dt != t {
                    breaches.push(format!("{name}: `{var}` is declared `{dq}` / `{dt}`, expected `{q}` / `{t}`"));
                }
            }
            if decls.len() != 1 {
                breaches.push(format!("{name}: {} declarations of `{var}`", decls.len()));
            }
        }
        if declaring != expected {
            breaches.push(format!("the sources declaring `{var}` are {declaring:?}, expected {expected:?} (a reader was added or dropped)"));
        }
    }

    // (c) no shader stores a 4-lane value into gNormal: a `float2(x, y)` or the encoded `oct`.
    for (name, text) in &sources {
        for store in stores(text, "gNormal[", "float2") {
            match store {
                Ok(args) if args.len() == 2 => {}
                Err(rhs) if rhs == "oct;" => {}
                other => breaches.push(format!("{name}: a gNormal store is not a float2: {other:?}")),
            }
        }
    }

    // (d) the mesh raster: the normal MRT is float2; albedo.a / material.a carry the id bytes.
    let mrt = &sources.iter().find(|(n, _)| n == "gbuffer_mrt.fs.hlsl").expect("invariant: gbuffer_mrt.fs.hlsl").1;
    if !mrt.lines().any(|l| l.contains("float2 normal") && l.contains("SV_Target1")) {
        breaches.push("gbuffer_mrt.fs.hlsl: SV_Target1 (gNormal) is not a float2".to_owned());
    }
    for (lhs, lane) in [("output.albedo", "id_ba.x"), ("output.material", "id_ba.y")] {
        let all = stores(mrt, lhs, "float4");
        if all.is_empty() {
            breaches.push(format!("gbuffer_mrt.fs.hlsl: no `{lhs} = float4(...)` store"));
        }
        for store in all {
            match &store {
                Ok(args) if args.len() >= 2 && args.last().map(String::as_str) == Some(lane) => {}
                _ => breaches.push(format!("gbuffer_mrt.fs.hlsl: `{lhs}` stores {store:?}; its alpha lane must be `{lane}`")),
            }
        }
    }
    if !mrt.contains("output.normal = oct;") {
        breaches.push("gbuffer_mrt.fs.hlsl: the normal MRT store is not `output.normal = oct;`".to_owned());
    }

    // (e) the composite: every attribute store carries its id byte; the one neutral store per image
    // (the empty-tile background arm) writes id 0; the mesh arm preserves the raster's high byte.
    let comp = &sources.iter().find(|(n, _)| n == "sdf_gbuffer_composite.hlsl").expect("invariant: the composite").1;
    for (lhs, lanes) in [("gAlbedo[", &["id_ba.x"][..]), ("gMaterial[", &["id_ba.y", "id_hi"][..])] {
        let all = stores(comp, lhs, "float4");
        let mut neutral = 0;
        for store in &all {
            match store.as_ref().ok().filter(|a| a.len() >= 2).and_then(|a| a.last()).map(String::as_str) {
                Some(l) if lanes.contains(&l) => {}
                Some("0.0") => neutral += 1,
                _ => breaches.push(format!("sdf_gbuffer_composite.hlsl: `{lhs}..] = {store:?}`: the alpha lane is not one of {lanes:?}")),
            }
        }
        if neutral != 1 || all.len() < 2 {
            breaches.push(format!("sdf_gbuffer_composite.hlsl: {} `{lhs}..]` stores, {neutral} neutral (expected exactly 1)", all.len()));
        }
    }
    if !comp.contains("float id_hi = gMaterial[uint2(px, py)].a;") {
        breaches.push("sdf_gbuffer_composite.hlsl: the mesh arm does not read the raster's id high byte back".to_owned());
    }

    assert!(breaches.is_empty(), "the G-buffer normal layout census is RED ({}):\n  {}", breaches.len(), breaches.join("\n  "));
}

// ---- T-N6 ------------------------------------------------------------------------------------

/// The committed modules that declare `gNormal` (the Deferred normal) — incl. the four IBL1
/// `-D REFL=1` resolve twins, which read it exactly as their base rows.
const GNORMAL_MODULES: [&str; 15] = [
    "deferred_pbr.comp.spv",
    "deferred_pbr_hwrt.comp.spv",
    "deferred_pbr_hwrt_denoised.comp.spv",
    "deferred_pbr_hwrt_denoised_refl.comp.spv",
    "deferred_pbr_hwrt_refl.comp.spv",
    "deferred_pbr_hwrt_vis.comp.spv",
    "deferred_pbr_hwrt_vis_mv.comp.spv",
    "deferred_pbr_refl.comp.spv",
    "deferred_pbr_wrap.comp.spv",
    "deferred_pbr_wrap_refl.comp.spv",
    "sdf_gbuffer_composite.comp.spv",
    "sdf_ssao_high.comp.spv",
    "sdf_ssao_low.comp.spv",
    "sdf_ssao_medium.comp.spv",
    "shadow_atrous.comp.spv",
];

/// The committed modules that declare `gThinNormal` (the VB split's thin-aux normal).
const THIN_NORMAL_MODULES: [&str; 8] = [
    "sdf_ssao_vb_high.comp.spv",
    "sdf_ssao_vb_low.comp.spv",
    "sdf_ssao_vb_medium.comp.spv",
    "shadow_atrous_vb.comp.spv",
    "vb_geo.comp.spv",
    "vb_geo_mv.comp.spv",
    "vb_geo_sv0.comp.spv",
    "vb_shadow_vis.comp.spv",
];

/// SPIR-V `ImageFormat` of a storage image bound with `format`.
fn spirv_image_format(format: Format) -> u32 {
    match format {
        Format::R8G8B8A8Unorm => 4,
        Format::R16G16Unorm => 12,
        other => panic!("invariant: no SPIR-V ImageFormat mapping for {other:?}"),
    }
}

/// The SPIR-V `ImageFormat` of every variable named `var` in the module `words`.
fn image_formats_of(words: &[u32], var: &str) -> Vec<u32> {
    const OP_NAME: u32 = 5;
    const OP_TYPE_IMAGE: u32 = 25;
    const OP_TYPE_POINTER: u32 = 32;
    const OP_VARIABLE: u32 = 59;
    assert!(words.len() > 5 && words[0] == 0x0723_0203, "invariant: a SPIR-V module");
    let mut names = Vec::new();
    let mut images = Vec::new();
    let mut pointers = Vec::new();
    let mut variables = Vec::new();
    let mut i = 5;
    while i < words.len() {
        let count = (words[i] >> 16) as usize;
        let op = words[i] & 0xFFFF;
        assert!(count > 0 && i + count <= words.len(), "invariant: a well-formed instruction stream");
        let ins = &words[i..i + count];
        match op {
            OP_NAME => {
                let bytes: Vec<u8> = ins[2..].iter().flat_map(|w| w.to_le_bytes()).take_while(|&b| b != 0).collect();
                names.push((ins[1], String::from_utf8_lossy(&bytes).into_owned()));
            }
            OP_TYPE_IMAGE => images.push((ins[1], ins[8])),
            OP_TYPE_POINTER => pointers.push((ins[1], ins[3])),
            OP_VARIABLE => variables.push((ins[2], ins[1])),
            _ => {}
        }
        i += count;
    }
    names
        .iter()
        .filter(|(_, n)| n == var)
        .filter_map(|(id, _)| variables.iter().find(|(v, _)| v == id))
        .filter_map(|(_, ptr)| pointers.iter().find(|(p, _)| p == ptr))
        .filter_map(|(_, ty)| images.iter().find(|(t, _)| t == ty))
        .map(|&(_, fmt)| fmt)
        .collect()
}

#[test]
fn gnormal_slot_spirv_formats() {
    let mut modules: Vec<PathBuf> = fs::read_dir(shaders_dir())
        .expect("invariant: shaders/ is readable")
        .map(|e| e.expect("invariant: a readable entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "spv"))
        .collect();
    modules.sort();
    assert!(modules.len() > 100, "anti-vacuity: only {} committed modules", modules.len());
    let mut breaches = Vec::new();
    for (var, format, expected) in
        [("gNormal", GNORMAL_FORMAT, &GNORMAL_MODULES[..]), ("gThinNormal", VB_THIN_NORMAL_FORMAT, &THIN_NORMAL_MODULES[..])]
    {
        let want = spirv_image_format(format);
        let mut declaring = Vec::new();
        for path in &modules {
            let bytes = fs::read(path).expect("invariant: a readable module");
            let words: Vec<u32> = bytes.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
            let found = image_formats_of(&words, var);
            if found.is_empty() {
                continue;
            }
            let name = path.file_name().expect("invariant: a file name").to_string_lossy().into_owned();
            if found.iter().any(|&f| f != want) {
                breaches.push(format!("{name}: `{var}` is declared with SPIR-V ImageFormat {found:?}; {format:?} binds {want}"));
            }
            declaring.push(name);
        }
        if declaring != expected {
            breaches.push(format!("the modules declaring `{var}` are {declaring:?}, expected {expected:?}"));
        }
    }
    assert!(breaches.is_empty(), "the G-buffer normal SPIR-V census is RED:\n  {}", breaches.join("\n  "));
}

// ---- T-N7 ------------------------------------------------------------------------------------

/// The body of `fn <name>(` in `text` (from the signature to the matching close brace).
fn fn_body<'a>(text: &'a str, name: &str) -> &'a str {
    let head = format!("fn {name}(");
    let start = text.find(&head).unwrap_or_else(|| panic!("invariant: `fn {name}` exists"));
    let open = start + text[start..].find('{').expect("invariant: a fn body");
    let mut depth = 0usize;
    for (i, c) in text[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &text[start..open + i + 1];
                }
            }
            _ => {}
        }
    }
    &text[start..]
}

#[test]
fn vb_split_atrous_binds_the_thin_twin() {
    let src = crate_dir().join("src");
    let targets = strip_comments(&read(&src.join("present/targets.rs")));
    let vb = strip_comments(&read(&src.join("present/passes/vb.rs")));
    let deferred = strip_comments(&read(&src.join("present/passes/gbuffer.rs")));
    let gpu_scene = strip_comments(&read(&workspace_root().join("crates/boyko_app/src/gpu_scene/mod.rs")));
    let mut breaches = Vec::new();

    let vb_sets = fn_body(&targets, "build_vb_shadow_atrous_sets");
    if !vb_sets.contains("texture: &tn[slot]") || vb_sets.contains("&normal[") {
        breaches.push("targets.rs: build_vb_shadow_atrous_sets must bind `thin_normal` (and never `normal`)".to_owned());
    }
    let deferred_sets = fn_body(&targets, "build_shadow_denoise_sets");
    if !deferred_sets.contains("texture: &normal[slot]") || deferred_sets.contains("&tn[") {
        breaches.push("targets.rs: build_shadow_denoise_sets' à-trous must bind `normal` (and never `thin_normal`)".to_owned());
    }
    if !vb.contains("sh.atrous_pipeline_thin.pipeline") || !vb.contains("sh.atrous_pipeline_thin.layout") || vb.contains("sh.atrous_pipeline.") {
        breaches.push("passes/vb.rs: the VB à-trous must dispatch `atrous_pipeline_thin` (the gThinNormal module)".to_owned());
    }
    if !deferred.contains("sh.atrous_pipeline.pipeline") || deferred.contains("atrous_pipeline_thin") {
        breaches.push("passes/gbuffer.rs: the Deferred à-trous must dispatch `atrous_pipeline` (the gNormal module)".to_owned());
    }
    if !gpu_scene.contains("shadow_atrous_vb_spirv()") {
        breaches.push("boyko_app gpu_scene/mod.rs: the thin twin is not built from `shadow_atrous_vb_spirv()`".to_owned());
    }
    assert!(breaches.is_empty(), "the shared à-trous binds a normal its module does not declare:\n  {}", breaches.join("\n  "));
}

// ---- T-N8 ------------------------------------------------------------------------------------

/// The device harnesses that build their own G-buffer and mesh-MRT pipelines, and how many MRT
/// pipelines each builds.
const HARNESSES: [(&str, usize); 2] = [("tests/window_present_gbuffer.rs", 4), ("tests/sdf_gbuffer_hybrid.rs", 3)];

#[test]
fn gbuffer_harnesses_name_the_shared_formats() {
    let mut breaches = Vec::new();
    for (file, mrt_pipelines) in HARNESSES {
        let text = strip_comments(&read(&crate_dir().join(file)));
        for local in ["const GBUFFER_FORMAT", "const RASTER_COLOR_FORMAT", "const GNORMAL_FORMAT"] {
            if text.contains(local) {
                breaches.push(format!("{file}: defines `{local}` — a G-buffer format of its own"));
            }
        }
        let named = text.matches("color_formats: &GBUFFER_MRT_COLOR_FORMATS").count();
        if named != mrt_pipelines {
            breaches.push(format!("{file}: {named} MRT pipelines name GBUFFER_MRT_COLOR_FORMATS, expected {mrt_pipelines}"));
        }
    }
    assert!(breaches.is_empty(), "a harness spells the G-buffer formats itself:\n  {}", breaches.join("\n  "));
}
