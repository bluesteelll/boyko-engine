# boyko_demo — Architecture & Implementation Plan

> Status: design (implementation-ready). Target branch: `ecs`.
> The ECS core (`boyko_ecs`, `boyko_macros`, `boyko_utils`, `boyko_threadpool`) is the product.
> `boyko_demo` is a NEW, separate workspace crate depending on `boyko_ecs` + `boyko_threadpool`,
> adding ZERO code to any core crate. The demo shows the engine off honestly: real
> `Schedule::run`, real `par_iter`, real Phase-17 states, real `for_each_chunk` SoA→GPU upload.

## 1. Scope

### 1.1 Deliverable (native-first)
`cargo run -p boyko_demo`: interactive GPU-instanced sandbox rendering 100k–1M+ instanced quads
in ONE `draw_indexed` per frame; three switchable modes via Phase-17 states (Particles, Boids,
Physics balls); live egui controls + `egui_plot` FPS graph + entity/ms readouts; mouse interaction
(gravity well on hold, click-to-spawn); multi-threaded scheduler + `par_iter` on native.

### 1.2 MVP (prove the pipeline first)
Native + Particles only, end-to-end: eframe window → ECS world + `Schedule` → integration via
`par_iter_mut` → `for_each_chunk` zero-copy upload → one instanced draw → egui panel (a few sliders
+ FPS plot + mouse well). Exercises every load-bearing seam. Boids/Physics/web are later waves.

### 1.3 Designed-for, deferred
Web (wasm): architecturally unblocked day 1 (§8), built last (Wave 7). Storage-buffer vertex
pulling (WebGPU-only native fast path): designed-for, behind a feature, not default.

### 1.4 Non-goals
Asset pipeline, audio, networking, serialization, 3D meshes (2D instanced quads only), hot-reload.

## 2. Workspace integration

### 2.1 Root `Cargo.toml` (the ONLY core-adjacent edit)
Append `"crates/boyko_demo"` to `workspace.members`:
```toml
workspace = { members = ["crates/boyko_ecs", "crates/boyko_macros", "crates/boyko_utils", "crates/boyko_threadpool", "crates/bench_bevy_vs_boyko", "crates/boyko_demo"] }
```
No other root edit; no edits to any `crates/boyko_*/` manifest or `src/`. The root also has a
`[package] boyko-engine` with its own `src/` — the demo does not touch it.

