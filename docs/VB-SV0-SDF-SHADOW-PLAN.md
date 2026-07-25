# VB-SV0 — SDF soft-shadow + contact-AO on mesh, inlined into the VB lit-producer tails

**Status:** DESIGN, **Rev 2** — NOT YET APPROVED. Stage 2 of the "finish VB completely" campaign
(Stage 1 = VB-P1 clustered cull, COMPLETE; Stage 3 = VB-P4 GPU-driven raster, out of scope).
Rev 1 drew **CHANGES REQUESTED (3 P0, 5 P1, 3 P2)**. Rev 2 answers all eleven. Two of the three P0s
refuted claims Rev 1 made about itself; both are **withdrawn**, not defended.

**This document states no measured number in prose.** Every fact that could drift is a named test,
and the test name is the citation. Numbers that appear are either *structural bounds* (derived from
a loop's own form) or explicit `MEASURE` placeholders a rung fills in **code**. Golden hashes are
never written here — gates read them from `goldens/PINS.toml`. Rev 1's 735-line budget is a
deliberate constraint inherited from `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md`, whose own status
block diagnoses that hand-copied numbers in prose caused every revision to introduce defects at the
lines it edited.

---

## Changelog Rev 1 → Rev 2

| # | Change | Cause |
|---|---|---|
| C1 | **WITHDRAWN**: the SPIR-V Merkle-hash dataflow-equivalence instrument (Rev 1 §3.3, rung S0, ~200 LOC). Replaced by §3.3's two-instrument pair, both built from helpers that already exist. | P0-1 |
| C2 | **WITHDRAWN**: the `spec_ao` sub-LSB hazard, the duplicated-recompute construction, S2's second "decisive" mutation, and risk R2 as written. No replacement hazard is invented. | P0-2 |
| C3 | S4's arming gate re-specified: its **selection is all 10 shipping lit-producer variants**, each with an executing assertion; the demonstrated red mutations partition by source. 2 of the 10 are shown *structurally* OFF-path and get a CPU truth-table proof instead. | P0-3 |
| C4 | S1's gate now asserts **fixture adequacy for the SV0 term** via a CPU-side host `Eval` oracle, not "the frame differs". | P1-1 |
| C5 | **CORRECTED**: `sdf_ao` is **not** eDSL-generated. §4.1 replaced by a shared `sdf_shadow_leaves.hlsli` that cuts the copy count from 4 to 2 and pins the survivor. | P1-2 |
| C6 | The arming predicate **consumes** `ResolvedRenderPath::shadow`; the stale `render_path_config.rs:727` citation is dropped. The `!hwrt` term's visual consequence is named in §1.2 and §10. | P1-3 |
| C7 | S4(i)'s "any difference is a bug" is now scoped to a precisely stated claim. C2 removes the second `spec_ao` site that made Rev 1's version a coin-flip. | P1-4 |
| C8 | No golden hash literal appears in this document. §5.3 records the `PINS.toml` self-contradiction as a **blocking precondition on S1** rather than inheriting it. | P1-5 |
| C9 | §4.3 "five sources" → **four** (verified). §2.3 states the `register(t0)` space-0 check and its near-miss. §2.4's citation corrected and R11's tripwire re-sited out of `debug_assert!`. | P2-1/2/3 |
| C10 | New §10 answers the critic's four open questions with verified evidence. | — |

---

## 0. What the stale row said, and what is now known

`docs/RENDER-PARITY-PLAN.md:351` specifies SV0 as: *"reuse `sdf_mesh_shadow.comp` under VB … one
`gSdfMeshShadow` binding added to vb Set 0 (O1: no 5th set)"*. It predates all of Stage 1.

| Premise | Status |
|---|---|
| "reuse `sdf_mesh_shadow.comp`" | **FALSE.** `sdf_mesh_shadow` greps to **doc files only** (`docs/RENDER-PARITY-PLAN.md`, this plan). SF0 was never implemented; there is no pass to reuse and no `ResolvedRenderPath::sdf_mesh_shadow` field. |
| "one binding added to vb Set 0" | **TRUE, and the slot is 10** — §2. Slot 8 is *not* universally free. |
| "a dedicated producer pass" | **REJECTED by measurement** (campaign record: cost for zero visual gain). Inline. |

The surviving prior art is the **Deferred** path, which already ships this exact visual:
`sdf_gbuffer_composite.hlsl:1853-1885` — the `!own_pixel` raster-owned arm writing
`gMaterial.RG = (mesh_shadow, mesh_ao)`, computed at `:1876-1878` and `:1881`. SV0 is the port of
those two lines into the VB tails.

---

## 1. Scope

### 1.1 What SV0 IS

Two per-pixel terms, evaluated **inline** in the VB lit-producer tails, for pixels the VB rasterizer
covered (`instance_id != VB_ID_SENTINEL`, `vb_resolve.comp.hlsl:241`):

* **SDF soft-shadow on mesh** — `min`-combined into the **primary directional light's** `vis`, at
  the site the tail already `min`-combines `csm_visibility` (`vb_resolve.comp.hlsl:308-315`;
  `vb_shade.comp.hlsl:475-482`; the split tail's own equivalent).
* **Contact-AO** — the 5-tap field-deficit AO `min`-combined into `ao_final`
  (`vb_resolve.comp.hlsl:288`, `vb_shade.comp.hlsl:450/452`, `vb_shade_split.comp.hlsl:453/455`),
  i.e. the *diffuse* ambient occlusion. `spec_ao` follows by the existing formula.

Both terms land in **all three** lit-producer sources: `vb_resolve.comp.hlsl` (fused),
`vb_shade.comp.hlsl` (classified), `vb_shade_split.comp.hlsl` (R9 geo/shade split) — §6's rung
**S4** closes that structurally across all 10 shipping `.spv`.

**The AO routing matches Deferred exactly**, verified: `deferred_pbr.hlsl:797` reads the marcher's
mesh-AO lane (`material_texel.g`), `:948` seeds `ao_final` from it, and `:977` derives `spec_ao`
from that `ao_final` by the identical expression. Deferred therefore *does* propagate mesh AO into
specular ambient, so §1.3's equivalence claim is structural, not aspirational.

### 1.2 What SV0 is NOT

* **Not a new pass, target, framegraph `ResId`, or barrier.** `field_distance` is pure-analytic over
  one SSBO (`sdf_field.hlsli:203-217`, gateway at `:246`). Its include contract requires
  `StructuredBuffer<uint> Buf : register(t0)` in scope before the `#include`
  (`deferred_pbr.hlsl:153-161`). Cost: +1 SSBO binding and a control-flow gate.
* **Not multi-caster.** The P6 R1 per-light flagged-caster march (cap
  `MAX_SDF_SHADOW_CASTERS_PER_PIXEL = 4`, `deferred_pbr.hlsl:479`) is out of scope. SV0 marches
  **exactly once per covered pixel**, for the primary directional only — matching the Deferred mesh
  arm's single `pc.light_dir` (`sdf_gbuffer_composite.hlsl:1868`).
* **Not mesh-SDF (MDF).** The `pc.mesh_sdf_enabled` arm is not ported.
* **Not correct under non-uniform instance scale.** §4.2.
* **Not armed under HWRT — and that is a visual gap, named here.** SV0 consumes
  `ShadowSources::SDF_SOFT_MARCH`, whose shipped predicate carries `!consumers.hwrt_denoise_or_vis_on`
  (`render_path_config.rs:904`). On `VB × Both × HWRT` the SDF field therefore casts **no** shadow on
  mesh, because HWRT traces only the mesh TLAS and SDF bodies are not in it. SV0 neither introduces
  nor fixes this; §10 Q3 records it as an unlisted follow-up made explicit.
* **Not byte-comparable to Deferred.** `goldens/PINS.toml:288-291` records that VB's analytic
  barycentric interpolation is a genuinely different FP path from hardware raster interpolation.
  VB-vs-Deferred parity is **visual** (owner-eval), never a byte gate.

### 1.3 Visual goal

On a VB×Both scene with a non-empty SDF edit list, an SDF body casts a soft, noise-free shadow onto
raster mesh surfaces, and mesh surfaces darken in the SDF geometry's ambient-occluded crevices —
visually equivalent to what `Deferred × Both` already produces on the same scene.

---

## 2. The binding decision

**Decision: VB Set 0, binding 10, `StructuredBuffer<uint> Buf : register(t0)`.**

### 2.1 Why not a 5th set

The TEXTURED VB variant already consumes four descriptor sets (0 core / 1 shadow / 2 geometry table
/ 3 bindless textures — `vb_shade_split.comp.hlsl:70-108`, `vb_geom_fetch.hlsli:20-34`,
`vb_shade.comp.hlsl:167`). Vulkan's guaranteed `maxBoundDescriptorSets` floor is exactly 4.

### 2.2 Why slot 10 and not slot 8

VB Set 0 today: 0 `gVbInstances` · 1 `instance_materials{,_tex}` · 2 `Camera` · 3 `LightBuf` ·
4 `Materials` · 5 `gVbId` · 6 `gLit` · 7 `gClassify` · **8, 9 `ClusterGrid`/`LightIndexList` —
`#ifdef FROXEL` ONLY** (`vb_resolve.comp.hlsl:151-154`).

Two host layout objects exist: `vb_layout0` (8 entries,
`crates/boyko_app/src/gpu_scene/mod.rs:3395-3459`) and `vb_layout0_froxel` (10 entries, `:4425-4492`).
**Slot 8 is free only in scenes that never arm the froxel cull** — using it would be a silent,
scene-config-dependent collision that no validation layer reports (validation is off on this box;
`robustBufferAccess` is off). **Slot 10 is free in both.**

### 2.3 The precedent — and the register check Rev 1 skipped

`deferred_pbr.hlsl:161` already declares `[[vk::binding(10)]] StructuredBuffer<uint> Buf : register(t0);`.
The comment at `:153-160` states the precedent precisely: *"the resolve's `t0` SRV register is free
(it uses t4/t6/t8/t9), and Vulkan binding 10 is free"* — i.e. it verifies **both** the HLSL register
and the Vulkan binding. Rev 1 paraphrased this as "the register is pinned, the binding is free" and
never performed the register half for the VB tails.

**Performed now, and it is a near-miss.** `register(t0)` **space 0** is free in all three tails:
`vb_resolve.comp.hlsl` uses t5/u6/t12/s12/t14/s14 only; `vb_shade.comp.hlsl` and
`vb_shade_split.comp.hlsl` declare `gTextures[] : register(t0, space3)` (`:167` / `:204`) — an
**unbounded** SRV array at t0, but in **space 3**, so no collision with a space-0 `t0`. A collision
would be a hard `dxc` error surfacing only at S2; S2's gate (e) catches it, but the check belongs
here.

### 2.4 Host changes

Binding numbers need not be contiguous; only the *entry count* is capped
(`crates/boyko_rhi_vulkan/src/rhi_impl/mod.rs:93`, `MAX_BIND_GROUP_BINDINGS = 24`). `vb_layout0`
goes 8 → 9 (`{0..7, 10}`), `vb_layout0_froxel` 10 → 11. Four Set-0 descriptor **set** instances gain
the entry, all binding the same buffer: `vb_set0` (`crates/boyko_rhi_vulkan/src/present/targets.rs:2995-3079`,
entry array `:3012-3024`), `vb_set0_tex` (`:3090`), `vb_set0_froxel` (`:3193`),
`vb_set0_tex_froxel` (`:3311`).

The bound resource is `BindGroupEntry::StorageBuffer { buffer: scene.edit_list }` — the identical
expression the deferred/marcher sets use at `targets.rs:1402`, `:2378`, `:2745`, `:2903`.
`scene.edit_list` is a plain (non-`Option`) field, valid on **every** VB boot including `legs: Mesh`.

**No new upload, no new barrier — with the citation corrected.** The edit list is a one-shot
boot-static write. `crates/boyko_app/src/runner.rs:1136-1149` is the *comment* describing it; the
write itself is `:1182-1197`, **inside the frame loop**, gated by `staging.is_dirty()` and followed
by `mark_uploaded()`.

> **R11 tripwire, re-sited (P2-3).** Rev 1 put a `debug_assert!` at the upload site. That site is in
> the frame loop and `debug_assert!` compiles out in release — and the goldens run release, so it
> could not fire where it matters (the existing `#[cfg(debug_assertions)]` block at `:1162-1181` has
> the same limitation). Rev 2 moves the invariant into **test code**, which is profile-independent:
> rung S1's fixture test drives the real runner for ≥2 frames and asserts
> `SdfEditStaging::is_dirty()` is **false** after frame 1 and stays false. A future rung that makes
> the edit list per-frame dirty reds that test regardless of build profile.

---

## 3. The gate mechanism

### 3.1 A 2-bit field in light-header word 7, bits 5..6

Word 7 (`sky_diffuse.w`) is the campaign's established gate word; the authoritative bit budget is
`crates/boyko_render/src/light.rs:386-409`, the shader decoders `light_table.hlsli:77-180`.
Bits 0..4 are `shadow_mode`/`contact_shadow_mode`/`csm_mode`/`punctual_shadow_mode`/`ddgi_mode`;
8..11 tonemap; 12..19 terminator softening. `light.rs:406` and `light_table.hlsli:154` both state
**bits 5..7 free**; SV0 claims 5..6 and leaves 7.

*Checked, because it is absent from that budget:* `load_ssao_mode` reads **`LightBuf[11]`**
(`light_table.hlsli:218-220`), a different header word entirely — it does not contend for word 7.

```hlsl
// light_table.hlsli (additive; each decoder masks only its own bits)
static const uint VB_SDF_MESH_OFF        = 0u;
static const uint VB_SDF_MESH_SHADOW_BIT = 1u; // bit 5
static const uint VB_SDF_MESH_AO_BIT     = 2u; // bit 6
uint load_vb_sdf_mesh_mode(StructuredBuffer<uint> LightBuf) { return (LightBuf[7] >> 5) & 3u; }
```

Host side, `boyko_render::light`: `VB_SDF_MESH_MODE_SHIFT: u32 = 5`, `VB_SDF_MESH_MODE_MASK: u32 = 3`,
two `LightingConfig` bools packed by the existing `shadow_gate_word`, plus a bit-position
`debug_assert_eq!` at the single writer — the idiom `ddgi_config.rs:288-289` uses.

### 3.2 Why a runtime gate and not `-D` — the arithmetic

**Shipping VB lit-producer `.spv` today: 10** (`crates/boyko_rhi_vulkan/shaders/`, embeds at
`compute.rs:811-1037`, manifest `docs/SHADER-VARIANT-MANIFEST.md:91-107`):
`vb_resolve{,_froxel}` (2, from `vb_resolve.comp.hlsl`) · `vb_shade{,_tex,_froxel,_tex_froxel}`
(4, from `vb_shade.comp.hlsl`) · `vb_shade_split{,_tex,_hwrt,_tex_hwrt}` (4, from
`vb_shade_split.comp.hlsl`).

| | `-D SV0=1` | runtime gate (chosen) |
|---|---|---|
| new `.spv` / manifest rows / embeds+accessors / pipelines+selector arms | **+10 / +10 / +20 / +10** | **0 / 0 / 0 / 0** |
| existing `.spv` re-pinned | 0 | 10, once |
| **orthogonal axes in the VB matrix** | **4** (tex × froxel × hwrt × sv0) | **3** |
| OFF-path inertness | free, by preprocessor | **must be proven** (§3.3) |

The decisive term is the axis count. The matrix is **multiplicative**: the split tail already ships
4 `.spv` from two axes; a fourth axis makes the *next* VB tail feature cost 40 variants, not 20.
`-D` buys inertness once and taxes every future VB tail feature forever. **Decision: runtime gate,
conditional on rung S0's harness proving red-capable.** If S0's sensitivity sub-assertion fails, the
design falls back to `-D` and the +10 becomes an owner VALUES call (§7 clause 1).

### 3.3 How OFF-path inertness is proven — two instruments, both constructible

**What Rev 1 proposed, and why it is withdrawn (P0-1).** Rev 1's gate assumed a phi at the `gLit`
store whose predecessor edge means "SV0 not taken". There is none. The `vis` combine is inside
`for (uint i = 0u; i < H.l0a_count; ++i)` (`vb_resolve.comp.hlsl:301`) nested in
`if (!primary_dir_seen)` (`:309`); the same shape holds at `vb_shade.comp.hlsl:468/476`. `ao_final`'s
combine sits *before* that loop (`:288`) with its consumers *inside* it (`:321`, `:325`). And the
store's operand is the loop-carried accumulator `lit_direct` (`vb_shade.comp.hlsl:582`).
Independently, a Merkle hash makes an `OpPhi` a function of **both** incoming values, so every value
downstream of an SV0 merge hashes differently **by construction**. Recovering the OFF value needs a
path-projected program slice, not a per-result-id hash — materially beyond the budgeted cost, and it
would also fire on benign restructuring (CSE hoisted across the branch), i.e. an over-strict gate
that gets weakened until vacuous. **Withdrawn in full.**

**G1 — the kill-switch compile (static, `dxc`-only, red-capable).** Every SV0 span in every touched
source is wrapped in `#ifndef VB_SV0_KILL … #endif`. The gate compiles each of the 10 rows under its
own frozen `-D` set **plus** `-D VB_SV0_KILL=1` into a temp dir — **never committed**, so zero new
manifest rows, zero embeds, zero pipelines — and compares the sha256 against the **pre-SV0 committed
bytes**, pinned as 10 literals in the test at S2 under the "MEASURED — do not edit these literals to
make a failing run pass" discipline. Anyone can re-derive them: `git show <S2^>:<path> | sha256`.

* **Built from helpers that already exist**: `redxc_with_defines`
  (`crates/boyko_rhi_vulkan/tests/cluster_cull_spv_sync.rs:73-86` — writes to `temp_dir`, *"Never
  overwrites a committed artifact"*) and `assert_spv_byte_identical` (`:89-103`).
* **Proves:** nothing outside the SV0 guards changed, and the guards are complete.
* **Red mutation:** move one SV0 statement outside a guard → the kill compile differs → red.
* **What G1 does NOT prove, stated plainly:** that the *shipped* module's `sv0_mode == 0` execution
  is bit-identical to frozen. DXC optimises the SV0-bearing module as a whole (the frozen recipe
  passes no `-O`; DXC defaults to `-O3`). G1 alone would be the same class of error Rev 1 made — a
  gate that cannot go red for the failure it exists to catch, in a new costume.

**G2 — the executing OFF-path golden, with a DEMONSTRATED sensitivity control.** With SV0 compiled
in and `sv0_mode = 0`, every OFF-configuration golden must be byte-identical to its `PINS.toml` pin.
The objection to an 8-bit hash is that it is sub-LSB blind. Rev 2 does not argue past that — it
**tests** it:

* An **uncommitted** `-D VB_SV0_ULP_PROBE=1` compile perturbs the final `lit` by exactly one ULP
  immediately before the `gLit` store. Rendering the same fixture with it must produce a
  **different** hash.
* **RED if the hashes are equal.** Then G2 is blind at this frame size, OFF-path inertness has no
  executing proof, and the stage escalates to §7 clause 1.
* Why the control is expected to fire is *structural*, and is exactly what the control tests rather
  than assumes: the probe perturbs every covered pixel, and the covered-channel count of a full
  frame is large, so at least one 8-bit code crossing a quantisation boundary is the overwhelmingly
  likely outcome. No number for that is stated here.

**G1 ∧ G2 is the gate.** G1 bounds the *textual* blast radius; G2 executes the real pipeline and has
a demonstrated red. Neither is claimed to be a proof of universal bit-identity, and §8-R2 records the
residual honestly.

### 3.4 The withdrawn hazard (P0-2)

Rev 1's headline claim — that making `ao_final` a phi de-folds `spec_ao`'s `x - 1.0 + 1.0` and moves
the OFF path by 1 ULP — **does not survive checking, and is withdrawn**, along with the duplicated-
recompute construction it motivated and S2's "decisive" second mutation. Three independent grounds:

1. **Analytic + numeric.** `NoV = max(dot(n,v), 1e-4)` (`vb_resolve.comp.hlsl:258`) and
   `roughness = clamp(m.mrr.y, 0.045, 1.0)` (`:262`) force `pow(NoV + 1, exp2(-16r - 1)) ≥ 1`; the
   subtraction is Sterbenz-exact on `[1,2]`; both forms produce bit-identical `1.0`. A sweep over
   the reachable domain found zero mismatches.
2. **The construction already ships.** `vb_shade_split.comp.hlsl:457-461` performs
   `ao_final = min(ao_final, ssao_blurred)` inside a **runtime** `if (ssao_mode != SSAO_MODE_OFF)`,
   feeding the identical `spec_ao` at `:462`, in **all four** split `.spv`. Deferred does the same
   (`deferred_pbr.hlsl:948/966/977`). The shape Rev 1 called a novel hazard is blessed production
   code with green goldens.
3. **It is a no-op on the TEXTURED producers regardless** — `ao_final` is already a runtime value
   there (`vb_shade.comp.hlsl:450`, `vb_shade_split.comp.hlsl:453`).

Counting exactly: of the 10 producers, only **4** carry `ao_final` as a compile-time literal at all
(`vb_resolve{,_froxel}`, `vb_shade{,_froxel}`); the other 6 are already runtime.

**No replacement hazard is invented.** Rev 2's binding rule is that a named hazard must be
demonstrated; none is currently demonstrable, so §3.3's instruments stand as generic inertness
gates, not as remedies for a specific defect.

---

## 4. The march

### 4.1 The leaves — one shared header, not three copies (P1-2)

**Correction of fact.** Rev 1 claimed both leaves are eDSL-generated. Only one is:

| leaf | generated? | evidence |
|---|---|---|
| `sdf_soft_shadow_ranged` | **YES** | `// === GENERATED sdf_soft_shadow_ranged BEGIN/END ===` at `deferred_pbr.hlsl:514`/`:532`; generator `emit_hlsl_sdf_soft_shadow_ranged` (`boyko_shaderdsl/src/emit/shaders.rs:903`); pin `sdf_soft_shadow_ranged_matches_edsl_emit` (`tests/sdf_field_edsl_sync.rs:443`). |
| `sdf_ao` | **NO** | `sdf_gbuffer_composite.hlsl:532-541` carries **no** sentinels (unlike `sdf_soft_shadow` directly above at `:502`/`:525`); there is **no** `emit_hlsl_sdf_ao` anywhere in `crates/boyko_shaderdsl/`; `sdf_field_edsl_sync.rs` has no `sdf_ao` test. |

**Consequence, absorbed rather than papered over.** Naively, SV0 would create three hand-authored
copies of `sdf_ao` with no generator to re-emit from, plus three more of
`sdf_soft_shadow_ranged` — which lives in `deferred_pbr.hlsl:514-532`, **not** a shared `.hlsli`.
Six copies, and S3's "extend the eDSL sync test" mechanism does not exist for the AO half. That is a
live tension with CLAUDE.md's "HLSL the eDSL owns is generated, never hand-edited" — `sdf_ao` is
HLSL the eDSL does **not** own, and Rev 2 does not pretend otherwise.

**Decision: a new shared `crates/boyko_rhi_vulkan/shaders/sdf_shadow_leaves.hlsli`** carrying
`sdf_soft_shadow_ranged` (moved verbatim from `deferred_pbr.hlsl:506-532`, sentinels included),
`sdf_ao` (copied from `sdf_gbuffer_composite.hlsl:528-541`), and the AO consts (`AO_STEP`,
`AO_FALLOFF`, `AO_STRENGTH` — `sdf_gbuffer_composite.hlsl:488-490`). Include contract, documented in
its header: `field_distance` plus `MAX_IT`/`SHADOW_K`/`SHADOW_MINT`/`SHADOW_MINT_STEP`/
`SHADOW_HIT_EPS`/`FIELD_LIPSCHITZ_L` must be in scope before the `#include`.

* `deferred_pbr.hlsl` replaces its `:506-532` span with the `#include` at the same point. Moving
  text into an include preprocesses to the same token stream, so **every `deferred_pbr` `.spv` stays
  byte-identical** — and that is S2 gate (c), a red-capable check, not a hope.
* The three VB tails `#include` it after `sdf_field.hlsli`.
* `sdf_soft_shadow_ranged_matches_edsl_emit` re-targets the new header (a one-line path change in
  the test). The generator stays the single source for the shadow leaf.
* **`sdf_gbuffer_composite.hlsl` is NOT touched.** Its `sdf_ao` remains the second (and last) copy.
  Rationale: the marcher is the frozen-`.spv` blast radius this campaign explicitly protects, and
  collapsing 2 copies → 1 does not justify re-DXC'ing every marcher variant. The pair is pinned
  mechanically by **`sdf_ao_body_matches_shared_header`** (S3), a cross-file textual-identity test
  whose red condition is "the two bodies diverged" — a different, weaker mechanism than an eDSL
  re-emit pin, and labelled as such.

Net: `sdf_ao` copies 4 → **2**, both pinned; `sdf_soft_shadow_ranged` copies 4 → **1**, generator-
pinned. Shadow consts (`deferred_pbr.hlsl:466-474`, including `SHADOW_NORMAL_BIAS` at `:474`) are
pinned by S3's `sv0_consts_match_deferred_and_marcher`, never by this prose.

### 4.2 Shadow-origin bias from the GEOMETRIC face normal

`vb_geom_fetch` already holds the three world-space triangle vertices in registers
(`vb_geom_fetch.hlsli:536-538`), so the geometric face normal costs one `cross` + one `normalize`
and **no extra memory traffic**:

```hlsl
float3 fn = cross(world_p1 - world_p0, world_p2 - world_p0);
float  l2 = dot(fn, fn);
// Degenerate-triangle guard: fall back to the interpolated normal rather than normalize(0) -> NaN.
float3 face_n = (l2 > FACE_N_EPS2) ? (fn * rsqrt(l2)) : normalize(result.world_normal);
// Winding-independent orientation: agree with the shading normal.
result.face_normal = (dot(face_n, result.world_normal) < 0.0) ? -face_n : face_n;
```

**Why geometric.** `cross(p1-p0, p2-p0)` is computed from *actual world positions*, so it is the true
plane normal under **any** affine instance transform. The interpolated normal is `mul(m3, n)` with
the plain linear 3×3 and **no inverse-transpose correction** — `vb_geom_fetch.hlsli:539-542`
documents this as a known limitation, correct only for uniform scale. Using the geometric normal for
the origin lift also removes the classic silhouette acne.

**Stated plainly: SV0's correctness is scoped to uniform instance scale.** The *bias direction* is
robust; the *shading* normal that drives `NoL`, the BRDF, and the AO ray direction is still the
plain-`m3` one, inheriting the limitation verbatim. Fixing that is the inverse-transpose rung
`vb_geom_fetch.hlsli:539-542` already defers.

**Deviation from Deferred, acknowledged:** the Deferred mesh arm lifts along the *shading* normal
(`sdf_gbuffer_composite.hlsl:1877`). SV0's term is therefore not bit-comparable to Deferred's even in
principle — already true for unrelated reasons (§1.2). **AO uses the shading normal**, verbatim per
Deferred (`:1881`), and takes **no bias**: taps start at `h = AO_STEP` (`:536`), already off-surface.

### 4.3 The `#ifdef VB_SV0` source-guard — and what it buys

`vb_geom_fetch.hlsli` is included by **four** sources (verified by grep, P2-1): `vb_geo.comp.hlsl:118`,
`vb_resolve.comp.hlsl:85`, `vb_shade.comp.hlsl:90`, `vb_shade_split.comp.hlsl:137`.
(`vb_shadow_vis.comp.hlsl` does **not** include it. Rev 1 said five.) `vb_geo` ships
`vb_geo.comp.spv` and `vb_geo_mv.comp.spv` and does not need a face normal, so:

```hlsl
#ifdef VB_SV0
    float3 face_normal;   // in VbGeomFetchResult
#endif
```

where `VB_SV0` is a **source-level `#define` written by each of the three tails before the
`#include`** — never a `-D` on the dxc command line, so it creates **zero** new compile variants.
`vb_geo.comp.hlsl` does not define it and therefore preprocesses **character-identical** to today,
keeping its two `.spv` **byte-identical by construction**. This is the frozen-base discipline
`deferred_pbr.hlsl:74-79` documents for `TERMINATOR_WRAP`, and it is a gate that can go red: delete
the guard and those two `.spv` change bytes (S2 gate (b)).

### 4.4 Termination, bounds, and why no device hang is possible

The march is `[loop] for (uint i = 0u; i < MAX_IT; ++i)` (`deferred_pbr.hlsl:519`) with
`MAX_IT = 128u` (`:468`) — a hard cap on **every** path. Beyond that, `t` strictly increases by at
least `SHADOW_MINT_STEP` every iteration, because the advance is
`t + max(d / FIELD_LIPSCHITZ_L, SHADOW_MINT_STEP)` (`:525`):

* `d` negative (inside the field) → `max` returns `SHADOW_MINT_STEP`.
* `d` huge (empty field, `FAR = 1.0e9`, `sdf_field.hlsli:41`) → `t` overshoots `t_max` → `break`.
* **`d` NaN** → HLSL `max` lowers to `GLSL.std.450 NMax`, which returns the **non-NaN** operand →
  `SHADOW_MINT_STEP`. The march still advances and still terminates.

**Structural bounds per covered mesh pixel** (from the loop form, not measured): `≤ MAX_IT` field
evaluations for the shadow plus exactly 5 for AO (`sdf_gbuffer_composite.hlsl:535`), and **exactly
one march per pixel** regardless of light count (§1.2). Each `field_distance` walks
`min(Buf[0], MAX_SDF_EDITS)` edits (`sdf_field.hlsli:204`) — **already clamped**, so SV0 introduces
**no new indexing and no new out-of-range surface**. That matters because `robustBufferAccess` is OFF
and there is no GPU-assisted validation: an out-of-range access is real UB nothing reports.

**Empty-edit-list behaviour (exact).** With `Buf[0] == 0` the loop at `sdf_field.hlsli:206-215` never
executes and `acc` stays `FAR`. Then `res = min(1.0, SHADOW_K*FAR/t) = 1.0`; the first `t` advance
overshoots `T_MAX`; the leaf returns `clamp(1.0, 0, 1) = 1.0` after **one** iteration. AO: every tap
deficit is `(h − FAR)`, so `1 − AO_STRENGTH*occ` saturates to exactly `1.0`. Both terms are **exactly
`1.0`**, and `min(x, 1.0)` is the bit-exact identity for every finite non-NaN `x`.
**Consequence, and it is a trap:** arming SV0 on any empty-edit-list scene is byte-identical — which
makes such a scene a *vacuous* arming gate. Rung S1 exists because of this.

### 4.5 Falloff, and the combine

Quilez basic soft-shadow: `res = min(res, SHADOW_K * d / t)` (`deferred_pbr.hlsl:521`),
`SHADOW_K = 8.0` (`:469`). No sqrt, no cone — deliberately, to keep the FP-parity surface minimal
against the host oracle. Both terms combine by `min`:

```hlsl
vis      = min(vis,      sv0_shadow);   // at the primary-directional site
ao_final = min(ao_final, sv0_ao);       // before spec_ao — the SAME shape vb_shade_split.comp.hlsl:460 already ships
```

`min` on floats is exact and commutative/associative for non-NaN, so SV0's combine is
**order-independent** with respect to the existing CSM combine, the SSAO combine
(`vb_shade_split.comp.hlsl:460`), and the split tail's HWRT denoised-visibility combine (`:51-54`).
Under NaN, `NMin` returns the non-NaN operand — a degenerate term **cannot** poison the pixel; it
degrades to "no SV0 contribution". That is the correct failure direction and is asserted in S3.

---

## 5. Variant matrix and gates

### 5.1 `.spv` created: ZERO. `.spv` perturbed: exactly 10.

The 10 rows are re-DXC'd and re-pinned **once**, at S2, with **no change to their `-D` combinations**
and **no new manifest rows** — each existing row in `docs/SHADER-VARIANT-MANIFEST.md:91-107` gains
one sentence noting the SV0 binding-10 interface delta. `vb_geo.comp.spv`, `vb_geo_mv.comp.spv`, and
every `deferred_pbr` `.spv` are **byte-identical**, and each of those is itself a gate.

Interface delta for all ten: `+ StructuredBuffer<uint> Buf @10 (register t0, space 0)`. Set 0 widens
to 9 entries (`vb_layout0`) or 11 (`vb_layout0_froxel`).

### 5.2 Gates and their skip behaviour

| gate | mechanism | can it skip? |
|---|---|---|
| G1 `vb_sv0_kill_switch_byte_identity` | `redxc_with_defines` + sha256 vs 10 pinned pre-SV0 hashes | yes, if `dxc` absent |
| G2 `vb_sv0_off_path_golden` + its ULP-probe control | executing render vs `PINS.toml` | probe compile needs `dxc`; the golden half does not |
| `vb_geo_spv_unperturbed`, `deferred_pbr_spv_unperturbed` | `assert_spv_byte_identical` (`cluster_cull_spv_sync.rs:89-103`) | yes, if `dxc` absent |
| `vb_sv0_spv_sync` (all 10 rows) | extends `tests/vb_froxel_spv_sync.rs` | yes, if `dxc` absent |
| `spirv-val` clean, all 10 | pinned SDK `spirv-val` | yes |
| image goldens | `goldens/PINS.toml` sha256 | no |

Rev 1 claimed one gate "can never skip". That claim went with the withdrawn instrument: **every
static gate here needs `dxc`** (`cluster_cull_spv_sync.rs:196-204` is the skip shape). Mitigation is
procedural and already used by this campaign: **a rung is not commit-eligible until its `dxc`-
dependent gate has been *run* and its output pasted into the commit message.** A gate proven only on
a box that skipped it is not a gate.

### 5.3 The `PINS.toml` contradiction — a blocking precondition, not an inheritance (P1-5)

`goldens/PINS.toml:293-297` states the `[vb_mesh]` `sha256_*` values are *"UNBLESSED placeholders —
NOT real hashes"*, while `PINS.toml:15` establishes `"PENDING"` as the unblessed sentinel (the
`[vb_mesh]` values are not `"PENDING"`), and `[vb_both]:326-328` treats the same value as live
(*"the orchestrator VERIFIES the equality"*). **This document cannot resolve which is true from the
file alone, and does not guess.**

Two consequences, both binding:
1. **No SV0 gate duplicates the literal.** Every gate reads the pin through
   `scripts\golden.ps1 -Pin <name>` / its `Read-Pins` reader. A hand-copied hash is exactly the
   failure mode `PINS.toml:1-5` was created to end.
2. **Rung S1 carries a precondition:** the `[vb_mesh]` comment block is reconciled before any SV0
   gate depends on it — either the stale paragraph is deleted because the value was blessed, or the
   value is reset to `PENDING` and re-blessed under the standard flow. S1 is not commit-eligible
   until one of the two has happened.

---

## 6. Rungs

Ladder: **cheap harness falsifier → fixture → cost falsifier → dark infra → device oracle → arm →
measure.** Each rung is independently committable, has **one** gate, and names the mutation that
turns it red. *A mutation that is only argued does not count; the commit message records the mutated
run's output.*

### S0 — the OFF-path harness (CPU + `dxc`, no shader edit)

**Lands:** `crates/boyko_rhi_vulkan/tests/vb_sv0_offpath.rs`, wiring the **existing**
`redxc_with_defines` / `assert_spv_byte_identical` / `find_dxc` helpers. No new SPIR-V parser, no
production code. ~80 LOC.

**Gate — the harness is validated BEFORE it is believed:**
1. **reproduction:** each of the 10 rows re-DXC'd under its frozen recipe is byte-identical to its
   committed `.spv`.
2. **sensitivity:** copy one tail to a temp dir, change **one float literal** in a region SV0 will
   never touch (e.g. `vb_resolve.comp.hlsl:258`'s `1e-4`), re-DXC, assert the bytes **differ**.

**RED if:** (2) reports equal, or (1) fails for any row (the frozen recipe no longer reproduces —
that is a pre-existing defect this rung surfaces before SV0 builds on it). On (2) failing, the
runtime-gate decision loses G1 and the rung **escalates**: `-D SV0=1` as an owner VALUES call, or
abort (§7).

**Skip policy:** needs `dxc`; not commit-eligible until run with output pasted into the commit
message.

### S1 — the fixture (host test only, no shader edit) — **BLOCKING**

**Why it exists.** Every current VB golden has an **empty** SDF edit list: `PINS.toml:322`
(`vb_both`: *"boot-seeded EMPTY (count == 0)"*) and `:355` (`vb_sdf_only`, same). By §4.4 the SV0
term on such a scene is exactly `1.0` and byte-identity is *vacuous*. Arming against today's fixtures
would produce a green gate quantified over an empty selection — the campaign's #1 named defect.

**Lands:** `crates/boyko_app/tests/vb_both_sdf.rs` — a clone of `vb_both.rs` with SDF primitives
actually spawned (edit list gathered by `collect_sdf_edits`, `boyko_app/src/runner.rs:589`),
positioned so at least one SDF body occludes the five-sphere scene's key light and sits near a mesh
surface. Plus a `[vb_both_sdf]` block in `goldens/PINS.toml` seeded `PENDING`.

**Gate — fixture ADEQUACY FOR SV0, not "the frame differs" (P1-1).** Under `VB × Both` a non-empty
edit list makes `sdf_forward_march` composite SDF-owned pixels into `gLit` *independently of SV0*, so
"the frame differs from the `[vb_mesh]` pin" goes green the moment any SDF body is visible — including
one casting on nothing. The gate is therefore a **CPU-side check against the host `boyko_shaderdsl`
`Eval` oracle** over the fixture's camera + edit list, with no GPU:

1. `edit_count > 0`;
2. **≥ `SV0_MIN_SHADOWED_PIXELS` covered mesh pixels** `P` (raster-covered per the fixture's own
   camera and mesh set) satisfying **both** (a) front-facing to the key light,
   `dot(N, L) > SHADOW_NDOTL_EPS` — otherwise `sdf_soft_shadow` returns `0.0` at its early-out
   (`sdf_gbuffer_composite.hlsl:499-501`) for a reason unrelated to the field; and (b) the ray from
   `P + face_N * SHADOW_NORMAL_BIAS` along `L` reaching a field distance below `SHADOW_HIT_EPS`
   within `T_MAX`, i.e. the leaf would return `< 1.0`;
3. the ≥2-frame `SdfEditStaging::is_dirty()` assertion from §2.4's re-sited R11 tripwire.

**RED if:** (1) or (3) fails, or (2)'s count falls below the floor. **Mutation:** remove the SDF
spawns → (1) and (2) both fail. This is the control that proves the input reaches the thing under
test.

**Precondition:** §5.3's `PINS.toml` reconciliation.

### S1.5 — the cost falsifier (measurement, zero new shader code) — can kill the stage

**Lands:** a bench in `crates/boyko_app/tests/` running the **shipped Deferred** path on S1's scene,
performing an **interleaved paired A/B** of `pc.lighting_flags` with `SHADOWS|AO` set vs cleared —
exactly the `sdf_gbuffer_composite.hlsl:1865` gate around the term SV0 will inline. Protocol,
non-negotiable (the VB-P1d lesson): interleaved pairs, warmup discarded, ≥30 pairs, **median paired
delta** + run-to-run spread across **3 sessions**. Sequential before/after measured a phantom
regression on this hardware that was entirely session drift.

**Gate:** relative spread of the median across the 3 sessions ≤ 10%. The value is recorded as
`SV0_DEFERRED_TERM_REFERENCE` in the test, under the "MEASURED — do not edit these literals to make
a failing run pass" discipline.

**RED if:** the spread exceeds 10% — the instrument is not trustworthy at this scale and §7's ABORT
clause cannot be adjudicated. **Mutation:** point the A/B at two *identical* configurations → the
median paired delta must fall to ~0; if it does not, the harness measures drift, not the term.

*Why this can kill the stage before a line of shader is written:* it measures the exact term, on the
exact fixture, using only shipped code. Under VB every covered pixel is a mesh pixel, so SV0's
coverage is a superset of Deferred's `!own_pixel` arm.

### S2 — dark infra (SV0 compiled in, host writes mode 0)

**Lands:**
1. New `shaders/sdf_shadow_leaves.hlsli` (§4.1); `deferred_pbr.hlsl:506-532` replaced by the include.
2. `vb_geom_fetch.hlsli`: `#ifdef VB_SV0` `face_normal` field + computation (§4.2, §4.3).
3. All three tails: `#define VB_SV0` before the include; `[[vk::binding(10)]] Buf : register(t0)`;
   `#include "sdf_field.hlsli"` **after** `Buf`, then `sdf_shadow_leaves.hlsli`; the shadow const
   block; `load_vb_sdf_mesh_mode` hoisted once per pixel; the guarded block, every SV0 span wrapped
   in `#ifndef VB_SV0_KILL` and the ULP probe in `#ifdef VB_SV0_ULP_PROBE`.
4. `light_table.hlsli`: `load_vb_sdf_mesh_mode` + the two bit constants.
5. `boyko_render::light`: `VB_SDF_MESH_MODE_SHIFT/_MASK`, two `LightingConfig` fields (default
   **false**), packing in `shadow_gate_word`, bit-position `debug_assert_eq!`.
6. `crates/boyko_app/src/gpu_scene/mod.rs:3395` and `:4425`: binding-10 entries.
7. `crates/boyko_rhi_vulkan/src/present/targets.rs:2995 / 3090 / 3193 / 3311`: `scene.edit_list` at
   slot 10.
8. Re-DXC + re-pin all 10 `.spv`; manifest notes; the 10 pre-SV0 sha256 literals for G1.

**Gate (one, six indivisible parts):**
(a) **G1** kill-switch byte-identity for all 10 rows against the pinned pre-SV0 hashes;
(b) `vb_geo.comp.spv` / `vb_geo_mv.comp.spv` byte-identical;
(c) every `deferred_pbr` `.spv` byte-identical (the §4.1 header move);
(d) every VB image golden byte-identical with `sv0_mode = 0`;
(e) `spirv-val` clean on all 10;
(f) **G2's ULP-probe control demonstrated RED** — the probe compile's render differs from the pin.

**RED if / mutations (DEMONSTRATED):**
* (a): move one SV0 statement outside its `#ifndef VB_SV0_KILL` guard → G1 red.
* (b): delete `#ifdef VB_SV0` from `vb_geom_fetch.hlsli` → `vb_geo.comp.spv` bytes change → red.
* (c): place the `#include` at a different point in `deferred_pbr.hlsl` than the moved span occupied
  → `deferred_pbr` `.spv` bytes change → red.
* (f) is itself the demonstration that (d) is not a blind gate.

### S3 — the device oracle (still mode 0 in production)

**Lands:** four verification layers, no production behaviour change.

1. **Span pins** — extend `tests/sdf_field_edsl_sync.rs`: `sdf_soft_shadow_ranged_matches_edsl_emit`
   re-targets `sdf_shadow_leaves.hlsli`; the header + the const block + the `Buf @ t0` precondition
   are `.contains()`-asserted in **all three** tails.
2. **`sdf_ao_body_matches_shared_header`** — cross-file textual identity between
   `sdf_gbuffer_composite.hlsl:532-541` and the shared header's copy. Weaker than an eDSL re-emit
   pin (there is no generator, §4.1) and labelled as such; its red condition is "the two bodies
   diverged".
3. **Leaf bit-exactness on device** — the leaf probe (`cpu_gpu_sdf_agreement.rs` family) evaluates
   both leaves over a ≥4096-sample `(P, N, L)` fixture and compares **bit-exactly** to the host
   `boyko_shaderdsl` `Eval` backend. Pure SSBO math, no sampler → demand exact `f32::to_bits()`
   equality, not a ULP tolerance.
4. **Host unit tests for the face normal** — under a non-uniform-scale affine, `face_normal` equals
   the analytic plane normal; under uniform scale it agrees in sign with the interpolated normal; a
   degenerate (zero-area) triangle returns the interpolated fallback, never NaN. Plus the §4.5 NaN
   assertion: a NaN term degrades to "no contribution".

**RED if / mutations:** (1) change one token in **one** tail's asserted span → that tail's pin fails.
(2) perturb the marcher's `sdf_ao` body → the cross-file pin fails. (3) perturb one host const in the
`Eval` backend → bit-exactness fails. (4) reverse the `cross` operand order without the orientation
fix → the sign test fails.

### S4 — arm

**Lands:** the host resolver sets word-7 bits 5..6 by **consuming the already-resolved bit** —
not by mirroring its derivation (P1-3):

```rust
// The bit is resolved ONCE at render_path_config.rs:904 as
//   sdf_leg && consumers.sdf_shadows_wanted && !consumers.hwrt_denoise_or_vis_on
// and stored in ResolvedRenderPath::shadow (:526). SV0 READS it; it does not re-derive it.
let sv0 = resolved.path_is_vb()
    && resolved.shadow.contains(ShadowSources::SDF_SOFT_MARCH)
    && mesh_leg;
```

*(Rev 1 cited `render_path_config.rs:727` for the predicate; that line is a doc comment on
`RenderPathDegrade`'s reason log. Rev 1 inherited the number from `docs/RENDER-PARITY-PLAN.md:413`
without opening the file. `ResolvedRenderPath::sdf_mesh_shadow` — which the critique pointed to —
**does not exist**: `RENDER-PARITY-PLAN.md:236/:324` are proposed code for the never-implemented SF0,
and `sdf_mesh_shadow` greps to doc files only. The field to consume is `shadow`.)*

*Checked, because it would have made the whole rung dead: `cap_vb_v1_consumers`
(`render_path_config.rs:1105-1170`) does **not** cap `sdf_shadows_wanted`, so `SDF_SOFT_MARCH`
survives under VB×Both.*

Owner-facing toggle via `LightingConfig`.

**Gate — the arming control, quantified over ALL 10 shipping variants (P0-3).** Rev 1 gave an
executing gate to 2 of 10 and a `.contains()` text pin to the rest; a text pin proves the source is
present, not that it executes — a wrong `sv0_mode` decode, a wrong bias site or a wrong combine
operand passes it green. That is the campaign's own defect class one level down.

Verified host reachability of each variant (`present/passes/vb.rs:883-1003` — the lit-producer choice
is **three-way**: `path_vb_split()` displaces both others; otherwise `vb_use_classified` selects
`vb_shade*`, else `vb_resolve*`; within `vb_shade*` the `(textured, froxel)` match at `:950-980`
picks the row):

| # | `.spv` | armed by | SV0-armable? |
|---|---|---|---|
| 1 | `vb_resolve` | default VB×Both, flat materials | yes |
| 2 | `vb_resolve_froxel` | + `clusters_wanted` | yes |
| 3 | `vb_shade` | `BOYKO_VB_FORCE_CLASSIFIED=1` | yes |
| 4 | `vb_shade_tex` | a textured material (auto — `vb_use_classified = force \|\| vb_tex_active`) | yes |
| 5 | `vb_shade_froxel` | force-classified + `clusters_wanted` | yes |
| 6 | `vb_shade_tex_froxel` | textured + `clusters_wanted` | yes |
| 7 | `vb_shade_split` | `ssao_on` (arms `mesh_geo_shade_split`, `render_path_config.rs:854`) | yes |
| 8 | `vb_shade_split_tex` | `ssao_on` + textured | yes |
| 9 | `vb_shade_split_hwrt` | `ssao_on` + `hwrt_denoise_or_vis_on` | **NO — structurally** |
| 10 | `vb_shade_split_tex_hwrt` | as 9, + textured | **NO — structurally** |

Rows 9–10 are **not** a coverage gap: `SDF_SOFT_MARCH` requires `!consumers.hwrt_denoise_or_vis_on`
(`:904`), which is exactly the condition those pipelines require to be **true**. SV0 can never be
armed while they are bound. They therefore get the OFF-path gate (G1/G2) plus a **CPU resolver
truth-table test**, `sv0_never_arms_under_hwrt`, extending the family at `:2283-2301` — a stronger
instrument than any text pin, and it needs no GPU.

*The reachability channel for rows 3/5 is not invented here:* `BOYKO_VB_FORCE_CLASSIFIED` already
exists and `present/scene_types.rs:2336-2338` documents it as *"the orchestrator's channel to
exercise `vb_shade` on real hardware"*.

**The gate, per variant:**
* **(i) no spurious perturbation** — with SV0 compiled in and `sv0_mode = 0`, every OFF-configuration
  golden is byte-identical to its `PINS.toml` pin. **Scope of the claim (P1-4):** the assertion is
  that the *shipped, SV0-compiled* module executing `sv0_mode == 0` produces the same 8-bit frame as
  the frozen module. It is **not** a claim of universal FP identity — C2 removed the second `spec_ao`
  site that would have made it one, so no independently-compiled duplicate exists in Rev 2. The
  claim's sensitivity is the demonstrated ULP probe (S2 gate (f)), not an assumption.
* **(ii) the input reached, per armable variant** — for each of rows 1–8, with that variant's knobs
  set and SV0 armed, `vb_both_sdf` **differs** from its own `sv0_mode = 0` render, in a changed-pixel
  count within `[1%, 60%]` of covered mesh pixels.
* **(iii) rows 9–10** — `sv0_never_arms_under_hwrt` green.
* **(iv)** owner visual eval on the dumped BMP before any hash is blessed.

*(ii) is used only to assert "the term reached", never to judge quality — image statistics lie about
render quality; the correctness verdict comes from S3's oracle and (iv).*

**RED if / mutations (DEMONSTRATED).** The SV0 span is common to a source's variants by construction
(it sits outside every `-D` guard), so the *mutation* partition is by source — three mutations
covering the ten rows, each reddening **exactly** its own rows:
* Revert only `vb_resolve.comp.hlsl`'s SV0 block → rows 1–2 red, 3–8 green.
* Revert only `vb_shade.comp.hlsl`'s → rows 3–6 red, others green.
* Revert only `vb_shade_split.comp.hlsl`'s → rows 7–8 red, others green. *(This is the structural
  closure of the three-tails hole: the split path cannot be silently left out.)*
* Force `sv0_mode = 0` host-side → every row's (ii) count falls to 0.

### S5 — measure

**Lands:** an interleaved paired A/B of the VB lit-producer dispatch, SV0 armed vs `sv0_mode = 0`, on
`vb_both_sdf`, at 512×512, same protocol as S1.5 (≥30 pairs, warmup discarded, 3 sessions, median
paired delta + spread). Results pinned as literals in the test. Run on **row 1** (fused) and **row 7**
(split) — the two structurally different tails.

**Gate:** reproducible (spread ≤ 10%) **and** adjudicated against §7.

**RED if:** the spread exceeds 10% — the number is not decidable at this scale (the VB-P1d precedent:
a single-sample ≤5% gate on a bench with 21% run-to-run spread is not decidable, and shipping one
manufactures confidence).

---

## 7. ABORT criteria

The stage is **reverted** — not softened, not re-scoped mid-flight — if any of:

1. **The OFF-path proof fails to exist.** S0's sensitivity sub-assertion reports equal (G1 is blind),
   **or** S2 gate (f)'s ULP probe fails to red (G2 is blind) — **and** the owner declines the `-D`
   fallback's +10 `.spv`. Without one of them, OFF-path inertness is unprovable and 10 re-pins ship
   on faith.
2. **S1's fixture cannot be made adequate** — no `VB × Both` configuration yields
   `SV0_MIN_SHADOWED_PIXELS` pixels satisfying S1's oracle predicate with a non-empty edit list. Then
   there is no scene in which SV0 can be observed and every downstream gate is vacuous by
   construction.
3. **Cost.** S5's median paired delta exceeds **2×** S1.5's measured `SV0_DEFERRED_TERM_REFERENCE` on
   the same fixture. The threshold is a ratio to a **measured sibling that already ships this visual
   at an accepted cost**, not a predicted number — the campaign's refuted-cost-model lesson. In
   `[1×, 2×]` it ships with the number recorded; above 2×, revert.
4. **S4 (i) cannot be made byte-identical.** With the ULP probe having demonstrated the gate is not
   blind, a persistent difference on an analytically-`1.0` term means the OFF path is not inert, and
   no amount of re-blessing fixes that.

Revert granularity: every rung is independently committable, so an abort at S5 reverts S2–S4 and
keeps S0 (the harness is reusable by every future runtime-gated shader feature) and S1 (the fixture
is a real coverage gain regardless).

---

## 8. Risks

Named first are the ones this campaign has actually hit.

| # | Risk | Precedent | Mitigation |
|---|---|---|---|
| R1 | **Vacuously-green gate** — assertion quantified over an empty selection. | Hit 3× in Stage 1. **Live here:** every VB golden has `edit_count == 0` (`PINS.toml:322`, `:355`). | S1 is blocking and its gate is an `Eval`-oracle adequacy check, not "the frame differs"; S4(ii) is quantified over **all 8** armable variants. |
| R2 | **OFF-path drift invisible to an 8-bit golden.** | Generic; Rev 1's specific instance was **refuted** (§3.4) and no replacement is invented. | G1 bounds the textual blast radius; G2 executes with a **demonstrated** 1-ULP red. **Residual, stated:** neither is a universal proof; a perturbation smaller than G2's demonstrated sensitivity would pass. |
| R3 | **Cost model instead of measurement.** | The refuted `a + b*(froxels*N)` model. | No predicted number in any gate. S1.5 and S5 are measurements; §7's threshold is a ratio to a measured sibling. |
| R4 | **Session drift read as a regression.** | The phantom regression on this hardware. | Interleaved paired A/B, warmup discarded, 3 sessions, spread reported — enforced in S1.5 and S5. |
| R5 | **Instrument that silently does nothing.** | The flat-curve knob. | S0 validates the harness against a deliberately mutated recompile before it is trusted; S1.5 has its own null-mutation control; S2(f) is G2's own control. |
| R6 | **Silent OOB with `robustBufferAccess` OFF.** | No layer reports it. | SV0 adds **no** new indexing — `Buf[0]` is already clamped by `min(Buf[0], MAX_SDF_EDITS)` (`sdf_field.hlsli:204`). Binding 10 is always a valid descriptor (`scene.edit_list` is non-`Option`). `MAX_IT = 128u` caps every march path including NaN (§4.4), so no device hang is possible. |
| R7 | **`NMin`/`NMax` NaN semantics.** | HLSL `min`/`max` → `NMin`/`NMax`, returning the non-NaN operand. | Exploited deliberately twice: guarantees march termination (§4.4) and makes a degenerate term degrade to "no contribution" rather than poison the pixel (§4.5). Asserted in S3(4). |
| R8 | **`sdf_ao` has no generator** — hand-authored HLSL the eDSL does not own, a live tension with CLAUDE.md's shader rule. | — | §4.1 cuts copies 4 → 2 via the shared header and pins the survivor pair with `sdf_ao_body_matches_shared_header`. The tension is **not** resolved and is stated, not hidden; writing a generator would perturb the frozen marcher `.spv` and is out of scope. |
| R9 | **A tail silently omitted.** | The P0-class hole this design closes. | S4's selection is all 10 variants with per-variant executing assertions; the three demonstrated revert-one-source mutations each red exactly their own rows. |
| R10 | **Non-uniform scale.** | `vb_geom_fetch.hlsli:539-542`. | Out of scope, stated plainly (§4.2). The bias is robust; the shading normal is not. |
| R11 | **Edit list becomes per-frame dirty** → a missing barrier under VB. | — | Re-sited out of `debug_assert!` (compiled out in release, where goldens run) into **test code**: S1's ≥2-frame `is_dirty()` assertion (§2.4). |
| R12 | **`dxc`-dependent gates skip.** | `cluster_cull_spv_sync.rs:196-204`. | §5.2 states honestly that **every** static gate needs `dxc` — Rev 1's "one gate can never skip" went with the withdrawn instrument. Procedural mitigation: no rung is commit-eligible until its `dxc` gate has been run and its output pasted into the commit message. |
| R13 | **`deferred_pbr` perturbed by the §4.1 header move.** | New this revision. | S2 gate (c): every `deferred_pbr` `.spv` byte-identical, with a demonstrated red (misplace the `#include`). |

---

## 9. Appendix — verified file:line anchors

Every line below was opened while writing **this revision**. Anchors Rev 1 asserted but did not
survive checking are marked **[Rev 1 WRONG]**.

**Field / leaves:** `sdf_field.hlsli:41` (`FAR = 1.0e9`), `:203-217` (`sdf`, edit-count clamp at
`:204`), `:246` (`field_distance` gateway) · `sdf_gbuffer_composite.hlsl:488-490` (AO consts),
`:498-526` (`sdf_soft_shadow`; generated span `:502`/`:525`), **`:532-541` (`sdf_ao` — hand-written,
NO sentinels)** [Rev 1 WRONG: claimed eDSL-generated, and gave `:532-540`], `:1853-1885` (Deferred
mesh arm), `:1876-1878` (shadow), `:1881` (AO).

**Binding precedent:** `deferred_pbr.hlsl:153-160` (the precedent text — it verifies **both** `t0`
free *and* binding 10 free), `:161` (the decl), `:466-474` (shadow consts; `SHADOW_NORMAL_BIAS` at
`:474`), `:479` (caster cap), `:506-532` (generated `sdf_soft_shadow_ranged`; `MAX_IT` loop `:519`,
advance `:525`, `t_max` break `:526`), `:74-79` (frozen-base `#ifdef` discipline) · AO routing:
`:797` (`ao = material_texel.g`), `:948` (`ao_final = ao`), `:966` (SSAO combine), `:977` (`spec_ao`).

**VB tails:** `vb_resolve.comp.hlsl:85` (geom-fetch include), `:123/127` (t5/u6), `:151-154`
(`#ifdef FROXEL` 8/9), `:167-168`, `:183-184` (Set-1 t12/t14), `:241` (sentinel), `:258` (`NoV`),
`:262` (`roughness` clamp), `:288-289` (`ao_final`/`spec_ao`), `:301` (the `l0a_count` loop),
`:308-315` (the primary-directional `vis` site), `:309` (`!primary_dir_seen`), `:321`/`:325`
(in-loop `spec_ao`/`ao_final` consumers) · `vb_shade.comp.hlsl:90` (include), `:167` (`gTextures[] :
register(t0, space3)`), `:450`/`:452` (TEXTURED/base `ao_final`), `:454` (`spec_ao`), `:468/475-482`
(loop + `vis` site), `:582` (the `gLit` store — a loop-carried accumulator) ·
`vb_shade_split.comp.hlsl:137` (include), `:204` (`gTextures[]` at t0 space3), `:453/455` (`ao_final`
seed), **`:457-461` (`ao_final = min(ao_final, ssao_blurred)` inside a RUNTIME `if` — the shipping
counter-example to Rev 1's R2)**, `:462` (`spec_ao`), `:51-54` (HWRT denoised combine) ·
`vb_geo.comp.hlsl:118` (include).

**Geometry fetch:** `vb_geom_fetch.hlsli:20-34` (Set-numbering deviation), `:533-538` (`m3`, world
positions), `:539-542` (**the plain-`m3` normal limitation**).

**Header word 7:** `light_table.hlsli:77/91/109/128/145` (bit decoders 0..4), `:154` ("Bits 5..7 stay
free"), **`:218-220` (`load_ssao_mode` reads `LightBuf[11]` — NOT word 7)** ·
`boyko_render/src/light.rs:386-409` (bit budget; "5..7 (free)" at `:406`) · `ddgi_config.rs:288-289`
(single-writer `debug_assert_eq!` idiom).

**Resolver:** `render_path_config.rs:447` (`SDF_SOFT_MARCH` declared), `:487` (`ResolvedRenderPath`),
`:508` (`mesh_geo_shade_split`), `:526` (`pub shadow: ShadowSources` — **the field to consume**),
`:854` (`mesh_geo_shade_split` derivation), **`:904-905` (the real `SDF_SOFT_MARCH` predicate:
`sdf_leg && consumers.sdf_shadows_wanted && !consumers.hwrt_denoise_or_vis_on`)**, `:907-908`
(`HWRT_VIS`), `:1105-1170` (`cap_vb_v1_consumers` — does **not** cap `sdf_shadows_wanted`),
`:2283-2301` (the `SDF_SOFT_MARCH` truth-table family) · **[Rev 1 WRONG] `:727` is a doc comment on
`RenderPathDegradeLog`, not a predicate.**

**Producer selection:** `crates/boyko_rhi_vulkan/src/present/passes/vb.rs:883-900` (the THREE-way
lit-producer choice), `:950-980` (the `(textured, froxel)` pipeline match), `:767-778` (classify
count dispatch) · `present/scene_types.rs:2329-2350` (`vb_use_classified = force ||
vb_tex_active_this_frame`; `BOYKO_VB_FORCE_CLASSIFIED` at `:2336`), `:2538-2543` (`vb_tex_active`),
`:2715-2722` (`path_vb_split`).

**Host layouts / sets / upload:** `crates/boyko_app/src/gpu_scene/mod.rs:3395-3459` (`vb_layout0`, 8
entries), `:4425-4492` (`vb_layout0_froxel`, 10 entries), `:3906-3920` (VB-P2 classify pipeline
build) · `crates/boyko_rhi_vulkan/src/present/targets.rs:2995-3079`, `:3090`, `:3193`, `:3311`,
`:1402` · `rhi_impl/mod.rs:93` (`MAX_BIND_GROUP_BINDINGS = 24`) · `boyko_app/src/runner.rs:589`
(`collect_sdf_edits`), **`:1182-1197` (the actual one-shot upload, inside the frame loop)** ·
**[Rev 1 WRONG] `:1136-1141` is the comment, not the upload.**

**Generators / gates:** `boyko_shaderdsl/src/emit/shaders.rs:903` (`emit_hlsl_sdf_soft_shadow_ranged`);
**no `emit_hlsl_sdf_ao` exists** · `tests/sdf_field_edsl_sync.rs:404` (`sdf_soft_shadow_matches_edsl_emit`),
`:443` (`sdf_soft_shadow_ranged_matches_edsl_emit`); **no `sdf_ao` test** ·
`tests/cluster_cull_spv_sync.rs:73-86` (`redxc_with_defines`, temp-dir output, never overwrites a
committed artifact), `:89-103` (`assert_spv_byte_identical`), `:196-204` (the skip path) ·
`tests/vb_froxel_spv_sync.rs`.

**Goldens:** `goldens/PINS.toml:15` (the `"PENDING"` sentinel), `:273-311` (`[vb_mesh]`; the
**stale-or-live** "NOT real hashes" paragraph at `:293-297`), `:288-291` (VB≠Forward FP-path note),
`:313-343` (`[vb_both]`; **empty edit list at `:322`**; `:326-328` treats the hash as live),
`:345-377` (`[vb_sdf_only]`; **empty at `:355`**).

**Manifest / stale row:** `docs/SHADER-VARIANT-MANIFEST.md:84` ("`vb_resolve.comp.hlsl` has no
TEXTURED variant"), `:91-107` (the VB table) · `docs/RENDER-PARITY-PLAN.md:236`/`:324` (**proposed**
SF0 code — `sdf_mesh_shadow` exists in no `.rs`), `:351` (the SV0 row), `:383-384` (the O3 sync-pin
discipline).

---

## 10. Answers to the critic's four open questions

**Q1 — If S0 is not constructible, where does the OFF-path proof go? Was the uncommitted third
compile considered before inventing the Merkle hash?**
It was not, and that was the error. Rev 2 adopts it as **G1** (§3.3), built from `redxc_with_defines`
and `assert_spv_byte_identical`, which already exist — S0 shrinks from a ~200 LOC SPIR-V parser to
~80 LOC of wiring. **But G1 alone is not sufficient**, and adopting only it would repeat Rev 1's
mistake in a new costume: it proves the *textual* OFF path is untouched, not that the shipped
module's `sv0_mode == 0` execution is bit-identical, because DXC optimises the SV0-bearing module as
a whole. Rev 2 therefore pairs it with **G2**, the executing golden gate, whose sub-LSB sensitivity
is *demonstrated* by an uncommitted 1-ULP probe compile (S2 gate (f)) rather than argued. `-D`
remains the §7 clause 1 fallback if either instrument fails to red.

**Q2 — Does the Deferred resolve propagate `gMaterial.G` into `spec_ao`, or only diffuse ambient?**
**Into `spec_ao`.** Verified: `deferred_pbr.hlsl:797` `float ao = material_texel.g;` (the marcher's
mesh-AO lane), `:948` `float ao_final = ao;`, `:977`
`float spec_ao = saturate(pow(NoV + ao_final, exp2(-16.0*roughness - 1.0)) - 1.0 + ao_final);`.
SV0's routing (AO → `ao_final` → `spec_ao`, §1.1) is therefore an exact structural match, and
S4(iv)'s owner eval compares like with like. No change to §1.3 is needed.

**Q3 — `VB × Both × HWRT`: is "SDF bodies cast no shadow on mesh" intentional, or an unlisted
follow-up?**
It is **inherited, not introduced** — and Rev 2 lists it. The `!consumers.hwrt_denoise_or_vis_on`
term at `render_path_config.rs:904` is shipped behaviour that SV0 *consumes* rather than re-derives
(P1-3). SV0 makes the gap visible under VB where it was previously moot (VB had no SDF-on-mesh
shadow at all). §1.2 now names it explicitly. Closing it means putting SDF bodies into the mesh TLAS
— out of scope for SV0, and now a **named follow-up** rather than an unlisted regression. Rows 9–10
of S4's table make the exclusion mechanical: `sv0_never_arms_under_hwrt` is a CPU test that reds if
a future rung ever lets the two co-occur without the TLAS work.

**Q4 — `vb_resolve.comp.hlsl` has no TEXTURED variant. Which tail does the shipping textured
configuration actually use?**
**`vb_shade_tex.comp.spv`, automatically.** `present/scene_types.rs:2329-2350`:
`vb_use_classified = force || vb_tex_active_this_frame`, and `vb.rs:896-900` makes the choice
three-way — so under VB a textured material *forces* the classified tail; `vb_resolve` never runs
textured. If a pre-light consumer (`ssao_on`, `ddgi_on`, `shadow_temporal_on`, or the hwrt carrier)
arms `mesh_geo_shade_split` (`render_path_config.rs:854`), the split displaces it and the producer is
`vb_shade_split_tex.comp.spv` instead. Consequence for the design: the fused tail S4's row 1
exercises is **not** the one a textured production scene runs, which is exactly why S4's selection is
all 10 variants rather than the two Rev 1 gated, and why S5 measures **both** row 1 and row 7.

---

## 11. Open questions (VALUES/SCOPE — owner)

1. **Default state.** Ship SV0 default-OFF (opt-in via `LightingConfig`) or default-ON when
   `path_is_vb() && resolved.shadow.contains(SDF_SOFT_MARCH) && mesh_leg`?
   *Recommendation: default-OFF through S4, flip after S5's number is known.*
2. **`-D` fallback authorisation.** If G1 or G2 fails to red (§7 clause 1), is +10 `.spv` acceptable,
   or is that an abort?
3. **S1 fixture composition.** The `vb_both_sdf` scene needs SDF bodies placed to satisfy S1's oracle
   predicate against the five spheres. Owner may prefer reusing `grand_showcase`'s SDF arrangement
   rather than a purpose-built one.
4. **`PINS.toml` reconciliation** (§5.3). Was `[vb_mesh]`'s hash ever blessed? If yes, the
   "NOT real hashes" paragraph at `:293-297` is stale and should be deleted; if no, the values should
   be reset to `PENDING`. This is a **precondition on S1**, and only the owner knows which.
5. **HWRT follow-up.** Should "SDF bodies in the mesh TLAS" (closing §10 Q3's gap) be scheduled, or
   is `VB × Both × HWRT` accepted as SDF-shadow-free indefinitely?
