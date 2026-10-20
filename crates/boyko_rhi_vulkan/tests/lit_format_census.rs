//! Render wave PX1a test 9 (`plan/H4-PX1a-hdr-lit.md` Tests 9, Scope 5 "one spelling of the HDR
//! storage format"; `px1a/cut.md` D-S1, §1.3 DS-H4 (a) and (f); orchestrator ruling 67 W6): **the
//! HDR scene colour's format is spelled once per language, and its encodings are decoded in one
//! crate.**
//!
//! 1. **Rust** — `present/targets.rs` holds exactly one `pub const LIT_FORMAT: Format = Format::<X>;`
//!    and `<X>` is PX0's decision (`R16G16B16A16Sfloat`, `docs/measurements/2026-10-16-px0-lit-format-decision.md`).
//! 2. **Generators** — `boyko_shaderdsl::post::LIT_STORAGE_FORMAT` is the storage qualifier of
//!    `LIT_FORMAT` ([`QUALIFIERS`]).
//! 3. **HLSL** — `shaders/lit_format.hlsli` holds exactly one
//!    `#define LIT_FORMAT_QUALIFIER "<q>"` with `<q>` the same qualifier, and every `gLit` storage
//!    declaration under `shaders/` names the macro (`[[vk::image_format(LIT_FORMAT_QUALIFIER)]]`),
//!    never a literal.
//! 4. **The decoder census (DS-H4 (a): decode through `boyko_shaderdsl::{pixfmt, dump}` only).** No
//!    Rust function outside `crates/boyko_shaderdsl/` hand-decodes binary16 or the packed unsigned
//!    floats, except the residuals PX0 named ([`RESIDUAL_DECODERS`], each with its reason). The
//!    matcher, specified (ruling 67 W6), runs on each `fn` item with comments AND string / char
//!    literals blanked:
//!    - **the fn returns a float** — its signature's return type names `f32` or `f64` (a decoder
//!      produces a float; an encoder returns bits and is out of scope); AND one of
//!    - **(name)** its name holds an encoding token (`f16`, `half`, `binary16`, `ufloat`,
//!      `b10g11r11`, `r11g11b10`) AND a decode token (`unpack`, `decode`, `to_f32`, `to_f64`,
//!      `bits_to_f`);
//!    - **(mask)** its body holds binary16's mantissa mask `0x3ff` (any case / leading zeros /
//!      `_` separators) together with the exponent shift `>> 10`;
//!    - **(exponent)** its body holds binary16's exponent mask `0x7c00`.
//!
//!    A fn whose body calls into `boyko_shaderdsl::` is a delegate, not a hand decoder. Each
//!    allow-listed residual must still exist (the list shrinks with the tree), and the matcher is
//!    run against planted fixtures every run, so it cannot be vacuous.
//! 5. **The pipeline census (orchestrator ruling 67 W2: close the class).** Every
//!    `GraphicsPipelineDesc` the production render crates (`boyko_app/src`, `boyko_render/src`)
//!    build is CLASSIFIED by `(file, fragment module)` in [`PIPELINE_TARGETS`] — the pipeline's
//!    colour attachment is `lit`, `lit_native` or something else (stated). Every `lit` /
//!    `lit_native` pipeline's `color_formats` is exactly `[LIT_FORMAT]`: a pipeline drawing into the
//!    HDR scene colour under any other declared format is a dynamic-rendering format mismatch
//!    (`VUID-vkCmdDraw-dynamicRenderingUnusedAttachments-08910`) that only a validation-ON worker on
//!    that exact configuration would see. An unclassified pipeline is RED — a new pipeline is
//!    classified in the commit that adds it, so the class cannot grow a seventh unchecked site.

use std::fs;
use std::path::{Path, PathBuf};

/// The `boyko_rhi::Format` variant ↔ HLSL storage-qualifier pairs the census knows.
const QUALIFIERS: &[(&str, &str)] =
    &[("R16G16B16A16Sfloat", "rgba16f"), ("B10G11R11UfloatPack32", "r11g11b10f"), ("R8G8B8A8Unorm", "rgba8")];

