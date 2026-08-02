//! VG rungs R2c0/R2c: the `vb_batch_cull.comp.spv` byte-identity gate **and its opcode census**.
//!
//! Two independent things are checked here, and the second is the one that matters:
//!
//! * **(a) byte identity** — the committed `vb_batch_cull.comp.spv` is the re-DXC of
//!   `vb_batch_cull.comp.hlsl` under the frozen recipe in that file's own header. The
//!   `cluster_cull_spv_sync.rs` shape verbatim, scoped to this single-artifact family (no `-D`
//!   variants, so no `docs/SHADER-VARIANT-MANIFEST.md` row — the manifest registers `-D`
//!   variants only).
//!
//! * **(b) WHAT THE MODULE DOES** — rung R2c0 pinned this as INERTNESS (no visibility decision at
//!   all); rung R2c armed the decision and RE-PINNED the same census against its own measured
//!   module. The reasoning below is kept because it is why the census exists at all. Rung R2c0 is
//!   the null control `docs/VG-DECIDABILITY-FLOOR.md` says every later cull delta needs: the
//!   compaction machinery present, dispatched, and provably changing nothing. "Provably" is this
//!   test. The shader's `visible` is the literal `true`, so DXC constant-folds the ternary and
//!   the module carries `OpSelect == 0`, `OpDot == 0`, `OpFOrdLessThan == 0` — while still
//!   carrying the `OpAtomicIAdd == 1` that is the compaction claim itself. A byte-identical
//!   golden alone would NOT establish this: a cull that happens to keep every batch on today's
//!   fully-on-screen scenes is also byte-identical, and would be a control in name only.
//!
//! Arming the frustum test at rung R2c made the R2c0 census RED — correctly, and that is what it
//! was for. R2c re-pinned it against its own measured module and stated the new numbers rather than
//! deleting the pin, so the file still discriminates: a module that silently reverted to a constant
//! `true` fails here even though it would render a byte-identical image on every pinned scene.
//!
//! Rung R2d-3 adds the per-INSTANCE compaction loop and two census fields with it — the module's
//! DECLARED BINDING SET (the "bound but unread" claim, stated instead of assumed) and
//! `OpControlBarrier` (which the region-disjointness invariant says must not exist). Its expected
//! counts ship as [`CENSUS_TBD`] and are filled from the rebuilt artifact; see that constant's own
//! doc for why a placeholder rather than a prediction.
//!
//! SKIPS (with an eprintln) when no `dxc` / `spirv-dis` resolves on the host — the byte gate is
//! only as hermetic as the pinned VulkanSDK 1.4.350.0 toolchain that produced the committed
//! artifact. The fixture control below runs unconditionally and cannot skip.

use std::path::PathBuf;
use std::process::Command;

/// The shaders directory (`CARGO_MANIFEST_DIR/shaders`), where the committed `.hlsl` and `.spv`
/// live (and where DXC must run so any `#include` resolves).
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Locates the `dxc` executable: pinned Vulkan-SDK path, then `$VULKAN_SDK/Bin`, then `PATH`.
/// `cluster_cull_spv_sync.rs`'s `find_dxc` verbatim.
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

/// Locates `spirv-dis` the same layered way. `cluster_cull_spv_sync.rs`'s `find_spirv_dis`.
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

/// Re-DXCs `hlsl_name` under the EXACT frozen recipe pinned in `vb_batch_cull.comp.hlsl`'s header
/// (`-spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3`, no `-O`) into a fresh temp `.spv`, and
/// returns the bytes. Never overwrites a committed artifact.
fn redxc(dxc: &PathBuf, dir: &PathBuf, hlsl_name: &str, out_tag: &str) -> Vec<u8> {
    let out_spv = std::env::temp_dir().join(format!("{out_tag}.redxc.spv"));
    let status = Command::new(dxc)
        .current_dir(dir)
        .args(["-spirv", "-T", "cs_6_0", "-E", "main", "-fspv-target-env=vulkan1.3", hlsl_name, "-Fo"])
        .arg(&out_spv)
        .status()
        .expect("invariant: dxc was located and must run");
    assert!(status.success(), "dxc failed re-compiling {hlsl_name} under the frozen recipe");
    let bytes = std::fs::read(&out_spv).expect("invariant: dxc wrote the re-DXC .spv");
    let _ = std::fs::remove_file(&out_spv); // best-effort tidy
    bytes
}

