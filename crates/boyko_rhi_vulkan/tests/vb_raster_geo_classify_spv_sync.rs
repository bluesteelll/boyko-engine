//! Re-DXC byte-identity gate for the **shipping VB `.spv` that no other byte gate reaches**:
//! the `vb_raster` pair, the `vb_geo` family, and the two atomic `vb_classify` passes.
//!
//! The row table is the authority on the membership, and this doc deliberately does NOT restate its
//! CARDINALITY. A count written into prose (or worse, into a test's NAME) is the `280/560` shape:
//! it goes stale on the first legitimate addition, and the repair everyone reaches for is to edit
//! the number rather than to read what changed. `VB_RASTER_GEO_CLASSIFY_ROWS.len()` is printed by
//! the gate itself on every run.
//!
//! # What was ungated, and why indirect cover is not the same thing
//!
//! Eighteen artifacts named `vb_*.spv` were committed under `shaders/` **when this file was
//! written**; ten of them had a re-DXC oracle — `vb_lit_producer_spv_sync.rs`'s
//! `VB_LIT_PRODUCER_ROWS` (the `vb_resolve` / `vb_shade` / `vb_shade_split` families;
//! `vb_froxel_spv_sync.rs` re-covers six of those same ten and adds none). Of the remaining eight,
//! two are outside this file's scope by construction (see below) and the rest shipped with no
//! byte-wise coverage at all.
//!
//! **That census is a snapshot of its own rung and has not been re-taken since** — the directory
//! now holds **21**, the three additions being `vb_batch_cull.comp.spv`,
//! `vb_batch_cull_debug.comp.spv` (both gated by `vb_batch_cull_spv_sync.rs`) and DP6b's
//! `vb_geo_sv0.comp.spv` (gated by row 5 below). It is left
//! standing rather than silently updated because re-deriving it is a real audit, not a number edit;
//! what the numbers still support is the argument below (indirect cover is weaker than re-DXC), and
//! that argument does not depend on the total. The membership this file actually gates is
//! [`VB_RASTER_GEO_CLASSIFY_ROWS`], never this paragraph.
//!
//! The rows:
//!
//! * `vb_raster.vs.spv`, `vb_raster.fs.spv` — the VB producer: the raster pair that WRITES the
//!   `vb_id` `R32G32_UINT` attachment every consumer below unpacks.
//! * `vb_geo.comp.spv`, `vb_geo_mv.comp.spv` — R9's thin-aux geometry pass and its
//!   `-D MOTION=1` motion-vector variant.
//! * `vb_geo_sv0.comp.spv` — VB-SV0 DP6b's `-D VB_SV0_TERM=1` variant, added with the axis itself
//!   rather than after the fact. Selected by nothing at DP6b, so it has no indirect golden cover
//!   either: this table is its whole coverage.
//! * `vb_classify_count.comp.spv`, `vb_classify_scatter.comp.spv` — the two atomic passes of the
//!   VB-P2 classify chain.
//!
//! MEASURED, not assumed: each of these is referenced from `src/compute.rs` (its `embed_spirv!`
//! site) and from nothing under any `tests/` directory in the workspace. Their only cover is
//! INDIRECT — some golden pin renders through them, so a drifted blob eventually shows up as an
//! image diff. That is a strictly weaker property for two reasons: it only fires on a host that
//! actually renders the pins (the GPU path is not exercised by CI), and an image diff answers
//! "does this blob still draw the same picture", never "are the COMMITTED BYTES the compile of the
//! COMMITTED SOURCE". Only the re-DXC layer answers the second, and it is the second that catches
//! an HLSL edit whose `.spv` was never re-emitted — the failure mode where source and artifact
//! disagree and the artifact wins silently, because the artifact is what ships.
//!
//! # The two `vb_*.spv` deliberately NOT here
//!
//! * `vb_classify_scan.comp.spv` — `vb_classify_scan.comp.hlsl`'s only `#include` is
//!   `vb_classify_common.hlsli`; it touches neither `vb_pack.hlsli` nor `vb_geom_fetch.hlsli`, so
//!   it is not on the `vb_id` encode's blast radius (it scans counts, never a pixel id).
//! * `vb_shadow_vis.comp.spv` — includes neither header either, and its single `gVbId` mention is
//!   its header doc stating it does NOT read one.
//!
//! Both are still un-re-DXC'd, and that remains true after this file. They are excluded because
//! this gate was scoped to the `vb_id`-perturbed set, not because anything covers them. Naming
//! that here so the gap is a recorded scope boundary rather than an oversight a later reader has
//! to rediscover.
//!
//! # (a) Why a NEW file rather than rows appended to `vb_lit_producer_spv_sync.rs`
//!
//! That file's name, module doc and row table all say one thing: the ten shipping VB **lit
//! producers**. None of the rows here is a lit producer — `vb_raster` writes ids and no lighting,
//! `vb_geo` writes an oct-encoded geometric normal and explicitly does no lighting, `vb_classify`
//! writes bin counts and pixel lists. Appending them would make that file's name false, and that
//! file is the one place in this repo whose module doc had to be written to stop a future sweep
//! from deleting it (it was renamed off a dead stage's name for exactly that reason). A gate whose
//! name misdescribes its contents is the same defect in a new coat.
//!
//! There is also a mechanical reason: `vb_lit_producer_spv_sync.rs`'s `redxc_with_defines`
//! hardcodes `-T cs_6_0` in its argument array. The raster pair is `vs_6_0` / `ps_6_0` — see (c).
//! Adding them would mean changing the signature of the helper that carries the ONLY byte-wise
//! coverage of the four `vb_shade_split` rows.
//!
//! # (c) Why the row table carries a PROFILE column
//!
//! The `-T` profile is read from each shader's own frozen header recipe and they are NOT uniform:
//! `vb_raster.vs.hlsl` pins `-T vs_6_0`, `vb_raster.fs.hlsl` pins `-T ps_6_0`, and every compute
//! row pins `-T cs_6_0`. So the profile is per-row data, not a constant folded into the helper.
//! (`vb_classify_scatter.comp.hlsl`'s header delegates — "identical flags, this file's own name
//! substituted" — to `vb_classify_count.comp.hlsl`'s `-T cs_6_0` recipe; `vb_geo_mv` is
//! `vb_geo.comp.hlsl`'s header's own second line, `-T cs_6_0 -D MOTION=1`, and `vb_geo_sv0` its
//! third, `-T cs_6_0 -D VB_SV0_TERM=1` — the SV0 march needs no `rayQuery`, so unlike the
//! `deferred_pbr` HWRT rows this axis does not move the profile either.)
//!
//! # (b) SKIP policy
//!
//! Both tests early-return with an `eprintln!` when no `dxc` resolves, the `find_dxc()` shape
//! `cluster_cull_spv_sync.rs` / `vb_froxel_spv_sync.rs` / `vb_lit_producer_spv_sync.rs` already
//! use. The reason is unchanged: the committed artifacts are the output of the pinned VulkanSDK
//! 1.4.350.0 toolchain, so a DIFFERENT `dxc` failing this test would mean "wrong toolchain", not
//! "drifted shader" — and a host with no `dxc` at all can prove neither. A skip here is an absence
//! of evidence and must never be read as a green.
//!
//! Two gates, the same pair the precedent files carry, plus one census added at rung R2d-4:
//!
//! 1. **Reproduction** — each row, re-DXC'd under its own frozen recipe, is
//!    byte-identical to its committed `.spv`.
//! 2. **Sensitivity** — the control that makes (1)'s green mean something. A scratch copy of
//!    `vb_raster.fs.hlsl` with its two `SV_Target0` lanes swapped must re-DXC to DIFFERENT bytes.
//! 3. **`SV_InstanceID`'s LOWERING** (VG rung R2d-4) — a `spirv-dis` census pinning which SPIR-V
//!    builtins `vb_raster.vs.spv` decorates. Byte identity already covers "these are the bytes of
//!    this source"; it does NOT tell a reader WHICH builtin the instance id comes from, and from
//!    rung R2d-4 that lowering is load-bearing rather than incidental — see
//!    [`vb_raster_vs_builtin_census_pins_the_sv_instance_id_lowering`].

