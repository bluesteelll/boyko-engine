> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 5 — `GpuColumn` + one `GpuSystem` + the `boyko_render` crate

> **Status: REVISED (architect → 3-lens critic panel [REVISE×3, vs merged code] → architect revise; all 8
> must-fixes resolved).** Branch `ecs`, 2026-06-17. **The "Post-Critic Binding Revisions" section at the BOTTOM
> is AUTHORITATIVE** where it conflicts with the decisions below. Core realization: Phase 4 reserved the seam
> TYPES but NOT the production WIRING — Phase 5 ADDS the wiring (it touches `boyko_ecs` core MORE than this
> plan originally implied; see the boyko_ecs-additions list). Implements
> [docs/RENDER-PHYSICS-GPU-PLAN.md](../RENDER-PHYSICS-GPU-PLAN.md) §7 Phase 5 + §0/§1/§2/§5/§6. Consumes the
> Phase-4 core seams ([docs/PHASE4-CORE-SEAMS-PLAN.md](PHASE4-CORE-SEAMS-PLAN.md)) + the Phase-1 RHI
> ([docs/RHI-TRAIT-PLAN.md](RHI-TRAIT-PLAN.md)).

## Goal / thesis

Prove the load-bearing GPU-ECS risk on a REAL ECS column: an archetype marked GPU-resident whose column lives
in a device SSBO behind the opaque `DeviceColumnHandle(u64)`; ONE compute system that updates it via
`vkCmdDispatch`, ordered relative to a CPU producer by the existing conflict graph, barrier lowered in
`boyko_render`; **steady state does ZERO per-frame CPU readback** (the single readback is a TEST oracle behind
`NonSendResMut<RhiDevice>`). NOT the SDF engine (Phase 6+). Steady-state frame cost = a few `cmd_*` + one
`vkQueueSubmit` doorbell + the apply-window fence — independent of entity count (Regime-C). 0%-gate preserved
(the Phase-4 residency bit is the only gate).

## Decisions

- **D1 — `GpuColumn` = thin manager owning the device buffer behind the RHI `ResourceRegistry`; core stores
  only the `u64`.** `GpuColumnManager` registers a device buffer → `slot_to_u64` → the `DeviceColumnHandle(u64)`
  the core `PoolBacking::Device` stores. Side map `GpuColumnMeta` (SoA `Vec` indexed by `Slot.index`, NOT a
  HashMap). Resolve = `u64_to_slot`→`resolve_buffer` (~3 ns). A grow bumps the `Slot` generation → a stale
  `u64` resolves to `None` (loud). Rejected: `Box<dyn DeviceColumnBacking>` in core (vtable+heap+impurity);
  HashMap on the resolve path.
- **D2 — GpuColumn is `DeviceLocal` (VRAM) with a host-visible staging buffer for the ONE-TIME upload; steady
  state never maps device memory.** Regime-C demands VRAM-resident streamed at VRAM bandwidth. Initial CPU→GPU
  via staging + `vkCmdCopyBuffer` + one fence at setup; then GPU-owned. Exposes the RHI gap: `create_buffer`
  must accept `DeviceLocal` + a `copy_buffer` encoder method (the Phase-1-reserved O2 seam, filled minimally —
  NOT general non-coherent map/flush). Rejected: host-visible column (not Regime-C; invites accidental CPU
  reads); `vkCmdUpdateBuffer` (64 KiB cap); BAR/ReBAR (not guaranteed; staging is the portable floor).
- **D3 — one new `DeviceLocalBlock` (never mapped); staging reuses the existing `HostVisibleBlock`.** Mirrors
  §4's free-list-with-coalescing pool, applied to VRAM; reuses the proven `SubAllocator`. The DeviceLocalBlock
  is never mapped → no CPU read path to the column exists (zero-readback by construction). Rejected: one
  mixed-type block (a block has one memory-type index); per-resource `vkAllocateMemory` (§4 forbids).
- **D4 — the grow path fills the Phase-4 `grow_rows` Device-arm via `boyko_render`, NOT core.** Core can't touch
  graphics, so growth of a GPU-resident archetype is a `boyko_render`-driven op (GPU owns spawn/despawn, §2).
  Core's Device arm stays `unreachable!` for CPU-initiated grow (a CPU spawn never targets a GPU archetype,
  D4-Phase4); the column grows only through `GpuColumnManager::grow_column` (realloc+copy+fence+re-fetch),
  which writes the new `u64` back into the core pool via a narrow `pub(crate) set_device_handle` (Phase-4
  reserves the field; Phase 5 needs the write path — **OQ1**). Foundation = stable-residency workload (membership
  fixed after setup; only values churn — Regime-C sweet spot); per-frame GPU spawn/despawn (indirect/atomic) is
  Phase 6.
