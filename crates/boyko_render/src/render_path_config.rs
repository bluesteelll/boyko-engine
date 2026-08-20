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
//! Only `Deferred` was implemented as of R1; every path const has since flipped `true` —
//! `FORWARD_IMPLEMENTED` (R4b-b), `FORWARD_PLUS_IMPLEMENTED` (R5), `SDF_FORWARD_IMPLEMENTED`
//! (R-SDFFWD), `VB_IMPLEMENTED` (R8), `VB_SDF_IMPLEMENTED` (R10). ALL FOUR `RenderPath` variants
//! therefore resolve for real: [`RenderPathDegrade::PathNotYetImplemented`] has no production
//! trigger left, and the only path-level degrade still reachable is the VB device-cap rule
//! ([`RenderPathDegrade::VbDeviceCapMissing`]). `SDF_SPLIT_IMPLEMENTED` is the one const still
//! `false` — it gates a derived FLAG, not a path. Each const flips `true` as its rung lands — no
//! other code in this module changes; the degrade ladder ([`resolve_render_path`]) and the pure
//! rule set ([`resolve_rules`]) are ALREADY correct for the fully-landed plan, tested directly
//! against the rule (not gated behind the rung consts) so they stayed live throughout.
//!
//! Rung R2 added a single combined `DEFERRED_LEG_DISABLE_IMPLEMENTED` guard (`false`); rung R3
//! split it into a per-leg pair (mesh-only / sdf-only) — the SAME "rung-staged const per landed
//! capability" idiom every other flag in this module uses — because the two legs landed at
//! DIFFERENT confidence levels at first (plan §H R3 row + the R3 rung audit):
//!
//! - **`Deferred × Sdf`** (mesh raster leg off, landed R3): the raster gbuffer pass is skipped, a
//!   `mesh_depth_neutral_clear` pass (`boyko_rhi_vulkan::present::graph_bridge`) replaces its
//!   depth-clear producer so the marcher's mesh-depth sample deterministically reads the
//!   far-plane sentinel (the SAME "no mesh in the scene" code path the marcher already handles
//!   byte-identically), reusing `sdf_gbuffer_composite.hlsl` UNCHANGED (no `HAS_MESH` compiled
//!   variant — a deliberate, documented deviation from the plan's literal text; see the R3 rung
//!   report for the risk analysis). Every MESH-shadow producer the raster pass fed is ALSO
//!   structurally suppressed — CSM cascade depth, the punctual spot/point atlas depth, and (under
//!   `hwrt`) the TLAS pack/build + the shadow_vis/à-trous/temporal denoise chain — because
//!   mesh-shadow producers are mesh-leg-owned (they rasterize/trace MESH casters only; the SDF
//!   leg's shadow is the marcher's own baked soft march). `GpuSceneBundles::scene()`
//!   (`boyko_app::gpu_scene`) is the single scene-assembly seam that gates this (capability =
//!   component presence, not a runtime flag); `declare_deferred_graph` carries a `debug_assert!`
//!   belt-and-braces check the seam was not missed.
//! - **`Deferred × Mesh`** (SDF leg off, landed R3b): the R3 audit found the marcher is the SOLE
//!   producer of the `gViewT` lane for MESH-owned pixels too (`sdf_gbuffer_composite.hlsl`'s
//!   `gViewT[...] = (own_pixel && mask==1.0) ? t : (has_mesh ? t_mesh : 1.0e30)`, both terminal
//!   write sites), and every `gViewT` consumer (the resolve's `P = ro + rd*view_t`
//!   reconstruction, SSAO's `view_t` mesh/SDF classification) reads it UNCONDITIONALLY under
//!   `mask == 1` — mesh pixels included, not just SDF ones. Skipping the marcher entirely (the
//!   plan's O2 "verified" mesh-only design) would leave `gViewT` wholly unwritten. Rung R3b closes
//!   this with a dedicated `viewt_from_depth` compute pass
//!   (`boyko_rhi_vulkan::present::graph_bridge` + `boyko_rhi_vulkan::compute::
//!   ViewtFromDepthPush`) that reproduces the marcher's own mesh-depth → `gViewT` conversion for
//!   every pixel — the approved producer-replacement design the R3 rung report's Phase-1 audit
//!   finding (a) called for. Gated by the SAME `GpuSceneBundles::scene()` scene-assembly seam +
//!   `declare_deferred_graph`'s belt-and-braces `debug_assert!`.
//!
//! Both legs are landed as of R3b, so the earlier `RenderPathDegrade::DeferredLegDisableNotYetImplemented`
//! variant — the "leg-disable landed but this specific leg has not" degrade — no longer has a
//! reachable trigger and was removed (dead code); a `Deferred` request now honors ANY
//! [`GeometryLegs`] value verbatim, never degrading. The legs-collapse-to-`Mesh` rule below (for
//! non-Deferred paths, pre-SDF-forward-march) is unaffected — it fires on a DIFFERENT condition
//! (`path != Deferred`) and stays live until `SDF_FORWARD_IMPLEMENTED` lands.
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
//! `GpuSceneBundles::scene()` → a field on `GBufferScene`, where R2's per-path declarator
//! dispatch reads it. (This paragraph said "DEAD-BUT-THREADED at R1 — nothing reads it downstream
//! yet" for several rungs after R2 landed.) The [`ResolvedRenderPath`] **Resource** is a separate
//! question, and there the R1 answer still holds — see that type's own doc.

use boyko_log::codes::{OnceSite, W2205};
use boyko_macros::Resource;

// ---- rung-staged implementation flags (plan §H) ---------------------------------------

/// Whether the `Forward` [`RenderPath`] has a landed declarator/pipeline yet. `true` as of
/// rung R4b-b: `graph_bridge.rs::declare_frame_graph`'s `Forward` arm now dispatches to
/// `declare_forward_graph`, the `TargetsProfile::ForwardMesh` allocation exists, the boot-time
/// pipeline/descriptor-set creation is wired (`boyko_app::gpu_scene`), and `record_forward`
/// records the pass. R4b-a landed this module's [`cap_forward_v1_consumers`] scope cut,
/// `forward_opaque.{vs,fs}.hlsl`, and `boyko_render::view::forward_view_proj_rows`; R4b-b landed
/// the GPU wiring half that makes `Forward` an actually-reachable path — flipping this const was
/// gated on that declarator existing (a `Forward` resolve before R4b-b landed would have hit
/// `declare_frame_graph`'s `unreachable!()` arm on the very first frame). Every `Forward` request
/// now resolves to `RenderPath::Forward` (subject to [`cap_forward_v1_consumers`]'s scope cut —
/// pre-light consumers + TAA still degrade until a later rung lands their producers).
const FORWARD_IMPLEMENTED: bool = true;

/// Whether the `ForwardPlus` [`RenderPath`] has a landed declarator/pipeline yet. `true` as of
/// rung R5: `declare_forward_graph`/`record_forward` (`boyko_rhi_vulkan::present`) now serve
/// BOTH `Forward` and `ForwardPlus` (Decision 2's shared per-path declarator — the SAME
/// framegraph/recorder machinery selects the depth-prepass + EQUAL-depth + froxel-FS variants
/// when the resolved path is `ForwardPlus`, mesh-only via `Forward`'s own pre-R-SDFFWD legs
/// collapse). `cap_forward_v1_consumers` was WIDENED (below) to also cap `ForwardPlus`'s
/// pre-light consumers + TAA off this rung — the prepass lands ONLY for its own zero-overdraw
/// early-Z contract (Decision 4), not yet for a consumer producer chain (that is a LATER rung's
/// scope, `needs_depth_prepass` staying the sole ForwardPlus-unconditional trigger).
const FORWARD_PLUS_IMPLEMENTED: bool = true;

/// Whether the `VisibilityBuffer` [`RenderPath`] has a landed declarator/pipeline yet. `true`
/// as of rung R8: `declare_vb_graph`/`record_vb` (`boyko_rhi_vulkan::present`) now serve the
/// fused (`mesh_geo_shade_split == false`) VB path — `vb_raster` (R32G32_UINT id + HW
/// reverse-Z raster) followed by `vb_resolve` (the fused compute fetch+bary+shade). A
/// `VisibilityBuffer` request now resolves for real, subject to [`cap_vb_v1_consumers`]'s
/// scope cut (pre-light consumers + TAA still degrade until R9 lands their thin-aux
/// producers). As of rung R10 [`VB_SDF_IMPLEMENTED`] is also `true`, so the SDF leg no longer
/// collapses to `Mesh` under VB.
const VB_IMPLEMENTED: bool = true;

/// Whether the `VisibilityBuffer` path's SDF leg (fused `sdf_forward_march` composited via the
/// view-Z ownership gate, or the future geo/shade split) has landed under VB specifically.
/// `true` as of rung R10: `declare_vb_graph`/`record_vb` now declare/record the SAME
/// `sdf_forward_march` compute pass the Forward family uses — the `HAS_MESH` variant composited
/// AFTER `vb_resolve` under `Both`, the mesh-less variant as the sole `lit` producer over
/// `vb_sky` under `Sdf` (where `vb_raster`/`vb_resolve` are `mesh_leg`-gated OFF entirely — they
/// need the Decision-0 geometry table, which carries no slot with no mesh leg). A
/// `VisibilityBuffer × {Both, Sdf}` request therefore resolves CLEAN now, arming
/// `sdf_forward_marched` (`resolve_rules`) exactly as `Forward × {Both, Sdf}` does.
///
/// Like `SDF_FORWARD_IMPLEMENTED`'s effect on the older `LegsCollapsedToMeshPreSdfForward` rule,
/// flipping this `true` makes [`degrade_ladder`]'s VB-scoped legs-collapse rule
/// ([`RenderPathDegrade::LegsCollapsedToMeshPreVbSdf`]) unconditionally dead in production — the
/// rule stays parameterized (threaded `false` by its isolation unit test) but is never reached
/// through the public [`resolve_render_path`] entry point again.
const VB_SDF_IMPLEMENTED: bool = true;

/// Whether the SDF-forward-march leg (Decision 6, the fused march-then-shade or the geo/shade
/// split under a non-Deferred path) has landed yet. `true` as of rung R-SDFFWD: the
/// `sdf_forward_march` compute pass (`boyko_rhi_vulkan::present::graph_bridge::declare_forward_graph`'s
/// `sdf_forward_march` arm + `record_forward`) is now the sole `lit` producer for the SDF leg
/// under a Forward-family path — a `Both`/`Sdf` request no longer degrades. `false` before this
/// rung meant any non-Deferred path requesting a non-[`GeometryLegs::Mesh`] leg set (`Both` or
/// `Sdf`) degraded that leg set to `Mesh` ([`RenderPathDegrade::LegsCollapsedToMeshPreSdfForward`]);
/// that degrade is now UNREACHABLE for `Forward`/`ForwardPlus` (kept live for a hypothetical
/// future path family via [`degrade_ladder`]'s own parameterized `sdf_forward_implemented` arg).
const SDF_FORWARD_IMPLEMENTED: bool = true;

/// Rung R9b honesty gate: the SDF geo/shade split (R-SDFSPLIT — `sdf_geo`/`sdf_shade` +
/// the SDF surface cache) has NOT landed. Before R9b this was unreachable through the public
/// entry point anyway (every pre-light consumer was capped under the non-Deferred paths), but
/// `ssao_on` now SURVIVES the VB cap on mesh-carrying leg sets, so an un-gated
/// `sdf_geo_shade_split = sdf_forward_marched && pre_light` would arm a flag whose producer
/// chain does not exist — a lie any future `ResolvedRenderPathGpu` consumer would trust
/// (the SAME hazard class the [`FORWARD_IMPLEMENTED`]-family consts exist to prevent). Flip
/// to `true` when R-SDFSPLIT lands its `sdf_geo`/`sdf_shade` producers.
const SDF_SPLIT_IMPLEMENTED: bool = false;

// Multi-paradigm render-path plan: `Deferred`'s per-leg disable landed both legs — `Sdf` at rung
// R3 (`mesh_depth_neutral_clear`), `Mesh` at rung R3b (`viewt_from_depth`, this module's doc).
// The rung-staged consts that used to gate them (`DEFERRED_SDF_ONLY_IMPLEMENTED` /
// `DEFERRED_MESH_ONLY_IMPLEMENTED`) and the degrade rule that read them were REMOVED once both
// landed (dead code — see this module's doc): `Deferred` now honors any `GeometryLegs` value
// verbatim, no degrade ladder step needed.

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

// ---- RenderPathFrozenConsumers (the boot-freeze snapshot — rung R9a, plan P2-d) --------

/// The boot-frozen pre-light consumer snapshot. Under a NON-Deferred resolved path the
/// pre-light consumer set commits at [`resolve_render_path`] (the framegraph/targets shape is
/// boot-locked, Decision 1), so a runtime flip of the corresponding config Resource must be a
/// warn-once NO-OP — otherwise the per-frame derivations (`ResolvedSsao`, the light-header
/// `ssao_mode` word) drift from the boot-shaped graph: the header would tell the shade to
/// COMBINE an SSAO term whose gather pass was never armed, folding seeded-UNDEFINED garbage
/// into the ambient term (visible corruption with ZERO validation errors, since the layout is
/// legal). ONE clamp helper — [`effective_ssao_config`] — applied by EVERY per-frame reader of
/// [`SsaoConfig`](crate::ssao_config::SsaoConfig) keeps all readers in lock-step with the boot
/// truth by construction (one truth, N readers; no reader-ordering constraint needed).
///
/// Under Deferred the freeze is INERT (`non_deferred == false`): live consumer toggles stay
/// free there — the fat G-buffer materializes every aux lane regardless of the consumer set.
///
/// Inserted as an inert [`Default`] by `SsaoPlugin` (this kernel has no `Option<Res<R>>`
/// SystemParam — the insert-default-then-boot-override discipline `ResolvedRenderPath`'s own
/// World Resource uses), then OVERWRITTEN once by the boot resolver's caller
/// (`boyko_app::runner`) with the real snapshot. Grows one field per consumer as the R9
/// stages land (SSAO at R9a/R9b; DDGI at R9c; shadow-denoise at R9d).
#[derive(Resource, Debug, Default)]
pub struct RenderPathFrozenConsumers {
    /// The boot-time [`SsaoConfig`](crate::ssao_config::SsaoConfig) snapshot — the FULL config
    /// (quality + à-trous levels), not just the consumer bit, so the frozen arming keeps the
    /// boot quality/levels too.
    pub ssao: crate::ssao_config::SsaoConfig,
    /// Rung R9c: the boot-time DDGI consumer bit (`DdgiConfig::enabled()` at boot). The frozen
    /// arming needs only the BIT — every other DDGI knob is owner-locked at the activation
    /// site, not a per-frame consumer toggle.
    pub ddgi_on: bool,
    /// Whether the freeze is active (`resolved_render_path.path != Deferred`). The inert
    /// default is `false`.
    pub non_deferred: bool,
    /// Warn-once latch (interior mutability: every reader holds a shared `Res` borrow).
    ///
    /// Rung L8a replaced the hand-rolled `AtomicBool` with [`OnceSite`], and the reason is a
    /// **per-frame RMW**, not tidiness. The old form was `!latch.swap(true, Relaxed)`, which is
    /// unconditional: once the config had actually diverged, the `&&`'s left side stayed true
    /// forever, so every per-frame reader stored `true` over `true` on a shared line for the rest
    /// of the session — from inside an `#[inline]` function whose whole purpose is to be cheap.
    /// [`OnceSite::claim`] loads first and short-circuits, so the steady state after the first
    /// warning is one `Relaxed` load of a line nobody writes.
    ///
    /// It stays a FIELD rather than becoming a `static`: the snapshot is per World, and a
    /// process-global latch would let the first world to diverge silence every other world's
    /// first divergence.
    warned_ssao: OnceSite,
    /// Rung R9c: the DDGI warn-once latch (the SAME discipline as `warned_ssao`).
    warned_ddgi: OnceSite,
}

impl RenderPathFrozenConsumers {
    /// The real boot snapshot — built by the boot resolver's caller right after
    /// [`resolve_render_path`], from the SAME config Resources the boot
    /// [`RenderPathConsumers`] was assembled from.
    #[inline]
    pub fn new(ssao: crate::ssao_config::SsaoConfig, ddgi_on: bool, non_deferred: bool) -> Self {
        Self {
            ssao,
            ddgi_on,
            non_deferred,
            warned_ssao: OnceSite::new(),
            warned_ddgi: OnceSite::new(),
        }
    }
}

/// Rung R9c: the DDGI half of the freeze clamp — returns the effective "DDGI enabled" bit
/// every per-frame reader must act on (the live one when the freeze is inert, the BOOT bit
/// when frozen; warn-once on the first observed divergence). The SAME one-truth/N-readers
/// contract as [`effective_ssao_config`].
#[inline]
pub fn effective_ddgi_enabled(live: bool, frozen: &RenderPathFrozenConsumers) -> bool {
    if !frozen.non_deferred {
        return live;
    }
    if live != frozen.ddgi_on && frozen.warned_ddgi.claim() {
        report_frozen_config_divergence("DdgiConfig");
    }
    frozen.ddgi_on
}

