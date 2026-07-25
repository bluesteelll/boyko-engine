//! Lighting L1 host-oracle (CPU-only — NO GPU required).
//!
//! Proves the L1 clustered froxel-cull RESOLVE path the GPU golden compares against:
//! - the cluster linearization `(x,y,z) <-> index` round-trips and agrees host/shader
//!   (`golden_cluster_index`, the ONE linearization both cull-write + resolve-read use);
//! - the exp-Z slice math (`golden_cluster_z_slice`) inverts the slice distribution
//!   `view_z(k) = near*(far/near)^(k/dim_z)`;
//! - the host sphere-vs-AABB cull (`golden_cluster_cull`) keeps every light whose sphere
//!   intersects a froxel's world AABB and drops the rest (no false drop under the cap);
//! - **the load-bearing L1 golden**: the clustered resolve
//!   (`golden_deferred_resolve_clustered`) produces the SAME packed color as the
//!   brute-force `golden_deferred_resolve_table` for a multi-light scene (the cull is exact
//!   for the test lights), and the L1-OFF header path is byte-identical to L0b.
//!
//! It also carries the provenance of `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` §1.3's published
//! occupancy table (VB-P1e rung HP): `cluster_cull_occupancy_profile_matches_the_published_table`
//! and `cluster_cull_rejection_ratio_at_n512_matches_the_headline_claim` drive the same
//! `golden_cluster_cull` oracle above with the VB-P1d bench camera / light rig
//! (`crates/boyko_app/tests/vb_p1d_cull_shade_bench.rs`) to pin the measured froxel occupancy
//! the hierarchical-cull design (§2 onward) is built on, replacing a session-ephemeral scratch
//! probe that was never committed to the repository.
//!
//! This file boots NO Vulkan context — it is the non-GPU gate the developer runs (the GPU
//! golden runs separately on the 3060).

use std::collections::BTreeSet;

use boyko_rhi_vulkan::compute::{composite_pixel_ray, CompositeCamera, SDF_IMG_H, SDF_IMG_W};
use boyko_rhi_vulkan::goldens::{golden_cluster_cull, golden_cluster_cull_hier, golden_cluster_index, golden_cluster_xy_tile, golden_cluster_z_slice, golden_deferred_resolve_clustered, golden_deferred_resolve_table, golden_froxel_aabb, golden_hier_groups_per_slice, golden_hier_thread_map, GoldenClusterConfig, GoldenLight, GoldenLightHeader, GoldenMaterial, MarcherAttributes, HIER_GROUP_THREADS};

/// The ortho ray-gen the resolve uses: `ro=(0,0,2)`, `rd=(0,0,-1)`, so `view_z == view_t`
/// and `P = ro + rd * view_t = (u*HE, v*HE, 2 - view_t)`.
const RO: [f32; 3] = [0.0, 0.0, 2.0];
const RD: [f32; 3] = [0.0, 0.0, -1.0];

/// A cluster config whose exp-Z near/far span the ortho scene's view-z (= ray param `t`):
/// surfaces sit near world z = 0 (camera at z = 2), so `t ≈ 2`. near=0.25, far=4.0 keeps the
/// froxel slices concentrated over the scene's depth band.
fn cfg() -> GoldenClusterConfig {
    GoldenClusterConfig {
        dim_x: 16,
        dim_y: 9,
        dim_z: 24,
        max_lights_per_cluster: 256,
        z_near: 0.25,
        z_far: 4.0,
    }
}

fn materials() -> Vec<GoldenMaterial> {
    vec![GoldenMaterial::default()]
}

/// A lit (mask == 1) attribute with a chosen `view_t` and an oct-encoded ~+Z normal.
fn lit_attrs(view_t: f32) -> MarcherAttributes {
    MarcherAttributes {
        base_rgb: [200, 200, 200],
        oct_rg: [128, 128], // oct (0.5, 0.5) -> +Z normal
        mat_id: 0,
        shadow: 255,
        ao: 255,
        mask: 1,
        view_t,
    }
}

/// The bench's square render target (`vb_p1d_cull_shade_bench.rs`'s own `CameraRig`, aspect 1.0).
const IMG: u32 = 512;

/// `boyko_render::light::INDEX_LIST_CAP`. The vulkan crate cannot depend on `boyko_render`
/// (see [`GoldenClusterConfig`]'s own doc comment), so this mirrors the constant rather than
/// importing it.
const INDEX_LIST_CAP: u32 = 16384;

/// The bench rig's own `DEFAULT_N_PS` (`vb_p1d_cull_shade_bench.rs:68`) — [`light_position`]'s
/// volume-scale factor is relative to this fixed baseline, not to the swept `n_ps`.
const BENCH_DEFAULT_N_PS: u32 = 14;

/// The published §1.3 table, pinned exactly: `(n_ps, total_indices, non_empty_froxels,
/// max_per_froxel)`. These are MEASURED values, not derived — a mismatch means the host oracle
/// or the bench rig changed since the table was published, which is exactly what this test
/// exists to catch. Do not edit these literals to make a failing run pass.
const PUBLISHED_TABLE: [(u32, usize, usize, usize); 8] = [
    (8, 789, 514, 3),
    (14, 1239, 543, 5),
    (32, 1916, 557, 10),
    (64, 2063, 364, 15),
    (128, 1654, 143, 24),
    (256, 2072, 115, 40),
    (512, 2597, 85, 64),
    (1024, 2709, 55, 109),
];

/// The M2 config's (16x9x24 PERSPECTIVE, VB-P1d bench camera) absolute `HierCullStats` pair
/// counts on the bench Kronecker rig (`lights_for`), pinned exactly: `(N_ps, pairs_coarse,
/// pairs_fine, pairs_hier)`. `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` §7's break-even table has
/// had its only recurring defect, across five consecutive design revisions, in exactly one place:
/// a human hand-copying these measured integers into prose while the numbers moved underneath
/// them as the host oracle and the bench rig evolved. §7's table is a TRANSCRIPTION of the
/// printed deliverable this test emits (`hier_cull_matches_flat_cull_exactly_across_the_grid_matrix`'s
/// `eprintln!` table) — this array is the READOUT that transcription must be checked against, not
/// a re-derivation from it. These are MEASURED values, not derived — do not edit these literals to
/// make a failing run pass; a mismatch means the host oracle or the bench rig changed since the
/// table was measured, which is exactly what this pin exists to catch.
const M2_HIER_PAIR_COUNT_TABLE: [(u32, u64, u64, u64); 8] = [
    (1, 24, 288, 312),
    (8, 192, 3456, 3648),
    (16, 384, 6768, 7152),
    (20, 480, 8208, 8688),
    (64, 1536, 20304, 21840),
    (128, 3072, 18720, 21792),
    (512, 12288, 33552, 45840),
    (1022, 24528, 54000, 78528),
];

/// `ClusterConfig::default()` mirrored as a [`GoldenClusterConfig`]: 16x9x24 = 3456 froxels,
/// `MAX_LIGHTS_PER_CLUSTER` 256, `z_near` 0.1, `z_far` 50.0. Distinct from this file's own
/// [`cfg`] (ortho-tuned, `z_near` 0.25 / `z_far` 4.0): this fixture is the VB-P1d bench's
/// PERSPECTIVE scene and must not be merged with the resolve fixtures above.
fn vb_p1d_bench_cluster_cfg() -> GoldenClusterConfig {
    GoldenClusterConfig {
        dim_x: 16,
        dim_y: 9,
        dim_z: 24,
        max_lights_per_cluster: 256,
        z_near: 0.1,
        z_far: 50.0,
    }
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / l, v[1] / l, v[2] / l]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// The VB-P1d bench camera (`vb_p1d_cull_shade_bench.rs:235-254`): eye `(0, 1.1, 7.8)` looking
/// at `(0, 0.55, 0)`, `fov_y` 52 degrees, aspect 1.0.
fn camera() -> CompositeCamera {
    let eye = [0.0, 1.1, 7.8];
    let fwd = norm([0.0 - eye[0], 0.55 - eye[1], 0.0 - eye[2]]);
    let right = norm(cross(fwd, [0.0, 1.0, 0.0]));
    let up = cross(right, fwd);
    CompositeCamera::Perspective {
        eye,
        forward: fwd,
        right,
        up,
        tan_half_fov: (52.0_f32 * core::f32::consts::PI / 180.0 / 2.0).tan(),
        aspect: 1.0,
    }
}

/// Verbatim mirror of `vb_p1d_cull_shade_bench.rs::light_position` (`:124-137`).
fn light_position(i: u32, n: u32) -> [f32; 3] {
    let scale = (f64::from(n) / f64::from(BENCH_DEFAULT_N_PS)).max(1.0).cbrt() as f32;
    let half_x = 4.5 * scale;
    let y_min = 0.3;
    let y_span = 3.3 * scale;
    let z_min = -2.0 * scale;
    let z_span = 6.0 * scale;

    let t = f64::from(i);
    let fx = (t * 0.618_033_988_75).fract() as f32;
    let fy = (t * 0.381_966_011_25).fract() as f32;
    let fz = (t * 0.236_067_977_5).fract() as f32;
    [(fx * 2.0 - 1.0) * half_x, y_min + fy * y_span, z_min + fz * z_span]
}

/// Verbatim mirror of `vb_p1d_cull_shade_bench.rs::light_range` (`:142-144`).
fn light_range(i: u32) -> f32 {
    1.2 + ((f64::from(i) * 0.142_857).fract() as f32) * 0.8
}

/// Appends `n_ps` point/spot lights placed by [`light_position`]/[`light_range`] to `lights`,
/// every 4th (`i % 4 == 3`) a spot aimed straight down, the rest points. `position_n` is
/// [`light_position`]'s volume-scale `n` argument, threaded separately from `n_ps` (the light
/// COUNT / index-sequence driver) so callers can pin the placement volume independently of how
/// many lights are placed — see [`lights_for_in_frustum`].
fn push_point_spot_lights(lights: &mut Vec<GoldenLight>, n_ps: u32, position_n: u32) {
    for i in 0..n_ps {
        let p = light_position(i, position_n);
        let r = light_range(i);
        if i % 4 == 3 {
            lights.push(GoldenLight::spot(p, [0.0, -1.0, 0.0], [1.0, 1.0, 1.0], 65.0, r, 15.0, 30.0));
        } else {
            lights.push(GoldenLight::point(p, [1.0, 1.0, 1.0], 65.0, r));
        }
    }
}

