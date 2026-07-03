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

use boyko_macros::{Resource, SystemSet};

use boyko_ecs::ecs::core::iters::query::{Query, With};
use boyko_ecs::ecs::core::system::{Res, ResMut};

use boyko_ecs::ecs::identifiers::primitives::EntityId;
use boyko_math::{Mat4, Vec3, Vec4};
use boyko_scene::{GlobalTransform, ViewUniform};

use crate::csm_caster::CsmCasterScratch;
use crate::light::{LightTableDirty, LightingConfig, PointLight, SpotLight};
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

// The resolve's point-cube shadow UV reconstruction (`punctual_atlas_visibility` in
// `deferred_pbr.hlsl`) is a deliberate 90°-face SPECIALIZATION: it DROPS the perspective
// `f = cot(FOV/2)` factor (which equals 1 ONLY at a 90° face) and rebuilds each face's UV
// by hand from the major axis, instead of sampling the uploaded per-face `view_proj` the
// way the arbitrary-FOV spot path does. That is correct BECAUSE cube faces are square 90°
// frusta — cheaper (no per-pixel mat-vec) and exact. Pin the precondition: if this FOV ever
// leaves 90°, `f != 1` and the hand-coded resolve silently drifts, so break the build rather
// than ship a mis-projected point shadow. (Bit-compare, not `==`, to sidestep
// `clippy::float_cmp`; the value is defined as `FRAC_PI_2`, so only a deliberate edit fails it.)
const _: () = assert!(
    POINT_FACE_FOV_Y.to_bits() == core::f32::consts::FRAC_PI_2.to_bits(),
    "punctual_atlas_visibility (deferred_pbr.hlsl) assumes 90° cube faces (f = 1); \
     update the resolve's hand-coded UV reconstruction if POINT_FACE_FOV_Y changes",
);

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
    /// Per-slot spot-vs-point tag — a bitmask where bit `s` set ⇒ atlas layer `s` is a POINT cube
    /// face, clear ⇒ a SPOT map (or an unused layer). The SINGLE SOURCE OF TRUTH the host feeds
    /// into the recorder's `PunctualDepthActivation.face_is_point` (do NOT re-derive from
    /// `inv_range`, which is set for both spot and point, nor from contiguity-of-six).
    ///
    /// Occupies the FIRST former pad word: the resolve shader's b15 cbuffer declares this word as
    /// `_gAtlasPad.x` and NEVER reads it (only the 1296-byte stride matters), so this is a
    /// host-only field that keeps the UBO layout byte-identical.
    pub face_point_mask: u32,
    /// Padding to the 16-byte stride after the trailing words (the shader's `_gAtlasPad.y`).
    pub _pad: u32,
}

// Layout pin: 80 × 16 + 4 + 4 + 4 + 4 = 1280 + 16 = 1296 B — the `ResolvedCsm` fingerprint shape
// UNCHANGED (`face_point_mask` repurposes the first former pad word; the shader's b15 cbuffer
// declares it as an unread `_gAtlasPad` lane, so the 1296-byte stride is preserved).
const _: () = assert!(size_of::<ResolvedShadowAtlas>() == 1296);
const _: () = assert!(core::mem::offset_of!(ResolvedShadowAtlas, faces) == 0);
const _: () = assert!(core::mem::offset_of!(ResolvedShadowAtlas, active_layers) == 1280);
const _: () = assert!(core::mem::offset_of!(ResolvedShadowAtlas, mode_word) == 1284);
const _: () = assert!(core::mem::offset_of!(ResolvedShadowAtlas, face_point_mask) == 1288);

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
        face_point_mask: 0,
        _pad: 0,
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

// ---- PunctualResolveSet (the cross-plugin resolve → light-table ordering seam) --------

/// The `Main`-schedule ordering seam that pins the punctual shadow resolve BEFORE the light-table
/// fold — the spot/point analogue of [`CameraSet`](boyko_scene::CameraSet).
///
/// # Why a named set, not add-order
///
/// [`resolve_shadow_atlas`] (in [`ShadowAtlasPlugin`](crate::shadow_plugin::ShadowAtlasPlugin))
/// publishes each punctual light's atlas base into [`PunctualSlotAssignment`], and
/// [`collect_lights`](crate::light_system::collect_lights) (in
/// [`LightingPlugin`](crate::light_plugin::LightingPlugin)) READS it to pack the base into the
/// light-table kind word. The two live in DIFFERENT plugins, so their per-system `SystemKey`s are
/// not co-visible — a `.after(key)` edge is impossible across the plugin boundary. A set-to-set
/// edge is pinned **by name** and holds REGARDLESS of plugin add-order (the R6
/// [`CameraSet`](boyko_scene::CameraSet) precedent).
///
/// # Why it must be a HARD edge (not the CSM stagger)
///
/// The atlas base assignment is priority-ranked by `range² / dist_to_camera²`, so a MOVING camera
/// can reorder which light wins base 0 frame-to-frame. An add-order stagger would then produce a
/// one-frame-WRONG shadow DURING camera motion (a light sampling the previous winner's map) — the
/// "wrong-only-in-motion" class this engine has been bitten by. This hard edge makes resolve →
/// publish → collect happen within ONE frame, so frame 0 (and every moving-camera frame) is already
/// correct.
///
/// * [`resolve_shadow_atlas`] joins this set (`.in_set(PunctualResolveSet)`);
/// * [`collect_lights`] runs `.after_set(PunctualResolveSet)`;
/// * both are registered into `CoreSchedule::Main`, so the edge binds.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Debug)]
pub struct PunctualResolveSet;

