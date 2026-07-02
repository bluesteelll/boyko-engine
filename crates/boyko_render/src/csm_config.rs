//! CSM Increment 1a — the cascade-fit ECS policy (CPU, unit-testable) for Cascaded
//! Shadow Maps. This is the contained data/policy layer; the GPU depth pass + resolve
//! are Increment 1b.
//!
//! Principle 0: ECS-native — [`CsmConfig`] is the owner-set `#[derive(Resource)]`
//! singleton (the cold config, NOT a side `std::Vec`/`HashMap`) and [`ResolvedCsm`] is
//! its derived companion Resource written by the cold [`resolve_csm_cascades`] system.
//! This mirrors the SSAO substrate exactly: [`SsaoConfig`](crate::ssao_config::SsaoConfig)
//! (the owner-set config) + [`ResolvedSsao`](crate::ssao_config::ResolvedSsao) (the derived
//! carrier) + [`resolve_ssao_policy`](crate::ssao_config::resolve_ssao_policy) (the cold
//! single-owner policy). The cascade array is an inline `[CascadeData; MAX_CASCADES]`, NOT
//! a `Vec`.
//!
//! # Capability is structural (no redundant `enabled: bool`)
//!
//! Whether CSM runs is keyed off [`CsmConfig::cascade_count`] (`> 0` AND a positive
//! `shadow_distance`), NOT a separate flag — `cascade_count == 0` IS "disabled", the
//! 0%-gate, mirroring how SSAO keys off `SsaoQuality::Off`. [`CsmConfig::enabled`] is a
//! derived predicate, not stored state.
//!
//! # The 0%-gate
//!
//! [`CsmConfig::default`] is DISABLED (`cascade_count == 0`). [`resolve_csm`] of the
//! default config is the all-zero [`ResolvedCsm`] (`csm_mode_word == 0`), and
//! [`ResolvedCsm::default`] is byte-identical to it — so a world that never inserts a
//! non-default [`CsmConfig`] carries the disabled selection and no render path is touched.
//!
//! # The fit (perspective-only this phase)
//!
//! [`resolve_csm`] is a PURE function of `(cfg, view, sun_dir)`: PSSM split distances →
//! per-cascade world-space frustum-slice corners → a rotation-invariant bounding-SPHERE
//! fit (the anti-shimmer body) → a texel-snapped light view → an orthographic
//! `view_proj`. The camera read is [`ViewUniform`]; it carries no orthographic
//! half-extents, so this phase asserts a perspective camera (critic W3).

use boyko_macros::Resource;

use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::system::{Res, ResMut};

use boyko_math::Vec3;
use boyko_scene::ViewUniform;

use crate::light::DirectionalLight;

// ---- constants -----------------------------------------------------------------------

/// The maximum number of shadow cascades — the inline cap on the [`ResolvedCsm`] cascade
/// array and the per-light shadow-map array-layer count. MUST equal the Inc-0
/// `boyko_rhi_vulkan::texture::MAX_CASCADES` (the shadow-map texture's `array_layers`
/// ceiling); the two are independent declarations kept numerically equal by this comment
/// and the const-assert in the RHI (`array_layers <= MAX_CASCADES`). `boyko_render` does
/// NOT depend on the RHI's texture module for this value (a dep just for a `4` would
/// couple the policy layer to a backend module), so it is re-declared here.
pub const MAX_CASCADES: usize = 4;

/// The light-space near plane (world units along the light ray). The light eye is pulled
/// back along `+sun_dir` by `z_far/2 = diameter`, so the depth range `[LIGHT_Z_NEAR,
/// 2·diameter]` brackets the fitted sphere symmetrically (center depth ≈ 0.5) with a
/// diameter-sized margin for casters between the sun and the slice. Mirrors the windowed
/// harness's `CSM_DEMO_NEAR` — the value the committed SPIR-V was validated with.
const LIGHT_Z_NEAR: f32 = 0.1;

/// The `|dot(sun_dir, up)|` threshold above which the world-up hint is collinear enough
/// with the light direction that the light-view right axis would be degenerate; past it the
/// fit swaps to the alternate up (W5 alt-up guard).
const UP_PARALLEL_THRESHOLD: f32 = 0.99;

/// The minimum cascade-sphere DIAMETER (world units). A degenerate (zero-extent) frustum
/// slice would otherwise produce a zero half-extent and a singular orthographic matrix; the
/// floor keeps every `view_proj` finite and invertible (W5 zero-radius floor).
const MIN_DIAMETER: f32 = 1.0e-3;

/// The default cascade count — `0`, the DISABLED 0%-gate (mirrors `SsaoQuality::Off`).
const DEFAULT_CASCADE_COUNT: u32 = 0;
/// The default shadow-map resolution per cascade (research default, a common 2K tile).
const DEFAULT_RESOLUTION: u32 = 2048;
/// The default shadow distance — the view-space far cap of the last cascade.
const DEFAULT_SHADOW_DISTANCE: f32 = 30.0;
/// The default PSSM blend `λ` (0 = uniform splits, 1 = logarithmic). `0.8` biases toward
/// logarithmic (more near-camera resolution), the common practical default.
const DEFAULT_LAMBDA: f32 = 0.8;
/// The default normal-bias (world units along the surface normal) the Inc-1b resolve uses
/// to push the shadow lookup off acne-prone grazing surfaces.
const DEFAULT_NORMAL_BIAS: f32 = 1.5;
/// The default constant depth bias (the rasterizer `depthBiasConstantFactor` term).
const DEFAULT_DEPTH_BIAS_CONSTANT: f32 = 0.0015;
/// The default slope-scaled depth bias (the rasterizer `depthBiasSlopeFactor` term).
const DEFAULT_DEPTH_BIAS_SLOPE: f32 = 1.5;

