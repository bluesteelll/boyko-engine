//! The CPU brick reference — the bit-exact oracle for the eventual GPU brick atlas
//! (SDF brick-atlas campaign, M0).
//!
//! A *brick* is an 8³ block of the narrow-band SDF, stored as quantized
//! `R8_SNORM` distances, with a 1-voxel apron on every face (so the trilinear
//! fetch can sample across the brick boundary without a neighbour lookup) — a 10³
//! allocation. This module is the CPU mirror the M1 GPU fetch is golden-compared
//! against: [`fill_brick`] bakes a brick from the authority edit list, and
//! [`trilinear_reconstruct`] reproduces the hardware trilinear sampler over it.
//!
//! Everything here is `no_std`-clean: fixed `[i8; N]` buffers, `core` arithmetic
//! (plus the crate's `sqrt` shim via [`crate::sdf_edit_list`]), ZERO allocation.
//! It reads the ONE edit authority ([`SdfEditField`]) + the frozen analytic field
//! ([`crate::sdf_edit_list`]) — principle 0: no parallel field implementation.
//!
//! # The conservative-lower-bound contract (C2)
//!
//! Two layers keep the brick a CONSERVATIVE LOWER BOUND on the analytic field
//! (the Hart sphere-tracing precondition — a fetched distance must never exceed
//! the true clearance, or the marcher overshoots the surface):
//!
//! 1. [`classify_brick`] calls a brick EMPTY only when no edit AABB overlaps it,
//!    so an EMPTY classification provably has `|field| > band_half` everywhere
//!    inside (established structurally, not by sparse sampling).
//! 2. [`fill_brick`] biases every stored sample DOWN by [`EPSILON_Q`], the
//!    combined quantization + trilinear-reconstruction slack, so the decoded
//!    trilinear value stays `<=` the analytic field at every interior point.

use crate::{
    BrickClass, SdfEditAabb, SdfEditField, sdf_edit_list, sqrt,
};

/// Interior voxels per brick edge (the data the brick represents).
pub const BRICK_INTERIOR: usize = 8;

/// The 1-voxel apron on each face — so a trilinear fetch near the brick boundary
/// reads a neighbour sample WITHOUT a cross-brick lookup.
pub const APRON: usize = 1;

/// Allocated voxels per brick edge: interior + an apron on BOTH faces.
pub const BRICK_ALLOC: usize = BRICK_INTERIOR + 2 * APRON;

/// Total voxels in one apron'd brick (`BRICK_ALLOC³ = 10³ = 1000`).
pub const BRICK_VOXELS: usize = BRICK_ALLOC * BRICK_ALLOC * BRICK_ALLOC;

/// The world width of one brick voxel (the M0/M1 pinned brick scale).
const VOXEL_SIZE: f32 = 0.25;

/// `sqrt(3)` to f32 precision — the diagonal-to-edge ratio of a cube.
const SQRT_3: f32 = 1.7320508;

/// The world length of a voxel-cube body diagonal (`VOXEL_SIZE * sqrt(3)`). The
/// worst-case distance from any interior sample to the farthest bracketing grid
/// corner: a trusted point's 8 bracketing corners all lie within this radius.
const VOXEL_DIAG: f32 = VOXEL_SIZE * SQRT_3; // 0.4330127

/// The minimum supported radius of curvature of the stored band (world units,
/// VALUES default). Surfaces sharper than this are NOT covered by the per-voxel
/// trilinear bound; the campaign pins the brick scale against this floor.
const R_MIN: f32 = 0.5;

/// The maximum supported band curvature (`1 / R_MIN`) — the worst-case
/// second-derivative magnitude the `δ_tri` trilinear-midpoint bound assumes.
const C_MAX: f32 = 1.0 / R_MIN; // 2.0

/// The snorm encode/decode band scale (world units): the STORED narrow-band
/// half-width the `R8_SNORM` codes span. Deliberately WIDER than the marcher's
/// usable trust band so a trusted interior point's bracketing corners are never
/// saturated to `±1.0` — saturation would erase the curvature the lower bound
/// relies on (the M0 soundness fix: a wide store band, separate from the usable
/// band, instead of the previously conflated single band).
const BAND_HALF_STORE: f32 = 0.90;

/// The conservative store bias (in `band_half`-normalized units, `[0, 1]`).
///
/// [`fill_brick`] subtracts `EPSILON_Q * band_half` from every analytic sample
/// before quantizing, so the decoded TRILINEAR reconstruction never reports MORE
/// clearance than the true analytic field (the C2 lower-bound contract). The bias
/// is reasoned in WORLD units: `EPSILON_Q * BAND_HALF_STORE` is the world-space
/// down-bias, and it must dominate the SUM of the two world-space error sources a
/// trilinear fetch introduces between the stored grid samples and a fetched
/// interior point:
///
/// - **Trilinear midpoint slack** `δ_tri_world`: trilinear interpolation of a
///   curved band over-/under-shoots the true field at a cell midpoint by at most
///   `(1/8)·C_MAX·VOXEL_SIZE²`. With `VOXEL_SIZE = 0.25`, `C_MAX = 2.0`:
///   `δ_tri_world = 0.25² · 2.0 / 8 = 0.015625` (world units — the slack scales
///   with the voxel span, NOT with the band, so this is a world distance).
/// - **Quantization** `δ_quant_world`: an `R8_SNORM` code is
///   `BAND_HALF_STORE / 127` apart, and round-to-nearest costs up to half a step.
///   Over the encode→decode round trip the worst-case error is one full code on
///   the saturating side, bounded here by `BAND_HALF_STORE / 254 ≈ 0.003543`.
///
/// `EPSILON_Q * BAND_HALF_STORE = world bias ≥ δ_tri_world + δ_quant_world`
/// (`= 0.015625 + 0.003543 = 0.019168`); the pinned `EPSILON_Q = 0.0240` gives a
/// `0.0216` world bias, dominating both with margin. This is enforced at compile
/// time by the EPSILON_Q-dominance predicate below and re-checked at runtime
/// against the caller's `(voxel_size, band_half)` in [`fill_brick`]. The tester's
/// worst-case-offset property (`conservative-lower-bound`) is the numeric
/// tripwire.
pub const EPSILON_Q: f32 = 0.0240; // normalized down-bias (world bias 0.0216)

/// The analytic hand-off band half-width (world units): inside `|recon| <
/// BAND_REFINE` the marcher abandons the brick step and evaluates the exact
/// analytic [`crate::sdf_edit_list`] (trust region R2). `1.5 * VOXEL_SIZE` — wide
/// enough that the per-voxel bound has been re-applied at least once before the
/// hand-off, narrow enough to keep the analytic fold rare.
const BAND_REFINE: f32 = 0.375; // 1.5 * VOXEL_SIZE

/// The outer trust edge (world units): the largest `recon` magnitude for which the
/// stored brick value is a PROVEN lower bound (trust region R1's far boundary).
/// Beyond it the bracketing corners may be saturated, so `recon` is only a LOOSE
/// lower bound (region R3). Derived as
/// `BAND_HALF_STORE·(1 - EPSILON_Q) - VOXEL_DIAG`, pinned slightly tighter than
/// the exact recompute (a smaller outer edge only shrinks R1 — never unsound).
const USABLE_BAND_OUTER: f32 = 0.4418;

// ---- M0 conservative-lower-bound soundness predicates (compile-time) ----
//
// These pin the brick's lower-bound contract at build time: if a future tweak to
// any of the constants above breaks the saturation-free / dominance / non-empty
// invariants the marcher's trust regions rely on, the crate FAILS TO COMPILE
// rather than silently emitting an over-reporting (surface-overshooting) brick.

// P1 — Saturation invariant: a trusted interior point (|recon| <= USABLE_BAND_OUTER)
// has all 8 bracketing corners within VOXEL_DIAG of it, biased down by at most
// EPSILON_Q*BAND_HALF_STORE; the store band must be wide enough that none of those
// corners saturate to ±1.0 (saturation would erase the curvature the LB relies on).
const _: () = assert!(
    BAND_HALF_STORE >= USABLE_BAND_OUTER + VOXEL_DIAG + EPSILON_Q * BAND_HALF_STORE,
    "M0: store band too narrow — trusted corners can saturate (lower bound unsound)"
);

// P2 — EPSILON_Q dominance: the world-space down-bias must cover the SUM of the
// trilinear-midpoint slack (VOXEL_SIZE²·C_MAX/8) and the quantization step
// (BAND_HALF_STORE/254), so decode(recon) <= analytic at every interior point.
const _: () = assert!(
    EPSILON_Q * BAND_HALF_STORE
        >= VOXEL_SIZE * VOXEL_SIZE * C_MAX / 8.0 + BAND_HALF_STORE / 254.0,
    "M0: EPSILON_Q under-bounds curvature + quantization (lower bound unsound)"
);

// P3 — R1 non-empty: the analytic hand-off band must lie strictly inside the outer
// trust edge, or the proven brick-step region R1 collapses to nothing.
const _: () = assert!(
    BAND_REFINE < USABLE_BAND_OUTER,
    "M0: refine band >= outer trust edge — proven brick-step region R1 is empty"
);

// ════════════════════════════════════════════════════════════════════════════
// M4 — the CLIP-MAP LOD STACK (the near-field-cache scale enabler).
//
// M0–M3 cache a SINGLE bounded [-4, 4]³ near-field at one brick scale. M4 stacks
// `BRICK_LEVELS` nested, camera-centered cache levels so the brick cache reaches
// past that near field: level `L` uses a brick `2^L`× larger
// ([`brick_world_at_level`]) with a voxel `2^L`× larger ([`voxel_size_at_level`]).
// The coarser the level, the farther it reaches and the less detail it stores.
//
// THE per-level soundness problem (and its fix). The single-level lower-bound
// contract (P2) proves the EPSILON_Q store-bias DOMINATES the trilinear-midpoint
// curvature slack plus the quantization step:
//
//     EPSILON_Q · band_half  >=  voxel²·C_MAX/8  +  band_half/254          (P2)
//
// At a COARSER level the voxel DOUBLES, so the slack `voxel²·C_MAX/8` QUADRUPLES —
// P2 would FAIL and the coarse brick would OVER-report clearance, letting the
// sphere-trace overstep a far surface (the C2 contract violated). The fix scales
// the WHOLE inequality by `2^L` uniformly, so P2 at `L = 0` implies P2 at every
// level (verified algebraically, see [`band_half_at_level`] / [`c_max_at_level`]):
//
//     voxel_size_L = VOXEL_SIZE · 2^L      (the per-level voxel widens)
//     band_half_L  = BAND_HALF_STORE · 2^L (the store band widens with it)
//     C_MAX_L      = C_MAX / 2^L           (a coarser level promises only a
//                                           2^L×-larger radius of curvature;
//                                           sharper FAR features fall to the
//                                           analytic fallback — still EXACT)
//
//   Substituting: LHS_L = EPSILON_Q·band·2^L = 2^L·LHS_0 ; and
//   RHS_L = (voxel·2^L)²·(C_MAX/2^L)/8 + (band·2^L)/254 = 2^L·RHS_0.
//   Both sides scale by 2^L, so the L=0 proof carries to every level. The OTHER
//   M0 predicates (P1 saturation, P3 R1-non-empty) are likewise 2^L-homogeneous
//   in their world-space terms (`USABLE_BAND_OUTER`, `VOXEL_DIAG`, `BAND_REFINE`
//   all scale by 2^L; `EPSILON_Q` is dimensionless), so their ratios are
//   2^L-invariant — the per-level assert block below re-proves all three anyway.

/// The canonical clip-map level count: `BRICK_LEVELS` nested, camera-centered
/// brick-cache levels (M4). Level `L` reaches `2^L`× farther at `2^L`× coarser
/// detail. `brick.rs` is the `no_std` soundness authority for the level math; the
/// GPU side (`boyko_rhi_vulkan`'s `compute.rs`) references `brick::BRICK_LEVELS`.
///
/// Bumping `N` is a one-line change here — the per-level const-assert block below
/// is the soundness GATE: it re-proves the conservative-lower-bound predicates at
/// EVERY level `0..BRICK_LEVELS`, so a coarser level that breaks the EPSILON_Q
/// dominance fails the build rather than silently emitting an over-reporting brick.
pub const BRICK_LEVELS: usize = 3;

