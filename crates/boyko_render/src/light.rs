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

use boyko_ecs::ecs::core::system::{Res, ResMut};
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

/// VB-P1e H2 (design D6): the hierarchical cull's groupshared coarse mask, `HIER_MASK_WORDS`
/// 32-bit words (`cluster_cull.hlsl`'s `#define HIER_MASK_WORDS 32u`), one bit per light-table
/// row relative to `l0a_count`.
pub const HIER_MASK_WORDS: u32 = 32;

/// D6's load-bearing EQUALITY (not `<=`): the hier mask must cover the light table's point/spot
/// capacity EXACTLY, because D7's single clamp `ps_n <= HIER_MASK_WORDS * 32` bounds BOTH the
/// groupshared mask WRITE and the device table READ. Under `<=` (say a wider `MAX_LIGHTS` against
/// the same 32 mask words) `ps_room` would exceed the table's row count and that one clamp would
/// no longer bound the device read. A future `MAX_LIGHTS` change is therefore a compile error
/// here, forcing a shader edit (widen `HIER_MASK_WORDS`) and a `.spv` re-bake — the intended price
/// of one clamp covering two bounds (`docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` D6).
const _: () = assert!(
    MAX_LIGHTS == HIER_MASK_WORDS * 32,
    "invariant: the hier mask covers the table EXACTLY — one clamp bounds both the groupshared \
     write and the device read"
);
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
/// - `dir_kind` (off 0): `xyz` = the light's world axis (DIRECTIONAL: the direction
///   TO the light, `dot(n, dir)`; SPOT: the SHINE axis, `dot(-l, dir)`) | unused
///   (POINT); `w` = bit-cast `u32` kind tag ([`LIGHT_KIND_DIRECTIONAL`] etc.).
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
    /// `xyz` = light world axis (DIRECTIONAL: to-light; SPOT: shine axis), `w` = bit-cast kind tag.
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
/// - `sky_spec` (off 32): ambient specular `rgb` (replaces `SKY_SPEC`), `w` = Render
///   P7-Q2's `ssao_mode` gate (bit-cast `u32`, `0`/`1` — the resolve's SSAO-combine
///   switch; see [`LightingConfig::ssao_mode`]).
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
    /// Ambient specular `rgb`, `w` = the Render P7-Q2 `ssao_mode` gate (bit-cast `u32`).
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
    /// World spot axis — the SHINE direction the cone points along (NOT "to-light";
    /// the resolve consumes it as `dot(-l, dir)`). Normalized host-side. When a
    /// `GlobalTransform` is present, `light_reconcile` overwrites this with the
    /// transform's world `-Z`, so aim the spot via the pose (`look_at`), not this field.
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

// ---- Light-header word 7 bit budget (the shared shadow/GI-gate + output-stage word) --
//
// Word 7 (`sky_diffuse.w` — the header lane NEVER read by the L0a sky ambient, which
// only consumes `sky_diffuse.rgb`, words 4..6) is ONE spare std430 word repurposed to
// carry every resolve-side boolean gate / small enum this crate has added since P6 R1,
// rather than growing `LIGHT_HEADER_WORDS` (which would shift `LIGHT_HEADER_BASE` and
// re-encode every golden). Each sub-field below is independently masked (the "BIT-N
// INDEPENDENCE PIN" proven per-field in `light_table.hlsli`), so setting one never
// perturbs another, and every sub-field defaults to `0` on a config that never touches
// it — the shared 0%-gate anchor every pre-existing golden pins. THIS is the
// authoritative map; the per-field detail (exact mask/shift, 0%-gate proof) lives at
// each site named below.
//
//   bits     field                   owning Rust file
//   0        shadow_mode             goldens.rs (`GoldenLightHeader`) — predates this
//                                     type; no `LightingConfig` field owns it
//   1        contact_shadow_mode     goldens.rs (`GoldenLightHeader`) — ditto
//   2        csm_mode                this file: CSM_MODE_BIT / LightingConfig::csm_shadows
//   3        punctual_shadow_mode    this file: PUNCTUAL_MODE_BIT / LightingConfig::punctual_shadows
//   4        ddgi_mode               this file: DDGI_MODE_BIT / LightingConfig::ddgi_indirect
//   5..6     vb_sdf_mesh (SV0)       this file: VB_SDF_MESH_MODE_SHIFT/_MASK /
//                                     LightingConfig::vb_sdf_mesh_shadow (bit 5) +
//                                     ::vb_sdf_mesh_ao (bit 6) — two INDEPENDENT terms
//   7        (free)                  —
//   8..11    tonemap operator        this file: TONEMAP_MODE_SHIFT/_MASK / LightingConfig::tonemapper
//   12..19   terminator softening    this file: TERMINATOR_SOFT_SHIFT/_MASK / LightingConfig::terminator_softening
//   20..31   (free)                  —
//
// Shader-side decode: `light_table.hlsli`'s `load_shadow_mode` / `load_contact_shadow_mode`
// / `load_csm_mode` / `load_punctual_shadow_mode` / `load_ddgi_mode` / `load_tonemap_mode`
// / `load_terminator_softening` cluster. Host packing of bits 0/1: `goldens.rs`'s
// `GoldenLightHeader` (the P6 R1 / Shadow-Phase-3 literals, near its other word-7 writers).

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

/// The bit position of the resolve's DDGI (SDF diffuse GI) gate inside light-header word 7
/// (`sky_diffuse.w`) — bit 4, immediately above [`PUNCTUAL_MODE_BIT`] (bit 3). Bits 2/3
/// are the CSM / punctual gates; bit 4 is free. Mirrors the shader's DDGI gate
/// (`light_table.hlsli`: `(LightBuf[7] >> 4) & 1`), which arms the gated probe-irradiance
/// injection in the resolve INDEPENDENTLY of the CSM / punctual bits. Its single
/// production writer is
/// [`sync_ddgi_light_gate`](crate::ddgi_config::sync_ddgi_light_gate). DEFAULT `false` on
/// every pre-SDFDDGI scene ⇒ word 7 bit 4 stays 0, the byte-identical 0%-gate.
pub const DDGI_MODE_BIT: u32 = 4;

/// Word-7 sub-field for VB-SV0's SDF-on-mesh gate: bits 5..6 (2 bits). Bit 5 arms the SDF
/// soft shadow, bit 6 the 5-tap contact AO, and they arm INDEPENDENTLY — SV0 is two terms,
/// not one, and giving them separate bits is what lets each half's arming gate be shown to
/// move pixels ON ITS OWN instead of hiding behind the other. Above the shadow/GI gate bits
/// (0..4) and below the tonemap sub-field (8..11); bit 7 stays free. Mirrors the shader's
/// `load_vb_sdf_mesh_mode` (`light_table.hlsli`: `(LightBuf[7] >> 5) & 3`). `0` on every
/// pre-SV0 scene ⇒ both gated blocks are structurally skipped ⇒ byte-identical (the 0%-gate).
pub const VB_SDF_MESH_MODE_SHIFT: u32 = 5;
/// The 2-bit mask for the VB-SV0 sub-field (bits [`VB_SDF_MESH_MODE_SHIFT`]..+2).
pub const VB_SDF_MESH_MODE_MASK: u32 = 0x3;
/// Bit 5 within the [`VB_SDF_MESH_MODE_SHIFT`] sub-field — the SDF soft shadow on mesh.
/// Mirrors the shader's `VB_SDF_MESH_SHADOW_BIT`.
pub const VB_SDF_MESH_SHADOW_BIT: u32 = 1;
/// Bit 6 within the [`VB_SDF_MESH_MODE_SHIFT`] sub-field — the 5-tap contact AO on mesh.
/// Mirrors the shader's `VB_SDF_MESH_AO_BIT`.
pub const VB_SDF_MESH_AO_BIT: u32 = 2;

// The bit-position pin, at COMPILE time rather than in a `debug_assert!`. The idiom
// `ddgi_config.rs:288-289` uses puts the pin at the single production writer, but SV0's writer
// is rung S4's resolver and does not exist yet — and a `debug_assert!` would in any case be
// compiled out of the release profile the goldens run under (the same trap the plan's R11
// tripwire had to be re-sited out of). A `const` assertion holds in every profile and needs no
// writer to exist. It reds the moment either the sub-field or a neighbour is moved onto it.
const _: () = assert!(
    VB_SDF_MESH_MODE_SHIFT == 5 && VB_SDF_MESH_MODE_MASK == 0x3,
    "invariant: the VB-SV0 header gate is word-7 bits 5..6"
);
const _: () = assert!(
    VB_SDF_MESH_MODE_SHIFT > DDGI_MODE_BIT
        && VB_SDF_MESH_MODE_SHIFT + 2 <= TONEMAP_MODE_SHIFT,
    "invariant: the VB-SV0 sub-field must sit strictly between the DDGI gate bit and the \
     tonemap sub-field, with no overlap on either side"
);

/// The resolve's output-stage tonemap curve — packed into light-header word 7
/// bits [`TONEMAP_MODE_SHIFT`..+4) by [`LightHeaderGpu::new`]. `#[repr(u32)]` so
/// `self as u32` is the wire value. `Aces` = 0 ⇒ zero bits ⇒ word 7 byte-identical
/// on every default scene (the 0%-gate). All curves are linear-in → linear-\[0,1\]-out;
/// the shared manual OETF (`pow(x, 1/2.2)`) is applied after, unchanged.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tonemapper {
    /// Stephen Hill ACES-fitted (today's curve). The BYTE-IDENTICAL default.
    #[default]
    Aces = 0,
    /// Khronos PBR Neutral — LUT-free, hue-preserving, gentle toe (no shadow crush).
    Neutral = 1,
    /// Reinhard-Jodie — cheap hybrid luminance/per-channel, hue-preserving.
    ReinhardJodie = 2,
}

/// Word-7 sub-field for the tonemap mode: bits 8..11 (4 bits, 16 operators).
/// Above the shadow/GI gate bits (0..4); bits 5..7 stay free.
pub const TONEMAP_MODE_SHIFT: u32 = 8;
/// The 4-bit mask for the tonemap sub-field (bits [`TONEMAP_MODE_SHIFT`]..+4).
pub const TONEMAP_MODE_MASK: u32 = 0xF;

