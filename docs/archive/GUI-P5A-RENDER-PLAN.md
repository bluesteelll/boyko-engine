# Architecture: GUI Phase P5a — Render UI Rects (instanced rounded-rect SDF)

> **Implementation status (2026-06-21).** The first commit lands **P5a FOUNDATION
> ONLY — Rungs 0–2 + the shared `record_ui_rects` draw recorder**: the RHI blend
> capability, the GPU-proven SSBO-in-graphics combination (Rung 0.5), `UiBackground`,
> the std430 `UiInstance` + `UiOrtho` (layout oracle), the CPU pack / reused scratch /
> O(1) z-sort gate, the POD-by-value `UiFramePlan` carrier, the two HLSL shaders +
> dxc-compiled `.spv`, and the generic one-draw recorder.
>
> **Deferred to a follow-up commit (Rungs 3–5):** the `RhiContext` UI capability
> (`ui_setup` / `ui_upload` / `ui_handles` + the owned `UiRenderResources` sub-owner
> with wired `Drop`/`destroy_all`), the per-frame-in-flight host-mapped STORAGE ring +
> per-FIF bind-groups + grow-on-overflow path (Decisions 7, 8), the `UiUploadSystem`
> (Rung 4), and the `present_sampled` swapchain wiring (Rung 5 step 13). Until those
> land, **nothing uploads a `UiInstance` or draws a UI rect end-to-end**, and the
> cross-frame `!Send` handoff *mechanism* (as distinct from the sound `UiFramePlan`
> carrier) is UNEXERCISED and MUST be re-audited end-to-end (+ Miri-TB) when it lands.
> The golden suite G1–G11 depends on Rungs 3–5 and is deferred with them; the
> pure-CPU pack/sort/ortho/premultiply unit + property tests are writable now.

## Goal
Rasterize every laid-out UI node as a crisp, anti-aliased, optionally-rounded, optionally-bordered rectangle on the in-house Vulkan path, reading **only** ECS columns (`ComputedRect`, `StackIndex`, `ComputedClip`, new `UiBackground`), at zero per-frame heap allocation and (steady-state) one draw call.

Performance targets:
- **One `vkCmdDraw(6, N, 0, 0)`** per frame for the common no-coarse-clip case (in-shader clip keeps the batch intact). Coarse scissor runs are out of scope for P5a (Decision 4).
- **Zero per-frame allocation, zero per-frame realloc** in steady state: one persistent-mapped, host-coherent, grow-only instance STORAGE buffer per frame-in-flight; one preallocated CPU pack buffer; capacity grows pow2 only on overflow.
- **CPU build**: `O(N)` SoA pack into a reused scratch `Vec<UiInstance>`, `O(N log N)` stable z-sort in place, one `O(N)` bulk `memcpy` into the mapped current-FIF slot. `N` = visible UI nodes (hundreds–low thousands).
- **D-cache**: `UiInstance` is `#[repr(C)]` POD, std430-pinned (per-field `offset_of!` const-asserts), 56 B; the pack loop streams.
- **GPU**: fixed-function blend + ROP do compositing; the fragment shader is a handful of analytic ops (`sdRoundedBox` + AA + border + clip), no marching.

## Context and constraints
- **Subsystems touched**: `boyko_rhi` (descriptor surface: blend), `boyko_rhi_vulkan` (pipeline blend lowering; swapchain UI pass hook), `boyko_render` (a NEW UI render capability on `RhiContext` + the UI render system + instance ring + pipeline + shaders), `boyko_ui` (new `UiBackground` component), authoring (`ui!`/`.ui` in P2/P3 — only field additions).
- **Invariants preserved**:
  - Principle 0 — no parallel data system. All durable UI render data is ECS columns; the instance ring is a legitimate FFI/GPU-contiguity buffer (the documented exception, identical class to the swapchain images). **The ring + pipeline + bind-groups are owned by a NEW first-class `RhiContext` capability layer (a named owner, not a side store) — see Decision 8 — so its `Drop` is wired and nothing leaks.**
  - Principle 1/5 — no `Box<dyn>`/`HashMap`/`Vec::new()`/realloc in the hot path; preallocate at setup.
  - `RhiContext` stays `!Send + !Sync`; reached only on the dispatcher via `DispatcherToken` (Option C). The UI system is a `GpuSystem`-shaped consumer with EMPTY `Access` and `is_gpu()` → `SystemKind::GpuCompute` (dispatcher-solo).
  - MF-7 — device handles resolved indirectly each frame by durable key; never cache a raw `u64`.
  - Every `unsafe` carries `// SAFETY:`. `expect("invariant: …")` for setup, `debug_assert!` for hot-path invariants.
- **Out of scope** (seams only): text/glyphs (P5b), world-space anchoring (P7), `UiImage` texture sampling (deferred), box-shadow/blur (future), derived/overflow-driven clip production (a later UI phase — P5a *consumes* author-set `ComputedClip` only; see Decision 4). Coarse scissor batching (deferred). Listed in §Out of scope.

---

## Key decisions

### Decision 1: Graphics-pipeline instanced quad, NOT a fullscreen compute marcher
**What**: Render UI as `N` instances of a 6-vertex unit quad through the existing `boyko_rhi` **graphics** pipeline (vertex+fragment, `vkCmdDraw`, dynamic viewport/scissor, blend ON), with the rounded-box SDF evaluated in the **fragment** stage. The compute marcher is reserved for the procedural 3D SDF field only.
**Why**:
- UI is thousands of small axis-aligned analytic rects. Fixed-function rasterization + hardware blend + ROP composite for free. A compute pass would reinvent per-tile rect binning, per-pixel blending, and rasterization — all in dedicated silicon on the raster path. Strictly fewer cycles and far less I-cache/D-cache pressure than a binning marcher.
- The graphics pipeline **already exists and is tested** in `boyko_rhi`/`boyko_rhi_vulkan` (`graphics_triangle`, `graphics_offscreen`, MRT, depth, sampled). The gaps are blend state and per-instance data delivery — small, scoped sub-rungs (Decisions 2, 3), not a new pipeline class.
- The shared asset with the marcher is the *SDF math in the fragment shader*, not the dispatch model — confirmed by WebRender/nical practice and the closest Rust precedent (Bevy `ui.wgsl`).
**Alternatives rejected**:
- *Fullscreen compute composite of rect SDFs*: loses early-Z/blend/ROP, requires hand-rolled binning to avoid `O(pixels × rects)`, and blend ordering must be re-implemented; a net loss for analytic rects (nical explicit). Rejected.
- *Tessellated rounded-rect geometry (Unity UI Toolkit style)*: per-corner arc tessellation explodes vertex count and CPU geometry build, no resolution-independent AA, conflicts with "one quad + instance" / zero-per-frame-alloc. Rejected.
**Trade-off**: Two small RHI capability additions (blend, instance data). Accepted — reusable engine features (P5b text, future 2D), not a per-crate adapter (Principle 0).

### Decision 2: Instance data via a STORAGE buffer indexed by `SV_InstanceID` (vertexless quad), NOT a per-instance vertex binding — a NEW (API-supported, never-yet-exercised) combination, gated independently
**What**: Keep the rung-2 vertexless `SV_VertexID` quad (no vertex buffer, `vertex_layout: None`). Put the per-rect records in a `StructuredBuffer<UiInstance>` (set 0, binding 0, STORAGE, visible at **VERTEX|FRAGMENT**), indexed by `SV_InstanceID`. Draw `draw(6, N, 0, 0)`.
**Why**:
- The RHI's `VertexBufferLayout` hardcodes a single `binding 0, VK_VERTEX_INPUT_RATE_VERTEX` (`rhi_impl.rs:1398–1451`). A per-instance vertex binding means extending `VertexBufferLayout` with `input_rate` + a second binding + a second `VkVertexInputBindingDescription`/attribute lowering — a wide, error-prone vertex-input surface change.
- `VertexFormat` has only `Float32x3/x4` (no `Float32x2`/`Unorm8x4`), so a vertex-buffer path needs new formats anyway; the SSBO sidesteps that (std430 struct, hand-unpacked in HLSL).
- The same record is readable in both stages (vertex for transform, fragment for SDF/clip) without duplication; smaller per-draw state (no `bind_vertex_buffer`).
- **The needed verbs genuinely exist**: `GraphicsPipelineDesc.bind_group_layout` accepts `Some(StorageBuffer)`; `create_bind_group_layout` honors a per-entry `ShaderStage` (so `VERTEX|FRAGMENT` SSBO visibility *is* expressible); `bind_descriptor_set` supports the GRAPHICS bind point (`rhi_impl.rs:2471`).
**CORRECTION vs prior draft (critic C/major)**: this is a **NEW, never-validated combination**, not a reuse. **Verified**: every existing `StorageBuffer` bind-group in `swapchain.rs` (l3808–3863) is bound on `VK_PIPELINE_BIND_POINT_COMPUTE`; **no graphics pipeline in the repo has ever been created with `bind_group_layout: Some(StorageBuffer)`**, and `enums.rs` documents "COMPUTE is the only stage the foundation uses; VERTEX/FRAGMENT are seam." The bits exist; the path has **zero prior GPU-golden coverage**. It is therefore gated by **Rung 0.5** (below) — a trivial graphics pipeline that reads ONE SSBO record **in the VERTEX stage** by `SV_InstanceID` and writes a solid quad, golden-verified on the RTX 3060 with validation = zero messages — *before* any SDF/blend complexity, so a backend stage-flag/descriptor-mismatch surfaces in isolation.
**Alternatives rejected**:
- *Extend `VertexBufferLayout` with `input_rate` + second binding*: larger RHI surface, new `VertexFormat`s, more lowering branches. Defer to a future general-instancing rung. Rejected for P5a.
**Trade-off**: A descriptor-set bind per frame (one buffer binding) instead of `bind_vertex_buffer`. Negligible; one bind-group per FIF slot, each created once (Decision 7).

