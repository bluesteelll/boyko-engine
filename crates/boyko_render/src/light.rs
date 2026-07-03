//! Lighting L0/L1: ECS light entities → a GPU `GpuLight[]` table (std430 POD).
//!
//! Lights are authoritative ECS data (Principle 0: NO parallel `std::Vec`/`HashMap`
//! light store). [`DirectionalLight`] / [`PointLight`] / [`SpotLight`] are ordinary
//! `#[derive(Component)]` PODs; [`LightingConfig`] is a `#[derive(Resource)]`
//! singleton (exposure + sky ambient). The GPU table is a *derived upload*: a
//! `collect_lights` system folds the live components into one contiguous
//! `[LightHeaderGpu || GpuLight[]]` staging slice and the per-frame recorder records
//! a fence-free staging→device copy (rung L0-r0).
//!
//! # Layout discipline (the std430 mirror)
//!
//! [`GpuLight`] (48 B) and [`LightHeaderGpu`] (64 B) are `#[repr(C, align(16))]` with
//! clean 16-byte `vec4` lanes — NO mixed-scalar greedy packing — so the std430 mapping
//! the shader (`light_table.hlsli`) reads is unambiguous, exactly [`MaterialGpu`]'s
//! discipline. Const-assert fingerprints (size / align / every offset) pin both layouts
//! so a host/shader desync is a build error, and [`GPU_LIGHT_WORDS`] /
//! [`LIGHT_HEADER_WORDS`] mirror the shader's `GPU_LIGHT_WORDS == 12` /
//! `LIGHT_HEADER_WORDS == 16` pins.
//!
//! [`MaterialGpu`]: crate::material::MaterialGpu

use core::f32::consts::PI;

use boyko_macros::{Component, Resource};
use boyko_scene::{GlobalTransform, Transform};

// ---- light kinds (mirror the shader's LIGHT_KIND_* and the host oracle) --------------

/// Tag value for a [`DirectionalLight`] in [`GpuLight::dir_kind`]`.w` (bit-cast `u32`).
pub const LIGHT_KIND_DIRECTIONAL: u32 = 0;
/// Tag value for a [`PointLight`] (L0b resolve path).
pub const LIGHT_KIND_POINT: u32 = 1;
/// Tag value for a [`SpotLight`] (L0b resolve path).
pub const LIGHT_KIND_SPOT: u32 = 2;
/// Tag value for a [`SkyLight`] (L0a resolve path — hemisphere ambient).
pub const LIGHT_KIND_SKY: u32 = 3;

// ---- L1 cluster constants (mirror docs/LIGHTING-L0-L1-PLAN.md Decision 6) -------------

/// L1 cluster grid X dimension (froxels across the screen width).
pub const CLUSTER_DIM_X: u32 = 16;
/// L1 cluster grid Y dimension (froxels down the screen height).
pub const CLUSTER_DIM_Y: u32 = 9;
/// L1 cluster grid Z dimension (exponential-Z froxel slices).
pub const CLUSTER_DIM_Z: u32 = 24;
/// Total L1 froxel count (`X * Y * Z`).
pub const CLUSTER_COUNT: u32 = CLUSTER_DIM_X * CLUSTER_DIM_Y * CLUSTER_DIM_Z;
/// Hard cap on the GPU light table (one `MAX_LIGHTS * 48 B` SSBO ≈ 48 KiB, L2-resident).
pub const MAX_LIGHTS: u32 = 1024;
/// L1 per-froxel light-index cap (clamp-and-drop above this — Decision 6 / Algorithm D).
pub const MAX_LIGHTS_PER_CLUSTER: u32 = 256;
/// L1 flat light-index-list capacity (the `light_index` SSBO length, in `u32`s). The cull
/// pass `InterlockedAdd`-claims disjoint slices out of this flat list; a claim past the cap
/// drops the light (clamp-and-drop, Decision 6 / Algorithm D — the global tail bound that
/// backstops the per-froxel cap). 16384 `u32` = 64 KiB, sized once at setup; grown only on a
/// capacity cross (setup-class). At `CLUSTER_COUNT` (3456) froxels this averages ~4.7
/// indices/froxel before the global bound bites — ample for a sparse scene, and any
/// over-budget froxel simply drops extras (no UB, no overflow).
pub const INDEX_LIST_CAP: u32 = 16384;

// ---- L1 cluster linearization (mirror cluster_cull.hlsl + deferred_pbr.hlsl) ----------

/// Linearizes a froxel `(x, y, z)` to its flat [`ClusterCell`] index. **This is the ONE
/// source of truth for the host; the cull-write and resolve-read shaders MUST use the
/// byte-identical `(y * dimX + x) * dimZ + z`** — a mismatch silently maps a pixel to the
/// wrong cluster (Decision 6 / the task's linearization FIX). The Z (froxel-depth) slice is
/// the innermost (fastest-varying) index so a pixel's `z` walk is contiguous; debug builds
/// assert each coordinate is within its grid dimension.
#[inline]
pub const fn cluster_index(x: u32, y: u32, z: u32) -> u32 {
    debug_assert!(x < CLUSTER_DIM_X, "invariant: cluster x within CLUSTER_DIM_X");
    debug_assert!(y < CLUSTER_DIM_Y, "invariant: cluster y within CLUSTER_DIM_Y");
    debug_assert!(z < CLUSTER_DIM_Z, "invariant: cluster z within CLUSTER_DIM_Z");
    (y * CLUSTER_DIM_X + x) * CLUSTER_DIM_Z + z
}

// ---- GpuLight (std430 POD, 48 B — mirrors MaterialGpu) -------------------------------

/// One GPU light-table element — a std430-compatible POD, uploaded once / on-change.
///
/// Three 16-byte `vec4` lanes (the shader's `GpuLight`), a tagged union over the three
/// light kinds (branchless `kind` dispatch in the resolve):
///
/// - `dir_kind` (off 0): `xyz` = world direction TO the light (DIRECTIONAL/SPOT) |
///   unused (POINT); `w` = bit-cast `u32` kind tag ([`LIGHT_KIND_DIRECTIONAL`] etc.).
/// - `pos_range` (off 16): `xyz` = world position (POINT/SPOT) | unused (DIRECTIONAL);
///   `w` = cull-sphere radius (POINT/SPOT) | `+inf` (DIRECTIONAL).
/// - `color_cone` (off 32): `rgb` = LINEAR color × baked intensity (directional =
///   irradiance, point/spot = the radiant `I`); `w` = bit-cast packed spot cone cosines
///   (`cos_inner` in the low `f16`, `cos_outer` in the high `f16`; SPOT only).
///
/// All radiometric values are LINEAR.
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuLight {
    /// `xyz` = direction TO the light (DIRECTIONAL/SPOT), `w` = bit-cast kind tag.
    pub dir_kind: [f32; 4],
    /// `xyz` = world position (POINT/SPOT), `w` = cull radius (`+inf` directional).
    pub pos_range: [f32; 4],
    /// `rgb` = LINEAR color × baked intensity, `w` = packed spot cone cosines.
    pub color_cone: [f32; 4],
}

/// Number of `u32` words one [`GpuLight`] occupies (`size_of / 4` = 12). Mirrors the
/// shader's `static const uint GPU_LIGHT_WORDS = 12u`; a desync is a build error
/// host-side (the const-asserts below) and a documented pin shader-side.
pub const GPU_LIGHT_WORDS: usize = core::mem::size_of::<GpuLight>() / 4;