use std::path::PathBuf;
use std::process::Command;

/// The shaders directory (`CARGO_MANIFEST_DIR/shaders`), where the committed `.hlsl` and `.spv`
/// live (and where DXC must run so any `#include` resolves). Mirrors `cluster_cull_spv_sync.rs`.
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Locates the `dxc` executable: first the pinned Vulkan-SDK path (the repo's offline recipe),
/// then `$VULKAN_SDK/Bin`, then `PATH`. Returns `None` if none resolve (the byte-identity tests
/// then SKIP) — the `cluster_cull_spv_sync.rs` idiom verbatim.
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
/// Returns `None` if none resolve (the census below then SKIPS) — the `cluster_cull_hier_dis_gate.rs`
/// idiom verbatim.
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

/// Disassembles a committed `.spv` — the artifact the engine actually loads, never a re-compile.
/// Panics on a non-zero exit: a malformed committed `.spv` is a build-integrity bug, not a skip.
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

/// Every SPIR-V builtin the module decorates, sorted and deduped — the census's whole selector.
///
/// Both decoration forms are collected, and that is not defensive padding: DXC emits the
/// `SV_Position` output as an `OpMemberDecorate <gl_PerVertex-style struct> 0 BuiltIn Position` on
/// some lowerings and a plain `OpDecorate %var BuiltIn Position` on others, so a selector that read
/// only `OpDecorate` could report a SMALLER set on a perfectly correct module and would then have
/// to be "fixed" by editing the pin. The opcode guard is what keeps an `OpName`/`OpMemberName`
/// token from contributing a phantom entry — see
/// [`the_builtin_selector_reads_both_decoration_forms_and_ignores_names`].
fn builtin_decorations(dis: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in dis.lines() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        let Some(opcode) = toks.first() else { continue };
        if *opcode != "OpDecorate" && *opcode != "OpMemberDecorate" {
            continue;
        }
        if let Some(i) = toks.iter().position(|t| *t == "BuiltIn")
            && let Some(name) = toks.get(i + 1)
        {
            out.push(String::from(*name));
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Re-DXCs `hlsl_name` (relative to the shaders dir) under the EXACT frozen recipe from that
/// shader's own header (`-spirv -T <profile> -E main -fspv-target-env=vulkan1.3`, no `-O`) plus
/// the given `-D` defines, into a fresh temp `.spv` named by `out_tag` (distinct per variant so
/// parallel test binaries never collide), and returns the bytes. Never overwrites a committed
/// artifact.
///
/// Differs from `vb_lit_producer_spv_sync.rs`'s namesake in exactly one way: `profile` is a
/// parameter rather than a hardcoded `cs_6_0`, because this file's rows span three profiles — see
/// the module doc, section (c).
fn redxc_row(dxc: &PathBuf, dir: &PathBuf, hlsl_name: &str, profile: &str, defines: &[&str], out_tag: &str) -> Vec<u8> {
    let out_spv = std::env::temp_dir().join(format!("{out_tag}.redxc.spv"));
    let mut cmd = Command::new(dxc);
    cmd.current_dir(dir).args(["-spirv", "-T", profile, "-E", "main"]);
    for d in defines {
        cmd.args(["-D", d]);
    }
    cmd.args(["-fspv-target-env=vulkan1.3", hlsl_name, "-Fo"]).arg(&out_spv);
    let status = cmd.status().expect("invariant: dxc was located and must run");
    assert!(status.success(), "dxc failed re-compiling {hlsl_name} -T {profile} {defines:?} under the frozen recipe");
    let bytes = std::fs::read(&out_spv).expect("invariant: dxc wrote the re-DXC .spv");
    let _ = std::fs::remove_file(&out_spv); // best-effort tidy
    bytes
}

/// One committed artifact must byte-equal its own re-DXC. Mirrors
/// `vb_lit_producer_spv_sync.rs`'s `assert_spv_byte_identical`, threaded with the row's profile.
fn assert_spv_byte_identical(
    dxc: &PathBuf,
    dir: &PathBuf,
    hlsl_name: &str,
    profile: &str,
    defines: &[&str],
    spv_name: &str,
) {
    let committed_path = dir.join(spv_name);
    let committed = std::fs::read(&committed_path)
        .unwrap_or_else(|e| panic!("missing committed {}: {e}", committed_path.display()));
    let fresh = redxc_row(dxc, dir, hlsl_name, profile, defines, spv_name);
    assert!(
        committed == fresh,
        "{spv_name} ({} bytes committed, fnv1a_64={:#018x}; {} bytes fresh, fnv1a_64={:#018x}) is \
         NOT the re-DXC of {hlsl_name} -T {profile} {defines:?} under the frozen recipe pinned in \
         that shader's own header — either the committed .spv is stale (re-run the header recipe \
         and commit the result) or the host dxc is not the pinned VulkanSDK 1.4.350.0 toolchain. \
         RED here is a real build-integrity defect, never expected drift.",
        committed.len(),
        fnv1a_64(&committed),
        fresh.len(),
        fnv1a_64(&fresh),
    );
}

/// A compact 64-bit FNV-1a fingerprint, used ONLY to make failures and the sensitivity report
/// human-readable. Not a build-integrity primitive — every gate below compares the full byte
/// vectors, never this hash.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The previously-ungated VB artifacts, each `(source, -T profile, defines, committed .spv)`.
///
/// Every profile and define here was read out of the named shader's OWN header recipe, not
/// inferred from a sibling: `vb_raster.vs.hlsl` -> `vs_6_0`, `vb_raster.fs.hlsl` -> `ps_6_0`,
/// `vb_geo.comp.hlsl` -> `cs_6_0` (and its second header line, `-T cs_6_0 -D MOTION=1`, for
/// `vb_geo_mv`; its third, `-T cs_6_0 -D VB_SV0_TERM=1`, for `vb_geo_sv0`),
/// `vb_classify_count.comp.hlsl` -> `cs_6_0` (whose flags `vb_classify_scatter.comp.hlsl`'s header
/// delegates to by name).
///
/// **Row 7 is VB-SV0 DP6b's `vb_geo_sv0`** — the `-D VB_SV0_TERM=1` variant that compiles the
/// SDF-on-mesh shadow + contact-AO march into `vb_geo` (design Decision 1). At DP6b it is selected
/// by NOTHING, so no golden renders through it and this row is its ONLY coverage of any kind — the
/// indirect-cover argument in the module doc does not even apply to it yet. Its additivity (that
/// rows 3 and 4 stay byte-frozen under the new axis) is a separate, stronger statement measured by
/// `vb_geo_preprocess_sync.rs`.
///
/// This table is the ONLY byte-wise coverage in the repo for all seven rows — do not thin it.
const VB_RASTER_GEO_CLASSIFY_ROWS: [(&str, &str, &[&str], &str); 7] = [
    ("vb_raster.vs.hlsl", "vs_6_0", &[], "vb_raster.vs.spv"),
    ("vb_raster.fs.hlsl", "ps_6_0", &[], "vb_raster.fs.spv"),
    ("vb_geo.comp.hlsl", "cs_6_0", &[], "vb_geo.comp.spv"),
    ("vb_geo.comp.hlsl", "cs_6_0", &["MOTION=1"], "vb_geo_mv.comp.spv"),
    ("vb_geo.comp.hlsl", "cs_6_0", &["VB_SV0_TERM=1"], "vb_geo_sv0.comp.spv"),
    ("vb_classify_count.comp.hlsl", "cs_6_0", &[], "vb_classify_count.comp.spv"),
    ("vb_classify_scatter.comp.hlsl", "cs_6_0", &[], "vb_classify_scatter.comp.spv"),
];

/// Gate (1) — reproduction: every one of the seven rows, re-DXC'd under its own frozen recipe,
/// byte-equals its committed artifact. RED for any row names that artifact and means the frozen
/// recipe no longer reproduces it on this host.
#[test]
fn vb_raster_geo_classify_rows_reproduce_under_frozen_recipe() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "vb_raster_geo_classify_spv_sync: dxc not found (no C:/VulkanSDK/.../dxc.exe, no \
             $VULKAN_SDK/Bin, not on PATH) — SKIPPING the reproduction check on this host. A skip \
             is NOT a pass: nothing was proven about these artifacts."
        );
        return;
    };
    let dir = shaders_dir();
    for (hlsl_name, profile, defines, spv_name) in VB_RASTER_GEO_CLASSIFY_ROWS {
        assert_spv_byte_identical(&dxc, &dir, hlsl_name, profile, defines, spv_name);
    }
    eprintln!(
        "vb_raster_geo_classify_spv_sync: all {} rows reproduced byte-identically.",
        VB_RASTER_GEO_CLASSIFY_ROWS.len()
    );
}

