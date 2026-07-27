//! Rung VB-P1d — the VisibilityBuffer froxel light-cull GPU-timestamp bench
//! (`docs/VB-PERFORMANCE-TRACK.md`'s VB-P1). [`vb_mesh_froxel.rs`](vb_mesh_froxel.rs)'s
//! five-sphere `grand_showcase_2mat` scene, verbatim, PLUS a PROCEDURALLY-generated
//! `N_ps`-light rig (`BOYKO_VB_BENCH_LIGHTS`) instead of that file's fixed 14-light row —
//! rendered through `RenderPath::VisibilityBuffer × GeometryLegs::Mesh`, with the runner's
//! VB-P1d bench collector (`BOYKO_VB_BENCH`) bracketing the froxel cull dispatch and the
//! `vb_shade`/`vb_resolve` lit-producer dispatch so `boyko_app::runner` can print their
//! averaged GPU wall-clock cost.
//!
//! # Why this measures only ONE leg per process
//!
//! `ResolvedRenderPath::froxel_light_cull` is a BOOT-FROZEN decision (resolved once from
//! `LightingConfig::clusters_enabled` before the window opens, never re-derived per frame —
//! see `GpuSceneBundles::scene`'s own doc) — the froxel arm's GPU pipelines either exist for
//! the WHOLE process or not at all. A single `app.run()` therefore measures exactly one leg
//! (flat OR froxel) of a given `N_ps`; comparing legs needs TWO process runs of this SAME
//! test, exactly as [`vb_mesh_froxel.rs`](vb_mesh_froxel.rs)'s own `BOYKO_VB_FROXEL_FORCE_OFF`
//! knob already establishes for its equality golden. The orchestrator runs this bench twice
//! per `N_ps` and reads the break-even from the two printed `VB-P1d ...` lines.
//!
//! # Env knobs
//!
//! - `BOYKO_VB_BENCH_LIGHTS=<n>` — the point/spot light count `N_ps` this run's [`setup`]
//!   spawns (default 14, matching [`vb_mesh_froxel.rs`](vb_mesh_froxel.rs)'s own base rig).
//!   Read TWICE, independently, by this file's [`setup`] (to spawn the lights) and by
//!   `boyko_app::runner`'s frame loop (as a print label only) — a single source of truth.
//! - `BOYKO_VB_BENCH=1` (any value) — arms the runner's timestamp collector + the bench
//!   accumulation/print loop (`boyko_app::runner`). Unset ⇒ this test behaves exactly like an
//!   ordinary windowed dump (no bench print, no query pools — byte-identical command stream).
//! - `BOYKO_VB_BENCH_FRAMES=<n>` — the TIMED frame budget (default 220, `VB_BENCH_DEFAULT_FRAMES`
//!   in `runner.rs`); the first 20 (`VB_BENCH_WARMUP`) are discarded as warm-up.
//! - `BOYKO_VB_FROXEL_FORCE_OFF` (any value; presence is the trigger) forces
//!   `LightingConfig::clusters_enabled = false` — the flat baseline leg. Unset (the default)
//!   arms clustering — the froxel leg. Mirrors [`vb_mesh_froxel.rs`](vb_mesh_froxel.rs)'s own
//!   knob exactly.
//! - `BOYKO_VB_BENCH_GRID=<dim_x>x<dim_y>x<dim_z>` (e.g. `32x18x24`) — overrides
//!   [`ClusterConfig`]'s froxel-grid dimensions for this ONE boot (VB-P1e H1.5,
//!   `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` §8.7 — the dispatch-shape transfer probe for §1.2's
//!   `13939 ns + 0.2736 ns/pair` fit, which was calibrated on a single shape). **Unset ⇒
//!   `ClusterConfig::default()`, untouched** — byte-identical to every prior run of this test, so
//!   existing goldens and the committed provenance table stay valid. Set, this file builds
//!   `ClusterConfig { dim_x, dim_y, dim_z, ..Default::default() }` exactly ONCE, before
//!   `app.insert_resource` / `app.run()` — never mutated after boot, so D11's boot-snapshot
//!   hazard (the dispatch size and the `ClusterGrid` buffer are boot-time snapshots; the
//!   light-table header tracks the LIVE config) is not exercised. §8.7 sweeps four grids at
//!   fixed `N_ps = 512`: `8x9x24` (1728 froxels), `16x9x24` (3456, the anchor the §1.2 rate was
//!   fit on), `16x9x48` (6912) and `32x18x24` (13824) — plus `32x16x24` (E2's dims) is reachable
//!   here too, run through this same BASE pipeline before the hierarchical arm exists, so if the
//!   base arm is green at those `gps >= 2` dims the config plumbing is proven independently of
//!   the later hierarchical mapping. Every set grid also raises `index_list_cap` to
//!   `cluster_count() * max_lights_per_cluster` — a proven upper bound on the flat list's total
//!   write volume (`cluster_cull.hlsl`'s O2 clamp caps every froxel's LOCAL list at
//!   `max_lights_per_cluster` before the single flush, so the GLOBAL sum can never exceed
//!   `cluster_count() * max_lights_per_cluster` regardless of `N_ps` or the light placement) —
//!   so the swept grids can never hit the O2 clamp-and-drop that would otherwise silently
//!   corrupt the cull-cost measurement (§8.7's own note).
//! - `BOYKO_VB_BENCH_RIG=kronecker|r3|infrustum` (VB-P1e H4, `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md`
//!   §1.4/§8.11) — selects this run's procedural light-placement rig. **Unset ⇒ `kronecker`**
//!   ([`light_position`], untouched), so every existing measurement and the committed
//!   provenance table (`boyko_render::light_policy`) stay reproducible verbatim. `r3` selects
//!   [`r3_light_position`] — the plastic-constant 3-D Kronecker sequence `alpha = (1/p, 1/p^2,
//!   1/p^3)`, `p = 1.220744084605760` (the real root of `x^4 = x + 1`) — genuinely 3-D
//!   equidistributed, unlike `kronecker`'s golden-ratio powers: `g + g^2 == 1` and
//!   `g - g^2 == g^3` collapse EVERY light in that rig onto one of two straight 3-D segments
//!   (§1.4, numerically verified: 0 violations / 1024), which is maximally favourable to a
//!   group-level reject and would flatter the hierarchy's win if measured alone. `infrustum`
//!   selects [`infrustum_light_position`] — stratified INSIDE the view frustum (screen `(u, v)`
//!   crossed with depth `d` in `[3, 12]`, mapped through the camera basis), so density RISES
//!   with `N` instead of leaking out of frustum the way `kronecker`'s cube-root volume growth
//!   does (§1.3: 514 → 55 non-empty froxels as `N` grows). It shares NO generator with
//!   `light_position`/`r3_light_position` (a prior in-frustum attempt reused `light_position`'s
//!   own formula and therefore fixed only the volume-growth defect, not the collinearity one).
//!   Composable with `BOYKO_VB_BENCH_GRID`/`BOYKO_VB_BENCH_LIGHTS` — the rig only changes WHERE
//!   the `N_ps` lights are placed, not how many, nor the froxel grid they are culled against.
//!
//! Windowed-test conventions (mirrors `vb_mesh_froxel.rs`): `#[ignore]` (needs a real windowed
//! GPU device), run with `BOYKO_DISABLE_VALIDATION=1` and `--test-threads=1`.
//!
//! Invoke (one leg, one `N_ps`):
//! ```text
//! BOYKO_DISABLE_VALIDATION=1 BOYKO_VB_BENCH=1 BOYKO_VB_BENCH_LIGHTS=64 \
//!   cargo test -p boyko-app --test vb_p1d_cull_shade_bench -- --ignored --nocapture --test-threads=1
//! BOYKO_DISABLE_VALIDATION=1 BOYKO_VB_BENCH=1 BOYKO_VB_BENCH_LIGHTS=64 BOYKO_VB_FROXEL_FORCE_OFF=1 \
//!   cargo test -p boyko-app --test vb_p1d_cull_shade_bench -- --ignored --nocapture --test-threads=1
//! ```
//! prints one `VB-P1d N_ps=64 config=froxel cull_reset_ns=.. cull_dispatch_ns=..
//! froxel_cull_ns=.. froxel_shade_ns=.. froxel_total_ns=..` line and one
//! `VB-P1d N_ps=64 config=flat flat_shade_ns=..` line respectively. (VB-P1e's rung H0 split the
//! cull bracket in two; `froxel_cull_ns` is now the sum of the first two fields.)
//!
//! Invoke (H1.5's dispatch-shape sweep, one grid, `N_ps = 512` — `BOYKO_VB_FROXEL_FORCE_OFF`
//! stays UNSET: §8.7 measures `froxel_cull_ns`, which the cull dispatch only emits on the
//! froxel/clustered leg):
//! ```text
//! BOYKO_DISABLE_VALIDATION=1 BOYKO_VB_BENCH=1 BOYKO_VB_BENCH_LIGHTS=512 \
//!   BOYKO_VB_BENCH_GRID=32x18x24 \
//!   cargo test -p boyko-app --test vb_p1d_cull_shade_bench -- --ignored --nocapture --test-threads=1
//! ```
//! the orchestrator repeats this for each of the four swept grids (plus `32x16x24`) — see §8.7.
//!
//! Invoke (VB-P1e H4's rig sweep, one rig, `N_ps = 512`, froxel leg, BASE arm — `BOYKO_VB_HIER_CULL`
//! stays UNSET):
//! ```text
//! BOYKO_DISABLE_VALIDATION=1 BOYKO_VB_BENCH=1 BOYKO_VB_BENCH_LIGHTS=512 BOYKO_VB_BENCH_RIG=r3 \
//!   cargo test -p boyko-app --test vb_p1d_cull_shade_bench -- --ignored --nocapture --test-threads=1
//! ```
//! the orchestrator repeats this for `BOYKO_VB_BENCH_RIG=infrustum` and for the unset (`kronecker`)
//! default, at every swept `N_ps`; each of the three rig runs is then repeated with
//! `BOYKO_VB_HIER_CULL=1` added (selects the `-D HIER=1` arm this rung arms in
//! `boyko_app::runner`) to fill in §2's numeric table on the HIER arm.
//!
//! # ⚠️ H1.5 RESULT — the `0.2736 ns/pair` cost model is REFUTED IN FORM (gate §8.7: RED)
//!
//! Measured on an RTX 3060 with this knob, froxel (clustered) leg, 220 timed frames. `froxel_cull_ns`
//! is **independent of the froxel count** over a **108x** range, and depends on `N_ps` alone:
//!
//! | `N_ps` | 128 froxels (`4x4x8`) | 13 824 froxels (`32x18x24`) | ratio |
//! |---|---|---|---|
//! | 8   | 15 796  | 17 456  | 1.11 |
//! | 64  | 64 280  | 65 602  | 1.02 |
//! | 128 | 119 738 | 121 784 | 1.02 |
//! | 512 | 723 350 | 729 472 | **1.01** |
//!
//! The implied marginal rate is `(729 472 - 723 350) / ((13 824 - 128) * 512)` = **0.0009 ns/pair**,
//! against §8.7's gate band of `[0.2052, 0.3420]` — **235x below it**. So
//! `cull_ns = a + b * (froxels * N)` (`docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` §1.2) is the wrong SHAPE:
//! the cost is `f(N)`, not `f(froxels * N)`.
//!
//! **The instrument was validated before the result was believed**, because a grid knob that silently
//! did nothing would produce exactly this flatness. Two controls: a malformed knob value panics (the
//! parse path is live), and `froxel_shade_ns` tracks the grid exactly as physics demands — fatter
//! froxels admit more lights per pixel — over a **6.5x** range: `2x2x2` (8 froxels) 211 624 ns,
//! `4x4x8` 80 263, `16x9x24` 32 722, `32x18x48` (27 648) 33 271. The grid reaches the shader and the
//! cull genuinely produced different per-froxel sets at each point.
//!
//! **Why**: every thread marches in lockstep through the SAME light array, so the loop is a dependent
//! chain fed by broadcast loads. Extra workgroups add threads that walk the same chain in parallel;
//! they do not shorten it. Wall clock is therefore `chain length x per-iteration latency`, and the
//! chain length is `N`.
//!
//! **What this means for VB-P1e** (recorded here rather than in the plan, whose prose is frozen): the
//! hierarchical arm's win is NOT "fewer (froxel, light) pair tests" but "a shorter per-thread chain" —
//! from `N` to `ceil(N/256)` coarse iterations plus `E_coarse` fine ones. At `N_ps = 512` that is
//! `512` -> about `2 + 40 = 42`, i.e. roughly **12x**, against the pair-count model's 38x. The
//! direction and a large magnitude survive; the arithmetic in §7 does not. H4 measures the real thing.
//!
//! ⚠️ **This bench does NOT reproduce across sessions above `N_ps` ≈ 128.** Re-measured on the
//! same RTX 3060 against the table committed at `e7a4767` (the provenance doc-comment on
//! `boyko_render::light_policy::CLUSTER_LO`): `N_ps` ≤ 128 reproduces within ~6% per leg,
//! `N_ps=256` is +21% (froxel) / +23% (flat), and `N_ps=512` is **+125% on the flat leg** / +55%
//! on the froxel leg. Run-to-run spread
//! at `N_ps=512` is ~21% (1.29 / 1.33 / 1.57 ms over three runs), while `BOYKO_VB_BENCH_FRAMES`
//! 40 vs 220 differ by 0.13% — so the pass is stable WITHIN a run and unstable ACROSS runs (GPU
//! power/clock state is the leading suspect; not identified). The two sweeps side by side
//! (`flat_shade_ns` / `froxel_total_ns` in ns; "margin" is froxel's advantage
//! `(flat - froxel) / flat`, positive ⇒ froxel wins):
//!
//! | `N_ps` | flat committed / re-meas | froxel committed / re-meas | margin committed → re-meas |
//! |---|---|---|---|
//! | 8   | 32 799 / 30 888     | 46 816 / 44 963   | -42.7% → -45.6% |
//! | 32  | 60 815 / 57 586     | 71 999 / 67 562   | -18.4% → -17.3% |
//! | 64  | 95 877 / 96 587     | 102 720 / 96 242  | **-7.1% → +0.4%** |
//! | 128 | 167 322 / 158 622   | 163 039 / 173 013 | **+2.6% → -9.1%** |
//! | 256 | 315 044 / 387 133   | 277 662 / 335 179 | +11.9% → +13.4% |
//! | 512 | 592 015 / 1 330 623 | 523 370 / 810 285 | +11.6% → +39.1% |
//!
//! Consequences.
//!
//! 1. A single-sample threshold comparison at high `N_ps` is not decidable on this harness —
//!    repeat runs and state a variance band.
//! 2. The ≈103 break-even is not supported at the precision that table claims. Its DETERMINING
//!    rows (`N_ps=64` and `N_ps=128`, the pair it is interpolated
//!    between) sit on the REPRODUCING side of the split above, and yet BOTH flipped SIGN. They
//!    flip because a margin is a RATIO of two legs that each hold only to ~6%: at 64 the flat leg
//!    moved +0.7% and the froxel leg -6.3%, closing a 7.1-point gap into a 0.4% tie; at 128 flat
//!    moved -5.2% and froxel +6.1%, turning a 2.6% froxel win into a 9.1% loss. Two legs each
//!    inside ±6% admit ~12.8% of movement in their ratio (1.06/0.94), which covers both margins —
//!    note that the +7.1% margin at 64 is NOT itself smaller than the ~6% per-leg figure, so it is
//!    the RATIO band, not the per-leg one, that makes these rows unresolvable.
//! 3. ⚠️ **The re-measurement does not support `CLUSTER_HI = 128`; "conservative" is the wrong
//!    word for it.** Four of the six rows do move toward clustering (+1.1 / +7.5 / +1.6 / +27.5
//!    points at `N_ps` 32 / 64 / 256 / 512), which is where the old "favours clustering MORE, so
//!    the constants stay conservative" reading came from — but `N_ps=128` is one of the two that
//!    move the other way (`N_ps=8` is the other, -2.8), and 128 is both the value `HI` takes and
//!    the row its "froxel already wins by ~2.6%" justification cites. There the margin moves
//!    **11.6 points AGAINST** clustering, measuring the froxel leg **9.1% SLOWER** at exactly the
//!    count `HI` arms it. Re-running the committed table's own interpolation (linear in the
//!    `flat - froxel` ns difference) over the re-measured
//!    rows moves the durable crossing from ≈103 to **≈156** — bracketed by 128 at `-14 391` ns and
//!    256 at `+51 954` ns, i.e. ABOVE `HI` rather than below it (on percentage margins the same
//!    pair gives ≈180). That ≈156 leans on the `N_ps=256` row, which is on the NON-reproducing
//!    side, so it is NOT a replacement constant — but the finding against `HI` does not need it:
//!    the reproducing `N_ps=128` row alone contradicts the sign `HI` was armed on.
//! 4. `CLUSTER_LO = 64` does survive. In the re-measured sweep, AT OR BELOW `N_ps=128` the froxel
//!    leg is ahead only inside a ~2.6-light-wide window (interpolated crossings at ≈63 and ≈65)
//!    and only by 345 ns (0.4%) — a blip below anything this harness can resolve — while below
//!    that window both sweeps agree flat wins by 17-46% (`N_ps` 32 and 8), margins far outside the
//!    ~12.8% ratio band. So disarming at `<= 64` costs at most that 0.4% at the edge itself and is
//!    decisively right beneath it. (Froxel does go ahead again above the ≈156 crossing of point 3,
//!    which is `HI`'s problem, not `LO`'s.)
//!
//! Re-tuning `HI` needs a repeated-run protocol with a stated variance band, not another single
//! sweep (tracked as VB-P1f).