/// Builds the bench's light table for `n_ps` point/spot lights: 2 global `l0a` lights (a
/// directional + a sky, matching `setup`'s own sun + sky count — [`golden_cluster_cull`] never
/// inspects `l0a`-indexed lights, so only their COUNT matters here, not their values), followed
/// by `n_ps` point/spot lights ([`push_point_spot_lights`]), the "bench Kronecker rig" — its
/// placement volume grows with `n_ps` (§1.4's defect), so lights disperse OUT of the camera
/// frustum as `n_ps` grows.
fn lights_for(n_ps: u32) -> Vec<GoldenLight> {
    let mut lights = vec![
        GoldenLight::directional([-0.35, -0.85, -0.4], [1.0, 0.96, 0.9], 4.0),
        GoldenLight::directional([0.0, -1.0, 0.0], [0.38, 0.44, 0.55], 0.0),
    ];
    push_point_spot_lights(&mut lights, n_ps, n_ps);
    lights
}

/// The "dense in-frustum" rig (§1.4, §8.6 H1 assertion 5's ABORT-clause pairing with the bench
/// Kronecker rig): IDENTICAL placement formula to [`light_position`], but [`push_point_spot_lights`]
/// is called with the volume-scale `n` argument PINNED at [`BENCH_DEFAULT_N_PS`] regardless of
/// `n_ps` — the bench rig's own stated VOLUME-GROWTH defect (§1.4: the placement volume grows as
/// `cbrt(n_ps/14)`, pushing lights out of the view frustum as `n_ps` grows, collapsing
/// `non_empty_froxels` 514 -> 55 over the published table) does not apply here BY CONSTRUCTION,
/// so froxel occupancy stays dense as `n_ps` grows instead of collapsing.
///
/// **This fixes ONLY the volume-growth defect, NOT §1.4's COLLINEARITY defect (W2, code
/// review — the previous doc comment's "does not apply here BY CONSTRUCTION" OVERCLAIMED).**
/// [`light_position`]'s Kronecker multipliers still satisfy `g + g^2 == 1` and
/// `g^3 == g - g^2` exactly, so `fy = 1 - fx` and `fz = frac(2*fx)` hold here IDENTICALLY —
/// pinning `n` changes only the placement BOX's size (`half_x`/`y_span`/`z_span`), never the
/// multipliers, so every light still lies on the SAME two straight 3-D segments (§1.4). A 1-D
/// locus is the geometry MOST favourable to a group-level reject (an axis-aligned coarse box
/// encloses a line segment far more tightly than a genuine 3-D volume fill), so this rig's
/// selectivity numbers are still a best case, not a worst case. [`lights_for_3d`] is the
/// genuinely 3-D equidistributed rig.
fn lights_for_in_frustum(n_ps: u32) -> Vec<GoldenLight> {
    let mut lights = vec![
        GoldenLight::directional([-0.35, -0.85, -0.4], [1.0, 0.96, 0.9], 4.0),
        GoldenLight::directional([0.0, -1.0, 0.0], [0.38, 0.44, 0.55], 0.0),
    ];
    push_point_spot_lights(&mut lights, n_ps, BENCH_DEFAULT_N_PS);
    lights
}

/// The plastic-constant per-axis multipliers ([`light_position_3d`]'s W2 fix): `phi3` is the
/// positive real root of `x^4 = x + 1` (Roberts' `R_3` low-discrepancy sequence generator),
/// `phi3 = 1.220744084605760`. `1/phi3`, `1/phi3^2`, `1/phi3^3` carry no small-integer linear
/// relation among themselves (unlike [`light_position`]'s golden-ratio triple, where
/// `g + g^2 == 1` exactly), so the per-axis fractional sequences are genuinely independent.
const PLASTIC_PHI3: f64 = 1.220_744_084_605_76;

/// A genuinely 3-D low-discrepancy placement (W2, code review): identical shape to
/// [`light_position`] (same placement box, same `scale`/`half_x`/`y_span`/`z_span` formula) but
/// driven by [`PLASTIC_PHI3`]'s three independent per-axis multipliers instead of the Kronecker
/// triple `g, g^2, g^3` — which collapses every light onto two straight 3-D segments (§1.4's
/// collinearity defect, `fy = 1 - fx`, `fz = frac(2*fx)`). No such identity holds for
/// `1/phi3, 1/phi3^2, 1/phi3^3`, so this sequence genuinely fills the placement VOLUME rather
/// than a 1-D locus.
fn light_position_3d(i: u32, n: u32) -> [f32; 3] {
    let a1 = 1.0 / PLASTIC_PHI3;
    let a2 = 1.0 / (PLASTIC_PHI3 * PLASTIC_PHI3);
    let a3 = 1.0 / (PLASTIC_PHI3 * PLASTIC_PHI3 * PLASTIC_PHI3);

    let scale = (f64::from(n) / f64::from(BENCH_DEFAULT_N_PS)).max(1.0).cbrt() as f32;
    let half_x = 4.5 * scale;
    let y_min = 0.3;
    let y_span = 3.3 * scale;
    let z_min = -2.0 * scale;
    let z_span = 6.0 * scale;

    let t = f64::from(i);
    let fx = (t * a1).fract() as f32;
    let fy = (t * a2).fract() as f32;
    let fz = (t * a3).fract() as f32;
    [(fx * 2.0 - 1.0) * half_x, y_min + fy * y_span, z_min + fz * z_span]
}

/// [`push_point_spot_lights`]'s counterpart driven by [`light_position_3d`] instead of
/// [`light_position`] — otherwise identical (same spot/point ratio, same [`light_range`]).
fn push_point_spot_lights_3d(lights: &mut Vec<GoldenLight>, n_ps: u32, position_n: u32) {
    for i in 0..n_ps {
        let p = light_position_3d(i, position_n);
        let r = light_range(i);
        if i % 4 == 3 {
            lights.push(GoldenLight::spot(p, [0.0, -1.0, 0.0], [1.0, 1.0, 1.0], 65.0, r, 15.0, 30.0));
        } else {
            lights.push(GoldenLight::point(p, [1.0, 1.0, 1.0], 65.0, r));
        }
    }
}

/// The genuinely 3-D rig (W2): [`lights_for`]'s counterpart built from [`light_position_3d`]
/// instead of the collinear [`light_position`], with the volume-scale `n` argument threaded the
/// SAME way `lights_for` threads it (grows with `n_ps`, §1.4's volume-growth behaviour intact —
/// only the collinearity defect is fixed here).
fn lights_for_3d(n_ps: u32) -> Vec<GoldenLight> {
    let mut lights = vec![
        GoldenLight::directional([-0.35, -0.85, -0.4], [1.0, 0.96, 0.9], 4.0),
        GoldenLight::directional([0.0, -1.0, 0.0], [0.38, 0.44, 0.55], 0.0),
    ];
    push_point_spot_lights_3d(&mut lights, n_ps, n_ps);
    lights
}

/// `n_ps` point/spot lights with NO `l0a` prefix (`l0a_count == 0`) — the light-table shape
/// §8.6 H1 assertion 6's mask-capacity-boundary config needs (`point_spot_count == MAX_LIGHTS`
/// with `l0a_count == 0` exercises mask word 31 / bit 1023, which every `l0a_count > 0` config
/// leaves dark since the point/spot span is clamped to `MAX_LIGHTS` by the host fold, D6).
fn lights_point_spot_only(n_ps: u32) -> Vec<GoldenLight> {
    let mut lights = Vec::with_capacity(n_ps as usize);
    push_point_spot_lights(&mut lights, n_ps, n_ps);
    lights
}

/// Runs the host cull oracle for `n_ps` point/spot lights at the VB-P1d bench rig / fixture
/// above, returning `(total_indices, non_empty_froxels, max_per_froxel)`.
fn occupancy_at(n_ps: u32) -> (usize, usize, usize) {
    let c = vb_p1d_bench_cluster_cfg();
    let cam = camera();
    let lights = lights_for(n_ps);
    let header = GoldenLightHeader::new(2, n_ps, 1.0);
    let grid = golden_cluster_cull(IMG, IMG, cam, &c, &header, &lights, None);
    let total: usize = grid.iter().map(Vec::len).sum();
    let non_empty = grid.iter().filter(|cell| !cell.is_empty()).count();
    let max_per_froxel = grid.iter().map(Vec::len).max().unwrap_or(0);
    (total, non_empty, max_per_froxel)
}

// ============================================================================
// VB-P1e rung H1 — the hierarchical CPU oracle (§8.6 of
// docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md). NO GPU, NO shader — falsifies the design's pair-count
// premise on the CPU before a single line of the `-D HIER=1` shader arm is written.
// ============================================================================

/// One grid config in H1's matrix (§8.6 table): all six share the SAME light-rig style,
/// differing only in `dim_x`/`dim_y`/`dim_z` — swept so `gps` (D3) covers 1 (M1/M2, the shipped
/// default), the `gps=1` boundary FROM ABOVE (E1, `dim_x*dim_y == 256` exactly), `gps=2` exact
/// (E2), `gps=2` RAGGED with a non-empty guard tail (E3 — the only `gps>=2` config with `G>0`),
/// and `gps=3` exact (E4).
struct HierMatrixCase {
    /// Human-readable label for assertion failure messages and the selectivity printout.
    name: &'static str,
    cfg: GoldenClusterConfig,
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    /// The saturation cap this config's rig is checked against — [`INDEX_LIST_CAP`] for the
    /// shipped M1/M2 dims, `cluster_count() * 8` for E1..E4 (the `sdf_gbuffer_hybrid.rs:5230`
    /// idiom, so a larger froxel count never binds the cap).
    cap: u32,
    /// Whether the `<= 1/8` selectivity gate (§8.6 assertion 5) applies to this config. The
    /// Kronecker/in-frustum rigs' placement box (`light_position`, `4.5 x 3.3 x 6.0` world
    /// units) is calibrated to the PERSPECTIVE VB-P1d bench camera (M2/E1..E4). M1 is the
    /// ORTHO 64x64 `l1_cluster_config` fixture, whose world extent is `SDF_HALF_EXTENT = 1`
    /// (a completely different scale) — applying the perspective-calibrated box there is a
    /// RIG mismatch, not a design failure, so M1 is excluded from the numeric gate and reported
    /// for information only.
    selectivity_gated: bool,
}

