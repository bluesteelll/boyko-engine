//! Shadow Phase 5 Increment 1 — the cold ECS shadow-atlas slot-assignment policy for
//! sparse perspective (SPOT) shadow maps. This is the contained data/policy layer; the GPU
//! depth pass + resolve are Increment 1-GPU.
//!
//! Principle 0: ECS-native — [`ShadowConfig`] is the owner-set `#[derive(Resource)]`
//! singleton (the cold config, NOT a side `std::Vec`/`HashMap`) and [`ResolvedShadowAtlas`]
//! is its derived companion Resource written by the cold [`resolve_shadow_atlas`] system.
//! This mirrors the CSM substrate exactly: [`CsmConfig`](crate::csm_config::CsmConfig) (the
//! owner-set config) + [`ResolvedCsm`](crate::csm_config::ResolvedCsm) (the derived carrier)
//! + [`resolve_csm`](crate::csm_config::resolve_csm) (the pure fit) +
//! [`resolve_csm_cascades`](crate::csm_config::resolve_csm_cascades) (the cold single-owner
//! policy). The atlas-face array is an inline `[FaceTransform; M_SLOTS]`, NOT a `Vec`; the
//! top-K selection runs in a FIXED `[(f32, usize); M_SLOTS]` stack scratch (Principle 1/5 —
//! no heap sort, no allocation).
//!
//! # The 0%-gate
//!
//! [`ShadowConfig::default`] is DISABLED (`enabled == false`). [`resolve_shadow_atlas_spots`]
//! of a disabled config (or a frame with no eligible spots) is [`ResolvedShadowAtlas::DISABLED`]
//! (`mode_word == 0`), and [`ResolvedShadowAtlas::default`] is byte-identical to it — so a
//! world that never inserts a non-default [`ShadowConfig`] carries the disabled selection and
//! no render path is touched.
//!
//! # SPOT + POINT (Increment 1 + 2)
//!
//! A spot is ONE atlas layer (a single perspective shadow map of the cone, NDC-z — exactly
//! like a CSM cascade). A POINT (Increment 2) is SIX CONTIGUOUS atlas layers (the ±X/±Y/±Z
//! cube faces), each a 90°-FOV perspective looking down one axis; the resolve does a
//! major-axis face-select + a LINEAR-DISTANCE compare (`dist(frag, light_pos) * inv_range`
//! vs the stored normalized radial distance), so the [`FaceTransform`] carries the per-face
//! `light_pos` / `inv_range` lanes the cube distance-compare reads (unused by the SPOT NDC-z
//! compare). Points and spots are RANKED TOGETHER by the same priority proxy and
//! bump-allocated (a selected point takes 6 layers, a selected spot 1) until the
//! [`M_SLOTS`]-layer pool is full; over-budget sources get [`SLOT_NONE`].

use boyko_macros::Resource;

use boyko_ecs::ecs::core::iters::query::{Query, With};
use boyko_ecs::ecs::core::system::{Res, ResMut};

use boyko_math::{Affine3A, Mat4, Vec3, Vec4};
use boyko_scene::{GlobalTransform, ViewUniform};

use crate::light::{PointLight, SpotLight};
use crate::shadow_marker::CastsPunctualShadow;

// ---- constants -----------------------------------------------------------------------

/// The shadow-atlas layer budget — the inline cap on the [`ResolvedShadowAtlas`] face array
/// and the number of array layers the Inc-1-GPU depth pass renders into. A spot consumes ONE
/// layer (Inc 1); a point would consume six (Inc 2), so `M_SLOTS` bounds spots-this-increment
/// at 16 simultaneously-mapped sources.
pub const M_SLOTS: usize = 16;

/// The per-layer shadow-map resolution in texels (a square depth tile). The Inc-1-GPU depth
/// target is `SHADOW_DIM × SHADOW_DIM × M_SLOTS`.
pub const SHADOW_DIM: u32 = 512;

/// The 5-bit "no map" sentinel packed into a light's atlas-slot field: the light fell outside
/// the top-K budget (or shadows are disabled), so the resolve uses the analytic fallback
/// rather than sampling a layer. `0x1F == 31` is the all-ones 5-bit value, distinct from every
/// real slot index `[0, M_SLOTS)` (`M_SLOTS == 16 ≤ 31`).
pub const SLOT_NONE: u32 = 0x1F;

/// The bit offset of the 5-bit atlas-slot field in a light's kind word
/// ([`GpuLight::dir_kind`](crate::light::GpuLight)`.w`). Bits `0..16` carry the kind tag, bit
/// `16` is [`CASTS_SHADOW_BIT`], and bits `17..22` carry the slot — so the slot never collides
/// with either (proven by `pack_atlas_slot_never_collides`).
pub const ATLAS_SLOT_SHIFT: u32 = 17;

/// The 5-bit mask for the atlas-slot field (covers `[0, 31]`, enough for `SLOT_NONE == 0x1F`
/// and every slot in `[0, M_SLOTS)`).
pub const ATLAS_SLOT_MASK: u32 = 0x1F;

/// The "this light casts an exact (mapped) shadow" bit in a light's kind word — bit `16`,
/// directly below the slot field and above the 16-bit kind tag. Set when a light was assigned
/// an atlas slot (`slot != SLOT_NONE`); the resolve tests it to branch onto the map sample vs
/// the analytic fallback.
pub const CASTS_SHADOW_BIT: u32 = 1 << 16;

/// Priority denominator floor — guards the `range² / dist²` screen-coverage proxy against a
/// divide-by-zero when a spot sits exactly at the camera.
const PRIORITY_DIST_EPS: f32 = 1.0e-4;

/// The minimum length-squared a spot direction must have to be used as the cone axis; below it
/// the spot is degenerate and the fit substitutes a valid default axis so the perspective stays
/// finite.
const MIN_DIR_LEN_SQ: f32 = 1.0e-12;

/// The shadow near plane (view-space) for a spot's perspective map — a small positive front
/// clip so the projection stays well-conditioned.
const SPOT_SHADOW_NEAR: f32 = 0.05;

/// The minimum spot far plane (the cone depth range). Floors a zero/negative `range` so the
/// perspective `near < far` invariant holds and the matrix is non-singular.
const MIN_SPOT_FAR: f32 = SPOT_SHADOW_NEAR + 1.0e-3;

/// The spot map's aspect ratio — `1.0` (a square depth tile, [`SHADOW_DIM`] each side).
const SPOT_ASPECT: f32 = 1.0;

/// The number of cube faces a POINT light consumes — six contiguous atlas layers (±X, ±Y, ±Z).
/// A selected point bump-allocates this many layers; a slot base `b` is valid only when
/// `b + POINT_FACE_COUNT <= M_SLOTS`.
pub const POINT_FACE_COUNT: usize = 6;

/// The point cube face near plane (view-space) — the same small positive front clip the spot map
/// uses, so the per-face perspective stays well-conditioned. The radial-distance FS compare does
/// not read NDC-z, but the projection must still be non-singular for the depth-pass rasterizer.
const POINT_SHADOW_NEAR: f32 = SPOT_SHADOW_NEAR;

/// The minimum point cube far plane (the light range) — floors a zero/negative range so the
/// per-face `near < far` invariant holds.
const MIN_POINT_FAR: f32 = MIN_SPOT_FAR;

