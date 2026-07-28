//! The froxel light-index bound is derived from the ALLOCATION, and its cost stays out of the
//! light loop. Both halves are asserted here, statically, against the committed `.spv`.
//!
//! # What the shaders actually do, and why it needed measuring
//!
//! `cluster_cull.hlsl` hands out disjoint slices of the flat `LightIndexList` with one
//! `InterlockedAdd`, clamps the claim to `pc.index_list_cap`, and records the **trimmed** count
//! into `ClusterGrid[fi]`. Every reader then walks `LightIndexList[offset .. offset + count)`.
//! That chain is sound only if two things hold:
//!
//! 1. `index_list_cap` is the buffer's real length rather than a number that merely accompanies
//!    it. It is: `gpu_scene` sizes the SSBO as `cluster_config.index_list_cap * 4` and fills
//!    `ClusterCullPush` from the same field of the same struct, in the same function.
//! 2. The reader's own guard is allocation-derived, because `robustBufferAccess` is OFF on this
//!    device and an out-of-range read would return garbage silently rather than zero. It is:
//!    every froxel reader gates `use_clusters` on `cluster_count <= grid_capacity`, where
//!    `grid_capacity` comes from `ClusterGrid.GetDimensions(...)` — SPIR-V **`OpArrayLength`**,
//!    which reports the bound descriptor's own element count.
//!
//! So the safety of the whole path rests on an instruction whose cost nobody had measured, on a
//! path that runs per fragment (`forward_opaque_froxel`) and per invocation (the `deferred_pbr`
//! and `vb_shade`/`vb_resolve` families). The measurement is what this file pins, and it is
//! **static** on purpose: this repository's GPU bench does not reproduce above N = 128 and carries
//! a 21% run-to-run spread, which is orders of magnitude above one scalar descriptor load. A
//! timing harness could not have answered the question; the disassembly answers it exactly.
//!
//! # The measured answer
//!
//! Nine committed variants carry an `OpArrayLength`, **exactly one each**, and in every one it
//! sits **outside every structured loop** — so it is one query per invocation, never per light and
//! never per loop iteration. That is the property worth defending: moving the `GetDimensions` call
//! inside the light loop, or adding a second query, would cost real work per iteration and no
//! existing gate would notice. Byte-identity gates would not: they pin the `.spv` against a re-DXC
//! of the *current* source, so a source change that moves the query is re-blessed and stays green.
//!
//! # How "inside a loop" is decided
//!
//! A SPIR-V structured loop spans from its `OpLoopMerge` to the `OpLabel` of the merge block it
//! names. An instruction is inside that loop iff it falls in the span. Nesting needs no special
//! handling — every span is tested independently, and being inside any one of them is enough.
//!
//! SKIPS (with an `eprintln`) when no `spirv-dis` resolves on the host, the same discipline the
//! `*_spv_sync` byte gates use for `dxc`: a gate that cannot run says so rather than passing.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

/// Shaders that must carry exactly one `OpArrayLength`, outside every loop.
///
/// Pinned as an exact set, in both directions, for the same reason the campaign's other baselines
/// are: a variant that GAINS the query is a new hot-path cost to account for, and one that LOSES
/// it has lost the allocation-derived guard and is now bounded by arithmetic alone.
const BOUND_BY_ARRAYLENGTH: [&str; 9] = [
    "cluster_cull.comp.spv",
    "deferred_pbr.comp.spv",
    "deferred_pbr_hwrt.comp.spv",
    "deferred_pbr_hwrt_denoised.comp.spv",
    "deferred_pbr_wrap.comp.spv",
    "forward_opaque_froxel.fs.spv",
    "vb_resolve_froxel.comp.spv",
    "vb_shade_froxel.comp.spv",
    "vb_shade_tex_froxel.comp.spv",
];

