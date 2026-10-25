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
//! The six tests:
//!   * [`a_polished_metal_keeps_its_ambient_specular_under_mild_ao`] — the behaviour, through
//!     the host resolve oracle `goldens::golden_deferred_resolve` (its spec-AO twin is
//!     `goldens::specular_ao`).
//!   * [`the_lagarde_exponent_lives_in_one_shared_function`] — the census: the exponent text
//!     appears in exactly one shader line, inside `spec_ao_lagarde` in `pbr_lighting.hlsli`.
//!   * [`every_lit_producer_feeds_spec_ao_the_ggx_alpha`] — each of the six producers calls
//!     the shared function once, with `a`, after `float a = roughness * roughness;`.
//!   * [`the_shared_body_is_the_host_formula`] — the shared body's text is pinned to the host
//!     twin's formula.
//!   * [`every_host_spec_ao_site_takes_the_ggx_alpha`] — the host census: every
//!     `specular_ao` / `budget_spec_ao` call in `src/goldens.rs` passes `a`, after
//!     `let a = roughness * roughness;` in the same function, and both host bodies build the
//!     exponent from `alpha`. It is the only host gate for the uncalled
//!     `golden_deferred_resolve_clustered_shadowed` and for the device-host bound
//!     `budget_spec_ao`, whose one host consumer asserts only finiteness.
//!   * [`the_clustered_resolve_keeps_a_polished_metal_under_mild_ao`] — the behaviour through
//!     `golden_deferred_resolve_clustered` with clusters ON, whose own spec-AO site no other
//!     host test feeds an AO below 255; it also equals the table resolve there.
//!
//! The other host sites are gated elsewhere: the table resolve by
//! `lighting_l0_host_oracle.rs`'s degenerate-table identity, and the shadowed resolve by
//! `goldens.rs`'s `l0b_table_oracle_is_the_shadowed_one_at_shadow_mode_0`.
//!
//! Host-only: no device, nothing ignored.

use std::fs;
use std::path::{Path, PathBuf};

use boyko_rhi_vulkan::compute::{
    CompositeCamera, PBR_SKY_DIFFUSE, SDF_IMG_H, SDF_IMG_W, composite_pixel_ray,
};
use boyko_rhi_vulkan::goldens::{
    GoldenClusterConfig, GoldenLight, GoldenLightHeader, GoldenMaterial, MarcherAttributes,
    golden_cluster_cull, golden_deferred_resolve, golden_deferred_resolve_clustered,
    golden_deferred_resolve_table, oct_decode,
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

/// The host oracle's source, relative to this crate's manifest directory.
const GOLDENS_SOURCE: &str = "src/goldens.rs";

/// Every host spec-AO call in `src/goldens.rs`, `(callee, enclosing function)`, in source
/// order. A new caller joins this list once the census has checked that it passes `a`.
const HOST_SPEC_AO_SITES: [(&str, &str); 6] = [
    ("specular_ao", "golden_deferred_resolve_with_pbr"),
    ("specular_ao", "golden_deferred_resolve_table_ssao"),
    ("specular_ao", "resolve_shadowed_impl"),
    ("budget_spec_ao", "resolve_shadowed_impl"),
    ("specular_ao", "golden_deferred_resolve_clustered"),
    ("specular_ao", "golden_deferred_resolve_clustered_shadowed"),
];

/// The callees the host census reads: the spec-AO twin and its device-host bound.
const HOST_SPEC_AO_CALLEES: [&str; 2] = ["specular_ao", "budget_spec_ao"];

/// The GGX alpha every host caller defines before its spec-AO call.
const HOST_ALPHA_DEFINITION: &str = "let a = roughness * roughness;";

/// The parameter list both host spec-AO functions declare (the second one is the alpha).
const HOST_ALPHA_PARAMS: &str = "(nov: f32, alpha: f32, ao: f32)";

/// The host Lagarde exponent's token, and the alpha form every occurrence must spell.
const HOST_EXPONENT_TOKEN: &str = "-16.0 *";
const HOST_EXPONENT_ALPHA: &str = "-16.0 * alpha - 1.0";

fn shaders_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

fn read_goldens_source() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDENS_SOURCE);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The 1-based line of byte `at` (the comment strip keeps every line, so it is the source line).
fn line_of(code: &str, at: usize) -> usize {
    code[..at].matches('\n').count() + 1
}

