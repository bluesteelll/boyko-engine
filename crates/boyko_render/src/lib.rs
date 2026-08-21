//! `boyko_render` — GPU-resident ECS columns (Phase 5).
//!
//! This crate is the bridge between the graphics-pure ECS core ([`boyko_ecs`])
//! and the backend-agnostic RHI ([`boyko_rhi`] / [`boyko_rhi_vulkan`]). It is the
//! ONLY crate that may name both surfaces — the dependency fan-out is `boyko_render`
//! depending DIRECTLY on `boyko_ecs` + `boyko_rhi` + `boyko_rhi_vulkan`
//! (and `boyko_rhi` itself depends only on `boyko_utils`, NOT on
//! `boyko_ecs`), with NO cycle, so the orphan-rule impls (`RhiContext:
//! NonSendResource`) and the graphics-aware types ([`GpuColumnManager`],
//! [`GpuColumnMeta`]) live here, never in `boyko_ecs`. (The former direct
//! `boyko_utils` dependency was never used and was removed by the 2026-07 audit.)
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

/// Asset-streaming plan F2 §1/§3 — the refcount lifetime apply system
/// ([`apply_refcount_deltas`](asset_refcount::apply_refcount_deltas)) that folds
/// the `MeshHandle`/`MaterialHandle` carrier hooks' pushed deltas into the two
/// GPU asset tables, plus the [`AssetRefcountPlugin`](asset_refcount::AssetRefcountPlugin)
/// that wires it into the app schedule. F6 adds the fence-gated device-free drain
/// ([`retire_deferred_frees`](asset_refcount::retire_deferred_frees)), its
/// [`RenderEpoch`](asset_refcount::RenderEpoch) clock resource, and
/// [`RETIRE_DELAY`](asset_refcount::RETIRE_DELAY).
pub mod asset_refcount;
pub mod barrier;
/// T4 — [`BindlessTextureTable`](bindless::BindlessTextureTable): the bindless
/// texture-array descriptor set owner (free-list slot allocator + fence-gated
/// slot recycle + magenta error texture). Registered as a `NonSendResource` at
/// boot by `boyko_app::runner` (textured-PBR rung T6b) — no pipeline binds its
/// descriptor set yet (T6c).
pub mod bindless;
/// Light-object bundle presets ([`DirectionalLightObject`] / [`PointLightObject`]
/// / [`SpotLightObject`]) — named `#[derive(Bundle)]` mixes of scene spatial
/// components with this crate's light components (std-lib S6). Cycle-free: render
/// depends on scene.
pub mod bundles;
/// CSM Inc-1a — the cascade-fit ECS policy ([`CsmConfig`](csm_config::CsmConfig) +
/// [`ResolvedCsm`](csm_config::ResolvedCsm) Resources + the pure
/// [`resolve_csm`](csm_config::resolve_csm) fit + the cold
/// [`resolve_csm_cascades`](csm_config::resolve_csm_cascades) policy). The
/// `docs/CSM-AUTOFIT-PLAN.md` caster-aware split-range fit
/// ([`CsmFitMode`](csm_config::CsmFitMode) knob, default OFF + byte-identical +
/// [`CsmCasterBounds`](csm_config::CsmCasterBounds) +
/// [`CsmFitState`](csm_config::CsmFitState) latch) lives here too.
pub mod csm_config;
/// CSM Increment 2 — the ECS-native shadow-caster gather
/// ([`CsmCasterScratch`](csm_caster::CsmCasterScratch) reused resource +
/// [`gather_shadow_casters`](csm_caster::gather_shadow_casters), the
/// `With<ShadowCaster>`-filtered count→prefix-sum→scatter that produces the cascade
/// depth-pass caster batches, reusing the M3 `gather_mixed_into` core).
pub mod csm_caster;
/// CSM Inc-1a — the structural [`ShadowCaster`](csm_marker::ShadowCaster) capability
/// marker (CSM-casting vs SDF/MDF-occlusion are mutually exclusive by presence).
pub mod csm_marker;
/// CSM Inc-1a — the [`CsmPlugin`](csm_plugin::CsmPlugin) that seeds the config
/// substrate and schedules the cold cascade-fit policy.
pub mod csm_plugin;
/// SDFDDGI I0 — the DDGI irradiance-probe-grid ECS policy
/// ([`DdgiConfig`](ddgi_config::DdgiConfig) +
/// [`ResolvedDdgi`](ddgi_config::ResolvedDdgi) Resources + the pure
/// [`resolve_ddgi`](ddgi_config::resolve_ddgi) fit + the cold
/// [`resolve_ddgi_grid`](ddgi_config::resolve_ddgi_grid) policy +
/// [`sync_ddgi_light_gate`](ddgi_config::sync_ddgi_light_gate), the sole writer of the
/// LightBuf word-7 bit-4 GI gate).
pub mod ddgi_config;
/// SDFDDGI I0 — the [`DdgiPlugin`](ddgi_plugin::DdgiPlugin) that seeds the config
/// substrate and schedules the cold grid-resolve policy under
/// [`DdgiResolveSet`](ddgi_config::DdgiResolveSet).
pub mod ddgi_plugin;
/// SDFDDGI I2 — the probe-update host data + policy: the b6
/// [`DdgiUpdateUbo`](ddgi_update::DdgiUpdateUbo) byte-mirror, the owner-set
/// [`DdgiUpdateConfig`](ddgi_update::DdgiUpdateConfig) knobs, the device-storage degrade gate
/// [`DdgiCaps`](ddgi_update::DdgiCaps) + [`resolve_ddgi_grid_gated`](ddgi_update::resolve_ddgi_grid_gated),
/// the boot [`fill_fibonacci_ray_table`](ddgi_update::fill_fibonacci_ray_table) precompute, and
/// the per-frame [`pack_ddgi_update_ubo`](ddgi_update::pack_ddgi_update_ubo) /
/// [`ddgi_update_dispatch_groups`](ddgi_update::ddgi_update_dispatch_groups) helpers.
pub mod ddgi_update;
pub mod error;
/// The gbuffer ⇄ marcher linear-depth contract (mesh foundation M2): the host
/// [`GBUFFER_T_MAX`](gbuffer_depth::GBUFFER_T_MAX) mirror + the C2 drift guard
/// pinning it to the marcher's `SDF_TRACE_T_MAX`.
pub mod gbuffer_depth;
pub mod gpu3d_instance;
pub mod gpu3d_system;
pub mod gpu_column;
pub mod gpu_system;
/// Pillar B B1 — the interpolation-pair dense component
/// [`GpuTransform3D`](gpu_transform3d::GpuTransform3D) (the first production
/// `#[component(storage = "dense")]` type), its [`TrsPacked`](gpu_transform3d::TrsPacked)
/// TRS packing, and the byte-mirror of the B2 shader's `TransformPair` layout.
pub mod gpu_transform3d;
/// Pillar B B1 — the per-substep interpolation-pair pack system
/// [`pack_gpu_transforms`](gpu_transform_pack::pack_gpu_transforms) (the single-site
/// prev-shuffle) + its [`add_gpu_transform_pack`](gpu_transform_pack::add_gpu_transform_pack)
/// wiring fn.
pub mod gpu_transform_pack;
/// Asset-system rung A3a — the GPU-upload SKELETON:
/// [`GpuUpload`](gpu_upload::GpuUpload) (the trait a resident asset implements to
/// turn its decoded CPU intermediate into a device record) + the generic
/// [`upload_assets`](gpu_upload::upload_assets) drain that drives it. No concrete
/// `GpuUpload` impl exists yet — rung A3b adds the first one.
pub mod gpu_upload;
/// VG R3 step S3 — the HOST ORACLE for the hierarchical-Z pyramid and the two-pass occlusion
/// test: the [`HzbLayout`](hzb::HzbLayout) / [`HzbAxis`](hzb::HzbAxis) integer base map,
/// [`build_pyramid`](hzb::build_pyramid)'s reverse-Z `min` chain, and the guarded
/// [`occlusion_verdict`](hzb::occlusion_verdict). Pure CPU, on no frame path — the reference the
/// eventual GPU pyramid and cull are gated against, landed BEFORE any shader exists so that a
/// later GPU/CPU disagreement is a shader bug rather than a math bug. Plays for the HZB arm
/// exactly the role [`frustum`] plays for the frustum arm.
pub mod hzb;
/// VG R3 piece 1 step P1-1 — the ECS-native depth-pyramid ARMING config
/// ([`HzbConfig`](hzb_config::HzbConfig) + its [`HzbMode`](hzb_config::HzbMode) knob).
/// Capability is structural (`Off` IS disabled); default `Off` is the 0%-gate. No derived
/// carrier and no policy system — the map to "build or not" is the identity, so the render
/// driver reads [`HzbConfig::enabled`](hzb_config::HzbConfig::enabled) directly. Read by
/// nothing in piece 1.
pub mod hzb_config;
/// VG R3 piece 1 step P1-1 — the [`HzbPlugin`](hzb_plugin::HzbPlugin) that seeds the
/// depth-pyramid config substrate. System-less (the
/// [`RenderPathPlugin`](render_path_plugin::RenderPathPlugin) shape).
pub mod hzb_plugin;
/// The per-entity 48-byte model-affine instance column (mesh foundation M3):
/// [`InstanceModelCol`](instance_model::InstanceModelCol), the exact SSBO layout the
/// M1/M2 gbuffer VS reads, + its `GlobalTransform` pack system.
pub mod instance_model;
pub mod light;
pub mod light_plugin;
pub mod light_policy;
pub mod light_reconcile;
pub mod light_system;
/// Asset-system rung A3b — concrete [`AssetLoader`](boyko_ecs::ecs::core::asset::AssetLoader)
/// impls: [`ObjMeshLoader`](loaders::ObjMeshLoader) (Wavefront `.obj` →
/// [`MeshData`](mesh_data::MeshData)) and
/// [`RonMaterialLoader`](loaders::RonMaterialLoader) (a `.mat` text KV format
/// → [`Material`](material::Material)). In-house, no `ron`/`serde`
/// dependency.
pub mod loaders;
/// Test-only: this crate's binding of `boyko_log::probe` to the `Render` target.
#[cfg(test)]
pub(crate) mod log_probe;
pub mod material;
/// Asset-system rung A1 — the GPU-resident mirror of `Assets<Material>`
/// ([`MaterialTable`](material_table::MaterialTable)): a `MeshRegistry`-shaped device
/// SSBO + a per-in-flight-frame staging ring, seeded from ONLY the `gpu` field of the
/// world's [`Assets<Material>`](boyko_ecs::ecs::core::asset::Assets) CPU authority, keyed by
/// [`MaterialId`](material::MaterialId). Replaces the standalone mesh-materials rung
/// M(-1) `MaterialRegistry`.
pub mod material_table;
/// The GPU-resident mesh asset record (mesh foundation M2, asset-system rung A2):
/// [`MeshGpu`] / [`Vertex`], the GPU vertex+index buffers a `MeshHandle` indexes.
pub mod mesh;
/// Asset-system rung A2 — the mesh-domain API over `Assets<MeshGpu>`
/// ([`MeshAssetsExt`](mesh_assets::MeshAssetsExt)): mint (`register_mesh` / `cube` /
/// `plane`), resolve (`mesh` / `try_get`), and teardown (`destroy`), plus the HW-RT
/// `blas_address` / `blas_generation`. Replaces the standalone mesh foundation M2
/// `MeshRegistry` — the records OWN their GPU buffers, so `Assets<MeshGpu>` itself is
/// the GPU-resident table (no separate mirror like [`MaterialTable`](material_table::MaterialTable)).
/// Rung A3b factors the device-upload half out as the free fn
/// [`build_mesh_gpu`](mesh_assets::build_mesh_gpu), shared with
/// [`GpuUpload`](gpu_upload::GpuUpload) for `MeshGpu`.
pub mod mesh_assets;
/// Multi-paradigm render-path plan, rung R-VBGEO (Decision 0 / C1) — the bindless
/// per-mesh geometry table [`MeshGeometryTable`](mesh_geometry_table::MeshGeometryTable)
/// (a sibling of [`BindlessTextureTable`](bindless::BindlessTextureTable)): the VB-only
/// Set-3 device object + the `gMeshMeta[]` backing buffer + the reused
/// [`BindlessSlotAllocator`](bindless::BindlessSlotAllocator), plus the
/// [`MeshGeometryTableSlot`](mesh_geometry_table::MeshGeometryTableSlot) always-present
/// `Option` wrapper resource. Live as of rung R8 (`VB_IMPLEMENTED == true`): armed on a
/// `VisibilityBuffer × Mesh` boot whose device carries the descriptor-indexing
/// prerequisite, `None` on every other boot — see the module doc.
pub mod mesh_geometry_table;
/// Asset-system rung A3b — [`MeshData`](mesh_data::MeshData): the `Send`-safe
/// CPU intermediate [`ObjMeshLoader`](loaders::ObjMeshLoader) decodes into,
/// now `MeshGpu`'s `Asset::Cpu` (replacing the pre-A3b `()` placeholder).
pub mod mesh_data;
/// The ECS-native bucketed instance gather (mesh foundation M3): the
/// [`MeshRenderScratch`](mesh_draw::MeshRenderScratch) reused resource, the
/// per-mesh [`DrawBatch`](mesh_draw::DrawBatch), and the
/// count→prefix-sum→scatter [`gather_mesh_draws`](mesh_draw::gather_mesh_draws) system.
/// VG rung R2c: camera-frustum plane extraction + the conservative AABB rejection test — the HOST
/// half of the per-batch draw cull, and the oracle its GPU half is compared against.
pub mod frustum;
pub mod mesh_draw;
/// HW-RT rung 3b + TAA W3 — the camera view-proj carry for temporal reprojection
/// ([`MotionCam`](motion_cam::MotionCam) UBO + [`MotionCamState`](motion_cam::MotionCamState)
/// persist Resource). Un-walled from `#[cfg(feature = "hwrt")]` (TAA W3): the resolve's
/// camera-only motion-vector reconstruction needs it on BOTH legs (a software TAA build has
/// no `rayQuery`, so it cannot be `hwrt`-gated) — this module carries only the CAMERA pair,
/// which both the hwrt shadow-temporal denoiser and TAA read. TAA rung D1 additionally
/// un-walled [`PrevInstanceModelCol`](instance_model::PrevInstanceModelCol) itself (the
/// per-object PREV-transform component + its sync system) as a pure data-layer reachability
/// change — see that type's own doc. The mesh-MV GPU producer (`gbuffer_mrt_mv`, `motion_vec`,
/// `MotionVecResources`, and the `prev_ring`/`upload_prev_instance_models` upload path) stays
/// `hwrt`-gated (v1.1, per-object mesh motion vectors) — un-walling those is a materially
/// bigger, cross-crate change this rung deliberately did not attempt (see the D1 report).
pub mod motion_cam;
/// VG R3 piece 2 step P2-1 — the structural
/// [`OcclusionCulling`](occlusion_marker::OcclusionCulling) capability marker (a table ZST:
/// presence IS the datum, Axis-1 beside `Enabled<RenderEnabled>`'s Axis-2) plus
/// [`VB_INST_FLAG_OCCLUSION_CULLING`](occlusion_marker::VB_INST_FLAG_OCCLUSION_CULLING), bit 0
/// of the per-instance flags word P2-2 folds into
/// [`VbInstanceRow::flags`](instance_model::VbInstanceRow::flags). Read on the host by both
/// gathers since P2-2; read by NOTHING on the device until piece 3.
pub mod occlusion_marker;
/// Particles P0 — the ECS component vocabulary
/// ([`ParticleEmitter`](particle::ParticleEmitter), the
/// [`EmitterActive`](particle::EmitterActive) enable tag, the refcount-hooked
/// [`ParticleEffectHandle`](particle::ParticleEffectHandle)) plus every GPU-facing POD record
/// ([`ParticleSim`](particle::ParticleSim) 48 B, [`ParticleRender`](particle::ParticleRender) 32 B,
/// [`ParticleCounters`](particle::ParticleCounters) 64 B,
/// [`ParticleDispatchArgs`](particle::ParticleDispatchArgs),
/// [`ParticleDrawArgs`](particle::ParticleDrawArgs),
/// [`EmitRequestGpu`](particle::EmitRequestGpu),
/// [`EffectParamsGpu`](particle::EffectParamsGpu)) with its `offset_of!` layout pins — the
/// generator inputs the particle shaders' word indices are emitted from.
pub mod particle;
/// Particles P0 (D6) — the subsystem's OWN fixed-rate [`ParticleClock`](particle_clock::ParticleClock),
/// advanced from `Time::delta_secs()` on the Main schedule. Deliberately NOT `FixedTime`: creating a
/// `CoreSchedule::Fixed` schedule would flip the whole process's event-update policy (D17).
pub mod particle_clock;
/// Particles P0 (D13/D14) — the owner-set [`ParticleConfig`](particle_config::ParticleConfig) +
/// its [`ParticleMode`](particle_config::ParticleMode) knob, rung P1's independent
/// [`ParticleCollision`](particle_config::ParticleCollision) axis, and rung P2's
/// [`ParticleSortMode`](particle_config::ParticleSortMode) (which also carries R10's
/// motion-vector rule). Capability is structural (`Off` IS disabled); the default is the 0%-gate
/// on all three.
pub mod particle_config;
/// Particles P0 — the authored [`ParticleEffect`](particle_effect::ParticleEffect) asset, its
/// host-side bake into `EffectParamsGpu` (`damping = exp2(-drag·timestep)` and the `(cos, sin)`
/// rotation multiplier as an f32 pair — which is what deletes `exp2` and all trig from the device),
/// and the [`ParticleEffectsExt`](particle_effect::ParticleEffectsExt) mint API.
pub mod particle_effect;
/// Particles P0 — the [`ParticlePlugin`](particle_plugin::ParticlePlugin) that composes the whole
/// ECS half under the D17 containment contract.
pub mod particle_plugin;
/// Particles P0 (A1) — the per-frame emitter fold, the effect-table bake and the carrier refcount
/// fold, plus their two `ScratchColumn`-backed staging Resources.
pub mod particle_system;
/// VG R3 piece 4 rung P4-4 — the ECS-native occlusion-decision CONSUMER config
/// ([`OcclusionConfig`](occlusion_config::OcclusionConfig) + its
/// [`OcclusionMode`](occlusion_config::OcclusionMode) knob), the consumer-side sibling of
/// [`HzbConfig`](hzb_config::HzbConfig)'s producer knob. Capability is structural (`Off` IS
/// disabled); default `Off` is the 0%-gate. No derived carrier and no policy system.
pub mod occlusion_config;
/// VG R3 piece 4 rung P4-4 — the [`OcclusionPlugin`](occlusion_plugin::OcclusionPlugin) that
/// seeds the occlusion-decision config substrate. System-less (the
/// [`HzbPlugin`](hzb_plugin::HzbPlugin) shape).
pub mod occlusion_plugin;
/// HW-RT rung R1 — the dormant unified ray / acceleration-structure backend seam:
/// the [`RayBackendConfig`](ray_backend::RayBackendConfig) derived carrier +
/// [`RayCaps`](ray_backend::RayCaps) device-tier input Resources, the pure
/// [`resolve_ray_backend`](ray_backend::resolve_ray_backend) fit + the cold
/// [`resolve_ray_backend_system`](ray_backend::resolve_ray_backend_system) writer,
/// the [`RayBackend`](ray_backend::RayBackend) / [`RayWorkload`](ray_backend::RayWorkload)
/// / [`RayGeom`](ray_backend::RayGeom) vocab, and the
/// [`RayResolveSet`](ray_backend::RayResolveSet) / [`AsBuildSet`](ray_backend::AsBuildSet)
/// ordering seams. Resolves all-software for every device tier in R1 (dormant — no
/// rendered-pixel change).
pub mod ray_backend;
/// HW-RT rung R1 — the [`RayPlugin`](ray_plugin::RayPlugin) that seeds the dormant
/// ray-backend substrate and schedules the cold resolve under
/// [`RayResolveSet`](ray_backend::RayResolveSet).
pub mod ray_plugin;
/// HW-RT rung 1b — the tunable soft-shadow ECS policy
/// ([`RayShadowConfig`](ray_shadow_config::RayShadowConfig) +
/// [`ResolvedRayShadow`](ray_shadow_config::ResolvedRayShadow) Resources + the pure
/// [`resolve_ray_shadow`](ray_shadow_config::resolve_ray_shadow) fit + the cold
/// [`resolve_ray_shadow_system`](ray_shadow_config::resolve_ray_shadow_system) writer).
/// The ray COUNT bakes into a spec-constant at pipeline build; cone/tmax/tmin/bias flow
/// through the HWRT resolve's binding-20 UBO. Defaults byte-identical to the R2a-4b consts.
pub mod ray_shadow_config;
pub mod render3d_plugin;
/// Asset-streaming plan F7 — the fence-gated retired-GPU-buffer queue
/// ([`RetiredGpuBuffers`](retired_gpu_buffers::RetiredGpuBuffers)) a grown GPU mirror
/// (the material table, the per-slot instance family) routes its superseded buffer
/// through, drained by [`retire_deferred_frees`](asset_refcount::retire_deferred_frees)
/// on the same fence horizon as every other F6/F7 device-free.
pub mod retired_gpu_buffers;
/// HW-RT Rung 3a Step 1 — the ECS-native shadow-denoise config
/// ([`ShadowDenoiseConfig`](shadow_denoise_config::ShadowDenoiseConfig) +
/// [`ResolvedShadowDenoise`](shadow_denoise_config::ResolvedShadowDenoise) Resources + the pure
/// [`resolve_shadow_denoise`](shadow_denoise_config::resolve_shadow_denoise) pack + the cold
/// [`resolve_shadow_denoise_policy`](shadow_denoise_config::resolve_shadow_denoise_policy) writer).
/// The a-trous loop-bound `levels` drives the host dispatch count; the edge-stop scalars flow
/// through a 16-byte std140 UBO. Default [`None`](shadow_denoise_config::ShadowDenoiseMode::None)
/// is the 0%-gate — byte-identical to today (no denoise pass).
pub mod shadow_denoise_config;
/// HW-RT Rung 3a Step 1 — the [`ShadowDenoisePlugin`](shadow_denoise_plugin::ShadowDenoisePlugin)
/// that seeds the config substrate and schedules the cold resolve single-writer.
pub mod shadow_denoise_plugin;
/// Multi-paradigm render-path plan, rung R1 — the config surface + boot-lock resolver: the
/// owner-set [`RenderPathConfig`](render_path_config::RenderPathConfig)
/// ([`RenderPath`](render_path_config::RenderPath) / [`GeometryLegs`](render_path_config::GeometryLegs)),
/// its derived [`ResolvedRenderPath`](render_path_config::ResolvedRenderPath) carrier (Decision
/// 1: resolved exactly ONCE, at boot — no per-frame policy system), the pure
/// [`resolve_render_path`](render_path_config::resolve_render_path) entry point (Rev 5's single
/// `pre_light_consumers` predicate), and the [`DepthKind`](render_path_config::DepthKind) /
/// [`ThinAuxMask`](render_path_config::ThinAuxMask) / [`ShadowSources`](render_path_config::ShadowSources)
/// sub-vocabulary.
pub mod render_path_config;
/// Multi-paradigm render-path plan, rung R1 — the
/// [`RenderPathPlugin`](render_path_plugin::RenderPathPlugin) that seeds the config substrate.
/// No per-frame system (Decision 1 — see [`render_path_config`]'s module doc).
pub mod render_path_plugin;
/// Host plan R7 — the SDF instance path: the per-entity
/// [`SdfPrimitive`](sdf_edit::SdfPrimitive) component (an `SdfEdit` carrier), the reused
/// [`SdfEditStaging`](sdf_edit::SdfEditStaging) gather scratch, the one-shot startup
/// [`collect_sdf_edits`](sdf_edit::collect_sdf_edits) gather, and the
/// [`SdfPlugin`](sdf_edit::SdfPlugin) that composes them.
pub mod sdf_edit;
/// Shadow Inc-1 — the sparse spot/point shadow-atlas slot-assignment policy
/// ([`ShadowConfig`](shadow_atlas::ShadowConfig) +
/// [`ResolvedShadowAtlas`](shadow_atlas::ResolvedShadowAtlas) Resources + the pure
/// [`resolve_shadow_atlas_spots`](shadow_atlas::resolve_shadow_atlas_spots) top-K fit + the
/// cold [`resolve_shadow_atlas`](shadow_atlas::resolve_shadow_atlas) policy +
/// [`pack_atlas_slot`](shadow_atlas::pack_atlas_slot) light-table packing).
pub mod shadow_atlas;
/// Shadow Inc-1 — the structural per-LIGHT
/// [`CastsPunctualShadow`](shadow_marker::CastsPunctualShadow) exact-shadow capability marker.
pub mod shadow_marker;
/// Shadow Inc-1 — the [`ShadowAtlasPlugin`](shadow_plugin::ShadowAtlasPlugin) that seeds the
/// config substrate and schedules the cold slot-assignment policy.
pub mod shadow_plugin;
/// Host plan D4 (R5) — the interpolation SNAP / teleport seam: the
/// [`SnapInterpolation`](snap_interpolation::SnapInterpolation) `EnableTag`, the
/// Main-schedule [`snap_apply`](snap_interpolation::snap_apply) zero-streak system,
/// and the [`TeleportCommandsExt`](snap_interpolation::TeleportCommandsExt) command
/// sugar.
pub mod snap_interpolation;
/// The anti-aliasing config substrate: the owner-set [`AaConfig`](aa_config::AaConfig)
/// ([`AaMode`](aa_config::AaMode) — `Off`/`Fxaa`/…), its derived
/// [`ResolvedAa`](aa_config::ResolvedAa) carrier, and the cold
/// [`resolve_aa_policy`](aa_config::resolve_aa_policy). Mirrors the SSAO substrate; the
/// render driver reads `ResolvedAa` to build the post-process AA pass at the resolve→present
/// seam (`Off` = the byte-identical 0%-gate).
pub mod aa_config;
pub mod aa_plugin;
/// Anti-aliasing Stage 2 — the embedded SMAA 1x LUT binaries ([`smaa_luts::AREA_TEX_BYTES`] /
/// [`smaa_luts::SEARCH_TEX_BYTES`]) extracted from the canonical iryoku headers, pinned by
/// SHA-256 in this module's own unit tests.
pub mod smaa_luts;
pub mod ssao_config;
pub mod ssao_plugin;
/// TAA rungs C1 + T2 — the author-facing [`TaaConfig`](taa_config::TaaConfig) tunable surface
/// (the full knob declaration, clean-architecture-first-time) + the cold
/// [`resolve_taa_policy`](taa_config::resolve_taa_policy) single-writer that completes the
/// [`ResolvedTaa`](aa_config::ResolvedTaa) substrate. C1 wired
/// [`TaaConfig::jitter_scope`](taa_config::TaaConfig::jitter_scope); rung T2 additionally wires
/// `clamp`/`clamp_space`/`clip`/`blend`/`luma_weight`/`history_filter` (live wave-uniform
/// shader branches) plus `disocclusion`/`depth_tol` (forwarded to the UBO, still unread — see
/// the module doc).
pub mod taa_config;
/// Anti-aliasing Stage 4 (TAA) — the raster-only sub-pixel jitter substrate: the [`HALTON_8`
/// table](taa_jitter::HALTON_8), the [`JitterState`](taa_jitter::JitterState) `Resource`
/// singleton, and the pure [`ndc_jitter`](taa_jitter::ndc_jitter) /
/// [`advance_jitter`](taa_jitter::advance_jitter) fns. v1 defaults to jittering ONLY the raster
/// mesh vertex push (see the module docs for the C1 rationale — rung C1 adds an opt-in b5
/// camera-basis shear via [`taa_config::TaaConfig::jitter_scope`]).
pub mod taa_jitter;
/// Anti-aliasing Stage 4 (TAA) — the temporal-resolve history-reset control: the
/// [`TaaState`](taa_state::TaaState) `Resource` singleton the host sets on a `Taa` mode
/// transition or a resize, forcing the next resolve to replace rather than blend.
pub mod taa_state;
/// Per-vertex tangent generation (Lengyel's method,
/// [`generate_tangents`](tangent::generate_tangents)) — a load-time, one-shot pass
/// deriving [`Vertex::tangent`](mesh::Vertex::tangent) from a mesh's final
/// `position`/`normal`/`uv`. Run by the `cube`/`plane` primitives and the `.obj`
/// loader's post-dedup pass.
pub mod tangent;
/// Textured-PBR campaign rung T2 — [`TextureGpu`](texture::TextureGpu): the
/// GPU-resident, mip-chained, bindless-registered texture asset record, its
/// [`ColorSpace`](texture::ColorSpace) sRGB/linear format mapping,
/// [`mip_levels_for`](texture::mip_levels_for), the mip-generating staged upload
/// [`upload_texture_2d`](texture::upload_texture_2d), the domain API
/// ([`TextureAssetsExt`](texture::TextureAssetsExt)), and the F6-style
/// fill-reject queue [`OrphanedTextureGpu`](texture::OrphanedTextureGpu). Loadable
/// (registered + upload-drained + fence-gated-retired) as of rung T6b, but still
/// pixel-dormant — no material references a texture yet and no pipeline binds
/// the bindless set (T6c).
pub mod texture;
/// Textured-PBR campaign rung T2 — [`TextureData`](texture_data::TextureData): the
/// `Send`-safe CPU intermediate [`PngTextureLoader`](loaders::PngTextureLoader)
/// decodes into, `TextureGpu`'s `Asset::Cpu`.
pub mod texture_data;
pub mod ui;
/// Token-typed per-slot ring uploads (host plan R3/R4): the fence-proved camera +
/// instance-model + light-table-staging + CSM-cascade-UBO memcpys the windowed host
/// runs each frame.
pub mod upload;
/// VG-R0 rung R0c: the host-side density reducer — one `vb_id` readback into a census row.
pub mod vg_census;
pub mod view;