// ---- CsmConfig (the owner-set Resource — mirrors SsaoConfig) --------------------------

/// The global CSM config (CSM Inc-1a) — a `World`-singleton Resource the owner sets, the
/// CSM analogue of [`SsaoConfig`](crate::ssao_config::SsaoConfig). Enablement is structural
/// (`cascade_count > 0 && shadow_distance > 0`), so there is no separate flag.
///
/// `#[derive(Resource)]` via [`boyko_macros::Resource`] (the same derive path
/// `SsaoConfig` / `LightingConfig` use). `Copy` so the cold policy reads it by value.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct CsmConfig {
    /// Number of shadow cascades (`0` ⇒ disabled — the 0%-gate; clamped to
    /// [`MAX_CASCADES`] by [`resolve_csm`]).
    pub cascade_count: u32,
    /// Per-cascade shadow-map resolution in texels (drives `texel_size` and the snap grid).
    pub resolution: u32,
    /// The view-space far cap of the last cascade (the maximum shadowed distance). `<= 0`
    /// ⇒ disabled.
    pub shadow_distance: f32,
    /// PSSM split blend `λ ∈ [0, 1]` (0 = uniform, 1 = logarithmic).
    pub lambda: f32,
    /// Normal-bias (world units) the Inc-1b resolve applies along the surface normal.
    pub normal_bias: f32,
    /// Constant depth-bias term (rasterizer `depthBiasConstantFactor`).
    pub depth_bias_constant: f32,
    /// Slope-scaled depth-bias term (rasterizer `depthBiasSlopeFactor`).
    pub depth_bias_slope: f32,
}

impl Default for CsmConfig {
    /// The DISABLED default (`cascade_count == 0` — the 0%-gate): a default world resolves
    /// the all-zero [`ResolvedCsm`] and touches no render path. The remaining fields carry
    /// the research defaults so that flipping `cascade_count` to a positive value yields a
    /// usable fit without further tuning.
    #[inline]
    fn default() -> Self {
        Self {
            cascade_count: DEFAULT_CASCADE_COUNT,
            resolution: DEFAULT_RESOLUTION,
            shadow_distance: DEFAULT_SHADOW_DISTANCE,
            lambda: DEFAULT_LAMBDA,
            normal_bias: DEFAULT_NORMAL_BIAS,
            depth_bias_constant: DEFAULT_DEPTH_BIAS_CONSTANT,
            depth_bias_slope: DEFAULT_DEPTH_BIAS_SLOPE,
        }
    }
}

impl CsmConfig {
    /// Whether CSM runs — the structural predicate `cascade_count > 0 && shadow_distance
    /// > 0.0` (NOT stored state). False ⇒ the 0%-gate (no depth pass, the resolve's shadow
    /// term off). Mirrors [`SsaoConfig::enabled`](crate::ssao_config::SsaoConfig::enabled)
    /// keying off `quality != Off`.
    #[inline]
    pub fn enabled(&self) -> bool {
        self.cascade_count > 0 && self.shadow_distance > 0.0
    }
}

// ---- CascadeData (the per-cascade GPU-ready record) -----------------------------------

/// One cascade's fitted shadow transform + metadata — the per-cascade record the Inc-1b
/// depth pass renders into and the resolve samples. `#[repr(C)]`, 80 B, GPU-ready.
///
/// `view_proj` is the COLUMN-MAJOR world→light-clip matrix (the WGSL `mat4x4` convention,
/// matching [`ViewUniform::view_proj`]), so it uploads directly. `split_far` is the
/// VIEW-space far distance of this cascade (the selection boundary the resolve compares the
/// fragment depth against). `texel_size` is the world-space size of one shadow texel (for
/// the resolve's filter footprint / normal-bias scaling).
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct CascadeData {
    /// Column-major world→light-clip transform (`ortho · light_view`), GPU-ready.
    pub view_proj: [[f32; 4]; 4],
    /// VIEW-space far distance of this cascade (the split boundary).
    pub split_far: f32,
    /// World-space size of one shadow texel (`diameter / resolution`).
    pub texel_size: f32,
    /// Padding to a 16-byte stride (the trailing `f32` pair after the two scalars).
    pub _pad: [f32; 2],
}

// Layout pin: 16 (mat4) × 4 + 4 + 4 + 8 = 64 + 16 = 80 B. A change is a deliberate
// decision, not an accident (the GPU side reads this stride in Inc-1b).
const _: () = assert!(size_of::<CascadeData>() == 80);

impl CascadeData {
    /// The all-zero cascade (an unused slot in a partially-filled [`ResolvedCsm`], or the
    /// whole array when CSM is disabled). A zero `view_proj` is intentionally NOT a valid
    /// transform — `active_count` bounds the slots the consumer reads.
    pub const ZERO: Self = Self {
        view_proj: [[0.0; 4]; 4],
        split_far: 0.0,
        texel_size: 0.0,
        _pad: [0.0; 2],
    };
}

// ---- ResolvedCsm (the derived carrier — mirrors ResolvedSsao) -------------------------