### 2.2 `crates/boyko_demo/Cargo.toml`
PIN `wgpu` to exactly what `eframe 0.34` resolves (version skew is the #1 build failure).
**Wave-0 step 0**: add `eframe = "0.34.3"`, run `cargo tree -p eframe -i wgpu`, pin the printed
version. Researcher's finding is `=29.0.1`; verify against the lockfile and treat the verified
value as authoritative.
```toml
[package]
name = "boyko_demo"
version = "0.1.0"
edition = "2024"
publish = false

[[bin]]
name = "boyko_demo"
path = "src/main.rs"

[dependencies]
boyko_ecs        = { path = "../boyko_ecs" }
boyko_macros     = { path = "../boyko_macros" }   # derives: Component, Bundle, Resource
boyko_threadpool = { path = "../boyko_threadpool" }

eframe    = { version = "0.34.3", default-features = false, features = ["wgpu", "default_fonts"] }
egui      = "0.34"
egui_plot = "0.35"
wgpu      = "=29.0.1"      # PIN to eframe's exact resolution (verify via `cargo tree`)
bytemuck  = { version = "1", features = ["derive"] }
rand      = "0.9"
log       = "0.4"

[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
env_logger = "0.11"
pollster   = "0.4"

[target.'cfg(target_arch = "wasm32")'.dependencies]
console_error_panic_hook = "0.1"
console_log              = "1"
wasm-bindgen             = "0.2"
wasm-bindgen-futures     = "0.4"
web-sys                  = "0.3"
getrandom                = { version = "0.2", features = ["js"] }

[features]
default = []
storage_instancing = []   # native-only WebGPU fast path (designed-for, off)
alloc_audit = []          # demo-only global-allocator counting shim for hot-path verification
```
`default-features = false` on eframe drops the glow (GL) renderer; keep `wgpu` + `default_fonts`.

### 2.3 Core-untouched confirmation
Only repo edits outside `crates/boyko_demo/`: one line in root `Cargo.toml`, this doc, and
Wave-7-only `index.html`/`Trunk.toml`/CI. The demo drives ONLY public `boyko_ecs` API (§9).

## 3. Decisions (numbered, justified)

### D1 — Stack: `eframe` (not raw winit+wgpu+egui glue). Pinned.
Choice: `eframe 0.34.3` (wgpu renderer) + `egui_plot 0.35` + `bytemuck 1`, `wgpu` pinned to
eframe's exact resolution. Why: one codebase native+web; eframe owns the winit-0.30
`resumed`/surface-lifetime dance and the wasm canvas entry; `egui_wgpu::CallbackTrait`
(prepare/paint) is confirmed sufficient for a fully custom instanced pipeline under the egui layer
(egui `custom3d_wgpu` example). Removes ~500 lines of event-loop/surface boilerplate that is pure
liability and not the product. Rejected: raw winit 0.30 + wgpu + egui-winit + egui-wgpu (control we
don't need; the version-skew across four glue crates is the dominant failure — eframe collapses it
to one pin). Trade-off: eframe owns the loop; we drive the sim from `App::update`. The loop is not
the product.

### D2 — Render component lives IN the sim archetype (zero-copy). The headline.
Choice: a `#[repr(C)] + bytemuck::Pod` `GpuInstance` component stored alongside Position/Velocity
in each sim entity's archetype; upload reads the column directly via
`for_each_chunk(|s| queue.write_buffer(buf, off, bytemuck::cast_slice(s)))`. Why:
`ComponentPool` allocates ONE contiguous 32B-aligned arena buffer per column
(`component_pool.rs:138`; chunks are metadata windows, `buffer_ptr()` :782 is the flat base);
`for_each_chunk` (`chunk_iter.rs`) yields exactly one `&[T]` per archetype over `[0,entity_count)`
at stride `size_of::<T>()`. A Pod component column is therefore ALREADY a valid GPU instance array
— `cast_slice` + `write_buffer`, NO intermediate AoS Vec, NO per-entity pack. Biggest honest
"what the SoA core buys you" moment; the bytes the GPU reads are exactly the bytes the ECS stored.
Rejected: a render-extraction pass (pack Position+Velocity into a `Vec<GpuInstance>` each frame) —
rebuilds an O(N) buffer every frame, defeating zero-copy and adding a per-frame alloc/clear.
Trade-off: a render concern sits in the sim archetype. Contained: `GpuInstance` is the demo's own
type (not core); a tiny `sync_gpu_instance` system writes it each frame (D3). The purity we protect
is the core engine, not the demo's component set.

### D3 — `GpuInstance` written by a dedicated `par_iter` system each frame.
Choice: `sync_gpu_instance` runs after integration, `par_iter_mut` over
`(&Position, &Velocity?, &Radius?, &mut GpuInstance)`, packing pos+size+color. Why: keeps
`GpuInstance` a plain Pod; the pack is a streaming SoA→SoA write (sequential read of Position,
sequential write of GpuInstance, both contiguous) — branch-free, autovectorizable, embarrassingly
parallel over disjoint rows (the demo's most honest `par_iter` win). Rejected: packing inside the
integration system (couples concerns, bloats register pressure; a separate system lets the
scheduler parallelize it). Trade-off: one extra column (≤32 B/entity) — but that column IS the GPU
buffer's CPU mirror, not overhead.

### D4 — Instancing via instance vertex buffer (`step_mode: Instance`), not PointList, not storage pulling.
Choice: a tiny static unit-quad vertex buffer (`step_mode: Vertex`, 4 verts + 6-index buffer) +
the per-instance `GpuInstance` buffer (`step_mode: Instance`); one `draw_indexed(0..6, 0, 0..N)`.
Why: instance vertex buffers work on BOTH WebGL2 and WebGPU (portable — required for deferred web).
`PointList` = 1px, no size control. Storage-buffer pulling is WebGPU-only (it's the
`storage_instancing` feature, not default). One draw of N instances structurally beats
sprite-batching engines (bevymark ~100–130k via batching, not instancing). Rejected: per-entity
draws (CPU-bound >10k); geometry/compute (WebGPU-only, overkill). Trade-off: instance buffer
re-uploaded each frame (it changes anyway); mitigated by D6 sizing + `write_buffer` (D5).

### D5 — Per-frame upload via `queue.write_buffer`, not StagingBelt.
Choice: one `queue.write_buffer(&instance_buf, off, cast_slice(col))` per matched archetype, in
`CallbackTrait::prepare` (has `&Device`,`&Queue`). Why: `write_buffer` is the
single-large-contiguous-upload path; the SoA column is exactly one large contiguous slice per
archetype. StagingBelt is for many small scattered writes (the opposite). Rejected: StagingBelt
(wrong shape + lifecycle for no benefit); `mapped_at_creation` per frame (re-creates buffer →
alloc churn). Trade-off: `write_buffer` does an internal staging copy (standard). Double/triple
buffer ONLY if profiling shows an upload stall (deferred until measured).

### D6 — Instance buffer: grow-once to a cap, no per-frame rotation.
Choice: allocate once at `max_instances` (default cap 1_048_576, possibly per-mode); each frame
upload `count` ≤ cap; draw `0..count`. Grow (recreate larger, round up to power of two) only if
`count` exceeds capacity (rare, on a big click-spawn burst). Why: the buffer changes wholesale each
frame but its SIZE is near-constant; a single stable allocation avoids per-frame `create_buffer`
(allocation in the frame loop is forbidden, principle 5) and keeps the vertex binding stable;
power-of-two growth bounds reallocs to O(log cap). Rejected: ring/rotation (only helps a stall not
yet observed; adds bookkeeping); exact-fit recreate each frame (alloc churn). Trade-off: ≤2× VRAM
headroom transiently after growth (16 B/inst × 1M = 16 MB; 2× = 32 MB — negligible).

### D7 — WebGPU with WebGL2 fallback (broad reach).
Choice: target WebGPU first; keep the pipeline WebGL2-compatible (this is WHY D4 avoids storage
pulling). Native: default backend. Web: eframe picks WebGPU, falls back to WebGL2. Why: WebGPU is
not universal in May 2026; WebGL2 is; the instance-vertex path is the common subset → one shader,
no fork. Broad reach matters for a promo artifact. Rejected: WebGPU-only (cuts visitors);
WebGL2-only (leaves perf on the table). Trade-off: forgoes WebGPU-only features in the default path
(available behind `storage_instancing` natively).

### D8 — GPU handles live in the app shell / `callback_resources`, never as ECS `Res<>`.
Choice: `wgpu::Device`/`Queue`/`RenderPipeline`/buffers in the eframe `App` struct and egui's
`callback_resources`; the ECS world holds NO GPU handle. Why: (a) keeps the ECS core a pure
headless, GPU-agnostic library; (b) wasm main-thread hazard — wgpu handles + surface are
main-thread/event-loop bound; putting them in `Res<>` invites worker access via a system
(unsound on wasm). The ECS produces DATA (`GpuInstance` columns); the shell consumes it. Rejected:
`Res<RenderContext>` (couples GPU into the world, risks worker access). Trade-off: the upload code
lives in the shell and reaches the world read-only once per frame in `prepare`.

### D9 — Fixed-timestep accumulator (sim/render decoupled).
Choice: sim advances in fixed `dt` (default 1/60 s) via an accumulator; render every `App::update`
at display rate; cap sub-steps (e.g. 5) against the spiral-of-death. Why: deterministic, stable
physics/boids independent of refresh/hitches; decouples draw rate from integrate rate. Rejected:
variable timestep (unstable for forces/collisions); fixed = render rate (a 144 Hz monitor would 4×
the sim cost for no visual gain). Trade-off: defer inter-state interpolation (render the latest sim
state; imperceptible at 60 FPS). Documented simplification.

### D10 — wasm scheduler: sequential runner on wasm; full pool on native. THE web design driver.
Resolved against the code.
- Native: real `ThreadPool` (`ThreadPoolBuilder::new().build()`, default =
  `available_parallelism`), `Schedule` built with it, `schedule.run(&mut world)` per fixed step;
  `par_iter*` fans out across workers.
- wasm: DO NOT construct a `ThreadPool` and DO NOT use `Schedule::run`. Run per-step system
  FUNCTIONS sequentially via a thin hand-written runner using the `EcsMaster` direct API
  (`world.query::<D,F>()` + `iter*`/`for_each_chunk`). `par_iter*` is never called on wasm (even
  if it were, PAR7 falls back to sequential — `par_iter.rs:261` `try_with_active_pool` → `None`).
Why (from source): `ThreadPoolBuilder::build` (`thread_pool.rs:357`) ALWAYS spawns
`clamp(1, MAX_WORKERS)` OS threads — there is no zero-thread mode, and `num_threads(1)` still
spawns ONE real worker thread (:417). On header-less GitHub Pages (no COOP/COEP → no
SharedArrayBuffer → no wasm threads), an OS thread spawn is unsupported, and the `!Send`/`!Sync`
arena (TLS) + bootstrap `Mutex`/`Condvar` are not viable. `Schedule::run` (`schedule.rs:279`)
enters `pool.install` and may `scope.spawn` onto workers — built around a live pool. So we bypass
it on wasm. The SYSTEMS are plain functions shared across targets; only the dispatch shell differs.
Rejected: `num_threads(1)` (spawns a thread — unsound on wasm); a web-worker + SharedArrayBuffer
polyfill (impossible without COOP/COEP). Trade-off: two dispatch paths behind
`#[cfg(target_arch="wasm32")]`, minimized to one `run_sim_step` with two bodies; the system
functions are shared. Native keeps the full multi-threaded showcase; wasm is single-threaded
(documented).

### D11 — Uniform grid `Resource`, rebuilt each frame, for boids & physics.
Choice: a `SpatialGrid` resource (CSR: `cell_starts: Vec<u32>` + `entity_idx: Vec<u32>`), cell =
neighbor radius (boids) / 2× max radius (physics), rebuilt each frame before the force/collision
pass. Why: naive is O(n²); uniform grid → O(n·k). Flat CSR (counting sort) is cache-friendly and
allocation-stable (two Vecs sized once, refilled — principle 5). The world is a bounded box →
direct array offsets, no hashing, no HashMap (forbidden). Rejected: spatial-hash
`HashMap<cell, Vec>` (both forbidden + churn); k-d/BVH (rebuild + pointer-chasing, worse for
uniform clouds); naive O(n²) (dies past a few thousand). Trade-off: degrades on wildly non-uniform
density; acceptable for a bounded box; cell size is a tuned constant.

### D12 — Boids read a pre-tick position snapshot to dodge read-during-write.
Choice: the boids force pass reads neighbor state from a snapshot (`Vec<BoidState>` resource
refreshed each step), writes new velocities into the live `Velocity` column; the grid indexes the
snapshot. Why: each boid reads neighbors' previous-frame state; reading the live column while a
sibling worker writes it is a conflict. A read-only snapshot makes the force pass a pure `par_iter`
(snapshot+grid read-only; each boid writes only its own row — disjoint). Standard ECS boids
pattern; keeps the force pass parallel. Rejected: double-buffered components (2× columns + swap);
reading live state (race → scheduler serialization → kills the par_iter win). Trade-off: one O(n)
snapshot copy/step (sequential, cheap) + ~N×16 B snapshot, reused.

### D13 — Physics broad-phase reuses the grid; narrow-phase circle-circle; showcases `Changed<T>`.
Choice: balls integrate, broad-phase via D11 grid, narrow-phase circle-circle, resolve by impulse
(restitution) + wall bounce; an optional `tint_collided` system uses `Changed<Velocity>` to flash
recently-collided balls. Why: reuses D11 (no second structure); circle-circle is the cheapest
correct narrow-phase; `Changed<T>` is a real Phase-10 feature worth showing. NOTE: `Changed<T>`
works only inside a SCHEDULER system (the direct `query()` API panics on change-detection — see
G2/§3 corrections); `tint_collided` runs in the schedule, so it's fine. Trade-off: single-pass
collision resolution is approximate (no global solver); acceptable for a visual sandbox.

### D14 — egui paint ordering: full-window scene with floating panels.
Choice: the instanced scene fills the entire `CentralPanel` rect via the custom paint callback;
controls live in a floating `egui::Window` (or `SidePanel`) on top. Sim mouse input read from the
central panel's `Response`; egui consumes input over its widgets (`ctx.wants_pointer_input()`).
Why: a full-window field is the impressive promo choice; a floating panel keeps controls accessible
without stealing pixels; `wants_pointer_input()`/`response.hovered()` cleanly separate "over the
sim" from "over a widget" (correct routing, no double-handling). Rejected: scene in a small Rect
(wastes pixels); opaque docked panel over scene (hides particles). Trade-off: the window overlaps
some particles (collapsible).

### D15 — Mode switching via `NextState<Mode>` + condition-gated spawn/despawn systems.
CORRECTED to boyko's actual Phase-17 shape: boyko implements states as condition-gated systems, NOT
separate Bevy-`OnEnter` schedules. `on_enter`/`on_exit`/`on_transition`/`in_state` are
run-conditions returning `impl System<Out=bool>` (`common_conditions.rs:82/106/129/151`).
- Native: a mode button writes `NextState<Mode>` via `world.resource_mut::<NextState<Mode>>()`;
  `Schedule::run` AUTO-applies the transition pass each frame (`schedule.rs:251`,
  `run_state_transitions`, gated on registered `state_entries`). Spawn/despawn systems are ordinary
  systems in the SAME schedule, gated `.run_if(on_enter(Mode::X))` / `.run_if(on_exit(Mode::X))`;
  per-mode sim systems gated `.run_if(in_state(Mode::X))`. Register the state via
  `builder.insert_state(Mode::Particles)` (or `init_state` if `Default`).
- wasm: same `State`/`NextState` resources; the sequential runner explicitly checks `NextState`,
  performs spawn/despawn, and only calls the active mode's system functions (the `in_state` /
  `on_enter` predicates evaluated inline). Identical observable behavior.
Why: exactly what the owner specified (drive the sandbox with Phase-17 states honestly), matched to
the engine's real machinery. Trade-off: spawn/despawn-on-transition is expressed as
`on_enter`/`on_exit`-gated systems (Bevy parity via run-conditions, not separate schedules).
G10 resolved: no manual transition system; the builder registration + `Schedule::run` handle it.

### D16 — `Mode` membership marker component for despawn-on-exit.
Choice: each sim entity carries a 1-byte tag matching its mode (`ParticleTag(u8)` etc.).
`on_exit(mode)`-gated despawn system removes all entities with that tag via
`world.query_entities(&[ParticleTag::component_id()]) -> Vec<Entity>` (`ecs_master.rs:1390`,
`&self`), then `delete_entity` each. Why: archetypes carry no intrinsic "which mode" tag; a marker
makes "despawn the previous mode" one query + loop. Trade-off: ZST markers are UNSUPPORTED
(`component_pool.rs:108` debug-asserts size>0), so use `ParticleTag(u8)` (1 B), not a true ZST
(G3).

## 4. Module / file layout (`crates/boyko_demo/`)
```
crates/boyko_demo/
├── Cargo.toml
├── index.html              # web only (Wave 7)
├── Trunk.toml              # web only (Wave 7)
└── src/
    ├── main.rs             # native eframe::run_native; wasm WebRunner start (cfg)
    ├── app.rs              # DemoApp: impl eframe::App; owns world+runner(+pool native);
    │                       #   update() = input→sim step→egui panel→register paint callback
    ├── config.rs           # SimParams, SimConfig (caps/consts), Mode enum
    ├── prelude.rs          # local re-exports of deep boyko paths (G0 ergonomics, demo-only)
    ├── render/
    │   ├── mod.rs
    │   ├── instance.rs      # GpuInstance (#[repr(C)] Pod) + QuadVertex + vertex layouts
    │   ├── pipeline.rs      # RenderState: pipeline, quad v/i buffers, instance buf, camera
    │   │                    #   uniform+bind group; ensure_instance_capacity (grow-once)
    │   ├── callback.rs      # ScenePaintCallback: egui_wgpu::CallbackTrait prepare/paint
    │   └── shaders.wgsl     # vertex applies instance pos/scale/color; fragment round dot
    ├── sim/
    │   ├── mod.rs
    │   ├── components.rs     # Position, Velocity, Radius, tags (+ GpuInstance re-export)
    │   ├── bundles.rs        # #[derive(Bundle)] Particle/Boid/Ball bundles
    │   ├── resources.rs      # InputState, SimParams, SpatialGrid, BoidSnapshot, FrameStats,
    │   │                    #   DeltaTime, mode rosters
    │   ├── grid.rs           # SpatialGrid layout + CSR rebuild
    │   ├── runner.rs         # native: build Schedule; wasm: run_sim_step_sequential
    │   ├── modes.rs          # Mode enum (States impl); spawn_/despawn_ per mode
    │   └── systems/
    │       ├── mod.rs
    │       ├── common.rs     # apply_input, spawn_on_click, sync_gpu_instance, collect_stats
    │       ├── particles.rs  # integrate_particles (par_iter_mut) + gravity well
    │       ├── boids.rs      # snapshot, build_grid, boid_forces (par_iter), integrate
    │       └── physics.rs    # integrate_balls, build_grid, collide (broad+narrow), tint_collided
    └── ui/
        ├── mod.rs
        └── panel.rs          # egui controls → SimParams / NextState<Mode>; FPS egui_plot
```
`render/` is GPU-only; `sim/` is ECS-only; `ui/` is egui-only; `app.rs` is the single seam.
`prelude.rs` mitigates G0 (no boyko prelude) by aliasing deep paths once.

## 5. The render pipeline

### 5.1 `GpuInstance` Pod component (exact layout)
Pod rules: `#[repr(C)]`, NO padding bytes (pad explicitly), all fields Pod, no interior mutability.
```rust
// render/instance.rs
use bytemuck::{Pod, Zeroable};
/// Per-instance GPU data. ECS component column AND instance vertex buffer (D2). 32 B, no padding.
/// offsets: 0 pos[f32;2] | 8 size:f32 | 12 _pad0:f32 | 16 color[f32;4]; size 32, align 4.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuInstance {
    pub pos: [f32; 2],
    pub size: f32,
    pub _pad0: f32,
    pub color: [f32; 4],
}
```
- 32 B = two per cache line; small column (1M = 32 MB). `_pad0` explicit (Pod forbids uninit pad);
  shader ignores it. `GpuInstance` derives `Component` (boyko) AND `Pod` (bytemuck) — both operate
  on a plain struct; coexistence verified in Wave 2 (G1).
- Alternative 24 B layout: `pos:[f32;2]; size:f32; color_rgba8:u32` (color as 4×u8), Pod, saves
  8 B/inst (1M→24 MB), unpack color in the vertex shader. DECISION: start 32 B (shader simplicity,
  richer color); switch to 24 B if the arena budget (G6) is tight at the target N. Pick in Wave 2.

### 5.2 Vertex layouts
Buffer 0 — unit quad (per-vertex, static):
```rust
#[repr(C)] #[derive(Clone, Copy, Pod, Zeroable)] struct QuadVertex { corner: [f32; 2] } // step_mode: Vertex
// layout: stride 8, attr [loc0 Float32x2 off0]; index buffer [0,1,2, 2,1,3]; draw_indexed(0..6,...)
```
Buffer 1 — instances (`GpuInstance`, per-instance, dynamic):
```rust
// stride 32, step_mode Instance, attrs:
//   loc1 Float32x2 off0  (pos)
//   loc2 Float32   off8  (size)
//   (off12 _pad0 skipped)
//   loc3 Float32x4 off16 (color)
```

### 5.3 WGSL (`shaders.wgsl`)
```wgsl
struct Camera { view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> camera: Camera;
struct VsIn {
  @location(0) corner: vec2<f32>,
  @location(1) i_pos:  vec2<f32>,
  @location(2) i_size: f32,
  @location(3) i_col:  vec4<f32>,
};
struct VsOut { @builtin(position) clip: vec4<f32>, @location(0) color: vec4<f32>, @location(1) uv: vec2<f32> };
@vertex fn vs_main(in: VsIn) -> VsOut {
  let world = in.i_pos + in.corner * in.i_size;
  var out: VsOut;
  out.clip = camera.view_proj * vec4<f32>(world, 0.0, 1.0);
  out.color = in.i_col;
  out.uv = in.corner;
  return out;
}
@fragment fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let r2 = dot(in.uv, in.uv);
  let alpha = in.color.a * smoothstep(1.0, 0.85, r2);
  return vec4<f32>(in.color.rgb, alpha);
}
```
Round dot via `uv` = texture-less + pretty (no asset pipeline). Pipeline uses
`BlendState::ALPHA_BLENDING`. WebGL2-compatible (no storage buffers, no compute).

### 5.4 `CallbackTrait` flow (`render/callback.rs`)
```rust
impl egui_wgpu::CallbackTrait for ScenePaintCallback {
    fn prepare(&self, device, queue, _screen, _encoder, resources) -> Vec<CommandBuffer> {
        let rs: &mut RenderState = resources.get_mut().unwrap();
        rs.update_camera(queue, self.view_proj);              // tiny uniform write
        rs.ensure_instance_capacity(device, self.total_count);// D6 grow-once
        let mut off = 0u64;                                   // D2/D5 zero-copy upload
        (self.run_upload)(&mut |slice: &[GpuInstance]| {
            debug_assert!(off + std::mem::size_of_val(slice) as u64 <= rs.instance_buf_bytes);
            queue.write_buffer(&rs.instance_buf, off, bytemuck::cast_slice(slice));
            off += std::mem::size_of_val(slice) as u64;
        });
        Vec::new()
    }
    fn paint(&self, _info, pass, resources) {
        let rs: &RenderState = resources.get().unwrap();
        pass.set_pipeline(&rs.pipeline);
        pass.set_bind_group(0, &rs.camera_bind_group, &[]);
        pass.set_vertex_buffer(0, rs.quad_vbuf.slice(..));
        pass.set_index_buffer(rs.quad_ibuf.slice(..), IndexFormat::Uint16);
        pass.set_vertex_buffer(1, rs.instance_buf.slice(..));
        pass.draw_indexed(0..6, 0, 0..self.total_count);     // ONE draw (D4)
    }
}
```
`run_upload` is the D8 boundary: built in `app.rs` while the App holds `&mut world` (update() owns
it), it calls `world.query::<&GpuInstance, ()>().for_each_chunk(|s| sink(s))` and immediately
`cast_slice`+`write_buffer`. Sound: `prepare` runs inside egui paint, still within `update()`'s
scope; the sim step finished; only a read happens. `query::<&GpuInstance,()>` is safe via the
direct API because `&GpuInstance` needs no change detection (the API panics only on CD — see G2).
`total_count` = Σ archetype `entity_count` (computed once).

### 5.5 Camera / resize
Each `update`: read central rect; build an orthographic `view_proj` mapping the sim box → clip
(optional pan/zoom via wheel + drag in empty space); upload the tiny uniform in `prepare`. No
per-frame allocation.

## 6. The ECS sim

### 6.1 Components (`sim/components.rs`)
All `#[repr(C)]` + `#[derive(Component)]`; hot ones also `Pod` where uploaded.
```rust
#[repr(C)] #[derive(Component, Clone, Copy, Pod, Zeroable)] pub struct Position { pub x: f32, pub y: f32 } // 8 B
#[repr(C)] #[derive(Component, Clone, Copy, Pod, Zeroable)] pub struct Velocity { pub x: f32, pub y: f32 } // 8 B
#[repr(C)] #[derive(Component, Clone, Copy, Pod, Zeroable)] pub struct Radius(pub f32);                    // 4 B
// GpuInstance: §5.1.
#[repr(C)] #[derive(Component, Clone, Copy)] pub struct ParticleTag(u8);  // 1 B (NOT ZST; G3)
#[repr(C)] #[derive(Component, Clone, Copy)] pub struct BoidTag(u8);
#[repr(C)] #[derive(Component, Clone, Copy)] pub struct BallTag(u8);
```
Each component is its own column (SoA automatic). No within-struct hot/cold split needed (each is
already minimal). Position/Velocity/GpuInstance are hot; Radius/tags colder.

### 6.2 Bundles (`sim/bundles.rs`)
`#[derive(Bundle)]` on NAMED structs (tuple/unit/generic bundles rejected — confirmed by
`bundle_compile_fail/*`):
```rust
#[derive(Bundle)] pub struct ParticleBundle { pub pos: Position, pub vel: Velocity, pub gpu: GpuInstance, pub tag: ParticleTag }
#[derive(Bundle)] pub struct BoidBundle     { pos: Position, vel: Velocity, gpu: GpuInstance, tag: BoidTag }
#[derive(Bundle)] pub struct BallBundle     { pos: Position, vel: Velocity, radius: Radius, gpu: GpuInstance, tag: BallTag }
```

### 6.3 Resources (`sim/resources.rs`)
All `#[derive(Resource)]` (confirmed macro exists, `boyko_macros/src/lib.rs:301`).
```rust
#[derive(Resource)] pub struct InputState { pub cursor_world: Option<[f32;2]>, pub primary_down: bool, pub spawn_click: bool }
#[derive(Resource)] pub struct SimParams  { pub gravity: f32, pub damping: f32, pub max_speed: f32,
    pub boid_sep: f32, pub boid_align: f32, pub boid_coh: f32, pub boid_radius: f32,
    pub restitution: f32, pub target_count: u32, pub spawn_burst: u32, pub paused: bool /* ... */ }
#[derive(Resource)] pub struct DeltaTime(pub f32);   // G8: no built-in Time; set before each run
#[derive(Resource)] pub struct FrameStats { pub frame_ms: f32, pub sim_ms: f32, pub entity_count: u32,
    pub history: [f32; 240], pub head: usize }       // fixed ring (no per-frame alloc)
#[derive(Resource)] pub struct SpatialGrid { /* §6.4 */ }
#[derive(Resource)] pub struct BoidSnapshot { pub state: Vec<BoidState> }   // reused buffer
#[repr(C)] #[derive(Clone, Copy)] pub struct BoidState { pos: [f32;2], vel: [f32;2] }
```

### 6.4 `SpatialGrid` (CSR uniform grid, `sim/grid.rs`)
```rust
pub struct SpatialGrid {
    pub origin: [f32;2], pub cell: f32, pub dims: [u32;2],
    cell_starts: Vec<u32>,  // len = dims.x*dims.y + 1 (CSR offsets); sized once, refilled
    entity_idx:  Vec<u32>,  // len = entity_count; archetype row index
}
```
Rebuild (counting sort, O(n)+O(cells), allocation-stable):
1) `cell_starts.fill(0)` (reuse capacity; resize only if `dims` changed). 2) histogram:
`cell_starts[cell+1] += 1`. 3) prefix-sum → CSR offsets. 4) scatter row index into
`entity_idx[cursor[cell]++]`. Cache: pass 1 reads Position sequentially; histogram fits L1/L2;
pass 2 is the one scatter (O(n) once). No HashMap, no per-cell Vec — two flat Vecs refilled.
Neighbor query: walk the 3×3 (boids) / 5×5 (physics) cell block; each cell is
`entity_idx[cell_starts[c]..cell_starts[c+1]]`. Built by a `build_grid` system (sequential
dispatcher pass; sole grid writer that frame → no conflict with the read-only force pass).

