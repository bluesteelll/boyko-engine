//! Systems shared across every mode (plan §6.5 "Common").

use boyko_ecs::ecs::core::iters::query::query::Query;

use crate::render::instance::GpuInstance;
use crate::sim::components::{Position, Velocity};

/// Half-extent of a particle quad in world units. Small enough that 100k dots
/// read as a field rather than a solid sheet.
const PARTICLE_SCALE: f32 = 0.6;

/// Speed (world units/s) mapped to the top of the color ramp. Velocities at or
/// above this render fully "hot".
const COLOR_SPEED_MAX: f32 = 200.0;

/// Packs `GpuInstance` from the sim state each frame (plan D3).
///
/// Runs after integration and before the upload. It is a streaming SoA->SoA
/// write — sequential read of `Position`/`Velocity`, sequential write of
/// `GpuInstance`, all contiguous — so it parallelizes over disjoint rows with
/// `par_iter_mut`. Position copies straight through; color encodes speed via a
/// blue->cyan->white ramp so motion is legible.
///
/// The closure parameter type is annotated explicitly: `par_iter_mut().for_each`
/// takes `Body: Fn(D::Item<'_>) + Send + Sync`, and rustc cannot infer the
/// higher-ranked closure type from an un-annotated `|(pos, vel, gpu)|` (it
/// reports a misleading "method not found" instead). Spelling the item tuple
/// fixes inference.
pub fn sync_gpu_instance(mut query: Query<(&Position, &Velocity, &mut GpuInstance)>) {
    query
        .par_iter_mut()
        .for_each(|(pos, vel, gpu): (&Position, &Velocity, &mut GpuInstance)| {
            let speed = (vel.x * vel.x + vel.y * vel.y).sqrt();
            let t = (speed / COLOR_SPEED_MAX).clamp(0.0, 1.0);
            *gpu = GpuInstance::new([pos.x, pos.y], PARTICLE_SCALE, speed_color(t));
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
