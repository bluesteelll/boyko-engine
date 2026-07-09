# R0 — Instrument the software ray baseline (HW-RT track, rung R0)

Converged build spec (architect wf + researcher wf, 2026-07-04). Rung R0 of the HW-RT
roadmap in [RENDER-HYBRID-RAY-SYSTEM-DESIGN.md](RENDER-HYBRID-RAY-SYSTEM-DESIGN.md) §8
("do first, unconditionally"). **Measurement only — the default rendered command stream
stays byte-identical.** This is the calibration-timestamp foundation for §4.

## Goal

Add a GPU timestamp-query primitive to the in-house raw-FFI Vulkan RHI, then bracket the
four software-ray passes (DDGI probe update, deferred resolve incl. inline SDF shadow
march, CSM cascade depth, punctual atlas depth) so an offline `#[ignore]` harness prints
per-pass GPU wall-clock (ns/pass) + derived ns/ray on the real showcase scene. The
existing `ddgi_probe_gi_cost.rs` measures via CPU wall-clock around a fenced, dispatch-only
isolated submit because no `vkCmdWriteTimestamp` subsystem exists; R0 removes that
limitation (timestamps bracket passes inside the real combined frame).

## Correctness rules (from the timestamp-query research — the implementation checklist)

1. **Reset before every write.** A TIMESTAMP query pool is UNDEFINED at creation; each
   query MUST be reset (`vkCmdResetQueryPool`) before a `vkCmdWriteTimestamp` targets it.
   Skipping the reset = *undefined* results (not stale). `vkCmdResetQueryPool` MUST be
   recorded OUTSIDE any render/dynamic-rendering scope (`VUID-vkCmdResetQueryPool-renderpass`).
   Compute-only prologue = trivially legal.
2. **Mask to `timestampValidBits` BEFORE subtracting**, then × `timestampPeriod`. High bits
   above the valid width are hardware garbage. Guard `1u64 << 64` (UB): `bits>=64 => u64::MAX`.
   `ns = ((t_end & mask) - (t_begin & mask)) & mask) as f64 * timestamp_period as f64`.
3. **`vkGetQueryPoolResults` + `VK_QUERY_RESULT_64_BIT | VK_QUERY_RESULT_WAIT_BIT`** after the
   existing `wait_fence` is the minimal correct readback (offline harness already fences).
   64-bit is mandatory (a 32-bit ~1ns counter overflows in ~0.43 s). NOT the pipelined
   `vkCmdCopyQueryPoolResults` path (unneeded for a synchronous fenced harness).
4. **Graceful-skip** on `timestampPeriod == 0` OR the queue family's `timestampValidBits == 0`
   OR `timestampComputeAndGraphics == false` (must read the specific family) → fall back to
   the CPU wall-clock path; never crash.
5. **TOP_OF_PIPE @ open / BOTTOM_OF_PIPE @ close** is the profiler-standard bracket. It
   over-approximates (front-pad + neighbour overlap), but the harness's fenced,
   single-frame-per-iteration, isolated structure removes neighbour contamination (what
   Nsight does with WaitForIdle). Intermediate stages are meaningless — use only TOP/BOTTOM.
6. **Methodology:** discard cold warm-up frames (shader compile + GPU clock ramp), report
   MEDIAN + p95 (GPU Boost makes absolute ns one-sided-noisy). No hard threshold assertion —
   emit stats, let the orchestrator derive cadence with margin. (Mirrors `ddgi_probe_gi_cost`:
   220 iters / discard 20 / median-p95-stddev.)

---

## Part A — the RHI timestamp-query primitive

### A.1 `boyko_rhi` vocabulary
- **`enums.rs`** — `TimestampStage` (`#[repr(i32)]`): `TopOfPipe = 0x1`, `BottomOfPipe = 0x2000`
  (single `VkPipelineStageFlagBits`; do NOT reuse the `BarrierStage` bitmask).
- **`descriptor.rs`** — `QueryPoolDesc { pub count: u32 }` (`#[repr(C)]` POD; maps to
  `VkQueryPoolCreateInfo` with `queryType = TIMESTAMP`, `queryCount = count`, `pipelineStatistics = 0`).
- **`api.rs`** — add `type QueryPool;` to `RhiApi` (owned resource, next to `Fence`).

