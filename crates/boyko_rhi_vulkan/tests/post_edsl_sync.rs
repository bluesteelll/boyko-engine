//! `post_final`'s eDSL ↔ HLSL ↔ `.spv` guard (render wave PXe, `plan/H2-PXe-post-edsl.md` rev 5 tests
//! 5, 7, 8a, 9, 10; `e0-pxe/cut.md` D-P1, D-P8, D-P9, D-P13, OPEN-ITEMS item 4).
//!
//! 1. **Generator sync** — the committed `post_final.comp.hlsl` IS
//!    `boyko_shaderdsl::emit::emit_post_final_comp()`, whole; each `// === GENERATED <span> ===` is
//!    the printer's output.
//! 2. **`.spv` byte identity** under the frozen recipe its header pins (a `SKIP` line when no `dxc`
//!    resolves is NOT a pass).
//! 3. **SPIR-V censuses** (`spirv-dis` of the committed module): exactly three `GLSL.std.450 Pow`
//!    whose exponent is the `OETF_GAMMA_EXP` constant, and (render wave PXt) three on AgX's
//!    `AGX_LINEARIZE_EXP` and three `Log2` — six `Pow` in all — and no `Exp2` (the OETF and AgX's
//!    `^2.2` cannot be rewritten into `exp2(y·log2 x)`; RCAS's strength is a host-computed `con`); the
//!    RCAS stage and the three-level tonemapper dispatch are REAL branches (four `DontFlatten`
//!    selection merges, no `float3` `OpSelect` anywhere — the leaves are per-channel scalar code, so
//!    a `v3float` select can only come from an if-converted branch); AgX's six transcendentals sit
//!    inside the dispatch selection that opens after the sharpen selection, so only word 3 issues
//!    them, and each `Log2` reads a `max` against AgX's `1e-10` floor (`pxt/cut.md` D-T6, critique
//!    W2: the floor is unobservable in any host value, so its presence is held here); exactly five
//!    `OpImageFetch` — the scene texel once, before the sharpen selection, and RCAS's four ring taps
//!    INSIDE it (render wave PX1b D-R10 (2): the sharpen-off path fetches once); the declared
//!    bindings and the UBO member offsets are the tables'.
//! 4. **Structure** — the skeleton outside the sentinels holds no fetch, no resource index but the
//!    one `gDisplay[…]` store, one `post_final_main` call and no arithmetic (H2 test 7); the HLSL
//!    declarations equal `POST_FINAL_BINDINGS` row for row (8a); and (4b, render wave PX1b) no
//!    division in the body runs on into `*` / `/` — the printer spells a multiplicative divisor flat,
//!    so such a chain may not compute what the trace (and every host oracle) computes.
//! 5. **The stage trace (OPEN-ITEMS item 4, part a)** — the emitted body's `post_stage_*`
//!    declarations, which the trace itself produces, equal `POST_FINAL_STAGE_ORDER` (9a), and each
//!    stage feeds the next (9b): its identifier is read after its declaration and at or before the
//!    next stage's declaration, the last one in the `return`.
//! 6. **Single-source discipline** — `post.rs` carries no backend introspection (D-P13), and no
//!    non-test caller of the traced body re-implements a stage (H2 test 10, armed by CONTENT, not
//!    by a path PX0 might not use). Every census here is also run against fixtures it must reject,
//!    every run, so none is vacuous.
//! 7. **Render wave PXt — the sharpen-off module** (`post_final_nosharpen.comp`, OPEN-ITEMS 19): the
//!    same generator sync, recipe pin and `.spv` byte gate; its censuses — the same six `Pow` and
//!    three `Log2`, AgX's inside the dispatch (here the FIRST `DontFlatten` selection) on the floor's
//!    `max`, three `DontFlatten` merges (the dispatch only: DXC folded the sharpen stage), exactly
//!    ONE `OpImageFetch`, before every selection — the bindings the table declares, the stage trace
//!    and the skeleton census.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use boyko_shaderdsl::post::{
    AGX_LINEARIZE_EXP, AGX_LOG_FLOOR, OETF_GAMMA_EXP, POST_FINAL_BINDINGS, POST_FINAL_STAGE_ORDER,
    POST_FINAL_UBO_LAYOUT, PostBindingKind, PostUboKind,
};

/// The committed shader's stem.
const STEM: &str = "post_final.comp";

fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// The workspace root (two levels above this crate).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Reads a committed shader source, LF-normalized.
fn read_shader(name: &str) -> String {
    let path = shaders_dir().join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("invariant: shaders/{name} must exist next to this crate: {e}"))
        .replace("\r\n", "\n")
}

/// The pinned SDK path, then `$VULKAN_SDK/Bin`, then bare on `PATH`.
fn find_tool(stem: &str) -> Option<PathBuf> {
    let bare = if cfg!(windows) { format!("{stem}.exe") } else { stem.to_string() };
    let pinned = PathBuf::from(format!("C:/VulkanSDK/1.4.350.0/Bin/{bare}"));
    if pinned.exists() {
        return Some(pinned);
    }
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let candidate = PathBuf::from(sdk).join("Bin").join(&bare);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if Command::new(&bare).arg("--version").output_within(DXC_DEADLINE).is_ok() {
        return Some(PathBuf::from(bare));
    }
    None
}

/// The text between `// === GENERATED <name> BEGIN ===` and its END sentinel.
fn generated_span<'a>(src: &'a str, name: &str) -> &'a str {
    let begin = format!("// === GENERATED {name} BEGIN ===\n");
    let end = format!("// === GENERATED {name} END ===\n");
    let b = src.find(&begin).unwrap_or_else(|| panic!("missing sentinel `{}`", begin.trim_end()));
    let rest = &src[b + begin.len()..];
    let e = rest.find(&end).unwrap_or_else(|| panic!("missing sentinel `{}`", end.trim_end()));
    &rest[..e]
}

/// The source with every GENERATED span (sentinels included) removed and `//` comments stripped —
/// the hand-written skeleton's code.
fn skeleton_code(src: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in src.lines() {
        if line.starts_with("// === GENERATED ") && line.ends_with(" BEGIN ===") {
            inside = true;
            continue;
        }
        if line.starts_with("// === GENERATED ") && line.ends_with(" END ===") {
            inside = false;
            continue;
        }
        if inside {
            continue;
        }
        let code = match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        };
        out.push_str(code);
        out.push('\n');
    }
    out
}

/// The disassembly of the committed `.spv`, or `None` (with a SKIP line) when no `spirv-dis`.
fn disassembly(test: &str) -> Option<String> {
    disassembly_of(STEM, test)
}

/// The disassembly of the committed `<stem>.spv`, or `None` (with a SKIP line) when no `spirv-dis`.
fn disassembly_of(stem: &str, test: &str) -> Option<String> {
    let Some(spirv_dis) = find_tool("spirv-dis") else {
        eprintln!("SKIP {test}: no spirv-dis on this host");
        return None;
    };
    let spv = shaders_dir().join(format!("{stem}.spv"));
    let out = Command::new(&spirv_dis)
        .arg(&spv)
        .output_within(DXC_DEADLINE)
        .expect("invariant: spirv-dis was located and must run");
    assert!(out.status.success(), "spirv-dis failed on {}", spv.display());
    Some(String::from_utf8(out.stdout).expect("invariant: spirv-dis emits UTF-8"))
}

