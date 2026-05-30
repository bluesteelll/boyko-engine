//! The physics (bouncing balls) pipeline (plan §6.5 Physics / D11 / D13 / Wave 6).
//!
//! Seven systems run in order each Physics-mode step:
//!
//! 1. [`integrate_balls`] — `par_iter_mut`: apply gravity, `pos += vel*dt`.
//! 2. [`build_ball_grid`] — snapshot `(pos, vel, radius)` into the
//!    [`BallSnapshot`] and counting-sort the positions into the [`SpatialGrid`]
//!    (broad-phase; cell ≈ 2× max radius, D11).
//! 3. [`collide_balls`] — **sequential** narrow-phase: circle-circle test over
//!    grid-neighbor candidate pairs, resolved by elastic impulse + positional
//!    de-penetration. Operates on the snapshot (random row access), marking
//!    touched rows.
//! 4. [`wall_bounce`] — clamp snapshot positions to the box, flip the contacted
//!    velocity component (restitution), marking touched rows.
//! 5. [`apply_ball_motion`] — write the solved snapshot back into the ECS.
//!    Position is a plain write; velocity is written through the change-tracking
//!    [`Mut<Velocity>`] guard **only for touched rows**, so a later
//!    `Changed<Velocity>` query sees exactly the balls that collided or bounced.
//! 6. [`sync_ball_gpu`] — packs pos + radius + base color into `GpuInstance`
//!    (runs after the shared `sync_gpu_instance`, overriding the ball scale +
//!    color).
//! 7. [`tint_collided`] — the `Changed<Velocity>` showcase: overlays a flash
//!    color onto the `GpuInstance` of every ball whose velocity changed this
//!    frame (collision or wall bounce).
//!
//! ## Why collision resolution is sequential (plan §9 G12)
//!
//! A collision pair `(a, b)` writes **both** rows. `par_iter_mut` hands each
//! worker disjoint rows, so two workers resolving `(a, b)` and `(b, c)` at once
//! would race on `b`. We ship the correct single-threaded resolver: the reads
//! come from a pre-tick snapshot (so the broad/narrow phases see a consistent
//! frame), and the pair resolution — which mutates two rows — runs on one thread.
//! Cell-coloring for parallel collision is a documented stretch goal (G12).
//!
//! ## Why the write-back is what makes `Changed<Velocity>` precise
//!
//! Only [`Mut<T>::deref_mut`] bumps a row's `changed` tick — a plain `&mut T`
//! query item writes the value WITHOUT touching the tick, and a `for_each_chunk`
//! `&mut [Velocity]` cannot touch ticks at all. So the velocity write-back
//! ([`apply_ball_motion`]) takes a [`Mut<Velocity>`] guard and deref-writes it
//! per row only when the solver flagged the row touched — that is what keeps
//! `Changed<Velocity>` matching exactly the collided/bounced balls. With the
//! default `gravity = 0`, [`integrate_balls`] does not write velocity at all, so
//! the only velocity writers are collisions and wall bounces; raising gravity
//! writes every ball's velocity every frame and the tint then flashes everything
//! (a documented footgun — see `docs/DEMO-DOGFOODING.md` W6-1).

use boyko_ecs::ecs::core::iters::query::data::Mut;
use boyko_ecs::ecs::core::iters::query::filter::{Changed, With};
use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_ecs::ecs::core::system::{Res, ResMut};

use crate::render::WORLD_HALF_EXTENT;
use crate::render::instance::GpuInstance;
use crate::sim::components::{BallTag, Position, Radius, Velocity};
use crate::sim::grid::SpatialGrid;
use crate::sim::resources::{BallSnapshot, DeltaTime, PhysicsParams};

/// Largest ball radius in world units. The broad-phase cell size is `2×` this so
/// any colliding pair lands within the 3×3 cell neighborhood the grid walks.
pub const MAX_BALL_RADIUS: f32 = 2.2;

/// Smallest ball radius in world units.
pub const MIN_BALL_RADIUS: f32 = 1.0;

/// Spatial-grid cell edge for the physics broad-phase (plan D11: 2× max radius).
pub const BALL_CELL_SIZE: f32 = 2.0 * MAX_BALL_RADIUS;

/// Below this squared center distance two balls are treated as coincident and
/// skipped (avoids a divide-by-zero when computing the collision normal).
const COINCIDENT_EPSILON_SQ: f32 = 1e-6;

/// Speed (world units/s) mapped to the top of the ball base-color ramp.
const BALL_COLOR_SPEED_MAX: f32 = 120.0;

/// Packed RGBA8 flash color for a just-collided ball (a hot white-yellow).
const COLLISION_FLASH_COLOR: [u8; 4] = [255, 240, 120, 255];

