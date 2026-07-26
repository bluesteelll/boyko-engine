//! **The SV0 rung-S1 adequacy instrument** (`docs/VB-SV0-SDF-SHADOW-PLAN.md`, "S1 — the fixture")
//! — a CPU rasterizer, the two SV0 adequacy predicates evaluated against the host field oracle,
//! and the changed-covered-pixel comparator rung S4(ii) consumes.
//!
//! # Why this exists at all
//!
//! Every VB golden shipped before this campaign has an EMPTY SDF edit list, so SV0's shadow and
//! contact-AO terms are both exactly `1.0` on them and any byte-identity gate quantified over
//! those pins is VACUOUS. S1's job is to prove — on the CPU, with no GPU — that the
//! `vb_both_sdf` / `vb_both_sdf_tex` fixtures carry enough pixels that SV0 actually darkens for
//! the downstream gates to mean something. Nothing in the tree computed raster coverage
//! host-side (`goldens.rs`'s host mirrors take a mesh depth buffer as an INPUT), so this module
//! lands it.
//!
//! # One instrument, two consumers
//!
//! The adequacy counts and the S4(ii) changed-pixel fraction are quantified over the SAME
//! selection ([`MeshSelection`]). That is deliberate: a comparator with its own notion of "which
//! pixels count" could report a healthy changed-fraction over a set of pixels the adequacy gate
//! never certified. Building both on one denominator makes that divergence unrepresentable.
//!
//! # Deliberately dependency-light
//!
//! This module names NO engine scene / render / ECS TYPE — only `[f32; N]` arrays,
//! [`boyko_sdf_math`] (the frozen field leaf), [`boyko_shaderdsl`] (the shipped shadow leaf's
//! single source) and five `f32` CONSTANTS re-exported from `boyko_rhi_vulkan::compute` (the host
//! mirrors that already carry the shader-sync discipline — see the constants section). The camera,
//! the mesh and the edit list arrive as plain data from the caller, which is what lets
//! `sv0_adequacy.rs` feed it the fixtures' OWN geometry via the engine's own `ViewUniform` /
//! `forward_view_proj_rows` construction sites instead of a re-derivation.

// The S4(ii) comparator half is landed here for a later rung to call, so the S1 test binary does
// not reference every item. Same reason `tests/common/mod.rs` carries the allow.
#![allow(dead_code)]

use std::cell::Cell;
use std::path::Path;

use boyko_sdf_math::{SdfEdit, sdf_edit_list};
use boyko_shaderdsl::EvalCf;
use boyko_shaderdsl::shadow::sdf_soft_shadow_body;

// ===========================================================================================
// The frozen shader constants this oracle mirrors
//
// The five names the host ALREADY exports are IMPORTED, never re-declared (review W3). They live
// in `boyko_rhi_vulkan::compute` — a normal (not dev-, not feature-gated) dependency of this
// crate — and are the mirrors that already carry the shader-sync discipline: retuning `AO_STEP`
// in `compute.rs` + the HLSL must not leave this oracle silently modelling the old leaf. Only the
// three values with NO host export are declared locally, each saying so.
//
// The march schedule (`SHADOW_MINT`, `SHADOW_K`, `SHADOW_HIT_EPS`, `FIELD_LIPSCHITZ_L`,
// `SDF_T_MAX`) is deliberately absent from BOTH lists: the eDSL body is called directly, so the
// shader's own schedule is executed rather than mirrored.
// ===========================================================================================

/// The shadow march's normal-offset origin lift — `boyko_rhi_vulkan::compute`'s mirror of
/// `sdf_gbuffer_composite.hlsl:478`'s `SHADOW_NORMAL_BIAS` (and `deferred_pbr.hlsl:474`'s
/// identical literal).
///
/// Applied along the GEOMETRIC face normal, per the S1 gate's wording (`P + face_N *
/// SHADOW_NORMAL_BIAS`). The SHIPPED mesh site (`sdf_gbuffer_composite.hlsl:1877-1878`) lifts
/// along the SHADING normal instead; SV0 introduces the face normal for exactly this bias
/// (plan §4.2/§4.3). On this fixture the two differ by at most the tessellation's
/// shading/geometric normal split (≈3° at 28×40), i.e. under `2e-5` of world offset — far below
/// the `0.25` surface-to-surface gap either predicate turns on.
pub use boyko_rhi_vulkan::compute::SHADOW_NORMAL_BIAS;

/// The signed `n·L` grazing / back-face cutoff — `boyko_rhi_vulkan::compute`'s mirror of
/// `sdf_gbuffer_composite.hlsl:477`.
///
/// `sdf_soft_shadow` returns `0.0` at its early-out when `dot(n, L) <= SHADOW_NDOTL_EPS`, for a
/// reason that has NOTHING to do with the field. Counting those pixels as "shadowed by the SDF
/// body" would make the adequacy gate pass on a scene with no occluder at all, so the predicate
/// excludes them explicitly rather than letting the leaf's own return value speak.
pub use boyko_rhi_vulkan::compute::SHADOW_NDOTL_EPS;

