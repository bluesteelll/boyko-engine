//! SDFDDGI I0 — the DDGI irradiance-probe-grid ECS policy (CPU, unit-testable). This
//! is the contained data/policy layer (the 0%-gate skeleton); the probe-update pass +
//! atlas allocation + the real resolve sample are later rungs (I1/I2/I3).
//!
//! Principle 0: ECS-native — [`DdgiConfig`] is the owner-set `#[derive(Resource)]`
//! singleton (the cold config, NOT a side `std::Vec`/`HashMap`) and [`ResolvedDdgi`] is
//! its derived companion Resource written by the cold [`resolve_ddgi_grid`] system.
//! This mirrors the CSM substrate EXACTLY: [`CsmConfig`](crate::csm_config::CsmConfig)
//! (the owner-set config) + [`ResolvedCsm`](crate::csm_config::ResolvedCsm) (the derived
//! carrier) + [`resolve_csm_cascades`](crate::csm_config::resolve_csm_cascades) (the cold
//! single-owner policy). The grid params are inline scalars/arrays, NOT a `Vec`.
//!
//! # The world-fixed bounded volume (Decision D1)
//!
//! The probe grid is a single WORLD-FIXED AABB (`origin + spacing + dims`), NOT
//! camera-centered cascades. Camera-independent ⇒ the grid UBO needs NO per-FIF ring and
//! temporal feedback needs NO reprojection (probe `i` is the same world point every
//! frame). So [`resolve_ddgi_grid`] writes a SINGLE Resource with no refit, unlike the
//! camera-dependent CSM/atlas resolves.
//!
//! # Capability is structural (no redundant `enabled: bool`)
//!
//! Whether the GI resolve runs is keyed off [`DdgiConfig::ddgi_indirect`], the 0%-gate
//! anchor (default `false`). [`DdgiConfig::enabled`] is a derived predicate, not stored
//! state — it ANDs in [`DdgiConfig::grid_is_sampleable`] ("dims nonzero" plus the spacing
//! validity the whole GI path's `mode_word == 1 ⇒ inv_spacing > 0` invariant rests on).
//!
//! # The 0%-gate
//!
//! [`DdgiConfig::default`] is DISABLED (`ddgi_indirect == false`). [`resolve_ddgi_grid`]
//! of the default config is the all-zero [`ResolvedDdgi`] (`ddgi_mode_word == 0`), and
//! [`ResolvedDdgi::default`] is byte-identical to it — so a world that never inserts a
//! non-default [`DdgiConfig`] carries the disabled selection and no render path samples
//! probe irradiance.

use boyko_macros::{Resource, SystemSet};

use boyko_ecs::ecs::core::system::{Res, ResMut};

use crate::light::{DDGI_MODE_BIT, LightTableDirty, LightingConfig};

// ---- constants -----------------------------------------------------------------------

/// The default probe-grid X dimension (probes along world X) — the owner-locked value
/// (`docs/RENDER-SDFDDGI-PLAN.md`, 2026-07-04: `16×8×16 = 2048` probes).
const DEFAULT_DIM_X: u32 = 16;
/// The default probe-grid Y dimension (probes along world Y) — owner-locked (8).
const DEFAULT_DIM_Y: u32 = 8;
/// The default probe-grid Z dimension (probes along world Z) — owner-locked (16).
const DEFAULT_DIM_Z: u32 = 16;
/// The default probe spacing in world units — owner-locked (`2.0` → a `32×16×32` unit
/// box for the default dims).
const DEFAULT_SPACING: f32 = 2.0;
/// The default grid origin (the minimum world corner of probe `(0,0,0)`). Placed so the
/// `32×16×32` default box centers a scene near the world origin with a low floor: X/Z
/// centered on 0, Y starting near the floor. The owner grows / re-places the volume by
/// config (Decision D1: "Grows by config, not code").
const DEFAULT_ORIGIN: [f32; 3] = [-16.0, -2.0, -16.0];

// ---- DdgiConfig (the owner-set Resource — mirrors CsmConfig) --------------------------

