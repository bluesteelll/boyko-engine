# `boyko_rhi` — Backend-Agnostic RHI Trait Crate (Render Foundation Phase 1)

> **Status: REVISED (architect → critic gate complete; all CRITICAL/IMPORTANT findings resolved below).**
> Branch `ecs`, 2026-06-16.
> Conforms to [docs/RENDER-PHYSICS-GPU-PLAN.md](RENDER-PHYSICS-GPU-PLAN.md) §3 (crate topology), §4 (in-house
> RHI + Vulkan backend), §5.4 (`PoolBacking` + opaque `u64` handle), §5.5 (barrier lowering: conflict graph =
> ordering only; explicit caller-side barriers), §7 Phase 1-3, §11.
>
> Phase 1 abstracts **only the headless compute path** (Slice 0) of the already-working, validation-clean
> Vulkan backend behind a static-dispatch RHI trait so DX12/Metal can plug in later. `boyko_ecs` is
> **untouched** this phase (its §5.4 seam lands in Phase 4).

## Scope decision (post-critic): Phase 1 = headless compute only; on-screen abstraction is Phase 2-3

The foundation thesis is **headless** (foundation plan §0: "validated headlessly"; §7 Phase 1-3 explicitly
"swapchain only when on-screen is needed"). The critic correctly found (C1/C3) that the first draft both
claimed the Slice-1 on-screen path was "abstracted NOW" *and* left its substance — the
UNDEFINED→COLOR→PRESENT **image-layout barriers** (`swapchain.rs::record_clear`), the per-frame **semaphore**
model, and the acquire→record→submit→present loop (`Renderer`) — outside the trait as an open question. That
is the "plan undercounts the real fire sites" class (cf. Phase 14b). **Resolution:**

