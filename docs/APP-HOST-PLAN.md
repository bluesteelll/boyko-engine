# `boyko_app` — the scene / present host (converged plan)

Status: **CONVERGED** (architect v1 → critic ITERATE (4×P0, 5×P1, 6×P2) → architect v2 →
focused critic verification → 3 final deltas folded here). Base commit: `1ee7ca8`.

## Goal

A first-class host layer taking a user from `App::new()` to a correct, interpolated,
windowed 3D scene in **~30 lines of ECS-native code**, with the entire frame discipline
(fence-before-write, per-slot uploads, prev/curr rotation, alpha push) enforced **by
construction**. Absorbs the frame loop currently inlined in
`crates/boyko_rhi_vulkan/tests/window_present_gbuffer.rs` (`run_interactive_viewer`).

Performance budget: host overhead ≤ 2 µs/frame vs the inline loop; **0 heap allocations
per frame** after warmup; `FrameWriteToken` = 8 B (`slot: usize`), register-passed;
camera upload = one ~96 B memcpy/frame; pair ring = one contiguous `cast_slice` memcpy
per frame (unconditional — see D5).

## Invariants preserved

- The slot fence is waited **before** any per-slot mapped write (the `80bf033`
  motion-shadow race class); `FrameWriteToken` is minted only by
  `Renderer::wait_frame_in_flight` (committed `1ee7ca8`).
- `boyko_rhi_vulkan` never depends up; per-entity durable data lives in ECS storage
  (Principle 0); `CoreSchedule` stays a closed enum; headless `App::run()` behavior
  preserved for existing users.

## Key decisions

### D1 — New top crate `boyko_app`; layering invariant amended

`boyko_app` (deps: `boyko_ecs`, `boyko_scene`, `boyko_render`, `boyko_rhi`,
`boyko_rhi_vulkan`, `boyko_input`, `boyko_math`) owns: the OS loop (Win32 pump), the
boot chain, the runner, plugin composition (`EnginePlugins`), the prelude.
`boyko_render` = "**what** to upload" (ECS-reading, token-typed upload fns);
`boyko_app` = "**when**" (sequencing, token minting, window/swapchain lifetime).

The `boyko_render/Cargo.toml` invariant ("the ONLY crate allowed to name both the RHI
and the ECS core") is **amended, not silently broken**: *`boyko_render` (the data
bridge) and `boyko_app` (the host) are the only two crates that name both the graphics
RHI and the ECS core; `boyko_app` must not define per-entity GPU data paths — those
belong in `boyko_render`.* Same sentence lands in `docs/ARCHITECTURE.md`.

Rejected: host inside `boyko_render` (couples input/OS loop into the data bridge);
examples-only (users re-hand-wire the fence discipline — the exact bug class just
fixed); backend-generic frame trait in `boyko_rhi` (no frame vocabulary exists there;
speculative for one backend — the runner is the containment seam for a future port).

### D2 — Device ownership: leaked `&'static VulkanContext` singleton + explicit world eviction

`VulkanContext` is already morally a handle (it holds `*const DeviceFns` into stable
storage). The runner makes that official:

1. Boot: `let ctx: &'static VulkanContext = Box::leak(Box::new(VulkanContext::boot(..)))`
   — one intentional setup-stage leak per process, lifecycle ended explicitly (below).
2. Host side: `Surface<'static>` / `Swapchain<'static>` / `Renderer<'static>` — the
   existing `<'ctx>` signatures instantiated at `'static`. No self-referential struct.
3. World side: `RhiContext::from_shared(ctx: &'static VulkanContext)`. `RhiContext`
   becomes **dual-mode owned | shared** internally (critic delta A2): the owned variant
   keeps today's destroy-on-Drop semantics so `RhiContext::new(ctx: VulkanContext)`
   (every existing test/example) tears down **verbatim** — device/instance/loader are
   destroyed exactly as before. The shared variant frees only column resources on Drop,
   never the device. The discriminant is touched only at setup/teardown — zero hot-path
   cost.
4. Startup device access: the runner inserts `GpuDevice(&'static VulkanContext)`
   (NonSend) **before** `finish()`; `MeshRegistry::register_mesh` gains a thin
   `&GpuDevice` overload.

**Teardown (critic delta A1 — the runner cannot drop the App it borrows):** the runner
**evicts** every device-referencing world resident explicitly, then destroys the device:

