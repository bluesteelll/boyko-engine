//! VB-SV0 rung S2 ("dark infra") — the static half of the rung's one gate
//! (`docs/VB-SV0-SDF-SHADOW-PLAN.md` §6 S2). S2 compiles SV0 into all ten shipping VB
//! lit-producer variants, binds the SDF edit list at Set-0 slot 10, and leaves the host writing
//! mode 0, so nothing is reachable. The whole point of the rung is that the feature enters the
//! binary WITHOUT changing a single rendered byte, and the gates below are what make that a
//! measurement rather than a claim.
//!
//! What lives here, and what deliberately does not:
//!
//! | part | instrument | where |
//! |---|---|---|
//! | (a) G1 kill-switch byte identity | this file, [`vb_sv0_kill_switch_byte_identity`] |
//! | (b) the `#ifdef VB_SV0` guard is observable | this file, [`vb_sv0_source_guard_is_visible_to_the_preprocessor`] |
//! | (b') `vb_geo` `.spv` unperturbed | this file, [`vb_geo_spv_unperturbed_by_the_sv0_guard`] |
//! | (c) all six `deferred_pbr` `.spv` unperturbed | ALREADY EXISTS — `cluster_grid_read_bound.rs::deferred_and_forward_families_spv_byte_identical` (VB-P1k), extended in doc only |
//! | (d) VB image goldens byte-identical | `scripts\golden.ps1`, run by the orchestrator |
//! | (e) `spirv-val` clean on all ten | this file, [`vb_sv0_ten_rows_are_spirv_val_clean`] + its harness control |
//! | (e') the Set-0 binding decision is collision-free | this file, [`vb_sv0_set0_binding_census`] |
//! | (e'') the DARK PATH pays nothing for SV0 | this file, [`vb_sv0_face_normal_chain_is_gated_not_straight_line`] |
//! | (f) the G2 ULP-probe control | `-D VB_SV0_ULP_PROBE=1`, an UNCOMMITTED compile the orchestrator renders |
//! | (g) the moved leaf's two sync pins | `ddgi_probe_gi_sync.rs` + `sdf_field_edsl_sync.rs`, re-targeted this rung |
//!
//! # Gate (e'') exists because NOTHING ELSE HERE CAN SEE A DARK COST
//!
//! Gate (d) is image byte-identity, blind to cost by construction. G1 compares the KILL compile, in
//! which the SV0 spans do not exist at all. So the whole gate set was satisfiable by a shipped
//! default that computed a `cross`, two `dot`s, an `rsqrt`, a `Normalize`, two branches and a
//! conditional negate on EVERY covered pixel for a feature whose runtime mode is 0 — and S2's first
//! landing did exactly that, with the face-normal math in `vb_geom_fetch`'s straight-line result
//! construction. Principles 1 and 3 were being violated in the shipped default while every gate
//! stayed green. [`vb_sv0_face_normal_chain_is_gated_not_straight_line`] is the instrument that
//! closes it, and its red is not hypothetical: it fails on all ten of the pre-fix artifacts.
//!
//! # Two of the plan's named red mutations were MEASURED DEAD, and are replaced here
//!
//! This plan's own standing rule is that a gate part with no demonstrated red is not yet a gate.
//! Running the mutations rather than reasoning about them turned up two that cannot fire:
//!
//! * **Gate (c)'s** named mutation — *"place the `#include` at a different point in
//!   `deferred_pbr.hlsl` than the moved span occupied"* — leaves all six `.spv` BYTE-IDENTICAL.
//!   DXC's SPIR-V backend does not preserve the source order of a definition whose dependencies
//!   are unchanged. What gate (c) actually catches is the moved span being CORRUPTED by the move,
//!   and that mutation does fire: perturbing one token of `sdf_soft_shadow_ranged` inside the
//!   shared header reddens 4 of the 6 rows (the two `SHADOW_STAGE=1` VIS rows return before
//!   lighting and dead-strip the leaf entirely — the same structural blindness their
//!   `array_lengths: 0` expectation in `cluster_grid_read_bound.rs` already records).
//! * **Gate (e)'s** named mutation — *"declare `Buf` at a binding already occupied in
//!   `vb_layout0_froxel` → `spirv-val` red"* — is GREEN. Two variables sharing a
//!   `(DescriptorSet, Binding)` pair is legal SPIR-V, and this codebase depends on it: the VB
//!   tails' own Set-1 declares `gCsm`/`gCsmCmp` and `gShadowAtlas`/`gShadowAtlasCmp` as exactly
//!   such aliased pairs. `spirv-val` validates the MODULE, never the module against a host
//!   descriptor layout, so a binding collision is invisible to it by construction.
//!   [`vb_sv0_set0_binding_census`] is the replacement: it reads the decoration that IS the
//!   decision out of each committed artifact and asserts Set 0 carries no duplicate binding and
//!   that `Buf` sits at 10 — where the mutation the plan wanted (`Buf` at 8) reddens.
//!
//! Every static gate here needs `dxc` (and (e) needs `spirv-val`), and every one SKIPS with an
//! `eprintln!` when the pinned toolchain is absent — so under this repo's standing rule the rung
//! is not commit-eligible until these have been RUN and their output pasted into the commit
//! message. A gate proven only on a box that skipped it is not a gate.

