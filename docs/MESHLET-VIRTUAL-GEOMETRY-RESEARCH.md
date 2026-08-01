# Virtual geometry (meshlets) — research, and the ladder it produced

**Status:** RESEARCH COMPLETE, 2026-07-26. **No design is approved and no code exists.** This is the
evidence base for the owner's decision that a meshlet / virtual-geometry system will be built with
the target *faster than Nanite on any scene*.

**Method.** Five parallel survey lenses (Nanite internals and cost map; the offline cluster/DAG
build; runtime culling and micro-polygon rasterization; the Vulkan hardware path reachable from this
engine's raw-FFI RHI; and this repository's own integration seams) followed by two adversarial
passes — one that killed every performance number whose source did not name *who measured it, on
what hardware, on what scene*, and one that killed every lever unreachable from raw Vulkan or in
conflict with the shipping visibility buffer — and then a synthesis. Eight agents, 450 tool calls.

**The headline result is a refutation, and it is the most useful thing here.** No measured Nanite
cost map exists in any source the survey could reach. The two figures that circulate have no
traceable primary source and are recorded below as folklore. Therefore *"beat Nanite" is not
currently falsifiable*, and the first rung of the ladder is a measurement rung that can kill the
campaign for free, three separate ways, before a line of cluster code is written.

**One correction to the premise that started this work.** Nanite's clusterization, cluster-group DAG
construction and quadric simplification are **offline**, at asset build time. Only the LOD-cut
selection through the prebuilt DAG, the culling and the rasterization are per-frame. Aiming at
"dynamic partitioning" would have aimed at nothing.

**A naming decision taken here rather than asked, because it is a one-way door.** In this codebase
`cluster` already means **light froxel** — `cluster_cull.hlsl`, `ClusterGrid`,
`MAX_LIGHTS_PER_CLUSTER`, and the whole VB-P1e campaign that measured 22.5x at 512 lights. Geometry
uses **`meshlet`** for the leaf and **`geo_group`** for the DAG group. `cluster` stays with lights.