/// PX0's lit-format decision (row 2: RGBA16F).
const DECIDED_LIT_FORMAT: &str = "R16G16B16A16Sfloat";

/// The hand decoders PX0 named and left in place (`px0/cut.md` O2): `(path suffix, fn name, reason)`.
const RESIDUAL_DECODERS: &[(&str, &str, &str)] = &[
    (
        "crates/boyko_rhi_vulkan/tests/ddgi_probe_gi_resolve.rs",
        "unpack_b10g11r11",
        "a test-local truncating model of the DDGI irradiance atlas texel (not `lit`); exact-representable inputs only",
    ),
    (
        "crates/boyko_rhi_vulkan/tests/ddgi_probe_gi_resolve.rs",
        "unpack_ufloat",
        "the per-channel half of the DDGI atlas model above",
    ),
    (
        "crates/boyko_rhi_vulkan/src/goldens.rs",
        "golden_f16_to_f32",
        "the host oracles' binary16 decode of a device readback; PX0 residual, not in `post_eval`'s chain",
    ),
    (
        "crates/boyko_render/src/light.rs",
        "half_to_f32",
        "a unit-test decode of the light table's packed f16 cone pair (`boyko_render` has no `boyko_shaderdsl` edge)",
    ),
    (
        "crates/boyko_render/src/particle_effect.rs",
        "f16_bits_to_f32",
        "a unit-test decode of the particle effect table's f16 ramps (`boyko_render` has no `boyko_shaderdsl` edge)",
    ),
];

const NAME_ENCODING_TOKENS: &[&str] = &["f16", "half", "binary16", "ufloat", "b10g11r11", "r11g11b10"];
const NAME_DECODE_TOKENS: &[&str] = &["unpack", "decode", "to_f32", "to_f64", "bits_to_f"];

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workspace_root() -> PathBuf {
    crate_dir().join("..").join("..")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("invariant: {} is readable UTF-8: {e}", path.display()))
}

fn qualifier_of(variant: &str) -> Option<&'static str> {
    QUALIFIERS.iter().find(|&&(v, _)| v == variant).map(|&(_, q)| q)
}

/// The `Format::<X>` of every `pub const LIT_FORMAT: Format = Format::<X>;` line in `text`.
fn lit_format_consts(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("pub const LIT_FORMAT: Format = Format::")?;
            Some(rest.strip_suffix(';')?.trim().to_string())
        })
        .collect()
}

/// The string of every `#define LIT_FORMAT_QUALIFIER "<q>"` line in `text`.
fn qualifier_macros(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("#define LIT_FORMAT_QUALIFIER")?.trim();
            Some(rest.strip_prefix('"')?.strip_suffix('"')?.to_string())
        })
        .collect()
}

/// `(line, format argument)` of every `RWTexture2D` declaration named `gLit` in `text`
/// (`//` comments ignored).
fn glit_storage_decls(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .filter_map(|(k, line)| {
            let code = line.split("//").next().unwrap_or("");
            let after_ty = code.split("RWTexture2D").nth(1)?;
            let name_ok = after_ty
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .any(|tok| tok == "gLit");
            if !name_ok {
                return None;
            }
            let fmt = code
                .split("vk::image_format(")
                .nth(1)
                .and_then(|r| r.split(")]]").next())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "<no image_format>".to_string());
            Some((k + 1, fmt))
        })
        .collect()
}

#[test]
fn lit_format_is_one_rust_const_naming_px0s_decision() {
    let targets = read(&crate_dir().join("src/present/targets.rs"));
    let consts = lit_format_consts(&targets);
    assert_eq!(consts.len(), 1, "present/targets.rs holds {} `pub const LIT_FORMAT` lines: no LIT_FORMAT", consts.len());
    assert_eq!(consts[0], DECIDED_LIT_FORMAT, "LIT_FORMAT is not PX0's decision");
}