/// The derived CSM selection the Inc-1b depth pass + resolve read — the CSM analogue of
/// [`ResolvedSsao`](crate::ssao_config::ResolvedSsao). [`resolve_csm_cascades`] is its
/// SINGLE writer (the one-producer-per-field discipline), recomputing it from
/// [`CsmConfig`] + the active [`ViewUniform`] + the primary sun each frame. `#[repr(C)]`
/// for a stable GPU-ready layout (the inline cascade array, NOT a `Vec`).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct ResolvedCsm {
    /// The fitted cascades; only `[0..active_count)` are valid (the rest are
    /// [`CascadeData::ZERO`]).
    pub cascades: [CascadeData; MAX_CASCADES],
    /// The number of valid cascades (`0` when disabled — the 0%-gate).
    pub active_count: u32,
    /// The CSM-enable mode word: `0` ⇒ off (no depth pass, resolve shadow term off), `1` ⇒
    /// on. Derived from the SAME enable predicate as `active_count`, so the two never
    /// disagree.
    pub csm_mode_word: u32,
    /// Padding to a 16-byte stride after the two trailing `u32` words.
    pub _pad: [u32; 2],
}

// Layout pin: 80 × 4 + 4 + 4 + 8 = 320 + 16 = 336 B.
const _: () = assert!(size_of::<ResolvedCsm>() == 336);

/// The byte size of the host-coherent CSM cascade UBO — `size_of::<ResolvedCsm>()`
/// (336 B: `[CascadeData; 4]` + `active_count` + `csm_mode_word` + pad). The resolve
/// binds a UBO of exactly this shape at binding 13; hosts size their cascade-UBO
/// ring slots from THIS constant (single source — no hand-copied `336`).
pub const RESOLVED_CSM_BYTES: usize = size_of::<ResolvedCsm>();

impl ResolvedCsm {
    /// The disabled selection — all-zero cascades, `active_count == 0`, `csm_mode_word ==
    /// 0`. The resolve of a disabled [`CsmConfig`] and the value [`ResolvedCsm::default`]
    /// returns.
    pub const DISABLED: Self = Self {
        cascades: [CascadeData::ZERO; MAX_CASCADES],
        active_count: 0,
        csm_mode_word: 0,
        _pad: [0; 2],
    };
}

impl Default for ResolvedCsm {
    /// The resolve of the default (disabled) [`CsmConfig`] — the 0%-gate, so a never-run
    /// policy already carries the no-shadow selection.
    #[inline]
    fn default() -> Self {
        Self::DISABLED
    }
}

// ---- ShadowCaster re-export (the structural caster capability) ------------------------

pub use crate::csm_marker::ShadowCaster;

// ---- the resolve decision (pure — the unit-testable fit) ------------------------------

