//! The in-house TGS-Soft rigid-body solver — sphere and box contacts with
//! cross-frame, per-contact-point warm-starting (P2 W2 + W3 + W4).
//!
//! [`SoftStepSolver`] is a real [`RigidSolver`] that resolves
//! the narrowphase manifolds with the Temporal Gauss-Seidel "Soft Step" scheme
//! (Box2D-v3 lineage): a velocity-level sequential-impulse solve with a soft
//! penetration-recovery bias, a 2-DOF coupled Coulomb friction cone, per-substep
//! integration, relaxation passes, and a single post-loop restitution pass. It
//! OWNS integration ([`owns_integration`](super::RigidSolver::owns_integration)
//! returns `true`), so the pipeline's
//! [`physics_integrate`](crate::systems::physics_integrate) is gated off (C2).
//!
//! # Warm-starting (W3 + W4 per-point)
//!
//! Sequential-impulse convergence is slow from a zero seed, so a stack jitters
//! apart under the per-step substep budget. The solver persists each contact
//! POINT's converged accumulated impulses (normal + 2 tangent) across frames via
//! the double-buffered [`WarmStartTable`]:
//!
//! 1. **Seed** (start of [`solve`](SoftStepSolver::solve)): each contact point's
//!    accumulated impulse is read from last frame's `read` table, keyed by THIS
//!    point's `pack(body_a, body_b, feature_id)` (zero on a miss). W4: every point
//!    of a multi-point box manifold seeds independently by its own feature id —
//!    the W3 wiring keyed the whole manifold by point 0 and left points 1..count
//!    permanently cold.
//! 2. **Apply** (per substep, after gravity, before the soft sweep, matching
//!    Box2D-v3 `b2WarmStartContactsTask`): the seeded impulse is applied to the
//!    bodies' velocities so the solve starts from the warm state. It re-applies
//!    each substep because each substep re-integrates gravity.
//! 3. **Store + swap** (end of `solve`): each point's converged impulse is
//!    inserted into a freshly-zeroed `write` table under its own key, in the
//!    flattened `(manifold order, point index)` order, then `read` ↔ `write`
//!    swap (C3 — rebuilt each frame, deterministic, no per-step alloc).
//!
//! A W3 sphere-sphere manifold has a single point with `feature_id == 0`, so its
//! per-point key equals the old per-manifold key — the sphere warm-start path is
//! byte-identical.
//!
//! # SDF contacts (W5 — the C1 sentinel path)
//!
//! An SDF-collision manifold keys `body_b == `[`SDF_SENTINEL`] (`u32::MAX`), NOT a
//! real dense row. The solver substitutes [`IMMOVABLE_AT_REST`] (`inv_mass = 0`,
//! `inv_inertia = ZERO`, velocity = 0) for body B and SKIPS every body-B impulse
//! apply — so it NEVER indexes `bodies[u32::MAX]` and never touches a non-existent
//! row. This rides the SAME one-sided `inv_mass == 0` impulse path a static rigid
//! floor exercises (no new branch class in the impulse math, only the body-fetch
//! picks the immovable surface), and the SDF warm-start key uses the dedicated
//! [`pack_sdf`](warm_start::pack_sdf) path so the sentinel `body_b` cannot corrupt
//! or alias a real body-body key.
//!
//! # Determinism (IM-2)
//!
//! Single-threaded over the deterministic manifold order (D4), fixed point order
//! `0..count`, normal-before-friction, fixed `substeps` / `relax_iterations`,
//! fixed float op order (no reduction reorder, no atomics, no rayon), warm-start
//! probe = pure function of key + a write table rebuilt each frame (C3). All
//! scratch buffers are capacity-reused — zero per-step allocation in steady
//! state. No `fast-math` / `float_algebraic` on this crate.

use boyko_ecs::ecs::core::component::scratch::ScratchColumn;
use boyko_macros::Resource as ResourceDerive;

use super::contact::{BodyEffective, effective_mass, tangent_basis};
use super::simd;
use super::warm_start::{self, WarmStartTable};
use super::RigidSolver;
use crate::components::BodyType;
use crate::manifold::{Manifold, SDF_SENTINEL};
use crate::math::{Mat3, Vec3};
use crate::resources::{BodyState, PhysicsConfig, SolverScratch};
use crate::scratch_ids::{body_eff_serial_id, register_scratch_layouts, scratch_reserve_rows};

/// Maximum penetration-recovery bias speed (world units/s) the soft normal solve
/// will inject, clamping the otherwise-unbounded `biasRate · separation` push so
/// a deep initial overlap cannot launch a body (Box2D's `maxBiasVelocity`).
///
/// `pub(crate)` so the colored solver ([`super::colored`]) reads the SAME source
/// (O2 — no copy-duplicated soft constant can drift); the value is unchanged.
pub(crate) const MAX_BIAS_VELOCITY: f32 = 4.0;

/// Minimum approach speed (world units/s) a contact must carry at gather time for
/// the post-loop restitution pass to bounce it (Box2D-v3's `b2_velocityThreshold`).
///
/// A body in SUSTAINED contact under gravity carries a small residual closing
/// velocity in the gather snapshot every frame (gravity re-adds `g·h·substeps`;
/// the soft solve leaves a residual), so without a threshold `v_target =
/// -e·vn_initial > 0` would inject energy every frame and a resting stack with
/// `restitution > 0` would jitter/creep upward. Contacts approaching slower than
/// this threshold are treated as resting (effectively `e = 0`); only a genuine
/// impact above it bounces. `1.0 m/s` is Box2D's meter-scale default — comfortably
/// above the per-frame gravity-residual closing speed (`|gravity|·dt`), well below
/// a real collision.
///
/// `pub(crate)` so the colored solver reads the SAME source (O2); value unchanged.
pub(crate) const RESTITUTION_THRESHOLD: f32 = 1.0;

