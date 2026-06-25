# Architecture: Hybrid Performance & First-Class Multi-Strategy Selection

> `docs/ARCHITECTURE-HYBRID-PERF.md` — branch `ecs`.
> Governing directive: the engine is **hybrid (mesh + SDF) by design**, the end goal is **maximum performance in any situation**, and the engine must carry **several interchangeable strategies per capability and pick the fastest for the current situation** — at build-time, scene-load, per-archetype, per-entity, or per-tile runtime, whichever is cheapest. This document formalizes that as an engine principle, names the ECS-native mechanism that already exists in scattered form, and gives a prioritized roadmap. **Analysis + proposed mechanism + roadmap — not an implementation.**
>
> **Number provenance tags** used throughout: `[MEASURED:<bench>]` = from a criterion/bench run; `[DERIVED:<formula>]` = first-principles op-count shown inline; `[ESTIMATE:needs-calibration]` = engineering estimate, NOT yet measured, must be calibrated before it gates an auto-selector. Any auto-selector shipping on an `[ESTIMATE]` threshold ships it as a **provisional const** and is a HARD dependency on P10 calibration (see Part 5).

---

## Part 1 — The Principle, Formalized

### 1.1 Statement (Principle 9: Adaptive Strategy Selection)

> **A capability carries N interchangeable implementations ("strategies"). The engine selects the one that is the measured/first-principles perf winner for the *current situation* via a *near-zero-cost selector*, hoisted to the *coarsest granularity at which the choice is stable*. A strategy is admitted to a capability ONLY with a perf rationale naming the situation in which it wins, backed by a `[MEASURED]` or `[DERIVED]` crossover. The default/OFF path of every strategy gate is byte-identical to the un-strategized code (the "0%-gate").**

This is not a new subsystem. It is the *naming and the admission discipline* for five selection carriers the engine already runs ad-hoc, plus one cold policy layer that is genuinely missing.

### 1.2 The selection granularities (coarsest → finest)

The single most important rule, validated by Nanite/Lumen/VSM and by the engine's own StorageKind/EnableTag idioms: **hoist the strategy decision to the coarsest granularity at which it is still stable, so the branch is amortized over the most elements and the inner loop stays monomorphic.** A per-element `match strategy { ... }` inside a hot loop is the anti-pattern — it multiplies branch density and I-cache footprint by N.

| Granularity | Selection cost | When appropriate | Existing engine carrier |
|---|---|---|---|
| **Build-time / platform** | **0 instr** (dead arm stripped by DCE) | Choice known at compile/config-finalize, never varies per-scene (ISA: AVX2 vs AVX-512; CPU-oracle vs GPU-recorder) | `cfg(target_feature)`; shaderdsl `S=f32` (Eval) vs `S=Emit`; const-fold `NEEDS_CHANGE_DETECTION` |
| **Scene-load** | 1 cold classify (paid once at archetype mint / scene build) | Choice is a static property of the data layout or whole scene (storage shape, residency, solver algorithm, clusters-on-for-this-scene) | `StorageKind {Table,Bitset,Dense}`; `ResidencyKind {Cpu,Gpu,CpuPinned}`; `RigidSolver` static-dispatch seam |
| **Per-archetype (mint-time)** | 1 `test`/`jz` per archetype (~0/row amortized) | Choice uniform across a chunk AND a **static function of the archetype signature/residency** (computable once at mint, never mutated live) | `ArchetypeFlags(u16)` 0%-gate (bit 11 `GPU_RESIDENT`; bits 12-15 free) |
| **Per-entity (static)** | **0 instr** (query mask routes the entity) | Capability is a structural property decided at spawn (mesh vs SDF, can-vs-cannot) | Capability-as-component (QueryState include/exclude mask) |
| **Per-entity (runtime)** | 1 bit test/row | Same entity flips a capability often, migration too costly | `EnableTag` bitset + `IsEnabled<T>` (LightEnabled, physics Simulated/Kinematic) |
| **Per-frame (global)** | 1 uniform load + 1 predicated branch / invocation | GPU whole-dispatch mode, A/B-toggleable with no re-record | `FineMarcherPush` gates (`brick_enabled`, `coarse_enabled`, `brick_levels`, `clusters_enabled`) |
| **Per-tile / per-pixel runtime** | 1 precomputed-scalar compare | Choice genuinely varies *within* a frame; produced by a cheap CLASSIFY pass | `select_level` (M4 clip-map); `classify_brick` → pointer grid; L1 cluster cull |

**Critical granularity constraint (folds critique B1/Primitive-C blocker):** the per-archetype carrier (`ArchetypeFlags`) is **mint-time-only**. Its word is read **locklessly as a stable `u16` during structural fires** (verified: `archetype_flags.rs` lines 58-69 — `HAS_ENTITY_OBSERVER` was made STICKY/set-once *precisely because* a runtime-mutable bit races that lockless read). Therefore a per-archetype strategy class is admissible **only if it is a pure static function of the archetype signature/residency, OR-computed once at mint exactly like `GPU_RESIDENT`, and never mutated while the archetype is live.** A strategy that varies for the *same* entity over time (LOD band, near/far, density class) is **NOT** an `ArchetypeFlags` candidate — it routes to the per-row carrier (C3) or to a cold side field outside the lockless word. This distinction is load-bearing and is the reason P4 below is scoped down to demand-driven mint-time classification only.

### 1.3 The Key Tension (and its reconciliation)

**The tension is real and load-bearing:** every additional strategy is I-cache weight and branch-predictor pressure *even when its gate is off* (a const-gated dead arm still occupies the binary; a resident-but-dormant GPU path still costs register pressure in an ubershader; a per-tile `match` with N arms bloats the loop body and diverges warps). "First-class ability to choose N solutions" is therefore in direct conflict with "zero runtime overhead" if pursued naively.

**The reconciliation is two-fold and non-negotiable:**

1. **The selector must dominate the work it saves.** Selection cost is on a strict ladder: build-time DCE (0) > scene-load classify (once) > per-archetype `test/jz` (~0/row) > per-entity bit (1/row) > per-frame uniform (1/dispatch) > per-tile scalar compare (1/tile). Pick the **coarsest** carrier the situation permits. Mis-placing granularity — a per-pixel branch for a scene-constant fact — is the main avoidable cost.

