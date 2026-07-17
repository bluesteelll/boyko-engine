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
//! [`resolve_csm`] is a PURE function of `(cfg, view, sun_dir, fit)`: PSSM split distances →
//! per-cascade world-space frustum-slice corners → a rotation-invariant bounding-SPHERE
//! fit (the anti-shimmer body) → a texel-snapped light view → an orthographic
//! `view_proj`. The camera read is [`ViewUniform`]; it carries no orthographic
//! half-extents, so this phase asserts a perspective camera (critic W3).
//!
//! # Caster-aware split-range fit (`docs/CSM-AUTOFIT-PLAN.md`, rung C3; default `CatchAll`)
//!
//! [`CsmFit`] is the already-decided split-range decision `resolve_csm` applies —
//! `resolve_csm` itself does not consult [`CsmConfig::fit_mode`]. [`resolve_csm_cascades`]
//! computes it via the private `select_fit` gate: [`CsmFitMode::Fixed`] selects
//! [`CsmFit::NONE`] immediately, without reading [`CsmCasterBounds`] or [`CsmFitState`] —
//! textually the pre-fit math, so it remains the exact byte-for-byte opt-out.
//! [`CsmFitMode::CatchAll`] is the DEFAULT as of 2026-07-16 (owner call): splitting by the
//! camera range spent cascades on caster-free space while the tail cascade smeared whatever
//! fell into it, and fitting the range costs nothing to fix that.
//! [`CsmFitMode::Shrink`] / [`CsmFitMode::CatchAll`]
//! fit the split range to the caster-derived `far_eff` (an anti-shimmer log-quantized,
//! Schmitt-latched value — see [`grid_value`] / [`latch_cell`]), requiring
//! [`reduce_caster_bounds`](crate::csm_caster::reduce_caster_bounds) to be app-registered
//! (rung C5); without it every mode renders as `Fixed`. The caster modes ALSO grow the
//! light eye's sun-axis pull-back ([`CsmFit::caster_aabb`], rung C4) to keep capturing any
//! caster up-sun of the fitted sphere — otherwise shrinking the split range's `diameter`
//! (the fit's entire point) would silently shrink that capture too and clip an up-sun
//! caster (D5).

use boyko_macros::{Resource, SystemSet};

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

// ---- the log-quantization grid + Schmitt latch (`docs/CSM-AUTOFIT-PLAN.md` Decision D6).
// Landed dark in C1; rung C3 wires the first caller (`select_fit`, called from
// `resolve_csm_cascades`). Under the default `CsmFitMode::Fixed` these functions are still
// never called — the fit-mode 0%-gate short-circuits before `select_fit` reaches them. ----

/// Log2 step of the far grid: `far_eff ∈ {2^(k/4)}` (ratio ≈ 1.18921 = `2^(1/4)`). The
/// ANTI-SHIMMER parameter (D6/D9), NOT an owner-facing tuning knob — exposing it would let
/// an owner set e.g. `1` cell/octave and reintroduce the texel-snap shimmer the grid exists
/// to bound. 4 cells per octave ⇒ the grid is exactly representable at every power of two
/// (`grid_value(4*n) == 2^n` exactly).
const FIT_GRID_CELLS_PER_OCTAVE: i32 = 4;

/// Grid mantissa thresholds `2^(0/4), 2^(1/4), 2^(2/4), 2^(3/4)` — the SAME table both
/// `grid_value` multiplies by and `grid_cell` compares against, which is why the round-trip
/// `grid_cell(grid_value(k)) == k` is exact (D6): scaling a normalized float by an exact
/// power of two only ever rewrites its exponent field, so the mantissa `grid_cell` recovers
/// from `grid_value(k)`'s bits is bit-identical to the entry that produced it.
const FIT_GRID_TABLE: [f32; 4] = [
    1.0,
    1.189_207_1,
    core::f32::consts::SQRT_2, // 2^(2/4) exactly — clippy::approx_constant prefers the named constant
    1.681_792_8,
];

/// The Schmitt shrink band, in grid cells: `latch_cell` only shrinks once `raw` has fallen
/// `FIT_SHRINK_BAND_CELLS` full cells below the latched cell's value (2 cells ≈ −29.3%).
/// Grow is unconditional/immediate (no band) — the asymmetry is principled, not an
/// oversight (D6): grow is the *masked* direction (the shadow shrinks on screen while the
/// camera recedes), so pops there are free, while shrink is the *scrutinised* direction (the
/// shadow grows on screen), so pops there are made rare.
const FIT_SHRINK_BAND_CELLS: i32 = 2;

/// Caps the sun-axis pull-back (`docs/CSM-AUTOFIT-PLAN.md` D5, rung C4) at
/// `MAX_PULLBACK_RATIO * diameter`, so `z_far <= (1 + MAX_PULLBACK_RATIO) * diameter` (5x
/// today's `2x`) — bounding the depth-precision / bias-scale drift a far up-sun caster could
/// otherwise inflate without limit. A correctness parameter (D9), NOT an owner-facing tuning
/// knob, for the same reason `FIT_GRID_CELLS_PER_OCTAVE` is not: exposing it lets an owner
/// set a value that clips casters again.
const MAX_PULLBACK_RATIO: f32 = 4.0;

/// `latch_cell`'s "never latched" sentinel for `prev_k` — `i32::MIN`, so it can never
/// collide with a real grid cell (a legitimate `k` stays within a few hundred of zero for
/// any plausible scene scale). Promoted to the public [`CsmFitState::UNLATCHED`] (rung C3):
/// that associated const is DEFINED as this private const, not a second `i32::MIN` literal,
/// so the two sentinels can never drift apart.
const FIT_UNLATCHED: i32 = i32::MIN;

/// The minimum exponent `exp2i` can represent as a NORMAL `f32` (IEEE-754 exponent field
/// `1`, unbiased `1 - 127`). Below this the bit pattern is either subnormal or zero —
/// outside `exp2i`'s exact-power-of-two domain.
const EXP2I_MIN_EXPONENT: i32 = -126;

/// The maximum exponent `exp2i` can represent as a NORMAL `f32` (exponent field `254`,
/// unbiased `254 - 127`); one past this is the reserved Inf/NaN exponent field `255`.
const EXP2I_MAX_EXPONENT: i32 = 127;

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
/// The default cascade split-RANGE policy — fit the range to the casters, keeping the last
/// cascade as the catch-all for everything beyond them. Owner call, 2026-07-16: it costs
/// nothing over [`CsmFitMode::Fixed`] (same cascades, same resolution, same draws) and stops
/// the tail cascade from smearing casters that fall off the previous cascade's edge. See
/// [`CsmFitMode::CatchAll`] for the measured numbers and the trade it makes.
const DEFAULT_FIT_MODE: CsmFitMode = CsmFitMode::CatchAll;
/// The default CSM shadow-edge PCF kernel — [`CsmPcfKernel::Tent13`], the crawl-free anchor
/// (rung E1). `Tent13 == 0` is load-bearing (see that variant's doc); this constant exists for
/// `CsmConfig::default`'s consistency with [`DEFAULT_FIT_MODE`], not because a non-zero
/// default was ever considered.
const DEFAULT_PCF_KERNEL: CsmPcfKernel = CsmPcfKernel::Tent13;

// ---- CsmFitMode (the cascade split-range policy knob, `docs/CSM-AUTOFIT-PLAN.md`) -----

/// The cascade split-RANGE policy — the owner's sharpness/coverage lever. Capability is
/// STRUCTURAL: [`Fixed`](CsmFitMode::Fixed) IS "auto-fit off"; there is no separate flag.
/// Mirrors [`ShadowDenoiseMode`](crate::shadow_denoise_config::ShadowDenoiseMode) / `AaMode`
/// in shape — but NOT in which variant is default (see below).
///
/// # This knob does NOT trade quality for performance
///
/// Every variant costs the same on the GPU: same cascade count, same map resolution, same
/// draws, same shader. The only cost is a cold, once-per-frame O(caster instances) CPU fold
/// (`Fixed` skips even that). What the variants trade is WHERE the sharpness lands and
/// whether distant receivers keep a shadow. The quality/perf levers are
/// [`CsmConfig::resolution`] and [`CsmConfig::cascade_count`], which really do cost VRAM and
/// a depth pass each.
///
/// The caster modes ([`Shrink`](CsmFitMode::Shrink) / [`CatchAll`](CsmFitMode::CatchAll))
/// require [`reduce_caster_bounds`](crate::csm_caster::reduce_caster_bounds) to be
/// app-registered (`docs/CSM-AUTOFIT-PLAN.md` rung C5). Without it [`CsmCasterBounds`]
/// never leaves [`CsmCasterBounds::EMPTY`], the fit never latches, and EVERY mode renders
/// as `Fixed` — silently, at zero cost. `EnginePlugins` registers it; a bare `CsmPlugin`
/// world does not, which is why that degradation is graceful rather than a panic.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CsmFitMode {
    /// Split `[view.near, min(view.far, shadow_distance)]` — the camera's whole shadow range,
    /// ignoring where casters actually are. BIT-IDENTICAL to the pre-auto-fit engine, and the
    /// caster bounds/latch state are never read (0 ns), so this is the exact opt-out for a
    /// scene that wants the old bytes back.
    ///
    /// NOT the default (owner call, 2026-07-16): splitting by the camera spends cascades on
    /// caster-free range while the tail cascade smears the casters that fall into it — at the
    /// shipped config a receiver just past cascade 1's edge lands on a 22-unit cascade whose
    /// texel is ~3.6 screen px. Paying that by default bought nothing.
    Fixed,
    /// F-shrink: all `N` cascades split `[view.near, far_eff]`. One extra cascade over the
    /// caster range; receivers beyond `far_eff` are FULLY LIT with no cross-fade
    /// (`shadow_apply.hlsli`'s `sel >= gCsmActive` early-out) — a HARD terminator that
    /// relocates into the visible scene and jumps up to 29.3% per latch transition. Maximum
    /// sub-range subdivision, at the cost of the terminator.
    Shrink,
    /// F-catch-all: cascades `0..N-2` split `[view.near, far_eff]`; cascade `N-1` is
    /// RESERVED for `[far_eff, far_cap]`, so distant shadows survive and the last split
    /// stays at `shadow_distance` exactly as today (no terminator). One of `N` cascades is
    /// spent on caster-free range. `cascade_count < 2` degenerates to `Fixed` (cannot
    /// reserve the only cascade).
    ///
    /// **The DEFAULT** (owner call, 2026-07-16, after an eval on `examples/room.rs`'s scene):
    /// it keeps the casters inside a tight cascade instead of letting them fall off its edge
    /// into the oversized tail, and it costs nothing to do so. Measured there: a receiver at
    /// view-depth ~8 goes from texel 0.0366 (~3.6 screen px) to 0.0112 (~1.1 px) — 3.2×,
    /// which also narrows the 13-tap PCF penumbra by the same factor, since that tent is
    /// measured in TEXELS. The trade, stated honestly: a receiver at ~6 gets ~20% coarser
    /// (0.0093 → 0.0112), because this mode redistributes sharpness toward the casters rather
    /// than adding any.
    #[default]
    CatchAll,
}

// ---- CsmPcfKernel (the CSM shadow-edge PCF tap-count knob, rung E1) -------------------

