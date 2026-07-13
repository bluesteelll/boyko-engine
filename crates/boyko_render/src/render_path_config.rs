//! Multi-paradigm render-path plan, rung R1 — the config surface + boot-lock resolver.
//!
//! Principle 0: ECS-native — [`RenderPathConfig`] is a `#[derive(Resource)]` singleton (the
//! cold owner-set config, NOT a side `std::Vec`/`HashMap`), mirroring [`AaConfig`](crate::aa_config::AaConfig)
//! / [`SsaoConfig`](crate::ssao_config::SsaoConfig). [`ResolvedRenderPath`] is its derived
//! carrier, but UNLIKE `ResolvedAa`/`ResolvedSsao` it has NO per-frame policy system: Decision 1
//! (plan §"Key decisions") commits path/legs/the pre-light-consumer set exactly ONCE, at
//! `WindowHost::boot` (the `host.ssaa_armed` precedent) — [`resolve_render_path`] is called a
//! single time by `boyko_app::runner`, never re-run per frame. A live per-frame path/leg toggle
//! is FORBIDDEN by design: it would re-allocate fixed-size images/pipelines mid-stream (the same
//! reason `ssaa_armed` is a boot commitment, not a per-frame read).
//!
//! # The 0%-gate
//!
//! [`RenderPathConfig::default`] is `Deferred + Both` — byte-identical to today (the shipped
//! single hardcoded deferred pipeline). [`resolve_render_path`] of the default config, default
//! [`RenderPathConsumers`], and default [`RenderPathDeviceCaps`] resolves to `Deferred + Both`
//! with every derived flag structurally `false`/off and NO degrades — the byte-identity anchor
//! the plan's golden gates rest on.
//!
//! # Rung-staged degrades (plan §H)
//!
//! Only `Deferred` is implemented as of R1 — [`FORWARD_IMPLEMENTED`], [`FORWARD_PLUS_IMPLEMENTED`],
//! [`VB_IMPLEMENTED`], and [`SDF_FORWARD_IMPLEMENTED`] are all `false`, so ANY non-Deferred
//! [`RenderPath`] request degrades to `Deferred` today (a [`RenderPathDegrade::PathNotYetImplemented`]
//! reason). Each const flips `true` as its rung lands (R4/R5/R8/R-SDFFWD respectively) — no other
//! code in this module changes; the degrade ladder ([`resolve_render_path`]) and the pure rule
//! set ([`resolve_rules`]) are ALREADY correct for the fully-landed plan, tested directly against
//! the rule (not gated behind the rung consts) so they are live today, not dead until later rungs.
//!
//! Rung R2 added a single combined `DEFERRED_LEG_DISABLE_IMPLEMENTED` guard (`false`); rung R3
//! SPLITS it into [`DEFERRED_SDF_ONLY_IMPLEMENTED`] / [`DEFERRED_MESH_ONLY_IMPLEMENTED`] — one
//! const per leg, the SAME "rung-staged const per landed capability" idiom every other flag in
//! this module already uses ([`FORWARD_IMPLEMENTED`] etc.) — because the two legs landed at
//! DIFFERENT confidence levels (plan §H R3 row + the R3 rung audit):
//!
//! - **`Deferred × Sdf`** (mesh raster leg off): [`DEFERRED_SDF_ONLY_IMPLEMENTED`] is `true` —
//!   the raster gbuffer pass is skipped, a `mesh_depth_neutral_clear` pass
//!   (`boyko_rhi_vulkan::present::graph_bridge`) replaces its depth-clear producer so the
//!   marcher's mesh-depth sample deterministically reads the far-plane sentinel (the SAME
//!   "no mesh in the scene" code path the marcher already handles byte-identically), reusing
//!   `sdf_gbuffer_composite.hlsl` UNCHANGED (no `HAS_MESH` compiled variant this rung — a
//!   deliberate, documented deviation from the plan's literal text; see the R3 rung report for
//!   the risk analysis). Every MESH-shadow producer the raster pass fed is ALSO structurally
//!   suppressed — CSM cascade depth, the punctual spot/point atlas depth, and (under `hwrt`) the
//!   TLAS pack/build + the shadow_vis/à-trous/temporal denoise chain — because mesh-shadow
//!   producers are mesh-leg-owned (they rasterize/trace MESH casters only; the SDF leg's shadow
//!   is the marcher's own baked soft march). `GpuSceneBundles::scene()`
//!   (`boyko_app::gpu_scene`) is the single scene-assembly seam that gates this (capability =
//!   component presence, not a runtime flag); `declare_deferred_graph` carries a
//!   `debug_assert!` belt-and-braces check the seam was not missed. VRAM/boot-time pipeline
//!   creation for the mesh raster leg (vertex buffer, instance rings, raster pipelines) is NOT
//!   yet gated this rung — an honest, documented follow-up (see the R3 rung report).
//! - **`Deferred × Mesh`** (SDF leg off): [`DEFERRED_MESH_ONLY_IMPLEMENTED`] stays `false` — the
//!   R3 audit found the marcher is the SOLE producer of the `gViewT` lane for MESH-owned pixels
//!   too (`sdf_gbuffer_composite.hlsl`'s `gViewT[...] = (own_pixel && mask==1.0) ? t : (has_mesh
//!   ? t_mesh : 1.0e30)`, both terminal write sites), and every `gViewT` consumer (the resolve's
//!   `P = ro + rd*view_t` reconstruction, SSAO's `view_t` mesh/SDF classification) reads it
//!   UNCONDITIONALLY under `mask == 1` — mesh pixels included, not just SDF ones. Skipping the
//!   marcher entirely (the plan's O2 "verified" mesh-only design) therefore leaves `gViewT`
//!   wholly unwritten (no producer, no clear) for a leg the plan's own "Untouched" list forbids
//!   patching via `gbuffer_mrt.fs.hlsl` (a new MRT write) or `deferred_pbr.hlsl`'s frozen
//!   binding-count sets (a new depth binding) — an architecture-level gap requiring an approved
//!   producer-replacement design, not a developer-turn workaround. This const stays `false`
//!   until that design lands (see the R3 rung report's Phase-1 audit finding (a)).
//!
//! Both rules fire ONLY when the FINAL (post-path-demotion) `path == Deferred`; the
//! legs-collapse-to-`Mesh` rule below fires ONLY when the final `path != Deferred` — the two
//! groups are mutually exclusive by construction (see [`degrade_ladder`]'s doc), so they never
//! co-fire in the same call.
//!
//! # Rev 5 — the single `pre_light_consumers` predicate (MANDATORY)
//!
//! [`resolve_rules`] computes ONE local union — `ssao ∥ ddgi ∥ shadow_denoise_spatial ∥
//! shadow_temporal ∥ ssr` — and that union is the SOLE trigger for
//! [`ResolvedRenderPath::needs_depth_prepass`] (Forward), [`ResolvedRenderPath::mesh_geo_shade_split`]
//! (VisibilityBuffer), and [`ResolvedRenderPath::sdf_geo_shade_split`] (the SDF leg) alike.
//! `shadow_temporal` is a MOTION-only pre-light consumer (it reads motion + `gViewT`, never
//! normal — `graph_bridge.rs:1129`); gating the three flags on a NORMAL-only consumer union
//! would leave `Forward + hwrt shadows + ShadowDenoiseMode::Temporal + no SSAO/DDGI/SSR` reading
//! frame-stale motion (the W4 hole). Folding `shadow_temporal` into the SAME union as the NORMAL
//! consumers is what closes it — see [`resolve_rules`]'s doc for the exact formula.
//!
//! # Threading (plan §A "Threading")
//!
//! `boyko_app::runner` reads [`RenderPathConfig`] + the consumer configs via `world.try_resource`
//! (graceful default), calls [`resolve_render_path`] ONCE at boot, stores the
//! [`ResolvedRenderPath`] on the host struct beside `ssaa_armed`, and threads the plain value into
//! `GpuSceneBundles::scene()` → a field on `GBufferScene` — DEAD-BUT-THREADED at R1 (nothing reads
//! it downstream yet; R2 wires the declarator dispatch).

use boyko_macros::Resource;

// ---- rung-staged implementation flags (plan §H) ---------------------------------------

/// Whether the `Forward` [`RenderPath`] has a landed declarator/pipeline yet. Lifted `true` at
/// rung R4. `false` today ⇒ every `Forward` request degrades to `Deferred`
/// ([`RenderPathDegrade::PathNotYetImplemented`]).
const FORWARD_IMPLEMENTED: bool = false;

