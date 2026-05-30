//! The ECS simulation layer (plan §6).
//!
//! Pure `boyko_ecs`: components, bundles, resources, systems, and the native
//! fixed-timestep runner. This module holds NO GPU state (plan D8) — it produces
//! the `GpuInstance` column that the render layer uploads. The single seam
//! between sim and render is [`crate::app`].

pub mod bundles;
pub mod components;
pub mod resources;
pub mod runner;
pub mod systems;