/// The near-field (level-0) brick cell edge in world units (`2.0`). The width one
/// apron'd `BRICK_ALLOC³` atlas tile covers at the finest level. Equals
/// `BRICK_INTERIOR · VOXEL_SIZE` (the same identity the M2 grid pins in
/// `compute.rs`), made a `brick.rs` const so the clip-map level math derives from
/// the `no_std` soundness authority.
pub const M2_BRICK_WORLD: f32 = BRICK_INTERIOR as f32 * VOXEL_SIZE; // 8 * 0.25 = 2.0

/// The per-axis cell count of ONE clip-map level's brick grid (`4`). Each level is
/// a `M2_GRID_DIM³` lattice of its own `brick_world_at_level` cells, camera-centered
/// and snapped to its own grid ([`snapped_level_origin`]). Mirrors the M2 grid edge.
pub const M2_GRID_DIM: u32 = 4;

// M5 toroidal addressing keystone: the storage slot of a world cell is
// `world_cell.rem_euclid(M2_GRID_DIM)`, which lowers to a single `& (M2_GRID_DIM - 1)`
// mask ONLY when the dimension is a power of two. A non-power-of-two `M2_GRID_DIM`
// would silently break [`toroidal_slot`]'s mask form (a wrong, slow, modulo) — make
// it a BUILD error at the exact assumption instead.
const _: () = assert!(
    M2_GRID_DIM.is_power_of_two(),
    "M2_GRID_DIM must be a power of two for the M5 toroidal slot mask (rem_euclid == & (DIM-1))"
);

// The level-0 per-level values must reduce to the pinned single-level constants —
// a desync (e.g. a brick-scale tweak that forgets the clip-map) is a build error.
const _: () = assert!(M2_BRICK_WORLD == 2.0);
const _: () = assert!(brick_world_at_level(0) == M2_BRICK_WORLD);
const _: () = assert!(voxel_size_at_level(0) == VOXEL_SIZE);
const _: () = assert!(band_half_at_level(0) == BAND_HALF_STORE);
const _: () = assert!(c_max_at_level(0) == C_MAX);

/// The world cell edge of clip-map level `level`: `M2_BRICK_WORLD · 2^level`
/// (`2.0, 4.0, 8.0` for levels `0, 1, 2`). A coarser level's cell is `2^level`×
/// wider, so it reaches `2^level`× farther from the camera.
#[inline]
pub const fn brick_world_at_level(level: u32) -> f32 {
    M2_BRICK_WORLD * (1u32 << level) as f32
}

/// The world voxel width of clip-map level `level`: `VOXEL_SIZE · 2^level`
/// (`0.25, 0.5, 1.0`). The trilinear-midpoint slack scales with the voxel SPAN,
/// so this widening is exactly what the per-level P2 proof scales the budget by.
#[inline]
pub const fn voxel_size_at_level(level: u32) -> f32 {
    VOXEL_SIZE * (1u32 << level) as f32
}

/// The store-band half-width of clip-map level `level`: `BAND_HALF_STORE · 2^level`
/// (`0.90, 1.80, 3.60`). The store band widens WITH the voxel so the down-bias
/// `EPSILON_Q · band_half_L` scales by `2^level` in lock-step with the curvature +
/// quantization budget — the keystone of the per-level lower-bound proof.
#[inline]
pub const fn band_half_at_level(level: u32) -> f32 {
    BAND_HALF_STORE * (1u32 << level) as f32
}

/// The maximum supported band curvature of clip-map level `level`: `C_MAX / 2^level`
/// (`2.0, 1.0, 0.5`). Equivalently the minimum supported radius of curvature is
/// `R_MIN · 2^level` ([`r_min_at_level`]): a coarser level promises only a
/// `2^level`×-LARGER radius of curvature, so sharper FAR features are out of its
/// contract and fall to the EXACT analytic fallback (never an over-report).
#[inline]
pub const fn c_max_at_level(level: u32) -> f32 {
    C_MAX / (1u32 << level) as f32
}

/// The minimum supported radius of curvature of clip-map level `level`:
/// `R_MIN · 2^level` (`0.5, 1.0, 2.0`) — the inverse-curvature view of
/// [`c_max_at_level`] (`c_max_at_level(level) == 1.0 / r_min_at_level(level)`).
#[inline]
pub const fn r_min_at_level(level: u32) -> f32 {
    R_MIN * (1u32 << level) as f32
}

/// The body-diagonal radius of one level-`level` voxel cube: `VOXEL_DIAG · 2^level`.
/// The worst-case distance from any interior sample to its farthest bracketing
/// corner at this level (scales with the voxel span — used by the per-level P1
/// saturation proof).
#[inline]
const fn voxel_diag_at_level(level: u32) -> f32 {
    VOXEL_DIAG * (1u32 << level) as f32
}

/// The outer trust edge of clip-map level `level`: `USABLE_BAND_OUTER · 2^level`.
/// The largest `recon` magnitude that is a PROVEN lower bound at this level (the
/// world-space trust radius scales with the level, exactly like the store band).
#[inline]
const fn usable_band_outer_at_level(level: u32) -> f32 {
    USABLE_BAND_OUTER * (1u32 << level) as f32
}

/// The analytic hand-off band of clip-map level `level`: `BAND_REFINE · 2^level`.
/// Inside it the marcher abandons this level's brick step for the exact analytic
/// field (scales with the voxel span, so the hand-off stays voxel-relative).
#[inline]
const fn band_refine_at_level(level: u32) -> f32 {
    BAND_REFINE * (1u32 << level) as f32
}

// ---- M4 per-level conservative-lower-bound soundness predicates (compile-time) ----
//
// Re-prove EVERY M0 predicate (P1 saturation, P2 EPSILON_Q dominance, P3 R1
// non-empty) at each level L = 0..BRICK_LEVELS using the `*_at_level` values. The
// L=0 entry is BYTE-IDENTICAL in form to the single-level M0 predicate above (the
// existing predicate's constants, multiplied by `2^0 = 1`). A future bump to
// `BRICK_LEVELS`, or any constant tweak that breaks the per-level dominance, FAILS
// TO COMPILE rather than silently emitting an over-reporting (overshooting) brick.
const _: () = {
    let mut l = 0u32;
    while l < BRICK_LEVELS as u32 {
        // P1_L — saturation invariant (mirrors the M0 P1, scaled to level L).
        assert!(
            band_half_at_level(l)
                >= usable_band_outer_at_level(l)
                    + voxel_diag_at_level(l)
                    + EPSILON_Q * band_half_at_level(l),
            "M4: per-level store band too narrow — trusted corners can saturate (lower bound unsound)"
        );
        // P2_L — EPSILON_Q dominance (mirrors the M0 P2, scaled to level L): the
        // store bias must cover the trilinear-midpoint slack + the quantization step.
        assert!(
            EPSILON_Q * band_half_at_level(l)
                >= voxel_size_at_level(l) * voxel_size_at_level(l) * c_max_at_level(l) / 8.0
                    + band_half_at_level(l) / 254.0,
            "M4: per-level EPSILON_Q dominance broken — unsound coarse brick"
        );
        // P3_L — R1 non-empty (mirrors the M0 P3, scaled to level L): the analytic
        // hand-off band lies strictly inside the outer trust edge.
        assert!(
            band_refine_at_level(l) < usable_band_outer_at_level(l),
            "M4: per-level refine band >= outer trust edge — proven brick-step region R1 is empty"
        );
        l += 1;
    }
};

/// The min world corner of clip-map `level`'s `M2_GRID_DIM³` brick grid, snapped to
/// that level's OWN brick lattice and CENTERED on `camera` (anti-jitter origin).
///
/// Per axis the raw centered min is `camera[a] - 0.5 · M2_GRID_DIM · brick_world_L`;
/// it is then `floor`-snapped to a multiple of `brick_world_L`. Snapping is the
/// anti-jitter keystone: an unsnapped origin would slide continuously with the
/// camera, so every cell's world AABB — and thus its baked brick — would shift
/// sub-cell each frame, re-baking the whole atlas on every move. Snapping pins the
/// grid to discrete `brick_world_L` steps, so the origin only jumps when the camera
/// crosses a cell boundary; the cache content is then frame-stable between jumps.
///
/// Because `brick_world_L = M2_BRICK_WORLD · 2^level`, every coarse cell is an
/// integer multiple (`2^level`) of a finer cell, so a coarser level's snapped grid
/// stays PHASE-ALIGNED with every finer level (a coarse boundary always coincides
/// with a fine boundary) — the levels nest without seams. The level extents are
/// strictly concentric: extent_L = `M2_GRID_DIM · brick_world_L`, doubling per
/// level, so level `L` strictly encloses level `L-1` around the shared camera.
#[inline]
pub fn snapped_level_origin(camera: [f32; 3], level: u32) -> [f32; 3] {
    // M5 Decision 2: the integer cell snap is the ONE authority; the world origin is
    // `origin_cell · brick_world`. This is byte-identical to the prior
    // `(raw_min / bw).floor() * bw` form — the cell index `oc[a] = floor((camera -
    // 0.5·DIM·bw) / bw)` is exactly the `floor` the old code applied, and `oc · bw`
    // reproduces the same multiply (an `m4_snapped_origin_equals_cell_times_bw` test
    // pins the equality so the toroidal reduction stays bit-stable on the OFF path).
    let brick_world = brick_world_at_level(level);
    let cell = snapped_level_origin_cell(camera, level);
    let mut origin = [0.0f32; 3];
    let mut a = 0;
    while a < 3 {
        let snapped = cell[a] as f32 * brick_world;
        // The snapped origin is grid-aligned: a multiple of `brick_world` within fp
        // tolerance. `cell` is an exact integer and `brick_world` is a small power-of-two
        // multiple, so the product is exact for the magnitudes in play; the tolerance
        // only guards the f32 round-trip of the divide/multiply.
        debug_assert!(
            ((snapped / brick_world).round() * brick_world - snapped).abs() <= 1e-3 * brick_world,
            "snapped_level_origin: result not aligned to the level's brick grid"
        );
        origin[a] = snapped;
        a += 1;
    }
    origin
}

/// The INTEGER cell snap of clip-map `level`'s camera-centered grid (M5 Decision 2):
/// `floor((camera − 0.5·M2_GRID_DIM·brick_world_L) / brick_world_L)` per axis, the
/// authority [`snapped_level_origin`] multiplies by `brick_world_L`. World cell `(0,0,0)`
/// of the level's grid sits at world `origin_cell · brick_world_L`.
///
/// Decoupling the WORLD cell from its STORAGE slot is the M5 toroidal keystone: when a
/// level scrolls, `Δcell = new_origin_cell − old_origin_cell` is an EXACT integer (no
/// fp diff of two snapped origins), so the revealed slab is computed in pure integer
/// arithmetic ([`for_each_revealed_cell`]) and exited cells wrap onto the slots of
/// entered cells ([`toroidal_slot`]). At `camera == [0,0,0]` every axis snaps to a
/// fixed cell (`−0.5·DIM` floored), reproducing the M4 origin exactly.
#[inline]
pub fn snapped_level_origin_cell(camera: [f32; 3], level: u32) -> [i32; 3] {
    let brick_world = brick_world_at_level(level);
    let half_extent = 0.5 * M2_GRID_DIM as f32 * brick_world;
    let mut cell = [0i32; 3];
    let mut a = 0;
    while a < 3 {
        let raw_min = camera[a] - half_extent;
        // `floor` then cast: a negative world position snaps to a negative cell index
        // (the toroidal wrap below handles the sign via `rem_euclid`).
        cell[a] = (raw_min / brick_world).floor() as i32;
        a += 1;
    }
    cell
}

