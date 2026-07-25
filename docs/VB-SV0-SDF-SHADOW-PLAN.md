# VB-SV0 — SDF soft-shadow + contact-AO on mesh, inlined into the VB lit-producer tails

**Status:** DESIGN, Rev 1 — **NOT APPROVED. DO NOT IMPLEMENT.** Stage 2 of the "finish VB
completely" campaign (Stage 1 = VB-P1 clustered cull, COMPLETE; Stage 3 = VB-P4 GPU-driven raster,
out of scope). The architecture-critic returned **CHANGES REQUESTED (3 × P0, 5 × P1)**; a Rev 2 must
land and pass re-review first. The three blockers are recorded below because two of them refute
claims made *in this document*, and a reader must not act on those claims.

**P0-1 — the OFF-path proof instrument is not constructible at the site SV0 lands.** §3.3's rung-S0
dataflow gate assumes a phi at the `gLit` store whose predecessor edge means "SV0 not taken". There
is none: the `vis` combine sits inside the `l0a_count` loop nested in an `if`, and the store's
operand is the loop-exit value of a loop-carried accumulator. Worse, the specified hash makes an
`OpPhi` a function of *both* incoming values, so everything downstream of the SV0 merge hashes
differently **by construction** whether or not the OFF path is arithmetically identical. Recovering
"the OFF path's value" needs a path-projected program slice, not a Merkle hash per result id — a
materially harder analysis than the ~200 LOC this plan budgets, and one that would also fire on
benign restructuring, i.e. an over-strict gate that gets weakened until it is vacuous. Since §3.2
makes the runtime-gate-over-`-D` decision *conditional on S0 passing*, the entire variant-matrix
decision rests on an instrument whose feasibility was never demonstrated. Rev 2 must either
re-specify the proof as something constructible (a build-time kill-switch compile byte-compared
against the frozen module is real, red-capable and `dxc`-only), or take `-D` and pay the +10 as an
owner call **now**.

**P0-2 — this document's headline sub-LSB hazard does not exist.** §3.3/§8-R2 claim that making
`ao_final` a phi turns `spec_ao`'s `x - 1.0 + 1.0` into a non-foldable expression and moves the OFF
path by 1 ULP. Refuted analytically and confirmed numerically (0 mismatches over a 30-point sweep of
the reachable `NoV` × `roughness` domain): `NoV ≥ 1e-4` and `roughness ∈ [0.045, 1]` force
`pow(NoV+1, exp2(-16r-1)) ≥ 1`, the subtraction is Sterbenz-exact on `[1,2]`, and **both forms
produce bit-identical `1.0`**. Independently, on the 4 TEXTURED lit producers `ao_final` is
*already* a runtime value (`vb_shade.comp.hlsl:450`), so the proposed "duplicated recompute" is a
no-op there. The plan's own binding rule is that every named mutation be demonstrated rather than
argued; its headline hazard was argued and did not survive checking. Rev 2 must withdraw it, and
rung S2's decisive mutation with it.

**P0-3 — the arming gate covers 2 of 10 shipping lit-producer `.spv`.** §1.1 claims rung S4 closes
the split-tail hole "structurally". It gives an *executing* gate only to `vb_resolve.comp.spv` and
`vb_shade_split.comp.spv`; the other eight — including the **entire classified tail**, which is the
production lit producer for textured VB frames — get only a `.contains()` text pin. A text pin
proves the source is present, not that it executes correctly: a wrong mode decode, a wrong bias
site or a wrong combine operand passes it green. This is the campaign's own named defect class one
level down — at-least-once coverage where exactly-once-per-variant is required, the same shape as
the earlier finding where an at-least-once check let a 2688-cell out-of-bounds write pass green.

**One finding this document got right, and it is the most valuable thing in it.** Every VB golden
scene boot-seeds an **empty** SDF edit list — `goldens/PINS.toml` states `count == 0` for both
`vb_both` and `vb_sdf_only`, and `vb_both`'s hash *equals* `vb_mesh`'s precisely because the march
finds nothing. Arming SV0 against today's fixtures would therefore produce a **vacuously green**
gate. Rung S1 (a new non-empty fixture, gated on "must differ from `vb_mesh`'s hash") is a genuine
prerequisite and no arming rung may precede it. Caught before a line of code was written.

