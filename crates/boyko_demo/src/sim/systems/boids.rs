//! The boids (flocking) pipeline (plan §6.5 Boids / D11 / D12 / Wave 5).
//!
//! Four systems run in order each Boids-mode step:
//!
//! 1. [`snapshot_boids`] — copy every boid's `(pos, vel)` into the
//!    [`BoidSnapshot`] resource (the pre-tick state, D12).
//! 2. [`build_grid`] — counting-sort the snapshot positions into the
//!    [`SpatialGrid`] (D11).
//! 3. [`boid_forces`] — `par_iter_mut` over `&mut Velocity`, reading the
//!    snapshot + grid read-only, applying separation / alignment / cohesion from
//!    the 3x3 neighbor block.
//! 4. [`integrate_boids`] — `par_iter_mut`, `pos += vel*dt`, clamp to the box.
//!
//! Why the snapshot (D12): each boid reads its neighbors' *previous-frame* state.
//! Reading the live `Velocity`/`Position` columns while a sibling worker writes
//! them is a data race; a read-only snapshot makes the force pass a sound
//! `par_iter` — the snapshot + grid are shared reads (broadcast to every worker)
//! and each boid writes only its own `Velocity` row (disjoint). This is the same
//! `par_iter_mut` + `Res<_>` shape `sync_gpu_instance` already uses.

use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::system::{Res, ResMut};

use crate::render::WORLD_HALF_EXTENT;
use crate::sim::components::{Position, Velocity};
use crate::sim::grid::SpatialGrid;
use crate::sim::resources::{BoidParams, BoidSnapshot, BoidState, DeltaTime};

/// Below this squared distance a candidate neighbor is treated as "self" and
/// skipped. The snapshot has no per-boid identity the force pass can match, so a
/// boid excludes itself (and exact-coincident others) by distance. Small enough
/// that genuinely distinct boids are never dropped.
const SELF_EPSILON_SQ: f32 = 1e-6;

/// Copies every boid's `(Position, Velocity)` into the [`BoidSnapshot`] (plan
/// D12), in archetype row order.
///
/// Sequential and allocation-free in steady state: the snapshot `Vec` is cleared
/// and refilled (its capacity is reused). `for_each_chunk` yields each
/// archetype's contiguous `Position`/`Velocity` columns, appended in the same
/// order the grid will index and the force pass will iterate.
pub fn snapshot_boids(
    mut query: Query<(&Position, &Velocity)>,
    mut snapshot: ResMut<BoidSnapshot>,
) {
    // Explicit type so the `&mut snapshot.state` field access deref-coerces
    // through `ResMut`'s `DerefMut` to `&mut Vec<BoidState>` (without the
    // annotation rustc keeps `state` typed as the `ResMut` wrapper).
    let state: &mut Vec<BoidState> = &mut snapshot.state;
    state.clear();
    query.for_each_chunk(|(positions, velocities): (&[Position], &[Velocity])| {
        for (p, v) in positions.iter().zip(velocities) {
            state.push(BoidState {
                pos: [p.x, p.y],
                vel: [v.x, v.y],
            });
        }
    });
}

/// Rebuilds the [`SpatialGrid`] from the boid snapshot (plan §6.4 / D11).
///
/// Runs after [`snapshot_boids`] and before [`boid_forces`] — the sole grid
/// writer that frame, so it never conflicts with the read-only force pass. The
/// cell size tracks the neighbor radius (the UI may change it); `set_cell_size`
/// is a no-op when the rounded cell count is unchanged, so it is cheap to call
/// every frame.
///
/// The snapshot is AoS (`BoidState { pos, vel }`), so there is no contiguous
/// positions slice to hand `rebuild`; `rebuild_with` bins the `pos` field
/// directly via a closure, materializing no temporary array.
pub fn build_grid(
    snapshot: Res<BoidSnapshot>,
    params: Res<BoidParams>,
    mut grid: ResMut<SpatialGrid>,
) {
    grid.set_cell_size(params.radius);
    let state = snapshot.state.as_slice();
    grid.rebuild_with(state.len(), |i| state[i].pos);
}

