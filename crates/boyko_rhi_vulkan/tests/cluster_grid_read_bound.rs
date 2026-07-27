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
//! 3. **The consumer census is CLOSED over the shader roots.** Items 1 and 2 read a
//!    hand-written table, and a hand-written table cannot notice a shader it was never told
//!    about: a SIXTH `ClusterGrid` consumer added tomorrow would fail nothing here, ship with no
//!    capacity bound, and index a boot-sized allocation off a live header — the exact defect the
//!    other two items exist to catch, arriving through the one door they do not watch.
//!    [`the_cluster_grid_consumer_census_is_closed`] therefore DERIVES the consumer set by
//!    walking every committed-HLSL root in the workspace at test time (see [`shader_roots`]:
//!    `boyko_rhi_vulkan/shaders` AND `boyko_render/shaders`) and asserts it EQUALS the table's
//!    source set. Today that set is five: the four readers above plus `cluster_cull.hlsl`'s write
//!    side (VB-P1j), all under this crate's root.
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

/// The remaining `ClusterGrid`-touching artifacts, as `(source, artifact, array_lengths)`. Their
/// byte identity is already gated elsewhere (`vb_froxel_spv_sync.rs`, `cluster_cull_spv_sync.rs`),
/// so only the read/write-bound census is asserted here — the point being that the census covers
/// EVERY consumer, not just the two families this file owns.
///
/// The `source` column is not decoration: together with [`OWNED_VARIANTS`]'s `hlsl` it IS the
/// enumerated consumer table that [`the_cluster_grid_consumer_census_is_closed`] compares against
/// the shader directory. Adding an artifact row without its true source, or a consumer source
/// without any artifact row, is RED there.
const CENSUS_ONLY: &[(&str, &str, usize)] = &[
    // VB: the `#ifdef FROXEL` rows carry the bound; the base rows have no cluster block.
    ("vb_resolve.comp.hlsl", "vb_resolve.comp.spv", 0),
    ("vb_resolve.comp.hlsl", "vb_resolve_froxel.comp.spv", 1),
    ("vb_shade.comp.hlsl", "vb_shade.comp.spv", 0),
    ("vb_shade.comp.hlsl", "vb_shade_tex.comp.spv", 0),
    ("vb_shade.comp.hlsl", "vb_shade_froxel.comp.spv", 1),
    ("vb_shade.comp.hlsl", "vb_shade_tex_froxel.comp.spv", 1),
    // The cull's WRITE side (VB-P1j) — the base arm reads the array length; the HIER arm is
    // bounded by D11's pushed boot capacity instead and carries none, deliberately.
    ("cluster_cull.hlsl", "cluster_cull.comp.spv", 1),
    ("cluster_cull.hlsl", "cluster_cull_hier.comp.spv", 0),
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
        .chain(CENSUS_ONLY.iter().map(|(_, spv, n)| (*spv, *n)));
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
        + CENSUS_ONLY.iter().filter(|(_, _, n)| *n > 0).count();
    assert!(expected_nonzero > 0, "the census table itself pins no positive row");
    let observed: usize = OWNED_VARIANTS
        .iter()
        .map(|v| v.spv)
        .chain(CENSUS_ONLY.iter().map(|(_, spv, _)| *spv))
        .filter(|spv| cluster_grid_array_lengths(&disassemble(&spirv_dis, &dir.join(spv))) > 0)
        .count();
    assert_eq!(
        observed, expected_nonzero,
        "expected {expected_nonzero} artifacts to carry a `ClusterGrid` array-length bound, \
         found {observed} — if this is 0 the census selector is matching nothing (a renamed \
         variable?) and every per-row assertion above is vacuously satisfied."
    );
}

// ---------------------------------------------------------------------------------------------
// The closure: the census is derived from the directory, not just transcribed into it.
// ---------------------------------------------------------------------------------------------