/// Disassembles `spv_path` via `spirv-dis`. Panics on a non-zero exit — a malformed committed
/// `.spv` is a build-integrity bug, not a skip.
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

/// The module census. Every field is counted by EXACT whole-token match on a whitespace-split
/// line — see [`the_inertness_census_uses_whole_token_matching`] for the near-miss that makes this
/// non-negotiable rather than stylistic.
///
/// The same fields serve every rung: R2c0 pinned them at "no decision at all", R2c re-pinned them
/// at "a real one", R2d-3 re-pins them again against ITS module and adds two — the declared BINDING
/// SET and `OpControlBarrier`. Which values ship is the assertion; the shape only ever grows.
#[derive(Debug, PartialEq, Eq)]
struct SpvCensus {
    /// THE decision field. Under R2c0's literal `true`, DXC folded `visible ? d.instance_count : 0u`
    /// to a plain load and NO `OpSelect` survived; under R2c's plane test exactly one does. Zero
    /// here on an armed module means the cull silently reverted to a constant.
    op_select: usize,
    /// A frustum plane test is a dot product — two per plane, in a ROLLED loop.
    op_dot: usize,
    /// …and a half-space rejection is a float compare.
    op_ford_less_than: usize,
    /// THE machinery field, and the reason neither rung's pin is vacuous: the compaction claim IS
    /// present. Exactly one `InterlockedAdd` on the visible counter — and it must hold at 1 ACROSS
    /// the arming, since R2c had no business touching R2c0's compaction.
    op_atomic_iadd: usize,
    /// The tail-group range guard (`i >= pc.batch_count`).
    op_ugreater_than_equal: usize,
    /// The clamp-and-drop bound (`slot < pc.visible_cap`), plus — since R2c — the plane
    /// loop's own bound.
    op_uless_than: usize,
    /// The two writes this pass performs: the record's `instanceCount` and the visible-list slot.
    op_store: usize,
    /// The module's declared workgroup width, read off `OpExecutionMode ... LocalSize <x> 1 1`.
    ///
    /// This is the ONE number the host and the shader must agree on that NOTHING else checks. The
    /// host dispatches `ceil(batch_count / VB_BATCH_CULL_LOCAL_SIZE_X)` groups; if the shader's
    /// `[numthreads]` were larger the tail batches would never be visited (they would keep last
    /// frame's `instanceCount` — stale draws, no crash, no validation message), and if it were
    /// smaller the host would over-dispatch groups whose lanes the range guard silently discards.
    /// HLSL requires a literal in `[numthreads]`, so the two spellings cannot be made one symbol;
    /// pinning the compiled value here is the next best thing.
    local_size_x: usize,
    /// VG rung R2d-3: the module's DECLARED BINDING SET — every `OpDecorate %x Binding <n>`,
    /// sorted and deduped.
    ///
    /// This is the field that states, rather than assumes, WHICH of the seven descriptors
    /// `vb_cull_layout` binds the module actually names. Rung R2d-2 bound @4/@5/@6 with the module
    /// declaring only @0..@3; R2d-3's HLSL declares all seven, and while `keep` is hardwired
    /// nothing loads from @4 (`gVbInstances`) or @5 (`gMeshBounds`). Whether DXC KEEPS or STRIPS a
    /// declared-but-unloaded resource is what this field REPORTS — it is measured off the built
    /// module, never predicted here. That is precisely the evidence "bound but unread" needs, and
    /// nothing else in this repository checks it.
    binding_set: Vec<usize>,
    /// VG rung R2d-3: workgroup synchronisation, which must not exist.
    ///
    /// The per-INSTANCE region write is thread-private by CONSTRUCTION (`vb_batch_cull.comp.hlsl`'s
    /// INVARIANT R2d-REGION-DISJOINT — the host's gather gives every batch a disjoint
    /// `[base, base + count)` range), so the pass needs no `groupshared`, no barrier and no atomic
    /// for it. A barrier appearing here would mean someone made the compaction shared state, which
    /// changes the cost model of the whole pass and would not show up in any image.
    op_control_barrier: usize,
}