2. **Strategy admission is gated by a named winning situation with a `[MEASURED]`/`[DERIVED]` number.** A capability's strategy set is *curated, not speculative*. This mirrors how UE5 carries a technique only where a real situation makes it win (overlapping instances → SW SDF tracing; huge view distance → VSM clipmap; massive source geometry → Nanite cut). The M1 brick empty-skip pays for itself *only* when scenes have large empty volumes; on a fully-covered tile it is pure overhead — which is *exactly why* it is a runtime A/B gate, not always-on. **This is the guard against I-cache blowup: speculative strategies are forbidden; the gate's existence is justified by a measured crossover, AND no hot-path carrier reserves capacity for a strategy whose first consumer does not yet exist** (the demand-driven rule — see P4).

The directive's "ability to choose" and Principle 7's "don't bloat the hot path" are reconciled because **the choosing is cold** (build-time, scene-load, or a per-tile classify pass amortized over many ray steps) and **the chosen kernel is hot, monomorphic, and branch-free**.

### 1.4 The 0%-gate invariant (load-bearing) — and its ONE failure mode

Every render/lighting gate already obeys it: the OFF (`==0`) path is *byte-identical* to the pre-strategy code (const-asserted `std430` offsets, cmp-`.spv`), and unused descriptors are bound-but-unread. This guarantees that **the worst case for strategy-N's GATE is today's performance, never worse.**

**The one failure mode the 0%-gate does NOT cover (folds critique B2):** when an auto-selector chooses *between two ON strategies* (e.g. AllPairs vs Grid), **both arms are "on" and the 0%-gate gives no protection** — a mis-calibrated threshold can select the *slower* strategy near the boundary, a real regression. The 0%-gate protects the *off* path; it does not make a *wrong selection* free. This is why every auto-selecting threshold (P1/P2/P3) is a HARD dependency on P10 offline calibration and ships as a provisional const flagged `[ESTIMATE]`, and why every temporal selector carries hysteresis (Part 2.4). The invariant must be preserved on every new gate, AND every auto-select must name its crossover provenance.

---

## Part 2 — The Mechanism

### 2.1 What already exists: five carriers + one missing layer

The engine has, today, five distinct zero/near-zero-cost strategy-selection carriers, each matched to a granularity. They are correct and battle-tested; they are merely **uncoordinated and unnamed**. The synthesis is to *name them as the legal implementations of a Strategy and add the one missing piece* — a cold policy layer — **not to invent a sixth carrier or a `dyn` registry.**

| # | Carrier | Granularity | Selection cost | Engine references |
|---|---|---|---|---|
| C1 | **Compile-time monomorphization** | build-time | 0 (path absent from binary) | shaderdsl `EvalCf`/`EmitCf`; `cfg(target_feature)` AVX2/AVX-512; const-fold gates |
| C2 | **Capability-as-component (structural skip)** | per-entity static | 0 (archetype not matched) | QueryState include/exclude mask; `find_matching_archetypes_into` |
| C3 | **Per-row capability datum** (`IsEnabled<T>`/EnableTag) | per-entity runtime | 1 bit test/row | `data_is_enabled.rs`; physics `Simulated`/`Kinematic`; `LightEnabled` |
| C4 | **Archetype-mint classification** (`StorageKind`/`ResidencyKind`) | scene-load / mint | cold AtomicU8 load at mint | `component_registry.rs` STORAGE_KIND/RESIDENCY_CLASS |
| C5 | **Per-archetype branchless gate** (`ArchetypeFlags` u16, mint-time-only) | per-archetype | 1 `test`/`jz` per archetype | `archetype_flags.rs` (bit 11 `GPU_RESIDENT`, **bits 12-15 free**) |
| C6 | **GPU wave-uniform gate-word** (`FineMarcherPush`/header) | per-frame global | 1 uniform load + predicated branch | `brick_enabled`, `coarse_enabled`, `brick_levels`, `clusters_enabled` |
| C7 | **GPU per-tile CLASSIFY→consume** | per-tile/per-pixel | 1 scalar compare/tile | `classify_brick`/pointer grid; `select_level`; L1 cluster cull |

**Missing:** a cold **policy layer** — there is no cost-model resource in the kernel. Every gate today is set by the caller/owner by hand (an A/B bool); none is chosen *automatically* from a measured situation key (e.g. "if light_count > T → clusters_enabled"). This is the only genuinely new component, and it is *cold* (runs at scene-load / a dedicated end-of-gather sync point, writes a gate word) so it adds **zero** hot-path cost.

### 2.2 The proposed mechanism: `StrategySelect` — a vocabulary + two new primitives

Define a Strategy as a triple **(Capability, Situation-key, Granularity) → Carrier**. Realize it with two concrete, ECS-native additions on top of the existing carriers — neither introduces `dyn`/`Box`/`HashMap` on the hot path:

#### Primitive A — `SceneStats` Resource (the cold cost-model input)

```rust
/// Cold aggregate read ONCE per policy per frame, NEVER per row. Principle-0:
/// an ECS Resource, not a side store. SEE "Write discipline" below — this is
/// NOT a free shared-mutable resource; its producer/consumer phasing is part
/// of the contract.
#[derive(Resource)]
#[repr(C)]
pub struct SceneStats {
    pub point_spot_light_count: u32,   // → clusters_enabled crossover
    pub active_body_count:      u32,   // → broadphase AllPairs vs Grid; CPU vs GPU
    pub max_island_constraints: u32,   // → colored-parallel vs manifold-order solve
    pub screen_empty_fraction:  f32,   // → coarse tile-cull on/off
    pub sdf_edit_count:         u32,   // → brick vs analytic crossover
    pub world_extent_ratio:     f32,   // → clip-map levels vs flat
    // ... grows only as a strategy needs a stat; each field justified by a gate.
    // --- selector band-state (folds hysteresis-carrier concern): the current
    // side of each banded selector lives HERE, Principle-0-native, not an
    // ad-hoc `static`. One bit per banded selector. ---
    pub selector_bands: u32,           // bit i = current side of banded selector i
}
```

**Write discipline (folds critique-2 B3 — the multi-writer ResMut blocker).** `SceneStats` is **NOT** a free shared-mutable resource. On the parallel scheduler a multi-writer `ResMut` is a conflict-graph edge that would serialize the gather phase or race if mis-declared. The contract is:

