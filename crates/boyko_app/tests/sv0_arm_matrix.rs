//! **The VB-SV0 rung-S4 arming gate** (`docs/VB-SV0-SDF-SHADOW-PLAN.md`, "S4 — arm", gate (ii)) —
//! the per-row, PER-TERM proof that SV0's input actually reaches every armable VB lit producer.
//!
//! # What gate (ii) is, and why it is split in two
//!
//! SV0 is TWO independently bit-gated terms — an SDF soft shadow (light-header word 7 bit 5) and a
//! 5-tap contact AO (bit 6). Rev 3 of the plan wrote ONE assertion ("SV0 armed") and never named
//! the mode value, so `sv0_mode = 3` passed on the shadow term alone: a contact-AO term wired to
//! the wrong lane, min-combined with the wrong operand, or structurally `1.0` would have sailed
//! through every rung with only the owner's eye between it and shipping. So each of the eight
//! armable rows is asserted TWICE, and neither half may lean on the other:
//!
//! * **(ii-a)** `sv0_mode = VB_SDF_MESH_SHADOW_BIT`, AO bit clear;
//! * **(ii-b)** `sv0_mode = VB_SDF_MESH_AO_BIT`, shadow bit clear.
//!
//! Each must, ON ITS OWN, differ from that row's `sv0_mode = 0` render, by a changed-pixel count
//! within [`S4_MIN_CHANGED_FRACTION`]..=[`S4_MAX_CHANGED_FRACTION`] of the row's covered mesh
//! pixels.
//!
//! # ⚠️ (ii-b) is measured THROUGH a pre-existing `min` chain (code-review P1-b)
//!
//! The AO term does not write `ao_final`; it combines into it —
//! `ao_final = min(ao_final, sdf_ao(P, n))` in all three producers. Whatever already darkened
//! `ao_final` therefore MASKS the SV0 term wherever it is the darker of the two, and the changed
//! pixel count under-reports by exactly that much, through no fault of SV0. Two pre-existing
//! terms feed that seed, and the band above was not derived with either in mind:
//!
//! * **The texture AO map** (`#if TEXTURED: float ao_final = ao_tex;`), on rows 4/6/8.
//!   **MEASURED, and it is not a factor.** The committed fixture map
//!   `assets/pbr_fixtures/synth_bumps/ao.png` is a CONSTANT `0xFF` over all 2048² texels in every
//!   channel, so `ao_tex == 1.0` exactly on every pixel of every textured row and the `min` seed
//!   is the same `1.0` the untextured rows start from. [`sv0_fixture_ao_map_has_no_floor`] pins
//!   that with the engine's own PNG decoder, so a swapped texture folder reds a test instead of
//!   silently deflating rows 4/6/8's counts. **The band needs no correction on those rows.**
//!
//! * **The SSAO gather** (`if (ssao_mode != SSAO_MODE_OFF) ao_final = min(ao_final, ssao_blurred);`),
//!   on rows 7/8 — which are the SPLIT rows, i.e. SSAO is what SELECTS them, so it is armed on
//!   both by construction. This one is a **STATED RESIDUAL**: measuring it needs a render of the
//!   split producer with SSAO off, which is not a configuration that exists (no SSAO ⇒ no split
//!   tail ⇒ a different row). The band is NOT widened for it. What the gate does instead is make
//!   a low count DIAGNOSABLE: [`Row::ao_floor`] classifies every row, the per-row line prints the
//!   class, and an (ii-b) failure on an SSAO row names the masking as the competing hypothesis
//!   and prescribes the discriminator — rows 7/8 against their floorless siblings 3/4, which the
//!   same 24-dump matrix already contains. The scene makes the residual small: five well-separated
//!   convex spheres over empty background, where a screen-space AO gather returns ~1 almost
//!   everywhere. That is an argument, not a measurement, and it is recorded here as one.
//!
//! A third attenuation applies to ALL EIGHT rows and is not a floor at all: `ao_final` reaches
//! only the AMBIENT terms (`eval_pbr_ambient_hemi` and the DDGI add), never direct light, while
//! (ii-a)'s `vis` multiplies the direct sun. So (ii-b)'s budget is structurally smaller than
//! (ii-a)'s on every row, and the two counts are not comparable to each other.
//!
//! # This asserts REACHED, never QUALITY
//!
//! A changed-pixel count is an image statistic, and image statistics lie about render quality —
//! this campaign has the scars. The count is used for exactly one claim: the term's input reached
//! this producer and moved pixels. The numerical correctness verdict is rung S3's leaf oracle, and
//! the visual verdict is the owner's eval on the dumped BMP (gate (iv)).
//!
//! # Inputs: 24 dumps + 24 logs, produced on a real GPU
//!
//! This test is CPU-only but consumes GPU output, so it is `#[ignore]`d and reads a directory
//! populated by `scripts\sv0_arm_matrix.ps1`:
//!
//! ```text
//! <dir>\row<N>_mode<M>.bmp   N in 1..=8, M in {0, 1, 2}
//! <dir>\row<N>_mode<M>.log   the run's captured stdout+stderr
//! ```
//!
//! Run it with:
//!
//! ```text
//! powershell -File scripts\sv0_arm_matrix.ps1
//! $env:RUSTUP_TOOLCHAIN='stable-x86_64-pc-windows-gnu'
//! cargo test -p boyko-app --test sv0_arm_matrix -- --ignored --nocapture
//! ```
//!
//! # Why the LOG is read and not just the pixels
//!
//! Which of the ten VB lit-producer `.spv` a run binds is decided from four inputs the test does
//! not observe (`path_vb_split` / `vb_use_classified` / `vb_tex_active` / `cluster_cull`), three of
//! them driven by env vars, a boot resolve and an asset load. "I set the env, therefore row 5 ran"
//! is a gate quantified over a row nobody verified — this campaign's signature defect. So
//! `record_vb` prints the producer it selected (`note_vb_lit_producer`,
//! `boyko_rhi_vulkan/src/present/passes/vb.rs`) and every row below asserts against that line.
//! The same log is checked for the SV0 clamp diagnostic, which is what a silently unhonoured
//! request looks like — a frame byte-identical to the unarmed one, i.e. indistinguishable from a
//! dead term.