pub use asset_refcount::{AssetRefcountPlugin, RETIRE_DELAY, RenderEpoch, apply_refcount_deltas, retire_deferred_frees};
pub use barrier::{PlannedBarrier, lower_barriers};
pub use bindless::BindlessTextureTable;
pub use bundles::{DirectionalLightObject, MeshBundle, PointLightObject, SpotLightObject};
pub use csm_config::{
    CascadeData, CsmCasterBounds, CsmConfig, CsmFit, CsmFitMode, CsmFitState, CsmPcfKernel,
    CsmResolveSet, MAX_CASCADES, RESOLVED_CSM_BYTES, ResolvedCsm, resolve_csm,
    resolve_csm_cascades,
};
pub use csm_caster::{
    CsmCasterScratch, CsmFitSet, gather_shadow_casters, reduce_bounds_into, reduce_caster_bounds,
    sync_csm_light_gate,
};
pub use csm_marker::ShadowCaster;
pub use csm_plugin::CsmPlugin;
pub use ddgi_config::{
    DdgiConfig, DdgiResolveSet, RESOLVED_DDGI_BYTES, ResolvedDdgi, resolve_ddgi, resolve_ddgi_grid,
    sync_ddgi_light_gate,
};
pub use ddgi_plugin::DdgiPlugin;
pub use ray_backend::{
    AsBuildSet, RAY_BACKEND_CONFIG_BYTES, RayBackend, RayBackendConfig, RayBackendPolicy, RayCaps,
    RayGeom, RayResolveSet, RayWorkload, resolve_ray_backend, resolve_ray_backend_system,
};
pub use ray_plugin::RayPlugin;
pub use ray_shadow_config::{
    RESOLVED_RAY_SHADOW_BYTES, RayShadowConfig, ResolvedRayShadow, resolve_ray_shadow,
    resolve_ray_shadow_system,
};
pub use retired_gpu_buffers::RetiredGpuBuffers;
pub use shadow_denoise_config::{
    MAX_ATROUS_LEVELS, RESOLVED_SHADOW_DENOISE_BYTES, RESOLVED_TEMPORAL_SHADOW_BYTES,
    ResolvedShadowDenoise, ResolvedTemporalShadow, ShadowDenoiseConfig, ShadowDenoiseMode,
    resolve_shadow_denoise, resolve_shadow_denoise_policy, resolve_temporal_shadow,
    resolve_temporal_shadow_policy,
};
pub use shadow_denoise_plugin::ShadowDenoisePlugin;
pub use render_path_config::{
    DepthKind, GeometryLegs, RenderPath, RenderPathConfig, RenderPathConsumers,
    RenderPathDegrade, RenderPathDegradeLog, RenderPathDeviceCaps, RenderPathFrozenConsumers,
    ResolvedRenderPath, ShadowSources, ThinAuxMask, effective_ddgi_enabled, effective_ssao_config,
    resolve_render_path, resolve_rules,
};
pub use render_path_plugin::RenderPathPlugin;
// HW-RT rung R1: re-export `RtTier` (defined in `boyko_rhi_vulkan::device`) since the
// crate surfaces the ray-caps tier (`RayCaps`) — a consumer that fills `RayCaps` from a
// device query gets the tier type from here.
pub use boyko_rhi_vulkan::device::RtTier;
pub use ddgi_update::{
    DDGI_DEFAULT_DIMS, DDGI_UPDATE_UBO_BYTES, DEFAULT_RAYS_PER_PROBE, DEFAULT_SUBSET_N, DdgiCaps,
    DdgiUpdateConfig, DdgiUpdateUbo, GI_MAX_RAYS, ddgi_update_dispatch_groups,
    fill_fibonacci_ray_table, pack_ddgi_update_ubo, resolve_ddgi_grid_clamped,
    resolve_ddgi_grid_gated,
};
pub use error::GpuColumnError;
pub use gpu3d_instance::{GPU3D_INSTANCE_SIZE, Gpu3dInstance};
pub use gpu3d_system::sync_gpu_3d_instances;
pub use gpu_transform3d::{
    GPU_TRANSFORM3D_BYTES, GpuTransform3D, TRS_PACKED_BYTES, TrsPacked,
};
pub use gpu_transform_pack::{add_gpu_transform_pack, pack_gpu_transforms};
pub use gpu_upload::{
    GpuUpload, backfill_vb_geometry_slots, upload_assets, upload_material_assets,
    upload_mesh_assets, upload_texture_assets,
};
pub use hzb::{
    HzbAxis, HzbLayout, HzbLayoutError, KeepReason, MAX_HZB_EXTENT, MAX_HZB_LEVELS,
    OcclusionVerdict, ScreenRect, TexelSelection, build_pyramid, conservative_min,
    occluder_depth, occlusion_verdict, prev_pow2, project_aabb, select_texels,
};
pub use hzb_config::{HzbConfig, HzbMode};
pub use hzb_plugin::HzbPlugin;
pub use occlusion_config::{OcclusionConfig, OcclusionMode};
// Particles P0 — the public surface named by the plan's §Public API. No internal type leaks: no
// `BoundBuffer`, `ResId`, `ScratchColumn`, `dyn` or `Vec` appears in any signature re-exported
// here (the two staging Resources expose their lanes as `&[T]` only).
pub use particle::{
    EffectParamsGpu, EmitRequestGpu, EmitterActive, MAX_EFFECTS, MAX_EMITTERS,
    PARTICLE_ADDITIVE_INSTANCE_COUNT_OFFSET, PARTICLE_ALPHA_INSTANCE_COUNT_OFFSET,
    PARTICLE_BLEND_ADDITIVE, PARTICLE_BLEND_ALPHA, PARTICLE_EFFECT_REF_GEN,
    PARTICLE_QUAD_INDEX_COUNT, PARTICLE_SUBSTEP_CEILING, ParticleCounters, ParticleDispatchArgs,
    ParticleDrawArgs, ParticleEffectHandle, ParticleEffectRefDelta, ParticleEffectRefs,
    ParticleEmitter, ParticleRender, ParticleSim, VkDrawIndexedIndirectCommandMirror,
};
pub use particle_clock::{PARTICLE_DEFAULT_HZ, ParticleClock};
pub use particle_config::{
    PARTICLE_DEFAULT_CAPACITY, ParticleCollision, ParticleConfig, ParticleMode, ParticleSortMode,
};
pub use particle_effect::{
    PARTICLE_RAMP_KEYS, PARTICLE_SHAPE_BOX, PARTICLE_SHAPE_CONE, PARTICLE_SHAPE_POINT,
    PARTICLE_SHAPE_SPHERE, ParticleEffect, ParticleEffectsExt, pack_effect_params, pack_f16x2,
};
pub use particle_plugin::ParticlePlugin;
pub use particle_system::{
    ParticleEffectScratch, ParticleEmitScratch, advance_emitter, particle_apply_effect_refs,
    particle_pack_effects, particle_tick_emitters,
};
pub use occlusion_marker::{OcclusionCulling, VB_INST_FLAG_OCCLUSION_CULLING};
pub use occlusion_plugin::OcclusionPlugin;
pub use instance_model::{
    INSTANCE_MODEL_COL_BYTES, InstanceModelCol, VB_INSTANCE_ROW_BYTES, VbInstanceRow,
    sync_instance_model_cols,
};
// HW-RT rung 3b / TAA rung D1: the previous-frame model-affine sibling + its copy system
// (temporal motion vectors). Un-walled from `hwrt` (D1) — see `PrevInstanceModelCol`'s own doc
// for the rationale (mirrors `mod motion_cam`'s W3 un-wall: pure data layer, no GPU consumer
// added). The GPU producer (`gbuffer_mrt_mv`, `motion_vec`, `MotionVecResources`) stays
// `hwrt`-gated, unchanged — `not(hwrt)`'s render output is byte-identical (no plugin adds this
// column on either leg yet).
pub use instance_model::{PrevInstanceModelCol, sync_prev_instance_model_cols};
// HW-RT rung 3b + TAA W3: the camera view-proj carry for motion-vector reprojection.
// Un-walled from `hwrt` (TAA W3) — see the `mod motion_cam` doc for the rationale.
pub use motion_cam::{MOTION_CAM_UBO_BYTES, MotionCam, MotionCamState};
pub use mesh_draw::{
    DrawBatch, MeshRenderScratch, PER_INSTANCE_MATERIAL_BYTES, PER_INSTANCE_MATERIAL_TEX_BYTES,
    PerInstanceMaterial, PerInstanceMaterialTex, gather_mesh_draws, sync_vb_instance_ring_system,
};
pub use light_plugin::LightingPlugin;
pub use light_reconcile::light_reconcile;
pub use render3d_plugin::Render3dPlugin;
pub use gpu_column::{GpuColumnManager, GpuColumnMeta, LOCAL_SIZE_X, ResolvedColumn, RhiContext};
pub use gpu_system::{GpuSystem, gpu_integrate_spirv};
pub use light::{
    CLUSTER_COUNT, CLUSTER_DIM_X, CLUSTER_DIM_Y, CLUSTER_DIM_Z, CLUSTER_FAR_DEFAULT,
    CLUSTER_NEAR_DEFAULT, CSM_MODE_BIT, ClusterCell, ClusterConfig, ClusterSelectMode,
    DDGI_MODE_BIT, DirectionalLight, GPU_LIGHT_WORDS, GpuLight, INDEX_LIST_CAP,
    LIGHT_HEADER_BASE_WORDS,
    LIGHT_HEADER_WORDS, LIGHT_KIND_DIRECTIONAL, LIGHT_KIND_POINT, LIGHT_KIND_SKY, LIGHT_KIND_SPOT,
    LightEnabled, LightHeaderGpu, LightTableDirty, LightingConfig, MAX_LIGHTS,
    MAX_LIGHTS_PER_CLUSTER, PUNCTUAL_MODE_BIT, PointLight, SPOT_COS_OUTER_MAX, SkyLight, SpotLight,
    TERMINATOR_SOFT_MASK, TERMINATOR_SOFT_SHIFT, TONEMAP_MODE_MASK, TONEMAP_MODE_SHIFT, Tonemapper,
    VB_SDF_MESH_AO_BIT, VB_SDF_MESH_MODE_MASK, VB_SDF_MESH_MODE_SHIFT, VB_SDF_MESH_SHADOW_BIT,
    cluster_index, sync_cluster_light_gate, sync_sv0_light_gate,
};
pub use light_policy::{CLUSTER_HI, CLUSTER_LO, LightStats, select_lighting_cull};
pub use light_system::{
    GPU_LIGHT_BYTES, LIGHT_HEADER_BYTES, LightChanged, LightCollectSet, LightSeedState,
    LightTableGeneration, LightTableStaging, SetLightEnabledById, collect_lights, evict_light,
    fold_light_table, fold_light_table_slotted, light_seed_state, set_light_enabled_now,
    write_light_table,
};
pub use gbuffer_depth::{GBUFFER_T_MAX, assert_gbuffer_marcher_t_max_agree, mesh_view_t_norm};
pub use loaders::{ObjMeshLoader, PngTextureLoader, RonMaterialLoader};
pub use material::{
    MATERIAL_FLAG_TEXTURED, MATERIAL_GPU_WORDS, Material, MaterialGpu, MaterialId, MaterialTextures,
};
pub use material_table::MaterialTable;
pub use mesh::{MeshGpu, U16_INDEX_VERTEX_LIMIT, VERTEX_STRIDE as MESH_VERTEX_STRIDE, Vertex};
pub use mesh_assets::{MeshAssetsExt, MeshAssetsVbExt, OrphanedMeshGpu, build_mesh_gpu};
pub use mesh_data::MeshData;
pub use mesh_geometry_table::{
    MESH_BOUNDS_UNKNOWN_COORD, MESH_GEOMETRY_META_BYTES, MESH_LOCAL_BOUNDS_BYTES, MeshGeometryMeta,
    MeshGeometryTable, MeshGeometryTableSlot, MeshLocalBounds, VB_GEOMETRY_RESERVED_SLOT,
    index_width_bytes, mesh_buffer_usage, prefill_bounds_unknown, tri_count,
};
pub use shadow_atlas::{
    ATLAS_SLOT_MASK, ATLAS_SLOT_SHIFT, CASTS_SHADOW_BIT, FaceTransform, M_SLOTS, POINT_FACE_COUNT,
    PointShadowInput, PunctualResolveSet, PunctualSlotAssignment, RESOLVED_SHADOW_ATLAS_BYTES,
    ResolvedShadowAtlas, SHADOW_DIM, SLOT_NONE, ShadowConfig, SpotShadowInput, light_atlas_slot,
    pack_atlas_slot, resolve_shadow_atlas, resolve_shadow_atlas_inputs, resolve_shadow_atlas_spots,
    spot_priority, sync_punctual_light_gate,
};
pub use sdf_edit::{
    MAX_SDF_EDITS, SdfEdit, SdfEditStaging, SdfPlugin, SdfPrimitive, collect_sdf_edits, sdf_kind,
    sdf_op,
};
pub use shadow_marker::CastsPunctualShadow;
pub use shadow_plugin::ShadowAtlasPlugin;
pub use snap_interpolation::{SnapInterpolation, TeleportCommandsExt, snap_apply};
pub use aa_config::{
    AaConfig, AaMode, RESOLVED_TAA_BYTES, ResolvedAa, ResolvedTaa, resolve_aa, resolve_aa_policy,
};
pub use aa_plugin::AaPlugin;
pub use taa_config::{
    BlendMode, ClampShape, ClampSpace, ClipMode, DisocclusionTest, HistoryFilter, JitterScope,
    JitterSequence, MvSource, SharpenMode, TaaConfig, resolve_taa, resolve_taa_policy,
};
pub use taa_jitter::{HALTON_8, JitterState, NdcJitter, advance_jitter, ndc_jitter};
pub use taa_state::TaaState;
pub use smaa_luts::{
    AREA_TEX_BYTES, AREA_TEX_H, AREA_TEX_SHA256, AREA_TEX_W, SEARCH_TEX_BYTES, SEARCH_TEX_H,
    SEARCH_TEX_SHA256, SEARCH_TEX_W,
};
pub use ssao_config::{
    MAX_SSAO_ATROUS_LEVELS, ResolvedSsao, SsaoConfig, SsaoQuality, resolve_ssao,
    resolve_ssao_policy, sync_ssao_light_gate,
};
pub use ssao_plugin::SsaoPlugin;
pub use tangent::generate_tangents;
pub use texture::{
    ColorSpace, OrphanedTextureGpu, TextureAssetsExt, TextureGpu, build_texture_gpu,
    load_material_folder, mip_levels_for, upload_texture_2d, upload_texture_2d_raw,
};
pub use texture_data::TextureData;
pub use upload::{
    upload_atlas_ring, upload_camera_ring, upload_camera_ring_sheared, upload_csm_ring,
    upload_instance_materials, upload_instance_materials_tex, upload_instance_models,
    upload_light_table, upload_pair_out_slot, upload_pair_ring, upload_particle_effects,
    upload_particle_emit_requests, upload_ray_shadow_ring, upload_sdf_edit_list,
    upload_shadow_denoise_ring, upload_taa_ring, upload_temporal_shadow_ring,
    upload_vb_instance_rows,
};
// HW-RT rung R2a-3 / rung 3b: `upload_mesh_ids` is genuinely `hwrt`-only (feeds the TLAS-instance
// packer's BLAS-address resolve). `upload_prev_instance_models` STAYS `hwrt`-gated too, despite
// `PrevInstanceModelCol` itself being un-walled (TAA rung D1) — it reads
// `MeshRenderScratch::prev_ring`, a SEPARATE `hwrt`-only wall in `mesh_draw.rs` this rung does not
// touch (see the D1 report).
#[cfg(feature = "hwrt")]
pub use upload::{upload_mesh_ids, upload_prev_instance_models};
// TAA W3: un-walled from `hwrt` — the resolve's camera-only MV reconstruction needs the
// MotionCam ring upload on BOTH legs (see the `mod motion_cam` doc for the rationale).
pub use upload::upload_motion_cam_ring;
pub use view::{
    composite_from_view, composite_from_view_sheared, composite_perspective_from_view,
    composite_perspective_from_view_sheared, demo_view_proj_from_view,
    forward_gbuffer_push_from_view, forward_view_proj_rows, gbuffer_push_from_view,
    gbuffer_push_from_view_jittered, marcher_view_proj_rows, marcher_view_proj_rows_jittered,
    view_proj_columns,
};
pub use ui::{
    gather_ui_nodes, pack_ui_image_instance, pack_ui_instance, premultiply_rgba8, record_ui_rects,
    ui_rect_fs_spirv, ui_rect_vs_spirv, ui_render_discovery, FLAG_BORDER_ANY, FLAG_CLIP_PRESENT,
    FLAG_TEXT, FLAG_TEXTURED, FRAMES_IN_FLIGHT as UI_FRAMES_IN_FLIGHT, PackInput, UiFramePlan,
    UiGatherScratch, UiImageInput, UiInstance, UiNode, UiOrtho, UiRenderGeneration,
    UiRenderScratch, UiSamplerMode, UiUploadSystem, UI_INSTANCE_SIZE, UI_RECORDS_PER_NODE,
    UI_SLOT_BITS, UI_SLOT_MASK, UI_SLOT_SHIFT, UI_STAGING_ROWS,
};