use std::path::PathBuf;
use std::process::Command;

/// The shaders directory (`CARGO_MANIFEST_DIR/shaders`), where the committed `.hlsl` and `.spv`
/// live (and where DXC must run so any `#include` resolves).
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Locates a pinned Vulkan-SDK tool by stem (`dxc`, `spirv-val`, `spirv-dis`): first the pinned
/// SDK path (the repo's offline recipe), then `$VULKAN_SDK/Bin`, then `PATH`. `None` makes the
/// caller SKIP — the `cluster_grid_read_bound.rs:64` idiom, generalized over the stem.
fn find_tool(stem: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{stem}.exe") } else { stem.to_string() };
    let pinned = PathBuf::from(format!("C:/VulkanSDK/1.4.350.0/Bin/{exe}"));
    if pinned.exists() {
        return Some(pinned);
    }
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let candidate = PathBuf::from(sdk).join("Bin").join(&exe);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if Command::new(&exe).arg("--version").output().is_ok() {
        return Some(PathBuf::from(exe));
    }
    None
}

/// One shipping VB lit-producer variant: its source, its `-D` set, its committed artifact, and
/// the sha256 of that artifact **as it stood BEFORE SV0 was compiled in**.
struct Row {
    hlsl: &'static str,
    defines: &'static [&'static str],
    spv: &'static str,
    /// MEASURED — do not edit these literals to make a failing run pass.
    ///
    /// The sha256 of the committed `.spv` at the commit immediately preceding rung S2, taken on a
    /// clean tree with the pinned `dxc` 1.4.350.0. Re-derivable by anyone, with no reference to
    /// this file: `git show <S2^>:crates/boyko_rhi_vulkan/shaders/<spv> | sha256sum`. Each was
    /// additionally confirmed to be the re-DXC of its own source under its frozen recipe before
    /// any SV0 edit was made — which is what validates the recipe table these rows encode.
    pre_sv0_sha256: &'static str,
}

/// The ten shipping VB lit-producer `.spv` (`docs/SHADER-VARIANT-MANIFEST.md`'s VB table,
/// `compute.rs`'s embeds). The same ten rows S0's harness (`vb_sv0_offpath.rs`) validated the
/// re-DXC instrument against, now carrying their pre-SV0 fingerprints.
const VB_SV0_ROWS: &[Row] = &[
    Row {
        hlsl: "vb_resolve.comp.hlsl",
        defines: &[],
        spv: "vb_resolve.comp.spv",
        pre_sv0_sha256: "cce3bc56f2387b593bb9cb07789f8065d1d8bb23cc9ada7974d9d80694187fa3",
    },
    Row {
        hlsl: "vb_resolve.comp.hlsl",
        defines: &["FROXEL=1"],
        spv: "vb_resolve_froxel.comp.spv",
        pre_sv0_sha256: "2b5dc4ccc16a4231f7e8aba94b8fff99a22047a98de3d9f09e26cb30750b5c93",
    },
    Row {
        hlsl: "vb_shade.comp.hlsl",
        defines: &[],
        spv: "vb_shade.comp.spv",
        pre_sv0_sha256: "181788b1e8a692f158ca15f231513b60ef5564c7f419afec0823f76a8fc5d718",
    },
    Row {
        hlsl: "vb_shade.comp.hlsl",
        defines: &["TEXTURED=1"],
        spv: "vb_shade_tex.comp.spv",
        pre_sv0_sha256: "b95ef6127dda16e3aaad335b796aaa38045b49fd3f5d81b5750f22c998ab85d8",
    },
    Row {
        hlsl: "vb_shade.comp.hlsl",
        defines: &["FROXEL=1"],
        spv: "vb_shade_froxel.comp.spv",
        pre_sv0_sha256: "b9c9e11e4eeddd22b49211ae395a7dd4cbec82487d4a71b972f747597d026a1f",
    },
    Row {
        hlsl: "vb_shade.comp.hlsl",
        defines: &["TEXTURED=1", "FROXEL=1"],
        spv: "vb_shade_tex_froxel.comp.spv",
        pre_sv0_sha256: "67e44e786edb775c3c9fc004e8c1e8b4953e5bec871b714459806188430161a1",
    },
    Row {
        hlsl: "vb_shade_split.comp.hlsl",
        defines: &[],
        spv: "vb_shade_split.comp.spv",
        pre_sv0_sha256: "cc950dcfae4eda6dcca3b828c81ccc9749ceb8c84c8d25b18291683e8db3905f",
    },
    Row {
        hlsl: "vb_shade_split.comp.hlsl",
        defines: &["TEXTURED=1"],
        spv: "vb_shade_split_tex.comp.spv",
        pre_sv0_sha256: "5066405ee29bfa02726b942380c641c1db15bcc16414747bbf1e9c1e606dfcdf",
    },
    Row {
        hlsl: "vb_shade_split.comp.hlsl",
        defines: &["HWRT=1"],
        spv: "vb_shade_split_hwrt.comp.spv",
        pre_sv0_sha256: "015470409bf0c9a9cb627aecccb7900f3a152f9042510c69f20b92cf734f1894",
    },
    Row {
        hlsl: "vb_shade_split.comp.hlsl",
        defines: &["TEXTURED=1", "HWRT=1"],
        spv: "vb_shade_split_tex_hwrt.comp.spv",
        pre_sv0_sha256: "69a62ef8114f0151feab65d1fd3c00272c211247597dcecb0b5bbd7e752aaaba",
    },
];