/// Fits the cascade transforms for a perspective camera lit by `sun_dir` — the PURE,
/// unit-testable CSM resolve (the analogue of [`resolve_ssao`](crate::ssao_config::resolve_ssao),
/// the core the cold system wraps).
///
/// `sun_dir` is the world "direction TO the light" (the convention
/// [`DirectionalLight::direction`] stores); the light looks back along `-sun_dir`.
///
/// Disabled (`!cfg.enabled()`) ⇒ [`ResolvedCsm::DISABLED`] (all-zero, `csm_mode_word ==
/// 0`). Else, for each cascade `i in 0..min(cascade_count, MAX_CASCADES)`:
///
/// 1. **PSSM split** — `split_i` blends a logarithmic and a uniform partition of
///    `[near, far_cap]` by `λ`, where `far_cap = min(view.far, shadow_distance)`. The
///    splits are a fixed function of `(near, far_cap, λ, N)` — static, no shimmer.
/// 2. **8 frustum-slice corners** in WORLD space for `depth ∈ [near_i, split_i]`, from the
///    camera eye + orthonormal basis + `fov_y` / `aspect`.
/// 3. **Bounding-SPHERE fit** — the sphere of the 8 corners. The radius is rotation
///    INVARIANT (the anti-shimmer body); `diameter` is the Bevy integer-stable
///    `max(body_diag, far_plane_diag).ceil()`, `texel_size = diameter / resolution`,
///    half-extent `r = diameter / 2`.
/// 4. **Light basis** — `fwd = -sun_dir` (the light looks FROM the sun toward the scene),
///    `right = normalize(up_hint × fwd)`, `up = fwd × right`, with the W5 alt-up guard
///    (swap the hint to `+Z` when `sun_dir ≈ ±world_up`).
/// 5. **Texel snap** — quantize the sphere center's light-plane (`right`/`up`)
///    coordinates to whole `texel_size` BEFORE the view is built (the anti-shimmer
///    translation: the radius is rotation-invariant, so only the center moves frame to
///    frame and each shadow texel keeps a stable world footprint).
/// 6. **Matrix assembly** — the PROVEN on-screen convention (the exact form the committed
///    depth-VS + resolve SPIR-V were validated against pixel-by-pixel in the windowed
///    harness): the light eye is pulled back along `+sun_dir` by `z_far/2`
///    (`z_far = 2·diameter`, bounding casters between the sun and the slice), and the
///    combined `view_proj` maps world → light clip as `clip.x = x_lv/r`,
///    `clip.y = -y_lv/r` (the engine's framebuffer Y-flip — the SAME flip the camera
///    projection carries), `clip.z = (z_lv - z_near)/(z_far - z_near)` (Vulkan `[0,1]`,
///    depth GROWING away from the sun into the scene), `clip.w = 1`. The W5 zero-radius
///    floor (`diameter ≥ MIN_DIAMETER`) keeps a degenerate slice finite + non-singular.
///
/// Do NOT re-derive this matrix through generic `look_at`/`ortho` helpers: the depth
/// direction, the eye side, and the Y-flip are SHADER CONTRACT, not style — an assembly
/// that is self-consistent between the depth pass and the resolve can still disagree with
/// the SPIR-V's hardcoded UV/compare conventions (the exact failure the R4 room shipped
/// with: lit floors, self-shadowed casters, no cast shadows). The convention tests below
/// pin all three axes.
///
/// # Perspective-only (critic W3)
///
/// [`ViewUniform`] carries no orthographic half-extents, so the frustum-corner step needs
/// `fov_y` / `aspect`. An orthographic camera (`fov_y == 0`) trips a `debug_assert!`; in
/// release it produces a degenerate (zero-radius-floored) fit rather than UB.
#[inline]
pub fn resolve_csm(cfg: &CsmConfig, view: &ViewUniform, sun_dir: [f32; 3]) -> ResolvedCsm {
    if !cfg.enabled() {
        return ResolvedCsm::DISABLED;
    }

    debug_assert!(
        view.fov_y != 0.0,
        "CSM requires a perspective camera this phase (ViewUniform has no ortho half-extents)"
    );

    let count = (cfg.cascade_count as usize).min(MAX_CASCADES);
    let n = count as f32;

    let eye = view.camera_pos.xyz();
    let forward = view.cam_forward.xyz();
    let right = view.cam_right.xyz();
    let up = view.cam_up.xyz();

    let near = view.near;
    // The last cascade's far is capped at the owner's shadow distance (and never beyond the
    // camera far). `enabled()` guarantees `shadow_distance > 0`; clamp to `> near` so the
    // partition is well-formed even for a misconfigured near/distance pair.
    let far_cap = view.far.min(cfg.shadow_distance).max(near + MIN_DIAMETER);

    let sun = Vec3::new(sun_dir[0], sun_dir[1], sun_dir[2]);
    // The light direction the W5 guard compares against world-up; a degenerate (zero) sun
    // falls back to a valid default so the fit stays finite.
    let sun = if sun.length_squared() > 1.0e-12 {
        sun.normalize()
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };

    // The camera frame + perspective scalars are constant across cascades — build once.
    let rig = FrustumRig {
        eye,
        forward,
        right,
        up,
        half_tan: (view.fov_y * 0.5).tan(),
        aspect: view.aspect,
    };

    // ── The light basis (constant across cascades — a function of the sun only), in the
    // PROVEN on-screen convention (doc step 4): `fwd = -sun` (the light looks FROM the sun
    // toward the scene). W5 alt-up guard: when the sun is (anti)parallel to world-up the
    // right axis is degenerate — swap the up HINT to `+Z` (the cross ORDER is unchanged,
    // so the basis chirality is identical to the nominal case).
    let world_up = Vec3::new(0.0, 1.0, 0.0);
    let up_hint = if sun.dot(world_up).abs() > UP_PARALLEL_THRESHOLD {
        Vec3::new(0.0, 0.0, 1.0)
    } else {
        world_up
    };
    let fwd = sun * -1.0;
    let light_right = up_hint.cross(fwd).normalize();
    let light_up = fwd.cross(light_right);

    let mut cascades = [CascadeData::ZERO; MAX_CASCADES];

    let mut near_i = near;
    for (i, slot) in cascades.iter_mut().enumerate().take(count) {
        let split_i = pssm_split(near, far_cap, cfg.lambda, i + 1, n);

        // 8 world-space frustum-slice corners for depth ∈ [near_i, split_i].
        let corners = slice_corners(&rig, near_i, split_i);

        // Bounding sphere: center = mean of corners, radius = max corner distance.
        let center = sphere_center(&corners);
        let body_radius = sphere_radius(&corners, center);

        // Bevy integer-stable diameter: ceil(2·radius), floored against a degenerate slice
        // (the W5 zero-radius floor) so the orthographic half-extent is never zero.
        let diameter = (2.0 * body_radius).ceil().max(MIN_DIAMETER);
        let texel_size = diameter / cfg.resolution.max(1) as f32;
        let r = diameter * 0.5;

        // Texel snap (anti-shimmer translation, doc step 5): quantize the center's
        // light-plane (right/up) coordinates to whole texels BEFORE the view is built —
        // the radius is rotation-invariant, so under camera motion only the center
        // translates and each shadow texel keeps a stable world footprint.
        let cx = light_right.dot(center);
        let cy = light_up.dot(center);
        let dx = (cx / texel_size).floor() * texel_size - cx;
        let dy = (cy / texel_size).floor() * texel_size - cy;
        let center = center + light_right * dx + light_up * dy;

        // Matrix assembly (doc step 6 — the PROVEN on-screen convention; see the fn doc
        // for why this must NOT be re-derived through generic look_at/ortho helpers).
        // The light eye is pulled back along +sun by z_far/2 (= the sphere diameter).
        let z_far = 2.0 * diameter;
        let eye = center + sun * (z_far * 0.5);
        let tx = -light_right.dot(eye);
        let ty = -light_up.dot(eye);
        let tz = -fwd.dot(eye);

        // pv[row][col] = ortho_row · light_view_row: clip.x = x_lv/r, clip.y = -y_lv/r
        // (the framebuffer Y-flip), clip.z = (z_lv - z_near)/(z_far - z_near) (depth
        // growing away from the sun), clip.w = 1.
        let inv_h = 1.0 / r;
        let zr = z_far - LIGHT_Z_NEAR;
        let pv: [[f32; 4]; 4] = [
            [
                inv_h * light_right.x,
                inv_h * light_right.y,
                inv_h * light_right.z,
                inv_h * tx,
            ],
            [
                -inv_h * light_up.x,
                -inv_h * light_up.y,
                -inv_h * light_up.z,
                -inv_h * ty,
            ],
            [fwd.x / zr, fwd.y / zr, fwd.z / zr, (tz - LIGHT_Z_NEAR) / zr],
            [0.0, 0.0, 0.0, 1.0],
        ];
        // COLUMN-MAJOR storage (the byte layout the depth-VS push @0 and the resolve
        // cbuffer expect): view_proj[col][row] = pv[row][col].
        let mut view_proj = [[0.0f32; 4]; 4];
        for (row, prow) in pv.iter().enumerate() {
            for (col, &v) in prow.iter().enumerate() {
                view_proj[col][row] = v;
            }
        }

        *slot = CascadeData {
            view_proj,
            split_far: split_i,
            texel_size,
            _pad: [0.0; 2],
        };

        near_i = split_i;
    }

    ResolvedCsm {
        cascades,
        active_count: count as u32,
        csm_mode_word: 1,
        _pad: [0; 2],
    }
}

