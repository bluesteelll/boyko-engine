//! The E0 primitive probe kernel's eDSL ↔ HLSL ↔ `.spv` guard (render wave E0, test-only kernel).
//!
//! Three layers, each failing independently:
//!
//! 1. **Generator sync** — the committed `e0_prims_probe.comp.hlsl` IS
//!    `boyko_shaderdsl::emit::emit_e0_prims_probe_comp()`, whole.
//! 2. **`.spv` byte identity** — the committed `.spv` is the re-DXC of its source under the frozen
//!    recipe its header pins. Prints a `SKIP` line when no `dxc` resolves: a skipped run is NOT a
//!    pass (the byte gate is then vacuous).
//! 3. **Lowering census** — the rung's "each op lowers to one GLSL.std.450 instruction" made a gate:
//!    `spirv-dis` of the committed module carries exactly one `OpExtInst` each of `Floor`, `Fract`,
//!    `Log2`, `Exp2`, `Pow` and no other extended instruction (a `Trunc` where `Floor` belongs, or
//!    `pow` expanded into `Exp2`/`Log2`, is red). The census function is run against fixture
//!    disassemblies of exactly those two mis-lowerings on every run, so it is shown able to fail.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use boyko_shaderdsl::prims_probe::E0Op;

/// The committed shader's stem.
const STEM: &str = "e0_prims_probe.comp";

fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Reads a committed shader source, LF-normalized (a CRLF checkout must not mismatch the LF
/// generator output).
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

// ---- Layer 1: generator sync -------------------------------------------------------------------

#[test]
fn committed_source_is_the_generator_output() {
    let committed = read_shader(&format!("{STEM}.hlsl"));
    let generated = boyko_shaderdsl::emit::emit_e0_prims_probe_comp();
    assert!(
        committed == generated,
        "{STEM}.hlsl DRIFTED from boyko_shaderdsl::emit::emit_e0_prims_probe_comp — re-run `cargo \
         run -p boyko_shaderdsl --features emit --bin emit_e0_probe`, re-DXC the .spv with the \
         recipe in the shader's header, and commit both.\n--- generated ---\n{generated}\n--- \
         committed ---\n{committed}"
    );
}

#[test]
fn header_pins_the_frozen_recipe() {
    let src = read_shader(&format!("{STEM}.hlsl"));
    let recipe = "C:\\VulkanSDK\\1.4.350.0\\Bin\\dxc.exe -spirv -T cs_6_0 -E main \\\n\
                  //       -fspv-target-env=vulkan1.3 e0_prims_probe.comp.hlsl -Fo e0_prims_probe.comp.spv";
    assert!(src.contains(recipe), "{STEM}.hlsl's header no longer pins the recipe this gate runs");
}

// ---- Layer 2: `.spv` byte identity ---------------------------------------------------------------

#[test]
fn e0_prims_probe_spv_byte_identical() {
    let Some(dxc) = find_tool("dxc") else {
        eprintln!("SKIP e0_prims_probe_spv_byte_identical: no dxc on this host");
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

// ---- Layer 3: the lowering census ------------------------------------------------------------------

/// Every `OpExtInst` of a `spirv-dis` disassembly by instruction name (`%r = OpExtInst %type %set
/// <Name> …` — whole-token match, so `OpExtInstImport` is not counted).
fn ext_inst_census(dis: &str) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for line in dis.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.get(1) == Some(&"=") && toks.get(2) == Some(&"OpExtInst")
            && let Some(name) = toks.get(5)
        {
            *out.entry((*name).to_string()).or_insert(0) += 1;
        }
    }
    out
}

/// The census the probe must have: one instruction per op, nothing else.
fn expected_census() -> BTreeMap<String, usize> {
    E0Op::ALL.iter().map(|op| (op.glsl_std_450().to_string(), 1)).collect()
}

#[test]
fn the_lowering_census_rejects_a_mislowered_module() {
    let good = "%5 = OpExtInst %float %1 Floor %4\n%6 = OpExtInst %float %1 Fract %4\n\
                %7 = OpExtInst %float %1 Log2 %4\n%8 = OpExtInst %float %1 Exp2 %4\n\
                %9 = OpExtInst %float %1 Pow %4 %3\n%1 = OpExtInstImport \"GLSL.std.450\"";
    assert_eq!(ext_inst_census(good), expected_census(), "the census misreads a correct module");
    let trunc_floor = good.replace(" Floor ", " Trunc ");
    assert_ne!(ext_inst_census(&trunc_floor), expected_census(), "a Trunc floor must be red");
    let expanded_pow = good.replace(
        "%9 = OpExtInst %float %1 Pow %4 %3",
        "%9 = OpExtInst %float %1 Log2 %4\n%10 = OpExtInst %float %1 Exp2 %9",
    );
    assert_ne!(ext_inst_census(&expanded_pow), expected_census(), "an expanded pow must be red");
}

#[test]
fn each_op_lowers_to_one_glsl_std_450_instruction() {
    let Some(spirv_dis) = find_tool("spirv-dis") else {
        eprintln!("SKIP each_op_lowers_to_one_glsl_std_450_instruction: no spirv-dis on this host");
        return;
    };
    let spv = shaders_dir().join(format!("{STEM}.spv"));
    let out = Command::new(&spirv_dis)
        .arg(&spv)
        .output_within(DXC_DEADLINE)
        .expect("invariant: spirv-dis was located and must run");
    assert!(out.status.success(), "spirv-dis failed on {}", spv.display());
    let dis = String::from_utf8(out.stdout).expect("invariant: spirv-dis emits UTF-8");
    let census = ext_inst_census(&dis);
    assert_eq!(
        census,
        expected_census(),
        "{STEM}.spv does not lower each E0 op to exactly one GLSL.std.450 instruction (a Trunc where \
         Floor belongs means the printer is wrong; two Log2 mean DXC expanded pow)"
    );
}

// `status_within` / `output_within` (a deadline on each child this file spawns) and `scratch_path`
// live in `tests/child_guard/mod.rs`; declared last so no cited line above moves.
mod child_guard;
use child_guard::{BoundedRun, DXC_DEADLINE};