/// THE freeze clamp (rung R9a): returns the [`SsaoConfig`](crate::ssao_config::SsaoConfig) every per-frame reader must act
/// on — the live one when the freeze is inert (Deferred, or a world that never boot-resolved
/// a path), the BOOT snapshot when frozen. Warns ONCE (across all readers, via the snapshot's
/// own latch) on the first observed divergence. Pure and allocation-free.
#[inline]
pub fn effective_ssao_config<'a>(
    cfg: &'a crate::ssao_config::SsaoConfig,
    frozen: &'a RenderPathFrozenConsumers,
) -> &'a crate::ssao_config::SsaoConfig {
    if !frozen.non_deferred {
        return cfg;
    }
    if (cfg.quality != frozen.ssao.quality || cfg.atrous_levels != frozen.ssao.atrous_levels)
        && frozen.warned_ssao.claim()
    {
        report_frozen_config_divergence("SsaoConfig");
    }
    &frozen.ssao
}

/// Reports `boyko-W2205` — a config the caller changed after boot that the frozen consumer set
/// will not act on, so the change silently does nothing.
///
/// `#[cold]` + `#[inline(never)]` because both callers are `#[inline]` per-frame readers whose
/// straight-line code must stay the two compares and a return: the whole emission body, including
/// the format arguments, lives out of line behind a branch the predictor never takes twice.
#[cold]
#[inline(never)]
fn report_frozen_config_divergence(what: &str) {
    boyko_log::warn!(
        boyko_log::Render,
        W2205,
        "{} changed at runtime, but the pre-light consumer set is FROZEN under a non-Deferred \
         render path -- the boot value stays in effect (set the config before boot to change it)",
        what
    );
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

/// Multi-paradigm render-path plan §F / §C sync-pin (rung R8): the SDF-owned pixel marker
/// stamped into `vb_id.R` (the `instance_id` channel) — a real per-frame instance index never
/// reaches `u32::MAX` (the instance ring is bounded far below it), so this value is
/// unambiguous. The shader-side mirror lives in `vb_pack.hlsli`'s `VB_ID_SENTINEL` — both sides
/// MUST agree bit-for-bit (pinned by a unit test in this module).
pub const VB_ID_SENTINEL: u32 = 0xFFFF_FFFF;

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

    /// Decision 7's structural exclusion: [`Self::SDF_SOFT_MARCH`] and [`Self::HWRT_VIS`] are
    /// never armed together. [`resolve_rules`] gates the former on
    /// `!consumers.hwrt_denoise_or_vis_on` and the latter on that SAME flag being true, so one
    /// boolean decides both with opposite polarity — the exclusion is a property of the
    /// resolver, not of any caller.
    ///
    /// Named rather than left implicit in those two `if`s because it is LOAD-BEARING at a site
    /// that cannot see the resolver: the two `-D HWRT=1` `vb_shade_split` variants
    /// (docs/SHADER-VARIANT-MANIFEST.md) are SELECTED on `hwrt_denoise_or_vis_on`, so the
    /// exclusion is what keeps the shipped variant matrix complete with respect to this field's
    /// arm space. Break it and the resolver records a shadow-source combination no shipped row
    /// expresses — an arm that binds cleanly, raises no validation message, and is silently
    /// ignored.
    ///
    /// Not to be misread as a defect in those two rows: `vb_shade_split.comp.hlsl` carries no
    /// `sdf_soft_shadow` arm in ANY of its four variants — `vis` starts at `1.0` and min-combines
    /// `gShadowVis` under `#if HWRT` or `csm_visibility` otherwise. So **MESH pixels under VB**
    /// receive no SDF-cast shadow: a v1 scope cut, deliberate, and unchanged by this exclusion.
    ///
    /// ⚠️ Two narrower statements replace two wider ones that were false. `deferred_pbr.hlsl` is
    /// NOT "the only shader in the tree that implements that march" — four shaders define one
    /// (`sdf_forward_march.comp.hlsl:297`, `sdf_gbuffer_composite.hlsl:498` and `:591`,
    /// `sdf_probe_update.comp.hlsl:160` alongside `deferred_pbr.hlsl:515`), and the repo even
    /// carries `sdf_soft_shadow_ranged_copy_matches_resolve` to pin one of those copies. Nor does
    /// "a VB frame never combines an SDF-march source" hold: a VB×Both / VB×Sdf frame records the
    /// same `sdf_forward_march` compute pass the Forward family uses, and that pass marches its
    /// own soft shadow for the primary directional. The scope cut is about MESH pixels, not about
    /// the frame.
    #[inline]
    pub const fn hwrt_vis_excludes_sdf_soft_march(self) -> bool {
        !(self.contains(Self::SDF_SOFT_MARCH) && self.contains(Self::HWRT_VIS))
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
    ///
    /// # What this field is, and what it is NOT
    ///
    /// A RECORDED boot decision that is CHECKED, not a dispatch input. No pass selects itself
    /// from these bits, and wiring one to do so would change behaviour rather than centralise
    /// it: each bit is a boot predicate over *config* (`CSM` ⇐ `CsmConfig::enabled()`,
    /// `PUNCTUAL_ATLAS` ⇐ `ShadowConfig::enabled()`), while the corresponding per-frame gate is
    /// strictly STRONGER — `boyko_app`'s `csm_armed` additionally requires a fitted sun AND live
    /// caster batches, `punctual_armed` a fitted atlas AND the same, and the hwrt chain a
    /// `HardwareTri` backend AND a non-empty TLAS. A config-enabled CSM in a frame with no
    /// casters sets this bit and runs no cascade pass; that is correct, not a divergence.
    ///
    /// So the sound relation is CONTAINMENT — the bit is a necessary condition of every
    /// per-frame arm, never a sufficient one — and that is what the checks assert:
    ///
    /// * `csm_config`'s `a_live_csm_mode_word_implies_the_boot_shadow_source_bit` and
    ///   `shadow_atlas`'s `a_live_atlas_mode_word_implies_the_boot_shadow_source_bit` pin
    ///   `pass gate ⇒ bit` for `CSM` / `PUNCTUAL_ATLAS`, driving the REAL `resolve_csm` /
    ///   `resolve_shadow_atlas_spots` rather than a restatement of them.
    /// * [`ShadowSources::hwrt_vis_excludes_sdf_soft_march`] and its sweep
    ///   (`sdf_soft_march_and_hwrt_vis_stay_exclusive_over_the_whole_input_space`) pin the
    ///   `SDF_SOFT_MARCH` ⊥ `HWRT_VIS` exclusion that the `vb_shade_split_*hwrt` shader variants
    ///   depend on for their reachability (docs/SHADER-VARIANT-MANIFEST.md).
    ///
    /// The `SDF_SOFT_MARCH` bit is additionally read through the GPU mirror at the
    /// `vb_shade_split_*hwrt` pipeline-selection site, by a `debug_assert!` — the ONE place a
    /// bit of this field is load-bearing, and as an assertion only.
    pub shadow: ShadowSources,
    /// `consumers.clusters_wanted && path == VisibilityBuffer` — the SINGLE boot-frozen arm bit
    /// gating the ENTIRE froxel light-cull machinery (the app-side cluster build, the VB
    /// `_froxel` pipeline selection, AND the `light_cull` graph pass) — VB-ONLY
    /// (`ForwardPlus`/`Deferred` keep their own, unrelated `cluster_cull` scaffolding untouched).
    /// `clusters_wanted` (VB-P1b) reads the booted scene's
    /// [`LightingConfig::clusters_enabled`](crate::light::LightingConfig::clusters_enabled),
    /// DEFAULT `false` — an unarmed scene (every scene that never opts in) builds/declares/
    /// records nothing here, so every pre-VB-P1b golden stays byte-identical (the 0%-gate).
    pub froxel_light_cull: bool,
    /// The device's RG8-UNORM `STORAGE_IMAGE` fact (`DeviceCaps::rg8_unorm_storage_ok`), copied
    /// at resolve: the SV0 `sdf_term` ring is an RG8 STORAGE target, so a device without the
    /// format feature cannot host the prepass — [`Self::vb_sdf_mesh_armable`] conjoins this.
    /// Defaults `true` through [`RenderPathDeviceCaps::default`] (the near-universal desktop
    /// answer; the host boot seam overrides with the real probe).
    pub vb_sdf_mesh_storage_ok: bool,
}

// P2-d note: "frozen at boot" here means [`resolve_render_path`] is never RE-CALLED after
// boot — R1 has no runtime re-resolution mechanism at all, so there is nothing yet to make a
// stale-consumer toggle a no-op against. The plan's validation-table row "runtime toggle of a
// frozen pre-light consumer (non-Deferred) → no-op, warn-once" describes a GUARD the first
// rung that actually CONSUMES `thin_aux`/`shadow` structurally (R4+, once a declarator reads
// these fields to decide framegraph shape) must add, not a missed R1 deliverable — R1's own
// gate is simply "resolved once, never re-derived," which [`resolve_render_path`] already is.

impl ResolvedRenderPath {
    /// TAA support matrix — the SINGLE predicate every TAA gate consumes
    /// (`GpuSceneBundles::scene`'s `AaMode::Taa` degrade and `boyko_app::runner`'s
    /// `taa_armed_now` arm-state both read THIS method, so the gates can never disagree — a
    /// split-brain half-armed state means jitter with no accumulator, or an armed `aa_out`
    /// with no matching dispatch):
    ///
    /// * `Deferred` (any legs) — the original TAA home: jittered raster push + marcher-owned
    ///   `gViewT` + the resolve's camera-differential reprojection.
    /// * `VisibilityBuffer` (any legs) — the jittered reverse-Z push
    ///   (`forward_view_proj_rows_jittered`) + the per-leg `gViewT` producer split that
    ///   mirrors Deferred's own: `viewt_from_depth_rz` covers the marcher-less `VB × Mesh`
    ///   config, and on the SDF-carrying legs (`Both`/`Sdf`) the `VIEWT`-variant
    ///   `sdf_forward_march` IS the composite and the SOLE producer (SDF-owned `t`,
    ///   mesh-owned `t_mesh`, background sentinel — the `sdf_gbuffer_composite` u8
    ///   discipline); the unchanged `taa_resolve` accumulates.
    /// * `Forward`/`ForwardPlus` — unsupported (no AA seam exists in that recorder at all;
    ///   its own future rung).
    #[inline]
    pub fn taa_supported(&self) -> bool {
        matches!(self.path, RenderPath::Deferred | RenderPath::VisibilityBuffer)
    }

    /// Whether this path's recorder contains a post-process AA seam **at all** — i.e. whether
    /// anything will ever write `aa_out` on this path.
    ///
    /// This is the predicate the FXAA / SMAA / SSAA / TAA *activation* must consult, and it is
    /// deliberately **separate from [`Self::taa_supported`] even though the two currently select
    /// the same paths**. They answer different questions and are free to diverge: `taa_supported`
    /// asks whether the temporal machinery exists (jitter push, history, `gViewT` producer),
    /// while this asks the strictly weaker question of whether the recorder has an AA block to
    /// hand `aa_out` to. A future Forward AA seam would flip this one first and `taa_supported`
    /// later — collapsing them into one predicate today would silently re-arm TAA at that moment.
    ///
    /// The distinction matters because `targets.rs` arms `aa_out` on
    /// `scene.aa || scene.smaa || scene.ssaa || scene.taa` with **no path term**, and the present
    /// blit repoints every slot at `aa_out` whenever it is `Some`. So on a path whose recorder
    /// writes nothing, an armed `aa_out` is presented **uninitialised** — the exact failure the
    /// `AaMode::Taa` degrade was written to prevent, which until now was prevented for TAA only
    /// while FXAA / SMAA / SSAA armed as usual.
    ///
    /// * `Deferred` — `passes/gbuffer.rs` records all four.
    /// * `VisibilityBuffer` — `passes/vb.rs` records all four.
    /// * `Forward`/`ForwardPlus` — `passes/forward.rs` records **none**, and
    ///   `graph_bridge.rs::declare_forward_graph` declares no AA pass either.
    #[inline]
    pub fn post_process_aa_supported(&self) -> bool {
        matches!(self.path, RenderPath::Deferred | RenderPath::VisibilityBuffer)
    }

    /// **VB-SV0 (`docs/VB-SV0-SDF-SHADOW-PLAN.md` §S4): whether this boot's VB lit producer is
    /// structurally capable of running the SDF-on-mesh shadow / contact-AO terms.**
    ///
    /// The SINGLE capability predicate every SV0 consumer reads —
    /// [`sync_sv0_light_gate`](crate::light::sync_sv0_light_gate) clamps the owner's request
    /// against it, and the `sv0_never_arms_under_hwrt` truth table is quantified over it.
    ///
    /// # It CONSUMES the already-resolved shadow bit; it does not re-derive it
    ///
    /// [`ShadowSources::SDF_SOFT_MARCH`] is resolved exactly once, in [`resolve_rules`], as
    /// `sdf_leg && consumers.sdf_shadows_wanted && !consumers.hwrt_denoise_or_vis_on`. Reading
    /// [`Self::shadow`] rather than restating those three terms is what keeps SV0 from drifting
    /// away from the engine's own answer to "is the SDF soft march the shadow source here?" — a
    /// mirrored derivation would have to be re-audited every time that rule moves.
    ///
    /// # Why the two extra terms
    ///
    /// * `path == VisibilityBuffer` — SV0 lives in the three VB lit-producer tails
    ///   (`vb_resolve` / `vb_shade` / `vb_shade_split`) and nowhere else. Deferred already ships
    ///   this visual through the marcher's own composite.
    /// * [`Self::mesh_leg`] — SV0 shades MESH pixels. On `VB × Sdf` there are none, so the term
    ///   would be quantified over an empty set.
    ///
    /// # Rows 9-10 of the §S4 variant matrix can never be armed, and that is structural
    ///
    /// The `_hwrt` split tails (`vb_shade_split_hwrt`, `vb_shade_split_tex_hwrt`) are selected
    /// exactly when the hardware shadow chain is armed, i.e. when `hwrt_denoise_or_vis_on` is
    /// `true` — which is precisely what [`ShadowSources::SDF_SOFT_MARCH`] requires to be `false`.
    /// So this predicate is `false` on every boot that binds them, with no separate `hwrt` term
    /// needed here.
    ///
    /// **That is a claim about the BOOT RESOLVER, and the record site is where the pipeline is
    /// actually chosen** (code-review P1-a). The two are joined by carrying this predicate's
    /// ANSWER down: `boyko_app::gpu_scene::to_gpu_resolved_render_path` calls it and copies the
    /// result into `ResolvedRenderPathGpu::vb_sdf_mesh_armable`, and
    /// `boyko_rhi_vulkan`'s `note_vb_lit_producer` `debug_assert!`s that no `_hwrt` producer is
    /// ever the bound pipeline while it is `true`. So an edit that makes a split-hwrt pipeline
    /// bindable on an SV0-armable boot fires an assertion instead of quietly landing in the gap
    /// between the resolver's claim and the recorder's choice.
    ///
    /// For the claim to stop holding, `resolve_rules` would have to drop the
    /// `!consumers.hwrt_denoise_or_vis_on` term from `SDF_SOFT_MARCH`'s arming (which reds
    /// [`tests::sv0_never_arms_under_hwrt`]) or `GBufferScene::path_vb_hwrt_shadow` would have to
    /// stop implying it (which reds the record-site `debug_assert!` on the first such frame).
    #[inline]
    pub fn vb_sdf_mesh_armable(&self) -> bool {
        matches!(self.path, RenderPath::VisibilityBuffer)
            && self.shadow.contains(ShadowSources::SDF_SOFT_MARCH)
            && self.mesh_leg
            && self.vb_sdf_mesh_storage_ok
    }
}

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
    /// Whether the owner wants the VisibilityBuffer froxel light-cull machinery armed
    /// (`ResolvedRenderPath::froxel_light_cull`'s single boot-frozen gate). VB-P1b: the
    /// `boyko_app::runner` boot call site threads this from the booted scene's
    /// [`LightingConfig::clusters_enabled`](crate::light::LightingConfig::clusters_enabled),
    /// DEFAULT `false` (the 0%-gate — every scene that never sets it stays unarmed).
    pub clusters_wanted: bool,
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
    /// Whether `R8G8_UNORM` supports `STORAGE_IMAGE` under OPTIMAL tiling
    /// (`DeviceCaps::rg8_unorm_storage_ok`) — the SV0 `sdf_term` ring's write format. Absent ⇒
    /// [`ResolvedRenderPath::vb_sdf_mesh_armable`] resolves `false` (degrade-not-panic: the
    /// request clamps, the prepass never exists, and the tails never READ the ring — their
    /// term read sits behind the mode gate, and mode is 0 on every frame of such a boot).
    pub rg8_unorm_storage: bool,
}