/// Applies gravity and integrates every ball's position (plan §6.5 Physics).
///
/// A sound `par_iter_mut` over disjoint rows: each ball reads/writes only its own
/// `Position`/`Velocity`. Gravity is a downward acceleration on velocity, applied
/// only when nonzero — so with the default `gravity = 0` this system writes
/// **only** `Position`, leaving the `Velocity` change ticks untouched for the
/// collision/bounce passes to set precisely (see the module docs). The closure
/// item type is annotated explicitly (the GAT/HRTB inference gap the other
/// integrators document).
pub fn integrate_balls(
    mut query: Query<(&mut Position, &mut Velocity, &Radius), With<BallTag>>,
    dt: Res<DeltaTime>,
    params: Res<PhysicsParams>,
) {
    let dt = dt.0;
    let gravity = params.gravity;
    query.par_iter_mut().for_each(
        move |(pos, vel, _radius): (&mut Position, &mut Velocity, &Radius)| {
            // Only touch velocity when gravity is active, so the change-tick
            // stays clean for the collision/bounce passes when gravity is off.
            if gravity != 0.0 {
                vel.y -= gravity * dt;
            }
            pos.x += vel.x * dt;
            pos.y += vel.y * dt;
        },
    );
}

/// Snapshots ball state and rebuilds the broad-phase grid from it (plan D11).
///
/// Runs after [`integrate_balls`] (so it snapshots the post-integration
/// positions) and before [`collide_balls`]. The snapshot is the single source of
/// truth for the sequential collision + wall passes; the grid indexes the
/// snapshot's row indices. Sequential and allocation-free in steady state (the
/// snapshot `Vec`s are cleared and refilled, capacity reused — plan §11.2).
///
/// Row order is the archetype iteration order, which [`apply_ball_motion`]
/// re-walks identically with `iter_mut` to write the solution back: no structural
/// change happens between snapshot and write-back, so row `i` is the same ball in
/// both passes.
///
/// `grid` is mutated through `&mut self` method calls (`set_cell_size`,
/// `rebuild_with`); `snapshot` through a `&mut *` reborrow.
#[allow(clippy::needless_pass_by_value)]
pub fn build_ball_grid(
    mut query: Query<(&Position, &Velocity, &Radius), With<BallTag>>,
    mut snapshot: ResMut<BallSnapshot>,
    mut grid: ResMut<SpatialGrid>,
) {
    let snap = &mut *snapshot;
    snap.clear();
    query.for_each_chunk(
        |(positions, velocities, radii): (&[Position], &[Velocity], &[Radius])| {
            for ((p, v), r) in positions.iter().zip(velocities).zip(radii) {
                snap.push(*p, *v, r.0);
            }
        },
    );

    // Cell = 2× max radius so a colliding pair is always within the 3×3 block.
    grid.set_cell_size(BALL_CELL_SIZE);
    let positions = snap.pos.as_slice();
    grid.rebuild_with(positions.len(), |i| [positions[i].x, positions[i].y]);
}

