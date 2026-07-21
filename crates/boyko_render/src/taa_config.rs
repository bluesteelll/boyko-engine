//! TAA rung C1 — the ECS-native TAA tunable config the author sets, plus the cold policy that
//! maps its live scalars onto the existing [`ResolvedTaa`] UBO carrier.
//!
//! Principle 0: ECS-native — [`TaaConfig`] is the author-set `#[derive(Resource)]` singleton
//! (the cold config, NOT a side `std::Vec`/`HashMap`) and [`ResolvedTaa`] is its derived UBO
//! companion, mirroring the shadow-denoise substrate exactly:
//! [`ShadowDenoiseConfig`](crate::shadow_denoise_config::ShadowDenoiseConfig) (the author
//! config) + [`ResolvedShadowDenoise`](crate::shadow_denoise_config::ResolvedShadowDenoise) (the
//! derived UBO) + [`resolve_shadow_denoise_policy`](crate::shadow_denoise_config::resolve_shadow_denoise_policy)
//! (the cold single-writer).
//!
//! # `ResolvedTaa` already existed with no writer
//!
//! [`ResolvedTaa`](crate::aa_config::ResolvedTaa) (`aa_config.rs`) shipped with its three live
//! fields carrying the shipped v1 tuning as HARDCODED literals in its `Default` impl — no
//! `World` policy ever wrote it. [`resolve_taa_policy`] is that missing single writer,
//! completing the substrate the same way [`resolve_shadow_denoise_policy`] completes
//! [`ResolvedShadowDenoise`].
//!
//! # Full knob surface now, wired incrementally (clean-architecture-first-time)
//!
//! [`TaaConfig`] declares every tunable the shipped `taa_resolve.comp.hlsl` algorithm has a
//! decision point for, each defaulting to the CURRENTLY SHIPPED behaviour (sourced from
//! `taa_resolve.comp.hlsl` itself, [`ResolvedTaa`]'s prior hardcoded defaults, or — for
//! [`TaaConfig::depth_tol`], which has no shipped TAA-side constant — the sibling temporal-
//! denoise substrate [`ShadowDenoiseConfig::disocclusion_depth_tol`](crate::shadow_denoise_config::ShadowDenoiseConfig::disocclusion_depth_tol)
//! this shader's own module doc says it is "Modeled on"). Declaring the full surface up front
//! avoids an interim struct shape a later rung would have to widen. Rung C1 wired
//! [`TaaConfig::jitter_scope`]; **rung T2 (this rung) additionally wires
//! [`clamp`](TaaConfig::clamp), [`clamp_space`](TaaConfig::clamp_space),
//! [`clip`](TaaConfig::clip), [`blend`](TaaConfig::blend),
//! [`luma_weight`](TaaConfig::luma_weight), [`history_filter`](TaaConfig::history_filter),
//! [`disocclusion`](TaaConfig::disocclusion), and [`depth_tol`](TaaConfig::depth_tol) — every
//! one of `taa_resolve.comp.hlsl`'s decision points EXCEPT `mv_source` (needs a new texture
//! binding, rung D2) and `sharpen` (needs an `aa_out` ping-pong, rung T3), which stay inert.
//! `disocclusion`/`depth_tol` are forwarded into the UBO but stay UNREAD by the shader this
//! rung too — see [`DisocclusionTest`]'s doc for why (a depth-based test needs a binding this
//! resolve does not have). Every T2 mode word branches through `taa_resolve.comp.hlsl` via
//! wave-uniform runtime `if`s (the SAME idiom `deferred_pbr.hlsl`'s `shadow_mode` and
//! `shadow_apply.hlsli`'s `csm_pcf_disc` use) — NOT a `-D` shader variant, so this rung
//! compiles to ONE `.spv`. Each field's doc states its wiring status.

use boyko_macros::Resource;

use boyko_ecs::ecs::core::system::{Res, ResMut};

use crate::aa_config::ResolvedTaa;

// ---- the knob enums (capability is structural: the shipped behaviour is each enum's #[default]) --

/// The sub-pixel jitter sequence [`TaaConfig::jitter`] selects. Only
/// [`Halton23`](Self::Halton23) is wired — the shipped 8-tap
/// [`HALTON_8`](crate::taa_jitter::HALTON_8) table. `R2`/`Off` are declared for the full knob
/// surface but read by no resolve this rung — a future rung would wire them into
/// [`crate::taa_jitter`]'s table selection.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum JitterSequence {
    /// The shipped 8-tap Halton(2,3) table ([`HALTON_8`](crate::taa_jitter::HALTON_8)). The
    /// DEFAULT — today's only implemented sequence.
    #[default]
    Halton23,
    /// A quasi-random R2 low-discrepancy sequence — declared, NOT wired this rung.
    R2,
    /// No jitter (a diagnostic mode: temporal accumulation over a static sample grid) —
    /// declared, NOT wired this rung.
    Off,
}

