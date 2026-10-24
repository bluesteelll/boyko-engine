//! SSAOM F2 — the specular occlusion takes the GGX alpha, in ONE shared shader function.
//!
//! Every lit producer decouples ambient SPECULAR occlusion from diffuse AO with Lagarde's
//! cone approximation, `saturate(pow(NoV + ao, exp2(-16 * x - 1)) - 1 + ao)`. Filament
//! (`surface_light_indirect.fs`), Bevy (`pbr_fragment.wesl`) and Frostbite feed it the GGX
//! alpha `x = roughness^2`; before SSAOM every producer fed it the PERCEPTUAL roughness. The
//! exponent is then far too small for a polished metal, so a mild AO dip passes almost
//! straight through to the metal's ambient specular (its ENTIRE appearance — a metal has no
//! diffuse): NoV 0.7, ao 0.85, roughness 0.18 gives `spec_ao` 0.880 under the perceptual
//! argument and 1.0 (saturated) under alpha.
//!
//! The four tests:
//!   * [`a_polished_metal_keeps_its_ambient_specular_under_mild_ao`] — the behaviour, through
//!     the host resolve oracle `goldens::golden_deferred_resolve` (its spec-AO twin is
//!     `goldens::specular_ao`).
//!   * [`the_lagarde_exponent_lives_in_one_shared_function`] — the census: the exponent text
//!     appears in exactly one shader line, inside `spec_ao_lagarde` in `pbr_lighting.hlsli`.
//!   * [`every_lit_producer_feeds_spec_ao_the_ggx_alpha`] — each of the six producers calls
//!     the shared function once, with `a`, after `float a = roughness * roughness;`.
//!   * [`the_shared_body_is_the_host_formula`] — the shared body's text is pinned to the host
//!     twin's formula.
//!
//! Host-only: no device, nothing ignored.

use std::fs;
use std::path::{Path, PathBuf};

use boyko_rhi_vulkan::goldens::{
    GoldenMaterial, MarcherAttributes, golden_deferred_resolve, oct_decode,
};

/// The six lit producers whose ambient specular reads `spec_ao`
/// (`crates/boyko_rhi_vulkan/shaders/`).
const LIT_PRODUCERS: [&str; 6] = [
    "deferred_pbr.hlsl",
    "forward_opaque.fs.hlsl",
    "vb_resolve.comp.hlsl",
    "vb_shade.comp.hlsl",
    "vb_shade_split.comp.hlsl",
    "sdf_forward_march.comp.hlsl",
];

/// The shared header that owns the one Lagarde specular-occlusion function.
const SHARED_HEADER: &str = "pbr_lighting.hlsli";

/// The shared function's signature, as `pbr_lighting.hlsli` declares it.
const SHARED_SIGNATURE: &str = "float spec_ao_lagarde(float NoV, float ao, float a)";

/// The call every producer makes, with the GGX alpha `a` as the third argument.
const PRODUCER_CALL: &str = "spec_ao_lagarde(NoV, ao_final, a)";

/// The GGX alpha every producer defines before its `spec_ao` (the text all six spell).
const ALPHA_DEFINITION: &str = "float a = roughness * roughness;";

/// The Lagarde exponent's text — the token the census counts.
const LAGARDE_EXPONENT_TOKEN: &str = "exp2(-16.0 *";

/// The pinned body of `spec_ao_lagarde`. Its host twin is `goldens::specular_ao`
/// (`crates/boyko_rhi_vulkan/src/goldens.rs`): edit both or neither.
const PINNED_BODY: &str = "return saturate(pow(NoV + ao, exp2(-16.0 * a - 1.0)) - 1.0 + ao);";

fn shaders_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

fn read_shader(name: &str) -> String {
    let path = shaders_dir().join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The text with every `//` line comment removed (HLSL in this tree carries no block comments
/// and no string literals, so a line comment runs from `//` to the end of the line).
fn strip_line_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let code = match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        };
        out.push_str(code);
        out.push('\n');
    }
    out
}