### 6.5 Systems & gating
System functions are plain `fn` (registered into the native `Schedule`; called by the wasm runner).
Ordering via `.before`/`.after`/`.in_set`.
Common (every mode): `apply_input` (reads InputState, primes well; first), `sync_gpu_instance`
(`par_iter_mut` `(&Position,&mut GpuInstance)` [+`&Velocity` for color, +`&Radius` for size];
after all integration, before the §5.4 upload), `collect_stats` (writes FrameStats; last).
Particles (`.run_if(in_state(Mode::Particles))`): `integrate_particles` (`par_iter_mut`
`(&mut Position,&mut Velocity)`: gravity-well force toward `InputState.cursor_world` when
`primary_down` (inverse-square clamped), damping, clamp `max_speed`, `pos += vel*dt`, wall bounce;
O(n), one predictable branch/row, SIMD-friendly). Color by speed folded into `sync_gpu_instance`.
Boids (`.run_if(in_state(Mode::Boids))`): `snapshot_boids` (copy `(Position,Velocity)`→
`BoidSnapshot`, sequential, reused; first) → `build_grid` (from snapshot; D11) → `boid_forces`
(`par_iter_mut` `(&Position,&mut Velocity)`; grid-neighbor query over the read-only snapshot,
accumulate sep/align/coh, write Velocity) → `integrate_boids` (`par_iter_mut`
`(&mut Position,&Velocity)`).
Physics (`.run_if(in_state(Mode::Physics))`): `integrate_balls` (`par_iter_mut`) → `build_grid`
→ `collide_balls` (grid neighbor pairs, circle-circle (Σ radii), impulse resolution + overlap
separation + wall bounce; a pair touches two rows so this CANNOT be a naive disjoint-row
`par_iter` — run SEQUENTIALLY on the dispatcher for correctness; parallel-by-cell-coloring is a
stretch goal, G12) → `tint_collided` (optional; `Query<&mut GpuInstance, Changed<Velocity>>` —
runs INSIDE the schedule so `Changed<T>` is valid; flashes collided balls).
`par_iter` on native (honest dogfooding): `integrate_*`, `boid_forces`, `sync_gpu_instance` are
disjoint-row parallel and exceed `MIN_ARCHETYPE_FOR_PARALLEL=1024` at the demo's N → genuinely fan
out. wasm calls the sequential `iter_mut`/`for_each_chunk` equivalents (D10).

