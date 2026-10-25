//! IBL1 R1 (`ibl1/cut.md` C10) — **the environment arm's generated header and its 20 `-D REFL=1`
//! modules are the eDSL's and the frozen recipes' output, and each module is a real twin**.
//!
//! 1. `env_ibl_spans_match_the_emitter`: the span between `env_ibl.hlsli`'s
//!    `// === GENERATED env_ibl BEGIN/END ===` sentinels equals
//!    `boyko_shaderdsl::emit::emit_hlsl_env_ibl()` (the `interp_edsl_sync` text gate). The leaves
//!    there are the SAME `FieldScalar` bodies the bake and the host oracles instantiate over f32, so
//!    a hand edit of the header is a red here, not a silent bake/read split.
//! 2. `every_refl_spv_is_the_redxc_of_its_source`: each of the 20 REFL rows re-DXC'd under its frozen
//!    recipe (the base row's recipe + `-D REFL=1`) is byte-identical to its committed `.spv`.
//! 3. `every_refl_spv_adds_exactly_two_explicit_lod_samples` (rule (xviii)'s anti-vacuity marker): each
//!    REFL module differs from its base and its disassembly holds exactly TWO more
//!    `OpImageSampleExplicitLod` than the base's — the environment's two per-level fetches. A twin
//!    built without `-D REFL` is byte-equal to its base and fails this.
//! 4. `refl_row_table_names_every_committed_spv`: the table names 20 distinct committed modules.
//!
//! Gates 2 and 3 SKIP (with an `eprintln`) when no `dxc` / `spirv-dis` resolves — a skipped run is
//! not a pass; the lane's tester runs them where the pinned VulkanSDK 1.4.350.0 is installed. The
//! base rows' own byte gates are the existing `*_spv_sync` files, which this rung leaves green with
//! no committed-byte change (the base text preprocesses character-identical with `REFL` undefined).

use std::path::PathBuf;
use std::process::Command;

mod child_guard;
use child_guard::{BoundedRun, DXC_DEADLINE};

/// The generated header.
const HEADER: &str = "env_ibl.hlsli";
/// The opening sentinel line of the generated span.
const BEGIN: &str = "// === GENERATED env_ibl BEGIN === (boyko_shaderdsl::emit::emit_hlsl_env_ibl)\n";
/// The closing sentinel of the generated span.
const END: &str = "// === GENERATED env_ibl END ===";