/// Sequential narrow-phase collision detection + elastic-impulse resolution
/// (plan §6.5 Physics / G12).
///
/// For each ball `i` the grid yields candidate neighbors from its 3×3 cell block;
/// we keep only `j > i` so each unordered pair is tested exactly once. A pair that
/// overlaps (`dist < r_i + r_j`) is de-penetrated (split the overlap evenly,
/// equal mass) and, if the balls are approaching, exchanges the normal component
/// of their relative velocity scaled by the restitution `e`. Both rows are
/// flagged touched so [`apply_ball_motion`] writes their velocity through the
/// change-tracking guard.
///
/// **Sequential** (G12): each resolution mutates two snapshot rows. Reading and
/// writing the flat snapshot `Vec`s by index on one thread has no aliasing
/// hazard, where a `par_iter_mut` over pairs would race on a shared ball.
//
// `clippy::needless_pass_by_value`: `ResMut<_>` is a by-value SystemParam by
// protocol (the param system delivers an owned guard; `&ResMut<_>` is not a
// valid param type). The body mutates it through a `&mut *snapshot` reborrow
// rather than a direct `&mut self` method call, which clippy cannot credit, so
// the lint false-positives here. Allowed with this justification.
#[allow(clippy::needless_pass_by_value)]
pub fn collide_balls(
    grid: Res<SpatialGrid>,
    mut snapshot: ResMut<BallSnapshot>,
    params: Res<PhysicsParams>,
) {
    let e = params.restitution;
    let snap = &mut *snapshot;
    let n = snap.pos.len();
    if n < 2 {
        return;
    }

    for i in 0..n {
        let pos_i = snap.pos[i];
        let r_i = snap.radius[i];
        // The grid borrow ends before the snapshot is mutated below: collect this
        // ball's higher-indexed candidates into the snapshot's reused scratch
        // buffer (no per-ball heap allocation, plan §11.2).
        snap.candidates.clear();
        grid.for_each_neighbor(pos_i.x, pos_i.y, |row| {
            let j = row as usize;
            if j > i {
                snap.candidates.push(j);
            }
        });

        // Resolve against each candidate. `candidates` is read by index below so
        // it does not alias the `pos`/`vel`/`touched` writes.
        let count = snap.candidates.len();
        for c in 0..count {
            let j = snap.candidates[c];
            let dx = snap.pos[j].x - snap.pos[i].x;
            let dy = snap.pos[j].y - snap.pos[i].y;
            let dist_sq = dx * dx + dy * dy;
            let radii = r_i + snap.radius[j];
            if dist_sq >= radii * radii || dist_sq <= COINCIDENT_EPSILON_SQ {
                continue;
            }
            let dist = dist_sq.sqrt();
            let inv_dist = dist.recip();
            let nx = dx * inv_dist;
            let ny = dy * inv_dist;

            // Positional correction: split the penetration evenly (equal mass).
            let half = (radii - dist) * 0.5;
            snap.pos[i].x -= nx * half;
            snap.pos[i].y -= ny * half;
            snap.pos[j].x += nx * half;
            snap.pos[j].y += ny * half;

            // Elastic impulse along the normal. With equal mass the normal
            // component of the relative velocity is exchanged, scaled by `1+e`.
            let rvx = snap.vel[j].x - snap.vel[i].x;
            let rvy = snap.vel[j].y - snap.vel[i].y;
            let vel_along_normal = rvx * nx + rvy * ny;
            // Only resolve when the balls are approaching (negative closing
            // velocity); receding pairs are left alone so they separate cleanly.
            if vel_along_normal < 0.0 {
                let impulse = vel_along_normal * (1.0 + e) * 0.5;
                snap.vel[i].x += impulse * nx;
                snap.vel[i].y += impulse * ny;
                snap.vel[j].x -= impulse * nx;
                snap.vel[j].y -= impulse * ny;
                snap.touched[i] = true;
                snap.touched[j] = true;
            }
        }
    }
}

/// Clamps every ball to the world box and flips the contacted velocity component
/// (plan §6.5 Physics wall bounce).
///
/// Sequential pass over the snapshot, after [`collide_balls`]. A wall contact
/// pins the position to the wall (accounting for the ball radius), reflects and
/// damps the axis velocity by the restitution `e`, and flags the row touched so
/// the tint flashes wall hits as well as collisions.
//
// `clippy::needless_pass_by_value`: same SystemParam false-positive as
// `collide_balls` — `ResMut<_>` is a by-value param mutated via a reborrow.
#[allow(clippy::needless_pass_by_value)]
pub fn wall_bounce(mut snapshot: ResMut<BallSnapshot>, params: Res<PhysicsParams>) {
    let e = params.restitution;
    let bound = WORLD_HALF_EXTENT;
    let snap = &mut *snapshot;
    let n = snap.pos.len();
    for i in 0..n {
        let r = snap.radius[i];
        let lo = -bound + r;
        let hi = bound - r;
        let mut touched = false;
        if snap.pos[i].x < lo {
            snap.pos[i].x = lo;
            snap.vel[i].x = snap.vel[i].x.abs() * e;
            touched = true;
        } else if snap.pos[i].x > hi {
            snap.pos[i].x = hi;
            snap.vel[i].x = -snap.vel[i].x.abs() * e;
            touched = true;
        }
        if snap.pos[i].y < lo {
            snap.pos[i].y = lo;
            snap.vel[i].y = snap.vel[i].y.abs() * e;
            touched = true;
        } else if snap.pos[i].y > hi {
            snap.pos[i].y = hi;
            snap.vel[i].y = -snap.vel[i].y.abs() * e;
            touched = true;
        }
        if touched {
            snap.touched[i] = true;
        }
    }
}