### 6.6 Mode transition spawn/despawn (`sim/modes.rs`)
```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] pub enum Mode { Particles, Boids, Physics }
impl boyko_ecs::ecs::core::state::states::States for Mode {}   // marker trait, no methods
```
- Register: `builder.insert_state(Mode::Particles)` (or impl `Default` + `init_state`).
- Spawn-on-enter: `add_system(spawn_particles).run_if(on_enter(Mode::Particles))`; spawn body uses
  `Commands::spawn_batch(iter_of_ParticleBundle)` (confirmed `commands.rs:242`,
  `I: IntoIterator<Item=B>`) for the 10k–1M initial population (Phase-12.5/12.6 warm path).
  Fallback if awkward: loop `Commands::spawn(bundle)` (`:158`) or setup-time
  `world.create_entity`.
- Despawn-on-exit: `add_system(despawn_particles).run_if(on_exit(Mode::Particles))`; body:
  `let ids = world.query_entities(&[ParticleTag::component_id()]); for e in ids { world.delete_entity(e); }`
  (`query_entities` :1390 is `&self`, returns `Vec<Entity>`). This is the despawn-by-tag path that
  sidesteps G7 (Entity is not a query term). All spawn/despawn runs in the dispatcher apply window
  (G11) — never from a `par_iter` body.
