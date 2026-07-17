//! Light pose reconciliation (standard-library Phase S4).
//!
//! [`light_reconcile`] derives a light's world position / direction from its
//! [`GlobalTransform`] and writes it into the light
//! component — so a parented or animated light tracks its transform. A light
//! WITHOUT a `GlobalTransform` keeps its self-contained pose (back-compat); the
//! `collect_lights` table builder is UNCHANGED.
//!
//! # The value gate (why a static light pays nothing)
//!
//! The write goes through a [`Mut<T>`](boyko_ecs::ecs::core::iters::query::Mut),
//! whose `DerefMut` bumps the component's `Changed` tick — and `collect_lights`
//! is `Changed`-gated, rebuilding only when a light's tick advanced. To avoid a
//! static parented light perpetually dirtying that rebuild, the write is gated
//! twice:
//!
//! 1. **`Changed<GlobalTransform>`** on the query: a frame in which no light's
//!    transform moved yields zero rows, so the system does nothing.
//! 2. **A per-lane bit-compare**: even when a light's `GlobalTransform` is
//!    re-marked `Changed` with an identical value (propagation re-touching it),
//!    the derived pose is bit-equal to the stored pose, so NO `DerefMut` happens
//!    — the light's own `Changed` tick does not advance, and `collect_lights`
//!    skips the rebuild.
//!
//! # Axis / sign convention
//!
//! The local forward axis is `-Z` (the engine convention). The stored
//! `direction` is the transform's world forward, `matrix3 · (0, 0, -1)`; its
//! MEANING is per-light-type, because the resolve consumes each kind
//! differently:
//!
//! - a `DirectionalLight` consumes it as the direction TO the light
//!   (`NoL = dot(n, dir)`), so aim the transform's `-Z` toward the light;
//! - a `SpotLight` consumes it as the SHINE axis, the way the cone points
//!   (`dot(-l, dir)`), so aim the transform's `-Z` along the beam.
//!
//! It is byte-compatible with the untouched `from_directional` / `from_spot`
//! bake, which simply re-normalizes and stores `direction`. So:
//!
//! ```text
//! direction = normalize(GlobalTransform.matrix3 · (0, 0, -1))
//! ```
//!
//! using `matrix3.mul_vec` (the row-major op). For a point light,
//! `position = GlobalTransform.translation`.

use boyko_ecs::ecs::core::iters::query::{Changed, Mut, Query};
use boyko_math::Vec3;
use boyko_scene::GlobalTransform;

use crate::light::{DirectionalLight, PointLight, SpotLight};

/// The light's local forward axis (`-Z`, the engine convention). The stored
/// light `direction` is `matrix3 · LOCAL_FORWARD`, normalized — the transform's
/// world `-Z` (a directional's to-light dir; a spot's shine axis — see the
/// module docs).
const LOCAL_FORWARD: Vec3 = Vec3::new(0.0, 0.0, -1.0);

/// Derives the light's world `direction` from a `GlobalTransform`:
/// `normalize(matrix3 · (0, 0, -1))` — the transform's world `-Z`. This is the
/// to-light direction for a `DirectionalLight` and the shine axis for a
/// `SpotLight` (the resolve consumes each per its kind; see the module docs).
///
/// Uses `Affine3A::transform_vector` (= `matrix3.mul_vec`, the row-major op) so
/// the result matches the math the `from_directional` / `from_spot` bake re-runs.
/// `Vec3::normalize` is the exact-`sqrt` form; it returns `Vec3::ZERO` on a
/// degenerate (zero-length) linear part rather than emitting `NaN`.
#[inline]
fn to_light_dir(g: &GlobalTransform) -> [f32; 3] {
    let n = g.affine().transform_vector(LOCAL_FORWARD).normalize();
    debug_assert!(
        n.is_finite(),
        "invariant: a normalized light direction must be finite (degenerate matrix3)"
    );
    [n.x, n.y, n.z]
}

/// Bit-exact per-lane inequality of two `[f32; 3]`.
///
/// A bit-compare (not `PartialEq`) is the load-bearing value-gate predicate: it
/// treats `-0.0` and `0.0` as different (a real byte change to upload) and never
/// reports two `NaN`s as equal-or-unequal ambiguously — the gate writes iff the
/// bytes actually differ, so the `Changed` tick bumps exactly on a real change.
///
/// NOTE: `Mut::set_if_neq` was considered and deliberately REJECTED here — it uses
/// `PartialEq`, under which `NaN != NaN` would defeat the static-light gate on a
/// degenerate matrix and `-0.0 == 0.0` could miss a real byte change. Do not
/// "simplify" this back to `set_if_neq`.
#[inline]
fn bits_ne(a: [f32; 3], b: [f32; 3]) -> bool {
    a[0].to_bits() != b[0].to_bits()
        || a[1].to_bits() != b[1].to_bits()
        || a[2].to_bits() != b[2].to_bits()
}

/// Writes each light's `GlobalTransform`-derived pose into its component, value-
/// and `Changed`-gated (see the module docs). Runs BEFORE `collect_lights` and
/// AFTER transform propagation (wired by
/// [`LightingPlugin`](crate::light_plugin::LightingPlugin)).
///
/// A `SkyLight` has no pose dependency and is not reconciled. A light without a
/// `GlobalTransform` is not matched by any query here, so its self-contained pose
/// is left byte-identical (back-compat).
#[allow(clippy::needless_pass_by_value)]
pub fn light_reconcile(
    mut dirs: Query<(&GlobalTransform, Mut<DirectionalLight>), Changed<GlobalTransform>>,
    mut points: Query<(&GlobalTransform, Mut<PointLight>), Changed<GlobalTransform>>,
    mut spots: Query<(&GlobalTransform, Mut<SpotLight>), Changed<GlobalTransform>>,
) {
    for (g, mut l) in dirs.iter_mut() {
        let d = to_light_dir(g);
        // Bit-gate BEFORE the `DerefMut`: the deref bumps the `Changed` tick, so
        // it must happen only on a real change.
        if bits_ne(d, l.direction) {
            l.direction = d;
        }
    }

    for (g, mut l) in points.iter_mut() {
        let t = g.translation();
        let p = [t.x, t.y, t.z];
        if bits_ne(p, l.position) {
            l.position = p;
        }
    }

    for (g, mut l) in spots.iter_mut() {
        let t = g.translation();
        let p = [t.x, t.y, t.z];
        let d = to_light_dir(g);
        // A single `DerefMut` bump covers both lanes: deref-mut once (only if
        // either lane changed), then write both. Reading `l.position` /
        // `l.direction` uses `Deref` (no bump).
        if bits_ne(p, l.position) || bits_ne(d, l.direction) {
            l.position = p;
            l.direction = d;
        }
    }
}