/// Word-7 sub-field for the diffuse terminator-softening amount: bits 12..19 (8 bits,
/// 0..255 = softening 0.0..1.0). Above the tonemap sub-field (8..11); bits 20..31 stay
/// free. 0 ⇒ OFF ⇒ byte-identical (the 0%-gate).
pub const TERMINATOR_SOFT_SHIFT: u32 = 12;
/// The 8-bit mask for the terminator-softening sub-field (bits
/// [`TERMINATOR_SOFT_SHIFT`]..+8).
pub const TERMINATOR_SOFT_MASK: u32 = 0xFF;

/// The global lighting config (Decision 3) — a `World`-singleton resource. `exposure`
/// defaults to identity (`1.0`) and `sky_*` default to the resolve's old `SKY_*`
/// constants, so a world that never inserts a non-default config reproduces today's
/// image (the 0%-gate anchor).
///
/// # Field taxonomy
///
/// - **Output stage** (applied once, after all lighting is accumulated):
///   [`exposure`](Self::exposure), [`tonemapper`](Self::tonemapper),
///   [`terminator_softening`](Self::terminator_softening).
/// - **Sky ambient** (the L0a hemisphere term): [`sky_diffuse`](Self::sky_diffuse),
///   [`sky_spec`](Self::sky_spec).
/// - **Cluster policy** (L1 froxel cull gate): [`clusters_enabled`](Self::clusters_enabled),
///   [`cluster_select`](Self::cluster_select).
/// - **Derived cluster geometry** (owner-set only in a harness that holds the lock-step
///   contract; the production writer is
///   [`sync_cluster_light_gate`](crate::light::sync_cluster_light_gate)):
///   [`cluster_z_scale`](Self::cluster_z_scale), [`cluster_z_bias`](Self::cluster_z_bias),
///   [`cluster_packed_dims`](Self::cluster_packed_dims).
/// - **Derived gates** (owner-set only in a harness that holds their lock-step
///   contract; production writers are the named sync systems, see each field's own
///   doc): [`csm_shadows`](Self::csm_shadows), [`punctual_shadows`](Self::punctual_shadows),
///   [`ddgi_indirect`](Self::ddgi_indirect).
///
/// See the "Light-header word 7 bit budget" table above this type for exactly which
/// word-7 bits each packed field occupies.
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
    /// The L1 exp-Z slice scale ([`ClusterConfig::z_scale`]), packed into light-header
    /// `cluster_params[0]` by [`LightHeaderGpu::new`]. DEFAULT `0.0` (the byte-identical
    /// 0%-gate — matches `LightHeaderGpu::new`'s pre-VB-P1b-0 hardcoded zero lane).
    ///
    /// # Single-writer / lock-step contract (VB-P1b-0, scoped by W1)
    ///
    /// DERIVED state, not owner state: its single production writer is
    /// [`sync_cluster_light_gate`], which keeps this lane in lock-step with the LIVE
    /// [`ClusterConfig`] the owner authored. **The invariant is "non-zero IFF the VB froxel
    /// cull is boot-armed"** — gated on
    /// [`ResolvedRenderPath::froxel_light_cull`](crate::render_path_config::ResolvedRenderPath::froxel_light_cull)
    /// (`clusters_enabled && path == VisibilityBuffer`, resolved ONCE at boot), NOT on
    /// [`Self::clusters_enabled`] alone: this lane is consumed by the VB `#ifdef FROXEL`
    /// resolve (`vb_resolve.comp.hlsl`/`vb_shade.comp.hlsl`), AND ALSO — unconditionally in
    /// `deferred_pbr.hlsl`, and at runtime in ForwardPlus's `forward_opaque_froxel.fs.hlsl` —
    /// by the non-VB resolves, whose `ClusterGrid`/`LightIndexList` bindings fall back to the
    /// light-table buffer as a placeholder whenever the real L1 cull buffers are not built
    /// (true on every current Deferred/ForwardPlus boot). Gating on `froxel_light_cull` keeps
    /// this lane's dims at `0` on every non-VB-armed path — the SAME pre-campaign state
    /// (`LightHeaderGpu::new` hardcoded it to zero) — so a `clusters_enabled == true` world
    /// under Deferred/ForwardPlus (an owner mistake, or `ClusterSelectMode::Auto` banding) is
    /// STILL byte-identical to before this campaign, never a new OOB surface. Set this
    /// manually only in a harness holding the SAME lock-step (the shadow/CSM/DDGI gates' own
    /// discipline).
    pub cluster_z_scale: f32,
    /// The L1 exp-Z slice bias ([`ClusterConfig::z_bias`]), packed into light-header
    /// `cluster_params[1]`. Same single-writer contract as [`Self::cluster_z_scale`]
    /// (including the `froxel_light_cull`-scoped armed condition). DEFAULT `0.0` (the
    /// 0%-gate).
    pub cluster_z_bias: f32,
    /// The L1 froxel grid dims, packed (`dim_x | dim_y<<8 | dim_z<<16`,
    /// [`ClusterConfig::packed_dims`]) into light-header `cluster_params[2]`. Same
    /// single-writer contract as [`Self::cluster_z_scale`] (including the
    /// `froxel_light_cull`-scoped armed condition). DEFAULT `0` (the 0%-gate — a zero-dims
    /// header reads as `dim_x == dim_y == dim_z == 0`; the VB `#ifdef FROXEL` resolve treats
    /// this as unarmed by construction — see `LightHeaderGpu::new`'s doc — and a non-VB
    /// resolve reading it while `clusters_enabled` happens to be `true` sees the SAME
    /// zero-dims state it always has, pre-campaign).
    pub cluster_packed_dims: u32,
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
    /// The resolve's DDGI (SDF diffuse GI) sample gate — packed into light-header word 7
    /// bit [`DDGI_MODE_BIT`] by [`shadow_gate_word`](Self::shadow_gate_word). DEFAULT
    /// `false` (word 7 bit 4 stays 0 — the byte-identical 0%-gate, INDEPENDENT of
    /// `csm_shadows` / `punctual_shadows`).
    ///
    /// # Single-writer / lock-step contract (SDFDDGI)
    ///
    /// Like the shadow gates, this is DERIVED state: its single production writer is
    /// [`sync_ddgi_light_gate`](crate::ddgi_config::sync_ddgi_light_gate), which keeps the
    /// header gate in lock-step with the structural GI predicate
    /// [`DdgiConfig::enabled`](crate::ddgi_config::DdgiConfig::enabled). At SDFDDGI I0 the
    /// gated resolve block is EMPTY (no probe sample yet), so even an armed gate leaves the
    /// pixels byte-identical; later rungs (I3) wire the probe-irradiance injection.
    pub ddgi_indirect: bool,
    /// The resolve's SSAO-combine gate — written into light-header word 11 (`sky_spec.w`,
    /// the ambient-specular lane's otherwise-unused `w`) by [`LightHeaderGpu::new`].
    /// Unlike `csm_shadows`/`punctual_shadows`/`ddgi_indirect` above (which share word 7's
    /// bit-packed budget via [`Self::shadow_gate_word`]), this gate owns its OWN dedicated
    /// word — no packing needed (Render P7-Q2's `ssao_mode` is a whole-word `0`/`1`, the
    /// same shape `GoldenLightHeader::with_ssao_mode` pins). DEFAULT `false` (word 11 stays
    /// `0.0` — the byte-identical 0%-gate, INDEPENDENT of every other gate).
    ///
    /// # Single-writer / lock-step contract (Render P7-Q2 live consumer)
    ///
    /// Like the shadow/GI gates, this is DERIVED state: its single production writer is
    /// [`sync_ssao_light_gate`](crate::ssao_config::sync_ssao_light_gate), which keeps the
    /// header gate in lock-step with the structural SSAO predicate
    /// [`SsaoConfig::enabled`](crate::ssao_config::SsaoConfig::enabled) — mirrors
    /// [`sync_ddgi_light_gate`](crate::ddgi_config::sync_ddgi_light_gate)'s bridge shape (a
    /// single cold config Resource, no caster dependency).
    pub ssao_mode: bool,
    /// VB-SV0: the VB lit-producer tails' SDF soft-shadow-on-mesh gate — packed into
    /// light-header word 7 bit `VB_SDF_MESH_MODE_SHIFT + 0` (bit 5) by
    /// [`shadow_gate_word`](Self::shadow_gate_word). DEFAULT `false` (bit 5 stays 0 — the
    /// byte-identical 0%-gate, INDEPENDENT of every other gate including its own AO sibling).
    ///
    /// # Why this is a SEPARATE field from [`vb_sdf_mesh_ao`](Self::vb_sdf_mesh_ao)
    ///
    /// SV0 is two terms — a shadow and a contact AO — and every gate written against it as one
    /// feature was satisfiable by the shadow half alone, which is how a structurally-dead AO
    /// term could have shipped green. Two bits means each half can be armed on its own, and an
    /// arming gate can require each half to move pixels on its own.
    ///
    /// # This field is the REQUEST; [`vb_sdf_mesh_shadow_armed`](Self::vb_sdf_mesh_shadow_armed)
    /// is what the header packs (rung S4, code-review P2-c)
    ///
    /// The OWNER writes this one and nothing else reads it except rung S4's
    /// [`sync_sv0_light_gate`], which ANDs it with
    /// [`ResolvedRenderPath::vb_sdf_mesh_armable`](crate::render_path_config::ResolvedRenderPath::vb_sdf_mesh_armable)
    /// — CONSUMING the already-resolved `SDF_SOFT_MARCH` bit rather than re-deriving the
    /// predicate — and publishes the result into the `_armed` sibling.
    ///
    /// **Why the request and the resolved value are two fields and not one.** The first S4
    /// revision clamped IN PLACE, writing the resolved value back over the owner's request. That
    /// makes an owner who sets this field every frame (the ordinary way to drive a per-frame
    /// toggle) pay a full light-table re-fold every frame on any boot that cannot carry SV0: the
    /// gate clears the field, the owner re-sets it, the gate sees a change and re-dirties. With
    /// the two separated, the gate's value comparison is against state only IT writes, so a
    /// per-frame owner writer costs exactly nothing. This is also the shape every sibling gate
    /// already has (`ssao_mode` ← `SsaoConfig`, `csm_shadows` ← the caster predicate): a DERIVED
    /// field with one production writer, fed from a separate owner-facing input.
    ///
    /// DEFAULT `false`, and the resolve is monotone DOWNWARD (`request && capability`), so a
    /// world that never opts in can never be armed by anything downstream — which is what makes
    /// every pre-SV0 golden byte-identical by construction rather than by argument.
    ///
    /// Rung S2 shipped SV0 DARK — no writer existed at all, so the compiled-in shader blocks were
    /// unreachable on every configuration.
    pub vb_sdf_mesh_shadow: bool,
    /// VB-SV0: the OWNER'S REQUEST for the contact-AO-on-mesh term — the AO sibling of
    /// [`vb_sdf_mesh_shadow`](Self::vb_sdf_mesh_shadow), resolved into
    /// [`vb_sdf_mesh_ao_armed`](Self::vb_sdf_mesh_ao_armed) by the same gate. DEFAULT `false`.
    ///
    /// See [`vb_sdf_mesh_shadow`](Self::vb_sdf_mesh_shadow) for why the two terms get separate
    /// bits and for the request/resolved contract they share.
    pub vb_sdf_mesh_ao: bool,
    /// VB-SV0: the RESOLVED SDF soft-shadow-on-mesh gate — packed into light-header word 7 bit
    /// `VB_SDF_MESH_MODE_SHIFT + 0` (bit 5) by [`shadow_gate_word`](Self::shadow_gate_word).
    /// DEFAULT `false` (bit 5 stays 0 — the byte-identical 0%-gate, INDEPENDENT of every other
    /// gate including its own AO sibling).
    ///
    /// # Single-writer contract
    ///
    /// DERIVED state with exactly ONE production writer, [`sync_sv0_light_gate`] — the same
    /// contract `csm_shadows` / `punctual_shadows` / `ddgi_indirect` / `ssao_mode` carry. Setting
    /// it by hand bypasses the capability clamp and arms a shader block on a boot whose producer
    /// may not exist; the only legitimate direct writes are in this module's own unit tests,
    /// which exercise the packing rather than the resolve.
    pub vb_sdf_mesh_shadow_armed: bool,
    /// VB-SV0: the RESOLVED contact-AO-on-mesh gate — packed into light-header word 7 bit
    /// `VB_SDF_MESH_MODE_SHIFT + 1` (bit 6) by [`shadow_gate_word`](Self::shadow_gate_word).
    /// DEFAULT `false`. Same single-writer contract as
    /// [`vb_sdf_mesh_shadow_armed`](Self::vb_sdf_mesh_shadow_armed).
    pub vb_sdf_mesh_ao_armed: bool,
    /// The resolve's output-stage tonemap curve — packed into light-header word 7 bits
    /// [`TONEMAP_MODE_SHIFT`..+4) by [`Self::tonemap_bits`]. DEFAULT [`Tonemapper::Aces`]
    /// (word 7 bits 8..11 stay 0 — the byte-identical 0%-gate).
    pub tonemapper: Tonemapper,
    /// Softens the diffuse light terminator — the harsh `max(dot(N,L),0)` boundary that
    /// turns normal-map bump slopes into hard dark islands under grazing light — into a
    /// wrapped ramp (`nol_wrapped`, Valve/half-Lambert style), packed into light-header
    /// word 7 bits [`TERMINATOR_SOFT_SHIFT`..+8) by [`Self::terminator_bits`]. DEFAULT
    /// `0.0` (word 7 bits 12..19 stay 0 — the byte-identical 0%-gate, the physically-sharp
    /// default); ~0.15-0.3 gives a soft film-like falloff. Applied ONLY to the diffuse NoL
    /// of direct lights — specular NoL and the shadow-gating NoL comparisons are untouched.
    pub terminator_softening: f32,
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
            cluster_z_scale: 0.0,
            cluster_z_bias: 0.0,
            cluster_packed_dims: 0,
            csm_shadows: false,
            punctual_shadows: false,
            ddgi_indirect: false,
            ssao_mode: false,
            vb_sdf_mesh_shadow: false,
            vb_sdf_mesh_ao: false,
            vb_sdf_mesh_shadow_armed: false,
            vb_sdf_mesh_ao_armed: false,
            tonemapper: Tonemapper::Aces,
            terminator_softening: 0.0,
        }
    }
}