// ---- std430 / repr(C) layout fingerprint (mirrors the shader's GpuLight) -------------
//
// A mismatch between this Rust struct and the std430 element the shader reads is silent
// GPU corruption neither the validation layer nor a golden diff would localize. These
// const-asserts make any drift a BUILD ERROR, exactly like `MaterialGpu`'s fingerprint.
const _: () = assert!(
    core::mem::size_of::<GpuLight>() == 48,
    "GpuLight must be 48 bytes (3 std430 vec4 lanes the shader reads)"
);
const _: () = assert!(
    core::mem::align_of::<GpuLight>() == 16,
    "GpuLight must be 16-byte aligned (std430 struct alignment)"
);
const _: () = assert!(core::mem::offset_of!(GpuLight, dir_kind) == 0, "dir_kind at offset 0");
const _: () = assert!(core::mem::offset_of!(GpuLight, pos_range) == 16, "pos_range at offset 16");
const _: () =
    assert!(core::mem::offset_of!(GpuLight, color_cone) == 32, "color_cone at offset 32");
const _: () = assert!(GPU_LIGHT_WORDS == 12, "GPU_LIGHT_WORDS must equal the shader's 12u");

// ---- LightHeaderGpu (std430 header — 64 B, one cache line, 4×vec4) -------------------

/// The leading region of the light SSBO (the HEADER_BASE pattern — the `GpuLight[]`
/// array starts at [`LIGHT_HEADER_BASE_WORDS`]). `#[repr(C, align(16))]`, 4 `vec4`
/// lanes = 64 B (one cache line). Read once per dispatch (a wave-uniform broadcast).
///
/// - `counts_exposure` (off 0): `x` = bit-cast `light_count` (`u32`), `y` = exposure
///   (default 1.0 → 0%-gate identity), `z` = bit-cast `l0a_count` (`u32`, the no-`P`
///   front block = directionals + sky), `w` = bit-cast `point_spot_count` (`u32`). The
///   split counts let L0a loop the no-`P` lights `[0..l0a_count)` without touching the
///   point/spot rows that need `gViewT`/`P` (L0b).
/// - `sky_diffuse` (off 16): ambient hemisphere diffuse `rgb` (replaces the resolve's
///   `SKY_DIFFUSE` constant), `w` unused.
/// - `sky_spec` (off 32): ambient specular `rgb` (replaces `SKY_SPEC`), `w` unused.
/// - `cluster_params` (off 48): L1 froxel dims `x/y/z` (bit-cast `u32`), `w` =
///   bit-cast `clusters_enabled` (`u32`; `0` ⇒ L1 OFF, loop the flat table). **Zero in
///   L0** (L1 fills these).
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightHeaderGpu {
    /// `[bitcast(light_count), exposure, bitcast(l0a_count), bitcast(point_spot_count)]`
    /// where `l0a_count` is the no-`P` front block (directionals + sky).
    pub counts_exposure: [f32; 4],
    /// Ambient hemisphere diffuse `rgb`, `w` unused.
    pub sky_diffuse: [f32; 4],
    /// Ambient specular `rgb`, `w` unused.
    pub sky_spec: [f32; 4],
    /// L1 cluster params (zero in L0): `[bitcast(dim_x), bitcast(dim_y), bitcast(dim_z),
    /// bitcast(clusters_enabled)]`.
    pub cluster_params: [f32; 4],
}

/// Number of `u32` words one [`LightHeaderGpu`] occupies (`size_of / 4` = 16). Mirrors
/// the shader's `LIGHT_HEADER_WORDS == 16u` pin.
pub const LIGHT_HEADER_WORDS: usize = core::mem::size_of::<LightHeaderGpu>() / 4;

/// The word offset at which the `GpuLight[]` array begins in the light SSBO (the header
/// occupies words `[0..LIGHT_HEADER_BASE_WORDS)`). Mirrors the shader's `HEADER_BASE`.
pub const LIGHT_HEADER_BASE_WORDS: usize = LIGHT_HEADER_WORDS;

const _: () = assert!(
    core::mem::size_of::<LightHeaderGpu>() == 64,
    "LightHeaderGpu must be 64 bytes (one cache line — fits with exposure added)"
);
const _: () = assert!(
    core::mem::align_of::<LightHeaderGpu>() == 16,
    "LightHeaderGpu must be 16-byte aligned (std430 struct alignment)"
);
const _: () = assert!(
    core::mem::offset_of!(LightHeaderGpu, counts_exposure) == 0,
    "counts_exposure at offset 0"
);
const _: () =
    assert!(core::mem::offset_of!(LightHeaderGpu, sky_diffuse) == 16, "sky_diffuse at offset 16");
const _: () =
    assert!(core::mem::offset_of!(LightHeaderGpu, sky_spec) == 32, "sky_spec at offset 32");
const _: () = assert!(
    core::mem::offset_of!(LightHeaderGpu, cluster_params) == 48,
    "cluster_params at offset 48"
);
const _: () = assert!(LIGHT_HEADER_WORDS == 16, "LIGHT_HEADER_WORDS must equal the shader's 16u");

// ---- ClusterCell (L1 only — std430) --------------------------------------------------

/// One L1 froxel's slice into the flat light-index list: `{offset, count}` (8 B). The
/// `light_index` SSBO is a flat `[u32]` of per-cluster index slices concatenated. Defined
/// now (L1 consumes it); the L1 cull/resolve path lands in a later rung.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClusterCell {
    /// Base offset of this froxel's index slice in the flat `light_index` list.
    pub offset: u32,
    /// Number of light indices in this froxel's slice (≤ [`MAX_LIGHTS_PER_CLUSTER`]).
    pub count: u32,
}

const _: () =
    assert!(core::mem::size_of::<ClusterCell>() == 8, "ClusterCell must be 8 bytes (std430)");

// ---- LightEnabled (Axis-2 runtime on/off — EnableTag bitset) -------------------------

/// Runtime on/off switch for a light — a first-class EnableTag bitset component
/// (Principle 0: NOT a `bool` field, NOT a side store).
///
/// A fieldless ZST tagged `#[component(storage = "bitset")]`, so toggling it is the
/// O(1) `enable`/`disable` bit flip (no migration, no `Changed` tick). It is read
/// per-row in [`collect_lights`](crate::light_system::collect_lights) via the
/// non-filtering [`IsEnabled<LightEnabled>`] datum: a disabled light is skipped from
/// the GPU light table, an enabled one is folded.
///
/// # Seed / back-compat
///
/// A never-toggled row reads DISABLED (the bitset default). To keep pre-existing /
/// non-tagged lights visible, the [`LightSeedState`](crate::light_system::LightSeedState)
/// seed enables the tag on every light it has not yet seeded — so a light spawned without
/// touching `LightEnabled` still appears in the table. This is why the read uses the
/// non-filtering `IsEnabled` datum rather than the `Enabled<LightEnabled>` filter
/// (which would DROP every never-tagged row).
///
/// [`IsEnabled<LightEnabled>`]: boyko_ecs::ecs::core::iters::query::IsEnabled
#[derive(Component)]
#[component(storage = "bitset")]
pub struct LightEnabled;

/// The structural-change rebuild channel for the GPU light table (Decision 2).
///
/// A bitset toggle ([`LightEnabled`]) bumps no `Changed` tick, and a removed /
/// despawned light advances no surviving row's tick — both are INVISIBLE to the
/// `Changed`-gate in [`collect_lights`](crate::light_system::collect_lights). This
/// resource is the channel that catches them: the set-light-enabled surface and the
/// `on_remove` eviction hook set `0 = true`, and `collect_lights` consumes it (sets
/// it back to `false`) on every rebuild. It is kept distinct from
/// [`LightTableStaging`](crate::light_system::LightTableStaging)`::dirty` (the
/// GPU-upload latch) so the two state machines stay clean.
#[derive(Resource)]
pub struct LightTableDirty(pub bool);

// ---- ECS components (authoritative store — Decision 4) -------------------------------