use std::path::{Path, PathBuf};

use boyko_render::mesh::Vertex;
use boyko_scene::ViewUniform;

mod sv0_oracle;
mod sv0_scene;

use sv0_oracle::{Bmp32, Coverage, MeshSelection, OracleVertex};

// ===========================================================================================
// The band, and where it comes from
// ===========================================================================================

/// **From the plan (§S4 gate (ii)) — do not edit this literal to make a failing run pass.**
///
/// The minimum fraction of a row's covered mesh pixels an SV0 term must move on its own. Below
/// this, "the term reached this producer" is indistinguishable from decode noise on a handful of
/// pixels, and rung S1's fixture floors (`SV0_MIN_SHADOWED_PIXELS` / `SV0_MIN_AO_PIXELS`) were
/// themselves DERIVED from this number — they are `2×` this band over this raster, so a fixture
/// that clears S1 clears this with margin by construction.
const S4_MIN_CHANGED_FRACTION: f64 = 0.01;

/// **From the plan (§S4 gate (ii)) — do not edit this literal to make a failing run pass.**
///
/// The maximum fraction. An upper bound exists because "most of the frame moved" is not evidence
/// that the SV0 term reached — it is evidence that something ELSE changed between the two runs
/// (a different producer bound, a different scene, a stray env knob). The gate is two-sided so
/// that failure mode reports itself instead of reading as an emphatic pass.
const S4_MAX_CHANGED_FRACTION: f64 = 0.60;

/// The env var naming the directory `scripts\sv0_arm_matrix.ps1` wrote its dumps into.
const MATRIX_DIR_ENV: &str = "BOYKO_SV0_MATRIX_DIR";
/// The default dump directory, matching the script's own default.
const DEFAULT_MATRIX_DIR: &str = r"D:\tmp\sv0";

/// The prefix `record_vb` prints the selected lit producer under.
const PRODUCER_MARKER: &str = "boyko_rhi_vulkan: VB lit producer = ";
/// The prefix `vb_both_sdf_tex` prints its resolved texture folder under — the provenance of the
/// AO map [`sv0_fixture_ao_map_has_no_floor`] measures (code-review P1-b).
const TEXTURE_DIR_MARKER: &str = "vb_both_sdf_tex: reading textures from ";
/// The prefix `sync_sv0_light_gate` prints when it could not honour an SV0 request. Its presence
/// in an armed run's log means the frame rendered UNARMED — the failure whose symptom is an
/// unchanged image.
const CLAMP_MARKER: &str = "boyko_render: VB-SV0 was requested";

// ===========================================================================================
// The variant matrix
// ===========================================================================================

/// What already darkens `ao_final` on a row BEFORE SV0's `min` combines into it — the
/// attenuation gate (ii-b) is measured through (code-review P1-b, and this module's header).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AoFloor {
    /// `ao_final` starts at a literal `1.0` and nothing else touches it: the SV0 term's effect on
    /// this row is unattenuated, and the band applies as written.
    None,
    /// `ao_final` starts at `ao_tex`. MEASURED to be exactly `1.0` on the committed fixture map
    /// (see [`sv0_fixture_ao_map_has_no_floor`]), i.e. equivalent to [`AoFloor::None`] in effect
    /// — carried as its own class so a texture-folder swap changes the diagnosis rather than
    /// silently changing the numbers.
    Texture,
    /// The SSAO gather's `min` runs before SV0's. Unmeasurable without a configuration that does
    /// not exist; the STATED RESIDUAL.
    Ssao,
    /// Both of the above.
    TextureAndSsao,
}

impl AoFloor {
    /// Whether this class can mask the SV0 AO term by an amount the matrix cannot measure.
    ///
    /// [`AoFloor::Texture`] is deliberately NOT such a class: its floor is measured, and pinned.
    const fn is_unmeasured(self) -> bool {
        matches!(self, Self::Ssao | Self::TextureAndSsao)
    }

    /// The short label the per-row line prints.
    const fn label(self) -> &'static str {
        match self {
            Self::None => "-",
            Self::Texture => "tex(=1.0)",
            Self::Ssao => "SSAO",
            Self::TextureAndSsao => "SSAO+tex(=1.0)",
        }
    }
}

/// One row of the plan's §S4 variant matrix: an armable VB lit producer, the fixture that can
/// reach it, and the knobs that select it.
struct Row {
    /// 1-based row number, matching the plan's table and the dump filenames.
    index: u32,
    /// The `.spv` stem `record_vb` must report for this row.
    producer: &'static str,
    /// The `#[ignore]`d dump test that renders it (`cargo test --test <fixture>`).
    fixture: &'static str,
    /// The env knobs beyond `BOYKO_SV0_MODE` that select this row — documentation for the failure
    /// message, kept beside the row so a mismatch can be diagnosed without opening the script.
    knobs: &'static str,
    /// What already darkens this row's `ao_final` before SV0's `min` (code-review P1-b).
    ao_floor: AoFloor,
    /// The row whose (ii-b) count is this one's unattenuated reference — the discriminator an
    /// (ii-b) failure on a floored row is diagnosed against. `None` on rows that carry no
    /// unmeasured floor and therefore need no reference.
    ao_reference_row: Option<u32>,
}