### A.2 `boyko_rhi` trait methods (all with `#[cold] #[inline(never)]` default bodies so Mock + ABI untouched — the `image_barrier` seam pattern)
- **`device.rs` (`RhiDevice`):**
  - `create_query_pool(&self, desc) -> Result<A::QueryPool, Error>` (default `Err(unsupported)`) — mirrors `create_fence`. Doc: queries UNDEFINED at creation; caller MUST reset before first write.
  - `unsafe destroy_query_pool(&self, pool)` (default `drop(pool)`) — mirrors `destroy_fence`; safety: no pending submit, destroyed once (move).
  - `read_query_pool_ns(&self, pool, pair_count, scratch: &mut [u64], out_ns: &mut [f64]) -> Result<(), Error>` (default `Err(unsupported)`). Host-waits + reads `2*pair_count` raw u64s, masks to valid bits, returns ns of each consecutive (begin,end) pair: `out[i]` = ns between query `2*i` and `2*i+1`. Uses `64_BIT | WAIT_BIT`.
- **`encoder.rs` (`RhiCommandEncoder`)** (no-op default bodies):
  - `reset_query_pool(&mut self, pool, first, count)` — `vkCmdResetQueryPool`; MUST be outside a render pass, before the frame's first write.
  - `write_timestamp(&mut self, pool, stage: TimestampStage, index)` — `vkCmdWriteTimestamp`; `index` must have been reset this frame.
- **`lib.rs`** — re-export `QueryPoolDesc`, `TimestampStage` if the crate re-exports its vocabulary.

### A.3 `boyko_rhi_vulkan` FFI (`ffi.rs`)
- `VkQueryPool(pub u64)` (`#[repr(transparent)]`, `NULL = Self(0)`); `VkQueryType { Timestamp = 2 }`;
  `VkQueryPoolCreateInfo { s_type, p_next, flags, query_type: i32, query_count: u32, pipeline_statistics: VkFlags }`;
  `VK_QUERY_RESULT_64_BIT = 0x1`, `VK_QUERY_RESULT_WAIT_BIT = 0x2`;
  `VkStructureType::QueryPoolCreateInfo == 11`.
- PFN typedefs (mirror `PfnVkCreateFence`/`PfnVkCmdDispatch`): `PfnVkCreateQueryPool`,
  `PfnVkDestroyQueryPool`, `PfnVkCmdResetQueryPool`,
  `PfnVkCmdWriteTimestamp(cmd, pipeline_stage: VkFlags, pool, query: u32)`, `PfnVkGetQueryPoolResults`.
- **`timestampPeriod` offset:** the existing `VkPhysicalDeviceLimitsBlob([u8;504])` covers it.
  `LIMITS_OFF_TIMESTAMP_PERIOD = 424` (const-assert `424+4 <= 504`; re-derive from the in-repo
  anchor `maxPerStageDescriptorStorageImages == 84`: `timestampComputeAndGraphics` (VkBool32) @420,
  `timestampPeriod` (float) @424). Add `VkPhysicalDeviceLimitsBlob::read_f32(offset) -> f32`
  (companion of `read_u32`). **Runtime guard:** a period `<= 0` or `> 1000` ns/tick ⇒ treat as
  unusable (a wrong offset degrades to graceful-skip, never to fake timings).

### A.4 `DeviceFns` (`device.rs`)
Add 5 fields (Vulkan 1.0 core, always present, next to `cmd_clear_color_image`):
`create_query_pool`, `destroy_query_pool`, `cmd_reset_query_pool`, `cmd_write_timestamp`,
`get_query_pool_results` — each `load_device_command(gdpa, device, c"vk...")?` (core ⇒ `?` safe).

### A.5 `DeviceCaps` (`device.rs`) — RECORDED, not fail-fast
- `timestamp_period: f32`, `timestamp_valid_bits: u32`.
- `timestamps_usable() -> bool` = `valid_bits > 0 && period > 0.0 && period < 1000.0`.
- `timestamp_mask() -> u64` = `if valid_bits >= 64 { u64::MAX } else { (1u64 << valid_bits) - 1 }`.
- Boot: `find_queue_family` currently reads `fam.timestamp_valid_bits` and DISCARDS it
  (`device.rs:2114/:2132`) — change it to also return the chosen family's valid bits; populate
  `timestamp_period = limits.read_f32(LIMITS_OFF_TIMESTAMP_PERIOD)` + `timestamp_valid_bits` at
  the `DeviceCaps` build site.

