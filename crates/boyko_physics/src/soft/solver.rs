//! The XPBD soft-body step (Physics O11 SP1, plan D2/D3) — a SEPARATE position
//! pass run after the rigid solve.
//!
//! [`physics_soft_step`] is a STRICTLY DISJOINT integrator: it operates entirely on
//! [`SoftBody`] columns, never reads or writes the rigid `SolverScratch.bodies`,
//! never sets a touched bit, and never enters `physics_apply`. The rigid solvers
//! and the whole rigid pipeline are byte-untouched (the campaign 0%-gate); the only
//! shared state read is the gather-stamped [`PhysicsConfig`] (`dt` / `gravity` /
//! `substeps`) and the [`SdfField`].
//!
//! # Determinism boundary (INVIOLABLE)
//!
//! Every floating-point operation here is EXACT `mul`/`add`/`sub`/`div`/`sqrt` —
//! NO `rsqrt`/`rcp`/`mul_add`/FMA-contraction, NO [`Vec3::normalize`] (it collapses
//! at `f32::MIN_POSITIVE`; the constraint projection normalizes with an explicit
//! `d * (1.0 / len)` past the [`LEN_EPS`] guard). The prediction adds gravity THEN
//! multiplies (`v + g*h`, separate ops), then multiplies THEN adds for the position
//! (`x + v*h`); never an FMA. Constraints are swept in pinned array order `0..m` in
//! ONE Gauss-Seidel iteration per substep; particles and bodies are visited in
//! fixed index order; pinned (`inv_mass == 0`) particles are frozen.

use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::system::{Res, ResMut};

use crate::math::Vec3;
use crate::resources::{BroadphaseGrid, PhysicsConfig, SolverScratch};
use crate::sdf_query::SdfField;
use crate::soft::collide::collide_sdf;
use crate::soft::component::SoftBody;
use crate::soft::coupling::{CouplingCtx, SoftRigidReaction, resolve_coupling};

/// Minimum constraint length below which a distance constraint is skipped (plan
/// SP1).
///
/// When two particles coincide (or a zero rest length collapses them), the
/// direction `d / |d|` is undefined; projecting would divide by ~zero and emit a
/// `NaN`. The constraint is skipped below this length — far above FP noise, far
/// below any real edge.
pub const LEN_EPS: f32 = 1e-6;

/// Speed below which a particle is considered at rest (plan SP1) — the tester's rest
/// gate threshold. Unused by the kernel itself; exported for the determinism /
/// settling tests.
pub const REST_SPEED_EPS: f32 = 1e-3;

/// Minimum volume-constraint denominator below which the projection is skipped
/// (SP2 D3) — the volume-constraint mirror of [`LEN_EPS`].
///
/// `denom = wsum + α̃` collapses toward zero only for a degenerate (coplanar /
/// fully-pinned) tet whose gradients vanish; projecting then divides by ~zero and
/// emits a `NaN`. The construction-time [`SoftBodyError::DegenerateTet`] guard
/// rejects coplanar-at-rest tets, so this is the run-time defense for a tet that
/// collapses DURING the sim. Also the construction coplanarity threshold
/// (`|V0| >= DENOM_EPS`).
pub const DENOM_EPS: f32 = 1e-12;

/// Speed below which a soft-body particle's velocity is hard-floored to zero (SP2
/// D5), gated by [`PhysicsConfig::soft_rest_clamp`].
///
/// The squared compare (`|v|² < REST_CLAMP_EPS²`) is used so no `sqrt` is taken.
/// Far above FP noise, far below any visible motion — a particle this slow is at
/// rest, and zeroing its velocity stops residual creep without perturbing a moving
/// body.
pub const REST_CLAMP_EPS: f32 = 2e-3;

