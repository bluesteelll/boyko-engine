//! Soft↔rigid two-way coupling for the XPBD soft-body pass (Physics O11 SP2,
//! plan D6/D7) — opt-in via
//! [`PhysicsConfig::soft_rigid_coupling`](crate::resources::PhysicsConfig).
//!
//! On the coupling-wired path the soft step ([`physics_soft_step_coupled`](
//! crate::soft::physics_soft_step_coupled)) resolves, per movable particle, the
//! SINGLE deepest soft-vs-rigid contact against the rigid bodies' frame-N snapshot
//! and (a) pushes the particle out of the rigid shape, recording the pre-push
//! position so the velocity update excludes the coupling push (D4), and (b)
//! accumulates the equal-and-opposite rigid REACTION into [`SoftRigidReaction`]
//! (D7). The reaction is applied to the [`RigidBody`](crate::components::RigidBody)
//! COMPONENT after `physics_apply` by [`physics_soft_rigid_apply`], like an external
//! force — so the rigid scratch and the gather are never mutated by the soft pass
//! (IM-1 safety preserved).
//!
//! # Determinism boundary (INVIOLABLE)
//!
//! Every floating-point operation here is EXACT `mul`/`add`/`sub`/`div`/`sqrt` —
//! NO `rsqrt`/`rcp`/`mul_add`/FMA-contraction, NO [`Vec3::normalize`] (the sphere
//! query normalizes with an explicit `d * (1.0 / dist)` past a
//! [`LEN_EPS`](crate::soft::solver::LEN_EPS) guard). The per-particle contact is
//! the deepest penetration, tie-broken by ASCENDING [`BodyIndex`](
//! crate::manifold::BodyIndex), so a body bucketed in several neighbour cells (or
//! also present in the oversized list) dedups to one deterministic choice. The
//! reaction reads the rigid body from the SAME frame-N snapshot
//! ([`SolverScratch::bodies`](crate::resources::SolverScratch)) the rigid solve
//! consumed (same-frame symmetric exchange, no lag).

use boyko_ecs::ecs::core::iters::query::data::Mut;
use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::system::ResMut;
use boyko_macros::Resource;

use crate::components::{ColliderShape, RigidBody};
use crate::math::Vec3;
use crate::resources::{BodyState, BroadphaseGrid};
use crate::soft::component::SoftBody;
use crate::soft::solver::LEN_EPS;

/// Per-body accumulated soft→rigid reaction (SP2 D7), keyed by dense BodyIndex
/// (the snapshot row).
///
/// Two dense columns — linear `Δv` and angular `Δω` — one row per rigid body in
/// the SAME order [`SolverScratch::bodies`](crate::resources::SolverScratch) /
/// `physics_apply` walk. The buffers are RESERVED to body capacity at wire-up and
/// CLEARED (not resized) at the start of each coupled soft step
/// ([`reset`](Self::reset)), so the coupling path does ZERO per-step heap
/// allocation in steady state. The reaction lands on the
/// [`RigidBody`](crate::components::RigidBody) component AFTER `physics_apply` (by
/// [`physics_soft_rigid_apply`]), like an external impulse — the rigid scratch is
/// never touched by the soft pass.
#[derive(Resource, Default)]
pub struct SoftRigidReaction {
    /// Per-body accumulated linear velocity delta (`Σ p_imp · −inv_mass`).
    dv_lin: Vec<Vec3>,
    /// Per-body accumulated angular velocity delta (`Σ inv_inertia · (r × −p_imp)`).
    dv_ang: Vec<Vec3>,
}

impl SoftRigidReaction {
    /// Builds the reaction accumulator pre-sized for up to `rows` bodies (no later
    /// reallocation in steady state).
    pub fn with_capacity(rows: usize) -> Self {
        Self {
            dv_lin: Vec::with_capacity(rows),
            dv_ang: Vec::with_capacity(rows),
        }
    }

    /// Resets both columns to `rows` zero entries for a fresh frame, reusing
    /// capacity (clear + zero-fill; no realloc once warmed).
    #[inline]
    pub fn reset(&mut self, rows: usize) {
        self.dv_lin.clear();
        self.dv_ang.clear();
        self.dv_lin.resize(rows, Vec3::ZERO);
        self.dv_ang.resize(rows, Vec3::ZERO);
    }

