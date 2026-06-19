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
use crate::solver::contact::is_dynamic_row;
use crate::soft::collide::collide_sdf;
use crate::soft::component::SoftBody;
use crate::soft::coupling::{CouplingCtx, SoftRigidReaction, resolve_coupling};
use crate::soft::self_collision::resolve_self_collision;

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
///
/// `pub(in crate::soft)` (SP4): the colored sibling reuses the SAME derivation so the
/// substep timestep matches the serial path bit-for-bit.
#[derive(Clone, Copy)]
pub(in crate::soft) struct StepParams {
    pub(in crate::soft) substeps: u32,
    pub(in crate::soft) h: f32,
    pub(in crate::soft) inv_h: f32,
    pub(in crate::soft) gravity: Vec3,
}

/// Derives the substep parameters from the gather-stamped [`PhysicsConfig`].
///
/// `pub(in crate::soft)` (SP4): shared by the serial step systems and the colored
/// sibling.
#[inline]
pub(in crate::soft) fn step_params(cfg: &PhysicsConfig) -> StepParams {
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
    // SP3 self-collision CSR scratch: offsets are `table + 1`, items are `n`.
    debug_assert!(
        body.sc_cell_start.len() == body.self_table_size() + 1
            && body.sc_cursor.len() == body.self_table_size() + 1
            && body.sc_cell_items.len() == n
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
    // SP3: number of self-collision GS sweeps per substep. Default `0` ⇒
    // `resolve_self_collision` early-returns before any hashing (the SP3 0%-gate).
    let self_collision_iters = cfg.self_collision_iters;

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

        // Self-collision (SP3) — AFTER the volume sweep, BEFORE the coupling, so the
        // corrected positions feed the coupled-velocity computation for free (no
        // separate velocity fold). `self_collision_iters == 0` early-returns before
        // any hashing ⇒ an SP1/SP2 world is byte-identical (the SP3 0%-gate).
        resolve_self_collision(body, self_collision_iters, radius);

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

/// Runs the SERIAL non-coupling [`step_body`] on one body — the SP4 0%-gate entry
/// the colored sibling calls when [`PhysicsConfig::soft_body_colored`] is `false`.
///
/// A thin `pub(in crate::soft)` forwarder to the LITERALLY-UNTOUCHED [`step_body`]
/// with `coupling = None` (C4): it produces a result byte-identical to
/// [`physics_soft_step`], so the colored soft step is the SP4 0%-gate when the flag
/// is off. `step_body` itself is not edited (its body, op order, and signature are
/// unchanged) — only this forwarder is added so `soft::colored` reaches it without
/// duplicating the driver passes for the 0%-gate path.
#[inline]
pub(in crate::soft) fn step_body_serial(
    body: &mut SoftBody,
    field: &SdfField,
    p: &StepParams,
    cfg: &PhysicsConfig,
) {
    step_body(body, field, p, cfg, None);
}

/// RAW column base pointers for the SHARED projection cores (SP4 W3-A soundness fix).
///
/// The colored-parallel solve hands each worker this bundle of `*mut`/`*const`
/// column bases instead of a `&mut SoftBody`. The leaf cores
/// ([`project_distance_raw`] / [`project_volume_raw`] /
/// [`project_self_pair_raw`](crate::soft::self_collision::project_self_pair_raw))
/// access a column ELEMENT through `*base.add(i)`, which under Tree-Borrows retags
/// ONLY element `i` — NOT the whole allocation. Two workers writing the C2-disjoint
/// dynamic rows of one color therefore never form overlapping protectors, so the
/// concurrent solve is data-race-free. Contrast a `&mut SoftBody` + `body.pos_x[a]`:
/// the `Deref` reborrows the WHOLE `pos_x` buffer per worker, so every worker retags
/// the entire allocation and the disjoint-element writes collide (the Miri-TB data
/// race this struct removes).
///
/// The SERIAL kernels build this from their own `&mut SoftBody` and call the SAME
/// cores, so the projection math is never duplicated (the C1/W3-A anti-drift rule)
/// and the serial path stays byte-identical.
///
/// `Copy` so a worker closure captures it by value (the wrapper is shared `&`
/// across the spawn loop; see the `Send`/`Sync` impls in `soft::colored`). All
/// fields are the live bases of the body's SoA columns; the cores read/write only
/// the elements named by the constraint index they are given.
#[derive(Copy, Clone)]
pub(in crate::soft) struct SoftCols {
    /// Current position X base — WRITTEN per element (the only mutated columns).
    pub(in crate::soft) pos_x: *mut f32,
    /// Current position Y base — WRITTEN per element.
    pub(in crate::soft) pos_y: *mut f32,
    /// Current position Z base — WRITTEN per element.
    pub(in crate::soft) pos_z: *mut f32,
    /// Per-particle inverse-mass base — read only.
    pub(in crate::soft) inv_mass: *const f32,
    /// Distance-constraint endpoint-A index base — read only.
    pub(in crate::soft) c_a: *const u32,
    /// Distance-constraint endpoint-B index base — read only.
    pub(in crate::soft) c_b: *const u32,
    /// Per-constraint rest-length base — read only.
    pub(in crate::soft) c_rest: *const f32,
    /// Per-constraint compliance base — read only.
    pub(in crate::soft) c_compliance: *const f32,
    /// Tet vertex-0 index base — read only.
    pub(in crate::soft) t0: *const u32,
    /// Tet vertex-1 index base — read only.
    pub(in crate::soft) t1: *const u32,
    /// Tet vertex-2 index base — read only.
    pub(in crate::soft) t2: *const u32,
    /// Tet vertex-3 index base — read only.
    pub(in crate::soft) t3: *const u32,
    /// Per-tet signed rest-volume base — read only.
    pub(in crate::soft) t_rest: *const f32,
    /// Per-tet compliance base — read only.
    pub(in crate::soft) t_compliance: *const f32,
}

impl SoftCols {
    /// Extracts the live column base pointers from `body`.
    ///
    /// `&mut SoftBody` → raw bases: the `pos_*` columns are taken `*mut` (the cores
    /// write them per element); every other column is `*const` (read only). Cheap —
    /// a handful of `Vec` base reads, done ONCE per dispatch (colored) or once per
    /// serial wrapper call. After this, the cores never re-`Deref` the `Vec`s, so the
    /// parallel path forms no whole-buffer reborrow.
    #[inline]
    pub(in crate::soft) fn from_body(body: &mut SoftBody) -> Self {
        SoftCols {
            pos_x: body.pos_x.as_mut_ptr(),
            pos_y: body.pos_y.as_mut_ptr(),
            pos_z: body.pos_z.as_mut_ptr(),
            inv_mass: body.inv_mass.as_ptr(),
            c_a: body.c_a.as_ptr(),
            c_b: body.c_b.as_ptr(),
            c_rest: body.c_rest.as_ptr(),
            c_compliance: body.c_compliance.as_ptr(),
            t0: body.t0.as_ptr(),
            t1: body.t1.as_ptr(),
            t2: body.t2.as_ptr(),
            t3: body.t3.as_ptr(),
            t_rest: body.t_rest.as_ptr(),
            t_compliance: body.t_compliance.as_ptr(),
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
///
/// # Per-endpoint write guard (SP4 C1 — LOAD-BEARING)
///
/// Each endpoint's position write is gated `if is_dynamic_row(w) { add }`, routed
/// through the SAME [`is_dynamic_row`] predicate the SP4 coloring uses. This is the
/// SHARED kernel for both the serial [`step_body`] and the colored
/// `step_body_colored`. Serially it is byte-preserving: a pinned endpoint has
/// `w == ±0.0`, so the skipped write was `pos += nrm*(s*±0.0) == pos + ±0.0 == pos`
/// exactly (on the finite-position states the kernel operates on). In the colored
/// sweep it removes a value-benign-but-real concurrent write to a pinned row two
/// same-color constraints may share (the pinned write race only Miri-TB/loom can
/// see). The LOAD-BEARING finiteness invariant is `s.is_finite()` past the divide —
/// a finite `s` is what makes `s*(±0.0)` a signed zero and the skip bit-equal to
/// the add (mirrors the rigid `apply_impulse` finiteness `debug_assert!`). It MUST
/// NOT be `w >= 0.0`: a `-0.0` or negative finite `w` is out-of-contract-but-
/// determinism-safe (the guard and the coloring both route through
/// `is_dynamic_row(w) = w != 0.0`, which treats `±0.0` identically and a negative
/// finite mass as dynamic on BOTH paths — no serial/colored divergence).
///
/// `pub(in crate::soft)` (SP4 W3-A): the SINGLE definition shared by the serial
/// [`step_body`] and the colored `soft::colored::step_body_colored` — never
/// duplicated (duplicating the hardest determinism math is the drift hazard C1
/// avoids). A thin wrapper extracting [`SoftCols`] from `body` and forwarding to
/// the raw core [`project_distance_raw`] (the math lives there ONCE; the serial
/// path stays byte-identical to the raw per-element form — `body.pos_x[a]` and
/// `*pos_x.add(a)` name the same storage).
#[inline]
pub(in crate::soft) fn project_distance(body: &mut SoftBody, c: usize, h: f32) {
    let cols = SoftCols::from_body(body);
    // SAFETY: `cols` names `body`'s live column bases; `c` is a valid constraint
    //   index (`c < constraint_count()`, the caller's loop bound) and its endpoints
    //   `c_a[c]`/`c_b[c]` index valid particle rows (the constructor invariant). The
    //   serial caller holds the unique `&mut SoftBody`, so no aliasing.
    unsafe { project_distance_raw(cols, c, h) };
}

/// The RAW per-element core of [`project_distance`] (SP4 W3-A soundness fix).
///
/// Identical XPBD math, but every column access is a raw `*base.add(i)` instead of a
/// `Vec` index, so under Tree-Borrows it retags ONLY the element it touches — never
/// the whole buffer. This is what makes the colored-parallel solve race-free: two
/// workers writing the C2-disjoint dynamic rows of one color form no overlapping
/// element protectors. The serial wrapper calls it through the unique `&mut SoftBody`
/// (no aliasing); a colored worker calls it through [`SoftCols`] built from the live
/// body, writing only its chunk's disjoint dynamic rows.
///
/// # Safety
/// `cols` must name a live `SoftBody`'s column bases; `c < constraint_count()`; the
/// endpoints `c_a[c]`/`c_b[c]` must be valid particle rows `< particle_count()`. In
/// the parallel path, the caller must invoke this only on constraints of ONE color
/// whose DYNAMIC endpoints are pairwise disjoint across concurrent workers (the C2
/// lemma); a SHARED PINNED row is read-only (the C1 guard never writes it). On those
/// conditions the per-element reads/writes touch only provably-disjoint elements
/// (writes) plus read-only shared columns (`inv_mass`/topology) — no UB.
#[inline]
pub(in crate::soft) unsafe fn project_distance_raw(cols: SoftCols, c: usize, h: f32) {
    // SAFETY (all the per-element accesses below): `c` and the endpoint rows are in
    //   range (the fn contract); each `*p.add(i)` retags only element `i`. Reads of
    //   `inv_mass`/`c_*` are read-only shared; the `pos_*` writes are gated to the
    //   constraint's own dynamic rows (disjoint across workers — the contract).
    let a = unsafe { *cols.c_a.add(c) } as usize;
    let b = unsafe { *cols.c_b.add(c) } as usize;
    let wa = unsafe { *cols.inv_mass.add(a) };
    let wb = unsafe { *cols.inv_mass.add(b) };
    let wsum = wa + wb;
    if wsum == 0.0 {
        // Both endpoints pinned — skip BEFORE the sqrt.
        return;
    }
    let d = unsafe {
        Vec3::new(
            *cols.pos_x.add(a) - *cols.pos_x.add(b),
            *cols.pos_y.add(a) - *cols.pos_y.add(b),
            *cols.pos_z.add(a) - *cols.pos_z.add(b),
        )
    };
    // EXACT sqrt (the determinism boundary) — never `rsqrt`.
    let len = d.length_squared().sqrt();
    if len < LEN_EPS {
        // Coincident / zero-rest / degenerate — direction is undefined.
        return;
    }
    // DIVIDE then mul (explicit; NOT `rsqrt`, NOT `Vec3::normalize`).
    let nrm = d * (1.0 / len);
    let cc = len - unsafe { *cols.c_rest.add(c) };
    let alpha_tilde = unsafe { *cols.c_compliance.add(c) } / (h * h);
    let denom = wsum + alpha_tilde;
    // `wsum > 0` here (the both-pinned case returned) and `alpha_tilde >= 0`.
    debug_assert!(
        denom > 0.0,
        "invariant: distance-constraint denom must be > 0"
    );
    let s = -cc / denom;
    // SP4 C1: `s` finite is the LOAD-BEARING invariant that makes the pinned-endpoint
    // skip below bit-equal to the unconditional `+= nrm*(s*±0.0)` (a finite `s` times
    // a signed zero is a signed zero, and `x + ±0.0 == x`). NOT `w >= 0.0` (a `-0.0`
    // / negative finite mass is out-of-contract-but-determinism-safe; see the doc).
    debug_assert!(s.is_finite(), "invariant: distance-constraint Lagrange step must be finite");
    // Split the correction by inverse mass; a pinned endpoint (w == 0) gets no move.
    let da = nrm * (s * wa);
    let db = nrm * (-s * wb);
    // SP4 C1 per-endpoint write guard (shared by serial + colored): write only a
    // DYNAMIC endpoint, removing the value-benign pinned `+= ±0.0` (a same-color
    // pinned-write race in the colored sweep). Byte-preserving serially.
    if is_dynamic_row(wa) {
        unsafe {
            *cols.pos_x.add(a) += da.x;
            *cols.pos_y.add(a) += da.y;
            *cols.pos_z.add(a) += da.z;
        }
    }
    if is_dynamic_row(wb) {
        unsafe {
            *cols.pos_x.add(b) += db.x;
            *cols.pos_y.add(b) += db.y;
            *cols.pos_z.add(b) += db.z;
        }
    }
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
///
/// # Per-vertex write guard (SP4 C1 — LOAD-BEARING)
///
/// Each of the FOUR vertex writes is gated `if is_dynamic_row(w) { add }` through
/// the SAME [`is_dynamic_row`] predicate the coloring uses (the SHARED kernel for
/// serial + colored). Byte-preserving serially (a pinned vertex's skipped write was
/// `+= gᵢ*(s*±0.0) == +±0.0`); in the colored sweep it removes a same-color
/// pinned-vertex write race. The LOAD-BEARING finiteness invariant is
/// `s.is_finite()` past the divide (see [`project_distance`]); it MUST NOT be
/// `w >= 0.0`.
///
/// `pub(in crate::soft)` (SP4 W3-A): the SINGLE definition shared by the serial
/// [`step_body`] and the colored `soft::colored::step_body_colored`. A thin wrapper
/// extracting [`SoftCols`] from `body` and forwarding to the raw core
/// [`project_volume_raw`] (the math lives there ONCE; the serial path stays
/// byte-identical to the raw per-element form — `body.pos_x[i0]` and
/// `*pos_x.add(i0)` name the same storage).
#[inline]
pub(in crate::soft) fn project_volume(body: &mut SoftBody, t: usize, h: f32) {
    let cols = SoftCols::from_body(body);
    // SAFETY: `cols` names `body`'s live column bases; `t` is a valid tet index
    //   (`t < tet_count()`, the caller's loop bound) and its vertices `t0..t3[t]`
    //   index valid particle rows (the constructor's DegenerateTet invariant). The
    //   serial caller holds the unique `&mut SoftBody`, so no aliasing.
    unsafe { project_volume_raw(cols, t, h) };
}

/// The RAW per-element core of [`project_volume`] (SP4 W3-A soundness fix).
///
/// Identical XPBD signed-volume math, but every column access is a raw
/// `*base.add(i)` instead of a `Vec` index, so under Tree-Borrows it retags ONLY the
/// element it touches — never the whole buffer. This is what makes the
/// colored-parallel volume solve race-free: two workers writing the C2-disjoint
/// dynamic rows of one color form no overlapping element protectors. The serial
/// wrapper calls it through the unique `&mut SoftBody` (no aliasing); a colored
/// worker calls it through [`SoftCols`] built from the live body, writing only its
/// chunk's disjoint dynamic rows.
///
/// # Safety
/// `cols` must name a live `SoftBody`'s column bases; `t < tet_count()`; the
/// vertices `t0..t3[t]` must be valid particle rows `< particle_count()`. In the
/// parallel path, the caller must invoke this only on tets of ONE color whose
/// DYNAMIC vertices are pairwise disjoint across concurrent workers (the C2 lemma);
/// a SHARED PINNED row is read-only (the C1 guard never writes it). On those
/// conditions the per-element reads/writes touch only provably-disjoint elements
/// (writes) plus read-only shared columns (`inv_mass`/topology) — no UB.
#[inline]
pub(in crate::soft) unsafe fn project_volume_raw(cols: SoftCols, t: usize, h: f32) {
    // SAFETY (all the per-element accesses below): `t` and the vertex rows are in
    //   range (the fn contract); each `*p.add(i)` retags only element `i`. Reads of
    //   `inv_mass`/`t_*` are read-only shared; the `pos_*` writes are gated to the
    //   tet's own dynamic vertices (disjoint across workers — the contract).
    let i0 = unsafe { *cols.t0.add(t) } as usize;
    let i1 = unsafe { *cols.t1.add(t) } as usize;
    let i2 = unsafe { *cols.t2.add(t) } as usize;
    let i3 = unsafe { *cols.t3.add(t) } as usize;
    let p0 = unsafe { Vec3::new(*cols.pos_x.add(i0), *cols.pos_y.add(i0), *cols.pos_z.add(i0)) };
    let p1 = unsafe { Vec3::new(*cols.pos_x.add(i1), *cols.pos_y.add(i1), *cols.pos_z.add(i1)) };
    let p2 = unsafe { Vec3::new(*cols.pos_x.add(i2), *cols.pos_y.add(i2), *cols.pos_z.add(i2)) };
    let p3 = unsafe { Vec3::new(*cols.pos_x.add(i3), *cols.pos_y.add(i3), *cols.pos_z.add(i3)) };
    // Edge-anchored at p0 (FP conditioning). Pinned cross operand order; dot
    // left-to-right.
    let e1 = p1 - p0;
    let e2 = p2 - p0;
    let e3 = p3 - p0;
    let vol = (1.0 / 6.0) * e1.cross(e2).dot(e3);
    let cc = vol - unsafe { *cols.t_rest.add(t) };
    let g1 = e2.cross(e3) * (1.0 / 6.0);
    let g2 = e3.cross(e1) * (1.0 / 6.0);
    let g3 = e1.cross(e2) * (1.0 / 6.0);
    // Pinned add order ⇒ Σg == 0 exactly (g0 + g1 + g2 + g3 == 0).
    let g0 = (g1 + g2 + g3) * -1.0;
    let w0 = unsafe { *cols.inv_mass.add(i0) };
    let w1 = unsafe { *cols.inv_mass.add(i1) };
    let w2 = unsafe { *cols.inv_mass.add(i2) };
    let w3 = unsafe { *cols.inv_mass.add(i3) };
    // Summed in fixed vertex order 0,1,2,3.
    let wsum = w0 * g0.dot(g0) + w1 * g1.dot(g1) + w2 * g2.dot(g2) + w3 * g3.dot(g3);
    let alpha_tilde = unsafe { *cols.t_compliance.add(t) } / (h * h);
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
    // SP4 C1: `s` finite is the LOAD-BEARING invariant that makes the pinned-vertex
    // skip below bit-equal to `+= gᵢ*(s*±0.0)`. NOT `w >= 0.0` (see project_distance).
    debug_assert!(s.is_finite(), "invariant: volume-constraint Lagrange step must be finite");
    // Fixed vertex order; a pinned vertex (w == 0) gets no move.
    let d0 = g0 * (s * w0);
    let d1 = g1 * (s * w1);
    let d2 = g2 * (s * w2);
    let d3 = g3 * (s * w3);
    // SP4 C1 per-vertex write guard (shared by serial + colored): write only the
    // DYNAMIC vertices, removing the value-benign pinned `+= ±0.0`. Byte-preserving
    // serially.
    if is_dynamic_row(w0) {
        unsafe {
            *cols.pos_x.add(i0) += d0.x;
            *cols.pos_y.add(i0) += d0.y;
            *cols.pos_z.add(i0) += d0.z;
        }
    }
    if is_dynamic_row(w1) {
        unsafe {
            *cols.pos_x.add(i1) += d1.x;
            *cols.pos_y.add(i1) += d1.y;
            *cols.pos_z.add(i1) += d1.z;
        }
    }
    if is_dynamic_row(w2) {
        unsafe {
            *cols.pos_x.add(i2) += d2.x;
            *cols.pos_y.add(i2) += d2.y;
            *cols.pos_z.add(i2) += d2.z;
        }
    }
    if is_dynamic_row(w3) {
        unsafe {
            *cols.pos_x.add(i3) += d3.x;
            *cols.pos_y.add(i3) += d3.y;
            *cols.pos_z.add(i3) += d3.z;
        }
    }
}