/// Gate (2) — the harness sensitivity control, which is what makes gate (1)'s green mean anything.
/// A gate that cannot detect a change is vacuously green, so the reproduction gate is only worth
/// its RED if a byte comparison has teeth on these modules.
///
/// The mutation is applied to `vb_raster.fs.hlsl` — deliberately the SMALLEST module in the table
/// (472 committed bytes, a single `return`). If a re-DXC byte comparison were going to be blind
/// anywhere in this set it would be here, on the module with the least code for a difference to
/// show up in; a control run on one of the 15 KB compute rows would prove much less. The swap
/// (`uint2(instance_id, prim_id)` -> `uint2(prim_id, instance_id)`) is also semantically real: it
/// is exactly the `vb_id` lane transposition every consumer's unpack would then misread.
///
/// Compiled from a scratch copy via `-I <shaders_dir>`, so the committed source and the committed
/// `.spv` are never touched.
///
/// RED here means a re-DXC byte comparison is BLIND for this family and gate (1) above is
/// vacuously green — a finding to report, not a mutation to retune until it passes.
#[test]
fn vb_raster_fs_redxc_is_sensitive_to_a_swapped_vb_id_lane() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "vb_raster_geo_classify_spv_sync: dxc not found (no C:/VulkanSDK/.../dxc.exe, no \
             $VULKAN_SDK/Bin, not on PATH) — SKIPPING the sensitivity control on this host. A skip \
             is NOT a pass: the reproduction gate's teeth were not demonstrated."
        );
        return;
    };
    let dir = shaders_dir();
    let source = std::fs::read_to_string(dir.join("vb_raster.fs.hlsl"))
        .expect("invariant: vb_raster.fs.hlsl is the committed shader source");
    let needle = "uint2(input.instance_id, raw_prim_id)";
    assert!(
        source.contains(needle),
        "invariant: {needle:?} must appear verbatim in vb_raster.fs.hlsl (the SV_Target0 vb_id \
         write) for this mutation to be meaningful — if the expression changed, update this test"
    );
    let mutated = source.replacen(needle, "uint2(raw_prim_id, input.instance_id)", 1);

    let scratch_path = std::env::temp_dir().join("vb_raster_fs_swapped_lane_mutant.hlsl");
    std::fs::write(&scratch_path, &mutated).expect("invariant: temp dir is writable");
    let out_spv = std::env::temp_dir().join("vb_raster_fs_swapped_lane_mutant.spv");
    let status = Command::new(&dxc)
        .args(["-spirv", "-T", "ps_6_0", "-E", "main", "-fspv-target-env=vulkan1.3", "-I"])
        .arg(&dir)
        .arg(&scratch_path)
        .arg("-Fo")
        .arg(&out_spv)
        .status()
        .expect("invariant: dxc was located and must run");
    assert!(status.success(), "dxc failed compiling the mutated vb_raster.fs.hlsl scratch copy");
    let mutated_bytes = std::fs::read(&out_spv).expect("invariant: dxc wrote the mutant .spv");
    let _ = std::fs::remove_file(&scratch_path);
    let _ = std::fs::remove_file(&out_spv);

    let committed = std::fs::read(dir.join("vb_raster.fs.spv"))
        .expect("invariant: vb_raster.fs.spv is the committed artifact");

    eprintln!(
        "vb_raster_geo_classify_spv_sync sensitivity control: vb_raster.fs.hlsl SV_Target0 lanes \
         swapped; committed vb_raster.fs.spv ({} bytes, fnv1a_64={:#018x}) vs mutant re-DXC ({} \
         bytes, fnv1a_64={:#018x})",
        committed.len(),
        fnv1a_64(&committed),
        mutated_bytes.len(),
        fnv1a_64(&mutated_bytes),
    );

    assert!(
        committed != mutated_bytes,
        "RED: swapping vb_raster.fs.hlsl's two SV_Target0 lanes re-DXC'd to a BYTE-IDENTICAL .spv. \
         A re-DXC byte comparison is therefore BLIND for this module, which makes the \
         reproduction gate above vacuously green — it would not catch a real edit either. This is \
         a real finding — do not tune the mutation to force a green."
    );
}

