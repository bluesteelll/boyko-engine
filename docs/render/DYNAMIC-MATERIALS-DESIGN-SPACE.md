# Dynamic materials: the design for THIS engine

> **Status:** architect's design, 2026-09-25, **pass 2**: revised after the first architecture
> critique (CHANGES_REQUESTED: 1 critical, 10 important, 9 optional). Every remark is resolved in
> the text; §11 (*Review log*) lists each one with its verdict and where it landed.
>
> **Survey.** The survey this design rests on is
> [`DYNAMIC-MATERIALS-RESEARCH.md`](DYNAMIC-MATERIALS-RESEARCH.md). Tree facts are cited from its §1,
> re-opened at trunk **`6394bc5e`**, and follow its anchor rule: a `path:line` appears only where the
> line was re-read at that commit.
>
> **Numbers.** Every number below is one of two kinds:
> - external and cited with its rig;
> - a **labelled estimate [E]**, with the arithmetic shown.
>
> Nothing was timed. No `cargo` command was run. The reference GPU is the owner's **RTX 3060**:
> - 12.8 FP32 TFLOPS with FMA counted as two operations (3584 cores × 1.78 GHz × 2, per NVIDIA's spec
>   page); **6.4 T lane-instructions/s** for converting an instruction count into time;
> - 360 GB/s memory bandwidth;
> - compute capability 8.6: 48 warps and 64 K registers per SM (RESEARCH, reference-GPU note).
>
> **Decision split.** §9 separates the technical forks decided here from the VALUES/SCOPE questions
> that go to the owner. Pass 2 withdrew three questions that were technical (§9.3).
>
> **Scope.** Dynamic materials only. The requesting message also names post-FX/AA, explosion and
> rain effects, sun rays and volumetrics. Those are other research threads. This design meets them
> only at the material seam:
> - flipbooks and per-instance timing for explosion and fire *meshes*;
> - hit flash and dissolve;
> - one gating policy for the lit shaders, shared with the effects thread (F9).
>
> Rain wetness is **not** designed here. The effects design owns it (F6, §2).

## 0. The shape in one paragraph

**Evaluate every time-varying quantity at the lowest frequency that is exact, and keep the GPU
time-free.** Concretely:

- **Per material, per frame, on the CPU.**
  - Driver entities compute a *modulation* over a material's **authored** value. They never
    overwrite the authored value, so gameplay edits and hot reloads compose with them.
  - The composed **effective** value reaches the GPU table through one `material_upload` copy pass,
    the light table's existing idiom, fed by an ECS stager system, so the runner takes no `&mut`
    borrow.
  - The same effective value feeds the per-instance lanes the gather fills. So every lit producer
    and both geometry legs see one value.
- **Per instance, per frame, in the gather that already runs.**
  - A `MaterialOverride` component becomes a compact override table.
  - Its slot index rides on the per-instance lane **each path already fetches**:
    - on VB, `VbInstanceRow._pad[0]`, uploaded every VB frame and loaded by every VB shader;
    - on the raster paths, `PerInstanceMaterial._pad[0]`.
  - Per-instance drivers give each instance its own clock: staggered flipbooks and out-of-phase
    flicker.
  - SDF edits take the same component through a private material row per edit.
- **Per pixel, only for the three operations that read a per-pixel input:**
  - the UV transform before sampling;
  - the flipbook cross-fade;
  - the fresnel rim.

  They sit behind a **specialization constant**, never a runtime branch in a shipped pipeline. When
  the capability is off, only the `false` pipelines exist. When it is on, the host picks the `false`
  or the `true` pipeline per frame.
- **Dissolve** rides the transparency design's `MASKED` rail, casters included.
- **Vertex animation and arbitrary material programs (Tier 3)** are later rungs, gated by
  prerequisites and by the owner's source-vs-asset decision.

**What rungs DM1–DM5 add:**
- one graph pass, three small tables, a handful of generated eDSL leaves;
- one RHI field (fragment-stage specialization constants);
- up to 21 extra pipelines (the `true` specializations, built only when the per-pixel capability is
  on).

**What they do not add:** `.spv` files, unless a site fails the fold gate (then one row for that
site, F9), or a GPU time source.

---

## 1. What earlier documents decided, and what this one changes

### 1.1 Earlier decisions this design inherits

| Source | Decision |
|---|---|
| [`PBR-MATERIALS-PLAN.md`](../PBR-MATERIALS-PLAN.md) D2/D3/D4 | A material SSBO indexed by a 16-bit id; the G-buffer id is `R16_UINT`; the SDF edit's material id lives in `SdfEdit.center.w` with no stride change. |
| Owner memo, 2026-08-27 (session memory, not in the tree) | Programmable materials are their own campaign, in three tiers. **Tier 1** is animated parameters ("works today"). **Tier 2** is a fixed procedural vocabulary: static permutations, a named selector, parameters in the same 48 B, "5–10 `.spv`". **Tier 3** is arbitrary per-material code. It has three blockers: indirect dispatch missing from the FFI; N `.spv`, manifest rows and byte gates; and the **policy question**, source or asset. Build nothing for Tier 3 until the policy is decided. Order: document Tier 1 → finish VB-P2 → Tier 2 → policy → Tier 3. |
| [`VB-P2-CLASSIFICATION-PLAN.md`](../VB-P2-CLASSIFICATION-PLAN.md) D1/D2/P1-4 | Full-screen per-material bins; over-dispatch with a sentinel "because the FFI lacks indirect dispatch"; a host selector that keeps the fused `vb_resolve` for flat frames. |
| [`TRANSPARENCY-DESIGN-SPACE.md`](TRANSPARENCY-DESIGN-SPACE.md) §1.2, §9 | `MaterialGpu` stays byte-frozen at 48 B. New per-material parameters go in a **cold extension table** indexed by the same id and bound only by the passes that read it (`MaterialXGpu`). Cutout rides a `-D MASKED` raster rail. |
| [`TRANSPARENCY-UPDATE-2026-09-25.md`](TRANSPARENCY-UPDATE-2026-09-25.md) D-U6 | Masked casters get their own `With<AlphaMasked>` query pair (ok + stale) and a `MaskedCasterScratch`, so the opaque caster path stays byte-identical. |
| [`RENDER-PARITY-PLAN.md`](../RENDER-PARITY-PLAN.md) §3.6 | For **one texture load** on one shader, a runtime-uniform gate with a one-time re-pin was preferred over a `-D` axis. |
| [`PARTICLES-PLAN.md`](../PARTICLES-PLAN.md) F24 (`:210`) | **A dark feature is not free.** The VB-SV0 inline march cost **+75 %** of the fused `vb_resolve` dispatch (24,576 → 41,984 ns) with the feature **off** (`crates/boyko_rhi_vulkan/shaders/sdf_mesh_shadow.comp.hlsl:7-9`). SV0's remedy: a dedicated pass, a per-producer byte budget, and a dark-dispatch ABBA A/B with a pre-registered budget ([`OPEN-QUESTIONS.md`](../OPEN-QUESTIONS.md), 2026-08-20 "SV0 returns as the dedicated pass"). |
| [`VFX-DESIGN-SPACE.md`](VFX-DESIGN-SPACE.md) §2.4, §7.4, FX10 | Wetness, puddles and snow cover are per-pixel eDSL leaves in every lit producer under **`-D WEATHER`**. A runtime branch in the same producers was rejected as "the F24 dark tax". Porosity v1 is the heuristic `(1 − metalness) · roughness`; an authored porosity lane is its owner question B11. |
| [`REFLECTIONS-UPDATE-2026-09-25.md`](REFLECTIONS-UPDATE-2026-09-25.md) §3.7 (U7, RK-16) | Indirect dispatch is recorded through the raw `fns.cmd_dispatch_indirect`, the particles shape. RK-16: the RHI trait's silent no-op becomes loud or implemented. |
| UI animation decisions (AD9, `crates/boyko_ui/src/animation.rs` module doc) | Endpoint tweens run on the real clock by default and on the virtual clock per row; looping effects such as flipbooks run on the virtual clock. `EasingId` is a `u8` (30 built-ins, a custom half); the built-in bodies are UI rung A2. |

### 1.2 Unchanged

- **The three-tier ladder** is the owner's shape, and it stays.
- **The 48-B `MaterialGpu`** stays frozen, including its flag bits.
- **The Tier 3 policy question** stays the owner's, and it still blocks Tier 3 infrastructure.
- **Shader compilation** stays offline and hermetic, with committed `.spv` files.
- **Parity across the four render paths and both geometry legs** stays the goal. Where it is not
  reached, the gap is stated per path (F13, F16).

### 1.3 What changed, and why

| Earlier statement | Now | Why |
|---|---|---|
| Tier 1 "works today" | Tier 1 does **not** work. **DM1** makes it work and fixes two live defects on the same path. | `flush_if_dirty` has no caller and nothing copies its staging ring (RESEARCH §1.1–§1.2). The defects are RESEARCH §1.11. |
| Tier 2 = static permutations, 5–10 `.spv` | Tier 2 splits in two. **2a:** values that vary with time and a per-material or per-instance parameter, evaluated on the CPU (0 `.spv`). **2b:** the three per-pixel operations, as a **specialization constant** on the existing `.spv` (0 new `.spv`, one re-pin, a `false`/`true` pipeline pair). | Most requested effects have no per-pixel input (RESEARCH §2.12). A `-D` axis multiplies `.spv` across the existing axes *and* across the effects thread's `-D WEATHER`; a specialization multiplies only pipelines, and only the ones built (F9). |
| Blocker 1: `vkCmdDispatchIndirect` is missing from the FFI | Stale. It is loaded and used by particles. What remains is the RHI trait's no-op `dispatch_indirect` (reflections RK-16) and `INDIRECT_BUFFER` usage on `gClassify`. | RESEARCH §1.4. |
| VB-P2 is dark infrastructure | P2c is live. The classify chain already runs on every textured VB frame. | RESEARCH §1.5. This lowers Tier 3's incremental cost on VB (F12). |
| Order: document Tier 1 → finish VB-P2 → Tier 2 → policy → Tier 3 | DM1 → DM2 → DM3 → DM4 → (transparency R2) → DM5 → **policy** → DM6. DM7 (vertex animation) is separate. | Tier 1 must be *made* true before it can be documented. VB-P2 is already finished far enough. |

**Pass 1 → pass 2, the decisions that moved:**
- F4: drivers modulate an effective value instead of overwriting the authored one.
- F7: the VB carrier moved to `VbInstanceRow`; the raster upload rule and pipeline selector changed.
- F9: a specialization constant replaced the runtime gate.
- F12: the uber-shader became the default.
- F16: SDF edits take the same override component.
- Wetness went to the effects thread.
- F17 (per-instance time) is new.

§11 gives the reason for each.

---

## 2. Where every requested effect lands

| Effect | Varies with | Where it is evaluated | Rung |
|---|---|---|---|
| Event change (a door turns red, a material swap) | a gameplay event | `Assets::get_mut` edits the **authored** row → edited bit → `stage_material_edits` → `material_upload` | DM1 |
| Pulsing or gradient emissive, blink, flicker, colour cycling (all instances in phase) | time × per-material params | the CPU driver `drive_materials` → the material's effective record | DM2 |
| Scrolling UV | time → a per-material offset; `uv` is per pixel | the driver writes the effective `MaterialDynGpu.uv_st`; the shader applies `uv·s + o` | DM2 (value) + DM4 (apply) |
| Flipbook (explosion or fire *meshes*; particles keep their own path) | time → a fractional frame, per material **or per instance** | a driver writes `frame`; the shader derives cells A and B from the material's grid, samples twice and blends | DM2 + DM4; per instance DM3 + DM4 (F17) |
| Hit flash, per-instance tint | per instance × time | gameplay or an `InstanceMaterialDriver` writes `MaterialOverride` → gather → override table | DM3 |
| Out-of-phase flicker, staggered explosions | per instance × time | `InstanceMaterialDriver` (own `t0` and phase) → the override's `k` (emissive) and `frame` | DM3 (+ DM4 for `frame`) |
| Fresnel rim | per pixel (N·V) × per-material params | the specialized leaf in every lit producer | DM4 |
| Dissolve | per-instance progress × per-pixel noise | the `MASKED` raster and the masked casters, with the override threshold | DM5 (after transparency R2) |
| Rain wetness, puddles, ripples, snow cover | per pixel (shelter, puddle noise) × global level | **the effects design**, `-D WEATHER` leaves ([`VFX-DESIGN-SPACE.md`](VFX-DESIGN-SPACE.md) FX10). It composes on top of this design's effective material values. | not here |
| Triplanar / procedural noise | per pixel | Tier 3, or baked to a texture offline (zero runtime cost, static only) | DM6 / tool |
| Wind, vertex animation textures (VAT) | per vertex × per instance × time | the vertex stages plus the VB re-fetch | DM7 (a separate campaign) |
| Arbitrary graphs | anything | per-program shading | DM6 (after the policy decision) |

**The same gameplay code on both geometry legs.** `MaterialOverride` (tint, flash, `k`) works on a
mesh entity and on an SDF edit entity (F16). The fields that need UVs or a masked raster (`frame`,
the dissolve `param`) are mesh-only, because SDF has neither; a debug build warns once when one is set
on an SDF edit.

---

## 3. The design space, fork by fork