/// A directional light (the sun): an infinitely-distant parallel beam. `#[repr(C)]` for
/// a predictable layout. Resolved in L0a (no `P` dependency).
///
/// # Required components (S8)
///
/// `#[require(Transform, GlobalTransform)]` (the `boyko_scene` pose pair) enforces
/// the invariant *a positioned/oriented light always has a pose*: inserting a
/// `DirectionalLight` alone auto-inserts a [`Transform`] / [`GlobalTransform`]
/// (each via its `Default`), so `light_reconcile` always finds a `GlobalTransform`
/// to derive the world direction from. Supplying either explicitly suppresses its
/// auto-insert. [`SkyLight`] carries NO such require — it is an environment term
/// with no position or direction.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
#[require(Transform, GlobalTransform)]
pub struct DirectionalLight {
    /// World direction TO the light (normalized host-side in the constructor).
    pub direction: [f32; 3],
    /// LINEAR `rgb` color.
    pub color: [f32; 3],
    /// Illuminance in lux (physical); the global exposure (O3) maps it to display range.
    pub illuminance: f32,
}

/// A point light: an omnidirectional source at a world position. `#[repr(C)]`. Its
/// resolve path (inverse-square attenuation) is L0b; the component is defined now.
///
/// # Required components (S8)
///
/// `#[require(Transform, GlobalTransform)]` — see [`DirectionalLight`] for the
/// pose invariant. `light_reconcile` derives `position` from the
/// [`GlobalTransform`] translation when one is present.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
#[require(Transform, GlobalTransform)]
pub struct PointLight {
    /// World position.
    pub position: [f32; 3],
    /// LINEAR `rgb` color.
    pub color: [f32; 3],
    /// Luminous power `Φ` (lumens); the baked intensity is `I = Φ / (4π)`.
    pub power: f32,
    /// Cull-sphere radius (the cutoff where attenuation is ~0).
    pub range: f32,
}

/// A spot light: a point source restricted to a cone. `#[repr(C)]`. Its resolve path
/// (inverse-square × cone falloff) is L0b; the component is defined now.
///
/// # Required components (S8)
///
/// `#[require(Transform, GlobalTransform)]` — see [`DirectionalLight`] for the
/// pose invariant. `light_reconcile` derives both `position` and `direction` from
/// the [`GlobalTransform`] when one is present.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
#[require(Transform, GlobalTransform)]
pub struct SpotLight {
    /// World position.
    pub position: [f32; 3],
    /// World spot axis (normalized host-side in the constructor).
    pub direction: [f32; 3],
    /// LINEAR `rgb` color.
    pub color: [f32; 3],
    /// Luminous power `Φ` (lumens); the baked intensity is `I = Φ / (2π(1 − cos(outer)))`
    /// (Decision 2, the reflector model).
    pub power: f32,
    /// Cull-sphere radius.
    pub range: f32,
    /// Inner cone half-angle in degrees (full intensity within).
    pub inner_deg: f32,
    /// Outer cone half-angle in degrees (zero beyond).
    pub outer_deg: f32,
}

/// A sky/ambient hemisphere light: a constant analytic ambient term with no position or
/// direction dependency. `#[repr(C)]`. Resolved in L0a (no `P` dependency) via the
/// hemisphere `lerp(ground, sky, dot(N, up) * 0.5 + 0.5)` diffuse + the analytic
/// env-BRDF specular, both modulated by AO. When `sky_color == ground_color` the `lerp`
/// folds to a constant — the 0%-gate anchor that reproduces the resolve's old `SKY_*`.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct SkyLight {
    /// LINEAR upper-hemisphere (sky) `rgb` color.
    pub sky_color: [f32; 3],
    /// LINEAR lower-hemisphere (ground) `rgb` color.
    pub ground_color: [f32; 3],
}

impl SkyLight {
    /// Builds a sky/ambient light from LINEAR hemisphere colors.
    #[inline]
    pub fn new(sky_color: [f32; 3], ground_color: [f32; 3]) -> Self {
        Self { sky_color, ground_color }
    }
}

/// Who drives [`LightingConfig::clusters_enabled`] (P1 — the cold StrategyPolicy
/// substrate). DEFAULT [`Manual`](ClusterSelectMode::Manual) — the 0%-gate: in `Manual`
/// the gate is owner-controlled exactly as before P1 (no behavior change), and the
/// `select_lighting_cull` policy leaves `clusters_enabled` untouched. Only
/// [`Auto`](ClusterSelectMode::Auto) lets the policy drive it from the live light count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClusterSelectMode {
    /// `clusters_enabled` is set by the owner; the policy never writes it. The default,
    /// so a pre-P1 world is byte-identical.
    #[default]
    Manual,
    /// `select_lighting_cull` drives `clusters_enabled` from the banded live point/spot
    /// light count (see [`LightStats`](crate::light_policy::LightStats)).
    Auto,
}

/// The bit position of the resolve's `csm_mode` gate inside light-header word 7
/// (`sky_diffuse.w`, never read by the L0a sky ambient). Mirrors the shader's
/// `load_csm_mode` (`light_table.hlsli`: `(LightBuf[7] >> 2) & 1`); bits 0/1/3 of the
/// same word carry `shadow_mode` / `contact_shadow_mode` / `punctual_shadow_mode`.
pub const CSM_MODE_BIT: u32 = 2;

/// The bit position of the resolve's `punctual_mode` gate inside light-header word 7
/// (`sky_diffuse.w`) — bit 3, immediately above [`CSM_MODE_BIT`] (bit 2). Mirrors the
/// shader's punctual gate (`light_table.hlsli`: `(LightBuf[7] >> 3) & 1`, the "BIT-3
/// INDEPENDENCE PIN"), which selects the spot/point atlas sample vs the analytic
/// fallback INDEPENDENTLY of the CSM bit. Its single production writer is
/// [`sync_punctual_light_gate`](crate::shadow_atlas::sync_punctual_light_gate).
pub const PUNCTUAL_MODE_BIT: u32 = 3;

/// The global lighting config (Decision 3) — a `World`-singleton resource. `exposure`
/// defaults to identity (`1.0`) and `sky_*` default to the resolve's old `SKY_*`
/// constants, so a world that never inserts a non-default config reproduces today's
/// image (the 0%-gate anchor).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct LightingConfig {
    /// Global exposure — the FINAL multiply on accumulated linear radiance. DEFAULT 1.0
    /// (`x * 1.0 == x` exact → 0%-gate byte-identical).
    pub exposure: f32,
    /// Ambient hemisphere diffuse `rgb` (default = the resolve's old `SKY_DIFFUSE`).
    pub sky_diffuse: [f32; 3],
    /// Ambient specular `rgb` (default = the resolve's old `SKY_SPEC`).
    pub sky_spec: [f32; 3],
    /// L1 cluster gate (default `false` → the L0b/L1 flat-table loop path). In
    /// [`ClusterSelectMode::Manual`] (the default) this is owner-controlled; in
    /// [`ClusterSelectMode::Auto`] it is driven by `select_lighting_cull` (P1).
    pub clusters_enabled: bool,
    /// Who owns `clusters_enabled` (P1). DEFAULT [`ClusterSelectMode::Manual`] → the
    /// gate stays owner-controlled and the policy is a no-op (the 0%-gate).
    pub cluster_select: ClusterSelectMode,
    /// The resolve's CSM sample gate — packed into light-header word 7 bit
    /// [`CSM_MODE_BIT`] by [`LightHeaderGpu::new`]. DEFAULT `false` (word 7 stays 0.0 —
    /// the byte-identical 0%-gate).
    ///
    /// # Single-writer / lock-step contract (host plan R4)
    ///
    /// This is DERIVED state, not owner state: when the CSM composition is wired
    /// (`CsmPlugin` + the caster gather), its single writer is
    /// [`sync_csm_light_gate`](crate::csm_caster::sync_csm_light_gate), which keeps the
    /// header gate in lock-step with the depth-pass activation predicate ("a fitted sun
    /// AND live casters exist") to within 1–2 frames. Layout soundness under that lag
    /// does NOT come from timing (review R4-W1): the windowed host boot-transitions the
    /// cascade map to `SHADER_READ_ONLY_OPTIMAL` once at scene boot (closing the
    /// gate-ON-but-never-rendered class) and uploads the CURRENT `ResolvedCsm` UBO
    /// every frame (a DISABLED fit early-outs the resolve) — see the sync system's
    /// layout-soundness note. Set this manually only in hosts that hold the same two
    /// guarantees (the showcase harness discipline).
    pub csm_shadows: bool,
    /// The resolve's punctual (spot/point atlas) sample gate — packed into light-header
    /// word 7 bit [`PUNCTUAL_MODE_BIT`] by [`shadow_gate_word`](Self::shadow_gate_word).
    /// DEFAULT `false` (word 7 bit 3 stays 0 — the byte-identical 0%-gate, INDEPENDENT of
    /// `csm_shadows`).
    ///
    /// # Single-writer / lock-step contract (host punctual rung)
    ///
    /// Like `csm_shadows`, this is DERIVED state: its single production writer is
    /// [`sync_punctual_light_gate`](crate::shadow_atlas::sync_punctual_light_gate), which
    /// keeps the header gate in lock-step with the depth-pass activation predicate ("a
    /// fitted atlas AND live casters exist") to within 1–2 frames. Layout soundness under
    /// that lag rests on the SAME two host guarantees the CSM path documents (boot-transition
    /// the atlas array to `SHADER_READ_ONLY_OPTIMAL` once + upload the CURRENT
    /// `ResolvedShadowAtlas` UBO every frame), not on this system's timing.
    pub punctual_shadows: bool,
}

