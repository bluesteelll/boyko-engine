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
//! state — later rungs also fold in "dims nonzero".
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
    /// The world-space distance between adjacent probes (uniform on all axes). `> 0`;
    /// the resolve's world→probe index divides by this (carried as `inv_spacing` in
    /// [`ResolvedDdgi`] so the per-pixel path is div-free).
    pub spacing: f32,
    /// The probe count per axis (`[x, y, z]`). Owner-locked default `[16, 8, 16]` = 2048
    /// probes. A zero dimension is a degenerate (empty) grid — later rungs fold "dims
    /// nonzero" into [`Self::enabled`].
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
    /// Whether the GI resolve runs — the structural predicate. At I0 this is exactly
    /// [`Self::ddgi_indirect`] (the 0%-gate anchor); a later rung ANDs in "dims nonzero"
    /// (a degenerate grid resolves nothing to sample). False ⇒ the 0%-gate (no probe
    /// update, the resolve's GI term off). Mirrors
    /// [`CsmConfig::enabled`](crate::csm_config::CsmConfig::enabled).
    #[inline]
    pub fn enabled(&self) -> bool {
        self.ddgi_indirect
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
/// [`ResolvedCsm`] / [`ResolvedShadowAtlas`](crate::shadow_atlas::ResolvedShadowAtlas) it
/// needs no per-FIF ring (one buffer, written once per frame, read by every in-flight
/// frame with no Write-After-Read hazard on a static config — but at I0 it is
/// bound-but-unread anyway, so even a dynamic config is benign).
///
/// DISABLED == [`Default`] == all-zero: the resolve gates on `ddgi_mode_word` (mirrored
/// from the LightBuf word-7 bit-4 gate the single writer sets), so all-zero is "off".
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
#[inline]
pub fn resolve_ddgi(cfg: &DdgiConfig) -> ResolvedDdgi {
    if !cfg.enabled() {
        return ResolvedDdgi::DISABLED;
    }

    // `enabled()` does not yet guarantee `spacing > 0`; guard the reciprocal so a
    // misconfigured non-positive spacing yields a finite `inv_spacing` of 0 (the resolve
    // then maps everything to probe 0 — a benign degenerate, never a NaN/inf in the UBO).
    let inv_spacing = if cfg.spacing > 0.0 { 1.0 / cfg.spacing } else { 0.0 };

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

/// Bridges the [`DdgiConfig`] gate and the [`LightingConfig`] header gate — the GI
/// analogue of
/// [`sync_punctual_light_gate`](crate::shadow_atlas::sync_punctual_light_gate). It is the
/// SOLE production writer of [`LightingConfig::ddgi_indirect`], keeping the header's
/// word-7 DDGI bit ([`DDGI_MODE_BIT`], bit 4) in lock-step with the structural GI
/// predicate [`DdgiConfig::enabled`].
///
/// # Value-gated write
///
/// `cfg.ddgi_indirect` is written only on an actual flip, so a static frame does zero work
/// and never dirties the light table (mirrors `sync_punctual_light_gate`'s value gate).
///
/// # Registration — app-wired (matches `sync_punctual_light_gate`)
///
/// NOT registered by any plugin here: it bridges the DDGI plugin's [`DdgiConfig`] and the
/// lighting plugin's [`LightingConfig`] / [`LightTableDirty`], so only the composing app
/// (which adds BOTH) may register it — after `resolve_ddgi_grid`, in the same builder
/// closure as the other light-gate sync systems.
#[allow(clippy::needless_pass_by_value)]
pub fn sync_ddgi_light_gate(
    ddgi: Res<DdgiConfig>,
    mut cfg: ResMut<LightingConfig>,
    mut dirty: ResMut<LightTableDirty>,
) {
    let on = ddgi.enabled();
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
}