### Decision 3: Add a minimal `BlendState` to `GraphicsPipelineDesc`; premultiplied alpha is the engine default
**What**: Add `blend: Option<BlendState>` to `GraphicsPipelineDesc` and a small `BlendState`/`BlendFactor`/`BlendOp` enum family to `boyko_rhi::enums`. `None` keeps today's `blend_enable: VK_FALSE` (every existing pipeline unchanged). The UI pipeline passes `Some(BlendState::PREMULTIPLIED_ALPHA)` → `srcColor=ONE, dstColor=ONE_MINUS_SRC_ALPHA, srcAlpha=ONE, dstAlpha=ONE_MINUS_SRC_ALPHA, op=ADD`.
**Why**:
- The Vulkan backend hardcodes opaque blend for **every** attachment (`rhi_impl.rs:1524–1537`); translucent UI and anti-aliased edges are impossible without it. The single load-bearing prerequisite.
- **Premultiplied** (RmlUi/WebRender) over straight alpha: AA edges over a transparent backdrop fringe/double-darken under straight alpha; premultiplied composes correctly under nested clips and future world-space layering (P7). Coverage-multiplied output stays correct because we emit `rgb*a*cov, a*cov`.
- `Option<BlendState>` (not a per-target slice) is the minimal change: UI has one color attachment. MRT-per-target blend is a non-goal here.
**Alternatives rejected**:
- *Per-target `ColorTargetState` slice*: future-proofs MRT blend but G-buffer passes are opaque and need none; adds slice-plumbing for zero current consumers. A future rung can widen `Option<BlendState>` → slice without breaking the UI single-target use. Rejected for P5a.
- *Straight alpha*: works (ImGui/Bevy) but inferior for layered/clipped/world-space UI. Rejected as default.
**Trade-off**: A new public RHI enum family + one descriptor field + a backend lowering branch. Small, reusable, additive (`None` = byte-identical existing behavior).

### Decision 4: In-shader clip rect is the `ComputedClip` mechanism; coarse scissor is OUT OF SCOPE for P5a
**What**: Carry the clip AABB in `UiInstance.clip` and a `CLIP_PRESENT` flag bit; fold clip into coverage in the fragment shader **only when the flag is set** (`if (flags & CLIP_PRESENT) cov *= clip_coverage(pos_px, clip, fw)`). Unclipped rects skip the clip entirely via the well-predicted uniform branch (matching the `BORDER_ANY` treatment). Hardware scissor batching is **not implemented in P5a** (deferred).
**Why**:
- In-shader clip keeps **all** rects in one draw call — batch breaks are the dominant draw-call cost (WebRender, nical).
- Supports anti-aliased clip edges and (future) rounded clips; pure scissor is hard-edged.
- **CORRECTION vs prior draft (clip-conditioning/minor)**: a `f32::MAX` "far-box" sentinel risks `Inf` in `pos_px - clip` and ill-conditioned `fwidth`. Replaced by a **`CLIP_PRESENT` flag branch** — unclipped rects do **not** evaluate `clip_coverage` at all, so there is no sentinel arithmetic to ill-condition. When clip *is* present, `clip` holds the real author AABB (always finite, bounded by viewport), so `fwidth` stays well-conditioned.
- **CORRECTION vs prior draft (clip-producer/minor)**: **Verified** `ComputedClip` is AUTHOR-OWNED in P1 (`components.rs:188` — "not computed; P1's overflow policy is 'allow overflow'"). P5a therefore **consumes author-set `ComputedClip` only**; there is no scroll/overflow producer yet (a later UI phase). The Decision-4 rationale is *forward-compatibility + correct author-clip*, not a churn workload P5a produces.
**Alternatives rejected**:
- *Scissor-only (clay SCISSOR_START/END brackets)*: a draw call per clip change; defeats the one-draw target. Rejected.
- *Far-box `f32::MAX` sentinel*: ill-conditioned `fwidth`/`Inf` risk. Rejected for the flag branch.
**Trade-off**: One well-predicted uniform branch per instance. Cheaper than a batch break; zero cost for the unclipped common case.

### Decision 5: Painter's order — stable sort instances by `(StackIndex, append_order)`, single blended pass, depth OFF
**What**: Stable-sort the packed instance array by `StackIndex` ascending (tie-broken by pack-append index, i.e. query traversal order), then draw all in one blended pass with depth test+write OFF. The GPU draws back-to-front in array order; hardware premultiplied blend composites.
**Why**:
- A transparency-heavy HUD overlay needs back-to-front blending; a depth buffer (WebRender front-to-back opaque batching) pays off only with large opaque-heavy UIs and complicates the blended subset. Painter's order + one pass is simpler, correct, matches clay's "sorted command array".
- Stable sort over a **preallocated** buffer is `O(N log N)`, in place, zero alloc. `StackIndex` is the author-owned z key (P1 doc).
**Alternatives rejected**:
- *Depth-buffer opaque/blended split*: needs a depth attachment, a `StackIndex→depth` map, still a blended pass for transparency; more state, no overlay win. Deferred. Rejected for P5a.
**Trade-off**: `O(N log N)` sort each frame the set changes. Negligible at UI scale; skipped by the change gate (Decision/Algorithm A1) when nothing ticked.

### Decision 6: A new `UiBackground` style component; pack DIRECTLY into a reused scratch `Vec<UiInstance>` (NO mirror column, NO per-chunk `cast_slice`)
**What**: Add `UiBackground { color, border_color, corner_radius[4], border_width[4], flags }` (authored, ECS column) to `boyko_ui::components`. The upload system queries `(&ComputedRect, &UiBackground, Option<&ComputedClip>, Option<&StackIndex>)`, packs each node into a preallocated, capacity-stable scratch `Vec<UiInstance>` (`clear()` + `extend`, never `Vec::new`), stable-sorts the scratch in place by `StackIndex`, then **memcpys the whole contiguous buffer once** into the mapped current-FIF ring slot.
**Why**:
- `UiBackground` does not exist yet (P1 has only the layout-inset `UiSpacing.border_*`, a layout concern). The visual style is its own component (Principle 0: a render concern is an ECS column).
- **CORRECTION vs prior draft (self-contradiction/major)**: the prior draft justified a separate `UiInstance` *mirror column* by "reuse the EXACT `for_each_chunk` + `cast_slice` zero-copy path", then designed it away (drop the column; global cross-archetype stable sort). **A per-chunk `cast_slice` blit is mutually exclusive with a global z-sort**: once instances must be reordered by `StackIndex` *across chunks/archetypes*, no archetype column can be blitted straight to the GPU — a single CPU pack buffer must be materialized and sorted, then memcpy'd. So **there is no mirror column and no per-chunk `cast_slice`**. The genuine, honest perf shape is: **`O(N)` pack + `O(N log N)` in-place sort + one `O(N)` bulk `memcpy`, zero alloc steady-state.**
- The transferable discipline from `GpuInstance` is **CPU-side only** (a `#[repr(C)]` POD record + a compile-time size/offset guard + sequential SoA pack). **CORRECTION vs prior draft (precedent/minor)**: **Verified** `GpuInstance` rides **wgpu** in `boyko_demo` (`queue.write_buffer`, a per-vertex/instance *vertex-buffer* stream — `instance.rs`/`app.rs:358`), **not** the in-house Vulkan SSBO path and **not** a persistent host-coherent map. It is **not** evidence the in-house SSBO-graphics path works (that is what Rung 0.5 proves). Only the layout/const-assert/pack idea transfers.
- The mirror column is recorded as the **deferred seam for per-node incremental upload (P5b+)**, not part of P5a.
**Alternatives rejected**:
- *`UiInstance` mirror ECS column*: cannot serve `cast_slice` either (the global sort forbids per-chunk blit), and adds a column + a write per changed node for no P5a benefit (the coarse change gate already gives 0%-when-static). Deferred. Rejected for P5a.
**Trade-off**: The pack/sort/memcpy is one bulk `memcpy`, not per-chunk DMA. At low-thousands instances this is sub-microsecond and the bench gate asserts zero steady-state alloc.

### Decision 7: One persistent-mapped STORAGE ring buffer + one bind-group PER frame-in-flight, each created ONCE; rebuilt ONLY on that slot's grow
**What**: For each of `FRAMES_IN_FLIGHT` (=2): one `HOST_VISIBLE|HOST_COHERENT` `BufferUsage::STORAGE` buffer, mapped once at create and never unmapped, plus one bind-group binding that slot's buffer at set0/binding0. Each is created once at setup. On overflow of slot `f`: fence-wait that slot, destroy+recreate its buffer at `need.next_power_of_two()`, re-map, **and rebuild that slot's bind-group** (the affected slot only). The current-FIF bind-group is selected by `frame_index` each frame and passed to the draw recorder.
**Why**:
- **CORRECTION vs prior draft (bind-group rebind/major)**: the prior draft offered "the single binding rebound to the current FIF ring slot each frame" as a co-equal variant. **This is impossible**: **verified** `create_bind_group` (`rhi_impl.rs:888`) writes the descriptor set ONCE via `vkUpdateDescriptorSets` at create — the code comment is explicit (l899–900): "the set is allocated once … there is NO per-frame rewrite." There is **no update-descriptor-set verb** in the RHI. **Only the one-bind-group-per-FIF-slot variant is valid**, and the rebind variant is **removed entirely**. The grow path therefore *must* rebuild the affected slot's bind-group (an explicit step in A1 step 4).
- Per-FIF double-buffering means a frame's writes never race the previous frame's in-flight reads — gated by the existing per-FIF `in_flight` fence.
**Trade-off**: 2 buffers + 2 bind-groups resident. Trivial; rebuilt only on growth (a setup-class cost).