/// Which camera surfaces [`TaaConfig`]'s sub-pixel jitter perturbs. Capability is structural
/// (mirrors [`AaMode`](crate::aa_config::AaMode)'s `Off`-keyed gate): the CHOICE of scope, not a
/// bool, decides which host producer runs.
///
/// **WIRED this rung**: `boyko_app::runner`'s frame loop reads
/// [`TaaConfig::basis_shear_enabled`] to decide whether to shear the b5 camera basis via
/// [`composite_perspective_from_view_sheared`](crate::composite_perspective_from_view_sheared).
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum JitterScope {
    /// The historical v1 shape (the C1 cut — see [`crate::taa_jitter`]'s module doc): ONLY the
    /// raster mesh vertex push is jittered
    /// ([`gbuffer_push_from_view_jittered`](crate::gbuffer_push_from_view_jittered)); the b5
    /// marcher/resolve/SSAO/CSM/froxel-shared camera basis stays UNJITTERED.
    ///
    /// **No longer the default** — kept as the explicit opt-OUT. Choose it only to reproduce the
    /// pre-C1 render, or if the SSCS corner watch-item below ever proves to matter for a scene.
    /// Be aware of what it costs: with this scope, TAA is a structural NO-OP on every
    /// SDF-marched pixel (proven bit-exactly — with the mesh leg disabled, Halton phase 0 vs
    /// phase 4 renders 0 differing pixels out of 810000) and it never reaches the shadow lookup,
    /// because both reconstruct their sample position from the unjittered basis.
    RasterOnly,
    #[default]
    /// **The DEFAULT.** The SAME `(jx, jy)` jitter ALSO shears the b5 camera forward basis
    /// (rung C1), so every ray-gen consumer — the SDF marcher, the deferred resolve, SSAO, the
    /// shadow lookup — samples at the jittered sub-pixel position. Invariant I2 (raster and
    /// reconstruction share one sample position) then holds across BOTH legs, not just the raster
    /// mesh. `docs/TAA-PLAN.md` Decision 1 has the derivation
    /// [`composite_perspective_from_view_sheared`](crate::composite_perspective_from_view_sheared)
    /// implements.
    ///
    /// # Why this is the default, and what it costs
    ///
    /// This knob only does anything once a world has deliberately armed
    /// [`AaMode::Taa`](crate::aa_config::AaMode::Taa), and
    /// arming TAA only to have it be a no-op on SDF pixels and shadows is a footgun that already
    /// cost someone a logged investigation. It also REPAIRS a latent inconsistency: without the
    /// shear the raster is jittered while the reconstruction is not, so the reconstructed `P`
    /// sits off-surface by a phase-varying amount.
    ///
    /// The honest cost, measured rather than argued: the sheared basis is NOT orthonormal
    /// (`fwd'·right ≈ 6.4e-4` at 900p/60°, shrinking as `1/resolution`), so every raw
    /// `dot(rd, cam_forward)` consumer accrues first-order error. Bounded and checked:
    /// `|fwd'| − 1 < 4e-6` (below f32 noise); the CSM cascade-select boundary wobbles 0.03–0.2%
    /// of a split, i.e. under 1/100 of the 20% `CSM_OVERLAP_PROPORTION` cross-fade band that is
    /// already blending there; and the SSCS screen-space round-trip is off by ~0.5 px (the jitter
    /// itself — `project_to_screen` recovers `ndc + jx`, not `ndc`), which temporal accumulation
    /// averages. [`RasterOnly`](Self::RasterOnly) is the opt-out if a scene ever proves otherwise.
    RasterAndBasis,
}

/// The neighborhood bound shape [`TaaConfig`]'s history clip evaluates against — WIRED this
/// rung (T2) via `taa_resolve.comp.hlsl`'s `clamp_word` (a wave-uniform runtime branch, the
/// SAME idiom `deferred_pbr.hlsl`'s `shadow_mode` uses). `#[repr(u32)]` in DECLARATION order
/// (NOT alphabetical) so the shipped default lands on word `0` — see [`ClampShape::as_word`]
/// for why that is load-bearing.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ClampShape {
    /// `mean ± variance_gamma * sigma` (the shipped shape, Salvi-style —
    /// `taa_resolve.comp.hlsl`'s `aabb_min`/`aabb_max`). The DEFAULT — word `0`, so a zeroed
    /// UBO clips exactly as today.
    #[default]
    Variance,
    /// No neighborhood bound — raw history, unclipped (`hist_clipped = hist_raw`; the
    /// neighborhood loop still runs, but its output is unread).
    Off,
    /// The 3×3 neighborhood min/max box — the tightest possible clip, no σ scale.
    MinMax,
}

impl ClampShape {
    /// The stable mode word `taa_resolve.comp.hlsl`'s `clamp_word` branches on — the
    /// `#[repr(u32)]` discriminant. `Variance => 0` (the shipped default — LOAD-BEARING: a
    /// zeroed/never-resolved [`ResolvedTaa`] must clip exactly as today, never `Off`/`MinMax`),
    /// `Off => 1`, `MinMax => 2`.
    #[inline]
    pub const fn as_word(self) -> u32 {
        self as u32
    }
}

