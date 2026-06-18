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

pub mod components;
pub mod manifold;
pub mod math;
pub mod narrowphase;
pub mod plugin;
pub mod resources;
pub mod sdf_query;
pub mod solver;
pub mod systems;

pub use components::{
    BodyType, Collider, ColliderShape, Contact, RigidBody, RigidBodyBundle, RigidBodyMass,
};
pub use manifold::{BodyIndex, ContactPoint, Manifold, SDF_SENTINEL};
pub use math::{MAX_CONTACT_POINTS, Mat3, Quat, Vec3};
pub use plugin::{
    PhysicsStageKeys, add_physics_colored, add_physics_colored_solve, add_physics_sdf,
    add_physics_systems,
};
pub use resources::{
    BodyState, BroadphaseGrid, BroadphaseKind, ConstraintGraph, ContactPairs, IntegrationMode,
    Manifolds, PhysicsConfig, SolverScratch, TouchedMask,
};
pub use sdf_query::{SdfField, sample_sdf};
pub use solver::{ColoredSoftStepSolver, NoopSolver, RigidSolver, SoftStepSolver};
pub use systems::{
    body_bounding_radius, physics_apply, physics_broadphase, physics_build_graph, physics_gather,
    physics_integrate, physics_narrowphase, physics_narrowphase_sdf, physics_solve_colored,
    physics_solve_step,
};