// ---- PunctualSlotAssignment (the entity-keyed resolve → light-table handoff) ----------

/// The per-light atlas-base handoff (Inc-1-GPU wiring): the resolve
/// ([`resolve_shadow_atlas`]) publishes each SELECTED punctual light's assigned atlas base
/// slot here, keyed by [`EntityId`], and the light-table fold
/// ([`collect_lights`](crate::light_system::collect_lights)) reads it to pack the base into that
/// light's kind word via [`pack_atlas_slot`].
///
/// # Why entity-keyed, not order-coupled
///
/// The resolve ranks lights by PRIORITY (a `With<CastsPunctualShadow>` top-K fit), while the fold
/// walks the point/spot query iterators in ARCHETYPE order — the two orders do NOT coincide. So a
/// winner is stored with its [`EntityId`] and looked up by id in the fold (`base_for`), never by a
/// positional index. Only the WINNERS are stored (a light that won no slot is simply absent →
/// `base_for` returns [`SLOT_NONE`], the analytic fallback), so a `SLOT_NONE` loser is the DEFAULT,
/// not a stored entry.
///
/// # Storage (Principle 0/1/5 — ECS-native, no heap, no map)
///
/// A `#[derive(Resource)]` `World`-singleton holding a FIXED inline `[(EntityId, u32); M_SLOTS]`
/// array (at most `M_SLOTS` winners — one spot per layer is the tightest bound; a point costs six
/// layers, so fewer fit). Lookups are a linear scan over `len` entries (≤ 16, cold path once per
/// light-table rebuild) — no `HashMap`, no `Vec`. `Copy` so the resolve overwrites it by value and
/// the change-detect compare is a plain `!=`.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct PunctualSlotAssignment {
    /// The `(entity, base)` winners; only `[0..len)` are valid. `base` is the atlas layer index a
    /// spot occupies (1 layer) or a point cube's first face (`POINT_FACE_COUNT` contiguous layers).
    winners: [(EntityId, u32); M_SLOTS],
    /// The number of valid `winners` entries (`0` when no light won a slot — the empty handoff).
    len: u32,
}

impl PunctualSlotAssignment {
    /// The empty handoff — no winners. The value a disabled resolve (0%-gate) publishes and the
    /// [`Default`], so the fold reads [`SLOT_NONE`] for every light (byte-identical to the
    /// pre-wiring path).
    pub const EMPTY: Self = Self { winners: [(EntityId(0), 0); M_SLOTS], len: 0 };

    /// Looks up the assigned atlas base for `entity`, or [`SLOT_NONE`] when the light won no slot
    /// (the analytic fallback). A linear scan over the `len` winners — `len <= M_SLOTS == 16`, run
    /// once per punctual light on a light-table rebuild (cold), so no map is warranted.
    #[inline]
    pub fn base_for(&self, entity: EntityId) -> u32 {
        // `len` bounds the valid prefix; the tail is uninitialised sentinel `(EntityId(0), 0)`.
        for &(id, base) in &self.winners[..self.len as usize] {
            if id == entity {
                return base;
            }
        }
        SLOT_NONE
    }

    /// Records a `(entity, base)` winner. Silently ignores an overflow past [`M_SLOTS`] (the resolve
    /// never selects more than `M_SLOTS` sources — the caller's top-K fit bounds it — so this is a
    /// defensive clamp, not a live branch). `base` MUST be a real layer (`< M_SLOTS`); the resolve
    /// only records selected sources, never [`SLOT_NONE`].
    #[inline]
    fn push(&mut self, entity: EntityId, base: u32) {
        debug_assert!(
            (base as usize) < M_SLOTS,
            "invariant: only a SELECTED source (real layer < M_SLOTS) is recorded"
        );
        let i = self.len as usize;
        if i < M_SLOTS {
            self.winners[i] = (entity, base);
            self.len = self.len.wrapping_add(1);
        }
    }

    /// The builder form of [`push`](Self::push) — returns `self` with the `(entity, base)` winner
    /// appended. The `base` MUST be a real layer (`< M_SLOTS`). Used to assemble an assignment in a
    /// functional style (tests, and any producer building the handoff off-system).
    #[inline]
    pub fn with_winner(mut self, entity: EntityId, base: u32) -> Self {
        self.push(entity, base);
        self
    }
}