/// Re-DXCs `hlsl_name` under the frozen VB recipe (`-spirv -T cs_6_0 -E main
/// -fspv-target-env=vulkan1.3`, no `-O`) plus `defines`, into a fresh temp `.spv` named by
/// `out_tag`, and returns the bytes. NEVER overwrites a committed artifact — the temp path is the
/// only output. Mirrors `vb_sv0_offpath.rs`'s helper.
fn redxc_to_temp(dxc: &PathBuf, dir: &PathBuf, hlsl_name: &str, defines: &[&str], out_tag: &str) -> Vec<u8> {
    let out_spv = std::env::temp_dir().join(format!("{out_tag}.sv0kill.spv"));
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

// ---- SHA-256, from scratch -----------------------------------------------------------------
//
// The pinned literals above must be re-derivable with an ORDINARY `sha256` tool by someone who
// has never opened this file — that is the whole value of pinning a hash rather than a byte
// blob, and it rules out a bespoke fingerprint. `boyko_render::smaa_luts`' test module already
// carries this exact from-scratch implementation for the same reason (a regression pin, never a
// hot path, never a `pub` surface), so a second copy here is the Principle-5-compliant choice
// over adding a dependency to a test-only need.

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Lowercase-hex SHA-256 of `data` — byte-for-byte the digest `sha256sum` / `Get-FileHash
/// -Algorithm SHA256` produce, which is what makes the pinned literals independently checkable.
fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    // Pad: 0x80, zeros, then the 64-bit BIG-ENDIAN bit length, to a multiple of 64 bytes.
    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().enumerate().take(16) {
            let b = i * 4;
            *word = u32::from_be_bytes([chunk[b], chunk[b + 1], chunk[b + 2], chunk[b + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    h.iter().map(|word| format!("{word:08x}")).collect()
}

/// FIPS 180-4 test vector — validates the from-scratch digest BEFORE the ten pinned literals are
/// trusted to it. Without this, a subtly wrong SHA-256 would make gate (a) compare two garbage
/// values and could go green on genuinely divergent bytes.
#[test]
fn sha256_self_test_matches_the_known_empty_string_digest() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

/// **S2 gate (a) — G1, the kill-switch byte identity.**
///
/// Each of the ten rows, compiled under its own frozen `-D` set PLUS `-D VB_SV0_KILL=1` into a
/// temp file that is never committed, must sha256-equal the artifact's PRE-SV0 bytes.
///
/// What this proves and what it does not, stated as narrowly as it is true, because two red
/// mutations were RUN against it and they did not agree:
///
/// * Moving an SV0 **statement** outside its `#ifndef VB_SV0_KILL` guard reddens G1 — the kill
///   compile then still carries that statement and the digest differs. That is the failure the
///   gate exists for and it fires.
/// * Moving the `Buf` **declaration** outside its guard does NOT redden G1. Under the kill compile
///   nothing references `Buf`, and DXC strips an unreferenced resource — decoration, variable and
///   all — so the bytes come back identical anyway.
///
/// So G1 bounds the SV0 spans' OBSERVABLE EFFECT under the kill compile; it does NOT prove
/// TEXTUAL CONTAINMENT, and it is not a guard-completeness proof. Anything DXC would dead-strip
/// (an unreferenced binding, an unread struct member, an uncalled function) can sit outside a guard
/// and stay invisible here. The shipped **direction** of that hole — a binding present in the
/// artifact at the wrong slot — is covered by [`vb_sv0_set0_binding_census`], which reads `Buf`'s
/// `DescriptorSet`/`Binding` decorations out of each committed `.spv` and therefore cannot be
/// satisfied by a stripped declaration.
///
/// G1 also does NOT prove the SHIPPED module's `sv0_mode == 0` execution is bit-identical to the
/// frozen one, because DXC optimises the SV0-bearing module as a whole. That half is the executing
/// golden (gate (d)) with its own demonstrated sensitivity control (gate (f)).
#[test]
fn vb_sv0_kill_switch_byte_identity() {
    let Some(dxc) = find_tool("dxc") else {
        eprintln!(
            "vb_sv0_kill_switch: dxc not found (no C:/VulkanSDK/.../dxc.exe, no $VULKAN_SDK/Bin, \
             not on PATH) — SKIPPING G1 on this host."
        );
        return;
    };
    let dir = shaders_dir();
    for row in VB_SV0_ROWS {
        let mut defines: Vec<&str> = row.defines.to_vec();
        defines.push("VB_SV0_KILL=1");
        let bytes = redxc_to_temp(&dxc, &dir, row.hlsl, &defines, &format!("kill_{}", row.spv));
        let got = sha256_hex(&bytes);
        assert_eq!(
            got, row.pre_sv0_sha256,
            "G1 RED for {}: the kill compile ({} bytes) does NOT reproduce the PRE-SV0 artifact. \
             Some SV0 span in {} sits OUTSIDE its `#ifndef VB_SV0_KILL` guard, so the feature is \
             not textually inert and the ten re-pins cannot be attributed to the guarded spans \
             alone. Re-derive the expected value with \
             `git show <S2^>:crates/boyko_rhi_vulkan/shaders/{} | sha256sum` — do NOT edit the \
             literal to make this pass.",
            row.spv,
            bytes.len(),
            row.hlsl,
            row.spv,
        );
    }
    eprintln!(
        "vb_sv0_kill_switch: G1 green — all {} rows kill-compile to their pre-SV0 bytes.",
        VB_SV0_ROWS.len()
    );
}

/// **S2 gate (b) — the `#ifdef VB_SV0` source guard, checked where it is OBSERVABLE.**
///
/// Rev 3 of the plan asserted that deleting the guard would move `vb_geo.comp.spv`'s bytes.
/// Rev 4 MEASURED that and it is false: the guarded members plus the face-normal math are fully
/// SROA'd and dead-stripped by DXC's default `-O3` when nothing reads them, and both `vb_geo`
/// artifacts come out byte-identical with or without the guard. That strengthens the protective
/// claim — `vb_geo` provably pays nothing for SV0 either way — while destroying the gate's
/// falsifiability, and a gate whose only red is unreachable is not a gate.
///
/// So the guard is checked one level up, at the preprocessor, where it is a preprocessor
/// construct: `dxc -P` on `vb_geo.comp.hlsl` must NOT emit the guarded symbols, and the same file
/// preprocessed WITH `-D VB_SV0=1` must. Deleting the `#ifdef` makes the first half fail
/// immediately — red by construction, not by codegen luck.
///
/// The probed symbols are `tri_p0` (the first of the three world-position members) and
/// `vb_sv0_face_normal` (the leaf the tails call from inside their runtime gate). Both are probed,
/// not just one, because the two halves of the guarded region can be broken independently: the
/// members without the leaf compiles but nothing consumes them, and the leaf without the members
/// does not compile at all.
#[test]
fn vb_sv0_source_guard_is_visible_to_the_preprocessor() {
    let Some(dxc) = find_tool("dxc") else {
        eprintln!("vb_sv0_kill_switch: dxc not found — SKIPPING the gate (b) preprocessor check.");
        return;
    };
    let dir = shaders_dir();

    let preprocess = |extra: &[&str]| -> String {
        let out = std::env::temp_dir().join(format!("vb_geo_sv0_pp_{}.i", extra.len()));
        let mut cmd = Command::new(&dxc);
        cmd.current_dir(&dir).args(["-P", "-T", "cs_6_0", "-E", "main"]);
        for d in extra {
            cmd.args(["-D", d]);
        }
        cmd.args(["vb_geo.comp.hlsl", "-Fi"]).arg(&out);
        let status = cmd.status().expect("invariant: dxc was located and must run");
        assert!(status.success(), "dxc -P failed on vb_geo.comp.hlsl {extra:?}");
        let text = std::fs::read_to_string(&out).expect("invariant: dxc -P wrote the expansion");
        let _ = std::fs::remove_file(&out);
        text
    };

    let without = preprocess(&[]);
    let with = preprocess(&["VB_SV0=1"]);

    for symbol in ["tri_p0", "vb_sv0_face_normal"] {
        assert!(
            !without.contains(symbol),
            "gate (b) RED: `vb_geo.comp.hlsl` preprocesses to text CONTAINING `{symbol}` without \
             `VB_SV0` defined. The `#ifdef VB_SV0` guard in `vb_geom_fetch.hlsli` is gone or \
             mis-spelled, so `vb_geo` no longer preprocesses character-identically to its pre-SV0 \
             form and its two `.spv` are byte-identical only by DXC's dead-code elimination rather \
             than by construction."
        );
        assert!(
            with.contains(symbol),
            "gate (b) RED: `vb_geo.comp.hlsl` preprocesses to text WITHOUT `{symbol}` even with \
             `VB_SV0` defined. Part of the guarded region is missing: without the three `tri_p*` \
             members the leaf cannot compile, and without `vb_sv0_face_normal` the three VB tails' \
             shadow-origin lift has nothing to call."
        );
    }
    assert_ne!(
        without, with,
        "gate (b) RED: the two preprocessor expansions are identical, so the `#ifdef VB_SV0` \
         guard has no observable effect at the level it operates on"
    );
    eprintln!(
        "vb_sv0_kill_switch: gate (b) green — vb_geo.comp.hlsl expands to {} chars without \
         VB_SV0 and {} chars with it; `tri_p0` and `vb_sv0_face_normal` appear only in the latter.",
        without.len(),
        with.len()
    );
}

/// **S2 gate (b′) — the protective half: `vb_geo`'s two `.spv` are unperturbed.**
///
/// `vb_geom_fetch.hlsli` gained a member and a computation this rung, and `vb_geo.comp.hlsl` is
/// its fourth includer. It does not `#define VB_SV0`, so it preprocesses character-identically to
/// its pre-SV0 form and both artifacts must re-DXC byte-identically.
///
/// Honestly labelled: for the mutation gate (b) exists to catch (deleting the guard) this
/// assertion provably CANNOT fail — see [`vb_sv0_source_guard_is_visible_to_the_preprocessor`].
/// It is kept because it does catch a different, real defect: the SV0 edit accidentally
/// perturbing the UNguarded part of `vb_geom_fetch.hlsli` that `vb_geo` does compile.
#[test]
fn vb_geo_spv_unperturbed_by_the_sv0_guard() {
    let Some(dxc) = find_tool("dxc") else {
        eprintln!("vb_sv0_kill_switch: dxc not found — SKIPPING the vb_geo byte-identity check.");
        return;
    };
    let dir = shaders_dir();
    for (defines, spv) in [
        (&[] as &[&str], "vb_geo.comp.spv"),
        (&["MOTION=1"] as &[&str], "vb_geo_mv.comp.spv"),
    ] {
        let committed = std::fs::read(dir.join(spv))
            .unwrap_or_else(|e| panic!("missing committed {spv}: {e}"));
        let fresh = redxc_to_temp(&dxc, &dir, "vb_geo.comp.hlsl", defines, spv);
        assert!(
            committed == fresh,
            "{spv} ({} bytes committed, {} bytes fresh) MOVED. `vb_geo.comp.hlsl` never defines \
             `VB_SV0`, so SV0 must not reach it — this means the rung perturbed the UNGUARDED \
             part of `vb_geom_fetch.hlsli`.",
            committed.len(),
            fresh.len(),
        );
    }
}

/// **S2 gate (e) — `spirv-val` clean on all ten re-pinned artifacts.**
///
/// Paired with its own harness control below, because a validator that silently accepts anything
/// is the "instrument that does nothing" failure this campaign has hit repeatedly.
#[test]
fn vb_sv0_ten_rows_are_spirv_val_clean() {
    let Some(val) = find_tool("spirv-val") else {
        eprintln!("vb_sv0_kill_switch: spirv-val not found — SKIPPING gate (e) on this host.");
        return;
    };
    let dir = shaders_dir();
    for row in VB_SV0_ROWS {
        let out = Command::new(&val)
            .args(["--target-env", "vulkan1.3"])
            .arg(dir.join(row.spv))
            .output()
            .expect("invariant: spirv-val was located and must run");
        assert!(
            out.status.success(),
            "spirv-val rejected the committed {}:\n{}",
            row.spv,
            String::from_utf8_lossy(&out.stderr),
        );
    }
}

/// The gate-(e) HARNESS CONTROL: `spirv-val` must REJECT a module whose bytes were corrupted.
/// Without this, "clean on all ten" is consistent with the validator never having looked at the
/// artifacts at all. Operates on a temp copy; the committed file is never written.
#[test]
fn spirv_val_rejects_a_corrupted_module() {
    let Some(val) = find_tool("spirv-val") else {
        eprintln!("vb_sv0_kill_switch: spirv-val not found — SKIPPING the gate (e) control.");
        return;
    };
    let dir = shaders_dir();
    let mut bytes = std::fs::read(dir.join("vb_resolve.comp.spv"))
        .expect("invariant: vb_resolve.comp.spv is a committed artifact");
    // Byte 200 lands well inside the module's instruction stream, past the 20-byte header.
    bytes[200] = bytes[200].wrapping_add(1);
    let path = std::env::temp_dir().join("vb_sv0_val_control_corrupt.spv");
    std::fs::write(&path, &bytes).expect("invariant: temp dir is writable");
    let out = Command::new(&val)
        .args(["--target-env", "vulkan1.3"])
        .arg(&path)
        .output()
        .expect("invariant: spirv-val was located and must run");
    let _ = std::fs::remove_file(&path);
    assert!(
        !out.status.success(),
        "the gate-(e) control is BLIND: spirv-val accepted a byte-corrupted module, so \
         `vb_sv0_ten_rows_are_spirv_val_clean` proves nothing about the ten artifacts"
    );
}

/// One parsed SPIR-V result instruction: its opcode and the ids appearing in its operand list.
struct Insn<'a> {
    /// Disassembly line index (0-based), used to find the enclosing `OpLabel`.
    line: usize,
    op: &'a str,
    ops: Vec<&'a str>,
}

/// Parses `spirv-dis` output into `id -> Insn` for every `%id = OpSomething ...` line. Operand ids
/// are every `%token` on the right of the `=`, which over-approximates (it includes type ids); that
/// is harmless here because the walk below only ever asks "can this id be reached", and a type id
/// leads nowhere.
/// `BTreeMap` and not `HashMap`: this is a load-time parser over a few thousand lines, and the
/// ordered iteration makes the "find the Cross that squares itself" search deterministic, so a
/// failure is reproducible rather than dependent on hash order.
fn parse_result_insns(dis: &str) -> std::collections::BTreeMap<&str, Insn<'_>> {
    let mut map: std::collections::BTreeMap<&str, Insn<'_>> = std::collections::BTreeMap::new();
    for (line, text) in dis.lines().enumerate() {
        let t = text.trim();
        let Some((lhs, rhs)) = t.split_once(" = ") else { continue };
        if !lhs.starts_with('%') {
            continue;
        }
        let mut it = rhs.split_whitespace();
        let Some(op) = it.next() else { continue };
        let ops = rhs.split_whitespace().filter(|w| w.starts_with('%')).collect();
        map.insert(lhs, Insn { line, op, ops });
    }
    map
}

/// **S2 gate (e″) — the DARK PATH pays nothing: the SV0 face-normal chain is behind the gate.**
///
/// `vb_sv0_face_normal` (`vb_geom_fetch.hlsli`) is a `cross` + two `dot`s + an `rsqrt` + a
/// `Normalize` + two selects + a conditional negate. It is called from inside each tail's
/// `if ((sv0_mode & VB_SDF_MESH_SHADOW_BIT) != 0u && NoL > SHADOW_NDOTL_EPS)`. If it ever migrates
/// back into `vb_geom_fetch`'s straight-line result construction — or if `-O3` speculates it out of
/// the conditional — every covered pixel on a SHIPPED, SV0-dark frame pays for it, and no other gate
/// in this rung can see that (see the module doc).
///
/// The check is STRUCTURAL, read out of the committed artifact by `spirv-dis`, so it does not
/// depend on the source's shape:
///
/// 1. locate the SV0 mode value — the result of `OpShiftRightLogical %uint %word7 %uint_5`;
/// 2. locate the face-normal `Cross` unambiguously: the one whose result feeds `OpDot %c %c`
///    (`dot(fn, fn)` — no other `Cross` in these shaders squares itself);
/// 3. take its enclosing basic-block label, and require that block to be the target of some
///    `OpBranchConditional` — a block reached only by a conditional branch. A straight-line
///    placement fails here immediately, which is exactly how the pre-fix artifacts fail;
/// 4. walk that branch's predicate BACKWARD to the mode value. Data operands alone are not enough:
///    DXC lowers `a && b` into short-circuit control flow, so the combining `OpPhi %bool %false
///    <blk> ...` depends on the SV0 bit through the CONTROL edge into `<blk>`, never through an
///    operand. The walk therefore also follows, for each block label it meets, the predicates of the
///    branches that target it and of its own terminator — the conditions that must hold for the
///    block to be entered at all.
///
/// MEASURED RED: all ten pre-fix artifacts (the shape S2 first landed) fail at step 3 — the chain's
/// enclosing block is the entry-reachable straight-line block and is the target of no conditional
/// branch. MEASURED GREEN: all ten as committed reach step 4 and the predicate derives from the
/// mode value, with the guarded region spanning ~300 disassembly lines (the face-normal chain plus
/// the inlined 128-iteration march).
#[test]
fn vb_sv0_face_normal_chain_is_gated_not_straight_line() {
    let Some(dis_tool) = find_tool("spirv-dis") else {
        eprintln!("vb_sv0_kill_switch: spirv-dis not found — SKIPPING gate (e'') on this host.");
        return;
    };
    let dir = shaders_dir();
    for row in VB_SV0_ROWS {
        let out = Command::new(&dis_tool)
            .arg(dir.join(row.spv))
            .output()
            .expect("invariant: spirv-dis was located and must run");
        assert!(out.status.success(), "spirv-dis failed on {}", row.spv);
        let dis = String::from_utf8_lossy(&out.stdout).into_owned();
        let insns = parse_result_insns(&dis);
        let lines: Vec<&str> = dis.lines().collect();

        // (1) the SV0 mode value: `>> 5` of the header word.
        let mode = insns
            .iter()
            .find(|(_, i)| i.op == "OpShiftRightLogical" && lines[i.line].trim().ends_with("%uint_5"))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "{}: no `OpShiftRightLogical ... %uint_5` — `load_vb_sdf_mesh_mode`'s header \
                     read is gone, so SV0's runtime gate is not in this module at all",
                    row.spv
                )
            });

        // (2) the face-normal Cross: its result is squared by an `OpDot %c %c`.
        let cross = insns
            .iter()
            .find(|(id, i)| {
                i.op == "OpExtInst"
                    && lines[i.line].split_whitespace().any(|w| w == "Cross")
                    && insns.values().any(|d| {
                        d.op == "OpDot" && d.ops.iter().filter(|o| *o == *id).count() == 2
                    })
            })
            .map(|(id, i)| (*id, i.line))
            .unwrap_or_else(|| {
                panic!(
                    "{}: no `Cross` whose result feeds `dot(fn, fn)` — `vb_sv0_face_normal` is not \
                     in this module. Either the shadow-origin lift was removed or the guard \
                     spelling changed; this gate cannot certify a module it cannot find the chain in",
                    row.spv
                )
            });

        // (3) its enclosing basic block, and the conditional branch that targets it.
        let block = (0..=cross.1)
            .rev()
            .find_map(|l| {
                let t = lines[l].trim();
                t.split_once(" = ").filter(|(_, r)| r.starts_with("OpLabel")).map(|(id, _)| id)
            })
            .unwrap_or_else(|| panic!("{}: the Cross has no enclosing OpLabel", row.spv));

        let mut pred: Option<&str> = None;
        for text in &lines {
            let t: Vec<&str> = text.split_whitespace().collect();
            if t.first() == Some(&"OpBranchConditional") && t.len() >= 4 && (t[2] == block || t[3] == block) {
                pred = Some(t[1]);
            }
        }
        let pred = pred.unwrap_or_else(|| {
            panic!(
                "gate (e'') RED for {}: the SV0 face-normal chain's basic block `{block}` is the \
                 target of NO conditional branch — it is STRAIGHT-LINE code. Every covered pixel \
                 therefore pays a cross + two dots + an rsqrt + a Normalize + a conditional negate \
                 while the SV0 runtime mode is 0, which is a dark cost for a feature that is off \
                 (principles 1 and 3). Move the computation back inside the tails' \
                 `if ((sv0_mode & VB_SDF_MESH_SHADOW_BIT) != 0u && ...)` block — `vb_geom_fetch` \
                 must export only the three `tri_p*` world positions.",
                row.spv
            )
        });

        // (4) backward reachability from the predicate to the mode value, over DATA and CONTROL.
        let mut seen: Vec<&str> = Vec::new();
        let mut stack: Vec<&str> = vec![pred];
        let mut reaches = false;
        while let Some(x) = stack.pop() {
            if seen.contains(&x) {
                continue;
            }
            seen.push(x);
            if x == mode {
                reaches = true;
                break;
            }
            let Some(i) = insns.get(x) else { continue };
            stack.extend(i.ops.iter().copied());
            if i.op != "OpLabel" {
                continue;
            }
            // the block's own terminator predicate
            for text in lines.iter().skip(i.line) {
                let t: Vec<&str> = text.split_whitespace().collect();
                if t.first() == Some(&"OpBranchConditional") && t.len() >= 2 {
                    stack.push(t[1]);
                    break;
                }
                if t.first() == Some(&"OpBranch") {
                    break;
                }
            }
            // every conditional branch that targets this block
            for text in &lines {
                let t: Vec<&str> = text.split_whitespace().collect();
                if t.first() == Some(&"OpBranchConditional") && t.len() >= 4 && (t[2] == x || t[3] == x) {
                    stack.push(t[1]);
                }
            }
        }
        assert!(
            reaches,
            "gate (e'') RED for {}: the face-normal chain's block `{block}` IS conditional, but its \
             predicate `{pred}` does not derive from the SV0 mode value `{mode}`. The chain is \
             gated on SOMETHING ELSE, so a frame with SV0 dark can still execute it.",
            row.spv
        );
    }
    eprintln!(
        "vb_sv0_kill_switch: gate (e'') green across all {} rows — the SV0 face-normal chain is \
         reached only under a predicate derived from the runtime mode; the dark path pays nothing.",
        VB_SV0_ROWS.len()
    );
}