// ---- 1. Generator sync -----------------------------------------------------------------------------

#[test]
fn committed_source_is_the_generator_output() {
    let committed = read_shader(&format!("{STEM}.hlsl"));
    let generated = boyko_shaderdsl::emit::emit_post_final_comp();
    assert!(
        committed == generated,
        "{STEM}.hlsl DRIFTED from boyko_shaderdsl::emit::emit_post_final_comp — re-run `cargo run -p \
         boyko_shaderdsl --features emit --bin emit_post`, re-DXC the .spv with the recipe in the \
         shader's header, and commit both.\n--- generated ---\n{generated}\n--- committed ---\n{committed}"
    );
}

#[test]
fn each_generated_span_is_the_printer_output() {
    let src = read_shader(&format!("{STEM}.hlsl"));
    assert_eq!(generated_span(&src, "post_final_bindings"), boyko_shaderdsl::emit::emit_hlsl_post_final_bindings());
    assert_eq!(generated_span(&src, "post_final_consts"), boyko_shaderdsl::emit::emit_hlsl_post_final_consts());
    let main_fn = generated_span(&src, "post_final_main");
    let body = boyko_shaderdsl::emit::emit_hlsl_post_final_main();
    assert_eq!(main_fn, format!("float4 post_final_main(uint2 px) {{\n{body}}}\n"), "post_final_main drifted");
}

#[test]
fn header_pins_the_frozen_recipe() {
    let src = read_shader(&format!("{STEM}.hlsl"));
    let recipe = "C:\\VulkanSDK\\1.4.350.0\\Bin\\dxc.exe -spirv -T cs_6_0 -E main \\\n\
                  //       -fspv-target-env=vulkan1.3 post_final.comp.hlsl -Fo post_final.comp.spv";
    assert!(src.contains(recipe), "{STEM}.hlsl's header no longer pins the recipe this gate runs");
}

// ---- 2. `.spv` byte identity ---------------------------------------------------------------------

#[test]
fn post_final_spv_byte_identical() {
    let Some(dxc) = find_tool("dxc") else {
        eprintln!("SKIP post_final_spv_byte_identical: no dxc on this host");
        return;
    };
    let committed_path = shaders_dir().join(format!("{STEM}.spv"));
    let committed = std::fs::read(&committed_path)
        .unwrap_or_else(|e| panic!("missing committed {}: {e}", committed_path.display()));
    let out_spv = child_guard::scratch_path(&format!("{STEM}.redxc.spv"));
    let status = Command::new(&dxc)
        .current_dir(shaders_dir())
        .args(["-spirv", "-T", "cs_6_0", "-E", "main", "-fspv-target-env=vulkan1.3"])
        .arg(format!("{STEM}.hlsl"))
        .arg("-Fo")
        .arg(&*out_spv)
        .status_within(DXC_DEADLINE)
        .expect("invariant: dxc was located and must run");
    assert!(status.success(), "dxc failed re-compiling {STEM}.hlsl under the frozen recipe");
    let fresh = std::fs::read(&*out_spv).expect("invariant: dxc wrote the re-DXC .spv");
    assert!(
        committed == fresh,
        "{STEM}.spv ({} bytes committed, {} fresh) is NOT the re-DXC of {STEM}.hlsl under the frozen \
         recipe — the committed .spv is stale, or the host dxc is not VulkanSDK 1.4.350.0.",
        committed.len(),
        fresh.len()
    );
}

// ---- 3. SPIR-V censuses -----------------------------------------------------------------------------

/// The whitespace tokens of every line of a disassembly.
fn token_lines(dis: &str) -> Vec<Vec<&str>> {
    dis.lines().map(|l| l.split_whitespace().collect()).collect()
}

/// The result ids of a disassembly's `OpConstant %float` whose value IS `value`.
fn float_constant_ids<'a>(lines: &[Vec<&'a str>], value: f32) -> Vec<&'a str> {
    lines
        .iter()
        .filter(|t| t.len() == 5 && t[1] == "=" && t[2] == "OpConstant" && t[3] == "%float")
        .filter(|t| t[4].parse::<f32>().is_ok_and(|v| v.to_bits() == value.to_bits()))
        .map(|t| t[0])
        .collect()
}

/// `(pows on the exponent constant, all Pow, Exp2, Log2)` of a disassembly: a `Pow` counts toward
/// the first number when its exponent operand is an `OpConstant %float` whose value IS `exponent`.
fn pow_census(dis: &str, exponent: f32) -> (usize, usize, usize, usize) {
    let lines = token_lines(dis);
    let gamma_ids = float_constant_ids(&lines, exponent);
    let mut gamma_pows = 0;
    let mut pows = 0;
    let mut exp2 = 0;
    let mut log2 = 0;
    for t in &lines {
        if t.len() >= 6 && t[1] == "=" && t[2] == "OpExtInst" {
            match t[5] {
                "Pow" => {
                    pows += 1;
                    if t.last().is_some_and(|e| gamma_ids.contains(e)) {
                        gamma_pows += 1;
                    }
                }
                "Exp2" => exp2 += 1,
                "Log2" => log2 += 1,
                _ => {}
            }
        }
    }
    (gamma_pows, pows, exp2, log2)
}

#[test]
fn oetf_is_three_pow_on_the_gamma_constant() {
    let Some(dis) = disassembly("oetf_is_three_pow_on_the_gamma_constant") else { return };
    let (gamma_pows, pows, exp2, log2) = pow_census(&dis, OETF_GAMMA_EXP);
    assert_eq!(gamma_pows, 3, "the OETF must be exactly three GLSL.std.450 Pow on OETF_GAMMA_EXP (one per channel); {pows} Pow in all");
    let (agx_pows, _, _, _) = pow_census(&dis, AGX_LINEARIZE_EXP);
    assert_eq!(agx_pows, 3, "AgX's linearisation must be exactly three GLSL.std.450 Pow on AGX_LINEARIZE_EXP (render wave PXt)");
    assert_eq!(pows, 6, "post_final carries six Pow: the OETF's three and AgX's three");
    assert_eq!(
        (exp2, log2),
        (0, 3),
        "no Exp2 may appear, and the only Log2 are AgX's three (a pow rewritten through exp2(y·log2 x) is red)"
    );
}

/// `(DontFlatten selection merges, v3float OpSelects)` of a disassembly.
fn branch_census(dis: &str) -> (usize, usize) {
    let lines = token_lines(dis);
    let dont_flatten = lines.iter().filter(|t| t.first() == Some(&"OpSelectionMerge") && t.last() == Some(&"DontFlatten")).count();
    let v3_selects = lines
        .iter()
        .filter(|t| t.len() >= 4 && t[1] == "=" && t[2] == "OpSelect" && t[3] == "%v3float")
        .count();
    (dont_flatten, v3_selects)
}