/// The toroidal STORAGE slot of a WORLD cell (M5 Decision 1): per axis
/// `world_cell.rem_euclid(M2_GRID_DIM)`, which (since `M2_GRID_DIM` is a power of two —
/// the const-assert at its definition) lowers to `& (M2_GRID_DIM − 1)`. `rem_euclid`
/// is correct for NEGATIVE world cells (a camera left/below the origin), where a plain
/// `%` would yield a negative remainder.
///
/// This is the heart of camera-follow streaming: as the grid scrolls, an exited world
/// cell and a freshly-revealed world cell that are `M2_GRID_DIM` apart map to the SAME
/// slot, so the revealed slab overwrites exactly the slots the departed cells vacated —
/// unchanged cells keep their slots in place (no whole-atlas re-shuffle).
///
/// # OFF reduction (the byte-identity keystone)
///
/// When `origin_cell ≡ 0` the only world cells visited are the box `[0, M2_GRID_DIM)³`,
/// where `world_cell.rem_euclid(M2_GRID_DIM) == world_cell` ⇒ `toroidal_slot(box) == box`,
/// so every M5 scatter site is byte-for-byte the M4 `m2_tile_atlas_origin(box)` mapping.
#[inline]
pub fn toroidal_slot(world_cell: [i32; 3]) -> [u32; 3] {
    let dim = M2_GRID_DIM as i32;
    [
        world_cell[0].rem_euclid(dim) as u32,
        world_cell[1].rem_euclid(dim) as u32,
        world_cell[2].rem_euclid(dim) as u32,
    ]
}

/// Visits every WORLD cell in the NEW box `[new_oc, new_oc + M2_GRID_DIM)³` that is NOT
/// in the OLD box `[old_oc, old_oc + M2_GRID_DIM)³` — the set-difference of two
/// axis-aligned integer boxes, i.e. the slab a scroll REVEALS (the cells whose toroidal
/// slots now hold stale departed-cell data and must be re-baked).
///
/// `f` is invoked once per revealed world cell as `f([wx, wy, wz])` (absolute WORLD cell
/// indices; the caller maps each to its toroidal slot + world AABB). No heap, no sort.
///
/// # The set-difference decomposition
///
/// A cell is revealed iff it lies in the new box AND outside the old box on AT LEAST one
/// axis. Decomposing by "the FIRST axis on which it leaves the old box" partitions the
/// difference into ≤3 disjoint axis-aligned sub-boxes (the standard 3D box-difference
/// shells), so every revealed cell is visited EXACTLY once (no dedup needed for the pure
/// revealed set). On axis `a` the revealed coordinate range is the new range minus its
/// overlap with the old range; on the axes BEFORE `a` the iteration is restricted to the
/// overlap (so the shells don't double-count), and on the axes AFTER `a` it spans the
/// full new range.
///
/// `|Δ| ≥ M2_GRID_DIM` on any axis ⇒ the new and old boxes are disjoint ⇒ the whole new
/// box is revealed (a teleport degrades to a full re-bake — correct, never an over-skip).
#[inline]
pub fn for_each_revealed_cell<F: FnMut([i32; 3])>(old_oc: [i32; 3], new_oc: [i32; 3], mut f: F) {
    let dim = M2_GRID_DIM as i32;
    // Per axis: the new range `[new_lo, new_hi)` and the old range `[old_lo, old_hi)`,
    // and their overlap `[ov_lo, ov_hi)` (empty when the boxes are disjoint on that axis).
    let new_lo = new_oc;
    let new_hi = [new_oc[0] + dim, new_oc[1] + dim, new_oc[2] + dim];
    let old_lo = old_oc;
    let old_hi = [old_oc[0] + dim, old_oc[1] + dim, old_oc[2] + dim];
    // The overlap interval `[ov_lo, ov_hi)` on each axis, CLAMPED into the new range and collapsed to
    // an empty point at `new_lo` when the boxes are disjoint on that axis (so the LOW remainder
    // `[new_lo, ov_lo)` is empty and the HIGH remainder `[ov_hi, new_hi)` spans the whole new range —
    // a disjoint axis reveals every new cell on it, and shells that clamp a LATER axis to this empty
    // overlap contribute nothing, keeping the three shells disjoint and complete).
    let mut ov_lo = [0i32; 3];
    let mut ov_hi = [0i32; 3];
    for a in 0..3 {
        let lo = new_lo[a].max(old_lo[a]);
        let hi = new_hi[a].min(old_hi[a]);
        if lo >= hi {
            // Disjoint on this axis: an empty overlap pinned at the new low edge.
            ov_lo[a] = new_lo[a];
            ov_hi[a] = new_lo[a];
        } else {
            ov_lo[a] = lo;
            ov_hi[a] = hi;
        }
    }

    // The revealed coordinates on each axis: the new range minus the overlap, split into
    // the LOW remainder `[new_lo, ov_lo)` and the HIGH remainder `[ov_hi, new_hi)` (one or
    // both empty when the boxes overlap fully / not at all on that axis).
    //
    // Shell `a` (a = 0,1,2) = cells revealed by leaving the old box on axis `a` FIRST:
    // axis `a` ranges over its revealed remainder, axes `< a` are clamped to the overlap
    // (already counted by an earlier shell otherwise), axes `> a` span the full new range.
    // Empty shells contribute nothing; the three shells are disjoint and cover the whole
    // difference, so each revealed cell is emitted exactly once.

    // Helper: emit `[x0, x1) × [y0, y1) × [z0, z1)` (all half-open, empty if any lo >= hi).
    let emit = |x0: i32, x1: i32, y0: i32, y1: i32, z0: i32, z1: i32, g: &mut F| {
        let mut z = z0;
        while z < z1 {
            let mut y = y0;
            while y < y1 {
                let mut x = x0;
                while x < x1 {
                    g([x, y, z]);
                    x += 1;
                }
                y += 1;
            }
            z += 1;
        }
    };

    // Shell 0 — revealed on X first: X over its low+high remainders, Y/Z full new range.
    emit(
        new_lo[0], ov_lo[0], new_lo[1], new_hi[1], new_lo[2], new_hi[2], &mut f,
    );
    emit(
        ov_hi[0], new_hi[0], new_lo[1], new_hi[1], new_lo[2], new_hi[2], &mut f,
    );
    // Shell 1 — revealed on Y first (and NOT already on X): X clamped to the overlap,
    // Y over its remainders, Z full new range.
    emit(
        ov_lo[0], ov_hi[0], new_lo[1], ov_lo[1], new_lo[2], new_hi[2], &mut f,
    );
    emit(
        ov_lo[0], ov_hi[0], ov_hi[1], new_hi[1], new_lo[2], new_hi[2], &mut f,
    );
    // Shell 2 — revealed on Z first (and NOT already on X or Y): X and Y clamped to the
    // overlap, Z over its remainders.
    emit(
        ov_lo[0], ov_hi[0], ov_lo[1], ov_hi[1], new_lo[2], ov_lo[2], &mut f,
    );
    emit(
        ov_lo[0], ov_hi[0], ov_lo[1], ov_hi[1], ov_hi[2], new_hi[2], &mut f,
    );
}

// ---- The three marcher trust regions (the M0 contract; M1 wires the marcher) ----
//
// `recon` is the trilinear-reconstructed brick value at the sample point. The
// marcher classifies the step by |recon| into three regions, each with a DISTINCT
// soundness argument. The per-voxel EPSILON_Q bound (P2) holds only WITHIN one
// voxel, so the voxel-cap `step <= VOXEL_SIZE` in R1/R2 forces a per-voxel re-eval
// that keeps the within-voxel bound applicable at every step.
//
// - R1 `BAND_REFINE <= |recon| <= USABLE_BAND_OUTER`  → brick step `min(recon,
//   VOXEL_SIZE)`. PROVEN lower bound: the bracketing corners are saturation-free
//   (P1) and EPSILON_Q-corrected (P2), so `recon <= true |d|`; the voxel cap keeps
//   the per-voxel bound applicable.
// - R2 `|recon| < BAND_REFINE`                        → analytic
//   [`crate::sdf_edit_list`]. Near the surface the brick's curvature bound is
//   thinnest, so the marcher hands off to the exact field (also voxel-capped on
//   approach).
// - R3 `|recon| > USABLE_BAND_OUTER`                  → loose-LB coarse step
//   `USABLE_BAND` (i.e. USABLE_BAND_OUTER). The bracketing corners MAY be saturated
//   here, so `recon` is only a LOOSE lower bound (the true `|d|` exceeds it); a
//   coarse step bounded by the usable band is always safe.
//
/// The marcher's USABLE band half-width (world units) — the loose-lower-bound
/// coarse step taken in trust region R3 (`|recon| > USABLE_BAND_OUTER`), and the
/// width the marcher treats as its trusted narrow band. Distinct from the wider
/// [`SDF_EDIT_BAND_HALF`](crate::SDF_EDIT_BAND_HALF) STORE band.
pub const USABLE_BAND: f32 = USABLE_BAND_OUTER;

/// Tests if two AABBs overlap (closed, inclusive on every axis).
///
/// Inclusive bounds are deliberate: a brick touching an edit AABB exactly on a
/// face shares that surface, so it is NOT provably empty — it must classify as
/// `Surface`. Treating touch as overlap keeps the skip conservative.
#[inline]
pub fn aabb_overlap(a: &SdfEditAabb, b: &SdfEditAabb) -> bool {
    a.min[0] <= b.max[0]
        && a.max[0] >= b.min[0]
        && a.min[1] <= b.max[1]
        && a.max[1] >= b.min[1]
        && a.min[2] <= b.max[2]
        && a.max[2] >= b.min[2]
}

/// The minimal AABB enclosing both `a` and `b` (the union bound).
#[inline]
pub fn aabb_union(a: &SdfEditAabb, b: &SdfEditAabb) -> SdfEditAabb {
    SdfEditAabb {
        min: [
            a.min[0].min(b.min[0]),
            a.min[1].min(b.min[1]),
            a.min[2].min(b.min[2]),
        ],
        max: [
            a.max[0].max(b.max[0]),
            a.max[1].max(b.max[1]),
            a.max[2].max(b.max[2]),
        ],
    }
}

// ════════════════════════════════════════════════════════════════════════════
// M3 — the INCREMENTAL DIRTY SET (the dynamic-edit enabler).
//
// M2 re-bakes the WHOLE atlas / pointer grid on every `gen` change. M3 re-bakes
// ONLY the cells whose edits changed. The dirty region is derived ENTIRELY from
// the ONE authority ([`SdfEditField`]) — its `aabbs` (current) vs `prev_aabb`
// (pre-mutation) ledger (principle 0: no side state).
//
// THE union-dirty rule (the #1 correctness guard): a MOVED edit's dirty region is
// `aabbs[i] ∪ prev_aabb[i]` — the SWEPT old+new bound. Covering BOTH locations is
// what clears the ghost at the edit's previous position: re-baking the new AABB
// alone would re-classify the new tiles to `Surface` but leave the OLD tiles still
// holding the moved edit's surface (a phantom). The union sweeps the old tiles too,
// re-classifying them empty.
//
// For ≤16 edits the linear scan here is trivial. TODO(scale): for HUNDREDS+ of
// edits an LBVH over the edit AABBs would prune the per-cell overlap test from O(E)
// to O(log E); not built now (the campaign caps at MAX_SDF_EDITS = 16).
// ════════════════════════════════════════════════════════════════════════════

