//! Cluster-cull `.spv` byte-identity gate — the re-DXC oracle for `cluster_cull.hlsl`.
//!
//! `cluster_cull.hlsl` grew an `#ifdef HIER` seam at rung VB-P1e H2 ("dark infra"): the
//! hierarchical two-level cull, compiled in only under `-D HIER=1`. This test clones the
//! `redxc_with_defines` multi-variant idiom `vb_froxel_spv_sync.rs:52-69` uses for the VB froxel
//! family, scoped to the two `cluster_cull.hlsl` variants:
//!
//! * `cluster_cull.comp.spv` (`cluster_cull.hlsl`, no `-D` — the base arm, gate (b): the seam
//!   must be physically inert on this compile, so it stays byte-identical to its pre-H2 build).
//! * `cluster_cull_hier.comp.spv` (`cluster_cull.hlsl`, `-D HIER=1` — the new variant, gate (a)).
//!
//! The arm bit is never armed this rung (no pipeline is created, nothing selects the HIER
//! module) — but both committed `.spv` must still byte-match their source under the frozen
//! recipe, exactly like every other variant family in this crate.
//!
//! SKIPS (with an eprintln) when no `dxc` resolves on the host — the byte gate is only as
//! hermetic as the pinned VulkanSDK 1.4.350.0 toolchain that produced the committed artifacts; a
//! DIFFERENT dxc version failing this test means "wrong toolchain", not "drifted shader" (the
//! committed recipe header pins the exact version).
//!
//! Also carries the H1.6 opcode/decoration census pin (`cluster_cull_spv_census_pinned`). It
//! closes the *base-module precondition* of the open P0 recorded in
//! `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md`'s errata — **not the P0 itself**. The P0: H2's
//! structural tripwires (assertions e5/e6) select instructions by "a `NoContraction`-decorated
//! `OpFAdd`" / "`NoContraction`-decorated `OpFSub`", and against the PRE-H1.6 base module
//! (`NoContraction == 0`, measured) both selectors pick the EMPTY SET — so their quantification is
//! vacuously true and would go green on an arbitrarily divergent module.
//!
//! What this pin does: it makes the base module's decoration count a non-zero, EXACT literal, so
//! an empty or shrunken selection is RED at the source instead of silently satisfying a later
//! rung's "for all decorated X" quantifier.
//!
//! What it does NOT do: it says nothing about the HIER module. `cluster_cull_hier_dis_gate.rs`
//! (H2 gate (e)) discharges that half, pre-registering e5's and e6's own non-empty selection
//! counts under BOTH compile options and demonstrating each can go RED.

use std::path::PathBuf;
use std::process::Command;