#[test]
fn the_generator_qualifier_and_the_hlsl_macro_match_lit_format() {
    let q = qualifier_of(DECIDED_LIT_FORMAT).expect("invariant: the decided format has a qualifier row");
    assert_eq!(boyko_shaderdsl::post::LIT_STORAGE_FORMAT, q, "post::LIT_STORAGE_FORMAT is not LIT_FORMAT's qualifier");
    let hlsli = crate_dir().join("shaders/lit_format.hlsli");
    assert!(hlsli.is_file(), "no shaders/lit_format.hlsli: the HLSL side of the format has no single spelling");
    let macros = qualifier_macros(&read(&hlsli));
    assert_eq!(macros, [q.to_string()], "lit_format.hlsli's LIT_FORMAT_QUALIFIER");
}

#[test]
fn every_glit_storage_declaration_names_the_macro() {
    let dir = crate_dir().join("shaders");
    let mut names: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("invariant: {} is readable: {e}", dir.display()))
        .map(|e| e.expect("invariant: directory entry is readable").path())
        .filter(|p| p.extension().is_some_and(|x| x == "hlsl" || x == "hlsli"))
        .collect();
    names.sort();
    let mut decls = 0usize;
    let mut offenders = Vec::new();
    for p in &names {
        for (line, fmt) in glit_storage_decls(&read(p)) {
            decls += 1;
            if fmt != "LIT_FORMAT_QUALIFIER" {
                offenders.push(format!(
                    "{}:{line} declares gLit with image_format({fmt})",
                    p.file_name().expect("invariant: a file has a name").to_string_lossy()
                ));
            }
        }
    }
    assert!(decls >= 6, "the census found only {decls} gLit storage declarations — wrong directory?");
    assert!(offenders.is_empty(), "{} gLit declaration(s) spell the format literally: {offenders:#?}", offenders.len());
}

// ---- The decoder census ----------------------------------------------------------------------

/// `text` with comments, string literals and char literals replaced by spaces (newlines kept).
fn blank_comments_and_literals(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for c in &mut out[from..to] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            let end = text[i..].find('\n').map_or(b.len(), |k| i + k);
            blank(&mut out, i, end);
            i = end;
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let end = text[i + 2..].find("*/").map_or(b.len(), |k| i + 2 + k + 2);
            blank(&mut out, i, end);
            i = end;
        } else if let Some(hashes) = raw_string_hashes(b, i) {
            let close: String = std::iter::once('"').chain(std::iter::repeat_n('#', hashes)).collect();
            let start = i + 2 + hashes;
            let end = text[start..].find(&close).map_or(b.len(), |k| start + k + close.len());
            blank(&mut out, i, end);
            i = end;
        } else if c == b'"' {
            let mut j = i + 1;
            while j < b.len() && b[j] != b'"' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let end = (j + 1).min(b.len());
            blank(&mut out, i, end);
            i = end;
        } else if c == b'\'' {
            // A char literal (`'x'`, `'\n'`, `'\u{..}'`); a lifetime (`'a`) is left alone.
            let end = if b.get(i + 1) == Some(&b'\\') {
                text[i + 2..].find('\'').map(|k| i + 2 + k + 1).filter(|&e| e - i <= 12)
            } else {
                let ch_len = text[i + 1..].chars().next().map_or(0, char::len_utf8);
                (b.get(i + 1 + ch_len) == Some(&b'\'')).then_some(i + 2 + ch_len)
            };
            match end {
                Some(e) => {
                    blank(&mut out, i, e);
                    i = e;
                }
                None => i += 1,
            }
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).expect("invariant: only ASCII bytes were blanked")
}

/// `Some(#count)` if a raw string literal (`r"…"`, `r#"…"#`) opens at `i`.
fn raw_string_hashes(b: &[u8], i: usize) -> Option<usize> {
    if b[i] != b'r' || (i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')) {
        return None;
    }
    let hashes = b[i + 1..].iter().take_while(|&&h| h == b'#').count();
    (b.get(i + 1 + hashes) == Some(&b'"')).then_some(hashes)
}

