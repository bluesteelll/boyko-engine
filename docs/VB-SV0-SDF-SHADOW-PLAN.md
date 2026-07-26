# VB-SV0 — SDF soft-shadow + contact-AO on mesh, inlined into the VB lit-producer tails

**Status:** DESIGN, **Rev 4** — NOT YET APPROVED. Stage 2 of the "finish VB completely" campaign
(Stage 1 = VB-P1 clustered cull, COMPLETE; Stage 3 = VB-P4 GPU-driven raster, out of scope).
Rev 1 drew **3 P0**; Rev 2 answered them and drew **3 new P0**; Rev 3 closed those by experiment
and drew **4 more** (3 from review, 1 promoted from P2 when a run settled it). Rev 4 answers all
four.

**The pattern in that sequence is the point, and it is not converging by accident.** Every
revision's P0s have been the same defect wearing a new costume: *a gate that cannot go red for the
failure it exists to catch.* Rev 4 found it in three fresh places at once — no gate could red for a
dead contact-AO term (§6 S1/S4), S2 gate (b)'s named mutation is **dead-code-eliminated** and can
never red (§11.4, measured), and S3's device oracle is not constructible as written (§6 S3). The
count is not evidence the design is bad; it is evidence the *review* is finally aimed at the right
thing. A revision that finds none of these is the one to distrust.