- **Phase 1 abstracts the headless compute surface only**: device, sub-allocated buffers, host-visible
  mapping, compute pipeline from SPIR-V, command encoding (begin/end/bind/dispatch/**buffer** barrier),
  submit + fence. This is exactly what Slice 0 (`compute.rs`/`ComputeHarness`) exercises.
- **The on-screen path stays concrete**: `Surface`/`Swapchain`/`Renderer` remain inherent
  `boyko_rhi_vulkan` types, used directly. The `window_present` test keeps driving them concretely and is
  **untouched** by this phase. `RhiSurface`/`RhiSwapchain`/`Semaphore`/`ImageBarrier` are **genuine
  deferred seams** (Phase 2-3 "on-screen in-trait"), not Phase-1 surface.
- **Net effect:** Wave C touches only the compute path; `swapchain.rs` is not refactored. C1 and C3 are
  resolved by construction (no image-layout or semaphore surface is claimed NOW).

---

## Goal

Define the in-house, backend-agnostic RHI seam that the existing `boyko_rhi_vulkan` backend implements via
**static dispatch**, that future DX12/Metal backends plug into, and that produces the opaque `u64` resource
handle `boyko_ecs` core will see (§5.4). The trait itself is not hot-path; the **handle lookup** and
**command recording** paths are, and must be `dyn`/`Box`/`HashMap`-free. Functionally cover exactly what
Slice 0 + Slice 1 already do (device, sub-allocated buffers, map/unmap, compute pipeline from SPIR-V, command
encoding with explicit barriers, submit+fence, surface/swapchain), with designed-but-stubbed seams for
Phase 5 (`GpuColumn`) and Phase 6+ (SDF textures, graphics pipelines, bind groups, indirect dispatch).

## Constraints preserved

- The three existing tests are the **regression oracle** (`compute_write_pattern_round_trip`,
  `compute_chained_barrier_golden`, `window_present`) — Miri cannot run FFI, so validation-layer-clean +
  bit-exact golden are the only soundness oracles. They MUST keep passing.
- The existing reverse-order `Drop` teardown discipline and every `// SAFETY:` invariant in
  `boyko_rhi_vulkan` survive the refactor unchanged (owned-value + reverse-`Drop` model preserved 1:1).
- `boyko_rhi` has **NO FFI** — pure trait + handle registry + enums/descriptors. All `Vk*` types and
  `unsafe extern` stay in `boyko_rhi_vulkan`.
- Rust 2024, x86_64 (non-dispatchable handles are `u64`; the registry assumes 64-bit).

## Target metrics

- Handle resolve (`Slot` → `&owned resource`): ~3 ns (the Phase-7 `SparseMap` number), zero alloc, zero `dyn`.
- Command recording: zero allocation per recorded command, zero `dyn` dispatch — every `CommandEncoder` call
  monomorphizes to a direct `(fns.cmd_*)` indirect call (identical codegen to today's inherent methods).
- Trait dispatch overhead: **zero** vs current inherent methods (associated-type static dispatch
  monomorphizes to the same code). Validated by the unchanged golden tests, not a microbench (no abstract
  call site exists to bench).

---

## The central decision (resolved): two layers, not one

> **`boyko_rhi` IS the hal layer** (wgpu-hal-shaped: associated resource types, owned values, explicit
> `unsafe` destroy, static dispatch, NOT object-safe). The **generational-index handle registry is a THIN,
> SEPARATE, backend-agnostic layer that lives in `boyko_rhi` as its own module** (`handle`), generic over
> `A: RhiApi`. It maps `Slot` → owned RHI resource and projects `Slot` → `DeviceColumnHandle(u64)`. The trait
> does **not** bake handles in.

### Why this split (against the four criteria)

- **(a) Minimal churn.** Every concrete type today is already an owned value with an explicit reverse-order
  `Drop`/destroy (`BoundBuffer`, `ComputePipeline`, `Surface`/`Swapchain`/`Renderer`). wgpu-hal's
  "owned resource, explicit destroy" shape is a near-isomorphism → the refactor is a rename + trait-impl
  wrapping, not a rewrite. Baking the slotmap into the trait would re-plumb every struct through a registry
  it never had, and force a registry into the headless compute path that has no need for one.
- **(b) §5.4 opaque `u64`.** The registry owns a release-checked `Slot ↔ u64` **packing bridge**
  (`handle::slot_to_u64`/`u64_to_slot`). Core sees only an opaque `u64`. The hal trait never names `u64`; the
  registry never appears in core. This is exactly wgpu's layering (hal = owned values; wgpu-core = the
  slotmap registry) adapted so the registry sits one crate lower.
  **Handle-home correction (critic O3 + §3 dependency arrow):** the foundation plan §3 has
  `boyko_rhi → boyko_ecs` (rhi depends on core). So the core-facing `DeviceColumnHandle(u64)` newtype is
  **defined in `boyko_ecs`** (Phase 4, a graphics-pure bare-`u64` newtype — no graphics type, §5.4), NOT in
  `boyko_rhi`. For **Phase 1**, `boyko_rhi` deps = `boyko_utils` **only** (the `boyko_ecs` dep is added in
  Phase 4 when the seam lands); the registry uses its own typed `Slot`-based handles (`BufferHandle(Slot)`,
  …) and exposes the `Slot ↔ u64` packing bridge that Phase 4 will wire to core's `DeviceColumnHandle`.
  Defining `DeviceColumnHandle` in `boyko_rhi` now would either force a `boyko_ecs → boyko_rhi` edge
  (circular, against §3) or duplicate core's newtype — so it is deferred to its correct home.
- **(c) Zero `dyn`/`Box`/`HashMap` on the hot path.** The registry is `SparseSlotMap<A::Buffer>` (etc.) —
  generic over the concrete backend type, monomorphized, no erasure. `resolve(handle) -> &A::Buffer` is a
  generation-checked array index. Command recording goes through `&mut A::CommandEncoder` — a concrete type,
  direct calls. No `dyn` anywhere.
- **(d) DX12/Metal pluggable.** A second backend implements the same associated types over its own owned
  resources; the registry is reused verbatim (`SparseSlotMap<Dx12::Buffer>`). Enums/descriptors are
  backend-agnostic inputs both backends translate at their boundary.

### Why the registry lives in `boyko_rhi`, not `boyko_render`

The `Slot` → `u64` projection and the generational discipline are backend-agnostic and needed by both
`boyko_render` (resource management) and the Phase-4 core seam (the `u64` definition). Placing it in
`boyko_rhi` (the lowest crate that knows `A::Buffer`) lets both higher crates depend on one definition.
`boyko_render` is where *policy* lives (pools, barrier lowering); the registry is *mechanism*. `boyko_rhi`
still has no FFI — `SparseSlotMap` is pure logic from `boyko_utils`.

---

## Crate layout & Cargo wiring

```
crates/boyko_rhi/
  Cargo.toml          # deps: boyko_utils (Slot/SparseSlotMap) ONLY. NO FFI, NO boyko_ecs.
  src/
    lib.rs            # re-exports; crate-level docs; RhiApi umbrella trait
    api.rs            # trait RhiApi (associated types only, à la wgpu-hal Api)
    device.rs         # trait RhiDevice (create/destroy resources, map, pipelines)
    queue.rs          # trait RhiQueue (submit + fence wait)
    encoder.rs        # trait RhiCommandEncoder (begin/end, bind, dispatch, BUFFER barrier)
    handle.rs         # generational handle registry (generic over RhiApi) + Slot<->u64 packing bridge
    enums.rs          # BufferUsage, ShaderStage, BarrierStage/Access, MemoryLocation (Format = Phase-2-3 seam)
    descriptor.rs     # BufferDesc, ComputePipelineDesc, BarrierDesc(buffer-only)
    error.rs          # RhiError (shared) + the Error associated-type bound
    # surface.rs (RhiSurface/RhiSwapchain) = Phase 2-3 (on-screen in-trait); NOT created this phase.
```

```toml
# crates/boyko_rhi/Cargo.toml
[package]
name = "boyko_rhi"
version = "0.1.0"
edition = "2024"
description = "Backend-agnostic RHI trait surface for boyko-engine. Pure trait + handle registry + enums. NO FFI."

[dependencies]
boyko_utils = { path = "../boyko_utils" }   # Slot, SparseSlotMap, Generation
```

Workspace `members` gains `"crates/boyko_rhi"`. `boyko_rhi_vulkan/Cargo.toml` gains
`boyko_rhi = { path = "../boyko_rhi" }` (still no third-party deps).

---

## Key decisions

### D1 — associated types on an umbrella `RhiApi` trait; backends are zero-sized markers

Mirror wgpu-hal `Api`. One umbrella trait collects all associated resource types; the operational traits
(`RhiDevice`, `RhiQueue`, `RhiCommandEncoder`, `RhiSurface`, `RhiSwapchain`) are separate and reference
`A: RhiApi`. The Vulkan backend is a zero-sized `pub struct Vulkan;` implementing `RhiApi`.
**Why:** static dispatch + monomorphization → byte-identical codegen to current inherent methods. Splitting
the operational traits keeps each small (I-cache; backend impls them in separate files mirroring today's
layout). Rejected: one giant `trait Rhi` (forces unrelated methods into one impl, hurts headless build),
object-safe `dyn Rhi` (virtual dispatch on the recording path — violates principle 1).

### D2 — resources are owned associated-type values; destroy is `unsafe fn(&self, resource)` by value

`type Buffer; type ComputePipeline; type ShaderModule; type CommandEncoder; type Fence; type Surface;
type Swapchain;` etc. Create returns `Result<A::Buffer, Self::Error>`; destroy is
`unsafe fn destroy_buffer(&self, b: A::Buffer)` consuming the value.
**Why:** isomorphic to the existing owned structs + reverse-order `Drop` (minimal churn). By-value destroy
encodes "destroyed exactly once" in the type system (the move consumes it) — precisely the invariant every
existing `// SAFETY:` asserts manually. `unsafe` on destroy because the caller must guarantee the GPU is no
longer using it (fence-waited) — the existing fence/`device_wait_idle` discipline is exactly this, now
contract-documented. Rejected: RAII `Drop`-on-resource (would force a `&Device` backref into every resource,
fighting the existing model where `BoundBuffer` deliberately does NOT borrow the device so teardown order is
explicit). **Trade-off:** a `Slot` dropped without `destroy` leaks — mitigated by the registry owning the
resources + a debug-assert-non-empty-at-drop.

### D3 — explicit caller-side **buffer** barriers via `BarrierDesc`; image barriers are a Phase-2-3 seam

`RhiCommandEncoder::pipeline_barrier(&mut self, &BarrierDesc)` with `src_stage`/`dst_stage` (`BarrierStage`
bitflags) and `buffers: &[BufferBarrier { buffer, src_access, dst_access }]`. **Phase 1 is buffer-only** —
this is exactly what `compute.rs::record_barrier` (the 0d chained-barrier path) needs. **`ImageBarrier`
(with `old_layout`/`new_layout`/`subresource_range`) is a genuine deferred seam (Phase 2-3)** — per the
scope decision, the only image-layout transitions that exist today (`swapchain.rs::record_clear`'s
UNDEFINED→COLOR→PRESENT) live in the concrete `Renderer` and are NOT routed through the trait this phase
(C1 resolution).
**Why:** §5.5 mandates explicit barriers, no auto-tracking; `record_barrier` is exactly this shape.
Abstracting the **masks** (not the call) is what lets `boyko_render`'s §5.5 edge→barrier lowering emit
backend-agnostic `BarrierDesc`s later. Rejected: automatic resource-state tracking (wgpu-core style) — §5.5
puts superset-correct explicit lowering in `boyko_render`; auto-tracking adds per-resource state + a
transition search to the hot path. A missed barrier is caught by sync-validation (the 0d golden test already
proves the layer flags it).

### D4 — error model: ONE unified per-backend `Error` + a shared `RhiError`; bound is `From<RhiError>` only (W3 resolved)

```rust
pub trait RhiDevice<A: RhiApi> {
    type Error: core::fmt::Debug + From<RhiError>;   // ONE direction only — no Into bound (avoids the coherence wall)
    // ...
}
```
- **All operational traits for a backend use the SAME associated `Error` type** (the Vulkan backend defines
  one `VulkanError` enum subsuming its Boot/Memory/Compute variants — the four existing rich enums fold into
  one, keeping the `command-name + VkResult` diagnostic detail the validation oracle relies on). A per-trait
  `Error` fragment buys nothing because `boyko_render` will `?`-chain device + encoder + queue calls and must
  unify them anyway (critic W3(2)).
- **The bound is `From<RhiError>` only** (needed so the `#[cold]` seam-stub defaults can write
  `Err(RhiError::Unsupported(...).into())`). The **agnostic projection is a hand-written
  `impl From<VulkanError> for RhiError`** in the backend (mapping `OUT_OF_DATE`→`SurfaceOutOfDate`, etc.) —
  NOT a blanket `impl<E: Into<RhiError>> From<E> for RhiError`, which is exactly the reflexive-collision
  coherence wall the critic flagged (Q5/W3(1)). With a hand-written `From`, there is no blanket and no
  collision; `boyko_render` projects via `RhiError::from(e)` at the agnostic boundary.
- Shared `RhiError` (`DeviceLost`, `OutOfMemory`, `Unsupported(&'static str)`, `SurfaceOutOfDate`,
  `SuboptimalSurface`, `BackendError(&'static str)`) carries the control-flow categories. **All `RhiError`
  conversions are `#[cold]` / `#[inline(never)]`** so the `?`-desugar's conversion call on the `Err` path
  never inlines into the hot recording code's I-cache footprint (critic O4/W3(3)).
- `boyko_ecs` stays `anyhow`-at-the-facade per convention; RHI does NOT use `anyhow` (low-level crate; would
  add an alloc + erasure per error).

### D5 — thin backend-agnostic enums; bitflag values equal Vulkan constants (identity cast for bitflags only — W1)

Define ONLY what the foundation uses, as `#[repr(transparent)] struct BufferUsage(u32)` bitflags + small
enums: `BufferUsage` (STORAGE + TRANSFER_SRC/DST + UNIFORM + INDIRECT), `ShaderStage` (COMPUTE now;
VERTEX/FRAGMENT seam), `BarrierStage`, `BarrierAccess`, `MemoryLocation` (HostVisibleCoherent | DeviceLocal).
**Choose the bitflag values to equal the Vulkan bit values** so `to_vk` is a no-op identity cast (`#[inline]`)
on Vulkan for the `VkFlags`(u32) families (`VK_BUFFER_USAGE_*`, `VK_PIPELINE_STAGE_*`, `VK_ACCESS_*`,
`VK_SHADER_STAGE_*` — all u32, verified `ffi.rs:528-655`); a real `match` only where DX12/Metal differ
(cold resource-create boundary, never a hot loop).
**W1 correction — `Format`/image-layout are NOT bitflags and are `i32` in the FFI** (`VkFormat`,
`VkImageLayout`, e.g. `B8G8R8A8_UNORM = 44`, `PRESENT_SRC_KHR = 1_000_001_002`). They are **not needed by
the headless compute path**, so `Format` is **deferred to the Phase-2-3 on-screen/texture seam** along with
`RhiSurface`. When it lands it gets a cheap (cold-path) `match`/sign-preserving cast **both directions**
(agnostic↔`i32`), not the "identity" the bitflag families enjoy — the identity-cast claim is scoped to the
bitflag families only.

### D6 — handle registry: `SparseSlotMap<A::Resource>` per resource kind; typed `Slot` handles; u32-capped `u64` bridge

```rust
#[repr(transparent)] pub struct BufferHandle(Slot);   // + ComputePipelineHandle, ShaderHandle, FenceHandle…

pub struct ResourceRegistry<A: RhiApi> {
    buffers:   SparseSlotMap<A::Buffer>,
    pipelines: SparseSlotMap<A::ComputePipeline>,
    shaders:   SparseSlotMap<A::ShaderModule>,
    fences:    SparseSlotMap<A::Fence>,
}

// The Slot <-> u64 packing BRIDGE (Phase 4 wires this to core's DeviceColumnHandle; see central decision (b)).
// `Slot { index: usize, generation: u32 }` — generation is u32 (high 32), index packed into low 32.
#[inline] pub fn slot_to_u64(s: Slot) -> u64 {
    // RELEASE-PRESENT check (NOT debug_assert — a vanished check would silently truncate a >2^32 index and
    // alias a live handle, defeating ABA). The registry caps the resource-index domain at u32 (a hard limit
    // of 2^32 live resources per kind — orders of magnitude above any real device-resource count).
    assert!(s.index() <= u32::MAX as usize, "invariant: RHI resource index exceeds u32 handle domain");
    ((s.generation() as u64) << 32) | (s.index() as u64)
}
#[inline] pub fn u64_to_slot(h: u64) -> Slot { Slot::new((h & 0xFFFF_FFFF) as usize, (h >> 32) as u32) }
```
`register_buffer(&mut self, A::Buffer) -> BufferHandle`, `resolve_buffer(&self, BufferHandle) -> Option<&A::Buffer>`
(generation-checked array index), `take_buffer(&mut self, BufferHandle) -> Option<A::Buffer>` (for explicit
destroy). **Why:** `SparseSlotMap` is the mandated ABA-safe, zero-heap, `Copy`-key structure; resolve is the
Phase-7 ~3 ns lookup. Each resource kind is a SEPARATE `SparseSlotMap` (SoA, not one map of an enum) so
resolve is type-monomorphic and cache-dense per kind.
**C2 resolution (handle-packing width):** `Slot.index` is `usize` (64-bit) but the `u64` bridge packs it into
the low 32 bits, so the pack carries a **release-present `assert`** (not a `debug_assert`, which vanishes in
release) and the registry treats the index domain as **capped at `u32::MAX`** — a documented hard limit.
The stale-handle-resolution half is already closed by `SparseSlotMap`'s generation bump on `remove`/`take`
(`sparse_slot_map.rs:203-205`): a handle resolved after `take` returns `None`.
**W4 resolution (drain-before-drop):** the owned `A::Buffer` can't self-`Drop` (needs `&DeviceFns`), so the
registry exposes `destroy_all(&self, device: &A::Device)` which **`device.wait_idle()` then walks each map in
reverse-resource-order calling the matching `unsafe destroy_*`** — mirroring the existing
`ComputeHarness::drop`/`Renderer::drop` discipline (`device_wait_idle` first, reverse order). This is a
**structural teardown step the owner must call** (release-present), with a secondary `debug_assert!`
(every map empty after `destroy_all`, and on `Drop`) as a leak tripwire — NOT the only guard.

### D7 — foundation-now vs deferred-seam, encoded in the trait surface

Foundation-now methods are fully specified + implemented (map onto existing code). Seam methods are declared
with a `#[cold] #[inline(never)]` default body returning `Err(RhiError::Unsupported("..."))`, and their
descriptor types are minimal placeholders. A backend overrides them when the feature lands → the trait stays
stable across phases (Phase 6 fills in `create_texture` with no ABI break). Foundation code never calls seam
methods.

---

## Public API (trait surface)

```rust
// ----- api.rs -----
/// Umbrella trait: collects every backend resource type (wgpu-hal `Api` shape).
/// NOT object-safe. Static dispatch only.
pub trait RhiApi: Sized + 'static {
    // ===== FOUNDATION-NOW (headless compute) =====
    type Device:         RhiDevice<Self>;
    type Queue:          RhiQueue<Self>;
    type CommandEncoder: RhiCommandEncoder<Self>;
    type Buffer;            // owned (Vulkan: BoundBuffer)
    type ShaderModule;      // owned
    type ComputePipeline;   // owned (Vulkan: ComputePipeline)
    type Fence;             // owned
    // ----- DEFERRED SEAM (Phase 2-3 on-screen; Phase 6+ SDF): declared, no trait bound + no impl yet -----
    // Plain associated types now; their operational-trait bounds (RhiSurface/RhiSwapchain) and the
    // Semaphore/ImageBarrier surface are ADDED in Phase 2-3, when the on-screen path moves in-trait.
    type Surface;           // seam: Phase 2-3 (concrete `Surface` used directly meanwhile)
    type Swapchain;         // seam: Phase 2-3
    type Semaphore;         // seam: Phase 2-3 (per-frame acquire/render-finished)
    type Texture;           // seam: Phase 6+ SDF 3D storage image
    type Sampler;           // seam: Phase 6+
    type GraphicsPipeline;  // seam: Phase 6+ dynamic-rendering graphics
    type BindGroup;         // seam: Phase 6+ descriptor set
    type BindGroupLayout;   // seam: Phase 6+
}

// ----- device.rs -----
pub trait RhiDevice<A: RhiApi> {
    type Error: core::fmt::Debug + From<RhiError>;

    // ===== FOUNDATION-NOW =====
    fn create_buffer(&self, desc: &BufferDesc) -> Result<A::Buffer, Self::Error>;
    /// # Safety: GPU must not be using `buffer` (fence-waited); called once.
    unsafe fn destroy_buffer(&self, buffer: A::Buffer);
    fn buffer_mapped_ptr(&self, buffer: &A::Buffer) -> Option<core::ptr::NonNull<u8>>;

    fn create_shader_module(&self, spirv: &[u32]) -> Result<A::ShaderModule, Self::Error>;
    /// # Safety: no pipeline referencing it is in flight; called once.
    unsafe fn destroy_shader_module(&self, module: A::ShaderModule);

    fn create_compute_pipeline(&self, desc: &ComputePipelineDesc<A>) -> Result<A::ComputePipeline, Self::Error>;
    /// # Safety: no submission using it is pending; called once.
    unsafe fn destroy_compute_pipeline(&self, pipeline: A::ComputePipeline);

    fn create_fence(&self, signaled: bool) -> Result<A::Fence, Self::Error>;
    /// # Safety: not pending; called once.
    unsafe fn destroy_fence(&self, fence: A::Fence);
    fn wait_fence(&self, fence: &A::Fence, timeout_ns: u64) -> Result<(), Self::Error>;
    fn reset_fence(&self, fence: &A::Fence) -> Result<(), Self::Error>;

    fn create_command_encoder(&self) -> Result<A::CommandEncoder, Self::Error>;
    /// # Safety: not pending; called once.
    unsafe fn destroy_command_encoder(&self, enc: A::CommandEncoder);

    fn wait_idle(&self) -> Result<(), Self::Error>;   // teardown belt-and-braces (vkDeviceWaitIdle)

    // ===== DEFERRED SEAM (Phase 5/6+) — default-erroring stubs =====
    #[cold] #[inline(never)]
    fn create_texture(&self, _d: &TextureDesc) -> Result<A::Texture, Self::Error> {
        Err(RhiError::Unsupported("create_texture").into())
    }
    // create_sampler, create_graphics_pipeline, create_bind_group_layout, create_bind_group,
    // map_buffer/unmap_buffer (non-coherent) — same stub shape.
}

// ----- queue.rs -----
pub trait RhiQueue<A: RhiApi> {
    type Error: core::fmt::Debug + From<RhiError>;
    /// Submit one recorded encoder, signaling `fence` on completion (no semaphores = headless).
    fn submit(&self, encoder: &A::CommandEncoder, signal_fence: &A::Fence) -> Result<(), Self::Error>;
    // submit_windowed (semaphore-waited present submit) is a Phase-2-3 seam — NOT Phase 1.
}

// ----- encoder.rs (HOT recording path: no dyn, no alloc) -----
pub trait RhiCommandEncoder<A: RhiApi> {
    type Error: core::fmt::Debug + From<RhiError>;
    fn begin(&mut self) -> Result<(), Self::Error>;     // one-time-submit reset+begin
    fn end(&mut self)   -> Result<(), Self::Error>;
    fn bind_compute_pipeline(&mut self, p: &A::ComputePipeline);
    fn bind_storage_buffer(&mut self, buffer: &A::Buffer, set: u32, binding: u32);
    fn push_constants(&mut self, stage: ShaderStage, offset: u32, bytes: &[u8]);
    fn dispatch(&mut self, gx: u32, gy: u32, gz: u32);
    fn pipeline_barrier(&mut self, barrier: &BarrierDesc);   // BUFFER-only (§5.5; no auto tracking)
    // ===== DEFERRED SEAM =====
    #[cold] #[inline(never)]
    fn dispatch_indirect(&mut self, _b: &A::Buffer, _off: u64) { /* unimplemented seam */ }
    // Phase 6+: begin_rendering/end_rendering, bind_graphics_pipeline, bind_group.
}