#[test]
fn tonemap_dispatch_is_a_real_branch() {
    let Some(dis) = disassembly("tonemap_dispatch_is_a_real_branch") else { return };
    let (dont_flatten, v3_selects) = branch_census(&dis);
    assert_eq!(
        dont_flatten, 4,
        "the RCAS stage and the three-level tonemapper dispatch (render wave PXt) must be four DontFlatten selections (`[branch]`)"
    );
    assert_eq!(v3_selects, 0, "a float3 OpSelect means a branch was if-converted (every arm evaluated per pixel)");
}

/// `(fetches before the first DontFlatten selection, fetches between its OpBranchConditional and its
/// merge label, fetches after it)` of a disassembly — `None` when it has no DontFlatten selection.
fn fetch_structure(dis: &str) -> Option<(usize, usize, usize)> {
    let lines = token_lines(dis);
    let is_fetch = |t: &Vec<&str>| t.len() >= 3 && t[1] == "=" && t[2] == "OpImageFetch";
    let header = lines.iter().position(|t| t.first() == Some(&"OpSelectionMerge") && t.last() == Some(&"DontFlatten"))?;
    let merge = lines[header][1];
    let end = lines.iter().position(|t| t.len() == 3 && t[0] == merge && t[1] == "=" && t[2] == "OpLabel")?;
    let before = lines[..header].iter().filter(|t| is_fetch(t)).count();
    let inside = lines[header..end].iter().filter(|t| is_fetch(t)).count();
    let after = lines[end..].iter().filter(|t| is_fetch(t)).count();
    Some((before, inside, after))
}

/// `Err(reason)` unless the module fetches the scene texel once before the sharpen selection, RCAS's
/// four ring taps inside it, and nothing after it (render wave PX1b D-R10 (2)).
fn check_fetch_structure(dis: &str) -> Result<(), String> {
    match fetch_structure(dis) {
        Some((1, 4, 0)) => Ok(()),
        Some(s) => Err(format!(
            "OpImageFetch (before, inside, after) the first DontFlatten selection = {s:?}, not (1, 4, 0): the sharpen-off path must fetch once"
        )),
        None => Err("no DontFlatten selection: the sharpen stage is not a real branch".to_string()),
    }
}

#[test]
fn the_cross_is_fetched_inside_the_sharpen_branch() {
    let Some(dis) = disassembly("the_cross_is_fetched_inside_the_sharpen_branch") else { return };
    if let Err(why) = check_fetch_structure(&dis) {
        panic!("{why}");
    }
}

/// Render wave PXt (`pxt/cut.md` D-T6; critique W2) — `Err(why)` unless every `Log2` and every `Pow`
/// on `AGX_LINEARIZE_EXP` lies inside the tonemapper dispatch — the second `DontFlatten` selection,
/// which opens after the sharpen selection's merge label, or the first when the module has no
/// sharpen stage (`after_sharpen == false`, the sharpen-off module) — between its
/// `OpSelectionMerge` and its merge label, so only the dispatch issues AgX's transcendentals and
/// none runs before it; and every `Log2` reads an `NMax` / `FMax` of which one operand is the
/// `AGX_LOG_FLOOR` constant.
fn check_agx_placement(dis: &str, after_sharpen: bool) -> Result<(), String> {
    let lines = token_lines(dis);
    let floor_ids = float_constant_ids(&lines, AGX_LOG_FLOOR);
    let exp_ids = float_constant_ids(&lines, AGX_LINEARIZE_EXP);
    let is_header = |t: &Vec<&str>| t.first() == Some(&"OpSelectionMerge") && t.last() == Some(&"DontFlatten");
    let label_of = |merge: &str| lines.iter().position(|t| t.len() == 3 && t[0] == merge && t[1] == "=" && t[2] == "OpLabel");
    let search_from = if after_sharpen {
        let sharpen = lines.iter().position(is_header).ok_or("no DontFlatten selection: the sharpen stage is not a real branch")?;
        label_of(lines[sharpen][1]).ok_or("the sharpen selection has no merge label")?
    } else {
        0
    };
    let dispatch = (search_from..lines.len())
        .find(|&i| is_header(&lines[i]))
        .ok_or("no DontFlatten selection after the sharpen stage: the tonemapper dispatch is not a real branch")?;
    let dispatch_end = label_of(lines[dispatch][1]).ok_or("the dispatch selection has no merge label")?;
    let ext = |t: &Vec<&str>, inst: &str| t.len() >= 7 && t[1] == "=" && t[2] == "OpExtInst" && t[5] == inst;
    let mut seen = 0;
    for (i, t) in lines.iter().enumerate() {
        let agx_pow = ext(t, "Pow") && t.last().is_some_and(|e| exp_ids.contains(e));
        let log2 = ext(t, "Log2");
        if !(agx_pow || log2) {
            continue;
        }
        seen += 1;
        if !(dispatch < i && i < dispatch_end) {
            return Err(format!("`{}` (line {i}) lies outside the tonemapper dispatch selection (lines {dispatch}..{dispatch_end}): AgX's transcendentals would run on every pixel", t.join(" ")));
        }
        if log2 {
            let operand = t[6];
            let floored = lines.iter().any(|d| {
                d.len() >= 8
                    && d[0] == operand
                    && d[1] == "="
                    && d[2] == "OpExtInst"
                    && (d[5] == "NMax" || d[5] == "FMax")
                    && d[6..].iter().any(|o| floor_ids.contains(o))
            });
            if !floored {
                return Err(format!("`{}` (line {i}) does not read a max against AGX_LOG_FLOOR: log2(0) may reach the polynomial", t.join(" ")));
            }
        }
    }
    if seen == 0 {
        return Err("no Log2 and no Pow on AGX_LINEARIZE_EXP: the AgX arm is not in the module, the census is vacuous".to_string());
    }
    Ok(())
}

#[test]
fn agx_transcendentals_sit_inside_the_dispatch() {
    let Some(dis) = disassembly("agx_transcendentals_sit_inside_the_dispatch") else { return };
    if let Err(why) = check_agx_placement(&dis, true) {
        panic!("{why}");
    }
}

/// `%name -> (set, binding)` of every decorated variable of a disassembly.
fn binding_decorations(dis: &str) -> BTreeMap<String, (u32, u32)> {
    let mut set: BTreeMap<String, u32> = BTreeMap::new();
    let mut bind: BTreeMap<String, u32> = BTreeMap::new();
    for t in token_lines(dis) {
        if t.len() == 4 && t[0] == "OpDecorate" {
            let v = t[3].parse::<u32>();
            match (t[2], v) {
                ("DescriptorSet", Ok(v)) => {
                    set.insert(t[1].trim_start_matches('%').to_string(), v);
                }
                ("Binding", Ok(v)) => {
                    bind.insert(t[1].trim_start_matches('%').to_string(), v);
                }
                _ => {}
            }
        }
    }
    bind.into_iter().map(|(k, b)| (k.clone(), (set.get(&k).copied().unwrap_or(u32::MAX), b))).collect()
}