impl LightingConfig {
    /// Packs the header's word-7 shadow/GI-gate bits from this config: the CSM bit
    /// ([`CSM_MODE_BIT`]), the punctual bit ([`PUNCTUAL_MODE_BIT`]), the DDGI bit
    /// ([`DDGI_MODE_BIT`]), and VB-SV0's 2-bit sub-field ([`VB_SDF_MESH_MODE_SHIFT`]), each
    /// independent. A default config returns 0 (word 7 == 0.0 — the 0%-gate anchor every
    /// pre-R4/pre-punctual/pre-SDFDDGI/pre-SV0 golden pins).
    #[inline]
    pub const fn shadow_gate_word(&self) -> u32 {
        // VB-SV0's two bits are OR-ed as one sub-field so the shift/mask pair stays the single
        // place the bit positions are spelled — the shader decodes with the same `>> 5 & 3`.
        //
        // The RESOLVED `_armed` pair, never the owner's request: the header must carry what the
        // boot can actually execute, and routing the request straight through would arm a shader
        // block on a producer that does not exist (code-review P2-c).
        let sv0 = ((self.vb_sdf_mesh_shadow_armed as u32) * VB_SDF_MESH_SHADOW_BIT)
            | ((self.vb_sdf_mesh_ao_armed as u32) * VB_SDF_MESH_AO_BIT);
        ((self.csm_shadows as u32) << CSM_MODE_BIT)
            | ((self.punctual_shadows as u32) << PUNCTUAL_MODE_BIT)
            | ((self.ddgi_indirect as u32) << DDGI_MODE_BIT)
            | ((sv0 & VB_SDF_MESH_MODE_MASK) << VB_SDF_MESH_MODE_SHIFT)
    }

    /// Word-7 tonemap sub-field bits (0 for [`Tonemapper::Aces`] ⇒ the 0%-gate).
    #[inline]
    pub const fn tonemap_bits(&self) -> u32 {
        (self.tonemapper as u32) << TONEMAP_MODE_SHIFT
    }

