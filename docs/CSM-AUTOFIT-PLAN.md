# Architecture: CSM caster-aware far-plane fit (`CsmFitMode` knob)

**Status:** final, and SHIPPED (rungs C0–C6). Supersedes the three candidate passes. Every critic finding is either fixed or explicitly refuted below (§0).

> ## ⚠️ POST-SHIP AMENDMENT (2026-07-16) — the default is now `CatchAll`, not `Fixed`
>
> This document is written throughout on the premise that the default is `CsmFitMode::Fixed`, which
> is what rungs C0–C5 shipped and what made every rung golden-free. **After the rung-C6 eval the
> owner changed that call: `CsmConfig::default().fit_mode` is now `CatchAll`.** Read every
> "default `Fixed`" / "no golden moves because the default is `Fixed`" statement below as historical.
>
> What the eval measured on `examples/room.rs`'s scene (the shipped config: `cascade_count: 3`,
> `shadow_distance: 30`, `resolution: 2048`, λ 0.8), taken from `resolve_csm`'s own output:
>
> | | splits | texels |
> |---|---|---|
> | `Fixed` | 2.55 / 7.59 / 30.00 | 0.0034 / 0.0093 / **0.0366** |
> | `CatchAll` | 1.67 / 8.98 / 30.00 | 0.0024 / **0.0112** / 0.0361 |
>
> A receiver at view-depth ~8 falls off `Fixed`'s cascade-1 edge into the 22-unit tail and its
> 0.0366 texel (~3.6 screen px at 900p); `CatchAll` keeps it in cascade 1 at 0.0112 — 3.2×, which
> narrows the 13-tap PCF penumbra by the same factor because that tent is measured in TEXELS. A
> receiver at ~6 gets ~20% coarser (0.0093 → 0.0112). So the mode redistributes sharpness toward
> the casters rather than adding any — but it costs nothing to do so, which is why paying `Fixed`'s
> blur by default bought nothing.
>
> **§11's "MOVES A GOLDEN: none" still holds, and that is itself a finding, not a reassurance:**
> flipping the default moved ZERO of the five frozen goldens, because none of them exercises CSM +
> casters through `CsmConfig` (`grand_showcase` mirrors the cascade math by hand at the RHI level;
> the VB scenes have no shadow receivers). The default change is therefore **ungated by byte
> goldens** — the verification was the C6 eval render plus `room_smoke_catch_all_fit`. This is the
> same gap `docs/`'s host-path lesson already records: host render rungs need a golden-independent
> visual regression.
>
> Open Question 1 below (the 13-tap PCF tent) is **not** superseded and is now the strongest
> remaining lever: the tent and the fit compound multiplicatively.

---

## 0. Disposition of the critics' findings

| # | Finding | Verdict |
|---|---|---|
| A | Shimmer proof covered caster motion only; camera motion crosses grid cells | **VALID — fixed** by Decision 6: the proof obligation is restated as two *testable* properties (S1 exact-stability at rest, S2 bounded pop magnitude + bounded pop rate under motion), plus an asymmetric Schmitt latch. A camera-invariant fit is **impossible** (the split range is a view-space depth; the camera moves) — so the obligation is *discharged by bounding*, not by claiming invariance. |
| B | "measure-zero dither" is false; latch/hysteresis is in scope | **VALID — fixed.** Schmitt latch is in scope (Decision 6), not v1.1. |
| C | Caster-set change (spawn/despawn/stream) pops globally; `!has_casters → SENTINEL` strobes | **VALID — fixed** by Decision 7 (hold-the-latch; never reset once latched; incomplete bounds ⇒ hold). |
| D | Union AABB conflates lateral extent with depth | **VALID — fixed** by Decision 4: the fold produces a **view-space depth scalar** reduced per *instance*, not a projection of a union AABB. The world AABB is kept **only** for the sun-axis (Z) term, where a union bound is correct. |
| E | `CasterShrink` shrinks `z_far`/pull-back ⇒ up-sun casters clipped ⇒ shadows vanish | **VALID — fixed** by Decision 5 (`pull_back = clamp(up_need_q, diameter, 4·diameter)`), which is byte-identical to today under `Fixed` and strictly *better* than today under the caster modes. |
| F | `Shrink` is dominated by `CatchAll` | **REFUTED with numbers** (§2 table): they are not identical (`Shrink` gives N cascades to the caster range, `CatchAll` gives N−1). At casters `[0.5,2.5]`, receiver @0.5: `Shrink` 2.74 px vs `CatchAll` 1.83 px — different, and the winner is scene-dependent. The critic only checked the deepest cascade, where they tie by construction (both have `df = far_eff`). |
| G | Headline gain computed at a config no scene uses (N=4/sd=200) | **VALID — fixed.** §2 recomputes at the *shipped* config (N=3, sd=30, res 2048, λ=0.8, fov 60°, near 0.1) from the verified code. |
| H | `Shrink`'s hard, un-cross-faded terminator moves into the visible scene | **VALID — accepted and documented** as `Shrink`'s defining trade-off, pinned by test T12, gated behind a non-default mode, and settled by an owner-eval rung. `CatchAll` is the recommended ON mode. |
| I | `resolve_csm_cascades` taking `Res<CsmCasterScratch>` panics under a bare `CsmPlugin` | **VALID — avoided.** The fit reads its own `CsmCasterBounds` Resource, which `CsmPlugin` inserts. |
| J | `CsmCasterBounds.caster_count` becomes a second caster predicate → drift vs `sync_csm_light_gate` | **VALID — fixed** by Decision 7: `CsmCasterBounds` carries **counters, not a predicate**, is documented as *not* a caster-presence authority, `sync_csm_light_gate` is untouched, and T14 pins the gate's behavior identical across all three modes. |
| K | Test T15 / `rotation_invariance_holds_in_casters_mode` contradict their own designs | **VALID — those tests do not appear here.** T2/T3/T4 pin properties that actually hold. |
| L | `_pad` → `cascade0_near` repurposing | **REJECTED** (Decision 1). All three candidates converged; the shader evidence is decisive. |
| M | The 13-tap PCF tent, not the fit, is the dominant softness | **PROBABLY VALID — out of scope, escalated** (§11 Open Question 1). It is a *sibling* rung, and the two fixes compound (the tent is measured in texels). |

---

## 1. Goal

Fit the cascade partition to the depth range that actually contains casters, exposed as an owner-set `CsmFitMode` knob with **both** requested policies (`Shrink`, `CatchAll`) plus the byte-identical `Fixed` default.