/// `(source, profile, defines, REFL module, base module)` — the 20 rows of the manifest's REFL axis.
const REFL_ROWS: [(&str, &str, &[&str], &str, &str); 20] = [
    ("deferred_pbr.hlsl", "cs_6_0", &["REFL=1"], "deferred_pbr_refl.comp.spv", "deferred_pbr.comp.spv"),
    ("deferred_pbr.hlsl", "cs_6_0", &["TERMINATOR_WRAP=1", "REFL=1"], "deferred_pbr_wrap_refl.comp.spv", "deferred_pbr_wrap.comp.spv"),
    ("deferred_pbr.hlsl", "cs_6_5", &["HWRT=1", "REFL=1"], "deferred_pbr_hwrt_refl.comp.spv", "deferred_pbr_hwrt.comp.spv"),
    ("deferred_pbr.hlsl", "cs_6_5", &["HWRT=1", "SHADOW_STAGE=2", "REFL=1"], "deferred_pbr_hwrt_denoised_refl.comp.spv", "deferred_pbr_hwrt_denoised.comp.spv"),
    ("forward_opaque.fs.hlsl", "ps_6_0", &["REFL=1"], "forward_opaque_refl.fs.spv", "forward_opaque.fs.spv"),
    ("forward_opaque.fs.hlsl", "ps_6_0", &["FROXEL=1", "REFL=1"], "forward_opaque_froxel_refl.fs.spv", "forward_opaque_froxel.fs.spv"),
    ("sdf_forward_march.comp.hlsl", "cs_6_0", &["HAS_MESH=1", "REFL=1"], "sdf_forward_march_refl.comp.spv", "sdf_forward_march.comp.spv"),
    ("sdf_forward_march.comp.hlsl", "cs_6_0", &["REFL=1"], "sdf_forward_march_sdfonly_refl.comp.spv", "sdf_forward_march_sdfonly.comp.spv"),
    ("sdf_forward_march.comp.hlsl", "cs_6_0", &["HAS_MESH=1", "VIEWT=1", "REFL=1"], "sdf_forward_march_viewt_refl.comp.spv", "sdf_forward_march_viewt.comp.spv"),
    ("sdf_forward_march.comp.hlsl", "cs_6_0", &["VIEWT=1", "REFL=1"], "sdf_forward_march_sdfonly_viewt_refl.comp.spv", "sdf_forward_march_sdfonly_viewt.comp.spv"),
    ("vb_resolve.comp.hlsl", "cs_6_0", &["REFL=1"], "vb_resolve_refl.comp.spv", "vb_resolve.comp.spv"),
    ("vb_resolve.comp.hlsl", "cs_6_0", &["FROXEL=1", "REFL=1"], "vb_resolve_froxel_refl.comp.spv", "vb_resolve_froxel.comp.spv"),
    ("vb_shade.comp.hlsl", "cs_6_0", &["REFL=1"], "vb_shade_refl.comp.spv", "vb_shade.comp.spv"),
    ("vb_shade.comp.hlsl", "cs_6_0", &["FROXEL=1", "REFL=1"], "vb_shade_froxel_refl.comp.spv", "vb_shade_froxel.comp.spv"),
    ("vb_shade.comp.hlsl", "cs_6_0", &["TEXTURED=1", "REFL=1"], "vb_shade_tex_refl.comp.spv", "vb_shade_tex.comp.spv"),
    ("vb_shade.comp.hlsl", "cs_6_0", &["TEXTURED=1", "FROXEL=1", "REFL=1"], "vb_shade_tex_froxel_refl.comp.spv", "vb_shade_tex_froxel.comp.spv"),
    ("vb_shade_split.comp.hlsl", "cs_6_0", &["REFL=1"], "vb_shade_split_refl.comp.spv", "vb_shade_split.comp.spv"),
    ("vb_shade_split.comp.hlsl", "cs_6_0", &["TEXTURED=1", "REFL=1"], "vb_shade_split_tex_refl.comp.spv", "vb_shade_split_tex.comp.spv"),
    ("vb_shade_split.comp.hlsl", "cs_6_0", &["HWRT=1", "REFL=1"], "vb_shade_split_hwrt_refl.comp.spv", "vb_shade_split_hwrt.comp.spv"),
    ("vb_shade_split.comp.hlsl", "cs_6_0", &["TEXTURED=1", "HWRT=1", "REFL=1"], "vb_shade_split_tex_hwrt_refl.comp.spv", "vb_shade_split_tex_hwrt.comp.spv"),
];

fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// An SDK tool: the pinned VulkanSDK's, then `$VULKAN_SDK/Bin`, then `PATH`.
fn find_tool(stem: &str) -> Option<PathBuf> {
    let bare = if cfg!(windows) { format!("{stem}.exe") } else { stem.to_string() };
    let pinned = PathBuf::from(format!("C:/VulkanSDK/1.4.350.0/Bin/{bare}"));
    if pinned.exists() {
        return Some(pinned);
    }
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        let c = PathBuf::from(sdk).join("Bin").join(&bare);
        if c.exists() {
            return Some(c);
        }
    }
    Command::new(&bare).arg("--version").output_within(DXC_DEADLINE).is_ok().then(|| PathBuf::from(bare))
}

#[test]
fn env_ibl_spans_match_the_emitter() {
    let path = shaders_dir().join(HEADER);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).replace("\r\n", "\n");
    let start = text.find(BEGIN).expect("env_ibl.hlsli carries the GENERATED env_ibl BEGIN sentinel") + BEGIN.len();
    let end = text[start..].find(END).expect("env_ibl.hlsli carries the GENERATED env_ibl END sentinel") + start;
    let committed = &text[start..end];
    let emitted = boyko_shaderdsl::emit::emit_hlsl_env_ibl();
    assert!(emitted.contains("float3 eval_pbr_ambient_env(") && emitted.contains("LIGHT_FLAG_ENV"), "anti-vacuity: the emitter prints the arm");
    assert_eq!(
        committed, emitted,
        "env_ibl.hlsli's generated span is not the eDSL's output: re-run `cargo run -p boyko_shaderdsl --features emit \
         --bin emit_env_ibl`, re-splice between the sentinels and re-DXC the 20 REFL rows"
    );
}