/// The point cube face FOV — 90° (`π/2`) full vertical FOV, so the six square faces exactly tile
/// the full sphere of directions around the light (the standard cube-map fit).
const POINT_FACE_FOV_Y: f32 = core::f32::consts::FRAC_PI_2;

// A point's slot base `b` is packed into the 5-bit atlas-slot field, so the maximum base (the
// last layer a point can start at, `M_SLOTS - POINT_FACE_COUNT`) MUST fit in `[0, ATLAS_SLOT_MASK)`
// and stay distinct from `SLOT_NONE`. `16 - 6 == 10 < 31` — proven at compile time.
const _: () = assert!((M_SLOTS - POINT_FACE_COUNT) < (ATLAS_SLOT_MASK as usize));
const _: () = assert!((M_SLOTS - POINT_FACE_COUNT) != (SLOT_NONE as usize));

/// Rec. 709 luminance weights (linear RGB → relative luminance) for the priority proxy.
const LUMA_R: f32 = 0.2126;
const LUMA_G: f32 = 0.7152;
const LUMA_B: f32 = 0.0722;

// ---- ShadowConfig (the owner-set Resource — mirrors CsmConfig) ------------------------

/// The global sparse-shadow config (Shadow Inc-1) — a `World`-singleton Resource the owner
/// sets, the spot/point analogue of [`CsmConfig`](crate::csm_config::CsmConfig). Enablement is
/// the `enabled` flag (an explicit on/off the owner flips), the spot/point analogue of CSM's
/// structural `cascade_count > 0`.
///
/// `#[derive(Resource)]` via [`boyko_macros::Resource`] (the same derive path `CsmConfig` /
/// `LightingConfig` use). `Copy` so the cold policy reads it by value.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ShadowConfig {
    /// Master on/off for sparse cube/spot shadow maps (`false` ⇒ disabled — the 0%-gate).
    pub enabled: bool,
    /// Per-layer shadow-map resolution in texels (drives the Inc-1-GPU depth target; the
    /// policy carries it so the consumer reads one config). Defaults to [`SHADOW_DIM`].
    pub dim: u32,
    /// Constant depth-bias term (rasterizer `depthBiasConstantFactor`) the Inc-1-GPU depth
    /// pass applies to the spot maps (acne control).
    pub depth_bias_constant: f32,
    /// Slope-scaled depth-bias term (rasterizer `depthBiasSlopeFactor`).
    pub depth_bias_slope: f32,
}

impl Default for ShadowConfig {
    /// The DISABLED default (`enabled == false` — the 0%-gate): a default world resolves the
    /// all-zero [`ResolvedShadowAtlas`] and touches no render path. The remaining fields carry
    /// the research defaults so flipping `enabled` to `true` yields a usable atlas without
    /// further tuning.
    #[inline]
    fn default() -> Self {
        Self {
            enabled: false,
            dim: SHADOW_DIM,
            depth_bias_constant: 0.0015,
            depth_bias_slope: 1.5,
        }
    }
}

impl ShadowConfig {
    /// Whether sparse shadow maps run — the `enabled` flag (the spot/point analogue of CSM's
    /// structural `cascade_count > 0`). False ⇒ the 0%-gate (no depth pass, the resolve's exact
    /// shadow term off, every light on the analytic fallback). Mirrors
    /// [`CsmConfig::enabled`](crate::csm_config::CsmConfig::enabled).
    #[inline]
    pub fn enabled(&self) -> bool {
        self.enabled
    }
}

// ---- FaceTransform (the per-layer GPU-ready record — shared SPOT/POINT) ----------------

/// One atlas layer's shadow transform + metadata — the per-face record the Inc-1-GPU depth
/// pass renders into and the resolve samples. `#[repr(C)]`, 80 B, GPU-ready, the SAME stride
/// as [`CascadeData`](crate::csm_config::CascadeData) so the shared shadow upload path treats
/// a cascade and an atlas face identically.
///
/// `view_proj` is the COLUMN-MAJOR world→light-clip matrix (the WGSL `mat4x4` convention,
/// matching [`ViewUniform::view_proj`]), so it uploads directly. For a SPOT (Inc 1) the resolve
/// compares against the NDC-z this projection produces (a single perspective map); `light_pos`
/// / `inv_range` are unused by that compare but are kept for the Inc-2 POINT cube face, whose
/// distance-compare reads them (`dist(frag, light_pos) * inv_range` vs the stored normalized
/// depth).
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct FaceTransform {
    /// Column-major world→light-clip transform (`perspective · light_view`), GPU-ready.
    pub view_proj: [[f32; 4]; 4],
    /// World-space light position (Inc-2 POINT cube distance-compare; unused by SPOT NDC-z).
    pub light_pos: [f32; 3],
    /// Reciprocal of the light range (Inc-2 POINT normalized-distance compare; unused by SPOT).
    pub inv_range: f32,
}

// Layout pin: 16 (mat4) × 4 + 12 (light_pos) + 4 (inv_range) = 64 + 16 = 80 B — the same
// stride as `CascadeData` (the shared shadow upload path). A change is a deliberate decision,
// not an accident (the GPU side reads this stride in Inc-1-GPU).
const _: () = assert!(size_of::<FaceTransform>() == 80);
const _: () = assert!(core::mem::offset_of!(FaceTransform, view_proj) == 0);
const _: () = assert!(core::mem::offset_of!(FaceTransform, light_pos) == 64);
const _: () = assert!(core::mem::offset_of!(FaceTransform, inv_range) == 76);

impl FaceTransform {
    /// The all-zero face (an unused atlas layer in a partially-filled [`ResolvedShadowAtlas`],
    /// or the whole array when shadows are disabled). A zero `view_proj` is intentionally NOT a
    /// valid transform — `active_layers` bounds the layers the consumer reads.
    pub const ZERO: Self = Self {
        view_proj: [[0.0; 4]; 4],
        light_pos: [0.0; 3],
        inv_range: 0.0,
    };
}

// ---- ResolvedShadowAtlas (the derived carrier — mirrors ResolvedCsm) ------------------

/// The derived shadow-atlas selection the Inc-1-GPU depth pass + resolve read — the spot/point
/// analogue of [`ResolvedCsm`](crate::csm_config::ResolvedCsm). [`resolve_shadow_atlas`] is its
/// SINGLE writer (the one-producer-per-field discipline), recomputing it from [`ShadowConfig`]
/// plus the active [`ViewUniform`] plus the eligible spots each frame. `#[repr(C)]` for a
/// stable GPU-ready layout (the inline face array, NOT a `Vec`).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct ResolvedShadowAtlas {
    /// The fitted atlas faces; only `[0..active_layers)` are valid (the rest are
    /// [`FaceTransform::ZERO`]).
    pub faces: [FaceTransform; M_SLOTS],
    /// The number of valid atlas layers (`0` when disabled — the 0%-gate).
    pub active_layers: u32,
    /// The shadow-atlas enable mode word: `0` ⇒ off (no depth pass, resolve exact-shadow term
    /// off), `1` ⇒ on. Derived from the SAME predicate as `active_layers > 0`, so the two never
    /// disagree.
    pub mode_word: u32,
    /// Padding to a 16-byte stride after the two trailing `u32` words.
    pub _pad: [u32; 2],
}