impl Default for LightingConfig {
    #[inline]
    fn default() -> Self {
        Self {
            exposure: 1.0,
            // == deferred_pbr.hlsl's old SKY_DIFFUSE / SKY_SPEC constants.
            sky_diffuse: [0.10, 0.10, 0.12],
            sky_spec: [0.10, 0.10, 0.12],
            clusters_enabled: false,
            cluster_select: ClusterSelectMode::Manual,
            csm_shadows: false,
            punctual_shadows: false,
        }
    }
}

impl LightingConfig {
    /// Packs the header's word-7 shadow-gate bits from this config: the CSM bit
    /// ([`CSM_MODE_BIT`]) and the punctual bit ([`PUNCTUAL_MODE_BIT`]), each independent.
    /// A default config returns 0 (word 7 == 0.0 — the 0%-gate anchor every pre-R4/pre-punctual
    /// golden pins).
    #[inline]
    pub const fn shadow_gate_word(&self) -> u32 {
        ((self.csm_shadows as u32) << CSM_MODE_BIT)
            | ((self.punctual_shadows as u32) << PUNCTUAL_MODE_BIT)
    }
}

// ---- L1 cluster config (Decision 6) — a World-singleton resource ---------------------

/// The default exp-Z near plane (view-space depth at froxel slice 0). The exp-Z slices run
/// `near * (far/near)^(k/dimZ)`; `near` is the cluster grid's front clamp, NOT the camera
/// near (the marcher is ortho/perspective with `t` in `[0, T_MAX]`). 0.1 is a safe small
/// front bound for the golden ortho/perspective scenes.
pub const CLUSTER_NEAR_DEFAULT: f32 = 0.1;
/// The default exp-Z far plane (view-space depth at froxel slice `dimZ`). Beyond it a pixel
/// clamps to the last slice. 50.0 spans the golden scenes' depth range (T_MAX = 10 plus
/// perspective headroom) with the froxel slices concentrated near the camera (the exp-Z
/// point).
pub const CLUSTER_FAR_DEFAULT: f32 = 50.0;

/// The L1 cluster cull config (Decision 6) — a `World`-singleton resource. Carries the
/// froxel grid dimensions, the per-froxel / flat-list capacities, and the exp-Z near/far
/// the slice math derives its scale/bias from. The defaults reproduce the
/// [`CLUSTER_DIM_*`] / [`MAX_LIGHTS_PER_CLUSTER`] / [`INDEX_LIST_CAP`] constants; a world
/// that never inserts a custom config uses them.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ClusterConfig {
    /// Froxel grid X dimension (default [`CLUSTER_DIM_X`] = 16).
    pub dim_x: u32,
    /// Froxel grid Y dimension (default [`CLUSTER_DIM_Y`] = 9).
    pub dim_y: u32,
    /// Froxel grid Z (exp-Z slice) dimension (default [`CLUSTER_DIM_Z`] = 24).
    pub dim_z: u32,
    /// Per-froxel light-index cap (default [`MAX_LIGHTS_PER_CLUSTER`] = 256).
    pub max_lights_per_cluster: u32,
    /// Flat light-index-list capacity in `u32`s (default [`INDEX_LIST_CAP`] = 16384).
    pub index_list_cap: u32,
    /// Exp-Z near plane (view-space depth at slice 0; default [`CLUSTER_NEAR_DEFAULT`]).
    pub z_near: f32,
    /// Exp-Z far plane (view-space depth at slice `dim_z`; default [`CLUSTER_FAR_DEFAULT`]).
    pub z_far: f32,
}

impl Default for ClusterConfig {
    #[inline]
    fn default() -> Self {
        Self {
            dim_x: CLUSTER_DIM_X,
            dim_y: CLUSTER_DIM_Y,
            dim_z: CLUSTER_DIM_Z,
            max_lights_per_cluster: MAX_LIGHTS_PER_CLUSTER,
            index_list_cap: INDEX_LIST_CAP,
            z_near: CLUSTER_NEAR_DEFAULT,
            z_far: CLUSTER_FAR_DEFAULT,
        }
    }
}

impl ClusterConfig {
    /// The total froxel count (`dim_x * dim_y * dim_z`). At the defaults this equals
    /// [`CLUSTER_COUNT`] (3456).
    #[inline]
    pub const fn cluster_count(&self) -> u32 {
        self.dim_x * self.dim_y * self.dim_z
    }

    /// The exp-Z slice scale: `dim_z / ln(far / near)`. The resolve maps a view-space depth
    /// `view_z` to its froxel slice via `slice = ln(view_z / near) * z_scale` (Decision 6) —
    /// the inverse of `view_z = near * (far/near)^(slice/dim_z)`. The cull pass builds froxel
    /// AABBs from the same `near`/`far`/`scale` so the build and the lookup agree. Returns 0
    /// for a degenerate `far <= near` (clusters are then meaningless; the caller gates L1 off).
    #[inline]
    pub fn z_scale(&self) -> f32 {
        let ratio = self.z_far / self.z_near;
        debug_assert!(
            self.z_far > self.z_near && self.z_near > 0.0,
            "invariant: cluster z_far > z_near > 0"
        );
        if ratio > 1.0 {
            (self.dim_z as f32) / ratio.ln()
        } else {
            0.0
        }
    }

    /// The exp-Z slice bias: `-ln(near) * z_scale`. With [`Self::z_scale`] this gives the
    /// affine `slice = ln(view_z) * z_scale + z_bias` the resolve uses (a single `mad` after
    /// the `log`), equivalent to `ln(view_z / near) * z_scale` but pre-folding the `ln(near)`.
    #[inline]
    pub fn z_bias(&self) -> f32 {
        -self.z_near.ln() * self.z_scale()
    }

    /// Packs the three small grid dims into one `u32` (`dim_x | dim_y<<8 | dim_z<<16`) for
    /// the header's `cluster_params` lane. The dims are ≤ 255 each (debug-asserted), so the
    /// pack is lossless and the shader unpacks with the inverse mask/shift.
    #[inline]
    pub fn packed_dims(&self) -> u32 {
        debug_assert!(
            self.dim_x <= 0xFF && self.dim_y <= 0xFF && self.dim_z <= 0xFF,
            "invariant: cluster dims must each fit in 8 bits for the header pack"
        );
        self.dim_x | (self.dim_y << 8) | (self.dim_z << 16)
    }
}