    /// Word-7 terminator-softening sub-field bits (0 for the 0.0 default ⇒ the 0%-gate).
    #[inline]
    pub const fn terminator_bits(&self) -> u32 {
        // Hand-rolled clamp (const-fn parity with `tonemap_bits` — L1 review G3):
        // `f32::clamp` is a trait-free inherent method but was not yet usable from a
        // `const fn` on this toolchain when this was written; the plain if/else below
        // is unconditionally const-evaluable.
        let x = self.terminator_softening;
        let clamped = if x < 0.0 {
            0.0
        } else if x > 1.0 {
            1.0
        } else {
            x
        };
        let q = (clamped * 255.0 + 0.5) as u32;
        (q & TERMINATOR_SOFT_MASK) << TERMINATOR_SOFT_SHIFT
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
/// `CLUSTER_DIM_*` / [`MAX_LIGHTS_PER_CLUSTER`] / [`INDEX_LIST_CAP`] constants; a world
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

    /// VB-P1e D11: the hierarchical cull's workgroup width — the host mirror of
    /// `cluster_cull.hlsl`'s `#define HIER_TPG 256u`. A `const fn` rather than the
    /// [`HIER_MASK_WORDS`]-style bare constant because it is paired 1:1 with
    /// [`Self::hier_group_count`] at every call site (D9's radix-16 fold hardcodes this exact
    /// width — see the shader's own `#error HIER_TPG != 256` guard).
    #[inline]
    pub const fn hier_group_threads() -> u32 {
        256
    }

    /// VB-P1e D11: the hierarchical cull's 1D dispatch group count — one 256-wide group per
    /// `ceil(dim_x * dim_y / 256)` screen block, repeated per `dim_z` slice
    /// (`ceil(dim_x * dim_y / 256) * dim_z`) — the same value as the shader's own
    /// `gps = (bdx * bdy + 255u) / 256u`. Rev 5 P2 wrote the host mirror as `(dim_x * dim_y +
    /// 255) / 256` (the shader's own token-for-token form) rather than `.div_ceil()`, whose
    /// const-stability the plan did not want to depend on; on this toolchain (`rustc 1.95`)
    /// `u32::div_ceil` IS `const fn`, and `clippy::manual_div_ceil` (`-D warnings`) rejects the
    /// hand-written form, so this fn uses `.div_ceil(256)` — arithmetically identical, `const
    /// fn`-compatible here, and the clippy-mandated spelling. The shader's extra `max(1u, …)`
    /// has no host counterpart on purpose — with `dim_x * dim_y == 0` this host dispatches ZERO
    /// groups, so the shader's guard is unreachable from here (D8 obligation 3 keeps it for the
    /// shader's own totality proof).
    #[inline]
    pub const fn hier_group_count(&self) -> u32 {
        (self.dim_x * self.dim_y).div_ceil(256) * self.dim_z
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

/// Bridges the [`ClusterConfig`] grid/near/far parameters and the [`LightingConfig`] header
/// gate — the cluster analogue of
/// [`sync_ssao_light_gate`](crate::ssao_config::sync_ssao_light_gate) (a single cold config
/// Resource read directly, no caster dependency — unlike
/// [`sync_csm_light_gate`](crate::csm_caster::sync_csm_light_gate)/
/// [`sync_punctual_light_gate`](crate::shadow_atlas::sync_punctual_light_gate), which also
/// gate on a live caster count). It is the SOLE production writer of
/// [`LightingConfig::cluster_z_scale`]/[`LightingConfig::cluster_z_bias`]/
/// [`LightingConfig::cluster_packed_dims`], keeping the header's L1 cluster lane
/// ([`LightHeaderGpu::pack_cluster_params`]) in lock-step with the LIVE `ClusterConfig` the
/// owner authored. Without this gate, `LightHeaderGpu::new`'s unconditional read of those
/// three fields would pack whatever `LightingConfig` happened to carry from the last
/// `ClusterConfig` edit — stale the moment the owner changes the grid/near/far without also
/// touching a light (the SAME staleness class the CSM/DDGI/punctual/SSAO gates close for
/// their own header bits).
///
/// # The armed condition is VB-froxel-boot-scoped, NOT `clusters_enabled` alone (W1 fix)
///
/// The dims/scale/bias are packed real ONLY when
/// [`ResolvedRenderPath::froxel_light_cull`](crate::render_path_config::ResolvedRenderPath::froxel_light_cull)
/// is `true` — the SAME boot-frozen bit [`boyko_app::gpu_scene::GpuSceneBundles::build_froxel_light_cull`]
/// gates on (`clusters_enabled && path == VisibilityBuffer`, resolved ONCE at boot, never
/// re-derived) — NOT on the live [`LightingConfig::clusters_enabled`] alone. This matters because
/// `deferred_pbr.hlsl` (unconditionally) and `forward_opaque_froxel.fs.hlsl` (ForwardPlus's
/// production shader) ALSO read this header lane, but their app-side `ClusterGrid`/
/// `LightIndexList` bindings fall back to `scene.light_table` as a placeholder whenever the real
/// L1 cull buffers are not built (`targets.rs`) — true on every current Deferred/ForwardPlus
/// boot. Gating on `froxel_light_cull` (which is `false` for every non-VB path, and for a VB
/// path whose `clusters_enabled` was `false` at BOOT, by construction) keeps this lane's dims
/// at `0` on every path OTHER than a genuinely VB-froxel-armed one — exactly the pre-campaign
/// state (`LightHeaderGpu::new` hardcoded the lane to all-zero) for Deferred/ForwardPlus, so
/// VB-P1b-0 introduces ZERO new reachability for that pre-existing cross-path hazard. A
/// dedicated cross-path shader-guard hardening (mirroring the VB `#ifdef FROXEL` seam's
/// `dim_x*dim_y*dim_z != 0` check) is tracked as a separate rung, not this one.
///
/// [`LightingConfig::clusters_enabled`] itself is NOT written here (it stays owner/
/// [`ClusterSelectMode::Auto`]-policy-set, `select_lighting_cull`'s own concern) — this gate
/// only derives the GEOMETRY the enabled bit's cluster path needs, zeroing it whenever the VB
/// froxel cull is not boot-armed (mirrors [`Self::z_scale`](ClusterConfig::z_scale)'s own
/// degenerate-config `0.0` fallback, so an armed-but-degenerate `ClusterConfig` never crashes,
/// only culls nothing).
///
/// # Value-gated write
///
/// Written only on an actual change (any of the three derived scalars differs), so a static
/// frame does zero work and never dirties the light table (mirrors the sibling gates' value-
/// gate discipline).
///
/// # Registration — app-wired (matches `sync_ssao_light_gate` / `sync_ddgi_light_gate`)
///
/// NOT registered by [`LightingPlugin`](crate::light_plugin::LightingPlugin): `ClusterConfig`
/// is seeded by the composing app (mirrors `LightingConfig` itself — see
/// `boyko_app::plugins::EnginePlugins::build`), so this lives alongside the other
/// `sync_*_light_gate` bridges in that SAME builder closure. UNLIKE the sibling gates it DOES
/// carry an explicit `.before_set(LightCollectSet)` edge (VB-P1b-0 C1 — this gate feeds a GPU
/// buffer INDEX, not merely a scalar bit, so a one-frame-stale header would be genuine GPU UB
/// rather than a benign wrong bit).
#[allow(clippy::needless_pass_by_value)]
pub fn sync_cluster_light_gate(
    cluster: Res<ClusterConfig>,
    resolved_path: Res<crate::render_path_config::ResolvedRenderPath>,
    mut cfg: ResMut<LightingConfig>,
    mut dirty: ResMut<LightTableDirty>,
) {
    let (z_scale, z_bias, packed_dims) = if resolved_path.froxel_light_cull {
        (cluster.z_scale(), cluster.z_bias(), cluster.packed_dims())
    } else {
        (0.0, 0.0, 0)
    };
    // Value gate BEFORE the `DerefMut`: flip-only write, flip-only table dirtying.
    let changed = cfg.cluster_z_scale != z_scale
        || cfg.cluster_z_bias != z_bias
        || cfg.cluster_packed_dims != packed_dims;
    if changed {
        cfg.cluster_z_scale = z_scale;
        cfg.cluster_z_bias = z_bias;
        cfg.cluster_packed_dims = packed_dims;
        dirty.0 = true;
    }
}

/// Logs the first SV0 request this boot could not honour, and nothing thereafter.
///
/// `#[cold]` + `#[inline(never)]` for the same reason as `light_system`'s
/// `report_dropped_non_finite_light`: only the two compares of [`sync_sv0_light_gate`]'s
/// capability test stay on the per-frame straight-line code.
///
/// The `eprintln!` is bounded at ONE per process by the latch below — unlike a per-transition
/// diagnostic it cannot be driven at frame rate by any input, so it needs no build-profile gate.
///
/// # Why this is worth a diagnostic at all
///
/// A silently-cleared request renders a frame that is byte-identical to the unarmed one, which
/// downstream reads as "the SV0 term moved zero pixels" — the failure and its symptom are the
/// same image. Naming the clamp in the run log is what separates "SV0 is broken" from "this
/// scene was never a VB x Both boot in the first place".
#[cold]
#[inline(never)]
fn report_sv0_request_clamped(shadow: bool, ao: bool) {
    use core::sync::atomic::{AtomicBool, Ordering};
    static LOGGED: AtomicBool = AtomicBool::new(false);
    // Relaxed: a best-effort one-shot log guard, not a synchronization edge — nothing is
    // published through this flag and a racing double-print is cosmetic.
    if !LOGGED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "boyko_render: VB-SV0 was requested (shadow={shadow}, ao={ao}) on a boot whose \
             resolved render path cannot carry it (needs path == VisibilityBuffer, a mesh leg, \
             and ShadowSources::SDF_SOFT_MARCH) — both gate bits forced OFF for this run"
        );
    }
}

/// **VB-SV0 (`docs/VB-SV0-SDF-SHADOW-PLAN.md` §S4, "arm"): the SOLE production writer of the
/// light header's word-7 bits 5..6** — the SDF-soft-shadow-on-mesh and contact-AO-on-mesh gates
/// the three VB lit-producer tails decode with `load_vb_sdf_mesh_mode`.
///
/// # Request in, capability-resolved value out
///
/// [`LightingConfig::vb_sdf_mesh_shadow`] / [`LightingConfig::vb_sdf_mesh_ao`] carry the OWNER's
/// REQUEST (both DEFAULT `false` — the 0%-gate: a world that never opts in is byte-identical to
/// every pre-SV0 pin). This gate ANDs each of them with
/// [`ResolvedRenderPath::vb_sdf_mesh_armable`](crate::render_path_config::ResolvedRenderPath::vb_sdf_mesh_armable)
/// and publishes the result into
/// [`LightingConfig::vb_sdf_mesh_shadow_armed`] / [`LightingConfig::vb_sdf_mesh_ao_armed`], which
/// is what [`LightingConfig::shadow_gate_word`] packs. So the header carries
/// `request && capability`, and the owner's own field is never written.
///
/// **The resolve is monotone DOWNWARD and that is the load-bearing property.** `_armed` can only
/// ever be `request && …`, never more, so no scene that has not asked for SV0 can be armed by
/// this system — the "every existing golden stays byte-identical" guarantee is structural here,
/// not an argument about which shipped fixtures happen to resolve `VB x Both`. (Several do:
/// `boyko_app::runner` hardwires `sdf_shadows_wanted: true`, so `[vb_both]`, `[vb_both_taa]` and
/// the two S1 fixtures are all *capable*. What keeps them unarmed is that they do not ask.)
///
/// # Why the request is not clamped IN PLACE (code-review P2-c)
///
/// An in-place clamp is invisible when the owner sets the field once at startup and pathological
/// when they set it every frame — which is the ordinary way to drive a toggle. On a boot that
/// cannot carry SV0 the gate would clear the request, the owner would re-set it, the gate would
/// see a change and re-dirty [`LightTableDirty`], and the WHOLE light table would be re-packed
/// and re-uploaded every frame, forever, with no visible symptom. Writing only state this system
/// owns removes the cycle: the value comparison below is against the gate's own last output, so a
/// per-frame owner writer is free.
///
/// # Why it CONSUMES `ResolvedRenderPath::shadow` instead of re-deriving the predicate
///
/// See [`ResolvedRenderPath::vb_sdf_mesh_armable`](crate::render_path_config::ResolvedRenderPath::vb_sdf_mesh_armable):
/// the `sdf_leg && sdf_shadows_wanted && !hwrt_denoise_or_vis_on` rule lives in `resolve_rules`
/// and nowhere else. A mirrored copy here would be a second truth to keep in sync, and the
/// campaign's own record is that mirrored predicates drift.
///
/// # Registration — app-wired, WITH an ordering edge (code-review P2-b)
///
/// NOT registered by [`LightingPlugin`](crate::light_plugin::LightingPlugin): it bridges
/// `RenderPathPlugin`'s [`ResolvedRenderPath`](crate::render_path_config::ResolvedRenderPath)
/// and `LightingPlugin`'s [`LightingConfig`], so it lives in `boyko_app::plugins`' builder
/// closure alongside the other `sync_*_light_gate` bridges.
///
/// It carries `.before_set(LightCollectSet)` — the edge `sync_cluster_light_gate` added at
/// VB-P1b for the same reason and `sync_ssao_light_gate` does not need. `sync_ssao_light_gate`
/// reads a config the owner sets and writes the bit that config implies, so an unordered first
/// frame packs a value that is merely one frame late. THIS gate resolves a request against a
/// CAPABILITY: unordered, the first armed frame packs whatever `_armed` held before the resolve
/// ran. The residue is a one-frame WRONG-STATE header, not a late one — and on a fixture that
/// dumps a small fixed number of frames, "one frame" can be the frame that gets measured.
///
/// # Value-gated write
///
/// Written only when the resolved pair actually moves, so an armed steady state does zero work
/// and never re-dirties the light table (the sibling gates' discipline).
#[allow(clippy::needless_pass_by_value)]
pub fn sync_sv0_light_gate(
    resolved_path: Res<crate::render_path_config::ResolvedRenderPath>,
    mut cfg: ResMut<LightingConfig>,
    mut dirty: ResMut<LightTableDirty>,
) {
    let armable = resolved_path.vb_sdf_mesh_armable();
    let requested_shadow = cfg.vb_sdf_mesh_shadow;
    let requested_ao = cfg.vb_sdf_mesh_ao;
    let shadow = requested_shadow && armable;
    let ao = requested_ao && armable;
    // Value gate BEFORE the `DerefMut`: flip-only write, flip-only table dirtying.
    if cfg.vb_sdf_mesh_shadow_armed != shadow || cfg.vb_sdf_mesh_ao_armed != ao {
        cfg.vb_sdf_mesh_shadow_armed = shadow;
        cfg.vb_sdf_mesh_ao_armed = ao;
        dirty.0 = true;
    }
    // Keyed on REQUEST-vs-CAPABILITY, not on the write above: an honoured request also moves the
    // value, and reporting that as a clamp would cry wolf on every armed boot. Two loads and a
    // branch per frame on the straight-line path; the message itself is `#[cold]` and one-shot.
    if !armable && (requested_shadow || requested_ao) {
        report_sv0_request_clamped(requested_shadow, requested_ao);
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
    /// the baked `I = Φ/(2π(1−cos(outer)))` stays bounded. `direction` (the SHINE axis) is
    /// normalized here; `color` is LINEAR; `power` is `Φ` (lumens). NOTE: `direction` is a
    /// SEED — when the entity has a `GlobalTransform`, `light_reconcile` overwrites it with
    /// the transform's world `-Z`, so aim the spot via the pose (`look_at`), not this arg.
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
    /// spot SHINE axis (un-negated). The L0b resolve consumes all three lanes.
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
    /// `cfg`. The L1 `cluster_params` lane is read verbatim from `cfg`'s derived
    /// [`LightingConfig::cluster_z_scale`]/[`cluster_z_bias`](LightingConfig::cluster_z_bias)/
    /// [`cluster_packed_dims`](LightingConfig::cluster_packed_dims) — single-writer
    /// [`sync_cluster_light_gate`] keeps those `0.0`/`0.0`/`0` while
    /// [`LightingConfig::clusters_enabled`] is `false` (the 0%-gate), so a world that never
    /// arms clustering reproduces the pre-VB-P1b-0 all-zero lane exactly. Word 7 (`sky_diffuse.w`) carries
    /// the shadow-gate bits ([`LightingConfig::shadow_gate_word`] — CSM bit
    /// [`CSM_MODE_BIT`]; 0 for a default config, the 0%-gate) ORed with the tonemap
    /// sub-field ([`LightingConfig::tonemap_bits`] — bits [`TONEMAP_MODE_SHIFT`]..+4)
    /// ORed with the terminator-softening sub-field
    /// ([`LightingConfig::terminator_bits`] — bits [`TERMINATOR_SOFT_SHIFT`]..+8). Word 11
    /// (`sky_spec.w`) carries [`LightingConfig::ssao_mode`] verbatim (a whole-word `0`/`1`,
    /// no packing — see that field's doc).
    #[inline]
    pub fn new(l0a_count: u32, point_spot_count: u32, cfg: &LightingConfig) -> Self {
        debug_assert!(cfg.exposure > 0.0 && cfg.exposure.is_finite(), "invariant: exposure > 0");
        let light_count = l0a_count + point_spot_count;
        debug_assert!(light_count <= MAX_LIGHTS, "invariant: light_count <= MAX_LIGHTS");
        debug_assert!(
            (cfg.tonemapper as u32) <= TONEMAP_MODE_MASK,
            "invariant: tonemapper fits the 4-bit word-7 sub-field"
        );
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
                f32::from_bits(cfg.shadow_gate_word() | cfg.tonemap_bits() | cfg.terminator_bits()),
            ],
            sky_spec: [
                cfg.sky_spec[0],
                cfg.sky_spec[1],
                cfg.sky_spec[2],
                // Render P7-Q2: the resolve's SSAO-combine gate (word 11, previously
                // always 0.0 — `sky_spec.w` was otherwise unused). `false` (the default)
                // keeps this lane `0.0`, byte-identical to every pre-P7-Q2 golden.
                f32::from_bits(u32::from(cfg.ssao_mode)),
            ],
            // VB-P1b-0: the cluster lane is no longer hardcoded zero — `cfg.cluster_z_scale`/
            // `cluster_z_bias`/`cluster_packed_dims` are DERIVED fields `sync_cluster_light_gate`
            // (the single production writer) keeps at `0.0`/`0.0`/`0` while `clusters_enabled`
            // is `false`, so this read is byte-identical to the old hardcoded-zero lane for
            // every world that never arms clustering (the 0%-gate).
            cluster_params: [
                cfg.cluster_z_scale,
                cfg.cluster_z_bias,
                f32::from_bits(cfg.cluster_packed_dims),
                f32::from_bits(u32::from(cfg.clusters_enabled)),
            ],
        }
    }