### A.6 `rhi_impl.rs`
- `VulkanQueryPool { pool: VkQueryPool, count: u32 }`; `type QueryPool = VulkanQueryPool`.
- `create_query_pool` (mirror `create_fence` shape); `unsafe destroy_query_pool`.
- `read_query_pool_ns`: call `get_query_pool_results(device, pool, 0, 2*pair_count, (2*pair_count)*8,
  scratch, stride=8, 64_BIT|WAIT_BIT)`; on success, per pair `out_ns[i] = ((scratch[2i]&mask) →
  wrapping_sub → &mask) as f64 * period`. debug_asserts: `2*pair_count <= pool.count`, scratch/out lens.
- Encoder `reset_query_pool` / `write_timestamp` — reach the command buffer + fn-table exactly as
  `dispatch`/`pipeline_barrier` do (mirror the accessor names verbatim — `self.fns()`/`self.command_buffer()` etc.).

### A.7 lifecycle (per measured frame, single graphics+compute queue)
`reset_query_pool(pool, 0, 2*PASS_COUNT)` at frame top (outside rendering) → per pass
`write_timestamp(TopOfPipe, 2*slot)` before its first cmd, `write_timestamp(BottomOfPipe, 2*slot+1)`
after its last → submit(fence) → `wait_fence` → `read_query_pool_ns`. **Per-FIF ringing:** the
collector holds `[VulkanQueryPool; FRAMES_IN_FLIGHT]` indexed by the renderer's `fi` (the offline
harness fences each frame so one pool would suffice, but ring for future-pipelining safety at no cost).

---

## Part B — instrument the passes (gated, byte-identical default)

### B.1 gate = `Option<&TimestampCollector>` on the scene struct (the `ddgi_update: Option<...>` precedent)
Add `gpu_timing: Option<&'a TimestampCollector>` to the `GBufferScene<'a>`-style struct. `None` on
EVERY golden/host frame ⇒ NO reset/write commands ⇒ byte-identical stream. **Runtime `Option`, NOT a
cargo feature** (a feature would risk the timed build diverging from the shipped pipeline the
calibration must measure; the `is_some()` branch is cold + perfectly predicted).

**`TimestampCollector`** (new `present/gpu_timing.rs`): `pools: [VulkanQueryPool; FRAMES_IN_FLIGHT]`;
`TimedPass` (`#[repr(u32)]`): `DdgiUpdate=0, DeferredResolve=1, CsmDepth=2, PunctualDepth=3`;
`PASS_COUNT = 4`. Exposes `pool(fi)`, `write_begin/write_end(enc, fi, TimedPass)`. Accumulation
(median/outlier) is HOST-side in the harness after readback, so the collector is `&`-shared
read-only during recording (writing timestamps mutates GPU memory, not the Rust struct).

### B.2 the four bracket sites (`present/passes/gbuffer.rs`), each `Some`-gated
| Slot | Pass | Site | Bracket |
|---|---|---|---|
| DdgiUpdate | probe-update dispatch | `:789`/`:833` | begin before `record_graph_pass`, end after `cmd_dispatch` |
| DeferredResolve | resolve dispatch (SDF shadow march INLINE) | resolve `record_graph_pass` | begin before resolve input barriers, end after resolve dispatch |
| CsmDepth | CSM cascade depth | ~`:1072` | begin before CSM `begin_rendering`, end after `end_rendering` |
| PunctualDepth | spot+point atlas depth | ~`:1291` | begin/end around the atlas depth scope |

**Attribution honesty (state in the report):** `DeferredResolve` = the whole resolve dispatch,
INCLUDING the inline SDF soft-shadow march — timestamps bracket passes, not shader sections. R0 does
NOT isolate the shadow march.

### B.3 byte-identity proof
`gpu_timing == None` emits ZERO commands ⇒ stream identical to today. Proven by the existing
framegraph auto-barrier byte-identity golden (grep `byte-identical`/`golden`/`framegraph` in
`crates/boyko_rhi_vulkan/tests/`) + `engine_grand_showcase_512_ddgi_screenshot_dump` (pixels), both
run without the collector. **Add `gpu_timing: None` to every named-field scene construction site**
(mechanical, byte-identity-preserving).

---

## Part C — the report harness

