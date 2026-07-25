//! H2 gate (e)/(f)/(g) — the `-D HIER=1` structural SPIR-V tripwire (VB-P1e, "dark infra").
//!
//! `cluster_cull.hlsl`'s `#ifdef HIER` arm is proven correct on paper by
//! `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` section 5 (the coarse-to-fine enclosure theorem).
//! That proof rests on a premise no `.spv` byte- or disassembly-gate can fully discharge —
//! DXC emits **zero** `Fma` in every measured variant, so FMA contraction is a *driver-side*
//! SPIR-V-to-ISA decision, invisible below the `.spv`. Section 8.9 calls this gate a
//! **tripwire**, not the proof: it catches the *reachable* regressions (dropping `precise`,
//! restoring `dot()`, an early `return` before a barrier, a widened shared push struct) that
//! would silently break the proof's premises, but it cannot see a driver that contracts one
//! `OpDot`-lowered call site and not the other — no variant measured emits `OpDot` in the cull
//! comparison at all (e1), which is exactly how D10 sidesteps that hole.
//!
//! # The open P0 this file closes (errata, plan lines ~75-82)
//!
//! Checks **e5** and **e6** select instructions by "a `NoContraction`-decorated `OpFAdd`" /
//! "`NoContraction`-decorated `OpFSub` only". A selector like that, applied to a module with
//! zero such decorations (the committed module BEFORE rung H1.6), selects the EMPTY SET — so
//! the quantification "for all selected X, P(X)" is **vacuously true** and would go green on an
//! arbitrarily divergent module. `cluster_cull_spv_sync.rs`'s `cluster_cull_spv_census_pinned`
//! test already closes the *base-module precondition* (`NoContraction > 0` on the committed
//! base `.spv`) — **not this P0**: it says nothing about e5's/e6's own selection counts, and
//! nothing about the HIER module at all. This file pre-registers **exact, non-empty selection
//! counts** for e5 and e6, on **both** compile options, and an empty or shrunken selection is a
//! hard `assert!` failure (RED), not a vacuously-passing loop.
//!
//! Pre-registered, measured counts (re-measure and update this doc block, not the literals in
//! the code, if `sq_dist_point_aabb` is ever re-shaped):
//!
//! | check | base | HIER |
//! |---|---|---|
//! | e1 `OpDot` (ray-gen only, zero in the cull compare) | 8 | 8 |
//! | e2 `NoContraction` decorations | 7 | 14 |
//! | e3 scalar (`%bool`) `OpFOrdLessThanEqual` | 1 | 2 |
//! | e4 vector (`%v3bool`) `OpFOrdLessThanEqual` (finiteness) | 0 | 2 |
//! | e5 id-normalised 14-instruction windows (HIER only — base has only one call site, nothing to pair) | n/a | 2 |
//! | e6 `NoContraction`-decorated `OpFSub` | 2 | 4 |
//! | e7 push-block member count / tail offsets | 4, last `Offset 12` | 6, `Offset 16`/`Offset 20` |
//! | e8 every `OpControlBarrier` on the entry function's top-level block chain | n/a (0 barriers) | 3/3 |
//!
//! Each selector was verified, during implementation, to actually go RED on a deliberate
//! mutation (dropping `precise` empties e5/e6's selections to zero; an isolated early `return`
//! drops the top-level chain from 18 blocks to 2 and takes all 3 barriers off it) — see the
//! commit message / implementation report for the transcript. This file itself asserts the
//! PRE-REGISTERED counts against the real, correct, committed artifacts, so a future regression
//! is caught the same way without anyone re-deriving the mutation by hand.
//!
//! # (f), (g), (h)
//!
//! (f) re-compiles a scratch copy of `cluster_cull.hlsl` with `HIER_MASK_WORDS` widened past its
//! `#error` guard's limit and asserts DXC **fails** — the guard is mechanical, not a "run it once
//! during review" note. (g) is a **source-text** belt to (e8)'s SPIR-V-level braces: `cluster_cull.hlsl`
//! is hand-authored HLSL, so a plain scan for a bare `return` token inside the `#ifdef HIER`
//! (pre-`#else`) span is cheap, human-readable, and independent of what DXC's structured-CFG
//! lowering happens to do with it.
//!
//! (h) closes P1-1 (adversarial review of this rung): `[numthreads(256, 1, 1)]` and `HIER_TPG`
//! (`#define HIER_TPG 256u`) were two independent literals nothing tied together — mutating the
//! attribute to `(128, 1, 1)` while leaving `HIER_TPG` at 256 compiled, passed `spirv-val`, and
//! passed every one of (e1)-(e8) with identical counts (none of them read the dispatch's actual
//! group size). `cluster_cull.hlsl:169` now drives the attribute FROM `HIER_TPG`, so that specific
//! mutation is unrepresentable in this one file going forward, but a source-level tie protects
//! only future edits to the `.hlsl`, not a stale or hand-crafted `.spv` — (h) is the independent,
//! artifact-level pin on the emitted `OpExecutionMode ... LocalSize` the errata still requires:
//! **256 1 1** on the HIER module, **64 1 1** on the base module.
//!
//! SKIPS (with an eprintln) when the required toolchain binary is missing, matching every other
//! `.spv` gate in this crate; (g) needs no toolchain and never skips.

