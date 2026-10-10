# Particle systems — research corpus (pre-design)

**Date:** 2026-08-20 · **Status:** research complete; feeds `docs/PARTICLES-PLAN.md` (architecture rung, in flight)
**Method:** five-engine survey (Bevy Hanabi, Unreal Niagara, Unity VFX Graph, Godot 4, Wicked Engine) + the classic GPU-driven literature, cross-checked against current source where the engine is open. Secondary sources are flagged. A parallel in-tree inventory of boyko mechanisms a particle system builds on is recorded at the end.

---

## TL;DR — what the design must know

1. **The industry converged on ONE GPU-driven skeleton** (unchanged since AMD GDC 2014): fixed-capacity particle pool + **dead-index free list** (atomic stack) + **two alive-index lists (ping/pong)** + a small **atomic counter block** + **indirect dispatch/draw args written by the GPU itself**. Wicked, Hanabi, GPUParticles11 and every credible implementation are variations of this. Zero CPU readback by construction.
2. **GPU attribute storage is AoS-of-packed-structs, not SoA.** Wicked packs one particle into exactly **64 B**; Hanabi builds a per-effect packed struct (**32 B default**: position, velocity, age, lifetime); Godot uses **112 B** (full 4×4 transform — the fattest, avoid). Each particle thread touches nearly all of its own attributes, so coalescing one contiguous 32–64 B read beats per-attribute columns. The CPU-side SoA argument does not transfer.
3. **Sorting is the biggest fork — and additive blending needs NO sort** ("additive blending just saturates the effect", GDC 2014). Where alpha-sorting is needed, production uses **GPU radix** (AMD FidelityFX Parallel Sort: Count→Reduce→Scan→Scatter, indirect), NOT bitonic — Wicked's `wiGPUSortLib` replaced its 2017 bitonic with FFX radix. WBOIT is the no-sort alternative for smoke-class low-opacity media.
4. **SDF collision is strictly better than depth-buffer collision, and boyko already owns the hard part** (brick atlas + analytic edit list). UE lists GPU particle collision as a first-class Global-Distance-Field use; Godot bakes a static SDF3D; Flax ships three Global-SDF particle modules. Depth-buffer collision is view-dependent, misses off-screen geometry, and needs previous-frame reprojection. Documented SDF failure modes to design around: thin geometry tunneling (UE), static bakes (Godot) — boyko's analytic primitive list can cover thin/dynamic colliders.
5. **The effect lives on the CPU/ECS; the particle lives on the GPU.** None of the five engines makes individual particles entities. The per-entity component is the emitter/effect instance; particles are a GPU-only datum indexed inside a pool.

---

## Per-engine facts (condensed; full source list at the end)

