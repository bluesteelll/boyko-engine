//! HW-RT rung 1b — the tunable soft-shadow ECS policy (CPU, unit-testable) for the
//! HWRT `rayQuery` mesh-shadow term. This is the contained data/policy layer that
//! makes the R2a-4b hardcoded shadow consts author-tunable: the ray COUNT bakes into
//! a spec-constant at pipeline build (Decision 5 — retune == relaunch), and
//! cone/tmax/tmin/bias flow through a per-FIF resolve UBO ([`ResolvedRayShadow`]).
//!
//! Principle 0: ECS-native — [`RayShadowConfig`] is the author-set `#[derive(Resource)]`
//! singleton (the cold config, NOT a side `std::Vec`/`HashMap`) and [`ResolvedRayShadow`]
//! is its derived companion Resource written by the cold [`resolve_ray_shadow_system`].
//! This mirrors the ray-backend substrate exactly:
//! [`RayBackendConfig`](crate::ray_backend::RayBackendConfig) (the carrier) +
//! [`resolve_ray_backend_system`](crate::ray_backend::resolve_ray_backend_system) (the
//! cold single-writer) + [`RayResolveSet`](crate::ray_backend::RayResolveSet) (the
//! by-name ordering seam this policy also joins).
//!
//! # No `enabled` field by design
//!
//! Shadow ENABLEMENT lives in
//! [`RayBackendConfig::table`](crate::ray_backend::RayBackendConfig)`[Shadow][Mesh]`
//! (the backend-toggle rung — HardwareTri vs Software); this struct is always-on TUNING
//! subordinate to that toggle. A never-touched default carries the exact R2a-4b consts,
//! so the render is byte-identical until the author retunes.

use boyko_macros::Resource;

use boyko_ecs::ecs::core::system::{Res, ResMut};

// ---- RayShadowConfig (the author-set Resource — mirrors RayBackendConfig) --------------

/// Author-facing soft-shadow tuning — a `World`-singleton Resource the author sets.
/// `Copy` so the cold policy reads it by value; [`Default`] == the current hardcoded
/// R2a-4b HWRT consts ⇒ a default world renders byte-identically.
///
/// NO `enabled` field by design — shadow enablement lives in
/// [`RayBackendConfig::table`](crate::ray_backend::RayBackendConfig)`[Shadow][Mesh]`
/// (the backend-toggle rung); this struct is always-on TUNING subordinate to that toggle.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct RayShadowConfig {
    /// Rays per pixel the Vogel-disk cone samples — spec-const id 0, BAKED at pipeline
    /// build (the loop unrolls against it, so a retune is a relaunch, Decision 5).
    /// Default `16` (the R2a-4b const).
    pub ray_count: u32,
    /// `tan(half-angle)` of the sun disk (~2°) — the Vogel-disk jitter radius. Default
    /// `0.035`.
    pub cone_radius: f32,
    /// World-space ray `TMax` — covers the bounded scene. Default `1e4`.
    pub tmax: f32,
    /// World-space ray `TMin`. Default `1e-3`.
    pub tmin: f32,
    /// World-space normal-offset bias (`origin += n * bias`) off the acne-prone surface.
    /// Default `1e-3`.
    pub bias: f32,
}

impl Default for RayShadowConfig {
    /// The R2a-4b hardcoded consts (`ray_count 16`, `cone_radius 0.035`, `tmax 1e4`,
    /// `tmin 1e-3`, `bias 1e-3`) — so a default world resolves the byte-identical
    /// soft-shadow selection.
    #[inline]
    fn default() -> Self {
        Self {
            ray_count: 16,
            cone_radius: 0.035,
            tmax: 1e4,
            tmin: 1e-3,
            bias: 1e-3,
        }
    }
}

// ---- ResolvedRayShadow (the derived UBO mirror) ---------------------------------------

/// The packed UBO the HWRT resolve reads at binding 20 — the derived companion the cold
/// [`resolve_ray_shadow_system`] writes. std140: 4 `f32` = 16 B = one `vec4` slot, no
/// trailing pad. `ray_count` is NOT here (it is the spec-const baked at pipeline build).
///
/// `#[repr(C)]` for a stable GPU-ready layout — the field ORDER + TYPES byte-mirror the
/// `deferred_pbr.hlsl` `RayShadowUbo` cbuffer (cone_radius @0, tmax @4, tmin @8, bias @12).
///
/// `#[derive(Resource)]` (the same derive path [`ResolvedCsm`](crate::csm_config::ResolvedCsm)
/// uses) so the plugin inserts it as a `World` singleton and the cold policy writes it via
/// `ResMut`.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct ResolvedRayShadow {
    /// `tan(half-angle)` of the sun disk (the Vogel-disk jitter radius). Offset 0.
    pub cone_radius: f32,
    /// World-space ray `TMax`. Offset 4.
    pub tmax: f32,
    /// World-space ray `TMin`. Offset 8.
    pub tmin: f32,
    /// World-space normal-offset bias. Offset 12.
    pub bias: f32,
}