/// Whether the `ForwardPlus` [`RenderPath`] has a landed declarator/pipeline yet. Lifted `true`
/// at rung R5.
const FORWARD_PLUS_IMPLEMENTED: bool = false;

/// Whether the `VisibilityBuffer` [`RenderPath`] has a landed declarator/pipeline yet. Lifted
/// `true` at rung R8.
const VB_IMPLEMENTED: bool = false;

/// Whether the SDF-forward-march leg (Decision 6, the fused march-then-shade or the geo/shade
/// split under a non-Deferred path) has landed yet. Lifted `true` at rung R-SDFFWD. `false`
/// today ⇒ any non-Deferred path requesting a non-[`GeometryLegs::Mesh`] leg set (`Both` or
/// `Sdf`) degrades that leg set to `Mesh` ([`RenderPathDegrade::LegsCollapsedToMeshPreSdfForward`]).
const SDF_FORWARD_IMPLEMENTED: bool = false;

/// Whether `Deferred × Sdf` (the mesh raster leg off) has landed. `true` as of rung R3: the
/// `mesh_depth_neutral_clear` pass (`boyko_rhi_vulkan::present::graph_bridge`) replaces the
/// raster pass's depth-clear, giving the (UNCHANGED) marcher a deterministic "no mesh" depth to
/// sample every frame. `false` ⇒ a `Deferred × Sdf` request degrades its LEGS to
/// [`GeometryLegs::Both`] ([`RenderPathDegrade::DeferredLegDisableNotYetImplemented`]). See this
/// module's doc for the full R3 rationale (incl. why the plan's literal `HAS_MESH` compiled
/// variant was deliberately not built this rung).
const DEFERRED_SDF_ONLY_IMPLEMENTED: bool = true;

/// Whether `Deferred × Mesh` (the SDF leg off) has landed. Still `false` — the R3 audit found the
/// marcher is the sole producer of `gViewT` for MESH-owned pixels too, and skipping the marcher
/// entirely (the plan's O2 mesh-only design) leaves every `gViewT` consumer reading an unwritten
/// image. See this module's doc for the full finding; this const stays `false` until an approved
/// gViewT producer-replacement design lands. `false` ⇒ a `Deferred × Mesh` request degrades its
/// LEGS to [`GeometryLegs::Both`] ([`RenderPathDegrade::DeferredLegDisableNotYetImplemented`]).
const DEFERRED_MESH_ONLY_IMPLEMENTED: bool = false;

// ---- RenderPath / GeometryLegs (the owner-set knobs; capability is structural) --------

/// The render-path technique the owner sets on [`RenderPathConfig`] — chooses *how geometry
/// becomes lit pixels*. `#[repr(u32)]` so it can be forwarded to the backend as a stable mode
/// word (mirrors [`AaMode`](crate::aa_config::AaMode)'s discriminant discipline).
///
/// Only [`Deferred`](RenderPath::Deferred) is implemented as of R1 (see the rung-staged
/// consts above); every other variant is a valid owner request that degrades to `Deferred` at
/// boot until its rung lands (`resolve_render_path`'s degrade ladder, never a panic).
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RenderPath {
    /// Fat-MRT G-buffer + compute resolve; custom-linear depth. The current shipped pipeline —
    /// byte-identical to today. The DEFAULT, so a world that never inserts a non-default
    /// [`RenderPathConfig`] is byte-identical.
    #[default]
    Deferred = 0,
    /// Raster fragment shader shades inline against every light; hardware reverse-Z depth (no
    /// gbuffer write/read bandwidth). Wins at few lights / small scenes / low overdraw.
    Forward = 1,
    /// Forward + a depth prepass with `DEPTH_EQUAL` early-Z zero-overdraw + froxel light reuse.
    /// Wins at many lights with forward variety.
    ForwardPlus = 2,
    /// `R32G32_UINT` id raster + compute shade against a bindless per-mesh geometry table
    /// (Decision 0). Needs `shaderStorageBufferArrayNonUniformIndexing` (degrades to `Deferred`
    /// at boot when absent). Wins at sub-pixel triangle density.
    VisibilityBuffer = 3,
}

/// The geometry-producer leg set the owner sets on [`RenderPathConfig`] — chooses *which
/// geometry producers exist*. A disabled leg allocates no images, builds no pipelines, records
/// no passes, binds no descriptors, and sets no extra buffer-usage bits (the plan's "zero-cost
/// leg toggle" invariant). `#[repr(u32)]`, mirrors [`RenderPath`]'s discriminant discipline.
///
/// Deliberately has **no** `None` variant: an app with no 3D geometry does not compose the
/// render plugin at all (the `world.try_resource` graceful-degrade precedent), so there is no
/// "both legs off" state to represent here.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GeometryLegs {
    /// Both the mesh raster leg and the SDF marched leg exist. The DEFAULT — byte-identical to
    /// today's shipped hybrid mesh+SDF Deferred pipeline.
    #[default]
    Both = 0,
    /// Only the mesh raster leg exists — zero SDF cost (no marcher dispatch, no brick/edit-list
    /// buffers touched beyond their boot-seed placeholders).
    Mesh = 1,
    /// Only the SDF marched leg exists — zero mesh cost (no vertex pipelines, no instance
    /// rings, no per-mesh geometry-table slots).
    Sdf = 2,
}

impl GeometryLegs {
    /// Whether the mesh raster leg is present — every variant except [`Sdf`](GeometryLegs::Sdf).
    #[inline]
    pub const fn has_mesh(self) -> bool {
        !matches!(self, GeometryLegs::Sdf)
    }

    /// Whether the SDF marched leg is present — every variant except [`Mesh`](GeometryLegs::Mesh).
    #[inline]
    pub const fn has_sdf(self) -> bool {
        !matches!(self, GeometryLegs::Mesh)
    }
}

// ---- RenderPathConfig (the owner-set Resource — mirrors AaConfig/SsaoConfig) ----------

/// The global render-path config — a `World`-singleton Resource the owner sets, structural (no
/// redundant `enabled: bool`): the [`RenderPath`] enum IS the pass-topology choice and
/// [`GeometryLegs`] IS the producer-set choice. `#[derive(Resource)]` via [`boyko_macros::Resource`]
/// (the same derive path `AaConfig`/`SsaoConfig` use).
#[derive(Resource, Clone, Copy, Debug)]
pub struct RenderPathConfig {
    /// The owner-requested render-path technique. Degrades to [`RenderPath::Deferred`] at boot
    /// if its rung has not landed yet (never a panic — see this module's doc).
    pub path: RenderPath,
    /// The owner-requested geometry-producer leg set. May degrade to [`GeometryLegs::Mesh`] at
    /// boot if a non-Deferred path is requested before the SDF-forward-march rung lands.
    pub legs: GeometryLegs,
}

impl Default for RenderPathConfig {
    #[inline]
    fn default() -> Self {
        // Deferred + Both == today (the byte-identity anchor): a default world resolves to
        // exactly the shipped hybrid mesh+SDF deferred pipeline.
        Self { path: RenderPath::Deferred, legs: GeometryLegs::Both }
    }
}

// ---- DepthKind / ThinAuxMask / ShadowSources (the resolved carrier's sub-vocabulary) ---

/// The depth-buffer contract a resolved path uses — Decision 4. `#[repr(u32)]` for a compact,
/// stable discriminant.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DepthKind {
    /// Deferred's existing custom-linear depth (both camera-mode-selected literals,
    /// `MESH_DEPTH_T_MAX` / `GBUFFER_T_MAX`) — referenced, never edited by any other path.
    CustomLinear = 0,
    /// Standard hardware reverse-Z depth on a separate allocation (Forward/ForwardPlus/
    /// VisibilityBuffer) — no `SV_Depth`, early-Z stays live; consumers reconstruct view
    /// position via the camera inverse-projection.
    HardwareReverseZ = 1,
}

/// The thin cross-pass auxiliary channel set (§D) a resolved path arms structurally — a plain
/// `#[repr(transparent)]` `u32` newtype (this crate carries no `bitflags` dependency; matching
/// the project's zero-extra-dep discipline over pulling one in for a 3-bit set).
///
/// Depth (`depth` D32 for mesh pixels, `gViewT` for SDF pixels) is ALWAYS present and is
/// therefore not a flag here.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ThinAuxMask(u32);