/// ⚠️ **PLACEHOLDER — NOT A MEASUREMENT.** The sorted, deduped set of SPIR-V builtins
/// `vb_raster.vs.spv` decorates, to be replaced with the value read off the REBUILT module.
///
/// It ships as an obviously-wrong sentinel on purpose. Predicting a census value and then
/// "confirming" it is a gate wearing a prediction's clothes — the rule
/// `tests/vb_batch_cull_spv_sync.rs` states for its own census, and the reason its binding-set
/// field was able to SETTLE an open question (whether DXC strips declared-but-unloaded resources)
/// rather than merely agree with a guess. Whether DXC lowers `SV_InstanceID` to a `firstInstance`-
/// relative or an absolute builtin is exactly such a question, so it is measured, not reasoned.
///
/// Once filled: do NOT edit these strings to make a failing run pass. They say what the module
/// DOES, and a change in them is a change in the lowering.
/// MEASURED off the rebuilt module, and the answer CONFIRMS the hazard rather than dismissing it:
/// DXC lowers `SV_InstanceID` to **`InstanceIndex`**, the builtin that INCLUDES `firstInstance` —
/// not to an absolute per-draw counter.
///
/// That is inert today only because `drawIndirectFirstInstance` is `VK_FALSE` on this device and
/// every record this engine emits carries `first_instance == 0`. The moment either changes, the
/// index the VS forms shifts — and since rung R2d-4 that index addresses `visible_instances`,
/// so the consequence is an OUT-OF-RANGE SSBO read with `robustBufferAccess` off, not merely a
/// wrong transform. This pin is what makes that a RED test rather than a silent corruption.
const VB_RASTER_VS_BUILTINS_TBD: &[&str] = &["InstanceIndex", "Position"];