Each fork records the options, their numbers, how each fares against the owner rules (**P0** ECS
storage and structural capability; **HP** hot path; **IH** in-house; **SH** shaders), and the
decision.

### F1: How a Tier-1 edit reaches the GPU table

| Option | Cost per edit frame | Idle frame | Owner rules | Verdict |
|---|---|---|---|---|
| A1: write the (host-visible) table directly from the CPU | ~0 | 0 | **Violates the in-flight write-after-read rule.** The table is bound by frame N−1 while the CPU writes for frame N. `material_table.rs` itself records why the staging ring exists. | rejected |
| A2: restage the whole table into the fenced staging slot and copy all of it | 48·H B memcpy + copy: H = 4,096 → 192 KB (19–38 µs CPU at 5–10 GB/s write-combined [E]); H = 65,536 → 3 MiB (0.31–0.63 ms CPU, ≈ 0.13–0.2 ms copy at 16–25 GB/s PCIe 4.0 x16 [E]) | 0 | OK | rejected on cost at the cap; kept for frame 0 and the grow frame (F3) |
| A3 (pass 1): edited rows, **row-mirrored** staging (each row at its own offset in slot `s`), ≤ 64 runs as regions, an overflow path that re-seeds `[min_row, max_row]` | k × 48 B + copy | 0 | OK, but correct only through the overflow re-seed, and a naive range copy breaks it | superseded |
| **A4: edited rows, compact staging.** Rows are packed in drain order at `[0, k·48)` of slot `s`. One copy region per run of consecutive rows: the drain is in ascending row order, so a run is contiguous on both sides. More than 64 runs → several `vkCmdCopyBuffer` calls from one 64-entry stack array. | k × 48 B memcpy twice (asset → CPU staging in the stager; CPU staging → the mapped slot in the runner) + the copy. k = 100 → 4.8 KB, ≈ 1–2 µs [E] | 0: no pass declared, no copy recorded (the `light_upload` idiom) | OK P0: the edited set is a kernel feature of `Assets<T>` (F2). OK HP: fixed stack region array, preallocated Resource-owned byte column, no heap on the frame. | **chosen** |

**Invariant, stated because a naive coalescing breaks it.** *Every byte range copied from staging
slot `s` in frame N must have been written into slot `s` in frame N.*
- The slots are per frame in flight. A row edited in frame N−1 was written only into the other slot.
- A4 satisfies the invariant **by construction**: the regions reference only `[0, k·48)` of slot `s`,
  which the runner wrote this frame. There is no overflow path to get wrong.
- DM1's red test (2) pins it with a row layout that a `[min_row, max_row]` copy from a row-mirrored
  slot provably fails (§7).

### F2: Dirtiness granularity

`Assets::dirty_gen` is per table. The existing `dirty: LiveBitmap` means *freed or retired*
(streaming lifetime), not *edited* (RESEARCH §1.1).

**Decision:** a new kernel column `edited: LiveBitmap` on `Assets<T>`, plus a count.
- **Meaning:** the GPU image of this row may differ from the CPU authority.
- **Set by** `get_mut`, `add`, `fill` **and `free`**. `free` is in the set so that a freed row is staged
  as zeros, which keeps the host's `dyn_rows` count exact (F9).
- **Drained by** `Assets::drain_edited(|row| …)` in ascending row order.
- **`edited_any()` is O(1)** through the count, so an idle frame never scans the bitmap.
- **Cost:** one bit per row (8 KiB at 65,536 rows) and one branch-free `OR` plus one add per
  `get_mut`.

It is a first-class kernel feature (Principle 0), not a render-side side store. Meshes and textures
get it for free when they need it. `dirty_gen` stays; its existing readers keep their meaning.

**Why not a render-side list of edited handles:** a second authority beside the asset, and it would
be wrong exactly when a non-render system calls `get_mut`.

**What `add` and `fill` in the set fix.** Live defect D-1 (RESEARCH §1.11): a material minted or
streamed into the table's power-of-two headroom after a grow keeps an all-zero GPU row. With `add` and
`fill` setting the edited bit, that row is staged on the frame it appears.

### F3: Where the table lives, and how it grows

**Today.** Both the table and the staging ring are `HostVisibleCoherent`. The allocator picks the
first `HOST_VISIBLE|HOST_COHERENT` type, which on common NVIDIA type orders is system memory
(RESEARCH §1.1, an inference, unmeasured).

**Decision:** a **device-local** table, mirroring the light table, with a host staging ring.
- **Correctness does not depend on this.** The copy protocol works into either memory kind.
- **Perf is the reason.** Every lit producer fetches `Materials[id]` per pixel, and on an L2 miss a
  system-memory table crosses PCIe.