/// Per-contact-point constraint scratch built once per solve and re-read each
/// substep (P2 W2 + W3).
///
/// Holds the precomputed geometry (anchors relative to each body's center of
/// mass, the friction tangent basis) and the accumulated impulses (warm-SEEDED
/// from last frame's converged value in W3, zero on a cache miss). One
/// [`PointConstraint`] per live manifold point, in the flattened
/// manifold-order × point-order sequence — the same order
/// [`SolverScratch::vn_initial`](crate::resources::SolverScratch) is indexed in.
#[derive(Clone, Copy, Debug, Default)]
struct PointConstraint {
    /// Anchor offset on body A from its center of mass (world frame).
    ra: Vec3,
    /// Anchor offset on body B from its center of mass (world frame).
    rb: Vec3,
    /// Signed separation at gather time (negative = penetrating); the soft bias
    /// drives this toward zero.
    separation: f32,
    /// Accumulated normal impulse `λn ≥ 0` (W3 warm-seeds it from last frame).
    normal_impulse: f32,
    /// Accumulated tangent impulse along `t1` (W3 warm-seeds it from last frame).
    tangent_impulse1: f32,
    /// Accumulated tangent impulse along `t2` (W3 warm-seeds it from last frame).
    tangent_impulse2: f32,
    /// This point's own warm-start key (W4): `pack(body_a, body_b, feature_id)`
    /// with THIS point's `feature_id`. Each contact point warm-starts
    /// independently, so a box manifold's 4 points each carry / persist their own
    /// converged impulse (W3 sphere-sphere has one point with `feature_id == 0`,
    /// so its key equals the W3 per-manifold key — byte-identical behavior).
    warm_key: u64,
}

/// An immovable, at-rest body view (P2 W5 / C1) — the body B every SDF-collision
/// manifold solves against.
///
/// `inv_mass == 0` + `inv_inertia == Mat3::ZERO` + zero velocity make every
/// [`apply_impulse`](BodyEffective::apply_impulse) on it a no-op and its
/// [`point_velocity`](BodyEffective::point_velocity) zero, so it rides the SAME
/// one-sided `inv_mass == 0` impulse path a static rigid floor exercises — the SDF
/// surface acts as an immovable wall with NO new branch class in the impulse math.
/// The solver substitutes this for body B whenever a manifold's `body_b` is the
/// [`SDF_SENTINEL`], so it NEVER indexes `bodies[u32::MAX]`.
///
/// `pub(crate)` so the colored solver reads the SAME immovable surface view (O2);
/// the layout and field values are unchanged.
pub(crate) const IMMOVABLE_AT_REST: BodyEffective = BodyEffective {
    inv_mass: 0.0,
    inv_inertia: Mat3::ZERO,
    linear_velocity: Vec3::ZERO,
    angular_velocity: Vec3::ZERO,
};

/// One manifold's solver state — the body-row indices, the contact frame, and
/// the span of its points in the flattened [`SoftStepSolver::points`] buffer
/// (P2 W2).
#[derive(Clone, Copy, Debug, Default)]
struct ManifoldConstraint {
    /// Dense row index of body A.
    ia: usize,
    /// Dense row index of body B, OR — when [`b_is_sentinel`](Self::b_is_sentinel)
    /// is set — an UNUSED placeholder (the [`SDF_SENTINEL`] row `u32::MAX` is never
    /// indexed; body B is [`IMMOVABLE_AT_REST`] instead).
    ib: usize,
    /// `true` when this is an SDF-collision manifold (`body_b == SDF_SENTINEL`):
    /// body B is [`IMMOVABLE_AT_REST`], not `bodies[ib]` — the C1 sentinel guard
    /// keeps the solver from indexing `bodies[u32::MAX]` (out of bounds) and from
    /// ever touching a non-existent row.
    b_is_sentinel: bool,
    /// Unit contact normal (points from A toward B).
    normal: Vec3,
    /// First friction tangent (unit, `⟂ normal`).
    tangent1: Vec3,
    /// Second friction tangent (unit, `normal × tangent1`).
    tangent2: Vec3,
    /// Index of this manifold's first point in [`SoftStepSolver::points`].
    point_start: usize,
    /// Number of live points (`points[point_start .. point_start + count]`).
    count: usize,
}

/// The in-house TGS-Soft rigid-body solver (P2 W2 + W3 warm-starting).
///
/// A `Resource` carrying the reused, capacity-preserving per-solve scratch plus
/// the double-buffered W3 warm-start cache (the `read`/`write`
/// [`WarmStartTable`]s). [`is_noop`](RigidSolver::is_noop) is `false` and
/// [`owns_integration`](RigidSolver::owns_integration) is `true`, so the pipeline
/// gates off its own integrate stage (C2) and this solver integrates DYNAMIC
/// bodies inside its substep loop.
#[derive(ResourceDerive)]
pub struct SoftStepSolver {
    /// Per-body solver view, parallel to `scratch.bodies` — refreshed each
    /// substep so the world inverse inertia tracks the advancing orientation.
    /// Backed by a [`ScratchColumn`] (engine-owned, address-stable) instead of a
    /// `std::Vec` parallel-data-system (audit Stage P). This is the SERIAL path —
    /// every access is single-threaded through the build view's `as_mut_slice`.
    bodies: ScratchColumn<BodyEffective>,
    /// Per-manifold constraint state, in deterministic manifold order.
    manifolds: Vec<ManifoldConstraint>,
    /// Flattened per-point constraint state, indexed by `manifold.point_start +
    /// p` (the same order `scratch.vn_initial` uses).
    points: Vec<PointConstraint>,
    /// Last frame's converged impulses (W3) — probed to seed this frame's
    /// contacts at the start of [`solve`](Self::solve).
    warm_read: WarmStartTable,
    /// This frame's converged impulses (W3) — freshly zeroed each frame, filled
    /// in manifold order after the solve, then swapped into `warm_read`.
    warm_write: WarmStartTable,
    /// Whether warm-starting is active (W3). Production default is `true`; the
    /// `false` mode (see [`with_warm_start`](Self::with_warm_start)) zero-seeds
    /// every contact each frame, used by the A/B convergence test to demonstrate
    /// the warm-start payoff.
    warm_start_enabled: bool,
}