/// Counts the census tokens in a `spirv-dis` disassembly by whole-token match.
fn census(dis: &str) -> SpvCensus {
    let mut c = SpvCensus {
        op_select: 0,
        op_dot: 0,
        op_ford_less_than: 0,
        op_atomic_iadd: 0,
        op_ugreater_than_equal: 0,
        op_uless_than: 0,
        op_store: 0,
        local_size_x: 0,
        binding_set: Vec::new(),
        op_control_barrier: 0,
    };
    for line in dis.lines() {
        for tok in line.split_whitespace() {
            match tok {
                "OpSelect" => c.op_select += 1,
                "OpDot" => c.op_dot += 1,
                "OpFOrdLessThan" => c.op_ford_less_than += 1,
                "OpAtomicIAdd" => c.op_atomic_iadd += 1,
                "OpUGreaterThanEqual" => c.op_ugreater_than_equal += 1,
                "OpULessThan" => c.op_uless_than += 1,
                "OpStore" => c.op_store += 1,
                "OpControlBarrier" => c.op_control_barrier += 1,
                _ => {}
            }
        }
        let toks: Vec<&str> = line.split_whitespace().collect();
        // `OpExecutionMode %main LocalSize 64 1 1` — the width is the token AFTER `LocalSize`.
        if let Some(i) = toks.iter().position(|t| *t == "LocalSize")
            && let Some(x) = toks.get(i + 1).and_then(|t| t.parse::<usize>().ok())
        {
            c.local_size_x = x;
        }
        // `OpDecorate %VbIndirect Binding 0` — the binding number is the token AFTER `Binding`.
        // Whole-token, so the sibling `OpDecorate %VbIndirect DescriptorSet 0` on the next line
        // cannot contribute a phantom "binding 0" (see the fixture control for that near-miss).
        if let Some(i) = toks.iter().position(|t| *t == "Binding")
            && let Some(b) = toks.get(i + 1).and_then(|t| t.parse::<usize>().ok())
        {
            c.binding_set.push(b);
        }
    }
    // Sorted + deduped so the pin is on the SET, not on DXC's emission order.
    c.binding_set.sort_unstable();
    c.binding_set.dedup();
    c
}

/// Gate (a): the committed artifact byte-equals its own re-DXC under the frozen recipe.
#[test]
fn vb_batch_cull_spv_byte_identical() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "vb_batch_cull_spv_sync: dxc not found (no C:/VulkanSDK/.../dxc.exe, no \
             $VULKAN_SDK/Bin, not on PATH) — SKIPPING the re-DXC byte-identity check on this host."
        );
        return;
    };
    let dir = shaders_dir();
    let committed_path = dir.join("vb_batch_cull.comp.spv");
    let committed = std::fs::read(&committed_path)
        .unwrap_or_else(|e| panic!("missing committed {}: {e}", committed_path.display()));
    let fresh = redxc(&dxc, &dir, "vb_batch_cull.comp.hlsl", "vb_batch_cull.comp.spv");
    assert!(
        committed == fresh,
        "vb_batch_cull.comp.spv ({} bytes committed, {} bytes fresh) is NOT the re-DXC of \
         vb_batch_cull.comp.hlsl under the frozen recipe — either the committed .spv is stale \
         (re-run the recipe in the shader's header and commit it) or the host dxc is not the \
         pinned VulkanSDK 1.4.350.0 toolchain.",
        committed.len(),
        fresh.len(),
    );
}