    /// Zeroes both columns IN PLACE, keeping the current length (SP2 M2
    /// clear-after-consume).
    ///
    /// Unlike [`reset`](Self::reset) (which resizes to a fresh row count), this only
    /// re-zeros the existing rows — used by [`physics_soft_rigid_apply`] after it has
    /// landed the reaction, so a frame producing no fresh reaction cannot re-apply a
    /// stale one. No realloc, no shape change.
    #[inline]
    pub fn clear_values(&mut self) {
        self.dv_lin.iter_mut().for_each(|v| *v = Vec3::ZERO);
        self.dv_ang.iter_mut().for_each(|v| *v = Vec3::ZERO);
    }

    /// Number of body rows currently accumulated (the column length).
    #[inline]
    pub fn len(&self) -> usize {
        self.dv_lin.len()
    }

    /// `true` when no body rows are accumulated.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.dv_lin.is_empty()
    }

    /// Accumulates a reaction into body row `idx` (the linear + angular deltas).
    #[inline]
    fn accumulate(&mut self, idx: usize, dv_lin: Vec3, dv_ang: Vec3) {
        self.dv_lin[idx] = self.dv_lin[idx] + dv_lin;
        self.dv_ang[idx] = self.dv_ang[idx] + dv_ang;
    }
}

/// The read-only rigid context + the reaction sink the coupled soft step threads
/// into [`resolve_coupling`] (SP2 D6/D7).
///
/// `bodies` is the frame-N rigid snapshot (READ-ONLY — never mutated by the soft
/// pass), `grid` the broadphase acceleration structure for the neighbourhood walk,
/// and `reaction` the per-body accumulator the D7 reaction writes.
pub struct CouplingCtx<'a> {
    /// The rigid bodies' frame-N snapshot (read-only).
    pub bodies: &'a [BodyState],
    /// The broadphase grid for the 27-cell neighbourhood query.
    pub grid: &'a BroadphaseGrid,
    /// The per-body reaction accumulator (written by D7).
    pub reaction: &'a mut SoftRigidReaction,
}

/// One deepest soft-vs-rigid contact for a particle (SP2 D6 local slot).
#[derive(Clone, Copy)]
struct Contact {
    /// Penetration depth (`> 0`).
    depth: f32,
    /// The contact normal (unit, pointing OUT of the rigid shape toward the
    /// particle).
    normal: Vec3,
    /// The contact point on the rigid surface (where the reaction is applied).
    point: Vec3,
    /// The rigid body row (dense BodyIndex).
    body: u32,
}

