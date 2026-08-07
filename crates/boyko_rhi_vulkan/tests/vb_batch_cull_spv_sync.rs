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
//! `OpControlBarrier` (which the region-disjointness invariant says must not exist).
//!
//! **Rung R2d-6 ARMS the level-2 predicate**, and this file is where the arming is proved to have
//! HAPPENED at all. Two of its fields carry that proof and are stated as REQUIREMENTS rather than
//! measurements — the binding set must GAIN @4/@5 (R2d-3 measured that DXC strips a
//! declared-but-unloaded resource, so their absence was the signature of the inert module, and
//! their return is the signature of a module that really reads the instance rows and the bounds),
//! and `OpAtomicIAdd` must still be exactly 1. Every other count ships as [`CENSUS_TBD`] and is
//! filled from the rebuilt artifact; see that constant's own doc for why a placeholder rather than
//! a prediction.
//!
//! # VG R3 piece 3 step P3-3: the PREDICTION, written down before the artifact was rebuilt
//!
//! P3-3 edits this module in exactly two places — `VbBatchCullPush` gains `phase` and `occ_flags`
//! (104 → 112 bytes), and `main` gains `if (pc.phase != VB_CULL_PHASE_EARLY) return;` immediately
//! after the tail-lane guard. **Every census field below is predicted UNMOVED**, and the prediction
//! is derived rather than hoped:
//!
//! * Two extra push members produce two extra `OpMemberDecorate … Offset` lines and nothing else.
//!   No census selector reads a member decoration — [`SpvCensus::binding_set`] reads the token after
//!   `Binding`, which a member offset never emits.
//! * The fork lowers to a push load, an `OpINotEqual`, an `OpSelectionMerge` +
//!   `OpBranchConditional` and an `OpReturn`. None of those is a counted token, and
//!   [`the_inertness_census_uses_whole_token_matching`] already proves `OpSelectionMerge` is not
//!   read as an `OpSelect` — that near-miss is the reason the selectors are whole-token.
//! * No resource is loaded that was not loaded before, so `binding_set` stays `[0,1,2,3,4,5,6]`.
//!   The five descriptors P3-2 bound are still unread; the step that makes them appear is P3-4.
//!
//! **The two fields with a non-zero risk of moving anyway, named so a movement is diagnosed instead
//! of blessed:** `op_select` and `op_ugreater_than_equal`, both because the module now has TWO early
//! returns in a row and DXC is free to fold them into one predicate. A fold would keep the count at
//! one `OpUGreaterThanEqual` and add an `OpLogicalOr`, which is uncounted — but a lowering that
//! reached for `OpSelect` instead would move `op_select` 1 → 2. **If either moves, the number is
//! MEASURED off the rebuilt module and re-pinned with the reason stated**, exactly as rungs R2c and
//! R2d-6 did. Editing an expectation to make a failing run pass is what this file exists to prevent.
//!
//! # VG R3 piece 3 step P3-4: the DERIVATION, written down before the artifact was rebuilt
//!
//! P3-4 gives the phase fork its two bodies — the occlusion leaf, the early two-way partition and
//! the late in-place compaction — and loads the five descriptors P3-2 bound. The delta is derived
//! per field rather than predicted wholesale, because only two classes of field are derivable:
//!
//! * **REQUIREMENTS, asserted** — `binding_set` becomes `[0..=11]` (each of the five new bindings in
//!   its OWN named assertion, because a joint `contains(&a) && contains(&b)` cannot say WHICH one is
//!   missing); `op_atomic_iadd` stays 1 (a partition is not a compaction change);
//!   `op_control_barrier` stays 0 (no `groupshared`, no barrier intrinsic); and the TWO NEW
//!   no-sampler pins are 0 by construction — the module declares no `SamplerState` and calls no
//!   `.Sample*`, only `.Load`.
//! * **COUNTS, [`CENSUS_TBD`]** — every arithmetic and control-flow field. Their DIRECTION is
//!   derivable and is stated at each field, but not their value:
//!   - `op_dot` is expected to RISE, because the six Arvo `dot()`s moved into `arvo_world_box` and
//!     that function is called from BOTH phases. ⚠️ **It is NOT asserted 0 and must never be.** The
//!     "no `dot()` in the projection" property is NOT expressible here — this census counts
//!     MODULE-WIDE, in a flat token loop, and the calls it would have to exclude are in code this
//!     step leaves unqualified on purpose. Scoping to a function range is not merely unbuilt, it is
//!     UNREACHABLE: a byte scan of the committed artifact finds exactly ONE `OpFunction` header,
//!     because DXC inlines every helper into `%main`. What stands in for it is
//!     [`the_projection_fold_is_written_out_and_carries_no_dot`] at the SOURCE level plus the new
//!     `no_contraction` count at the artifact level — and both limits are stated on those gates.
//!   - `op_select`, `op_uless_than`, `op_ugreater_than_equal`, `op_ford_less_than` and `op_store` all
//!     rise by amounts DXC decides: the corner `? :` triple, `hzb_msb`'s zero guard, the disarmed
//!     address mask, `hzb_conservative_min`'s compare-and-select at four inlined call sites, two new
//!     loops and five new stores may each lower to a select or to a branch-and-phi.
//!   - `no_contraction` is a NEW field and is MEASURED. It is the only artifact-level evidence that
//!     `precise` survived DXC at all; control D6 of the differential (drop `precise` from the
//!     projection locals) must MOVE it, and if it does not, `precise` is not reaching the artifact
//!     and THAT is the finding.
//!
//! SKIPS (with an eprintln) when no `dxc` / `spirv-dis` resolves on the host — the byte gate is
//! only as hermetic as the pinned VulkanSDK 1.4.350.0 toolchain that produced the committed
//! artifact. The fixture control below and the SOURCE-level fold gate run unconditionally and
//! cannot skip.

use std::path::PathBuf;
use std::process::Command;