/// Gate (3) — VG rung R2d-4: WHICH SPIR-V builtins the committed `vb_raster.vs.spv` decorates,
/// read off the artifact rather than assumed from the HLSL. The instance-id builtin in that set is
/// the one this gate exists for.
///
/// # Why this is not redundant with the byte gate above
///
/// Gate (1) proves the committed bytes are the compile of the committed source. It says nothing
/// about WHAT that compile chose, and the choice here acquired a safety consequence at rung R2d-4.
///
/// `vb_raster.vs.hlsl`'s instanced arm now uses `pc.base_instance + SV_InstanceID` as an INDEX INTO
/// `gVbVisibleInstance` (Set-0 @11), whose written region for a batch is exactly
/// `[base_instance, base_instance + instance_count)`. DXC has two lowerings for `SV_InstanceID`:
/// the raw builtin (into which the draw's `firstInstance` is folded) and the `BaseInstance`-
/// subtracting form it emits under `-fvk-support-nonzero-base-instance`, which this shader's frozen
/// recipe does not pass. Today the two are indistinguishable, because the recorder writes
/// `first_instance = 0` into every indirect record and `drawIndirectFirstInstance` is not enabled
/// on this device.
///
/// A LATER RUNG that enables that feature and writes a nonzero `firstInstance` turns the difference
/// into an OUT-OF-RANGE read of a storage buffer — undefined (`robustBufferAccess` is off on this
/// device), invisible to the validation layers (they do not follow buffer contents) and invisible
/// to every golden (with the list holding the identity, a stale or out-of-range read still looks
/// plausible). Such a rung must go RED **here**, where the lowering it depends on is stated, rather
/// than ship and be diagnosed from a corrupted frame.
///
/// SKIPS by name when `spirv-dis` is absent — a skip is an absence of evidence, never a pass.
#[test]
fn vb_raster_vs_builtin_census_pins_the_sv_instance_id_lowering() {
    let Some(spirv_dis) = find_spirv_dis() else {
        eprintln!(
            "vb_raster_geo_classify_spv_sync: spirv-dis not found (no \
             C:/VulkanSDK/.../spirv-dis.exe, no $VULKAN_SDK/Bin, not on PATH) — SKIPPING the \
             SV_InstanceID lowering census on this host. A skip is NOT a pass: nothing was proven \
             about which builtin carries the instance id."
        );
        return;
    };
    let committed_path = shaders_dir().join("vb_raster.vs.spv");
    assert!(committed_path.exists(), "missing committed {}", committed_path.display());
    let actual = builtin_decorations(&disassemble_committed(&spirv_dis, &committed_path));

    assert_eq!(
        actual, VB_RASTER_VS_BUILTINS_TBD,
        "vb_raster.vs.spv's BuiltIn set is {actual:?}. If the expected side still reads \
         `<MEASURE-ME...>`, this is the rung-R2d-4 PLACEHOLDER awaiting the measured value — paste \
         the actual set into `VB_RASTER_VS_BUILTINS_TBD`. If it does NOT, the lowering of \
         `SV_InstanceID` (or of `SV_Position`) CHANGED: read that constant's doc before touching \
         it, because `firstInstance`-relative vs absolute decides whether this VS's \
         `gVbVisibleInstance` index can run out of range."
    );
}