/// Gate (b): the module carries the rung-R2c DECISION, still carries the rung-R2c0 MACHINERY, and
/// (rung R2d-3) names exactly the descriptors it really uses with no workgroup synchronisation.
///
/// This pin was `vb_batch_cull_module_is_inert` at rung R2c0 and asserted the exact opposite
/// (`OpSelect == 0`, `OpDot == 0`, `OpFOrdLessThan == 0`). Arming the cull made that RED, which is
/// what it was for — so R2c RE-PINNED it against its own measured module rather than deleting it,
/// and R2d-3 re-pins it again.
///
/// # Why every expected number is [`CENSUS_TBD`] as this rung is authored
///
/// R2d-3 restructures the body (a per-instance loop, a relocated atomic, three new resource
/// declarations of which DXC will strip the unread ones), so no field's value survives the change
/// by inspection — not even the ones that "obviously" should not move. The numbers are read off the
/// REBUILT module and pasted in; they are not predicted here and then confirmed. Once filled they
/// are MEASURED, and the rule the previous rungs stated applies again: do not edit these literals
/// to make a failing run pass — they say what the module DOES, and a change in them is a change in
/// the cull.
#[test]
fn vb_batch_cull_module_carries_the_armed_decision() {
    let Some(spirv_dis) = find_spirv_dis() else {
        eprintln!(
            "vb_batch_cull_spv_sync: spirv-dis not found — SKIPPING the R2c arming census on this              host."
        );
        return;
    };
    let dir = shaders_dir();
    let committed_path = dir.join("vb_batch_cull.comp.spv");
    assert!(committed_path.exists(), "missing committed {}", committed_path.display());
    let actual = census(&disassemble(&spirv_dis, &committed_path));

    let expected = SpvCensus {
        // MEASURED off the built module, then pinned — never predicted. Every number below was
        // read from `spirv-dis` output after the frozen-recipe re-DXC; the rung's own instruction
        // was that predicting a census value and then confirming it is a gate wearing a
        // prediction's clothes.
        //
        // The `visible ? k : 0u` decision survived as an `OpSelect` even though the stored value
        // is now loop-carried.
        op_select: 1,
        // `dot(pl.xyz, c)` and `dot(abs(pl.xyz), h)`, in a rolled plane loop. Unmoved by R2d-3.
        op_dot: 2,
        // The `dist + radius < 0.0` rejection. Unmoved by R2d-3.
        op_ford_less_than: 1,
        // THE COMPACTION CLAIM, and the number that had to hold across this rung: R2d-3 RELOCATED
        // the single `InterlockedAdd` past the per-instance loop and did NOT add another. One,
        // exactly as R2c0 and R2c each pinned.
        op_atomic_iadd: 1,
        // The tail-group range guard `i >= pc.batch_count`.
        op_ugreater_than_equal: 1,
        // 2 -> 3: the new `j < d.instance_count` loop bound. The one-op growth is the whole
        // footprint of the per-instance loop in this census.
        op_uless_than: 3,
        // 2 -> 3: the record store and the counter slot, plus the new region write.
        op_store: 3,
        // NOT a placeholder and NOT a prediction: this field is READ FROM the host constant the
        // separate assertion below compares `actual` against, so it states the CONTRACT rather
        // than a measurement. `[numthreads]` is untouched by this rung.
        local_size_x: boyko_rhi_vulkan::compute::VB_BATCH_CULL_LOCAL_SIZE_X as usize,
        // ⚠️ THE FIELD THAT ANSWERED ITS OWN QUESTION. `vb_cull_layout` binds SEVEN descriptors
        // (@0..@6). The module names FIVE: DXC **stripped @4 (`gVbInstances`) and @5
        // (`gMeshBounds`)**, which R2d-3 declares in HLSL but never loads from while `keep` is
        // hardwired. That was an open question when this field was written — the shader header and
        // this file both deliberately said "whether DXC keeps or strips them is what this reports"
        // rather than guessing — and this measurement settles it.
        //
        // It also makes this field the load-bearing gate for the ARMING rung: when `keep` becomes
        // real, @4 and @5 acquire loads and MUST reappear here. A `[0,1,2,3,6]` still passing after
        // the arming would mean the arming shader does not read the instance rows or the bounds
        // at all — i.e. the arming silently did nothing, which is precisely the failure a golden
        // on an all-on-screen corpus cannot see.
        binding_set: vec![0, 1, 2, 3, 6],
        // Measured zero, as the construction implies: no `groupshared`, no barrier intrinsic, and
        // the region write is thread-private.
        op_control_barrier: 0,
    };
    assert_eq!(
        actual, expected,
        "vb_batch_cull.comp.spv's census diverged. Expected {expected:?}, got {actual:?}."
    );

    // Stated separately because it is a HOST/SHADER CONTRACT, not a property of the module alone.
    // `boyko_rhi_vulkan::compute::VB_BATCH_CULL_LOCAL_SIZE_X` sizes the dispatch; a mismatch either
    // leaves tail batches unvisited (stale `instanceCount`, so last frame's draw count silently
    // persists) or over-dispatches groups the range guard discards. Neither shows up in a golden on
    // a scene whose batch count is a multiple of the width, which is most of them.
    assert_eq!(
        actual.local_size_x, boyko_rhi_vulkan::compute::VB_BATCH_CULL_LOCAL_SIZE_X as usize,
        "invariant: the shader's [numthreads] width must equal the host's dispatch divisor"
    );

    // DIRECTIONAL properties, stated separately so a failure names the PROPERTY rather than "the
    // census drifted". These are claims about the SOURCE surviving compilation, not counts:
    // `OpSelect` is deliberately NOT among them any more, because a loop-carried value's ternary
    // may lawfully become a branch — the plane ARITHMETIC is what proves the decision is still
    // computed at all.
    assert!(
        actual.op_dot > 0 && actual.op_ford_less_than > 0,
        "invariant: the module must still carry a real plane test. Both at zero means the cull          reverted to R2c0's constant `true`, which renders identically on today's fully-on-screen          scenes and would therefore pass every golden while culling nothing."
    );
    assert!(
        actual.op_atomic_iadd > 0,
        "invariant: the compaction claim must SURVIVE rung R2d-3's restructuring — the          `InterlockedAdd` was MOVED past the per-instance loop, not removed. Its exact          multiplicity is pinned by the census above once that is filled from the module."
    );
    // A statement about the SOURCE, not a predicted count: `vb_batch_cull.comp.hlsl` contains no
    // barrier intrinsic and no `groupshared`, and DXC does not synthesise workgroup
    // synchronisation. The region write is thread-private by construction (INVARIANT
    // R2d-REGION-DISJOINT), so a barrier here would mean the compaction became shared state.
    assert_eq!(
        actual.op_control_barrier, 0,
        "invariant: the per-INSTANCE compaction is region-addressed and thread-private — it needs          no workgroup barrier, and one appearing here means someone made it shared state."
    );
}

