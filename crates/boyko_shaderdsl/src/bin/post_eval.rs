//! Render wave PX0 (H3 Scope 5; `px0/cut.md` D-T2) — **`post_eval`, the display oracle**: the host
//! `f32` instantiation of `post_final`'s traced body (`post::post_final_main::<EvalCf>`) over a
//! leg's own dumps, so a display dump can be judged pixel by pixel against the body the shader runs.
//!
//! A pure function of explicit inputs — it reads OBSD dumps (`boyko_shaderdsl::dump`) and the eight
//! `PostFinalGpu` words, never a receipt (`tools/golden_diff.py eval-post` resolves those) — and it
//! contains no stage: every row of `post::POST_FINAL_BINDINGS` is resolved by its KIND, and the
//! resources are built by `post::eval_res`, so a rung that grows the body adds rows in `post.rs`
//! and leaves this file alone (the oracle census in `post_edsl_sync.rs` reds a stage call here).
//!
//! ```text
//! post_eval bindings
//!     One line per row: `<name> <kind> <source>` — source `resource:<name>`, `pass:<pass>:<k>`
//!     (the k-th fetching row of that pass ↔ the pass's k-th declared read access), or `none`.
//! post_eval eval --words w0,..,w7 --bind <row>=<dump.bin>... --display <dump.bin>
//!                [--subtexel-bits N]
//!     Evaluates every pixel of the display's extent and compares within the per-pixel bound:
//!     1 LSB (the two 8-bit roundings) + E0's bound on the OETF's `pow` + one filter term per
//!     FILTERED row (the pixel re-evaluated with that row's texture at ±2^-N TEXEL per axis, the
//!     largest display change). `--subtexel-bits` is REQUIRED when a filtered row is bound.
//!     Exit 0 every pixel within, 1 otherwise, 2 an input error (a missing binding is a named one).
//! post_eval bound --words .. --bind .. --steps K
//!     The tolerance pin's `max_delta_bound`: per pixel, the largest display change over a JOINT
//!     perturbation of the scene input by ±K format steps — 8 channel sign patterns × {the
//!     footprint of radius `POST_FINAL_TAP_RADIUS` the same sign, the centre opposite} (verify r5
//!     O2; 8 evaluations at radius 0) — maximised over pixels. Prints `max_delta_bound=<lsb>`.
//! ```

use std::process::ExitCode;

use boyko_shaderdsl::cf::{EvalCf, HostTex2, HostTex3};
use boyko_shaderdsl::dump;
use boyko_shaderdsl::pixfmt::{self, Channel};
use boyko_shaderdsl::post::{
    self, DumpSource, EvalBinding, OETF_GAMMA_EXP, POST_FINAL_BINDINGS, POST_FINAL_TAP_RADIUS,
    PostBinding, PostBindingKind,
};
use boyko_shaderdsl::prims_probe::vk_pow_bound;

/// A decoded image dump: its extent, its layers (a 3D image's depth), its format and its texels.
struct Image {
    w: u32,
    h: u32,
    layers: u32,
    format: u32,
    texels: Vec<[f32; 4]>,
}

/// One row's loaded contents.
enum Loaded {
    Image(Image),
    Words(Vec<u32>),
}

fn read(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{path}: {e}"))
}

fn load_image(path: &str) -> Result<Image, String> {
    let bytes = read(path)?;
    let (h, t) = dump::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
    if h.format == pixfmt::FORMAT_BUFFER {
        return Err(format!("{path}: a buffer dump where an image was expected"));
    }
    let tb = h.texel_bytes as usize;
    let texels = t
        .chunks_exact(tb)
        .map(|c| pixfmt::decode_texel(h.format, c).ok_or_else(|| format!("{path}: VkFormat {} has no decoder", h.format)))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Image { w: h.width, h: h.height, layers: h.layers, format: h.format, texels })
}