impl Default for SoftStepSolver {
    /// The production default — empty scratch, warm-starting ON.
    #[inline]
    fn default() -> Self {
        Self::with_capacity(0, 0)
    }
}

impl SoftStepSolver {
    /// Builds a solver with the scratch buffers pre-sized for up to `bodies`
    /// rows and `contacts` contact points (no later realloc in steady state),
    /// warm-starting ON.
    pub fn with_capacity(bodies: usize, contacts: usize) -> Self {
        register_scratch_layouts();
        let reserve = bodies.max(scratch_reserve_rows(size_of::<BodyEffective>()));
        Self {
            bodies: ScratchColumn::new(body_eff_serial_id(), reserve),
            manifolds: Vec::with_capacity(contacts),
            points: Vec::with_capacity(contacts),
            warm_read: WarmStartTable::with_capacity(contacts),
            warm_write: WarmStartTable::with_capacity(contacts),
            warm_start_enabled: true,
        }
    }

    /// Builds a solver with warm-starting toggled `enabled` (P2 W3, test hook).
    ///
    /// Production code uses the warm-starting-ON [`Default`] / [`Self::with_capacity`];
    /// passing `false` zero-seeds every contact each frame (the W2 behavior),
    /// which the `warm_start_improves_convergence` A/B test runs against to show
    /// the payoff. Pre-sizes nothing (the steady-state capacity grows on the
    /// first solve).
    pub fn with_warm_start(enabled: bool) -> Self {
        Self {
            warm_start_enabled: enabled,
            ..Self::with_capacity(0, 0)
        }
    }

    /// Rebuilds the per-body solver views from the gather snapshot.
    ///
    /// `inv_inertia` starts as the gather's world tensor; the substep loop
    /// refreshes it from `inv_inertia_local` + the advancing orientation.
    fn build_bodies(&mut self, bodies: &[BodyState]) {
        let mut view = self.bodies.build_view();
        view.clear();
        for b in bodies {
            view.push(BodyEffective {
                inv_mass: b.inv_mass,
                inv_inertia: b.inv_inertia,
                linear_velocity: b.linear_velocity,
                angular_velocity: b.angular_velocity,
            });
        }
    }

