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
use boyko_ecs::ecs::core::system::Res;

use crate::math::Vec3;
use crate::resources::PhysicsConfig;
use crate::sdf_query::SdfField;
use crate::soft::collide::collide_sdf;
use crate::soft::component::SoftBody;

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
    // `max(1)` is release-safe (never a div-by-zero on `dt / substeps`); the
    // `debug_assert!` documents the user-facing invariant `substeps >= 1`.
    debug_assert!(
        cfg.substeps >= 1,
        "invariant: PhysicsConfig::substeps must be >= 1"
    );
    let substeps = cfg.substeps.max(1);
    let h = cfg.dt / substeps as f32;
    let inv_h = 1.0 / h;
    let gravity = cfg.gravity;

    for body in query.iter_mut() {
        step_body(body, field, substeps, h, inv_h, gravity);
    }
}

/// One full XPBD advance of a single soft body (`substeps` substeps).
///
/// Split out so the hot per-substep loop is a compact function the body iteration
/// calls per body; keeps `physics_soft_step` itself small (I-cache).
fn step_body(
    body: &mut SoftBody,
    field: &SdfField,
    substeps: u32,
    h: f32,
    inv_h: f32,
    gravity: Vec3,
) {
    let n = body.particle_count();
    let m = body.constraint_count();
    // Column-length invariants (cheap, vanish in release).
    debug_assert!(body.pos_x.len() == n && body.pos_y.len() == n && body.pos_z.len() == n);
    debug_assert!(body.prev_x.len() == n && body.prev_y.len() == n && body.prev_z.len() == n);
    debug_assert!(body.vel_x.len() == n && body.vel_y.len() == n && body.vel_z.len() == n);
    debug_assert!(body.inv_mass.len() == n);
    debug_assert!(body.c_a.len() == m && body.c_b.len() == m);
    debug_assert!(body.c_rest.len() == m && body.c_compliance.len() == m);
    debug_assert!(
        body.particle_radius >= 0.0,
        "invariant: particle_radius must be >= 0"
    );

    let radius = body.particle_radius;
    let gh = gravity * h;

    for _ in 0..substeps {
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
        // `0..m`.
        for c in 0..m {
            project_distance(body, c, h);
        }

        // SDF collision — fixed particle order.
        for i in 0..n {
            if body.inv_mass[i] != 0.0 {
                let p = Vec3::new(body.pos_x[i], body.pos_y[i], body.pos_z[i]);
                let p = collide_sdf(field, p, radius);
                body.pos_x[i] = p.x;
                body.pos_y[i] = p.y;
                body.pos_z[i] = p.z;
            }
        }

        // Velocity update — (x - prev) * inv_h for movable particles.
        for i in 0..n {
            if body.inv_mass[i] != 0.0 {
                body.vel_x[i] = (body.pos_x[i] - body.prev_x[i]) * inv_h;
                body.vel_y[i] = (body.pos_y[i] - body.prev_y[i]) * inv_h;
                body.vel_z[i] = (body.pos_z[i] - body.prev_z[i]) * inv_h;
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