/// The body of `spec_ao_lagarde` (between its braces, comments stripped, whitespace collapsed),
/// or `None` when the header does not declare the function.
fn shared_body(header: &str) -> Option<String> {
    let code = strip_line_comments(header);
    let start = code.find(SHARED_SIGNATURE)?;
    let open = start + code[start..].find('{')?;
    let mut depth = 0usize;
    for (off, ch) in code[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let inner = &code[open + 1..open + off];
                    return Some(inner.split_whitespace().collect::<Vec<_>>().join(" "));
                }
            }
            _ => {}
        }
    }
    None
}

/// The shader file names (`*.hlsl` and `*.hlsli`) in `shaders/`, sorted.
fn shader_sources() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(shaders_dir())
        .expect("read the shaders directory")
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            (name.ends_with(".hlsl") || name.ends_with(".hlsli")).then_some(name)
        })
        .collect();
    names.sort();
    names
}

/// One lit pixel of a polished gold metal (roughness 0.18) viewed at NoV ~= 0.7, through the
/// host resolve oracle, at the given A2 AO byte.
fn polished_gold_lit(ao: u8) -> ([f32; 3], f32) {
    // The normal the producers' oct store writes for the byte pair `[128, 128]` (~= +Z).
    let n = oct_decode([128.0 / 255.0, 128.0 / 255.0]);
    // A unit tangent perpendicular to `n`, then the view `v = 0.7 n + sqrt(1 - 0.49) t`.
    let x_dot_n = n[0];
    let t_raw = [1.0 - x_dot_n * n[0], -x_dot_n * n[1], -x_dot_n * n[2]];
    let t_len = (t_raw[0] * t_raw[0] + t_raw[1] * t_raw[1] + t_raw[2] * t_raw[2]).sqrt();
    let t = [t_raw[0] / t_len, t_raw[1] / t_len, t_raw[2] / t_len];
    let s = (1.0f32 - 0.49).sqrt();
    let v = [
        0.7 * n[0] + s * t[0],
        0.7 * n[1] + s * t[1],
        0.7 * n[2] + s * t[2],
    ];
    let v_len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let rd = [-v[0] / v_len, -v[1] / v_len, -v[2] / v_len];
    let nov = -(n[0] * rd[0] + n[1] * rd[1] + n[2] * rd[2]);

    let gold = [0.9f32, 0.68, 0.30];
    let base_rgb = gold.map(|c| (c * 255.0 + 0.5).floor() as u8);
    let materials = [GoldenMaterial::new(
        [gold[0], gold[1], gold[2], 1.0],
        1.0,
        0.18,
        0.5,
        [0.0, 0.0, 0.0],
    )];
    let attrs = MarcherAttributes {
        base_rgb,
        oct_rg: [128, 128],
        mat_id: 0,
        shadow: 255,
        ao,
        mask: 1,
        view_t: 3.0,
    };
    (golden_deferred_resolve(attrs, rd, &materials), nov)
}

/// The behaviour: a mild AO dip (217/255 ~= 0.85) leaves a polished metal's lit value
/// untouched under the alpha argument (`spec_ao` saturates to 1.0, and a metal has no diffuse
/// for the diffuse AO to darken), while the perceptual argument passes ~12 % of it through.
#[test]
fn a_polished_metal_keeps_its_ambient_specular_under_mild_ao() {
    let (open, nov) = polished_gold_lit(255);
    let (occluded, nov_occluded) = polished_gold_lit(217);
    assert_eq!(nov.to_bits(), nov_occluded.to_bits());
    assert!(
        (nov - 0.7).abs() < 1.0e-3,
        "fixture: the view must sit at NoV ~= 0.7, got {nov}"
    );
    println!("NoV = {nov}, lit(ao=255) = {open:?}, lit(ao=217) = {occluded:?}");
    for c in 0..3 {
        assert!(
            open[c] > 0.0,
            "fixture: channel {c} of the unoccluded metal must be lit, got {}",
            open[c]
        );
        let ratio = occluded[c] / open[c];
        println!("channel {c}: lit(217) / lit(255) = {ratio}");
        assert!(
            ratio >= 0.99,
            "a polished metal (roughness 0.18, NoV {nov}) must keep its ambient specular under \
             mild AO (217/255): channel {c} ratio {ratio} < 0.99 — the specular occlusion is fed \
             the perceptual roughness, not the GGX alpha"
        );
    }
}

