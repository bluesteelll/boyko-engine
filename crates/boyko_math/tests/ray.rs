//! GUI P7b — ray vocabulary unit tests (`boyko_math::{Ray, ray_sphere, ray_aabb}`).
//!
//! Test matrix (GUI-P7B-PLAN §4, ray math 1-5):
//!  1. `ray_sphere`: hit ahead / miss / behind / tangent / origin-inside (t==0).
//!  2. `ray_aabb`: hit ahead / miss / behind / axis-parallel-through /
//!     axis-parallel-miss / origin-inside / grazing.
//!  3. nearest-of-many: the intersectors over several shapes resolve the nearest t.
//!  4. `Ray::at(t)` == `origin + dir * t`.
//!  5. degenerate `dir == ZERO` → BOTH `ray_sphere` & `ray_aabb` return `None`
//!     (incl. origin-inside), no panic / no NaN.
//!
//! The intersectors assume a near-unit `dir` (debug-asserted inside); every ray
//! here is built from a normalized direction so the debug builds do not trip.

use boyko_math::ray::RAY_DIR_MIN_SQ;
use boyko_math::{Ray, Vec3, ray_aabb, ray_sphere};

/// Tolerance for the analytic `t` values (a couple of ULPs over a sqrt).
const EPS: f32 = 1.0e-4;

#[track_caller]
fn approx(a: f32, b: f32, what: &str) {
    assert!((a - b).abs() <= EPS, "{what}: expected {b}, got {a} (|Δ|={})", (a - b).abs());
}

/// A normalized-direction ray (the caller contract the intersectors debug-assert).
fn ray(origin: Vec3, dir: Vec3) -> Ray {
    Ray::new(origin, dir.normalize())
}

// ════════════════════════════════════════════════════════════════════════════
// 1. ray_sphere
// ════════════════════════════════════════════════════════════════════════════

/// The canonical golden from the plan: unit sphere at (0,0,-5), ray from the
/// origin along -z hits the near surface at t ≈ 4.
#[test]
fn ray_sphere_hit_ahead_returns_entry_distance() {
    let r = ray(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
    let t = ray_sphere(r, Vec3::new(0.0, 0.0, -5.0), 1.0).expect("ray hits the sphere ahead");
    approx(t, 4.0, "near-surface entry distance");
}

/// A ray pointing away from a sphere off to the side misses (disc < 0).
#[test]
fn ray_sphere_miss_returns_none() {
    let r = ray(Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0)); // straight up
    assert_eq!(ray_sphere(r, Vec3::new(0.0, 0.0, -5.0), 1.0), None, "off-axis ray misses the sphere");
}

/// A sphere entirely BEHIND the origin (both roots negative) is a miss.
#[test]
fn ray_sphere_behind_origin_returns_none() {
    // Ray looks -z; the sphere sits at +z (behind), well clear of the origin.
    let r = ray(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
    assert_eq!(ray_sphere(r, Vec3::new(0.0, 0.0, 5.0), 1.0), None, "a sphere fully behind the origin is a miss");
}

/// A tangent ray (disc == 0) grazes the sphere at a single root.
#[test]
fn ray_sphere_tangent_returns_single_root() {
    // Ray along -z offset by exactly the radius in +x grazes the sphere's side.
    let r = ray(Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 0.0, -1.0));
    let t = ray_sphere(r, Vec3::new(0.0, 0.0, -5.0), 1.0).expect("a tangent ray grazes the sphere");
    approx(t, 5.0, "tangent grazes at the closest-approach distance (z = -5)");
}