/// The AO probe's step between taps — `boyko_rhi_vulkan::compute`'s mirror of
/// `sdf_gbuffer_composite.hlsl:488`'s `AO_STEP`.
pub use boyko_rhi_vulkan::compute::AO_STEP;

/// The AO accumulator's per-tap geometric weight — `boyko_rhi_vulkan::compute`'s mirror of
/// `sdf_gbuffer_composite.hlsl:489`'s `AO_FALLOFF`. The `i`-th tap's field deficit is weighted
/// `AO_FALLOFF^i`, which is what makes far taps contribute NEGATIVE terms that cancel near ones —
/// the fact [`has_contact_ao`] exists to respect.
pub use boyko_rhi_vulkan::compute::AO_FALLOFF;

/// The AO accumulator's overall strength — `boyko_rhi_vulkan::compute`'s mirror of
/// `sdf_gbuffer_composite.hlsl:490`'s `AO_STRENGTH`.
pub use boyko_rhi_vulkan::compute::AO_STRENGTH;

/// The AO probe's tap count — `sdf_ao`'s `for (uint i = 1u; i <= 5u; ++i)`
/// (`sdf_gbuffer_composite.hlsl:535`).
///
/// Declared locally because the loop bound is the ONLY one of the six `sdf_ao` inputs the host
/// does not export as a constant: `goldens.rs::host_ao` hardcodes the same `1..=5u32` literal.
///
/// The taps reach `AO_TAPS * AO_STEP` (`0.5`) along the shading normal, so the probe cannot SEE
/// geometry past a `1.0` surface-to-surface gap (the far tap sits `0.5` out and reads `0.5` of
/// clearance). But "a tap sees something" is NOT "the leaf darkens": the accumulation's negative
/// terms push the darkening boundary in to a `0.579506` gap — see [`sdf_ao`].
pub const AO_TAPS: u32 = 5;

/// The primary marcher's surface-hit threshold — `sdf_gbuffer_composite.hlsl:440`'s `EPS`.
/// Used ONLY by [`sdf_occludes_eye_ray`], the SDF-ownership exclusion.
///
/// Declared locally: the composite marcher's `EPS` / `MAX_IT` have no `compute.rs` export (its
/// `EPS_COARSE` / `MAX_IT_COARSE` are the COARSE-pass budget, a different pair of numbers).
pub const MARCHER_EPS: f32 = 0.001;

/// The primary marcher's step ceiling — `sdf_gbuffer_composite.hlsl:442`'s `MAX_IT`. Used ONLY
/// by [`sdf_occludes_eye_ray`]. Locally declared for the same reason as [`MARCHER_EPS`].
pub const MARCHER_MAX_IT: u32 = 128;

// ===========================================================================================
// Minimal `[f32; 3]` vocabulary
//
// Deliberately local rather than `boyko_math::Vec3`: every value in this module crosses into
// `boyko_sdf_math` / `boyko_shaderdsl`, both of which speak `[f32; 3]`, so a SIMD-aligned
// vector type would only add a conversion at every call.
// ===========================================================================================