/// **S2 gate (e′) — the Set-0 binding census: the instrument gate (e)'s dead mutation needed.**
///
/// The hazard §2 of the plan reasons about is a SILENT descriptor collision: slot 8 is
/// `ClusterGrid` under `#ifdef FROXEL`, so binding SV0's `Buf` there would be correct in every
/// scene that never arms the froxel cull and wrong — with `robustBufferAccess` off, validation
/// off, and no GPU-assisted layer — in every scene that does. `spirv-val` cannot see this: it
/// validates a module, never a module against a host layout, and aliased `(set, binding)` pairs
/// are legal and already shipped by these very files' Set 1.
///
/// This reads the decorations that ARE the decision out of each committed artifact:
///
/// 1. `Buf` carries `DescriptorSet 0` + `Binding 10` in all ten rows;
/// 2. no two DISTINCT Set-0 variables share a binding number in any row;
/// 3. no Set-0 binding is `8` or `9` other than the froxel pair itself.
///
/// Red mutation: declare `Buf` at binding 8. (1) reddens on all ten and (2) reddens on the four
/// froxel rows, where `Buf` and `ClusterGrid` would then both decorate 8.
#[test]
fn vb_sv0_set0_binding_census() {
    let Some(dis) = find_tool("spirv-dis") else {
        eprintln!("vb_sv0_kill_switch: spirv-dis not found — SKIPPING the Set-0 binding census.");
        return;
    };
    let dir = shaders_dir();
    for row in VB_SV0_ROWS {
        let out = Command::new(&dis)
            .arg(dir.join(row.spv))
            .output()
            .expect("invariant: spirv-dis was located and must run");
        assert!(out.status.success(), "spirv-dis failed on {}", row.spv);
        let dis_text = String::from_utf8_lossy(&out.stdout);

        // Collect `OpDecorate %var DescriptorSet N` / `OpDecorate %var Binding N` pairs. Matching
        // is by EXACT whitespace-split token, the `cluster_grid_read_bound.rs:174` idiom, so a
        // longer identifier can never false-match a shorter one.
        let mut sets: Vec<(&str, u32)> = Vec::new();
        let mut bindings: Vec<(&str, u32)> = Vec::new();
        for line in dis_text.lines() {
            let t: Vec<&str> = line.split_whitespace().collect();
            if t.len() < 4 || t[0] != "OpDecorate" {
                continue;
            }
            let Ok(n) = t[3].parse::<u32>() else { continue };
            match t[2] {
                "DescriptorSet" => sets.push((t[1], n)),
                "Binding" => bindings.push((t[1], n)),
                _ => {}
            }
        }

        // (1) `Buf` is in Set 0 at binding 10, exactly once.
        let buf_set: Vec<u32> = sets.iter().filter(|(v, _)| *v == "%Buf").map(|(_, n)| *n).collect();
        let buf_binding: Vec<u32> =
            bindings.iter().filter(|(v, _)| *v == "%Buf").map(|(_, n)| *n).collect();
        assert_eq!(
            buf_set,
            vec![0],
            "{}: SV0's `Buf` must be decorated `DescriptorSet 0` exactly once (got {buf_set:?}) — \
             the VB tails have no fifth set to put it in (the TEXTURED variant already consumes \
             all four and Vulkan's guaranteed floor is exactly 4)",
            row.spv
        );
        assert_eq!(
            buf_binding,
            vec![10],
            "{}: SV0's `Buf` must be decorated `Binding 10` exactly once (got {buf_binding:?}). \
             Slot 10 is the ONLY slot free in BOTH `vb_layout0` and `vb_layout0_froxel`; slot 8 \
             is `ClusterGrid` under `#ifdef FROXEL`, so binding there is a collision that is \
             correct on unclustered scenes and silently wrong on clustered ones.",
            row.spv
        );

        // (2) No two DISTINCT Set-0 variables share a binding number.
        let set0_vars: Vec<&str> =
            sets.iter().filter(|(_, n)| *n == 0).map(|(v, _)| *v).collect();
        let mut seen: Vec<(u32, &str)> = Vec::new();
        for (var, n) in &bindings {
            if !set0_vars.contains(var) {
                continue;
            }
            if let Some((_, other)) = seen.iter().find(|(b, o)| b == n && o != var) {
                panic!(
                    "{}: Set-0 binding {n} is decorated by BOTH `{other}` and `{var}`. Aliased \
                     `(set, binding)` pairs are legal SPIR-V (this file's Set 1 uses them for the \
                     combined image+sampler shadow tables), which is exactly why `spirv-val` \
                     cannot catch this — but in Set 0 it means two different resources are being \
                     written to one descriptor slot.",
                    row.spv
                );
            }
            seen.push((*n, var));
        }

        // (3) Bindings 8/9 in Set 0 belong to the froxel pair alone.
        for (var, n) in &bindings {
            if !set0_vars.contains(var) || (*n != 8 && *n != 9) {
                continue;
            }
            assert!(
                *var == "%ClusterGrid" || *var == "%LightIndexList",
                "{}: Set-0 binding {n} is decorated by `{var}`, but 8/9 are reserved for the \
                 `#ifdef FROXEL` cluster pair",
                row.spv
            );
        }
    }
    eprintln!(
        "vb_sv0_kill_switch: Set-0 binding census green across all {} rows — `Buf` @10, no \
         duplicate Set-0 binding, 8/9 reserved to the froxel pair.",
        VB_SV0_ROWS.len()
    );
}