/// Advances every opted-in [`SoftBody`] by one XPBD step (plan SP1, D2/D3).
///
/// Early-returns when [`PhysicsConfig::soft_body`] is `false` (the entire opt-in
/// gate). Otherwise, for each soft body, runs `substeps` XPBD substeps of
/// predict → project (one Gauss-Seidel pass of distance constraints) → SDF collide
/// → velocity update, all in place on the body's SoA columns (zero per-step alloc).
///
/// `dt` / `gravity` / `substeps` come from the gather-stamped [`PhysicsConfig`]; the
/// shared [`SdfField`] is read-only. This system is registered `.after(solve)` and
/// `.before(apply)` by [`add_physics_soft`](crate::plugin::add_physics_soft), and is
/// a strictly disjoint integrator (it never touches the rigid scratch).
//
// `clippy::needless_pass_by_value`: `Res<_>` is a by-value `SystemParam` read via a
// `&*` reborrow — the same false-positive the rigid systems document.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_soft_step(
    mut query: Query<&mut SoftBody>,
    cfg: Res<PhysicsConfig>,
    field: Res<SdfField>,
) {
    if !cfg.soft_body {
        // The 0%-gate: an un-opted world does no soft-body work.
        return;
    }
    let field = &*field;
    let p = step_params(&cfg);
    for body in query.iter_mut() {
        // No coupling context: byte-identical to SP1 on a body with no tets (the
        // volume sweep is `0..0`) and `soft_damping == 0` / `soft_rest_clamp ==
        // false` (the clamp is a `* 1.0` identity then a disabled floor).
        step_body(body, field, &p, &cfg, None);
    }
}

/// The soft↔rigid-COUPLED XPBD soft-body step (SP2 D6/D7) — registered in place of
/// [`physics_soft_step`] ONLY on the coupling-wired path
/// ([`add_physics_soft`](crate::plugin::add_physics_soft) with `coupling == true`).
///
/// Identical to [`physics_soft_step`] but additionally reads the read-only rigid
/// snapshot ([`SolverScratch::bodies`], the SAME frame-N gather the rigid solve
/// consumed — never mutated here) and the [`BroadphaseGrid`] to resolve per-particle
/// soft-vs-rigid collisions, accumulating the rigid REACTION into
/// [`SoftRigidReaction`] (applied to the [`RigidBody`](crate::components::RigidBody)
/// component AFTER `physics_apply` by
/// [`physics_soft_rigid_apply`](crate::soft::coupling::physics_soft_rigid_apply)).
/// It remains a strictly disjoint integrator with respect to the rigid scratch: it
/// READS `scratch.bodies` but never writes it, never sets a touched bit, and never
/// enters `physics_apply`.
//
// `clippy::needless_pass_by_value`: the `Res<_>` params are by-value `SystemParam`s
// read via `&*` reborrows — the same false-positive the rigid systems document.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_soft_step_coupled(
    mut query: Query<&mut SoftBody>,
    cfg: Res<PhysicsConfig>,
    field: Res<SdfField>,
    scratch: Res<SolverScratch>,
    grid: Res<BroadphaseGrid>,
    mut reaction: ResMut<SoftRigidReaction>,
) {
    if !cfg.soft_body {
        // The 0%-gate: an un-opted world does no soft-body work.
        return;
    }
    let field = &*field;
    let scratch = &*scratch;
    let grid = &*grid;
    let reaction = &mut *reaction;
    // Reset the reaction accumulator for this frame (CLEARED, never resized — the
    // dense per-body rows are reserved to body capacity at wire-up; zero per-step
    // alloc). Sized to the current snapshot row count.
    reaction.reset(scratch.bodies.len());

    let p = step_params(&cfg);
    let couple = cfg.soft_rigid_coupling;
    // SP2 M1: `resolve_coupling`/`deepest_contact` read the broadphase grid's CSR
    // cell slices + oversized list, populated ONLY by the `BroadphaseKind::Grid`
    // broadphase arm. `add_physics_pipeline` forces `Grid` on the coupling path, and
    // `physics_broadphase` runs `.after(gather)` (before this `.after(solve)` step),
    // so the grid MUST be built here whenever coupling is active and bodies exist —
    // an unbuilt grid means the broadphase ran the wrong arm and coupling silently
    // resolves zero contacts. The `is_empty()` arm keeps an empty world valid (the
    // grid is not built when there is nothing to bucket).
    debug_assert!(
        !couple || scratch.bodies.is_empty() || grid.is_built(),
        "invariant: soft↔rigid coupling requires the broadphase grid to be built \
         (set PhysicsConfig::broadphase = BroadphaseKind::Grid — add_physics_pipeline \
         does this on the coupling path)"
    );
    for body in query.iter_mut() {
        let ctx = if couple {
            Some(CouplingCtx {
                bodies: &scratch.bodies,
                grid,
                reaction,
            })
        } else {
            // The flag is off but the system is wired: behave exactly like the
            // non-coupling path (no coupling reads/writes).
            None
        };
        step_body(body, field, &p, &cfg, ctx);
    }
}

