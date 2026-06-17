//! `boyko_render` — GPU-resident ECS columns (Phase 5).
//!
//! This crate is the bridge between the graphics-pure ECS core ([`boyko_ecs`])
//! and the backend-agnostic RHI ([`boyko_rhi`] / [`boyko_rhi_vulkan`]). It is the
//! ONLY crate that may name both surfaces — the dependency fan-out is `boyko_render`
//! depending DIRECTLY on `boyko_ecs` + `boyko_rhi` + `boyko_rhi_vulkan` +
//! `boyko_utils` (and `boyko_rhi` itself depends only on `boyko_utils`, NOT on
//! `boyko_ecs`), with NO cycle, so the orphan-rule impls (`RhiContext:
//! NonSendResource`) and the graphics-aware types ([`GpuColumnManager`],
//! [`GpuColumnMeta`]) live here, never in `boyko_ecs`.
//!
//! # Wave B scope
//!
//! Wave B mints REAL device-local component pools:
//! - [`GpuColumnManager`] allocates a `DeviceLocal` (VRAM) buffer through the RHI
//!   registry, packs its generational slot into the opaque
//!   [`DeviceColumnHandle`](boyko_ecs::ecs::memory::device_column::DeviceColumnHandle)
//!   the core stores, and drives the A2 seam
//!   [`Archetype::make_component_device_backed`](boyko_ecs::ecs::core::archetype::archetype::Archetype::make_component_device_backed).
//! - [`RhiContext`] is the `!Send` handle (`impl NonSendResource`) the dispatcher
//!   later reaches; for Wave B it owns the manager.
//!
//! The `GpuSystem` / barrier lowering / shaders are LATER waves (C/D).

pub mod error;
pub mod gpu_column;

pub use error::GpuColumnError;
pub use gpu_column::{GpuColumnManager, GpuColumnMeta, ResolvedColumn, RhiContext};