// ---- constructors --------------------------------------------------------------------

/// Normalizes a 3-vector; returns it unchanged if its length is ~0 (a degenerate
/// direction the caller is responsible for — debug builds flag it).
#[inline]
fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let len_sq = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    debug_assert!(len_sq > 1e-12, "invariant: light direction must be non-degenerate");
    // Release safety net: gate on the SAME 1e-12 threshold as the assert. Without it,
    // `0.0 < len_sq <= 1e-12` would compute `1.0 / len_sq.sqrt()` and explode the vector;
    // a near-degenerate direction is returned unchanged (the assert flags it in debug).
    if len_sq <= 1e-12 {
        return v;
    }
    let inv = 1.0 / len_sq.sqrt();
    [v[0] * inv, v[1] * inv, v[2] * inv]
}

impl DirectionalLight {
    /// Builds a directional light. `direction` is the world direction TO the light
    /// (normalized here); `color` is LINEAR; `illuminance` is in lux.
    #[inline]
    pub fn new(direction: [f32; 3], color: [f32; 3], illuminance: f32) -> Self {
        Self { direction: normalize3(direction), color, illuminance }
    }
}

impl PointLight {
    /// Builds a point light. `position` is world-space; `color` is LINEAR; `power` is the
    /// luminous power `Φ` (lumens); `range` is the cull-sphere radius.
    #[inline]
    pub fn new(position: [f32; 3], color: [f32; 3], power: f32, range: f32) -> Self {
        Self { position, color, power, range }
    }
}

/// The maximum `cos(outer)` allowed (Decision 2 trade-off): bounds `I = Φ/(2π(1−cos))`
/// as the cone narrows to a pencil beam (`1 − cos(outer)` → 0).
pub const SPOT_COS_OUTER_MAX: f32 = 0.9999;

impl SpotLight {
    /// Builds a spot light. Clamps `cos(outer) ≤ `[`SPOT_COS_OUTER_MAX`] (Decision 2) so
    /// the baked `I = Φ/(2π(1−cos(outer)))` stays bounded. `direction` (the spot axis) is
    /// normalized here; `color` is LINEAR; `power` is `Φ` (lumens).
    #[inline]
    pub fn new(
        position: [f32; 3],
        direction: [f32; 3],
        color: [f32; 3],
        power: f32,
        range: f32,
        inner_deg: f32,
        outer_deg: f32,
    ) -> Self {
        let cos_outer = (outer_deg.to_radians()).cos();
        debug_assert!(
            cos_outer <= SPOT_COS_OUTER_MAX,
            "invariant: spot cos(outer) must be <= SPOT_COS_OUTER_MAX (Decision 2 bound)"
        );
        // The clamp keeps the runtime bake bounded even if the assert is compiled out.
        let outer_deg = if cos_outer > SPOT_COS_OUTER_MAX {
            SPOT_COS_OUTER_MAX.acos().to_degrees()
        } else {
            outer_deg
        };
        Self {
            position,
            direction: normalize3(direction),
            color,
            power,
            range,
            inner_deg,
            outer_deg,
        }
    }
}

// ---- canonical CPU → GpuLight conversions --------------------------------------------

/// Packs two cosines into the `f16 | f16` bit pattern carried in [`GpuLight::color_cone`]
/// `.w` (`cos_inner` in the low half, `cos_outer` in the high half). The shader's
/// `unpack_cones` is the inverse.
#[inline]
fn pack_cones(cos_inner: f32, cos_outer: f32) -> f32 {
    let lo = f16_from_f32(cos_inner) as u32;
    let hi = f16_from_f32(cos_outer) as u32;
    f32::from_bits(lo | (hi << 16))
}

/// IEEE-754 binary32 → binary16 (round-to-nearest-even), for the spot cone pack. The
/// cosines live in `[-1, 1]`, well inside the `f16` normal range, so no special-case
/// for subnormals/overflow is needed beyond the standard rounding.
#[inline]
fn f16_from_f32(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x007f_ffff;
    if exp == 0xff {
        // inf / NaN — preserve NaN-ness; cones are finite so this is defensive only.
        let m = if mant != 0 { 0x0200 } else { 0 };
        return sign | 0x7c00 | m;
    }
    let new_exp = exp - 127 + 15;
    if new_exp >= 0x1f {
        return sign | 0x7c00; // overflow → inf
    }
    if new_exp <= 0 {
        if new_exp < -10 {
            return sign; // underflow → signed zero
        }
        // Subnormal half: shift the implicit-1 mantissa into the 10-bit field, then
        // round-to-nearest on the bit just below the new LSB.
        let full_mant = mant | 0x0080_0000;
        let shift = (14 - new_exp) as u32; // = (1 - new_exp) + 13
        let m = (full_mant >> shift) as u16;
        let round_bit = ((full_mant >> (shift - 1)) & 1) as u16;
        return sign | (m + round_bit);
    }
    let half = sign | ((new_exp as u16) << 10) | ((mant >> 13) as u16);
    // round-to-nearest-even on the 13 discarded mantissa bits.
    let round = (mant >> 12) & 1;
    let sticky = mant & 0x0fff;
    if round == 1 && (sticky != 0 || (half & 1) == 1) {
        half + 1
    } else {
        half
    }
}

impl GpuLight {
    /// Folds a [`DirectionalLight`] into a [`GpuLight`]: `dir_kind.xyz` = direction,
    /// `.w` = [`LIGHT_KIND_DIRECTIONAL`]; `pos_range.w` = `+inf` (no cull sphere);
    /// `color_cone.rgb` = `color × illuminance` (LINEAR irradiance premultiply).
    #[inline]
    pub fn from_directional(l: &DirectionalLight) -> Self {
        let d = normalize3(l.direction);
        Self {
            dir_kind: [d[0], d[1], d[2], f32::from_bits(LIGHT_KIND_DIRECTIONAL)],
            pos_range: [0.0, 0.0, 0.0, f32::INFINITY],
            color_cone: [
                l.color[0] * l.illuminance,
                l.color[1] * l.illuminance,
                l.color[2] * l.illuminance,
                0.0,
            ],
        }
    }

    /// Folds a [`PointLight`] into a [`GpuLight`], baking `I = Φ / (4π)` (Decision 2,
    /// the point-source normalization) into `color_cone.rgb`. The L0b resolve consumes
    /// `pos_range` + the baked intensity.
    #[inline]
    pub fn from_point(l: &PointLight) -> Self {
        let intensity = l.power / (4.0 * PI);
        Self {
            dir_kind: [0.0, 0.0, 0.0, f32::from_bits(LIGHT_KIND_POINT)],
            pos_range: [l.position[0], l.position[1], l.position[2], l.range],
            color_cone: [
                l.color[0] * intensity,
                l.color[1] * intensity,
                l.color[2] * intensity,
                0.0,
            ],
        }
    }

    /// Folds a [`SpotLight`] into a [`GpuLight`], baking `I = Φ / (2π(1 − cos(outer)))`
    /// (Decision 2, the reflector model) into `color_cone.rgb` and packing the cone
    /// cosines (`cos_inner`, `cos_outer`) into `color_cone.w`. `dir_kind.xyz` carries the
    /// spot axis. The L0b resolve consumes all three lanes.
    #[inline]
    pub fn from_spot(l: &SpotLight) -> Self {
        let cos_inner = l.inner_deg.to_radians().cos();
        let cos_outer = l.outer_deg.to_radians().cos().min(SPOT_COS_OUTER_MAX);
        let denom = 2.0 * PI * (1.0 - cos_outer);
        let intensity = l.power / denom;
        let d = normalize3(l.direction);
        Self {
            dir_kind: [d[0], d[1], d[2], f32::from_bits(LIGHT_KIND_SPOT)],
            pos_range: [l.position[0], l.position[1], l.position[2], l.range],
            color_cone: [
                l.color[0] * intensity,
                l.color[1] * intensity,
                l.color[2] * intensity,
                pack_cones(cos_inner, cos_outer),
            ],
        }
    }