/// The last `fn <name>` header before byte `at`: its byte position and the function's name.
fn enclosing_fn(code: &str, at: usize) -> Option<(usize, &str)> {
    let mut end = at;
    loop {
        let pos = code[..end].rfind("fn ")?;
        let on_boundary = code[..pos]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        let rest = &code[pos + 3..];
        let len = rest.find(|c: char| !is_ident_char(c)).unwrap_or(rest.len());
        if on_boundary && len > 0 {
            return Some((pos, &rest[..len]));
        }
        end = pos;
    }
}

/// One call of a host spec-AO function: where it is, what it calls, its enclosing function and
/// its arguments (trimmed, whitespace collapsed).
struct HostCall<'a> {
    at: usize,
    callee: &'static str,
    fn_at: usize,
    enclosing: &'a str,
    args: Vec<String>,
}

/// Every call (not the definition) of `callee` in the comment-stripped `code`.
fn host_calls<'a>(code: &'a str, callee: &'static str) -> Vec<HostCall<'a>> {
    let needle = format!("{callee}(");
    let mut calls = Vec::new();
    for (at, _) in code.match_indices(&needle) {
        let preceded_by_ident = code[..at].chars().next_back().is_some_and(is_ident_char);
        if preceded_by_ident || code[..at].trim_end().ends_with("fn") {
            continue;
        }
        let mut args = Vec::new();
        let mut current = String::new();
        let mut depth = 1usize;
        for ch in code[at + needle.len()..].chars() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                break;
            }
            if ch == ',' && depth == 1 {
                args.push(std::mem::take(&mut current));
            } else {
                current.push(ch);
            }
        }
        args.push(current);
        let args = args
            .iter()
            .map(|a| a.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        let (fn_at, enclosing) = enclosing_fn(code, at).unwrap_or_else(|| {
            panic!(
                "goldens.rs:{}: `{callee}(` outside any fn",
                line_of(code, at)
            )
        });
        calls.push(HostCall {
            at,
            callee,
            fn_at,
            enclosing,
            args,
        });
    }
    calls
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
        oct_rg: boyko_rhi_vulkan::goldens::oct_codes_from_oct8([128, 128]),
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