/// The hand decoders in one Rust source: `(line, fn name)`.
fn hand_decoders(text: &str) -> Vec<(usize, String)> {
    let src = blank_comments_and_literals(text);
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(k) = src[from..].find("fn ") {
        let at = from + k;
        from = at + 3;
        if at > 0 && (b[at - 1].is_ascii_alphanumeric() || b[at - 1] == b'_') {
            continue;
        }
        let name: String =
            src[at + 3..].trim_start().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if name.is_empty() {
            continue;
        }
        // The signature runs to the first `{` or `;` outside brackets (`-> [f32; 3]` holds a `;`).
        let mut j = at + 3;
        let mut depth = 0i32;
        while j < b.len() {
            match b[j] {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                b'{' | b';' if depth == 0 => break,
                _ => {}
            }
            j += 1;
        }
        if j >= b.len() || b[j] == b';' {
            continue;
        }
        let sig = &src[at..j];
        let mut depth = 0usize;
        let mut end = j;
        for (k, &c) in b.iter().enumerate().skip(j) {
            if c == b'{' {
                depth += 1;
            } else if c == b'}' {
                depth -= 1;
                if depth == 0 {
                    end = k;
                    break;
                }
            }
        }
        let body_raw = &src[j..=end];
        if is_hand_decoder(&name, sig, body_raw) {
            out.push((src[..at].matches('\n').count() + 1, name));
        }
    }
    out
}

/// The matcher (module doc, rule 4).
fn is_hand_decoder(name: &str, sig: &str, body: &str) -> bool {
    let ret = sig.split("->").nth(1).unwrap_or("");
    if !(ret.contains("f32") || ret.contains("f64")) || body.contains("boyko_shaderdsl::") {
        return false;
    }
    let lname = name.to_ascii_lowercase();
    let by_name = NAME_ENCODING_TOKENS.iter().any(|t| lname.contains(t)) && NAME_DECODE_TOKENS.iter().any(|t| lname.contains(t));
    let norm: String = body.to_ascii_lowercase().chars().filter(|&c| c != '_').collect();
    let hexes: Vec<u64> = hex_literals(&norm);
    let by_mask = hexes.contains(&0x3ff) && shifts_by(&norm, 10);
    let by_exponent = hexes.contains(&0x7c00);
    by_name || by_mask || by_exponent
}

/// The values of the `0x…` literals in `norm` (lowercase, `_` removed).
fn hex_literals(norm: &str) -> Vec<u64> {
    let mut out = Vec::new();
    let b = norm.as_bytes();
    let mut i = 0;
    while let Some(k) = norm[i..].find("0x") {
        let at = i + k;
        let digits: String = norm[at + 2..].chars().take_while(char::is_ascii_hexdigit).collect();
        let preceded = at > 0 && (b[at - 1].is_ascii_alphanumeric());
        if !digits.is_empty()
            && !preceded
            && let Ok(v) = u64::from_str_radix(&digits, 16)
        {
            out.push(v);
        }
        i = at + 2;
    }
    out
}

/// `true` if `norm` holds `>> n` (any spacing) with `n` not followed by another digit.
fn shifts_by(norm: &str, n: u32) -> bool {
    let want = n.to_string();
    norm.match_indices(">>").any(|(at, _)| {
        let rest = norm[at + 2..].trim_start().trim_start_matches('=').trim_start();
        rest.starts_with(&want) && !rest[want.len()..].starts_with(|c: char| c.is_ascii_digit())
    })
}

