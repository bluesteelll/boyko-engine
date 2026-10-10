# RHI & Vulkan Backend

> Boyko Engine talks to the GPU through its **own** Render Hardware Interface (RHI) — a static-dispatch trait seam with no `dyn`, no `Box`, and no foreign FFI leaking through the public surface — implemented by a hand-rolled, raw-FFI Vulkan backend (no `ash`, no `vulkano`).

## What it is

Two crates form the GPU boundary:

- [`boyko_rhi`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi/src/lib.rs) — the **backend-agnostic seam**. It declares device, queue, encoder, resource, and handle abstractions as traits and POD descriptors. It names no Vulkan type and links no FFI.
- [`boyko_rhi_vulkan`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi_vulkan/src/lib.rs) — the **Vulkan backend**, hand-declared raw FFI over the Vulkan loader. It implements the `boyko_rhi` traits.

The split exists so the rest of the engine compiles against the seam, never against Vulkan. A second backend (DX12 / Metal) would slot in by implementing the same traits, with no change to callers. See [Rendering Overview](overview.md) for where this sits in the frame, and [GPU Columns](gpu-columns.md) for the ECS-resident storage that rides on top.

## Why in-house, not wgpu

The engine's first principle is **zero abstraction overhead on the hot path**. A `dyn`-based HAL (wgpu-hal's object-safe `trait Api`) pays a virtual call per command. Boyko's seam is deliberately **not object-safe**: every call monomorphizes to a direct, non-virtual call, so the abstraction costs the same as calling the backend's inherent method. The module doc states this directly — "every call monomorphizes to a direct, non-virtual call, so there is zero abstraction overhead vs the backend's inherent methods" ([lib.rs](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi/src/lib.rs)).

The Vulkan FFI is hand-rolled for the same reason a third-party allocator was rejected for [memory](../memory/arena.md): the engine wants full control of the calling convention, the loader dispatch, and the soundness story, with no transitive supply-chain surface. The Vulkan command functions are not even linked at build time — they are resolved at runtime through `vkGetInstanceProcAddr` / `vkGetDeviceProcAddr`, mirroring the virtual-memory FFI idiom of `boyko_memory`'s `vm.rs` ([ffi.rs](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi_vulkan/src/ffi.rs)).

> **Honest scope note.** "No FFI through the seam" applies to `boyko_rhi`, which is FFI-free. The Vulkan *path* in `boyko_rhi_vulkan` is 100% in-house FFI (no `ash` / `vulkano` / `libc`). The one exception is **OS windowing and Raw-Input**, which use the official Microsoft `windows-sys` bindings (`SetWindowLongPtrW`, `RegisterRawInputDevices`, the `RAWINPUT*` structs, `WM_*` constants), target-gated to `cfg(windows)` ([Cargo.toml](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi_vulkan/Cargo.toml)). The `vk*` surface itself never touches `windows-sys`.

## The seam: `RhiApi` and the operational traits

The seam is shaped after `wgpu-hal`'s `Api` marker, minus the `dyn`. One umbrella trait gathers every backend resource type; the operational traits are separate and reference `A: RhiApi`.

