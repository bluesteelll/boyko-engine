//! VB-P1k — the `ClusterGrid` READ bound, gated on the committed artifacts.
//!
//! # The defect this pins
//!
//! Four shader sources read `ClusterGrid`. Each maps a pixel to a froxel with
//! `cluster_linear_index(tile.x, tile.y, zsl, cp.dim_x, cp.dim_z)`, whose result is
//! `< dim_x * dim_y * dim_z` **by construction** — so the LIVE light-table header's dims are the
//! only bound the read has. But `ClusterGrid` was SIZED at scene boot from
//! `ClusterConfig::cluster_count()` and is never re-allocated, while
//! `boyko_render::light::sync_cluster_light_gate` republishes the LIVE `ClusterConfig` dims into
//! that header every frame. A post-boot `ClusterConfig` edit that GROWS the grid therefore walks
//! the read off the end of the allocation — silently, `robustBufferAccess` being OFF here with no
//! GPU-assisted validation to report it.
//!
//! Two of the four also lacked the older non-zero-dims defence (VB-P1b-0 C1), which is the
//! sharper of the two holes: a zero-`dim_z` header makes `cluster_z_slice` clamp to
//! `(int)dim_z - 1 == -1` and return `0xFFFFFFFF`, and that is exactly the header
//! `sync_cluster_light_gate` publishes on every boot whose `ResolvedRenderPath::froxel_light_cull`
//! is false — i.e. on every Deferred and every ForwardPlus boot, the two shaders that lacked it.
//!
//! Both are closed by one three-term `use_clusters`, uniform across all four sources:
//! `clusters_enabled != 0 && cluster_count != 0 && cluster_count <= ClusterGrid.GetDimensions()`.
//! `GetDimensions` reports the BOUND DESCRIPTOR's own element count (SPIR-V `OpArrayLength`), so
//! the bound is the allocation itself rather than a host-side mirror of it, and a skewed frame
//! falls back to the in-bounds flat light scan instead of indexing a grid that does not fit.
//!
//! # What this file gates
//!
//! 1. **Byte identity** for the `deferred_pbr.hlsl` and `forward_opaque.fs.hlsl` families under
//!    their own frozen recipes. Those two families had **no `*_spv_sync` gate at all** before
//!    this rung (`vb_froxel_spv_sync.rs` covers the six VB rows, `cluster_cull_spv_sync.rs` the
//!    two cull rows, and nothing covered these eight) — so a stale `deferred_pbr*.comp.spv` was
//!    a silent failure mode, not a loud one.
//! 2. **The read bound is present in every artifact that can index `ClusterGrid`**, and absent
//!    from exactly the ones that cannot. Deleting the capacity term from any of the four sources
//!    drops that artifact's `OpArrayLength` count to 0, which is RED here.
//!
//! # Also serving as VB-SV0's gate (c) — extended in doc only
//!
//! `docs/VB-SV0-SDF-SHADOW-PLAN.md` rung S2 moves `sdf_soft_shadow_ranged` out of
//! `deferred_pbr.hlsl` into the shared `sdf_shadow_leaves.hlsli` (which also carries `sdf_ao` and
//! the three A2 AO consts, neither of which `deferred_pbr.hlsl` references), replacing the span
//! with an `#include` at the point it occupied. Gate (c) of that rung is *"all six `deferred_pbr`
//! `.spv` byte-identical"*, and [`deferred_and_forward_families_spv_byte_identical`] below ALREADY
//! IS that gate — the plan's D3 note that `redxc_with_defines` would need a new `profile`
//! parameter was discharged by VB-P1k, which is when [`Variant::profile`] was introduced. Nothing
//! is added here; a second copy of the same six rows in an SV0-named file would be duplication,
//! not coverage.
//!
//! **What that gate CAN and CANNOT go red for — measured, not reasoned.** The plan named
//! *"place the `#include` at a different point than the moved span occupied"* as gate (c)'s red
//! mutation. Executed, that leaves all six `.spv` **byte-identical**: DXC's SPIR-V backend does
//! not preserve the source position of a definition whose dependencies are unchanged. The
//! mutation that does fire is a CORRUPTED moved span — perturbing one token of
//! `sdf_soft_shadow_ranged` inside the shared header reddens 4 of the 6 rows. The two that stay
//! green are exactly `deferred_pbr_hwrt_vis` and `deferred_pbr_hwrt_vis_mv`, whose
//! `SHADOW_STAGE=1` arm returns before lighting and dead-strips the leaf entirely — the same
//! structural blindness their `array_lengths: 0` expectation in [`OWNED_VARIANTS`] already
//! records, arrived at independently.
//!
//! SKIPS (with an eprintln) when no `dxc` / `spirv-dis` resolves — the byte gate is only as
//! hermetic as the pinned VulkanSDK 1.4.350.0 toolchain that produced the committed artifacts;
//! a DIFFERENT dxc version failing this test means "wrong toolchain", not "drifted shader".

