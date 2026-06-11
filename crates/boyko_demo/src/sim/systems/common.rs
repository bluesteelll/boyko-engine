//! Systems shared across every mode (plan §6.5 "Common").

use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::system::Res;

use crate::render::instance::GpuInstance;
use crate::sim::components::{Position, Velocity};
use crate::sim::resources::SimParams;

/// Speed (world units/s) mapped to the top of the color ramp. Velocities at or
/// above this render fully "hot".
const COLOR_SPEED_MAX: f32 = 200.0;

/// Packs `GpuInstance` from the sim state every substep (plan D3, Phase 20.1
/// D2) and maintains the interpolation pair: the OLD `gpu.pos` (the previous
/// substep's packed position) is shuffled into `prev_pos` before the full 24 B
/// record is rewritten, so the GPU lerp `mix(prev_pos, pos, alpha)` always
/// spans exactly one substep.
///
/// Runs after integration and before the upload. It is a streaming SoA->SoA
/// write — sequential read of `Position`/`Velocity`, sequential read of the old
/// `gpu.pos` (the same line being written), sequential write of `GpuInstance`,
/// all contiguous — so it parallelizes over disjoint rows with `par_iter_mut`.
/// Position copies straight through; the quad half-extent comes from
/// `SimParams.particle_size` so the panel's size slider drives the dot size
/// live (plan §7); color encodes speed via a blue->cyan->white ramp so motion is
/// legible.
///
/// ## Load-bearing in EVERY mode (Phase 20.1 ★n6)
///
/// This system is the SINGLE `prev_pos` maintainer (D3): the shuffle here is
/// the only per-substep `prev_pos` writer in the whole demo. In Physics mode
/// `sync_ball_gpu` overrides pos/scale/color AFTER this pass with field writes
/// that never touch `prev_pos` — so a future "optimization" that gates this
/// system out of Physics (it looks redundant there) would kill prev
/// maintenance and freeze the lerp's rear endpoint. It must run every substep
/// in all three modes.
///
/// `Res<SimParams>` is a shared read, broadcast to every worker for the parallel
/// pass; `particle_size` is hoisted to a local before the loop so each row reads
/// a register, not the resource.
///
/// The closure parameter type is annotated explicitly: `par_iter_mut().for_each`
/// takes `Body: Fn(D::Item<'_>) + Send + Sync`, and rustc cannot infer the
/// higher-ranked closure type from an un-annotated `|(pos, vel, gpu)|` (it
/// reports a misleading "method not found" instead). Spelling the item tuple
/// fixes inference.
pub fn sync_gpu_instance(
    mut query: Query<(&Position, &Velocity, &mut GpuInstance)>,
    params: Res<SimParams>,
) {
    let scale = params.particle_size;
    query
        .par_iter_mut()
        .for_each(move |(pos, vel, gpu): (&Position, &Velocity, &mut GpuInstance)| {
            let speed = (vel.x * vel.x + vel.y * vel.y).sqrt();
            let t = (speed / COLOR_SPEED_MAX).clamp(0.0, 1.0);
            // Prev shuffle (Phase 20.1 D2): the old packed pos becomes the
            // lerp's rear endpoint, exactly one substep behind.
            let prev = gpu.pos;
            *gpu = GpuInstance::with_prev(prev, [pos.x, pos.y], scale, speed_color(t));
        });
}

/// Maps a normalized speed `t` in `[0, 1]` to an `RGBA8` color along a
/// blue -> cyan -> white ramp. Slow particles are deep blue; fast ones whiten.
#[inline]
fn speed_color(t: f32) -> [u8; 4] {
    // Red ramps in only past the midpoint (cyan -> white); green ramps the whole
    // way (blue -> cyan); blue stays high throughout.
    let r = (t.mul_add(2.0, -1.0).max(0.0) * 255.0) as u8;
    let g = ((t * 1.4).min(1.0) * 255.0) as u8;
    let b = (180.0 + t * 75.0) as u8;
    [r, g, b, 255]
}