/// The PSSM (Parallel-Split Shadow Maps) split distance for split `idx` of `N`
/// (`idx ∈ 1..=N`): `λ·log + (1−λ)·uniform`, where `log = near·(far/near)^(idx/N)` and
/// `uniform = near + (far−near)·(idx/N)`. `idx == N` returns `far` exactly (both terms do),
/// so the last cascade's far equals `far_cap` — the property the monotonic-splits test
/// pins.
#[inline]
fn pssm_split(near: f32, far: f32, lambda: f32, idx: usize, n: f32) -> f32 {
    let ratio = idx as f32 / n;
    let log = near * (far / near).powf(ratio);
    let uniform = near + (far - near) * ratio;
    lambda * log + (1.0 - lambda) * uniform
}

/// The camera frame + perspective scalars the frustum-corner step reads — grouped so
/// [`slice_corners`] stays under the argument-count budget and the frame is built once per
/// fit (it is constant across cascades). Eye + orthonormal basis (world space) + the
/// half-FOV tangent and aspect.
#[derive(Clone, Copy)]
struct FrustumRig {
    eye: Vec3,
    forward: Vec3,
    right: Vec3,
    up: Vec3,
    half_tan: f32,
    aspect: f32,
}

/// The 8 world-space corners of the camera frustum slice between view-space depths
/// `[depth_near, depth_far]`. Each corner is `eye + forward·d ± right·(aspect·half_tan·d) ±
/// up·(half_tan·d)`.
#[inline]
fn slice_corners(rig: &FrustumRig, depth_near: f32, depth_far: f32) -> [Vec3; 8] {
    let mut corners = [Vec3::ZERO; 8];
    let mut k = 0;
    for &d in &[depth_near, depth_far] {
        let hh = rig.half_tan * d; // half-height at depth d
        let hw = hh * rig.aspect; // half-width at depth d
        let center = rig.eye + rig.forward * d;
        for &sy in &[-1.0_f32, 1.0] {
            for &sx in &[-1.0_f32, 1.0] {
                corners[k] = center + rig.right * (hw * sx) + rig.up * (hh * sy);
                k += 1;
            }
        }
    }
    corners
}

/// The mean of 8 corners — the bounding-sphere center (the cheap, rotation-stable center
/// used by the integer-stable fit).
#[inline]
fn sphere_center(corners: &[Vec3; 8]) -> Vec3 {
    let mut sum = Vec3::ZERO;
    for &c in corners {
        sum = sum + c;
    }
    sum * (1.0 / 8.0)
}

/// The bounding-sphere radius — the maximum corner distance from `center`. This is
/// rotation INVARIANT (a yaw/pitch of the camera permutes the corner set but not the set of
/// distances), the anti-shimmer property the rotation-invariance test pins.
#[inline]
fn sphere_radius(corners: &[Vec3; 8], center: Vec3) -> f32 {
    let mut max_sq = 0.0_f32;
    for &c in corners {
        let d = c - center;
        max_sq = max_sq.max(d.length_squared());
    }
    max_sq.sqrt()
}

// ---- the cold StrategyPolicy system (mirrors resolve_ssao_policy) ---------------------