/// The HLSL declarations of the bindings span, as `(name, kind word, set, binding)`.
fn hlsl_declarations(span: &str) -> Vec<(String, String, u32, u32)> {
    let mut out = Vec::new();
    for line in span.lines() {
        let Some(rest) = line.strip_prefix("[[vk::binding(") else { continue };
        let close = rest.find(")]]").expect("a binding attribute closes");
        let mut nums = rest[..close].split(',').map(|n| n.trim().parse::<u32>().expect("binding numbers"));
        let (binding, set) = (nums.next().expect("binding"), nums.next().expect("set"));
        let toks: Vec<&str> = rest[close + 3..].split_whitespace().filter(|t| !t.starts_with("[[")).collect();
        let kind = toks[0].to_string();
        let name = toks[1].trim_end_matches(';').to_string();
        out.push((name, kind, set, binding));
    }
    out
}

/// The HLSL resource type a binding kind declares.
fn kind_word(kind: PostBindingKind) -> &'static str {
    match kind {
        PostBindingKind::Tex2Load | PostBindingKind::Tex2Linear => "Texture2D<float4>",
        PostBindingKind::Tex3Linear => "Texture3D<float4>",
        PostBindingKind::Buf => "StructuredBuffer<uint>",
        PostBindingKind::Ubo => "cbuffer",
        PostBindingKind::Store => "RWTexture2D<float4>",
    }
}

#[test]
fn declarations_equal_post_final_bindings() {
    let src = read_shader(&format!("{STEM}.hlsl"));
    let decls = hlsl_declarations(generated_span(&src, "post_final_bindings"));
    let want: Vec<(String, String, u32, u32)> = POST_FINAL_BINDINGS
        .iter()
        .map(|b| (b.name.to_string(), kind_word(b.kind).to_string(), b.set, b.binding))
        .collect();
    assert_eq!(decls, want, "the HLSL declarations are not POST_FINAL_BINDINGS row for row");

    // What the compiled module actually binds, and where the UBO members actually sit — the
    // printer's binding numbers and member order, and DXC's packing, against the tables.
    let Some(dis) = disassembly("declarations_equal_post_final_bindings") else { return };
    let decorated = binding_decorations(&dis);
    for b in POST_FINAL_BINDINGS {
        assert_eq!(decorated.get(b.name), Some(&(b.set, b.binding)), "{} is not decorated (set {}, binding {})", b.name, b.set, b.binding);
    }
    let offsets: Vec<u32> = token_lines(&dis)
        .iter()
        .filter(|t| t.len() == 5 && t[0] == "OpMemberDecorate" && t[1] == "%type_PostFinal" && t[3] == "Offset")
        .map(|t| t[4].parse::<u32>().expect("an offset"))
        .collect();
    let want_offsets: Vec<u32> = POST_FINAL_UBO_LAYOUT.iter().map(|f| f.offset).collect();
    assert_eq!(offsets, want_offsets, "the compiled PostFinal member offsets are not POST_FINAL_UBO_LAYOUT's");
    let uint2 = POST_FINAL_UBO_LAYOUT.iter().filter(|f| f.kind == PostUboKind::Uint2).count();
    assert_eq!(uint2, 1, "the layout's one uint2 (extent) is the guard's operand");
}

// ---- 4. The skeleton ----------------------------------------------------------------------------------

/// `Err(reason)` when the skeleton code holds a fetch, an index other than `gDisplay[`, not exactly
/// one `post_final_main(` call, or any arithmetic operator.
fn skeleton_violations(code: &str) -> Result<(), String> {
    for fetch in [".Load(", ".Sample", ".SampleLevel(", ".Gather"] {
        if code.contains(fetch) {
            return Err(format!("the skeleton fetches (`{fetch}`) — a stage written as skeleton text"));
        }
    }
    let bytes = code.as_bytes();
    let mut indexed = Vec::new();
    for (i, &c) in bytes.iter().enumerate() {
        if c == b'[' && i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
            let start = code[..i].rfind(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_')).map_or(0, |s| s + 1);
            indexed.push(&code[start..i]);
        }
    }
    if indexed != ["gDisplay"] {
        return Err(format!("the skeleton indexes {indexed:?}; only the one `gDisplay[…]` store is allowed"));
    }
    let calls = code.matches("post_final_main(").count();
    if calls != 1 {
        return Err(format!("the skeleton calls post_final_main {calls} times, not once"));
    }
    if let Some(op) = ['+', '-', '*', '/', '%'].iter().find(|op| code.contains(**op)) {
        return Err(format!("the skeleton computes (`{op}`) — arithmetic belongs in the traced body"));
    }
    Ok(())
}

#[test]
fn skeleton_holds_no_fetch_and_one_call() {
    let src = read_shader(&format!("{STEM}.hlsl"));
    if let Err(why) = skeleton_violations(&skeleton_code(&src)) {
        panic!("{why}");
    }
}

// ---- 4b. The printed association (render wave PX1b) ------------------------------------------------------

/// The end of the whole group that opens at `s[i]` (`open`), nesting included.
fn group_end(s: &[u8], mut i: usize, open: u8, close: u8) -> usize {
    let mut depth = 0usize;
    while i < s.len() {
        if s[i] == open {
            depth += 1;
        } else if s[i] == close {
            depth -= 1;
            if depth == 0 {
                return i + 1;
            }
        }
        i += 1;
    }
    i
}

/// The end of the one operand that starts at byte `i` of `s` (after spaces): an optional unary `-`,
/// then a parenthesised group, or an identifier / literal (a float's `e-5` exponent included) with an
/// optional call's argument list, then any `[index]` suffixes.
fn operand_end(s: &[u8], mut i: usize) -> usize {
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'.';
    while s.get(i) == Some(&b' ') {
        i += 1;
    }
    if s.get(i) == Some(&b'-') {
        i += 1;
    }
    if s.get(i) == Some(&b'(') {
        i = group_end(s, i, b'(', b')');
    } else {
        let start = i;
        while i < s.len() && word(s[i]) {
            i += 1;
            let exponent = s[start].is_ascii_digit() && matches!(s[i - 1], b'e' | b'E');
            if exponent && matches!(s.get(i), Some(b'-' | b'+')) {
                i += 1;
            }
        }
        if s.get(i) == Some(&b'(') {
            i = group_end(s, i, b'(', b')');
        }
    }
    while s.get(i) == Some(&b'[') {
        i = group_end(s, i, b'[', b']');
    }
    i
}

/// The divisions of a body whose right operand runs on into another `*` or `/` (`a / b * c`,
/// `a / b / c`), as `line <n>: <statement>`.
///
/// The printer spells a multiplicative child of a `/` flat (`Node::Div` prints both operands at
/// `OperandPos::MulSide`, where a `*` / `/` node takes no parentheses), so `a.div(b.mul(c))` and
/// `a.div(b).mul(c)` both print `a / b * c` — which HLSL reads as the second. The text cannot say
/// which one the trace meant, so the body never spells such a chain: a product or quotient divisor,
/// and a quotient that feeds a multiplicative operator, are materialised as named temps. Render wave
/// PX1b found the hazard live: RCAS's `hitMin = min(mn4, e) / (4·mx4)` printed `-min(…) / 4.0 * t22`,
/// i.e. `(−min / 4)·mx4` on the device, while Eval (every host oracle) computed the quotient.
fn flat_division_chains(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (n, line) in body.lines().enumerate() {
        let code = line.split_once("//").map_or(line, |(c, _)| c);
        let s = code.as_bytes();
        for (k, w) in s.windows(3).enumerate() {
            if w != b" / " {
                continue;
            }
            let mut j = operand_end(s, k + 3);
            while s.get(j) == Some(&b' ') {
                j += 1;
            }
            if matches!(s.get(j), Some(b'*' | b'/')) {
                out.push(format!("line {}: {}", n + 1, code.trim()));
            }
        }
    }
    out
}