use std::path::PathBuf;
use std::process::Command;

// ============================================================================================
// Toolchain locators + re-DXC / disassemble helpers (the `cluster_cull_spv_sync.rs` /
// `field_probe_gate.rs` idiom, duplicated per this crate's own convention of self-contained
// integration-test files).
// ============================================================================================

/// The shaders directory (`CARGO_MANIFEST_DIR/shaders`), where the committed `.hlsl`/`.spv`
/// live (and where DXC must run, or be given `-I`, so any `#include` resolves).
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Locates `dxc`: first the pinned Vulkan-SDK path, then `$VULKAN_SDK/Bin`, then `PATH`.
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

/// Locates `spirv-dis`: first the pinned Vulkan-SDK path, then `$VULKAN_SDK/Bin`, then `PATH`.
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

/// Disassembles a committed `.spv` (the artifact the engine would load), returning the textual
/// SPIR-V. Panics on a non-zero exit — a malformed committed `.spv` is a build-integrity bug.
fn disassemble_committed(spirv_dis: &PathBuf, spv_path: &PathBuf) -> String {
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

// ============================================================================================
// A minimal SPIR-V text-disassembly parser: one `Insn` per source line (result id, opcode,
// operand tokens). Every SSA-referencing token in `spirv-dis` output is `%`-prefixed (result
// ids, type ids, named constants), so treating "any `%`-token" as an id is complete for this
// instruction set.
// ============================================================================================

struct Insn<'a> {
    result: Option<&'a str>,
    opcode: &'a str,
    operands: Vec<&'a str>,
}

/// Parses a `spirv-dis` text disassembly into one [`Insn`] per non-blank, non-comment line.
fn parse(dis: &str) -> Vec<Insn<'_>> {
    dis.lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with(';'))
        .filter_map(|l| {
            let toks: Vec<&str> = l.split_whitespace().collect();
            if toks.is_empty() {
                return None;
            }
            if toks.len() >= 2 && toks[1] == "=" {
                Some(Insn { result: Some(toks[0]), opcode: toks[2], operands: toks[3..].to_vec() })
            } else {
                Some(Insn { result: None, opcode: toks[0], operands: toks[1..].to_vec() })
            }
        })
        .collect()
}

/// Linear scan for the instruction that defines SSA id `id` (this module is small enough —
/// a few hundred instructions — that a `Vec` scan is simpler and faster to audit than a
/// `HashMap`, and `HashMap` is disallowed on principle outside a documented exception).
fn find_def<'a, 'b>(insns: &'a [Insn<'b>], id: &str) -> Option<&'a Insn<'b>> {
    insns.iter().find(|i| i.result == Some(id))
}