- **D5 — the `GpuSystem` adapter declares `GpuAccessIntent` + carries the compute pipeline; registers as
  `SystemKind::GpuCompute`, runs on the dispatcher.** `access()` empty (touches no CPU column) + `is_gpu=true`
  on its `SystemDescriptor` → `SystemKind::GpuCompute` → `runs_on_dispatcher()` → EXC2 solo dispatch at
  `running==0` — exactly the sound RHI record/submit site (§5.3), after all worker writes are visible.
  `run_unsafe` records (begin→[barrier]→bind→push→dispatch→end)+submit via `NonSendResMut<RhiDevice>`/
  `NonSendRes<RhiQueue>`; `apply` no-op. Rejected: `DispatcherConcurrent` (YAGNI; recording is single-threaded
  by the NonSend RHI contract anyway).
- **D6 — ordering = a directed `.after()` edge; the barrier is lowered from that DIRECTED edge +
  the two systems' `GpuAccessIntent`.** The symmetric `conflict_bits` give ordering-only (no direction → can't
  build a barrier's src/dst); the directed `ConflictGraph::successors` give producer→consumer; the intents give
  the Vulkan stage/access masks. Together → `BarrierDesc`. Lowered at schedule-BUILD time (cold) into a
  per-system `PlannedBarrier` plan, replayed on the frame path (no per-frame graph walk). Superset-correct:
  ambiguous → widen to `ALL_COMMANDS`/`MEMORY_READ|WRITE` rather than omit (a missing barrier trips
  sync-validation, the test gate). Rejected: derive from symmetric bits (no direction); auto-track resource
  state (wgpu-core style, §5.5 forbids); per-frame walk.
- **D7 — the CPU-query-skip of GPU archetypes at query-collection time via `is_gpu_resident()`.** Fills the
  Phase-4-reserved consume-side skip: `if archetype.is_gpu_resident() { continue; }` in the cold archetype
  collection (query rebuild), so `row_ptr` never sees a Device pool. 0%-gate airtight (collection-time, not
  per-row; bit never set in a CPU-only world → identical candidate set → byte-identical hot loop). A CPU query
  NAMING a GPU component silently matches nothing → must be a LOUD debug diagnostic (**OQ4**).
- **D8 — a minimal `DeviceLocal` + staging + `copy_buffer` RHI extension (additive, no ABI break).**
  `create_buffer(DeviceLocal)` routes to the DeviceLocalBlock (`buffer_mapped_ptr → None`);
  `RhiCommandEncoder::copy_buffer(src, dst, &[BufferCopy])` (POD `BufferCopy { src_offset, dst_offset, size }`)
  for the staging upload + the test-only readback. Buffer-only (no images) — keeps the §5.5 buffer-barrier model
  intact.

## Key types (boyko_render; all graphics-aware here, NOT in boyko_ecs)
```rust
#[repr(C)] pub struct GpuColumnMeta { handle: DeviceColumnHandle, stride: u32, device_len: u32, device_cap: u32, archetype: ArchetypeId, component: ComponentId } // 32 B, cold
pub struct GpuColumnManager { registry: ResourceRegistry<Vulkan>, meta: Vec<Option<GpuColumnMeta>>, staging: Option<BufferHandle>, staging_cap: u64 } // a NonSendResource
#[repr(C)] pub struct PlannedBarrier { src_stage: BarrierStage, dst_stage: BarrierStage, buffer: DeviceColumnHandle, src_access: BarrierAccess, dst_access: BarrierAccess }
pub struct GpuSystem { pipeline: ComputePipelineHandle, target: DeviceColumnHandle, intent: GpuAccessIntent, barriers: Box<[PlannedBarrier]>, group_count_x: u32, meta: SystemMeta } // impl boyko_ecs::System
// boyko_rhi additive: #[repr(C)] BufferCopy { src_offset, dst_offset, size: u64 }; RhiCommandEncoder::copy_buffer(&mut self, src, dst, &[BufferCopy])
// boyko_ecs Phase-5 seam fill: pub(crate) fn set_device_handle(&mut self, h); pub(crate) fn device_handle(&self) -> Option<DeviceColumnHandle>;  (on PoolBacking::Device)
```

## Public API (boyko_render)
`GpuColumnManager::{new, create_column (the PoolBacking::Device grow-callback the Phase-4 plan reserved),
upload_initial (staging+copy+fence, setup), grow_column (#[inline(never)] realloc, returns NEW handle),
resolve (~3 ns frame-path), readback_for_test (device→staging+fence+map, TEST-ONLY), destroy_all}`.
`GpuSystem::new(pipeline, target, intent, barriers)` impl `boyko_ecs::System`.
`lower_barriers(edges, intents) -> Vec<(SystemIndex, Box<[PlannedBarrier]>)>` (schedule-build time, cold).

