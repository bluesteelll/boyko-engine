//! **RD6 / D-DS — H7 and the VB dark-path census.**
//!
//! 1. **The splices are the generator's output (H7).** `forward_opaque.fs.hlsl`,
//!    `gbuffer_mrt.fs.hlsl` and `vb_geom_fetch.hlsli` carry the `mesh_face_consts` and
//!    `mesh_face_flip` spans, and `vb_geom_fetch.hlsli` the `vb_screen_front_ccw` span, each
//!    byte-equal to `boyko_shaderdsl::emit` re-run now. A hand-edit of a spliced span reds here.
//!    Fix: `cargo run -p boyko_shaderdsl --features emit --bin emit_mesh_face`, re-splice, re-DXC.
//! 2. **One call per producer, at the documented site.** Each producer calls `mesh_face_flip(`
//!    exactly once; the Deferred producer calls it on the FINAL normal — after the `TEXTURED`
//!    TBN block, immediately before `oct_encode` (negating the geometric normal before the TBN
//!    would give the reflection through the plane, not the back side).
//! 3. **The VB facing math stays behind the double-sided branch (critique W6).** In every
//!    committed `.spv` that includes `vb_geom_fetch.hlsli`, the facing chain — the `OpSelect
//!    %uint … %uint_1 %uint_0` of `vb_screen_front_ccw` and the `OpBitwiseXor`s of
//!    `mesh_face_flip` — sits inside the conditional region of the `DontFlatten` branch on
//!    `flags & MESH_FACE_DOUBLE_SIDED`, so a frame with no double-sided instance pays one AND and
//!    a coherent branch, never the chain (the dark-cost rule `vb_geom_fetch.hlsli`'s SV0 block
//!    records). The census runs on `spirv-dis` output and is shown able to fail on a mutant whose
//!    facing call is hoisted above the branch.
//!
//! Gates 1 and 2 need no tool. Gate 3 SKIPS (with an eprintln) when `dxc` / `spirv-dis` do not
//! resolve on the host, exactly like the `*_spv_sync` precedents.

use std::path::{Path, PathBuf};
use std::process::Command;

fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

fn source(name: &str) -> String {
    std::fs::read_to_string(shaders_dir().join(name))
        .unwrap_or_else(|e| panic!("invariant: shaders/{name} is committed: {e}"))
        .replace("\r\n", "\n")
}

/// The text between `// === GENERATED <name> BEGIN === (boyko_shaderdsl::face)` and its END line.
fn span(src: &str, name: &str) -> String {
    let begin = format!("// === GENERATED {name} BEGIN === (boyko_shaderdsl::face)\n");
    let end = format!("// === GENERATED {name} END ===");
    let i = src.find(&begin).unwrap_or_else(|| panic!("the `{name}` BEGIN sentinel is missing")) + begin.len();
    let j = src[i..].find(&end).unwrap_or_else(|| panic!("the `{name}` END sentinel is missing")) + i;
    src[i..j].to_string()
}

const PRODUCERS: [&str; 3] = ["forward_opaque.fs.hlsl", "gbuffer_mrt.fs.hlsl", "vb_geom_fetch.hlsli"];

/// Gate 1 (H7): every committed splice is the generator's output, byte for byte.
#[test]
fn producer_spans_match_the_generator() {
    let consts = boyko_shaderdsl::emit::emit_hlsl_mesh_face_consts();
    let flip = boyko_shaderdsl::emit::emit_hlsl_mesh_face_flip();
    let front = boyko_shaderdsl::emit::emit_hlsl_vb_screen_front_ccw();
    for name in PRODUCERS {
        let src = source(name);
        assert_eq!(span(&src, "mesh_face_consts"), consts, "{name}: the mesh_face_consts splice drifted");
        assert_eq!(span(&src, "mesh_face_flip"), flip, "{name}: the mesh_face_flip splice drifted");
    }
    assert_eq!(
        span(&source("vb_geom_fetch.hlsli"), "vb_screen_front_ccw"),
        front,
        "vb_geom_fetch.hlsli: the vb_screen_front_ccw splice drifted"
    );
}

