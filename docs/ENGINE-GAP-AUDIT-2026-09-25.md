# Engine gap audit — what a shippable 3D game engine provides, and where boyko stands (2026-09-25)

> **Status:** architect's audit, revision 2, 2026-09-25. Revision 1 was reviewed the same day
> (CHANGES_REQUESTED: 2 critical, 6 important, 6 optional remarks). Every remark is either applied
> or answered in §10, the review log. This is a read-only pass over the trunk at **`6394bc5e`**
> (branch `integ/unified`, the merge of `u/phys-l9-c4`), read through the worktree `D:/wt/docs`.
> No build, test, timing or GPU run was taken, because three build lanes were running on the
> machine.
>
> **Citations.** Every `crates/…` path below was checked to exist at `6394bc5e`. Code is cited as a
> **path plus a symbol**. A `path:line` appears only where this revision re-read that line by
> content at `6394bc5e`. This document is not in `GATED_DOCS` (`tests/internal_docs_anchors.rs`),
> so no machine checks it: treat each citation as "true at `6394bc5e`".
>
> **Why it exists, and what it is not.** On 2026-09-25 the owner asked for research on:
>
> - post-processing and anti-aliasing in general;
> - an effects system (explosions, rain);
> - sun rays;
> - volumetric effects.
>
> This research batch answers that request in several documents:
>
> - **Technique-level research** is in three same-day sibling surveys:
>   [render/POSTFX-AA-RESEARCH.md](render/POSTFX-AA-RESEARCH.md),
>   [render/VOLUMETRICS-RESEARCH.md](render/VOLUMETRICS-RESEARCH.md) and
>   [render/VFX-RESEARCH.md](render/VFX-RESEARCH.md). Each one is the input to its own design
>   space. The batch also updates reflections and transparency, and adds surveys of dynamic
>   materials and audio.
> - **This audit** is the frame around them:
>   - the status of each of the owner's topics against the tree and its committed plans;
>   - their dependencies on each other and on the rest of the engine;
>   - the order of work;
>   - the hybrid (mesh vs SDF) forks each topic must settle;
>   - one table (§3.5) collecting each candidate technique's cost on the reference GPU, with its
>     source.
>
> §2 covers the rest of a complete engine at row granularity.
>
> **Numbers.** Each number is one of three kinds:
>
> - **published**, with its source and rig;
> - **derived**, with the arithmetic shown;
> - an **estimate**, labelled as one.
>
> Vendor numbers are marked as vendor claims. The in-tree reference GPU is an **RTX 3060 Laptop
> (6 GB)** ([OPTIMIZATION-PLAN-RENDER.md](OPTIMIZATION-PLAN-RENDER.md) §0). Its bandwidth is
> 288–336 GB/s peak, depending on the laptop's configuration. This document uses one assumed
> **250 GB/s effective** figure for every bandwidth floor. That is `TAA-PLAN.md`'s assumption,
> and the post-FX survey uses the same figure. Ratios between GPUs use peak against peak. No
> number here was measured on the reference GPU for this pass.

---

## 0. Summary

**The shape of the engine today.** boyko is unusually far along in three areas:

- **The kernel.** ECS, parallel scheduler, relationships, observers, reflection, serialization,
  determinism censuses.
- **The GPU-driven opaque renderer.** Four render paths, a visibility buffer, two-phase HZB,
  froxel light cull (VB only), CSM, SDF soft shadows and AO, SDF-DDGI, optional hardware
  ray-traced shadows, TAA and SMAA.
- **Diagnostics.** Logging, profiler, telemetry wire format, validation gates, goldens.

Most of what a player sees and hears beyond opaque lit geometry is **absent or exists only as a
design**:

- HDR post-processing;
- atmosphere and volumetrics;
- a complete VFX set;
- animation;
- audio;
- gameplay-grade physics queries;
- AI;
- networking;
- the editor;
- the shipping pipeline.

**Row counts.** A row is one capability. §2 and §3 together hold **204 rows**. The counts below
were taken mechanically from the Status and P columns, by a script over the tables.

| Status | Rows | Of which P0 | Meaning |
|---|---|---|---|
| PRESENT | 36 | 28 | Shipped on the trunk and reachable from `EnginePlugins` or a public API |
| PARTIAL | 40 | 12 | Shipped, but missing a part a shipped game needs (the part is named). A declared but unread seam counts here |
| PLANNED | 49 | 13 | No implementing code, but a committed design or plan in `docs/` covers it (a parked design included) |
| ABSENT | 79 | 15 | No code and no committed plan |

Of the 68 P0 rows, **40 are not PRESENT**. 13 of those 40 already have a committed design:

- HDR scene colour (reflections R4a = transparency R11);
- animation and GPU skinning;
- alpha-tested cutout;
- transparency;
- reflections;
- the character controller;
- the cook;
- stable asset ids;
- the data and scene languages;
- soft particles.

**The ten gaps that block shipping a real 3D game** (all P0). Every priority in this document is
set against one reference game: a single-player third-person 3D action game on Windows, sold on
Steam. §1.3 defines its scale.

1. **Linear HDR scene colour** (E1). It is **designed but not built**: R4a in
   [render/REFLECTIONS-DESIGN-SPACE.md](render/REFLECTIONS-DESIGN-SPACE.md), which is also R11 in
   the transparency design. Today, tonemap and OETF are baked into eight shader producers, which
   write an 8-bit `lit`. Every post effect, volumetric, additive VFX and reflection composes in
   linear HDR, so this one change unblocks the rest of the owner's current topics.
2. **Skeletal animation and GPU skinning** (H1–H5, D16). Designed, not built.
3. **Audio** (J1–J4, J8, C15). Nothing exists.
4. **Gamepad input** (A6) and **window modes and DPI** (A2). Only a gamepad seam exists:
   `BindSpec::Stick` is ignored at runtime.
5. **Physics for gameplay** (I2–I6, I9). The layer and mask fields exist and are read by nothing.
   Also missing: a capsule collider, ray/shape/overlap queries, trigger and contact events,
   kinematic motion and a character controller.
6. **Gameplay cameras** (K3). An orbit rig exists; follow with collision does not.
7. **Alpha-tested cutout and sorted transparency** (D7, D8).
8. **VFX fundamentals** (G2, G4, G10): soft particles, flipbooks, and HDR compositing of additive
   particles.
9. **GPU texture compression, an offline cook and stable asset ids** (C5, C10, C11).
10. **Navigation meshes and pathfinding** for any game with NPCs (S1, S2).

**The recommended order** (§4).

- **Wave 0** is the work already in flight: the unified plan's Phases C, D, E and F1–F3, the
  physics performance levers, and SI1.
- **The refactor campaign (Phase F, step F4) is not in Wave 0.** The owner's ruling Q-4 puts it
  "after everything else". §4.1 carries that reasoning over to the feature waves and places F4
  after Wave 5; Q7 in §7 asks the owner to confirm.
- **Six waves follow.** Each is a set of lanes on disjoint files:

| Wave | Render | Runtime and platform | Tools, content and gameplay |
|---|---|---|---|
| 1 Foundations | R4a HDR scene colour: tonemap moved into one tail pass on all four paths, TAA moved pre-tonemap, HDR particle compositing | window modes and DPI, gamepad → gamepad UI navigation, MMCSS; audio backend, mixer, WAV and Vorbis decoding | debug draw, console and cvars, native crash dumps; the Gaia bake (the cook) and data profile |
| 2 P0 gameplay + first image quality | exposure, bloom, LUT, specular AA, height fog, physical sky; alpha test, soft particles, flipbooks; **the SDF field at game scale** | animation rung 1 → skinning → root motion; the physics gameplay set → character controller → gameplay cameras; 3D audio | BC texture cook, async IO, pipeline cache; the Gaia scene profile (stable asset ids, the load fixup, cell streaming) |
| 3 World feel | froxel fog and god rays (classical vs SDF measured); particles to completion; dynamic materials, decals, rain; motion vectors on every pixel; temporal upscaler; transparency R0a–R6; reflections R0–R2 | DSP, music; IK, morph targets | navmesh, pathfinding, crowds, perception; timers, save completion, settings |
| 4 Breadth | the rest of reflections and transparency; terrain, foliage, water, clouds; area lights; DOF, motion blur, lens effects; Forward AA | joints, CCD, ragdolls, vehicles | editor v1, inspector, gizmos, undo; localization, shaping, accessibility |
| 5 Scale and ship | virtual-geometry LOD, texture streaming | Linux platform layer | VFS and packs, packaging, Steamworks, telemetry uploader, sequencer, video |
| 6 Genre and optional | HDR display output, MSAA | networking (moves to Wave 2 if the game is multiplayer) | destruction, motion matching, XR, runtime scripting |

---

## 1. Method

### 1.1 The reference taxonomy

The domain list is the union of six feature surfaces:

- **The runtime-architecture layering** of Gregory's *Game Engine Architecture* [S1].
- **Godot 4.7's published feature list** [S2]. It is the most complete single enumeration found:
  platforms, renderer, post, VFX, physics, audio, navigation, networking, i18n, XR.
- **Unity HDRP's feature list** [S3] and **Unity's DOTS package set** [S4][S5].
- **Unreal Engine 5.8 documentation** for post-processing, fog, volumes, world partition, Mass,
  Niagara and PSO caching [S9]–[S15].
- **The O3DE gem reference** [S6]. Its categories became domain rows: Animation, Audio, AI,
  Debug, Environment, Gameplay, Multiplayer, Scripting, UI and Utility.
- **Bevy 0.19** [S7], the closest peer: a Rust ECS engine.

Shipped-engine talks supply the practice and the numbers per row:

- Frostbite [S16][S17][S19]
- Decima [S20]
- id Tech 7 [S25]
- Call of Duty [S22]
- Assassin's Creed IV [S18]
- Overwatch [S31]
- NetherRealm [S32]
- Insomniac [S33]
- Ubisoft [S27]
- Halo 2 [S30]

### 1.2 How a status was assigned

| Status | Rule |
|---|---|
| **PRESENT** | Code on the trunk implements the capability, and a game can reach it: a plugin in `EnginePlugins` (`crates/boyko_app/src/plugins.rs`), or a public type or function. |
| **PARTIAL** | Code exists, but a named part that a shipped game needs is missing. The Evidence cell names both halves. **A declared seam that nothing reads or produces** (a type, field, enum variant or event reserved for the capability) counts as PARTIAL, with "seam" in the Evidence cell. A6, A7, I4 and D25 are all judged by this one standard. |
| **PLANNED** | No implementing code, but a committed design or plan in `docs/` covers it. The plan is cited, not its rung status. A design **parked** by an owner or measurement decision still counts as PLANNED, with "parked" in the Evidence cell. |
| **ABSENT** | No code and no committed plan. "No code" means a case-insensitive `rg` over `crates/**` for the capability's vocabulary found nothing relevant; the query is named where it is not obvious. |

**The same-day sibling documents do not change a status.** They are the surveys and design spaces
of this research batch, uncommitted when this was written. They are cited for technique and
numbers. A row they cover says so in its Evidence cell, and its status stays what the trunk and
the committed plans support.

**How the evidence was gathered.** Each status comes from a grep or a file read at `6394bc5e`,
plus the scout's engine-context brief of the same day for the rendering rows. Revision 2
re-opened every status the critique disputed, and every plan item named in an Evidence cell. The
brief's status-deciding claims were re-checked; for example:

- `ColliderShape` has two variants.
- `window.rs` handles no `WM_SIZE` or DPI message.
- The RHI `PrimitiveTopology` has only `TriangleList`.
- The glTF loader refuses skins.

### 1.3 Priority, effort and path coverage

**The reference game** is a single-player third-person 3D action game on Windows, released on
Steam, at 1080p–1440p on an RTX 3060-class GPU. This is an assumption; §7 Q1 asks the owner to
confirm or replace it. A multiplayer, open-world or VR target moves the P2 rows of those domains
to P0.

| Priority | Meaning for the reference game |
|---|---|
| **P0** | Cannot ship without it |
| **P1** | Players and reviewers expect it; shipping without it is a visible deficit |
| **P2** | Genre- or scale-dependent (open world, multiplayer, vehicles, cinematics) |
| **P3** | Niche or future (XR, consoles, path tracing) |

**Effort** is an estimate for one lane working under this repository's discipline: architect →
critic → red-first gates → goldens. It is calibrated against in-tree plan sizes: the particle
plan sizes its rungs S/M/L, and [OPTIMIZATION-PLAN-RENDER.md](OPTIMIZATION-PLAN-RENDER.md) sizes
rows [S]..[XL].

| Effort | Estimate |
|---|---|
| S | ≤ 2 weeks |
| M | 2–6 weeks |
| L | 2–4 months |
| XL | more than 4 months, or several campaigns |

**Path coverage** (§3 only). The tree differs sharply by render path, so every row in §3 carries
a Paths cell:

- **D**, **F**, **F+** and **VB** stand for Deferred, Forward, Forward+ and VisibilityBuffer.
  "all" means all four.
- The post and AA seam exists only on D and VB. `post_process_aa_supported` in
  `crates/boyko_render/src/render_path_config.rs` is true for those two only: `passes/forward.rs`
  records no post or AA pass, and `declare_forward_graph` declares none.
- The froxel light list exists only on VB. `ResolvedRenderPath::froxel_light_cull` is
  `clusters_wanted && path == VisibilityBuffer`, and the test `froxel_light_cull_is_vb_only` pins
  it.
- D and F+ bind placeholder cluster buffers. The `cluster_z_scale` doc in
  `crates/boyko_render/src/light.rs` records this.
- The particle draw is declared on all four paths: `declare_particle_draw` is called from
  `declare_deferred_graph`, `declare_forward_graph` and `declare_vb_graph`.

§3.6 collects what those three facts cost.

---

## 2. The audit, by domain

The Ref column cites §9. The Deps column names other rows by id.

### A. Platform and core runtime

| ID | Capability | Ref | Status | Evidence at `6394bc5e` | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| A1 | Window, event pump, swapchain recreation on resize | [S1][S2] | **PRESENT** | raw Win32 in `crates/boyko_rhi_vulkan/src/window.rs`; recreation on resize or out-of-date in `crates/boyko_rhi_vulkan/src/present/swapchain.rs` | P0 | — | — |
| A2 | Window modes (borderless and exclusive fullscreen), DPI awareness, cursor capture and confine | Godot [S2] | **ABSENT** | `window.rs` handles only `WM_CLOSE`, `WM_DESTROY`, `WM_INPUT`, and key and mouse messages; there is no `WM_SIZE`, `WM_DPICHANGED` or fullscreen path | P0 | A1 | S |
| A3 | Present modes (vsync on/off, mailbox) | — | **PRESENT** | `PresentModeConfig::{Fifo, Immediate, Mailbox}`, probed with fallback (`present/swapchain.rs`) | P0 | — | — |
| A4 | Frame pacing, frames in flight | — | **PRESENT** | `FRAMES_IN_FLIGHT = 2` (`crates/boyko_rhi_vulkan/src/present/mod.rs`); `wait_frame_in_flight` | P0 | — | — |
| A5 | Keyboard and mouse raw input; rebindable actions; persisted bindings | Godot input map [S2] | **PRESENT** | `boyko_input`: `InputMap`, `ActionState`, `persist/`, and the Raw Input translation in `win32.rs` | P0 | — | — |
| A6 | Gamepad: sticks, triggers, rumble, hot-plug | GameInput is a superset of XInput, DirectInput, Raw Input and HID [S35]; Godot supports 8 gamepads [S2] | **PARTIAL** | seam only: `BindSpec::Stick` is "the reserved gamepad seam — parsed and round-tripped", ignored at runtime in v1 (`crates/boyko_input/src/action/map.rs`) | P0 | A5 | S–M |
| A7 | Text input: IME composition, clipboard | Bevy 0.19 `EditableText` with IME [S7]; Godot [S2] | **PARTIAL** | seam only: `RawInputEvent::Text(char)` (`crates/boyko_input/src/raw/event.rs`, "text fields only, never gameplay") is matched as a no-op in `raw/queue.rs` and produced by nothing; `window.rs` handles no `WM_CHAR`, and there is no IME or clipboard path | P1 | L3 | M |
| A8 | Linux platform layer (window, input, audio) | Godot, UE and Bevy ship it [S2][S7] | **PARTIAL** | the `#[cfg(not(windows))]` `run_windowed` in `crates/boyko_app/src/runner.rs` is a stub; `libc` is used only for VM mmap | P2 (see §7 Q2) | A1, A2, J1 | L |
| A9 | Job system / work stealing | Gregory [S1]; DOTS jobs [S4] | **PRESENT** | `ThreadPool` (`crates/boyko_threadpool/src/thread_pool.rs`), Chase-Lev; parallel schedule | P0 | — | — |
| A10 | Thread priorities and MMCSS classes (audio thread, render submit) | Microsoft: tag audio work "Audio" or "Pro Audio" through the real-time work queue [S34] | **ABSENT** | no `SetThreadPriority`, `THREAD_PRIORITY` or `AvSetMmThreadCharacteristics` anywhere; affinity is a no-op stub | P1 | — | S |
| A11 | Memory: VM reservations, one allocator | Gregory [S1] | **PARTIAL** | kernel `VmReservation` (`crates/boyko_ecs/src/ecs/memory/vm.rs`); the `boyko_memory` crate is Phase C of the unified plan and is in flight ([UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md](unification/UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md) §4) | P0 | — | in flight (Wave 0) |
| A12 | Clocks: frame time, fixed step, interpolation alpha, timer resolution | Godot physics interpolation [S2] | **PRESENT** | `Time` (`crates/boyko_ecs/src/ecs/core/time/time.rs`), `FixedTime` (`fixed_time.rs`); the GPU prev/curr lerp in `GpuTransform3D`; `TimerResolutionGuard` (`crates/boyko_app/src/timer_resolution.rs`) | P0 | — | — |
| A13 | File-system abstraction: mount points, virtual paths | Godot `FileAccess` and PCK [S2] | **ABSENT** | assets load with `std::fs::read` on the calling thread (`AssetServer`, `crates/boyko_ecs/src/ecs/core/asset/server.rs`) | P1 | — | M |
| A14 | Archives: pack files, compression, patch and DLC packs | Godot PCK/ZIP and exportable packs for mods/DLC [S2] | **ABSENT** | — | P1 | A13, C10 | M–L |
| A15 | Async IO thread (overlapped IO / IoRing) | Spider-Man streaming [S33] | **ABSENT** | loading is synchronous (`server.rs`) | P1 | A13 | M |
| A16 | Logging with diagnostic codes | — | **PRESENT** | `boyko_log` (per-thread SPSC lanes, sink thread `boyko-log-sink`); code pages under `docs/diagnostics/` | P0 | — | — |
| A17 | Headless and dedicated-server mode | Godot headless server [S2] | **PARTIAL** | headless `App::run()` is preserved ([APP-HOST-PLAN.md](APP-HOST-PLAN.md) invariants); there is no server build profile | P2 | M1 | S |
| A18 | Web target (wasm32) | Godot web export [S2] | **PARTIAL** | `boyko_demo` has a wasm leg, but its build is "blocked upstream in `boyko_ecs` (32-bit const-eval on the layout asserts)" and its CI leg is non-fatal (a comment in `crates/boyko_demo/Cargo.toml`) | P3 | A1 | M |

