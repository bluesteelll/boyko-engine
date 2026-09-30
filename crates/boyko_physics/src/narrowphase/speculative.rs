//! V2's speculative contact margin (`levers/V2-speculative/01-DESIGN.md` §2.2, §2.6): how far
//! apart two shapes may be at the start of a step and still keep a contact point.
//!
//! A point is kept while its separation is at most
//!
//! ```text
//! d_eff = d + min(cap, max(0, approach) · h)
//! ```
//!
//! (owner rulings 2026-09-30, V2a/V2b and item 9): `d` the fixed speculative distance
//! ([`PhysicsConfig::speculative_distance`]), `h` the step length, `cap`
//! ([`PhysicsConfig::speculative_velocity_cap`]) the bound on the velocity term, and `approach` the
//! rate at which the two bodies close the gap at the start of the step, from the two gathered
//! (step-start) velocities, gravity not included:
//!
//! * **per point** — at a contact point `p` along the A→B normal `n`,
//!   `approach = −(v_B(p) − v_A(p))·n` with `v(p) = v + ω × (p − c)`, linear and angular, both
//!   bodies taken at the same point ([`SpecMargin::keeps`], [`SpecMargin::drops`]);
//! * **per axis** — along a SAT axis `n`, where no point is known yet, the bound no point of the
//!   pair can exceed: `approach = −(v_B − v_A)·n + |ω_A| R_A + |ω_B| R_B`, `R` the circumradius
//!   ([`SpecMargin::separates`]).
//!
//! A fixed margin only moves the boundary a landing crosses; the velocity term makes a pair
//! that closes more than `d` in one step a contact on the step before it touches, which is what
//! holds J-T's pile at every drop height (F0g: 88 of 88 runs over the gap sweep, against 35 of 88
//! with `d` alone). `cap = 0` switches the term off (`d_eff = d`); `d = 0` and `cap = 0` together
//! are the overlap-only rule, every comparison bit for bit the engine before V2.
//!
//! The same margin inflates each body's broadphase bounding sphere by
//! `d / 2 + min(cap, (|v| + |ω| R) · h)` ([`SpecStep::bp_margin`]): the two per-body terms sum to
//! at least the pair's `d_eff`, so every pair a site could keep is a candidate.
//!
//! [`PhysicsConfig::speculative_distance`]: crate::resources::PhysicsConfig::speculative_distance
//! [`PhysicsConfig::speculative_velocity_cap`]: crate::resources::PhysicsConfig::speculative_velocity_cap

use crate::components::ColliderShape;
use crate::math::Vec3;
use crate::resources::BodyState;

/// The factor [`SpecMargin::reach`] widens its bound by, so floating-point rounding in the
/// per-point velocity (a cross product and a dot) can never lift a point's `d_eff` above it.
const REACH_SLACK: f32 = 1.0 + 1.0 / 256.0;

/// The bounding radius of a collider shape about its body's centre: a sphere's radius, a box's
/// half-diagonal `|half_extents|` (the circumradius `R` of the per-axis bound).
#[inline]
pub(crate) fn circumradius(shape: &ColliderShape) -> f32 {
    match *shape {
        ColliderShape::Sphere { radius } => radius,
        ColliderShape::Box { half_extents } => half_extents.length(),
    }
}

/// The step's speculative contact parameters: the fixed distance, the velocity term's cap and
/// the step length. `Copy`, 12 B; it rides in the narrowphase's per-step parameters
/// ([`ReuseStep`](super::reuse::ReuseStep)) to every pair.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SpecStep {
    /// `d`, metres (`PhysicsConfig::speculative_distance`).
    pub(crate) d: f32,
    /// The cap on the velocity term, metres (`PhysicsConfig::speculative_velocity_cap`); `0`
    /// switches the term off.
    pub(crate) vcap: f32,
    /// `h`, the step length in seconds (`PhysicsConfig::dt`); read only while the term is on.
    pub(crate) h: f32,
}