impl Default for RenderPathDeviceCaps {
    #[inline]
    fn default() -> Self {
        // Assume supported until the host overrides with the real boot query (mirrors
        // `DdgiCaps::default`'s "most desktop GPUs support it" rationale) — a bench/test
        // harness that never queries the device wants the enabled path.
        Self { storage_buffer_array_non_uniform_indexing: true, rg8_unorm_storage: true }
    }
}

impl RenderPathDeviceCaps {
    /// Builds the caps from a device query result (the host boot seam). RG8 storage defaults
    /// `true` here for the eight pre-SV0 callers; the host chains
    /// [`with_rg8_unorm_storage`](Self::with_rg8_unorm_storage) with the real probe.
    #[inline]
    pub const fn new(storage_buffer_array_non_uniform_indexing: bool) -> Self {
        Self { storage_buffer_array_non_uniform_indexing, rg8_unorm_storage: true }
    }

    /// Overrides the RG8-UNORM `STORAGE_IMAGE` fact with the device's real probe answer.
    #[inline]
    pub const fn with_rg8_unorm_storage(mut self, ok: bool) -> Self {
        self.rg8_unorm_storage = ok;
        self
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
    /// Rung R4b (Forward v1 scope cut; widened to [`RenderPath::ForwardPlus`] at rung R5): a
    /// pre-light consumer (SSAO/DDGI/shadow-denoise-spatial/shadow-temporal/SSR/
    /// hwrt-denoise-or-vis) was requested alongside a Forward-family path, which has no
    /// thin-aux producer yet — every one of those consumers is forced OFF for this resolve
    /// (`cap_forward_v1_consumers`). Under `ForwardPlus` this does NOT disable the depth
    /// prepass itself (`needs_depth_prepass` stays `true` unconditionally for that path) — only
    /// the consumer-driven `thin_aux`/split flags.
    ForwardPreLightConsumersNotYetImplemented,
    /// Rung R4b (Forward v1 scope cut; widened to [`RenderPath::ForwardPlus`] at rung R5): TAA
    /// was requested alongside a Forward-family path, which writes no motion vector yet — TAA
    /// is forced OFF for this resolve (`cap_forward_v1_consumers`).
    ForwardTaaNotYetImplemented,
    /// [`RenderPath::VisibilityBuffer`] was requested with a non-[`GeometryLegs::Mesh`] leg set
    /// (`Both`/`Sdf`) before `VB_SDF_IMPLEMENTED` landed (rung R10) — collapsed to
    /// [`GeometryLegs::Mesh`]. The VB-scoped sibling of
    /// [`LegsCollapsedToMeshPreSdfForward`](Self::LegsCollapsedToMeshPreSdfForward), needed
    /// because that OLDER rule reads `SDF_FORWARD_IMPLEMENTED` (`true` today), which makes it
    /// unconditionally dead for every non-Deferred path — including `VisibilityBuffer` — so it
    /// could not gate VB's own SDF leg while that was still unimplemented.
    ///
    /// As of rung R10 `VB_SDF_IMPLEMENTED` is `true`, so THIS rule is now unconditionally dead
    /// in production too (like its Forward-family sibling): it is only ever constructed by the
    /// `degrade_ladder` isolation unit test, which threads `vb_sdf_implemented = false`
    /// directly. Kept as a live variant so that test — and any future device that must fall back
    /// off the VB SDF path — retains a typed degrade reason.
    LegsCollapsedToMeshPreVbSdf,
    /// Rung R8 (VB v1 scope cut, mirrors [`ForwardPreLightConsumersNotYetImplemented`](Self::ForwardPreLightConsumersNotYetImplemented)):
    /// a pre-light consumer was requested alongside `VisibilityBuffer`, which has no split
    /// (`vb_geo`/`vb_shade`) thin-aux producer yet (only the fused `vb_resolve`) — every one of
    /// those consumers is forced OFF for this resolve (`cap_vb_v1_consumers`).
    VbPreLightConsumersNotYetImplemented,
    // (The former `VbTaaNotYetImplemented` variant was DELETED by the TAA-under-VB rungs: first
    // narrowed to the SDF-carrying legs when VB×Mesh TAA landed, then removed outright once the
    // `VIEWT`-variant `sdf_forward_march` became the gViewT producer for `Both`/`Sdf` — TAA now
    // passes through `cap_vb_v1_consumers` uncapped on every VB leg set, and a variant no rule
    // can construct would be dead API surface.)
}

/// A fixed-capacity log of [`RenderPathDegrade`] reasons. `degrade_ladder`'s THREE rules fire
/// AT MOST ONE per call: `PathNotYetImplemented` XOR `VbDeviceCapMissing` (the latter needs
/// `path_implemented == true`, which the former's arm never reaches) are the only path-level
/// rules, and EITHER of them demotes the FINAL path to [`RenderPath::Deferred`] — which is
/// exactly the condition [`LegsCollapsedToMeshPreSdfForward`](RenderPathDegrade::LegsCollapsedToMeshPreSdfForward)
/// requires to be FALSE (`path != Deferred`) to fire, so a path-level demotion and the legs-level
/// rule can never co-occur. (Multi-paradigm render-path plan, rung R3b: `Deferred`'s own
/// per-leg-disable rule — the one case that USED to co-fire with a path-level demotion — was
/// removed once both `Deferred` legs landed; see this module's doc.)
///
/// Rung R4b added `cap_forward_v1_consumers`, a FOURTH (independent) rule that COULD co-occur
/// with `LegsCollapsedToMeshPreSdfForward` while `SDF_FORWARD_IMPLEMENTED` was still `false`:
/// `RenderPath::Forward × {Both, Sdf}` with a pre-light consumer AND TAA both requested stacked
/// THREE reasons in one call (legs-collapse + `ForwardPreLightConsumersNotYetImplemented` +
/// `ForwardTaaNotYetImplemented`). Rung R-SDFFWD lifted `SDF_FORWARD_IMPLEMENTED` to `true`, so
/// `LegsCollapsedToMeshPreSdfForward` is UNREACHABLE through the public [`resolve_render_path`]
/// entry point today (every non-Deferred path either demotes to `Deferred` at the PATH level —
/// never reaching the legs-level rule — or is SDF-forward-implemented and never triggers it); it
/// stays live only as a rule `degrade_ladder` itself still encodes (tested directly, a future path
/// family could reintroduce the 3-stack). `cap_forward_v1_consumers` alone still stacks TWO
/// reasons (`ForwardPreLightConsumersNotYetImplemented` + `ForwardTaaNotYetImplemented`) on a
/// `Forward`/`ForwardPlus` request today. The array stays sized to 4 slots (headroom for a future
/// rule combination; a fixed array needs no heap allocation either way, Principle 5).
/// `boyko_app::runner` iterates [`Self::reasons`] and `warn!`s each ONCE.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderPathDegradeLog {
    reasons: [Option<RenderPathDegrade>; 4],
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
        // invariant: no combination of the degrade ladder + the rung-staged consumer caps
        // (`cap_forward_v1_consumers`) emits more than 4 reasons in one resolve — a 5th push is
        // a ladder/cap bug, not user error, so this is a debug-only invariant check (Principle
        // 8's "panic only on invariant violation" — vanishes in release, matching every other
        // hot-path guard).
        debug_assert!(false, "invariant: RenderPathDegradeLog overflowed its 4-slot capacity");
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
/// it is directly unit-testable against a HYPOTHETICAL fully-landed path (e.g. `ForwardPlus`/
/// `VisibilityBuffer`, still `false` as of R4b-b) — the Rev-5 MOTION-only `prepass_writes_motion`
/// rule must be provably correct TODAY for those paths too, not merely once their own rung lands.
/// `Forward` itself is real as of R4b-b (`FORWARD_IMPLEMENTED` `true`), so its own truth table
/// is exercised through the public [`resolve_render_path`] entry point (this module's tests).
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

    // Multi-paradigm render-path plan, rung R-SDFFWD: `mesh_leg` gates the prepass too — its
    // ENTIRE purpose is the zero-overdraw early-Z contract for `forward_opaque`'s MESH draws
    // (Decision 4); under `GeometryLegs::Sdf` (no mesh leg) there is nothing for it to cull, so a
    // `ForwardPlus`-resolved `Sdf`-only frame would otherwise record a begin/end depth-only pass
    // that draws (and reads back) NOTHING — a wasted GPU round-trip, not a correctness bug (the
    // SAME single predicate this fn computes is read at BOTH `declare_forward_graph` and
    // `record_forward` via `GBufferScene::path_needs_depth_prepass`, so declaring the pass OFF
    // here means it is never recorded either — no separate gate needed at either call site).
    let needs_depth_prepass = mesh_leg
        && (matches!(path, RenderPath::ForwardPlus) || (matches!(path, RenderPath::Forward) && pre_light));
    // Decision 8: the prepass ALSO writes motion only when a PRE-LIGHT motion consumer
    // (shadow_temporal) is armed; a TAA-only (post-light) config keeps the cheaper
    // mesh_forward-writes-motion form (not encoded here — a per-pass framegraph decision).
    let prepass_writes_motion = needs_depth_prepass && consumers.shadow_temporal_on;

    let sdf_forward_marched = sdf_leg && !matches!(path, RenderPath::Deferred);
    // Rung R9a: `mesh_leg` gates the VB split too (the R-SDFFWD "mesh_leg gates the prepass"
    // precedent above): the split's ENTIRE purpose is separating the MESH raster's geometry
    // fetch from its shade — under `GeometryLegs::Sdf` there is no `vb_raster`/`vb_id` to
    // split, so a mesh-less pre-light config must not arm `vb_geo`/`vb_shade_split` (the
    // plan-doc's literal "VB && pre_light" rule carries a recorded erratum for this).
    let mesh_geo_shade_split =
        matches!(path, RenderPath::VisibilityBuffer) && mesh_leg && pre_light;
    // Gated on the R-SDFSPLIT rung const (see [`SDF_SPLIT_IMPLEMENTED`]'s doc): the RULE is
    // `sdf_forward_marched && pre_light`, but the flag must not arm while no producer exists.
    let sdf_geo_shade_split = sdf_forward_marched && pre_light && SDF_SPLIT_IMPLEMENTED;
    let sdf_surface_cache = sdf_geo_shade_split;

    let vb_geometry_table = matches!(path, RenderPath::VisibilityBuffer)
        && mesh_leg
        && caps.storage_buffer_array_non_uniform_indexing;

    let depth_kind =
        if matches!(path, RenderPath::Deferred) { DepthKind::CustomLinear } else { DepthKind::HardwareReverseZ };

    let mut thin_aux = ThinAuxMask::NONE;
    // Rung R9a: `hwrt_denoise_or_vis_on` joins the NORMAL union on NON-Deferred paths — the
    // hardware `shadow_vis` gather READS a per-pixel normal for its cone-trace origin/bias.
    // Under Deferred it reads the fat `gNormal` G-buffer lane (no thin-aux image involved —
    // arming unchanged there, the Deferred truth-table rows and pins hold), but on a thin-aux
    // path (VB split / Forward prepass) its normal source IS `thin_normal`, so the vis pass
    // is a NORMAL consumer there. Consequence (recorded as a plan-doc erratum): the Rev-5
    // "Temporal-only (MOTION-only arming)" config arms `NORMAL|MOTION` on non-Deferred paths,
    // which also makes `mesh_geo_shade_split ⇒ NORMAL` hold for every hwrt-armed split config.
    if consumers.ssao_on
        || consumers.ddgi_on
        || consumers.shadow_denoise_spatial_on
        || consumers.ssr_on
        || (consumers.hwrt_denoise_or_vis_on && !matches!(path, RenderPath::Deferred))
    {
        thin_aux = thin_aux.insert(ThinAuxMask::NORMAL);
    }
    // TAA arms the MOTION channel ONLY under Deferred (whose raster/marcher own a motion
    // producer story). VB TAA (every leg set) reconstructs reprojection camera-differentially
    // from the gViewT lane (`viewt_from_depth_rz` on Mesh, the VIEWT-variant marcher composite
    // on Both/Sdf) — arming a producer-less MOTION channel there would declare an aux image
    // nothing writes (the bound-but-UNDEFINED 09600 class).
    if (consumers.taa_on && matches!(path, RenderPath::Deferred)) || consumers.shadow_temporal_on {
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
    // Decision 7's exclusion, asserted at the ONE site that can violate it: the two `if`s above
    // read the SAME `hwrt_denoise_or_vis_on` with opposite polarity, so any future edit that
    // splits them (a per-path carve-out, a second hwrt flag) breaks the exclusion HERE and
    // nowhere else. Debug-only per the project's hot-path convention; the property is also
    // pinned unconditionally by
    // `sdf_soft_march_and_hwrt_vis_stay_exclusive_over_the_whole_input_space`, so a release-mode
    // test run is not blind to it.
    debug_assert!(
        shadow.hwrt_vis_excludes_sdf_soft_march(),
        "invariant (Decision 7): SDF_SOFT_MARCH and HWRT_VIS are mutually exclusive, got {shadow:?}"
    );

    // VB-P1a ("dark infra"): VB-ONLY this rung — `ForwardPlus`/`Deferred` keep their own,
    // unrelated `cluster_cull` scaffolding untouched (see this field's own doc).
    let froxel_light_cull = consumers.clusters_wanted && matches!(path, RenderPath::VisibilityBuffer);

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
        froxel_light_cull,
        vb_sdf_mesh_storage_ok: caps.rg8_unorm_storage,
    }
}

// ---- the degrade ladder (plan §A validation table + §H) --------------------------------

/// The rung-staged degrade ladder, parameterized by the "is this rung landed" flags (rather
/// than reading the module consts directly) so it stays independently unit-testable — the SAME
/// "test the rule directly" discipline [`resolve_rules`] uses for the Rev-5 predicate.
/// [`resolve_render_path`] is the ONLY caller that threads the real
/// [`FORWARD_IMPLEMENTED`]/[`FORWARD_PLUS_IMPLEMENTED`]/[`VB_IMPLEMENTED`]/
/// [`SDF_FORWARD_IMPLEMENTED`] consts. Every one of those is `true` today, so rule (1) below is
/// production-dead; parameterizing it anyway is what keeps the rule itself testable — a test
/// threads `false` to exercise a demotion the shipped consts can no longer produce.
///
/// Order: (1) an unimplemented path collapses to `Deferred`; (2) a `VisibilityBuffer` path that
/// survived (1) but whose device lacks the geometry-table cap ALSO collapses to `Deferred`; (3) a
/// FINAL `path != Deferred` requesting a non-`Mesh` leg set before the SDF-forward-march rung
/// lands collapses the legs to `Mesh`; (4) rung R8 — a FINAL `path == VisibilityBuffer`
/// requesting a non-`Mesh` leg set before VB's OWN SDF-composite rung (R10) lands ALSO collapses
/// the legs to `Mesh` ([`RenderPathDegrade::LegsCollapsedToMeshPreVbSdf`]). Rule (4) is a
/// VB-scoped sibling of rule (3), needed because rule (3)'s `sdf_forward_implemented` param is
/// `true` in production (`SDF_FORWARD_IMPLEMENTED`) — making rule (3) unconditionally dead for
/// EVERY non-Deferred path today, VB included — while VB's own SDF-composite wiring has not
/// landed. Rules (3)/(4) both require `legs != Mesh`, so whichever fires first already sets
/// `legs = Mesh`, making the other's guard false — they never double-fire. Rules (3)/(4) each
/// require `path != Deferred`, which rules (1)/(2)'s demotion (to `Deferred`) always contradicts
/// — so a path-level demotion and a legs-level collapse never co-occur (see
/// [`RenderPathDegradeLog`]'s doc). Never a panic — degrade-not-panic by construction. `Deferred`
/// itself never degrades its `legs` (Multi-paradigm render-path plan, rung R3b: both `Deferred`
/// legs are landed — see this module's doc for the removed per-leg rule).
fn degrade_ladder(
    requested_path: RenderPath,
    requested_legs: GeometryLegs,
    caps: RenderPathDeviceCaps,
    path_implemented: bool,
    sdf_forward_implemented: bool,
    vb_sdf_implemented: bool,
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
    if !matches!(path, RenderPath::Deferred) && !matches!(legs, GeometryLegs::Mesh) && !sdf_forward_implemented
    {
        degrades.push(RenderPathDegrade::LegsCollapsedToMeshPreSdfForward);
        legs = GeometryLegs::Mesh;
    }

    // Rung R8: VB's own SDF-composite gate (see this fn's doc, rule 4) — a fresh rule because
    // the one above is unconditionally dead for VB (and every other non-Deferred path) once
    // `sdf_forward_implemented` is `true` in production. Rung R10: `VB_SDF_IMPLEMENTED` is now
    // `true` too, so in production this rule is ALSO dead — it only fires when the isolation unit
    // test threads `vb_sdf_implemented = false`.
    if matches!(path, RenderPath::VisibilityBuffer) && !matches!(legs, GeometryLegs::Mesh) && !vb_sdf_implemented
    {
        degrades.push(RenderPathDegrade::LegsCollapsedToMeshPreVbSdf);
        legs = GeometryLegs::Mesh;
    }

    (path, legs, degrades)
}