    /// Folds a [`SkyLight`] into a [`GpuLight`]: `dir_kind.w` = [`LIGHT_KIND_SKY`];
    /// `color_cone.rgb` = `sky_color` (the upper-hemisphere term); `pos_range.xyz` =
    /// `ground_color` (the lower-hemisphere term). The L0a resolve computes
    /// `lerp(ground, sky, dot(N, up) * 0.5 + 0.5)` for the diffuse ambient + the analytic
    /// env-BRDF specular against `sky_color`, both × AO. No `P` dependency (L0a).
    #[inline]
    pub fn from_sky(l: &SkyLight) -> Self {
        Self {
            dir_kind: [0.0, 0.0, 0.0, f32::from_bits(LIGHT_KIND_SKY)],
            pos_range: [l.ground_color[0], l.ground_color[1], l.ground_color[2], 0.0],
            color_cone: [l.sky_color[0], l.sky_color[1], l.sky_color[2], 0.0],
        }
    }
}

impl LightHeaderGpu {
    /// Builds the table header (Decision 3). `l0a_count` is the no-`P` front block
    /// (directionals + sky) and `point_spot_count` is the L0b block, so the array is
    /// laid out `[no-P front block || point/spot]` and
    /// `light_count = l0a_count + point_spot_count`. `exposure` + `sky_*` come from
    /// `cfg`. The L1 `cluster_params` are zero in L0 (`clusters_enabled` reflects `cfg`,
    /// but the dims stay 0 until L1 mints the grid). Word 7 (`sky_diffuse.w`) carries
    /// the shadow-gate bits ([`LightingConfig::shadow_gate_word`] — CSM bit
    /// [`CSM_MODE_BIT`]; 0 for a default config, the 0%-gate).
    #[inline]
    pub fn new(l0a_count: u32, point_spot_count: u32, cfg: &LightingConfig) -> Self {
        debug_assert!(cfg.exposure > 0.0 && cfg.exposure.is_finite(), "invariant: exposure > 0");
        let light_count = l0a_count + point_spot_count;
        debug_assert!(light_count <= MAX_LIGHTS, "invariant: light_count <= MAX_LIGHTS");
        Self {
            counts_exposure: [
                f32::from_bits(light_count),
                cfg.exposure,
                f32::from_bits(l0a_count),
                f32::from_bits(point_spot_count),
            ],
            sky_diffuse: [
                cfg.sky_diffuse[0],
                cfg.sky_diffuse[1],
                cfg.sky_diffuse[2],
                f32::from_bits(cfg.shadow_gate_word()),
            ],
            sky_spec: [cfg.sky_spec[0], cfg.sky_spec[1], cfg.sky_spec[2], 0.0],
            // L0: clusters off (dims zero); `clusters_enabled` is reported for the resolve
            // gate but the L1 grid is not minted until the L1 rung.
            cluster_params: [
                0.0,
                0.0,
                0.0,
                f32::from_bits(u32::from(cfg.clusters_enabled)),
            ],
        }
    }

    /// Builds the L1 header: identical to [`Self::new`] but the `cluster_params` lane carries
    /// the exp-Z froxel-lookup factors instead of zeros. Lane 3 is
    /// `[z_scale, z_bias, bitcast(packed_dims), bitcast(clusters_enabled)]` (Decision 6):
    /// - `z_scale` / `z_bias` — the affine exp-Z slice map `slice = ln(view_z) * z_scale +
    ///   z_bias` the resolve applies (the cull builds froxel AABBs from the same near/far);
    /// - `packed_dims` — `dim_x | dim_y<<8 | dim_z<<16` (the resolve unpacks to map a pixel
    ///   to its `(x, y)` tile + clamp the slice);
    /// - `clusters_enabled` — `1` gates the resolve onto the cluster path, `0` ⇒ the flat
    ///   L0b loop (the L1 0%-gate). When `cfg.clusters_enabled` is `false` the lane stays all
    ///   zero (byte-identical to [`Self::new`]'s L0 header — the 0%-gate anchor).
    #[inline]
    pub fn new_clustered(
        l0a_count: u32,
        point_spot_count: u32,
        cfg: &LightingConfig,
        cluster: &ClusterConfig,
    ) -> Self {
        let mut header = Self::new(l0a_count, point_spot_count, cfg);
        if cfg.clusters_enabled {
            header.cluster_params = [
                cluster.z_scale(),
                cluster.z_bias(),
                f32::from_bits(cluster.packed_dims()),
                f32::from_bits(1),
            ];
        }
        header
    }

    /// The `light_count` field (bit-cast back from `counts_exposure.x`).
    #[inline]
    pub fn light_count(&self) -> u32 {
        self.counts_exposure[0].to_bits()
    }

    /// The `l0a_count` field — the no-`P` front block (directionals + sky), bit-cast
    /// back from `counts_exposure.z`. The L0a resolve loops `[0..l0a_count)`.
    #[inline]
    pub fn l0a_count(&self) -> u32 {
        self.counts_exposure[2].to_bits()
    }

    /// The `point_spot_count` field (bit-cast back from `counts_exposure.w`).
    #[inline]
    pub fn point_spot_count(&self) -> u32 {
        self.counts_exposure[3].to_bits()
    }

    /// Whether the resolve's CSM sample gate is armed (word 7 bit [`CSM_MODE_BIT`],
    /// bit-cast back from `sky_diffuse.w`) — the host mirror of the shader's
    /// `load_csm_mode`.
    #[inline]
    pub fn csm_mode(&self) -> bool {
        (self.sky_diffuse[3].to_bits() >> CSM_MODE_BIT) & 1 != 0
    }

    /// Whether the resolve's punctual (spot/point atlas) sample gate is armed (word 7 bit
    /// [`PUNCTUAL_MODE_BIT`], bit-cast back from `sky_diffuse.w`) — the host mirror of the
    /// shader's punctual gate. Independent of [`csm_mode`](Self::csm_mode) (the BIT-3
    /// INDEPENDENCE PIN).
    #[inline]
    pub fn punctual_mode(&self) -> bool {
        (self.sky_diffuse[3].to_bits() >> PUNCTUAL_MODE_BIT) & 1 != 0
    }

    /// Whether the L1 cluster path is enabled (`cluster_params.w` bit-cast `!= 0`). `false`
    /// ⇒ the resolve loops the flat table (the L1 0%-gate == L0b).
    #[inline]
    pub fn clusters_enabled(&self) -> bool {
        self.cluster_params[3].to_bits() != 0
    }

    /// The L1 exp-Z slice scale (`cluster_params.x`). Meaningful only when
    /// [`Self::clusters_enabled`]; zero otherwise.
    #[inline]
    pub fn cluster_z_scale(&self) -> f32 {
        self.cluster_params[0]
    }

    /// The L1 exp-Z slice bias (`cluster_params.y`). Meaningful only when
    /// [`Self::clusters_enabled`]; zero otherwise.
    #[inline]
    pub fn cluster_z_bias(&self) -> f32 {
        self.cluster_params[1]
    }

    /// The L1 packed froxel dims `dim_x | dim_y<<8 | dim_z<<16` (`cluster_params.z` bit-cast).
    /// Zero when clusters are disabled.
    #[inline]
    pub fn cluster_packed_dims(&self) -> u32 {
        self.cluster_params[2].to_bits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f32::consts::PI;

    /// Per-component tolerance for f32 equality where a transcendental (cos/acos) is
    /// involved.
    const EPS: f32 = 1e-5;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() <= EPS * (1.0 + a.abs().max(b.abs()))
    }