impl SpecStep {
    /// The overlap-only rule: no fixed distance, no velocity term. What a pair with a sensor on
    /// either side uses, so an overlap report is exact (Box2D's sensors use no margin either).
    pub(crate) const OVERLAP: Self = Self { d: 0.0, vcap: 0.0, h: 0.0 };

    /// The step's parameters from the configuration's distance `d`, cap `vcap` and step `h`.
    #[inline]
    pub(crate) fn new(d: f32, vcap: f32, h: f32) -> Self {
        debug_assert!(
            d.is_finite() && d >= 0.0,
            "invariant: PhysicsConfig::speculative_distance is finite and >= 0, got {d}"
        );
        debug_assert!(
            vcap.is_finite() && vcap >= 0.0,
            "invariant: PhysicsConfig::speculative_velocity_cap is finite and >= 0, got {vcap}"
        );
        Self { d, vcap, h }
    }

    /// The fixed distance `d` alone, the velocity term off (the unit gates' step).
    #[cfg(test)]
    pub(crate) const fn fixed(d: f32) -> Self {
        Self { d, vcap: 0.0, h: 0.0 }
    }

    /// Whether the velocity term is on (`cap > 0`).
    #[inline]
    pub(crate) fn moving(self) -> bool {
        self.vcap > 0.0
    }

    /// The margin of the pair `(a, b)` (A→B), from the two bodies' gathered poses, velocities and
    /// shapes. With the velocity term off it reads neither body.
    #[inline]
    pub(crate) fn pair(self, a: &BodyState, b: &BodyState) -> SpecMargin {
        if !self.moving() {
            return SpecMargin::fixed(self.d);
        }
        SpecMargin::with_motion(self, Motion::of(a), circumradius(&a.shape), Motion::of(b), circumradius(&b.shape))
    }

    /// The margin of body `a` against the immovable SDF field (body B at rest, no extent).
    #[inline]
    pub(crate) fn against_field(self, a: &BodyState) -> SpecMargin {
        if !self.moving() {
            return SpecMargin::fixed(self.d);
        }
        SpecMargin::with_motion(self, Motion::of(a), circumradius(&a.shape), Motion::REST, 0.0)
    }

    /// Body `body`'s broadphase margin, what its bounding sphere is inflated by:
    /// `d / 2 + min(cap, (|v| + |ω| R) · h)`, or `d / 2` with the velocity term off (`0.5 · 0.0`
    /// is `+0.0`, so the overlap-only rule leaves every radius bit as it was).
    #[inline]
    pub(crate) fn bp_margin(self, body: &BodyState) -> f32 {
        let half = 0.5 * self.d;
        if !self.moving() {
            return half;
        }
        let sweep = body.linear_velocity.length()
            + body.angular_velocity.length() * circumradius(&body.shape);
        half + (sweep * self.h).min(self.vcap)
    }
}

/// One body's step-start motion: centre, linear and angular velocity.
#[derive(Clone, Copy, Debug)]
struct Motion {
    c: Vec3,
    v: Vec3,
    w: Vec3,
}

impl Motion {
    /// A body at rest at the origin: the SDF field's side of a field contact.
    const REST: Self = Self { c: Vec3::ZERO, v: Vec3::ZERO, w: Vec3::ZERO };

    #[inline]
    fn of(body: &BodyState) -> Self {
        Self { c: body.position, v: body.linear_velocity, w: body.angular_velocity }
    }

    /// The velocity of the body's material point at `p`.
    #[inline]
    fn at(&self, p: Vec3) -> Vec3 {
        self.v + self.w.cross(p - self.c)
    }
}

/// One pair's speculative margin on one step (A→B): answers every keep and early-out site of
/// every generator. Built once per pair by [`SpecStep::pair`] / [`SpecStep::against_field`] and
/// passed down by reference; with the velocity term off it is `d` alone and no site reads a
/// velocity.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SpecMargin {
    d: f32,
    vcap: f32,
    h: f32,
    /// `|ω_A| R_A + |ω_B| R_B`, the angular part of the per-axis bound.
    spin: f32,
    a: Motion,
    b: Motion,
}