/// The shaders directory (`CARGO_MANIFEST_DIR/shaders`), where the committed `.hlsl` and `.spv`
/// live (and where DXC must run so any `#include` resolves).
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Locates the `dxc` executable: first the pinned Vulkan-SDK path (the repo's offline recipe),
/// then `$VULKAN_SDK/Bin`, then `PATH`. Returns `None` if none resolve (the byte-identity test
/// then SKIPS) — the `marcher_spv_sync.rs` idiom verbatim.
fn find_dxc() -> Option<PathBuf> {
    let pinned = PathBuf::from("C:/VulkanSDK/1.4.350.0/Bin/dxc.exe");
    if pinned.exists() {
        return Some(pinned);
    }
    let bare = if cfg!(windows) { "dxc.exe" } else { "dxc" };
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let candidate = PathBuf::from(sdk).join("Bin").join(bare);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if Command::new(bare).arg("--version").output().is_ok() {
        return Some(PathBuf::from(bare));
    }
    None
}

/// Re-DXCs `hlsl_name` (relative to the shaders dir) under the EXACT frozen recipe pinned in
/// `cluster_cull.hlsl`'s own header comment (`-spirv -T cs_6_0 -E main
/// -fspv-target-env=vulkan1.3`, no `-O`) plus the given `-D` defines, into a fresh temp `.spv`
/// named by `out_tag` (distinct per variant so parallel test binaries never collide), and
/// returns the bytes. Never overwrites a committed artifact. Mirrors
/// `vb_froxel_spv_sync.rs:56-69`'s `redxc_with_defines`.
fn redxc_with_defines(dxc: &PathBuf, dir: &PathBuf, hlsl_name: &str, defines: &[&str], out_tag: &str) -> Vec<u8> {
    let out_spv = std::env::temp_dir().join(format!("{out_tag}.redxc.spv"));
    let mut cmd = Command::new(dxc);
    cmd.current_dir(dir).args(["-spirv", "-T", "cs_6_0", "-E", "main"]);
    for d in defines {
        cmd.args(["-D", d]);
    }
    cmd.args(["-fspv-target-env=vulkan1.3", hlsl_name, "-Fo"]).arg(&out_spv);
    let status = cmd.status().expect("invariant: dxc was located and must run");
    assert!(status.success(), "dxc failed re-compiling {hlsl_name} {defines:?} under the frozen recipe");
    let bytes = std::fs::read(&out_spv).expect("invariant: dxc wrote the re-DXC .spv");
    let _ = std::fs::remove_file(&out_spv); // best-effort tidy
    bytes
}

/// One committed artifact must byte-equal its own re-DXC. Mirrors `vb_froxel_spv_sync.rs:72-86`.
fn assert_spv_byte_identical(dxc: &PathBuf, dir: &PathBuf, hlsl_name: &str, defines: &[&str], spv_name: &str) {
    let committed_path = dir.join(spv_name);
    let committed = std::fs::read(&committed_path)
        .unwrap_or_else(|e| panic!("missing committed {}: {e}", committed_path.display()));
    let fresh = redxc_with_defines(dxc, dir, hlsl_name, defines, spv_name);
    assert!(
        committed == fresh,
        "{spv_name} ({} bytes committed, {} bytes fresh) is NOT the re-DXC of {hlsl_name} \
         {defines:?} under the frozen recipe — either the committed .spv is stale (re-run the \
         recipe in the shader's header and commit it) or the host dxc is not the pinned \
         VulkanSDK 1.4.350.0 toolchain.",
        committed.len(),
        fresh.len(),
    );
}

/// Locates `spirv-dis`: first the pinned Vulkan-SDK path (beside the pinned `dxc.exe`), then
/// `$VULKAN_SDK/Bin`, then `PATH`. Mirrors [`find_dxc`]'s layered lookup and
/// `field_probe_gate.rs`'s `find_spirv_dis` (`field_probe_gate.rs:43-59`). Returns `None` if none
/// resolve (the census test then SKIPS).
fn find_spirv_dis() -> Option<PathBuf> {
    let pinned = PathBuf::from("C:/VulkanSDK/1.4.350.0/Bin/spirv-dis.exe");
    if pinned.exists() {
        return Some(pinned);
    }
    let bare = if cfg!(windows) { "spirv-dis.exe" } else { "spirv-dis" };
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let candidate = PathBuf::from(sdk).join("Bin").join(bare);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if Command::new(bare).arg("--version").output().is_ok() {
        return Some(PathBuf::from(bare));
    }
    None
}

/// Disassembles `spv_path` via `spirv-dis`, returning the textual SPIR-V. Panics on a non-zero
/// exit — a malformed committed `.spv` is a build-integrity bug, not a skip.
fn disassemble(spirv_dis: &PathBuf, spv_path: &PathBuf) -> String {
    let out = Command::new(spirv_dis)
        .arg(spv_path)
        .output()
        .expect("invariant: spirv-dis was located and must run");
    assert!(
        out.status.success(),
        "spirv-dis failed on {}: {}",
        spv_path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("invariant: spirv-dis emits UTF-8 disassembly")
}

/// The opcode/decoration counts `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` D10 and section 8.8 pin
/// for `cluster_cull.comp.spv`. Every field is counted by EXACT per-line token match (split on
/// whitespace, not substring search), so e.g. `NMin` cannot false-match inside a longer mnemonic.
#[derive(Debug, PartialEq, Eq)]
struct SpvCensus {
    op_dot: usize,
    no_contraction: usize,
    op_ford_less_than_equal: usize,
    op_return: usize,
    n_min: usize,
    n_max: usize,
    f_min: usize,
    f_max: usize,
    op_control_barrier: usize,
}

/// Counts the census tokens in a `spirv-dis` disassembly. `GLSL.std.450` ext-inst calls
/// (`NMin`/`NMax`/`FMin`/`FMax`) appear as a bare operand token on the `OpExtInst` line
/// (e.g. `%482 = OpExtInst %v3float %1 NMax %481 %60`), so a whitespace-split exact match finds
/// them without needing to distinguish opcode position from operand position.
fn census(dis: &str) -> SpvCensus {
    let mut c = SpvCensus {
        op_dot: 0,
        no_contraction: 0,
        op_ford_less_than_equal: 0,
        op_return: 0,
        n_min: 0,
        n_max: 0,
        f_min: 0,
        f_max: 0,
        op_control_barrier: 0,
    };
    for line in dis.lines() {
        for tok in line.split_whitespace() {
            match tok {
                "OpDot" => c.op_dot += 1,
                "NoContraction" => c.no_contraction += 1,
                "OpFOrdLessThanEqual" => c.op_ford_less_than_equal += 1,
                "OpReturn" => c.op_return += 1,
                "NMin" => c.n_min += 1,
                "NMax" => c.n_max += 1,
                "FMin" => c.f_min += 1,
                "FMax" => c.f_max += 1,
                "OpControlBarrier" => c.op_control_barrier += 1,
                _ => {}
            }
        }
    }
    c
}

/// H2 gate (a): the new `-D HIER=1` variant byte-equals its own re-DXC under the frozen recipe.
#[test]
fn cluster_cull_hier_variant_spv_byte_identical() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "cluster_cull_spv_sync: dxc not found (no C:/VulkanSDK/.../dxc.exe, no \
             $VULKAN_SDK/Bin, not on PATH) — SKIPPING the cluster-cull-hier re-DXC \
             byte-identity check on this host."
        );
        return;
    };
    let dir = shaders_dir();
    assert_spv_byte_identical(&dxc, &dir, "cluster_cull.hlsl", &["HIER=1"], "cluster_cull_hier.comp.spv");
}