### Decision 8: A NEW first-class UI render capability on `RhiContext` (owned, Drop-wired) — NOT a reuse of nonexistent `RhiContext` verbs
**What**: Extend `RhiContext` with an owned `UiRenderResources` sub-owner (the per-FIF rings + maps + bind-groups + the UI graphics pipeline + bind-group layout), plus explicit setup/frame methods that **forward through `split_mut().0` (the `&VulkanContext` device)** to the real `RhiDevice` verbs. The new methods mirror how `create_compute_pipeline` forwards to the manager:
- `RhiContext::ui_setup(&mut self, swapchain_format, spirv_vs, spirv_fs) -> Result<…>` — builds the UI pipeline + bind-group layout + per-FIF rings + bind-groups, once.
- `RhiContext::ui_upload(&mut self, packed: &[u8], frame_index: usize) -> Result<UiFramePlan, …>` — ensures the slot's capacity (grow + rebuild bind-group if needed), memcpys into the mapped slot, returns a by-value `UiFramePlan { instance_count, ortho }` (no borrows escape — see Multithreading).
- The on-screen recorder reaches the current-FIF pipeline+bind-group **indirectly by `frame_index`** (MF-7 re-resolution), never a cached raw `u64`.
**Why**:
- **CORRECTION vs prior draft (RhiContext-surface/critical, both reviewers)**: the prior draft said the upload "reuses `RhiContext::split_mut, create_buffer, buffer_mapped_ptr, create_graphics_pipeline, create_bind_group_layout/create_bind_group`." **Verified false for `RhiContext`**: `RhiContext` (`gpu_column.rs:105–243`) exposes ONLY `context()`, `manager()/manager_mut()`, `split_mut()`, `create_compute_pipeline()`, `dispatch_compute()`, `destroy_all()` — it is a **compute-only facade**. `create_buffer` (`device.rs:227`), `buffer_mapped_ptr` (`device.rs:239`), `create_graphics_pipeline` (`device.rs:363`), `create_bind_group_layout` (`device.rs:398`), `map_buffer` (`device.rs:461`), `create_bind_group` (`rhi_impl.rs:888`) are all **`RhiDevice` methods** reached via `split_mut().0`. So P5a must **ADD** a graphics/host-ring capability layer, not "reuse" it.
- **Drop/leak (critic note)**: **Verified** `RhiContext::Drop` (`gpu_column.rs:245–259`) only drains `self.manager`. A ring/pipeline owned *outside* the manager would **leak**. Therefore `UiRenderResources` is owned **as a field on `RhiContext`** with its **own `Drop`** (or an explicit teardown invoked from `RhiContext::destroy_all` *and* its `Drop`, idempotent like the manager). This makes it a first-class kernel capability with wired teardown (Principle 0), not a side store.
**Alternatives rejected**:
- *A free-standing `UiRing` resource owned by the ECS world `NonSend` slab, separate from `RhiContext`*: splits GPU ownership across two owners, risks a teardown-order bug (the device could drop before the ring), and duplicates the `split_mut` device-access discipline. Folding it into `RhiContext` keeps one device owner and one teardown order. Rejected.
**Trade-off**: `RhiContext` grows a UI-specific sub-owner + a handful of methods. Justified — it is the engine's single device-owning facade; UI is a first-class engine system, so its GPU resources belong with the device owner (Principle 0).

### Decision 9: The UI pass opens its OWN `begin_rendering(LoadOp::Load)` at the FULL swapchain extent, swapchain-format pipeline; the upload is a `GpuSystem`-shaped consumer; the cross-frame handoff is by-value, re-resolved
**What**:
1. **Upload** (ECS system, dispatcher-solo, `GpuSystem`-shaped): pack → sort → `RhiContext::ui_upload` → returns a **by-value** `UiFramePlan { instance_count, ortho, frame_index }` stashed in a `NonSend` resource the swapchain step reads. No borrowed RHI handles escape the token projection.
2. **Draw**: recorded into the **same swapchain `cmd`** that `present_sampled` uses, in a **fresh `begin_rendering` scope** (color = swapchain image, `LoadOp::Load`, `StoreOp::Store`, no depth, blend ON), opened **after** the composite scope's `end_rendering` and **before** the COLOR→PRESENT barrier. The recorder re-resolves the current-FIF UI pipeline + bind-group from `RhiContext` **by `frame_index`** (MF-7), binds them, pushes `ortho`, sets viewport+scissor to the **FULL swapchain extent**, and records `draw(6, N, 0, 0)`.
**Why**:
- UI must composite **over** the resolved scene in the **same frame's** swapchain image, before present. Recording into the existing `present_sampled` `cmd` (one submit, one fence) is the only way to get correct ordering and frame pacing without a second submit/sync.
- **CORRECTION vs prior draft (present-scope/major + ortho-space/critical)**: **Verified** `record_present_sampled` (`swapchain.rs:1734–1817`) opens **one** scope with `LoadOp::Clear`, sets viewport+scissor to **`present_extent = min(swapchain_extent, composite.texture_extent)` at origin (0,0)** (l1771–1786) — a 1:1 top-left sub-region — then **`end_rendering`** and barriers COLOR→PRESENT. Two consequences the prior draft glossed:
  - **(a)** The composite scope has **already ended**; the UI pass needs its **own** `begin_rendering` with `LoadOp::Load` (preserve the composite, do **not** re-clear) and its **own viewport+scissor = the FULL swapchain extent** (not `present_extent`), or any UI outside the top-left composite region is scissored away.
  - **(b)** The UI `begin_rendering` uses the **swapchain format**; the UI pipeline's `color_formats[0]` MUST equal the **swapchain surface format** (the W2-b contract), **distinct from** the offscreen `R8G8B8A8` the golden test uses. → **two pipelines** (one swapchain-format for on-screen, one `R8G8B8A8` for the test) sharing one shader blob, OR a format parameter to `ui_setup`. **Chosen: a `color_format` parameter to `ui_setup`** so the on-screen path passes the swapchain format and the test passes `R8G8B8A8`; both build from the same SPIR-V.
- **CORRECTION vs prior draft (ortho-space/critical)**: the ortho denominator MUST be the **extent of the image the UI pass actually renders into** (the swapchain `VkExtent2D`), NOT necessarily `UiViewport`. **Contract (new, explicit)**: the UI pass renders into the **full swapchain extent**, and the ortho is computed from that **same swapchain extent** (passed into `ui_upload`/the recorder), so the rect at `(0,0)` maps to NDC `(-1,+1)` and the rect at `(swap.w, swap.h)` to `(+1,-1)` of the **same** image. `UiViewport.{width,height,scale_factor}` is the **logical→physical authoring space**; the **host is responsible** for keeping `UiViewport` physical extent (`width*scale_factor, height*scale_factor`) equal to the swapchain `VkExtent2D` (it owns both the surface resize and the `UiViewport` resource). When they would diverge (resize mid-frame, WSI `current_extent` clamp), the **ortho uses the swapchain extent the UI pass is recording into** (the authoritative pixel space), so UI is never misplaced relative to the image it draws on; layout reflow to the new logical size is the host's next-frame responsibility. A golden case renders into a target whose extent ≠ the nominal viewport and asserts the bottom-right rect lands at the bottom-right texel.
- The upload is `!Send` GPU work touching `RhiContext`; the established mechanism is a `GpuSystem` (EMPTY access, `is_gpu()`, dispatcher-solo, `DispatcherToken::nonsend_resource_mut::<RhiContext>()`). It runs in the apply window before present.
- **CORRECTION vs prior draft (handoff soundness/critical)**: the prior draft "stashes … the current bind-group" via a `UiDrawData<'a>` borrowing `&'a BindGroup` from the token-projected `RhiContext`. **That borrow cannot legally outlive the `nonsend_resource_mut` projection** (lifetime bound to `&mut token`) to be read later by the swapchain recorder — the same aliasing/lifetime class that killed the Option-C C1/M1 path (`gpu_system.rs:27–43`). **Resolved**: the handoff carries **only POD by value** (`UiFramePlan { instance_count: u32, ortho: UiOrtho, frame_index: usize }`); it borrows **no** RHI handle. The swapchain recorder, running in the **same dispatcher-solo window**, takes a **fresh `&RhiContext`** and **re-resolves** the current-FIF pipeline+bind-group **by `frame_index`** (MF-7) — so a grow that rebuilt the bind-group between upload and draw cannot leave a stale handle, and no `!Send` handle crosses the token drop. SAFETY argument in §Multithreading.
**Alternatives rejected**:
- *Separate UI submit after present's submit*: a second submit + semaphore chain to the same image, more sync, WAR risk on the swapchain image; no benefit. Rejected.
- *Route the on-screen UI draw fully through the RHI trait encoder against the swapchain texture*: the on-screen renderer is intentionally concrete; forcing it through the trait is a larger refactor than P5a warrants. The factored draw-recording fn keeps the test on the trait encoder and the swapchain path on the concrete recorder, sharing the *logic*. Chosen.
- *Borrowing `UiDrawData<'a>` across the token drop*: unsound (see above). Rejected.
**Trade-off**: The draw call lives in `swapchain.rs` (concrete) and re-resolves handles by `frame_index`; the upload in `boyko_render`. Two files, one shared `record_ui_rects(enc, full_area, &UiFramePlan, pipeline, bind_group)` helper. Accepted — mirrors the existing concrete-present / trait-test split.