```mermaid
classDiagram
    class RhiApi {
        <<trait, Sized + 'static>>
        type Device
        type Queue
        type CommandEncoder
        type Buffer
        type ShaderModule
        type ComputePipeline
        type Fence
        type QueryPool
        type Texture
        type TextureView
        type Sampler
        type GraphicsPipeline
        type BindGroup
        type BindGroupLayout
        type AccelerationStructure
        type Surface
        type Swapchain
        type Semaphore
    }
    class RhiDevice {
        <<trait>>
        create_buffer() / create_shader_module()
        create_compute_pipeline() / create_graphics_pipeline()
        create_texture() / create_texture_view() / create_sampler()
        create_bind_group_layout() / create_bind_group()
        create_query_pool() / read_query_pool_*()
        create_acceleration_structure()
        get_buffer_device_address()
        create_command_encoder() / buffer_mapped_ptr()
        wait_fence() / reset_fence() / wait_idle()
        map_buffer() / unmap_buffer() : unsupported seam
    }
    class RhiQueue {
        <<trait>>
        submit(encoder, signal_fence)
    }
    class RhiCommandEncoder {
        <<trait>>
        begin() / end()
        bind_compute_pipeline() / bind_storage_buffer() / dispatch()
        pipeline_barrier() / image_barrier()
        begin_rendering() / end_rendering()
        bind_graphics_pipeline() / bind_descriptor_set_at()
        bind_vertex_buffer() / bind_index_buffer()
        draw() / draw_indexed()
        copy_buffer() / copy_buffer_to_image() / blit_image()
        write_timestamp()
        cmd_build_acceleration_structures()
        dispatch_indirect() : no-op seam
    }
    RhiApi --> RhiDevice : Device
    RhiApi --> RhiQueue : Queue
    RhiApi --> RhiCommandEncoder : CommandEncoder
```

- [`RhiApi`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi/src/api.rs) — the umbrella marker, bound `Sized + 'static`. A backend is a **zero-sized struct** (`struct Vulkan;`) that implements it. Its associated types name the backend's owned resources (buffers, shaders, pipelines, fences, query pools, textures and views, samplers, bind groups, acceleration structures) and its operational types (`Device`, `Queue`, `CommandEncoder`). Only the three operational types carry trait bounds; every resource type is a plain, unbounded associated type. `Surface`, `Swapchain` and `Semaphore` are declared, but no trait method takes them: the on-screen present is concrete in the backend (below).
- [`RhiDevice`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi/src/device.rs) — resource lifecycle and synchronization: buffers, shader modules, compute and graphics pipelines, textures, views and samplers, bind-group layouts and bind groups, fences, encoders, timestamp query pools (with the `read_query_pool_*` readers), and ray-tracing acceleration structures with their build sizes and buffer device addresses. It also exposes a host-visible buffer's persistent CPU pointer via `buffer_mapped_ptr` (`Option<NonNull<u8>>`, `None` when the buffer is not host-mappable). Only `map_buffer` / `unmap_buffer`, a seam for non-coherent device-local staging, are still unimplemented: both carry `#[cold] #[inline(never)]` default bodies that return `Err(RhiError::unsupported(...))`.
- [`RhiQueue`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi/src/queue.rs) — submission. Its `submit` signals a `Fence` and takes no semaphores; the semaphore-waited present submit lives in the backend's concrete `present` module.
- [`RhiCommandEncoder`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi/src/encoder.rs) — the hot recording path. The compute verbs (`begin` → `bind_compute_pipeline` → `bind_storage_buffer` → `push_constants` → `dispatch` → `pipeline_barrier` → `end`) are joined by buffer copies, image barriers, image copies and blits, dynamic rendering (`begin_rendering` / `end_rendering`), graphics binds, viewport and scissor, vertex and index buffers, `draw` / `draw_indexed`, timestamp writes, and acceleration-structure builds. Each method lowers to a direct `(fns.cmd_*)` indirect call — byte-identical codegen to a hand-written inherent Vulkan method. `dispatch_indirect` is declared but still a no-op default that the Vulkan backend does not override.

Errors flow through one unified [`RhiError`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi/src/error.rs); each operational trait carries an associated `Error: From<RhiError>` so the backend can widen it.

### Current scope