/// Resolves the SINGLE deepest soft-vs-rigid contact for every movable particle of
/// `body`, pushing the particle out and recording the D7 reaction (SP2 D6/D7).
///
/// Runs between the volume sweep and the SDF collide, in fixed particle order. The
/// `coupling_hit` flags are reset to `0` first, then for a particle with a contact:
/// records the pre-push position into `coupling_prev_*` (D4 — so the velocity update
/// excludes the coupling push), pushes the particle out by the penetration depth
/// along the contact normal, sets `coupling_hit = 1`, and (for a DYNAMIC rigid body)
/// records the D7 particle impulse into `coupling_dv_*` AND accumulates the
/// equal-and-opposite rigid reaction into [`SoftRigidReaction`].
///
/// The D7 particle impulse is DEFERRED to the velocity update (via `coupling_dv_*`)
/// rather than applied to `vel` here, so the SP1 position-diff velocity overwrite
/// does not wipe it: the velocity update then sets a coupled particle's velocity to
/// the D4 baseline `(coupling_prev − prev)·inv_h` PLUS the D7 delta (the momentum
/// exchange is carried by the D7 reaction, not the position diff).
pub fn resolve_coupling(body: &mut SoftBody, ctx: &mut CouplingCtx<'_>, inv_h: f32) {
    let n = body.particle_count();
    let radius = body.particle_radius;
    // Reset the per-substep "pushed" flags (the velocity update reads them to pick
    // the coupled baseline; a particle not pushed THIS substep must read `0`).
    for h in body.coupling_hit.iter_mut() {
        *h = 0;
    }
    for i in 0..n {
        if body.inv_mass[i] == 0.0 {
            // Pinned particle — frozen (matches the SP1 collide / velocity guards).
            continue;
        }
        let particle = Vec3::new(body.pos_x[i], body.pos_y[i], body.pos_z[i]);
        let Some(contact) = deepest_contact(ctx.bodies, ctx.grid, particle, radius) else {
            continue;
        };

        // D4: record the pre-push position so the velocity update can exclude the
        // coupling push (the momentum exchange is carried by the D7 reaction).
        body.coupling_prev_x[i] = body.pos_x[i];
        body.coupling_prev_y[i] = body.pos_y[i];
        body.coupling_prev_z[i] = body.pos_z[i];
        body.coupling_hit[i] = 1;
        // Clear the deferred D7 delta (overwritten below for a dynamic body; left
        // zero for a static contact ⇒ baseline-only velocity).
        body.coupling_dv_x[i] = 0.0;
        body.coupling_dv_y[i] = 0.0;
        body.coupling_dv_z[i] = 0.0;

        // Push the particle out along the contact normal by the penetration depth.
        let push = contact.normal * contact.depth;
        body.pos_x[i] += push.x;
        body.pos_y[i] += push.y;
        body.pos_z[i] += push.z;

        // D7: the velocity-constraint reaction. The particle's pre-collision
        // velocity is the D4 baseline `(coupling_prev - prev) * inv_h` (the SP1
        // substep velocity EXCLUDING this coupling push).
        let w_particle = body.inv_mass[i];
        let prev = Vec3::new(body.prev_x[i], body.prev_y[i], body.prev_z[i]);
        let pre_push = Vec3::new(
            body.coupling_prev_x[i],
            body.coupling_prev_y[i],
            body.coupling_prev_z[i],
        );
        let v_particle = (pre_push - prev) * inv_h;
        apply_reaction(ctx, &contact, v_particle, w_particle, body, i);
    }
}

/// The velocity-constraint reaction for one contact (SP2 D7) — IM-1-safe.
///
/// For a DYNAMIC rigid body (`inv_mass > 0`) approaching the particle along the
/// normal, applies one normal velocity impulse: it records the PARTICLE'S impulse
/// `p_imp · w_particle` into `coupling_dv_*` (deferred to the velocity update so the
/// position-diff overwrite does not wipe it) and accumulates the equal-and-opposite
/// rigid reaction into [`SoftRigidReaction`]. A static body (`inv_mass == 0` ⇒
/// `inv_inertia == Mat3::ZERO`) contributes a zero reaction; the
/// `inv_mass > 0` early-out also skips writing a static body's reaction row (its
/// `coupling_dv_*` stays zero ⇒ a baseline-only velocity).
#[inline]
fn apply_reaction(
    ctx: &mut CouplingCtx<'_>,
    contact: &Contact,
    v_particle: Vec3,
    w_particle: f32,
    body: &mut SoftBody,
    i: usize,
) {
    let rb = &ctx.bodies[contact.body as usize];
    if rb.inv_mass <= 0.0 {
        // Static / immovable rigid body — one-sided push only, zero reaction.
        return;
    }
    let r = contact.point - rb.position;
    let n = contact.normal;
    // The rigid material-point velocity (frame-N snapshot): v + ω × r.
    let v_b = rb.linear_velocity + rb.angular_velocity.cross(r);
    let v_rel_n = (v_particle - v_b).dot(n);
    if v_rel_n >= 0.0 {
        // Separating (or grazing) — no impulse (one-sided velocity constraint).
        return;
    }
    // Angular effective-mass term — the EXACT `effective_mass` form (contact.rs):
    // `n · ((I⁻¹ · (r × n)) × r)`.
    let rd = r.cross(n);
    let ang = n.dot(rb.inv_inertia.mul_vec(rd).cross(r));
    let k = w_particle + rb.inv_mass + ang;
    debug_assert!(
        k > 0.0,
        "invariant: coupling reaction denom k must be > 0 (dynamic body + positive particle mass)"
    );
    let j = -v_rel_n / k;
    let p_imp = n * j;
    // Defer the particle's velocity gain to the velocity update (the soft side gains
    // `p_imp * w_particle`).
    let dvp = p_imp * w_particle;
    body.coupling_dv_x[i] = dvp.x;
    body.coupling_dv_y[i] = dvp.y;
    body.coupling_dv_z[i] = dvp.z;
    // The equal-and-opposite rigid reaction (`-p_imp`), accumulated for the
    // post-apply write. `p_imp * -1.0` keeps the explicit op (no `Neg`).
    let p_back = p_imp * -1.0;
    let dv_lin = p_back * rb.inv_mass;
    let dv_ang = rb.inv_inertia.mul_vec(r.cross(p_back));
    ctx.reaction.accumulate(contact.body as usize, dv_lin, dv_ang);
}