#[test]
fn no_division_runs_on_into_a_product() {
    let body = boyko_shaderdsl::emit::emit_hlsl_post_final_main();
    let chains = flat_division_chains(&body);
    assert!(
        chains.is_empty(),
        "post_final_main spells a division whose right operand runs on into `*` / `/` — HLSL reads it \
         left-associated, while the trace may have meant the divisor as one group; materialise it: {chains:#?}"
    );
}

// ---- 5. The stage trace (OPEN-ITEMS item 4)--------------------------------------------------------------

/// The statements of a body span, one per line, trimmed.
fn statements(body: &str) -> Vec<&str> {
    body.lines().map(str::trim).filter(|l| !l.is_empty()).collect()
}

/// The `post_stage_*` declarations of a body, in program order, as `(label, statement index)`.
fn stage_declarations(stmts: &[&str]) -> Vec<(String, usize)> {
    stmts
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let rest = s.strip_prefix("float3 post_stage_")?;
            let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            Some((format!("post_stage_{name}"), i))
        })
        .collect()
}

/// Whole-word occurrence of `ident` in `s`.
fn mentions(s: &str, ident: &str) -> bool {
    s.match_indices(ident).any(|(i, _)| {
        let before = s[..i].chars().next_back();
        let after = s[i + ident.len()..].chars().next();
        let word = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        !word(before) && !word(after)
    })
}

/// 9a: the declared stage labels equal `order`, each once.
fn check_stage_order(body: &str, order: &[&str]) -> Result<(), String> {
    let declared: Vec<String> = stage_declarations(&statements(body)).into_iter().map(|(l, _)| l).collect();
    if declared != order {
        return Err(format!("the body's stage trace is {declared:?}, POST_FINAL_STAGE_ORDER says {order:?}"));
    }
    Ok(())
}

/// 9b: each stage is read after its declaration and at or before the next one's; the last one in
/// the `return`.
fn check_stage_chain(body: &str) -> Result<(), String> {
    let stmts = statements(body);
    let decls = stage_declarations(&stmts);
    if decls.is_empty() {
        return Err("the body declares no stage".to_string());
    }
    for (k, (label, at)) in decls.iter().enumerate() {
        let window_end = match decls.get(k + 1) {
            Some((_, next)) => *next,
            None => stmts.iter().rposition(|s| s.starts_with("return ")).ok_or("the body has no return")?,
        };
        let read = stmts[at + 1..=window_end].iter().any(|s| mentions(s, label));
        if !read {
            return Err(format!("`{label}` is not read before the next stage is declared — a stage computed and discarded, or the next one fed from elsewhere"));
        }
    }
    let (last, _) = decls.last().expect("non-empty");
    let ret = stmts.iter().rfind(|s| s.starts_with("return ")).ok_or("the body has no return")?;
    if !mentions(ret, last) {
        return Err(format!("the return does not read the last stage `{last}`"));
    }
    Ok(())
}

/// The documented order's labels.
fn order_labels() -> Vec<&'static str> {
    POST_FINAL_STAGE_ORDER.iter().map(|s| s.label()).collect()
}

#[test]
fn stage_trace_equals_post_final_stage_order() {
    let body = boyko_shaderdsl::emit::emit_hlsl_post_final_main();
    if let Err(why) = check_stage_order(&body, &order_labels()) {
        panic!("{why}\n--- body ---\n{body}");
    }
}

#[test]
fn every_stage_feeds_the_next() {
    let body = boyko_shaderdsl::emit::emit_hlsl_post_final_main();
    if let Err(why) = check_stage_chain(&body) {
        panic!("{why}\n--- body ---\n{body}");
    }
}

// ---- 6. Single-source discipline ---------------------------------------------------------------------------

/// The tokens that would let `post.rs` branch on its backend.
const INTROSPECTION: [&str; 6] = ["TypeId", "type_name", "core::any", "std::any", "IS_EMIT", "is_emit"];

/// `Err(token)` when `src` names a backend-introspection token.
fn introspection(src: &str) -> Result<(), String> {
    match INTROSPECTION.iter().find(|t| src.contains(**t)) {
        Some(t) => Err(format!("post.rs names `{t}`: the body must not branch on its backend")),
        None => Ok(()),
    }
}

/// `crates/boyko_shaderdsl/src/post.rs`.
fn post_rs() -> String {
    std::fs::read_to_string(repo_root().join("crates/boyko_shaderdsl/src/post.rs")).expect("invariant: post.rs exists")
}

#[test]
fn post_rs_has_no_backend_introspection() {
    if let Err(why) = introspection(&post_rs()) {
        panic!("{why}");
    }
}

/// The `pub fn` names of `post.rs` an oracle must NOT call: the STAGES `post_final_main` composes —
/// every `pub fn` its body calls, and every one those call, to a fixed point — never the traced
/// body's entry points (`post_final_main`, `post_final_eval`) or the input readers (`eval_params`,
/// `ubo_word`, and PX0's resource builder `eval_res`).
///
/// Render wave PX1a: `post.rs` also holds leaves that are NOT stages of `post_final` —
/// `lit_store_saturate` (the producers' store bound, W5: what `lit` holds BEFORE `post_final`
/// reads it), `hdr_history_guard` (TAA's history write) and `srgb_eotf` (the present blit's sRGB
/// variant). A host oracle of `lit` (`goldens.rs`) mirrors the producers' store through the first
/// and displays through `post_final_eval`; that composes no `post_final` stage, so the census
/// takes the stage set from `post_final_main`'s own call graph rather than from every `pub fn`.
fn stage_leaves(post: &str) -> Vec<String> {
    const ENTRY: [&str; 5] = ["post_final_main", "post_final_eval", "eval_params", "ubo_word", "eval_res"];
    let post = post.replace("\r\n", "\n");
    let all: Vec<String> = post
        .lines()
        .filter_map(|l| {
            // Top-level items only (an unindented `pub fn`): a method such as `PostStage::label` is
            // not a stage.
            let rest = l.strip_prefix("pub fn ").or_else(|| l.strip_prefix("pub const fn "))?;
            let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            (!ENTRY.contains(&name.as_str())).then_some(name)
        })
        .collect();
    let mut stages: Vec<String> = Vec::new();
    let mut frontier = vec![fn_body(&post, "post_final_main")];
    assert!(!frontier[0].is_empty(), "post.rs has no `post_final_main` body: the stage set is vacuous");
    while let Some(body) = frontier.pop() {
        for leaf in &all {
            if !stages.contains(leaf) && reimplements_a_stage(body, core::slice::from_ref(leaf)).is_err() {
                stages.push(leaf.clone());
                frontier.push(fn_body(&post, leaf));
            }
        }
    }
    stages
}

