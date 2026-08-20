# Architecture: GPU Particle System (`boyko_render` + `boyko_rhi_vulkan`) — **Rev 4**

> **Rev 4 delta** (three surgical closures of the final verify's K-findings; everything else is Rev 3 verbatim):
> **K1** — the drift bound modeled the re-quantization noise while the DOMINANT term was the snorm16-stored
> multiplier's systematic magnitude error δ ~ 1–3×10⁻⁵ compounding as (1+δ)ⁿ (8–17× the claimed bound,
> coherent across an effect's particles). Fixed at the SOURCE: `EffectParamsGpu` stores the rotation
> multiplier as an **f32 pair** (the struct had spare padding), so |δ| ≤ 1 f32 ULP ≈ 6×10⁻⁸ ⇒ (1+δ)⁶⁴⁰ − 1
> ≈ 4×10⁻⁵ — below the per-particle record's own snorm16 precision, and the √n re-quantization bound
> stands exactly as written. Zero GPU cost, zero eDSL change.
> **K2** — seed row 8: the sim's returning `InterlockedAdd` on `instance_count` is a READ-modify-write;
> the access column now says `C/RW` (the tree's own atomic-counter precedent: `light_index_alloc`,
> `graph_bridge.rs:3837-3844`), so the derived kickoff→sim barrier's destination scope covers the read half.
> **K3** — rung P2b's arm is a NAMED compile-time variant (`-D PARTICLE_INTERP` + the host `cfg` sizing the
> record 40 B), with its own SHADER-VARIANT-MANIFEST row — never a runtime flag over an always-40 B record
> (that would be the F24 dark tax: +25 % draw-read paid while off).

> **Status:** Rev 4 — APPROVED by the final verify pass. Rev 3 closed `architecture-critic`'s final-verify findings on Rev 2 (4 P1, 3 P2). No P0 survives; all 9 N-findings and all 12 Rev-0 findings remain closed. Endorsed items — the industry skeleton, the buffer split, the per-path compare op, the role-keyed seeds, the kickoff pre-decrement, the four-boundary algebra including frame 0 — are carried **unchanged**. `graphify` CLI is not installed on this machine; orientation was Grep/Read.

---

## Rev 3 changelog

The critic's diagnosis: M1–M7 are the same residual pattern already fixed for the bookkeeping — **a value living in two places**. Rev 3 is surgical. One of the seven (M4) turned out to have a cause outside the subsystem, and fixing it properly deleted three other things.

**M1 — `wave_max(draw_args.additive.instanceCount, n + k)`: `k` undefined, `n` overloaded.** *Closed, and the mirror is gone.* The critic verified the intended wave-level mechanism is correct (contiguous reservations ⇒ `max(base+count) == Σcount == alive_count_next`, commutative under retirement order). Rev 3 moves that mechanism into §Counter and list ownership with **one symbol per value** — `w_count`, `w_lane`, `w_base`, `idx` — and derives D3's pseudocode from it. Working through M2 then showed the `InterlockedMax` mirror was unnecessary: the render counter and the list counter are *different quantities*, so the render counter can be an `InterlockedAdd` that yields both the render index and the final count. The `InterlockedMax` is deleted; the atomic count is unchanged.

**M2 — the P2 alpha survivor's slot index was unspecified, and the obvious reading leaks every alpha particle.** *Closed.* If alpha wrote `p_alive_write[CAP-1-m]`, kickoff — which reads only `alive_count_next` — would never see them: a total leak, at a rung whose struct P0 already pins. Resolution, as the coordinator directed: **the list index and the render index are separate concerns with separate counters.**
- `alive_count_next` (in `p_counters`) is the **list** counter, shared by both blend classes. Every survivor of either class takes `idx` from it and writes `p_alive_write[idx]`. No class can leak.
- `additive.instanceCount` and `alpha.instanceCount` (in `p_draw_args`) are the two **render** counters. Each `InterlockedAdd` yields both the class-dense render position and, at the end, the class's `instanceCount`. Additive maps position → `p_render[pos]`; alpha maps position → `p_render[CAP-1-pos]`.

Budget, corrected and stated per wave rather than per particle: **1** atomic for an all-dying wave, **2** for an all-surviving single-class wave, **3** for a wave that both survives and dies, or that mixes blend classes (P2 only). P0 is additive-only, so the mixed-class arm is unreachable at P0. The alpha row is added to §Counter and list ownership.

**M3 — the substep ceiling is reachable.** *Closed.* Verified: the in-tree bound is `⌈(max_delta × speed + timestep) / timestep⌉` (`app.rs:449-450`) and `relative_speed` is public and validated only `finite && >= 0` (`time.rs:134-138`), so speed 8.0 at stock defaults gives **129 > 64**. Rev 2 had the host advancing unclamped while the shader clamped — the same two-numbers defect N6 was supposed to have killed. Fix per the one-number rule: **the clamp happens once, on the host**; A1 computes `steps = min(raw_steps, PARTICLE_SUBSTEP_CEILING)`, uses that value for its own advance, and pushes that value to the shader. The shader's own `min` survives only as the F25 hang guard against a corrupt push constant. The clamp **drops** the excess time rather than carrying it (carrying would build a backlog that never drains under sustained slow-motion — the same reason `Time::max_delta` drops rather than carries), and the shortfall is counted in `dropped_steps`.