```
1. renderer.wait_idle()
2. drop WindowHost fields in declaration order:
   renderer → targets → gpu_bundles → swapchain → surface → window
3. EVICT from the World (explicit, in the runner):
   - remove_non_send_resource::<RhiContext>() and drop it (frees columns; shared mode
     does not touch the device)
   - take MeshRegistry and call its unsafe destroy(ctx) under the step-1 idle
   - remove GpuDevice (no dangling &'static may remain in a live structure)
4. unsafe { VulkanContext::destroy(ctx as *const _) } — LAST statement of the runner
```

Post-run App state is pinned: the World is no longer GPU-capable; a `debug_assert!`
guards against a subsequent GPU-touching `app.update()`.

SAFETY invariants (verbatim for the implementation): both sides hold only shared
`&'static` refs; `VulkanContext` is write-never after boot (no `&mut` to it ever
exists); all queue access is runner-thread-only (`NonSend` resources + `!Send` App);
`destroy` is called exactly once, after steps 1–3 evicted every holder, so no
`&'static` reference *exists* afterwards (reference-validity, not merely "no deref").

### D3 — Token outside the World; real move semantics

`FrameWriteToken` stays a stack value inside the runner — the only mint-and-consume
site. Sequence per frame: `update_with_delta` (all ECS work — needs no token) →
`wait_frame_in_flight() -> token` → token-typed uploads (`&token`) →
`render_gbuffer_frame(token)` / `present_sampled(token)` **consume the token by value**.

R0b makes the move real (critic P0-2): remove `Clone, Copy`; `slot(self)` →
`slot(&self)`; borrow-taking writers migrate to `&FrameWriteToken`
(`RhiContext::ui_upload`, `pack_sort_upload`, the pair-ring write); consuming APIs take
it by value (`render_gbuffer_frame` **and** `present_sampled` — the discipline must not
fork between the gbuffer and UI-composite paths). Churn: ~20 call sites in
`window_present_gbuffer.rs`, 2 in `sdf_gbuffer_hybrid.rs`, UI goldens, 3+ doc
locations. Compile-fail tests become writable: reuse-after-move, `clone()`, mint
without wait (`forge_unfenced` stays the audited `unsafe` setup-seeding hatch).

Compile-time guarantees: no write before the wait (only mint = `wait_frame_in_flight`);
no write after submit (token moved); no cross-frame retention. v1 users cannot touch
per-slot mapped memory at all — they mutate ECS data; the host's upload steps are the
only writers. (v2: token as a steal-proofed frame-scoped resource for user-authored
GPU writes — deferred.)

### D4 — Interpolation host-owned; ordering seam; snap/teleport

- **Ordering seam:** public `#[derive(SystemSet)] enum FixedSet { Gameplay, Snapshot }`
  in `boyko_scene`; `EnginePlugins` wires `Snapshot.after(Gameplay)` in
  `CoreSchedule::Fixed`; `pack_gpu_transforms` runs `.in_set(FixedSet::Snapshot)`.
  Users (and physics, when composed) put Fixed gameplay `.in_set(FixedSet::Gameplay)`.
  Pack-after-write is pinned **by name**, not topological accident — no one-substep lag.
- **Pipeline:** `pack_gpu_transforms` (Fixed, exists) rotates prev/curr in the
  `GpuTransform3D` dense pair → `gather_mesh_draw_pairs` (Main, exists) buckets
  `DrawBatch`es + the 96 B pair ring into `MeshRenderScratch` → the runner uploads and
  arms `scene.interp = activation(s, FixedTime::overstep_fraction())`. The test-local
  `InterpPair` + inline fixed loop are deleted (replaced by the production parts they
  duplicated).
- **Snap/teleport (day one):** `SnapInterpolation` EnableTag. The pack runs two chunked
  passes: `.without_enabled(SNAP)` → normal shuffle; `.with_enabled(SNAP)` →
  `prev = curr = new`. A new Main system **`snap_apply`** (`.before(gather_mesh_draw_pairs)`)
  writes `prev = curr` for flagged entities and issues the deferred
  `disable::<SnapInterpolation>()`. Pinned mechanism attribution (critic): a Commands
  enable issued during substep k flushes at that substep's END, so the same-substep pack
  does not see the bit — the zero-streak property is delivered by `snap_apply` alone
  (Main, pre-gather); the pack's `with_enabled` pass covers bits set in earlier
  substeps/frames. The R5 test asserts the exact last-substep-teleport path.
  `EntityCommands::teleport_to(Transform)` = write Transform + enable SNAP in one
  deferred command. R5 also unit-tests the dense-component × EnableTag-filter query
  combination; pinned fallback if unsupported: `SnapInterpolation` becomes a plain ZST
  table tag (archetype-moving; teleports are rare).
