//! VG rung R2c0: the `vb_batch_cull.comp.spv` byte-identity gate **and its inertness pin**.
//!
//! Two independent things are checked here, and the second is the one that matters:
//!
//! * **(a) byte identity** — the committed `vb_batch_cull.comp.spv` is the re-DXC of
//!   `vb_batch_cull.comp.hlsl` under the frozen recipe in that file's own header. The
//!   `cluster_cull_spv_sync.rs` shape verbatim, scoped to this single-artifact family (no `-D`
//!   variants, so no `docs/SHADER-VARIANT-MANIFEST.md` row — the manifest registers `-D`
//!   variants only).
//!
//! * **(b) INERTNESS** — the committed module contains **no visibility decision**. Rung R2c0 is
//!   the null control `docs/VG-DECIDABILITY-FLOOR.md` says every later cull delta needs: the
//!   compaction machinery present, dispatched, and provably changing nothing. "Provably" is this
//!   test. The shader's `visible` is the literal `true`, so DXC constant-folds the ternary and
//!   the module carries `OpSelect == 0`, `OpDot == 0`, `OpFOrdLessThan == 0` — while still
//!   carrying the `OpAtomicIAdd == 1` that is the compaction claim itself. A byte-identical
//!   golden alone would NOT establish this: a cull that happens to keep every batch on today's
//!   fully-on-screen scenes is also byte-identical, and would be a control in name only.
//!
//! When rung R2c arms the frustum test, this census goes RED — correctly. R2c re-pins it against
//! its own measured module and states the new numbers; it does not delete the pin.
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

/// The rung-R2c0 inertness census. Every field is counted by EXACT whole-token match on a
/// whitespace-split line — see [`the_inertness_census_uses_whole_token_matching`] for the
/// near-miss that makes this non-negotiable rather than stylistic.
#[derive(Debug, PartialEq, Eq)]
struct SpvCensus {
    /// THE inertness field. `visible` is the literal `true`, so DXC folds
    /// `visible ? d.instance_count : 0u` to a plain load and NO `OpSelect` survives. A non-zero
    /// count means a decision entered the module.
    op_select: usize,
    /// A frustum plane test is a dot product. Zero of them on this compile.
    op_dot: usize,
    /// …and a half-space rejection is a float compare. Zero of those either.
    op_ford_less_than: usize,
    /// THE machinery field, and the reason inertness is not vacuous: the compaction claim IS
    /// present. Exactly one `InterlockedAdd` on the visible counter.
    op_atomic_iadd: usize,
    /// The tail-group range guard (`i >= pc.batch_count`).
    op_ugreater_than_equal: usize,
    /// The clamp-and-drop bound (`slot < pc.visible_cap`).
    op_uless_than: usize,
    /// The two writes this pass performs: the record's `instanceCount` and the visible-list slot.
    op_store: usize,
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

/// Gate (b): THE RUNG'S DELIVERABLE — the committed module carries no visibility decision, and
/// does carry the compaction machinery.
///
/// MEASURED on the artifact this rung commits. Do not edit these literals to make a failing run
/// pass: a non-zero `op_select` means rung R2c's decision landed, and R2c's own job includes
/// re-pinning this census with its new measured numbers and re-stating what "inert" then means
/// (nothing — R2c is not a null control, which is exactly why R2c0 exists separately).
#[test]
fn vb_batch_cull_module_is_inert() {
    let Some(spirv_dis) = find_spirv_dis() else {
        eprintln!(
            "vb_batch_cull_spv_sync: spirv-dis not found — SKIPPING the R2c0 inertness census on \
             this host."
        );
        return;
    };
    let dir = shaders_dir();
    let committed_path = dir.join("vb_batch_cull.comp.spv");
    assert!(committed_path.exists(), "missing committed {}", committed_path.display());
    let actual = census(&disassemble(&spirv_dis, &committed_path));

    let expected = SpvCensus {
        op_select: 0,
        op_dot: 0,
        op_ford_less_than: 0,
        op_atomic_iadd: 1,
        op_ugreater_than_equal: 1,
        op_uless_than: 1,
        op_store: 2,
    };
    assert_eq!(
        actual, expected,
        "vb_batch_cull.comp.spv's inertness census diverged. Expected {expected:?}, got {actual:?}."
    );

    // Stated separately from the aggregate so a failure names the PROPERTY rather than "the
    // census drifted". These two are the whole point of the rung.
    assert_eq!(
        actual.op_select, 0,
        "invariant: rung R2c0's module must carry NO `OpSelect` — the visibility decision is the \
         literal `true` and DXC must fold it away. A non-zero count means the module makes a \
         choice, and it is no longer the null control `docs/VG-DECIDABILITY-FLOOR.md` requires."
    );
    assert_eq!(
        actual.op_atomic_iadd, 1,
        "invariant: the compaction claim must be PRESENT — exactly one `InterlockedAdd` on the \
         visible counter. A count of 0 would make the inertness above vacuous: a module that does \
         nothing at all is trivially inert and de-risks nothing."
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

    // A longer mnemonic that merely CONTAINS a counted token must not match either direction.
    let compare_near_miss = "         %33 = OpFOrdLessThanEqual %bool %31 %32\n";
    assert_eq!(
        census(compare_near_miss).op_ford_less_than,
        0,
        "`OpFOrdLessThanEqual` false-matched `OpFOrdLessThan`; a module carrying a half-space \
         rejection spelled with `<=` would still read as decision-free"
    );
}