/// The color space [`TaaConfig`]'s neighborhood clamp is evaluated in — WIRED this rung (T2)
/// via `taa_resolve.comp.hlsl`'s `clamp_space_word`. `#[repr(u32)]` already lands the shipped
/// default at word `0` in declaration order.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ClampSpace {
    /// Direct RGB (the shipped shape): the resolve clips the raw LDR `lit` RGB directly, no
    /// color-space transform. The DEFAULT — word `0`.
    #[default]
    Rgb,
    /// YCoCg (Karis/Salvi's decorrelated luma-chroma space, the TAA-literature clip space the
    /// design plan specified — the shipped v1 deviated to `Rgb`, see the module doc): the 3×3
    /// neighborhood samples and the history sample are transformed via `rgb_to_ycocg` before
    /// the clip, and the clipped result is transformed back via `ycocg_to_rgb`.
    YCoCg,
}

impl ClampSpace {
    /// The stable mode word `taa_resolve.comp.hlsl`'s `clamp_space_word` branches on. `Rgb =>
    /// 0` (the shipped default), `YCoCg => 1`.
    #[inline]
    pub const fn as_word(self) -> u32 {
        self as u32
    }
}

/// How [`TaaConfig`]'s out-of-bound history sample is pulled back into the neighborhood —
/// WIRED this rung (T2) via `taa_resolve.comp.hlsl`'s `clip_word`. `#[repr(u32)]` in
/// DECLARATION order so the shipped default lands on word `0`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ClipMode {
    /// Pull back along the ray from the AABB center through the color (the shipped Karis/Lottes
    /// directional clip, `taa_resolve.comp.hlsl`'s `clip_toward_aabb_center` — preserves
    /// hue/saturation rather than shifting it). The DEFAULT — word `0`.
    #[default]
    TowardCenter,
    /// Per-channel clamp to `[aabb_min, aabb_max]` — cheaper (one HLSL intrinsic, no ray
    /// projection), can shift hue.
    Clamp,
}

impl ClipMode {
    /// The stable mode word `taa_resolve.comp.hlsl`'s `clip_word` branches on. `TowardCenter =>
    /// 0` (the shipped default), `Clamp => 1`.
    #[inline]
    pub const fn as_word(self) -> u32 {
        self as u32
    }
}

/// The temporal feedback STRATEGY [`TaaConfig::default_blend`]/[`TaaConfig::min_blend`]
/// parameterize — WIRED this rung (T2) via `taa_resolve.comp.hlsl`'s `blend_word`.
/// [`resolve_taa_policy`] forwards `default_blend`/`min_blend` regardless of this field's
/// value; the shader picks which one CONSUMES them.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BlendMode {
    /// `blend_factor = clamp(1 / confidence, min_blend, default_blend)` (the shipped shape). The
    /// DEFAULT — word `0`.
    #[default]
    ConfidenceAdaptive,
    /// `blend_factor = default_blend` unconditionally, ignoring the accumulated-frame
    /// confidence counter — a diagnostic mode (no adaptive settle after a reset; every frame
    /// blends at the same rate).
    Fixed,
}

impl BlendMode {
    /// The stable mode word `taa_resolve.comp.hlsl`'s `blend_word` branches on.
    /// `ConfidenceAdaptive => 0` (the shipped default), `Fixed => 1`.
    #[inline]
    pub const fn as_word(self) -> u32 {
        self as u32
    }
}

/// The history reconstruction filter [`TaaConfig`] selects — WIRED this rung (T2) via
/// `taa_resolve.comp.hlsl`'s `history_filter_word`. `#[repr(u32)]` in DECLARATION order so the
/// shipped default lands on word `0`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum HistoryFilter {
    /// The shipped 16-tap separable bicubic Catmull-Rom reconstruction
    /// (`taa_resolve.comp.hlsl`'s `sample_history_catmull_rom`). The DEFAULT — word `0`.
    #[default]
    CatmullRom,
    /// A single bilinear tap (`sample_history_bilinear`, 4 `Load`s + 3 `lerp`s) — cheaper,
    /// blurs faster under repeated accumulation.
    Bilinear,
}

impl HistoryFilter {
    /// The stable mode word `taa_resolve.comp.hlsl`'s `history_filter_word` branches on.
    /// `CatmullRom => 0` (the shipped default), `Bilinear => 1`.
    #[inline]
    pub const fn as_word(self) -> u32 {
        self as u32
    }
}

