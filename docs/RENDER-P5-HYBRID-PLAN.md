# Render P5 — Mesh-First-Class-Citizen Hybrid Composite (LOCKED PLAN)

**Branch:** `ecs` · **Status:** LOCKED (post architecture-critic convergence) · **Owner sign-off pending on Owner-call #1 only**

The marcher in scope is `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl`
(the production P1b MRT G-buffer composite), its host driver
`crates/boyko_rhi_vulkan/src/compute.rs`, and the coarse-cull producer
`crates/boyko_rhi_vulkan/shaders/sdf_tile_cull.hlsl`. The deferred resolve
(`crates/boyko_rhi_vulkan/shaders/deferred_pbr.hlsl`) is a READER of the G-buffer
and is NOT edited by P5.

---

## CHANGELOG

| Rev | Change | Why |
|-----|--------|-----|
| r1 | Initial P5 charter: mesh becomes a first-class citizen via a per-pixel SDF/mesh **ownership gate** in the marcher (B1), driven by the rasterized-depth `has_mesh` flag (B2); barrier/gViewT-survivor accounting (M1); M2 binding-10 R2 note; rung decomposition; perf model. | Foundation. |
| r2 | Critic round 1: anchored B1 to BOTH clobber sites (not one), proved B2's reasoned conservatism + added the tripwire, anchored M1's gViewT survivor to the 3-terminal-exit contract, pinned W4's blob, named the M2 binding. | Make every claim source-grounded. |
| r3 | Critic round 2: dispositions accepted. ONE substantive item left open — **W1↔W2 internal consistency** (flat-prologue-frozen vs indirect-tile-dispatch could not both be true) — plus the process fix (plan must be a committed file, not agent prose). | Convergence gate. |
| **r4 (this rev)** | **RESOLVED W1↔W2 with Option (i): KEEP the flat marcher prologue byte-frozen; DO NOT elevate indirect tile-dispatch into P5.** Demoted the indirect classify→compact→`DispatchIndirect` step to a chartered **follow-up P5.2**. P5's only `.comp.spv` delta is now the **B1 terminal ownership gating** (NOT a prologue index remap). Corrected Decision 5, W2's disposition, and OQ-D accordingly. Plan is now internally consistent. **Also: wrote the plan to disk** (`docs/RENDER-P5-HYBRID-PLAN.md`) so the critic can issue APPROVED against a committed artifact. | Resolve the one open item + the process fix. |

---

## Goal

Make the mesh a **first-class citizen** of the hybrid SDF+raster pipeline: on a pixel
the raster G-buffer already owns (a real mesh fragment was rasterized in front of any
SDF surface), the SDF marcher must **not** write that pixel's G-buffer lanes — the
raster owns them. Today the marcher unconditionally clobbers all four G-buffer lanes
for every pixel it processes (lines 1321-1329 on the empty-tile arm, 1644-1653 on the
terminal arm); the mesh arm only happens to agree because `base = MESH_COLOR` is
re-derived inside the marcher. P5 replaces that "marcher re-derives the mesh color"
coupling with a clean **per-pixel ownership predicate**: the raster owns mesh-covered
pixels; the marcher owns the rest. The mesh stops being a value the marcher reconstructs
and becomes a producer the marcher yields to.

**Performance content of P5 (the real win):** on a mesh-covered pixel the marcher does
**zero** G-buffer storage stores (4 image stores × pixel-covered-count saved per frame)
and, with the B1 own-pixel short-circuit, also skips the per-pixel material attribution
(`pick_material_id`, an O(edit_count) argmin) and the A1/A2 shadow/AO marches that a
mesh pixel would otherwise pay if `t_mesh` did not bound them early. The correctness
content is the ownership invariant: exactly one producer writes each G-buffer texel per
frame.