impl SpecMargin {
    /// The overlap-only rule (`d = 0`, no velocity term): what the public `d = 0` generators and
    /// every pre-V2 unit oracle use.
    pub(crate) const OVERLAP: Self = Self::fixed(0.0);

    /// The fixed distance `d` alone.
    #[inline]
    pub(crate) const fn fixed(d: f32) -> Self {
        Self { d, vcap: 0.0, h: 0.0, spin: 0.0, a: Motion::REST, b: Motion::REST }
    }

    #[inline]
    fn with_motion(step: SpecStep, a: Motion, ra: f32, b: Motion, rb: f32) -> Self {
        let spin = a.w.length() * ra + b.w.length() * rb;
        Self { d: step.d, vcap: step.vcap, h: step.h, spin, a, b }
    }

    /// The fixed distance `d`.
    #[inline]
    pub(crate) fn d(&self) -> f32 {
        self.d
    }

    /// Whether the velocity term is on.
    #[inline]
    pub(crate) fn moving(&self) -> bool {
        self.vcap > 0.0
    }

    /// An upper bound on every `d_eff` the pair can produce: `d + cap`, or `d` with the velocity
    /// term off. A kept point's separation is never above it.
    #[inline]
    pub(crate) fn bound(&self) -> f32 {
        if self.moving() { self.d + self.vcap } else { self.d }
    }

    /// `d + min(cap, approach · h)` for a positive `approach`, `d` otherwise (a NaN included).
    #[inline]
    fn widen(&self, approach: f32) -> f32 {
        if approach > 0.0 { self.d + (approach * self.h).min(self.vcap) } else { self.d }
    }

    /// `d_eff` along the SAT axis `n` (unit, oriented A→B): the per-axis bound.
    #[inline]
    fn axis(&self, n: Vec3) -> f32 {
        self.widen(-(self.b.v - self.a.v).dot(n) + self.spin)
    }

    /// `d_eff` at the point `p` along the A→B normal `n`: both bodies' velocities at `p`.
    #[inline]
    pub(crate) fn point(&self, p: Vec3, n: Vec3) -> f32 {
        self.widen(-(self.b.at(p) - self.a.at(p)).dot(n))
    }

    /// Whether a SAT axis `n` (unit, A→B) with penetration `depth` separates the pair:
    /// `depth < −d_eff(n)`. `d_eff >= d`, so the velocity term is evaluated only for an axis
    /// already separating by `d`; with the term off this is `depth < −d`, a NaN depth `false`.
    #[inline]
    pub(crate) fn separates(&self, depth: f32, n: Vec3) -> bool {
        depth < -self.d && (!self.moving() || depth < -self.axis(n))
    }

    /// Whether a clipped point at `p` with separation `s` along the A→B normal `n` is kept:
    /// `s <= d_eff(p)` (the box-box sites, whose rule keeps an exact touch). With the term off
    /// this is `s <= d`; a NaN separation is dropped.
    #[inline]
    pub(crate) fn keeps(&self, s: f32, p: Vec3, n: Vec3) -> bool {
        s <= self.d || (self.moving() && s <= self.point(p, n))
    }

    /// [`keeps`](Self::keeps) for a vertex of a reference-face clip: `ref_normal` is the reference
    /// face's outward normal (toward the incident box) and `a_is_reference` whether body A owns
    /// that face, so the A→B normal is `ref_normal` or its negation. The orientation is resolved
    /// only when the velocity term is on.
    #[inline]
    pub(crate) fn keeps_on_face(&self, s: f32, p: Vec3, ref_normal: Vec3, a_is_reference: bool) -> bool {
        s <= self.d
            || (self.moving()
                && s <= self.point(p, if a_is_reference { ref_normal } else { ref_normal * -1.0 }))
    }

    /// Whether a point at `p` with separation `s` along the A→B normal `n` is dropped:
    /// `s >= d_eff(p)` (the sphere and SDF sites, whose rule drops an exact touch). With the term
    /// off this is `s >= d`.
    #[inline]
    pub(crate) fn drops(&self, s: f32, p: Vec3, n: Vec3) -> bool {
        s >= self.d && (!self.moving() || s >= self.point(p, n))
    }