/// The host census: every spec-AO call in the host oracle (`src/goldens.rs`) feeds the GGX
/// alpha `a`, defined as `roughness * roughness` earlier in the same function, and both host
/// spec-AO bodies (the twin and its device-host bound) build the Lagarde exponent from their
/// `alpha` parameter.
#[test]
fn every_host_spec_ao_site_takes_the_ggx_alpha() {
    let code = strip_line_comments(&read_goldens_source());
    let mut calls: Vec<HostCall<'_>> = HOST_SPEC_AO_CALLEES
        .iter()
        .flat_map(|callee| host_calls(&code, callee))
        .collect();
    calls.sort_by_key(|call| call.at);

    let mut failures: Vec<String> = Vec::new();
    for call in &calls {
        let line = line_of(&code, call.at);
        println!(
            "goldens.rs:{line}: {}({}) in `{}`",
            call.callee,
            call.args.join(", "),
            call.enclosing
        );
        if call.args.len() != 3 || call.args[1] != "a" {
            failures.push(format!(
                "goldens.rs:{line}: `{}({})` in `{}` — the alpha argument must be `a`",
                call.callee,
                call.args.join(", "),
                call.enclosing
            ));
        }
        let before = &code[call.fn_at..call.at];
        let definition = before.rfind("let a =").map(|def| {
            let stmt = &before[def..];
            let stmt = &stmt[..stmt.find(';').map_or(stmt.len(), |end| end + 1)];
            stmt.split_whitespace().collect::<Vec<_>>().join(" ")
        });
        if definition.as_deref() != Some(HOST_ALPHA_DEFINITION) {
            failures.push(format!(
                "goldens.rs:{line}: `{}` in `{}` — the last `let a` before it is {definition:?}, \
                 expected `{HOST_ALPHA_DEFINITION}`",
                call.callee, call.enclosing
            ));
        }
    }
    let found: Vec<(&str, &str)> = calls.iter().map(|c| (c.callee, c.enclosing)).collect();
    assert_eq!(
        found,
        HOST_SPEC_AO_SITES.to_vec(),
        "the host spec-AO call sites in `{GOLDENS_SOURCE}` changed: check that every new one \
         passes `a`, then update `HOST_SPEC_AO_SITES`"
    );

    let mut exponent_fns: Vec<&str> = Vec::new();
    for (at, _) in code.match_indices(HOST_EXPONENT_TOKEN) {
        let line = line_of(&code, at);
        let (fn_at, name) = enclosing_fn(&code, at)
            .unwrap_or_else(|| panic!("goldens.rs:{line}: the Lagarde exponent outside any fn"));
        exponent_fns.push(name);
        let spelled = code[at..].lines().next().unwrap_or("").trim();
        println!("goldens.rs:{line}: `{spelled}` in `{name}`");
        if !code[at..].starts_with(HOST_EXPONENT_ALPHA) {
            failures.push(format!(
                "goldens.rs:{line}: `{spelled}` in `{name}` — the exponent must read \
                 `{HOST_EXPONENT_ALPHA}`"
            ));
        }
        if !code[fn_at..at].starts_with(&format!("fn {name}{HOST_ALPHA_PARAMS}")) {
            failures.push(format!(
                "goldens.rs:{line}: `{name}` must declare `{HOST_ALPHA_PARAMS}`"
            ));
        }
    }
    assert_eq!(
        exponent_fns,
        HOST_SPEC_AO_CALLEES.to_vec(),
        "the Lagarde exponent `{HOST_EXPONENT_TOKEN}` must occur exactly once in each host \
         spec-AO function, in source order"
    );
    assert!(
        failures.is_empty(),
        "every host spec-AO site must take the GGX alpha:\n  {}",
        failures.join("\n  ")
    );
}

/// The cluster config of the clustered-resolve fixture (`lighting_l1_host_oracle.rs`'s grid:
/// its exp-Z slices span the ortho scene's `view_t ~= 2`).
fn cluster_cfg() -> GoldenClusterConfig {
    GoldenClusterConfig {
        dim_x: 16,
        dim_y: 9,
        dim_z: 24,
        max_lights_per_cluster: 256,
        z_near: 0.25,
        z_far: 4.0,
    }
}

/// The oct8 byte pair `[x, 128]` whose decoded normal sits closest to NoV 0.7 under the ortho
/// camera (view `+Z`), and that NoV.
fn oct_at_nov_0_7() -> ([u8; 2], f32) {
    (0..=255u8)
        .map(|x| {
            let n = oct_decode([f32::from(x) / 255.0, 128.0 / 255.0]);
            ([x, 128], n[2])
        })
        .min_by(|a, b| (a.1 - 0.7).abs().total_cmp(&(b.1 - 0.7).abs()))
        .expect("invariant: the byte range is not empty")
}