/// The global DDGI config (SDFDDGI I0) — a `World`-singleton Resource the owner sets, the
/// GI analogue of [`CsmConfig`](crate::csm_config::CsmConfig). Enablement is structural
/// ([`Self::ddgi_indirect`]), so there is no separate flag beyond the 0%-gate anchor.
///
/// `#[derive(Resource)]` via [`boyko_macros::Resource`] (the same derive path
/// `CsmConfig` / `LightingConfig` use). `Copy` so the cold policy reads it by value.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct DdgiConfig {
    /// The GI-indirect gate (the 0%-gate anchor). DEFAULT `false` ⇒ disabled: no probe
    /// update, no resolve sample, the 3 DDGI resolve bindings bound-but-unread. Flipping
    /// it to `true` is the ONLY change that alters a rendered pixel (later rungs wire the
    /// sample); at I0 the gated resolve block is still empty, so even `true` is a no-op.
    pub ddgi_indirect: bool,
    /// The minimum world corner of probe `(0,0,0)` — the grid AABB origin (Decision D1,
    /// world-fixed). The AABB spans `origin .. origin + spacing * (dims - 1)`.
    pub origin: [f32; 3],
    /// The world-space distance between adjacent probes (uniform on all axes). MUST be
    /// normal and positive — the resolve's world→probe index divides by this (carried as
    /// `inv_spacing` in [`ResolvedDdgi`] so the per-pixel path is div-free). Anything else
    /// (zero, negative, NaN, ±inf, subnormal) DISABLES the config via
    /// [`Self::grid_is_sampleable`] rather than reaching the GPU.
    pub spacing: f32,
    /// The probe count per axis (`[x, y, z]`). Owner-locked default `[16, 8, 16]` = 2048
    /// probes. A zero dimension is a degenerate (empty) grid and DISABLES the config —
    /// [`Self::grid_is_sampleable`] folds "dims nonzero" into [`Self::enabled`].
    pub dims: [u32; 3],
}

impl Default for DdgiConfig {
    /// The DISABLED default (`ddgi_indirect == false` — the 0%-gate): a default world
    /// resolves the all-zero [`ResolvedDdgi`] and touches no GI render path. The grid
    /// params carry the owner-locked values so that flipping `ddgi_indirect` to `true`
    /// yields a usable grid without further tuning.
    #[inline]
    fn default() -> Self {
        Self {
            ddgi_indirect: false,
            origin: DEFAULT_ORIGIN,
            spacing: DEFAULT_SPACING,
            dims: [DEFAULT_DIM_X, DEFAULT_DIM_Y, DEFAULT_DIM_Z],
        }
    }
}

impl DdgiConfig {
    /// Whether the GI resolve runs — the structural predicate: the [`Self::ddgi_indirect`]
    /// 0%-gate anchor AND [`Self::grid_is_sampleable`] (the promised "dims nonzero" fold, plus
    /// the spacing validity the lane's invariant rests on). False ⇒ the 0%-gate (no probe
    /// update, the resolve's GI term off). Mirrors
    /// [`CsmConfig::enabled`](crate::csm_config::CsmConfig::enabled).
    ///
    /// THIS is where the degenerate clamp lives (W1), and it is the only place: every
    /// downstream predicate — [`resolve_ddgi`], the caps clamp
    /// ([`resolve_ddgi_grid_clamped`](crate::ddgi_update::resolve_ddgi_grid_clamped)), the R9c
    /// freeze fold ([`resolve_ddgi_grid_frozen`](crate::ddgi_update::resolve_ddgi_grid_frozen))
    /// and the host's boot freeze snapshot — funnels through it, so a config the resolve
    /// cannot sample is DISABLED everywhere rather than at each reader's discretion.
    #[inline]
    pub fn enabled(&self) -> bool {
        self.ddgi_indirect && self.grid_is_sampleable()
    }

    /// Whether the grid params yield a sampleable volume — i.e. whether an ENABLED carrier
    /// derived from this config would carry a FINITE, POSITIVE `inv_spacing` over a non-empty
    /// probe lattice.
    ///
    /// # Why this is a correctness clamp and not a nicety (W1)
    ///
    /// The lane's load-bearing invariant is `header bit == 1 ⇒ ddgi_mode_word == 1 ⇒
    /// inv_spacing > 0`: the resolve shader recomputes `spacing = 1.0 / inv_spacing`
    /// (`ddgi_resolve.hlsli`) and then `origin + float3(c) * spacing`. An `inv_spacing` of `0`
    /// makes that `+inf` and the probe position NaN at `c == 0`; a NaN inverts under fast-math
    /// `NMin`/`NMax`, so `clamp(NaN, 0, 1)` is `0` — a BLACK pixel on every `is_sdf_lit`
    /// receiver. Zeroing the reciprocal at the pack site would produce exactly that carrier, so
    /// the config is DISABLED instead.
    ///
    /// `is_normal()` is the exact predicate wanted, not a stylistic choice: it rejects NaN,
    /// ±inf, ±0 AND subnormals, and a positive subnormal is the non-obvious case — `1.0 /
    /// 1.0e-40` overflows to `+inf`, and `(p - origin) * inf` is NaN at `p == origin`. Over the
    /// remaining (normal, positive) domain the reciprocal is finite and positive for every
    /// input: `1.0 / f32::MIN_POSITIVE` ≈ `8.5e37` and `1.0 / f32::MAX` ≈ `2.9e-39` (subnormal
    /// but nonzero), so both ends stay in range.
    ///
    /// A zero dimension is an empty grid: there is no probe to blend, and the shader's
    /// `max(dims, 1u)` would silently sample probe 0 as if the volume existed.
    #[inline]
    pub fn grid_is_sampleable(&self) -> bool {
        self.spacing.is_normal()
            && self.spacing > 0.0
            && self.dims[0] != 0
            && self.dims[1] != 0
            && self.dims[2] != 0
    }
}

