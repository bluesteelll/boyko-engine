//! # boyko_physics — the physics foundation (seam only).
//!
//! This crate ships the physics **seam**, not a physics engine: the universal
//! contact currency ([`Manifold`](manifold::Manifold)), a swappable, zero-`dyn`-
//! on-the-hot-path [`RigidSolver`](solver::RigidSolver) trait, a no-op default
//! solver that proves the seam compiles + integrates
//! ([`NoopSolver`](solver::NoopSolver)), the physics components
//! ([`RigidBody`](components::RigidBody) / [`Collider`](components::Collider) /
//! [`Contact`](components::Contact)) as ordinary `#[derive(Component)]` columns
//! with a Phase-10-ready hot/cold split, and the fixed step pipeline a user adds
//! to their schedule via [`add_physics_systems`](plugin::add_physics_systems).
//!
//! **The real solver is Phase 10.** The Soft-Step solver, SDF-native collision,
//! and any external (Rapier/Jolt) backend are out of scope — but the seam is
//! designed so each slots in by implementing
//! [`RigidSolver`](solver::RigidSolver) on its own `Resource`, with no edit to
//! this crate.
//!
//! ADD-ONLY: a new `boyko_ecs`-dependent crate, zero core edit.

pub mod components;
pub mod manifold;
pub mod math;
pub mod plugin;
pub mod resources;
pub mod solver;
pub mod systems;

pub use components::{
    BodyType, Collider, ColliderShape, Contact, RigidBody, RigidBodyBundle, RigidBodyMass,
};
pub use manifold::{BodyIndex, ContactPoint, Manifold};
pub use math::{MAX_CONTACT_POINTS, Vec2};
pub use plugin::{PhysicsStageKeys, add_physics_systems};
pub use resources::{BodyState, ContactPairs, Manifolds, PhysicsConfig, SolverScratch, TouchedMask};
pub use solver::{NoopSolver, RigidSolver};
pub use systems::{
    physics_apply, physics_broadphase, physics_gather, physics_integrate, physics_narrowphase,
    physics_solve_step,
};
