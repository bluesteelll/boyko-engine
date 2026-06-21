//! Physics ⇄ `Transform` pose sync (std-lib S5) — the single-source-of-truth
//! bridge between the physics [`RigidBody`] pose and the engine's canonical
//! [`Transform`](boyko_scene::Transform) component.
//!
//! # One pose, one writer per schedule window (Principle 0)
//!
//! There is no separate "physics pose" datum duplicated alongside `Transform`:
//! the world pose lives in TWO ECS columns ([`RigidBody`] and `Transform`) with a
//! **`BodyType`-selected, one-directional** copy between them, so no datum has two
//! writers in one schedule window:
//!
//! - **Dynamic** bodies: physics OWNS the pose. The solver integrates
//!   `RigidBody.{position, rotation}`; [`sync_body_to_transform`] copies it OUT to
//!   `Transform` AFTER `physics_apply`. `Transform` is downstream.
//! - **Static / Kinematic** bodies: gameplay OWNS the pose. It authors
//!   `Transform`; [`sync_transform_to_body`] copies it IN to `RigidBody` BEFORE
//!   `physics_gather`. `RigidBody` is downstream.
//!
//! The per-frame `propagate_transforms` (in `boyko_scene`) remains the sole
//! `GlobalTransform` writer, so the full chain is
//! `Transform → RigidBody → (solve) → RigidBody → Transform → GlobalTransform`
//! with exactly one writer per datum per window. See the schedule-placement
//! contract on [`add_scene_sync`](crate::plugin::add_scene_sync).
//!
//! # Bit-determinism (HARD CONSTRAINT)
//!
//! These systems wrap AROUND the physics pipeline; they do NOT touch the solve.
//! The copies are PLAIN field assignments (`Vec3` / `Quat` `Copy`), exact, with no
//! FMA, no re-normalization, no decompose/recompose — so the physics determinism
//! suite is byte-identical whether or not the sync runs. A `Dynamic` root's
//! `Transform.translation` therefore bit-equals `RigidBody.position` after sync.
//!
//! # Change-detection reconciliation
//!
//! `sync_body_to_transform` is **value-gated**: it writes through the
//! [`Mut`](boyko_ecs::ecs::core::iters::query::data::Mut) guard (bumping
//! `Changed<Transform>`) ONLY when the integrated pose actually differs from the
//! current `Transform`. A resting / sleeping dynamic body produces an unchanged
//! pose → no deref-write → no `Changed` tick bump → its subtree is skipped by the
//! dirty-gated propagation, so a scene of resting bodies pays no per-frame
//! recompose.

use boyko_ecs::ecs::core::hierarchy::ChildOf;
use boyko_ecs::ecs::core::iters::query::data::Mut;
use boyko_ecs::ecs::core::iters::query::filter::{With, Without};
use boyko_ecs::ecs::core::iters::query::query::Query;
use boyko_scene::Transform;

use crate::components::{BodyType, RigidBody, RigidBodyMass};

/// Copies `Transform` → `RigidBody` for STATIC / KINEMATIC bodies (std-lib S5).
///
/// Runs FIRST in the fixed schedule, before `physics_gather`, so the gather
/// snapshots the gameplay-authored pose. Dynamic bodies are skipped (the solver
/// owns their pose; [`sync_body_to_transform`] copies the other direction).
///
/// The `body_type` test is a per-row read of the cold
/// [`RigidBodyMass`] column (which carries [`BodyType`]); only a non-`Dynamic`
/// body is written, so a pure-Dynamic world does zero writes here. The copy is
/// **value-gated** through the [`Mut`] guard: it bumps `Changed<RigidBody>` only
/// when the authored pose actually differs from the body's current pose, so a
/// static body whose `Transform` never moves does not perpetually dirty
/// `RigidBody`.
///
/// `Transform.scale` is intentionally NOT copied — the physics `RigidBody` has no
/// scale concept (the collider carries its own extents); scale stays a pure
/// `Transform` property.
//
// `clippy::needless_pass_by_value`: `Query<_>` is a by-value `SystemParam` by
// protocol (the param system delivers an owned handle); the body uses it through
// `iter_mut`. Same idiom as the physics pipeline stages.
#[allow(clippy::needless_pass_by_value)]
pub fn sync_transform_to_body(mut query: Query<(&Transform, Mut<RigidBody>, &RigidBodyMass)>) {
    for (transform, mut body, mass) in query.iter_mut() {
        // Dynamic bodies own their own pose — physics integrates them and
        // `sync_body_to_transform` writes the result back to `Transform`. Only the
        // gameplay-driven Static / Kinematic bodies are synced IN here.
        if mass.body_type == BodyType::Dynamic {
            continue;
        }
        // Value-gated bit-exact copy: `Deref` reads the current pose without
        // bumping the changed tick; only when the authored pose differs do we
        // `deref_mut` (one tick bump) and write. `Vec3` / `Quat` `PartialEq` is the
        // derived per-field IEEE `==` — exact, no epsilon.
        if body.position != transform.translation || body.rotation != transform.rotation {
            let body = &mut *body;
            body.position = transform.translation;
            body.rotation = transform.rotation;
        }
    }
}