    #[test]
    fn pod_sizes_match_the_shader_pins() {
        // The const-asserts already make a drift a build error; these mirror them as a
        // runtime regression so the fingerprint is visible in the test report.
        assert_eq!(core::mem::size_of::<GpuLight>(), 48);
        assert_eq!(core::mem::align_of::<GpuLight>(), 16);
        assert_eq!(core::mem::size_of::<LightHeaderGpu>(), 64);
        assert_eq!(core::mem::align_of::<LightHeaderGpu>(), 16);
        assert_eq!(core::mem::size_of::<ClusterCell>(), 8);
        assert_eq!(GPU_LIGHT_WORDS, 12);
        assert_eq!(LIGHT_HEADER_WORDS, 16);
        assert_eq!(LIGHT_HEADER_BASE_WORDS, 16);
    }

    #[test]
    fn default_config_is_the_zero_gate_anchor() {
        let cfg = LightingConfig::default();
        assert_eq!(cfg.exposure, 1.0);
        // The old resolve constants (deferred_pbr.hlsl SKY_DIFFUSE / SKY_SPEC).
        assert_eq!(cfg.sky_diffuse, [0.10, 0.10, 0.12]);
        assert_eq!(cfg.sky_spec, [0.10, 0.10, 0.12]);
        assert!(!cfg.clusters_enabled);
        // P1: the policy substrate defaults to Manual so the gate stays owner-controlled.
        assert_eq!(cfg.cluster_select, ClusterSelectMode::Manual);
        // The punctual gate defaults OFF (the byte-identical 0%-gate).
        assert!(!cfg.punctual_shadows);
    }

    #[test]
    fn shadow_gate_word_bits_are_independent() {
        // A default config packs a zero gate word (word-7 == 0 — the 0%-gate anchor).
        assert_eq!(LightingConfig::default().shadow_gate_word(), 0);

        // The CSM bit alone (bit 2).
        let csm = LightingConfig { csm_shadows: true, ..LightingConfig::default() };
        assert_eq!(csm.shadow_gate_word(), 1 << CSM_MODE_BIT);
        assert_eq!(csm.shadow_gate_word() & (1 << PUNCTUAL_MODE_BIT), 0, "csm must not touch bit 3");

        // The punctual bit alone (bit 3), INDEPENDENT of the CSM bit.
        let punc = LightingConfig { punctual_shadows: true, ..LightingConfig::default() };
        assert_eq!(punc.shadow_gate_word(), 1 << PUNCTUAL_MODE_BIT);
        assert_eq!(punc.shadow_gate_word() & (1 << CSM_MODE_BIT), 0, "punctual must not touch bit 2");

        // Both set — the two independent bits OR together.
        let both = LightingConfig {
            csm_shadows: true,
            punctual_shadows: true,
            ..LightingConfig::default()
        };
        assert_eq!(both.shadow_gate_word(), (1 << CSM_MODE_BIT) | (1 << PUNCTUAL_MODE_BIT));
    }

    #[test]
    fn from_directional_premultiplies_color_by_illuminance() {
        let l = DirectionalLight::new([0.0, 0.0, 1.0], [1.0, 0.5, 0.25], 2.0);
        let g = GpuLight::from_directional(&l);
        assert_eq!(g.dir_kind[3].to_bits(), LIGHT_KIND_DIRECTIONAL);
        // direction normalized (already unit → unchanged).
        assert!(approx(g.dir_kind[0], 0.0) && approx(g.dir_kind[2], 1.0));
        // color × illuminance.
        assert!(approx(g.color_cone[0], 2.0));
        assert!(approx(g.color_cone[1], 1.0));
        assert!(approx(g.color_cone[2], 0.5));
        assert_eq!(g.pos_range[3], f32::INFINITY);
    }