/// The polished gold metal (roughness 0.18) at NoV ~= 0.7 under the degenerate light table (one
/// directional, one sky, no punctual light), at the given A2 AO byte: the CLUSTERED resolve with
/// clusters ON, the brute-force table resolve of the same pixel, and the NoV.
fn polished_gold_clustered(ao: u8) -> ([f32; 3], [f32; 3], f32) {
    let cfg = cluster_cfg();
    let lights = [
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
        GoldenLight::sky(PBR_SKY_DIFFUSE, PBR_SKY_DIFFUSE),
    ];
    let clustered_header = GoldenLightHeader::new_clustered(2, 0, 1.0, &cfg);
    assert!(
        clustered_header.clusters_enabled(),
        "fixture: the clustered header must enable clusters, or the resolve delegates to the table"
    );
    let table_header = GoldenLightHeader::new(2, 0, 1.0);
    let camera = CompositeCamera::Ortho;
    let grid = golden_cluster_cull(
        SDF_IMG_W,
        SDF_IMG_H,
        camera,
        &cfg,
        &clustered_header,
        &lights,
        None,
    );

    let gold = [0.9f32, 0.68, 0.30];
    let materials = [GoldenMaterial::new(
        [gold[0], gold[1], gold[2], 1.0],
        1.0,
        0.18,
        0.5,
        [0.0, 0.0, 0.0],
    )];
    let (oct_rg, _) = oct_at_nov_0_7();
    let attrs = MarcherAttributes {
        base_rgb: gold.map(|c| (c * 255.0 + 0.5).floor() as u8),
        oct_rg: boyko_rhi_vulkan::goldens::oct_codes_from_oct8(oct_rg),
        mat_id: 0,
        shadow: 255,
        ao,
        mask: 1,
        view_t: 2.0,
    };
    let (px, py) = (32, 32);
    let (ro, rd) = composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, camera);
    let n = oct_decode([f32::from(oct_rg[0]) / 255.0, f32::from(oct_rg[1]) / 255.0]);
    let nov = -(n[0] * rd[0] + n[1] * rd[1] + n[2] * rd[2]);
    let clustered = golden_deferred_resolve_clustered(
        attrs,
        px,
        py,
        SDF_IMG_W,
        SDF_IMG_H,
        camera,
        &materials,
        &clustered_header,
        &lights,
        &cfg,
        &grid,
    );
    let table = golden_deferred_resolve_table(attrs, ro, rd, &materials, &table_header, &lights);
    (clustered, table, nov)
}

/// The behaviour through the CLUSTERED resolve (clusters ON), whose own spec-AO site no other
/// host test reaches with an AO below 255: the polished metal keeps its lit value under mild AO
/// (217/255), and with no punctual light in the scene the clustered value is the table
/// resolve's, bit for bit.
#[test]
fn the_clustered_resolve_keeps_a_polished_metal_under_mild_ao() {
    let (open, open_table, nov) = polished_gold_clustered(255);
    let (occluded, occluded_table, _) = polished_gold_clustered(217);
    println!(
        "NoV = {nov}, clustered lit(ao=255) = {open:?}, lit(ao=217) = {occluded:?}; \
         table lit(ao=217) = {occluded_table:?}"
    );
    assert!(
        (nov - 0.7).abs() < 1.0e-2,
        "fixture: the view must sit at NoV ~= 0.7 (one oct8 step), got {nov}"
    );
    assert_eq!(
        open, open_table,
        "the clustered resolve (no punctual light, ao 255) must equal the table resolve"
    );
    assert_eq!(
        occluded, occluded_table,
        "the clustered resolve (no punctual light, ao 217) must equal the table resolve: their \
         spec-AO sites disagree"
    );
    for c in 0..3 {
        assert!(
            open[c] > 0.0,
            "fixture: channel {c} of the unoccluded metal must be lit, got {}",
            open[c]
        );
        let ratio = occluded[c] / open[c];
        println!("channel {c}: clustered lit(217) / lit(255) = {ratio}");
        assert!(
            ratio >= 0.99,
            "the clustered resolve must keep a polished metal (roughness 0.18, NoV {nov}) under \
             mild AO (217/255): channel {c} ratio {ratio} < 0.99 — its specular occlusion is fed \
             the perceptual roughness, not the GGX alpha"
        );
    }
}