/// Gate 2 (H7): one `mesh_face_flip(` call per producer (the definition aside), and the Deferred
/// producer's sits on the final normal, right before the encode.
#[test]
fn each_producer_orients_its_final_normal_once() {
    for name in PRODUCERS {
        let src = source(name);
        let calls = src.matches("mesh_face_flip(").count() - src.matches("uint mesh_face_flip(").count();
        assert_eq!(calls, 1, "{name}: expected exactly one mesh_face_flip call site, found {calls}");
    }
    let gbuffer = source("gbuffer_mrt.fs.hlsl");
    let call = gbuffer.find("n = (mesh_face_flip(").expect("the Deferred producer orients `n`");
    let encode = gbuffer.find("float2 oct = oct_encode(n);").expect("the Deferred producer encodes `n`");
    let tbn = gbuffer.find("n = normalize(T * n_ts.x + B * n_ts.y + N * n_ts.z);").expect("the TEXTURED TBN line");
    assert!(tbn < call && call < encode, "the Deferred flip must follow the TBN block and precede oct_encode");
    let between = gbuffer[call..encode].trim_end();
    assert_eq!(between.lines().count(), 1, "nothing may sit between the flip and the encode:\n{between}");
    let forward = source("forward_opaque.fs.hlsl");
    let n_line = forward.find("float3 n = normalize(input.normal);").expect("the Forward normal");
    let fcall = forward.find("n = (mesh_face_flip(").expect("the Forward producer orients `n`");
    let first_use = forward.find("float NoV = max(dot(n, v), 1e-4);").expect("the Forward NoV");
    assert!(n_line < fcall && fcall < first_use, "the Forward flip must precede every lighting use of `n`");
    let vb = source("vb_geom_fetch.hlsli");
    assert!(
        vb.contains("[branch] if ((inst.flags & MESH_FACE_DOUBLE_SIDED) != 0u) {"),
        "the VB orientation must be gated on the instance's double-sided bit"
    );
}

// --- Gate 3: the SPIR-V dark-path census ----------------------------------------------------------

/// Every committed `.spv` whose source includes `vb_geom_fetch.hlsli` (`rg -l` over the shaders
/// directory finds five sources: vb_resolve, vb_shade, vb_shade_split, vb_geo, sdf_mesh_shadow).
const VB_FETCH_SPV: [&str; 14] = [
    "vb_resolve.comp.spv",
    "vb_resolve_froxel.comp.spv",
    "vb_shade.comp.spv",
    "vb_shade_tex.comp.spv",
    "vb_shade_froxel.comp.spv",
    "vb_shade_tex_froxel.comp.spv",
    "vb_shade_split.comp.spv",
    "vb_shade_split_tex.comp.spv",
    "vb_shade_split_hwrt.comp.spv",
    "vb_shade_split_tex_hwrt.comp.spv",
    "vb_geo.comp.spv",
    "vb_geo_mv.comp.spv",
    "vb_geo_sv0.comp.spv",
    "sdf_mesh_shadow.comp.spv",
];