/// The text of the top-level `fn <name>` of `post` (LF endings), from its signature to the `}` that
/// closes it at column 0; empty when there is none.
fn fn_body<'a>(post: &'a str, name: &str) -> &'a str {
    let start = [format!("fn {name}<"), format!("fn {name}(")]
        .iter()
        .filter_map(|sig| post.find(sig.as_str()))
        .min();
    let Some(start) = start else { return "" };
    let end = post[start..].find("\n}\n").map_or(post.len(), |e| start + e + 2);
    &post[start..end]
}

/// `Err` when `text` (an oracle's source) CALLS a stage leaf — by any path (`post::aces_hill(`, a
/// glob-imported `aces_hill(`, a turbofish `aces_hill::<EvalCf>(`).
fn reimplements_a_stage(text: &str, leaves: &[String]) -> Result<(), String> {
    for leaf in leaves {
        let called = text.match_indices(leaf.as_str()).any(|(i, _)| {
            let before = text[..i].chars().next_back();
            let after = &text[i + leaf.len()..];
            let word_before = before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
            !word_before && (after.starts_with('(') || after.starts_with("::<"))
        });
        if called {
            return Err(format!("calls the stage leaf `{leaf}` — an oracle must evaluate post_final_main, not re-compose stages"));
        }
    }
    Ok(())
}

/// Every `.rs` under `dir`, recursively, skipping `target` and `tests` directories.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if n != "target" && n != "tests" && !n.starts_with('.') {
                rust_files(&p, out);
            }
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn post_eval_calls_only_the_traced_body() {
    let leaves = stage_leaves(&post_rs());
    assert!(leaves.len() >= 8, "the leaf list is vacuous: {leaves:?}");
    // Armed by CONTENT, not by a path: every non-test source that instantiates the traced body on
    // the host (PX0's `post_eval`, wherever it lives), plus any file named `post_eval.rs`.
    let mut files = Vec::new();
    rust_files(&repo_root().join("crates"), &mut files);
    rust_files(&repo_root().join("src"), &mut files);
    let post_path = repo_root().join("crates/boyko_shaderdsl/src/post.rs");
    let mut armed = Vec::new();
    for f in &files {
        if f.canonicalize().ok() == post_path.canonicalize().ok() {
            continue;
        }
        let text = std::fs::read_to_string(f).unwrap_or_default();
        let is_oracle = f.file_name().is_some_and(|n| n == "post_eval.rs")
            || text.contains("post_final_main::<EvalCf>")
            || text.contains("post_final_eval(");
        if is_oracle {
            if let Err(why) = reimplements_a_stage(&text, &leaves) {
                panic!("{}: {why}", f.display());
            }
            armed.push(f.display().to_string());
        }
    }
    // REQUIRED-armed since render wave PX0 landed `post_eval` (the PXe cut's residual): a census
    // that arms nothing proves nothing, so an empty set is red, never a note.
    assert!(
        armed.iter().any(|f| f.ends_with("post_eval.rs")),
        "post_eval census: NOT ARMED — no `post_eval.rs` among the host callers of the traced body {armed:?}"
    );
    eprintln!("post_eval census: ARMED over {armed:?}");
}