    /// An upper bound on `d_eff` at every point of the pair, known before any normal:
    /// `d + min(cap, (|v_B − v_A| + |ω_A| R_A + |ω_B| R_B) · h)`, widened by [`REACH_SLACK`]
    /// against rounding. A site that must sample a normal before it can apply
    /// [`drops`](Self::drops) (the box-vs-SDF corners) skips a point past it without the sample;
    /// with the term off it is `d`.
    #[inline]
    pub(crate) fn reach(&self) -> f32 {
        if !self.moving() {
            return self.d;
        }
        let sweep = ((self.b.v - self.a.v).length() + self.spin) * self.h * REACH_SLACK;
        self.d + sweep.min(self.vcap)
    }
}

#[cfg(test)]
impl SpecMargin {
    /// A unit test's margin: `step`, and each body's `(centre, linear velocity, angular velocity,
    /// circumradius)`, A then B.
    pub(crate) fn for_test(step: SpecStep, a: (Vec3, Vec3, Vec3, f32), b: (Vec3, Vec3, Vec3, f32)) -> Self {
        if !step.moving() {
            return Self::fixed(step.d);
        }
        Self::with_motion(
            step,
            Motion { c: a.0, v: a.1, w: a.2 },
            a.3,
            Motion { c: b.0, v: b.1, w: b.2 },
            b.3,
        )
    }
}

#[cfg(test)]
mod tests {
    //! The margin's arithmetic (V2, ruling 9): the rule's form, the cap, the per-axis bound
    //! against the per-point value, `reach` against every point, and the overlap-only identity.

    use super::*;

    const D: f32 = 0.02;
    const H: f32 = 1.0 / 60.0;

    fn step() -> SpecStep {
        SpecStep::new(D, 0.5, H)
    }