**Evidence classes are load-bearing and are used throughout:** MEASURED (who / hardware / scene all
named), AUTHOR-CLAIMED (asserted without those three), INFERRED (the synthesis's own reasoning),
FOLKLORE (killed by the adversarial pass; must never re-enter as fact), REPO-VERIFIED (read from
this tree during the study).

---

# Virtual Geometry: Decision Document

**Status:** synthesis of 5 surveys + 2 adversarial passes. No engine code written. Every number carries an evidence class: **MEASURED** (who / hardware / scene stated), **AUTHOR-CLAIMED** (asserted by an author or vendor without those three), **INFERRED** (my reasoning), **FOLKLORE** (killed — never re-enters as fact), **REPO-VERIFIED** (I read the file this session).

**Baseline restated, because everything hangs on it:** Nanite's clusterization, cluster-group DAG construction and quadric simplification are **OFFLINE** (import/bake, DDC-cached). Runtime does LOD-cut selection through the prebuilt DAG, culling, rasterization and material resolve. All five surveys agree; not one source claims per-frame partitioning. One precision the surveys did not state: UE 5.4+ tessellation *does* dice patches into micropolys at runtime and 5.6/5.7 Assemblies resolve part transforms during culling — so "nothing is generated per frame" is now too strong, while "nothing is **partitioned or simplified** per frame" remains exactly right.

---

## 1. The cost map — where a Nanite frame actually goes

### 1.1 The finding that dominates this section

**There is no measured Nanite cost map in existence that any of five surveys could find.** Not one number states Nanite's own cost with named GPU + named scene + named resolution + named error target. The two figures that circulate ("~2.5 ms cull+raster on PS5", "VisBuffer < 4 ms / BasePass < 3 ms / ~8 ms budget") have no traceable primary source and are **FOLKLORE**. Every real per-pass measurement in the evidence set is of a *clone* (Bevy) or an *unrelated design* (Granite, CuRast).

Consequence: **"faster than Nanite" is currently unfalsifiable.** That is not a reason to abandon the goal; it is the reason rung 1 is a measurement rung (§4).

### 1.2 The structural map (what the passes are)

Per **view**, and a lit frame runs this more than once:

| Stage | What it costs | Nanite's form |
|---|---|---|
| Instance cull | O(instances) | frustum + HZB vs last frame |
| **LOD cut selection + cluster cull** | O(traversed DAG nodes) | BVH traversal, persistent-threads MPMC queue (`r.Nanite.PersistentThreadCulling` toggles a multi-dispatch arm) |
| Raster SW | O(covered pixels), global 64-bit `InterlockedMax` | 1 thread/triangle, 128-thread group |
| Raster HW | O(primitives) setup-bound | mesh shaders on PC (`r.Nanite.MeshShaderRasterization=1`) |
| HZB build + 2nd pass | small | pass 2 tests against **current**-frame HZB |
| Emit depth / motion / material depth | O(pixels) | separate pass, because SW raster owns depth in the atomic |
| Material resolve | O(pixels) + per-bin dispatch overhead | 5.4 compute "shading bins" |
| **× N views** | primary + VSM directional pass + VSM local-light pass | full cull+raster each |

Sources: Epic UE 5.8 docs (VSM structure, invalidation rules); elopezr RenderDoc capture of *Valley of the Ancient* (pass names, R32G32_UINT visbuffer, 25b cluster + 7b triangle + separate 32-bit depth); Wyatt's UE 5.0 source read (PersistentCull, HZB mip where the screen rect is ≤4×4). All **AUTHOR-CLAIMED** or capture-derived, none timed.

**The multi-view fact is a hard fairness constraint on this whole campaign:** any cost map that compares only the primary VisBuffer is not a comparison. (Correction the surveys got wrong for *us*: we ship **no VSM**. Our mesh-leg shadows are separate CSM-cascade and punctual-atlas depth-only raster passes. So "cull once, rasterize N views" does not reuse our primary raster — it requires a *second, depth-only* variant of the virtual-geometry path. The idea survives; its cost estimate in the surveys did not.)

### 1.3 The only real per-pass numbers we have

**Bevy 0.15 virtual geometry — MEASURED.** jms55, RTX 3080 at base clocks, 2240×1260, GPU timestamps, 10-frame average, **excluding shading and all CPU work**:

- *Bunny* (3,375 Stanford bunny instances): Fill 0.12 / Cull-1st 0.19 / SW raster 0.42 / HW raster <0.01 / DownsampleDepth 0.03 / Cull-2nd 0.06 / ResolveDepth 0.04 / ResolveMatDepth 0.04 = **0.93 ms**
- *Icelandic lava cliff* (847 instances, 15,616 meshlets at LOD0): **2.32 ms**, of which **culling 1.27 ms vs SW raster 0.34 ms**

**Read that last row carefully.** On the heavier scene, cut selection + culling costs ~4× rasterization. But this is **Bevy's** bottleneck, not Nanite's — Bevy dispatches one thread per scene cluster with no BVH early-out, and its author names that as the cause and says his BVH traversal was never finished. **Nanite already has BVH + persistent-thread traversal. This headroom is not headroom against Nanite.** No survey drew that distinction; it changes the ranking.

**Granite mesh-shader ladder — MEASURED.** Arntzen, RTX 3070, kitten 13×13×13 = 63.59M triangles, 1080p: `vkCmdDrawIndexed` no culling 5.5 → frustum 4.3 → MDI 3.9 → meshlet encoded 4.1 → decoded 4.0 → **per-primitive culling 3.3 → micro-poly rejection 1.9** (1.65 with stats atomics removed) → VertexID-passthrough attribute fetch 1.0 ms. Caveats that must travel with it: the 5.5 ms baseline is a strawman (no culling), and the 1.0 ms row changes the *attribute-fetch scheme* to a visibility-buffer shape, so 5.5 → 1.0 is not a mesh-shader speedup.

### 1.4 Where the cost is *not*

- **Second occlusion pass:** 0.06 ms cull + 0.03–0.05 ms pyramid, second-pass raster <0.01 ms (MEASURED, Bevy/RTX 3080). There is nothing to reclaim by deleting it.
- **Offline build is a solved cost, not a risk.** zeux, Zorah 1.64B unique / 18.9B instanced triangles, 36.1 GB → full cluster-LOD hierarchy in **~2m35s** on a Ryzen 7950X at 16 threads, 45–55 GB peak RAM (MEASURED; named tool, hardware, scene). Bake time is not a reason to avoid a DAG.

### 1.5 What "faster" is allowed to mean

`r.Nanite.MaxPixelsPerEdge` (default 1.0; 4.0 used as an aggressive perf test) changes rendered triangle count by roughly an order of magnitude. **Without a pinned error target plus an image-equivalence gate, any "win" is just a coarser LOD.** No survey defined this axis. It is a gate condition on rung 1, not an afterthought.

---

## 2. The honest verdict on "beat Nanite"

The owner has committed to building this. The question is where the win comes from. Axis by axis:

### Plausible — build toward these

**A. Offline DAG quality (triangles submitted at fixed screen-space error).** This is the strongest axis, and it is the multiplier on *every* downstream runtime pass, on *every* scene including Nanite-unfavourable ones. Evidence that Nanite-class builders are provably not near-optimal:
- Boundary over-locking: 30–50% of a meshlet's vertices locked at DAG level 0 under the naive border definition, dropping below 30% under a group-border-only definition (AUTHOR-CLAIMED, Scthe; definition-dependent, no asset list — usable as a shape, not a constant).
- Simplification **gets stuck** as a first-class terminal case in every production builder: meshoptimizer `clusterlod.h` aborts and sets `bounds.error = FLT_MAX` above `simplify_threshold = 0.85`; Bevy's `SIMPLIFICATION_FAILURE_PERCENTAGE = 0.60`; occupancy degrades 31,244 meshlets @99.6% (LOD0) → 18,265 @70.7% (LOD1); some models plateau at ~3,000 triangles in coarse LODs (**MEASURED**, source-level + named meshes).
- Grouping objective mismatch: METIS minimizes **edge cut**; the quantity that binds simplification is the count of distinct **boundary vertices** (INFERRED from Bevy/jglrxavpok/NVIDIA source, but the fix direction is corroborated — NVIDIA's `nv_cluster_lod_builder` explicitly applies a min-cut "to keep old borders internal to groups").
- A 2026 Eurographics paper attacks locking head-on: Ladeuil, Trabucato, Vaisse, Faraj, *Construction of clustered HLOD with As-Simplified-As-Possible boundaries*, CGF, doi 10.1111/cgf.70380. Paywalled; **no quantitative results obtained**.

Epic's own strongest admission against interest is here: UE 5.7 replaced triangle clusters with **voxel** clusters for aggregate foliage because triangles gave "sub-optimal cluster culling and poor simplification in the distance."

**B. Fixed-cost floor on low-density scenes.** Nanite has one path and pays its fixed cost unconditionally; Epic documents the loss cases (large non-occluding triangles, sky spheres). We can collapse structurally to a plain draw below a measured threshold. This is the only defensible route to *"higher than Nanite on ANY scene."* Correction to the surveys: this is **not** "architecturally free" — our path resolver is a boot-time, whole-frame selection, and per-instance routing + a cost model + hysteresis is entirely new machinery.

**C. Material/shading resolve.** Epic measured 3,075 of 3,779 shading bins **empty** (81%) in one frame and states the naive compute-shading implementation was *slower* than UE 5.0 before optimization (AUTHOR-CLAIMED, no hardware/scene). We already ship a fused compute resolve over a pure visibility buffer with no G-buffer materialization — we start on the side of that tradeoff Epic had to engineer back toward.

**D. Residency per triangle.** Nanite's own figure is ~13 B/triangle (AUTHOR-CLAIMED, and it is Epic measuring its own format against its own *uncompressed* static-mesh format on an unnamed asset — not a comparison against any compressed baseline). Published block formats bound the achievable: DGF 3–6 B/tri topology + 1.57–3.38 B/tri attributes (AUTHOR-CLAIMED); meshopt meshlet codec 9–12 bits/triangle aggregate (AUTHOR-CLAIMED). Real headroom, weak evidence on both sides.

### Unlikely — do not build the campaign around these

**E. Cluster culling / LOD cut selection.** **No.** Nanite already does BVH traversal with persistent threads and a fallback arm. The dramatic culling headroom in the evidence set is *Bevy's*, from a flat per-cluster dispatch Nanite does not use. Matching Nanite here is parity, not a win, and the mechanism (persistent threads) relies on inter-workgroup forward progress that **Vulkan does not guarantee** — a deadlock would be device-specific and would not reproduce on our single RTX 3060. *What we win instead:* the correct target here is not throughput but **cut churn** — Nanite re-derives the whole cut every frame by construction, which maximizes temporal instability. See §3 L11.

**F. Software rasterization as a headline win.** **Unproven in either direction, and the evidence used to justify it is dead.** Epic's "~3× faster for tiny triangles" has no GPU, no scene, no resolution, no baseline definition in any reachable source, and has now been laundered into a peer-reviewed citation without being measured — it cannot carry this fork. Bevy's 4.97 → 0.93 ms is **not** an SW-vs-HW measurement: the 0.14 "hardware" baseline was a single indirect draw over the total triangle count fed by a per-triangle ID buffer (a degenerate primitive-assembly path), and the delta additionally confounds cluster size, group size, vertex-locking policy and a culling rewrite. CuRast is **CUDA, not Vulkan compute**, has no LOD on either side, and its crossover sits at ~28.7M *visible* triangles on an RTX 4090 — where the paper itself labels it "comparable performance" against an indexed hardware draw. **No public controlled A/B of the same post-LOD cluster set through SW vs HW raster exists anywhere.** *What we win instead:* micro-poly **rejection before setup**, which is the measured effect (3.3 → 1.9 ms, MEASURED, RTX 3070) and needs no software rasterizer at all.

**G. Anything requiring GPU work graphs.** The only Vulkan path is `VK_AMDX_shader_enqueue` — AMD vendor-only, formally provisional ("should not be used in production applications"). Our only test box is an RTX 3060: **the experiment cannot be run here at all.** Shelve entirely; do not budget a prototype.

### Ill-posed — different quality/product targets

**H. Feature coverage.** Nanite has no translucency, no direct RT against Nanite geometry, and is desktop-renderer-only (a *support-policy* gap, not a hardware impossibility — do not build strategy on it). We would have different gaps. Comparing here is comparing products.

**I. "Faster" without a fixed error target.** Ill-posed until §4's gate exists.

**J. Nanite's post-5.4 surface** (tessellation, Assemblies, Skinning, Voxels). Several 2021-era criticisms are stale. Voxel aggregates in particular have **zero performance data anywhere in the evidence set**, and they are exactly the "hard scene" class the owner's goal names.

---

## 3. Ranked levers (expected win per effort)

Ranking rule, as this project's existing perf tracks do it: enablers and measurement first, then cheap measured wins, then expensive uncertain ones. Cost/risk are stated **in this engine**, from repo-verified facts.

---

**L1 — Load the indirect seam.** *(enabler, XS)*
- **Axis:** ability to express a GPU-decided cut at all.
- **Mechanism:** add `vkCmdDrawIndexedIndirect` + `vkCmdDispatchIndirect` to the device fn table; add `VK_PIPELINE_STAGE_DRAW_INDIRECT_BIT` / `VK_ACCESS_INDIRECT_COMMAND_READ_BIT` to `ffi.rs`; route the already-ABI-guarded `BufferUsage::INDIRECT` through `create_buffer`; fix `barrier.rs`'s `GpuStage::Indirect`, which currently widens to a full stage/access superset *because the constants are missing* (and has a test freezing that behaviour).
- **Magnitude:** none directly. **Prerequisite for L2, L6, L7, L8, L9.**
- **Cost:** ~3 fn-table entries + 2 constants + 1 usage path + 1 test update. No shader change, no `.spv` change, no golden change.
- **Risk / trap:** **REPO-VERIFIED this session** — `device.rs` loads exactly `vkCmdDispatch`, `vkCmdDraw`, `vkCmdDrawIndexed` and nothing else, and contains **no** `VkPhysicalDeviceVulkan12Features`, **no** `shaderInt64`, **no** `drawIndirectCount`. The two commands above are core 1.0 with no feature bit and are genuinely free. **`vkCmdDrawIndexedIndirectCount` is not** — it needs the `drawIndirectCount` feature in the unchained `VkPhysicalDeviceVulkan12Features`. Ship the two free ones in this rung; the Count variant belongs to L4b.

> ✅ **LANDED. Three things the rung established that the estimate did not.**
>
> **(1) The usage path needed NOTHING.** `BufferUsage::INDIRECT` was already routed: `create_buffer`
> does `let usage: VkFlags = desc.usage.bits()` — a raw pass-through — and `INDIRECT`'s value *is*
> `VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT`. One of the four listed items was complete by construction.
> The estimate said "1 usage path"; the correct answer was zero, and finding that out cost one grep.
>
> **(2) The fn-table load IS the feature probe, so "core 1.0, no feature bit" is now MEASURED.**
> `load_device_command(..)?` fails boot when a command is absent, and every golden pin booted — so
> the claim is a measurement rather than a reading of the spec.
>
> **(3) `wide_stage()` was DELETED, not left unused.** Its only caller was the `Indirect` arm this
> rung narrowed. A widening helper kept "in case" is a thing an implementer greps for and reaches
> for. `wide_access()` survives because it still has one honest caller — see below.
>
> ⚠️ **The barrier fix is a NARROWING ON ONE ARM ONLY, and the asymmetry is Vulkan's.**
> `(Indirect, Read)` becomes exactly `DRAW_INDIRECT` / `INDIRECT_COMMAND_READ`. `(Indirect, Write)`
> still widens, because **Vulkan defines no indirect-write access bit at all**: an indirect-argument
> buffer is *written* by compute or transfer and only *read* by the indirect-fetch stage, so an
> "indirect write" declaration names a stage that cannot write. Substituting `SHADER_WRITE` there
> would **under**-synchronise whenever the real producer was a transfer — the one direction a
> barrier may never be wrong in. The replacement test asserts **both** arms, so the asymmetry is
> checked rather than described.

---

**L2 — Per-instance GPU cull + indirect draw.** *(the rung every survey skipped)*
- **Axis:** CPU draw-record cost; establishes the entire GPU-driven plumbing.
- **Mechanism:** compute frustum (later HZB) cull over the instance ring → compacted indirect draw list + count buffer → one indirect draw. Today the engine does **zero** GPU-side culling of any kind and records one `vkCmdDrawIndexed` per `DrawBatch`.
- **Magnitude:** frustum culling alone measured 5.5 → 4.3 ms (MEASURED, Arntzen, RTX 3070, 63.59M tris) — but that is *his* scene, not ours. On our current content the honest expectation is **near zero**, and that is the point: it is measurable on scenes we already have, and it de-risks cull-pass declaration, compaction, indirect barriers and count buffers *before* any meshlet exists.
- **Cost:** one compute shader (via eDSL), one framegraph pass, ResIds appended at the tail with matching sink slots, one indirect buffer.
- **Risk:** low. Byte-identical goldens are achievable if the cull is conservative-exact.

---

**L3 — The Nanite baseline harness + quality gate.** *(precondition for the goal statement)*
- **Axis:** falsifiability of the entire campaign.
- **Mechanism:** UE5 + `stat GPU` / Unreal Insights / RenderDoc on **our** GPU, **our** resolution, on a scene our engine can also load, with `r.Nanite.MaxPixelsPerEdge` pinned. Plus an image-equivalence gate and a **stated decidability floor** (n-of-m sampling, clock pinning, confidence intervals).
- **Magnitude:** none. Without it, no claim in this campaign is decidable.
- **Cost:** an importer (see §5 K1), a UE5 install on the measurement box, a capture protocol, a documented baseline table.
- **Risk:** the repo's own record — the GPU bench class **does not reproduce above N=128, ~21% run-to-run spread, a single-sample ≤5% gate at high N is not decidable**. A harness that cannot resolve the delta we intend to claim will silently bless wrong constants. This is the risk, and it is why L3 is rung-1 material.

---

**L4 — DAG-quality harness (bake-time).** *(makes every offline lever decidable)*
- **Axis:** measurability of §2A.
- **Mechanism:** bake-time evaluator that walks the DAG over a fixed camera path and reports the **raw curve** of triangles/clusters vs screen-space error, plus per-level locked-vertex %, stuck-group count and root count.
- **Magnitude:** none directly; it is the instrument.
- **Cost:** low-medium, bake-time only, no runtime cost, no golden churn.
- **Risk:** **do not** adjudicate builder changes with a rendered-image error metric. This project has already recorded that image statistics lie about render changes — a metric once scored a TAA blur regression as a 10-point antialiasing *gain*. The number must come from the DAG.

---

**L5 — Offline DAG builder quality.** *(the main headroom — §2A)*
- **Axis:** triangles submitted per frame at equal visual error. Multiplies every downstream pass, on every scene.
- **Mechanism, in ascending cost:**
  - (a) **Certify monotonicity by construction + a bake-time verifier:** `parent_error = max(own, max(child))`, parent sphere conservatively enlarged to contain children's, parent and child bounds computed by the **same code path**, fail the *bake* not the frame. Corroborated by three independent source bases (meshoptimizer's `clusterlod.h` comment that precise merged bounds "may violate monotonicity"; Bevy's runtime `verify_bvh` asserts; NVIDIA's sphere enlargement). **LOW cost, days.** Not a speed win — it protects every other lever from silently producing an invalid cut.
  - (b) **Lock only group-shared vertices, not the full topological border.** Bevy did exactly this in 0.15.
  - (c) **Explicitly schedule boundary rotation:** carry a "was-locked at level L" bit and weight the level-(L+1) partition so those edges prefer to fall *inside* a group. Nanite gets this as a side effect of re-grouping; nothing in a METIS edge-cut objective enforces it. NVIDIA's builder already applies a min-cut for precisely this. **LOW cost.**
  - (d) **Attack the stuck case:** UV-aware welding pre-pass with per-level thresholds, attribute-weighted quadrics, permissive collapse across attribute discontinuities (protect rather than hard-lock).
  - (e) **Boundary-vertex-minimizing (hypergraph) partitioning** instead of edge-cut. HIGH uncertainty: Mt-KaHIP's 18.7%-smaller cut vs Mt-Metis is MEASURED **on graph benchmarks, not mesh grouping** — transfer unproven.
  - (f) **As-Simplified-As-Possible boundaries** (CGF 2026) — RESEARCH ONLY until the paper is read. Identified risk is the right one: if neighbours must agree on a boundary refinement level, cut selection stops being per-cluster-independent and cost moves from bake to frame.
- **Magnitude:** the honest statement is that root-triangle-count differences of ~54% between two builders differing mainly in what they lock **were** MEASURED (Aug 2024, named meshes) — but the criticized target (Bevy pre-0.15) no longer exists, so that figure is **stale by ~20 months** and must be re-measured, not cited.
- **Cost:** medium (a–d) to research-grade (e–f). Gated on the L4 harness and on §6 Q1 (third-party dependency policy).
- **Risk:** cracks and non-monotone error are silent correctness bugs that show only as popping. Permissive collapse across UV seams trades geometric depth for texture artifacts — content-dependent, needs per-asset opt-out and a visual oracle.

---

**L6 — Micro-poly rejection before any rasterizer dispatch.**
- **Axis:** primitives reaching triangle setup.
- **Mechanism:** per-primitive screen-space bbox + backface + cone test at cull time, *then* choose a raster route.
- **Magnitude:** **3.3 → 1.9 ms, 96% of primitives rejected (31M → 1.2M)** — **MEASURED**, Arntzen, RTX 3070, 63.59M tris, 1080p, **on a path with no software rasterizer at all**. This is the strongest single verified number in the entire evidence set, and it says the win comes from *refusing* to hand tiny triangles to fixed-function setup, not from replacing it.
- **Cost:** work inside the cull stage; couples to the meshlet format (positions must be decodable early).
- **Risk:** low. Conservativeness bugs produce holes — golden-gated.
- **Note on backface culling specifically:** **REPO-VERIFIED** — `CullMode::None` is *not* a free one-line win. `pbr_showcase.rs:38,71`, `grand_showcase_2mat.rs:22,51` and `grand_showcase_mvpm.rs:40` all state that the procedurally generated spheres' and quads' winding is non-load-bearing **because** the raster is `CullMode::None`, and ≥7 pipelines in `gpu_scene/mod.rs` are set that way. Enabling it holes every scene our goldens render. Real cost: fix every generator's winding + re-bless the corpus.

---

**L7 — HZB + two-pass occlusion, with an *instrumented* second-pass yield.**
- **Axis:** cluster cull false-negative rate.
- **Mechanism:** mip-chain reduction over the depth we already write; pass 1 vs previous-frame HZB, rebuild, pass 2 vs current-frame HZB.
- **Magnitude:** second pass measured ~0.06 ms + 0.03–0.05 ms pyramid (MEASURED, Bevy/RTX 3080) — so *deleting* it is worth almost nothing. The value is pass-1 hit rate. Published failure mode: conservative min/max downsampling punches holes in occluders (a wall with a window loses most of its occlusion power).
- **Cost:** one reduction pass + one image ResId appended; a second raster pass declaration. **Our depth is already a separate readable D32 reverse-Z attachment**, so the HZB source costs no extra write — that is a genuine structural convenience.
- **Risk:** **correction to the surveys** — "same-frame HZB beats Nanite's previous-frame HZB" is **false**. Nanite's pass 2 already tests against the current frame's HZB; the previous-frame HZB is only the pass-1 guess. A design with *only* a same-frame HZB has nothing to test against before the first raster. Do not sell this as an edge.

---

**L8 — Attack the fixed-cost floor, not the peak.** *(§2B)*
- **Axis:** frame cost on low-complexity scenes — the "any scene" requirement.
- **Mechanism:** measure the pipeline's cost at low cluster counts and structurally collapse to a plain indirect draw (skip HZB, skip second pass, skip any SW path) below measured thresholds, with hysteresis.
- **Magnitude:** the qualitative asymmetry is well supported — software/compute rasterization loses at low geometric density and wins only at extreme density (CuRast, direction only; the specific ratios quoted in the surveys were transcribed wrong and are struck). Epic documents Nanite's own loss cases. **No usable multiplier exists; this must be measured locally.**
- **Cost:** a second simple arm + switching policy + hysteresis; a nearly-free CPU-side classification (not a GPU readback); both arms golden-gated.
- **Risk:** a visible discontinuity at the switch (different raster ⇒ different pixel tie-breaks). The switch must be provably pixel-identical or gated by a quality-neutrality test.

---

**L9 — Variable / per-mesh / per-DAG-level cluster granularity.** *(a degree of freedom Nanite cannot reclaim)*
- **Axis:** cull effectiveness traded against vertex reuse and wave occupancy.
- **Mechanism:** choose triangles-per-meshlet at bake time per mesh and per DAG level; make the runtime wave mapping granularity-agnostic.
- **Magnitude:** the trade is monotonic and opposed — ACMR 0.75 → 0.57 and perimeter/triangle 0.45 → 0.13 going 32 → 256 vertices, against occlusion-cull effectiveness ~80% → 66% and backface 14% → 3% (MEASURED by zeux, but on synthetic grid meshlets and an unnamed "test scene" — **directional only, percentages not transferable**). Per-vendor optima are inverted and both are MEASURED: Steam Deck RDNA2 32/32 = 9.3 ms vs 256/256 = 12.8 ms; RX 7600 RDNA3 64/64 = 2.2 vs 256/256 = 2.7 ms (Arntzen). NVIDIA's *published guidance* is 64 verts / 126 prims. **Struck from the record:** the claim that "NVIDIA measured best at 32/32" — that is the Steam Deck row, measured by Arntzen on AMD.
- **Cost:** builder support + per-cluster header + a granularity-agnostic runtime. Offline side is nearly free (meshopt `buildMeshletsFlex`/`Spatial` expose min/max/split_factor).
- **Risk:** we have **one GPU**. Any AMD/Intel arm ships unmeasured, which the surveys themselves call "a lie". Constrain this to *per-mesh/per-level* variation (measurable here) and explicitly **do not** ship per-vendor arms until hardware exists (§6 Q3).
- **Our structural advantage is real:** Nanite's cluster size is pinned by its visbuffer packing. We have no shipped cluster format, so we are not pinned. **Struck:** "128 tris / ~384 vertices" as a vertex budget — 384 = 128×3 is the *non-indexed* vertex count of a HW draw in a RenderDoc capture, not a unique-vertex capacity. Nanite's true per-cluster unique-vertex cap is **UNVERIFIED**.

---

**L10 — Treat the bake as a systems problem.**
- **Axis:** bake wall-clock and peak RAM (iteration speed and CI cost, **not** frame time).
- **Mechanism:** `boyko_threadpool` + VM-reserved arenas + sparse (indexed-only) initialization instead of full memsets + per-thread allocation caches + descending-size work ordering + Morton pre-sort + SIMD box merging + **inner** (within-mesh) parallelism.
- **Magnitude:** zeux profiled exactly this and took 1.64B triangles from >30 minutes (7 threads) to **~2m35s** (16 threads, Ryzen 7950X), with **no change to the DAG algorithm** — itemized: sparse init ~2.7×, SIMD box merge ~9%, Morton sort ~5%, per-thread allocation caching ~3.5% on Linux and larger on Windows (**MEASURED**). Windows — where allocator contention was worst — is our primary platform, and this is the single best fit between an external result and this engine's existing machinery.
- **Cost:** medium if we own the clusterizer; low-medium if we own only the parallel driver, arenas and layout.
- **Risk:** it buys iteration speed, not the stated goal. Rank it accordingly.

---

**L11 — LOD-cut stability as a first-class metric; temporal cut caching as the mechanism.**
- **Axis:** image quality per ms, and (secondarily) cut-selection cost.
- **Mechanism:** an engine-side **cut-churn counter** (clusters entering/leaving per frame) reported alongside ms; then per-instance cut caching with a conservative error band, a bounded re-traversal budget, and forced full re-walk on transform change / streaming residency change / camera cut.
- **Magnitude:** **unprecedented — no published Nanite-class system does this**, so there is no measured precedent and it must be gated by our own A/B against a full-traversal arm. The *quality* justification is documented: LOD topology switches invalidate temporal reuse, serious enough to warrant a dedicated SIGGRAPH paper for ReSTIR (AUTHOR-CLAIMED). We are unusually exposed: we ship TAA, a dedicated `vb_geo_mv` motion-vector shader, and DDGI probes. A cluster LOD switch changes topology, so the previous frame's (instance, meshlet, triangle) triple may not exist — motion vectors need an explicit policy.
- **Cost:** cut state must be a **Resource-owned GPU column** (the consumer is a compute shader), not a CPU-side dense component; plus an invalidation scheduler.
- **Risk:** a stale cut that is too coarse is a visible pop; correctness across *stitched* instances is unverified. The project's recorded recurring bug fingerprint is "wrong only in motion" — this lever lives in that exact hazard class.

---

**L12 — GPU-decodable cluster block format, decoder emitted by `boyko_shaderdsl`, byte-gated against a host oracle.**
- **Axis:** resident bytes/triangle → clusters within the residency budget.
- **Magnitude:** bounded by AUTHOR-CLAIMED figures on both sides (§2D). **Counterweight, MEASURED-ish:** AMD's own course reports that with GTS + quantization, raw rendering performance *generally declines* relative to uncompressed meshlets — the win is recovered only via the culling the freed bandwidth enables (GPU model not named).
- **Cost:** medium. This is the one lever whose verification discipline already exists and ships (the `*_edsl_sync` / `*_spv_sync` re-DXC gates, f32-vs-`Emit` dual instantiation).
- **Risk:** format churn is expensive once assets exist — argues for doing it before any assets ship, which fights the schedule. Bit-exactness between host encoder and GPU decoder is a known corruption source.

---

**L13 — Software rasterizer.** *(DEFERRED — ranked here deliberately, not omitted)*
- **Axis:** raster of sub-pixel triangles.
- **Why it is not near the top:** the justification is unproven in both directions (§2F), and the engine prerequisites are severe and unpriced by any survey:
  - **REPO-VERIFIED:** shaders target `cs_6_0`/`vs_6_0`/`ps_6_0` with a single `cs_6_5` precedent under `hwrt`. 64-bit atomics need **SM 6.6** — an unprecedented target bump in this tree. `build_tlas_instances.comp.hlsl` states plainly "no uint64_t, no shaderInt64". No wave/subgroup intrinsics anywhere; `subgroup_size_control = VK_FALSE`, `compute_full_subgroups = VK_FALSE`.
  - **REPO-VERIFIED:** `VkPhysicalDeviceVulkan12Features` is never chained at device creation, and core `shaderInt64` is not requested. So `shaderBufferInt64Atomics`, `drawIndirectCount`, `scalarBlockLayout` and `vulkanMemoryModel` are **all currently off**. Turning them on touches boot for all four render paths.
  - `VK_EXT_shader_image_atomic_int64` support on our specific driver is **UNVERIFIED** (`sparseImageInt64Atomics` is a separate bit). Nothing in the repo probes int64 capability.
  - **The depth collision no survey priced:** our separate D32 reverse-Z attachment is a hard contract with at least four consumers that never read `vb_id` — `viewt_from_depth_rz` (the gViewT producer for TAA under VB×Mesh), the `VB_THIN` SSAO arm, `sdf_forward_march`'s mesh-depth decode that composites the SDF leg on **every** VB×Both frame, and the depth-composite chain. A SW rasterizer forces an `EmitDepthTargets` equivalent and perturbs all of them — a blast radius far larger than the `vb_id` re-encode that was carefully analysed.
  - **Struck:** "ours starts ~60% built." The eDSL's `vb_barycentric`/`vb_uv_grad`/`vb_interp` pieces are the *attribute* half and already run downstream in the resolve. A SW rasterizer emits only depth+id per covered pixel; it needs edge functions, fixed-point setup, a top-left fill rule and tile binning — none of which is analytic barycentric code. `vb_near_clip` is genuinely reusable.
  - **Struck:** the "texture-vs-buffer atomics free win nobody has banked." The 0.68 ms clear was a **wgpu artifact** (`vkCmdCopyBufferToImage` instead of a clear); our raw-FFI RHI does not have that layer, so there is no hole to bank.
- **Correctness gates no survey stated:** exact fill rule, consistent subpixel snapping, watertightness across adjacent clusters. Karis himself concedes Nanite's 64-pixel coverage clamp tears at close range and that near-plane triangles are culled rather than clipped. A SW raster needs an **SW-vs-HW pixel-identity oracle on large-triangle content plus an adjacent-cluster watertightness test** as a gate *before* any performance work.
- **Decide it with our own A/B, not with literature.** We already ship a HW-raster VB path. Once L5 produces clusters, routing the same post-LOD cluster set through both is the single cheapest way to settle the biggest and most expensive fork in the design — and nobody in the world has published that A/B.

---

**L14 — Persistent-thread BVH traversal.** *(parity, HIGH risk, deferred)*
Vulkan gives no inter-workgroup forward-progress guarantee; device-scope acquire/release additionally wants `vulkanMemoryModel`, which we do not chain (two stacked unspecified behaviours, not one). **Struck:** "we already own a Chase-Lev pool" — that is a CPU-side Rust structure over `core::sync::atomic`; none of it is reusable on the GPU. Nanite itself keeps a non-persistent fallback toggle; copy that shape, and only after L2/L7 prove the plumbing.

**L15 — `VK_EXT_device_generated_commands`.** Cross-vendor since Vulkan 1.3.296 (survives the portability test), but three layers above a foundation we do not have, and MEASURED bound says its GPU draw time is **no better** than a re-recorded command buffer (EXT 7.0 vs 6.9 ms unsorted; 1.4 vs 1.5 ms sorted, RTX 6000 Ada, nvpro stress scene) — the win is CPU-side, at up to 436 MB of preprocess buffer. **Not a geometry-throughput lever.** Defer.

**Struck entirely, do not re-propose:** `VK_KHR_fragment_shader_barycentric` attribute-fetch (we already export only `SV_Position` + one flat `instance_id`; there is no hardware interpolation to remove — pure cost, no target). Task/amplification shaders (measured loss on AMD, parity on our hardware, and we have no mesh-shader binding at all — a written constraint, not work). Work graphs (§2G). DDGI/SDF reuse for an aggregate tier (octahedral probes store irradiance, not coverage; and the SDF leg composites into `gLit` *after* `vb_resolve` — it is architected **not** to write the visibility buffer).

---

### The `vb_id` re-encode, correctly stated

The **decode** side is a one-line change: **REPO-VERIFIED**, `vb_geom_fetch.hlsli:521` is exactly `uint local_tri = raw_prim_id % tri_count;`, and the 64-bit `R32G32_UINT` width was chosen deliberately *because* there was no meshlet system — so a meshlet re-encode costs **no format change and no extra bandwidth**. That part is real and it is a genuine advantage over a system pinned by its packing.

But the **encode** side is not independently reachable. The G lane is filled by `SV_PrimitiveID`, a fixed-function rasterizer system value; a VS/FS pipeline has no way to author a meshlet id into it. Getting one there requires *one of*: a mesh shader emitting a per-primitive attribute (`VK_EXT_mesh_shader` — absent), one draw per meshlet (the measured-slowest option), or a software rasterizer that writes the id itself. **The re-encode is downstream of the raster-path decision, not independent of it.** And the R lane is not spare: `instance_id` is a dense index addressing the `VbInstanceRow` SSBO, the `PerInstanceMaterial` ring, and (via the row's `mesh_id`) the bindless geometry table.

Blast radius, as corrected by the repo survey: **four** HLSL sources include `vb_geom_fetch.hlsli` (not eight); **eight** sources touch the encoding; **sixteen** committed `.spv` are perturbed. This paragraph closed with a prescription — *"only ten have a re-DXC byte-identity gate; `vb_raster.vs`, `vb_raster.fs`, `vb_geo`, `vb_geo_mv`, `vb_classify_count`, `vb_classify_scatter` would drift silently. Write those six gates as a byte-neutral rung before touching the encoding."* **That rung is done, at `598f4ff`** (`crates/boyko_rhi_vulkan/tests/vb_raster_geo_classify_spv_sync.rs`), so all sixteen are gated and the prerequisite is discharged rather than pending. One qualification the original did not need and this one does: both gate files SKIP where no `dxc` resolves, so the coverage holds on a host carrying the pinned VulkanSDK 1.4.350.0 and a skipped run proves nothing.

Also: the existing classify chain's scan is one 256-thread workgroup looping blocks serially through a groupshared carry, plus a serial `group_to_mat` fill. Correct and cheap at material cardinality (a few hundred); **catastrophic at meshlet cardinality (10⁵–10⁶)**. Reusing it as-is is the single easiest way to make the meshlet path *slower* than today's CPU draw loop. And its region offsets are fixed at `VB_MAX_MATERIAL_ROWS` as a deliberate sync pin — rescaling is a four-shader contract change, not a parameter.

---

## 4. The staged ladder

**Rung 1: `VG-R0 — "The Ruler"`.**

**What it is:** a high-poly ingest path + a density census + a reference measurement — and **no render change whatsoever**.

1. A glTF (or equivalent) importer and a licence-clean high-poly test corpus, plus the beginnings of a bake artifact format. *(Today the only importer is OBJ; every golden renders five procedurally generated UV spheres or a small room at 512×512. There is no bake stage. This is the actual first blocker and no survey named it.)*
2. A **density census**: for that corpus at the intended camera paths, report the screen-space triangle-size histogram and triangles-per-pixel at the intended error target — measured through the *existing* VB path, from engine counters.
3. A **Nanite reference capture**: UE5, same GPU, same resolution, same corpus, `r.Nanite.MaxPixelsPerEdge` pinned, per-pass ms recorded with the pass names documented.
4. A **decidability statement** for the harness: clock pinning, n-of-m sampling, confidence intervals, and the smallest delta it can resolve.

**The ONE gate:** *A reproducible per-pass Nanite cost table exists — named GPU, named scene, named resolution, named error target — together with a stated decidability floor smaller than the delta we intend to claim.*

**Why this is the falsification-first rung:** it can kill the campaign for free, three separate ways, before a single line of cluster code exists.
- If the density census shows our target content never approaches ~1 triangle/pixel at the intended error, cluster LOD has **no mechanism of action** on our content and the campaign is refuted.
- If a Nanite baseline cannot be produced, "beat Nanite" is unfalsifiable and the goal must be restated as an **absolute** ms/quality target — a scope change the owner should make consciously, not discover in month six.
- If the harness cannot resolve the intended delta, every future result will be arguable and the campaign will relitigate its own numbers. *(This is not hypothetical: this project has already recorded a bench that does not reproduce above N=128 with ~21% spread, and shipped a "22×" result measured inside that regime.)*

**Independently committable:** yes — an importer, a harness, a corpus and a docs table. Zero shader change, zero `.spv` change, **byte-identical goldens**.

**The ladder after R0:**

| Rung | Content | Gate | Golden impact |
|---|---|---|---|
| **R1** ✅ **LANDED** | L1 indirect seam + `GpuStage::Indirect` barrier fix | clippy/tests green; `barrier.rs` superset test updated with rationale | none — **5 pins re-measured byte-identical across all four render paths** |
| **R2** | L2 per-instance GPU cull → indirect draw | ⚠️ **its stated gate was UNSATISFIABLE, and three of its premises were FALSE — re-scoped into R2a′/R2c0/R2c below** | byte-identical if cull is conservative-exact |
| **R2a′** ✅ **LANDED** (`d12e9ff`) | The indirect draw seam under a REAL barrier: device-local `VkDrawIndexedIndirectCommand` records filled by an inline `vkCmdUpdateBuffer`, `TRANSFER → DRAW_INDIRECT` **derived by the framegraph** *(R2c0 re-sourced that edge onto the cull's `COMPUTE` write — the graph tracks the last writer, so no declaration changed)* | 9 pins byte-identical across all four render paths + **validation-layer clean** on the VB pin (indirect draws carry VUIDs ordinary draws do not) | none |
| **R2c0** ✅ **LANDED** | Compaction + count buffers, provably **INERT** — the batch-cull compute pass, its atomic-append visible list and its counter, dispatched every VB frame and changing nothing. **The NULL CONTROL** `VG-DECIDABILITY-FLOOR.md` says every later delta needs | 9 pins byte-identical + validation-clean, **an artifact-level inertness census** (`vb_batch_cull_spv_sync.rs`: `OpSelect == 0` ⇒ the decision folded away, `OpAtomicIAdd == 1` ⇒ the machinery is present, so neither half can be vacuous), **and the derived barrier chain asserted from the sync algebra** (`vb_indirect_barrier_chain.rs`: WAW upload→cull then RAW cull→raster, with a sensitivity control — a golden cannot see a missing buffer dependency, and neither can the validation layers) | none |
| **R2c** ✅ **LANDED (VB site)** | Per-batch, all-or-nothing **camera** cull, armed on the VB raster. Host-computed per-batch world AABBs through the Arvo transform SHARED with the CSM caster fit; six frustum planes extracted host-side **from the raster push's own 64 `view_proj` bytes** and pushed to the shader, so oracle and GPU read identical numbers | 9 pins byte-identical WITH the cull armed + validation-clean; the `.spv` census RE-PINNED from R2c0's inertness to the armed decision (`OpSelect` 0→1, `OpDot` 0→2, `OpFOrdLessThan` 0→1) while `OpAtomicIAdd` HOLDS at 1 — a cross-rung invariant that the arming did not disturb the compaction; 7 host-oracle tests incl. a 2000-box sweep against the 8-corner definition and a batch-behind-camera → 0 case | none on existing pins |
| **R2c-tail** ⚠️ **OPEN** | The gates R2c could NOT close, recorded rather than waved past: (a) **no GPU-side proof that the cull ever culls** — every pinned scene is entirely on-screen, so an armed-but-inert cull is byte-identical to a correct one, and the "it culls" evidence is currently HOST-ONLY (the oracle test); (b) the 3 remaining camera sites (`forward.rs` ×2, `gbuffer.rs`) are untouched — only the VB raster is armed | **one NEW pin with genuine off-screen geometry**, plus a `vb_cull_count` readback compared against the host oracle (the compaction buffers R2c0 built exist precisely to make that comparison possible, and still have no consumer) | new pin |
| **R2b** ✅ **LANDED** (`598f4ff`) | Write the six missing re-DXC gates | `vb_raster_geo_classify_spv_sync.rs` (6 rows) + `vb_lit_producer_spv_sync.rs` (10 rows) — **exact complements, 16 distinct `.spv`, no overlap** (verified against the two row tables, not against this text). Sensitivity-asserted by `vb_raster_fs_redxc_is_sensitive_to_a_swapped_vb_id_lane` and `vb_lit_producer_redxc_is_sensitive_to_an_untouched_literal`. ⚠️ Both files SKIP by design where no pinned `dxc` resolves, so a green run is not evidence they RAN — **re-verified 2026-08-01 executing (no skip message) on the pinned VulkanSDK 1.4.350.0 host** | none |
| **R3** | L7 HZB + two-pass occlusion, second-pass yield **instrumented** | measured pass-1 hit rate + second-pass marginal yield on our scenes | new pins for the HZB arm |
| **R4** | L4 DAG-quality harness + L5(a)(b)(c) builder | triangles-at-error curve improves vs a baseline builder, monotonicity verifier green | bake-only, none |
| **R5** | Meshlet metadata as a 4th Set-2 binding (mesh cardinality) + meshlet cull, **dark infra** (Option stays `None`, seeds declared correctly at *build* time not *arm* time) | byte-identical goldens | none |
| **R6** | Arm the meshlet cull + `vb_id` re-encode | owner visual bless of 9 mesh-leg pins | **all 9 move** |
| **R7** | **The fork:** in-house A/B of the R6 cluster set through HW raster vs a SW raster prototype, with the watertightness + SW-vs-HW pixel-identity oracle as a *precondition* | decided by our numbers, not literature | prototype behind a flag |
| **R8** | L8 fixed-cost floor collapse; then L11 churn metric; then material resolve / aggregates | per-rung | per-rung |

> ⚠️ **R2's GATE CITES A FLOOR THAT DOES NOT EXIST, AND THE FLOOR IS NOW MEASURED TO BE A MOVING
> TARGET.** *"Measured Δ … decidable by R0's floor"* names something R0 never produces: the
> decidability apparatus left `VG-CAMPAIGN-THRESHOLDS.toml` at Rev 8, and the R0 plan states in its
> own words that *"R0 builds no harness and measures no delta, so there is nothing here for K3 to be
> true or false about."* **Every rung from R2 down inherits that** — R3's hit rate, R4's curve, R7's
> *"decided by our numbers"* are all measured deltas with no stated resolution.
>
> [`VG-DECIDABILITY-FLOOR.md`](VG-DECIDABILITY-FLOOR.md) is that missing measurement — K3's test,
> run as a **null experiment** (same bench, same scene, same configuration, separate processes, so
> every observed difference is instrument plus environment). Its result is not a threshold:
>
> **Four runs of the same protocol on this box reported floors of 6.3 %, 14.3 %, 4.7 % and 13.5 %.**
> Identical-protocol pairs differ by ~3×. Changing the statistic did not fix it and tripling the
> sessions did not fix it. **The floor drifts faster than the gap between two measurements of it.**
>
> So the rule that replaces the number: **a claimed GPU-timing delta below ~15 % is not defensible
> on this box without a NULL CONTROL measured in the same sitting.** That fully explains the failure
> this document already records — a *"22×"* result measured inside a regime that *"does not
> reproduce"* — and it makes R2's gate unsatisfiable in **both** directions at once, because R2's own
> expected magnitude here is stated above as **"near zero"**.
>
> **R2 is still worth building; its GATE is what needs replacing.** Its value, as this document
> already says, is that it *"de-risks cull-pass declaration, compaction, indirect barriers and count
> buffers before any meshlet exists"*. Those are all **correctness** properties — byte-identical
> goldens under a conservative-exact cull, a cull that provably drops no visible instance, the
> indirect plumbing existing and being exercised — and none of them needs a decidable delta. A rung
> gated on a delta it cannot resolve would either red forever or be blessed on noise.

> ⚠️ **AND THREE OF R2's PREMISES WERE FALSE IN THIS ENGINE.** Verified by opening the files before
> writing a line of it, which is why the re-scoping above exists at all:
>
> 1. **`instanceCount` is a PREFIX, not a mask.** Every VS reads
>    `instances[pc.base_instance + SV_InstanceID]`, so lowering `instanceCount` from N to K draws the
>    **first K of the bucket**, not the K visible ones. If instances {2,7,9} of 10 survive, no
>    `instanceCount` expresses that. *"Overwrite `instanceCount` from a frustum test"* therefore
>    **cannot be a per-instance cull** — it is all-or-nothing per batch, which is what R2c is.
> 2. **`multiDrawIndirect` is not enabled** (`device.rs` enables only `samplerAnisotropy`), so
>    `drawCount` must be 0 or 1. R2's stated endpoint *"one indirect draw"* additionally needs
>    `drawIndirectCount`, `vkCmdDrawIndexedIndirectCount` (deliberately unloaded) **and** a merged
>    vertex/index arena that does not exist — all ten draw loops rebind a per-mesh VB/IB with a
>    per-mesh index width. **That endpoint is gated on an arena rung, not on a cull, and has left R2.**
> 3. **There is no per-instance mesh id on the GPU** — the `mesh_ids` lane is documented
>    *"Host-side only; the raster draw does not read it"*, and `GBufferMeshDraw` carries no
>    `mesh_id`. A cull cannot look up per-mesh bounds by id, which is why R2c's bounds are
>    **host-computed per-batch world AABBs** rather than a per-mesh GPU table.
>
> Also recorded rather than smuggled: **true per-instance culling** needs the instance ring
> compacted across **both** lanes in lock-step (or an indirection lane), edits to six vertex
> shaders, and a golden re-bless. Its own rung, budgeted honestly — not a sub-clause of R2.
>
> One trap worth stating because nothing would catch it: `drawIndirectFirstInstance` is **also**
> `VK_FALSE`, and the validation layers **cannot read buffer CONTENTS**. A non-zero `firstInstance`
> in a record is therefore silent corruption, not a caught error. R2a′ asserts it host-side, and
> R2c0's shader deliberately writes **only** word 1 of each record so the invariant stays inside
> the reach of that assert.

**One-way door to decide now, before the first file:** "cluster" is already taken in this codebase and means **light froxel** (`cluster_cull.hlsl`, `cluster_cull_spv_sync.rs`, `ClusterGrid`, `MAX_LIGHTS_PER_CLUSTER`, the whole VB-P1e "22× at 512 lights" campaign). Use **`meshlet`** for the leaf and **`geo_group`** for the DAG group; leave `cluster` to lights. Decided, not asked.

---

## 5. The killers — falsifiable tests, run early

**K1 — No content, no mechanism.** *Test (R0):* density census. If the target corpus stays below ~1 triangle/pixel at the intended error target, cluster LOD has nothing to do. **Kills the campaign.**

**K2 — No baseline.** *Test (R0):* produce the Nanite reference table. If it cannot be produced, the goal is unfalsifiable as stated. **Forces a scope restatement** (absolute target instead of relative).

**K3 — Undecidable harness.** *Test (R0):* state the resolvable delta with CIs. If it exceeds the delta we intend to claim, **no result from this campaign is defensible.**

**K4 — The feature floor is not payable.** *Test (one boot-only rung):* chain `VkPhysicalDeviceVulkan12Features`, request `shaderInt64` + `shaderBufferInt64Atomics` + `drawIndirectCount`, probe `VK_EXT_shader_image_atomic_int64`, boot all four render paths and re-run the golden corpus. If boot regresses or goldens move, **the SW-raster and indirect-count branches are off the table** and the design collapses to HW raster + per-instance/per-meshlet culling.

**K5 — Builder nondeterminism vs the byte-golden regime.** *Test (R4):* build the same mesh at 1 / 4 / 16 threads and compare artifact hashes. Partitioners and parallel simplifiers are order-, seed- and thread-count-sensitive (Bevy pins METIS `seed = 17` for exactly this). If the builder cannot be made deterministic, **the cluster data cannot be byte-gated** and the project's entire verification discipline stops covering the largest new subsystem.

**K6 — In-house builder cannot reach the quality bar.** *Test (R4, against the L4 harness):* if an in-house simplifier's triangles-at-error curve is materially worse than a reference and the gap does not close within budget, **the main headroom (§2A) is gone** — and with it the strongest reason to believe this beats Nanite anywhere.

**K7 — Temporal cost exceeds the ms win.** *Test (R6+):* cut-churn counter + owner eval under TAA and DDGI. If LOD-cut churn produces artifacts that cannot be damped within the triangle budget, the mechanism costs quality that ms does not capture, and the comparison against Nanite becomes ill-posed again.

**K8 — Golden bless throughput.** *Test (now):* count byte-moving rungs per week the owner can actually bless. Pins are `#[ignore]` screenshot dumps requiring a real windowed device and human sign-off, with separate software and hwrt legs — and **REPO-VERIFIED, `vb_both_sdf` and `vb_both_sdf_tex` are already sitting at the literal `PENDING` sentinel**. The VB corpus is not fully green today. A byte-moving campaign that starts on an unfixed baseline will not converge.

---

## 6. Open questions for the owner (VALUES / SCOPE only)

1. **Third-party dependencies.** May the offline builder link `meshoptimizer` / METIS / a hypergraph partitioner, or must it be in-house? The workspace today pulls only criterion, proptest, bytemuck, windows-sys, libc, loom — the demonstrated posture is fully in-house (raw-FFI Vulkan, in-house PNG/zlib, in-house physics). In-house means owning exactly the part the literature calls least-solved. **This is the single biggest schedule fork in the document.**

2. **Content-capability cut for v1.** Is "no alpha-tested foliage and no WPO on the virtual-geometry path" acceptable? Keeping the rasterizer free of all material logic is what makes Epic's largest documented cost centre structurally unpayable for us — but it is a content cut, not a perf choice.

3. **Hardware scope.** We have one RTX 3060. Do AMD/Intel arms count as shippable if unmeasured, or is the target explicitly single-vendor until hardware exists? This decides whether L9 ships per-vendor arms or per-mesh variation only.

4. **Test corpus and reference rig.** Who provides / licenses the high-poly corpus, and is a UE5 install on the measurement box acceptable for the reference capture? Without both, K2 fires at rung 1.

5. **Quality target.** What pixel-error budget counts as "equal quality" (our equivalent of `MaxPixelsPerEdge = 1.0`), and is the owner the arbiter via visual eval, or do we bind to a metric? Note the standing lesson that image statistics have already misled this project twice.

6. **Bless bandwidth.** How many byte-moving rungs per week can be blessed? That number caps rung width and therefore the ladder's shape.

7. **Aggregate geometry in scope?** Epic abandoned triangle clusters for voxel aggregates in 5.7 for exactly the "hard scene" class the goal names. There is **zero performance data on this anywhere in the evidence set**. Including it means committing to an unresearched area; excluding it means "any scene" has a stated exception.

---

### One-paragraph verdict

Start. The direction is sound, and this engine's existing shape — a pure visibility buffer with no G-buffer materialization, a fused compute resolve, eDSL-generated shaders with host-oracle byte gates, VM-native storage, and a full-width `vb_id` that was never pinned by a packing — is unusually well-placed for the parts of Nanite that are *not* near-optimal. But the win does not come from where the folklore says it does. It is **not** in the rasterizer (unproven in both directions, and the "3×" that justified it is dead). It is **not** in cluster culling (that headroom belongs to Bevy; Nanite already has BVH plus persistent threads). It is in **offline DAG quality**, which multiplies every runtime pass on every scene; in the **fixed-cost floor**, which is the only honest route to "any scene"; and in **material resolve**, where we already start on the side Epic had to engineer back toward. And before any of it: build the ruler, because right now the target has no number.