### B. Kernel services (beyond storage and scheduling)

The ECS core itself is PRESENT. It is catalogued in [FEATURE_MAP.md](FEATURE_MAP.md) and
[REMAINING-GAPS.md](REMAINING-GAPS.md). The rows below are the services a game framework leans on.

| ID | Capability | Ref | Status | Evidence | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| B1 | ECS: storage, queries, scheduler, change detection, hooks, observers, relationships, commands | DOTS [S4]; UE Mass [S12] | **PRESENT** | [FEATURE_MAP.md](FEATURE_MAP.md) | P0 | — | — |
| B2 | App states and hierarchical state machines | UE StateTree [S12] | **PRESENT** | `crates/boyko_ecs/src/ecs/core/state/`; `state_chart!` (`crates/boyko_macros/src/state_chart/mod.rs`) | P0 | — | — |
| B3 | Reflection (type info, field access) | — | **PRESENT** | the `boyko_reflect` crate | P1 | — | — |
| B4 | World serialization | Godot text and binary scenes [S2] | **PARTIAL** | `save_world` (`crates/boyko_serialize/src/save.rs`). The loader runs no hooks and does not run the `requires` closure. The cure is ruled: **Gaia ballot F4 was answered on 2026-08-30** as GK-3's four-sub-pass suppress-then-fixup seam, which lands at Gaia G6 together with GK-1 ([gaia/CAMPAIGN.md](gaia/CAMPAIGN.md)). [SERIALIZATION-PLAN.md](SERIALIZATION-PLAN.md)'s "open ballot F4" predates the ruling | P0 | C11, R3 | M |
| B5 | Prefabs / entity templates | O3DE Prefab Builder [S6] | **PRESENT** | `Prefab` (`crates/boyko_ecs/src/ecs/core/clone/prefab.rs`), `EntityCloner` (`clone/cloner.rs`), `#[require]` | P0 | — | — |
| B6 | Spatial index as a kernel service (proximity, triggers, perception) | — | **PLANNED** | rung SI1, Phase E ([UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md](unification/UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md)); [aether-v2/SPATIAL.md](aether-v2/SPATIAL.md) (`boyko_spatial`) | P1 | Phase D | L |
| B7 | Deterministic replay | Overwatch leverages determinism [S31] | **PLANNED** | rungs RP-0..RP-3 and `boyko_replay` (unified-plan rulings U-23 and U-24); the FMA and transcendental censuses are already enforced | P2 | Phase D | L |
| B8 | Code hot reload | Bevy hot-patches systems via `subsecond` [S8] | **ABSENT** | modding v1 is load-only; unified-plan ruling U-10 rules out hot reload | P2 | — | M |
| B9 | Runtime-layout (dynamic) components for scripting and data | — | **ABSENT** | the P1 list in [REMAINING-GAPS.md](REMAINING-GAPS.md); only dynamic *tags* exist | P2 | — | M |
| B10 | Transform hierarchy and propagation | Gregory [S1] | **PRESENT** | `propagate_transforms` (`crates/boyko_scene/src/propagation.rs`); `Transform`, `GlobalTransform` | P0 | — | — |

### C. Content pipeline

| ID | Capability | Ref | Status | Evidence | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| C1 | glTF 2.0 import: static meshes, materials, embedded images | glTF is Godot's primary 3D import, also at runtime [S2] | **PARTIAL** | `GlbMeshLoader` (`crates/boyko_render/src/loaders/glb.rs`), binary `.glb` only. Supported: TRIANGLES, `POSITION`, `NORMAL`, `TEXCOORD_0`, `TANGENT`, `COLOR_0`. Refused with an error: skins, animations, morph targets, Draco/meshopt. `decode_static_pose` returns the bind pose. The missing half is H1's import | P0 | — | S (the rest is in H1) |
| C2 | FBX / USD / `.blend` import | Godot imports FBX and `.blend` [S2] | **ABSENT** | — | P3 | — | L |
| C3 | OBJ import | — | **PRESENT** | `ObjMeshLoader` (`crates/boyko_render/src/loaders/obj.rs`) | P3 | — | — |
| C4 | Image formats beyond PNG: JPEG, HDR/EXR (sky, IBL), DDS/KTX2 containers | Godot runtime image loading [S2] | **PARTIAL** | PNG only, in-house (`boyko_image`: `png.rs`, `inflate.rs`); glTF images of any other MIME type are skipped | P1 | — | M |
| C5 | GPU block compression (BC1–BC7 on PC) | Godot VRAM compression: BPTC, S3TC, ASTC, ETC2, Basis [S2] | **ABSENT** | the RHI `Format` enum (`crates/boyko_rhi/src/enums.rs`) has no BC format | P0 | C10 | M |
| C6 | Mip generation | — | **PARTIAL** | a runtime LINEAR blit on the UNORM image (`mip_levels_for`, `crates/boyko_render/src/texture.rs`, whose header records the gamma-space trade-off); no offline mips | P1 | C10 | S |
| C7 | Mesh processing: tangents, vertex-cache order, LOD simplification | — | **PARTIAL** | `crates/boyko_render/src/tangent.rs`; no simplifier or LOD generator | P1 | D4 | M |
| C8 | Shader build: offline, hermetic, byte-gated | UE PSO precaching exists because first-use compiles hitch [S15] | **PRESENT** | the offline DXC recipe; committed `.spv` byte-gated; [SHADER-VARIANT-MANIFEST.md](SHADER-VARIANT-MANIFEST.md) | P0 | — | — |
| C9 | Pipeline cache (`VkPipelineCache` persisted across runs) | [S15] | **ABSENT** | `crates/boyko_rhi_vulkan/src/ffi.rs` declares no `vkCreatePipelineCache` at all. The `PFN_vkCreateComputePipelines` and `PFN_vkCreateGraphicsPipelines` declarations take a `pipeline_cache` argument, and it is passed null. Every pipeline is built at boot from a fixed set today, so the cost is startup time, not hitching | P2 | — | S |
| C10 | Cook: authoring text → platform binary, built offline | UE cook; Godot export [S2] | **PLANNED** | the Gaia bake: own text → build-time bake → binary. "A second byte format is forbidden" ([gaia/CAMPAIGN.md](gaia/CAMPAIGN.md); work order in [AETHER-GAIA-REVISION-2026-08-29.md](AETHER-GAIA-REVISION-2026-08-29.md)) | P0 | N2 | L |
| C11 | Asset database: stable asset ids across runs | — | **PLANNED** | handles are process-local, "the single most dangerous finding for scenes". Gaia ballot F4's ruling supplies the carrier: a stable asset NAME in the file, resolved at load to the existing `MeshHandle(u32)`. It lands at G6 ([gaia/CAMPAIGN.md](gaia/CAMPAIGN.md); [ASSET-STREAMING-PLAN.md](ASSET-STREAMING-PLAN.md)) | P0 | C10 | M |
| C12 | Asset hot reload (textures, meshes, materials, shaders) | Godot live reloading [S2] | **PARTIAL** | `.ui` documents only: `UiHotReload`, an mtime+size poll (`crates/boyko_ui/src/reload/state.rs`) | P1 | A13 | M |
| C13 | Runtime residency: refcount, retire, staging | — | **PARTIAL** | kernel store F1–F5 landed (`Assets` refcount, `Retiring`, `AssetStaging`); IO is synchronous. Gaia ballots F4 and F5 were ruled on 2026-08-30, and F5 moved streaming into G6 | P1 | A15 | M |
| C14 | World partition / cell streaming | UE World Partition [S11] | **PLANNED** | Gaia G6: the cell catalog, `load_cell`/`unload_cell`, GK-1's cross-load map and cross-cell references. Ballot F5 ("all at once, properly, from the start") moved all of this out of G8 into G6 ([gaia/CAMPAIGN.md](gaia/CAMPAIGN.md)) | P2 | C10, C13, A15 | L |
| C15 | Audio import (WAV, Ogg Vorbis / Opus) | Godot: WAV, QOA, Ogg Vorbis, MP3 [S2] | **ABSENT** | no audio decoder in the tree; the same-day audio survey is [audio/AUDIO-RESEARCH.md](audio/AUDIO-RESEARCH.md) | P0 | J1 | M |
| C16 | Video playback | Godot Theora [S2]; O3DE Video Playback Framework [S6] | **ABSENT** | — | P2 | J1 | M |
| C17 | VRAM budget accounting with per-consumer caps | — | **PLANNED** | [OPTIMIZATION-PLAN-RENDER.md](OPTIMIZATION-PLAN-RENDER.md) §2a sums the large allocations against the reference part's 6 GB. The owner sets per-consumer caps (Open-Q13), and an overflow is "a setup-time hard error … never silent OOM" | P1 | — | S–M |
| C18 | Texture and mip streaming | Spider-Man streaming [S33] | **ABSENT** | every texture is resident from load (`AssetServer` is synchronous); no plan found (`rg -i 'mip.?stream\|texture.?stream\|virtual textur'` over `docs/`) | P1 | A15, C5, C17 | L |

### D. Rendering core

| ID | Capability | Ref | Status | Evidence | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| D1 | Render paths (Deferred, Forward, Forward+, VB) × geometry legs (Mesh, SDF, Both) | Godot's 3 renderers [S2] | **PRESENT** | `RenderPath`, `resolve_render_path` (`crates/boyko_render/src/render_path_config.rs`), boot-frozen | P0 | — | — |
| D2 | Render graph with derived barriers | Frostbite FrameGraph [S16] | **PARTIAL** | `FrameGraph` (`crates/boyko_rhi_vulkan/src/framegraph/graph.rs`), re-declared every frame with zero allocation. Missing: the transient-memory aliasing FrameGraph introduced [S16]; there are sync1 barriers only; `aa_out`, `taa_resolved` and the swapchain sit outside the graph | P1 | — | M |
| D3 | GPU-driven culling: frustum + occlusion | — | **PRESENT** | VB batch cull + two-phase HZB (`OcclusionConfig`, `HzbConfig`) | P0 | — | — |
| D4 | LOD / virtual geometry (cluster hierarchy) | UE Nanite (not cited here) | **PARTIAL** | the meshlet batch cull exists; there is no LOD hierarchy; VG-R0 K1 is adjudicated UNDECIDED ([MESHLET-VIRTUAL-GEOMETRY-PLAN.md](MESHLET-VIRTUAL-GEOMETRY-PLAN.md)) | P1 | C7 | XL |
| D5 | PBR metallic-roughness materials with bindless textures | Godot, HDRP [S2][S3] | **PRESENT** | `MaterialGpu` (48 B), `Material` (`crates/boyko_render/src/material.rs`), `MaterialTable` | P0 | — | — |
| D6 | Material authoring: per-material shaders, animated parameters, a global time uniform | HDRP Shader Graph [S3]; Godot visual shaders [S2] | **ABSENT** | `vb_shade` is one shader for all materials; `ViewUniform` (`crates/boyko_scene/src/camera.rs`) carries no time. The same-day survey and design are [render/DYNAMIC-MATERIALS-RESEARCH.md](render/DYNAMIC-MATERIALS-RESEARCH.md) and [render/DYNAMIC-MATERIALS-DESIGN-SPACE.md](render/DYNAMIC-MATERIALS-DESIGN-SPACE.md), not yet committed | P1 | — | L |
| D7 | Alpha-tested cutout (foliage, fences, hair cards) | Godot alpha hash/antialiasing [S2] | **PLANNED** | `vb_raster.fs.hlsl` and `forward_opaque.fs.hlsl` state "NO discard"; `MaterialGpu::base_color.w` is unread; the `MASKED` variant (R2, a hashed threshold) is in [render/TRANSPARENCY-DESIGN-SPACE.md](render/TRANSPARENCY-DESIGN-SPACE.md) | P0 | — | S–M |
| D8 | Transparency: sorted blend, OIT | HDRP; UE [S3][S9] | **PLANNED** | the ladder in [render/TRANSPARENCY-DESIGN-SPACE.md](render/TRANSPARENCY-DESIGN-SPACE.md), updated by [render/TRANSPARENCY-UPDATE-2026-09-25.md](render/TRANSPARENCY-UPDATE-2026-09-25.md); its R11 is E1 | P0 | E1 (for R5 and later) | L |
| D9 | Decals (clustered or deferred) | Godot clustered decals [S2]; HDRP decals [S3]; Doom Eternal decaling [S25] | **ABSENT** | — | P1 | D6 | M |
| D10 | Light types: directional, point, spot, sky | — | **PRESENT** | `DirectionalLight`, `PointLight`, `SpotLight`, `SkyLight` (`crates/boyko_render/src/light.rs`); froxel cull 16×9×24, VB only | P0 | — | — |
| D11 | Area lights, IES profiles, light cookies, light layers | HDRP rect/tube/disk lights, IES, cookies, rendering layers [S3]; Godot rectangular area lights [S2]; Bevy 0.19 rect area lights [S7] | **ABSENT** | — | P2 | — | M |
| D12 | Shadows: CSM, punctual atlas, contact, SDF, RT | — | **PRESENT** | `csm_config.rs` (≤ 4 × 2048²), `shadow_atlas.rs` (512² × 16), SSCS in `deferred_pbr.hlsl`, the SDF soft-shadow march, the `hwrt` feature | P0 | — | — |
| D13 | Global illumination | Godot: lightmaps, VoxelGI, SDFGI, SSIL [S2] | **PARTIAL** | SDF-DDGI, off by default (`DdgiConfig`); no baked lightmaps, no screen-space GI | P1 | — | L |
| D14 | Reflections: IBL, probes, SSR | Godot and HDRP ship probes + SSR [S2][S3] | **PLANNED** | [render/REFLECTIONS-DESIGN-SPACE.md](render/REFLECTIONS-DESIGN-SPACE.md), updated by [render/REFLECTIONS-UPDATE-2026-09-25.md](render/REFLECTIONS-UPDATE-2026-09-25.md). Today there is only the analytic `EnvBRDFApprox` + hemisphere sky; `RenderPathConsumers::ssr_on` is a reserved bit, "ALWAYS capped (no SSR exists engine-wide)" | P0 | E1 | L |
| D15 | Ambient occlusion | — | **PRESENT** | HBAO-lite + à-trous (`SsaoQuality`, `crates/boyko_render/src/ssao_config.rs`); the SDF 5-tap AO (`sdf_ao` in `vb_geo.comp.hlsl`) | P1 | — | — |
| D16 | GPU skinning and morph targets | — | **PLANNED** | the skin-cache compute pass in [animation/ANIMATION-DESIGN-SPACE.md](animation/ANIMATION-DESIGN-SPACE.md) | P0 | H1 | M |
| D17 | Special materials: hair, eye, fabric, subsurface, clear coat | HDRP Hair/Eye/Fabric/StackLit [S3]; Godot SSS [S2] | **ABSENT** | only the flag `MATERIAL_FLAG_TEXTURED` exists | P3 | D6 | L |
| D18 | Debug draw: lines, shapes, text in world space | O3DE Debug Draw gem [S6]; Bevy gizmos and text gizmos [S7] | **ABSENT** | the RHI topology enum (`crates/boyko_rhi/src/enums.rs`) has only `TriangleList` | P0 (dev) | — | S |
| D19 | Render debug views (buffers, overdraw, light complexity) | HDRP Rendering Debugger [S3] | **PARTIAL** | developer dumps only: `crates/boyko_app/src/hzb_dump.rs`, `host_dump.rs` | P1 | D18 | S–M |
| D20 | GPU timestamps and zones | — | **PRESENT** | `crates/boyko_rhi_vulkan/src/present/gpu_zone.rs`; no zone covers an AA or post pass yet | P0 | — | — |
| D21 | Async compute queue, timeline semaphores | Wronski: fog "in parallel with regular scene rendering … using asynchronous compute" [S18] | **ABSENT** | one GRAPHICS\|COMPUTE queue (`find_queue_family`, `crates/boyko_rhi_vulkan/src/device.rs`); no timeline semaphores | P2 | — | M |
| D22 | Indirect draws/dispatch with GPU-side counts; mesh shaders | — | **PARTIAL** | `cmd_dispatch_indirect` and `vkCmdDrawIndexedIndirect` are present; `DrawIndirectCount` and mesh shaders are absent. Two comments say of `vkCmdDispatchIndirect` and `vkCmdDrawIndexedIndirectCount` that "neither is in this device's fn table", and the dispatch half is stale in both: one in `present/passes/vb.rs`, and the late batch-cull comment in `present/graph_bridge.rs` (`declare_vb_graph`) | P2 | — | S |
| D23 | Dynamic resolution | HDRP [S3] | **ABSENT** | render scale is fixed at boot (`EnginePlugins::with_ssaa_scale`) | P1 | E14 | M |
| D24 | HDR display output (scRGB / HDR10) | Godot, HDRP [S2][S3] | **ABSENT** | the swapchain prefers `*_UNORM` in `SRGB_NONLINEAR`; no `VK_EXT_swapchain_colorspace` | P2 | E1 | M |
| D25 | Multiple views: split-screen, render-to-texture cameras, mirrors | — | **PARTIAL** | seam: `Camera { order, is_active, viewport: Option<Viewport> }` and `ActiveCamera` / `resolve_active_camera` select one camera per frame (`crates/boyko_scene/src/camera.rs`). There is no second view, per-camera target or viewport-sized pass | P2 | — | M |
| D26 | The SDF field at game scale: edit capacity, the brick backend, clipmap LOD, mesh distance fields | in-house design: [sdf-engine-architecture.md](sdf-engine-architecture.md) §6; render plan P9/P10 | **PARTIAL** | see the list after this table | P1 (P0 if levels are SDF-authored) | — | L |

**D26 evidence.** The edit authority `SdfEditField` is a fixed `[SdfEdit; MAX_SDF_EDITS]` array,
with `MAX_SDF_EDITS = 16` (`crates/boyko_sdf_math/src/lib.rs:105`), and its layout is
byte-identical to the physics kernel's edit list. Several consequences follow:

- **The brick atlas also bakes from at most 16 edits.** The atlas exists
  (`crates/boyko_rhi_vulkan/src/brick_atlas.rs`: the M2 40³ bake and the M3 dirty-brick re-bake),
  and so do a clipmap and a CPU mesh→SDF baker (`crates/boyko_sdf_math/src/mesh_sdf.rs`, MDF
  stage 2a). A marcher shadow tap (`FineMarcherPush::mesh_sdf_enabled`) reads the baker's output.
- **The windowed host arms none of them.** It binds a clipmap "baked from the EMPTY edit field"
  (`BrickClipmap::create`, `crates/boyko_app/src/gpu_scene/mod.rs:1809`), and it sets
  `mesh_sdf_enabled: false` (`:6693`).
- **Every SDF candidate in §5.3 is gated on this row** (§5.3).

