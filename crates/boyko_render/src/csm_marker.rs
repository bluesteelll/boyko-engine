//! CSM Increment 1a — the structural shadow-caster capability marker.
//!
//! Critic C2: CSM-casting and SDF/MDF-occluding are MUTUALLY-EXCLUSIVE capabilities keyed
//! by COMPONENT PRESENCE, not a runtime flag — an entity with [`ShadowCaster`] is gathered
//! into the cascade depth pass (Inc-1b), an entity that occludes through the SDF/MDF field
//! is not, and the two never mix. This is the capability-is-structural principle (a
//! capability is the presence of its component; a system iterates only the entities that
//! structurally have it).
//!
//! The marker is DEFINED here (Inc-1a, the data/policy layer). The gather that ENFORCES the
//! exclusivity — iterating `Query<&InstanceModelCol, With<ShadowCaster>>` into the per-
//! cascade draw lists — lands with the GPU depth pass (Inc-1b).

use boyko_macros::Component;

/// The structural shadow-caster capability: an entity carrying [`ShadowCaster`] casts into
/// the CSM cascades (the Inc-1b depth pass gathers it); an entity WITHOUT it does not. A
/// zero-sized marker (`#[derive(Component)]`, table storage) — its PRESENCE is the whole
/// datum, exactly as a `Dynamic` / `Simulated` physics marker or `LightEnabled` is.
///
/// CSM-casting and SDF/MDF-occlusion are mutually exclusive by this presence (critic C2):
/// a mesh that casts CSM shadows carries [`ShadowCaster`]; geometry occluding through the
/// SDF field does not, so the cascade gather and the field marcher never double-count an
/// occluder.
#[derive(Component, Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct ShadowCaster;

// Layout pin: a zero-sized marker carries no data — its presence is the datum.
const _: () = assert!(size_of::<ShadowCaster>() == 0);