    /// Builds the per-manifold + per-point constraint scratch, SEEDS each
    /// contact's accumulated impulse from the warm-start `read` table (W3), and
    /// captures the initial relative normal approach velocity into `vn_initial`
    /// (for the post-loop restitution pass).
    ///
    /// Anchors are taken relative to each body's CURRENT center of mass (the
    /// gather position); the soft solve re-uses them across substeps (it does not
    /// re-run narrowphase). Tangent bases are degeneracy-safe (`tangent_basis`).
    ///
    /// The warm seed (W4 — per-point): each contact POINT is keyed by
    /// `pack(body_a, body_b, point.feature_id)` — its OWN feature id, not the
    /// manifold's point-0 id. A `read`-table hit seeds that point's accumulated
    /// normal + tangent impulses with last frame's converged value; a miss (a new
    /// or just-reformed point — e.g. a box manifold point whose feature id flipped)
    /// seeds zero, a one-frame convergence cost, no error. A box manifold's 4
    /// points therefore warm-start independently (the W3 limitation that left
    /// points 1..count always cold). When `warm_start_enabled` is `false` (the A/B
    /// test hook) every seed is zero (the W2 behavior). A W3 sphere-sphere manifold
    /// has one point with `feature_id == 0`, so its key equals the old per-manifold
    /// key — the sphere path is byte-identical.
    fn build_constraints(
        &mut self,
        manifolds: &[Manifold],
        bodies: &[BodyState],
        vn_initial: &mut Vec<f32>,
    ) {
        // Disjoint-field borrows: the BodyEffective read slice (built by
        // `build_bodies` just above) is read while `self.manifolds` / `self.points`
        // are written. `bodies_eff` is the read view of the solver's body column.
        let Self {
            bodies: body_col,
            manifolds: out_manifolds,
            points: out_points,
            warm_read,
            warm_start_enabled,
            ..
        } = self;
        let bodies_eff = body_col.as_read_slice();
        out_manifolds.clear();
        out_points.clear();
        vn_initial.clear();

        for m in manifolds {
            let count = m.count as usize;
            if count == 0 {
                continue;
            }
            let ia = m.body_a.0 as usize;
            // C1: an SDF-collision manifold keys `body_b == SDF_SENTINEL`
            // (`u32::MAX`) — NOT a real row. Body B is `IMMOVABLE_AT_REST`, so we
            // must never index `bodies[u32::MAX]`; `ib` is left at `ia` purely as a
            // harmless in-range placeholder (never read for the sentinel side).
            let b_is_sentinel = m.body_b == SDF_SENTINEL;
            let ib = if b_is_sentinel { ia } else { m.body_b.0 as usize };
            let normal = m.normal;
            let (tangent1, tangent2) = tangent_basis(normal);
            let point_start = out_points.len();

            let pa = bodies[ia].position;
            // The sentinel surface has no position; anchors are expressed relative
            // to it directly (anchor_b is the surface contact point itself, so
            // `rb == 0` — an immovable B with a zero lever arm, the correct
            // one-sided contact frame).
            let pb = if b_is_sentinel {
                Vec3::ZERO
            } else {
                bodies[ib].position
            };

            for p in 0..count {
                let cp = &m.points[p];
                let ra = cp.anchor_a - pa;
                let rb = if b_is_sentinel {
                    // Immovable surface: a zero lever arm (it never moves or spins).
                    Vec3::ZERO
                } else {
                    cp.anchor_b - pb
                };
                // Relative normal velocity of the contact point, B relative to A,
                // projected on the normal. Negative = approaching (the bodies
                // are closing) — the value restitution bounces back. The sentinel
                // B is at rest (`IMMOVABLE_AT_REST`), contributing zero.
                let dv = {
                    let ba = &bodies_eff[ia];
                    let bb = if b_is_sentinel {
                        &IMMOVABLE_AT_REST
                    } else {
                        &bodies_eff[ib]
                    };
                    bb.point_velocity(rb) - ba.point_velocity(ra)
                };
                vn_initial.push(dv.dot(normal));
                // W4 per-point warm key: this point's OWN feature id. Each point
                // probes the `read` table independently, so a box manifold's 4
                // points each seed from their own last-frame converged impulse. C1:
                // an SDF contact uses the dedicated `pack_sdf` key path so the
                // `u32::MAX` sentinel `body_b` never trips the 24-bit field / aliases
                // a real pair (it maps to the reserved all-ones body_b tag).
                let warm_key = if b_is_sentinel {
                    warm_start::pack_sdf(m.body_a, cp.feature_id)
                } else {
                    warm_start::pack(m.body_a, m.body_b, cp.feature_id)
                };
                let seed = if *warm_start_enabled {
                    warm_read.get(warm_key)
                } else {
                    None
                };
                // Warm-seed the accumulated impulses (zero on miss / disabled).
                let (normal_impulse, tangent_impulse1, tangent_impulse2) = match seed {
                    Some(e) => (e.normal_impulse, e.tangent_impulse[0], e.tangent_impulse[1]),
                    None => (0.0, 0.0, 0.0),
                };
                out_points.push(PointConstraint {
                    ra,
                    rb,
                    separation: cp.separation,
                    normal_impulse,
                    tangent_impulse1,
                    tangent_impulse2,
                    warm_key,
                });
            }

            out_manifolds.push(ManifoldConstraint {
                ia,
                ib,
                b_is_sentinel,
                normal,
                tangent1,
                tangent2,
                point_start,
                count,
            });
        }
    }

    /// Applies the seeded accumulated impulse of every contact point to both
    /// bodies' velocities (the W3 warm-start apply, Algorithm step 2).
    ///
    /// For each point the total impulse is `P = λn·n + λt1·t1 + λt2·t2`; it is
    /// applied as `v ∓= invMass·P`, `ω ∓= I⁻¹·(r×P)` (A gets `-P`, B gets `+P`,
    /// matching the solve's sign convention). Run once per substep BEFORE the
    /// soft sweep (after the per-substep gravity integrate) — matching Box2D-v3's
    /// `b2WarmStartContactsTask`. The accumulated impulse persists across
    /// substeps in `points[]`; re-applying each substep restores the warm
    /// velocity after each substep re-integrates gravity, so the soft sweep
    /// always refines from the warm state rather than from the cold post-gravity
    /// velocity. A zero-seeded (missed / disabled) contact applies a zero impulse
    /// — a branchless no-op.
    //
    // `clippy::needless_range_loop`: `p` is the flattened contact-point key into
    // `points`; the body also indexes the disjoint `bodies_eff[mc.ia/ib]`, which a
    // single `iter_mut` cannot express (the same three-buffer pattern as
    // `solve_velocities`).
    #[allow(clippy::needless_range_loop)]
    fn warm_start_apply(
        manifolds: &[ManifoldConstraint],
        points: &[PointConstraint],
        bodies_eff: &mut [BodyEffective],
    ) {
        for mc in manifolds {
            let normal = mc.normal;
            let (t1, t2) = (mc.tangent1, mc.tangent2);
            for p in mc.point_start..(mc.point_start + mc.count) {
                let pc = points[p];
                let impulse =
                    normal * pc.normal_impulse + t1 * pc.tangent_impulse1 + t2 * pc.tangent_impulse2;
                bodies_eff[mc.ia].apply_impulse(pc.ra, impulse * -1.0);
                // C1: for an SDF contact body B is `IMMOVABLE_AT_REST` (the impulse
                // is a no-op on it); crucially `mc.ib` is the A-row placeholder for
                // the sentinel, so applying to it would DOUBLE-apply to A — guard it.
                if !mc.b_is_sentinel {
                    bodies_eff[mc.ib].apply_impulse(pc.rb, impulse);
                }
            }
        }
    }