/// H1's six-entry grid matrix (§8.6): M1 (the ORTHO 64x64 `l1_cluster_config` fixture,
/// `sdf_gbuffer_hybrid.rs:5215`, mirrored here by [`cfg`]), M2 (the PERSPECTIVE VB-P1d bench
/// camera, [`vb_p1d_bench_cluster_cfg`]/[`camera`]), and E1..E4 (the same PERSPECTIVE bench
/// world at swept `dim_x`/`dim_y` — E2/E3/E4 are the `gps >= 2` entries the default 16x9x24 grid
/// cannot exercise at all, since `16*9 = 144 < 256` collapses the map to the degenerate
/// `slice = gid; s = lane` form).
fn hier_matrix_cases() -> [HierMatrixCase; 6] {
    let bench_cam = camera();
    let m2 = vb_p1d_bench_cluster_cfg();
    let e1 = GoldenClusterConfig { dim_x: 16, dim_y: 16, dim_z: 24, ..m2 };
    let e2 = GoldenClusterConfig { dim_x: 32, dim_y: 16, dim_z: 24, ..m2 };
    let e3 = GoldenClusterConfig { dim_x: 16, dim_y: 17, dim_z: 24, ..m2 };
    let e4 = GoldenClusterConfig { dim_x: 32, dim_y: 24, dim_z: 24, ..m2 };
    [
        HierMatrixCase {
            name: "M1 16x9x24 ORTHO",
            cfg: cfg(),
            img_w: SDF_IMG_W,
            img_h: SDF_IMG_H,
            camera: CompositeCamera::Ortho,
            cap: INDEX_LIST_CAP,
            selectivity_gated: false,
        },
        HierMatrixCase {
            name: "M2 16x9x24 PERSPECTIVE (bench)",
            cfg: m2,
            img_w: IMG,
            img_h: IMG,
            camera: bench_cam,
            cap: INDEX_LIST_CAP,
            selectivity_gated: true,
        },
        HierMatrixCase {
            name: "E1 16x16x24 gps=1-from-above",
            cfg: e1,
            img_w: IMG,
            img_h: IMG,
            camera: bench_cam,
            cap: e1.cluster_count() * 8,
            selectivity_gated: true,
        },
        HierMatrixCase {
            name: "E2 32x16x24 gps=2-exact",
            cfg: e2,
            img_w: IMG,
            img_h: IMG,
            camera: bench_cam,
            cap: e2.cluster_count() * 8,
            selectivity_gated: true,
        },
        HierMatrixCase {
            name: "E3 16x17x24 gps=2-ragged",
            cfg: e3,
            img_w: IMG,
            img_h: IMG,
            camera: bench_cam,
            cap: e3.cluster_count() * 8,
            selectivity_gated: true,
        },
        HierMatrixCase {
            name: "E4 32x24x24 gps=3-exact",
            cfg: e4,
            img_w: IMG,
            img_h: IMG,
            camera: bench_cam,
            cap: e4.cluster_count() * 8,
            selectivity_gated: true,
        },
    ]
}

/// H1's own N sweep, TRIMMED for the 6-config test suite's runtime. §8.6 crosses the matrix
/// with `N` in `{0,1,8,64,128,512,1024}`; this keeps `0` (the vacuous/empty path), `1`, `8`/`64`
/// (bracketing the break-even), `16`/`20` (the break-even itself), the selectivity threshold
/// (`128`), the headline point (`512`) and a near-mask-capacity point (`1022`). This is a scope
/// trade-off against the plan's full 7-value x 6-config cross product, stated here rather than
/// silently.
///
/// **`1` is RESTORED (P1-h, adversarial review).** This array previously dropped `1`, citing
/// `cluster_cull_occupancy_profile_matches_the_published_table` as covering it "at full
/// fidelity" — that citation was WRONG on two independent counts: that test's own
/// `occupancy_at` helper drives ONLY the flat oracle (`golden_cluster_cull`), never
/// `golden_cluster_cull_hier`, and its own `PUBLISHED_TABLE` starts at `N_ps=8`, never testing
/// `N_ps=1` at all. `N=1` therefore had ZERO coverage of the hierarchical mirror before this fix.
///
/// **`64` is RESTORED (W1, code review).** §7's break-even (`N ~ 17-19`) is interpolated between
/// this array's `N=8` and `N=64` fine-pair counts (§7's own "Recovering a break-even number
/// requires an `N=64` row" note); dropping it made that interpolation unrecoverable from the
/// shipped test, so §7's own deliverable table could not be reproduced from the artifact. It costs
/// one more `(config, N)` cell per matrix config (the 6-config suite's runtime stays sub-second).
///
/// **`16` and `20` are ADDED (the plan's §7 table, adversarial review).** §7's break-even crosses
/// between `N=8` and `N=64` at `N ~ 16.7` (an interpolation, previously the only way to recover it
/// from the shipped artifact); these two points bracket the crossing directly, so the break-even is
/// readable off this sweep's own printed table without interpolating between two far-apart rows.
///
/// **`1022`, not `1024`.** [`lights_for`] always prepends 2 `l0a` lights, so `N_ps=1024` here
/// would give `light_count = 1026 > MAX_LIGHTS (1024)` — a MALFORMED header per D6's own
/// invariant ("`fold_light_table_slotted` clamps `light_count <= MAX_LIGHTS` in ALL build
/// profiles"), on which flat (unclamped, D7) and hier (D7-clamped to `ps_room`) are EXPECTED to
/// diverge by design, not a bug. `1022` is the largest `N_ps` that keeps `light_count ==
/// MAX_LIGHTS` exactly with this rig's `l0a_count == 2`, so it is itself a `ps_n == ps_room`
/// boundary point. The literal `N_ps=1024`, well-formed `l0a_count == 0` case is exercised
/// separately (and correctly) by `hier_cull_matches_flat_cull_at_the_mask_capacity_boundary`
/// (assertion 6).
const HIER_MATRIX_N: [u32; 9] = [0, 1, 8, 16, 20, 64, 128, 512, 1022];

/// Verifies §8.6 H1 assertion 2's PERMUTATION property (not merely a cover, which is exactly
/// the check that catches P0-1 on the CPU): the multiset of `fi` produced by every `valid`
/// `(group_id, lane)` pair under [`golden_hier_thread_map`] is EXACTLY `[0, capacity)` — no
/// duplicate, no gap, no index `>= capacity` — and separately, the number of `valid` pairs
/// equals `capacity` (the clause §8.2(B1) consumes to turn device totality into device
/// exactly-once). Pure arithmetic — no camera, no lights, no GPU — so it is run once per grid
/// config (dims-only) rather than once per `(config, N)` pair.
fn assert_hier_map_is_a_permutation(dim_x: u32, dim_y: u32, dim_z: u32, capacity: u32, what: &str) {
    let gps = golden_hier_groups_per_slice(dim_x, dim_y);
    let groups = gps * dim_z;
    let mut seen = vec![false; capacity as usize];
    let mut valid_count = 0_u32;
    for group_id in 0..groups {
        for lane in 0..HIER_GROUP_THREADS {
            let (_, _, _, fi, valid) =
                golden_hier_thread_map(group_id, lane, dim_x, dim_y, dim_z, capacity);
            if !valid {
                continue;
            }
            valid_count += 1;
            let idx = fi as usize;
            assert!(idx < capacity as usize, "{what}: valid lane produced fi={fi} >= capacity={capacity}");
            assert!(!seen[idx], "{what}: duplicate write to fi={fi} — the map is not injective");
            seen[idx] = true;
        }
    }
    assert_eq!(valid_count, capacity, "{what}: valid-lane count {valid_count} != capacity {capacity}");
    assert!(seen.iter().all(|&s| s), "{what}: at least one fi in [0,{capacity}) was never produced (a gap)");
}