/// Applies separation / alignment / cohesion to every boid (plan §6.5 Boids).
///
/// A sound `par_iter_mut` (D12): the snapshot, grid, params, and dt are shared
/// reads broadcast to every worker; each boid writes only its own `Velocity`
/// row. For each boid it walks the 3x3 grid block around its position, reads
/// neighbors from the snapshot (previous-frame state), accumulates the three
/// steering terms, then nudges and clamps its velocity. The closure item type is
/// annotated explicitly (the GAT/HRTB inference gap `sync_gpu_instance`
/// documents).
pub fn boid_forces(
    mut query: Query<(&Position, &mut Velocity)>,
    snapshot: Res<BoidSnapshot>,
    grid: Res<SpatialGrid>,
    params: Res<BoidParams>,
    dt: Res<DeltaTime>,
) {
    let dt = dt.0;
    let radius_sq = params.radius * params.radius;
    let sep_w = params.separation;
    let align_w = params.alignment;
    let coh_w = params.cohesion;
    let max_speed = params.max_speed;
    let max_speed_sq = max_speed * max_speed;
    let state = snapshot.state.as_slice();
    let grid = &*grid;

    query
        .par_iter_mut()
        .for_each(move |(pos, vel): (&Position, &mut Velocity)| {
            let px = pos.x;
            let py = pos.y;

            // Accumulators for the three flocking terms over in-radius neighbors.
            let mut sep = [0.0f32, 0.0f32]; // sum of away-vectors (separation)
            let mut vel_sum = [0.0f32, 0.0f32]; // sum of neighbor velocities (align)
            let mut pos_sum = [0.0f32, 0.0f32]; // sum of neighbor positions (cohesion)
            let mut count = 0u32;

            grid.for_each_neighbor(px, py, |idx| {
                let n = state[idx as usize];
                let dx = px - n.pos[0];
                let dy = py - n.pos[1];
                let dist_sq = dx * dx + dy * dy;
                // Skip self (and exact-coincident points) and out-of-radius.
                if dist_sq < SELF_EPSILON_SQ || dist_sq > radius_sq {
                    return;
                }
                // Separation: push away, weighted by inverse distance so closer
                // neighbors push harder. `dist_sq` is finite and > epsilon here.
                let inv = dist_sq.sqrt().recip();
                sep[0] += dx * inv;
                sep[1] += dy * inv;
                // Alignment + cohesion accumulate neighbor vel/pos.
                vel_sum[0] += n.vel[0];
                vel_sum[1] += n.vel[1];
                pos_sum[0] += n.pos[0];
                pos_sum[1] += n.pos[1];
                count += 1;
            });

            if count > 0 {
                let inv_n = (count as f32).recip();
                // Alignment: steer toward the neighbors' average velocity.
                let align_x = vel_sum[0] * inv_n - vel.x;
                let align_y = vel_sum[1] * inv_n - vel.y;
                // Cohesion: steer toward the neighbors' average position.
                let coh_x = pos_sum[0] * inv_n - px;
                let coh_y = pos_sum[1] * inv_n - py;

                vel.x += (sep[0] * sep_w + align_x * align_w + coh_x * coh_w) * dt;
                vel.y += (sep[1] * sep_w + align_y * align_w + coh_y * coh_w) * dt;
            }

            // Clamp to max speed (only rescale when over the cap — branch-light).
            let speed_sq = vel.x * vel.x + vel.y * vel.y;
            if speed_sq > max_speed_sq {
                let scale = max_speed * speed_sq.sqrt().recip();
                vel.x *= scale;
                vel.y *= scale;
            }
        });
}

/// Integrates boid positions and keeps them in the world box (plan §6.5 Boids).
///
/// `par_iter_mut` over disjoint rows: `pos += vel*dt`, then bounce off the walls
/// (clamp the position AND reflect the velocity) so the flock stays in
/// `[-WORLD_HALF_EXTENT, WORLD_HALF_EXTENT]^2`. Reflecting (rather than wrapping)
/// avoids neighbors teleporting across the box, which would confuse the grid's
/// locality; reflecting velocity (not just clamping position) stops boids from
/// pinning against a wall with their heading still pointing out of bounds.
///
/// Velocity is `&mut` here because the wall bounce reverses it. The force pass
/// (the other `Velocity` writer) runs earlier and finishes before this system
/// starts (it is ordered after it), so there is no write-write conflict.
pub fn integrate_boids(mut query: Query<(&mut Position, &mut Velocity)>, dt: Res<DeltaTime>) {
    let dt = dt.0;
    let bound = WORLD_HALF_EXTENT;
    query
        .par_iter_mut()
        .for_each(move |(pos, vel): (&mut Position, &mut Velocity)| {
            pos.x += vel.x * dt;
            pos.y += vel.y * dt;
            if pos.x < -bound {
                pos.x = -bound;
                vel.x = -vel.x;
            } else if pos.x > bound {
                pos.x = bound;
                vel.x = -vel.x;
            }
            if pos.y < -bound {
                pos.y = -bound;
                vel.y = -vel.y;
            } else if pos.y > bound {
                pos.y = bound;
                vel.y = -vel.y;
            }
        });
}
