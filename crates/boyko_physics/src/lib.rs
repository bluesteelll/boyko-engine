//! # boyko_physics — the physics foundation (seam only).
//!
//! This crate ships the physics seam AND the in-house solver: the universal
//! contact currency ([`Manifold`]), a swappable, zero-`dyn`-
//! on-the-hot-path [`RigidSolver`] trait, a no-op default
//! solver that proves the seam compiles + integrates
//! ([`NoopSolver`]), the real TGS-Soft
//! [`SoftStepSolver`] (P2 — sphere/box convex contacts
//! with soft normal recovery, a 2-DOF Coulomb friction cone, and restitution),
//! the physics components
//! ([`RigidBody`] / [`Collider`] /
//! [`Contact`]) as ordinary `#[derive(Component)]` columns
//! with a Phase-10-ready hot/cold split, the convex narrowphase contact
//! generators ([`narrowphase`] — sphere-sphere / sphere-box / box-box), and the
//! fixed step pipeline a user adds to their schedule via
//! [`add_physics_systems`].
//!
//! **Scope (through W5):** the [`SoftStepSolver`] resolves
//! sphere-sphere, sphere-box, and box-box (OBB) contacts with cross-frame
//! warm-starting (W3) and feature-id-stable box manifolds (W4), plus body-vs-SDF
//! contacts against the analytic [`SdfField`] (W5 — opt-in via
//! [`add_physics_sdf`], the C1 [`SDF_SENTINEL`] one-sided path, ZERO GPU readback).
//! An external (Rapier/Jolt) backend stays out of scope — but the seam is designed
//! so each slots in by implementing [`RigidSolver`] on its own `Resource`, with no
//! edit to this crate.
//!
//! ADD-ONLY: a new `boyko_ecs`-dependent crate, zero core edit.

/// Object-category physics bundle presets ([`DynamicBody`], [`Trigger`]) — named
/// `#[derive(Bundle)]` mixes of scene spatial/render components with this crate's
/// physics columns (std-lib S6). Cycle-free: physics depends on scene.
pub mod bundles;
/// P3 — the cold broadphase StrategyPolicy ([`PhysicsStats`] + `select_broadphase`):
/// a density-driven banded selector for [`PhysicsConfig::broadphase`] (AllPairs ↔
/// Grid), gated Manual-by-default (the 0%-gate). The physics analogue of the P1
/// lighting policy; ECS-native, cold, zero hot-path cost.
pub mod broadphase_policy;
pub mod components;
pub mod manifold;
pub mod math;
pub mod narrowphase;
pub mod plugin;
pub mod resources;
/// Synthetic `ComponentId` band + layout registration for the rigid solver's
/// transient [`ScratchColumn`](boyko_ecs::ecs::core::component::scratch::ScratchColumn)
/// gather mirrors (audit Stage P). Internal — the ids are an implementation
/// detail of the scratch-column owners.
pub(crate) mod scratch_ids;
/// Physics ⇄ `Transform` pose sync + the parented-dynamic guard (std-lib S5).
/// The `Simulated`-bit-selected, one-directional copies that keep the world pose
/// in ONE datum (Principle 0 — no parallel pose store), wrapped around the
/// physics pipeline so the solve stays bit-identical.
pub mod scene_sync;
pub mod sdf_query;
/// O9 — width-only AVX2 batched SDF edit-list narrowphase kernel. Compiled ONLY in
/// an `+avx2` build (the default / Miri build ships the verbatim scalar narrowphase
/// arm in [`systems`]); a non-x86_64 / non-AVX2 target never references it.
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
pub(crate) mod sdf_simd;
pub mod soft;
pub mod solver;
pub mod systems;

pub use broadphase_policy::{GRID_HI, GRID_LO, PhysicsStats, select_broadphase};
pub use bundles::{DynamicBody, Trigger};
pub use components::{
    Collider, ColliderShape, Contact, Kinematic, RigidBody, RigidBodyBundle, RigidBodyMass, Sensor,
    Simulated,
};
pub use manifold::{BodyIndex, ContactPoint, Manifold, SDF_SENTINEL};
pub use math::{MAX_CONTACT_POINTS, Mat3, Quat, Vec3};
pub use plugin::{
    PhysicsStageKeys, SceneSyncKeys, add_physics_colored, add_physics_colored_solve,
    add_physics_sdf, add_physics_soft, add_physics_soft_colored, add_physics_systems,
    add_physics_systems_with_scene_sync,
};
pub use scene_sync::{
    debug_assert_dynamic_bodies_are_roots, sync_body_to_transform, sync_transform_to_body,
};
pub use resources::{
    BodyState, BroadphaseGrid, BroadphaseKind, BroadphaseSelectMode, ConstraintGraph,
    ContactPairs, DEFAULT_SLEEP_FRAMES, DEFAULT_SLEEP_THRESHOLD, IntegrationMode, IslandSleep,
    Manifolds, PhysicsConfig, SolverScratch, TouchedMask,
};
pub use sdf_query::{SdfField, sample_sdf};
pub use soft::{
    ParticleColorGraph, SoftBody, SoftBodyError, SoftColorScratch, SoftRigidReaction,
    physics_soft_step_colored,
};
pub use solver::{ColoredSoftStepSolver, NoopSolver, RigidSolver, SoftStepSolver};
pub use systems::{
    body_bounding_radius, physics_apply, physics_broadphase, physics_build_graph, physics_gather,
    physics_integrate, physics_narrowphase, physics_narrowphase_sdf, physics_solve_colored,
    physics_solve_step,
};