#[test]
fn the_censuses_reject_their_fixtures() {
    // 9a / 9b on mis-composed bodies.
    let good = "float4 post_scene = gScene.Load(int3((int)px.x, (int)px.y, 0));\n\
                float3 post_stage_a = post_scene.xyz;\n\
                float3 post_stage_b = post_stage_a * 2.0;\n\
                return float4(post_stage_b.x, post_stage_b.y, post_stage_b.z, 1.0);\n";
    assert!(check_stage_order(good, &["post_stage_a", "post_stage_b"]).is_ok());
    assert!(check_stage_chain(good).is_ok());
    assert!(check_stage_order(good, &["post_stage_b", "post_stage_a"]).is_err(), "swapped stages must be red");
    assert!(check_stage_order(good, &["post_stage_a"]).is_err(), "a marker without a list row must be red");
    assert!(check_stage_order(good, &["post_stage_a", "post_stage_b", "post_stage_c"]).is_err(), "a row without a marker must be red");
    let discarded = good.replace("post_stage_a * 2.0", "post_scene.xyz * 2.0");
    assert!(check_stage_chain(&discarded).is_err(), "a stage computed and discarded must be red");
    let unreturned = good.replace("return float4(post_stage_b.x, post_stage_b.y, post_stage_b.z, 1.0)", "return float4(post_stage_a.x, post_stage_a.y, post_stage_a.z, 1.0)");
    assert!(check_stage_chain(&unreturned).is_err(), "a return that skips the last stage must be red");
    // The skeleton census.
    let skel_ok = "void main(uint3 dtid) {\n if (any(dtid.xy >= extent)) { return; }\n gDisplay[dtid.xy] = post_final_main(dtid.xy);\n}\n";
    assert!(skeleton_violations(skel_ok).is_ok());
    assert!(skeleton_violations(&skel_ok.replace("post_final_main(dtid.xy)", "post_final_main(dtid.xy) + gScene.Load(int3(0, 0, 0))")).is_err());
    assert!(skeleton_violations(&skel_ok.replace("dtid.xy >= extent", "dtid.xy + 1u >= extent")).is_err());
    // D-P13.
    assert!(introspection("fn f<C: Cf>() { if core::any::TypeId::of::<C>() == x {} }").is_err());
    // H2 test 10 — the qualified, the glob-imported and the turbofish calls are all flagged; a body
    // naming only the traced entry point passes.
    let leaves = vec!["aces_hill".to_string(), "hdr_sanitize".to_string()];
    assert!(reimplements_a_stage("let o = post::aces_hill(c);", &leaves).is_err());
    // PX1a: the stage set is `post_final_main`'s call graph — the curves (reached through
    // `tonemap_select`), the sanitize and the OETF are in; the producers' store bound, TAA's history
    // guard and the present's sRGB decode are not.
    let stages = stage_leaves(&post_rs());
    for s in ["hdr_sanitize", "pre_ratio_apply", "tonemap_select", "aces_hill", "khronos_pbr_neutral", "reinhard_jodie", "oetf_gamma22"] {
        assert!(stages.iter().any(|l| l == s), "`{s}` is a stage of post_final_main: {stages:?}");
    }
    // Render wave PX1b (critique O2): the RCAS stage is a top-level `pub fn`, so its leaves are in the
    // set too — an oracle that re-composes the sharpen stage is flagged like one that re-composes a
    // curve.
    for s in ["rcas_stage", "tonemap_reversible", "tonemap_reversible_inverse", "rcas_lobe", "rcas_resolve"] {
        assert!(stages.iter().any(|l| l == s), "`{s}` is a stage of post_final_main: {stages:?}");
    }
    assert!(reimplements_a_stage("let l = rcas_lobe::<EvalCf>(b, d, e, f, h, con);", &stages).is_err());
    // The fetch-structure census (PX1b D-R10 (2)): the scene fetched once before the sharpen
    // selection and the cross inside it; a flattened RCAS (five fetches before any selection), a
    // body without the cross and one that fetches after the selection are each red.
    let fetch = |id: u32| format!("%{id} = OpImageFetch %v4float %img %c{id} Lod %int_0\n");
    let selection = "OpSelectionMerge %90 DontFlatten\nOpBranchConditional %cond %90 %91\n%91 = OpLabel\n";
    let merged = "OpBranch %90\n%90 = OpLabel\nOpSelectionMerge %95 DontFlatten\nOpBranchConditional %t %93 %94\n%95 = OpLabel\n";
    let good = format!("{}{selection}{}{}{}{}{merged}", fetch(1), fetch(2), fetch(3), fetch(4), fetch(5));
    assert_eq!(check_fetch_structure(&good), Ok(()));
    let flattened = format!("{}{}{}{}{}{merged}", fetch(1), fetch(2), fetch(3), fetch(4), fetch(5));
    assert!(check_fetch_structure(&flattened).is_err(), "five fetches before the first DontFlatten selection must be red");
    let crossless = format!("{}{selection}{merged}", fetch(1));
    assert!(check_fetch_structure(&crossless).is_err(), "a body without the cross must be red");
    let late = format!("{}{selection}{}{}{}{merged}{}", fetch(1), fetch(2), fetch(3), fetch(4), fetch(5));
    assert!(check_fetch_structure(&late).is_err(), "a tap fetched after the sharpen selection must be red");
    assert!(check_fetch_structure("%1 = OpImageFetch %v4float %img %c Lod %int_0\n").is_err(), "no DontFlatten selection must be red");
    // The printed-association census (PX1b): a product or quotient run on after a divisor is red,
    // whatever the divisor's shape; a materialised or parenthesised divisor and a division ended by
    // `+` / `,` / `)` / `;` pass.
    for red in [
        "float t23 = max(((t22 > 0.0) ? ((-min(t21, e.x) / 4.0 * t22)) : (-0.25)), h);",
        "float t = a / b / c;",
        "float t = x.y / f(a, b) * c[2];",
        "float t = 1.0 / (4.0 * l + 1.0) * w;",
        "float t = a / 1.5e-5 * c;",
    ] {
        assert_eq!(flat_division_chains(red).len(), 1, "`{red}` must be red");
    }
    let flat_ok = "float t = -min(a, b) / t4;\nfloat u = (1.0 - m) / (4.0 * n - 4.0);\nfloat v = max(a / b, c) * (d / e);\n\
                   float w = a * b / c + 1.0 / d; // a / b * c in a comment\n";
    assert_eq!(flat_division_chains(flat_ok), Vec::<String>::new(), "grouped divisors must pass");
    for s in ["lit_store_saturate", "hdr_history_guard", "srgb_eotf"] {
        assert!(!stages.iter().any(|l| l == s), "`{s}` is not a stage of post_final_main: {stages:?}");
    }
    assert!(reimplements_a_stage("let l = lit_store_saturate::<EvalCf>(c); let o = post_final_eval(px, &p, t);", &stages).is_ok());
    assert!(reimplements_a_stage("let l = lit_store_saturate::<EvalCf>(c); let o = reinhard_jodie::<EvalCf>(l);", &stages).is_err());
    assert!(reimplements_a_stage("use boyko_shaderdsl::post::*;\nlet o = aces_hill(c);", &leaves).is_err());
    assert!(reimplements_a_stage("let o = hdr_sanitize::<EvalCf>(c);", &leaves).is_err());
    assert!(reimplements_a_stage("let o = post_final_main::<EvalCf>(px, &p, &r, &cell); // not aces_hill_x(", &leaves).is_ok());
    // The SPIR-V parsers.
    let gamma = OETF_GAMMA_EXP;
    let dis = format!(
        "%9 = OpConstant %float {gamma}\n%7 = OpConstant %float 2.4000001\n\
         %20 = OpExtInst %float %1 Pow %10 %9\n%21 = OpExtInst %float %1 Pow %11 %9\n\
         %22 = OpExtInst %float %1 Pow %12 %7\n%23 = OpSelect %v3float %c %a %b\n\
         OpSelectionMerge %30 DontFlatten\nOpSelectionMerge %31 None\n"
    );
    assert_eq!(pow_census(&dis, gamma), (2, 3, 0, 0), "a Pow on another exponent is not an OETF pow");
    assert_eq!(pow_census(&dis, 2.4).0, 1, "the exponent census counts the Pow on its own constant only");
    assert_eq!(branch_census(&dis), (1, 1), "the branch census must see the DontFlatten and the float3 select");
    // Render wave PXt — the AgX placement census: AgX's Log2 (reading a max against the floor) and
    // its Pow on 2.2 inside the second DontFlatten selection pass; a Log2 or a Pow before the
    // dispatch, a Log2 without the floor and a module without either are each red.
    let consts = format!("%f = OpConstant %float {AGX_LOG_FLOOR:e}\n%e = OpConstant %float {AGX_LINEARIZE_EXP}\n%g = OpConstant %float 0.5\n");
    let sharpen = "OpSelectionMerge %90 DontFlatten\nOpBranchConditional %s %91 %90\n%91 = OpLabel\nOpBranch %90\n%90 = OpLabel\n";
    let arm = "%40 = OpExtInst %float %1 NMax %39 %f\n%41 = OpExtInst %float %1 Log2 %40\n%42 = OpExtInst %float %1 Pow %43 %e\n";
    let dispatch = |body: &str| format!("OpSelectionMerge %95 DontFlatten\nOpBranchConditional %t %93 %94\n%93 = OpLabel\n{body}OpBranch %95\n%94 = OpLabel\nOpBranch %95\n%95 = OpLabel\n");
    assert_eq!(check_agx_placement(&format!("{consts}{sharpen}{}", dispatch(arm)), true), Ok(()));
    assert!(check_agx_placement(&format!("{consts}{arm}{sharpen}{}", dispatch("")), true).is_err(), "a Log2 before the sharpen stage must be red");
    assert!(check_agx_placement(&format!("{consts}{sharpen}{arm}{}", dispatch("")), true).is_err(), "a Log2 between the sharpen stage and the dispatch must be red");
    let unfloored = arm.replace("NMax %39 %f", "NMax %39 %g");
    assert!(check_agx_placement(&format!("{consts}{sharpen}{}", dispatch(&unfloored)), true).is_err(), "a Log2 without the 1e-10 floor must be red");
    assert!(check_agx_placement(&format!("{consts}{sharpen}{}", dispatch("")), true).is_err(), "a module without AgX's transcendentals must be red");
    // The sharpen-off module: the dispatch is the first selection.
    assert_eq!(check_agx_placement(&format!("{consts}{}", dispatch(arm)), false), Ok(()));
    assert!(check_agx_placement(&format!("{consts}{arm}{}", dispatch("")), false).is_err(), "a Log2 before the sharpen-off dispatch must be red");
    // The sharpen-off fetch census: one fetch before every selection passes; a ring tap (a second
    // fetch) or a fetch inside a selection is red.
    let one = format!("{}{}", fetch(1), dispatch(""));
    assert_eq!(check_nosharpen_fetches(&one), Ok(()));
    assert!(check_nosharpen_fetches(&format!("{}{}{}", fetch(1), fetch(2), dispatch(""))).is_err(), "a second fetch must be red");
    assert!(check_nosharpen_fetches(&dispatch(&fetch(1))).is_err(), "a fetch inside a selection must be red");
}