#![cfg(windows)]

use boyko_app::prelude::*;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_render::Material;
use boyko_render::generate_tangents;
use boyko_render::mesh::Vertex;
use boyko_render::{
    ClusterConfig, GeometryLegs, LightingConfig, MeshAssetsVbExt, MeshGeometryTableSlot,
    RenderPath, RenderPathConfig,
};

/// The sun direction TO the light (byte-identical to `grand_showcase_2mat.rs`'s / `vb_mesh.rs`'s
/// / `vb_mesh_froxel.rs`'s).
const SUN_DIR: [f32; 3] = [-0.40, 0.78, 0.48];

/// The default `N_ps` when `BOYKO_VB_BENCH_LIGHTS` is unset — matches `vb_mesh_froxel.rs`'s own
/// base rig (10 points + 4 spots).
const DEFAULT_N_PS: u32 = 14;

/// Verbatim copy of `vb_mesh_froxel.rs::uv_sphere` (itself a verbatim copy of
/// `grand_showcase_2mat.rs::uv_sphere` via `vb_mesh.rs`) — see that file's NOTE for why this is
/// a local copy rather than a shared `tests/common` helper.
fn uv_sphere(radius: f32, stacks: u32, slices: u32, color: [f32; 4]) -> (Vec<Vertex>, Vec<u32>) {
    let pi = core::f32::consts::PI;
    let mut verts = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);
    for i in 0..=stacks {
        let phi = (i as f32 / stacks as f32) * pi; // 0..π, north pole to south
        let (sp, cp) = phi.sin_cos();
        let v = i as f32 / stacks as f32; // phi / π
        for j in 0..=slices {
            let theta = (j as f32 / slices as f32) * (2.0 * pi); // 0..2π
            let (st, ct) = theta.sin_cos();
            let n = [sp * ct, cp, sp * st]; // unit outward normal
            let u = j as f32 / slices as f32; // theta / 2π
            let mut vertex = Vertex::new([n[0] * radius, n[1] * radius, n[2] * radius], n, color);
            vertex.uv = [u, v];
            verts.push(vertex);
        }
    }
    let stride = slices + 1;
    let mut idx = Vec::with_capacity((stacks * slices * 6) as usize);
    for i in 0..stacks {
        for j in 0..slices {
            let a = i * stride + j;
            let b = (i + 1) * stride + j;
            idx.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    generate_tangents(&mut verts, &idx);
    (verts, idx)
}

/// A small fixed warm/cool palette the procedural rig cycles through (index `i % PALETTE.len()`)
/// — mirrors `vb_mesh_froxel.rs`'s own varied per-row colors without needing a per-light table.
const PALETTE: [[f32; 3]; 6] = [
    [1.0, 0.75, 0.5],
    [0.6, 0.8, 1.0],
    [1.0, 0.6, 0.6],
    [0.7, 1.0, 0.7],
    [0.8, 0.8, 1.0],
    [1.0, 0.9, 0.6],
];

/// Golden-ratio Kronecker placement for light `i` of `n` — low-discrepancy along each axis
/// SEPARATELY but NOT in 3D (see the collinearity paragraph below). It spreads `N_ps` lights
/// across a placement box whose volume is `178.2 * scale^3` with `scale^3 = max(n / 14, 1)`, so
/// the NOMINAL lights-per-unit-VOLUME of that box (`n` ÷ box volume) is constant by construction
/// for `n >= 14`. That is the only density this rig actually holds fixed.
///
/// ⚠️ **Per-box-volume is not per-FROXEL, and the claim this doc used to make — that the
/// cube-root scaling "keeps the AVERAGE per-froxel light density, and so the per-cluster light
/// count, roughly constant regardless of `N_ps`" — is refuted by this rig's own measured
/// occupancy.** The probe is `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` §1.3 (a CPU sweep over the
/// host oracle `golden_cluster_cull`, this file's camera, default `ClusterConfig`
/// 16x9x24 = 3456 froxels); §1.4 states the refutation. Over `N_ps` 8 → 1024 — a 128x rise in
/// light count:
///
/// - non-empty froxels COLLAPSE 514 → 55 (9.4x fewer) while `max per froxel` CLIMBS 3 → 109
///   (36x) — the opposite of a constant per-cluster count;
/// - the average over NON-EMPTY froxels rises 1.5 → 49.3 (32x), and even the average over all
///   3456 froxels rises 0.23 → 0.78 (3.4x), the emitted index total going only 789 → 2709.
///
/// The failure mode the old text claimed to avoid is in fact the measured steady state: 85% of
/// froxels are already empty at `N_ps=8` and 97.5% are empty at `N_ps=512` (§1.3). The cause is
/// the collinearity documented below — every light lands on one of two straight segments running
/// diagonally OUT of the frustum, so growing the box pushes them laterally out of the view cone
/// instead of filling it.
///
/// What the probe DOES confirm is the cap half: `max per froxel` peaks at 109 against
/// `MAX_LIGHTS_PER_CLUSTER = 256`, and the 2709-index total is 16.5% of `INDEX_LIST_CAP = 16384`
/// (`boyko_render::light`), across the whole range [`setup`] admits (`n_ps < MAX_LIGHTS = 1024`,
/// asserted there) — so the O2 clamp-and-drop never fires and never silently corrupts a cull-cost
/// reading here. The sweep therefore remains valid as a cull-COST sweep — but a REJECTION-
/// dominated one (99.85% of the `3456 x 512` pair tests fail at `N_ps=512`), i.e. a best case for
/// any hierarchical group reject. That is why §1.4's consequence 2 requires a second, IN-FRUSTUM
/// rig to be reported alongside this one — H4's `infrustum`, with `r3` added to separate the
/// collinearity defect from the volume-growth one (`BOYKO_VB_BENCH_RIG`, module doc).
///
/// The three fractional-part multipliers are the golden-ratio conjugate's powers `g`, `g^2`,
/// `g^3`, and they are NOT mutually independent: `g` is a root of the QUADRATIC `x^2 + x - 1`, so
/// `{1, g, g^2, g^3}` is necessarily linearly DEPENDENT over the rationals — exactly the property
/// a 3-D Kronecker sequence must NOT have. The committed literals carry that dependence:
/// `0.618_033_988_75 + 0.381_966_011_25 == 1.0` exactly in `f64`, and their difference equals the
/// third literal to within 1 ULP. The three axes are therefore maximally CORRELATED rather than
/// alias-free — `fy == -fx (mod 1)` and `fz == 2*fx (mod 1)` hold for every `i` in
/// `[0, MAX_LIGHTS)` (0 violations / 1024, residual <= 2e-13), collapsing every light onto one of
/// two straight 3-D segments. Each axis taken ALONE is still evenly spread (largest gap only
/// ~1.2-1.7x the mean gap over 1024 samples), which is why the sweep remains usable at all.
/// The rig is kept byte-for-byte anyway ([`BenchRig::Kronecker`]) so the committed provenance
/// table stays reproducible verbatim; `BOYKO_VB_BENCH_RIG=r3` is the genuinely 3-D-equidistributed
/// replacement, and the module doc (VB-P1e H4 §1.4) explains why this collinearity would flatter
/// a group-level reject if the hierarchical arm were measured on this rig alone.
fn light_position(i: u32, n: u32) -> [f32; 3] {
    let scale = (f64::from(n) / f64::from(DEFAULT_N_PS)).max(1.0).cbrt() as f32;
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

/// A small jittered range in `[1.2, 2.0]` — kept modest (well below the 2.5-4.0 the fixed
/// `vb_mesh_froxel.rs` rig uses) so each light's froxel footprint stays bounded even at a
/// large `N_ps` (`light_position`'s own doc explains the companion volume-scaling half).
fn light_range(i: u32) -> f32 {
    1.2 + ((f64::from(i) * 0.142_857).fract() as f32) * 0.8
}

/// VB-P1e H4 (module doc, `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` §1.4/§8.11): which procedural
/// light-placement rig `BOYKO_VB_BENCH_RIG` selects.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BenchRig {
    /// The original golden-ratio-power rig ([`light_position`]) — the DEFAULT, kept byte-for-byte
    /// so every prior measurement and the committed provenance table stay reproducible verbatim.
    Kronecker,
    /// The plastic-constant 3-D Kronecker sequence ([`r3_light_position`]).
    R3,
    /// Stratified inside the view frustum ([`infrustum_light_position`]).
    Infrustum,
}

/// Parses `BOYKO_VB_BENCH_RIG` (this file's module doc). Unset ⇒ [`BenchRig::Kronecker`] —
/// byte-identical to every prior run of this test. Panics loudly on an unrecognized value (the
/// SAME "a bench operator's typo must fail immediately" discipline [`parse_grid_spec`] uses).
fn bench_rig() -> BenchRig {
    match std::env::var("BOYKO_VB_BENCH_RIG").ok().as_deref() {
        None => BenchRig::Kronecker,
        Some("kronecker") => BenchRig::Kronecker,
        Some("r3") => BenchRig::R3,
        Some("infrustum") => BenchRig::Infrustum,
        Some(other) => panic!(
            "invariant: BOYKO_VB_BENCH_RIG must be one of kronecker|r3|infrustum, got `{other}`"
        ),
    }
}

/// VB-P1e H4 rig `r3` (`docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` §8.11): the plastic-constant 3-D
/// Kronecker sequence — genuinely 3-D equidistributed, unlike [`light_position`]'s golden-ratio
/// powers (§1.4: `g + g^2 == 1` and `g - g^2 == g^3` collapse every light in that rig onto one of
/// two straight 3-D segments). `p` is the plastic constant, the real root of `x^4 = x + 1`;
/// `alpha = (1/p, 1/p^2, 1/p^3)` carries no such algebraic relation among its three components,
/// so the three axes cannot collapse onto each other the way `light_position`'s can. Otherwise
/// byte-for-byte the SAME volume/placement shape as [`light_position`] (`scale`/`half_x`/`y_min`/
/// `y_span`/`z_min`/`z_span`) — this rig isolates ONLY the "genuinely 3-D" variable, holding the
/// frustum-leak behavior fixed so the two defects (§1.4) are not conflated in one measurement.
fn r3_light_position(i: u32, n: u32) -> [f32; 3] {
    /// The plastic constant `p`, the real root of `x^4 = x + 1`.
    const P: f64 = 1.220_744_084_605_76;
    const P_INV: f64 = 1.0 / P;

    let scale = (f64::from(n) / f64::from(DEFAULT_N_PS)).max(1.0).cbrt() as f32;
    let half_x = 4.5 * scale;
    let y_min = 0.3;
    let y_span = 3.3 * scale;
    let z_min = -2.0 * scale;
    let z_span = 6.0 * scale;

    let t = f64::from(i);
    let fx = (t * P_INV).fract() as f32;
    let fy = (t * P_INV * P_INV).fract() as f32;
    let fz = (t * P_INV * P_INV * P_INV).fract() as f32;
    [(fx * 2.0 - 1.0) * half_x, y_min + fy * y_span, z_min + fz * z_span]
}

/// VB-P1e H4 rig `infrustum` (`docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` §1.4/§8.11): stratified
/// INSIDE the view frustum — screen `(u, v)` crossed with depth `d` in `[3, 12]`, mapped through
/// the camera basis — so density RISES with `N` instead of leaking out of frustum the way
/// [`light_position`]'s cube-root volume growth does (§1.3: 514 → 55 non-empty froxels as `N`
/// grows). Shares NO generator with [`light_position`]/[`r3_light_position`] — a prior in-frustum
/// attempt reused `light_position`'s own multipliers and therefore fixed only the volume-growth
/// defect, not the collinearity one; this rig's `(u, v)` come from an independent
/// sqrt(2)/sqrt(3) Kronecker sequence. The camera basis (`EYE`/`TARGET`/`WORLD_UP` below) is
/// duplicated from this file's own [`setup`] `CameraRig` pose, since lights are spawned before
/// the camera entity exists.
fn infrustum_light_position(i: u32, n: u32) -> [f32; 3] {
    const EYE: Vec3 = Vec3::new(0.0, 1.1, 7.8);
    const TARGET: Vec3 = Vec3::new(0.0, 0.55, 0.0);
    const WORLD_UP: Vec3 = Vec3::new(0.0, 1.0, 0.0);
    const FOV_Y: f32 = 52.0 * core::f32::consts::PI / 180.0;
    const ASPECT: f32 = 1.0;
    /// The near/far depth band (view-space, along the camera's forward axis) the rig samples
    /// within — comfortably inside the `CameraRig`'s own `[0.1, 100.0]` clip range.
    const D_MIN: f32 = 3.0;
    const D_SPAN: f32 = 9.0;

    let forward = (TARGET - EYE).normalize();
    let right = forward.cross(WORLD_UP).normalize();
    let up = right.cross(forward);

    let t = f64::from(i);
    let u = (t * 0.414_213_562_373_095).fract() as f32; // frac(i * (sqrt(2) - 1))
    let v = (t * 0.732_050_807_568_877).fract() as f32; // frac(i * (sqrt(3) - 1))
    let d = D_MIN + (f64::from(i) / f64::from(n.max(1))) as f32 * D_SPAN;

    let half_h = d * (FOV_Y * 0.5).tan();
    let half_w = half_h * ASPECT;
    let pos = EYE + forward * d + right * ((u * 2.0 - 1.0) * half_w) + up * ((v * 2.0 - 1.0) * half_h);
    [pos.x, pos.y, pos.z]
}

/// Verbatim copy of `vb_mesh_froxel.rs::setup`'s five-sphere geometry + sun + sky, PLUS a
/// PROCEDURALLY-generated `N_ps`-light rig (`BOYKO_VB_BENCH_LIGHTS`, default [`DEFAULT_N_PS`])
/// in place of that file's fixed 14-row table — every 4th light (`i % 4 == 3`) is a spot,
/// aimed down at the sphere row exactly as `vb_mesh_froxel.rs`'s own spots are; the rest are
/// points. No shadow-casting flags (this bench measures cull/shade cost, not the atlas).
fn setup(
    mut commands: Commands,
    mut meshes: NonSendResMut<Assets<MeshGpu>>,
    mut materials: ResMut<Assets<Material>>,
    mut geo_table: NonSendResMut<MeshGeometryTableSlot>,
    dev: NonSendRes<GpuDevice>,
) {
    let (verts, idx) = uv_sphere(0.62, 28, 40, [0.7, 0.7, 0.72, 1.0]);
    let sphere = match geo_table.0.as_mut() {
        Some(table) => meshes.register_mesh_vb(dev.get(), &verts, &idx, table),
        None => meshes.register_mesh(dev.get(), &verts, &idx),
    };

    let red = materials.add(Material::new([0.72, 0.04, 0.04, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let green = materials.add(Material::new([0.05, 0.46, 0.10, 1.0], 0.0, 0.38, 0.5, [0.0; 3], 0));
    let gold = materials.add(Material::new([1.0, 0.71, 0.29, 1.0], 1.0, 0.13, 0.5, [0.0; 3], 0));
    let blue = materials.add(Material::new([0.20, 0.38, 0.92, 1.0], 1.0, 0.42, 0.5, [0.0; 3], 0));

    let spacing = 1.55;
    let materials_row: [Option<u16>; 5] =
        [None, Some(red.index() as u16), Some(green.index() as u16), Some(gold.index() as u16), Some(blue.index() as u16)];
    for (i, mat) in materials_row.iter().enumerate() {
        let x = (i as f32 - 2.0) * spacing;
        let e = commands
            .spawn(MeshBundle::new(sphere, Transform::from_translation(Vec3::new(x, 0.6, 0.0))))
            .id();
        if let Some(id) = mat {
            commands.entity(e).insert(MaterialHandle(*id));
        }
    }

    let sun_pose = Affine3A::look_at_rh(
        Vec3::ZERO,
        Vec3::new(SUN_DIR[0], SUN_DIR[1], SUN_DIR[2]),
        Vec3::new(0.0, 1.0, 0.0),
    );
    commands.spawn(DirectionalLightObject {
        transform: Transform {
            translation: Vec3::ZERO,
            rotation: Quat::from_mat3(sun_pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        light: DirectionalLight::new(SUN_DIR, [1.0, 0.97, 0.92], 3.1),
    });

    commands.spawn(SkyLight::new([0.38, 0.44, 0.55], [0.20, 0.20, 0.22]));

    // The procedural `N_ps` point/spot rig (this file's own module doc).
    let n_ps: u32 = std::env::var("BOYKO_VB_BENCH_LIGHTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_N_PS);
    debug_assert!(
        n_ps < boyko_render::MAX_LIGHTS,
        "invariant: N_ps must stay below MAX_LIGHTS (the point/spot table capacity)"
    );
    let rig = bench_rig();
    let aim = Vec3::new(0.0, 0.6, 0.0);
    for i in 0..n_ps {
        let pos = match rig {
            BenchRig::Kronecker => light_position(i, n_ps),
            BenchRig::R3 => r3_light_position(i, n_ps),
            BenchRig::Infrustum => infrustum_light_position(i, n_ps),
        };
        let color = PALETTE[(i as usize) % PALETTE.len()];
        let range = light_range(i);
        let power = 65.0;
        if i % 4 == 3 {
            let p = Vec3::new(pos[0], pos[1], pos[2]);
            let pose = Affine3A::look_at_rh(p, aim, Vec3::new(0.0, 1.0, 0.0));
            commands.spawn(SpotLightObject {
                transform: Transform {
                    translation: p,
                    rotation: Quat::from_mat3(pose.matrix3),
                    scale: Vec3::ONE,
                },
                global: GlobalTransform::IDENTITY,
                light: SpotLight::new(pos, [0.0, -1.0, 0.0], color, power, range, 15.0, 30.0),
            });
        } else {
            commands.spawn(PointLightObject {
                transform: Transform::from_translation(Vec3::new(pos[0], pos[1], pos[2])),
                global: GlobalTransform::IDENTITY,
                light: PointLight::new(pos, color, power, range),
            });
        }
    }

    let pose = Affine3A::look_at_rh(
        Vec3::new(0.0, 1.1, 7.8),
        Vec3::new(0.0, 0.55, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    );
    commands.spawn(CameraRig {
        transform: Transform {
            translation: pose.translation,
            rotation: Quat::from_mat3(pose.matrix3),
            scale: Vec3::ONE,
        },
        global: GlobalTransform::IDENTITY,
        camera: Camera::DEFAULT,
        projection: Projection::Perspective {
            fov_y: 52.0 * core::f32::consts::PI / 180.0,
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        },
    });
}

/// **The VB-P1d froxel cull/shade GPU-timestamp bench (one leg, one `N_ps`, one grid, per
/// process).** This file's own module doc covers the env knobs + why two runs are needed per
/// `N_ps`, and (VB-P1e H1.5) how `BOYKO_VB_BENCH_GRID` sweeps the froxel-grid dimensions.
///
/// `#[ignore]`: needs a real windowed GPU device. Run with `BOYKO_DISABLE_VALIDATION=1`,
/// `BOYKO_VB_BENCH=1`, `BOYKO_VB_BENCH_LIGHTS=<n>`, optionally `BOYKO_VB_FROXEL_FORCE_OFF` and/or
/// `BOYKO_VB_BENCH_GRID=<dim_x>x<dim_y>x<dim_z>`; the orchestrator sweeps `N_ps ∈ {8, 64, 256,
/// 1024}` × `{froxel, flat}`, and separately (H1.5) the four grids at fixed `N_ps = 512`.
#[test]
#[ignore = "needs a real windowed GPU device; BOYKO_VB_BENCH=1 BOYKO_VB_BENCH_LIGHTS=<n> \
            [BOYKO_VB_FROXEL_FORCE_OFF=1] [BOYKO_VB_BENCH_GRID=<x>x<y>x<z>] \
            BOYKO_DISABLE_VALIDATION=1 -- --ignored --nocapture --test-threads=1; the \
            orchestrator sweeps N_ps, both legs, and (H1.5) the froxel grid"]
fn vb_p1d_cull_shade_bench() {
    let mut app = App::new();
    let plugins = EnginePlugins::window("boyko_engine vb-p1d cull/shade bench", 512, 512);
    app.add_plugins(plugins);
    app.add_startup_system(setup);
    app.insert_resource(RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh });
    // The clusters on/off knob (this file's module doc): unset arms clustering (the froxel
    // leg), `BOYKO_VB_FROXEL_FORCE_OFF` forces it off (the flat baseline leg) — the SAME
    // env-toggle convention `vb_mesh_froxel.rs` uses.
    let clusters_enabled = std::env::var("BOYKO_VB_FROXEL_FORCE_OFF").is_err();
    app.insert_resource(LightingConfig { clusters_enabled, ..LightingConfig::default() });
    // The froxel-grid knob (this file's module doc, VB-P1e H1.5 / plan §8.7): unset reproduces
    // today's dispatch exactly. Built ONCE, here, before `app.run()` — never mutated after boot
    // (D11's boot-snapshot hazard).
    app.insert_resource(swept_cluster_config());
    app.run();
}

/// Builds this run's [`ClusterConfig`] from `BOYKO_VB_BENCH_GRID` (this file's module doc, VB-P1e
/// H1.5 / plan §8.7). Unset ⇒ [`ClusterConfig::default()`] verbatim — byte-identical to every
/// prior run of this test. Set ⇒ the swept `dim_x`/`dim_y`/`dim_z`, with `index_list_cap` raised
/// to `cluster_count() * max_lights_per_cluster`: an exact upper bound on the flat index list's
/// total write volume, since `cluster_cull.hlsl`'s O2 clamp caps every froxel's LOCAL list at
/// `max_lights_per_cluster` before the single flush that adds it to the GLOBAL list — so this
/// bound holds regardless of `N_ps` or the light placement, and the swept grids can never hit
/// the clamp-and-drop that would otherwise silently corrupt the cull-cost measurement.
fn swept_cluster_config() -> ClusterConfig {
    let Ok(spec) = std::env::var("BOYKO_VB_BENCH_GRID") else {
        return ClusterConfig::default();
    };
    let (dim_x, dim_y, dim_z) = parse_grid_spec(&spec);
    let base = ClusterConfig::default();
    let index_list_cap = dim_x * dim_y * dim_z * base.max_lights_per_cluster;
    ClusterConfig { dim_x, dim_y, dim_z, index_list_cap, ..base }
}

/// Parses `BOYKO_VB_BENCH_GRID`'s `<dim_x>x<dim_y>x<dim_z>` grammar (e.g. `"32x18x24"`). Panics
/// loudly on a malformed knob (missing/extra field, non-numeric, or zero dimension) — a bench
/// operator's typo must fail immediately, not silently fall back to the default grid and
/// misattribute the measurement to the wrong dispatch shape.
fn parse_grid_spec(spec: &str) -> (u32, u32, u32) {
    const MSG: &str =
        "invariant: BOYKO_VB_BENCH_GRID must be `<dim_x>x<dim_y>x<dim_z>` of u32s, e.g. `32x18x24`";
    let mut fields = spec.split('x');
    let dim_x: u32 = fields.next().and_then(|s| s.parse().ok()).expect(MSG);
    let dim_y: u32 = fields.next().and_then(|s| s.parse().ok()).expect(MSG);
    let dim_z: u32 = fields.next().and_then(|s| s.parse().ok()).expect(MSG);
    assert!(
        fields.next().is_none(),
        "invariant: BOYKO_VB_BENCH_GRID must have exactly three `x`-separated fields, got `{spec}`"
    );
    assert!(
        dim_x > 0 && dim_y > 0 && dim_z > 0,
        "invariant: BOYKO_VB_BENCH_GRID dimensions must be non-zero, got `{spec}`"
    );
    (dim_x, dim_y, dim_z)
}