// ---- ResolvedDdgi (the derived carrier — mirrors ResolvedCsm) -------------------------

/// The derived DDGI grid selection the resolve reads — the GI analogue of
/// [`ResolvedCsm`](crate::csm_config::ResolvedCsm). [`resolve_ddgi_grid`] is its SINGLE
/// writer (the one-producer-per-field discipline), recomputing it from [`DdgiConfig`]
/// each frame. `#[repr(C)]` for a stable GPU-ready layout — the byte-mirror of the
/// resolve shader's binding-18 `ResolvedDdgi` cbuffer.
///
/// The grid is WORLD-FIXED (Decision D1), so this carrier is CAMERA-INDEPENDENT: unlike
/// [`ResolvedCsm`](crate::csm_config::ResolvedCsm) /
/// [`ResolvedShadowAtlas`](crate::shadow_atlas::ResolvedShadowAtlas) it is uploaded into ONE
/// binding-18 buffer, not a per-FIF ring — and the reason is the DESCRIPTOR contract, not the
/// config's staticness: the resolve descriptor sets are built once at G-buffer creation and
/// capture the boot buffer, so a host ring would never be observed by the GPU. A single
/// buffer read by an in-flight frame is a Write-After-Read hazard on every transition, so the
/// host write ([`upload_ddgi_grid`](crate::upload_ddgi_grid)) is MONOTONE and value-gated:
/// written only when `ddgi_mode_word != 0` and the carrier changed, and the DISABLED (zero)
/// image is never written after boot — a sibling in-flight frame can then observe only finite
/// grids, never the zero one (see the upload's doc for the three-case argument).
///
/// DISABLED == [`Default`] == all-zero: the resolve gates on the LightBuf word-7 bit-4 header
/// gate, which [`sync_ddgi_light_gate`] sets from THIS carrier's `ddgi_mode_word`, so all-zero
/// is "off" and the bit can be 1 only over a non-zero grid.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct ResolvedDdgi {
    /// The grid origin (probe `(0,0,0)`'s minimum world corner), `.w` padding to a
    /// 16-byte `vec4` lane. Mirrors [`DdgiConfig::origin`].
    pub origin: [f32; 4],
    /// `inv_spacing` = `1.0 / spacing` (the resolve multiplies to get the fractional
    /// probe coordinate — a div-free per-pixel path), then the three `u32` grid dims
    /// (`dims.x`, `dims.y`, `dims.z`) bit-cast into the `.y/.z/.w` `f32` lanes.
    pub inv_spacing_dims: [f32; 4],
    /// The GI-enable mode word: `0` ⇒ off (the resolve's GI term off — the 0%-gate), `1`
    /// ⇒ on. Derived from the SAME [`DdgiConfig::enabled`] predicate as the LightBuf gate,
    /// so the two never disagree.
    pub ddgi_mode_word: u32,
    /// Padding to a 16-byte stride after the trailing `u32` word (three reserved `u32`s,
    /// zero at I0 — later rungs may carry atlas tile metrics here).
    pub _pad: [u32; 3],
}

// Layout pin: 16 (origin vec4) + 16 (inv_spacing_dims vec4) + 4 (mode) + 12 (pad) = 48 B.
// A change is a deliberate decision (the GPU cbuffer reads this stride at binding 18).
const _: () = assert!(size_of::<ResolvedDdgi>() == 48);
const _: () = assert!(core::mem::offset_of!(ResolvedDdgi, origin) == 0);
const _: () = assert!(core::mem::offset_of!(ResolvedDdgi, inv_spacing_dims) == 16);
const _: () = assert!(core::mem::offset_of!(ResolvedDdgi, ddgi_mode_word) == 32);
const _: () = assert!(core::mem::offset_of!(ResolvedDdgi, _pad) == 36);

/// The byte size of the host-coherent DDGI grid UBO — `size_of::<ResolvedDdgi>()` (48 B).
/// The resolve binds a UBO of exactly this shape at binding 18; hosts size their DDGI UBO
/// from THIS constant (single source — no hand-copied `48`).
pub const RESOLVED_DDGI_BYTES: usize = size_of::<ResolvedDdgi>();