/// The cold CSM resolve policy — reads [`CsmConfig`] + the active [`ViewUniform`] + the
/// PRIMARY directional light, and writes the derived [`ResolvedCsm`]. The CSM analogue of
/// [`resolve_ssao_policy`](crate::ssao_config::resolve_ssao_policy) /
/// [`select_lighting_cull`](crate::light_policy::select_lighting_cull). It is the SINGLE
/// owner of [`ResolvedCsm`] (the one-producer write discipline).
///
/// # Primary sun selection
///
/// The fit needs ONE light direction. The primary directional is the FIRST
/// [`DirectionalLight`] the query yields — the same "first/primary directional" the SDF
/// marcher writes into `gMaterial.R` and the lighting resolve treats as the sun. With no
/// directional light present, [`ResolvedCsm`] is left at [`ResolvedCsm::DISABLED`] (no sun
/// ⇒ no cascades).
///
/// Cold by construction (zero hot-path cost): a single fit run once per frame; the per-row
/// render path never reads [`CsmConfig`].
//
// `clippy::needless_pass_by_value`: `Res`/`ResMut`/`Query` are by-value `SystemParam`s
// read/written through reborrows — the same false-positive `resolve_ssao_policy` and
// `resolve_active_camera` carry.
#[allow(clippy::needless_pass_by_value)]
pub fn resolve_csm_cascades(
    cfg: Res<CsmConfig>,
    view: Res<ViewUniform>,
    suns: Query<&DirectionalLight>,
    mut out: ResMut<ResolvedCsm>,
) {
    // The primary sun: the first directional light. No directional ⇒ the disabled selection
    // (a CSM pass with no sun has nothing to fit).
    let Some(sun) = suns.iter().next() else {
        *out = ResolvedCsm::DISABLED;
        return;
    };

    // No RESOLVED perspective camera ⇒ the disabled selection (the structural mirror of
    // the no-sun arm above). `fov_y == 0` is the engine sentinel for "orthographic or no
    // active camera resolved yet" — the frustum-corner fit is undefined for it (the pure
    // `resolve_csm` debug-asserts a perspective view, critic W3). This arm ALSO absorbs
    // the documented cross-plugin add-order stagger: on the first frame this policy may
    // run before `resolve_active_camera` has written `ViewUniform`, in which case the
    // default (sentinel) view lands here, the frame carries the disabled selection, and
    // the next frame's re-fit self-corrects (host plan R4 — without this arm the stagger
    // was a debug-assert panic on a worker thread the moment CSM + a sun were live).
    if view.fov_y == 0.0 {
        *out = ResolvedCsm::DISABLED;
        return;
    }

    *out = resolve_csm(&cfg, &view, sun.direction);
}

#[cfg(test)]
mod tests {
    use super::*;

    use boyko_math::{Affine3A, Vec3};
    use boyko_scene::Projection;

    /// A perspective `ViewUniform` from `eye`, oriented by `yaw` / `pitch` (radians), FOV
    /// 60°, 16:9, near 0.1, far 1000 — the camera the fit reads. Built through the real
    /// `ViewUniform::from_camera` (off an `Affine3A::look_at_rh` world transform) so the
    /// basis / scalars are exactly what the live policy carries. The look direction is the
    /// yaw/pitch spherical direction (yaw 0, pitch 0 ⇒ looking down `-Z`).
    fn perspective_view(eye: Vec3, yaw: f32, pitch: f32) -> ViewUniform {
        let (sp, cp) = pitch.sin_cos();
        let (sy, cy) = yaw.sin_cos();
        // Forward = the yaw/pitch direction; yaw 0, pitch 0 → (0, 0, -1).
        let forward = Vec3::new(cp * sy, sp, -cp * cy);
        let world = Affine3A::look_at_rh(eye, eye + forward, Vec3::new(0.0, 1.0, 0.0));
        let proj = Projection::Perspective {
            fov_y: core::f32::consts::FRAC_PI_3,
            aspect: 16.0 / 9.0,
            near: 0.1,
            far: 1000.0,
        };
        ViewUniform::from_camera(world, proj)
    }

    /// An ENABLED config: 4 cascades, 2K, 30-unit distance, λ 0.8.
    fn enabled_cfg() -> CsmConfig {
        CsmConfig {
            cascade_count: 4,
            ..CsmConfig::default()
        }
    }

    fn all_finite(m: &[[f32; 4]; 4]) -> bool {
        m.iter().flatten().all(|x| x.is_finite())
    }

    /// A 4×4 column-major determinant — nonzero ⇒ non-singular `view_proj`.
    fn det4(m: &[[f32; 4]; 4]) -> f32 {
        // Laplace expansion along the first column (m[col][row]).
        let a = m[0][0];
        let b = m[1][0];
        let c = m[2][0];
        let d = m[3][0];
        let minor = |c0: usize, c1: usize, c2: usize| {
            let r = [1usize, 2, 3];
            let g = |ci: usize, ri: usize| m[ci][ri];
            g(c0, r[0]) * (g(c1, r[1]) * g(c2, r[2]) - g(c1, r[2]) * g(c2, r[1]))
                - g(c1, r[0]) * (g(c0, r[1]) * g(c2, r[2]) - g(c0, r[2]) * g(c2, r[1]))
                + g(c2, r[0]) * (g(c0, r[1]) * g(c1, r[2]) - g(c0, r[2]) * g(c1, r[1]))
        };
        a * minor(1, 2, 3) - b * minor(0, 2, 3) + c * minor(0, 1, 3) - d * minor(0, 1, 2)
    }

    #[test]
    fn disabled_config_is_the_all_zero_zero_gate() {
        let view = perspective_view(Vec3::new(0.0, 2.0, 0.0), 0.0, 0.0);
        let resolved = resolve_csm(&CsmConfig::default(), &view, [0.3, -1.0, 0.2]);
        assert_eq!(resolved, ResolvedCsm::DISABLED);
        assert_eq!(resolved.csm_mode_word, 0);
        assert_eq!(resolved.active_count, 0);
        for c in &resolved.cascades {
            assert_eq!(*c, CascadeData::ZERO);
        }
    }

    #[test]
    fn default_resolved_matches_resolving_the_default_config() {
        let view = perspective_view(Vec3::new(0.0, 2.0, 0.0), 0.0, 0.0);
        assert_eq!(ResolvedCsm::default(), resolve_csm(&CsmConfig::default(), &view, [0.0, -1.0, 0.0]));
    }

