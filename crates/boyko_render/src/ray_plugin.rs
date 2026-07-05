//! The [`RayPlugin`] (HW-RT rung R1) — inserts the derived [`RayBackendConfig`]
//! carrier + its [`RayCaps`] device-tier input and registers the cold
//! [`resolve_ray_backend_system`] under [`RayResolveSet`], symmetric with
//! [`DdgiPlugin`](crate::ddgi_plugin::DdgiPlugin).

use boyko_ecs::ecs::core::app::{App, Plugin};

use crate::ray_backend::{
    RayBackendConfig, RayCaps, RayResolveSet, resolve_ray_backend_system,
};
use crate::ray_shadow_config::{RayShadowConfig, ResolvedRayShadow, resolve_ray_shadow_system};

/// Registers the dormant ray-backend seam (HW-RT rung R1): inserts the derived
/// [`RayBackendConfig`] carrier (default DISABLED — every cell
/// [`RayBackend::Software`](crate::ray_backend::RayBackend::Software)) + its
/// [`RayCaps`] device-tier input (default [`RtTier::Absent`](boyko_rhi_vulkan's
/// `RtTier`) — dormant until the host fills it), and schedules the cold
/// [`resolve_ray_backend_system`] (the SINGLE writer of `RayBackendConfig`) under
/// [`RayResolveSet`].
///
/// # Mirror of [`DdgiPlugin`](crate::ddgi_plugin::DdgiPlugin)
///
/// The DDGI plugin inserts the owner-set `DdgiConfig` + the derived `ResolvedDdgi`
/// and registers the cold `resolve_ddgi_grid_gated` in `DdgiResolveSet`. This
/// plugin is the ray analogue: the device-tier `RayCaps` input + the derived
/// `RayBackendConfig` + the cold `resolve_ray_backend_system` in `RayResolveSet`.
///
/// # Dormant in R1 (byte-identity)
///
/// The default `RayCaps` tier is `Absent` (`DeviceCaps::ray_query` hard-wired
/// `false`), so `resolve_ray_backend_system` writes the all-software carrier
/// regardless of add-order. No render pass reads `RayBackendConfig` and
/// `RayResolveSet` carries no command-recording consumer in R1, so nothing arms a
/// trace path. The host OVERRIDES the default `RayCaps` at device boot with the
/// real `rt_tier()` query (still `Absent` in R1), at the SAME site it fills
/// [`DdgiCaps`](crate::ddgi_update::DdgiCaps).
///
/// # `AsBuildSet` — declared, not configured here (R1)
///
/// The empty [`AsBuildSet`](crate::ray_backend::AsBuildSet) anchor is NOT
/// interned by this plugin: it has no member and no consumer in R1, and the
/// scheduler interns a set by value on first reference (a R2a
/// `.after_set(AsBuildSet)` consumer). A `configure_set(AsBuildSet)` here would be
/// an inert no-op — see the `AsBuildSet` doc.
#[derive(Default)]
pub struct RayPlugin;

impl Plugin for RayPlugin {
    fn build(&self, app: &mut App) {
        // The derived carrier (default DISABLED — all-software) + its device-tier
        // input (default `Absent`). `resolve_ray_backend_system` is the single
        // writer of `RayBackendConfig`; the default carrier already reads the
        // disabled selection, so the world is correct even before the first policy
        // run. The host overrides `RayCaps` at device boot (still `Absent` in R1).
        app.insert_resource(RayBackendConfig::default());
        app.insert_resource(RayCaps::default());

        // HW-RT rung 1b: the author-set soft-shadow tuning + its derived UBO carrier.
        // `resolve_ray_shadow_system` is the single writer of `ResolvedRayShadow`; the
        // default carrier is INSERTED as the resolved default (`ResolvedRayShadow::default`
        // == `resolve_ray_shadow(&RayShadowConfig::default())`) so frame 0 — before the
        // policy first runs — already carries the byte-identical R2a-4b UBO scalars.
        app.insert_resource(RayShadowConfig::default());
        app.insert_resource(ResolvedRayShadow::default());

        // `resolve_ray_backend_system` + `resolve_ray_shadow_system` join `RayResolveSet` —
        // the by-name ordering seam a consumer pins BEFORE (via `.after_set(RayResolveSet)`).
        // Set-to-set ordering is add-order-independent; the tier is device-fixed so there is
        // no camera/light edge to express.
        app.add_systems_cfg(|b| {
            b.add_system(resolve_ray_backend_system).in_set(RayResolveSet);
            b.add_system(resolve_ray_shadow_system).in_set(RayResolveSet);
        });
    }

    fn name(&self) -> &'static str {
        "boyko_render::RayPlugin"
    }
}