    #[test]
    fn the_margin_is_d_plus_the_capped_approach_times_h() {
        // B closing on A at 3 m/s along +y (A below): approach 3, d_eff = d + 3 h.
        let b = (Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, -3.0, 0.0), Vec3::ZERO, 0.5);
        let a = (Vec3::ZERO, Vec3::ZERO, Vec3::ZERO, 0.5);
        let n = Vec3::new(0.0, 1.0, 0.0);
        let m = SpecMargin::for_test(step(), a, b);
        let want = D + 3.0 * H;
        assert_eq!(m.point(Vec3::new(0.0, 0.5, 0.0), n).to_bits(), want.to_bits());
        assert_eq!(m.axis(n).to_bits(), want.to_bits(), "no spin: the axis bound is the point value");
        // Receding: d alone.
        let away = SpecMargin::for_test(step(), a, (b.0, Vec3::new(0.0, 3.0, 0.0), b.2, b.3));
        assert_eq!(away.point(Vec3::new(0.0, 0.5, 0.0), n).to_bits(), D.to_bits());
        // The cap binds: 1 mm.
        let capped = SpecMargin::for_test(SpecStep::new(D, 0.001, H), a, b);
        assert_eq!(capped.point(Vec3::new(0.0, 0.5, 0.0), n).to_bits(), (D + 0.001).to_bits());
        assert_eq!(capped.bound().to_bits(), (D + 0.001).to_bits());
    }

    #[test]
    fn the_overlap_only_margin_answers_every_site_as_before_v2() {
        let m = SpecMargin::OVERLAP;
        for x in [-1.0f32, -0.0, 0.0, 1.0e-30, 0.5, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let p = Vec3::ZERO;
            let n = Vec3::new(0.0, 1.0, 0.0);
            assert_eq!(m.separates(x, n), x < 0.0, "separates({x})");
            assert_eq!(m.keeps(x, p, n), x <= 0.0, "keeps({x})");
            assert_eq!(m.keeps_on_face(x, p, n, false), x <= 0.0, "keeps_on_face({x})");
            assert_eq!(m.drops(x, p, n), x >= 0.0, "drops({x})");
        }
        assert_eq!(m.bound().to_bits(), 0.0f32.to_bits());
        assert_eq!(m.reach().to_bits(), 0.0f32.to_bits());
        let body = BodyState::default();
        assert_eq!(SpecStep::OVERLAP.bp_margin(&body).to_bits(), 0.0f32.to_bits());
        assert_eq!(SpecStep::fixed(D).bp_margin(&body).to_bits(), (0.5 * D).to_bits());
    }

    /// The broadphase margin: `d / 2 + min(cap, (|v| + |ω| R) · h)`, linear and angular, capped.
    #[test]
    fn the_broadphase_margin_is_half_d_plus_the_capped_sweep() {
        let mut body = BodyState {
            shape: ColliderShape::Box { half_extents: Vec3::new(0.3, 0.4, 1.2) },
            ..BodyState::default()
        };
        let r = Vec3::new(0.3, 0.4, 1.2).length();
        body.linear_velocity = Vec3::new(0.0, -3.0, 4.0);
        body.angular_velocity = Vec3::new(0.0, 0.0, 2.0);
        let want = 0.5 * D + (5.0 + 2.0 * r) * H;
        assert_eq!(step().bp_margin(&body).to_bits(), want.to_bits(), "linear and angular");
        let capped = SpecStep::new(D, 0.01, H);
        assert_eq!(capped.bp_margin(&body).to_bits(), (0.5 * D + 0.01).to_bits(), "capped");
        body.linear_velocity = Vec3::ZERO;
        assert_eq!(step().bp_margin(&body).to_bits(), (0.5 * D + 2.0 * r * H).to_bits(), "spin alone");
    }

    /// A spinning, moving pair: at every point within both bodies' circumradii (where a contact
    /// point of the pair lies), the per-axis bound is at least the per-point value along the axis,
    /// and `reach` at least the per-point value along any direction. A deterministic sweep, not a
    /// proof: it guards the bounds' form against an edit that drops a term.
    #[test]
    fn the_axis_bound_and_the_reach_cover_every_point() {
        let mut worst_axis = f32::INFINITY;
        let mut worst_reach = f32::INFINITY;
        for k in 0..400u32 {
            let t = k as f32 * 0.37;
            let ca = Vec3::new(t.sin(), 0.3 * t.cos(), -0.2);
            // Centres 0.2 apart; radii 0.9 and 0.7: a point within 0.45 of A's centre is within
            // 0.65 of B's, inside both.
            let cb = ca + Vec3::new((0.5 * t).cos(), (0.5 * t).sin(), 0.0) * 0.2;
            let a = (ca, Vec3::new(2.0 * (1.3 * t).cos(), -1.5, 0.7 * t.sin()), Vec3::new(3.0 * t.cos(), 5.0 * (0.7 * t).sin(), -2.0), 0.9);
            let b = (cb, Vec3::new(-t.sin(), -4.0 * (0.3 * t).cos(), 1.0), Vec3::new(-6.0, 2.0 * t.sin(), 4.0 * t.cos()), 0.7);
            let m = SpecMargin::for_test(SpecStep::new(D, 10.0, H), a, b);
            let n = Vec3::new((0.9 * t).cos(), (0.9 * t).sin(), 0.3).normalize();
            for j in 0..26u32 {
                let u = j as f32 * 0.7;
                let dir = Vec3::new(u.cos() * (1.3 * u).sin(), (1.3 * u).cos(), u.sin() * (1.3 * u).sin());
                let p = ca + dir * (0.45 * (j as f32 / 25.0));
                worst_axis = worst_axis.min(m.axis(n) - m.point(p, n));
                for q in [n, dir, Vec3::new(0.0, 0.0, 1.0)] {
                    if q.length_squared() > 0.0 {
                        worst_reach = worst_reach.min(m.reach() - m.point(p, q.normalize()));
                    }
                }
            }
        }
        assert!(worst_axis >= -1.0e-6, "the per-axis bound fell below a point's margin by {worst_axis}");
        assert!(worst_reach >= 0.0, "reach fell below a point's margin by {worst_reach}");
    }
}