/// **A census count that has not been MEASURED yet.**
///
/// `usize::MAX` cannot be mistaken for a plausible opcode count and cannot satisfy any equality
/// below, so an unfilled field fails loudly rather than asserting something convenient.
///
/// # Why a placeholder rather than a prediction
///
/// Rung R2d-6 replaces the level-2 `keep` expression with a real per-instance frustum test. That
/// adds two resource loads, a vector compare, a second call site for `aabb_outside_frustum` and an
/// Arvo fold — and NO field's value survives that by inspection, not even the ones that
/// "obviously" should not move (DXC may inline the plane test at one site and not the other,
/// unroll or rotate the loop, or lower the `keep` seed to a phi instead of a select). Predicting a
/// census value and then confirming it is a gate wearing a prediction's clothes; the numbers are
/// read off the REBUILT module and pasted in.
///
/// The two fields that are NOT placeholders are the two the arming OWES this file — see
/// [`vb_batch_cull_module_carries_the_armed_decision`].
const CENSUS_TBD: usize = usize::MAX;

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
    redxc_defines(dxc, dir, hlsl_name, out_tag, &[])
}

/// [`redxc`] with extra `-D` arguments, for the `-D`-variant rows of
/// `docs/SHADER-VARIANT-MANIFEST.md`. `defines` are passed VERBATIM after the frozen flags, in the
/// order the variant's own header recipe spells them — a re-ordered command line is a different
/// artifact for a compiler that embeds nothing, but the equality this gates is on the OUTPUT, so
/// the order is fixed here only to keep the recipe and the gate one text.
fn redxc_defines(
    dxc: &PathBuf,
    dir: &PathBuf,
    hlsl_name: &str,
    out_tag: &str,
    defines: &[&str],
) -> Vec<u8> {
    let out_spv = std::env::temp_dir().join(format!("{out_tag}.redxc.spv"));
    let status = Command::new(dxc)
        .current_dir(dir)
        .args(["-spirv", "-T", "cs_6_0", "-E", "main", "-fspv-target-env=vulkan1.3"])
        .args(defines.iter().flat_map(|d| ["-D", d]))
        .args([hlsl_name, "-Fo"])
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
/// at "a real one", R2d-3 re-pinned them against ITS module and added two — the declared BINDING
/// SET and `OpControlBarrier` — and R2d-6 re-pins them against the ARMED one. Which values ship is
/// the assertion; the shape only ever grows.
#[derive(Debug, PartialEq, Eq)]
struct SpvCensus {
    /// THE decision field. Under R2c0's literal `true`, DXC folded `visible ? d.instance_count : 0u`
    /// to a plain load and NO `OpSelect` survived; under R2c's plane test exactly one does. Zero
    /// here on an armed module means the LEVEL-1 decision reverted to a constant — the level-2 one
    /// may lawfully lower to a branch-and-phi instead, which is why the binding set rather than
    /// this field is what proves the R2d-6 arming.
    op_select: usize,
    /// A frustum plane test is a dot product — two per plane, in a ROLLED loop. Since R2d-6 the
    /// same test has a second call site (per INSTANCE) and the Arvo fold adds six more.
    op_dot: usize,
    /// …and a half-space rejection is a float compare.
    op_ford_less_than: usize,
    /// **THE OCCLUSION VERDICT'S OWN OPCODE**, and the field that exists because its absence was a
    /// measured blind spot. P3-4 replaced `depth_near < occ` with the division-free universal test
    /// `for all i: cz_i < occ * cw_i`, written `!(cz < bound)` so that a NaN exits KEEP. DXC lowers
    /// that negation to `OpFUnordGreaterThanEqual` — a DIFFERENT opcode — so `op_ford_less_than`
    /// went DOWN by two (one per inlined copy of the leaf) at the exact moment the decision changed.
    /// A census that counted only the ordered compare would therefore have read a verdict's removal
    /// as a small decrease and pinned it without comment. Two of the four are the verdict itself,
    /// one per inlined copy; the loop is ROLLED, so eight corners contribute one compare each.
    ///
    /// It is also the artifact-level evidence for the NaN claim: the unordered form is what makes
    /// `!(NaN < x)` TRUE, and an ordered `OpFOrdGreaterThanEqual` here would silently invert it.
    op_funord_greater_than_equal: usize,
    /// THE machinery field, and the reason no rung's pin is vacuous: the compaction claim IS
    /// present. Exactly one `InterlockedAdd` on the visible counter — and it must hold at 1 ACROSS
    /// every arming, since neither R2c nor R2d-6 had business touching R2c0's compaction.
    op_atomic_iadd: usize,
    /// The tail-group range guard (`i >= pc.batch_count`).
    op_ugreater_than_equal: usize,
    /// The clamp-and-drop bound (`slot < pc.visible_cap`), the plane loop's own bound (since R2c)
    /// and the per-instance loop's (since R2d-3).
    op_uless_than: usize,
    /// The writes this pass performs: the record's `instanceCount`, the visible-list slot and —
    /// since R2d-3 — the per-instance survivor region.
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
    /// This is the field that states, rather than assumes, WHICH of the descriptors
    /// `vb_cull_layout` binds the module actually names — seven of them at rung R2d-6, TWELVE since
    /// VG R3 piece 3 step P3-2 widened the host layout while leaving the module alone. Rung R2d-2
    /// bound @4/@5/@6 with the module
    /// declaring only @0..@3; R2d-3's HLSL declared all seven while loading from neither @4
    /// (`gVbInstances`) nor @5 (`gMeshBounds`), and this field MEASURED the answer to "does DXC
    /// keep or strip a declared-but-unloaded resource": it strips them, and that module's set was
    /// `[0,1,2,3,6]`.
    ///
    /// **That measurement is what makes this the arming rung's load-bearing field.** Rung R2d-6's
    /// armed predicate loads from both, so both must reappear — and a module that still reported
    /// five bindings would be one whose arming compiled away, which is invisible to every golden on
    /// an all-on-screen corpus. Nothing else in this repository checks it.
    binding_set: Vec<usize>,
    /// VG rung R2d-3: workgroup synchronisation, which must not exist.
    ///
    /// The per-INSTANCE region write is thread-private by CONSTRUCTION (`vb_batch_cull.comp.hlsl`'s
    /// INVARIANT R2d-REGION-DISJOINT — the host's gather gives every batch a disjoint
    /// `[base, base + count)` range), so the pass needs no `groupshared`, no barrier and no atomic
    /// for it. A barrier appearing here would mean someone made the compaction shared state, which
    /// changes the cost model of the whole pass and would not show up in any image.
    op_control_barrier: usize,
    /// VG R3 piece 3 step P3-4 (plan D7): `OpTypeSampler` declarations, which must be ZERO.
    ///
    /// The pyramid is read POINT-SAMPLED — `.Load(int3(x, y, level))`, integer coordinates and an
    /// explicit mip — and **no `VkSampler` is created anywhere in this piece**. That is a SOUNDNESS
    /// property, not a tidiness one: a min-reduced pyramid stores a bound over a footprint, not a
    /// band-limited signal, so a bilinear blend of four reduced texels is a convex combination lying
    /// strictly between their min and max. Under reverse-Z the stored value must be `<=` every depth
    /// in the footprint; a blend can be GREATER, and would therefore reject something VISIBLE.
    ///
    /// ⚠️ Zero here is not satisfied by blindness, and the reason is the field above it: DXC STRIPS a
    /// declared-but-unloaded resource (MEASURED at rung R2d-3), so `binding_set` containing @9 is
    /// what proves the module really taps the pyramid. This field then says the tap is a FETCH.
    op_type_sampler: usize,
    /// VG R3 piece 3 step P3-4 (plan D7): every `OpImageSample*` opcode, which must be ZERO.
    ///
    /// Counted by PREFIX rather than by whole token, deliberately and against this file's usual
    /// rule: there are a dozen `OpImageSample…` mnemonics (`ImplicitLod`, `ExplicitLod`,
    /// `DrefImplicitLod`, `ProjDrefExplicitLod`, the `Sparse*` family …) and the claim is about ALL
    /// of them. The near-miss the whole-token rule exists to stop cannot arise in this direction —
    /// no non-sampling SPIR-V opcode begins with `OpImageSample` — and the fixture control asserts
    /// both halves: a real `OpImageSampleExplicitLod` counts, and `OpImageFetch` (which IS what
    /// `.Load` emits, and which must stay legal) does not.
    op_image_sample: usize,
    /// VG R3 piece 3 step P3-4 (plan D11): `OpDecorate <target> NoContraction`, MEASURED.
    ///
    /// This is the only artifact-level evidence that `precise` on the projection fold survived DXC.
    /// `precise` forbids CONTRACTION (fusing `a*b + c` into an FMA), which is decided BELOW the
    /// `.spv` where no byte gate can see it — so a decoration count is the last point at which the
    /// intent is still observable.
    ///
    /// ⚠️ What it CANNOT claim: WHICH nodes carry the decoration. It counts, it does not locate. A
    /// refactor that moved `precise` from the projection to some other expression would keep this
    /// number and lose the property. The differential's control D6 (drop `precise` from the
    /// projection locals) must MOVE it; a control that leaves it unmoved means `precise` is not
    /// reaching the artifact, and that — not the differential's own result — is the finding.
    no_contraction: usize,
}

/// Counts the census tokens in a `spirv-dis` disassembly by whole-token match.
fn census(dis: &str) -> SpvCensus {
    let mut c = SpvCensus {
        op_select: 0,
        op_dot: 0,
        op_ford_less_than: 0,
        op_funord_greater_than_equal: 0,
        op_atomic_iadd: 0,
        op_ugreater_than_equal: 0,
        op_uless_than: 0,
        op_store: 0,
        local_size_x: 0,
        binding_set: Vec::new(),
        op_control_barrier: 0,
        op_type_sampler: 0,
        op_image_sample: 0,
        no_contraction: 0,
    };
    for line in dis.lines() {
        for tok in line.split_whitespace() {
            match tok {
                "OpSelect" => c.op_select += 1,
                "OpDot" => c.op_dot += 1,
                "OpFOrdLessThan" => c.op_ford_less_than += 1,
                "OpFUnordGreaterThanEqual" => c.op_funord_greater_than_equal += 1,
                "OpAtomicIAdd" => c.op_atomic_iadd += 1,
                "OpUGreaterThanEqual" => c.op_ugreater_than_equal += 1,
                "OpULessThan" => c.op_uless_than += 1,
                "OpStore" => c.op_store += 1,
                "OpControlBarrier" => c.op_control_barrier += 1,
                "OpTypeSampler" => c.op_type_sampler += 1,
                // VG R3 P3-4: the ONE prefix selector in this census — see the field's own doc for
                // why the whole-token rule is inverted here and what the fixture control asserts.
                t if t.starts_with("OpImageSample") => c.op_image_sample += 1,
                // `OpDecorate %x NoContraction` — the decoration `precise` emits. A whole token, and
                // the only SPIR-V mnemonic containing it is the decoration itself.
                "NoContraction" => c.no_contraction += 1,
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

/// Gate (a'): the `-D VB_CULL_DEBUG_PROBE=1` DIAGNOSTIC artifact byte-equals its own re-DXC.
///
/// The variant is the instrument `hzb_verdict_oracle_gate.rs`'s boundary survey reads its
/// `depth_near` out of. A STALE artifact there does not fail loudly — it reports numbers from an
/// older leaf, which is a measurement that agrees with nothing and says so about the wrong module.
/// This is the same reason gate (a) exists for the shipping one.
///
/// ⚠️ It does NOT gate that the two artifacts compute the same thing. That claim is executed where
/// it can be: the boundary corpus dispatches BOTH modules over every probe and asserts their
/// partitions agree.
#[test]
fn vb_batch_cull_debug_spv_byte_identical() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "vb_batch_cull_spv_sync: dxc not found — SKIPPING the re-DXC byte-identity check on \
             the VB_CULL_DEBUG_PROBE variant."
        );
        return;
    };
    let dir = shaders_dir();
    let committed_path = dir.join("vb_batch_cull_debug.comp.spv");
    let committed = std::fs::read(&committed_path)
        .unwrap_or_else(|e| panic!("missing committed {}: {e}", committed_path.display()));
    let fresh = redxc_defines(
        &dxc,
        &dir,
        "vb_batch_cull.comp.hlsl",
        "vb_batch_cull_debug.comp.spv",
        &["VB_CULL_DEBUG_PROBE=1"],
    );
    assert!(
        committed == fresh,
        "vb_batch_cull_debug.comp.spv ({} bytes committed, {} bytes fresh) is NOT the re-DXC of \
         vb_batch_cull.comp.hlsl under `-D VB_CULL_DEBUG_PROBE=1` — re-run the SECOND recipe in \
         that shader's header and commit it.",
        committed.len(),
        fresh.len(),
    );
}

/// Gate (b): the module carries BOTH armed decisions (level 1 since rung R2c, level 2 since rung
/// R2d-6), still carries the rung-R2c0 MACHINERY, and names exactly the descriptors it really uses
/// with no workgroup synchronisation.
///
/// This pin was `vb_batch_cull_module_is_inert` at rung R2c0 and asserted the exact opposite
/// (`OpSelect == 0`, `OpDot == 0`, `OpFOrdLessThan == 0`). Arming the cull made that RED, which is
/// what it was for — so R2c RE-PINNED it against its own measured module rather than deleting it,
/// R2d-3 re-pinned it again, and R2d-6 re-pins it against the ARMED module.
///
/// # The two numbers the arming OWES this file, and the ones it merely moves
///
/// **Owed** (stated as requirements, derived from the source, and each with its own named
/// assertion): the binding set must contain @4 and @5 — R2d-3 measured that DXC strips a
/// declared-but-unloaded resource, so their return is the artifact-level proof that the level-2
/// predicate reads the instance rows and the mesh bounds at all — and `OpAtomicIAdd` must still be
/// exactly 1, because arming a predicate has no business touching the compaction.
///
/// **Merely moved** (shipped as [`CENSUS_TBD`], filled from the rebuilt module): every other
/// count. See that constant's doc for why none of them survives the change by inspection, and why
/// predicting one and then confirming it would not be a measurement.
#[test]
fn vb_batch_cull_module_carries_the_armed_decision() {
    let Some(spirv_dis) = find_spirv_dis() else {
        eprintln!(
            "vb_batch_cull_spv_sync: spirv-dis not found — SKIPPING the arming census on this \
             host. NOTHING about what the cull module contains is checked by this run."
        );
        return;
    };
    let dir = shaders_dir();
    let committed_path = dir.join("vb_batch_cull.comp.spv");
    assert!(committed_path.exists(), "missing committed {}", committed_path.display());
    let actual = census(&disassemble(&spirv_dis, &committed_path));

    // ⚠️ THE STRONGEST SINGLE CHECK IN THE RUNG, stated FIRST so it names itself instead of
    // arriving as one differing field inside a whole-struct diff.
    //
    // R2d-3 MEASURED that DXC STRIPS a declared-but-unloaded resource: its module declared all
    // seven descriptors in HLSL and reported the binding set `[0,1,2,3,6]`. So @4/@5 present is
    // exactly the artifact-level signature of "the level-2 predicate really reads the instance
    // rows and the per-mesh bounds", and @4/@5 absent is the signature of an arming that compiled
    // away — which renders a byte-identical image on every all-on-screen pinned scene and would
    // pass every golden.
    assert!(
        actual.binding_set.contains(&4) && actual.binding_set.contains(&5),
        "the ARMED module's binding set is {:?} — it does not name @4 (`gVbInstances`) and/or @5 \
         (`gMeshBounds`). DXC strips a declared-but-unloaded resource (MEASURED at rung R2d-3, \
         whose inert module reported [0,1,2,3,6]), so this means the level-2 `keep` predicate \
         loads neither the instance rows nor the mesh bounds: the arming silently did nothing. No \
         golden can see that state, which is why this assertion exists.",
        actual.binding_set
    );

    // === VG R3 piece 3 step P3-4: the FIVE bindings this step LOADS, each in its OWN assertion. ===
    //
    // ⚠️ A NEW SHAPE, not a copied one. The @4/@5 check above is ONE JOINT assertion, so it can say
    // "one of the two is missing" and not which. Five bindings arriving in one commit need the
    // failure to name itself: a module missing @9 alone (the pyramid never tapped ⇒ a cull that
    // rejects nothing) and a module missing @10 alone (the late record never written ⇒ the late scope
    // draws whatever the host seeded) are completely different defects with the same joint message.
    //
    // Every one of them is a REQUIREMENT derived from the source, not a measurement: DXC strips what
    // is not loaded, so each `contains` is the artifact-level proof that the named load reached the
    // module. And each of these five states, in its message, the failure mode a golden CANNOT see.
    for (binding, name, what_absence_means) in [
        (
            7usize,
            "VbLateVisible",
            "the early phase writes no candidate list and the late phase reads none, so the \
             two-way partition compiled away and every frustum survivor is drawn early — which is \
             byte-identical to a correct run on any scene where nothing is occluded",
        ),
        (
            8,
            "VbCullUni",
            "the occlusion leaf never loaded the view-projection, the pyramid extents or the level \
             count, so it cannot be projecting anything: the verdict degenerates to whatever the \
             uninitialised locals hold",
        ),
        (
            9,
            "gHzbPyramid",
            "NOTHING TAPS THE DEPTH PYRAMID. `occ` stays at its `+INFINITY` seed, `depth_near < occ` \
             is true for every finite instance, and the cull would REJECT EVERYTHING it tests — or, \
             if the leaf folded away entirely, reject nothing. Both are invisible to a golden on an \
             unoccluded scene",
        ),
        (
            10,
            "VbIndirectLate",
            "the late phase never writes `instanceCount`, so the late scope draws exactly what the \
             host upload seeded (zero) forever — and every image gate stays green on a late cull \
             that does not exist",
        ),
        (
            11,
            "VbLateCount",
            "the per-batch deferral count is neither written nor read, so the late phase compacts \
             against an undefined word",
        ),
    ] {
        assert!(
            actual.binding_set.contains(&binding),
            "the P3-4 module's binding set is {:?} — it does not name @{binding} (`{name}`). DXC \
             strips a declared-but-unloaded resource (MEASURED at rung R2d-3), so this means \
             {what_absence_means}.",
            actual.binding_set
        );
    }

    let expected = SpvCensus {
        // ---- MEASURED, and filled from the REBUILT module (see `CENSUS_TBD`) -------------------
        //
        // Every `CENSUS_TBD` below is replaced by the number `spirv-dis` reports for the armed
        // artifact. Once filled they are MEASURED, and the rule the previous rungs stated applies
        // again: do not edit these literals to make a failing run pass — they say what the module
        // DOES, and a change in them is a change in the cull.
        //
        // The `visible ? k : 0u` ternary was ONE `OpSelect` at rung R2d-3 and stayed one through
        // P3-3. ⚠️ P3-4 adds seven ternary SITES — the three corner selectors, `hzb_msb`'s zero
        // guard (×2 axes), `hzb_pyramid_load`'s disarmed address mask, `hzb_conservative_min`'s
        // `b < a ? b : a` at four inlined call sites, the two `occ_armed ? … : 0u` reads and
        // `visible ? n_defer : 0u` — and every one of them may lawfully lower to a select OR to a
        // branch-and-phi. The direction is UP; the value is DXC's.
        op_select: 13,
        // `dot(pl.xyz, c)` and `dot(abs(pl.xyz), h)` were 2 in a single rolled plane loop; the R2d-6
        // module measured 10 with the Arvo fold's six inlined beside them.
        //
        // ⚠️ **THIS FIELD IS RE-MEASURED AND MUST NEVER BE ASSERTED 0.** "The projection contains no
        // `dot()`" is NOT what this counts: the count is MODULE-WIDE, and the eight `dot()` calls it
        // would have to exclude (the plane test's two, `arvo_world_box`'s six) are exactly the ones
        // P3-4 leaves unqualified on purpose. It is expected to RISE, because `arvo_world_box` is
        // now called from BOTH phases and DXC inlines every helper into `%main`. The property this
        // field cannot carry lives on `the_projection_fold_is_written_out_and_carries_no_dot` (the
        // SOURCE) and on `no_contraction` (the artifact) — and each of those states its own limit.
        op_dot: 16,
        // Was 2: the `dist + radius < 0.0` rejection, once per surviving copy of the plane test.
        // P3-4 adds `px1 < px0`, `py1 < py0` and `hzb_conservative_min`'s `b < a` at four inlined
        // sites. ⚠️ The leaf's `cw <= 0.0` and `mn <= mx` are `OpFOrdLessThanEqual` — a DIFFERENT
        // token this census does not count, which is precisely the near-miss
        // `the_inertness_census_uses_whole_token_matching` pins.
        //
        // ⚠️ MEASURED AT 12, DOWN FROM P3-3's 14, and the decrease is the whole reason the field
        // below exists. The step that ADDED a per-corner comparison REMOVED two ordered compares:
        // the verdict left `depth_near < occ` (one per inlined copy) for `!(cz < bound)`, which DXC
        // lowers to `OpFUnordGreaterThanEqual`. Predicted to rise; measured to fall. The number here
        // is read off the built module, never derived from that prediction.
        op_ford_less_than: 12,
        // MEASURED at 4. Two are the occlusion verdict itself, one per inlined copy of the leaf;
        // the loop over the eight corners is ROLLED, so all eight share one compare. See the field's
        // declaration for why counting only the ordered form was a blind spot over the decision.
        op_funord_greater_than_equal: 4,
        // ⚠️ THE COMPACTION CLAIM — NOT a placeholder, and the second number the arming OWES this
        // file. R2c0, R2c and R2d-3 each pinned exactly one `InterlockedAdd`; rung R2d-6 replaces
        // a predicate and has no business touching the compaction, so it must still be one.
        op_atomic_iadd: 1,
        // Was 1: the tail-group range guard `i >= pc.batch_count`. P3-4 adds `level >= uni.levels`,
        // which DXC may also spell as an inverted `OpULessThan`.
        op_ugreater_than_equal: 3,
        // Was 4: the clamp-and-drop bound, the plane loop's bound and the per-instance loop's bound.
        // P3-4 adds the corner loop (`corner < 8u`) and the late compaction loop (`j < n_defer`),
        // and the corner loop may be unrolled by DXC of its own accord even though no `[unroll]` is
        // written — which is exactly why the loop's shape is pinned here rather than assumed.
        //
        // MEASURED at 9. The corner loop came out ROLLED (`OpLoopMerge` present, one compare in the
        // body), so it contributes ONE bound and not eight — the assumption this comment refused to
        // make, decided by the artifact.
        op_uless_than: 9,
        // Was 3: the record store, the counter slot and the region write. P3-4 adds the early
        // candidate store, the early `VbLateCount[i]`, the lane-0 frame-index stamp, the late
        // compaction store and `VbIndirectLate.Store` — and DXC's handling of the 96-byte uniform
        // load plus the two `out` parameters of `arvo_world_box` can move it further.
        //
        // MEASURED at 16, up from 8: the two per-corner arrays `corner_cz`/`corner_cw` are written
        // in the fold's own loop, and their zero seed is what makes an unwritten slot force KEEP.
        op_store: 16,
        // NOT a placeholder and NOT a prediction: this field is READ FROM the host constant the
        // separate assertion below compares `actual` against, so it states the CONTRACT rather
        // than a measurement. `[numthreads]` is untouched by this rung.
        local_size_x: boyko_rhi_vulkan::compute::VB_BATCH_CULL_LOCAL_SIZE_X as usize,
        // ⚠️ THE FIELD THAT ANSWERED ITS OWN QUESTION, and now the field that proves the arming.
        //
        // `vb_cull_layout` bound SEVEN descriptors (@0..@6) when this expectation was written, and
        // binds TWELVE (@0..@11) since VG R3 piece 3 step P3-2. Rung R2d-3's module named FIVE: DXC
        // **stripped @4 (`gVbInstances`) and @5 (`gMeshBounds`)**, which that rung declared in
        // HLSL but never loaded from. The armed module loads from both, and every one of the seven
        // the MODULE declares is either loaded (@1/@4/@5), stored (@0/@2/@6) or atomically updated
        // (@3) — so this is a REQUIREMENT derived from the source, not a measurement to be filled
        // in. It is UNMOVED by P3-2, which widened the host layout and the descriptor set and
        // touched no HLSL: the five new bindings are bound-but-unread, so the module cannot name
        // them. When the shader does load from them (step P3-4) this list grows, and its growing is
        // the artifact-level evidence that the load is real. The separately-named assertion above
        // is what reports a violation.
        binding_set: vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        // Zero, as the construction implies: no `groupshared`, no barrier intrinsic, and the
        // region write is thread-private. Unmoved by the arming — it replaced a predicate, not the
        // compaction — and unmoved by P3-4, whose late compaction is a single-lane argument
        // (`n_keep <= j`) rather than a shared one. The separately-named assertion below states it
        // as a property.
        op_control_barrier: 0,
        // VG R3 P3-4, plan D7: ZERO, and a REQUIREMENT rather than a measurement — the module
        // declares no `SamplerState` and this piece creates no `VkSampler` anywhere.
        op_type_sampler: 0,
        // VG R3 P3-4, plan D7: ZERO. `.Load` emits `OpImageFetch`; any `OpImageSample*` here would
        // mean a FILTERED read of a min-reduced pyramid, which bounds the footprint from neither
        // side and can therefore reject something visible.
        op_image_sample: 0,
        // VG R3 P3-4, plan D11: MEASURED. `precise` is written on eight locals in the leaf (the four
        // projection rows and the four post-divide values), but the DECORATION lands per
        // INSTRUCTION, not per declaration — so the count is DXC's arithmetic over the fold, not
        // eight, and predicting it would be a gate wearing a prediction's clothes.
        //
        // MEASURED at 70, up from 68 by exactly two: the verdict's `precise float bound = occ * cw`,
        // one per inlined copy of the leaf. That the delta is 2 and not 16 is itself the artifact's
        // statement that the corner loop is rolled, agreeing with `op_uless_than`.
        no_contraction: 70,
    };
    assert_eq!(
        actual, expected,
        "vb_batch_cull.comp.spv's census diverged. Expected {expected:?}, got {actual:?}. \
         ({CENSUS_TBD} is the unfilled placeholder: paste the measured counts from `got` into the \
         expectation above.)"
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
        "invariant: the module must still carry a real plane test. Both at zero means the cull \
         reverted to a constant `true` at BOTH levels, which renders identically on today's \
         fully-on-screen scenes and would therefore pass every golden while culling nothing. (A \
         level-2-only revert does NOT show up here — the level-1 test keeps these non-zero on its \
         own — which is why the binding-set assertion at the top of this test, not this one, is \
         what proves the ARMING.)"
    );
    assert_eq!(
        actual.op_atomic_iadd, 1,
        "invariant: the compaction claim must SURVIVE the arming UNTOUCHED — exactly one \
         `InterlockedAdd`, the same number rungs R2c0, R2c and R2d-3 each pinned. Rung R2d-6 \
         replaced a predicate; it had no business adding or removing an atomic."
    );
    // A statement about the SOURCE, not a predicted count: `vb_batch_cull.comp.hlsl` contains no
    // barrier intrinsic and no `groupshared`, and DXC does not synthesise workgroup
    // synchronisation. The region write is thread-private by construction (INVARIANT
    // R2d-REGION-DISJOINT), so a barrier here would mean the compaction became shared state.
    assert_eq!(
        actual.op_control_barrier, 0,
        "invariant: the per-INSTANCE compaction is region-addressed and thread-private — it needs          no workgroup barrier, and one appearing here means someone made it shared state."
    );

    // === VG R3 piece 3 step P3-4 (plan D7): THE PYRAMID IS READ POINT-SAMPLED. ===
    //
    // Two separate claims, so a failure names which one broke. Both are SOUNDNESS claims: a filtered
    // read of a min-reduced pyramid is a convex combination of four bounds, which bounds the
    // footprint from NEITHER side — under reverse-Z it can come out GREATER than every depth in the
    // footprint and reject something VISIBLE. False negatives are missing geometry.
    assert_eq!(
        actual.op_type_sampler, 0,
        "invariant: NO `VkSampler` exists anywhere in this piece, so the module must declare no \
         `OpTypeSampler`. One appearing here means someone gave the pyramid a filter to get wrong."
    );
    assert_eq!(
        actual.op_image_sample, 0,
        "invariant: the pyramid is tapped with `.Load(int3(x, y, level))` — `OpImageFetch`, point, \
         explicit mip. An `OpImageSample*` here is a FILTERED read of a min-reduced pyramid, whose \
         bilinear blend lies strictly between the four texels' min and max and therefore bounds the \
         footprint from neither side. That is the direction that DELETES geometry."
    );
    // A DIRECTIONAL property, not a count: `precise` must have produced SOMETHING. Zero here would
    // mean the projection fold's `NoContraction` never reached the artifact — i.e. the one
    // artifact-level thing standing behind "no reassociation, no FMA contraction in the projection"
    // is absent — while the source-level gate below stays perfectly green.
    assert!(
        actual.no_contraction > 0,
        "invariant: the projection fold is written `precise` on every node, which must emit at \
         least one `OpDecorate ... NoContraction`. Zero means `precise` did not survive DXC, and \
         the source-level fold gate cannot see that: it reads the .hlsl, not the artifact."
    );
}

/// The two sentinel comments the SOURCE-level fold gate reads.
const FOLD_BEGIN: &str = "// === PROJECTION FOLD BEGIN ===";
const FOLD_END: &str = "// === PROJECTION FOLD END ===";

/// Extracts the text strictly BETWEEN the two sentinels, or `Err` naming which one is missing.
///
/// Fails loudly on a missing marker rather than matching empty — that distinction is the whole
/// difference between a gate and a decoration, because "no `dot(` in an empty string" is trivially
/// true and would stay green after someone deleted the fold.
fn extract_projection_fold(src: &str) -> Result<&str, String> {
    let Some(b) = src.find(FOLD_BEGIN) else {
        return Err(format!("`{FOLD_BEGIN}` is absent"));
    };
    let Some(e) = src.find(FOLD_END) else {
        return Err(format!("`{FOLD_END}` is absent"));
    };
    if e <= b {
        return Err(format!("`{FOLD_END}` precedes `{FOLD_BEGIN}`"));
    }
    if src[b + FOLD_BEGIN.len()..].find(FOLD_BEGIN).is_some()
        || src[e + FOLD_END.len()..].find(FOLD_END).is_some()
    {
        return Err("a sentinel occurs more than once; the extracted region is ambiguous".into());
    }
    Ok(&src[b + FOLD_BEGIN.len()..e])
}

/// VG R3 piece 3 step P3-4 (plan D11): **the projection is an explicit, written-out `precise` fold
/// and contains no `dot()`.**
///
/// # Why this is a SOURCE gate and why that is not a cop-out
///
/// The property wanted is "no `OpDot` in the projection". It is **not constructible** at the
/// artifact:
///
/// 1. [`census`] counts `OpDot` MODULE-WIDE, in a flat token loop over the whole disassembly.
/// 2. The calls it would have to exclude — the plane test's two and `arvo_world_box`'s six — are in
///    code this step leaves unqualified ON PURPOSE, so that `no_contraction` measures one change.
/// 3. Scoping a census to a function range is UNREACHABLE on this artifact, not merely unbuilt: a
///    byte scan of the committed `vb_batch_cull.comp.spv` finds exactly ONE `OpFunction` header,
///    because DXC inlines every helper into `%main`. There is no artifact-level boundary to scope to.
///
/// So the property is gated where it IS expressible, and the limit is written down rather than
/// worked around.
///
/// # Why `dot()` is forbidden there at all
///
/// `cluster_cull.hlsl`'s `sq_dist_point_aabb` is this repository's own reasoned rejection, in
/// writing: Vulkan specifies `OpFAdd` / `OpFSub` / `OpFMul` as *"Correctly rounded"* but specifies
/// `OpDot` only as *"inherited from a formula"*, and the same appendix permits that formula to *"be
/// transformed using the mathematical associativity, commutativity, and distributivity of the
/// operators involved"*. `depth_near = max(cz · inv_w)` is downstream of this sum, and a
/// `depth_near` one ULP LOW is the geometry-deleting direction.
///
/// # ⚠️ What this CANNOT claim
///
/// * **Nothing about the compiled artifact.** DXC could in principle pattern-match the written-out
///   sum back into an `OpDot`; this gate would not see it. What stands behind that is the
///   differential's control D5 (swap the fold for `dot()` and report whether the verdict moves),
///   and a null result there is itself the finding.
/// * **Nothing about a `dot(` written OUTSIDE the sentinels.** The leaf's other arithmetic is
///   unguarded by construction.
/// * **The sentinel comments are unpinned text a refactor can move.** That is why the extractor
///   errors on a missing, duplicated or inverted marker instead of matching empty.
///
/// Runs unconditionally — it reads a committed source file, so it cannot SKIP.
#[test]
fn the_projection_fold_is_written_out_and_carries_no_dot() {
    let path = shaders_dir().join("vb_batch_cull.comp.hlsl");
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing committed {}: {e}", path.display()));
    let fold = extract_projection_fold(&src).unwrap_or_else(|why| {
        panic!(
            "the PROJECTION FOLD sentinels in {} are not usable: {why}. They are read by this gate; \
             moving or deleting one silently removes the only check that the occlusion leaf's \
             projection is a written-out fold rather than a `dot()`.",
            path.display()
        )
    });

    assert!(
        !fold.contains("dot("),
        "the projection fold contains a `dot(`:\n{fold}\nVulkan specifies OpDot only as \
         \"inherited from a formula\" and permits that formula to be reassociated, while OpFAdd / \
         OpFMul are \"correctly rounded\". `depth_near` is downstream of this sum and one ULP LOW \
         is the geometry-deleting direction — see `cluster_cull.hlsl`'s own rejection of `dot()` \
         for the governing precedent."
    );

    // The SHAPE, so "no `dot(`" cannot be satisfied by an empty or gutted region. Four `precise`
    // declarations — one per math ROW of `clip = pv · world` — each spelling
    // `r.x*p.x + r.y*p.y + r.z*p.z + r.w`: THREE products and THREE adds per row, mirroring
    // `boyko_render::hzb::project_aabb`'s own left fold (`r[0]*p[0] + r[1]*p[1] + r[2]*p[2] + r[3]`)
    // term for term.
    //
    // ⚠️ The plan's prose says "four products and three adds", counting the `+ r[3]` translation
    // term as a fourth product — which is the expansion of a FOUR-component `dot(row, world4)`, the
    // form D11 REJECTS. The oracle multiplies three terms and adds three times, and it is the oracle
    // this leaf must mirror, so the numbers below are 12 and 12 rather than 16 and 12.
    assert_eq!(
        fold.matches("precise").count(),
        4,
        "the projection fold must carry exactly four `precise` declarations, one per math row of \
         `clip = pv * world`. `precise` is what emits `NoContraction`, and a row without it may be \
         FMA-contracted below the .spv where no gate can see it. Got:\n{fold}"
    );
    assert_eq!(
        fold.matches('*').count(),
        12,
        "the projection fold must spell three products per row over four rows. Got:\n{fold}"
    );
    assert_eq!(
        fold.matches('+').count(),
        12,
        "the projection fold must spell three adds per row over four rows. Got:\n{fold}"
    );
}

/// FIXTURE CONTROL for [`extract_projection_fold`], and it is the reason the gate above is not
/// decorative.
///
/// A sentinel extractor that returns an empty slice on a missing marker turns "contains no `dot(`"
/// into a tautology — the exact shape of vacuity this campaign has shipped and then caught. Every
/// arm below is a corruption the real gate must reject, run on synthetic text so it cannot skip.
#[test]
fn the_projection_fold_extractor_fails_loudly_on_a_broken_sentinel() {
    let good = "prefix\n// === PROJECTION FOLD BEGIN ===\nprecise float cx = a * b + c;\n\
                // === PROJECTION FOLD END ===\nsuffix\n";
    assert_eq!(
        extract_projection_fold(good).expect("invariant: the well-formed fixture extracts"),
        "\nprecise float cx = a * b + c;\n",
        "the extractor must return the text strictly BETWEEN the sentinels"
    );

    // A deleted BEGIN marker. Without this arm the extractor could return `&src[..end]` — the whole
    // file up to END — and "contains no `dot(`" would then be asserted over the plane test too.
    assert!(
        extract_projection_fold("precise float cx = a * b;\n// === PROJECTION FOLD END ===\n")
            .is_err(),
        "a missing BEGIN sentinel must be an ERROR, never a silent match"
    );
    assert!(
        extract_projection_fold("// === PROJECTION FOLD BEGIN ===\nprecise float cx = a;\n")
            .is_err(),
        "a missing END sentinel must be an ERROR, never a silent match"
    );
    // Inverted order would otherwise slice backwards (a panic, or an empty region).
    assert!(
        extract_projection_fold(
            "// === PROJECTION FOLD END ===\nx\n// === PROJECTION FOLD BEGIN ===\n"
        )
        .is_err(),
        "an inverted sentinel pair must be an ERROR"
    );
    // Duplicated markers make the region ambiguous: a second BEGIN after the first would let a
    // `dot(` hide between them while `find` kept matching the outermost pair.
    assert!(
        extract_projection_fold(
            "// === PROJECTION FOLD BEGIN ===\na\n// === PROJECTION FOLD BEGIN ===\nb\n\
             // === PROJECTION FOLD END ===\n"
        )
        .is_err(),
        "a duplicated BEGIN sentinel must be an ERROR — the extracted region would be ambiguous"
    );

    // …and the positive direction: a `dot(` PLANTED between the sentinels must be visible to the
    // gate's own predicate. This is control D-source: the corruption that must go RED.
    let poisoned = "// === PROJECTION FOLD BEGIN ===\nprecise float cx = dot(row, p);\n\
                    // === PROJECTION FOLD END ===\n";
    assert!(
        extract_projection_fold(poisoned)
            .expect("invariant: the poisoned fixture is well-formed")
            .contains("dot("),
        "the gate's own predicate missed a `dot(` planted between the sentinels — it would then be \
         satisfied by blindness"
    );
}

/// FIXTURE CONTROL for [`census`]'s selectors, and it is not decorative.
///
/// # The near-miss that inverts the pin
///
/// `OpSelectionMerge` has `OpSelect` as a strict prefix, and rung R2c0's module contained
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

    // === VG R3 piece 3 step P3-4's three new selectors, every one in BOTH directions. ===

    // `OpTypeSampler` must be seen…
    assert_eq!(
        census("          %type_sampler = OpTypeSampler\n").op_type_sampler,
        1,
        "the selector missed a REAL `OpTypeSampler`; the no-sampler pin would be satisfied by \
         blindness and a filtered pyramid read could ship under a green gate"
    );
    // …and must NOT fire on the two neighbours that always sit beside it in a sampled-image module.
    // `OpTypeSampledImage` is the COMBINED type (which this module must also not have, but which is
    // a different claim), and `OpTypeImage` is what `Texture2D<float>` legitimately declares.
    assert_eq!(
        census(
            "        %type_sampled = OpTypeSampledImage %type_2d_image\n              \
             %type_2d_image = OpTypeImage %float 2D 2 0 0 1 Unknown\n"
        )
        .op_type_sampler,
        0,
        "`OpTypeSampledImage` / `OpTypeImage` were counted as `OpTypeSampler`; the pin would then \
         be RED on the correct module — a gate that fires on the state it exists to certify"
    );

    // ⚠️ THE ONE PREFIX SELECTOR. It must catch EVERY `OpImageSample…` mnemonic…
    for mnemonic in [
        "OpImageSampleImplicitLod",
        "OpImageSampleExplicitLod",
        "OpImageSampleDrefImplicitLod",
        "OpImageSampleProjExplicitLod",
        "OpImageSparseSampleExplicitLod",
    ] {
        // `OpImageSparseSample*` does NOT begin with `OpImageSample`, so it is deliberately outside
        // the prefix — and it is listed here to state that, not to claim coverage of it.
        let expected = usize::from(mnemonic.starts_with("OpImageSample"));
        assert_eq!(
            census(&format!("         %9 = {mnemonic} %v4float %8 %7\n")).op_image_sample,
            expected,
            "the `OpImageSample` prefix selector disagreed with its own rule on `{mnemonic}`"
        );
    }
    // …and must NOT fire on `OpImageFetch`, which is what `.Load` emits and what MUST stay legal.
    // Counting it would make the pin RED on the correct module.
    assert_eq!(
        census("         %9 = OpImageFetch %v4float %8 %7 Lod %uint_0\n").op_image_sample,
        0,
        "`OpImageFetch` was counted as a sample; the point-sampled pyramid read would then be \
         indistinguishable from the filtered read the pin exists to forbid"
    );
    assert_eq!(
        census("         %9 = OpImageQuerySize %v2uint %8\n").op_image_sample,
        0,
        "an unrelated `OpImage*` opcode was counted as a sample"
    );

    // `NoContraction` is a DECORATION, so the selector reads the token wherever it appears on an
    // `OpDecorate` line…
    assert_eq!(
        census("               OpDecorate %42 NoContraction\n").no_contraction,
        1,
        "the selector missed a REAL `NoContraction` decoration; the `precise` evidence would be \
         satisfied by blindness"
    );
    // …and must not be confused with the `NoPerspective` / `NonWritable` decorations that share its
    // prefix letters, nor counted twice off one line.
    assert_eq!(
        census(
            "               OpDecorate %10 NoPerspective\n               OpDecorate %11 NonWritable\n"
        )
        .no_contraction,
        0,
        "a neighbouring decoration false-matched `NoContraction`; the count would then move for \
         reasons unrelated to `precise`"
    );
}
