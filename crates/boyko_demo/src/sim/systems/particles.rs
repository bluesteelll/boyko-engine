//! Particle integration system (plan §6.5, Particles mode).

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::iters::query::Disabled;
use boyko_ecs::ecs::core::system::Res;

use crate::render::WORLD_HALF_EXTENT;
use crate::sim::components::{Frozen, ParticleTag, Position, Velocity};
use boyko_ecs::ecs::core::time::FixedTime;
use crate::sim::resources::{InputState, SimParams};

/// Stride of the frozen subset: every `FREEZE_STRIDE`-th particle (in
/// query-entity order) carries the [`Frozen`] enable-bit. A coarse stride keeps
/// the visible frozen set a thin, legible lattice without freezing a large
/// fraction of the cloud.
const FREEZE_STRIDE: usize = 64;

/// Advances every NON-FROZEN particle one fixed step (plan §6.5; EnableTag
/// Wave 6 / Step 11 dogfood).
///
/// For each row, in this order: apply the gravity well toward the cursor (when
/// the primary button is held), damp the velocity, clamp it to `max_speed`,
/// integrate position, and bounce off the world walls. The body is branch-light
/// and touches only its own row, so `par_iter_mut` fans it out across disjoint
/// rows with no synchronization (the demo's most honest `par_iter` win,
/// plan D3 rationale).
///
/// The query filter is [`Disabled<Frozen>`]: a particle whose [`Frozen`]
/// enable-bit is SET is skipped by the integrator and holds its position, while
/// the rest of the cloud flows. The per-row bit is toggled O(1) by
/// [`freeze_pulse`] with no
/// archetype migration — the enable-bit tag backend's headline property. The
/// per-row `filter_fetch` gate composes with `par_iter_mut` (the parallel path
/// applies the same per-row enable test the sequential walk does), and with the
/// wasm sequential fallback unchanged.
///
/// All three resources are plain `Res<_>` reads, shared across workers for the
/// duration of the parallel pass.
pub fn integrate_particles(
    mut query: Query<(&mut Position, &mut Velocity), Disabled<Frozen>>,
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

/// Refreshes the [`Frozen`] enable-bit on the particle cloud each step
/// (EnableTag Wave 6 / Step 11 dogfood — the O(1) toggle in a real frame loop).
///
/// EXCLUSIVE `fn(&mut EcsMaster)`: [`enable`](EcsMaster::enable) /
/// [`disable`](EcsMaster::disable) take `&mut self`, so a body that toggles the
/// bit directly on the world must be an exclusive system (universal access), the
/// same shape the mode spawn/despawn systems use. The runner gates it
/// `.run_if(in_state(Mode::Particles))` and orders it before
/// [`integrate_particles`], so each step the integrator sees a stable frozen
/// set.
///
/// Every `FREEZE_STRIDE`-th particle (in `query_entities` order) is frozen and
/// the rest are cleared. The toggle is the enable-bit backend's headline
/// operation: a per-row bit flip at `(archetype, row)` with NO archetype
/// migration, NO structural-generation bump, NO hook/observer fire, and NO
/// per-row bytes — the non-fragmenting alternative to a fragmenting `Frozen`
/// data component. `integrate_particles`'s `Disabled<Frozen>` filter then skips
/// exactly the frozen rows, so they hold position while the cloud flows.
///
/// `query_entities` is a `&self` archetype scan, so its result is collected
/// before the `&mut self` toggle loop (the same two-statement borrow split the
/// despawn systems use). The cost is bounded: the scan is over the particle
/// archetype, and the toggle is O(1) per entity.
//
// `clippy::needless_pass_by_ref_mut`: `enable`/`disable` are `&mut self`, so the
// `&mut EcsMaster` IS required — but the lint cannot see through the cross-crate
// method calls and false-positively suggests `&EcsMaster`, which would not
// compile. Allowed with this justification (mirrors `modes::despawn_tagged`).
#[allow(clippy::needless_pass_by_ref_mut)]
pub fn freeze_pulse(world: &mut EcsMaster) {
    let particles = world.query_entities(&[ParticleTag::component_id()]);
    for (i, entity) in particles.into_iter().enumerate() {
        if i % FREEZE_STRIDE == 0 {
            world.enable::<Frozen>(entity);
        } else {
            world.disable::<Frozen>(entity);
        }
    }
}