// Layout pin: 80 × 16 + 4 + 4 + 8 = 1280 + 16 = 1296 B — the `ResolvedCsm` fingerprint
// shape (an inline GPU-ready array + two count words + 8 B pad).
const _: () = assert!(size_of::<ResolvedShadowAtlas>() == 1296);
const _: () = assert!(core::mem::offset_of!(ResolvedShadowAtlas, faces) == 0);
const _: () = assert!(core::mem::offset_of!(ResolvedShadowAtlas, active_layers) == 1280);
const _: () = assert!(core::mem::offset_of!(ResolvedShadowAtlas, mode_word) == 1284);

/// The byte size of the host-coherent shadow-atlas UBO —
/// `size_of::<ResolvedShadowAtlas>()` (1296 B: `[FaceTransform; M_SLOTS]` +
/// `active_layers` + `mode_word` + pad). The resolve binds a UBO of exactly this
/// shape at binding 15; hosts size their atlas UBO from THIS constant (single
/// source — no hand-copied `1296`).
pub const RESOLVED_SHADOW_ATLAS_BYTES: usize = size_of::<ResolvedShadowAtlas>();

impl ResolvedShadowAtlas {
    /// The disabled selection — all-zero faces, `active_layers == 0`, `mode_word == 0`. The
    /// resolve of a disabled [`ShadowConfig`] and the value [`ResolvedShadowAtlas::default`]
    /// returns.
    pub const DISABLED: Self = Self {
        faces: [FaceTransform::ZERO; M_SLOTS],
        active_layers: 0,
        mode_word: 0,
        _pad: [0; 2],
    };
}

impl Default for ResolvedShadowAtlas {
    /// The resolve of the default (disabled) [`ShadowConfig`] — the 0%-gate, so a never-run
    /// policy already carries the no-shadow selection.
    #[inline]
    fn default() -> Self {
        Self::DISABLED
    }
}

// ---- atlas-slot pack / unpack (the light-table integration helper) --------------------

/// Packs a 5-bit atlas-slot index into a light's kind word at bits `17..22`, also setting the
/// [`CASTS_SHADOW_BIT`] when the slot is real (`slot != SLOT_NONE`). The kind tag (bits `0..16`)
/// is preserved unchanged. The inverse is [`light_atlas_slot`].
///
/// The slot occupies the field above the kind tag and the casts-shadow bit, so it NEVER
/// collides with either (proven by `pack_atlas_slot_never_collides`). A `slot == SLOT_NONE`
/// (the "no map" sentinel) leaves [`CASTS_SHADOW_BIT`] clear, so the resolve falls back to the
/// analytic term for that light.
///
/// `slot` MUST be `< M_SLOTS` or exactly [`SLOT_NONE`]; a debug build asserts it (a larger
/// value would overflow the 5-bit field and corrupt the kind tag).
#[inline]
pub fn pack_atlas_slot(kind_word: u32, slot: u32) -> u32 {
    debug_assert!(
        (slot as usize) < M_SLOTS || slot == SLOT_NONE,
        "invariant: atlas slot must be a real layer (< M_SLOTS) or SLOT_NONE"
    );
    // Clear any prior slot field + casts bit, then write the new slot (masked to 5 bits) and
    // set the casts bit iff the slot is real.
    let base = kind_word & !(ATLAS_SLOT_MASK << ATLAS_SLOT_SHIFT) & !CASTS_SHADOW_BIT;
    let with_slot = base | ((slot & ATLAS_SLOT_MASK) << ATLAS_SLOT_SHIFT);
    if slot == SLOT_NONE {
        with_slot
    } else {
        with_slot | CASTS_SHADOW_BIT
    }
}

/// Unpacks the 5-bit atlas-slot index from a light's kind word (the inverse of
/// [`pack_atlas_slot`]). Returns the slot index `[0, M_SLOTS)` or [`SLOT_NONE`].
#[inline]
pub fn light_atlas_slot(kind_word: u32) -> u32 {
    (kind_word >> ATLAS_SLOT_SHIFT) & ATLAS_SLOT_MASK
}

// ---- the resolve decision (pure — the unit-testable slot assignment + fit) -------------

/// A spot's world inputs for the fit: the cone apex (world position), the cone axis (the
/// world "direction the light shines along" — `-direction`, since `SpotLight::direction` is
/// "direction TO the light"), the outer cone half-angle (radians), the range, and the priority
/// proxy. Decoupled from `SpotLight` / `GlobalTransform` so the pure core is testable with
/// plain data.
#[derive(Clone, Copy, Debug)]
pub struct SpotShadowInput {
    /// Cone apex — the world position of the spot (the perspective eye).
    pub position: [f32; 3],
    /// Cone axis — the world direction the light SHINES along (the look target is
    /// `position + axis`).
    pub axis: [f32; 3],
    /// Outer cone half-angle in radians (the perspective half-FOV; the full FOV is `2·outer`).
    pub outer_rad: f32,
    /// Cone range (the far plane of the perspective map; floored to keep `near < far`).
    pub range: f32,
    /// The screen-coverage priority proxy (higher ⇒ assigned a slot first).
    pub priority: f32,
}

/// A point's world inputs for the cube fit: the light world position (the shared cube center /
/// per-face perspective eye), the range (each face's far plane + the `inv_range` distance
/// normalizer), and the priority proxy. Decoupled from `PointLight` / `GlobalTransform` so the
/// pure core is testable with plain data.
#[derive(Clone, Copy, Debug)]
pub struct PointShadowInput {
    /// The light world position — the shared center of the six cube faces (each face's eye) and
    /// the `light_pos` the resolve's distance-compare reads.
    pub position: [f32; 3],
    /// The light range (each cube face's far plane, floored to keep `near < far`; its reciprocal
    /// is `inv_range`, the distance normalizer both the depth FS and the resolve compare share).
    pub range: f32,
    /// The screen-coverage priority proxy (higher ⇒ assigned the six contiguous layers first).
    pub priority: f32,
}

/// Fits the spot atlas faces + assigns slots — the PURE, unit-testable shadow-atlas resolve
/// (the spot analogue of [`resolve_csm`](crate::csm_config::resolve_csm), the core the cold
/// system wraps). Returns the derived [`ResolvedShadowAtlas`] AND, in `out_slots`, the per-spot
/// slot index (parallel to `spots`) the caller packs into each spot's light-table entry.
///
/// `out_slots` MUST be the same length as `spots` (one slot per spot); a debug build asserts it.
///
/// Disabled (`!cfg.enabled()`) or no eligible spots ⇒ [`ResolvedShadowAtlas::DISABLED`]
/// (all-zero, `mode_word == 0`) and every `out_slots[i] == SLOT_NONE`. Else:
///
/// 1. **Priority** per spot = `luminance(color) · range² / max(dist_to_camera², EPS)` (a
///    screen-coverage proxy — no projection needed); the caller bakes it into
///    [`SpotShadowInput::priority`].
/// 2. **Top-K partial select** (K = [`M_SLOTS`]) into a FIXED `[(f32, usize); M_SLOTS]` stack
///    scratch — NO heap sort, NO allocation. The K highest-priority spots get a slot; the rest
///    get [`SLOT_NONE`].
/// 3. **Bump-allocate** one layer per selected spot (spot = 1 layer); for each selected spot at
///    layer `L`, compute the spot's perspective `view_proj` (`look_at` from the apex along the
///    cone axis, FOV `2·outer`, near [`SPOT_SHADOW_NEAR`], far `range`; column-major) →
///    `faces[L]`, and record `out_slots[spot] = L`.
/// 4. `active_layers = count`, `mode_word = (count > 0) as u32`.
pub fn resolve_shadow_atlas_spots(
    cfg: &ShadowConfig,
    spots: &[SpotShadowInput],
    out_slots: &mut [u32],
) -> ResolvedShadowAtlas {
    // The spot-only entry point delegates to the unified core with an empty point set, so the
    // Inc-1 SPOT path (and its goldens) keep their exact contract.
    let mut no_points: [u32; 0] = [];
    resolve_shadow_atlas_inputs(cfg, spots, &[], out_slots, &mut no_points)
}