- **Each field has exactly ONE owning producer system.** `point_spot_light_count` is written only by `collect_lights` (which already counts them host-side); `active_body_count`/`max_island_constraints` only by the broadphase/`build_graph` geometry pass; `screen_empty_fraction`/`sdf_edit_count` only by the render gather. No two systems write the same field → no write-write conflict on a field.
- **All writes complete in the gather/setup phase; all policy reads happen in a LATER stage.** The `StrategyPolicy` systems (Primitive B) are scheduled strictly *after* a dedicated end-of-gather sync point. A policy therefore reads `SceneStats` with **no concurrent writer** — it is `Res<SceneStats>` (shared read), never `ResMut`, and the producers are `ResMut` only during gather. This is the single cold sync point; it adds one barrier already present at the gather→simulate boundary, not a new one.
- Net: producers contend only field-locally during a phase the scheduler already separates from consumers; the hot path never touches `SceneStats`.

#### Primitive B — `StrategyPolicy` (a cold pure fn per capability)

```rust
/// A pure, COLD function: SceneStats → gate word / StrategyId. Runs AFTER the
/// gather sync point (see write discipline), writes the EXISTING gate (a
/// FineMarcherPush field, a LightHeader word, a BroadphaseKind enum, a
/// SceneStats.selector_bands bit). The ONLY place a heuristic lives. Thresholds
/// are CALIBRATED offline (P10) then frozen as provisional consts; each carries
/// a [MEASURED]/[ESTIMATE] tag at its definition site.
fn select_lighting_cull(s: &mut SceneStats) -> bool {
    // banded: switch-up at T+δ, switch-down at T−δ; current side in selector_bands.
    banded(s, BAND_CLUSTER, s.point_spot_light_count, CLUSTER_LO, CLUSTER_HI)
}
fn select_broadphase(s: &mut SceneStats) -> BroadphaseKind {
    if banded(s, BAND_BROAD, s.active_body_count, GRID_LO, GRID_HI)
        { BroadphaseKind::Grid } else { BroadphaseKind::AllPairs }
}
```

#### Primitive C — mint-time `StrategyClass` riding `ArchetypeFlags` (demand-driven, mint-time-only)

The only admissible per-archetype hot-path strategy carrier is a **static-at-mint classification**, OR-computed once from a cold per-component table exactly as `GPU_RESIDENT` is, read as one `test`/`jz` on the already-loaded `flags` word, and **never mutated while the archetype is live** (Part 1.2 constraint; the lockless-read race is real). Its admission is **demand-driven, not speculative** (folds critique-1 B1):

- **No bits are reserved now.** Bits 12-15 remain free. `ArchetypeFlags` is **not** widened to `u32`. Widening would edit a `#[repr(transparent)] u16` guarded by a `const { size_of == 2 }` assert (`archetype_flags.rs` ~line 342) and re-touch every structural-fire immediate — a hot-path kernel-struct change with **zero present benefit**, so it is rejected outright, not deferred.
- **One strategy bit is introduced at the moment a concrete CPU consumer with a `[MEASURED]`/`[DERIVED]` crossover lands** — i.e. an actual system that reads it, classifying on a property that is a *static function of the archetype signature* (so it cannot vary live). The clearest near-term candidate: a **per-archetype broadphase eligibility class** (e.g. "all bodies in this archetype are static" → skip the dynamic broadphase entirely), which is a signature property, not a live-varying one. Until such a consumer exists, no bit is claimed.
- Runtime-varying per-archetype facts (LOD band, near/far, density) are explicitly **routed away** from `ArchetypeFlags` to C3 (per-row datum) or a cold side field. The GPU twin (`select_level`) is per-pixel and lives in the shader — it is **not** an `ArchetypeFlags` consumer and was never proposed as one.

This makes Primitive C identical in discipline to how `GPU_RESIDENT` itself was added: a free bit claimed only when its consumer (Phase-4 GPU residency) existed.

### 2.3 GPU-side: keep selectors wave-uniform, sort for coherence

GPU divergence is the I-cache tension in GPU clothing: an incoherent per-thread strategy branch can halve occupancy. Three rules:

1. **Whole-dispatch mode = wave-uniform gate word (C6).** `brick_enabled`/`coarse_enabled`/`clusters_enabled` are coherent uniform branches — every lane takes the same path, the untaken side is skipped not serialized. Free. Keep this for any scene/frame-constant choice.
2. **Per-tile/per-object choice = CLASSIFY → bin → consume (C7), the Nanite pattern.** A cheap classify pass labels each tile/cell with its strategy (`classify_brick` → EmptyOutside/Surface; the coarse TileBound pass → empty/near_t), written to a **transient** buffer (Principle-0-legal: rebuilt from the one authority each regen, never a durable side store). The fine kernel then reads one class code + takes a *uniform-per-tile* branch. The selection is paid once per tile, amortized over 64 fine pixels and many ray steps.
3. **Divergent-within-warp = separate compute pipeline per per-strategy archetype — a NEW seam, NOT free (folds critique-2 B-GPU blocker).** Where strategies need different resource bindings (the M1/M2 resource-binding islands the eDSL cannot dynamically index), a distinct pipeline dodges uber-shader register pressure AND divergence. **This is explicitly a new record-time seam, not a consequence of the data-column key:** (a) each per-strategy pipeline is a **distinct PSO recorded under `DispatcherToken` on the `!Send` GPU thread** — it lives inside the compiler-enforced GPU-access discipline, not outside it; (b) its binding layout MUST be a **stable superset (declare-but-gate)** to preserve the byte-frozen `.comp.spv` contract — a new strategy adds a *gated* binding, never mutates the frozen layout; (c) the `DeviceColumnHandle` + `(ArchetypeId, ComponentId)` key addresses **data only**, never pipeline/binding selection. This rule is therefore tied to the P5/P6 risk tier, not presented as already-supported infra.

### 2.4 Selector stability: hysteresis is mandatory, and the TRANSITION is budgeted

Any selector keyed on a temporal metric (camera distance → LOD level, light count crossing a threshold, island size) MUST carry hysteresis or a dirty-channel, or it thrashes at the boundary (re-bake whole atlas / LOD pop every frame / re-alloc a grid). The engine already does this correctly: `snapped_level_origin` floor-snaps each clip-map level to its own brick lattice (the discrete-snap analog of LOD hysteresis), and `LightTableDirty` is the explicit rebuild channel. Every banded selector here switches up at `T+δ` and down at `T−δ`, with the current side stored in `SceneStats.selector_bands` (named carrier — folds the hysteresis-carrier concern; it is Principle-0-native, not an ad-hoc `static`).

**The transition cost itself is budgeted, not assumed free (folds critique-1 transition concern).** A flip is rare (hysteresis guarantees it) but **not free**: `clusters_enabled` off→on must populate the cluster grid; AllPairs→Grid must build the broadphase grid. The rule: **every flippable strategy's working buffers are preallocated at scene-load from the pool (Principle 5), sized for the worst case, so a flip is a *fill*, never a `Vec::new`/grow.** The transition is a one-frame fill amortized over the many frames hysteresis keeps the new strategy active; P1/P3 line items below carry an explicit transition-cost note and the preallocation requirement.