    /// Stores every contact POINT's converged accumulated impulses into the
    /// freshly rebuilt `write` table, then swaps `read` ↔ `write` (the W3/W4
    /// store, C3).
    ///
    /// [`rebuild`](WarmStartTable::rebuild) zeroes the `write` table sized for the
    /// live contact-POINT count, then each point's converged impulse is inserted
    /// under its OWN `warm_key` (W4 — per point, not per manifold) in the
    /// deterministic flattened order `points[]` already holds (manifold order,
    /// then point index `0..count`). The resulting occupancy is a pure function of
    /// this frame's key set (order-independent, no carried history), so the
    /// swapped-in `read` table is bit-deterministic next frame. When warm-starting
    /// is disabled the store is skipped (the `read` table stays empty, so every
    /// seed misses).
    fn store_and_swap(&mut self) {
        if !self.warm_start_enabled {
            return;
        }
        let point_count = self.points.len();
        self.warm_write.rebuild(point_count);
        // Each point persists independently under its own per-point key, in the
        // flattened `(manifold order, point index)` order — the deterministic C3
        // insertion order. A box manifold's 4 points therefore each carry their
        // converged impulse to next frame.
        for pc in &self.points {
            self.warm_write.insert(
                pc.warm_key,
                pc.normal_impulse,
                [pc.tangent_impulse1, pc.tangent_impulse2],
            );
        }
        core::mem::swap(&mut self.warm_read, &mut self.warm_write);
    }

    /// Refreshes each dynamic body's world inverse inertia from its local tensor
    /// and current orientation (`R · I⁻¹_local · Rᵀ`) for the next substep's
    /// effective mass. Static bodies keep `Mat3::ZERO`.
    ///
    /// Delegates to [`simd::refresh_inertia`], which runs the O1 AVX2 width-only
    /// kernel when `simd` is set on an AVX2 build (bit-identical to the scalar
    /// path) and the scalar oracle otherwise — the scalar path stays the default
    /// + the bit-oracle (the 0%-gate).
    #[inline]
    fn refresh_inertia(bodies_eff: &mut [BodyEffective], snapshot: &[BodyState], simd: bool) {
        simd::refresh_inertia(bodies_eff, snapshot, simd);
    }

    /// Solves the normal + coupled-friction impulses for every contact point
    /// once (one Gauss-Seidel sweep), using the supplied soft coefficients.
    ///
    /// `bias_active` selects the recovery bias: `true` during the main substep
    /// (the soft penetration push) and `false` during relaxation (bias-free,
    /// energy-removing). The friction cone clamps the ACCUMULATED 2-vector
    /// tangent impulse magnitude to `μ · λn` (a cone, not two box clamps).
    //
    // `clippy::needless_range_loop`: the index `p` is the flattened contact-point
    // key into `points`, and the loop body also indexes `bodies_eff[mc.ia]` /
    // `bodies_eff[mc.ib]` (disjoint buffers) — a single `iter_mut` cannot express
    // the three-buffer Gauss-Seidel read/apply, so the explicit index is correct.
    #[allow(clippy::needless_range_loop)]
    fn solve_velocities(
        manifolds: &[ManifoldConstraint],
        points: &mut [PointConstraint],
        bodies_eff: &mut [BodyEffective],
        snapshot: &[BodyState],
        soft: SoftCoefficients,
        bias_active: bool,
    ) {
        for mc in manifolds {
            let normal = mc.normal;
            let (t1, t2) = (mc.tangent1, mc.tangent2);
            // Combined friction coefficient. The foundation stores friction per
            // body; W2 combines by `max` (a simple, symmetric, deterministic rule
            // — a sliding pair is as sticky as its stickiest surface). A
            // geometric-mean (`sqrt(µA·µB)`) combine is W4 polish. C1: an SDF
            // surface has no friction material — `mc.ib` is the A-row placeholder
            // for the sentinel, so this resolves to A's own friction (the field is
            // treated as carrying the body's friction, a deterministic convention).
            let friction = snapshot[mc.ia].friction.max(snapshot[mc.ib].friction);

            for p in mc.point_start..(mc.point_start + mc.count) {
                let pc = points[p];
                let (ra, rb) = (pc.ra, pc.rb);

                // ── Normal solve ────────────────────────────────────────────
                let m_eff = {
                    let ba = bodies_eff[mc.ia];
                    // C1: body B is `IMMOVABLE_AT_REST` for an SDF contact (never
                    // `bodies_eff[u32::MAX]`); it contributes 0 to the effective mass.
                    let bb = if mc.b_is_sentinel {
                        IMMOVABLE_AT_REST
                    } else {
                        bodies_eff[mc.ib]
                    };
                    effective_mass(normal, ra, rb, &ba, &bb)
                };
                let vn = {
                    let ba = &bodies_eff[mc.ia];
                    let bb = if mc.b_is_sentinel {
                        IMMOVABLE_AT_REST
                    } else {
                        bodies_eff[mc.ib]
                    };
                    (bb.point_velocity(rb) - ba.point_velocity(ra)).dot(normal)
                };
                // Soft bias drives the penetration toward zero; clamp the push so
                // a deep overlap cannot launch the body.
                let bias = if bias_active {
                    (soft.bias_rate * pc.separation).max(-MAX_BIAS_VELOCITY)
                } else {
                    0.0
                };
                // dλ = -massCoeff · mEff · (vn + bias) − impulseCoeff · λ
                let d_lambda = if bias_active {
                    -soft.mass_coeff * m_eff * (vn + bias) - soft.impulse_coeff * pc.normal_impulse
                } else {
                    // Relaxation: rigid (no soft mass/impulse scaling, no bias).
                    -m_eff * vn
                };
                let new_lambda = (pc.normal_impulse + d_lambda).max(0.0);
                let applied_n = new_lambda - pc.normal_impulse;
                points[p].normal_impulse = new_lambda;
                {
                    let impulse = normal * applied_n;
                    bodies_eff[mc.ia].apply_impulse(ra, impulse * -1.0);
                    // C1: skip the immovable sentinel B (also avoids double-applying
                    // to A, since `mc.ib` is the A-row placeholder for the sentinel).
                    if !mc.b_is_sentinel {
                        bodies_eff[mc.ib].apply_impulse(rb, impulse);
                    }
                }

                // ── Friction solve (2-DOF coupled cone) ─────────────────────
                let max_friction = friction * points[p].normal_impulse;
                let m_eff_t1 = {
                    let ba = bodies_eff[mc.ia];
                    let bb = if mc.b_is_sentinel {
                        IMMOVABLE_AT_REST
                    } else {
                        bodies_eff[mc.ib]
                    };
                    effective_mass(t1, ra, rb, &ba, &bb)
                };
                let m_eff_t2 = {
                    let ba = bodies_eff[mc.ia];
                    let bb = if mc.b_is_sentinel {
                        IMMOVABLE_AT_REST
                    } else {
                        bodies_eff[mc.ib]
                    };
                    effective_mass(t2, ra, rb, &ba, &bb)
                };
                let (vt1, vt2) = {
                    let ba = &bodies_eff[mc.ia];
                    let bb = if mc.b_is_sentinel {
                        IMMOVABLE_AT_REST
                    } else {
                        bodies_eff[mc.ib]
                    };
                    let dv = bb.point_velocity(rb) - ba.point_velocity(ra);
                    (dv.dot(t1), dv.dot(t2))
                };
                // Tentative new accumulated tangent impulse, then clamp the 2D
                // magnitude to the cone `|λt| ≤ μ·λn` (NOT two box clamps).
                let mut new_t1 = points[p].tangent_impulse1 - m_eff_t1 * vt1;
                let mut new_t2 = points[p].tangent_impulse2 - m_eff_t2 * vt2;
                let len_sq = new_t1 * new_t1 + new_t2 * new_t2;
                if len_sq > max_friction * max_friction && len_sq > 0.0 {
                    let scale = max_friction / len_sq.sqrt();
                    new_t1 *= scale;
                    new_t2 *= scale;
                }
                let applied_t1 = new_t1 - points[p].tangent_impulse1;
                let applied_t2 = new_t2 - points[p].tangent_impulse2;
                points[p].tangent_impulse1 = new_t1;
                points[p].tangent_impulse2 = new_t2;
                {
                    let impulse = t1 * applied_t1 + t2 * applied_t2;
                    bodies_eff[mc.ia].apply_impulse(ra, impulse * -1.0);
                    // C1: skip the immovable sentinel B (and the A-row placeholder).
                    if !mc.b_is_sentinel {
                        bodies_eff[mc.ib].apply_impulse(rb, impulse);
                    }
                }
            }
        }
    }