/// **The eight SV0-armable rows.** Rows 9 and 10 (`vb_shade_split_hwrt`,
/// `vb_shade_split_tex_hwrt`) are absent on purpose: `ShadowSources::SDF_SOFT_MARCH` requires
/// `!hwrt_denoise_or_vis_on`, which is exactly what selects those two pipelines, so SV0 can never
/// be armed while they are bound.
///
/// Their instruments are TWO, at the two ends of that claim (code-review P1-a):
///
/// * `boyko_render::render_path_config::tests::sv0_never_arms_under_hwrt` — the boot resolver can
///   never produce an SV0-armable hwrt boot. Its red mutation (delete the
///   `!consumers.hwrt_denoise_or_vis_on` term) was demonstrated at rung S4.
/// * `boyko_rhi_vulkan`'s `note_vb_lit_producer` `debug_assert!` — no FRAME binds an `_hwrt` lit
///   producer while `ResolvedRenderPathGpu::vb_sdf_mesh_armable` holds. The resolver's claim is
///   about boots; the recorder is where a pipeline is actually chosen, and nothing but this
///   assertion joins the two.
const ROWS: [Row; 8] = [
    Row {
        index: 1,
        producer: "vb_resolve",
        fixture: "vb_both_sdf",
        knobs: "(none)",
        ao_floor: AoFloor::None,
        ao_reference_row: None,
    },
    Row {
        index: 2,
        producer: "vb_resolve_froxel",
        fixture: "vb_both_sdf",
        knobs: "BOYKO_SV0_FROXEL=1",
        ao_floor: AoFloor::None,
        ao_reference_row: None,
    },
    Row {
        index: 3,
        producer: "vb_shade",
        fixture: "vb_both_sdf",
        knobs: "BOYKO_VB_FORCE_CLASSIFIED=1",
        ao_floor: AoFloor::None,
        ao_reference_row: None,
    },
    Row {
        index: 4,
        producer: "vb_shade_tex",
        fixture: "vb_both_sdf_tex",
        knobs: "(textured, auto)",
        ao_floor: AoFloor::Texture,
        ao_reference_row: None,
    },
    Row {
        index: 5,
        producer: "vb_shade_froxel",
        fixture: "vb_both_sdf",
        knobs: "BOYKO_VB_FORCE_CLASSIFIED=1 + BOYKO_SV0_FROXEL=1",
        ao_floor: AoFloor::None,
        ao_reference_row: None,
    },
    Row {
        index: 6,
        producer: "vb_shade_tex_froxel",
        fixture: "vb_both_sdf_tex",
        knobs: "BOYKO_SV0_FROXEL=1 (textured, auto)",
        ao_floor: AoFloor::Texture,
        ao_reference_row: None,
    },
    // Rows 7/8 are the SPLIT rows, and SSAO is what selects them — so the SSAO `min` always runs
    // ahead of SV0's here. Row 3 (`vb_shade`, untextured, no SSAO) and row 4 (`vb_shade_tex`, the
    // measured-1.0 texture floor) are their floorless references.
    Row {
        index: 7,
        producer: "vb_shade_split",
        fixture: "vb_both_sdf",
        knobs: "BOYKO_SV0_SSAO=1",
        ao_floor: AoFloor::Ssao,
        ao_reference_row: Some(3),
    },
    Row {
        index: 8,
        producer: "vb_shade_split_tex",
        fixture: "vb_both_sdf_tex",
        knobs: "BOYKO_SV0_SSAO=1 (textured, auto)",
        ao_floor: AoFloor::TextureAndSsao,
        ao_reference_row: Some(4),
    },
];

/// The three `BOYKO_SV0_MODE` values every row is rendered at: unarmed, shadow-only, AO-only.
/// The numbering IS the shader's `sv0_mode` (bit 0 shadow, bit 1 contact AO).
const MODE_UNARMED: u32 = 0;
const MODE_SHADOW_ONLY: u32 = 1;
const MODE_AO_ONLY: u32 = 2;

// ===========================================================================================
// The shared denominator
// ===========================================================================================

/// The dump directory for this run.
fn matrix_dir() -> PathBuf {
    std::env::var(MATRIX_DIR_ENV).map_or_else(|_| PathBuf::from(DEFAULT_MATRIX_DIR), PathBuf::from)
}

/// The fixtures' projection, from the engine's own construction site — verbatim
/// `sv0_adequacy.rs::scene_view_proj_rows`, and for the same reason: the oracle must place pixels
/// where the VB fixtures actually draw them, not where a re-derived matrix would.
fn scene_view_proj_rows() -> [[f32; 4]; 4] {
    let view = ViewUniform::from_camera(
        sv0_scene::camera_transform().to_affine(),
        sv0_scene::camera_projection(),
    );
    boyko_render::forward_view_proj_rows(&view, sv0_scene::DUMP_EXTENT, sv0_scene::DUMP_EXTENT)
}