/// Re-DXCs one row into a scratch file and returns the bytes.
fn redxc(dxc: &PathBuf, src: &str, profile: &str, defines: &[&str], tag: &str) -> Vec<u8> {
    let out = child_guard::scratch_path(&format!("{tag}.redxc"));
    let mut cmd = Command::new(dxc);
    cmd.current_dir(shaders_dir()).args(["-spirv", "-T", profile, "-E", "main"]);
    for d in defines {
        cmd.args(["-D", d]);
    }
    cmd.args(["-fspv-target-env=vulkan1.3", src, "-Fo"]).arg(&out);
    let status = cmd.status_within(DXC_DEADLINE).expect("invariant: dxc was located and must run");
    assert!(status.success(), "dxc failed on {src} {defines:?} under the frozen recipe");
    let bytes = std::fs::read(&out).expect("invariant: dxc wrote the re-DXC .spv");
    let _ = std::fs::remove_file(&out);
    bytes
}

#[test]
fn every_refl_spv_is_the_redxc_of_its_source() {
    let Some(dxc) = find_tool("dxc") else {
        eprintln!("SKIP every_refl_spv_is_the_redxc_of_its_source: no dxc resolves on this host — a skipped run is NOT a pass");
        return;
    };
    let mut stale = Vec::new();
    for (src, profile, defines, refl, _) in REFL_ROWS {
        let fresh = redxc(&dxc, src, profile, defines, refl);
        let committed = std::fs::read(shaders_dir().join(refl)).unwrap_or_else(|e| panic!("{refl}: {e}"));
        if fresh != committed {
            stale.push(format!("{refl} ({} bytes committed, {} re-DXC'd) is stale against {src} {defines:?}", committed.len(), fresh.len()));
        }
    }
    assert!(stale.is_empty(), "re-compile the REFL rows under their header recipes:\n{}", stale.join("\n"));
}

/// The `OpImageSampleExplicitLod` count of a module's disassembly.
fn explicit_lod_samples(dis: &PathBuf, spv: &str) -> usize {
    let out = Command::new(dis)
        .arg(shaders_dir().join(spv))
        .output_within(DXC_DEADLINE)
        .expect("invariant: spirv-dis was located and must run");
    assert!(out.status.success(), "spirv-dis failed on {spv}");
    String::from_utf8_lossy(&out.stdout).matches("OpImageSampleExplicitLod ").count()
}

#[test]
fn every_refl_spv_adds_exactly_two_explicit_lod_samples() {
    let mut problems = Vec::new();
    for (_, _, _, refl, base) in REFL_ROWS {
        let r = std::fs::read(shaders_dir().join(refl)).unwrap_or_else(|e| panic!("{refl}: {e}"));
        let b = std::fs::read(shaders_dir().join(base)).unwrap_or_else(|e| panic!("{base}: {e}"));
        if r == b {
            problems.push(format!("{refl} is byte-equal to {base}: a twin built without -D REFL"));
        }
    }
    let Some(dis) = find_tool("spirv-dis") else {
        assert!(problems.is_empty(), "{}", problems.join("\n"));
        eprintln!("SKIP the OpImageSampleExplicitLod census: no spirv-dis resolves on this host — a skipped run is NOT a pass");
        return;
    };
    for (_, _, _, refl, base) in REFL_ROWS {
        let (nr, nb) = (explicit_lod_samples(&dis, refl), explicit_lod_samples(&dis, base));
        if nr != nb + 2 {
            problems.push(format!("{refl}: {nr} OpImageSampleExplicitLod vs {nb} in {base} (want +2)"));
        }
    }
    assert!(problems.is_empty(), "the REFL twins are not the base + the environment's two fetches:\n{}", problems.join("\n"));
}

#[test]
fn refl_row_table_names_every_committed_spv() {
    for (i, (_, _, defines, refl, base)) in REFL_ROWS.iter().enumerate() {
        assert!(REFL_ROWS[i + 1..].iter().all(|r| r.3 != *refl), "{refl} twice");
        assert!(defines.last() == Some(&"REFL=1"), "{refl}: the REFL define is the row's last");
        assert!(shaders_dir().join(refl).exists(), "{refl} is not committed");
        assert!(shaders_dir().join(base).exists(), "{base} (the base row) is not committed");
    }
}