## Multithreading
All GPU work single-threaded on the dispatcher in the apply-window (`running==0`, `SystemKind::GpuCompute` →
EXC2 solo). RHI types are `NonSendResource` (`!Send`, reachable only via the `unsafe` NonSend accessor whose
contract holds only on the dispatcher — CR-A). One sync point: the per-GpuSystem `wait_fence` (deferred to the
next frame's dispatch for overlap). No locks/atomics added. Race-freedom: column bytes live in VRAM, never
CPU-aliased (DeviceLocalBlock never mapped; the only CPU touch is the test readback, which fences first); the
CPU-query skip (D7) guarantees no CPU↔GPU byte aliasing; RHI types are `!Send` (borrow checker forbids
cross-thread).

## Integration
New crate `crates/boyko_render/` (deps boyko_ecs + boyko_rhi + boyko_rhi_vulkan + boyko_utils): `gpu_column.rs`,
`gpu_system.rs`, `barrier.rs`, `error.rs`, `shaders/gpu_integrate.{hlsl,comp.spv}`, `tests/{zero_readback,
sync_validation}.rs`. Additive `boyko_rhi` (`BufferCopy`, `copy_buffer`, TRANSFER stage/access + usage enums).
`boyko_rhi_vulkan` (`DeviceLocalBlock`, `create_buffer(DeviceLocal)`, `copy_buffer` impl). Narrow `boyko_ecs`
seam fill (`set_device_handle`/`device_handle` on `PoolBacking::Device`; the D7 query skip + debug diagnostic).
Dependency direction clean (`boyko_render → boyko_rhi → boyko_ecs`; no cycle; purity grep on `boyko_ecs` stays
clean). Additive = no ABI break.

## Implementation waves (each compiles + keeps the validation oracle + 0%-gate green)
- **A (RHI extension, serial — oracle-first):** `boyko_rhi` `BufferCopy`/`copy_buffer`/TRANSFER enums (+ Mock
  impl); `boyko_rhi_vulkan` `DeviceLocalBlock` + `create_buffer(DeviceLocal)` + `copy_buffer` impl; a new
  staging→device→staging copy round-trip test (validation-clean) BEFORE anything depends on it. The 3 existing
  Vulkan tests stay green.
- **B (boyko_render skeleton + GpuColumnManager):** crate + `create_column`/`resolve`/`destroy_all`;
  `upload_initial`/`readback_for_test` (upload→readback bit-exact test).
- **C (demo shader + GpuSystem):** `gpu_integrate.hlsl` (per-element transform reusing the 0c/0d arithmetic so
  the CPU golden is `golden_chained`-shaped) → committed `.spv`; `GpuSystem` impl `System` (record+submit;
  `access()` empty; `is_gpu`; `apply` no-op).
- **D (barrier lowering + ECS seam fills):** `lower_barriers` + `PlannedBarrier` (pure-logic test on synthetic
  edges/intents); `boyko_ecs` `set_device_handle`/`device_handle` + the D7 query skip + debug diagnostic;
  0%-gate A/B benches flat; existing Miri suites green.
- **E (end-to-end proof):** `zero_readback.rs` (N-frame run, single test-only readback, bit-exact golden,
  validation `total()==0`, per-frame-readback counter == 0); `sync_validation.rs` (omit/wrong barrier → sync
  hazard → test fails; correct barrier → clean — proves the lowering is load-bearing).
- Parallelism: A serial; B after A; C-shader ∥ B; D after B+C; E after all.

## Validation
0%-gate benches ±2% + `cargo asm` byte-identical on `row_ptr`/`for_each_chunk`/the CPU query collect loop.
Unit: RHI `copy_buffer` (Mock); Vulkan DeviceLocal copy round-trip; manager create/resolve/destroy/upload/
readback/grow-stale-handle; `GpuSystem` access-empty + `SystemKind::GpuCompute` + apply-no-op; `lower_barriers`
directed/GPU-GPU/superset cases. Property: every GpuCompute consumer with a touched column gets ≥1 covering
barrier (never a missed barrier). Golden: `zero_readback` + `sync_validation`. The GPU half is NOT
Miri-checkable (§6) → validation-layer-fail-the-test + golden-buffer are the oracle. `debug_assert!`s: resolve
is Some in `run_unsafe`; `device_len <= device_cap`; full dispatch coverage; CPU-query-over-GPU-component loud;
per-frame-readback counter == 0; destroy_all-before-drop.

## Open questions (for the critic — the Phase-4 seam boundary is the attack surface)
1. **Device-handle WRITE path**: Phase 5 needs `pub(crate) set_device_handle` to write the new `u64` after a
   grow realloc. Confirm in-scope for Phase 5 + that it doesn't violate a Phase-4 write-once-`backing` invariant.
2. **`grow_rows` Device-arm `unreachable!` vs a callback**: I argue the realloc fill belongs in `boyko_render`
   (core can't touch RHI), core only exposing the handle setter. Confirm this division vs the Phase-4 plan's
   "Phase 5 fills realloc in the core grow path" wording.
3. **Per-frame encoder + fence ownership**: a reused encoder + `frame_fence` on the manager; single-in-flight
   submit, fence waited at the NEXT dispatch (zero-readback, overlap-friendly). Confirm this is the right
   reservable seam for the deferred frame-overlap (§1.11).
4. **CPU-query-over-GPU-component (D7 footgun)**: debug diagnostic vs release-present panic (the Phase-4
   residency-conflict reject is release-present). Lean: release-present (partition-integrity precedent).
5. **Initial upload provenance**: setup-stage CPU-provided `&[u8]` is the only CPU→GPU transfer, at setup, not
   on the frame path. Confirm acceptable for the foundation (real GPU-resident spawn is Phase 6 indirect).
6. **Barrier-plan staleness across grow**: resolve target/barrier handles INDIRECTLY by a stable
   `(archetype, component)` key each frame (one cold lookup) to survive a grow, vs "grow invalidates the plan;
   rebuild". Lean: indirect resolve (eliminates the staleness class).

---

# Post-Critic Binding Revisions (AUTHORITATIVE)

> Resolutions to the 3-lens critic panel (all REVISE, verified vs MERGED `ecs` code). Where these conflict with
> D1–D8 / the original waves, THESE WIN. Every item is grounded in merged-code file:line. Rewrite rule: replace
> "Phase 5 fills the reserved seam X" with "Phase 5 ADDS the wiring X on the Phase-4-reserved type".

## MF-1 — GpuCompute builder seam (boyko_ecs ADD). 
ADD `pub fn SystemConfig::gpu(self) -> Self` (sets `descriptor.is_gpu = true`; sibling of `before/after/chain/
in_set/run_if` at system_config.rs:66-183) → the build resolver at schedule_builder.rs:376 reads it. REJECTED
deriving `is_gpu` from `gpu_intent`/`requires_dispatcher` (the latter is also set by `NonSendResMut`, so every
NonSend CPU system would be mis-marked GpuCompute). 0%-gate: kind-resolution is build-time/cold (not the frame
path). Verified: is_gpu pub(crate)+false-only (system_descriptor.rs:70/84), only set in a unit test (:1513);
schedule.rs:986-989 admits "not creatable end-to-end in Phase 4".

## MF-2/MF-3 — device-pool mint + handle accessors (boyko_ecs ADD; the only flip today is `make_device_backed_for_test` `#[cfg(test)]`).
ADD: (a) `pub(crate) ComponentPool::make_device_backed(&mut self, handle)` — a `#[cfg(not(miri))]` (NOT test-
gated) sibling of the test fn, same `assert!(len==0)` + `backing = Device(Box::new(DeviceColumn::new(handle)))`;
(b) `pub(crate) ComponentPool::set_device_handle(&mut self, h)` + `device_handle(&self) -> Option<_>`;
(c) `pub(crate) DeviceColumn::set_handle(&mut self, h)` (only `handle()` getter exists, device_column.rs:85);
(d) the boyko_render driver calls them via a boyko_ecs funnel (MF-6). **MF-3:** `set_device_handle` mutates ONLY
the boxed `DeviceColumn.handle` — DISTINCT from the write-once `buffer`/`added_base`/`changed_base` (which DANGLE
after the Host `VmReservation` is dropped at the flip, component_pool.rs:1757-1766) → no base-pointer-invariant
violation; the device grow does NOT call `grow_rows`/`host_vm_mut` (the `unreachable!` arm stays unreachable,
component_pool.rs:99/117-119); add `debug_assert!(self.len==0)`. **Layout-neutral:** all four mutate existing
fields, add ZERO fields → the size pins (128 host / 144 miri, component_pool.rs:33-41) + SEND10 hold. Resolves
OQ1 (in-scope, layout-neutral) + OQ2 (the realloc lives in boyko_render; core only exposes the setter; the
`unreachable!` arm is PRESERVED, not replaced).

## MF-4 — the CPU-query GPU-skip is a SOUNDNESS PREREQUISITE in BOTH funnels, ordered FIRST (boyko_ecs ADD).
The skip does NOT exist; BOTH collection funnels match purely on `component_mask()`: `update_archetypes`
(query_state.rs:195-209) AND `seed_from_candidates` (query_state.rs:276-283). ADD `if arch.flags.is_gpu_resident()
{ continue; }` (archetype_flags.rs:140-141) to BOTH. This is a SOUNDNESS prereq (NOT a 0%-gate optimization): the
Device-arm Send/Sync proof (component_pool.rs:1818-1825) + the dangling-Host-base contract rest on "CPU Query
skips GPU archetypes so row_ptr on a device pool is never reached". → it MUST land in a new **Wave A2 (BEFORE
any device pool is minted in Wave B)**. Footgun policy = **release-present panic** (resolve OQ4) `cpu_query_over_
gpu_component_panic` (`#[cold]`, prefer detecting at QueryState-build that `include` names a `ResidencyKind::Gpu`
component, to keep the collection loop clean), matching the release-present residency reject
(archetype_bundle.rs:530-532). 0%-gate: `cargo asm` BOTH rebuild loops byte-identical when no GPU archetype.

## MF-5 — the GpuSystem mechanism (THE deep one; rewrites D5). 
The `NonSendResMut<RhiDevice>` route is BROKEN: (i) `RhiDevice<A>` is a generic TRAIT (device.rs:69), not a
sized type → type-incoherent as a SystemParam arg; (ii) `NonSendResMut::init_access` calls `mark_universal()`
(nonsend_resmut.rs:82) → forces `CpuExclusive`+universal, contradicting empty-access + `GpuCompute`. RESOLVED:
the **GpuSystem is a HAND-WRITTEN `impl boyko_ecs::System`** (NOT a FunctionSystem, NOT `NonSendResMut`): it
(1) declares EMPTY component access + `is_gpu=true` (via `.gpu()`) → `GpuCompute` → `runs_on_dispatcher()`
(system_kind.rs:93) → EXC2 solo at `running==0` (the FIX-3 live-running-set gate, schedule.rs:991-994); (2) is
ordered after its CPU producer by an explicit `.after(producer)` directed edge (system_config.rs:77) feeding
`ConflictGraph::successors` (NOT an access conflict — access is empty); (3) reaches the `!Send` RHI by storing a
CONCRETE `boyko_render` newtype `RhiContext` (wraps the Vulkan device+queue+`GpuColumnManager`) that `impl
NonSendResource` (orphan rule: the impl MUST live in `boyko_render`), projected from the world via
`UnsafeEcsCell::nonsend_resources_mut()` directly in `run_unsafe` (replicating nonsend_resmut.rs:99-110's
projection WITHOUT `mark_universal`). SOUND: `GpuCompute`→dispatcher-solo at `running==0` → hand-reaching the
`!Send` RhiContext on the dispatcher is the same single-thread-touch discipline as NonSend (ecs_master.rs:3536-
3543), without the param's universal-access side effect. A GpuCompute system with empty access IS dispatched
(not starved): EXC2 accepts it once the producer completes + `running==0`.

## MF-6 — boyko_render can't reach the barrier-lowering inputs (boyko_ecs ADD).
`SystemIndex`/`ConflictGraph`/`successors`/`Schedule.systems`/`conflict_graph` are ALL `pub(crate)`
(conflict_graph.rs:47/65/73/77, schedule.rs:96/100). ADD a PUBLIC accessor exposing PUBLIC POD (no internal-type
leak): `pub struct GpuBarrierEdge { producer: u32, consumer: u32, producer_intent: GpuAccessIntent,
consumer_intent: GpuAccessIntent }` + `impl Schedule { pub fn gpu_barrier_inputs(&self) -> impl
Iterator<Item=GpuBarrierEdge> + '_ }` (walks `successors` internally, yields edges whose consumer `is_gpu`,
u32 indices not `SystemIndex`). `lower_barriers` consumes this iterator; its output `PlannedBarrier` is keyed by
the stable `(ArchetypeId, ComponentId)` (per MF-7), NOT a `u64`/`SystemIndex`.

## MF-7 — grow handle-staleness (boyko_render + barrier-plan policy). 
`take` bumps the generation THEN frees the LIFO index (handle.rs:124-130); `register` pops that index LIFO with
the bumped generation (handle.rs:90-115). So `grow_column` MUST `take_buffer(old)` BEFORE `register_buffer(new)`
→ the reused index carries the bumped generation → a stale `u64` resolves `None` (loud). Register-then-take
would mint a fresh index + leave the stale `u64` resolving LIVE (orphan + aliasing). + commit to OQ6: resolve
`GpuSystem.target` + `PlannedBarrier.buffer` INDIRECTLY by the stable `(archetype, component)` key each frame
(one cold lookup → `GpuColumnMeta` → current handle); NEVER cache the raw `u64`. Change `target:
DeviceColumnHandle` → `target_key: (ArchetypeId, ComponentId)`; same for `PlannedBarrier`.

## MF-8 — D8 over-claimed the RHI additions (correct it). 
`MemoryLocation::DeviceLocal` (enums.rs:180) + ALL TRANSFER stage/access/usage enums (enums.rs:29/31/106/146/148)
ALREADY EXIST. The GENUINE additions: (a) `BufferCopy` POD + `RhiCommandEncoder::copy_buffer` as a `#[cold]
#[inline(never)]` DEFAULT-body trait method (mirrors `dispatch_indirect`, encoder.rs:52-57 → Mock + ABI
untouched); (b) `DeviceLocal` ROUTING in the Vulkan `create_buffer` (`buffer_mapped_ptr → None`); (c) the
`DeviceLocalBlock` — reuses `SubAllocator` only, NOT `HostVisibleBlock` (which persistently maps): new
device-local memory-type selection + non-mapping block + device buffer create/bind; (d) the D6 superset-widen =
OR of EXISTING bits (`COMPUTE_SHADER|TRANSFER` / `SHADER_READ|SHADER_WRITE|TRANSFER_READ|TRANSFER_WRITE`), NOT new
`ALL_COMMANDS`/`MEMORY_*` constants (they don't exist; Phase-1 D5 "only what the foundation uses", enums.rs:3).

## REVISED WAVE PLAN (soundness ordering — query-skip BEFORE device-mint)
- **A (RHI, serial, oracle-first):** `BufferCopy` + `copy_buffer` (`#[cold]` default-body); Vulkan
  `DeviceLocalBlock` + `DeviceLocal` routing + `copy_buffer` impl; staging→device→staging round-trip test
  (validation-clean). The 3 existing Vulkan tests stay green. (No enum work — DeviceLocal/TRANSFER exist.)
- **A2 (boyko_ecs seam ADDs — soundness-first, BEFORE any device pool):** `SystemConfig::gpu()` (MF-1);
  `ComponentPool::{make_device_backed, set_device_handle, device_handle}` + `DeviceColumn::set_handle` (MF-2/3,
  `debug_assert!(len==0)`); the DUAL query-skip + the release-panic reject (MF-4); `Schedule::gpu_barrier_inputs`
  + `GpuBarrierEdge` (MF-6). 0%-gate `cargo asm` on `row_ptr`/`for_each_chunk`/BOTH rebuild loops; existing Miri
  suites green. **Establishes the CPU-skip invariant the Device Send/Sync proof rests on.**
- **B (boyko_render skeleton + GpuColumnManager):** the `RhiContext: NonSendResource` newtype (MF-5);
  `create_column` (→ `make_device_backed`) / `resolve` / `destroy_all` / `upload_initial` / `readback_for_test`;
  `grow_column` with take-before-register (MF-7). Safe to mint device pools now (A2 skip live).
- **C (demo shader + GpuSystem):** `gpu_integrate.hlsl`→committed `.spv`; the hand-written `GpuSystem` `impl
  System` (empty access; `.gpu()`; `run_unsafe` projects `RhiContext` via `UnsafeEcsCell`, records+submits;
  resolves `target_key` indirectly per frame).
- **D (barrier lowering):** `lower_barriers(Schedule::gpu_barrier_inputs())` → `PlannedBarrier` keyed by
  `(archetype, component)`; pure-logic test; superset-widen = OR of existing bits.
- **E (end-to-end proof):** `zero_readback.rs` (now reachable end-to-end); `sync_validation.rs` (omit/wrong
  barrier → hazard → test fails).
- Parallelism: A serial; A2 after A; C-shader ∥ A2; B after A2 (soundness gate); C after B; D after B+C; E last.

## boyko_ecs PUBLIC seam ADDITIONS (Phase 5 touches core MORE than originally implied — warrants a mini-critic)
1. `SystemConfig::gpu()` (PUBLIC). 2. `ComponentPool::make_device_backed` (`pub(crate)`, `#[cfg(not(miri))]`).
3. `ComponentPool::set_device_handle`/`device_handle` (`pub(crate)`). 4. `DeviceColumn::set_handle`
(`pub(crate)`). 5. the DUAL query-skip + the release reject (touches the query hot-rebuild path). 6.
`Schedule::gpu_barrier_inputs()` + the public `GpuBarrierEdge` POD (PUBLIC). 7. `GpuAccessIntent` made
pub-reachable via `GpuBarrierEdge`. **→ A focused mini-critic on the A2 boyko_ecs surface is warranted (0%-gate
on both query funnels, the `gpu()` API vs the config-chain idiom, the `GpuBarrierEdge` POD not leaking internal
types) before A2 lands.**

---

# Mini-Critic C1/C2 Resolution (AUTHORITATIVE — supersedes MF-4 above; MF-1/2/3/5/6 confirmed sound)

> The A2 mini-critic (architect-recommended) found 2 CRITICAL soundness gaps in MF-4 + a focused
> re-resolution, all grounded in merged code. MF-1, MF-2/3 (+ addendum), MF-5, MF-6 confirmed SOUND. This is the
> FINAL authoritative form of MF-4.

## C2 — GPU-resident archetypes are GPU-PURE
The current reject (`saw_gpu && saw_cpu_pinned`) permits `Gpu + ordinary Cpu` mixing → a `Query<&CpuComp>` over
a mixed archetype would be silently dropped by the blanket skip (correctness regression). FIX: tighten to
**`saw_gpu && saw_non_gpu`** (a Gpu component alongside ANY non-Gpu — Cpu OR CpuPinned — is rejected) at ALL
THREE mint sites, identically: live `archetype_bundle.rs:520-532` (FIX-2), dead `archetype.rs:328-353`,
third-funnel `archetype_master.rs:529-541`. Aligns with umbrella §2 (whole-archetype device residency) + §5.1.
Widen the panic message to "a GPU-resident archetype must be GPU-pure: mixes Gpu and non-Gpu". `GPU_RESIDENT ⇔
all-components-Gpu` (was OR-of-any-Gpu — the leak). CpuPinned semantics subsumed.

## C1 — the direct-access funnel (`get_component_raw`) bypasses the query skip
`Column.ptr` caches `ComponentPool::buffer_ptr()` (archetype.rs:32-33,562-563); after `make_device_backed` frees
the Host `VmReservation`, that cached ptr DANGLES but is non-null (component_pool.rs:1757-1766) →
`get_component_raw`'s null-check (ecs_master.rs:1410) would PASS → UAF. FIX: the device-mint must NULL the
archetype column cache, and pool-level `make_device_backed` CANNOT (no handle to the archetype's `columns`). ADD
an **archetype-level funnel** `Archetype::make_component_device_backed(cid, handle)` (`pub(crate)`,
`#[cfg(not(miri))]`): flip the pool, then `self.columns[cid.0] = Column::null()` DIRECTLY — NOT `refresh_column`
(it re-caches the dangling base; note this at archetype.rs:550-553). Then every direct reader's existing
null-check returns `None`/`false`/skip — correct "CPU can't touch GPU bytes" (§2). **Enumerated direct readers
(all covered):** `get_component_raw{,_mut}` (ecs_master.rs:1410/1458), `get_component{,_mut}` (1515/1584),
`set_component_raw` (1485 via mut), `has_component` (1660), `get_components_raw{,_mut}` (1753/1795);
`query_entities` (1704) reads no `columns[].ptr` (exposes only entity handles — covered). **Invariant (PER-COMPONENT
debug_assert at the funnel tail — CORRECTED post-A2-review, finding C1-1):** `make_component_device_backed` is a
per-COMPONENT funnel (Wave B calls it once per Gpu column), so it asserts ONLY the postcondition it upholds at
that point: `debug_assert!(arch.columns[cid.0].is_null())` (the just-flipped column is null). The whole-archetype
property — every column of a GPU-resident archetype is null — holds by CONSTRUCTION once every component has been
flipped; it is NOT (and cannot be) checked at this per-component site. (The original spec's `!arch.flags
.is_gpu_resident() || arch.component_ids.iter().all(|c| arch.columns[c.0].is_null())` was a BUG: `GPU_RESIDENT` is
stamped at MINT over the whole signature, so on a multi-component GPU-pure archetype the first per-component flip
leaves siblings non-null → both disjuncts false → guaranteed debug/test panic on the intermediate state.
Intermediate states are sound: a not-yet-flipped component's Host pool stays valid until ITS OWN flip frees the
reservation, and queries skip the archetype via the mint-stamped `is_gpu_resident()` regardless of column state.) **SEND10 update
(component_pool.rs:1818-1825):** cite BOTH guards — (1) the Query-path skip AND (2) the direct-access
null-column → null-check — as the proof the dangling Host base is CPU-unreachable. **Semantic note
(correct-by-design, document):** `has_component(e, gpu_id)` → `false` (a device component reads as ABSENT from
the CPU surface — the intended §2 contract; a future CPU-visible "is-device-resident" predicate must consult the
signature mask, not the column cache).

## Query skip (the D7 fill, correct now that archetypes are GPU-pure)
ADD `if arch.flags.is_gpu_resident() { continue; }` to `update_archetypes` (query_state.rs:195-209) ONLY. In
`seed_from_candidates` (query_state.rs:276-283) ADD **`debug_assert!(!arch.flags.is_gpu_resident())`** instead of
a `continue` (W2): EnableTags are `StorageKind::Bitset`/always `Cpu` (archetype.rs:331-346) → a GPU-pure
archetype carries none → its id is structurally absent from any `EnablePresence[A]` candidate set; a silent
`continue` would mask a regression. (Reuse the `get_archetype(...)` Some-binding — zero extra lookup.)

## W1 — footgun: a CPU query NAMING a Gpu component (release-present, resolves OQ4)
At `QueryState::new` (query_state.rs:74): `if include.intersects(&GPU_COMPONENT_SET) {
cpu_query_over_gpu_component_panic(...) }` (`#[cold] #[inline(never)]`). `GPU_COMPONENT_SET` = a new static
`ComponentMask`-shaped bitset (bit per id where `residency_class==Gpu`), maintained write-once in
`set_residency_class` (component_registry.rs:578+). O(MAX_COMPONENTS/64) words, build-time/cold → the collection
loop stays byte-identical (0%-gate). Only `include` panics (exclude/optional harmless).

## O1 / O2
**O1:** `ComponentPool::make_device_backed` is `#[cfg(not(miri))]` (NOT `cfg(test)`; aligns with `DeviceColumn`)
with a **release `assert!(self.len == 0)`** (data-loss guard, mirrors the test fn at component_pool.rs:1769); the
CR-C Drop check + the C1 column-null post-condition stay `debug_assert!`. **O2:** doc on `GpuBarrierEdge` — the
`u32` indices are a TRANSIENT build-time `SystemIndex` projection valid only against the producing `Schedule`
(consumed same-pass by `lower_barriers`); NOT durable — the durable barrier key is `(ArchetypeId, ComponentId)`
(MF-7); never persist the `u32` past the build pass.

## Wave-A2 scope additions (beyond the MF-4 query-skip already listed)
1. The tightened `saw_gpu && saw_non_gpu` reject at all 3 mint sites + widened panic message.
2. `Archetype::make_component_device_backed` (archetype-level device-mint that nulls `columns[cid]`) + the
   GPU-pure-all-columns-null `debug_assert` + the SEND10 update. **Load-bearing C1 fix — MUST land in A2 BEFORE
   Wave B mints any device pool.**
3. The W1 build-time footgun: `GPU_COMPONENT_SET` static + `set_residency_class` maintenance + the
   `QueryState::new` intersect check + `cpu_query_over_gpu_component_panic`.
   Plus zero-code: the SEND10 comment update, the W2 `debug_assert` swap, the O1 `assert!` wording, the O2 doc.
All stay inside `boyko_ecs` (no graphics types — purity grep holds).

## MF-5 amendment (Phase 5 Option C — supersedes the raw cell projection)

The original MF-5 mechanism reached the `!Send` `RhiContext` from `GpuSystem::run_unsafe`
through a PUBLIC `UnsafeEcsCell::nonsend_resource_mut` (the `NonSendResMut::get_param`
projection minus `mark_universal`). The Wave-C review found that accessor opened 3 real UB
paths: **C1** — it was reachable on the concurrent WORKER path (any system holding a cell
copy could project the `!Send` payload off-dispatcher); **M1** — its `'w` return lifetime let
two back-to-back calls hand out two live `&mut R` aliases; **M2 (latent)** — no tripwire caught
a wrong-thread touch.

**Option C** replaces the raw cell projection with a dispatcher-only capability,
`DispatcherToken<'w>` (`crates/boyko_ecs/src/ecs/core/system/dispatcher_token.rs`):

- The public `UnsafeEcsCell::nonsend_resource_mut` is **DELETED** (C1/M1 kill — the
  worker-reachable surface is gone; a `compile_fail` test proves it).
- `System` gains two DEFAULT-bodied methods: `is_gpu()` (defense-in-depth GPU marker, the
  builder ORs it with `SystemConfig::gpu()`) and `unsafe fn run_dispatcher(token)` (the
  default forwards to `run_unsafe` via `token.into_cell()`, so every CPU system is
  byte-identical — the 0%-gate).
- The scheduler's dispatcher-solo path (and `EcsMaster::run_system_once`) mints a
  `DispatcherToken` at `running == 0` and calls `run_dispatcher`; the CPU-concurrent WORKER
  path is untouched (still `run_unsafe(cell_copy)`).
- `GpuSystem` overrides `run_dispatcher` to project `token.nonsend_resource_mut::<RhiContext>()`
  (return lifetime tied to `&mut self` — the M1 fix), and its `run_unsafe` becomes a loud
  debug-panic no-op (it has no token, so `RhiContext` is structurally unreachable on a worker).
- `DispatcherToken` is NOT `Copy`/`Clone` (the borrowck M1 fix depends on it) and names no
  graphics type (`boyko_ecs` stays graphics-pure).
- A debug-only `NonSendResources::owning_thread` stamp (M2) tripwires any projection off the
  slab's owning thread, on both the `DispatcherToken` and the existing `NonSendResMut` paths.