---

## Data structures

### New ECS component: `UiBackground` (authored style)
```rust
// boyko_ui/src/components.rs
/// Visual fill + border style for a node. AUTHOR-OWNED, OPT-IN (absent ⇒ the node
/// is layout-only / invisible). Read by the P5a pack system together with
/// ComputedRect. POD Copy; its own SoA column; the change gate covers it.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct UiBackground {
    pub color: u32,             // STRAIGHT RGBA8 authored; premultiplied at pack. byte0=R..byte3=A
    pub border_color: u32,      // STRAIGHT RGBA8
    pub corner_radius: [f32; 4],// tl, tr, br, bl — order matches sdRoundedBox select
    pub border_width: [f32; 4], // l, t, r, b (logical px); 0 = no border that side
    pub flags: u32,             // reserved (P5a derives UiInstance.flags at pack)
}
// Default = transparent fill, no border, zero radius (a node with default
// UiBackground is invisible — authors opt in to a visible fill/border).
```
Size: `u32×2 + f32×4 + f32×4 + u32 = 8 + 16 + 16 + 4 = 44 B`, `#[repr(C)]` align 4, no tail pad → **44 B**. Const-asserted.

### New ECS component: `UiBorder` scope decision (P5a = UNIFORM border only)
**CORRECTION vs prior draft (per-side border/major)**: the prior A4 packed per-side `border_width` as `u8×4` but the fragment math collapsed to a single inner inset ("approximation; per-side via `max(out,-in)`"), simultaneously claiming "exact corner handling to avoid #17561" AND "approximation" — contradictory, and per-side widths cannot be expressed by a single uniformly-inset rounded box. **Resolved: P5a ships UNIFORM border width only.** `UiBackground.border_width[4]` is authored per-side for forward-compat, but **P5a uses `border_width[0]` (a single uniform width) and `debug_assert!`s the four sides are equal** (or takes the max with a documented note). The shader uses one inner-inset rounded box (exact, no approximation, no #17561 corner bleed for the uniform case). **Asymmetric per-side borders are deferred** to a later phase with the correct per-side inner-SDF formulation. This removes the contradiction: the uniform case is *exact*.

### New POD GPU mirror record: `UiInstance` (std430, owned by `boyko_render`, NOT an ECS column)
```rust
// boyko_render/src/ui/instance.rs
/// One instanced UI quad on the GPU — the std430 record the shader's
/// StructuredBuffer<UiInstance> reads by SV_InstanceID. #[repr(C)] POD.
/// PACK SCRATCH ONLY (Decision 6): materialized into a reused Vec<UiInstance>,
/// stable-sorted by StackIndex, memcpy'd in bulk. NOT an ECS column, NOT
/// per-chunk cast_slice (the global z-sort forbids per-chunk blit).
/// PHYSICAL px (scale folded at pack). PREMULTIPLIED color.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct UiInstance {
    pub min_px:        [f32; 2], // off 0  : top-left, physical px
    pub size_px:       [f32; 2], // off 8  : w,h physical px
    pub clip:          [f32; 4], // off 16 : clip AABB min.xy,max.xy physical px (valid iff CLIP_PRESENT)
    pub corner_radius: [f32; 4], // off 32 : tl,tr,br,bl physical px
    pub color:         u32,      // off 48 : PREMULTIPLIED RGBA8
    pub border_color:  u32,      // off 52 : PREMULTIPLIED-at-pack RGBA8
    pub border_width:  f32,      // off 56 : uniform border width, physical px (P5a uniform)
    pub flags:         u32,      // off 60 : bit0 BORDER_ANY, bit1 CLIP_PRESENT, rest reserved
}
```
**CORRECTION vs prior draft (layout/std430/f16/major+minor, both reviewers)**: the prior draft oscillated 68/52/44 B with **f16 corners** and **deferred the layout to implementation** (Open Question 2). That is a load-bearing interface left open, and **f16 in an SSBO requires the `shaderFloat16`/16-bit-storage device feature** (not free). **Resolved now, one ratified layout:**
- **All-f32, no f16, no u8 packing** — eliminates the 16-bit-storage feature dependency and any hand-unpack offset risk. The size delta vs 52 B at low-thousands instances is sub-microsecond and irrelevant.
- **std430-pinned by construction**: `float2` aligns to 8, `float4` aligns to 16, scalars to 4. Field order places the two `float2`s first (off 0, 8 — `clip` `float4` lands on a 16 B boundary at off 16), then the two `float4`s (off 16, 32), then four scalars (off 48–60). **Total stride = 64 B**, naturally 16-aligned, and `64 % 16 == 0` so the array stride is std430-legal with **no internal padding and no tail pad**.
```rust
pub const UI_INSTANCE_SIZE: usize = 64;
const _: () = assert!(size_of::<UiInstance>() == UI_INSTANCE_SIZE);
const _: () = assert!(align_of::<UiInstance>() == 16); // float4 forces 16
// Per-field offset oracles (catch a Rust↔HLSL offset drift the size-assert misses):
const _: () = assert!(core::mem::offset_of!(UiInstance, min_px) == 0);
const _: () = assert!(core::mem::offset_of!(UiInstance, size_px) == 8);
const _: () = assert!(core::mem::offset_of!(UiInstance, clip) == 16);
const _: () = assert!(core::mem::offset_of!(UiInstance, corner_radius) == 32);
const _: () = assert!(core::mem::offset_of!(UiInstance, color) == 48);
const _: () = assert!(core::mem::offset_of!(UiInstance, border_color) == 52);
const _: () = assert!(core::mem::offset_of!(UiInstance, border_width) == 56);
const _: () = assert!(core::mem::offset_of!(UiInstance, flags) == 60);
```
Note: `align_of == 16` forces the Rust struct to 16-align, matching the std430 array stride — no surprise padding. The HLSL `struct UiInstance` declares the fields in the same order with the same scalar/vector types; the per-field offset asserts + the std430 stride-equals-64 check are the compile-time oracle, and the golden test (per-field-distinguishing samples) is the runtime oracle.

**No-bytemuck POD upload** — **CORRECTION vs prior draft (bytemuck/major)**: **Verified** `boyko_render/Cargo.toml` has **no bytemuck dependency** (the `cast_slice`/`Pod` precedent is `boyko_demo`-on-wgpu only). P5a does **not** add bytemuck. The upload uses a hand-rolled `&[u8]` view with a SAFETY note (the cross-crate FFI-buffer convention):
```rust
// SAFETY: UiInstance is #[repr(C)] all-POD (f32/u32), no padding (const-asserted
// 64 B, 16-align), so its byte image is a valid initialized [u8]. The slice is
// not retained past the memcpy.
let bytes = unsafe {
    core::slice::from_raw_parts(
        scratch.pack.as_ptr().cast::<u8>(),
        scratch.pack.len() * UI_INSTANCE_SIZE,
    )
};
```
(No `Pod`/`Zeroable` derive; no `cast_slice`.)

### CPU scratch (a `Resource`, preallocated once — Principle 0 storage)
```rust
// boyko_render/src/ui/mod.rs
/// Reused per-frame UI render scratch. Allocated ONCE at setup; only sorted/
/// truncated per frame (capacity persists). Mirrors LayoutScratch's discipline.
pub struct UiRenderScratch {
    pack: Vec<UiInstance>,   // packed records, sorted by StackIndex; clear()+extend, never Vec::new
    last_count: u32,
    /// O(1) change-gate signal (see A1 step 1). Bumped by any writer of the inputs.
    last_seen_generation: u64,
}
```

### Persistent instance ring (owned by `RhiContext::UiRenderResources`, Decision 8)
```text
For each of FRAMES_IN_FLIGHT (=2): one HOST_VISIBLE|HOST_COHERENT BufferUsage::STORAGE
buffer, created via RhiDevice::create_buffer (through split_mut().0), mapped once via
buffer_mapped_ptr/map_buffer, never unmapped. cap_bytes grows pow2 only on overflow
(fence-wait that slot, destroy+recreate, re-map, REBUILD that slot's bind-group).
One bind-group per slot, created once at setup, bound at set0/binding0
(StorageBuffer, ShaderStage VERTEX|FRAGMENT). Owned + Drop-wired on RhiContext.
```
The push-constant block (VERTEX+FRAGMENT, 16 B) carries the ortho transform:
```rust
#[repr(C)]
pub struct UiOrtho { pub scale: [f32; 2], pub translate: [f32; 2] } // 16 B
```

---

## Public API
```rust
// boyko_ui::components
pub struct UiBackground { /* fields above */ }
impl Default for UiBackground { /* transparent, no border */ }

// boyko_render::ui  (new module)
#[repr(C)] pub struct UiInstance { /* std430 mirror, all-f32, 64 B */ }
pub const UI_INSTANCE_SIZE: usize; // = 64
#[repr(C)] pub struct UiOrtho { pub scale: [f32;2], pub translate: [f32;2] }

/// POD by-value handoff from the dispatcher-solo upload system to the swapchain
/// recorder. Borrows NO RHI handle (Decision 9 soundness). frame_index selects the
/// FIF ring/bind-group; the recorder re-resolves the handles indirectly (MF-7).
#[derive(Clone, Copy)]
pub struct UiFramePlan {
    pub instance_count: u32,
    pub ortho: UiOrtho,
    pub frame_index: usize,
}

/// The dispatcher-solo upload system (GpuSystem-shaped): packs UiInstance scratch,
/// sorts by StackIndex, grows/ensures the per-FIF ring, memcpys, stashes UiFramePlan.
/// EMPTY Access, is_gpu()->true, projects RhiContext via DispatcherToken.
pub struct UiUploadSystem { /* scratch key, ortho-source key */ }
impl UiUploadSystem { pub fn new(/* … */) -> Self; }

// boyko_render::RhiContext  (NEW first-class UI capability — Decision 8)
impl RhiContext {
    /// SETUP-only: build the UI graphics pipeline (for `color_format`), bind-group
    /// layout, per-FIF host-mapped STORAGE rings + bind-groups. Owned + Drop-wired.
    pub fn ui_setup(
        &mut self, color_format: Format, spirv_vs: &[u32], spirv_fs: &[u32],
        initial_rows: u32,
    ) -> Result<(), GpuColumnError>;
    /// Frame: ensure slot capacity (grow + rebuild that slot's bind-group on
    /// overflow), memcpy `packed` into the mapped slot. Returns the by-value plan.
    pub fn ui_upload(
        &mut self, packed: &[u8], instance_count: u32, ortho: UiOrtho,
        frame_index: usize,
    ) -> Result<UiFramePlan, GpuColumnError>;
    /// Re-resolve the current-FIF UI pipeline + bind-group by frame_index (MF-7).
    /// Used by the swapchain recorder, in the same dispatcher window.
    pub fn ui_handles(&self, frame_index: usize)
        -> (&<Vulkan as RhiApi>::GraphicsPipeline, &<Vulkan as RhiApi>::BindGroup);
}

/// Records the UI rect pass into an already-open color target (swapchain or test
/// offscreen). Shared by the concrete swapchain path and the golden test.
/// Caller opens begin_rendering(LoadOp::Load) at FULL target extent first.
pub unsafe fn record_ui_rects<A: RhiApi>(
    enc: &mut impl RhiCommandEncoder<A>,
    full_area: &RenderArea,
    plan: &UiFramePlan,
    pipeline: &A::GraphicsPipeline,
    bind_group: &A::BindGroup,
);

// boyko_rhi::enums  (new)
pub enum BlendFactor { Zero, One, SrcAlpha, OneMinusSrcAlpha /* minimal set */ }
pub enum BlendOp { Add /* … */ }
#[repr(C)] pub struct BlendState {
    pub src_color: BlendFactor, pub dst_color: BlendFactor, pub color_op: BlendOp,
    pub src_alpha: BlendFactor, pub dst_alpha: BlendFactor, pub alpha_op: BlendOp,
}
impl BlendState { pub const PREMULTIPLIED_ALPHA: BlendState; pub const STRAIGHT_ALPHA: BlendState; }

// boyko_rhi::descriptor  GraphicsPipelineDesc gains:
pub blend: Option<BlendState>, // None = today's opaque (every existing pipeline unchanged)
```

---

## Algorithms for critical paths

### A1: Pack + z-sort + upload (the per-frame CPU build)
**Steps** (in `UiUploadSystem::run_dispatcher`, after projecting `RhiContext`):
1. **Change gate (O(1), alloc-free)** — **CORRECTION vs prior draft (gate cost/major)**: the prior "if any of four components ticked" implies an `O(N)` Changed scan, which contradicts the "~0 work static frame" budget (and `Option<&ComputedClip>`/`Option<&StackIndex>` are non-filtering, so the query visits every node regardless). **Resolved**: a single **monotonic `u64` generation counter** (`UiRenderGeneration`, a `Resource`) is bumped by the layout system (on any `ComputedRect` change) and by any writer of `UiBackground`/`StackIndex`/`ComputedClip` (via a tiny `on_change` hook or an explicit bump in the authoring/command path) and by `UiViewport`/swapchain-extent changes. The gate is one `u64` compare: `if gen == scratch.last_seen_generation { return; }`. **The 0%-when-static guarantee is an O(1) compare, not an O(N) scan.** (If a writer cannot cheaply bump the counter, the fallback is an `O(N)` tick scan and the "static ~0" claim is downgraded to "static = O(N) read-only, no alloc, no sort, no upload" — stated honestly; the counter is the chosen primary.)
2. **Pack**: `world.query::<(&ComputedRect, &UiBackground, Option<&ComputedClip>, Option<&StackIndex>), ()>().for_each_chunk(...)` — `scratch.pack.clear()` then `extend` one `UiInstance` per node (preallocated; never `Vec::new`). Per node: fold `scale_factor` (logical→physical) into `min_px`/`size_px`/`corner_radius`/`border_width`; **premultiply** `color` and `border_color` (author straight → premul); set `clip` + `CLIP_PRESENT` from `ComputedClip` (else leave `clip` zero, flag clear); set `BORDER_ANY` from uniform `border_width > 0`; record the `StackIndex` (or 0) in a parallel sort-key lane (or sort by reading the node's stack before extend — see step 3).
3. **Sort**: stable sort `scratch.pack` by `(StackIndex, append_index)`. Since `UiInstance` does not carry `stack`, the sort uses a **parallel `Vec<(u32 stack, u32 idx)>` key lane** (also capacity-stable, reused) OR an index-permutation sort; the chosen form is a key lane sorted with `sort_by_key` then a gather — both `O(N log N)`, zero alloc. (Append order is the natural tie-break since the key lane is filled in traversal order.)
4. **Ensure ring capacity + grow**: `need = N * UI_INSTANCE_SIZE`; if `need > cap[fif]` → **fence-wait slot `fif`**, destroy+recreate that slot's buffer to `need.next_power_of_two()`, re-map, **rebuild that slot's bind-group** (Decision 7). Setup-class cost; only on growth.
5. **Upload**: `RhiContext::ui_upload(bytes, N, ortho, fif)` → `memcpy` the contiguous `&[u8]` view of `scratch.pack` into the mapped current-FIF slot. HOST_COHERENT → no explicit flush; if a non-coherent memory type is selected, `vkFlushMappedMemoryRanges` once.
6. Return `UiFramePlan { instance_count: N, ortho, frame_index: fif }`; the system stashes it in the `NonSend` `UiFramePlan` resource the swapchain step reads (by value, no borrow).

**Complexity**: pack `O(N)` sequential SoA; sort `O(N log N)`; upload `O(N)` bulk `memcpy`. No alloc in steady state (the first frame and any capacity-crossing frame realloc the pack `Vec`/ring pow2-amortized — **excluded from the steady-state guarantee, asserted by the counting-allocator bench**).
**Cache**: pack streams (sequential column reads + sequential `pack` writes); upload is one contiguous `memcpy`.
**Branching**: per-node only on `Option` presence (resolved per-archetype by the query) and the `BORDER_ANY`/`CLIP_PRESENT` flag sets — minimal.
**SIMD**: `scale_factor` multiply over `min/size/corner` auto-vectorizes (SoA source).

### A2: Ortho transform (pixel→NDC, top-left origin, computed from the SWAPCHAIN extent)

> **DEVIATION (GPU oracle overrides this section).** The plan's original A2 formula
> below (`scale = (2/SW, -2/SH)`, `translate = (-1, +1)`, GL-style negative y) is
> **WRONG on the in-house Vulkan path**: the Rung-0.5 GPU oracle (`ssbo_graphics_
> probe`, RTX 3060, validation clean) shows it lands pixel row 0 at the framebuffer
> *bottom*. The **implemented, canonical** convention is the POSITIVE-y form:
> ```text
> scale     = (2.0 / SW, +2.0 / SH)   // positive y → top-left origin in y-down NDC
> translate = (-1.0, -1.0)
> ndc       = pos_px * scale + translate   // (0,0)->(-1,-1), (SW,SH)->(+1,+1)
> ```
> See `UiOrtho::for_extent` and its doc (`boyko_render/src/ui/instance.rs`). The GL
> `-2/SH, +1` form is the rejected one — do not reintroduce it.

Original (rejected) A2 text, kept for the audit trail — **superseded by the deviation
box above**. From the **swapchain `VkExtent2D` the UI pass renders into** (`SW`, `SH`)
— **not** `UiViewport` unless the host guarantees equality (Decision 9 contract):
```text
scale     = (2.0 / SW, -2.0 / SH)   // negative y → top-left origin (y-down)
translate = (-1.0,  +1.0)
clip_xy   = pos_px * scale + translate
```
`pos_px` is physical px (scale_factor applied at pack). Top-left origin via the y-flip baked into the ortho push-constant (not a negative-height viewport — keeps the viewport unchanged from the present path, one fewer Vulkan-state divergence). One mat-free vec2 MAC in the vertex shader. **The denominator is the same extent the UI viewport/scissor are set to (full swapchain), so `(0,0)→(-1,+1)` and `(SW,SH)→(+1,-1)` of the image actually drawn into.**

### A3: Vertex shader (HLSL, vertexless quad, reads the SSBO in the VERTEX stage)
```text
SV_VertexID 0..5 → unit-quad corner (0,0 / 1,0 / 0,1 / 0,1 / 1,0 / 1,1)
inst = UiInstance[SV_InstanceID]                 // SSBO read in VERTEX stage (Rung-0.5 proves this works)
pos_px = inst.min_px + corner * inst.size_px
out.clip_pos  = float4(pos_px * ortho.scale + ortho.translate, 0, 1)
out.pos_px    = pos_px                            // physical px, for FS clip + SDF
out.local_px  = pos_px - (inst.min_px + 0.5*inst.size_px) // rect-centered
out.inst_index = SV_InstanceID                   // fetch full record in FS
```
Branchless; SDF stays in physical px so AA is one device pixel.

### A4: Fragment shader (rounded-box SDF + AA + UNIFORM border + flag-gated clip, premultiplied out)
```text
inst = UiInstance[in.inst_index]
half = 0.5 * inst.size_px
d    = sdRoundedBox(in.local_px, half, inst.corner_radius)   // Quilez/Bevy per-corner
fw   = fwidth(d)                                             // resolution-independent AA
fill_cov = 1.0 - smoothstep(-fw, fw, d)                      // outer shape coverage

// UNIFORM border (P5a): inner shape = same rounded box inset by border_width, EXACT.
rgb = premul_rgb(inst.color); a = alpha(inst.color)
if (inst.flags & BORDER_ANY) {
    d_inner   = sdRoundedBox(in.local_px, half - inst.border_width, max(inst.corner_radius - inst.border_width, 0))
    inner_cov = 1.0 - smoothstep(-fw, fw, d_inner)
    border_cov = saturate(fill_cov - inner_cov)             // exact ring, no #17561 corner bleed (uniform)
    // blend border over fill within the shape (premultiplied)
    rgb = lerp(rgb, premul_rgb(inst.border_color), select(fill_cov>0, border_cov/max(fill_cov,eps), 0))
    a   = lerp(a,   alpha(inst.border_color),       select(fill_cov>0, border_cov/max(fill_cov,eps), 0))
}

cov = fill_cov
if (inst.flags & CLIP_PRESENT) {                            // well-predicted uniform branch; no sentinel
    cov *= clip_coverage(in.pos_px, inst.clip, fw)          // AA at clip edges; clip is finite author AABB
}
out.rgba = float4(rgb * cov, a * cov)                       // PREMULTIPLIED (rgb already premul)
```
- `sdRoundedBox` is the verbatim Quilez/Bevy per-corner select. **Uniform** inner-inset is exact (no approximation); asymmetric per-side is deferred (UiBorder scope note).
- AA via `fwidth` (resolution-independent) — chosen over Bevy's `saturate(0.5-d)` for DPI correctness. **CORRECTION vs prior draft (fwidth unprecedented/major)**: **Verified** no existing HLSL shader uses `fwidth`/`ddx`/`ddy` (all current shaders are compute). This is the first derivative-using fragment shader AND the first VS+FS analytic-SDF graphics shader in the engine. `fwidth(d)` with `d` in physical px assumes a 1:1 quad→pixel mapping — which the Decision-9 full-swapchain-extent contract guarantees (the UI viewport spans the same pixels the ortho denominator uses). A golden case **measures the AA band width** at a straight edge (count texels between full-bg and full-fg along a scanline) and asserts ~1–2 device-pixel band at two resolutions, proving `fwidth` operates on the true device-pixel gradient.
- Output premultiplied → matches `src=ONE` blend.
**Branching**: one `BORDER_ANY` + one `CLIP_PRESENT` uniform branch (both per-instance, well-predicted). **ALU**: ~25 analytic ops/pixel, no loops, no marching.

### A5: Draw recording (`record_ui_rects`, shared) + swapchain hook
Caller opens a **fresh** `begin_rendering` (color = target, `LoadOp::Load`, `StoreOp::Store`, no depth, the UI pipeline's `blend=PREMULTIPLIED`) at the **FULL target extent**:
```text
enc.bind_graphics_pipeline(ui_pipeline)              // re-resolved by frame_index (MF-7)
enc.bind_descriptor_set(ui_bind_group, ui_pipeline)  // current FIF slot, re-resolved
enc.push_graphics_constants(ui_pipeline, VERTEX|FRAGMENT, 0, bytes_of(plan.ortho))
enc.set_viewport(FULL_swapchain_viewport); enc.set_scissor(FULL_swapchain_area)  // NOT present_extent
enc.draw(6, plan.instance_count, 0, 0)               // ONE draw
```
Swapchain wiring: `record_present_sampled` ends the composite scope (unchanged), then **opens a second `begin_rendering(LoadOp::Load)` at the full swapchain extent**, calls `record_ui_rects` against the concrete encoder (handles re-resolved by `frame_index` from the projected `RhiContext`), ends the UI scope, then the COLOR→PRESENT barrier (unchanged). `present_sampled` gains an `Option<UiFramePlan>` arg. Complexity `O(1)` recording.

---

## Multithreading model
- **Single-threaded GPU touch**: `RhiContext` is `!Send + !Sync`. Both the upload system and the swapchain draw recording run on the **dispatcher thread** during the apply window (`running == 0`). The upload system is `SystemKind::GpuCompute` (EMPTY `Access`, `is_gpu()`), dispatched solo — no worker holds an aliasing cell, no other system body in flight (the Option-C invariant, proven Phase 9.1/9.2).
- **Cross-frame handoff soundness (Decision 9, critic C2)**: `UiFramePlan` is **POD by value** — `instance_count: u32`, `ortho: UiOrtho` (16 B POD), `frame_index: usize`. It borrows **no** RHI handle, so nothing `!Send`/`!Sync` crosses the `nonsend_resource_mut` token drop. The swapchain recorder runs in the **same dispatcher-solo window**, takes a **fresh `&RhiContext`** (re-projection), and calls `ui_handles(frame_index)` to **re-resolve** the current-FIF pipeline + bind-group (MF-7). **SAFETY**: (a) no borrow outlives its projection; (b) a grow between `ui_upload` and `record_ui_rects` that rebuilt slot `fif`'s bind-group is invisible to the recorder because it re-resolves by `frame_index` *after* the grow, never holding a pre-grow handle; (c) the only durable state shared upload→draw is the by-value `frame_index` + count, both immutable POD.
- **Shared data**: none across threads. The instance ring + bind-groups are touched only on the dispatcher thread. The CPU `pack`/key-lane scratch is a `Resource` accessed only by the dispatcher-solo upload system.
- **Sync points**: the existing per-FIF `in_flight` fence (already in `present_sampled`) is the only sync. Growth fence-waits the affected slot before destroy+recreate (setup-class). No new atomics, no `Mutex`/`RwLock`/`RefCell` — Principle 4 upheld.
- **Frames-in-flight correctness**: one ring + one bind-group **per FIF slot**, written/bound/re-resolved for the matching `frame_index`, so a frame's writes never race the previous frame's in-flight reads (the standard double-buffer; same discipline as the present fences).
- **Send/Sync**: `UiBackground` is `Send+Sync` POD (ECS column). `UiInstance`/`UiOrtho`/`UiFramePlan` are POD. `UiUploadSystem` holds only durable keys; it is the `!Send`-reaching-via-token shape of `GpuSystem`. `UiRenderResources` is `!Send`-owned inside `RhiContext`. **Data-race freedom**: no datum reachable from two threads; the only cross-frame sharing (ring slots) is fence-gated by FIF index. ∎

---

## Integration
- **`boyko_rhi`** (`descriptor.rs`, `enums.rs`): add `BlendState`/`BlendFactor`/`BlendOp`; add `blend: Option<BlendState>` to `GraphicsPipelineDesc`. Additive — every existing pipeline passes `blend: None` and is byte-identical. *Existing-API change*: every `GraphicsPipelineDesc` constructor site adds `blend: None` (mechanical; test triangle/MRT/depth/sampled descriptors).
- **`boyko_rhi_vulkan`** (`rhi_impl.rs`): in `create_graphics_pipeline`, when `desc.blend == Some(bs)` emit `blend_enable: VK_TRUE` + lowered factors/op on the (single) color attachment; `None` keeps the current `VK_FALSE` block. **No vertex-input change** (Decision 2).
- **`boyko_rhi_vulkan`** (`swapchain.rs`): `present_sampled`/`record_present_sampled` gain an `Option<UiFramePlan>` arg; after the composite scope's `end_rendering`, **open a fresh `begin_rendering(LoadOp::Load)` at the FULL swapchain extent**, set viewport+scissor to the full extent, call the shared `record_ui_rects` (handles re-resolved by `frame_index`), end the UI scope, then the existing COLOR→PRESENT barrier.
- **`boyko_render`** (NEW `ui/` module + `RhiContext` extension, Decision 8): `UiInstance`, `UiOrtho`, `UiFramePlan`, `UiUploadSystem`, `record_ui_rects`, `UiRenderScratch`, `UiRenderGeneration`, and the **new `RhiContext::{ui_setup, ui_upload, ui_handles}`** + the owned `UiRenderResources` sub-owner with **wired `Drop`/`destroy_all`** (forwarding to the device verbs via `split_mut().0`). **These methods do NOT exist today and are net-new (critic-corrected).** The verbs they forward to (`create_buffer`, `buffer_mapped_ptr`/`map_buffer`, `create_graphics_pipeline`, `create_bind_group_layout`, `create_bind_group`) are `RhiDevice` methods reached via `split_mut().0`.
- **`boyko_render/Cargo.toml`**: **no new dep** (no bytemuck; hand-rolled `&[u8]` POD view). The two SPIR-V entry blobs are embedded via the existing `SpirvBlob<N>` pattern — **note**: `SpirvBlob` is currently single-entry compute (`gpu_system.rs:64`, `SpirvBlob<968>`); P5a embeds **two** blobs (`ui_rect.vert.spv`, `ui_rect.frag.spv`) as two `SpirvBlob<N>` statics. Rung 0.5 confirms the dxc two-entry compile + two-module pipeline create works.
- **`boyko_ui`** (`components.rs`): add `UiBackground` (+ `Default`, const-assert, doc). `UiInstance` stays in `boyko_render` (keeps any POD/FFI concern out of `boyko_ui`). Add `UiRenderGeneration` bump points (layout system + authoring/command writers of `UiBackground`/`StackIndex`/`ComputedClip`).
- **Authoring** (P2 `ui!` / P3 `.ui`): `UiBackground` becomes a settable named-struct component via the static bundle cache; the writer bumps `UiRenderGeneration`. No grammar change beyond the field keys.
- **New shaders**: `crates/boyko_render/shaders/ui_rect.hlsl` (VS+FS entries) → dxc-compiled offline to `ui_rect.vert.spv`/`ui_rect.frag.spv`, embedded via `SpirvBlob<N>`. A module note documents the dxc recompile step.

---

## Implementation plan (for the developer)

**Rung 0 — RHI blend capability** (`boyko_rhi`, `boyko_rhi_vulkan`)
1. `boyko_rhi/src/enums.rs`: add `BlendFactor`, `BlendOp`, `BlendState` + `PREMULTIPLIED_ALPHA`/`STRAIGHT_ALPHA` consts.
2. `boyko_rhi/src/descriptor.rs`: add `blend: Option<BlendState>` to `GraphicsPipelineDesc`; doc; update all in-repo constructors to `blend: None`.
3. `boyko_rhi_vulkan/src/rhi_impl.rs`: lower `Some(blend)` → `VkPipelineColorBlendAttachmentState { blend_enable: VK_TRUE, … }`; keep `None` byte-identical.
4. Gate: existing `graphics_*` tests green; a new blend unit test (two overlapping translucent triangles → analytic premultiplied composite) on the RTX 3060, validation = zero messages.

**Rung 0.5 — Prove the SSBO-in-graphics combination IN ISOLATION** (NEW, de-risks Decision 2) (`boyko_render` or `boyko_rhi_vulkan/tests`)
5. A trivial graphics pipeline: `vertex_layout: None`, `bind_group_layout: Some([StorageBuffer @ VERTEX|FRAGMENT])`, reads ONE `{min_px,size_px,color}` SSBO record by `SV_InstanceID`, the **VS reads `min_px`/`size_px`** (so a VS-stage SSBO read is exercised) and writes a solid quad; FS reads `color`. Offscreen `R8G8B8A8` golden on the RTX 3060, validation = **zero messages**. Proves the never-exercised graphics+SSBO+VERTEX-stage path before SDF/blend complexity. **Blocks Rung 2+.**

**Rung 1 — `UiBackground` + `UiInstance`** (`boyko_ui`, `boyko_render`)
6. `boyko_ui/src/components.rs`: add `UiBackground` (+ `Default`, const-assert 44 B, doc; uniform-border note).
7. `boyko_render/src/ui/instance.rs`: add `UiInstance` (all-f32, 64 B, **per-field `offset_of!` const-asserts** + size/align), pack helpers (premultiply, sentinel-free `CLIP_PRESENT`, `BORDER_ANY`), the hand-rolled `&[u8]` POD view + SAFETY.

**Rung 2 — Shaders** (`boyko_render/shaders/`)
8. Author `ui_rect.hlsl` (VS: vertexless quad + ortho + SSBO transform read; FS: `sdRoundedBox` + `fwidth` AA + uniform border + flag-gated clip + premultiplied out); dxc-compile to two `.spv`; embed via two `SpirvBlob<N>`. Document the recompile step. HLSL `struct UiInstance` mirrors the ratified offsets.

**Rung 3 — `RhiContext` UI capability: pipeline + rings + bind-groups** (`boyko_render/src/ui/mod.rs` + `RhiContext`)
9. Add `UiRenderResources` (owned on `RhiContext`, Drop/`destroy_all` wired). `ui_setup`: build the UI pipeline for a `color_format` param (swapchain for on-screen, `R8G8B8A8` for test), bind-group layout (`StorageBuffer @ VERTEX|FRAGMENT`), per-FIF host-mapped STORAGE rings (map once) + per-FIF bind-groups — all via `split_mut().0` device verbs.
10. `ui_upload` (grow-on-overflow: fence-wait slot + recreate + remap + **rebuild that slot's bind-group**; memcpy) and `ui_handles(frame_index)` (MF-7 re-resolve).

**Rung 4 — Upload system** (`boyko_render/src/ui/upload.rs`)
11. `UiUploadSystem` (`unsafe impl System`, EMPTY `Access`, `is_gpu()`, `run_dispatcher`): **O(1) generation change-gate** → pack → key-lane stable sort → `ui_upload` → stash POD `UiFramePlan`.

**Rung 5 — Draw recording** (`boyko_render/src/ui/draw.rs` + `swapchain.rs`)
12. `record_ui_rects(enc, full_area, &UiFramePlan, pipeline, bind_group)` (generic over `RhiApi`, shared).
13. Wire into `present_sampled`/`record_present_sampled`: fresh `begin_rendering(LoadOp::Load)` at **full swapchain extent**, re-resolve handles by `frame_index`, call `record_ui_rects`, end scope, then COLOR→PRESENT.

**Rung 6 — Golden tests** (`boyko_rhi_vulkan/tests/ui_rects_golden.rs` and/or `boyko_render`)
14. The golden suite below (RTX 3060, `boot_render_or_skip`, `--test-threads=1`, validation = zero messages).

---

## Metrics and validation

### Premultiplied-alpha golden discipline (critic C2) — interior-vs-edge sample split
**CORRECTION vs prior draft**: **Verified** the existing oracle `assert_texel_close` (`graphics_deferred.rs:223–231`) asserts **RGB within TOL but ALPHA EXACTLY** (`got[3] == want[3]`). With premultiplied output `(rgb*cov, a*cov)`, the stored alpha **is** coverage·alpha — fractional and driver-variant at AA/clip/corner texels. A naive port flakes on correct output; a uniformly loose tolerance misses wrong-color bugs. **Resolved — per-case sample points + tolerances:**
- **Interior full-coverage, unclipped, non-corner texel** → assert the **exact** premultiplied color **including exact alpha** (`assert_texel_close` as-is is valid HERE).
- **AA-band / corner / clip-edge texel** → **monotonic-coverage** assertions only: alpha strictly between background and full; `R/G/B == color·alpha` within a band; never exact-match.
- **G8 (blend correctness)** → compute the analytic premultiplied composite over a KNOWN opaque background and assert **`dst = src + dst·(1-src_a)`** at an interior texel — this is what catches a straight-vs-premultiplied dark-fringe regression.

### Golden image-diff suite (RTX 3060, validation ON = zero messages)
Render fixed `UiInstance`s into an offscreen `R8G8B8A8` target via `record_ui_rects` (trait encoder, the `graphics_offscreen` flow — **note the test pipeline uses `R8G8B8A8`, the on-screen pipeline the swapchain format; both from one shader, Decision 9**), copy-to-buffer, map, assert.
- **G1 position/size**: solid rect at `(x,y,w,h)` covers exactly those physical px (±AA band); outside-corner texels are background. **Interior** sample = exact.
- **G2 color**: interior full-coverage texel == authored premultiplied color, **exact** (incl. exact A).
- **G3 rounded corners**: a sample inside the radius arc is background, just inside the straight edge is fill (crisp AA). **Distinguishing**: distinct `corner_radius` per corner so a wrong `corner_radius` offset/value fails (per-field-offset oracle).
- **G4 border (uniform)**: a ring of `border_color` of `border_width` px between outer edge and inner shape; interior is fill. Corner sample asserts no #17561 bleed.
- **G5 z-order + tie-break + translucent over**: (a) two overlapping rects, higher `StackIndex` on top, the **top rect translucent** so the overlap texel must equal the **analytic premultiplied over-result** (catches reversed order or wrong blend factor an opaque test hides); (b) two **EQUAL `StackIndex`** rects pin the stable append-order tie-break.
- **G6 clip**: a rect with `ComputedClip` smaller than its rect — texels outside the clip AABB are background, inside are fill (proves flag-gated in-shader clip). Plus a **full-screen UNCLIPPED rect** (CLIP_PRESENT clear) asserts no edge artifact (proves the sentinel-free path).
- **G7 one-draw batch**: **CORRECTION vs prior draft** — no draw-counter exists. **Resolved**: exploit `record_ui_rects` being **generic over `RhiApi`** — a **test-only `RhiCommandEncoder` wrapper that tallies `draw()` calls**, asserting exactly **one** `draw(6, N, 0, 0)` for the no-clip-batch case. (The trait-encoder golden path already uses such a recorder; the wrapper is a thin counting decorator.)
- **G8 premultiplied blend**: per the discipline above — exact `src + dst·(1-src_a)` over a known opaque background at an interior texel.
- **G9 AA band width** (critic major): along a scanline crossing a straight edge, count texels between full-bg and full-fg; assert a **~1–2 device-pixel band** at **two** target resolutions/scales — proves `fwidth` operates on the true device-pixel gradient.
- **G10 per-field offset** (critic major): ONE instance with **distinct, mutually-exclusive** values for `corner_radius` (rounded sample), `clip` (clip sample), `color`, `border_color`, `border_width` — a swapped/shifted field offset makes one sample fail. The std430 stride (= 64) and per-field `offset_of!` asserts back this at compile time.
- **G11 extent mismatch** (critic C1): render into a target whose extent ≠ the nominal `UiViewport`; a rect authored at the bottom-right corner lands at the bottom-right **texel** of the **render target** (proves the ortho denominator = the render-target/swapchain extent).

### Unit tests
- `UiInstance` per-field `offset_of!` + size(64)/align(16) const-asserts (compile-time); `UiBackground` size(44) const-assert.
- Pack: `ComputedRect`+`UiBackground` → expected `UiInstance` bytes (scale folding, premultiply, `CLIP_PRESENT`/`BORDER_ANY`, uniform border).
- Ortho: `pixel→NDC` maps `(0,0)→(-1,+1)` and `(SW,SH)→(+1,-1)` for a given **swapchain** extent (top-left origin).
- Sort: stable order by `StackIndex` with append-order tie-break (key-lane permutation).
- Change gate: equal generation → upload counter at 0; bumped generation → re-upload (counter +1).

### Property tests
- Pack ∘ any `(ComputedRect, UiBackground, ComputedClip?, StackIndex?)` never produces NaN/Inf in `UiInstance` (finite-assert).
- Sort is a permutation of the input (no drops/dupes).

### Benchmarks (criterion, `bench.ps1` median-of-N)
- Pack+sort+memcpy throughput at N = 100 / 1k / 5k; **counting allocator asserts ZERO allocations in steady state** (first/grow frames excluded explicitly).
- Steady-state static frame (generation gate hit) ≈ O(1) (one `u64` compare, no pack/sort/upload).

### `debug_assert!` invariants
- `instance_count * UI_INSTANCE_SIZE <= cap[fif]` before `memcpy`.
- ring slot index == current `frame_index` (no cross-FIF write).
- pack `Vec` not reallocated mid-frame (capacity ≥ N after the one allowed growth).
- finite `min_px`/`size_px`/`corner_radius`/`clip` before each pack write; `clip` finite **only when `CLIP_PRESENT`**.
- uniform-border invariant: `border_width[0..4]` equal (or documented max-collapse) before pack.
- UI scope viewport/scissor extent == swapchain `VkExtent2D` (not `present_extent`).

---

## Open questions
*(None blocking — all prior open questions are now decided.)*
1. ~~Mirror column vs direct pack~~ → **DECIDED: direct pack into reused scratch, no mirror column** (Decision 6); mirror column is the deferred P5b+ incremental-upload seam.
2. ~~`UiInstance` byte layout / f16~~ → **DECIDED: all-f32, 64 B, std430-pinned by per-field `offset_of!` asserts** (Data structures); no f16, no 16-bit-storage feature dependency.
3. ~~Coarse scissor batching~~ → **DECIDED: in-shader clip only in P5a**; scissor batching deferred (Decision 4).
4. ~~Color premultiply point~~ → **DECIDED: authors write straight RGBA8 in `UiBackground`; premultiply at pack** (color + border_color).
5. **`UiViewport.scale_factor` per-root / multi-monitor DPI** is out of scope for P5a (single viewport); the ortho-from-swapchain-extent contract (Decision 9/A2) is the seam for P7/multi-window. *(Informational, not blocking.)*

**Out of scope (seams)**: text/glyph quads → **P5b** (a `TEXTURED` flag bit + an atlas-sampling FS branch, or a sibling pipeline; the `flags` field + the SSBO record are forward-compatible). World-space/diegetic UI → **P7** (projects to screen, then rides this exact instanced path, optional depth-test). `UiImage` textured rects, box-shadow/blur, **asymmetric per-side borders**, **derived/overflow-driven `ComputedClip` production**, **coarse scissor batching** → future (the `flags` bits + a sampler binding + the per-side inner-SDF are the seams).

---

## Changes from review

**Critical (3) — all fixed:**
- **C1 (ortho space vs swapchain extent)** — Decision 9 + A2 + A5 now mandate the UI pass opens its **own `begin_rendering(LoadOp::Load)` at the FULL swapchain extent** (verified the present scope uses `LoadOp::Clear` clamped to `present_extent` top-left, `swapchain.rs:1734–1817`), and the **ortho denominator = the swapchain `VkExtent2D` the UI pass renders into** (not `UiViewport`). Added golden **G11** (extent-mismatch bottom-right texel) and a host lockstep contract.
- **C2 (premultiplied vs exact-alpha oracle)** — added the **interior-vs-edge golden discipline** (verified `assert_texel_close` asserts exact alpha, `graphics_deferred.rs:231`): interior full-coverage = exact incl. alpha; AA/clip/corner = monotonic-coverage band; **G8** asserts exact `src + dst·(1-src_a)`.
- **C (RhiContext-surface, both reviewers)** — **Decision 8 (new)**: P5a ADDS a first-class `RhiContext` UI capability (`ui_setup`/`ui_upload`/`ui_handles` + owned `UiRenderResources`, **Drop-wired** to avoid the leak past `RhiContext::Drop`); verbs forward through `split_mut().0` to `RhiDevice`. Removed every false "reuses `RhiContext::create_buffer/...`" claim (verified `RhiContext` is compute-only, `gpu_column.rs:105–243`).
- **C (cross-frame handoff soundness, reviewer 2 C2)** — Decision 9 + §Multithreading: the handoff is now **POD by value** (`UiFramePlan`), borrows no RHI handle, and the recorder **re-resolves** pipeline+bind-group by `frame_index` (MF-7) in the same dispatcher window. Eliminated the unsound `UiDrawData<'a>` borrow across the token drop.

**Major (8) — all fixed/justified:**
- **SSBO-in-graphics is NEW, not reuse** — Decision 2 corrected (verified all SSBO bindings are COMPUTE bind-point); added **Rung 0.5** isolated golden (VS-stage SSBO read) before SDF/blend.
- **Bind-group rebind impossible** — Decision 7: removed the "rebind each frame" variant (verified `create_bind_group` writes once, no update verb, `rhi_impl.rs:899–900`); mandated one bind-group per FIF slot, rebuilt only on grow.
- **Decision 6 self-contradiction** — resolved to **direct pack + bulk memcpy, no mirror column, no per-chunk `cast_slice`** (a global z-sort forbids per-chunk blit); honest cost stated. `GpuInstance` precedent narrowed to CPU-side discipline only (verified it is wgpu vertex-buffer, not in-house SSBO).
- **Per-frame alloc / change-gate cost** — A1 step 1: an **O(1) `u64` generation gate** (not an O(N) Changed scan); steady-state zero-alloc asserted by the counting-allocator bench; first/grow frames explicitly excluded.
- **fwidth AA unprecedented** — A4 corrected (verified no fwidth in-tree); added golden **G9** (AA band width ~1–2 px at two resolutions).
- **Per-side border contradiction** — `UiBorder` scope: **P5a = uniform border only (exact)**; asymmetric deferred. Removed the "approximation"/"exact" contradiction.
- **bytemuck not a dep** — verified; P5a uses a **hand-rolled `&[u8]` POD view** with SAFETY, no bytemuck, no `cast_slice`.
- **std430 offsets vs Rust size** — ratified the **all-f32 64 B** layout with **per-field `offset_of!` const-asserts** + a std430-stride check; golden **G10** distinguishes each field; **G7** draw-count via a **test-only counting encoder** (exploiting the generic `record_ui_rects`).

**Post-implementation review (foundation commit) — fixed:**
- **Border composite was a premultiplied-space `lerp` (major)** — `ui_rect.fs.hlsl`
  composited border-over-fill with `lerp(fill, border, t)` on PREMULTIPLIED operands.
  `lerp`/`mix` is the "over" operator only for STRAIGHT-alpha colors; for premultiplied
  operands it under-/over-weights the fill where the border is translucent (border
  alpha < 255), reintroducing exactly the fringe premultiplied alpha exists to avoid.
  **Fixed**: the border ring is now composited with the true premultiplied OVER —
  `result = src + dst*(1 - src.a)`, `src = border_premul * border_cov`,
  `dst = fill_premul * inner_cov` — exact for both opaque and translucent borders. The
  `.spv` was recompiled (dxc ps_6_0, vulkan1.3) + `spirv-val` clean. The mandated
  golden **G4** must add a TRANSLUCENT-border case (flagged for the tester).
- **Ortho-convention plan deviation recorded** — the implemented positive-y ortho
  overrides plan A2 (the GL `-2/SH,+1` form); the GPU oracle (Rung 0.5) is the
  authority. See the A2 deviation box and the `UiOrtho` doc. The ortho unit test
  (maps `(0,0)->(-1,-1)`, `(SW,SH)->(+1,+1)`) is mandated to pin it in CI.

**Minor (all addressed):**
- `GpuInstance` precedent narrowed (CPU discipline only; it is wgpu) — Decision 6.
- f16/16-bit-storage feature dependency eliminated (all-f32) — Data structures.
- `ComputedClip` is author-owned (verified `components.rs:188`); Decision 4 rationale aligned to "consume author clip; derived clip deferred."
- Far-box clip sentinel replaced by a `CLIP_PRESENT` flag branch (well-conditioned, sentinel-free) — Decision 4 / A4; **G6** full-screen-unclipped case added.
- VERTEX|FRAGMENT SSBO visibility is first-use — Rung 0.5 exercises a VS-stage SSBO read under the zero-message oracle.
- G5 strengthened (translucent-over + equal-StackIndex tie-break).
```