/// The CSM shadow-edge PCF kernel — the owner's SHARPNESS/anti-crawl trade (rung E1). Every
/// variant samples the SAME `gCsm`/`gCsmCmp` combined descriptor (binding 12) through a
/// wave-UNIFORM runtime branch in `csm_pcf_disc` (`shadow_apply.hlsli`) — no extra `.spv`, no
/// per-lane divergence (the word is identical for every pixel in every wave; `deferred_pbr.hlsl`'s
/// `shadow_mode` uses the same idiom). `#[repr(u32)]` so it forwards to the shader's
/// `gCsmPcfKernel` (via [`ResolvedCsm::pcf_kernel_word`]) as a stable mode word. CSM-only —
/// the spot/point atlas (`atlas_pcf_disc`) is unaffected; `FaceTransform` has no spare pad, and
/// the motivating measurement (cascade-texel variance across the fit) does not apply there.
///
/// # The penumbra IS the anti-crawl ramp — this is NOT a free perf knob
///
/// Measured from `resolve_csm`'s own outputs (the shipped [`CsmFitMode::CatchAll`] fit,
/// 2048-map resolution): [`Tent13`](Self::Tent13)'s ~10-texel footprint is a ~9–11 screen-px
/// penumbra at every cascade's FAR edge, widening to ~30–52 screen px at its NEAR edge
/// (`csm_pcf_disc`'s doc in `shadow_apply.hlsli` has the full derivation). A single-tap compare
/// CRAWLS under sub-pixel camera motion (the shadow-motion A/B harness: a 3 mrad yaw flips
/// shadow-edge pixels at near-full swing, 226/255); PCF cures it by turning the binary edge into
/// a ramp, so NARROWING the kernel re-admits crawl — the tap count is the only cost term, and
/// every variant binds the identical descriptor and runs the identical `.spv`, so this is a pure
/// sharpness/anti-crawl trade, not a quality/perf lever like [`CsmConfig::resolution`].
///
/// # Why a narrower kernel is safe once TAA is armed
///
/// Rung C1 (`TaaConfig::jitter_scope == RasterAndBasis`, `crate::taa_config`) is the host-side
/// camera-basis shear that makes TAA's Halton jitter reach the shading sample position — proven
/// bit-exactly: with the mesh leg disabled, phase 0 vs phase 4 went from 0 differing pixels
/// (jitter reached no SDF shading at all) to 70 662. With [`AaMode::Taa`](crate::aa_config::AaMode::Taa)
/// armed and the shear on, temporal accumulation supplies the sample variance a narrower spatial
/// kernel alone cannot — so [`Cross5`](Self::Cross5)/[`Bilinear1`](Self::Bilinear1) become
/// viable trades. TAA is OPT-IN and OFF by default (`AaMode::Off`), so this knob's OWN default
/// must stand alone without it.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CsmPcfKernel {
    /// The 13-tap tent disc (center w4, the ±2 ring of 8 w2, the ±4 axis ring of 4 w1, sum 24)
    /// — ~10-texel support with the hardware 2x2 bilinear folded in. The crawl-free anchor: the
    /// shadow-motion A/B harness proved a 3x3 (1-texel) kernel did NOT suppress crawl, and this
    /// is the narrowest kernel that did. **The DEFAULT** — `Tent13 == 0` is LOAD-BEARING (not a
    /// coincidence): a zeroed [`ResolvedCsm`] UBO (the disabled selection, or any producer that
    /// never sets [`CsmConfig::pcf_kernel`]) MUST degrade to this crawl-free kernel, never to a
    /// crawling one.
    #[default]
    Tent13,
    /// A 5-tap cross (center w4, the ±2 axis ring of 4 w2, sum 12) — ~6-texel support, about
    /// half `Tent13`'s footprint. Sharper cascade edges at the cost of some crawl resistance;
    /// intended for a scene running `AaMode::Taa` (rung C1's basis shear), where temporal
    /// accumulation supplies the sample variance `Tent13`'s wider spatial ramp exists to fake.
    Cross5,
    /// The bare hardware 2x2 PCF comparison — 1 tap, no disc, ~2-texel support (the hardware
    /// bilinear alone). The sharpest edge this engine can produce. **CRAWLS without TAA** — a
    /// near-identical single-tap kernel is what the shadow-motion A/B harness measured flipping
    /// shadow-edge pixels at near-full swing under a 3 mrad camera yaw; only pair this with
    /// `AaMode::Taa` + `jitter_scope == RasterAndBasis`, and even then expect visible boil the
    /// wider kernels do not have.
    Bilinear1,
}

impl CsmPcfKernel {
    /// The stable mode word forwarded to the shader as `gCsmPcfKernel`
    /// ([`ResolvedCsm::pcf_kernel_word`]) — the `#[repr(u32)]` discriminant. `Tent13 => 0`,
    /// `Cross5 => 1`, `Bilinear1 => 2`.
    #[inline]
    pub const fn as_word(self) -> u32 {
        self as u32
    }
}

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
    /// The cascade split-RANGE policy. Default [`CsmFitMode::CatchAll`] (fit the split range
    /// to the casters, reserving the last cascade for the rest) — see [`CsmFitMode`] for the
    /// measured trade and for why this is not a quality/perf knob.
    pub fit_mode: CsmFitMode,
    /// The CSM shadow-edge PCF kernel (rung E1) — the owner's sharpness/anti-crawl trade.
    /// Default [`CsmPcfKernel::Tent13`] (the crawl-free anchor) — see [`CsmPcfKernel`] for the
    /// measured penumbra numbers and the honest trade each variant makes.
    pub pcf_kernel: CsmPcfKernel,
}

impl Default for CsmConfig {
    /// The DISABLED default (`cascade_count == 0` — the 0%-gate): a default world resolves
    /// the all-zero [`ResolvedCsm`] and touches no render path. The remaining fields carry
    /// the research defaults so that flipping `cascade_count` to a positive value yields a
    /// usable fit without further tuning — which now includes fitting the split range to the
    /// casters ([`CsmFitMode::CatchAll`]) and the crawl-free PCF kernel
    /// ([`CsmPcfKernel::Tent13`], rung E1), since splitting by the camera instead spends
    /// cascades on caster-free range at no saving.
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
            fit_mode: DEFAULT_FIT_MODE,
            pcf_kernel: DEFAULT_PCF_KERNEL,
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
    /// The CSM shadow-edge PCF kernel selection ([`CsmPcfKernel::as_word`], rung E1) — mirrors
    /// the shader's `gCsmPcfKernel`. `0` ([`CsmPcfKernel::Tent13`]) is the crawl-free default
    /// AND the value a zeroed/disabled UBO carries (see that variant's doc): a producer that
    /// never sets [`CsmConfig::pcf_kernel`] degrades to the safe kernel, never a crawling one.
    pub pcf_kernel_word: u32,
    /// Padding to a 16-byte stride after the three trailing `u32` words.
    pub _pad: u32,
}

// Layout pin: 80 × 4 + 4 + 4 + 4 + 4 = 320 + 16 = 336 B.
const _: () = assert!(size_of::<ResolvedCsm>() == 336);

/// The byte size of the host-coherent CSM cascade UBO — `size_of::<ResolvedCsm>()`
/// (336 B: `[CascadeData; 4]` + `active_count` + `csm_mode_word` + `pcf_kernel_word` + pad).
/// The resolve binds a UBO of exactly this shape at binding 13; hosts size their cascade-UBO
/// ring slots from THIS constant (single source — no hand-copied `336`).
pub const RESOLVED_CSM_BYTES: usize = size_of::<ResolvedCsm>();