### 2.5 What is forbidden (Principles 0/1/4/7)

- **No `Box<dyn Strategy>` / `HashMap<StrategyId, fn>` / vtable on the hot path.** The discriminant is always a precomputed scalar/bit/marker. The **only** runtime-set escape hatch, used strictly *off* the hot loop, is a **hand-written `match` on a `#[repr(u8)]` enum discriminant** — exactly the existing `RigidSolver` Noop/SoftStep/Colored seam. **No external dispatch crate** (`enum_dispatch` and similar are proc-macro dependencies — rejected under the in-house-only constraint; the hand-rolled `match` is what the engine already ships and needs no dependency).
- **No parallel std::Vec/HashMap side store for strategy state** (Principle 0 — the SP4 race lesson). `StrategyId` rides ECS storage: a marker component (C2), a per-row datum (C3), a cold registry table (C4), an `ArchetypeFlags` bit (C5, mint-time-only), a transient classify buffer (C7), or a `Resource` (`SceneStats`/policy output, with the write discipline of 2.2).
- **The frozen `sdf_field.hlsli` + `.comp.spv` are byte-frozen contracts.** A new GPU strategy is a new record-time variant behind a stable *superset* binding layout (the declare-but-gate discipline) or via `boyko_shaderdsl` with a byte-identity gate — never a field mutation.
- **Thresholds are offline-calibrated (P10) then frozen as provisional consts.** A const guess is robust across the single target band (AVX2 x86_64 Win/Linux — the only platform); should the target band widen, the const becomes a cold scene-load re-pick. No threshold is hot-path-recomputed.

---

## Part 3 — Per-Subsystem Analysis

> Number provenance is tagged inline. Where a claim is `[ESTIMATE]`, it does NOT gate an auto-selector until P10 calibrates it.

### 3.1 Render

#### Hard commitments (and the cost they impose)
- **Primary visibility is hard-committed to per-pixel SDF sphere-tracing for the whole screen.** Mesh is a second-class citizen: it contributes ONLY a rasterized D32 depth bound (`t_mesh`) the march clamps against — never its own albedo/normal/material raster. The deferred G-buffer present and the FineMarcherPush global-gate mechanism are also hard-committed (and correct).

#### Candidate strategies + winning situations

**Primary visibility — THE biggest under-delivery on the directive.**

| Strategy | Situation it wins | Perf rationale |
|---|---|---|
| Per-pixel SDF march (current) | Analytic CSG / organic surfaces with no mesh; cheap shared-depth occlusion | `[DERIVED]` per shadowed pixel: `steps × active_edits` FMA-chains. With the campaign cap `MAX_SDF_EDITS=16` and a typical 16–64 steps to convergence → **256–1024 field-edit evals/pixel**; at ~1–2 FMA + branch each, order **~10²–10³ cyc/pixel**. The 10× span is the step-count variance (near-grazing rays converge slow); it is an `[ESTIMATE]` band pending a marcher microbench. |
| **Mesh G-buffer RASTER pass** writing all attributes, marcher dispatched ONLY on tiles with SDF coverage in front of mesh depth | **Mesh-dense content** (characters, props, level geometry) | `[DERIVED]` raster cost/covered-pixel = 1 triangle interpolation + 1 attribute write ≈ **fixed ~10–50 cyc/pixel, independent of edit count** (the structural difference: raster is O(1) in edits, march is O(steps×edits)). The crossover is therefore **not** a tuned constant — raster wins a mesh-covered pixel **whenever `steps × active_edits` exceeds the raster fixed cost, i.e. essentially always for any non-trivial field** (≥1 step × ≥1 edit already ties; 16 steps × 4 edits = 64 evals ≫ a single interpolation). The win scales with edit count and step count, which is exactly the expensive regime. This is the structural justification for P5 being the largest absolute render lever; the absolute cyc numbers remain `[ESTIMATE:needs-calibration]` but the *direction* is `[DERIVED]` and threshold-free. |

- **Selection carrier:** per-tile "has-SDF-in-front" bit carried by the **existing TileBound coarse pass (C7)** — dispatch the cheaper primary-visibility per tile. ECS-native, no dyn dispatch.
- **Contract impact (precise, folds critique-1 P5-contract concern):** P5 touches exactly: (1) a **NEW** mesh-raster PSO (does not alter any frozen `.spv`); (2) the marcher's **invocation domain** — it is dispatched over a tile subset instead of the full screen. Whether the dispatch grid is inside the frozen `.comp.spv`'s assumptions must be verified at the `.spv`-pin rigor the SDF campaign applied to its live-splice pins: if the marcher reads `gl_GlobalInvocationID` against a full-screen extent, a tiled dispatch is contract-compatible (the shader is domain-agnostic); if it bakes the full-screen extent internally, P5 requires a record-time superset variant, not a field edit. **P5 leaves `sdf_field.hlsli` and the marcher field math frozen; it changes the present composite (a NEW tile-classified dispatch) and adds a NEW raster pipeline. This is escalated as Owner-call #1 with this exact artifact list — it is a render-architecture change, NOT a strategy-first-class-ification, and does NOT gate the P0–P4 mechanism.**

**Surface eval — analytic vs brick.** Already orthogonal per-frame gates (`brick_enabled` M1 empty-skip, `brick_trilinear` M2 cubic), selected scene-wide.
- Analytic wins: few edits / sharp CSG (brick rounds creases — campaign keystone). `[DERIVED]` O(active_edits) FMA chains/step.
- M1 empty-skip wins: sparse fields, large empty regions. `[DERIVED]` O(1) AABB-exit jump vs O(edits)/step — wins iff the skipped span saves more steps than the AABB test costs (`[ESTIMATE]` crossover = scene empty-fraction).
- M2 cubic brick wins: smooth/organic far-field where a 16-edit analytic fold dominates. Atlas sample vs fold; pays VRAM bandwidth + bake.
- **Make adaptive:** the classifier *already* computes per-brick EmptyOutside/Surface state — feed it into a **per-tile StrategyTag (C7)**, unifying the scattered global gates into ONE per-tile strategy field instead of N global pushes.