/// The per-step substep parameters derived once from [`PhysicsConfig`].
#[derive(Clone, Copy)]
struct StepParams {
    substeps: u32,
    h: f32,
    inv_h: f32,
    gravity: Vec3,
}

/// Derives the substep parameters from the gather-stamped [`PhysicsConfig`].
#[inline]
fn step_params(cfg: &PhysicsConfig) -> StepParams {
    // `max(1)` is release-safe (never a div-by-zero on `dt / substeps`); the
    // `debug_assert!` documents the user-facing invariant `substeps >= 1`.
    debug_assert!(
        cfg.substeps >= 1,
        "invariant: PhysicsConfig::substeps must be >= 1"
    );
    let substeps = cfg.substeps.max(1);
    let h = cfg.dt / substeps as f32;
    StepParams {
        substeps,
        h,
        inv_h: 1.0 / h,
        gravity: cfg.gravity,
    }
}

/// One full XPBD advance of a single soft body (`substeps` substeps).
///
/// Split out so the hot per-substep loop is a compact function the body iteration
/// calls per body; keeps the step systems themselves small (I-cache). `coupling`
/// is `None` on the SP1 / non-coupling path; `Some(ctx)` resolves per-particle
/// soft-vs-rigid collisions (SP2 D6/D7) between the volume sweep and the SDF
/// collide.
fn step_body(
    body: &mut SoftBody,
    field: &SdfField,
    p: &StepParams,
    cfg: &PhysicsConfig,
    mut coupling: Option<CouplingCtx<'_>>,
) {
    let n = body.particle_count();
    let m = body.constraint_count();
    let k = body.tet_count();
    // Column-length invariants (cheap, vanish in release).
    debug_assert!(body.pos_x.len() == n && body.pos_y.len() == n && body.pos_z.len() == n);
    debug_assert!(body.prev_x.len() == n && body.prev_y.len() == n && body.prev_z.len() == n);
    debug_assert!(body.vel_x.len() == n && body.vel_y.len() == n && body.vel_z.len() == n);
    debug_assert!(body.inv_mass.len() == n);
    debug_assert!(body.c_a.len() == m && body.c_b.len() == m);
    debug_assert!(body.c_rest.len() == m && body.c_compliance.len() == m);
    debug_assert!(body.t0.len() == k && body.t1.len() == k && body.t2.len() == k);
    debug_assert!(body.t3.len() == k && body.t_rest.len() == k && body.t_compliance.len() == k);
    debug_assert!(
        body.coupling_prev_x.len() == n
            && body.coupling_prev_y.len() == n
            && body.coupling_prev_z.len() == n
    );
    debug_assert!(
        body.coupling_dv_x.len() == n
            && body.coupling_dv_y.len() == n
            && body.coupling_dv_z.len() == n
            && body.coupling_hit.len() == n
    );
    debug_assert!(
        body.particle_radius >= 0.0,
        "invariant: particle_radius must be >= 0"
    );

    let radius = body.particle_radius;
    let gh = p.gravity * p.h;
    let h = p.h;
    let inv_h = p.inv_h;
    // SP2 D5(a): viscous factor. Default `soft_damping == 0.0` ⇒ `1.0`, an EXACT
    // identity multiply ⇒ the SP1 0%-gate.
    let visc = 1.0 - cfg.soft_damping;
    let rest_clamp = cfg.soft_rest_clamp;
    let clamp_sq = REST_CLAMP_EPS * REST_CLAMP_EPS;

    for _ in 0..p.substeps {
        // Predict — fixed particle index order. Pinned (w == 0) particles only
        // carry `prev = pos` (frozen position); movable particles integrate.
        for i in 0..n {
            if body.inv_mass[i] != 0.0 {
                // Separate add THEN mul (NO FMA): v = v + g*h.
                body.vel_x[i] += gh.x;
                body.vel_y[i] += gh.y;
                body.vel_z[i] += gh.z;
                body.prev_x[i] = body.pos_x[i];
                body.prev_y[i] = body.pos_y[i];
                body.prev_z[i] = body.pos_z[i];
                // Separate mul THEN add (NO FMA): x = x + v*h.
                body.pos_x[i] += body.vel_x[i] * h;
                body.pos_y[i] += body.vel_y[i] * h;
                body.pos_z[i] += body.vel_z[i] * h;
            } else {
                body.prev_x[i] = body.pos_x[i];
                body.prev_y[i] = body.pos_y[i];
                body.prev_z[i] = body.pos_z[i];
            }
        }

        // Project distance constraints — ONE Gauss-Seidel pass, fixed array order
        // `0..m` (SP1).
        for c in 0..m {
            project_distance(body, c, h);
        }

        // Project volume constraints — ONE pass, fixed array order `0..k` (SP2 D3).
        // `k == 0` for an SP1-only body ⇒ this loop is a per-body no-op.
        for t in 0..k {
            project_volume(body, t, h);
        }

        // Soft↔rigid coupling (SP2 D6/D7) — between the volume sweep and the SDF
        // collide. Only on the coupling path; a no-op otherwise.
        if let Some(ctx) = coupling.as_mut() {
            resolve_coupling(body, ctx, inv_h);
        }

        // SDF collision — fixed particle order (SP1; one-sided, no reaction).
        //
        // SP2 W1: the SDF push is a one-sided static collision that, like on the
        // uncoupled path, MUST contribute to the particle's velocity. The coupled
        // velocity baseline `(coupling_prev - prev) * inv_h` excludes ONLY the
        // coupling push — `coupling_prev` is the position just BEFORE the coupling
        // push but it was captured BEFORE this SDF collide. So for a particle BOTH
        // rigid-coupled AND SDF-pushed this substep, fold the SDF displacement
        // `(pos_after - pos_before)` into `coupling_prev`, so the baseline retains
        // the SDF push while still excluding the coupling push (the uncoupled path
        // gets the SDF push for free since it reads the final `pos`). Exact
        // `sub`/`add` only (the determinism boundary); gated on the coupling path so
        // the SP1 / non-coupling loop stays byte-identical (the per-body 0%-gate).
        if coupling.is_some() {
            for i in 0..n {
                if body.inv_mass[i] != 0.0 {
                    let before = Vec3::new(body.pos_x[i], body.pos_y[i], body.pos_z[i]);
                    let after = collide_sdf(field, before, radius);
                    body.pos_x[i] = after.x;
                    body.pos_y[i] = after.y;
                    body.pos_z[i] = after.z;
                    // Only a coupled particle has a `coupling_prev` baseline the
                    // velocity update reads; for it, add the SDF displacement so the
                    // SDF push survives into the coupled velocity.
                    if body.coupling_hit[i] != 0 {
                        body.coupling_prev_x[i] += after.x - before.x;
                        body.coupling_prev_y[i] += after.y - before.y;
                        body.coupling_prev_z[i] += after.z - before.z;
                    }
                }
            }
        } else {
            for i in 0..n {
                if body.inv_mass[i] != 0.0 {
                    let pos = Vec3::new(body.pos_x[i], body.pos_y[i], body.pos_z[i]);
                    let pos = collide_sdf(field, pos, radius);
                    body.pos_x[i] = pos.x;
                    body.pos_y[i] = pos.y;
                    body.pos_z[i] = pos.z;
                }
            }
        }

        // Velocity update — (x - prev) * inv_h for movable particles (SP1), then the
        // SP2 D5 rest-residual clamp (viscous THEN floor, fixed order). A particle
        // the coupling pushed THIS substep uses the D4 baseline
        // `(coupling_prev - prev) * inv_h` (excluding the coupling push) PLUS the
        // deferred D7 momentum-exchange delta `coupling_dv` (SP2 D6/D7). Per SP2 W1
        // `coupling_prev` already had any post-coupling SDF-collide displacement
        // folded in (above), so the SDF push contributes to the coupled velocity
        // exactly as on the uncoupled path.
        for i in 0..n {
            if body.inv_mass[i] != 0.0 {
                // `coupling_hit` is `0` for every particle off the coupling path
                // (the columns are never written there) ⇒ the SP1 branch ⇒ the
                // 0%-gate.
                let coupled = !body.coupling_hit.is_empty() && body.coupling_hit[i] != 0;
                let (mut vx, mut vy, mut vz) = if coupled {
                    (
                        (body.coupling_prev_x[i] - body.prev_x[i]) * inv_h + body.coupling_dv_x[i],
                        (body.coupling_prev_y[i] - body.prev_y[i]) * inv_h + body.coupling_dv_y[i],
                        (body.coupling_prev_z[i] - body.prev_z[i]) * inv_h + body.coupling_dv_z[i],
                    )
                } else {
                    (
                        (body.pos_x[i] - body.prev_x[i]) * inv_h,
                        (body.pos_y[i] - body.prev_y[i]) * inv_h,
                        (body.pos_z[i] - body.prev_z[i]) * inv_h,
                    )
                };
                // SP2 D5(a) viscous: `* (1 - soft_damping)`. Identity when the
                // damping is the default `0.0`.
                vx *= visc;
                vy *= visc;
                vz *= visc;
                // SP2 D5(b) hard floor: gated by `soft_rest_clamp` (default off).
                // Squared compare, strict `<`, after the viscous scale.
                if rest_clamp && (vx * vx + vy * vy + vz * vz) < clamp_sq {
                    vx = 0.0;
                    vy = 0.0;
                    vz = 0.0;
                }
                body.vel_x[i] = vx;
                body.vel_y[i] = vy;
                body.vel_z[i] = vz;
            }
        }
    }
}

