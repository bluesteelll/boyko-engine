//! Re-DXC byte-identity gate for the **six shipping VB `.spv` that no byte gate reached**:
//! the `vb_raster` pair, the `vb_geo` pair, and the two atomic `vb_classify` passes.
//!
//! # What was ungated, and why indirect cover is not the same thing
//!
//! Eighteen artifacts named `vb_*.spv` are committed under `shaders/`. Before this file, exactly
//! ten of them had a re-DXC oracle — `vb_lit_producer_spv_sync.rs`'s `VB_LIT_PRODUCER_ROWS` (the
//! `vb_resolve` / `vb_shade` / `vb_shade_split` families; `vb_froxel_spv_sync.rs` re-covers six of
//! those same ten and adds none). Of the remaining eight, two are outside this file's scope by
//! construction (see below) and SIX shipped with no byte-wise coverage at all:
//!
//! * `vb_raster.vs.spv`, `vb_raster.fs.spv` — the VB producer: the raster pair that WRITES the
//!   `vb_id` `R32G32_UINT` attachment every consumer below unpacks.
//! * `vb_geo.comp.spv`, `vb_geo_mv.comp.spv` — R9's thin-aux geometry pass and its
//!   `-D MOTION=1` motion-vector variant.
//! * `vb_classify_count.comp.spv`, `vb_classify_scatter.comp.spv` — the two atomic passes of the
//!   VB-P2 classify chain.
//!
//! MEASURED, not assumed: each of the six is referenced from `src/compute.rs` (its `embed_spirv!`
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
//! producers**. None of the six here is a lit producer — `vb_raster` writes ids and no lighting,
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
//! `vb_raster.vs.hlsl` pins `-T vs_6_0`, `vb_raster.fs.hlsl` pins `-T ps_6_0`, and the four
//! compute rows pin `-T cs_6_0`. So the profile is per-row data, not a constant folded into the
//! helper. (`vb_classify_scatter.comp.hlsl`'s header delegates — "identical flags, this file's own
//! name substituted" — to `vb_classify_count.comp.hlsl`'s `-T cs_6_0` recipe; `vb_geo_mv` is
//! `vb_geo.comp.hlsl`'s header's own second line, `-T cs_6_0 -D MOTION=1`.)
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
//! Two gates, the same pair the precedent files carry:
//!
//! 1. **Reproduction** — each of the six rows, re-DXC'd under its own frozen recipe, is
//!    byte-identical to its committed `.spv`.
//! 2. **Sensitivity** — the control that makes (1)'s green mean something. A scratch copy of
//!    `vb_raster.fs.hlsl` with its two `SV_Target0` lanes swapped must re-DXC to DIFFERENT bytes.

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

/// The six previously-ungated VB artifacts, each `(source, -T profile, defines, committed .spv)`.
///
/// Every profile and define here was read out of the named shader's OWN header recipe, not
/// inferred from a sibling: `vb_raster.vs.hlsl` -> `vs_6_0`, `vb_raster.fs.hlsl` -> `ps_6_0`,
/// `vb_geo.comp.hlsl` -> `cs_6_0` (and its second header line, `-T cs_6_0 -D MOTION=1`, for
/// `vb_geo_mv`), `vb_classify_count.comp.hlsl` -> `cs_6_0` (whose flags
/// `vb_classify_scatter.comp.hlsl`'s header delegates to by name).
///
/// This table is the ONLY byte-wise coverage in the repo for all six rows — do not thin it.
const VB_RASTER_GEO_CLASSIFY_ROWS: [(&str, &str, &[&str], &str); 6] = [
    ("vb_raster.vs.hlsl", "vs_6_0", &[], "vb_raster.vs.spv"),
    ("vb_raster.fs.hlsl", "ps_6_0", &[], "vb_raster.fs.spv"),
    ("vb_geo.comp.hlsl", "cs_6_0", &[], "vb_geo.comp.spv"),
    ("vb_geo.comp.hlsl", "cs_6_0", &["MOTION=1"], "vb_geo_mv.comp.spv"),
    ("vb_classify_count.comp.hlsl", "cs_6_0", &[], "vb_classify_count.comp.spv"),
    ("vb_classify_scatter.comp.hlsl", "cs_6_0", &[], "vb_classify_scatter.comp.spv"),
];

/// Gate (1) — reproduction: every one of the six rows, re-DXC'd under its own frozen recipe,
/// byte-equals its committed artifact. RED for any row names that artifact and means the frozen
/// recipe no longer reproduces it on this host.
#[test]
fn vb_raster_geo_classify_six_rows_reproduce_under_frozen_recipe() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "vb_raster_geo_classify_spv_sync: dxc not found (no C:/VulkanSDK/.../dxc.exe, no \
             $VULKAN_SDK/Bin, not on PATH) — SKIPPING the reproduction check on this host. A skip \
             is NOT a pass: nothing was proven about these six artifacts."
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
         A re-DXC byte comparison is therefore BLIND for this module, which makes the six-row \
         reproduction gate above vacuously green — it would not catch a real edit either. This is \
         a real finding — do not tune the mutation to force a green."
    );
}