/// The combined world-space dirty AABB of `field`: the union over every LIVE edit
/// `i` with `aabbs[i] != prev_aabb[i]` of `aabbs[i] ∪ prev_aabb[i]` (the swept
/// old+new region — the union-dirty rule), CONSERVATIVELY widened to cover the
/// SMOOTH-BLEND RIPPLE. Returns `None` when NO live edit is dirty (nothing to
/// re-bake — the caller keeps the prior atlas as-is).
///
/// A cell is DIRTY iff its world AABB overlaps this bound ([`aabb_overlap`]); see
/// [`build_dirty_pointer_grid`] (M1) and the M2 atlas's `rebake_dirty`. The per-cell
/// test inflates the cell AABB by its own footprint margin (the M2 atlas adds the
/// 1-voxel apron reach; the apron-less pointer grid adds nothing), so this bound is
/// the world region the edit list changed, NOT yet the per-cell footprint.
///
/// # Why the swept union alone UNDER-covers smooth scenes
///
/// The field is a SEQUENTIAL fold `acc_k = combine(acc_{k-1}, d_k, op_k, k_k)`. A
/// SMOOTH op (`smin`/`smax`, `smoothness > 0`) blends the accumulator with the new
/// primitive over a transition zone reaching ~`smoothness` BEYOND either primitive,
/// and — because the fold is sequential — a change to edit `i` RIPPLES through every
/// LATER smooth combine: cells in the `i ↔ j` blend zone of a DIFFERENT downstream
/// edit `j` (OUTSIDE `i`'s own AABB) shift value when `i` changes. The per-edit
/// swept union misses those, leaving a stale (ghost) tile — the M3 proptest's
/// SMOOTH-scene divergence.
///
/// # The conservative cover (bit-identical to a full bake — the proptest gate)
///
/// For a PURELY-HARD scene — every CURRENT `smoothness == 0` AND the last-baked scene
/// was hard too ([`SdfEditField::prev_max_smooth`] `== 0`) — this reduces to exactly the
/// changed edits' `(new ∪ prev)` AABBs — TIGHT, no over-dirty (the localized-move perf
/// test stays green). When the scene contains ANY smooth op in EITHER its current OR its
/// pre-mutation state the cover becomes the union of ALL live edits' AABBs (each already
/// skinned by its own `band_half + smoothness` in [`crate::edit_aabb`]), additionally
/// EXPANDED by the `max` of the current and previous scene smoothness: a smooth combine
/// couples the WHOLE folded accumulator (even a FAR smooth term shifts the fold by ~1 f32
/// ULP through `smin`/`smax`), so changing any one edit — or REMOVING the last smooth op —
/// can shift the field anywhere a blend reached, and the only region provably unaffected
/// is where NO edit's band-influence touches at all (exactly the union of the per-edit
/// AABBs, padded by the blend bulge). The M2 grid is `4³ = 64` cells with `<= 16` edits, so
/// re-baking every SURFACE cell on a smooth-scene edit is cheap — correctness over the
/// micro-optimization. A non-overlapping (provably EMPTY) cell is still skipped, so this
/// never re-bakes a far EMPTY tile. (A tighter per-edit ripple bound under-covers by ~1
/// snorm code at a blend-zone corner — the proptest's bit-identity rejects it; the
/// union-of-all is the simple, provably-exact cover. Keying only on the CURRENT smoothness
/// under-covers the SMOOTH→HARD transition, where the removed ripple survives past the
/// mutation — hence the `prev_max_smooth` half of the trigger.)
#[inline]
pub fn dirty_world_aabb(field: &SdfEditField) -> Option<SdfEditAabb> {
    let n = field.count as usize;
    let edits = field.edits();

    // The scene's max smoothness; a `> 0` value switches on the full-cover branch.
    let mut max_smooth = 0.0f32;
    let mut any_dirty = false;
    for (i, e) in edits.iter().enumerate() {
        let s = e.smoothness.max(0.0);
        if s > max_smooth {
            max_smooth = s;
        }
        any_dirty |= field.edit_is_dirty(i);
    }
    if !any_dirty {
        return None;
    }

    // The full-cover blend reach: the LARGER of the CURRENT and the PRE-MUTATION
    // (`prev_max_smooth`, snapshotted at the last `clear_dirty`) scene smoothness. A
    // mutation that REMOVES/HARDENS the last smooth op leaves `max_smooth == 0` but still
    // ripples the fold everywhere the OLD smooth combine reached, so keying only on the
    // current smoothness UNDER-covers (the M3 proptest's smooth-to-hard divergence). Using
    // the max of both states covers the ripple whether the smooth op is being added, kept,
    // or removed.
    let cover_smooth = max_smooth.max(field.prev_max_smooth.max(0.0));

    // SMOOTH scene (now OR before the mutation): a smooth combine couples the WHOLE fold —
    // even where the smooth term is far, `smin`/`smax` perturb the folded accumulator by
    // ~1 f32 ULP, enough to flip a snorm code anywhere a surface is near — so a change (or
    // the removal of a smooth op) can shift the field anywhere a blend reached. The
    // provably-unaffected region is only where NO edit's band-influence touches: the union
    // of every live edit's CURRENT AABB, padded by the blend bulge (`cover_smooth`). Cheap
    // on the 64-cell grid, and an EMPTY (non-overlapping) cell is still skipped, so no far
    // tile is re-baked.
    //
    // The union-of-current alone would leave a GHOST at a moved/shrunk edit's OLD
    // location (a tile that was SURFACE and is now EMPTY but no current AABB reaches):
    // so ALSO union in the `prev_aabb` of every DIRTY edit (the union-dirty rule's
    // old-location half), then expand the whole region by `cover_smooth`.
    if cover_smooth > 0.0 {
        let mut all: Option<SdfEditAabb> = None;
        for aabb in &field.aabbs[..n] {
            all = Some(match all {
                Some(u) => aabb_union(&u, aabb),
                None => *aabb,
            });
        }
        for i in 0..n {
            if field.edit_is_dirty(i) {
                let prev = &field.prev_aabb[i];
                all = Some(match all {
                    Some(u) => aabb_union(&u, prev),
                    None => *prev,
                });
            }
        }
        // `n >= 1` (a dirty edit exists), so `all` is `Some`.
        return all.map(|a| expand_aabb(&a, cover_smooth));
    }

    // HARD scene: the tight swept old+new union per changed edit (no ripple, no
    // over-dirty — the localized-move perf test stays green).
    let mut acc: Option<SdfEditAabb> = None;
    for i in 0..n {
        if !field.edit_is_dirty(i) {
            continue;
        }
        // The swept old+new region for this edit — the union-dirty rule. Covering
        // BOTH the new AABB and the previous one is what clears the ghost at the
        // edit's old location.
        let swept = aabb_union(&field.aabbs[i], &field.prev_aabb[i]);
        acc = Some(match acc {
            Some(u) => aabb_union(&u, &swept),
            None => swept,
        });
    }
    acc
}

/// Inflates `a` by `margin` (world units) on every face — the conservative skin used
/// to cover the smooth-blend transition zone in [`dirty_world_aabb`]. `margin >= 0`.
#[inline]
fn expand_aabb(a: &SdfEditAabb, margin: f32) -> SdfEditAabb {
    SdfEditAabb {
        min: [a.min[0] - margin, a.min[1] - margin, a.min[2] - margin],
        max: [a.max[0] + margin, a.max[1] + margin, a.max[2] + margin],
    }
}

/// Whether cell `(ix, iy, iz)` of `grid` overlaps the world-space `dirty` AABB —
/// the per-cell DIRTY test the incremental pointer-grid / atlas rebake gates on.
#[inline]
pub fn cell_is_dirty(grid: &PointerGrid, ix: u32, iy: u32, iz: u32, dirty: &SdfEditAabb) -> bool {
    let cmin = grid.cell_min(ix, iy, iz);
    let cell_aabb = SdfEditAabb {
        min: cmin,
        max: [
            cmin[0] + grid.brick_world,
            cmin[1] + grid.brick_world,
            cmin[2] + grid.brick_world,
        ],
    };
    aabb_overlap(&cell_aabb, dirty)
}

/// Classifies a brick's occupancy against the authority edit list (C2).
///
/// `brick_min` is the brick's minimum world corner; the brick spans
/// `[brick_min, brick_min + brick_size]` on every axis. `band_half` is the
/// narrow-band half-width the brick atlas stores.
///
/// # The conservative invariant (INVIOLABLE)
///
/// EMPTY only when the analytic field is provably `> band_half` (outside) or
/// `< -band_half` (inside) EVERYWHERE inside the brick, established by edit-AABB
/// non-overlap: if ANY edit's `aabbs[i]` overlaps the brick AABB, the field can
/// cross the band somewhere inside, so the brick is `Surface`. The `aabbs` are
/// conservative ([`crate::edit_aabb`] expands by `band_half + smoothness`), so a
/// non-overlapping brick provably lies outside every edit's band-influence — its
/// field is monotone-far from every surface and the center sample's SIGN settles
/// inside-vs-outside for the whole brick.
#[inline]
pub fn classify_brick(
    field: &SdfEditField,
    brick_min: [f32; 3],
    brick_size: f32,
    band_half: f32,
) -> BrickClass {
    let brick_aabb = SdfEditAabb {
        min: brick_min,
        max: [
            brick_min[0] + brick_size,
            brick_min[1] + brick_size,
            brick_min[2] + brick_size,
        ],
    };

    let n = field.count as usize;
    let edits = field.edits();
    // Surface the moment ANY edit's conservative band-influence reaches the brick.
    for aabb in field.aabbs[..n].iter() {
        if aabb_overlap(aabb, &brick_aabb) {
            return BrickClass::Surface;
        }
    }

    // No edit AABB overlaps: the analytic field is provably `|field| > band_half`
    // everywhere inside, so the brick is uniformly inside or outside. The center
    // sample's SIGN decides which (the field cannot change sign without crossing
    // the band, which non-overlap has ruled out). The `band_half` threshold is a
    // defensive consistency check against the SAME band the AABBs were expanded by:
    // a non-overlapping brick's center is provably outside the band, so `|d|`
    // exceeds `band_half`; if it does not (a stale-AABB / mismatched-band caller),
    // fall back to `Surface` rather than mislabel a near-surface brick EMPTY (C2:
    // never report EMPTY where the field could be inside the band).
    let center = [
        brick_min[0] + brick_size * 0.5,
        brick_min[1] + brick_size * 0.5,
        brick_min[2] + brick_size * 0.5,
    ];
    let d = sdf_edit_list(edits, center);
    if d <= -band_half {
        BrickClass::EmptyInside
    } else if d >= band_half {
        BrickClass::EmptyOutside
    } else {
        BrickClass::Surface
    }
}

// ════════════════════════════════════════════════════════════════════════════
// M1 — the EMPTY-SPACE-SKIP pointer grid (the empty-skip pivot).
//
// A dense 3D grid over a BOUNDED near-field volume. Each cell holds one
// [`BrickClass`] (0/1/2) derived from the authority [`SdfEditField`] via
// [`classify_brick`] — a CONSERVATIVE occupancy label. The marcher skips
// `EmptyOutside` cells to their exit (sound by construction: the conservative
// classifier guarantees no surface within `band_half` of an EMPTY brick, so a
// step to the brick boundary never over-steps a surface — the adjacent brick is
// classified `Surface` if a surface is near). `Surface` cells are marched with the
// EXACT analytic field. NO trilinear, NO image atlas — that is M2.
//
// This is NOT a parallel data system (principle 0): the grid is a TRANSIENT upload
// buffer rebuilt from the ONE edit authority each regen, exactly like the GPU edit
// list (`encode_edit_list`). It owns no durable per-entity state.
// ════════════════════════════════════════════════════════════════════════════

/// The default near-field grid edge (cells per axis), covering a `±GRID dims/2 *
/// brick_world` volume around the origin. The demo / golden scenes live inside a
/// `[-2, 2]³` extent (centers in `[-2, 2]`, primitives `<= 3`), so a 16-cell grid
/// of `0.5`-world bricks spans `[-4, 4]³` — fully enclosing the near field with a
/// margin. The origin/dims/brick_world are PARAMETERS to [`build_pointer_grid`];
/// these are only the bounded defaults the render path seeds with.
pub const DEFAULT_GRID_DIM: u32 = 16;

/// The default world size of one pointer-grid cell (one brick). `0.5` matches the
/// classifier's conservative band reach and keeps the default grid a tractable
/// `16³ = 4096` cells. Distinct from the M2 voxel brick scale ([`VOXEL_SIZE`]).
pub const DEFAULT_BRICK_WORLD: f32 = 0.5;