// RhiSurface / RhiSwapchain / submit_windowed / image-barrier surface are DEFERRED to Phase 2-3 (on-screen
// in-trait). They are intentionally NOT part of the Phase-1 trait surface — the concrete `Surface`/
// `Swapchain`/`Renderer` are used directly meanwhile (scope decision; C1/C3 resolution).
```

### Q1 resolution — where the shared compute layouts live after `ComputeHarness` dissolves (W2)

- **The `Device` owns/caches the fixed foundation `VkDescriptorSetLayout` + `VkPipelineLayout`** (one
  STORAGE_BUFFER @ set0/binding0 + a 4-byte push-constant range). Created once (lazily cached at first
  `create_compute_pipeline`, or at device init); `create_compute_pipeline` consumes the cached pipeline
  layout (it is needed at pipeline-create time). This is exactly the surface the **Phase-6 bind-group seam
  replaces** — `bind_storage_buffer`'s fixed layout is explicitly what `create_bind_group_layout`/
  `create_bind_group` supersede.
- **The `CommandEncoder` owns its `VkCommandPool` + command buffer + `VkDescriptorPool` + the descriptor
  set** (matching `ComputeHarness`'s ownership). The descriptor set is allocated **once** at
  `create_command_encoder` (preserving the harness's "set built once in `new`" property — NO per-record
  `vkUpdateDescriptorSets` regression). `bind_storage_buffer` updates the set's binding **only when the
  bound buffer differs from the cached binding** (the foundation binds once per recording, so the update
  fires at most once); the actual `vkCmdBindDescriptorSets` is recorded at `dispatch`. `push_constants` and
  the bind reference the **Device's** shared pipeline layout (the cross-object coupling W2 flagged — resolved
  by homing the pipeline layout in the Device and lending it to the encoder via `&A::Device` at record
  setup, or caching a copy of the layout handle in the encoder at creation).

**Backend construction (loader/instance/device boot) stays a backend-specific constructor**
(`Vulkan::boot(InstanceConfig) -> Result<VulkanContext, BootError>`), NOT a trait method — boot is inherently
backend-shaped (DX12 has no `VkInstance`). The trait begins at the `Device`/`Queue` boot produces.

## Descriptors (POD)

```rust
#[repr(C)]
pub struct BufferDesc { pub size: u64, pub usage: BufferUsage, pub location: MemoryLocation }