// Layout pin: 4 × 4 = 16 B = one std140 vec4 slot. A change is a deliberate decision (the
// HWRT resolve's binding-20 cbuffer reads this stride).
const _: () = assert!(core::mem::size_of::<ResolvedRayShadow>() == 16);

/// The byte size of the host-coherent HWRT shadow-params UBO — `size_of::<ResolvedRayShadow>()`
/// (16 B). The HWRT resolve binds a UBO of exactly this shape at binding 20; hosts size their
/// per-FIF ring slots from THIS constant (single source — no hand-copied `16`). Mirrors
/// [`RESOLVED_CSM_BYTES`](crate::csm_config::RESOLVED_CSM_BYTES).
pub const RESOLVED_RAY_SHADOW_BYTES: usize = core::mem::size_of::<ResolvedRayShadow>();

impl Default for ResolvedRayShadow {
    /// The resolve of the default [`RayShadowConfig`] — the byte-identical R2a-4b consts,
    /// so a never-run policy (frame 0) already carries the correct UBO scalars.
    #[inline]
    fn default() -> Self {
        resolve_ray_shadow(&RayShadowConfig::default())
    }
}

// ---- the resolve decision (pure — the unit-testable policy) ----------------------------

/// Derives the [`ResolvedRayShadow`] UBO from the author [`RayShadowConfig`] — the PURE,
/// unit-testable soft-shadow resolve (the analogue of
/// [`resolve_ray_backend`](crate::ray_backend::resolve_ray_backend), the core the cold
/// system wraps). Pure COLD policy — no allocation, no `World` access.
///
/// `ray_count` is NOT carried here (it is consumed at the spec-const call site in the host
/// boot); [`ResolvedRayShadow`] carries only the runtime UBO scalars. The `debug_assert!`
/// is the retune tripwire: a `ray_count == 0` would bake an EMPTY unrolled loop
/// (`occ / 0` ⇒ NaN visibility), which is a caller bug, so it is clamped `>= 1` at the
/// spec-const site in EVERY build and asserted here.
#[inline]
pub fn resolve_ray_shadow(cfg: &RayShadowConfig) -> ResolvedRayShadow {
    debug_assert!(
        cfg.ray_count >= 1,
        "invariant: ray_count 0 bakes an empty unrolled loop -> occ/0 NaN-visibility"
    );
    ResolvedRayShadow {
        cone_radius: cfg.cone_radius,
        tmax: cfg.tmax,
        tmin: cfg.tmin,
        bias: cfg.bias,
    }
}

// ---- the cold single-writer system ----------------------------------------------------

/// Writes [`ResolvedRayShadow`] from the author [`RayShadowConfig`] — the SINGLE writer of
/// the derived UBO carrier (the one-producer-per-field discipline), the soft-shadow
/// analogue of
/// [`resolve_ray_backend_system`](crate::ray_backend::resolve_ray_backend_system). Joins
/// [`RayResolveSet`](crate::ray_backend::RayResolveSet) so it runs before any consumer.
//
// `clippy::needless_pass_by_value`: `Res`/`ResMut` are by-value `SystemParam`s
// read/written through reborrows — the same false-positive `resolve_ray_backend_system`
// carries.
#[allow(clippy::needless_pass_by_value)]
pub fn resolve_ray_shadow_system(cfg: Res<RayShadowConfig>, mut out: ResMut<ResolvedRayShadow>) {
    *out = resolve_ray_shadow(&cfg);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `RayShadowConfig::default()` carries the exact R2a-4b hardcoded consts.
    #[test]
    fn default_matches_r2a4b_consts() {
        let cfg = RayShadowConfig::default();
        assert_eq!(cfg.ray_count, 16);
        assert_eq!(cfg.cone_radius, 0.035);
        assert_eq!(cfg.tmax, 1e4);
        assert_eq!(cfg.tmin, 1e-3);
        assert_eq!(cfg.bias, 1e-3);
    }

    /// The resolve of the default drops `ray_count` and carries the four UBO scalars.
    #[test]
    fn resolve_of_default_is_the_ubo_scalars() {
        let r = resolve_ray_shadow(&RayShadowConfig::default());
        assert_eq!(
            r,
            ResolvedRayShadow { cone_radius: 0.035, tmax: 1e4, tmin: 1e-3, bias: 1e-3 }
        );
        // The `Default` impl equals the resolve of the default config (the frame-0 seed).
        assert_eq!(r, ResolvedRayShadow::default());
    }

    /// The UBO layout pin (16 B — one std140 vec4 slot).
    #[test]
    fn resolved_ray_shadow_is_16_bytes() {
        assert_eq!(core::mem::size_of::<ResolvedRayShadow>(), 16);
        assert_eq!(RESOLVED_RAY_SHADOW_BYTES, 16);
    }
}