**New `#[ignore]` test `crates/boyko_rhi_vulkan/tests/software_ray_baseline_cost.rs`** (name avoids
"update"/"time"/"setup" os-error-740 substrings). Reuses `run_showcase_body_ddgi` (GI ON — exercises
all four passes) via a timing variant:
1. Boot offscreen; if `!caps.timestamps_usable()` → print skip line + `return` (graceful, no panic).
2. Create `TimestampCollector` (`create_query_pool` × FRAMES_IN_FLIGHT, `2*PASS_COUNT` queries each).
3. Frame loop with `scene.gpu_timing = Some(&collector)`: reset top → passes bracket → submit →
   `wait_fence` → `read_query_pool_ns` → push `[f64; PASS_COUNT]` sample.
4. `>= 200` measured frames, discard first 20; report **median + p95 + stddev** per pass. NO
   empty-submit subtraction (GPU timestamps bracket on-device — a strict improvement over the CPU precedent).
5. Teardown: `wait_idle` → `destroy_query_pool` × FRAMES_IN_FLIGHT → normal showcase teardown.

**ns/ray attribution:** DdgiUpdate = `DDGI_PROBE_COUNT * DDGI_UPDATE_RAYS = 2048*64 = 131072` rays →
`ns/ray`. DeferredResolve = shaded-pixel count (from the resolve dispatch group-count × numthreads) →
`ns/px` (shadow march inclusive). CsmDepth/PunctualDepth = `ns/pass` only (no clean ray count → `n/a`).

Run: `cargo test -p boyko_rhi_vulkan --test software_ray_baseline_cost -- --ignored --nocapture
--test-threads=1` with `BOYKO_DISABLE_VALIDATION=1` (orchestrator runs it on the RTX box).

---

## RISKS
1. Pool not reset before write → UB/garbage. Mit: collector resets `(0, 2*PASS_COUNT)` at frame top, outside rendering.
2. `timestampValidBits` masking → garbage on `bits<64` GPUs. Mit: `timestamp_mask()` ANDs both endpoints + the `>=64 => u64::MAX` guard.
3. `timestampPeriod==0` / `validBits==0` / `computeAndGraphics==false` → Mit: `timestamps_usable()` gate → skip line + return.
4. GPU async/overlap noise → Mit: median-of-N + warm-up over ~180 kept frames.
5. Readback fence timing → Mit: `wait_fence` before read + `WAIT_BIT`.
6. TOP/BOTTOM honest semantics: measures pass wall-clock inclusive of pipeline-overlap, not isolated kernel time. Stated in report header.
7. New scene field breaks struct-literal sites → Mit: add `gpu_timing: None` everywhere (mechanical).
8. Wrong `timestampPeriod` offset → Mit: compile-assert + runtime plausibility guard (degrades to skip).

## Files (dependency order)
1 `boyko_rhi/src/enums.rs` (TimestampStage) · 2 `descriptor.rs` (QueryPoolDesc) · 3 `api.rs` (type QueryPool) ·
4 `device.rs` (3 RhiDevice methods) · 5 `encoder.rs` (2 verbs) · 6 `lib.rs` (re-exports) ·
7 `boyko_rhi_vulkan/src/ffi.rs` (handles/enum/struct/consts/5 PFN/offset+read_f32) ·
8 `device.rs` (5 DeviceFns + loads; DeviceCaps 2 fields + helpers; find_queue_family valid-bits; boot populate) ·
9 `rhi_impl.rs` (VulkanQueryPool + create/destroy/read + encoder verbs) ·
10 `present/gpu_timing.rs` (new: TimestampCollector, TimedPass, PASS_COUNT) ·
11 scene struct (+ `gpu_timing` field + `None` at all construction sites) ·
12 `present/passes/gbuffer.rs` (4 Some-gated brackets) ·
13 `tests/window_present_gbuffer.rs` (timing entry point) ·
14 `tests/software_ray_baseline_cost.rs` (new #[ignore] harness).

**Gates:** existing golden suite (framegraph byte-identity + grand_showcase dumps) stays green with
`gpu_timing: None`; `cargo clippy --all-targets -- -D warnings`; then the orchestrator runs the new
`#[ignore]` harness on the RTX box.

## Open notes (verify at implement time)
Exact encoder accessor names (mirror `dispatch`/`pipeline_barrier`); exact scene-struct name/file +
all construction sites (grep `ddgi_update: Option` in `present/`); exact framegraph byte-identity test
name; `VkStructureType::QueryPoolCreateInfo == 11`; resolve shaded-pixel count source.