pub struct ComputePipelineDesc<'a, A: RhiApi> {
    pub module: &'a A::ShaderModule,
    pub entry: &'a core::ffi::CStr,     // "main"
    pub push_constant_bytes: u32,       // 4, today
}

#[repr(C)]
pub struct BufferBarrier<'a, A: RhiApi> {
    pub buffer: &'a A::Buffer,
    pub src_access: BarrierAccess,
    pub dst_access: BarrierAccess,
}
pub struct BarrierDesc<'a, A: RhiApi> {
    pub src_stage: BarrierStage,
    pub dst_stage: BarrierStage,
    pub buffers: &'a [BufferBarrier<'a, A>],     // foundation: 0 or 1
    // NO `images` field in Phase 1 — `ImageBarrier` (old_layout/new_layout/subresource_range) is a Phase-2-3
    // seam (C1). The concrete `Renderer` records its UNDEFINED->COLOR->PRESENT image barriers directly.
}
```

No struct exceeds a cache line for the foundation cases; barriers are stack locals walked once by the backend.
False-sharing is N/A — recording is single-threaded on the dispatcher in the apply-window (§5.3).

---

## Multithreading model

- **Single-threaded at the RHI boundary.** Per §5.3, the RHI `Device`/`Queue`/`Swapchain` live in a
  `NonSendResource` and are touched only by the dispatcher during the apply-window (`running == 0`) —
  disjoint from concurrent workers → the retired-Arena `!Send + !Sync` discipline applies with no new
  soundness model.
- **`RhiApi` resource types are `!Send + !Sync` by default** (wrap raw handles + borrow `&DeviceFns`). The
  trait does NOT require `Send`/`Sync`. No atomics in this crate.
- **One sync point** — `wait_fence`. No locks. Race-freedom by single-ownership + `!Send`/`!Sync` (the type
  system forbids the resources crossing threads). `par_iter` over CPU archetypes never touches a GPU resource
  (a CPU `Access` never matches a GPU archetype, §5.4) → no aliasing.

---

## Concrete `boyko_rhi_vulkan` → trait refactor map

| Existing item | Becomes / implements | Migration |
|---|---|---|
| `struct Vulkan;` (new ZST) | `impl RhiApi for Vulkan` | new ~30-line `rhi_impl.rs`; binds all associated types |
| `VulkanContext` | `A::Device` + `A::Queue` views | `impl RhiDevice<Vulkan>` (+ `RhiQueue` via thin `VulkanQueue { queue, fns }` or `Device::queue()`); `boot` stays inherent |
| `BootError` | `Self::Error` source | `From`/`Into<RhiError>`; `LoaderUnavailable`→`Unsupported`, `VkError`→`BackendError` keep detail |
| `BoundBuffer` | `Vulkan::Buffer` | unchanged; `create_buffer` wraps `create_bound_buffer`; `buffer_mapped_ptr` returns `.mapped` |
| `HostVisibleBlock` + `SubAllocator` | internal to the Vulkan device's memory manager | unchanged; `create_buffer` routes here (foundation has one host-visible block) |
| `MemoryError` | folds into `Self::Error` | `From<MemoryError>` exists for `ComputeError`; add `Into<RhiError>` |
| `ComputePipeline` (private) | `Vulkan::ComputePipeline` (now pub) | promote visibility; `create_compute_pipeline` wraps `ComputePipeline::new`; relocate fixed set/pipeline layout from harness → device/encoder |
| `ComputeHarness` | **dissolved** into trait calls | Slice-0 scaffold → `Device::create_*` + `CommandEncoder::{begin,bind,dispatch,pipeline_barrier,end}` + `Queue::submit` + `Device::wait_fence`. `run_*` become test code; `golden_*` stay |
| `record_dispatch`/`record_barrier`/`begin_and_bind_set` | `RhiCommandEncoder` methods | bodies move ~verbatim; `record_barrier` builds `VkBufferMemoryBarrier` from `BarrierDesc` |
| descriptor pool/set + cmd pool/buffer + fence | `Vulkan::CommandEncoder` internals + `Device::create_fence` | encoder owns its cmd pool+buffer+descriptor pool+set; created via `create_command_encoder` |
| `VkShaderModule` + `SpirvBlob` | `Vulkan::ShaderModule` + `create_shader_module(&[u32])` | `SpirvBlob`'s align(4) wrapping stays at the call site (test/asset code passes `&[u32]`) |
| `Surface`/`Swapchain`/`Renderer` + `SwapchainError` | **UNTOUCHED this phase** | stay concrete inherent types; bound to plain `RhiApi::{Surface,Swapchain}` associated types (no trait bound yet); `window_present` test keeps driving them directly. Trait abstraction = Phase 2-3 (C1/C3 resolution) |
| `ffi.rs` bitflags (`VK_BUFFER_USAGE_*`, `VK_PIPELINE_STAGE_*`, `VK_ACCESS_*`, `VK_SHADER_STAGE_*` — all u32) | translated FROM `boyko_rhi` enums via `to_vk` | agnostic bitflag values chosen to equal these constants → `to_vk` is identity (`#[inline]`). `VkFormat`/`VkImageLayout` (i32) NOT mapped this phase (W1; Format = Phase-2-3 seam) |
| `ffi.rs` PFN tables, all `unsafe extern` | **stays entirely in `boyko_rhi_vulkan`** | no FFI moves up; `boyko_rhi` never sees a `Vk*` type |