fn load_words(path: &str) -> Result<Vec<u32>, String> {
    let bytes = read(path)?;
    let (h, t) = dump::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
    if h.format != pixfmt::FORMAT_BUFFER || !t.len().is_multiple_of(4) {
        return Err(format!("{path}: not a word buffer dump (format 0, a whole number of u32)"));
    }
    Ok(t.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect())
}

/// The kind word `bindings` prints.
fn kind_word(k: PostBindingKind) -> &'static str {
    match k {
        PostBindingKind::Tex2Load => "tex2_load",
        PostBindingKind::Tex2Linear => "tex2_linear",
        PostBindingKind::Tex3Linear => "tex3_linear",
        PostBindingKind::Buf => "buf",
        PostBindingKind::Ubo => "ubo",
        PostBindingKind::Store => "store",
    }
}

fn fetches(row: &PostBinding) -> bool {
    !matches!(row.kind, PostBindingKind::Ubo | PostBindingKind::Store)
}

fn filtered(row: &PostBinding) -> bool {
    matches!(row.kind, PostBindingKind::Tex2Linear | PostBindingKind::Tex3Linear)
}

/// `bindings`: the rows and where their dumps come from.
fn cmd_bindings() {
    for (i, row) in POST_FINAL_BINDINGS.iter().enumerate() {
        let source = match row.dump {
            DumpSource::Resource(name) => format!("resource:{name}"),
            DumpSource::PassInput { pass, .. } => {
                // The k-th FETCHING PassInput row of this pass (the receipt rule PX1a pins).
                let k = POST_FINAL_BINDINGS[..i]
                    .iter()
                    .filter(|r| fetches(r) && matches!(r.dump, DumpSource::PassInput { pass: p, .. } if p == pass))
                    .count();
                format!("pass:{pass}:{k}")
            }
            DumpSource::NoDump => "none".to_owned(),
        };
        println!("{} {} {source}", row.name, kind_word(row.kind));
    }
}

/// The parsed command line of `eval` / `bound`.
struct Args {
    words: [u32; 8],
    binds: Vec<(String, String)>,
    display: Option<String>,
    subtexel_bits: Option<u32>,
    steps: Option<i32>,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut a = Args { words: [0; 8], binds: Vec::new(), display: None, subtexel_bits: None, steps: None };
    let mut words = None;
    let mut it = argv.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().cloned().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--words" => {
                let v = value()?;
                let w: Vec<u32> = v.split(',').map(|s| s.trim().parse().map_err(|e| format!("--words `{s}`: {e}"))).collect::<Result<_, _>>()?;
                words = Some(<[u32; 8]>::try_from(w.as_slice()).map_err(|_| format!("--words needs 8 words, got {}", w.len()))?);
            }
            "--bind" => {
                let v = value()?;
                let (name, path) = v.split_once('=').ok_or_else(|| format!("--bind `{v}` is not <row>=<dump.bin>"))?;
                a.binds.push((name.to_owned(), path.to_owned()));
            }
            "--display" => a.display = Some(value()?),
            "--subtexel-bits" => a.subtexel_bits = Some(value()?.parse().map_err(|e| format!("--subtexel-bits: {e}"))?),
            "--steps" => a.steps = Some(value()?.parse().map_err(|e| format!("--steps: {e}"))?),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    a.words = words.ok_or("--words is required")?;
    Ok(a)
}