**This document is deliberately short.** `docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md` reached ~3000
lines across six revisions and its own status block diagnoses why: every revision introduced
defects at exactly the lines it edited, because measured numbers were hand-copied into prose.
This plan therefore states **no measured number in prose**. Every fact that could drift is a
named test, and the test name is the citation. Where a number appears it is either a *structural
bound* (derived from a loop's own form, not from a run) or an explicit `MEASURE` placeholder
that a rung fills in **code**, never here.

---

## 0. What the stale row said, and what is now known

`docs/RENDER-PARITY-PLAN.md:351` specifies SV0 as: *"reuse `sdf_mesh_shadow.comp` under VB …
one `gSdfMeshShadow` binding added to vb Set 0 (O1: no 5th set)"*. It predates all of Stage 1.
Three of its premises are resolved:

| Premise | Status |
|---|---|
| "reuse `sdf_mesh_shadow.comp`" | **FALSE.** A repo-wide grep for `sdf_mesh_shadow` matches exactly one file: `docs/RENDER-PARITY-PLAN.md` itself. SF0 was never implemented. There is no pass to reuse. |
| "one binding added to vb Set 0" | **TRUE, and the slot is 10** — see §2. Slot 8 is *not* universally free. |
| "a dedicated producer pass" | **REJECTED by measurement** (campaign record: +5–12% for zero visual gain). Inline. |

The surviving prior art is the **Deferred** path, which already ships this exact visual:
`crates/boyko_rhi_vulkan/shaders/sdf_gbuffer_composite.hlsl:1853-1885` — the `!own_pixel`
raster-owned arm writing `gMaterial.RG = (mesh_shadow, mesh_ao)`, computed at `:1876-1878`
(`sdf_soft_shadow(P_mesh + N_mesh * SHADOW_NORMAL_BIAS, N_mesh, light)`) and `:1881`
(`sdf_ao(P_mesh, N_mesh)`). SV0 is the port of those two lines into the VB tails.

---

## 1. Scope

### 1.1 What SV0 IS

Two per-pixel terms, evaluated **inline** in the VB lit-producer tails, for pixels the VB
rasterizer covered (`instance_id != VB_ID_SENTINEL`, `vb_resolve.comp.hlsl:241`):

* **SDF soft-shadow on mesh** — the SDF field casts a clean analytic penumbra onto raster mesh
  surfaces. Applied by `min`-combine into the **primary directional light's** `vis`, at the same
  site the tail already `min`-combines `csm_visibility` (`vb_resolve.comp.hlsl:308-315`).
* **Contact-AO** — the 5-tap field-deficit AO (`sdf_gbuffer_composite.hlsl:532-540`) applied by
  `min`-combine into `ao_final` (`vb_resolve.comp.hlsl:288`), i.e. the *diffuse* ambient
  occlusion, matching the Deferred/Filament decoupling. `spec_ao` follows from `ao_final` by the
  existing formula (`:289`) — see the §4.5 hazard.

Both terms land in **all three** lit-producer tails: `vb_resolve.comp.hlsl` (fused),
`vb_shade.comp.hlsl` (classified), `vb_shade_split.comp.hlsl` (R9 geo/shade split). Landing in
two of three would leave the split path silently producing wrong shadows — §6's rung **S4**
closes that structurally, with a demonstrated red mutation, rather than by review discipline.

### 1.2 What SV0 is NOT

* **Not a new pass, target, framegraph `ResId`, or barrier.** `field_distance` is pure-analytic
  over one SSBO (`sdf_field.hlsli:203-217`, gateway at `:246`). Its include contract requires
  `StructuredBuffer<uint> Buf : register(t0)` in scope before the `#include`
  (`deferred_pbr.hlsl:153-161`). Inlining costs +1 SSBO binding and a control-flow gate.
* **Not multi-caster.** The P6 R1 per-light flagged-caster march
  (`light_table.hlsli:56` `LIGHT_FLAG_CASTS_SHADOW`, cap `MAX_SDF_SHADOW_CASTERS_PER_PIXEL = 4`
  at `deferred_pbr.hlsl:479`) is **out of scope**. SV0 marches **exactly once per covered
  pixel**, for the primary directional only — matching the Deferred mesh arm, which uses the
  single `pc.light_dir` (`sdf_gbuffer_composite.hlsl:1868`). This keeps the per-pixel field-eval
  bound at §4.4's figure regardless of light count.
* **Not mesh-SDF (MDF).** `sdf_soft_shadow_mesh` (the `pc.mesh_sdf_enabled` arm at
  `sdf_gbuffer_composite.hlsl:1876`) is not ported. SV0 marches the analytic edit-list field only.
* **Not correct under non-uniform instance scale.** See §4.2.
* **Not byte-comparable to Deferred.** `goldens/PINS.toml:288-291` already records that VB's
  analytic barycentric interpolation is a genuinely different floating-point path from hardware
  raster interpolation. VB-vs-Deferred parity is **visual** (owner-eval), never a byte gate.

### 1.3 Visual goal

On a VB×Both scene with a non-empty SDF edit list, an SDF body casts a soft, noise-free shadow
onto raster mesh surfaces, and mesh surfaces darken in the SDF geometry's ambient-occluded
crevices — visually equivalent to what `Deferred × Both` already produces on the same scene.

---

## 2. The binding decision

**Decision: VB Set 0, binding 10, `StructuredBuffer<uint> Buf : register(t0)`.**

### 2.1 Why not a 5th set

The TEXTURED VB variant already consumes four descriptor sets (0 core / 1 shadow / 2 geometry
table / 3 bindless textures — `vb_shade_split.comp.hlsl:70-108`,
`vb_geom_fetch.hlsli:20-34`, `vb_shade.comp.hlsl:163`). Vulkan's guaranteed
`maxBoundDescriptorSets` floor is exactly 4. A 5th set is unavailable, full stop.

### 2.2 Why slot 10 and not slot 8

VB Set 0 today:

| slot | contents | declared in |
|---|---|---|
| 0 | `gVbInstances` | `vb_geom_fetch.hlsli:51` |
| 1 | `instance_materials` / `instance_materials_tex` | `vb_resolve.comp.hlsl:95`, `vb_shade_split.comp.hlsl:154/163` |
| 2 | `Camera` UBO | `vb_resolve.comp.hlsl:99` |
| 3 | `LightBuf` | `vb_resolve.comp.hlsl:111` |
| 4 | `Materials` | `vb_resolve.comp.hlsl:119` |
| 5 | `gVbId` | `vb_resolve.comp.hlsl:123` |
| 6 | `gLit` | `vb_resolve.comp.hlsl:127` |
| 7 | `gClassify` | `gpu_scene/mod.rs:3450-3455` |
| 8, 9 | `ClusterGrid`, `LightIndexList` — **`#ifdef FROXEL` ONLY** | `vb_resolve.comp.hlsl:151-154` |

Two host layout objects exist: `vb_layout0` (8 entries, `gpu_scene/mod.rs:3395-3459`) and
`vb_layout0_froxel` (10 entries, `:4425-4492`). **Slot 8 is free only in scenes that never arm
the froxel cull** — using it would be a silent, scene-config-dependent collision that no
validation layer reports (validation is off on this box; `robustBufferAccess` is off).
**Slot 10 is free in both.**

### 2.3 The precedent

`crates/boyko_rhi_vulkan/shaders/deferred_pbr.hlsl:161` already declares

```hlsl
[[vk::binding(10)]] StructuredBuffer<uint> Buf : register(t0);
```

for precisely this purpose, and `:153-160` explains why it did not reuse `t0`'s Vulkan slot: the
`sdf_field.hlsli` include contract pins the HLSL *register* to `t0`, while the Vulkan *binding*
is free to be anything. SV0 copies that construction verbatim, including the "strict
FIELD-CONSUMER: calls `field_distance` read-only, never edits" contract.

### 2.4 Host changes

Binding numbers need not be contiguous; only the *entry count* is capped
(`rhi_impl/mod.rs:93`, `MAX_BIND_GROUP_BINDINGS = 24`). `vb_layout0` goes 8 → 9 entries
(`{0..7, 10}`), `vb_layout0_froxel` 10 → 11 (`{0..9, 10}`). Both far under the cap.

Four Set-0 descriptor **set** instances must gain the entry, all binding the same buffer:

| set | build site |
|---|---|
| `vb_set0` | `present/targets.rs:2995-3079` (entry array at `:3012-3024`) |
| `vb_set0_tex` | `present/targets.rs:3090` |
| `vb_set0_froxel` | `present/targets.rs:3193` |
| `vb_set0_tex_froxel` | `present/targets.rs:3311` |

The bound resource is `BindGroupEntry::StorageBuffer { buffer: scene.edit_list }` — the identical
expression the deferred/marcher sets already use at `present/targets.rs:1402`, `:2378`, `:2745`,
`:2903`. `scene.edit_list` is a plain (non-`Option`) field, so it is valid on **every** VB boot
including `legs: Mesh`, where the edit list is the empty boot seed. No new upload, no new
barrier: the edit list is a **one-shot boot-static write** performed before the first
`render_gbuffer_frame` (`boyko_app/src/runner.rs:1136-1141`), and SV0 adds only a second
COMPUTE reader ordered after it.

> **Invariant to encode, not assume:** if a future rung makes the edit list per-frame dirty, the
> VB tails need a barrier they do not have today. Rung S2 lands
> `debug_assert!(!staging.is_dirty_after_boot())` at the upload site so that change fails loudly.

---

## 3. The gate mechanism

### 3.1 A 2-bit field in light-header word 7, bits 5..6

Word 7 (`sky_diffuse.w`) is the campaign's established gate word. The authoritative bit budget
is `crates/boyko_render/src/light.rs:386-408`; the shader decoders are
`light_table.hlsli:77-180`.

| bits | owner |
|---|---|
| 0 | `shadow_mode` (`light_table.hlsli:77`) |
| 1 | `contact_shadow_mode` (`:91`) |
| 2 | `csm_mode` (`:109`) |
| 3 | `punctual_shadow_mode` (`:128`) |
| 4 | `ddgi_mode` (`:145`) |
| **5..6** | **`vb_sdf_mesh_mode` — SV0** |
| 7 | free |
| 8..11 | tonemap (`:163`) |
| 12..19 | terminator softening (`:179`) |
| 20..31 | free |

`light.rs:406` states bits 5..7 are free; SV0 claims 5..6 and leaves 7. The two bits are
independent flags, mirroring Deferred's `pc.lighting_flags` pair
(`sdf_gbuffer_composite.hlsl:1865/1869/1880`) one-for-one:

```hlsl
// light_table.hlsli (additive; no existing decoder perturbed — each masks only its own bits)
static const uint VB_SDF_MESH_OFF        = 0u;
static const uint VB_SDF_MESH_SHADOW_BIT = 1u; // bit 5
static const uint VB_SDF_MESH_AO_BIT     = 2u; // bit 6
uint load_vb_sdf_mesh_mode(StructuredBuffer<uint> LightBuf) { return (LightBuf[7] >> 5) & 3u; }
```

Host side, `boyko_render::light`: `VB_SDF_MESH_MODE_SHIFT: u32 = 5`, `VB_SDF_MESH_MODE_MASK: u32 = 3`,
two `LightingConfig` bools packed by the existing `shadow_gate_word`, plus a bit-position
`debug_assert_eq!` at the single writer — the exact idiom `ddgi_config.rs:288-289` uses.

### 3.2 Why a runtime gate and not `-D` — the arithmetic

**Shipping VB lit-producer `.spv` today: 10.** (`crates/boyko_rhi_vulkan/shaders/`, embeds at
`compute.rs:811-1037`, manifest rows at `docs/SHADER-VARIANT-MANIFEST.md:91-107`.)

| # | `.spv` | source | `-D` |
|---|---|---|---|
| 1 | `vb_resolve.comp.spv` | `vb_resolve.comp.hlsl` | — |
| 2 | `vb_resolve_froxel.comp.spv` | `vb_resolve.comp.hlsl` | `FROXEL=1` |
| 3 | `vb_shade.comp.spv` | `vb_shade.comp.hlsl` | — |
| 4 | `vb_shade_tex.comp.spv` | `vb_shade.comp.hlsl` | `TEXTURED=1` |
| 5 | `vb_shade_froxel.comp.spv` | `vb_shade.comp.hlsl` | `FROXEL=1` |
| 6 | `vb_shade_tex_froxel.comp.spv` | `vb_shade.comp.hlsl` | `TEXTURED=1 FROXEL=1` |
| 7 | `vb_shade_split.comp.spv` | `vb_shade_split.comp.hlsl` | — |
| 8 | `vb_shade_split_tex.comp.spv` | `vb_shade_split.comp.hlsl` | `TEXTURED=1` |
| 9 | `vb_shade_split_hwrt.comp.spv` | `vb_shade_split.comp.hlsl` | `HWRT=1` |
| 10 | `vb_shade_split_tex_hwrt.comp.spv` | `vb_shade_split.comp.hlsl` | `TEXTURED=1 HWRT=1` |

| | `-D SV0=1` | runtime gate (chosen) |
|---|---|---|
| new `.spv` | **+10** (10 → 20) | **0** |
| new manifest rows | +10 | 0 (10 rows get an updated recipe note) |
| new `embed_spirv!` consts + accessors | +10 + 10 | 0 |
| new pipeline objects + selector arms | +10 | 0 |
| existing `.spv` re-pinned | 0 | 10, once |
| re-DXC gate invocations | ×2 | unchanged |
| **orthogonal axes in the VB matrix** | **4** (tex × froxel × hwrt × sv0) | **3** |
| OFF-path byte-identity | free, by preprocessor | **must be proven** (§3.3) |

The decisive term is the last row but one. The matrix is **multiplicative**: the split tail
already ships 4 `.spv` from two axes; a fourth axis makes the *next* feature cost 40 variants,
not 20. `-D` buys byte-identity once and taxes every future VB tail feature forever. The runtime
gate buys a permanent axis-count freeze and pays a one-time cost: building the instrument that
proves OFF-path inertness — an instrument that is then reusable by every future runtime-gated
feature. **Decision: runtime gate, conditional on rung S0 passing.** If S0 fails, the design
falls back to `-D` and the +10 becomes an owner VALUES call (§7).

### 3.3 How OFF-path byte-identity is PROVEN

**A golden image pin is necessary and NOT sufficient.** The VB goldens are 8-bit sRGB sha256
pins (`goldens/PINS.toml:302-305`). An 8-bit hash cannot see a sub-LSB change in the pre-
quantisation radiance. A gate that cannot go red for the failure it exists to catch is worse
than no gate — this campaign shipped that mistake three times.

**And the hazard is real and concrete, not hypothetical.** `vb_resolve.comp.hlsl:288-289`:

```hlsl
float ao_final = 1.0;                                                       // :288  a LITERAL
float spec_ao  = saturate(pow(NoV + ao_final, exp2(-16.0*roughness - 1.0)) - 1.0 + ao_final); // :289
```

Today DXC (`-O3` by default; the frozen recipe passes no `-O`) sees `ao_final` as the constant
`1.0` and may fold `… - 1.0 + 1.0`. Turning `ao_final` into an `OpPhi` destroys that fold, and
`(y - 1.0) + 1.0 != y` in general for finite floats. **That is a 1-ULP move on the OFF path that
the image hash cannot see.** The three tails all carry this shape.

**Construction (mandatory):** every SV0 write is paired with a *duplicated recompute* of every
downstream value that today folds against the pre-SV0 literal, so the OFF path keeps the literal:

```hlsl
float ao_final = 1.0;
float spec_ao  = saturate(pow(NoV + ao_final, exp2(-16.0*roughness - 1.0)) - 1.0 + ao_final);
if ((sv0_mode & VB_SDF_MESH_AO_BIT) != 0u) {
    ao_final = min(ao_final, sdf_ao(P, n));                                  // NMin: NaN -> keeps ao_final
    spec_ao  = saturate(pow(NoV + ao_final, exp2(-16.0*roughness - 1.0)) - 1.0 + ao_final);
}
```

`vis` needs no such treatment — it is already a runtime value at `vb_resolve.comp.hlsl:308-313`.

**Gate (the decisive one): a CPU-only SPIR-V dataflow-equivalence instrument** — rung S0. For a
committed `.spv`, parse the word stream, and for every result id compute a Merkle hash over
`(opcode, type, literal operands, hashes of operand ids in positional order)`. Two modules are
*dataflow-equivalent at a store site* iff the hash of the stored value agrees. For SV0's OFF
path: the new module's `gLit` store operand is an `OpPhi`; the gate asserts **the hash of that
phi's incoming value from the SV0-not-taken predecessor equals the frozen predecessor module's
`gLit` store-value hash.**

Why this is decisive where the image hash is not:
* It compares **instructions and constants**, not pixels — a 1-ULP arithmetic change is a
  different dataflow graph and is therefore visible.
* It is robust to SSA renumbering and basic-block reordering (only structure is hashed).
* Reassociation (`a*(b*c)` → `(a*b)*c`) changes the tree shape, so it is caught — which a plain
  opcode multiset would miss.
* It reads **committed `.spv` bytes**, so it needs no external tool and **can never silently
  skip** — unlike the re-DXC gates, which skip when `dxc` is absent
  (`cluster_cull_spv_sync.rs:197-205`).

**S0's own red mutation is DEMONSTRATED, not argued** — see §6.

---

## 4. The march

### 4.1 Reuse, verbatim, of the eDSL-generated leaves

Both leaves are already machine-generated and sync-pinned; SV0 authors **no new field math**.

* **Shadow:** `sdf_soft_shadow_ranged` (`deferred_pbr.hlsl:514-532`), generated by
  `boyko_shaderdsl::emit::emit_hlsl_sdf_soft_shadow_ranged()`, pinned by
  `sdf_soft_shadow_ranged_matches_edsl_emit` in
  `crates/boyko_rhi_vulkan/tests/sdf_field_edsl_sync.rs`. Loop+tail only; the `NoL` early-out
  lives in the caller. SV0 passes `t_max = T_MAX` (the directional bound,
  `deferred_pbr.hlsl:467`).
* **AO:** `sdf_ao` (`sdf_gbuffer_composite.hlsl:532-540`), 5 `[unroll]`ed taps against
  `AO_STEP`/`AO_FALLOFF`/`AO_STRENGTH` (`:488-490`).
* **Constants** (`deferred_pbr.hlsl:466-474`): `EPS`, `T_MAX`, `MAX_IT`, `SHADOW_K`,
  `SHADOW_MINT`, `SHADOW_MINT_STEP`, `SHADOW_HIT_EPS`, `SHADOW_NDOTL_EPS`,
  `SHADOW_NORMAL_BIAS`. `GRAD_H` and `FIELD_LIPSCHITZ_L` come from `sdf_field.hlsli:42` / `:287`.

`SHADOW_NORMAL_BIAS`'s value is **pinned by a test, not by this prose**: rung S3's
`sv0_consts_match_deferred_and_marcher` asserts the SV0 copy equals both the marcher's
(used at `sdf_gbuffer_composite.hlsl:1877-1878`) and `deferred_pbr.hlsl:474`'s.

Rung S3 extends `sdf_field_edsl_sync.rs` so the generated spans **and** the const block are
`.contains()`-pinned in all three tails plus the `Buf @ t0` precondition — the O3 discipline
`docs/RENDER-PARITY-PLAN.md:383-384` already specifies for SF0.

### 4.2 Shadow-origin bias from the GEOMETRIC face normal

`vb_geom_fetch` already has the three world-space triangle vertices in registers
(`vb_geom_fetch.hlsli:536-538`), so the geometric face normal costs one `cross` + one `normalize`
and **no extra memory traffic**:

```hlsl
float3 e1 = world_p1 - world_p0;
float3 e2 = world_p2 - world_p0;
float3 fn = cross(e1, e2);
float  l2 = dot(fn, fn);
// Degenerate-triangle guard: fall back to the interpolated normal rather than normalize(0) -> NaN.
float3 face_n = (l2 > FACE_N_EPS2) ? (fn * rsqrt(l2)) : normalize(result.world_normal);
// Winding-independent orientation: agree with the shading normal.
result.face_normal = (dot(face_n, result.world_normal) < 0.0) ? -face_n : face_n;
```

**Why geometric, not interpolated.** `cross(p1-p0, p2-p0)` is computed from *actual world
positions*, so it is the true plane normal under **any** affine instance transform. The
interpolated normal is `mul(m3, n)` with the plain linear 3×3 and **no inverse-transpose
correction** — `vb_geom_fetch.hlsli:539-542` documents this as a known limitation, correct only
for uniform scale. Using the geometric normal for the origin lift also removes the classic
silhouette acne where the interpolated normal diverges from the facet.

**Stated plainly: SV0's correctness is scoped to uniform instance scale.** The *bias direction*
is robust; the *shading* normal that drives `NoL`, the BRDF, and the AO ray direction is still
the plain-`m3` interpolated one, inheriting `vb_geom_fetch`'s limitation verbatim. Fixing that
is the inverse-transpose rung `vb_geom_fetch.hlsli:539-542` already defers, and SV0 does not
attempt it.

**Deviation from Deferred, acknowledged:** the Deferred mesh arm lifts along the *shading*
normal (`sdf_gbuffer_composite.hlsl:1877`). SV0's term is therefore not bit-comparable to
Deferred's even in principle — which is already true for unrelated reasons (§1.2), so nothing
is lost.

**AO uses the shading normal**, verbatim per Deferred (`:1881`), and takes **no bias**: the taps
start at `h = AO_STEP` (`:536`), already off-surface.

### 4.3 The `#ifdef VB_SV0` source-guard — and what it buys

`vb_geom_fetch.hlsli` is included by **five** sources: the three tails plus
`vb_geo.comp.hlsl:118` (which ships `vb_geo.comp.spv` and `vb_geo_mv.comp.spv`). `vb_geo` does
not need a face normal. The new field and its computation therefore sit behind

```hlsl
#ifdef VB_SV0
    float3 face_normal;   // in VbGeomFetchResult
#endif
```

where `VB_SV0` is a **source-level `#define` written by each of the three tails before the
`#include`** — never a `-D` on the dxc command line, so it creates **zero** new compile
variants. `vb_geo.comp.hlsl` does not define it and therefore preprocesses **character-identical**
to today, keeping `vb_geo.comp.spv` / `vb_geo_mv.comp.spv` **byte-identical by construction**.
This is exactly the frozen-base discipline `deferred_pbr.hlsl:74-79` documents for
`TERMINATOR_WRAP`. It is also a gate that can go red: delete the guard and those two `.spv`
change bytes (rung S2 gate (b)).

### 4.4 Termination, bounds, and why no device hang is possible

The march is `[loop] for (uint i = 0u; i < MAX_IT; ++i)` (`deferred_pbr.hlsl:518-529`) — a hard
iteration cap. Beyond that, `t` is **strictly increasing by at least `SHADOW_MINT_STEP`** every
iteration, because the advance is `t + max(d / FIELD_LIPSCHITZ_L, SHADOW_MINT_STEP)` (`:525`):

* `d` negative (inside the field) → `max` returns `SHADOW_MINT_STEP`.
* `d` huge (empty field, `FAR = 1.0e9`, `sdf_field.hlsli:41`) → `t` overshoots `t_max` → `break`.
* **`d` NaN** → HLSL `max` lowers to `GLSL.std.450 NMax`, which returns the **non-NaN** operand
  → `SHADOW_MINT_STEP`. The march still advances and still terminates. The campaign's NMin/NMax
  lesson is load-bearing here rather than a hazard.

**Structural bounds per covered mesh pixel** (derived from the loop form, not measured):
`≤ MAX_IT` field evaluations for the shadow (`deferred_pbr.hlsl:468`) plus exactly 5 for AO
(`sdf_gbuffer_composite.hlsl:535`), and **exactly one march per pixel** regardless of light
count (§1.2). Each `field_distance` walks `min(Buf[0], MAX_SDF_EDITS)` edits
(`sdf_field.hlsli:204`) — already clamped, so SV0 introduces **no new indexing and no new
out-of-range surface**. That matters because this engine has `robustBufferAccess` OFF and no
GPU-assisted validation: an out-of-range access is real UB nothing reports.

**Empty-edit-list behaviour (exact, not approximate).** With `Buf[0] == 0` the loop at
`sdf_field.hlsli:206-215` never executes and `acc` stays `FAR`. Then:
`res = min(1.0, SHADOW_K*FAR/t) = 1.0`; the first `t` advance overshoots `T_MAX`; the leaf
returns `clamp(1.0, 0, 1) = 1.0` after **one** iteration. AO: every tap deficit is
`(h − FAR)`, so `1 − AO_STRENGTH*occ` saturates to exactly `1.0`. Both terms are **exactly
`1.0`**, and `min(x, 1.0)` is the bit-exact identity for every finite non-NaN `x`.
**Consequence, and it is a trap:** arming SV0 on any empty-edit-list scene is byte-identical —
which makes such a scene a *vacuous* arming gate. §5.1 and rung S1 exist because of this.

### 4.5 Falloff, and the combine

The penumbra falloff is Quilez basic soft-shadow: `res = min(res, SHADOW_K * d / t)`
(`deferred_pbr.hlsl:521`), `SHADOW_K = 8.0` (`:469`). No sqrt, no cone — deliberately, to keep
the FP-parity surface minimal against the host oracle (`sdf_gbuffer_composite.hlsl:492-497`).

Both terms combine by `min`:

```hlsl
vis      = min(vis,      sv0_shadow);   // at the primary-directional site, vb_resolve.comp.hlsl:308-315
ao_final = min(ao_final, sv0_ao);       // before spec_ao, with the §3.3 duplicated recompute
```

`min` on floats is exact (no rounding) and commutative/associative for non-NaN, so SV0's combine
is **order-independent** with respect to the existing CSM combine (`:313`) and the split tail's
HWRT denoised-visibility combine (`vb_shade_split.comp.hlsl:51-54`). Under NaN, `NMin` returns
the non-NaN operand — so a degenerate term **cannot** poison the pixel; it degrades to "no SV0
contribution". That is the correct failure direction and is asserted in rung S3.

---

## 5. Variant matrix and byte gates

### 5.1 `.spv` created: ZERO. `.spv` perturbed: exactly 10.

The ten rows of §3.2's table are re-DXC'd and re-pinned **once**, at rung S2, with **no change to
their `-D` combinations** and **no new manifest rows** — each existing row in
`docs/SHADER-VARIANT-MANIFEST.md:93-98` (and the split rows) gains one sentence noting the SV0
binding-10 interface delta. `vb_geo.comp.spv` and `vb_geo_mv.comp.spv` are **byte-identical**
(§4.3), and that is itself a gate.

Interface delta for all ten: `+ StructuredBuffer<uint> Buf @10 (register t0)`. Set 0 widens to 9
entries (`vb_layout0`) or 11 (`vb_layout0_froxel`).

### 5.2 Byte gates

| gate | mechanism | can it skip? |
|---|---|---|
| `vb_sv0_off_path_dataflow_equivalence` | S0's Merkle-hash instrument over committed `.spv` | **No** — reads bytes only |
| `vb_geo_spv_unperturbed` | `assert_spv_byte_identical` (`cluster_cull_spv_sync.rs:89-103`) on `vb_geo`, `vb_geo_mv` | yes, if `dxc` absent |
| `vb_sv0_spv_sync` | `redxc_with_defines` (`cluster_cull_spv_sync.rs:73-87`) over all 10 rows, extending `vb_froxel_spv_sync.rs` | yes, if `dxc` absent |
| `spirv-val` clean | pinned SDK `spirv-val`, all 10 | yes |
| image goldens | `goldens/PINS.toml` sha256 | no |

The skippable gates are **necessary but not sufficient** — hence the first row, which cannot skip.

---

## 6. Rungs

Ladder shape: **cheap CPU-only falsifier → fixture → cost falsifier → dark infra → device oracle
→ arm → measure.** Each rung is independently committable, has **one** gate, and names the
mutation that turns it red. *A mutation that is only argued does not count; the rung's commit
message records the mutated run's output.*

---

### S0 — the OFF-path instrument (CPU only, no GPU, no shader edit)

**Lands:** `crates/boyko_rhi_vulkan/tests/spv_dataflow.rs` — a test-only SPIR-V word-stream
parser producing, per entry point, a Merkle hash per result id and a hash for each store site.
No production code. ~200 LOC.

**Gate — the instrument is validated BEFORE it is believed** (three sub-assertions):

1. **self-match:** `hash(vb_resolve.comp.spv) == hash(vb_resolve.comp.spv)`.
2. **must-differ:** `hash(vb_shade.comp.spv) != hash(vb_shade_froxel.comp.spv)`.
3. **sensitivity:** copy `vb_resolve.comp.hlsl` to a temp dir, change **one float literal** in a
   region SV0 will never touch (e.g. the `1e-4` at `:258`), re-DXC under the frozen recipe, and
   assert the store-site hash **differs**.

**RED if:** (3) reports equal. Then the instrument is blind to exactly the class of change SV0
must exclude, the runtime-gate decision loses its proof, and the rung **escalates**: fall back to
`-D SV0=1` (+10 `.spv`, §3.2) as an owner VALUES call, or abort (§7).

**Skip policy:** (1) and (2) read committed bytes and **never skip**. (3) needs `dxc`; it skips
only when `dxc` is absent, and the rung is not commit-eligible until (3) has been *run* and its
output pasted into the commit message. A gate proven only on a box that skipped it is not a gate.

---

### S1 — the fixture (host test only, no shader edit) — **BLOCKING**

**The problem this rung exists for.** Every current VB golden has an **empty** SDF edit list:
`goldens/PINS.toml:322` (`vb_both`: *"This scene's SDF edit-list is boot-seeded EMPTY
(count == 0)"*) and `:355` (`vb_sdf_only`, same). By §4.4 the SV0 term on such a scene is exactly
`1.0` and byte-identity is *vacuous*. Arming against today's fixtures would produce a green gate
quantified over an empty selection — the campaign's #1 named defect, verbatim.

**Lands:** `crates/boyko_app/tests/vb_both_sdf.rs` — a clone of `vb_both.rs` with SDF primitives
actually spawned (the edit list is gathered by `collect_sdf_edits`,
`boyko_app/src/runner.rs:589`), positioned so at least one SDF body occludes the five-sphere
scene's key light and sits near a mesh surface. Plus a `[vb_both_sdf]` block in
`goldens/PINS.toml` (unblessed placeholder until owner sign-off).

**Gate:** the rendered frame is **NOT** byte-identical to `vb_mesh`'s
`f4719cbf13da5badb7a659d572d1817bbc45db683e5f0311f9bed8c933913ea1`
(`goldens/PINS.toml:304`), *and* the host reports `edit_count > 0`.

**RED if:** the frame equals `f4719cbf` (the SDF leg contributes nothing → the fixture is
vacuous), or `edit_count == 0`. **Mutation:** remove the SDF spawns → both assertions fail.
This is the control that proves the input reaches the thing under test — the lesson that a knob
which silently does nothing yields a perfectly flat curve.

---

### S1.5 — the cost falsifier (measurement, zero new shader code) — can kill the stage

**Lands:** a bench in `crates/boyko_app/tests/` that runs the **shipped Deferred** path on S1's
scene and performs an **interleaved paired A/B** of `pc.lighting_flags` with
`SHADOWS|AO` set vs cleared — i.e. exactly the `sdf_gbuffer_composite.hlsl:1865` gate around the
term SV0 will inline. Protocol, non-negotiable (the VB-P1d lesson): interleaved pairs, warmup
discarded, ≥30 pairs, report the **median paired delta** and the run-to-run spread across **3
sessions**. Sequential before/after measured a +9% phantom "regression" on this hardware that was
entirely session drift.

**Gate:** the measurement is **reproducible** — relative spread of the median across the 3
sessions ≤ 10%. The number itself is recorded as `SV0_DEFERRED_TERM_REFERENCE` in the test as a
literal, under the "MEASURED — do not edit these literals to make a failing run pass" discipline.

**RED if:** the spread exceeds 10% (the instrument is not trustworthy at this scale, and §7's
ABORT clause cannot be adjudicated — escalate before writing any shader). **Mutation:** point the
A/B at two *identical* configurations → the median paired delta must fall to ~0; if it does not,
the harness is measuring drift, not the term.

*Why this can kill the stage before a line of shader is written:* it measures the exact term, on
the exact fixture, using only shipped code. Under VB every covered pixel is a mesh pixel, so
SV0's coverage is a superset of Deferred's `!own_pixel` arm; if the term is already expensive
there, it will be worse here.

---

### S2 — dark infra (SV0 compiled in, host writes mode 0)

**Lands:**
1. `vb_geom_fetch.hlsli`: `#ifdef VB_SV0` `face_normal` field + computation (§4.2, §4.3).
2. All three tails: `#define VB_SV0` before the include; `[[vk::binding(10)]] Buf : register(t0)`;
   `#include "sdf_field.hlsli"` **after** `Buf`; the const block; the two generated leaf spans;
   `load_vb_sdf_mesh_mode` hoisted once per pixel; the guarded block with §3.3's duplicated
   `spec_ao` recompute.
3. `light_table.hlsli`: `load_vb_sdf_mesh_mode` + the two bit constants.
4. `boyko_render::light`: `VB_SDF_MESH_MODE_SHIFT/_MASK`, two `LightingConfig` fields (default
   **false**), packing in `shadow_gate_word`, bit-position `debug_assert_eq!`.
5. `gpu_scene/mod.rs:3395` and `:4425`: binding 10 entries.
6. `present/targets.rs:2995 / 3090 / 3193 / 3311`: `scene.edit_list` at slot 10.
7. Re-DXC + re-pin all 10 `.spv`; manifest notes.

**Gate (one, with four indivisible parts):**
(a) S0's instrument reports OFF-path dataflow equivalence for **all 10** re-pinned `.spv` against
their frozen predecessors; (b) `vb_geo.comp.spv` and `vb_geo_mv.comp.spv` **byte-identical**;
(c) every VB image golden byte-identical; (d) `spirv-val` clean on all 10.

**RED if / mutations (both DEMONSTRATED):**
* (a): delete the `if ((sv0_mode & …) != 0u)` guard so the block always runs → dataflow hash
  differs → red. Additionally: **omit the §3.3 duplicated `spec_ao` recompute** → the OFF-path
  `spec_ao` becomes a phi-fed expression → hash differs → red. *This second mutation is the one
  that matters: it is the failure the image golden in (c) would have passed.*
* (b): delete `#ifdef VB_SV0` from `vb_geom_fetch.hlsli` → `vb_geo.comp.spv` bytes change → red.

---

### S3 — the device oracle (still mode 0 in production)

**Lands:** three verification layers, no production behaviour change.

1. **Span pins** — extend `crates/boyko_rhi_vulkan/tests/sdf_field_edsl_sync.rs`: the
   `sdf_soft_shadow_ranged` and `sdf_ao` spans, the const block, and the `Buf @ t0` precondition
   are `.contains()`-asserted in **all three** tails.
2. **Leaf bit-exactness on device** — the leaf probe (the `field_probe_gate` /
   `cpu_gpu_sdf_agreement.rs` family) evaluates the leaves over a ≥4096-sample `(P, N, L)`
   fixture and compares **bit-exactly** to the host `boyko_shaderdsl` `Eval` backend
   (`sdf_field.hlsli:219-231` documents the shared-source relationship). Pure SSBO math, no
   sampler → demand exact `f32::to_bits()` equality, not a ULP tolerance.
3. **Host unit tests for the face normal** — under a non-uniform-scale affine, `face_normal`
   equals the analytic plane normal; under uniform scale it agrees in sign with the interpolated
   normal; a degenerate (zero-area) triangle returns the interpolated fallback, never NaN.

**Gate:** all three green.

**RED if / mutations:** (1) change one token in **one** tail's generated span → that tail's pin
fails (this is what makes "all three tails" mechanical rather than aspirational). (2) perturb one
host const in the `Eval` backend → bit-exactness fails. (3) reverse the `cross` operand order
without the orientation fix → the sign test fails.

---

### S4 — arm

**Lands:** the host resolver sets bits 5..6 when
`path_is_vb() && sdf_leg && mesh_leg && sdf_shadows_wanted && !hwrt` — mirroring
`SDF_SOFT_MARCH`'s existing predicate shape (`render_path_config.rs:727` per
`docs/RENDER-PARITY-PLAN.md:413-414`). Owner-facing toggle via `LightingConfig`.

**Gate — the arming control PAIR (both halves required; either alone is worthless):**
* **(i) no spurious perturbation:** with SV0 armed, `vb_mesh` (empty edit list) is **byte-
  identical** to `f4719cbf`. By §4.4 the term is analytically exactly `1.0`, so any difference is
  a bug — not a tolerance.
* **(ii) the input reached:** with SV0 armed, `vb_both_sdf` (S1's fixture) **differs** from its
  own mode-0 render, in a pixel count within `[1%, 60%]` of covered mesh pixels.
* **(iii) the split tail is covered:** (ii) re-run with `path_vb_split()` forced on.
* **(iv)** owner visual eval on the dumped BMP before any hash is blessed.

*(ii)/(iii) are used only to assert "the term reached", never to judge quality — image statistics
lie about render quality; the correctness verdict comes from S3's oracle and (iv).*

**RED if / mutations (DEMONSTRATED):**
* Force `sv0_mode = 0` host-side → (ii)'s changed-pixel count falls to 0 → red. *(Without (ii),
  (i) alone would pass trivially — that is the vacuous-gate shape this pair exists to defeat.)*
* **Revert only `vb_shade_split.comp.hlsl`'s SV0 block** → (iii) goes red while (ii) stays green.
  This is the structural closure of the three-tails P0 hole: the split path cannot be silently
  left out, because a gate exists that only the split can turn red.

---

### S5 — measure

**Lands:** an interleaved paired A/B of the VB lit-producer dispatch, SV0 armed vs `sv0_mode = 0`,
on `vb_both_sdf`, at 512×512, same protocol as S1.5 (≥30 pairs, warmup discarded, 3 sessions,
median paired delta + spread). Results pinned as literals in the test.

**Gate:** the measurement is reproducible (spread ≤ 10%) **and** adjudicated against §7.

**RED if:** the spread exceeds 10% — the number is not decidable at this scale and the ABORT
clause cannot be evaluated (the VB-P1d precedent: a single-sample ≤5% gate on a bench with 21%
run-to-run spread is not decidable, and shipping one manufactures confidence).

---

## 7. ABORT criteria

The stage is **reverted** — not softened, not re-scoped mid-flight — if any of:

1. **S0's instrument is blind** (S0 sub-assertion 3 reports equal) **and** the owner declines the
   `-D` fallback's +10 `.spv`. Without either, OFF-path inertness is unprovable and the 10
   re-pins ship on faith.
2. **S1's fixture cannot be made non-vacuous** — i.e. no VB×Both configuration produces a frame
   differing from `f4719cbf` with a non-empty edit list. Then there is no scene in which SV0 can
   be observed, and every downstream gate is vacuous by construction.
3. **Cost.** S5's median paired delta exceeds **2×** S1.5's measured `SV0_DEFERRED_TERM_REFERENCE`
   on the same fixture. The threshold is expressed against a **measured sibling that already
   ships this visual at an accepted cost**, not against a predicted number — the campaign's
   refuted-cost-model lesson. If the ratio lands in `[1×, 2×]`, it ships with the number recorded;
   above 2×, revert.
4. **S4 (i) cannot be made byte-identical.** A non-identical `vb_mesh` under an analytically-`1.0`
   term means the OFF/degenerate path is not inert, and no amount of re-blessing fixes that.

Revert granularity: every rung is independently committable, so an abort at S5 reverts S2–S4 and
keeps S0 (the instrument is reusable) and S1 (the fixture is a real coverage gain regardless).

---

## 8. Risks

Named first are the ones this campaign has actually hit.

| # | Risk | Precedent | Mitigation |
|---|---|---|---|
| R1 | **Vacuously-green gate** — assertion quantified over an empty selection. | Hit 3× in Stage 1. **Live here:** every VB golden has `edit_count == 0` (`PINS.toml:322`, `:355`). | S1 is blocking; S4's gate is a *pair* where one half proves the input arrived. |
| R2 | **Sub-LSB drift invisible to the golden.** | Named concretely at `vb_resolve.comp.hlsl:288-289`. | S0's dataflow instrument + §3.3's duplicated `spec_ao` recompute; S2's second mutation targets exactly this. |
| R3 | **Cost model instead of measurement.** | The refuted `a + b*(froxels*N)` model. | No predicted number in any gate. S1.5 and S5 are measurements; §7's threshold is a ratio to a measured sibling. |
| R4 | **Session drift read as a regression.** | +9% phantom on this hardware. | Interleaved paired A/B, warmup discarded, 3 sessions, spread reported — enforced in S1.5 and S5. |
| R5 | **Instrument that silently does nothing.** | The flat-curve knob. | S0 validates itself against a deliberately mutated recompile before it is trusted; S1.5's harness has its own null-mutation check. |
| R6 | **Silent OOB with `robustBufferAccess` OFF.** | No layer reports it. | SV0 adds **no** new indexing — `Buf[0]` is already clamped by `min(Buf[0], MAX_SDF_EDITS)` (`sdf_field.hlsli:204`). Binding 10 is always a valid descriptor (`scene.edit_list` is non-`Option`). |
| R7 | **`NMin`/`NMax` NaN semantics.** | HLSL `min`/`max` → `NMin`/`NMax`, returning the non-NaN operand. | Exploited deliberately twice: it guarantees march termination (§4.4) and makes a degenerate term degrade to "no contribution" rather than poison the pixel (§4.5). Asserted in S3. |
| R8 | **Two call sites of one function optimised independently** (no `-O` in the frozen recipe; DXC defaults to `-O3`). | Stage 1 record. | SV0 has exactly **one** call site per leaf per tail. The `spec_ao` duplication in §3.3 creates a second site *by design*, and S0's per-store-site hashing compares the OFF path's site specifically. |
| R9 | **The split tail silently omitted.** | The P0-class hole this design closes. | S4 gate (iii) + its demonstrated revert-only-the-split mutation; S3's per-tail span pins. |
| R10 | **Non-uniform scale.** | `vb_geom_fetch.hlsli:539-542`. | Out of scope and stated plainly (§4.2). The bias is robust; the shading normal is not. |
| R11 | **Edit list becomes per-frame dirty** → a missing barrier under VB. | — | §2.4's `debug_assert!` at the upload site fails loudly on the change that would introduce it. |
| R12 | **`dxc`-dependent gates skip.** | `cluster_cull_spv_sync.rs:197-205`. | The one decisive gate (S0) reads committed bytes and cannot skip. Skippable gates are labelled necessary-but-not-sufficient in §5.2, and S0(3)'s output is pasted into the rung's commit message. |

---

## 9. Appendix — verified file:line anchors

Every line below was opened while writing this plan.

**Field / leaves:** `sdf_field.hlsli:41` (`FAR = 1.0e9`), `:42` (`GRAD_H = 0.0005`), `:203-217`
(`sdf`, edit-count clamp at `:204`), `:246` (`field_distance` gateway), `:256-287` (lower-bound
invariant; `FIELD_LIPSCHITZ_L` at `:287`) · `sdf_gbuffer_composite.hlsl:488-490` (AO consts),
`:498-526` (`sdf_soft_shadow`), `:532-540` (`sdf_ao`), `:1853-1885` (the Deferred mesh arm),
`:1876-1878` (shadow), `:1881` (AO).

**Binding precedent:** `deferred_pbr.hlsl:153-161` (binding 10 + include contract; the decl at
`:161`), `:444-447` (`#include` after `Buf`), `:466-474` (consts; `SHADOW_NORMAL_BIAS` at `:474`),
`:479` (caster cap), `:506-532` (generated `sdf_soft_shadow_ranged`; `MAX_IT` loop `:519`,
advance `:525`, `t_max` break `:526`), `:74-79` (frozen-base `#ifdef` discipline).

**VB tails:** `vb_resolve.comp.hlsl:40-70` (bindings doc), `:84-85` (includes), `:95/99/111/119/123/127`
(bindings 1..6), `:151-154` (`#ifdef FROXEL` 8/9), `:192` (`shadow_apply.hlsli`), `:241` (sentinel),
`:249-252` (`geo`/`n`/`P`), `:288-289` (`ao_final`/`spec_ao` — the R2 hazard), `:297` / `:308-315`
(the primary-directional `vis` site) · `vb_shade.comp.hlsl:50/68/87/90/163` · `vb_shade_split.comp.hlsl:51-55`
(HWRT denoised combine), `:68-108` (4-set bindings), `:120-133` (dxc recipe), `:136-137` (includes) ·
`vb_geo.comp.hlsl:118` (`#include "vb_geom_fetch.hlsli"`).

**Geometry fetch:** `vb_geom_fetch.hlsli:20-34` (Set-numbering deviation), `:44-51`
(`VbInstanceRow`, binding 0/0), `:478-494` (`VbGeomFetchResult`), `:516` (signature), `:533-538`
(`m3`, world positions), `:539-545` (**the plain-`m3` normal limitation**).

**Header word 7:** `light_table.hlsli:55-56`, `:77-79`, `:91-93`, `:109-111`, `:128-130`,
`:145-147`, `:149-165` ("Bits 5..7 stay free" at `:154`), `:179-180` ·
`boyko_render/src/light.rs:386-408` (bit budget; "5..7 (free)" at `:406`), `:420` `CSM_MODE_BIT` ·
`ddgi_config.rs:288-289` (the single-writer `debug_assert_eq!` idiom).

**Host layouts / sets:** `gpu_scene/mod.rs:3395-3459` (`vb_layout0`, 8 entries; `gClassify` at
`:3450-3455`), `:4425-4492` (`vb_layout0_froxel`, 10 entries), `:4096-4212` (split pipelines
against `vb_layout0`) · `present/targets.rs:2995-3079` (`vb_set0`; entries `:3012-3024`), `:3090`,
`:3193`, `:3311`, `:1402` (`scene.edit_list` precedent) · `rhi_impl/mod.rs:93`
(`MAX_BIND_GROUP_BINDINGS = 24`) · `boyko_app/src/runner.rs:589`, `:1136-1141` (one-shot
edit-list upload).

**Variants / gates:** `compute.rs:811-1037` (the 10 embeds) ·
`docs/SHADER-VARIANT-MANIFEST.md:91-107` (the VB table; "`vb_resolve.comp.hlsl` has no TEXTURED
variant" at `:84`) · `tests/cluster_cull_spv_sync.rs:73-87` (`redxc_with_defines`), `:89-103`
(`assert_spv_byte_identical`), `:105` (`spirv-dis` lookup), `:197-205` (the skip path) ·
`tests/vb_froxel_spv_sync.rs`, `tests/sdf_field_edsl_sync.rs`.

**Goldens:** `goldens/PINS.toml:273-311` (`vb_mesh`; hash `f4719cbf…` at `:304`), `:313-343`
(`vb_both`; **empty edit list at `:322`**), `:345-377` (`vb_sdf_only`; **empty at `:355`**),
`:288-291` (VB≠Forward FP-path note), `:379-421` (`vb_mesh_tex`).

**Stale row:** `docs/RENDER-PARITY-PLAN.md:351` (SV0), `:366` (Contact-AO "out of scope — charted
follow-up"; SV0 closes it for VB), `:383-384` (the O3 sync-pin discipline), `:404-406`, `:413-414`.

---

## 10. Open questions (VALUES/SCOPE — owner)

1. **Default state.** Ship SV0 default-OFF (opt-in via `LightingConfig`) or default-ON when
   `path_is_vb() && sdf_leg && mesh_leg`? Deferred's equivalent is push-constant-flag driven.
   *Recommendation: default-OFF through S4, flip after S5's number is known.*
2. **`-D` fallback authorisation.** If S0 sub-assertion 3 fails, is +10 `.spv` acceptable, or is
   that an abort? (§7 clause 1.)
3. **S1 fixture composition.** The new `vb_both_sdf` scene needs SDF bodies placed to produce a
   visible shadow on the five spheres. Owner may prefer reusing `grand_showcase`'s SDF
   arrangement rather than a purpose-built one.