- Switch: a mode button writes `NextState<Mode>`; `Schedule::run` auto-applies the transition pass
  (`schedule.rs:251`), then the `on_exit(old)` despawn + `on_enter(new)` spawn systems fire that
  frame.

### 6.7 Timestep driver (`sim/runner.rs`)
```rust
// Native:
pub struct SimRunner { schedule: Schedule, accumulator: f32 }
impl SimRunner {
    pub fn step(&mut self, world: &mut EcsMaster, frame_dt: f32, fixed_dt: f32) {
        if world.resource::<SimParams>().paused { return; }
        self.accumulator += frame_dt.min(MAX_FRAME_DT);
        let mut sub = 0;
        while self.accumulator >= fixed_dt && sub < MAX_SUBSTEPS {
            world.resource_mut::<DeltaTime>().0 = fixed_dt;
            self.schedule.run(world);            // REAL multi-threaded schedule
            self.accumulator -= fixed_dt; sub += 1;
        }
    }
}
// wasm: same accumulator; call run_sim_step_sequential(world, &mut state) instead of schedule.run.
```
`fixed_dt` from `SimParams` (default 1/60). Systems read `DeltaTime` (G8).

## 7. egui control panel (`ui/panel.rs`)
Drawn each `update` after the scene callback is registered (panels paint over the scene, D14). All
widgets mutate RESOURCES via `world.resource_mut::<_>()` (the App owns `&mut world`), picked up next
step with zero plumbing.

