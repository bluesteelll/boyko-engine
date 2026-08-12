# Feature map — where to find what (branch `ecs`)

First point of contact for agents. When you need to know *where* a particular
piece of functionality lives, start here, then go to
[SYSTEMS.md](SYSTEMS.md) for details and finally to the source.

**Legend:**
- ✅ Implemented and tested
- ⚠️ Implemented with documented caveats
- 📋 Planned / filed as a future phase
- ❌ Not implemented (deliberately — see linked rationale)

> The `ecs` branch builds clean. The current state is the cumulative result of
> Phases 2 → 22 plus the 9.x executor-soundness series, the X.x perf series
> (X.A `for_each_chunk`, X.B `Unit` removal, X.D EntityMaster slot reduction,
> X.E bench methodology, X.C/X.F/X.G/X.H/X.I reserve/commit storage —
> culminating in X.J retiring the shared Arena), Phases 14a/14b
> (hooks + observers), Phase 19 (parent-child hierarchies on the hook
> substrate), Phase 21 (multi-world hardening), and Phase 22 (tags: static
> ZST + dynamic runtime, the empty archetype). Each phase's authoritative
> record is its `docs/PHASE-*-RESULTS.md`.

> **Anchors are partly gated, and the boundary is stated because it moved four
> times.** `tests/internal_docs_anchors.rs` runs under the ordinary
> `cargo test --workspace` and checks **exactly two notations**: the suffix form
> `file.rs:N` (including `(:N)`) and the bare `(N)` member line. For those: the
> path must exist, line N must still hold a definition, and where a line's
> backticked symbols pair one-to-one with its numbers, line N must also name the
> symbol it stands beside. An anchor written `:N~` / `(N~)` deliberately points
> at a non-definition — a struct field, an enforcement site, a module-doc
> invariant — and waives **both** the shape and the identity check, keeping only
> the in-file bounds check.
>
> **NOT read by the gate:** line numbers spelled `line N` or `(line N)`. Nine
> such sites exist across these documents. That form is invisible — changing one
> to `(line 99999)` leaves the suite green and does not even move the anchor
> count. Write new citations in the `:N` form.
>
> **Binding rule, stated exactly because getting it wrong is what caused the
> worst rot found here:** an anchor binds to the nearest resolvable file-shaped
> path mention since the last heading — a **File:** header, **but also any inline
> markdown link or bare `crates/...` mention.** So an inline file link dropped
> into a member table silently rebinds every row after it, and a table whose
> members live in several files needs its header split.
>
> Do not read this box as a freshness guarantee for the whole document. It states
> which notations are machine-checked; a number in any other form is unverified.

> **Crate layout (19 members).** *Kernel:* `boyko_ecs` (core) · `boyko_macros`
> (derives) · `boyko_utils` (collections) · `boyko_threadpool` (Chase-Lev
> work-stealing pool, on crossbeam-deque primitives). *Std-lib / sim:*
> `boyko_math` (SIMD POD math) · `boyko_scene` (Transform / Camera) ·
> `boyko_sdf_math` (analytic SDF field leaf) · `boyko_physics` (in-house 3D
> TGS-Soft) · `boyko_input` (action mapping) · `boyko_serialize` (binary
> save/load). *Render / UI / shaders:* `boyko_rhi` (RHI trait surface) ·
> `boyko_rhi_vulkan` (raw-FFI Vulkan backend + framegraph) · `boyko_render`
> (GPU-resident columns, lighting, SDF, the boot render-path resolver) ·
> `boyko_shaderdsl` (Rust shader eDSL) ·
> `boyko_fontbake` (MTSDF atlas baker) · `boyko_ui` (ECS-native UI). *Host /
> apps / bench:* `boyko_app` (windowed host: `EnginePlugins`, device-singleton
> boot, the token-fenced G-buffer runner — host plan R2/R3) · `boyko_demo`
> (wgpu+egui sandbox, dogfoods the public API) · `bench_bevy_vs_boyko`
> (comparison benches). This file catalogs the ECS kernel in depth; the std-lib
> and render/UI subsystems are indexed below and detailed per-crate in
> [SYSTEMS.md](SYSTEMS.md).

---

## Quick "I want to …" index