/// Blanks out `//` line comments and `/* */` block comments, preserving everything else verbatim
/// (newlines included, so the residue still lines up 1:1 with the source's lines).
///
/// Double-quoted string literals are passed through UNSTRIPPED, so a `//` inside `#include "…"`
/// or `#error "…"` cannot be mistaken for a comment opener and swallow the rest of the line —
/// over-stripping is the one bug class here that would go QUIET rather than loud.
///
/// The shader corpus is NOT a positive control for this routine. An earlier revision of this doc
/// claimed it was; the claim was MEASURED false. Deleting the string-literal state outright
/// (`state = S::Str` → `state = S::Code`, i.e. exactly the over-strip described above) leaves
/// every corpus test in this file GREEN, because no string literal in the five consumers contains
/// a comment marker for the `S::Str` arm to matter on. What the corpus can still catch is only the
/// extreme case: each consumer names `ClusterGrid` on several LIVE lines (measured: 4 in
/// `cluster_cull.hlsl`, 3 in each of the other four — the declaration, the `GetDimensions` call,
/// and the indexed access), so the discovered set only changes if an over-strip eats EVERY one of
/// them within the same file. A PARTIAL over-strip is invisible there.
///
/// The actual control is the fixture pair below —
/// [`strip_comments_keeps_identifiers_inside_string_literals`] and
/// [`strip_comments_drops_commented_out_mentions`] — which drives synthetic inputs through this
/// routine instead of relying on what today's shaders happen to contain. The mutation above turns
/// the first of those RED and nothing else in the file.
fn strip_comments(src: &str) -> String {
    #[derive(Clone, Copy, PartialEq)]
    enum S {
        Code,
        Line,
        Block,
        Str,
    }
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut state = S::Code;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match state {
            S::Code => {
                if c == '/' && next == Some('/') {
                    state = S::Line;
                    i += 2;
                } else if c == '/' && next == Some('*') {
                    state = S::Block;
                    i += 2;
                } else {
                    if c == '"' {
                        state = S::Str;
                    }
                    out.push(c);
                    i += 1;
                }
            }
            S::Line => {
                if c == '\n' {
                    out.push('\n');
                    state = S::Code;
                }
                i += 1;
            }
            S::Block => {
                if c == '*' && next == Some('/') {
                    // A space, not nothing: `a/*x*/b` must not fuse into the identifier `ab`.
                    out.push(' ');
                    state = S::Code;
                    i += 2;
                } else {
                    if c == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
            S::Str => {
                out.push(c);
                if c == '\\' {
                    // Escaped char cannot close the literal.
                    if let Some(n) = next {
                        out.push(n);
                        i += 1;
                    }
                } else if c == '"' {
                    state = S::Code;
                }
                i += 1;
            }
        }
    }
    out
}

/// True when `code` contains `ident` as a WHOLE identifier (neither neighbour is `[A-Za-z0-9_]`),
/// so a future `ClusterGridDebug` or `OldClusterGrid` cannot false-match.
fn names_identifier(code: &str, ident: &str) -> bool {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    code.match_indices(ident).any(|(i, _)| {
        let before_ok = code[..i].chars().next_back().is_none_or(|c| !is_word(c));
        let after_ok = code[i + ident.len()..].chars().next().is_none_or(|c| !is_word(c));
        before_ok && after_ok
    })
}

/// FIXTURE CONTROL for [`strip_comments`], over-strip direction: a comment marker inside a string
/// literal is not a comment opener, so live code following it on the same line survives.
///
/// This exists because the shader corpus cannot play this role (see [`strip_comments`]'s doc for
/// the measurement). Mutating `state = S::Str` to `state = S::Code` — deleting string-literal
/// handling — keeps the whole corpus census GREEN and turns every case below RED, which is the
/// point: the control has to be sensitive to the bug, not merely adjacent to it.
///
/// Runs unconditionally — no `dxc` / `spirv-dis`, so it cannot SKIP the way the artifact gates do.
#[test]
fn strip_comments_keeps_identifiers_inside_string_literals() {
    // `//` inside a literal. Over-strip swallows the rest of the line, taking the identifier.
    let line_marker = r#"#error "a // b" ClusterGrid"#;
    assert!(
        names_identifier(&strip_comments(line_marker), "ClusterGrid"),
        "a `//` INSIDE a string literal was treated as a comment opener and swallowed the live \
         identifier after the closing quote. Input {line_marker:?} stripped to {:?}",
        strip_comments(line_marker)
    );

    // `/*` inside a literal, never closed. Over-strip here swallows the REST OF THE FILE, not just
    // the line — the widest blast radius this bug class has, and the quietest.
    let block_marker = "#error \"a /* b\" ClusterGrid\nStructuredBuffer<uint2> Trailing;\n";
    let block_stripped = strip_comments(block_marker);
    assert!(
        names_identifier(&block_stripped, "ClusterGrid"),
        "an unterminated `/*` INSIDE a string literal opened a block comment and ate the rest of \
         the input. Stripped to {block_stripped:?}"
    );
    assert!(
        names_identifier(&block_stripped, "Trailing"),
        "an unterminated `/*` inside a string literal ate PAST the end of its line — every later \
         declaration in the file would vanish from discovery. Stripped to {block_stripped:?}"
    );

    // An escaped quote does not close the literal, so the `//` after it is still inside the string.
    // This is the only case that exercises the `S::Str` escape branch.
    let escaped = r#"#error "a \" // b" ClusterGrid"#;
    assert!(
        names_identifier(&strip_comments(escaped), "ClusterGrid"),
        "an ESCAPED quote was treated as closing the literal, so the `//` that follows opened a \
         comment. Input {escaped:?} stripped to {:?}",
        strip_comments(escaped)
    );
}

/// FIXTURE CONTROL for [`strip_comments`], under-strip direction: a mention that lives only in a
/// comment must NOT survive, since that is the rule [`discover_cluster_grid_sources`] enrols on.
/// Also pins the two structural properties the strip's doc claims — 1:1 line correspondence, and
/// no identifier fusion across a removed block comment.
#[test]
fn strip_comments_drops_commented_out_mentions() {
    for dead in [
        "// ClusterGrid[fi] = uint2(0, 0);\n",
        "/* ClusterGrid[fi] = uint2(0, 0); */\n",
        "uint x = 0; // trailing prose about ClusterGrid\n",
        "/*\n * ClusterGrid\n */\n",
    ] {
        assert!(
            !names_identifier(&strip_comments(dead), "ClusterGrid"),
            "a commented-out mention survived stripping and would enrol its file as a consumer \
             whose artifacts can carry no bound. Input {dead:?} stripped to {:?}",
            strip_comments(dead)
        );
    }

    // The mirror: prose on one line must not suppress a LIVE declaration on the next.
    let live = "// ClusterGrid is discussed here\nStructuredBuffer<uint2> ClusterGrid;\n";
    assert!(
        names_identifier(&strip_comments(live), "ClusterGrid"),
        "a line comment ate past its own newline: {:?}",
        strip_comments(live)
    );
    assert_eq!(
        strip_comments(live).lines().count(),
        live.lines().count(),
        "the residue no longer lines up 1:1 with the source, so any line-based diagnostic built on \
         it reports the wrong line"
    );

    // `a/*x*/b` must not fuse: the fused form would MANUFACTURE the identifier we search for.
    let fusable = "uint Cluster/*x*/Grid;";
    assert!(
        !names_identifier(&strip_comments(fusable), "ClusterGrid"),
        "a removed block comment fused its neighbours into `ClusterGrid`, inventing a consumer. \
         Input {fusable:?} stripped to {:?}",
        strip_comments(fusable)
    );
}

/// FIXTURE CONTROL for [`names_identifier`]: whole-identifier matching, both neighbours checked.
#[test]
fn names_identifier_matches_whole_identifiers_only() {
    assert!(names_identifier("StructuredBuffer<uint2> ClusterGrid : register(t8);", "ClusterGrid"));
    assert!(names_identifier("uint2 cell = ClusterGrid[cluster];", "ClusterGrid"));
    for near_miss in [
        "RWStructuredBuffer<uint2> ClusterGridDebug;",
        "uint x = OldClusterGrid[0];",
        "gClusterGrid_v2 = 0;",
    ] {
        assert!(
            !names_identifier(near_miss, "ClusterGrid"),
            "{near_miss:?} false-matched `ClusterGrid`; the census would enrol a file that never \
             touches the real binding"
        );
    }
}

/// The shader root this file's tables enumerate. [`Variant::hlsl`] and [`CENSUS_ONLY`]'s source
/// column are BARE file names relative to it, because it is also the directory dxc compiles in.
const OWNED_SHADER_ROOT: &str = "boyko_rhi_vulkan/shaders";

/// The other committed-HLSL root in the workspace.
const RENDER_SHADER_ROOT: &str = "boyko_render/shaders";

/// Every committed-HLSL root in the workspace, as `(label, absolute path)`.
///
/// TWO roots, not one. [`shaders_dir`] covers only this crate's, but `boyko_render/shaders` holds
/// committed `.hlsl` + `.spv` of exactly the same kind (`gpu_integrate.hlsl`, `ui_rect.*.hlsl`), so
/// a `ClusterGrid` consumer authored there would be invisible to a census whose own doc calls
/// itself CLOSED. Merely DOCUMENTING that boundary was the cheaper option and was rejected: it
/// leaves the identical hole one directory over, and the hole is the whole reason this test exists.
/// Walking it costs one extra `read_dir` at test time.
///
/// A consumer discovered under any root other than [`OWNED_SHADER_ROOT`] lands in the
/// "unenumerated" bucket by construction, since the tables cannot name it — that is the intended
/// outcome: it forces an explicit decision about which crate owns the artifact and its bound,
/// instead of letting the shader ship uncensused.
fn shader_roots() -> [(&'static str, PathBuf); 2] {
    let crates_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("invariant: this crate lives at `<workspace>/crates/boyko_rhi_vulkan`")
        .to_path_buf();
    [
        (OWNED_SHADER_ROOT, shaders_dir()),
        (RENDER_SHADER_ROOT, crates_dir.join("boyko_render").join("shaders")),
    ]
}

/// Walks every root in [`shader_roots`] and returns `(per-root file counts, consumers)` — every
/// `.hlsl`/`.hlsli` whose LIVE (comment-stripped) text names `ClusterGrid`, as `root-label/file`,
/// sorted.
///
/// # The rule for prose-only mentions
///
/// **A mention that survives comment-stripping is a consumer; a mention that does not is not.**
/// Every one of these sources discusses `ClusterGrid` in its header prose at least as often as it
/// touches it — `deferred_pbr.hlsl` names it on eight lines, MEASURED as five comment lines and
/// three live ones (the declaration, the `GetDimensions` call, the single indexed read) — and this
/// repo has precedent for a file naming a resource *only* to disclaim it (`vb_shadow_vis.comp.hlsl`
/// mentions `gVbId` solely to say it does NOT read it). Counting prose would enrol every such file
/// and the census would drown in rows that pin nothing. The same rule intentionally drops
/// COMMENTED-OUT code: a `// ClusterGrid[fi] = …` compiles to no instruction, so it can carry no
/// bound and there is nothing for the artifact census to assert about it.
///
/// The rule's blind spot is stated rather than papered over: a source that reaches `ClusterGrid`
/// through an `#include` without naming it would not be discovered. It cannot happen today (no
/// `.hlsli` under either root names `ClusterGrid`; each of the five declares its own binding), and
/// if a future refactor moves the declaration into a header, that header becomes the discovered
/// consumer and every current row goes stale — RED in both directions at once, which is the
/// correct moment to re-bless the table.
fn discover_cluster_grid_sources() -> (Vec<(&'static str, usize)>, Vec<String>) {
    let mut scanned: Vec<(&'static str, usize)> = Vec::new();
    let mut consumers: Vec<String> = Vec::new();
    for (label, dir) in shader_roots() {
        let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| {
            panic!("cannot enumerate the shader root {label} at {}: {e}", dir.display())
        });
        let mut here = 0usize;
        for entry in entries {
            let entry = entry.expect("invariant: the shader directory is readable entry by entry");
            let name = entry.file_name().to_string_lossy().into_owned();
            if !(name.ends_with(".hlsl") || name.ends_with(".hlsli")) {
                continue;
            }
            here += 1;
            let path = entry.path();
            let src = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read shader source {}: {e}", path.display()));
            if names_identifier(&strip_comments(&src), "ClusterGrid") {
                consumers.push(format!("{label}/{name}"));
            }
        }
        scanned.push((label, here));
    }
    consumers.sort();
    (scanned, consumers)
}

/// The enumerated consumer set: every distinct source named by [`OWNED_VARIANTS`] or
/// [`CENSUS_ONLY`], qualified with [`OWNED_SHADER_ROOT`] so it compares against the multi-root
/// discovery, sorted.
fn enumerated_cluster_grid_sources() -> Vec<String> {
    let mut sources: Vec<String> = OWNED_VARIANTS
        .iter()
        .map(|v| v.hlsl)
        .chain(CENSUS_ONLY.iter().map(|(hlsl, _, _)| *hlsl))
        .map(|name| format!("{OWNED_SHADER_ROOT}/{name}"))
        .collect();
    sources.sort_unstable();
    sources.dedup();
    sources
}

/// The census is CLOSED: the `ClusterGrid` consumers DISCOVERED across [`shader_roots`] are
/// exactly the ones the tables above enumerate.
///
/// Without this, the two gates above are open on the side that matters most. They walk a fixed
/// list, so they can only ever check shaders they were already told about — a sixth consumer added
/// tomorrow passes every test in this file by being invisible to it, and ships indexing a
/// boot-sized `ClusterGrid` off live header dims with `robustBufferAccess` OFF. This test is the
/// only thing in the repository that can see a shader nobody registered.
#[test]
fn the_cluster_grid_consumer_census_is_closed() {
    let (scanned, discovered) = discover_cluster_grid_sources();

    // Anti-vacuity, first, and PER ROOT: a wrong path or a botched extension filter yields an empty
    // walk, and an empty walk would satisfy every set comparison below by having nothing to
    // compare. Per root rather than in total, so a broken second root cannot hide behind a healthy
    // first one — that is the same "invisible to the gate" defect the whole test is against.
    for (label, count) in &scanned {
        assert!(
            *count > 0,
            "the shader-root walk found ZERO .hlsl/.hlsli files under `{label}` — the discovery is \
             broken (wrong path or bad extension filter), not the shaders. Every assertion in this \
             test would otherwise pass over an empty set. Per-root counts: {scanned:?}"
        );
    }
    let total: usize = scanned.iter().map(|(_, n)| *n).sum();
    assert!(
        !discovered.is_empty(),
        "scanned {total} shader sources ({scanned:?}) and found NO live `ClusterGrid` reference in \
         any of them. Either the identifier was renamed (in which case this whole file's \
         `%ClusterGrid` SPIR-V selector is dead too and the artifact census is vacuous), or \
         comment-stripping is eating live code."
    );

    let enumerated = enumerated_cluster_grid_sources();

    let unenumerated: Vec<&str> = discovered
        .iter()
        .map(String::as_str)
        .filter(|d| !enumerated.iter().any(|e| e == d))
        .collect();
    assert!(
        unenumerated.is_empty(),
        "UNENUMERATED `ClusterGrid` consumer(s): {unenumerated:?}. These shader sources name \
         `ClusterGrid` in live code but appear in neither OWNED_VARIANTS nor CENSUS_ONLY, so \
         nothing in this repository checks that their artifacts carry the VB-P1k capacity bound. \
         Add one row per committed .spv variant — to OWNED_VARIANTS if this file should also \
         byte-gate the family, to CENSUS_ONLY if another *_spv_sync test already does — with the \
         MEASURED `OpArrayLength` count for each (1 where the cluster block survives into the \
         module, 0 where it is dead-stripped or `#ifdef`-ed out). A consumer under a root OTHER \
         than `{OWNED_SHADER_ROOT}` cannot be expressed by these tables as they stand (both source \
         columns are bare names relative to that root, which is also dxc's working directory here) \
         — it needs a sibling census in the owning crate, and this failure is the forcing \
         function for that call. Enumerated today: {enumerated:?}"
    );

    let stale: Vec<&str> = enumerated
        .iter()
        .map(String::as_str)
        .filter(|e| !discovered.iter().any(|d| d == e))
        .collect();
    assert!(
        stale.is_empty(),
        "STALE census row(s): {stale:?}. The tables name these sources, but the shader directory \
         has no such file with a live `ClusterGrid` reference — the source was renamed, deleted, \
         or lost its cluster block (in which case its artifact rows pin a bound that no longer \
         exists). Discovered today: {discovered:?}"
    );
}