/// A combined punctual source for the unified top-K rank — a spot or a point, indexed back into
/// its own input slice. The priority lane drives the descending select; the layer COST (`1` for a
/// spot, [`POINT_FACE_COUNT`] for a point) drives the bump-allocate budget check.
#[derive(Clone, Copy)]
enum PunctualRef {
    Spot(usize),
    Point(usize),
}

/// Fits the spot + point atlas faces + assigns slots — the PURE, unit-testable unified shadow-atlas
/// resolve. Points and spots are RANKED TOGETHER by [`SpotShadowInput::priority`] /
/// [`PointShadowInput::priority`]; a selected SPOT bump-allocates ONE layer and a selected POINT
/// six CONTIGUOUS layers ([`POINT_FACE_COUNT`]), in descending-priority order, until a source no
/// longer fits the remaining `[next..M_SLOTS)` budget — that source and every lower-priority one get
/// [`SLOT_NONE`]. Returns the derived [`ResolvedShadowAtlas`] AND, in `out_spot_slots` /
/// `out_point_slots`, each source's assigned slot (the LAYER for a spot, the slot BASE `b` for a
/// point — its six faces occupy `b..b+POINT_FACE_COUNT`).
///
/// `out_spot_slots.len() == spots.len()` and `out_point_slots.len() == points.len()`; a debug build
/// asserts both.
///
/// Disabled (`!cfg.enabled()`) or no eligible sources ⇒ [`ResolvedShadowAtlas::DISABLED`] and every
/// `out_*_slots[i] == SLOT_NONE`.
pub fn resolve_shadow_atlas_inputs(
    cfg: &ShadowConfig,
    spots: &[SpotShadowInput],
    points: &[PointShadowInput],
    out_spot_slots: &mut [u32],
    out_point_slots: &mut [u32],
) -> ResolvedShadowAtlas {
    debug_assert_eq!(spots.len(), out_spot_slots.len(), "invariant: one slot per spot");
    debug_assert_eq!(points.len(), out_point_slots.len(), "invariant: one slot per point");

    // Default every source to the analytic fallback; selected ones overwrite their slot below.
    for s in out_spot_slots.iter_mut() {
        *s = SLOT_NONE;
    }
    for s in out_point_slots.iter_mut() {
        *s = SLOT_NONE;
    }

    if !cfg.enabled() || (spots.is_empty() && points.is_empty()) {
        return ResolvedShadowAtlas::DISABLED;
    }

    // ---- top-K partial selection into a fixed stack scratch (no heap, no sort) ----
    //
    // `top` holds the up-to-K highest-priority `(priority, ref)` pairs seen so far, kept descending
    // by priority. K is bounded by the source count a full atlas can hold: at most `M_SLOTS` spots
    // (1 layer each), so `M_SLOTS` candidate entries always suffice (a point costs 6 layers, so
    // even fewer points fit). A candidate is inserted iff it beats the current weakest; an
    // insertion shifts the tail down by one. The O(K) shifted-insert is cheaper than any heap and
    // allocates nothing. The bump-allocate below applies the per-source LAYER COST against the
    // 16-layer budget, so the rank is over ALL sources but the fit stops when the budget is spent.
    let mut top: [(f32, PunctualRef); M_SLOTS] = [(f32::NEG_INFINITY, PunctualRef::Spot(usize::MAX)); M_SLOTS];
    let mut filled: usize = 0;

    let consider = |p: f32, r: PunctualRef, top: &mut [(f32, PunctualRef); M_SLOTS], filled: &mut usize| {
        // Skip non-positive (or NaN) priority sources (zero luminance / zero range).
        if p <= 0.0 || p.is_nan() {
            return;
        }
        // If the buffer is full and this candidate cannot beat the weakest kept, drop it.
        if *filled == M_SLOTS && p <= top[M_SLOTS - 1].0 {
            return;
        }
        // Find the insertion point (first slot whose priority is strictly less than `p`).
        let mut pos = (*filled).min(M_SLOTS - 1);
        while pos > 0 && top[pos - 1].0 < p {
            pos -= 1;
        }
        // Shift the tail down by one (drop the weakest when full), then place the candidate.
        let end = if *filled < M_SLOTS { *filled } else { M_SLOTS - 1 };
        let mut j = end;
        while j > pos {
            top[j] = top[j - 1];
            j -= 1;
        }
        top[pos] = (p, r);
        if *filled < M_SLOTS {
            *filled += 1;
        }
    };

    for (i, spot) in spots.iter().enumerate() {
        consider(spot.priority, PunctualRef::Spot(i), &mut top, &mut filled);
    }
    for (i, point) in points.iter().enumerate() {
        consider(point.priority, PunctualRef::Point(i), &mut top, &mut filled);
    }

    // ---- bump-allocate the layers in descending-priority order + fit each source's faces ----
    //
    // A spot takes 1 layer; a point takes 6 CONTIGUOUS layers. A source is placed only if its full
    // layer cost fits the remaining `[next..M_SLOTS)` budget; the first source that does not fit
    // (and every lower-priority source after it) stays on the analytic fallback (`SLOT_NONE`).
    let mut faces = [FaceTransform::ZERO; M_SLOTS];
    let mut next: usize = 0;
    for &(_, r) in top.iter().take(filled) {
        match r {
            PunctualRef::Spot(idx) => {
                if next + 1 > M_SLOTS {
                    continue; // a smaller source might still fit a 1-layer gap — keep scanning
                }
                faces[next] = spot_face(&spots[idx]);
                out_spot_slots[idx] = next as u32;
                next += 1;
            }
            PunctualRef::Point(idx) => {
                if next + POINT_FACE_COUNT > M_SLOTS {
                    continue;
                }
                let cube = point_faces(&points[idx]);
                faces[next..next + POINT_FACE_COUNT].copy_from_slice(&cube);
                out_point_slots[idx] = next as u32;
                next += POINT_FACE_COUNT;
            }
        }
    }

    if next == 0 {
        return ResolvedShadowAtlas::DISABLED;
    }

    ResolvedShadowAtlas {
        faces,
        active_layers: next as u32,
        mode_word: 1,
        _pad: [0; 2],
    }
}