| Widget | Writes | Effect |
|---|---|---|
| Mode buttons | `NextState<Mode>` (native) / explicit transition (wasm) | switch → on_exit/on_enter |
| target_count slider | `SimParams.target_count` | maintain population |
| gravity/damping/max_speed | `SimParams.*` | particles |
| boid sep/align/coh/radius | `SimParams.*` | boids (shown in Boids) |
| restitution | `SimParams.restitution` | physics (Physics) |
| "click spawns N" | `SimParams.spawn_burst` | click-to-spawn count |
| pause toggle | `SimParams.paused` | runner skips run |

Readouts (entity count, ms/frame, sim ms, FPS) from `FrameStats`. FPS graph: `egui_plot::Plot`
fed `FrameStats.history` (fixed ring) as a frame-time `Line` over the last 240 frames (no per-frame
alloc).

Mouse mapping (`app.rs`, before the sim step):
1. central panel `Response`; if `ctx.wants_pointer_input()` → `cursor_world=None`,
   `primary_down=false` (egui owns it).
2. else map `response.hover_pos()` → world via the inverse camera; `cursor_world=Some(world)`.
3. `primary_down = response.dragged() || pointer.primary_down()` (well while held).
4. `spawn_click = response.clicked()` (one-shot; `spawn_on_click` consumes it, spawns
   `spawn_burst` at `cursor_world`).

## 8. wasm plan (deferred, designed-for)

### 8.1 What must NOT be precluded (and how this satisfies it)
- Single-thread scheduler path → D10: systems are plain functions; the wasm runner calls them
  sequentially; NO `ThreadPool` constructed; NO `Schedule::run`; `par_iter` not called (PAR7 would
  fall back anyway).
- No GPU-in-Res → D8: GPU handles in the shell; eframe owns the surface (main thread).
- WebGPU + WebGL2 → D4/D7: instance-vertex pipeline, no storage/compute in the default path; one
  shader.
- No SharedArrayBuffer dependency → nothing in the demo (or the core on the sequential path) needs
  shared memory; the arena stays single-threaded on wasm by construction (never handed to a pool).

### 8.2 Build/deploy (Wave 7)
`index.html` with `<canvas>`; `Trunk.toml`; `trunk build --release` → `dist/`. `main.rs` wasm entry:
`console_error_panic_hook::set_once()`, `console_log` init,
`wasm_bindgen_futures::spawn_local(eframe::WebRunner::new().start(...))` on the canvas. Deploy:
extend the existing Pages CI (`.github/workflows/docs.yml`) with a job that runs `trunk build` and
publishes `dist/` to `/demo/`. `getrandom` `js` feature wires `rand` entropy.

### 8.3 Explicitly deferred on wasm
Multi-threading (impossible on header-less Pages); the `storage_instancing` fast path; any OS-thread
feature. wasm targets correctness + the visual, single-threaded, at a lower N (e.g. 50k–200k).

### 8.4 Divergence surface (minimal — two cfg seams)
1) `sim/runner.rs`: `schedule.run` (native) vs `run_sim_step_sequential` (wasm) — system functions
shared. 2) `main.rs`: `eframe::run_native` vs `WebRunner::start`; env_logger vs console_log; pool
construction (native only). Everything else (render, components, systems, ui) is identical.

## 9. API-gap log (dogfooding findings; all confirmed against source)
None require a core change — each has a demo-side workaround. Starred = candidate follow-up ECS work.

