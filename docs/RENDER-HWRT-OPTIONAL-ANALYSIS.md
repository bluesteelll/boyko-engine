# Optional Hardware Ray Tracing for boyko-engine — Final Architecture Analysis

**Status:** analysis / pre-implementation. The software marcher is and stays the always-on baseline; HW-RT is a per-workload perf toggle, not a replacement. **No implementation code is proposed — only the seam, honestly-costed crux, phased gates, and a go/consider/avoid verdict.** Values calls for the owner are flagged **[VALUES CALL]**.

---

## Changes from critique

Every P0/P1 folded (none refuted — all were grounded and correct):

- **P0-1 (the one win-case was asserted, not costed; RT mechanism argues against it).** Rewrote §2 and §0 row 2. The competitor is no longer "TLAS vs today's linear 16-fold" — it is now explicitly **"TLAS + custom AABB intersection shader vs. a compute-shader spatial-partition (grid/loose-BVH over instance AABBs) calling the same fold."** Added the intersection-shader occupancy/divergence penalty as a first-class cost: every ray piercing an instance AABB re-invokes a *full* sphere-trace, and RT cores only accelerate the AABB cull, not the leaf. Consequence: the surviving verdict **downgraded from CONSIDER to AVOID-UNLESS**, with the "unless" precisely bounded.
- **P0-2 (`gen`-gate doesn't hold at the stated scale; refit-vs-rebuild conflated).** Rewrote §2.2/§2.3. Verified in-tree: `bump_gen` is a *single global wrapping stamp* (`boyko_sdf_math/lib.rs` L557) — under hundreds of independently-moving instances *something moves every frame*, so it bumps every frame and gates nothing. Removed it as a "ready mitigation." Split the win-case explicitly into **rigid-transform** (TLAS refit-only, cheap, gen irrelevant) vs. **deforming** (per-BLAS rebuild, likely disqualifying) and made that fork the decisive owner question. Noted per-instance dirty tracking is NEW infrastructure not currently present.
- **P1-1 (in-house AS-builder effort listed but not weighed vs. opportunity cost).** Added §3.6 pricing the raw-FFI surface against the software roadmap, and made H0's number-to-beat bar explicitly high *because* the build cost is large and hand-rolled (zero third-party: no VMA, no nv-helpers).
- **P1-2 (brick-atlas case inherits the intersection-shader problem; competes with AS-free DDA).** Qualified §0 row 6 and §2.3: the real competitor is **compute-shader DDA over the regular brick grid** (the classic sparse-voxel method, no AS at all). HW-RT wins there only if the brick set is irregular/hierarchical enough that BVH beats DDA — stated as a precondition.
- **P1-3 (tolerance gate is per-vendor; project has no multi-vendor CI).** Added to §4-validation and §6: the HW path is **untestable in this project's CI** (single windows-gnu box, validation-crash-prone) → permanently **owner-eval-only**, which materially raises every rung past H1.
- **P2-1/P2-2/P2-3 preserved/tightened:** rayQuery-in-compute kept (§3.1); collision-AVOID kept verbatim (§5); the "thousands of primitives" phrasing tightened so count-alone is not read as the threshold (§0).

**Net effect on the recommendation:** it got *more* negative and *more* precise. The primary path was already AVOID; the one "CONSIDER" case is now AVOID-UNLESS-rigid-instances-at-scale-beat-a-software-grid-on-a-bench. Build H0 (measure) and optionally H1 (dormant seam). Do not build execution.

---

## 1. Executive summary — the honest verdict up front

**State the crux first, because it decides everything.** HW-RT cores accelerate exactly one operation: **traversing a prebuilt acceleration structure (BVH) of geometry to find ray-vs-primitive hits.** Our geometry is not geometry — it is a **per-eval CSG fold of ≤16 analytic primitives** (`field_distance(p)`, `MAX_SDF_EDITS=16`, boot-static gather, verified). There is no triangle mesh and no BVH. RT cores **cannot evaluate our SDF.** To use them at all we must either:

- **(A)** build an acceleration structure of primitive/brick **AABBs** and supply a **custom intersection shader** that runs *our same software sphere-trace* inside each pierced AABB — RT cores accelerate only the AABB cull, never the leaf; or
- **(B)** **mesh** the SDF (marching cubes) into triangles each time geometry changes and traverse a triangle BVH — forfeiting the analytic field, exact normals, byte-identity, and shared CPU/GPU authority.

Both pay a build/refit cost the software marcher does not. **At 16 primitives there is nothing to accelerate** — a linear fold has no traversal stack, no AABB tests, and better I-cache than any BVH. RT cores need thousands of *cullable* primitives with *real empty space between them* AND a leaf test cheap enough that the intersection-shader re-invocation cost does not dominate. Our leaf test (a full sphere-trace) is the opposite of cheap.

### Verdict per use-case

| Use-case | Verdict | One-line reason |
|---|---|---|
| **SDF primary G-buffer march** (`sdf_gbuffer_composite`) | **AVOID** | Coherent primary rays vectorize on compute; 16-edit fold has no traversal cost; feeds the byte-identical golden. |
| **SDFDDGI probe rays** (secondary, incoherent) | **AVOID now** | Best *technical* candidate (many incoherent rays) but at 16 edits the fold beats any AS; only reconsider at large instance count. |
| **SDF soft shadows / AO** (secondary) | **AVOID** | Many secondary rays but low traversal cost at 16 edits; cone/DDA software tricks win first. |
| **Many independent SDF *instances*, RIGID transform** (`MAX_SDF_EDITS` → hundreds+) | **CONSIDER — but only if it beats a *software spatial-partition*, not the linear scan** | The only regime with real empty space to cull; TLAS refit-only is cheap. Must out-bench a compute-shader grid/BVH calling the same fold, net of intersection-shader cost. |
| **Many SDF instances, DEFORMING field per frame** | **AVOID** | Per-instance BLAS rebuild every frame; likely costs more than it saves. |
| **Meshed (marching-cubes) SDF triangle BVH** | **AVOID** | Per-frame meshing of dynamic geometry costs more than the march; forfeits the analytic field and byte-identity. |
| **Brick-atlas AABB-AS** (if brick campaign ships) | **CONSIDER (research) — competitor is AS-free DDA, not "no acceleration"** | A sparse brick set has real empty space, but a regular brick grid is traversed by compute-shader DDA with no AS; HW-RT wins only if the brick set is irregular/hierarchical enough that BVH beats DDA. |
| **Physics collision queries** (`sample_sdf`) | **AVOID (permanent)** | Zero-readback deterministic CPU fold; a GPU rayQuery round-trip adds latency + readback + a second geometry authority (Principle 0) + non-determinism. |

**Bottom line:** For every *current* workload — including the in-flight SDFDDGI at 16 edits — **HW-RT does not pay off.** It has exactly one conditional future win (rigid SDF instances at large count, *if* it out-benches a software grid), and one conditional research case (irregular brick sets, *if* BVH beats DDA). Build the seam for option value; build execution only with a winning bench number in hand.

---

## 2. The technique and the dynamic-SDF crux — honestly costed

### 2.1 The fundamental mismatch (unchanged — this framing is correct)

RT cores traverse a **BVH of geometry**; we have a **distance function**. The two feeds are Route A (AABB-AS + intersection shader) and Route B (mesh + triangle BVH), as in §1.

### 2.2 Route A at scale — costed against the *right* competitor (P0-1, P0-2 folded)

The prior draft compared "AABB-per-instance TLAS" against "today's O(N) linear 16-fold" and concluded a win. **That comparison is wrong** — nobody would linearly scan hundreds of instances in a shader; they would spatially partition in software. The honest accounting:

**Cost of Route A (HW-RT) at N instances:**
1. **AS build/refit** — for *rigid* instances, TLAS refit is cheap (transforms only); per-instance BLAS is static, built once. For *deforming* instances, each changed instance's BLAS rebuilds — expensive, and (P0-2) `bump_gen` is a *single global stamp* that bumps whenever *anything* moves, so it cannot gate per-instance rebuilds; per-instance dirty tracking is **new infrastructure that does not exist today.**
2. **Traversal** — RT cores cull the instance AABBs a ray misses. This is the real HW-RT contribution.
3. **Intersection shader (the buried cost)** — every ray that *pierces* an instance AABB invokes a custom intersection shader that runs a **complete sphere-trace** inside that AABB. With overlapping/adjacent instances one ray fires this many times. Procedural-AABB intersection shaders are a **known HW-RT anti-pattern**: they serialize badly, defeat RT-core throughput, and suffer severe occupancy/divergence penalties (training-knowledge, corroborated by vendor guidance on procedural geometry — flagged as not from a fetched source here). You have not removed the marcher; you have wrapped it in traversal overhead *plus* the intersection-shader penalty.

**Cost of the software competitor (a compute-shader spatial partition):** a coarse uniform grid or loose BVH over the N instance AABBs, traversed inside the existing compute pass, leaf = the same `field_distance` fold. This captures most of the empty-space-skip **without** any AS build, **without** the RHI surface, and **without** the intersection-shader occupancy cliff (it is straight-line compute, no shader re-invocation boundary).

**The comparison HW-RT must win is (3)+(1)+(2) vs. the software grid.** RT cores buy faster AABB culling than a software grid's DDA — but they pay it back in intersection-shader serialization. **Whether the net is positive is entirely empirical and vendor-dependent**, and it is plausible the software grid wins outright (no AS, no RHI, no CI-untestable HW path). Therefore §1's row is **CONSIDER only if a bench shows HW-RT beats the software grid** — not a standing "yes."

**The rigid-vs-deforming fork (P0-2) is decisive:**
- **Rigid instances** (transform-only): TLAS refit-only, cheap, `gen` irrelevant (you refit unconditionally). This is the *only* regime where HW-RT is even in the running.
- **Deforming instances** (edit params change per frame): per-instance BLAS rebuild every frame, needs per-instance dirty tracking that doesn't exist, likely disqualifying.

The owner's "independently-moving" is ambiguous between these; the verdict flips on it (VALUES CALL #2).

### 2.3 Route A for the brick atlas — competitor is DDA, not nothing (P1-2 folded)

If the brick campaign ships, the occupied bricks are a sparse set of AABBs with genuine empty space — the most physically plausible HW-RT case. **But** a regular brick grid/clipmap is traversed extremely efficiently by **compute-shader DDA** (the classic sparse-voxel ray-march), with **no AS at all**, and the per-brick leaf (trilinear fetch + short local march) still runs in an intersection shader under HW-RT with the same occupancy penalty. **HW-RT wins here only if the brick set is irregular/hierarchical enough that a BVH beats a DDA over the grid.** For a regular clipmap, DDA is the competitor and likely wins. Stated as a hard precondition on the CONSIDER.

### 2.4 Route B (meshing) — rejected outright (unchanged)

Per-frame marching cubes on dynamic geometry costs more than the march it replaces and forfeits the analytic field, exact normals, shared CPU/GPU authority, and byte-identity. Not for this engine.

### 2.5 AS verdict

- **16-edit current design point: no AS pays off. Full stop.**
- **Rigid instances at large N: CONSIDER, gated on out-benching a software spatial-partition, net of intersection-shader cost.**
- **Deforming instances at large N: AVOID** (per-frame BLAS rebuild + missing per-instance dirty infra).
- **Brick atlas: CONSIDER (research), competitor = compute-shader DDA; HW-RT only if BVH beats DDA on an irregular set.**
- **Meshing: AVOID.**
- The AS, if ever built, is an **RHI-owned GPU resource derived from the `SdfPrimitive` ECS column** (like `GpuColumn` derives from components) — **not** a parallel data system (Principle 0 preserved).

---

## 3. The optional software/hardware ray-backend seam — where it's worth it

### 3.1 rayQuery-in-compute, not an RT pipeline (P2-1 preserved)

Use inline **`rayQuery` (`VK_KHR_ray_query`, `OpRayQueryProceedKHR`)** inside the *existing* compute passes — no ray-tracing pipeline, no shader binding table, no raygen/miss/closesthit split, no RTPSO. Our marcher is already a compute dispatch; rayQuery fires rays inline against an AS from the shader body where `field_distance` already lives. An RT pipeline would force an enormous second raw-FFI abstraction (SBT, `vkCmdTraceRaysKHR`, RTPSO layouts) for zero benefit on our workloads. **Rejected:** RT pipeline / SBT.

### 3.2 The seam is a capability flag + a setup-resolved backend enum — never `dyn` in the hot path

Two layers, both mirroring proven codebase patterns (`DeviceCaps`, `ResolvedDdgi`):

1. **`boyko_rhi` `DeviceCaps.ray_query: bool`** — true iff the device advertises `VK_KHR_ray_query` + `VK_KHR_acceleration_structure` + the `rayQuery` feature bit **and** we chose to enable them. Populated at boot via the RECORDED-vs-fail-fast degrade pattern (`device.rs` L184-204, verified) — absent → `false`, never a boot failure.
2. **`boyko_render` `RayBackendConfig`** — a cold POD `Resource`, 0%-gate discipline (DISABLED == Default == every workload bit 0 == byte-identical to no-HW-RT), one-for-one with `resolve_ddgi`. A per-workload `RayWorkloadMask` bit is honored **only if** `feature=hwrt` AND `DeviceCaps.ray_query` AND the AS precondition (§2) hold — else silent software fallback.

**Rejected:** `Box<dyn RayBackend>` per ray (virtual dispatch in the hottest loop — Principle 1/4); a single über-shader branching on a `use_hwrt` uniform (bloats I-cache with both paths, breaks the `field_probe_gate` byte-identity isolation, and `rayQueryEXT` needs the SPIR-V `RayQueryKHR` capability a software-only device may reject at pipeline creation). **Two SPIR-V variants selected at build time is safer and faster.**

### 3.3 The seam lives at `field_distance`'s CONSUMERS, not the frozen gateway

`field_distance(p)` / `sdf(p)` are **unchanged** (the `field_probe_gate` byte-identity contract holds). What changes is the *ray integrator* calling them: the HW variant replaces *traversal* (which AABB/brick a ray hits) while the *leaf test still calls our field*. The perf toggle is confined to the march loop.

### 3.4 eDSL implication

`field_distance` math **stays in `boyko_shaderdsl`**, emitted byte-identically, and the rayQuery variant *calls* it inside the intersection test. But `rayQueryEXT` / `OpRayQuery*` are control-flow + resource intrinsics the eDSL does not model. So the **rayQuery march loop is hand-written HLSL** (consistent with the already-hand-written runtime control flow in `sdf_field.hlsli`), and **only the field leaf** is eDSL-emitted and shared. eDSL owns the field polynomial; hand-HLSL owns traversal control flow.

### 3.5 Which workloads route through the seam

| Workload | Route candidate | Note |
|---|---|---|
| SDFDDGI probe rays | HW-eligible (best technical candidate) | but AVOID until large instance count — §1 |
| SDF soft shadows | HW-eligible (2nd) | low traversal cost at 16 edits |
| AO | HW-eligible (3rd) | software cone/DDA wins first |
| Reflections (future) | HW-eligible | not a current workload |
| Physics `sample_sdf` | **software-only, permanent** | §5 |
| Primary G-buffer march | **software-only** | coherent + feeds the golden |

### 3.6 In-house AS-builder effort — weighed against opportunity cost (P1-1 folded)

Zero-third-party means we hand-write the **entire** AABB-AS build: `vkGetAccelerationStructureBuildSizesKHR`, scratch-buffer suballocation, `vkCmdBuildAccelerationStructuresKHR`, device-address plumbing (`VK_KHR_buffer_device_address`), compaction, and AS-build→ray-read barriers — no VMA, no nv-pro-samples helpers. This is genuinely fiddly raw Vulkan. Against it stand software rungs with **known** payoff (finishing the brick atlas; the owner-locked SDFDDGI). **Disposition:** the effort is not reliably estimable before H0, so the mandate is inverted — **H0's measured software ray-cost must clear a deliberately high bar** (large enough to amortize a multi-rung hand-rolled AS builder *and* a CI-untestable HW path) before any execution rung is authorized. If H0 shows the software ray cost is small, the builder is never worth writing.

---

## 4. Vulkan API path, feature-gate/degrade, and byte-identity disposition

### 4.1 Concrete raw-FFI RHI additions (only if a §2 bench passes)

In `boyko_rhi_vulkan` (raw-FFI, zero third-party):

1. **Device create** — the verified `VkDeviceCreateInfo` seam (`device.rs` ~L2449): chain `VkPhysicalDeviceAccelerationStructureFeaturesKHR` + `VkPhysicalDeviceRayQueryFeaturesKHR` into `pNext`; add `VK_KHR_acceleration_structure`, `VK_KHR_ray_query`, `VK_KHR_deferred_host_operations`, `VK_KHR_buffer_device_address` to `pp_enabled_extension_names`. All gated behind a boot query (`supports_ray_query`, mirroring `supports_dynamic_rendering`) → absent sets `DeviceCaps.ray_query=false`, never a boot fail.
2. **New PFNs** loaded via `vkGetDeviceProcAddr` (§3.6 list).
3. **New associated type** `RhiApi::AccelerationStructure` on the static-dispatch `RhiApi` umbrella (`api.rs` already has the deferred-unbounded-associated-type pattern — one more, no ABI break, no `dyn`).
4. **AS backing** — DeviceLocal buffer + scratch from the existing suballocator with `buffer_device_address` usage.
5. **Encoder** — `cmd_build_acceleration_structure` + a framegraph AS-build→ray-read barrier.

### 4.2 Feature gate + graceful degrade (mirror `resolve_ddgi` exactly)

- Cargo feature `hwrt` (off by default) gates the whole AS code path and the second shader variant → a build without it has **zero** RT surface and zero SPIR-V-capability risk.
- Runtime: `RayBackendConfig::default()` = all-software (0%-gate). A workload bit is honored only if `feature=hwrt` AND `DeviceCaps.ray_query` AND the AS precondition hold — else silent software fallback. **Never crashes; degrades** (the `!enabled() → DISABLED` SDFDDGI pattern one-for-one).

### 4.3 Byte-identity disposition — the discipline adapts, per-vendor cost exposed (P1-3 folded)

**Hard truth:** a HW-RT path **cannot** be byte-identical to the software march — RT-core traversal, `rayQueryEXT` ordering, and driver intersection math produce different bits than our IEEE sphere-trace. The `field_probe_gate` cannot cover the HW path.

**The adapted discipline:**
1. **The frozen field gate is UNTOUCHED and absolute.** `field_distance`/`sdf`/`smin`/`smax`/normal SPIR-V stays byte-identical — the HW variant *reuses* the same emitted leaf. `field_probe_gate` still passes.
2. **The HW integrator gets a NEW gate class: a tolerance-bounded perceptual regression** (per-pixel ΔE / SSIM; irradiance-RMS for DDGI) vs. the software golden. Honest for a lossy, temporally-filtered quantity like DDGI irradiance.
3. **The software render stays the byte-golden authority.** HW-RT is validated *against* it within tolerance; it never becomes the reference.
4. **CPU physics stays byte-identical** to the golden (never touches HW-RT — §5), so the render↔physics geometric agreement (the P4 invariant) is fully preserved.

**The per-vendor multiplier (P1-3):** the tolerance gate is **not one gate — it is a per-vendor matrix.** A tolerance passing on NVIDIA RTX may fail on AMD/Intel/integrated, or mask a real bug within tolerance on one vendor and not another. This project has **no multi-vendor CI** (single windows-gnu box, validation-layer-crash-prone — recorded environment fact). Therefore the HW path is **effectively untestable in CI and permanently owner-eval-only** (like the shadow-host rungs). This materially raises the true cost of every rung past H1 and independently reinforces "do not build execution without a decisive win."

**[VALUES CALL #3]:** accepting the HW path reframes the engine's flagship guarantee from "byte-identical everywhere" to **"byte-identical software-always; HW-RT is a bounded-tolerance, owner-eval-only opt-in."** Unavoidable — RT cores are not IEEE-deterministic across our math. Confirm the framing.

---

## 5. Collisions — HW-RT is not worth it; blunt (P2-2 preserved)

`boyko_physics::sample_sdf` folds the **same `boyko_sdf_math` leaf** as the GPU, on the CPU, with **zero readback**, deterministic, AVX2-vectorized (the W4 0%-regression kernel), inside the TGS-Soft narrowphase on the work-stealing threadpool.

Offloading batched physics queries to a GPU rayQuery pass pays: (1) **round-trip latency** — upload rays → dispatch → fence → **read back**, per fixed-timestep substep (often multiple/frame) — a stall the CPU fold never has; (2) **a second geometry authority on the GPU** diverging from the CPU field the solver trusts — a **Principle 0 violation** and a re-run of the exact SP4 data-race class the charter root-caused; (3) **loss of determinism** — physics MUST be reproducible (fixed timestep, save/load); RT intersection is not bit-reproducible; (4) **low ray count** — a handful of contacts/raycasts per body, nowhere near the millions-of-rays amortization regime.

**Verdict: AVOID, permanently.** If physics ever needs *bulk* scene queries (thousands of particles vs. scene per step), the right answer is a **CPU-side batched SIMD sweep of the same leaf on the threadpool**, not a GPU RT pass.

---

## 6. Phased roadmap — the first rung is a MEASUREMENT

Every rung has an honest gate; a rung that fails its bench does not proceed.

**Rung H0 — Instrument the software baseline (NO HW-RT code).** Add GPU timestamp queries around the DDGI probe-update march and the SDF shadow march. Produce *ns per ray per workload on our real scene at current edit count.* **Gate:** this is the bar HW-RT must beat, and per §3.6 the bar is deliberately high (must amortize a hand-rolled AS builder + a CI-untestable owner-eval-only HW path). **Cheap, zero risk, immediately useful for the software path regardless. Do this first, unconditionally.**

**Rung H1 — The dormant seam.** Land `DeviceCaps.ray_query` (boot query) and `RayBackendConfig` (0%-gate Resource, all-software default). **No AS, no shader variant, no RT extensions enabled.** Pure option-value; the baseline is byte-untouched. **Gate:** `RayBackendConfig::default()` resolves to all-software and the golden stays byte-identical (trivially).

**Rung H2 — [GATED ON VALUES CALL #2 = rigid instances at scale] AABB-AS + rayQuery prototype behind `feature=hwrt`.** Only if the roadmap has large *rigid* SDF-instance counts (deforming → skip) OR the brick atlas ships. Build the AABB-AS, a hand-HLSL rayQuery variant calling the eDSL field leaf in the intersection test, tolerance gate vs. the software golden (owner-eval, per-vendor). **Gate:** on the real scene, the HW path beats **the software spatial-partition baseline** (not the linear scan — P0-1) by a margin justifying the AS-build + RHI surface + owner-eval-only testing, AND the tolerance regression is within the owner-set bound. **If it doesn't beat the software grid: stop, keep the seam dormant, document the negative result.**

**Rung H3 — Extend to SDF shadows / AO** (only if H2 wins and workloads share the AS).

**Rung H4 — TLAS refit for dynamic *rigid* instances** (only if the dynamic-edit campaign shipped, instances are rigid, and H2/H3 proved the win at scale). Requires per-instance dirty tracking (NEW infra — §2.2) if not refit-unconditional.

**Gate philosophy:** ship H1 (free option value); ship execution (H2+) **only** with a winning number vs. the *software-grid* competitor in hand. Mirrors the SDFDDGI "~3 ms unbenched → gate on bench" discipline.

---

## 7. Final recommendation, what would change the answer, and the VALUES calls

**Recommendation.** Build **H0 (measure — now, useful regardless)** and optionally **H1 (dormant seam — cheap, zero-risk option value)**. **Do NOT build H2+** unless a bench proves HW-RT beats a *software spatial-partition* on a real large-rigid-instance scene, net of the intersection-shader cost, and the owner accepts an owner-eval-only tolerance path. For the **current 16-edit design point — including the in-flight SDFDDGI — HW-RT does not pay off.** Physics HW-RT: **avoid, permanently.**

**What would change the answer (the honest levers):**
1. **`MAX_SDF_EDITS` → hundreds+ of RIGID SDF instances** with real empty space between them — the only regime where HW-RT is in the running, and only if it out-benches a software grid.
2. **The brick atlas shipping with an IRREGULAR/hierarchical occupied-brick set** where a BVH beats compute-shader DDA (a regular clipmap → DDA wins, no AS).
3. A future workload of **millions of incoherent secondary rays** (large-scale RT reflections/GI) that a linear/grid software fold genuinely cannot serve.

If none of these materialize, the seam stays dormant forever and that is the correct outcome.

**What would NOT change the answer:** merely "having an RTX GPU," or "more edits but still tens," or "deforming instances" (BLAS-rebuild-bound), or "physics wants raycasts" (CPU fold wins).

**VALUES CALLs for the owner:**
1. Prototype HW-RT (if ever) on the in-flight SDFDDGI probe pass (best technical candidate) or a throwaway shadow pass (keeps DDGI frozen)?
2. **The decisive one:** are large-instance-count SDF scenes on the roadmap AND are those instances **rigid (transform-only → possibly viable)** or **deforming (per-frame BLAS rebuild → likely disqualifying)**? The verdict flips on this fork; without "rigid at scale," §2's machinery is never built.
3. Accept the HW path as a **bounded-tolerance, owner-eval-only** opt-in (no automated multi-vendor CI gate exists on this project's hardware), reframing the determinism guarantee as "byte-identical software-always; HW-RT tolerance opt-in"? If owner-eval-only is unacceptable, HW-RT execution should not be built at all.

---

**Grounding note.** In-tree citations verified on branch `ecs`: `MAX_SDF_EDITS=16` + boot-static gather (`crates/boyko_render/src/sdf_edit.rs` L3-124); `SdfEditField`/`bump_gen` as a **single global wrapping brick-cache stamp** (`crates/boyko_sdf_math/src/lib.rs` L318, L557 — confirms P0-2's wrong-granularity finding); `DeviceCaps` RECORDED-vs-fail-fast degrade (`crates/boyko_rhi_vulkan/src/device.rs` L184-204); the `VkDeviceCreateInfo` feature/extension seam (`device.rs` ~L2449); frozen `field_distance` gateway (`crates/boyko_rhi_vulkan/shaders/sdf_field.hlsli`); static-dispatch `RhiApi` umbrella (`crates/boyko_rhi/src/api.rs`); zero-readback CPU query (`crates/boyko_physics/src/sdf_query.rs`); zero `VK_KHR_ray_tracing`/`acceleration_structure`/`rayQuery` anywhere in the tree. The intersection-shader occupancy/divergence penalty (§2.2) and regular-grid-DDA efficiency (§2.3) are stated from training knowledge of HW-RT literature and vendor procedural-geometry guidance, **not** from a fetched source — flagged as such.

**Relevant files (absolute):**
- `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\src\device.rs` — `VkDeviceCreateInfo` seam (~L2449), `DeviceCaps` degrade (L184-204).
- `D:\claude\BoykoEngine\crates\boyko_render\src\sdf_edit.rs` — `SdfPrimitive` column, boot-static gather, `MAX_SDF_EDITS`.
- `D:\claude\BoykoEngine\crates\boyko_sdf_math\src\lib.rs` — `SdfEditField`/`bump_gen` global stamp (L318/557).
- `D:\claude\BoykoEngine\crates\boyko_render\src\ddgi_config.rs` — the 0%-gate resolve pattern `RayBackendConfig` mirrors.
- `D:\claude\BoykoEngine\crates\boyko_rhi\src\api.rs` — static-dispatch `RhiApi` seam for the `AccelerationStructure` associated type.
- `D:\claude\BoykoEngine\crates\boyko_physics\src\sdf_query.rs` — the zero-readback CPU query HW-RT collision would wrongly displace.
- `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\shaders\sdf_field.hlsli` — frozen field gateway; the seam is at its consumers.