- **Camera:** `FlyCamera` integrates real dt in Main — render-rate view smoothness while
  scene objects lerp between 64 Hz poses (the standard Unity/Godot split; matches the
  inline viewer). Documented bypass; camera-on-interpolated-target (vehicle cam) = v2.

### D5 — Upload gating: unconditional pair upload; deterministic light generation

**Pair ring (critic delta C):** uploaded **unconditionally every frame** — one
contiguous `bytemuck::cast_slice(&scratch.pair_ring)` memcpy into the mapped slot.
Numbers: N=1k instances ≈ 96 KB ≈ 5–10 µs to write-combined memory; v1 scenes are
sub-µs; `gather_mesh_draw_pairs` already rebuilds the ring every Main frame, so a gate
saves only the memcpy on fully-static no-substep frames while costing per-row
fingerprint work, four tracking fields, and a protocol whose hash variant had a
concrete stride-64 collision class (rotl-xor is invariant for rows 64 apart — silent
wrong-render under id-recycled respawn). Correct by construction beats
probability-correct. If large-N profiling later justifies gating, it returns as a
**deterministic writer-side generation** (pack/snap/structural hooks bump a counter) —
never a hash.

**Light table:** `LightTableGeneration(u64)` resource bumped by the light reconcile
whenever it rewrites staging (writer-side, deterministic, conservative is fine); host
keeps `light_uploaded_gen: [u64; FRAMES_IN_FLIGHT]`, uploads slot `s` iff its gen
differs.

### D6 — Runner installation and the `run()/finish()/startup` contract

`App::set_runner(&mut self, Box<dyn FnOnce(&mut App) -> AppExit>)` — the `&mut App`
signature avoids App extraction entirely. One setup-stage box, called once (the only
`dyn` in the design).

```
App::run(&mut self) -> AppExit:
  if let Some(runner) = self.runner.take():
      return runner(self)        // finish() NOT called by run() — the runner owns it
  <legacy headless path — byte-for-byte today's, incl. the unconditional
   AppExit(false) insert and "at least one frame" doc>
```

Windowed runner: boot → insert NonSend (`RhiContext::from_shared`, `GpuDevice`,
`MeshRegistry`, `WindowInfo`) → **insert-if-absent `AppExit(false)`** → `app.finish()`
(startup one-shots drain WITH the device present) → if `AppExit` set: teardown + return
(a startup-requested exit is honored) → frame loop → teardown (D2 order) → return exit.
AppExit semantics pinned per mode (critic P2): legacy keeps the unconditional insert
(true byte-for-byte); windowed uses insert-if-absent.

### D7 — `GBufferScene` stays the wire format; homes pinned

- `GBufferScene` (host-agnostic borrow bundle in `present/scene_types.rs`) remains the
  host→backend interface, assembled **on the stack** each frame (POD + refs, zero
  alloc).
- `InterpGpuProd` (productionized `InterpGpu`: pairs/draw rings, bind groups, pipeline)
  lives in host `GpuSceneBundles` — its consumers are exclusively host steps; the
  producer side only touches `MeshRenderScratch` (World). The UI ring stays
  world-resident (systems produce its data) — both placements principled.
- UI path: the seam ships in v1 (`RhiContext` is world-resident anyway); `UiPlugin` is
  NOT in `EnginePlugins` defaults; `has_ui: bool` resolved once at boot gates the
  token-typed `pack_sort_upload` behind one predictable branch.
- **Resize policy v1:** composite extent **fixed at boot from the window client size**
  (not hardcoded), presented 1:1 top-left; window resize recreates the swapchain
  (existing `Ok(false)` path) and the blit clamps to `min(window, composite)`.
  `FlyCamera` aspect fixed at boot. Dynamic composite tracking = v2 (ripple enumerated:
  `tiles_buffer` via `tile_grid_extent`, camera UBO extent fields, SSAO/dispatch
  counts, `GBufferTargets` recreation).
