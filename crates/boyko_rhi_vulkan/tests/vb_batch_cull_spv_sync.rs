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
/// The same seven fields serve both rungs: R2c0 pinned them at "no decision at all", R2c re-pinned
/// them at "a real one". Which values ship is the assertion; the shape does not change.
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
                _ => {}
            }
        }
        // `OpExecutionMode %main LocalSize 64 1 1` — the width is the token AFTER `LocalSize`.
        let toks: Vec<&str> = line.split_whitespace().collect();
        if let Some(i) = toks.iter().position(|t| *t == "LocalSize")
            && let Some(x) = toks.get(i + 1).and_then(|t| t.parse::<usize>().ok())
        {
            c.local_size_x = x;
        }
    }
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

/// Gate (b): the module carries the rung-R2c DECISION, and still carries the rung-R2c0 MACHINERY.
///
/// This pin was `vb_batch_cull_module_is_inert` at rung R2c0 and asserted the exact opposite
/// (`OpSelect == 0`, `OpDot == 0`, `OpFOrdLessThan == 0`). Arming the cull made that RED, which is
/// what it was for — so R2c RE-PINS it against its own measured module rather than deleting it.
/// The one field that must NOT have moved is `op_atomic_iadd`: the compaction claim is R2c0's
/// contribution and R2c was not supposed to touch it, so holding it at 1 across the arming is a
/// cross-rung invariant rather than a restatement.
///
/// MEASURED on the artifact this rung commits. Do not edit these literals to make a failing run
/// pass: they say what the module DOES, and a change in them is a change in the cull.
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
        // The ternary on `visible` no longer folds — the decision is real.
        op_select: 1,
        // `dot(pl.xyz, c)` and `dot(abs(pl.xyz), h)`, in a ROLLED loop (2, not 12).
        op_dot: 2,
        // The single `dist + radius < 0.0` rejection.
        op_ford_less_than: 1,
        // Unchanged from R2c0 — see this test's doc.
        op_atomic_iadd: 1,
        op_ugreater_than_equal: 1,
        // The visible-list clamp, plus the plane loop's own bound.
        op_uless_than: 2,
        op_store: 2,
        // Must equal the host's `VB_BATCH_CULL_LOCAL_SIZE_X`, asserted by name below.
        local_size_x: 64,
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

    // Stated separately so a failure names the PROPERTY rather than "the census drifted".
    assert!(
        actual.op_select > 0 && actual.op_dot > 0 && actual.op_ford_less_than > 0,
        "invariant: rung R2c's module must carry a real decision — an `OpSelect` fed by a plane          test. All three at zero means the cull silently reverted to R2c0's constant `true`, which          renders identically on today's fully-on-screen scenes and would therefore pass every          golden while culling nothing."
    );
    assert_eq!(
        actual.op_atomic_iadd, 1,
        "invariant: the compaction claim must SURVIVE the arming — exactly one `InterlockedAdd`,          the same count rung R2c0 pinned. R2c was not supposed to touch the machinery."
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
}
