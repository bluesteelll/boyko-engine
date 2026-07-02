//! Pillar B increment B1 — the per-substep interpolation-pair pack system
//! [`pack_gpu_transforms`] + its wiring fn [`add_gpu_transform_pack`].
//!
//! Runs once per fixed substep, AFTER the physics scene-sync tail (so `curr` mirrors
//! the post-solve [`Transform`](boyko_scene::Transform) of a dynamic root body). For
//! each entity carrying both a `Transform` and a [`GpuTransform3D`] it performs the
//! D3 single-site prev-shuffle: `prev = old curr`, THEN `curr = from(Transform)` — so
//! the GPU lerp `mix(prev, curr, alpha)` spans exactly one substep. NO other system
//! writes `prev` (the demo's Phase-20.1 `with_prev` discipline, lifted to 3D).

use boyko_ecs::ecs::core::iters::query::Query;
use boyko_ecs::ecs::core::schedule::ScheduleBuilder;
use boyko_ecs::ecs::core::schedule::system_config::SystemConfig;
use boyko_scene::Transform;

use crate::gpu_transform3d::{GpuTransform3D, TrsPacked};

/// Packs each entity's decomposed [`Transform`] into its [`GpuTransform3D`]
/// interpolation pair, maintaining the prev endpoint via the D3 single-site shuffle.
///
/// Per row: shuffle `prev = old curr` (the prior substep's packed pose becomes the
/// lerp's rear endpoint), THEN write `curr = TrsPacked::from_transform(&transform)`
/// (this substep's pose). One `Transform` read + one 48-byte copy + one 48-byte pack
/// per row, alloc-free.
///
/// # The query shape (mixed table + dense)
///
/// `(&Transform, &mut GpuTransform3D)` is a mixed query: `Transform` is a table
/// column, `GpuTransform3D` is the dense column. `iter_mut` yields exactly the rows
/// carrying BOTH — an entity without the pair is skipped (the dense per-row
/// membership trim), and an entity without the pair never enters (the pack is opt-in
/// by the pair's PRESENCE, Principle-0 capability-as-component).
///
/// # No `RenderEnabled` filter (a deliberate departure from `sync_instance_model_cols`)
///
/// The M3 pack systems filter on `Enabled<RenderEnabled>` because their rows are
/// renderables carrying that render-caps EnableTag. The interpolation pair is a
/// SIMULATION mirror maintained every substep for EVERY interpolated body — including
/// a dynamic physics body that may carry no `RenderEnabled` bit. Gating the shuffle on
/// `RenderEnabled` would FREEZE `prev` for such a body (the demo's ★n6 lesson: the
/// shuffle site must run for every interpolated row every substep, or the lerp's rear
/// endpoint stales). The pair's PRESENCE is the opt-in; a per-frame draw toggle is a
/// separate concern applied at the gather/draw, not the pack.
///
/// # 0%-gate
///
/// A world with no `GpuTransform3D` column yields zero matching rows, so the system
/// does zero work — a scene that never opts into interpolation pays nothing.
#[allow(clippy::needless_pass_by_value)]
pub fn pack_gpu_transforms(mut q: Query<(&Transform, &mut GpuTransform3D)>) {
    for (transform, pair) in q.iter_mut() {
        // D3 single-site shuffle: the old `curr` becomes the lerp's rear endpoint,
        // exactly one substep behind, BEFORE `curr` is overwritten.
        pair.prev = pair.curr;
        pair.curr = TrsPacked::from_transform(transform);
    }
}

/// Registers [`pack_gpu_transforms`] on `builder`, returning its [`SystemConfig`] so
/// the caller can pin the ordering (the add-system idiom, mirroring the physics
/// pipeline's free-fn wiring).
///
/// # Ordering contract (the caller's responsibility)
///
/// The pack MUST run AFTER the physics scene-sync tail
/// (`sync_body_to_transform` — the sole writer of a dynamic root's post-solve
/// `Transform`) so `curr` captures the integrated pose, and once per fixed substep
/// (register it in [`CoreSchedule::Fixed`]). The caller chains the edge on the
/// returned handle:
///
/// ```ignore
/// let body_to_transform = builder.add_system(sync_body_to_transform).after(apply).key();
/// add_gpu_transform_pack(&mut builder).after(body_to_transform);
/// ```
///
/// # Why the edge is the CALLER's job, not wired here
///
/// The engine keeps `SystemKey` in a `pub(crate)` module (the same limitation the
/// physics `PhysicsStageKeys` documents), so this crate cannot NAME a `SystemKey` to
/// pin the cross-edge from `sync_body_to_transform` — a downstream physics-plugin
/// key. Returning the `SystemConfig` hands the caller (which registered the scene-sync
/// tail and holds the real key in scope) the maximally-flexible handle to wire the
/// edge, WITHOUT coupling this render-side pack into the physics plugin.
///
/// A conflict edge alone does not suffice: the pack READS `Transform` (shared) while
/// `sync_body_to_transform` WRITES it (`&mut`), so the two conflict and the executor
/// serializes them — but that does not PIN the pack AFTER the write. The explicit
/// `.after` edge is load-bearing.
#[inline]
pub fn add_gpu_transform_pack(builder: &mut ScheduleBuilder) -> SystemConfig<'_> {
    builder.add_system(pack_gpu_transforms)
}