/// Projects one distance constraint `c` (EXACT sqrt + divide only).
///
/// XPBD distance projection with per-constraint compliance: computes the current
/// length, the compliance-tilded denominator, and the position deltas split between
/// the two endpoints by inverse mass. With one Gauss-Seidel iteration per substep
/// the Lagrange multiplier `λ` starts at zero each substep, so it is neither
/// accumulated nor stored (SP1 is one-iteration; SP2 may carry `λ`).
///
/// Skips both-pinned constraints BEFORE the `sqrt`, and coincident / zero-rest
/// constraints whose length is below [`LEN_EPS`] (an undefined direction). The
/// direction is `d * (1.0 / len)` — an explicit divide, NOT `rsqrt` and NOT
/// [`Vec3::normalize`] (which would collapse the direction at `f32::MIN_POSITIVE`).
#[inline]
fn project_distance(body: &mut SoftBody, c: usize, h: f32) {
    let a = body.c_a[c] as usize;
    let b = body.c_b[c] as usize;
    let wa = body.inv_mass[a];
    let wb = body.inv_mass[b];
    let wsum = wa + wb;
    if wsum == 0.0 {
        // Both endpoints pinned — skip BEFORE the sqrt.
        return;
    }
    let d = Vec3::new(
        body.pos_x[a] - body.pos_x[b],
        body.pos_y[a] - body.pos_y[b],
        body.pos_z[a] - body.pos_z[b],
    );
    // EXACT sqrt (the determinism boundary) — never `rsqrt`.
    let len = d.length_squared().sqrt();
    if len < LEN_EPS {
        // Coincident / zero-rest / degenerate — direction is undefined.
        return;
    }
    // DIVIDE then mul (explicit; NOT `rsqrt`, NOT `Vec3::normalize`).
    let nrm = d * (1.0 / len);
    let cc = len - body.c_rest[c];
    let alpha_tilde = body.c_compliance[c] / (h * h);
    let denom = wsum + alpha_tilde;
    // `wsum > 0` here (the both-pinned case returned) and `alpha_tilde >= 0`.
    debug_assert!(
        denom > 0.0,
        "invariant: distance-constraint denom must be > 0"
    );
    let s = -cc / denom;
    // Split the correction by inverse mass; a pinned endpoint (w == 0) gets no move.
    let da = nrm * (s * wa);
    let db = nrm * (-s * wb);
    body.pos_x[a] += da.x;
    body.pos_y[a] += da.y;
    body.pos_z[a] += da.z;
    body.pos_x[b] += db.x;
    body.pos_y[b] += db.y;
    body.pos_z[b] += db.z;
}