/// Locates `spirv-dis`: the pinned Vulkan-SDK path first (the repo's offline recipe), then
/// `$VULKAN_SDK/Bin`, then `PATH`. `None` makes this test SKIP.
fn find_spirv_dis() -> Option<PathBuf> {
    let pinned = PathBuf::from("C:/VulkanSDK/1.4.350.0/Bin/spirv-dis.exe");
    if pinned.is_file() {
        return Some(pinned);
    }
    let bare = if cfg!(windows) {
        "spirv-dis.exe"
    } else {
        "spirv-dis"
    };
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let p = PathBuf::from(sdk).join("Bin").join(bare);
        if p.is_file() {
            return Some(p);
        }
    }
    Command::new(bare)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| PathBuf::from(bare))
}

fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// `(line index of each OpArrayLength, spans of every structured loop)` for one module.
fn disassemble(dis: &PathBuf, spv: &PathBuf) -> (Vec<usize>, Vec<(usize, usize)>) {
    let out = Command::new(dis)
        .arg(spv)
        .output()
        .expect("invariant: spirv-dis was located and must run");
    assert!(
        out.status.success(),
        "spirv-dis failed on {}",
        spv.display()
    );
    let asm = String::from_utf8(out.stdout).expect("spirv-dis emits UTF-8");
    parse_asm(&asm)
}

/// The containment decision, split out from the process call so it can be falsified on synthetic
/// input. A gate whose verdict is only ever observed on inputs that pass it is not a gate.
fn parse_asm(asm: &str) -> (Vec<usize>, Vec<(usize, usize)>) {
    let lines: Vec<&str> = asm.lines().collect();

    let mut arraylen = Vec::new();
    let mut label_line: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, l) in lines.iter().enumerate() {
        if l.contains("OpArrayLength") {
            arraylen.push(i);
        }
        // `%295 = OpLabel`
        if let Some(rest) = l.trim_start().strip_prefix('%')
            && let Some((name, tail)) = rest.split_once(" = ")
            && tail.trim() == "OpLabel"
        {
            label_line.insert(&l.trim_start()[..name.len() + 1], i);
        }
    }

    let mut spans = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        // `OpLoopMerge %merge %continue None`
        if let Some(pos) = l.find("OpLoopMerge ") {
            let after = &l[pos + "OpLoopMerge ".len()..];
            let merge = after.split_whitespace().next().unwrap_or_default();
            if let Some(&end) = label_line.get(merge) {
                spans.push((i, end));
            }
        }
    }
    (arraylen, spans)
}

#[test]
fn the_froxel_bound_query_is_one_per_invocation_and_outside_every_loop() {
    let Some(dis) = find_spirv_dis() else {
        eprintln!(
            "SKIP the_froxel_bound_query_is_one_per_invocation_and_outside_every_loop: no \
             spirv-dis on this host (tried the pinned SDK path, $VULKAN_SDK/Bin and PATH). The \
             hot-path property is NOT checked in this run."
        );
        return;
    };

    let dir = shaders_dir();
    let mut carriers = Vec::new();
    let mut report = String::new();

    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("invariant: the shaders directory exists")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "spv"))
        .collect();
    entries.sort();
    assert!(
        !entries.is_empty(),
        "no .spv found in {} — a gate that scans an empty set is not a gate",
        dir.display()
    );

    for spv in &entries {
        let (arraylen, spans) = disassemble(&dis, spv);
        if arraylen.is_empty() {
            continue;
        }
        let name = spv
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        carriers.push(name.clone());

        if arraylen.len() != 1 {
            report.push_str(&format!(
                "  {name}: {} OpArrayLength, expected exactly 1 — every extra one is another \
                 descriptor query on the hot path\n",
                arraylen.len()
            ));
        }
        for a in &arraylen {
            let enclosing: Vec<&(usize, usize)> =
                spans.iter().filter(|(s, e)| s < a && a < e).collect();
            if !enclosing.is_empty() {
                report.push_str(&format!(
                    "  {name}: OpArrayLength at asm line {} is INSIDE {} structured loop(s) — it \
                     is the bound query for the froxel light walk and belongs before the loop, \
                     not in it\n",
                    a + 1,
                    enclosing.len()
                ));
            }
        }
        println!("{name}: 1 OpArrayLength, outside every loop ({} loops)", spans.len());
    }

    assert!(
        report.is_empty(),
        "the froxel bound query moved onto the per-iteration path.\n{report}"
    );

    let expected: Vec<String> = BOUND_BY_ARRAYLENGTH.iter().map(|s| (*s).to_string()).collect();
    assert_eq!(
        carriers, expected,
        "the set of shaders carrying an allocation-derived bound query moved.\n\
         GAINED one: a new hot-path descriptor query to account for — add it to \
         BOUND_BY_ARRAYLENGTH in the same commit.\n\
         LOST one: that shader's `use_clusters` guard is no longer derived from the bound \
         descriptor's own length, so its `ClusterGrid`/`LightIndexList` reads are bounded by \
         arithmetic alone — and `robustBufferAccess` is OFF, so an out-of-range read returns \
         garbage silently."
    );
}