**Net churn:** one new `rhi_impl.rs` + error conversions + relocating the shared compute layouts from
`ComputeHarness` into the device/encoder. No existing `// SAFETY:` block changes meaning; owned-value +
reverse-`Drop` preserved 1:1. The three tests are rewritten to drive the trait (same asserts, same `golden_*`,
same validation-clean oracle).

---

## Foundation-now vs deferred-seam

| Capability | Status | Trait item |
|---|---|---|
| Device/Queue from boot | NOW | `RhiDevice`/`RhiQueue` (boot is backend-inherent) |
| Buffer create/destroy + sub-alloc bind | NOW | `create_buffer`/`destroy_buffer` |
| Host-visible persistent mapped ptr | NOW | `buffer_mapped_ptr` |
| Compute pipeline from SPIR-V | NOW | `create_shader_module` + `create_compute_pipeline` |
| Command encode: begin/end/bind/push/dispatch | NOW | `RhiCommandEncoder` |
| Explicit pipeline barrier (stage/access masks) | NOW | `pipeline_barrier(&BarrierDesc)` |
| Submit + fence wait/reset | NOW | `RhiQueue::submit`, `RhiDevice::wait_fence`/`reset_fence` |
| Image-layout barriers (`ImageBarrier`) | SEAM (Phase 2-3) | concrete `Renderer` records them directly meanwhile (C1) |
| Surface + swapchain in-trait (`RhiSurface`/`RhiSwapchain`) + semaphores + present submit | SEAM (Phase 2-3) | concrete `Surface`/`Swapchain`/`Renderer` used directly meanwhile (C3) |
| `Format` enum + i32 layout mapping | SEAM (Phase 2-3) | W1 — not needed by headless compute |
| Explicit map/unmap + flush (non-coherent) | SEAM (Phase 5) | `map_buffer`/`unmap_buffer` stubs (device-local `GpuColumn` staging, O2) |
| Textures / 3D storage images (SDF) | SEAM (Phase 6+) | `create_texture`, `TextureDesc` placeholder |
| Graphics pipelines + dynamic-rendering attachments | SEAM (Phase 6+) | `create_graphics_pipeline`, `begin_rendering` |
| Descriptor sets / bind groups + layouts | SEAM (Phase 6+) | `create_bind_group(_layout)`, replaces the fixed compute layout |
| `vkCmdDispatchIndirect` | SEAM (Phase 6+) | `dispatch_indirect` stub |
| `Slot ↔ u64` packing bridge | NOW (defined) | `handle::slot_to_u64`/`u64_to_slot` (core's `DeviceColumnHandle` is Phase 4, in `boyko_ecs`) |

---

## Implementation plan (steps + waves)

**Wave A (parallel — independent files in the new crate):**
1. `Cargo.toml` (deps: `boyko_utils` only) + `lib.rs` skeleton + workspace member; `error.rs` (`RhiError`,
   `#[cold]` conversions).
2. `enums.rs` — thin bitflag/enum set (D5), bitflag values equal to Vulkan constants (no `Format` this phase).
3. `descriptor.rs` — `BufferDesc`, `ComputePipelineDesc`, `BarrierDesc`/`BufferBarrier` (buffer-only).

**Wave B (serial — the trait definitions reference each other):**
4. `api.rs` — `RhiApi` umbrella (foundation associated types + plain seam associated types, no bounds).
5. `device.rs`, `queue.rs`, `encoder.rs` — operational traits (D1–D4, D7). No `surface.rs` this phase.
6. `handle.rs` — `slot_to_u64`/`u64_to_slot` (release-checked, u32-capped) + packing tests, typed handles,
   `ResourceRegistry<A>` (+ `destroy_all(&device)`).
7. `boyko_rhi` compiles + builds clean alone (no backend). Unit-test the registry + handle packing here
   (no GPU; Miri-clean).

**Wave C (serial — backend refactor, one worktree, gated by the regression oracle; COMPUTE PATH ONLY):**
8. `boyko_rhi_vulkan`: add `boyko_rhi` dep; `rhi_impl.rs` with `struct Vulkan;` + `impl RhiApi` (compute
   associated types bound; `Surface`/`Swapchain`/`Semaphore`/… = the concrete types bound to the unbounded
   seam associated types).
9. Unify the backend error enums into one `VulkanError`; hand-write `impl From<VulkanError> for RhiError`
   (`#[cold]`). `swapchain.rs`'s `SwapchainError` may stay separate (on-screen path untouched).
10. `impl RhiDevice for VulkanContext` (Device caches the shared descriptor-set + pipeline layouts — Q1);
    `impl RhiQueue` (thin `VulkanQueue` wrapper — Q2).
11. `impl RhiCommandEncoder` (encoder owns cmd pool+buffer + descriptor pool+set; move `record_dispatch`/
    `record_barrier`/`begin_and_bind_set` in; build `VkBufferMemoryBarrier` from `BufferBarrier`).
12. Dissolve `ComputeHarness` into trait-driven test scaffolding (it was a Slice-0 test bundle).

**Wave D (serial — prove the oracle still holds):**
13. Rewrite `tests/compute.rs` + `tests/roundtrip.rs` to drive the trait (Device→Encoder→Queue→fence); keep
    `N`, `golden_*`, `assert_validation_clean`. **`tests/window_present.rs` is UNTOUCHED** (drives the
    concrete `Renderer`).
14. Add a GPU-backed registry register/resolve/take round-trip in the Vulkan crate.

Wave A items 1–3 are mutually parallel; B is serial; C is serial (shared files); D follows C.

---

## Validation

**Regression oracle (mandatory, unchanged behavior):** `tests/compute.rs` + `tests/roundtrip.rs` (rewritten
to drive the trait) and `tests/window_present.rs` (**untouched**, drives the concrete `Renderer`) must all
pass post-refactor on the RTX 3060 (graceful-skip on GPU-less CI preserved). The chained-barrier golden
proves `pipeline_barrier(&BarrierDesc)` lowering is correct — a wrong/missing barrier trips sync-validation
→ fail.

**New unit tests (no GPU — pure logic, Miri-clean):**
- `handle`: `u64_to_slot(slot_to_u64(s)) == s` for boundary slots (index 0, **`u32::MAX` index** (the
  capped-domain max, C2), generation 0 / `u32::MAX`); index low / generation high. Plus an
  index-`> u32::MAX` case that the release-present `assert` rejects (a `#[should_panic]` test).
- `ResourceRegistry`: register → resolve returns the value; `take` removes it; a stale handle (after
  take+re-register, ABA) resolves to `None`; `destroy_all` empties every map.
- `enums`: `BufferUsage::STORAGE.to_vk() == VK_BUFFER_USAGE_STORAGE_BUFFER_BIT` (in the Vulkan crate).
- **Property:** registry op-sequences never alias or resolve a stale handle (proptest, pure-logic).

**No new benchmarks** — the trait adds zero abstract call sites in the hot path (monomorphized to the same
`fns.cmd_*` calls). Byte-identical-codegen validated by the unchanged golden tests.

**`assert`/`debug_assert!` invariants:** handle packing index `<= u32::MAX` is a **release-present `assert`**
(C2); `ResourceRegistry::destroy_all`/`Drop` assert every map empty (leak catch, secondary to the structural
drain, W4); `create_shader_module` non-empty word slice; `dispatch` non-zero group counts;
`pipeline_barrier` non-empty stages when a barrier is present.

---

## Resolved findings (architecture-critic gate — verdict was REVISE; all CRITICAL/IMPORTANT now closed)

- **C1 (image-layout barriers undercounted) — RESOLVED by scope:** image barriers + `ImageBarrier` are a
  genuine Phase-2-3 seam; Phase-1 `BarrierDesc` is buffer-only; the concrete `Renderer` keeps recording its
  UNDEFINED→COLOR→PRESENT image barriers directly. (Scope decision at top.)
- **C2 (handle-packing width) — RESOLVED:** `slot_to_u64` carries a **release-present `assert`** (not
  `debug_assert`), the registry index domain is **capped at `u32::MAX`** (documented hard limit), and the
  boundary test covers index `u32::MAX` + the over-domain rejection. (D6.)
- **C3 (Q3 contradiction: on-screen NOW vs out-of-trait) — RESOLVED:** the on-screen path
  (`Surface`/`Swapchain`/`Renderer` + semaphores + present submit) is **out of the Phase-1 trait surface**;
  `RhiSurface`/`RhiSwapchain`/`Semaphore` are plain unbounded seam associated types; concrete types are used
  directly; `window_present` untouched. (Scope decision + trait surface.)
- **W1 (Format/layout are i32, not u32 bitflags) — RESOLVED:** identity-cast claim scoped to the u32 bitflag
  families; `Format` deferred to the Phase-2-3 on-screen seam. (D5.)
- **W2 (ComputeHarness layout-ownership churn / Q1) — RESOLVED:** Device caches the shared descriptor-set +
  pipeline layouts; Encoder owns cmd pool+buffer + descriptor pool+set built once (no per-record
  `vkUpdateDescriptorSets` regression). (Q1 resolution block.)
- **W3 (Q5 error-model coherence wall) — RESOLVED:** one unified per-backend `Error`; bound is
  `From<RhiError>` only; hand-written `impl From<VulkanError> for RhiError` (no blanket → no collision);
  conversions `#[cold]`. (D4.)
- **W4 (drain-before-drop under-specified) — RESOLVED:** `ResourceRegistry::destroy_all(&device)` is a
  structural release-present teardown step (`wait_idle` then reverse-order `destroy_*`), with the
  debug-assert-empty as a secondary tripwire. (D6.)
- **O1 (Q2 queue separation) — ACCEPTED:** thin `VulkanQueue` wrapper (matches wgpu-hal + DX12). (Step 10.)
- **O2 (Q6 map/unmap seam) — ACCEPTED:** deferred to a Phase-5 stub; device-local `GpuColumn` will need
  staging+flush (host-coherent mapping does not extend to device-local). (Capability table.)
- **O3 (0%-gate + handle home + dependency direction) — RESOLVED:** `boyko_ecs` untouched this phase;
  `boyko_rhi` deps = `boyko_utils` only; core's `DeviceColumnHandle` lives in `boyko_ecs` (Phase 4),
  preserving the §3 `boyko_rhi → boyko_ecs` arrow. (Central decision (b).)
- **O4 (zero-overhead) — CONFIRMED:** static-dispatch monomorphizes to the same `fns.cmd_*` calls; `#[cold]`
  seam-stub defaults are free when overridden; error conversions cold. (D4/D7.)
- **O5 (scope) — CONFIRMED:** seam associated types declared now (cheap; avoids a later ABA break);
  no over-building beyond the now-resolved image-barrier issue.