/// H2 gate (b): the BASE (no `-D`) compile stays byte-identical to the H1.6-re-pinned blob —
/// the `#ifdef HIER` seam (the new push-tail members + the new arm of `main`) must be
/// physically inert on this compile. **Re-measured at this rung, not inherited**: the push
/// struct's HIER-only tail widened by a second word since the last time this inertness was
/// probed, so a stale measurement would not cover the artifact being committed here.
#[test]
fn cluster_cull_base_variant_spv_unperturbed_by_the_hier_seam() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "cluster_cull_spv_sync: dxc not found (no C:/VulkanSDK/.../dxc.exe, no \
             $VULKAN_SDK/Bin, not on PATH) — SKIPPING the cluster-cull base-variant \
             byte-identity check on this host."
        );
        return;
    };
    let dir = shaders_dir();
    assert_spv_byte_identical(&dxc, &dir, "cluster_cull.hlsl", &[], "cluster_cull.comp.spv");
}

/// H1.6 census pin (D10, section 8.8 gate (a)): the committed `cluster_cull.comp.spv` carries
/// EXACTLY the opcode/decoration counts D10's measured table predicts for the 7-decoration
/// `precise` placement, and — the mechanical P0 discharge — `NoContraction` is asserted non-zero
/// as an explicit, separate check so a future edit that silently drops `precise` (collapsing
/// `NoContraction` back to 0) is caught HERE, before it can make an H2 structural selector
/// vacuously true. SKIPS (with an eprintln) when no `spirv-dis` resolves, matching
/// `field_probe_gate.rs`'s skip semantics.
#[test]
fn cluster_cull_spv_census_pinned() {
    let Some(spirv_dis) = find_spirv_dis() else {
        eprintln!(
            "cluster_cull_spv_sync: spirv-dis not found (no C:/VulkanSDK/.../spirv-dis.exe, no \
             $VULKAN_SDK/Bin, not on PATH) — SKIPPING the H1.6 opcode/decoration census check on \
             this host."
        );
        return;
    };
    let dir = shaders_dir();
    let committed_path = dir.join("cluster_cull.comp.spv");
    assert!(
        committed_path.exists(),
        "missing committed {}",
        committed_path.display()
    );
    let dis = disassemble(&spirv_dis, &committed_path);
    let actual = census(&dis);

    // MEASURED values (docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md D10, section 8.8 gate (a)) — do not
    // edit these literals to make a failing run pass; a mismatch means the census DRIFTED and the
    // fix is in the shader source, not in this test.
    let expected = SpvCensus {
        op_dot: 8,
        no_contraction: 7,
        op_ford_less_than_equal: 1,
        op_return: 1,
        n_min: 8,
        n_max: 18,
        f_min: 0,
        f_max: 0,
        op_control_barrier: 0,
    };
    assert_eq!(
        actual, expected,
        "cluster_cull.comp.spv opcode/decoration census diverged from the H1.6 pin. Expected \
         {expected:?} (D10's measured 7-decoration `precise` placement — 2 `OpFSub` + 3 `OpFMul` \
         + 2 `OpFAdd` decorated `NoContraction`, `OpDot` down from 9 to 8), got {actual:?}. If \
         `sq_dist_point_aabb` was intentionally re-shaped, re-run the D10 measurement, update \
         this pin AND re-run the H1.6 gate (docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md section 8.8) \
         in full, including its zero-golden-move budget and perf gate."
    );

    // The BASE-MODULE PRECONDITION of the errata's open P0 (not the P0 itself — see this file's
    // module doc): H2's e5/e6 selectors pick instructions BY `NoContraction`-decoration, and on
    // the pre-H1.6 base module (`NoContraction == 0`) both pick the empty set and vacuously pass
    // on any module, however divergent. Asserting non-zero HERE, on the artifact this rung ships,
    // means that failure mode cannot silently return once H2 is built. It does NOT relieve H2 of
    // pre-registering e5's and e6's OWN non-empty selection counts on both compile options.
    assert!(
        actual.no_contraction > 0,
        "invariant: NoContraction must be non-zero on the H1.6-re-pinned base module — a zero \
         count means a later `NoContraction`-scoped structural selector (H2's e5/e6) would \
         select the empty set and vacuously pass on an arbitrarily divergent module"
    );
}