/// Copies `RigidBody` → `Transform` for DYNAMIC ROOT bodies (std-lib S5).
///
/// Runs LAST in the fixed schedule, after `physics_apply`, so the integrated pose
/// reaches `Transform` (and, on the next per-frame run, `GlobalTransform`). Only
/// Dynamic bodies are written (Static / Kinematic poses are gameplay-owned and
/// flow the other direction). Parented bodies are structurally excluded by the
/// `Without<ChildOf>` filter — see "Dynamic bodies must be roots" below.
///
/// **Field-granular, value-gated copy** (no decompose/recompose drift):
///
/// ```text
/// transform.translation = body.position;   // bit-exact
/// transform.rotation    = body.rotation;   // bit-exact (no re-normalize)
/// // transform.scale  UNTOUCHED
/// ```
///
/// The write goes through the [`Mut`] guard ONLY when the integrated pose differs
/// from the current `Transform` (bit-compare), so a resting / sleeping dynamic
/// body bumps no `Changed<Transform>` tick and its subtree is skipped by
/// propagation (the dirty-gate reconciliation).
///
/// # Dynamic bodies must be roots
///
/// A parented Dynamic body would have its WORLD pose written into a LOCAL
/// `Transform`, then double-composed by `propagate_transforms` — silent
/// corruption. The `Without<ChildOf>` filter STRUCTURALLY excludes such a body
/// from this write (it can never be mis-synced). The complementary diagnostic
/// [`debug_assert_dynamic_bodies_are_roots`] catches the misuse in debug builds.
//
// `clippy::needless_pass_by_value`: see `sync_transform_to_body`.
#[allow(clippy::needless_pass_by_value)]
pub fn sync_body_to_transform(
    mut query: Query<(Mut<Transform>, &RigidBody, &RigidBodyMass), Without<ChildOf>>,
) {
    for (mut transform, body, mass) in query.iter_mut() {
        // Only Dynamic bodies are downstream of the solve; a Static / Kinematic
        // body's `Transform` is gameplay-owned and must not be overwritten.
        if mass.body_type != BodyType::Dynamic {
            continue;
        }
        // Value-gate: read the current `Transform` via `Deref` (no tick bump), and
        // only when the integrated pose differs do we `deref_mut` (bump
        // `Changed<Transform>`) and write translation + rotation. A resting body
        // (equal pose) writes nothing, so propagation skips its subtree.
        if body.position != transform.translation || body.rotation != transform.rotation {
            let transform = &mut *transform;
            transform.translation = body.position;
            transform.rotation = body.rotation;
            // `transform.scale` is deliberately left as authored.
        }
    }
}

/// Debug-only diagnostic that asserts NO Dynamic body carries a `ChildOf`
/// (std-lib S5 — the "Dynamic bodies must be roots" guard).
///
/// v1 does not support parented dynamics: their world-space integrated pose would
/// be written into a local-space `Transform` and then re-composed by
/// `propagate_transforms`. The [`sync_body_to_transform`] `Without<ChildOf>`
/// filter already EXCLUDES such a body from the write (it can never be silently
/// mis-synced), so this system is purely a developer tripwire — it fires a
/// `debug_assert!` if it ever finds a parented Dynamic body, surfacing the misuse
/// at its source instead of leaving it as a silent no-sync.
///
/// It is registered alongside the sync systems by
/// [`add_scene_sync`](crate::plugin::add_scene_sync). In release builds the
/// `debug_assert!` compiles out and the loop body is empty, so the system is a
/// near-free archetype walk (and the scheduler may run it in parallel — it only
/// READS the cold `RigidBodyMass` column). It allocates nothing.
//
// `clippy::needless_pass_by_value`: see `sync_transform_to_body`.
#[allow(clippy::needless_pass_by_value)]
pub fn debug_assert_dynamic_bodies_are_roots(query: Query<&RigidBodyMass, With<ChildOf>>) {
    // Only meaningful in debug builds; the loop and assert vanish in release.
    if cfg!(debug_assertions) {
        for mass in query.iter() {
            debug_assert!(
                mass.body_type != BodyType::Dynamic,
                "invariant: a Dynamic RigidBody must be a hierarchy ROOT (no ChildOf) — \
                 v1 does not support parented dynamics; the body's world pose would be \
                 written into a local Transform and double-composed by propagation. \
                 Parent a kinematic proxy instead, or keep the dynamic body unparented."
            );
        }
    }
}
