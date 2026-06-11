//! Particle integration system (plan §6.5, Particles mode).

use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::system::Res;

use crate::render::WORLD_HALF_EXTENT;
use crate::sim::components::{Position, Velocity};
use boyko_ecs::ecs::core::time::FixedTime;
use crate::sim::resources::{InputState, SimParams};

/// Advances every particle one fixed step (plan §6.5).
///
/// For each row, in this order: apply the gravity well toward the cursor (when
/// the primary button is held), damp the velocity, clamp it to `max_speed`,
/// integrate position, and bounce off the world walls. The body is branch-light
/// and touches only its own row, so `par_iter_mut` fans it out across disjoint
/// rows with no synchronization (the demo's most honest `par_iter` win,
/// plan D3 rationale).
///
/// All three resources are plain `Res<_>` reads, shared across workers for the
/// duration of the parallel pass.
pub fn integrate_particles(
    mut query: Query<(&mut Position, &mut Velocity)>,
    dt: Res<FixedTime>,
    input: Res<InputState>,
    params: Res<SimParams>,
) {
    let dt = dt.delta_secs();
    let gravity = params.gravity;
    // Per-step damping derived from the per-second retention factor.
    let damping = params.damping.powf(dt);
    let max_speed = params.max_speed;
    let max_speed_sq = max_speed * max_speed;
    let bound = WORLD_HALF_EXTENT;

    // The well is active only while the primary button is held over the scene
    // AND the panel's gravity toggle is on (plan §7). Resolve it once outside the
    // hot loop so each row sees a branch on a local.
    let well = if params.gravity_enabled && input.primary_down {
        input.cursor_world
    } else {
        None
    };

    // The closure item type is annotated explicitly: `for_each` is bound by
    // `Fn(D::Item<'_>) + Send + Sync`, and rustc cannot infer the higher-ranked
    // closure type from an un-annotated `move |(pos, vel)|` (a known GAT/HRTB
    // inference gap — it surfaces as a misleading "method not found").
    query
        .par_iter_mut()
        .for_each(move |(pos, vel): (&mut Position, &mut Velocity)| {
            if let Some([wx, wy]) = well {
                let dx = wx - pos.x;
                let dy = wy - pos.y;
                // Inverse-distance pull, softened near the cursor so the
                // acceleration stays finite at the singularity.
                let dist_sq = dx * dx + dy * dy + 1.0;
                let inv_dist = dist_sq.sqrt().recip();
                let accel = gravity * dt * inv_dist;
                vel.x += dx * inv_dist * accel;
                vel.y += dy * inv_dist * accel;
            }

            vel.x *= damping;
            vel.y *= damping;

            // Clamp speed without a per-row branch on the common case: only
            // rescale when above the cap.
            let speed_sq = vel.x * vel.x + vel.y * vel.y;
            if speed_sq > max_speed_sq {
                let scale = max_speed * speed_sq.sqrt().recip();
                vel.x *= scale;
                vel.y *= scale;
            }

            pos.x += vel.x * dt;
            pos.y += vel.y * dt;

            // Reflect off the world walls so the cloud stays in the box.
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