use std::path::PathBuf;
use std::process::Command;

/// The shaders directory (`CARGO_MANIFEST_DIR/shaders`), where the committed `.hlsl` and `.spv`
/// live (and where DXC must run so any `#include` resolves).
fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders")
}

/// Locates the `dxc` executable: first the pinned Vulkan-SDK path (the repo's offline recipe),
/// then `$VULKAN_SDK/Bin`, then `PATH`. Returns `None` if none resolve (the caller then SKIPS) —
/// the `cluster_cull_spv_sync.rs` idiom verbatim.
fn find_dxc() -> Option<PathBuf> {
    find_tool("dxc")
}

/// Locates `spirv-dis` by the same layered lookup [`find_dxc`] uses.
fn find_spirv_dis() -> Option<PathBuf> {
    find_tool("spirv-dis")
}

/// Shared layered tool lookup: pinned SDK path, then `$VULKAN_SDK/Bin`, then `PATH`.
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

/// One committed variant: its source, its dxc target profile, its `-D` defines and its artifact.
/// The rows below ARE the frozen recipes pinned in each source's own header comment, transcribed;
/// a row that disagrees with its header is the bug this table is meant to surface.
struct Variant {
    hlsl: &'static str,
    profile: &'static str,
    defines: &'static [&'static str],
    spv: &'static str,
    /// How many `OpArrayLength` instructions the artifact must carry on `ClusterGrid` — 1 where
    /// the cluster block survives into the module, 0 where it is legitimately absent (see each
    /// row's comment). MEASURED on the artifacts this rung commits.
    array_lengths: usize,
}

/// The `deferred_pbr.hlsl` + `forward_opaque.fs.hlsl` families — the two whose byte identity was
/// previously ungated, and the two whose `use_clusters` gained BOTH defence terms this rung.
const OWNED_VARIANTS: &[Variant] = &[
    // The cluster block is UNCONDITIONAL in `deferred_pbr.hlsl` (no `#ifdef FROXEL`), so every
    // variant that still runs lighting carries the bound.
    Variant { hlsl: "deferred_pbr.hlsl", profile: "cs_6_0", defines: &[], spv: "deferred_pbr.comp.spv", array_lengths: 1 },
    Variant { hlsl: "deferred_pbr.hlsl", profile: "cs_6_0", defines: &["TERMINATOR_WRAP=1"], spv: "deferred_pbr_wrap.comp.spv", array_lengths: 1 },
    Variant { hlsl: "deferred_pbr.hlsl", profile: "cs_6_5", defines: &["HWRT=1"], spv: "deferred_pbr_hwrt.comp.spv", array_lengths: 1 },
    // SHADOW_STAGE=1 (VIS) writes `gShadowVis` and returns BEFORE lighting, so DXC dead-strips
    // the whole cluster block — 0 is the correct expectation, and it is also why these two
    // artifacts stayed byte-identical across this rung.
    Variant { hlsl: "deferred_pbr.hlsl", profile: "cs_6_5", defines: &["HWRT=1", "SHADOW_STAGE=1"], spv: "deferred_pbr_hwrt_vis.comp.spv", array_lengths: 0 },
    Variant { hlsl: "deferred_pbr.hlsl", profile: "cs_6_5", defines: &["HWRT=1", "SHADOW_STAGE=2"], spv: "deferred_pbr_hwrt_denoised.comp.spv", array_lengths: 1 },
    Variant { hlsl: "deferred_pbr.hlsl", profile: "cs_6_5", defines: &["HWRT=1", "SHADOW_STAGE=1", "MOTION_VECTORS=1"], spv: "deferred_pbr_hwrt_vis_mv.comp.spv", array_lengths: 0 },
    // `forward_opaque.fs.hlsl`'s cluster block is `#ifdef FROXEL`-gated: the base `Forward`
    // compile has no cluster block at all, the ForwardPlus one does.
    Variant { hlsl: "forward_opaque.fs.hlsl", profile: "ps_6_0", defines: &[], spv: "forward_opaque.fs.spv", array_lengths: 0 },
    Variant { hlsl: "forward_opaque.fs.hlsl", profile: "ps_6_0", defines: &["FROXEL=1"], spv: "forward_opaque_froxel.fs.spv", array_lengths: 1 },
];