/// Sensitivity control: the containment test must actually detect a query inside a loop.
///
/// The live run above is green on all nine shaders, so it never exercises the failing branch —
/// exactly the shape that let a dead range check sit in this repository's anchors gate for a whole
/// revision while its own recorded measurement was true. This drives `parse_asm` with synthetic
/// disassembly in three arrangements and checks the verdict flips where it must.
#[test]
fn the_containment_test_detects_a_query_moved_into_the_loop() {
    // `%m` is the merge block; the loop spans OpLoopMerge..%m's OpLabel.
    let inside = "\
%entry = OpLabel
       OpBranch %head
%head = OpLabel
       OpLoopMerge %m %cont None
       OpBranch %body
%body = OpLabel
 %len = OpArrayLength %uint %ClusterGrid 0
       OpBranch %cont
%cont = OpLabel
       OpBranch %head
   %m = OpLabel
       OpReturn
";
    let (arr, spans) = parse_asm(inside);
    assert_eq!(arr.len(), 1, "one query in the synthetic module");
    assert_eq!(spans.len(), 1, "one structured loop in the synthetic module");
    let a = arr[0];
    assert!(
        spans.iter().any(|(s, e)| *s < a && a < *e),
        "a query between OpLoopMerge and its merge label MUST read as inside the loop — this is \
         the only branch that makes the live assertion mean anything"
    );

    // The same module with the query hoisted above the loop: must read as outside.
    let outside = "\
%entry = OpLabel
 %len = OpArrayLength %uint %ClusterGrid 0
       OpBranch %head
%head = OpLabel
       OpLoopMerge %m %cont None
       OpBranch %body
%body = OpLabel
       OpBranch %cont
%cont = OpLabel
       OpBranch %head
   %m = OpLabel
       OpReturn
";
    let (arr2, spans2) = parse_asm(outside);
    let a2 = arr2[0];
    assert!(
        !spans2.iter().any(|(s, e)| *s < a2 && a2 < *e),
        "a query before the loop header must read as outside — otherwise the live run is green \
         for the wrong reason"
    );

    // A query AFTER the merge label of a closed loop is also outside. This is the arrangement the
    // real shaders have (`forward_opaque_froxel`: two loops close at asm lines 536 and 1127, the
    // query sits at 1143, the light loop opens at 1235), so if this case were mis-decided the
    // whole measurement would invert.
    let after = "\
%entry = OpLabel
       OpBranch %head
%head = OpLabel
       OpLoopMerge %m %cont None
       OpBranch %body
%body = OpLabel
       OpBranch %cont
%cont = OpLabel
       OpBranch %head
   %m = OpLabel
 %len = OpArrayLength %uint %ClusterGrid 0
       OpReturn
";
    let (arr3, spans3) = parse_asm(after);
    let a3 = arr3[0];
    assert_eq!(spans3.len(), 1);
    assert!(
        !spans3.iter().any(|(s, e)| *s < a3 && a3 < *e),
        "a query after a closed loop's merge label must read as outside"
    );
}