/// Rasterizes the fixtures' five-sphere row exactly as they spawn it.
fn scene_coverage() -> Coverage {
    let (verts, idx) = sv0_scene::scene_sphere_mesh();
    let oracle_verts: Vec<OracleVertex> = verts
        .iter()
        .map(|v: &Vertex| OracleVertex { position: v.position, normal: v.normal })
        .collect();
    let instances: Vec<[f32; 3]> =
        (0..sv0_scene::MESH_ROW_COUNT).map(sv0_scene::mesh_center).collect();

    sv0_oracle::rasterize(
        &oracle_verts,
        &idx,
        &instances,
        scene_view_proj_rows(),
        sv0_scene::DUMP_EXTENT,
        sv0_scene::DUMP_EXTENT,
        sv0_scene::CAMERA_NEAR,
    )
}

/// **The denominator gate (ii) divides by**: the mesh pixels SV0 can shade, under the edit list
/// the rendered scene actually GATHERS ([`sv0_scene::gathered_edits`], the same construction rung
/// S1's gates quantify over).
///
/// Gathered rather than reconstructed for review C2's reason: if this rebuilt its own edit list,
/// deleting the body from `sv0_scene::spawn_scene` would leave this gate measuring a body the
/// frame no longer contains.
fn mesh_selection(coverage: &Coverage) -> MeshSelection {
    let edits = sv0_scene::gathered_edits();
    assert!(!edits.is_empty(), "the fixture scene must gather a non-empty SDF edit list");
    sv0_oracle::select_mesh_pixels(coverage, &edits, sv0_scene::CAMERA_EYE)
}

// ===========================================================================================
// Reading one cell of the matrix
// ===========================================================================================

/// The dump path for one (row, mode) cell.
fn dump_path(dir: &Path, row: u32, mode: u32) -> PathBuf {
    dir.join(format!("row{row}_mode{mode}.bmp"))
}

/// The captured-log path for one (row, mode) cell.
fn log_path(dir: &Path, row: u32, mode: u32) -> PathBuf {
    dir.join(format!("row{row}_mode{mode}.log"))
}

/// Reads one cell's BMP, or fails with the command that produces it.
fn read_dump(dir: &Path, row: &Row, mode: u32) -> Bmp32 {
    let path = dump_path(dir, row.index, mode);
    sv0_oracle::read_bmp32(&path).unwrap_or_else(|e| {
        panic!(
            "row {} ({}) mode {mode}: {e}\n  produce the whole matrix with \
             `powershell -File scripts\\sv0_arm_matrix.ps1`\n  this row's knobs: {} (fixture {})",
            row.index, row.producer, row.knobs, row.fixture
        )
    })
}

/// What one cell's captured run log tells the gate.
struct CellLog {
    /// The lit producer the run selected — the LAST [`PRODUCER_MARKER`] line.
    ///
    /// LAST, not first: `vb_use_classified` is a per-frame decision, so a textured boot can spend
    /// its opening frames on the untextured producer while the bindless slots are still being
    /// filled. The steady state is what rendered the dumped frame.
    producer: String,
    /// The texture folder a TEXTURED fixture reported loading from, when it printed one — the
    /// provenance of the AO map [`sv0_fixture_ao_map_has_no_floor`] measures (code-review P1-b).
    texture_dir: Option<String>,
}

/// The last value following `marker` in `text`, trimmed — or `None` when the marker never
/// appears.
fn last_marked_value(text: &str, marker: &str) -> Option<String> {
    text.rmatch_indices(marker).next().map(|(at, _)| {
        let tail = &text[at + marker.len()..];
        tail.lines().next().unwrap_or("").trim().to_string()
    })
}

/// Reads one cell's captured run log and extracts everything the gate reads from it.
fn read_log(dir: &Path, row: &Row, mode: u32) -> CellLog {
    let path = log_path(dir, row.index, mode);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "row {} ({}) mode {mode}: cannot read {} ({e}) — the matrix script captures each run's \
             output beside its dump; re-run `scripts\\sv0_arm_matrix.ps1`",
            row.index,
            row.producer,
            path.display()
        )
    });
    // The clamp diagnostic is checked here rather than in a separate pass because its whole
    // danger is that it produces an image nothing else can distinguish from a working unarmed
    // one — reading the log is the ONLY place the difference exists.
    assert!(
        !text.contains(CLAMP_MARKER) || mode == MODE_UNARMED,
        "row {} ({}) mode {mode}: the boot could not honour the SV0 request — \
         `sync_sv0_light_gate` clamped it, so this dump is UNARMED and every count taken from it \
         is meaningless. Log: {}",
        row.index,
        row.producer,
        path.display()
    );
    let producer = last_marked_value(&text, PRODUCER_MARKER).unwrap_or_else(|| {
        panic!(
            "row {} ({}) mode {mode}: no `{PRODUCER_MARKER}` line in {} — the run never recorded a \
             VB frame, so no row was exercised at all. (The instrument is \
             `debug_assertions`-only: a `--release` render carries no such line by design.)",
            row.index,
            row.producer,
            path.display()
        )
    });
    CellLog { producer, texture_dir: last_marked_value(&text, TEXTURE_DIR_MARKER) }
}

// ===========================================================================================
// Dump provenance — the `-Rows` partial-rerun hole (code-review P2-e)
// ===========================================================================================

/// The sidecar `scripts\sv0_arm_matrix.ps1` writes beside each cell's dump.
const META_SUFFIX: &str = "meta";

/// The sidecar's first line — a format marker, so a `.meta` from somewhere else is rejected
/// rather than parsed into a plausible-looking provenance record.
const META_MARKER: &str = "sv0-matrix-cell 1";