/// Every `.rs` file under `dir`, skipping build output and `boyko_shaderdsl` (the decoders' home).
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.map(|e| e.expect("invariant: directory entry is readable").path()).collect();
    entries.sort();
    for p in entries {
        let fname = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if p.is_dir() {
            if matches!(fname.as_str(), "target" | ".git" | "boyko_shaderdsl" | "node_modules") {
                continue;
            }
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

#[test]
fn no_rust_fn_outside_boyko_shaderdsl_hand_decodes_a_pixel_encoding() {
    let root = workspace_root().canonicalize().expect("invariant: the workspace root exists");
    let mut files = Vec::new();
    for top in ["crates", "src", "tests", "benches", "examples"] {
        rust_files(&root.join(top), &mut files);
    }
    assert!(files.len() > 500, "the decoder census read only {} Rust files — wrong root?", files.len());
    let mut found = Vec::new();
    for p in &files {
        let r = rel(&root, p);
        for (line, name) in hand_decoders(&read(p)) {
            found.push((r.clone(), line, name));
        }
    }
    let offenders: Vec<String> = found
        .iter()
        .filter(|(r, _, n)| !RESIDUAL_DECODERS.iter().any(|&(f, rn, _)| f == r && rn == n))
        .map(|(r, l, n)| format!("{r}:{l} fn {n} hand-decodes a pixel encoding — use boyko_shaderdsl::pixfmt"))
        .collect();
    assert!(offenders.is_empty(), "{} hand decoder(s) outside boyko_shaderdsl: {offenders:#?}", offenders.len());
    let stale: Vec<String> = RESIDUAL_DECODERS
        .iter()
        .filter(|&&(f, n, _)| !found.iter().any(|(r, _, fname)| r == f && fname == n))
        .map(|&(f, n, _)| format!("{f}::{n}"))
        .collect();
    assert!(stale.is_empty(), "allow-listed residual decoder(s) no longer found — shrink the list: {stale:?}");
}

/// The matcher against fixtures it must flag and fixtures it must pass — every run.
#[test]
fn the_decoder_matcher_rejects_its_fixtures() {
    let planted = r#"
/// fn not_a_decoder() -> f32 { 0x7c00 }
fn decode_texel_by_hand(h: u16) -> f32 {
    let e = (h >> 10) & 0x1F;
    let m = h & 0x03_FF;
    if e == 0 { return (m as f32) * 2f32.powi(-24); }
    f32::from_bits(((e as u32 + 112) << 23) | ((m as u32) << 13))
}
fn exp_masked(h: u16) -> f64 { f64::from(h & 0x7C00) }
fn unpack_ufloat_channel(p: u32, mant: u32) -> f32 { f32::from_bits(p << mant) }
fn half_to_f32(h: u16) -> f32 { let _ = h; 0.0 }
"#;
    let names: Vec<String> = hand_decoders(planted).into_iter().map(|(_, n)| n).collect();
    assert_eq!(names, ["decode_texel_by_hand", "exp_masked", "unpack_ufloat_channel", "half_to_f32"]);

    let clean = r#"
fn f32_to_f16_bits(v: f32) -> u16 { ((v.to_bits() >> 13) & 0x3ff) as u16 | 0x7c00 }
fn bit_of(idx: u32) -> u32 { (idx >> 5) & 0x1f }
fn delegate(h: u16) -> f32 { boyko_shaderdsl::half::f16_bits_to_f32(h) }
fn pack_unpack_round_trips_within_f16_precision() { let _ = 0x3ff >> 10; }
fn text() -> &'static str { "fn fake_decode_f16(h: u16) -> f32 { 0x7c00 }" }
fn lifetimes<'a>(s: &'a str) -> &'a str { s }
fn char_lit() -> char { '"' }
fn after_char() -> f32 { 0.0 }
"#;
    assert!(hand_decoders(clean).is_empty(), "{:?}", hand_decoders(clean));
}

// ---- 5. The pipeline census (ruling 67 W2) ---------------------------------------------------

/// What a production graphics pipeline draws into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    /// The HDR scene colour `lit` — its `color_formats` must be `[LIT_FORMAT]`.
    Lit,
    /// `lit_native`, the SSAA downsample's output in `lit`'s format — `[LIT_FORMAT]` too.
    LitNative,
    /// Another attachment set (stated): not this census's class.
    Other(&'static str),
}

/// Every production `GraphicsPipelineDesc`, by `(path suffix, fragment-module identifier)`.
const PIPELINE_TARGETS: &[(&str, &str, Target)] = &[
    ("boyko_app/src/gpu_scene/mod.rs", "fs", Target::Other("the G-buffer raster's three MRTs (albedo / normal / material)")),
    ("boyko_app/src/gpu_scene/mod.rs", "pm_fs", Target::Other("the per-instance-material G-buffer raster's three MRTs")),
    ("boyko_app/src/gpu_scene/mod.rs", "sample_fs", Target::Other("the present blit: the swapchain")),
    ("boyko_app/src/gpu_scene/mod.rs", "fxaa_fs", Target::Other("FXAA: `aa_out` (display-referred RGBA8)")),
    ("boyko_app/src/gpu_scene/mod.rs", "smaa_edge_fs", Target::Other("SMAA edges: `smaa_edges` (RG8)")),
    ("boyko_app/src/gpu_scene/mod.rs", "smaa_weight_fs", Target::Other("SMAA weights: `smaa_weights` (RGBA8)")),
    ("boyko_app/src/gpu_scene/mod.rs", "smaa_blend_fs", Target::Other("SMAA blend: `aa_out` (display-referred RGBA8)")),
    ("boyko_app/src/gpu_scene/mod.rs", "ssaa_fs", Target::LitNative),
    ("boyko_app/src/gpu_scene/mod.rs", "forward_fs", Target::Lit),
    ("boyko_app/src/gpu_scene/mod.rs", "sky_fs", Target::Lit),
    ("boyko_app/src/gpu_scene/mod.rs", "prepass_fs", Target::Other("the ForwardPlus depth prepass: depth only")),
    ("boyko_app/src/gpu_scene/mod.rs", "forward_plus_fs", Target::Lit),
    ("boyko_app/src/gpu_scene/mod.rs", "mv_fs", Target::Other("the motion-vector G-buffer raster: the MRTs + the MV target")),
    ("boyko_app/src/gpu_scene/mod.rs", "mvpm_fs", Target::Other("the per-instance-material MV G-buffer raster")),
    ("boyko_app/src/gpu_scene/mod.rs", "vb_raster_fs", Target::Other("the visibility-buffer raster: `vb_id` (RG32UI)")),
    ("boyko_app/src/gpu_scene/mod.rs", "vb_sky_fs", Target::Lit),
    ("boyko_app/src/gpu_scene/mod.rs", "tex_fs", Target::Other("the textured G-buffer raster's MRTs")),
    ("boyko_app/src/gpu_scene/csm.rs", "depth_fs", Target::Other("the CSM cascades: depth only")),
    ("boyko_app/src/gpu_scene/csm.rs", "point_depth_fs", Target::Other("the punctual atlas: depth only")),
    ("boyko_app/src/gpu_scene/sdf_depth_merge.rs", "fs", Target::Other("the SDF depth merge: depth only")),
    ("boyko_app/src/gpu_scene/particle.rs", "draw_fs", Target::Lit),
    ("boyko_render/src/ui/resources.rs", "fragment_module", Target::Other("the UI pass: the UI / swapchain colour target")),
];

/// One `GraphicsPipelineDesc { .. }` literal: where, its fragment module, its `color_formats` list.
#[derive(Debug)]
struct PipelineSite {
    file: String,
    line: usize,
    fragment: String,
    color_formats: Vec<String>,
}

/// The index just past the bracket matching the `open` at `at` (`text[at] == open`).
fn matching(text: &[u8], at: usize, open: u8, close: u8) -> usize {
    let mut depth = 0usize;
    for (i, &c) in text.iter().enumerate().skip(at) {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return i + 1;
            }
        }
    }
    text.len()
}