/// Component-wise sum.
#[inline]
fn v_add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// Component-wise difference `a - b`.
#[inline]
fn v_sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Scalar product.
#[inline]
fn v_mul(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// Dot product.
#[inline]
fn v_dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Cross product `a × b`.
#[inline]
fn v_cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Euclidean length.
#[inline]
fn v_len(a: [f32; 3]) -> f32 {
    v_dot(a, a).sqrt()
}

/// Unit-normalizes `a`, returning the ZERO vector for a degenerate input rather than `NaN` —
/// the same zero-guard [`boyko_sdf_math`]'s `v_normalize` carries, and for the same reason: a
/// `NaN` normal would silently pass every `>` comparison downstream instead of failing loudly.
#[inline]
fn v_normalize(a: [f32; 3]) -> [f32; 3] {
    let len_sq = v_dot(a, a);
    if len_sq <= 0.0 {
        return [0.0, 0.0, 0.0];
    }
    v_mul(a, len_sq.sqrt().recip())
}

// ===========================================================================================
// The CPU coverage rasterizer
// ===========================================================================================

/// One mesh vertex as the rasterizer consumes it — the caller strips `boyko_render::mesh::Vertex`
/// down to the two lanes coverage needs (colour / uv / tangent play no part in either predicate).
#[derive(Clone, Copy, Debug)]
pub struct OracleVertex {
    /// Model-space position.
    pub position: [f32; 3],
    /// Model-space outward normal — the SHADING normal both leaves consume, since the fixtures'
    /// instances are pure translations (no rotation, no scale) and the raster interpolates the
    /// vertex normal into `gNormal`.
    pub normal: [f32; 3],
}

/// Everything the two predicates need about one raster-covered pixel.
#[derive(Clone, Copy, Debug)]
pub struct CoveredPixel {
    /// Perspective-correct world-space surface point — the `P_mesh` of
    /// `sdf_gbuffer_composite.hlsl:1866`.
    pub world_pos: [f32; 3],
    /// Perspective-correct, re-normalized interpolated vertex normal — the `N_mesh` the raster
    /// writes into `gNormal` and both leaves read back.
    pub shading_normal: [f32; 3],
    /// The triangle's GEOMETRIC normal, oriented outward (agreeing with the vertex normals).
    /// Carries the `SHADOW_NORMAL_BIAS` lift, per the S1 gate's `P + face_N * …` wording.
    pub face_normal: [f32; 3],
    /// View-space depth `dot(forward, P) − dot(forward, eye)` — the projection's `clip.w`. The
    /// depth test keeps the SMALLEST, which is nearest under both the Deferred `clip.z == clip.w`
    /// convention and Forward/VB's reverse-Z (the two differ by a monotone remap of this value,
    /// so coverage is invariant under the choice).
    pub view_z: f32,
}

/// The rasterized frame: which pixels a mesh covers, and what the shading leaves would see there.
pub struct Coverage {
    /// Raster width in pixels.
    pub width: u32,
    /// Raster height in pixels.
    pub height: u32,
    /// Triangles rejected because a vertex fell at or in front of the near plane. Exposed rather
    /// than swallowed: this rasterizer does NOT clip polygons, so a non-zero count means part of
    /// the scene was silently dropped and every pixel count below is a lower bound of unknown
    /// depth. The S1 gate asserts it is zero.
    pub near_rejected_triangles: usize,
    /// Row-major `width * height`, indexed `y * width + x` with `y` increasing DOWNWARD (the
    /// screen convention the projection's y-flip already establishes).
    pixels: Vec<Option<CoveredPixel>>,
}

impl Coverage {
    /// The covered pixel at `(x, y)`, or `None` where no mesh triangle won the depth test.
    ///
    /// # Panics
    ///
    /// Panics when `(x, y)` is outside the raster — every caller iterates the raster's own
    /// extent, so an out-of-range probe is a caller bug, not a runtime condition.
    #[inline]
    pub fn get(&self, x: u32, y: u32) -> Option<&CoveredPixel> {
        assert!(
            x < self.width && y < self.height,
            "invariant: ({x},{y}) is outside the {}x{} raster",
            self.width,
            self.height
        );
        self.pixels[(y * self.width + x) as usize].as_ref()
    }

    /// The covered pixel at flat index `i` (`y * width + x`), or `None`.
    #[inline]
    pub fn at(&self, i: usize) -> Option<&CoveredPixel> {
        self.pixels[i].as_ref()
    }

    /// How many pixels a mesh covers — the raw raster count, BEFORE the SDF-ownership exclusion
    /// [`MeshSelection`] applies.
    #[inline]
    pub fn covered_count(&self) -> usize {
        self.pixels.iter().filter(|p| p.is_some()).count()
    }
}

/// Rasterizes `instances` of one indexed triangle mesh under `view_proj_rows`, producing the
/// world position + shading normal + face normal of the nearest surface at every pixel.
///
/// `view_proj_rows` is a ROW-major `proj·view` exactly as `boyko_render::forward_view_proj_rows`
/// / `marcher_view_proj_rows` build it: row 0 is `clip.x`, row 1 is `clip.y` (already carrying
/// the marcher y-flip, so screen `y` grows downward), row 3 is `clip.w == view_z`. Row 2 is
/// UNUSED — the depth test runs on `view_z`, which is monotone in both paths' depth encodes.
///
/// `instances` are pure TRANSLATIONS, which is what the fixtures spawn (`MeshBundle::new` with
/// `Transform::from_translation`); a rotated or scaled instance would need its normals
/// transformed by the inverse-transpose and is deliberately not supported rather than silently
/// mishandled.
///
/// Interpolation is perspective-correct (attributes divided by `clip.w`, recovered through the
/// interpolated `1/w`) — the same thing the hardware does, so `world_pos` lands on the triangle
/// plane along the pixel's eye ray rather than on a screen-space chord.
///
/// # Panics
///
/// Panics on a zero-sized raster or an index list whose length is not a multiple of three —
/// both are fixture construction errors.
pub fn rasterize(
    vertices: &[OracleVertex],
    indices: &[u32],
    instances: &[[f32; 3]],
    view_proj_rows: [[f32; 4]; 4],
    width: u32,
    height: u32,
    near: f32,
) -> Coverage {
    assert!(width > 0 && height > 0, "invariant: the oracle raster extent is non-zero");
    assert!(
        indices.len().is_multiple_of(3),
        "invariant: the index list is a triangle list ({} indices)",
        indices.len()
    );

    let count = (width as usize) * (height as usize);
    let mut pixels: Vec<Option<CoveredPixel>> = vec![None; count];
    let mut depth = vec![f32::INFINITY; count];
    let mut near_rejected_triangles = 0usize;

    let row_x = view_proj_rows[0];
    let row_y = view_proj_rows[1];
    let row_w = view_proj_rows[3];
    let project = |p: [f32; 3], row: [f32; 4]| row[0] * p[0] + row[1] * p[1] + row[2] * p[2] + row[3];

    let fw = width as f32;
    let fh = height as f32;

    for &translation in instances {
        for tri in indices.chunks_exact(3) {
            // Instance transform: a pure translation moves positions and leaves normals alone.
            let world: [[f32; 3]; 3] = [
                v_add(vertices[tri[0] as usize].position, translation),
                v_add(vertices[tri[1] as usize].position, translation),
                v_add(vertices[tri[2] as usize].position, translation),
            ];
            let normals: [[f32; 3]; 3] = [
                vertices[tri[0] as usize].normal,
                vertices[tri[1] as usize].normal,
                vertices[tri[2] as usize].normal,
            ];

            let w: [f32; 3] = [
                project(world[0], row_w),
                project(world[1], row_w),
                project(world[2], row_w),
            ];
            if w[0] <= near || w[1] <= near || w[2] <= near {
                // No polygon clipping: a straddling triangle is dropped whole and COUNTED, so the
                // caller can tell "nothing crossed the near plane" from "coverage silently lost".
                near_rejected_triangles += 1;
                continue;
            }

            // Screen space: NDC = clip.xy / clip.w, then the standard `[-1,1] -> [0,extent]` map.
            let mut sx = [0.0f32; 3];
            let mut sy = [0.0f32; 3];
            for k in 0..3 {
                let ndc_x = project(world[k], row_x) / w[k];
                let ndc_y = project(world[k], row_y) / w[k];
                sx[k] = (ndc_x * 0.5 + 0.5) * fw;
                sy[k] = (ndc_y * 0.5 + 0.5) * fh;
            }

            let area = (sx[1] - sx[0]) * (sy[2] - sy[0]) - (sy[1] - sy[0]) * (sx[2] - sx[0]);
            if area == 0.0 {
                // Zero screen area — includes `uv_sphere`'s degenerate pole-fan triangles, which
                // are real entries in the fixtures' index buffer and cover nothing.
                continue;
            }
            // Fold the winding into the barycentric sign so one `>= 0` test covers both windings
            // (no back-face cull: a closed sphere's far side always loses the depth test, and
            // culling would need the raster pipeline's own front-face convention as an input).
            let inv_area = area.recip();

            // The GEOMETRIC normal, oriented to agree with the vertex normals — `uv_sphere` emits
            // outward vertex normals by construction, so their sum is an unambiguous "outward".
            let mut face_normal =
                v_normalize(v_cross(v_sub(world[1], world[0]), v_sub(world[2], world[0])));
            let outward = v_add(v_add(normals[0], normals[1]), normals[2]);
            if v_dot(face_normal, outward) < 0.0 {
                face_normal = v_mul(face_normal, -1.0);
            }

            let min_x = sx.iter().fold(f32::INFINITY, |a, &b| a.min(b)).floor().max(0.0) as u32;
            let max_x =
                (sx.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b)).ceil()).min(fw) as u32;
            let min_y = sy.iter().fold(f32::INFINITY, |a, &b| a.min(b)).floor().max(0.0) as u32;
            let max_y =
                (sy.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b)).ceil()).min(fh) as u32;

            let inv_w = [w[0].recip(), w[1].recip(), w[2].recip()];

            for py in min_y..max_y {
                let cy = py as f32 + 0.5;
                for px in min_x..max_x {
                    let cx = px as f32 + 0.5;

                    // Edge functions against each opposite edge, scaled by `1/area` so the sign
                    // convention follows the winding automatically.
                    let b0 = ((sx[1] - cx) * (sy[2] - cy) - (sy[1] - cy) * (sx[2] - cx)) * inv_area;
                    let b1 = ((sx[2] - cx) * (sy[0] - cy) - (sy[2] - cy) * (sx[0] - cx)) * inv_area;
                    let b2 = ((sx[0] - cx) * (sy[1] - cy) - (sy[0] - cy) * (sx[1] - cx)) * inv_area;
                    if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 {
                        continue;
                    }

                    // Perspective-correct recovery: `1/w` interpolates linearly in screen space.
                    let inv_denom = b0 * inv_w[0] + b1 * inv_w[1] + b2 * inv_w[2];
                    if inv_denom <= 0.0 {
                        continue;
                    }
                    let view_z = inv_denom.recip();

                    let i = (py * width + px) as usize;
                    if view_z >= depth[i] {
                        continue;
                    }

                    let pw = [b0 * inv_w[0] * view_z, b1 * inv_w[1] * view_z, b2 * inv_w[2] * view_z];
                    let mut world_pos = [0.0f32; 3];
                    let mut shading_normal = [0.0f32; 3];
                    for k in 0..3 {
                        world_pos = v_add(world_pos, v_mul(world[k], pw[k]));
                        shading_normal = v_add(shading_normal, v_mul(normals[k], pw[k]));
                    }

                    depth[i] = view_z;
                    pixels[i] = Some(CoveredPixel {
                        world_pos,
                        shading_normal: v_normalize(shading_normal),
                        face_normal,
                        view_z,
                    });
                }
            }
        }
    }

    Coverage { width, height, near_rejected_triangles, pixels }
}