// ---- 7. Render wave PXt — the sharpen-off module (OPEN-ITEMS 19) ------------------------------------

/// The committed sharpen-off module's stem.
const NOSHARPEN_STEM: &str = "post_final_nosharpen.comp";

/// `Err(why)` unless the module carries exactly ONE `OpImageFetch` (the scene texel), before every
/// `DontFlatten` selection — the sharpen-off module, whose RCAS branch DXC folded away.
fn check_nosharpen_fetches(dis: &str) -> Result<(), String> {
    let lines = token_lines(dis);
    let fetches: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, t)| t.len() >= 3 && t[1] == "=" && t[2] == "OpImageFetch")
        .map(|(i, _)| i)
        .collect();
    let first_selection = lines.iter().position(|t| t.first() == Some(&"OpSelectionMerge") && t.last() == Some(&"DontFlatten"));
    match (fetches.as_slice(), first_selection) {
        ([f], Some(s)) if *f < s => Ok(()),
        ([f], None) => Err(format!("one OpImageFetch (line {f}) but no DontFlatten selection: the tonemapper dispatch is not a real branch")),
        (f, _) => Err(format!("OpImageFetch at lines {f:?}: the sharpen-off module must fetch exactly once, before every selection")),
    }
}

#[test]
fn nosharpen_committed_source_is_the_generator_output() {
    let committed = read_shader(&format!("{NOSHARPEN_STEM}.hlsl"));
    let generated = boyko_shaderdsl::emit::emit_post_final_nosharpen_comp();
    assert!(
        committed == generated,
        "{NOSHARPEN_STEM}.hlsl DRIFTED from boyko_shaderdsl::emit::emit_post_final_nosharpen_comp — re-run `cargo run -p \
         boyko_shaderdsl --features emit --bin emit_post`, re-DXC the .spv with the recipe in the \
         shader's header, and commit both.\n--- generated ---\n{generated}\n--- committed ---\n{committed}"
    );
    // The two committed modules trace one body: the sharpen-off one differs from post_final.comp
    // only by its header and by the masked mode word.
    let uber = read_shader(&format!("{STEM}.hlsl"));
    assert_eq!(
        generated_span(&committed, "post_final_bindings"),
        generated_span(&uber, "post_final_bindings"),
        "the two modules must declare the same bindings (one set layout serves both)"
    );
    assert!(
        committed.contains("uint post_mode_words = mode_words & 4294967293u;"),
        "the sharpen-off module masks the sharpen bit off the runtime word"
    );
}

#[test]
fn nosharpen_header_pins_the_frozen_recipe() {
    let src = read_shader(&format!("{NOSHARPEN_STEM}.hlsl"));
    let recipe = "C:\\VulkanSDK\\1.4.350.0\\Bin\\dxc.exe -spirv -T cs_6_0 -E main \\\n\
                  //       -fspv-target-env=vulkan1.3 post_final_nosharpen.comp.hlsl -Fo post_final_nosharpen.comp.spv";
    assert!(src.contains(recipe), "{NOSHARPEN_STEM}.hlsl's header no longer pins the recipe this gate runs");
}

#[test]
fn post_final_nosharpen_spv_byte_identical() {
    let Some(dxc) = find_tool("dxc") else {
        eprintln!("SKIP post_final_nosharpen_spv_byte_identical: no dxc on this host");
        return;
    };
    let committed_path = shaders_dir().join(format!("{NOSHARPEN_STEM}.spv"));
    let committed = std::fs::read(&committed_path)
        .unwrap_or_else(|e| panic!("missing committed {}: {e}", committed_path.display()));
    let out_spv = child_guard::scratch_path(&format!("{NOSHARPEN_STEM}.redxc.spv"));
    let status = Command::new(&dxc)
        .current_dir(shaders_dir())
        .args(["-spirv", "-T", "cs_6_0", "-E", "main", "-fspv-target-env=vulkan1.3"])
        .arg(format!("{NOSHARPEN_STEM}.hlsl"))
        .arg("-Fo")
        .arg(&*out_spv)
        .status_within(DXC_DEADLINE)
        .expect("invariant: dxc was located and must run");
    assert!(status.success(), "dxc failed re-compiling {NOSHARPEN_STEM}.hlsl under the frozen recipe");
    let fresh = std::fs::read(&*out_spv).expect("invariant: dxc wrote the re-DXC .spv");
    assert!(
        committed == fresh,
        "{NOSHARPEN_STEM}.spv ({} bytes committed, {} fresh) is NOT the re-DXC of {NOSHARPEN_STEM}.hlsl under the \
         frozen recipe — the committed .spv is stale, or the host dxc is not VulkanSDK 1.4.350.0.",
        committed.len(),
        fresh.len()
    );
}

#[test]
fn nosharpen_spirv_censuses() {
    let Some(dis) = disassembly_of(NOSHARPEN_STEM, "nosharpen_spirv_censuses") else { return };
    let (gamma_pows, pows, exp2, log2) = pow_census(&dis, OETF_GAMMA_EXP);
    let (agx_pows, _, _, _) = pow_census(&dis, AGX_LINEARIZE_EXP);
    assert_eq!(
        (gamma_pows, agx_pows, pows, exp2, log2),
        (3, 3, 6, 0, 3),
        "the sharpen-off module carries the OETF's three Pow, AgX's three Pow on 2.2 and three Log2, no Exp2"
    );
    let (dont_flatten, v3_selects) = branch_census(&dis);
    assert_eq!(dont_flatten, 3, "the sharpen-off module's only DontFlatten selections are the three-level dispatch (DXC folded the sharpen stage)");
    assert_eq!(v3_selects, 0, "a float3 OpSelect means a branch was if-converted");
    if let Err(why) = check_nosharpen_fetches(&dis) {
        panic!("{why}");
    }
    if let Err(why) = check_agx_placement(&dis, false) {
        panic!("{why}");
    }
    let decorated = binding_decorations(&dis);
    for b in POST_FINAL_BINDINGS {
        assert_eq!(decorated.get(b.name), Some(&(b.set, b.binding)), "{NOSHARPEN_STEM}: `{}` is not at (set {}, binding {})", b.name, b.set, b.binding);
    }
}

#[test]
fn nosharpen_stage_trace_and_skeleton() {
    let src = read_shader(&format!("{NOSHARPEN_STEM}.hlsl"));
    let body = generated_span(&src, "post_final_main")
        .strip_prefix("float4 post_final_main(uint2 px) {\n")
        .and_then(|b| b.strip_suffix("}\n"))
        .expect("invariant: the post_final_main span wraps one function");
    if let Err(why) = check_stage_order(body, &order_labels()) {
        panic!("{NOSHARPEN_STEM}: {why}");
    }
    if let Err(why) = check_stage_chain(body) {
        panic!("{NOSHARPEN_STEM}: {why}");
    }
    if let Err(why) = skeleton_violations(&skeleton_code(&src)) {
        panic!("{NOSHARPEN_STEM}: {why}");
    }
}

// `status_within` / `output_within` and `scratch_path` live in `tests/child_guard/mod.rs`; declared
// last so no cited line above moves.
mod child_guard;
use child_guard::{BoundedRun, DXC_DEADLINE};