    #[test]
    fn pssm_splits_monotonic_and_last_equals_far_cap() {
        let view = perspective_view(Vec3::new(0.0, 2.0, 0.0), 0.0, 0.0);
        let cfg = enabled_cfg();
        let resolved = resolve_csm(&cfg, &view, [0.3, -1.0, 0.2]);
        assert_eq!(resolved.active_count, 4);

        let far_cap = view.far.min(cfg.shadow_distance);
        let mut prev = view.near;
        for (i, c) in resolved.cascades.iter().enumerate().take(4) {
            assert!(
                c.split_far > prev,
                "split {i} must be strictly increasing (prev {prev}, split {})",
                c.split_far
            );
            prev = c.split_far;
        }
        // The last cascade's far equals the far cap (within fp epsilon).
        let last = resolved.cascades[3].split_far;
        assert!(
            (last - far_cap).abs() <= 1.0e-2 * far_cap,
            "last split {last} must equal far_cap {far_cap}"
        );
    }

    #[test]
    fn bounding_sphere_diameter_is_rotation_invariant() {
        let cfg = enabled_cfg();
        let eye = Vec3::new(5.0, 3.0, -2.0);
        let sun = [0.3, -1.0, 0.2];

        let base = resolve_csm(&cfg, &perspective_view(eye, 0.0, 0.0), sun);
        // Rotating the camera yaw/pitch permutes the corner set but not the radius — the
        // anti-shimmer property. texel_size = diameter / resolution, so equal texel_size ⇒
        // equal diameter.
        for &(yaw, pitch) in &[
            (0.7_f32, 0.0_f32),
            (0.0, 0.4),
            (1.3, -0.5),
            (-2.1, 0.2),
        ] {
            let rotated = resolve_csm(&cfg, &perspective_view(eye, yaw, pitch), sun);
            for i in 0..4 {
                let a = base.cascades[i].texel_size;
                let b = rotated.cascades[i].texel_size;
                assert!(
                    (a - b).abs() <= 1.0e-4 * a.max(1.0),
                    "cascade {i} texel_size must be rotation-invariant (base {a}, rotated {b})"
                );
            }
        }
    }

    #[test]
    fn texel_snap_is_idempotent() {
        // Resolving the SAME (camera, sun) twice yields a bit-identical fit — the snap is a
        // deterministic, idempotent function of its inputs (re-snapping a snapped center is a
        // no-op).
        let cfg = enabled_cfg();
        let view = perspective_view(Vec3::new(1.5, 4.0, -3.0), 0.6, -0.2);
        let sun = [0.2, -1.0, 0.35];
        let a = resolve_csm(&cfg, &view, sun);
        let b = resolve_csm(&cfg, &view, sun);
        assert_eq!(a, b, "the fit must be a deterministic (idempotent) function of its inputs");
    }

    #[test]
    fn alt_up_engaged_yields_finite_nonsingular_view_proj() {
        // Sun ≈ ±world-up: the alt-up guard must engage and keep every view_proj finite +
        // non-singular (a degenerate light-view right axis would otherwise NaN the matrix).
        let cfg = enabled_cfg();
        let view = perspective_view(Vec3::new(0.0, 2.0, 0.0), 0.0, 0.0);
        for &sun in &[[0.0_f32, 1.0, 0.0], [0.0, -1.0, 0.0], [0.001, 1.0, 0.001]] {
            let resolved = resolve_csm(&cfg, &view, sun);
            for (i, c) in resolved.cascades.iter().enumerate().take(4) {
                assert!(all_finite(&c.view_proj), "cascade {i} view_proj must be finite for sun {sun:?}");
                assert!(
                    det4(&c.view_proj).abs() > 1.0e-12,
                    "cascade {i} view_proj must be non-singular for sun {sun:?}"
                );
            }
        }
    }

    #[test]
    fn zero_radius_floor_keeps_ortho_finite_on_degenerate_slice() {
        // A pathological config: a single cascade with a tiny shadow distance just past the
        // near plane makes a near-degenerate slice — the MIN_DIAMETER floor must still
        // produce a finite, non-singular ortho.
        let cfg = CsmConfig {
            cascade_count: 1,
            shadow_distance: view_near_plus_epsilon(),
            ..CsmConfig::default()
        };
        let view = perspective_view(Vec3::new(0.0, 2.0, 0.0), 0.0, 0.0);
        let resolved = resolve_csm(&cfg, &view, [0.0, -1.0, 0.0]);
        let c = &resolved.cascades[0];
        assert!(all_finite(&c.view_proj), "degenerate slice view_proj must be finite");
        assert!(det4(&c.view_proj).abs() > 1.0e-30, "degenerate slice view_proj must be non-singular");
    }

    /// A shadow distance a hair past the near plane (drives a near-zero-extent slice).
    fn view_near_plus_epsilon() -> f32 {
        0.1 + 1.0e-5
    }

    #[test]
    fn every_view_proj_element_finite_for_random_camera_and_sun() {
        // Deterministic pseudo-random sweep over (eye, yaw, pitch, sun) in range — every
        // produced view_proj element must be finite (no NaN/Inf escapes the fit).
        let cfg = enabled_cfg();
        let mut state = 0x1234_5678_9abc_def0u64;
        let mut next = || {
            // xorshift64 → [0, 1)
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f32 / (1u64 << 53) as f32
        };
        for _ in 0..64 {
            let eye = Vec3::new(
                (next() - 0.5) * 40.0,
                (next() - 0.5) * 20.0,
                (next() - 0.5) * 40.0,
            );
            let yaw = (next() - 0.5) * core::f32::consts::TAU;
            let pitch = (next() - 0.5) * (core::f32::consts::FRAC_PI_2 - 0.05) * 2.0;
            let sun = [next() - 0.5, -(0.2 + next() * 0.8), next() - 0.5];
            let resolved = resolve_csm(&cfg, &perspective_view(eye, yaw, pitch), sun);
            for (i, c) in resolved.cascades.iter().enumerate().take(4) {
                assert!(
                    all_finite(&c.view_proj),
                    "cascade {i} view_proj must be finite (eye {eye:?}, yaw {yaw}, pitch {pitch}, sun {sun:?})"
                );
            }
        }
    }