fn find_tool(exe: &str) -> Option<PathBuf> {
    let pinned = PathBuf::from("C:/VulkanSDK/1.4.350.0/Bin").join(exe);
    if pinned.exists() {
        return Some(pinned);
    }
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let candidate = PathBuf::from(sdk).join("Bin").join(exe);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn disassemble(spirv_dis: &Path, spv: &Path) -> String {
    let out = Command::new(spirv_dis)
        .arg(spv)
        .output_within(DXC_DEADLINE)
        .expect("invariant: spirv-dis was located and must run");
    assert!(out.status.success(), "spirv-dis failed on {}", spv.display());
    String::from_utf8(out.stdout).expect("spirv-dis prints UTF-8")
}

/// The census verdict for one disassembly: `Ok(())` when the facing chain lives only inside the
/// double-sided branch's region, else the reason.
fn dark_path_census(dis: &str) -> Result<(), String> {
    let lines: Vec<&str> = dis.lines().map(str::trim).collect();
    // The gate: `%a = OpBitwiseAnd %uint %flags %uint_4`, `%c = OpINotEqual %bool %a %uint_0`,
    // `OpSelectionMerge %M DontFlatten`, `OpBranchConditional %c %T %M`.
    let mut region = None;
    for w in lines.windows(4) {
        let Some((and_id, and_rhs)) = w[0].split_once(" = OpBitwiseAnd %uint ") else { continue };
        if !and_rhs.ends_with(" %uint_4") {
            continue;
        }
        let Some((cond_id, cond_rhs)) = w[1].split_once(" = OpINotEqual %bool ") else { continue };
        if cond_rhs != format!("{and_id} %uint_0") {
            continue;
        }
        let Some(merge) = w[2].strip_prefix("OpSelectionMerge ").and_then(|r| r.strip_suffix(" DontFlatten")) else {
            continue;
        };
        let Some(branch) = w[3].strip_prefix(&format!("OpBranchConditional {cond_id} ")) else { continue };
        let Some((taken, not_taken)) = branch.split_once(' ') else { continue };
        if not_taken != merge {
            continue;
        }
        let start = lines.iter().position(|l| *l == format!("{taken} = OpLabel"));
        let end = lines.iter().position(|l| *l == format!("{merge} = OpLabel"));
        if let (Some(s), Some(e)) = (start, end)
            && region.replace((s, e)).is_some()
        {
            return Err("more than one double-sided gate".into());
        }
    }
    let Some((s, e)) = region else {
        return Err("no `DontFlatten` branch on `flags & MESH_FACE_DOUBLE_SIDED` found".into());
    };
    let is_front_select = |l: &&str| l.contains(" = OpSelect %uint ") && l.ends_with(" %uint_1 %uint_0");
    let is_xor = |l: &&str| l.contains(" = OpBitwiseXor %uint ");
    let inside = &lines[s..e];
    if !inside.iter().any(is_front_select) || !inside.iter().any(is_xor) {
        return Err("the facing chain is not inside the double-sided region".into());
    }
    if !inside.iter().any(|l| l.contains(" = OpFNegate %v3float ")) {
        return Err("no normal negation inside the double-sided region".into());
    }
    let outside = lines[..s].iter().chain(&lines[e..]);
    let leaked: Vec<&&str> = outside.filter(|l| is_front_select(l) || is_xor(l)).collect();
    if !leaked.is_empty() {
        return Err(format!("facing ops hoisted out of the double-sided region: {leaked:?}"));
    }
    Ok(())
}

/// Gate 3: every committed VB-fetch `.spv` keeps the facing chain behind the branch.
#[test]
fn vb_facing_math_stays_behind_the_double_sided_branch() {
    let Some(spirv_dis) = find_tool("spirv-dis.exe").or_else(|| find_tool("spirv-dis")) else {
        eprintln!("mesh_face_edsl_sync: spirv-dis not found — SKIPPING the SPIR-V census on this host.");
        return;
    };
    for spv in VB_FETCH_SPV {
        let dis = disassemble(&spirv_dis, &shaders_dir().join(spv));
        if let Err(why) = dark_path_census(&dis) {
            panic!("{spv}: {why}");
        }
    }
}

/// Gate 3's anti-vacuity: a mutant `vb_geom_fetch.hlsli` that computes the facing BEFORE the
/// branch (the hoist the dark-cost rule forbids) must fail the census.
#[test]
fn the_census_rejects_a_hoisted_facing_chain() {
    let (Some(dxc), Some(spirv_dis)) = (find_tool("dxc.exe").or_else(|| find_tool("dxc")), find_tool("spirv-dis.exe").or_else(|| find_tool("spirv-dis"))) else {
        eprintln!("mesh_face_edsl_sync: dxc / spirv-dis not found — SKIPPING the census mutant on this host.");
        return;
    };
    let gate = "    [branch] if ((inst.flags & MESH_FACE_DOUBLE_SIDED) != 0u) {\n        uint front_ccw = vb_screen_front_ccw(float3(sx0, sx1, sx2), float3(sy0, sy1, sy2));\n";
    let hoisted = "    uint front_ccw = vb_screen_front_ccw(float3(sx0, sx1, sx2), float3(sy0, sy1, sy2));\n    uint hoisted_flip = mesh_face_flip(front_ccw, inst.flags);\n    [branch] if ((inst.flags & MESH_FACE_DOUBLE_SIDED) != 0u) {\n";
    let fetch = source("vb_geom_fetch.hlsli");
    assert_eq!(fetch.matches(gate).count(), 1, "the mutant's anchor is the committed gate");
    let mutant = fetch
        .replace(gate, hoisted)
        .replace("        if (mesh_face_flip(front_ccw, inst.flags) != 0u) {", "        if (hoisted_flip != 0u) {");
    // The mutant header and a copy of the including source live in a scratch directory with the
    // other includes resolved from the committed shaders directory (`-I`).
    let scratch = child_guard::scratch_path("mesh_face_census_mutant");
    let dir = scratch.to_path_buf();
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::fs::write(dir.join("vb_geom_fetch.hlsli"), &mutant).expect("write the mutant header");
    let main_src = dir.join("vb_resolve.comp.hlsl");
    std::fs::write(&main_src, source("vb_resolve.comp.hlsl")).expect("copy the includer");
    let out = dir.join("mutant.spv");
    let status = Command::new(&dxc)
        .current_dir(&dir)
        .args(["-spirv", "-T", "cs_6_0", "-E", "main", "-fspv-target-env=vulkan1.3", "-I"])
        .arg(shaders_dir())
        .arg("vb_resolve.comp.hlsl")
        .arg("-Fo")
        .arg(&out)
        .status_within(DXC_DEADLINE)
        .expect("invariant: dxc runs");
    assert!(status.success(), "dxc compiles the hoisted mutant");
    let dis = disassemble(&spirv_dis, &out);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        dark_path_census(&dis).is_err(),
        "RED (vacuous census): the hoisted mutant passed — the census cannot see a facing chain \
         outside the double-sided branch"
    );
}

// `status_within` / `output_within` (a deadline on each tool this file spawns) and `scratch_path`
// live in `tests/child_guard/mod.rs`, whose module doc records the hung-dxc stall behind them.
mod child_guard;
use child_guard::{BoundedRun, DXC_DEADLINE};