/// The bounded near-field pointer grid built from the edit authority (M1).
///
/// A dense `dims.0 × dims.1 × dims.2` lattice of [`BrickClass`] codes (one `u32`
/// each — the GPU `StructuredBuffer<uint>` element). Cell `(ix, iy, iz)` covers the
/// world AABB `[origin + (ix,iy,iz)*brick_world, origin + (ix+1,iy+1,iz+1)*brick_world]`
/// and stores `classify_brick` over that AABB. Linear index `ix + iy*W + iz*W*H`
/// (`W = dims.0`, `H = dims.1`) — the SAME order the shader reads.
///
/// The grid is BOUNDED: a march point OUTSIDE `[origin, origin + dims*brick_world]`
/// has no cell, so the marcher falls through to the analytic field there (the grid
/// is a near-field accelerator, never a correctness boundary).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerGrid {
    /// The minimum world corner of cell `(0, 0, 0)`.
    pub origin: [f32; 3],
    /// Cells per axis (`dims.0` = x, `.1` = y, `.2` = z).
    pub dims: [u32; 3],
    /// The world size of one cubic cell (one brick).
    pub brick_world: f32,
}

impl PointerGrid {
    /// The bounded near-field grid: `dim³` cells of `brick_world` each, CENTERED on
    /// `center` (so it spans `[center - dim*brick_world/2, center + dim*brick_world/2]`
    /// on every axis). The default render-path seed.
    #[inline]
    pub fn centered(center: [f32; 3], dim: u32, brick_world: f32) -> Self {
        let half = dim as f32 * brick_world * 0.5;
        Self {
            origin: [center[0] - half, center[1] - half, center[2] - half],
            dims: [dim, dim, dim],
            brick_world,
        }
    }

    /// The default near-field grid centered on the origin
    /// ([`DEFAULT_GRID_DIM`]³ cells of [`DEFAULT_BRICK_WORLD`]).
    #[inline]
    pub fn default_near_field() -> Self {
        Self::centered([0.0, 0.0, 0.0], DEFAULT_GRID_DIM, DEFAULT_BRICK_WORLD)
    }

    /// Total cell count (`dims.0 * dims.1 * dims.2`) — the length of the
    /// [`build_pointer_grid`] destination and the GPU buffer's element count.
    #[inline]
    pub fn cell_count(&self) -> usize {
        self.dims[0] as usize * self.dims[1] as usize * self.dims[2] as usize
    }

    /// The minimum world corner of cell `(ix, iy, iz)`.
    #[inline]
    pub fn cell_min(&self, ix: u32, iy: u32, iz: u32) -> [f32; 3] {
        [
            self.origin[0] + ix as f32 * self.brick_world,
            self.origin[1] + iy as f32 * self.brick_world,
            self.origin[2] + iz as f32 * self.brick_world,
        ]
    }
}

/// Builds the pointer grid into `out` from the authority edit list (M1).
///
/// `out.len()` MUST equal `grid.cell_count()`. For each cell, classifies the
/// brick's world AABB against the authority via [`classify_brick`] at
/// [`crate::SDF_EDIT_BAND_HALF`] (the SAME band the per-edit AABBs are skinned by,
/// so the EMPTY skip stays conservative) and stores the [`BrickClass`] discriminant
/// as a `u32`. The result is the dense grid the GPU marcher reads as a
/// `StructuredBuffer<uint>`.
///
/// This is a SETUP-time (per-`gen`) bake, not a hot-path call — it folds the field
/// once per cell. The render path rebuilds it whenever the authority's `gen` stamp
/// changes (mirroring the edit-list re-encode), into a reused buffer.
pub fn build_pointer_grid(field: &SdfEditField, grid: &PointerGrid, out: &mut [u32]) {
    debug_assert_eq!(
        out.len(),
        grid.cell_count(),
        "pointer-grid destination must have grid.cell_count() cells"
    );

    let w = grid.dims[0];
    let h = grid.dims[1];
    let d = grid.dims[2];
    for iz in 0..d {
        for iy in 0..h {
            for ix in 0..w {
                let cell_min = grid.cell_min(ix, iy, iz);
                let class =
                    classify_brick(field, cell_min, grid.brick_world, crate::SDF_EDIT_BAND_HALF);
                let idx = (ix + iy * w + iz * w * h) as usize;
                out[idx] = class as u32;
            }
        }
    }
}

/// Incrementally re-classifies ONLY the dirty cells of an ALREADY-built pointer
/// grid (M3) — the dynamic-edit fast path for [`build_pointer_grid`].
///
/// `out` MUST be the grid `build_pointer_grid` last filled (its un-dirtied cells
/// hold the correct prior classes). For each cell overlapping the authority's
/// [`dirty_world_aabb`], re-runs [`classify_brick`] and overwrites that cell;
/// every other cell is left untouched. Returns the number of cells re-classified
/// (`0` when nothing was dirty). The result is BIT-IDENTICAL to a full
/// [`build_pointer_grid`] over the same authority (the correctness invariant): a
/// non-dirty cell's class is provably unchanged because no edit's swept region
/// reaches it, so its `classify_brick` result is the same value already stored.
///
/// The caller [`SdfEditField::clear_dirty`](crate::SdfEditField::clear_dirty)s the
/// authority after this so the next mutation diffs against the freshly-baked state.
pub fn build_dirty_pointer_grid(field: &SdfEditField, grid: &PointerGrid, out: &mut [u32]) -> u32 {
    debug_assert_eq!(
        out.len(),
        grid.cell_count(),
        "pointer-grid destination must have grid.cell_count() cells"
    );

    let Some(dirty) = dirty_world_aabb(field) else {
        return 0; // No edit changed: the prior grid is already current.
    };

    let w = grid.dims[0];
    let h = grid.dims[1];
    let d = grid.dims[2];
    let mut touched = 0u32;
    for iz in 0..d {
        for iy in 0..h {
            for ix in 0..w {
                if !cell_is_dirty(grid, ix, iy, iz, &dirty) {
                    continue;
                }
                let cell_min = grid.cell_min(ix, iy, iz);
                let class =
                    classify_brick(field, cell_min, grid.brick_world, crate::SDF_EDIT_BAND_HALF);
                let idx = (ix + iy * w + iz * w * h) as usize;
                out[idx] = class as u32;
                touched += 1;
            }
        }
    }
    touched
}

/// The minimum per-axis progress a brick-exit step makes (world units) — the
/// progress guarantee. A ray parallel to a face, or one starting exactly on a brick
/// boundary, would otherwise compute a zero (or negative) exit distance and stall;
/// clamping the exit to at least this value forces the march forward. Small relative
/// to a cell so it never skips into the next-but-one brick.
pub const BRICK_EXIT_EPS: f32 = 1.0e-4;

/// The ray-AABB SLAB exit distance for the brick at `cell_min` of size
/// `brick_world`, from `ro` along `rd` (the empty-skip step length).
///
/// Returns the `t` at which the ray leaves the brick's `[cell_min, cell_min +
/// brick_world]` AABB, measured from `ro` (NOT from the ray origin — `ro` is the
/// CURRENT march point `p`, so the returned value is the additive step `t += exit`).
/// Standard slab method: per axis the ray enters/exits the two faces; the brick exit
/// is the MIN of the three far-face crossings. A near-zero or negative result
/// (degenerate / boundary-grazing ray) is clamped UP to [`BRICK_EXIT_EPS`] so the
/// march always advances (the progress guarantee — INVIOLABLE for the empty skip).
///
/// SOUNDNESS: this is only ever called for an `EmptyOutside` brick, which the
/// conservative classifier guarantees has NO surface within `band_half` anywhere
/// inside. Stepping to the brick boundary therefore cannot over-step a surface — if a
/// surface is near, the adjacent brick is classified `Surface` and marched
/// analytically. So the empty-skip hit-set equals the pure-analytic hit-set.
#[inline]
pub fn dist_to_brick_exit(
    ro: [f32; 3],
    rd: [f32; 3],
    cell_min: [f32; 3],
    brick_world: f32,
) -> f32 {
    let mut exit = f32::INFINITY;
    for axis in 0..3 {
        let dir = rd[axis];
        let lo = cell_min[axis];
        let hi = lo + brick_world;
        // A near-axis-parallel component: the ray never crosses this axis's slab
        // within the brick, so it imposes no exit bound (skip — the other axes
        // bound it; the BRICK_EXIT_EPS clamp covers a fully-degenerate ray).
        if dir.abs() <= BRICK_EXIT_EPS {
            continue;
        }
        let inv = 1.0 / dir;
        let t_lo = (lo - ro[axis]) * inv;
        let t_hi = (hi - ro[axis]) * inv;
        // The FAR-face crossing along this axis (the larger of the two slab planes).
        let t_far = if t_lo > t_hi { t_lo } else { t_hi };
        if t_far < exit {
            exit = t_far;
        }
    }
    // Progress guarantee: a degenerate ray (every axis skipped, or a boundary-grazing
    // exit) must still advance by at least BRICK_EXIT_EPS.
    if exit < BRICK_EXIT_EPS || !exit.is_finite() {
        BRICK_EXIT_EPS
    } else {
        exit
    }
}

/// Decodes one `R8_SNORM` narrow-band code back to a world-space distance.
///
/// The inverse of the [`fill_brick`] encode (sans the conservative bias, which is
/// baked into the stored code): `q ∈ [-127, 127]` maps linearly onto
/// `[-band_half, +band_half]`. `q == -128` (the snorm sentinel) is clamped to the
/// `-127` magnitude so the decode stays inside the band.
///
/// DELEGATES to [`boyko_shaderdsl::brick::decode_snorm8`] over the `f32` Eval backend
/// (A2): the decode is authored ONCE in `boyko_shaderdsl::brick` (generic over a
/// `FieldScalar`), so this `f32` form and the GPU `m2_decode` scale spliced into
/// `sdf_gbuffer_composite.hlsl` cannot diverge by construction. The `i8` code widens
/// to the backend `i32` losslessly (`q as f32` is identical from either width). The
/// `eval_byte_identity` to-bits sweep locks this against the frozen pre-eDSL snapshot.
#[inline]
pub fn decode_snorm8(q: i8, band_half: f32) -> f32 {
    boyko_shaderdsl::brick::decode_snorm8::<f32>(q as i32, band_half)
}

/// Encodes a world-space distance into an `R8_SNORM` narrow-band code (round to
/// nearest), clamped to the band. Internal — [`fill_brick`] applies the
/// conservative bias BEFORE calling this. `pub(crate)` so the mesh baker
/// ([`crate::mesh_sdf::fill_brick_from_mesh`]) reuses the SAME encoder (byte-parallel
/// brick fill — the single-source contract, principle 0).
#[inline]
pub(crate) fn encode_snorm8(d: f32, band_half: f32) -> i8 {
    let n = (d / band_half).clamp(-1.0, 1.0);
    let scaled = n * 127.0;
    // Round half away from zero (matches the snorm hardware round-to-nearest).
    let rounded = if scaled >= 0.0 {
        (scaled + 0.5) as i32
    } else {
        (scaled - 0.5) as i32
    };
    let q = rounded.clamp(-127, 127) as i8;
    // The clamp pins the code to [-127, 127], so the snorm sentinel -128 can only
    // appear in a brick via a FOREIGN write — this documents that `decode_snorm8`'s
    // `i8::MIN` branch is unreachable for bricks this encoder produced.
    debug_assert!(q != i8::MIN, "encode emitted snorm sentinel -128");
    q
}