/// The census: the Lagarde exponent appears in exactly one shader line, inside the shared
/// `spec_ao_lagarde` of `pbr_lighting.hlsli` — no producer carries its own copy.
#[test]
fn the_lagarde_exponent_lives_in_one_shared_function() {
    let mut hits: Vec<String> = Vec::new();
    for name in shader_sources() {
        let code = strip_line_comments(&read_shader(&name));
        for (i, line) in code.lines().enumerate() {
            if line.contains(LAGARDE_EXPONENT_TOKEN) {
                hits.push(format!("{name}:{}: {}", i + 1, line.trim()));
            }
        }
    }
    println!(
        "`{LAGARDE_EXPONENT_TOKEN}` occurrences:\n  {}",
        hits.join("\n  ")
    );
    assert_eq!(
        hits.len(),
        1,
        "the Lagarde exponent `{LAGARDE_EXPONENT_TOKEN}` must occur in exactly ONE shader line \
         (inside `spec_ao_lagarde`, `{SHARED_HEADER}`); found {}:\n  {}",
        hits.len(),
        hits.join("\n  ")
    );
    assert!(
        hits[0].starts_with(&format!("{SHARED_HEADER}:")),
        "the one Lagarde exponent must live in `{SHARED_HEADER}`, found it at {}",
        hits[0]
    );
    let body = shared_body(&read_shader(SHARED_HEADER))
        .unwrap_or_else(|| panic!("`{SHARED_HEADER}` must declare `{SHARED_SIGNATURE}`"));
    assert!(
        body.contains(LAGARDE_EXPONENT_TOKEN),
        "the one Lagarde exponent must sit inside `spec_ao_lagarde`'s body, got body `{body}`"
    );
}

/// Every lit producer calls the shared function exactly once, with the GGX alpha `a`, and
/// defines `a` before that call.
#[test]
fn every_lit_producer_feeds_spec_ao_the_ggx_alpha() {
    let mut failures: Vec<String> = Vec::new();
    for name in LIT_PRODUCERS {
        let code = strip_line_comments(&read_shader(name));
        let calls: Vec<usize> = code
            .match_indices(PRODUCER_CALL)
            .map(|(at, _)| at)
            .collect();
        if calls.len() != 1 {
            failures.push(format!(
                "{name}: {} calls of `{PRODUCER_CALL}` (expected exactly 1)",
                calls.len()
            ));
            continue;
        }
        match code.find(ALPHA_DEFINITION) {
            Some(def) if def < calls[0] => {}
            Some(_) => failures.push(format!(
                "{name}: `{ALPHA_DEFINITION}` appears only AFTER `{PRODUCER_CALL}`"
            )),
            None => failures.push(format!("{name}: no `{ALPHA_DEFINITION}`")),
        }
    }
    assert!(
        failures.is_empty(),
        "every lit producer must feed the specular occlusion the GGX alpha through the shared \
         `spec_ao_lagarde`:\n  {}",
        failures.join("\n  ")
    );
}

/// The shared body's text is the host twin's formula (`goldens::specular_ao`: `(nov +
/// ao).powf((-16.0 * a - 1.0).exp2()) - 1.0 + ao`, clamped to `[0, 1]`). Edit both or neither.
#[test]
fn the_shared_body_is_the_host_formula() {
    let body = shared_body(&read_shader(SHARED_HEADER))
        .unwrap_or_else(|| panic!("`{SHARED_HEADER}` must declare `{SHARED_SIGNATURE}`"));
    assert_eq!(
        body, PINNED_BODY,
        "`spec_ao_lagarde`'s body drifted from the pinned host formula (`goldens::specular_ao`)"
    );
}