**Target metrics (mesh-dense target frame, 1080p-class, the offscreen + windowed
goldens' scene scaled up):**
- G-buffer storage stores on mesh-covered pixels: **4 → 0** per covered pixel.
- Marcher work on a mesh-covered pixel: bounded to ray-gen + depth fetch + the
  ownership test + the gViewT sentinel store — **no march loop, no `pick_material_id`,
  no A1/A2** (the own-pixel gate short-circuits before the sphere-trace).
- `.comp.spv` delta: **terminal-gating only** (the prologue stays byte-frozen).
- 0%-gate: on a **no-mesh** scene `has_mesh` is always false ⇒ `own_pixel` is always
  true ⇒ the marcher's writes are **byte-identical** to today (the gate is a no-op).

---

## Context and constraints

- **Determinism boundary is INVIOLABLE.** The field math (`sdf_field.hlsli`), ray-gen
  (`ray_gen.hlsli`), and the host golden (`golden_composite_pixel_ex` in compute.rs)
  must stay byte-identical. P5 touches only the marcher's **terminal write arms** and a
  host-side mode flag — never the field, never ray-gen, never the march loop body.
- **The flat dispatch is the frozen contract.** The marcher is a FLAT 1-D compute
  dispatch: `[numthreads(64, 1, 1)]`, `idx = tid.x`, `px = idx % w`, `py = idx / w`
  (lines 1256-1269), guarded by the `idx >= count` over-hang early-out (lines
  1258-1261). `compute.rs` documents "the 1D dispatch index maps to (px,py)" row-major.
  P5 does **not** change this prologue (see Decision 5 / the W1↔W2 resolution).
- **gViewT exactly-once-per-real-pixel (C2).** `gViewT` (binding 8, R32_SFLOAT, lines
  229-241) is "WRITTEN AT ALL THREE TERMINAL EXITS, exactly once per REAL pixel per
  frame." The over-hang exit (`idx >= count`) is the ONE legitimate non-writing exit.
  P5 must preserve exactly-once: the ownership gate may **change which producer writes a
  pixel**, but it must not leave a real pixel's `gViewT` lane unwritten or double-written.
- **The M2 binding-10 R2 contract.** Binding 10 is a COMBINED image+sampler
  (`Texture3D<float> BrickAtlas` + `SamplerState BrickSampler`, lines 285-286). DXC keeps
  the static `register(t10)`/`register(s10)` refs past the runtime `brick_trilinear`
  branch, so the layout MUST declare binding 10 = combined-image-sampler and bind a VALID
  atlas+sampler even when gated OFF, or `vkCreateComputePipelines`/`vkCmdDispatch` trip
  the layout VUIDs (the M1 R2 lesson at t9). P5 adds NO binding; it does not touch this.
- **Affected subsystems:** `boyko_rhi_vulkan` (the marcher .hlsl + .comp.spv + compute.rs
  driver + the barrier/transition schedule), the offscreen golden tests, the windowed
  present test. NOT touched: `boyko_render`, `boyko_ecs`, the deferred resolve shader.

---

## Key decisions

### Decision 1: The per-pixel SDF/mesh OWNERSHIP gate (B1)

**What.** Introduce one boolean per thread, `own_pixel`, computed BEFORE any G-buffer
write. `own_pixel == true` means "the SDF marcher is the producer for this pixel's
G-buffer lanes this frame"; `false` means "the raster owns this pixel — the marcher must
not write its G-buffer lanes." The predicate:

```
own_pixel = !has_mesh || (sdf_hit && t < t_mesh)
```

- `!has_mesh` — no mesh fragment covers this pixel ⇒ the marcher is the sole producer
  (today's universal case on a no-mesh scene; this is what makes the 0%-gate trivial).
- `has_mesh && sdf_hit && t < t_mesh` — a mesh covers the pixel BUT an SDF surface is in
  FRONT of it (closer along the ray) ⇒ the SDF wins the depth test, the marcher owns it.
- `has_mesh && !(sdf_hit && t < t_mesh)` — a mesh covers the pixel and the SDF did NOT
  win (no SDF hit, or the SDF hit is behind the mesh) ⇒ **the raster owns it; the marcher
  writes NOTHING to gAlbedo/gNormal/gMaterial** for this pixel. (gViewT: see Decision 3.)

The gate is applied at **BOTH** marcher write sites (Decision 2). The current marcher's
`else if (has_mesh) { base = MESH_COLOR; }` arm (line 1636-1637) is exactly the
`!own_pixel` case re-expressed as "marcher re-derives the mesh color" — P5 replaces that
re-derivation with "marcher yields; the raster's G-buffer fragment, already present,
stands."

**Why (perf/cache).** On `!own_pixel`, the marcher skips 4 storage-image stores
(`gAlbedo`/`gNormal`/`gMaterial` + the gViewT sentinel becomes the only store) AND — by
short-circuiting BEFORE the sphere-trace when `has_mesh && t_seed-bounded` makes a
front-SDF impossible — skips `pick_material_id` (an O(edit_count) argmin over the edit
list, lines 534-553) and the A1/A2 marches (each up to MAX_IT field evals, lines
450-493). Storage-image writes are uncached streaming stores; eliminating them on every
mesh-covered pixel is a direct bandwidth saving on the dominant cost of a mesh-dense
frame. The raster already paid to produce those texels; re-writing them is pure waste.

**Why (correctness).** Today the invariant "exactly one producer writes each G-buffer
texel" holds only by the accident that the marcher's mesh arm re-derives `MESH_COLOR`
identically to what the raster would imply. That coupling is fragile: any divergence
(PBR material on the mesh, a textured mesh, a different mesh shading model) silently
corrupts the G-buffer. The ownership gate makes the invariant STRUCTURAL: the raster is
the single source of truth for mesh pixels; the marcher provably does not touch them.

**Alternatives rejected.**
- *Keep the marcher re-deriving MESH_COLOR (status quo).* Rejected: couples the mesh
  shading model into the marcher, blocks textured/PBR meshes, and pays 4 stores +
  attribution on every mesh pixel. The whole point of P5 is to break this.
- *Depth-test in a separate compositing pass.* Rejected: a second fullscreen pass reads
  back and re-writes the entire G-buffer — strictly more bandwidth than gating the
  marcher in place. The marcher already has `has_mesh`, `t_mesh`, and `t` in registers.

**Trade-off.** One boolean + one branch per thread on the marcher's terminal path. On a
no-mesh scene the branch is perfectly predicted (`own_pixel` always true) and the gate is
a no-op (0%-gate). On a mesh-dense scene the branch is coherent within raster-covered
tiles (mesh coverage is spatially clustered), so branch-predictor cost is low.

### Decision 2: The gate applies at BOTH terminal write sites (the 4-state table)

**What.** The marcher has TWO sites that unconditionally clobber the G-buffer lanes:

- **Site A — the EMPTY-tile early-return** (lines 1314-1331, taken only when
  `coarse_enabled != 0` and the tile is flagged EMPTY): writes `gAlbedo` (1321),
  `gNormal` (1322), `gMaterial` (1323), `gViewT` (1329), then `return` (1330).
- **Site B — the terminal write block** (lines 1644-1653, the universal fall-through):
  writes `gAlbedo` (1644), `gNormal` (1645), `gMaterial` (1646), `gViewT` (1653).

Both sites MUST honour the ownership gate, or a mesh pixel that happens to fall in an
EMPTY tile (Site A) would still be clobbered while a mesh pixel in a non-empty tile (Site
B) would be respected — an inconsistency that produces a visible tile-boundary seam.

At **Site A**, the existing `empty_color = has_mesh ? MESH_COLOR : BACKGROUND` (line
1320) already branches on `has_mesh`; under P5 the `has_mesh` arm becomes "the raster owns
it — do NOT write gAlbedo/gNormal/gMaterial; write only the gViewT sentinel" (the mesh is
necessarily in front because an EMPTY tile has no SDF surface, so `own_pixel` is false
whenever `has_mesh`). At **Site B**, the gate wraps the three attribute stores; gViewT is
handled per Decision 3.

**The 4-state ownership table (the complete case analysis):**

| `has_mesh` | SDF result | `own_pixel` | gAlbedo/gNormal/gMaterial | gViewT |
|------------|------------|-------------|---------------------------|--------|
| false | hit | **true** | marcher writes SDF attrs (mask=1) | marcher writes real `t` |
| false | miss | **true** | marcher writes BACKGROUND (mask=0) | marcher writes `1.0e30` sentinel |
| true | SDF hit in FRONT (`t < t_mesh`) | **true** | marcher writes SDF attrs (mask=1) | marcher writes real `t` |
| true | no front SDF (miss, or SDF behind mesh) | **false** | **marcher writes NOTHING — raster owns** | marcher writes `1.0e30` sentinel (Decision 3) |

The first three rows are byte-identical to today (the marcher's mask=1 / background /
SDF-in-front arms). The fourth row is the ONLY behavioural change: today the marcher
writes `MESH_COLOR` + the mask=0 G-buffer there (Site B line 1637, Site A line 1320);
under P5 it writes nothing to the three attribute lanes and lets the raster's fragment
stand. On a no-mesh scene the fourth row is unreachable (`has_mesh` always false), so the
output is byte-identical — the 0%-gate.

**Why.** Symmetry across both clobber sites is required for correctness (no tile seam).
Listing all four states proves the gate is total (no unhandled case) and that exactly
three of the four are unchanged — bounding the blast radius to the one mesh-owned arm.

**Trade-off.** The gate is duplicated at two sites. Both are tiny (`if (own_pixel) {
...three stores... }`), so the I-cache cost is negligible; the duplication is the price of
the EMPTY-tile fast path existing as a separate terminal exit (which the C2 gViewT
contract already requires).

### Decision 3: gViewT stays EXACTLY-ONCE — the gate changes the VALUE, never the WRITE

**What.** The ownership gate does **not** gate the `gViewT` store. `gViewT` is written
exactly once per real pixel at every terminal exit (the C2 contract, lines 229-241). On a
`!own_pixel` pixel the marcher still writes `gViewT`, but writes the `1.0e30` sentinel
(the same sentinel the mesh/background arm already writes today, Site B line 1653's
`(mask == 1.0) ? t : 1.0e30`). The resolve gates its `gViewT` READ inside `mask == 1`, so
the sentinel on a raster-owned pixel is never read — but it MUST be written to satisfy
exactly-once (an unwritten lane would carry the previous frame's `t` and, if a later
frame's mask flips, be read stale).

**Why.** gViewT exactly-once is a hard invariant (M1). Gating the gViewT store along with
the attribute stores would leave a raster-owned pixel's gViewT lane unwritten — exactly
the bug the EMPTY-tile site already documents it avoids (lines 1324-1328). The clean rule:
**the ownership gate controls the three ATTRIBUTE lanes (gAlbedo/gNormal/gMaterial); the
gViewT lane is always written, with the value selected by `mask`** (and `!own_pixel`
forces `mask`'s effective value to the non-lit sentinel). This keeps gViewT's exactly-once
contract structurally separate from the attribute-ownership gate.

**Trade-off.** A raster-owned pixel pays one R32_SFLOAT store it does not strictly need
(the resolve won't read it). This is one store vs the four it would otherwise pay — and it
is the price of the exactly-once invariant, which is non-negotiable. (P5.2 could fold the
gViewT clear into the raster pass, eliminating even this store; out of P5 scope.)

### Decision 4: B2 — `has_mesh` is the ownership driver; reasoned conservatism + a tripwire

**What.** The ownership predicate keys off `has_mesh = (md < DEPTH_CLEAR)` (line 1285),
where `md = gDepth.Load(int3(px, py, 0)).r` (line 1284) is the rasterized D32_SFLOAT mesh
depth and `DEPTH_CLEAR = 1.0` (line 417) is the far-plane clear the depth attachment was
cleared to. `has_mesh == true` iff a mesh fragment was rasterized strictly in front of the
far plane for this pixel.

**The reasoned-conservatism proof.** `has_mesh` is **conservative in the correct
direction** for the ownership decision:
- The depth attachment is cleared to exactly `1.0` (`DEPTH_CLEAR`), and a rasterized
  fragment writes `depth < 1.0` (any real geometry in front of the far plane). So
  `md < 1.0` is true **iff** a fragment was rasterized — no false positives (a cleared
  pixel reads exactly `1.0`, fails `< 1.0`) and no false negatives (a real fragment
  writes `< 1.0`). The single edge — a fragment landing exactly at the far plane
  (`depth == 1.0`) — is classified as "no mesh," which is SAFE: a far-plane-grazing mesh
  fragment is visually indistinguishable from background, and mis-classifying it as
  "no mesh" makes the marcher own it (it would just write BACKGROUND there, matching the
  raster's far-plane fragment within tolerance). There is no UNSAFE misclassification
  (a real near-mesh pixel can never read `>= 1.0`).
- `t_mesh = has_mesh ? (md * T_MAX) : 1.0e30` (line 1286) is the mesh's ray parameter.
  The depth test `t < t_mesh` (SDF in front) uses the SAME `md` value the ortho
  convention defines, so the front/behind decision is exact in the depth metric the rest
  of the marcher already uses (it is the same bound the existing march already respects).

**The tripwire (debug-build invariant).** To catch a future depth-convention drift (e.g.
a perspective depth nonlinearity, a reversed-Z change, or a clear-value change), add a
host-side GPU validation in the offscreen test: render a known single-mesh-quad-over-empty
scene and assert that **every** pixel inside the quad's raster footprint reads
`own_pixel == false` (mesh-owned) and **every** pixel outside reads `own_pixel == true`
(marcher-owned). A drift in the depth convention flips the gate and the footprint test
fails loudly. This is a test-only tripwire (zero runtime cost in release); it pins the B2
assumption to a runtime check.

**Why.** The ownership gate's correctness rests entirely on `has_mesh` meaning exactly
"the raster owns this pixel." The proof shows the only misclassification (far-plane
grazing) is in the safe direction; the tripwire converts the proof's assumption (the depth
convention) into a checked invariant so a future change can't silently invert the gate.

**Trade-off.** The tripwire is a test artifact, not a runtime guard — it catches drift in
CI, not in production. That is the right place: a runtime per-pixel assert would defeat
the perf win. The proof (not the test) is what guarantees production correctness.

### Decision 5: KEEP the flat marcher prologue byte-frozen — do NOT elevate indirect tile-dispatch into P5 (the W1↔W2 resolution)

**What.** P5 ships with the marcher dispatch UNCHANGED: the flat `[numthreads(64,1,1)]`,
`idx = tid.x`, `px = idx % w`, `py = idx / w` prologue (lines 1256-1269) stays
byte-frozen. The classify→compact→`DispatchIndirect`-over-a-compacted-TILE-ID-list idea is
**demoted to a chartered follow-up, P5.2** (see the rung table), and is explicitly OUT of
P5 scope. P5's only `.comp.spv` change is the B1 **terminal ownership gating** at the two
write sites (Decision 2) — NOT a prologue index remap.

**Why (the decisive reasoning, one paragraph).** A compacted tile-id list dispatched
indirectly necessarily implies a group→tile→pixel remap in the marcher prologue (a 2-D /
tiled index), which IS a prologue index-math change, which IS a `.comp.spv` re-pin AND a
re-domaining of the marcher (Owner-call #1 candidate (b)) requiring the runtime no-mesh
byte-compare to prove image-identity. That cost is NOT justified inside P5 because the win
it buys is already second-order: **the empty-tile MARCH cost is ALREADY eliminated today.**
The EMPTY-tile early-return (lines 1314-1331) composites mesh/background and `return`s at
line 1330 — BEFORE the sphere-trace loop at line 1348+. So on an EMPTY tile the marcher
still LAUNCHES the wavefront but returns before the O(steps × edits) march; that cost is
gone. What indirect dispatch would additionally remove is ONLY the empty-tile **wavefront
LAUNCH** overhead — a legitimately second-order term that is self-contained and does not
entangle with the mesh-first-class-citizen change. P5's real perf+correctness content is
the B1 ownership gate (4 stores + attribution + A1/A2 saved per mesh pixel), which needs
NO prologue change. Therefore: keep the flat prologue, ship the B1 gate, and charter the
launch-overhead removal to P5.2 where the prologue remap + its `.spv` re-pin + the
byte-compare obligation can be owned in isolation. This is the low-risk convergence — the
0%-gate stays trivially provable (flat dispatch unchanged; only the terminal gating
differs; on a no-mesh scene `has_mesh` is always false ⇒ `own_pixel` always true ⇒
byte-identical writes).

**Alternatives rejected.**
- *Option (ii): elevate indirect dispatch into P5 and own the prologue remap + `.spv`
  re-pin + the runtime byte-compare.* Rejected: it entangles a self-contained perf
  follow-up with the mesh-first-class-citizen correctness change, enlarges the `.spv`
  delta from "terminal gating only" to "terminal gating + prologue remap," and converts
  the trivially-provable 0%-gate into one that REQUIRES the runtime no-mesh byte-compare
  to discharge. The launch-overhead win it unlocks is second-order (the march is already
  skipped on empty tiles), so the entanglement is not justified. Reconsider in P5.2 if a
  profiler shows empty-wavefront-launch dominant on the mesh-dense target.

**Trade-off (honestly stated).** Option (i) leaves the **O(tile_count) empty-WAVEFRONT-
LAUNCH residual** on the table — the GPU still launches a 64-thread group per empty tile
that early-returns before the march. This residual is removed only by P5.2's indirect
dispatch. P5 does not claim to remove it; P5 removes the per-mesh-pixel store/attribution/
shading cost (the B1 gate) and relies on the EXISTING empty-tile march short-circuit for
the empty-tile cost.

---

## Public API / host surface

P5 is a shader + host-driver change inside `boyko_rhi_vulkan`; it adds NO public engine
API. The host-visible surface is one push-constant interpretation (no new field — the
ownership gate is derived ENTIRELY from existing in-shader state `has_mesh`, `t`,
`t_mesh`, and the SDF hit flag, so the `FineMarcherPush` block is UNCHANGED) and the
existing barrier schedule (Decision/M1 below). No new binding, no layout change, no new
descriptor (the M2 R2 cap is untouched).

```rust
// compute.rs — NO new push field. The marcher derives own_pixel from existing state.
// The driver change is limited to the barrier/transition schedule (gDepth must be
// SHADER_READ_ONLY before the marcher dispatch — already true today) and the offscreen
// tripwire test (Decision 4).
```

The marcher shader edit, in signature terms (pseudo-HLSL, NOT the implementation):

```hlsl
// Site B (terminal), replacing lines 1636-1646:
//   bool own_pixel = !has_mesh || (hit && t < t_mesh);   // hit/t/t_mesh already in scope
//   if (own_pixel) {
//       // the existing mask=1 (SDF) / mask=0 (background) attribute writes, UNCHANGED
//       gAlbedo[uint2(px,py)]   = ...;
//       gNormal[uint2(px,py)]   = ...;
//       gMaterial[uint2(px,py)] = ...;
//   }                                // !own_pixel: raster owns it — write nothing here
//   gViewT[uint2(px,py)] = own_pixel && mask == 1.0 ? t : 1.0e30;  // always written (C2)

// Site A (empty-tile early-return), replacing lines 1320-1329:
//   // In an EMPTY tile there is no SDF surface, so own_pixel == !has_mesh.
//   if (!has_mesh) {                 // marcher owns: write BACKGROUND attrs (today's else arm)
//       gAlbedo[uint2(px,py)]   = float4(BACKGROUND, 1.0);
//       gNormal[uint2(px,py)]   = ...;
//       gMaterial[uint2(px,py)] = ...;
//   }                                // has_mesh: raster owns — write nothing here
//   gViewT[uint2(px,py)] = 1.0e30;   // always written (C2); never read on a non-lit pixel
//   return;
```

---

## Algorithms for the critical path

**The marcher terminal, per pixel (the only changed steps):**

1. Compute `own_pixel = !has_mesh || (hit && t < t_mesh)` — 1 compare + 1 AND + 1 OR, all
   on values already in registers (`has_mesh` from line 1285, `hit`/`t` from the march,
   `t_mesh` from line 1286). **O(1), no memory access.**
2. Branch on `own_pixel`. On `true` (the no-mesh-scene universal case): the three
   attribute stores, byte-identical to today. On `false`: skip the three stores.
3. Always store `gViewT` (the sentinel on `!own_pixel`).

- **Complexity.** O(1) added per pixel (one predicate, one branch). No new loop, no new
  memory traffic. On `!own_pixel` it REMOVES work: −3 storage stores, and (when the
  own-pixel short-circuit is placed before the march on a provably-mesh-front pixel) −1
  `pick_material_id` argmin and −2 A1/A2 marches.
- **Cache behavior.** The three gated stores are streaming storage-image writes
  (uncached, write-combining); eliminating them on mesh-covered pixels reduces store-buffer
  / write-combine pressure. No new reads. `has_mesh`/`t`/`t_mesh` are already resident.
- **Branching.** One added branch on `own_pixel`. Coherent within a wavefront in the
  common cases: on a no-mesh scene it is uniformly true (no divergence); on a mesh-dense
  scene mesh coverage is spatially clustered (the 8×8 tile granularity already exploited by
  the coarse cull), so the branch is largely wavefront-coherent. The worst case (a mesh
  silhouette edge cutting a wavefront) is the same divergence the existing `has_mesh`
  branch at line 1320 already incurs.
- **SIMD/wavefront potential.** `own_pixel` is a per-lane predicate; the gated stores
  predicate cleanly. No change to the field eval's vectorization.

---

## Multithreading / GPU concurrency model

- **The marcher is a flat 1-D dispatch** (Decision 5): each thread owns exactly one pixel
  `idx = tid.x → (px, py)`. There is **no cross-lane communication** — each thread reads
  its own `gDepth` texel and writes (or yields) its own G-buffer texels. The ownership
  gate is a per-thread local decision; it introduces NO new synchronization.
- **No data race on the G-buffer.** Each texel `(px, py)` is written by exactly one
  marcher thread (the bijection `idx ↔ (px,py)`). The raster pass that produced `gDepth`
  and the mesh G-buffer fragments completed BEFORE the marcher dispatch (the barrier in
  M1). The ownership gate ensures that for a mesh-owned texel the marcher does not write,
  so the raster's fragment (written in the prior pass) is the final value — read-after-
  write across a barrier, no concurrent write.
- **gViewT exactly-once** is preserved per-thread (Decision 3): each thread writes its own
  gViewT texel exactly once at its single terminal exit. No two threads target the same
  texel; no thread targets two texels.

### M1 — the barrier / transition schedule + the gViewT survivor model

The pipeline ordering P5 relies on (UNCHANGED by P5 — documented here for the
correctness proof):

1. **Raster pass** rasterizes the mesh into the D32_SFLOAT depth image and (in the
   mesh-first-class model) the mesh's G-buffer fragments.
2. **Barrier / layout transition:** the depth image transitions to SHADER_READ_ONLY
   (so the marcher's `gDepth.Load` is a valid sampled-image fetch — already the contract,
   header lines 11-16, 119-122), and the mesh's G-buffer writes are made visible to the
   marcher's subsequent reads/writes (a memory + execution barrier between the raster pass
   and the compute dispatch). This barrier already exists for the current hybrid composite;
   P5 adds no new barrier.
3. **Marcher dispatch** (the compute pass): reads `gDepth`, and per the ownership gate
   either writes the SDF G-buffer attrs (own_pixel) or yields to the raster's fragment
   (!own_pixel). Writes gViewT for every real pixel.
4. **Barrier** before the deferred resolve reads the G-buffer + gViewT.

**The gViewT survivor (M1).** "Survivor" = the value that survives in a gViewT texel to be
read by the resolve. P5's invariant: every REAL pixel's gViewT texel is written exactly
once by the marcher this frame (the C2 contract, lines 229-241). On an own_pixel SDF-lit
pixel the survivor is the real marched `t`; on every other real pixel (background, mesh-
owned, empty-tile) the survivor is the `1.0e30` sentinel. The resolve reads gViewT ONLY
inside `mask == 1`, so the sentinel survivors are never consumed — but they MUST be written
to prevent a stale prior-frame `t` from surviving into a frame where `mask` flips. The ONE
non-writing exit is the over-hang guard (`idx >= count`, lines 1258-1261): those threads
own no pixel (the bijection only covers `idx < count`), so their gViewT non-write is
correct (there is no real pixel to leave unwritten). The ownership gate does NOT add a
non-writing exit — it only changes whether the THREE ATTRIBUTE lanes are written, never
whether gViewT is written.

### M2 — the binding-10 R2 contract (untouched, documented for completeness)

Binding 10 = combined image+sampler (`Texture3D<float> BrickAtlas` @ `register(t10)` +
`SamplerState BrickSampler` @ `register(s10)`, both at `[[vk::binding(10, 0)]]`, lines
285-286). DXC keeps the static refs past the runtime `brick_trilinear` branch, so the
pipeline layout MUST declare binding 10 as combined-image-sampler and bind a VALID
atlas+sampler even on the gated-OFF path, or pipeline creation / dispatch trips the layout
VUIDs. **P5 adds no binding and does not touch the M2 path** — this note exists only to
record that the ownership gate's edits at Sites A/B are downstream of (and independent
from) the M2 SURFACE-brick branch, so they cannot perturb the R2 binding contract.

---

## Integration

- **Modules touched.** `crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl`
  (the two terminal write sites), its committed `sdf_gbuffer_composite.comp.spv` (re-DXC'd
  for the terminal gating — Owner-call #1), `crates/boyko_rhi_vulkan/src/compute.rs` (no
  push-field change; barrier schedule unchanged; the offscreen tripwire test added), and
  the offscreen + windowed golden tests.
- **Modules NOT touched.** `sdf_field.hlsli`, `ray_gen.hlsli`, `deferred_pbr.hlsl`,
  `sdf_tile_cull.hlsl`, all of `boyko_render`, `boyko_ecs`. The determinism boundary is
  intact.
- **Existing-API impact.** None. The `FineMarcherPush` block is byte-unchanged (no new
  field), so the host `#[repr(C)]` POD + its const-asserts are unchanged. The pipeline
  layout, bindings, and barrier schedule are unchanged.

### The rung decomposition (implementation order)

| Rung | What | File | Gate |
|------|------|------|------|
| P5-r1 | Add `own_pixel` predicate + gate the THREE attribute stores at **Site B** (terminal, lines 1636-1646); gViewT stays always-written (Decision 3). | `sdf_gbuffer_composite.hlsl` | Re-DXC; offscreen no-mesh golden BYTE-IDENTICAL (0%-gate). |
| P5-r2 | Gate the three attribute stores at **Site A** (empty-tile early-return, lines 1320-1329) symmetrically; gViewT sentinel stays. | `sdf_gbuffer_composite.hlsl` | Re-DXC; FULL-mode empty-tile golden BYTE-IDENTICAL on no-mesh; mesh-in-empty-tile test shows raster survives. |
| P5-r3 | The B2 tripwire: offscreen single-quad-over-empty test asserting the `own_pixel` footprint matches the raster footprint (Decision 4). | `compute.rs` test | Footprint test passes; flips if depth convention drifts. |
| P5-r4 | Windowed-present visual check (owner is the visual oracle): a mesh + SDF scene, mesh pixels show the RASTER's shading (not the marcher's re-derived MESH_COLOR), SDF-in-front pixels show SDF. | windowed present test | Owner visual OK before commit. |
| **P5.2 (chartered follow-up, OUT of P5)** | Classify→compact→`DispatchIndirect` over a compacted tile-id list to remove the empty-WAVEFRONT-LAUNCH residual. **Requires** the prologue group→tile→(px,py) remap, its `.comp.spv` re-pin, and the runtime no-mesh byte-compare to discharge the remapped 0%-gate. | (future) | Profiler shows empty-launch dominant on the mesh-dense target FIRST. |

---

## Perf validation

- **Offscreen golden (0%-gate):** the existing no-mesh ORTHO golden
  (`golden_composite_pixel_ex`) must produce a BYTE-IDENTICAL G-buffer before and after
  P5 (on a no-mesh scene `own_pixel` is always true ⇒ every write path is unchanged). This
  is the primary correctness gate and is trivially provable from Decision 5 (flat prologue
  frozen, only terminal gating differs).
- **Mesh-coverage A/B bench:** render the mesh-dense target frame with the marcher
  instrumented (a debug counter, or a profiler capture) and confirm the marcher performs
  **zero** gAlbedo/gNormal/gMaterial stores on mesh-covered pixels post-P5 (vs 3 each
  pre-P5), and skips `pick_material_id` + A1/A2 on the short-circuited mesh pixels.
- **Frame-time A/B:** the mesh-dense target frame should show a net frame-time reduction
  proportional to the mesh-covered pixel fraction × (3 stores + attribution + A1/A2). On a
  no-mesh scene frame-time must be within noise of pre-P5 (the 0%-gate, asserted by the
  byte-identical golden).
- **NOT a P5 metric:** empty-wavefront-launch overhead (that is P5.2's metric).

---

## Risks

| # | Risk | Mitigation |
|---|------|------------|
| R1 | The ownership gate at Site A and Site B diverge, producing a tile-boundary seam. | Decision 2's 4-state table + the symmetric rung order (r1 then r2) + the FULL-mode empty-tile golden. |
| R2 | A future depth-convention change (reversed-Z, perspective nonlinearity, clear-value change) silently inverts the gate. | Decision 4's tripwire (footprint test) flips loudly in CI. |
| R3 | gViewT left unwritten on a `!own_pixel` pixel ⇒ stale prior-frame `t` survives. | Decision 3: the gate controls only the three ATTRIBUTE lanes; gViewT is ALWAYS written (the sentinel). Asserted by the C2 exactly-once contract. |
| R4 | The committed `.comp.spv` drifts from the edited `.hlsl` (the blob is the artifact actually dispatched). | Re-DXC the marcher and re-commit `sdf_gbuffer_composite.comp.spv` in the SAME commit as the .hlsl edit (W4 blob pin); the offscreen golden re-runs against the blob. |
| R5 | Scope creep: indirect dispatch entangles into P5. | Decision 5 demotes it to P5.2; the rung table marks it OUT of P5. |

---

## Open questions (all answered)

- **OQ-A — Does the ownership gate need a new push-constant field?** **No.** `own_pixel`
  is derived entirely from in-shader state (`has_mesh`, `hit`, `t`, `t_mesh`), so
  `FineMarcherPush` is byte-unchanged. (Answered: Decision 1 / Public API.)
- **OQ-B — Does P5 change the M2 binding-10 R2 contract or any binding?** **No.** P5 edits
  only the terminal write arms, downstream of and independent from the M2 SURFACE-brick
  branch. No binding added or changed. (Answered: M2 note.)
- **OQ-C — Where does the mesh's G-buffer color come from on a `!own_pixel` pixel if the
  marcher no longer writes MESH_COLOR?** From the **raster pass**, which writes the mesh's
  G-buffer fragments BEFORE the marcher dispatch (M1 step 1). The marcher yielding is
  exactly "let the raster's already-written fragment stand." (Answered: Decision 1 + M1.)
- **OQ-D — Does P5 remove the empty-tile cost?** The empty-tile MARCH cost is ALREADY
  removed today (the EMPTY-tile early-return at lines 1314-1331 returns before the
  sphere-trace loop at line 1348+). P5 does NOT change the dispatch, so the empty-
  WAVEFRONT-LAUNCH residual remains — **that residual is chartered to P5.2; P5 itself keeps
  the flat prologue and wins via the B1 ownership gate + the existing empty-tile march
  short-circuit.** (Answered: Decision 5 / the W1↔W2 resolution.)
- **OQ-E (Owner-call #1) — Re-pinning the `.comp.spv`.** P5 re-DXCs and re-commits
  `sdf_gbuffer_composite.comp.spv` for the **terminal-gating edit only** (NOT a prologue
  remap). This is the owner's go/no-go on touching the frozen marcher blob. The 0%-gate
  (byte-identical no-mesh golden) discharges the risk; owner approves the blob re-pin.

---

## Readiness checklist

### Plan structure
- [x] Goal stated in perf + functionality terms (4 stores → 0 on mesh pixels; mesh as a
  structural first-class producer).
- [x] Target metrics concrete (per-mesh-pixel store/attribution/shading saved; .spv delta
  = terminal-gating only; 0%-gate on no-mesh scenes).
- [x] Every decision justified via perf/cache/parallelism.
- [x] Each alternative rejected with reasoning (Decisions 1, 5).
- [x] Trade-offs listed honestly (incl. the empty-launch residual left to P5.2).

### Data / shader structure
- [x] Both clobber sites anchored (Site A 1314-1331, Site B 1644-1653) with exact lines.
- [x] The 4-state ownership table is total (all `has_mesh` × SDF-result cases).
- [x] gViewT exactly-once handled separately from the attribute gate (Decision 3).
- [x] No new binding / no layout change (M2 R2 untouched).

### Correctness
- [x] Edge cases: no-mesh scene (0%-gate), far-plane-grazing mesh (safe misclassification),
  mesh-in-empty-tile (Site A), SDF-behind-mesh (!own_pixel).
- [x] B2 conservatism proven + a CI tripwire pins the depth-convention assumption.
- [x] gViewT survivor / exactly-once preserved; the over-hang is the only non-writing exit.
- [x] Determinism boundary untouched (field, ray-gen, golden unchanged).

### Multithreading / GPU
- [x] Flat 1-D dispatch, per-thread pixel bijection, no cross-lane comm (Decision 5).
- [x] No G-buffer data race (per-texel single writer across the raster→compute barrier).
- [x] Barrier schedule documented (M1) — unchanged by P5.

### Integration
- [x] Modules touched / not touched enumerated.
- [x] No public-API change; `FineMarcherPush` byte-unchanged.
- [x] `.comp.spv` re-pin owned (W4 / Owner-call #1).
- [x] Rung decomposition + P5.2 charter line.

### Validation
- [x] Offscreen byte-identical no-mesh golden (the 0%-gate).
- [x] Mesh-coverage store-count A/B + frame-time A/B benches.
- [x] The B2 footprint tripwire test.
- [x] Windowed-present visual check (owner = visual oracle) before commit.

### W1↔W2 internal consistency
- [x] **No remaining "flat prologue unchanged" vs "indirect tile dispatch" contradiction.**
  Decision 5 keeps the flat prologue byte-frozen and moves indirect dispatch to P5.2;
  every Decision, the rung table, the perf section, and OQ-D agree. P5's `.comp.spv` delta
  is terminal-gating only.


---

# Render P5 — Addendum r0: The Mesh-MRT G-buffer PRODUCER (the rung BEFORE r1)

**Branch:** `ecs` · **Status:** DESIGN (architect, revised) · **Slots BEFORE P5-r1** in the rung table · **Owner-call carried:** OQ-r0-B (mesh material source), OQ-r0-C (raster `.spv` pair re-pin)

This addendum closes the **producer gap** the orchestrator found: the locked plan's ownership gate (r1–r4) makes the marcher *yield* on mesh-owned pixels, but today's raster pass writes only **depth** + a throwaway color — so a yielded mesh pixel's `gAlbedo`/`gNormal`/`gMaterial` would be stale. r0 upgrades pass A from a **depth-only prepass** into a **3-lane MRT G-buffer producer** that writes the mesh's albedo/normal/material in the marcher's EXACT encoding, so the gate has a real fragment to yield to.

**Revision note (post-critique):** the gViewT raster path (the draft's Decision r0-4) and the marcher gViewT-yield amendment are **removed from r0 and moved to r1b** — they edit the frozen marcher and belong with the gate commit, not the producer commit. r0 is now a pure raster + barrier + image-usage change with the marcher `.comp.spv` genuinely byte-frozen. The barrier source half is fully specified (no copy-paste of the `UNDEFINED`/`src=0` shape). A `COLOR_ATTACHMENT` format-feature caps fail-fast is added. See "Critique resolutions" at the end.

---

## Goal

Make raster pass A the **single source of truth** for mesh-covered G-buffer texels' three attribute lanes: it writes `gAlbedo`/`gNormal`/`gMaterial` for every mesh fragment in the marcher's byte-exact encoding, with `mask = 1` so the deferred resolve lights mesh pixels first-class (full Cook-Torrance, identical to an SDF pixel). After r0, the r1–r4 ownership gate composes: marcher yields ⇒ the raster's fragment stands ⇒ resolve reads a fully-written, correctly-encoded texel.

**Performance/correctness content of r0:**
- The mesh stops being a constant the marcher re-derives (`base = MESH_COLOR`, mask=0, flat-shaded, no PBR). It becomes a *rasterized* producer with a real interpolated normal + a real material id ⇒ the mesh gets the SAME PBR pipeline as SDF surfaces.
- One 3-MRT raster pass (3 color + depth) replaces a 1-throwaway-color depth-prepass. The mesh fragment cost is the rasterizer's, not 64-wide compute lanes re-deriving a flat color — strictly cheaper per mesh pixel AND it unlocks the r1–r4 store-elimination win.

**Target metrics:**
- No-mesh scene: **byte-identical** to today's output. This is r0's 0%-gate.
- Mesh-covered pixel (post-r1): G-buffer attribute lanes written ONCE by the raster (3 MRT stores + 1 depth), then the marcher yields ⇒ exactly-one-producer invariant holds structurally.

---

## Context and constraints (what r0 must not break)

- **The marcher's encoding is the contract, not the raster's.** The resolve (`deferred_pbr.hlsl`, L11-31/62-65) reads `base = gAlbedo.rgb` (RAW LINEAR), `n = oct_decode(gNormal.rg)`, `id = round(gNormal.b*255)|round(gNormal.a*255)<<8`, `shadow=gMaterial.r, ao=gMaterial.g, mask=gMaterial.b>0.5`. r0's raster fragment MUST write that exact layout for the three lanes or the resolve mis-decodes a mesh pixel.
- **The G-buffer images are STORAGE images in GENERAL** (`RWTexture2D<float4>`, written by the compute marcher, read by the compute resolve, both in `VK_IMAGE_LAYOUT_GENERAL`). A raster pass writes COLOR attachments. r0 reconciles this dual-usage on the SAME images — the central wrinkle below.
- **Today's pass A**: `vkCmdBeginRendering`, 1 throwaway COLOR attachment (`raster_color`, CLEAR/STORE, never read) + the D32 depth (CLEAR=1.0/STORE). The G-buffer images are transitioned `UNDEFINED → GENERAL` AFTER pass A (step 4, `swapchain.rs` ~L2618-2645, `src_access=0`/`TOP_OF_PIPE`), and are NOT touched by pass A today.
- **Formats** (confirmed): `GBUFFER_FORMAT = R8G8B8A8_UNORM` (albedo/normal/material/lit), `GVIEWT_FORMAT = R32_SFLOAT` (viewt), depth = D32_SFLOAT.
- **The marcher `.comp.spv` stays byte-frozen in r0.** r0 is a raster-side + barrier-side + image-usage change ONLY. The marcher's gViewT behavior (incl. the gViewT-yield) is r1b. r0 and r1+ are SEPARABLE commits, and this separability is now genuine (the draft's gViewT amendment that broke it is gone).
- **Binding cap**: the marcher's vocab set sits at 15/16. r0 adds NOTHING to the vocab set — the raster pass has its OWN descriptor set. Confirmed: r0 does not approach the marcher's binding cap.

---

## The CENTRAL WRINKLE: one image written by BOTH a raster pass and a compute pass

The same `gAlbedo`/`gNormal`/`gMaterial` images must be (a) COLOR-attachment targets of pass A's raster, then (b) STORAGE images the marcher writes and the resolve reads in GENERAL. Resolution:

### Decision r0-1: The three RGBA8 G-buffer images carry `STORAGE | COLOR_ATTACHMENT` usage; r0 writes them as MRT color attachments, then a fully-specified `COLOR_ATTACHMENT_OPTIMAL → GENERAL` barrier (with a real `COLOR_ATTACHMENT_WRITE` source) hands them to the marcher.

**What.**
1. **Usage flags** (in `GBufferTargets::create`):
   - `albedo`: `STORAGE | SAMPLED | COLOR_ATTACHMENT` (was `STORAGE | SAMPLED`)
   - `normal`: `STORAGE | COLOR_ATTACHMENT` (was `STORAGE`)
   - `material`: `STORAGE | COLOR_ATTACHMENT` (was `STORAGE`)
   - `viewt`: **UNCHANGED — `STORAGE`-only.** r0 does NOT touch gViewT (the draft's r0-4 raster-gViewT moves to r1b). No R32_SFLOAT-as-color-attachment dependency in r0.
   - `raster_color` (the throwaway): **DELETED** — the MRT now binds the three real G-buffer images, so the throwaway is obsolete. Removing it also removes its barrier step (0) and its create/teardown lines.

2. **A boot-time `COLOR_ATTACHMENT` caps fail-fast** (NEW, mirroring the existing `gbuffer_storage_format_ok` pattern in `device.rs` ~L1908-1947): before `GBufferTargets::create`, `GetPhysicalDeviceFormatProperties(R8G8B8A8_UNORM)` and assert `optimalTilingFeatures & VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT`. RGBA8_UNORM color-attachment renderability is mandatory in Vulkan, so this passes universally — but the explicit gate is the project's fail-fast discipline (constraint 6: no validation oracle, so an unsupported usage must fail at boot with a clear message, not as a device-lost on the RTX). **R32_SFLOAT is NOT checked in r0** because r0 does not add COLOR_ATTACHMENT to gViewT; that caps-gate ships WITH r1b (see r1b's caps note).

3. **Layout cycle per frame** (the new pass-A schedule):
   - **Pass A barrier-in**: the 3 RGBA8 images `UNDEFINED → COLOR_ATTACHMENT_OPTIMAL`. `src_stage = TOP_OF_PIPE`, `dst_stage = COLOR_ATTACHMENT_OUTPUT`, `src_access = 0`, `dst_access = COLOR_ATTACHMENT_WRITE`. Depth `UNDEFINED → DEPTH_ATTACHMENT_OPTIMAL` (unchanged). This REPLACES today's `raster_color UNDEFINED→COLOR` (step 0), now on the three real images.
   - **Pass A**: `vkCmdBeginRendering` with **3 MRT color attachments** (`albedo`@0, `normal`@1, `material`@2) + depth. All color + depth attachments `LOAD_OP = CLEAR` (clear values = Decision r0-2 neutrals), `STORE_OP = STORE`. Draw the mesh.
   - **Pass A barrier-out** (the central spec — NOT a copy of today's `src=0`/`TOP_OF_PIPE` shape, because the source half genuinely changes now that the raster actually wrote the images): the 3 RGBA8 images `COLOR_ATTACHMENT_OPTIMAL → GENERAL`, **per-image**, batched into one `vkCmdPipelineBarrier`:
     ```
     src_stage   = COLOR_ATTACHMENT_OUTPUT
     dst_stage   = COMPUTE_SHADER
     src_access  = COLOR_ATTACHMENT_WRITE        // NOT 0 — the raster's writes must be made available
     dst_access  = SHADER_READ | SHADER_WRITE    // marcher reads/writes; resolve (post-r1) reads
     old_layout  = COLOR_ATTACHMENT_OPTIMAL      // NOT UNDEFINED
     new_layout  = GENERAL
     ```
     The depth barrier (step 3, `DEPTH_ATTACHMENT_OPTIMAL → SHADER_READ_ONLY`, `LATE_FRAGMENT_TESTS → COMPUTE_SHADER`) is UNCHANGED. gViewT's `UNDEFINED → GENERAL` (`src=0`/`TOP_OF_PIPE`) stays AS TODAY — r0 does not rasterize into gViewT, so its source half is correctly still `UNDEFINED`/`0`.
   - **Marcher (pass B) + resolve**: unchanged — they see the three images in GENERAL carrying the rasterized mesh fragments where the mesh covered, and the CLEAR neutrals everywhere else. gViewT is still wholly marcher-produced in r0.

**Why this option over the alternatives.**
- **vs a SEPARATE mesh-G-buffer image set + a compute composite.** Rejected: doubles G-buffer VRAM, adds a fullscreen merge pass (the bandwidth waste the locked plan's Decision-1 already dismissed for the gate), and the resolve would need a second image set or a copy. Writing into the SAME images is strictly less bandwidth, zero extra VRAM.
- **vs an aliased / two-views-one-memory trick.** Rejected: needs `MUTABLE_FORMAT` / aliasing for no benefit — RGBA8_UNORM is a valid COLOR-attachment AND storage format, so ONE image with `STORAGE|COLOR_ATTACHMENT` + a layout transition is the clean, caps-checkable path. Bytes are layout-invariant (UNORM RGBA8); only the access layout transitions.

**Why it's correct.** The barrier closes the raster→marcher WAW/RAW hazard precisely because `src_stage/src_access = COLOR_ATTACHMENT_OUTPUT/COLOR_ATTACHMENT_WRITE` makes the raster's color writes *available*, and `dst = COMPUTE_SHADER/SHADER_READ|SHADER_WRITE` makes them *visible* to the marcher's access. The marcher then (post-r1, under the gate) does NOT write mesh-owned texels, so the raster's value survives to the resolve. Single producer per texel across the two barriers.

**Trade-off.** The three images now toggle `COLOR_ATTACHMENT_OPTIMAL → GENERAL` every frame (one extra image barrier each vs today's single `UNDEFINED → GENERAL`). Negligible on the desktop RTX target; the throwaway-color deletion offsets one barrier + one image alloc.

### Decision r0-2: The MRT CLEAR values ARE the marcher's mask=0 neutral G-buffer — so a no-fragment pixel is byte-identical to today's marcher background write.

**What.** Pass A's three color attachments clear to EXACTLY the bytes the marcher writes on its `mask=0` / background arm, so a pixel with no mesh fragment holds the cleared neutral, which the marcher (owning that pixel) overwrites anyway:
- `gAlbedo` clear = `(BACKGROUND.rgb, 1.0)` — the marcher's background base.
- `gNormal` clear = `(0.5, 0.5, 0.0, 0.0)` — neutral oct + id=0.
- `gMaterial` clear = `(1.0, 1.0, 0.0, 1.0)` — shadow=1, ao=1, **mask=0**, 1. mask=0 ⇒ the resolve passes base through untouched.

**UNORM-quantization note** (critique concern): these clears are RGBA8_UNORM `loadOp` clear-color floats; they pass through the SAME float→UNORM8 quantizer the marcher's `RWTexture2D<float4>` store uses (`round(c*255)`), so the cleared neutral is bit-identical to a marcher-written neutral. `0.5→128`, `1.0→255`, `0.0→0` are all exact under round-to-nearest — no 1-LSB wobble. A debug_assert in the host test pins the clear-color constants to the marcher's background-arm constants.

**Why.** Two birds: (1) r0's no-mesh 0%-gate becomes trivial *independent of the marcher* — a no-mesh frame's cleared G-buffer is the mask=0 neutral; (2) on a MESH scene, a depth-failed/partially-missed mesh fragment falls back to a valid mask=0 neutral rather than garbage. The clear is the safety net; the rasterized fragment is the real value.

### Decision r0-3: The raster fragment shader writes the marcher's EXACT 3-lane encoding, with `oct_encode` / `pack_material_id_ba` SPLICED from the SAME eDSL printer call — one source, two splice sites, not a checked copy.

**What.** A new fragment shader `gbuffer_mrt.fs.hlsl` outputs 3 SV_Targets:
```hlsl
struct PsOut {
    float4 albedo   : SV_Target0;  // -> gAlbedo  (RAW LINEAR base color, a=1)
    float4 normal   : SV_Target1;  // -> gNormal  (oct.x, oct.y, id_ba.x, id_ba.y)
    float4 material : SV_Target2;  // -> gMaterial (shadow=1, ao=1, mask=1, a=1)
};
```
Per mesh fragment:
- `base = mesh_base_color` (interpolated LINEAR vertex color; see OQ-r0-B). `albedo = float4(saturate(base), 1.0)`.
- `n = normalize(interpolated world normal)`; `oct = oct_encode(n)`; `id_ba = pack_material_id_ba(mesh_material_id)`. `normal = float4(oct.x, oct.y, id_ba.x, id_ba.y)`.
- `material = float4(1.0, 1.0, 1.0, 1.0)` — **shadow=1, ao=1, mask=1**. mask=1 routes the resolve through full Cook-Torrance. (P5 ships analytic shadow=ao=1 for the mesh — contact shadow/AO via the SDF march is a charted follow-up, NOT P5.)

**The eDSL single-sourcing (constraint 2 / owner rule — now stated as splice, not copy).** `oct_encode` and `pack_material_id_ba` are single-sourced in `boyko_shaderdsl` (`oct::oct_encode_body`, `pack::pack_material_id_ba_body`) and printed by `emit::emit_hlsl_oct_encode` / `emit::emit_hlsl_pack_material_id_ba`. The discipline (matching the SDF eDSL campaign's 22 live-splice sync pins):
- The marcher's spans (`sdf_gbuffer_composite.hlsl` ~L507-527) and `gbuffer_mrt.fs.hlsl`'s spans are produced by the **SAME `emit_hlsl_*` entrypoint** — one emission, **two splice sites** — written between identical `// === GENERATED oct_encode BEGIN === … END ===` sentinels. The raster span is a GENERATED ARTIFACT spliced into the `.fs.hlsl`, NOT a hand-pasted copy.
- A **hard build-gate sync test** (`gbuffer_mrt_edsl_sync.rs`, mirroring `sdf_field_edsl_sync.rs` by name and mechanism) asserts the two splice sites are byte-identical to the live `emit_hlsl_*` output. Test failure = no ship. The test GUARDS the splice; the eDSL printer is the source of truth.
- Raster-specific glue (the `PsIn`/`PsOut` structs, `SV_Target` plumbing, `normalize`, `saturate`, the material/mask constants) is hand-written — legitimate raster I/O, NOT the drift-prone encode math.

**Why.** The resolve's `oct_decode` assumes the marcher's `oct_encode`. A divergent hemisphere-sign/axis fold would mis-light every mesh normal, invisible until an owner-visual on an asymmetric primitive (slow, crash-prone loop, constraint 6). Splicing one source into two sites makes raster↔marcher↔resolve a compile-time identity.

**The mesh material note (OQ-r0-B).** The mesh quad's vertex buffer already carries position + color. For P5, `base` = interpolated vertex color, `mesh_material_id` = a push-constant (default 0 ⇒ default material). A material-table fetch / textured PBR mesh is a charted follow-up. The marcher's `MESH_COLOR` constant becomes dead once r1 lands.

---

## gViewT — explicitly DEFERRED out of r0 (was the draft's Decision r0-4)

**r0 does NOT touch gViewT.** The marcher remains the sole gViewT producer in r0 (it writes gViewT unconditionally today: Site B `(mask==1)?t:1.0e30` at ~L1653, and the empty-tile early-return `1.0e30` at ~L1329). r0 adds no `COLOR_ATTACHMENT` to the gViewT image, no 4th MRT attachment, and no marcher edit. This restores r0's frozen-marcher + separable-commit invariant.

**The mesh-point/spot-lighting story moves to r1b** (a sub-rung paired with r1, where the marcher is ALREADY edited to yield):
- r1b adds `COLOR_ATTACHMENT` to the gViewT image **gated by a NEW boot caps fail-fast** on `R32_SFLOAT`'s `VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT` (R32_SFLOAT color-attachment renderability is OPTIONAL in Vulkan — if absent on the target, r1b falls back to the locked plan's Decision 3: marcher writes the gViewT sentinel on a mesh pixel, mesh is directional-only).
- r1b adds a 4th MRT target `SV_Target3 = SV_Position.z * T_MAX` (the post-projection rasterizer depth × ortho `T_MAX` — **bit-identical to the marcher's `t_mesh = md * T_MAX`** because the marcher reads the SAME D32 depth the rasterizer wrote; the fragment MUST use `SV_Position.z`, NOT a custom linear-depth varying, or the two formulas de-sync).
- r1b amends the marcher to NOT write gViewT on a `!own_pixel` mesh pixel (yield gViewT alongside the three attribute lanes), so the raster's real `t` survives. Because this marcher edit lands in the SAME commit as the raster's 4th-MRT write, the exactly-once invariant is provable in a single diff — the split-brain the critique flagged is gone.

This keeps r0 a pure producer of the three RGBA8 lanes. Between r0 and r1, a mesh pixel's gViewT is whatever the un-gated marcher writes (sentinel, since mesh pixels are still mask=0 in the marcher pre-r1) — harmless because mesh pixels aren't position-lit until r1b flips them.

---

## The 0%-gate proof (r0 + r1–r4 compose to byte-identity on no-mesh)

A no-mesh scene rasterizes ZERO fragments. Therefore:
1. **Pass A** clears the 3 color attachments to the mask=0 neutrals (Decision r0-2) and depth to 1.0, draws the mesh ⇒ **no fragment passes** ⇒ the three images hold the CLEAR neutrals; depth holds 1.0 everywhere; **gViewT is untouched by pass A** (still `UNDEFINED→GENERAL` as today).
2. The fully-specified `COLOR_ATTACHMENT_OPTIMAL → GENERAL` barrier hands the cleared neutrals to the marcher. (The `src=COLOR_ATTACHMENT_WRITE` half is harmless on a clear-only pass — a clear IS a color write, correctly made available.)
3. **Marcher (pass B)** — covering BOTH the marcher's write sites:
   - **Site B (per-pixel hit path)**: `has_mesh = (md < 1.0)` is FALSE for every pixel (depth = clear 1.0 everywhere) ⇒ post-r1 `own_pixel = !has_mesh = true` ⇒ the marcher writes EVERY pixel exactly as today. Pre-r1 the marcher is un-gated and also writes every pixel. Either way the cleared neutrals are entirely overwritten — never read.
   - **Site A (empty-tile early-return, ~L1314-1331)** — the actual edge case the critique flagged: on a coarse-culled empty tile the marcher writes all four lanes incl. gViewT at ~L1329. With no mesh, this path writes the same background/sentinel as today. The raster-cleared neutral under it is overwritten, never observed. **This is the ONE path that could expose a raster-cleared-but-marcher-unread texel; it is closed because the empty-tile return unconditionally writes all lanes.**
4. **Resolve**: reads the marcher-written G-buffer, identical to today.

⇒ On a no-mesh scene the output is **byte-identical** to pre-r0. The clear values are dead (overwritten by the marcher at Site A or Site B); the extra COLOR_ATTACHMENT usage + the layout cycle do not change image CONTENT (RGBA8 UNORM bytes are layout-invariant); the deleted throwaway-color is unobservable; gViewT is wholly unchanged by r0. **r0's own 0%-gate is the byte-identical no-mesh golden; r1–r4's 0%-gate composes because their gate is a no-op when `has_mesh` is always false.**

**The r0-before-r1 subtlety.** r0 lands BEFORE r1. On a no-mesh scene, byte-identical (proven above). On a MESH scene, r0-alone DOUBLE-writes the three mesh lanes: the raster writes the real mesh G-buffer, THEN the un-gated marcher clobbers it with `MESH_COLOR`/mask=0 — the marcher wins, the raster write is dead-but-correct. That is *transiently* the status-quo mesh appearance, NOT a regression. So r0 is independently shippable (no-mesh byte-identical; mesh = status-quo), and r1 flips mesh pixels to the raster's first-class G-buffer.

---

## Binding / descriptor plan (constraint 7: the cap is untouched)

- The **raster pass A has its OWN pipeline + descriptor set**, entirely separate from the marcher's 15/16 vocab set. The raster pipeline needs: the VERTEX push range (the 64-byte MVP, unchanged) + a small FRAGMENT push (the 4-byte mesh material id). No StorageBuffer in r0 (vertex-color base; the material-table fetch is the OQ-r0-B follow-up). The marcher's vocab set is NOT touched: r0 adds ZERO vocab bindings.
- The raster pipeline's `color_formats` grows from `[GBUFFER_RASTER_COLOR_FORMAT]` (1 throwaway) to `[R8G8B8A8_UNORM, R8G8B8A8_UNORM, R8G8B8A8_UNORM]` (3 MRT formats). `MAX_COLOR_ATTACHMENTS = 8` (`rhi_impl.rs` L130) so 3 fits. **One `VkPipelineColorBlendAttachmentState` per declared color_format** (blend OFF, `colorWriteMask = RGBA`) — the pipeline builder MUST emit exactly `color_formats.len()` blend-attachments; a count mismatch is a silent dynamic-rendering pipeline-creation failure with no validation. A host debug_assert pins `color_formats.len() == blend_attachments.len() == bound MRT count`.
- `GBufferScene::raster_pipeline`'s doc + the W2-b format contract: "declares 3 color formats = the G-buffer RGBA8 MRT formats" instead of "one throwaway color format."

⇒ The marcher's 16-binding vocab cap is provably untouched; r0 lives entirely in the raster pipeline's separate set.

---

## Integration — files touched

| File | Change |
|------|--------|
| `crates/boyko_rhi_vulkan/shaders/gbuffer_mrt.fs.hlsl` (NEW) | The MRT producer fragment shader: 3 SV_Targets in the marcher's encoding, oct/pack SPLICED from the `emit_hlsl_*` printer under BEGIN/END sentinels. |
| `crates/boyko_rhi_vulkan/shaders/gbuffer_mrt.vs.hlsl` (NEW or extend `gbuffer.vs.hlsl`) | Vertex: transform the quad, pass through world normal + LINEAR color. The existing `gbuffer.vs.hlsl` already emits NORMAL — extend to carry color. |
| their committed `.spv` pair (NEW) | DXC'd `ps_6_0` / `vs_6_0`, committed in the same commit (W4 blob-pin). |
| `crates/boyko_rhi_vulkan/src/device.rs` (~L1908-1947 region) | NEW boot caps fail-fast: `COLOR_ATTACHMENT` feature on `R8G8B8A8_UNORM` (mirror `gbuffer_storage_format_ok`). |
| `crates/boyko_rhi_vulkan/src/swapchain.rs` `GBufferTargets::create` | Add `COLOR_ATTACHMENT` to `albedo`/`normal`/`material`; gViewT UNCHANGED; DELETE `raster_color` (image + create + teardown + field). |
| `crates/boyko_rhi_vulkan/src/swapchain.rs` `record_gbuffer` | Pass A: bind 3 G-buffer images as MRT with the r0-2 CLEAR values (replace the throwaway); barrier-in `UNDEFINED → COLOR_ATTACHMENT_OPTIMAL` on the 3 images (replace step 0); barrier-out `COLOR_ATTACHMENT_OPTIMAL → GENERAL` with the **EXACT** `src=COLOR_ATTACHMENT_OUTPUT/COLOR_ATTACHMENT_WRITE`, `dst=COMPUTE_SHADER/SHADER_READ|SHADER_WRITE` spec (replace the `src=0`/`UNDEFINED` step 4 for these 3 images); depth barriers UNCHANGED; gViewT's `UNDEFINED→GENERAL` UNCHANGED. |
| `crates/boyko_rhi_vulkan/src/swapchain.rs` `GBufferScene` doc / W2-b | raster_pipeline declares 3 color formats; remove the throwaway-color contract. |
| raster pipeline construction (host test/driver) | Build with 3 `color_formats` + 3 per-target blend states; bind the new `.spv` pair. **Pre-delete check**: grep `raster_color` / `GBUFFER_RASTER_COLOR_FORMAT` across `swapchain.rs` + the offscreen/windowed-present drivers; update every reference atomically (a stale 1-format driver against a 3-attachment pass is a format/attachment-count mismatch). |
| `crates/boyko_rhi_vulkan/tests/gbuffer_mrt_edsl_sync.rs` (NEW) | Hard build-gate: the raster shader's oct/pack splice == the live `emit_hlsl_*` output (mirror `sdf_field_edsl_sync.rs`). |

**NOT touched by r0**: the marcher `.comp.spv` (frozen — gViewT-yield is r1b), `deferred_pbr.hlsl` (the resolve already reads the 3-lane encoding r0 produces; mask=1 mesh pixels route through Cook-Torrance with no resolve change), the gViewT image usage, `sdf_field.hlsli`, `ray_gen.hlsli`, `boyko_render`, `boyko_ecs`.

---

## Rung order (where r0 slots)

| Rung | What | Gate |
|------|------|------|
| **P5-r0 (THIS addendum, BEFORE r1)** | The 3-lane mesh-MRT producer: RGBA8 G-buffer images gain `COLOR_ATTACHMENT` (+ the RGBA8 caps fail-fast); pass A writes albedo/normal/material in the marcher's eDSL-spliced encoding, mask=1; the EXACT `COLOR→GENERAL` barrier; the throwaway-color deletion. gViewT + marcher untouched. | (1) No-mesh offscreen golden BYTE-IDENTICAL (r0's 0%-gate). (2) eDSL oct/pack splice byte-identity test (hard build-gate). (3) Mesh scene = status-quo appearance (un-gated marcher still wins). (4) Owner visual: a mesh+SDF windowed frame still looks like today. |
| P5-r1 | Marcher Site B + Site A ownership gate (3 attribute lanes). | No-mesh byte-identical; mesh pixels now show the RASTER's first-class G-buffer (mask=1 PBR, real normal). |
| **P5-r1b (gViewT, was draft r0-4)** | gViewT image gains `COLOR_ATTACHMENT` (+ the R32_SFLOAT caps fail-fast / directional-only fallback); raster adds `SV_Target3 = SV_Position.z*T_MAX`; marcher yields gViewT on `!own_pixel` — all ONE commit. | Mesh point/spot lit; exactly-once gViewT provable in a single diff. |
| P5-r2 | Marcher empty-tile gate symmetry (if not already in r1). | As locked plan. |
| P5-r3 | The B2 footprint tripwire (own_pixel footprint == raster footprint; r1b extends it to gViewT). | As locked plan. |
| P5-r4 | Owner windowed visual: mesh shows raster PBR + point/spot lit; SDF-in-front shows SDF. | Owner visual OK. |

r0 is the PRODUCER (3 lanes); r1 is the gate; r1b adds the gViewT/position-lighting; r2–r4 finish. r0 ships first, independently green.

---

## Verification (constraint 6: no validation oracle on this box)

- **r0 0%-gate**: the existing no-mesh offscreen ORTHO golden must be BYTE-IDENTICAL pre/post r0, run with `BOYKO_DISABLE_VALIDATION=1`. This discharges the central-wrinkle risk: if the COLOR_ATTACHMENT-usage / layout-cycle / clear-value / barrier change perturbed any no-mesh pixel, the golden breaks.
- **eDSL splice identity (compile-time, no GPU)**: `gbuffer_mrt_edsl_sync.rs` asserts the raster `oct_encode`/`pack_material_id_ba` splice == the live `emit_hlsl_*` output (the same printer feeding the marcher). Hard build-gate — caught at `cargo test`, not on a screenshot.
- **Caps fail-fast (boot)**: the RGBA8 `COLOR_ATTACHMENT` feature gate aborts at device init with a clear message if (impossibly) unsupported — never a silent device-lost.
- **Mesh G-buffer pixel golden (offscreen, RTX)**: a single-mesh-quad-over-empty offscreen scene; assert inside the quad footprint `gAlbedo`/`gNormal`/`gMaterial` hold the raster's encoded mesh values (mask=1, oct(quad normal), the base color). Pre-r1 the marcher overwrites these — so land this pixel-golden assertion in the **r0+r1 pair**, asserting the FINAL resolved mesh pixel is PBR-lit (mask=1) not flat MESH_COLOR.
- **Owner visual oracle (RTX, `BOYKO_DISABLE_VALIDATION=1`)**: offscreen screenshot → BMP→PNG → open by path → owner is the visual oracle. r0-alone: "looks like today." r0+r1: "the mesh shows real shaded normals / PBR, not flat green." Use an ASYMMETRIC mesh primitive (a box, not a sphere) so a wrong oct-normal axis-swap is visible; LEVEL camera; watch the windowed BGRA→RGBA R/B swap on readback. Commit only after owner visual OK (crash-prone GPU).
- **debug_assert invariants** (host): raster pipeline declared color-format count == blend-attachment count == bound MRT count (== 3); the three G-buffer images carry `COLOR_ATTACHMENT` before pass A binds them; the r0-2 clear-color constants == the marcher's background-arm constants (UNORM-exact).

---

## Open questions

- **OQ-r0-B — mesh material source + LINEAR-vs-sRGB watch-point.** P5-r0 uses the interpolated vertex color as `base` + a pushed material id. **The vertex color MUST be LINEAR** to match gAlbedo's "RAW LINEAR base color" contract (the marcher feeds linear base into Cook-Torrance). If the quad's vertex color was authored as an sRGB-ish constant to match the old flat `MESH_COLOR` *look*, feeding it as linear under mask=1 PBR will shift the mesh's lit appearance vs the old flat pass — the owner-visual oracle catches this, but it is a known watch-point. Confirm the vertex color is linear (or convert in the .vs). A material-table fetch (one StorageBuffer in the raster's OWN set) + textured PBR are charted follow-ups.
- **OQ-r0-C — the raster `.spv` pair re-pin (Owner-call).** r0 commits a NEW `gbuffer_mrt.{vs,fs}.spv` pair (the marcher blob stays frozen). The no-mesh byte-identical golden + the eDSL splice-identity test discharge the risk. Owner go/no-go on the new raster blob pair.
- **OQ-r1b-A — gViewT raster path vs directional-only fallback (deferred to r1b, was OQ-r0-A).** Does P5 want mesh point/spot lighting (r1b: gViewT `COLOR_ATTACHMENT` + R32_SFLOAT caps-gate + the 4th MRT + the marcher gViewT-yield) or is directional-only mesh lighting acceptable (keep locked Decision 3, no r1b gViewT mechanism)? Recommendation: take the r1b raster-gViewT path **iff** the R32_SFLOAT `COLOR_ATTACHMENT` caps-gate passes on the RTX; else fall back to directional-only automatically. This is now an r1b owner-call, cleanly out of r0.

---

**Ground (absolute paths):** raster pass A + barrier schedule + G-buffer usage in `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\src\swapchain.rs` (`record_gbuffer` ~L2375-2647 incl. step-4 barrier ~L2618-2645, `GBufferTargets::create` ~L4185-4307, formats ~L4119-4135, `GBufferScene` doc ~L3753-3815); the caps fail-fast pattern in `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\src\device.rs` (~L1908-1947); the marcher's terminal encoding + `has_mesh`/`t_mesh` + the two write sites in `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\shaders\sdf_gbuffer_composite.hlsl` (Site A empty-tile ~L1314-1331 incl. gViewT ~L1329, Site B ~L1588-1653 incl. gViewT ~L1653, the GENERATED oct/pack spans ~L507-527, `t_mesh = md*T_MAX` ~L1284-1286); the eDSL bodies in `D:\claude\BoykoEngine\crates\boyko_shaderdsl\src\oct.rs` + `D:\claude\BoykoEngine\crates\boyko_shaderdsl\src\pack.rs`; the resolve's read contract in `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\shaders\deferred_pbr.hlsl` (L11-31, 62-65); `MAX_COLOR_ATTACHMENTS`/blend-array in `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\src\rhi_impl.rs` (~L130, ~L1554); the MRT precedent `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\shaders\gbuffer.fs.hlsl`; the locked plan `D:\claude\BoykoEngine\docs\RENDER-P5-HYBRID-PLAN.md`.

---

## Critique resolutions

**BLOCKER 1 (caps lens) — no `COLOR_ATTACHMENT` caps fail-fast; R32_SFLOAT color-attachment is OPTIONAL.** RESOLVED. (a) Added a boot-time `VK_FORMAT_FEATURE_COLOR_ATTACHMENT_BIT` fail-fast on `R8G8B8A8_UNORM` (Decision r0-1 step 2, mirroring `gbuffer_storage_format_ok`). (b) Removed R32_SFLOAT from r0 entirely — gViewT no longer gets `COLOR_ATTACHMENT` in r0 (it moved to r1b), so r0 has no optional-format dependency. The R32_SFLOAT `COLOR_ATTACHMENT` caps-gate ships WITH r1b, with an automatic directional-only fallback if absent on the RTX.

**BLOCKER 2 (caps lens) — the barrier source half is `src=0`/`TOP_OF_PIPE` (copy of the `UNDEFINED` shape); a genuine raster write needs `COLOR_ATTACHMENT_WRITE`.** RESOLVED. Decision r0-1 step 3 now specifies the barrier-out EXACTLY as a NEW spec: `src_stage=COLOR_ATTACHMENT_OUTPUT`, `src_access=COLOR_ATTACHMENT_WRITE`, `old_layout=COLOR_ATTACHMENT_OPTIMAL`, `dst_stage=COMPUTE_SHADER`, `dst_access=SHADER_READ|SHADER_WRITE`, `new_layout=GENERAL`, per-image batched — explicitly NOT the `src=0`/`UNDEFINED` step-4 shape. The Integration table and verification call this out.

**BLOCKER 3 (0%-gate/scope lens) — Decision r0-4 mutates the frozen marcher and breaks r0's separable-commit / frozen-`.comp.spv` claim; gViewT producer/consumer is split-brained between r0 and the deferred amendment.** RESOLVED by the recommended fix: the entire gViewT raster mechanism (4th MRT, gViewT `COLOR_ATTACHMENT` usage, the marcher gViewT-yield amendment) is moved to **r1b** — a sub-rung where the marcher is already being edited to yield, so the raster gViewT write + the marcher gViewT yield land in ONE diff and the exactly-once invariant is provable atomically. r0 now writes only the three RGBA8 lanes; the marcher `.comp.spv` is genuinely frozen in r0; the separability claim is restored.

**BLOCKER 4 (eDSL lens) — "carries the identical generated spans" is ambiguous: generated artifact vs checked copy; the owner rule mandates single-source splice.** RESOLVED. Decision r0-3 now states the spans are produced by the SAME `emit_hlsl_oct_encode`/`emit_hlsl_pack_material_id_ba` printer call (one emission, two splice sites) under identical BEGIN/END sentinels, and the sync test is the GUARD on the splice, not the source of truth — pinned by name to the `sdf_field_edsl_sync.rs` mechanism as a hard build-gate.

**Concerns folded in:** (1) gViewT `SV_Position.z` (not a custom linear varying) pinned in r1b for bit-identity with `t_mesh`. (2) Per-target blend-attachment count == `color_formats.len()` debug_assert added (binding plan + verification). (3) `raster_color` deletion now lists an explicit pre-delete grep across `swapchain.rs` + the offscreen/windowed drivers. (4) The eDSL sync test is a hard build-gate (no ship on failure). (5) The empty-tile early-return (Site A, ~L1329) is now explicitly closed in the 0%-gate proof as the one path that could expose a raster-cleared-but-marcher-unread texel. (6) UNORM clear-quantization note added (clears go through the same `round(c*255)` quantizer as the marcher store; `0.5/1.0/0.0` are exact) + a host debug_assert pinning clear constants to the marcher's background-arm constants. (7) LINEAR-vs-sRGB vertex-color watch-point captured in OQ-r0-B.

**Rejected:** none.