impl ThinAuxMask {
    /// No thin-aux channel armed.
    pub const NONE: Self = Self(0);
    /// Octahedral world/view normal — armed for SSAO/DDGI/shadow-denoise-spatial/SSR.
    pub const NORMAL: Self = Self(1 << 0);
    /// 8-bit roughness, packed in `thin_normal.BA` — armed for SSR (future).
    pub const ROUGHNESS: Self = Self(1 << 1);
    /// RG16F screen-space motion — armed for TAA / shadow-temporal.
    pub const MOTION: Self = Self(1 << 2);

    /// Whether every bit of `other` is set in `self`.
    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Returns `self` with `other`'s bits set.
    #[inline]
    pub const fn insert(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The raw bit pattern — the GPU/host packing seam (mirrors how `ResolvedCsm`/`ResolvedShadowAtlas`
    /// pack their mode words for UBO upload).
    #[inline]
    pub const fn bits(self) -> u32 {
        self.0
    }
}

/// The shadow-visibility source set (Decision 7 / W2) a resolved path arms structurally — a
/// plain `#[repr(transparent)]` `u32` newtype, same discipline as [`ThinAuxMask`]. Multiple
/// sources combine (exactly as `deferred_pbr.hlsl` combines them today) into one visibility
/// term fed to `eval_pbr_direct`.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ShadowSources(u32);

impl ShadowSources {
    /// No shadow source armed.
    pub const NONE: Self = Self(0);
    /// Directional cascade shadow maps (PCF, bindings 12/13).
    pub const CSM: Self = Self(1 << 0);
    /// Sparse spot/point shadow atlas (bindings 14/15).
    pub const PUNCTUAL_ATLAS: Self = Self(1 << 1);
    /// Inline SDF soft-march visibility (`sdf_soft_shadow_ranged`, edit-list) — needs the SDF
    /// leg; Deferred's default non-hwrt SDF soft shadow, restored to Forward/VB by Decision 7.
    pub const SDF_SOFT_MARCH: Self = Self(1 << 2);
    /// Hardware ray-traced / denoised visibility (`gShadowVis` or an inline `rayQuery`;
    /// `feature = "hwrt"` only).
    pub const HWRT_VIS: Self = Self(1 << 3);

    /// Whether every bit of `other` is set in `self`.
    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Returns `self` with `other`'s bits set.
    #[inline]
    pub const fn insert(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The raw bit pattern — the GPU/host packing seam.
    #[inline]
    pub const fn bits(self) -> u32 {
        self.0
    }
}

// ---- ResolvedRenderPath (the boot-committed carrier — plan §A) ------------------------

/// The boot-committed render-path selection — Decision 1: resolved exactly ONCE (at
/// `WindowHost::boot`) into this immutable `Copy` carrier, never re-derived per frame.
/// `#[repr(C)]`, ~44 B — read-only after boot, fits any cache line.
///
/// `#[derive(Resource)]` so a future ECS system CAN read it via `Res<ResolvedRenderPath>` (the
/// plan's declarator-dispatch consumer, R2+); R1 itself has no such consumer — this Resource is
/// dead-but-threaded, but it IS genuinely authoritative: `boyko_app::runner` overwrites the
/// `RenderPathPlugin`-inserted default with the real boot-resolved value (the SAME
/// `DdgiCaps`/`RayCaps` post-boot `insert_resource` override precedent), so a future
/// `Res<ResolvedRenderPath>` reader never observes a stale default. `boyko_app::runner` ALSO
/// stores the same value directly on `WindowHost` (the `ssaa_armed` precedent), since the RHI
/// seam (`GBufferScene`) is threaded host-side, not through a per-frame `World` read.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct ResolvedRenderPath {
    /// The FINAL (post-degrade) render-path technique.
    pub path: RenderPath,
    /// The FINAL (post-degrade) geometry-producer leg set.
    pub legs: GeometryLegs,
    /// `legs.has_mesh()` — cached so a consumer never has to call back into [`GeometryLegs`].
    pub mesh_leg: bool,
    /// `legs.has_sdf()`.
    pub sdf_leg: bool,
    /// `sdf_leg && path != Deferred` — the SDF leg is forward-marched (fused or geo/shade
    /// split) rather than composited into a fat gbuffer.
    pub sdf_forward_marched: bool,
    /// `ForwardPlus` always, OR `Forward` with a pre-light consumer armed (the Rev-5 union —
    /// see [`resolve_rules`]'s doc). Includes the MOTION-only `shadow_temporal` case.
    pub needs_depth_prepass: bool,
    /// Decision 8: `needs_depth_prepass && shadow_temporal_armed` — the depth prepass ALSO
    /// writes `motion_vec` (id Tech 6 depth+motion prepass) so a pre-light motion consumer sees
    /// current-frame motion; otherwise `mesh_forward` writes it (cheaper, post-tail).
    pub prepass_writes_motion: bool,
    /// `VisibilityBuffer` with a pre-light consumer armed — the SAME single predicate as
    /// `needs_depth_prepass` (Rev 5).
    pub mesh_geo_shade_split: bool,
    /// `sdf_forward_marched` with a pre-light consumer armed (Decision 6) — the SAME single
    /// predicate again.
    pub sdf_geo_shade_split: bool,
    /// `== sdf_geo_shade_split` — the thin SDF-surface cache (albedo/normal/material,
    /// SDF-pixels-only) that lets a pre-light consumer see the SDF leg without a second march.
    pub sdf_surface_cache: bool,
    /// `path == VisibilityBuffer && mesh_leg && device supports it` (Decision 0) — the bindless
    /// per-mesh geometry table exists.
    pub vb_geometry_table: bool,
    /// The depth-buffer contract this path uses (Decision 4).
    pub depth_kind: DepthKind,
    /// The armed thin cross-pass auxiliary channel set (§D). FROZEN at boot under non-Deferred
    /// paths (P2-d); under Deferred the fat gbuffer materializes normal/motion regardless, so
    /// this is informational only there (no cost either way).
    pub thin_aux: ThinAuxMask,
    /// The armed shadow-visibility source set (Decision 7). FROZEN at boot under non-Deferred
    /// paths, same P2-d rationale.
    pub shadow: ShadowSources,
}

// P2-d note: "frozen at boot" here means [`resolve_render_path`] is never RE-CALLED after
// boot — R1 has no runtime re-resolution mechanism at all, so there is nothing yet to make a
// stale-consumer toggle a no-op against. The plan's validation-table row "runtime toggle of a
// frozen pre-light consumer (non-Deferred) → no-op, warn-once" describes a GUARD the first
// rung that actually CONSUMES `thin_aux`/`shadow` structurally (R4+, once a declarator reads
// these fields to decide framegraph shape) must add, not a missed R1 deliverable — R1's own
// gate is simply "resolved once, never re-derived," which [`resolve_render_path`] already is.

impl Default for ResolvedRenderPath {
    #[inline]
    fn default() -> Self {
        // The resolve of the default config/consumers/caps — Deferred + Both, no consumers
        // armed, no degrades. A never-resolved world already carries this selection.
        resolve_render_path(
            &RenderPathConfig::default(),
            RenderPathConsumers::default(),
            RenderPathDeviceCaps::default(),
        )
        .0
    }
}

// ---- RenderPathConsumers (the resolver's consumer-arming input) -----------------------

/// The pre-light-consumer arming snapshot [`resolve_render_path`] needs — a boot-time READ of
/// each consumer's OWN structural enablement (its config Resource's `enabled()` / mode
/// predicate), NOT the per-frame "did anything actually cast a shadow this frame" arming
/// (`csm_armed`/`punctual_armed` in `boyko_app::runner`'s frame loop, which depend on a live
/// caster gather that has not run yet at boot). Decision 1 commits path/legs/consumer-set
/// exactly once, before any frame executes.
///
/// Plain fields, no `bool` redundancy beyond what each source config already carries — the
/// caller (`boyko_app::runner`) assembles this from `world.try_resource` reads, defaulting a
/// missing plugin's consumer to `false` (graceful degrade, never a panic).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderPathConsumers {
    /// `SsaoConfig::enabled()` — SSAO wants depth + normal pre-light.
    pub ssao_on: bool,
    /// `DdgiConfig::enabled()` — DDGI relight wants depth + normal pre-light.
    pub ddgi_on: bool,
    /// `ShadowDenoiseConfig::spatial_enabled()` — the à-trous shadow denoise wants depth +
    /// normal pre-light.
    pub shadow_denoise_spatial_on: bool,
    /// `ShadowDenoiseConfig::temporal_enabled()` — the temporal shadow reproject wants motion +
    /// `gViewT` pre-light (MOTION-only, no normal — the Rev-5 W4 case).
    pub shadow_temporal_on: bool,
    /// Screen-space reflections. No `SsrConfig` exists yet (out of scope this rung) — the
    /// caller threads a literal `false` until one lands.
    pub ssr_on: bool,
    /// `AaConfig.mode == AaMode::Taa` — TAA wants motion post-light (and pre-light only via
    /// [`shadow_temporal_on`](Self::shadow_temporal_on), not this flag).
    pub taa_on: bool,
    /// `CsmConfig::enabled()` — directional cascade shadows are configured on.
    pub csm_on: bool,
    /// `ShadowConfig::enabled()` — the sparse spot/point shadow atlas is configured on.
    pub punctual_shadows_on: bool,
    /// `feature = "hwrt"` AND (`ShadowDenoiseConfig::spatial_enabled()` OR
    /// `temporal_enabled()`) AND the device/tlas are ray-capable — hardware-traced or denoised
    /// shadow visibility is armed. `false` on every software build.
    pub hwrt_denoise_or_vis_on: bool,
    /// Whether the SDF leg should cast its own soft-march shadow when it is not already served
    /// by [`hwrt_denoise_or_vis_on`](Self::hwrt_denoise_or_vis_on). No owner-facing toggle
    /// exists yet — this mirrors Deferred's current unconditional non-hwrt SDF soft shadow
    /// (Decision 7's restoration target), so the caller threads `true` until a dedicated config
    /// lands.
    pub sdf_shadows_wanted: bool,
}

// ---- RenderPathDeviceCaps (the resolver's device-capability input) --------------------

/// The device-capability gate [`resolve_render_path`] needs for the [`VisibilityBuffer`](RenderPath::VisibilityBuffer)
/// path (Decision 0) — a plain input struct, NOT a `World` Resource (mirrors how
/// [`DdgiCaps`](crate::ddgi_update::DdgiCaps) is a Resource the host overrides post-boot, except
/// this one is passed directly into the pure resolve fn rather than fetched by an ECS system,
/// since `resolve_render_path` itself is a one-shot boot call, not a per-frame policy).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderPathDeviceCaps {
    /// Whether the device advertises `shaderStorageBufferArrayNonUniformIndexing`
    /// (`VkPhysicalDeviceDescriptorIndexingFeatures`) — the VisibilityBuffer path's bindless
    /// per-mesh geometry table needs it to index `gMeshVerts[]`/`gMeshIndices[]` by a
    /// wave-non-uniform `mesh_id` (`NonUniformResourceIndex`). Near-universal on desktop; absent
    /// ⇒ [`resolve_render_path`] degrades `VisibilityBuffer` to `Deferred` at boot.
    pub storage_buffer_array_non_uniform_indexing: bool,
}