/// Every `GraphicsPipelineDesc {` literal in `text` (comments and literals blanked first).
fn pipeline_sites(file: &str, text: &str) -> Vec<PipelineSite> {
    let clean = blank_comments_and_literals(text);
    let b = clean.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(off) = clean[from..].find("GraphicsPipelineDesc {") {
        let start = from + off;
        let open = start + "GraphicsPipelineDesc ".len();
        let end = matching(b, open, b'{', b'}');
        let body = &clean[open..end];
        let fragment = body
            .find("fragment_module:")
            .map(|i| {
                body[i + "fragment_module:".len()..]
                    .trim_start()
                    .trim_start_matches('&')
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect::<String>()
            })
            .unwrap_or_default();
        let color_formats = body
            .find("color_formats:")
            .and_then(|i| body[i..].find('[').map(|j| i + j))
            .map(|lb| {
                let rb = matching(body.as_bytes(), lb, b'[', b']');
                body[lb + 1..rb - 1].split(',').map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()).collect()
            })
            .unwrap_or_default();
        out.push(PipelineSite { file: file.to_owned(), line: clean[..start].lines().count() + 1, fragment, color_formats });
        from = end;
    }
    out
}

/// The census over a set of sites: every site classified, every `lit` / `lit_native` site naming
/// exactly `[LIT_FORMAT]`, every table row present. `Ok(lit-class sites checked)` or the breaches.
fn census(sites: &[PipelineSite]) -> Result<usize, Vec<String>> {
    let mut breaches = Vec::new();
    let mut lit_sites = 0;
    for site in sites {
        let row = PIPELINE_TARGETS.iter().find(|(f, frag, _)| site.file.ends_with(f) && site.fragment == *frag);
        match row {
            None => breaches.push(format!(
                "{}:{} — an unclassified graphics pipeline (fragment module `{}`): add its row to PIPELINE_TARGETS",
                site.file, site.line, site.fragment
            )),
            Some(&(_, _, Target::Lit | Target::LitNative)) => {
                lit_sites += 1;
                if site.color_formats != ["LIT_FORMAT"] {
                    breaches.push(format!(
                        "{}:{} — `{}` draws into lit / lit_native but declares color_formats {:?}, not [LIT_FORMAT]",
                        site.file, site.line, site.fragment, site.color_formats
                    ));
                }
            }
            Some(_) => {}
        }
    }
    for (f, frag, _) in PIPELINE_TARGETS {
        if !sites.iter().any(|s| s.file.ends_with(f) && s.fragment == *frag) {
            breaches.push(format!("PIPELINE_TARGETS row ({f}, {frag}) names no pipeline on this tree: a stale row"));
        }
    }
    if breaches.is_empty() { Ok(lit_sites) } else { Err(breaches) }
}