/// Writes the solved snapshot back into the ECS columns (plan D13).
///
/// Re-walks the ball rows in the same order [`build_ball_grid`] snapshotted them
/// (no structural change happened in between, so row `i` is the same ball).
/// Position is a plain `&mut Position` (nothing filters on it). **Velocity is a
/// [`Mut<Velocity>`] change-tracking guard, deref-written only for rows the
/// solver flagged touched** — `Mut::deref_mut` bumps the row's `changed` tick, so
/// the later `Changed<Velocity>` query in [`tint_collided`] matches exactly the
/// collided/bounced balls. Untouched rows never deref their velocity guard, so
/// their tick stays put.
///
/// This is the precise-tick subtlety the plan calls out: a plain `&mut Velocity`
/// (or a `for_each_chunk` `&mut [Velocity]`) does NOT touch change ticks at all
/// (only `Mut<T>::deref_mut` does), so the showcase requires the `Mut<Velocity>`
/// guard and a per-row write. Sequential (it is the write-back of an
/// already-sequential solve).
///
/// `clippy::needless_pass_by_value`: `Res<_>` is a by-value SystemParam read via
/// a `&*` reborrow — the same false-positive as the `ResMut` systems above.
#[allow(clippy::needless_pass_by_value)]
pub fn apply_ball_motion(
    mut query: Query<(&mut Position, Mut<Velocity>), With<BallTag>>,
    snapshot: Res<BallSnapshot>,
) {
    let snap = &*snapshot;
    for (row, (pos, mut vel)) in query.iter_mut().enumerate() {
        // Defensive: the row count must match the snapshot; if a structural
        // change ever desynchronized them, stop rather than index out of bounds.
        if row >= snap.pos.len() {
            break;
        }
        // Position: plain write, no change tracking.
        pos.x = snap.pos[row].x;
        pos.y = snap.pos[row].y;
        // Velocity: only the touched rows deref the `Mut` guard, bumping the
        // `changed` tick exactly for balls that collided or bounced — which is
        // what keeps `Changed<Velocity>` (the tint) precise. The `vel.x = ...`
        // field write routes through `Mut::deref_mut`.
        if snap.touched[row] {
            vel.x = snap.vel[row].x;
            vel.y = snap.vel[row].y;
        }
    }
}

/// Packs each ball's position, radius, and base color into its `GpuInstance`
/// column for rendering (plan D3, physics variant).
///
/// A sound `par_iter_mut` over disjoint rows. The shared `sync_gpu_instance`
/// (mode-agnostic) also writes balls, sizing them by the particle slider; this
/// dedicated pass runs AFTER it and overrides the scale with the ball's actual
/// [`Radius`] (so a ball renders at its collision size) and writes a
/// speed-cued base color. [`tint_collided`] then overlays the collision flash on
/// top. Keeping this in a separate physics system leaves the shared
/// `sync_gpu_instance` untouched for the other modes.
pub fn sync_ball_gpu(
    mut query: Query<(&Position, &Velocity, &Radius, &mut GpuInstance), With<BallTag>>,
    params: Res<PhysicsParams>,
) {
    let size_scale = params.ball_size;
    query.par_iter_mut().for_each(
        move |(pos, vel, radius, inst): (&Position, &Velocity, &Radius, &mut GpuInstance)| {
            // Base color cues speed (slow = teal, fast = warm) so motion reads
            // even before a collision; the tint overlays the flash afterward.
            let speed = (vel.x * vel.x + vel.y * vel.y).sqrt();
            *inst = GpuInstance::new([pos.x, pos.y], radius.0 * size_scale, ball_base_color(speed));
        },
    );
}

/// The `Changed<Velocity>` showcase (plan D13): flashes balls that collided or
/// bounced this frame.
///
/// Runs after [`sync_ball_gpu`] (which packs every ball's position + base color),
/// so this overlay is not overwritten. `Changed<Velocity>` matches exactly the
/// rows [`apply_ball_motion`] wrote velocity to (collisions + wall bounces), and
/// is valid here because the system runs INSIDE the schedule — the direct
/// `EcsMaster::query()` API would panic on a change-detection filter (plan §9
/// G2). Only the `color` field is rewritten; the position and scale `sync_ball_gpu`
/// wrote are preserved, so the flash reads as a recolor of the same dot.
pub fn tint_collided(mut query: Query<&mut GpuInstance, Changed<Velocity>>) {
    for inst in query.iter_mut() {
        // Read-modify-write: keep the position/scale `sync_ball_gpu` wrote this
        // frame, replace only the color with the collision flash. The item is a
        // plain `&mut GpuInstance` (no change tracking on `GpuInstance` itself),
        // so the field reads and the `*inst = ...` write are direct.
        let pos = inst.pos;
        let scale = inst.scale;
        *inst = GpuInstance::new(pos, scale, COLLISION_FLASH_COLOR);
    }
}

/// Maps a ball's speed to a packed RGBA8 base color (teal → warm). Pure.
#[inline]
fn ball_base_color(speed: f32) -> [u8; 4] {
    let t = (speed / BALL_COLOR_SPEED_MAX).clamp(0.0, 1.0);
    let r = (60.0 + t * 180.0) as u8;
    let g = (180.0 - t * 60.0) as u8;
    let b = (200.0 - t * 120.0) as u8;
    [r, g, b, 255]
}