/// Finds the SINGLE deepest soft-vs-rigid contact for `particle` (radius
/// `radius`), or `None` (SP2 D6).
///
/// Queries the 27-cell neighbourhood (in-place CSR slice iteration, NO temporary
/// `Vec`) plus the oversized list as a separate pass, testing each candidate rigid
/// body's shape. Keeps the deepest penetrating contact (strict `>` on depth,
/// tie-broken by ASCENDING [`BodyIndex`](crate::manifold::BodyIndex)) — so a body
/// bucketed in several neighbour cells (or also in the oversized list) dedups to
/// one deterministic choice.
fn deepest_contact(
    bodies: &[BodyState],
    grid: &BroadphaseGrid,
    particle: Vec3,
    radius: f32,
) -> Option<Contact> {
    let mut best: Option<Contact> = None;
    let dims = grid.dims();
    // Decompose the particle's linear cell index into its row-major coordinate
    // (`x + dims.x·(y + dims.y·z)`), so the neighbourhood walks coordinates without
    // a private coord accessor.
    let cell = grid.cell_of(particle);
    let dx_dim = dims[0];
    let dy_dim = dims[1];
    let cx = cell % dx_dim;
    let cy = (cell / dx_dim) % dy_dim;
    let cz = cell / (dx_dim * dy_dim);

    // 27-cell neighbourhood (the particle's cell ± 1 per axis), clamped to the
    // grid bounds.
    for oz in -1i32..=1 {
        let z = cz as i32 + oz;
        if z < 0 || z >= dims[2] as i32 {
            continue;
        }
        for oy in -1i32..=1 {
            let y = cy as i32 + oy;
            if y < 0 || y >= dims[1] as i32 {
                continue;
            }
            for ox in -1i32..=1 {
                let x = cx as i32 + ox;
                if x < 0 || x >= dims[0] as i32 {
                    continue;
                }
                // Re-flatten the neighbour coordinate (the same row-major order
                // `cell_of` produces).
                let ncell = x as u32 + dx_dim * (y as u32 + dy_dim * z as u32);
                for &b in grid.cell_body_slice(ncell) {
                    consider(bodies, b, particle, radius, &mut best);
                }
            }
        }
    }

    // Oversized bodies (never bucketed) — a separate pass.
    for &b in grid.oversized_slice() {
        consider(bodies, b, particle, radius, &mut best);
    }

    best
}

/// Tests rigid body row `b` against the particle and updates `best` if it is a
/// deeper contact (strict `>`), tie-broken by ascending row (SP2 D6).
#[inline]
fn consider(
    bodies: &[BodyState],
    b: u32,
    particle: Vec3,
    radius: f32,
    best: &mut Option<Contact>,
) {
    let Some(contact) = query_shape(&bodies[b as usize], b, particle, radius) else {
        return;
    };
    let take = match best {
        None => true,
        Some(cur) => {
            // Deeper wins; on an exact depth tie the lower BodyIndex wins (the
            // candidates are visited cell-by-cell, so the tie-break must be
            // explicit, not first-seen).
            contact.depth > cur.depth || (contact.depth == cur.depth && contact.body < cur.body)
        }
    };
    if take {
        *best = Some(contact);
    }
}