**This document states no measured number in prose — with one fenced exception, added in Rev 3.**
Every fact that could drift is a named test, and the test name is the citation. Numbers that appear
are either *structural bounds* (derived from a loop's own form) or explicit `MEASURE` placeholders a
rung fills in **code**. Golden hashes are never written here — gates read them from
`goldens/PINS.toml`. The exception is **§11**, a dated record of the three Rev 3 experiments: those
numbers are evidence for a design decision, **no gate reads them**, and every rung that depends on
one must re-derive it in its own test under the "MEASURED — do not edit these literals to make a
failing run pass" discipline. The rule exists because hand-copied numbers in prose caused every
prior revision to introduce defects at the lines it edited; a number that nothing reads cannot
drift a gate, but it is fenced and dated so a later reader can tell it apart from a live threshold.

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
| C8 | No golden hash literal appears in this document. §5.3 records the `PINS.toml` self-contradiction as a **blocking precondition on S1** rather than inheriting it. ***SUPERSEDED by D5** — the precondition is discharged and deleted; read §5.3 as it now stands.* | P1-5 |
| C9 | §4.3 "five sources" → **four** (verified). §2.3 states the `register(t0)` space-0 check and its near-miss. §2.4's citation corrected and R11's tripwire re-sited out of `debug_assert!`. | P2-1/2/3 |
| C10 | New §10 answers the critic's four open questions with verified evidence. | — |

## Changelog Rev 2 → Rev 3

| # | Change | Cause |
|---|---|---|
| D1 | §3.4 rewritten: the OFF-path codegen hazard is **refuted by measurement**, and the refutation is stated with its own limits. The stated *mechanism* (DXC folds `diff_ambient * 1.0`) is FALSE — DXC emits the multiply. The residual (a driver contraction **choice**) is quantified against a **measured** golden sensitivity floor. | P0-A |
| D2 | §3.3's stated *reason* for expecting the ULP probe to fire is **withdrawn**: "structurally the overwhelmingly likely outcome" is not supported, and a 1-ULP perturbation measured on a real term did **not** fire. The probe's *placement* is what saves it — it sits AFTER the OETF, so it is not gamma-attenuated. Rev 2's procedural demand that the control be DEMONSTRATED red is now the only thing carrying that gate, and it is enough. | P0-A |
| D3 | S2 gate (c) enumerates **all six** `deferred_pbr` `.spv`, not the two a `cs_6_0`-only helper could reach. All six were re-DXC'd and are byte-identical today (§11.2), so the gate is proven implementable before it is written. `redxc_with_defines` gains a **profile** parameter — a `-T` cannot be smuggled through the `defines` slice, which unconditionally `-D`-prefixes every element (`vb_sv0_offpath.rs:76-78`). | P0-C |
| D4 | S1 lands **two** fixtures, `vb_both_sdf` and `vb_both_sdf_tex`, making S4(ii) constructible for all 8 armable rows. Rev 2 could not build rows 4/6/8. The textured-ness is entirely in-test and the assets are committed; no host plumbing is added. | P0-B |
| D5 | §5.3's **blocking precondition on S1 is DELETED**. It described a `PINS.toml` self-contradiction that no longer exists: `d93e425` reconciled eight pin blocks, and `goldens/PINS.toml:283-285` now states the `[vb_mesh]` values are the real bless output. §5.3's other consequence (no gate duplicates a hash literal) stands. | P0-A review |
| D6 | Recorded, not fixed here: `deferred_pbr_wrap.comp.spv` ships and is built unconditionally (`gpu_scene/mod.rs:1654-1656`) but has **no row** in `docs/SHADER-VARIANT-MANIFEST.md` — a standing-rule violation predating SV0, fixed in its own commit so this stage does not absorb it. | P0-C |
| D7 | Two structural blind spots recorded that no earlier revision named: metals have `diffuse_color` **exactly** 0, so `diff_ambient * ao_final` is identically zero for 2 of `[vb_mesh]`'s 5 spheres; and `float ao_final = 1.0;` has **5** sites in `crates/boyko_rhi_vulkan/shaders/`, of which SV0 scopes 3. | P0-A review |

## Changelog Rev 3 → Rev 4

| # | Change | Cause |
|---|---|---|
| E1 | **SV0 is TWO terms and every gate was satisfied by ONE.** §3.1 gives shadow and contact-AO separate bits, so they can be armed apart — yet S1's adequacy oracle quantified only over the shadow predicate and S4(ii) never named the mode value. A contact-AO term wired to the wrong lane, min-combined with the wrong operand, or structurally `1.0` passed S1, S2, S3, S4 and S5. S1 gains `SV0_MIN_AO_PIXELS`; S4(ii) splits into per-term (ii-a)/(ii-b) with per-term red mutations. | P0-E1 |
| E2 | **§4.1's copy arithmetic was wrong and its header move breaks a test it never named.** `sdf_soft_shadow_ranged` has **two** shipping HLSL definitions, not one — the second is `sdf_probe_update.comp.hlsl:160`, generated from a verbatim string constant. `ddgi_probe_gi_sync.rs`'s `sdf_soft_shadow_ranged_copy_matches_resolve` extracts the function **out of `deferred_pbr.hlsl` by signature search and panics if absent** — which is exactly what the include move causes. Counts restated 5 → 2; the test is re-targeted, not broken; the generator constant is deliberately **not** re-pointed so §5.1's "`.spv` perturbed: exactly 10" survives. | P0-E2 |
| E3 | **S3 layer 3 was not constructible on either axis.** "On device" — the cited `cpu_gpu_sdf_agreement.rs` family is **GPU-FREE by design** and no on-device leaf probe is ever dispatched. "Both leaves" — `boyko_shaderdsl` has **no `sdf_ao` body** (consistent with §4.1's own finding, and R8 forecloses writing one). Split by leaf: the shadow leaf keeps host bit-exactness against an oracle that exists; the AO leaf gets a pre-registered tolerance with its reason written down, and is **labelled the weaker instrument**. | P0-E3 |
| E4 | **S2 gate (b)'s red mutation is DEAD — measured (§11.4).** An unread `float3 face_normal` member plus its `normalize(cross(…))` is fully eliminated by DXC: both `vb_geo.comp.spv` and `vb_geo_mv.comp.spv` come out **byte-identical**. Rev 3 asserted this mutation as demonstrated; it was analysis, the same error §3.4.1 had to withdraw. Replaced by a preprocessor-level check that is red-capable by construction. | P0-E4 (was P2) |
| E5 | Bookkeeping the review caught, all verified: **S0 is COMPLETE @`189d063`** (§3.2's runtime-vs-`-D` decision is discharged, §7 clause 1's G1 disjunct is settled, §12 Q2 narrows to G2); **D6/§5.4's manifest gap is DISCHARGED @`a4824a8`**; §5.3's "every gate reads the pin" is scoped to **image** goldens (G1's 10 `.spv` literals are deliberate and disciplined); C8 carries a superseded-by-D5 marker. | review P2 |
| E6 | Red mutations added for the two gate parts that shipped without one (S2(e) `spirv-val`, S4(iii) the hwrt truth-table) — under this plan's own rule an undemonstrated gate part is not yet a gate, and (iii) is the ONLY mechanical instrument covering rows 9–10. §7 gains **clause 5** so S1.5/S5 reddening on instrument spread has a defined outcome instead of the dangling state S1.5's own text conceded. | review P1 |
| E7 | S1's oracle gloss corrected: the ray-hit predicate is **sufficient, not equivalent** to "the leaf returns < 1.0" — the Quilez accumulator darkens well before the hard-hit early-out — so it undercounts and can reject an adequate fixture. §3.4 ground 1's undated sweep is downgraded to rest on the analytic Sterbenz argument, which stands alone. | review P2 |

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
`-D` buys inertness once and taxes every future VB tail feature forever. **Decision: runtime gate —
and as of Rev 4 the condition is DISCHARGED, not pending.** S0 shipped at `189d063` with both gates
green: all ten rows reproduce byte-identically, and the sensitivity control **fired** (47824 vs
47840 bytes, distinct fingerprints, from a single float literal in an untouched region). So the
`-D` fallback is no longer a live branch of this decision; it survives only as §7 clause 1's abort
route if G2 — the half S0 does not cover — comes back blind.

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
* **Rev 3 withdraws Rev 2's stated reason for expecting it to fire.** Rev 2 argued this was
  "structural … the overwhelmingly likely outcome". §11.1 measured the analogous case and it came
  out the other way: a **1-ULP** perturbation of a real shading term produced a **byte-identical**
  frame on `[vb_mesh]`. So "perturb by 1 ULP ⇒ the 8-bit hash moves" is **not** structural.
* What actually protects this probe is its **placement**, and that is worth stating because it is
  the difference between the two cases. The probe sits *immediately before the `gLit` store*
  (`vb_resolve.comp.hlsl:411` exposure → `:412` `tonemap_select` → `:413`
  `lit = pow(lit, OETF_GAMMA_EXP)` → `:414` store; `OETF_GAMMA_EXP = 1.0/2.2`,
  `pbr_lighting.hlsli:202`). Every attenuator that made §11.1's perturbation invisible — the OETF,
  the tonemap, and the ambient-fraction dilution — is **upstream** of the injection point, so none
  of them applies to the probe. The §11.1 result therefore does **not** refute this probe; it
  refutes the *argument* Rev 2 gave for it.
* Consequently **no expectation is pre-registered here at all.** Rev 2's procedural demand — the
  control must be *demonstrated* red, in a run whose output is pasted into the rung's commit
  message — is the whole of the gate, and it is sufficient precisely because it does not depend on
  predicting the answer. If it comes back BLIND, §7 clause 1 adjudicates it; that is a finding, not
  a failure of this document.

**G1 ∧ G2 is the gate.** G1 bounds the *textual* blast radius; G2 executes the real pipeline and has
a demonstrated red. Neither is claimed to be a proof of universal bit-identity, and §8-R2 records the
residual honestly.

### 3.4 The withdrawn hazard (P0-2)

Rev 1's headline claim — that making `ao_final` a phi de-folds `spec_ao`'s `x - 1.0 + 1.0` and moves
the OFF path by 1 ULP — **does not survive checking, and is withdrawn**, along with the duplicated-
recompute construction it motivated and S2's "decisive" second mutation. Three independent grounds:

1. **Analytic + numeric.** `NoV = max(dot(n,v), 1e-4)` (`vb_resolve.comp.hlsl:258`) and
   `roughness = clamp(m.mrr.y, 0.045, 1.0)` (`:262`) force `pow(NoV + 1, exp2(-16r - 1)) ≥ 1`; the
   subtraction is Sterbenz-exact on `[1,2]`; both forms produce bit-identical `1.0`. *(Rev 2 also
   cited "a sweep over the reachable domain found zero mismatches". **Rev 4 drops that clause**: it
   is an undated measurement with no test name and no §11 row — the one number in this document that
   could not be traced to a rung, a test or a record, and §3.4.1 promotes this ground to
   load-bearing. The analytic argument stands on its own and needs no sweep.)*
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

#### 3.4.1 The SECOND consumer — raised as P0-A against Rev 2, and refuted by experiment

Rev 2's withdrawal checked **one** of `ao_final`'s consumers. It has two. Besides `spec_ao`
(`vb_resolve.comp.hlsl:289`), it is passed to `eval_pbr_ambient_hemi` at `:325`, whose body ends
`return diff_ambient * ao_final + spec_ambient * spec_ao;` (`pbr_lighting.hlsli:165`). P0-A claimed
that with the literal `1.0` the compiler folds `diff_ambient * 1.0` away, leaving one multiply and
one add — contractible into a single FMA — so SV0's runtime `min` would introduce a **second**
multiply competing for that add, and a driver contracting the other one would move the OFF path.

**The mechanism is FALSE and the claim is withdrawn.** DXC does not fold the multiply. §11.1's
normalized disassembly diff of the shipping module against an SV0-shaped mutant shows `OpFMul` 244
in both, `OpVectorTimesScalar` 28 in both, `OpFunctionCall` 0 in both, no `Fma` in either; the site
itself is `OpVectorTimesScalar %v3float %diff_ambient %float_1` in the base and the same opcode at
the same position with an `OpSelect` operand in the mutant. There is no second multiply, so there
is no contraction contest. The claim was mine, it was stated as analysis, and analysis was the
wrong instrument.

**What survives, stated as narrowly as it is true.** A driver may fold `x * 1.0` when the operand
is a SPIR-V constant and cannot when it is an `OpSelect`, so the two modules can still receive
different *back-end* treatment. The multiply by exactly `1.0f` is IEEE-exact, so the only reachable
consequence is a **contraction choice** — one rounding, a ≤1-ULP-class effect. Two facts bound it:

1. §11.1 rendered the SV0-shaped mutant through the real pipeline: **byte-identical to the pin**.
2. §11.1 also measured what that proves. The `[vb_mesh]` golden is **blind** to a 1-ULP and a
   2^-20 perturbation of this term and **sees** 2^-16 and coarser. Its floor sits between ~8 and
   ~128 ULP.

So (2) says (1) is weak evidence: the gate cannot resolve the effect (1) appears to exclude. The
honest position, and the one Rev 3 adopts, is **not** "the OFF path is bit-identical" but: *the
residual is a ≤1-ULP-class effect, it is below the measured resolution of every shipping gate in
this repo, and it therefore cannot break a golden — nor can its absence be proven here.* §8-R2
carries that residual; §5.1's "byte-identical" claims are claims **at gate resolution**, which is
what they were always able to mean.

**Two blind spots this exercise exposed, neither previously named.**

* **Metals are structurally blind to the expression under test.** A metal's `diffuse_color` is
  `base * (1.0 - metallic)` = exactly 0 (`vb_resolve.comp.hlsl:268`), so `diff_ambient * ao_final`
  is identically zero for 2 of `[vb_mesh]`'s 5 spheres regardless of `ao_final`. Any future gate
  aimed at this site must be read against 3 spheres, not 5.
* **The literal has five sites, not two sources.** `float ao_final = 1.0;` occurs at
  `vb_resolve.comp.hlsl:288`, `vb_shade.comp.hlsl:452`, `vb_shade_split.comp.hlsl:455` (demoted to
  a phi at `:460`), and — outside SV0's scope — `forward_opaque.fs.hlsl:257` and
  `sdf_forward_march.comp.hlsl:1040`. The last two carry the identical decl/`spec_ao`/hemi trio
  reaching the same `pbr_lighting.hlsli:165`, so a later rung extending SV0 to the Forward or
  SDF-forward legs inherits this analysis verbatim rather than needing it redone.

**One correction to Rev 2's own reasoning, since it will otherwise be reused.** Rev 2 §3.4 ground 2
("the construction already ships in `vb_shade_split` and `deferred_pbr` with green goldens") proves
**non-pathology** — the two-multiply shape is production-blessed and renders correctly. It does not
prove *bit-neutrality of a form change*, because those goldens were blessed **with** the
two-multiply form and were never compared against a one-multiply counterpart of the same producer.
Ground 1 (Sterbenz exactness on `spec_ao`) and §11.1 are the load-bearing arguments; ground 2 is
supporting only.

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
* **TWO tests re-target, not one (P0-E2).** Rev 3 named only
  `sdf_soft_shadow_ranged_matches_edsl_emit`. `ddgi_probe_gi_sync.rs`'s
  **`sdf_soft_shadow_ranged_copy_matches_resolve`** (`:79-103`) also reads `deferred_pbr.hlsl` and
  pulls the function out of it by signature search through an extractor that
  **`panic!`s when the signature is absent** (`:44-49`). Moving the span into an include is exactly
  that condition, so this rung **breaks a shipping test inside the very span it moves** — and no
  earlier revision mentioned the file at all. Both tests re-target the shared header; the pin's
  meaning is preserved exactly, since what it asserts (the probe-update copy equals the resolve's)
  is unchanged by where the resolve's copy lives. `sdf_soft_shadow_ranged_copy_matches_resolve`
  green is an S2 gate part, asserted rather than assumed.
* **The generator constant is deliberately NOT re-pointed.** `SDF_SOFT_SHADOW_RANGED_COPY`
  (`boyko_shaderdsl/src/bin/emit_probe_gi.rs:554-563`, whose own doc defers an `.hlsli` dedup to
  "I3") keeps emitting `sdf_probe_update.comp.hlsl:160` verbatim. Re-pointing it would move that
  shader and its `.spv`, widening §5.1's "`.spv` perturbed: **exactly 10**" — a scope change this
  stage does not take. The dedup stays I3's.
* **`sdf_gbuffer_composite.hlsl` is NOT touched.** Its `sdf_ao` remains the second (and last) copy.
  Rationale: the marcher is the frozen-`.spv` blast radius this campaign explicitly protects, and
  collapsing 2 copies → 1 does not justify re-DXC'ing every marcher variant. The pair is pinned
  mechanically by **`sdf_ao_body_matches_shared_header`** (S3), a cross-file textual-identity test
  whose red condition is "the two bodies diverged" — a different, weaker mechanism than an eDSL
  re-emit pin, and labelled as such.

**Net, corrected in Rev 4 — the earlier arithmetic was wrong in its baseline (P0-E2).**
`sdf_soft_shadow_ranged` has **two** shipping HLSL definitions today, not one:
`deferred_pbr.hlsl:515` and `sdf_probe_update.comp.hlsl:160`. So the naive post-SV0 count is **5**,
not 4, and the post-move count is **2**, not 1 — the generated probe copy survives by the decision
above. `sdf_ao` copies **4 → 2**, both pinned, unchanged.

Shadow consts (`deferred_pbr.hlsl:466-474`, including `SHADOW_NORMAL_BIAS` at `:474`) are pinned by
`sv0_consts_match_deferred_and_marcher`. **Rev 4 promotes that test into S3's enumerated layers and
widens its selection to the three AO consts**, because Rev 3 left a hole: layer 2 pins the `sdf_ao`
**body**, while the term's entire numeric behaviour lives in `AO_STEP`/`AO_FALLOFF`/`AO_STRENGTH`
declared **outside** it (`sdf_gbuffer_composite.hlsl:488-490`), duplicated into the shared header,
with the marcher deliberately untouched. Change one in one file and the bodies stay byte-identical
while the two copies compute different things — the gate green for exactly the divergence it names.

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
`deferred_pbr.hlsl:74-79` documents for `TERMINATOR_WRAP`.

**Rev 3 added "and it is a gate that can go red: delete the guard and those two `.spv` change
bytes". That is FALSE, and §11.4 measured it (P0-E4).** With the guard deleted — an unread
`float3 face_normal` member plus its `normalize(cross(…))` — DXC eliminates the member and every
instruction feeding it, and `vb_geo.comp.spv` / `vb_geo_mv.comp.spv` come out **byte-identical**
(15888 B / 17864 B, both matching the committed artifacts). Nothing reads the field in that shader,
`-O3` is the default, and SROA + DCE of a wholly-unused local-struct member is routine. The claim
was stated as analysis and analysis was again the wrong instrument — the identical error §3.4.1 had
to withdraw, one section away.

Note the asymmetry, because it decides the fix. The *protective* half is **strengthened**, not
weakened: `vb_geo`'s `.spv` is byte-identical whether or not the guard is there, so the guard costs
that shader nothing and the byte-identity claim is now proven twice over. What is gone is
**falsifiability** — a gate whose only red is unreachable is not a gate.

**S2 gate (b) is therefore respecified at the level where the guard is actually observable:** the
check is a **preprocessor** comparison, `dxc -P vb_geo.comp.hlsl` with and without the guard, whose
outputs must **differ**. Red by construction, since the guard is a preprocessor construct and this
compares preprocessor output. The `.spv` byte-identity assertion stays as a separate, protective
gate part — true, useful, and now honestly labelled as one that cannot fail for this mutation.

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
**all six** `deferred_pbr` `.spv` (§5.4 enumerates them) are **byte-identical**, and each of those is
itself a gate.

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

### 5.3 Pins are read, never copied — and the Rev 2 precondition is DISCHARGED

Rev 2 recorded a `PINS.toml` self-contradiction (a `[vb_mesh]` comment calling its own `sha256_*`
values *"UNBLESSED placeholders — NOT real hashes"* while the sibling `[vb_both]` block treated the
same value as live) and made reconciling it a **blocking precondition on S1**.

**That precondition is discharged and is deleted from this plan.** Commit `d93e425` reconciled the
eight affected blocks; `goldens/PINS.toml:283-285` now states the values are *"the real bless
output, not placeholders"* and names the blessing commit. `grep -n UNBLESSED goldens/PINS.toml`
returns one hit — `:15`, the generic `"PENDING"` sentinel rule. Nothing blocks S1.

The other consequence stands, **scoped correctly in Rev 4**: no SV0 gate duplicates an **image-
golden** hash literal — every one reads the pin through `scripts\golden.ps1 -Pin <name>` / its
`Read-Pins` reader, because a hand-copied image hash is the failure mode `PINS.toml:1-5` was created
to end. Rev 3 wrote that as a universal over *every* gate, which is false: G1 pins **10 `.spv`
sha256 literals in the test** (§3.3), and the three `.spv`-comparison gates are not `PINS.toml`
consumers at all — there is no `PINS.toml` for `.spv`. Those literals are deliberate, carry the
"MEASURED — do not edit these literals to make a failing run pass" discipline, and are re-derivable
by anyone via `git show <S2^>:<path> | sha256`. The defect was the unqualified universal, not the
design.

### 5.4 The `deferred_pbr` family is SIX rows, and four of them are `cs_6_5` (P0-C)

SV0 moves a shared span out of `deferred_pbr.hlsl` into `sdf_shadow_leaves.hlsli` (§4.1), and that
span sits **before the file's first preprocessor conditional**, so it is compiled into every
variant. Gate (c) must therefore quantify over all of them. Rev 2 said "every `deferred_pbr` `.spv`"
without enumerating, and an implementer following the existing helpers would have covered **two**:
every re-DXC helper in `crates/boyko_rhi_vulkan/tests/` hardcodes `-T cs_6_0`
(`cluster_cull_spv_sync.rs:76`, `marcher_spv_sync.rs:57`, `ssao_edsl_sync.rs:288`,
`vb_froxel_spv_sync.rs:59`, `vb_sv0_offpath.rs:75`, `cluster_cull_hier_dis_gate.rs:539`), and the
profile **cannot** be smuggled through the `defines` slice because every element of it is
unconditionally `-D`-prefixed (`vb_sv0_offpath.rs:76-78`).

The frozen recipe (`deferred_pbr.hlsl:71-93`) reads *"add `-T cs_6_5 …`"* for the HWRT rows, i.e.
the profile is **replaced**, not appended. §11.2 settles that reading by running it:

| # | `.spv` | `-T` | `-D` |
|---|---|---|---|
| 1 | `deferred_pbr.comp.spv` | `cs_6_0` | — |
| 2 | `deferred_pbr_wrap.comp.spv` | `cs_6_0` | `TERMINATOR_WRAP=1` |
| 3 | `deferred_pbr_hwrt.comp.spv` | `cs_6_5` | `HWRT=1` |
| 4 | `deferred_pbr_hwrt_vis.comp.spv` | `cs_6_5` | `HWRT=1`, `SHADOW_STAGE=1` |
| 5 | `deferred_pbr_hwrt_denoised.comp.spv` | `cs_6_5` | `HWRT=1`, `SHADOW_STAGE=2` |
| 6 | `deferred_pbr_hwrt_vis_mv.comp.spv` | `cs_6_5` | `HWRT=1`, `SHADOW_STAGE=1`, `MOTION_VECTORS=1` |

**Three things this buys, all of which Rev 2 would have discovered at implementation time:**

1. **`redxc_with_defines` gains a `profile: &str` parameter.** One line, and it is the only change
   the four `cs_6_5` rows need — §11.2 reproduced all six byte-for-byte with the helper's existing
   argument order (`-spirv -T <p> -E main`, each `-D`, `-fspv-target-env`, source, `-Fo`).
2. **No test in this repo re-DXCs a `cs_6_5` artifact today**, so this is a first. `cs_6_5` frozen
   recipes do exist in shader headers (`vb_shadow_vis.comp.hlsl:87`,
   `hwrt_as_descriptor_smoke.comp.hlsl:18`); the VB-family one is a natural later row and is
   **out of scope for SV0**, named here so its absence is a decision rather than an oversight.
3. **The `hwrt` cargo feature is irrelevant to this gate.** It reads committed **files** and
   compares sha256; it never touches an embed, so it runs on a default-feature build.

**Recorded, and fixed in its own commit rather than absorbed here — DISCHARGED @`a4824a8`.**
`deferred_pbr_wrap.comp.spv` is built unconditionally (`gpu_scene/mod.rs:1654-1656`, and its
accessor at `compute.rs:1470` carries no `#[cfg(feature = "hwrt")]`, unlike the four HWRT ones) yet
had **no row** in `docs/SHADER-VARIANT-MANIFEST.md`, which `CLAUDE.md` requires per variant. It
predates SV0. The manifest now carries all six rows and they match the table above exactly — an
independent corroboration of this enumeration, since the two were derived from the same frozen
recipe but written separately.

---

## 6. Rungs

Ladder: **cheap harness falsifier → fixture → cost falsifier → dark infra → device oracle → arm →
measure.** Each rung is independently committable, has **one** gate, and names the mutation that
turns it red. *A mutation that is only argued does not count; the commit message records the mutated
run's output.*

### S0 — the OFF-path harness (CPU + `dxc`, no shader edit) — ✅ **COMPLETE @`189d063`**

**Outcome, measured:** both gates green. All ten rows re-DXC byte-identically, and the sensitivity
control **fired** — `vb_resolve.comp.hlsl:258`'s `1e-4` → `2e-4` gives 47824 B / `fnv1a_64
0xcfdc60ff4ea57052` committed vs 47840 B / `0x99e860db9dd753e2` mutant. A re-DXC byte comparison is
therefore **not blind** for these modules, which is what §3.2's runtime-gate decision rested on. The
harness stays a standing regression gate; it can no longer newly fail the *design*.

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

**Lands: TWO fixtures, not one (P0-B).** Rev 2 landed only the flat one, which left S4(ii)
unconstructible for rows 4, 6 and 8 — the three textured rows. Those need a scene that is textured
**and** `legs: Both` **and** carries a non-empty SDF edit list; the shipping textured pins are
`legs: Mesh`, where SV0 is structurally unarmable, so nothing in the tree could arm them.

1. `crates/boyko_app/tests/vb_both_sdf.rs` — a clone of `vb_both.rs` with SDF primitives actually
   spawned (edit list gathered by `collect_sdf_edits`, `boyko_app/src/runner.rs:589`), positioned
   so at least one SDF body occludes the five-sphere scene's key light and sits near a mesh surface.
2. `crates/boyko_app/tests/vb_both_sdf_tex.rs` — the same scene with a textured material.

Plus `[vb_both_sdf]` and `[vb_both_sdf_tex]` blocks in `goldens/PINS.toml`, seeded `PENDING`.

**Why the textured fixture is a clone-and-graft and not new plumbing — verified, because Rev 2
assumed the opposite.** Textured-ness is entirely in-test: two extra system params
(`NonSendResMut<Assets<TextureGpu>>`, `NonSendResMut<BindlessTextureTable>`,
`vb_mesh_tex.rs:98-99`), one `load_material_folder` call (`:156-157`), and `Material::with_textures`
(`:167-170`) — the **only** constructor that can set `MATERIAL_FLAG_TEXTURED`
(`boyko_render/src/material.rs:225-233`). That flag OR-reduces over mesh draws
(`mesh_draw.rs:1011`) into `vb_tex_active` (`present/scene_types.rs:2538-2543`), which is what
auto-selects the classified `vb_shade_tex` row — no `RenderPathConfig` field, no env knob. The
textures are **committed** files (`crates/boyko_app/assets/pbr_fixtures/synth_bumps/`, four PNGs,
path compiled in at `vb_mesh_tex.rs:47-48`).

Three structural facts make the combination safe, each checked rather than assumed:

* **`legs: Both` + a non-empty edit list already boots and already has a blessed pin** —
  `taa_jitter_eval.rs:305` spawns an `SdfEdit::sphere` and `goldens/PINS.toml`'s `[vb_both_taa]`
  block drives it through VB×Both, explicitly contrasting itself with `[vb_both]`'s empty list.
  This fixture is therefore not a new capability, only a new scene.
* **Nothing couples textured materials to the SDF gather** — `collect_sdf_edits`
  (`boyko_render/src/sdf_edit.rs:115-129`) is a pure `Query<&SdfPrimitive>` walk with no material,
  texture or path input, run once after `app.finish()`.
* **An SDF body cannot pick up the textured material** — the SDF surface reads only `base_color`
  (`sdf_gbuffer_composite.hlsl:1799-1800`, no `MATERIAL_FLAG_TEXTURED` reference in that file), and
  `SdfEdit::sphere` leaves material lane 0, which is the engine-minted default
  (`runner.rs:238-242`), not the fixture's first `Assets::add`.

**Row 8 additionally needs SSAO on**, which arms the split path. The knob is a `SsaoConfig` insert
placed **after** `add_plugins` so the boot resolver sees it — the shape `vb_mesh_ssao.rs:190` already
uses, read at `runner.rs:442`.

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
   within `T_MAX`. **(b) is SUFFICIENT, not equivalent, and Rev 3's "i.e. the leaf would return
   `< 1.0`" gloss is withdrawn (Rev 4):** the Quilez accumulator drops below 1.0 as soon as
   `SHADOW_K * d / t < 1` at **any** step, whereas `d < SHADOW_HIT_EPS` is the hard-hit early-out
   returning `0.0` (`sdf_gbuffer_composite.hlsl:498-526`). So this counts fully-occluded pixels and
   **undercounts** the pixels the term actually darkens. It errs safe — false red, never false
   green — but it can reject an adequate fixture under §7 clause 2, or push the fixture toward hard
   contact shadows when §1.3's visual goal is penumbra. Read the floor with that in mind;
3. **≥ `SV0_MIN_AO_PIXELS` covered mesh pixels with a non-trivial contact-AO term — NEW in Rev 4,
   and it is the P0 this rung existed to prevent and did not (P0-E1).** The predicate: some tap at
   `h ∈ {AO_STEP … 5·AO_STEP}` along the **shading** normal returns a field distance `< h`, i.e.
   `sdf_ao` is not saturated at its far-field value. Same host `Eval` oracle, same fixture, no GPU;
4. the ≥2-frame `SdfEditStaging::is_dirty()` assertion from §2.4's re-sited R11 tripwire.

**Why (3) had to exist.** SV0 ships **two independently gated terms** — §3.1 gives them separate
bits (5 shadow, 6 AO) precisely so they can be armed apart. Every gate in Rev 3 was satisfied by the
**shadow half alone**: this oracle quantified only over the shadow predicate, and the fixture
requirement "sits near a mesh surface" was **prose, not a gate**. §4.4 already documents that the AO
term saturates to exactly `1.0` when the field is far — the identical vacuity trap this rung exists
to prevent for the shadow half, left wide open for the other half.

**RED if:** (1) or (4) fails, or (2)'s or (3)'s count falls below its floor. **Mutations, and they
must be independent:** remove the SDF spawns → (1), (2), (3) all fail; **move the SDF body far from
the mesh surface while keeping it between the mesh and the key light → (3) drops below floor while
(2) survives.** The second is the one that proves the two counts are not the same assertion wearing
two names.

**Both floors are fixed BEFORE the fixture is authored and are never lowered** — the same
"MEASURED — do not edit these literals to make a failing run pass" discipline S1.5 and S5 carry.
A floor chosen after seeing the scene is a floor tuned to pass.

**The coverage oracle is named, because Rev 3 left it unnamed and unlanded.** Both counts need
CPU-side raster coverage, and nothing in the tree computes it (`goldens.rs`'s host mirrors take a
mesh depth buffer as an *input*). S1 therefore **lands** it: the fixture's own `uv_sphere()`
triangles projected with the fixture's camera and rasterized on the CPU — small, exact, and it
reuses the scene's own geometry rather than approximating it. An analytic-sphere coverage
approximation is acceptable **only** if labelled as such with the floors set clear of the silhouette
error; it is not the default.

**Precondition:** none. Rev 2's `PINS.toml` precondition is discharged — §5.3.

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
(c) **all six** `deferred_pbr` `.spv` byte-identical (the §4.1 header move) — the enumeration and
    its feasibility are §5.4, and all six were re-DXC'd green before this gate was written (§11.2);
(d) every VB image golden byte-identical with `sv0_mode = 0`;
(e) `spirv-val` clean on all 10;
(f) **G2's ULP-probe control demonstrated RED** — the probe compile's render differs from the pin;
(g) **`sdf_soft_shadow_ranged_copy_matches_resolve` green** (§4.1, P0-E2) — the test whose extractor
    `panic!`s when `deferred_pbr.hlsl` stops containing the signature, i.e. the one this rung's
    header move breaks unless it is re-targeted in the same commit.

**RED if / mutations (DEMONSTRATED):**
* (a): move one SV0 statement outside its `#ifndef VB_SV0_KILL` guard → G1 red.
* (b): **respecified in Rev 4 — the old mutation is DEAD, measured (§11.4).** `dxc -P` of
  `vb_geo.comp.hlsl` with and without the `#ifdef VB_SV0` guard must **differ**; red by
  construction. The `.spv` byte-identity half stays as a protective assertion that, for this
  mutation, provably cannot fail.
* (e): declare `Buf` at a binding already occupied in `vb_layout0_froxel` → `spirv-val` red. *(Rev 3
  gave (e) no mutation; under this ladder's own rule an undemonstrated gate part is not yet a gate.)*
* (g): perform the §4.1 header move **without** re-targeting the extractor → the test panics → red.
  This one reddens by default, which is the point: it fails unless the rung does the work.
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
3. **Leaf agreement — SPLIT BY LEAF, and HOST, not device (P0-E3).** Rev 3 specified "leaf
   bit-exactness **on device** … evaluates **both leaves** … demand exact `f32::to_bits()`
   equality". Neither half is constructible, and this is the stage's only numerical correctness
   instrument, so an implementer would have blocked at S3 or silently degraded it unlabelled.

   *Why "on device" is wrong:* the cited family is **GPU-free by design** —
   `cpu_gpu_sdf_agreement.rs:30-33` states *"This file is GPU-FREE (no `VulkanContext::boot`) …
   It therefore runs even on a GPU-less host."* It never executes HLSL. And no on-device leaf probe
   exists to borrow: `sdf_field_probe.comp.spv` is never dispatched — `field_probe_gate.rs`
   disassembles the committed file against a frozen baseline, nothing more. Building one means a new
   `.spv` plus a manifest row, contradicting §5.1's **zero-new-`.spv`** invariant that §3.2's whole
   runtime-gate-vs-`-D` arithmetic rests on. Not worth trading that away for this.

   *Why "both leaves" is wrong:* there is **no `sdf_ao` body in `boyko_shaderdsl`** — the only two
   occurrences in that crate are prose (`refine.rs:8`, `ssao.rs:69`), exactly consistent with §4.1's
   own finding that `sdf_ao` is not eDSL-generated, and R8 forecloses writing a generator.
   `sdf_soft_shadow_ranged_body` **does** exist (`shadow.rs:174`).

   **3a — the shadow leaf, unchanged in strength.** `sdf_soft_shadow_ranged` evaluated over a
   ≥4096-sample `(P, N, L)` fixture against `sdf_soft_shadow_ranged_body::<Eval>` on the **host**,
   **bit-exact `f32::to_bits()` equality**. Pure SSBO math, no sampler, an oracle that exists.

   **3b — the AO leaf, and it is the WEAKER instrument, labelled.** The only host AO model is
   `goldens::host_ao`, whose own doc concedes it *mirrors the shader within ±3/255* — and the gap is
   structural, not a bug to fix: the host computes `AO_FALLOFF.powi(i)` where the HLSL computes
   `pow(AO_FALLOFF, (float)i)` (`sdf_gbuffer_composite.hlsl:538`), and `pow` is a
   platform-dependent transcendental, so bit-exactness against it is **unreachable**, not merely
   untested. 3b is therefore agreement at a **pre-registered ULP tolerance**, fixed before the run
   and never widened to make a failing run pass, with that reason written into the test so nobody
   discovers it later and quietly relaxes it. The AO half's strongest correctness evidence remains
   S4(iv), and §8's risk table says so rather than implying parity with 3a.
4. **Host unit tests for the face normal** — under a non-uniform-scale affine, `face_normal` equals
   the analytic plane normal; under uniform scale it agrees in sign with the interpolated normal; a
   degenerate (zero-area) triangle returns the interpolated fallback, never NaN. Plus the §4.5 NaN
   assertion: a NaN term degrades to "no contribution".
5. **`sv0_consts_match_deferred_and_marcher` — promoted to a layer in Rev 4, and widened.** §4.1
   pointed at this test as the const pin but scoped it to the *shadow* consts and named it in no
   rung's Lands, so nothing actually landed it. It now covers the shadow consts **and** the three AO
   consts `AO_STEP`/`AO_FALLOFF`/`AO_STRENGTH`, which layer 2 structurally cannot see: layer 2 pins
   the `sdf_ao` **body**, the consts are declared outside it
   (`sdf_gbuffer_composite.hlsl:488-490`), and §4.1 duplicates them into the shared header while
   leaving the marcher untouched.

**RED if / mutations:** (1) change one token in **one** tail's asserted span → that tail's pin fails.
(2) perturb the marcher's `sdf_ao` body → the cross-file pin fails. (3a) perturb one host const in
the `Eval` backend → the shadow leaf's bit-exactness fails. (3b) perturb the AO tap count → the AO
leaf exceeds its pre-registered tolerance. (4) reverse the `cross` operand order without the
orientation fix → the sign test fails. **(5) change `AO_FALLOFF` in the marcher only → layer 5 red
while layer 2 stays green** — that pairing is the whole reason layer 5 exists.

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
* **(ii) the input reached, per armable variant AND PER TERM.** Rev 3 wrote one assertion with
  "SV0 armed" and **never named the mode value** — so `sv0_mode = 3` passed on the shadow term
  alone, and a contact-AO term wired to the wrong lane, min-combined with the wrong operand, or
  structurally `1.0` went green through S1, S2, S3, S4 and S5, leaving only the owner's eye
  (iv) between it and shipping. The 2-bit field already exists; Rev 4 simply uses it (P0-E1):
  * **(ii-a) shadow only** — mode = `VB_SDF_MESH_SHADOW_BIT`, AO bit clear.
  * **(ii-b) contact-AO only** — mode = `VB_SDF_MESH_AO_BIT`, shadow bit clear.

  For each of rows 1–8, **each** of (ii-a) and (ii-b) must **on its own** differ from that row's
  `sv0_mode = 0` render, in a changed-pixel count within `[1%, 60%]` of covered mesh pixels.
  Neither may lean on the other. **Which fixture, per row (P0-B):** rows 1, 2, 3, 5, 7 use
  `vb_both_sdf`; rows **4, 6, 8** use `vb_both_sdf_tex`, because a textured row cannot be selected
  at all without a `MATERIAL_FLAG_TEXTURED` material in the scene, and the shipping textured pins
  are `legs: Mesh`, where SV0 is structurally unarmable. Rev 2 named this gate for all 8 rows
  without a fixture that could carry 3 of them; S1 now lands both.

  **The changed-pixel instrument is landed by S1, not assumed.** No `changed_pixels`/percent-of-
  coverage comparator exists in `crates/` or `scripts/` today, and this gate needs one that also
  knows the covered-pixel denominator — which is exactly the CPU coverage oracle S1 lands. One
  instrument, two consumers.
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
* **Per-term, added in Rev 4** — the three source reverts above partition by *file*, so all four of
  Rev 3's mutations red the shadow and AO halves together and none isolates a term. Two more, each
  reddening **exactly one** sub-gate: delete the `min` into `vis` → every row's **(ii-a)** falls to
  0 while **(ii-b)** survives; delete the `min` into `ao_final` → the converse. Without this pair,
  (ii-a)/(ii-b) are two names for one assertion.
* **(iii)'s own red, added in Rev 4** — Rev 3 gave (iii) no mutation at all, and it is the **only**
  mechanical instrument covering rows 9–10, the two variants the plan removes from executing
  coverage on a structural argument. Delete the `!consumers.hwrt_denoise_or_vis_on` term from
  `render_path_config.rs:904` → `sv0_never_arms_under_hwrt` red. Cheap, and until it is run the
  structural argument is unguarded.

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

1. **The OFF-path proof fails to exist.** **The G1 half is SETTLED** — S0 shipped green at `189d063`
   with its sensitivity control demonstrated (see S0), so this clause reduces to: S2 gate (f)'s ULP
   probe fails to red (G2 is blind) **and** the owner declines the `-D` fallback's +10 `.spv`.
   Without it, OFF-path inertness has no executing proof and 10 re-pins ship on faith.
2. **S1's fixture cannot be made adequate** — no `VB × Both` configuration yields
   `SV0_MIN_SHADOWED_PIXELS` pixels satisfying S1's oracle predicate with a non-empty edit list. Then
   there is no scene in which SV0 can be observed and every downstream gate is vacuous by
   construction.
3. **Cost.** S5's median paired delta exceeds **2×** S1.5's measured `SV0_DEFERRED_TERM_REFERENCE` on
   the same fixture. The threshold is a ratio to a **measured sibling that already ships this visual
   at an accepted cost**, not a predicted number — the campaign's refuted-cost-model lesson. In
   `[1×, 2×]` it ships with the number recorded; above 2×, revert.
4. **S4 (i) cannot be made byte-identical.** A persistent difference on an analytically-`1.0` term
   means the OFF path is not inert, and no amount of re-blessing fixes that. **Read this clause
   together with clause 1, and in that order** — it has force only once S2(f) has shown the gate can
   go red. If the probe comes back BLIND, a byte-identical result here is uninformative rather than
   reassuring, and clause 1 fires first. Rev 3 removed the assumption that the probe *would* fire
   (§3.3), so this clause no longer inherits one.

5. **The cost instrument is not reproducible** — S1.5's or S5's cross-session spread exceeds 10%.
   Added in Rev 4 because Rev 3 left it dangling: S1.5 is advertised as able to kill the stage, its
   own RED text concedes that *"§7's ABORT clause cannot be adjudicated"*, and no clause covered it.
   The exposure is not hypothetical — the campaign's own record is a 21% run-to-run spread on this
   box, and unlike §11.2/§11.4 no feasibility run was made for the paired protocol on this fixture.
   **Outcome is an owner VALUES call**: revert, or ship unmeasured with the spread recorded and
   clause 3 (the 2× ratio) explicitly waived — clause 3 divides two numbers that an irreproducible
   instrument makes incommensurable, so it cannot silently stand. S1.5's null control also gets a
   numeric criterion: `|median paired delta|` on two *identical* configurations must be ≤ a
   pre-registered fraction of the armed delta, not "~0".

Revert granularity: every rung is independently committable, so an abort at S5 reverts S2–S4 and
keeps S0 (COMPLETE, and the harness is reusable by every future runtime-gated shader feature) and
S1 (the fixture, the coverage oracle and the changed-pixel comparator are a real gain regardless —
S4(ii) and any future VB visual feature need all three).

---

## 8. Risks

Named first are the ones this campaign has actually hit.

| # | Risk | Precedent | Mitigation |
|---|---|---|---|
| R1 | **Vacuously-green gate** — assertion quantified over an empty selection. | Hit 3× in Stage 1. **Live here:** every VB golden has `edit_count == 0` (`PINS.toml:322`, `:355`). | S1 is blocking and its gate is an `Eval`-oracle adequacy check, not "the frame differs"; S4(ii) is quantified over **all 8** armable variants. |
| R2 | **OFF-path drift invisible to an 8-bit golden.** | Rev 1's instance refuted (§3.4); Rev 2's second instance refuted by measurement (§3.4.1). The risk itself is **CONFIRMED REAL and quantified** — §11.1 measured the golden blind to 1 ULP and to 2^-20 on a live shading term. | G1 bounds the textual blast radius; G2 executes and must go **demonstrably** red, with no expectation pre-registered (§3.3). **Residual, stated at full strength:** a ≤1-ULP-class perturbation of the OFF path would pass every gate in this repo. That is the accepted inertness standard for this stage, not an oversight — §12 Q5 puts the choice to the owner. |
| R3 | **Cost model instead of measurement.** | The refuted `a + b*(froxels*N)` model. | No predicted number in any gate. S1.5 and S5 are measurements; §7's threshold is a ratio to a measured sibling. |
| R4 | **Session drift read as a regression.** | The phantom regression on this hardware. | Interleaved paired A/B, warmup discarded, 3 sessions, spread reported — enforced in S1.5 and S5. |
| R5 | **Instrument that silently does nothing.** | The flat-curve knob. | S0 validates the harness against a deliberately mutated recompile before it is trusted; S1.5 has its own null-mutation control; S2(f) is G2's own control. |
| R6 | **Silent OOB with `robustBufferAccess` OFF.** | No layer reports it. | SV0 adds **no** new indexing — `Buf[0]` is already clamped by `min(Buf[0], MAX_SDF_EDITS)` (`sdf_field.hlsli:204`). Binding 10 is always a valid descriptor (`scene.edit_list` is non-`Option`). `MAX_IT = 128u` caps every march path including NaN (§4.4), so no device hang is possible. |
| R7 | **`NMin`/`NMax` NaN semantics.** | HLSL `min`/`max` → `NMin`/`NMax`, returning the non-NaN operand. | Exploited deliberately twice: guarantees march termination (§4.4) and makes a degenerate term degrade to "no contribution" rather than poison the pixel (§4.5). Asserted in S3(4). |
| R8 | **`sdf_ao` has no generator** — hand-authored HLSL the eDSL does not own, a live tension with CLAUDE.md's shader rule. **Rev 4: the consequence runs deeper than the copy count** — it also means the AO leaf has **no host `Eval` oracle**, so S3(3b) is a tolerance check where the shadow leaf gets bit-exactness. | — | §4.1 cuts copies 4 → 2 via the shared header and pins the survivor pair with `sdf_ao_body_matches_shared_header`, plus §4.1's promoted const pin (layer 5) for the three AO consts the body pin structurally cannot see. The tension is **not** resolved and is stated, not hidden; writing a generator would perturb the frozen marcher `.spv` and is out of scope. The asymmetry between 3a and 3b is labelled at the point of use, so nobody reads "S3's oracle" as one uniform instrument. |
| R9 | **A tail silently omitted.** | The P0-class hole this design closes. | S4's selection is all 10 variants with per-variant executing assertions; the three demonstrated revert-one-source mutations each red exactly their own rows. |
| R10 | **Non-uniform scale.** | `vb_geom_fetch.hlsli:539-542`. | Out of scope, stated plainly (§4.2). The bias is robust; the shading normal is not. |
| R11 | **Edit list becomes per-frame dirty** → a missing barrier under VB. | — | Re-sited out of `debug_assert!` (compiled out in release, where goldens run) into **test code**: S1's ≥2-frame `is_dirty()` assertion (§2.4). |
| R12 | **`dxc`-dependent gates skip.** | `cluster_cull_spv_sync.rs:196-204`. | §5.2 states honestly that **every** static gate needs `dxc` — Rev 1's "one gate can never skip" went with the withdrawn instrument. Procedural mitigation: no rung is commit-eligible until its `dxc` gate has been run and its output pasted into the commit message. |
| R13 | **`deferred_pbr` perturbed by the §4.1 header move.** | New in Rev 2. | S2 gate (c): **all six** `.spv` byte-identical (§5.4 enumerates; §11.2 already ran them green), with a demonstrated red (misplace the `#include`). |
| R15 | **Half a feature ships dead.** SV0 is TWO independently-bit-gated terms, and every Rev 3 gate was satisfied by the shadow half alone. | Found in review of Rev 3; §4.4 documents that the AO term saturates to exactly `1.0` when the field is far, i.e. the same vacuity trap S1 exists to prevent for the shadow half. | S1 gate (3)'s independent `SV0_MIN_AO_PIXELS` count with its own mutation, and S4(ii)'s split into per-term (ii-a)/(ii-b), each required to move pixels **alone**, with the two `min`-deletion mutations. Generalised lesson: *a gate quantified over a feature with N independently-armable terms must have N assertions, not one.* |
| R16 | **A red mutation that the optimiser deletes.** Gate (b)'s named mutation was DCE'd — measured, §11.4. | Third occurrence of the campaign's signature defect inside this one plan (Rev 1's Merkle instrument, Rev 3's gate (b), and R15 above). | Respecify the gate at the level where the construct is *observable* — a preprocessor guard is checked with `dxc -P`, not with `.spv` bytes. Standing rule: before asserting a mutation reddens a compiled artifact, ask whether the compiler can see the mutation at all. |
| R14 | **A shader-swap experiment measures its own previous iteration.** | Hit in §11.1: five different `.spv` produced one hash because `Copy-Item` carries the source mtime and cargo skipped the relink. | Any rung that swaps a `.spv` must stamp the destination's mtime **and assert the test binary was rebuilt**. This is the project's known false-fresh failure reaching a new surface; it is silent, and it fabricates agreement rather than disagreement — the direction that gets believed. |

---

## 9. Appendix — verified file:line anchors

Every line below was opened while writing **this revision**. Anchors Rev 1 asserted but did not
survive checking are marked **[Rev 1 WRONG]**.

**Added in Rev 3, all opened while writing it.**
*OFF-path codegen (§3.4.1):* `vb_resolve.comp.hlsl:288` (the literal), `:289` (`spec_ao`), `:268`
(`diffuse_color` — exactly 0 for a metal), `:325` (the hemi call), `:411-414` (exposure → tonemap →
OETF → store) · `pbr_lighting.hlsli:165` (the combine), `:202` (`OETF_GAMMA_EXP = 1.0/2.2`) ·
the other four literal sites: `vb_shade.comp.hlsl:452`, `vb_shade_split.comp.hlsl:455` (phi at
`:460`), and out of scope `forward_opaque.fs.hlsl:257`, `sdf_forward_march.comp.hlsl:1040`.
*Variants (§5.4):* `deferred_pbr.hlsl:71-93` (six frozen recipes) · `compute.rs:1470`
(`deferred_pbr_wrap_spirv`, **no** `#[cfg(feature = "hwrt")]`) · `gpu_scene/mod.rs:1654-1656`
(built unconditionally) · `vb_sv0_offpath.rs:75` (`-T cs_6_0` hardcoded), `:76-78` (every `defines`
element is `-D`-prefixed) · other `cs_6_5` recipes, out of scope:
`vb_shadow_vis.comp.hlsl:87`, `hwrt_as_descriptor_smoke.comp.hlsl:18`.
*Fixture (S1):* `vb_mesh_tex.rs:47-48` (asset path), `:98-99` (the two params), `:156-157`
(`load_material_folder`), `:167-170` (`with_textures`) · `material.rs:225-233` (the only constructor
setting `MATERIAL_FLAG_TEXTURED`) · `mesh_draw.rs:1011` (OR-reduce) ·
`present/scene_types.rs:2538-2543` (`vb_tex_active`) · `sdf_edit.rs:115-129` (`collect_sdf_edits` —
a pure `Query<&SdfPrimitive>`) · `runner.rs:238-242` (default material lane 0), `:442` (`ssao_on`),
`:589` (the gather site) · `sdf_gbuffer_composite.hlsl:1799-1800` (SDF reads `base_color` only) ·
`vb_mesh_ssao.rs:190` (the `SsaoConfig` knob) · `taa_jitter_eval.rs:305` (a `legs: Both` scene with
a non-empty edit list that already ships).

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

**Goldens:** `goldens/PINS.toml:15` (the `"PENDING"` sentinel), `:263-311` (`[vb_mesh]`; `:283-285`
declares the values **blessed**, superseding Rev 2's stale-or-live reading — §5.3),
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

## 11. Experiment record (Rev 3 §§11.1–11.3, Rev 4 §11.4) — dated, and NOTHING READS THESE NUMBERS

Fenced exception to this document's no-measured-numbers-in-prose rule (see the status block). These
are the runs that closed Rev 2's three P0s. They are **evidence for design decisions, not gate
thresholds**: no test reads them, and any rung that needs one re-derives it in its own code under
the "MEASURED — do not edit these literals to make a failing run pass" discipline.

**Environment for all three:** 2026-07-26, single RTX 3060, `windows-gnu`, `dxc` 1.4.350.0 at the
pinned SDK path, `BOYKO_DISABLE_VALIDATION=1`. Working tree at `d93e425`, clean.

### 11.1 The OFF-path codegen experiment (closes P0-A, §3.4.1)

**Method.** Copy `crates/boyko_rhi_vulkan/shaders/*.hlsl{,i}` to a scratch tree; mutate the copy;
re-DXC under the frozen recipe; copy the result over the committed `vb_resolve.comp.spv`; render
`scripts\golden.ps1 -Pin vb_mesh`; `git checkout --` the artifact.

**Harness fidelity, established first.** The *unmutated* scratch copy re-DXC'd to 47824 bytes,
**byte-identical to the committed `vb_resolve.comp.spv`** — so the scratch tree is the shipping
input, not an approximation of it.

**A false-green was hit and defeated, and it is the reusable lesson.** The first sweep returned the
**same** frame hash for five materially different `.spv`. Cause: `Copy-Item` carries the *source's*
`LastWriteTime`, and all five were written by one `dxc` loop, so after the first rebuild the test
binary was newer than every subsequent copy and cargo skipped the relink — the project's known
false-fresh failure in a new costume. Fix: stamp the destination's `LastWriteTime` to now after
each copy **and assert the `vb_mesh-*.exe` mtime advanced**. Every row below relinked. *A
shader-swap experiment that does not assert the relink is measuring its previous iteration.*

**Result A — the mechanism.** Normalized `spirv-dis` diff, shipping module vs a mutant whose
`ao_final` is a runtime `OpSelect` numerically equal to 1.0 on the OFF path:

| | base | mutant |
|---|---|---|
| `OpFMul` | 244 | 244 |
| `OpVectorTimesScalar` | 28 | 28 |
| `OpFunctionCall` | 0 | 0 |
| GLSL.std.450 `Fma` | 0 | 0 |

Site: `OpVectorTimesScalar %v3float %diff_ambient %float_1` → same opcode, same position,
`%select` operand. DXC does **not** fold the multiply.

**Result B — the golden's sensitivity floor for this term.** `ao_final` perturbed; each row a
distinct `.spv`, each relinked, and the perturbed constant verified present in each disassembly so
"blind" is not constant-folding.

| perturbation | frame vs pin |
|---|---|
| +1 ULP (2^-23) | **BLIND** |
| −2^-20 | **BLIND** |
| −2^-16 | SEES |
| −2^-12 | SEES |
| −2^-10 | SEES |
| −2^-8 | SEES |
| →0.5 (gross control) | SEES |

Floor: between ~8 ULP (invisible) and ~128 ULP (visible). The SV0-shaped mutant of Result A
rendered **byte-identical to the pin** — which Result B shows is weak evidence, and §3.4.1 states
the conclusion at that strength and no stronger.

### 11.2 The `deferred_pbr` six-row re-DXC (closes P0-C, §5.4)

All six recipes from `deferred_pbr.hlsl:71-93` were run against the committed artifacts, reading
*"add `-T cs_6_5`"* as **replacing** the base profile, with the helper's existing argument order:

    -spirv -T <profile> -E main [-D <def>]... -fspv-target-env=vulkan1.3 deferred_pbr.hlsl -Fo <out>

**All six byte-identical.** So: the six-row table is right, the profile substitution is the correct
reading, the argument order needs no change, and gate (c) is implementable for the four `cs_6_5`
rows with a single new helper parameter — proven before the gate is written rather than discovered
when it fails.

### 11.3 Fixture constructibility (closes P0-B, S1)

Not a run but a source audit, recorded here because Rev 2 asserted the opposite: the
textured × `Both` × non-empty-SDF combination needs **no new host plumbing**. Anchors are in S1.

### 11.4 The S2 gate-(b) DCE probe (closes P0-E4, §4.3)

**Question.** Rev 3 asserted that deleting the `#ifdef VB_SV0` guard would change
`vb_geo.comp.spv`'s bytes, making gate (b) red-capable. `vb_geo` never reads `face_normal`, DXC
defaults to `-O3`, and the frozen recipe passes no `-O` override — so the assertion was a codegen
claim stated as analysis, the same shape §3.4.1 had to withdraw.

**Method.** Scratch copy of the shader tree; add to `VbGeomFetchResult` an **unguarded**
`float3 face_normal` plus `result.face_normal = normalize(cross(world_p1 - world_p0,
world_p2 - world_p0));` at the result-construction site; re-DXC `vb_geo.comp.hlsl` under the frozen
recipe, base and `-D MOTION=1`.

**Result — both byte-identical to the committed artifacts:**

| `.spv` | probe | committed |
|---|---|---|
| `vb_geo.comp.spv` | 15888 B | **identical** |
| `vb_geo_mv.comp.spv` | 17864 B | **identical** |

DXC eliminates the member and every instruction feeding it. **Gate (b)'s named mutation cannot go
red.** The protective claim is strengthened — `vb_geo` provably pays nothing for SV0 with or without
the guard — but the gate needed respecifying at the preprocessor level, which §4.3 now does.

---

## 12. Open questions (VALUES/SCOPE — owner)

1. **Default state.** Ship SV0 default-OFF (opt-in via `LightingConfig`) or default-ON when
   `path_is_vb() && resolved.shadow.contains(SDF_SOFT_MARCH) && mesh_leg`?
   *Recommendation: default-OFF through S4, flip after S5's number is known.*
2. **`-D` fallback authorisation.** If **G2** fails to red (§7 clause 1), is +10 `.spv` acceptable,
   or is that an abort? *(Narrowed in Rev 4 — G1's half is settled by S0 @`189d063`.)*
3. **S1 fixture composition.** The `vb_both_sdf` / `vb_both_sdf_tex` scenes need SDF bodies placed to
   satisfy S1's oracle predicate against the five spheres. Owner may prefer reusing
   `grand_showcase`'s or `taa_jitter_eval`'s SDF arrangement rather than a purpose-built one.
   **Rev 4 adds a constraint that narrows the choice:** the scene must satisfy **two** independent
   predicates — occluding the key light (S1 gate 2) *and* placing a body within
   `5·AO_STEP` of a mesh surface (S1 gate 3) — and a mutation must exist that breaks the second
   while leaving the first intact. A reused arrangement is fine only if it admits that separation.
4. **HWRT follow-up.** Should "SDF bodies in the mesh TLAS" (closing §10 Q3's gap) be scheduled, or
   is `VB × Both × HWRT` accepted as SDF-shadow-free indefinitely?
5. **Sub-gate-resolution drift** (§3.4.1, §11.1). SV0's OFF path is provably inert *at the measured
   resolution of every shipping gate*, with a ≤1-ULP-class residual that no instrument in this repo
   can see. Accept that as the stage's inertness standard, or is a stronger one wanted — which
   would mean building an FP oracle that reads `gLit` as `f32` rather than as an 8-bit dump, and
   that is a rung of its own, not a line in S2?

6. **The AO half's correctness standard** (§6 S3(3b), R8). The shadow leaf gets host bit-exactness;
   the AO leaf cannot — `pow` is a platform-dependent transcendental and there is no eDSL body to
   generate one from. Accept a pre-registered ULP tolerance plus S4(iv) as the AO half's standard,
   or is a stronger instrument wanted? The only stronger one is an eDSL `sdf_ao` generator, which
   R8 rules out this stage because it would perturb the frozen marcher `.spv`.

*Rev 2's question 4 (`PINS.toml` reconciliation) is **withdrawn** — discharged by `d93e425`, see
§5.3. Rev 3's question 2 (the `-D` fallback) narrows to **G2 only**: S0 shipped green at `189d063`,
so G1's half of that question is answered by measurement.*