/// One cell's provenance record, parsed from its `.meta` sidecar.
///
/// # Why the gate needs one at all
///
/// `golden.ps1` solved single-artifact staleness by deleting the target `.bmp` and asserting the
/// file was freshly written, and the matrix script copies that per cell. But this gate reads
/// TWENTY-FOUR cells, and the script's own `-Rows` parameter — which the plan's mutation workflow
/// prescribes ("re-render only the two split rows after reverting `vb_shade_split.comp.hlsl`") —
/// refreshes a SUBSET. Every refreshed cell is individually fresh and every untouched cell is
/// individually old, so per-cell freshness is silent about the matrix as a whole: the other
/// eighteen dumps can predate the very edit under test, and the gate reads their staleness as
/// evidence that eighteen rows are fine.
///
/// The stamp that closes it is the test EXECUTABLE's content hash. A cell's image is a
/// deterministic function of the binary that rendered it and the env it ran under, and the script
/// rebuilds both fixture binaries at the top of every invocation — so "all cells that ran binary
/// `B` carry the same hash for `B`" is exactly "no cell of `B` predates the current build". Cells
/// of the OTHER binary need no cross-check: a source change that could move one binary's output
/// necessarily rebuilds it, and a change confined to the other fixture's own `tests/*.rs` cannot
/// reach this one's pixels.
struct CellProvenance {
    /// The row this sidecar claims to describe — cross-checked against the filename, so a
    /// hand-copied `.meta` is caught rather than believed.
    row: u32,
    /// The `BOYKO_SV0_MODE` this sidecar claims.
    mode: u32,
    /// The `--test` binary that rendered the cell.
    binary: String,
    /// SHA-256 of that binary's executable, as the script observed it.
    exe_sha256: String,
    /// The script's ISO-8601 UTC timestamp for the cell — reported, never asserted on (clock
    /// comparisons across runs prove nothing the hash does not).
    run_utc: String,
}

/// The `.meta` path for one (row, mode) cell.
fn meta_path(dir: &Path, row: u32, mode: u32) -> PathBuf {
    dir.join(format!("row{row}_mode{mode}.{META_SUFFIX}"))
}

/// Reads one cell's provenance sidecar, or fails naming the script that writes it.
fn read_provenance(dir: &Path, row: &Row, mode: u32) -> CellProvenance {
    let path = meta_path(dir, row.index, mode);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "row {} ({}) mode {mode}: cannot read the provenance sidecar {} ({e}) — every cell \
             carries one as of rung S4's review; re-run `scripts\\sv0_arm_matrix.ps1` (a dump \
             with no sidecar predates the provenance check and cannot be certified)",
            row.index,
            row.producer,
            path.display()
        )
    });
    // PowerShell 5.1's `-Encoding utf8` emits a BOM; `str::lines()` hands it back on line 1, where
    // it would defeat every `strip_prefix` below. Stripped here rather than worked around at each
    // parse site.
    let text = text.trim_start_matches('\u{feff}');
    assert!(
        text.starts_with(META_MARKER),
        "the sidecar {} does not start with `{META_MARKER}` — it is not an SV0 matrix cell record",
        path.display()
    );
    let field = |key: &str| -> String {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")))
            .unwrap_or_else(|| {
                panic!("row {} mode {mode}: `{key}` missing from {}", row.index, path.display())
            })
            .trim()
            .to_string()
    };
    let parse_u32 = |key: &str| -> u32 {
        let raw = field(key);
        raw.parse().unwrap_or_else(|_| {
            panic!("row {} mode {mode}: `{key}={raw}` is not a number", row.index)
        })
    };
    let prov = CellProvenance {
        row: parse_u32("row"),
        mode: parse_u32("mode"),
        binary: field("binary"),
        exe_sha256: field("exe_sha256"),
        run_utc: field("run_utc"),
    };
    // The sidecar must describe the cell whose NAME it carries — otherwise a copied or renamed
    // file would launder one cell's provenance onto another.
    assert_eq!(
        (prov.row, prov.mode),
        (row.index, mode),
        "the sidecar {} describes row {} mode {} — it does not belong to this cell",
        path.display(),
        prov.row,
        prov.mode
    );
    assert_eq!(
        prov.binary,
        row.fixture,
        "row {} mode {mode}: rendered by `{}`, but this row's fixture is `{}`",
        row.index,
        prov.binary,
        row.fixture
    );
    prov
}

// ===========================================================================================
// The pre-existing AO floor, measured (code-review P1-b)
// ===========================================================================================

/// The committed fixture AO map — the texture whose values seed `ao_final` on the textured rows,
/// and therefore the pre-existing floor gate (ii-b) is measured through there.
///
/// Compiled in relative to this crate's manifest, the SAME construction `vb_both_sdf_tex.rs`
/// resolves its default texture folder with.
const FIXTURE_AO_MAP: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/pbr_fixtures/synth_bumps/ao.png");