/// Verifies §8.6 H1 assertion 4. Defence in depth only (§5 Case B already handles a non-finite
/// AABB correctly on device): this documents that the RIGS themselves never feed the substitution
/// a non-finite own-AABB on the well-formed matrix, it does not protect the shader.
fn assert_all_froxel_aabbs_finite(
    img_w: u32,
    img_h: u32,
    camera: CompositeCamera,
    cfg: &GoldenClusterConfig,
    label: &str,
) {
    for y in 0..cfg.dim_y {
        for x in 0..cfg.dim_x {
            for z in 0..cfg.dim_z {
                let (mn, mx) = golden_froxel_aabb(x, y, z, img_w, img_h, camera, cfg);
                for c in mn.iter().chain(mx.iter()) {
                    assert!(
                        c.is_finite(),
                        "{label}: froxel ({x},{y},{z}) AABB has a non-finite bound: \
                         min={mn:?} max={mx:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn cluster_index_is_a_bijection_with_z_innermost() {
    let c = cfg();
    // Z innermost: incrementing z by 1 increments the index by 1.
    assert_eq!(golden_cluster_index(0, 0, 0, c.dim_x, c.dim_z), 0);
    assert_eq!(golden_cluster_index(0, 0, 1, c.dim_x, c.dim_z), 1);
    assert_eq!(golden_cluster_index(1, 0, 0, c.dim_x, c.dim_z), c.dim_z);
    assert_eq!(golden_cluster_index(0, 1, 0, c.dim_x, c.dim_z), c.dim_x * c.dim_z);
    assert_eq!(
        golden_cluster_index(c.dim_x - 1, c.dim_y - 1, c.dim_z - 1, c.dim_x, c.dim_z),
        c.cluster_count() - 1
    );
    // Bijection: every froxel maps to a distinct index in [0, COUNT).
    let mut seen = vec![false; c.cluster_count() as usize];
    for y in 0..c.dim_y {
        for x in 0..c.dim_x {
            for z in 0..c.dim_z {
                let idx = golden_cluster_index(x, y, z, c.dim_x, c.dim_z) as usize;
                assert!(!seen[idx], "linearization collision at ({x},{y},{z})");
                seen[idx] = true;
            }
        }
    }
    assert!(seen.iter().all(|&s| s));
}

#[test]
fn exp_z_slice_inverts_the_distribution() {
    let c = cfg();
    let scale = c.z_scale();
    let bias = c.z_bias();
    // The boundary view-z at slice k maps back to slice k (round-trip).
    for k in 0..c.dim_z {
        let view_z = c.z_near * (c.z_far / c.z_near).powf(k as f32 / c.dim_z as f32);
        // Use the midpoint of slice [k, k+1) so the floor lands on k unambiguously.
        let view_z_mid = c.z_near * (c.z_far / c.z_near).powf((k as f32 + 0.5) / c.dim_z as f32);
        assert_eq!(golden_cluster_z_slice(view_z_mid, &c), k, "slice {k} midpoint");
        let _ = (view_z, scale, bias);
    }
    // Below near clamps to slice 0; above far clamps to the last slice.
    assert_eq!(golden_cluster_z_slice(c.z_near * 0.5, &c), 0);
    assert_eq!(golden_cluster_z_slice(c.z_far * 2.0, &c), c.dim_z - 1);
    // A non-positive (sentinel/behind) view-z clamps to slice 0.
    assert_eq!(golden_cluster_z_slice(0.0, &c), 0);
    assert_eq!(golden_cluster_z_slice(-1.0, &c), 0);
}

#[test]
fn cull_keeps_an_in_range_light_and_drops_an_out_of_range_one() {
    let c = cfg();
    // One point light at world (0,0,0) (center of the ortho view), range 3.0 — its sphere
    // intersects froxels along the central column. A second far-away point at (100,100,0)
    // range 0.1 — its sphere intersects NO froxel.
    let header = GoldenLightHeader::new_clustered(0, 2, 1.0, &c);
    let lights = vec![
        GoldenLight::point([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 100.0, 3.0),
        GoldenLight::point([100.0, 100.0, 0.0], [1.0, 1.0, 1.0], 100.0, 0.1),
    ];
    let grid = golden_cluster_cull(SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, &c, &header, &lights, None);
    assert_eq!(grid.len(), c.cluster_count() as usize);

    // Light 0 (in range) must appear in at least one froxel; light 1 (far) in none.
    let mut light0_seen = 0u32;
    let mut light1_seen = 0u32;
    for cell in &grid {
        for &i in cell {
            if i == 0 {
                light0_seen += 1;
            }
            if i == 1 {
                light1_seen += 1;
            }
        }
    }
    assert!(light0_seen > 0, "an in-range point light must land in at least one froxel");
    assert_eq!(light1_seen, 0, "an out-of-range point light must be in NO froxel");
}

#[test]
fn cull_directional_and_sky_are_global_never_in_a_froxel() {
    // The no-`P` front block (directionals + sky) is GLOBAL: the cull never appends those
    // indices to any froxel (the resolve always loops them outside the cluster path).
    let c = cfg();
    let header = GoldenLightHeader::new_clustered(2, 1, 1.0, &c);
    let lights = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0), // index 0 (l0a)
        GoldenLight::sky([0.1, 0.1, 0.12], [0.1, 0.1, 0.12]),            // index 1 (l0a)
        GoldenLight::point([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 100.0, 3.0), // index 2 (point)
    ];
    let grid = golden_cluster_cull(SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, &c, &header, &lights, None);
    for cell in &grid {
        for &i in cell {
            assert!(i >= 2, "a froxel must only carry point/spot indices (>= l0a_count), got {i}");
        }
    }
}

#[test]
fn cull_keeps_a_shadow_flagged_and_an_atlas_slotted_punctual() {
    // HOST-ORACLE mask invariant (CPU-only — does NOT exercise the GPU `cluster_cull.hlsl`).
    // `golden_cluster_cull` culls by `GoldenLight::kind()`, which masks off bit 16
    // (`casts_sdf_shadow`) and bits 17..21 (the atlas slot) before the POINT/SPOT comparison —
    // this pins that a shadow-flagged point and an atlas-slotted spot are therefore treated as
    // their BASE kind and survive the cull, landing in the froxel whose world AABB contains
    // them, EXACTLY the masking VB-P1-0 added to `cluster_cull.hlsl` (`light_kind()`, mirrored
    // 1:1 here). The GPU shader's masked-kind byte content is separately pinned by
    // `cluster_cull_spv_sync.rs`; the end-to-end "flagged lights survive on hardware" proof is
    // VB-P1b's `vb_mesh_froxel` equality golden.
    let c = cfg();
    let header = GoldenLightHeader::new_clustered(0, 2, 1.0, &c);
    let lights = vec![
        GoldenLight::point([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 100.0, 3.0).with_sdf_shadow(),
        GoldenLight::spot([0.2, 0.2, 0.5], [0.0, 0.0, 1.0], [1.0, 1.0, 0.5], 3000.0, 3.0, 20.0, 35.0)
            .with_atlas_slot(4),
    ];
    assert!(lights[0].casts_sdf_shadow(), "index 0 must carry the P6 R1 shadow flag (bit 16)");
    assert_eq!(lights[1].atlas_slot(), 4, "index 1 must carry a real atlas slot (bits 17..21)");
    assert!(lights[1].casts_sdf_shadow(), "a real atlas slot also sets the shadow flag (bit 16)");

    let grid = golden_cluster_cull(SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, &c, &header, &lights, None);
    assert_eq!(grid.len(), c.cluster_count() as usize);

    let mut shadow_flagged_point_seen = 0u32;
    let mut atlas_slotted_spot_seen = 0u32;
    for cell in &grid {
        for &i in cell {
            if i == 0 {
                shadow_flagged_point_seen += 1;
            }
            if i == 1 {
                atlas_slotted_spot_seen += 1;
            }
        }
    }
    assert!(
        shadow_flagged_point_seen > 0,
        "the shadow-flagged point must SURVIVE the host-oracle cull: `GoldenLight::kind()` masks \
         off bit 16 before the LIGHT_KIND_POINT comparison, so the flag never perturbs the kind \
         classification"
    );
    assert!(
        atlas_slotted_spot_seen > 0,
        "the atlas-slotted spot must SURVIVE the host-oracle cull: `GoldenLight::kind()` masks \
         off bits 17..21 before the LIGHT_KIND_SPOT comparison, so the slot never perturbs the \
         kind classification"
    );
}

#[test]
fn clustered_resolve_off_is_byte_identical_to_brute_force() {
    // The L1 0%-gate: a header with clusters DISABLED makes `golden_deferred_resolve_clustered`
    // delegate to the brute-force table resolve — byte-identical to L0b for every pixel.
    let c = cfg();
    let mats = materials();
    // clusters_enabled == false (plain L0 header).
    let header = GoldenLightHeader::new(1, 1, 1.0);
    let lights = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
        GoldenLight::point([0.0, 0.0, 1.5], [1.0, 1.0, 1.0], 5000.0, 10.0),
    ];
    let grid: Vec<Vec<u32>> = vec![Vec::new(); c.cluster_count() as usize]; // unused on OFF
    for &view_t in &[0.5_f32, 1.0, 1.5] {
        let attrs = lit_attrs(view_t);
        let (ro, rd) = (RO, RD);
        let want = golden_deferred_resolve_table(attrs, ro, rd, &mats, &header, &lights);
        let got = golden_deferred_resolve_clustered(
            attrs, 32, 32, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, &mats, &header, &lights, &c,
            &grid,
        );
        assert_eq!(got, want, "L1 OFF must be byte-identical to the brute-force resolve");
    }
}

#[test]
fn clustered_resolve_equals_brute_force_for_a_multi_light_scene() {
    // THE load-bearing L1 golden: the CLUSTERED resolve (looping only the pixel's froxel
    // lights) must produce the SAME image as the brute-force resolve (looping all lights) —
    // because the cull is EXACT for the test scene (no light wrongly dropped, all under the
    // cap). Tested per pixel across the whole frame.
    let c = cfg();
    let mats = materials();
    // A multi-light scene: 1 directional (global) + several point/spot spread through the
    // view. The clustered header carries the exp-Z factors.
    let header = GoldenLightHeader::new_clustered(1, 4, 1.0, &c);
    let lights = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
        // Point lights at world positions across the view (z near 0, the surface band), each
        // with a generous range so several froxels keep them.
        GoldenLight::point([-0.5, 0.3, 0.0], [1.0, 0.2, 0.2], 2000.0, 2.0),
        GoldenLight::point([0.5, -0.3, 0.0], [0.2, 1.0, 0.2], 2000.0, 2.0),
        GoldenLight::point([0.0, 0.0, 0.2], [0.2, 0.2, 1.0], 2000.0, 2.5),
        GoldenLight::spot([0.2, 0.2, 0.5], [0.0, 0.0, 1.0], [1.0, 1.0, 0.5], 3000.0, 3.0, 20.0, 35.0),
    ];

    // Build the cull grid ONCE for the whole frame (as the GPU cull pass does).
    let grid = golden_cluster_cull(SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, &c, &header, &lights, None);

    // Compare the clustered vs brute-force resolve per pixel. Since the cull is geometric +
    // exact (the froxel AABB conservatively encloses the pixel's world point) and the
    // per-light sum is table-ordered in both, the results match bit-for-bit.
    let l0b_header = GoldenLightHeader::new(1, 4, 1.0); // same lights, clusters OFF (brute force)
    let mut compared = 0u64;
    let mut lit_pixels = 0u64;
    for py in (0..SDF_IMG_H).step_by(3) {
        for px in (0..SDF_IMG_W).step_by(3) {
            let (ro, rd) = composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
            // A surface at world z ≈ 0 -> view_t ≈ 2 (camera at z = 2). Use view_t = 2.0 so
            // the reconstructed P sits in the scene's lit band where the lights live.
            let attrs = lit_attrs(2.0);
            let brute = golden_deferred_resolve_table(attrs, ro, rd, &mats, &l0b_header, &lights);
            let clustered = golden_deferred_resolve_clustered(
                attrs, px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, &mats, &header,
                &lights, &c, &grid,
            );
            assert_eq!(
                clustered, brute,
                "clustered resolve != brute force at ({px},{py}) — the cull dropped an in-range light"
            );
            compared += 1;
            if clustered != golden_deferred_resolve_table(attrs, ro, rd, &mats, &l0b_header, &[lights[0]]) {
                lit_pixels += 1;
            }
        }
    }
    assert!(compared > 0);
    assert!(lit_pixels > 0, "the multi-light scene must light at least one pixel beyond the directional");
}

#[test]
fn pixel_maps_to_a_unique_froxel_and_the_cull_set_is_a_superset_of_in_range() {
    // No false drop under the cap (the property-style L1 invariant): for a pixel mapped to
    // its froxel, EVERY light whose bounding sphere contains the pixel's reconstructed P is
    // present in that froxel's cull set (the froxel AABB encloses P, so a sphere reaching P
    // reaches the AABB). Checked against a brute scan of the point/spot block.
    let c = cfg();
    let header = GoldenLightHeader::new_clustered(0, 3, 1.0, &c);
    let lights = vec![
        GoldenLight::point([-0.3, 0.2, 0.0], [1.0, 1.0, 1.0], 100.0, 1.5),
        GoldenLight::point([0.4, -0.1, 0.1], [1.0, 1.0, 1.0], 100.0, 1.0),
        GoldenLight::point([0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 100.0, 2.5),
    ];
    let grid = golden_cluster_cull(SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho, &c, &header, &lights, None);

    for py in (0..SDF_IMG_H).step_by(7) {
        for px in (0..SDF_IMG_W).step_by(7) {
            let (ro, rd) = composite_pixel_ray(px, py, SDF_IMG_W, SDF_IMG_H, CompositeCamera::Ortho);
            let view_t = 2.0_f32; // P at world z ≈ 0
            let p = [ro[0] + rd[0] * view_t, ro[1] + rd[1] * view_t, ro[2] + rd[2] * view_t];
            let view_z = view_t; // ortho
            let (tx, ty) = golden_cluster_xy_tile(px, py, SDF_IMG_W, SDF_IMG_H, &c);
            let zsl = golden_cluster_z_slice(view_z, &c);
            let cluster = golden_cluster_index(tx, ty, zsl, c.dim_x, c.dim_z) as usize;
            let set = &grid[cluster];
            // Every light whose sphere contains P must be in the froxel's cull set.
            for (i, l) in lights.iter().enumerate() {
                let pos = [l.pos_range[0], l.pos_range[1], l.pos_range[2]];
                let r = l.pos_range[3];
                let d2 = (pos[0] - p[0]).powi(2) + (pos[1] - p[1]).powi(2) + (pos[2] - p[2]).powi(2);
                if d2 <= r * r {
                    assert!(
                        set.contains(&(i as u32)),
                        "no false drop: light {i} reaches P at ({px},{py}) but is absent from froxel {cluster}"
                    );
                }
            }
        }
    }
}

/// Pins §1.3's occupancy table exactly, and re-asserts the cap non-saturation property §6's
/// byte-identity discharge depends on.
#[test]
fn cluster_cull_occupancy_profile_matches_the_published_table() {
    let c = vb_p1d_bench_cluster_cfg();
    assert_eq!(
        c.cluster_count(),
        3456,
        "invariant: the ClusterConfig::default() mirror must stay 16x9x24 — every literal below \
         was measured against exactly this froxel count"
    );

    for &(n_ps, expected_total, expected_non_empty, expected_max) in &PUBLISHED_TABLE {
        let (total, non_empty, max_per_froxel) = occupancy_at(n_ps);

        assert_eq!(
            (total, non_empty, max_per_froxel),
            (expected_total, expected_non_empty, expected_max),
            "N_ps={n_ps}: occupancy drifted from docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md §1.3's \
             published table (total_indices/non_empty_froxels/max_per_froxel) — either the host \
             oracle (`golden_cluster_cull`) or the bench rig (camera / `light_position` / \
             `light_range`) changed since the table was measured. Do not adjust these literals to \
             match a new run; §6/§7/§10 of the plan are anchored on the published numbers"
        );

        assert!(
            total < INDEX_LIST_CAP as usize,
            "N_ps={n_ps}: total_indices ({total}) must stay under INDEX_LIST_CAP \
             ({INDEX_LIST_CAP}) — the plan's byte-identity argument for the hierarchical arm \
             depends on this: once the flat cull's global InterlockedAdd saturates the cap, claim \
             order decides which froxel loses its tail, and the flat/hierarchical arms are no \
             longer comparable byte-for-byte"
        );
        assert!(
            max_per_froxel < c.max_lights_per_cluster as usize,
            "N_ps={n_ps}: max_per_froxel ({max_per_froxel}) must stay under \
             max_lights_per_cluster ({}) — the O2 per-froxel clamp-and-drop must never trigger on \
             this rig, or the two arms diverge for the same claim-order reason as the \
             INDEX_LIST_CAP check above",
            c.max_lights_per_cluster
        );
    }
}

/// Pins §1.3's headline claim: at `N_ps=512` the cull is dominated by rejection work — under
/// 0.2 % of the `froxel_count * N_ps` pair tests actually succeed.
#[test]
fn cluster_cull_rejection_ratio_at_n512_matches_the_headline_claim() {
    let c = vb_p1d_bench_cluster_cfg();
    let n_ps = 512_u32;
    let (total, _non_empty, _max_per_froxel) = occupancy_at(n_ps);

    let pair_tests = u64::from(c.cluster_count()) * u64::from(n_ps);
    let accept_ratio = total as f64 / pair_tests as f64;

    assert!(
        accept_ratio < 0.002,
        "§1.3's headline claim: at N_ps=512 the cull performs {pair_tests} froxel x light pair \
         tests and only {total} succeed (accept_ratio={accept_ratio:.5}, must be < 0.2%) — the \
         pass is meant to be >99.8% pure rejection work, which is the entire justification for a \
         hierarchical level that rejects whole blocks of froxels against whole ranges of lights"
    );
}

/// H1's core matrix (§8.6 assertions 1, 2, 3, 4, 5): across all six grid configs and
/// [`HIER_MATRIX_N`], the hierarchical mirror ([`golden_cluster_cull_hier`]) must reproduce the
/// flat oracle ([`golden_cluster_cull`]) EXACTLY — per-froxel content AND order — stay under the
/// saturation caps, keep every froxel AABB finite, and meet the `pairs_hier / pairs_flat <= 1/8`
/// selectivity gate at `N_ps >= 128`, on the bench Kronecker rig ([`lights_for`]).
#[test]
fn hier_cull_matches_flat_cull_exactly_across_the_grid_matrix() {
    // `(name, N_ps, pairs_coarse, pairs_fine, pairs_hier, pairs_flat, selectivity)` — the
    // ABSOLUTE `HierCullStats` numbers (W1, plan §7's "the number H1 replaces with a
    // measurement"), not merely the ratio: §7's fine-column table and §8.7's `pairs_hier(512)`
    // re-derivation are recomputed from this printout, not from a derivation off the ratio.
    let mut deliverable: Vec<(&str, u32, u64, u64, u64, u64, f64)> = Vec::new();

    for case in hier_matrix_cases() {
        // Assertions 2 and 4 are dims/geometry-only — run once per config, not once per
        // `(config, N)` pair.
        assert_hier_map_is_a_permutation(
            case.cfg.dim_x,
            case.cfg.dim_y,
            case.cfg.dim_z,
            case.cfg.cluster_count(),
            case.name,
        );
        assert_all_froxel_aabbs_finite(case.img_w, case.img_h, case.camera, &case.cfg, case.name);

        for &n_ps in &HIER_MATRIX_N {
            let header = GoldenLightHeader::new(2, n_ps, 1.0);
            let lights = lights_for(n_ps);
            let flat = golden_cluster_cull(
                case.img_w, case.img_h, case.camera, &case.cfg, &header, &lights, None,
            );
            let (hier, stats) = golden_cluster_cull_hier(
                case.img_w, case.img_h, case.camera, &case.cfg, &header, &lights, None,
            );

            // Assertion 1: exact per-froxel content AND order. `Vec<Vec<u32>>` equality is
            // element-order-sensitive, so this single comparison is the whole assertion.
            assert_eq!(
                hier, flat,
                "{}: hier grid != flat grid (content or order) at N_ps={n_ps}",
                case.name
            );
            // Assertion 2's second clause (§8.2(B1)): valid-lane count == capacity.
            assert_eq!(
                stats.valid_lanes,
                case.cfg.cluster_count(),
                "{}: valid_lanes {} != capacity {} at N_ps={n_ps}",
                case.name,
                stats.valid_lanes,
                case.cfg.cluster_count()
            );

            // THE HIGH-VALUE CHANGE (adversarial review): pin M2's absolute pair counts as
            // literals ([`M2_HIER_PAIR_COUNT_TABLE`]) so §7's break-even table is a READOUT of
            // this test, never a hand-transcribed number.
            if case.name == "M2 16x9x24 PERSPECTIVE (bench)"
                && let Some(&(_, exp_coarse, exp_fine, exp_hier)) =
                    M2_HIER_PAIR_COUNT_TABLE.iter().find(|&&(table_n, _, _, _)| table_n == n_ps)
            {
                assert_eq!(
                    (stats.pairs_coarse, stats.pairs_fine, stats.pairs_hier()),
                    (exp_coarse, exp_fine, exp_hier),
                    "M2_HIER_PAIR_COUNT_TABLE drifted at N_ps={n_ps} (got pairs_coarse={}, \
                     pairs_fine={}, pairs_hier={}) -- either the host oracle \
                     (`golden_cluster_cull_hier`) or the bench rig (`lights_for`/camera) \
                     changed since these literals were measured; do not edit this literal to \
                     match a new run -- docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md §7's own table \
                     is transcribed FROM this pin",
                    stats.pairs_coarse,
                    stats.pairs_fine,
                    stats.pairs_hier()
                );
            }

            // Assertion 3: saturation non-triviality.
            let total: usize = hier.iter().map(Vec::len).sum();
            let max_per_froxel = hier.iter().map(Vec::len).max().unwrap_or(0);
            assert!(
                total < case.cap as usize,
                "{}: total_indices {total} >= cap {} at N_ps={n_ps}",
                case.name,
                case.cap
            );
            assert!(
                max_per_froxel < case.cfg.max_lights_per_cluster as usize,
                "{}: max_per_froxel {max_per_froxel} >= max_lights_per_cluster {} at N_ps={n_ps}",
                case.name,
                case.cfg.max_lights_per_cluster
            );

            // C0-2 (code review): M1's ORTHO world (`SDF_HALF_EXTENT == 1`) is a RIG-SCALE
            // mismatch for this perspective-calibrated light box (`light_position`, a
            // `4.5 x 3.3 x 6.0` world-unit placement volume, `r <= 2.0`) — at
            // `groups/capacity == 24/3456 == 0.0069444`, NO light reaches ANY M1 froxel at
            // `N_ps >= 128`, so `pairs_fine == 0` for every group and BOTH grids are empty.
            // `assert_eq!(hier, flat)` above then compares two vacuously-equal empty grids,
            // which carries zero information — printing its selectivity (`groups/capacity`,
            // independent of N) into the deliverable table would silently misrepresent an
            // empty run as a real measurement. Pin the vacuity EXPLICITLY (the same
            // "invalid run, not a pass" idiom §8.3's mutation (vii) rig requirement uses)
            // rather than let it pass silently — M1 carries real selectivity information
            // only at N_ps in {8, 16, 20, 64} (measured non-vacuous).
            let vacuous = total == 0;
            if !case.selectivity_gated && n_ps > 0 {
                // `N_ps == 0` is trivially vacuous for EVERY config (no point/spot lights at
                // all) -- that is not the rig-scale phenomenon this guard pins, so it is
                // excluded here (already excluded from the deliverable by `pairs_flat > 0`).
                //
                // `N_ps == 1` is ALSO known-vacuous (P1-h's `N=1` restoration, adversarial
                // review, measured: `total_indices == 0`) -- NOT the same rig-scale phenomenon
                // as `N_ps >= 128` (C0-2's comment above), but a SEPARATE, equally deterministic
                // fact of this specific rig: with only one Kronecker light in play, `lights_for`'s
                // fixed placement sequence puts that single light at world `(-4.5, 0.3, -2.0)`
                // (`light_position(0, 1)`), far outside M1's `SDF_HALF_EXTENT == 1` ortho world --
                // pure chance of the deterministic starting point, not the systematic
                // scale-mismatch that makes every `N_ps >= 128` cell vacuous.
                let expected_vacuous = n_ps == 1 || n_ps >= 128;
                assert_eq!(
                    vacuous, expected_vacuous,
                    "{}: the known-vacuous N_ps set changed at N_ps={n_ps} (total_indices={total}) \
                     -- if this now carries real information, update this guard and re-enable it \
                     in the deliverable table for this N_ps",
                    case.name
                );
            }

            // Assertion 5: the selectivity gate, `N_ps >= 128` only, on `selectivity_gated`
            // configs (M1's ORTHO world is a rig-scale mismatch for this box-shaped rig — see
            // `HierMatrixCase::selectivity_gated` — so it is reported, not gated).
            let pairs_flat = u64::from(case.cfg.cluster_count()) * u64::from(stats.ps_n);
            // P1-c (adversarial review): the `!vacuous` guard below silently SKIPS assertion 5
            // (no assertion, no printed row) for any vacuous cell — correct for M1 (the only
            // `!selectivity_gated` config, whose vacuity above `N_ps=128` is the pinned rig-scale
            // mismatch), but a `selectivity_gated` config's placement box is calibrated to its
            // own PERSPECTIVE bench world, so a vacuous run there is NOT a benign scale mismatch —
            // it means the rig or the config drifted. Fail loudly instead of letting the gate go
            // silently unenforced for that cell.
            assert!(
                !(pairs_flat > 0 && case.selectivity_gated && vacuous),
                "{}: a `selectivity_gated` config produced a VACUOUS run (total_indices=0) at \
                 N_ps={n_ps} — assertion 5's `<= 1/8` selectivity gate would be silently skipped \
                 for this cell instead of enforced; this is an invalid run, not a pass",
                case.name
            );
            if pairs_flat > 0 && !vacuous {
                let selectivity = stats.pairs_hier() as f64 / pairs_flat as f64;
                deliverable.push((
                    case.name,
                    n_ps,
                    stats.pairs_coarse,
                    stats.pairs_fine,
                    stats.pairs_hier(),
                    pairs_flat,
                    selectivity,
                ));
                if n_ps >= 128 && case.selectivity_gated {
                    assert!(
                        selectivity <= 1.0 / 8.0,
                        "{}: selectivity {selectivity:.6} misses the 1/8 gate at N_ps={n_ps} \
                         (pairs_hier={}, pairs_flat={pairs_flat}) — §10 ABORT clause 1 fires",
                        case.name,
                        stats.pairs_hier()
                    );
                }
            }
        }
    }

    eprintln!("\n=== H1 deliverable (bench Kronecker rig) — absolute HierCullStats pair counts ===");
    eprintln!(
        "{:<32} {:>6} {:>12} {:>12} {:>12} {:>12} {:>12} {:>8}",
        "config", "N_ps", "coarse", "fine", "hier", "flat", "selectivity", "1/x"
    );
    for (name, n_ps, coarse, fine, hier, flat, selectivity) in &deliverable {
        eprintln!(
            "{name:<32} {n_ps:>6} {coarse:>12} {fine:>12} {hier:>12} {flat:>12} {selectivity:>12.6} {:>7.2}x",
            1.0 / selectivity
        );
    }
}

/// §10 ABORT clause 1 — H1's own cheap kill switch: **"pair-count selectivity below 4x on
/// BOTH the kronecker and infrustum rigs at `N >= 128`"** falsifies the whole rung at zero GPU
/// cost. This is a DIFFERENT (weaker) threshold than §8.6 assertion 5's `<= 1/8` (8x), which is
/// explicitly scoped to "the bench rig" only (checked by
/// [`hier_cull_matches_flat_cull_exactly_across_the_grid_matrix`]) — this test computes BOTH
/// rigs' selectivity per `(config, N_ps)` and asserts the clause does NOT fire, i.e. at least
/// ONE rig stays at or above 4x. `N_ps` is `{128, 512, 1022}` (`1022`, not `1024` — see
/// [`HIER_MATRIX_N`]'s doc: this rig's `l0a_count == 2` makes `light_count == 1026` at
/// `N_ps=1024`, exceeding `MAX_LIGHTS`). M1's ORTHO world is excluded — see
/// `HierMatrixCase::selectivity_gated` — and still printed for information.
///
/// **W2 (code review): [`lights_for_3d`]'s genuinely 3-D selectivity is ALSO reported here**,
/// alongside the two 1-D-locus rigs, for information — it is NOT part of the abort predicate,
/// which the plan (§10) scopes literally to "the kronecker and infrustum rigs" only.
#[test]
fn hier_cull_abort_clause_1_does_not_fire() {
    // `(config, N_ps, kronecker selectivity, in-frustum selectivity, 3d selectivity)`.
    let mut table: Vec<(&str, u32, f64, f64, f64)> = Vec::new();

    for case in hier_matrix_cases() {
        for &n_ps in &[128_u32, 512, 1022] {
            let header = GoldenLightHeader::new(2, n_ps, 1.0);

            // P1-f (adversarial review): all three rigs' hier grids are needed for the
            // selectivity numbers below anyway — comparing them against their own flat oracle
            // here is free coverage, so every rig this function computes gets its equality
            // discharged, not just the Kronecker rig (already independently pinned by
            // `hier_cull_matches_flat_cull_exactly_across_the_grid_matrix`'s own `lights_for`
            // sweep at the same `(case, n_ps)` cells).
            let kronecker_lights = lights_for(n_ps);
            let kronecker_flat = golden_cluster_cull(
                case.img_w, case.img_h, case.camera, &case.cfg, &header, &kronecker_lights, None,
            );
            let (kronecker_hier, kstats) = golden_cluster_cull_hier(
                case.img_w, case.img_h, case.camera, &case.cfg, &header, &kronecker_lights, None,
            );
            assert_eq!(
                kronecker_hier, kronecker_flat,
                "{}: kronecker rig hier grid != flat grid at N_ps={n_ps}",
                case.name
            );

            let in_frustum_lights = lights_for_in_frustum(n_ps);
            let in_frustum_flat = golden_cluster_cull(
                case.img_w, case.img_h, case.camera, &case.cfg, &header, &in_frustum_lights, None,
            );
            let (in_frustum_hier, istats) = golden_cluster_cull_hier(
                case.img_w, case.img_h, case.camera, &case.cfg, &header, &in_frustum_lights, None,
            );
            assert_eq!(
                in_frustum_hier, in_frustum_flat,
                "{}: in-frustum rig hier grid != flat grid at N_ps={n_ps}",
                case.name
            );

            let lights_3d = lights_for_3d(n_ps);
            let flat_3d = golden_cluster_cull(
                case.img_w, case.img_h, case.camera, &case.cfg, &header, &lights_3d, None,
            );
            let (hier_3d, stats_3d) = golden_cluster_cull_hier(
                case.img_w, case.img_h, case.camera, &case.cfg, &header, &lights_3d, None,
            );
            assert_eq!(
                hier_3d, flat_3d,
                "{}: 3-D rig hier grid != flat grid at N_ps={n_ps}",
                case.name
            );

            // Same header (same `ps_n`) drives all three rigs, so `pairs_flat` is shared.
            let pairs_flat = u64::from(case.cfg.cluster_count()) * u64::from(kstats.ps_n);
            if pairs_flat == 0 {
                continue;
            }
            let kronecker_sel = kstats.pairs_hier() as f64 / pairs_flat as f64;
            let in_frustum_sel = istats.pairs_hier() as f64 / pairs_flat as f64;
            let sel_3d = stats_3d.pairs_hier() as f64 / pairs_flat as f64;
            table.push((case.name, n_ps, kronecker_sel, in_frustum_sel, sel_3d));

            if case.selectivity_gated {
                let abort_clause_1_fires = kronecker_sel > 0.25 && in_frustum_sel > 0.25;
                assert!(
                    !abort_clause_1_fires,
                    "{}: §10 ABORT clause 1 FIRES at N_ps={n_ps} — kronecker selectivity \
                     {kronecker_sel:.6} ({:.2}x) AND in-frustum selectivity {in_frustum_sel:.6} \
                     ({:.2}x) are BOTH below the 4x threshold",
                    case.name,
                    1.0 / kronecker_sel,
                    1.0 / in_frustum_sel
                );
            }
        }
    }

    eprintln!("\n=== §10 ABORT clause 1 — pairs_hier / pairs_flat, three rigs ===");
    eprintln!(
        "{:<32} {:>8} {:>16} {:>18} {:>12}",
        "config", "N_ps", "kronecker (1/x)", "in-frustum (1/x)", "3d (1/x)"
    );
    for (name, n_ps, kronecker_sel, in_frustum_sel, sel_3d) in &table {
        eprintln!(
            "{name:<32} {n_ps:>8} {:>15.2}x {:>17.2}x {:>11.2}x",
            1.0 / kronecker_sel,
            1.0 / in_frustum_sel,
            1.0 / sel_3d
        );
    }
}

/// H1 assertion 6: a config with `l0a_count == 0` and `point_spot_count == MAX_LIGHTS` (1024)
/// must be present, so mask word 31 / bit 1023 is exercised, and the produced set must still
/// equal the flat oracle. Every other config in the matrix leaves word 31 dark — `light_count`
/// is clamped to 1024 by the host fold, so any `l0a_count > 0` pushes the point/spot span below
/// 1024 (§8.6: "a 20 000-trial randomized simulation hit word 31 in only 196 runs").
#[test]
fn hier_cull_matches_flat_cull_at_the_mask_capacity_boundary() {
    let cfg = vb_p1d_bench_cluster_cfg();
    let cam = camera();
    let n_ps = 1024_u32;
    let header = GoldenLightHeader::new(0, n_ps, 1.0);
    let lights = lights_point_spot_only(n_ps);

    let flat = golden_cluster_cull(IMG, IMG, cam, &cfg, &header, &lights, None);
    let (hier, stats) = golden_cluster_cull_hier(IMG, IMG, cam, &cfg, &header, &lights, None);

    assert_eq!(
        stats.ps_n, 1024,
        "mask-capacity-boundary rig must saturate HIER_MASK_BITS exactly (ps_n={})",
        stats.ps_n
    );
    assert_eq!(hier, flat, "mask-capacity-boundary: hier grid != flat grid");
    assert_eq!(stats.valid_lanes, cfg.cluster_count());

    // C0-1 (code review): discharge mutation (iii)'s rig requirement (§8.3: "walk mask words
    // descending", detector "needs a froxel holding at least two accepted lights in two
    // different mask words; the rig must run N >= 64 with l0a_count = 0 so bits span words 0
    // and 1") -- this config already satisfies both preconditions (`n_ps=1024 >= 64`,
    // `l0a_count=0`, so a light's mask word is `index / 32` directly, `ps_begin == 0`), but
    // nothing previously asserted the requirement is ACTUALLY met rather than assumed.
    let spans_two_words = flat.iter().any(|cell| {
        let words: BTreeSet<u32> = cell.iter().map(|&idx| idx / 32).collect();
        words.len() >= 2
    });
    assert!(
        spans_two_words,
        "mutation (iii)'s rig requirement (§8.3) is undischarged: no froxel in this config's \
         grid has >= 2 accepted lights spanning 2 different mask words -- the descending-walk \
         mutation would be invisible on this rig"
    );
}

/// H1 assertion 7: replicate the shader's thread-to-froxel walk on a dims matrix that includes
/// non-64-aligned and degenerate grids — pure arithmetic, no camera, no lights, no GPU. Scope
/// (stated in [`assert_hier_map_is_a_permutation`]'s own doc, and here because it is the point
/// of this test): a Rust RE-IMPLEMENTATION of the shader's walk, not a pin on the HLSL.
///
/// **`0x0x1` (P1-e, adversarial review): `0x0x0` alone gives ZERO coverage of D8's divide guards.**
/// At `0x0x0`, `capacity == 0` makes `groups == gps * dim_z == 1 * 0 == 0`
/// ([`golden_hier_groups_per_slice`]'s own `if gps == 0 { 1 }` floor), so
/// `assert_hier_map_is_a_permutation`'s `group_id` loop never runs — [`golden_hier_thread_map`] is
/// called ZERO times and the empty-capacity check passes vacuously, with no coverage at all of the
/// `if dim_x != 0 { .. } else { 0 }` guards (`goldens.rs`'s D8 divide guards). `0x0x1` (`dim_z=1`)
/// keeps `dim_x*dim_y == 0` (so `valid` is `false` on every lane, `s < 0*0 == 0` never holds) while
/// making `groups == 1`, so the loop runs once per lane across all
/// [`boyko_rhi_vulkan::goldens::HIER_GROUP_THREADS`] lanes (256 calls), actually exercising the
/// `dim_x != 0` guard on the `else` branch — the coverage `0x0x0` alone cannot provide.
#[test]
fn hier_thread_map_is_a_permutation_on_degenerate_dims() {
    for &(dim_x, dim_y, dim_z, label) in &[
        (16_u32, 9_u32, 23_u32, "16x9x23 (non-64-aligned)"),
        (1, 1, 1, "1x1x1 (minimal)"),
        (0, 0, 0, "0x0x0 (degenerate header, zero groups dispatched)"),
        (0, 0, 1, "0x0x1 (degenerate width, one group dispatched — exercises D8's divide guards)"),
        (255, 255, 255, "255x255x255 (max packed dims)"),
    ] {
        let capacity = dim_x * dim_y * dim_z;
        assert_hier_map_is_a_permutation(dim_x, dim_y, dim_z, capacity, label);
    }
}

// ============================================================================
// C0-1 (code review) — the adversarial boundary rig (§8.6 matrix, plan §8.3's rig requirement).
// ============================================================================

/// The §8.6 matrix's **adversarial boundary rig** (C0-1, code review): nine lights placed EXACTLY
/// tangent (`sq_dist_point_aabb == r*r` bit-for-bit) to a face, an edge and a corner of a CHOSEN
/// froxel's own world AABB, plus a `+-1 ulp` nudge on `r` for each — the boundary of the cull's
/// `<=` test, where a non-conservative coarse level fails FIRST (plan: "scale the coarse extents
/// inward ... and the adversarial rig must fail assertion 1").
///
/// **Exactness, not approximation.** The offsets are chosen so every add/subtract feeding
/// `sq_dist_point_aabb` is exact IEEE-754 arithmetic — never a `sqrt`-then-square round-trip,
/// which does not generally reproduce the original value bit-for-bit:
/// - `target_max[0]`/`target_max[1]` (X/Y) are `((pixel+0.5)/64)*2-1` (`composite_ray`'s ORTHO
///   branch) — a dyadic fraction (denominator `2^7`) for any pixel in `[0, 64)`, exact in f32.
/// - `target_max[2]` (Z) at slice 0 is `SDF_CAM_Z - z_near` — exact because
///   `golden_slice_view_z(0, cfg) == cfg.z_near` bit-exactly (`x.powf(0.0) == 1.0` for any finite
///   `x`), and `z_near == 0.25` is itself a power of two.
///
/// Adding a small integer offset (`1.0`..`4.0`) to any of these stays exact (well under f32's
/// 24-bit mantissa budget), so `c[i] - aabb_max[i]` reproduces the offset bit-for-bit. The FACE
/// case needs one such term (`r == d`, no `sqrt`); the EDGE/CORNER cases use Pythagorean integer
/// tuples (`3,4,5` / `1,2,2,3`) so the SUM of exact integer squares is itself an exact perfect
/// square, giving `r` with `r*r == sq_dist` exactly.
///
/// **Rig requirement (mutation (ii), §8.3): the target froxel must NOT be the one group-lane-0
/// holds** — with a lane-0 target, "replace the fold with lane 0's value" computes the CORRECT
/// box for that froxel by construction (lane 0's own AABB IS the mutated coarse box), and the
/// mutation is invisible. Requires `target_min`/`target_max` from [`cfg`]'s ORTHO fixture at Z
/// SLICE 0 (only there is the Z bound exact).
///
/// **Rig requirement, second and load-bearing one (found by executing the review mutation, not
/// assumed): the target froxel must be the EXTREMAL froxel of its group on the offset axes.**
/// At `gps == 1` (this config) a group's coarse box encloses the WHOLE z-slice — 144 froxels, the
/// full screen width/height — so a light merely tangent to a NON-extremal froxel's own (small)
/// face sits deep inside the (much larger) coarse box regardless of a 0.1% shrink; the mutation
/// is undetectable there. The coarse box's own face on an axis is exactly the extremal froxel's
/// own face on that axis (its AABB IS one of the values the componentwise max is taken over), so
/// only an extremal froxel makes a tangent-to-froxel light also (at least approximately) tangent
/// to the coarse box, where a 0.1% shrink can flip the accept/reject decision.
fn adversarial_boundary_lights(target_min: [f32; 3], target_max: [f32; 3]) -> Vec<GoldenLight> {
    let cy = (target_min[1] + target_max[1]) * 0.5;
    let cz = (target_min[2] + target_max[2]) * 0.5;

    let mk = |pos: [f32; 3], r: f32| GoldenLight::point(pos, [1.0, 1.0, 1.0], 100.0, r);
    // The standard "next representable float" bit trick for a POSITIVE finite `r` (monotone bit
    // pattern for positives) — avoids depending on `f32::next_up`/`next_down`'s stabilization.
    let bits_nudge = |r: f32, delta: i32| f32::from_bits((r.to_bits() as i32 + delta) as u32);

    let mut lights = Vec::with_capacity(9);

    // FACE (+X): d_x = 2.0 exactly (single term, others 0) -- r = d_x, no sqrt needed.
    let face_pos = [target_max[0] + 2.0, cy, cz];
    lights.push(mk(face_pos, 2.0)); // exact tangent: ACCEPT (index 2)
    lights.push(mk(face_pos, bits_nudge(2.0, 1))); // r + 1 ulp: ACCEPT (index 3)
    lights.push(mk(face_pos, bits_nudge(2.0, -1))); // r - 1 ulp: REJECT (index 4)

    // EDGE (+X, +Y): Pythagorean triple (3, 4, 5) -- d_x^2 + d_y^2 == 25.0 exactly.
    let edge_pos = [target_max[0] + 3.0, target_max[1] + 4.0, cz];
    lights.push(mk(edge_pos, 5.0)); // index 5
    lights.push(mk(edge_pos, bits_nudge(5.0, 1))); // index 6
    lights.push(mk(edge_pos, bits_nudge(5.0, -1))); // index 7

    // CORNER (+X, +Y, +Z): Pythagorean quadruple (1, 2, 2, 3) -- sum of squares == 9.0 exactly.
    // The Z offset is exact ONLY because `target_max[2]` is Z-slice 0's exact near boundary.
    let corner_pos = [target_max[0] + 1.0, target_max[1] + 2.0, target_max[2] + 2.0];
    lights.push(mk(corner_pos, 3.0)); // index 8
    lights.push(mk(corner_pos, bits_nudge(3.0, 1))); // index 9
    lights.push(mk(corner_pos, bits_nudge(3.0, -1))); // index 10

    lights
}

/// C0-1: closes rung H1's P0 gap — without a shipped adversarial rig, §8.6's own RED-if mutation
/// ("scale the coarse extents inward ... and the adversarial rig must fail assertion 1") was
/// UNEXECUTABLE, and mutation (ii)'s detector (§8.3) had no rig at all. Runs the hier oracle
/// against the [`adversarial_boundary_lights`] rig on a chosen NON-lane-0 froxel and asserts
/// `hier == flat` (assertion 1) — the CORRECT enclosure property holding on exactly the rig the
/// plan's own review mutation targets — plus a self-check that the rig's OWN construction is
/// exercising the intended boundary (the 6 "ACCEPT" lights land in the target froxel, the 3
/// "REJECT" lights do not).
#[test]
fn hier_cull_matches_flat_cull_on_the_adversarial_boundary_rig() {
    let cluster_cfg = cfg(); // M1's ORTHO 64x64 fixture -- the only config with exact-dyadic AABB bounds.
    let cam = CompositeCamera::Ortho;
    // The LAST column (x=15, u increases with px) and the FIRST row (y=0, v decreases with py,
    // so the top row holds the max v) -- the group's own EXTREMAL froxel on both X and Y, so its
    // own face coincides with the group-0 coarse box's own face (the second rig requirement
    // above). `z=0` gives the exact Z bound (slice 0's near boundary).
    let (tx, ty, tz) = (15_u32, 0_u32, 0_u32);
    // Group-lane-0's own froxel, at `gps == 1` (this config), is `(x, y) == (0, 0)` for every
    // z-slice -- the mutation (ii) rig requirement above.
    assert_ne!((tx, ty), (0, 0), "the target froxel must not be group-lane-0's own froxel");
    let (target_min, target_max) =
        golden_froxel_aabb(tx, ty, tz, SDF_IMG_W, SDF_IMG_H, cam, &cluster_cfg);

    let mut lights = vec![
        GoldenLight::directional([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0),
        GoldenLight::directional([0.0, -1.0, 0.0], [0.5, 0.5, 0.5], 0.5),
    ];
    lights.extend(adversarial_boundary_lights(target_min, target_max));
    let header = GoldenLightHeader::new(2, 9, 1.0);

    let flat = golden_cluster_cull(SDF_IMG_W, SDF_IMG_H, cam, &cluster_cfg, &header, &lights, None);
    let (hier, stats) =
        golden_cluster_cull_hier(SDF_IMG_W, SDF_IMG_H, cam, &cluster_cfg, &header, &lights, None);
    assert_eq!(hier, flat, "adversarial boundary rig: hier grid != flat grid");
    assert_eq!(stats.valid_lanes, cluster_cfg.cluster_count());

    // Self-check: the 6 ACCEPT lights (exact tangent + r+1ulp, each of face/edge/corner) land in
    // the target froxel, and the 3 REJECT lights (r-1ulp) do not -- confirms the rig's exact
    // arithmetic actually lands on the intended side of the `<=` boundary.
    let target_fi = golden_cluster_index(tx, ty, tz, cluster_cfg.dim_x, cluster_cfg.dim_z) as usize;
    let cell = &flat[target_fi];
    for &accepted_idx in &[2_u32, 3, 5, 6, 8, 9] {
        assert!(
            cell.contains(&accepted_idx),
            "adversarial rig: light {accepted_idx} (ACCEPT boundary) missing from the target froxel"
        );
    }
    for &rejected_idx in &[4_u32, 7, 10] {
        assert!(
            !cell.contains(&rejected_idx),
            "adversarial rig: light {rejected_idx} (REJECT boundary, r-1ulp) unexpectedly present \
             in the target froxel"
        );
    }
}

// ============================================================================
// W3 (code review) — H3 mutation (vii)'s HOST LEG.
// ============================================================================

/// W3: H3 mutation (vii)'s HOST LEG (§8.3 "Mutation (vii) in full", §8.10 item 5, plan Rev 6):
/// poisons froxel `fi == 168` (M2's `(x,y,z) = (7,0,0)`, HIER group 0 lane 7) to an all-NaN AABB
/// in BOTH [`golden_cluster_cull_hier`] (mitigated: the finiteness substitution) and
/// [`golden_cluster_cull`] (froxel 168's own fine test, so its accept pattern matches the
/// mitigated hier arm's — §5 Case B's corollary). `golden_cluster_cull` had NO injection
/// parameter before this rung (a public-API parameter with zero exercising call site on the hier
/// side); this is the `Some` call site both sides needed.
///
/// **Rig requirement, three parts (§8.10 item 5, all mandatory):**
/// - (a) pinned at `N_ps = 128` (`ps_n < max_lights_per_cluster = 256`) -- at `N_ps = 512` the
///   per-froxel clamp can make both arms emit the identical 256-index prefix regardless of the
///   mitigation, which is why Rev 6 moved this rung off `N_ps = 512`;
/// - (b) group 0's coarse box must reject >= 1 punctual light, asserted from
///   [`HierCullStats::group_coarse_accept`]`[0]` on the `inject_nan_froxel: None` run, BEFORE the
///   arm comparison -- a vacuous run (the coarse box already accepts everything) is reported as
///   invalid, not a pass;
/// - (c) the injection point (inside both [`golden_cluster_cull_hier`] and [`golden_cluster_cull`])
///   is pinned after phase 0's AABB build and before the finiteness test -- already the case in
///   both functions' shipped implementation. **Verified by the OBSERVABLE-EFFECT assertions below
///   (P0-1, adversarial review), not by this test's GREEN result alone**: `assert_eq!(hier, flat)`
///   at `N_ps=128` on this exact `(cfg, N)` cell is ALREADY true with ZERO injection -- pinned
///   independently by `hier_cull_matches_flat_cull_exactly_across_the_grid_matrix`'s own M2 row --
///   so a mistyped, ignored, or mis-targeted `inject_nan_froxel` argument would ship this test
///   green while exercising mutation (vii) not at all. The effect assertions below force a
///   measurable before/after delta at froxel 168 (its flat-arm accept count) and at group 0 (its
///   coarse-accept count, D8's absorbing-substitution Case B) so a no-op injection point fails.
#[test]
fn hier_cull_mutation_vii_host_leg_mitigated_arm_matches_flat() {
    let cluster_cfg = vb_p1d_bench_cluster_cfg();
    let cam = camera();
    let n_ps = 128_u32;
    let header = GoldenLightHeader::new(2, n_ps, 1.0);
    let lights = lights_for(n_ps);

    // Rig requirement (b): WITHOUT injection, group 0's coarse box rejects >= 1 punctual light.
    let (_, baseline_stats) =
        golden_cluster_cull_hier(IMG, IMG, cam, &cluster_cfg, &header, &lights, None);
    let group0_coarse_accept = baseline_stats.group_coarse_accept[0];
    assert!(
        group0_coarse_accept < baseline_stats.ps_n,
        "mutation (vii) rig requirement (b): group 0's coarse box accepts every punctual light \
         ({group0_coarse_accept} == ps_n {}) -- a vacuous run, invalid per plan §8.10 item 5b",
        baseline_stats.ps_n
    );

    // The injected comparison: fi=168 poisoned in BOTH arms (the GREEN/mitigated leg).
    let (hier, hier_stats) =
        golden_cluster_cull_hier(IMG, IMG, cam, &cluster_cfg, &header, &lights, Some(168));
    let flat = golden_cluster_cull(IMG, IMG, cam, &cluster_cfg, &header, &lights, Some(168));
    assert_eq!(
        hier, flat,
        "mutation (vii) mitigated arm: hier grid != flat grid with fi=168 poisoned in both -- \
         §5 Case B's corollary requires every froxel of group 0 (168 included) to emit exactly \
         the flat arm's sequence"
    );

    // P0-1 (adversarial review): the equality above is ALREADY true for this exact
    // `(cfg, N_ps=128)` cell with ZERO injection (the M2 row of
    // `hier_cull_matches_flat_cull_exactly_across_the_grid_matrix`), so it cannot by itself prove
    // `inject_nan_froxel` actually fired. Assert the injection's OWN observable effects: without
    // it, froxel 168's real geometry accepts no point/spot light at all (the precondition the
    // delta below needs); with it, EVERY light in the ps range is accepted, because the
    // NaN-poisoned AABB's `f32::max` NaN-swallow (`golden_sq_dist_point_aabb`) collapses every
    // per-axis distance term to `0.0`, forcing `sq_dist == 0 <= r*r` unconditionally.
    let flat_none = golden_cluster_cull(IMG, IMG, cam, &cluster_cfg, &header, &lights, None);
    assert_eq!(
        flat_none[168].len(),
        0,
        "mutation (vii) rig precondition failed: froxel 168 must accept ZERO point/spot lights on \
         the UNPOISONED flat arm for this rig -- without this baseline the before/after delta \
         asserted next carries no information"
    );
    assert_eq!(
        flat[168].len(),
        hier_stats.ps_n as usize,
        "mutation (vii) injection had NO OBSERVABLE EFFECT on the flat arm: `inject_nan_froxel(168)` \
         must make froxel 168 accept EVERY point/spot light ({} of them, ps_n) -- flat[168].len()=={} \
         means the injection point is a no-op (mistyped, ignored, or targeting the wrong froxel), so \
         `assert_eq!(hier, flat)` above is not exercising mutation (vii) at all",
        hier_stats.ps_n,
        flat[168].len()
    );
    assert_eq!(
        hier_stats.group_coarse_accept[0],
        hier_stats.ps_n,
        "mutation (vii) injection had NO OBSERVABLE EFFECT on the hier arm: group 0's coarse-accept \
         count must equal ps_n ({}) once froxel 168's poisoned lane forces D8's ABSORBING \
         substitution ((-f32::MAX, f32::MAX), Case B universe box) into the group's coarse fold -- \
         group_coarse_accept[0]=={} means the substitution never fired, so the mitigated-arm \
         equality above is vacuous, not a proof the mitigation actually degrades to the flat arm's \
         walk",
        hier_stats.ps_n,
        hier_stats.group_coarse_accept[0]
    );
}

// ============================================================================
// W5 (code review) — the per-froxel truncation path.
// ============================================================================

/// W5: a SMALL `max_lights_per_cluster` (8, vs every other matrix config's 256) so O2's
/// clamp-and-drop (`(cell.len() as u32) < cfg.max_lights_per_cluster`) actually FIRES in both
/// mirrors. Every other config in the matrix sets 256, which `max_per_froxel < 256` (assertion 3)
/// never approaches at the tested `N_ps` values, so the "hier and flat truncate the SAME prefix"
/// property had zero coverage before this test.
#[test]
fn hier_cull_truncates_the_same_prefix_as_flat_when_the_cap_binds() {
    let m2 = vb_p1d_bench_cluster_cfg();
    let cluster_cfg = GoldenClusterConfig { max_lights_per_cluster: 8, ..m2 };
    let cam = camera();
    let n_ps = 512_u32;
    let header = GoldenLightHeader::new(2, n_ps, 1.0);
    let lights = lights_for(n_ps);

    let flat = golden_cluster_cull(IMG, IMG, cam, &cluster_cfg, &header, &lights, None);
    let (hier, stats) = golden_cluster_cull_hier(IMG, IMG, cam, &cluster_cfg, &header, &lights, None);

    assert_eq!(hier, flat, "hier grid != flat grid when the per-froxel cap binds at 8");
    assert_eq!(stats.valid_lanes, cluster_cfg.cluster_count());

    let max_per_froxel = flat.iter().map(Vec::len).max().unwrap_or(0);
    assert_eq!(
        max_per_froxel, 8,
        "the truncation short-circuit never actually fired at N_ps={n_ps} \
         (max_per_froxel={max_per_froxel}) -- raise N_ps or check the rig"
    );
}