### Bevy Hanabi (v0.19)
- ECS mapping: `EffectAsset` (asset) / `ParticleEffect` + `CompiledParticleEffect` + `EffectSpawner` + `EffectProperties` (components) / `EffectParent` for GPU parent→child spawn events. Particles never entities.
- Storage: per-LAYOUT slab (`ParticleSlab`, min capacity 65,536, sub-allocated to many effects via a free-slice list) + an indirect index buffer of **3×u32 per particle**: `ping` (alive now), `pong` (alive next), `dead` (free list). Ping/pong rule: update writes ping, update AND render read pong, swap in the indirect pass — required because a one-frame-lifetime particle must be removable next frame.
- The packing algorithm (worth porting to a const-eval layout builder in `boyko_macros`): dedupe → sort by size → 16 B fields first → pair vec3+f32 (12+4) → pair vec2+vec2 → pad → trailing f32s. Default particle 32 B.
- The indirect pass (`vfx_indirect.wgsl`): `dead_count = capacity − alive_count`, `max_spawn = dead_count`, draw instance count reset to 0 and repopulated by update; a prefix-sum pass batches many effects into one dispatch.
- Known limits: collision **unimplemented**; effects with different Z cannot batch (documented perf cliff); shipped a 12 B indirect-buffer overrun at ~260 concurrent instances (issue #493 — size per-effect GPU tables from live instance count, not a constant).

### Unreal Niagara
- Stack: System → Emitter → Module (HLSL node graphs), groups × stages with explicit read/write namespaces.
- CPU/GPU split per emitter. Epic's own warning: "a simulation of 1 particle can potentially take up the same resources as one with 64" — many tiny GPU emitters is an anti-pattern.
- **Fixed Tick Delta**: substeps when fixed dt < frame time, but forces game-thread ticking — the determinism/parallelism trade made explicit.
- Collision: Scene Depth vs **Distance Field** (view-independent, "more reliable"; documented weaknesses: thin objects passed through, corners rounded) vs async HW ray trace (one frame late). Mesh DFs are offline, ≤8 MB / ≤128³ per mesh, composited into camera-centred clipmaps.
- Renderers: Sprite (ViewDistance/ViewDepth/Custom sort), instanced Mesh, Ribbon (tessellation/twist/link-order), per-particle Light (perf caution), Decal, Component ("expensive").
- Simulation Stages (GPU multi-pass over Grid2D/Grid3D/particles) power Niagara Fluids; practitioner figure: 1024² grid with complex logic ≈ 1–2 ms GPU (secondary).

### Unity VFX Graph
- Contexts Spawn→Initialize→Update→Output; attributes stored per-system, capacity allocated up front (`capacity ≈ spawn_rate × lifetime` is the field rule; oversizing costs FPS and VRAM).
- No exposed dead list: `alive` is a stored bool; kill is deferred one update (same one-frame constraint that forces ping/pong elsewhere).
- **GPU Events**: Trigger Event On Die / Rate / Always spawn into another system entirely GPU-side — the reference for event-driven sub-effects.
- DOTS story: **none first-party** — VFX Graph is a GameObject component; the field bridges ECS→VFX via GraphicsBuffer property binders. Treat as a cautionary example.
- Shipped bug: particle strips rendered the entire CAPACITY, not the live count.

### Godot 4 GPUParticles3D
- Compute-based (ParticlesShaderRD), not transform feedback. **112 B/particle** (`xform[16]` + velocity + active + color + custom).
- Frame params carry the whole environment as fixed arrays: ≤32 attractors, ≤32 colliders (sphere/box/**baked SDF3D**/heightfield/2D-SDF), ≤7 3D collision textures.
- SDF3D collider: editor-baked, **static at runtime**, "can represent holes, tunnels and overhangs".
- Trails: mesh skinning against RibbonTrailMesh/TubeTrailMesh with a bind-pose buffer.
- **Trap relevant to boyko's TAA: motion vectors only work with `draw_order = INDEX`** — per-frame depth reorder invalidates them.

### Wicked Engine (the cleanest shipping reference; read from current source)
- Particle struct exactly **64 B**: position+mass, force+packed rotation, velocity+maxLife, sizeBeginEnd+life+packed color.
- Counters (6×u32, never read back): `aliveCount, deadCount, realEmitCount, aliveCount_afterSimulation, culledCount, cellAllocator`. `THREADCOUNT_SIMULATION = 256` (64 with SPH grid on).
- Four passes: **kickoffUpdateCS** (ONE thread: swap alive counts, clamp deadCount ≥ 0, write next dispatch args) → **emitCS** (atomic-claim a dead index) → **simulateCS** (forces, force fields, capsule/sphere/plane colliders, depth-buffer collision via previous-frame reprojection, frustum cull, distance² sort key, append alive-new or return to dead, write vertex data) → **finishUpdateCS** (write `draw_culled` {4 verts × culledCount} and `draw_all` indirect args).
- Sort: **FidelityFX Parallel Sort (radix)** — the 2017 blog's bitonic is gone from the source.

---

## Classic literature — the load-bearing results

- **AMD GDC 2014 (Gareth Thomas)** — the canon's origin: ConsumeStructuredBuffer dead list, alive lists, bitonic (since superseded), depth collision, tiled compute rendering (≤1024 particles/tile, LDS sort, manual blend).
- **Mike Turitzin** — compute rasterization with per-pixel atomics: **2M+ particles, two VR eyes, 90 Hz**; rejects tiles ("particles are clumpy in screenspace"); measured **−27%…−81% vs raster** (worst case clumpy: 11.7→2.2 ms on GTX 980). Needs int64 atomics for the fast variant; does not compose with a VB path — a measured spike someday, not a default.
- **inFAMOUS: Second Son** — expression language → PSSL on PS4 async compute; curl noise; ribbons; **168k particles** (secondary).
- **Destiny (SIGGRAPH 2017)** — node graphs whose parameters are expressions with BOTH an HLSL converter and a CPU/GPU bytecode interpreter. boyko's eDSL (one generic body → f32 oracle + Emit printer) is the zero-overhead version of the same parity idea; an interpreter would violate principle 1.
- **Frostbite (SIGGRAPH 2015)** — particles voxelized INTO the froxel extinction volume (the high-quality lit-particle answer); the cheap answer is sampling the clustered light lists per billboard pixel.
- **Curl noise (Bridson 2007)** — divergence-free turbulence by construction; the standard "smoke that swirls without clumping" primitive; author it as a `boyko_shaderdsl` leaf so the CPU oracle validates it.
- **Half-res + bilateral upsample** — the standard fill-rate mitigation for soft particles; interacts with TAA motion vectors (see Godot trap).

## The universal algorithms (appear in every implementation)

1. Dead list = atomic stack (`deadCount` atomically decremented to claim; clamp `max(0, deadCount)` in kickoff — concurrent over-consume drives it negative).
2. Two alive lists + swap — required, not an optimization (one-frame-lifetime removal).
3. One-thread kickoff pass writing the next pass's DispatchIndirect args: this is HOW zero-readback is achieved.
4. Counters in one small GPU struct, never read back.
5. `InstanceCount` from a GPU counter, 4 verts/quad, `SV_VertexID`-synthesized corners + `SV_InstanceID` attribute pulling — no vertex buffer, no geometry shader.
6. Group-local atomic aggregation; one global atomic per group.
7. Sort-key (distance² or plane depth) generated inside simulate; sort is its own indirect pass.

## Pitfalls ledger (each one shipped somewhere)

- Indirect/metadata buffers sized from a constant instead of live instance count (Hanabi #493).
- Per-effect state in the batch key kills batching (Hanabi Z-sort).
- Many tiny GPU emitters (Epic: 1 particle can cost as 64).
- Bitonic as "the" GPU sort (production moved to radix).
- Sorting additive particles (pure waste — commutative operator).
- Tiled particle rendering under clumpy distributions (Turitzin's measured failure mode).
- Depth-buffer collision as the ONLY collision (view-dependent, reprojection, discontinuities).
- CPU readback anywhere in the loop (async, 1–2 frames late where unavoidable).
- Depth-sorted particles + TAA motion vectors (Godot: motion vectors only under index order).
- Rendering capacity instead of live count (Unity strips).
- Read-modify-write of one attribute in one pass (race; double-buffer + toggle).

## Scale calibration points

| Source | Count | Conditions |
|---|---|---|
| inFAMOUS: Second Son | 168k | PS4, async compute (secondary) |
| Turitzin | 2M+ | 2 VR eyes @90 Hz, compute raster, additive unsorted, GTX 980-class |
| Team Nutshell (Vulkan) | 99,720 | sim 0.05 ms + draw 0.17 ms, RTX 4070, 64 B/particle |
| Brian-Jiang/GPUParticles | >1M @60fps | DX11, GTX 1080, indirect everything |
| Bitonic reference | 1M u32 in 4 ms | Vulkan compute (vs std::sort 40 ms) |

## Applicability to boyko — copy / adapt / reject

**Copy:** the dead-list/dual-alive/indirect skeleton; a 32–64 B packed particle struct laid out by a compile-time packer (`boyko_macros`, port Hanabi's algorithm); the one-thread kickoff pass; vertex-pulled quads; sort-skipping for additive as a STRUCTURAL property (pipeline without a sort pass, not a runtime branch); FFX-style radix where sorting is real; curl noise as an eDSL leaf.

**Adapt:** SDF collision as the PRIMARY mode (everyone else retrofitted it after depth collision; boyko's field is world-space, already bindable — collision is ~10 lines in the simulate leaf: `d = field(p); if d < r { n = field_normal(p); v = reflect(v,n)*restitution; p += n*(r−d); }`); pool = subsystem-owned Resource with per-FIF device buffers (the in-tree inventory confirms dense components are CPU-only by plan decision and `GpuColumnManager::create_column` has zero production callers — `gpu_scene/interp.rs` is the shipping pattern); lit particles sample the existing froxel lists first, Frostbite-style voxel injection is a later rung; fixed timestep via `FixedSet::Gameplay` + uniform-fed dt without Niagara's game-thread constraint.

**Reject:** per-particle ECS entities (nobody does it; pure overhead at 10⁵–10⁶); a runtime expression interpreter (the eDSL's monomorphization already gives CPU/GPU parity at zero runtime cost); `Box<dyn Modifier>` stacks (static composition instead); a general CPU mirror sim (recreates the O11-SP4 parallel-data failure mode — scope any CPU path to "gameplay particles with CPU-visible state" if ever).

## Open questions handed to the architecture rung

1. Pool granularity: one global slab vs per-layout slabs (compile-time-declared layout set vs authoring-derived)?
2. Layout from `#[derive]`-style compile-time declaration (monomorphized, zero shader cache) vs authored effect asset (runtime layout, needs a permutation cache)? — the single decision that determines whether a shader cache exists.
3. Render-path landing: transparent composite after VB shade, reading which depth (`forward_depth` is available under VB)?
4. Sorting policy: per-effect vs one global radix vs WBOIT for smoke-class media?
5. TAA: do particles feed motion vectors, and does that force index-order rendering?
6. SDF collision band: brick atlas only vs atlas + analytic list for thin/dynamic colliders; tunneling threshold at typical `v × fixed_dt`?
7. Determinism: GPU dead-list atomics make spawn-slot assignment non-reproducible — waived for VFX, or bounded?
8. CPU-visible live counts for gameplay/audio: budget the 1–2-frame-late readback or forbid it?

## In-tree mechanism inventory (verified file:line, 2026-08-20)

The full 12-topic inventory is in the session record; the load-bearing findings:

- **Buffers/binding:** `RhiDevice::create_buffer/buffer_mapped_ptr` (`crates/boyko_rhi/src/device.rs:498,510`), bind-group builder (`device.rs:302-462`); the canonical subsystem example to copy end-to-end is `crates/boyko_app/src/gpu_scene/interp.rs:68-137` (module→layout→pipeline→per-FIF buffers→per-FIF sets).
- **Graph passes:** `add_pass`/`image_access`/`buffer_access` (`framegraph/graph.rs:459,496,518`); the armed/disarmed template is `sv0_pass` (`graph_bridge.rs:4607-4628` + `Option<PassId>` plan field + ResIds appended LAST) — disarmed frames name no ResId, so goldens stay byte-identical by construction.
- **Indirect:** `vkCmdDrawIndexedIndirect` live at `passes/vb.rs:2454,2980` (stride 20, instanceCount word 1 written by cull compute); **`vkCmdDispatchIndirect` is loaded but has ZERO call sites** — the particle emitter would be its first user; `BufferUsage::INDIRECT` + `BarrierStage::DRAW_INDIRECT` already exist.
- **Storage verdict:** dense components are CPU-only (plan non-goal: "GPU-resident dense" out of scope; zero production `storage="dense"` users); `GpuColumnManager::create_column` is test-only. Particle GPU state ⇒ subsystem-owned Resource with per-FIF `BoundBuffer`s (Principle-0-legitimate FFI/GPU-contiguity exception, same as every `gpu_scene` ring).
- **Froxel lists:** `cluster_grid`/`light_index`/`light_index_alloc` SSBOs (`gpu_scene/mod.rs:5364-5384`), helpers in `light_table.hlsli:335-381`, three-term runtime gate every consumer replicates (`vb_shade.comp.hlsl:549-577`).
- **SDF bindings:** `sdf_field.hlsli` include-contract (Buf@t0 before include); brick atlas ladder bindings in `sdf_forward_march.comp.hlsl:228-239`; the strict read-only field consumer template is `sdf_mesh_shadow.comp.hlsl` (bindings doc :42-58).
- **Transparent leg: does not exist.** No blend-enabled world-geometry pipeline today (only UI); `BlendState::PREMULTIPLIED_ALPHA` + lowering exist. The right slot for a depth-tested particle draw is the `sdf_forward_march` position: after the lit producer, before `present_sample`, in BOTH forward (`graph_bridge.rs:2344`) and VB (`:5367`) legs, reading `lit` as color attachment + `forward_depth` for soft depth.
- **eDSL recipe for a new compute leaf:** generic body module in `boyko_shaderdsl/src/`, `emit_hlsl_*` printer in `emit/shaders.rs` (copy the `:31-69` Names/ARENA pattern), an `emit_*` bin behind `required-features=["emit"]`, GENERATED sentinels in the `.comp.hlsl`, frozen dxc line in the header, `embed_spirv!` accessor, `*_spv_sync` + `*_edsl_sync` twin tests.
- **Knob plumbing to copy:** `BOYKO_SDF_MESH` six-hop pattern — env → request fields (default OFF) → armable predicate → clamp system (`sync_sv0_light_gate`, value-gated before DerefMut) → packed word bits with compile-time neighbour guards → runner passes only the RESOLVED mode into `scene()`.
- **Fixed timestep:** particle sim belongs in `FixedSet::Gameplay` (Snapshot ordering already wired in `plugins.rs:679-682`); GPU compute systems ride `SystemKind::GpuCompute` (dispatcher-solo, `gpu_system.rs:258-330`).

## Sources

AMD GDC 2014 (gdcvault.com/play/1020002) · GPUParticles11 (gpuopen.com) · Wicked Engine source (ShaderInterop_EmittedParticle.h, emittedparticle_{kickoffUpdate,simulate,finishUpdate}CS.hlsl, wiGPUSortLib.cpp) · FidelityFX Parallel Sort (gpuopen.com/fidelityfx-parallel-sort) · Turitzin (miketuritzin.com/post/rendering-particles-with-compute-shaders) · Hanabi (github.com/djeedai/bevy_hanabi: README, CHANGELOG, src/attributes.rs, src/render/vfx_indirect.wgsl, issue #493) · Niagara docs (dev.epicgames.com: key-concepts, scalability, DF collision, GPU raytracing collisions, mesh distance fields, render module reference) · Unity VFX Graph docs 17.0 (attributes, systems, initialize, GPU events) + changelogs · Godot (particles_storage.h, GPUParticles3D class ref, particle shaders, 3D particle collisions, 4.0 article) · inFAMOUS GDC 2014 (gdcvault.com/play/1020367) · Destiny SIGGRAPH 2017 (advances.realtimerendering.com/s2017) · Frostbite SIGGRAPH 2015 (advances.realtimerendering.com/s2015) · Bridson curl noise 2007 (cs.ubc.ca/~rbridson) · WBOIT (therealmjp.github.io, casual-effects.blogspot.com) · bitonic Vulkan (poniesandlight.co.uk) · Flax Global SDF (docs.flaxengine.com) · Team Nutshell (team-nutshell.dev) · half-res particles (github.com/slipster216/OffScreenParticleRendering, realtimecollisiondetection.net/blog/?p=91)