| # | Gap / friction | Status | Workaround (no core edit) | Follow-up? |
|---|---|---|---|---|
| G0 | No prelude; `boyko_ecs::lib.rs` re-exports only `EcsError`/`EcsResult`. Types live at deep paths (`boyko_ecs::ecs::core::component::component::Component`, `...::system::params::commands::Commands`, etc.); derives in `boyko_macros`. | Confirmed (`lib.rs`). | Demo `prelude.rs` aliases the deep paths once. | *A `boyko_ecs::prelude`.* |
| G1 | `#[derive(Component)]` + bytemuck `Pod` on the same struct. | Should coexist (Component derive needs only a type id). | Verify compiles in Wave 2; else hand-impl `Component`. | — |
| G2 | Read-only upload + change-detection rule. `query::<D,F>()` is `&mut self` (`:2391`) AND PANICS if `D`/`F` need change detection. | Confirmed. | App holds `&mut world` in `update()`; call `world.query::<&GpuInstance,()>().for_each_chunk(..)` (no CD). `Changed<T>` only inside scheduler systems. | *A `&self` `query_ref`.* |
| G3 | ZST marker components. | Confirmed UNSUPPORTED (`component_pool.rs:108` size>0). | 1-byte tags `ParticleTag(u8)` (D16). | *ZST component support.* |
| G4 | `#[derive(Resource)]`. | Confirmed EXISTS (`boyko_macros:301`). | Use it directly. | — |
| G5 | `Commands::spawn_batch` for bulk population. | Confirmed (`commands.rs:242`, `I: IntoIterator<Item=B>` → `EcsResult<SpawnBatchIter>`); single `Commands::spawn(bundle)` at `:158`. | Use `spawn_batch` for the initial 10k–1M; fallback loop `spawn` / setup-time `create_entity`. | — |
| G6 | Arena budget + pool capacity at high N. 64 MB arena (`DEFAULT_ARENA_SIZE`). Per-column pool sizes to `DEFAULT_CHUNKS_PER_POOL × per_chunk`; per-row tick buffers are `Box` (NOT arena, `component_pool.rs:80`). Physics entity ≈ Position8+Velocity8+Radius4+GpuInstance32+tag1 ≈ 53 B + SIMD-align gaps. | Confirmed mechanics; capacities to MEASURE. | Wave 2: measure per-column footprint; CONFIRM default pool capacity reaches the target N per column, else use `with_capacity` / lower the cap per mode (e.g. particles 1M, boids 250k, balls 100k). | *Configurable arena size / pool capacity.* |
| G7 | `Entity` is NOT a query term. | Confirmed (no `impl QueryData for Entity`). | Despawn-by-tag via `world.query_entities(&[Tag::component_id()]) -> Vec<Entity>` (`:1390`, `&self`) → `delete_entity` each. Spawn returns `Entity` from `create_entity`/`spawn_one`. | *`Entity`/`EntityRef` as `QueryData`.* |
| G8 | No built-in `Time`/`DeltaTime`. | Confirmed absent. | Demo `#[derive(Resource)] DeltaTime(f32)`; set before each `run`. | *Optional `Time`.* |
| G9 | No built-in input resource. | Expected (headless). | Demo `InputState` (§6.3). | — |
| G10 | Phase-17 transition application timing. | RESOLVED: `Schedule::run` auto-runs the transition pass for registered states (`schedule.rs:251`). | Register via `builder.insert_state(..)`; no manual transition system. | — |
| G11 | Despawn during a mode switch vs in-flight entities. | `delete_entity` is `&mut self`; transitions + apply window are dispatcher-only. | All spawn/despawn in `on_enter`/`on_exit`-gated systems (apply window) — never from a `par_iter` body. | — |
| G12 | Parallel collide (pair writes). | `par_iter` needs disjoint rows; a pair touches two. | Wave 6 ships SEQUENTIAL `collide_balls` (correct). Cell-colored parallel is a stretch. | *Doc note on pair-mutation patterns.* |

These are honest engine-maturity findings (no prelude, no Time/input, ZST gap, capacity knobs,
Entity-not-a-term). The demo ships entirely on the existing public API via the workarounds.

## 10. File-by-file implementation plan + waves
Legend: [P] parallelizable with wave-siblings (different files, no sequential dep); [S] sequential.

Wave 0 — pin & skeleton (precedes everything)
- 0.1 [S] root `Cargo.toml`: add `crates/boyko_demo`.
- 0.2 [S] `Cargo.toml` with `eframe=0.34.3`; `cargo tree -p eframe -i wgpu`; pin `wgpu="=<resolved>"`; commit the verified pin.
- 0.3 [S] `main.rs` + `app.rs`: minimal `eframe::App`, empty `CentralPanel` + "hello" `Window`. `cargo run -p boyko_demo` shows a window. `prelude.rs` stub (G0).

Wave 1 — wgpu under egui (prove the seam)
- 1.1 [P] `render/instance.rs`: `GpuInstance` + `QuadVertex` + vertex layouts.
- 1.2 [P] `render/shaders.wgsl`: §5.3.
- 1.3 [S] `render/pipeline.rs`: `RenderState` (pipeline, quad buffers, a tiny static instance buffer with a few hardcoded instances, camera uniform + bind group).
- 1.4 [S] `render/callback.rs`: `ScenePaintCallback` drawing the hardcoded instances (paint only; prepare updates camera); register via `egui_wgpu::Callback` in `CentralPanel`.
- 1.5 [S] `app.rs`: store `RenderState` in `callback_resources` (eframe `CreationContext.wgpu_render_state`). Milestone: colored quads under an egui window.

Wave 2 — instanced particles from a static buffer (prove instancing + upload)
- 2.1 [S] `pipeline.rs`: `ensure_instance_capacity` (D6) + CPU `Vec<GpuInstance>` of ~100k random instances.
- 2.2 [S] `callback.rs`: `prepare` uploads the Vec via `write_buffer(cast_slice(..))`; `paint` `draw_indexed(0..6,0,0..N)`. Milestone: 100k instanced quads, one draw. Decide 32 B vs 24 B here (G6).
- 2.3 [P] `config.rs`: `SimParams`, `SimConfig`, `Mode` stub.

Wave 3 — wire the ECS (prove SoA→GPU zero-copy + real schedule)
- 3.1 [P] `sim/components.rs`, `sim/bundles.rs` (resolve G1/G3).
- 3.2 [P] `sim/resources.rs` (G4, `DeltaTime` G8, `InputState`).
- 3.3 [S] `sim/systems/particles.rs` (`integrate_particles`) + `sim/systems/common.rs` (`sync_gpu_instance`).
- 3.4 [S] `sim/runner.rs` (native): `ThreadPool` + `ScheduleBuilder` (`add_system(integrate)`, then `sync_gpu_instance.after(integrate)`) + `SimRunner` accumulator (G5).
- 3.5 [S] `app.rs`: own `EcsMaster` + `SimRunner` (+ pool); spawn 100k particles at startup (MVP); each `update`: `runner.step(&mut world, dt, fixed)`.
- 3.6 [S] Replace Wave-2 static upload with the ZERO-COPY path: `prepare` → `world.query::<&GpuInstance,()>().for_each_chunk(|s| write_buffer)` (G2). Milestone (MVP): ECS-driven 100k+ particles, real `Schedule::run`, real `par_iter_mut`, zero-copy upload, fixed timestep.