- **Cross-frame ordering is inherited.** Frame N+1's copy writes the table that frame N's shaders
  read. The light table already has this shape, and the graph derives a cross-frame seed-WAR buffer
  barrier for its `light_upload` pass (`record_vb`'s comment on it). `material_upload` is declared
  the same way and gets the same barrier.
- **Gate:** DM1 (c) times `deferred_pbr` and `vb_resolve` with both memory kinds on the RTX 3060.
  The device-local version must not be slower.

**The grow frame.** Today's grow seeds the new table by writing through its mapping
(`crates/boyko_render/src/material_table.rs:430`, `expect("host-visible grown material table is
mapped")`). A device-local table has no mapping, so a literal port would panic. The protocol, on a
grow frame N with fenced slot `s`:
1. Create the new device-local table T′ (capacity C′), `STORAGE | TRANSFER_DST`.
2. If staging slot `s` is smaller than C′ × 48 B, replace it now. Slot `s` is fenced, so this is safe.
   The other slot is replaced at **its own** next fenced occupancy (the O3 rule `material_table.rs`
   records).
3. Write the **full image** into slot `s`: every row's effective value (F4), zeros elsewhere. This is
   today's `seed_rows`, pointed at the staging mapping instead of the table's.
4. Record `material_upload` with **one** region, `[0, C′ × 48)` → T′. This frame's edited runs are
   **not** recorded: the full image was built after the stager ran, so it already contains them.
5. T′ is new, so the copy has no earlier reader and needs no WAR barrier. The recorder's barrier
   before this frame's readers is the normal TRANSFER → SHADER one.
6. Repoint the per-slot descriptor sets through the existing `rebind_pending`. Retire T with the
   existing `RETIRE_DELAY`.
7. Frame N+1 (the other slot) copies its own edited runs into T′. The cross-frame barrier orders
   them after frame N's reads.

Frame 0 is the same full-image copy into the boot table. The `MaterialDynGpu` table (F10) has the
same capacity, grows in lockstep, and is covered by the same frame-0 and grow copies.

**Cost:** the grow frame copies C′ × 48 B once: 3 MiB at the cap ≈ 0.13–0.2 ms [E]. That frame
already pays today's host seed of the same bytes.

### F4: Per-material time-varying values: CPU drivers, composed onto the authored value

**The deciding fact.** The per-instance lanes `PerInstanceMaterial.base_color` and
`PerInstanceMaterialTex{base_color, metallic, roughness}` are CPU copies. The gather refills them
from `Assets<Material>` every frame (RESEARCH §1.1). The Deferred `pm`/`tex` raster and the textured
VB tails read `base_color` **from these lanes, not from the table**. Any GPU-side evaluation would
therefore desynchronize the paths.

**Where the result goes.** Pass 1 had drivers overwrite the asset row. That breaks three things:
- a gameplay `get_mut` recolour of a driven lane is overwritten on the next frame;
- a hot reload is overwritten too;
- two drivers on one lane cannot compose.

Pass 2 keeps the authored value intact and derives an effective one.

| Option | CPU / frame | GPU / frame | Consistent across paths and legs? | Owner rules | Verdict |
|---|---|---|---|---|---|
| B1 (pass 1): drivers overwrite `Assets<Material>` rows | K × 12–25 ns [E] | 0 shading | yes | clobbers events and reloads; no composition | withdrawn |
| **B1′: drivers write a plugin-owned effective record. The stager and the gather read effective where a record exists, the asset otherwise.** | per driver: ≈ 12–25 ns [E: polynomial curve ~20 flops + handle resolve + 16–48 B write]. Per driven material: reset + compose + a 48-B compare, ≈ 10 ns [E]. Gather: +1 `u16` load and a predictable branch per instance, ≈ 0.3–1 ns [E] (100k instances ≈ 0.03–0.1 ms), and **0 when no material is driven** (the test is hoisted out of the loop). K = 1,000 drivers ≈ 12–25 µs. | **0** shading; copy ≈ 1–5 µs at K = 1,000 [E] | **Yes, by construction.** The table and the per-instance lanes are both built from the same effective value in the same frame. SDF pixels read the same table. | OK P0: driver entities + one Resource-owned column; OK HP; OK SH (no shader change) | **chosen** |
| B2: a GPU `material_animate` compute pass rewrites table rows from `(curve, params, t)` | ~1 µs to record | one dispatch of `ceil(K/64)` groups plus a barrier ≈ 5–15 µs [E] | **No.** Animated `base_color`, metallic and roughness would not reach the lanes the Deferred `pm`/`tex` raster and the textured VB tails read. | New `.spv` + manifest row; the table needs `STORAGE` write | rejected now; escalation path |
| B3: every lit producer evaluates the curve per pixel from a global time | 0 | 10–30 ALU/pixel: 1080p ≈ 21–62 M lane-instructions ≈ **3–10 µs** at 6.4 T/s; 4K ×4 [E]. Pass 1 said 2–5 µs, having divided by the FMA-doubled rate. | Only if every lit producer, SDF sites included, carries the leaf and a time carrier. The camera blocks differ per stage (RESEARCH §1.3). | FAILS SH: ~20 `.spv` re-pinned plus a time carrier in each block; the f32 precision pitfalls of RESEARCH §2.13 | rejected |

**The composition rule (several writers of one lane).** For each `(material, lane)`, per component:

```
effective = R × Π M_i + Σ A_j
R   = the output of the lane's one Replace driver, if any; else the AUTHORED value
M_i = outputs of Mul drivers (default 1),  A_j = outputs of Add drivers (default 0)
```

- **At most one `Replace` per `(material, lane)`.** A debug build asserts it with one bitmap per lane:
  7 lanes × 8 KiB = 56 KiB, debug only.
- **Order is fixed.** Π and Σ fold in the dense query's storage order. That order is deterministic for
  a given spawn history, and goldens use fixed scenes.
- **Driver + event.** `get_mut` edits the authored value, and the next drive recomputes the effective
  value from it. Under `Mul`/`Add` drivers the recolour composes: a pulsing door turned red pulses
  red. Under a `Replace` driver the lane is owned by that driver by definition. Gameplay then
  recolours by editing the driver's parameters, which the driver's doc states.
- **Driver + reload.** A hot reload `fill`s the authored value, which composes in the same way.
- **Driver + driver.** A wet, colour-pulsing material is a `Mul` pulse here plus the effects thread's
  per-pixel wetness downstream, which composes by construction (§2).
- **Skip if equal.** A driven material whose new effective value equals the stored one (a 48-B
  compare) sets no changed bit, so the flat parts of `Square`/`Ease` upload nothing.

**Why B1′ wins for the hybrid engine specifically:**
- It is the only option that reaches **both geometry legs and all four paths with zero shader
  change**.
- Its cost scales with the number of *animated materials* (tens to hundreds in a scene; the tree
  caps rows at 65,536), not with pixels. The gather pays per instance only while at least one
  material is driven.

**Precedent.** Destiny's TFX compiles material expressions to a bytecode that is interpreted on the
CPU or on the GPU (RESEARCH §2.14). Evaluating material expressions on the CPU is shipped practice,
not a workaround.

**Escalation trigger** (recorded, not scheduled): if a profiled scene shows `drive_materials` +
`material_upload` above **0.1 ms CPU**, move the periodic subset to B2. That move comes *after* the
per-instance lanes stop carrying material copies. That refactor is a later rung, per the owner's
"refactor after everything".

### F5: Clocks and precision

- **Drivers read the kernel `Time`**, virtual by default. A per-driver `REAL_CLOCK` flag switches to
  the real clock. This is the UI AD9 split: a looping effect under a pause menu and in slow motion
  should follow the game; a tween that must finish while paused opts into real time.
- **The phase is computed in f64 from `Time::elapsed()`,** the integer-nanosecond `Duration`, as
  `frac(f · t)`, and only the result is narrowed to f32.
  - Precision stays below 1 µs of phase error for ~10⁹ s [E: f64 has 53 mantissa bits].
  - None of the wrap, rollover or half-precision pitfalls from RESEARCH §2.13 apply.
  - No GPU clock is needed in DM1–DM5. Per-instance clocks (F17) are CPU-side too.
- **The only GPU time consumer is vertex animation** (DM7, per vertex), plus F17's GPU escalation if
  it is ever taken. The policy is recorded here so it is not re-derived:
  - deliver `t mod P` with **P = 1,024 s**, computed exactly from integer nanoseconds;
  - **quantize every authored rate to a multiple of 1/P** at authoring time (frequency resolution
    ≈ 0.001 Hz), so `sin(2π f t)` and every scroll stay continuous across the wrap;
  - for 512 ≤ t < 1,024 s, one f32 ulp is 2⁻¹⁴ s ≈ 61 µs, and rounding errs by at most half of that,
    ≈ 31 µs. A 10 Hz term's phase error is then ≤ 2π·10·31 µs ≈ **0.002 rad** [E]. Pass 1 said
    2⁻¹³ s and 0.008 rad.

### F6: The curve vocabulary and golden determinism

The curves are:

| Kind | Curves |
|---|---|
| Periodic | `Sine`, `Triangle`, `Square{duty}` (blink), `Flicker{seed, rate}` (integer hash) |
| Endpoint | `Ease{EasingId, duration}` |
| Structural | `Scroll{rate}`, `Flipbook{frames, fps, looping}` (writes a fractional frame) |
| Output | `Gradient{a, b, mode}`: ping-pong or loop, the owner's own "shimmering gradient" |

Each driver also carries its composition operator: `Replace`, `Mul` or `Add` (F4).

**Pass 2 withdrew `Wetness`.** The effects design owns wetness as a per-pixel leaf ([`VFX-DESIGN-SPACE.md`](VFX-DESIGN-SPACE.md) §2.4, FX10). That is also the right owner on the merits:
- wetness depends on **shelter**, per pixel: its occlusion map keeps surfaces under a roof dry;
- a per-material CPU curve cannot know shelter, and would wet the whole material everywhere;
- keeping both would darken surfaces twice.

The material seam is this: the WEATHER leaf applies per pixel to whatever effective material values
this design produced. An authored porosity lane, if the owner wants one, is that thread's question
B11.

**Decisions:**
- **Every curve is an eDSL leaf** (`boyko_shaderdsl`). The CPU driver calls its `f32` instance. The
  HLSL instance exists for DM7 wind, for F17's GPU escalation, and for any later per-pixel use.
- **`Sine` is a range-reduced, FMA-free polynomial** built from eDSL arithmetic, not the `Sin` node.
  The node's `f32` instance is the host `f32::sin` (platform libm, RESEARCH §1.7). A 1-ulp
  MSVC-vs-glibc difference can flip an 8-bit golden at a rounding boundary. This is the same host
  sensitivity the tree has already been bitten by once (the gnu→msvc optimisation that inverted).
  **`Exp2` gets the same treatment** for the `expo` and `elastic` easing families: exact integer part
  by exponent bits, polynomial fraction. `sqrt` (the `circ` family) is correctly rounded by IEEE 754
  and needs nothing.
- **The endpoint curves use one shared easing table, landed by DM2.**
  - Today `boyko_ui`'s `ease()` is the identity (`crates/boyko_ui/src/animation.rs:600`). The 30
    built-in bodies are UI rung A2, which nothing has scheduled.
  - DM2 therefore lands the 30 built-in bodies itself, as eDSL leaves in **`boyko_shaderdsl`**. That
    crate has **zero dependencies** (its `Cargo.toml`), so `boyko_ui` can depend on it without a cycle.
  - `EasingId` (`crates/boyko_ui/src/components.rs:1016`) moves there with them. `boyko_ui`
    re-exports it, so `boyko_ui::EasingId` keeps its path and no UI caller changes.
  - UI rung A2 shrinks to replacing `ease()`'s identity body with a call into the shared table, plus
    its own custom-LUT half (ids `128..=255`), which stays UI-owned.
  - If A2 happens to land first, DM2 moves A2's bodies instead. Exactly one rung lands them.
  - DM2's gate carries an `Ease` red-first check of its own (§7), so linear easing cannot pass.
  - A second curve vocabulary is the thing Principle 0 forbids.
- **Drivers are pure functions** of `(t, params, authored)` and **never read the effective value they
  write.** So they are idempotent, frame-rate independent and free of drift.

### F7: The per-instance override data path

**Who reads per-instance material data, per carrier** (RESEARCH §1.9, corrected in pass 2):

| Carrier | Readers | Uploaded |
|---|---|---|
| `VbInstanceRow` (64 B; `_pad: [u32; 2]` free at offset 56, `crates/boyko_render/src/instance_model.rs:270`) | **every** VB shading shader, flat and textured, through `vb_geom_fetch`, which loads the whole row | **every VB frame**, unconditionally |
| `PerInstanceMaterial` (32 B; `_pad` 12 B free) | the 6 flat VB tails and the classify (`.id` only), the Deferred `pm`/`mvpm` raster, `forward_opaque.vs` | only on `any_non_default_material` frames (`crates/boyko_app/src/runner.rs:1788`) |
| `PerInstanceMaterialTex` (48 B; no spare) | the **4 textured VB tails**, at the same binding 1 through `vb_set0_tex{,_froxel}` (`crates/boyko_rhi_vulkan/shaders/vb_shade.comp.hlsl:92-110`; `crates/boyko_rhi_vulkan/src/present/targets.rs:657`, `:687`), and the Deferred `tex` raster | only on `any_textured_material` frames |

**Pass 1's carrier was wrong for the textured VB tails.** It put the slot in
`PerInstanceMaterial._pad[0]`, which the four textured VB tails never bind. Hit flash and tint would
have silently failed on every textured VB frame.

| Option | Upload / frame | Reach | Owner rules | Verdict |
|---|---|---|---|---|
| D1: a unique material row per instance (the UE/Unity per-object-instance anti-pattern) | 48 B × edited rows | all paths, both legs | FAILS one VB classification bin per instance, the Nanite MID-bin tax (RESEARCH §2.1); a 70k-instance crowd cannot be expressed under the 65,536-row cap | rejected for meshes; **kept for SDF edits** as private rows (F16) |
| D2: a dense lane aligned to the instance index, 16 B per instance | 16 B × N whenever any override exists: N = 100k → 1.6 MB; N = 1M → 16 MB [E] | needs a new binding in every consumer | OK | rejected on bandwidth in the common case (below) |
| D3 (pass 1): the slot in `PerInstanceMaterial._pad[0]` for every reader | 32 B × K | misses the 4 textured VB tails; wrong on Deferred for default-material entities (W2) | — | withdrawn |
| **D3′: the slot on the lane each path already fetches, plus a compact override table** | **VB: 0 extra bytes** (the VB ring uploads every frame anyway) + 32 B × K. **Raster:** the `PerInstanceMaterial` ring (already uploaded on any scene with a non-default material) + 32 B × K. | all 10 VB tails; every Deferred raster variant; Forward | OK P0: component → gather → a `ScratchColumn`; structural zero | **chosen** |
| D3″: pack the slot into the upper 16 bits of `PerInstanceMaterialTex.material_id` and `PerInstanceMaterial.id` (both hold 16-bit ids) | 0 | all | every id reader must mask: the classify, `pack_material_id_ba`, three shading families. A missed mask silently renders another material. Caps K at 65,535. | rejected |
| D3‴: append the override records to the VB instance ring (no new binding) | 0 extra binding | VB only | couples ring capacity to K; the slot becomes an absolute index that moves with N | rejected |
| D4: a fixed float array per primitive (UE Custom Primitive Data, 32 floats) | 128 B × N | as D2 | OK | rejected: 8× D2's bytes for lanes nobody named |

**The bandwidth case pass 1 did not state.** On a raster path whose scene uses **only** the default
material but carries overrides, D3′ uploads the full 32·N ring plus 32·K. That is **twice** D2's
16·N. In every scene with at least one non-default material, the ring is uploaded anyway, and D3′
adds only 32·K. On VB, D3′ adds only 32·K in every case. D3′ is chosen because the common case wins
and VB, the classified path, never pays.

**The carriers, per path:**
- **VB.** `VbInstanceRow._pad[0]` (offset 56) becomes `override_slot`.
  - The gather scatters a new parallel lane `override_slots` in lock-step with `inst_flags`.
  - `sync_vb_instance_ring` (`crates/boyko_render/src/mesh_draw.rs:519`) packs it exactly as it
    packs `flags`: one more sequential `u32` load per row.
  - Offset 56 sits in the same 16-B lane as `mesh_id`, which every VB shader already loads, so it
    costs no extra device fetch.
  - With no override anywhere, the lane is all zero, which is what `_pad[0]` carries today. The
    uploaded ring bytes are **unchanged, not merely equivalent** (the `inst_flags` precedent).
  - The override ring is **+1 binding in `vb_layout0` and `vb_layout0_froxel`**, written into every
    set built against them: `vb_set0`, `vb_set0_tex`, `vb_set0_froxel`, `vb_set0_tex_froxel` and the
    VG late-raster twin. All VB tails are re-pinned in DM3 anyway, for `override_apply`.
- **Forward.** `PerInstanceMaterial._pad[0]` becomes `override_slot`. `forward_opaque.vs` forwards it
  as a flat interpolant, and the fragment stage reads the override ring (+1 binding).
- **Deferred `pm`/`mvpm`.** Same field. The vertex stage forwards it; the fragment stage applies
  `override_apply` and writes the F8 channel.
- **Deferred `tex`.** `PerInstanceMaterialTex` has no spare bytes, so the `tex` raster set gains the
  `PerInstanceMaterial` ring as **one more binding**. The vertex stage reads `override_slot` beside its
  `PerInstanceMaterialTex` row. That is one more 32-B load per vertex, of an element every vertex of
  the instance shares [E: L1-resident].

**The upload rule for `PerInstanceMaterial` (fixes live defect D-2 and the override-removal case).**
- Let `pm_live = any_non_default_material || any_override`.
- Per ring slot `s`, the runner keeps `pm_hw[s]`: the high-water row count written into slot `s` since
  it was last all-zero.
- **On a `pm_live` frame:** upload the gathered lane `[0, N)` (as today), and set
  `pm_hw[s] = max(pm_hw[s], N)`.
- **On a non-live frame with `pm_hw[s] > 0`:** zero-fill `[0, pm_hw[s])` of slot `s` once, then set
  `pm_hw[s] = 0`. After `FRAMES_IN_FLIGHT` frames both slots are clean.
- **Why zero is exact.** On a non-live frame every instance has `id = 0` and `override_slot = 0`. The
  readers still bound on such a frame read only those two words: the flat VB tails, the classify,
  `forward_opaque.vs`, and the Deferred `tex` raster's new slot read. The Deferred `pm` raster, the
  only `base_color` reader, is deselected. A grown ring is zero-filled at creation
  (`crates/boyko_app/src/gpu_scene/mod.rs:5885`), so the rule holds across growth.
- **Cost:** one memset of 32 × `pm_hw` bytes per slot, once per falling edge. At 100k instances that
  is 3.2 MB ≈ 0.3–0.6 ms CPU at 5–10 GB/s write-combined [E], paid only on the rare frame the last
  override or last non-default material disappears.

**The Deferred pipeline selector.** `pm_enabled`, `raster_pipeline_pm`, `pm_bind_group`
(`gpu_scene/mod.rs:7066-7068`) and `raster_pipeline_mvpm`, `mvpm_bind_group` (`:7030-7036`) key on
`pm_live` instead of `any_non_default_material`. Otherwise an overridden entity that uses the
default material (id 0) gets the base pipeline, which has no per-instance ring, and its flash is
dropped on the default path. Precondition, checked first in DM3: the `pm` pipeline must render an
all-default, override-free scene byte-identically to the base pipeline. If it does not, the two
pipelines already disagree on material 0, and that is fixed before the selector widens.

**The four gather sites.** `gather_mesh_draws` has two `cfg(hwrt)` twins. Each has an ok query and a
complementary `Enabled<MaterialStale>` query (`crates/boyko_render/src/mesh_draw.rs:1315`, `:1327`,
`:1530`, `:1542`). All four gain the non-filtering `Option<&MaterialOverride>` term, for the
`OcclusionCulling` reason: a filtering term would renumber the ring.
- **Decided:** an override applies to an instance whose material is still streaming. That instance
  draws with the pinned default material, and the override on top of it.
- A hit flash is gameplay feedback and must not wait for a texture.

**The layout, following the Unity precedent of "absent means zero" [RESEARCH §2.2]:**
- `override_slot = 0` means "no override". Its bytes are 0 today on both carriers.
- The override table's row 0 is the identity, written explicitly (tint 1, `k` 0, flags 0).
- Rows `1..=K` are filled per frame by the gather.

**The record** (32 B, `#[repr(C, align(16))]`, two std430 lanes):

```
InstanceOverrideGpu {
    a: [tint.r, tint.g, tint.b, k],                          // k ∈ [0,1]: the per-instance emissive scalar
    b: [bitcast(pack_unorm4x8(flash.rgb, flags)), param, frame, _reserved],
}
flags: bit 0 SCALE  (k scales the material's own emissive, instead of adding a flash)
       bit 1 FRAME  (frame overrides the material's flipbook frame)
FLASH (bit 0 clear): base' = lerp(base·tint, flash_rgb, k);  emissive' = emissive + base'·k·FLASH_GAIN
SCALE (bit 0 set):   base' = base·tint;                       emissive' = emissive·(1 + k·(SCALE_MAX − 1))
param: DM5 dissolve progress (0 = none).  frame: a fractional flipbook frame (F17).
FLASH_GAIN, SCALE_MAX: named leaf constants, host-mirrored.
```

- The flash colour is LDR (≤ 1), so 8 bits per channel lose nothing that F8's 8-bit Deferred channel
  keeps. HDR intensity comes from `FLASH_GAIN`.
- **`k` is one scalar per instance.** An instance either flashes (FLASH) or scales its own emissive
  (SCALE, e.g. a flickering torch), not both at once. The limit comes from Deferred's single free
  G-buffer byte (F8). It is stated, not hidden.
- With `k = 0` both formulas are the identity, so row 0 is exact.

### F8: The per-instance emissive on the Deferred path

The Deferred resolve knows a pixel's **material** id, not its **instance**. Per-instance emissive
therefore has to travel through the G-buffer.

`gAlbedo.a` is written `1.0` by every raster variant and never read by the resolve (RESEARCH §1.9).

**Decision:**
- **The raster** writes `gAlbedo.rgb = base'`. It encodes `v = round(k·127) | (SCALE << 7)` and
  writes `gAlbedo.a = (255 − v) / 255`.
- **The resolve** decodes `v = 255 − round(a·255)`, `k = (v & 127) / 127`, mode `= v >> 7`. It then
  applies F7's FLASH or SCALE formula to the `emissive` it already has. That is one select between
  two fused multiply-adds, on a texel it already loaded (`deferred_pbr.hlsl:820`).
- **The OFF path is exact.** A pixel with no override has `v = 0` and `a = 1.0`. `R8G8B8A8_UNORM`
  stores 255, which decodes to exactly 1.0, so `k = 0`. Both formulas then return `emissive`
  unchanged, bit for bit. **Every Deferred golden stays byte-identical.**
- **Cost:** 0 extra bytes, 0 new attachments. `k` is quantized to 1/127. A 0.5 s fade at 60 fps
  uses 30 of those 127 steps [E], so it does not band. Pass 1 used 8 bits and one mode; one bit now
  selects the mode.

### F9: The per-pixel operations: gating in the lit producers

**Where the operations apply** (RESEARCH §1.6, counted in pass 2):
- **UV transform and flipbook:** where material textures are **sampled**, 5 sites:
  `gbuffer_mrt_tex.fs`, `vb_shade_tex{,_froxel}` and `vb_shade_split_tex{,_hwrt}`. Forward has no
  textured variant, so it has nothing to transform.
- **Rim:** wherever a lit pixel's `emissive` is formed, 20 sites: 10 VB tails, the 4 lit
  `deferred_pbr` resolves, `forward_opaque{,_froxel}.fs` and 4 `sdf_forward_march` variants.
- **Together: 21 distinct `.spv`.** 18 are compute; 3 are raster (`gbuffer_mrt_tex.fs`,
  `forward_opaque{,_froxel}.fs`).

**The in-tree rule this must meet: F24.** A disabled block compiled into the VB tails cost +75 % of
`vb_resolve` while off (§1.1). Pass 1 chose a runtime branch and cited RENDER-PARITY §3.6 as
precedent. That precedent gates **one texture load**. The right comparison is F24:

| | SV0 inline march (F24) | DM4 block |
|---|---|---|
| Size proxy | the dedicated `sdf_mesh_shadow.comp.spv` is **26,312 B**, against `vb_resolve.comp.spv` at 48,596 B. That is an upper bound for the march: the file also holds its own skeleton and fetch. | the whole 5-slot sampling block is `gbuffer_mrt_tex.fs.spv` − `gbuffer_mrt.fs.spv` = 6,096 − 2,252 = **3,844 B**. That is an upper bound for one extra sample per slot, since it also holds the normal-map path. UV + rim ≈ 20 ALU. So **≈ 3–5 KB** [E]. |
| Shape | a loop with loop-carried state and a field fetch per step | straight-line. The UV transform runs **once** per pixel: all five slots sample one `geo.uv` with one gradient pair (`vb_shade.comp.hlsl`). The flipbook adds one `SampleGrad` per sampled slot. |
| Where its registers peak | inside the march, on top of the shading stack | the flipbook's second samples land **at the textured tails' register peak**: up to 5 × 4 = 20 more live values while the samples are in flight [E] |
| Measured dark cost | **+75 %** of the dispatch | unmeasured |

So the DM4 block is ~5–8× smaller than the march, but its heaviest part sits exactly where occupancy
is decided. On CC 8.6, +2 registers have already crossed an allocation step once in this tree
(`docs/PARTICLES-PLAN.md:1377`). A dark tax of the F24 class cannot be ruled out for the textured
tails. The feature must cost exactly zero when off, **by construction, not by a branch**.

| Option | New `.spv` | Pipelines built | Cost when off | Composes with `-D WEATHER` (effects thread) | Owner rules | Verdict |
|---|---|---|---|---|---|---|
| E1: a `-D` per operation (3) | ×8 on every affected file | ×8 | 0 | ×16 | FAILS SH | rejected |
| E2: one `-D MAT_DYN` axis | +21 | ×1 of 2 | **0**: the base variant is the old source | +21 now, and **+~20 more** for the MAT_DYN × WEATHER product: ≈ +41 on top of WEATHER's +22 | SH: +21 to +41 manifest rows and byte pins | **fallback per site** (below) |
| **E3′: a specialization constant `MAT_DYN` on the existing `.spv`.** Compute sites use `ComputePipelineDesc::spec_constants` (`crates/boyko_rhi/src/descriptor.rs:267`); the 3 raster sites need the RHI field below. | **0** (a one-time re-pin of the 21 files) | `false` only, when the capability is off. `false` + `true` when it is on, and the host picks one per frame. | **0 if the driver folds the `false` block.** Measured per site by gate (c1); if it does not fold, that site takes E2. | **+0 .spv.** Pipelines multiply, .spv files add: WEATHER's +22 stays +22. | OK P0: capability at boot, per-frame selection by a host count; OK SH | **chosen** |
| E4 (pass 1): a runtime-uniform gate word | 0 | ×1 | the F24 class: the worst-case registers of a never-taken block | 0 | **FAILS** owner rule 3 (unused costs zero); it is the shape the effects thread rejected as its W2 | withdrawn |
| E5: branch only in the classified `vb_shade` | 0 | ×1 | 0 | — | FAILS parity: Deferred is the default path | rejected |
| E6: a separate pass for the rim, which is purely additive | 0 | +1 pass | 0 | — | Deferred: a full-screen pass reading normal, id and `lit` and writing `lit`, ≈ 2.07 M × 24 B ≈ 50 MB ≈ **0.14 ms** at 360 GB/s [E], against ≈ 6.5 µs of ALU for the rim inline (2.07 M × 20 / 6.4 T [E]). VB and Forward have **no stored normal**: the pass would re-run `vb_geom_fetch` per pixel, 3 index + 3 vertex fetches. | rejected |

**Per-frame selection, and what replaces pass 1's `TABLE_ACTIVE` word.**
- When the capability is on, both specializations exist.
- The host binds `true` iff `dyn_rows > 0`: the number of material rows whose **staged**
  `MaterialDynGpu.flags` are non-zero.
- `stage_material_edits`, the only writer of GPU rows, maintains that count. For each row it stages,
  it compares the new flags with a per-row "staged non-zero" bit (8 KiB at the cap).
- Because `free` sets the edited bit (F2), a freed row is staged as zeros and the count decrements.
- The count can therefore never under-count, and a frame without any dynamic material runs the
  `false` pipeline. This answers the critique's open question 1. There is no shader-side gate word
  left to keep correct.
- This is the tree's existing idiom: Deferred already picks base, `pm` or `tex` per frame from host
  flags (`crates/boyko_rhi_vulkan/src/present/passes/gbuffer.rs:954`, `:958`).

**Inside the `true` specialization**, each pixel tests its material's own `MaterialDynGpu.flags`
(UV_ST, FLIP_BLEND, RIM). On the classified VB path that test is wave-uniform. When the capability is
off, the `MaterialDynGpu` binding **aliases the material table**. No buffer is created, and the read
sits behind the constant, so it never executes.

**The RHI change the raster sites need.** `GraphicsPipelineDesc` (`crates/boyko_rhi/src/descriptor.rs:352`)
gains `fragment_spec_constants: &[SpecConstant]`, mirroring the compute field:
- empty ⇒ a literal null `pSpecializationInfo` ⇒ byte-identical create-info for every existing
  pipeline;
- non-empty ⇒ the backend fills the fragment stage's `VkSpecializationInfo` on its create-call stack.

It closes the one spec-constant gap the survey named (RESEARCH §1.6).

**Boot cost** of the `true` specializations: 21 pipelines. At UE's 5–10 ms per PSO compile [P 5],
that is ≈ 0.1–0.2 s of boot [E], paid only when the capability is on.

**The measured gate** (DM4 (c)), with a route for every failure mode:
- **(c1) Fold, per site.** The register probe reads the driver's Register Count and ISA bytes for the
  `false` specialization and for the pre-DM4 committed `.spv` (the rung's parent blob). **Pass:**
  both numbers equal.
  - The probe is the particle plan's headless harness (`boyko_rhi_vulkan/tests/particle_sim_occupancy.rs`),
    which today builds **compute** pipelines only. Extending it to graphics pipelines is a DM4
    prerequisite. `vkGetPipelineExecutablePropertiesKHR` reports per-stage executables for graphics
    pipelines too.
  - Red → that site takes **E2**: one `-D MAT_DYN` row, whose base variant is the pre-DM4 source, so it
    is exactly zero by construction.
- **(c2) Off-path wall clock, per site zone.** ABBA quadruplets (SV0's protocol): pre-DM4 `.spv` vs
  the `false` specialization, in the owner's quiet window.
  - Pre-registered budget: |Δ| ≤ max(1 %, the zone's own A/A noise).
  - Red with (c1) green (folded, yet slower, e.g. code layout) → **E2** for that site.
- **(c3) Armed cost.** Full-screen UV + rim ≤ 0.07 ms at 1080p [E]. The flipbook's `true` Register
  Count and occupancy bucket are reported per textured site.
  - If the flipbook costs a bucket that UV + rim alone would not, split a second constant `MAT_FLIP`,
    so materials without flipbooks stop paying for it, and re-measure.
  - If it still costs a bucket, restrict the flipbook to the albedo and emissive slots (a per-slot
    mask), which halves its extra samples. This is armed cost, paid only while a dynamic material is
    on screen.

**Where the gate for the per-material parameters lives: in the table.** The lit compute producers
bind the 80-B `CompositePushConstants`, but the raster fragment shaders do not. The per-material
flags in `MaterialDynGpu` are therefore the one carrier every site has.

### F9a: One gating policy for the lit producers (shared with the effects thread)

Two same-day designs add per-pixel work to the same ~20 lit producers: this one (DM4) and the
effects design's `-D WEATHER` (FX10). Pass 1 contradicted the effects design. The policy both now
satisfy:

1. **A lit-producer feature costs exactly zero when off, by construction:** a `-D` variant or a
   specialization constant. **Never** a never-taken runtime branch in a shipped pipeline (F24).
2. **Per site, the mechanism is the cheaper of the two that fold.**
   - A specialization adds pipelines, not `.spv`, and composes additively with other axes. It is
     preferred at compute sites and, after the RHI change above, at raster sites.
   - `-D` remains correct and is the fallback wherever the fold gate fails.
3. **Every such feature carries an off-path gate:** register/ISA equality or ABBA, with a
   pre-registered budget.

The effects design's `-D WEATHER` complies with 1 and 3 as written. **Proposal to that thread,
recorded here and not decided here:** the WEATHER set is 20 compute sites and 2 raster ones
(`forward_opaque{,_froxel}.fs`). With the fragment-stage field this design adds, all 22 could take
a specialization pair and add **0** rows instead of +22, subject to WEATHER's own fold gate.
With both features landed:
- as specified today: the 22 lit-producer `.spv` (10 VB, 6 `deferred_pbr`, 2 Forward, 4 SDF march)
  become up to 44 with WEATHER, and DM4 adds none. There are ≤ 4 pipelines per site
  (WEATHER × MAT_DYN), each built only if both capabilities are on.
- DM4 re-pins 21 files, plus the WEATHER variants of the same sources if FX10 lands first (≈ 20).

§6 carries the arithmetic.

### F10: Where the per-pixel parameters live

| Option | Cost | Verdict |
|---|---|---|
| Grow `MaterialGpu` to 64 B | Changes `MATERIAL_GPU_WORDS` in all eight binding readers; re-blesses every pin; +33 % of every per-pixel material fetch for lanes most rows never use | rejected, by the same number that decided the transparency split |
| Reuse the transparency design's `MaterialXGpu` | Its three lanes are full, and the **opaque** producers never bind it | rejected |
| **A cold `MaterialDynGpu` table, 48 B per row, dense by `MaterialId`** | ≤ 3.0 MiB at the cap; bound only by the 21 sites | **chosen** |

```
MaterialDynGpu {                                  // 48 B, #[repr(C, align(16))], const-asserted
    uv_st: [scale.x, scale.y, offset.x, offset.y],             // applied to the material-texture UV
    flip:  [frame, bitcast(cols | rows << 16), bitcast(frames), bitcast(flags)],
    rim:   [rim.r, rim.g, rim.b, _reserved],                   // emissive += rim · (1 − N·V)^5
}
// flags: bit 0 UV_ST, bit 1 FLIP_BLEND, bit 2 RIM
// flipbook: c = floor(f) mod frames, n = (c + 1) mod frames, w = frac(f),
//           f = the instance's frame if its override FRAME bit is set, else flip.x
```

Pass 1 stored the two cell offsets computed on the CPU. Pass 2 stores the **grid and a fractional
frame** instead, so a per-instance frame (F17) can pick its own cells in-shader. The cost is about
10 integer ALU for the cell arithmetic [E].

**Supporting decisions:**
- **Where the authored values live: a plugin-owned cold column, not a sidecar on `Material`.**
  - Pass 1 added a 48-B `dynamic` field to every `Material` row, which costs every material even with
    the plugin absent. And "stays on the first cache line" was true only for a 64-B-multiple row stride.
  - Pass 2: `MaterialDynAuthored`, a Resource-owned column keyed by material row. It exists only when
    the per-pixel capability is on and grows with the table (the rare grow frame).
  - `Material` itself is unchanged.
- **How it uploads.** The same edited-row upload (F1) stages both tables for a dirty row: 96 B instead
  of 48.
- **Rim exponent.** Schlick's fixed ^5 is 3 multiplies of exact eDSL arithmetic. A variable exponent
  needs `Exp2`/`Log2`; F6's host-stable `Exp2` makes it possible, but nobody has asked for it.
- **Derivatives.** Under `uv' = (uv·s + o + cell) / grid` the chain rule gives
  `ddx' = s·ddx / grid`. So `SampleGrad` scales the existing analytic VB gradients (`vb_uv_grad`) by
  `s / grid`, once per pixel. The raster's implicit derivatives are already correct, because `s`,
  `o`, the cell and the grid are constant across a quad.

### F11: Dissolve

- **Dissolve is cutout.** It stays on the transparency design's **`MASKED` opaque rail**: the
  `-D MASKED` raster variants, `VB_INST_FLAG_MASKED`, and the masked caster pipelines. It is not
  built separately.
- **The delta over transparency rung R2.** The discard threshold is `max(material cutoff,
  hash_noise(p) < override.param)`, with `param` from F7's record. The noise is a new FMA-free eDSL
  hash leaf, a sibling of transparency's hashed-alpha leaf.
- **Shadows follow the dissolve through the masked casters.**
  - Transparency's D-U6 gives masked casters their own `With<AlphaMasked>` query pair (ok + stale)
    feeding `MaskedCasterScratch`.
  - DM5 adds the non-filtering `Option<&MaterialOverride>` term to **both** of those queries, and an
    `override_slot` lane to `MaskedCasterScratch`'s row.
  - The masked caster pipelines bind the override ring (+1 binding) and apply the same threshold.
  - The opaque caster path is untouched and stays byte-identical, which is D-U6's own reason for a
    separate scratch.
- **Capability is structural.** Gameplay inserts `AlphaMasked` when a dissolve starts and removes it
  when it ends. That is two archetype moves per dissolve event, not per frame.
- **HZB stays correct.** The masked raster writes depth only for the pixels it keeps, so the pyramid
  built from that depth stays conservative.
- **SDF edits do not dissolve.** The SDF marcher has no masked rail. Stated, not hidden (§2).

### F12: Tier 3 on the visibility buffer: shading N programs

**Definitions.**
- A *program* is a shading function.
- Materials map many-to-one onto programs.
- `MaterialProgram(u16)` is a per-material field.

| Option | GPU at 1080p (RTX 3060) | Registers | RHI | Reaches SDF pixels? | Verdict |
|---|---|---|---|---|---|
| **G0: one classified `vb_shade` that switches on `program(mat)` (uber-shader)** | Classify only (below). **Empty-bin tax 0:** no per-program dispatch. **Switch cost:** one wave-uniform branch per group, since the classified shade has one material per 64-thread group = two whole warps (RESEARCH §2.10). | the **maximum** over programs sets every program's occupancy | **none** | **yes**: the same switch compiles into `sdf_gbuffer_composite` and `sdf_forward_march`, which already key on the material id (divergent there, see F13) | **default** |
| G1: classify by material, have the scan emit per-program group ranges plus `VkDispatchIndirectCommand`, then one `vkCmdDispatchIndirect` per program present | Classify, plus the empty-bin tax ≤ 0.33 µs × P [E: Epic's ~1 ms / 3,075]. P = 64 → ≤ 21 µs. | each program its own | `INDIRECT_BUFFER` usage on `gClassify`. The dispatch goes through the raw `fns.cmd_dispatch_indirect` (particles idiom) or through reflections' RK-16 trait method, whichever exists first. **`VkDispatchIndirectCommand` is only `{x, y, z}`** with no base workgroup, so each program's shader reads its group-range start from a buffer indexed by a push-constant program index. | no | **per heavy program** (below) |
| G2: over-dispatch per program with a sentinel (today's D2, ×P) | P full grids, each doing 1/P useful work: P = 16 → 16 × 32,400 groups [E] | each its own | none | no | rejected |
| G3: material depth plus one full-screen quad per program (Nanite 5.0) | Hardware depth-EQUAL rejection; N draws; puts a raster pass back into a compute-shaded path | — | none | no | rejected: Epic moved away from it |
| G4: DGC indirect execution sets | Removes the empty binds by design | each its own | `VK_EXT_device_generated_commands`: NVIDIA and RADV verified, AMD Windows not | no | a later **probed** extra over G1, never the only path |

**Classify cost, both G0 and G1.** Both need the classified path, because wave-uniformity is what
makes either cheap. So a scene that uses any non-default program runs classify on flat frames too:
**0.42–0.54 ms at 1080p**, 0.75–0.97 ms at 1440p, 1.7–2.2 ms at 4K [E: Hable 0.34 ms × 1.24–1.6;
RESEARCH §2.9]. It is already paid on textured frames.

**Decision: G0 by default, G1 for the programs that would cost the uber-shader an occupancy step.**
- The register probe measures the uber-shader's Register Count against the base `vb_shade`'s.
- A program whose inclusion moves the uber-shader across a CC 8.6 allocation step (48 warps/SM, 64 K
  registers/SM, 256-register-per-warp unit) leaves the switch and gets its own G1 dispatch.
- The rest stay in the switch.
- **Why G0 first:**
  - it costs no RHI work and no empty bins;
  - it is the Doom Eternal end of the design space ("very few uber shaders", RESEARCH §2.8);
  - it is the only candidate that reaches the SDF shading sites (owner rule 2).
- **Why not G0 alone:** one heavy program would lower every program's occupancy. That is AMD's
  warning (RESEARCH §2.10), measured rather than assumed.
- The uber-shader's code grows linearly with P. Its instruction-cache effect is unmeasured and is part
  of DM6's timing gate.

**The classify scan under each option** (the critique's open question 3):
- The scan's loop runs to `capacity_rows`, up to 65,536 (`material_table.rs`, `capacity_rows` doc).
- **G0 adds nothing to the scan.**
- G1 needs the present materials in program order. The host uploads a program-sorted permutation of
  material ids (2 B × rows, only when a program assignment changes), and the scan iterates it. That is
  **one extra 2-B load per iteration** of the existing loop: 128 KB at the cap.
- The scan's absolute cost at the cap is unmeasured. DM6 measures it before choosing, and may bound
  the loop by the frame's present-material count (the plan's original D2).

**PSO and `.spv` counts.**
- **G0:** one uber `vb_shade` per axis variant. Under the source policy (§9 Q1), adding a program
  **re-pins** the uber `.spv` instead of adding rows.
- **G1:** P_heavy × the `vb_shade` axis set (8), plus the raster fragment variants (F13).
- **Asset policy:** the same counts, compiled at content time outside the cargo byte gates. The
  uber-shader is then regenerated at content time from every program.

That dependence is why the policy question is not a formality.

### F13: Tier 3 on the raster paths, and on SDF pixels, per path

- **Deferred and Forward mesh pixels:**
  - per-program fragment pipelines; the gather's bucket key grows from `mesh_id` to
    `(program, mesh_id)`;
  - the Deferred resolve stays program-agnostic: a program writes G-buffer channels, as Nanite's
    compute material pass does;
  - Forward shades inline, so a program runs inside `forward_opaque`'s skeleton.
- **SDF pixels are not reached on any path** by per-program raster pipelines or by VB classification:

| Path | Mesh pixels | SDF pixels, today's design |
|---|---|---|
| VB | G0 switch / G1 dispatch (F12) | the base program, via the compute SDF shaders |
| Deferred | per-program raster (above) | the base program: `sdf_gbuffer_composite` is one compute shader |
| Forward, F+ | per-program fragment inside `forward_opaque` | the base program: `sdf_forward_march` is one compute shader |

**The SDF candidate: G0 inside `sdf_gbuffer_composite` and `sdf_forward_march`,** switching on
`program(mat)` at the shading step.
- SDF pixels are unclassified, so a wave containing several programs runs each one present.
- `MAX_SDF_EDITS = 16` (`crates/boyko_sdf_math/src/lib.rs:105`) bounds that to 16 programs per wave.
- It is named here and built only with DM6, after the policy decision.

### F14: The authoring surface: can the eDSL express a material graph?

**Yes for the arithmetic core, and structurally it is already the right shape.**
- A graph compiles to calls of generic eDSL functions, one per node kind.
- The thread-local SSA recorder is exactly a topologically ordered graph-to-HLSL printer: the
  MaterialX and Frostbite architecture (RESEARCH §2.5, §2.11).
- The `f32` instance then evaluates **the same program on the CPU as its oracle**, which makes
  per-program golden values possible.
- **Precedent, qualified in pass 2:** Destiny's TFX already evaluates one material expression on the
  CPU or the GPU, from one bytecode (RESEARCH §2.14). What no surveyed engine does is **compile** one
  source into both a native host function and generated shader code, so that the host result is an
  oracle for the GPU's. Pass 1 said "No surveyed engine has that" without the qualification.

**Missing, each small and each needed only at DM6:**
1. **A `Dual<S>` scalar** `(v, ∂x, ∂y)` implementing the eDSL scalar trait, so derivatives propagate
   by the chain rule through procedural nodes. This is Hable's scheme; the VB UV gradients it starts
   from already exist (`vb_uv_grad`).
2. **An emit-only bindless `SampleGrad` resource node**, the sibling of today's `Texture3D`.
3. **A graph walker** from authored data (Gaia) to eDSL calls.

What the eDSL **must not** gain is the skeleton: bindings, stores, `discard`. The tree's rule stands
(`emit_particles.rs:34`): skeletons stay generator-owned or hand-written.

### F15: Vertex animation (wind, VAT)

| Option | Cost | Verdict |
|---|---|---|
| **H1: evaluate in-shader in every vertex-producing stage** (`gbuffer_mrt.vs`, `forward_opaque.vs`, `vb_raster.vs`, `csm_depth.vs`, `punctual_depth.vs`) **and** in the VB re-fetch (3 vertices per pixel) | ALU per vertex (~20–40 [E]) plus 3× per VB pixel. Aaltonen reports one branch on the animation type at **+2%**. | **chosen for DM7** |
| H2: a compute pre-pass that deforms into per-instance vertex copies (the skinning shape) | Memory ∝ instances × vertices: 10k trees × 5k verts × 12 B ≈ **600 MB** [E] | rejected for instanced foliage; admissible for a few hero meshes |
| H3: VAT textures | The same stages as H1, with a texture fetch | a data source for H1, not an alternative |

**What DM7 needs** before anything moves:
- **Cull bounds** inflated by a clamped maximum displacement in the meshlet, batch and HZB culls
  (UE clamps WPO for the same reason).
- **Shadow vertex shaders.**
- **A GPU time carrier per stage,** with F5's policy.
- **A motion-vector decision.** `MvSource::PerObject` was declined with reasons that still hold. Wind
  is small-amplitude and the variance clamp absorbs it, but that must be measured, not assumed.
- **HWRT BLAS.** Accept static shadows from swaying foliage, or refit.
- **Its own lit-producer gate** under F9a's policy, if any per-pixel part appears.

It is the most invasive class, hence its own campaign (§9 Q2).

### F16: The SDF leg

- **Tier 1 and drivers (DM1, DM2):** work unchanged. SDF pixels read `Materials[id]` through
  `sdf_gbuffer_composite` and `sdf_forward_march`, and the table holds effective values.
- **Per-instance values: the same `MaterialOverride` component, through a private row per edit.**
  - Pass 1 had gameplay give an SDF edit a unique material, and relied on re-encoding the edit list
    when an edit changes rows. That path does not exist: the edit list is boot-static, and "the
    gather runs once" (`crates/boyko_render/src/sdf_edit.rs:24`).
  - Pass 2 never re-encodes. When the plugin is present, `collect_sdf_edits` gives **each edit its
    own private material row** at boot (`Assets::add` of the edit's authored material) and writes that
    row's id into `center.w` once.
  - The edit entity carries `SdfMaterialSource { source, private }`.
  - Each frame, `drive_materials` composes `effective[private] = effective_or_authored[source] ⊙
    override(edit)`, using the same FLASH/SCALE/tint formulas as F7, after all driven rows. Skip-if-
    equal keeps a static edit at zero uploads.
  - So gameplay writes `MaterialOverride` on an SDF edit exactly as on a mesh entity: **one API for
    both legs** (owner rule 2).
  - **Row cost:** at most `MAX_SDF_EDITS` = 16 extra rows, 768 B (§9.3 withdraws pass 1's Q3 on this
    arithmetic).
  - Plugin absent: no private rows, byte-identical to today. Plugin present: the SDF pixels' G-buffer
    id channel names the private rows, and lit images are unchanged, since the values are identical.
- **Not on SDF:** `frame` (no UVs) and the dissolve `param` (no masked rail). A debug build warns once.
- **Per-pixel operations:** the rim applies at the SDF sites (the `deferred_pbr` resolves and
  `sdf_forward_march`). UV operations do not apply, since SDF has no UVs. SDF texturing is triplanar,
  which is Tier 3 or a baked tool.
- **Global illumination:** SDF-DDGI bounces a constant albedo and binds no material table (RESEARCH
  §1.1). No material value, static or animated, reaches the indirect light today. That is a DDGI
  follow-up, not this campaign.
- **Tier 3:** the base program on SDF pixels on every path; the named candidate is F13's switch.

### F17: Per-instance time (staggered explosions, out-of-phase flicker)

Material drivers target a *material*, so every instance of it animates in lockstep. Two mesh
explosions spawned 0.5 s apart would show the same flipbook frame, and every torch would flicker in
phase.

| Option | CPU | GPU | Paths | Verdict |
|---|---|---|---|---|
| T1: a unique material per instance | per material | 0 | all | the MID anti-pattern; 65,536-row cap | rejected |
| T2: gameplay rewrites `MaterialOverride` by hand each frame | per instance, ad hoc | 0 | all | no shared curve vocabulary; every gameplay system reinvents one | allowed, not the mechanism |
| **T3: per-instance drivers on the CPU.** `InstanceMaterialDriver` (on the mesh entity, 64 B) holds up to two lanes (`K`, `Frame`, `Tint`) with a curve, its own `t0` (virtual-clock nanoseconds at spawn) and a phase or seed. `drive_instance_overrides` writes the entity's `MaterialOverride`. | ≈ 12–25 ns per lane [E, as F4]: 1,000 animated lanes ≈ 12–25 µs; 10,000 ≈ 0.12–0.25 ms | 0 extra: the override path (F7) | all four: `k` reaches Deferred through F8; `frame` applies wherever textures are sampled | **chosen** |
| T4: GPU evaluation. `t0`/seed + curve id in the record, time in-shader (UE `PerInstanceRandom` × `Time`, RESEARCH §2.1) | 0 per instance | ≈ 15 ALU per covered pixel: 2.07 M × 15 / 6.4 T ≈ **5 µs** full-screen at 1080p [E] | a GPU time carrier in every stage's block (RESEARCH §1.3) and F5's P = 1,024 s policy | escalation |

**Escalation trigger:** `drive_instance_overrides` above **0.1 ms CPU** (≈ 4,000–8,000 animated
lanes [E]) moves that subset to T4. Mesh explosions and torches number tens to hundreds; mass effects
are particles, which keep their own clock.

The same F6 curve leaves serve both material and instance drivers. The clock rules are F5's.

---

## 4. The architecture

### 4.1 ECS data and systems

| Item | Kind | Owner crate | Notes |
|---|---|---|---|
| `Assets<Material>` + `edited: LiveBitmap` + count | kernel asset column | `boyko_ecs` | F2. Set by `get_mut`, `add`, `fill`, `free`; drained by the stager. |
| `Material { gpu, textures }` | asset row | `boyko_render` | **Unchanged** in pass 2. Holds the **authored** value. |
| `MaterialUploadStaging` | `Resource` (Send) | `boyko_render` | Compact edited-row bytes for both tables, the run list and `dyn_rows` with its per-row bit. Preallocated to table capacity, regrown only on the grow frame. |
| `stage_material_edits` | system (core render plugin, DM1) | `boyko_render` | `ResMut<Assets<Material>>` (drain `edited`), `Option<ResMut<MaterialModulation>>` (drain `changed`), `Option<Res<MaterialDynAuthored>>`, `ResMut<MaterialUploadStaging>`. All Send, so this is an ordinary system borrow. |
| `MaterialDriver { target: Handle<Material>, lane: DrivenLane, op: Replace\|Mul\|Add, curve: CurveKind, flags: u8, params: [f32; 11] }` | **dense component** on driver entities (64 B, one cache line) | `boyko_render` (dynamic-materials module) | At most one `Replace` per `(material, lane)`; a debug build checks it with 7 lanes × 8 KiB. Capability is structural: no driver entities, no work. |
| `DrivenLane` | enum | | `BaseColor`, `Emissive`, `Metallic`, `Roughness`, `UvSt`, `FlipFrame`, `Rim` (7) |
| `MaterialModulation` | `Resource` (Send) | | `slot_of`: a `u16` column keyed by material row (0 = not driven). Dense effective records (`MaterialGpu` + `MaterialDynGpu`). A `changed` bitmap. Slots are allocated and freed by `MaterialDriver`'s add and remove hooks, refcounted per material. |
| `MaterialDynAuthored` | `Resource` (Send), per-pixel capability only | | Authored `MaterialDynGpu` per material row (F10). Absent ⇒ no per-pixel operations. |
| `MaterialOverride { tint: [f32; 3], k: f32, flash: [f32; 3], flags: u8, param: f32, frame: f32 }` | component on mesh or SDF edit entities | `boyko_render` | Gameplay or an instance driver writes it. Presence is the capability; absence reads as the identity. |
| `InstanceMaterialDriver` | dense component on mesh entities (64 B) | | F17. Two lanes. |
| `SdfMaterialSource { source, private }` | component on SDF edit entities | | F16. Written once at boot when the plugin is present. |
| `drive_materials` | system | | `Res<Time>`, `Res<Assets<Material>>` (**read-only**), dense `Query<&MaterialDriver>`, the SDF edit query, `ResMut<MaterialModulation>`. Serial: one writer of the modulation resource. |
| `drive_instance_overrides` | system | | `Res<Time>`, dense `Query<(&InstanceMaterialDriver, &mut MaterialOverride)>`. Parallel-safe per entity. |
| `DynamicMaterialsPlugin { per_pixel: bool }` | optional plugin | | Registers the drivers, the modulation resource, the override types and, with `per_pixel`, the dyn table and the `true` specializations. **Absent means no system, no resource and no cost.** |
| `gather_mesh_draws` (existing) | system | | Gains `Option<&MaterialOverride>` on all **four** queries (F7). Reads effective values through `Option<Res<MaterialModulation>>`, testing `slot_of` per instance only while a material is driven. Fills `override_rows: ScratchColumn<InstanceOverrideGpu>` and the `override_slots` lane. |

**Why drivers are entities, not a field of the asset:**
- A field would make `drive_materials` scan every material row (O(high_water)) or keep a list, which
  is a second authority.
- Entities give structural zero cost, several lanes per material, gameplay lifetimes (a door's glow
  despawns with the door), and Aether/Gaia authoring as ordinary entities.

**Who holds which borrow, per frame, with no `&mut` in the runner.**
- Pass 1 had the runner call `MaterialTable::stage_edited`, which needs `&mut Assets<Material>` and
  the NonSend `MaterialTable` at once. The runner documents that the World cannot hand those out
  together. Its take-out workaround heap-allocates, "correct to pay on the rare grow frame, wrong to
  pay every frame" (`crates/boyko_app/src/runner.rs:1359-1361`).
- In pass 2, **the drain is an ECS system** (`stage_material_edits`, above) over Send resources only.
- **The runner holds two shared borrows:** `world.resource::<MaterialUploadStaging>()` and
  `world.non_send_resource::<MaterialTable>()`. Both are `&self` accessors returning `&T`
  (`crates/boyko_ecs/src/ecs/core/ecs_master/resource_api.rs:52`, `:172`), so they coexist. The
  documented restriction is on a `&mut` NonSend borrow held beside a Send one.
- The runner memcpys the compact bytes into `MaterialTable`'s staging slot `s` through its persistent
  mapping, taking only `&BoundBuffer`. That is how `upload_light_table` takes
  `&host.gpu.light_staging[s]` (`runner.rs:1880-1891`).
- **The take-out remains only inside the existing rare grow branch** (`runner.rs:1379`).

### 4.2 Frame order

```
Time::advance_with                              (frame driver)
… gameplay: Assets::get_mut (authored edits), MaterialOverride writes …
drive_materials            [DynamicMaterialsPlugin] → MaterialModulation effective records + changed bits
drive_instance_overrides   [DynamicMaterialsPlugin] → MaterialOverride fields
stage_material_edits       (core)                  → MaterialUploadStaging: compact rows, runs, dyn_rows
gather_mesh_draws          (existing, + override term, + effective read) → rings, override_slots, override_rows
runner, fenced slot s:
    [rare] grow: new device-local tables + full image into staging[s]         (F3)
    upload PerInstanceMaterial / Tex rings        (existing; F7 upload rule)
    upload VbInstanceRow ring                     (existing, every VB frame; now carries override_slot)
    upload override_rows → override_ring[s]       (only if any_override)
    memcpy MaterialUploadStaging → staging[s], dyn_staging[s]   (shared borrows only)
render graph (all three declarators, identical position right after light_upload):
    material_upload  (TRANSFER: staging[s] → table, dyn_staging[s] → dyn_table; declared only on an edited or grow frame)
    … raster / lit producers (the MAT_DYN specialization picked by dyn_rows > 0) …
```

**The load-bearing ordering edges:**
- `drive_materials → gather_mesh_draws` and `drive_materials → stage_material_edits`. Without them,
  the per-instance lanes and the table disagree for a frame: a Deferred `base_color` from frame N−1
  against an emissive from frame N.
- `drive_materials` runs after every system that edits `Assets<Material>` in the frame, so the
  effective value is always computed from the current authored one.
- A schedule-census test pins all three edges (DM2 gate).

### 4.3 GPU data

| Buffer | Memory | Size | Per frame in flight? | Bound by |
|---|---|---|---|---|
| material table | **device-local** (F3) | 48 B × capacity (≤ 3.0 MiB) | no; rebinding is the existing `rebind_pending` | the eight binding readers (unchanged) |
| material staging | host-coherent | 48 B × capacity (compact use per frame; a full image on frame 0 and grow frames) | ×2, each grown at its own fenced occupancy | nothing (a copy source) |
| `MaterialDynGpu` table | device-local, per-pixel capability only | 48 B × capacity; when off, the binding aliases the material table | no | the 21 F9 sites |
| dyn staging | host-coherent | as above | ×2 | nothing |
| override ring | host-coherent | 32 B × (1 + K_cap) | ×2 | the VB tails (new `vb_layout0` binding), the Deferred `pm`/`mvpm`/`tex` raster, `forward_opaque`, the masked casters (DM5) |

**Growth:**
- The material table and the `MaterialDynGpu` table grow together, by F3's protocol.
- Each staging slot is replaced at **its own** next fenced occupancy.
- The override ring grows in lockstep with the instance rings, like `pm_instance_material_rings`.

### 4.4 Shader side

**Generated leaves** (eDSL, `// === GENERATED … ===` splices, a new `material_dyn_edsl_sync` test
alongside `gbuffer_mrt_edsl_sync`):
- `override_apply(base, emissive, a, b) → (base', emissive')` (FLASH/SCALE), used in the VB tails,
  Forward and the Deferred raster;
- `override_encode_a(k, flags) → alpha` and `deferred_override_resolve(albedo, emissive) → emissive'`,
  used in the Deferred raster and `deferred_pbr`;
- `uv_st(uv, st)`, `flip_cells(frame, grid, frames) → (cell_a, cell_b, w)`, `uv_grad_st(grad, st,
  grid)`, `flip_blend(a, b, w)`, `rim_schlick(nov, rgb)`, used in the F9 sites;
- `dissolve_hash(p) → f32`, used in the `MASKED` variants and masked casters (DM5);
- the curve and easing leaves of F6 (the CPU drivers use the `f32` instances).

**Hand-written skeleton edits:**
- the bindings;
- the `[[vk::constant_id(K)]] const bool MAT_DYN` declaration and the block it guards;
- the flipbook's second sample;
- the `gAlbedo.a` write;
- the `override_slot` fetch.

### 4.5 Zero cost when unused, piece by piece

| Piece | Off state | Why it costs nothing |
|---|---|---|
| DM1 upload | no edited rows | `edited_any()` is false → no staging, no `material_upload` declared, no copy recorded (the `light_upload` idiom) |
| DM2 drivers | plugin absent, or no driver entities | no system and no resource; or an empty dense query. The gather's effective-value test is hoisted out of its loop when nothing is driven. |
| DM3 overrides | no `MaterialOverride` anywhere | the `override_slots` lane is all zero, which is what both carriers' `_pad[0]` hold today, so **the VB ring and the PM ring are byte-identical**. The PM ring's upload rule (F7) keeps a ring that was once written clean. The Deferred `a = 1.0` term is exactly 0. The override ring is not uploaded. |
| DM4 operations | capability off | only the `false` specializations exist, and gate (c1) proves each one folds, else that site is E2. With the capability on and `dyn_rows == 0`, the host still binds `false`. |
| All shader edits | — | one-time `.spv` re-pins, **image goldens byte-identical by construction** |

---

## 5. RHI gaps, as the tree stands at `6394bc5e`

| Need | Status | Needed by |
|---|---|---|
| A `vkCmdCopyBuffer` with multiple regions | exists (`cmd_copy_buffer`) | DM1 |
| Device-local buffer + staging, with a cross-frame WAR barrier | exists (the light table) | DM1 |
| Specialization constants on compute pipelines | exists (`ComputePipelineDesc::spec_constants`) | DM4 (18 sites) |
| **Specialization constants on graphics pipelines** | **absent** (`GraphicsPipelineDesc`, `crates/boyko_rhi/src/descriptor.rs:352`) | **DM4** (3 raster sites): add `fragment_spec_constants` (F9) |
| `VK_KHR_pipeline_executable_properties` statistics for **graphics** pipelines | the probe exists for **compute** only (`particle_sim_occupancy.rs`) | **DM4** gate (c1): extend the probe |
| `vkCmdDispatchIndirect` | **loaded and used** (`DeviceFns::cmd_dispatch_indirect`, particles) | DM6, only for G1 programs |
| A base workgroup for indirect dispatch | **does not exist in Vulkan** (`VkDispatchIndirectCommand` is `{x, y, z}`) | DM6 G1: each program reads its group-range start in-shader |
| `RhiCommandEncoder::dispatch_indirect` | **no-op default, not overridden** (`crates/boyko_rhi/src/encoder.rs:408`) | reflections' **RK-16** asks for it to become loud or implemented. Whichever of reflections R6 and DM6 lands first implements it; the other consumes it. With G0 as the default, DM6 may not need it at all. |
| `INDIRECT_BUFFER` usage on `gClassify` | absent | DM6, G1 only |
| `vkCmdDrawIndexedIndirectCount` | not loaded, deliberately (1.2 feature not chained) | not needed: F13 buckets on the host |
| `VK_EXT_device_generated_commands` | absent | optional, after DM6 (G4) |
| Async compute | one queue | not needed |

---

## 6. Shader-variant manifest impact

| Rung | New rows | Changed rows (interface delta text) | One-time re-pins |
|---|---|---|---|
| DM1 | 0 | 0 | 0 (no shader edits) |
| DM2 | 0 | 0 | 0 |
| DM3 | 0 | 10 VB tails (+`override_slot` from the VB instance row, + override ring binding); `gbuffer_mrt` `pm`/`mvpm`/`tex` vs+fs (`tex` + the PM ring binding); 4 lit `deferred_pbr`; `forward_opaque.{vs,fs}`, `forward_opaque_froxel.fs` | **23**: 10 VB + 6 raster + 4 deferred + 3 Forward |
| DM4 | **0**, unless gate (c1)/(c2) sends a site to E2 (+1 row each) | the 21 F9 sites gain the `MAT_DYN` constant and block | **21**; plus the WEATHER variants of the same sources (≈ 20) if FX10 lands first |
| DM5 | owned by transparency R2 (`-D MASKED`) | the masked raster variants and the masked caster pipelines gain the override threshold and binding | R2's set |
| DM6 | G0: 0 (the uber `vb_shade` re-pins per program under the source policy); G1: P_heavy × live variants | — | policy-dependent (§9 Q1) |

**With the effects thread's WEATHER landed as specified** (F9a): the 22 lit-producer `.spv` become up
to 44 (+22 WEATHER), and DM4 adds none. If WEATHER also took the specialization route, with the
fragment-stage field covering its 2 raster sites, it would add 0 rows instead of 22.

DM3 and DM4 re-pin overlapping files twice. That is chosen deliberately. Each rung's red-first test
and golden set stay small enough to attribute a moved hash to one change.

---

## 7. The rung plan

Every rung has three gates:
- **(a) red-first:** tests written first that are RED on the trunk and turn GREEN with the rung;
- **(b) golden:** the existing image goldens byte-identical, plus any new golden it adds;
- **(c) perf:** a measured budget on the RTX 3060, taken in the owner's quiet window. Timing runs need
  his go-ahead; this design only specifies them.

### DM0: documentation truth

Corrects the stale statements of §10: a doc agent for the docs, the owner for the memo. Also records
the two live defects (§10.2) where their code lives. Gate: the anchors gate green.

### DM1: Tier 1 made real, and the two live defects fixed

**Lands:**
- `Assets.edited` + count (kernel);
- `MaterialUploadStaging` and `stage_material_edits`;
- the device-local table with compact staging (F1) and the grow protocol (F3);
- the `material_upload` pass in all three declarators;
- the runner's shared-borrow memcpy;
- the frame-0 full copy;
- per-slot fenced staging growth;
- the `PerInstanceMaterial` upload rule (F7).

This rung comes first because two of its tests are live defects, and the owner's rule puts bugs
before features.

**(a) Red-first tests:**
1. Edit `emissive` at frame 10 and assert that the lit pixel changes on **VB, Deferred, Forward and
   F+, for mesh and SDF pixels**. RED on the trunk (RESEARCH §1.2).
2. **The slot invariant, with a pinned row layout.** Rows `a < b < c`, frame slots 0, 1, 0:
   - frame N−2 writes `b = B0` (slot 0);
   - frame N−1 writes `b = B1` (slot 1);
   - frame N writes `a` and `c` only (slot 0).

   Read the table back after N: `b` must be `B1`. A `[min_row, max_row]` copy from a row-mirrored
   slot 0 writes the stale `B0` and fails. A test with one row per frame could not fail, because then
   min equals max.
3. **Defect D-1, the headroom mint.** Boot with 5 materials (capacity 5). Mint a sixth at frame 10,
   which grows the table to 8 and seeds rows 0..5. Mint a seventh at frame 12, into the headroom
   without a grow. Assert that row 6's values render and read back. Also `fill` a streaming material
   into the headroom after the grow. Both are RED on the trunk by derivation (RESEARCH §1.11), not
   reproduced.
4. **Defect D-2, the falling edge.** Spawn a non-default-material instance at frame 5 among default
   ones. Despawn it at frame 10 while spawning default instances, so the ring renumbers. From frame
   `10 + FRAMES_IN_FLIGHT` on, every instance must render material 0 on VB and Forward. RED on the
   trunk by derivation.
5. **The device-local grow frame.** Mint across a power-of-two boundary while editing another row in
   the same frame. Both rows render correctly, and the grow frame records exactly one full-image
   region.

**(b) Golden:** all goldens byte-identical. Idle frames record no copy, and frame 0's copy writes the
values boot used to write.

**(c) Perf:**
- idle frame: 0 added commands (a command-count assert);
- a 100-row edit: CPU ≤ 5 µs, copy ≤ 10 µs;
- `deferred_pbr` and `vb_resolve` ms with the device-local table ≤ host-visible, at
  1080p/1440p/4K;
- the grow frame's copy time, reported.

### DM2: drivers (Tier 2a), composition, and the shared easing table

**Lands:**
- `DynamicMaterialsPlugin`, `MaterialDriver`, `MaterialModulation`, `drive_materials`;
- the effective-value read in the gather and the stager;
- the curve leaves (eDSL; polynomial sine and exp2);
- the 30 easing bodies and `EasingId` in `boyko_shaderdsl`, re-exported by `boyko_ui`.

**(a) Red-first tests:**
1. A `Sine` `Mul` driver on `emissive` at fixed `Time` steps: the captured pixel follows the `f32`
   oracle within 8-bit quantization on every path.
2. **Composition:** a `Mul` pulse on `base_color` plus a gameplay `get_mut` recolour at frame 20.
   From frame 20 the pixel is `recolour × pulse`, not `authored × pulse`, and not the pulse alone.
   A `Replace` driver on the same lane under a recolour keeps the driver's value, and a second
   `Replace` on that `(material, lane)` trips the debug assert.
3. **A hot-reload `fill`** of a driven material composes the same way.
4. **`Ease`, per family:** each of the 30 built-in `EasingId` curves at t ∈ {0.25, 0.5, 0.75} matches
   an f64 closed form (the RmlUi/Penner definitions) within 4 ulp of f32, the tolerance transparency's
   G20 uses for its host-vs-device leaf. RED on the trunk, where the only evaluator is the identity:
   at t = 0.25 an in-out cubic is 0.0625, not 0.25, so a linear `Ease` cannot pass.
5. **Schedule census:** `drive_materials` precedes `gather_mesh_draws` and `stage_material_edits`,
   and follows every `Assets<Material>` writer.

**(b) Golden:** a new golden at fixed `t` (pulse mid-cycle) on each path; with the plugin absent,
every golden byte-identical.

**(c) Perf:**
- `drive_materials` ≤ 25 µs at 1,000 drivers, ≤ 0.25 ms at 10,000 (criterion);
- gather delta ≤ 5 % at 100k instances with one driven material, and 0 with none;
- upload bytes = 48–96 × changed rows.

### DM3: per-instance overrides and per-instance time

**Lands:**
- `MaterialOverride` and the gather term on all four queries;
- `override_slots` → `VbInstanceRow._pad[0]` and `PerInstanceMaterial._pad[0]`;
- `override_rows` and the override ring, with its bindings (`vb_layout0{,_froxel}`, Forward, Deferred
  `pm`/`mvpm`/`tex`);
- the widened Deferred selector;
- `override_apply` in the VB tails, Forward and the Deferred raster;
- the F8 encoding;
- `InstanceMaterialDriver` and `drive_instance_overrides` (the `K` and `Tint` lanes; `Frame` activates
  in DM4);
- SDF private rows (F16).

**(a) Red-first tests:**
1. Two instances of one material, one overridden: only that one flashes, on all four paths, **for a
   flat and for a textured material**. The textured case exercises the 4 textured VB tails and the
   Deferred `tex` raster, the path pass 1 missed.
2. **A default-material entity (id 0) with an override** flashes on Deferred. RED under a flag-only
   selector. Checked first: the precondition that `pm` and base render an all-default scene
   identically (F7).
3. **Override removal:** remove the component at frame N while the ring renumbers. From
   `N + FRAMES_IN_FLIGHT` no instance flashes on VB, Forward or Deferred.
4. **A streaming-material instance** (`MaterialStale`) with an override flashes on the default
   material.
5. **SDF:** the same gameplay call that flashes a mesh flashes an SDF edit (private row).
6. **Out of phase:** two instances with one flicker `InstanceMaterialDriver` (SCALE mode), different
   seeds, differ in emissive at a fixed `t` on all four paths.

**(b) Golden:** no override → byte-identical after the one-time re-pin; a new flash golden and a SCALE
golden per path.

**(c) Perf:**
- VB shade ms delta: ≤ 1 % with 0 overrides; ≤ 0.07 ms at 1080p with 100 % overridden coverage
  (upper bound [E]: 2.07 M × (32-B L2 load + ~15 ALU));
- gather delta per 10k overrides ≤ 30 µs;
- `drive_instance_overrides` ≤ 25 µs at 1,000 lanes.

### DM4: per-pixel operations (Tier 2b)

**Prerequisites:** `GraphicsPipelineDesc::fragment_spec_constants`, and the register probe extended
to graphics pipelines.

**Lands:**
- `MaterialDynAuthored` and the `MaterialDynGpu` table + staging;
- `dyn_rows` and the per-frame specialization pick;
- the `MAT_DYN` block in the 21 sites;
- the `uv_st`, `flip_cells`, `uv_grad_st`, `flip_blend` and `rim_schlick` splices;
- the driver lanes `UvSt`, `FlipFrame`, `Rim`, and the override's `Frame` lane.

**(a) Red-first tests:**
1. A scrolled-UV material's texel moves by `rate·Δt`.
2. A flipbook blends two cells with the oracle weight.
3. Two instances of one flipbook material with different `t0` show different cells (F17).
4. The rim's `(1−N·V)^5` profile matches the oracle on a sphere, including on an SDF sphere.

**(b) Golden:** capability off → byte-identical after the re-pin; capability on with `dyn_rows == 0`
→ byte-identical; one golden per operation.

**(c) Perf:** F9's gates (c1) fold, (c2) off-path ABBA, (c3) armed cost, each with its fallback.

### DM5: dissolve (after transparency R2)

**Lands:**
- the `max(cutoff, hash < param)` threshold in the `MASKED` raster variants **and the masked caster
  pipelines**;
- the override term on D-U6's masked caster query pair and the `override_slot` lane in
  `MaskedCasterScratch`;
- `AlphaMasked` inserted and removed by the dissolve lifecycle.

**(a) Red-first tests:**
- a dissolving instance's discarded fraction ≈ `param` (±2 % over a 256² patch);
- **its CSM shadow's kept-texel fraction tracks `param` within 5 %** (the transparency G22 shape);
- the HZB follows it.

**(b) Golden:** R2's goldens + one dissolve golden. **(c) Perf:** R2's masked budget; no extra pass.

### The rest of the ladder

**Owner policy decision (§9 Q1).** DM6 waits on it.

**DM6: Tier 3 infrastructure.**
- **Lands:**
  - `MaterialProgram`;
  - the G0 uber `vb_shade` (and G1 for heavy programs, measured);
  - `Dual<S>`, the `SampleGrad` node and the Gaia→eDSL walker;
  - per-program raster pipelines.
- **(a) Red-first:**
  - **P = 1 program reproduces `vb_shade` byte-identically** (the classified path is the oracle);
  - a two-program scene shades each region with its own function, matching the `f32` program oracle.
- **(b) Golden:** all VB goldens byte-identical at P = 1.
- **(c) Perf:**
  - classify vs fused ≤ 0.42–0.54 ms at 1080p (already paid on textured frames);
  - the uber-shader's Register Count bucket vs the base, per program added;
  - G1 empty-bin tax ≤ 0.33 µs × P;
  - the scan's cost at `capacity_rows` measured.

**DM7: vertex animation.** A separate campaign (§9 Q2). F15's prerequisites come first.

---

## 8. Risks

| # | Risk | Mitigation |
|---|---|---|
| R1 | A specialized block does not fold on the driver, and costs occupancy when off (F24, AMD's warning) | DM4 gate (c1) per site, with an E2 fallback per site; (c2) ABBA catches a folded-but-slower site |
| R2 | A copy reads another slot's stale bytes | compact staging makes the F1 invariant structural; DM1 red test (2) with the pinned layout |
| R3 | A missing `drive_materials → gather` edge desynchronizes the per-instance lanes from the table by a frame | the schedule census (DM2); the escalation to B2 is gated on removing the lane copies |
| R4 | Two `Replace` drivers on one `(material, lane)` | the debug uniqueness maps; `Mul`/`Add` compose by rule (F4) |
| R5 | The frame-0 and grow copies change the boot ordering contract (`boyko_app::runner`) | DM1 red tests (1) and (5) run from frame 0 and across a grow; the table is not bound before `material_upload` |
| R6 | Live defects D-1 (headroom mint) and D-2 (stale PM ring) keep hitting streaming scenes until DM1 lands | DM1 is ordered first; red tests (3) and (4) make both visible |
| R7 | Tier 3 stalls on the policy question and infrastructure is built anyway | DM6 is explicitly after §9 Q1; nothing in DM1–DM5 presumes Tier 3 |
| R8 | The 7-bit `k` in Deferred bands on slow fades | 1/127 steps; a 0.5 s fade at 60 fps uses 30 of them [E]. Revisit only on an observed artefact. |
| R9 | The libm `sin` or `exp2` flips a golden across hosts | the polynomial sine and exp2 (F6) |
| R10 | Wind or VAT started before its prerequisites breaks culling, motion vectors or shadows silently (UE and AMD documented all three) | DM7 lists them first; nothing in DM1–DM6 depends on it |
| R11 | The `pm` and base Deferred pipelines disagree on material 0, so widening the selector moves pixels when the first override appears | DM3 checks that precondition first (F7) |
| R12 | `k` is one scalar per instance: a flickering torch cannot also be hit-flashed | stated (F7); the gameplay layer picks the mode. Lifting it needs a second free Deferred G-buffer byte. |
| R13 | The effects thread lands `-D WEATHER` and this design lands `MAT_DYN` without a common policy, doubling re-pins or reintroducing a runtime branch | F9a's policy, recorded in both directions; §6's arithmetic covers either order |

---

## 9. Decisions taken here, and the owner's questions

### 9.1 Technical forks decided in this document

- F1: edited rows only, compact staging, the slot invariant by construction.
- F2: `Assets.edited` as a kernel feature, set on free too.
- F3: a device-local table and its grow protocol.
- F4: CPU drivers composing onto the authored value (`Replace` / `Mul` / `Add`).
- F5: a virtual clock by default and f64 phase, with P = 1,024 s kept for DM7.
- F6: eDSL curves, polynomial sine and exp2, the easing table in `boyko_shaderdsl`; wetness left to
  the effects thread.
- F7: the override slot on the VB instance row and the PM lane; the PM upload rule; the widened
  selector; overrides on streaming instances.
- F8: `k` and its mode through `gAlbedo.a`.
- F9, F9a: a specialization constant with per-frame selection and a measured E2 fallback; one gating
  policy for the lit producers.
- F10: a cold `MaterialDynGpu` and a plugin-owned authored column.
- F11: dissolve on the `MASKED` rail, casters included.
- F12: G0 by default, G1 for heavy programs.
- F13: per-program raster pipelines; SDF's Tier-3 gap stated per path.
- F14: extend the eDSL with `Dual`, `SampleGrad` and a walker.
- F15: in-shader vertex animation.
- F16: private rows for SDF edits under the same override component.
- F17: per-instance drivers on the CPU.

### 9.2 Owner VALUE / SCOPE questions (only these)

1. **Tier 3 policy (unchanged from the 2026-08-27 memo).** Is a material program a **source**
   (engine-authored, committed `.spv`, byte-gated) or an **asset** (designer-authored, compiled at
   content time outside the cargo byte gates)? Under G0 a new program re-pins the uber-shader
   (source) or regenerates it at content time (asset). DM6 waits on this.
2. **Scope: vertex animation.** Are wind and VAT their own campaign (recommended: they touch the most
   passes, F15), or part of this one?

### 9.3 Questions withdrawn in pass 2 (they were technical)

| Pass-1 question | Why withdrawn | Now |
|---|---|---|
| Q3: is a per-edit unique material row acceptable for SDF, given the 65,536-row cap? | `MAX_SDF_EDITS = 16`, so SDF uses at most 16 rows. That is arithmetic, not a value. | F16, decided |
| Q4: accept that UV operations reach Forward only after textured Forward lands? | Forward samples no textures, so there is nothing to transform. The question was vacuous. | F9 counts no Forward UV site |
| Q5: is the material side of rain in DM2? | A technical fork, and the effects design had already taken it (per-pixel `-D WEATHER`, which also knows shelter) | F6, withdrawn from this design |

---

## 10. Stale statements and live defects this design found

### 10.1 Stale statements

Each item is left for its owner to correct: DM0 for the docs, the owner for the memo.

| Where | Statement | Correction |
|---|---|---|
| Owner memo, 2026-08-27 | Tier 1 "WORKS TODAY" | It does not; DM1. |
| Owner memo; `docs/VB-P2-CLASSIFICATION-PLAN.md:37` (D2) | The FFI lacks indirect dispatch | Loaded and used (particles). D2's over-dispatch is still a valid choice for one shader. |
| `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:2790-2791` | Neither `vkCmdDispatchIndirect` nor the Count variant is in the fn table | Only the Count variant is absent. |
| Owner memo | VB-P2 is "dark infra, unwired" | P2c is live (`vb_use_classified`). |
| `crates/boyko_render/src/material_table.rs` module doc and `flush_if_dirty` doc | "At this rung (A1) no caller ever mutates a material after boot" | False in effect: a post-boot `add` or `fill` into the power-of-two headroom is a change the table never sees (§10.2 D-1). |
| `crates/boyko_sdf_math/src/lib.rs:125`, `:135` (`SdfEdit` layout and `params` field docs) | `params.w` "unused" | It is the capsule radius (`edit_view`, `lib.rs:732`). |
| Upstream research report | `ViewUniform`'s free lanes as a time carrier; spec constants "contradictory" | `CompositePushConstants` is the shader block; spec constants ship for compute only. |
| This design's own pass 1 | the override slot in `PerInstanceMaterial` reaches "VB (all tails)" | The 4 textured VB tails bind `PerInstanceMaterialTex` at that binding (F7). |

### 10.2 Live defects (traced, not reproduced; RESEARCH §1.11)

| # | Defect | Where | Fixed by |
|---|---|---|---|
| D-1 | A material minted or streamed into the table's power-of-two headroom after a grow keeps an all-zero GPU row, until a later grow (forever, after the last one). | `material_table.rs:404` (early return), `:407` (power-of-two capacity); `flush_if_dirty` has no caller; `add` does not bump `dirty_gen` | DM1: `add`/`fill` set the edited bit; red test (3) |
| D-2 | After the last non-default-material instance disappears, the `PerInstanceMaterial` ring keeps stale ids that VB and Forward read at renumbered indices. | `runner.rs:1788` (flag-gated upload) vs `vb_resolve.comp.hlsl:269`, `vb_classify_count.comp.hlsl:76`, `forward_opaque.vs.hlsl:157` (unconditional reads) | DM1: the F7 upload rule; red test (4) |

---

## 11. Review log

The first architecture critique of pass 1 returned CHANGES_REQUESTED with 1 critical, 10 important
and 9 optional remarks. Every claim below was re-checked against the tree at `6394bc5e` before it was
accepted. **Result: 11 of 11 critical and important remarks accepted, 0 refuted. All 9 optional
remarks adopted.**

| # | Remark (short) | Verdict | Where it landed |
|---|---|---|---|
| C1 | The override slot in `PerInstanceMaterial` never reaches the 4 textured VB tails | **accepted**: the textured sets bind `PerInstanceMaterialTex` at binding 1 (`vb_shade.comp.hlsl:92-110`, `targets.rs:657`, `:687`) | F7: the VB carrier moved to `VbInstanceRow._pad[0]`, with a carrier-by-path table and costs; DM3 test (1) adds a textured fixture; RESEARCH §1.9 corrected |
| W1 | F9's runtime branch ignores F24 and contradicts the VFX design | **accepted.** One detail corrected: the UV transform runs once per pixel, not once per slot, because all five slots share one UV and gradient pair. The flipbook still lands at the register peak, so the conclusion stands. | F9: F24 comparison with size proxies; E4 withdrawn; E3′ specialization with per-frame selection; compute/raster split with the RHI change; gates (c1)–(c3) with a route per failure; F9a: one policy with the effects thread; §0 headline and §6 arithmetic corrected |
| W2 | Widening only the upload gate leaves the selector and the unconditional readers wrong | **accepted**, both parts. Part (b) is also a live defect for `pm.id` today. | F7: the selector keyed on `pm_live`; the `pm_hw[s]` upload rule; §10.2 D-2; DM1 test (4); DM3 tests (2) and (3) |
| W3 | Per-frame staging in the runner needs a heap-allocating take-out | **accepted** | §4.1: the drain is an ECS system over Send resources; the runner holds two shared borrows (`resource_api.rs:52`, `:172`); the take-out stays in the grow branch |
| W4 | DM1 test (2) cannot fail; test (3) misses the wider headroom bug | **accepted** | F1: compact staging; DM1 test (2) with the pinned `a < b < c` layout; test (3) retargeted at the headroom mint and post-grow `fill`; §10.2 D-1; DM1 ordered first as a bug fix |
| W5 | No per-instance time: explosions and flicker play in lockstep | **accepted** | F17 (T3 per-instance drivers, T4 GPU escalation with its trigger); the override record gains `k`, a mode bit and `frame`; F10 stores the grid so an instance can pick its cells; DM3 test (6), DM4 test (3) |
| W6 | Wetness is decided twice; drivers on one lane do not compose | **accepted** | F6: `Wetness` and `SurfaceWetness` withdrawn, the effects thread owns wetness; F4: the authored/effective split and the `Replace`/`Mul`/`Add` rule; DM2 tests (2) and (3); Q5 withdrawn |
| W7 | SDF limits understated; Q3 and Q4 are technical | **accepted**, all four parts | F16: the same component on SDF through private rows, no re-encode (`sdf_edit.rs:24`); F13: Tier 3 per path, with the SDF switch candidate; §9.3: Q3 and Q4 withdrawn |
| W8 | F12 leaves out the uber-shader | **accepted** | F12: G0 row with registers, empty bins, RHI and SDF reach; decision G0 by default, G1 per heavy program by measurement |
| W9 | Easing depends on an unscheduled UI rung; the gate tests only `Sine` | **accepted** | F6: DM2 lands the 30 bodies and `EasingId` in `boyko_shaderdsl` (zero dependencies), re-exported by `boyko_ui`; host-stable `exp2`; DM2 test (4) |
| W10 | A device-local table cannot regrow "as today" | **accepted** | F3: the grow-frame protocol (full image into the fenced slot, one region, ordering against the edited runs, per-slot staging growth); DM1 test (5) |
| O1 | Reader census: 8 binders, not 11; SDF-DDGI binds no material table | adopted | RESEARCH §1.1; F16 (global illumination); F10 "eight binding readers" |
| O2 | `SdfEdit.params.w` is the capsule radius | adopted | RESEARCH §1.9, §1.10; §10.1 |
| O3 | Five small numeric errors | adopted, all five | RESEARCH reference-GPU note (1.24×, sourced [S 45]), §2.9 (0.42–0.54 ms), §2.13 (0.49 rad); F5 (2⁻¹⁴ s, 0.002 rad); F4 B3 (3–10 µs); §4.1 (7 lanes) |
| O4 | Indirect dispatch has no base workgroup; the probe is compute-only; RK-16 ownership | adopted | F12 G1; F9 gate (c1) prerequisite; §5 |
| O5 | Destiny TFX supports F4 and qualifies F14 | adopted, verified at the source | RESEARCH §2.14; F4 precedent; F14 qualified |
| O6 | The `dynamic` sidecar costs every `Material` row | adopted | F10: plugin-owned `MaterialDynAuthored`; `Material` unchanged |
| O7 | D2 vs D3 bandwidth on an all-default scene | adopted | F7: the case stated (2× D2 on raster; 0 extra on VB) |
| O8 | Four gather query sites; streaming instances | adopted | F7: all four; overrides apply to `MaterialStale` instances; DM3 test (4) |
| O9 | The dissolve `param` must reach the masked casters | adopted | F11 and DM5: the override term on D-U6's query pair, a lane in `MaskedCasterScratch`, a binding on the masked caster pipelines, a shadow test |

**The critique's open questions:**
1. *Who keeps `TABLE_ACTIVE` correct?* The word is gone. The host's `dyn_rows` count replaces it,
   maintained by the stager (the only writer of GPU rows), exact because `free` sets the edited bit
   (F2, F9).
2. *Do the frame-0 and grow copies cover `MaterialDynGpu`?* Yes. Both tables share capacity, grow
   together, and take the same full-image copy (F3).
3. *What does ordering materials by program add to the scan?* Nothing under G0. Under G1, one 2-B
   load per iteration of the existing `capacity_rows` loop, plus a permutation upload on
   program-assignment changes. The scan's absolute cost at the cap is measured in DM6 (F12).
