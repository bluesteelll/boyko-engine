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
/// Light-object bundle presets ([`DirectionalLightObject`] / [`PointLightObject`]
/// / [`SpotLightObject`]) — named `#[derive(Bundle)]` mixes of scene spatial
/// components with this crate's light components (std-lib S6). Cycle-free: render
/// depends on scene.
pub mod bundles;
pub mod error;
/// The gbuffer ⇄ marcher linear-depth contract (mesh foundation M2): the host
/// [`GBUFFER_T_MAX`](gbuffer_depth::GBUFFER_T_MAX) mirror + the C2 drift guard
/// pinning it to the marcher's `SDF_TRACE_T_MAX`.
pub mod gbuffer_depth;
pub mod gpu3d_instance;
pub mod gpu3d_system;
pub mod gpu_column;
pub mod gpu_system;
pub mod light;
pub mod light_plugin;
pub mod light_policy;
pub mod light_reconcile;
pub mod light_system;
pub mod material;
/// The renderer-owned mesh asset table (mesh foundation M2): [`MeshRegistry`] /
/// [`MeshGpu`] / [`Vertex`], the GPU vertex+index buffers a `MeshHandle` indexes.
pub mod mesh_registry;
pub mod render3d_plugin;
pub mod ssao_config;
pub mod ssao_plugin;
pub mod ui;
pub mod view;

pub use barrier::{PlannedBarrier, lower_barriers};
pub use bundles::{DirectionalLightObject, PointLightObject, SpotLightObject};
pub use error::GpuColumnError;
pub use gpu3d_instance::{GPU3D_INSTANCE_SIZE, Gpu3dInstance};
pub use gpu3d_system::sync_gpu_3d_instances;
pub use light_plugin::LightingPlugin;
pub use light_reconcile::light_reconcile;
pub use render3d_plugin::Render3dPlugin;
pub use gpu_column::{GpuColumnManager, GpuColumnMeta, LOCAL_SIZE_X, ResolvedColumn, RhiContext};
pub use gpu_system::{GpuSystem, gpu_integrate_spirv};
pub use light::{
    CLUSTER_COUNT, CLUSTER_DIM_X, CLUSTER_DIM_Y, CLUSTER_DIM_Z, CLUSTER_FAR_DEFAULT,
    CLUSTER_NEAR_DEFAULT, ClusterCell, ClusterConfig, ClusterSelectMode, DirectionalLight,
    GPU_LIGHT_WORDS, GpuLight, INDEX_LIST_CAP, LIGHT_HEADER_BASE_WORDS, LIGHT_HEADER_WORDS,
    LIGHT_KIND_DIRECTIONAL, LIGHT_KIND_POINT, LIGHT_KIND_SKY, LIGHT_KIND_SPOT, LightEnabled,
    LightHeaderGpu, LightTableDirty, LightingConfig, MAX_LIGHTS, MAX_LIGHTS_PER_CLUSTER, PointLight,
    SPOT_COS_OUTER_MAX, SkyLight, SpotLight, cluster_index,
};
pub use light_policy::{CLUSTER_HI, CLUSTER_LO, LightStats, select_lighting_cull};
pub use light_system::{
    GPU_LIGHT_BYTES, LIGHT_HEADER_BYTES, LightChanged, LightSeedState, LightTableStaging,
    SetLightEnabledById, collect_lights, evict_light, fold_light_table, light_seed_state,
    set_light_enabled_now, write_light_table,
};
pub use gbuffer_depth::{GBUFFER_T_MAX, assert_gbuffer_marcher_t_max_agree};
pub use material::{MATERIAL_GPU_WORDS, MaterialGpu, MaterialId};
pub use mesh_registry::{
    MeshGpu, MeshRegistry, U16_INDEX_VERTEX_LIMIT, VERTEX_STRIDE as MESH_VERTEX_STRIDE, Vertex,
};
pub use ssao_config::{ResolvedSsao, SsaoConfig, SsaoQuality, resolve_ssao, resolve_ssao_policy};
pub use ssao_plugin::SsaoPlugin;
pub use view::{
    composite_from_view, composite_perspective_from_view, demo_view_proj_from_view,
    view_proj_columns,
};
pub use ui::{
    pack_ui_instance, premultiply_rgba8, record_ui_rects, ui_rect_fs_spirv, ui_rect_vs_spirv,
    FLAG_BORDER_ANY, FLAG_CLIP_PRESENT, FLAG_TEXT, FRAMES_IN_FLIGHT as UI_FRAMES_IN_FLIGHT,
    PackInput, UiFramePlan, UiInstance, UiNode, UiOrtho, UiRenderGeneration, UiRenderScratch,
    UiUploadSystem, UI_INSTANCE_SIZE,
};