**Tile pre-cull — CRITICAL free lever.** P4b coarse cone-trace is **fully built, golden, 0%-gate-safe — but DISABLED on-screen**: the windowed path hardcodes `coarse_enabled = 0` (verified: `swapchain.rs:3575`, with the descriptor still bound at the offsets noted around lines 2704/3694/3730/4166 so the OFF path is byte-identical). Wins for empty/distant screen regions (`[DERIVED]`: converts 64 full marches into one cone reject + 64 short marches). Loses for screen-filling near surfaces (pure added latency). **Highest-ROI immediate action: activation + a `screen_empty_fraction` heuristic, not new code.** Byte-identical when off → near-zero risk.

**LOD.** M4 clip-map (per-pixel finest-first `select_level`, ≤3 AABB tests) wins for large worlds with near-detail; flat (`brick_levels==1`) wins for bounded scenes (the scan is pure overhead). Selector input = `world_extent_ratio` at scene-load.

### 3.2 Lighting

#### Hard commitments (the biggest gap vs the hybrid directive)
- **Shadows are hard-committed on TWO axes the hybrid goal contradicts:** (1) **exactly ONE caster** (single `pc.light_dir`), while the resolve loops the entire N-light table → point/spot/extra-directional lights are **unshadowed**; (2) **SDF occluders only** (the march calls `field_distance`, never mesh depth) → in a hybrid scene **mesh geometry casts no shadow**. This is the single biggest gap between shipped lighting and the directive.
- AO is SDF-march-only (5-tap), representation-bound: occludes SDF-vs-SDF only, silent across the mesh↔SDF boundary.

#### Candidate strategies + winning situations

**Shadows — the critical hybrid number.**

| Strategy | Situation it wins | Perf rationale |
|---|---|---|
| SDF soft-shadow march (current) | Few SDF surfaces near camera, 1 dominant light | On-demand only on visible shadowed pixels; exact soft penumbra free |
| SDF march extended to N table lights | >1 caster needed | `N_casters × single-light cost` — **MUST be range/cluster-gated** (reuse the L1 cluster list) to stay bounded |
| **Cached SDF visibility map** (bake once) | **Static light + static SDF** | The ONLY regime where a "shadow map of an SDF scene" beats on-demand march — see derivation |
| **Rasterized shadow map** | The **MESH half** of the hybrid | Triangles rasterize ~free; MIN with the SDF march |
| Screen-space contact shadows | Near-field augment | Short ray-march in existing depth buffer |

- **`[DERIVED]` critical finding (the template number — do NOT assume a shadow map helps):** a shadow map of an SDF scene has **no triangles to rasterize — every map texel must itself sphere-march the field.** A 2K×2K map = `4.19e6 texels × ~128 steps × ≤16 edits ≈ 8.6e9 field-edit evals/light/frame`, *more* than the current `1080p ≈ 2.07e6 px × (fraction shadowed) × 64 steps × 16 edits` on-demand cost (`≈ 2.1e9` at a 50%-shadowed frame). A shadow map wins the SDF half **only when cached across frames** (static light + static SDF → bake once → amortized ~0/frame). The hybrid-correct strategy is a **split**: rasterize mesh occluders into a map (`[DERIVED]` ~free per triangle), march+cache SDF occluder visibility, **MIN them**. All selector inputs (light static? SDF static? shadowed-pixel fraction?) are known host-side at scene-load → a near-zero-cost scene-load pick.

**AO — the clearest place to add a SECOND strategy.**
- SDF 5-tap wins: isolated SDF surfaces (`[DERIVED]` 5 field evals × ≤16 edits per SDF pixel, **0 for mesh pixels**).
- **SSAO/GTAO** wins: **mixed mesh+SDF contact regions** — the ONLY AO that crosses the representation boundary, at a fixed K samples/pixel **independent of edit count** (`[DERIVED]` K depth-buffer taps, O(1) in edits). Wins as edit count or SDF-surface fraction grows.
- **Make adaptive:** carry both, select per-pixel by the **existing `is_sdf_lit` mask bit + a depth-discontinuity test** (C7-style, near-zero-cost).

**Many-lights culling — the mature template to generalize.** Flat (L0b) wins ≤~4–8 lights (`[ESTIMATE]`: cull dispatch + grid VRAM unamortized); clustered (L1) wins for many overlapping point/spot lights (`[DERIVED]` per-pixel work bounded to ~1–8 cluster lights vs the full table). Already a one-word header gate (`clusters_enabled`) with a 0%-gate. **The gap: it is a manual scene-load bool, not an auto-select.** `collect_lights` already knows the live count host-side → make the pick automatic from `SceneStats.point_spot_light_count` (one banded compare; provisional const `[ESTIMATE]` → P10). **This is the exact template every other lighting fork (shadow technique, AO technique, GI technique) should adopt: a header word, a structural if, an OFF path byte-identical to the cheapest strategy.**

### 3.3 Physics

#### Hard commitments
- **Narrowphase is a CLOSED primitive `match (ColliderShape, ColliderShape)`** (sphere/box only) — any capsule/cylinder/convex-hull is a **compile wall, not a slow path** (no GJK+EPA fallback). The SDF-vs-analytic split is committed by **stage registration**, not per-pair dispatch.
- **TGS-Soft is the only real solver math** (correct — supersedes PGS), behind the `RigidSolver` static-dispatch seam.
- **CPU for all rigid** (owner values call — correct and permanent; GPU rigid is a poor fit for irregular contact data-dependence).

#### Candidate strategies + winning situations (selection mechanism exists; the SELECTOR is missing)