**The mechanism, stated precisely** (this is what none of the candidates got right, and it explains the owner's "~2×2 screen pixels" exactly):

> A cascade's world texel is `texel = ceil(2r)/resolution` with `r ≈ df·1.1776` at fov 60°/16:9 (derived from `slice_corners`+`sphere_radius`, csm_config.rs:471-510). A screen pixel at depth `d` is `2·d·tan(fov/2)/H`. So the shadow texel measured **in screen pixels** is ≈ `1.1 · df/d`: it is ~1.1 px at a cascade's **far** end and degrades by the cascade's **depth ratio** `df/dn` toward its near end.
>
> At the shipped config, `Fixed`'s cascade 0 spans `[0.1, 2.549]` — **ratio 25.5**. A close-up receiver at depth 0.5 therefore gets a **6.4 screen-px** texel. That is the defect. The fix is to compress cascade 0's depth ratio, which is exactly what fitting the range to the casters does.

**Cost:** 0 allocations/frame · 0 shader edits · 0 SPIR-V rebuilds · 0 GPU-layout changes · 0 goldens moved · ≤15 µs @ 10k caster instances (cold, once/frame) · `Fixed` = 0 ns (0%-gate).

---

## 2. Measured gain — recomputed at the SHIPPED config

Config verified in-tree: every scene uses `CsmConfig { cascade_count: 3, ..default() }` ⇒ `resolution: 2048` (csm_config.rs:77), `shadow_distance: 30.0` (:79), `lambda: 0.8` (:82), `MIN_DIAMETER: 1.0e-3` (:72 — **not** a meaningful floor; `ceil` is the quantizer). Camera fov_y 60°, aspect 16:9, near 0.1, 1080p.

`Fixed` splits of `[0.1, 30]`, N=3, λ=0.8: **2.549 / 7.592 / 30**.

| Scene | Receiver depth | `Fixed` | `Shrink` | `CatchAll` |
|---|---|---|---|---|
| casters `[0.5, 2.5]` → `far_eff = 2.828` | **0.5** | diam 7, 3.42 mm, **6.39 px** | diam 3, 1.47 mm, **2.74 px** (2.3×) | diam 2, 0.98 mm, **1.83 px** (**3.5×**) |
| casters `[0.5, 2.5]` | 2.0 | diam 7, **1.60 px** | diam 7, 1.60 px (1.0×) | diam 7, 1.60 px (1.0×) |
| casters `[2, 6]` → `far_eff = 6.727` | 2.6 | diam 19, **3.34 px** | diam 17, 2.99 px (1.12×) | diam 17, 2.99 px (1.12×) |
| casters `[2, 6]` | 6.0 | diam 19, **1.45 px** | diam 17, 1.29 px | diam 17, 1.29 px |

**Honest reading — state this to the owner verbatim:**
- The win is **concentrated at the near end of cascade 0** and is large (**2.3–3.5×**) exactly in the close-up scene the owner reported.
- It is **~1.1×, i.e. invisible**, when the caster set already spans a wide log range (`[2,6]`).
- `Shrink` and `CatchAll` **tie** for receivers in the last caster cascade (both have `df = far_eff` there) and differ only in how the sub-range is subdivided. `CatchAll`'s N−1 split of a short range often beats `Shrink`'s N split, because λ=0.8's uniform term makes N splits of a *short* range near-uniform, wasting cascade 0. **This is a real, measured difference — the knob is not a no-op — but it is a sharpness-distribution/coverage lever, not a quality/perf lever.** The quality/perf levers remain `resolution` and `cascade_count`, which already exist on `CsmConfig`.
- **Go/no-go gate (rung C6):** if the owner's live scene has casters spanning a log range ≳ 8, the fit buys < 1.2× and the rung should be closed in favour of Open Question 1 (the PCF tent) or `cascade_count: 4`.

---

## 3. Key decisions

### D1 — Far-plane only. `near` never moves. REJECT `cascade0_near` / any `_pad` repurposing.
`near` stays `view.near` (csm_config.rs:314) in all modes. **Why:** `shadow_apply.hlsli:259` hardcodes `float prev_split = 0.0;` — the shader *assumes* cascade 0 starts at view-z 0. Moving cascade 0's near to `near_eff` makes a fragment at depth 0.5 still select cascade 0 (selection is `sel = Σ step(split_far[c], view_z)`, :260-265) and sample **outside** the fitted ortho box → garbage on the closest, most visible pixels. Fixing it needs a `split_near` word in **five** independent shader redeclarations + the eDSL that owns the marcher, a `CascadeData`/`ResolvedCsm` layout change, and a golden re-bless — to buy ≤8% on one cascade (the far face dominates the bounding sphere: `r=7.57` for `[0.1,5.93]` vs `7.05` for `[4.0,5.93]`).
**Consequence:** `size_of::<ResolvedCsm>() == 336` (csm_config.rs:212) and `size_of::<CascadeData>() == 80` (:174) hold untouched; `_pad` stays pad; `RESOLVED_CSM_BYTES` (:218), every host ring slot, and every `.spv` stay frozen.
*Record for the future:* `prev_split = 0.0` is a latent bug that is only dormant because cascade 0 starts at `view.near`. Any future near-side fit must ship `split_near` first.

### D2 — Far-only shrink needs ZERO shader change.
`shadow_apply.hlsli:268-270`: `if (sel >= gCsmActive) { return 1.0; }` — a receiver beyond the last `split_far` is **fully lit**. That IS `Shrink`'s documented trade-off, delivered for free, and it is a lit/unlit decision, not an out-of-box sample. This single fact makes the whole feature a pure-CPU change.
**Cost, stated loudly:** `shadow_apply.hlsli:282` (`has_next = (sel+1u < gCsmActive)`) means the **last cascade never cross-fades**. Under `Shrink` the hard lit-terminator relocates from `shadow_distance` (30 m, far, faint) to `far_eff` (~2.8 m in the close-up scene) — a **hard, un-blended lit/shadow line a few metres ahead that jumps up to 29.3% on each latch transition**. This is `Shrink`'s defining artifact. `CatchAll` has no terminator (its last split is `far_cap`, exactly as today). **`CatchAll` is the recommended ON mode.**

### D3 — The knob is a field on `CsmConfig`, following the house idiom exactly.
`CsmConfig.fit_mode: CsmFitMode`, `#[repr(u32)]`, `#[default] Fixed`. Mirrors `ShadowDenoiseConfig { mode, .. }` (shadow_denoise_config.rs:56-72) and `AaConfig`/`AaMode`: mode + tuning params on **one** Resource, `#[default]` variant IS the 0%-gate. A sibling Resource would fragment the policy and add a second read surface to the single writer at csm_config.rs:535. Two bools would encode an invalid 4th state (capability is structural).
`CsmConfig` is CPU-only, not `#[repr(C)]`, not size-pinned; the field is free. Verified: **every** in-tree literal uses `..CsmConfig::default()` (room.rs:29, showcase.rs:31, viewer.rs:50, bounce.rs:43, sdf_room.rs:34, punctual_room.rs:33, paradigm_lab.rs:77, shadow_denoise_eval.rs:65, room_smoke.rs:126, sdf_room_smoke.rs:112, pbr_material_showcase.rs:325, book/src/app/windowed-host.md ×3) ⇒ non-breaking.

### D4 — The caster **depth** statistic is a per-instance reduction, NOT a projection of a union AABB.
`raw_far = max over instances of (view-depth of that instance's own world AABB max corner)`.
**Why (refutes the union-AABB error):** projecting a *union* AABB conflates lateral with depth extent — two casters at depth 3, at world x = ±50, give the union `h.x = 50`, and with `|fwd.x| = 0.5` that inflates `far_eff` from 3 to 28, silently killing the feature. Reducing **per instance** (each instance's own tight AABB, projected, then `max`) is exact up to each instance's own abs-matrix bound and costs the same pass.
**The world union AABB is still folded — but used ONLY for the sun-axis term** (D5), where a union bound is exactly what is wanted.
Per-instance world AABB via the abs-matrix (Arvo) transform on `InstanceModelCol.rows` (3×4 row-major, instance_model.rs:61):
`wc[r] = Σⱼ rows[r][j]·lc[j] + rows[r][3]`, `wh[r] = Σⱼ |rows[r][j]|·lh[j]`.
Strictly dominates a bounding sphere: 9 mul + 6 add, **no sqrt**; no √3 circumscription loss; and `|A|·h` is the **exact** AABB of the transformed box for *any* linear `A` **including shear** — the sphere route via max-column-norm **under**estimates under hierarchical non-uniform-scale shear (a real soundness hole, since `GlobalTransform` composition can shear).

### D5 — The light-space Z pull-back is derived from the caster bounds. (Fixes the vanishing-shadow regression.)
csm_config.rs:386-387 today: `z_far = 2.0*diameter; eye = center + sun*(z_far*0.5)` ⇒ **up-sun caster capture = `diameter`**. The fit's entire point is to shrink `diameter` (7→2 in the close-up scene) — which would shrink up-sun capture from 7 m to 2 m and make a caster 2.5 m up-sun (a ceiling beam, a tall pillar) **vanish**. That is a correctness regression, not a trade.

Replace lines 386-387 with:
```
pull_back = clamp(up_need_q, diameter, MAX_PULLBACK_RATIO * diameter)   // ≥ diameter ALWAYS
z_far     = pull_back + diameter                                        // down-sun capture = diameter, as today
eye       = center + sun * pull_back
```
- `up_need_q` = `grid_ceil(max over the 8 world caster-AABB corners of sun·(corner − center))`, or `diameter` when there are no bounds.
- **Under `Fixed` / unlatched:** `up_need_q = diameter` ⇒ `pull_back = diameter`, `z_far = 2·diameter`, `eye = center + sun*(z_far*0.5)` — **byte-identical to today**. Pinned by T1.
- Under the caster modes it only ever **grows** the capture volume ⇒ strictly ≥ today. Never worse.
- **This touches Z only.** `eye` moves along `sun`; the ortho basis is `light_right`/`light_up` ⊥ `sun`; the texel snap (:377-381) is in right/up. **The ortho XY extent and the snap grid are provably untouched — no shimmer channel exists here.** A caster outside the XY box projects its shadow outside the box, so XY must *not* grow (that is the Microsoft trap); only Z must.
- `MAX_PULLBACK_RATIO = 4.0` bounds `z_far` to 5·diameter (vs 2·diameter today), bounding depth-precision / bias-scale drift. `z_far` already varies per cascade today (it is `2·diameter` and `diameter` differs per cascade) while the rasterizer bias is pass-constant, so this is within the existing tolerance band. Acne is checked in the owner-eval rung C6.

### D6 — Anti-shimmer: log grid + asymmetric Schmitt latch, libm-free. The obligation restated as two pinnable properties.
**The honest chain:** `far_eff → pssm_split(near, far_eff, λ, i, N) → (dn, df) → slice_corners(rig) → r → diameter=ceil(2r) → texel_size`. `rig` depends on the camera, so `far_eff` is *necessarily* camera-dependent. **A camera-invariant caster fit does not exist** — the split range is a view-space depth and the camera moves. Therefore the obligation is discharged by **bounding**, and it is bounded on three axes:

- **Grid** — `far_eff ∈ {2^(k/4)}` (ratio 1.18921), `k = grid_cell(raw)+1` ⇒ `far_eff ≥ raw` always (**never clips**), waste ≤18.9%. Without a grid, `raw_far` moves continuously ⇒ `2r` crosses an integer `ceil` bucket every frame or two ⇒ `texel_size` flickers ⇒ **the snap grid itself re-phases every frame** = continuous boiling. The grid converts per-frame-small into rare-and-bounded. Shimmer is a *temporal* artifact: rare-and-bounded wins.
- **Schmitt latch, asymmetric — and the asymmetry is principled:**
  - **Grow (camera receding / caster retreating): immediate** at the cell boundary (every 18.9%). It must be immediate or `Shrink` clips. It is also the *masked* direction: the shadow is shrinking in screen space and the camera is moving away.
  - **Shrink (camera approaching / caster nearing): only after 2 full cells (41.4%).** This is the *scrutinised* direction (the shadow is growing on screen), so pops are made rare there. Approaching from 2.5 m to 1.0 m ⇒ **~2 pops**, not ~5/s.
  - Pop magnitude: grow = +18.9% of `far_eff`; shrink = −29.3% (2 cells). Bound: `|Δ log far_eff| ≤ |Δ log raw_far| + 1 cell` — the latch never overshoots the input's own motion by more than one cell. (Candidate 3 mis-stated the shrink magnitude as 18.9%; it is 29.3%.)
  - **No limit cycle, proved:** after growing `k → k+1`, `raw ≈ grid_value(k) > grid_value(k−1) = grid_value((k+1)−2)`, so the shrink predicate is false. Any oscillation of amplitude < 18.9% (grow side) / < 41.4% (shrink side) **never re-quantizes**.
- **Libm-free, bit-exact grid.** `grid_value(k) = TABLE[k.rem_euclid(4)] · 2^(k.div_euclid(4))` is exact (mantissa × exact power of two); `grid_cell` recovers `(exp, phase)` from the IEEE bits with 3 branchless compares against the same TABLE thresholds ⇒ `grid_cell(grid_value(k)) == k` and `grid_value(grid_cell(x)) ≤ x < grid_value(grid_cell(x)+1)` hold **exactly on every platform**. (`pssm_split` already uses `powf` at :448, so this is not about goldens — `Fixed` never calls the grid at all. It is about the round-trip being an *exact* property a test can pin, which `powf(1.18921, k)` cannot give.)

**The two properties a test pins — this IS the discharged obligation:**
> **S1 (rest ⇒ exact):** with a static camera and static casters, `resolve_csm` is bit-identical frame to frame. Shadows are *exactly* as rock-solid as today. Pinned by **T2** with `assert_eq!` on the whole `ResolvedCsm`.
> **S2 (motion ⇒ bounded):** `texel_size` changes ONLY at a latch transition. A dolly over a raw-far ratio `R` produces at most `ceil(log_1.18921 R)` transitions when receding and `ceil(log_1.41421 R)` when approaching, and each transition changes `far_eff` by ≤ +18.92% / ≤ −29.3%. Pinned by **T3** (a camera sweep that *counts* transitions and asserts both the count bound and the per-transition magnitude bound) — not by a tautological jitter test.

**What S2 costs, honestly:** today `texel_size` is invariant under camera motion; under the caster modes it steps a handful of times during locomotion, and each step shifts every shadow edge in that cascade by ≤1 texel (≤ ~1-2 screen px), *at a moment when the camera is already moving* (the step can only be caused by that motion). The alternative is a permanently 6.4-screen-px blocky shadow. This is the trade the knob exists to let the owner make, and rung C6 puts it in front of the owner's eye **in motion** (the `shadow_lag_dump` / in-motion discipline).

### D7 — Bounds authority, streaming, and the blink strobe.
- `CsmCasterBounds` is folded from **`CsmCasterScratch`'s existing `batches()` + `ring()`** (csm_caster.rs:106,115) — the output `gather_shadow_casters` (:169) already produced. **No second query.** A second `Query<..., With<ShadowCaster>>` would re-derive the same set from a second authority — the drift surface the ground truth prohibits.
- **Not folded into `gather_mixed_into`** (:182): that core is shared verbatim with the main `gather_mesh_draws`, and its closure is invoked **twice** (count + scatter) ⇒ an in-closure fold would double-count without interior mutability.
- **A separate Resource, not a field on `CsmCasterScratch`:** `gather_shadow_casters` is the single writer of the scratch (one-producer-per-field, csm_config.rs:191-193); more decisively, `CsmPlugin` deliberately does **not** insert the scratch (the app does, plugins.rs:284) but **must** insert the fit's input — otherwise a bare-`CsmPlugin` world panics in `Res::get_param` (`missing_resource_panic`, res.rs:128-130). `CsmPlugin` inserts `CsmCasterBounds::EMPTY` + `CsmFitState::UNLATCHED`; without the app-wired reducer the fit stays unlatched ⇒ `Fixed` ⇒ today's picture.
- **`CsmCasterBounds` carries counters, NOT a predicate.** `sync_csm_light_gate`'s `csm_mode_word == 1 && batch_count() > 0` (csm_caster.rs:258) remains the **sole** caster-presence authority and is **not touched**. T14 pins the gate byte-identical across all three modes. The doc comment states this contract explicitly.
- **Streaming + blink, fixed:** `reduce_bounds_into` skips a batch whose mesh is not `Loaded` (`try_get` → `None` — the F6 never-deref invariant, csm_caster.rs:186-191). So bounds can be **incomplete** (`resolved_batches < total_batches`) or **empty**. Rule:
  - `far_k == UNLATCHED` **and** bounds are complete + non-empty ⇒ latch fresh.
  - `far_k == UNLATCHED` otherwise ⇒ **`Fixed` fallback** (satisfies ground-truth constraint 3 exactly: a world that never has casters never latches).
  - `far_k != UNLATCHED` **and** bounds are empty or incomplete ⇒ **HOLD the latch** (reuse the previous `far_eff`). Never reset.
  
  This kills candidate 3's 36%-diameter-strobe: a blinking `RenderEnabled` caster or a mesh streaming in/out can no longer alternate `Fixed`↔fitted per frame. The value **and** the predicate gating it are both debounced. Pinned by T5/T6.

### D8 — `MeshGpu` gains a model-space AABB.
Verified: **no mesh bounds exist anywhere** in `boyko_render`; `MeshGpu` (mesh.rs:137) carries only `vertex_buffer, index_buffer, index_count, index_type, vertex_count, blas, geometry_slot`; the caster ring holds **affines only** (translations alone are unusable — a 20 m plane at the origin would report a point). `MeshGpu` is the correct owner, with exact precedent on the same struct: `blas` (mesh.rs:148) is documented *"durable per-mesh data ON the record (Principle 0: NOT a parallel `Vec<BuiltBlas>`)"*. `build_mesh_gpu` (mesh_assets.rs:216, literal at :405) is the **single** choke point and the only place vertices are in scope; the pass is O(V) at **setup**. `MeshGpu` is not `#[repr(C)]` and has **no size pin** ⇒ +24 B is free. Blast radius: **3** construction sites (mesh_assets.rs:405 + 2 test dummies).
Rejected: a side `HashMap<MeshHandle, Aabb>` (Principle 0 violation); per-frame bounds from vertex buffers (a device read in the frame loop).

### D9 — Grid ratio, margin and pull-back ratio stay private `const`s.
They are **anti-shimmer / correctness parameters**, not quality levers. Exposing the grid lets an owner set ratio 1.001 and reintroduce exactly the shimmer texel-snap exists to kill. An owner-facing knob must not be able to express a broken engine.

### D10 — Default `Fixed` ⇒ goldens frozen. **Checkable claim:** `resolve_csm(cfg, view, sun, CsmFit::NONE)` is *textually* today's code path — `far_cap` reduces to `view.far.min(cfg.shadow_distance).max(near + MIN_DIAMETER)` (:318), the loop calls `pssm_split(near, far_cap, λ, i+1, n)` (:358), and `pull_back = diameter` ⇒ `z_far = 2·diameter`, `eye = center + sun*(z_far*0.5)` (:386-387). T1 pins it by `assert_eq!` and **every existing test at csm_config.rs:621-884 must pass with only a mechanical `, CsmFit::NONE` argument added.** If any CSM golden's bytes move, T1 is lying — that is a build-breaking bug, not a re-bless.

### D11 — Ordering via `SystemSet` (verified expressible cross-plugin).
`SystemKey` is a per-builder descriptor index (csm_plugin.rs:29-36) ⇒ `.after(key)` is not cross-closure. **Sets are:** `App::add_systems_cfg` passes `self.builder` — one single Main builder — to *every* closure (**verified**, app.rs:313-319), and `configure_set` interns by value (**verified**, schedule_builder.rs:206-212, and :201-205 documents that even a memberless set gets an id so the edge resolves). So a set edge declared in the app resolves against membership declared in `CsmPlugin`. Declaring `configure_set` in `plugins.rs` (which registers **both**) rather than in `CsmPlugin` avoids the memberless-set **W1501** warning in a bare-`CsmPlugin` world.

---

## 4. Data structures

```rust
// ═══ crates/boyko_render/src/csm_config.rs ═══════════════════════════════════

/// The cascade split-RANGE policy — the owner's sharpness/coverage lever. Capability is
/// STRUCTURAL: `Fixed` IS "auto-fit off" (the 0%-gate); there is no separate flag.
/// Mirrors `ShadowDenoiseMode` (shadow_denoise_config.rs:56-72) / `AaMode`.
///
/// The caster modes require `reduce_caster_bounds` to be app-registered (see
/// `csm_caster.rs`). Without it the fit never latches and EVERY mode renders as `Fixed`.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CsmFitMode {
    /// Split `[view.near, min(view.far, shadow_distance)]` — TODAY's math, BIT-IDENTICAL.
    /// The DEFAULT: a world that never sets this renders byte-identically to today, and
    /// the caster bounds are never read (0 ns).
    #[default]
    Fixed,
    /// F-shrink: all N cascades split `[view.near, far_eff]`. One extra cascade over the
    /// caster range; receivers beyond `far_eff` are FULLY LIT with **no cross-fade**
    /// (shadow_apply.hlsli:268-270 + :282) — a HARD terminator that relocates into the
    /// visible scene and jumps up to 29.3% per latch transition. Max sub-range
    /// subdivision, accept the terminator.
    Shrink,
    /// F-catch-all: cascades `0..N-2` split `[view.near, far_eff]`; cascade `N-1` is
    /// RESERVED for `[far_eff, far_cap]`, so distant shadows survive and the last split
    /// stays at `shadow_distance` exactly as today (no terminator). One of N cascades is
    /// spent on caster-free range. `cascade_count < 2` degenerates to `Fixed`.
    /// RECOMMENDED ON-mode.
    CatchAll,
}

/// `CsmConfig` (csm_config.rs:99) gains ONE field. CPU-only (`ResolvedCsm` is the GPU
/// carrier): not `repr(C)`, not size-pinned, not uploaded — the field is free.
pub struct CsmConfig {
    // ... 7 existing fields unchanged ...
    /// The cascade split-RANGE policy. Default `Fixed` (the golden-preserving 0%-gate).
    pub fit_mode: CsmFitMode,
}
// Default (csm_config.rs:126) gains: `fit_mode: CsmFitMode::Fixed,`

/// The caster-derived fit input, folded per frame by `reduce_caster_bounds` from
/// `CsmCasterScratch`'s batches+ring (the gather's OUTPUT — not a second query).
///
/// # NOT a caster-presence authority
/// `sync_csm_light_gate` (csm_caster.rs:258) remains the SINGLE predicate for "do we have
/// casters" (`csm_mode_word == 1 && batch_count() > 0`). The counters here exist only to
/// tell the fit whether this frame's fold is usable; a batch whose mesh is not yet
/// `Loaded` is SKIPPED (the F6 invariant), which makes the fold INCOMPLETE, not
/// caster-less. Never read these to decide whether shadows are on.
///
/// Not `repr(C)`, no size pin — never reaches the GPU. 36 B.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct CsmCasterBounds {
    /// Max VIEW-space depth over all caster instances, reduced PER INSTANCE (each
    /// instance's own world AABB projected on the view axis, then `max`). NOT the
    /// projection of a union AABB — that conflates lateral with depth extent.
    /// Valid iff `resolved_batches > 0`.
    pub raw_far: f32,
    /// World-space UNION AABB of all caster instances. Used ONLY for the sun-axis
    /// pull-back (`up_need`), where a union bound is exactly what is wanted.
    pub world_min: [f32; 3],
    pub world_max: [f32; 3],
    /// Batches whose mesh resolved via `try_get` and contributed.
    pub resolved_batches: u32,
    /// Batches the gather emitted. `resolved < total` ⇒ INCOMPLETE ⇒ the fit HOLDS.
    pub total_batches: u32,
}
impl CsmCasterBounds {
    pub const EMPTY: Self = Self {
        raw_far: 0.0, world_min: [0.0; 3], world_max: [0.0; 3],
        resolved_batches: 0, total_batches: 0,
    };
    /// This frame's fold is usable as a fit input.
    #[inline] pub fn is_usable(&self) -> bool {
        self.resolved_batches > 0 && self.resolved_batches == self.total_batches
    }
}
// Default == EMPTY.

/// The anti-shimmer hysteresis latch — CPU-ONLY state, deliberately NOT inside the
/// `repr(C)` GPU-uploaded `ResolvedCsm` (that would entangle `DISABLED`/`Default`/
/// `PartialEq` with frame state and break the 336 B contract's purity).
#[derive(Resource, Clone, Copy, Debug, PartialEq, Default)]
pub struct CsmFitState {
    /// The latched grid cell of `far_eff`. `UNLATCHED` ⇒ never latched ⇒ `Fixed`.
    pub far_k: i32,
}
impl CsmFitState { pub const UNLATCHED: i32 = i32::MIN; }
// Default::default() gives far_k == 0, which is a VALID cell — the ctor MUST be
// `CsmFitState { far_k: CsmFitState::UNLATCHED }`; `Default` is derived only for the
// derive-completeness of the Resource and is not used for insertion. (debug_assert in
// resolve_csm_cascades: inserted state is UNLATCHED on frame 0.)

/// The already-latched fit decision handed to the PURE `resolve_csm`. `NONE` == "Fixed /
/// unlatched / no usable bounds" == today's fit EXACTLY.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct CsmFit {
    /// The quantized, latched split far. `None` ⇒ `far_cap` (today).
    pub far_eff: Option<f32>,
    /// `true` ⇒ reserve cascade `N-1` for `[far_eff, far_cap]` (CatchAll).
    pub reserve_tail: bool,
    /// The world caster AABB for the sun-axis pull-back. `None` ⇒ `pull_back = diameter`
    /// (today, byte-identical).
    pub caster_aabb: Option<([f32; 3], [f32; 3])>,
}
impl CsmFit {
    pub const NONE: Self = Self { far_eff: None, reserve_tail: false, caster_aabb: None };
}

// ── private consts, beside MIN_DIAMETER (csm_config.rs:72) ──────────────────
/// Log2 step of the far grid: `far_eff ∈ {2^(k/4)}` (ratio ≈ 1.18921). The ANTI-SHIMMER
/// parameter, NOT a tuning knob (D9). 4 cells per octave ⇒ the grid is exactly
/// representable at every power of two.
const FIT_GRID_CELLS_PER_OCTAVE: i32 = 4;
/// Grid mantissa thresholds `2^(0/4..3/4)` — both the `grid_value` table and the
/// `grid_cell` compare thresholds, which is WHY the round-trip is exact.
const FIT_GRID_TABLE: [f32; 4] = [1.0, 1.189_207_1, 1.414_213_6, 1.681_792_8];
/// Schmitt band: shrink only after the raw far falls below `grid_value(k - 2)` (−29.3%).
/// Grow is immediate (never clip). Asymmetric on purpose (D6).
const FIT_SHRINK_BAND_CELLS: i32 = 2;
/// Caps the sun-axis pull-back at `4 × diameter` (⇒ `z_far ≤ 5 × diameter` vs `2 ×` today),
/// bounding depth-precision / bias-scale drift. Always `≥ diameter` ⇒ never worse than today.
const MAX_PULLBACK_RATIO: f32 = 4.0;

/// Ordering set: `resolve_csm_cascades`. The app pins `.after(CsmFitSet)` (D11).
pub struct CsmResolveSet;
impl SystemSet for CsmResolveSet {}

// ResolvedCsm (csm_config.rs:197) / CascadeData (:161): UNCHANGED. 336 B / 80 B pins hold.

// ═══ crates/boyko_render/src/csm_caster.rs ═══════════════════════════════════
/// Ordering set: `reduce_caster_bounds`.
pub struct CsmFitSet;
impl SystemSet for CsmFitSet {}

// ═══ crates/boyko_render/src/mesh.rs (MeshGpu, :137) ═════════════════════════
pub struct MeshGpu {
    // ... existing fields unchanged ...
    /// Model-space AABB min over `vertices[].position` (mesh.rs:85). Durable per-mesh data
    /// ON the record — the same shape of datum as `blas` (:148, "Principle 0: NOT a
    /// parallel `Vec<BuiltBlas>`"). Minted once in `build_mesh_gpu`; CPU-only, never uploaded.
    pub local_min: [f32; 3],
    /// Model-space AABB max.
    pub local_max: [f32; 3],
}
```

---

## 5. Public API

```rust
// ── csm_config.rs ────────────────────────────────────────────────────────────
/// (was :296) `#[inline]`, PURE. +1 param. `CsmFit::NONE` ⇒ today's fit, bit-identical.
pub fn resolve_csm(
    cfg: &CsmConfig, view: &ViewUniform, sun_dir: [f32; 3], fit: CsmFit,
) -> ResolvedCsm;

/// (was :535) the cold single writer of `ResolvedCsm`; now also the single writer of
/// `CsmFitState`. Plain `Res`/`ResMut` only ⇒ stays `Send` (the `NonSend` `Assets<MeshGpu>`
/// term lives on the reducer, where it already is).
pub fn resolve_csm_cascades(
    cfg: Res<CsmConfig>,
    view: Res<ViewUniform>,
    suns: Query<&DirectionalLight>,
    bounds: Res<CsmCasterBounds>,
    mut state: ResMut<CsmFitState>,
    mut out: ResMut<ResolvedCsm>,
);

/// Bit-exact, libm-free, branchless. `x` must be finite and `> 0` (debug_assert).
#[inline] pub fn grid_cell(x: f32) -> i32;
/// Exact: `TABLE[k mod 4] * 2^(k div 4)`.
#[inline] pub fn grid_value(k: i32) -> f32;
/// The Schmitt latch. `prev_k == CsmFitState::UNLATCHED` ⇒ fresh latch.
#[inline] pub fn latch_cell(raw: f32, prev_k: i32) -> i32;

// ── csm_caster.rs ────────────────────────────────────────────────────────────
/// The cold fold SYSTEM. UNWIRED EXPORTED API by design — mirrors `gather_shadow_casters`
/// (:152-164): `CsmPlugin` does NOT register it; the owning app co-registers it
/// `.after(gather_shadow_casters)` and `.in_set(CsmFitSet)`. Without it the fit never
/// latches and every mode renders as `Fixed`.
pub fn reduce_caster_bounds(
    cfg: Res<CsmConfig>,                    // the fit_mode 0%-gate
    view: Res<ViewUniform>,                 // the view axis for the per-instance depth
    scratch: Res<CsmCasterScratch>,         // the gather's OUTPUT — not a new query
    mesh_assets: NonSendRes<Assets<MeshGpu>>,
    mut out: ResMut<CsmCasterBounds>,
);

/// The pure, World-free core (unit-testable without an ECS; mirrors the closure-meta idiom
/// at csm_caster.rs:188).
pub fn reduce_bounds_into(
    batches: &[DrawBatch],
    ring: &[InstanceModelCol],
    eye: [f32; 3], forward: [f32; 3],
    mesh_aabb: impl Fn(u32) -> Option<([f32; 3], [f32; 3])>,   // None ⇒ skip (F6 discipline)
) -> CsmCasterBounds;
```
`resolve_csm`'s only call sites are csm_config.rs's own system (:562) and its tests (:621-884) — no external breakage.

---

## 6. Algorithms

### A. `reduce_bounds_into` — O(instances), cold, once/frame
1. `if cfg.fit_mode == Fixed → return EMPTY` (the 0%-gate: default costs **0 ns**). *(In the system; the pure fn is always callable for tests.)*
2. `acc = EMPTY; total = batches.len()`.
3. Per batch (cold, ~10s): `mesh_aabb(batch.mesh_id)` → `None` ⇒ **skip, do not count as resolved** (never dereference a non-`Loaded` slot — csm_caster.rs:186-191). Hoist `lc = (min+max)*0.5`, `lh = (max-min)*0.5`.
4. Per instance in `ring[base_instance .. +instance_count]` — **branch-free**:
   - `wc[r] = Σⱼ rows[r][j]·lc[j] + rows[r][3]`; `wh[r] = Σⱼ |rows[r][j]|·lh[j]` (Arvo; exact for any linear map incl. shear).
   - `world_min = min(world_min, wc−wh)`, `world_max = max(world_max, wc+wh)` (union — for `up_need` only).
   - `d_center = dot(forward, wc − eye)`; `d_half = |fwd·(wh signed-abs)| = Σᵣ |forward[r]|·wh[r]`; `raw_far = max(raw_far, d_center + d_half)` — **per instance**, so lateral spread never inflates depth (D4).
5. `acc.resolved_batches = resolved; acc.total_batches = total`.

**Complexity** O(instances + batches) · **Cache** strictly sequential 48 B stride over the ring the gather *just wrote* (L1/L2-resident; 3 instances per 2 lines; prefetcher-perfect) · **Branching** one `Option` per *batch*, zero in the inner loop · **SIMD** ~45 flops, no sqrt, no division, 3× `f32x4` row loads — **autovectorizable; do NOT hand-write intrinsics** until a profile says otherwise (Principle 7) · **Budget** ≤15 µs @ 10k instances · **Monoid** (min/max) ⇒ trivially parallelizable later if it ever profiles hot.

### B. `resolve_csm_cascades` — the latch (cold, O(1))
1. Existing sun / `fov_y == 0` guards (:543-560) unchanged.
2. `usable = bounds.is_usable()`.
3. Gate:
   - `cfg.fit_mode == Fixed` ⇒ `resolve_csm(.., CsmFit::NONE)`. **Bounds and state are never read** ⇒ zero staleness exposure on every existing golden. *(Do not touch `far_k` here — a mode toggle mid-session must not corrupt the latch.)*
   - `cfg.fit_mode == CatchAll && cfg.cascade_count < 2` ⇒ `CsmFit::NONE` (cannot reserve the only cascade).
   - `usable` ⇒ `k = latch_cell(clamp(bounds.raw_far, near+MIN_DIAMETER, far_cap_today), state.far_k)`; `state.far_k = k`.
   - `!usable && state.far_k != UNLATCHED` ⇒ **HOLD**: `k = state.far_k` (D7 — kills the streaming/blink strobe).
   - `!usable && state.far_k == UNLATCHED` ⇒ `CsmFit::NONE` (ground-truth constraint 3).
4. `far_eff = grid_value(k).clamp(near + MIN_DIAMETER, far_cap_today)`.
   `if far_eff >= far_cap_today` ⇒ `CsmFit::NONE` (casters already reach the shadow distance — nothing to shrink; also kills `CatchAll`'s zero-width tail).
5. `resolve_csm(cfg, view, sun, CsmFit { far_eff: Some(far_eff), reserve_tail: fit_mode == CatchAll, caster_aabb: Some((bounds.world_min, bounds.world_max)) })`.

`latch_cell(raw, prev_k)`:
```
if prev_k == UNLATCHED            -> grid_cell(raw) + 1                  // fresh; far_eff ≥ raw
if raw >  grid_value(prev_k)      -> grid_cell(raw) + 1                  // GROW: immediate, never clip
if raw <  grid_value(prev_k - 2)  -> grid_cell(raw) + 1                  // SHRINK: only after 2 cells
else                              -> prev_k                              // sticky
```

### C. `resolve_csm` — exactly three edits to a proven function
- **:318** `let far_cap = view.far.min(cfg.shadow_distance).max(near + MIN_DIAMETER);` → keep as `far_cap_today`; add
  `let (pssm_far, pssm_n, tail) = match fit.far_eff { None => (far_cap_today, count, None), Some(f) if fit.reserve_tail => (f, count-1, Some(far_cap_today)), Some(f) => (f, count, None) };`
- **:358** `let split_i = pssm_split(near, far_cap, cfg.lambda, i+1, n);` →
  `let split_i = if tail.is_some() && i == count-1 { tail.unwrap() } else { pssm_split(near, pssm_far, cfg.lambda, i+1, pssm_n as f32) };`
  *(`near_i` at :356/:429 is UNTOUCHED — it chains from the previous split, so `CatchAll`'s reserved cascade naturally slices `[far_eff, far_cap]`.)*
- **:386-387** `let z_far = 2.0*diameter; let eye = center + sun*(z_far*0.5);` →
  ```
  let up_need = match fit.caster_aabb {
      None => diameter,                                   // ⇒ byte-identical to today
      Some((mn, mx)) => grid_value(grid_cell(
            max over the 8 corners of sun.dot(corner - center)  // ≤0 ⇒ diameter
      ) + 1),
  };
  let pull_back = up_need.clamp(diameter, MAX_PULLBACK_RATIO * diameter);
  let z_far = pull_back + diameter;
  let eye   = center + sun * pull_back;
  ```
**Everything else (:361-381 corners→sphere→ceil→texel snap, :392-427 matrix assembly) is byte-for-byte untouched.** The PROVEN convention is not re-derived.

---

## 7. Multithreading

- All new work is **cold, once-per-frame**. No atomics, no locks, no interior mutability, **no `unsafe`** ⇒ **Miri N/A** for this feature.
- Single-writer discipline: `gather_shadow_casters` ⊳ `CsmCasterScratch`; `reduce_caster_bounds` ⊳ `CsmCasterBounds`; `resolve_csm_cascades` ⊳ `ResolvedCsm` + `CsmFitState`. Every new Resource has exactly one `ResMut`. Data-race freedom is proved by the scheduler from the `Res`/`ResMut` declarations.
- `reduce_caster_bounds` is `NonSend` (it reads `NonSendRes<Assets<MeshGpu>>`, the same class as `gather_shadow_casters`, :171) ⇒ main thread. **The pin does not propagate:** `resolve_csm_cascades` takes only plain `Res<CsmCasterBounds>`, so the fit stays thread-agnostic.
- `Res<CsmCasterScratch>` (shared read) runs concurrently with any other reader; it conflicts with the gather's `ResMut`, so the scheduler already serialises those two — D11's set edge makes the order deterministic rather than incidental.
- `Send`/`Sync`: all new types are POD `Copy` ⇒ auto. `MeshGpu`'s Send-ness unchanged.
- **Ordering edges:** `gather_shadow_casters → reduce_caster_bounds` (same closure, `SystemKey`, plugins.rs:300) and `CsmFitSet → CsmResolveSet` (cross-plugin, `configure_set` in plugins.rs — mechanism verified, D11). **No accepted stagger.**

---

## 8. Correctness — edge cases

| Case | Behavior | Pin |
|---|---|---|
| `fit_mode == Fixed` (DEFAULT) | today's fit, bit-identical; bounds/state never read | T1 |
| no casters ever (`total_batches == 0`, unlatched) | `Fixed` (constraint 3) | T4 |
| reducer not registered | bounds stay `EMPTY` ⇒ never latches ⇒ `Fixed` (silent no-op — mitigated by wiring it in `EnginePlugins` + the `CsmFitMode` doc) | T4, T15 |
| mesh not `Loaded` (streaming) | batch skipped ⇒ `resolved < total` ⇒ **HOLD** the latch (or `Fixed` if unlatched) | T5, T13 |
| caster blinks (`RenderEnabled` toggling) | HOLD ⇒ **no strobe** | T6 |
| bare `CsmPlugin` world (no `CsmCasterScratch`) | `CsmPlugin` inserts `CsmCasterBounds::EMPTY` + `CsmFitState::UNLATCHED` ⇒ no panic, `Fixed` | T15 |
| all casters behind the camera (`raw_far <= near`) | clamped to `near + MIN_DIAMETER` ⇒ step 4's `far_eff >= far_cap` is false; the fit degenerates to a 1e-3 range ⇒ every cascade is `MIN_DIAMETER`-floored. **Explicit rule:** `raw_far <= near + MIN_DIAMETER` ⇒ `CsmFit::NONE` | T7 |
| casters reach `shadow_distance` (`far_eff >= far_cap`) | `Fixed` (nothing to shrink; kills `CatchAll`'s zero-width tail) | T8 |
| `CatchAll` + `cascade_count == 1` | `Fixed` | T9 |
| `cascade_count == 0` / `shadow_distance <= 0` | `DISABLED` at :297 — unreached | existing |
| `up_need <= 0` (all casters down-sun) | `pull_back = diameter` ⇒ today's Z volume | T11 |
| `up_need` huge (a far up-sun caster) | clamped to `4·diameter` ⇒ bounded `z_far`; still ≥ today | T11 |
| `far_k` overflow / subnormal input to `grid_cell` | `#[cold] #[inline(never)]` clamp to the safe exponent range | T10 |
| Drop order | N/A — all new types are `Copy` POD; `MeshGpu` still has no `Drop` (mesh.rs:131-136) | — |

**`debug_assert!`s:** `grid_cell` input finite `> 0`; `grid_value`'s `q` within the `exp2i`-safe exponent range; `far_eff > near && far_eff.is_finite() && far_eff <= far_cap_today`; splits strictly increasing; `base+count <= ring.len()`; `world_min[i] <= world_max[i]` when `resolved_batches > 0`; `local_min[i] <= local_max[i]` in `build_mesh_gpu`; `pull_back >= diameter`; on frame 0, the inserted `CsmFitState.far_k == UNLATCHED`.

---

## 9. Rungs (each independently buildable, `cargo clippy --all-targets -- -D warnings` + `cargo test` green, committed separately)

**C0 — mesh AABB (dark).** `mesh.rs:137` +`local_min`/`local_max` with the `blas` Principle-0 doc precedent; `mesh_assets.rs:216-405` min/max fold inside `build_mesh_gpu`'s existing vertex walk + `debug_assert`; 2 test dummies (`tests/asset_streaming_f5_validation.rs:68`, `tests/asset_streaming_f8_material_gather.rs:43`). Tests: T16. *No behavior change.*

**C1 — the grid (dark, pure).** `csm_config.rs` +consts, `grid_cell`/`grid_value`/`latch_cell` + `#[cold]` clamps. Tests: T10, T17, T18, T19. *Nothing calls them.*

**C2 — caster bounds (dark).** `csm_caster.rs` +`CsmFitSet`, `reduce_bounds_into` (pure), `reduce_caster_bounds` (system, with the unwired-API doc mirroring :152-164); `csm_config.rs` +`CsmCasterBounds`; `csm_plugin.rs:58` inserts `CsmCasterBounds::EMPTY`. Tests: T13, T20, T21. *Nobody reads the Resource.*

**C3 — the knob + the fit (live, default-off).** `csm_config.rs` +`CsmFitMode`, `CsmConfig.fit_mode` (:100/:126), `CsmFitState`, `CsmFit`, `CsmResolveSet`; widen `resolve_csm` (:296) and edit :318/:358; `resolve_csm_cascades` (:535) +`bounds`/`state` + the latch; `csm_plugin.rs:62` `.in_set(CsmResolveSet)` + insert `CsmFitState`; mechanical `, CsmFit::NONE` on the existing test call sites (:621-884, **otherwise unmodified** — that is the golden-freeze proof); `lib.rs` re-exports. Tests: T1–T9, T12, T14. **Goldens frozen (default `Fixed`).**

**C4 — the sun-axis pull-back.** `csm_config.rs:386-387` → `up_need`/`pull_back`/`z_far`. Tests: T11 + T1 must still pass (the `Fixed`-byte-identity pin covers it). **Goldens frozen.**

**C5 — app wiring.** `plugins.rs:300` `b.add_system(reduce_caster_bounds).after(casters).in_set(CsmFitSet);` + `b.configure_set(CsmResolveSet).after(CsmFitSet);` (same closure as `casters`). No scene changes `fit_mode` ⇒ **goldens frozen**. Tests: T15, T22.

**C6 — owner-eval (no golden).** An `#[ignore]` screenshot pair (`Fixed` vs `CatchAll` vs `Shrink`) on the owner's close-up scene, **plus an in-motion dolly capture** (the `shadow_lag_dump` / "diagnose in-motion" discipline) so the owner judges (a) the sharpness win, (b) the S2 pop, (c) `Shrink`'s terminator, (d) acne under the grown `z_far`. **Deliver §2's measured table with the capture.** The owner is the visual oracle; only the owner can flip a pinned scene's `fit_mode` — and that is what would move a golden.

---

## 10. Tests

| # | Test | Pins | File |
|---|---|---|---|
| **T1** | `fixed_mode_is_bit_identical_to_today` — `resolve_csm(Fixed, v, s, populated_fit) == resolve_csm(Fixed, v, s, CsmFit::NONE)`, and every existing test at :621-884 passes with only a `, CsmFit::NONE` arg added | **THE golden-freeze pin** (D10) + `Fixed`'s `pull_back == diameter` (D5) | csm_config.rs |
| **T2** | `fit_is_bit_identical_at_rest` — same camera + same bounds, two calls, `assert_eq!` on the whole `ResolvedCsm` incl. every `texel_size` | **Property S1** (D6) | csm_config.rs |
| **T3** | `camera_dolly_pop_count_and_magnitude_are_bounded` — sweep the **camera** through a raw-far ratio R (both approaching and receding) in 200 steps; collect the distinct `texel_size` sequences; assert transitions ≤ `ceil(log_1.18921 R)` receding / `ceil(log_1.41421 R)` approaching, and each transition's `far_eff` ratio ∈ `[1/1.4143, 1.1893]` | **Property S2** — the real anti-shimmer obligation (D6) | csm_config.rs |
| **T4** | `unlatched_or_no_casters_falls_back_to_fixed` — `Shrink`/`CatchAll` + `EMPTY` + `UNLATCHED` `==` `Fixed` | constraint 3 | csm_config.rs |
| **T5** | `incomplete_bounds_hold_the_latch` — latch, then feed `resolved < total`; assert `far_k` unchanged and the fit identical to the previous frame | streaming drift (D7) | csm_config.rs |
| **T6** | `blinking_caster_does_not_strobe` — alternate usable/`EMPTY` for 10 frames after latching; assert every frame's `ResolvedCsm` is `assert_eq!`-identical | the 36% strobe (D7) | csm_config.rs |
| **T7** | `casters_behind_camera_fall_back_to_fixed` | `raw_far <= near` | csm_config.rs |
| **T8** | `casters_reaching_shadow_distance_fall_back_to_fixed` | `far_eff >= far_cap`; `CatchAll` zero-tail | csm_config.rs |
| **T9** | `catch_all_with_one_cascade_degenerates_to_fixed` | `count < 2` | csm_config.rs |
| **T10** | `grid_cell_grid_value_round_trip` — ∀k in the safe range `grid_cell(grid_value(k)) == k`; `grid_value(grid_cell(x)) <= x < grid_value(grid_cell(x)+1)`; `far_eff >= raw` (never clips) | the exact-grid claim (D6) | csm_config.rs |
| **T11** | `up_sun_caster_shadow_survives_the_shrink` — a caster 2.5 m up-sun of cascade 0's centre with `diameter == 2`: assert its world position maps inside `[LIGHT_Z_NEAR, z_far]` of cascade 0's `view_proj` under `CatchAll`, and that `pull_back >= diameter` and `z_far <= 5*diameter` always | **the vanishing-shadow regression** (D5) | csm_config.rs |
| **T12** | `shrink_relocates_the_terminator_catch_all_does_not` — `Shrink`: `split_far[N-1] == far_eff < shadow_distance`; `CatchAll`: `split_far[N-1] == far_cap` and `split_far[N-2] == far_eff` | the documented `Shrink` trade-off (D2) — deliberate, not a bug | csm_config.rs |
| **T13** | `reduce_skips_non_loaded_mesh` — `mesh_aabb → None` ⇒ batch skipped, `resolved < total`, no panic | F6 discipline | csm_caster.rs |
| **T14** | `light_gate_is_identical_across_all_fit_modes` — `sync_csm_light_gate`'s output for the same scratch under `Fixed`/`Shrink`/`CatchAll` | **no predicate drift** (D7) | csm_caster.rs |
| **T15** | `bare_csm_plugin_world_runs_without_the_reducer` — build an `App` with `CsmPlugin` only, run 2 frames, assert no panic and `ResolvedCsm == resolve_csm(cfg, v, s, NONE)` | the missing-resource panic (D7/I) | csm_plugin.rs |
| **T16** | `local_aabb_of_cube_is_half_edge` — against `cube_geometry` (mesh_assets.rs:422) | C0 | mesh_assets.rs |
| **T17** | `latch_grows_immediately_shrinks_after_two_cells` | the Schmitt property | csm_config.rs |
| **T18** | `latch_has_no_limit_cycle` — a monotone up-then-down sweep yields ≤1 `k` change per cell traversed; and a ±9% oscillation astride a grid line yields **zero** `k` changes | **refutes the "measure-zero dither" hand-wave** (B) | csm_config.rs |
| **T19** | `grid_is_monotone_step_over_a_decade` — non-decreasing; distinct count ≈ `log_1.18921(10) ≈ 14` | grid sanity | csm_config.rs |
| **T20** | `reduce_matches_manual_transform_for_sheared_instance` — rotated + non-uniformly-scaled + **sheared**; assert the abs-matrix AABB contains all 8 transformed local corners | D4's exactness/conservativeness | csm_caster.rs |
| **T21** | `laterally_spread_casters_do_not_inflate_raw_far` — two instances at view-depth 3, world x = ±50, oblique `forward`: assert `raw_far ≈ 3`, **not ≈ 28** | **the union-AABB error** (D) | csm_caster.rs |
| **T22** | `room_smoke` (room_smoke.rs:177) — assert `CsmCasterBounds.total_batches > 0` (valid under the **default** `Fixed`? **No** — the 0%-gate returns `EMPTY`. So: assert the reducer RAN by asserting the Resource exists and `resolve_csm_cascades` produced today's fit; and add a second `#[test]` that sets `fit_mode: CatchAll` on a **non-pinned** smoke world and asserts `resolved_batches == total_batches > 0`) | the wiring, without contradicting the 0%-gate (fixes flaw K) | room_smoke.rs |

**Property test:** extend `every_view_proj_element_finite_for_random_camera_and_sun` (:746) to sweep `fit_mode` × random bounds × random sun ⇒ every `view_proj` finite ∧ non-singular ∧ `split_far` strictly monotone ∧ `texel_size > 0`.

**Benchmarks:** `bench_reduce_caster_bounds` @ 100/1k/10k/100k (≤15 µs @10k, **0 allocations**, **linear** scaling — a super-linear result means the ring walk regressed); `bench_resolve_csm` per mode (`Fixed` must not be slower than today; all modes stay flat O(4)).

**Miri:** N/A — this feature adds **zero `unsafe`**. State that in the tester's report rather than skipping the question.

---

## 11. MOVES A GOLDEN

**None.**

Checkable claim: the default is `CsmFitMode::Fixed`; under `Fixed`, `resolve_csm` takes `CsmFit::NONE`, which reduces textually to today's `far_cap` (:318), today's `pssm_split(near, far_cap, λ, i+1, n)` (:358), and today's `z_far = 2·diameter` / `eye = center + sun*(z_far*0.5)` (:386-387, via `pull_back = diameter`). Bounds and latch state are **never read** under `Fixed`. `MeshGpu`'s new fields never reach a shader. No `.hlsl`/`.spv`/host/UBO-ring is touched **in any mode**. `goldens/PINS.toml` is untouched; **no re-bless, no owner sign-off required for C0–C5**.

Guard: T1 + the unmodified :621-884 suite. If any CSM golden's bytes move, T1 is lying — that is a build-breaking bug, not a re-bless.

The **only** thing that could move a golden is a pinned scene opting into `Shrink`/`CatchAll`. Rung C6 deliberately does that in an `#[ignore]` owner-eval capture, not in a pinned scene. Flipping a pinned scene is a separate, owner-signed rung.

---

## 12. Open questions (for the owner)

1. **The 13-tap PCF tent is very likely the bigger lever, and it is a sibling rung — not this one.** `shadow_apply.hlsli:143-152` blurs over a ~10-texel footprint; its own doc sizes that at *"2-3 screen px at room viewing distance"*, but the tent is measured in **texels**, so at the shipped config's 6.4-screen-px texel the penumbra is **~64 screen px**, and even after this fit lands (1.83 px texel) it is **~18 px**. Good news: it scales *with* `texel_size`, so the two fixes **compound** multiplicatively. Recommendation: land C0–C5 (they are cheap, dark and golden-free), then measure the tent as the next rung. If the owner's complaint is "мыло" (mush) rather than "блоки" (blocks), the tent is the defect and this whole feature is secondary.
2. **`cascade_count: 3 → 4` is a zero-risk lever with the same mechanism** (it compresses cascade 0's depth ratio 25.5 → 12.4). It already exists on `CsmConfig`, costs one more depth pass, and moves no golden by default. Worth an A/B in the C6 capture alongside the fit modes.
3. **Is `Shrink` worth shipping?** §2 shows it is *not* dominated (it differs measurably from `CatchAll`), but it is worse in the tested close-up scene **and** carries the hard terminator (D2). The owner asked for both; both are shipped. If the C6 capture shows `Shrink` is never preferred, deleting the variant later is a one-line change (the enum is `#[non_exhaustive]`-free and internal).

---

**Files:** `D:\claude\BoykoEngine\crates\boyko_render\src\csm_config.rs` · `D:\claude\BoykoEngine\crates\boyko_render\src\csm_caster.rs` · `D:\claude\BoykoEngine\crates\boyko_render\src\csm_plugin.rs` · `D:\claude\BoykoEngine\crates\boyko_render\src\mesh.rs` · `D:\claude\BoykoEngine\crates\boyko_render\src\mesh_assets.rs` · `D:\claude\BoykoEngine\crates\boyko_render\src\lib.rs` · `D:\claude\BoykoEngine\crates\boyko_app\src\plugins.rs` · `D:\claude\BoykoEngine\crates\boyko_render\tests\asset_streaming_f5_validation.rs` · `D:\claude\BoykoEngine\crates\boyko_render\tests\asset_streaming_f8_material_gather.rs` · `D:\claude\BoykoEngine\crates\boyko_app\tests\room_smoke.rs` · `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\shaders\shadow_apply.hlsli` (**read-only evidence — NOT edited in any rung**)