Wave 4 — egui controls + mouse + FPS plot
- 4.1 [P] `ui/panel.rs`: sliders/toggles → `SimParams`; readouts; `egui_plot` FPS (§7).
- 4.2 [P] `sim/systems/common.rs`: `apply_input` + `spawn_on_click` + `collect_stats`.
- 4.3 [S] `app.rs`: mouse→`InputState` mapping (§7); `FrameStats` ring update. Milestone: mouse well + click-to-spawn + live sliders + FPS graph.

Wave 5 — boids + grid + state switch
- 5.1 [P] `sim/grid.rs`: `SpatialGrid` + CSR rebuild (§6.4).
- 5.2 [P] `sim/systems/boids.rs`: snapshot, build_grid, boid_forces (`par_iter`), integrate; `BoidSnapshot`.
- 5.3 [S] `sim/modes.rs`: `Mode` `States` impl; `builder.insert_state`; spawn/despawn systems gated `.run_if(on_enter/on_exit(Mode::X))`; per-mode sim systems `.run_if(in_state(Mode::X))`; mode buttons → `NextState<Mode>` (G6/G7/G11). Milestone: switch Particles↔Boids live, each spawns/despawns its set, boids flock.

Wave 6 — physics
- 6.1 [P] `sim/systems/physics.rs`: integrate_balls, build_grid (reuse), collide_balls (sequential, correct), wall bounce; `tint_collided` via `Changed<Velocity>` (in-schedule).
- 6.2 [S] `sim/modes.rs`: Physics mode spawn/despawn + gating + button. Milestone: three-mode sandbox; balls collide; `Changed<T>` flash.

Wave 7 — web (last)
- 7.1 [P] `index.html`, `Trunk.toml`, `main.rs` wasm entry (§8.2).
- 7.2 [S] `sim/runner.rs`: `#[cfg(wasm32)]` `run_sim_step_sequential` (D10) — shared system functions called in dependency order, sequentially; no pool.
- 7.3 [S] CI: extend Pages workflow → `trunk build --release` → `/demo/`. Milestone: browser sandbox (single-threaded), deployed.

Parallel summary: W1 (1.1‖1.2), W2 (2.3 ‖ 2.1→2.2), W3 (3.1‖3.2), W4 (4.1‖4.2), W5 (5.1‖5.2),
W7 (7.1 ‖ 7.2 prep).

## 11. Verification

### 11.1 Builds & runs
- `cargo build -p boyko_demo` clean; `cargo run -p boyko_demo` opens a window, scene renders, panel
  responds, mode switch works, mouse well visibly attracts particles.
- `cargo clippy -p boyko_demo -- -D warnings` clean.
- Core untouched: `git status` shows changes only under `crates/boyko_demo/`, one root `Cargo.toml`
  line, and `docs/`. `cargo check -p boyko_ecs` / `cargo test -p boyko_ecs` identical before/after
  adding the member (the demo only depends on core; it cannot affect core compilation).
- Wave 7: `trunk build --release` succeeds; `dist/` loads in a WebGPU browser and a WebGL2-only
  browser.

### 11.2 Performance targets (native)
- Primary: ≥ 100k particles at 60 FPS (16.6 ms) on a typical dev GPU (bevymark-class, structurally
  beaten by single-draw instancing).
- Stretch: 1M particles ≥ 30 FPS (sim may limit; upload+draw stays sub-ms — one ~32 MB
  `write_buffer` + one draw).
- Sim scaling: `FrameStats.sim_ms` must DROP as worker count increases (proves the scheduler +
  `par_iter` do real parallel work, not faking it).
- No per-frame heap alloc on the hot path: verify via the demo-only `alloc_audit` global-allocator
  counting shim, OR by inspection (no `Vec::new`/`format!`/`create_buffer` in the frame loop;
  grow-once buffer, fixed FrameStats ring, reused grid/snapshot Vecs).
- Zero-copy upload assertion (debug): in `prepare`, assert the slice handed to `write_buffer` lies
  within the pool buffer range (no intermediate Vec materialized) — a `debug_assert!` documenting D2.

### 11.3 Behavioral checks (`crates/boyko_demo/tests/`)
- `sim_smoke.rs`: headless (no window) — build `World`+`Schedule`, spawn N particles, run K steps,
  assert positions advanced and stayed in-box (sim independent of rendering).
- `grid.rs`: build `SpatialGrid` over known points; assert neighbor queries return the right
  buckets (CSR correctness); `cell_starts.last() == entity_idx.len()`.
- `mode_switch.rs`: drive `NextState<Mode>` headless; assert the previous mode's tagged entities
  despawn and the new mode's spawn.

### 11.4 `debug_assert!` invariants to embed
- `const _: () = assert!(size_of::<GpuInstance>() == 32 && align_of::<GpuInstance>() == 4);`
- Upload: `debug_assert!(off + len_bytes <= instance_buf_bytes)` before each `write_buffer`.
- Grid: `debug_assert!(cell < dims.x*dims.y)` after clamp; `cell_starts.last() == entity_idx.len()`.
- Substep cap: `debug_assert!(sub <= MAX_SUBSTEPS)`.
- Zero-copy provenance (§11.2).

## 12. Open questions (for the critic / user)
1. Default max N per mode vs the 64 MB arena + pool capacities (G6): 1M particles target, or cap
   particles ~500k / physics ~250k? Finalize from the Wave-2 measured per-column footprint.
   (Leaning: particles 1M, boids 250k, balls 100k — physics is heaviest per-entity.)
2. 24 B packed-color vs 32 B float `GpuInstance` (§5.1): decide in Wave 2 from VRAM headroom.
   Default 32 B unless the arena is tight.
3. Parallel collide (G12): sequential in Wave 6; is cell-colored parallel collision in scope as a
   stretch, or explicitly out?
4. `for_each_chunk` borrow shape from the App's `&mut world` in `prepare` (G2): confirm in Wave 3
   it composes with the egui callback's lifetime; if not, materialize the upload offsets/lengths in
   `update()` (still no intermediate AoS Vec — just record `(offset, len, archetype)` triples).
5. Default pool worker count on native: `available_parallelism` (full showcase) vs a capped value
   to leave a core for the OS/render thread? Lean to full; expose as a `SimConfig` knob.

=== END docs/DEMO-PLAN.md ===

This plan is ready for `architecture-critic`. After the file is written by the orchestrator, the critic can review at `D:\claude\BoykoEngine\docs\DEMO-PLAN.md`.