### E–G. Post-processing, atmosphere and VFX

These are the owner's topics. §3 audits them: rows E1–E19, F1–F10 and G1–G16.

### H. Animation

| ID | Capability | Ref | Status | Evidence | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| H1 | Skeleton and clip assets; glTF skin/animation import | glTF; O3DE EMotion FX [S6] | **PLANNED** | [animation/ANIMATION-DESIGN-SPACE.md](animation/ANIMATION-DESIGN-SPACE.md), [animation/RUNG-1-PLAN.md](animation/RUNG-1-PLAN.md); `GlbMeshLoader` refuses skins today | P0 | C1 | L |
| H2 | Clip compression | — | **PLANNED** | the design's compressed clip | P1 | H1 | M |
| H3 | Sampling, blending, layered/additive, blend spaces | Godot [S2] | **PLANNED** | the design's pose bank + lane jobs | P0 | H1 | M |
| H4 | Animation state machines / graphs | UE StateTree [S12] | **PARTIAL** | the gameplay HSM `state_chart!` exists; an animation graph through the Aether `machine` is ladder rung L3 | P0 | H3 | M |
| H5 | Root motion | — | **PLANNED** | ladder rung L4 ("root motion + kinematic drive"); rung 1 writes the offsets as 0 | P1 | H3, I6 | S |
| H6 | IK: two-bone, look-at, foot placement | Godot IK [S2] | **PLANNED** | ladder rung L5 | P1 | H3, I3 | M |
| H7 | Morph targets | — | **PLANNED** | ladder rung L6, after skeletal; rung 1 refuses `targets` | P1 | H1 | M |
| H8 | Retargeting between skeletons | — | **ABSENT** | not on the ladder (the rung-1 plan mentions it only as a lane-reservation constraint) | P2 | H1 | M |
| H9 | Animation events / notifies | — | **PLANNED** | named in the design | P1 | H3 | S |
| H10 | Ragdoll and physical-animation blend | Jolt maps animation skeleton ↔ ragdoll [S28] | **PLANNED** | ladder rung L7; [physics/ADVANCED-PHYSICS-DESIGN-SPACE.md](physics/ADVANCED-PHYSICS-DESIGN-SPACE.md) | P2 | H3, I8 | M |
| H11 | Motion matching | Clavet, GDC 2016 [S27] | **PLANNED** | ladder rung L9; needs a mocap set | P3 | H3 | L |
| H12 | Cloth | Jolt soft-body cloth [S28]; O3DE NVIDIA Cloth [S6] | **PARTIAL** | XPBD soft bodies exist (`SoftBody`, `crates/boyko_physics/src/soft/component.rs`); there is no bending constraint or skinned-attachment cloth | P2 | H5, I13 | M |

### I. Physics beyond the rigid solver

Inventory source: [physics/ADVANCED-PHYSICS-RESEARCH.md](physics/ADVANCED-PHYSICS-RESEARCH.md) §0.
It was re-checked at `6394bc5e` for the collider enum, the collision filter, sensor output and the
absence of joints.

| ID | Capability | Ref | Status | Evidence | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| I1 | Rigid bodies, TGS-Soft solver, parallel colored solve, tree broadphase | Jolt [S28] | **PRESENT** | `boyko_physics` (`PhysicsConfig`, `BroadphaseTree`) | P0 | — | — |
| I2 | Collider shapes: capsule, convex hull, triangle mesh, heightfield | Jolt, Godot [S28][S2] | **PARTIAL** | `ColliderShape` has exactly `Sphere` and `Box` (`crates/boyko_physics/src/components.rs`); body-vs-SDF collision covers static SDF level geometry | P0 | — | L |
| I3 | Scene queries: raycast, shape cast, overlap | Jolt; Godot [S28][S2] | **ABSENT** | no query API in `boyko_physics/src`; `sdf_query.rs` is the internal body-vs-SDF narrowphase; `Ray` exists in `boyko_math` | P0 | I2, B6 | M |
| I4 | Collision filtering: layers, masks | Godot [S2] | **PARTIAL** | seam: `Collider { layer: u32, mask: u32 }` is documented as the broadphase filter ("the fields are wired for the Phase-10 broadphase"), and nothing in `boyko_physics/src` reads either field; only tests construct them. **The work is wiring these fields, not adding a second representation** | P0 | — | S |
| I5 | Triggers and contact events | Godot areas [S2] | **PARTIAL** | the narrowphase refills `Manifolds::sensor_overlaps()` (`crates/boyko_physics/src/resources.rs:2627`) every step for bodies carrying the `Sensor` marker. Missing: the row → entity mapping, enter and exit events, and any producer for the `Contact` component (`components.rs`) | P0 | I4 | S–M |
| I6 | Kinematic bodies (moving platforms, animated colliders) | Godot animatable bodies [S2] | **PARTIAL** | a `Kinematic` EnableTag bit exists, but kinematic motion is "an intentional deferral" (research §0) | P0 | — | S |
| I7 | Continuous collision detection | Jolt [S28] | **PLANNED** | [physics/ADVANCED-PHYSICS-DESIGN-SPACE.md](physics/ADVANCED-PHYSICS-DESIGN-SPACE.md) | P1 | — | M |
| I8 | Joints / constraints | Jolt, Godot [S28][S2] | **PLANNED** | same design: joints as a second constraint kind in the one graph | P1 | — | L |
| I9 | Character controller | Jolt rigid and virtual characters [S28] | **PLANNED** | same design: "a query pass whose state is a component" | P0 | I2, I3, I4 | M |
| I10 | Vehicles | Jolt wheeled, tracked, motorcycles [S28] | **PLANNED** | same design | P2 | I8 | L |
| I11 | Destruction / fracture | — | **PLANNED** | breakable welds only, in the same design | P3 | I8 | L–XL |
| I12 | Physics debug visualisation | O3DE PhysX Debug [S6] | **ABSENT** | — | P1 | D18 | S |
| I13 | Soft bodies | Jolt soft bodies [S28] | **PRESENT** | XPBD distance + tet volume, self-collision, SDF and rigid coupling (O11 SP1–SP4) | P2 | — | — |

### J. Audio

Nothing exists: no crate, backend, decoder, mixer or component. A grep for
`audio|wasapi|xaudio|sound` in `crates/` returns only unrelated words, so every row is ABSENT.
The same-day survey is [audio/AUDIO-RESEARCH.md](audio/AUDIO-RESEARCH.md).

| ID | Capability | Ref | Status | Evidence | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| J1 | Output backend: WASAPI shared mode, event-driven, small buffers | Windows audio engine latency is 1.3 ms since Windows 10; the default buffer is 10 ms; `IAudioClient3` selects smaller periods; the inbox HDAudio driver supports 128–480 samples (2.66–10 ms at 48 kHz) [S34] | **ABSENT** | — | P0 | A10 | M |
| J2 | Mixer: voices, buses, volume and pitch, sample-rate conversion | Godot buses and effects [S2] | **ABSENT** | — | P0 | J1 | M |
| J3 | Decoders and streaming music | Godot WAV/QOA/Vorbis/MP3 [S2] | **ABSENT** | — | P0 | C15 | M |
| J4 | 3D spatialization: panning, attenuation, Doppler, listener | Godot positional 2D/3D audio with Doppler [S2] | **ABSENT** | — | P0 | J2 | S–M |
| J5 | DSP effects and reverb zones | Godot effects [S2]; UE MetaSounds (search result, not fetched) | **ABSENT** | — | P1 | J2 | M |
| J6 | Occlusion and obstruction | — | **ABSENT** | a hybrid opportunity: the SDF field answers "is the path blocked" with a march (§5.3) | P2 | J4, I3 | M |
| J7 | Music system (layers, stingers, transitions) | — | **ABSENT** | — | P1 | J3 | M |
| J8 | ECS surface: `AudioSource`, `AudioListener`, one-shot events | — | **ABSENT** | — | P0 | J1–J4 | S |

The usual shortcut is middleware: O3DE ships a Wwise gem and a MiniAudio gem [S6]. The owner's
rule is in-house by default. §5.4 prices the one part that is costly to build in-house: the
compressed-audio decoder.

### K. Cameras

| ID | Capability | Ref | Status | Evidence | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| K1 | Camera component, projections, view uniform | — | **PRESENT** | `Camera`, `ViewUniform` (`crates/boyko_scene/src/camera.rs`) | P0 | — | — |
| K2 | Debug fly camera | — | **PRESENT** | `FlyCameraPlugin` (`crates/boyko_app/src/fly.rs`) | P1 | — | — |
| K3 | Gameplay cameras: third-person follow, spring arm with collision, first-person, shake, blends, rails | O3DE Camera Framework and Starting Point Camera [S6] | **PARTIAL** | `OrbitCamera` + `orbit_camera_system` (`crates/boyko_scene/src/camera.rs`) are a pure-state rig: target, distance, yaw, pitch, with the pose derived by the system. Missing: follow with collision (a spring arm), first-person, shake, blends and rails | P0 | I3 (or an SDF query) | M |

### L. UI, text, localization, accessibility

| ID | Capability | Ref | Status | Evidence | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| L1 | UI: layout, widgets, interaction, data binding, hot reload, world-space UI, UI animation, sprites | Godot GUI [S2]; O3DE LyShine [S6] | **PRESENT** | `boyko_ui` (`layout.rs`, `widgets.rs`, `interaction/`, `binding/`, `reload/`, `world/`, `animation.rs`, `sprite.rs`) | P0 | — | — |
| L2 | Latin text: MSDF atlas, kerning, wrapping | — | **PRESENT** | `boyko_fontbake`; `shape_into` (`crates/boyko_ui/src/text/shape.rs`), which is "Latin-first … no complex shaping / BiDi" | P0 | — | — |
| L3 | Complex shaping, bidirectional text, font fallback, CJK coverage | Godot bidi and OpenType shaping [S2]; Bevy 0.19 bidi [S7]; UAX #9 [S40]; HarfBuzz [S41] | **ABSENT** | see L2 | P1 | — | L |
| L4 | Localization: string tables, plurals, pseudo-localization, RTL mirroring | Godot CSV/gettext, pluralization, pseudolocalization, UI mirroring [S2] | **ABSENT** | — | P1 | L3 | M |
| L5 | UI navigation by keyboard and gamepad | — | **PARTIAL** | tab order through `Focusable { tab_index }` (`crates/boyko_ui/src/interaction/components.rs`); no gamepad spatial navigation | P0 | A6 | S |
| L6 | Accessibility: subtitles, text scaling, colour-blind filters, remapping, screen reader | Xbox Accessibility Guidelines [S36]; Godot screen-reader support [S2]; AccessKit [S50] | **PARTIAL** | remapping exists (`boyko_input` `persist/`); the rest is absent. Colour-blind filters need E4 | P1 | E4, L4, J8 | M |

### M. Networking

Nothing exists: no socket, replication or prediction code. The determinism contract and the
planned replay keys (B7) are strong foundations for lockstep or rollback.

| ID | Capability | Ref | Status | Evidence | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| M1 | Transport: UDP, reliability, fragmentation, encryption | Godot ENet high-level multiplayer [S2] | **ABSENT** | — | P2 | — | M |
| M2 | Replication: snapshots, deltas, interest management | UE Iris [S13]; Unity Netcode for Entities [S5] | **ABSENT** | — | P2 | M1, B7 | L |
| M3 | Client prediction, reconciliation, lag compensation | Overwatch ECS netcode [S31]; Netcode for Entities is "server authoritative with client prediction" [S5] | **ABSENT** | — | P2 | M2 | L |
| M4 | Rollback / deterministic lockstep | NetherRealm rolls back and re-simulates up to 8 frames in 16 ms [S32] | **ABSENT** | the determinism censuses and RP-* rungs are the prerequisites | P2 | B7, M1 | L |
| M5 | Sessions, lobbies, matchmaking, NAT traversal | platform SDKs [S47] | **ABSENT** | — | P2 | T1 | M |

### N. Scripting and modding

| ID | Capability | Ref | Status | Evidence | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| N1 | Compile-time authoring DSL | UE Verse and Godot GDScript are the runtime equivalents (not cited) | **PRESENT** | Aether v1: `aether!` (`crates/aether/src/lib.rs`), which expands to ordinary Rust | P1 | — | — |
| N2 | Data language: scenes, UI documents, data tables | Bevy 0.19 BSN scenes [S7] | **PLANNED** | Gaia ([gaia/CAMPAIGN.md](gaia/CAMPAIGN.md), rungs G0..G8); its syntax plan awaits the owner's approval | P0 | — | L |
| N3 | Runtime scripting (VM or WASM) | O3DE Script Canvas and Lua [S6]; Godot GDScript/C# [S2] | **ABSENT** | the options are surveyed in [modding/MODDING-DESIGN-SPACE.md](modding/MODDING-DESIGN-SPACE.md) | P3 | B9 | L |
| N4 | Modding | Godot exportable PCKs [S2] | **PLANNED** | [modding/MODDING-DESIGN-SPACE.md](modding/MODDING-DESIGN-SPACE.md) (closed at rev 6); [unification/UNIFIED-SYSTEM-PLAN-05-MODDING-READINESS.md](unification/UNIFIED-SYSTEM-PLAN-05-MODDING-READINESS.md) | P2 | Phase D | L |

### O. Tools

| ID | Capability | Ref | Status | Evidence | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| O1 | Editor: scene editing, play-in-editor | Godot editor [S2]; Bevy editor prototypes [S49] | **PLANNED** | [editor/EDITOR-DESIGN.md](editor/EDITOR-DESIGN.md) rev 4 (the v1 ladder, agent-drivable) | P1 | N2, O2, O3, O4 | XL |
| O2 | Inspector / property editing | Bevy plans an entity inspector [S7] | **ABSENT** | the enabler, `boyko_reflect`, exists | P1 | B3, L1 | M |
| O3 | Gizmos: translate, rotate, scale, picking, grid | Bevy 0.19 transform gizmo and infinite grid [S7] | **ABSENT** | — | P1 | D18 | M |
| O4 | Undo / redo | — | **PLANNED** | [editor/EDITOR-COMMANDS.md](editor/EDITOR-COMMANDS.md) | P1 | O1 | M |
| O5 | Profiler: CPU zones, GPU zones, statistics, in-game overlay | Godot visual profiler plus Tracy/Perfetto tracing [S2] | **PRESENT** | `boyko_diag`; `crates/boyko_ecs/src/ecs/core/profiling/`; the overlay in `crates/boyko_ui/src/profiling_overlay.rs` (rung 15). Export to an external viewer is absent | P0 | — | — |
| O6 | Console, cvars, cheat commands | O3DE Remote Tools and ImGui gems [S6] | **ABSENT** | no cvar or console code | P1 | B3 | S–M |
| O7 | Crash reporting: native minidump, symbolication, upload | O3DE Crash Reporting gem [S6]; Crashpad [S45] | **PARTIAL** | a crash file on the panic path only: `arm` (`crates/boyko_log/src/sink/crash.rs`). There is no structured-exception handler and no minidump, and an access violation (`0xC0000005`) is not a Rust panic | P1 | — | M |
| O8 | Player telemetry | O3DE AWS Metrics [S6] | **PARTIAL** | the wire format and codec, profiling rung 13 (`crates/boyko_diag/src/telemetry.rs`); no uploader | P2 | T1 | M |
| O9 | Bug-repro replay | — | **PLANNED** | [editor/EDITOR-REPLAY.md](editor/EDITOR-REPLAY.md); RP-* (B7) | P2 | B7 | L |
| O10 | Automated tests, goldens, CI, validation gate | — | **PRESENT** | `.github/workflows/ci.yml`; `goldens/PINS.toml`; `crates/boyko_app/tests/boot_validation_clean.rs` | P0 | — | — |

### P. World building

| ID | Capability | Ref | Status | Evidence | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| P1 | Terrain: heightfield, quadtree/clipmap LOD, splat materials, holes | Far Cry 5 GPU quadtree terrain [S37]; O3DE Terrain [S6] | **ABSENT** | SDF terrain is the hybrid alternative, decided by measurement (§5.3) | P1 | D6, I2 | L |
| P2 | Foliage: GPU instancing, procedural placement, wind, impostors | Ghost of Tsushima procedural grass [S38]; O3DE Vegetation [S6] | **ABSENT** | — | P2 | D7, D6, P1, G16 | L |
| P3 | Water: ocean, rivers, refraction, caustics, buoyancy | HDRP Water Surface [S3]; Doom Eternal water [S25] | **ABSENT** | — | P2 | D8, E1 | L |
| P4 | Large-world coordinates (f64 or origin rebasing) | UE 5 large world coordinates (not re-verified this pass) | **ABSENT** | `boyko_math` is f32 | P2 | — | M |
| P5 | Procedural content generation | O3DE Gradient Signal and Surface Data [S6] | **ABSENT** | — | P3 | P1 | L |

Sky, weather, time of day and wind are rows F5, G14, F10 and G16 in §3.

### Q. Cinematics

| ID | Capability | Ref | Status | Evidence | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| Q1 | Sequencer / timeline: keyed tracks on entities, cameras, events, audio | O3DE Maestro Cinematics [S6] | **ABSENT** | — | P2 | H3, K3, J8, Q4 | L |
| Q2 | Curves and tweens for world entities | — | **PARTIAL** | UI animation exists (`crates/boyko_ui/src/animation.rs`); nothing for world entities | P2 | — | S |
| Q3 | Offline movie capture (fixed-step render to file) | Godot Movie Maker mode [S2] | **ABSENT** | — | P3 | J1 | S |
| Q4 | Splines: spatial paths for camera rails, AI routes, roads, ribbons | — | **ABSENT** | `rg -i 'spline\|bezier'` over `crates/` finds only texture filters (Catmull-Rom history reads in TAA and SSAO) | P2 | — | S–M |

### R. Gameplay framework