    /// Packs `cluster`'s exp-Z slice scale/bias + froxel grid dims into this header's
    /// `cluster_params` lanes 0..2, UNCONDITIONALLY — the caller gates on
    /// [`LightingConfig::clusters_enabled`] beforehand (lane 3, the enabled bit, is left
    /// untouched here; [`Self::new`]/[`Self::new_clustered`] already set it from `cfg`).
    ///
    /// `[z_scale, z_bias, bitcast(packed_dims)]` (Decision 6):
    /// - `z_scale` / `z_bias` — the affine exp-Z slice map `slice = ln(view_z) * z_scale +
    ///   z_bias` the resolve applies (the cull builds froxel AABBs from the same near/far);
    /// - `packed_dims` — `dim_x | dim_y<<8 | dim_z<<16` (the resolve unpacks to map a pixel
    ///   to its `(x, y)` tile + clamp the slice).
    ///
    /// This is the SINGLE fn both [`Self::new_clustered`] (the test/host-oracle direct
    /// constructor) and the production [`sync_cluster_light_gate`] derive their packed
    /// values from — via [`ClusterConfig::z_scale`]/[`ClusterConfig::z_bias`]/
    /// [`ClusterConfig::packed_dims`] — so the two paths can never disagree bit-for-bit.
    #[inline]
    pub fn pack_cluster_params(&mut self, cluster: &ClusterConfig) {
        self.cluster_params[0] = cluster.z_scale();
        self.cluster_params[1] = cluster.z_bias();
        self.cluster_params[2] = f32::from_bits(cluster.packed_dims());
    }