// ---- Forward v1 scope cut (rung R4b, orchestrator-directed; widened to ForwardPlus at R5) ----

/// Rung R4b: `RenderPath::Forward`'s v1 declarator has NO depth prepass, NO thin-aux producer
/// (no `SSAO`/`DDGI`/shadow-denoise-spatial/shadow-temporal/SSR support — [`resolve_rules`]'s
/// `needs_depth_prepass` MUST stay `false` under plain `Forward` this rung), and writes no motion
/// vector (no TAA). Any pre-light consumer OR TAA requested alongside `RenderPath::Forward` is
/// forced OFF here — BEFORE [`resolve_rules`] computes `needs_depth_prepass`/`thin_aux`/
/// `mesh_geo_shade_split`/`sdf_geo_shade_split` from the (adjusted) [`RenderPathConsumers`] — so
/// those derived fields stay structurally `false`/empty under Forward today, exactly as if the
/// owner had never configured the consumer (the plan §A validation table's "pre-light consumer
/// plus prepass suppressed by app flag => consumer -> Off, warn" row, specialized to Forward
/// v1's full scope cut). `hwrt_denoise_or_vis_on` is folded into the SAME cap as the five
/// `pre_light`-union consumers: it drives [`ShadowSources::HWRT_VIS`], which — like SSAO/DDGI —
/// needs its own pre-tail producer chain (`shadow_vis`/à-trous/temporal) that neither Forward
/// variant declares yet.
///
/// Rung R5 (ForwardPlus) WIDENED this cap to `RenderPath::ForwardPlus` too — SCOPE: the R5
/// `depth_prepass` exists for its OWN zero-overdraw early-Z contract (Decision 4) and the froxel
/// `#ifdef FROXEL` light reuse (Decision 6's `#ifdef FROXEL` seam), NOT yet for a pre-light
/// consumer producer chain (SSAO/DDGI/shadow-denoise/SSR still have no thin-aux MRT under either
/// Forward variant). `ForwardPlus`'s `needs_depth_prepass` stays `true` regardless of this cap
/// (`resolve_rules`'s `matches!(path, RenderPath::ForwardPlus)` unconditional arm — the prepass
/// is structural to the path, not consumer-gated), so capping the FIVE consumers + TAA here only
/// suppresses `thin_aux`/`mesh_geo_shade_split`/`sdf_geo_shade_split`/`ShadowSources::HWRT_VIS`,
/// exactly mirroring plain Forward's own scope cut.
///
/// A no-op (returns `consumers` unchanged, pushes nothing) for every OTHER `path` —
/// `Deferred`/`VisibilityBuffer` are untouched by this rung's scope cut.
///
/// Lifted (narrowed, then removed) once the Forward family's thin-aux producers + motion MRT
/// land (a later rung); until then this is the Forward family's OWN rung-staged gate,
/// independent of [`degrade_ladder`]'s path/legs-level rules (mirrors how [`resolve_rules`]
/// itself stays a pure, "fully landed" truth table that a direct caller — see this module's
/// tests — can exercise AHEAD of any rung gate; this fn is the rung-staged gate
/// `resolve_render_path` applies before handing `resolve_rules` its inputs).
fn cap_forward_v1_consumers(
    path: RenderPath,
    mut consumers: RenderPathConsumers,
    degrades: &mut RenderPathDegradeLog,
) -> RenderPathConsumers {
    if !matches!(path, RenderPath::Forward | RenderPath::ForwardPlus) {
        return consumers;
    }

    let pre_light_requested = consumers.ssao_on
        || consumers.ddgi_on
        || consumers.shadow_denoise_spatial_on
        || consumers.shadow_temporal_on
        || consumers.ssr_on
        || consumers.hwrt_denoise_or_vis_on;
    if pre_light_requested {
        degrades.push(RenderPathDegrade::ForwardPreLightConsumersNotYetImplemented);
        consumers.ssao_on = false;
        consumers.ddgi_on = false;
        consumers.shadow_denoise_spatial_on = false;
        consumers.shadow_temporal_on = false;
        consumers.ssr_on = false;
        consumers.hwrt_denoise_or_vis_on = false;
    }

    if consumers.taa_on {
        degrades.push(RenderPathDegrade::ForwardTaaNotYetImplemented);
        consumers.taa_on = false;
    }

    consumers
}

/// Rung R8 (progressively lifted by the R9 stages): the VB pre-light consumer cap. As of rung
/// R9b, `ssao_on` PASSES THROUGH on every mesh-carrying leg set — the split pair (`vb_geo`
/// thin-aux producer + `vb_shade_split` consumer) backs it structurally. Still forced OFF here,
/// BEFORE [`resolve_rules`] computes `mesh_geo_shade_split`/`thin_aux`:
/// * EVERY pre-light consumer under `VB && !mesh_leg` (VB×Sdf — no mesh raster to split;
///   the R-SDFSPLIT boundary, `docs/R9-VB-SPLIT-PLAN.md` §1.3)
/// * `ddgi_on` on every leg set EXCEPT `Both` (rung R9c: consumption lives in the mesh split
///   shade and the probes are SDF-marched, so both legs must be present)
/// * `ssr_on` — ALWAYS capped (no SSR exists engine-wide, no rung lifts this)
///
/// As of rung R9d, `shadow_denoise_spatial_on`/`shadow_temporal_on`/`hwrt_denoise_or_vis_on`
/// have their OWN narrower rule instead of an unconditional zero (see the fn body): the hwrt
/// shadow chain (TLAS pack/build + `shadow_vis` + à-trous + temporal, all `feature = "hwrt"`)
/// is now the SOLE producer for either denoise stage under VB, and it exists ONLY when the mesh
/// leg is present (it traces MESH casters only, mirroring Deferred's own mesh-only hwrt shadow
/// producers) — so `shadow_denoise_spatial_on`/`shadow_temporal_on` PASS THROUGH only when
/// `legs.has_mesh() && hwrt_denoise_or_vis_on` (the hwrt carrier itself must be present; a
/// software `ShadowDenoiseConfig` with no ray-query carrier has NO producer under the split arm
/// — `vb_shade_split` wires no soft-march/CSM/atlas fallback — and stays capped). Independently,
/// `hwrt_denoise_or_vis_on` itself PASSES THROUGH whenever `legs.has_mesh()` (it is its own
/// carrier, gating [`ShadowSources::HWRT_VIS`] regardless of whether a denoise stage rides it).
///
/// Mirrors [`cap_forward_v1_consumers`]'s scope-cut mechanism for the Forward family. A
/// separate fn (not a widened `cap_forward_v1_consumers`) so the pushed [`RenderPathDegrade`]
/// variants stay VB-labeled, not misleadingly "Forward"-labeled. (The former TAA half of this
/// cap was DELETED by the TAA-under-VB rungs — TAA now passes through on every VB leg set;
/// see [`ResolvedRenderPath::taa_supported`]'s matrix doc.)
///
/// A no-op (returns `consumers` unchanged, pushes nothing) for every OTHER `path`.
///
/// Lifted (narrowed, then removed) once `vb_geo`/`vb_shade` land (rung R9); until then this is
/// VB's OWN rung-staged gate, independent of [`degrade_ladder`]'s path/legs-level rules — the
/// SAME "this fn is the rung-staged gate `resolve_render_path` applies before handing
/// `resolve_rules` its inputs" discipline [`cap_forward_v1_consumers`]'s doc explains.
fn cap_vb_v1_consumers(
    path: RenderPath,
    legs: GeometryLegs,
    mut consumers: RenderPathConsumers,
    degrades: &mut RenderPathDegradeLog,
) -> RenderPathConsumers {
    if !matches!(path, RenderPath::VisibilityBuffer) {
        return consumers;
    }

    // Rung R9b: `ssao_on` PASSES THROUGH when the mesh leg is present — the split pair
    // (`vb_geo` thin-aux producer + `vb_shade_split` consumer) exists now, so the resolver's
    // `mesh_geo_shade_split`/`thin_aux` derivation is structurally backed. Under `VB && !mesh_leg`
    // (VB×Sdf) EVERY pre-light consumer stays zeroed — there is no mesh raster to split; that
    // residual is R-SDFSPLIT's boundary, not this rung's (docs/R9-VB-SPLIT-PLAN.md §1.3).
    let ssao_capped = consumers.ssao_on && !legs.has_mesh();
    // Rung R9c: `ddgi_on` passes through on VB×Both ONLY — consumption lives in the mesh split
    // shade (`vb_shade_split`'s probe sample) and the probes themselves are SDF-marched
    // (`gpu_scene` ANDs the per-frame arming with `sdf_leg`), so a mesh-less or SDF-less leg
    // set keeps it zeroed.
    let ddgi_capped = consumers.ddgi_on && !(legs.has_mesh() && legs.has_sdf());
    // Rung R9d: the hwrt shadow chain is the SOLE producer for either denoise stage under VB,
    // and it exists ONLY on a mesh-carrying leg set with the hwrt carrier itself armed — a
    // software-only `ShadowDenoiseConfig` (no ray-query) has no producer under the split arm
    // and stays capped (see this fn's doc for the full rationale).
    let spatial_capped =
        consumers.shadow_denoise_spatial_on && !(legs.has_mesh() && consumers.hwrt_denoise_or_vis_on);
    let temporal_capped =
        consumers.shadow_temporal_on && !(legs.has_mesh() && consumers.hwrt_denoise_or_vis_on);
    // `hwrt_denoise_or_vis_on` is its own carrier — it survives whenever the mesh leg is
    // present, independent of whether a denoise stage rides it.
    let hwrt_vis_capped = consumers.hwrt_denoise_or_vis_on && !legs.has_mesh();
    let pre_light_requested = ssao_capped
        || ddgi_capped
        || spatial_capped
        || temporal_capped
        || consumers.ssr_on
        || hwrt_vis_capped;
    if pre_light_requested {
        degrades.push(RenderPathDegrade::VbPreLightConsumersNotYetImplemented);
        if ssao_capped {
            consumers.ssao_on = false;
        }
        if ddgi_capped {
            consumers.ddgi_on = false;
        }
        if spatial_capped {
            consumers.shadow_denoise_spatial_on = false;
        }
        if temporal_capped {
            consumers.shadow_temporal_on = false;
        }
        if hwrt_vis_capped {
            consumers.hwrt_denoise_or_vis_on = false;
        }
        consumers.ssr_on = false;
    }

    // TAA-under-VB: the cap is REMOVED — TAA passes through on every VB leg set. VB×Mesh uses
    // the `viewt_from_depth_rz` gViewT producer; the SDF-carrying legs (`Both`/`Sdf`) dispatch
    // the `VIEWT`-variant `sdf_forward_march`, whose composite writes the gViewT lane itself
    // (SDF-owned `t`, mesh-owned `t_mesh`, background sentinel — see
    // [`ResolvedRenderPath::taa_supported`]'s matrix doc).

    consumers
}

// ---- resolve_render_path (the single boot-time entry point — Decision 1) --------------