    #[test]
    fn cascade_count_clamped_to_max_cascades() {
        // An over-large cascade_count is clamped to MAX_CASCADES (the inline array bound).
        let cfg = CsmConfig {
            cascade_count: 99,
            ..CsmConfig::default()
        };
        let view = perspective_view(Vec3::new(0.0, 2.0, 0.0), 0.0, 0.0);
        let resolved = resolve_csm(&cfg, &view, [0.0, -1.0, 0.0]);
        assert_eq!(resolved.active_count, MAX_CASCADES as u32);
    }

    // ---- convention pins (the shader contract — doc step 6) ---------------------------

    /// Applies a cascade's COLUMN-MAJOR `view_proj` to a world point (`w = 1`), returning
    /// the raw clip lanes: `clip[row] = Σ_col m[col][row] · p[col]`.
    fn clip(m: &[[f32; 4]; 4], p: Vec3) -> [f32; 4] {
        let ph = [p.x, p.y, p.z, 1.0];
        let mut out = [0.0f32; 4];
        for (row, o) in out.iter_mut().enumerate() {
            *o = (0..4).map(|col| m[col][row] * ph[col]).sum();
        }
        out
    }

    /// The light basis of the PROVEN on-screen convention, re-derived independently
    /// (the windowed harness's `csm_light_basis` formula) — the oracle the convention
    /// pins compare the fit's axes against.
    fn light_basis(sun: Vec3) -> (Vec3, Vec3) {
        let fwd = sun * -1.0;
        let up_hint = if sun.dot(Vec3::new(0.0, 1.0, 0.0)).abs() > 0.99 {
            Vec3::new(0.0, 0.0, 1.0)
        } else {
            Vec3::new(0.0, 1.0, 0.0)
        };
        let right = up_hint.cross(fwd).normalize();
        let up = fwd.cross(right);
        (right, up)
    }

    #[test]
    fn matrix_convention_is_the_proven_on_screen_one() {
        // The shader-contract axes of the cascade matrix, pinned against the convention
        // the committed depth-VS + resolve SPIR-V were validated with pixel-by-pixel in
        // the windowed harness: depth GROWS away from the sun into the scene, `clip.y`
        // carries the framebuffer Y-flip, `clip.x` follows the light's right axis, and
        // the fitted slice lands inside the clip box. A matrix assembly that is merely
        // SELF-consistent between the depth pass and the resolve can still flip these
        // axes and break every shadow lookup (the R4 room regression: lit floors,
        // self-shadowed casters, no cast shadows) — these pins fail on that assembly.
        let cfg = enabled_cfg();
        for &(eye, yaw, pitch, sun_dir) in &[
            (Vec3::new(0.0, 1.7, 6.0), 0.0_f32, 0.0_f32, [-0.45_f32, 0.82, 0.36]),
            (Vec3::new(5.0, 3.0, -2.0), 0.7, -0.2, [0.3, 0.9, 0.2]),
            (Vec3::new(-3.0, 8.0, 4.0), -1.2, 0.3, [0.1, 0.7, -0.6]),
        ] {
            let view = perspective_view(eye, yaw, pitch);
            let resolved = resolve_csm(&cfg, &view, sun_dir);
            let sun = Vec3::new(sun_dir[0], sun_dir[1], sun_dir[2]).normalize();
            let (light_right, light_up) = light_basis(sun);

            // A world point inside cascade 0's fitted slice: the camera-ray midpoint of
            // the [near, split_0] view-z range.
            let fwd_cam = view.cam_forward.xyz();
            let mid = eye + fwd_cam * ((view.near + resolved.cascades[0].split_far) * 0.5);
            let m = &resolved.cascades[0].view_proj;

            let c0 = clip(m, mid);
            assert!(
                (c0[3] - 1.0).abs() < 1.0e-5,
                "orthographic clip.w must be 1 (got {})",
                c0[3]
            );
            assert!(
                c0[0].abs() <= 1.0 && c0[1].abs() <= 1.0 && (0.0..=1.0).contains(&c0[2]),
                "the slice midpoint must land inside the clip box (clip {c0:?})"
            );

            // Depth axis: moving TOWARD the sun lowers depth (depth grows away from the
            // sun into the scene) — inverted on the broken assembly.
            let toward_sun = clip(m, mid + sun * 1.0);
            assert!(
                toward_sun[2] < c0[2],
                "depth must DECREASE toward the sun ({} !< {})",
                toward_sun[2],
                c0[2]
            );

            // Y-flip: moving along the light's UP axis lowers clip.y (the engine's
            // framebuffer convention) — absent on the broken assembly.
            let up_moved = clip(m, mid + light_up * 0.5);
            assert!(
                up_moved[1] < c0[1],
                "clip.y must carry the framebuffer Y-flip ({} !< {})",
                up_moved[1],
                c0[1]
            );

            // X axis: moving along the light's RIGHT axis raises clip.x.
            let right_moved = clip(m, mid + light_right * 0.5);
            assert!(
                right_moved[0] > c0[0],
                "clip.x must follow the light right axis ({} !> {})",
                right_moved[0],
                c0[0]
            );
        }
    }
}