/// The motion-vector source [`TaaConfig`]'s resolve reprojects with. Only
/// [`CameraOnly`](Self::CameraOnly) is wired — the shipped `gViewT` + shared camera basis +
/// `MotionCam.prev_view_proj` reconstruction (the C1 v1 scope; see `taa_resolve.comp.hlsl`'s
/// module doc). Declared, NOT wired this rung.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MvSource {
    /// Reproject through the camera-only ray (the shipped shape; exact for a moving camera over
    /// static geometry, and for a fully static scene). The DEFAULT.
    #[default]
    CameraOnly,
    /// A per-object motion-vector buffer. **Declared, and DELIBERATELY NOT WIRED** — rung D2 was
    /// investigated, costed, and declined. Read this before attempting it again.
    ///
    /// # Why it is not merely "unfinished"
    ///
    /// Rung 3b already built both MV producers, but they are not equally portable:
    /// - the MESH producer (`gbuffer_mrt.*` with `-D MOTION_VECTORS=1`, a 4th MRT) is pure raster
    ///   — no rayQuery, no TLAS — so it *is* portable off `hwrt`;
    /// - the SDF-pixel producer is a `-D HWRT=1` variant of the hardware rayQuery VIS pass. There
    ///   is **no software equivalent**; on a `not(hwrt)` build it does not exist.
    ///
    /// `taa_resolve.comp.hlsl` binds no G-buffer mask, so it cannot tell a mesh pixel from an SDF
    /// one. Reading an MV texture at every pixel is therefore correct ONLY if both producers wrote
    /// every pixel that frame. With just the mesh half, SDF pixels would silently regress from the
    /// shipped camera-only differential to `Δuv ≡ 0` — **worse than [`CameraOnly`](Self::CameraOnly)**,
    /// and invisible in a still frame.
    ///
    /// Worse, `sdf_mv_active()` additionally requires `RayShadowConfig` — hardware ray-traced mesh
    /// shadows — to be ARMED. So wiring this would make arming an ANTI-ALIASING mode silently
    /// activate an unrelated ray-tracing pipeline (an extra MRT attachment plus a VIS-MV compute
    /// dispatch) for a user who asked for neither RT shadows nor shadow denoise.
    ///
    /// # And the defect it would fix was not observed
    ///
    /// Per-object MV exists to stop a MOVING mesh from ghosting under camera-only reprojection.
    /// Measured on the in-motion eval (`taa_jitter_eval.rs`, `BOYKO_TAA_MOTION=object`: a caster
    /// translating ~7 px/frame across a static camera — the worst case, since the camera-only MV
    /// is then identically zero): **no ghosting**. The variance clamp rejects the stale history and
    /// falls back to the current sample, so a moving edge loses temporal AA rather than smearing.
    /// Owner-confirmed by eye.
    ///
    /// So the honest trade is: buy temporal AA on moving edges, pay a cross-feature coupling that
    /// forces an RT pipeline on. That is the wrong side of the trade for a defect that is not there.
    /// If a scene ever DOES ghost, revisit — and note the prerequisite found while costing this:
    /// `upload_prev_instance_models` cannot be un-walled without also porting
    /// `MeshRenderScratch::prev_ring` and the `hwrt`-forked second `gather_mesh_draws`, which is a
    /// real refactor of an established per-leg-fork pattern, not a `cfg` removal.
    ///
    /// # The convention, recorded so it is not re-derived
    ///
    /// The 4th MRT stores `clip_to_uv(prev_clip) − clip_to_uv(cur_clip)` (RG16F) — i.e. exactly the
    /// `prev_uv − cur_uv` shape rung C2's differential already adds to `pixel_uv`. So the correct
    /// integration is `history_uv = pixel_uv + gMotionVec.Load(coord).rg`, with **no sign flip**.
    PerObject,
}

/// The history-reset (disocclusion) test [`TaaConfig`] evaluates. [`OffScreenOnly`](Self::OffScreenOnly)
/// is the shipped off-screen/behind-camera test (`taa_resolve.comp.hlsl`'s `off_screen` check)
/// — the ONLY test the resolve can run today, since it retains no previous-frame depth to
/// compare against (only the CURRENT frame's `gViewT`; `taa_hist`'s alpha channel carries
/// confidence, not depth).
///
/// **This word is forwarded into [`ResolvedTaa::disocclusion_word`] this rung (T2), but
/// `taa_resolve.comp.hlsl` does NOT read it.** [`OffScreenAndDepth`](Self::OffScreenAndDepth)
/// would need a previous-frame depth binding the resolve does not have — adding a new bound
/// resource is a separate rung's decision (out of scope here, not silently invented). Selecting
/// [`OffScreenAndDepth`](Self::OffScreenAndDepth) today therefore behaves identically to
/// [`OffScreenOnly`](Self::OffScreenOnly) — this is DOCUMENTED inertness, not a silent no-op.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DisocclusionTest {
    /// Reset iff the reprojected UV is off-screen or behind the camera (the shipped shape, the
    /// ONLY test the resolve runs). The DEFAULT — word `0`.
    #[default]
    OffScreenOnly,
    /// ALSO reset on a reprojected-vs-current depth mismatch beyond [`TaaConfig::depth_tol`] —
    /// mirrors `shadow_temporal.comp.hlsl`'s `depth_swap` test. UNREAD by
    /// `taa_resolve.comp.hlsl` this rung (see this enum's doc) — carries identically to
    /// `OffScreenOnly` until a future rung adds the depth binding.
    OffScreenAndDepth,
}

impl DisocclusionTest {
    /// The stable mode word [`ResolvedTaa::disocclusion_word`] carries — the `#[repr(u32)]`
    /// discriminant. `OffScreenOnly => 0` (the shipped default), `OffScreenAndDepth => 1`.
    /// UNREAD by the shader this rung (see the enum doc).
    #[inline]
    pub const fn as_word(self) -> u32 {
        self as u32
    }
}

/// A post-resolve sharpening pass [`TaaConfig`] may select. Only [`None`](Self::None) is wired
/// — the shipped resolve writes `aa_out` directly, no sharpen pass. Declared, NOT wired this
/// rung.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SharpenMode {
    /// No sharpen pass (the shipped shape). The DEFAULT.
    #[default]
    None,
    /// AMD RCAS-style contrast-adaptive sharpen. Declared, NOT wired this rung.
    Rcas,
}