/// Every SSA id decorated `NoContraction` (`OpDecorate %id NoContraction`).
fn no_contraction_ids<'a>(insns: &'a [Insn<'_>]) -> Vec<&'a str> {
    insns
        .iter()
        .filter(|i| i.opcode == "OpDecorate" && i.operands.get(1).copied() == Some("NoContraction"))
        .filter_map(|i| i.operands.first().copied())
        .collect()
}

fn count_opcode(insns: &[Insn<'_>], opcode: &str) -> usize {
    insns.iter().filter(|i| i.opcode == opcode).count()
}

/// (h): `OpExecutionMode %main LocalSize x y z` as literal `(x, y, z)` triples. `LocalSize`
/// takes bare integer-literal operands, never `%`-id specialisation constants, so no
/// `OpExecutionModeId` form is possible for this mode — confirmed absent from both committed
/// modules by disassembly inspection before this gate was written.
fn local_size_triples(insns: &[Insn<'_>]) -> Vec<(u32, u32, u32)> {
    insns
        .iter()
        .filter(|i| i.opcode == "OpExecutionMode" && i.operands.get(1).copied() == Some("LocalSize"))
        .filter_map(|i| {
            let x: u32 = i.operands.get(2)?.parse().ok()?;
            let y: u32 = i.operands.get(3)?.parse().ok()?;
            let z: u32 = i.operands.get(4)?.parse().ok()?;
            Some((x, y, z))
        })
        .collect()
}

/// e3/e4: `OpFOrdLessThanEqual` split by result type — `%bool` (scalar, the cull compare) vs
/// `%v3bool` (vector, the section-4 finiteness predicate `all(abs(v) <= 1e30)`).
fn count_ford_less_than_equal(insns: &[Insn<'_>], result_ty: &str) -> usize {
    insns
        .iter()
        .filter(|i| i.opcode == "OpFOrdLessThanEqual" && i.operands.first().copied() == Some(result_ty))
        .count()
}

/// Replaces every `%`-prefixed SSA-id token with a fresh placeholder assigned in order of
/// first appearance WITHIN this window (result ids, type ids, and operand ids are all
/// canonicalised by the same rule, since they are all `%`-tokens) — an alpha-equivalence
/// normalisation. Two structurally identical instruction sequences at different SPIR-V ids
/// produce byte-identical canonical text; a structural divergence does not.
fn canonicalize_window(window: &[&Insn<'_>]) -> String {
    let mut seen: Vec<(&str, u32)> = Vec::new();
    let mut next = 0u32;
    let mut out = String::new();
    for insn in window {
        if let Some(r) = insn.result {
            out.push_str(&canonical_token(r, &mut seen, &mut next));
            out.push_str(" = ");
        }
        out.push_str(insn.opcode);
        for op in &insn.operands {
            out.push(' ');
            out.push_str(&canonical_token(op, &mut seen, &mut next));
        }
        out.push('\n');
    }
    out
}

fn canonical_token<'a>(tok: &'a str, seen: &mut Vec<(&'a str, u32)>, next: &mut u32) -> String {
    if !tok.starts_with('%') {
        return tok.to_string();
    }
    if let Some((_, n)) = seen.iter().find(|(t, _)| *t == tok) {
        format!("#{n}")
    } else {
        let n = *next;
        *next += 1;
        seen.push((tok, n));
        format!("#{n}")
    }
}

/// e5's selection: every scalar (`%bool`) `OpFOrdLessThanEqual` whose LHS operand is a
/// `NoContraction`-decorated `OpFAdd`, paired with the CONTIGUOUS 14 preceding instructions
/// (itself included) — the exact `sq_dist_point_aabb` call-site expansion (D10's per-node
/// audit table has 14 nodes: 2 `OpFSub`, 2 `NMax`, 3 `OpCompositeExtract`, 3 `OpFMul`,
/// 2 `OpFAdd`, the `r*r` `OpFMul`, the compare itself).
fn e5_windows<'a, 'b>(insns: &'a [Insn<'b>], nc_ids: &[&str]) -> Vec<Vec<&'a Insn<'b>>> {
    let mut windows = Vec::new();
    for (idx, insn) in insns.iter().enumerate() {
        if insn.opcode != "OpFOrdLessThanEqual" || insn.operands.first().copied() != Some("%bool") {
            continue;
        }
        let Some(lhs) = insn.operands.get(1) else { continue };
        let Some(def) = find_def(insns, lhs) else { continue };
        if def.opcode != "OpFAdd" || !nc_ids.contains(lhs) || idx + 1 < 14 {
            continue;
        }
        windows.push(insns[idx + 1 - 14..=idx].iter().collect());
    }
    windows
}

/// e6's selection: every `NoContraction`-decorated `OpFSub` (the two inside
/// `sq_dist_point_aabb`, per call site — D10's placement). MUST stay scoped to decorated
/// `OpFSub` only: the module carries ~24 undecorated `OpFSub` in ray-gen whose operands ARE
/// `OpFMul` results, so an unscoped producer check false-REDs a correct module.
fn e6_decorated_fsub<'a, 'b>(insns: &'a [Insn<'b>], nc_ids: &[&str]) -> Vec<&'a Insn<'b>> {
    insns.iter().filter(|i| i.opcode == "OpFSub" && i.result.is_some_and(|r| nc_ids.contains(&r))).collect()
}

/// e7: `(member_index, offset)` pairs for every `OpMemberDecorate %type_name <idx> Offset <val>`
/// on the named push-constant struct type.
fn push_member_offsets(insns: &[Insn<'_>], type_name: &str) -> Vec<(u32, u32)> {
    insns
        .iter()
        .filter(|i| i.opcode == "OpMemberDecorate" && i.operands.first().copied() == Some(type_name))
        .filter_map(|i| {
            let idx: u32 = i.operands.get(1)?.parse().ok()?;
            if i.operands.get(2).copied() != Some("Offset") {
                return None;
            }
            let off: u32 = i.operands.get(3)?.parse().ok()?;
            Some((idx, off))
        })
        .collect()
}

/// e8's definition (plan section 8.9, verified to discriminate — M9): starting at the entry
/// function's first block, follow its `OpSelectionMerge`/`OpLoopMerge` MERGE TARGET if it has
/// one, else its unconditional `OpBranch` target; stop at a terminator with neither. The blocks
/// visited are the "top-level chain". Every `OpControlBarrier` must sit in one of them.
///
/// Deliberately NOT "exactly one `OpReturn`" or "every barrier in a merge block" — both were
/// measured (M7/M8) to NOT discriminate a correct shader from a deliberately broken one (DXC
/// canonicalises every function to a single exit block regardless, and a barrier can validly
/// sit in a non-merge block, e.g. an `OpSwitch` case).
fn top_level_chain<'a>(insns: &'a [Insn<'a>]) -> Vec<&'a str> {
    enum Terminator<'a> {
        None,
        Merge(&'a str),
        Uncond(&'a str),
    }
    let mut blocks: Vec<(&str, Terminator<'_>)> = Vec::new();
    let mut cur_label: Option<&str> = None;
    let mut cur_merge: Option<&str> = None;
    let mut cur_uncond: Option<&str> = None;
    let flush = |blocks: &mut Vec<(&'a str, Terminator<'a>)>,
                 cur_label: &mut Option<&'a str>,
                 cur_merge: &mut Option<&'a str>,
                 cur_uncond: &mut Option<&'a str>| {
        if let Some(lbl) = cur_label.take() {
            let term = match (cur_merge.take(), cur_uncond.take()) {
                (Some(m), _) => Terminator::Merge(m),
                (None, Some(u)) => Terminator::Uncond(u),
                (None, None) => Terminator::None,
            };
            blocks.push((lbl, term));
        }
    };
    for insn in insns {
        match insn.opcode {
            "OpLabel" => {
                flush(&mut blocks, &mut cur_label, &mut cur_merge, &mut cur_uncond);
                cur_label = insn.result;
            }
            "OpSelectionMerge" | "OpLoopMerge" => cur_merge = insn.operands.first().copied(),
            "OpBranch" => cur_uncond = insn.operands.first().copied(),
            "OpFunctionEnd" => flush(&mut blocks, &mut cur_label, &mut cur_merge, &mut cur_uncond),
            _ => {}
        }
    }
    let Some((first, _)) = blocks.first() else { return Vec::new() };
    let mut chain = vec![*first];
    loop {
        let current = *chain.last().expect("invariant: chain is seeded with `first` above");
        let Some((_, term)) = blocks.iter().find(|(id, _)| *id == current) else { break };
        let next = match term {
            Terminator::Merge(m) => Some(*m),
            Terminator::Uncond(u) => Some(*u),
            Terminator::None => None,
        };
        match next {
            Some(n) if !chain.contains(&n) => chain.push(n),
            _ => break,
        }
    }
    chain
}

/// The block a given instruction index belongs to (the nearest preceding `OpLabel`).
fn block_of<'a>(insns: &'a [Insn<'a>], target_idx: usize) -> Option<&'a str> {
    let mut cur = None;
    for (i, insn) in insns.iter().enumerate() {
        if insn.opcode == "OpLabel" {
            cur = insn.result;
        }
        if i == target_idx {
            return cur;
        }
    }
    None
}

/// The push-constant struct's DXC friendly name, stable because it is derived from the HLSL
/// type name `ClusterCullPush` (`cluster_cull.hlsl`'s own `struct ClusterCullPush { .. }`).
const PUSH_TYPE: &str = "%type_PushConstant_ClusterCullPush";

// ============================================================================================
// (e) — the structural tripwire.
// ============================================================================================

/// H2 gate (e): pre-registered, exact structural counts on the COMMITTED artifacts (the bytes
/// the engine would load), for both the base and the HIER module. See this file's module doc
/// for the measured table and the errata this discharges.
#[test]
fn cluster_cull_hier_structural_tripwire_e() {
    let Some(spirv_dis) = find_spirv_dis() else {
        eprintln!(
            "cluster_cull_hier_dis_gate: spirv-dis not found (no C:/VulkanSDK/.../spirv-dis.exe, \
             no $VULKAN_SDK/Bin, not on PATH) — SKIPPING the H2 structural tripwire on this host."
        );
        return;
    };
    let dir = shaders_dir();
    let base_dis = disassemble_committed(&spirv_dis, &dir.join("cluster_cull.comp.spv"));
    let hier_dis = disassemble_committed(&spirv_dis, &dir.join("cluster_cull_hier.comp.spv"));
    let base = parse(&base_dis);
    let hier = parse(&hier_dis);
    let base_nc = no_contraction_ids(&base);
    let hier_nc = no_contraction_ids(&hier);

    // e1: OpDot == 8 on both (the ray-gen `dot(rd, cam_forward.xyz)`, 4 corners x near/far;
    // ZERO in the cull comparison — that is the whole point of D10).
    assert_eq!(count_opcode(&base, "OpDot"), 8, "e1: base OpDot count drifted from 8");
    assert_eq!(count_opcode(&hier, "OpDot"), 8, "e1: HIER OpDot count drifted from 8");

    // e2: NoContraction decoration count -- 7 on base (one sq_dist_point_aabb call site),
    // 14 on HIER (two call sites: coarse + fine).
    assert_eq!(base_nc.len(), 7, "e2: base NoContraction count drifted from 7");
    assert_eq!(hier_nc.len(), 14, "e2: HIER NoContraction count drifted from 14");

    // e3/e4: scalar vs vector OpFOrdLessThanEqual. Base has ONE scalar cull compare and no
    // vector compare at all. HIER has TWO scalar cull compares (coarse + fine) and TWO vector
    // finiteness compares (`all(abs(aabb_min) <= 1e30)`, `all(abs(aabb_max) <= 1e30)`).
    assert_eq!(count_ford_less_than_equal(&base, "%bool"), 1, "e3: base scalar compare count drifted from 1");
    assert_eq!(count_ford_less_than_equal(&hier, "%bool"), 2, "e3: HIER scalar compare count drifted from 2");
    assert_eq!(count_ford_less_than_equal(&base, "%v3bool"), 0, "e4: base vector compare count drifted from 0");
    assert_eq!(count_ford_less_than_equal(&hier, "%v3bool"), 2, "e4: HIER vector compare count drifted from 2");

    // e5 (HIER only): the errata's open P0. PRE-REGISTER a non-empty, EXACT selection count —
    // an empty or shrunken selection here (e.g. because `precise` was dropped and no
    // NoContraction decoration exists to select on) must be RED, not vacuously green.
    let hier_windows = e5_windows(&hier, &hier_nc);
    assert_eq!(
        hier_windows.len(),
        2,
        "e5: expected EXACTLY 2 id-normalised windows (one per sq_dist_point_aabb call site) on \
         the HIER module; got {} -- an empty or shrunken selection means the NoContraction-scoped \
         selector is not discharging the errata's P0 (it may be selecting the vacuous empty set)",
        hier_windows.len()
    );
    let w0 = canonicalize_window(&hier_windows[0]);
    let w1 = canonicalize_window(&hier_windows[1]);
    assert_eq!(
        w0, w1,
        "e5: the coarse and fine sq_dist_point_aabb call sites' id-normalised windows diverged -- \
         the two sites are no longer structurally identical, which breaks section 5 Step 0's \
         same-function premise"
    );

    // e6 (both): PRE-REGISTER a non-empty, EXACT selection count for the NoContraction-decorated
    // OpFSub selector, on BOTH compile options -- the errata names this exact failure mode.
    let base_fsub = e6_decorated_fsub(&base, &base_nc);
    let hier_fsub = e6_decorated_fsub(&hier, &hier_nc);
    assert_eq!(base_fsub.len(), 2, "e6: expected EXACTLY 2 decorated OpFSub on base; got {}", base_fsub.len());
    assert_eq!(hier_fsub.len(), 4, "e6: expected EXACTLY 4 decorated OpFSub on HIER; got {}", hier_fsub.len());
    // Producer check, SCOPED to the decorated OpFSub only (the module carries ~24 undecorated
    // OpFSub in ray-gen whose operands ARE OpFMul results -- an unscoped form would false-RED).
    for (label, insns, fsubs) in [("base", &base, &base_fsub), ("HIER", &hier, &hier_fsub)] {
        for fsub in fsubs {
            for operand in &fsub.operands[1..] {
                if let Some(def) = find_def(insns, operand) {
                    assert_ne!(
                        def.opcode, "OpFMul",
                        "e6: {label} decorated OpFSub {:?} has an OpFMul-produced operand {operand} \
                         -- a contraction partner may exist where D10's proof assumes none",
                        fsub.result
                    );
                }
            }
        }
    }

    // e7: push-block member count + the HIER-only tail's offsets. Base stays 4 members, last
    // (index 3) at Offset 12; HIER is 6 members, cluster_dims_packed (index 4) at Offset 16,
    // cluster_capacity (index 5) at Offset 20.
    let base_push = push_member_offsets(&base, PUSH_TYPE);
    let hier_push = push_member_offsets(&hier, PUSH_TYPE);
    assert_eq!(base_push.len(), 4, "e7: base push block member count drifted from 4");
    assert!(
        base_push.contains(&(3, 12)),
        "e7: base push block's last member (index 3) is not at Offset 12: {base_push:?}"
    );
    assert_eq!(hier_push.len(), 6, "e7: HIER push block member count drifted from 6");
    assert!(
        hier_push.contains(&(4, 16)),
        "e7: HIER push block's cluster_dims_packed (index 4) is not at Offset 16: {hier_push:?}"
    );
    assert!(
        hier_push.contains(&(5, 20)),
        "e7: HIER push block's cluster_capacity (index 5) is not at Offset 20: {hier_push:?}"
    );

    // e8: every OpControlBarrier lies on the entry function's top-level block chain (D1's 3
    // barriers -- B1, B2, B3). This is a DESIGN-CONFORMANCE check on section 4's shape, not a
    // general legality check (a barrier inside a group-uniform loop would be legal Vulkan and
    // would fail this) -- section 4 has no such barrier.
    let chain = top_level_chain(&hier);
    let barrier_idxs: Vec<usize> =
        hier.iter().enumerate().filter(|(_, i)| i.opcode == "OpControlBarrier").map(|(i, _)| i).collect();
    assert_eq!(barrier_idxs.len(), 3, "e8: expected EXACTLY 3 OpControlBarrier on HIER (B1, B2, B3)");
    for idx in barrier_idxs {
        let blk = block_of(&hier, idx);
        assert!(
            blk.is_some_and(|b| chain.contains(&b)),
            "e8: OpControlBarrier at instruction {idx} (block {blk:?}) is NOT on the entry \
             function's top-level block chain ({} blocks) -- a barrier reached only through \
             divergent/conditional control flow is UB and typically a device hang",
            chain.len()
        );
    }
}

// ============================================================================================
// (f) — the `#error` guards, mechanical.
// ============================================================================================

/// H2 gate (f): copies `cluster_cull.hlsl` into the temp dir with `HIER_MASK_WORDS` widened past
/// its own `#error` guard's limit (32 -> 64), compiles it with `-D HIER=1` under the frozen
/// recipe (via `-I` so the unmodified `ray_gen.hlsli`/`light_table.hlsli` still resolve), and
/// asserts the compile FAILS with the expected diagnostic. Never touches the committed `.hlsl`.
#[test]
fn cluster_cull_hier_error_guards_mechanical_f() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "cluster_cull_hier_dis_gate: dxc not found (no C:/VulkanSDK/.../dxc.exe, no \
             $VULKAN_SDK/Bin, not on PATH) — SKIPPING the H2(f) #error mechanical gate on this \
             host."
        );
        return;
    };
    let dir = shaders_dir();
    let source = std::fs::read_to_string(dir.join("cluster_cull.hlsl"))
        .expect("invariant: cluster_cull.hlsl is the committed shader source");
    let needle = "#define HIER_MASK_WORDS 32u";
    assert!(
        source.contains(needle),
        "invariant: {needle:?} must appear verbatim in cluster_cull.hlsl for this mutation to be \
         meaningful -- if the constant's declaration changed, update this test"
    );
    let mutated = source.replacen(needle, "#define HIER_MASK_WORDS 64u", 1);

    let scratch_path = std::env::temp_dir().join("cluster_cull_hier_dis_gate_mask64_mutant.hlsl");
    std::fs::write(&scratch_path, &mutated).expect("invariant: temp dir is writable");
    let out_spv = std::env::temp_dir().join("cluster_cull_hier_dis_gate_mask64_mutant.spv");
    let out = Command::new(&dxc)
        .args(["-spirv", "-T", "cs_6_0", "-E", "main", "-fspv-target-env=vulkan1.3", "-I"])
        .arg(&dir)
        .args(["-D", "HIER=1"])
        .arg(&scratch_path)
        .arg("-Fo")
        .arg(&out_spv)
        .output()
        .expect("invariant: dxc was located and must run");
    let _ = std::fs::remove_file(&scratch_path);
    let _ = std::fs::remove_file(&out_spv);

    assert!(
        !out.status.success(),
        "H2(f) MUTATION DID NOT FIRE: widening HIER_MASK_WORDS to 64 compiled successfully -- \
         the #error guard at cluster_cull.hlsl's HIER_MASK_WORDS > 32 check is missing or \
         inert. This guard exists because gs_summary is a single uint (one bit per mask word)."
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("HIER_MASK_WORDS > 32"),
        "H2(f): dxc failed as expected, but not with the HIER_MASK_WORDS #error text -- got: \
         {stderr}"
    );
}

// ============================================================================================
// (g) — source-text pin for D8 (belt to e8's braces).
// ============================================================================================

/// Strips a `//` line comment (this file is entirely `//`-commented — no `/* */` block
/// comments) so a doc-comment mentioning the word "return" in prose (e.g. D8's own review-gate
/// commentary) does not false-positive the token scan below.
fn strip_line_comment(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// True iff `text` contains the bare identifier token `return` (word-boundary matched by
/// splitting on non-identifier characters — not a substring match).
fn contains_return_token(text: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric() && c != '_').any(|tok| tok == "return")
}

/// Whether a preprocessor conditional frame was opened by `#ifdef HIER` and, if so, whether we
/// are still on its pre-`#else` (active) side.
enum Frame {
    Hier(bool),
    Other,
}

/// Scans `hlsl` for a bare `return` token inside any `#ifdef HIER` span, restricted to the
/// span BEFORE that same frame's `#else` (the base arm's own `return` -- legitimate, D8 does
/// not apply to it -- lives in the `#else` half and must NOT be flagged). Correctly nests
/// through the unrelated `#if (HIER_MASK_WORDS) > 32 ... #endif` guards inside the HIER
/// prologue: those are `Frame::Other` and inherit the nearest HIER ancestor's active state.
fn hier_active_lines_with_return(hlsl: &str) -> Vec<(usize, String)> {
    let mut stack: Vec<Frame> = Vec::new();
    let mut violations = Vec::new();
    for (i, raw) in hlsl.lines().enumerate() {
        let trimmed = raw.trim_start();
        if trimmed.starts_with("#ifdef") || trimmed.starts_with("#ifndef") || trimmed.starts_with("#if") {
            if trimmed.trim_end() == "#ifdef HIER" {
                stack.push(Frame::Hier(true));
            } else {
                stack.push(Frame::Other);
            }
            continue;
        }
        if trimmed.starts_with("#else") {
            if let Some(Frame::Hier(active)) = stack.last_mut() {
                *active = false;
            }
            continue;
        }
        if trimmed.starts_with("#endif") {
            stack.pop();
            continue;
        }
        if trimmed.starts_with('#') {
            continue; // #define / #error / #include -- not scanned
        }
        let nearest_hier = stack.iter().rev().find_map(|f| match f {
            Frame::Hier(active) => Some(*active),
            Frame::Other => None,
        });
        if nearest_hier == Some(true) && contains_return_token(strip_line_comment(raw)) {
            violations.push((i + 1, raw.to_string()));
        }
    }
    violations
}

/// H2 gate (g): the `#ifdef HIER` (pre-`#else`) span of `cluster_cull.hlsl` contains NO bare
/// `return` token, at the source-text level. Cheap, human-readable, and independent of DXC's
/// structured-CFG lowering (e8 is the SPIR-V-level counterpart; this is its belt). Needs no
/// toolchain and never skips.
#[test]
fn cluster_cull_hier_no_early_return_source_pin_g() {
    let dir = shaders_dir();
    let source = std::fs::read_to_string(dir.join("cluster_cull.hlsl"))
        .expect("invariant: cluster_cull.hlsl is the committed shader source");
    let violations = hier_active_lines_with_return(&source);
    assert!(
        violations.is_empty(),
        "H2(g): found a bare `return` token inside the #ifdef HIER (pre-#else) span of \
         cluster_cull.hlsl -- D8 requires every lane to reach every barrier; an early `return` \
         is UB and typically a device hang. Violations: {violations:?}"
    );
}

// ============================================================================================
// (h) — [numthreads] vs the emitted LocalSize (P1-1, adversarial review of this rung).
// ============================================================================================

/// H2 gate (h): the committed modules' emitted `OpExecutionMode ... LocalSize` is pinned by
/// exact literal, independently of `cluster_cull.hlsl`'s own source-level tie
/// (`[numthreads(HIER_TPG, 1, 1)]`, `cluster_cull.hlsl:169`).
///
/// Before that tie, `[numthreads(256, 1, 1)]` and `HIER_TPG` (`#define HIER_TPG 256u`) were two
/// independent literals nothing tied together: mutating the attribute to `(128, 1, 1)` while
/// leaving `HIER_TPG` at 256 compiled, passed `spirv-val`, and passed every one of (e1)-(e8) with
/// identical counts (none of them read the dispatch's actual group size) — that shader would be
/// catastrophically wrong at runtime, folding and testing against only half its lanes' worth of
/// froxels. The source-level tie makes that ONE mutation unrepresentable in this file going
/// forward, but it does not protect the committed `.spv` itself (a stale artifact, or one built
/// from a hand-edited/reverted source, is not covered by a tie that lives in a DIFFERENT file) —
/// this gate is the artifact-level backstop the errata requires regardless.
#[test]
fn cluster_cull_hier_local_size_pinned_h() {
    let Some(spirv_dis) = find_spirv_dis() else {
        eprintln!(
            "cluster_cull_hier_dis_gate: spirv-dis not found (no C:/VulkanSDK/.../spirv-dis.exe, \
             no $VULKAN_SDK/Bin, not on PATH) — SKIPPING the H2(h) LocalSize pin on this host."
        );
        return;
    };
    let dir = shaders_dir();
    let base_dis = disassemble_committed(&spirv_dis, &dir.join("cluster_cull.comp.spv"));
    let hier_dis = disassemble_committed(&spirv_dis, &dir.join("cluster_cull_hier.comp.spv"));
    let base = parse(&base_dis);
    let hier = parse(&hier_dis);

    let base_local = local_size_triples(&base);
    let hier_local = local_size_triples(&hier);
    assert_eq!(
        base_local,
        vec![(64, 1, 1)],
        "H2(h): base module's OpExecutionMode LocalSize drifted from 64 1 1 -- cluster_cull.hlsl's \
         base (no `-D`) arm is `[numthreads(64, 1, 1)]`"
    );
    assert_eq!(
        hier_local,
        vec![(256, 1, 1)],
        "H2(h): HIER module's OpExecutionMode LocalSize drifted from 256 1 1 -- this is the exact \
         triple HIER_TPG's #error guard pins (cluster_cull.hlsl's `#if (HIER_TPG) != 256u`) and \
         D9's radix-16 fold hardcodes (16 folding lanes x 16 entries == 256); a mismatch here means \
         the workgroup size and the shader's own fold/mask arithmetic have come apart"
    );
}