    #[test]
    fn from_directional_default_matches_the_old_constant() {
        // The 0%-gate directional: +Z, white, illuminance 1.0 reproduces the old
        // LIGHT_DIR = (0,0,1) / LIGHT_COLOR = (1,1,1).
        let g = GpuLight::from_directional(&DirectionalLight::new([0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 1.0));
        assert_eq!([g.dir_kind[0], g.dir_kind[1], g.dir_kind[2]], [0.0, 0.0, 1.0]);
        assert_eq!([g.color_cone[0], g.color_cone[1], g.color_cone[2]], [1.0, 1.0, 1.0]);
    }

    #[test]
    fn from_sky_maps_sky_to_color_lane_ground_to_pos_lane() {
        let l = SkyLight::new([0.2, 0.3, 0.4], [0.05, 0.06, 0.07]);
        let g = GpuLight::from_sky(&l);
        assert_eq!(g.dir_kind[3].to_bits(), LIGHT_KIND_SKY);
        assert_eq!([g.color_cone[0], g.color_cone[1], g.color_cone[2]], [0.2, 0.3, 0.4]);
        assert_eq!([g.pos_range[0], g.pos_range[1], g.pos_range[2]], [0.05, 0.06, 0.07]);
    }

    #[test]
    fn from_point_bakes_phi_over_4pi() {
        let phi = 100.0_f32;
        let l = PointLight::new([1.0, 2.0, 3.0], [1.0, 1.0, 1.0], phi, 10.0);
        let g = GpuLight::from_point(&l);
        assert_eq!(g.dir_kind[3].to_bits(), LIGHT_KIND_POINT);
        let i = phi / (4.0 * PI);
        assert!(approx(g.color_cone[0], i));
        assert_eq!([g.pos_range[0], g.pos_range[1], g.pos_range[2]], [1.0, 2.0, 3.0]);
        assert_eq!(g.pos_range[3], 10.0);
    }

    #[test]
    fn from_spot_bakes_phi_over_2pi_one_minus_cos_outer() {
        let phi = 200.0_f32;
        let outer = 30.0_f32;
        let l = SpotLight::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0], phi, 5.0, 15.0, outer);
        let g = GpuLight::from_spot(&l);
        assert_eq!(g.dir_kind[3].to_bits(), LIGHT_KIND_SPOT);
        let cos_outer = outer.to_radians().cos();
        let i = phi / (2.0 * PI * (1.0 - cos_outer));
        assert!(approx(g.color_cone[0], i), "expected I={i}, got {}", g.color_cone[0]);
    }

    #[test]
    fn from_spot_clamps_a_pencil_beam_cone_to_a_finite_intensity() {
        // A 0.1-degree outer cone has cos(outer) ~ 0.99999847 > SPOT_COS_OUTER_MAX, which
        // would make I = Φ/(2π(1−cos)) blow up. `from_spot` clamps cos(outer) ≤
        // SPOT_COS_OUTER_MAX (the runtime safety net, separate from the constructor's
        // authoring `debug_assert!`), so the baked intensity stays finite. Build the spot
        // via a struct literal to exercise the bake's OWN clamp (not the `new` assert).
        let l = SpotLight {
            position: [0.0, 0.0, 0.0],
            direction: [0.0, 0.0, 1.0],
            color: [1.0, 1.0, 1.0],
            power: 100.0,
            range: 5.0,
            inner_deg: 0.05,
            outer_deg: 0.1, // cos ~ 0.99999847 > SPOT_COS_OUTER_MAX
        };
        let g = GpuLight::from_spot(&l);
        assert!(g.color_cone[0].is_finite(), "baked intensity must stay finite under the clamp");
        // The clamp bounds I to at most Φ / (2π(1 − SPOT_COS_OUTER_MAX)).
        let i_max = 100.0 / (2.0 * PI * (1.0 - SPOT_COS_OUTER_MAX));
        assert!(g.color_cone[0] <= i_max * (1.0 + EPS), "I must be bounded by the clamp");
    }

    // Gated to debug: the contract under test is a `debug_assert!`, which is compiled out under
    // `--release`, so `#[should_panic]` would (correctly) not fire there. The release safety net
    // (the bake clamp) is covered by `spot_constructor_clamps_pencil_beam_intensity` above.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "cos(outer)")]
    fn spot_constructor_debug_asserts_a_pencil_beam() {
        // The authoring contract (Decision 2): a pencil-beam cone trips the constructor's
        // `debug_assert!` in debug builds (a likely authoring mistake); the bake's clamp is
        // the release safety net (covered by the test above).
        let _ = SpotLight::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 1.0], 100.0, 5.0, 0.05, 0.1);
    }

    #[test]
    fn header_carries_exposure_and_split_counts() {
        let cfg = LightingConfig { exposure: 2.5, ..Default::default() };
        let h = LightHeaderGpu::new(3, 4, &cfg);
        assert_eq!(h.light_count(), 7);
        assert_eq!(h.l0a_count(), 3);
        assert_eq!(h.point_spot_count(), 4);
        assert_eq!(h.counts_exposure[1], 2.5);
        assert_eq!(h.sky_diffuse[0], cfg.sky_diffuse[0]);
        // L0: cluster dims are zero.
        assert_eq!(h.cluster_params[0], 0.0);
    }

    #[test]
    fn cluster_index_linearizes_z_innermost_and_round_trips() {
        // (y * dimX + x) * dimZ + z — Z is the fastest-varying (innermost) index, so a
        // contiguous z-walk inside one (x,y) tile produces consecutive indices.
        assert_eq!(cluster_index(0, 0, 0), 0);
        assert_eq!(cluster_index(0, 0, 1), 1);
        assert_eq!(cluster_index(0, 0, CLUSTER_DIM_Z - 1), CLUSTER_DIM_Z - 1);
        // x increments by dimZ (one full slice column); y by dimX*dimZ.
        assert_eq!(cluster_index(1, 0, 0), CLUSTER_DIM_Z);
        assert_eq!(cluster_index(0, 1, 0), CLUSTER_DIM_X * CLUSTER_DIM_Z);
        // The last froxel maps to CLUSTER_COUNT - 1 and every index is unique + in range.
        assert_eq!(
            cluster_index(CLUSTER_DIM_X - 1, CLUSTER_DIM_Y - 1, CLUSTER_DIM_Z - 1),
            CLUSTER_COUNT - 1
        );
        let mut seen = vec![false; CLUSTER_COUNT as usize];
        for y in 0..CLUSTER_DIM_Y {
            for x in 0..CLUSTER_DIM_X {
                for z in 0..CLUSTER_DIM_Z {
                    let idx = cluster_index(x, y, z) as usize;
                    assert!(!seen[idx], "cluster_index collision at ({x},{y},{z})");
                    seen[idx] = true;
                }
            }
        }
        assert!(seen.iter().all(|&s| s), "cluster_index is not a bijection onto [0, COUNT)");
    }

    #[test]
    fn exp_z_factors_invert_the_slice_distribution() {
        // The affine map `slice = ln(view_z) * z_scale + z_bias` must invert the exp-Z slice
        // distribution `view_z(k) = near * (far/near)^(k/dimZ)` to the SAME k (round-trip).
        let cfg = ClusterConfig::default();
        let scale = cfg.z_scale();
        let bias = cfg.z_bias();
        for k in 0..=cfg.dim_z {
            let view_z = cfg.z_near * (cfg.z_far / cfg.z_near).powf(k as f32 / cfg.dim_z as f32);
            let slice = view_z.ln() * scale + bias;
            assert!(
                (slice - k as f32).abs() < 1e-3,
                "exp-Z slice {k}: view_z={view_z} mapped back to slice {slice}"
            );
        }
        // The boundaries: view_z == near maps to slice 0, view_z == far maps to slice dimZ.
        assert!((cfg.z_near.ln() * scale + bias).abs() < 1e-4);
        assert!((cfg.z_far.ln() * scale + bias - cfg.dim_z as f32).abs() < 1e-3);
    }

    #[test]
    fn cluster_config_default_matches_the_constants() {
        let c = ClusterConfig::default();
        assert_eq!(c.dim_x, CLUSTER_DIM_X);
        assert_eq!(c.dim_y, CLUSTER_DIM_Y);
        assert_eq!(c.dim_z, CLUSTER_DIM_Z);
        assert_eq!(c.cluster_count(), CLUSTER_COUNT);
        assert_eq!(c.max_lights_per_cluster, MAX_LIGHTS_PER_CLUSTER);
        assert_eq!(c.index_list_cap, INDEX_LIST_CAP);
        assert_eq!(c.packed_dims(), CLUSTER_DIM_X | (CLUSTER_DIM_Y << 8) | (CLUSTER_DIM_Z << 16));
    }

    #[test]
    fn clustered_header_off_is_byte_identical_to_l0_header() {
        // clusters_enabled == false ⇒ new_clustered's cluster_params stays all-zero, so the
        // header is byte-identical to the plain L0 header (the L1 0%-gate anchor).
        let cfg = LightingConfig::default(); // clusters_enabled == false
        let cluster = ClusterConfig::default();
        let l0 = LightHeaderGpu::new(2, 1, &cfg);
        let l1 = LightHeaderGpu::new_clustered(2, 1, &cfg, &cluster);
        assert_eq!(l0, l1);
        assert!(!l1.clusters_enabled());
        assert_eq!(l1.cluster_params, [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn clustered_header_on_carries_factors_and_dims() {
        let cfg = LightingConfig { clusters_enabled: true, ..Default::default() };
        let cluster = ClusterConfig::default();
        let h = LightHeaderGpu::new_clustered(1, 0, &cfg, &cluster);
        assert!(h.clusters_enabled());
        assert_eq!(h.cluster_z_scale(), cluster.z_scale());
        assert_eq!(h.cluster_z_bias(), cluster.z_bias());
        assert_eq!(h.cluster_packed_dims(), cluster.packed_dims());
        // Unpack the dims back out of the packed word.
        let d = h.cluster_packed_dims();
        assert_eq!(d & 0xFF, CLUSTER_DIM_X);
        assert_eq!((d >> 8) & 0xFF, CLUSTER_DIM_Y);
        assert_eq!((d >> 16) & 0xFF, CLUSTER_DIM_Z);
    }

    #[test]
    fn pack_unpack_cones_round_trips_within_f16_precision() {
        // The cone cosines live in [-1, 1]; f16 has ~3 decimal digits there.
        for &(ci, co) in &[(1.0_f32, 0.5_f32), (0.866, 0.707), (0.0, -1.0)] {
            let packed = pack_cones(ci, co);
            let bits = packed.to_bits();
            let lo = half_to_f32((bits & 0xFFFF) as u16);
            let hi = half_to_f32(((bits >> 16) & 0xFFFF) as u16);
            assert!((lo - ci).abs() < 1e-3, "cos_inner {ci} round-trip {lo}");
            assert!((hi - co).abs() < 1e-3, "cos_outer {co} round-trip {hi}");
        }
    }

    /// IEEE-754 binary16 → binary32 (test-only inverse of `f16_from_f32`, for the cone
    /// round-trip assertion).
    fn half_to_f32(h: u16) -> f32 {
        let sign = ((h >> 15) & 1) as u32;
        let exp = ((h >> 10) & 0x1f) as u32;
        let mant = (h & 0x3ff) as u32;
        let bits = if exp == 0 {
            if mant == 0 {
                sign << 31
            } else {
                // subnormal → normalize.
                let mut e = -1i32;
                let mut m = mant;
                loop {
                    e += 1;
                    m <<= 1;
                    if m & 0x400 != 0 {
                        break;
                    }
                }
                let new_exp = (127 - 15 - e) as u32;
                (sign << 31) | (new_exp << 23) | ((m & 0x3ff) << 13)
            }
        } else if exp == 0x1f {
            (sign << 31) | 0x7f80_0000 | (mant << 13)
        } else {
            let new_exp = exp + (127 - 15);
            (sign << 31) | (new_exp << 23) | (mant << 13)
        };
        f32::from_bits(bits)
    }
}