impl Default for RenderPathDeviceCaps {
    #[inline]
    fn default() -> Self {
        // Assume supported until the host overrides with the real boot query (mirrors
        // `DdgiCaps::default`'s "most desktop GPUs support it" rationale) — a bench/test
        // harness that never queries the device wants the enabled path.
        Self { storage_buffer_array_non_uniform_indexing: true }
    }
}

impl RenderPathDeviceCaps {
    /// Builds the caps from a device query result (the host boot seam).
    #[inline]
    pub const fn new(storage_buffer_array_non_uniform_indexing: bool) -> Self {
        Self { storage_buffer_array_non_uniform_indexing }
    }
}

// ---- RenderPathDegrade / RenderPathDegradeLog (the boot warn-once payload) ------------

/// Why [`resolve_render_path`] fell back to a plan-documented degrade instead of the owner's
/// literal [`RenderPathConfig`] request — the boot-time warn-once payload (plan §A
/// "degrade-not-panic").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderPathDegrade {
    /// The requested [`RenderPath`] has no rung-landed implementation yet (the rung-staged
    /// consts at the top of this module) — collapsed to [`RenderPath::Deferred`].
    PathNotYetImplemented(RenderPath),
    /// [`RenderPath::VisibilityBuffer`] was requested but the device lacks
    /// `shaderStorageBufferArrayNonUniformIndexing` (Decision 0) — collapsed to
    /// [`RenderPath::Deferred`].
    VbDeviceCapMissing,
    /// A non-Deferred path was requested with a non-[`GeometryLegs::Mesh`] leg set
    /// (`Both`/`Sdf`) before the SDF-forward-march rung lands — collapsed to
    /// [`GeometryLegs::Mesh`].
    LegsCollapsedToMeshPreSdfForward,
    /// `Deferred` was requested with a leg whose own per-leg flag has not landed —
    /// collapsed to [`GeometryLegs::Both`]. As of rung R3, `Sdf` is landed
    /// ([`DEFERRED_SDF_ONLY_IMPLEMENTED`]) so this fires only for `Mesh`
    /// ([`DEFERRED_MESH_ONLY_IMPLEMENTED`] — blocked on the R3-audited `gViewT`
    /// producer gap, see this module's doc).
    DeferredLegDisableNotYetImplemented,
}

/// A fixed-capacity log of [`RenderPathDegrade`] reasons. [`degrade_ladder`]'s FOUR rules stack
/// at most TWO deep per call: at most ONE path-level rule fires (`PathNotYetImplemented` XOR
/// `VbDeviceCapMissing` — the latter needs `path_implemented == true`, which the former's arm
/// never reaches), and at most ONE legs-level rule fires (`DeferredLegDisableNotYetImplemented`
/// XOR `LegsCollapsedToMeshPreSdfForward` — the FIRST requires the FINAL `path == Deferred`, the
/// SECOND requires the FINAL `path != Deferred`; mutually exclusive by construction, so they
/// never co-fire). The array is sized to 2 slots for exactly this worst case (a fixed array
/// needs no heap allocation either way, Principle 5). `boyko_app::runner` iterates
/// [`Self::reasons`] and `warn!`s each ONCE.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderPathDegradeLog {
    reasons: [Option<RenderPathDegrade>; 2],
}

impl RenderPathDegradeLog {
    /// Records one degrade reason in the first free slot.
    fn push(&mut self, reason: RenderPathDegrade) {
        for slot in &mut self.reasons {
            if slot.is_none() {
                *slot = Some(reason);
                return;
            }
        }
        // invariant: the degrade ladder in `resolve_render_path` never emits more than one
        // path-level reason and one legs-level reason — a third push is a ladder bug, not user
        // error, so this is a debug-only invariant check (Principle 8's "panic only on
        // invariant violation" — vanishes in release, matching every other hot-path guard).
        debug_assert!(false, "invariant: RenderPathDegradeLog overflowed its 2-slot capacity");
    }

    /// Iterates the recorded degrade reasons, in ladder order (path-level, then legs-level).
    #[inline]
    pub fn reasons(&self) -> impl Iterator<Item = RenderPathDegrade> + '_ {
        self.reasons.iter().filter_map(|r| *r)
    }

    /// Whether nothing degraded — the owner's literal [`RenderPathConfig`] request was honored
    /// in full.
    #[inline]
    pub const fn is_clean(&self) -> bool {
        self.reasons[0].is_none()
    }
}

// ---- the pure rule set (Rev 5) ---------------------------------------------------------