- `WindowInfo` staleness contract: written post-present → Main readers observe the
  previous frame's size; inert in v1; documented on the type.

## Data structures

```rust
// boyko_app/src/host.rs — field order IS drop order (renderer first, window last)
pub(crate) struct WindowHost {
    renderer: Renderer<'static>,
    targets: GBufferTargets,
    gpu: GpuSceneBundles,          // interp: InterpGpuProd, camera/csm rings, tables,
                                   // pipelines, samplers, SDF static set, csm/atlas
    swapchain: Swapchain<'static>,
    surface: Surface<'static>,
    window: Window,
    light_uploaded_gen: [u64; FRAMES_IN_FLIGHT],
    has_ui: bool,
    composite_extent: (u32, u32),  // boot-fixed
}
// World (NonSend): RhiContext (from_shared), GpuDevice(&'static VulkanContext),
//   MeshRegistry, MeshRenderScratch, LightTableGeneration(u64), WindowInfo,
//   ViewUniform, CsmConfig, SsaoConfig, LightingFlags
// Components: Transform/GlobalTransform/Camera/Projection (boyko_scene), MeshHandle +
//   RenderEnabled + material, GpuTransform3D dense pair, lights, CsmCaster,
//   FlyCamera (boyko_app), SnapInterpolation (EnableTag; fallback: ZST table tag)
```

## Public API

```rust
// boyko_ecs (R1)
impl App { pub fn set_runner(&mut self, r: Box<dyn FnOnce(&mut App) -> AppExit>); }

// boyko_scene (R3)
#[derive(SystemSet)] pub enum FixedSet { Gameplay, Snapshot }

// boyko_render (R2/R3/R5)
impl RhiContext {
    pub fn from_shared(ctx: &'static VulkanContext) -> Self;  // shared mode
    // new(ctx: VulkanContext) keeps owned mode: destroy-on-Drop semantics unchanged
}
pub fn upload_camera_ring(token: &FrameWriteToken, ...);
pub fn upload_pair_ring(token: &FrameWriteToken, interp: &InterpGpuProd,
                        scratch: &MeshRenderScratch, slot: usize); // cast_slice memcpy
pub fn upload_light_table(token: &FrameWriteToken, ...);
impl EntityCommands { pub fn teleport_to(self, t: Transform) -> Self; }

// boyko_rhi_vulkan (R0b) — token is !Clone !Copy after R0b
pub fn wait_frame_in_flight(&self) -> Result<FrameWriteToken, SwapchainError>;
pub unsafe fn render_gbuffer_frame(&mut self, token: FrameWriteToken, ...) -> ...;
pub unsafe fn present_sampled(&mut self, token: FrameWriteToken, ...) -> ...;

// boyko_app
pub struct EnginePlugins { .. }   // ::window(title, w, h).present_mode(..)
pub struct FlyCameraPlugin;  pub struct FlyCamera { speed, sensitivity }
pub struct GpuDevice(..);    pub mod prelude;
```

Target user code (~30 lines): `App::new()` +
`add_plugins((EnginePlugins::window(..), FlyCameraPlugin))` + `add_startup_system(setup)`
+ `run()`; `setup(mut commands: Commands, mut meshes: NonSendMut<MeshRegistry>,
dev: NonSendRes<GpuDevice>)` spawns `MeshBundle` / `DirLightBundle`+`CsmCaster` /
`PointLightBundle` / `FlyCameraBundle` via `Commands`; Fixed gameplay systems land in
`FixedSet::Gameplay`.

## Runner frame

| # | Op |
|---|----|
| 1 | `window.drain_input` → `translate_win32*` → `RawInputQueue` → InputPlugin ingest |
| 2 | `app.update_with_delta(dt)` — Time → events → Fixed×N (pack in `Snapshot`) → Main (`snap_apply` → gather, light reconcile, propagation, `resolve_active_camera`) |
| 3 | `AppExit` / quit check |
| 4 | `token = renderer.wait_frame_in_flight()?` (the pacing point; `s = token.slot()`) |
| 5 | uploads, all demanding `&token`: camera ring (~96 B); pair ring (unconditional `cast_slice` memcpy); light table iff `light_uploaded_gen[s] != LightTableGeneration`; `if has_ui { pack_sort_upload(&token) }` |
| 6 | assemble `GBufferScene` on the stack (`composite_from_view`; `interp = activation(s, overstep_fraction())`) |
| 7 | `render_gbuffer_frame(token, ...)` — consumes the token; `Ok(false)` ⇒ recreate-skip (`#[cold]`), `Err` ⇒ exit |
| 8 | `refresh_size()`; write `WindowInfo` |