/// Builds one spot's atlas face: the column-major perspective `view_proj` of its cone plus the
/// POINT-shared `light_pos` / `inv_range` lanes (unused by the SPOT NDC-z compare). The
/// perspective looks from the apex along the cone axis (a degenerate axis substitutes `-Z`),
/// FOV `2·outer`, near [`SPOT_SHADOW_NEAR`], far `max(range, MIN_SPOT_FAR)`.
#[inline]
fn spot_face(spot: &SpotShadowInput) -> FaceTransform {
    let eye = Vec3::new(spot.position[0], spot.position[1], spot.position[2]);

    // The cone axis (world direction the light shines along); substitute -Z if degenerate so
    // the look-at + perspective stay finite.
    let axis_raw = Vec3::new(spot.axis[0], spot.axis[1], spot.axis[2]);
    let axis = if axis_raw.length_squared() > MIN_DIR_LEN_SQ {
        axis_raw.normalize()
    } else {
        Vec3::new(0.0, 0.0, -1.0)
    };
    let target = eye + axis;

    // World up hint; `Affine3A::look_at_rh` swaps to a valid fallback when `up ∥ axis`, so a
    // straight-up / straight-down spot stays non-singular without a guard here.
    let world_up = Vec3::new(0.0, 1.0, 0.0);

    // Light VIEW = inverse of the light WORLD look-at (world → light-view).
    let light_world = Affine3A::look_at_rh(eye, target, world_up);
    let light_view = light_world.inverse().unwrap_or(Affine3A::IDENTITY);

    // Perspective: full FOV = 2 · outer half-angle, square aspect, near/far the cone depth
    // range (far floored so `near < far` on a zero-range spot).
    let fov_y = 2.0 * spot.outer_rad;
    let far = spot.range.max(MIN_SPOT_FAR);
    let proj = Mat4::perspective_rh(fov_y, SPOT_ASPECT, SPOT_SHADOW_NEAR, far);

    let view_proj = proj.mul_mat4(light_view.to_mat4());

    let inv_range = if far > 0.0 { far.recip() } else { 0.0 };

    FaceTransform {
        view_proj: mat4_to_cols_array(view_proj),
        light_pos: spot.position,
        inv_range,
    }
}

/// Builds a POINT light's six cube-face atlas faces: for each of the ±X/±Y/±Z axes, a 90°-FOV
/// perspective `view_proj` looking from the light position down that axis, with the shared
/// `light_pos` (the cube center, the resolve's distance-compare origin) + `inv_range` (the radial
/// distance normalizer). The face ORDER is `[+X, -X, +Y, -Y, +Z, -Z]` — the standard cube-map
/// major-axis order the resolve's face-select indexes (axis `0..3`, then the sign within the axis).
///
/// Each face is a square (aspect 1) perspective, near [`POINT_SHADOW_NEAR`], far
/// `max(range, MIN_POINT_FAR)`, column-major. The far plane only conditions the rasterizer
/// projection; the stored depth is the LINEAR radial distance the FS writes (`SV_Depth`), so the
/// resolve compares `dist(P, light_pos) * inv_range` against it (NOT the perspective NDC-z).
#[inline]
fn point_faces(point: &PointShadowInput) -> [FaceTransform; POINT_FACE_COUNT] {
    let eye = Vec3::new(point.position[0], point.position[1], point.position[2]);
    let far = point.range.max(MIN_POINT_FAR);
    let inv_range = if point.range > 0.0 { point.range.recip() } else { 0.0 };
    let proj = Mat4::perspective_rh(POINT_FACE_FOV_Y, SPOT_ASPECT, POINT_SHADOW_NEAR, far);

    // The six face look directions, in `[+X, -X, +Y, -Y, +Z, -Z]` order. For the ±Y faces the
    // standard +Y world-up is collinear with the look axis; `Affine3A::look_at_rh` swaps to a valid
    // fallback when `up ∥ axis`, so a ±Y face stays non-singular without a per-axis up table.
    const DIRS: [Vec3; POINT_FACE_COUNT] = [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(0.0, 0.0, -1.0),
    ];
    let world_up = Vec3::new(0.0, 1.0, 0.0);

    let mut out = [FaceTransform::ZERO; POINT_FACE_COUNT];
    for (face, &dir) in out.iter_mut().zip(DIRS.iter()) {
        let light_world = Affine3A::look_at_rh(eye, eye + dir, world_up);
        let light_view = light_world.inverse().unwrap_or(Affine3A::IDENTITY);
        let view_proj = proj.mul_mat4(light_view.to_mat4());
        *face = FaceTransform {
            view_proj: mat4_to_cols_array(view_proj),
            light_pos: point.position,
            inv_range,
        };
    }
    out
}

/// Decomposes a column-major [`Mat4`] into the `[[f32; 4]; 4]` column array
/// [`FaceTransform::view_proj`] stores (column `j` is `m.cols[j]`). Mirrors the CSM
/// `mat4_to_cols_array`.
#[inline]
fn mat4_to_cols_array(m: Mat4) -> [[f32; 4]; 4] {
    let col = |v: Vec4| [v.x, v.y, v.z, v.w];
    [
        col(m.cols[0]),
        col(m.cols[1]),
        col(m.cols[2]),
        col(m.cols[3]),
    ]
}

/// Rec. 709 relative luminance of a LINEAR RGB color — the magnitude term of the priority
/// proxy.
#[inline]
fn luminance(color: [f32; 3]) -> f32 {
    LUMA_R * color[0] + LUMA_G * color[1] + LUMA_B * color[2]
}

/// The screen-coverage priority proxy for a spot: `luminance(color) · range² /
/// max(dist_to_camera², EPS)`. Higher ⇒ a brighter / closer / wider-reaching spot, assigned a
/// slot first. A pure function of the spot + camera position (no projection), so it is cheap
/// and the cold policy computes one per eligible spot.
#[inline]
pub fn spot_priority(color: [f32; 3], range: f32, position: [f32; 3], camera_pos: [f32; 3]) -> f32 {
    let dx = position[0] - camera_pos[0];
    let dy = position[1] - camera_pos[1];
    let dz = position[2] - camera_pos[2];
    let dist_sq = (dx * dx + dy * dy + dz * dz).max(PRIORITY_DIST_EPS);
    luminance(color) * range * range / dist_sq
}

// ---- the cold StrategyPolicy system (mirrors resolve_csm_cascades) --------------------

