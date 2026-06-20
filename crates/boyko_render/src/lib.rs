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
//!   reaches; it owns the manager.
//!
//! # Wave C scope
//!
//! Wave C adds the hand-written [`GpuSystem`] (`impl boyko_ecs::System`) — the MF-5
//! mechanism, in its shipped Phase 5 Option-C shape. It declares EMPTY access + the
//! `is_gpu` marker, projects the `!Send` [`RhiContext`] from the world's NonSend
//! slab inside [`GpuSystem::run_dispatcher`](GpuSystem) through the dispatcher-only
//! [`DispatcherToken::nonsend_resource_mut`] (NOT `run_unsafe`, and NOT a raw cell
//! accessor — the superseded MF-5 `UnsafeEcsCell::nonsend_resource_mut` was DELETED
//! because it was reachable on the concurrent worker path and its `'w` return
//! lifetime allowed two live `&mut R` aliases). The token is mintable ONLY on the
//! dispatcher-solo path (`running == 0`), so a worker never reaches the `!Send`
//! context, and `run_unsafe` is a debug-panic no-op. The system resolves its target
//! column indirectly by `(archetype, component)` (MF-7), and records + submits the
//! `gpu_integrate` compute dispatch. The compute pipeline is built once at setup
//! from the committed [`gpu_integrate_spirv`] via
//! [`RhiContext::create_compute_pipeline`].
//!
//! [`DispatcherToken::nonsend_resource_mut`]: boyko_ecs::ecs::core::system::dispatcher_token::DispatcherToken::nonsend_resource_mut
//!
//! # Wave D scope
//!
//! Wave D adds barrier lowering ([`barrier`]): it consumes the schedule's
//! [`Schedule::gpu_barrier_inputs`](boyko_ecs::ecs::core::schedule::schedule::Schedule::gpu_barrier_inputs)
//! ([`GpuBarrierEdge`](boyko_ecs::ecs::core::schedule::schedule::GpuBarrierEdge))
//! and lowers each producer→GPU-consumer edge + its `GpuAccessIntent`s into a
//! per-consumer [`PlannedBarrier`] plan (keyed by the durable
//! `(ArchetypeId, ComponentId)` — MF-7). A [`GpuSystem`] then REPLAYS its plan
//! (resolve key → current device buffer → `vkCmdPipelineBarrier`) into the same
//! encoder as its compute dispatch, BEFORE the dispatch — the load-bearing
//! synchronisation between a prior GPU write and this dispatch's read/write.

pub mod barrier;
pub mod error;
pub mod gpu_column;
pub mod gpu_system;
pub mod light;
pub mod light_system;
pub mod material;

pub use barrier::{PlannedBarrier, lower_barriers};
pub use error::GpuColumnError;
pub use gpu_column::{GpuColumnManager, GpuColumnMeta, LOCAL_SIZE_X, ResolvedColumn, RhiContext};
pub use gpu_system::{GpuSystem, gpu_integrate_spirv};
pub use light::{
    CLUSTER_COUNT, CLUSTER_DIM_X, CLUSTER_DIM_Y, CLUSTER_DIM_Z, CLUSTER_FAR_DEFAULT,
    CLUSTER_NEAR_DEFAULT, ClusterCell, ClusterConfig, DirectionalLight, GPU_LIGHT_WORDS, GpuLight,
    INDEX_LIST_CAP, LIGHT_HEADER_BASE_WORDS, LIGHT_HEADER_WORDS, LIGHT_KIND_DIRECTIONAL,
    LIGHT_KIND_POINT, LIGHT_KIND_SKY, LIGHT_KIND_SPOT, LightHeaderGpu, LightingConfig, MAX_LIGHTS,
    MAX_LIGHTS_PER_CLUSTER, PointLight, SPOT_COS_OUTER_MAX, SkyLight, SpotLight, cluster_index,
};
pub use light_system::{
    GPU_LIGHT_BYTES, LIGHT_HEADER_BYTES, LightChanged, LightTableStaging, collect_lights,
    fold_light_table, write_light_table,
};
pub use material::{MATERIAL_GPU_WORDS, MaterialGpu, MaterialId};