/// The pure per-path/leg rule set (plan §A validation table + §D arming rules + Decisions
/// 4/6/7/8) — computed from the FINAL (post-degrade) `path`/`legs`, with no knowledge of which
/// rung has landed (that is [`resolve_render_path`]'s degrade-ladder concern, applied BEFORE
/// this fn is called). Kept as its own fn (rather than inlined into [`resolve_render_path`]) so
/// it is directly unit-testable against a HYPOTHETICAL fully-landed path (e.g. `Forward` with
/// `FORWARD_IMPLEMENTED` still `false`) — the Rev-5 MOTION-only `prepass_writes_motion` rule
/// must be provably correct TODAY, not merely once R4 lands.
///
/// # The Rev-5 single predicate
///
/// `pre_light = ssao_on ∥ ddgi_on ∥ shadow_denoise_spatial_on ∥ shadow_temporal_on ∥ ssr_on` is
/// computed ONCE and is the SOLE trigger for `needs_depth_prepass` (Forward),
/// `mesh_geo_shade_split` (VisibilityBuffer), and `sdf_geo_shade_split` (the SDF leg) — three
/// flags, one predicate, no drift. `shadow_temporal_on` is folded into the SAME union as the
/// NORMAL consumers (`ssao`/`ddgi`/`shadow_denoise_spatial`/`ssr`) because it is a MOTION-only
/// pre-light consumer (reads motion + `gViewT`, never normal — `graph_bridge.rs:1129`): gating
/// the prepass/split on a NORMAL-only union would leave `Forward + hwrt shadows +
/// ShadowDenoiseMode::Temporal + no SSAO/DDGI/SSR` reading frame-stale motion (the W4 hole,
/// re-opened without this fold — see this module's doc + Decision 8).
///
/// `pub` (cross-crate): besides this module's own tests, `boyko_app::gpu_scene`'s
/// `to_gpu_resolved_render_path` round-trip test builds a rich, non-default carrier through
/// this fn to verify the `boyko_render` → `boyko_rhi_vulkan` POD conversion copies every field
/// correctly — a realistic derived carrier (not a hand-built, possibly-inconsistent literal).
#[inline]
pub fn resolve_rules(
    path: RenderPath,
    legs: GeometryLegs,
    consumers: RenderPathConsumers,
    caps: RenderPathDeviceCaps,
) -> ResolvedRenderPath {
    let mesh_leg = legs.has_mesh();
    let sdf_leg = legs.has_sdf();

    // Rev 5 / final-critic P1: the single pre-light-consumer union (MANDATORY — see this fn's
    // doc). `shadow_temporal_on` MUST stay in this union, not a separate NORMAL-only union.
    let pre_light = consumers.ssao_on
        || consumers.ddgi_on
        || consumers.shadow_denoise_spatial_on
        || consumers.shadow_temporal_on
        || consumers.ssr_on;

    let needs_depth_prepass =
        matches!(path, RenderPath::ForwardPlus) || (matches!(path, RenderPath::Forward) && pre_light);
    // Decision 8: the prepass ALSO writes motion only when a PRE-LIGHT motion consumer
    // (shadow_temporal) is armed; a TAA-only (post-light) config keeps the cheaper
    // mesh_forward-writes-motion form (not encoded here — a per-pass framegraph decision).
    let prepass_writes_motion = needs_depth_prepass && consumers.shadow_temporal_on;

    let sdf_forward_marched = sdf_leg && !matches!(path, RenderPath::Deferred);
    let mesh_geo_shade_split = matches!(path, RenderPath::VisibilityBuffer) && pre_light;
    let sdf_geo_shade_split = sdf_forward_marched && pre_light;
    let sdf_surface_cache = sdf_geo_shade_split;

    let vb_geometry_table = matches!(path, RenderPath::VisibilityBuffer)
        && mesh_leg
        && caps.storage_buffer_array_non_uniform_indexing;

    let depth_kind =
        if matches!(path, RenderPath::Deferred) { DepthKind::CustomLinear } else { DepthKind::HardwareReverseZ };

    let mut thin_aux = ThinAuxMask::NONE;
    if consumers.ssao_on || consumers.ddgi_on || consumers.shadow_denoise_spatial_on || consumers.ssr_on {
        thin_aux = thin_aux.insert(ThinAuxMask::NORMAL);
    }
    if consumers.taa_on || consumers.shadow_temporal_on {
        thin_aux = thin_aux.insert(ThinAuxMask::MOTION);
    }
    if consumers.ssr_on {
        thin_aux = thin_aux.insert(ThinAuxMask::ROUGHNESS);
    }

    let mut shadow = ShadowSources::NONE;
    if consumers.csm_on {
        shadow = shadow.insert(ShadowSources::CSM);
    }
    if consumers.punctual_shadows_on {
        shadow = shadow.insert(ShadowSources::PUNCTUAL_ATLAS);
    }
    if sdf_leg && consumers.sdf_shadows_wanted && !consumers.hwrt_denoise_or_vis_on {
        shadow = shadow.insert(ShadowSources::SDF_SOFT_MARCH);
    }
    if consumers.hwrt_denoise_or_vis_on {
        shadow = shadow.insert(ShadowSources::HWRT_VIS);
    }

    ResolvedRenderPath {
        path,
        legs,
        mesh_leg,
        sdf_leg,
        sdf_forward_marched,
        needs_depth_prepass,
        prepass_writes_motion,
        mesh_geo_shade_split,
        sdf_geo_shade_split,
        sdf_surface_cache,
        vb_geometry_table,
        depth_kind,
        thin_aux,
        shadow,
    }
}

// ---- the degrade ladder (plan §A validation table + §H) --------------------------------

/// The rung-staged degrade ladder, parameterized by the "is this rung landed" flags (rather
/// than reading the module consts directly) so it stays independently unit-testable — the SAME
/// "test the rule directly" discipline [`resolve_rules`] uses for the Rev-5 predicate.
/// [`resolve_render_path`] is the ONLY caller that threads the real
/// [`FORWARD_IMPLEMENTED`]/[`FORWARD_PLUS_IMPLEMENTED`]/[`VB_IMPLEMENTED`]/
/// [`SDF_FORWARD_IMPLEMENTED`]/[`DEFERRED_SDF_ONLY_IMPLEMENTED`]/[`DEFERRED_MESH_ONLY_IMPLEMENTED`]
/// consts; a test can thread `true` early to exercise e.g. the VB device-cap degrade in
/// isolation, before `VB_IMPLEMENTED` itself lands.
///
/// Order: (1) an unimplemented path collapses to `Deferred`; (2) a `VisibilityBuffer` path that
/// survived (1) but whose device lacks the geometry-table cap ALSO collapses to `Deferred`; (3)
/// a FINAL `path == Deferred` (whether requested directly or reached via (1)/(2)) requesting
/// `Mesh` before `deferred_mesh_only_implemented` lands, OR `Sdf` before
/// `deferred_sdf_only_implemented` lands, collapses the legs to `Both`
/// ([`RenderPathDegrade::DeferredLegDisableNotYetImplemented`] — rung R3 SPLIT the single R2 flag
/// into one per leg, see this module's doc); (4) a FINAL `path != Deferred` requesting a
/// non-`Mesh` leg set before the SDF-forward-march rung lands collapses the legs to `Mesh`. Rules
/// (3) and (4) gate on COMPLEMENTARY `path == Deferred` / `path != Deferred` conditions, so at
/// most one of them ever fires in a single call (see [`RenderPathDegradeLog`]'s doc for why the
/// log never needs more than 2 slots). Never a panic — degrade-not-panic by construction.
fn degrade_ladder(
    requested_path: RenderPath,
    requested_legs: GeometryLegs,
    caps: RenderPathDeviceCaps,
    path_implemented: bool,
    sdf_forward_implemented: bool,
    deferred_mesh_only_implemented: bool,
    deferred_sdf_only_implemented: bool,
) -> (RenderPath, GeometryLegs, RenderPathDegradeLog) {
    let mut degrades = RenderPathDegradeLog::default();

    let mut path = if path_implemented {
        requested_path
    } else {
        degrades.push(RenderPathDegrade::PathNotYetImplemented(requested_path));
        RenderPath::Deferred
    };

    if matches!(path, RenderPath::VisibilityBuffer) && !caps.storage_buffer_array_non_uniform_indexing {
        degrades.push(RenderPathDegrade::VbDeviceCapMissing);
        path = RenderPath::Deferred;
    }

    let mut legs = requested_legs;
    // Rung R3: fires ONLY when the FINAL path is Deferred (whether requested directly or
    // reached via the path-level demotion above) AND the REQUESTED leg's own per-leg flag has
    // not landed — mutually exclusive with the SDF-forward legs-collapse rule below, which
    // fires ONLY when the final path is NOT Deferred.
    let leg_not_yet_implemented = match legs {
        GeometryLegs::Both => false,
        GeometryLegs::Mesh => !deferred_mesh_only_implemented,
        GeometryLegs::Sdf => !deferred_sdf_only_implemented,
    };
    if matches!(path, RenderPath::Deferred) && leg_not_yet_implemented {
        degrades.push(RenderPathDegrade::DeferredLegDisableNotYetImplemented);
        legs = GeometryLegs::Both;
    }
    if !matches!(path, RenderPath::Deferred) && !matches!(legs, GeometryLegs::Mesh) && !sdf_forward_implemented
    {
        degrades.push(RenderPathDegrade::LegsCollapsedToMeshPreSdfForward);
        legs = GeometryLegs::Mesh;
    }

    (path, legs, degrades)
}