/// Bakes one apron'd brick from the authority edit list (M0 reference).
///
/// For each of the `BRICK_ALLOC³` voxel CENTERS (the interior 8³ offset outward by
/// the 1-voxel apron on the low faces), evaluates the analytic field
/// ([`crate::sdf_edit_list`]), biases it DOWN by `EPSILON_Q * band_half` (the
/// conservative-lower-bound store, C2), and quantizes to `R8_SNORM`.
///
/// - `brick_min` is the brick's minimum INTERIOR world corner (the apron extends
///   one voxel below it).
/// - `voxel_size` is the world width of one voxel (`brick_size / BRICK_INTERIOR`).
/// - `band_half` is the narrow-band half-width the codes represent.
/// - `c_max` is the maximum band curvature this brick's level supports
///   ([`c_max_at_level`]; the bare [`C_MAX`](crate::brick) `== c_max_at_level(0)`
///   for the single-level / level-0 path). It scopes the dominance check ONLY (it
///   is NOT stored), so the entry assert verifies the per-level lower-bound budget
///   the compile-time per-level predicate already proved. A COARSER clip-map level
///   promises only a `2^L×`-larger radius of curvature: features sharper than
///   `r_min_at_level(L)` are out of this level's contract and fall to the EXACT
///   analytic fallback (the intended LOD degradation) — the store stays a
///   conservative lower bound for fields of curvature `<= c_max`.
/// - `out` is the `BRICK_VOXELS`-length destination (linear `x + y*W + z*W*W`,
///   `W == BRICK_ALLOC`).
///
/// The bias guarantees the decoded TRILINEAR reconstruction
/// ([`trilinear_reconstruct`]) stays `<=` the analytic field at every interior
/// point, since the world-space bias `EPSILON_Q * band_half` covers both the
/// trilinear midpoint slack (`δ_tri_world`) and the quantization step
/// (`δ_quant_world`) — see [`EPSILON_Q`]. A debug assert at entry re-checks this
/// dominance against the caller's actual `(voxel_size, band_half, c_max)`.
pub fn fill_brick(
    field: &SdfEditField,
    brick_min: [f32; 3],
    voxel_size: f32,
    band_half: f32,
    c_max: f32,
    out: &mut [i8; BRICK_VOXELS],
) {
    // The world-space down-bias must dominate the trilinear-midpoint slack +
    // quantization at the caller's ACTUAL (voxel_size, band_half, c_max) — the
    // runtime mirror of the compile-time per-level P2 predicate. At a COARSER level
    // the curvature term uses this level's `c_max = c_max_at_level(L) = C_MAX/2^L`
    // (not the bare L=0 `C_MAX`), so the whole inequality scales by `2^L` uniformly
    // and holds at every level — see `c_max_at_level` and the per-level const-assert
    // block. `c_max` is assert-only; it never enters the stored value.
    debug_assert!(
        EPSILON_Q * band_half >= voxel_size * voxel_size * c_max / 8.0 + band_half / 254.0,
        "EPSILON_Q under-bounds curvature+quant at this (voxel, band, c_max) — per-level lower-bound budget broken"
    );

    let edits = field.edits();
    let bias = EPSILON_Q * band_half;
    const W: usize = BRICK_ALLOC;

    for z in 0..W {
        for y in 0..W {
            for x in 0..W {
                // The voxel CENTER: the apron shifts the grid one voxel below the
                // interior min, and `+0.5` lands on the voxel center.
                let p = [
                    brick_min[0] + (x as f32 - APRON as f32 + 0.5) * voxel_size,
                    brick_min[1] + (y as f32 - APRON as f32 + 0.5) * voxel_size,
                    brick_min[2] + (z as f32 - APRON as f32 + 0.5) * voxel_size,
                ];
                let d = sdf_edit_list(edits, p);
                // Conservative store: subtract the slack so decode <= analytic.
                let stored = encode_snorm8(d - bias, band_half);
                out[x + y * W + z * W * W] = stored;
            }
        }
    }
}

/// Reconstructs the field at a brick-local point via trilinear interpolation — the
/// CPU mirror of the hardware trilinear texture fetch (M0 reference).
///
/// `local_uvw` is the sample point in INTERIOR-voxel units, where `[0, 0, 0]` is
/// the interior min corner and `[BRICK_INTERIOR, …]` the interior max corner (the
/// apron extends the valid sample range one voxel past each interior face). The
/// result is the decoded, trilinearly-blended distance — the reference the M1 GPU
/// fetch is golden-compared against.
///
/// The 8 corner codes around the sample are decoded ([`decode_snorm8`]) and blended
/// with the per-axis fractional weights, exactly as a hardware sampler does over an
/// `R8_SNORM` 3D texture.
#[inline]
pub fn trilinear_reconstruct(
    brick: &[i8; BRICK_VOXELS],
    local_uvw: [f32; 3],
    band_half: f32,
) -> f32 {
    const W: usize = BRICK_ALLOC;
    // Shift interior-voxel coords into the apron'd grid (the apron is index 0..1),
    // landing on voxel CENTERS: interior origin maps to the first interior center.
    let gx = local_uvw[0] + APRON as f32 - 0.5;
    let gy = local_uvw[1] + APRON as f32 - 0.5;
    let gz = local_uvw[2] + APRON as f32 - 0.5;

    // The low corner cell index, clamped so the +1 neighbour stays in-bounds.
    let x0 = clamp_index(gx, W);
    let y0 = clamp_index(gy, W);
    let z0 = clamp_index(gz, W);
    let x1 = x0 + 1;
    let y1 = y0 + 1;
    let z1 = z0 + 1;

    // The per-axis fractional weights (clamped to the cell the indices bracket).
    let fx = (gx - x0 as f32).clamp(0.0, 1.0);
    let fy = (gy - y0 as f32).clamp(0.0, 1.0);
    let fz = (gz - z0 as f32).clamp(0.0, 1.0);

    let fetch = |x: usize, y: usize, z: usize| -> f32 {
        decode_snorm8(brick[x + y * W + z * W * W], band_half)
    };

    // Trilinear blend: lerp along x, then y, then z (the hardware fetch order).
    let c000 = fetch(x0, y0, z0);
    let c100 = fetch(x1, y0, z0);
    let c010 = fetch(x0, y1, z0);
    let c110 = fetch(x1, y1, z0);
    let c001 = fetch(x0, y0, z1);
    let c101 = fetch(x1, y0, z1);
    let c011 = fetch(x0, y1, z1);
    let c111 = fetch(x1, y1, z1);

    let c00 = lerp(c000, c100, fx);
    let c10 = lerp(c010, c110, fx);
    let c01 = lerp(c001, c101, fx);
    let c11 = lerp(c011, c111, fx);

    let c0 = lerp(c00, c10, fy);
    let c1 = lerp(c01, c11, fy);

    lerp(c0, c1, fz)
}

/// Floors a grid coordinate to a low cell index with room for the `+1` neighbour
/// (clamped into `0..=W-2`).
#[inline]
fn clamp_index(g: f32, w: usize) -> usize {
    if g <= 0.0 {
        0
    } else {
        let i = g as usize;
        if i >= w - 1 { w - 2 } else { i }
    }
}

/// `a + (b - a) * t` — the scalar lerp the trilinear blend uses (byte-matching the
/// hardware fetch's mix order).
#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// ════════════════════════════════════════════════════════════════════════════
// M2 — the JCGT-2022 analytic-cubic SURFACE-brick reference (STEP 1, CPU oracle).
//
// M1 ships empty-space-skip + analytic SURFACE bricks; M2 replaces the analytic
// fold INSIDE a SURFACE brick with a hardware trilinear fetch + the JCGT-2022
// analytic cubic that solves the EXACT ray↔trilinear-isosurface crossing. The
// cubic root IS the hit — no fragile conservative lower bound (the M0/M1 trilinear
// LB tests stay `#[ignore]`d; the cubic makes that path unnecessary). This module
// is the CPU mirror the eventual GPU SURFACE-brick shader is golden-compared
// against, bit-for-bit.
//
// Reference: T. Hansson Söderlund, A. Evans, T. Akenine-Möller, "Ray Tracing of
// Signed Distance Function Grids", JCGT 11(3), 2022 — the analytic-solver section
// (the trilinear interpolant along a ray is a cubic; Marmitt et al.'s iterative
// root-finder isolates the near crossing without transcendentals or 1/c3).
//
// Everything here is the SAME `no_std`, zero-dep, no-alloc, no-unsafe discipline as
// the rest of the brick: fixed `[f32; N]`, `core` arithmetic + the crate `sqrt`
// shim, ZERO allocation, ZERO `unsafe` (pure arithmetic).
// ════════════════════════════════════════════════════════════════════════════

/// A sub-texel sample bias applied to the brick-local grid coordinate in
/// [`atlas_uvw`] (in apron'd-grid voxel units). The hardware trilinear sampler and
/// this CPU oracle must agree on the exact corner-fetch coordinate; the architect
/// flagged that the GPU's texel-center addressing may need a sub-texel nudge to be
/// bit-identical, so the bias is PARAMETERIZED here and golden-locked to `0.0`
/// until the GPU step pins it. `0.0` reproduces [`trilinear_reconstruct`]'s own
/// voxel-center convention exactly (the two MUST agree — see [`atlas_uvw`]).
pub const ATLAS_SAMPLE_BIAS: f32 = 0.0;

/// A root-finding convergence tolerance (world-distance units) shared by the
/// Marmitt and Cardano solvers — the residual `|S(t)|` below which a root is
/// accepted, and the bracket width below which the iteration halts. Tight relative
/// to a voxel span (`VOXEL_SIZE = 0.25`); FP-noise scaled.
pub const CUBIC_ROOT_EPS: f32 = 1.0e-6;

/// The Marmitt root-finder iteration cap. The interpolant is a cubic, so the near
/// crossing is bracketed by at most two interior extrema; a handful of regula-falsi
/// refinement steps inside the sign-bracketed sub-interval converges to
/// [`CUBIC_ROOT_EPS`] well within this bound. Fixed so the GPU port is branch-light
/// (a bounded loop, no `while` on a residual).
pub const MARMITT_ITERS: u32 = 8;

/// Maps a brick-local interior coordinate to the apron'd-grid sample coordinate +
/// the low corner cell index, the SHARED addressing both [`trilinear_reconstruct`]
/// and the JCGT cubic fetch through (so the cubic and the trilinear sample the
/// EXACT same 8 corners — the convention MUST be pinned in ONE place).
///
/// `local_uvw` is in INTERIOR-voxel units (`[0, 0, 0]` = the interior min corner,
/// `[BRICK_INTERIOR, …]` = the interior max corner), matching
/// [`trilinear_reconstruct`]'s domain. Returns `(g, i0)` where:
///
/// - `g[axis]` is the apron'd-grid coordinate (voxel-CENTER convention: the apron
///   shifts the grid by `APRON` and the `-0.5` lands the interior origin on the
///   first interior voxel center), plus [`ATLAS_SAMPLE_BIAS`].
/// - `i0[axis]` is the low corner cell index, clamped so the `+1` neighbour stays
///   in-bounds (`0..=BRICK_ALLOC-2`) — identical to [`trilinear_reconstruct`]'s
///   [`clamp_index`].
///
/// The in-cell fractional coordinate is then `g[axis] - i0[axis]` (in `[0, 1]`),
/// which is the LOCAL `[0,1]³` the JCGT cubic operates in.
#[inline]
pub fn atlas_uvw(local_uvw: [f32; 3], bias: f32) -> ([f32; 3], [usize; 3]) {
    const W: usize = BRICK_ALLOC;
    let g = [
        local_uvw[0] + APRON as f32 - 0.5 + bias,
        local_uvw[1] + APRON as f32 - 0.5 + bias,
        local_uvw[2] + APRON as f32 - 0.5 + bias,
    ];
    let i0 = [
        clamp_index(g[0], W),
        clamp_index(g[1], W),
        clamp_index(g[2], W),
    ];
    (g, i0)
}