/// **The measurement that removes the texture half of the (ii-b) attenuation** (code-review
/// P1-b): the fixture's AO map is a constant maximum, so `ao_tex == 1.0` on every pixel and the
/// textured rows' `min(ao_final, sdf_ao)` starts from the same `1.0` the untextured rows do.
///
/// Without this pin the claim is an assumption about a binary asset, and a folder swap — the
/// fixture takes one from `BOYKO_PBR_TEXTURE_DIR` — would deflate rows 4/6/8's counts with no
/// indication of why. With it, the swap reds HERE, naming the cause, instead of surfacing as a
/// mysteriously thin AO term three rows away.
///
/// GPU-free and NOT `#[ignore]`d: it runs in the ordinary suite, so the claim is checked on every
/// `cargo test` rather than only when someone produces the 24-dump matrix.
#[test]
fn sv0_fixture_ao_map_has_no_floor() {
    let bytes = std::fs::read(FIXTURE_AO_MAP)
        .unwrap_or_else(|e| panic!("cannot read the fixture AO map {FIXTURE_AO_MAP}: {e}"));
    let img = boyko_image::decode_png(&bytes)
        .unwrap_or_else(|e| panic!("cannot decode {FIXTURE_AO_MAP}: {e:?}"));
    assert_eq!(img.bit_depth, 8, "the fixture AO map is an 8-bit PNG");

    // `decode_png` always expands to RGBA; the shader samples `.r`, so that is the lane the floor
    // lives in. Scanning it is ~4 M byte compares — trivial next to reading the file.
    let min_r = img
        .pixels
        .chunks_exact(4)
        .map(|texel| texel[0])
        .min()
        .expect("invariant: a decoded PNG has at least one texel");
    assert_eq!(
        min_r, u8::MAX,
        "the fixture AO map's darkest red texel is {min_r}/255, so `ao_tex < 1.0` somewhere and \
         the SV0 contact-AO term is MASKED wherever that happens. Rows 4/6/8 of the arming matrix \
         then under-report through no fault of SV0, and their [1%, 60%] band was not derived for \
         an attenuated chain. Either restore a floorless AO map or re-derive the band for those \
         rows — do NOT widen it to make them pass."
    );
}

// ===========================================================================================
// The gate
// ===========================================================================================