impl ResolvedCsm {
    /// The disabled selection — all-zero cascades, `active_count == 0`, `csm_mode_word ==
    /// 0`, `pcf_kernel_word == 0` ([`CsmPcfKernel::Tent13`], harmless here since shadows are
    /// off). The resolve of a disabled [`CsmConfig`] and the value [`ResolvedCsm::default`]
    /// returns.
    pub const DISABLED: Self = Self {
        cascades: [CascadeData::ZERO; MAX_CASCADES],
        active_count: 0,
        csm_mode_word: 0,
        pcf_kernel_word: 0,
        _pad: 0,
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
///    `[near, pssm_far]` by `λ`, where `pssm_far` is `far_cap_today =
///    min(view.far, shadow_distance)` when `fit.far_eff` is `None`, or the caster-fitted
///    `far_eff` otherwise (see "The `fit` parameter" below). The splits are a fixed
///    function of `(near, pssm_far, λ, pssm_n)` — static, no shimmer.
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
///    harness): the light eye is pulled back along `+sun_dir` by `pull_back` world units
///    (`docs/CSM-AUTOFIT-PLAN.md` D5, rung C4) — `diameter` under [`CsmFit::NONE`] (today,
///    bounding casters between the sun and the slice symmetrically), or grown to also cover
///    any caster up-sun of the fitted sphere when `fit.caster_aabb` is populated, clamped to
///    `[diameter, MAX_PULLBACK_RATIO * diameter]` so it never shrinks below today's capture
///    and never grows unbounded. `z_far = pull_back + diameter` (down-sun capture stays
///    `diameter`, as today), and the combined `view_proj` maps world → light clip as
///    `clip.x = x_lv/r`,
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
///
/// # The `fit` parameter (`docs/CSM-AUTOFIT-PLAN.md` rung C3)
///
/// `resolve_csm` does NOT consult [`CsmConfig::fit_mode`] — it is driven entirely by the
/// already-decided `fit: CsmFit`, which [`resolve_csm_cascades`] computes from `fit_mode` +
/// the caster bounds + the anti-shimmer latch BEFORE calling in. This keeps the fit
/// mechanics (this function) oblivious to mode NAMES, so a new mode is a
/// `resolve_csm_cascades`-side change only.
///
/// - [`CsmFit::NONE`] (`far_eff: None`, `caster_aabb: None`) ⇒ TEXTUALLY today's fit: the
///   PSSM split range is `[near, far_cap_today]` over all `count` cascades, and the
///   sun-axis pull-back reduces to `pull_back = diameter` ⇒ `z_far = 2·diameter` /
///   `eye = center + sun·(z_far·0.5)` — the D10 byte-identity claim, pinned by
///   `fixed_mode_is_bit_identical_to_today`.
/// - `Some(far_eff)` with `reserve_tail: false` (`Shrink`) ⇒ all `count` cascades split
///   `[near, far_eff]`.
/// - `Some(far_eff)` with `reserve_tail: true` (`CatchAll`) ⇒ cascades `0..count-1` split
///   `[near, far_eff]`; cascade `count-1` is reserved for `[far_eff, far_cap_today]`
///   (`near_i`'s cross-cascade chaining is untouched, so the reserved cascade naturally
///   slices that tail range).
///
/// The sun-axis pull-back (`docs/CSM-AUTOFIT-PLAN.md` D5, rung C4) is driven independently
/// by `fit.caster_aabb` — populated by the SAME caster modes that populate `far_eff`, but
/// logically separate (a `Shrink`/`CatchAll` fit with `caster_aabb: None`, as a synthetic
/// test input, still shrinks the split range without growing the pull-back).
///
/// # The PCF kernel (rung E1)
///
/// `resolved.pcf_kernel_word` is `cfg.pcf_kernel.as_word()` verbatim — a config knob, not
/// part of the fit, so it carries through unchanged regardless of `fit`.
#[inline]
pub fn resolve_csm(
    cfg: &CsmConfig,
    view: &ViewUniform,
    sun_dir: [f32; 3],
    fit: CsmFit,
) -> ResolvedCsm {
    if !cfg.enabled() {
        return ResolvedCsm::DISABLED;
    }

    debug_assert!(
        view.fov_y != 0.0,
        "CSM requires a perspective camera this phase (ViewUniform has no ortho half-extents)"
    );

    let count = (cfg.cascade_count as usize).min(MAX_CASCADES);

    let eye = view.camera_pos.xyz();
    let forward = view.cam_forward.xyz();
    let right = view.cam_right.xyz();
    let up = view.cam_up.xyz();

    let near = view.near;
    // The last cascade's far is capped at the owner's shadow distance (and never beyond the
    // camera far). `enabled()` guarantees `shadow_distance > 0`; clamp to `> near` so the
    // partition is well-formed even for a misconfigured near/distance pair. Renamed (not
    // rewritten) from `far_cap` -- `fit.far_eff == None` still reduces to EXACTLY this
    // value (D10's byte-identity claim).
    let far_cap_today = view.far.min(cfg.shadow_distance).max(near + MIN_DIAMETER);

    debug_assert!(
        fit.far_eff.is_none_or(|f| f.is_finite() && f > near && f <= far_cap_today),
        "resolve_csm: fit.far_eff must be finite, > near, and <= far_cap_today"
    );
    debug_assert!(
        !fit.reserve_tail || count >= 2,
        "resolve_csm: fit.reserve_tail requires at least 2 cascades (got count={count})"
    );

    // docs/CSM-AUTOFIT-PLAN.md algorithm C edit 1: `fit.far_eff == None` reduces to
    // EXACTLY today's math (`pssm_far = far_cap_today`, `pssm_n = count`, no reserved
    // tail). `Some(f)` with `reserve_tail` is `CatchAll` (cascade `count-1` is reserved for
    // `[f, far_cap_today]`); `Some(f)` without is `Shrink` (all `count` cascades split
    // `[near, f]`).
    let (pssm_far, pssm_n, tail) = match fit.far_eff {
        None => (far_cap_today, count, None),
        Some(f) if fit.reserve_tail => (f, count - 1, Some(far_cap_today)),
        Some(f) => (f, count, None),
    };

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

    // docs/CSM-AUTOFIT-PLAN.md D5 (rung C4): the 8 world corners of the caster union AABB,
    // hoisted once (the AABB itself is constant across cascades — only the per-cascade
    // `center` the pull-back is measured from varies).
    let caster_corners = fit.caster_aabb.map(|(mn, mx)| aabb_corners(mn, mx));

    let mut cascades = [CascadeData::ZERO; MAX_CASCADES];

    let mut near_i = near;
    for (i, slot) in cascades.iter_mut().enumerate().take(count) {
        // docs/CSM-AUTOFIT-PLAN.md algorithm C edit 2: the reserved tail cascade
        // (`CatchAll`'s `count-1`) takes `far_cap_today` DIRECTLY, not another PSSM split
        // -- `near_i` still chains from the previous split (untouched below), so this
        // cascade naturally slices `[far_eff, far_cap_today]`.
        let split_i = match tail {
            Some(t) if i == count - 1 => t,
            _ => pssm_split(near, pssm_far, cfg.lambda, i + 1, pssm_n as f32),
        };

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
        //
        // docs/CSM-AUTOFIT-PLAN.md algorithm C edit 3 (D5, rung C4): the light eye pulls
        // back along +sun far enough to also capture any caster UP-SUN of the fitted
        // sphere -- otherwise shrinking `diameter` (the caster-fit's entire point) would
        // silently clip an up-sun caster (a ceiling beam, a tall pillar) that the
        // un-fitted `z_far = 2*diameter` used to capture for free. `None` (Fixed /
        // unlatched / no usable bounds) reduces to `up_need = diameter` with ZERO extra
        // float ops -- see `fixed_mode_is_bit_identical_to_today` for the exactness proof.
        let up_need = match &caster_corners {
            None => diameter,
            Some(corners) => {
                let mut raw_up = f32::MIN;
                for &c in corners {
                    raw_up = raw_up.max(sun.dot(c - center));
                }
                // <= 0 => every corner is down-sun (or AT the center) of this cascade;
                // nothing to grow for -- fall back to today's symmetric pull-back.
                if raw_up > 0.0 {
                    grid_ceil(raw_up)
                } else {
                    diameter
                }
            }
        };
        // >= diameter ALWAYS (never worse than today) and <= MAX_PULLBACK_RATIO * diameter
        // (bounded depth-precision / bias-scale drift, D5/D9).
        let pull_back = up_need.clamp(diameter, MAX_PULLBACK_RATIO * diameter);
        debug_assert!(
            pull_back >= diameter,
            "resolve_csm: pull_back must never fall below diameter (D5 invariant)"
        );
        // Down-sun capture stays `diameter`, exactly as today.
        let z_far = pull_back + diameter;
        let eye = center + sun * pull_back;
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
        pcf_kernel_word: cfg.pcf_kernel.as_word(),
        _pad: 0,
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

/// The 8 world-space corners of the AABB `[mn, mx]` — `docs/CSM-AUTOFIT-PLAN.md` D5 (rung
/// C4): the sun-axis pull-back's caster-coverage term reads the caster union AABB as its 8
/// corners (not just `mn`/`mx`), since the up-sun-most CORNER, not the up-sun-most face, is
/// what the light eye must clear.
#[inline]
fn aabb_corners(mn: [f32; 3], mx: [f32; 3]) -> [Vec3; 8] {
    let mut corners = [Vec3::ZERO; 8];
    let mut k = 0;
    for &x in &[mn[0], mx[0]] {
        for &y in &[mn[1], mx[1]] {
            for &z in &[mn[2], mx[2]] {
                corners[k] = Vec3::new(x, y, z);
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

// ---- the log-quantization grid + Schmitt latch ------------------------------------------
//
// Landed standalone in C1 (`docs/CSM-AUTOFIT-PLAN.md` Decision D6) so the exactness and
// no-limit-cycle properties were unit-testable before a caller existed. Rung C3's
// `select_fit` is now that caller — reached only when `cfg.fit_mode != Fixed`.

/// Exact `2^q` as `f32`, built directly from its IEEE-754 bits (sign `0`, exponent field
/// `q + 127`, mantissa `0`) — NOT `powf`, which is a libm call with no cross-platform
/// bit-exactness guarantee and would silently break the `grid_cell`/`grid_value` round-trip
/// (D6, T10). `f32::from_bits` is a safe reinterpretation of an already-valid bit pattern,
/// not a transmute.
#[inline]
fn exp2i(q: i32) -> f32 {
    debug_assert!(
        (EXP2I_MIN_EXPONENT..=EXP2I_MAX_EXPONENT).contains(&q),
        "exp2i: exponent {q} outside the safe normal f32 range [{EXP2I_MIN_EXPONENT}, {EXP2I_MAX_EXPONENT}]"
    );
    let q = if (EXP2I_MIN_EXPONENT..=EXP2I_MAX_EXPONENT).contains(&q) {
        q
    } else {
        exp2i_clamp(q)
    };
    let bits = ((q + 127) as u32) << 23;
    f32::from_bits(bits)
}

/// Clamps an out-of-range exponent into `exp2i`'s safe normal range — `#[cold]` /
/// `#[inline(never)]` per Principle 3 (I-cache): only a pathological `k` (many octaves off
/// scene scale) reaches this, so it stays off `exp2i`'s straight-line bit-shift body.
#[cold]
#[inline(never)]
fn exp2i_clamp(q: i32) -> i32 {
    q.clamp(EXP2I_MIN_EXPONENT, EXP2I_MAX_EXPONENT)
}

/// The far-grid value of cell `k`: `FIT_GRID_TABLE[k mod 4] · 2^(k div 4)`. Exact — a
/// normalized mantissa times an exact power of two only ever rewrites the exponent field, so
/// this never rounds (unlike a `powf`/`powi` reconstruction, which would accumulate error
/// across octaves). This exactness, together with `grid_cell`'s matching bit decomposition,
/// is what makes `grid_cell(grid_value(k)) == k` hold bit-for-bit on every platform (D6,
/// T10) — a property `powf(1.18921, k)` cannot give.
#[inline]
pub fn grid_value(k: i32) -> f32 {
    let phase = k.rem_euclid(FIT_GRID_CELLS_PER_OCTAVE) as usize;
    let exp = k.div_euclid(FIT_GRID_CELLS_PER_OCTAVE);
    debug_assert!(
        (EXP2I_MIN_EXPONENT..=EXP2I_MAX_EXPONENT).contains(&exp),
        "grid_value: k={k} decodes to exponent {exp}, outside the safe range [{EXP2I_MIN_EXPONENT}, {EXP2I_MAX_EXPONENT}]"
    );
    FIT_GRID_TABLE[phase] * exp2i(exp)
}

/// Recovers the grid cell `k` such that `grid_value(k) <= x < grid_value(k + 1)`, from `x`'s
/// IEEE-754 bits: the exponent field gives `k`'s octave directly, and the mantissa field —
/// reassembled with the bias exponent so it reads as a value in `[1.0, 2.0)` — is compared
/// against the SAME `FIT_GRID_TABLE` thresholds `grid_value` multiplies by. Scaling a float
/// by an exact power of two rewrites ONLY its exponent field, so the mantissa recovered here
/// from a `grid_value(k)` input is bit-identical to the `FIT_GRID_TABLE` entry that produced
/// it — which is why `grid_cell(grid_value(k)) == k` is exact, not approximate (D6, T10).
///
/// `x` must be finite and `> 0` (debug_assert). A subnormal, zero, negative, infinite or NaN
/// `x` — never produced by a well-formed caster/camera distance — is clamped to the smallest
/// normal magnitude by the `#[cold]` path below rather than mis-decoded (a subnormal's
/// mantissa has no implicit leading `1`, so reading its bits through the normal-number path
/// would silently recover the wrong exponent).
#[inline]
pub fn grid_cell(x: f32) -> i32 {
    debug_assert!(
        x.is_finite() && x > 0.0,
        "grid_cell: x must be finite and > 0, got {x}"
    );
    let x = if x.is_finite() && x >= f32::MIN_POSITIVE {
        x
    } else {
        grid_cell_clamp(x)
    };

    let bits = x.to_bits();
    let exp = ((bits >> 23) & 0xFF) as i32 - 127;
    // Reassemble the mantissa bits with the bias exponent field (127) so the value reads in
    // [1.0, 2.0) — the SAME domain FIT_GRID_TABLE lives in.
    let m = f32::from_bits((bits & 0x007F_FFFF) | 0x3F80_0000);

    // Branchless: FIT_GRID_TABLE is sorted, so the count of thresholds `<= m` IS the phase.
    let phase = (m >= FIT_GRID_TABLE[1]) as i32
        + (m >= FIT_GRID_TABLE[2]) as i32
        + (m >= FIT_GRID_TABLE[3]) as i32;

    exp * FIT_GRID_CELLS_PER_OCTAVE + phase
}

/// Clamps a non-finite / non-positive / subnormal `x` to the smallest NORMAL `f32` magnitude
/// before `grid_cell` decodes its bits — `#[cold]` / `#[inline(never)]` per Principle 3
/// (I-cache): grid inputs are scene-scale (metre) camera/caster distances, so this path
/// fires only under a misconfigured scene or an adversarial test, and keeping it out of
/// `grid_cell`'s body keeps that body a straight-line bit decode.
#[cold]
#[inline(never)]
fn grid_cell_clamp(_x: f32) -> f32 {
    f32::MIN_POSITIVE
}

/// The smallest grid value strictly greater than `x`: `grid_value(grid_cell(x) + 1)`.
/// `x` must be finite and `> 0` (debug_assert, inherited from [`grid_cell`]). Used by the
/// sun-axis pull-back (`docs/CSM-AUTOFIT-PLAN.md` D5, rung C4) to quantize `up_need` onto
/// the SAME log grid `far_eff` lives on, so a caster's up-sun capture requirement is rounded
/// UP to a frame-stable value (never clips, mirrors `latch_cell`'s grow branch) instead of
/// tracking the caster's exact depth continuously — which would make the pull-back itself a
/// shimmer channel.
#[inline]
fn grid_ceil(x: f32) -> f32 {
    grid_value(grid_cell(x) + 1)
}

/// The asymmetric Schmitt latch (D6). `prev_k` is the previously latched cell (or
/// `FIT_UNLATCHED` for a fresh latch — mirrors `CsmFitState::UNLATCHED`, wired in C3).
///
/// - **Fresh** (`prev_k == FIT_UNLATCHED`) or **grow** (`raw > grid_value(prev_k)`):
///   immediate — `grid_cell(raw) + 1`, the cell whose value always satisfies `far_eff >=
///   raw` (T10's never-clips property). Grow MUST be immediate: `far_eff` is a HARD upper
///   bound the `Shrink` fit mode caps `far_cap` at (D2), so a delayed grow would clip
///   casters the frame `raw` first exceeds it.
/// - **Shrink** (`raw < grid_value(prev_k - FIT_SHRINK_BAND_CELLS)`): only after `raw` has
///   fallen a full 2 cells (≈ −29.3%) below the latched value — `grid_cell(raw) + 1`.
/// - **Else**: sticky — returns `prev_k` unchanged. This is what discharges the anti-dither
///   obligation (T18): any oscillation smaller than the grow/shrink bands re-quantizes zero
///   times, because neither predicate above trips.
///
/// No limit cycle: after a grow `prev_k -> k+1`, `raw` is just above `grid_value(k) ==
/// grid_value((k+1) - 1) > grid_value((k+1) - 2)`, so the shrink predicate is false at the
/// instant the grow lands — the latch cannot immediately re-trigger shrink on the same
/// sample.
#[inline]
pub fn latch_cell(raw: f32, prev_k: i32) -> i32 {
    debug_assert!(
        raw.is_finite() && raw > 0.0,
        "latch_cell: raw must be finite and > 0, got {raw}"
    );

    if prev_k == FIT_UNLATCHED {
        return grid_cell(raw) + 1;
    }
    if raw > grid_value(prev_k) {
        return grid_cell(raw) + 1;
    }
    if raw < grid_value(prev_k - FIT_SHRINK_BAND_CELLS) {
        return grid_cell(raw) + 1;
    }
    prev_k
}

// ---- CsmCasterBounds (`docs/CSM-AUTOFIT-PLAN.md`). Folded once per frame by
// `csm_caster::reduce_caster_bounds` from `CsmCasterScratch`'s existing gather output.
// Landed dark in C2; rung C3's `select_fit` (called from `resolve_csm_cascades`) is its
// first consumer, reached only when `cfg.fit_mode != Fixed`. -----------------------------

/// The caster-derived fit input, folded once per frame by
/// [`reduce_caster_bounds`](crate::csm_caster::reduce_caster_bounds) from
/// [`CsmCasterScratch`](crate::csm_caster::CsmCasterScratch)'s existing `batches()` +
/// `ring()` — the shadow-caster gather's OUTPUT, not a second query (Decision D7,
/// `docs/CSM-AUTOFIT-PLAN.md`).
///
/// # NOT a caster-presence authority
///
/// [`sync_csm_light_gate`](crate::csm_caster::sync_csm_light_gate) (`csm_mode_word == 1
/// && batch_count() > 0`) remains the SINGLE predicate for "do we have casters" and is
/// untouched by this type. The counters here exist ONLY to tell a future fit whether this
/// frame's fold is usable as an input — a batch whose mesh has not yet resolved `Loaded`
/// (streaming) is SKIPPED (the F6 never-deref invariant), which makes the fold
/// INCOMPLETE, not caster-less. Never read `resolved_batches`/`total_batches` to decide
/// whether shadows are on; that decision belongs exclusively to `sync_csm_light_gate`.
///
/// Not `#[repr(C)]`, no size pin — this never reaches the GPU (a CPU-only fit input). 36 B.
///
/// # Read only when `fit_mode != Fixed` (rung C3)
///
/// [`select_fit`] reads this Resource, but ONLY past the `CsmFitMode::Fixed` 0%-gate — a
/// world that never sets `fit_mode` never reads it. [`CsmPlugin`](crate::csm_plugin::CsmPlugin)
/// inserts [`CsmCasterBounds::EMPTY`] so a bare-`CsmPlugin` world never panics resolving
/// it, but the fold only runs once the owning app registers
/// [`reduce_caster_bounds`](crate::csm_caster::reduce_caster_bounds) — an unwired exported
/// API (rung C5). Until then this Resource stays `EMPTY` forever: inert, zero cost, and
/// the fit falls back to `Fixed` for any non-`Fixed` mode too (never latches).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct CsmCasterBounds {
    /// Max VIEW-space depth over all caster instances, reduced PER INSTANCE — each
    /// instance's own world AABB (the abs-matrix/Arvo transform of its mesh's model-space
    /// AABB) is individually projected onto the view axis, then `max`'d. Deliberately NOT
    /// the projection of the union AABB: two casters at the same depth but opposite
    /// lateral extremes would otherwise inflate this value by their lateral separation
    /// (D4). Valid iff `resolved_batches > 0`.
    pub raw_far: f32,
    /// World-space UNION AABB minimum of all caster instances. Used ONLY by the sun-axis
    /// pull-back term ([`CsmFit::caster_aabb`], `docs/CSM-AUTOFIT-PLAN.md` D5, rung C4),
    /// where a union bound is exactly what is wanted — NOT for `raw_far` (see its doc).
    pub world_min: [f32; 3],
    /// World-space union AABB maximum. See [`Self::world_min`].
    pub world_max: [f32; 3],
    /// Batches whose mesh resolved via `try_get` to a `Loaded`, valid (non-inverted) AABB
    /// and contributed to the fold.
    pub resolved_batches: u32,
    /// Batches the gather emitted this frame. `resolved_batches < total_batches` ⇒ the
    /// fold is INCOMPLETE (a streaming mesh, or a zero-vertex placeholder — see
    /// `reduce_bounds_into`'s doc).
    pub total_batches: u32,
}

impl CsmCasterBounds {
    /// No caster batches folded — every field's zero value, and the value a
    /// disabled/unregistered/streaming-incomplete fold holds.
    pub const EMPTY: Self = Self {
        raw_far: 0.0,
        world_min: [0.0; 3],
        world_max: [0.0; 3],
        resolved_batches: 0,
        total_batches: 0,
    };

    /// Whether this frame's fold is a usable fit input: at least one batch contributed,
    /// and EVERY emitted batch resolved (no streaming/inverted-box gap). A future fit
    /// (C3) must HOLD its previous decision, not read a partial fold, when this is false.
    #[inline]
    pub fn is_usable(&self) -> bool {
        self.resolved_batches > 0 && self.resolved_batches == self.total_batches
    }
}

impl Default for CsmCasterBounds {
    /// `Default == EMPTY` — a `CsmCasterBounds` inserted before the first fold (or on a
    /// world where the reducer is never registered) already reads as "no usable bounds."
    #[inline]
    fn default() -> Self {
        Self::EMPTY
    }
}

// ---- CsmFitState (C3, the anti-shimmer latch — `docs/CSM-AUTOFIT-PLAN.md` D6) ---------

/// The anti-shimmer hysteresis latch — CPU-ONLY state, deliberately NOT inside the
/// `#[repr(C)]` GPU-uploaded [`ResolvedCsm`] (that would entangle `DISABLED`/`Default`/
/// `PartialEq` with frame state and break the 336 B contract's purity). [`resolve_csm_cascades`]
/// is its SINGLE writer (the one-producer-per-field discipline).
#[derive(Resource, Clone, Copy, Debug, PartialEq, Default)]
pub struct CsmFitState {
    /// The latched grid cell of `far_eff`. [`CsmFitState::UNLATCHED`] ⇒ never latched ⇒
    /// the fit falls back to `Fixed` (ground-truth constraint 3 — a world that never has
    /// usable casters never latches).
    pub far_k: i32,
}

impl CsmFitState {
    /// Never latched. Defined AS the private [`FIT_UNLATCHED`] sentinel (not a second
    /// `i32::MIN` literal), so the two can never drift apart — `latch_cell`'s "fresh latch"
    /// branch and this Resource's "never latched" state are the SAME value by construction.
    pub const UNLATCHED: i32 = FIT_UNLATCHED;
}

// `Default::default()` gives `far_k == 0`, which is a VALID grid cell, NOT `UNLATCHED` —
// `CsmPlugin` inserts `CsmFitState { far_k: CsmFitState::UNLATCHED }` explicitly; this
// derive exists only for the Resource trait's derive-completeness, never for insertion
// (`resolve_csm_cascades` debug-asserts the inserted state is `UNLATCHED` on frame 0 via
// its own gate logic never HOLD-ing a value it has not itself latched).

// ---- CsmFit (C3, the already-decided fit handed to the pure resolve) ------------------

/// The already-latched fit decision handed to the PURE [`resolve_csm`]. [`CsmFit::NONE`] ==
/// "Fixed / unlatched / no usable bounds" == today's fit EXACTLY — `resolve_csm` does not
/// consult [`CsmConfig::fit_mode`] at all; the caller ([`resolve_csm_cascades`], via
/// [`select_fit`]) has already decided.
///
/// # `caster_aabb` — the sun-axis pull-back input (`docs/CSM-AUTOFIT-PLAN.md` D5, rung C4)
///
/// Shrinking `diameter` (the split-range fit's entire point) would, unfixed, also shrink
/// the up-sun caster capture `resolve_csm`'s light eye pull-back provides — silently
/// vanishing a caster between the sun and the fitted slice (a ceiling beam, a tall pillar).
/// `caster_aabb` carries the world union caster AABB so `resolve_csm` can grow the pull-back
/// to cover it. `None` ⇒ `pull_back = diameter`, today's math EXACTLY (byte-identical).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct CsmFit {
    /// The quantized, latched split far. `None` ⇒ `far_cap_today` (today's math).
    pub far_eff: Option<f32>,
    /// `true` ⇒ reserve cascade `count - 1` for `[far_eff, far_cap_today]` (`CatchAll`).
    /// Meaningless when `far_eff` is `None`.
    pub reserve_tail: bool,
    /// The world-space union caster AABB (`CsmCasterBounds::world_min`/`world_max`), for the
    /// sun-axis pull-back. `None` ⇒ `pull_back = diameter` (today, byte-identical) — the
    /// value under `Fixed` / unlatched / no usable bounds.
    pub caster_aabb: Option<([f32; 3], [f32; 3])>,
}

impl CsmFit {
    /// `Fixed` / unlatched / no usable bounds — today's fit exactly.
    pub const NONE: Self = Self { far_eff: None, reserve_tail: false, caster_aabb: None };
}

// ---- CsmResolveSet (the cross-plugin caster-bounds → fit-resolve ordering seam) -------

/// The `Main`-schedule ordering seam that pins [`resolve_csm_cascades`] AFTER the
/// caster-bounds fold ([`reduce_caster_bounds`](crate::csm_caster::reduce_caster_bounds),
/// [`CsmFitSet`](crate::csm_caster::CsmFitSet)) — the CSM-fit analogue of
/// [`DdgiResolveSet`](crate::ddgi_config::DdgiResolveSet) /
/// [`PunctualResolveSet`](crate::shadow_atlas::PunctualResolveSet).
///
/// # Why a named set, not add-order
///
/// `reduce_caster_bounds` is registered at a DIFFERENT call site than
/// `resolve_csm_cascades` (rung C5 wires the app-level edge), so their per-system
/// `SystemKey`s are not co-visible — a `.after(key)` edge cannot cross that boundary. A
/// set-to-set edge is pinned BY NAME and holds regardless of registration order:
/// [`CsmPlugin`](crate::csm_plugin::CsmPlugin) joins `resolve_csm_cascades`
/// `.in_set(CsmResolveSet)`; the owning app (rung C5) configures
/// `CsmResolveSet.after(CsmFitSet)`.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Debug)]
pub struct CsmResolveSet;

// ---- the cold StrategyPolicy system (mirrors resolve_ssao_policy) ---------------------

/// The pure, World-free fit-selection core (`docs/CSM-AUTOFIT-PLAN.md` algorithm B, rung
/// C3) [`resolve_csm_cascades`] wraps — mirrors
/// [`reduce_bounds_into`](crate::csm_caster::reduce_bounds_into)'s pure-core idiom, so the
/// gate + latch decision is unit-testable without constructing `Res`/`ResMut`.
///
/// # The gate, in order (section 8's edge-case table)
///
/// 1. `Fixed` ⇒ [`CsmFit::NONE`] IMMEDIATELY — `bounds`/`state` are not read past this
///    line (D10: zero staleness exposure on every existing golden).
/// 2. `CatchAll` with `cfg.cascade_count < 2` ⇒ [`CsmFit::NONE`] (cannot reserve the only
///    cascade) — checked BEFORE the latch, so it never touches `state`.
/// 3. `bounds.is_usable()`:
///    - Usable, but `raw_far <= near + MIN_DIAMETER` (every caster behind/at the camera)
///      ⇒ [`CsmFit::NONE`] — nothing usable to fit, and NOT latched (a degenerate value
///      must never become a Schmitt-latch anchor).
///    - Usable ⇒ [`latch_cell`] on `raw_far` clamped to `[near + MIN_DIAMETER,
///      far_cap_today]`; `state.far_k` is updated to the new cell.
///    - Not usable, but `state.far_k != UNLATCHED` ⇒ HOLD: reuse `state.far_k` UNCHANGED
///      (the streaming/blink-strobe fix, D7) — never reset once latched.
///    - Not usable and never latched ⇒ [`CsmFit::NONE`] (ground-truth constraint 3: a
///      world that never has usable casters never latches).
/// 4. `far_eff = grid_value(k)` clamped to `[near + MIN_DIAMETER, far_cap_today]`. If
///    `far_eff >= far_cap_today` (casters already reach the shadow distance) ⇒
///    [`CsmFit::NONE`] — nothing to shrink, and it would otherwise give `CatchAll` a
///    zero-width reserved tail.
/// 5. Otherwise: `CsmFit { far_eff: Some(far_eff), reserve_tail: fit_mode == CatchAll,
///    caster_aabb: Some((bounds.world_min, bounds.world_max)) }` — the CURRENT frame's
///    bounds, even on the HOLD path (rung C4, `docs/CSM-AUTOFIT-PLAN.md` D5): unlike
///    `far_eff`, the sun-axis pull-back this drives is not itself a shimmer channel (its
///    own grid-quantized [`grid_ceil`] already makes it frame-stable) and is clamped to
///    `[diameter, MAX_PULLBACK_RATIO * diameter]` regardless of input, so a stale/partial
///    AABB during a HOLD frame degrades no worse than `pull_back == diameter` (today).
#[inline]
fn select_fit(
    cfg: &CsmConfig,
    near: f32,
    far_cap_today: f32,
    bounds: &CsmCasterBounds,
    state: &mut CsmFitState,
) -> CsmFit {
    if cfg.fit_mode == CsmFitMode::Fixed {
        return CsmFit::NONE;
    }
    if cfg.fit_mode == CsmFitMode::CatchAll && cfg.cascade_count < 2 {
        return CsmFit::NONE;
    }

    let k = if bounds.is_usable() {
        let raw = bounds.raw_far;
        if raw <= near + MIN_DIAMETER {
            return CsmFit::NONE;
        }
        let clamped = raw.clamp(near + MIN_DIAMETER, far_cap_today);
        let k = latch_cell(clamped, state.far_k);
        state.far_k = k;
        k
    } else if state.far_k != CsmFitState::UNLATCHED {
        state.far_k
    } else {
        return CsmFit::NONE;
    };

    let far_eff = grid_value(k).clamp(near + MIN_DIAMETER, far_cap_today);
    if far_eff >= far_cap_today {
        return CsmFit::NONE;
    }

    CsmFit {
        far_eff: Some(far_eff),
        reserve_tail: cfg.fit_mode == CsmFitMode::CatchAll,
        caster_aabb: Some((bounds.world_min, bounds.world_max)),
    }
}

/// The cold CSM resolve policy — reads [`CsmConfig`] + the active [`ViewUniform`] + the
/// PRIMARY directional light, and writes the derived [`ResolvedCsm`]. The CSM analogue of
/// [`resolve_ssao_policy`](crate::ssao_config::resolve_ssao_policy) /
/// [`select_lighting_cull`](crate::light_policy::select_lighting_cull). It is the SINGLE
/// owner of [`ResolvedCsm`] (the one-producer write discipline). It is ALSO the SINGLE
/// writer of [`CsmFitState`] (`docs/CSM-AUTOFIT-PLAN.md` rung C3) — the anti-shimmer latch
/// the caster fit modes carry across frames.
///
/// # Primary sun selection
///
/// The fit needs ONE light direction. The primary directional is the FIRST
/// [`DirectionalLight`] the query yields — the same "first/primary directional" the SDF
/// marcher writes into `gMaterial.R` and the lighting resolve treats as the sun. With no
/// directional light present, [`ResolvedCsm`] is left at [`ResolvedCsm::DISABLED`] (no sun
/// ⇒ no cascades).
///
/// # The caster fit gate (rung C3)
///
/// `bounds`/`state` feed [`select_fit`], which decides — per [`CsmConfig::fit_mode`] — the
/// [`CsmFit`] handed to [`resolve_csm`]. Under the default [`CsmFitMode::Fixed`],
/// `select_fit` returns [`CsmFit::NONE`] immediately WITHOUT reading `bounds`/`state`, so a
/// pinned scene that never sets `fit_mode` is byte-identical to before this rung (D10).
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
    bounds: Res<CsmCasterBounds>,
    mut state: ResMut<CsmFitState>,
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

    let near = view.near;
    let far_cap_today = view.far.min(cfg.shadow_distance).max(near + MIN_DIAMETER);
    let fit = select_fit(&cfg, near, far_cap_today, &bounds, &mut state);

    *out = resolve_csm(&cfg, &view, sun.direction, fit);
}

#[cfg(test)]
mod tests {
    use super::*;

    use boyko_math::{Affine3A, Vec3, Vec4};
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
        let resolved = resolve_csm(&CsmConfig::default(), &view, [0.3, -1.0, 0.2], CsmFit::NONE);
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
        assert_eq!(ResolvedCsm::default(), resolve_csm(&CsmConfig::default(), &view, [0.0, -1.0, 0.0], CsmFit::NONE));
    }

    #[test]
    fn pssm_splits_monotonic_and_last_equals_far_cap() {
        let view = perspective_view(Vec3::new(0.0, 2.0, 0.0), 0.0, 0.0);
        let cfg = enabled_cfg();
        let resolved = resolve_csm(&cfg, &view, [0.3, -1.0, 0.2], CsmFit::NONE);
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

        let base = resolve_csm(&cfg, &perspective_view(eye, 0.0, 0.0), sun, CsmFit::NONE);
        // Rotating the camera yaw/pitch permutes the corner set but not the radius — the
        // anti-shimmer property. texel_size = diameter / resolution, so equal texel_size ⇒
        // equal diameter.
        for &(yaw, pitch) in &[
            (0.7_f32, 0.0_f32),
            (0.0, 0.4),
            (1.3, -0.5),
            (-2.1, 0.2),
        ] {
            let rotated = resolve_csm(&cfg, &perspective_view(eye, yaw, pitch), sun, CsmFit::NONE);
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
        let a = resolve_csm(&cfg, &view, sun, CsmFit::NONE);
        let b = resolve_csm(&cfg, &view, sun, CsmFit::NONE);
        assert_eq!(a, b, "the fit must be a deterministic (idempotent) function of its inputs");
    }

    #[test]
    fn alt_up_engaged_yields_finite_nonsingular_view_proj() {
        // Sun ≈ ±world-up: the alt-up guard must engage and keep every view_proj finite +
        // non-singular (a degenerate light-view right axis would otherwise NaN the matrix).
        let cfg = enabled_cfg();
        let view = perspective_view(Vec3::new(0.0, 2.0, 0.0), 0.0, 0.0);
        for &sun in &[[0.0_f32, 1.0, 0.0], [0.0, -1.0, 0.0], [0.001, 1.0, 0.001]] {
            let resolved = resolve_csm(&cfg, &view, sun, CsmFit::NONE);
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
        let resolved = resolve_csm(&cfg, &view, [0.0, -1.0, 0.0], CsmFit::NONE);
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
            let resolved = resolve_csm(&cfg, &perspective_view(eye, yaw, pitch), sun, CsmFit::NONE);
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
        let resolved = resolve_csm(&cfg, &view, [0.0, -1.0, 0.0], CsmFit::NONE);
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
            let resolved = resolve_csm(&cfg, &view, sun_dir, CsmFit::NONE);
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

    // ---- C3: the fit-mode knob + gate (T1-T9, T12, docs/CSM-AUTOFIT-PLAN.md section 10) ---

    /// The near/far_cap_today pair the C3 tests share (matches `enabled_cfg`'s
    /// `shadow_distance: 30.0` under a far-clipped `view.far: 1000.0`).
    const TEST_NEAR: f32 = 0.1;
    const TEST_FAR_CAP: f32 = 30.0;

    /// A usable `CsmCasterBounds` at the given `raw_far` (one resolved-of-one batch).
    fn usable_bounds(raw_far: f32) -> CsmCasterBounds {
        CsmCasterBounds { raw_far, resolved_batches: 1, total_batches: 1, ..CsmCasterBounds::EMPTY }
    }

    #[test]
    fn fixed_mode_is_bit_identical_to_today() {
        // Even with fully USABLE, populated caster bounds and a live latch, `Fixed` must
        // select CsmFit::NONE and must NOT touch the latch state (D10 -- bounds/state are
        // NEVER READ under Fixed).
        let cfg = CsmConfig { fit_mode: CsmFitMode::Fixed, ..enabled_cfg() };
        let view = perspective_view(Vec3::new(0.0, 2.0, 0.0), 0.0, 0.0);
        let mut state = CsmFitState { far_k: 42 };
        let state_before = state;

        let fit = select_fit(&cfg, TEST_NEAR, TEST_FAR_CAP, &usable_bounds(5.0), &mut state);
        assert_eq!(fit, CsmFit::NONE, "Fixed must select CsmFit::NONE regardless of usable bounds");
        assert_eq!(state, state_before, "Fixed must never write the latch state");

        // The wider byte-identity claim (D10) is pinned by the pre-C3 suite above: every
        // test at csm_config.rs's `resolve_csm` call sites passes with only a mechanical
        // `, CsmFit::NONE` argument added -- those already re-derive the SAME
        // matrices/splits/pull-back this `CsmFit::NONE` path carries.
        let sun = [0.3_f32, -1.0, 0.2];
        let a = resolve_csm(&cfg, &view, sun, fit);
        let b = resolve_csm(&cfg, &view, sun, CsmFit::NONE);
        assert_eq!(a, b, "Fixed's selected fit must resolve identically to CsmFit::NONE");
    }

    #[test]
    fn fit_is_bit_identical_at_rest() {
        // Property S1 (D6): a static camera + an already-decided fit resolves
        // bit-identically frame to frame -- shadows are exactly as rock-solid as today.
        let cfg = CsmConfig { fit_mode: CsmFitMode::CatchAll, ..enabled_cfg() };
        let view = perspective_view(Vec3::new(1.0, 3.0, -4.0), 0.4, -0.1);
        let sun = [0.2_f32, -1.0, 0.35];
        let fit = CsmFit { far_eff: Some(5.0), reserve_tail: true, caster_aabb: None };

        let a = resolve_csm(&cfg, &view, sun, fit);
        let b = resolve_csm(&cfg, &view, sun, fit);
        assert_eq!(a, b, "resolving the SAME (cfg, view, sun, fit) twice must be bit-identical");
    }

    #[test]
    fn camera_dolly_pop_count_and_magnitude_are_bounded() {
        // Property S2 (D6): sweeping the caster-depth signal `raw_far` (what a camera
        // dolly toward/away from a fixed caster set changes) monotonically over 8 grid
        // cells (a ratio of exactly 4x, `grid_value(-4)..grid_value(4)`, fine-grained at
        // 500 steps/cell -- the same resolution `latch_has_no_limit_cycle` uses) must
        // trigger at most one latch transition per grid-cell boundary crossed, and every
        // transition's magnitude must stay inside the documented +18.92% / -29.3% band.
        const STEPS: usize = 4000;
        let cfg = CsmConfig { fit_mode: CsmFitMode::CatchAll, ..enabled_cfg() };
        let lo = grid_value(-4);
        let hi = grid_value(4);
        let cells_span = grid_cell(hi) - grid_cell(lo);

        let run_sweep = |from: f32, to: f32, max_ratio: f32, min_ratio: f32| -> i32 {
            let mut state = CsmFitState { far_k: CsmFitState::UNLATCHED };
            let mut prev_k = CsmFitState::UNLATCHED;
            let mut transitions = 0i32;
            for i in 0..=STEPS {
                let t = i as f32 / STEPS as f32;
                let raw = from * (to / from).powf(t);
                let fit = select_fit(&cfg, TEST_NEAR, TEST_FAR_CAP, &usable_bounds(raw), &mut state);
                assert!(fit.far_eff.is_some(), "raw {raw} in [{lo},{hi}] must always be usable");
                if state.far_k != prev_k {
                    if prev_k != CsmFitState::UNLATCHED {
                        transitions += 1;
                        let ratio = grid_value(state.far_k) / grid_value(prev_k);
                        assert!(
                            ratio <= max_ratio && ratio >= min_ratio,
                            "transition ratio {ratio} outside the documented band [{min_ratio}, {max_ratio}]"
                        );
                    }
                    prev_k = state.far_k;
                }
            }
            transitions
        };

        // Receding (growing raw): immediate, at most one transition per cell, ratio > 1.
        let grow_transitions = run_sweep(lo, hi, 1.1893, 1.0);
        assert!(
            grow_transitions <= cells_span,
            "receding sweep must trigger at most one transition per grid-cell boundary \
             crossed ({grow_transitions} transitions over {cells_span} boundaries)"
        );

        // Approaching (shrinking raw): only after 2 full cells, ratio < 1.
        let shrink_transitions = run_sweep(hi, lo, 1.0, 1.0 / 1.414_22);
        assert!(
            shrink_transitions <= cells_span / 2 + 1,
            "approaching sweep must trigger at most one transition per TWO grid-cell \
             boundaries crossed ({shrink_transitions} transitions, span {cells_span})"
        );
    }

    #[test]
    fn unlatched_or_no_casters_falls_back_to_fixed() {
        // Ground-truth constraint 3: EMPTY bounds + never-latched state must fall back to
        // Fixed under BOTH caster modes.
        for mode in [CsmFitMode::Shrink, CsmFitMode::CatchAll] {
            let cfg = CsmConfig { fit_mode: mode, ..enabled_cfg() };
            let mut state = CsmFitState { far_k: CsmFitState::UNLATCHED };
            let fit = select_fit(&cfg, TEST_NEAR, TEST_FAR_CAP, &CsmCasterBounds::EMPTY, &mut state);
            assert_eq!(fit, CsmFit::NONE, "{mode:?}: EMPTY + unlatched must fall back to Fixed");
            assert_eq!(
                state.far_k,
                CsmFitState::UNLATCHED,
                "{mode:?}: must not latch off EMPTY/unusable bounds"
            );
        }
    }

    #[test]
    fn incomplete_bounds_hold_the_latch() {
        let cfg = CsmConfig { fit_mode: CsmFitMode::CatchAll, ..enabled_cfg() };
        let mut state = CsmFitState { far_k: CsmFitState::UNLATCHED };

        let fit_latched = select_fit(&cfg, TEST_NEAR, TEST_FAR_CAP, &usable_bounds(3.0), &mut state);
        assert!(fit_latched.far_eff.is_some());
        let k_latched = state.far_k;

        // Next frame: a mesh is streaming in -- resolved < total (incomplete fold).
        let incomplete = CsmCasterBounds { raw_far: 9.0, resolved_batches: 1, total_batches: 2, ..CsmCasterBounds::EMPTY };
        let fit_held = select_fit(&cfg, TEST_NEAR, TEST_FAR_CAP, &incomplete, &mut state);
        assert_eq!(state.far_k, k_latched, "an incomplete fold must not move the latch");
        assert_eq!(
            fit_held, fit_latched,
            "an incomplete fold must reproduce the previous frame's fit exactly"
        );
    }

    #[test]
    fn blinking_caster_does_not_strobe() {
        let cfg = CsmConfig { fit_mode: CsmFitMode::CatchAll, ..enabled_cfg() };
        let mut state = CsmFitState { far_k: CsmFitState::UNLATCHED };

        let bounds = usable_bounds(4.0);
        let fit0 = select_fit(&cfg, TEST_NEAR, TEST_FAR_CAP, &bounds, &mut state);
        assert!(fit0.far_eff.is_some());

        for i in 0..10 {
            let this_frame = if i % 2 == 0 { bounds } else { CsmCasterBounds::EMPTY };
            let fit = select_fit(&cfg, TEST_NEAR, TEST_FAR_CAP, &this_frame, &mut state);
            assert_eq!(fit, fit0, "frame {i}: a blinking caster set must never strobe the fit");
        }
    }

    #[test]
    fn casters_behind_camera_fall_back_to_fixed() {
        let cfg = CsmConfig { fit_mode: CsmFitMode::Shrink, ..enabled_cfg() };
        let mut state = CsmFitState { far_k: CsmFitState::UNLATCHED };

        let behind = usable_bounds(TEST_NEAR);
        let fit = select_fit(&cfg, TEST_NEAR, TEST_FAR_CAP, &behind, &mut state);
        assert_eq!(fit, CsmFit::NONE, "raw_far <= near + MIN_DIAMETER must fall back to Fixed");
        assert_eq!(
            state.far_k,
            CsmFitState::UNLATCHED,
            "a degenerate raw_far must never become a latch anchor"
        );
    }

    #[test]
    fn casters_reaching_shadow_distance_fall_back_to_fixed() {
        let cfg = CsmConfig { fit_mode: CsmFitMode::CatchAll, ..enabled_cfg() };
        let mut state = CsmFitState { far_k: CsmFitState::UNLATCHED };

        // Casters already at the shadow distance -- the clamped/quantized far_eff lands
        // AT (or past) far_cap_today.
        let far = usable_bounds(TEST_FAR_CAP);
        let fit = select_fit(&cfg, TEST_NEAR, TEST_FAR_CAP, &far, &mut state);
        assert_eq!(
            fit, CsmFit::NONE,
            "far_eff >= far_cap_today must fall back to Fixed (nothing to shrink)"
        );
    }

    #[test]
    fn catch_all_with_one_cascade_degenerates_to_fixed() {
        let cfg = CsmConfig { fit_mode: CsmFitMode::CatchAll, cascade_count: 1, ..CsmConfig::default() };
        let mut state = CsmFitState { far_k: CsmFitState::UNLATCHED };

        let fit = select_fit(&cfg, TEST_NEAR, TEST_FAR_CAP, &usable_bounds(3.0), &mut state);
        assert_eq!(fit, CsmFit::NONE, "CatchAll with < 2 cascades cannot reserve the only cascade");
        assert_eq!(
            state.far_k,
            CsmFitState::UNLATCHED,
            "the degenerate CatchAll check must run BEFORE the latch is ever touched"
        );
    }

    #[test]
    fn shrink_relocates_the_terminator_catch_all_does_not() {
        // D2's documented trade-off: Shrink's last split IS far_eff (a hard, un-cross-faded
        // terminator relocated into the visible scene); CatchAll reserves cascade N-1 for
        // `[far_eff, far_cap]`, so its last split stays at far_cap exactly as today.
        let view = perspective_view(Vec3::new(0.0, 2.0, 0.0), 0.0, 0.0);
        let sun = [0.3_f32, -1.0, 0.2];
        let far_eff = 5.0_f32;

        let cfg_shrink = CsmConfig { fit_mode: CsmFitMode::Shrink, ..enabled_cfg() };
        let shrink = resolve_csm(
            &cfg_shrink, &view, sun,
            CsmFit { far_eff: Some(far_eff), reserve_tail: false, caster_aabb: None },
        );
        let n = shrink.active_count as usize;
        assert!(
            (shrink.cascades[n - 1].split_far - far_eff).abs() < 1.0e-3,
            "Shrink's last split must equal far_eff (the hard terminator), got {}",
            shrink.cascades[n - 1].split_far
        );
        assert!(
            shrink.cascades[n - 1].split_far < cfg_shrink.shadow_distance - 1.0,
            "Shrink's terminator must relocate well short of shadow_distance"
        );

        let cfg_catch = CsmConfig { fit_mode: CsmFitMode::CatchAll, ..enabled_cfg() };
        let catch = resolve_csm(
            &cfg_catch, &view, sun,
            CsmFit { far_eff: Some(far_eff), reserve_tail: true, caster_aabb: None },
        );
        let m = catch.active_count as usize;
        assert!(
            (catch.cascades[m - 2].split_far - far_eff).abs() < 1.0e-3,
            "CatchAll's second-to-last split must equal far_eff, got {}",
            catch.cascades[m - 2].split_far
        );
        let far_cap = view.far.min(cfg_catch.shadow_distance);
        assert!(
            (catch.cascades[m - 1].split_far - far_cap).abs() <= 1.0e-2 * far_cap,
            "CatchAll's last split must equal far_cap (no terminator), got {}",
            catch.cascades[m - 1].split_far
        );
    }

    // ---- C4: the sun-axis pull-back (T11, docs/CSM-AUTOFIT-PLAN.md D5, section 8/10) ------

    #[test]
    fn up_sun_caster_shadow_survives_the_shrink() {
        // docs/CSM-AUTOFIT-PLAN.md D5 (rung C4) -- the vanishing-shadow regression this
        // rung fixes: today's `z_far = 2*diameter` gives an up-sun caster capture of
        // exactly `diameter`. Shrinking `diameter` (the caster fit's entire point) would,
        // unfixed, shrink that capture too and silently clip a caster placed up-sun of the
        // fitted sphere (D5's "ceiling beam" example) -- a correctness regression, not a
        // trade-off. This pins section 8's three `up_need` rows in one hand-verifiable
        // scene: a 90-degree FOV / 1:1 aspect camera at the world origin looking down -Z,
        // so the frustum-slice corners (and the fitted sphere they derive) are simple,
        // round numbers -- eliminating trig-precision guesswork from the caster placement
        // below.
        let view = ViewUniform {
            camera_pos: Vec4::new(0.0, 0.0, 0.0, 1.0),
            cam_forward: Vec4::new(0.0, 0.0, -1.0, 0.0),
            cam_right: Vec4::new(1.0, 0.0, 0.0, 0.0),
            cam_up: Vec4::new(0.0, 1.0, 0.0, 0.0),
            fov_y: core::f32::consts::FRAC_PI_2,
            aspect: 1.0,
            near: 1.0,
            far: 1000.0,
            ..ViewUniform::IDENTITY
        };
        let sun_dir = [0.0_f32, 0.0, 1.0];
        let sun = Vec3::new(sun_dir[0], sun_dir[1], sun_dir[2]);
        // CatchAll's minimum cascade count (2): cascade 0 splits `[near, far_eff]` with
        // `pssm_n == count - 1 == 1`, so `pssm_split`'s ratio is `idx / n == 1 / 1 == 1.0`
        // and `split_0 == far_eff` EXACTLY (both the log and uniform terms reduce to `far`
        // at ratio 1) -- one less unknown in the hand-derivation below.
        let far_eff = 5.0_f32;
        let cfg = CsmConfig { fit_mode: CsmFitMode::CatchAll, cascade_count: 2, ..CsmConfig::default() };

        // Cascade 0's REAL fitted sphere, reproduced via the SAME functions `resolve_csm`
        // calls internally for the SAME (view, near, far_eff, cascade index) -- `caster_aabb`
        // does not affect the split/corners/sphere steps at all (only the Z pull-back this
        // test exercises), so this is exact, not a guess.
        let rig = FrustumRig {
            eye: view.camera_pos.xyz(),
            forward: view.cam_forward.xyz(),
            right: view.cam_right.xyz(),
            up: view.cam_up.xyz(),
            half_tan: (view.fov_y * 0.5).tan(),
            aspect: view.aspect,
        };
        let split0 = pssm_split(view.near, far_eff, cfg.lambda, 1, 1.0);
        let corners0 = slice_corners(&rig, view.near, split0);
        let center0 = sphere_center(&corners0);
        let radius0 = sphere_radius(&corners0, center0);
        let diameter0 = (2.0 * radius0).ceil().max(1.0e-3);

        // Recovers z_far from cascade 0's Z row: `pv[2] = fwd / zr` component-wise, and
        // `fwd` (= `-sun`, already unit-length here) makes the row's magnitude exactly
        // `1 / zr` -- independent of `resolve_csm`'s own locals.
        let z_far_of = |m: &[[f32; 4]; 4]| -> f32 {
            let row2 = Vec3::new(m[0][2], m[1][2], m[2][2]);
            1.0 / row2.length() + LIGHT_Z_NEAR
        };
        let resolve_with = |point: Vec3| -> ResolvedCsm {
            let p = [point.x, point.y, point.z];
            let fit = CsmFit { far_eff: Some(far_eff), reserve_tail: true, caster_aabb: Some((p, p)) };
            resolve_csm(&cfg, &view, sun_dir, fit)
        };

        // ---- Case A: a caster genuinely up-sun of the fitted sphere's OWN edge (2x the
        // diameter, offset by +1 to a guaranteed-odd integer so it can never exactly
        // coincide with a `2^(k/4)` grid boundary -- `diameter0` is itself always an exact
        // integer, via `.ceil()`) must still land inside cascade 0's depth range. Under
        // today's un-fitted `pull_back == diameter0`, this caster (up-sun distance >
        // diameter0) would have been clipped -- the regression this rung fixes.
        let up_sun_dist = 2.0 * diameter0 + 1.0;
        let up_sun_point = center0 + sun * up_sun_dist;
        let resolved_a = resolve_with(up_sun_point);
        let m_a = &resolved_a.cascades[0].view_proj;
        let c = clip(m_a, up_sun_point);
        assert!(
            (0.0..=1.0).contains(&c[2]),
            "the up-sun caster must map inside [LIGHT_Z_NEAR, z_far] of cascade 0's clip Z \
             (clip.z = {}, up_sun_dist = {up_sun_dist}, diameter0 = {diameter0})",
            c[2]
        );
        let z_far_a = z_far_of(m_a);
        assert!(
            z_far_a >= 2.0 * diameter0 - 1.0e-2,
            "pull_back must never fall below diameter (D5): z_far {z_far_a} must be >= \
             2*diameter0 {}",
            2.0 * diameter0
        );
        assert!(
            z_far_a <= 5.0 * diameter0 + 1.0e-2,
            "z_far {z_far_a} must stay <= 5*diameter0 {} (MAX_PULLBACK_RATIO bound)",
            5.0 * diameter0
        );

        // ---- Case B: every caster corner DOWN-sun of the centre (`up_need <= 0`) -- the
        // pull-back must fall back to EXACTLY today's `diameter0` (not merely "small").
        let down_sun_point = center0 - sun * 10.0;
        let resolved_b = resolve_with(down_sun_point);
        let z_far_b = z_far_of(&resolved_b.cascades[0].view_proj);
        assert!(
            (z_far_b - 2.0 * diameter0).abs() < 1.0e-2,
            "up_need <= 0 must fall back to pull_back == diameter (z_far == 2*diameter0): \
             got {z_far_b}, expected {}",
            2.0 * diameter0
        );

        // ---- Case C: a far up-sun caster -- the pull-back must clamp at
        // MAX_PULLBACK_RATIO * diameter0, not track the caster's raw (unbounded) distance.
        let far_up_sun_point = center0 + sun * (diameter0 * 1000.0);
        let resolved_c = resolve_with(far_up_sun_point);
        let z_far_c = z_far_of(&resolved_c.cascades[0].view_proj);
        assert!(
            (z_far_c - (MAX_PULLBACK_RATIO + 1.0) * diameter0).abs() < 1.0e-1,
            "a far up-sun caster must clamp z_far at (MAX_PULLBACK_RATIO + 1) * diameter0, \
             got {z_far_c}, expected {}",
            (MAX_PULLBACK_RATIO + 1.0) * diameter0
        );
    }

    // ---- the log-quantization grid + Schmitt latch (T10, T17, T18, T19) -------------------
    // Tests T10, T17, T18, T19 (docs/CSM-AUTOFIT-PLAN.md D6, section 10) plus targeted
    // edge/adversarial coverage of `grid_value`/`grid_cell`/`latch_cell` in ISOLATION (the
    // primitives standalone) -- `select_fit`'s own T1-T9/T12 tests, just above, cover them
    // wired through the fit gate.

    /// The next representable f32 strictly ABOVE a positive, finite `x` (not `f32::MAX`).
    /// A local bit-stepping helper (mirrors the module's own bit-decomposition style) so
    /// the threshold-boundary tests below do not depend on `f32::next_up`'s stabilization.
    fn ulp_above(x: f32) -> f32 {
        debug_assert!(x.is_finite() && x > 0.0, "ulp_above: x must be finite and > 0");
        f32::from_bits(x.to_bits() + 1)
    }

    /// The next representable f32 strictly BELOW a positive, finite `x` (not the smallest
    /// positive subnormal).
    fn ulp_below(x: f32) -> f32 {
        debug_assert!(x.is_finite() && x > 0.0, "ulp_below: x must be finite and > 0");
        f32::from_bits(x.to_bits() - 1)
    }

    /// `2^n` built directly from IEEE-754 bits -- an oracle INDEPENDENT of `exp2i` (a
    /// separate implementation of the same bit trick, not a call into the code under
    /// test), for pinning `grid_value`'s exact-octave-boundary claim without relying on
    /// `f32::powi` (not guaranteed bit-exact the way a direct exponent-field write is).
    fn exact_pow2(n: i32) -> f32 {
        debug_assert!((EXP2I_MIN_EXPONENT..=EXP2I_MAX_EXPONENT).contains(&n));
        f32::from_bits(((n + 127) as u32) << 23)
    }

    #[test]
    fn grid_cell_grid_value_round_trip() {
        // Exhaustive (not sampled) over the FULL safe exponent range: the round-trip is a
        // bit-identity claim (D6/T10), not a statistical one. `k` safe range is
        // `[EXP2I_MIN_EXPONENT*4, EXP2I_MAX_EXPONENT*4 + 3]` -- one step past either end
        // decodes to an out-of-range exponent (pinned separately below).
        let k_min = EXP2I_MIN_EXPONENT * FIT_GRID_CELLS_PER_OCTAVE;
        let k_max = EXP2I_MAX_EXPONENT * FIT_GRID_CELLS_PER_OCTAVE + (FIT_GRID_CELLS_PER_OCTAVE - 1);
        for k in k_min..=k_max {
            let v = grid_value(k);
            assert!(v.is_finite() && v > 0.0, "grid_value({k}) = {v} must be finite and positive");
            assert_eq!(
                grid_cell(v),
                k,
                "grid_cell(grid_value({k})) must round-trip bit-exactly; grid_value({k}) = {v} \
                 (bits {:#010x}), decoded back to {}",
                v.to_bits(),
                grid_cell(v)
            );
        }

        // Wide sweep of arbitrary positive x: random NORMAL f32 bit patterns (deterministic
        // xorshift64, matching this module's own random-sweep style at
        // `every_view_proj_element_finite_for_random_camera_and_sun`), exponent field kept
        // inside [1, 253] (unbiased [-126, 126]) so `grid_cell(x)+1` always stays inside the
        // safe k range above -- the bracket and never-clips properties are pinned over the
        // grid's full safe operating domain, not just a narrow band around 1.0.
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next_bits = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..20_000 {
            let bits = next_bits();
            let exp_field = 1 + (bits % 253) as u32; // 1..=253 -> unbiased -126..=126
            let mantissa = ((bits >> 20) & 0x007F_FFFF) as u32;
            let x = f32::from_bits((exp_field << 23) | mantissa);
            assert!(x.is_finite() && x > 0.0, "generator bug: x = {x}");

            let cell = grid_cell(x);
            let lo = grid_value(cell);
            let hi = grid_value(cell + 1);
            assert!(
                lo <= x,
                "grid_value(grid_cell(x)) must be <= x (x = {x}, cell = {cell}, grid_value(cell) = {lo})"
            );
            assert!(
                x < hi,
                "x must be < grid_value(grid_cell(x)+1) (x = {x}, cell = {cell}, grid_value(cell+1) = {hi})"
            );
            // The load-bearing never-clips property (D6): the grid must never report a
            // far_eff smaller than the raw distance that produced it -- `Shrink`'s
            // correctness depends on this holding for every raw the fit can plausibly see.
            assert!(
                hi >= x,
                "grid must never clip: grid_value(grid_cell(raw)+1) must be >= raw (raw = {x}, got {hi})"
            );
        }
    }

    #[test]
    fn grid_value_is_exact_at_every_octave_boundary() {
        // The module doc's own claim (csm_config.rs ~L82): "grid_value(4*n) == 2^n exactly"
        // (4 cells/octave -> the grid is exactly representable at every power of two).
        // `exact_pow2` is an INDEPENDENT bit-construction oracle, not `f32::powi`.
        for n in -100..=100 {
            let k = n * FIT_GRID_CELLS_PER_OCTAVE;
            let expected = exact_pow2(n);
            assert_eq!(
                grid_value(k),
                expected,
                "grid_value({k}) must be EXACTLY 2^{n} (bit-exact octave boundary), got {}",
                grid_value(k)
            );
        }
    }

    #[test]
    fn grid_cell_places_threshold_values_in_the_upper_cell() {
        // Exact table entries + one ULP on either side, at the phase boundaries
        // FIT_GRID_TABLE encodes -- the compare in `grid_cell` is `m >= TABLE[phase]`, so a
        // threshold value itself belongs to the UPPER cell, not the lower one.
        for (phase, &threshold) in FIT_GRID_TABLE.iter().enumerate().skip(1) {
            let phase = phase as i32;
            assert_eq!(
                grid_cell(threshold),
                phase,
                "the exact table threshold FIT_GRID_TABLE[{phase}] = {threshold} must decode to phase {phase}"
            );
            assert_eq!(
                grid_cell(ulp_above(threshold)),
                phase,
                "one ULP above the threshold must stay in phase {phase}"
            );
            assert_eq!(
                grid_cell(ulp_below(threshold)),
                phase - 1,
                "one ULP below the threshold must fall into the PRIOR phase {}",
                phase - 1
            );
        }
        // The octave wrap: 2.0 == grid_value(4) (phase 0 of the next octave); one ULP below
        // it must land in phase 3 of THIS octave, not wrap incorrectly into phase 0.
        assert_eq!(grid_cell(2.0), FIT_GRID_CELLS_PER_OCTAVE, "2.0 must decode to k=4 (phase 0, exp 1)");
        assert_eq!(
            grid_cell(ulp_below(2.0)),
            FIT_GRID_CELLS_PER_OCTAVE - 1,
            "one ULP below 2.0 must fall into the LAST phase of the exp-0 octave"
        );
    }

    #[test]
    fn grid_cell_clamps_subnormal_input_up_to_the_smallest_normal_cell() {
        // A subnormal x is FINITE and > 0 -- it satisfies grid_cell's documented
        // debug_assert contract -- but is below f32::MIN_POSITIVE, so it exercises the
        // #[cold] clamp path THROUGH the public API (not bypassing it).
        let smallest_subnormal = f32::from_bits(1); // ~1.4e-45
        let mid_subnormal = f32::MIN_POSITIVE / 2.0;
        let min_positive_cell = grid_cell(f32::MIN_POSITIVE);
        assert_eq!(
            grid_cell(smallest_subnormal),
            min_positive_cell,
            "the smallest positive subnormal must clamp UP to the same cell as f32::MIN_POSITIVE, never below it"
        );
        assert_eq!(
            grid_cell(mid_subnormal),
            min_positive_cell,
            "a mid-range subnormal must also clamp up to f32::MIN_POSITIVE's cell"
        );
        // f32::MIN_POSITIVE itself is NOT subnormal -- the boundary condition
        // (`x >= f32::MIN_POSITIVE`) must NOT clamp it away from its true decoded cell.
        assert_eq!(
            min_positive_cell,
            EXP2I_MIN_EXPONENT * FIT_GRID_CELLS_PER_OCTAVE,
            "f32::MIN_POSITIVE must decode to the bottom of the safe grid range, not be clamped"
        );
    }

    #[test]
    fn grid_cell_clamp_helper_returns_min_positive_for_any_input() {
        // Direct pin of the #[cold] clamp path `grid_cell` falls back to for a non-finite /
        // negative / zero x that would slip past a RELEASE build's disabled debug_assert.
        // Called directly (bypassing `grid_cell`'s own debug_assert gate) so this test does
        // not depend on debug_assertions being off -- it pins the helper's OWN
        // unconditional behavior, per Principle 3's #[cold] discipline.
        for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, -0.0, f32::MIN_POSITIVE / 2.0] {
            assert_eq!(
                grid_cell_clamp(x),
                f32::MIN_POSITIVE,
                "grid_cell_clamp must floor ANY input to the smallest normal f32 magnitude, got {} for x={x}",
                grid_cell_clamp(x)
            );
        }
    }

    #[test]
    fn exp2i_clamp_helper_clamps_to_the_safe_exponent_range() {
        assert_eq!(exp2i_clamp(1_000_000), EXP2I_MAX_EXPONENT, "an overlarge exponent clamps to the max");
        assert_eq!(exp2i_clamp(-1_000_000), EXP2I_MIN_EXPONENT, "an underlarge exponent clamps to the min");
        assert_eq!(exp2i_clamp(EXP2I_MAX_EXPONENT), EXP2I_MAX_EXPONENT, "an in-range exponent is unchanged");
        assert_eq!(exp2i_clamp(EXP2I_MIN_EXPONENT), EXP2I_MIN_EXPONENT, "an in-range exponent is unchanged");
    }

    #[test]
    #[should_panic(expected = "must be finite and > 0")]
    fn grid_cell_debug_asserts_on_non_positive_input() {
        let _ = grid_cell(0.0);
    }

    #[test]
    #[should_panic(expected = "outside the safe range")]
    fn grid_value_debug_asserts_on_out_of_range_k() {
        // One step past the safe range's top boundary (pinned exact at the boundary by
        // `grid_cell_grid_value_round_trip` above).
        let k_max = EXP2I_MAX_EXPONENT * FIT_GRID_CELLS_PER_OCTAVE + (FIT_GRID_CELLS_PER_OCTAVE - 1);
        let _ = grid_value(k_max + 1);
    }

    #[test]
    #[should_panic(expected = "must be finite and > 0")]
    fn latch_cell_debug_asserts_on_non_positive_raw() {
        let _ = latch_cell(-1.0, FIT_UNLATCHED);
    }

    #[test]
    fn latch_grows_immediately_shrinks_after_two_cells() {
        // A fresh latch lands ONE cell above raw (never-clips on frame 1: `grid_cell(raw)+1`).
        let k0 = latch_cell(grid_value(10), FIT_UNLATCHED);
        assert_eq!(k0, 11, "a fresh latch must land exactly one cell above raw");

        // ---- GROW: crosses immediately, at exactly one cell (18.92%) ----------------------
        let v = grid_value(k0); // the latched far_eff
        let grown = latch_cell(ulp_above(v), k0);
        assert_eq!(
            grown,
            k0 + 1,
            "raw crossing grid_value(prev_k) by even one ULP must grow the latch IMMEDIATELY \
             by exactly one cell, not wait"
        );
        assert_eq!(
            latch_cell(v, k0),
            k0,
            "raw exactly AT the latched value must not grow (strict '>', not '>=')"
        );
        assert_eq!(
            latch_cell(ulp_below(v), k0),
            k0,
            "raw just below the latched value must not grow"
        );

        // ---- SHRINK: sticky through one full cell, only trips after two -------------------
        // One cell below the latched value must stay sticky (shrink needs TWO full cells).
        assert_eq!(
            latch_cell(grid_value(k0 - 1), k0),
            k0,
            "raw ONE cell below the latched value must stay sticky"
        );
        // Exactly at the two-cell threshold is the boundary, NOT strictly past it -- the
        // predicate is `raw < grid_value(prev_k-2)`, so equality must still be sticky.
        let two_cells_down = grid_value(k0 - 2);
        assert_eq!(
            latch_cell(two_cells_down, k0),
            k0,
            "raw AT the two-cell threshold must still be sticky (strict '<', not '<=')"
        );
        // One ULP past the two-cell threshold must shrink, landing exactly 2 cells below the
        // previous latch -- the documented -29.3% pop magnitude (D6).
        let shrunk = latch_cell(ulp_below(two_cells_down), k0);
        assert_eq!(
            shrunk,
            k0 - 2,
            "shrink must reland exactly two cells below the previous latch (~-29.3%), the \
             documented pop magnitude, got a jump to {shrunk} instead of {}",
            k0 - 2
        );
    }

    #[test]
    fn latch_has_no_limit_cycle() {
        // (a) MONOTONE up-then-down sweep: at most one latch transition per grid-cell
        // BOUNDARY the raw signal itself crosses. Proved in the doc: after a grow,
        // `raw > grid_value(k) == grid_value((k+1)-2)`'s complement holds, so the shrink
        // predicate is false the instant a grow lands -- no immediate re-trigger.
        let r0 = grid_value(0); // 1.0
        let peak = grid_value(8); // 2 octaves up, exactly 8 grid cells above r0
        let cells_up = grid_cell(peak) - grid_cell(r0);
        assert_eq!(cells_up, 8, "sweep must span exactly 8 grid-cell boundaries going up");

        const STEPS: usize = 4000;
        let mut k = FIT_UNLATCHED;
        let mut transitions = 0i32;

        // Up leg: r0 -> peak, log-uniform steps (uniform ratio steps across a log-spaced grid,
        // fine enough -- 500 steps/cell -- that no cell boundary is ever skipped in one step).
        for i in 0..=STEPS {
            let t = i as f32 / STEPS as f32;
            let raw = r0 * (peak / r0).powf(t);
            let new_k = latch_cell(raw, k);
            if new_k != k {
                transitions += 1;
            }
            k = new_k;
        }
        // Down leg: peak -> r0.
        for i in 0..=STEPS {
            let t = i as f32 / STEPS as f32;
            let raw = peak * (r0 / peak).powf(t);
            let new_k = latch_cell(raw, k);
            if new_k != k {
                transitions += 1;
            }
            k = new_k;
        }

        let cells_traversed = 2 * cells_up; // each boundary crossed once up, once down
        assert!(
            transitions <= cells_traversed,
            "a monotone up-then-down sweep must trigger AT MOST one latch transition per grid \
             cell boundary the raw signal crosses ({transitions} transitions over \
             {cells_traversed} boundary crossings) -- more would mean the latch amplifies the \
             input's own crossing rate (a limit cycle)"
        );
        // Returning raw to its starting value must settle the latch back near its starting
        // cell, not leave it drifted -- drift would itself be a symptom of a ratchet bug.
        let k_start = latch_cell(r0, FIT_UNLATCHED);
        assert!(
            (k - k_start).abs() <= 1,
            "after returning raw to its starting value the latch must settle back near its \
             starting cell, not drift (k_start = {k_start}, k_end = {k})"
        );

        // (b) ADVERSARIAL: a +/-9% oscillation astride a grid line must produce ZERO latch
        // transitions -- this directly refutes the "measure-zero dither" hand-wave (critic
        // finding B). The centre is placed EXACTLY on a grid line boundary (the tightest,
        // most delicate placement floating-point-wise) and, by construction, a fresh latch
        // established there sits at the geometric MIDPOINT of its own 2-cell sticky band
        // (`line == grid_value(k_line - 1)`), which is the worst-case-symmetric position: the
        // theoretical safe margin is ~+18.9% on the grow side and ~-15.9% on the shrink side
        // from `line` itself. +/-9% sits comfortably (with several points of margin) inside
        // BOTH, while still being a real, non-trivial-amplitude oscillation, not an
        // infinitesimal wobble.
        let line = grid_value(5);
        let k_line = latch_cell(line, FIT_UNLATCHED);
        let hi = line * 1.09;
        let lo = line * 0.91;
        let mut k2 = k_line;
        let mut dither_transitions = 0i32;
        for cycle in 0..500 {
            let raw = if cycle % 2 == 0 { hi } else { lo };
            let new_k = latch_cell(raw, k2);
            if new_k != k2 {
                dither_transitions += 1;
            }
            k2 = new_k;
        }
        assert_eq!(
            dither_transitions, 0,
            "a +/-9% oscillation astride a grid line must never re-quantize (0 transitions); \
             got {dither_transitions} -- this would refute the latch's no-limit-cycle claim"
        );
    }

    /// A DELIBERATE adversarial construction beyond the spec's ask, per the mandate to try
    /// to construct a cycling input: D6's own text scopes the no-limit-cycle guarantee to
    /// oscillations of amplitude "< 18.92% (grow side) / < 41.4% (shrink side)" -- NOT to an
    /// arbitrarily large alternating input. This test demonstrates that scope is REAL: an
    /// input alternating between two values ~2.4x apart (raw amplitude ~138%, far outside the
    /// documented dither band) settles into a PERSISTENT 2-cycle that never converges. This is
    /// expected for any two-threshold hysteresis given an adversarially alternating input
    /// (proved analytically: a grow always lands at the SAME k for a fixed high sample, a
    /// shrink always lands at the SAME k for a fixed low sample, so the pair is a stable
    /// period-2 orbit) and does NOT violate D6's claim -- but it is reported here explicitly
    /// so the amplitude qualifier is not lost by a future reader of the doc comment. See the
    /// tester report for the full derivation.
    #[test]
    fn latch_sustains_a_two_cycle_only_far_outside_the_documented_dither_band() {
        let a = grid_value(10); // high sample
        let b = grid_value(5); // low sample, > 2 cells below (grid_cell(a)+1) - 2
        let mut k = FIT_UNLATCHED;
        let mut trace = Vec::with_capacity(40);
        for i in 0..40 {
            let raw = if i % 2 == 0 { a } else { b };
            k = latch_cell(raw, k);
            trace.push(k);
        }
        // The tail must show a genuine, PERSISTENT alternation between exactly two distinct
        // cells -- not a converged fixed point and not a monotone drift.
        for w in trace[trace.len() - 10..].windows(2) {
            assert_ne!(w[0], w[1], "trace must keep alternating every sample: {trace:?}");
        }
        let tail_set: std::collections::HashSet<i32> = trace[trace.len() - 10..].iter().copied().collect();
        assert_eq!(
            tail_set.len(),
            2,
            "the sustained tail alternation must be between exactly 2 distinct cells: {trace:?}"
        );
    }

    #[test]
    fn grid_is_monotone_step_over_a_decade() {
        const STEPS: usize = 200_000;
        let lo = 1.0_f32;
        let hi = 10.0_f32;
        let mut prev_cell = grid_cell(lo);
        let mut distinct = std::collections::BTreeSet::new();
        distinct.insert(prev_cell);
        for i in 1..=STEPS {
            let x = lo + (hi - lo) * (i as f32 / STEPS as f32);
            let cell = grid_cell(x);
            assert!(
                cell >= prev_cell,
                "grid_cell must be non-decreasing over the decade (x = {x}, prev cell {prev_cell}, cell {cell})"
            );
            distinct.insert(cell);
            prev_cell = cell;
        }
        let count = distinct.len();
        // A decade spans `4 * log2(10)` grid cells exactly, by the grid's own definition
        // (4 cells/octave, and a decade is log2(10) ~= 3.32 octaves) ~= 13.29.
        let expected = 10.0_f32.ln() / core::f32::consts::LN_2 * FIT_GRID_CELLS_PER_OCTAVE as f32;
        assert!(
            (count as f32 - expected).abs() <= 2.0,
            "a decade must contain ~4*log2(10) ~= {expected:.2} distinct grid cells, got {count}"
        );
    }
}