// ---- TaaConfig (the author-set Resource — mirrors ShadowDenoiseConfig) ----------------------

/// The author-facing TAA tunable surface — a `World`-singleton Resource mirroring
/// [`ShadowDenoiseConfig`](crate::shadow_denoise_config::ShadowDenoiseConfig) /
/// [`SsaoConfig`](crate::ssao_config::SsaoConfig): the cold, owner-set config
/// [`resolve_taa_policy`] maps onto the derived [`ResolvedTaa`] UBO carrier every frame.
///
/// See the module doc for the "full surface now, wired incrementally" rationale — every field
/// below states its shipped source and whether it is read this rung.
///
/// `#[derive(Resource)]` via [`boyko_macros::Resource`] (the same derive path
/// [`ShadowDenoiseConfig`](crate::shadow_denoise_config::ShadowDenoiseConfig) uses).
#[derive(Resource, Clone, Copy, Debug)]
pub struct TaaConfig {
    /// The sub-pixel jitter sequence. Only [`JitterSequence::Halton23`] is wired (the shipped
    /// [`HALTON_8`](crate::taa_jitter::HALTON_8) table). Default `Halton23`.
    pub jitter: JitterSequence,
    /// The jitter cycle length. `8` mirrors [`HALTON_8`](crate::taa_jitter::HALTON_8)'s shipped
    /// length; NOT read by [`crate::taa_jitter::ndc_jitter`] this rung (the table length is a
    /// compile-time const there). Default `8`.
    pub jitter_samples: u32,
    /// Which camera surfaces the jitter perturbs — see [`JitterScope`]. **WIRED this rung**:
    /// [`basis_shear_enabled`](Self::basis_shear_enabled) gates the b5 camera-basis shear at the
    /// host call site (`boyko_app::runner`). Default [`JitterScope::RasterOnly`] (today's
    /// shipped raster-only jitter).
    pub jitter_scope: JitterScope,
    /// The neighborhood bound shape — see [`ClampShape`]. **WIRED this rung (T2)**. Default
    /// [`ClampShape::Variance`] (the shipped `mean ± γσ` AABB).
    pub clamp: ClampShape,
    /// The color space the clamp AABB is computed in — see [`ClampSpace`]. **WIRED this rung
    /// (T2)**. Default [`ClampSpace::Rgb`] (the shipped direct-RGB clip).
    pub clamp_space: ClampSpace,
    /// How an out-of-bound history sample is pulled back — see [`ClipMode`]. **WIRED this rung
    /// (T2)**. Default [`ClipMode::TowardCenter`] (the shipped Karis/Lottes directional clip).
    pub clip: ClipMode,
    /// The clip AABB half-width scale (`× σ`, Salvi-style). Forwarded into
    /// [`ResolvedTaa::variance_gamma`] by [`resolve_taa_policy`]. Shipped default `1.0`
    /// (`ResolvedTaa`'s prior hardcoded value).
    pub variance_gamma: f32,
    /// The feedback weight at confidence == 1 (just after a reset). Forwarded into
    /// [`ResolvedTaa::default_blend`] by [`resolve_taa_policy`]. Shipped default `0.1`.
    pub default_blend: f32,
    /// The steady-state feedback floor (confidence → ∞). Forwarded into
    /// [`ResolvedTaa::min_blend`] by [`resolve_taa_policy`]. Shipped default `0.015`.
    pub min_blend: f32,
    /// The blend STRATEGY [`default_blend`](Self::default_blend)/[`min_blend`](Self::min_blend)
    /// parameterize — see [`BlendMode`]. **WIRED this rung (T2)**. Default
    /// [`BlendMode::ConfidenceAdaptive`] (the shipped ramp).
    pub blend: BlendMode,
    /// Whether the blend is Karis inverse-tonemap luma-weighted (`w = 1 / (1 + luma)`,
    /// suppressing a single bright outlier tap from dominating the average). **WIRED this rung
    /// (T2)** — forwarded INVERTED into [`ResolvedTaa::disable_luma_weight`] so the
    /// zero-is-shipped-default invariant holds (the shipped default applies the weight). The
    /// shipped resolve always applies it. Default `true`.
    pub luma_weight: bool,
    /// The history reconstruction filter — see [`HistoryFilter`]. **WIRED this rung (T2)**.
    /// Default [`HistoryFilter::CatmullRom`] (the shipped 16-tap separable bicubic).
    pub history_filter: HistoryFilter,
    /// The motion-vector source — see [`MvSource`]. Default [`MvSource::CameraOnly`] (the
    /// shipped C1 v1 scope). Out of scope for T2 (needs a new texture binding — rung D2).
    pub mv_source: MvSource,
    /// The history-reset (disocclusion) test — see [`DisocclusionTest`]. Default
    /// [`DisocclusionTest::OffScreenOnly`] (the shipped off-screen/behind-camera test). The
    /// word is forwarded into [`ResolvedTaa::disocclusion_word`] this rung (T2), but the shader
    /// does NOT read it — see [`DisocclusionTest`]'s doc for why (a depth-based test needs a
    /// binding this resolve does not have).
    pub disocclusion: DisocclusionTest,
    /// The relative depth-mismatch tolerance a future [`DisocclusionTest::OffScreenAndDepth`]
    /// would consume. Forwarded into [`ResolvedTaa::depth_tol`] this rung (T2), but UNREAD by
    /// the shader while [`disocclusion`](Self::disocclusion) stays inert (see that field's
    /// doc). Default `0.02` — TAA's own resolve has no depth-tolerance constant to source from
    /// (it tests off-screen only); this is sourced instead from
    /// [`ShadowDenoiseConfig::disocclusion_depth_tol`](crate::shadow_denoise_config::ShadowDenoiseConfig::disocclusion_depth_tol)'s
    /// shipped default (`0.02`) — the sibling temporal-denoise substrate
    /// `taa_resolve.comp.hlsl`'s own module doc says this shader is "Modeled on"
    /// (`shadow_temporal.comp.hlsl`'s `depth_swap` test uses the identical relative-tolerance
    /// shape this field would parameterize).
    pub depth_tol: f32,
    /// A post-resolve sharpen pass — see [`SharpenMode`]. Default [`SharpenMode::None`] (the
    /// shipped resolve has no sharpen pass).
    pub sharpen: SharpenMode,
    /// The [`SharpenMode::Rcas`] strength in `[0, 1]`, forwarded to the RCAS pass as a PUSH
    /// constant (NOT the `ResolvedTaa` UBO — it is host-read at record time, so it does NOT affect
    /// the 48-byte UBO byte-mirror). `0` = mild (peak `-1/8`), `1` = strong (peak `-1/5`), per the
    /// FidelityFX CAS sharpness mapping in `rcas.comp.hlsl`. Inert unless [`sharpen`](Self::sharpen)
    /// is [`SharpenMode::Rcas`]. Default `0.25` (a mild starting point the owner retunes by eye).
    pub rcas_sharpness: f32,
}