- **Broadphase never auto-selects by density.** `[DERIVED]` default AllPairs O(n²): `n(n-1)/2` pairs → 100 bodies = 4 950 tests (L1-fine); 5 000 bodies = **12.5M tests/step** (dominant). Grid is `[DERIVED]` ~O(n) but loses below an `[ESTIMATE]` ~100–200 bodies on build+sort overhead. **The `BroadphaseKind` branch exists — only the density SELECTOR is missing.** Add `select_broadphase(SceneStats)` (one banded compare on `active_body_count`; provisional const → P10). **Transition cost (folds 2.4):** AllPairs→Grid populates the grid; the grid cell arrays are **preallocated at scene-load sized to the body cap** (Principle 5) so the flip is a fill, not an alloc; hysteresis keeps it from thrashing at the boundary.
- **Broadphase has NO strategy for size-disparate scenes:** every body whose AABB spans ≥8 cells is dumped into an `[DERIVED]` O(k·n) all-against-all residual (k=10, n=5000 → 50k extra tests, defeating the grid). **Add a second coarse size-class grid** (keeps the grid mechanism, one more cell array) rather than a BVH (avoids tree-refit cost). Wins for "a few large colliders + many small bodies" — a very common hybrid scene.
- **Narrowphase generic-convex arm:** when a 3rd/4th shape lands, migrate the `match` to a **static 2D function-pointer table** indexed by `(ShapeSubType, ShapeSubType)` — one array load + indirect call (the pointer hoisted out of the per-pair loop, not per-element). Add a **GJK+EPA arm entered ONLY for the convex class** (`[ESTIMATE]` ~5–15 support evals) so it never costs the common sphere/box pairs (Principle 7). **Defer until a real non-primitive shape exists — keep the `match` until then (it is optimal).**
- **Solver colored-vs-manifold-order is build-time, not island-size-adaptive.** `[DERIVED]` `build_graph` + greedy coloring is pure overhead at small contact counts; below an `[ESTIMATE]` ~256 contacts the partition cost exceeds the parallel-solve saving; above it `parallel_solve`→#cores and `simd_solve`→up-to-8× per lane. **Add a `max_island_constraints` threshold gate** computed during `build_graph` (already iterating manifolds) — the LargeIslandSplitter trigger, a single scalar compare (provisional const → P10). **Lowest-effort, biggest single-big-pile win.**
- **CPU↔GPU for large-N** (particle/soft/fluid only): the seam exists (SP5) but no element-count selector. GPU amortizes only past an `[ESTIMATE]` ~10⁴ uniform elements — **this is the PhysX-folklore number, NOT boyko's; it depends on `GpuColumnManager` transfer cost and MUST be measured for our path (Owner-call #5).** Wire as a marker component (`GpuSimulated`) gating which system iterates it — the **Simulated-bit structural-skip pattern (C2)**. Future-roadmap; respects "rigid stays CPU."

---

## Part 4 — Existing Adaptivity (credit + mapping)

The engine is **not** ad-hoc randomness — it already runs a coherent multi-strategy discipline. Crediting and mapping each onto the generalized mechanism:

| Existing mechanism | Carrier | Granularity | Maps to |
|---|---|---|---|
| `brick_enabled` / `brick_trilinear` / `brick_levels` / `coarse_enabled` (FineMarcherPush) | C6 | per-frame | wave-uniform gate word |
| `clusters_enabled` (LightHeader word) — **the most mature axis** | C6 | per-frame/scene | the template for all lighting forks |
| M4 `select_level` finest-first scan + `snapped_level_origin` hysteresis | C7 | per-pixel | classify→consume + discrete-snap hysteresis |
| `classify_brick` → pointer grid / `build_dirty_pointer_grid` | C7 | per-cell (cache-by-change) | the Nanite CLASSIFY pass + VSM static/dynamic page caching |
| mesh↔SDF shared-depth present | C6/C7 | per-pixel | nearest-surface composite (degenerate cases of one path) |
| `StorageKind {Table,Bitset,Dense}` | C4 | per-component-id | scene-load classify, monomorphic in the loop |
| `ResidencyKind {Cpu,Gpu,CpuPinned}` | C4 | per-component-id | residency classify → `GPU_RESIDENT` stamp |
| `ArchetypeFlags(u16)` 0%-gate (bit 11 GPU_RESIDENT, bits 12-15 free) | C5 | per-archetype (mint-time) | **Primitive C generalizes — demand-driven, mint-time-only** |
| Capability-as-component (QueryState mask) | C2 | per-entity static | the cheapest selector; mesh-vs-SDF, can/cannot |
| `IsEnabled<T>` / EnableTag (Simulated, Kinematic, LightEnabled) | C3 | per-entity runtime | O(1) flip, no migration — **the home for runtime-varying per-entity strategy** |
| `PhysicsConfig.broadphase` (BroadphaseKind), `IntegrationMode`, `MIN_PARALLEL_BODIES` | C4/scene-load | scene-load | config-enum + count threshold |
| `RigidSolver` static-dispatch seam (Noop/SoftStep/Colored) | C1 | build/wire-up | monomorphized, zero vtable — **the hand-rolled `match` template** |
| `owns_integration()` → `IntegrationMode` early-return | C4 | wire-up | "strategy declares what it needs, pipeline adapts at stage entry" |
| shaderdsl `EvalCf`/`EmitCf`; `cfg(target_feature)` AVX | C1 | build-time | DCE, path absent from binary |
| QueryState archetype-match cache + Added/Changed Tick + `LightTableDirty` | C7-temporal | per-frame | cached-plan + version-compare skip + dirty channel |

**The common pattern across all of them:** (i) a near-zero-cost SELECT, and (ii) an OFF/default path byte-identical to the un-strategized code, with unused resources bound-but-unread. **The cautionary template** (physics body-type) confirms the discipline is a *perf* constraint, not just purity: the pre-remediation enum-branch + std::Vec mirror carried a hidden data race (SP4) AND a fragmenting cache-hostile side store; the fix — dense components + a 1-bit `IsEnabled` test — was strictly *faster AND sound*.

---

## Part 5 — Prioritized Roadmap

Ordered by perf-impact-per-effort. **[FIRST-CLASS]** = make an existing gate auto-selecting/first-class (near-zero risk, 0%-gate proven). **[NEW]** = add a new strategy (requires a `[MEASURED]`/`[DERIVED]` crossover before admission). **P10 is a HARD dependency of every auto-select**, not an optional tail item — an auto-selector on an uncalibrated const can pick the slower arm near the boundary (Part 1.4), a regression the 0%-gate does not catch.