// ---- resolve_render_path (the single boot-time entry point — Decision 1) --------------

/// Resolves the owner's [`RenderPathConfig`] + the boot-time [`RenderPathConsumers`] snapshot +
/// [`RenderPathDeviceCaps`] into a [`ResolvedRenderPath`] plus the [`RenderPathDegradeLog`] of
/// any plan-documented fallback applied along the way. Pure, total (never panics — degrade, not
/// panic, on every unsupported combo) and called EXACTLY ONCE, at `WindowHost::boot`
/// (`boyko_app::runner`, the `ssaa_armed` precedent) — see this module's doc for why a per-frame
/// re-resolve is forbidden by design (Decision 1).
///
/// Applies the rung-staged [`degrade_ladder`] against the REQUESTED `(path, legs)` first (using
/// the real [`FORWARD_IMPLEMENTED`]/[`FORWARD_PLUS_IMPLEMENTED`]/[`VB_IMPLEMENTED`]/
/// [`SDF_FORWARD_IMPLEMENTED`]/[`DEFERRED_MESH_ONLY_IMPLEMENTED`]/[`DEFERRED_SDF_ONLY_IMPLEMENTED`]
/// consts), then computes every derived field via [`resolve_rules`] against the FINAL
/// (post-degrade) `(path, legs)` — so e.g. `depth_kind`/`thin_aux`/`shadow` always describe what
/// will ACTUALLY be recorded, never the owner's un-degraded request.
#[inline]
pub fn resolve_render_path(
    cfg: &RenderPathConfig,
    consumers: RenderPathConsumers,
    caps: RenderPathDeviceCaps,
) -> (ResolvedRenderPath, RenderPathDegradeLog) {
    let path_implemented = match cfg.path {
        RenderPath::Deferred => true,
        RenderPath::Forward => FORWARD_IMPLEMENTED,
        RenderPath::ForwardPlus => FORWARD_PLUS_IMPLEMENTED,
        RenderPath::VisibilityBuffer => VB_IMPLEMENTED,
    };
    let (path, legs, degrades) = degrade_ladder(
        cfg.path,
        cfg.legs,
        caps,
        path_implemented,
        SDF_FORWARD_IMPLEMENTED,
        DEFERRED_MESH_ONLY_IMPLEMENTED,
        DEFERRED_SDF_ONLY_IMPLEMENTED,
    );
    (resolve_rules(path, legs, consumers, caps), degrades)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps_ok() -> RenderPathDeviceCaps {
        RenderPathDeviceCaps { storage_buffer_array_non_uniform_indexing: true }
    }

    fn caps_missing() -> RenderPathDeviceCaps {
        RenderPathDeviceCaps { storage_buffer_array_non_uniform_indexing: false }
    }

    // ---- default = Deferred+Both, no degrades ------------------------------------------

    #[test]
    fn default_config_is_deferred_both() {
        let cfg = RenderPathConfig::default();
        assert_eq!(cfg.path, RenderPath::Deferred);
        assert_eq!(cfg.legs, GeometryLegs::Both);
    }

    #[test]
    fn default_resolve_is_clean_and_matches_resolved_default() {
        let (resolved, degrades) =
            resolve_render_path(&RenderPathConfig::default(), RenderPathConsumers::default(), caps_ok());
        assert!(degrades.is_clean());
        assert_eq!(degrades.reasons().count(), 0);
        assert_eq!(resolved.path, RenderPath::Deferred);
        assert_eq!(resolved.legs, GeometryLegs::Both);
        assert!(resolved.mesh_leg && resolved.sdf_leg);
        assert!(!resolved.sdf_forward_marched);
        assert!(!resolved.needs_depth_prepass);
        assert!(!resolved.prepass_writes_motion);
        assert!(!resolved.mesh_geo_shade_split);
        assert!(!resolved.sdf_geo_shade_split);
        assert!(!resolved.sdf_surface_cache);
        assert!(!resolved.vb_geometry_table);
        assert_eq!(resolved.depth_kind, DepthKind::CustomLinear);
        assert_eq!(resolved.thin_aux, ThinAuxMask::NONE);
        assert_eq!(resolved.shadow, ShadowSources::NONE);
        assert_eq!(resolved, ResolvedRenderPath::default());
    }

    // ---- every non-Deferred path degrades today (with reason) -------------------------

    #[test]
    fn every_non_deferred_path_degrades_today() {
        for path in [RenderPath::Forward, RenderPath::ForwardPlus, RenderPath::VisibilityBuffer] {
            let cfg = RenderPathConfig { path, legs: GeometryLegs::Both };
            let (resolved, degrades) =
                resolve_render_path(&cfg, RenderPathConsumers::default(), caps_ok());
            assert_eq!(resolved.path, RenderPath::Deferred, "{path:?} must degrade to Deferred today");
            let reasons: Vec<_> = degrades.reasons().collect();
            assert_eq!(reasons, [RenderPathDegrade::PathNotYetImplemented(path)]);
        }
    }

    #[test]
    fn deferred_both_never_degrades() {
        let cfg = RenderPathConfig { path: RenderPath::Deferred, legs: GeometryLegs::Both };
        let (resolved, degrades) = resolve_render_path(&cfg, RenderPathConsumers::default(), caps_ok());
        assert!(degrades.is_clean(), "Deferred x Both must never degrade -- the byte-identity anchor");
        assert_eq!(resolved.path, RenderPath::Deferred);
        assert_eq!(resolved.legs, GeometryLegs::Both);
    }

    // ---- rung R3: Deferred x Sdf lands; Deferred x Mesh stays blocked ------------------

    #[test]
    fn deferred_mesh_only_still_degrades_legs_to_both_at_r3() {
        // The R3 audit found the marcher is the sole `gViewT` producer for MESH-owned pixels
        // too (see this module's doc) -- `DEFERRED_MESH_ONLY_IMPLEMENTED` stays `false` until an
        // approved producer-replacement design lands, so this leg keeps degrading.
        let cfg = RenderPathConfig { path: RenderPath::Deferred, legs: GeometryLegs::Mesh };
        let (resolved, degrades) = resolve_render_path(&cfg, RenderPathConsumers::default(), caps_ok());
        assert_eq!(resolved.path, RenderPath::Deferred);
        assert_eq!(resolved.legs, GeometryLegs::Both, "Deferred x Mesh stays blocked past R3 (gViewT gap)");
        let reasons: Vec<_> = degrades.reasons().collect();
        assert_eq!(reasons, [RenderPathDegrade::DeferredLegDisableNotYetImplemented]);
    }

    #[test]
    fn deferred_sdf_only_no_longer_degrades_at_r3() {
        let cfg = RenderPathConfig { path: RenderPath::Deferred, legs: GeometryLegs::Sdf };
        let (resolved, degrades) = resolve_render_path(&cfg, RenderPathConsumers::default(), caps_ok());
        assert_eq!(resolved.path, RenderPath::Deferred);
        assert_eq!(resolved.legs, GeometryLegs::Sdf, "Deferred x Sdf lands at R3 -- no degrade");
        assert!(degrades.is_clean());
        assert!(!resolved.mesh_leg && resolved.sdf_leg);
    }

    // ---- VB-without-cap degrade (tested directly against the ladder, not gated on ------
    // ---- VB_IMPLEMENTED still being false today) ---------------------------------------

    #[test]
    fn vb_without_device_cap_degrades_to_deferred() {
        let (path, legs, degrades) = degrade_ladder(
            RenderPath::VisibilityBuffer,
            GeometryLegs::Mesh,
            caps_missing(),
            true, // hypothetically landed, to exercise the device-cap rule in isolation
            true,
            true, // isolate: the Deferred-leg-disable rules must not co-fire with this one
            true,
        );
        assert_eq!(path, RenderPath::Deferred);
        assert_eq!(legs, GeometryLegs::Mesh);
        let reasons: Vec<_> = degrades.reasons().collect();
        assert_eq!(reasons, [RenderPathDegrade::VbDeviceCapMissing]);
    }

    #[test]
    fn vb_with_device_cap_stays_visibility_buffer_once_landed() {
        let (path, _legs, degrades) = degrade_ladder(
            RenderPath::VisibilityBuffer,
            GeometryLegs::Mesh,
            caps_ok(),
            true,
            true,
            true,
            true,
        );
        assert_eq!(path, RenderPath::VisibilityBuffer);
        assert!(degrades.is_clean());
    }

    #[test]
    fn unimplemented_path_short_circuits_before_the_device_cap_check() {
        // Today (VB_IMPLEMENTED == false), a VB request degrades for
        // PathNotYetImplemented, NOT VbDeviceCapMissing, regardless of device caps. `legs:
        // Both` keeps this test focused on the path-level ladder order alone (a `Mesh` leg
        // request here would ALSO trip the R2 Deferred-leg-disable rule once the demotion
        // lands `path == Deferred`, which is a separate concern covered by
        // `deferred_mesh_only_degrades_legs_to_both_before_r3`).
        let cfg = RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Both };
        let (resolved, degrades) =
            resolve_render_path(&cfg, RenderPathConsumers::default(), caps_missing());
        assert_eq!(resolved.path, RenderPath::Deferred);
        let reasons: Vec<_> = degrades.reasons().collect();
        assert_eq!(reasons, [RenderPathDegrade::PathNotYetImplemented(RenderPath::VisibilityBuffer)]);
    }

    // ---- legs collapse pre-SDF-forward --------------------------------------------------

    #[test]
    fn non_mesh_legs_collapse_to_mesh_before_sdf_forward_lands() {
        for legs in [GeometryLegs::Both, GeometryLegs::Sdf] {
            let (path, resolved_legs, degrades) =
                degrade_ladder(RenderPath::Forward, legs, caps_ok(), true, false, false, false);
            assert_eq!(path, RenderPath::Forward);
            assert_eq!(resolved_legs, GeometryLegs::Mesh);
            let reasons: Vec<_> = degrades.reasons().collect();
            assert_eq!(reasons, [RenderPathDegrade::LegsCollapsedToMeshPreSdfForward]);
        }
    }

    #[test]
    fn mesh_only_legs_never_collapse() {
        let (path, legs, degrades) =
            degrade_ladder(RenderPath::Forward, GeometryLegs::Mesh, caps_ok(), true, false, false, false);
        assert_eq!(path, RenderPath::Forward);
        assert_eq!(legs, GeometryLegs::Mesh);
        assert!(degrades.is_clean());
    }

    #[test]
    fn legs_stay_as_requested_once_sdf_forward_lands() {
        for legs in [GeometryLegs::Both, GeometryLegs::Sdf, GeometryLegs::Mesh] {
            let (path, resolved_legs, degrades) =
                degrade_ladder(RenderPath::ForwardPlus, legs, caps_ok(), true, true, true, true);
            assert_eq!(path, RenderPath::ForwardPlus);
            assert_eq!(resolved_legs, legs);
            assert!(degrades.is_clean());
        }
    }

    // ---- rung R3: degrade_ladder direct — per-leg independence (Deferred) --------------

    #[test]
    fn deferred_ladder_degrades_only_the_not_yet_implemented_leg() {
        // Mesh-only implemented, sdf-only not: a Mesh request stays; a Sdf request degrades.
        let (path, legs, degrades) =
            degrade_ladder(RenderPath::Deferred, GeometryLegs::Mesh, caps_ok(), true, false, true, false);
        assert_eq!(path, RenderPath::Deferred);
        assert_eq!(legs, GeometryLegs::Mesh);
        assert!(degrades.is_clean());

        let (path, legs, degrades) =
            degrade_ladder(RenderPath::Deferred, GeometryLegs::Sdf, caps_ok(), true, false, true, false);
        assert_eq!(path, RenderPath::Deferred);
        assert_eq!(legs, GeometryLegs::Both);
        let reasons: Vec<_> = degrades.reasons().collect();
        assert_eq!(reasons, [RenderPathDegrade::DeferredLegDisableNotYetImplemented]);
    }

    // ---- has_mesh / has_sdf table --------------------------------------------------------

    #[test]
    fn geometry_legs_has_mesh_has_sdf_table() {
        assert!(GeometryLegs::Both.has_mesh() && GeometryLegs::Both.has_sdf());
        assert!(GeometryLegs::Mesh.has_mesh() && !GeometryLegs::Mesh.has_sdf());
        assert!(!GeometryLegs::Sdf.has_mesh() && GeometryLegs::Sdf.has_sdf());
    }

    // ---- the Rev-5 MOTION-only case, tested directly against resolve_rules -------------
    // (live TODAY: Forward is threaded here even though FORWARD_IMPLEMENTED is still false)

    #[test]
    fn motion_only_shadow_temporal_arms_depth_prepass_under_forward() {
        let consumers = RenderPathConsumers { shadow_temporal_on: true, ..RenderPathConsumers::default() };
        assert!(!consumers.ssao_on && !consumers.ddgi_on && !consumers.shadow_denoise_spatial_on && !consumers.ssr_on);

        let resolved = resolve_rules(RenderPath::Forward, GeometryLegs::Mesh, consumers, caps_ok());
        assert!(
            resolved.needs_depth_prepass,
            "a MOTION-only pre-light consumer (shadow_temporal) must still arm the depth prepass (W4)"
        );
        assert!(resolved.prepass_writes_motion, "the armed prepass must write motion (Decision 8)");
        assert!(resolved.thin_aux.contains(ThinAuxMask::MOTION));
        assert!(!resolved.thin_aux.contains(ThinAuxMask::NORMAL), "shadow_temporal is MOTION-only, no normal");
    }

    #[test]
    fn normal_only_ssao_arms_depth_prepass_without_motion_in_the_prepass() {
        // The complementary Decision-8 truth-table row: a NORMAL-only pre-light consumer
        // (SSAO, no shadow_temporal) still arms `needs_depth_prepass` (Rev-5 union), but the
        // prepass must NOT write motion — `prepass_writes_motion` is gated separately on
        // `shadow_temporal_on`, which is false here.
        let consumers = RenderPathConsumers { ssao_on: true, ..RenderPathConsumers::default() };
        assert!(!consumers.shadow_temporal_on);

        let resolved = resolve_rules(RenderPath::Forward, GeometryLegs::Mesh, consumers, caps_ok());
        assert!(resolved.needs_depth_prepass, "a NORMAL-only pre-light consumer (SSAO) must arm the prepass");
        assert!(
            !resolved.prepass_writes_motion,
            "no shadow_temporal armed -> the prepass must NOT write motion (Decision 8)"
        );
        assert!(resolved.thin_aux.contains(ThinAuxMask::NORMAL));
        assert!(!resolved.thin_aux.contains(ThinAuxMask::MOTION), "SSAO alone arms no MOTION channel");
    }

    #[test]
    fn motion_only_shadow_temporal_arms_vb_split_and_sdf_split() {
        let consumers = RenderPathConsumers { shadow_temporal_on: true, ..RenderPathConsumers::default() };

        let vb = resolve_rules(RenderPath::VisibilityBuffer, GeometryLegs::Mesh, consumers, caps_ok());
        assert!(vb.mesh_geo_shade_split, "VB must split under a MOTION-only pre-light consumer too");

        let sdf = resolve_rules(RenderPath::Forward, GeometryLegs::Sdf, consumers, caps_ok());
        assert!(sdf.sdf_geo_shade_split);
        assert!(sdf.sdf_surface_cache);
    }

    #[test]
    fn taa_only_does_not_arm_the_prepass_or_the_split() {
        // TAA is a POST-light motion consumer — it must NOT trigger needs_depth_prepass /
        // mesh_geo_shade_split / sdf_geo_shade_split on its own (only pre-light consumers do).
        let consumers = RenderPathConsumers { taa_on: true, ..RenderPathConsumers::default() };
        let resolved = resolve_rules(RenderPath::Forward, GeometryLegs::Mesh, consumers, caps_ok());
        assert!(!resolved.needs_depth_prepass);
        assert!(!resolved.prepass_writes_motion);
        assert!(resolved.thin_aux.contains(ThinAuxMask::MOTION), "TAA still arms the MOTION channel itself");
    }

    #[test]
    fn forward_plus_always_needs_the_prepass() {
        let resolved =
            resolve_rules(RenderPath::ForwardPlus, GeometryLegs::Mesh, RenderPathConsumers::default(), caps_ok());
        assert!(resolved.needs_depth_prepass, "ForwardPlus always runs the depth prepass");
        assert!(!resolved.prepass_writes_motion, "no shadow_temporal armed -> no motion in the prepass");
    }

    // ---- thin_aux arming table -----------------------------------------------------------

    #[test]
    fn thin_aux_normal_arms_from_any_normal_consumer() {
        for (flag_setter, label) in [
            (RenderPathConsumers { ssao_on: true, ..Default::default() }, "ssao"),
            (RenderPathConsumers { ddgi_on: true, ..Default::default() }, "ddgi"),
            (RenderPathConsumers { shadow_denoise_spatial_on: true, ..Default::default() }, "spatial denoise"),
            (RenderPathConsumers { ssr_on: true, ..Default::default() }, "ssr"),
        ] {
            let resolved = resolve_rules(RenderPath::Deferred, GeometryLegs::Both, flag_setter, caps_ok());
            assert!(resolved.thin_aux.contains(ThinAuxMask::NORMAL), "{label} must arm NORMAL");
        }
    }

    #[test]
    fn thin_aux_roughness_arms_only_from_ssr() {
        let ssr = RenderPathConsumers { ssr_on: true, ..Default::default() };
        let resolved = resolve_rules(RenderPath::Deferred, GeometryLegs::Both, ssr, caps_ok());
        assert!(resolved.thin_aux.contains(ThinAuxMask::ROUGHNESS));

        let ssao = RenderPathConsumers { ssao_on: true, ..Default::default() };
        let resolved = resolve_rules(RenderPath::Deferred, GeometryLegs::Both, ssao, caps_ok());
        assert!(!resolved.thin_aux.contains(ThinAuxMask::ROUGHNESS));
    }

    #[test]
    fn thin_aux_motion_arms_from_taa_or_shadow_temporal() {
        let taa = RenderPathConsumers { taa_on: true, ..Default::default() };
        assert!(resolve_rules(RenderPath::Deferred, GeometryLegs::Both, taa, caps_ok())
            .thin_aux
            .contains(ThinAuxMask::MOTION));

        let temporal = RenderPathConsumers { shadow_temporal_on: true, ..Default::default() };
        assert!(resolve_rules(RenderPath::Deferred, GeometryLegs::Both, temporal, caps_ok())
            .thin_aux
            .contains(ThinAuxMask::MOTION));
    }

    // ---- shadow arming table --------------------------------------------------------------

    #[test]
    fn shadow_sources_arm_from_their_own_consumer() {
        let csm = RenderPathConsumers { csm_on: true, ..Default::default() };
        assert_eq!(
            resolve_rules(RenderPath::Deferred, GeometryLegs::Mesh, csm, caps_ok()).shadow,
            ShadowSources::CSM
        );

        let punctual = RenderPathConsumers { punctual_shadows_on: true, ..Default::default() };
        assert_eq!(
            resolve_rules(RenderPath::Deferred, GeometryLegs::Mesh, punctual, caps_ok()).shadow,
            ShadowSources::PUNCTUAL_ATLAS
        );
    }

    #[test]
    fn sdf_soft_march_needs_the_sdf_leg_and_is_wanted_and_no_hwrt() {
        let wanted = RenderPathConsumers { sdf_shadows_wanted: true, ..Default::default() };

        // No SDF leg -> no SDF_SOFT_MARCH even if wanted.
        let mesh_only = resolve_rules(RenderPath::Deferred, GeometryLegs::Mesh, wanted, caps_ok());
        assert!(!mesh_only.shadow.contains(ShadowSources::SDF_SOFT_MARCH));

        // SDF leg + wanted + no hwrt -> armed.
        let with_sdf = resolve_rules(RenderPath::Deferred, GeometryLegs::Sdf, wanted, caps_ok());
        assert!(with_sdf.shadow.contains(ShadowSources::SDF_SOFT_MARCH));

        // hwrt vis/denoise takes over -> soft march NOT armed, HWRT_VIS is.
        let hwrt = RenderPathConsumers {
            sdf_shadows_wanted: true,
            hwrt_denoise_or_vis_on: true,
            ..Default::default()
        };
        let with_hwrt = resolve_rules(RenderPath::Deferred, GeometryLegs::Sdf, hwrt, caps_ok());
        assert!(!with_hwrt.shadow.contains(ShadowSources::SDF_SOFT_MARCH));
        assert!(with_hwrt.shadow.contains(ShadowSources::HWRT_VIS));
    }

    #[test]
    fn every_shadow_source_can_combine() {
        let all = RenderPathConsumers {
            csm_on: true,
            punctual_shadows_on: true,
            sdf_shadows_wanted: false,
            hwrt_denoise_or_vis_on: true,
            ..Default::default()
        };
        let resolved = resolve_rules(RenderPath::Deferred, GeometryLegs::Mesh, all, caps_ok());
        assert!(resolved.shadow.contains(ShadowSources::CSM));
        assert!(resolved.shadow.contains(ShadowSources::PUNCTUAL_ATLAS));
        assert!(resolved.shadow.contains(ShadowSources::HWRT_VIS));
    }

    // ---- depth_kind table -------------------------------------------------------------

    #[test]
    fn depth_kind_table() {
        let d = resolve_rules(RenderPath::Deferred, GeometryLegs::Both, RenderPathConsumers::default(), caps_ok());
        assert_eq!(d.depth_kind, DepthKind::CustomLinear);

        for path in [RenderPath::Forward, RenderPath::ForwardPlus, RenderPath::VisibilityBuffer] {
            let resolved = resolve_rules(path, GeometryLegs::Mesh, RenderPathConsumers::default(), caps_ok());
            assert_eq!(resolved.depth_kind, DepthKind::HardwareReverseZ, "{path:?} must use HW reverse-Z");
        }
    }

    // ---- vb_geometry_table / sdf_forward_marched tables --------------------------------

    #[test]
    fn vb_geometry_table_needs_path_mesh_leg_and_device_cap() {
        let on = resolve_rules(RenderPath::VisibilityBuffer, GeometryLegs::Mesh, RenderPathConsumers::default(), caps_ok());
        assert!(on.vb_geometry_table);

        let no_cap =
            resolve_rules(RenderPath::VisibilityBuffer, GeometryLegs::Mesh, RenderPathConsumers::default(), caps_missing());
        assert!(!no_cap.vb_geometry_table);

        let sdf_only =
            resolve_rules(RenderPath::VisibilityBuffer, GeometryLegs::Sdf, RenderPathConsumers::default(), caps_ok());
        assert!(!sdf_only.vb_geometry_table, "no mesh leg -> no geometry table");

        let deferred = resolve_rules(RenderPath::Deferred, GeometryLegs::Both, RenderPathConsumers::default(), caps_ok());
        assert!(!deferred.vb_geometry_table);
    }

    #[test]
    fn sdf_forward_marched_only_under_non_deferred_with_sdf_leg() {
        let deferred = resolve_rules(RenderPath::Deferred, GeometryLegs::Sdf, RenderPathConsumers::default(), caps_ok());
        assert!(!deferred.sdf_forward_marched, "Deferred composites the SDF leg, never forward-marches it");

        let forward = resolve_rules(RenderPath::Forward, GeometryLegs::Sdf, RenderPathConsumers::default(), caps_ok());
        assert!(forward.sdf_forward_marched);

        let forward_mesh_only =
            resolve_rules(RenderPath::Forward, GeometryLegs::Mesh, RenderPathConsumers::default(), caps_ok());
        assert!(!forward_mesh_only.sdf_forward_marched, "no SDF leg -> nothing to forward-march");
    }

    // ---- degrade log capacity ------------------------------------------------------------

    #[test]
    fn degrade_log_reasons_are_empty_by_default() {
        let log = RenderPathDegradeLog::default();
        assert!(log.is_clean());
        assert_eq!(log.reasons().count(), 0);
    }

    #[test]
    fn degrade_log_capacity_holds_two_distinct_reasons() {
        // `degrade_ladder`'s own rules never stack two reasons in one call (see
        // `RenderPathDegradeLog`'s doc) -- this test proves the LOG's raw storage capacity
        // directly, independent of whether today's ladder logic ever exercises both slots.
        let mut log = RenderPathDegradeLog::default();
        log.push(RenderPathDegrade::PathNotYetImplemented(RenderPath::Forward));
        log.push(RenderPathDegrade::LegsCollapsedToMeshPreSdfForward);
        let reasons: Vec<_> = log.reasons().collect();
        assert_eq!(
            reasons,
            [
                RenderPathDegrade::PathNotYetImplemented(RenderPath::Forward),
                RenderPathDegrade::LegsCollapsedToMeshPreSdfForward,
            ]
        );
    }
}