/// Loads every fetching row's dump; a row with no `--bind` is a named error.
fn load_rows(binds: &[(String, String)]) -> Result<Vec<(&'static PostBinding, Loaded)>, String> {
    POST_FINAL_BINDINGS
        .iter()
        .filter(|r| fetches(r))
        .map(|row| {
            let path = binds
                .iter()
                .find(|(n, _)| n == row.name)
                .map(|(_, p)| p.as_str())
                .ok_or_else(|| format!("binding `{}` has no dump: pass --bind {}=<dump.bin>", row.name, row.name))?;
            let loaded = match row.kind {
                PostBindingKind::Buf => Loaded::Words(load_words(path)?),
                PostBindingKind::Tex2Load | PostBindingKind::Tex2Linear => {
                    let img = load_image(path)?;
                    if img.layers != 1 {
                        return Err(format!("binding `{}`: a 2D row bound to a {}-layer dump", row.name, img.layers));
                    }
                    Loaded::Image(img)
                }
                PostBindingKind::Tex3Linear => Loaded::Image(load_image(path)?),
                PostBindingKind::Ubo | PostBindingKind::Store => unreachable!("filtered out above"),
            };
            Ok((row, loaded))
        })
        .collect()
}

/// The resolver `eval_res` calls: each row's dump, the `biased` row's texture offset by `bias`.
fn binding<'a>(rows: &'a [(&'static PostBinding, Loaded)], row: &PostBinding, biased: Option<(&str, [f32; 3])>) -> EvalBinding<'a> {
    let (_, loaded) = rows.iter().find(|(r, _)| r.name == row.name).expect("invariant: every fetching row was loaded");
    let bias = biased.filter(|(n, _)| *n == row.name).map(|(_, b)| b);
    match (row.kind, loaded) {
        (PostBindingKind::Buf, Loaded::Words(w)) => EvalBinding::Buf(w),
        (PostBindingKind::Tex3Linear, Loaded::Image(i)) => {
            let t = HostTex3::new(&i.texels, i.w, i.h, i.layers);
            EvalBinding::Tex3(bias.map_or(t, |b| t.with_subtexel_bias(b)))
        }
        (_, Loaded::Image(i)) => {
            let t = HostTex2::new(&i.texels, i.w, i.h);
            EvalBinding::Tex2(bias.map_or(t, |b| t.with_subtexel_bias([b[0], b[1]])))
        }
        (_, Loaded::Words(_)) => unreachable!("a word dump is loaded only for a Buf row"),
    }
}

/// The body at `px` with every row from `rows` (one optionally biased), on the Eval backend.
fn eval_px(px: [u32; 2], words: [u32; 8], rows: &[(&'static PostBinding, Loaded)], biased: Option<(&str, [f32; 3])>) -> [f32; 4] {
    let params = post::eval_params(words);
    let res = post::eval_res(|row| binding(rows, row, biased));
    let cell = core::cell::Cell::new([0.0f32; 4]);
    let _ = post::post_final_main::<EvalCf>(px, &params, &res, &cell);
    cell.get()
}

/// E0's bound on the OETF's `pow`, in display LSB, for a display value `o` (`x = o^(1/g)`).
fn oetf_bound_lsb(o: f32) -> f64 {
    let g = f64::from(OETF_GAMMA_EXP);
    let x = f64::from(o.max(0.0)).powf(1.0 / g);
    if x <= 0.0 {
        return 0.0;
    }
    255.0 * vk_pow_bound(x as f32, OETF_GAMMA_EXP, x.log2(), x.powf(g))
}

/// The filter term of every filtered row at `px`: the largest display change, in LSB, over the
/// sub-texel corners `±2^-bits` texel per axis.
fn filter_term(px: [u32; 2], a: &Args, rows: &[(&'static PostBinding, Loaded)], base: [f32; 4]) -> f64 {
    let Some(bits) = a.subtexel_bits else { return 0.0 };
    let e = 2f32.powi(-(bits as i32));
    let mut total = 0.0;
    for (row, _) in rows.iter().filter(|(r, _)| filtered(r)) {
        let dims = if row.kind == PostBindingKind::Tex3Linear { 3 } else { 2 };
        let mut worst = 0.0f64;
        for corner in 0..(1u32 << dims) {
            let b: [f32; 3] = core::array::from_fn(|k| {
                if k >= dims {
                    0.0
                } else if ((corner >> k) & 1) == 1 {
                    e
                } else {
                    -e
                }
            });
            let v = eval_px(px, a.words, rows, Some((row.name, b)));
            for c in 0..3 {
                worst = worst.max(255.0 * f64::from((v[c] - base[c]).abs()));
            }
        }
        total += worst;
    }
    total
}

/// `eval`: the display against the body; returns (pixels, pixels over their bound, worst excess).
fn cmd_eval(a: &Args) -> Result<(usize, usize, f64), String> {
    let rows = load_rows(&a.binds)?;
    if a.subtexel_bits.is_none() && rows.iter().any(|(r, _)| filtered(r)) {
        return Err("a filtered row is bound: --subtexel-bits is required (the device's subTexelPrecisionBits)".to_owned());
    }
    let display = load_image(a.display.as_deref().ok_or("--display is required")?)?;
    let mut over = 0;
    let mut worst = 0.0f64;
    for y in 0..display.h {
        for x in 0..display.w {
            let host = eval_px([x, y], a.words, &rows, None);
            let dev = display.texels[(y * display.w + x) as usize];
            let mut bad = false;
            for c in 0..3 {
                let d = 255.0 * f64::from((dev[c] - host[c]).abs());
                let bound = 1.0 + oetf_bound_lsb(host[c]) + filter_term([x, y], a, &rows, host);
                if d > bound {
                    bad = true;
                    worst = worst.max(d - bound);
                }
            }
            over += usize::from(bad);
        }
    }
    Ok(((display.w * display.h) as usize, over, worst))
}

/// The scene input's channel encodings (its dump format).
fn scene_channels(format: u32) -> Result<[Channel; 3], String> {
    match format {
        pixfmt::VK_FORMAT_B10G11R11_UFLOAT_PACK32 => Ok(pixfmt::B10G11R11_CHANNELS),
        pixfmt::VK_FORMAT_R16G16B16A16_SFLOAT => Ok([Channel::F16; 3]),
        pixfmt::VK_FORMAT_R8G8B8A8_UNORM | pixfmt::VK_FORMAT_B8G8R8A8_UNORM => Ok([Channel::Unorm8; 3]),
        f => Err(format!("the scene input's VkFormat {f} has no step encoding")),
    }
}

/// The scene row's texels, mutably (perturbed in place and restored by `cmd_bound`).
fn scene_texels<'r>(rows: &'r mut [(&'static PostBinding, Loaded)], at: usize) -> &'r mut Vec<[f32; 4]> {
    match &mut rows[at].1 {
        Loaded::Image(i) => &mut i.texels,
        Loaded::Words(_) => unreachable!("gScene is a texture row"),
    }
}

/// `bound`: the joint forward difference at ±`steps` format steps, maximised over pixels.
///
/// The footprint's texels are perturbed IN PLACE and restored after each evaluation, so the cost
/// is one body evaluation per pattern, never a texture copy.
fn cmd_bound(a: &Args) -> Result<f64, String> {
    let steps = a.steps.ok_or("--steps is required")?;
    let mut rows = load_rows(&a.binds)?;
    let at = rows.iter().position(|(r, _)| r.name == "gScene").expect("invariant: the gScene row is a fetching row");
    let (w, h, chans) = match &rows[at].1 {
        Loaded::Image(i) => (i.w, i.h, scene_channels(i.format)?),
        Loaded::Words(_) => unreachable!("gScene is a texture row"),
    };
    let r = i64::from(POST_FINAL_TAP_RADIUS);
    // Radius 0: the footprint IS the centre, so "centre opposite" equals "same sign" with every
    // sign flipped — already among the eight patterns.
    let configs: &[bool] = if r == 0 { &[false] } else { &[false, true] };
    let mut worst = 0.0f64;
    for y in 0..h {
        for x in 0..w {
            let centre = (y * w + x) as usize;
            let c0 = scene_texels(&mut rows, at)[centre];
            if pixfmt::pixel_is_nonfinite([c0[0], c0[1], c0[2]]) {
                continue;
            }
            let base = eval_px([x, y], a.words, &rows, None);
            let mut foot: Vec<(usize, bool)> = Vec::new();
            for dy in -r..=r {
                for dx in -r..=r {
                    let (tx, ty) = (i64::from(x) + dx, i64::from(y) + dy);
                    if tx >= 0 && ty >= 0 && tx < i64::from(w) && ty < i64::from(h) {
                        foot.push(((ty as u32 * w + tx as u32) as usize, dx == 0 && dy == 0));
                    }
                }
            }
            let saved: Vec<[f32; 4]> = foot.iter().map(|(i, _)| scene_texels(&mut rows, at)[*i]).collect();
            for pattern in 0..8u32 {
                for &centre_opposite in configs {
                    for ((i, is_centre), orig) in foot.iter().zip(&saved) {
                        let mut t = *orig;
                        for c in 0..3 {
                            let up = ((pattern >> c) & 1) == 1;
                            let sign = if up != (centre_opposite && *is_centre) { 1 } else { -1 };
                            if let Ok(v) = pixfmt::step_by(chans[c], orig[c], sign * steps) {
                                t[c] = v;
                            }
                        }
                        scene_texels(&mut rows, at)[*i] = t;
                    }
                    let v = eval_px([x, y], a.words, &rows, None);
                    for c in 0..3 {
                        worst = worst.max(255.0 * f64::from((v[c] - base[c]).abs()));
                    }
                    for ((i, _), orig) in foot.iter().zip(&saved) {
                        scene_texels(&mut rows, at)[*i] = *orig;
                    }
                }
            }
        }
    }
    Ok(worst)
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first() else {
        eprintln!("post_eval: usage: post_eval bindings | eval ... | bound ...");
        return ExitCode::from(2);
    };
    match cmd.as_str() {
        "bindings" => {
            cmd_bindings();
            ExitCode::SUCCESS
        }
        "eval" => match parse_args(&argv[1..]).and_then(|a| cmd_eval(&a)) {
            Ok((n, over, worst)) => {
                println!("post_eval eval: {n} pixels, {over} over their bound (failing_frac={:.6}), worst excess {worst:.3} LSB", over as f64 / n.max(1) as f64);
                if over == 0 { ExitCode::SUCCESS } else { ExitCode::from(1) }
            }
            Err(e) => {
                eprintln!("post_eval eval: {e}");
                ExitCode::from(2)
            }
        },
        "bound" => match parse_args(&argv[1..]).and_then(|a| cmd_bound(&a)) {
            Ok(b) => {
                println!("max_delta_bound={b:.6}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("post_eval bound: {e}");
                ExitCode::from(2)
            }
        },
        other => {
            eprintln!("post_eval: unknown command `{other}`");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boyko_shaderdsl::cf::Cf;

    /// A test-only traced body ending in `tex2_sample_linear` at the pixel's centre shifted by a
    /// third of a texel — the shape of the bloom composite's tap (PX4) that the filter term covers.
    fn sampled(t: HostTex2<'_>, px: [u32; 2]) -> [f32; 4] {
        let uv = [(px[0] as f32 + 0.5 + 1.0 / 3.0) / t.width() as f32, (px[1] as f32 + 0.5) / t.height() as f32];
        <EvalCf as Cf>::tex2_sample_linear(t, uv)
    }

    /// The filter term in TEXEL units on an axis WIDER than 2^bits (`px0/critique.md` W4): a
    /// display whose tap moved by one texel exceeds the ±2^-8-texel bound, the exact display is
    /// within it. A UV-unit bias (×1024 here) would admit the one-texel shift.
    #[test]
    fn the_filter_term_admits_sub_texel_and_refuses_a_one_texel_shift() {
        let (w, h) = (1024u32, 2u32);
        let texels: Vec<[f32; 4]> = (0..w * h).map(|i| [(i % w) as f32 / w as f32, 0.0, 0.0, 1.0]).collect();
        let t = HostTex2::new(&texels, w, h);
        let px = [500u32, 0];
        let exact = sampled(t, px);
        let e = 2f32.powi(-8);
        let bound: f32 = [-e, e]
            .iter()
            .map(|b| (sampled(t.with_subtexel_bias([*b, 0.0]), px)[0] - exact[0]).abs())
            .fold(0.0, f32::max);
        let shifted = sampled(t, [px[0] + 1, px[1]]);
        assert!((shifted[0] - exact[0]).abs() > bound, "a one-texel shift must exceed the sub-texel bound");
        assert!(bound > 0.0 && bound < 1.0 / w as f32, "the bound is sub-texel: {bound}");
        // The wrong unit, for contrast: the same bias applied in UV units moves the tap by 4 texels.
        let uv_unit = (sampled(t.with_subtexel_bias([e * w as f32, 0.0]), px)[0] - exact[0]).abs();
        assert!(uv_unit >= (shifted[0] - exact[0]).abs(), "a UV-unit bias admits the one-texel shift");
    }

    /// A zero bias is the unbiased path, bit for bit.
    #[test]
    fn a_zero_bias_is_bit_identical() {
        let texels: Vec<[f32; 4]> = (0..64).map(|i| [i as f32 * 0.37, -0.0, 1.0, 0.5]).collect();
        let t = HostTex2::new(&texels, 8, 8);
        for i in 0..200 {
            let uv = [i as f32 / 199.0 - 0.01, (i * 7 % 199) as f32 / 199.0];
            let a = <EvalCf as Cf>::tex2_sample_linear(t, uv);
            let b = <EvalCf as Cf>::tex2_sample_linear(t.with_subtexel_bias([0.0, 0.0]), uv);
            assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
        }
    }

    /// `bindings` resolves the PassInput row to fetching row 0 of `post_final`.
    #[test]
    fn the_pass_input_row_is_fetching_row_zero() {
        let row = &POST_FINAL_BINDINGS[0];
        assert_eq!(row.name, "gScene");
        assert!(fetches(row) && !filtered(row));
    }

    /// A row of each kind `POST_FINAL_BINDINGS` has none of yet (PX2+ adds them): only a synthetic
    /// row reaches `binding`'s `Buf` and `Tex3` arms before then.
    const BUF_ROW: PostBinding = PostBinding {
        name: "tWords",
        kind: PostBindingKind::Buf,
        set: 0,
        binding: 3,
        image_format: None,
        dump: DumpSource::Resource("t_words"),
    };
    const TEX3_ROW: PostBinding = PostBinding {
        name: "tVolume",
        kind: PostBindingKind::Tex3Linear,
        set: 0,
        binding: 4,
        image_format: None,
        dump: DumpSource::Resource("t_volume"),
    };

    /// Writes an OBSD dump (header + texel bytes, the recorder's layout) to a scratch file.
    fn scratch_dump(name: &str, header: dump::DumpHeader, texels: &[u8]) -> String {
        let p = std::env::temp_dir().join(format!("boyko_post_eval_unit_{}_{name}.bin", std::process::id()));
        let mut bytes = header.to_bytes().to_vec();
        bytes.extend_from_slice(texels);
        std::fs::write(&p, bytes).expect("the temp dir is writable");
        p.display().to_string()
    }

    /// **The `Buf` and `Tex3` resolver arms on synthetic dumps** (`px0/cut.md` PX0-8; triage r1 W2):
    /// a word-buffer dump and a 2×2×2 volume dump, written as the recorder writes them, load
    /// through `load_words` / `load_image` (each refusing the other's dump), and `binding`
    /// resolves them to `EvalBinding::Buf` (the words, in order) and `EvalBinding::Tex3` (the
    /// volume; the sub-texel bias applied to the named row only).
    #[test]
    fn the_buf_and_tex3_resolver_arms_on_synthetic_dumps() {
        let words: [u32; 5] = [7, 0xDEAD_BEEF, 0, 1, u32::MAX];
        let wbytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let buf = dump::DumpHeader {
            format: pixfmt::FORMAT_BUFFER,
            width: wbytes.len() as u32,
            height: 1,
            mip: 0,
            layers: 1,
            frame: 0,
            texel_bytes: 1,
        };
        let buf_path = scratch_dump("words", buf, &wbytes);
        // Texel (x, y, z) = [x + 2y + 4z, 0.5, 0.25 z, 1]: exact in f32, distinct per texel.
        let vol: Vec<[f32; 4]> = (0..8u32).map(|i| [i as f32, 0.5, 0.25 * (i / 4) as f32, 1.0]).collect();
        let vbytes: Vec<u8> = vol.iter().flat_map(|t| t.iter().flat_map(|c| c.to_le_bytes())).collect();
        let vh = dump::DumpHeader {
            format: pixfmt::VK_FORMAT_R32G32B32A32_SFLOAT,
            width: 2,
            height: 2,
            mip: 0,
            layers: 2,
            frame: 0,
            texel_bytes: 16,
        };
        let vol_path = scratch_dump("volume", vh, &vbytes);

        assert!(load_words(&vol_path).err().is_some_and(|e| e.contains("not a word buffer dump")));
        assert!(load_image(&buf_path).err().is_some_and(|e| e.contains("a buffer dump where an image was expected")));
        let rows = vec![
            (&BUF_ROW, Loaded::Words(load_words(&buf_path).expect("the word dump loads"))),
            (&TEX3_ROW, Loaded::Image(load_image(&vol_path).expect("the volume dump loads"))),
        ];
        let _ = std::fs::remove_file(&buf_path);
        let _ = std::fs::remove_file(&vol_path);

        match binding(&rows, &BUF_ROW, None) {
            EvalBinding::Buf(w) => assert_eq!(w, &words[..]),
            other => panic!("a Buf row resolved to {other:?}"),
        }
        // A texel centre returns that texel; half a texel of bias on x lands midway to the next one.
        let centre = |i: u32| [(0.5 + (i % 2) as f32) / 2.0, (0.5 + (i / 2 % 2) as f32) / 2.0, (0.5 + (i / 4) as f32) / 2.0];
        let sample = |b: EvalBinding<'_>, uvw: [f32; 3]| match b {
            EvalBinding::Tex3(t) => t.sample_linear(uvw),
            other => panic!("a Tex3Linear row resolved to {other:?}"),
        };
        for i in 0..8 {
            assert_eq!(sample(binding(&rows, &TEX3_ROW, None), centre(i)), vol[i as usize], "texel {i}");
        }
        let half = sample(binding(&rows, &TEX3_ROW, Some(("tVolume", [0.5, 0.0, 0.0]))), centre(4));
        assert_eq!(half, [4.5, 0.5, 0.25, 1.0], "the bias moves the named row's taps");
        let other = sample(binding(&rows, &TEX3_ROW, Some(("gScene", [0.5, 0.0, 0.0]))), centre(4));
        assert_eq!(other, vol[4], "a bias naming another row leaves this one unbiased");
    }

    /// The argument parser refuses a short word list and a malformed bind, by name.
    #[test]
    fn the_parser_names_its_errors() {
        let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        assert!(parse_args(&s(&["--words", "1,2,3"])).is_err());
        assert!(parse_args(&s(&["--words", "0,0,0,0,0,0,0,0", "--bind", "gScene"])).is_err());
        assert!(parse_args(&s(&["--bind", "gScene=x"])).err().is_some_and(|e| e.contains("--words")));
        assert!(load_rows(&[]).err().is_some_and(|e| e.contains("gScene")), "a missing binding is a named error");
    }
}