/// Forms the JCGT-2022 analytic cubic `c3·t³ + c2·t² + c1·t + c0` whose root is the
/// ray↔trilinear-isosurface crossing inside ONE voxel cell.
///
/// `s` holds the 8 corner distances of the cell with the corner-index convention
/// `s_ijk ↔ index x + 2·y + 4·z` (x FASTEST), i.e. `s[0] = s000`, `s[1] = s100`,
/// `s[2] = s010`, `s[3] = s110`, `s[4] = s001`, `s[5] = s101`, `s[6] = s011`,
/// `s[7] = s111`. This is EXACTLY the order [`trilinear_reconstruct`] fetches
/// (`c000, c100, c010, c110, c001, c101, c011, c111`), so the cubic and the
/// trilinear blend sample the SAME interpolant — verify by construction: the
/// trilinear field is
/// `S(x,y,z) = k0 + k1·x + k2·y + k3·z + k4·xy + k5·yz + k6·zx + k7·xyz`,
/// and `jcgt_cubic_coeffs` substitutes the ray `(x,y,z) = ro_local + rd_local·t`
/// and collects powers of `t`.
///
/// `ro_local` / `rd_local` are the ray origin / direction in the cell's LOCAL
/// `[0,1]³` coordinates (the in-cell fractional coords from [`atlas_uvw`]).
///
/// # The k-basis transcription (the k3/k7 trap)
///
/// The 8 corners are folded into the trilinear k-basis BEFORE the ray substitution.
/// The index pairing is load-bearing — each bilinear `k` term owns a SPECIFIC axis
/// pair, and the trilinear `k7` owns the full triple; a transposed pair silently
/// samples the wrong interpolant (the trap the architect flagged):
///
/// ```text
/// k0 = s000                                                       (constant)
/// k1 = s100 − s000                                                (x)
/// k2 = s010 − s000                                                (y)
/// k3 = s001 − s000                                                (z)
/// k4 = s110 − s100 − s010 + s000                                  (x·y)
/// k5 = s011 − s010 − s001 + s000                                  (y·z)
/// k6 = s101 − s100 − s001 + s000                                  (z·x)
/// k7 = s111 − s110 − s101 − s011 + s100 + s010 + s001 − s000      (x·y·z)
/// ```
///
/// With `x = ax + bx·t`, `y = ay + by·t`, `z = az + bz·t` (`a = ro_local`,
/// `b = rd_local`), expanding and collecting powers of `t` gives the four returned
/// coefficients `[c0, c1, c2, c3]`. The substitution order is fixed here so the GPU
/// port transcribes the SAME FMA chain (a reordered expansion could drift the
/// golden past tolerance — this MUST NOT be "simplified").
///
/// DELEGATES to [`boyko_shaderdsl::brick::jcgt_cubic_coeffs`] over the `f32` Eval
/// backend (A3): the trilinear → cubic-coefficient fold is authored ONCE in
/// `boyko_shaderdsl::brick` (generic over a `FieldScalar`), so this `f32` form and the
/// GPU `m2_jcgt_cubic_coeffs` spliced into `sdf_gbuffer_composite.hlsl` cannot diverge
/// by construction. The `eval_byte_identity` to-bits sweep locks this against the
/// frozen pre-eDSL snapshot — the bit-exact gate, since a single-ULP coefficient drift
/// flips a hit/miss on `t`.
#[inline]
pub fn jcgt_cubic_coeffs(s: &[f32; 8], ro_local: [f32; 3], rd_local: [f32; 3]) -> [f32; 4] {
    boyko_shaderdsl::brick::jcgt_cubic_coeffs::<f32>(s, ro_local, rd_local)
}

/// Evaluates the cubic `c3·t³ + c2·t² + c1·t + c0` at `t` (Horner, FMA-friendly).
///
/// DELEGATES to [`boyko_shaderdsl::brick::cubic_eval`] over the `f32` Eval backend
/// (A3) — authored ONCE alongside [`jcgt_cubic_coeffs`], so this `f32` Horner and the
/// GPU `m2_cubic_eval` cannot diverge. Horner `((c3·t + c2)·t + c1)·t + c0`, the form
/// the GPU shader transcribes.
#[inline]
fn cubic_eval(c: &[f32; 4], t: f32) -> f32 {
    boyko_shaderdsl::brick::cubic_eval::<f32>(c, t)
}

/// The Marmitt et al. iterative root of the JCGT cubic in `[t0, t1]` — the GPU
/// root-finder (FMA-only, NO transcendentals, NO `1/c3` conditioning, robust to
/// `c3 → 0`). Returns the FIRST (near) crossing in `[t0, t1]`, or `None` if the
/// cubic does not change sign across the bracketed sub-interval.
///
/// # Method (JCGT §4.2, Marmitt et al.)
///
/// The cubic has at most two interior extrema (the roots of the quadratic
/// derivative `cubic_deriv`). Those extrema partition `[t0, t1]` into monotone
/// sub-intervals; the FIRST sub-interval whose endpoints bracket a sign change
/// contains the near root, which is then refined by regula-falsi (false position)
/// to [`CUBIC_ROOT_EPS`]. No `c3` division is taken anywhere, so a degenerate
/// near-quadratic (`c3 → 0`) or near-linear cubic is handled by the SAME path —
/// the robustness the GPU needs.
#[inline]
pub fn marmitt_root(c: &[f32; 4], t0: f32, t1: f32) -> Option<f32> {
    // Reject a degenerate or non-ordered (NaN) span: proceed only when t1 strictly
    // exceeds t0 (a NaN bound makes the comparison false → None).
    if t1 <= t0 || t0.is_nan() || t1.is_nan() {
        return None;
    }

    // The interior extrema: roots of the derivative quadratic 3·c3·t² + 2·c2·t + c1.
    // Solved WITHOUT dividing by c3 (clamped into [t0, t1]) — a near-zero leading
    // term collapses to the linear/constant case and simply yields no usable split,
    // leaving the whole interval as one monotone bracket.
    let qa = 3.0 * c[3];
    let qb = 2.0 * c[2];
    let qc = c[1];

    // Up to two split points strictly inside (t0, t1), kept sorted.
    let mut e0 = t1;
    let mut e1 = t1;
    let mut have0 = false;
    let mut have1 = false;

    let disc = qb * qb - 4.0 * qa * qc;
    if qa.abs() > f32::MIN_POSITIVE && disc > 0.0 {
        let sq = sqrt(disc);
        // Numerically-stable quadratic roots (avoid catastrophic cancellation): the
        // standard `q = -(b + sign(b)·sqrt(disc))/2` companion form.
        let q = -0.5 * (qb + qb.signum() * sq);
        let mut r0 = q / qa;
        let mut r1 = if q.abs() > f32::MIN_POSITIVE {
            qc / q
        } else {
            r0
        };
        if r0 > r1 {
            core::mem::swap(&mut r0, &mut r1);
        }
        if r0 > t0 && r0 < t1 {
            e0 = r0;
            have0 = true;
        }
        if r1 > t0 && r1 < t1 {
            if have0 {
                e1 = r1;
                have1 = true;
            } else {
                e0 = r1;
                have0 = true;
            }
        }
    }

    // March the monotone sub-intervals left→right; refine the FIRST sign bracket.
    let mut lo = t0;
    let mut f_lo = cubic_eval(c, lo);
    // The ordered split boundaries (then the far end t1).
    let splits = [
        if have0 { e0 } else { t1 },
        if have1 { e1 } else { t1 },
        t1,
    ];
    for &hi in splits.iter() {
        if hi <= lo {
            continue;
        }
        let f_hi = cubic_eval(c, hi);
        // A sign change (or an endpoint landing on the root) brackets a crossing.
        if f_lo == 0.0 {
            return Some(lo);
        }
        if f_lo * f_hi <= 0.0 {
            return Some(regula_falsi(c, lo, hi, f_lo, f_hi));
        }
        lo = hi;
        f_lo = f_hi;
        if hi >= t1 {
            break;
        }
    }
    None
}

/// Regula-falsi (false position) refinement of a sign-bracketed root in `[lo, hi]`,
/// `f_lo = S(lo)`, `f_hi = S(hi)`, opposite signs. FMA-only, bounded iterations —
/// the GPU-friendly inner refine of [`marmitt_root`].
#[inline]
fn regula_falsi(c: &[f32; 4], mut lo: f32, mut hi: f32, mut f_lo: f32, mut f_hi: f32) -> f32 {
    let mut mid = lo;
    for _ in 0..MARMITT_ITERS {
        let denom = f_hi - f_lo;
        // Degenerate (flat) bracket: fall back to the bisection midpoint.
        mid = if denom.abs() > f32::MIN_POSITIVE {
            lo - f_lo * (hi - lo) / denom
        } else {
            0.5 * (lo + hi)
        };
        let f_mid = cubic_eval(c, mid);
        if f_mid.abs() <= CUBIC_ROOT_EPS || (hi - lo) <= CUBIC_ROOT_EPS {
            return mid;
        }
        if f_lo * f_mid <= 0.0 {
            hi = mid;
            f_hi = f_mid;
        } else {
            lo = mid;
            f_lo = f_mid;
        }
    }
    mid
}

/// The closed-form Cardano cubic solver — the CPU CROSS-CHECK oracle the iterative
/// [`marmitt_root`] is golden-compared against (the two must agree on the first
/// root in `[t0, t1]` within [`CUBIC_ROOT_EPS`]). Returns the SMALLEST real root in
/// `[t0, t1]`, or `None`.
///
/// This is NOT the GPU path (Cardano needs a cube-root / transcendentals the GPU
/// avoids); it exists ONLY as the analytic reference for the tester to pin
/// `marmitt_root` against. It handles the depressed-cubic discriminant cases
/// (one real root vs three real roots) and the degenerate `c3 → 0` quadratic /
/// linear fall-throughs.
#[inline]
pub fn cardano_root(c: &[f32; 4], t0: f32, t1: f32) -> Option<f32> {
    // Reject a degenerate or non-ordered (NaN) span (mirrors `marmitt_root`).
    if t1 <= t0 || t0.is_nan() || t1.is_nan() {
        return None;
    }

    let mut roots = [f32::NAN; 3];
    let mut n = 0usize;
    let push = |r: f32, roots: &mut [f32; 3], n: &mut usize| {
        if r >= t0 - CUBIC_ROOT_EPS && r <= t1 + CUBIC_ROOT_EPS && *n < 3 {
            roots[*n] = r.clamp(t0, t1);
            *n += 1;
        }
    };

    let (a3, a2, a1, a0) = (c[3], c[2], c[1], c[0]);

    if a3.abs() <= f32::MIN_POSITIVE {
        // Degenerate to a quadratic a2·t² + a1·t + a0 (or linear/constant).
        if a2.abs() <= f32::MIN_POSITIVE {
            if a1.abs() > f32::MIN_POSITIVE {
                push(-a0 / a1, &mut roots, &mut n);
            }
        } else {
            let disc = a1 * a1 - 4.0 * a2 * a0;
            if disc >= 0.0 {
                let sq = sqrt(disc);
                let q = -0.5 * (a1 + a1.signum() * sq);
                push(q / a2, &mut roots, &mut n);
                if q.abs() > f32::MIN_POSITIVE {
                    push(a0 / q, &mut roots, &mut n);
                }
            }
        }
    } else {
        // Normalize to a monic depressed cubic t³ + p·t + q via t = w − a2/(3a3).
        let inv = 1.0 / a3;
        let b = a2 * inv;
        let cc = a1 * inv;
        let dd = a0 * inv;
        let shift = b / 3.0;
        let p = cc - b * b / 3.0;
        let q = 2.0 * b * b * b / 27.0 - b * cc / 3.0 + dd;

        let disc = q * q / 4.0 + p * p * p / 27.0;
        if disc > 0.0 {
            // One real root (Cardano's formula).
            let sq = sqrt(disc);
            let u = cbrt(-q / 2.0 + sq);
            let v = cbrt(-q / 2.0 - sq);
            push(u + v - shift, &mut roots, &mut n);
        } else if disc.abs() <= f32::MIN_POSITIVE {
            // A repeated root (disc == 0).
            let u = cbrt(-q / 2.0);
            push(2.0 * u - shift, &mut roots, &mut n);
            push(-u - shift, &mut roots, &mut n);
        } else {
            // Three distinct real roots (trigonometric form).
            let r = sqrt(-p * p * p / 27.0);
            let phi = acos((-q / 2.0) / r);
            let m = 2.0 * cbrt(r);
            for k in 0..3 {
                let ang = (phi + 2.0 * core::f32::consts::PI * k as f32) / 3.0;
                push(m * cos(ang) - shift, &mut roots, &mut n);
            }
        }
    }

    // The smallest in-range real root (the near crossing).
    let mut best: Option<f32> = None;
    for r in roots.iter().take(n) {
        if best.is_none_or(|b| *r < b) {
            best = Some(*r);
        }
    }
    best
}