The seam covers compute **and** graphics today: buffers, textures, samplers, compute and graphics pipelines, bind groups, dynamic rendering, draws, copies and blits, timestamp queries, and acceleration structures. The Vulkan backend implements all of it ([`rhi_impl/device.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi_vulkan/src/rhi_impl/device.rs), [`rhi_impl/encoder.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi_vulkan/src/rhi_impl/encoder.rs)). Three methods remain unimplemented seams: `map_buffer` / `unmap_buffer` and `dispatch_indirect`. A seam method carries a `#[cold]` default body (an `Unsupported` error, or a no-op), and a backend overrides it when the feature lands. (The crate's own module doc still describes the older "headless compute only" scope; the trait bodies above are current.)

Some of the backend stays concrete, used directly off its own types rather than through the seam:

- **The on-screen present** — the [`present`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_rhi_vulkan/src/present) module tree: surface, swapchain, the frame driver, the per-pass recorders in `present/passes/`, and the framegraph bridge. `swapchain.rs` is now a re-export shim over it.
- **Bindless descriptor sets** — the textured-PBR texture array (`bindless.rs`) and the Visibility Buffer's per-mesh geometry arrays (`geometry_bindless.rs`).
- **GPU timestamp zones** — `present/gpu_zone.rs` records per-pass GPU timings for the profiler.
- [`window`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi_vulkan/src/window.rs) — the raw Win32 window and Raw-Input.

### Synchronization: the framegraph

The on-screen frame does not hand-write its barriers. Each pass **declares** the resources it reads and writes, with the pipeline stage, access and image layout. `FrameGraph::compile` then runs a per-resource synchronization state machine and **derives** the minimal `vkCmdPipelineBarrier` set, and each pass's recorder emits its share. The graph is reset and re-declared every frame without allocating, and the render path picks the declarator (`declare_deferred_graph`, `declare_forward_graph`, `declare_vb_graph`). See [The framegraph](framegraph.md) and the source in [`framegraph/`](https://github.com/bluesteelll/boyko-engine/tree/master/crates/boyko_rhi_vulkan/src/framegraph).

Host writes have their own proof. `Renderer::wait_frame_in_flight` returns a `FrameWriteToken`, which shows that the frame slot's fence was waited. Per-slot host-write APIs take the token instead of a slot index, and the frame-ending submit consumes it, so a host write to a slot the GPU may still read is a compile error.

## Handles: generational, `u64`-bridgeable

Owned GPU resources are not handed around by reference. Each lives in a [`ResourceRegistry`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi/src/handle.rs) behind a typed, generational handle.

- [`BufferHandle`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi/src/handle.rs), [`ComputePipelineHandle`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi/src/handle.rs), `ShaderHandle`, `FenceHandle` — each is `#[repr(transparent)]` over a [`Slot`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_utils/src/identifiers/slot.rs) (generational index). The handle *is* the index; it carries no extra footprint.
- The registry stores one `SparseSlotMap` per resource kind (struct-of-arrays). `resolve_*` is a generation-checked array index — the same O(1) lookup the [entity store](../architecture/entities-and-generations.md) uses — with no `dyn`, no `Box`, no `HashMap`.
- A stale handle (one whose generation no longer matches) resolves to `None` instead of aliasing a recycled slot. This is the ABA guarantee that makes recycling safe.

### The `u64` bridge to the ECS

The ECS core (`boyko_ecs`) is **graphics-pure** — it names no Vulkan or RHI type. A GPU-resident column stores its rows behind an opaque [`DeviceColumnHandle`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/memory/device_column.rs): a bare `#[repr(transparent)] u64` that the engine "neither interprets nor dereferences" ([device_column.rs](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/memory/device_column.rs)).

The render side packs a registry `Slot` into that `u64` and back through the seam's bridge functions [`slot_to_u64`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi/src/handle.rs) / [`u64_to_slot`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi/src/handle.rs) (generation in the high 32 bits, index in the low 32). This keeps two invariants at once: the RHI trait never names `u64`, and the ECS never names the registry.

```mermaid
flowchart LR
    A["boyko_ecs<br/>DeviceColumnHandle(u64)<br/>(graphics-pure, opaque)"] -->|u64_to_slot| B["boyko_rhi<br/>Slot (generational)"]
    B --> C["ResourceRegistry&lt;Vulkan&gt;<br/>resolve_buffer(slot)"]
    C --> D["&Vulkan::Buffer<br/>(device-local VRAM SSBO)"]
```

Because the `u64` is a `Copy` POD — never a pointer — a `ComponentPool` carrying one is trivially `Send + Sync` with respect to that field, and the handle may even change on a device-side grow without invalidating any cached CPU pointer (no CPU code caches a device row pointer).

## Why a `DeviceColumnHandle` is a bare `u64`

This is the seam that lets a component column live in **VRAM** while the ECS stays graphics-pure. A device-resident `ComponentPool` keeps its host-side `len` at `0` for its whole life — the live device row count lives only on the device side — so the CPU drop loop is a no-op and nothing on the CPU dereferences device memory. The backend allocates a **device-local SSBO**, and the ECS holds only the opaque token. The full storage story is in [GPU Columns](gpu-columns.md).

## Making `!Send` GPU access compiler-enforced

A Vulkan context is `!Send + !Sync` by construction — the queue and encoder hold raw `*const DeviceFns` into the owning context and must be touched from a single thread ([rhi_impl/mod.rs](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi_vulkan/src/rhi_impl/mod.rs)). The engine runs systems in parallel across a work-stealing pool, so the open question is: *how does a parallel scheduler ever touch a `!Send` resource without unsafe-by-convention?*

The answer is the **DispatcherToken** (the "Option C" design in [`boyko_ecs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/core/system/dispatcher_token.rs)). It is a dispatcher-only capability that projects a `!Send` resource, and it makes the soundness story a **compiler check** rather than a comment:

- **A worker can never reach one.** The token is minted *only* by the scheduler on the dispatcher-solo path (when `running == 0`, no worker live) and by `EcsMaster::run_system_once`. It is passed by value to `System::run_dispatcher`; CPU systems use the default forwarder and never see it. The `!Send` projection is structurally unreachable from a worker thread ([dispatcher_token.rs](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/core/system/dispatcher_token.rs)).
- **No two live `&mut R` can alias.** `DispatcherToken::nonsend_resource_mut` ties the returned `&mut R` to `&mut self`, *not* to the world's lifetime. A second projection cannot alias the first — the borrow checker forbids holding two `&mut self` borrows of the token at once ([dispatcher_token.rs](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/core/system/dispatcher_token.rs)).
- **The token is neither `Copy` nor `Clone`**, deliberately: a `Copy` token would let a system mint two independent handles and re-open the aliasing hole ([dispatcher_token.rs](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/core/system/dispatcher_token.rs)).
- A debug-only `owning_thread` stamp trips an `assert_eq!` if a projection ever runs off the minting thread — a release-free tripwire for a routing mistake.

```mermaid
sequenceDiagram
    participant S as Scheduler
    participant D as Dispatcher thread (running == 0)
    participant G as GpuSystem (boyko_render)
    participant R as RhiContext (!Send)

    S->>D: apply window opens, workers idle
    D->>G: run_dispatcher(DispatcherToken)
    G->>G: token.nonsend_resource_mut::<RhiContext>()
    G->>R: bind → push count → dispatch(ceil(len/64)) → submit → fence
    R-->>G: () (no readback)
    Note over G,R: &mut R is tied to &mut self<br/>a second projection cannot alias it
```

The render-side consumer is [`GpuSystem`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/gpu_system.rs) — a hand-written `boyko_ecs` `System` that declares **empty** component/resource access (no conflict-graph edges), is scheduled as `SystemKind::GpuCompute` so it runs solo on the dispatcher inside the apply window, and reaches the `!Send` `RhiContext` only through the token ([gpu_system.rs](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_render/src/gpu_system.rs)). It stores a `(ArchetypeId, ComponentId)` target key, never a raw `DeviceColumnHandle`, so a device grow that rotates the handle is transparent. This is the engine's **CPU-orchestrate / GPU-execute, zero-readback** discipline: the CPU records and submits; the GPU column is never copied back per frame.

The capability is enforced by `compile_fail` tests — e.g. [`token_double_mut_aliases_rejected.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/tests/compile_fail_dispatcher_token/token_double_mut_aliases_rejected.rs) asserts that minting two aliasing `&mut R` from one token does not compile.

## A compute dispatch through the seam

The headless compute path is exercised end to end against the seam. The following sketch shows the real method names; mark it `ignore` because booting a device needs a GPU and the exact wiring lives in the backend's tests.

```rust,ignore
use boyko_rhi_vulkan::device::{InstanceConfig, VulkanContext};
use boyko_rhi_vulkan::memory::HostVisibleBlock;
use boyko_rhi_vulkan::ffi::VK_BUFFER_USAGE_STORAGE_BUFFER_BIT;

// Boot a headless device (returns Err on a GPU-less machine).
let ctx = VulkanContext::boot(InstanceConfig::default()).expect("no GPU");

// Sub-allocate a host-visible storage buffer from one large block.
let mut block = HostVisibleBlock::new(
    ctx.device(),
    ctx.device_fns(),
    ctx.memory_properties(),
    16 * 1024 * 1024,
)
.expect("alloc");
let bound = block
    .create_bound_buffer(4096, VK_BUFFER_USAGE_STORAGE_BUFFER_BIT)
    .expect("buffer");
// `bound.mapped` is `Some(ptr)` for a host-visible block: the CPU pointer to the
// buffer's first byte (a device-local block carries `None`).
let _ = bound;
```

The hot recording sequence on the encoder (begin → `bind_compute_pipeline` → `bind_storage_buffer` → `push_constants` → `dispatch(ceil(count / 64), 1, 1)` → `pipeline_barrier` → end), then `queue.submit(&encoder, &fence)` and `device.wait_fence(&fence)`, is recorded once and submitted once. In production the single readback used by the backend's tests is **only** a test oracle — the per-frame path never reads device memory back.

## Soundness on the raw-FFI path

Miri cannot execute raw Vulkan syscalls, so the backend substitutes the `VK_LAYER_KHRONOS_validation` messenger as the soundness oracle ([lib.rs](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi_vulkan/src/lib.rs)). Every `unsafe` FFI block carries a concrete `// SAFETY:` ABI comment, per [Principle 8](../architecture/principles.md). On the ECS side the device column is `#[cfg(not(miri))]` and collapses to a Miri-modelable host-only fallback, so the rest of the kernel stays Miri-clean.

Two gates hold the backend to that oracle:

- **The absolute validation gate** (`crates/boyko_app/tests/boot_validation_clean.rs`) boots the whole engine through the windowed runner with the layer armed, renders a few frames, tears down, and is red on any error or validation/performance warning. It reads the verdict from the callback's counters, not from log text.
- **The SPIR-V capability census** ([`spirv_capability_census.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi_vulkan/src/spirv_capability_census.rs)) needs no device. It reads every committed `.spv`'s declared capabilities and requires each to be licensed by a device feature the backend enables.

In an ordinary windowed run the layer is off; `BOYKO_ENABLE_VALIDATION` arms it.

## Resource teardown is a hard invariant

A backend resource needs `&Device` to be destroyed, so owned resources cannot self-`Drop`. The owner **must** call [`ResourceRegistry::destroy_all`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi/src/handle.rs) before dropping the registry — a structural, release-present teardown step. Dropping a non-empty registry leaks every live GPU resource; `Drop` emits a release-surviving hard-error diagnostic plus a test-failing `debug_assert!` as tripwires (not the primary guard) ([handle.rs](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi/src/handle.rs)).

## See also

- [Rendering Overview](overview.md) — how the RHI fits the frame
- [The framegraph](framegraph.md) — declared passes, derived barriers, `FrameWriteToken`
- [GPU Columns](gpu-columns.md) — VRAM-resident ECS storage behind `DeviceColumnHandle`
- [SDF Rendering](sdf.md) and [Lighting](lighting.md) — the passes the backend drives
- [Scheduler](../scheduler.md) — where the dispatcher-solo apply window comes from
- Source: [`boyko_rhi`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi/src/lib.rs), [`boyko_rhi_vulkan`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_rhi_vulkan/src/lib.rs), [`dispatcher_token.rs`](https://github.com/bluesteelll/boyko-engine/blob/master/crates/boyko_ecs/src/ecs/core/system/dispatcher_token.rs)
