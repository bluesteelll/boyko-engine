//! Ray vocabulary: [`Ray`] plus analytic ray-vs-shape intersectors
//! ([`ray_sphere`], [`ray_aabb`]).
//!
//! These are tiny CPU-only geometry helpers (no SIMD lane / GPU / serialize
//! contract beyond `#[repr(C)]` for layout stability) used by the world-space UI
//! cursor pick: a per-pickable loop ray-tests each bound, so every fn here is
//! `#[inline]` (trivial, cross-crate, must be visible to LTO). No FMA / fast-math
//! path is used, matching the crate's bit-deterministic discipline.

use crate::vec::Vec3;

/// Squared length below which a ray direction is treated as degenerate.
///
/// The intersectors return [`None`] for any `dir` with `length_squared() <=
/// RAY_DIR_MIN_SQ`. This is the load-bearing release guard (W2): it makes a
/// zero/near-zero direction ALWAYS a miss, INCLUDING the origin-inside case that
/// would otherwise spuriously report a hit at `t == 0.0`.
pub const RAY_DIR_MIN_SQ: f32 = 1.0e-12;

/// A parametric ray `origin + t * dir` (`t >= 0`).
///
/// `dir` is expected normalized for the returned `t` to read as a Euclidean
/// distance; the formulas in [`ray_sphere`] / [`ray_aabb`] assume a unit `dir`
/// (the caller passes a normalized direction from the camera ray-gen). A
/// near-unit `dir` is `debug_assert!`ed inside each intersector as the caller
/// contract.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    /// World-space ray origin.
    pub origin: Vec3,
    /// Ray direction (normalized for `t` to be a distance).
    pub dir: Vec3,
}

impl Ray {
    /// Constructs a ray from an origin and a direction.
    #[inline]
    pub const fn new(origin: Vec3, dir: Vec3) -> Self {
        Self { origin, dir }
    }

    /// The point at parameter `t`: `origin + dir * t`.
    #[inline]
    pub fn at(self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }
}

/// Nearest non-negative intersection `t` of `ray` with the sphere
/// `(center, radius)`, or [`None`].
///
/// Returns [`None`] on a miss (`disc < 0`), when the whole sphere is behind the
/// origin, or on a degenerate `dir` (the W2 guard). An origin INSIDE the sphere
/// returns `t == 0.0` (the entry root is behind, the exit root ahead — the pick
/// treats an inside hit as a hit at distance 0), EXCEPT on a zero/near-zero
/// `dir`, which returns [`None`].
///
/// Geometric quadratic form (assumes a unit `dir`): with `oc = origin - center`,
/// `b = oc·dir`, `c = oc·oc - radius²`, `disc = b² - c`. The roots are
/// `t = -b ∓ sqrt(disc)`.
#[inline]
pub fn ray_sphere(ray: Ray, center: Vec3, radius: f32) -> Option<f32> {
    // W2: degenerate-ray guard FIRST — a zero/near-zero dir is always a miss,
    // so an origin-inside-sphere with a degenerate dir cannot return Some(0.0).
    if ray.dir.length_squared() <= RAY_DIR_MIN_SQ {
        return None;
    }
    // Caller contract: the pick always passes a normalized dir; the unit
    // assumption below relies on it. The release guard above is the safety.
    debug_assert!(
        (ray.dir.length_squared() - 1.0).abs() < 1e-3,
        "invariant: ray_sphere expects a near-unit dir"
    );

    let oc = ray.origin - center;
    let b = oc.dot(ray.dir);
    let c = oc.dot(oc) - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None; // miss
    }

    let sq = disc.sqrt();
    let t0 = -b - sq; // near root (entry)
    if t0 >= 0.0 {
        return Some(t0); // ahead, outside the sphere
    }
    let t1 = -b + sq; // far root (exit)
    if t1 >= 0.0 {
        return Some(0.0); // origin inside the sphere -> distance 0
    }
    None // both roots behind -> the sphere is fully behind the origin
}

/// Nearest non-negative intersection `t` of `ray` with the axis-aligned box
/// `[center - half_extents, center + half_extents]`, or [`None`].
///
/// Returns [`None`] on a miss, when the box is fully behind the origin, or on a
/// degenerate `dir` (the W2 guard). Origin-inside returns `t == 0.0` (except on a
/// zero/near-zero `dir`, which returns [`None`]).
///
/// Slab method: a zero `dir` component yields a `±inf` reciprocal, which the
/// per-axis `min`/`max` slab combination tolerates (an axis-parallel ray that
/// misses on that axis yields `tmax < tmin` → [`None`]; an axis-parallel ray
/// through the box still intersects via the other two finite slabs).
#[inline]
pub fn ray_aabb(ray: Ray, center: Vec3, half_extents: Vec3) -> Option<f32> {
    // W2: degenerate-ray guard FIRST — see ray_sphere. An origin-inside-box with
    // a degenerate dir returns None here, not Some(0.0).
    if ray.dir.length_squared() <= RAY_DIR_MIN_SQ {
        return None;
    }
    debug_assert!(
        (ray.dir.length_squared() - 1.0).abs() < 1e-3,
        "invariant: ray_aabb expects a near-unit dir"
    );
    debug_assert!(
        half_extents.x >= 0.0 && half_extents.y >= 0.0 && half_extents.z >= 0.0,
        "invariant: ray_aabb expects non-negative half-extents"
    );

    // Component-wise reciprocal. A zero component -> ±inf; the slab min/max below
    // turns an axis-parallel miss into tmax < tmin (handled), and a finite-but-
    // tiny dir that slipped the guard into a huge inv that likewise collapses to
    // a miss on an off-axis ray.
    let inv = Vec3::new(
        1.0 / ray.dir.x,
        1.0 / ray.dir.y,
        1.0 / ray.dir.z,
    );
    let lo = (center - half_extents - ray.origin).componentwise_mul(inv);
    let hi = (center + half_extents - ray.origin).componentwise_mul(inv);

    let tmin = lo
        .x
        .min(hi.x)
        .max(lo.y.min(hi.y))
        .max(lo.z.min(hi.z));
    let tmax = lo
        .x
        .max(hi.x)
        .min(lo.y.max(hi.y))
        .min(lo.z.max(hi.z));

    if tmax < 0.0 || tmin > tmax {
        return None; // box behind the origin, or no slab overlap (miss)
    }
    if tmin >= 0.0 {
        return Some(tmin); // entry ahead
    }
    Some(0.0) // origin inside the box -> distance 0
}