impl Default for TaaConfig {
    /// Every RESOLVE field defaults to the CURRENTLY SHIPPED behaviour — see each field's doc for
    /// its source — so a world that never customizes [`TaaConfig`] reproduces
    /// [`ResolvedTaa::default`]'s prior hardcoded values through [`resolve_taa`].
    ///
    /// [`jitter_scope`](TaaConfig::jitter_scope) is the ONE deliberate exception: it defaults to
    /// [`JitterScope::RasterAndBasis`], NOT to the historical `RasterOnly`. See that variant's doc
    /// for the reasoning and the measured cost. Note this only takes effect once a world has
    /// deliberately armed [`AaMode`](crate::AaMode)`::Taa` — the AA default is still `Off`, so no
    /// existing scene moves.
    ///
    /// This impl is the ONLY authority on the field's default: [`JitterScope`]'s own `#[default]`
    /// attribute is not consulted here, so the two are kept in agreement by hand. (They were not,
    /// briefly, and the enum attribute alone silently changed nothing.)
    #[inline]
    fn default() -> Self {
        Self {
            jitter: JitterSequence::Halton23,
            jitter_samples: 8,
            jitter_scope: JitterScope::RasterAndBasis,
            clamp: ClampShape::Variance,
            clamp_space: ClampSpace::Rgb,
            clip: ClipMode::TowardCenter,
            variance_gamma: 1.0,
            default_blend: 0.1,
            min_blend: 0.015,
            blend: BlendMode::ConfidenceAdaptive,
            luma_weight: true,
            history_filter: HistoryFilter::CatmullRom,
            mv_source: MvSource::CameraOnly,
            disocclusion: DisocclusionTest::OffScreenOnly,
            depth_tol: 0.02,
            sharpen: SharpenMode::None,
            rcas_sharpness: 0.25,
        }
    }
}

impl TaaConfig {
    /// Whether the b5 camera-basis shear runs — the structural predicate
    /// `jitter_scope == RasterAndBasis` (NOT stored state), mirroring
    /// [`SsaoConfig::enabled`](crate::ssao_config::SsaoConfig::enabled)'s shape. The host call
    /// site (`boyko_app::runner`) reads this (ANDed with the frame's TAA-armed state) to decide
    /// whether to pass `Some(jitter)` or the structural-skip `None` into
    /// [`composite_perspective_from_view_sheared`](crate::composite_perspective_from_view_sheared).
    #[inline]
    pub const fn basis_shear_enabled(&self) -> bool {
        matches!(self.jitter_scope, JitterScope::RasterAndBasis)
    }
}

// ---- the resolve decision (pure — mirrors resolve_shadow_denoise) ---------------------------