/// FIXTURE CONTROL for [`builtin_decorations`]'s selector, and it is not decorative: a selector
/// that silently returned the EMPTY set fails the census above LOUDLY today (the placeholder
/// differs from `[]`), but would be vacuously green forever if `[]` were ever the pinned value.
/// These fixtures pin the two decoration forms that carry builtins and the near-miss that must not
/// contribute.
///
/// Runs unconditionally — pure string handling, no toolchain, so it cannot SKIP.
#[test]
fn the_builtin_selector_reads_both_decoration_forms_and_ignores_names() {
    // ⚠️ The fixture builtins are DELIBERATELY SYNTHETIC (`Aaa`/`Bbb`, names SPIR-V does not
    // define). Feeding this control the real spellings would put a plausible answer one line
    // above the constant that must be MEASURED off the module — an invitation to paste rather
    // than to run `spirv-dis`. The control's job is to prove the PARSER reads both decoration
    // forms; it must not double as a hint about the result.
    assert_eq!(
        builtin_decorations("               OpDecorate %some_var BuiltIn Aaa\n"),
        vec!["Aaa".to_string()],
        "the plain OpDecorate form must be read"
    );
    assert_eq!(
        builtin_decorations("               OpMemberDecorate %some_struct 0 BuiltIn Bbb\n"),
        vec!["Bbb".to_string()],
        "the MEMBER decoration form must be read too — DXC emits SV_Position through it on some \
         lowerings, and a selector blind to it would report a smaller set on a correct module and \
         then have to be 'fixed' by editing the pin"
    );
    assert!(
        builtin_decorations("               OpName %in_var_BuiltIn \"in.var.BuiltIn\"\n").is_empty(),
        "a name is not a decoration — only OpDecorate/OpMemberDecorate may contribute"
    );
    assert_eq!(
        builtin_decorations(
            "               OpDecorate %b BuiltIn Bbb\n               OpDecorate %a BuiltIn \
             Aaa\n               OpDecorate %c BuiltIn Bbb\n"
        ),
        vec!["Aaa".to_string(), "Bbb".to_string()],
        "the pin must be on the SET — sorted and deduped, so a re-ordered or repeated emission \
         cannot flip it"
    );
}
