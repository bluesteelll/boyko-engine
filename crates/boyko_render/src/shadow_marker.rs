//! Shadow Phase 5 Increment 1 — the structural per-LIGHT exact-shadow capability marker.
//!
//! A spot (Inc 1) / point (Inc 2) light carrying [`CastsPunctualShadow`] is eligible for an
//! exact atlas shadow map — the cold [`resolve_shadow_atlas`](crate::shadow_atlas::resolve_shadow_atlas)
//! policy gathers only `With<CastsPunctualShadow>` spots; a light WITHOUT it is structurally
//! skipped (it falls on the analytic fallback). This is the capability-is-structural principle
//! (a capability is the presence of its component; a system iterates only the entities that
//! structurally have it), exactly as a CSM caster carries
//! [`ShadowCaster`](crate::csm_marker::ShadowCaster) — except that marks a CASTER (a mesh that
//! casts INTO a map), whereas this marks a LIGHT (a source that OWNS a map).

use boyko_macros::Component;

/// The structural per-LIGHT exact-shadow capability: a spot / point light carrying
/// [`CastsPunctualShadow`] is eligible for a dedicated atlas shadow map (the cold policy
/// gathers it and may assign it a slot); a light WITHOUT it is structurally skipped and uses
/// the analytic fallback. A zero-sized marker (`#[derive(Component)]`, table storage) — its
/// PRESENCE is the whole datum, exactly as a `Dynamic` / `Simulated` physics marker,
/// `LightEnabled`, or [`ShadowCaster`](crate::csm_marker::ShadowCaster) is.
///
/// This is the LIGHT-side capability (a source that owns a map), the complement of the
/// CASTER-side [`ShadowCaster`](crate::csm_marker::ShadowCaster) (a mesh rendered INTO a map):
/// a spot light with [`CastsPunctualShadow`] gets an atlas layer, and the meshes carrying
/// [`ShadowCaster`](crate::csm_marker::ShadowCaster) are what render into it.
#[derive(Component, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct CastsPunctualShadow;

// Layout pin: a zero-sized marker carries no data — its presence is the datum.
const _: () = assert!(size_of::<CastsPunctualShadow>() == 0);