/// The remaining `ClusterGrid`-touching artifacts. Their byte identity is already gated
/// elsewhere (`vb_froxel_spv_sync.rs`, `cluster_cull_spv_sync.rs`), so only the read/write-bound
/// census is asserted here — the point being that the census covers EVERY consumer, not just the
/// two families this file owns.
const CENSUS_ONLY: &[(&str, usize)] = &[
    // VB: the `#ifdef FROXEL` rows carry the bound; the base rows have no cluster block.
    ("vb_resolve.comp.spv", 0),
    ("vb_resolve_froxel.comp.spv", 1),
    ("vb_shade.comp.spv", 0),
    ("vb_shade_tex.comp.spv", 0),
    ("vb_shade_froxel.comp.spv", 1),
    ("vb_shade_tex_froxel.comp.spv", 1),
    // The cull's WRITE side (VB-P1j) — the base arm reads the array length; the HIER arm is
    // bounded by D11's pushed boot capacity instead and carries none, deliberately.
    ("cluster_cull.comp.spv", 1),
    ("cluster_cull_hier.comp.spv", 0),
];

/// Re-DXCs one variant under its frozen recipe into a temp `.spv` and returns the bytes. Never
/// overwrites a committed artifact.
fn redxc(dxc: &PathBuf, dir: &PathBuf, v: &Variant) -> Vec<u8> {
    let out_spv = std::env::temp_dir().join(format!("{}.cgrb.redxc.spv", v.spv));
    let mut cmd = Command::new(dxc);
    cmd.current_dir(dir).args(["-spirv", "-T", v.profile, "-E", "main"]);
    for d in v.defines {
        cmd.args(["-D", d]);
    }
    cmd.args(["-fspv-target-env=vulkan1.3", v.hlsl, "-Fo"]).arg(&out_spv);
    let status = cmd.status().expect("invariant: dxc was located and must run");
    assert!(
        status.success(),
        "dxc failed re-compiling {} {:?} under the frozen recipe",
        v.hlsl,
        v.defines
    );
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

/// Counts `OpArrayLength` instructions whose operand is the `ClusterGrid` variable. Matching is
/// by EXACT whitespace-split token (`%ClusterGrid`), so no longer identifier can false-match.
fn cluster_grid_array_lengths(dis: &str) -> usize {
    dis.lines()
        .filter(|line| {
            let toks: Vec<&str> = line.split_whitespace().collect();
            toks.contains(&"OpArrayLength") && toks.contains(&"%ClusterGrid")
        })
        .count()
}

/// The `deferred_pbr` and `forward_opaque` families byte-equal their own re-DXC under the frozen
/// recipes pinned in their header comments. This gate did not exist before VB-P1k.
#[test]
fn deferred_and_forward_families_spv_byte_identical() {
    let Some(dxc) = find_dxc() else {
        eprintln!(
            "cluster_grid_read_bound: dxc not found (no C:/VulkanSDK/.../dxc.exe, no \
             $VULKAN_SDK/Bin, not on PATH) — SKIPPING the deferred/forward re-DXC byte-identity \
             check on this host."
        );
        return;
    };
    let dir = shaders_dir();
    for v in OWNED_VARIANTS {
        let committed_path = dir.join(v.spv);
        let committed = std::fs::read(&committed_path)
            .unwrap_or_else(|e| panic!("missing committed {}: {e}", committed_path.display()));
        let fresh = redxc(&dxc, &dir, v);
        assert!(
            committed == fresh,
            "{} ({} bytes committed, {} bytes fresh) is NOT the re-DXC of {} {:?} under the \
             frozen recipe — either the committed .spv is stale (re-run the recipe in the \
             shader's header and commit ALL sibling variants together: this family has {} rows) \
             or the host dxc is not the pinned VulkanSDK 1.4.350.0 toolchain.",
            v.spv,
            committed.len(),
            fresh.len(),
            v.hlsl,
            v.defines,
            OWNED_VARIANTS.iter().filter(|o| o.hlsl == v.hlsl).count(),
        );
    }
}

/// Every committed artifact that can index `ClusterGrid` carries the VB-P1k capacity bound, and
/// every one that cannot carries none. This is the artifact-level tripwire: removing the
/// `cluster_count <= grid_capacity` term from any of the four sources takes its count to 0.
#[test]
fn every_cluster_grid_consumer_carries_the_capacity_bound() {
    let Some(spirv_dis) = find_spirv_dis() else {
        eprintln!(
            "cluster_grid_read_bound: spirv-dis not found — SKIPPING the ClusterGrid \
             capacity-bound census on this host."
        );
        return;
    };
    let dir = shaders_dir();
    let rows = OWNED_VARIANTS
        .iter()
        .map(|v| (v.spv, v.array_lengths))
        .chain(CENSUS_ONLY.iter().copied());
    for (spv, want) in rows {
        let path = dir.join(spv);
        assert!(path.exists(), "missing committed {}", path.display());
        let got = cluster_grid_array_lengths(&disassemble(&spirv_dis, &path));
        assert_eq!(
            got, want,
            "{spv}: expected {want} `OpArrayLength` on `ClusterGrid`, got {got}. A drop to 0 on \
             a row that expects 1 means that module reads `ClusterGrid` bounded only by the LIVE \
             header dims against a BOOT-sized allocation — the VB-P1k out-of-bounds read, which \
             nothing else in this repository detects (`robustBufferAccess` is OFF and no \
             GPU-assisted validation runs). A rise to 1 on a row that expects 0 means a variant \
             grew a cluster block it is not supposed to have."
        );
    }
}

/// At least one row must be non-zero. Guards the census against the failure mode where a future
/// refactor renames the `ClusterGrid` variable and every count silently collapses to the
/// "expected 0" rows plus zero matches — which would make the assertion above vacuous.
#[test]
fn the_capacity_bound_census_is_not_vacuous() {
    let Some(spirv_dis) = find_spirv_dis() else {
        eprintln!("cluster_grid_read_bound: spirv-dis not found — SKIPPING the non-vacuity check.");
        return;
    };
    let dir = shaders_dir();
    let expected_nonzero: usize = OWNED_VARIANTS.iter().filter(|v| v.array_lengths > 0).count()
        + CENSUS_ONLY.iter().filter(|(_, n)| *n > 0).count();
    assert!(expected_nonzero > 0, "the census table itself pins no positive row");
    let observed: usize = OWNED_VARIANTS
        .iter()
        .map(|v| v.spv)
        .chain(CENSUS_ONLY.iter().map(|(s, _)| *s))
        .filter(|spv| cluster_grid_array_lengths(&disassemble(&spirv_dis, &dir.join(spv))) > 0)
        .count();
    assert_eq!(
        observed, expected_nonzero,
        "expected {expected_nonzero} artifacts to carry a `ClusterGrid` array-length bound, \
         found {observed} — if this is 0 the census selector is matching nothing (a renamed \
         variable?) and every per-row assertion above is vacuously satisfied."
    );
}