/// Resolves the owner's [`RenderPathConfig`] + the boot-time [`RenderPathConsumers`] snapshot +
/// [`RenderPathDeviceCaps`] into a [`ResolvedRenderPath`] plus the [`RenderPathDegradeLog`] of
/// any plan-documented fallback applied along the way. Pure, total (never panics — degrade, not
/// panic, on every unsupported combo) and called EXACTLY ONCE, at `WindowHost::boot`
/// (`boyko_app::runner`, the `ssaa_armed` precedent) — see this module's doc for why a per-frame
/// re-resolve is forbidden by design (Decision 1).
///
/// Applies the rung-staged `degrade_ladder` against the REQUESTED `(path, legs)` first (using
/// the real `FORWARD_IMPLEMENTED`/`FORWARD_PLUS_IMPLEMENTED`/`VB_IMPLEMENTED`/
/// `SDF_FORWARD_IMPLEMENTED` consts), then `cap_forward_v1_consumers` (rung R4b: forces every
/// pre-light consumer + TAA off under the FINAL `Forward` path, its own rung-staged gate,
/// independent of the ladder), then computes every derived field via [`resolve_rules`] against
/// the FINAL `(path, legs)` and the (possibly capped) consumers — so e.g.
/// `depth_kind`/`thin_aux`/`shadow`/`needs_depth_prepass` always describe what will ACTUALLY be
/// recorded, never the owner's un-degraded/un-capped request. `FORWARD_IMPLEMENTED` is `true`
/// as of rung R4b-b, so a `Forward` request now reaches `cap_forward_v1_consumers` for real
/// (through THIS fn, not only through the module's own direct-pipeline tests) — its scope-cut
/// behavior (forcing every pre-light consumer + TAA off) is exercised end-to-end here.
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
    let (path, legs, mut degrades) = degrade_ladder(
        cfg.path,
        cfg.legs,
        caps,
        path_implemented,
        SDF_FORWARD_IMPLEMENTED,
        VB_SDF_IMPLEMENTED,
    );
    let consumers = cap_forward_v1_consumers(path, consumers, &mut degrades);
    let consumers = cap_vb_v1_consumers(path, legs, consumers, &mut degrades);
    (resolve_rules(path, legs, consumers, caps), degrades)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps_ok() -> RenderPathDeviceCaps {
        RenderPathDeviceCaps { storage_buffer_array_non_uniform_indexing: true, rg8_unorm_storage: true }
    }

    fn caps_missing() -> RenderPathDeviceCaps {
        RenderPathDeviceCaps { storage_buffer_array_non_uniform_indexing: false, rg8_unorm_storage: true }
    }

    // ---- rung R8 sync-pin: VB_ID_SENTINEL host<->shader ---------------------------------

    #[test]
    fn vb_id_sentinel_is_u32_max() {
        // `vb_pack.hlsli`'s `VB_ID_SENTINEL` MUST spell the identical `0xFFFFFFFF` literal —
        // the shared invariant is "no real per-frame instance index ever reaches u32::MAX".
        assert_eq!(VB_ID_SENTINEL, u32::MAX);
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

    // NOTE: the historical `every_non_deferred_path_degrades_today` test was retired at rung
    // R8 — all four `RenderPath` variants are implemented now (`FORWARD_IMPLEMENTED` @R4b-b,
    // `FORWARD_PLUS_IMPLEMENTED` @R5, `SDF_FORWARD_IMPLEMENTED` @R-SDFFWD, `VB_IMPLEMENTED`
    // @R8), so no path unconditionally degrades any more. Per-path behavior is pinned by the
    // `forward_*`/`forward_plus_*`/`vb_*` truth-table blocks below; the `PathNotYetImplemented`
    // ladder rule itself stays covered by `degrade_ladder`'s direct tests.

    // ---- rung R4b-b: `Forward` is real through the public `resolve_render_path` entry point ---

    #[test]
    fn forward_both_and_sdf_stay_as_requested_once_sdf_forward_lands() {
        // Rung R-SDFFWD: `SDF_FORWARD_IMPLEMENTED` is now `true` — a non-`Mesh` leg set no
        // longer collapses under `Forward`; `sdf_forward_marched` arms whenever the FINAL legs
        // carry the SDF leg (`sdf_leg && path != Deferred`).
        for (legs, sdf_leg) in [(GeometryLegs::Both, true), (GeometryLegs::Sdf, true), (GeometryLegs::Mesh, false)] {
            let cfg = RenderPathConfig { path: RenderPath::Forward, legs };
            let (resolved, degrades) = resolve_render_path(&cfg, RenderPathConsumers::default(), caps_ok());
            assert_eq!(resolved.path, RenderPath::Forward);
            assert_eq!(resolved.legs, legs, "{legs:?} must stay as requested — no legs-collapse");
            assert!(degrades.is_clean(), "{legs:?}: no degrade once SDF-forward-march has landed");
            assert_eq!(resolved.sdf_leg, sdf_leg);
            assert_eq!(resolved.sdf_forward_marched, sdf_leg, "{legs:?}: sdf_forward_marched == sdf_leg under Forward");
        }
    }

    // ---- rung R5: `ForwardPlus` is real through the public `resolve_render_path` entry point ---

    #[test]
    fn forward_plus_both_and_sdf_stay_as_requested_once_sdf_forward_lands() {
        // Rung R-SDFFWD: mirrors `forward_both_and_sdf_stay_as_requested_once_sdf_forward_lands`
        // for `ForwardPlus` — every leg set resolves clean, `sdf_forward_marched` tracks `sdf_leg`.
        for legs in [GeometryLegs::Both, GeometryLegs::Sdf] {
            let cfg = RenderPathConfig { path: RenderPath::ForwardPlus, legs };
            let (resolved, degrades) =
                resolve_render_path(&cfg, RenderPathConsumers::default(), caps_ok());
            assert_eq!(resolved.path, RenderPath::ForwardPlus);
            assert_eq!(resolved.legs, legs, "{legs:?} must stay as requested — no legs-collapse");
            assert!(degrades.is_clean(), "{legs:?}: no degrade once SDF-forward-march has landed");
            assert!(resolved.sdf_leg);
            assert!(resolved.sdf_forward_marched);
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

    // ---- rung R3b: every Deferred leg combination resolves clean (no degrade) ----------

    #[test]
    fn deferred_mesh_only_no_longer_degrades_at_r3b() {
        // Rung R3b closed the R3 audit's gViewT gap (the `viewt_from_depth` producer,
        // `boyko_rhi_vulkan::present::graph_bridge` + `boyko_rhi_vulkan::compute::
        // ViewtFromDepthPush`) -- `Deferred x Mesh` now resolves verbatim, like `Deferred x Sdf`
        // did at R3.
        let cfg = RenderPathConfig { path: RenderPath::Deferred, legs: GeometryLegs::Mesh };
        let (resolved, degrades) = resolve_render_path(&cfg, RenderPathConsumers::default(), caps_ok());
        assert_eq!(resolved.path, RenderPath::Deferred);
        assert_eq!(resolved.legs, GeometryLegs::Mesh, "Deferred x Mesh lands at R3b -- no degrade");
        assert!(degrades.is_clean());
        assert!(resolved.mesh_leg && !resolved.sdf_leg);
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

    // ---- VB-without-cap degrade (tested directly against the ladder, so the rule stays ----
    // ---- pinned independently of what `VB_IMPLEMENTED` happens to be) ---------------------

    #[test]
    fn vb_without_device_cap_degrades_to_deferred() {
        let (path, legs, degrades) = degrade_ladder(
            RenderPath::VisibilityBuffer,
            GeometryLegs::Mesh,
            caps_missing(),
            true, // hypothetically landed, to exercise the device-cap rule in isolation
            true,
            true,
        );
        assert_eq!(path, RenderPath::Deferred);
        assert_eq!(legs, GeometryLegs::Mesh);
        let reasons: Vec<_> = degrades.reasons().collect();
        assert_eq!(reasons, [RenderPathDegrade::VbDeviceCapMissing]);
    }

    #[test]
    fn vb_with_device_cap_stays_visibility_buffer_once_landed() {
        let (path, _legs, degrades) =
            degrade_ladder(RenderPath::VisibilityBuffer, GeometryLegs::Mesh, caps_ok(), true, true, true);
        assert_eq!(path, RenderPath::VisibilityBuffer);
        assert!(degrades.is_clean());
    }

    #[test]
    fn vb_now_resolves_for_real_since_vb_implemented_landed_at_r8() {
        // Rung R8: VB_IMPLEMENTED flipped true — a VisibilityBuffer x Mesh request with no
        // device-cap issue and no pre-light consumer now resolves CLEAN (no degrade), unlike
        // the pre-R8 unconditional PathNotYetImplemented collapse this test used to assert.
        let cfg = RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh };
        let (resolved, degrades) =
            resolve_render_path(&cfg, RenderPathConsumers::default(), caps_ok());
        assert_eq!(resolved.path, RenderPath::VisibilityBuffer);
        assert!(degrades.is_clean());
        assert!(resolved.mesh_leg && !resolved.sdf_leg);
        assert!(resolved.vb_geometry_table);
        assert_eq!(resolved.depth_kind, DepthKind::HardwareReverseZ);
    }

    #[test]
    fn vb_without_device_cap_degrades_to_deferred_through_the_public_entry_point() {
        // The device-cap degrade IS still reachable end-to-end (VB_IMPLEMENTED == true does not
        // bypass it) — mirrors `vb_without_device_cap_degrades_to_deferred`'s direct-ladder
        // check, but through the real `resolve_render_path` entry point.
        let cfg = RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Mesh };
        let (resolved, degrades) =
            resolve_render_path(&cfg, RenderPathConsumers::default(), caps_missing());
        assert_eq!(resolved.path, RenderPath::Deferred);
        let reasons: Vec<_> = degrades.reasons().collect();
        assert_eq!(reasons, [RenderPathDegrade::VbDeviceCapMissing]);
    }

    // ---- legs collapse pre-SDF-forward --------------------------------------------------

    #[test]
    fn non_mesh_legs_collapse_to_mesh_before_sdf_forward_lands() {
        for legs in [GeometryLegs::Both, GeometryLegs::Sdf] {
            let (path, resolved_legs, degrades) =
                degrade_ladder(RenderPath::Forward, legs, caps_ok(), true, false, false);
            assert_eq!(path, RenderPath::Forward);
            assert_eq!(resolved_legs, GeometryLegs::Mesh);
            let reasons: Vec<_> = degrades.reasons().collect();
            assert_eq!(reasons, [RenderPathDegrade::LegsCollapsedToMeshPreSdfForward]);
        }
    }

    #[test]
    fn mesh_only_legs_never_collapse() {
        let (path, legs, degrades) =
            degrade_ladder(RenderPath::Forward, GeometryLegs::Mesh, caps_ok(), true, false, false);
        assert_eq!(path, RenderPath::Forward);
        assert_eq!(legs, GeometryLegs::Mesh);
        assert!(degrades.is_clean());
    }

    #[test]
    fn legs_stay_as_requested_once_sdf_forward_lands() {
        for legs in [GeometryLegs::Both, GeometryLegs::Sdf, GeometryLegs::Mesh] {
            let (path, resolved_legs, degrades) =
                degrade_ladder(RenderPath::ForwardPlus, legs, caps_ok(), true, true, false);
            assert_eq!(path, RenderPath::ForwardPlus);
            assert_eq!(resolved_legs, legs);
            assert!(degrades.is_clean());
        }
    }

    // ---- rung R8: VB-scoped legs collapse pre-VB-SDF (`LegsCollapsedToMeshPreVbSdf`) ----

    #[test]
    fn vb_non_mesh_legs_collapse_to_mesh_before_vb_sdf_lands() {
        // Rung R8: SDF_FORWARD_IMPLEMENTED is true today, so the OLDER
        // LegsCollapsedToMeshPreSdfForward rule is unconditionally dead for VB (and every other
        // non-Deferred path) — this fresh, VB-scoped rule is what actually fires.
        for legs in [GeometryLegs::Both, GeometryLegs::Sdf] {
            let (path, resolved_legs, degrades) = degrade_ladder(
                RenderPath::VisibilityBuffer,
                legs,
                caps_ok(),
                true,
                SDF_FORWARD_IMPLEMENTED,
                false,
            );
            assert_eq!(path, RenderPath::VisibilityBuffer);
            assert_eq!(resolved_legs, GeometryLegs::Mesh);
            let reasons: Vec<_> = degrades.reasons().collect();
            assert_eq!(reasons, [RenderPathDegrade::LegsCollapsedToMeshPreVbSdf]);
        }
    }

    #[test]
    fn vb_legs_stay_as_requested_once_vb_sdf_lands() {
        for legs in [GeometryLegs::Both, GeometryLegs::Sdf, GeometryLegs::Mesh] {
            let (path, resolved_legs, degrades) = degrade_ladder(
                RenderPath::VisibilityBuffer,
                legs,
                caps_ok(),
                true,
                SDF_FORWARD_IMPLEMENTED,
                true,
            );
            assert_eq!(path, RenderPath::VisibilityBuffer);
            assert_eq!(resolved_legs, legs);
            assert!(degrades.is_clean());
        }
    }

    // ---- rung R3b: degrade_ladder direct — every Deferred leg stays as requested -------

    #[test]
    fn deferred_ladder_never_collapses_any_leg() {
        for legs in [GeometryLegs::Both, GeometryLegs::Mesh, GeometryLegs::Sdf] {
            let (path, resolved_legs, degrades) =
                degrade_ladder(RenderPath::Deferred, legs, caps_ok(), true, false, false);
            assert_eq!(path, RenderPath::Deferred);
            assert_eq!(resolved_legs, legs);
            assert!(degrades.is_clean());
        }
    }

    // ---- rung R4b-b: Forward v1 scope cut (`cap_forward_v1_consumers`) truth table -----
    //
    // `FORWARD_IMPLEMENTED` is `true` as of rung R4b-b (`declare_forward_graph` landed —
    // `boyko_rhi_vulkan::present::graph_bridge::declare_frame_graph`'s `Forward` arm is real), so
    // these tests now route through the PUBLIC `resolve_render_path` entry point directly — the
    // same "the resolve fn is the gate" discipline every other truth-table block in this module
    // uses (see `every_non_deferred_path_degrades_today` above for the sibling paths that still
    // route through the same fn while un-implemented).

    /// Resolves a `Forward` request through the real, public [`resolve_render_path`] entry point
    /// (kept as a thin named helper — not inlined — purely to keep this block's call sites
    /// concise; it is no longer a hypothetical pipeline, `FORWARD_IMPLEMENTED` being `true` means
    /// this is byte-for-byte what production `boyko_app::runner` calls at boot).
    fn resolve_forward_v1(
        legs: GeometryLegs,
        consumers: RenderPathConsumers,
    ) -> (ResolvedRenderPath, RenderPathDegradeLog) {
        let cfg = RenderPathConfig { path: RenderPath::Forward, legs };
        resolve_render_path(&cfg, consumers, caps_ok())
    }

    #[test]
    fn forward_mesh_clean_lands_on_forward_with_no_degrades() {
        let (resolved, degrades) =
            resolve_forward_v1(GeometryLegs::Mesh, RenderPathConsumers::default());
        assert_eq!(resolved.path, RenderPath::Forward);
        assert!(degrades.is_clean());
        assert!(resolved.mesh_leg && !resolved.sdf_leg);
        assert!(!resolved.needs_depth_prepass);
        assert!(!resolved.prepass_writes_motion);
        assert_eq!(resolved.thin_aux, ThinAuxMask::NONE);
        assert_eq!(resolved.depth_kind, DepthKind::HardwareReverseZ);
    }

    #[test]
    fn forward_pre_light_consumer_is_capped_off_with_a_warn() {
        for (consumers, label) in [
            (RenderPathConsumers { ssao_on: true, ..Default::default() }, "ssao"),
            (RenderPathConsumers { ddgi_on: true, ..Default::default() }, "ddgi"),
            (RenderPathConsumers { shadow_denoise_spatial_on: true, ..Default::default() }, "shadow_denoise_spatial"),
            (RenderPathConsumers { shadow_temporal_on: true, ..Default::default() }, "shadow_temporal"),
            (RenderPathConsumers { ssr_on: true, ..Default::default() }, "ssr"),
            (RenderPathConsumers { hwrt_denoise_or_vis_on: true, ..Default::default() }, "hwrt_denoise_or_vis"),
        ] {
            let (resolved, degrades) = resolve_forward_v1(GeometryLegs::Mesh, consumers);
            assert_eq!(resolved.path, RenderPath::Forward, "{label}: path itself does not degrade");
            assert!(!resolved.needs_depth_prepass, "{label}: must stay capped off under Forward v1");
            assert!(!resolved.mesh_geo_shade_split);
            assert!(!resolved.sdf_geo_shade_split);
            assert_eq!(resolved.thin_aux, ThinAuxMask::NONE, "{label}: no thin-aux under Forward v1");
            let reasons: Vec<_> = degrades.reasons().collect();
            assert_eq!(
                reasons,
                [RenderPathDegrade::ForwardPreLightConsumersNotYetImplemented],
                "{label}: exactly one capped-consumer warn"
            );
        }
    }

    #[test]
    fn forward_taa_is_capped_off_with_a_warn() {
        let consumers = RenderPathConsumers { taa_on: true, ..Default::default() };
        let (resolved, degrades) = resolve_forward_v1(GeometryLegs::Mesh, consumers);
        assert_eq!(resolved.path, RenderPath::Forward);
        assert_eq!(resolved.thin_aux, ThinAuxMask::NONE, "TAA capped off -> no MOTION channel armed");
        let reasons: Vec<_> = degrades.reasons().collect();
        assert_eq!(reasons, [RenderPathDegrade::ForwardTaaNotYetImplemented]);
    }

    #[test]
    fn forward_pre_light_and_taa_both_capped_stack_two_reasons() {
        let consumers = RenderPathConsumers { ssao_on: true, taa_on: true, ..Default::default() };
        let (resolved, degrades) = resolve_forward_v1(GeometryLegs::Mesh, consumers);
        assert_eq!(resolved.path, RenderPath::Forward);
        assert!(!resolved.needs_depth_prepass);
        let reasons: Vec<_> = degrades.reasons().collect();
        assert_eq!(
            reasons,
            [
                RenderPathDegrade::ForwardPreLightConsumersNotYetImplemented,
                RenderPathDegrade::ForwardTaaNotYetImplemented,
            ]
        );
    }

    #[test]
    fn forward_both_consumer_caps_stack_two_reasons() {
        // Rung R-SDFFWD: Forward x Both, with SSAO+TAA both requested — the legs no longer
        // collapse (SDF-forward-march has landed), so only the two consumer-level caps
        // (`cap_forward_v1_consumers`) fire in one call.
        let consumers = RenderPathConsumers { ssao_on: true, taa_on: true, ..Default::default() };
        let (resolved, degrades) = resolve_forward_v1(GeometryLegs::Both, consumers);
        assert_eq!(resolved.path, RenderPath::Forward);
        assert_eq!(resolved.legs, GeometryLegs::Both, "Both stays as requested — no legs-collapse");
        let reasons: Vec<_> = degrades.reasons().collect();
        assert_eq!(
            reasons,
            [
                RenderPathDegrade::ForwardPreLightConsumersNotYetImplemented,
                RenderPathDegrade::ForwardTaaNotYetImplemented,
            ]
        );
    }

    // ---- rung R5: ForwardPlus scope cut (`cap_forward_v1_consumers` widened) truth table ---
    //
    // `FORWARD_PLUS_IMPLEMENTED` is `true` as of rung R5 — these tests mirror the Forward v1
    // block above exactly, with ONE structural difference: `needs_depth_prepass` stays `true`
    // for `ForwardPlus` regardless of the cap (`resolve_rules`'s unconditional `ForwardPlus` arm,
    // Decision 4) — only the consumer-driven `thin_aux`/`mesh_geo_shade_split`/
    // `sdf_geo_shade_split`/`ShadowSources::HWRT_VIS` fields stay capped off.

    /// Resolves a `ForwardPlus` request through the real, public [`resolve_render_path`] entry
    /// point — the `resolve_forward_v1` sibling for `ForwardPlus`.
    fn resolve_forward_plus(
        legs: GeometryLegs,
        consumers: RenderPathConsumers,
    ) -> (ResolvedRenderPath, RenderPathDegradeLog) {
        let cfg = RenderPathConfig { path: RenderPath::ForwardPlus, legs };
        resolve_render_path(&cfg, consumers, caps_ok())
    }

    #[test]
    fn forward_plus_mesh_clean_lands_on_forward_plus_with_no_degrades() {
        let (resolved, degrades) =
            resolve_forward_plus(GeometryLegs::Mesh, RenderPathConsumers::default());
        assert_eq!(resolved.path, RenderPath::ForwardPlus);
        assert!(degrades.is_clean());
        assert!(resolved.mesh_leg && !resolved.sdf_leg);
        assert!(resolved.needs_depth_prepass, "ForwardPlus always runs the depth prepass");
        assert!(!resolved.prepass_writes_motion, "no shadow_temporal armed -> no motion in the prepass");
        assert_eq!(resolved.thin_aux, ThinAuxMask::NONE, "no consumer requested -> no thin-aux channel");
        assert_eq!(resolved.depth_kind, DepthKind::HardwareReverseZ);
    }

    #[test]
    fn forward_plus_pre_light_consumer_is_capped_off_with_a_warn() {
        for (consumers, label) in [
            (RenderPathConsumers { ssao_on: true, ..Default::default() }, "ssao"),
            (RenderPathConsumers { ddgi_on: true, ..Default::default() }, "ddgi"),
            (RenderPathConsumers { shadow_denoise_spatial_on: true, ..Default::default() }, "shadow_denoise_spatial"),
            (RenderPathConsumers { shadow_temporal_on: true, ..Default::default() }, "shadow_temporal"),
            (RenderPathConsumers { ssr_on: true, ..Default::default() }, "ssr"),
            (RenderPathConsumers { hwrt_denoise_or_vis_on: true, ..Default::default() }, "hwrt_denoise_or_vis"),
        ] {
            let (resolved, degrades) = resolve_forward_plus(GeometryLegs::Mesh, consumers);
            assert_eq!(resolved.path, RenderPath::ForwardPlus, "{label}: path itself does not degrade");
            assert!(
                resolved.needs_depth_prepass,
                "{label}: the prepass stays on -- ForwardPlus's own unconditional trigger"
            );
            assert!(!resolved.mesh_geo_shade_split, "{label}: split stays capped off under R5 scope");
            assert!(!resolved.sdf_geo_shade_split);
            assert_eq!(resolved.thin_aux, ThinAuxMask::NONE, "{label}: no thin-aux under R5 scope");
            let reasons: Vec<_> = degrades.reasons().collect();
            assert_eq!(
                reasons,
                [RenderPathDegrade::ForwardPreLightConsumersNotYetImplemented],
                "{label}: exactly one capped-consumer warn"
            );
        }
    }

    #[test]
    fn forward_plus_taa_is_capped_off_with_a_warn() {
        let consumers = RenderPathConsumers { taa_on: true, ..Default::default() };
        let (resolved, degrades) = resolve_forward_plus(GeometryLegs::Mesh, consumers);
        assert_eq!(resolved.path, RenderPath::ForwardPlus);
        assert_eq!(resolved.thin_aux, ThinAuxMask::NONE, "TAA capped off -> no MOTION channel armed");
        let reasons: Vec<_> = degrades.reasons().collect();
        assert_eq!(reasons, [RenderPathDegrade::ForwardTaaNotYetImplemented]);
    }

    #[test]
    fn forward_plus_both_consumer_caps_stack_two_reasons() {
        // Rung R-SDFFWD: mirrors `forward_both_consumer_caps_stack_two_reasons` for `ForwardPlus`
        // — legs stay `Both` (no collapse), only the two consumer-level caps fire.
        let consumers = RenderPathConsumers { ssao_on: true, taa_on: true, ..Default::default() };
        let (resolved, degrades) = resolve_forward_plus(GeometryLegs::Both, consumers);
        assert_eq!(resolved.path, RenderPath::ForwardPlus);
        assert_eq!(resolved.legs, GeometryLegs::Both, "Both stays as requested — no legs-collapse");
        assert!(resolved.needs_depth_prepass, "mesh_leg is true under Both — the prepass stays armed");
        let reasons: Vec<_> = degrades.reasons().collect();
        assert_eq!(
            reasons,
            [
                RenderPathDegrade::ForwardPreLightConsumersNotYetImplemented,
                RenderPathDegrade::ForwardTaaNotYetImplemented,
            ]
        );
    }

    #[test]
    fn forward_plus_sdf_only_never_needs_the_prepass() {
        // Rung R-SDFFWD: the mesh_leg gate on `needs_depth_prepass` — `ForwardPlus x Sdf` (no
        // mesh leg) has nothing for the depth prepass to cull, so it stays OFF despite
        // `ForwardPlus`'s otherwise-unconditional trigger.
        let resolved =
            resolve_rules(RenderPath::ForwardPlus, GeometryLegs::Sdf, RenderPathConsumers::default(), caps_ok());
        assert!(!resolved.mesh_leg && resolved.sdf_leg);
        assert!(!resolved.needs_depth_prepass, "no mesh leg -> the prepass has nothing to cull");
        assert!(!resolved.prepass_writes_motion);
    }

    #[test]
    fn cap_forward_v1_consumers_is_a_noop_for_every_other_path() {
        for path in [RenderPath::Deferred, RenderPath::VisibilityBuffer] {
            let consumers = RenderPathConsumers {
                ssao_on: true,
                taa_on: true,
                shadow_temporal_on: true,
                ..Default::default()
            };
            let mut degrades = RenderPathDegradeLog::default();
            let out = cap_forward_v1_consumers(path, consumers, &mut degrades);
            assert_eq!(out, consumers, "{path:?}: consumers must pass through unchanged");
            assert!(degrades.is_clean(), "{path:?}: no cap-related warn");
        }
    }

    // ---- rung R8: VisibilityBuffer v1 (fused `vb_resolve`) scope cut (`cap_vb_v1_consumers`) --
    // ---- truth table — mirrors the Forward v1 block above -----------------------------------

    /// Resolves a `VisibilityBuffer` request through the real, public [`resolve_render_path`]
    /// entry point — `VB_IMPLEMENTED` is `true` as of rung R8, so this is byte-for-byte what
    /// production `boyko_app::runner` calls at boot (mirrors `resolve_forward_v1`'s doc).
    fn resolve_vb_v1(
        legs: GeometryLegs,
        consumers: RenderPathConsumers,
    ) -> (ResolvedRenderPath, RenderPathDegradeLog) {
        let cfg = RenderPathConfig { path: RenderPath::VisibilityBuffer, legs };
        resolve_render_path(&cfg, consumers, caps_ok())
    }

    #[test]
    fn vb_mesh_clean_lands_on_visibility_buffer_with_no_degrades() {
        let (resolved, degrades) = resolve_vb_v1(GeometryLegs::Mesh, RenderPathConsumers::default());
        assert_eq!(resolved.path, RenderPath::VisibilityBuffer);
        assert!(degrades.is_clean());
        assert!(resolved.mesh_leg && !resolved.sdf_leg);
        assert!(!resolved.mesh_geo_shade_split);
        assert!(resolved.vb_geometry_table);
        assert_eq!(resolved.thin_aux, ThinAuxMask::NONE);
        assert_eq!(resolved.depth_kind, DepthKind::HardwareReverseZ);
    }

    #[test]
    fn vb_pre_light_consumer_is_capped_off_with_a_warn() {
        // Rung R9b: `ssao_on` is NO LONGER in this capped set on a mesh-carrying leg (see
        // `vb_mesh_ssao_passes_through_and_arms_the_split` below). Rung R9d: `shadow_denoise_spatial_on`/
        // `shadow_temporal_on` stay capped here ONLY because the hwrt carrier
        // (`hwrt_denoise_or_vis_on`) is ABSENT in each row — a software-only `ShadowDenoiseConfig`
        // has no producer under the VB split arm (see `vb_mesh_software_temporal_stays_capped` and
        // `cap_vb_v1_consumers`'s doc). `ddgi`/`ssr` stay capped unconditionally on this leg set
        // (ddgi needs `Both`; ssr has no producer anywhere).
        for (consumers, label) in [
            (RenderPathConsumers { ddgi_on: true, ..Default::default() }, "ddgi"),
            (RenderPathConsumers { shadow_denoise_spatial_on: true, ..Default::default() }, "shadow_denoise_spatial_no_carrier"),
            (RenderPathConsumers { shadow_temporal_on: true, ..Default::default() }, "shadow_temporal_no_carrier"),
            (RenderPathConsumers { ssr_on: true, ..Default::default() }, "ssr"),
        ] {
            let (resolved, degrades) = resolve_vb_v1(GeometryLegs::Mesh, consumers);
            assert_eq!(resolved.path, RenderPath::VisibilityBuffer, "{label}: path itself does not degrade");
            assert!(!resolved.mesh_geo_shade_split, "{label}: must stay capped off (fused only) under VB v1");
            assert_eq!(resolved.thin_aux, ThinAuxMask::NONE, "{label}: no thin-aux under VB v1");
            let reasons: Vec<_> = degrades.reasons().collect();
            assert_eq!(
                reasons,
                [RenderPathDegrade::VbPreLightConsumersNotYetImplemented],
                "{label}: exactly one capped-consumer warn"
            );
        }

        // Rung R9d row pair: `shadow_temporal_on` + the hwrt carrier (`hwrt_denoise_or_vis_on`)
        // together PASS THROUGH on the SAME `GeometryLegs::Mesh` leg set the rows above capped —
        // the ONLY difference is the carrier's presence. See `cap_vb_v1_consumers`'s doc and
        // `vb_mesh_temporal_with_hwrt_carrier_arms_split_normal_motion` for the full assertion.
        let hwrt_pair = RenderPathConsumers {
            shadow_temporal_on: true,
            hwrt_denoise_or_vis_on: true,
            ..Default::default()
        };
        let (resolved, degrades) = resolve_vb_v1(GeometryLegs::Mesh, hwrt_pair);
        assert!(degrades.is_clean(), "temporal+hwrt_vis with the carrier present resolves clean");
        assert!(resolved.mesh_geo_shade_split, "the pair arms the split");
    }

    #[test]
    fn vb_mesh_temporal_with_hwrt_carrier_arms_split_normal_motion() {
        // Rung R9d: `shadow_temporal_on` + `hwrt_denoise_or_vis_on` together resolve CLEAN under
        // VB×Mesh — the hwrt shadow chain (TLAS + `shadow_vis` + à-trous + temporal) is the
        // producer, `shadow_temporal_on` is a pre-light consumer (arms the split), and
        // `hwrt_denoise_or_vis_on` joins the NORMAL union on non-Deferred paths (the vis pass's
        // cone-trace origin reads `thin_normal`) while `shadow_temporal_on` itself arms MOTION.
        let consumers = RenderPathConsumers {
            shadow_temporal_on: true,
            hwrt_denoise_or_vis_on: true,
            ..Default::default()
        };
        let (resolved, degrades) = resolve_vb_v1(GeometryLegs::Mesh, consumers);
        assert_eq!(resolved.path, RenderPath::VisibilityBuffer);
        assert!(degrades.is_clean(), "temporal+hwrt_vis under VB×Mesh resolves clean at R9d");
        assert!(resolved.mesh_geo_shade_split, "shadow_temporal is a pre-light consumer -> split");
        assert_eq!(
            resolved.thin_aux,
            ThinAuxMask::NORMAL.insert(ThinAuxMask::MOTION),
            "the vis pass's normal read + the temporal reproject's motion read both arm"
        );
        assert!(resolved.shadow.contains(ShadowSources::HWRT_VIS), "the hwrt carrier arms HWRT_VIS");
    }

    #[test]
    fn vb_mesh_software_temporal_stays_capped() {
        // Rung R9d: WITHOUT the hwrt carrier, `shadow_temporal_on` alone under VB×Mesh has no
        // producer under the split arm (`vb_shade_split` wires no soft-march/CSM/atlas fallback)
        // and stays capped — one warn, no split.
        let consumers = RenderPathConsumers { shadow_temporal_on: true, ..Default::default() };
        let (resolved, degrades) = resolve_vb_v1(GeometryLegs::Mesh, consumers);
        assert_eq!(resolved.path, RenderPath::VisibilityBuffer);
        assert!(!resolved.mesh_geo_shade_split, "no hwrt carrier -> no producer -> stays capped");
        assert_eq!(resolved.thin_aux, ThinAuxMask::NONE);
        let reasons: Vec<_> = degrades.reasons().collect();
        assert_eq!(reasons, [RenderPathDegrade::VbPreLightConsumersNotYetImplemented], "one warn");
    }

    #[test]
    fn vb_mesh_ssao_passes_through_and_arms_the_split() {
        // Rung R9b: SSAO survives the cap on every mesh-carrying VB leg set — the resolve is
        // CLEAN, the split arms, and thin_aux carries NORMAL (the `vb_geo` thin_normal write's
        // structural backing). This is the public-entry sibling of the resolve_rules-level
        // `vb_split_requires_the_mesh_leg` row.
        for legs in [GeometryLegs::Mesh, GeometryLegs::Both] {
            let consumers = RenderPathConsumers { ssao_on: true, ..Default::default() };
            let (resolved, degrades) = resolve_vb_v1(legs, consumers);
            assert_eq!(resolved.path, RenderPath::VisibilityBuffer, "{legs:?}");
            assert!(degrades.is_clean(), "{legs:?}: VB SSAO resolves clean at R9b");
            assert!(resolved.mesh_geo_shade_split, "{legs:?}: SSAO is a pre-light consumer -> split");
            assert!(
                resolved.thin_aux.contains(ThinAuxMask::NORMAL),
                "{legs:?}: split implies the thin_normal lane"
            );
        }
    }

    #[test]
    fn vb_both_ddgi_passes_through_and_arms_the_split() {
        // Rung R9c: DDGI survives the cap ONLY on VB×Both (the split shade samples the probes
        // for mesh pixels; the probes are SDF-marched) — Mesh/Sdf leg sets keep it zeroed.
        let consumers = RenderPathConsumers { ddgi_on: true, ..Default::default() };
        let (resolved, degrades) = resolve_vb_v1(GeometryLegs::Both, consumers);
        assert_eq!(resolved.path, RenderPath::VisibilityBuffer);
        assert!(degrades.is_clean(), "VB×Both DDGI resolves clean at R9c");
        assert!(resolved.mesh_geo_shade_split, "DDGI is a pre-light consumer -> split");
        assert!(resolved.thin_aux.contains(ThinAuxMask::NORMAL));

        for legs in [GeometryLegs::Mesh, GeometryLegs::Sdf] {
            let (resolved, degrades) = resolve_vb_v1(legs, consumers);
            assert!(
                !resolved.mesh_geo_shade_split,
                "{legs:?}: DDGI alone must not arm the split off VB×Both"
            );
            let reasons: Vec<_> = degrades.reasons().collect();
            assert_eq!(
                reasons,
                [RenderPathDegrade::VbPreLightConsumersNotYetImplemented],
                "{legs:?}: DDGI stays capped with the standing warn"
            );
        }
    }

    #[test]
    fn vb_sdf_only_ssao_stays_capped() {
        // Rung R9b residual: under VB && !mesh_leg there is no raster to split — SSAO stays
        // zeroed with the standing degrade warn (the R-SDFSPLIT boundary).
        let consumers = RenderPathConsumers { ssao_on: true, ..Default::default() };
        let (resolved, degrades) = resolve_vb_v1(GeometryLegs::Sdf, consumers);
        assert_eq!(resolved.path, RenderPath::VisibilityBuffer);
        assert!(!resolved.mesh_geo_shade_split);
        assert_eq!(resolved.thin_aux, ThinAuxMask::NONE);
        let reasons: Vec<_> = degrades.reasons().collect();
        assert_eq!(reasons, [RenderPathDegrade::VbPreLightConsumersNotYetImplemented]);
    }

    #[test]
    fn vb_mesh_taa_passes_through_uncapped() {
        // TAA-under-VB: VB×Mesh TAA is implemented — the cap no longer fires, the resolve is
        // CLEAN, and the surviving `taa_on` arms NO MOTION channel (VB TAA reprojects
        // camera-differentially from the depth-derived gViewT lane; MOTION is Deferred-only).
        let consumers = RenderPathConsumers { taa_on: true, ..Default::default() };
        let (resolved, degrades) = resolve_vb_v1(GeometryLegs::Mesh, consumers);
        assert_eq!(resolved.path, RenderPath::VisibilityBuffer);
        assert!(degrades.is_clean(), "VB×Mesh TAA resolves clean (no cap)");
        assert_eq!(
            resolved.thin_aux,
            ThinAuxMask::NONE,
            "VB TAA must NOT arm a producer-less MOTION channel"
        );
        assert!(resolved.taa_supported(), "the single predicate agrees: VB×Mesh supports TAA");
    }

    #[test]
    fn vb_sdf_legs_taa_passes_through_uncapped() {
        // TAA-under-VB (the VIEWT rung): the SDF-carrying leg sets now resolve CLEAN with TAA
        // requested — the `VIEWT`-variant `sdf_forward_march` composite is the gViewT producer
        // (SDF-owned `t`, mesh-owned `t_mesh`, background sentinel), so the former
        // `VbTaaNotYetImplemented` cap is gone. MOTION stays un-armed (VB TAA reprojects
        // camera-differentially from the gViewT lane; MOTION is Deferred-only).
        for legs in [GeometryLegs::Both, GeometryLegs::Sdf] {
            let consumers = RenderPathConsumers { taa_on: true, ..Default::default() };
            let (resolved, degrades) = resolve_vb_v1(legs, consumers);
            assert_eq!(resolved.path, RenderPath::VisibilityBuffer);
            assert!(degrades.is_clean(), "{legs:?}: VB TAA resolves clean on every leg set");
            assert!(
                resolved.taa_supported(),
                "{legs:?}: the single predicate accepts TAA under every VB leg set"
            );
            assert_eq!(
                resolved.thin_aux,
                ThinAuxMask::NONE,
                "{legs:?}: VB TAA must NOT arm a producer-less MOTION channel"
            );
        }
    }

    #[test]
    fn vb_mesh_ssao_plus_taa_resolves_clean_with_the_split() {
        // Rung R9b: VB×Mesh with SSAO+TAA both requested resolves CLEAN — SSAO arms the split
        // (thin_normal), TAA rides the gViewT lane, and the DUAL viewt-producer config
        // (`vb_viewt` pre-tail + none — Mesh has no marcher) stays single-producer.
        let consumers = RenderPathConsumers { ssao_on: true, taa_on: true, ..Default::default() };
        let (resolved, degrades) = resolve_vb_v1(GeometryLegs::Mesh, consumers);
        assert_eq!(resolved.path, RenderPath::VisibilityBuffer);
        assert!(degrades.is_clean(), "SSAO+TAA under VB×Mesh is fully implemented at R9b");
        assert!(resolved.mesh_geo_shade_split);
        assert!(resolved.taa_supported());
        assert!(resolved.thin_aux.contains(ThinAuxMask::NORMAL));
        assert!(
            !resolved.thin_aux.contains(ThinAuxMask::MOTION),
            "VB TAA still arms no MOTION channel (camera-differential reprojection)"
        );
    }

    #[test]
    fn taa_supported_matrix() {
        // The single-predicate truth table (every TAA gate reads taa_supported()).
        let rows: [(RenderPath, GeometryLegs, bool); 7] = [
            (RenderPath::Deferred, GeometryLegs::Both, true),
            (RenderPath::Deferred, GeometryLegs::Mesh, true),
            (RenderPath::VisibilityBuffer, GeometryLegs::Mesh, true),
            (RenderPath::VisibilityBuffer, GeometryLegs::Both, true),
            (RenderPath::VisibilityBuffer, GeometryLegs::Sdf, true),
            (RenderPath::Forward, GeometryLegs::Mesh, false),
            (RenderPath::ForwardPlus, GeometryLegs::Both, false),
        ];
        for (path, legs, want) in rows {
            let resolved = resolve_rules(path, legs, RenderPathConsumers::default(), caps_ok());
            assert_eq!(
                resolved.taa_supported(),
                want,
                "taa_supported({path:?}, {legs:?}) must be {want}"
            );
        }
    }

    /// The post-process AA seam's truth table, and the reason it is a SEPARATE test from
    /// [`taa_supported_matrix`] even though the two predicates currently agree on every row.
    ///
    /// The defect this guards: `targets.rs` arms `aa_out` on
    /// `scene.aa || scene.smaa || scene.ssaa || scene.taa` with **no path term**, and the present
    /// blit repoints every slot at `aa_out` whenever it is `Some`. `passes/forward.rs` contains
    /// **zero** AA sites and `declare_forward_graph` declares no AA pass, so before this predicate
    /// existed an `AaMode::Fxaa`/`Smaa`/`Ssaa` request on Forward/ForwardPlus presented an image
    /// **nothing had written**. Only `AaMode::Taa` was degraded, and the comment at the degrade
    /// site said so outright — the hole was documented, not closed.
    ///
    /// Keeping the two tables apart is what lets a future Forward AA seam flip this predicate
    /// without silently re-arming TAA on a path that still has no temporal machinery.
    #[test]
    fn post_process_aa_supported_matrix() {
        let rows: [(RenderPath, GeometryLegs, bool); 7] = [
            (RenderPath::Deferred, GeometryLegs::Both, true),
            (RenderPath::Deferred, GeometryLegs::Mesh, true),
            (RenderPath::VisibilityBuffer, GeometryLegs::Mesh, true),
            (RenderPath::VisibilityBuffer, GeometryLegs::Both, true),
            (RenderPath::VisibilityBuffer, GeometryLegs::Sdf, true),
            (RenderPath::Forward, GeometryLegs::Mesh, false),
            (RenderPath::ForwardPlus, GeometryLegs::Both, false),
        ];
        for (path, legs, want) in rows {
            let resolved = resolve_rules(path, legs, RenderPathConsumers::default(), caps_ok());
            assert_eq!(
                resolved.post_process_aa_supported(),
                want,
                "post_process_aa_supported({path:?}, {legs:?}) must be {want} — a true here on a \
                 recorder with no AA block means `aa_out` arms with nothing to write it"
            );
        }
    }

    /// A path that cannot run a temporal resolve must not claim a post-process AA seam either.
    ///
    /// This is the implication that actually protects the invariant, and it is the one direction
    /// that can never be safely violated: TAA is recorded *inside* the same AA block as FXAA and
    /// SMAA, so `taa_supported() && !post_process_aa_supported()` would mean a temporal resolve
    /// scheduled into a recorder that has nowhere to put it. The converse is fine and is the
    /// expected shape of a future Forward AA seam.
    #[test]
    fn taa_support_implies_a_post_process_aa_seam() {
        for path in [
            RenderPath::Deferred,
            RenderPath::VisibilityBuffer,
            RenderPath::Forward,
            RenderPath::ForwardPlus,
        ] {
            for legs in [GeometryLegs::Mesh, GeometryLegs::Both, GeometryLegs::Sdf] {
                let resolved = resolve_rules(path, legs, RenderPathConsumers::default(), caps_ok());
                assert!(
                    !resolved.taa_supported() || resolved.post_process_aa_supported(),
                    "{path:?}/{legs:?} claims TAA support without a post-process AA seam — the \
                     temporal resolve is recorded inside that seam, so this cannot hold"
                );
            }
        }
    }

    #[test]
    fn vb_both_legs_survive_and_ssao_taa_arm_the_mesh_split_only() {
        // Rung R10 + R9b: VB x Both with SSAO+TAA both requested resolves CLEAN — the legs stay
        // `Both` (`sdf_forward_marched` arms), SSAO passes the cap on a mesh-carrying leg set
        // and arms the MESH split, TAA rides the gViewT lane (the dual-producer config:
        // `vb_viewt` pre-tail for SSAO + the VIEWT marcher as the LAST writer for TAA). The SDF
        // split does NOT arm — sdf_geo_shade_split is R-SDFSPLIT's rung, and SSAO under VB×Both
        // is mesh-pixels-only this rung (SDF pixels read the background sentinel).
        let consumers = RenderPathConsumers { ssao_on: true, taa_on: true, ..Default::default() };
        let (resolved, degrades) = resolve_vb_v1(GeometryLegs::Both, consumers);
        assert_eq!(resolved.path, RenderPath::VisibilityBuffer);
        assert_eq!(resolved.legs, GeometryLegs::Both, "Both survives post-R10");
        assert!(resolved.mesh_leg && resolved.sdf_leg);
        assert!(resolved.sdf_forward_marched, "the SDF leg forward-marches under VB post-R10");
        assert!(degrades.is_clean(), "SSAO+TAA under VB×Both resolves clean at R9b");
        assert!(resolved.mesh_geo_shade_split, "SSAO arms the mesh split");
        assert!(
            !resolved.sdf_geo_shade_split,
            "the SDF split stays down (SDF_SPLIT_IMPLEMENTED gate — R-SDFSPLIT not landed)"
        );
        assert!(resolved.taa_supported(), "TAA survives the resolve on VB x Both");
        assert!(resolved.thin_aux.contains(ThinAuxMask::NORMAL));
    }

    #[test]
    fn vb_sdf_only_resolves_clean_and_forward_marches_post_r10() {
        // Rung R10: VB x Sdf (mesh-less) resolves CLEAN — the legs stay `Sdf`, `sdf_forward_marched`
        // arms, and `vb_geometry_table` is OFF (no mesh leg -> no Decision-0 table slot -> the
        // `vb_raster`/`vb_resolve` pair is `mesh_leg`-gated OFF at record time).
        let (resolved, degrades) = resolve_vb_v1(GeometryLegs::Sdf, RenderPathConsumers::default());
        assert_eq!(resolved.path, RenderPath::VisibilityBuffer);
        assert_eq!(resolved.legs, GeometryLegs::Sdf);
        assert!(degrades.is_clean());
        assert!(!resolved.mesh_leg && resolved.sdf_leg);
        assert!(resolved.sdf_forward_marched);
        assert!(!resolved.vb_geometry_table, "no mesh leg -> no VB geometry table");
        assert_eq!(resolved.depth_kind, DepthKind::HardwareReverseZ);
    }

    #[test]
    fn cap_vb_v1_consumers_is_a_noop_for_every_other_path() {
        for path in [RenderPath::Deferred, RenderPath::Forward, RenderPath::ForwardPlus] {
            let consumers = RenderPathConsumers {
                ssao_on: true,
                taa_on: true,
                shadow_temporal_on: true,
                ..Default::default()
            };
            let mut degrades = RenderPathDegradeLog::default();
            let out = cap_vb_v1_consumers(path, GeometryLegs::Both, consumers, &mut degrades);
            assert_eq!(out, consumers, "{path:?}: consumers must pass through unchanged");
            assert!(degrades.is_clean(), "{path:?}: no cap-related warn");
        }
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
    fn motion_only_shadow_temporal_arms_vb_split_while_sdf_split_stays_gated() {
        let consumers = RenderPathConsumers { shadow_temporal_on: true, ..RenderPathConsumers::default() };

        let vb = resolve_rules(RenderPath::VisibilityBuffer, GeometryLegs::Mesh, consumers, caps_ok());
        assert!(vb.mesh_geo_shade_split, "VB must split under a MOTION-only pre-light consumer too");

        let sdf = resolve_rules(RenderPath::Forward, GeometryLegs::Sdf, consumers, caps_ok());
        // Rung R9b: the SDF split flag is honesty-gated on `SDF_SPLIT_IMPLEMENTED` (false —
        // R-SDFSPLIT has no producers yet), so the RULE fires but the flag stays down. Flip
        // these two asserts when the const flips.
        assert!(!sdf.sdf_geo_shade_split, "gated on SDF_SPLIT_IMPLEMENTED until R-SDFSPLIT");
        assert!(!sdf.sdf_surface_cache);
    }

    #[test]
    fn vb_split_requires_the_mesh_leg() {
        // Rung R9a: the VB split separates the MESH raster's geometry fetch from its shade —
        // under GeometryLegs::Sdf there is no vb_raster/vb_id to split, so a mesh-less
        // pre-light config must not arm it (the SDF leg's own split is sdf_geo_shade_split).
        let consumers = RenderPathConsumers { ssao_on: true, ..RenderPathConsumers::default() };
        let resolved = resolve_rules(RenderPath::VisibilityBuffer, GeometryLegs::Sdf, consumers, caps_ok());
        assert!(!resolved.mesh_geo_shade_split, "no mesh leg -> no mesh split");
        assert!(
            !resolved.sdf_geo_shade_split,
            "the SDF leg's split RULE fires but the flag is gated on SDF_SPLIT_IMPLEMENTED"
        );

        for legs in [GeometryLegs::Mesh, GeometryLegs::Both] {
            let resolved = resolve_rules(RenderPath::VisibilityBuffer, legs, consumers, caps_ok());
            assert!(resolved.mesh_geo_shade_split, "{legs:?}: mesh leg present -> split arms");
        }
    }

    #[test]
    fn hwrt_vis_arms_thin_normal_only_off_deferred() {
        // Rung R9a: the hardware shadow_vis gather reads a per-pixel normal. Under Deferred
        // that is the fat gNormal G-buffer lane (no thin-aux involvement — arming unchanged);
        // on a thin-aux path (Forward prepass / VB split) its normal source IS thin_normal,
        // so hwrt_denoise_or_vis_on joins the NORMAL union there.
        let consumers =
            RenderPathConsumers { hwrt_denoise_or_vis_on: true, ..RenderPathConsumers::default() };

        let deferred = resolve_rules(RenderPath::Deferred, GeometryLegs::Both, consumers, caps_ok());
        assert!(
            !deferred.thin_aux.contains(ThinAuxMask::NORMAL),
            "Deferred: the vis pass reads gNormal, not thin_normal — NORMAL stays un-armed"
        );

        for path in [RenderPath::Forward, RenderPath::ForwardPlus, RenderPath::VisibilityBuffer] {
            let resolved = resolve_rules(path, GeometryLegs::Mesh, consumers, caps_ok());
            assert!(
                resolved.thin_aux.contains(ThinAuxMask::NORMAL),
                "{path:?}: the vis pass is a NORMAL (thin_normal) consumer on thin-aux paths"
            );
        }
    }

    #[test]
    fn temporal_plus_hwrt_arms_normal_and_motion_under_vb() {
        // Rung R9a (the revised Rev-5 row): the hardware "Temporal-only" config
        // (shadow_temporal_on + its hwrt_denoise_or_vis_on carrier) arms NORMAL|MOTION on a
        // thin-aux path — NOT the plan's original "MOTION-only" label (plan-doc erratum: the
        // vis gather that temporal filters is itself a thin_normal consumer there). NOTE:
        // `shadow_temporal_on` WITHOUT `hwrt_denoise_or_vis_on` (a software-leg
        // ShadowDenoiseConfig) still arms MOTION alone at THIS layer (`resolve_rules` has no
        // knowledge of the carrier requirement) — rung R9d's `cap_vb_v1_consumers` is what keeps
        // that software-only config capped before it ever reaches this fn (see
        // `vb_mesh_software_temporal_stays_capped`).
        let consumers = RenderPathConsumers {
            shadow_temporal_on: true,
            hwrt_denoise_or_vis_on: true,
            ..RenderPathConsumers::default()
        };
        let resolved =
            resolve_rules(RenderPath::VisibilityBuffer, GeometryLegs::Mesh, consumers, caps_ok());
        assert!(resolved.mesh_geo_shade_split, "temporal is a pre-light consumer -> split");
        assert!(resolved.thin_aux.contains(ThinAuxMask::NORMAL), "vis gather -> thin_normal");
        assert!(resolved.thin_aux.contains(ThinAuxMask::MOTION), "temporal -> motion pre-tail");
    }

    // ---- the R9a boot-freeze clamp (RenderPathFrozenConsumers) ---------------------------

    #[test]
    fn effective_ssao_config_is_passthrough_when_inert() {
        // The inert default (Deferred, or a world that never boot-resolved): the live config
        // wins — runtime toggles stay free.
        let frozen = RenderPathFrozenConsumers::default();
        let live = crate::ssao_config::SsaoConfig {
            quality: crate::ssao_config::SsaoQuality::High,
            atrous_levels: 2,
        };
        let eff = effective_ssao_config(&live, &frozen);
        assert_eq!(eff.quality, live.quality);
        assert_eq!(eff.atrous_levels, live.atrous_levels);
    }

    #[test]
    fn effective_ssao_config_clamps_to_the_boot_snapshot_when_frozen() {
        // Frozen (non-Deferred boot): the BOOT snapshot wins regardless of the live value —
        // both directions (boot-on/runtime-off and boot-off/runtime-on).
        let boot = crate::ssao_config::SsaoConfig {
            quality: crate::ssao_config::SsaoQuality::High,
            atrous_levels: 3,
        };
        let frozen = RenderPathFrozenConsumers::new(boot, false, true);
        let runtime_off = crate::ssao_config::SsaoConfig {
            quality: crate::ssao_config::SsaoQuality::Off,
            atrous_levels: 3,
        };
        let eff = effective_ssao_config(&runtime_off, &frozen);
        assert_eq!(eff.quality, boot.quality, "boot-on / runtime-off: boot stays in effect");

        let frozen_off =
            RenderPathFrozenConsumers::new(crate::ssao_config::SsaoConfig::default(), false, true);
        let runtime_on = boot;
        let eff = effective_ssao_config(&runtime_on, &frozen_off);
        assert_eq!(
            eff.quality,
            crate::ssao_config::SsaoQuality::Off,
            "boot-off / runtime-on: the gather stays disarmed AND the header word stays 0 \
             (both readers share this one clamp)"
        );
    }

    #[test]
    fn effective_ssao_config_warns_exactly_once() {
        // The warn latch flips on the FIRST divergence and stays flipped (one warn across all
        // readers — the latch lives on the shared snapshot).
        let frozen =
            RenderPathFrozenConsumers::new(crate::ssao_config::SsaoConfig::default(), false, true);
        let live = crate::ssao_config::SsaoConfig {
            quality: crate::ssao_config::SsaoQuality::Low,
            atrous_levels: 0,
        };
        assert!(!frozen.warned_ssao.has_fired());
        let _ = effective_ssao_config(&live, &frozen);
        assert!(frozen.warned_ssao.has_fired(), "first divergence latches");
        let _ = effective_ssao_config(&live, &frozen);
        assert!(frozen.warned_ssao.has_fired(), "stays latched");
    }

    #[test]
    fn w2205_reports_each_frozen_consumer_once_and_the_latches_are_independent() {
        // Rung L8a. The claim under test is the one the hand-rolled latch could not make: the
        // SECOND reader of a diverged config emits nothing, and the DDGI latch does not spend the
        // SSAO one. The old code passed the sibling test above while performing an unconditional
        // `swap` on every diverged frame -- `has_fired()` was true either way, so a per-frame
        // store was invisible to it. Counting the RECORDS is what makes the difference visible.
        crate::log_probe::arm();

        let frozen =
            RenderPathFrozenConsumers::new(crate::ssao_config::SsaoConfig::default(), false, true);
        let live = crate::ssao_config::SsaoConfig {
            quality: crate::ssao_config::SsaoQuality::Low,
            atrous_levels: 0,
        };

        // Four diverged reads across the two consumers: two records, not four.
        boyko_log::probe::watch(b'W', W2205.number());
        let _ = effective_ssao_config(&live, &frozen);
        let _ = effective_ssao_config(&live, &frozen);
        let _ = effective_ddgi_enabled(true, &frozen);
        let _ = effective_ddgi_enabled(true, &frozen);
        assert_eq!(
            boyko_log::probe::watched(),
            2,
            "each frozen consumer reports once; a shared latch would give 1 and no latch 4"
        );

        // Both latches spent -- further divergence is silent.
        boyko_log::probe::watch(b'W', W2205.number());
        let _ = effective_ssao_config(&live, &frozen);
        let _ = effective_ddgi_enabled(true, &frozen);
        assert_eq!(boyko_log::probe::watched(), 0, "a spent latch emits nothing");
    }

    #[test]
    fn taa_only_does_not_arm_the_prepass_or_the_split() {
        // TAA is a POST-light motion consumer — it must NOT trigger needs_depth_prepass /
        // mesh_geo_shade_split / sdf_geo_shade_split on its own (only pre-light consumers do).
        // MOTION is Deferred-only now (TAA-under-VB reprojects from the depth-derived gViewT
        // lane; a non-Deferred MOTION channel would be producer-less).
        let consumers = RenderPathConsumers { taa_on: true, ..RenderPathConsumers::default() };
        let resolved = resolve_rules(RenderPath::Forward, GeometryLegs::Mesh, consumers, caps_ok());
        assert!(!resolved.needs_depth_prepass);
        assert!(!resolved.prepass_writes_motion);
        assert!(
            !resolved.thin_aux.contains(ThinAuxMask::MOTION),
            "TAA outside Deferred must NOT arm the producer-less MOTION channel"
        );
        let deferred =
            resolve_rules(RenderPath::Deferred, GeometryLegs::Mesh, consumers, caps_ok());
        assert!(
            deferred.thin_aux.contains(ThinAuxMask::MOTION),
            "under Deferred, TAA still arms the MOTION channel itself"
        );
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

    /// Every `RenderPathConsumers` bool, so the sweep below stays exhaustive by CONSTRUCTION:
    /// adding a field to that struct without adding it here leaves the sweep silently narrower
    /// than it claims. The setters are listed in declaration order.
    const CONSUMER_SETTERS: [fn(&mut RenderPathConsumers); 11] = [
        |c| c.ssao_on = true,
        |c| c.ddgi_on = true,
        |c| c.shadow_denoise_spatial_on = true,
        |c| c.shadow_temporal_on = true,
        |c| c.ssr_on = true,
        |c| c.taa_on = true,
        |c| c.csm_on = true,
        |c| c.punctual_shadows_on = true,
        |c| c.hwrt_denoise_or_vis_on = true,
        |c| c.sdf_shadows_wanted = true,
        |c| c.clusters_wanted = true,
    ];

    /// Builds the consumer snapshot whose set bits are `mask`'s (bit `i` ⇒ `CONSUMER_SETTERS[i]`).
    fn consumers_from_mask(mask: u32) -> RenderPathConsumers {
        let mut c = RenderPathConsumers::default();
        for (i, set) in CONSUMER_SETTERS.iter().enumerate() {
            if mask & (1 << i) != 0 {
                set(&mut c);
            }
        }
        c
    }

    /// docs/SHADER-VARIANT-MANIFEST.md's `vb_shade_split_*hwrt` reachability note: the two
    /// `-D HWRT=1` rows require `hwrt_denoise_or_vis_on`, which is exactly what
    /// `ShadowSources::SDF_SOFT_MARCH` requires FALSE — so no SDF-march-sourced shadow term can
    /// ever be armed while they are bound.
    ///
    /// The exclusion was previously pinned only by
    /// `sdf_soft_march_needs_the_sdf_leg_and_is_wanted_and_no_hwrt`'s SINGLE
    /// `Deferred × Sdf` point. MEASURED gap (this is why the sweep exists, not a hypothetical):
    /// gating the exclusion on `matches!(path, RenderPath::Deferred)` — which arms BOTH bits
    /// under `VisibilityBuffer`, the ONLY path that ever binds `vb_shade_split_hwrt` — passed
    /// all 384 `boyko-render` tests before this test was added.
    ///
    /// Swept over BOTH entry points: `resolve_rules` (the raw rule) and `resolve_render_path`
    /// (the real boot entry, whose degrade ladder + `cap_forward_v1_consumers` /
    /// `cap_vb_v1_consumers` rewrite `hwrt_denoise_or_vis_on` before the rule sees it — a cap
    /// that cleared the flag for `HWRT_VIS` but not for `SDF_SOFT_MARCH` would arm both).
    #[test]
    fn sdf_soft_march_and_hwrt_vis_stay_exclusive_over_the_whole_input_space() {
        let mut rule_rows = 0u32;
        let mut boot_rows = 0u32;
        for path in [
            RenderPath::Deferred,
            RenderPath::Forward,
            RenderPath::ForwardPlus,
            RenderPath::VisibilityBuffer,
        ] {
            for legs in [GeometryLegs::Both, GeometryLegs::Mesh, GeometryLegs::Sdf] {
                for caps in [caps_ok(), RenderPathDeviceCaps::default()] {
                    for mask in 0..(1u32 << CONSUMER_SETTERS.len()) {
                        let consumers = consumers_from_mask(mask);

                        let resolved = resolve_rules(path, legs, consumers, caps);
                        assert!(
                            resolved.shadow.hwrt_vis_excludes_sdf_soft_march(),
                            "resolve_rules({path:?}, {legs:?}, mask={mask:#013b}, caps={caps:?}) \
                             armed both SDF_SOFT_MARCH and HWRT_VIS: {:?}",
                            resolved.shadow
                        );
                        rule_rows += 1;

                        let cfg = RenderPathConfig { path, legs };
                        let (booted, _) = resolve_render_path(&cfg, consumers, caps);
                        assert!(
                            booted.shadow.hwrt_vis_excludes_sdf_soft_march(),
                            "resolve_render_path({path:?}, {legs:?}, mask={mask:#013b}, \
                             caps={caps:?}) armed both SDF_SOFT_MARCH and HWRT_VIS: {:?}",
                            booted.shadow
                        );
                        boot_rows += 1;
                    }
                }
            }
        }
        // A census, so a sweep silently reduced to nothing (an emptied `CONSUMER_SETTERS`, a
        // narrowed range) fails instead of passing vacuously: 4 paths x 3 leg sets x 2 cap sets
        // x 2^11 masks. The `11` is spelled as a LITERAL rather than `CONSUMER_SETTERS.len()` on
        // purpose — deriving it from the array would make the census self-fulfilling and blind
        // to exactly the shrinkage it exists to catch. Adding a 12th consumer therefore fails
        // this line by design: update it in the same edit that extends the array.
        assert_eq!(rule_rows, 4 * 3 * 2 * (1 << 11));
        assert_eq!(boot_rows, rule_rows);
    }

    /// The exclusion holds because ONE boolean drives both bits with opposite polarity — pinned
    /// positively so the sweep above cannot pass by arming NEITHER bit everywhere (a resolver
    /// that stopped setting `SDF_SOFT_MARCH` at all would satisfy an exclusion-only property).
    #[test]
    fn the_exclusion_is_a_split_not_a_mutual_suppression() {
        let base = RenderPathConsumers { sdf_shadows_wanted: true, ..Default::default() };
        for path in [RenderPath::Deferred, RenderPath::VisibilityBuffer] {
            let software = resolve_rules(path, GeometryLegs::Both, base, caps_ok());
            assert!(
                software.shadow.contains(ShadowSources::SDF_SOFT_MARCH)
                    && !software.shadow.contains(ShadowSources::HWRT_VIS),
                "{path:?}: no hwrt carrier => the SDF soft march is the shadow source"
            );

            let hwrt = RenderPathConsumers { hwrt_denoise_or_vis_on: true, ..base };
            let traced = resolve_rules(path, GeometryLegs::Both, hwrt, caps_ok());
            assert!(
                traced.shadow.contains(ShadowSources::HWRT_VIS)
                    && !traced.shadow.contains(ShadowSources::SDF_SOFT_MARCH),
                "{path:?}: the hwrt carrier DISPLACES the SDF soft march"
            );
        }
    }

    // ---- VB-SV0 arming truth table (plan §S4 gate (iii)) ------------------------------------

    /// The consumer set a VB-SV0 fixture boots under: `boyko_app::runner` hardwires
    /// `sdf_shadows_wanted: true` (no owner-facing toggle exists), and nothing else is armed.
    fn sv0_consumers() -> RenderPathConsumers {
        RenderPathConsumers { sdf_shadows_wanted: true, ..Default::default() }
    }

    /// **VB-SV0 plan §S4 gate (iii) — the ONLY mechanical instrument covering variant rows 9-10.**
    ///
    /// Rows 9 (`vb_shade_split_hwrt`) and 10 (`vb_shade_split_tex_hwrt`) are removed from S4's
    /// executing per-row coverage on a STRUCTURAL argument: they are selected exactly when
    /// `hwrt_denoise_or_vis_on` is `true`, and [`ShadowSources::SDF_SOFT_MARCH`] requires it to be
    /// `false`. An argument with no test is an assumption, so this is that argument's red.
    ///
    /// This is the RESOLVER half. The RECORD-SITE half is `note_vb_lit_producer`'s
    /// `debug_assert!` in `boyko_rhi_vulkan::present::passes::vb` (code-review P1-a): this test
    /// pins that no boot CAN arm SV0 under hwrt, that one pins that no FRAME binds an `_hwrt`
    /// producer while SV0 is armable. Neither implies the other — the resolver's answer only
    /// reaches the recorder because `ResolvedRenderPathGpu::vb_sdf_mesh_armable` carries it.
    ///
    /// # The red mutation this test exists to catch
    ///
    /// Delete the `&& !consumers.hwrt_denoise_or_vis_on` term from [`resolve_rules`]'
    /// `SDF_SOFT_MARCH` arming: the hwrt rows then report `vb_sdf_mesh_armable() == true` and the
    /// `!armable` assertions below fail. DEMONSTRATED at rung S4 — not asserted to be
    /// demonstrable.
    #[test]
    fn sv0_never_arms_under_hwrt() {
        // Rows 1-8: VB x Both, no hwrt carrier -> the SDF soft march IS the shadow source, and
        // SV0 is armable. This is the row the negative half must be contrasted against; without
        // it, a predicate hardwired to `false` would pass the whole test.
        let armable = resolve_rules(
            RenderPath::VisibilityBuffer,
            GeometryLegs::Both,
            sv0_consumers(),
            caps_ok(),
        );
        assert!(
            armable.shadow.contains(ShadowSources::SDF_SOFT_MARCH),
            "VB x Both with sdf_shadows_wanted and no hwrt must arm SDF_SOFT_MARCH"
        );
        assert!(armable.vb_sdf_mesh_armable(), "rows 1-8 must be SV0-armable");

        // Rows 9-10: the SAME configuration plus the hwrt carrier — which is exactly what selects
        // `vb_shade_split_hwrt` / `vb_shade_split_tex_hwrt`. `HWRT_VIS` armed is the proof this
        // row really is the hwrt family and not merely an un-armed variation of the row above.
        let hwrt_consumers = RenderPathConsumers {
            sdf_shadows_wanted: true,
            hwrt_denoise_or_vis_on: true,
            // `ssao_on` is what arms the SPLIT tail those two `.spv` live in — included so the
            // negative is quantified over the real row, not over a fused-tail lookalike.
            ssao_on: true,
            ..Default::default()
        };
        let hwrt =
            resolve_rules(RenderPath::VisibilityBuffer, GeometryLegs::Both, hwrt_consumers, caps_ok());
        assert!(hwrt.shadow.contains(ShadowSources::HWRT_VIS), "the hwrt carrier must arm HWRT_VIS");
        assert!(hwrt.mesh_geo_shade_split, "ssao_on under VB x Both must arm the split tail");
        // The LOAD-BEARING assertion comes first, so the named red mutation reddens THIS claim
        // and not a context assertion standing in front of it.
        assert!(
            !hwrt.vb_sdf_mesh_armable(),
            "rows 9-10 (`vb_shade_split_hwrt`/`_tex_hwrt`) must be STRUCTURALLY unarmable"
        );
        assert!(
            !hwrt.shadow.contains(ShadowSources::SDF_SOFT_MARCH),
            "the hwrt carrier displaces the SDF soft march"
        );

        // The same negative through the PRODUCTION entry point, because `resolve_rules` is the
        // rules layer and `cap_vb_v1_consumers` sits above it — a cap that ever cleared
        // `hwrt_denoise_or_vis_on` while leaving the hwrt pipelines bound would re-open the row.
        let (resolved_hwrt, _) = resolve_render_path(
            &RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Both },
            hwrt_consumers,
            caps_ok(),
        );
        assert!(
            resolved_hwrt.shadow.contains(ShadowSources::HWRT_VIS),
            "the hwrt carrier survives cap_vb_v1_consumers on a mesh-carrying leg set"
        );
        assert!(
            !resolved_hwrt.vb_sdf_mesh_armable(),
            "rows 9-10 must be unarmable through the production resolve too"
        );
    }

    /// The rest of the SV0 capability table: every configuration OUTSIDE the eight armable rows
    /// resolves unarmable, each for its own reason.
    ///
    /// Kept separate from [`sv0_never_arms_under_hwrt`] so a failure names which term broke: this
    /// one moves `path` and `legs`, that one moves only the hwrt carrier.
    #[test]
    fn sv0_armable_only_on_vb_with_both_legs() {
        // Non-VB paths: SV0 exists only in the three VB lit-producer tails.
        for path in [RenderPath::Deferred, RenderPath::Forward, RenderPath::ForwardPlus] {
            let resolved = resolve_rules(path, GeometryLegs::Both, sv0_consumers(), caps_ok());
            assert!(
                resolved.shadow.contains(ShadowSources::SDF_SOFT_MARCH),
                "{path:?} x Both still arms the SDF soft march (its own producer consumes it)"
            );
            assert!(!resolved.vb_sdf_mesh_armable(), "{path:?} has no VB lit-producer tail");
        }

        // VB x Mesh: no SDF leg, so there is no field to march.
        let mesh_only =
            resolve_rules(RenderPath::VisibilityBuffer, GeometryLegs::Mesh, sv0_consumers(), caps_ok());
        assert!(!mesh_only.shadow.contains(ShadowSources::SDF_SOFT_MARCH));
        assert!(!mesh_only.vb_sdf_mesh_armable(), "VB x Mesh has no SDF field");

        // VB x Sdf: the march exists but there are no mesh pixels to shade.
        let sdf_only =
            resolve_rules(RenderPath::VisibilityBuffer, GeometryLegs::Sdf, sv0_consumers(), caps_ok());
        assert!(sdf_only.shadow.contains(ShadowSources::SDF_SOFT_MARCH));
        assert!(!sdf_only.mesh_leg);
        assert!(!sdf_only.vb_sdf_mesh_armable(), "VB x Sdf has no mesh pixels");

        // A device without descriptor indexing degrades VB to Deferred, which takes SV0 with it.
        let (degraded, _) = resolve_render_path(
            &RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Both },
            sv0_consumers(),
            caps_missing(),
        );
        assert!(!matches!(degraded.path, RenderPath::VisibilityBuffer));
        assert!(!degraded.vb_sdf_mesh_armable(), "a VB->Deferred degrade must disarm SV0");

        // A device without RG8-UNORM STORAGE keeps the full VB boot but cannot host the SV0
        // prepass (the `sdf_term` ring is an RG8 storage target): the FULLY-armed row goes
        // unarmable on the cap alone, and NOTHING ELSE about the resolution moves.
        let (no_rg8, _) = resolve_render_path(
            &RenderPathConfig { path: RenderPath::VisibilityBuffer, legs: GeometryLegs::Both },
            sv0_consumers(),
            caps_ok().with_rg8_unorm_storage(false),
        );
        assert!(matches!(no_rg8.path, RenderPath::VisibilityBuffer), "the path itself survives");
        assert!(no_rg8.mesh_leg && no_rg8.shadow.contains(ShadowSources::SDF_SOFT_MARCH));
        assert!(!no_rg8.vb_sdf_mesh_armable(), "no RG8 storage => no SV0 prepass");
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

    /// `froxel_light_cull` is the SINGLE boot-frozen arm bit gating the entire froxel light-cull
    /// machinery — this pins the whole scoping claim, VB-ONLY: armed iff
    /// `consumers.clusters_wanted && path == VisibilityBuffer`, never for
    /// `ForwardPlus`/`Deferred`/`Forward` (which keep their own, unrelated `cluster_cull`
    /// scaffolding) even when `clusters_wanted` is `true`, and never when `clusters_wanted`
    /// itself is `false` (the production default — `LightingConfig::clusters_enabled` defaults
    /// to `false`, so `boyko_app::runner` threads `false` for every scene that never opts in).
    #[test]
    fn froxel_light_cull_is_vb_only() {
        let wanted = RenderPathConsumers { clusters_wanted: true, ..Default::default() };

        let vb = resolve_rules(RenderPath::VisibilityBuffer, GeometryLegs::Mesh, wanted, caps_ok());
        assert!(vb.froxel_light_cull, "VisibilityBuffer + clusters_wanted -> armed");

        for path in [RenderPath::ForwardPlus, RenderPath::Deferred, RenderPath::Forward] {
            let resolved = resolve_rules(path, GeometryLegs::Mesh, wanted, caps_ok());
            assert!(
                !resolved.froxel_light_cull,
                "{path:?}: froxel_light_cull is VB-ONLY, even with clusters_wanted"
            );
        }

        // The default (`clusters_wanted == false`) never arms, even under VisibilityBuffer.
        let default_vb = resolve_rules(
            RenderPath::VisibilityBuffer,
            GeometryLegs::Mesh,
            RenderPathConsumers::default(),
            caps_ok(),
        );
        assert!(!default_vb.froxel_light_cull, "clusters_wanted defaults to false -> never armed");
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