/// The cold shadow-atlas resolve policy — reads [`ShadowConfig`], the active [`ViewUniform`],
/// and the eligible spots (`With<CastsPunctualShadow>`), and writes the derived
/// [`ResolvedShadowAtlas`]. The spot/point analogue of
/// [`resolve_csm_cascades`](crate::csm_config::resolve_csm_cascades). It is the SINGLE owner of
/// [`ResolvedShadowAtlas`] (the one-producer write discipline).
///
/// # Eligibility is structural (critic-C2 mirror)
///
/// Only spots carrying [`CastsPunctualShadow`] are gathered (the `With<CastsPunctualShadow>`
/// filter) — a light without the marker is structurally skipped, exactly as a CSM caster
/// carries [`ShadowCaster`](crate::csm_marker::ShadowCaster). The marker is the per-LIGHT
/// "eligible for an exact map" capability.
///
/// # The slot-pack integration seam (Inc-1-GPU)
///
/// This system writes [`ResolvedShadowAtlas`] (the faces + counts). The per-spot slot
/// assignment ([`pack_atlas_slot`] into the light-table entry's `dir_kind.w`) is NOT applied
/// here: the L0 light-table assembly ([`fold_light_table`](crate::light_system::fold_light_table))
/// is order-keyed (it walks `&SpotLight` iterators with no entity id) and `Changed`-gated, while
/// this policy is priority- and entity-keyed, so threading the per-spot slot through the fold
/// is an Inc-1-GPU light-table-assembly change. The pure core
/// [`resolve_shadow_atlas_spots`] RETURNS the per-spot slot assignments and
/// [`pack_atlas_slot`] / [`light_atlas_slot`] are ready for that wiring; the documented seam
/// mirrors the unwired
/// [`gather_shadow_casters`](crate::csm_caster::gather_shadow_casters) /
/// [`gather_mesh_draws`](crate::mesh_draw::gather_mesh_draws) gather APIs.
///
/// Cold by construction (zero hot-path cost): a single fit run once per frame; the per-row
/// render path never reads [`ShadowConfig`].
//
// `clippy::needless_pass_by_value`: `Res`/`ResMut`/`Query` are by-value `SystemParam`s
// read/written through reborrows — the same false-positive `resolve_csm_cascades` carries.
#[allow(clippy::needless_pass_by_value)]
pub fn resolve_shadow_atlas(
    cfg: Res<ShadowConfig>,
    view: Res<ViewUniform>,
    spots: Query<(&SpotLight, &GlobalTransform), With<CastsPunctualShadow>>,
    points: Query<(&PointLight, &GlobalTransform), With<CastsPunctualShadow>>,
    mut out: ResMut<ResolvedShadowAtlas>,
) {
    if !cfg.enabled() {
        *out = ResolvedShadowAtlas::DISABLED;
        return;
    }

    let cam = view.camera_pos.xyz();
    let camera_pos = [cam.x, cam.y, cam.z];

    // Gather the eligible spots + points into FIXED stack scratches bounded by `M_SLOTS`
    // (Principle 1/5 — no heap, no per-frame `Vec`). The live source count can exceed the layer
    // budget, so each gather keeps only the top-`M_SLOTS` by priority via a weakest-slot
    // replacement: a candidate past capacity displaces the current weakest only when it is
    // stronger. A point costs 6 layers, so at most `M_SLOTS / POINT_FACE_COUNT` points and
    // `M_SLOTS` spots can be SELECTED; bounding each gather buffer at `M_SLOTS` is a safe upper
    // bound (the core's bump-allocate applies the real per-source layer cost). The core then
    // ranks points + spots TOGETHER + fits + assigns slots over the bounded set.
    let mut spot_inputs: [SpotShadowInput; M_SLOTS] = [SPOT_INPUT_ZERO; M_SLOTS];
    let mut spot_slots: [u32; M_SLOTS] = [SLOT_NONE; M_SLOTS];
    let mut spot_count = 0usize;
    for (spot, gt) in spots.iter() {
        let priority = spot_priority(spot.color, spot.range, spot.position, camera_pos);
        if priority <= 0.0 || priority.is_nan() {
            continue;
        }
        let input = spot_input_from(spot, gt, priority);
        if spot_count < M_SLOTS {
            spot_inputs[spot_count] = input;
            spot_count += 1;
            continue;
        }
        let mut weakest = 0usize;
        for k in 1..M_SLOTS {
            if spot_inputs[k].priority < spot_inputs[weakest].priority {
                weakest = k;
            }
        }
        if priority > spot_inputs[weakest].priority {
            spot_inputs[weakest] = input;
        }
    }

    let mut point_inputs: [PointShadowInput; M_SLOTS] = [POINT_INPUT_ZERO; M_SLOTS];
    let mut point_slots: [u32; M_SLOTS] = [SLOT_NONE; M_SLOTS];
    let mut point_count = 0usize;
    for (point, _gt) in points.iter() {
        let priority = spot_priority(point.color, point.range, point.position, camera_pos);
        if priority <= 0.0 || priority.is_nan() {
            continue;
        }
        let input = PointShadowInput {
            position: point.position,
            range: point.range,
            priority,
        };
        if point_count < M_SLOTS {
            point_inputs[point_count] = input;
            point_count += 1;
            continue;
        }
        let mut weakest = 0usize;
        for k in 1..M_SLOTS {
            if point_inputs[k].priority < point_inputs[weakest].priority {
                weakest = k;
            }
        }
        if priority > point_inputs[weakest].priority {
            point_inputs[weakest] = input;
        }
    }

    *out = resolve_shadow_atlas_inputs(
        &cfg,
        &spot_inputs[..spot_count],
        &point_inputs[..point_count],
        &mut spot_slots[..spot_count],
        &mut point_slots[..point_count],
    );
}

/// A zero [`SpotShadowInput`] for the fixed gather scratch (overwritten before use; never read
/// while zero because `count` bounds the valid prefix).
const SPOT_INPUT_ZERO: SpotShadowInput = SpotShadowInput {
    position: [0.0; 3],
    axis: [0.0, 0.0, -1.0],
    outer_rad: 0.0,
    range: 0.0,
    priority: 0.0,
};

/// A zero [`PointShadowInput`] for the fixed gather scratch (overwritten before use; never read
/// while zero because `count` bounds the valid prefix).
const POINT_INPUT_ZERO: PointShadowInput = PointShadowInput {
    position: [0.0; 3],
    range: 0.0,
    priority: 0.0,
};