| I want to … | Go to |
|-------------|-------|
| Build an app with a frame loop | [App + Plugin facade](#app--plugin-facade) |
| Fixed timestep / game clock / interpolation alpha | [App + Plugin facade](#app--plugin-facade) (Phase 20 rows) |
| Define a component / resource / bundle / event / system-set | [Macros](#macros-derives) |
| Spawn / despawn / mutate entities directly | [EcsMaster facade](#high-level-facade-ecsmaster) |
| Spawn / despawn deferred (inside a system) | [Commands](#commands--entitycommands-deferred-mutation) |
| Iterate entities with components | [Typed Query DSL](#typed-query-dsl-queryd-f) |
| Tag entities (zero-data markers, static or runtime-named) | [Tags](#tags-static-zst--dynamic-runtime--phase-22) |
| Spawn an entity with zero components | [Tags](#tags-static-zst--dynamic-runtime--phase-22) (`spawn_empty`) |
| Enable/disable a transient flag with NO archetype migration | [EnableTag (enable-bit backend)](#enabletag-enable-bit-non-fragmenting-tag-backend) |
| High-churn / non-fragmenting tag (toggle is O(1), no migration) | [EnableTag (enable-bit backend)](#enabletag-enable-bit-non-fragmenting-tag-backend) |
| `Enabled<T>` / `Disabled<T>` query filter | [EnableTag (enable-bit backend)](#enabletag-enable-bit-non-fragmenting-tag-backend) |
| Data-less bounded global scan of (en/dis)abled entities | [EnableTag (enable-bit backend)](#enabletag-enable-bit-non-fragmenting-tag-backend) (candidate-seeded, D7) |
| SIMD/batched columnar iteration | [for_each_chunk](#chunked--parallel-iteration) |
| Run systems in parallel | [Schedule + scheduler](#schedule--parallel-scheduler) |
| Pick / resolve a render path ({Deferred, Forward, Forward+, VB} × {Both, Mesh, Sdf}) | [Render subsystems](#render--ui--shader-subsystems) (`render_path_config` rows) |
| Verify / re-pin a golden render, or learn the byte-identity gate | [Golden byte-identity harness](#golden-byte-identity-harness-render-regression-gate) |
| Order systems / group into sets | [Ordering & sets](#system-ordering--sets) |
| Conditionally run systems | [Run conditions](#run-conditions-run_if) |
| Application states / state machines | [States](#states) |
| React to component add/remove | [Hooks & observers](#component-lifecycle-hooks--observers) |
| Parent-child hierarchies | [Hierarchies](#hierarchies-parent-child) |
| Detect changed/added components | [Change detection](#change-detection-tick--addedt--changedt) |
| Send/read events between systems | [Events](#events) |
| Shared global data | [Resources](#resources) |
| Low-level component byte storage | [Type-erased component storage](#type-erased-component-storage) |
| Reserve/commit raw memory | [Memory and allocation](#memory-and-allocation) |

### Std-lib / simulation subsystems

| I want to … | Crate + key files |
|-------------|-------------------|
| SIMD-aligned POD math (Vec2/3/4, Quat, Mat3/4, Affine3A, Ray) | `boyko_math` — [vec.rs](../crates/boyko_math/src/vec.rs) · [quat.rs](../crates/boyko_math/src/quat.rs) · [mat.rs](../crates/boyko_math/src/mat.rs) · [affine.rs](../crates/boyko_math/src/affine.rs) · [ray.rs](../crates/boyko_math/src/ray.rs) |
| Transform / GlobalTransform + hierarchy propagation | `boyko_scene` — [transform.rs](../crates/boyko_scene/src/transform.rs) · [propagation.rs](../crates/boyko_scene/src/propagation.rs) |
| Camera / camera rig / ViewUniform / visibility | `boyko_scene` — [camera.rs](../crates/boyko_scene/src/camera.rs) · [camera_plugin.rs](../crates/boyko_scene/src/camera_plugin.rs) · [visibility_sync.rs](../crates/boyko_scene/src/visibility_sync.rs) · [render_caps.rs](../crates/boyko_scene/src/render_caps.rs) |
| Rigid-body physics (3D TGS-Soft solver, narrowphase, contacts) | `boyko_physics` — [solver/](../crates/boyko_physics/src/solver/) · [soft/](../crates/boyko_physics/src/soft/) · [narrowphase/](../crates/boyko_physics/src/narrowphase/) · [components.rs](../crates/boyko_physics/src/components.rs) · [plugin.rs](../crates/boyko_physics/src/plugin.rs) |
| Body-vs-SDF collision (CPU field query, zero readback) | `boyko_physics` — [sdf_query.rs](../crates/boyko_physics/src/sdf_query.rs) + `boyko_sdf_math` |
| Analytic SDF edit-list field (shared GPU golden + CPU physics) | `boyko_sdf_math` — [lib.rs](../crates/boyko_sdf_math/src/lib.rs) · [brick.rs](../crates/boyko_sdf_math/src/brick.rs) · [mesh_sdf.rs](../crates/boyko_sdf_math/src/mesh_sdf.rs) |
| Rebindable input actions (raw events → typed actions) | `boyko_input` — [raw/](../crates/boyko_input/src/raw/) · [action/](../crates/boyko_input/src/action/) · [win32.rs](../crates/boyko_input/src/win32.rs) · [plugin.rs](../crates/boyko_input/src/plugin.rs) |
| Save / load a world (custom binary; codegen not reflection) | `boyko_serialize` — [save.rs](../crates/boyko_serialize/src/save.rs) · [load.rs](../crates/boyko_serialize/src/load.rs) · [format.rs](../crates/boyko_serialize/src/format.rs) |

### Render / UI / shader subsystems

| I want to … | Crate + key files |
|-------------|-------------------|
| Backend-agnostic RHI (device / buffers / pipelines / encoder) | `boyko_rhi` — [api.rs](../crates/boyko_rhi/src/api.rs) · [device.rs](../crates/boyko_rhi/src/device.rs) · [encoder.rs](../crates/boyko_rhi/src/encoder.rs) · [handle.rs](../crates/boyko_rhi/src/handle.rs) |
| Raw-FFI Vulkan backend (loader/device/suballocator/swapchain) | `boyko_rhi_vulkan` — [rhi_impl/](../crates/boyko_rhi_vulkan/src/rhi_impl/) · [device.rs](../crates/boyko_rhi_vulkan/src/device.rs) · [suballocator.rs](../crates/boyko_rhi_vulkan/src/suballocator.rs) · [swapchain.rs](../crates/boyko_rhi_vulkan/src/swapchain.rs) · [compute.rs](../crates/boyko_rhi_vulkan/src/compute.rs) |
| Render Dependency Graph (declare → compile barriers → execute) | `boyko_rhi_vulkan` — [framegraph/](../crates/boyko_rhi_vulkan/src/framegraph/) |
| Choose the render path + geometry legs (`RenderPath::{Deferred, Forward, ForwardPlus, VisibilityBuffer}` × `GeometryLegs::{Both, Mesh, Sdf}`) | `boyko_render` — [render_path_config.rs](../crates/boyko_render/src/render_path_config.rs) (`RenderPathConfig` owner knob → `resolve_render_path` → the immutable `ResolvedRenderPath` carrier; `GeometryLegs` leg-disable, the `RenderPathDegrade` ladder + `RenderPathDegradeLog`) · [render_path_plugin.rs](../crates/boyko_render/src/render_path_plugin.rs). Spec: [MULTI-PARADIGM-RENDER-PLAN.md](MULTI-PARADIGM-RENDER-PLAN.md) |
| Read *where* the path is committed (boot-once, never per-frame) | `boyko_app` — [runner.rs](../crates/boyko_app/src/runner.rs) (`run_windowed`'s boot section calls `resolve_render_path` exactly once and OVERWRITES the plugin's default `ResolvedRenderPath` — Decision 1; a live per-frame path/leg toggle is forbidden by design, it would re-allocate fixed-size images/pipelines mid-stream) · [plugins.rs](../crates/boyko_app/src/plugins.rs) (`BOYKO_RENDER_PATH` / `BOYKO_GEOMETRY_LEGS` dev/test launch seam) |
| Per-path framegraph + target/descriptor profile | `boyko_rhi_vulkan` — [present/graph_bridge.rs](../crates/boyko_rhi_vulkan/src/present/graph_bridge.rs) (`declare_frame_graph` → `declare_deferred_graph` / `declare_forward_graph` / `declare_vb_graph`) · [present/targets.rs](../crates/boyko_rhi_vulkan/src/present/targets.rs) (`TargetsProfile::{DeferredFull, DeferredMeshOnly, DeferredSdfOnly, ForwardMesh, VbMesh}`) · [present/scene_types.rs](../crates/boyko_rhi_vulkan/src/present/scene_types.rs) (the `GBufferScene` path predicates the declarators branch on) |
| Visibility-buffer pass chain (id raster → classify → resolve/shade) | `boyko_rhi_vulkan` — [present/passes/vb.rs](../crates/boyko_rhi_vulkan/src/present/passes/vb.rs) (`record_vb`) · [shaders/](../crates/boyko_rhi_vulkan/shaders/) — `vb_raster.vs/.fs.hlsl` → `vb_resolve.comp.hlsl` (fused) or `vb_classify_{count,scan,scatter}.comp.hlsl` → `vb_shade.comp.hlsl` (material-classified), shared `vb_pack.hlsli` / `vb_geom_fetch.hlsli` + `boyko_render` — [mesh_geometry_table.rs](../crates/boyko_render/src/mesh_geometry_table.rs) (bindless per-mesh geometry, `ResolvedRenderPath::vb_geometry_table`) |
| VB geo/shade split (thin aux for pre-light consumers) | `boyko_render` — [render_path_config.rs](../crates/boyko_render/src/render_path_config.rs) (`mesh_geo_shade_split` / `sdf_geo_shade_split` — ONE `pre_light_consumers` union: SSAO ∥ DDGI ∥ spatial shadow denoise ∥ shadow temporal ∥ SSR; `ThinAuxMask`, `ShadowSources`, `DepthKind`) + `boyko_rhi_vulkan` — shaders `vb_geo.comp.hlsl` (+ `MOTION` variant) · `vb_shade_split.comp.hlsl` (+ `TEXTURED`/`HWRT` variants) |
| Two-phase HZB occlusion culling (owner knob · capability marker · depth pyramid) | **Knobs:** `boyko_render` — [occlusion_config.rs](../crates/boyko_render/src/occlusion_config.rs) (`OcclusionConfig { mode: OcclusionMode }` — the CONSUMER knob, two variants, **default `Off`**, read live per frame) · [hzb_config.rs](../crates/boyko_render/src/hzb_config.rs) (`HzbConfig`/`HzbMode` — the PRODUCER knob) · [occlusion_plugin.rs](../crates/boyko_render/src/occlusion_plugin.rs) / [hzb_plugin.rs](../crates/boyko_render/src/hzb_plugin.rs). **Capability:** [occlusion_marker.rs](../crates/boyko_render/src/occlusion_marker.rs) — `OcclusionCulling` is a ZST whose PRESENCE is the datum; `Off` means *do not test*, never *do not gather*. **Host:** `boyko_app` — [hzb_plan.rs](../crates/boyko_app/src/hzb_plan.rs) (`hzb_plan_for` — a pyramid is planned iff a producer asks **or** a consumer needs one) · [occlusion_arm.rs](../crates/boyko_app/src/occlusion_arm.rs) (`occlusion_arm_for` → `VbOcclusionArm`) · [occlusion_force.rs](../crates/boyko_app/src/occlusion_force.rs) (`OcclusionForce{None,KeepAll,DeferAll}` — the **diagnostic** verdict override; NOT owner surface) · [hzb_dump.rs](../crates/boyko_app/src/hzb_dump.rs). **Verdict oracle (host mirror of the shader):** [hzb.rs](../crates/boyko_render/src/hzb.rs). **Device:** `boyko_rhi_vulkan` — [present/passes/vb.rs](../crates/boyko_rhi_vulkan/src/present/passes/vb.rs) (`record_vb`'s early/late raster scopes, `record_hzb_poison_build`), shaders `vb_batch_cull.comp.hlsl` / `hzb_build.comp.hlsl`; the arming predicate is `GBufferScene::path_vb_occlusion_split()` ([present/scene_types.rs](../crates/boyko_rhi_vulkan/src/present/scene_types.rs)). **Test-side single insert site:** [crates/boyko_app/tests/occ_fixture/](../crates/boyko_app/tests/occ_fixture/) — one module owns the `BOYKO_VG_OCC`/`BOYKO_VG_OCC_FORCE` decode AND the Resource insert for every App-booting fixture, which is what makes "one edit disarms five pins" a gate-visible event. Spec: [VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md](VG-R3-P4-CONFIG-AND-INSTRUMENT-PLAN.md) |
| Froxel (clustered) light cull | `boyko_render` — [light.rs](../crates/boyko_render/src/light.rs) (`ClusterCell`, `CLUSTER_DIM_*` grid consts, `cluster_index` — the ONE host-side linearization the cull-write and resolve-read shaders must match byte-identically) · [light_system.rs](../crates/boyko_render/src/light_system.rs) · [light_policy.rs](../crates/boyko_render/src/light_policy.rs) (banded `ClusterSelectMode::Auto`) + `boyko_rhi_vulkan` — shaders `cluster_cull.hlsl` (base + `HIER` hierarchical variant) · `light_table.hlsli`; the `light_cull` graph pass. Under VB the whole machinery hangs off the single boot-frozen `ResolvedRenderPath::froxel_light_cull` arm bit (default off ⇒ nothing built/declared/recorded) |
| GPU-resident ECS component columns (DeviceLocal VRAM pools) | `boyko_render` — [gpu_column.rs](../crates/boyko_render/src/gpu_column.rs) · [gpu_system.rs](../crates/boyko_render/src/gpu_system.rs) |
| GPU instancing / mesh draw / 3D instances | `boyko_render` — [mesh_draw.rs](../crates/boyko_render/src/mesh_draw.rs) · [gpu3d_instance.rs](../crates/boyko_render/src/gpu3d_instance.rs) · [gpu3d_system.rs](../crates/boyko_render/src/gpu3d_system.rs) |
| Lighting (directional / point / spot / clustered cull) | `boyko_render` — [light.rs](../crates/boyko_render/src/light.rs) · [light_system.rs](../crates/boyko_render/src/light_system.rs) · [light_plugin.rs](../crates/boyko_render/src/light_plugin.rs) |
| Shadows (CSM cascades + punctual atlas) | `boyko_render` — [csm_config.rs](../crates/boyko_render/src/csm_config.rs) · [csm_caster.rs](../crates/boyko_render/src/csm_caster.rs) · [shadow_atlas.rs](../crates/boyko_render/src/shadow_atlas.rs) |
| Textured PBR materials (bindless textures: albedo/normal/metal-rough/AO/emissive) | `boyko_render` — [texture.rs](../crates/boyko_render/src/texture.rs) · [texture_data.rs](../crates/boyko_render/src/texture_data.rs) · [bindless.rs](../crates/boyko_render/src/bindless.rs) · [tangent.rs](../crates/boyko_render/src/tangent.rs) · [loaders/png_texture.rs](../crates/boyko_render/src/loaders/png_texture.rs) + `boyko_rhi_vulkan` — [bindless.rs](../crates/boyko_rhi_vulkan/src/bindless.rs) |
| Textured G-buffer + tonemap/terminator-wrap resolve shader variants | `boyko_rhi_vulkan` — [compute.rs](../crates/boyko_rhi_vulkan/src/compute.rs) (`gbuffer_mrt_tex_vs_spirv`/`gbuffer_mrt_tex_fs_spirv`, `deferred_pbr_spirv`/`deferred_pbr_wrap_spirv`) |
| Decode a PNG (in-house, zero third-party dependency) | `boyko_image` — [lib.rs](../crates/boyko_image/src/lib.rs) · [png.rs](../crates/boyko_image/src/png.rs) · [inflate.rs](../crates/boyko_image/src/inflate.rs) |
| Author shader math once (Rust eDSL → f32 mirror + HLSL) | `boyko_shaderdsl` — [field.rs](../crates/boyko_shaderdsl/src/field.rs) · [marcher.rs](../crates/boyko_shaderdsl/src/marcher.rs) · [emit/](../crates/boyko_shaderdsl/src/emit/) · [scalar.rs](../crates/boyko_shaderdsl/src/scalar.rs) |
| Bake an MTSDF font atlas → .bfont | `boyko_fontbake` — [face.rs](../crates/boyko_fontbake/src/face.rs) · [extract.rs](../crates/boyko_fontbake/src/extract.rs) · [msdf/](../crates/boyko_fontbake/src/msdf/) · [atlas.rs](../crates/boyko_fontbake/src/atlas.rs) |
| ECS-native UI (widgets = entities; layout systems; MSDF text) | `boyko_ui` — [layout.rs](../crates/boyko_ui/src/layout.rs) · [components.rs](../crates/boyko_ui/src/components.rs) · [text/](../crates/boyko_ui/src/text/) · [widgets.rs](../crates/boyko_ui/src/widgets.rs) · [interaction/](../crates/boyko_ui/src/interaction/) |
| World-space / diegetic 3D HUD (cursor-ray pick, depth-occlude) | `boyko_ui` — [world/](../crates/boyko_ui/src/world/) |
| Data-bind UI to ECS state / hot-reload `.ui` markup | `boyko_ui` — [binding/](../crates/boyko_ui/src/binding/) · [reload/](../crates/boyko_ui/src/reload/) · [text/](../crates/boyko_ui/src/text/) (`.ui` format) |
| Open a window + run the frame loop (device boot, runner, teardown) | `boyko_app` — [plugins.rs](../crates/boyko_app/src/plugins.rs) (`EnginePlugins`) · [runner.rs](../crates/boyko_app/src/runner.rs) · [host.rs](../crates/boyko_app/src/host.rs) · [device.rs](../crates/boyko_app/src/device.rs) (`GpuDevice`) |
| Windowed G-buffer scene host (static bundles, token-fenced uploads) | `boyko_app` — [gpu_scene/](../crates/boyko_app/src/gpu_scene/) (`mod.rs` boot, `csm.rs`, `interp.rs`, `tlas.rs`) + `boyko_render` — [upload.rs](../crates/boyko_render/src/upload.rs) · [view.rs](../crates/boyko_render/src/view.rs) (`gbuffer_push_from_view`) |
| Spawn a drawable mesh from ECS (bundle + primitives + example) | `boyko_render` — [bundles.rs](../crates/boyko_render/src/bundles.rs) (`MeshBundle`) · [mesh_assets.rs](../crates/boyko_render/src/mesh_assets.rs) (`MeshAssetsExt::cube`/`plane` on `Assets<MeshGpu>` — the asset-system fold that replaced the standalone `MeshRegistry`) + `boyko_app` — [examples/room.rs](../crates/boyko_app/examples/room.rs) |
| Light a windowed scene from ECS light entities (host plan R4) | `boyko_render` — [light_system.rs](../crates/boyko_render/src/light_system.rs) (`LightTableGeneration`) · [upload.rs](../crates/boyko_render/src/upload.rs) (`upload_light_table`) + `boyko_app` — [light_gate.rs](../crates/boyko_app/src/light_gate.rs) · [runner.rs](../crates/boyko_app/src/runner.rs) (staging ring, gen gate) |
| Sun shadows in the windowed host (CSM arming + caster-driven draws) | `boyko_render` — [csm_caster.rs](../crates/boyko_render/src/csm_caster.rs) (`sync_csm_light_gate`) · [upload.rs](../crates/boyko_render/src/upload.rs) (`upload_csm_ring`) + `boyko_app` — [gpu_scene/mod.rs](../crates/boyko_app/src/gpu_scene/mod.rs) (`CsmDepthActivation` arming) · [examples/room.rs](../crates/boyko_app/examples/room.rs) (`ShadowCaster` cubes) |
| Order Fixed gameplay vs engine snapshots (host plan D4 seam) | `boyko_scene` — [sets.rs](../crates/boyko_scene/src/sets.rs) (`FixedSet`) — wired by `EnginePlugins` |

---

## High-level facade (EcsMaster)

The world object. Owns the entity manager, archetype manager (whose pools own
their backing reservations), resources, event dispatcher, change-detection
tick, the deferred-hook queue, and the per-`(D,F)` query/bundle caches.

The type is declared once and its inherent `impl` blocks are split one file per
API surface under
[core/ecs_master/](../crates/boyko_ecs/src/ecs/core/ecs_master/) — the
refactoring campaign turned the god-file into a directory. A member's line
number therefore means nothing without the file that declares it, so each table
below is read against the **File:** line directly above it, and that is the
binding the anchor gate checks.

**File:** [core/ecs_master/ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) — the struct, construction, the typed query entry, teardown.

| What you want to do | Method (line) |
|---------------------|---------------|
| Construct an ECS instance | `EcsMaster::new()` (426) / `with_capacity(entity_cap, arch_cap)` (473) |
| Direct typed query (no SystemParam) | `query::<D, F>() -> QueryView<'_, D, F>` (825) |
| Drop everything | `clear()` (1026) |
| Spawn many (typed bundle) | `spawn_batch::<B, I>(iter) -> EcsResult<Vec<Entity>>` (1079) |

**File:** [core/ecs_master/entity_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/entity_api.rs) — archetypes, spawn, despawn.

| What you want to do | Method (line) |
|---------------------|---------------|
| Create an archetype | `create_archetype(&[ComponentId])` (48) / `get_or_create_archetype(...)` (55) |
| Spawn (raw byte API) | `create_entity(arch_id, &[(ComponentId, &[u8])]) -> EcsResult<Entity>` (137) |
| Spawn (typed, 1–2 comps) | `spawn_one::<A>(arch, a)` (582) / `spawn_two::<A, B>(arch, a, b)` (618) |
| Spawn with ZERO components (Phase 22) | `spawn_empty() -> Entity` (667) — the empty archetype is created lazily on first use |
| Delete an entity | `delete_entity(entity) -> bool` (798) |

**File:** [core/ecs_master/component_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/component_api.rs) — per-entity component access.

| What you want to do | Method (line) |
|---------------------|---------------|
| Read a component (raw) | `get_component_raw(entity, id)` (176) |
| Write a component (raw bytes) | `set_component_raw(entity, id, &[u8])` (444) |
| Mutate a component (change-tracked) | `get_component_mut::<T>(entity) -> Option<Mut<'_, T>>` (553) |
| Check component presence | `has_component(entity, id)` (673) |

**File:** [core/ecs_master/entity_query_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/entity_query_api.rs) — cold inspection of the entity set.

| What you want to do | Method (line) |
|---------------------|---------------|
| Check entity presence | `has_entity(entity)` (17) |
| Counts | `entity_count()` (69) / `archetype_count()` (75) |
| Iterate entities (cold inspection) | `iter_entities()` (87) — O(capacity) fast-store scan |
| Query entity IDs by components | `query_entities(&[ComponentId]) -> Vec<Entity>` (92) — allocates; prefer the typed query DSL |

**File:** [core/ecs_master/tag_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/tag_api.rs) — dynamic (runtime-named) tags, Phase 22.

| What you want to do | Method (line) |
|---------------------|---------------|
| Mint a tag id | `try_register_tag(name)` (47) / `register_tag(name)` (65) |
| Look one up | `tag_by_name(name)` (76) / `has_tag(entity, tag)` (89) — one signature bit test |
| Toggle one | `add_tag(entity, tag)` (130) / `remove_tag(entity, tag)` (200) |

**File:** [core/ecs_master/system_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/system_api.rs) — one-shot system runners.

| What you want to do | Method (line) |
|---------------------|---------------|
| Run a closure as a system once | `run_system::<F, M, Out>(system) -> Out` (111) |
| Run a pre-built cached system | `run_cached_system::<S>(&mut system) -> S::Out` (142) |

**File:** [core/ecs_master/resource_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/resource_api.rs) — world-global singletons.

| What you want to do | Method (line) |
|---------------------|---------------|
| Resources | `insert_resource::<R>` (21) / `resource::<R>() -> &R` (52) / `resource_mut::<R>() -> &mut R` (76) |

**File:** [core/ecs_master/observer_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/observer_api.rs) — hooks and observers (Phases 14a / 14b).

| What you want to do | Method (line) |
|---------------------|---------------|
| Hooks (runtime) | `register_component_hooks::<C>() -> ComponentHooksBuilder` (91) |
| Observers (typed) | `observe_on_add::<C>(runner)` (143) / `observe_on_insert` (151) / `observe_on_replace` (161) / `observe_on_remove` (170) |
| Observers (type-erased) | `add_observer(kind, cid, runner)` (182) / `remove_observer(id)` (199) |

**File:** [core/ecs_master/state_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/state_api.rs) — direct state access (Phase 17).

| What you want to do | Method (line) |
|---------------------|---------------|
| States (direct) | `insert_state` (33) / `init_state` (51) / `state::<S>()` (62) / `set_next_state` (85) |

**File:** [core/ecs_master/event_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/event_api.rs) — direct event send and frame swap.

| What you want to do | Method (line) |
|---------------------|---------------|
| Events (direct) | `send_event::<E>(thread_index, event)` (50) / `update_events()` (68) |

The remaining surfaces live in sibling files of the same directory and are
documented at their own sections:
[bundle_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/bundle_api.rs),
[enable_tag_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/enable_tag_api.rs),
[relationship_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/relationship_api.rs).

Spawn / fallible paths return `EcsResult<T>` — see
[core/error.rs](../crates/boyko_ecs/src/ecs/error.rs) for the
`#[non_exhaustive] enum EcsError` (C-019 closed: the historical `anyhow`
dependency is gone). The two-phase commit pattern (C-007 + C-009) guarantees a
failed spawn leaks no EntityIDs.

---

## App + Plugin facade

Builder over `EcsMaster` + `ScheduleBuilder` + `Schedule` + `ThreadPool`
(Phase 18). `App::new().add_plugins(..).add_systems_cfg(..).run()`. Re-exported
at the crate root: `boyko_ecs::{App, Plugin, Plugins, AppExit}`.

**Files:** [core/app/](../crates/boyko_ecs/src/ecs/core/app/) —
`app.rs`, `plugin.rs`, `plugins.rs`, `app_exit.rs`.

| What you want to do | Where | Method (line) |
|---------------------|-------|---------------|
| Construct an app | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `App::new()` (201) / `with_threads(n)` (208) / `with_pool(Arc<ThreadPool>)` (214) |
| Add a plugin / plugin tuple | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `add_plugin::<P>` (550); `add_plugins((A, B, ..))` via the sealed `Plugins` trait ([plugins.rs](../crates/boyko_ecs/src/ecs/core/app/plugins.rs), 1..=12 + nesting) |
| Insert a resource / state | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `insert_resource` (261) / `init_state` (273) / `insert_state` (289) |
| Add systems (ordered) | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `add_systems_cfg(\|b: &mut ScheduleBuilder\| …)` (318) — full Phase-15/16/17 chaining |
| Add a system (unordered) | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `add_systems(system)` (339) |
| Add a one-shot startup system | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `add_startup_system(system)` (508) — runs once before the loop |
| Run the loop | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `run() -> AppExit` (dispatches to an installed runner first, else loops until `AppExit(true)`), `run_n(frames)`, `update()` (self-clocked via `Instant`) |
| Hand the loop to a host (windowed runner) | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `set_runner(Box<dyn FnOnce(&mut App) -> AppExit>)` — `run()` hands control to it BEFORE `finish()`; the runner owns `finish()`, the `AppExit` policy, and teardown (APP-HOST-PLAN rung R1) |
| Run one frame with an external clock | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `update_with_delta(raw)` — the Phase-20 frame driver (① Time → ② check-ticks → ③ event swap → ④ fixed loop → ⑤ Main); `run_n_with_delta(frames, delta)` — the deterministic loop for tests/benches |
| Fixed-timestep systems | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `add_systems_in(CoreSchedule::Fixed, system)` / `add_systems_cfg_in` / `init_state_in` / `insert_state_in` — closed `CoreSchedule { Main, Fixed }` set (Phase 20 D5); fixed systems read `Res<FixedTime>` |
| Configure the fixed timestep | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `set_fixed_timestep(Duration)` / `set_fixed_hz(f64)` — default exactly 64 Hz; config phase only |
| Read the clock from a system | [core/time/](../crates/boyko_ecs/src/ecs/core/time/) ✅ | `Res<Time>` (per-frame: `delta_secs` / `elapsed` / `pause` / `set_relative_speed` / `set_max_delta`) or `Res<FixedTime>` (per-substep: `delta_secs() == timestep`, `steps_this_frame`) |
| Interpolation alpha | [time/fixed_time.rs](../crates/boyko_ecs/src/ecs/core/time/fixed_time.rs) ✅ | `FixedTime::overstep_fraction()` ∈ [0, 1) — read from Main after the fixed loop (Phase 20 D9; snapshot/lerp layers on top) |
| Render interpolation alpha → GPU lerp (reference consumer) | [boyko_demo render/instance.rs](../crates/boyko_demo/src/render/instance.rs) ✅ | `FixedTime::overstep_fraction()` → 80 B camera uniform → WGSL `mix(prev_pos, pos, alpha)` over the 24 B `GpuInstance` — the Phase-20.1 demo GPU mirror (`sync_gpu_instance` maintains `prev_pos`) |
| Event swap policy | [app.rs](../crates/boyko_ecs/src/ecs/core/app/app.rs) ✅ | `set_event_update_policy(EventUpdatePolicy::{WaitForFixed, EveryFrame})` — auto-resolved at `finish()` (Phase 20 D6); see the `WaitForFixed` pause-hold hazard doc |
| Drive the rhythm without an App (wasm / hand-rolled) | [time/fixed_loop.rs](../crates/boyko_ecs/src/ecs/core/time/fixed_loop.rs) ✅ | `Time::advance_with(raw)` then `fixed_advance(world, \|w\| …)` exactly once per frame — insert `Time`/`FixedTime` manually first (the wasm demo runner's path) |
| The plugin trait | [plugin.rs](../crates/boyko_ecs/src/ecs/core/app/plugin.rs) ✅ | `trait Plugin { fn build(&self, &mut App); fn name(&self) -> &'static str }` — `'static`, NOT `Send + Sync`; consumed at build |
| Exit signal | [app_exit.rs](../crates/boyko_ecs/src/ecs/core/app/app_exit.rs) ✅ | `AppExit(bool)` resource (hand-impls `Resource` — see [PHASE-18-RESULTS.md](archive/PHASE-18-RESULTS.md) macro-cycle note) |

`App` is `!Send + !Sync` (single-threaded-owned). Multi-schedule landed in
Phase 20 as the CLOSED `CoreSchedule` set — a user-mintable label map remains
deliberately rejected (D5; no `HashMap` on the frame path). `set_runner`
landed as [APP-HOST-PLAN](APP-HOST-PLAN.md) rung R1 (see the table above).
Still DEFERRED: SubApps, `PluginGroup`/`DefaultPlugins`,
`App::with_world` (Phase 20.1) — see [PHASE-18-RESULTS.md](archive/PHASE-18-RESULTS.md)
+ [PHASE-20-RESULTS.md](archive/PHASE-20-RESULTS.md).

---

## Macros (derives)

**File:** [boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs).
`boyko_macros` is a **dev-dependency** of `boyko_ecs` (cycle constraint, Phase
18) — import derives directly: `use boyko_macros::{Component, Resource, Bundle, SystemSet};`.

| Macro | What it generates |
|-------|-------------------|
| `#[derive(Component)]` ✅ | `Component` impl (lazy `component_id()` via per-type `OnceLock`) + inherent `SIZE`/`ALIGN`/`layout()` consts. Since Phase 22 it ALSO emits a single-component `Bundle` (so `commands.spawn(PlayerTag)` works; requires `Send + Sync + Unpin` — named const-assert diagnostic) — suppressed by the `#[component(no_bundle)]` flag key. ZSTs are auto-detected tags (size 0, no attribute). Optional `#[component(on_add = path, …)]` binds Phase-14a lifecycle hooks (mutually exclusive with the runtime builder). |
| `#[derive(Resource)]` ✅ | `Resource` impl (lazy `resource_id()`); panics if the type is already a `Component` (audit M6). |
| `#[derive(Bundle)]` ✅ | `Bundle` impl over a named struct (sealed; `Send + Sync + Unpin + 'static`). Tuple bundles were dropped in Phase 8.5 — named structs only. |
| `#[derive(SystemSet)]` ✅ | `SystemSet` impl for fieldless enums (variant → discriminant). Data-carrying variants / unions / generics rejected (Phase 15). |
| `#[event]` ✅ | Rewrites a user struct with `#[participant(...)]` / `#[parameter]` fields into a two-field `{ participants, parameters }` native layout + `Event` impl. |

---

## Entities

| What you want to do | Where | How |
|---------------------|-------|-----|
| Construct an Entity literal | [core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs) ✅ | `Entity::new(id, generation)` / `with_id(id)` |
| Compare entities (id + generation) | [core/entity/entity.rs](../crates/boyko_ecs/src/ecs/core/entity/entity.rs) ✅ | `e1 == e2` — compares BOTH fields (load-bearing ABA defence) |
| Allocate an entity (recycle if available) | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `EntityMaster::allocate_entity()` (124) — recycles from `free_entity_ids`, else `fetch_add` on the atomic |
| Register into the fast store | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `register_entity_with_ptr(entity, *mut Archetype, row)` / `register_batch(...)` |
| Validate an entity (gen-checked) | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `is_entity_valid(entity)` / `get_entity(id)` |
| Deallocate (bumps generation) | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `deallocate_entity(entity) -> bool` (decrements `live_count` on success only) |
| Iterate only LIVE entities | [core/entity/entity_master.rs](../crates/boyko_ecs/src/ecs/core/entity/entity_master.rs) ✅ | `iter_entities()` — **O(capacity)** scan of `entities_inland`, skips `is_null()` slots (cold/inspection API; Phase X.D removed the dense `active_ids` index) |

`EntityMaster` (Phase 7 + X.D + X.G) is four fields (`#[repr(C)]`, hot cluster
on cache line 0): `entities_inland: InlandStore` (the hot fast store, indexed
by `EntityId.0`, `is_null()` ⇔ dead — since Phase X.G an address-stable
reserve/commit store: lazy 1 GiB reservation, frontier slab commits, growth
copies/writes NOTHING), `next_entity_id: AtomicUsize`, `live_count: usize`,
`free_entity_ids`. The fast-store record is
[`EntityInland`](../crates/boyko_ecs/src/ecs/core/entity/entity_inland.rs)
= 16 B `{ archetype_ptr: *mut Archetype, unit_index: u32, generation: u32 }`
— a **direct slab pointer** (no `SparseMap` indirection on the hot read path);
`NULL` is all-zero bytes (demand-zero pages = free NULL fill, invariant J).
See [SYSTEMS.md §4](SYSTEMS.md) + [PHASE-XD-RESULTS.md](archive/PHASE-XD-RESULTS.md) +
[PHASE-XG-RESULTS.md](archive/PHASE-XG-RESULTS.md).

The `id`/`generation` pair is the ABA defence at the entity layer.
`SparseSlotMap` (boyko_utils) has a parallel slot-layer ABA fix (M-016).

---

## Components

| What you want to do | Where | How |
|---------------------|-------|-----|
| Define a component type | [boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs) ✅ | `#[derive(Component)] struct MyComp { … }` |
| Get the unique ID | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::component_id() -> ComponentId` (lazy, per-type `OnceLock`) |
| Size / align / layout / type id / name | [core/component/component.rs](../crates/boyko_ecs/src/ecs/core/component/component.rs) ✅ | `MyComp::SIZE` / `ALIGN` / `layout()`; trait `mem_size()` / `alignment()` / `type_id()` / `debug_type_name()` |
| Fetch a layout from the registry | [core/component/component_registry/mod.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs) ✅ | `get_layout(id)`, `get_layout_unchecked(id)` |
| Register a layout explicitly (escape hatch) | [core/component/component_registry/mod.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs) ✅ | `register_new::<T>()` (production) / `register_layout::<T>(id)` (test) |
| Build a ComponentMask | [core/component/component_mask.rs](../crates/boyko_ecs/src/ecs/core/component/component_mask.rs) ✅ | `ComponentMask::new()` + `set(id)` |
| Pools for one archetype | [core/component/component_pool_bundle.rs](../crates/boyko_ecs/src/ecs/core/component/component_pool_bundle.rs) ✅ | `ComponentPoolBundle` (two-phase `can_push_*` + `push_*`) |
| ZST components (tags) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | first-class since Phase 22: size-0 components get a tick-only pool (8 B/row, no data region, dangling SIMD-aligned base; `grow_rows_zst`). See [Tags](#tags-static-zst--dynamic-runtime--phase-22). ZST resources/events remain rejected. |

ID assignment (C-003): a per-type `OnceLock<ComponentId>` caches
`register_new::<Self>()` (first call mints from a global `AtomicUsize`, also
registering the `Layout`). **IDs are unstable across processes** — external-ID
consumers must warm up the registry at startup. `MAX_COMPONENTS = 512`
([component_registry/mod.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs):61)
— shared since Phase 22 with dynamic tags.

---

## Tags (static ZST + dynamic runtime) — Phase 22

Zero-data components, two flavors. Storage for BOTH: a tick-only pool —
exactly 8 B/row (the `added`/`changed` tick pair), kept so
`Added<Tag>`/`Changed<Tag>` work (the 0 B/row alternative is a
compile-but-lie). Entities may hold zero components (the EMPTY archetype,
lazy). Public-book pages: `book/src/concepts/tags.md`,
`book/src/concepts/dynamic-tags.md`,
`book/src/architecture/storage-tradeoffs.md`.

| What you want to do | Where | How |
|---------------------|-------|-----|
| Define a static tag | derive ✅ | `#[derive(Component)] struct Player;` — auto-detected via `size == 0` (no attribute); spawnable directly (`commands.spawn(Player)`) via the derive-emitted single-component Bundle |
| Mint a dynamic tag by name | [ecs_master/tag_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/tag_api.rs):47/:65 ✅ | `try_register_tag(name) -> Option<TagId>` (None = 512 budget exhausted) / `register_tag(name)` (panicking sugar); NAME-keyed idempotent, process-global, cold |
| Resolve a name | [tag_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/tag_api.rs):76 ✅ | `tag_by_name(name)` — never mints; the name is the stable key (ids are process-unstable) |
| The handle + id bridge | [component_registry/tags.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs):49 ✅ | `TagId` (**deviation: lives in the registry, NOT the planned `identifiers/tag_id.rs`** — mint-protocol locality + constructor privacy); one-way `component_id()` (:56) / `From<TagId> for ComponentId` (:61) |
| The mint protocol | [component_registry/tags.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs) + [mod.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs) ✅ | `TAG_NAMES` intern ([tags.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs):155) → `try_register_tag_by_name` ([tags.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs):182) → `try_register_dynamic` ([mod.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs):965, bounded CAS, None at ceiling; slot-occupied ⇒ `#[cold]` panic :996 — O2); sentinel `DynamicTagMarker` TypeId (:201); `ComponentLayout::new_dynamic_tag` (:171), `is_zst` (:157) |
| Attach / detach / check (direct) | [tag_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/tag_api.rs):130/:200/:89 ✅ | `add_tag` (present = in-place replace semantics) / `remove_tag` (absent = no-op; last component → EMPTY archetype) / `has_tag` (O(1) signature bit test) |
| Attach / detach (deferred) | [params/entity_commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_commands.rs):182/:196 ✅ | `.add_tag(tag)` / `.remove_tag(tag)` → [tag_commands.rs](../crates/boyko_ecs/src/ecs/core/commands/tag_commands.rs):38/:54 |
| Query by dynamic tag | [query/tag_terms.rs](../crates/boyko_ecs/src/ecs/core/iters/query/tag_terms.rs) ✅ | `with_tag`/`without_tag` on `Query` + `QueryView`; ≤ `MAX_DYN_TAG_TERMS = 8`; archetype-granularity (zero per-row); see [SYSTEMS.md §8.6](SYSTEMS.md) for the `_pre_terms` funnel |
| Hooks on a dynamic tag | [component_registry/mod.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs):885 ✅ | `register_hooks_by_id(tag.component_id(), hooks)` — **mint → register hooks → first attach** (H1: `Err(AlreadyArchetyped)` after) |
| Observers on a dynamic tag | [ecs_master.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/ecs_master.rs) ✅ | the existing `add_observer(kind, tag.component_id(), runner)` — no gate (dynamic bit walk) |
| The dynamic migration paths | [commands/migration_helpers.rs](../crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs):1230/:1305/:1372/:1658/:1922 ✅ | `merged_archetype_id_dyn` / `without_ids_archetype_id` (`kept.is_empty()` → EMPTY — O3) / `migrate_entity_attach_ids` / `migrate_entity_detach_ids` / `retag_in_place` — allocation-free, fire hooks+observers (ledger rows 8–10) |
| Empty entities | [ecs_master/entity_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/entity_api.rs):667, [params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs):184 ✅ | `EcsMaster::spawn_empty` / `Commands::spawn_empty` (via `EmptyBundle`, [self_bundle.rs](../crates/boyko_ecs/src/ecs/core/bundle/self_bundle.rs):135); empty signature matches only zero-required-component queries |
| ZST pool internals | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs), [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs):85/:90 ✅ | tick-only layout, dangling SIMD-aligned base, `grow_rows_zst`; VA: 128 MiB reserve per tag pool per hosting archetype (2 MiB cfg fallback), zero resident until commit — see [SYSTEMS.md §2.3](SYSTEMS.md) |

Ceilings (all loud): 512 shared ComponentIds, `MAX_ARCHETYPES = 1024`
(N tags → up to 2^N hosting archetypes — the fragmentation ceiling; churn
ladder + the EnableTag enable-bit mitigation below), 8 dynamic terms per
query, bundle arity 16. Suites: `tests/phase22_{tags,tags_exhaustion,
query_terms,static_tags,bundles,empty_archetype}.rs`.

---

## EnableTag (enable-bit, non-fragmenting tag backend)

The **second tag storage backend** (the first is the signature/table backend of
[Tags](#tags-static-zst--dynamic-runtime--phase-22)). An EnableTag is NOT part
of any archetype signature and owns no `ComponentPool`; its presence is a single
per-row bit in a paged per-archetype bitset. Toggling is therefore **O(1): no
archetype migration, no structural-generation bump, no hook/observer fire, no
deferred drain** (flecs `CanToggle` semantics) — the right backend for
high-churn transient flags (`Stunned`, `Visible`, `Sleeping`). The trade-off:
no per-row tick storage, so `Added<T>`/`Changed<T>` are compile-rejected on a
bitset tag (the "compile-but-lie" guard). Authoritative design:
[ENABLE-TAG-PLAN.md](archive/ENABLE-TAG-PLAN.md) +
[ENABLE-TAG-PLAN-AMENDMENT-D7.md](archive/ENABLE-TAG-PLAN-AMENDMENT-D7.md). Details +
invariants: [SYSTEMS.md §3.8](SYSTEMS.md).

| What you want to do | Where | How |
|---------------------|-------|-----|
| Define a static enable tag | derive ✅ | `#[component(storage = "bitset")] struct Stunned;` — must be a ZST (a fielded bitset tag is a compile error: no pool to hold data); emits `const STORAGE_IS_BITSET = true` + an `install_storage_kind::<Self>` call; suppresses the single-component Bundle ([boyko_macros/src/component.rs](../crates/boyko_macros/src/component.rs):71~/:84~/:178~/:315~) |
| Mint a dynamic enable tag by name | [ecs_master/enable_tag_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/enable_tag_api.rs):60/:72 ✅ | `register_enable_tag(name) -> EnableTagId` (panicking) / `try_register_enable_tag(name) -> Option<EnableTagId>`; NAME-keyed, classifies the id `StorageKind::Bitset`, cold |
| Toggle (typed) | [enable_tag_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/enable_tag_api.rs):87/:95/:104 ✅ | `enable::<T>(entity)` / `disable::<T>(entity)` / `is_enabled::<T>(entity)` — dead/stale entity = silent no-op |
| Toggle (dynamic) | [enable_tag_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/enable_tag_api.rs):113/:119/:126 ✅ | `enable_id` / `disable_id` / `is_enabled_id` (take `EnableTagId`) |
| Toggle (deferred, in a system) | [params/entity_commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_commands.rs):220/:236/:249 ✅ | `commands.entity(e).enable::<T>()` / `.disable::<T>()` / `.enable_id(tag)` → [enable_tag_commands.rs](../crates/boyko_ecs/src/ecs/core/commands/enable_tag_commands.rs):45 `EnableTagCommand` (POD `{entity, tag, value}`) |
| The id handle + bridge | [component_registry/tags.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs):93 ✅ | `EnableTagId` (`#[repr(transparent)]` over `ComponentId`, proof-of-mint); one-way `component_id()` (:99) / `From<EnableTagId> for ComponentId` (:104) |
| The storage-kind classifier | [component_registry/mod.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs):323 ✅ | `enum StorageKind { Table = 0, Bitset = 1, Dense = 2 }`; cold parallel `STORAGE_KIND: [AtomicU8; 512]` (:373), `storage_kind(id)` (:388), write-once `set_storage_kind` (:433), `install_storage_kind::<C>` (:729), `try_register_enable_tag_by_name` ([tags.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/tags.rs):134) |
| Typed query filter | [query/filter_enable.rs](../crates/boyko_ecs/src/ecs/core/iters/query/filter_enable.rs) ✅ | `Enabled<T>` / `Disabled<T>` — non-archetypal per-row `QueryFilter` (NULL column reads as disabled); rejects `Or<…>` and `for_each_chunk` at the bound |
| Dynamic query terms | [query/enable_terms.rs](../crates/boyko_ecs/src/ecs/core/iters/query/enable_terms.rs) ✅ | `with_enabled(EnableTagId)` / `without_enabled(EnableTagId)` on `Query` ([query.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query.rs):170/:185) AND `QueryView` ([query_view.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs):283/:298); `EnableTerms` (per-view, ≤ `MAX_ENABLE_TERMS = 8`, [constants.rs](../crates/boyko_ecs/src/ecs/constants.rs):397); per-row runtime gate (loop-invariant `is_empty()`, bench-flat 0%-gate) |
| The cull oracle | [component/enable/enable_presence.rs](../crates/boyko_ecs/src/ecs/core/component/enable/enable_presence.rs) ✅ | `EnablePresence` — per-world per-tag archetype bitset; O(1) `contains` + lock-free `epoch`; the bounded candidate snapshot for the D7 global scan (`snapshot_present`) |
| The bit storage | [component/enable/enable_store.rs](../crates/boyko_ecs/src/ecs/core/component/enable/enable_store.rs) ✅ | `EnableStore` (per-archetype, inline-4 `SmallList4`) → `EnableColumn` (lazily-paged) → `EnablePage` (512 B = `[AtomicU64; 64]`, 4096 rows); read-first `swap_remove_bit` |

`EnableTagId` shares the 512-slot `ComponentId` budget with typed components and
both dynamic-tag flavors (exhaustion = loud panic / `None`). Toggle is a
structural-class `&mut EcsMaster` op (v1 `Relaxed` atomics are sound under
`&mut`-exclusivity; the `AtomicU64` words are the forward seam for the D7
worker-marking `&self` toggle). Suites: `tests/loom_term_list.rs`,
`tests/miri_phase22_1.rs`, the per-file `#[cfg(test)]` units, and the EnableTag
plan's named test ranges.

---

## Bundles (typed multi-component spawn payloads)

**Files:** [core/bundle/](../crates/boyko_ecs/src/ecs/core/bundle/) —
`bundle.rs`, `bundle_type_registry.rs`, `bundle_column_cache.rs`.

| What you want to do | Where | How |
|---------------------|-------|-----|
| Define a bundle | [boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs) ✅ | `#[derive(Bundle)] struct SpawnBundle { pos: Position, vel: Velocity }` |
| The bundle trait | [bundle/bundle.rs](../crates/boyko_ecs/src/ecs/core/bundle/bundle.rs):417 ✅ | `trait Bundle: BundleSealed + Send + Sync + Unpin + 'static` — `component_ids()`, `for_each_component_bytes(FnMut)` |
| Per-bundle-type ID | [bundle/bundle_type_registry.rs](../crates/boyko_ecs/src/ecs/core/bundle/bundle_type_registry.rs) ✅ | `BundleTypeId` (73, lazy mint) / `MAX_BUNDLE_TYPES` (84) = 1024 |
| Cached `(BundleType → ArchetypeId, columns)` | [bundle/bundle_column_cache.rs](../crates/boyko_ecs/src/ecs/core/bundle/bundle_column_cache.rs) ✅ | `BundleColumnCache` (Phase 8.5/12.5; sub-ns warm lookups, lazy via `OnceLock`) |

Tuple bundles were intentionally dropped (Phase 8.5) — named `#[derive(Bundle)]`
structs only, so the column cache has a stable per-type address. See
[PHASE-8.5-STATIC-BUNDLE-CACHE-PLAN.md](archive/PHASE-8.5-STATIC-BUNDLE-CACHE-PLAN.md).

Phase 22 additions: `MAX_BUNDLE_ARITY` raised **8 → 16**
([migration_helpers.rs](../crates/boyko_ecs/src/ecs/core/commands/migration_helpers.rs):58
+ the derive, lock-step); every `#[derive(Component)]` type is now ALSO a
single-component bundle (opt-out `#[component(no_bundle)]`; deriving both
`Component` and `Bundle` is a duplicate-impl error without it); the in-crate
mirror `impl_self_bundle!` + the zero-component `EmptyBundle`
([bundle/self_bundle.rs](../crates/boyko_ecs/src/ecs/core/bundle/self_bundle.rs):135,
backs `Commands::spawn_empty`, zero unsafe) replace the Phase-19
`ChildOfBundle`/`ChildrenBundle` newtypes (deleted).

---

## Typed Query DSL (`Query<D, F>`)

The Bevy-shape typed query (Phase 8b). `Query<'w, 's, D, F>` is a `SystemParam`;
`D: QueryData`, `F: QueryFilter`. Drives iteration through the Phase-7 inline
column table (`archetype.columns[c].ptr.add(row * stride)`).

**Files:** [core/iters/query/](../crates/boyko_ecs/src/ecs/core/iters/query/) —
see `mod.rs` for the re-export surface.

| What you want | Where | Type / method |
|---------------|-------|---------------|
| The query SystemParam | [query/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query.rs):62 ✅ | `Query<'w, 's, D, F = ()>` |
| Per-row iteration | [query/iter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/iter.rs) ✅ | `for x in &q` / `for x in &mut q`; `QueryIter` (95) / `QueryIterMut` (410) |
| Data leaves | [query/data.rs](../crates/boyko_ecs/src/ecs/core/iters/query/data.rs) ✅ | `&T`, `&mut T`, `Ref<T>` ([data/ref_.rs](../crates/boyko_ecs/src/ecs/core/iters/query/data/ref_.rs):25), `Mut<T>` ([data/mut_.rs](../crates/boyko_ecs/src/ecs/core/iters/query/data/mut_.rs):30), tuples 1..=12 |
| Read-only marker | [query/data.rs](../crates/boyko_ecs/src/ecs/core/iters/query/data.rs):428 ✅ | `ReadOnlyQueryData` (gates `&q` IntoIterator) |
| Filters | [query/filter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/filter.rs) ✅ | `With<C>` (513), `Without<C>` (693), `Added<C>` (863), `Changed<C>` (1253), `Or<F>` (1535), tuples |
| Direct-API query (no SystemParam) | [query/query_view.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs):83 ✅ | `QueryView<'w, D, F>` via `EcsMaster::query::<D, F>()` |
| Per-`(D,F)` archetype-match cache | [query/state.rs](../crates/boyko_ecs/src/ecs/core/iters/query/state.rs):47 ✅ | `QueryDataState<D, F>` (wraps the Phase-5c `QueryState`) |
| Per-`(D,F)` type interning | [query/query_type_registry.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query_type_registry.rs) ✅ | `QueryTypeId` / `QueryTypeKey`; `MAX_QUERY_TYPES = 1024` (4096 with `big_query_table`) |
| Dynamic-tag term carrier (Phase 22) | [query/tag_terms.rs](../crates/boyko_ecs/src/ecs/core/iters/query/tag_terms.rs) ✅ | `TagTerms` (51, stack-only, per-view) / `archetype_passes_tag_terms` (150) / `MAX_DYN_TAG_TERMS` (42, hard cap 8, loud panic past it) |
| Apply tag terms to a `Query` | [query/query.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query.rs) ✅ | `with_tag(TagId)` (138) / `without_tag(TagId)` (148) |
| Apply tag terms to a `QueryView` | [query/query_view.rs](../crates/boyko_ecs/src/ecs/core/iters/query/query_view.rs) ✅ | `with_tag(TagId)` (249) / `without_tag(TagId)` (259) — every driver funnels the terms through the shared _pre_terms entry point (§8.6) |

The archetype-yielding low-level seam under the typed DSL is
[`QueryState`](../crates/boyko_ecs/src/ecs/core/iters/query_state.rs) —
`QueryState::with_component_ids(&[ComponentId])` + `QueryStateIter`, the
id-driven form the typed cache wraps. The former `LegacyQuery` back-compat
wrapper (`iter_one`/`iter_two`) is **gone**; all code uses `Query<D, F>` or
`QueryState` directly.

### Chunked / parallel iteration

| What | Where | Method |
|------|-------|--------|
| Sequential per-archetype columnar slice | [query/chunk_iter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/chunk_iter.rs) ✅ | `Query::for_each_chunk(\|slice\| …)` (also on `QueryView`) — flecs-style batched API (Phase X.A) |
| Parallel per-archetype-subrange | [query/par_chunk.rs](../crates/boyko_ecs/src/ecs/core/iters/query/par_chunk.rs) ✅ | `Query::par_for_each_chunk(\|slice\| …, BatchingStrategy)` |
| Parallel per-row | [query/par_iter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/par_iter.rs) ✅ | `Query::par_iter()` / `par_iter_mut()` → `ParQuery` (138) / `ParQueryMut` (206); `MIN_ARCHETYPE_FOR_PARALLEL` (73) = 1024 rows, Phase 9 |
| Chunked-data bound | [query/chunked_data.rs](../crates/boyko_ecs/src/ecs/core/iters/query/chunked_data.rs):72 ✅ | `ChunkedQueryData` (`&T`/`&mut T`/`()` + tuples) — `Changed`/`Added`/`Ref`/`Mut` deliberately excluded |
| Archetypal-filter bound | [query/filter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/filter.rs):2512 ✅ | `ArchetypalQueryFilter` (`With`/`Without`/`Or`/tuples) |

`for_each_chunk` lands a credible multi-component SIMD win (boyko 1.28–1.34×
Bevy, native-SIMD) — see [PHASE-X.A-RESULTS.md](archive/PHASE-X.A-RESULTS.md).

---

## SystemParam + Resources + IntoSystem

The ergonomic system machinery (Phases 8a/8c).

**Files:** [core/system/](../crates/boyko_ecs/src/ecs/core/system/).

| What you want | Where | Type / method |
|---------------|-------|---------------|
| The system trait | [system/system.rs](../crates/boyko_ecs/src/ecs/core/system/system.rs):? ✅ | `trait System { type Out; fn name; fn access; unsafe fn run_unsafe(UnsafeEcsCell); fn apply; fn set_change_ticks; }` (`Out` defaults to `()` via `SystemBox`) |
| Function → system | [system/into_system.rs](../crates/boyko_ecs/src/ecs/core/system/into_system.rs):47 ✅ | `trait IntoSystem<In, Out, Marker>`; `FunctionSystem` + markers `IsFunctionSystem` (67) / `ExclusiveSystemMarker` (154) |
| The SystemParam trait | [system/system_param.rs](../crates/boyko_ecs/src/ecs/core/system/system_param.rs) ✅ | `unsafe trait SystemParam` (GAT-based, two-phase `init_state` + `init_access`); tuples 0..=12 ([params/tuple_impl.rs](../crates/boyko_ecs/src/ecs/core/system/params/tuple_impl.rs)) |
| Read a resource | [system/params/res.rs](../crates/boyko_ecs/src/ecs/core/system/params/res.rs):40 ✅ | `Res<'w, R>` |
| Mutate a resource | [system/params/resmut.rs](../crates/boyko_ecs/src/ecs/core/system/params/resmut.rs):42 ✅ | `ResMut<'w, R>` |
| Per-system local state | [system/params/local.rs](../crates/boyko_ecs/src/ecs/core/system/params/local.rs):62 ✅ | `Local<'s, T>` (Phase 13) |
| Conflict / access surface | [system/system_meta.rs](../crates/boyko_ecs/src/ecs/core/system/system_meta.rs), [system/filtered_access_set.rs](../crates/boyko_ecs/src/ecs/core/system/filtered_access_set.rs) ✅ | `SystemMeta` (carries `last_run`/`this_run` ticks), `Access`, `FilteredAccessSet` |
| The worker-side world cell | [system/unsafe_ecs_cell.rs](../crates/boyko_ecs/src/ecs/core/system/unsafe_ecs_cell.rs) ✅ | `UnsafeEcsCell<'w>` (Copy, by-value receivers — Phase 8a C1) |

### Resources storage

| What | Where | Method |
|------|-------|--------|
| The slab | [core/resources/resources.rs](../crates/boyko_ecs/src/ecs/core/resources/resources.rs):100 ✅ | `Resources` — `insert::<R>` (154), `remove::<R>` (252), `contains::<R>` (370); clear-bit-first protocol (Phase 8a C3) |
| The trait | [core/resources/resource.rs](../crates/boyko_ecs/src/ecs/core/resources/resource.rs) ✅ | `trait Resource: Send + Sync + 'static` |
| The registry | [core/resources/resource_registry.rs](../crates/boyko_ecs/src/ecs/core/resources/resource_registry.rs) ✅ | lazy ids; `RESOURCE_SLOT_COUNT = 256` (51) |

---

## Commands + EntityCommands (deferred mutation)

Per-system byte-arena queue flushed via `SystemParam::apply` after the body
returns. No `Box<dyn Command>`, no per-command alloc (Phases 8d/11).

**Files:** [core/system/params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs),
[core/commands/](../crates/boyko_ecs/src/ecs/core/commands/).

| What you want | Where | Method (line) |
|---------------|-------|---------------|
| The SystemParam | [params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs):97 ✅ | `Commands<'s>` |
| Spawn (chainable) | [params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs):164 ✅ | `commands.spawn(bundle) -> EntityCommands` → `.insert(extra).id()` |
| Despawn | [params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs):251 ✅ | `commands.despawn(entity)` |
| Address an existing entity | [params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs):228 ✅ | `commands.entity(entity) -> EntityCommands` |
| Spawn many | [params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs):313 ✅ | `commands.spawn_batch(iter)` |
| Spawn empty (Phase 22) | [params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs):184 ✅ | `commands.spawn_empty() -> EntityCommands` (= `spawn(EmptyBundle)`, warm path hits the static bundle cache) |
| Add / remove a dynamic tag (Phase 22) | [params/entity_commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_commands.rs):182/:196 ✅ | `.add_tag(TagId)` / `.remove_tag(TagId)` → `AddTagCommand`/`RemoveTagCommand` ([commands/tag_commands.rs](../crates/boyko_ecs/src/ecs/core/commands/tag_commands.rs):38/:54, POD id payload) |
| Custom command | [params/commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/commands.rs):125 ✅ | `commands.add::<C: Command>(cmd)` |
| The chainable handle | [params/entity_commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_commands.rs):80 ✅ | `EntityCommands<'a, 's>` — `.insert(..)`, `.remove::<C>()`, `.despawn()`, `.id()` |
| The queue + cmd structs | [commands/](../crates/boyko_ecs/src/ecs/core/commands/) ✅ | `CommandQueue` (CursorSync RAII panic-recovery), `SpawnAtCommand` / `InsertCommand` / `RemoveCommand` / `DespawnCommand` / `SpawnBatchCommand` / `SendEventCommand`; entity-id reservation via `EntityCounter` ([params/entity_counter.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_counter.rs):75) |

---

## Schedule + parallel scheduler

Bevy-class multi-system executor (Phase 9) on the custom
[`boyko_threadpool`](../crates/boyko_threadpool/) (Chase-Lev work-stealing +
`Scope` fork/join). Conflict graph + Tarjan SCC + Kahn topo + apply-window
barrier.

**Files:** [core/schedule/](../crates/boyko_ecs/src/ecs/core/schedule/).

| What you want | Where | Method (line) |
|---------------|-------|---------------|
| Build a schedule | [schedule/schedule_builder.rs](../crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs) ✅ | `ScheduleBuilder::new(Arc<ThreadPool>)` (150); `add_system(system) -> SystemConfig` (177); `build(&mut world) -> Schedule` (324) / `try_build(...)` (350, diagnostics) |
| Run a frame | [schedule/schedule.rs](../crates/boyko_ecs/src/ecs/core/schedule/schedule.rs):? ✅ | `Schedule::run(&mut world)` — bumps tick, runs state pass, dispatches |
| Conflict bitsets + DAG | [schedule/conflict_graph.rs](../crates/boyko_ecs/src/ecs/core/schedule/conflict_graph.rs) ✅ | `ConflictGraph` |
| Per-frame scratch | [schedule/executor_scratch.rs](../crates/boyko_ecs/src/ecs/core/schedule/executor_scratch.rs) ✅ | `ExecutorScratch` (`pred_remaining`, `running`, `completed`, out-of-line completion channel — Phase 9.3c) |
| Erased system slot | [schedule/system_box.rs](../crates/boyko_ecs/src/ecs/core/schedule/system_box.rs) ✅ | `SystemBox` (1-cache-line `Out=()` hot slot) + `BoolSystem` (conditions) |
| The thread pool | [boyko_threadpool/](../crates/boyko_threadpool/src/lib.rs) ✅ | `ThreadPool` / `ThreadPoolBuilder` / `Scope` — `install` (dispatcher) vs `scope` (worker-safe, used by `par_iter`) |

**Soundness:** the executor is proven sound and Tree-Borrows-clean (Phase
9.1/9.2/9.3 — loom + Miri). Structural allocation (frontier commits, container
growth) is restricted to the dispatcher + `ScheduleBuilder::build` (ALLOC1 TLS
discipline).
See [PHASE-9.2-RESULTS.md](archive/PHASE-9.2-RESULTS.md), [PHASE-9.3c-RESULTS.md](archive/PHASE-9.3c-RESULTS.md).

### System ordering & sets

| What | Where | Method |
|------|-------|--------|
| Order one system | [schedule/system_config.rs](../crates/boyko_ecs/src/ecs/core/schedule/system_config.rs) ✅ | `.before(set)` / `.after(set)` / `.in_set(set)` (value-based) |
| Configure a set | [schedule/schedule_builder.rs](../crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs):217 ✅ | `configure_set(set) -> ConfigureSet` (`.before`/`.after`/`.in_set` + hierarchy) |
| Set ids / derive | [schedule/system_set.rs](../crates/boyko_ecs/src/ecs/core/schedule/system_set.rs) ✅ | `SystemSetId` (interned from `(TypeId, discriminant)`); `#[derive(SystemSet)]` on fieldless enums |
| Build diagnostics | [schedule/schedule_builder.rs](../crates/boyko_ecs/src/ecs/core/schedule/schedule_builder.rs):350 ✅ | `try_build()` → `ScheduleBuildError` (`OrderingCycle` B9001, `SetHierarchyCycle` B9002, …) |
| Topo / Tarjan plumbing | [schedule/ordering.rs](../crates/boyko_ecs/src/ecs/core/schedule/ordering.rs) ✅ | `OrderingEdge` / `SystemKey` (Phase 9 scaffold completed in Phase 15) |

See [PHASE-15-RESULTS.md](archive/PHASE-15-RESULTS.md).

### Run conditions (`.run_if`)

| What | Where | Method |
|------|-------|--------|
| Condition on a system / set | [schedule/system_config.rs](../crates/boyko_ecs/src/ecs/core/schedule/system_config.rs) ✅ | `.run_if(cond)` where `cond: impl IntoSystem<(), bool, M>` |
| Built-in conditions | [schedule/common_conditions.rs](../crates/boyko_ecs/src/ecs/core/schedule/common_conditions.rs) ✅ | `run_once`, `in_state`, `on_enter`, `on_exit`, `on_transition` |
| Executor integration | [schedule/schedule.rs](../crates/boyko_ecs/src/ecs/core/schedule/schedule.rs) ✅ | `evaluate_ready_conditions` pass at the apply-window barrier (0%-gate via `has_condition` bitset) |

`run_if` conditions are pure predicates (no `apply`). Eager AND fold (no
short-circuit). Tick-aware conditions (`Changed`/`Added`/`Ref`) work correctly
since Phase 16.1 ✅: a condition's window advances only on a frame it is
evaluated, and a gated system's ticks advance only on a frame it runs, so
dormant changes are never silently missed (Bevy "since-last-actual-run"
parity). See [PHASE-16-RESULTS.md](archive/PHASE-16-RESULTS.md) +
[PHASE-16.1-RESULTS.md](archive/PHASE-16.1-RESULTS.md).

---

## States

Application/game states layered on the single `Schedule` (Phase 17).

**Files:** [core/state/](../crates/boyko_ecs/src/ecs/core/state/).

| What | Where | How |
|------|-------|-----|
| The marker trait | [state/states.rs](../crates/boyko_ecs/src/ecs/core/state/states.rs) ✅ | `trait States: Send + Sync + Clone + PartialEq + Eq + Hash + 'static` (hand-impl, no derive) |
| Current / queued value | [state/state.rs](../crates/boyko_ecs/src/ecs/core/state/state.rs), [state/next_state.rs](../crates/boyko_ecs/src/ecs/core/state/next_state.rs) ✅ | `State<S>` (current), `NextState<S>` (`Unchanged`/`Pending(S)`) |
| Run conditions | [schedule/common_conditions.rs](../crates/boyko_ecs/src/ecs/core/schedule/common_conditions.rs) ✅ | `in_state(s)` / `on_enter(s)` / `on_exit(s)` / `on_transition(a, b)` |
| Transition pass | [state/transition_record.rs](../crates/boyko_ecs/src/ecs/core/state/transition_record.rs) ✅ | `StateTransitionRecord<S>` + `apply_state_transition::<S>`; runs once per `Schedule::run` (0%-gate via `state_entries.is_empty()`) |
| Generic-resource id trap fix | [resources/resource_type_registry.rs](../crates/boyko_ecs/src/ecs/core/resources/resource_type_registry.rs) ✅ | `TypeId`-keyed registry `resource_id_for::<T>()` (avoids the rust#22991 `State<S>`-aliases-one-slot trap); shared with `boyko_input`'s `ActionState<A>`/`InputMap<A>` |
| Builder / world entry | builder `init_state`/`insert_state`; `EcsMaster::{insert_state, init_state, state, set_next_state}` ✅ | see [App](#app--plugin-facade) + [EcsMaster](#high-level-facade-ecsmaster) |

See [PHASE-17-RESULTS.md](archive/PHASE-17-RESULTS.md).

---

## Component lifecycle hooks & observers

Two reactive-callback mechanisms firing at the four structural-op kinds —
**add / insert / replace / remove**. A despawn fires `replace` + `remove` per
dying component (no separate despawn kind). Both gate on the per-archetype
`ArchetypeFlags` `u16` bit-test → a world with no callback pays one `test`/`jz`
and zero allocation ("0% when unused").

| What you want to do | Where | How |
|---------------------|-------|-----|
| **Hooks** — ONE write-once callback per component *type* | [core/component/hooks/](../crates/boyko_ecs/src/ecs/core/component/hooks/) ✅ | `#[component(on_add = path, …)]` derive XOR runtime `EcsMaster::register_component_hooks::<C>()` (Phase 14a — [PHASE-14-RESULTS.md](archive/PHASE-14-RESULTS.md)) |
| **Observers** — `add`/`remove`-able LIST per `(kind, component)` | [core/component/observers/mod.rs](../crates/boyko_ecs/src/ecs/core/component/observers/mod.rs):189 ✅ | `EcsMaster::observe_on_{add,insert,replace,remove}::<C>(runner)` (Phase 14b) |
| Register an observer by `ComponentId` | [ecs_master/observer_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/observer_api.rs):182 ✅ | `add_observer(kind, cid, runner) -> ObserverId` |
| Remove an observer | [ecs_master/observer_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/observer_api.rs):199 ✅ | `remove_observer(id) -> bool` (recomputes archetype bits on last-of-kind removal) |
| The observer runner / context | [core/component/observers/mod.rs](../crates/boyko_ecs/src/ecs/core/component/observers/mod.rs):98 ✅ | `ObserverFn = unsafe fn(DeferredEcsMaster<'_>, ObserverContext)`; mutate only via the view's deferred `commands()` |
| The 4 cold dispatch fns | [core/component/observers/dispatch.rs](../crates/boyko_ecs/src/ecs/core/component/observers/dispatch.rs):115~ ✅ | `fire_on_{add,insert,replace,remove}_observers` (`#[cold] #[inline(never)]`, wired at **10** fire sites — Phase 22 added the 3 tag-migration sites; full ledger in [SYSTEMS.md §3.6](SYSTEMS.md)) |
| Register hooks by id (no Rust type — dynamic tags) | [core/component/component_registry/mod.rs](../crates/boyko_ecs/src/ecs/core/component/component_registry/mod.rs):885 ✅ | `register_hooks_by_id(ComponentId, ComponentHooks) -> Result<(), HooksError>` — H1 gate: `Err(AlreadyArchetyped)` after the id's first attach; **contract: mint → register hooks → first attach** (Phase 22 D8) |

A **hook** is a single fn-ptr in the process-global `HOOKS` table (staleness-
panics if an archetype with `C` already exists); an **observer** is one of a
runtime-mutable per-world list (no staleness panic). At each fire site hooks run
first, then observers. Full catalog: [SYSTEMS.md §3.6](SYSTEMS.md).

---

## Hierarchies (parent-child)

Bevy-0.16 relationship model on the hooks substrate (Phase 19). `ChildOf` (FK on
the child, source of truth) + `Children` (reverse collection on the parent), kept
consistent by component hooks; default-recursive despawn cascade.

**Files:** [core/hierarchy/](../crates/boyko_ecs/src/ecs/core/hierarchy/) —
`mod.rs` (components + hand-impl `Component` + hook registration), `commands.rs`
(Link/Unlink/Clear deferred commands + the `ChildOf`/`Children` hook bodies),
`bundles.rs` (1-field `Bundle` newtypes routing the first-child insert through the
audited `migrate_entity_insert`).

| What you want to do | Where | How |
|---------------------|-------|-----|
| Parent component (FK on child) | [hierarchy/mod.rs](../crates/boyko_ecs/src/ecs/core/hierarchy/mod.rs) ✅ | `ChildOf(pub Entity)` — insert links, overwrite reparents, remove unlinks |
| Children collection (read-only) | [hierarchy/mod.rs](../crates/boyko_ecs/src/ecs/core/hierarchy/mod.rs) ✅ | `Children` — `as_slice()` / `len()` / `is_empty()` / `contains()`; maintained reactively, never written by user code |
| Add a child / children | [params/entity_commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_commands.rs) ✅ | `commands.entity(parent).add_child(c)` / `.add_children(&[..])`; `Commands::add_child(p, c)` |
| Set / clear parent | [params/entity_commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_commands.rs) ✅ | `.set_parent(p)` / `.remove_parent()` |
| Remove specific / all children | [params/entity_commands.rs](../crates/boyko_ecs/src/ecs/core/system/params/entity_commands.rs) ✅ | `.remove_children(&[..])` (listed only) / `.clear_children()` (all) |
| Despawn keeping children | [ecs_master/entity_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/entity_api.rs):821 ✅ | `despawn_without_children(e)` — opt out of the default recursive cascade |
| Recursive despawn (default) | [ecs_master/entity_api.rs](../crates/boyko_ecs/src/ecs/core/ecs_master/entity_api.rs):798 ✅ | `delete_entity(e)` / `commands.despawn(e)` cascades to all descendants |

`Children` consistency is at the deferred-hook-queue drain (same-frame apply
window). Guards: self-ref + dangling-parent are reactively rejected (the bad
`ChildOf` is removed, the collection untouched); deep `ChildOf` cycles are a
documented footgun (only self-ref is checked). Sibling order is unspecified
(`swap_remove`); an emptied `Children` is retained (no archetype thrash). The
net new `unsafe` for the whole feature is **one** (the `MaybeUninit` cascade
buffer). DEFERRED: transform propagation, parallel tree walk, `iter_descendants`,
a generic `Relationship` trait. See [PHASE-19-RESULTS.md](archive/PHASE-19-RESULTS.md).

> The cascade exposed **BUG-P19-TB-1**, a pre-existing latent Tree-Borrows UB in
> the deferred command-queue re-entrant drain (`commands/command_queue.rs`
> `apply_via_raw_twin` cached a `NonNull<Vec>` foreign-written by a re-entrant
> `push`). Fixed by walking a stack-local `mem::take`'d copy of the queue (the
> audited `apply` on a disjoint allocation). See
> [BUG-P19-TB-1-PLAN.md](archive/BUG-P19-TB-1-PLAN.md).

---

## Change detection (Tick / `Added<T>` / `Changed<T>`)

Bevy-style per-row tick storage (Phase 10).

**Files:** [core/change_detection/](../crates/boyko_ecs/src/ecs/core/change_detection/).

| What you want | Where | How |
|---------------|-------|-----|
| The tick type | [change_detection/tick.rs](../crates/boyko_ecs/src/ecs/core/change_detection/tick.rs) ✅ | `Tick(u32)` — `is_newer_than`; `MAX_CHANGE_AGE` / `CHECK_TICK_THRESHOLD` |
| Per-row tick storage | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs):147 ✅ | `ComponentPool::{added_ticks, changed_ticks}: Box<[UnsafeCell<Tick>]>` |
| Filter on added/changed | [query/filter.rs](../crates/boyko_ecs/src/ecs/core/iters/query/filter.rs) ✅ | `Added<C>` (863) / `Changed<C>` (1253) |
| Read with change info | [query/data/ref_.rs](../crates/boyko_ecs/src/ecs/core/iters/query/data/ref_.rs) ✅ | `Ref<T>` (25, immutable + flags) / `Mut<T>` ([data/mut_.rs](../crates/boyko_ecs/src/ecs/core/iters/query/data/mut_.rs):30, deref-guard bumps the tick) |
| Frame bump + wraparound scan | [change_detection/check_ticks.rs](../crates/boyko_ecs/src/ecs/core/change_detection/check_ticks.rs) ✅ | `run_check_ticks_scan`; `EcsMaster::change_tick: AtomicU32` bumped per `Schedule::run` |

0% measurable overhead on queries that use no change detection. See
[PHASE-10-CHANGE-DETECTION-PLAN.md](archive/PHASE-10-CHANGE-DETECTION-PLAN.md).

---

## Events

A full double-buffered event dispatcher (Phase 6) plus the `EventReader` /
`EventWriter` SystemParam wrappers (Phase 12). **Note:** earlier revisions of
these docs said "no dispatcher" — that is now stale; the dispatcher exists.

**Files:** [core/events/](../crates/boyko_ecs/src/ecs/core/events/),
[core/system/params/event_reader.rs](../crates/boyko_ecs/src/ecs/core/system/params/event_reader.rs),
[core/system/params/event_writer.rs](../crates/boyko_ecs/src/ecs/core/system/params/event_writer.rs).

| What you want | Where | How |
|---------------|-------|-----|
| Define an event type | [boyko_macros/src/lib.rs](../crates/boyko_macros/src/lib.rs) ✅ | `#[event] struct DamageEvent { #[participant(...)] victim: Entity, #[parameter] amount: f32 }` |
| Read events in a system | [params/event_reader.rs](../crates/boyko_ecs/src/ecs/core/system/params/event_reader.rs):87 ✅ | `EventReader<'s, E>` → `EventIter` (245) (cursor checkpointed on partial iter) |
| Write events in a system | [params/event_writer.rs](../crates/boyko_ecs/src/ecs/core/system/params/event_writer.rs):89 ✅ | `EventWriter<'s, E>` (per-lane TLS routing; parallel writers OK) |
| The dispatcher | [events/event_dispatcher.rs](../crates/boyko_ecs/src/ecs/core/events/event_dispatcher.rs) ✅ | `EventDispatcher` — `send_event::<E>` (274), `send::<E>(thread_index, ..)` (292), `update_events()` (436, frame swap) |
| The double-buffer | [events/event_buffer.rs](../crates/boyko_ecs/src/ecs/core/events/event_buffer.rs) ✅ | `EventBuffer<E>` — split cache-line lanes (Phase 12 false-sharing fix) |
| Config / capacity | [events/event_config.rs](../crates/boyko_ecs/src/ecs/core/events/event_config.rs) ✅ | `EventConfig`; `MAX_EVENT_THREADS = 64`, `MAX_EVENT_CAPACITY = 16384` ([constants.rs](../crates/boyko_ecs/src/ecs/constants.rs)) |
| Registry / metadata | [events/event_registry.rs](../crates/boyko_ecs/src/ecs/core/events/event_registry.rs) ✅ | `MAX_EVENTS` (51) = 256; `register_event` (159) / `register_event_new` (109) mint the lazy per-type id |
| Participants / parameters | [events/participants/](../crates/boyko_ecs/src/ecs/core/events/participants/), [events/parameters/](../crates/boyko_ecs/src/ecs/core/events/parameters/) ✅ | `Participants` / `Parameters` traits + TypeId-guarded buffers (Q-019) |

Events sit OUTSIDE the conflict graph (Option A) — parallel writers of the same
`E` are OK via per-lane TLS routing. See [PHASE-12-RESULTS via memory] and
[PHASE-6-EVENT-DISPATCH-PLAN.md](archive/PHASE-6-EVENT-DISPATCH-PLAN.md).

---

## Resources

See [SystemParam + Resources](#systemparam--resources--intosystem) above for the
storage + `Res`/`ResMut` params, and [EcsMaster](#high-level-facade-ecsmaster)
for `insert_resource` / `resource`.

---

## Archetypes (lower-level discovery)

| What | Where | Method |
|------|-------|--------|
| The archetype | [core/archetype/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs):127 ✅ | `Archetype` — inline `columns: [Column; 512]` at offset 0 (Phase 7 fast read path), `entity_ids`, `flags`, `signature` |
| Hot column entry | [core/archetype/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs):32 ✅ | `Column { ptr: *mut u8, stride: u32 }` (16 B; `is_null()` ⇔ absent) |
| Remove outcome | [core/archetype/archetype.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype.rs):89 ✅ | `enum RemoveOutcome { Last, Swapped { moved_entity }, PoolFailure }` (C-006) |
| The manager | [core/archetype/archetype_master.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_master.rs):18 ✅ | `ArchetypeMaster` — owns the `ObserverRegistry` (65~, the field); dual gen counters |
| Slab storage | [core/archetype/archetype_bundle.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_bundle.rs) ✅ | `ArchetypeBundle` (stable-address slab + sparse id map) |
| Signature | [core/archetype/archetype_signature.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_signature.rs) ✅ | `ArchetypeSignature { mask, block_summary, section_summary }` |
| Discovery (registry) | [core/archetype/archetype_registry.rs](../crates/boyko_ecs/src/ecs/core/archetype/archetype_registry.rs) ✅ | `find_archetypes_with_components` / `find_matching_archetypes` / `find_with_filter` (+ `_into(out)` variants) |
| ABA-safe match cache | [core/iters/query_state.rs](../crates/boyko_ecs/src/ecs/core/iters/query_state.rs), [core/iters/archetype_bit_set.rs](../crates/boyko_ecs/src/ecs/core/iters/archetype_bit_set.rs) ✅ | `QueryState` (dual gen counters), `ArchetypeBitSet` (1024-bit dedup) |

The dual-generation design (`generation` for creation deltas,
`structural_generation` for removal/clear) is the load-bearing ArchetypeId-ABA
fix (Phase 5c). `MAX_ARCHETYPES = 1024`.

---

## Type-erased component storage

| What you want to do | Where | Method |
|---------------------|-------|--------|
| Create a pool | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `ComponentPool::new(component_id, reserve_rows)` — explicit row ceiling EXACTLY, clamp-bypass by design (★R1-9; X.J collapsed the legacy `(arena, id, n, m)` shape, `reserve_rows = n × m`); `with_default_sizes(component_id)` = byte-targeted clamp sizing |
| Grow a pool (automatic) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `#[cold] grow_rows` — per-pool `VmReservation [data\|added\|changed]`, slab doubling 64 KiB…64 MiB, ticks lockstep, idempotent, O(1) in live rows, bases never move (Phase X.I). 1M-entity single-archetype ramp **2.24× faster than Bevy**, worst-batch spike **0.022×** ([PHASE-XI-RESULTS.md](archive/PHASE-XI-RESULTS.md)) |
| Committed-rows frontier (diagnostics) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `committed_rows()`; `capacity()` = reserve ceiling |
| Append a component (raw bytes) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `add(&[u8])` |
| Append a component (typed, TypeId-guarded) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `add_typed::<T>(value)` |
| Read a component (typed) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `get_typed::<T>(idx)` / `get_mut_typed::<T>(idx)` (C-004) |
| Read a component (raw) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `get_raw(idx)` / `get_raw_mut(idx)` |
| Overwrite a slot | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `set_component(idx, &[u8])` (runs `drop_fn` on the old value) |
| Remove (swap with last) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `swap_remove(idx)` / `pop()` (run `drop_fn`) |
| Address row `i`'s bytes | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs):? ✅ | private `row_ptr(i)` = `buffer.as_ptr().add(i * stride)` (Phase X.B removed the `Vec<Unit>` cache) |
| Live-row count | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs):161~ ✅ | the `len` field / `count()` |
| Dense base pointer | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `buffer_ptr()` — SIMD-aligned (`SIMD_BUFFER_ALIGN = 32`, Phase X.A) |

Type erasure: the pool stores raw bytes + the `Layout` from the
`ComponentRegistry`. Drop discipline: a cached `drop_fn: Option<DropFn>` runs on
`swap_remove` / `pop` / `set_component` / `Drop` (M-004). **Phase X.B** deleted
the parallel `units: Vec<Unit>` (each entry == `buffer + i*stride`) — rows are
now computed arithmetic, which net-removed `unsafe`. **Phase 10** added the
per-row tick columns; **Phase X.I** moved them into the pool's own reservation
(`[data | added | changed]` sub-regions), made the pool self-growing, and
DELETED the chunk machinery (`memory/chunk.rs` — the dirty flags were
written-never-read; a per-mutation `udiv` died with them). See
[PHASE-XB-RESULTS.md](archive/PHASE-XB-RESULTS.md), [PHASE-XI-RESULTS.md](archive/PHASE-XI-RESULTS.md).

---

## Memory and allocation

| What you want to do | Where | Method |
|---------------------|-------|--------|
| Reserve/commit a VM range | [memory/vm.rs](../crates/boyko_ecs/src/ecs/memory/vm.rs) ✅ | `VmReservation::{reserve, commit, base, os_len}` — the single per-OS primitive under `InlandStore` and every `ComponentPool` (X.G/X.H/X.I) |
| Grow a pool (automatic) | [memory/component_pool.rs](../crates/boyko_ecs/src/ecs/memory/component_pool.rs) ✅ | `#[cold] grow_rows` — slab doubling 64 KiB…64 MiB, ticks in lockstep, bases never move (Phase X.I; see [Type-erased component storage](#type-erased-component-storage)) |
| Grow the entity store (automatic) | [entity/inland_store.rs](../crates/boyko_ecs/src/ecs/core/entity/inland_store.rs) ✅ | `#[cold] grow_to` via `ensure(n)` — 256 KiB…16 MiB slabs, demand-zero = `EntityInland::NULL` (Phase X.G) |
| Align an address/size | [memory/utils.rs](../crates/boyko_ecs/src/ecs/memory/utils.rs) ✅ | `align_up(value, alignment)` |

There is no shared allocator: **the Arena + `MemFreeBlockMaster` were DELETED
in Phase X.J** (client-less since X.I — every pool owns its memory via a
per-pool `VmReservation`). Backing acquisition: reserve-only syscall
(`VirtualAlloc(MEM_RESERVE, PAGE_NOACCESS)` on Windows, `mmap(PROT_NONE)` on
Unix) + lazy geometric slab commits at the frontier (`MEM_COMMIT` /
`mprotect`); Miri / wasm32 / exotic targets eagerly `alloc_zeroed` the full
reserve (commit = no-op). `Drop` uses the per-cfg-arm matching deallocator
(M-001). See [PHASE-XI-RESULTS.md](archive/PHASE-XI-RESULTS.md) +
[PHASE-XJ-RESULTS.md](archive/PHASE-XJ-RESULTS.md).

---

## Identifiers

| What | Where | Type |
|------|-------|------|
| Entity / archetype / component / etc. IDs | [identifiers/primitives.rs](../crates/boyko_ecs/src/ecs/identifiers/primitives.rs) ✅ | `#[repr(transparent)] EntityId(usize)` + siblings (C-017: strongly-typed newtypes, defined via one `define_id!` macro) |
| Generation counter | [identifiers/primitives.rs](../crates/boyko_ecs/src/ecs/identifiers/primitives.rs) ✅ | `Generation = usize` (alias — only paired with `EntityId`) |
| Slot (boyko_utils) | [boyko_utils/identifiers/slot.rs](../crates/boyko_utils/src/identifiers/slot.rs) ✅ | `Slot { index, generation }` |

Newer dense-table sizing newtypes (`ResourceId`, `BundleTypeId`, `QueryTypeId`,
`ObserverId`, `SystemSetId`) live next to their subsystems, not in
`primitives.rs`.

---

## boyko_utils (reusable collections)

| What | Where | Type |
|------|-------|------|
| Dense sparse set (usize keys) | [boyko_utils/sparse_map/sparse_map.rs](../crates/boyko_utils/src/sparse_map/sparse_map.rs) ✅ | `SparseMap<U>` (used by `ArchetypeBundle`/registry; `EntityMaster` moved off it in Phase 7) |
| Generation-tracked slot map | [boyko_utils/sparse_map/sparse_slot_map.rs](../crates/boyko_utils/src/sparse_map/sparse_slot_map.rs) ✅ | `SparseSlotMap<U>` (ABA-fixed via tombstone+gen, M-016) |
| Trait abstraction | [boyko_utils/sparse_map/sparse_collection.rs](../crates/boyko_utils/src/sparse_map/sparse_collection.rs) ✅ | `SparseCollection<K, V>` |
| Bitset (generic word size) | [boyko_utils/bit_mask/bit_set.rs](../crates/boyko_utils/src/bit_mask/bit_set.rs) ✅ | `BitSet<T: BitInteger>` |
| Fixed 256-bit set | [boyko_utils/bit_mask/bit_set_256.rs](../crates/boyko_utils/src/bit_mask/bit_set_256.rs) ✅ | `BitSet256` (+ `pop_lowest_set_bit`) — Phase 6, backs resource/event lane masks |
| Identifier primitives | [boyko_utils/identifiers/](../crates/boyko_utils/src/identifiers/) ✅ | `Generation`, `Slot` |

---

## boyko_demo (dogfooding sandbox)

A wgpu+egui GPU-instanced sandbox exercising the public API (particles / boids /
physics via Phase-17 states, real `Schedule::run` + `par_iter` + zero-AoS-copy
`for_each_chunk` → GPU upload — substep-gated since Phase 20.1 to
min(display, sim) rate via `upload_due`, −55 % upload events at 144/64, with
GPU-side `mix(prev_pos, pos, alpha)` interpolation off the 24 B `GpuInstance`
`{pos, scale, color, prev_pos}`; `sync_gpu_instance` is the single load-bearing
`prev_pos` maintainer, incl. Physics). Compiles for wasm32 too.

**Files:** [crates/boyko_demo/src/](../crates/boyko_demo/src/) — `app.rs`,
`sim/` (systems, grid, modes, runner), `render/`, `ui/`. See
[DEMO-PLAN.md](archive/DEMO-PLAN.md) + [DEMO-DOGFOODING.md](DEMO-DOGFOODING.md).

---

## What is NOT in the engine (deliberately / deferred)

| Missing | Why / where tracked |
|---------|--------------------|
| ~~ZST components~~ | ✅ LANDED — Phase 22 tags (tick-only pools); ZST **resources / events** remain ❌ rejected (compile-time guard) |
| ~~Non-fragmenting tag toggle (enable bits)~~ | ✅ LANDED — the EnableTag enable-bit backend (`#[component(storage = "bitset")]` / `register_enable_tag` + `Enabled`/`Disabled` filters); see [EnableTag](#enabletag-enable-bit-non-fragmenting-tag-backend). v1 toggle is `&mut`; the `&self` worker-marking toggle (D7) is the deferred seam |
| Typed `Added`/`Changed` for dynamic tags | 📋 follow-up (`DynAdded(TagId)` term — ticks already maintained, no storage change needed) |
| `Option<Res<R>>` SystemParam → `resource_exists` condition | 📋 deferred (Phase 16 residual) |
| ~~Tick-aware run conditions (`Changed`/`Added`)~~ | ✅ LANDED — Phase 16.1 (dormancy-correct ticks, [PHASE-16.1-RESULTS.md](archive/PHASE-16.1-RESULTS.md)) |
| `for_each_chunk` with `Changed`/`Added`/`Ref`/`Mut` | ❌ gated out at compile time; use `iter()` — Phase 13.X `ChunkedTickedQueryData` |
| ~~Multi-schedule~~ | ✅ LANDED — Phase 20, as the closed `CoreSchedule { Main, Fixed }` set ([PHASE-20-RESULTS.md](archive/PHASE-20-RESULTS.md)); a user-mintable label map stays ❌ rejected (D5) |
| SubApps / `PluginGroup` / `App::with_world` | 📋 deferred (Phase 18 boundaries; `with_world` filed as Phase 20.1) |
| Single-dep prelude including derives | 📋 deferred — needs the `boyko-macros` cycle refactor (Phase 18) |
| 5× `for_each_chunk` headline on a wide/SIMD-heavy workload | 📋 Phase X.A.2 (credible 1.3× multi-component win already landed) |
| Auto sync-point insertion (coalesced command flush) | 📋 deferred (per-system apply window already a sync point) — Phase 15 residual |
| `Participants`/`Parameters` split revisit (Q-020) | ❌ deferred — no participant-filtered dispatch use case yet |

---

## Tests / benchmarks at a glance

The workspace now carries **3600+ test functions** across the 18 crates (raw
`#[test]` / `#[tokio::test]` sites; the authoritative per-run pass count is the
CI job, since proptest/loom cases and feature-gated GPU tests expand further).
The `boyko-ecs` kernel alone is ~918 passing debug / 903 release
(`cargo test -p boyko-ecs`, Phase 19 baseline) across in-module `#[cfg(test)]`
units + the integration files under `crates/boyko_ecs/tests/`; the render / sim /
UI crates add the balance. Miri (`-Zmiri-tree-borrows`, `-Zmiri-ignore-leaks` for
the spawn-reaching suites) is clean for the change-detection / hooks / observers /
hierarchies / states / executor-soundness suites. For the exact gate per phase,
read the relevant `docs/PHASE-*-RESULTS.md`.

**Benchmarks** (criterion, `harness = false`) live in
[crates/boyko_ecs/benches/](../crates/boyko_ecs/benches/) (see the `[[bench]]`
list in [Cargo.toml](../crates/boyko_ecs/Cargo.toml)) and the cross-engine
comparison in [crates/bench_bevy_vs_boyko/](../crates/bench_bevy_vs_boyko/).
Methodology (deterministic `[profile.bench]` codegen + opt-in `bench-alloc`
mimalloc + the median-of-N `bench.ps1`) is in
[BENCHMARKING.md](BENCHMARKING.md) (Phase X.E).

### Golden byte-identity harness (render regression gate)

GPU render regressions are caught by a **byte-identity** gate: the whole scene is dumped
to a `.bmp` and SHA-256'd, and any logical no-op (god-file split, `embed_spirv!`, a
gated-OFF path) must keep that hash unchanged (the "Tier-0" gate). Where things live:

| Piece | Location | What it is |
|-------|----------|------------|
| The gate command | [scripts/golden.ps1](../scripts/golden.ps1) | `-Check` / `-Bless` — force-compiles the test bin, DELETE-then-regen, asserts fresh mtime, hashes, compares against the pin. Encodes every anti-false-green lesson. |
| The pins | [goldens/PINS.toml](../goldens/PINS.toml) | Single source of truth for the SHA-256 pin(s) + the feature/env each was blessed under. Replaces the hash formerly hand-copied across ~10 docs. Update ONLY via `golden.ps1 -Bless`. |
| CPU host oracles | [crates/boyko_rhi_vulkan/src/goldens.rs](../crates/boyko_rhi_vulkan/src/goldens.rs) | ~4.2 kLOC of `golden_*`/`host_*` functions mirroring shader math bit-for-bit (gated by the `goldens` feature). Diffed against GPU readback within `CHANNEL_TOL` or bit-exact. Incrementally migrating to eDSL-derived references — see [GOLDEN-EDSL-MIGRATION-PLAN.md](GOLDEN-EDSL-MIGRATION-PLAN.md). |
| Shared test helpers | [crates/boyko_rhi_vulkan/tests/common/mod.rs](../crates/boyko_rhi_vulkan/tests/common/mod.rs) | `Vertex`, `SpirvBlob`, `write_words`, and `diff_in_bbox` (bbox-scoped diff — use it alongside a whole-frame diff so a localized effect is never averaged into invisibility). |
| The dump tests | `crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs`, `sdf_gbuffer_hybrid.rs` + [crates/boyko_app/tests/](../crates/boyko_app/tests/) (`vb_*`, `forward*`, `sdf_forward_only`, `taa_jitter_eval`, `grand_showcase_2mat` — **27 of the 30 pins**, counted from `PINS.toml`'s `crate =` rows, one per render-path × legs × feature cell) | `#[ignore]`d windowed presents (`--test-threads=1`, single RTX-3060). Run via `golden.ps1`. |
| The render-path matrix sweep | [scripts/paradigm-matrix.ps1](../scripts/paradigm-matrix.ps1) | Renders ONE scene across every path (`-Full`: all 12 path × legs cells) to BMPs for a side-by-side eyeball, via `BOYKO_HOST_DUMP` + the `BOYKO_RENDER_PATH`/`BOYKO_GEOMETRY_LEGS` env seam. A visual companion to the hash gate, not a substitute for it. |

**Usage:** `scripts\golden.ps1` (software leg) or `-Hwrt` (hwrt leg) to verify; `-Bless`
after a visual owner sign-off to re-pin. Never run on CI (no GPU there — goldens skip).

### VG-R0 density census (virtual-geometry measurement rung) ✅ COMPLETE

A **measurement** instrument, not a render feature: it reads the VisibilityBuffer path's `vb_id`
image back to the host and reports screen-space triangle density over a real high-poly corpus. It
exists to adjudicate one pre-registered kill — *is real content actually in the micro-polygon regime
a cluster-LOD system would serve?* — before any meshlet code is written. Verdict on the shipped
corpus: **UNDECIDED, escalate** (`min D_est = 0.509 < 1.0`), so the owner's pre-registered
disposition routes to building a non-saturating **upper-bound** instrument, which R0 records as an
unsolved design problem rather than a scheduling one.

| Piece | Location | What it is |
|-------|----------|------------|
| The spec | [MESHLET-VIRTUAL-GEOMETRY-PLAN.md](MESHLET-VIRTUAL-GEOMETRY-PLAN.md) | Rung R0 only. 37 revisions; §9.1 enumerates what R0 deliberately does NOT decide. |
| Frozen thresholds | [VG-CAMPAIGN-THRESHOLDS.toml](VG-CAMPAIGN-THRESHOLDS.toml) | Author-set, decision-bearing, sha256-pinned, **never edited** — amendments go through the plan's §11.1. |
| Owner VALUES calls | [VG-CAMPAIGN-CLAIM.toml](VG-CAMPAIGN-CLAIM.toml) | Unhashed, `PENDING`-sentinel gated; deliberately split from the frozen half by *update discipline*, not by subject. |
| The result | [VG-R0-DENSITY-CENSUS.md](VG-R0-DENSITY-CENSUS.md) | **Machine-written** by the run that measured it — rows, `D_est`, cross-process digests, and the two measured-not-asserted residuals. |
| Why K1 cannot be FIRED | [VG-R11-UPPER-BOUND-INSTRUMENT.md](VG-R11-UPPER-BOUND-INSTRUMENT.md) | The upper-bound instrument, still UNSOLVED — with seven adjudicated candidate deaths and four structural results, including the theorem that on this corpus **no sound instrument can fire K1**. Read it before proposing an eighth candidate. |
| Host reducer | [crates/boyko_render/src/vg_census.rs](../crates/boyko_render/src/vg_census.rs) | Turns one `vb_id` readback into a census row. Distinct triangles by sorting packed `(instance, primitive)` keys and counting runs — `HashMap` is banned and a run's *length* is that triangle's pixel count, so the histogram falls out of the same pass. Also carries the workspace's streaming SHA-256. |
| Armed readback | [crates/boyko_app/src/vg_census_dump.rs](../crates/boyko_app/src/vg_census_dump.rs) | `BOYKO_VG_CENSUS=<path.toml>`. Settle → request → drain, so the readback frame's fence is re-waited before the per-FIF ring is mapped. **Unarmed frames record zero extra commands** — the byte-neutrality all 13 VB pins verify. |
| The one permanent render edit | [crates/boyko_rhi_vulkan/src/present/targets.rs](../crates/boyko_rhi_vulkan/src/present/targets.rs) | `TRANSFER_SRC` on the `vb_id` ring. Provably cannot move a texel: the image is `R32G32_UINT`, uncompressed, `.Load`ed unfiltered. |
| Instrument gate | [crates/boyko_app/tests/vg_density_census.rs](../crates/boyko_app/tests/vg_density_census.rs) + [vg_fixture/](../crates/boyko_app/tests/vg_fixture/) | R0c. A procedural fixture of ISOLATED right triangles offset by a quarter pixel, so the covered count is one exact number and no fill rule can decide it — the GPU and `sv0_oracle` agree **to the pixel**. |
| Census run | [crates/boyko_app/tests/vg_r0d_census.rs](../crates/boyko_app/tests/vg_r0d_census.rs) + [vg_corpus_scene/](../crates/boyko_app/tests/vg_corpus_scene/) | R0d. One worker process per `(camera path, ladder rung)` pair, so each rung negotiates its own window and the achieved extent is a measurement rather than an echo of the request. |
| Shared ladder readers | [crates/boyko_app/tests/vg_thresholds/](../crates/boyko_app/tests/vg_thresholds/) | The frozen-file parsers, the per-rung extent route, the row parser and the worker spawner — one text, because two copies of a ladder are two texts that can disagree. |
| Corpus | [assets/vg_corpus/CORPUS.toml](../assets/vg_corpus/CORPUS.toml) + [scripts/fetch_corpus.ps1](../scripts/fetch_corpus.ps1) | Manifest tracked, payload **gitignored** and sha256-pinned before extraction. 7 licence-clean glTF assets, 2 279 237 triangles. |
| In-house `.glb` decoder | [crates/boyko_render/src/loaders/glb.rs](../crates/boyko_render/src/loaders/glb.rs) | glTF 2.0 binary → `MeshData`, zero third-party deps. Concatenates primitives and composes node hierarchies; bakes each placement into model space. |
| Reference-rig probe | [crates/boyko_app/tests/vg_r0_reference_rig.rs](../crates/boyko_app/tests/vg_r0_reference_rig.rs) | R0a. Records whether a Nanite reference is producible on this box — it is not — with the negative *re-derived by the machine* from the documented registry authorities rather than asserted by the author. |
| Frozen-symbol sweep | [tests/vg_symbol_reachability.rs](../tests/vg_symbol_reachability.rs) | Every frozen field must have a consumer or a recorded exception. Catches the campaign's signature defect: a threshold nobody reads, which manufactures the appearance of pre-registration while binding nothing. |

⚠️ **The census is armed by env and renders nothing on a normal run.** The GPU parts are `#[ignore]`d
and R0d additionally **skips by name** without the fetched payload — a payload-dependent gate that
stays silent is indistinguishable from one that passed.
