//! XPBD soft-body foundation (Physics O11 SP1) — distance-constraint soft bodies
//! solved by a SEPARATE position pass after the rigid solve.
//!
//! SP1 ships the distance-constraint Extended Position-Based Dynamics (XPBD)
//! kernel: a [`SoftBody`] component (SoA-by-axis, preallocated, zero per-step
//! allocation) plus the [`physics_soft_step`] system that predicts, projects one
//! Gauss-Seidel pass of distance constraints, resolves one-sided SDF collision,
//! and updates velocities — all in place on the component's own columns.
//!
//! # Determinism (INVIOLABLE)
//!
//! The whole pass is bit-deterministic and uses ONLY exact `sqrt` + divide — no
//! `rsqrt`/`rcp`/`mul_add`/FMA-contraction, no [`Vec3::normalize`](boyko_math::Vec3::normalize) (it collapses
//! at `f32::MIN_POSITIVE`; the kernel uses an explicit `d * (1.0 / len)` past the
//! [`solver::LEN_EPS`] guard). The constraint sweep is one Gauss-Seidel iteration
//! in pinned array order `0..m`; particles and bodies are visited in fixed index
//! order; pinned (`inv_mass == 0`) particles are frozen.
//!
//! # The 0%-gate (rigid path untouched)
//!
//! Soft-body is OPT-IN via [`PhysicsConfig::soft_body`](crate::PhysicsConfig)
//! (default `false`) and wired by [`add_physics_soft`](crate::plugin::add_physics_soft). The rigid solvers,
//! `BodyState`/`SolverScratch`/`physics_apply`/`physics_integrate`/the constraint
//! graph are byte-untouched; [`physics_soft_step`] is a STRICTLY DISJOINT
//! integrator — it operates entirely on [`SoftBody`] columns, never reads or
//! writes `scratch.bodies`, never sets a touched bit, and never enters
//! `physics_apply`. A world that never sets `soft_body = true` and never calls
//! [`add_physics_soft`](crate::plugin::add_physics_soft) is byte-for-byte unaffected.
//!
//! # Seams for SP2+
//!
//! SP1 is distance-only and single-threaded. Volume constraints (SP2),
//! self-collision (SP3), parallel coloring (SP4), and the GPU mirror (SP5) slot
//! in behind their own opt-in flags without touching this kernel.

pub mod collide;
pub mod colored;
pub mod component;
pub mod coupling;
pub mod self_collision;
pub mod solver;

pub use colored::{ParticleColorGraph, SoftColorScratch, physics_soft_step_colored};
pub use component::{SoftBody, SoftBodyError};
pub use coupling::{SoftRigidReaction, physics_soft_rigid_apply};
pub use solver::{physics_soft_step, physics_soft_step_coupled};