    /// Builds the L1 header: identical to [`Self::new`] but the `cluster_params` lane 0..2
    /// carries `cluster`'s REAL exp-Z froxel-lookup factors ([`Self::pack_cluster_params`])
    /// instead of whatever `cfg` happened to carry — a direct, one-shot constructor for
    /// tests/host oracles that do not want to pre-populate `cfg`'s derived cluster fields.
    /// When `cfg.clusters_enabled` is `false` the lane stays exactly what [`Self::new`]
    /// produced (byte-identical to the 0%-gate anchor).
    #[inline]
    pub fn new_clustered(
        l0a_count: u32,
        point_spot_count: u32,
        cfg: &LightingConfig,
        cluster: &ClusterConfig,
    ) -> Self {
        let mut header = Self::new(l0a_count, point_spot_count, cfg);
        if cfg.clusters_enabled {
            header.pack_cluster_params(cluster);
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

    /// Whether the resolve's DDGI (SDF diffuse GI) sample gate is armed (word 7 bit
    /// [`DDGI_MODE_BIT`], bit-cast back from `sky_diffuse.w`) — the host mirror of the
    /// shader's DDGI gate. Independent of [`csm_mode`](Self::csm_mode) /
    /// [`punctual_mode`](Self::punctual_mode) (bit 4).
    #[inline]
    pub fn ddgi_mode(&self) -> bool {
        (self.sky_diffuse[3].to_bits() >> DDGI_MODE_BIT) & 1 != 0
    }

    /// Whether the resolve's SSAO-combine gate is armed (word 11, bit-cast back from
    /// `sky_spec.w`) — the host mirror of the shader's `load_ssao_mode`. Unlike
    /// [`csm_mode`](Self::csm_mode)/[`punctual_mode`](Self::punctual_mode)/
    /// [`ddgi_mode`](Self::ddgi_mode) (word 7 bits), this gate owns its own whole word —
    /// no bit shift, mirrors [`Self::clusters_enabled`]'s shape.
    #[inline]
    pub fn ssao_mode(&self) -> bool {
        self.sky_spec[3].to_bits() != 0
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
        // The DDGI gate defaults OFF (the byte-identical 0%-gate).
        assert!(!cfg.ddgi_indirect);
        // The tonemapper defaults to ACES (today's curve, the byte-identical 0%-gate).
        assert_eq!(cfg.tonemapper, Tonemapper::Aces);
        // Terminator softening defaults OFF (the physically-sharp, byte-identical 0%-gate).
        assert_eq!(cfg.terminator_softening, 0.0);
        // Word 7 is exactly 0 for a default config (the 0%-gate anchor).
        assert_eq!(cfg.shadow_gate_word(), 0);
        assert_eq!(cfg.tonemap_bits(), 0);
        assert_eq!(cfg.terminator_bits(), 0);
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

        // The DDGI bit alone (bit 4), INDEPENDENT of the CSM / punctual bits.
        let ddgi = LightingConfig { ddgi_indirect: true, ..LightingConfig::default() };
        assert_eq!(ddgi.shadow_gate_word(), 1 << DDGI_MODE_BIT);
        assert_eq!(
            ddgi.shadow_gate_word() & ((1 << CSM_MODE_BIT) | (1 << PUNCTUAL_MODE_BIT)),
            0,
            "ddgi must not touch bit 2 or 3"
        );

        // All three set — the independent bits OR together.
        let both = LightingConfig {
            csm_shadows: true,
            punctual_shadows: true,
            ddgi_indirect: true,
            ..LightingConfig::default()
        };
        assert_eq!(
            both.shadow_gate_word(),
            (1 << CSM_MODE_BIT) | (1 << PUNCTUAL_MODE_BIT) | (1 << DDGI_MODE_BIT)
        );
    }

    /// VB-SV0's two terms occupy word-7 bits 5 and 6 and are independently armable — of each
    /// other and of every neighbouring sub-field. The independence of the two SV0 bits FROM EACH
    /// OTHER is the load-bearing half: SV0 is two terms, and a packing that armed both from one
    /// flag would make every downstream per-term gate satisfiable by the shadow half alone.
    ///
    /// Written against the RESOLVED `_armed` pair, because that is what the packer reads: this
    /// test pins the PACKING, and `sv0_gate_*` below pins the resolve that feeds it.
    #[test]
    fn vb_sv0_gate_bits_are_independent() {
        // Default: SV0 contributes nothing — the 0%-gate rung S2 ships under.
        let base = LightingConfig::default();
        assert!(!base.vb_sdf_mesh_shadow_armed);
        assert!(!base.vb_sdf_mesh_ao_armed);
        assert_eq!(base.shadow_gate_word(), 0);

        // An owner REQUEST that no gate has resolved packs nothing — the request is not a value
        // (code-review P2-c). Without this, a packer wired to the request would still pass every
        // other assertion in this test.
        let requested_only = LightingConfig {
            vb_sdf_mesh_shadow: true,
            vb_sdf_mesh_ao: true,
            ..LightingConfig::default()
        };
        assert_eq!(
            requested_only.shadow_gate_word(),
            0,
            "an unresolved request must not reach the header — only `sync_sv0_light_gate` arms SV0"
        );

        // Shadow alone: bit 5 only.
        let sh = LightingConfig { vb_sdf_mesh_shadow_armed: true, ..LightingConfig::default() };
        assert_eq!(sh.shadow_gate_word(), 1 << 5);
        assert_eq!(
            (sh.shadow_gate_word() >> VB_SDF_MESH_MODE_SHIFT) & VB_SDF_MESH_MODE_MASK,
            VB_SDF_MESH_SHADOW_BIT,
            "the shader decodes `(word >> 5) & 3` and must see the shadow bit alone"
        );

        // AO alone: bit 6 only. NOT the same assertion as the shadow case wearing another name.
        let ao = LightingConfig { vb_sdf_mesh_ao_armed: true, ..LightingConfig::default() };
        assert_eq!(ao.shadow_gate_word(), 1 << 6);
        assert_eq!(
            (ao.shadow_gate_word() >> VB_SDF_MESH_MODE_SHIFT) & VB_SDF_MESH_MODE_MASK,
            VB_SDF_MESH_AO_BIT,
            "the shader decodes `(word >> 5) & 3` and must see the AO bit alone"
        );
        assert_eq!(sh.shadow_gate_word() & ao.shadow_gate_word(), 0, "the two SV0 bits must not overlap");

        // Both: the two bits OR together into the full sub-field.
        let both = LightingConfig {
            vb_sdf_mesh_shadow_armed: true,
            vb_sdf_mesh_ao_armed: true,
            ..LightingConfig::default()
        };
        assert_eq!(
            (both.shadow_gate_word() >> VB_SDF_MESH_MODE_SHIFT) & VB_SDF_MESH_MODE_MASK,
            VB_SDF_MESH_SHADOW_BIT | VB_SDF_MESH_AO_BIT
        );

        // Neither SV0 bit touches a neighbouring sub-field, and no neighbour touches SV0's.
        let sv0_mask = VB_SDF_MESH_MODE_MASK << VB_SDF_MESH_MODE_SHIFT;
        let neighbours = (1 << CSM_MODE_BIT) | (1 << PUNCTUAL_MODE_BIT) | (1 << DDGI_MODE_BIT);
        assert_eq!(both.shadow_gate_word() & neighbours, 0, "SV0 must not touch bits 2..4");
        let all_neighbours = LightingConfig {
            csm_shadows: true,
            punctual_shadows: true,
            ddgi_indirect: true,
            tonemapper: Tonemapper::ReinhardJodie,
            terminator_softening: 1.0,
            ..LightingConfig::default()
        };
        let neighbour_word = all_neighbours.shadow_gate_word()
            | all_neighbours.tonemap_bits()
            | all_neighbours.terminator_bits();
        assert_eq!(
            neighbour_word & sv0_mask,
            0,
            "no neighbouring sub-field may write into word-7 bits 5..6"
        );
    }

    #[test]
    fn tonemapper_default_is_aces() {
        assert_eq!(Tonemapper::default(), Tonemapper::Aces);
    }

    #[test]
    fn tonemap_bits_zero_for_aces() {
        let cfg = LightingConfig { tonemapper: Tonemapper::Aces, ..LightingConfig::default() };
        assert_eq!(cfg.tonemap_bits(), 0);
    }

    #[test]
    fn header_word7_is_zero_for_default_config() {
        // The byte-identity anchor: a default config's header word 7 (sky_diffuse.w) is
        // exactly 0.0 bits — no shadow/GI gate, no tonemap mode.
        let h = LightHeaderGpu::new(1, 0, &LightingConfig::default());
        assert_eq!(h.sky_diffuse[3].to_bits(), 0);
    }

    #[test]
    fn tonemap_mode_bits_are_independent_of_the_shadow_gate_bits() {
        let cfg = LightingConfig { tonemapper: Tonemapper::Neutral, ..LightingConfig::default() };
        let h = LightHeaderGpu::new(1, 0, &cfg);
        let word7 = h.sky_diffuse[3].to_bits();
        assert_eq!(
            (word7 >> TONEMAP_MODE_SHIFT) & TONEMAP_MODE_MASK,
            Tonemapper::Neutral as u32
        );
        // Bits 0..4 (shadow/contact/CSM/punctual/DDGI gates) must stay untouched.
        assert_eq!(word7 & 0x1F, 0, "tonemap mode must not touch the shadow/GI gate bits 0..4");
    }

    #[test]
    fn tonemap_mode_coexists_with_a_shadow_gate() {
        let cfg = LightingConfig {
            tonemapper: Tonemapper::Neutral,
            csm_shadows: true,
            ..LightingConfig::default()
        };
        let h = LightHeaderGpu::new(1, 0, &cfg);
        let word7 = h.sky_diffuse[3].to_bits();
        assert_eq!(
            (word7 >> TONEMAP_MODE_SHIFT) & TONEMAP_MODE_MASK,
            Tonemapper::Neutral as u32
        );
        assert_ne!(word7 & (1 << CSM_MODE_BIT), 0, "the csm bit must still be armed");
    }

    #[test]
    fn terminator_bits_zero_for_default() {
        let cfg = LightingConfig { terminator_softening: 0.0, ..LightingConfig::default() };
        assert_eq!(cfg.terminator_bits(), 0);
    }

    #[test]
    fn terminator_softening_bits_are_independent_of_the_shadow_and_tonemap_bits() {
        let cfg = LightingConfig { terminator_softening: 0.2, ..LightingConfig::default() };
        let h = LightHeaderGpu::new(1, 0, &cfg);
        let word7 = h.sky_diffuse[3].to_bits();
        let expected = (0.2_f32 * 255.0).round() as u32;
        assert_eq!((word7 >> TERMINATOR_SOFT_SHIFT) & TERMINATOR_SOFT_MASK, expected);
        // Shadow/GI gate bits 0..4 must stay untouched.
        assert_eq!(word7 & 0x1F, 0, "terminator softening must not touch the shadow/GI gate bits 0..4");
        // The tonemap sub-field 8..11 must stay untouched (Aces default -> 0).
        assert_eq!(
            (word7 >> TONEMAP_MODE_SHIFT) & TONEMAP_MODE_MASK,
            0,
            "terminator softening must not touch the tonemap sub-field 8..11"
        );
    }

    #[test]
    fn terminator_softening_coexists_with_a_tonemapper_and_a_shadow_gate() {
        let cfg = LightingConfig {
            terminator_softening: 0.2,
            tonemapper: Tonemapper::Neutral,
            csm_shadows: true,
            ..LightingConfig::default()
        };
        let h = LightHeaderGpu::new(1, 0, &cfg);
        let word7 = h.sky_diffuse[3].to_bits();
        let expected = (0.2_f32 * 255.0).round() as u32;
        assert_eq!((word7 >> TERMINATOR_SOFT_SHIFT) & TERMINATOR_SOFT_MASK, expected);
        assert_eq!(
            (word7 >> TONEMAP_MODE_SHIFT) & TONEMAP_MODE_MASK,
            Tonemapper::Neutral as u32,
            "the tonemap sub-field must still carry Neutral"
        );
        assert_ne!(word7 & (1 << CSM_MODE_BIT), 0, "the csm bit must still be armed");
    }

    #[test]
    fn terminator_bits_clamps_out_of_range_softening() {
        let over = LightingConfig { terminator_softening: 1.5, ..LightingConfig::default() };
        assert_eq!((over.terminator_bits() >> TERMINATOR_SOFT_SHIFT) & TERMINATOR_SOFT_MASK, 255);

        let under = LightingConfig { terminator_softening: -1.0, ..LightingConfig::default() };
        assert_eq!((under.terminator_bits() >> TERMINATOR_SOFT_SHIFT) & TERMINATOR_SOFT_MASK, 0);
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

    /// VB-P1b-0 bit-exactness: the PRODUCTION path (`sync_cluster_light_gate` writing
    /// `LightingConfig`'s derived cluster fields, then `LightHeaderGpu::new` reading them)
    /// MUST produce the identical `cluster_params` lane the direct, test/oracle
    /// `LightHeaderGpu::new_clustered` constructor produces for the SAME `ClusterConfig` — the
    /// load-bearing invariant the cull (`cluster_cull.hlsl`) and the resolve
    /// (`vb_resolve.comp.hlsl`) both rely on to build valid, in-range froxel indices.
    #[test]
    fn sync_cluster_light_gate_matches_new_clustered_bit_for_bit() {
        use boyko_ecs::ecs::core::app::App;

        use crate::render_path_config::{
            GeometryLegs, RenderPath, RenderPathConfig, RenderPathConsumers, RenderPathDeviceCaps,
            resolve_render_path,
        };

        let cluster = ClusterConfig::default();
        // The REAL boot resolve of a VisibilityBuffer scene that wants clusters — the SAME
        // production entry point `boyko_app::runner` calls, not a hand-built literal (W1 fix,
        // code review): `sync_cluster_light_gate` now gates on `froxel_light_cull`, not
        // `clusters_enabled` alone, so the test must arm the SAME resolved carrier.
        let (resolved_path, _) = resolve_render_path(
            &RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh },
            RenderPathConsumers { clusters_wanted: true, ..Default::default() },
            RenderPathDeviceCaps::new(true),
        );
        assert!(
            resolved_path.froxel_light_cull,
            "test setup invariant: VisibilityBuffer + clusters_wanted must arm froxel_light_cull"
        );

        let mut app = App::new();
        app.insert_resource(cluster);
        app.insert_resource(resolved_path);
        app.insert_resource(LightingConfig { clusters_enabled: true, ..LightingConfig::default() });
        app.insert_resource(LightTableDirty(false));
        app.world_mut().run_system(sync_cluster_light_gate);

        let synced_cfg = *app.world().resource::<LightingConfig>();
        let got = LightHeaderGpu::new(2, 1, &synced_cfg);

        let direct_cfg = LightingConfig { clusters_enabled: true, ..LightingConfig::default() };
        let want = LightHeaderGpu::new_clustered(2, 1, &direct_cfg, &cluster);
        // Full-header equality (O3, code review): pins that the `new_clustered` ->
        // `pack_cluster_params` refactor (and `sync_cluster_light_gate`'s production path)
        // perturbs NOTHING outside the cluster lane — not just that lane in isolation.
        assert_eq!(
            got, want,
            "sync_cluster_light_gate's packed header must equal new_clustered's bit-for-bit"
        );

        // Non-zero dims when VB-froxel-armed: `cluster_z_slice`/`cluster_linear_index`
        // (light_table.hlsli) underflow to an out-of-range index when any dim is 0 — this is
        // the exact regression VB-P1b-0 fixes (pre-fix, `LightHeaderGpu::new` always packed
        // all-zero dims here).
        let d = synced_cfg.cluster_packed_dims;
        assert_ne!(d & 0xFF, 0, "dim_x must be nonzero when froxel_light_cull is armed");
        assert_ne!((d >> 8) & 0xFF, 0, "dim_y must be nonzero when froxel_light_cull is armed");
        assert_ne!((d >> 16) & 0xFF, 0, "dim_z must be nonzero when froxel_light_cull is armed");
        assert_ne!(synced_cfg.cluster_z_scale, 0.0, "z_scale must be nonzero when froxel_light_cull is armed");
    }

    /// The 0%-gate half of the VB-P1b-0 contract: `sync_cluster_light_gate` must zero the
    /// header's cluster lane regardless of what `ClusterConfig` carries, whenever
    /// `froxel_light_cull` is unarmed (here: the never-resolved default carrier, `clusters_enabled
    /// == false`) — a non-default `ClusterConfig` alone must never leak its geometry into the
    /// header of an unarmed scene.
    #[test]
    fn sync_cluster_light_gate_zeroes_the_lane_when_disabled() {
        use boyko_ecs::ecs::core::app::App;

        use crate::render_path_config::ResolvedRenderPath;

        // A deliberately NON-default grid, to prove the gate does not merely happen to zero
        // the default — it actively zeroes REGARDLESS of `ClusterConfig`'s contents.
        let cluster = ClusterConfig { dim_x: 8, dim_y: 4, dim_z: 12, ..ClusterConfig::default() };
        let mut app = App::new();
        app.insert_resource(cluster);
        // `ResolvedRenderPath::default()` == Deferred + Both, no consumers armed —
        // `froxel_light_cull == false` by construction.
        app.insert_resource(ResolvedRenderPath::default());
        app.insert_resource(LightingConfig::default()); // clusters_enabled == false
        app.insert_resource(LightTableDirty(false));
        app.world_mut().run_system(sync_cluster_light_gate);

        let synced_cfg = *app.world().resource::<LightingConfig>();
        assert_eq!(synced_cfg.cluster_z_scale, 0.0);
        assert_eq!(synced_cfg.cluster_z_bias, 0.0);
        assert_eq!(synced_cfg.cluster_packed_dims, 0);

        let h = LightHeaderGpu::new(2, 1, &synced_cfg);
        assert_eq!(h.cluster_params, [0.0, 0.0, 0.0, 0.0], "unarmed header stays the 0%-gate anchor");
    }

    /// The W1 regression guard (code review): a NON-VB path whose `LightingConfig::clusters_enabled`
    /// is (mistakenly, or via `ClusterSelectMode::Auto` banding) `true` must STILL leave the
    /// header's cluster dims at `0` — `sync_cluster_light_gate` gates the armed write on
    /// `ResolvedRenderPath::froxel_light_cull`, which is `false` for every `RenderPath` other
    /// than `VisibilityBuffer`, REGARDLESS of `clusters_enabled`. Without this, `deferred_pbr.hlsl`
    /// (which reads this lane unconditionally) and ForwardPlus's `forward_opaque_froxel.fs.hlsl`
    /// would compute a valid-looking-but-WRONG cluster index into their `ClusterGrid`/
    /// `LightIndexList` bindings, which fall back to the light-table buffer as a placeholder on
    /// every current Deferred/ForwardPlus boot (a separate, tracked hardening rung — this test
    /// only pins that VB-P1b-0 does not make that pre-existing hazard MORE reachable).
    #[test]
    fn sync_cluster_light_gate_zeroes_the_lane_on_a_non_vb_path_even_when_clusters_enabled() {
        use boyko_ecs::ecs::core::app::App;

        use crate::render_path_config::{
            GeometryLegs, RenderPath, RenderPathConfig, RenderPathConsumers, RenderPathDeviceCaps,
            resolve_render_path,
        };

        let cluster = ClusterConfig::default();
        // A Deferred scene that ALSO wants clusters (an owner mistake, or what Auto-banding
        // would produce with no RenderPath awareness) — `froxel_light_cull` is VB-only by
        // construction, so it stays `false` here regardless of `clusters_wanted`.
        let (resolved_path, _) = resolve_render_path(
            &RenderPathConfig { path: RenderPath::Deferred, legs: GeometryLegs::Both },
            RenderPathConsumers { clusters_wanted: true, ..Default::default() },
            RenderPathDeviceCaps::new(true),
        );
        assert!(
            !resolved_path.froxel_light_cull,
            "test setup invariant: Deferred must never arm froxel_light_cull, even with clusters_wanted"
        );

        let mut app = App::new();
        app.insert_resource(cluster);
        app.insert_resource(resolved_path);
        app.insert_resource(LightingConfig { clusters_enabled: true, ..LightingConfig::default() });
        app.insert_resource(LightTableDirty(false));
        app.world_mut().run_system(sync_cluster_light_gate);

        let synced_cfg = *app.world().resource::<LightingConfig>();
        assert_eq!(synced_cfg.cluster_z_scale, 0.0, "non-VB path: z_scale must stay 0 despite clusters_enabled");
        assert_eq!(synced_cfg.cluster_z_bias, 0.0, "non-VB path: z_bias must stay 0 despite clusters_enabled");
        assert_eq!(
            synced_cfg.cluster_packed_dims, 0,
            "non-VB path: dims must stay 0 despite clusters_enabled"
        );

        // Word 15 (`clusters_enabled`) is STILL packed verbatim by `LightHeaderGpu::new` (that
        // bit is not this gate's concern) — the dims-only scoping is what this test pins.
        let h = LightHeaderGpu::new(2, 1, &synced_cfg);
        assert_eq!(h.cluster_params, [0.0, 0.0, 0.0, f32::from_bits(1)]);
    }

    // ---- VB-SV0 §S4 arming gate ------------------------------------------------------------

    /// A REAL boot resolve of the rung-S1 fixture configuration (`VisibilityBuffer × Both`, the
    /// runner's hardwired `sdf_shadows_wanted: true`, no hwrt), plus the optional consumers the
    /// §S4 variant rows need. Built through the production entry point rather than as a literal,
    /// so a change to the arming rules reaches these tests instead of being mirrored past them.
    fn sv0_resolved(
        legs: crate::render_path_config::GeometryLegs,
        ssao_on: bool,
        hwrt: bool,
    ) -> crate::render_path_config::ResolvedRenderPath {
        use crate::render_path_config::{
            RenderPath, RenderPathConfig, RenderPathConsumers, RenderPathDeviceCaps,
            resolve_render_path,
        };
        let (resolved, _) = resolve_render_path(
            &RenderPathConfig { path: RenderPath::VisibilityBuffer, legs },
            RenderPathConsumers {
                sdf_shadows_wanted: true,
                ssao_on,
                hwrt_denoise_or_vis_on: hwrt,
                ..Default::default()
            },
            RenderPathDeviceCaps::new(true),
        );
        resolved
    }

    /// Runs `sync_sv0_light_gate` over one (`resolved`, request) pair and returns the resolved
    /// config plus whether the light table was dirtied.
    ///
    /// Also asserts, on EVERY call, that the owner's two request fields came back untouched — the
    /// code-review P2-c property. Placed here rather than in one dedicated test so no future case
    /// can be added that quietly reintroduces the in-place clamp.
    fn run_sv0_gate(
        resolved: crate::render_path_config::ResolvedRenderPath,
        request_shadow: bool,
        request_ao: bool,
    ) -> (LightingConfig, bool) {
        use boyko_ecs::ecs::core::app::App;

        let mut app = App::new();
        app.insert_resource(resolved);
        app.insert_resource(LightingConfig {
            vb_sdf_mesh_shadow: request_shadow,
            vb_sdf_mesh_ao: request_ao,
            ..LightingConfig::default()
        });
        app.insert_resource(LightTableDirty(false));
        app.world_mut().run_system(sync_sv0_light_gate);
        let cfg = *app.world().resource::<LightingConfig>();
        assert_eq!(
            (cfg.vb_sdf_mesh_shadow, cfg.vb_sdf_mesh_ao),
            (request_shadow, request_ao),
            "the gate must never write the OWNER's request fields — an in-place clamp makes a \
             per-frame owner writer re-fold the whole light table every frame"
        );
        (cfg, app.world().resource::<LightTableDirty>().0)
    }

    /// **Code-review P2-c, the cost the separation buys.** An owner who re-asserts the request
    /// EVERY frame on a boot that cannot carry SV0 dirties the light table exactly zero times.
    ///
    /// With the request and the resolved value fused into one field this test cannot pass: the
    /// gate clears the field, the owner re-sets it, and every subsequent frame sees a change and
    /// re-folds + re-uploads the entire table — a silent per-frame cost whose only symptom is
    /// throughput.
    #[test]
    fn sv0_gate_does_not_refold_under_a_per_frame_owner_writer() {
        use boyko_ecs::ecs::core::app::App;

        use crate::render_path_config::GeometryLegs;

        // VB x Mesh: structurally unarmable, so the request can never be honoured — the exact
        // configuration the fused design would have re-folded on forever.
        let resolved = sv0_resolved(GeometryLegs::Mesh, false, false);
        assert!(!resolved.vb_sdf_mesh_armable(), "test setup: this boot must NOT be armable");

        let mut app = App::new();
        app.insert_resource(resolved);
        app.insert_resource(LightingConfig::default());
        app.insert_resource(LightTableDirty(false));

        for frame in 0..8 {
            // The owner's per-frame write, verbatim: re-assert the request, every frame.
            app.world_mut().resource_mut::<LightingConfig>().vb_sdf_mesh_shadow = true;
            app.world_mut().resource_mut::<LightingConfig>().vb_sdf_mesh_ao = true;
            app.world_mut().run_system(sync_sv0_light_gate);
            assert!(
                !app.world().resource::<LightTableDirty>().0,
                "frame {frame}: an unhonourable request must never dirty the light table"
            );
            let cfg = *app.world().resource::<LightingConfig>();
            assert!(!cfg.vb_sdf_mesh_shadow_armed && !cfg.vb_sdf_mesh_ao_armed);
            assert_eq!(cfg.shadow_gate_word(), 0);
        }
    }

    /// **The 0%-gate, and the reason no shipped golden moves at rung S4.** An armable boot that
    /// does NOT request SV0 keeps both bits clear and the header word at its pre-SV0 anchor.
    ///
    /// This is the assertion that makes "every existing golden stays byte-identical" structural:
    /// `[vb_both]`, `[vb_both_taa]` and both S1 fixtures all resolve `VB × Both` with the
    /// runner's hardwired `sdf_shadows_wanted`, i.e. they are all CAPABLE. What keeps them
    /// unarmed is only that they never set the request — so the gate must never set it for them.
    #[test]
    fn sv0_gate_leaves_an_unrequesting_armable_boot_at_the_zero_gate() {
        use crate::render_path_config::GeometryLegs;

        let resolved = sv0_resolved(GeometryLegs::Both, false, false);
        assert!(resolved.vb_sdf_mesh_armable(), "test setup: VB x Both must be SV0-armable");

        let (cfg, dirty) = run_sv0_gate(resolved, false, false);
        assert!(!cfg.vb_sdf_mesh_shadow_armed);
        assert!(!cfg.vb_sdf_mesh_ao_armed);
        assert_eq!(cfg.shadow_gate_word(), 0, "an unrequesting boot packs the pre-SV0 word");
        assert!(!dirty, "a no-op resolve must not dirty the light table");
    }

    /// Each term arms ON ITS OWN. SV0 is two independently-gated terms and every gate written
    /// against it as one feature was satisfiable by the shadow half alone — so the host gate is
    /// required to pass each bit through without the other.
    #[test]
    fn sv0_gate_passes_each_requested_term_through_independently() {
        use crate::render_path_config::GeometryLegs;

        let resolved = sv0_resolved(GeometryLegs::Both, false, false);

        let (shadow_only, _) = run_sv0_gate(resolved, true, false);
        assert!(shadow_only.vb_sdf_mesh_shadow_armed);
        assert!(!shadow_only.vb_sdf_mesh_ao_armed);
        assert_eq!(
            (shadow_only.shadow_gate_word() >> VB_SDF_MESH_MODE_SHIFT) & VB_SDF_MESH_MODE_MASK,
            VB_SDF_MESH_SHADOW_BIT,
            "the shader must decode sv0_mode == VB_SDF_MESH_SHADOW_BIT (gate ii-a)"
        );

        let (ao_only, _) = run_sv0_gate(resolved, false, true);
        assert!(!ao_only.vb_sdf_mesh_shadow_armed);
        assert!(ao_only.vb_sdf_mesh_ao_armed);
        assert_eq!(
            (ao_only.shadow_gate_word() >> VB_SDF_MESH_MODE_SHIFT) & VB_SDF_MESH_MODE_MASK,
            VB_SDF_MESH_AO_BIT,
            "the shader must decode sv0_mode == VB_SDF_MESH_AO_BIT (gate ii-b)"
        );

        let (both, dirty) = run_sv0_gate(resolved, true, true);
        assert!(both.vb_sdf_mesh_shadow_armed && both.vb_sdf_mesh_ao_armed);
        // An HONOURED request moves `_armed` false→true, so the table MUST be re-folded — the
        // header is what carries the two bits to the shader, and a stale pack would render the
        // armed frame unarmed.
        assert!(dirty, "arming a term must dirty the light table so the header is re-packed");
    }

    /// The resolve clears BOTH bits on every structurally unarmable boot, including the hwrt
    /// configuration that selects §S4's rows 9-10.
    ///
    /// The `dirty` assertion matters, and it is the OPPOSITE of the armed case: `_armed` starts
    /// `false` and stays `false`, so nothing is written and the table is never re-folded. That is
    /// the whole point of resolving into a separate field (code-review P2-c) — an unhonourable
    /// request costs nothing, however often it is re-asserted.
    #[test]
    fn sv0_gate_clears_a_request_the_boot_cannot_carry() {
        use crate::render_path_config::{GeometryLegs, ResolvedRenderPath, ShadowSources};

        // Rows 9-10: `ssao_on` selects the split tail, the hwrt carrier selects its `_hwrt`
        // variants — and displaces `SDF_SOFT_MARCH`, which is exactly why SV0 cannot ride them.
        let hwrt = sv0_resolved(GeometryLegs::Both, true, true);
        assert!(hwrt.shadow.contains(ShadowSources::HWRT_VIS), "test setup: the hwrt rows");
        assert!(!hwrt.vb_sdf_mesh_armable());
        let (cfg, dirty) = run_sv0_gate(hwrt, true, true);
        assert!(!cfg.vb_sdf_mesh_shadow_armed && !cfg.vb_sdf_mesh_ao_armed, "rows 9-10 never arm");
        assert_eq!(cfg.shadow_gate_word() >> VB_SDF_MESH_MODE_SHIFT & VB_SDF_MESH_MODE_MASK, 0);
        assert!(!dirty, "a request that resolves to the already-published OFF state writes nothing");

        // VB x Mesh (no field to march) and VB x Sdf (no mesh pixels to shade).
        for legs in [GeometryLegs::Mesh, GeometryLegs::Sdf] {
            let resolved = sv0_resolved(legs, false, false);
            assert!(!resolved.vb_sdf_mesh_armable(), "{legs:?} must not be SV0-armable");
            let (cfg, _) = run_sv0_gate(resolved, true, true);
            assert!(
                !cfg.vb_sdf_mesh_shadow_armed && !cfg.vb_sdf_mesh_ao_armed,
                "{legs:?} must never arm"
            );
        }

        // The never-resolved default carrier (Deferred + Both) — the state a world that never
        // booted the windowed runner carries.
        let (cfg, _) = run_sv0_gate(ResolvedRenderPath::default(), true, true);
        assert!(
            !cfg.vb_sdf_mesh_shadow_armed && !cfg.vb_sdf_mesh_ao_armed,
            "Deferred must never arm SV0"
        );
    }

    /// **The DISARM direction.** A boot whose capability goes away after a term was armed must
    /// have the header bit cleared and the table re-folded — otherwise the shader keeps executing
    /// a block whose producer is gone.
    ///
    /// Reachable state, not a hypothetical: `_armed` is `Resource` state that survives whatever
    /// the owner does to the request, so an owner who withdraws the request mid-run lands here.
    #[test]
    fn sv0_gate_disarms_and_refolds_when_the_request_is_withdrawn() {
        use boyko_ecs::ecs::core::app::App;

        use crate::render_path_config::GeometryLegs;

        let mut app = App::new();
        app.insert_resource(sv0_resolved(GeometryLegs::Both, false, false));
        app.insert_resource(LightingConfig {
            vb_sdf_mesh_shadow: true,
            vb_sdf_mesh_ao: true,
            ..LightingConfig::default()
        });
        app.insert_resource(LightTableDirty(false));

        app.world_mut().run_system(sync_sv0_light_gate);
        assert!(app.world().resource::<LightingConfig>().vb_sdf_mesh_shadow_armed);
        assert!(app.world().resource::<LightTableDirty>().0, "arming re-folds");

        app.world_mut().resource_mut::<LightTableDirty>().0 = false;
        app.world_mut().resource_mut::<LightingConfig>().vb_sdf_mesh_shadow = false;
        app.world_mut().resource_mut::<LightingConfig>().vb_sdf_mesh_ao = false;
        app.world_mut().run_system(sync_sv0_light_gate);

        let cfg = *app.world().resource::<LightingConfig>();
        assert!(!cfg.vb_sdf_mesh_shadow_armed && !cfg.vb_sdf_mesh_ao_armed);
        assert_eq!(cfg.shadow_gate_word(), 0, "the header returns to the pre-SV0 anchor");
        assert!(app.world().resource::<LightTableDirty>().0, "disarming must re-fold too");
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