#[test]
fn every_pipeline_drawing_into_lit_names_lit_format() {
    let root = workspace_root();
    let mut files = Vec::new();
    for dir in ["crates/boyko_app/src", "crates/boyko_render/src"] {
        rust_files(&root.join(dir), &mut files);
    }
    let sites: Vec<PipelineSite> = files.iter().flat_map(|p| pipeline_sites(&rel(&root, p), &read(p))).collect();
    let lit_rows = PIPELINE_TARGETS.iter().filter(|r| matches!(r.2, Target::Lit | Target::LitNative)).count();
    assert!(lit_rows >= 6 && sites.len() >= PIPELINE_TARGETS.len(), "anti-vacuity: {} sites, {lit_rows} lit rows", sites.len());
    match census(&sites) {
        Ok(n) => assert_eq!(n, lit_rows, "every lit / lit_native row was checked against a site"),
        Err(breaches) => panic!("the lit pipeline census is RED:\n{}", breaches.join("\n")),
    }
}

#[test]
fn the_pipeline_census_rejects_its_fixtures() {
    let fixture = r#"
fn boot() {
    let a = GraphicsPipelineDesc { fragment_module: &forward_fs, color_formats: &[RASTER_COLOR_FORMAT], depth_format: None };
    let b = GraphicsPipelineDesc { fragment_module: &ssaa_fs, color_formats: &[LIT_FORMAT] };
    let c = GraphicsPipelineDesc { fragment_module: &brand_new_fs, color_formats: &[LIT_FORMAT] };
}
"#;
    let sites = pipeline_sites("boyko_app/src/gpu_scene/mod.rs", fixture);
    assert_eq!(sites.len(), 3, "{sites:?}");
    assert_eq!(sites[0].color_formats, ["RASTER_COLOR_FORMAT"]);
    let breaches = census(&sites).expect_err("the fixture must red");
    let joined = breaches.join("\n");
    assert!(joined.contains("`forward_fs` draws into lit"), "a lit site naming another format: {joined}");
    assert!(!joined.contains("`ssaa_fs` draws"), "a lit_native site naming LIT_FORMAT passes: {joined}");
    assert!(joined.contains("unclassified graphics pipeline (fragment module `brand_new_fs`)"), "{joined}");
    assert!(joined.contains("(boyko_app/src/gpu_scene/mod.rs, sky_fs) names no pipeline"), "a stale row: {joined}");
}