/// Maps a [`TaaConfig`] onto the derived [`ResolvedTaa`] UBO carrier — the pure, unit-testable
/// resolve [`resolve_taa_policy`] wraps. Forwards the three C1 scalars
/// ([`variance_gamma`](TaaConfig::variance_gamma), [`default_blend`](TaaConfig::default_blend),
/// [`min_blend`](TaaConfig::min_blend)) plus every T2 knob
/// ([`clamp`](TaaConfig::clamp), [`clamp_space`](TaaConfig::clamp_space),
/// [`clip`](TaaConfig::clip), [`blend`](TaaConfig::blend),
/// [`luma_weight`](TaaConfig::luma_weight), [`history_filter`](TaaConfig::history_filter),
/// [`disocclusion`](TaaConfig::disocclusion), [`depth_tol`](TaaConfig::depth_tol)); `mv_source`
/// and `sharpen` stay unforwarded (their own future rungs, D2/T3 — see the module doc).
///
/// `luma_weight` is forwarded INVERTED (`disable_luma_weight = !luma_weight`): the shipped
/// default is `true` (apply the weight), so the UBO word must read `0` at the default to keep
/// the zero-is-shipped-default invariant every other T2 word holds (see
/// [`ResolvedTaa::disable_luma_weight`]).
#[inline]
pub fn resolve_taa(cfg: &TaaConfig) -> ResolvedTaa {
    ResolvedTaa {
        default_blend: cfg.default_blend,
        min_blend: cfg.min_blend,
        variance_gamma: cfg.variance_gamma,
        _pad: 0.0,
        clamp_word: cfg.clamp.as_word(),
        clamp_space_word: cfg.clamp_space.as_word(),
        clip_word: cfg.clip.as_word(),
        blend_word: cfg.blend.as_word(),
        disable_luma_weight: u32::from(!cfg.luma_weight),
        history_filter_word: cfg.history_filter.as_word(),
        disocclusion_word: cfg.disocclusion.as_word(),
        depth_tol: cfg.depth_tol,
    }
}

// ---- the cold single-writer system (mirrors resolve_shadow_denoise_policy) ------------------