/// FIXTURE CONTROL for [`census`]'s selectors, and it is not decorative.
///
/// # The near-miss that inverts the pin
///
/// `OpSelectionMerge` has `OpSelect` as a strict prefix, and the committed module contains
/// **three** of them (MEASURED) against **zero** real `OpSelect`. A substring selector would
/// therefore read the inertness field as `3` on a module that is perfectly inert — reporting a
/// decision that is not there, and, phrased the other way round ("non-zero ⇒ armed"), certifying
/// an *armed* cull as the null control. The whole-token form is what separates the two.
///
/// Runs unconditionally — no `dxc` / `spirv-dis`, so it cannot SKIP the way the artifact gates do.
#[test]
fn the_inertness_census_uses_whole_token_matching() {
    // The real near-miss, verbatim from `spirv-dis vb_batch_cull.comp.spv`.
    let selection_merge = "               OpSelectionMerge %30 None\n";
    assert_eq!(
        census(selection_merge).op_select,
        0,
        "`OpSelectionMerge` was counted as an `OpSelect`; the inertness pin would then read 3 on \
         the very module it certifies as inert"
    );

    // …and the selector must still SEE a real one, or `== 0` is satisfied by blindness.
    let real_select = "         %42 = OpSelect %uint %41 %40 %uint_0\n";
    assert_eq!(
        census(real_select).op_select,
        1,
        "the selector missed a REAL `OpSelect` — the inertness assertion is vacuous and would \
         stay green after rung R2c arms the decision"
    );

    // The same both-directions check for the machinery field: `OpAtomicIAdd` must not be
    // confused with the other atomic bump DXC can emit for an increment-by-one.
    let real_atomic = "         %51 = OpAtomicIAdd %uint %50 %uint_1 %uint_0 %uint_1\n";
    assert_eq!(census(real_atomic).op_atomic_iadd, 1, "the selector missed a REAL `OpAtomicIAdd`");
    let other_atomic = "         %51 = OpAtomicIIncrement %uint %50 %uint_1 %uint_0\n";
    assert_eq!(
        census(other_atomic).op_atomic_iadd,
        0,
        "`OpAtomicIIncrement` was counted as the `InterlockedAdd` claim; the machinery pin would \
         then be satisfied by an instruction the shader does not emit"
    );

    // The LocalSize selector must read the WIDTH, not the line's other numbers, and must not fire
    // on a line that merely mentions the token.
    assert_eq!(
        census("               OpExecutionMode %main LocalSize 64 1 1
").local_size_x,
        64,
        "the selector missed the real LocalSize width"
    );
    assert_eq!(
        census("               OpExecutionMode %main LocalSize 128 1 1
").local_size_x,
        128,
        "the selector is returning a constant rather than reading the width"
    );

    // A longer mnemonic that merely CONTAINS a counted token must not match either direction.
    let compare_near_miss = "         %33 = OpFOrdLessThanEqual %bool %31 %32\n";
    assert_eq!(
        census(compare_near_miss).op_ford_less_than,
        0,
        "`OpFOrdLessThanEqual` false-matched `OpFOrdLessThan`; a module carrying a half-space \
         rejection spelled with `<=` would still read as decision-free"
    );

    // === VG rung R2d-3's two new selectors, both directions. ===

    // The BINDING SET reads the number after `Binding`, and must not be fooled by the
    // `DescriptorSet` decoration that always sits beside it — that near-miss would inject a
    // phantom binding 0 into EVERY module and make the set field unable to distinguish "@0 is
    // declared" from "@0 is not".
    let decorations = "               OpDecorate %VbIndirect DescriptorSet 0
               OpDecorate %VbIndirect Binding 0
               OpDecorate %VbVisibleInstance DescriptorSet 0
               OpDecorate %VbVisibleInstance Binding 6
";
    assert_eq!(
        census(decorations).binding_set,
        vec![0, 6],
        "the binding-set selector must read the number after `Binding` only — a `DescriptorSet 0` \
         counted as a binding would put a phantom @0 in every module's set"
    );
    assert!(
        census("               OpDecorate %x DescriptorSet 3\n").binding_set.is_empty(),
        "a lone `DescriptorSet` decoration declares no binding"
    );
    // Sorted + deduped: the pin is on the SET, so DXC's emission order must not matter.
    let out_of_order = "               OpDecorate %b Binding 6
               OpDecorate %a Binding 1
               OpDecorate %a Binding 1
";
    assert_eq!(
        census(out_of_order).binding_set,
        vec![1, 6],
        "the binding set must be order-independent and duplicate-free, or a re-ordered emission \
         reads as a changed module"
    );

    // The barrier selector must see a REAL `OpControlBarrier` and must not fire on the memory
    // barrier DXC emits for a plain `DeviceMemoryBarrier` — the two are different claims about the
    // pass, and only the first one means "the compaction became shared state".
    assert_eq!(
        census("               OpControlBarrier %uint_2 %uint_2 %uint_264\n").op_control_barrier,
        1,
        "the selector missed a REAL `OpControlBarrier`; the no-synchronisation invariant would be \
         satisfied by blindness"
    );
    assert_eq!(
        census("               OpMemoryBarrier %uint_1 %uint_72\n").op_control_barrier,
        0,
        "`OpMemoryBarrier` was counted as a control barrier; the invariant is about workgroup \
         SYNCHRONISATION, not about memory ordering"
    );
}