| # | Item | Type | Win / situation | Cost | Risk |
|---|---|---|---|---|---|
| **P0** | **Re-enable P4b coarse tile-cull on-screen** (`swapchain.rs:3575` `coarse_enabled=0` → policy-driven) behind a `screen_empty_fraction` heuristic | [FIRST-CLASS] | Empty/distant tiles: 64 full marches → 1 cone reject + 64 short marches. **Built, golden, byte-identical-when-off.** | S — flip one hardcoded gate + 1 heuristic, no new code | Near-zero (0%-gate) |
| **P1** | **`SceneStats` Resource + `StrategyPolicy` cold layer** (single-owner-per-field writes, post-gather read — Part 2.2); auto-select `clusters_enabled` from live light count | [FIRST-CLASS] | Flat→clustered auto-flip at break-even; the template for all forks. **Transition:** cluster grid preallocated at scene-load → flip is a fill, banded to prevent thrash | M — one cold resource + banded compare; ships provisional const → **P10** | Low (cold path only) |
| **P2** | **Large-island threshold gate** on existing coloring (colored-parallel only past `max_island_constraints`) | [FIRST-CLASS] | Biggest single-big-pile physics lever; reuses ConstraintGraph | S — one compare in build_graph; provisional const → **P10** | Low |
| **P3** | **Broadphase density auto-select** (AllPairs↔Grid from `active_body_count`) | [FIRST-CLASS] | Avoids 12.5M-test/step default on mid-large scenes AND grid overhead on tiny. **Transition:** grid cells preallocated → flip is a fill, banded | S — one banded compare; provisional const → **P10** | Low |
| **P4** | **Demand-driven mint-time `StrategyClass` bit on `ArchetypeFlags`** — claim ONE free bit (12-15) ONLY when a concrete CPU consumer + measured crossover lands (first candidate: per-archetype static-broadphase eligibility). **No reservation, no u32 widen.** | [NEW primitive, demand-gated] | Per-archetype static strategy at ~0/row WHEN a consumer exists; until then, nothing ships | S — one bit + its consumer when justified | Low (mint-time-only, no live mutation) |
| **P5** | **Mesh G-buffer raster pass** (full attributes) + per-tile "has-SDF-in-front" dispatch of the marcher | [NEW strategy — render-arch] | **Largest absolute render win** on mesh-dense frames (`[DERIVED]` direction: raster O(1)-in-edits beats march O(steps×edits) on covered pixels) | L — new raster PSO + tile dispatch | Med-High (Owner-call #1; precise contract impact in 3.1) |
| **P6** | **Hybrid shadows**: rasterized mesh shadow map MIN SDF march, range/cluster-gated to N lights; cache SDF visibility for static light+SDF | [NEW strategy — render-arch] | Closes the "mesh casts no shadow / single caster" gap (`[DERIVED]` shadow-map-of-SDF only wins cached) | L — new map pass + cache + multi-light gate | High (Owner-call #2) |
| **P7** | **Screen-space AO** (SSAO/GTAO) as a second AO strategy, per-pixel selected by `is_sdf_lit` + depth-discontinuity | [NEW strategy] | The only AO crossing the mesh↔SDF boundary | M | Low-Med |
| **P8** | **Size-class coarse grid** for size-disparate broadphase (kills O(k·n) oversized residual) | [NEW strategy] | "few big + many small" common hybrid scene | M | Low |
| **P9** | **Narrowphase 2D function-pointer table + GJK arm** | [NEW strategy] | Unblocks capsule/convex (compile wall today) | M | **Defer** until a real non-primitive shape lands |
| **P10** | **Offline threshold calibration** (criterion) for ALL crossovers (cluster, grid, island, parallel, GPU-N) | [FIRST-CLASS] **— HARD dep of P1/P2/P3** | Replaces `[ESTIMATE]` consts with `[MEASURED]` break-evens; without it, auto-selects can regress near the boundary | M | Low |

**Sequencing note:** P0–P3 are first-class-ification of existing gates (highest ROI, lowest risk, ship first); **each ships its threshold as a provisional `[ESTIMATE]` const and is gated on P10 for the calibrated value** — P10 is therefore promoted into the early-ship critical path, not a tail. P1 (`SceneStats`) is the enabling substrate for P2/P3. **P4 is scoped to nothing-until-a-consumer-exists** (demand-driven), so it carries no speculative hot-path cost. **P5/P6 are render-architecture changes, not mechanism work — the P0–P4 mechanism stands alone and is NOT gated on them** (folds critique-2 P5/P6-overreach concern); they are sequenced last and escalated as owner calls.

---

## Part 6 — GI as a Worked Example (illustrative — deliberately NOT on the roadmap)

> **This section is ILLUSTRATIVE, not a design commitment (folds both critiques' Part-6 YAGNI concern).** GI is absent from P0–P10 by intent. Its purpose here is to show how the mechanism *would* apply when GI is scoped — admitting strategies **one at a time per measured situation**, exactly as Part 1.3 forbids doing speculatively. No GI strategy is admitted by this document.

When GI is scoped, it is **a strategy family over ONE Resource-owned probe column, NOT three subsystems.** The discipline: admit ONE baker first (the measured default for the target scenes), add a second only when a real situation flips the winner. The storage is a single `Resource`-owned `DeviceColumn` (DeviceLocal SSBO via `GpuColumnManager`, Principle-0) — the candidate bakers (baked irradiance probes / DDGI / SDF-VCT / SSGI / lightmaps) are *updaters over the same layout*, selected by a cold scene-load policy, with the per-pixel resolve staying a single trilinear fetch regardless of which baker ran.

The **one genuinely SDF-perf-justified point worth keeping** (`[DERIVED]`): an SDF voxel-cone-traced baker needs **no separate voxelization pass — the field IS the cone-trace medium**, a structural saving unique to the SDF half; it is the GI strategy that wins specifically when the scene is SDF-dense + glossy. The hybrid requirement, when GI is built: the baker must gather from **both** representations (march `field_distance` for the SDF half, gather the mesh G-buffer / P5 raster for the mesh half) so a mesh wall bounces onto an SDF surface and vice-versa — the same boundary-crossing requirement as P6 shadows. Caching-by-change (the M3 incremental dirty-brick / VSM static-dynamic-page idiom) re-bakes only invalidated probes. **End of illustration — nothing here is admitted; GI enters the roadmap only as a separate scoped proposal.**

---

## Part 7 — Risks + Owner Values Calls

**Genuine values/scope decisions — escalated, NOT silently decided:**

1. **Mesh as a first-class G-buffer citizen (P5).** Largest architectural change and largest absolute perf win; shifts primary visibility from "SDF marches everything" to "raster owns mesh tiles, SDF owns SDF tiles." **Precise contract impact (per 3.1):** P5 adds a NEW raster PSO and re-domains the marcher dispatch; it leaves `sdf_field.hlsli` and the marcher field math byte-frozen. Whether a tiled dispatch is contract-compatible depends on whether the marcher reads invocation-ID against a full-screen extent (compatible) or bakes the extent internally (requires a record-time superset variant) — to be verified at `.spv`-pin rigor. **Owner call:** is the "max perf" mandate sufficient to rework primary visibility under this precise, bounded contract impact?

2. **Hybrid shadows scope (P6).** Closing the "mesh casts no shadow / single caster" gap is correct but highest-risk (new map pass + cache invalidation + multi-light range-gating). **Owner call:** ship the cheap half first (extend SDF march to N range-culled lights) and defer the rasterized-mesh-shadow-map MIN, or do both together?

3. **GI default baker (Part 6, only if/when GI is scoped).** Which baker is the *first* ship — baked probes (mostly-static, most situations) vs SSGI (cheapest, no new storage)? A quality/scope fork, not a perf fork (both are perf-justified for their situation). **Not on the current roadmap.**

4. **`StrategyClass` bit budget (P4) — RESOLVED to demand-driven, escalation withdrawn.** The draft's "widen to u32" option is **rejected, not deferred**: it edits a `#[repr(transparent)] u16` under a `const size_of==2` assert and re-touches every structural-fire immediate for zero present benefit. P4 claims ONE of bits 12-15 only when a consumer exists. No owner call needed unless/until ≥4 simultaneous mint-time strategy classes are justified (not foreseen).

5. **GPU large-N physics threshold (P-future).** The CPU↔GPU crossover for particle/soft/fluid is **not** the PhysX-folklore ~10⁴; it depends on `GpuColumnManager` transfer cost and **must be measured** for boyko's path. **Owner call:** confirm this stays future-roadmap (rigid stays CPU — already a values call, honored).

**Architectural (decided, not escalated):** all selection stays ECS-native (marker/bit/scalar/Resource); no `dyn` registry; **no external dispatch crate** (hand-rolled `#[repr(u8)]` `match` only); selectors are cold or wave-uniform; the per-archetype carrier is **mint-time-only** (no live mutation of the lockless word); GPU per-strategy pipelines are an explicit record-time superset-binding seam under `DispatcherToken`; the 0%-gate is preserved on every new gate; `SceneStats` has single-owner-per-field writes read only post-gather; temporal selectors carry hysteresis with band-state in `SceneStats.selector_bands` and preallocated transition buffers.

**Open question for measurement (not a values call):** the exact crossover thresholds (cluster light-count, grid body-count, large-island constraint-count, parallel break-even, GPU-N) are `[ESTIMATE]` — **P10 calibrates them, and P0–P3 are HARD-gated on it** for their final consts; until then they ship as provisional consts and the runtime selector stays a const/compare (only the *constant* is empirical, never the code path).

---

## Critique resolutions

| # | Blocker / concern | Lens | Resolution in this final |
|---|---|---|---|
| B1 | P4 `StrategyTag` admitted as a NEW primitive with no named situation/consumer (self-violates Part 1.3) | Perf/YAGNI | **Demoted to demand-driven (Primitive C / P4):** no bits reserved, u32-widen **rejected outright**, ONE bit claimed only when a concrete CPU consumer + measured crossover lands (first candidate named: per-archetype static-broadphase eligibility). Sequenced after P0–P3; carries no speculative hot-path cost. |
| B2 | Perf claims mix real and hand-waved numbers with equal authority; auto-selects on uncalibrated consts | Perf | **All numbers tagged `[MEASURED]`/`[DERIVED]`/`[ESTIMATE]`.** P5 raster-vs-march given a threshold-free `[DERIVED]` *direction* (raster O(1)-in-edits beats march O(steps×edits)). **P10 promoted to a HARD dependency** of P1/P2/P3; the 0%-gate's failure mode for two-ON-strategy selection made explicit (Part 1.4). |
| B3 | `SceneStats` multi-writer `ResMut` access pattern unspecified — scheduler conflict/serialize/race | Principle 0/4 | **Write discipline added (Part 2.2):** single owning producer per field (no write-write conflict), all writes in the gather phase, policies read `Res` (not `ResMut`) strictly after a gather sync point the scheduler already has → no concurrent writer. |
| B-C | Primitive C runtime per-archetype strategy races the lockless `ArchetypeFlags` read; u32-widen touches the size_of assert | Soundness | **Constrained to mint-time-only** (Part 1.2 + 2.2): static function of the signature, OR-computed once like `GPU_RESIDENT`, never mutated live. Runtime-varying facts routed to C3/cold-side-field. u32-widen rejected. |
| B-GPU | Per-strategy GPU pipeline asserted as already-supported by the column key | Buildability | **Demoted to an explicit NEW record-time seam (2.3 rule 3):** distinct PSO under `DispatcherToken` on the `!Send` thread, superset binding to preserve frozen `.spv`, column key addresses data only. Tied to P5/P6 risk tier. |
| c1 | Part 6 GI = speculative five-strategy subsystem, contradicts own YAGNI | Both | **Recast as explicitly illustrative, NOT a commitment;** GI stays off P0–P10; only the `[DERIVED]` no-voxelization VCT point kept; "admit one at a time" stated. |
| c2 | Hysteresis mandated but transition cost unbudgeted; no carrier for band-state | Perf | **Transition cost budgeted (2.4):** flippable strategies preallocate worst-case buffers (Principle 5) → flip is a fill; **band-state carrier named** = `SceneStats.selector_bands`. P1/P3 carry explicit transition notes. |
| c3 | `enum_dispatch` external crate endorsed against in-house-only | Both | **Removed;** replaced by the hand-written `#[repr(u8)]` `match` (the existing `RigidSolver` seam); external dispatch crates explicitly rejected (2.5, Part 7). |
| c4 | P5 contract impact self-contradictory ("preserved — but composite changes") | Perf | **Precise artifact list (3.1 + Owner-call #1):** NEW raster PSO + re-domained marcher dispatch; `sdf_field.hlsli`/field math stay frozen; tiled-dispatch compatibility conditioned on the marcher's invocation-ID assumption, to be `.spv`-pin-verified. |
| c5 | Thresholds-as-const tension with max-perf-any-situation | Soundness | Stated robust across the single target band (AVX2 x86_64); widens to a cold scene-load re-pick if the band widens; never hot-path-recomputed (2.5). |
| c6 | P5/P6 overreach into a rewrite under the multi-strategy banner | Principle 0 | **Mechanism (P0–P4) explicitly stands alone and is NOT gated on P5/P6** (Part 5 sequencing note); P5/P6 labelled render-architecture, escalated as owner calls. |
| c7 | `coarse_enabled` disabled-line not verified | Buildability | **Verified and cited:** `swapchain.rs:3575` hardcodes `coarse_enabled = 0` on the windowed path (descriptor bound at the offsets near 2704/3694/3730/4166 → OFF path byte-identical). P0 references the exact line. |
| base | Factual base (ArchetypeFlags repr/bits, ResidencyKind/StorageKind, gate words) | Both | Confirmed correct; bits 12-15 free (the "4 free bits" claim holds); mechanism is genuinely ECS-native and pulls in no external lib. |