    /// The post-loop restitution pass (P2 W2) — velocity-only, bias-free, run
    /// ONCE after the substeps.
    ///
    /// For each contact point whose gather-time approach speed exceeds
    /// [`RESTITUTION_THRESHOLD`] (`vn_initial < -RESTITUTION_THRESHOLD`) it drives
    /// the current relative normal velocity up to the target
    /// `v_target = -e · vn_initial`, keeping the total normal impulse `λn ≥ 0`.
    /// Separating or slowly-approaching (resting) contacts are skipped, so a stack
    /// settled under gravity does not re-bounce every frame (C1). No position is
    /// written. Zero-normal contacts (none in W2 sphere-sphere) are skipped by the
    /// manifold build.
    ///
    /// W3 forward risk: this single bias-free sweep is EXACT for single-point
    /// manifolds (W2 sphere-sphere is always one point), because there is no
    /// cross-point coupling to converge. A 4-point box manifold (W4) couples the
    /// points through the shared body, so a single sweep under-resolves the corner
    /// velocities — W4 must revisit this with a small iteration loop.
    //
    // `clippy::needless_range_loop`: `p` indexes both `points[p]` and the
    // parallel `vn_initial[p]`, plus the loop applies to `bodies_eff[mc.ia/ib]`;
    // the explicit flattened index is the constraint-point key, not a position.
    #[allow(clippy::needless_range_loop)]
    fn apply_restitution(
        manifolds: &[ManifoldConstraint],
        points: &mut [PointConstraint],
        bodies_eff: &mut [BodyEffective],
        snapshot: &[BodyState],
        vn_initial: &[f32],
    ) {
        for mc in manifolds {
            let normal = mc.normal;
            // C1: an SDF surface has no restitution material — `mc.ib` is the A-row
            // placeholder for the sentinel, so this resolves to A's own restitution.
            let restitution = snapshot[mc.ia].restitution.max(snapshot[mc.ib].restitution);
            if restitution <= 0.0 {
                continue;
            }
            for p in mc.point_start..(mc.point_start + mc.count) {
                let vn0 = vn_initial[p];
                // Only a contact APPROACHING above the velocity threshold bounces.
                // Separating (`vn0 >= 0`) and resting / slowly-closing contacts
                // (`vn0 > -RESTITUTION_THRESHOLD`) are skipped — the latter guards a
                // gravity-loaded stack from re-bouncing every frame (C1).
                if vn0 > -RESTITUTION_THRESHOLD {
                    continue;
                }
                let pc = points[p];
                let (ra, rb) = (pc.ra, pc.rb);
                let m_eff = {
                    let ba = bodies_eff[mc.ia];
                    // C1: immovable sentinel B (never `bodies_eff[u32::MAX]`).
                    let bb = if mc.b_is_sentinel {
                        IMMOVABLE_AT_REST
                    } else {
                        bodies_eff[mc.ib]
                    };
                    effective_mass(normal, ra, rb, &ba, &bb)
                };
                let vn = {
                    let ba = &bodies_eff[mc.ia];
                    let bb = if mc.b_is_sentinel {
                        IMMOVABLE_AT_REST
                    } else {
                        bodies_eff[mc.ib]
                    };
                    (bb.point_velocity(rb) - ba.point_velocity(ra)).dot(normal)
                };
                // Target separating velocity = -e · approach speed.
                let v_target = -restitution * vn0;
                let d_lambda = m_eff * (v_target - vn);
                let new_lambda = (pc.normal_impulse + d_lambda).max(0.0);
                let applied = new_lambda - pc.normal_impulse;
                points[p].normal_impulse = new_lambda;
                let impulse = normal * applied;
                bodies_eff[mc.ia].apply_impulse(ra, impulse * -1.0);
                // C1: skip the immovable sentinel B (and the A-row placeholder).
                if !mc.b_is_sentinel {
                    bodies_eff[mc.ib].apply_impulse(rb, impulse);
                }
            }
        }
    }