/// **Gate (ii), all eight armable rows, both terms.**
///
/// For each row: the run really bound that row's producer (from its log), and each of the two SV0
/// terms moves a fraction of the row's covered mesh pixels inside the band, ON ITS OWN.
///
/// # The mutations this gate is required to go red under (plan §S4, DEMONSTRATED at that rung)
///
/// * revert `vb_resolve.comp.hlsl`'s SV0 block → rows 1-2 red, 3-8 green;
/// * revert `vb_shade.comp.hlsl`'s → rows 3-6 red;
/// * revert `vb_shade_split.comp.hlsl`'s → rows 7-8 red;
/// * force `sv0_mode = 0` host-side → every row's counts fall to 0;
/// * delete the `min` into `vis` → every row's (ii-a) falls to 0 while (ii-b) survives;
/// * delete the `min` into `ao_final` → the converse.
///
/// The last pair is what makes (ii-a) and (ii-b) two assertions rather than one wearing two names.
///
/// # What it does NOT claim
///
/// The (ii-b) counts on rows 7/8 are measured through the SSAO `min` — see this module's header
/// for that stated residual, and [`AoFloor`] for the per-row classification the failure messages
/// use. The texture half of the same concern is measured and pinned by
/// [`sv0_fixture_ao_map_has_no_floor`].
#[test]
#[ignore = "consumes the 24 GPU dumps `scripts\\sv0_arm_matrix.ps1` produces; the orchestrator runs the script first"]
fn sv0_each_armable_row_moves_pixels_under_each_term_alone() {
    let dir = matrix_dir();
    let coverage = scene_coverage();
    let selection = mesh_selection(&coverage);
    assert!(
        !selection.is_empty(),
        "the covered-mesh selection is empty — every fraction below would be a vacuous 0.0"
    );
    println!(
        "SV0 S4(ii): denominator = {} covered mesh pixels ({} taken by the SDF leg), band \
         [{:.0}%, {:.0}%]",
        selection.len(),
        selection.sdf_occluded,
        S4_MIN_CHANGED_FRACTION * 100.0,
        S4_MAX_CHANGED_FRACTION * 100.0
    );

    // (-1) PROVENANCE, before a single pixel is read (code-review P2-e). Every cell rendered by a
    //      given fixture binary must carry that binary's SAME content hash, or the matrix is a
    //      mixture of builds and its green means nothing. Checked up front so a partially-stale
    //      matrix reports THAT, instead of surfacing as an arbitrary row's count moving.
    let mut stamps: Vec<(&'static str, String, u32, u32, String)> = Vec::new();
    for row in &ROWS {
        for mode in [MODE_UNARMED, MODE_SHADOW_ONLY, MODE_AO_ONLY] {
            let p = read_provenance(&dir, row, mode);
            stamps.push((row.fixture, p.exe_sha256, p.row, p.mode, p.run_utc));
        }
    }
    for fixture in ["vb_both_sdf", "vb_both_sdf_tex"] {
        let mut seen: Vec<&str> = stamps
            .iter()
            .filter(|(f, ..)| *f == fixture)
            .map(|(_, h, ..)| h.as_str())
            .collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            seen.len(),
            1,
            "the cells rendered by `{fixture}` came from {} DIFFERENT builds of that test binary \
             ({seen:?}) — this matrix is a mixture of builds, so every row that happens to pass \
             may be passing on a dump that predates the change under test. This is what a \
             partial `-Rows` re-run leaves behind: re-render the whole matrix with \
             `powershell -File scripts\\sv0_arm_matrix.ps1`. Cells: {:?}",
            seen.len(),
            stamps
                .iter()
                .filter(|(f, ..)| *f == fixture)
                .map(|(_, h, r, m, t)| format!("row{r}_mode{m} {t} {}", &h[..8.min(h.len())]))
                .collect::<Vec<_>>()
        );
        println!("SV0 S4(ii): {fixture} exe = {}", seen[0]);
    }

    // (ii-b) reference counts, keyed by row index — filled as the loop goes so a floored row's
    // failure message can quote its floorless sibling's number. `ROWS` is in ascending index
    // order and every `ao_reference_row` points BACKWARD, so the reference is always already in.
    let mut ao_fraction_by_row: Vec<(u32, f64)> = Vec::new();

    for row in &ROWS {
        // (0) The row is the row. Asserted for all three modes: a mode that silently fell back to
        //     a different producer would otherwise be compared against the intended one's dump.
        for mode in [MODE_UNARMED, MODE_SHADOW_ONLY, MODE_AO_ONLY] {
            let log = read_log(&dir, row, mode);
            assert_eq!(
                log.producer, row.producer,
                "row {} mode {mode}: expected the `{}` lit producer but the run bound `{}` — \
                 this row's knobs ({}) did not select it, so every count below would be \
                 quantified over the WRONG variant",
                row.index, row.producer, log.producer, row.knobs
            );
            // A textured row's AO floor is only measured for the folder it actually loaded
            // (code-review P1-b): `sv0_fixture_ao_map_has_no_floor` reads the COMMITTED map, so a
            // run driven at a different folder through `BOYKO_PBR_TEXTURE_DIR` would be certified
            // against a texture it never sampled.
            if let Some(loaded) = log.texture_dir.as_deref() {
                let expected = Path::new(FIXTURE_AO_MAP)
                    .parent()
                    .expect("invariant: FIXTURE_AO_MAP names a file inside a folder");
                assert_eq!(
                    Path::new(loaded),
                    expected,
                    "row {} mode {mode} loaded its textures from `{loaded}`, but the AO floor this \
                     gate reasons about was measured on `{}` — the (ii-b) counts on this row are \
                     not quantified over the map that rendered it",
                    row.index,
                    expected.display()
                );
            }
        }

        let unarmed = read_dump(&dir, row, MODE_UNARMED);
        let shadow_only = read_dump(&dir, row, MODE_SHADOW_ONLY);
        let ao_only = read_dump(&dir, row, MODE_AO_ONLY);

        // (ii-a) the shadow term, alone.
        let a = sv0_oracle::changed_covered_pixels(&selection, &unarmed, &shadow_only)
            .unwrap_or_else(|e| panic!("row {} (ii-a): {e}", row.index));
        // (ii-b) the contact-AO term, alone.
        let b = sv0_oracle::changed_covered_pixels(&selection, &unarmed, &ao_only)
            .unwrap_or_else(|e| panic!("row {} (ii-b): {e}", row.index));

        // The AO floor is printed on EVERY row, pass or fail: a reader comparing rows must be
        // able to see which (ii-b) numbers were measured through a `min` chain that could mask
        // the term, without going back to the source (code-review P1-b).
        println!(
            "  row {} {:<22} (ii-a) {:>6} px = {:>5.2}%   (ii-b) {:>6} px = {:>5.2}%   \
             ao_floor={}",
            row.index,
            row.producer,
            a.changed,
            a.fraction() * 100.0,
            b.changed,
            b.fraction() * 100.0,
            row.ao_floor.label()
        );

        assert!(
            a.fraction() >= S4_MIN_CHANGED_FRACTION,
            "row {} ({}) gate (ii-a): the SDF soft shadow moved only {} of {} covered mesh pixels \
             ({:.3}%), under the {:.0}% floor — with the AO bit CLEAR, so the shadow term did not \
             reach this producer",
            row.index,
            row.producer,
            a.changed,
            a.covered,
            a.fraction() * 100.0,
            S4_MIN_CHANGED_FRACTION * 100.0
        );
        assert!(
            a.fraction() <= S4_MAX_CHANGED_FRACTION,
            "row {} ({}) gate (ii-a): {:.3}% of covered mesh pixels moved, over the {:.0}% ceiling \
             — something other than the SV0 shadow term differs between these two runs",
            row.index,
            row.producer,
            a.fraction() * 100.0,
            S4_MAX_CHANGED_FRACTION * 100.0
        );
        // ⚠️ The (ii-b) floor is measured THROUGH this row's pre-existing `min` chain. On an
        // `AoFloor::None` / `AoFloor::Texture` row that chain is a measured no-op, so a count
        // under the floor means the term did not reach. On an SSAO row it does not, and the
        // message has to say so rather than let a masked term read as a dead one. The BAND IS THE
        // SAME either way — the diagnosis differs, not the threshold (code-review P1-b).
        let ao_diagnosis = if row.ao_floor.is_unmeasured() {
            let reference = row.ao_reference_row.and_then(|r| {
                ao_fraction_by_row.iter().find(|(i, _)| *i == r).map(|(i, f)| (*i, *f))
            });
            let reference = reference.map_or_else(
                || String::from("(no reference row measured)"),
                |(i, f)| format!("row {i} (same term, NO SSAO floor) moved {:.3}%", f * 100.0),
            );
            format!(
                "⚠️ this row's `ao_final` is ALSO min-ed with the SSAO gather BEFORE SV0's own \
                 `min`, so a masked term and a dead term produce the same count here. Discriminate \
                 against the reference: {reference}. Comparable ⇒ SV0 reached and SSAO is not \
                 masking; near-zero HERE while the reference is healthy ⇒ either the split \
                 producer's SV0 block is dead or SSAO is darker than `sdf_ao` over the whole \
                 selection (inspect the dumps). Do NOT widen the band to make this row pass"
            )
        } else {
            format!(
                "this row's `ao_final` carries no floor that could mask the term (ao_floor={}, \
                 pinned by `sv0_fixture_ao_map_has_no_floor`), so a count under the band means the \
                 AO term did not reach this producer",
                row.ao_floor.label()
            )
        };
        assert!(
            b.fraction() >= S4_MIN_CHANGED_FRACTION,
            "row {} ({}) gate (ii-b): the contact AO moved only {} of {} covered mesh pixels \
             ({:.3}%), under the {:.0}% floor, with the SHADOW bit CLEAR. {ao_diagnosis}",
            row.index,
            row.producer,
            b.changed,
            b.covered,
            b.fraction() * 100.0,
            S4_MIN_CHANGED_FRACTION * 100.0
        );
        assert!(
            b.fraction() <= S4_MAX_CHANGED_FRACTION,
            "row {} ({}) gate (ii-b): {:.3}% of covered mesh pixels moved, over the {:.0}% ceiling \
             — something other than the SV0 contact-AO term differs between these two runs",
            row.index,
            row.producer,
            b.fraction() * 100.0,
            S4_MAX_CHANGED_FRACTION * 100.0
        );

        // The two terms are not one write wearing two bit names. The plan closes this with a pair
        // of `min`-deletion mutations; this assertion catches the same defect without a GPU
        // re-run, because a shadow-only and an AO-only render of the same scene that agree
        // pixel-for-pixel can only mean both bits drove the SAME code.
        let cross = sv0_oracle::changed_covered_pixels(&selection, &shadow_only, &ao_only)
            .unwrap_or_else(|e| panic!("row {} (ii-a vs ii-b): {e}", row.index));
        assert!(
            cross.changed > 0,
            "row {} ({}): the shadow-only and AO-only renders are identical over the selection — \
             the two gate bits are driving the same term",
            row.index,
            row.producer
        );

        ao_fraction_by_row.push((row.index, b.fraction()));
    }
}