/// Queries one rigid body's shape against the particle sphere, returning the
/// contact (normal out of the rigid shape, surface point, penetration depth) or
/// `None` when not penetrating (SP2 D6).
///
/// Exact `sqrt`/`divide` only — no [`Vec3::normalize`]. Sphere: the center-line
/// normal past a [`LEN_EPS`] guard. Box-OBB: the sphere center is rotated into the
/// body's local frame, clamped to the half-extents; an OUTSIDE center uses the
/// clamp delta as the normal, an INSIDE center the minimum-penetration face axis.
fn query_shape(rb: &BodyState, row: u32, particle: Vec3, radius: f32) -> Option<Contact> {
    match rb.shape {
        ColliderShape::Sphere { radius: body_r } => {
            let d = particle - rb.position;
            let dist = d.length();
            let pen = (body_r + radius) - dist;
            if pen <= 0.0 || dist < LEN_EPS {
                // Non-penetrating, or a coincident center (no usable normal).
                return None;
            }
            // Explicit divide (NOT `rsqrt`, NOT `normalize`).
            let normal = d * (1.0 / dist);
            // The contact point on the rigid sphere surface (where the reaction is
            // applied): `position + normal · body_r`.
            let point = rb.position + normal * body_r;
            Some(Contact {
                depth: pen,
                normal,
                point,
                body: row,
            })
        }
        ColliderShape::Box { half_extents } => {
            // Rotate the particle into the box's LOCAL frame.
            let local = rb.rotation.inverse_rotate(particle - rb.position);
            let clamped = local.clamp_symmetric(half_extents);
            let delta = local - clamped;
            let dist_sq = delta.length_squared();
            if dist_sq > radius * radius {
                // The closest local point is farther than the particle radius — no
                // contact (outside the inflated box).
                return None;
            }
            if dist_sq > 0.0 {
                // OUTSIDE (or on an edge/face of) the box: the clamp delta is the
                // outward direction. `dist > 0`, so the divide is well-defined.
                let dist = dist_sq.sqrt();
                if dist < LEN_EPS {
                    return None;
                }
                let local_normal = delta * (1.0 / dist);
                let pen = radius - dist;
                if pen <= 0.0 {
                    return None;
                }
                // The local surface point is the clamped center; rotate both the
                // normal and the point back to world.
                let normal = rb.rotation.rotate(local_normal);
                let point = rb.position + rb.rotation.rotate(clamped);
                Some(Contact {
                    depth: pen,
                    normal,
                    point,
                    body: row,
                })
            } else {
                // INSIDE the box (`local == clamped`): pick the minimum-penetration
                // face axis. The distance to each `±half_extent` face is
                // `half - |local_axis|`; the smallest is the shallowest exit.
                let dist_x = half_extents.x - local.x.abs();
                let dist_y = half_extents.y - local.y.abs();
                let dist_z = half_extents.z - local.z.abs();
                let (min_dist, axis) = if dist_x <= dist_y && dist_x <= dist_z {
                    (dist_x, 0)
                } else if dist_y <= dist_z {
                    (dist_y, 1)
                } else {
                    (dist_z, 2)
                };
                // The outward face normal in local frame (sign of the local axis;
                // a zero axis pushes toward +).
                let local_normal = match axis {
                    0 => Vec3::new(if local.x >= 0.0 { 1.0 } else { -1.0 }, 0.0, 0.0),
                    1 => Vec3::new(0.0, if local.y >= 0.0 { 1.0 } else { -1.0 }, 0.0),
                    _ => Vec3::new(0.0, 0.0, if local.z >= 0.0 { 1.0 } else { -1.0 }),
                };
                // The particle is fully inside: penetration is the exit distance
                // PLUS the particle radius (push the whole sphere out).
                let pen = min_dist + radius;
                let normal = rb.rotation.rotate(local_normal);
                // The surface point: the local center projected onto the chosen
                // face.
                let mut surface_local = local;
                match axis {
                    0 => {
                        surface_local.x = if local.x >= 0.0 {
                            half_extents.x
                        } else {
                            -half_extents.x
                        }
                    }
                    1 => {
                        surface_local.y = if local.y >= 0.0 {
                            half_extents.y
                        } else {
                            -half_extents.y
                        }
                    }
                    _ => {
                        surface_local.z = if local.z >= 0.0 {
                            half_extents.z
                        } else {
                            -half_extents.z
                        }
                    }
                }
                let point = rb.position + rb.rotation.rotate(surface_local);
                Some(Contact {
                    depth: pen,
                    normal,
                    point,
                    body: row,
                })
            }
        }
    }
}