impl ResolvedDdgi {
    /// The disabled selection — all-zero (origin zero, `inv_spacing_dims` zero,
    /// `ddgi_mode_word == 0`). The resolve of a disabled [`DdgiConfig`] and the value
    /// [`ResolvedDdgi::default`] returns. All-zero is load-bearing: the 0%-gate byte-image
    /// argument rests on DISABLED == Default == every byte zero.
    pub const DISABLED: Self = Self {
        origin: [0.0; 4],
        inv_spacing_dims: [0.0; 4],
        ddgi_mode_word: 0,
        _pad: [0; 3],
    };

    /// This carrier as its 48 raw bytes — EXACTLY what [`upload_ddgi_grid`](crate::upload_ddgi_grid)
    /// memcpys into the resolve's binding-18 UBO (the mirror of
    /// [`DdgiUpdateUbo::as_bytes`](crate::ddgi_update::DdgiUpdateUbo::as_bytes)). Twelve
    /// little-endian words: `origin` ×4, `inv_spacing_dims` ×4, `ddgi_mode_word`, `_pad` ×3 —
    /// field-for-field the shader's `gDdgiOrigin`@0 / `gDdgiInvSpacDims`@16 / `gDdgiMode`@32 /
    /// `_gDdgiPad`@36 block (member 3 `Offset 36`, name `_gDdgiPad`, in every committed b18
    /// RESOLVE `.spv` — `deferred_pbr*.comp.spv` and `vb_shade_split*.comp.spv`).
    ///
    /// The scope of that pin is the b18 resolve consumers ONLY. The standalone probe-GI
    /// harness `ddgi_probe_gi_resolve.comp.hlsl` mirrors the same 48-byte block at its own
    /// `b0`, but splits the trailing pad: member 3 there is `uint gSampleCount` @36 (the
    /// invocation bound, which cannot ride push-constants against that shared layout) with
    /// `uint2 _gDdgiPad` @40. Offsets 0/16/32 still match byte-for-byte — only the meaning of
    /// the first pad word differs, and this carrier writes it as zero.
    #[inline]
    pub fn as_bytes(&self) -> [u8; RESOLVED_DDGI_BYTES] {
        // SAFETY: `ResolvedDdgi` is `#[repr(C)]` with the pinned 48-byte layout (size and every
        // field offset const-asserted above — 16 + 16 + 4 + 12, no padding holes), and every
        // field is a POD `f32`/`u32` lane, so all 48 bytes are initialized and every bit
        // pattern is a valid `u8`. The transmute reads only those bytes.
        unsafe { core::mem::transmute::<Self, [u8; RESOLVED_DDGI_BYTES]>(*self) }
    }
}

impl Default for ResolvedDdgi {
    /// The resolve of the default (disabled) [`DdgiConfig`] — the 0%-gate, so a never-run
    /// policy already carries the no-GI selection.
    #[inline]
    fn default() -> Self {
        Self::DISABLED
    }
}

// ---- the resolve decision (pure — the unit-testable fit) ------------------------------

/// Derives the [`ResolvedDdgi`] grid carrier from `cfg` — the PURE, unit-testable DDGI
/// resolve (the analogue of [`resolve_csm`](crate::csm_config::resolve_csm), the core the
/// cold system wraps). CAMERA-INDEPENDENT (Decision D1: world-fixed volume), so it is a
/// pure function of `cfg` alone (no view, no refit).
///
/// Disabled (`!cfg.enabled()`) ⇒ [`ResolvedDdgi::DISABLED`] (all-zero, `ddgi_mode_word ==
/// 0` — the 0%-gate). Else it packs `origin`, `inv_spacing = 1/spacing`, the three `u32`
/// dims (bit-cast into the `f32` lanes), and `ddgi_mode_word == 1`.
///
/// A degenerate grid (non-positive / non-finite / subnormal spacing, or a zero dimension) is
/// DISABLED, not packed with a zeroed reciprocal — [`DdgiConfig::grid_is_sampleable`] carries
/// the reason. That is what makes `ddgi_mode_word == 1 ⇒ inv_spacing > 0` true BY
/// CONSTRUCTION for every carrier this function can return (W1).
#[inline]
pub fn resolve_ddgi(cfg: &DdgiConfig) -> ResolvedDdgi {
    if !cfg.enabled() {
        return ResolvedDdgi::DISABLED;
    }

    // `enabled()` folds `grid_is_sampleable()`, so `spacing` is normal and positive here and
    // the reciprocal is finite and positive (see that predicate's doc for both range ends).
    let inv_spacing = 1.0 / cfg.spacing;
    debug_assert!(
        inv_spacing.is_finite() && inv_spacing > 0.0,
        "invariant: an enabled config carries a sampleable spacing (grid_is_sampleable)"
    );

    ResolvedDdgi {
        origin: [cfg.origin[0], cfg.origin[1], cfg.origin[2], 0.0],
        inv_spacing_dims: [
            inv_spacing,
            f32::from_bits(cfg.dims[0]),
            f32::from_bits(cfg.dims[1]),
            f32::from_bits(cfg.dims[2]),
        ],
        ddgi_mode_word: 1,
        _pad: [0; 3],
    }
}