    /// Writes the solved velocities back into the gather snapshot and flags every
    /// DYNAMIC body the solver integrated as touched (so
    /// [`physics_apply`](crate::systems::physics_apply) writes those rows back).
    ///
    /// In solver-owned mode (C2) this solver is the SOLE integrator, so EVERY
    /// dynamic body — contacting or free-falling — must be written back and
    /// touched: a free body's new gravity-applied velocity lives in `self.bodies`
    /// and its new position was integrated in place into `scratch.bodies`, but
    /// `physics_apply` only writes rows whose touched bit is set. Without touching
    /// the free body it would be integrated in scratch yet never written to the
    /// `RigidBody` column — appearing frozen. Static / `inv_mass == 0` rows are
    /// left UNtouched (so they are not written back and cannot drift; the
    /// `static_body_unmoved` C2 guard depends on this), matching the integrate gate.
    //
    // `clippy::needless_range_loop`: `row` indexes the disjoint `eff` (read) AND the
    // `scratch` snapshot via `write_body` — a single `iter` cannot express the two-
    // buffer pattern (the same idiom the build/solve loops use).
    #[allow(clippy::needless_range_loop)]
    fn write_back(&self, scratch: &mut SolverScratch) {
        let eff = self.bodies.as_read_slice();
        let n = eff.len();
        for row in 0..n {
            // Touch exactly the rows the substep loop integrated (the same
            // `Dynamic && inv_mass != 0` gate), so a free body's integrated state
            // is written back and a static/kinematic row stays bit-identical.
            // The snapshot read + write borrows are disjoint from `eff` (distinct
            // ScratchColumns), bridged through the per-row helper.
            let is_dynamic = scratch.bodies()[row].body_type == BodyType::Dynamic;
            if is_dynamic && eff[row].inv_mass != 0.0 {
                Self::write_body(scratch, row, &eff[row]);
            }
        }
    }

    /// Copies one solved body view's velocity (position/orientation were
    /// integrated into the snapshot in place) back and marks the row touched.
    fn write_body(scratch: &mut SolverScratch, row: usize, eff: &BodyEffective) {
        {
            let mut view = scratch.bodies.build_view();
            let snapshot = view.as_mut_slice();
            snapshot[row].linear_velocity = eff.linear_velocity;
            snapshot[row].angular_velocity = eff.angular_velocity;
        }
        scratch.touched.set(row);
    }
}

/// The TGS-Soft constraint coefficients derived from `contact_hertz`,
/// `contact_damping` (ζ), and the substep `h` (P2 W2, Box2D-v3 "Soft Step").
///
/// `omega = 2π·hertz; a1 = 2·ζ + omega·h; a2 = h·omega·a1; a3 = 1/(1+a2);`
/// `biasRate = omega/a1; massCoeff = a2·a3; impulseCoeff = a3`.
///
/// `pub(crate)` (type + fields + `new`) so the colored solver derives its soft
/// terms from the SAME source — O2: the colored kernel cannot drift from this
/// reference derivation. Values and layout are unchanged.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SoftCoefficients {
    /// Penetration-recovery bias rate (`omega / a1`): the per-unit-separation
    /// recovery speed.
    pub(crate) bias_rate: f32,
    /// Soft mass scale (`a2 · a3`) applied to the rigid `mEff · (vn + bias)`.
    pub(crate) mass_coeff: f32,
    /// Accumulated-impulse decay (`a3`) — the soft term that pulls the impulse
    /// toward the spring's steady state.
    pub(crate) impulse_coeff: f32,
}

impl SoftCoefficients {
    /// Derives the coefficients for hertz `hertz`, damping ratio `zeta`, and
    /// substep `h`.
    #[inline]
    pub(crate) fn new(hertz: f32, zeta: f32, h: f32) -> Self {
        let omega = 2.0 * core::f32::consts::PI * hertz;
        let a1 = 2.0 * zeta + omega * h;
        let a2 = h * omega * a1;
        let a3 = 1.0 / (1.0 + a2);
        Self {
            bias_rate: omega / a1,
            mass_coeff: a2 * a3,
            impulse_coeff: a3,
        }
    }
}