/// An origin INSIDE the sphere returns t == 0.0 (inside hit = distance 0).
#[test]
fn ray_sphere_origin_inside_returns_zero() {
    // Origin AT the sphere center, radius 2 — the origin is inside.
    let r = ray(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
    let t = ray_sphere(r, Vec3::ZERO, 2.0).expect("origin inside the sphere is a hit");
    assert_eq!(t, 0.0, "an inside hit is at distance 0");
}

// ════════════════════════════════════════════════════════════════════════════
// 2. ray_aabb
// ════════════════════════════════════════════════════════════════════════════

/// A box ahead on the ray: entry distance is the near face.
#[test]
fn ray_aabb_hit_ahead_returns_entry_distance() {
    let r = ray(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
    // Box centered at z=-5, half-extent 1 ⇒ near face at z=-4 ⇒ t == 4.
    let t = ray_aabb(r, Vec3::new(0.0, 0.0, -5.0), Vec3::new(1.0, 1.0, 1.0)).expect("ray hits the box ahead");
    approx(t, 4.0, "near-face entry distance");
}

/// An off-axis ray that never overlaps the box's slabs misses.
#[test]
fn ray_aabb_miss_returns_none() {
    let r = ray(Vec3::new(10.0, 0.0, 0.0), Vec3::new(0.0, 0.0, -1.0));
    assert_eq!(
        ray_aabb(r, Vec3::new(0.0, 0.0, -5.0), Vec3::new(1.0, 1.0, 1.0)),
        None,
        "a laterally-offset ray misses the box"
    );
}

/// A box entirely behind the origin (tmax < 0) is a miss.
#[test]
fn ray_aabb_behind_origin_returns_none() {
    let r = ray(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
    assert_eq!(
        ray_aabb(r, Vec3::new(0.0, 0.0, 5.0), Vec3::new(1.0, 1.0, 1.0)),
        None,
        "a box fully behind the origin is a miss"
    );
}

/// An axis-parallel ray passing THROUGH the box: the parallel axis' slab is
/// ±inf, the other two finite slabs still bracket a hit.
#[test]
fn ray_aabb_axis_parallel_through_returns_hit() {
    // Ray along -z, laterally centered ⇒ x/y slabs (perpendicular) are finite and
    // overlap; the z slab drives the entry. (Not the degenerate-component case —
    // here the ray dir HAS a z component; the "parallel" axes are x and y which
    // have zero dir components.)
    let r = ray(Vec3::new(0.0, 0.0, 10.0), Vec3::new(0.0, 0.0, -1.0));
    let t = ray_aabb(r, Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0))
        .expect("an axis-parallel ray through the box hits");
    approx(t, 9.0, "entry at the +z face (z=1) from z=10");
}

/// An axis-parallel ray OFFSET so it misses on the parallel axis returns None
/// (the zero-dir-component slab is entirely outside the box ⇒ tmax < tmin).
#[test]
fn ray_aabb_axis_parallel_miss_returns_none() {
    // Ray along -z but offset in x by 5 (box half-extent 1): the x slab (a zero
    // dir component ⇒ ±inf) never contains the box ⇒ miss.
    let r = ray(Vec3::new(5.0, 0.0, 10.0), Vec3::new(0.0, 0.0, -1.0));
    assert_eq!(
        ray_aabb(r, Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0)),
        None,
        "an axis-parallel ray offset past the box misses"
    );
}

/// Origin INSIDE the box returns t == 0.0.
#[test]
fn ray_aabb_origin_inside_returns_zero() {
    let r = ray(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
    let t = ray_aabb(r, Vec3::ZERO, Vec3::new(2.0, 2.0, 2.0)).expect("origin inside the box is a hit");
    assert_eq!(t, 0.0, "an inside hit is at distance 0");
}

/// A near-edge axis-parallel ray (just INSIDE the +x face) still intersects via
/// the two finite slabs — the "grazing"/edge case the impl documents as a hit.
#[test]
fn ray_aabb_grazing_near_edge_returns_hit() {
    // Ray along -z, offset in x to JUST inside the +x face (0.999 < 1.0 = he):
    // the x slab is finite (dir.x == 0 but the origin is strictly inside the
    // x range), the z slab drives the entry. (The EXACTLY-on-the-face value
    // x == 1.0 is the degenerate 0*inf = NaN boundary — pinned separately below.)
    let r = ray(Vec3::new(0.999, 0.0, 10.0), Vec3::new(0.0, 0.0, -1.0));
    let t = ray_aabb(r, Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0)).expect("a near-edge axis-parallel ray intersects");
    approx(t, 9.0, "near-edge entry at the +z face");
}

/// PINS the observed behavior at the measure-zero boundary where the ray origin
/// lies EXACTLY on a face plane AND travels parallel to that face: the slab math
/// computes `(face - origin) * inv = 0.0 * inf = NaN`, which collapses `tmax` and
/// yields `None`. This is a documented robustness edge of the slab method (not a
/// "through-box" hit, which the impl's doc-comment scopes to the OTHER finite
/// slabs). It is irrelevant to the pick (a cursor ray exactly tangent to a face
/// plane is measure-zero; either answer is acceptable). Pinned so a future change
/// to the NaN handling is a deliberate, visible decision rather than silent drift.
#[test]
fn ray_aabb_exactly_on_face_parallel_is_observed_none() {
    // Origin EXACTLY on the +x face (x == 1.0 == half-extent), dir parallel (-z).
    let r = ray(Vec3::new(1.0, 0.0, 10.0), Vec3::new(0.0, 0.0, -1.0));
    assert_eq!(
        ray_aabb(r, Vec3::ZERO, Vec3::new(1.0, 1.0, 1.0)),
        None,
        "exactly-on-face + parallel-dir is the 0*inf=NaN boundary → observed None (pinned, not a hit guarantee)"
    );
}

// ════════════════════════════════════════════════════════════════════════════
// 3. nearest-of-many
// ════════════════════════════════════════════════════════════════════════════

/// Two spheres on the ray at t=3 and t=7: the nearest-selection (the minimum over
/// the per-shape intersectors) is the t=3 one. (Replicates the pick's nearest
/// rule with the public intersectors.)
#[test]
fn nearest_of_many_spheres_picks_closest() {
    let r = ray(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
    // Sphere A: center z=-4, radius 1 ⇒ near face t=3. B: center z=-8, radius 1 ⇒ t=7.
    let ta = ray_sphere(r, Vec3::new(0.0, 0.0, -4.0), 1.0).expect("A is hit");
    let tb = ray_sphere(r, Vec3::new(0.0, 0.0, -8.0), 1.0).expect("B is hit");
    approx(ta, 3.0, "near sphere A entry");
    approx(tb, 7.0, "far sphere B entry");
    assert!(ta < tb, "the nearest hit is sphere A (t=3 < t=7)");
}

/// A mixed sphere + AABB scene: the nearest of the two is selected by t.
#[test]
fn nearest_of_many_mixed_sphere_aabb_picks_closest() {
    let r = ray(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
    // Sphere far: center z=-10 r=1 ⇒ t=9. Box near: center z=-3 he=1 ⇒ near face t=2.
    let ts = ray_sphere(r, Vec3::new(0.0, 0.0, -10.0), 1.0).expect("sphere is hit");
    let tb = ray_aabb(r, Vec3::new(0.0, 0.0, -3.0), Vec3::new(1.0, 1.0, 1.0)).expect("box is hit");
    approx(ts, 9.0, "far sphere entry");
    approx(tb, 2.0, "near box entry");
    assert!(tb < ts, "the nearest hit is the box (t=2 < t=9)");
}

// ════════════════════════════════════════════════════════════════════════════
// 4. Ray::at
// ════════════════════════════════════════════════════════════════════════════

/// `Ray::at(t) == origin + dir * t` for several t (incl. 0 and negative).
#[test]
fn ray_at_equals_origin_plus_dir_times_t() {
    let origin = Vec3::new(1.0, -2.0, 3.0);
    let dir = Vec3::new(0.0, 0.0, -1.0); // already unit
    let r = Ray::new(origin, dir);
    for t in [0.0_f32, 1.0, 4.5, -2.0] {
        let p = r.at(t);
        let expect = origin + dir * t;
        approx(p.x, expect.x, "at().x");
        approx(p.y, expect.y, "at().y");
        approx(p.z, expect.z, "at().z");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// 5. degenerate dir == ZERO → None on BOTH, incl. origin-inside; no NaN
// ════════════════════════════════════════════════════════════════════════════

/// The degenerate-ray guard constant is the documented tiny eps² (sanity check
/// that ZERO is well below it).
#[test]
fn ray_dir_min_sq_guards_zero_direction() {
    assert!(Vec3::ZERO.length_squared() <= RAY_DIR_MIN_SQ, "ZERO is at/below the degenerate threshold");
}

/// A zero direction makes `ray_sphere` return `None` even when the origin is
/// INSIDE the sphere (the W2 guard — without it this would be a spurious Some(0)).
#[test]
fn ray_sphere_zero_dir_returns_none_even_inside() {
    // Origin AT the center (inside), but a degenerate dir.
    let degenerate = Ray::new(Vec3::ZERO, Vec3::ZERO);
    let t = ray_sphere(degenerate, Vec3::ZERO, 2.0);
    assert_eq!(t, None, "a degenerate dir is always a miss, even origin-inside");
}

/// A zero direction makes `ray_aabb` return `None` even when the origin is INSIDE
/// the box (the W2 guard).
#[test]
fn ray_aabb_zero_dir_returns_none_even_inside() {
    let degenerate = Ray::new(Vec3::ZERO, Vec3::ZERO);
    let t = ray_aabb(degenerate, Vec3::ZERO, Vec3::new(2.0, 2.0, 2.0));
    assert_eq!(t, None, "a degenerate dir is always a miss, even origin-inside");
}

/// A zero direction never escapes a NaN/Inf t (the guard returns None before any
/// divide or sqrt). Asserts the result is exactly None for both intersectors
/// across several center placements (inside, ahead, behind).
#[test]
fn zero_dir_never_yields_non_finite() {
    let degenerate = Ray::new(Vec3::new(0.5, 0.5, 0.5), Vec3::ZERO);
    for center in [Vec3::ZERO, Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.0, 0.0, 5.0)] {
        assert_eq!(ray_sphere(degenerate, center, 1.0), None, "sphere: zero dir → None (center {center:?})");
        assert_eq!(
            ray_aabb(degenerate, center, Vec3::new(1.0, 1.0, 1.0)),
            None,
            "aabb: zero dir → None (center {center:?})"
        );
    }
}