// ===========================================================================================
// The selection: mesh pixels SV0 can actually shade
// ===========================================================================================

/// The pixels every S1 count and the S4(ii) comparator are quantified over: raster-covered mesh
/// pixels that the SDF leg does NOT own.
///
/// The exclusion matters because under `legs: Both` a non-empty edit list lets
/// `sdf_forward_march` composite SDF-owned pixels into `gLit` INDEPENDENTLY of SV0. A mesh pixel
/// hidden behind the SDF body is raster-covered but never carries an SV0 term, so counting it
/// would inflate the adequacy numbers with pixels the frame does not show.
///
/// # Which way this can be wrong
///
/// Excluding a pixel can only LOWER a count — that direction is safe. FAILING to exclude one
/// (which [`sdf_occludes_eye_ray`] can do, see its own doc) raises it, which is not. That is why
/// [`Self::sdf_occluded`] is reported rather than swallowed and why the S1 gate asserts it is
/// exactly `0` at the shipped placement: on a fixture where the exclusion never fires, its
/// direction of error cannot matter.
pub struct MeshSelection {
    /// Raster width in pixels (mirrors the [`Coverage`] it was built from).
    pub width: u32,
    /// Raster height in pixels.
    pub height: u32,
    /// Flat pixel indices (`y * width + x`), ascending.
    pub indices: Vec<u32>,
    /// How many raster-covered pixels the SDF leg took ownership of — reported so a fixture whose
    /// SDF body has drifted in front of the mesh is visible rather than silently shrinking the
    /// denominator.
    pub sdf_occluded: usize,
}