**M4 — the heartbeat changes event-buffering behaviour for the whole process.** *Closed by removing the dependency, not by patching around it.* Verified: `fixed_builder` is created lazily on the first `*_in(CoreSchedule::Fixed, …)` call (`app.rs:142-145`), and `event_policy_cfg: None` auto-resolves at `finish` to "`WaitForFixed` **iff a Fixed schedule was configured**" (`app.rs:157-159`), with `fixed_steps_since_swap` holding the swap across 0-substep frames (`app.rs:164-167`). So installing a *rendering* plugin would have flipped every event type in the process from `EveryFrame` to `WaitForFixed` — at 200 fps / 64 Hz, two frames in three, silently changing input, UI and collision event delivery. Patching the policy back is worse than the disease (it would then override the resolution a user's *own* later Fixed schedule should have produced, and it makes plugin order load-bearing).

**Rev 3 gives the subsystem its own clock.** `ParticleClock` (a `Resource`) owns `timestep`, `accumulator`, `steps` and `dropped_steps`, and is advanced by `Time::delta_secs()` — which is already `min(raw, max_delta)`-clamped, speed-scaled and pause-aware (`time.rs:39`, `:165`). `ParticlePlugin` **never touches `CoreSchedule::Fixed`, never creates `fixed_builder`, and never writes `event_policy_cfg`.** Everything D6 actually needed — a *constant* `dt` so `damping` and the rotation multiplier are host-precomputable, plus bounded tunneling — is preserved exactly. Three things are deleted along with the dependency: `particle_fixed_heartbeat`, the F28 no-Fixed-schedule freeze hazard, and Rev 2's Open Question 7. A gate is added that asserts installing `ParticlePlugin` leaves `App::finish`'s resolved `EventUpdatePolicy` byte-identical to the no-plugin resolution. Two bonuses fall out: pausing the game pauses particles and slow-motion slows them, both free; and a project may run particles at a different rate from gameplay physics.

**M5 — rows 4/5 mix the two physical buffers.** *Closed.* The seed table gains an explicit **"physical buffer this row tracks"** column, and row 4's "first access next frame" is corrected: it is **emit's `C/W` ⇒ WAW**, not the sim's read. A note is added on why the sim's read of the *un-overwritten prefix* is nonetheless visible (the availability chain composes: the seed barrier at emit carries `srcAccess = SHADER_WRITE @ COMPUTE`, whose first synchronisation scope includes the sibling frame's write, and the intra-frame emit→sim barrier then makes COMPUTE writes visible to COMPUTE reads). Gate #3's implementation now names, per row, the pass at which the barrier is expected.

**M6 — `steps == 0` is the common case above the step rate.** *Closed by stating it and naming the rung.* At 200 fps with a 64 Hz particle timestep, two frames in three step zero times and particles visibly move at 64 Hz against a 200 Hz camera. **P0 accepts fixed-rate stepping without render-time interpolation**, stated as a limitation rather than discovered as a bug. The follow-up is named as rung **P2b — render-time interpolation**, riding the engine's existing interpolation seam (`FixedTime::overstep_fraction()` / the RDG Pillar-B work), for which `ParticleClock::overstep_fraction()` is the subsystem's own equivalent. Particles are the easy case: `pos + vel · (overstep · timestep)` is one fused multiply-add in the VS. Cost is stated: a packed `vel` lane grows the render record 32 B → 40 B, **+25 % of the draw's read traffic**, so the rung defaults OFF.

**M7 — the rotation renormalization needs `rsqrt` or a divide, neither claimed on `Cf`/`FieldScalar`.** *Closed by showing reachability and then removing the need.* `FieldScalar` **does** have `div` (`scalar.rs:77`) and `sqrt` (`scalar.rs:101`), so `1/√(c²+s²)` is reachable today with no E-rung item — Rev 2's "needs `rsqrt`" was loose wording. But it should not be used: putting a division inside a leaf would drag `OpFDiv`'s 2.5 ULP into that leaf's oracle, and the house rule is that **division is never part of a bit-exact contract**. Rev 3 drops the renormalization and bounds the drift; **Rev 4 (K1) corrects the bound's dominant term**: the multiplier is stored as an **f32 pair** in `EffectParamsGpu` (not snorm16 — a quantized multiplier's magnitude error δ ~ 1–3×10⁻⁵ is a PER-EFFECT CONSTANT and compounds geometrically, (1+δ)⁶⁴⁰ ≈ ±1 %, coherent across the effect). At f32, |δ| ≤ 1 ULP ≈ 6×10⁻⁸ ⇒ (1+δ)⁶⁴⁰ − 1 ≈ 4×10⁻⁵. The remaining error is the per-step snorm16 re-quantization of the STATE, which is unbiased (round-to-nearest; the convex-term bias ≈ q²/4 per step is ~1.4×10⁻⁷ over 640 steps) and random-walks: ≈ 3×10⁻⁵·√640 ≈ **7.6×10⁻⁴** — a 0.08 % billboard-size error, below the record's own storage precision. The leaf stays pure mul/add and remains bit-exact. Rung E is unchanged (E1, E2, E3).

**Endorsed and untouched:** the industry skeleton (D3), the `p_dispatch_args`/`p_draw_args` split (D4), the per-path compare op (D7), the role-keyed seeds (both parities re-verified by the critic), the kickoff pre-decrement, the four-boundary partition including frame 0, and all 21 prior closures.

---

## Goal

A GPU-resident particle system where per-particle state never touches the CPU and never crosses PCIe: spawn counts and per-emitter transforms go up (≤16 KB/frame, **0 B on a frame with no spawns**); everything else lives and dies in VRAM.

**P0 functional target:** N emitters as ECS entities → one fixed-capacity GPU pool → one indirect instanced draw of additive billboards composited into `lit`, **on all four render paths** (Deferred with `LESS` / custom-linear depth; Forward, ForwardPlus, VisibilityBuffer with `GREATER` / reverse-Z), default-off with structural absence, and **with zero observable effect on any other subsystem when installed** (M4).

| Metric | 100k live | 1M live |
|---|---|---|
| VRAM at that CAP (92 B/particle) | 9.2 MB | 92 MB |
| Host→device per frame | ≤ 16 KB; **0 B when `total_spawn == 0`** | same |
| GPU kickoff+emit+sim | **≤ 80 µs** | ≤ 700 µs |
| GPU draw (fill-dependent) | **≤ 250 µs** | ≤ 2.5 ms |
| CPU per frame (≤256 emitters) | ≤ 15 µs | ≤ 15 µs |
| CPU allocations per frame | **0** | **0** |
| Draw calls | **1** at P0 (all effects/textures — bindless), 2 at P2 | same |
| Global atomics: emit | **0** | **0** |
| Global atomics: sim | **1–3 per wave** (see D5) | same |
| Readback | **none, ever** | **none, ever** |
| Effect on other subsystems' schedules/event policy | **none** (M4) | **none** |
| `goldens/PINS.toml` (35 pins) when `mode == Off` | **byte-identical by construction** | same |

Calibration (`PARTICLES-RESEARCH.md` §Scale): Team Nutshell 99 720 particles = 0.05 ms sim + 0.17 ms draw, RTX 4070, 64 B/particle; Brian-Jiang >1M @60 fps, GTX 1080.

---

## Context and constraints

### Verified in-tree facts

| # | Fact | Anchor |
|---|---|---|
| F1 | `lit[fi]` is created `STORAGE \| SAMPLED \| TRANSFER_SRC \| **COLOR_ATTACHMENT**` on every path — "purely PERMISSIVE for Deferred … an unexercised capability, byte-identical output" | `present/targets.rs:2855-2864` |
| F2 | `lit` is `R8G8B8A8_UNORM`, **already 8-bit post-tonemap** | `targets.rs:478~`, `:123-127` |
| F3 | `depth[fi]` is `D32_SFLOAT`, `DEPTH_STENCIL_ATTACHMENT \| SAMPLED`, per-FIF ring | `targets.rs:102-108` |
| F3b | `build_graphics_pipeline(desc, set1, depth_compare, depth_write)` is **already parameterised**; `depth_test_enable` hardcoded `VK_TRUE` | `rhi_impl/device.rs:1622`, `:1974-1976`, `:925`, `:2143` |
| F3c | Depth conventions differ **by path**: `LESS` for Deferred's custom-linear depth, `GREATER` for hardware reverse-Z | `rhi_impl/device.rs:1619-1621`, `:2132-2134` |
| F3d | Deferred's depth reaches the transparent slot at `SHADER_READ_ONLY_OPTIMAL`; Forward/VB at a depth-stencil write (or SRO under an SDF leg); **only ForwardPlus** already declares `FRAG / DS_ATTACHMENT_READ / DEPTH_ATTACHMENT_OPTIMAL` | `graph_bridge.rs:1259-1263`, `:1296-1300`, `:1327-1331`, `:2238-2242`, `:743`, `:2001` |
| F4 | `vkCmdDispatchIndirect` is **loaded** and has **zero call sites** | `device.rs:578`, `:2029` |
| F5 | `vkCmdDrawIndexedIndirect` is loaded and used in production; stride 20; `draw_count` forced to 1 | `device.rs:652`; `passes/vb.rs:2454` |
| F5b | `VkDrawIndexedIndirectCommand` is `#[repr(C)]` ⇒ `offset_of!(.., instance_count) == 4` is real; **`first_instance` MUST be 0 on this device** — "a nonzero value here is a silent corruption class" | `ffi.rs:3595-3616` (verbatim `:3599-3602`) |
| F6 | `vkCmdDrawIndirect` (non-indexed) and `…IndirectCount` are **not** loaded | `passes/vb.rs:2646` |
| F7 | Framegraph API: `add_buffer[_seeded]/add_image[_seeded] → add_pass → image_access/buffer_access` | `framegraph/graph.rs:334,349,415,459,496,518` |
| F7b | Four seed constructors. `transition`'s src half prefers `flush` over `visible`; a **read clears the pending flush** ⇒ `seeded_readers` **is** the written-then-drained state | `sync.rs:245,271,288,313`; `:351-413` |
| F7c | `reset()` + `compile()` refill per-resource state from `res_seed` every frame — **the seed is the ONLY cross-frame carrier** | `graph.rs:303-326`, `:588-594` |
| F7d | A read with no prior in-graph writer **panics** — the provenance guard | `graph.rs:652-674` |
| F8 | Declaration order **is** execution order | `graph.rs:17-25` |
| F9 | A declared ResId no pass names routes **zero** barriers | `graph_bridge.rs:3716-3727` |
| F9b | The shipped armed/disarmed template is a **conditional tail**: `Option<PassId>` + ResIds appended last | `graph_bridge.rs:4607-4628`, `:1051-1058` |
| F10 | Exactly one `Some(BlendState)` pipeline exists workspace-wide (UI → swapchain, outside the graph) | `boyko_render/src/ui/resources.rs:237` |
| F12 | Shared COMPUTE push range = `max(80, 112) = 112` of a 128 B floor | `rhi_impl/mod.rs:222-242` |
| F13 | The eDSL has no atomics, no `groupshared`, no stores, no texture sampling — function bodies only | `cf.rs`, `emit/cf.rs` |
| F13b | `Cf` has `uadd`/`umul`/`uint_lit`/`float_to_uint`/`buffer_load`/`uge`/`ugt`/`select`/`vec3_*`/`call1`/`call2`; **no** bitwise-or-shift on `Uint`, **no** `asuint`/`asfloat`/`f16tof32`, **no** `dot`, **no** `exp2` | `cf.rs:246-448`, `:276-339` |
| F13c | `sin`/`cos` live on **`InterpBackend`**, not `FieldScalar`; `Cf::Scalar: FieldScalar` ⇒ a `Cf` leaf cannot call them | `interp.rs:75,77`; `scalar.rs:75-105` |
| **F13d** | `FieldScalar` **does** carry `div` and `sqrt` (also `add/sub/mul/neg/min/max/clamp01/lerp/abs/select`) — so a reciprocal square root is reachable, though the house forbids putting a division in a bit-exact contract | `scalar.rs:71-105` (`div`@:77, `sqrt`@:101) |
| F14 | `emit_probe_gi.rs` owns a **whole** `.hlsl` as a Rust template with eDSL holes | `boyko_shaderdsl/src/bin/emit_probe_gi.rs:42-67` |
| F15 | `*_spv_sync` tests **skip (=PASS) without dxc** | `marcher_spv_sync.rs:32-48` |
| F16 | Dense components are **always** `ResidencyKind::Cpu` | `component_registry/mod.rs:779-796` |
| F17 | A `ResidencyKind::Gpu` component forces a GPU-pure archetype; a CPU `Query` naming it panics | `archetype.rs:1476-1505`; `query_state.rs:107-115` |
| F18 | `ScratchColumn<T>` is `ComponentPool`-backed, Build/Solve split; capacity is `pool_reserve_rows`; unused ⇒ **unbacked VA, zero committed pages** | `mesh_draw.rs:412-415`, `:429-465` |
| F19 | Per-frame host writes need a borrowed `FrameWriteToken` into a **per-FIF staging ring**; re-upload gated by a **writer-side monotonic generation, never a hash** | `upload.rs:1-23`; `light_gate.rs:21-48` |
| F20 | `Assets<T>` is `ComponentPool`+`VmColumn`-backed with `high_water()` / `dirty_gen()` | `asset/assets.rs:200-216,607,697` |
| F21 | Wave intrinsics available under `-T cs_6_0`; `groupshared` + `InterlockedAdd` shipped | `cluster_cull.hlsl:170-173,376` |
| F22 | Bare `register(tN/uN)` shares one binding-number space; mitigation is a host layout table with a `const` assert | `vb_batch_cull.comp.hlsl:475-481`; `gpu_scene/mod.rs:471-554` |
| F23 | `FrameGraph::with_capacity(16,16,64)`; `declare_vb_graph` already declares 33 resources | `frame_driver.rs:189` |
| F24 | A **dark** feature is not free: the VB-SV0 inline detour cost **+75 %** with the feature OFF, invisible to every byte gate | `sdf_mesh_shadow.comp.hlsl:5-13` |
| F25 | `robustBufferAccess` is **OFF** — an out-of-range fetch is UB, not a clamp | `graph_bridge.rs:3794-3796` |
| F26 | `&mut T` has **no** change tracking; only `Mut<T>` stamps ticks | `query/data/mut_.rs:11` |
| F27 | `Time::delta_secs()` is already `min(raw, max_delta)` (default **250 ms**, "the single death-spiral guard") scaled by `relative_speed`, and **zero while paused** | `time.rs:6`, `:39`, `:141-160`, `:165` |
| **F27b** | The worst-case substep bound is `⌈(max_delta × speed + timestep) / timestep⌉`; `relative_speed` is public and validated only `finite && >= 0` ⇒ **the bound is unbounded in practice** (speed 8.0 at stock defaults ⇒ 129) | `app.rs:449-450`; `time.rs:120-139` |
| **F28b** | `fixed_builder` is created **lazily on the first `*_in(CoreSchedule::Fixed, …)`** call, and `event_policy_cfg: None` auto-resolves at `finish` to "`WaitForFixed` **iff a Fixed schedule was configured**"; `fixed_steps_since_swap` then holds the event swap across 0-substep frames | `app.rs:142-145`, `:157-159`, `:164-167` |

### Research corpus facts

R1 the 2014 skeleton · R2 **AoS packed structs, not SoA** ("the CPU-side SoA argument does not transfer") · R3 dual alive lists are **required** · R4 Wicked's `simulateCS` writes vertex data separately; `THREADCOUNT_SIMULATION = 256` · R5 additive needs no sort; bitonic → FFX radix · R6 SDF collision beats depth-buffer collision · R7 no engine makes particles entities · R8 pitfall: buffers sized from a constant (Hanabi #493) · R9 pitfall: rendering capacity instead of live count · R10 pitfall: sorted particles + TAA motion vectors · R11 pitfall: many tiny emitters; per-effect batch keys · R12 scale calibration.

### Invariants

1. **Principle 0** — per-particle state is a GPU-contiguity buffer (the sanctioned exception) with **no CPU mirror**. CPU staging is `ScratchColumn`, never `std::Vec`.
2. **Structural absence** — `mode == Off` ⇒ no pass, no ResId, no pipeline, no buffer, no shader loaded.
3. **Subsystem containment (new in Rev 3, M4)** — installing `ParticlePlugin` must not change any schedule set, event policy, or clock that another subsystem observes.
4. **Declare/record parity** — one predicate read at both sites.
5. Every `unsafe` carries `// SAFETY:`. **No new third-party dependency.**

---

## Counter and list ownership *(NORMATIVE — everything below derives from this section)*

> This section is the single source of truth for the alive/dead bookkeeping, the wave-level atomic mechanism, and the clock. D3's pseudocode, A1–A5, the seed table's **access column**, and the hazard table are **derived** from it. Where any of them appears to disagree, this section wins and the other is a defect.

### The four pass boundaries

```
B0   frame edge (start of frame N; state carried only by the seeds, F7c)
       kickoff        1 thread
B1   kickoff → emit
       emit           DispatchIndirect
B2   emit → sim
       sim            DispatchIndirect
B3   sim → draw
       draw           DrawIndexedIndirect  (P0: 1 draw; P2: 2 draws)
B0'  frame edge (= B0 of frame N+1, with the alive roles swapped by host parity)
```

### Who writes what, and where

| Datum | Home | Written by | At which pass | Read by |
|---|---|---|---|---|
| `alive_count_cur` | `p_counters` | **kickoff only** (`= alive_count_next`, then `+= real_emit_count`) | kickoff | sim (the guard), kickoff |
| `alive_count_next` | `p_counters` | **sim only** — the **LIST** counter, shared by both blend classes | sim | **kickoff of frame N+1** |
| `dead_count` | `p_counters` | kickoff (pre-decrement) **and** sim (push) — different passes, barrier between | kickoff, sim | kickoff, sim |
| `dead_base` · `emit_append_base` · `real_emit_count` | `p_counters` | **kickoff only** | kickoff | emit |
| `clamped_spawns` | `p_counters` | kickoff only (diagnostic) | kickoff | host, cold |
| `p_alive_read[..]` | alive buffer @ parity | **emit only** (writes at `emit_append_base + gid`) | emit | sim |
| `p_alive_write[..]` | alive buffer @ parity^1 | **sim only** (writes at `idx`, from `alive_count_next`) | sim | sim of frame N+1, as `p_alive_read` |
| `p_dead[..]` | dead buffer | emit reads `[dead_base + gid]`; sim pushes | emit (read), sim (write) | emit |
| `p_particle[slot]` | particle buffer | emit (init) and sim (step) — **one lane owns a slot** | emit, sim | sim |
| `p_render[·]` | render buffer | **sim only**, at the class-dense render index | sim | draw (VS) |
| `additive.instanceCount` | `p_draw_args` | **sim only** — the **additive RENDER** counter; the `InterlockedAdd` yields the position, the final value is the count | sim | command processor at draw |
| `alpha.instanceCount` *(P2)* | `p_draw_args` | **sim only** — the **alpha RENDER** counter; same trick, index mirrored to the top | sim | command processor at draw |
| *(parity)* | **host** | host frame counter | pre-submit | selects `sets[parity]`; **no GPU field** |
| `steps` *(the substep count)* | `ParticleClock` (CPU) | **A1 only**, clamped once (M3) | Main | A1's own advance **and** the sim's push constant — one value, two consumers |

### The wave-level atomic mechanism *(M1 — one symbol per value)*

Inside `particle_sim`, for the lanes of one wave. `survives` is the per-lane predicate; the dying path is symmetric on `dead_count`.

```
w_count = WaveActiveCountBits(survives)        // survivors in THIS wave           (per-wave)
w_lane  = WavePrefixCountBits(survives)        // this lane's rank among them, 0-based (per-lane)
if (WaveIsFirstLane())
    w_base_raw = InterlockedAdd(p_counters.alive_count_next, w_count)   // returns the OLD value
w_base  = WaveReadLaneFirst(w_base_raw)        // the wave's LIST reservation base  (per-wave)
idx     = w_base + w_lane                      // this lane's LIST position         (per-lane)
p_alive_write[idx] = slot
```

and, for the render index, on the counter of this lane's blend class (P0 has only the additive class, so the class select is a compile-time constant):

```
r_count = WaveActiveCountBits(survives && class == C)
r_lane  = WavePrefixCountBits(survives && class == C)
if (first lane with class == C)
    r_base_raw = InterlockedAdd(p_draw_args.C.instanceCount, r_count)
r_base  = WaveReadLaneFirst(r_base_raw)
r_pos   = r_base + r_lane                      // class-dense render position
p_render[render_index(C, r_pos)] = pack_render(p, e)
      where render_index(Additive, r) = r
            render_index(Alpha,    r) = CAP - 1 - r          // P2 only
```

**Exactness (critic-verified, restated).** Reservations from one `InterlockedAdd` counter are contiguous and disjoint, so the multiset of `(w_base, w_count)` pairs partitions `[0, Σ w_count)` exactly; the counter's final value is `Σ w_count`, independent of the order in which waves retire, because integer addition is commutative and associative. The same holds per class for the render counters. **No `InterlockedMax` and no mirror is needed** — Rev 2's mirror existed only because it had one counter trying to serve two different quantities.

**Why two counters and not one (the N3c proof, restated because Rev 3's counters are different quantities).** The sim must publish to two consumers with incompatible synchronisation needs: next frame's *kickoff* (a compute read needing **availability** of the sim's write) and this frame's *command processor* (a `DRAW_INDIRECT` fetch, which is the buffer's frame **terminal** and therefore what next frame's first write must WAR against). `ResSync` cannot express both on one resource — `flush_*` and `visible_*` are mutually exclusive across the four constructors and `transition` prefers `flush` (`sync.rs:373-383`). So the list count lives in `p_counters` (terminal: an undrained compute write ⇒ `seeded_writer` ⇒ real RAW for kickoff) and the render counts live in `p_draw_args` (terminal: the indirect read ⇒ `seeded_readers` ⇒ real WAR). In Rev 3 these are genuinely different numbers whenever more than one blend class is live, which is why the arrangement is now natural rather than a mirror.

**Emit takes zero atomics.** Kickoff is a one-thread pass, so it pre-decrements `dead_count` **and** pre-increments `alive_count_cur`, publishing `dead_base` and `emit_append_base`. Emit lane `gid` computes both indices arithmetically:

```
slot = p_dead[dead_base + gid]
pos  = emit_append_base + gid
```

The sim's guard bound and kickoff's dispatch size are then **the same field**, `alive_count_cur`, read twice — not two derivations that must be kept in agreement.

### The partition at each boundary

Let `A` = `alive_count_cur`, `D` = `dead_count`, `E` = `real_emit_count`, `N` = `alive_count_next`.

| Boundary | Partition of the `CAP` slots | Equality |
|---|---|---|
| **B0** (frame edge) | live = `p_alive_read[0 .. N_prev)` · free = `p_dead[0 .. D)` | `N_prev + D == CAP` |
| **B1** (kickoff→emit) | live = `p_alive_read[0 .. emit_append_base)` · **reserved = `p_alive_read[emit_append_base .. A)` ⟷ `p_dead[dead_base .. dead_base+E)`** (a one-to-one in-flight window) · free = `p_dead[0 .. D)` | `A + D == CAP` — kickoff pre-increments `A` and pre-decrements `D` in the same one-thread pass, so the reservation is accounted on both sides simultaneously |
| **B2** (emit→sim) | live = `p_alive_read[0 .. A)` (incl. the fresh spawns) · free = `p_dead[0 .. D)` | `A + D == CAP` |
| **B3** (sim→draw) | live = `p_alive_write[0 .. N)` · free = `p_dead[0 .. D)` · render records at `p_render[0 .. additive.instanceCount)` **and** `p_render[CAP-alpha.instanceCount .. CAP)`, with `additive.instanceCount + alpha.instanceCount == N` | `N + D == CAP` |

The two-term equality therefore holds at **all four boundaries**. The three-term form is stated anyway because it names the in-flight window, and the property test asserts the *window* explicitly — a lost reservation would keep the equality true while leaking slots. **At B3 the test also asserts the class split sums to `N`** (M2: this is the assertion that would have caught the alpha leak).

**Frame 0.** The monotonic frame counter starts at 0; `sets[0]` binds physical `alive[0]` → `p_alive_read` and `alive[1]` → `p_alive_write`. Every device buffer is zero-filled at boot **except** `p_dead`, which is boot-initialised to the identity permutation `p_dead[i] = i` with `dead_count = CAP` — the only non-zero boot fill, and what makes B0's equality true at `N_prev = 0`. Frame 0: kickoff sets `A = 0 + E`, `emit_append_base = 0`; emit writes `E` records and `E` list entries; the sim walks exactly those `E`. No leak, no stale read, no dependence on any GPU-side parity state.

### The clock *(M3, M4, M6)*

`ParticleClock` is a subsystem-owned `Resource`. It does **not** use `FixedTime` and does **not** cause a `CoreSchedule::Fixed` schedule to exist (F28b — that would flip the process-wide event policy).

```
// A1, once per rendered frame, on CoreSchedule::Main
accumulator += Time::delta_secs();               // already min(raw,max_delta)·speed, 0 while paused (F27)
raw_steps    = floor(accumulator / timestep);    // unbounded under relative_speed (F27b)
steps        = min(raw_steps, PARTICLE_SUBSTEP_CEILING);   // THE clamp — once, here (M3)
dropped_steps += raw_steps - steps;              // diagnostic
accumulator -= raw_steps as f32 * timestep;      // DROP the excess, do not carry it
```

**One number.** `steps` is used by A1 to advance every emitter (`dt = steps · timestep`) **and** is pushed verbatim to `particle_sim` as its loop bound. The shader's own `min(pc.steps, PARTICLE_SUBSTEP_CEILING)` survives solely as the F25 hang guard against a corrupt push constant; it can never bind, because the host already clamped.

**Why the excess is dropped, not carried.** Carrying `raw_steps - steps` would build a backlog that never drains under sustained slow-motion, converting a transient into a permanent lag. Dropping is what `Time::max_delta` already does for the frame delta (F27, "the single death-spiral guard"); this is the same policy at the subsystem's own rate, and it is counted.

**`overstep_fraction() = accumulator / timestep`** ∈ [0,1) is exposed for rung P2b.

---

## Key decisions

### D1 — Particles are not entities; emitters are *(unchanged; R7)*

At the house's measured spawn rate, 20k spawns/frame through `Commands` is 0.6–2 ms of CPU against ~2 µs of GPU emit. Rejected: dense storage (CPU-only, F16 ⇒ 48 MB/frame upload at 1M ≈ 3 ms) and `ResidencyKind::Gpu` columns (GPU-pure archetype, CPU queries panic — F17). Trade-off: particles are not queryable, observable, or serializable.

### D2 — Packed AoS sim record (48 B) + packed AoS render record (32 B) *(unchanged; R2, R4)*

```
p_particle : ParticleSim    48 B   emit W / sim RW              — the sim's working set
p_render   : ParticleRender 32 B   sim W  / draw R (sequential) — the draw's working set
p_dead     : u32             4 B
p_alive[2] : u32             8 B
                            ────
                            92 B / particle
```

AoS because R2 is decisive: under the alive-list gather a 48 B 16-aligned record is **one fully-consumed 64 B line per particle**, where SoA would fetch three lines and use 48 of 192 bytes. Two records because the draw reads a dense sequential 32 B stream with no gather, and — the larger win — all curve/ramp evaluation (and P3's lighting) moves into the sim, **once per particle**, instead of the VS's 4× per particle.

| | Sim writes | Draw reads | Effective |
|---|---|---|---|
| Render record | +32 B sequential (~80 %) | 32 B sequential (~80 %) | 64/0.8 = **80** |
| Draw gathers `p_particle` | 0 | 64 B line, random (~45 %) | 64/0.45 = **142** |

Named as a P0 gate measurement so the number can refute the choice.

### D3 — Dead list + dual alive lists + one-thread kickoff *(endorsed; pseudocode derived from §Counter and list ownership)*

```
kickoff (1 thread)
    A = alive_count_cur = alive_count_next
    alive_count_next = 0
    D = dead_count = max(0, dead_count)             // release-present clamp
    E = real_emit_count = min(pc.requested_spawn, D)
    clamped_spawns += pc.requested_spawn - E
    dead_base        = (dead_count = D - E)         // pre-DECREMENT
    emit_append_base = A
    alive_count_cur  = A + E                        // pre-INCREMENT
    dispatch_args.emit = (ceil(E / 256), 1, 1)
    dispatch_args.sim  = (ceil(alive_count_cur / 256), 1, 1)
    draw_args.additive = { indexCount: 6, instanceCount: 0, firstIndex: 0, vertexOffset: 0, firstInstance: 0 }
    draw_args.alpha    = { …, instanceCount: 0, …, firstInstance: 0 }        // P2; zeroed at P0

emit (indirect, 256 thr)   — ZERO atomics
    if (gid >= real_emit_count) return
    slot = p_dead[dead_base + gid]
    p_particle[slot] = init(effect, pcg32(seed ^ gid ^ frame))
    p_alive_read[emit_append_base + gid] = slot

sim (indirect, 256 thr)
    if (i >= alive_count_cur) return                 // the same field kickoff sized the dispatch from
    slot = p_alive_read[i];  p = p_particle[slot];  e = Effects[p.effect_index]
    for s in 0 .. pc.steps:  integrate(p, e, pc.timestep)     // pc.steps is host-clamped (M3)
    if (p.life <= 0) { wave-append slot to p_dead via dead_count; return }
    p_particle[slot] = p
    // §Counter and list ownership, "wave-level mechanism":
    idx   = w_base + w_lane        // from alive_count_next  — the LIST position (both classes)
    r_pos = r_base + r_lane        // from <class>.instanceCount — the class-dense RENDER position
    p_alive_write[idx] = slot
    p_render[render_index(class, r_pos)] = pack_render(p, e)

draw   DrawIndexedIndirect(p_draw_args + 0)   // additive; P2 adds a second at +24
```

Cost is `O(alive)` with **no premise about slot distribution**. R3: dual alive lists are required, not optional.

**No concurrent push/pop on the dead stack.** Emit only reads `p_dead`; the sim only pushes; a derived barrier separates the passes. The classic dead-stack race is **structurally impossible**.

**No fourth "finish" pass.** The sim's own atomics publish the list count and both render counts. This closes R9 (rendering capacity instead of live count) by construction.

**Rejected:** Rev 0's slot-order stream; compaction; **fusing emit+sim** (would make the dead stack concurrently pushed and popped — rejected outright, not deferred).

### D4 — Three dispatches + indirect draw(s); the split indirect block *(endorsed)*

`p_dispatch_args` (32 B): two `VkDispatchIndirectCommand` at offsets 0 and 16; **written only by kickoff, read only by `DRAW_INDIRECT`**. `p_draw_args` (64 B): **two** `VkDrawIndexedIndirectCommand` at offsets 0 and 24, so `additive.instanceCount` is at byte **4** and `alpha.instanceCount` at byte **28** — both single-sourced by `offset_of!` and **fed into the generator**, so the shaders' word indices are emitted, never typed. Both offsets are multiples of 4.

The split exists because a fused block would take both a `DRAW_INDIRECT` read and a `COMPUTE` write on one ResId inside the sim pass, and the framegraph has no sub-buffer granularity. It also buys `p_dispatch_args` a **barrier-free second read**.

Indirect dispatch is required (`alive_count_cur` and `real_emit_count` are GPU-side); particles are the first consumer of the F4 seam, via the raw fn table exactly as `passes/vb.rs:2454` does. `vkCmdDrawIndirect` is not loaded (F6), so the draw is indexed with a **12-byte** 6-entry `u16` index buffer.

**Group size 256** for emit and sim (R4), 1 for kickoff — a `static const`, mirrored host-side, pinned by a `LocalSize` opcode assertion.

### D5 — Wave-aggregated atomics; the budget, per wave *(M1, M2)*

A returning `InterlockedAdd` on one address serializes at ≈1 op/clock in L2: 1M naive RMWs ≈ **0.5 ms/frame**; wave-aggregated at wave32 ≈ **16 µs**.

| Wave content | Counters touched | Atomics |
|---|---|---|
| all dying | `dead_count` | **1** |
| all surviving, one blend class | `alive_count_next`, `<class>.instanceCount` | **2** |
| mixed survive/die | + `dead_count` | **3** |
| mixed blend classes *(P2 only)* | `alive_count_next`, both `instanceCount`s | **3** (+1 if also mixed survive/die ⇒ 4) |

**P0 is additive-only, so the mixed-class arm is unreachable at P0** and the steady state is 2 per wave. At 1M survivors: 2 × 31 250 ≈ 62 500 ops ≈ **32 µs**. `groupshared` aggregation (÷256) is rejected at P0 — two `GroupMemoryBarrierWithGroupSync` cost more than the 16 µs they would save.

### D6 — A subsystem-owned fixed-rate clock *(M3, M4, M6 — replaces Rev 2's `FixedTime` dependency)*

**What.** `ParticleClock` (Resource) owns `timestep` (default 1/64 s), `accumulator`, `steps` and `dropped_steps`. A1 advances it once per rendered frame from `Time::delta_secs()` and computes `steps` per §Counter and list ownership. The **same** `steps` drives A1's emitter advance and the sim's loop bound.

**Why not `FixedTime` (M4).** `fixed_builder` is created lazily on the first `*_in(CoreSchedule::Fixed, …)` call, and `event_policy_cfg: None` auto-resolves to "`WaitForFixed` iff a Fixed schedule was configured" (F28b). A particle plugin that registered anything on `Fixed` would therefore flip **every event type in the process** from `EveryFrame` to `WaitForFixed`, and `fixed_steps_since_swap` would hold event swaps across 0-substep frames — at 200 fps / 64 Hz, two frames in three. Input, UI and collision consumers would change behaviour because a rendering plugin was installed. Patching `event_policy_cfg` back is worse: it would then override the resolution a user's own later Fixed schedule should have produced, and it would make plugin order load-bearing. Owning the clock removes the coupling entirely.

**What is preserved.** Everything D6 ever actually needed:
1. **A constant `dt`**, so `damping = exp2(-drag · timestep)` and the rotation multiplier `(cos ω·timestep, sin ω·timestep)` stay host-precomputable per effect — this is what deletes `exp2` and all trig from the GPU.
2. **Bounded tunneling** for D9's SDF collision, governed by `v · timestep`.
3. **The engine's inflow guard**, inherited free: `Time::delta_secs()` is already `min(raw, max_delta)`-clamped (F27).

**What is gained.** Pausing the game pauses particles and slow-motion slows them, both free (`Time` is pause-aware and speed-scaled). A project may run particles at a different rate from gameplay physics.

**What is deleted.** `particle_fixed_heartbeat`; the F28 no-Fixed-schedule freeze hazard (`steps_this_frame()` is never consulted); Rev 2's Open Question 7.

**The ceiling is real and reachable (M3).** F27b: the worst case is `⌈(max_delta × speed + timestep) / timestep⌉` and `relative_speed` is public and unbounded, so speed 8.0 at stock defaults yields 129 > 64. `PARTICLE_SUBSTEP_CEILING = 64` is therefore a **time-dropping clamp**, applied once on the host, counted in `dropped_steps`. Consequence, stated: at speed 8.0 on a 250 ms hitch, particles age at 64/129 of wall-clock for that one frame. The alternative — an unbounded device loop — is a GPU-hang class (F25). The ceiling is a boot constant the owner may raise; it costs only shader loop iterations on hitch frames.

**Cost of a 64-step frame** at 100k alive: 64 × 100k integrator iterations ≈ 6.4M × ~20 flops ≈ 128 MFLOP ≈ a few hundred µs; with `-D SDF_COLLIDE`, 64× the field evaluations ≈ 6 % of a 10 TFLOP part. That frame is already ≥250 ms of wall-clock. Stated rather than assumed.

**Trade-off, stated (M6).** Above the step rate, most frames step zero times — at 200 fps with a 64 Hz timestep, two in three — so particles visibly move at 64 Hz against a 200 Hz camera. **P0 accepts fixed-rate stepping without render-time interpolation.** Rung P2b adds it.

### D7 — Composite into `lit`; per-path compare op; per-path depth state *(unchanged)*

| Path | lit ResId | depth ResId | compare op | Depth state at the transparent slot | Derived |
|---|---|---|---|---|---|
| **Deferred** | `lit` (5) | `depth` (3) | `LESS` | `SHADER_READ_ONLY_OPTIMAL` (three pre-lit consumers) | **layout transition** `SRO → DEPTH_ATTACHMENT_OPTIMAL` |
| **Forward** | `lit` (0) | `forward_depth` (1) | `GREATER` | depth-stencil write (or SRO under an SDF leg) | **availability barrier** (or a transition) |
| **ForwardPlus** | `lit` (0) | `forward_depth` (1) | `GREATER` | `FRAG / DS_ATTACHMENT_READ / DEPTH_ATTACHMENT_OPTIMAL` already declared | **nothing — free** |
| **VisibilityBuffer** | `lit` (0) | `vb_depth` (2) | `GREATER` | depth-stencil write (or SRO) | **availability barrier** (or a transition) |

```rust
// rhi_impl/device.rs — additive; build_graphics_pipeline is already parameterised (F3b)
pub fn create_graphics_pipeline_particle(
    &self, desc: &GraphicsPipelineDesc<Vulkan>, set1_layout: VkDescriptorSetLayout,
    depth_compare: i32,                       // LESS (Deferred) | GREATER (reverse-Z paths)
) -> Result<VulkanGraphicsPipeline, VulkanError> {
    self.build_graphics_pipeline(desc, Some(set1_layout), depth_compare, /*depth_write=*/ false)
}
```

The compare op is resolved **once at boot** from `ResolvedRenderPath`, so exactly one `VkPipeline` exists per process. **No image-create change** — F1: `lit` already carries `COLOR_ATTACHMENT` on every path.

P0 depth access is attachment-only (`EARLY|LATE_FRAGMENT_TESTS`, `DEPTH_STENCIL_ATTACHMENT_READ`, `DEPTH_ATTACHMENT_OPTIMAL`); no `FRAGMENT_SHADER|SHADER_READ` bit, no new layout constant, no new `BindGroupEntry` variant. All read-only-depth plumbing is P2's.

**Trade-off (F2).** `lit` is 8-bit post-tonemap: additive clips at white and contributions below 1/255 round to zero. Effects must be authored with contributions ≥ 2/255. Open Question 1.

### D8 — Emit: the prefix orders **lanes only** *(unchanged)*

`first_spawn` maps `gid → emitter_index` (cooperative `groupshared` load of the ≤256-entry array, 8-step branchless binary search). The slot comes from `p_dead[dead_base + gid]` and the list position from `emit_append_base + gid`. **Three independent indexings, none assuming structure in another.**

**R11 closed structurally.** One global pool ⇒ one emit and one sim dispatch regardless of emitter count; and `tex_index` is a bindless index in the render record, so there is **no per-effect batch key** and one draw covers every effect.

### D9 — P1 SDF collision with a Lipschitz-bounded skip *(unchanged; R6)*

`-D SDF_COLLIDE` binds `StructuredBuffer<uint> Buf` then `#include "sdf_field.hlsli"` (the include contract requires `Buf` first; `sdf_mesh_shadow.comp.hlsl:93-97` is the template). Per substep:

```
if (cached_d - speed*timestep/FIELD_LIPSCHITZ_L > radius) { cached_d -= speed*timestep/L; skip }
else { d = sdf(p); if (d < radius) { n = sdf_normal(p); resolve } cached_d = d }
```

`sdf()` walks ≤16 edits × ~15 flops ≈ 240 flops — 0.1 % of a 10 TFLOP part at 1M particles × 1 substep. Response: `p += n*(radius−d)`; `v' = (v − v_n)(1−friction) − v_n·restitution`.

### D10 — Sorting: additive needs none; P2 is one FFX-shaped radix pass; the blend partition *(M2)*

Additive is commutative, and under 8-bit saturation `sat(sat(x)+y) = min(1, x+y)` is order-independent. With `depth_write = OFF` and `depth_test = ON`, opaque geometry still occludes. **P0 ships unsorted, provably.**

P2 sort: **one FFX-shaped pass** (histogram → 256-bin scan → scatter) over an 8-bit quantized log-depth key — 3 dispatches, ≈0.3–0.5 ms at 1M. Rejected: bitonic (R5 — production moved off it), 4-pass 32-bit radix (3–4× the cost for precision invisible in an 8-bit blend). `SortMode::Wboit` is kept as an opt-in for smoke-class media.

**The blend partition.** `first_instance` **must be 0** (F5b, verbatim: "a nonzero value here is a silent corruption class"), so two draws cannot be distinguished by `firstInstance`. The mechanism, completed per M2:

- **List index — shared.** Every survivor of either class takes `idx` from `alive_count_next` and writes `p_alive_write[idx]`. **This is what prevents the alpha leak**: kickoff reads only `alive_count_next`, so a class that allocated its list index anywhere else would vanish from the next frame's walk entirely.
- **Render index — per class.** Additive takes `r_pos` from `additive.instanceCount` and writes `p_render[r_pos]`; alpha takes `r_pos` from `alpha.instanceCount` and writes `p_render[CAP-1-r_pos]`.
- **Draw.** Two `VkDrawIndexedIndirectCommand` slots, `first_instance = 0` in both; the VS computes `render_index = pc.index_base + pc.index_step * SV_InstanceID`, with `(0, +1)` for additive and `(CAP-1, -1)` for alpha.

This gives: no `firstInstance`, **no finish pass** (each base is a compile-time constant per draw), **no per-class capacity cap** (the two ends share `CAP` dynamically), **no shader variant** (two push-constant values, one pipeline), and A5's sequential read preserved in both directions. At P0 the transform is the identity, slot 1 is zeroed and its pass undeclared.

**R10 — sorting vs motion vectors.** Godot: motion vectors work only under index order. **Hard rule: `SortMode != None` ⇒ particle motion vectors disabled**, as a resolver truth-table row and a boot `debug_assert`.

### D11 — P3 lit particles: per-particle froxel lookup, in the sim *(unchanged)*

Per-fragment lighting at 100k particles × ~50 px coverage = 5M cluster lookups/frame; evaluating at the particle centre in the sim = 100k — 50×, on exactly the geometry class where per-pixel lighting buys least. D2's render record makes this natural. `-D LIT_PERPIXEL` remains for the minority that need it.

### D12 — Shaders: one generator owns each whole `.hlsl` *(unchanged)*

F13: the eDSL has no atomics, `groupshared`, stores, or texture sampling, so the skeleton is hand-written. Following F14, **`boyko_shaderdsl/src/bin/emit_particles.rs` owns all five files as Rust `format!` templates with eDSL holes** — the glue is single-sourced and cannot be hand-edited undetected. The `p_draw_args` offsets are generator inputs.

**Binding discipline (F22).** Explicit `[[vk::binding(N, S)]]`, never bare `register(tN/uN)`; a host-side `PARTICLE_LAYOUT_ENTRIES` table with a `const fn` assert makes a kind/index mismatch a **build** error.

**Push constants** ≤ 32 B each, under F12's 112 B; dedicated layouts, so `COMPUTE_PUSH_CONSTANT_RANGE_BYTES` does not move.

### D13 — Default-off on three axes, conditional tail *(unchanged)*

| Axis | Mechanism | Precedent |
|---|---|---|
| Owner armed? | `ParticleConfig { mode }`, `#[default] Off`, `enabled()` derived from the discriminant | `OcclusionConfig:119-140` |
| Entity is an emitter? | `ParticleEmitter` presence (opt-**IN**) | `OcclusionCulling:155` |
| Emitter on now? | `#[component(storage = "bitset")] EmitterActive` + `Enabled<EmitterActive>` | `RenderEnabled:251` |

Declaration follows the shipped conditional-tail template (F9b): `Option<ResId>` / `Option<PassId>`, ResIds appended last, **nothing declared when disarmed**. Disarmed ⇒ no pass, no ResId, no pipeline, no buffer, no bytes, `lit`'s usage unchanged.

**F24 applied:** particles get their own passes and pipeline, never a gated branch inside anyone else's shader — so there is no dark tax to measure.

### D14 — Capacity is boot-frozen *(unchanged)*

`CAP` is read once at boot; exceeding it clamps in kickoff and increments `clamped_spawns`. Default **262 144** (24.1 MB). `CAP` bounds **memory only** — per-frame work is `O(alive)`.

### D15 — Fixed-size CPU-facing tables get release-present clamps *(unchanged; R8)*

Hanabi shipped a 12 B indirect overrun at ~260 instances because a GPU table was sized from a constant. Mitigation: (a) the writer clamps in **release** (extra emitters dropped and counted), (b) `debug_assert!` alongside, (c) an OOB test at `MAX + 1`. F25 makes this non-negotiable.

### D16 — `ParticleEmitter`'s hot/cold mix is a decided exception *(unchanged)*

At ≤256 rows a split would cost a second column fetch to save nothing measurable. F26: `&mut T` stamps nothing, so the unconditional accumulator write is free.

### D17 — Subsystem containment *(new; M4)*

`ParticlePlugin::build` may insert **only**: its own resources (`ParticleConfig`, `ParticleClock`, the two `ScratchColumn` resources, the two generation resources), its own components' hooks, and its own systems into `CoreSchedule::Main`. It may **not** touch `CoreSchedule::Fixed`, `event_policy_cfg`, `Time`, `FixedTime`, or any schedule label another subsystem observes. Enforced by a gate that asserts the resolved `EventUpdatePolicy` and the presence/absence of a Fixed schedule are byte-identical with and without the plugin.

---

## Data structures

```rust
/// The sim's working set — one contiguous AoS record, one 64 B line per particle
/// under the alive-list gather (R2). Field order follows Hanabi's packer.
#[repr(C, align(16))]
pub struct ParticleSim {                 // 48 B
    position: [f32; 3], life_remaining: f32,     // 16
    velocity: [f32; 3], cached_field_d: f32,     // 16  (.w = P1 Lipschitz cache; 0 at P0)
    color_rgba8:   u32,                          //  4
    size0_invlife: u32,                          //  4  f16 size0 | f16 inv_life_total
    effect_flags:  u32,                          //  4  u16 effect_index | u16 flags (incl. blend class)
    rot_cs:        u32,                          //  4  snorm16 cos | snorm16 sin (M7: no trig, no renorm)
}

/// The draw's working set. Written by the sim at the CLASS-DENSE render index;
/// read by the VS at `pc.index_base + pc.index_step * SV_InstanceID`.
#[repr(C, align(16))]
pub struct ParticleRender {              // 32 B  (40 B under rung P2b — see M6)
    position: [f32; 3], size: f32,               // 16
    color_rgba8: u32,                            //  4  ramp- (and P3: light-) resolved
    rot_cs:      u32,                            //  4  snorm16 cos | snorm16 sin
    tex_index:   u32,                            //  4  bindless — one draw covers every effect
    flags:       u32,                            //  4
}

/// One cache line. NO `ping` field — the host owns parity. NO `alpha_count`
/// — the render counters live in `p_draw_args` (M2).
#[repr(C, align(64))]
pub struct ParticleCounters {            // 64 B
    alive_count_cur:  u32,   // kickoff only
    alive_count_next: u32,   // sim only — the LIST counter, BOTH blend classes (M2)
    dead_count:       u32,   // kickoff (pre-decrement) + sim (push)
    dead_base:        u32,   // kickoff only
    emit_append_base: u32,   // kickoff only
    real_emit_count:  u32,   // kickoff only
    clamped_spawns:   u32,   // kickoff only, diagnostic
    _pad: [u32; 9],
}

/// Written ONLY by kickoff; read ONLY by DRAW_INDIRECT.
#[repr(C, align(16))]
pub struct ParticleDispatchArgs {        // 32 B
    emit: [u32; 3], _p0: u32,            // VkDispatchIndirectCommand @ 0
    sim:  [u32; 3], _p1: u32,            // VkDispatchIndirectCommand @ 16
}

/// TWO commands. first_instance is 0 in BOTH — F5b forbids anything else.
/// Each `instance_count` is ALSO its class's render counter (M1/M2): the
/// InterlockedAdd yields the position, the final value is the count.
#[repr(C, align(16))]
pub struct ParticleDrawArgs {            // 64 B
    additive: VkDrawIndexedIndirectCommand,  // @ 0    instance_count @ 4
    alpha:    VkDrawIndexedIndirectCommand,  // @ 24   instance_count @ 28  (P2; zeroed at P0)
    _pad: [u32; 6],
}
pub const PARTICLE_ADDITIVE_INSTANCE_COUNT_OFFSET: u32 = /* offset_of! chain */;   // == 4
pub const PARTICLE_ALPHA_INSTANCE_COUNT_OFFSET:    u32 = /* offset_of! chain */;   // == 28
// ^ both are GENERATOR INPUTS; the shaders' word indices are emitted, never typed.

#[repr(C, align(16))]
pub struct EmitRequestGpu {              // 64 B
    origin:  [f32; 3], effect_index: u32,
    basis_x: [f32; 3], spawn_count:  u32,
    basis_y: [f32; 3], first_spawn:  u32,   // CPU prefix sum — orders LANES only (D8)
    basis_z: [f32; 3], rng_seed:     u32,   // per-emitter CONSTANT; the frame enters via push
}

#[repr(C, align(16))]
pub struct EffectParamsGpu {             // 128 B
    gravity: [f32; 3], damping: f32,        // exp2(-drag * timestep), HOST-computed (D6)
    rot_mul_cos: f32,  rot_mul_sin: f32,    // (cos ω·timestep, sin ω·timestep) — F32, NOT snorm16:
    _r0: [u32; 2],                          // K1 — a quantized multiplier's magnitude error compounds
    color_keys:  [u32; 4],
    color_times: [u32; 2], size_keys: [u32; 2],
    lifetime_min: f32, lifetime_max: f32, speed_min: f32, speed_max: f32,
    size_base: f32, cone_cos: f32, _r1: f32, _r2: f32,
    tex_index: u32, blend_class: u32, flags: u32, collision_radius: f32,
    restitution: f32, friction: f32, emitter_shape: u32, _r3: u32,
}
```

### VRAM (`CAP`-indexed; "typical alive" is independent)

| `CAP` | `p_particle` 48 B | `p_render` 32 B | lists 12 B | **Total** | Typical alive |
|---|---|---|---|---|---|
| 65 536 | 3.1 MB | 2.1 MB | 0.8 MB | **6.0 MB** | 10k–40k |
| 262 144 *(default)* | 12.6 MB | 8.4 MB | 3.1 MB | **24.1 MB** | 40k–160k |
| 1 048 576 | 50.3 MB | 33.6 MB | 12.6 MB | **96.5 MB** | 150k–700k |

Fixed overhead: counters 64 B + dispatch args 32 B + draw args 64 B + `quad_ib` 12 B + `p_emit_req_device` 16 KB + `p_effects_device` 32 KB + staging 2×48 KB ≈ **145 KB**. Rung P2b adds 8 B/particle to `p_render`.

### CPU-side ECS

```rust
#[repr(u32)] #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ParticleMode { #[default] Off, GpuUnlit }        // P3 adds GpuLit
#[derive(Resource, Clone, Copy, Debug)]
pub struct ParticleConfig { pub mode: ParticleMode, pub capacity: u32 }
impl ParticleConfig {
    #[inline] pub const fn enabled(&self) -> bool { !matches!(self.mode, ParticleMode::Off) }
}

/// The subsystem's OWN clock (D6/M4). Does NOT read FixedTime and does NOT
/// cause a CoreSchedule::Fixed schedule to exist (F28b).
#[derive(Resource, Clone, Copy, Debug)]
pub struct ParticleClock { timestep: f32, accumulator: f32, steps: u32, dropped_steps: u32 }
impl ParticleClock {
    pub fn from_hz(hz: f32) -> Self;                 // default 64.0
    #[inline] pub const fn timestep(&self) -> f32;
    #[inline] pub const fn steps(&self) -> u32;      // THE one number this frame (M3)
    #[inline] pub const fn dropped_steps(&self) -> u32;
    #[inline] pub fn overstep_fraction(&self) -> f32;   // [0,1) — rung P2b (M6)
}

#[repr(C)] #[derive(Component, Clone, Copy, Debug)]
#[require(Transform, GlobalTransform, ParticleEffectHandle)]
pub struct ParticleEmitter {             // 16 B (D16: hot/cold mix is a decided exception)
    pub rate: f32, pub accumulator: f32, pub burst: u32, pub speed_scale: f32,
}

#[derive(Component, Clone, Copy, Debug)]
#[component(storage = "bitset")]
pub struct EmitterActive;                // O(1) toggle; Added/Changed compile-rejected

#[repr(transparent)] #[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[component(on_insert = effect_on_insert, on_replace = effect_on_replace)]
pub struct ParticleEffectHandle(pub u32);

// ScratchColumn-backed staging — Principle 0. Capacity is pool_reserve_rows
// (register_asset_layout::<T>(None) + pool_reserve_rows, mesh_draw.rs:429-465);
// MAX_EMITTERS is the WRITER's release-present clamp + debug_assert (D15).
#[derive(Resource)]
pub struct ParticleEmitScratch { requests: ScratchColumn<EmitRequestGpu>, total_spawn: u32 }
#[derive(Resource)]
pub struct ParticleEffectScratch { rows: ScratchColumn<EffectParamsGpu>, seen_gen: u64 }

pub const MAX_EMITTERS: usize = 256;
pub const MAX_EFFECTS:  usize = 256;
pub const PARTICLE_SUBSTEP_CEILING: u32 = 64;   // host-applied clamp (M3); shader min = hang guard only
```

### Host-side

```rust
pub(crate) struct ParticleGpuBundle {                  // built ONLY when enabled() at boot
    particle: BoundBuffer, render: BoundBuffer, dead: BoundBuffer,
    alive: [BoundBuffer; 2],                           // physical; roles ride sets[parity]
    counters: BoundBuffer, dispatch_args: BoundBuffer, draw_args: BoundBuffer,
    quad_ib: BoundBuffer,                              // 12 B, boot-uploaded, never rewritten
    emit_req_device: BoundBuffer, emit_req_staging: [BoundBuffer; FRAMES_IN_FLIGHT],
    effects_device:  BoundBuffer, effects_staging:  [BoundBuffer; FRAMES_IN_FLIGHT],
    kickoff: ComputePipeline, emit: ComputePipeline, sim: ComputePipeline,
    draw: GraphicsPipeline,                            // compare op frozen at boot (D7)
    sets: [VulkanBindGroup; 2],                        // sets[0]: alive[0]=read, alive[1]=write
}
```

---

## Public API

```rust
// boyko_render
pub struct ParticlePlugin;                    // D17: touches nothing outside the subsystem
pub enum ParticleMode { Off, GpuUnlit }
pub struct ParticleConfig { pub mode: ParticleMode, pub capacity: u32 }
impl ParticleConfig { pub const fn enabled(&self) -> bool; }

pub struct ParticleClock { /* … */ }
impl ParticleClock {
    pub fn from_hz(hz: f32) -> Self;
    pub const fn timestep(&self) -> f32;
    pub const fn steps(&self) -> u32;
    pub const fn dropped_steps(&self) -> u32;
    pub fn overstep_fraction(&self) -> f32;
}

pub struct ParticleEmitter { pub rate: f32, pub accumulator: f32, pub burst: u32, pub speed_scale: f32 }
pub struct EmitterActive;
pub struct ParticleEffectHandle(pub u32);

#[repr(C)] pub struct ParticleEffect { /* POD authoring struct → EffectParamsGpu */ }
pub trait ParticleEffectsExt {
    fn spark(&mut self) -> Handle<ParticleEffect>;
    fn smoke(&mut self) -> Handle<ParticleEffect>;
}
impl ParticleEffectsExt for Assets<ParticleEffect> { .. }

pub const MAX_EMITTERS: usize;
pub const MAX_EFFECTS:  usize;
pub const PARTICLE_SUBSTEP_CEILING: u32;

// boyko_render::upload — token-fenced, per-FIF slot
pub unsafe fn upload_particle_emit_requests(token: &FrameWriteToken, slot: &BoundBuffer, bytes: &[u8]);
pub unsafe fn upload_particle_effects(token: &FrameWriteToken, slot: &BoundBuffer, bytes: &[u8]);

// boyko_app::particle_gate
pub fn particle_effects_upload_due<const N: usize>(uploaded: &mut [u64; N], slot: usize, gen: u64) -> bool;

// boyko_rhi
impl BlendState { pub const ADDITIVE: BlendState; }
```

No internal type leaks: no `BoundBuffer`, `ResId`, `ScratchColumn`, `dyn`, or `Vec` in any public signature.

---

## Algorithms for critical paths

*(All derive from §Counter and list ownership.)*

### A1 — `particle_tick_emitters` (CPU, `CoreSchedule::Main`, once per frame)

```
clock.accumulator += Time::delta_secs();                   // F27: clamped, scaled, pause-aware
raw   = floor(clock.accumulator / clock.timestep)
steps = min(raw, PARTICLE_SUBSTEP_CEILING)                 // THE clamp, once, here (M3)
clock.dropped_steps += raw - steps
clock.accumulator   -= raw as f32 * clock.timestep         // drop the excess, do not carry it
clock.steps = steps
dt = steps as f32 * clock.timestep                         // the SAME number the shader gets
for (emitter, xform, handle) in Query<.., Enabled<EmitterActive>>:   // sequential
    acc += rate * dt;  n = floor(acc) + burst;  acc -= floor(acc);  burst = 0
    write EmitRequestGpu { origin, basis from xform, effect_index, spawn_count: n,
                           first_spawn: running };  running += n
total_spawn = running
```

- **Complexity** O(emitters), ≤256. **Sequential, not `par_iter`**: 256 rows × ~40 ns ≈ 10 µs; a fork/join costs more, and sequential lets each emitter write its own `ScratchColumn` row with a plain counter — **no atomic**.
- **One `ScratchColumn` fill, one `total_spawn` per rendered frame.**
- **Cache** two linear column streams. **Branching** one predicted `Enabled` bit test, one `floor`. **Change detection** F26 — `&mut T` stamps nothing.
- **Upload gate** `total_spawn > 0` — a writer-side signal, no hash. When zero, the emit pass is **not declared**.

### A2 — `particle_kickoff` (GPU, 1 thread)

Exactly D3's kickoff block. O(1), ~24 scalar ops over three cache lines, two `min`/`max`, branchless. It reads `alive_count_next` (written by the previous frame's sim) from `p_counters`, whose `seeded_writer` seed carries the availability. It **never reads `p_draw_args`**.

### A3 — `particle_emit` (GPU, `DispatchIndirect`, 256 thr) — **zero atomics**

`groupshared` cooperative load of the ≤256-entry `first_spawn` prefix (one coalesced 1 KB load, one `GroupMemoryBarrierWithGroupSync`) → 8-step branchless binary search → `slot = p_dead[dead_base + gid]`, `pos = emit_append_base + gid` → `seed = pcg32(rng_seed ^ gid ^ frame)` → write `p_particle[slot]` (48 B) and `p_alive_read[pos] = slot`.

O(`real_emit_count`); prefix search LDS-resident; the record write is a scatter of one 48 B line; the list write is sequential in `gid`.

### A4 — `particle_sim` (GPU, `DispatchIndirect`, 256 thr) — the hot loop

Exactly D3's sim block, with the wave-level mechanism of §Counter and list ownership. Guard `i >= alive_count_cur` — **the same field kickoff sized the dispatch from**.

- **Complexity** O(`alive_count_cur`).
- **Cache** `p_particle` read/write is a gather of one fully-consumed 64 B line per particle (R2); `p_render` and `p_alive_write` are sequential in their indices. `Effects[]` (32 KB) is effectively constant-resident.
- **Atomics** D5's per-wave budget.
- **Branching** one liveness test (wave-coherent in the steady state); the substep loop is **wave-uniform** (push constant); one rare collision branch under `-D SDF_COLLIDE`.

### A5 — `particle_draw` (GPU, `DrawIndexedIndirect`)

VS: `r = p_render[pc.index_base + pc.index_step * SV_InstanceID]` — at P0 `(0, +1)`, the identity: **sequential, no indirection**. Corner offset from `cam_right`/`cam_up` (UBO fields) scaled by `r.size` and rotated by the stored `(cos, sin)` pair — **no trig, no renormalization** (M7). FS (P0): `color * tex[r.tex_index].Sample(uv)`, additive.

O(alive) instances × 4 vertices; sequential; the 4 vertices of an instance share a wave; zero index bandwidth. **R9 closed**: `instanceCount` is the live count, never `CAP`.

---

## Multithreading model

### CPU

| Data | Sharing | Mechanism |
|---|---|---|
| `ParticleEmitter` rows | one system, `&mut` | conflict graph; sequential by design |
| `ParticleClock` | one writer (A1), one reader (the record site) | `ResMut` then a dispatcher-side read at `running == 0` |
| `ParticleEmitScratch` / `ParticleEffectScratch` | one writer, one reader (host upload) | `ResMut` serialises writers; the host reads on the dispatcher after `Schedule::run` returns with zero workers in flight |
| `Assets<ParticleEffect>` | `NonSendResource` | reached only via `DispatcherToken::nonsend_resource_mut` at `running == 0` |
| `ParticleGpuBundle` | host-owned, never in the world | not `Send` |

**No `Mutex`, `RwLock`, `Rc`, `RefCell`, or CPU atomics anywhere in this subsystem.**

### Host → GPU

F19: a borrowed `FrameWriteToken`, a per-FIF staging ring, a writer-side gate (`total_spawn > 0`; `Assets::dirty_gen()`). **Parity:** the host's monotonic frame counter selects `sets[parity]` pre-submit; there is no GPU-side parity field.

### Cross-frame seed table *(access column derived from §Counter and list ownership; M5 adds the physical-buffer column)*

> **This table's access column is the single source the declarators are written from.** A `buffer_access` call that does not appear here is a defect; an access here that the declarator omits deletes a derived barrier (the F7d/N1 class — measured-invisible, `robustBufferAccess` OFF).

`transition` prefers `flush` over `visible` (F7b), so a writer seed on a buffer whose sibling frame ended on a read leaves that read unordered. `ResSync` has no separate "written then drained by a read" constructor because **`seeded_readers` is that state**: a read clears the pending flush and accumulates visibility (`sync.rs:406-413`). Layout is irrelevant for buffers.

| # | ResId | **Physical buffer this row tracks** | **Accesses this frame, in order** | Terminal **this frame** | Seed *(describes the SIBLING frame's terminal on the same physical buffer)* | **First access next frame, on the ResId bound to this physical buffer** ⇒ derived |
|---|---|---|---|---|---|---|
| 1 | `p_particle` | `particle` (single) | emit `C/W` → sim `C/RW` | C write | `seeded_writer(COMPUTE, SHADER_WRITE)` | emit `C/W` ⇒ **WAW** |
| 2 | `p_render` | `render` (single) | sim `C/W` → draw `VERTEX_SHADER/R` | **VS read** | `seeded_readers(VERTEX_SHADER, SHADER_READ)` | sim `C/W` ⇒ **WAR**, src `(VERTEX_SHADER, 0)` |
| 3 | `p_dead` | `dead` (single) | kickoff `C/RW` → emit `C/R` → sim `C/RW` | C write | `seeded_writer(COMPUTE, SHADER_WRITE)` | kickoff `C/RW` ⇒ **WAW/RAW** |
| 4 | `p_alive_read` | `alive[p]` at frame N — **the buffer that was `p_alive_write` at frame N−1** | **emit `C/W`** → **sim `C/R`** | C read | `seeded_writer(COMPUTE, SHADER_WRITE)` — the sibling's sim write | **emit's `C/W` ⇒ WAW** at the **emit** pass *(not the sim's read — M5)* |
| 5 | `p_alive_write` | `alive[p^1]` at frame N — **the buffer that was `p_alive_read` at frame N−1** | **sim `C/W`** *(only)* | C write | `seeded_readers(COMPUTE, SHADER_READ)` — the sibling's sim read | **sim's `C/W` ⇒ WAR**, src `(COMPUTE, 0)`, at the **sim** pass |
| 6 | `p_counters` | `counters` (single) | kickoff `C/RW` → sim `C/RW` | C write | `seeded_writer(COMPUTE, SHADER_WRITE)` | kickoff `C/RW` ⇒ **RAW**, carrying the sim's `alive_count_next` availability |
| 7 | `p_dispatch_args` | `dispatch_args` (single) | kickoff `C/W` → emit `DI/ICR` → sim `DI/ICR` | DI read | `seeded_readers(DRAW_INDIRECT, INDIRECT_COMMAND_READ)` | kickoff `C/W` ⇒ **WAR** |
| 8 | `p_draw_args` | `draw_args` (single) | kickoff `C/W` → sim **`C/RW`** *(the returning `InterlockedAdd` is a read-modify-write — the `light_index_alloc` precedent, `graph_bridge.rs:3837-3844`; K2)* → draw `DI/ICR` *(P2: two draws, both `DI/ICR`)* | DI read | `seeded_readers(DRAW_INDIRECT, INDIRECT_COMMAND_READ)` | kickoff `C/W` ⇒ **WAR**. *Kickoff never reads it.* |
| 9 | `p_emit_req_device` | single | *(upload `T/TW`)* → emit `C/R` | C read | `seeded_readers(COMPUTE, SHADER_READ)` | idle: emit `C/R` ⇒ **free**; upload frame: `T/TW` ⇒ **WAR** |
| 10 | `p_effects_device` | single | *(upload `T/TW`)* → emit `C/R` → sim `C/R` | C read | `seeded_readers(COMPUTE, SHADER_READ)` | as row 9 |
| — | `quad_ib` | — | **not a graph resource** | — | — | boot-written once under its own `TRANSFER_WRITE → VERTEX_INPUT/INDEX_READ` barrier; read-only thereafter |

**Rows 4/5 read (M5).** Each row tracks **one physical buffer across two frames**. Row 4's ResId is bound, this frame, to the buffer the *previous* frame drove as `p_alive_write` — hence a writer seed, and hence the barrier appears at **emit**, the ResId's first access, not at the sim's read. Gate #3 must look for it there.

**Why the sim's read of the un-overwritten prefix is visible.** Emit rewrites only `[emit_append_base, A)`; the sim reads `[0, A)`, so `[0, emit_append_base)` was written by the *sibling* frame. The seed barrier at emit carries `srcStage/srcAccess = (COMPUTE, SHADER_WRITE)`, whose first synchronisation scope includes the sibling's write; the intra-frame emit→sim barrier then carries `(COMPUTE, SHADER_WRITE) → (COMPUTE, SHADER_READ)`, whose first scope includes everything ordered before it on the queue. The availability chain composes, so the prefix is visible to the sim's read.

### Conditional-pass proof

| # | `total_spawn > 0` | effects dirty | `p_emit_req_device` accesses | `p_effects_device` accesses | Terminal matches the seed? |
|---|---|---|---|---|---|
| 1 | yes | yes | `T/TW` → emit `C/R` | `T/TW` → emit `C/R` → sim `C/R` | ✓ C/R · ✓ C/R |
| 2 | yes | no | emit `C/R` (free read) | emit `C/R` → sim `C/R` | ✓ · ✓ |
| 3 | no | yes | **none** — untouched; state persists from the seed (F7c) | `T/TW` → sim `C/R` | ✓ (vacuous) · ✓ |
| 4 | no | no | **none** | sim `C/R` (free) | ✓ (vacuous) · ✓ |

The load-bearing detail: `particle_upload_emit` **and** `particle_emit` are gated by the **same** predicate, so "written but unread this frame" — which would make row 9's reader seed wrong — cannot be constructed. `p_effects_device` always has at least one reader (the sim needs effect params every frame).

### Intra-frame hazards

| Hazard | Resolution |
|---|---|
| emit reads `p_dead` / sim pushes `p_dead` | **Impossible concurrently** — different passes, derived barrier between |
| emit writes `p_alive_read` / sim reads it | derived `COMPUTE→COMPUTE` RAW (row 4's access column) |
| two waves append to `p_alive_write` | one `InterlockedAdd` on `alive_count_next` reserves a disjoint contiguous range; `WavePrefixCountBits` assigns disjoint lanes |
| two waves allocate render positions | per-class `InterlockedAdd`; reservations contiguous and disjoint; final value = Σ counts, order-independent |
| kickoff writes `p_dispatch_args` / command processor fetches | derived `C/SHADER_WRITE → DI/ICR`; the sim's second read is barrier-free |
| `particle_draw` blends into `lit` while the resolve still writes it | derived `GENERAL → COLOR_ATTACHMENT_OPTIMAL` |
| depth | per-path, D7's table |
| cross-lane RMW of one attribute | `p_particle[slot]` is touched by exactly one lane |

---

## Integration

### New modules

`boyko_render/src/{particle_config,particle_clock,particle,particle_effect,particle_system,particle_plugin}.rs` · `boyko_app/src/particle_gate.rs` · `boyko_app/src/gpu_scene/particle.rs` · `boyko_rhi_vulkan/src/present/passes/particles.rs` · `boyko_shaderdsl/src/particle.rs` · `boyko_shaderdsl/src/bin/emit_particles.rs` · `boyko_rhi_vulkan/shaders/particle_{kickoff,emit,sim}.comp.hlsl` + `particle_draw.{vs,fs}.hlsl` · `boyko_rhi_vulkan/tests/{particle_edsl_sync,particle_barrier_stream}.rs` · `boyko_render/tests/particle_containment.rs` *(M4)*

### Changes to existing code

| File | Change | Risk |
|---|---|---|
| `boyko_rhi/src/enums.rs` | `BlendState::ADDITIVE` | none — pure addition |
| `rhi_impl/device.rs` | `create_graphics_pipeline_particle(desc, set1, depth_compare)` | none — already parameterised (F3b) |
| `ffi.rs` | add `VK_PIPELINE_STAGE_VERTEX_INPUT_BIT` / `VK_ACCESS_INDEX_READ_BIT` if absent | none — two consts |
| `compute.rs` | 5 × `embed_spirv!` + accessors + `PARTICLE_*_PUSH_BYTES` **not** added to the shared-range `max` | none — F12's 112 B does not move |
| `graph_bridge.rs` | +10 conditional-tail buffer ResIds + 5 `Option<PassId>` passes in all three declarators | **moderate** — `sv0_pass` template + declare/record parity `debug_assert` |
| `graph_bridge.rs:3804-3809` | the `vb_cull_uniform`-is-LAST assert moves | **named**; updated in the same edit, pinned by a test |
| `frame_driver.rs:189` | `with_capacity(16,16,64)` → `(48, 32, 192)` | none — first-frame allocation only |
| `scene_types.rs` | `Option<ParticleActivation<'a>>` + `path_has_particles()` | none |
| `gpu_scene/mod.rs` | `particle: Option<ParticleGpuBundle>` | none when `None` |
| `gpu_scene/mod.rs:7491` | **`destroy` particle arm** — 8 device buffers + `quad_ib` + 2×FIF staging ×2 + 3 compute + 1 graphics pipeline + 2 pipeline layouts + 1 set layout + the pool | **named**; leak at shutdown otherwise |
| `runner.rs` | boot: `if config.enabled() { build_particle_bundle(..) }`; frame: two gated uploads + `sets[parity]` | none when disarmed |
| `boyko_render/src/upload.rs` | two `unsafe fn upload_particle_*` | none |
| `docs/SHADER-VARIANT-MANIFEST.md` | rows for `SDF_COLLIDE`, `DEPTH_LINEAR`, `SOFT`, `LIT_PERPIXEL`, `MOTION`, `PARTICLE_INTERP` | doc |
| **`App` / `Time` / `FixedTime` / `CoreSchedule::Fixed`** | **NONE (D17/M4)** | — |

---

## Rung ladder

Unconditional gate on every rung: all 35 `goldens/PINS.toml` hashes unchanged; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace --all-targets --no-fail-fast`; Miri where new `unsafe`; **`*_spv_sync` run locally with dxc present and the result reported** (F15); author-only commit + push.

### Rung E — eDSL prerequisites *(unchanged; M7 adds nothing)*

| Rung | Missing nodes | Unlocks | Required by | Size |
|---|---|---|---|---|
| **E1** | `ushl`, `ushr`, `uxor`, `uand`, `uor` on `Self::Uint` (`or`@:302 / `and2`@:329 are **Mask** combinators) | `particle_rng` (PCG32) | **P0** | S |
| **E2** | `asuint`, `asfloat`, `f16tof32`, `f32tof16` | packed-attribute decode/encode | **P0** | S |
| **E3** | `dot(Vec3f, Vec3f) → Scalar` | `particle_sdf_response` | **P1** | XS |
| ~~E4~~ | ~~`exp2`~~ | — | **DELETED** — D6's constant `timestep` lets the host precompute `damping` | — |
| ~~E5~~ | ~~`sin`/`cos`~~ | — | **NOT ADDED** — trig-free rotation and cone sampling | — |
| ~~E6~~ | ~~`rsqrt`~~ | — | **NOT NEEDED (M7)** — see below | — |

**Shader leaves and their backend family.** F13c: `sin`/`cos` are on `InterpBackend` and `Cf::Scalar: FieldScalar`, so a `Cf` leaf cannot call them. Rev 2 removed the need; Rev 3 also removes the renormalization that would have needed a divide.

| Leaf | Family | Trig? | Divide? | Note |
|---|---|---|---|---|
| `particle_integrate` | `Cf` | no | no | `damping` is a host constant (D6) |
| `particle_rng` (PCG32) | `Cf` | no | no | integer ⇒ **bit-exact by construction** |
| `particle_spawn_state` | `Cf` | **no** | no | cone direction via the concentric-disc form (`sqrt`, `FieldScalar:101`) |
| `particle_curve_eval` | `Cf` | no | no | 4-key ramps over unpacked f16 |
| `particle_billboard_corner` | `Cf` | **no** | no | rotation is a stored `(cos, sin)` pair; the VS multiplies |
| `particle_rot_advance` | `Cf` | **no** | **no (M7)** | complex multiply by the host-precomputed `(cos ω·timestep, sin ω·timestep)`; **no renormalization** |
| `particle_sdf_response` (P1) | `Cf` | no | no | needs E3's `dot` |

**M7, resolved two ways.** *Reachability:* `FieldScalar` carries `div` (`scalar.rs:77`) **and** `sqrt` (`:101`), so `1/√(c²+s²)` is expressible today with no new node — Rev 2's "needs `rsqrt`" was loose wording. *Why it is not used:* a division inside a leaf drags `OpFDiv`'s 2.5 ULP into that leaf's oracle, and the house rule is that **division is never part of a bit-exact contract**. Rev 4 (K1): the multiplier is an **f32 pair** in `EffectParamsGpu` — a snorm16-quantized multiplier's magnitude error is a per-effect constant that compounds geometrically ((1+δ)⁶⁴⁰ ≈ ±1 % at δ ~ 2×10⁻⁵); at f32 it is ≤ 1 ULP and (1+δ)⁶⁴⁰ − 1 ≈ 4×10⁻⁵. The per-step snorm16 re-quantization of the STATE is unbiased and random-walks to ≈ **7.6×10⁻⁴** over a 10 s life. The leaf stays pure mul/add and bit-exact. **Every leaf is authored against `Cf`; none needs `InterpBackend`; none contains a divide.**

### P0 — one emitter, unlit additive billboards, GPU sim, all four paths, default-off — **size L**

**Lands.** Components + `ParticleConfig` + `ParticleClock` + `ParticlePlugin`; `Assets<ParticleEffect>` + the effect table; two `ScratchColumn`s; `ParticleGpuBundle`; `emit_particles.rs` + 5 shaders; kickoff/emit/sim + the indirect draw; `BlendState::ADDITIVE`; `create_graphics_pipeline_particle`; conditional-tail declaration in all three declarators; `record_particles`; two gated uploads; the destroy arm; the `with_capacity` bump; the `vb_cull_uniform` assert move; the two-slot `ParticleDrawArgs` (slot 1 zeroed, its pass undeclared).

**Gate.**
1. **Disarmed byte-identity, three levels:** (a) five *new* `.spv`, **no existing `.spv` byte changes**; (b) **derived-barrier-stream baselines per declarator** with `mode == Off`; (c) all 35 image pins.
2. **Armed barrier assertions, per path:** `lit: GENERAL → COLOR_ATTACHMENT_OPTIMAL` on every path; **plus** `depth: SHADER_READ_ONLY_OPTIMAL → DEPTH_ATTACHMENT_OPTIMAL` on Deferred; **plus** an availability-only depth barrier on Forward/VB; **and exactly zero depth barriers on ForwardPlus**. Four separate assertions.
3. **Seed-table test:** for each of the ten resources, the derived first-access barrier's `(src_stage, src_access)` equals the table's row — **and is looked for at the pass the table's last column names** (M5: rows 4/5's barrier is at **emit** and at the **sim** respectively, on different physical buffers).
4. **Access-column test:** the set of `buffer_access` calls each declarator emits **equals** the seed table's access column, per pass. This catches a missing intra-frame declaration, which gate #3 cannot see.
5. **Idle-vs-active stream pinning:** `p_dispatch_args`' derived stream is pinned in **both** variants — `total_spawn > 0` (barrier at emit, free read at sim) and `total_spawn == 0` (emit undeclared, barrier derives at sim) — per declarator.
6. **Declare/record parity** `debug_assert` on `path_has_particles()`.
7. **Pool-partition readback:** at each of the four boundaries, `alive + dead == CAP`; every slot appears exactly once; **the B1 in-flight window is asserted explicitly**; **and `additive.instanceCount + alpha.instanceCount == alive_count_next`** (M2 — the assertion that catches an alpha leak; at P0 the alpha term is 0).
8. **Offset pins:** `PARTICLE_ADDITIVE_INSTANCE_COUNT_OFFSET == 4`, `PARTICLE_ALPHA_INSTANCE_COUNT_OFFSET == 28`, **and** the generated shaders contain the word indices derived from those consts. `first_instance == 0` asserted host-side in both slots (F5b).
9. **Frame-0 test:** boot → one frame → `alive_count_cur == real_emit_count`, no leak, and `p_counters` contains **no** parity field (compile-time absence).
10. **Clock test (M3):** a synthetic frame at `relative_speed = 8.0` on a 250 ms delta produces `raw_steps == 129`, `steps == 64`, `dropped_steps += 65`, the accumulator drained to its fractional remainder, and **the sim's push constant equals 64** — one value asserted at both consumers.
11. **Containment test (M4/D17):** build two apps differing only by `add_plugins(ParticlePlugin)`; assert the resolved `EventUpdatePolicy` is identical, that no Fixed schedule exists in either, and that the set of registered schedule labels is identical.
12. **Per-path depth test:** boot-resolved compare op is `LESS` under Deferred and `GREATER` elsewhere, plus an owner-eval screenshot per path showing particles occluded by opaque geometry (a wrong compare op inverts occlusion and no automated gate would see it).
13. **Determinism harness:** (a) **single-step exact** — host-authored `p_dead`/`p_alive_read`, one emit + one sim, per-slot comparison against the `EvalCf` oracle; (b) **multi-step multiset** — N steps, compare the *multiset* of particle states (dead-stack pop order is nondeterministic from frame 2). Frame count pinned in the test name.
14. **`particle_edsl_sync`:** per-leaf `*_matches_edsl_emit`; `particle_*_spv_byte_identical` × 5; `LocalSize == 256`; an `OpAtomicIAdd` census on `particle_sim` (exactly the wave-leader sites, **and no `OpAtomicUMax`** — M1 deleted the mirror) **and zero atomics in `particle_emit`**; **no `OpFDiv` in `particle_rot_advance`'s generated span** (M7).
15. **OOB test (D15/R8):** `MAX_EMITTERS + 1` / `MAX_EFFECTS + 1` — writes stay in bounds, `clamped_spawns` counts the shortfall exactly.
16. **New golden `particle_additive`** — one emitter, fixed seed, fixed `timestep`, frame 30, **spatially separated particles (no inter-particle overdraw)** so blend order is irrelevant and the pin is bit-reproducible. The constraint is stated in the pin's own doc. Owner-blessed.
17. **Measurements reported:** kickoff/emit/sim/draw µs at 10k/100k/1M against R12; **A/B render record vs direct gather** with the corrected break-even derivation; **A/B wave-aggregated vs naive atomics** at 1M; **A/B 4-vertex instanced vs `vid/6`**; the 64-substep worst-case frame cost.

**Limitations recorded:** LDR additive clipping/quantization (D7/F2); **fixed-rate stepping without interpolation** (M6 — above 64 Hz most frames step zero times); TAA ghosting until P3.

### P1 — SDF collision — **size M** *(prerequisite: E3)*

`-D SDF_COLLIDE`; D9's Lipschitz skip and response leaf.
**Gate:** base `particle_sim.comp.spv` **byte-unperturbed** with the define undefined; `EvalCf` oracle for the response; golden `particle_sdf_collide`; measured skip rate and armed/disarmed delta; **a tunneling test at `v_max · timestep` against a thin collider** (R6); a manifest row.

### P2 — Alpha blending, sorting, soft particles — **size L**

One FFX-shaped radix pass; **D10's partition** (shared list counter, per-class render counters, `first_instance = 0` in both slots, push-constant index transform); `-D SOFT` FS sampling depth; `SortMode` incl. `Wboit`.
**New plumbing named:** `VK_IMAGE_LAYOUT_DEPTH_STENCIL_READ_ONLY_OPTIMAL` in `ffi.rs`; a `BindGroupEntry::SampledDepthAtReadOnly` variant (the `SampledImageAtGeneral` precedent); the depth access gains `FRAGMENT_SHADER|SHADER_READ` and the read-only layout; `-D DEPTH_LINEAR` FS variant for Deferred's decode (interface-identical, same layout object — the `TERMINATOR_WRAP` precedent).
**Hard rule (R10):** `SortMode != None` ⇒ motion vectors disabled.
**Gate:** sort monotonicity readback; reverse-Z **and** linear soft-fade oracles; `first_instance == 0` on both slots; the alpha reverse index transform verified by readback; **`additive.instanceCount + alpha.instanceCount == alive_count_next` with both terms non-zero** (the M2 assertion, now exercised); `SortMode::None` byte-identical to P1.

### P2b — Render-time interpolation — **size S**, default OFF *(M6)*

`ParticleRender` gains a packed `vel` lane (32 B → **40 B**, **+25 % of the draw's read traffic**) — **as a COMPILE-TIME variant (K3): `-D PARTICLE_INTERP` on the sim and VS, plus the host `cfg` that sizes the record and the buffer 40 B**, with its own `docs/SHADER-VARIANT-MANIFEST.md` row. Never a runtime flag over an always-40 B record — that would pay the +25 % draw-read while off, the F24 dark-tax class this plan cites everywhere else. The VS advances `pos + vel · (pc.overstep · timestep)` — one fused multiply-add — with `overstep = ParticleClock::overstep_fraction()`. Rides the engine's existing interpolation seam (`FixedTime::overstep_fraction()` / RDG Pillar B).
**Gate:** off ⇒ the 32 B record and the P2 goldens byte-identical; on ⇒ a new pin plus the measured draw-traffic delta; a 200 fps / 64 Hz smoothness capture as owner-eval (no automated gate can see stutter).

### P3 — Lit particles + motion vectors — **size M**

Per-particle froxel lookup in the sim writing the render record's colour lane; `-D LIT_PERPIXEL`; `-D MOTION` FS (`pos` and `pos − vel·timestep` are both exact) **only when `SortMode == None`**.
**Gate:** `taa_*` pins byte-identical with particles off; new `particle_lit` pin; per-particle vs per-fragment cost measured; a test that `SortMode != None && MOTION` is unconstructible.

### P4 — Trails / ribbons / mesh particles — **size L**

Ribbon = per-particle history ring expanded to a strip; mesh particles = the same pool driving `DrawIndexedIndirect` against a `MeshGeometryTable` mesh. Ribbons are the first consumer to hold a slot across frames — fine under D3, re-checked against any future compaction.

### P5 — Measurement-gated micro-optimisations — **size S**, may be dropped

`groupshared` atomic aggregation if a profile shows the wave form is hot; mesh-shader billboard expansion; Turitzin-style compute rasterization as a **spike only**. **Explicitly excluded:** fusing emit+sim.

---

## Perf analysis

### Per-frame GPU traffic at `A` alive, `E` emitted

| Pass | Bytes/particle | Pattern | 100k / 20k | 1M / 50k |
|---|---|---|---|---|
| kickoff | — | 3 lines | 192 B | 192 B |
| emit | 56 (48 W + 4 R + 4 W) | scatter + sequential | 1.1 MB | 2.8 MB |
| sim | 136 (48 R + 48 W + 32 W + 8) | 48 B gathered as one line; rest sequential | 13.6 MB | 136 MB |
| draw VS | 32 (×4 verts, L1-absorbed) | **sequential** | 3.2 MB | 32 MB |
| **Total** | **≈168 B/particle** | | **17.9 MB** | **171 MB** |
| **@ 60 Hz** | | | **1.07 GB/s** | **10.3 GB/s** |

0.1–0.3 % of GPU bandwidth at 100k, 1–2.5 % at 1M. Raster and overdraw dominate at 1M. Cross-check against R12: our 80 B of state vs Nutshell's 64 B ⇒ ~1.25× their traffic ⇒ predicted sim ≈ 0.06 ms at 100k, just under the ≤80 µs budget. Rung P2b adds 8 B/particle to the draw read (+25 % of that term).

### Dispatch sizes

`[numthreads(256,1,1)]` for emit and sim (R4), `[numthreads(1,1,1)]` for kickoff.

| Pass | Groups @100k | Groups @1M |
|---|---|---|
| kickoff | 1 | 1 |
| emit (20k / 50k) | 79 | 196 |
| sim | 469 | 4 096 |

### Overhead outside the particle passes

| Path | `lit` transition | Depth | Intra-block barriers | **Total** |
|---|---|---|---|---|
| Deferred | 1 (`GENERAL → COLOR_ATTACHMENT_OPTIMAL`) | **1 (layout: `SRO → DEPTH_ATTACHMENT_OPTIMAL`)** | 2 | **≈5–10 µs** |
| Forward | 1 | **1 (availability, or layout under an SDF leg)** | 2 | ≈4–9 µs |
| ForwardPlus | 0 (already an attachment) | **0 — free** | 2 | ≈2–4 µs |
| VisibilityBuffer | 1 | **1 (availability, or layout)** | 2 | ≈4–9 µs |

**CPU overhead outside the subsystem: zero (D17/M4).** No schedule is created, no event policy resolved differently, no clock shared.

**Declaration-order latency win, free:** the kickoff/emit/sim block is declared early and the draw late; declaration order is execution order (F8) and no barrier separates the sim from the opaque work that follows, so ≈80–700 µs of particle compute overlaps the opaque pass at zero cost.

---

## Metrics and validation

### Mandatory unit tests

`particle_config_default_is_off_the_zero_gate`; `default_mode_is_off` (the **second** route into `Off` — the hand-written `Default` vs the `#[default]` attribute; neither implies the other); `enabled_agrees_with_the_discriminant_on_every_variant`; `every_variant_states_its_own_answer_without_a_wildcard` (a new variant must fail to **compile**) · `emitter_accumulator_carries_fractional_spawns_exactly`; `burst_is_consumed_once` · `prefix_sum_is_monotone_and_totals_match`; `first_spawn_orders_lanes_not_slots` (against a shuffled `p_dead`) · POD offset pins for every GPU struct, plus the two `instance_count` offsets cross-checked against the generated shader text and `first_instance == 0` · `particle_layout_table_is_well_formed` (`const fn`) · `push_bytes_do_not_widen_the_shared_compute_range` (`== 112`) · `seed_table_rows_match_the_derived_first_access_at_the_named_pass` (M5) · `access_column_matches_the_declarator` · `counters_have_no_parity_field` (compile-time) · `counters_have_no_alpha_count_field` (M2, compile-time) · **`plugin_does_not_change_event_policy_or_schedule_set`** (M4).

### Property-based tests

**Four-boundary partition** over random spawn/death sequences on a CPU model: `alive + dead == CAP` at B0/B1/B2/B3; the **B1 in-flight window is one-to-one**; **the B3 class split sums to `alive_count_next`** (M2); every slot appears exactly once; no leak; `real_emit ≤ dead_count` always.
**Clock properties (M3):** for arbitrary `(delta, speed, timestep)`, `steps == min(floor(acc/timestep), CEILING)`; the accumulator never grows without bound; `dropped_steps` is monotone; `steps` is the same value in A1's `dt` and the push constant.
**Edge cases:** `CAP` reached exactly; `requested > dead_count`; `dead_count == 0`; `alive_count_cur == 0`; `alive_count_cur == CAP`; `lifetime == 0` (spawn-and-die in one frame — the R3 case that forces dual alive lists); **`steps == 0`** (the common case above the step rate — M6: particles hold position, the sim still rebuilds the list and the render records); `steps == CEILING`; `relative_speed == 0` (pause — zero delta, zero steps); `MAX_EMITTERS`/`MAX_EFFECTS` exceeded; a stale `ParticleEffectHandle` (the effect table is **append-only** until handle generations reach the render path).

### `debug_assert!` invariants

`slot < CAP` at every record write; `dead_base + gid < CAP`; `emit_append_base + gid < CAP`; `real_emit ≤ dead_count_before`; `alive_count_cur ≤ CAP`; `idx < CAP` and `render_index(class, r_pos) < CAP` at every write; `effect_index < MAX_EFFECTS`; `emitter_count ≤ MAX_EMITTERS`; `pc.steps ≤ PARTICLE_SUBSTEP_CEILING` (host-side, before the push); `first_instance == 0` at both draw slots; declare/record parity on `path_has_particles()`; `SortMode == None || !motion_vectors`.

### Benchmarks

Criterion `particle_tick_emitters` at 1/16/256 emitters (≤15 µs) · GPU timestamps at 10k/100k/1M armed and disarmed, against R12 · **A/B render record vs direct gather** · **A/B wave-aggregated vs naive atomics** · **A/B instanced vs non-instanced quad** · the 64-substep worst-case frame cost · sort cost at 100k/1M (P2) · the P2b interpolation traffic delta · the armed-vs-disarmed declaration cost (expected exactly zero — nothing is declared).

---

## Open questions

**For the owner (VALUES / SCOPE):**

1. **HDR scene colour.** `lit` is 8-bit post-tonemap (F2): additive clips at white and contributions below 1/255 vanish. The fix is an HDR scene-colour target with tonemap lifted into its own pass across all four paths — a separate campaign re-pinning every image golden. Accept the LDR limitation for P0–P4, or schedule HDR first?
2. **Default `CAP = 262 144` (24.1 MB).** `CAP` bounds memory only. 256k, or 64k default with 1M opt-in?
3. **`MAX_EMITTERS` / `MAX_EFFECTS` = 256**, both clamped in release and counted. Enough?
4. **Serialization.** Emitters serialize; live particles do not. Confirm out of scope.
5. **P4 scope.** Trails/ribbons and mesh particles as one rung, or split?
6. **Determinism waiver.** Dead-stack pop order makes spawn-slot assignment non-reproducible; correctness is gated by the single-step-exact + multi-step-multiset harness instead. Confirm.
7. ~~*(Rev 2's heartbeat question)*~~ — **dissolved by M4**: the subsystem owns its clock and touches no shared schedule.
8. **New — the particle step rate and P2b (M6).** Default `ParticleClock` rate is 64 Hz, matching the engine's fixed default. Above that rate most frames step zero times and particles visibly move at 64 Hz. P0 ships without interpolation; **P2b** adds it at **+8 B/particle on the render record (+25 % of the draw's read traffic)**, default OFF. Confirm: is 64 Hz the right default, and should P2b be scheduled or left opt-in indefinitely?
9. **New — `PARTICLE_SUBSTEP_CEILING = 64` (M3).** It is reachable only through `relative_speed` (speed 8.0 at stock defaults ⇒ 129 required). Above it, particle time is dropped and counted. Raise it (costs only loop iterations on hitch frames), or keep 64 and accept slow-motion particle lag at extreme speeds?

**Technical, closed by measurement at the P0 gate:**

10. Render record vs direct gather under real alive-list decay.
11. Instancing granularity on the target AMD part (4-vertex instanced vs `vid/6`).
12. Whether `LOCAL_SIZE_X = 256` (Wicked's number) beats 128/64 on this engine's gather pattern.

**The reversal ledger** — kept because the reasoning outlives the decisions:

- **Rev 0 → Rev 1:** the departure from the dead-list/dual-alive skeleton. The bottom-packing premise had no mechanism, and the research corpus removed the bandwidth argument behind it — an SoA argument that does not transfer from CPU to GPU.
- **Rev 1 → Rev 2:** `MAX_SUBSTEPS = 4` invented a clamp on top of an existing engine-wide guard and desynchronised two step counts that should never have been two.
- **Rev 2 → Rev 3:** the same class twice more. A counter serving two consumers needed an `InterlockedMax` mirror (M1) — until the two consumers turned out to want *different numbers*, at which point the mirror vanished. And a subsystem reached for the engine's shared fixed clock (M4), which would have re-tuned event buffering for every unrelated consumer in the process; owning the clock deleted the coupling, the heartbeat, a freeze hazard and an open question in one move. **The recurring lesson is not "check the bookkeeping" — it is that a value with two homes, or a dependency taken for convenience rather than need, is the shape the defects keep arriving in.**