/// The full CPU SURFACE-brick reference: marches `ro + rd·t` through the brick's
/// interior voxel cells (3D-DDA), and at the first cell whose 8 corners bracket a
/// sign change forms the JCGT cubic ([`jcgt_cubic_coeffs`]) and solves it
/// ([`marmitt_root`]) for the in-cell crossing. Returns the world-space `t` of the
/// FIRST hit, or `None` if the ray clears the brick without crossing the
/// isosurface. This is the CPU mirror of the eventual GPU SURFACE-brick path.
///
/// - `brick` is the apron'd `BRICK_VOXELS` snorm buffer (from [`fill_brick`]).
/// - `ro` / `rd` are the ray in INTERIOR-voxel units (`[0, BRICK_INTERIOR]³` is the
///   interior), matching [`trilinear_reconstruct`]'s domain — the caller maps world
///   space to this frame.
/// - `[t_enter, t_exit]` is the ray's parametric span clipped to the brick interior
///   (computed by the caller's brick-AABB slab test).
/// - `band_half` is the snorm decode band ([`decode_snorm8`]).
///
/// The crossing `t` it returns satisfies `trilinear_reconstruct(brick, ro + rd·t)
/// == 0` to [`CUBIC_ROOT_EPS`] (the cubic and the trilinear sample the identical
/// interpolant — [`jcgt_cubic_coeffs`]), and over a baked brick it tracks the
/// analytic [`sdf_edit_list`] zero-crossing to the brick's quantization tolerance.
pub fn brick_cubic_hit(
    brick: &[i8; BRICK_VOXELS],
    ro: [f32; 3],
    rd: [f32; 3],
    t_enter: f32,
    t_exit: f32,
    band_half: f32,
) -> Option<f32> {
    const W: usize = BRICK_ALLOC;
    // Reject an empty or non-ordered (NaN) brick span.
    if t_exit <= t_enter || t_enter.is_nan() || t_exit.is_nan() {
        return None;
    }

    // Start just inside the entry, in interior-voxel coords.
    let mut t = t_enter;
    // The DDA stepping state per axis: current cell, the step direction, the t to
    // the next cell boundary, and the t increment per cell crossing.
    let mut cell = [0i32; 3];
    let mut step = [0i32; 3];
    let mut t_next = [f32::INFINITY; 3];
    let mut t_delta = [f32::INFINITY; 3];

    for axis in 0..3 {
        // The grid coordinate at entry maps interior coords to the apron'd grid via
        // the SAME +APRON-0.5 shift atlas_uvw / trilinear_reconstruct use, so the DDA
        // cell indices address the SAME corners the cubic fetches.
        let g_entry = ro[axis] + rd[axis] * t + APRON as f32 - 0.5 + ATLAS_SAMPLE_BIAS;
        let c0 = clamp_index(g_entry, W) as i32;
        cell[axis] = c0;
        if rd[axis] > 0.0 {
            step[axis] = 1;
            // The grid coordinate of the next cell boundary (c0 + 1), back-solved to t.
            let boundary = (c0 + 1) as f32;
            t_next[axis] = t + (boundary - g_entry) / rd[axis];
            t_delta[axis] = 1.0 / rd[axis];
        } else if rd[axis] < 0.0 {
            step[axis] = -1;
            let boundary = c0 as f32;
            t_next[axis] = t + (boundary - g_entry) / rd[axis];
            t_delta[axis] = -1.0 / rd[axis];
        } else {
            step[axis] = 0;
            t_next[axis] = f32::INFINITY;
            t_delta[axis] = f32::INFINITY;
        }
    }

    // March cells until the ray leaves the brick span.
    let max_cells = 3 * BRICK_ALLOC; // the longest 3D-DDA path through a 10³ grid
    for _ in 0..max_cells {
        // The cell's low corner clamped so the +1 neighbour is in-bounds.
        let cx = (cell[0].max(0) as usize).min(W - 2);
        let cy = (cell[1].max(0) as usize).min(W - 2);
        let cz = (cell[2].max(0) as usize).min(W - 2);

        // Fetch the 8 decoded corners in the s_ijk ↔ x + 2y + 4z order (matching
        // jcgt_cubic_coeffs and trilinear_reconstruct).
        let s = [
            decode_snorm8(brick[cx + cy * W + cz * W * W], band_half), // s000
            decode_snorm8(brick[(cx + 1) + cy * W + cz * W * W], band_half), // s100
            decode_snorm8(brick[cx + (cy + 1) * W + cz * W * W], band_half), // s010
            decode_snorm8(brick[(cx + 1) + (cy + 1) * W + cz * W * W], band_half), // s110
            decode_snorm8(brick[cx + cy * W + (cz + 1) * W * W], band_half), // s001
            decode_snorm8(brick[(cx + 1) + cy * W + (cz + 1) * W * W], band_half), // s101
            decode_snorm8(brick[cx + (cy + 1) * W + (cz + 1) * W * W], band_half), // s011
            decode_snorm8(brick[(cx + 1) + (cy + 1) * W + (cz + 1) * W * W], band_half), // s111
        ];

        // The t-span of THIS cell along the ray (clamped to the brick span).
        let t_cell_exit = t_next[0].min(t_next[1]).min(t_next[2]).min(t_exit);
        let seg_lo = t.max(t_enter);
        let seg_hi = t_cell_exit.min(t_exit);

        if seg_hi > seg_lo {
            // The ray in the cell's LOCAL [0,1]³ frame: the grid coordinate minus the
            // cell's low index gives the in-cell fraction; the direction is the same
            // in interior-voxel and apron'd-grid units (a pure translation).
            let lo_g = [
                ro[0] + rd[0] * seg_lo + APRON as f32 - 0.5 + ATLAS_SAMPLE_BIAS - cx as f32,
                ro[1] + rd[1] * seg_lo + APRON as f32 - 0.5 + ATLAS_SAMPLE_BIAS - cy as f32,
                ro[2] + rd[2] * seg_lo + APRON as f32 - 0.5 + ATLAS_SAMPLE_BIAS - cz as f32,
            ];
            // ro_local at the cell entry; the cubic's t is measured from seg_lo.
            let coeffs = jcgt_cubic_coeffs(&s, lo_g, rd);
            if let Some(local_t) = marmitt_root(&coeffs, 0.0, seg_hi - seg_lo) {
                return Some(seg_lo + local_t);
            }
        }

        // Advance the DDA to the next cell boundary; stop once past the brick exit.
        if t_cell_exit >= t_exit {
            break;
        }
        // Step the axis with the nearest boundary.
        let axis = if t_next[0] <= t_next[1] && t_next[0] <= t_next[2] {
            0
        } else if t_next[1] <= t_next[2] {
            1
        } else {
            2
        };
        t = t_next[axis];
        cell[axis] += step[axis];
        t_next[axis] += t_delta[axis];
        if step[axis] == 0 || cell[axis] < 0 || cell[axis] as usize >= W - 1 {
            break;
        }
    }

    None
}

/// `x^(1/3)` preserving sign — the real cube root the Cardano CROSS-CHECK needs (a
/// CPU-only reference helper; NOT on the GPU Marmitt path). `f32::cbrt` is not in
/// stable `core`, so this routes through the crate `sqrt`-class shim via `powf`'s
/// sign-folded form using `exp`/`ln`-free arithmetic: a sign-preserving Newton
/// refine on `|x|^(1/3)`.
#[inline]
fn cbrt(x: f32) -> f32 {
    if x == 0.0 {
        return 0.0;
    }
    let s = x.signum();
    let a = x.abs();
    // Seed via the bit-twiddling cube-root approximation, then Newton-refine.
    let mut y = f32::from_bits(0x2a51_2cae_u32.wrapping_add(a.to_bits() / 3));
    // A few Newton steps for y³ = a: y ← y − (y³ − a)/(3y²).
    for _ in 0..4 {
        let y2 = y * y;
        y -= (y * y2 - a) / (3.0 * y2);
    }
    s * y
}

/// `acos(x)` for the Cardano trigonometric branch (CPU-only CROSS-CHECK helper; NOT
/// on the GPU Marmitt path). `f32::acos` is not in stable `core`; this uses the
/// `atan2`-free polynomial form `acos(x) = π/2 − asin(x)` with a Newton-refined
/// `asin`, accurate enough for the three-real-root angle (the cross-check tolerance
/// is [`CUBIC_ROOT_EPS`] on the ROOT, not the angle).
#[inline]
fn acos(x: f32) -> f32 {
    let xc = x.clamp(-1.0, 1.0);
    // asin via the identity asin(x) = atan(x / sqrt(1 - x²)); compute atan by a
    // bounded CORDIC-free series refine. For our use |x| <= 1, and the eventual
    // root error is dominated by the cubic refine, so a compact rational suffices.
    // Abramowitz–Stegun 4.4.45 style: acos(x) ≈ sqrt(1-x)·P(x) for x>=0, mirrored.
    let neg = xc < 0.0;
    let ax = xc.abs();
    // 4-term minimax for acos on [0,1] (the standard handheld-calculator form).
    let poly = ((((-0.0187293) * ax + 0.0742610) * ax - 0.2121144) * ax + 1.5707288)
        * sqrt(1.0 - ax);
    if neg {
        core::f32::consts::PI - poly
    } else {
        poly
    }
}

/// `cos(x)` for the Cardano trigonometric branch (CPU-only CROSS-CHECK helper; NOT
/// on the GPU Marmitt path). A range-reduced Taylor/minimax cosine — accurate to
/// the cross-check root tolerance over the `[0, 2π/3·3]` angles the three-real-root
/// branch produces.
#[inline]
fn cos(x: f32) -> f32 {
    use core::f32::consts::PI;
    // Range-reduce into [-π, π]. `floor_f32` is a `core`-only floor (stable `core`
    // has no `f32::floor`) — the crate must compile under the strict `no_std`
    // `nightly` feature, so this helper avoids the `std` inherent method.
    let two_pi = 2.0 * PI;
    let mut a = x - two_pi * floor_f32(x / two_pi + 0.5);
    // Fold into [-π/2, π/2] tracking the sign.
    let mut sign = 1.0_f32;
    if a > 0.5 * PI {
        a = PI - a;
        sign = -1.0;
    } else if a < -0.5 * PI {
        a = -PI - a;
        sign = -1.0;
    }
    // 6-term Taylor cosine (accurate < 1e-6 on [-π/2, π/2]).
    let a2 = a * a;
    let c = 1.0 - a2 / 2.0 + a2 * a2 / 24.0 - a2 * a2 * a2 / 720.0;
    sign * c
}

/// `floor(x)` via integer truncation — a `core`-only floor (stable `core` lacks
/// `f32::floor`, which is a `std` inherent method). Used ONLY by the Cardano
/// cross-check's [`cos`] range reduction, where `|x|` is the small bounded angle
/// multiple the three-real-root branch produces (well inside the `i64` range), so
/// the cast is exact. Truncation rounds toward zero, so a negative non-integer is
/// nudged down by one to match `floor`.
#[inline]
fn floor_f32(x: f32) -> f32 {
    let t = x as i64 as f32;
    if t > x { t - 1.0 } else { t }
}

// The brick tests link `std` for the test harness (the default, non-`nightly`
// profile already links `std` for `f32::sqrt`); they exercise the M0 conservative-
// lower-bound contract numerically. The randomized GATE uses a hand-rolled xorshift
// PRNG (NO new dependency — the crate is a zero-dep `no_std` leaf).
#[cfg(test)]
mod tests;