Simulation fully precedes the fence wait (CPU/GPU overlap preserved or improved vs the
inline loop). `drive_frame`'s internal signalled-fence re-wait stays as
defense-in-depth. Minimized window (0×0): skip 4–7, keep pumping.

## Threading

Runner thread = OS + render thread; `App` is `!Send + !Sync`; `WindowHost` never enters
the World or crosses threads; ECS parallelism inside `update_with_delta` unchanged; all
Vulkan queue access is runner-thread-only; the leaked `&'static VulkanContext` is
immutable after boot — no writable shared state crosses the host/World boundary.

## Migration

Test-only keeps: readback/BMP/3-drain harness, goldens, SKIP flow, AbPose +
shadow-lag/dolly/motion-A/B/interp-smoke diagnostics, P pose probe. R6 gate:
`examples/viewer.rs` must reproduce the owner flight-check workflow (env-var diagnostic
forks, pose probe, on-demand BMP dump behind a dev feature) **and receive owner OK**
before `run_interactive_viewer` is deleted.

## Rung ladder (each rung compiles green, independently commit-able)

- **R0b** — token move semantics, full scope: drop `Clone`/`Copy`, `slot(&self)`;
  `&token` for `ui_upload`/`pack_sort_upload`/`write_pairs`; by-value consume for
  `render_gbuffer_frame` + `present_sampled`; migrate ~22 call sites + docs;
  compile-fail tests (reuse-after-move, clone, forge).
- **R1** — `App::set_runner(FnOnce(&mut App) -> AppExit)` + the `run()` contract:
  runner-before-finish, per-mode AppExit semantics, startup-exit honored; headless path
  byte-for-byte; 4 unit tests.
- **R2** — `RhiContext` dual-mode ownership (`from_shared` + owned `new` semantics
  preserved); `boyko_app` crate: leak-boot topology, `WindowHost`, `GpuDevice`,
  clear-color runner (via existing `render_frame`), full teardown incl. world eviction
  + `VulkanContext::destroy`; layering amendment; `examples/clear.rs`.
- **R3** — camera + mesh: `FixedSet` in `boyko_scene`; boot-fixed composite policy +
  `WindowInfo`; `MeshRegistry` `&GpuDevice` overload; unconditional pair upload;
  zero-alloc counting-allocator test; structural-change-on-0-substep-frame test;
  `examples/room.rs`.
- **R4** — lights/shadows/SSAO composition; `LightTableGeneration` per-slot gen compare;
  world-fixed CSM resources.
- **R5** — interpolation: pack in `FixedSet::Snapshot`; `InterpGpuProd` (cast_slice
  upload); `snap_apply` + `teleport_to`; last-substep-teleport test; dense×EnableTag
  filter unit test (fallback: table tag); `interp_smoke` port over the host path;
  bouncing-cube example.
- **R6** — input + `FlyCameraPlugin`; `examples/viewer.rs` with owner flight-check
  parity; owner eval; then delete `run_interactive_viewer`.
- **R7** — `present::bootstrap` dedupe for the golden tests; SDF instance path; book
  page (doc-writer); v2 resize-tracking spike.

## Metrics and validation

Criterion benches (scene assembly + upload overhead vs baseline); frame-time A/B viewer
example vs the old inline loop; compile-fail token suite; zero-alloc/frame assertion;
goldens stay authoritative; windowed smoke under `BOYKO_DISABLE_VALIDATION=1
--test-threads=1`. `debug_assert!` set: `token.slot() == s` in every ringed write;
`light_uploaded_gen[s] <= LightTableGeneration` monotonicity; boot
`composite_extent > 0`; post-run world is GPU-evicted.

## Open questions (deferred, not blocking)

1. Backend-generic frame-driver trait in `boyko_rhi` — when a second backend exists.
2. v2 user-extensible upload phase (token as steal-proofed frame-scoped resource).
3. Camera-on-interpolated-target (vehicle cam) mode.
4. Dynamic composite/resize tracking (ripple list in D7).