impl MeshSelection {
    /// The size of the selection — the denominator both consumers divide by.
    #[inline]
    pub fn len(&self) -> usize {
        self.indices.len()
    }

    /// Whether the selection is empty (the vacuity condition every S1 gate exists to refute).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// Builds the [`MeshSelection`] for `coverage` under `edits`, seen from `eye`.
///
/// A covered pixel is dropped when the SDF field occludes the segment from `eye` to its surface
/// point (see [`sdf_occludes_eye_ray`]). The result depends on the edit list, so the S1 mutation
/// test rebuilds it per placement rather than reusing one selection across placements.
pub fn select_mesh_pixels(coverage: &Coverage, edits: &[SdfEdit], eye: [f32; 3]) -> MeshSelection {
    let mut indices = Vec::new();
    let mut sdf_occluded = 0usize;
    for i in 0..(coverage.width as usize) * (coverage.height as usize) {
        let Some(px) = coverage.at(i) else { continue };
        if sdf_occludes_eye_ray(edits, eye, px.world_pos) {
            sdf_occluded += 1;
            continue;
        }
        indices.push(i as u32);
    }
    MeshSelection { width: coverage.width, height: coverage.height, indices, sdf_occluded }
}

// ===========================================================================================
// The field seam + the two adequacy predicates
// ===========================================================================================

/// The host field oracle — `boyko_sdf_math::sdf_edit_list`, which delegates to
/// `boyko_shaderdsl::field::sdf_field_body::<f32>`, the SAME single source the shader's
/// `field_distance` (`sdf_field.hlsli:246`) is emitted from.
#[inline]
pub fn field_distance(edits: &[SdfEdit], p: [f32; 3]) -> f32 {
    sdf_edit_list(edits, p)
}

/// Runs the SHIPPED soft-shadow leaf on the host: `boyko_shaderdsl::shadow::sdf_soft_shadow_body`
/// instantiated over `EvalCf`, threading [`field_distance`] as the field seam.
///
/// This is the generator that emits the committed `sdf_soft_shadow` HLSL span between its
/// `// === GENERATED sdf_soft_shadow BEGIN/END ===` sentinels, so the march schedule
/// (`SHADOW_MINT` start, the `d / FIELD_LIPSCHITZ_L` step floored at `SHADOW_MINT_STEP`, the
/// `t > T_MAX` break, the `d < SHADOW_HIT_EPS` early return) is not re-derived here — it IS the
/// shader's. Returns visibility in `[0, 1]`.
///
/// `n` is carried for signature parity only: the generated span never reads it (the `dot(n, L)`
/// early-out is the shader's hand-written preamble, which [`is_fully_shadowed`] applies itself).
pub fn shadow_visibility(edits: &[SdfEdit], origin: [f32; 3], n: [f32; 3], l: [f32; 3]) -> f32 {
    let out = Cell::new(0.0f32);
    let field = |q: [f32; 3]| field_distance(edits, q);
    sdf_soft_shadow_body::<EvalCf, _>(origin, n, l, field, &out);
    out.get()
}

/// **S1 gate 2's predicate.** Whether `pixel` is front-facing to the key light AND the shadow
/// march from `P + face_N * SHADOW_NORMAL_BIAS` along `l` hits the field.
///
/// # This is SUFFICIENT, NOT EQUIVALENT to "SV0 darkens this pixel" — and it undercounts
///
/// The Quilez accumulator drops below `1.0` as soon as `SHADOW_K * d / t < 1` at ANY step, so the
/// set of pixels the term actually darkens is strictly LARGER than the set this predicate counts.
/// What is counted here is the HARD-HIT early-out (`d < SHADOW_HIT_EPS` returning `0.0`), i.e.
/// FULLY-occluded pixels only. The gate therefore errs safe — false red, never false green — but
/// a floor read as if it measured "darkened pixels" would be reading the wrong quantity.
///
/// # Why testing the leaf's return against `0.0` is exactly the hard-hit test
///
/// The leaf returns `0.0` from two places: the `d < SHADOW_HIT_EPS` early-out, and the tail
/// `clamp(res, 0, 1)` when `res <= 0`. But `res` is a running `min` of `SHADOW_K * d / t` with
/// `SHADOW_K > 0` and `t > 0`, so `res <= 0` requires some step's `d <= 0 < SHADOW_HIT_EPS` — and
/// the hit test on that very step fires first. The two conditions coincide, so `== 0.0` is the
/// hard-hit predicate with no re-implementation of the march.
pub fn is_fully_shadowed(edits: &[SdfEdit], pixel: &CoveredPixel, l: [f32; 3]) -> bool {
    // The shader's hand-written preamble (`sdf_gbuffer_composite.hlsl:499-501`), applied here
    // rather than read off the leaf's return: its `0.0` on a back-face is not a field result.
    if v_dot(pixel.shading_normal, l) <= SHADOW_NDOTL_EPS {
        return false;
    }
    let origin = v_add(pixel.world_pos, v_mul(pixel.face_normal, SHADOW_NORMAL_BIAS));
    shadow_visibility(edits, origin, pixel.shading_normal, l) == 0.0
}

/// The SHIPPED `sdf_ao` leaf on the host — `sdf_gbuffer_composite.hlsl:532-541` transcribed term
/// for term, with [`field_distance`] as the field seam. Returns an occlusion factor in `[0, 1]`
/// (`1.0` = unoccluded, SV0's no-op).
///
/// ```text
/// occ = Σ_{i=1..AO_TAPS} (h_i − d_i) · AO_FALLOFF^i ,  h_i = i·AO_STEP , d_i = field(p + n·h_i)
/// return clamp(1 − AO_STRENGTH·occ, 0, 1)
/// ```
///
/// # Why the accumulation is reproduced instead of a "does any tap see the body" shortcut
///
/// Because the shortcut is FALSE-GREEN, which is the direction that matters. Taps with `d_i > h_i`
/// contribute NEGATIVE terms, so "some tap has `d < h`" does **not** imply the leaf darkens
/// anything. On-axis with a surface-to-surface gap `g` (so `d_i = g − h_i`) the closed form is
/// `occ = 2·Σ h_i·AO_FALLOFF^i − g·Σ AO_FALLOFF^i = 2.490811 − g·4.298162`, i.e. the leaf darkens
/// iff `g < 0.579506` — while the shortcut fires all the way out to `g < 2·AO_TAPS·AO_STEP = 1.0`.
/// Every pixel in `0.58 < g < 1.0` would be counted WITHOUT being darkened at all, inflating the
/// very number [`contact_ao_pixel_count`]'s floor is compared against.
///
/// # Relationship to `goldens.rs::host_ao`
///
/// `boyko_rhi_vulkan::goldens::host_ao` is the same accumulation and is pinned against the GPU
/// (within ±3/255) by the marcher goldens — but it is `pub(crate)` inside a module gated on
/// `#[cfg(any(test, feature = "goldens"))]`, so reaching it from here would mean turning that
/// feature on for every `--all-targets` build of this crate to borrow six lines. The three tuning
/// constants — the values that can actually drift — ARE imported from the single host source
/// (`compute.rs`), which is where the sync discipline lives; `powi` matches `host_ao`'s own choice
/// so the two hosts agree bit-for-bit.
pub fn sdf_ao(edits: &[SdfEdit], p: [f32; 3], n: [f32; 3]) -> f32 {
    let mut occ = 0.0f32;
    for i in 1..=AO_TAPS {
        let h = i as f32 * AO_STEP;
        let d = field_distance(edits, v_add(p, v_mul(n, h)));
        occ += (h - d) * AO_FALLOFF.powi(i as i32);
    }
    (1.0 - AO_STRENGTH * occ).clamp(0.0, 1.0)
}

/// **S1 gate 3's predicate.** Whether the shipped [`sdf_ao`] leaf actually DARKENS `pixel` — its
/// value is below the far-field `1.0` at which SV0's AO term is a no-op.
///
/// This is `occ > 0` on the real accumulation, not "some tap sees the body"; see [`sdf_ao`] for why
/// the difference is a false-GREEN of roughly 2.6× on this fixture.
///
/// Unlike [`is_fully_shadowed`], this predicate is EXACT rather than conservative: `sdf_ao < 1.0`
/// is precisely the condition under which the shipped leaf multiplies something other than `1.0`
/// into the pixel. It neither over- nor under-counts.
///
/// Note the origin is the UNBIASED surface point, matching the shipped call site's
/// `sdf_ao(P_mesh, N_mesh)` (`:1881`) — the AO probe takes no normal-offset lift.
pub fn has_contact_ao(edits: &[SdfEdit], pixel: &CoveredPixel) -> bool {
    sdf_ao(edits, pixel.world_pos, pixel.shading_normal) < 1.0
}

/// Whether the SDF field occludes the eye→surface segment, i.e. whether the SDF leg owns this
/// pixel instead of the raster.
///
/// A plain sphere trace along the exact `eye → p` direction. It is NOT a mirror of the marcher's
/// SOR/retreat schedule and does not try to be. On an exact field (which the analytic edit list is)
/// a plain sphere trace never steps THROUGH a surface, so it cannot tunnel past an occluder. The
/// ray direction needs no ray-gen replication — it is the direction to the already-known surface
/// point.
///
/// # The one way it can under-report, stated plainly
///
/// Exhausting [`MARCHER_MAX_IT`] returns `false`. A near-tangential ray that grazes the body can
/// burn its whole step budget on tiny advances and fall out of the loop un-occluded — a MISSED
/// exclusion, which RAISES the count, the unsafe direction. So this is "conservative" only against
/// tunnelling, not against step-budget exhaustion, and the guard against it is empirical: the S1
/// gate asserts [`MeshSelection::sdf_occluded`] is exactly `0` on the shipped fixture, i.e. this
/// function never fires there at all and neither of its error directions is live.
pub fn sdf_occludes_eye_ray(edits: &[SdfEdit], eye: [f32; 3], p: [f32; 3]) -> bool {
    let seg = v_sub(p, eye);
    let seg_len = v_len(seg);
    if seg_len <= 0.0 {
        return false;
    }
    let dir = v_mul(seg, seg_len.recip());
    let mut t = 0.0f32;
    for _ in 0..MARCHER_MAX_IT {
        let d = field_distance(edits, v_add(eye, v_mul(dir, t)));
        if d < MARCHER_EPS {
            // A hit AT the mesh surface (within the marcher's own hit threshold) is not an
            // occlusion — the two surfaces simply touch there.
            return t < seg_len - MARCHER_EPS;
        }
        t += d;
        if t >= seg_len {
            return false;
        }
    }
    false
}

// ===========================================================================================
// The two counts
// ===========================================================================================

/// Counts the pixels of `selection` satisfying [`is_fully_shadowed`] — S1 gate 2's quantity.
///
/// `l` must be the UNIT light direction (the shader shades with `normalize(L.dir)`); a
/// non-normalized direction would rescale the march's step schedule and silently change the
/// count.
pub fn shadowed_pixel_count(
    coverage: &Coverage,
    selection: &MeshSelection,
    edits: &[SdfEdit],
    l: [f32; 3],
) -> usize {
    debug_assert!(
        (v_len(l) - 1.0).abs() < 1.0e-4,
        "invariant: the S1 shadow predicate takes the UNIT light direction (|l| = {})",
        v_len(l)
    );
    selection
        .indices
        .iter()
        .filter(|&&i| {
            coverage
                .at(i as usize)
                .is_some_and(|px| is_fully_shadowed(edits, px, l))
        })
        .count()
}

/// Counts the pixels of `selection` satisfying [`has_contact_ao`] — S1 gate 3's quantity.
pub fn contact_ao_pixel_count(
    coverage: &Coverage,
    selection: &MeshSelection,
    edits: &[SdfEdit],
) -> usize {
    selection
        .indices
        .iter()
        .filter(|&&i| coverage.at(i as usize).is_some_and(|px| has_contact_ao(edits, px)))
        .count()
}

// ===========================================================================================
// The S4(ii) changed-pixel comparator
// ===========================================================================================

/// A decoded 32-bpp BMP in TOP-DOWN screen order — the same order [`Coverage`] indexes, so a
/// flat index means the same pixel in both.
pub struct Bmp32 {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Row-major `width * height` BGRA texels, `y` increasing DOWNWARD.
    pub bgra: Vec<[u8; 4]>,
}

/// Decodes the 32-bpp uncompressed BMP `boyko_app::host_dump::write_bmp` emits.
///
/// That writer produces a 54-byte header (14-byte `BITMAPFILEHEADER` + 40-byte
/// `BITMAPINFOHEADER`), 32 bpp, `BI_RGB`, and a POSITIVE height — i.e. BOTTOM-UP rows. This
/// decoder undoes the flip so the result is top-down, matching the rasterizer. A negative height
/// (top-down BMP) is accepted too, since the format allows it and a future writer might use it.
///
/// # Errors
///
/// Returns a describing message when the file cannot be read, is truncated, or is not the 32-bpp
/// uncompressed shape this comparator understands. Errors are values rather than panics so a
/// caller comparing many dumps can report WHICH one is malformed.
pub fn read_bmp32(path: &Path) -> Result<Bmp32, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if bytes.len() < 54 || &bytes[0..2] != b"BM" {
        return Err(format!("{}: not a BMP (short or missing 'BM' magic)", path.display()));
    }