/// Projects one volume constraint `t` — the XPBD signed-volume constraint over a
/// tetrahedron (SP2 D3, EXACT `mul`/`add`/`sub`/`div` only).
///
/// The current signed volume is `V = (1/6)·(e1 × e2)·e3` with edges anchored at
/// `p0` (`e1 = p1 − p0`, etc.) for FP conditioning. The constraint is `C = V − V0`;
/// its gradients are `g1 = (1/6)·(e2 × e3)`, `g2 = (1/6)·(e3 × e1)`,
/// `g3 = (1/6)·(e1 × e2)`, and `g0 = −(g1 + g2 + g3)` (the pinned add order makes
/// `Σg == 0` exactly, so the projection conserves the centroid). The Lagrange step
/// is `s = −C / (Σ wᵢ·gᵢ·gᵢ + α̃)`; each vertex moves `gᵢ·(s·wᵢ)` (a pinned
/// `wᵢ == 0` vertex never moves).
///
/// Uses the same cross / dot leaves as the rest math ([`Vec3::cross`] = the
/// separate `mul`/`sub` form, no FMA; [`Vec3::dot`] = left-to-right), so the
/// runtime `V` matches the construction-time `V0` op sequence (`C == 0` at rest).
/// Skips a collapsed tet whose `denom < DENOM_EPS` (mirrors the distance sweep's
/// `len < LEN_EPS`) before the divide.
#[inline]
fn project_volume(body: &mut SoftBody, t: usize, h: f32) {
    let i0 = body.t0[t] as usize;
    let i1 = body.t1[t] as usize;
    let i2 = body.t2[t] as usize;
    let i3 = body.t3[t] as usize;
    let p0 = Vec3::new(body.pos_x[i0], body.pos_y[i0], body.pos_z[i0]);
    let p1 = Vec3::new(body.pos_x[i1], body.pos_y[i1], body.pos_z[i1]);
    let p2 = Vec3::new(body.pos_x[i2], body.pos_y[i2], body.pos_z[i2]);
    let p3 = Vec3::new(body.pos_x[i3], body.pos_y[i3], body.pos_z[i3]);
    // Edge-anchored at p0 (FP conditioning). Pinned cross operand order; dot
    // left-to-right.
    let e1 = p1 - p0;
    let e2 = p2 - p0;
    let e3 = p3 - p0;
    let vol = (1.0 / 6.0) * e1.cross(e2).dot(e3);
    let cc = vol - body.t_rest[t];
    let g1 = e2.cross(e3) * (1.0 / 6.0);
    let g2 = e3.cross(e1) * (1.0 / 6.0);
    let g3 = e1.cross(e2) * (1.0 / 6.0);
    // Pinned add order ⇒ Σg == 0 exactly (g0 + g1 + g2 + g3 == 0).
    let g0 = (g1 + g2 + g3) * -1.0;
    let w0 = body.inv_mass[i0];
    let w1 = body.inv_mass[i1];
    let w2 = body.inv_mass[i2];
    let w3 = body.inv_mass[i3];
    // Summed in fixed vertex order 0,1,2,3.
    let wsum = w0 * g0.dot(g0) + w1 * g1.dot(g1) + w2 * g2.dot(g2) + w3 * g3.dot(g3);
    let alpha_tilde = body.t_compliance[t] / (h * h);
    let denom = wsum + alpha_tilde;
    if denom < DENOM_EPS {
        // Collapsed tet — all gradients vanish (mirrors the distance sweep's
        // `len < LEN_EPS`).
        return;
    }
    debug_assert!(
        denom >= DENOM_EPS,
        "invariant: volume-constraint denom must be >= DENOM_EPS past the skip"
    );
    let s = -cc / denom;
    // Fixed vertex order; a pinned vertex (w == 0) gets no move.
    let d0 = g0 * (s * w0);
    let d1 = g1 * (s * w1);
    let d2 = g2 * (s * w2);
    let d3 = g3 * (s * w3);
    body.pos_x[i0] += d0.x;
    body.pos_y[i0] += d0.y;
    body.pos_z[i0] += d0.z;
    body.pos_x[i1] += d1.x;
    body.pos_y[i1] += d1.y;
    body.pos_z[i1] += d1.z;
    body.pos_x[i2] += d2.x;
    body.pos_y[i2] += d2.y;
    body.pos_z[i2] += d2.z;
    body.pos_x[i3] += d3.x;
    body.pos_y[i3] += d3.y;
    body.pos_z[i3] += d3.z;
}