/// The matrix's own vacuity guard: the row table covers the eight armable variants exactly once
/// each, and names no `_hwrt` row.
///
/// Cheap, GPU-free, and it runs in the ordinary suite — so a row silently dropped from [`ROWS`]
/// (which would make the gate above green by covering less) is caught without the 24-dump run.
#[test]
fn sv0_row_table_covers_the_eight_armable_variants_exactly() {
    assert_eq!(ROWS.len(), 8, "the plan's matrix has exactly 8 SV0-armable rows");
    for (i, row) in ROWS.iter().enumerate() {
        assert_eq!(row.index as usize, i + 1, "rows must be listed in plan order");
        assert!(
            !row.producer.contains("hwrt"),
            "row {} names an `_hwrt` producer, which is structurally unarmable — its instrument is \
             the CPU truth table `sv0_never_arms_under_hwrt`, not this matrix",
            row.index
        );
    }
    // Code-review P1-b: the (ii-b) diagnosis is only as good as this classification. Every row
    // that carries an UNMEASURED floor must name a reference row, that reference must itself be
    // floorless, and it must come EARLIER — the gate fills its reference table as it walks `ROWS`
    // in order, so a forward reference would silently degrade to "(no reference row measured)".
    for row in &ROWS {
        match row.ao_reference_row {
            None => assert!(
                !row.ao_floor.is_unmeasured(),
                "row {} carries the unmeasured floor {:?} but names no reference row — its (ii-b) \
                 failure could not be told from a dead term",
                row.index,
                row.ao_floor
            ),
            Some(reference) => {
                assert!(
                    row.ao_floor.is_unmeasured(),
                    "row {} names a reference row but carries no unmeasured floor",
                    row.index
                );
                assert!(reference < row.index, "row {}'s reference must come earlier", row.index);
                let r = ROWS
                    .iter()
                    .find(|r| r.index == reference)
                    .unwrap_or_else(|| panic!("row {}'s reference row {reference} does not exist", row.index));
                assert!(
                    !r.ao_floor.is_unmeasured(),
                    "row {}'s reference row {reference} carries the unmeasured floor {:?} — a \
                     reference that can be masked the same way discriminates nothing",
                    row.index,
                    r.ao_floor
                );
            }
        }
    }
    // The SPLIT rows are exactly the SSAO-floored ones: SSAO is what arms `mesh_geo_shade_split`,
    // so a split row without an SSAO floor (or a fused row with one) is a misclassification that
    // would send an (ii-b) failure to the wrong diagnosis.
    for row in &ROWS {
        assert_eq!(
            row.producer.starts_with("vb_shade_split"),
            matches!(row.ao_floor, AoFloor::Ssao | AoFloor::TextureAndSsao),
            "row {} ({}): the SSAO floor and the split producer must coincide",
            row.index,
            row.producer
        );
        assert_eq!(
            row.producer.contains("_tex"),
            matches!(row.ao_floor, AoFloor::Texture | AoFloor::TextureAndSsao),
            "row {} ({}): the texture floor and the `_tex` producer must coincide",
            row.index,
            row.producer
        );
    }
    for (i, a) in ROWS.iter().enumerate() {
        for b in &ROWS[i + 1..] {
            assert_ne!(a.producer, b.producer, "each lit producer gets exactly one row");
        }
    }
    // The three modes are the shader's own bit values, not an independent numbering — a drift
    // here would render the wrong image under the right filename.
    assert_eq!(MODE_UNARMED, 0);
    assert_eq!(MODE_SHADOW_ONLY, boyko_render::VB_SDF_MESH_SHADOW_BIT);
    assert_eq!(MODE_AO_ONLY, boyko_render::VB_SDF_MESH_AO_BIT);
}