Game flow states (O3DE's Game State gem [S6]) are row B2 and are not repeated here.

| ID | Capability | Ref | Status | Evidence | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| R2 | Tags, events, messages | — | **PRESENT** | static and dynamic tags, EnableTag, events ([FEATURE_MAP.md](FEATURE_MAP.md)) | P0 | — | — |
| R3 | Scene / level files | Bevy BSN [S7] | **PLANNED** | Gaia's `scene` profile (G6, N2); Aether's `scene` construct spawns one fn | P0 | N2, C11 | L |
| R4 | Timers and cooldowns as components | — | **ABSENT** | only the `Time` / `FixedTime` resources | P1 | — | S |
| R5 | Save games: slots, versioning, migration of old saves | O3DE Save Data gem [S6] | **PARTIAL** | world serialization exists with B4's and C11's gaps; save-slot management, format versioning and migration are absent | P0 | B4, C11 | M |
| R6 | Settings: graphics presets, rebinding UI, persistence | — | **PARTIAL** | input bindings persist; graphics arms are boot-frozen (`resolve_render_path` runs once), so settings need a restart-to-apply flow | P1 | — | S–M |
| R7 | Ability / damage / inventory kits | — | **ABSENT** | genre-specific; component kits, not engine code | P2 | — | M |

### S. AI

| ID | Capability | Ref | Status | Evidence | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| S1 | Navmesh generation, tiled, with dynamic obstacles | Recast + DetourTileCache [S29]; Godot runtime generation [S2]; O3DE Recast gem [S6] | **ABSENT** | — | P0 (NPC games) | I2 | L |
| S2 | Pathfinding: A* over polygons, string pulling | Detour [S29]; Godot A* [S2] | **ABSENT** | — | P0 (NPC games) | S1 | M |
| S3 | Crowds and local avoidance | DetourCrowd [S29]; UE Mass for "tens of thousands of AI agents" [S12] | **ABSENT** | — | P1 | S2, B6 | M |
| S4 | Decision making: behaviour trees, utility, HSM | Halo 2 behaviour DAG [S30]; UE StateTree = behaviour-tree selectors + a state machine [S12] | **PARTIAL** | HSM through `state_chart!`; per-entity machines are planned ([aether-v2/MACHINES.md](aether-v2/MACHINES.md)); no BT or utility layer | P1 | B2 | M |
| S5 | Perception: sight, hearing, line of sight | — | **ABSENT** | needs B6 (the spatial index) and I3 (raycast) | P1 | B6, I3 | M |

### T. Shipping and platform services

| ID | Capability | Ref | Status | Evidence | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| T1 | Store SDK: achievements, cloud saves, overlay, rich presence | Steamworks [S47]; O3DE Achievements gem [S6] | **ABSENT** | — | P1 | — | M |
| T2 | Release packaging, installer layout, symbol store | — | **ABSENT** | — | P0 (at ship) | A14, O7 | S–M |
| T3 | CI and docs deploy | — | **PRESENT** | `.github/workflows/ci.yml`, `docs.yml` | P0 | — | — |
| T4 | Patching and DLC | Godot exportable packs [S2] | **ABSENT** | — | P2 | A14 | M |
| T5 | Consoles | — | **ABSENT** | — | P3 | — | XL |

### U. XR

| ID | Capability | Ref | Status | Evidence | P | Deps | Effort |
|---|---|---|---|---|---|---|---|
| U1 | OpenXR runtime integration | Godot OpenXR [S2]; HDRP VR [S3]; OpenXR [S46] | **ABSENT** | — | P3 | D25 | L |

---

## 3. The owner's topics in depth

This section covers post-processing and anti-aliasing (E), sun rays and volumetrics (F), and the
effects system including explosions and rain (G).

### 3.0 Where the technique research lives

| Topic | Survey (same research batch) | What it contributes that this audit does not repeat |
|---|---|---|
| Post-FX and AA | [render/POSTFX-AA-RESEARCH.md](render/POSTFX-AA-RESEARCH.md) | techniques T1–T30, pitfalls P1–P24, the chain order in shipped engines, a bandwidth model |
| Volumetrics and sun rays | [render/VOLUMETRICS-RESEARCH.md](render/VOLUMETRICS-RESEARCH.md) | techniques T1–T31; per-froxel cost normalized from AC4, Frostbite and UE and scaled to the reference GPU; seven plan-delta facts about Track E |
| Effects, explosions, rain | [render/VFX-RESEARCH.md](render/VFX-RESEARCH.md) | the blend-fill model of the reference GPU, explosion and rain practice, a numbers ledger |
| HDR, reflections, transparency | [render/REFLECTIONS-UPDATE-2026-09-25.md](render/REFLECTIONS-UPDATE-2026-09-25.md), [render/TRANSPARENCY-UPDATE-2026-09-25.md](render/TRANSPARENCY-UPDATE-2026-09-25.md) | R4a re-verified; the re-bless count (34 pins, 61 legs) |

Each survey has a same-day design space, pass 1 and not yet critiqued:
[render/POSTFX-AA-DESIGN-SPACE.md](render/POSTFX-AA-DESIGN-SPACE.md),
[render/VOLUMETRICS-DESIGN-SPACE.md](render/VOLUMETRICS-DESIGN-SPACE.md) and
[render/VFX-DESIGN-SPACE.md](render/VFX-DESIGN-SPACE.md). All three take R4a as designed and as
their first prerequisite. The post-FX design grows R4a's `tonemap` pass into its uber pass
(`post_final`, rung PX1), which is this audit's fused tail pass. It also decides `display` as a
single image with a cross-frame seed.

§3.5 lifts from those surveys the one thing this audit's order depends on: what each candidate
technique costs on the reference GPU. Where a survey or design and this audit disagree, they rule
on technique and this audit rules on status and order.

### 3.1 The dependency that decides the order: HDR scene colour — adopt R4a as designed

**Today.** Tonemap, exposure and a manual `pow(x, 1/2.2)` OETF run **inside eight producers**:

- `deferred_pbr.hlsl`
- `forward_opaque.fs.hlsl`
- `forward_sky.fs.hlsl`
- `sdf_forward_march.comp.hlsl`
- `ssaa_downsample.fs.hlsl`
- `vb_resolve.comp.hlsl`
- `vb_shade.comp.hlsl`
- `vb_shade_split.comp.hlsl`

Each producer stores into `lit`, which is `R8G8B8A8_UNORM`. `lit` is a **ring of
`FRAMES_IN_FLIGHT` = 2** images (`GBufferTargets::lit`,
`crates/boyko_rhi_vulkan/src/present/targets.rs:127`). The eight-producer list is R4a's own
(`grep -l 'OETF_GAMMA_EXP|tonemap_select'` over `shaders/`), and it corrects an earlier count of
three. Revision 1 of this audit repeated that three-producer undercount.

Everything after lighting therefore works on 8-bit display-referred colour:

- **Particles.** The particle plan records "LDR additive clipping/quantization"
  ([PARTICLES-PLAN.md](PARTICLES-PLAN.md)).
- **TAA and SMAA.** Both resolve on the 8-bit post-tonemap image. Karis's TAA resolves in HDR with
  a luminance weight that tames fireflies [S23]; that option does not exist here.
- **Every missing post effect.** Bloom, auto-exposure, LUT grading, motion blur and DOF all need
  scene-referred input.
- **Volumetrics.** In-scattered light adds to surface radiance before the tonemap.

**The design exists, and has been through critique.** R4a in
[render/REFLECTIONS-DESIGN-SPACE.md](render/REFLECTIONS-DESIGN-SPACE.md) and its decision D4 are
the same decision as R11 in [render/TRANSPARENCY-DESIGN-SPACE.md](render/TRANSPARENCY-DESIGN-SPACE.md).
[TAA-PLAN.md](TAA-PLAN.md) technical item 8 had booked the TAA half. This audit adopts R4a
unchanged and re-opens none of its decisions:

| Decision | What R4a decided |
|---|---|
| Format | `lit` becomes **B10G11R11** when the boot probe RK-14, `DeviceCaps::lit_hdr_format_ok`, passes. B10G11R11 storage is device-optional. Otherwise `lit` becomes **R16G16B16A16_SFLOAT**, whose storage support is mandatory. The fallback degrades bandwidth and is never a fail-fast. The format is read once, at target creation; the shaders differ only by a `-D LIT_FORMAT` variant row |
| Producers | All eight lose their tail. One `tonemap` pass writes a new full-resolution `display` (`R8G8B8A8`), which FXAA, SMAA and the present read instead of `lit` |
| SSAA | `ssaa_downsample` becomes a plain HDR box filter `lit(2×) → lit(native)` ahead of the tonemap. That closes the recorded "avg(tonemap(x)), not tonemap(avg(x))" ceiling |
| TAA | TAA moves pre-tonemap with luma-weighted blending ([TAA-PLAN.md](TAA-PLAN.md) item 8) |
| Gate | **Eight per-producer transition pins**. For the un-armed configuration, the pre-move hash must equal the post-move `display` hash, once per producer, so that a producer that keeps its tail cannot hide behind a deferred-only pin. A 50× highlight fixture must clip before the move and not after it. The RK-14 fallback arm is forced in a test boot |
| Goldens | Every pin re-blesses by construction (P41). The reflections update counts **34 pins and 61 legs**, not the 30 pins of 2026-09-10 |

**What revision 1 got wrong here, and what changes:**

- It treated the format as "a measurement, not a preference". D4 decided it. The one open
  measurement is what the fallback arm costs on a device that fails RK-14.
- It proposed an `HdrMode::Off` arm. Neither design contains one. Keeping an LDR arm would double
  every producer's variants and would contradict D4 and P41, so the proposal is withdrawn.
- **Forward and Forward+ have no post seam** (§1.3). The tail `tonemap` pass therefore needs a
  new seam in `passes/forward.rs` and in `declare_forward_graph`. That is in E1's effort now:
  **M + S**.
- A side effect is worth stating: once that seam exists, **every effect fused into the tail pass
  reaches all four paths**. That includes exposure, bloom composite, LUT, vignette, grain and
  analytic fog.

**What it costs** (derived, decimal MB; 1080p = 2.07 Mpx, 1440p = 3.69 Mpx, 4K = 8.29 Mpx). R4a
adds `display` and re-types the `lit` ring. `lit_prev` is a reflections R5 image (R4a's §1.5
table), so it is not part of E1 and is listed separately.

| Item | 1080p | 1440p | 4K |
|---|---|---|---|
| Today: the `lit` ring, 2 × RGBA8 | 16.6 MB | 29.5 MB | 66.4 MB |
| R4a on the probed arm: the `lit` ring, 2 × B10G11R11 | 16.6 (+0) | 29.5 (+0) | 66.4 (+0) |
| + `display`, 1 × RGBA8 | **+8.3** | **+14.7** | **+33.2** |
| R4a on the fallback arm: the `lit` ring, 2 × RGBA16F | 33.2 (+16.6) | 59.0 (+29.5) | 132.7 (+66.4) |
| + `display` | +8.3 | +14.7 | +33.2 |
| **E1 delta over today, probed / fallback arm** | **+8.3 / +24.9** | **+14.7 / +44.2** | **+33.2 / +99.5** |
| Under SSAA (the `lit` ring at 2× per axis): the fallback arm's extra | +66.4 | +118.0 | +265.4 |
| + the native HDR downsample target, 1 image (probed / fallback) | +8.3 / +16.6 | +14.7 / +29.5 | +33.2 / +66.4 |
| Reflections R5's `lit_prev` ring, 2 × `lit` format (probed / fallback), not E1 | +16.6 / +33.2 | +29.5 / +59.0 | +66.4 / +132.7 |

If `display` is ringed like `lit` and `aa_out`, double its line. R4a's table says "full", with no
ring. Revision 1's "costs no VRAM over today" was false: even on the probed arm, E1 adds
`display`. On the fallback arm at 4K, E1 plus SSAA adds about 0.43 GB on a 6 GB part
(99.5 + 265.4 + 66.4 MB). That arm needs a cap in the §2a VRAM budget (C17).

**The bandwidth rule a post chain must obey** (derived, at the assumed 250 GB/s effective). A
byte per pixel costs 0.0083 / 0.0147 / 0.0332 ms at 1080p / 1440p / 4K; the post-FX survey §4.2
uses the same table.

- A separate full-screen pass that reads and writes `lit` moves 8 B/px on the probed arm and
  16 B/px on the fallback arm. That is a floor of **0.066 / 0.12 / 0.27 ms** and
  **0.13 / 0.24 / 0.53 ms** per pass, respectively.
- Five such passes at 4K cost 1.33 ms on the probed arm and 2.65 ms on the fallback arm.
- One fused tail pass costs about 9 B/px on the probed arm and 13 B/px on the fallback arm:
  - it reads `lit` (4 or 8 B/px);
  - it writes `display` (4 B/px);
  - it reads the first bloom level (about 1 B/px);
  - the exposure, LUT, vignette and grain inputs are negligible or cache-resident.

  That is **0.075 / 0.13 / 0.30 ms** and **0.11 / 0.19 / 0.43 ms**.

Unreal's tonemapper stage is built this way, with grading, film, vignette and grain as properties
of one post stage [S9]. HDRP states that it "combines some effects into the same Compute Shader"
(post-FX survey T10). The recommendation stands: **one fused tail pass, plus the few passes that
genuinely need neighbourhoods** (the bloom pyramid, DOF, motion blur, TAA).

### 3.2 Post-processing and anti-aliasing (E)

| ID | Capability | Ref | Status | Evidence | Paths | P | Deps | Effort |
|---|---|---|---|---|---|---|---|---|
| E1 | Linear HDR scene colour; tonemap moved to one tail pass | all surveyed engines [S2][S3][S9] | **PLANNED** | R4a + D4 ([render/REFLECTIONS-DESIGN-SPACE.md](render/REFLECTIONS-DESIGN-SPACE.md)) = R11 ([render/TRANSPARENCY-DESIGN-SPACE.md](render/TRANSPARENCY-DESIGN-SPACE.md)); [TAA-PLAN.md](TAA-PLAN.md) item 8; §3.1 | all (8 producers); F and F+ need a new post seam | P0 | RK-14 (inside R4a) | M + S |
| E2 | Exposure: manual and automatic (histogram, eye adaptation, local exposure) | Godot auto exposure [S2]; UE Exposure and Local Exposure [S9]; HDRP Exposure [S3] | **PARTIAL** | a manual scalar only: `LightingConfig::exposure` (`crates/boyko_render/src/light.rs`), "the FINAL multiply". Auto-exposure is plan item E-EXP (a 256-bin log-luma histogram) in [OPTIMIZATION-PLAN-RENDER.md](OPTIMIZATION-PLAN-RENDER.md) Track E | all, after E1 | P1 | E1 | S–M |
| E3 | Tonemapping operators | Godot: Linear, Reinhard, Filmic, ACES, AgX [S2] | **PRESENT** | `Tonemapper` (the Hill ACES fit, Khronos PBR Neutral, Reinhard-Jodie); hand-written HLSL, not eDSL | all | P0 | — | — |
| E4 | Colour grading: 3D LUT, white balance, lift/gamma/gain | Godot 1D/3D LUT [S2]; UE Color Grading [S9] | **PLANNED** | plan item E-LUT, "3D-LUT grade + grain/CA/vignette" (Track E, polish tier) | all, after E1 | P1 | E1 | S |
| E5 | Bloom | CoD:AW bloom [S22]; Godot glow with dirt map [S2]; UE Bloom [S9] | **PLANNED** | plan item E-BLOOM: "13-tap Karis down + 9-tap tent up, ~6 mips", consuming X-SPD | D, VB; all once the pyramid pass is declared on F | P1 | E1 | S–M |
| E6 | Lens flare, dirt mask | UE [S9]; HDRP data-driven and screen-space flare [S3] | **ABSENT** | in no plan | all, after E1 | P2 | E5 | S |
| E7 | Depth of field | CoD:AW [S22]; Godot bokeh DOF [S2]; UE Cinematic DOF [S9] | **PLANNED** | plan item E-DOF (separable disk / scatter-as-gather) | D, VB | P2 | E1 | M |
| E8 | Motion blur | CoD:AW [S22]; McGuire's reconstruction filter [S39]; UE [S9] | **PLANNED** | plan item E-MBLUR (TileMax → NeighborMax → reconstruct) | D, VB | P2 | E1, E13 | M |
| E9 | Film grain, vignette, chromatic aberration, lens distortion | UE [S9]; Bevy 0.19 vignette and lens distortion [S7] | **PLANNED** | E-LUT covers grain, CA and vignette; **lens distortion is in no plan** | all, after E1 | P3 | E1 | S (inside the tail pass) |
| E10 | Game-authored full-screen post passes | HDRP Custom Post-processing [S3]; UE Post Process Materials [S9] | **PLANNED** | [RENDER-GRAPH-API-PLAN.md](RENDER-GRAPH-API-PLAN.md) is CONVERGED; its dogfood client G5 is a user-shaped "chain post-effect (vignette/tonemap-class)". `ExtensionPoint` and `TransientImagePool` are plan-only | D, VB | P2 | E1 | M |
| E11 | Spatial AA: FXAA, SMAA | Godot FXAA; HDRP SMAA [S2][S3] | **PRESENT** | `AaMode::{Fxaa, Smaa, Ssaa}` (`crates/boyko_render/src/aa_config.rs`) | D, VB | P1 | — | — |
| E12 | TAA | Karis 2014 [S23] | **PARTIAL** | `TaaConfig` (`crates/boyko_render/src/taa_config.rs`): 8-tap Halton(2,3), variance clip, RGBA16F history. It runs on 8-bit post-tonemap input; the pre-tonemap move is part of R4a. Budget: ≤ 0.5 ms at 1080p on the reference GPU, derived as 0.30–0.45 ms ([TAA-PLAN.md](TAA-PLAN.md), "Target metrics") | D, VB | P1 | E1 | inside E1 |
| E13 | Per-object motion vectors and previous-frame depth | CoD:AW motion blur and every TAA use them [S22][S23] | **PARTIAL** | see the E13 note after this table | mesh MRT on all raster paths (`hwrt` only today); SDF pixels: `hwrt` only | P1 | — | M + the SDF fork (§5.3) |
| E14 | Temporal upscaling (TAAU / FSR2-class) + dynamic resolution | FSR 2 at 4K Quality: < 1.3 ms on an RX 6800 XT, < 2.4 ms on an RX 5700 XT (**vendor claim**) [S24]; HDRP lists DLSS, FSR and TAA upscaling [S3] | **PLANNED** | plan item C-TSR [L] in [OPTIMIZATION-PLAN-RENDER.md](OPTIMIZATION-PLAN-RENDER.md) Track C (the FSR2/TSR recipe); §4 of [RENDER-AA-AND-TAILS-PLAN.md](RENDER-AA-AND-TAILS-PLAN.md) defers it | D, VB | P1 | E12, E13, D23 | L |
| E15 | MSAA | Godot, HDRP [S2][S3] | **ABSENT** | `rasterization_samples` is hard-coded to 1. MSAA cannot reach an SDF pixel, because the marcher and the resolve are single-sample compute (post-FX survey T20) | raster only | P3 | — | M |
| E16 | Sharpening | — | **PRESENT** | RCAS, `SharpenMode::Rcas`; it runs only after TAA | D, VB | P2 | — | — |
| E17 | AA on the Forward and Forward+ paths | — | **ABSENT** | `post_process_aa_supported` and `taa_supported` are true only for Deferred and VB; TAA on Forward degrades with `RenderPathDegrade::ForwardTaaNotYetImplemented`. E1 builds the tonemap seam; E17 adds AA on it | F, F+ | P1 if Forward ships | E1 | S |
| E18 | Specular (shading) AA: NDF filtering from normal derivatives | Kaplanyan, Hill, Patney, Lefohn, HPG 2016 [S51]; Tokuyoshi, Kaplanyan, I3D 2019 [S52] | **ABSENT** | `rg -i 'toksvig\|specular.?aa\|geometric.?specular\|roughness.?limit\|kaplanyan'` over `crates/` finds nothing. TAA only partly masks this shimmer, and FXAA and SMAA do not touch it | D and F (raster quad derivatives); VB (analytic gradients, as `vb_shade` already derives for `SampleGrad`); **SDF: no derivative source** (§5.3) | P1 | — | S (mesh leg) |
| E19 | Analytic SDF edge AA (C-AA) | — | **PLANNED** | plan item C-AA, **designed and parked by the owner** in 2026-06 ([RENDER-C-AA-DESIGN-PARKED.md](RENDER-C-AA-DESIGN-PARKED.md)). The workable design costs 4 frozen-field taps per hit pixel; the owner chose zero new field reads. Its stated unblockers are a 2-D marcher dispatch, a neighbour-aware resolve, or C-TSR (E14) | SDF legs | P2 | E14 (one of the unblockers) | S |

**E13 evidence.** Today's reprojection is camera-only: `gViewT` + `MotionCamState`
(`crates/boyko_render/src/motion_cam.rs`). Motion-vector producers exist only under `hwrt`: a
mesh RG16F MRT and the SDF-pixel VIS-MV pass.

**`MvSource::PerObject` was investigated, costed and declined at rung D2.** Its doc in
`taa_config.rs` gives four reasons:

- the moving-object evaluation measured **no ghosting**, and the owner confirmed it by eye;
- **no software SDF-pixel producer exists**, so without `hwrt` SDF pixels would get Δuv ≡ 0;
- wiring it would force the RT pipeline on whenever AA is armed;
- `upload_prev_instance_models` would first need a real refactor.

`DisocclusionTest::OffScreenAndDepth` is declared and inert.

**How E13 is justified now.** Its drivers are the **consumers that read a motion vector at every
pixel**: motion blur (E8), the temporal upscaler (E14) and lit particles' `MOTION` (G3). TAA
quality is not among them, because the measurement refuted it. Two rules follow:

- E13 lands with its first consumer (Wave 3), not in Wave 1.
- No MV-at-every-pixel consumer is armed on a non-`hwrt` build until one of two things holds:
  either an SDF-pixel producer exists without `hwrt` (§5.3), or the consumer binds the G-buffer
  mask and falls back to camera-only on SDF pixels.

**Ordering inside E:**

1. **E1**, with E12's pre-tonemap move and G10.
2. **E2, E4 and E5** inside the tail pass, and **E18** (specular AA, mesh leg). Specular AA is
   the only remedy for a class of aliasing no current AA mode fixes, and its mesh leg costs ALU
   in a pass that is bandwidth-bound anyway (§3.5).
3. **E13 with E14**: per-object and SDF motion vectors, then the upscaler.
4. **E7, E8, E6 and E9** are polish.
5. **E19** waits for its prerequisites, one of which is E14.

### 3.3 Sun rays and volumetrics (F)

| ID | Capability | Ref | Status | Evidence | Paths | P | Deps | Effort |
|---|---|---|---|---|---|---|---|---|
| F1 | Analytic height and distance fog (with sun scattering) | Godot exponential depth/height fog with sun scattering [S2]; UE Exponential Height Fog [S10]; Quilez's closed form (volumetrics survey T12) | **ABSENT** | no fog of any kind in the code (`rg -i 'fog\|volumetric\|god ray\|light shaft\|participating medi'`); Track E has no analytic-fog item | all, in E1's tail pass, from each path's depth proxy | P1 | E1 | S |
| F2 | Froxel volumetric fog and lighting: lights and shadows scatter | Wronski, AC4 [S18]; Hillaire, Frostbite [S17]; UE Volumetric Fog [S10]; Godot volumetric fog [S2]; HDRP fog [S3] | **PLANNED** | E-FOG (160×90×64) in [OPTIMIZATION-PLAN-RENDER.md](OPTIMIZATION-PLAN-RENDER.md); two defects below. The volumetrics survey §6 finds P6-S "not a live prerequisite" | **VB only** has a froxel light list. D and F+ bind placeholders, and F has none. Three ways out: walk all lights (≤ `MAX_LIGHTS` = 1024); arm the cull per path, which the transparency design D8 priced and declined; or use a fog-owned light table culled by a second instance of the cluster-cull kernel, as the volumetrics design (pass 1) proposes, which makes F2 path-independent | P1 | E1, F1; a 3D **storage** image (§5.2) | L (+ M per path if the light list is armed per path) |
| F3 | Local fog volumes (box, sphere, 3D texture) | UE Local Fog Volumes [S10]; Godot FogVolume [S2] | **PLANNED** | E-DENS-A: "local fog volumes as `SdfEdit`-like ECS components". The volumetrics survey §6 notes the brick atlas it names covers only [-4, 4]³ | as F2 | P2 | F2 | S–M |
| F4 | Sun shafts / god rays | from fog self-shadowing [S17][S18]; screen-space radial blur (Mitchell, GPU Gems 3 ch. 13) [S21] | **PLANNED** | E-GOD. Route (a) derives rays from volumetric self-shadowing, and is "FREE" only for SDF casters: mesh casters reach fog only through CSM or the atlas (volumetrics survey §6). Route (b), screen-space, is optional | route (a) as F2; route (b) all, after E1 | P1 | F2 (route a) or E1 (route b) | S |
| F5 | Physically based sky and aerial perspective | Hillaire EGSR 2020 [S19]; UE Sky Atmosphere [S10]; HDRP Physically Based Sky [S3] | **ABSENT** | a two-colour gradient + sun disc at three sites (`forward_sky.fs.hlsl`, `vb_sky`, the background branch of `deferred_pbr.hlsl`); Track E has no sky item (volumetrics survey §6) | all (three sites) | P1 | E1 | M |
| F6 | Volumetric clouds | Nubis: ~2 ms and 20 MB in Horizon Zero Dawn; the naive march cost ~20 ms; a quarter-res buffer updates 1 of 16 pixels per 4×4 block per frame, with reprojection [S20]; HDRP Volumetric Clouds [S3] | **PLANNED** | E-CLOUD [XL] | all | P2 | F2, F5 | L–XL |
| F7 | SDF-native volumetric self-shadow and AO; brick-atlas density | — | **PLANNED** | E-SDFVOL, E-DENS-A/B. The ">16 edits" trigger for P9 is unreachable while the authority is capped at 16 (D26) | as F2 | P2 | F2, D26 | M–L |
| F8 | Sparse volumes (VDB smoke and fire) | UE Heterogeneous Volumes [S10] | **ABSENT** | — | as F2 | P3 | F2 | L |
| F9 | Particles inject density into fog; particles lit by the fog volume | Hillaire voxelizes particles into the extinction volume [S17]; Wronski reuses the volume to light particles and translucents [S18] | **PLANNED** | E-PART; particle design D11 (P3 lit particles through a froxel lookup) ([PARTICLES-PLAN.md](PARTICLES-PLAN.md)) | as F2 | P2 | F2, G3 | M |
| F10 | Time of day | — | **PARTIAL** | the sun is an ordinary `DirectionalLight`; there is no time-of-day system, and the gradient sky does not respond to it | all | P2 | F5 | S |

**Published numbers that bound F2 and F6:**

- **F2, Wronski.** Assassin's Creed IV's froxel fog cost "around 1.1ms" in total on the talk's
  console target. Density and lighting were about 0.43 ms of that, and double resolution cost
  1.6 ms [S18].
- **F2, normalized.** The volumetrics survey §4.2–4.3 normalizes AC4, Frostbite and UE to
  0.42–1.07 ns per froxel on the reference GPU [E]. That gives:
  - **0.39–0.99 ms** to inject and integrate the 160×90×64 grid;
  - 29.5 MB for four RGBA16F volumes;
  - an apply pass of 0.07–0.10 / 0.13–0.18 / 0.30–0.40 ms at 1080p / 1440p / 4K.
- **F6, Nubis.** About 2 ms and 20 MB [S20]. The volumetrics survey scales a 2.5D variant to
  0.6–0.95 ms at 1440p [E]. Any full-3D figure for the reference GPU is an estimate at best; the
  campaign must measure it.

**Two defects in the E-FOG plan row.** Both are recorded here and not fixed, because this pass
edits only this file. The volumetrics survey §6 (items 1 and 3) found both independently.

1. **The memory figure does not match its own arithmetic.** The plan's resource table gives
   "E-FOG 160×90×64 ×RGBA16 ×2 … ~30 MB".
   - The grid holds 921,600 froxels.
   - One RGBA16F volume is 921,600 × 8 B = **7.37 MB**, so two volumes are **14.7 MB**.
   - ~30 MB is four volumes (29.5 MB). That is the chain the volumetrics survey counts: a scatter
     ping-pong that doubles as the history, and one integrated volume per frame in flight.
2. **The grid constraint contradicts the shipped cluster grid.** The plan says "the froxel XY×Z IS
   the cluster grid". The shipped grid is `CLUSTER_DIM_X/Y/Z = 16×9×24`
   (`crates/boyko_render/src/light.rs`). It exists on VB only, and its dimensions are packed at 8
   bits per axis. 160×90×64 is not that grid, and 64 is not a multiple of 24. The fix is a
   performance decision for the volumetrics design: either fog froxels look up their parent
   cluster's light list, or the cluster grid is refined.

**The hybrid fork F2 must settle (owner rule 2):**

- **Classical:** shadow each froxel's in-scatter from the existing CSM and atlas depth, using a
  downsampled or exponential shadow map, as Wronski did [S18].
- **SDF-native:** cone-march the field toward each light (E-SDFVOL). The plan warns that at about
  920K froxels this is "potentially billions of FROZEN evals/frame" on the analytic path.

The SDF route is gated on **D26**, not on a missing atlas:

- the brick atlas exists, but as a 40³ near-field image that the windowed host binds empty;
- the edit authority is capped at 16.

**The measurement** is F2 at 1080p, 1440p and 4K on the reference GPU, over both routes, in a
scene with meshes, SDF and ≥ 64 lights. It runs **on VB**, the only path with a froxel light
list today. It can run on any path if the fog-owned light table the volumetrics design proposes
is adopted. The faster route becomes the default for this hybrid scene, and the other stays a
capability. A verdict taken before D26 lands was taken on a ≤ 16-edit fixture, and is
provisional.

### 3.4 The effects system: explosions and rain (G)

| ID | Capability | Ref | Status | Evidence | Paths | P | Deps | Effort |
|---|---|---|---|---|---|---|---|---|
| G1 | GPU particles: emit, simulate, sort, draw, SDF collision | Niagara GPU emitters [S14]; Godot GPU particles [S2] | **PRESENT** | `ParticlePlugin`, `ParticleEffect`, `ParticleMode::GpuUnlit`; P0, P1 and P2 items 1–3 landed ([PARTICLES-PLAN.md](PARTICLES-PLAN.md)). Sim ≈ 1.0–1.1 ns per particle on the reference GPU, flagged SUSPECT by the plan. **Known visible gap:** a fixed 64 Hz step with no render-time interpolation, so particles move at 64 Hz against a 144–200 Hz camera (the plan's M6 trade-off; G15) | all (`declare_particle_draw` on all three declarators) | P0 | — | — |
| G2 | Soft particles (depth fade) | Godot proximity fade [S2] | **PLANNED** | P2 item 4, designed (`-D SOFT`, not yet built per [SHADER-VARIANT-MANIFEST.md](SHADER-VARIANT-MANIFEST.md)) | all, with a per-path depth state (plan D7) | P0 | — | S |
| G3 | Lit particles and particle motion vectors | Wronski reuses the fog volume to light particles [S18] | **PLANNED** | P3 (`LIT_PERPIXEL`, `MOTION`) | all | P1 | E13, F2 (optional) | M |
| G4 | Flipbook animation (sprite sheets, motion-vector frame blending) | Niagara modules [S14]; motion-vector flipbooks, > 10× frame extension (VFX survey §8) | **ABSENT** | one bindless texture per effect (`tex_index`) | all | P0 | — | S |
| G5 | Colour and size over life | Niagara [S14] | **PARTIAL** | a size ramp and 4 colour keys; only key 0 is rendered | all | P1 | — | S |
| G6 | Sub-emitters and particle events (spawn on death or collision) | Godot sub-emitters [S2]; Niagara events [S14] | **ABSENT** | — | all | P1 | G1 | M |
| G7 | Trails, ribbons, mesh particles | Godot ribbon and tube trails [S2] | **PLANNED** | P4 | all | P1 | G1 | L |
| G8 | Forces: vector fields, curl noise, attractors | Godot 3D attractors [S2] | **ABSENT** | — | all | P2 | G1, G16 | S–M |
| G9 | Depth-buffer collision (mesh-only scenes, rain splashes) | Godot particle collision [S2] | **ABSENT** | SDF collision only (`ParticleCollision::Sdf`). The particle research's fact R6 says "SDF collision beats depth-buffer collision", but the mesh leg has no SDF a particle can query (D26: the MDF path is off in the host). A hybrid fork (§5.3) | per path's depth proxy | P1 | G1 | S–M |
| G10 | HDR compositing of additive and blended particles | — | **ABSENT** | particles draw into the 8-bit `lit` (§3.1); lands with E1 | all | P0 | E1 | S |
| G11 | Distortion / heat haze / refraction | HDRP Screen Space Distortion [S3] | **ABSENT** | — | D, VB; all after E1 | P2 | E1, D8 | S–M |
| G12 | VFX authoring: module stacks, simulation stages | Niagara modules and simulation stages [S14] | **ABSENT** | effects are `ParticleEffect` assets only | — | P2 | N2 | L |
| G13 | Explosion as a composite | Niagara systems combine emitters [S14] | **ABSENT** | parts missing: G4, G6, G10, D9, G11, K3 shake, J8, I3 radial impulse | all | P1 | the listed rows | S once the parts exist |
| G14 | Weather: rain and snow, occlusion under roofs, splashes, wet surfaces, puddles | Lagarde's dynamic rain and wet-surface series for *Remember Me* [S26] | **ABSENT** | — | all | P2 | G1, G9, D6, D9, G16 | M–L |
| G15 | Particle render-time interpolation | — | **PLANNED** | rung P2b in [PARTICLES-PLAN.md](PARTICLES-PLAN.md): a compile-time `-D PARTICLE_INTERP` variant (the record grows 32 → 40 B, +25 % draw-read traffic, only when on). Its manifest row is listed as "not yet built" | all | P1 | G1 | S |
| G16 | Global wind field (shared by rain, foliage, cloth, clouds) | VFX survey §7.5 | **ABSENT** | `rg -i '\bwind\b'` over `crates/` finds only "unwind" | all | P2 | — | S–M |

**An explosion is not an engine feature; it is a composition test.** A shipped explosion is made
of ten parts:

- a burst emitter (G1);
- a flipbook fireball (G4). Shipped explosions are motion-vector flipbooks with baked lighting,
  not simulations (VFX survey §0 item 2);
- smoke that lingers, soft (G2) and ideally lit (G3);
- sub-emitted debris and sparks (G6) with collision (G9, G1);
- a brief HDR light flash: a point light with a lifetime, composited in HDR (D10 + E1);
- a scorch decal (D9), or a crater (an SDF subtractive edit, §5.3);
- a shockwave distortion (G11);
- camera shake (K3);
- sound (J8);
- a radial impulse on nearby bodies, through an overlap query (I3).

Seven of those ten parts are absent. So the "explosions" topic resolves to finishing G and its
neighbours, then building one explosion prefab as a dogfood scene whose golden pins the
composition.

**Rain has the same shape.** It needs:

- **Camera-local drops.** They need no per-drop memory: NVIDIA's 2007 sample re-spawns drops that
  leave a camera-following box (VFX survey §0 item 4).
- **A way to stop rain under roofs.** Either a top-down "rain occlusion" depth map (Remember Me:
  256² over 20 m, 0.32 ms on PS3 [S26]), or an SDF query from above (the hybrid fork).
- **Splashes on collision** (G9, or SDF).
- **A wetness response on surfaces:** darker albedo, lower roughness, puddle masks [S26]. This
  needs material parameters driven by a global weather state, which is D6: `ViewUniform` carries
  no time, and materials have one flag.
- **A wind field** (G16), shared with foliage and cloth.

**ECS shape (owner rule 3) for everything in §3.**

- **Post effects** follow the existing `XConfig` → `ResolvedX` → cold-policy pattern. The
  capability is an enum with an `Off` variant. `Off` means the pass is not declared in the frame
  graph and its images are not allocated. The graph is re-declared every frame with zero
  allocation, so a pass gated at declare time costs nothing on frames where it is off.
- **Fog volumes, decals and emitters** are per-entity **components**, so an entity without
  `FogVolume` is never iterated ([CAPABILITY-STATE-MODEL.md](CAPABILITY-STATE-MODEL.md), axis 1).
  A runtime on/off flag is an `EnableTag` bit on entities that carry the capability.
- **Global weather and wind** are `Resource`s read by the material and particle stages. They are
  never a side store.

### 3.5 The candidate techniques and what they cost on the reference GPU

Every row names its kind. [P] is published (rig named). [M] is a third party's measurement. [V]
is a vendor claim. [E] is an estimate or derivation, with the working shown or referenced. The
surveys hold the full arguments; this table is what the order in §4 leans on. Unless noted, the
costs are at 1080p / 1440p / 4K on the reference GPU.

| Topic | Candidate | Cost | Kind and source |
|---|---|---|---|
| Tail pass (tonemap, exposure apply, bloom composite, LUT, vignette, grain) | one fused compute pass (§3.1) | 0.075 / 0.13 / 0.30 ms on the probed arm; 0.11 / 0.19 / 0.43 ms on the fallback arm | [E] §3.1 |
| Auto-exposure | 256-bin log-luma histogram with adaptation (E-EXP; Tardif) | 0.17 ms at full 1080p, against 0.095 ms for a plain reduction, on an RTX 2080. Building it on a half-resolution input cuts the atomic volume 4× | [M] post-FX survey T4; not scaled to the reference GPU |
| Bloom | CoD:AW 13-tap down + tent up, Karis average on the first downsample, 5–6 mips, R11G11B10 (E-BLOOM, survey T6) | about 9.7 B/px of traffic over the chain (down-chain reads 4·(1 + ¼ + …) ≈ 5.3 and writes ≈ 1.3; up-chain ≈ 3.0), so **0.08 / 0.14 / 0.32 ms** floor. No published ms | [E] |
| Spatial AA | FXAA / SMAA 1x (shipped) | 0.11 / 0.30 ms at 1080p and 0.37 / 0.98 ms at 4K on a GTX 1080 (Intel's CMAA2 table) | [M] post-FX survey T19 |
| TAA | shipped; to move pre-tonemap | ≤ 0.5 ms budget at 1080p; derived as 0.30–0.45 ms; +49.8 MB VRAM | [E] [TAA-PLAN.md](TAA-PLAN.md) |
| Temporal upscaler | C-TSR (the FSR2 recipe, in-house) | FSR 2.2.1 on an RX 6650 XT, the survey's nearest proxy: 1.2 ms at 1440p Quality, 2.8 ms at 4K Quality; 207 MB working memory at 1440p Quality | [V] post-FX survey T22 |
| Specular AA, mesh leg | NDF filtering from quad or analytic normal derivatives (E18) | ALU in a bandwidth-dominated pass. The authors state that the filter's arithmetic is typically not the bottleneck of a G-buffer pass [S52]. No ms | [P] |
| SDF edge AA | C-AA (E19, parked) | 4 frozen-field taps per SDF hit pixel | [T] [RENDER-C-AA-DESIGN-PARKED.md](RENDER-C-AA-DESIGN-PARKED.md) |
| Height fog | analytic closed form, in the tail pass (F1) | reading a 4 B/px depth proxy adds 0.03 / 0.06 / 0.13 ms; the ALU is a few operations | [E] |
| Froxel fog | E-FOG at 160×90×64 (F2) | inject and integrate 0.39–0.99 ms, flat in resolution; apply 0.07–0.10 / 0.13–0.18 / 0.30–0.40 ms; 29.5 MB | [E] volumetrics survey §4.3 |
| Sky and aerial perspective | Hillaire 2020 LUTs (F5) | ≈ 0.2 ms, taken from 0.17 ms on a GTX 1080 and used unscaled | [E] volumetrics survey §4.4 |
| Sun shafts, screen-space | radial blur at half resolution, sun only (F4 route b) | 0.2–0.4 ms at 1440p | [E] volumetrics survey §4.4 |
| Sun shafts, per-pixel march | shadow-map march per pixel | 1.4–2.8 ms per light at 1440p (the texel rate was not fetched) | [E] volumetrics survey §4.4 |
| Clouds | 2.5D, Nubis-class (F6) | 0.6–0.95 ms at 1440p; 8.5 MB of noise | [E] volumetrics survey §4.4 |
| Blended particles | one full screen of blended coverage | 0.049–0.137 / 0.088–0.244 / 0.198–0.549 ms at 4 B/px (bandwidth-bound: a 3 MB L2 against an 8.3 MB target). R4a's fallback arm doubles it | [E] VFX survey §2 |
| Rain | layers + an occlusion map + ripples (Remember Me) | the whole system ≈ 2.8 ms on PS3 at 720p; occlusion map 0.32 ms | [P] VFX survey §8 |

**What the table implies for the order.** The whole first image-quality set is inexpensive:

- the tail pass;
- the exposure histogram;
- bloom;
- height fog;
- the sky LUTs;
- mesh-leg specular AA.

It costs about **0.6–0.8 ms at 1440p** in total [E]: tail 0.13 + histogram ≤ 0.17 + bloom
0.14 + fog 0.06 + sky 0.2. That is why Wave 2 can carry it beside the P0 gameplay set. Froxel fog
(about 0.5–1.2 ms at 1440p, apply included), a temporal upscaler (≥ 1.2 ms class) and clouds are
the items that need a budget decision. They land in Wave 3 or later.

### 3.6 Path coverage — what the per-path differences cost

| Fact | Consequence | Where it is carried |
|---|---|---|
| F and F+ record no post or AA seam | E1 builds the tail seam on F and F+ (+S); E17 adds AA on it | E1's effort; row E17 |
| After E1, every effect fused into the tail pass reaches all four paths | E2, E4, E9, F1 and the bloom composite are all-path for free. The bloom pyramid and TAA are separate passes, declared per path | E-row Paths cells |
| The froxel light list is VB-only | F2 and F9 have a light list only on VB. Every other path must do one of three things: walk all lights (≤ 1024); arm the cull (+M per path); or take the fog-owned light table of the volumetrics design, which is path-independent | F2's effort; §3.3's measurement runs on VB until that choice is made |
| The motion-vector producers are `hwrt`-only | E8, E14 and G3 cannot read a motion vector at every pixel on non-`hwrt` builds (§5.3) | E13 |
| The SDF marcher is a 1-D dispatch with no `ddx`/`ddy` | specular AA (E18) and C-AA (E19) have no derivative source on SDF pixels | §5.3 |
| The particle draw exists on all four paths | G rows are all-path, with the per-path depth state the particle plan's D7 already specifies | G Paths cells |

---

## 4. Recommended order of campaigns

### 4.1 Wave 0 — the work in flight, and where F4 goes

**Wave 0** is not re-ordered. It contains:

- **The unified plan** ([UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md](unification/UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md) §4):
  - Phase C (the memory crate);
  - Phase D (the kernel contract);
  - Phase E (subsystems: physics U4..S0; the engine lanes IN/HO/RE/AS/UI);
  - **Phase F rungs F1–F3 only.**
- **The physics performance levers after L9.** L9 C4's merge is the trunk head `6394bc5e`
  ("merge: u/phys-l9-c4 … into integ/unified", per `git log -1`).
- **SI1**, the spatial index as a kernel feature (Phase E).

**Why Wave 0 stays first.** Phase D changes the storage, asset and host contracts, and Phase E
rungs lock the files a subsystem campaign would otherwise edit:

- `boyko_app/src/runner.rs` on the HO* chain;
- the asset loader, server and staging files on AS2..AS5;
- `boyko_render/src/{mesh_draw, particle_system, particle, light_system}.rs` on RE*.

([UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md](unification/UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md) §4.3.)
A subsystem built against the pre-D contract gets ported twice. That is the principle-0 failure
the `std::Vec` physics mirror produced (CLAUDE.md, principle 0).

**Where F4, the refactor campaign, goes.** Owner answer Q-4 reads: "The refactoring campaign runs
after everything else, so the plan is not reworked by it." At the time, "everything else" was the
unified plan. The ruling's own reason carries over to the feature waves unchanged:

- F4's census files are exactly the files Waves 1–5 edit. The device waves RF-V, RF-R and RF-A
  cover `targets.rs`, `graph_bridge.rs`, `passes/{gbuffer,vb}.rs`, `light.rs`,
  `render_path_config.rs` and `gpu_scene/mod.rs`; RF-P covers the physics solver; RF-U covers
  `boyko_ui/src/layout.rs`.
- Those device waves already wait for **owner step O4** to declare the render and RHI files
  quiet. No render feature wave can run while that holds.
- F4's own first step re-derives the census on the trunk of the day. Running it later costs no
  rework, while running it first would put every owner topic behind the largest campaign in the
  plan.

**Recommendation:** F4 runs **after Wave 5**, the last wave before the first ship. F4's kernel
waves (RF-0, RF-K1..K3, RF-L) could run earlier, **after Wave 2**: Gaia's kernel requests (GK-1,
GK-3, GK-4) land by then. GK-4 adds derive-emitted field tables to `boyko_macros`, whose
`component.rs` is an RF-K2 file, and GK-1 and GK-3 change the load path in the kernel. So the
kernel waves must not run during Waves 1–2. This extends an owner ruling, so §7 Q7 asks the
owner to confirm it.

**The counter-argument, stated.** Waves 1–5 will edit files that are already very long
(`graph_bridge.rs` and `targets.rs` run to thousands of lines). Features built on them cost more
per change than they would after F4. The owner weighed exactly this trade when answering Q-4, and
chose "after".

**What may run beside Wave 0.** Work in files outside every Phase D/E lock set can run beside it:

- **New crates**: audio, navigation, the tools.
- **Shaders.**
- **The render and RHI files E1 edits**: `present/*`, `targets.rs`, `graph_bridge.rs`,
  `render_path_config.rs` and `light.rs`. The RE* rungs lock `light_system.rs`, which is a
  different file.
  - The §4.3 lock table names only `goldens/PINS.toml` among render files, assigned to rung AH of
    Phase A, which is closed.
  - The RE* rungs lock `mesh_draw`, `particle_system`, `particle` and `light_system`.
  - These are the owner's files (step O4), so a render lane runs in the owner's domain.
- **RHI additions.**

Each cut is checked against the §4.3 shared-file table first. **With F4 out of Wave 0, nothing in
Wave 0 asks the render files to be quiet**, so revision 1's contradiction between "may run beside
it" and O4 no longer exists.

### 4.2 Waves 1–6

Each wave is a set of lanes on disjoint files. The lane count is bounded by the machine: three
build lanes run today, and lanes beyond three queue. Render lanes share the frame-graph
declarators (`graph_bridge.rs`, `targets.rs`), and they serialize their edits there in the order
the table lists them.

**Wave 1 — foundations.**

| Lane | Items | Rows | Effort |
|---|---|---|---|
| Render | R4a as designed: HDR `lit` behind RK-14, `display`, one tail `tonemap` pass (including the new F/F+ post seam), eight transition pins, TAA pre-tonemap, HDR particle compositing | E1, E12, G10 | M + S |
| Platform | Window modes and DPI; gamepad behind the `BindSpec::Stick` seam → gamepad UI navigation; thread priorities and MMCSS | A2, A6, L5, A10 | S + S–M + S + S |
| Audio | Event-driven WASAPI backend on an MMCSS thread; mixer with a fixed voice pool; WAV import, then the Vorbis decoder and streaming music; the ECS surface | J1, J2, C15, J3, J8 | M + M + M + S |
| Tools | Debug draw (line topology in the RHI, an immediate-mode ECS buffer, zero cost without the plugin), physics debug view; console and cvars over `boyko_reflect`; native crash minidumps | D18, I12, O6, O7 | S + S + S–M + M |
| Content | The Gaia bake (the cook) and the `data` profile, per Gaia's own work order, after the owner approves its pending syntax plan | C10, N2 | L |

**Reason.**

- Every item in the owner's topics composes in linear HDR (§3.1). Doing E1 first means bloom, fog
  and particles are written once, and the 61 legs re-bless once rather than per effect.
- The mixer ships together with a decoder, since a mixer without one plays nothing.
- Gamepad and window modes are small P0 items in `boyko_input` and `window.rs`, with no render
  dependency.
- Debug draw and cvars speed up every later campaign.
- The cook is the prerequisite of Waves 2 and 5, so it starts here.

**Wave 2 — the P0 gameplay set, plus the first image-quality set.** Only the runtime and content
lanes and three render rows (D7, G2, G4) are P0 ship-blockers. The image lane is P1: it is
visible quality, not a ship-blocker. It is placed here because it is cheap (about 0.6–0.8 ms at
1440p in total, §3.5), because it is in files the gameplay lanes do not touch, and because the
owner asked for it (§4.3).

| Lane | Items | Rows | Effort |
|---|---|---|---|
| Render: image | auto-exposure, bloom and LUT inside the tail pass; specular AA (mesh leg); analytic height fog; the physically based sky with aerial perspective | E2, E4, E5, E18, F1, F5 | S–M each |
| Render: content | alpha-tested cutout; soft particles; flipbooks; colour over life | D7, G2, G4, G5 | S–M each |
| SDF scale | the SDF field at game scale: lift the 16-edit authority cap (it is shared byte-for-byte with physics, so this is coordinated with the physics lane), arm the brick backend and clipmap in the host, arm the mesh distance field | D26 | L |
| Runtime | animation rung 1 → GPU skinning → root motion; glTF skins; the physics gameplay set (wire layer and mask, queries, capsule, trigger and contact producers, kinematic motion) → character controller → gameplay cameras | H1–H5, D16, C1, I2–I6, I9, K3 | L + M–L + M |
| Audio | 3D spatialization | J4 | S–M |
| Content | offline BC texture cook with mips; async IO; persisted pipeline cache; the Gaia `scene` profile (G6): stable asset names, GK-1 + GK-3's load fixup, and cell streaming as ballot F5 ruled | C5, C6, A15, C9, C11, R3, B4, C14 | M + M + S + L |

**Reason.**

- The runtime and content lanes hold the P0 rows the reference game cannot ship without.
- The runtime lane coordinates with the Phase E physics rungs (U4..U7), which lock the physics
  touch set. The physics gameplay set may have to follow U7 in the same worktree, which the
  owner's physics campaign order already implies.
- D26 is here because every hybrid measurement in Waves 3–4 depends on it (§5.3).

**Wave 3 — world feel.** This is where the owner's topics land in full.

| Lane | Items | Rows | Effort |
|---|---|---|---|
| Render: volumetrics | froxel fog with the classical-vs-SDF measurement on VB (§3.3); god rays; local fog volumes; particle fog injection | F2, F3, F4, F9 | L + S + S–M + M |
| Render: VFX and materials | the global time uniform and per-instance material parameters; sub-emitters and events; lit particles; trails; forces; depth collision; render-time interpolation; the wind field; decals; explosion and rain dogfood scenes | D6, G3, G6, G7, G8, G9, G15, G16, D9, G13, G14 | L + L |
| Render: image and surfaces | motion vectors on every pixel, including the non-`hwrt` SDF producer (§5.3); the temporal upscaler and dynamic resolution; transparency R0a–R6; reflections R0–R2 (environment and probes) | E13, E14, D23, D8 (R0a–R6), D14 (R0–R2) | M + L + M + L + M |
| Runtime | DSP and music; IK; morph targets | J5, J7, H6, H7 | M each |
| Gameplay and AI | navmesh, pathfinding and crowds; perception on SI1; timers; save-game completion; settings with restart-to-apply | S1–S5, R4, R5, R6 | L + M |

**Reason.** Every render item depends on Wave 1 (HDR) and on inputs that already exist or land
in Wave 2: the VB froxel grid, CSM, the SDF field at scale, and the particle core. AI depends on
SI1 (Wave 0) and on the queries (Wave 2). Transparency and reflections start here because they
are P0 and their early rungs need E1 only.

**Wave 4 — breadth.**

| Lane | Items | Rows |
|---|---|---|
| Render | the rest of the reflections ladder (SSR, stochastic, SDF misses, hardware); the rest of transparency (OIT, R7–R10); terrain (heightfield vs SDF by measurement); foliage; water; volumetric clouds; area lights; DOF, motion blur, lens effects, film effects; Forward AA | D14, D8, P1, P2, P3, F6, D11, E7, E8, E6, E9, E17 |
| Runtime | joints, CCD, ragdolls, vehicles | I7, I8, H10, I10 |
| Tools and text | editor v1 with inspector, gizmos and undo; localization and complex shaping; the accessibility baseline; splines | O1–O4, L3, L4, L6, Q4 |

**Reason.** These are P1–P2 and large, and each rests on Waves 1–3:

- reflections and transparency need E1 and their own early rungs;
- foliage needs D7 and G16;
- the editor needs debug draw, cvars and reflection.

**Wave 5 — scale and ship.**

- Virtual-geometry LOD (D4) and texture streaming (C18).
- VFS and pack files (A13, A14).
- Release packaging and a symbol store (T2).
- The Linux platform layer (A8).
- A Steamworks plugin, as an optional crate (T1).
- A telemetry uploader (O8).
- The sequencer and video playback (Q1, C16).

**Reason.** These are needed at content scale and at release. They depend on the cook (C10,
Wave 1), async IO (A15, Wave 2) and the VRAM budget (C17). **The rest of F4** follows Wave 5
(§4.1, §7 Q7).

**Wave 6 — genre and optional.**

- Networking (M1–M5).
- Destruction (I11) and motion matching (H11).
- XR (U1).
- HDR display output (D24) and MSAA (E15).
- Runtime scripting (N3) and modding stage 3 (N4).
- Large-world coordinates (P4) and procedural content (P5).
- C-AA (E19), once one of its prerequisites exists.

**Reason.** These are P2–P3 for the reference game. **If the owner's game is multiplayer,
networking moves to Wave 2.** Prediction and rollback shape how gameplay systems are written:
Overwatch built its netcode into the ECS architecture [S31]. The determinism work already done
is the asset that makes rollback cheap here [S32].

### 4.3 Departures from standing rulings, argued

| Standing ruling | This audit's order | Argument |
|---|---|---|
| **Render plan M1:** the generic post chain is "polish-tier", "sequenced last, never competing with the differentiators"; the SDF-native fast track goes first ([OPTIMIZATION-PLAN-RENDER.md](OPTIMIZATION-PLAN-RENDER.md)) | exposure, bloom and LUT land in Wave 2; DOF, motion blur and lens effects stay polish (Wave 4) | see the M1 argument after this table |
| **Owner answer Q-4:** the refactor runs after everything else | F4 after Wave 5 (§4.1) | this applies Q-4's own reason to feature waves the ruling did not see; the owner confirms (§7 Q7) |
| **Transparency ballot 1 (2026-09-10):** HDR "after R6" | E1 first in the render queue (Wave 1) | the ballot is now joint with reflections R4a; §7 Q4 argues it |

**The M1 argument.** M1's condition has been met:

- **The fast track has landed or been parked.** A1 (SDF soft shadows), A2 (SDF 5-tap AO) and B1
  have shipped (`RENDER-C-AA-DESIGN-PARKED.md` header; `sdf_ao` in `vb_geo.comp.hlsl`). C-AA was
  parked by the owner, and B7 by measurement ([RENDER-B7-DESIGN-PARKED.md](RENDER-B7-DESIGN-PARKED.md)).
  B5's state was not verified in this pass.
- **The owner raised the post chain's priority on 2026-09-25** by asking for this research.
- **The cost is small** (§3.5), and it runs in files no differentiator is using.
- **E1 is not a post item under M1.** It is reflections R4a, the substrate every campaign shares.

### 4.4 P0 placement check (mechanical)

All 40 P0 rows that are not PRESENT are placed. A script cross-tabulated the §2/§3 tables
against the Rows cells of the wave tables and the Wave 5–6 lists, ignoring reason prose. A11 is
in Wave 0. Each other row is in exactly one wave, except D8 and D14: their ladders span Waves 3–4
by rung.

| Wave | P0 rows not yet PRESENT |
|---|---|
| 0 | A11 |
| 1 | A2, A6, C10, C15, D18, E1, G10, J1, J2, J3, J8, L5, N2 |
| 2 | B4, C1, C5, C11, D7, D16, G2, G4, H1, H3, H4, I2, I3, I4, I5, I6, I9, J4, K3, R3 |
| 3 | D8 (early rungs), D14 (early rungs), R5, S1, S2 |
| 4 | D8, D14 (completion) |
| 5 | T2 |

Revision 1 left seven of these rows in no wave: A2, C10, C15, J3, L5, N2 and R3. The
cross-tabulation for this revision found three more: C1, D16 and B4.

---

## 5. How the recommendations fare against the owner's rules

### 5.1 Per wave

| Wave | R1 web-sourced | R2 hybrid perf with numbers | R3 ECS-native, zero cost when off | R4 hot path | R5 in-house | R6 eDSL, manifest, RHI |
|---|---|---|---|---|---|---|
| 1 | every item cites a shipped engine or talk; the technique surveys (§3.0) carry the rest | HDR format decided by R4a D4 behind RK-14. The open number is the fallback arm's cost (§3.1). Post budget by the bandwidth floor | post stages are `Config` → `Resolved`, where `Off` = not declared; debug draw is a plugin, and absent = nothing compiled in | HDR changes no CPU loop; post passes are GPU; the audio mixer has a fixed voice pool, with no allocation after boot | all in-house; WASAPI and GameInput/XInput are OS APIs, not dependencies | tonemap and OETF move from hand-written `pbr_lighting.hlsli` into eDSL leaves (the eDSL lacks `log2`/`exp2`/`pow` today, post-FX survey §0.5); `-D LIT_FORMAT` and tail-pass rows in the manifest; the RHI adds line topology |
| 2 | yes | sky, fog and bloom priced in §3.5, then measured at 3 resolutions; queries and the character controller benched against Jolt 5.6 (owner feedback: compare only with Jolt); **D26 is the precondition of every SDF-candidate measurement** | components for colliders; queries as system params; `AnimInstance` dense (per the animation design) | pose bank SoA; queries allocation-free | the BC encoder in-house is feasible, and is the wave's largest in-house cost (§5.4) | skin-cache pass as eDSL leaves; fog and sky manifest rows |
| 3 | yes | fog classical vs SDF on VB with ≥ 64 lights (§3.3); SDF motion vectors vs `hwrt` VIS-MV; G9 depth vs SDF collision; navmesh from triangles vs from SDF bricks | `FogVolume`, `Decal` and emitters as components; `Off` builds nothing | particles already heap-free; navmesh queries on scratch columns | navmesh in-house (Recast as the zlib-licensed reference [S29]) | new particle and fog variants as manifest rows; a 3D storage image; `DrawIndirectCount` and timeline semaphores become worth adding |
| 4 | yes | terrain heightfield vs SDF; reflections per the design's own numbers | the editor is an optional plugin set, as the editor design specifies | — | text-shaping scope set by the languages the owner ships (§7 Q5) | reflection and transparency variants per their designs |
| 5–6 | yes | streaming IO measured; networking bandwidth per entity | networking as optional crates: a game that does not link them pays zero, the same rule the unified plan sets for replay (UG-15) | — | Steamworks is external by necessity (a closed platform, no in-house alternative), kept in an optional crate | — |

**Platform seam (owner rule 5 and §7 Q2).** Wave 1 names Windows APIs: WASAPI, GameInput/XInput,
MMCSS, IoRing and `MiniDumpWriteDump`. Each sits behind a platform module with a
`#[cfg(windows)]` implementation, in the shape `run_windowed`'s `cfg(not(windows))` stub already
has. The Linux leg (A8) then fills in modules rather than untangling calls.

### 5.2 RHI gaps the waves need (checked in `boyko_rhi_vulkan` at `6394bc5e`)

| Capability | Today | Needed by | Note |
|---|---|---|---|
| `vkCmdDispatchIndirect` | **present** (`cmd_dispatch_indirect`, used by particles) | G6, G7, F9 | the comments in `present/passes/vb.rs` and in `present/graph_bridge.rs` (`declare_vb_graph`) that say otherwise are stale |
| `vkCmdDrawIndexedIndirect` | present | G7, P2 | — |
| `vkCmdDrawIndirectCount` | absent | P2 foliage, G7 mesh particles, D9 decals if GPU-culled | a Vulkan 1.2 core command |
| Line topology | absent (`TriangleList` only) | D18, I12, O3 | S |
| BC texture formats in `Format` | absent | C5 | S on the RHI side; the encoder is the cost |
| Async compute queue + timeline semaphores | absent (one queue family) | F2, E5 and SSAO overlap | Wronski ran fog in parallel with the scene on async compute [S18]; the value is a measurement |
| `vkCmdPipelineBarrier2` (sync2) | absent | nothing in these waves strictly | — |
| Transient aliasing in the frame graph | absent; the public `TransientImagePool` is plan-only | the Wave 1–3 post and volume targets | FrameGraph's aliasing [S16]; many short-lived post targets make it pay |
| HDR swapchain colour space | absent | D24 | — |
| 3D images | present, **sampled only** (`TextureDimension::D3`; the brick atlas is `TRANSFER_DST \| SAMPLED`) | F2, F6 | **no 3D storage image exists yet** (post-FX survey §0.4); the froxel volumes need one, which is a usage bit plus a first device test |
| Pipeline cache | absent; no `vkCreatePipelineCache` in `ffi.rs`, and a null `pipeline_cache` is passed | C9 | — |
| Multiple windows / swapchains | absent | O1 editor (optional) | — |

### 5.3 Where a hybrid (mesh vs SDF) fork appears, and the number that decides it

**Every SDF candidate below is gated on D26.** While the edit authority is capped at 16 and the
host binds an empty clipmap, an SDF candidate is measured on a fixture that no shipped level can
resemble. A verdict taken before D26 lands is provisional, and is re-taken after.

| Campaign | Classical candidate | SDF candidate | Deciding measurement (reference GPU; 1080p/1440p/4K where GPU-bound) |
|---|---|---|---|
| F2 volumetric shadowing | CSM/atlas sampling per froxel [S18] | a cone-march per froxel (E-SDFVOL, D26) | fog pass ms on VB for both routes, with ≥ 64 lights |
| F4 god rays | from F2's classical shadowing, or a screen-space radial blur [S21] | from E-SDFVOL | ms at equal visual criteria (a golden pair) |
| E13 motion vectors on SDF pixels | the `hwrt` VIS-MV pass (exists; `hwrt` builds only) | a software producer: the marcher writes Δuv from the dominant edit's previous pose. That needs a per-edit previous-pose lane, and no extra field evaluation if the fold returns its argmin edit | marcher ms delta; MV error against the `hwrt` pass on `hwrt` boots; whether it works at all on non-`hwrt` builds |
| E18 specular AA on SDF pixels | none (no quad derivatives on the 1-D marcher) | curvature or footprint from extra field taps, which **meets the owner's zero-new-field-reads ruling that parked C-AA** | an owner value call first; then ms per hit pixel and highlight error against a 16-spp reference |
| E19 SDF edge AA | TAA with `RasterAndBasis` jitter (shipped) | C-AA's analytic coverage (4 frozen-field taps per hit pixel; parked) | edge error against a 16-spp reference on SDF silhouettes; ms per path |
| G9 particle collision | depth-buffer collision (mesh-only scenes, rain splashes) | SDF collision (shipped for the SDF leg; for the mesh leg it needs D26's mesh distance field). Particle research fact R6: "SDF collision beats depth-buffer collision" | collisions per ms; the miss rate on mesh geometry; VRAM for the mesh distance field |
| Explosion craters | a scorch decal (D9) plus a physics collider change | an SDF subtractive edit through the M3 dirty re-bake | re-bake ms per crater (M3's sub-region upload); physics query cost; each crater spends an edit slot, so this needs D26 |
| I3 queries / I9 character controller | a BVH or broadphase tree over shapes | a field distance query (the `sdf_query.rs` shape) | µs per 1k raycasts and per 1k capsule sweeps; CPU, W = 1 and 8 |
| S1 navmesh voxelization | rasterize triangles into spans (Recast) [S29] | sample the SDF sign into spans | build ms per tile at equal path quality |
| J6 audio occlusion | a physics raycast | an SDF march | µs per source per update |
| G14 rain occlusion | a top-down depth pass (a CSM-like cascade; Remember Me's 256² over 20 m [S26]) | an SDF query from above | ms per frame; VRAM MB |
| P1 terrain | heightfield + GPU quadtree [S37] | SDF terrain through the existing marcher | ms at equal coverage; VRAM MB; physics query µs |

### 5.4 In-house cost ledger (owner rule 5)

| Item | In-house scope | External candidate | Recommendation |
|---|---|---|---|
| BC1/BC5/BC7 encoder (offline) | an offline tool, not shipped in the game binary | bc7enc-class libraries (not cited here) | in-house by default, following the PNG precedent. An external encoder is defensible because it never links into the game. Owner call (§7 Q3) |
| Compressed audio decoder | Ogg Vorbis is moderate; Opus is large | libvorbis / libopus (not cited here) | in-house Vorbis first (Godot imports Vorbis [S2]); defer Opus |
| Complex text shaping | only the scripts the game ships | HarfBuzz [S41] | scope-limited in-house until the language list is known (§7 Q5) |
| Navmesh | voxelize → regions → polygons → detail mesh; A*; crowd | Recast/Detour, zlib [S29] | in-house, with Recast as the behavioural oracle in tests |
| Temporal upscaler | C-TSR in-house (the FSR2 recipe is MIT-licensed and documented) | FSR 2 (MIT), DLSS and XeSS (binary-only licences; post-FX survey T23) | in-house, following the published FSR2 stages; binary-only upscalers conflict with owner rule 5 |
| Store SDK | none possible | Steamworks [S47] | external, in an optional crate, zero cost when not linked |
| Crash dumps | `MiniDumpWriteDump` through the OS's dbghelp (an OS DLL, not a dependency) | Crashpad [S45] | an in-house writer plus a symbol store |
| FBX import | a proprietary format | the vendor SDK | do not build (C2 is P3); convert to glTF offline |

---

## 6. Numbers used in this document

| Number | Kind | Source |
|---|---|---|
| Froxel fog ~1.1 ms total, 0.43 ms density+lighting, 1.6 ms at double resolution (AC4 console target) | published | [S18] slide 47 |
| Froxel fog on the reference GPU: 0.42–1.07 ns/froxel; 0.39–0.99 ms at 160×90×64; apply 0.07–0.40 ms by resolution | estimate | [render/VOLUMETRICS-RESEARCH.md](render/VOLUMETRICS-RESEARCH.md) §4.2–4.3 |
| Nubis ~2 ms, 20 MB; naive ~20 ms; 1 of 16 pixels per frame | published | [S20] slides 2, 93–95 |
| Sky LUTs ≈ 0.2 ms; radial-blur shafts 0.2–0.4 ms at 1440p; per-pixel march 1.4–2.8 ms per light at 1440p; 2.5D clouds 0.6–0.95 ms at 1440p | estimate | volumetrics survey §4.4 |
| FSR 2 at 4K Quality: < 1.3 ms (RX 6800 XT), < 1.9 ms (RX 6700 XT), < 2.4 ms (RX 5700 XT) | vendor claim | [S24] |
| FSR 2.2.1 on RX 6650 XT: 1.2 ms at 1440p Quality, 2.8 ms at 4K Quality; 207 MB working memory at 1440p Quality | vendor claim | post-FX survey T22 |
| FXAA 0.11 ms, SMAA 0.30 ms at 1080p; 0.37 / 0.98 ms at 4K (GTX 1080) | third-party measurement | post-FX survey T19 (Intel CMAA2 table) |
| Histogram auto-exposure 0.17 ms at full 1080p (RTX 2080) | third-party measurement | post-FX survey T4 (Tardif) |
| TAA ≤ 0.5 ms at 1080p; derived 0.30–0.45 ms; +49.8 MB | in-tree estimate | [TAA-PLAN.md](TAA-PLAN.md) |
| Blended fill 15.1–42 Gpx/s; one screen of blended coverage 0.049–0.549 ms by resolution | estimate | [render/VFX-RESEARCH.md](render/VFX-RESEARCH.md) §2 |
| Remember Me rain ≈ 2.8 ms (PS3, 720p); occlusion map 256², 20 m, 0.32 ms | published | [S26] via VFX survey §8 |
| Windows audio engine latency 1.3 ms; default buffer 10 ms; inbox HDAudio 128–480 samples = 2.66–10 ms at 48 kHz | vendor documentation | [S34] |
| RTX 3060 Laptop particle sim ≈ 1.0–1.1 ns per particle (102 µs at 102k, 1.11 ms at 1M) | measured in-tree, flagged SUSPECT | [PARTICLES-PLAN.md](PARTICLES-PLAN.md) |
| Reference GPU bandwidth 288–336 GB/s peak; **250 GB/s effective assumed** | vendor spec; in-tree assumption | [PARTICLES-PLAN.md](PARTICLES-PLAN.md); [TAA-PLAN.md](TAA-PLAN.md) |
| E1 VRAM: +8.3 / +14.7 / +33.2 MB (probed arm), +24.9 / +44.2 / +99.5 MB (fallback arm); SSAA and `lit_prev` lines | derived | §3.1 |
| Post-pass floors: 0.066 / 0.12 / 0.27 ms (probed) and 0.13 / 0.24 / 0.53 ms (fallback) per separate pass; the fused tail 0.075 / 0.13 / 0.30 and 0.11 / 0.19 / 0.43 ms | derived at the assumed 250 GB/s | §3.1 |
| Bloom chain ≈ 9.7 B/px → 0.08 / 0.14 / 0.32 ms | derived at the assumed 250 GB/s | §3.5 |
| First image-quality set ≈ 0.6–0.8 ms at 1440p | estimate (sum of §3.5 rows) | §3.5 |
| E-FOG volumes: 7.37 MB each, 14.7 MB for 2, 29.5 MB for 4 | derived | §3.3 |
| Audio mixer CPU for 64 voices, 48 kHz stereo: ~61k multiply-adds per 10 ms block for a plain mix. With resampling, filtering and panning at ~20 flops per sample, ~1.2 MFLOP per block: ≈ 0.3 ms per 10 ms block on one core at a conservative 4 GFLOP/s scalar, and well under 0.1 ms with 8-wide AVX2 | **estimate** | derivation in this cell |
| BC7 = 8 bits per texel, BC1 = 4, RGBA8 = 32; a 2048² RGBA8 texture with mips is ~22.4 MB, and ~5.6 MB as BC7 | derived from the format definitions | — |

---

## 7. Owner questions (values and scope only)

1. **The target game.** Is the first shipped game a single-player third-person action game on
   Windows (this document's assumption)? Or is it multiplayer, open-world, or something else?
   - Multiplayer moves networking from Wave 6 to Wave 2.
   - Open world moves C14, P1, P2, C18 and D4 up a wave.
2. **Platforms.** CLAUDE.md's "Target platform" says Windows and Linux, but this audit's reference
   game assumed Windows only; that is the conflict.
   - If Linux (for example the Steam Deck) is a shipping target, A8 becomes P1 and joins Wave 3.
   - Either way, Wave 1's Windows APIs go behind the platform seam of §5.1.
3. **Offline-tool dependencies.** Does rule 5 apply to offline tools that never link into the game
   (the BC encoder, a mesh simplifier) as strictly as it does to the runtime?
4. **The HDR ballot — one decision.** Four entries in the tree are the same decision:
   - reflections R4a;
   - transparency R11 and its ballot 1;
   - the reflections update's Q1;
   - this audit's E1.

   **Recommendation: land it first in the render queue (Wave 1),** before transparency R1,
   reflections R5 and any post, volumetric or VFX-compositing rung. That goes against the
   2026-09-10 transparency ballot's "after R6". Three arguments:
   1. **The re-bless.** The 61 legs re-bless once whenever R4a lands. Every rung that lands before
      it (transparency R1–R6, soft particles, flipbooks, bloom) adds pins blessed in LDR, and
      those re-bless a second time.
   2. **Scope.** Transparency's "after R6" was argued inside its own scope: "Nothing above needs it
      for the seam". Post-FX, volumetrics and VFX need it for the seam itself: a bloom threshold,
      an exposure histogram, in-scatter addition and additive compositing all read
      scene-referred values.
   3. **The gate.** R4a's eight per-producer transition pins make the re-bless checked rather than
      trusted, and they are cheapest to run when no other rung is moving pixels.

   Deferring has a cost too: every effect in the owner's current topics waits, or gets written
   twice.
5. **Languages.** Which languages ship first? CJK and RTL decide whether L3 is S or L.
6. **The editor's place.** Is the editor (O1) a Wave 4 item, or does content production need it
   earlier? It is the largest single item after the unified plan.
7. **The refactor's place.** Owner answer Q-4 put the refactor campaign (Phase F step F4) "after
   everything else". Should that include the feature waves, as §4.1 recommends (F4 after Wave 5,
   with its kernel waves optionally after Wave 2)? Or should F4 run at the end of Phase F, before
   Wave 1? The latter delays every topic in this audit behind it.

The C-AA parking (E19, §5.3) is an existing owner value decision. This audit does not re-open it;
it only records that specular AA's SDF leg would meet the same constraint.

---

## 8. What was verified, and what was not

- **Verified at `6394bc5e`** by grep or read in this pass:
  - every `crates/…` path and symbol named above;
  - the status-deciding facts in §1.2;
  - every critique-disputed row: E1, E4, E5, E7–E10, E13, E14, F3, K3, I4, I5, A7 and D25;
  - R4a's text, its target table (§1.5) and decision D4;
  - transparency R11 and ballot 1;
  - `TAA-PLAN.md`'s target metrics and item 8;
  - Track C (C-AA, C-TSR), Track E (E-FOG, E-DENS-A, the polish tier) and ruling M1;
  - the parked C-AA and B7 designs;
  - the unified plan's Q-4 answer, Phase E/F rungs, §4.3 lock table and §5 census;
  - Gaia's F4/F5 rulings;
  - the particle plan's P2b and fact R6;
  - the brick atlas, clipmap and mesh-SDF state in the host;
  - the trunk head (`git log -1`).
- **Taken from the scout's same-day brief without re-reading the source:** the rendering pass
  lists and the RHI's required-feature lists. Rows that depend on these say so in their Evidence
  cells.
- **Taken from the sibling surveys** (§3.0), cited by section and technique number: the
  per-technique costs in §3.5 and §6 marked as theirs, and the volumetrics survey's plan-delta
  facts. They were read on 2026-09-25, while those documents were uncommitted.
- **Web.** Primary pages were fetched for [S2], [S3], [S6], [S7], [S9], [S24], [S28], [S29], [S30],
  [S34] and [S51]:
  - The Wronski [S18], Nubis [S20] and Tokuyoshi–Kaplanyan [S52] PDFs were downloaded and their
    text extracted; the numbers and statements above are quoted from that text.
  - The session's web-search budget (200 calls) ran out during revision 1, so revision 2 fetched
    known URLs only.
  - Items marked "not fetched" or "bibliographic" in §9 are cited by title, venue and year, with
    their URL unverified in this pass.
- **Not done:** no build, no test, no timing, no GPU run. Nothing here is a measurement of boyko.

---

## 9. Sources

The Verified column uses three terms:

- **fetched**: the page was opened in this pass;
- **search**: a web search returned the URL, and its result summary supports the claim, but the
  page itself was not opened;
- **bibliographic**: the item is cited by title and venue, with no URL verified this pass.

| # | Source | Verified |
|---|---|---|
| S1 | J. Gregory, *Game Engine Architecture*, 4th ed. (two volumes). https://www.gameenginebook.com/ | fetched |
| S2 | Godot Engine 4.7, "List of features". https://docs.godotengine.org/en/stable/about/list_of_features.html | fetched |
| S3 | Unity HDRP 17, "Features". https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.0/manual/HDRP-Features.html | fetched |
| S4 | Unity Entities 1.x, "Further DOTS and ECS packages". https://docs.unity3d.com/Packages/com.unity.entities@1.2/manual/ecs-packages.html | search |
| S5 | Unity Netcode for Entities 1.10. https://docs.unity3d.com/Packages/com.unity.netcode@1.10/manual/index.html | search |
| S6 | O3DE, "Gem Reference". https://docs.o3de.org/docs/user-guide/gems/reference/ | fetched |
| S7 | Bevy 0.19 release notes. https://bevy.org/news/bevy-0-19/ | fetched |
| S8 | Bevy, "Hot patching systems with subsecond", PR #19309. https://github.com/bevyengine/bevy/pull/19309 ; Bevy 0.17 notes https://bevy.org/news/bevy-0-17/ | search |
| S9 | Unreal Engine 5.8, "Post Process Effects". https://dev.epicgames.com/documentation/en-us/unreal-engine/post-process-effects-in-unreal-engine | fetched |
| S10 | Unreal Engine 5.8: "Local Fog Volumes" https://dev.epicgames.com/documentation/unreal-engine/local-fog-volumes-in-unreal-engine?lang=en-US ; "Volumetric Fog" https://dev.epicgames.com/documentation/unreal-engine/volumetric-fog-in-unreal-engine ; "Environmental Light with Fog, Clouds, Sky and Atmosphere" https://dev.epicgames.com/documentation/en-us/unreal-engine/environmental-light-with-fog-clouds-sky-and-atmosphere-in-unreal-engine | search |
| S11 | Unreal Engine 5.8, "World Partition". https://dev.epicgames.com/documentation/unreal-engine/world-partition-in-unreal-engine?lang=en-US | search |
| S12 | Unreal Engine 5.8, "Overview of Mass Gameplay" (Mass, StateTree). https://dev.epicgames.com/documentation/unreal-engine/overview-of-mass-gameplay-in-unreal-engine?lang=en-US | search |
| S13 | Unreal Engine public roadmap, "Iris Networking System". https://portal.productboard.com/epicgames/1-unreal-engine-public-roadmap/c/2030-iris-networking-system | search |
| S14 | Unreal Engine 5.8, "Overview of Niagara Effects" https://dev.epicgames.com/documentation/unreal-engine/overview-of-niagara-effects-for-unreal-engine?lang=en-US ; "Key Concepts in Niagara" https://dev.epicgames.com/documentation/en-us/unreal-engine/key-concepts-in-niagara-effects-for-unreal-engine | search |
| S15 | Unreal Engine 5.8, "PSO Precaching". https://dev.epicgames.com/documentation/en-us/unreal-engine/pso-precaching-for-unreal-engine | search |
| S16 | Y. O'Donnell, "FrameGraph: Extensible Rendering Architecture in Frostbite", GDC 2017. https://www.gdcvault.com/play/1024612/FrameGraph-Extensible-Rendering-Architecture-in | search |
| S17 | S. Hillaire, "Physically Based and Unified Volumetric Rendering in Frostbite", SIGGRAPH 2015 Advances. https://www.ea.com/frostbite/news/physically-based-unified-volumetric-rendering-in-frostbite | search |
| S18 | B. Wronski, "Volumetric Fog: Unified Compute Shader-Based Solution to Atmospheric Scattering", SIGGRAPH 2014 Advances. https://bartwronski.com/wp-content/uploads/2014/08/bwronski_volumetric_fog_siggraph2014.pdf | fetched (PDF text extracted) |
| S19 | S. Hillaire, "A Scalable and Production Ready Sky and Atmosphere Rendering Technique", EGSR 2020, CGF 39(4). https://onlinelibrary.wiley.com/doi/abs/10.1111/cgf.14050 ; reference implementation https://github.com/sebh/UnrealEngineSkyAtmosphere | search; repo fetched |
| S20 | A. Schneider, "The Real-time Volumetric Cloudscapes of Horizon Zero Dawn", SIGGRAPH 2015 Advances. https://advances.realtimerendering.com/s2015/The%20Real-time%20Volumetric%20Cloudscapes%20of%20Horizon%20-%20Zero%20Dawn%20-%20ARTR.pdf | fetched (PDF text extracted) |
| S21 | K. Mitchell, "Volumetric Light Scattering as a Post-Process", GPU Gems 3, ch. 13. https://developer.nvidia.com/gpugems/gpugems3/part-ii-light-and-shadows/chapter-13-volumetric-light-scattering-post-process | search |
| S22 | J. Jimenez, "Next Generation Post Processing in Call of Duty: Advanced Warfare", SIGGRAPH 2014 Advances. https://www.iryoku.com/next-generation-post-processing-in-call-of-duty-advanced-warfare/ | search |
| S23 | B. Karis, "High Quality Temporal Supersampling", SIGGRAPH 2014 Advances. https://de45xmedrsdbp.cloudfront.net/Resources/files/TemporalAA_small-59732822.pdf | search |
| S24 | AMD GPUOpen, "FidelityFX Super Resolution 2". https://gpuopen.com/fidelityfx-superresolution-2/ | fetched |
| S25 | J. Geffroy, Y. Wang, A. Gneiting, "Rendering the Hellscape of Doom Eternal", SIGGRAPH 2020 Advances. https://advances.realtimerendering.com/s2020/RenderingDoomEternal.pdf | search |
| S26 | S. Lagarde, "Water drop 2a – Dynamic rain and its effects" https://seblagarde.wordpress.com/2012/12/27/water-drop-2a-dynamic-rain-and-its-effects/ ; "Water drop 3b – Physically based wet surfaces" https://seblagarde.wordpress.com/2013/04/14/water-drop-3b-physically-based-wet-surfaces/ | search (the VFX survey fetched 2a) |
| S27 | S. Clavet, "Motion Matching and The Road to Next-Gen Animation", GDC 2016. https://www.gdcvault.com/play/1023280/Motion-Matching-and-The-Road | search |
| S28 | Jolt Physics README. https://github.com/jrouwe/JoltPhysics/blob/master/README.md | search (README summary) |
| S29 | Recast Navigation. https://github.com/recastnavigation/recastnavigation | fetched |
| S30 | D. Isla, "Handling Complexity in the Halo 2 AI", GDC 2005. https://www.gamedeveloper.com/programming/gdc-2005-proceeding-handling-complexity-in-the-i-halo-2-i-ai | fetched |
| S31 | T. Ford, "Overwatch Gameplay Architecture and Netcode", GDC 2017. https://www.gdcvault.com/play/1024001/-Overwatch-Gameplay-Architecture-and | search |
| S32 | M. Stallone, "8 Frames in 16ms: Rollback Networking in Mortal Kombat and Injustice 2", GDC 2018. https://www.gdcvault.com/play/1025471/8-Frames-in-16ms-Rollback | search |
| S33 | E. Ruskin, "Marvel's Spider-Man: A Technical Postmortem", GDC 2019. https://www.gdcvault.com/play/1026496/-Marvel-s-Spider-Man | search |
| S34 | Microsoft, "Low Latency Audio". https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/low-latency-audio | fetched |
| S35 | Microsoft GDK, "GameInput introduction" https://learn.microsoft.com/en-us/gaming/gdk/docs/features/common/input/overviews/input-overview?view=gdk-2604 ; "XInput Game Controller APIs" https://learn.microsoft.com/en-us/windows/win32/xinput/xinput-game-controller-apis-portal | search |
| S36 | Microsoft, "Xbox Accessibility Guidelines" (v3.2). https://learn.microsoft.com/en-us/gaming/accessibility/guidelines | fetched |
| S37 | J. Moore, "Terrain Rendering in Far Cry 5", GDC 2018 | bibliographic |
| S38 | E. Wohllaib, "Procedural Grass in Ghost of Tsushima", GDC 2021 | bibliographic |
| S39 | M. McGuire, P. Hennessy, M. Bukowski, B. Osman, "A Reconstruction Filter for Plausible Motion Blur", I3D 2012 | bibliographic |
| S40 | Unicode Standard Annex #9, "Unicode Bidirectional Algorithm". https://www.unicode.org/reports/tr9/ | not fetched |
| S41 | HarfBuzz text shaping engine. https://github.com/harfbuzz/harfbuzz | not fetched |
| S45 | Crashpad. https://chromium.googlesource.com/crashpad/crashpad/ | not fetched |
| S46 | Khronos OpenXR. https://www.khronos.org/openxr/ | not fetched |
| S47 | Valve, Steamworks SDK documentation. https://partner.steamgames.com/doc/sdk | not fetched |
| S49 | Bevy editor prototypes, "Roadmap". https://bevyengine.github.io/bevy_editor_prototypes/roadmap.html | search |
| S50 | AccessKit (Rust UI accessibility infrastructure). https://github.com/AccessKit/accesskit | not fetched |
| S51 | A. Kaplanyan, S. Hill, A. Patney, A. Lefohn, "Filtering Distributions of Normals for Shading Antialiasing", HPG 2016 (best paper): "compatible with deferred shading, normal maps". https://research.nvidia.com/publication/2016-06_filtering-distributions-normals-shading-antialiasing | fetched |
| S52 | Y. Tokuyoshi, A. S. Kaplanyan, "Improved Geometric Specular Antialiasing", I3D 2019. For deferred rendering, it filters with the average normal's derivatives inside the shading quad, taken at G-buffer time. https://www.jp.square-enix.com/tech/library/pdf/ImprovedGeometricSpecularAA.pdf | fetched (PDF text extracted) |

In-repository sources are linked inline. The ones this audit leans on:

- [FEATURE_MAP.md](FEATURE_MAP.md), [REMAINING-GAPS.md](REMAINING-GAPS.md)
- [OPTIMIZATION-PLAN-RENDER.md](OPTIMIZATION-PLAN-RENDER.md), [RENDER-AA-AND-TAILS-PLAN.md](RENDER-AA-AND-TAILS-PLAN.md),
  [TAA-PLAN.md](TAA-PLAN.md), [RENDER-GRAPH-API-PLAN.md](RENDER-GRAPH-API-PLAN.md),
  [RENDER-C-AA-DESIGN-PARKED.md](RENDER-C-AA-DESIGN-PARKED.md), [RENDER-B7-DESIGN-PARKED.md](RENDER-B7-DESIGN-PARKED.md),
  [PARTICLES-PLAN.md](PARTICLES-PLAN.md), [sdf-engine-architecture.md](sdf-engine-architecture.md)
- [render/REFLECTIONS-DESIGN-SPACE.md](render/REFLECTIONS-DESIGN-SPACE.md),
  [render/TRANSPARENCY-DESIGN-SPACE.md](render/TRANSPARENCY-DESIGN-SPACE.md)
- the same-day batch: [render/POSTFX-AA-RESEARCH.md](render/POSTFX-AA-RESEARCH.md),
  [render/VOLUMETRICS-RESEARCH.md](render/VOLUMETRICS-RESEARCH.md), [render/VFX-RESEARCH.md](render/VFX-RESEARCH.md),
  [render/POSTFX-AA-DESIGN-SPACE.md](render/POSTFX-AA-DESIGN-SPACE.md),
  [render/VOLUMETRICS-DESIGN-SPACE.md](render/VOLUMETRICS-DESIGN-SPACE.md), [render/VFX-DESIGN-SPACE.md](render/VFX-DESIGN-SPACE.md),
  [render/REFLECTIONS-UPDATE-2026-09-25.md](render/REFLECTIONS-UPDATE-2026-09-25.md),
  [render/TRANSPARENCY-UPDATE-2026-09-25.md](render/TRANSPARENCY-UPDATE-2026-09-25.md),
  [render/DYNAMIC-MATERIALS-RESEARCH.md](render/DYNAMIC-MATERIALS-RESEARCH.md), [audio/AUDIO-RESEARCH.md](audio/AUDIO-RESEARCH.md)
- [animation/ANIMATION-DESIGN-SPACE.md](animation/ANIMATION-DESIGN-SPACE.md),
  [physics/ADVANCED-PHYSICS-DESIGN-SPACE.md](physics/ADVANCED-PHYSICS-DESIGN-SPACE.md)
- [editor/EDITOR-DESIGN.md](editor/EDITOR-DESIGN.md), [modding/MODDING-DESIGN-SPACE.md](modding/MODDING-DESIGN-SPACE.md)
- [gaia/CAMPAIGN.md](gaia/CAMPAIGN.md), [AETHER-GAIA-REVISION-2026-08-29.md](AETHER-GAIA-REVISION-2026-08-29.md),
  [ASSET-STREAMING-PLAN.md](ASSET-STREAMING-PLAN.md), [SERIALIZATION-PLAN.md](SERIALIZATION-PLAN.md)
- [unification/UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md](unification/UNIFIED-SYSTEM-PLAN-00-OVERVIEW.md),
  [unification/UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md](unification/UNIFIED-SYSTEM-PLAN-02-ORDER-OF-WORK.md)
- [CAPABILITY-STATE-MODEL.md](CAPABILITY-STATE-MODEL.md), [SHADER-VARIANT-MANIFEST.md](SHADER-VARIANT-MANIFEST.md)

---

## 10. Review log (revision 1 → revision 2)

The review returned CHANGES_REQUESTED with 2 critical, 6 important and 6 optional remarks. All 14
are accepted. None is refuted. Three are accepted with a correction to the remark itself, noted
in its row.

| # | Remark | Verdict | What changed, and the evidence |
|---|---|---|---|
| C1 | E1 is PLANNED, not ABSENT; §3.1 re-opened R4a's decisions; wrong producer count; `HdrMode::Off` question; VRAM table | **accepted** | E1 → PLANNED, citing R4a + D4, R11 and TAA-PLAN item 8. §3.1 rewritten as "adopt R4a", with its decisions tabled. Eight producers (R4a's own list). The VRAM table is rebuilt: the `lit` ring ×2, `display` 1 × RGBA8 (from R4a's §1.5 table), SSAA at 2× per axis, and `lit_prev` split out as reflections R5's. Q4 is replaced by one joint HDR ballot with an argued recommendation (§7 Q4). The sibling reflections update's Q1 and transparency update's ballot 1 are the same decision, and the text says so. **Correction:** `lit_prev` belongs to R5 in R4a's own target table, so it is not an E1 cost |
| C2 | Seven open P0 rows unscheduled; Wave 2 mislabelled | **accepted, extended** | All seven placed: A2, L5, C15, J3, C10 and N2 in Wave 1, R3 in Wave 2. The mechanical cross-tabulation (§4.4) found three more the review missed: **C1, D16 and B4**. They are now in Wave 2. Wave 2 is relabelled "the P0 gameplay set, plus the first image-quality set", with the P1 lane named as such. J4 (P0) moved from Wave 3 to Wave 2, and D7, G2 and G4 (P0) from Wave 3 to Wave 2. C14 moved to Wave 2 because Gaia ballot F5 put cell streaming into G6 |
| W1 | Status rule misapplied | **accepted** | E4, E5, E7, E8, E9 (lens distortion noted as unplanned), E10, E14 and F3 → PLANNED. K3, I4 and A7 → PARTIAL, under the rule now written into §1.2 for declared-but-unread seams (D25 → PARTIAL under the same rule, O6). I5's evidence is corrected to `Manifolds::sensor_overlaps()`. I4 now says the work is wiring, not a second representation. **Also found:** B4, C11 and C13 called Gaia ballots F4/F5 "open", but both were ruled on 2026-08-30 ([gaia/CAMPAIGN.md](gaia/CAMPAIGN.md)); fixed |
| W2 | F4 in Wave 0 conflicts with Q-4 and with §4.1's carve-out; the M1 reversal is unargued | **accepted** | F4 is out of Wave 0. Its placement after Wave 5 is argued from Q-4's own reason (§4.1), with the counter-argument stated and owner question Q7. The carve-out's conflict with O4 disappears once F4 leaves Wave 0: O4 gates only F4's device waves. The M1 reversal is argued in §4.3: the fast track shipped or was parked, the owner raised the priority, and the cost is measured against §3.5. DOF, motion blur and lens effects stay polish |
| W3 | E13 in Wave 1 ignores a recorded, measured decline | **accepted** | E13 moved to Wave 3, with its real consumers (E8, E14, G3). The decline and its four reasons are recorded in the row. A rule is added: no MV-at-every-pixel consumer is armed on a non-`hwrt` build without an SDF producer or a mask fallback. The SDF motion-vector fork is added to §5.3 |
| W4 | C-AA, specular AA, in-tree AA numbers and particle interpolation are missing | **accepted, with one correction** | E18 (specular AA, sourced from [S51] and [S52], both fetched), E19 (C-AA) and G15 (P2b) added. The in-tree TAA budget is cited in E12 and §3.5. G1 records the 64 Hz stutter. Two forks added to §5.3: C-AA vs TAA, and specular AA on SDF. **Correction:** C-AA is not merely missing. It was designed and **parked by the owner** ([RENDER-C-AA-DESIGN-PARKED.md](RENDER-C-AA-DESIGN-PARKED.md)) to keep the frozen field at zero new reads. So E19 is PLANNED (parked), and its fork records an owner value constraint rather than asking for a measurement the owner declined |
| W5 | E/F/G rows lack path coverage | **accepted** | A Paths column on every §3 row, plus §3.6 (what the per-path facts cost). E1's effort is now M + S for the F/F+ post seam. §3.3's measurement is scoped to VB. F2's effort carries +M per path that arms a light list |
| W6 | No row for the SDF field at game scale | **accepted, extended** | D26 added (PARTIAL, Wave 2). §5.3 now opens with the gating rule. **Found while verifying:** the edit *authority* itself (`SdfEditField`) is a fixed 16-slot array, byte-identical to physics, so even the existing brick atlas bakes from ≤ 16 edits. The mesh distance field (the MDF baker and a marcher tap) exists and is off in the host (`mesh_sdf_enabled: false`) |
| O1 | Smaller taxonomy omissions | **accepted** | B10 (transform hierarchy, PRESENT), C17 (VRAM budget, PLANNED in §2a), C18 (texture streaming, ABSENT), A18 (wasm, PARTIAL), G16 (wind field) and Q4 (splines) added |
| O2 | The Linux conflict with CLAUDE.md; a platform seam | **accepted** | §7 Q2 records the conflict; §5.1 adds the platform-seam rule |
| O3 | Evidence nits | **accepted** | J8's Bevy "Better Audio" citation removed. C9: no `vkCreatePipelineCache` in `ffi.rs`, and a null `pipeline_cache` is passed. D22 and §5.2 name both stale comments. "F4" is disambiguated as "Gaia ballot F4" vs "Phase F step F4" |
| O4 | One bandwidth figure | **accepted** | 250 GB/s effective everywhere (TAA-PLAN's figure, the same one the post-FX survey uses); 288–336 GB/s peak stated; ratios use peak against peak. Every floor is recomputed |
| O5 | Two more hybrid forks | **accepted** | G9 depth vs SDF collision, and explosion craters through the M3 re-bake, added to §5.3 |
| O6 | D25 has a camera-selection seam | **accepted** | D25 → PARTIAL |

**The review's open questions:**

1. **Is the technique-level research produced elsewhere?** Yes. The same batch holds the post-FX,
   volumetrics and VFX surveys (§3.0), and §3.5 carries their per-technique reference-GPU
   numbers into this audit.
2. **Where does F4 sit?** After Wave 5; its kernel waves may optionally run after Wave 2 (§4.1,
   §7 Q7).
3. **Transparency ballot 1?** This audit recommends "first in the render queue", against "after
   R6". §7 Q4 gives three arguments: the double re-bless, the seam-vs-physics scope, and the
   transition gate.
4. **Is L9 C4 the trunk head?** Verified: `git log -1` at `6394bc5e` is "merge: u/phys-l9-c4 …
   into integ/unified".