// ---- DdgiResolveSet (the cross-plugin resolve → consumer ordering seam) ---------------

/// The `Main`-schedule ordering seam that pins the DDGI grid resolve BEFORE its consumer —
/// the GI analogue of
/// [`PunctualResolveSet`](crate::shadow_atlas::PunctualResolveSet).
///
/// # Why a named set, not add-order
///
/// [`resolve_ddgi_grid`] (in [`DdgiPlugin`](crate::ddgi_plugin::DdgiPlugin)) writes
/// [`ResolvedDdgi`], and the consumer that uploads the grid UBO + reads the gate lives in
/// a DIFFERENT plugin, so their per-system `SystemKey`s are not co-visible — a
/// `.after(key)` edge is impossible across the plugin boundary. A set-to-set edge is
/// pinned **by name** and holds REGARDLESS of plugin add-order (the
/// [`PunctualResolveSet`](crate::shadow_atlas::PunctualResolveSet) precedent). A consumer
/// joins with `.after_set(DdgiResolveSet)`.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Debug)]
pub struct DdgiResolveSet;

// ---- the cold single-writer systems ---------------------------------------------------

/// Writes [`ResolvedDdgi`] from the cold [`DdgiConfig`] — the SINGLE writer of the grid
/// carrier (the one-producer-per-field discipline), the GI analogue of
/// [`resolve_csm_cascades`](crate::csm_config::resolve_csm_cascades). CAMERA-INDEPENDENT
/// (Decision D1): no `ViewUniform` read, no per-FIF ring, no refit — it recomputes the
/// world-fixed grid from cold owner state each frame (a DISABLED config early-outs to the
/// all-zero selection).
//
// `clippy::needless_pass_by_value`: `Res`/`ResMut` are by-value `SystemParam`s
// read/written through reborrows — the same false-positive `resolve_csm_cascades` carries.
#[allow(clippy::needless_pass_by_value)]
pub fn resolve_ddgi_grid(cfg: Res<DdgiConfig>, mut out: ResMut<ResolvedDdgi>) {
    *out = resolve_ddgi(&cfg);
}