impl RigidSolver for SoftStepSolver {
    /// Resolves all sphere-sphere contacts for one step with the TGS-Soft scheme,
    /// integrating DYNAMIC bodies inside its substep loop (C2) and flagging every
    /// integrated DYNAMIC row touched (contacting or free-falling), so a body with
    /// no current contact still falls under gravity and is written back.
    fn solve(
        &mut self,
        config: &PhysicsConfig,
        manifolds: &[Manifold],
        scratch: &mut SolverScratch,
    ) {
        let substeps = config.substeps.max(1);
        let h = config.dt / substeps as f32;

        // Build the per-body views and per-contact constraints over the gather
        // snapshot; `vn_initial` captures the pre-substep approach velocity for
        // the restitution pass.
        self.build_bodies(scratch.bodies());
        // Split the scratch borrow: the snapshot positions feed the constraint
        // build while `vn_initial` is filled. Both columns are addressed by
        // distinct ScratchColumns, so the body-read slice and `vn_initial` are
        // disjoint borrows.
        {
            // Disjoint field borrows of `scratch`: the BodyState snapshot read
            // slice feeds the constraint build while `vn_initial` is filled.
            let SolverScratch {
                bodies: body_col,
                vn_initial,
                ..
            } = &mut *scratch;
            self.build_constraints(manifolds, body_col.as_read_slice(), vn_initial);
        }
        // No `manifolds.is_empty()` early-return: in solver-owned mode (C2) this
        // solver is the SOLE integrator, so the substep loop must run its gravity
        // (step 1) + position (step 5) integration for every dynamic body EVERY
        // step, even with zero contacts — a free body must keep falling. With no
        // manifolds the contact-solve / friction / relax / restitution sweeps
        // iterate an empty manifold list and naturally do nothing. The only valid
        // skip is a world with no dynamic body to integrate at all (then there is
        // nothing to integrate and nothing to write back).
        let has_dynamic = scratch
            .bodies()
            .iter()
            .any(|b| b.body_type == BodyType::Dynamic && b.inv_mass != 0.0);
        if !has_dynamic {
            return;
        }

        let soft = SoftCoefficients::new(config.contact_hertz, config.contact_damping, h);
        let gravity = config.gravity;

        let use_simd = config.simd;

        // Disjoint-field borrows for the serial substep loop: `bodies_eff` is the
        // single-threaded mutable slice over the solver's BodyEffective column (no
        // parallel access — this is the SERIAL solver), while `manifolds` / `points`
        // / the warm tables stay borrowable through the destructured fields.
        let Self {
            bodies,
            manifolds: mc,
            points,
            ..
        } = self;
        let mut bodies_view = bodies.build_view();
        let bodies_eff = bodies_view.as_mut_slice();

        for _ in 0..substeps {
            // (1) Gravity integrate DYNAMIC bodies only (C2 gate (2)). O1: the AVX2
            // SoA kernel when `simd` is on (bit-identical to the scalar oracle).
            simd::apply_gravity(bodies_eff, scratch.bodies(), gravity, h, use_simd);

            // (2) Warm-start apply (W3): re-apply the seeded accumulated impulse
            // to the post-gravity velocities so the soft sweep refines from the
            // warm state (matching Box2D-v3's per-substep `b2WarmStartContactsTask`).
            // A zero seed (missed / disabled) applies nothing.
            Self::warm_start_apply(mc, points, bodies_eff);

            // (3)+(4) Soft normal solve + coupled-friction cone (one sweep).
            Self::solve_velocities(mc, points, bodies_eff, scratch.bodies(), soft, true);

            // (5) Position integrate DYNAMIC bodies only, then re-rotate the
            // world inertia for the next substep's effective mass.
            //
            // KINEMATIC bodies: their externally-set velocity is READ by contacts
            // (a correct one-sided response — inv_mass==0 makes them immovable to
            // impulses), but their position is NOT advanced here (this gate, and the
            // gravity gate above, admit `Dynamic && inv_mass != 0` only). So in
            // solver-owned mode a kinematic body's externally-driven MOTION does not
            // progress within a step — a known, intentional W2 deferral (kinematic
            // integration is not built yet).
            //
            // O1: position + quaternion integrate. MEASURED-SCALAR: the SoA AVX2
            // `position_integrate` kernel REGRESSES (~1.6× slower) on this AoS
            // `BodyState` layout — the per-body gather/scatter of 14 scattered
            // fields overwhelms the light integrate arithmetic, while the scalar
            // loop auto-vectorizes well. So the solver keeps this sub-pass scalar
            // (passing `false`) regardless of the flag; the bit-identical
            // `simd::position_integrate` kernel still ships + is differential-tested
            // (it pays off only on a future SoA `BodyState`). The HOT kernel is the
            // inertia refresh below (~1.46× under AVX2, run substeps×(1+relax) times).
            {
                let mut view = scratch.bodies.build_view();
                let snapshot = view.as_mut_slice();
                simd::position_integrate(bodies_eff, snapshot, h, false);
            }
            Self::refresh_inertia(bodies_eff, scratch.bodies(), use_simd);

            // (6) Relax passes: re-solve bias-free to remove soft-bias energy.
            for _ in 0..config.relax_iterations {
                Self::solve_velocities(mc, points, bodies_eff, scratch.bodies(), soft, false);
            }
        }

        // Post-loop restitution: ONCE, velocity-only, bias-free. Read `vn_initial`
        // into a local borrow disjoint from the bodies read slice.
        let vn_initial = core::mem::take(&mut scratch.vn_initial);
        Self::apply_restitution(mc, points, bodies_eff, scratch.bodies(), &vn_initial);
        scratch.vn_initial = vn_initial;

        // (W3) Store the converged accumulated impulses into the freshly-zeroed
        // write table (in manifold order) and swap read ↔ write so next frame
        // seeds from this frame's solution.
        self.store_and_swap();

        // Write the solved velocities back (positions/orientations were
        // integrated in place into the snapshot) and flag every integrated
        // DYNAMIC row — including free bodies that just fell with no contact.
        self.write_back(scratch);
    }

    /// Always `false` — this solver does real work.
    #[inline]
    fn is_noop(&self) -> bool {
        false
    }

    /// Always `true` — the TGS solver integrates DYNAMIC bodies inside its
    /// substep loop, so the pipeline's `physics_integrate` must be gated off (C2).
    #[inline]
    fn owns_integration(&self) -> bool {
        true
    }
}
