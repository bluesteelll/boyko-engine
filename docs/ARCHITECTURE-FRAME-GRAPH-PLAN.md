# Industrial Frame System — Render Dependency Graph + Sim/Render Interpolation (PLAN)

> **STATUS: BUILDING (owner `/goal делай весь RDG по плану`, 2026-07-02).**
> Architect→critic loop converged (2026-06-28); owner greenlit the whole 1a→1f sequence.
>
> ## BUILD LOG
> - **Step 1a — DONE (impl + static-verified; visual-OK gate still owner's).** Array-batched the 5
>   `count=1` barrier LOOPS in `swapchain::record_gbuffer` into sync1 array-form `vkCmdPipelineBarrier`
>   (15 barrier calls → 5). Byte-identical GPU semantics by construction (N consecutive same-stage
>   barriers over non-aliasing resources ≡ one array call). `cargo check` + `clippy -D warnings` green.
> - **Step 1b — DONE (CPU-verified, GPU-free, code-reviewer-hardened).** New module
>   `crates/boyko_rhi_vulkan/src/framegraph/` (`ids`, `sync`, `graph`, `record`) + equivalence gate
>   `tests/framegraph_gbuffer_equiv.rs` (7 tests green). The graph auto-derives the minimal barrier
>   plan from declarative pass/resource accesses via a Granite per-resource sync state machine
>   (`sync::transition`); `record` lowers it into batched sync1 `vkCmdPipelineBarrier` array calls via
>   a `BarrierSink` seam (real sink resolves ResId→VkImage, test sink counts). The gate proves
>   SOUND-SUPERSET equivalence to `record_gbuffer`'s hand inventory: layout trajectories (from the
>   ground-truth compiled sync state), producer→consumer hazard coverage (incl. the marcher→SSAO
>   store→load + the WSI swapchain acquire→color→present), exact barrier count (23 image + 5 buffer),
>   W5 optional-additivity, and **C6 quantified honestly: the record step emits exactly 18 array
>   calls == the post-1a hand path (PARITY — the graph fuses some batches, splits others, nets zero;
>   its win is auto-derivation + correctness + enabling history-rotation/aliasing, NOT fewer calls).**
>   The test caught a real soundness bug (first-touch write not recording its flush → `src_access=0` =
>   stale-read UB). **code-reviewer verdict: the sync engine is SOUND for this frame** (every
>   RAW/WAR/WAW/layout hazard derived; optimistic-but-never-unsound on multi-read); its blockers were
>   all in the test (present modeled as blit not fragment-sample; missing swapchain resource; circular
>   count) — all fixed. Does NOT drive the GPU (zero deletion risk).
>
> - **Step 1c — IMPLEMENTED (compiler-verified, pending owner GPU visual gate).** The framegraph now
>   DRIVES the leading raster barrier-in batch of `record_gbuffer`, behind a DEFAULT-OFF
>   `Renderer::use_framegraph` flag (`set_use_framegraph`). `RasterBarrierSink` (swapchain.rs) resolves
>   each derived `ImgBarrier.res` → the current-slot physical `VkImage` (`targets.{albedo,normal,
>   material,depth}[fi].image`, ResId order pinned in `Renderer::new`) and records the batched
>   `vkCmdPipelineBarrier`. The leading-raster `FrameGraph` is compiled ONCE in `Renderer::new` (the
>   barrier plan is frame-invariant). **Flag OFF (default) keeps the Step-1a hand code verbatim in the
>   `else` — byte-identical live path.** Flag ON emits the identical 2 calls (color group + depth
>   group). `cargo check` + `clippy -D warnings` + framegraph tests green; **code-reviewer APPROVED**
>   (exhaustive field-by-field trace: flag-OFF byte-identical, flag-ON≡flag-OFF proven, `from_fn` tail
>   sound, `res.index()∈0..4` guaranteed, SAFETY accurate; only 2 cosmetic 🟢).
>   **Owner action: `set_use_framegraph(true)` on the RTX viewer to visually confirm flag-ON renders
>   identically, then it can become default.** Cannot be runtime-verified without a GPU.
>
> - **Steps 1d+1e — IMPLEMENTED (compiler-verified + ADVERSARIALLY REVIEWED CLEAN; pending owner GPU).**
>   The framegraph now DRIVES the ENTIRE `record_gbuffer` frame's barriers behind the DEFAULT-OFF flag
>   (`GbufferPassPlan` + `GbufferBarrierSink` [9 images + 5 buffers] + `declare_gbuffer_graph`
>   [per-frame, config-gated] + `record_graph_pass`; all 10 barrier sites wired). `record_pass` is
>   recorded immediately before each pass's GPU work (execution-order contract — NOT the old hand site,
>   which would order a not-yet-issued write). The WSI swapchain barriers (7,9) + pass C stay
>   hand-recorded in both modes. **1d's history/FIF is the [fi]-resolver; 1e's layered barriers are
>   `depth_layers`+SubRange — both already in the 1c sink.** A 3-lens adversarial review workflow
>   (declaration-exactness / placement-ordering / soundness+flag-OFF+sink, each finding adversarially
>   verified) returned **0 confirmed defects** (1 raised finding — a `light_cull` graph-vs-record gate
>   asymmetry — rejected: agrees under the documented `cull_layout⟺cluster_cull` invariant; ON≡OFF even
>   if violated). `cargo check --workspace` + `clippy -D warnings` + framegraph tests green; flag-OFF
>   byte-identical. **KNOWN defensive nicety (not a defect): align the graph's `light_cull` gate to the
>   record gate (also key on `cull_layout`) when 1f touches this code.**
>
> ## THE AUTONOMOUS CEILING — reached at 1a–1e; ONLY 1f remains (structurally owner-GPU-gated)
> **1a+1b+1c+1d+1e are BUILT + verified + adversarially-reviewed-clean, all behind the DEFAULT-OFF
> flag (live path byte-identical, commit-safe).** Beyond the leading-raster batch the graph path is
> SOUND-SUPERSET (D-A3): identical RENDERING, different command stream — its result-correctness is
> verifiable ONLY by the owner's GPU pixels + validation-layer cleanliness. **The one remaining step,
> 1f (delete `record_gbuffer`'s hand barriers + the ring + unify the 4 skeletons), STRUCTURALLY requires
> the owner's GPU visual first** — it removes the byte-identical hand FALLBACK and ships the
> un-GPU-validated sound-superset graph path as the ONLY path; deleting the reference before the GPU
> confirms flag-ON renders identically would be reckless (+ violates "commit render changes only after
> visual OK"). **Owner action: `set_use_framegraph(true)` on the RTX viewer → confirm the frame renders
> identically + validation-clean → then 1f deletes the hand path.** Pillar B (interpolation, the jitter
> fix) is the GPU-free-buildable parallel track meanwhile.
>
> ## ARCHITECTURE DECISIONS taken during build (mine per "decide architecture yourself"; owner may veto at checkpoint)
> - **D-A1 Crate = `boyko_rhi_vulkan`, NOT `boyko_render`.** Barriers reproduce `record_gbuffer`'s exact
>   `Vk*` masks/layouts and are verified against it in-crate. A backend-agnostic `boyko_render` layer
>   would need the full graphics-pipeline stage/layout surface bolted onto `boyko_rhi`'s buffer-only
>   `BarrierStage`/`BarrierAccess` + a re-lowering pass — a speculative abstraction for a single
>   (Vulkan) backend, against Principle 0. Diverges from the plan's file-map; resolves C3.
> - **D-A2 Substrate = build-time preallocated `Vec`s** (the `boyko_render::barrier` precedent), not
>   `VmReservation` (which stays `pub(crate)` in `boyko_ecs`). Zero per-frame alloc via `reset` +
>   re-declare. Resolves C4 without boyko_ecs surgery.
> - **D-A3 Gate = sound-superset + hazard coverage**, NOT byte-identical (plan open-decision #1 / C5):
>   the machine places barriers at true first-use; the hand path batches some eagerly.
> - **D-A4 Ordering = linear** (declaration order = execution order; the frame is a straight line).
>   The alloc-free `u16` topo/SCC (C4) is deferred until a genuinely branching frame exists (YAGNI /
>   Principle-0 anti-speculation). The per-resource sync machine — the real industrial win — is done.
>
> Architect→critic loop converged (2026-06-28). The critic returned 6 blockers (C1–C6), all resolved
> below; C3/C4/C5 additionally settled by D-A1..D-A4 above.

## Why this exists

The owner: "СРАЗУ сделать готовую промышленную систему... изучи все варианты... разработай наилучшую
систему." Replace the ad-hoc per-slot G-buffer ring (the just-shipped motion-jitter fix) + the
~18–30 hand-authored, unbatched `vkCmdPipelineBarrier(count=1)` calls in `record_gbuffer` with a
proper **industrial Render Dependency Graph**, and add the **sim/render interpolation** that today
exists ONLY in the wgpu demo (the in-house Vulkan path renders instantaneous state → judders under
load).

## Verified research basis (deep-research, 25/25 adversarial claims confirmed)

The proven production pattern (Frostbite FrameGraph / O'Donnell GDC2017, UE5 RDG, AMD RPS,
Themaister/Granite) is three-phase **SETUP → COMPILE → EXECUTE**:
- SETUP: passes declare per-resource reads/writes; create virtual resources.
- COMPILE: topologically order passes (recurse backward from the backbuffer to leaf producers);
  compute per-transient liveness (first-write..last-read); **auto-derive** minimal Vulkan
  stage+access+layout barriers; batch co-located barriers.
- EXECUTE: record per-pass command lambdas.

Key verified facts:
- **Cross-frame / history resources** (our G-buffer case) are handled by a **separate persistence
  mechanism**, NOT transient aliasing: mark has-history, exclude from the alias pool, and **rotate
  by swapping the attachment + its sync events each frame** (Granite `physical_image_has_history[]`,
  UE extracted/pooled, RPS `RESOURCE_FLAG_PERSISTENT`). Subsumes the ad-hoc ring; no mandatory VRAM
  doubling.
- **Transient aliasing** (~40–50% transient VRAM via disjoint-liveness → shared allocation, with an
  aliasing barrier) is a **separate opt-in compile pass** (UE `r.RDG.TransientAllocator`, RPS flag,
  Granite `build_aliases`). Phase 2.
- **Async compute** (timeline semaphores, producer signals value V, consumer waits ≥V) is the last
  opt-in layer. Phase 3.
- **Pacing** (Fiedler "Fix Your Timestep!"): fixed-dt accumulator; render interpolates prev→curr by
  `alpha = accumulator/dt` as `mix(prev,curr,alpha)`. Interpolation (1-frame latency, always smooth)
  > extrapolation. Spiral-of-death clamp = max accumulated time / max substeps.

## Current state (audit, file:line)

- Frame loop `render_gbuffer_frame` (swapchain.rs:2174); FRAMES_IN_FLIGHT=2 (:63). Waits ONLY
  `frames[frame_index].in_flight` at start (drains the submit **2 frames back = N−2**, NOT the
  sibling N−1). FOUR near-duplicate frame skeletons (738/1032/1566/2174).
- `record_gbuffer` (:2380): ~18 hand barriers plain / ~30+ fully-on, each a separate
  `cmd_pipeline_barrier(count=1)`; self-flagged "correct-but-unbatched; P3a batches later" (:2329).
- Just-shipped per-slot ring: `GBufferTargets` images `[VulkanTexture; FRAMES_IN_FLIGHT]` (:5245),
  descriptor sets ringed + written once, recorder `[fi]`-indexed. This is a **pipelining** ring
  (its doc :5248-5251 states verbatim it exists so frame N+1 writes `depth[1]` while frame N reads
  `depth[0]`). `csm_cascade_texture`/`shadow_atlas_texture` left single (world-fixed).
- ZERO transient aliasing; `ssao` allocated even when off (dead alloc). Present FIFO-only (:403).
- Viewer paces by FAKE `VIEWER_DT=1/60` (tests/window_present_gbuffer.rs:5179), renders
  instantaneous CPU state — NO interpolation.
- Interpolation substrate EXISTS but unused in-house: `fixed_advance` (fixed_loop.rs:51),
  `FixedTime::overstep_fraction()` (fixed_time.rs:141 = THE alpha). `mix(prev,pos,alpha)` lives ONLY
  in boyko_demo (wgpu). `boyko_rhi_vulkan` has ZERO refs to it.

## Architecture — the RDG design (decisions)

- **D1 ECS-native storage (Principle 0 — critic CONFIRMED this is the right side of the line):** the
  graph is a `!Send+!Sync` **NonSend resource** owning **index-based SoA arenas on one
  `VmReservation`** (the same GPU-contiguity/transient-scratch exception as the threadpool deques +
  `barrier.rs`'s build-time Vec). Passes are NOT ECS entities (transient, rebuilt every frame, no
  entity identity, walked backward — wrong access pattern for archetype iteration). Durable render
  inputs (transforms/instance columns) stay in `ComponentPool`/dense. Reached only via
  `DispatcherToken` like `RhiContext` → data-race-free by the &mut-projection proof.
- **D2 Granite per-resource sync state machine** (`{layout, to_flush_access,
  pipeline_barrier_src_stages, invalidated_in_stage[16]}`) for MINIMAL barriers + free read-combine,
  over UE5 dependency-levels (which over-synchronize).
- **D3 Batch per pass boundary.** (CORRECTED — see C2: batching works in **sync1** array form; sync2
  is a separate opt-in.)
- **D4 History-rotation** for value-carrying cross-frame resources (swap slot + sync state).
  (CORRECTED — see C1: slot-count is ORTHOGONAL to value-carry.)
- **D5 Resource lifetime classes:** Transient / History / Persistent / Imported.

## Critic blockers → resolutions (all verified against code)

- **C1 (KEYSTONE, R3) — "Transient = 1 slot" would REINTRODUCE the jitter race.** The ring is a
  *pipelining* ring; the `in_flight[fi]` fence drains N−2, not the sibling N−1, so a within-frame
  G-buffer image collapsed to ONE physical slot races frame N read vs N+1 write again. A **static**
  golden CANNOT catch this (a stopped scene fills all slots identically). **RESOLUTION:** slot-count
  must derive from the **liveness-vs-FIF** relation, ORTHOGONAL to the History/Transient value-carry
  axis. Every current G-buffer image needs **FIF slots** (per-`frame_index`). Do NOT delete the ring
  until the graph reproduces FIF-slotting AND passes a **MOVING-scene** golden.
- **C2 (R1) — sync2 is DISABLED at the device** (`synchronization2: VK_FALSE`, device.rs:1883;
  ffi.rs:2133/2151), not just un-FFI'd — using `vkCmdPipelineBarrier2` without enabling = UB.
  **RESOLUTION:** the batching win does NOT need sync2 (sync1 `vkCmdPipelineBarrier` already takes a
  barrier ARRAY; today's code just loops `count=1`). Make **sync1 array-batching the default**;
  sync2 is a later opt-in behind a runtime support query.
- **C3 — `barrier.rs` is a DIFFERENT subsystem** (build-time ECS conflict-graph → buffer-column
  lowering, keyed by durable `(ArchetypeId, ComponentId)`; no image/layout/cmd-buffer concept). NOT
  extensible to image barriers. **RESOLUTION:** image-barrier derivation is a **NEW** `framegraph/`
  module; only factor shared `BarrierStage`/`BarrierAccess` mapping helpers into `boyko_rhi`.
- **C4 — reused primitives are NOT free.** `tarjan_scc`/`kahn` (schedule_builder.rs:896/993) are
  private, `SystemKey`-keyed, AND **allocate** (`vec![Vec::new(); n]`) → breaks the "0 alloc"
  promise; `VmReservation::base/commit/os_len` are `pub(crate)`. **RESOLUTION:** write
  framegraph-local, **arena-backed, allocation-free** topo/SCC over `u16` indices (the graph is
  tiny); specify the real public `VmReservation` bump-alloc API needed; re-derive "0 alloc".
- **C5 — the "sound-superset" 0%-gate can PASS on a buggy graph** (misses an implicitly-ordered
  barrier the hand path never made explicit) AND contradicts the "byte-identical" goal.
  **RESOLUTION:** pick ONE contract: (a) **byte-identical** (hard — the Granite minimal machine must
  be tuned to reproduce every hand barrier's exact masks/layout/subresource) OR (b)
  **sound-superset + a sync-validation oracle** (Vulkan sync2 validation or a CPU race-model) + drop
  "byte-identical". EITHER WAY the gate MUST use a **moving** scene (to catch C1-class races).
- **C6 — quantify the count win honestly.** Part of the reduction (e.g. the 3-image loop at :2424) is
  achievable in **plain sync1** array form with NO graph. **RESOLUTION:** produce before/after
  barrier-*call* counts: (a) today, (b) after trivial sync1 array-batching, (c) with the graph. The
  graph must beat **(b)**, not (a). Some barriers are necessarily mid-pass (not one-per-boundary).

### Important (W1–W6, fold into the design)
- W1: per-frame arena reuse without re-zero → stale-SoA hazard; memset the live slice OR carry a
  per-entry frame-epoch. Document the invariant.
- W2: split `ResourceSyncState` hot (`layout`+`to_flush_access`+`src_stages`, ~16B) vs cold
  (`invalidated_in_stage[16]`); state the barrier-derivation loop's iteration order (sequential SoA);
  back the "≤2µs" with an op-count/microbench.
- W3: **CSM/atlas are 2D arrays** — barriers carry a runtime multi-layer `VkImageSubresourceRange`
  (:3313-3375). Subresource ranges (aspect+base-layer+layer-count+mip) must be **first-class** in
  `ResAccess` from Phase 1, or CSM stays on the hand path (then `record_gbuffer` can't be deleted).
- W4: **Phase 1 is too large** and front-loads the risky ring deletion → the re-sequencing below.
- W5: optional passes (SSAO/CSM/L1 off) must emit EXACTLY zero commands and not perturb neighbors'
  layouts — gate per optional pass (off-graph == off-hand, byte-identical).
- W6: enumerate every value the current closures capture (`fi`, `targets`, `scene`, extents,
  `active_count`, `ssao_mode`) and show `RecordCtx` resolves them by index without a fat god-struct;
  a small `fn(&mut RecordCtx, &PassParams)` (params in the cold arena) beats a huge `RecordCtx`.

## THE DE-RISKED INCREMENTAL PLAN (the approved sequencing — ring deletion LAST)

| Step | What | Gate |
|------|------|------|
| **1a** | **sync1 array-batching** of the EXISTING hand barriers — NO graph (collapse the count=1 loops into array-form `vkCmdPipelineBarrier`). | Byte-identical command stream; offscreen goldens bit-identical. Ships the headline count-reduction win + de-risks C2/C6. |
| **1b** | Introduce the graph module in **PARALLEL behind a flag**, recording into a **capture-sink only** (does NOT drive the GPU); diff its derived barriers + draw stream vs the LIVE hand path on a **MOVING** scene. | Equivalence proven with ZERO deletion risk. |
| **1c** | Flip the graph to DRIVE a **non-CSM, non-history** subset of passes. | Moving-scene golden bit-identical for that subset. |
| **1d** | Add **history-rotation + per-FIF slotting** (C1) so the graph reproduces the ring's cross-frame coverage. | MOVING-scene golden — no jitter regression. |
| **1e** | **CSM/atlas layered-array** subresource barriers (W3). | Moving golden incl. shadows. |
| **1f** | **Delete `record_gbuffer` + `GBufferTargets` ring** + unify the 4 frame skeletons — **LAST**, gated on the moving-scene golden + sync-validation. | The ring is the reference impl until the graph provably matches it. |

Cross-cutting: framegraph-local alloc-free `u16` topo/SCC; image-barriers a NEW module; subresource
ranges first-class; the gate uses a MOVING scene + (byte-identical OR superset+sync-validation).

## Pillar B — sim/render interpolation (PARALLEL, independent of A)

1. Promote the demo's 24B 2D `GpuInstance{pos,scale,color,prev_pos}` (boyko_demo/render/instance.rs:45)
   to a kernel **dense non-fragmenting component** `GpuTransform3D { curr, prev }` (Mat3x4 ×2, or
   pos+quat+scale ×2 for slerp) in a `DenseStore`; the existing `for_each_chunk` SoA→GPU upload blits
   it. `prev` written only by a per-substep shuffle (mirror the demo's `with_prev`).
2. Feed `overstep_fraction()` (fixed_time.rs:141) as a `frame_alpha` push-constant; GPU does
   `mix(prev,curr,alpha)` / `slerp(prev_q,curr_q,alpha)` — authored through **boyko_shaderdsl** (the
   standing byte-identity rule).
3. Spiral-of-death clamp in the fixed loop (accumulator ≤ max_substeps·dt).
4. Viewer: replace `VIEWER_DT=1/60` with real dt + the accumulator; render the interpolated mirror.
   FIFO+interp baseline; mailbox / `VK_EXT_present_*` low-latency later opt-in.

## Data model (from the architect, corrected) — `crates/boyko_render/src/framegraph/`

`u16` newtype indices `PassId/ResId/PhysId`. SoA on one `VmReservation` (~20KB worst-case, committed
once, `ArenaSlice::len` reset per frame → zero per-frame heap alloc; ~2KB hot working set L1d-resident):
- `PassHot{access_begin,access_count,kind,flags}` (8B) + parallel cold `PassCold{execute: PassFn,
  debug_name}`.
- `ResDesc{class,kind,format,extent_class,flags,phys}` (8B); `ResAccess{res,usage,stage,SUBRESOURCE}`
  (+subresource per W3); `Liveness{first/last_write, first/last_read}` (8B).
- `ResourceSyncState` — SPLIT hot(`layout,to_flush_access,src_stages`) / cold(`invalidated_in_stage[16]`)
  per W2.
- `ImgBarrier2/BufBarrier2` POD; `BarrierBatch{img/buf begin+count}` per pass.
- `history_pair[(curr,prev)]` swapped each frame; per-FIF physical slots for pipelining resources.
Compile: rotate → edges → backward-cull from present roots → alloc-free `u16` topo (cycle-guarded) →
liveness → Granite barrier state machine (barrier iff `to_flush_access!=0` OR layout-mismatch OR
not-yet-visible-in-stage; else dropped) → batch. Record: one array-`vkCmdPipelineBarrier` (sync1
default) per pass + invoke `PassFn = fn(&mut RecordCtx)` (bare fn, no `Box<dyn>`).

## Open decisions (owner/architect to settle before/at Step 1d–1f)

1. **Gate contract:** byte-identical vs sound-superset+sync-validation? (C5) — recommend
   superset+sync-validation on a moving scene (the Granite machine won't naturally byte-match).
2. **Slot model:** "Transient" redefined as ALWAYS FIF-slotted, or per-resource proof that 1 slot is
   safe? (C1) — recommend always-FIF for G-buffer images (the safe answer).
3. **CSM in Phase 1** (migrate with layered subresource barriers) vs keep on the hand path until a
   later step? (W3)
4. **sync2** now (enable+query+sync1 fallback) or defer entirely to a later opt-in? (C2) — recommend
   sync1-only for Steps 1a–1f.

## Task breakdown (Step 1a first)

1a: array-batch the existing `record_gbuffer` barriers (sync1) + assert byte-identical goldens.
Then 1b: `framegraph/{ids,access_table,graph,compile,history,record}.rs` + the capture-sink + the
moving-scene diff harness. Then 1c–1f per the table. `boyko_ecs`: expose alloc-free topo (or write
framegraph-local). `boyko_rhi_vulkan`: array-form barrier calls (1a), optional sync2 FFI later.

## File map
- `crates/boyko_rhi_vulkan/src/swapchain.rs` — `record_gbuffer`:2380, hand barriers :2424-3924,
  `render_gbuffer_frame`:2174, 4 skeletons 738/1032/1566/2174, `GBufferTargets` ring :5245, fence
  :702-709/:847, CSM layered :3313-3375, FIFO :403, sync2-off device.rs:1883/ffi.rs:2133.
- `crates/boyko_render/src/barrier.rs` — column-edge lowering (do NOT extend; factor helpers only).
- `crates/boyko_ecs/.../schedule_builder.rs:896/993` — private allocating tarjan/kahn (do NOT reuse
  per-frame as-is). `.../memory/vm.rs` — `VmReservation` (pub(crate) surface). `.../time/fixed_time.rs:141`
  — `overstep_fraction()`. `.../component/dense/dense_store.rs` — Pillar-B column.
- `crates/boyko_demo/src/render/instance.rs:45` — the 24B prev/curr mirror to generalize.
- NEW: `crates/boyko_render/src/framegraph/`.