    let u32_at = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let i32_at = |o: usize| i32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let u16_at = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);

    let data_offset = u32_at(10) as usize;
    let bpp = u16_at(28);
    let compression = u32_at(30);
    if bpp != 32 || compression != 0 {
        return Err(format!(
            "{}: expected 32-bpp BI_RGB, found {bpp} bpp compression {compression}",
            path.display()
        ));
    }

    let width_i = i32_at(18);
    let height_i = i32_at(22);
    if width_i <= 0 || height_i == 0 {
        return Err(format!("{}: degenerate extent {width_i}x{height_i}", path.display()));
    }
    let width = width_i as u32;
    let bottom_up = height_i > 0;
    let height = height_i.unsigned_abs();

    // 32 bpp rows are inherently 4-byte aligned, so the BMP row padding rule is a no-op here.
    let row_bytes = (width as usize) * 4;
    let needed = data_offset + row_bytes * (height as usize);
    if bytes.len() < needed {
        return Err(format!(
            "{}: truncated ({} bytes, need {needed})",
            path.display(),
            bytes.len()
        ));
    }

    let mut bgra = vec![[0u8; 4]; (width as usize) * (height as usize)];
    for row in 0..height as usize {
        let src_row = if bottom_up { (height as usize) - 1 - row } else { row };
        let src = data_offset + src_row * row_bytes;
        for x in 0..width as usize {
            let o = src + x * 4;
            bgra[row * (width as usize) + x] = [bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]];
        }
    }

    Ok(Bmp32 { width, height, bgra })
}