/// Builds a [`SpotShadowInput`] from a live `SpotLight` + its `GlobalTransform`. The cone apex
/// is the world position; the cone axis is the world direction the light shines along
/// (`-direction`, since `SpotLight::direction` is "direction TO the light"); the outer
/// half-angle comes from `outer_deg`.
#[inline]
fn spot_input_from(spot: &SpotLight, _gt: &GlobalTransform, priority: f32) -> SpotShadowInput {
    SpotShadowInput {
        position: spot.position,
        axis: [-spot.direction[0], -spot.direction[1], -spot.direction[2]],
        outer_rad: spot.outer_deg.to_radians(),
        range: spot.range,
        priority,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::light::{LIGHT_KIND_POINT, LIGHT_KIND_SPOT};

    /// A spot input with explicit priority (apex at origin, axis -Z, 30° outer, range 10).
    fn spot(priority: f32) -> SpotShadowInput {
        SpotShadowInput {
            position: [0.0, 0.0, 0.0],
            axis: [0.0, 0.0, -1.0],
            outer_rad: 30.0_f32.to_radians(),
            range: 10.0,
            priority,
        }
    }

    fn enabled_cfg() -> ShadowConfig {
        ShadowConfig { enabled: true, ..ShadowConfig::default() }
    }

    fn all_finite(m: &[[f32; 4]; 4]) -> bool {
        m.iter().flatten().all(|x| x.is_finite())
    }

    #[test]
    fn const_sizes_are_pinned() {
        assert_eq!(size_of::<FaceTransform>(), 80);
        assert_eq!(size_of::<ResolvedShadowAtlas>(), 1296);
        assert_eq!(M_SLOTS, 16);
        assert_eq!(SLOT_NONE, 0x1F);
    }

    #[test]
    fn disabled_or_empty_is_the_zero_gate() {
        // Disabled config.
        let mut slots = [SLOT_NONE; 4];
        let spots = [spot(1.0), spot(2.0), spot(3.0), spot(4.0)];
        let r = resolve_shadow_atlas_spots(&ShadowConfig::default(), &spots, &mut slots);
        assert_eq!(r, ResolvedShadowAtlas::DISABLED);
        assert_eq!(r.mode_word, 0);
        assert_eq!(r.active_layers, 0);
        assert!(slots.iter().all(|&s| s == SLOT_NONE));

        // Enabled but no spots.
        let mut none: [u32; 0] = [];
        let r2 = resolve_shadow_atlas_spots(&enabled_cfg(), &[], &mut none);
        assert_eq!(r2, ResolvedShadowAtlas::DISABLED);
        assert_eq!(r2.mode_word, 0);
    }

    #[test]
    fn default_resolved_matches_disabled() {
        assert_eq!(ResolvedShadowAtlas::default(), ResolvedShadowAtlas::DISABLED);
    }

    #[test]
    fn top_k_picks_highest_priority_and_over_budget_get_slot_none() {
        // 20 spots (> M_SLOTS == 16), priorities 1..=20; the top 16 (priorities 5..=20) get a
        // slot, the bottom 4 (priorities 1..=4) get SLOT_NONE. Never more than M_SLOTS layers.
        let n = 20usize;
        let spots: Vec<SpotShadowInput> = (0..n).map(|i| spot((i + 1) as f32)).collect();
        let mut slots = vec![SLOT_NONE; n];
        let r = resolve_shadow_atlas_spots(&enabled_cfg(), &spots, &mut slots);

        assert_eq!(r.active_layers, M_SLOTS as u32);
        assert!(r.active_layers as usize <= M_SLOTS);

        // The 16 highest-priority spots (indices 4..20, priorities 5..20) each got a real slot;
        // the 4 lowest (indices 0..4) got SLOT_NONE.
        for (i, &s) in slots.iter().enumerate() {
            if i >= n - M_SLOTS {
                assert_ne!(s, SLOT_NONE, "spot {i} (high priority) must get a slot");
                assert!((s as usize) < M_SLOTS, "slot {s} must be in [0, M_SLOTS)");
            } else {
                assert_eq!(s, SLOT_NONE, "spot {i} (over budget) must get SLOT_NONE");
            }
        }
    }

    #[test]
    fn one_layer_per_selected_spot_contiguous() {
        // 5 spots, all selected (< M_SLOTS): exactly 5 contiguous layers [0..5), each slot
        // unique, and exactly active_layers slots are real.
        let spots = [spot(10.0), spot(20.0), spot(30.0), spot(40.0), spot(50.0)];
        let mut slots = [SLOT_NONE; 5];
        let r = resolve_shadow_atlas_spots(&enabled_cfg(), &spots, &mut slots);
        assert_eq!(r.active_layers, 5);

        let mut seen = [false; M_SLOTS];
        let mut real = 0;
        for &s in &slots {
            if s != SLOT_NONE {
                let l = s as usize;
                assert!(l < 5, "selected slot must land in [0, active_layers)");
                assert!(!seen[l], "slot {l} assigned twice");
                seen[l] = true;
                real += 1;
            }
        }
        assert_eq!(real, 5);
        // Layers are contiguous [0..5).
        for (l, &occupied) in seen.iter().enumerate().take(5) {
            assert!(occupied, "layer {l} must be occupied (contiguous bump alloc)");
        }
        // The faces past active_layers stay zero.
        for f in r.faces.iter().skip(5) {
            assert_eq!(*f, FaceTransform::ZERO);
        }
    }

    #[test]
    fn spot_view_proj_is_finite_and_projects_cone_center_in_bounds() {
        // A single spot at the origin looking down -Z, 30° outer, range 10. The view_proj must
        // be finite, and a point on the cone axis at half-range must project inside the NDC
        // box (clip.xyz / clip.w in [-1,1]×[-1,1]×[0,1]).
        let spots = [spot(1.0)];
        let mut slots = [SLOT_NONE; 1];
        let r = resolve_shadow_atlas_spots(&enabled_cfg(), &spots, &mut slots);
        assert_eq!(r.active_layers, 1);
        assert_eq!(slots[0], 0);

        let vp = &r.faces[0].view_proj;
        assert!(all_finite(vp), "spot view_proj must be finite");

        // Cone-center sample: 5 units down -Z (inside near 0.05 .. far 10).
        let p = [0.0_f32, 0.0, -5.0, 1.0];
        // clip = view_proj · p (column-major: sum of columns weighted by p components).
        let mut clip = [0.0_f32; 4];
        for row in 0..4 {
            clip[row] = vp[0][row] * p[0] + vp[1][row] * p[1] + vp[2][row] * p[2] + vp[3][row] * p[3];
        }
        assert!(clip[3] > 0.0, "cone center must be in front of the spot (w > 0)");
        let ndc_x = clip[0] / clip[3];
        let ndc_y = clip[1] / clip[3];
        let ndc_z = clip[2] / clip[3];
        assert!(ndc_x.abs() <= 1.0 + 1e-4, "cone center x in NDC bounds, got {ndc_x}");
        assert!(ndc_y.abs() <= 1.0 + 1e-4, "cone center y in NDC bounds, got {ndc_y}");
        assert!((0.0..=1.0 + 1e-4).contains(&ndc_z), "cone center z in [0,1], got {ndc_z}");
    }

    #[test]
    fn straight_down_spot_view_proj_is_finite() {
        // A spot shining straight down (axis ∥ world-up): look_at_rh's pole guard keeps the
        // view finite — no NaN escapes.
        let s = SpotShadowInput {
            position: [0.0, 10.0, 0.0],
            axis: [0.0, -1.0, 0.0],
            outer_rad: 25.0_f32.to_radians(),
            range: 12.0,
            priority: 1.0,
        };
        let mut slots = [SLOT_NONE; 1];
        let r = resolve_shadow_atlas_spots(&enabled_cfg(), &[s], &mut slots);
        assert_eq!(r.active_layers, 1);
        assert!(all_finite(&r.faces[0].view_proj), "straight-down spot view_proj must be finite");
    }

    #[test]
    fn pack_atlas_slot_round_trips() {
        // A real slot round-trips and sets the casts bit; SLOT_NONE round-trips and leaves the
        // casts bit clear.
        for kind in [LIGHT_KIND_SPOT, LIGHT_KIND_POINT] {
            for slot in 0..M_SLOTS as u32 {
                let word = pack_atlas_slot(kind, slot);
                assert_eq!(light_atlas_slot(word), slot, "slot {slot} must round-trip");
                assert_ne!(word & CASTS_SHADOW_BIT, 0, "a real slot must set CASTS_SHADOW_BIT");
                // The kind tag (bits 0..16) is preserved.
                assert_eq!(word & 0xFFFF, kind, "kind tag must be preserved");
            }
            let none = pack_atlas_slot(kind, SLOT_NONE);
            assert_eq!(light_atlas_slot(none), SLOT_NONE, "SLOT_NONE must round-trip");
            assert_eq!(none & CASTS_SHADOW_BIT, 0, "SLOT_NONE must leave CASTS_SHADOW_BIT clear");
            assert_eq!(none & 0xFFFF, kind, "kind tag must be preserved for SLOT_NONE");
        }
    }

    /// A point input with explicit priority (at the origin, range 10).
    fn point(priority: f32) -> PointShadowInput {
        PointShadowInput { position: [0.0, 0.0, 0.0], range: 10.0, priority }
    }

    /// Projects a world direction `dir` through face `f`'s `view_proj` (column-major) and returns
    /// the post-perspective-divide NDC plus the clip `w` — used to assert each cube face covers its
    /// own axis direction (a ray down `dir` lands in-bounds in the matching face).
    fn project_face(view_proj: &[[f32; 4]; 4], p: [f32; 3]) -> ([f32; 3], f32) {
        let pw = [p[0], p[1], p[2], 1.0];
        let mut clip = [0.0f32; 4];
        for (r, clip_r) in clip.iter_mut().enumerate() {
            *clip_r = view_proj[0][r] * pw[0]
                + view_proj[1][r] * pw[1]
                + view_proj[2][r] * pw[2]
                + view_proj[3][r] * pw[3];
        }
        if clip[3].abs() < 1e-12 {
            return ([0.0; 3], clip[3]);
        }
        ([clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]], clip[3])
    }

    #[test]
    fn point_consumes_six_contiguous_layers_with_light_pos_and_inv_range() {
        // One point (range 10) at a known position consumes exactly 6 contiguous layers [0..6),
        // each face finite + carrying the shared light_pos + inv_range = 1/range.
        let pos = [1.0, 2.0, -3.0];
        let pts = [PointShadowInput { position: pos, range: 10.0, priority: 5.0 }];
        let mut spot_none: [u32; 0] = [];
        let mut pslots = [SLOT_NONE; 1];
        let r = resolve_shadow_atlas_inputs(&enabled_cfg(), &[], &pts, &mut spot_none, &mut pslots);

        assert_eq!(r.active_layers, POINT_FACE_COUNT as u32);
        assert_eq!(pslots[0], 0, "the point's slot base is layer 0");
        for f in r.faces.iter().take(POINT_FACE_COUNT) {
            assert!(all_finite(&f.view_proj), "every cube face view_proj must be finite");
            assert_eq!(f.light_pos, pos, "every face shares the light position");
            assert!((f.inv_range - 0.1).abs() < 1e-6, "inv_range == 1/range");
        }
        // The faces past the cube stay zero (bound-but-unread).
        for f in r.faces.iter().skip(POINT_FACE_COUNT) {
            assert_eq!(*f, FaceTransform::ZERO);
        }
    }

    #[test]
    fn point_cube_faces_cover_all_six_axis_directions() {
        // A ray a short distance down each ±axis must project IN-BOUNDS in the matching face
        // (the union of the six 90°-FOV faces covers the full sphere of directions).
        let pts = [point(5.0)];
        let mut spot_none: [u32; 0] = [];
        let mut pslots = [SLOT_NONE; 1];
        let r = resolve_shadow_atlas_inputs(&enabled_cfg(), &[], &pts, &mut spot_none, &mut pslots);

        // Face order: [+X, -X, +Y, -Y, +Z, -Z]; sample 2 units down each axis from the origin.
        let samples: [[f32; 3]; POINT_FACE_COUNT] = [
            [2.0, 0.0, 0.0],
            [-2.0, 0.0, 0.0],
            [0.0, 2.0, 0.0],
            [0.0, -2.0, 0.0],
            [0.0, 0.0, 2.0],
            [0.0, 0.0, -2.0],
        ];
        for (face, sample) in samples.iter().enumerate() {
            let (ndc, w) = project_face(&r.faces[face].view_proj, *sample);
            assert!(w > 0.0, "face {face}: the axis sample must be in front (w > 0), got {w}");
            assert!(ndc[0].abs() <= 1.0 + 1e-4, "face {face}: ndc.x in bounds, got {}", ndc[0]);
            assert!(ndc[1].abs() <= 1.0 + 1e-4, "face {face}: ndc.y in bounds, got {}", ndc[1]);
        }
    }

    #[test]
    fn priority_interleaves_points_and_spots() {
        // A point (priority 100) outranks a spot (priority 1): the point takes layers [0..6) and the
        // spot takes layer 6. Reversing the priorities flips which gets the leading layers.
        let spots = [spot(1.0)];
        let pts = [point(100.0)];
        let mut sslots = [SLOT_NONE; 1];
        let mut pslots = [SLOT_NONE; 1];
        let r = resolve_shadow_atlas_inputs(&enabled_cfg(), &spots, &pts, &mut sslots, &mut pslots);
        assert_eq!(r.active_layers, POINT_FACE_COUNT as u32 + 1);
        assert_eq!(pslots[0], 0, "the higher-priority point gets the leading 6 layers");
        assert_eq!(sslots[0], POINT_FACE_COUNT as u32, "the spot follows at layer 6");

        let spots2 = [spot(100.0)];
        let pts2 = [point(1.0)];
        let mut sslots2 = [SLOT_NONE; 1];
        let mut pslots2 = [SLOT_NONE; 1];
        let r2 = resolve_shadow_atlas_inputs(&enabled_cfg(), &spots2, &pts2, &mut sslots2, &mut pslots2);
        assert_eq!(sslots2[0], 0, "the higher-priority spot now leads at layer 0");
        assert_eq!(pslots2[0], 1, "the point follows at layer 1 (base of its 6-face cube)");
        assert_eq!(r2.active_layers, POINT_FACE_COUNT as u32 + 1);
    }

    #[test]
    fn over_budget_points_get_slot_none() {
        // Three points each cost 6 layers (18 > M_SLOTS == 16): only the two highest-priority fit
        // (12 layers); the lowest-priority point is over budget → SLOT_NONE.
        let pts = [point(30.0), point(20.0), point(10.0)];
        let mut spot_none: [u32; 0] = [];
        let mut pslots = [SLOT_NONE; 3];
        let r = resolve_shadow_atlas_inputs(&enabled_cfg(), &[], &pts, &mut spot_none, &mut pslots);

        assert_eq!(r.active_layers, (2 * POINT_FACE_COUNT) as u32);
        assert_eq!(pslots[0], 0, "the strongest point leads at layer 0");
        assert_eq!(pslots[1], POINT_FACE_COUNT as u32, "the next point at layer 6");
        assert_eq!(pslots[2], SLOT_NONE, "the over-budget point falls back to SLOT_NONE");
    }

    #[test]
    fn point_slot_base_packs_round_trips() {
        // A point's slot base (its cube starts at `b`, faces b..b+6) packs into the 5-bit slot field
        // and round-trips — the resolve reads `base` then offsets by the per-face index.
        for base in [0u32, 6, 10] {
            let word = pack_atlas_slot(LIGHT_KIND_POINT, base);
            assert_eq!(light_atlas_slot(word), base, "point slot base {base} must round-trip");
            assert_ne!(word & CASTS_SHADOW_BIT, 0, "a real point slot sets CASTS_SHADOW_BIT");
            assert_eq!(word & 0xFFFF, LIGHT_KIND_POINT, "the POINT kind tag is preserved");
        }
    }

    #[test]
    fn pack_atlas_slot_never_collides() {
        // The slot field (bits 17..22) and the casts bit (bit 16) must NEVER touch the kind tag
        // (bits 0..16) for any slot in [0, M_SLOTS) or SLOT_NONE. Sweep every kind tag value and
        // every slot; the low 16 bits must equal the original kind.
        for kind in 0u32..=0xFFFF {
            for slot in (0..M_SLOTS as u32).chain(core::iter::once(SLOT_NONE)) {
                let word = pack_atlas_slot(kind, slot);
                assert_eq!(word & 0xFFFF, kind, "slot pack collided with kind {kind} (slot {slot})");
                assert_eq!(light_atlas_slot(word), slot, "slot {slot} must survive next to kind {kind}");
            }
        }
    }
}