/// Bridges the resolved [`ResolvedDdgi`] carrier and the [`LightingConfig`] header gate —
/// the GI analogue of
/// [`sync_punctual_light_gate`](crate::shadow_atlas::sync_punctual_light_gate) (which likewise
/// reads its `Resolved*` carrier's `mode_word`, not the owner config). It is the SOLE
/// production writer of [`LightingConfig::ddgi_indirect`], keeping the header's word-7 DDGI
/// bit ([`DDGI_MODE_BIT`], bit 4) in lock-step with `ResolvedDdgi::ddgi_mode_word`.
///
/// # Why the carrier, not `DdgiConfig` (the SDFDDGI host-hook defect)
///
/// The carrier is written by ONE system
/// ([`resolve_ddgi_grid_gated`](crate::ddgi_update::resolve_ddgi_grid_gated)) that folds the
/// config, the rung-R9c boot freeze AND the device caps. Reading it here means the header bit
/// can be 1 ONLY when the carrier — the exact bytes [`upload_ddgi_grid`](crate::upload_ddgi_grid)
/// puts in the resolve's b18 UBO — is a non-zero grid: `bit == 1 ⇒ mode_word == 1 ⇒
/// inv_spacing > 0`. The last implication is enforced at ONE site,
/// [`DdgiConfig::grid_is_sampleable`] (folded into [`DdgiConfig::enabled`], which every
/// resolve variant funnels through): a degenerate spacing or a zero dimension DISABLES the
/// config rather than packing `mode_word == 1` over a zeroed reciprocal (W1). The
/// [`debug_assert`] in the host's b18 upload step codifies the invariant; the clamp is what
/// enforces it in release. The previous shape recomputed its own predicate from
/// `DdgiConfig` + the freeze and ignored caps, so a no-storage device could open the gate over a
/// zero grid (NaN through the resolve's `1 / inv_spacing`).
///
/// # Value-gated write
///
/// `cfg.ddgi_indirect` is written only on an actual flip, so a static frame does zero work
/// and never dirties the light table (mirrors `sync_punctual_light_gate`'s value gate).
///
/// # Registration — app-wired, with BOTH ordering edges
///
/// NOT registered by any plugin here: it bridges the DDGI plugin's carrier and the lighting
/// plugin's [`LightingConfig`] / [`LightTableDirty`], so only the composing app (which adds
/// BOTH) registers it — `boyko_app::plugins::register_main_frame_systems`, in the same `Main`
/// builder as the other `sync_*_light_gate`s, as
/// `.after_set(DdgiResolveSet).before_set(LightCollectSet)`: after the resolve so it reads
/// THIS frame's carrier, and before `collect_lights` so the header packs the bit in the SAME
/// frame it flips (the `sync_cluster_light_gate` / `sync_sv0_light_gate` precedent).
#[allow(clippy::needless_pass_by_value)]
pub fn sync_ddgi_light_gate(
    resolved: Res<ResolvedDdgi>,
    mut cfg: ResMut<LightingConfig>,
    mut dirty: ResMut<LightTableDirty>,
) {
    // ONE predicate: the carrier's mode word (config + R9c freeze + caps, folded upstream).
    let on = resolved.ddgi_mode_word != 0;
    // Value gate BEFORE the `DerefMut`: flip-only write, flip-only table dirtying.
    if cfg.ddgi_indirect != on {
        cfg.ddgi_indirect = on;
        dirty.0 = true;
    }
    // Keep the bit-position pin visible at the single writer (the header packs it via
    // `LightingConfig::shadow_gate_word`, which reads `DDGI_MODE_BIT`).
    debug_assert_eq!(DDGI_MODE_BIT, 4, "invariant: DDGI header gate is word-7 bit 4");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_ddgi_size_is_pinned() {
        assert_eq!(RESOLVED_DDGI_BYTES, 48);
        assert_eq!(size_of::<ResolvedDdgi>(), 48);
    }

    #[test]
    fn default_config_is_the_zero_gate() {
        let cfg = DdgiConfig::default();
        assert!(!cfg.ddgi_indirect);
        assert!(!cfg.enabled());
        // The resolve of the default (disabled) config is the all-zero carrier.
        assert_eq!(resolve_ddgi(&cfg), ResolvedDdgi::DISABLED);
        // Default matches resolving the default (the 0%-gate anchor).
        assert_eq!(ResolvedDdgi::default(), resolve_ddgi(&DdgiConfig::default()));
    }

    #[test]
    fn disabled_resolved_is_all_zero_bytes() {
        // The byte-image argument: DISABLED must be every byte zero.
        let bytes: [u8; RESOLVED_DDGI_BYTES] =
            unsafe { core::mem::transmute(ResolvedDdgi::DISABLED) };
        assert!(bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn enabled_resolved_packs_grid_params() {
        let cfg = DdgiConfig { ddgi_indirect: true, ..DdgiConfig::default() };
        let r = resolve_ddgi(&cfg);
        assert_eq!(r.ddgi_mode_word, 1);
        assert_eq!(r.origin, [cfg.origin[0], cfg.origin[1], cfg.origin[2], 0.0]);
        assert_eq!(r.inv_spacing_dims[0], 1.0 / cfg.spacing);
        assert_eq!(r.inv_spacing_dims[1].to_bits(), cfg.dims[0]);
        assert_eq!(r.inv_spacing_dims[2].to_bits(), cfg.dims[1]);
        assert_eq!(r.inv_spacing_dims[3].to_bits(), cfg.dims[2]);
    }

    // ---- SDFDDGI host-hook defect gates (b) + (c) ---------------------------------------
    //
    // These pin the ONE carrier the host uploads into the resolve's b18 UBO and the ONE
    // predicate the header gate reads. Before the fix the b18 buffer was never written after
    // its zero seed, `sync_ddgi_light_gate` recomputed its own predicate from `DdgiConfig` +
    // the freeze (ignoring `DdgiCaps`), and nothing registered it — the atlas was written and
    // never read (`docs/RENDER-SDFDDGI-PLAN.md`, "Defects found after SHIPPED").

    /// Gate (b): the 48 bytes the b18 upload writes for an ENABLED default resolve are the
    /// twelve little-endian words the shader's `ResolvedDdgi` cbuffer reads
    /// (`deferred_pbr.hlsl` b18: `gDdgiOrigin`@0, `gDdgiInvSpacDims`@16, `gDdgiMode`@32,
    /// `_gDdgiPad`@36 — member 3 `Offset 36` pinned in every committed b18 resolve `.spv`,
    /// i.e. `deferred_pbr*` / `vb_shade_split*`; the standalone `ddgi_probe_gi_resolve`
    /// harness splits that pad into `gSampleCount`@36 + `uint2`@40, see `as_bytes`).
    #[test]
    fn enabled_default_as_bytes_is_the_twelve_expected_le_words() {
        let cfg = DdgiConfig { ddgi_indirect: true, ..DdgiConfig::default() };
        let bytes = resolve_ddgi(&cfg).as_bytes();
        let words: [u32; 12] = core::array::from_fn(|i| {
            u32::from_le_bytes([bytes[4 * i], bytes[4 * i + 1], bytes[4 * i + 2], bytes[4 * i + 3]])
        });
        // The owner-locked default grid: origin (-16, -2, -16), spacing 2.0 ⇒ inv 0.5, dims
        // 16×8×16, mode 1, three zero pad words. Literal values on purpose — a drift in the
        // default is a pixel change and must be a deliberate edit here too.
        let expected = [
            (-16.0f32).to_bits(),
            (-2.0f32).to_bits(),
            (-16.0f32).to_bits(),
            0,
            (1.0f32 / 2.0).to_bits(),
            16,
            8,
            16,
            1,
            0,
            0,
            0,
        ];
        assert_eq!(words, expected, "b18 byte image of the enabled default grid");
    }

    /// Gate (b): the DISABLED carrier's byte image is all zero — the boot seed of the b18
    /// buffer IS this image, which is what lets the host never write DISABLED after boot.
    #[test]
    fn disabled_as_bytes_is_all_zero() {
        assert!(ResolvedDdgi::DISABLED.as_bytes().iter().all(|&b| b == 0));
    }

    /// Gate (b): `as_bytes` is exactly the `#[repr(C)]` byte image (the transmute the
    /// existing `disabled_resolved_is_all_zero_bytes` pin uses) — no reordering, no gaps.
    #[test]
    fn as_bytes_equals_the_repr_c_transmute() {
        let r = resolve_ddgi(&DdgiConfig {
            ddgi_indirect: true,
            origin: [1.5, -2.25, 3.0],
            spacing: 0.75,
            dims: [4, 3, 2],
        });
        // SAFETY: `ResolvedDdgi` is `#[repr(C)]`, 48 bytes with const-asserted offsets and
        // no padding holes; every lane is a POD `f32`/`u32`, so every byte is initialized.
        let raw: [u8; RESOLVED_DDGI_BYTES] = unsafe { core::mem::transmute(r) };
        assert_eq!(r.as_bytes(), raw);
    }

    /// W1: a grid whose params cannot be sampled without a non-finite probe coordinate is
    /// DISABLED at the ONE carrier, so no downstream reader can open the header gate over it.
    ///
    /// The load-bearing invariant of the whole lane is `bit == 1 ⇒ mode_word == 1 ⇒
    /// inv_spacing > 0` — the header gate, the b18 bytes and the update arming all derive from
    /// `ddgi_mode_word`, and the resolve shader recomputes `spacing = 1 / inv_spacing`. An
    /// `inv_spacing` of 0 makes that `+inf`, and `origin + float3(0) * inf` is NaN, which
    /// inverts under fast-math `NMin`/`NMax` into a BLACK pixel on every `is_sdf_lit` receiver.
    /// So a non-positive / non-finite / subnormal spacing (and a zero dimension — a grid with
    /// no probes to blend) must resolve to the all-zero carrier, not to `mode_word == 1` with
    /// a zeroed reciprocal.
    #[test]
    fn a_degenerate_grid_resolves_disabled() {
        let base = DdgiConfig { ddgi_indirect: true, ..DdgiConfig::default() };

        // `1/spacing` is not finite-and-positive for any of these.
        for spacing in [
            0.0f32,
            -0.0,
            -2.0,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            // Subnormal: positive and finite, yet `1.0 / 1e-40` overflows to `+inf`, and
            // `(p - origin) * inf` is NaN at `p == origin`.
            1.0e-40,
        ] {
            let cfg = DdgiConfig { spacing, ..base };
            assert!(!cfg.enabled(), "spacing {spacing:?} is not a sampleable grid");
            assert_eq!(
                resolve_ddgi(&cfg),
                ResolvedDdgi::DISABLED,
                "spacing {spacing:?} must resolve to the all-zero carrier"
            );
        }

        // A zero dimension is an empty grid — the promise `DdgiConfig::enabled`'s doc has
        // carried since I0 ("a later rung ANDs in dims nonzero").
        for dims in [[0, 8, 16], [16, 0, 16], [16, 8, 0], [0, 0, 0]] {
            let cfg = DdgiConfig { dims, ..base };
            assert!(!cfg.enabled(), "dims {dims:?} is an empty grid");
            assert_eq!(resolve_ddgi(&cfg), ResolvedDdgi::DISABLED, "dims {dims:?} ⇒ DISABLED");
        }

        // The enabled path is untouched: every carrier with `mode_word == 1` carries a
        // finite, positive `inv_spacing` — the invariant stated as a property, not as prose.
        for spacing in [f32::MIN_POSITIVE, 0.01, 0.75, 2.0, 1.0e30] {
            let r = resolve_ddgi(&DdgiConfig { spacing, ..base });
            assert_eq!(r.ddgi_mode_word, 1, "spacing {spacing:?} is sampleable");
            assert!(
                r.inv_spacing_dims[0].is_finite() && r.inv_spacing_dims[0] > 0.0,
                "spacing {spacing:?} ⇒ inv_spacing {} must be finite and positive",
                r.inv_spacing_dims[0]
            );
        }
    }

    /// W1: the degenerate clamp is at the ONE carrier, so the DEVICE-caps and R9c-freeze folds
    /// inherit it — a frozen-ON boot cannot resurrect a grid the config cannot sample.
    #[test]
    fn a_degenerate_grid_stays_disabled_through_the_frozen_fold() {
        use crate::ddgi_update::{DdgiCaps, resolve_ddgi_grid_frozen};
        use crate::render_path_config::RenderPathFrozenConsumers;
        use crate::ssao_config::SsaoConfig;

        let bad = DdgiConfig { ddgi_indirect: true, spacing: 0.0, ..DdgiConfig::default() };
        let frozen_on = RenderPathFrozenConsumers::new(SsaoConfig::default(), true, true);
        assert_eq!(
            resolve_ddgi_grid_frozen(&bad, &DdgiCaps::new(true), &frozen_on),
            ResolvedDdgi::DISABLED,
            "the boot bit re-enables the config, but the grid is still not sampleable"
        );
    }

    /// Gate (c): the header gate reads ONLY the carrier. Over `DISABLED` the bit stays 0 and
    /// the table is not dirtied; an enabled carrier sets both; a second run over the same
    /// carrier does NOT re-dirty (the value gate); a DISABLED carrier drops the bit again.
    ///
    /// This is the property that makes "the upload precedes the first open gate" hold by
    /// construction: the bit cannot be 1 while the carrier (the only thing the host uploads)
    /// is the zero image.
    #[test]
    fn sync_ddgi_light_gate_follows_the_carrier_and_value_gates_the_dirty_bit() {
        use boyko_ecs::ecs::core::app::App;

        let mut app = App::new();
        app.insert_resource(ResolvedDdgi::DISABLED);
        app.insert_resource(LightingConfig::default());
        app.insert_resource(LightTableDirty(false));

        app.world_mut().run_system(sync_ddgi_light_gate);
        assert!(!app.world().resource::<LightingConfig>().ddgi_indirect, "DISABLED ⇒ bit 0");
        assert!(!app.world().resource::<LightTableDirty>().0, "DISABLED ⇒ table untouched");

        let on = resolve_ddgi(&DdgiConfig { ddgi_indirect: true, ..DdgiConfig::default() });
        *app.world_mut().resource_mut::<ResolvedDdgi>() = on;
        app.world_mut().run_system(sync_ddgi_light_gate);
        {
            let cfg = app.world().resource::<LightingConfig>();
            assert!(cfg.ddgi_indirect, "enabled carrier ⇒ bit 1");
            assert_eq!((cfg.shadow_gate_word() >> DDGI_MODE_BIT) & 1, 1, "word-7 bit 4 packed");
        }
        assert!(app.world().resource::<LightTableDirty>().0, "the flip dirties the table");

        // Value gate: the same carrier again must NOT re-dirty.
        app.world_mut().resource_mut::<LightTableDirty>().0 = false;
        app.world_mut().run_system(sync_ddgi_light_gate);
        assert!(!app.world().resource::<LightTableDirty>().0, "static carrier ⇒ zero work");

        // A DISABLED carrier drops the bit (the DISABLE transition) and dirties once.
        *app.world_mut().resource_mut::<ResolvedDdgi>() = ResolvedDdgi::DISABLED;
        app.world_mut().run_system(sync_ddgi_light_gate);
        assert!(!app.world().resource::<LightingConfig>().ddgi_indirect, "DISABLED ⇒ bit 0");
        assert!(app.world().resource::<LightTableDirty>().0, "the drop dirties the table");
    }
}