/// The comparator's result: how many of the selection's pixels differ between two dumps.
#[derive(Clone, Copy, Debug)]
pub struct ChangedPixels {
    /// The denominator — [`MeshSelection::len`].
    pub covered: usize,
    /// How many of those pixels differ in at least one byte.
    pub changed: usize,
}

impl ChangedPixels {
    /// The changed fraction of the covered set, or `0.0` for an empty selection.
    ///
    /// An empty selection returning `0.0` is NOT a "no change" verdict — it is the vacuity S1's
    /// gates exist to refute, and a caller reading this number must have already asserted the
    /// selection is non-empty.
    #[inline]
    pub fn fraction(&self) -> f64 {
        if self.covered == 0 {
            return 0.0;
        }
        self.changed as f64 / self.covered as f64
    }
}

/// **What rung S4(ii) consumes.** The fraction of `selection`'s pixels that differ between two
/// dumps of the same fixture.
///
/// A pixel counts as changed when ANY of its four bytes differs — the strictest reading, and the
/// same currency the byte-golden pins trade in.
///
/// # Errors
///
/// Returns a message when either image's extent disagrees with the selection's raster. That is a
/// hard error rather than a resize, because a silent extent mismatch would compare unrelated
/// pixels and report a plausible-looking fraction.
pub fn changed_covered_pixels(
    selection: &MeshSelection,
    a: &Bmp32,
    b: &Bmp32,
) -> Result<ChangedPixels, String> {
    if a.width != selection.width || a.height != selection.height {
        return Err(format!(
            "image A is {}x{} but the selection's raster is {}x{}",
            a.width, a.height, selection.width, selection.height
        ));
    }
    if b.width != selection.width || b.height != selection.height {
        return Err(format!(
            "image B is {}x{} but the selection's raster is {}x{}",
            b.width, b.height, selection.width, selection.height
        ));
    }

    let changed = selection
        .indices
        .iter()
        .filter(|&&i| a.bgra[i as usize] != b.bgra[i as usize])
        .count();
    Ok(ChangedPixels { covered: selection.len(), changed })
}