/// Writes the accumulated soft→rigid reaction onto the
/// [`RigidBody`](crate::components::RigidBody) column AFTER `physics_apply` (SP2
/// D7 apply path).
///
/// Registered `.after(apply)` ONLY on the coupling-wired path. Walks the SAME row↔
/// body order `physics_apply` uses (`iter_mut().enumerate()` → row = BodyIndex),
/// deref-writing `linear_velocity += dv_lin[row]` / `angular_velocity +=
/// dv_ang[row]` through the [`Mut`] guard (so the row's `changed` tick bumps, like
/// an external force). The reaction lands on the component POST-apply: next frame's
/// gather re-projects it into the scratch cleanly (IM-1) — the scratch and the
/// gather are never mutated by the soft pass.
///
/// # Stale-reaction safety (SP2 M2)
///
/// The buffer is CLEARED HERE after it is consumed (clear-after-consume), not left
/// for the next coupled step's [`SoftRigidReaction::reset`] to zero. The producer's
/// `reset()` runs only when [`physics_soft_step_coupled`](
/// crate::soft::physics_soft_step_coupled) does NOT early-return — but it early-returns
/// before `reset()` when [`PhysicsConfig::soft_body`](crate::resources::PhysicsConfig)
/// is toggled OFF at runtime while the coupling stages stay registered. Relying on the
/// producer to re-zero would then re-apply this frame's reaction every frame (a
/// phantom recurring impulse). Zeroing the columns here makes the apply
/// self-consistent: a frame with no fresh reaction (whatever the producer's state)
/// applies nothing. The producer's `reset()` still runs each coupled frame (it
/// resizes the columns to the live snapshot row count), so this clear is the robust
/// floor, not a replacement for it.
//
// `clippy::needless_pass_by_value`: `ResMut<_>` is a by-value `SystemParam`
// mutated through a reborrow — the same false-positive the rigid systems document.
#[allow(clippy::needless_pass_by_value)]
pub fn physics_soft_rigid_apply(
    mut query: Query<Mut<RigidBody>>,
    mut reaction: ResMut<SoftRigidReaction>,
) {
    let reaction = &mut *reaction;
    let mut row = 0usize;
    for mut body in query.iter_mut() {
        if row < reaction.dv_lin.len() {
            let dl = reaction.dv_lin[row];
            let da = reaction.dv_ang[row];
            // Only deref-write (bumping the changed tick) when there is a reaction,
            // so an uncoupled body is not spuriously marked changed.
            if dl != Vec3::ZERO || da != Vec3::ZERO {
                let mut next = *body;
                next.linear_velocity = next.linear_velocity + dl;
                next.angular_velocity = next.angular_velocity + da;
                *body = next;
            }
        }
        row += 1;
    }
    debug_assert!(
        reaction.dv_lin.is_empty() || row == reaction.dv_lin.len(),
        "invariant: soft-rigid apply walks exactly the snapshot row count (live rows {} != reaction len {})",
        row,
        reaction.dv_lin.len()
    );
    // SP2 M2: clear-after-consume. Zero the columns now the reaction has been
    // landed, so a subsequent frame that produces NO fresh reaction (e.g.
    // `soft_body` toggled off at runtime, which makes the coupled step early-return
    // before its `reset()`) cannot re-apply this frame's impulse. Length is kept
    // (the next coupled step's `reset()` resizes to the live row count); the columns
    // are simply re-zeroed in place — no realloc, no shape change.
    reaction.clear_values();
}