impl Default for PunctualSlotAssignment {
    #[inline]
    fn default() -> Self {
        Self::EMPTY
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
/// world direction the light shines along — `SpotLight::direction` UN-negated, since
/// `light_reconcile` writes it as the transform's world `-Z` = the shine axis), the outer
/// cone half-angle (radians), the range, and the priority proxy. Decoupled from `SpotLight` /
/// `GlobalTransform` so the pure core is testable with plain data.
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
    // Bit `s` set ⇒ layer `s` is a POINT cube face; clear ⇒ a SPOT map (or unused). The single
    // source of truth for the host's `PunctualDepthActivation.face_is_point`.
    let mut face_point_mask: u32 = 0;
    let mut next: usize = 0;
    for &(_, r) in top.iter().take(filled) {
        match r {
            PunctualRef::Spot(idx) => {
                if next + 1 > M_SLOTS {
                    continue; // a smaller source might still fit a 1-layer gap — keep scanning
                }
                faces[next] = spot_face(&spots[idx]);
                out_spot_slots[idx] = next as u32;
                // A spot leaves its mask bit clear (the default) — no write needed.
                next += 1;
            }
            PunctualRef::Point(idx) => {
                if next + POINT_FACE_COUNT > M_SLOTS {
                    continue;
                }
                let cube = point_faces(&points[idx]);
                faces[next..next + POINT_FACE_COUNT].copy_from_slice(&cube);
                out_point_slots[idx] = next as u32;
                // Tag all six cube-face layers as POINT.
                for s in next..next + POINT_FACE_COUNT {
                    face_point_mask |= 1 << s;
                }
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
        face_point_mask,
        _pad: 0,
    }
}

/// Builds one spot's atlas face: the column-major perspective `view_proj` of its cone plus the
/// POINT-shared `light_pos` / `inv_range` lanes (unused by the SPOT NDC-z compare). The
/// perspective looks from the apex along the cone axis (a degenerate axis substitutes `-Z`),
/// FOV `2·outer`, near [`SPOT_SHADOW_NEAR`], far `max(range, MIN_SPOT_FAR)`.
///
/// The `view_proj` is built by the hand-rolled [`look_at_perspective_view_proj`] convention
/// (`clip.y = -f·y` Y-flip, positive-z-into-scene depth) — the SAME convention the resolve
/// shader's spot sample (`deferred_pbr.hlsl` `spot_atlas_visibility`, `uv.y = ndc.y*0.5+0.5`
/// with NO second flip) REQUIRES. It must NOT be re-derived through the generic
/// `Mat4::perspective_rh` (m[1][1] = +f, no Y-flip) + `Affine3A::look_at_rh` (camera looks down
/// -Z, z negative into scene) helpers — those carry the OPPOSITE convention and produce a
/// vertically-mirrored map (masked only when the caster sits on the cone axis, the flip fixed
/// point). See the CSM standing warning (`csm_config.rs` doc step 6) for the same discipline.
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

    // Full FOV = 2 · outer half-angle; `cot(half_fov)` (= `f`) is the perspective scale the
    // hand-rolled builder needs — pass the outer HALF-angle directly.
    let far = spot.range.max(MIN_SPOT_FAR);
    let view_proj = look_at_perspective_view_proj(eye, axis, spot.outer_rad, SPOT_SHADOW_NEAR, far);

    let inv_range = if far > 0.0 { far.recip() } else { 0.0 };

    FaceTransform {
        view_proj: mat4_to_cols_array(view_proj),
        light_pos: spot.position,
        inv_range,
    }
}

/// The near-collinear threshold (|dot(fwd, world-up)|) at which the look-at up-hint swaps from
/// `+Y` to `+Z`, mirroring the golden `spot_demo_view_proj` pole guard exactly (so a straight-up
/// / straight-down spot and every ±Y point cube face pick the SAME basis the resolve shader's
/// per-face `uvc` sign table is derived from).
const UP_HINT_SWAP_DOT: f32 = 0.99;

/// Builds a column-major world→light-clip `view_proj` in the ENGINE convention the resolve
/// shader requires — a hand-rolled right-handed look-at along `axis` plus a Vulkan-`[0,1]`
/// perspective with the framebuffer Y-flip baked into row 1 (`clip.y = -f·y`) and depth growing
/// positively into the scene (`clip.w = z_light`, `z_light = fwd·(P-eye) > 0`).
///
/// This is the byte/epsilon-exact port of the golden `spot_demo_view_proj`
/// (`window_present_gbuffer.rs`) into `boyko_math`; both the spot map and the point cube faces
/// use it, so production and the golden produce identical matrices for identical inputs. The
/// generic `Mat4::perspective_rh` + `Affine3A::look_at_rh` helpers carry the OPPOSITE
/// (un-flipped, look-down-`-Z`) convention and MUST NOT be substituted here.
///
/// `axis` is the world direction the light shines along (assumed already substituted for a valid
/// default if degenerate); `outer_rad` is the perspective HALF-FOV (`cot(outer_rad) = f`); `near`
/// / `far` are the view-space clip planes (`near < far`, floored by the caller).
#[inline]
fn look_at_perspective_view_proj(
    eye: Vec3,
    axis: Vec3,
    outer_rad: f32,
    near: f32,
    far: f32,
) -> Mat4 {
    // RH look-at basis (the golden convention): fwd = the look/shine direction; up-hint swaps
    // +Y → +Z when nearly collinear with fwd (the pole guard). right = norm(up_hint × fwd),
    // up = fwd × right. The view rows are (right, up, fwd), so z_light = fwd·(P-eye) is POSITIVE
    // into the scene.
    let fwd = if axis.length_squared() > MIN_DIR_LEN_SQ {
        axis.normalize()
    } else {
        Vec3::new(0.0, 0.0, -1.0)
    };
    let up_hint = if fwd.dot(Vec3::new(0.0, 1.0, 0.0)).abs() > UP_HINT_SWAP_DOT {
        Vec3::new(0.0, 0.0, 1.0)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };
    let right = up_hint.cross(fwd).normalize();
    let up = fwd.cross(right);

    // light_view translation = -dot(basis, eye).
    let tx = -right.dot(eye);
    let ty = -up.dot(eye);
    let tz = -fwd.dot(eye);

    // Perspective (Vulkan [0,1] depth, square aspect): clip.x = f·x, clip.y = -f·y (the Y-flip),
    // clip.z = far/zr·z - far·near/zr, clip.w = z. `f = cot(outer_rad)` (half-FOV = outer_rad).
    let f = 1.0 / outer_rad.tan();
    let zr = far - near;

    // pv[row][col] = proj_row · light_view_row (the golden `spot_demo_view_proj` assembly).
    let pv: [[f32; 4]; 4] = [
        [f * right.x, f * right.y, f * right.z, f * tx],
        [-f * up.x, -f * up.y, -f * up.z, -f * ty],
        [
            (far / zr) * fwd.x,
            (far / zr) * fwd.y,
            (far / zr) * fwd.z,
            (far / zr) * tz - far * near / zr,
        ],
        [fwd.x, fwd.y, fwd.z, tz],
    ];

    // COLUMN-MAJOR storage: cols[col][row] = pv[row][col].
    Mat4 {
        cols: [
            Vec4::new(pv[0][0], pv[1][0], pv[2][0], pv[3][0]),
            Vec4::new(pv[0][1], pv[1][1], pv[2][1], pv[3][1]),
            Vec4::new(pv[0][2], pv[1][2], pv[2][2], pv[3][2]),
            Vec4::new(pv[0][3], pv[1][3], pv[2][3], pv[3][3]),
        ],
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
///
/// Each face is built by the hand-rolled [`look_at_perspective_view_proj`] convention (the same
/// engine `-f·up` Y-flip + positive-z depth as the spot map and the golden `point_face_view_proj`),
/// with the FOV's outer HALF-angle = 45° (`POINT_FACE_FOV_Y / 2`), so the per-face RH basis
/// (`right = up_hint × fwd`, `up = fwd × right`, `up_hint = +Y` except `+Z` for a ±Y axis) matches
/// the basis the resolve shader's per-face `uvc` sign table (`deferred_pbr.hlsl` L683-694) is derived
/// from. It must NOT be re-derived through the generic un-flipped helpers.
#[inline]
fn point_faces(point: &PointShadowInput) -> [FaceTransform; POINT_FACE_COUNT] {
    let eye = Vec3::new(point.position[0], point.position[1], point.position[2]);
    let far = point.range.max(MIN_POINT_FAR);
    let inv_range = if point.range > 0.0 { point.range.recip() } else { 0.0 };

    // The point cube uses a 90° full FOV per face; the hand-rolled builder takes the outer
    // HALF-angle (45°), so `cot(45°) = 1` — the standard cube-map fit.
    let half_fov = POINT_FACE_FOV_Y * 0.5;

    // The six face look directions, in `[+X, -X, +Y, -Y, +Z, -Z]` order. For the ±Y faces the
    // standard +Y up-hint is collinear with the look axis; `look_at_perspective_view_proj` swaps
    // the up-hint to +Z (the `UP_HINT_SWAP_DOT` guard), so a ±Y face stays non-singular AND lands
    // on the +Z-up basis the shader's ±Y `uvc` table (sc = ∓x, tc = -z) expects.
    const DIRS: [Vec3; POINT_FACE_COUNT] = [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(0.0, 0.0, -1.0),
    ];

    let mut out = [FaceTransform::ZERO; POINT_FACE_COUNT];
    for (face, &dir) in out.iter_mut().zip(DIRS.iter()) {
        let view_proj = look_at_perspective_view_proj(eye, dir, half_fov, POINT_SHADOW_NEAR, far);
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
    mut assignment: ResMut<PunctualSlotAssignment>,
    mut table_dirty: ResMut<LightTableDirty>,
) {
    if !cfg.enabled() {
        *out = ResolvedShadowAtlas::DISABLED;
        // 0%-gate: publish the empty handoff so the fold packs NOTHING (every punctual row's
        // `dir_kind.w` stays byte-identical to the pre-wiring path). Value-gated so a static
        // disabled frame never dirties the light table.
        publish_assignment(&mut assignment, &mut table_dirty, PunctualSlotAssignment::EMPTY);
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
    //
    // A PARALLEL entity array tracks each kept input's owning `EntityId` (swapped in lock-step
    // with the input on a weakest replacement) so the assignment publish below can key each
    // resolved base back to its light — the fold walks a DIFFERENT (archetype) order, so the
    // handoff MUST be entity-keyed, never positional.
    let mut spot_inputs: [SpotShadowInput; M_SLOTS] = [SPOT_INPUT_ZERO; M_SLOTS];
    let mut spot_ents: [EntityId; M_SLOTS] = [EntityId(0); M_SLOTS];
    let mut spot_slots: [u32; M_SLOTS] = [SLOT_NONE; M_SLOTS];
    let mut spot_count = 0usize;
    for (entity, (spot, gt)) in spots.iter_entities() {
        let priority = spot_priority(spot.color, spot.range, spot.position, camera_pos);
        if priority <= 0.0 || priority.is_nan() {
            continue;
        }
        let input = spot_input_from(spot, gt, priority);
        if spot_count < M_SLOTS {
            spot_inputs[spot_count] = input;
            spot_ents[spot_count] = entity;
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
            spot_ents[weakest] = entity;
        }
    }

    let mut point_inputs: [PointShadowInput; M_SLOTS] = [POINT_INPUT_ZERO; M_SLOTS];
    let mut point_ents: [EntityId; M_SLOTS] = [EntityId(0); M_SLOTS];
    let mut point_slots: [u32; M_SLOTS] = [SLOT_NONE; M_SLOTS];
    let mut point_count = 0usize;
    for (entity, (point, _gt)) in points.iter_entities() {
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
            point_ents[point_count] = entity;
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
            point_ents[weakest] = entity;
        }
    }

    *out = resolve_shadow_atlas_inputs(
        &cfg,
        &spot_inputs[..spot_count],
        &point_inputs[..point_count],
        &mut spot_slots[..spot_count],
        &mut point_slots[..point_count],
    );

    // Publish the per-light base handoff: read back the slot the core assigned each SELECTED
    // source and store it keyed by the gathered `EntityId`. `SLOT_NONE` (a loser / over-budget
    // source) is NOT recorded — its absence makes `base_for` return `SLOT_NONE` (the analytic
    // fallback), so the fold packs no map for it. `spot_ents[i]` / `point_ents[i]` line up with
    // `spot_slots[i]` / `point_slots[i]` (both indexed by the SAME gather position `i`), which is
    // why the entity arrays are swapped in lock-step with the inputs on a weakest replacement.
    let mut next = PunctualSlotAssignment::EMPTY;
    for i in 0..spot_count {
        if spot_slots[i] != SLOT_NONE {
            next.push(spot_ents[i], spot_slots[i]);
        }
    }
    for i in 0..point_count {
        if point_slots[i] != SLOT_NONE {
            next.push(point_ents[i], point_slots[i]);
        }
    }
    publish_assignment(&mut assignment, &mut table_dirty, next);
}

/// Publishes the per-light atlas-base handoff into the [`PunctualSlotAssignment`] resource,
/// value-gated: the store + the [`LightTableDirty`] mark fire ONLY when the assignment actually
/// changed. This is the resolve → fold synchronisation edge (mirrors
/// [`sync_punctual_light_gate`]'s value-gated `LightTableDirty` write): the fold
/// ([`collect_lights`](crate::light_system::collect_lights)) is `Changed`-gated and does not see a
/// resource-only mutation, so a real assignment change must dirty the table for the fold to re-pack
/// the new bases. A static frame writes nothing and never dirties the table.
#[inline]
fn publish_assignment(
    assignment: &mut PunctualSlotAssignment,
    table_dirty: &mut LightTableDirty,
    next: PunctualSlotAssignment,
) {
    if *assignment != next {
        *assignment = next;
        table_dirty.0 = true;
    }
}

/// Bridges the [`ResolvedShadowAtlas`] resolve and the [`LightingConfig`] header gate — the
/// spot/point analogue of [`sync_csm_light_gate`](crate::csm_caster::sync_csm_light_gate). It is
/// the SINGLE production writer of [`LightingConfig::punctual_shadows`], keeping the header's
/// word-7 punctual bit ([`PUNCTUAL_MODE_BIT`](crate::light::PUNCTUAL_MODE_BIT)) in lock-step with
/// the depth-pass activation predicate: **a fitted atlas** (`resolved.mode_word == 1`) **AND live
/// casters** (`casters.batch_count() > 0`). The casters are the SAME
/// [`CsmCasterScratch`](crate::csm_caster::CsmCasterScratch) the CSM path gathers —
/// [`ShadowCaster`](crate::shadow_marker::ShadowCaster) meshes cast into BOTH the cascade array
/// and the punctual atlas, so one gather feeds both gates.
///
/// # The never-rendered-VALUES invariant (review W3)
///
/// Every SAMPLED layer `s < active_layers` was rendered THIS frame (the slot-pack guarantees a
/// bump-allocated layer is written by the depth pass); an unslotted light (`SLOT_NONE`) takes the
/// analytic fallback and never samples an unwritten layer; the host boot-seed covers only the
/// first-frame / gate-on-before-render LAYOUT defense; the depth pass barriers the WHOLE `M_SLOTS`
/// array to `SHADER_READ_ONLY_OPTIMAL`. So no sampled layer is ever unwritten or wrong-layout, and
/// soundness does NOT rest on this system's 1–2-frame flip timing (the same discipline the CSM
/// sync documents).
///
/// # Value-gated write
///
/// `cfg.punctual_shadows` is written only on an actual flip, so a static frame does zero work and
/// never dirties the light table.
///
/// # Registration — app-wired (matches [`sync_csm_light_gate`])
///
/// NOT registered by any plugin here: it bridges the shadow-atlas plugin's
/// [`ResolvedShadowAtlas`] and the lighting plugin's [`LightingConfig`] / [`LightTableDirty`], so
/// only the composing app (which adds BOTH) may register it — after `resolve_shadow_atlas` (the
/// `mode_word` producer) and the caster gather, in the same builder closure as
/// `sync_csm_light_gate`.
#[allow(clippy::needless_pass_by_value)]
pub fn sync_punctual_light_gate(
    resolved: Res<ResolvedShadowAtlas>,
    casters: Res<CsmCasterScratch>,
    mut cfg: ResMut<LightingConfig>,
    mut dirty: ResMut<LightTableDirty>,
) {
    let on = resolved.mode_word == 1 && casters.batch_count() > 0;
    // Value gate BEFORE the `DerefMut`: flip-only write, flip-only table dirtying.
    if cfg.punctual_shadows != on {
        cfg.punctual_shadows = on;
        dirty.0 = true;
    }
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
/// is the world position; the cone axis is the world direction the light SHINES along, taken
/// from `SpotLight::direction` UN-negated.
///
/// This resolve runs AFTER `light_reconcile`, which writes `direction` as the transform's
/// world `-Z` — i.e. the SHINE direction for a `look_at_rh(pos, target)`-aimed spot — the
/// same convention the pool-lighting cone test consumes (`dot(-l, dir)`). Negating it here
/// (the old "direction is TO-light" reading) pointed the shadow-map camera 180° away, so the
/// caster fell behind the near plane and no shadow was ever written. The golden spot builds
/// its map from the un-negated shine axis; this matches it.
#[inline]
fn spot_input_from(spot: &SpotLight, _gt: &GlobalTransform, priority: f32) -> SpotShadowInput {
    SpotShadowInput {
        position: spot.position,
        axis: spot.direction,
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

    /// The GOLDEN spot/point-face `view_proj` reference — a self-contained plain-`[f32; 16]`
    /// re-implementation of the on-screen-proven harness helper `spot_demo_view_proj`
    /// (`crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs`), with NO `boyko_math` dep, so
    /// the arbiter test compares production `spot_face`/`point_faces` against the EXACT convention
    /// the resolve shader (`deferred_pbr.hlsl`) samples. Returns 16 COLUMN-MAJOR floats
    /// (`out[col*4 + row]`), the same order [`FaceTransform::view_proj`] stores as `[[f32; 4]; 4]`
    /// columns (`view_proj[col][row]`).
    ///
    /// `axis` is the world direction the light shines along; `outer_rad` is the perspective
    /// HALF-FOV (full FOV = `2·outer_rad`). This carries the engine's `clip.y = -f·y` Y-flip and
    /// the positive-z-into-scene depth (`clip.w = z_light`) — the convention production MUST match.
    fn golden_view_proj(eye: [f32; 3], axis: [f32; 3], outer_rad: f32, near: f32, far: f32) -> [f32; 16] {
        let norm = |v: [f32; 3]| {
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            [v[0] / l, v[1] / l, v[2] / l]
        };
        let cross = |a: [f32; 3], b: [f32; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

        let fwd = norm(axis);
        // Pole guard: swap the +Y up-hint to +Z when nearly collinear with fwd (matches the
        // harness helper and production `UP_HINT_SWAP_DOT`).
        let up_hint = if dot(fwd, [0.0, 1.0, 0.0]).abs() > 0.99 {
            [0.0, 0.0, 1.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let right = norm(cross(up_hint, fwd));
        let up = cross(fwd, right);
        let tx = -dot(right, eye);
        let ty = -dot(up, eye);
        let tz = -dot(fwd, eye);

        let far = far.max(near + 1.0e-3);
        let f = 1.0 / outer_rad.tan();
        let zr = far - near;
        let pv: [[f32; 4]; 4] = [
            [f * right[0], f * right[1], f * right[2], f * tx],
            [-f * up[0], -f * up[1], -f * up[2], -f * ty],
            [
                (far / zr) * fwd[0],
                (far / zr) * fwd[1],
                (far / zr) * fwd[2],
                (far / zr) * tz - far * near / zr,
            ],
            [fwd[0], fwd[1], fwd[2], tz],
        ];
        let mut out = [0.0f32; 16];
        for col in 0..4 {
            for row in 0..4 {
                out[col * 4 + row] = pv[row][col];
            }
        }
        out
    }

    /// Asserts a production `[[f32; 4]; 4]` column-major `view_proj` equals a golden `[f32; 16]`
    /// column-major reference within `eps`, naming the differing lane. Keyed on the FULL matrix
    /// (not just row 1) so a convention drift in ANY lane is caught.
    fn assert_view_proj_eq(prod: &[[f32; 4]; 4], golden: &[f32; 16], eps: f32, ctx: &str) {
        for col in 0..4 {
            for row in 0..4 {
                let a = prod[col][row];
                let b = golden[col * 4 + row];
                assert!(
                    (a - b).abs() <= eps,
                    "{ctx}: view_proj[col={col}][row={row}] production {a} != golden {b} (diff {})",
                    (a - b).abs()
                );
            }
        }
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

    // ---- STEP-1 arbiter: production fit matches the golden (on-screen-proven) convention ----
    //
    // The P0 bug: production `spot_face`/`point_faces` formerly built `view_proj` via
    // `Mat4::perspective_rh` (m[1][1] = +f, NO Y-flip) + `Affine3A::look_at_rh` (camera looks down
    // -Z), while the resolve shader (`deferred_pbr.hlsl`) samples `uv.y = ndc.y*0.5+0.5` with NO
    // second flip and REQUIRES the matrix to carry the `-f·up` Y-flip + positive-z depth (the
    // convention the harness golden `spot_demo_view_proj` uses). Production was latently
    // vertically-MIRRORED, masked on the cone axis (the flip fixed point). These tests are
    // OFF-AXIS (not the fixed point), keyed on the flipped-Y sign, so they FAIL pre-fix and PASS
    // post-fix. `spot_view_proj_is_finite_and_projects_cone_center_in_bounds` above is flip-BLIND
    // (on-axis) — this is the objective arbiter.

    #[test]
    fn spot_face_matches_golden_convention_off_axis() {
        // An OFF-AXIS spot: apex up and to the side, shining diagonally down-forward, so the basis
        // right/up rows carry non-trivial values (the row-1 Y-flip is NOT masked). Production
        // `spot_face(view_proj)` must be epsilon-equal to the golden `spot_demo_view_proj`
        // convention for the same inputs.
        let pos = [2.0, 5.0, -1.0];
        let axis = [0.3, -1.0, 0.6]; // diagonal shine direction (normalized inside both builders)
        let outer_rad = 28.0_f32.to_radians();
        let range = 12.0;

        let input = SpotShadowInput { position: pos, axis, outer_rad, range, priority: 1.0 };
        let prod = spot_face(&input);

        let far = range.max(MIN_SPOT_FAR);
        let golden = golden_view_proj(pos, axis, outer_rad, SPOT_SHADOW_NEAR, far);

        assert_view_proj_eq(&prod.view_proj, &golden, 1.0e-5, "off-axis spot");

        // Direct flipped-Y sign anchor: an off-axis receiver at +light-up must land at the agreed
        // ndc.y sign. Sample a world point offset along the +up basis vector from the apex; with the
        // `-f·up` Y-flip, its clip.y is NEGATIVE (post-divide ndc.y < 0). A mirrored (un-flipped)
        // matrix would put it at ndc.y > 0 — the exact defect this arbiter pins.
        let fwd = {
            let l = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
            [axis[0] / l, axis[1] / l, axis[2] / l]
        };
        let up_hint = [0.0f32, 1.0, 0.0];
        // right = norm(up_hint × fwd)
        let rx = up_hint[1] * fwd[2] - up_hint[2] * fwd[1];
        let ry = up_hint[2] * fwd[0] - up_hint[0] * fwd[2];
        let rz = up_hint[0] * fwd[1] - up_hint[1] * fwd[0];
        let rl = (rx * rx + ry * ry + rz * rz).sqrt();
        let right = [rx / rl, ry / rl, rz / rl];
        // up = fwd × right
        let up = [
            fwd[1] * right[2] - fwd[2] * right[1],
            fwd[2] * right[0] - fwd[0] * right[2],
            fwd[0] * right[1] - fwd[1] * right[0],
        ];
        // A receiver 2 units down +fwd (in front, w > 0) and +1 unit along +up (off-axis, +Y side).
        let p = [
            pos[0] + fwd[0] * 2.0 + up[0],
            pos[1] + fwd[1] * 2.0 + up[1],
            pos[2] + fwd[2] * 2.0 + up[2],
            1.0,
        ];
        let vp = &prod.view_proj;
        let mut clip = [0.0f32; 4];
        for (row, c) in clip.iter_mut().enumerate() {
            *c = vp[0][row] * p[0] + vp[1][row] * p[1] + vp[2][row] * p[2] + vp[3][row] * p[3];
        }
        assert!(clip[3] > 0.0, "off-axis receiver must be in front (w > 0), got {}", clip[3]);
        let ndc_y = clip[1] / clip[3];
        assert!(
            ndc_y < 0.0,
            "a +light-up receiver must map to ndc.y < 0 under the -f·up Y-flip (mirror bug ⇒ > 0), got {ndc_y}"
        );
    }

    /// SEAM arbiter for the spot-shadow SIGN-INVERSION bug (the one the golden arbiter MISSED).
    ///
    /// The golden tests above build a [`SpotShadowInput`] DIRECTLY with a shine `axis`, bypassing
    /// the host reconcile → [`spot_input_from`] → [`spot_face`] seam where the flip actually lived:
    /// `spot_input_from` once NEGATED `SpotLight::direction` to build the cone axis, while
    /// `light_reconcile` already writes `direction` as the transform's world `-Z` (the SHINE
    /// direction). The double-negation aimed the shadow-map camera 180° AWAY, so the caster fell
    /// BEHIND the near plane (`clip.w < 0`), the atlas layer stayed cleared, and no spot shadow
    /// ever rendered. The goldens were green through the whole bug — this test closes that gap by
    /// STARTING FROM A TRANSFORM and running the real production path.
    ///
    /// Discriminator: a receiver directly under the cone (the aim target) must project IN FRONT of
    /// the near plane (`clip.w > 0`, `0 ≤ ndc.z ≤ 1`, uv in `[0,1]`). With the inverted axis it
    /// lands behind the apex (`clip.w < 0`) and the whole assertion fails — the exact defect.
    #[test]
    fn reconciled_spot_projects_cone_receiver_in_front_of_near_plane() {
        use boyko_math::Affine3A;

        // Aim a spot from `pos` at `target` the way an authored `SpotLightObject` does: a
        // `look_at_rh(pos, target, +Y)` world pose. `light_reconcile` derives `direction` from this
        // pose; we reproduce its derivation below and feed it through the production seam.
        let pos = Vec3::new(3.0, 6.0, -2.0);
        let target = Vec3::new(-1.0, 0.5, 1.0);
        let up = Vec3::new(0.0, 1.0, 0.0);
        let gt = GlobalTransform(Affine3A::look_at_rh(pos, target, up));

        // `SpotLight::direction` = the world `-Z` of the pose, normalized — this MIRRORS
        // `light_reconcile::to_light_dir` (= normalize(matrix3 · (0,0,-1))), the value
        // the reconcile system writes before this resolve runs. Deriving it here (rather than
        // hand-picking `target - pos`) is what makes the test exercise the real reconcile math.
        let dir = gt.affine().transform_vector(Vec3::new(0.0, 0.0, -1.0)).normalize();
        let spot = SpotLight {
            position: [pos.x, pos.y, pos.z],
            direction: [dir.x, dir.y, dir.z],
            color: [1.0, 1.0, 1.0],
            power: 1000.0,
            range: 20.0,
            inner_deg: 20.0,
            outer_deg: 30.0,
        };

        // The PRODUCTION path: reconcile-written `SpotLight` → `spot_input_from` (the seam that
        // held the flip) → `spot_face` → `FaceTransform::view_proj`. No hand-built input.
        let input = spot_input_from(&spot, &gt, 1.0);
        let face = spot_face(&input);
        let vp = &face.view_proj;
        assert!(all_finite(vp), "reconciled spot view_proj must be finite");

        // A receiver clearly in front of the apex, along the shine direction, inside the cone: the
        // aim target itself (between apex and beyond, well within near .. far). Under a correctly
        // aimed camera it projects in front of the near plane; under the OLD inverted axis it lands
        // behind the apex (`clip.w < 0`) — the caster-behind-near-plane failure that killed the
        // shadow. Column-major apply: clip = Σ_col vp[col] · p[col].
        let p = [target.x, target.y, target.z, 1.0];
        let mut clip = [0.0f32; 4];
        for (row, c) in clip.iter_mut().enumerate() {
            *c = vp[0][row] * p[0] + vp[1][row] * p[1] + vp[2][row] * p[2] + vp[3][row] * p[3];
        }

        assert!(
            clip[3] > 0.0,
            "cone receiver must be IN FRONT of the apex (w > 0); an inverted axis puts it behind \
             the near plane (w < 0), got w = {}",
            clip[3]
        );
        let ndc_x = clip[0] / clip[3];
        let ndc_y = clip[1] / clip[3];
        let ndc_z = clip[2] / clip[3];
        assert!(
            (0.0..=1.0 + 1e-4).contains(&ndc_z),
            "cone receiver depth must be within the map's [0,1] range, got ndc.z = {ndc_z}"
        );
        // NDC in [-1,1] ⇔ uv = ndc*0.5+0.5 in [0,1]: the receiver samples inside the atlas layer.
        let uv_u = ndc_x * 0.5 + 0.5;
        let uv_v = ndc_y * 0.5 + 0.5;
        assert!(
            (0.0..=1.0).contains(&uv_u) && (0.0..=1.0).contains(&uv_v),
            "cone receiver must sample inside the shadow map [0,1]², got uv = ({uv_u}, {uv_v})"
        );
    }

    #[test]
    fn point_face_matches_golden_convention_diagonal() {
        // A point whose +X cube face is exercised by a DIAGONAL (non-axis-aligned within the face)
        // receiver, so the minor-axis (Y) sign is load-bearing. Each of the six faces must be
        // epsilon-equal to the golden per-face convention.
        let pos = [1.0, 2.0, -3.0];
        let range = 9.0;
        let input = PointShadowInput { position: pos, range, priority: 1.0 };
        let faces = point_faces(&input);

        let far = range.max(MIN_POINT_FAR);
        let half_fov = POINT_FACE_FOV_Y * 0.5;
        const DIRS: [[f32; 3]; POINT_FACE_COUNT] = [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ];
        for (i, dir) in DIRS.iter().enumerate() {
            let golden = golden_view_proj(pos, *dir, half_fov, POINT_SHADOW_NEAR, far);
            assert_view_proj_eq(&faces[i].view_proj, &golden, 1.0e-5, &format!("point face {i}"));
        }

        // Flipped-Y sign anchor on the +X face (index 0): the shader's +X `uvc = (-dir.z, -dir.y)`
        // means a receiver at +Y (dir.y > 0) reads tc = -dir.y < 0, i.e. ndc.y < 0. Project a world
        // point diagonally off the +X axis (into +X, +Y, +Z) and assert its ndc.y is negative —
        // exactly the `-f·up` Y-flip the mirror bug inverted.
        let p = [pos[0] + 3.0, pos[1] + 1.0, pos[2] + 0.7, 1.0]; // +X dominant, +Y off-axis
        let vp = &faces[0].view_proj;
        let mut clip = [0.0f32; 4];
        for (row, c) in clip.iter_mut().enumerate() {
            *c = vp[0][row] * p[0] + vp[1][row] * p[1] + vp[2][row] * p[2] + vp[3][row] * p[3];
        }
        assert!(clip[3] > 0.0, "+X face diagonal receiver must be in front (w > 0), got {}", clip[3]);
        let ndc_y = clip[1] / clip[3];
        assert!(
            ndc_y < 0.0,
            "+X face: a +Y-offset receiver must map to ndc.y < 0 under the -f·up Y-flip (mirror bug ⇒ > 0), got {ndc_y}"
        );
    }
}