/// Single writer of [`ResolvedTaa`] (cold, once/frame) — the missing policy the module doc
/// describes. Mirrors
/// [`resolve_shadow_denoise_policy`](crate::shadow_denoise_config::resolve_shadow_denoise_policy)
/// / [`resolve_ssao_policy`](crate::ssao_config::resolve_ssao_policy). Reads the author
/// [`TaaConfig`] and writes the derived UBO carrier (the one-producer-per-field write
/// discipline).
//
// `clippy::needless_pass_by_value`: `Res`/`ResMut` are by-value `SystemParam`s read/written
// through reborrows — the same false-positive `resolve_shadow_denoise_policy` carries.
#[allow(clippy::needless_pass_by_value)]
pub fn resolve_taa_policy(cfg: Res<TaaConfig>, mut out: ResMut<ResolvedTaa>) {
    *out = resolve_taa(&cfg);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `TaaConfig::default()` carries every shipped-behaviour default this module's docs claim.
    #[test]
    fn default_config_matches_shipped_constants() {
        let cfg = TaaConfig::default();
        assert_eq!(cfg.jitter, JitterSequence::Halton23);
        assert_eq!(cfg.jitter_samples, 8);
        // The ONE knob that deliberately does NOT default to the historical shape: the b5 basis
        // shear is opt-OUT, not opt-in. Pinned here precisely because the flip is easy to make
        // silently ineffective -- `TaaConfig`'s Default is a MANUAL impl, so `JitterScope`'s own
        // `#[default]` attribute is NOT what this reads, and changing only the attribute (as I
        // first did) compiles, reads correctly, and changes nothing.
        assert_eq!(cfg.jitter_scope, JitterScope::RasterAndBasis);
        assert_eq!(JitterScope::default(), cfg.jitter_scope, "the enum attribute and the manual impl must agree");
        assert_eq!(cfg.clamp, ClampShape::Variance);
        assert_eq!(cfg.clamp_space, ClampSpace::Rgb);
        assert_eq!(cfg.clip, ClipMode::TowardCenter);
        assert_eq!(cfg.variance_gamma, 1.0);
        assert_eq!(cfg.default_blend, 0.1);
        assert_eq!(cfg.min_blend, 0.015);
        assert_eq!(cfg.blend, BlendMode::ConfidenceAdaptive);
        assert!(cfg.luma_weight);
        assert_eq!(cfg.history_filter, HistoryFilter::CatmullRom);
        assert_eq!(cfg.mv_source, MvSource::CameraOnly);
        assert_eq!(cfg.disocclusion, DisocclusionTest::OffScreenOnly);
        assert_eq!(cfg.depth_tol, 0.02);
        assert_eq!(cfg.sharpen, SharpenMode::None);
        assert_eq!(cfg.rcas_sharpness, 0.25);
        assert!(
            cfg.basis_shear_enabled(),
            "the shear is opt-OUT: a world that arms Taa and sets nothing else gets it"
        );
        // The 0%-gate is NOT this knob -- it is `AaMode::Off`, which is still the AA default and
        // is what keeps every existing scene byte-identical. `jitter_scope` cannot render anything
        // on its own; `basis_shear_enabled()` is only ever consulted on an already-armed frame.
        assert_eq!(crate::AaConfig::default().mode, crate::AaMode::Off);
    }

    /// Capability is structural: `basis_shear_enabled` keys ONLY off `jitter_scope`.
    #[test]
    fn basis_shear_enabled_is_structural_over_jitter_scope() {
        let raster_only = TaaConfig { jitter_scope: JitterScope::RasterOnly, ..TaaConfig::default() };
        assert!(!raster_only.basis_shear_enabled());

        let raster_and_basis =
            TaaConfig { jitter_scope: JitterScope::RasterAndBasis, ..TaaConfig::default() };
        assert!(raster_and_basis.basis_shear_enabled());
    }

    /// Every T2 mode word's `#[repr(u32)]` discriminant — pinned so the shader's mirrored
    /// constants (`taa_resolve.comp.hlsl`'s `TAA_CLAMP_*`/`TAA_CLIP_*`/`TAA_BLEND_*`/
    /// `TAA_HISTORY_FILTER_*`) never drift from the host side.
    #[test]
    fn mode_words_are_the_repr_discriminants() {
        assert_eq!(ClampShape::Variance.as_word(), 0);
        assert_eq!(ClampShape::Off.as_word(), 1);
        assert_eq!(ClampShape::MinMax.as_word(), 2);
        assert_eq!(ClampSpace::Rgb.as_word(), 0);
        assert_eq!(ClampSpace::YCoCg.as_word(), 1);
        assert_eq!(ClipMode::TowardCenter.as_word(), 0);
        assert_eq!(ClipMode::Clamp.as_word(), 1);
        assert_eq!(BlendMode::ConfidenceAdaptive.as_word(), 0);
        assert_eq!(BlendMode::Fixed.as_word(), 1);
        assert_eq!(HistoryFilter::CatmullRom.as_word(), 0);
        assert_eq!(HistoryFilter::Bilinear.as_word(), 1);
        assert_eq!(DisocclusionTest::OffScreenOnly.as_word(), 0);
        assert_eq!(DisocclusionTest::OffScreenAndDepth.as_word(), 1);
    }

    /// `resolve_taa(&TaaConfig::default())` equals the shipped constants: every T2 mode word is
    /// `0` — a zeroed/never-resolved [`ResolvedTaa`] UBO must degrade to today's shipped
    /// behaviour, never a different clip/filter/blend arm (mirrors `CsmPcfKernel::Tent13 == 0`'s
    /// load-bearing discipline in `csm_config.rs`) — AND every scalar equals the shipped C1
    /// tuning (`_pad`/`depth_tol` included, so nothing silently drifted).
    #[test]
    fn every_default_knob_resolves_to_mode_word_zero() {
        let resolved = resolve_taa(&TaaConfig::default());
        assert_eq!(resolved.default_blend, 0.1);
        assert_eq!(resolved.min_blend, 0.015);
        assert_eq!(resolved.variance_gamma, 1.0);
        assert_eq!(resolved._pad, 0.0);
        assert_eq!(resolved.clamp_word, 0);
        assert_eq!(resolved.clamp_space_word, 0);
        assert_eq!(resolved.clip_word, 0);
        assert_eq!(resolved.blend_word, 0);
        assert_eq!(
            resolved.disable_luma_weight, 0,
            "the shipped default APPLIES the luma weight (inverted encoding)"
        );
        assert_eq!(resolved.history_filter_word, 0);
        assert_eq!(resolved.disocclusion_word, 0);
        assert_eq!(resolved.depth_tol, 0.02);
    }

    /// `resolve_taa` forwards every field — the T2 successor of the C1-era three-field-only
    /// test. `depth_tol`/`disocclusion_word` are forwarded even though the shader does not read
    /// them yet (see [`DisocclusionTest`]'s doc).
    #[test]
    fn resolve_taa_forwards_every_field() {
        let cfg = TaaConfig {
            default_blend: 0.2,
            min_blend: 0.03,
            variance_gamma: 1.5,
            clamp: ClampShape::MinMax,
            clamp_space: ClampSpace::YCoCg,
            clip: ClipMode::Clamp,
            blend: BlendMode::Fixed,
            luma_weight: false,
            history_filter: HistoryFilter::Bilinear,
            disocclusion: DisocclusionTest::OffScreenAndDepth,
            depth_tol: 0.05,
            ..TaaConfig::default()
        };
        assert_eq!(
            resolve_taa(&cfg),
            ResolvedTaa {
                default_blend: 0.2,
                min_blend: 0.03,
                variance_gamma: 1.5,
                _pad: 0.0,
                clamp_word: ClampShape::MinMax.as_word(),
                clamp_space_word: ClampSpace::YCoCg.as_word(),
                clip_word: ClipMode::Clamp.as_word(),
                blend_word: BlendMode::Fixed.as_word(),
                disable_luma_weight: 1,
                history_filter_word: HistoryFilter::Bilinear.as_word(),
                disocclusion_word: DisocclusionTest::OffScreenAndDepth.as_word(),
                depth_tol: 0.05,
            }
        );
    }

    /// The `ResolvedTaa::default` shortcut must equal resolving a default `TaaConfig`, so a
    /// never-run policy already carries the correct shipped scalars and mode words.
    #[test]
    fn default_resolved_matches_resolving_the_default_config() {
        assert_eq!(ResolvedTaa::default(), resolve_taa(&TaaConfig::default()));
    }

    /// Layout pin (rung T2): the UBO grew 16 -> 48 B (three std140 vec4 slots) — also
    /// const-asserted at `ResolvedTaa`'s definition (`aa_config.rs`); this is the runtime unit
    /// test companion.
    #[test]
    fn resolved_taa_is_48_bytes() {
        assert_eq!(core::mem::size_of::<ResolvedTaa>(), 48);
    }
}
