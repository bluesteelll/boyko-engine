# Shader `-D` Variant Manifest

**Single source of truth for the monolithic `-D`-preprocessor shader variants** that *cannot*
collapse to one `.spv` (they change the descriptor set / capability / emitted code), so they stay
**N `.spv` compiled from ONE `.hlsl`**. This is the A-3 half of the shader-growth remediation (see
[REFACTORING-PLAN.md](REFACTORING-PLAN.md) §A3): the registry de-dup (A-2 `embed_spirv!` macro) removed
the hand-counted embed sizes, and this table removes the *scavenger hunt* — a new variant is a row here,
not archaeology through `compute.rs` doc-paragraphs.

Contrast with the **spec-constant-collapsible** families (`GI_MAX_IT` — SHIPPED @3c10826) and the
**perf-justified** ones (SSAO quality — `[unroll]`, kept as 3 files; see the plan). A `-D` variant belongs
HERE only if it changes the *interface* (adds/removes a binding, a capability, or half the shader) — a
spec-constant cannot do that. (That SSAO *quality* axis is a source-text re-emit, not a `-D`, and
correctly stays out of the tables — but the SAME shader's `VB_THIN` axis IS a `-D` and DOES delete a
binding, so it has its own section below. Do not read "SSAO quality is excluded" as "the SSAO gather
has no variant axis".)

## The three axes

| Axis (`-D`) | Values | What it changes |
|---|---|---|
| `SHADOW_STAGE` | `RESOLVE_INLINE` (unset/0), `VIS` (1), `DENOISED` (2) | How the mesh shadow term is produced/consumed. INLINE combines the trace into lighting directly; VIS **writes** `gShadowVis` (the à-trous/temporal pre-pass) and strips lighting; DENOISED **reads** the final denoised `gShadowVis` and combines it. |
| `HWRT` | unset (0), `1` | 0 = software SDF-marched mesh shadows (no ray HW). 1 = hardware inline `rayQuery` against a TLAS — adds `OpCapability RayQueryKHR`, the acceleration-structure descriptor, and **requires `-T cs_6_5`** (vs `cs_6_0`). Gated by `feature = "hwrt"` + a runtime `ctx.ray_query_enabled()`. |
| `MOTION_VECTORS` | unset (0), `1` | 0 = no motion output. 1 = also emit per-pixel motion (Δuv) for the temporal shadow denoiser — a new storage binding (deferred) or a 4th MRT (gbuffer). Static camera ⇒ (0,0). |

## `deferred_pbr.hlsl` — the fullscreen deferred resolve (compute)

One source `shaders/deferred_pbr.hlsl`; the host selects a variant by **binding a different pipeline**,
never a dynamic branch. All share the base 0..11 binding block (G-buffer STORAGE images, material SSBO,
camera UBO, light table, cluster grid, SDF edit-list, SSAO). The deltas:

| Variant | `SHADOW_STAGE` | `HWRT` | `MV` | `TW` | `.spv` | dxc `-T` | Interface delta vs base (0..11) |
|---|---|---|---|---|---|---|---|
| RESOLVE_INLINE (software) | — | — | — | — | `deferred_pbr.comp.spv` | `cs_6_0` | none — software SDF `sdf_soft_shadow_ranged`; +CSM `gCsm`/`gCsmCmp` @12/13/14 + punctual atlas + (bound-unread) DDGI @16/17/18. |
| TERMINATOR_WRAP (software) | — | — | — | `1` | `deferred_pbr_wrap.comp.spv` | `cs_6_0` | **none — the interface is identical to the base row**, which is why it reuses the same `resolve_layout` (`gpu_scene/mod.rs:1654-1656`). The delta is arithmetic: `nol_wrapped` (`deferred_pbr.hlsl:587`) replaces the raw `NoL` at BOTH direct-diffuse accumulation sites — the directional one at `:1140` and the punctual one at `:1353` — specular keeping the physical clamp at each. Frozen-base discipline — with the flag undefined the source preprocesses **character-identical** to the pre-feature file, so the base row's `.spv` is untouched by construction. |
| RESOLVE_INLINE (hardware) | — | `1` | — | — | `deferred_pbr_hwrt.comp.spv` | `cs_6_5` | `+RaytracingAccelerationStructure gTlas` @19 + `OpCapability RayQueryKHR`; the `#if HWRT` Vogel-disk cone trace (`SHADOW_RAY_COUNT` spec-const) replaces the software march. |
| VIS | `1` | `1` | — | — | `deferred_pbr_hwrt_vis.comp.spv` | `cs_6_5` | hwrt layout **+ `RWTexture2D<float2> gShadowVis`** @21 (**write** `RG(mesh_vis, validity)`); lighting stripped (writes vis, not lit). The à-trous/temporal pre-pass. |
| DENOISED | `2` | `1` | — | — | `deferred_pbr_hwrt_denoised.comp.spv` | `cs_6_5` | same 22-binding VIS/DENOISED layout, but `gShadowVis` @21 is **read** (`mesh_vis = gShadowVis.Load().r`, the final denoised output) and combined `vis = min(vis, mesh_vis)`. Declares NO acceleration structure. |
| VIS + motion | `1` | `1` | `1` | — | `deferred_pbr_hwrt_vis_mv.comp.spv` | `cs_6_5` | VIS layout **+ `MotionCam` UBO** @22 **+ `RWTexture2D<float2> gMotionVec`** (`rg16f`, SIGNED) @23 — writes clip-space Δuv for the temporal reproject. |

`TW` = `TERMINATOR_WRAP`. Reachability note: `SHADOW_STAGE ∈ {VIS, DENOISED}` and `MOTION_VECTORS=1`
are only reachable **with** `HWRT=1` (the spatial/temporal shadow-vis denoise pipeline is built on the
hardware mesh-shadow trace); there is no software VIS/DENOISED/MV `.spv`. `TERMINATOR_WRAP` is
**software-resolve-only** and mutually exclusive with `HWRT` by scope decision — the combination is
never compiled and never selected (`deferred_pbr.hlsl:76-79`). The wrap variant is built
**unconditionally** (its accessor `compute.rs:1470` carries no `#[cfg(feature = "hwrt")]`, unlike the
four HWRT ones) and the host binds it only when `LightingConfig::terminator_softening > 0`; the
variant itself is the opt-in, so the wrap arm carries no runtime `if`.

*Provenance of this row: it was **missing** until 2026-07-26 — the variant has shipped since its own
introduction, so this table has been incomplete for that whole time. Found while enumerating the
family for the VB-SV0 gate that must prove all six unperturbed
(`docs/VB-SV0-SDF-SHADOW-PLAN.md` §5.4); all six were re-DXC'd byte-identical against the committed
artifacts at that time.*

*Byte gate (added at rung VB-P1k): all six rows — and both `forward_opaque.fs.hlsl` rows below —
are now re-DXC'd and byte-compared by `crates/boyko_rhi_vulkan/tests/cluster_grid_read_bound.rs`.
Until that rung these two families had **no `*_spv_sync` test at all**, so a stale artifact was a
silent failure rather than a loud one. Four of the six rows moved at VB-P1k (the `use_clusters`
capacity bound); `deferred_pbr_hwrt_vis` and `deferred_pbr_hwrt_vis_mv` did not, because
`SHADOW_STAGE=1` returns before lighting and DXC dead-strips the whole cluster block — the same
reason their `OpArrayLength` census row is 0.*

## `gbuffer_mrt.{vs,fs}.hlsl` — the mesh G-buffer raster

| Variant | `MV` | `MAT` | `.spv` | Interface delta |
|---|---|---|---|---|
| base | — | — | `gbuffer_mrt.vs.spv` / `gbuffer_mrt.fs.spv` | 3 MRT attachments (albedo / normal+id / material). |
| motion | `1` | — | `gbuffer_mrt_mv.vs.spv` / `gbuffer_mrt_mv.fs.spv` | **+ a 4th MRT** carrying per-pixel Δuv (prev-instance ring @1 + `MotionCam` UBO @2); static instance+camera ⇒ (0,0). |
| material | — | `1` | `gbuffer_mrt_pm.vs.spv` / `gbuffer_mrt_pm.fs.spv` | per-instance material PAYLOAD SSBO @1 (VERTEX) — `gAlbedo` sources the material's `base_color`, `gNormal.BA` packs the real material id; no 4th MRT. |
| motion + material (F8-mv) | `1` | `1` | `gbuffer_mrt_mvpm.vs.spv` / `gbuffer_mrt_mvpm.fs.spv` | both deltas above, combined: the 4th MRT Δuv AND the material-driven albedo/id. The nested `#if defined(MOTION_VECTORS)` inside the `PER_INSTANCE_MATERIAL` block moves `instance_materials` from binding 1 → binding 3 (bindings 1/2 stay `prev_instances`/`MotionCam`) to resolve the collision — the `motion`/`material` rows' own `.spv` are untouched by this move (the `#else` arm is byte-identical to their source). |

Reachability note: all four rows are independently reachable (`motion`/`material` are each opt-in
via one `-D`; `motion + material` needs both) — the host selects among them by binding a different
pipeline per frame (never a dynamic branch), gated on `mesh_mvpm_active()` checked BEFORE
`mesh_mv_active()`/`mesh_pm_active()` at the recorder's selection site (priority mvpm > mv > pm > base).

## `particle_draw.{vs,fs}.hlsl` — the GPU particle billboard draw (raster)

One source per stage (both GENERATED — `boyko_shaderdsl/src/bin/emit_particles.rs`; never
hand-edited), two artifacts each. The axis is `DEPTH_LINEAR`, and the variant exists because
**the Deferred path's depth buffer does not hold depth**: `gbuffer_mrt.fs.hlsl` overwrites it
through `SV_Depth` with the marcher-aligned euclidean encode, while the projection that path hands
the particle VS is the marcher's, whose `row2 == row3` pins every billboard vertex to
`SV_Position.z == 1.0`. Under that path's `VK_COMPARE_OP_LESS` the base compile fails on **every**
pixel, sky included — MEASURED at P0 live fire (`docs/PARTICLES-PLAN.md`, the P0 live-fire
erratum), and not fixable host-side (`z_ndc` is a ratio of affine functions of the world position;
a euclidean norm is not one).

| Variant | `DL` | `.spv` | dxc `-T` | Interface delta vs base |
|---|---|---|---|---|
| base (reverse-Z paths) | — | `particle_draw.vs.spv` / `particle_draw.fs.spv` | `vs_6_0` / `ps_6_0` | hardware depth from the VS's projective `SV_Position.z`; the FS writes colour only, so **early-Z stays live**. Bound by Forward / ForwardPlus / VisibilityBuffer (`VK_COMPARE_OP_GREATER`). |
| depth-linear (Deferred) | `1` | `particle_draw_dlin.vs.spv` / `particle_draw_dlin.fs.spv` | `vs_6_0` / `ps_6_0` | **no descriptor delta — the layout object is unchanged** (the `deferred_pbr_wrap` precedent): the VS forwards two extra interpolants (`eye_rel` = `cam_eye.xyz - world`, perspective-correct, and `cam_mode`) read from the ALREADY-BOUND camera UBO @1, and the FS gains `SV_Depth` = `(cam_mode > 0.5) ? length(eye_rel) / MESH_DEPTH_T_MAX : position.z` — term for term `gbuffer_mrt.fs.hlsl:327`. **COST 1: an `SV_Depth` write disables early-Z on this leg** (the value the test needs does not exist until the shader has run); accepted, and bounded by a fragment that is one modulate + at most one bindless sample. `depth_write` stays OFF, so nothing is stored. **COST 2 — a per-path DIVERGENCE: the encode's range is the particle FAR HORIZON here.** Beyond `MESH_DEPTH_T_MAX = 64` world units the quotient exceeds 1, the write clamps to `[0,1]`, and `LESS` then fails against anything stored (the 1.0 sky clear included) — so Deferred particles vanish at 64 units while the base rows carry them to the camera's far plane. Same horizon this path's raster meshes already have (same divisor); moving it means moving both sites and re-blessing every Deferred pin. |

`DL` = `DEPTH_LINEAR`. Selection is **boot-frozen, once per process**: `particle_draw_spirv_for`
and `particle_depth_compare_for` (`boyko_app/src/gpu_scene/particle.rs`) are two answers to one
question — what does this path's depth image hold — and take the SAME `deferred_path` predicate,
so exactly one `VkPipeline` exists per run and the two answers cannot disagree.

*Byte gate:* `crates/boyko_rhi_vulkan/tests/particle_edsl_sync.rs` re-DXCs **all twelve** particle
artifacts under the recipes their headers pin and byte-compares. The base rows are load-bearing in
both directions: they prove the `#ifdef` leaves the undefined compile byte-frozen (it does —
verified at the landing). The same file pins the encode agreement itself
(`particle_depth_linear_encodes_exactly_what_deferred_s_depth_buffer_holds`): the two shaders'
depth right-hand sides are compared as text, and `MESH_DEPTH_T_MAX` / `CAM_MODE_PERSPECTIVE` are
pinned against their host consts. A drifted normalizer or arm-select would mis-occlude with nothing
in the image to say so.

## `particle_sim.comp.hlsl` — the GPU particle hot loop (compute)

One source (GENERATED — `boyko_shaderdsl/src/bin/emit_particles.rs`), **three** artifacts. The axis is
`SDF_COLLIDE`, rung P1 of `docs/PARTICLES-PLAN.md` (D9), plus rung P1b's `SDF_COLLIDE_STATS` stacked
on top of it; both are `-D` rather than runtime flags for the F24 reason this plan cites everywhere:
a field consumer's `#include`d code and register pressure — and, for the instrument, its atomics —
would otherwise be paid on every frame of every scene that does not use them.

| Variant | `SC` | `SCS` | `.spv` | dxc `-T` | Interface delta vs base |
|---|---|---|---|---|---|
| base (no collision) | — | — | `particle_sim.comp.spv` | `cs_6_0` | the shipping module: Set-0 bindings 0/2/3/4/5/6/7/9, **12 B push** (`steps`, `timestep`, `capacity` — the third arrived at rung P2 as the alpha class's render-index mirror), **four** wave-leader `OpAtomicIAdd`, **zero `OpFDiv`**. `cached_field_d` is written 0 at spawn and never read. |
| sdf-collide (P1) | `1` | — | `particle_sim_sdf.comp.spv` | `cs_6_0` | **+`StructuredBuffer<uint> Buf` @10** — the engine's ONE SDF edit list, the same binding number every other field consumer uses (`sdf_mesh_shadow.comp.hlsl:97`), followed by `#include "sdf_field.hlsli"`. The buffer is boot-static and read-only for the whole present loop, so it is **not** a framegraph resource: no `ResId`, no seed row, no barrier, and `particle_barrier_stream` is byte-unmoved by this rung. Push range, layout object, descriptor sets and atomic census are all UNCHANGED against the base row of its own generation (collision publishes nothing — it moves state one lane already owns exclusively; the 3 → 4 move above is the base module's, and this variant inherits it). The one census that moves is `OpFDiv`: 0 → **28**, all of them the frozen field header's (7 `sdf` instantiations — 1 for `field_distance`, 6 for `sdf_normal` — × 4 divides each: `smin`, both `smax` arms, `sd_capsule`). The particle-owned text adds none, pinned source-side. |
| **sdf-collide + skip census (P1b)** | `1` | `1` | `particle_sim_stats.comp.spv` | `cs_6_0` | **ZERO interface delta against the row above** — same nine bindings, same 12 B push, same layout object, same descriptor sets, same 28 divides, same simulation. What it adds is the SKIP-RATE INSTRUMENT: one `WaveActiveCountBits` ballot on the collide arm's own skip predicate, folded by one `WaveIsFirstLane()` into three counter words (`waves_evaluated` @7, `waves_skipped` @8, `lanes_evaluated` @9) carved out of `ParticleCounters`' pad — so it needs no buffer, no `ResId` and no barrier, and rides the existing `particle_counters_readback` channel. **⚠️ CENSUS EXCEPTION — see below. A MEASUREMENT module: nothing in a pinned boot selects it.** |

`SC` = `SDF_COLLIDE`, `SCS` = `SDF_COLLIDE_STATS` (which implies `SC`: the census instruments the
collide arm and has nothing to count without it). Selection is **boot-frozen, once per process**:
`particle_sim_spirv_for` (`boyko_app/src/gpu_scene/particle.rs`) takes the `ParticleCollision` enum
in a wildcard-free match, so exactly one sim `VkPipeline` exists per run. Binding 10 is in the host
layout table **on every arm** — bound-but-unread under the base module, the same shape the marcher's
`tiles_buffer`/`PointerGrid` bindings have — so the pick never reaches the descriptor plumbing.

### ⚠️ The shipping budget moved 3 → 4 at rung P2 — a re-bless, not a drift

D10's blend partition specifies one `InterlockedAdd` RENDER counter **per class** (the two classes
take their positions from opposite ends of `p_render`, so a single shared counter cannot yield both),
so the sim gained the `alpha.instanceCount` site. The per-wave budget is now 1 (all dying) / 2 (all
surviving, ONE class) / 3 (surviving, BOTH classes) / 4 (that, mixed with dying), and each site is
`> 0u`-guarded — so a wave carrying no alpha survivor, which is every wave of every additive-only
scene, issues exactly the three it always did. It is **not** a widening of the aggregation: still one
op per wave per counter, never one per lane. `SIM_WAVE_LEADER_ATOMIC_SITES` in `particle_edsl_sync`
carries the number and the reason.

### ⚠️ The atomic-census exception, stated on the row that takes it

`particle_sim_stats.comp.spv` runs **3–6 atomics per wave against the shipping modules' 1–4, BY
DESIGN.** Re-derived here rather than transcribed, because the two bounds move differently:

| | retirement (the budget above) | + census, per substep | total |
|---|---|---|---|
| **lower** — all-surviving, ONE class, every lane skips the field | 2 | 1 (`waves_skipped`) | **3** |
| **upper** — mixed survive/die, BOTH classes, some lane evaluates | 4 | 2 (`waves_evaluated` + `lanes_evaluated`) | **6** |

The census adds **1–2 per wave per substep** — one for the wave counter (the two arms are exclusive:
a wave that evaluates counts as evaluated, never as both) plus one for `lanes_evaluated` on the
evaluating arm — and the plan's steady state is ONE substep per dispatch, which the host hard-refuses
to violate for a census frame.

**Rung P2 moved the UPPER bound only, 5 → 6.** The lower stayed at 3, because an additive-only wave
still takes exactly 2 retirement atomics — the same headline argument the shipping row makes.

Statically the artifact carries **7** `OpAtomicIAdd` sites (4 retirement + 3 census) against the
shipping modules' 4. The static count and the per-wave count are different quantities and are stated
separately on purpose: 7 is what an opcode census can see, 3–6 is what the device runs, and putting
the static number in a per-wave row is the transcription error this table exists to prevent.

The architect's reason, recorded rather than paraphrased: **a census that forbids the instrument is a
census that forbids measuring itself.** Gate #17 measured that the `ZONE_PARTICLE_SIM`
armed-vs-disarmed delta — which the plan had named as the skip-rate instrument — is dominated by a
kernel-level term of the OPPOSITE sign at 4–6× the row's resolution, so the rate is not recoverable
from a timing difference at all and a device-side counter is the only remaining instrument.

**The exception is NOT a widening.** `particle_sim_atomic_census_is_exactly_the_wave_leader_sites`
and `the_collide_variant_adds_no_atomic` still assert an EXACT count for the two modules that ship
(**4** since rung P2), and were deliberately left alone: a bound of "4 or 7" would stop gating the
modules D5's budget is load-bearing for. The instrument's own bound lives in its own test
(`the_stats_variant_carries_its_declared_census_exception_and_nothing_more`), which asserts 4 + 3 and
**no other atomic of any kind** — a fourth census site would mean a counter nobody derives a rate
from, and a second `OpAtomic*` species would mean the census reached for a primitive D5's
aggregation argument does not cover.

*Byte gate:* the same `particle_edsl_sync` battery. Beyond the three byte rows it pins each variant's
binding set (`[0,2,3,4,5,6,7,9,10]` for both collide rows against the base's `[0,2,3,4,5,6,7,9]` —
DXC strips a declared-but-unread resource, so this is a real "the field is actually consumed"
claim), that rung P1's variant adds **no** atomic, the divide count above (unmoved by the census —
it counts, it does not divide), the collide block's own hand-written skeleton (the include contract,
the skip test's direction, the cache write-back), the census block's skeleton, and — the pin that
makes the instrument trustworthy — that the census **ballots on the branch's own predicate**: DXC
folds the re-spelled test into ONE `OpFOrdGreaterThan` feeding both the ballot's `OpLogicalNot` and
the branch, so the two are provably the same value. The `WaveIsFirstLane()` fold is pinned at the
artifact too (+1 `OpGroupNonUniformElect`, +1 `OpGroupNonUniformBallotBitCount` against the collide
module), because a census that lost its leader would still count correctly while running 32× the
atomics. The selector itself is pinned in-crate by identity AND by artifact property
(`the_collide_arm_takes_the_sdf_module_and_the_base_arm_does_not`,
`the_stats_arm_takes_the_instrumented_module_and_the_shipping_arms_do_not` — the latter separates the
instrument from the module it measures by ATOMIC POPULATION, since the two share every binding),
because a swapped arm is invisible to every text and byte pin in the tree.

## `particle_sort_{hist,scan,scatter}.comp.hlsl` — the alpha class's radix sort (compute)

**Three sources, three artifacts, and ZERO `-D` rows — which is why this section exists at all.**
Rung P2 item 3 (plan D10) is the first particle feature whose arming is resolved into *separate
pipelines* rather than into a define, and recording that here is the point: a reader looking for
"where is the sort's variant row?" must find the answer *no such row exists* rather than conclude
the manifest is stale.

The sort is a boot arming (`ParticleSortMode`, default `None`), and everything it adds is
structurally absent when it is off — two device buffers, three compute pipelines, one descriptor
ring, three framegraph passes. There is nothing to make conditional *inside* a shader, so there is
nothing for a define to gate. The three modules are unconditional compiles of three whole files:

| Source | Artifact | Profile | Bindings (declared ∧ read) | Push | Atomics (device, workgroup) |
|---|---|---|---|---|---|
| `particle_sort_hist.comp.hlsl` | `particle_sort_hist.comp.spv` | `cs_6_0` | `[2, 7, 12]` | 16 B (`float3 cam_eye`, `uint capacity`) | **(1, 1)** |
| `particle_sort_scan.comp.hlsl` | `particle_sort_scan.comp.spv` | `cs_6_0` | `[12]` | **none** | **(0, 0)** |
| `particle_sort_scatter.comp.hlsl` | `particle_sort_scatter.comp.spv` | `cs_6_0` | `[2, 7, 11, 12]` | 16 B (the same block) | **(1, 2)** |

Bindings 11 (`p_render_sorted`) and 12 (`p_sort_bins`) are new Set-0 rows; on an unsorted run they
are filled with live PLACEHOLDERS (`p_render`, `p_counters`) so ONE layout serves both armings —
the bound-but-unread shape rung P1's edit list at binding 10 already established.

**⚠️ The atomic pair is the budget claim, and the WORKGROUP half is what discriminates.** The
dangerous edit is a scatter that reserves per ELEMENT rather than per occupied bin — it sorts
correctly, passes the monotonicity readback, and costs up to 256× the global traffic (D5's
~0.5 ms/frame-at-1M shape). That form carries **one device atomic too**, so a device-only bound
cannot see it; it reports `(1, 0)`, because it needs no LDS phases at all. **Proven by mutation
2026-08-21**: rewriting the scatter to the per-element form reddens
`the_sort_modules_carry_their_derived_atomic_budget` on the workgroup count and nothing else in the
battery.

**The scan's re-zero is why `particle_kickoff` needed no variant.** `p_sort_bins` is one allocation
in two halves — histogram `[0, 256)`, running offsets `[256, 512)` — and the scan reads its own bin
into a register, publishes the exclusive prefix into the offsets half, and writes `0u` back to the
histogram half in the same dispatch. So the histogram is clean for the next frame without any
shipping shader learning that the sort exists; the alternative was a `-D` variant of a module that
ships in every configuration, i.e. a thirteenth artifact to zero 1 KB. Frame 0 works because the
boot fill zeroes the buffer, and the seed row's WRITER constructor is what makes the re-zero
*visible* to the next frame's accumulate.

*Byte gate:* the same `particle_edsl_sync` battery — three re-DXC byte rows, the three binding sets
above, the three declared widths (all `256`, which must equal the bin count: the modules are
one-bin-per-lane), zero `OpFDiv` in all three (the octave span's reciprocal is host-computed and
printed as a literal), the atomic pair above, the scan's re-zero and its read-before-zero ordering,
the scatter's mirror + F25 clamp + one-load-per-element traffic claim, and — the pin that makes the
two key-bearing modules one module — that `particle_sort_key` is **character-identical** in the
histogram and the scatter. A key that differed between them would size a bin from one population
and fill it from another. The range constants (`SORT_NEAR`, `SORT_LOG_NEAR`, `SORT_INV_LOG_SPAN`,
`SORT_BINS`, `SORT_BIN_MAX`, `SORT_OFFSET_BASE`) are pinned against
`boyko_rhi_vulkan::compute::PARTICLE_SORT_*`, because the host recomputes the key for the
monotonicity readback and a drifted constant would make that instrument lie in both directions.

*Not yet built:* the remaining particle `-D` rows the plan schedules — `SOFT` (P2), `LIT_PERPIXEL` /
`MOTION` (P3), `PARTICLE_INTERP` (P2b). Each gets its own row here when it lands. `SortMode::Wboit`
is **not** on that list: D10 keeps it as an opt-in, but it is a different technique (a weighted
order-independent accumulation, not a permutation) and would be its own rung, not a define.

## `ui_rect.{vs,fs}.hlsl` — the UI rect/text draw (raster)

One source per stage (both GENERATED — `boyko_shaderdsl/src/bin/emit_ui.rs`; never hand-edit the
`// === GENERATED … ===` spans), **NO `-D` axis** — each source compiles to exactly ONE artifact.
The rows exist because the workspace rule ("HLSL the eDSL owns is generated, never hand-edited;
committed `.spv` are byte-gated") did not bind these files before UI-ADVANCED rung S1
(`docs/UI-PLAN-SPRITES.md`; architecture D30): the only pin on the two binaries was the
const-generic byte LENGTH (`SpirvBlob<2368>` / `SpirvBlob<7060>`, `boyko_render/src/ui/mod.rs`),
which cannot see a re-compile drift at the same size.

| Variant | `-D` | `.spv` | dxc `-T` | Notes |
|---|---|---|---|---|
| the only one | — | `ui_rect.vs.spv` | `vs_6_0` | vertexless unit quad from `SV_VertexID`; per-instance transform read from `StructuredBuffer<UiInstance>` @ set 0 binding 0 (VERTEX-stage SSBO read); pixel→NDC ortho push constant (16 B). The `UiInstance` mirror span is generated from the `UiInstanceLayout` GENERATOR INPUTS and pinned to the HOST `offset_of!` by `ui_rect_edsl_sync`. |
| the only one | — | `ui_rect.fs.spv` | `ps_6_0` | rounded-box SDF + `fwidth` AA + uniform border (premultiplied over) + MSDF text branch (`FLAG_TEXT`) + **SPRITE branch (`FLAG_TEXTURED`, UI-ADVANCED S3)** + flag-gated clip; PREMULTIPLIED output (src=ONE blend). Six eDSL leaves (`boyko_shaderdsl::ui`): `ui_unpack_rgba8`, `ui_sd_rounded_box`, `ui_clip_coverage`, `ui_median3`, `ui_screen_px_range`, `ui_premultiplied_over`. **TWO descriptor sets since S3:** set 0 = the ring SSBO @0 (VERTEX\|FRAGMENT), the MSDF atlas @1, the per-atlas UBO @2, the UI's OWN sprite sampler @3 (FRAGMENT, a `COMBINED_IMAGE_SAMPLER` whose SAMPLER half alone is read — S-D4); set 1 = the shared bindless `Texture2D g_sprites[]` @0, indexed by `NonUniformResourceIndex` from `flags` bits 20..31. The stage STATICALLY uses set 1, so every UI pipeline is 2-set and every UI draw binds set 1 — a rect-only draw included. |

Frozen recipe (each source's header pins it verbatim; no `-O`, no `-D`):
`dxc.exe -spirv -T {vs_6_0|ps_6_0} -E main -fspv-target-env=vulkan1.3 ui_rect.{vs|fs}.hlsl`.

*Byte gates:* `boyko_render/tests/ui_rect_edsl_sync.rs` (every generated span IS the printer's
output, inside the right function; the struct mirror is pinned to the live host struct, not to a
copied literal) + `boyko_render/tests/ui_rect_spv_sync.rs` (each committed `.spv` is the re-DXC of
its own source; SKIPs with an `eprintln` when no dxc resolves — **a skipped run is not a pass**,
the PARTICLES-PLAN F15 rule) + `boyko_shaderdsl/tests/ui_leaves.rs` (the `EvalCf` oracle tables
and the literal span pins, host-side, dxc-independent). The S1 landing itself moved **neither**
binary: both re-DXC'd byte-identical from the generator's re-spliced sources.

*Landing history of the two binaries* (each `.spv` byte length is pinned by a `SpirvBlob<N>` in
`boyko_render/src/ui/mod.rs`, so every move below is a deliberate, recorded re-bless):

| Rung | `ui_rect.vs.spv` | `ui_rect.fs.spv` | Why |
|---|---|---|---|
| S1 (eDSL migration) | 2368 → **2368** | 7060 → **7060** | a refactor: identical bytes from the re-spliced sources |
| S2 (the 80 B record) | 2368 → **2408** | 7060 → **7136** | the shared mirror gained `uv`; the FS text branch reads it instead of the retired `corner_radius` alias |
| S3 (the sprite lane) | 2408 → **2408**, byte-identical | 7136 → **8760** | the VS's only S3 edit is a COMMENT inside the shared mirror, and DXC's output is measurably indifferent to it; the FS gained the set-1 `g_sprites[]` array, the set-0 binding-3 sampler, the `FLAG_TEXTURED`/`UI_SLOT_*` constants and the `NonUniformResourceIndex` sprite branch |

## `sdf_forward_march.comp.hlsl` — the Forward/VB fused SDF march+shade (compute)

One source `shaders/sdf_forward_march.comp.hlsl`; the `{HAS_MESH} x {VIEWT}` matrix, all four built
against ONE shared 14-binding Set-0 layout (+ Set 1 = the Forward shadow set verbatim): `t12`
(`gForwardDepth`) is HAS_MESH-referenced only and `u13` (`gViewT`) VIEWT-referenced only — each
bound-but-unread by the other variants (the R2 contract). Recorder selection: `mesh_leg` x
`GBufferScene::path_sdf_forward_writes_viewt()` (never a dynamic branch).

| Variant | `HAS_MESH` | `VIEWT` | `.spv` | dxc `-T` | Interface delta vs base |
|---|---|---|---|---|---|
| mesh-bounded | `1` | — | `sdf_forward_march.comp.spv` | `cs_6_0` | samples `gForwardDepth` @12 (march bounded at `t_mesh`; `sdf_owns = hit && t < t_mesh`). |
| mesh-less | — | — | `sdf_forward_march_sdfonly.comp.spv` | `cs_6_0` | no depth reference (`sdf_owns = hit`). |
| mesh-bounded + viewt | `1` | `1` | `sdf_forward_march_viewt.comp.spv` | `cs_6_0` | + `RWTexture2D<float>` `gViewT` @13 (r32f) — the TAA-under-VB gViewT producer: exactly-once per pixel (SDF `t` / mesh `t_mesh` / `1.0e30` background). |
| mesh-less + viewt | — | `1` | `sdf_forward_march_sdfonly_viewt.comp.spv` | `cs_6_0` | the depth-less sibling (SDF `t` / `1.0e30`). |

Reachability note: the VIEWT rows dispatch only under an SDF-carrying `VisibilityBuffer` leg set
with TAA armed (`path_sdf_forward_writes_viewt()`); the Forward family never arms them (no AA seam —
tripwired by a `debug_assert` in `declare_forward_graph`).

## `vb_resolve.comp.hlsl` / `vb_shade.comp.hlsl` — the VisibilityBuffer shading family (compute)

Two sources, each its own `{TEXTURED} x {FROXEL}` matrix against a shared VB-only Set-0 layout
(`vb_layout0` — 9 bindings `{0..7, 11}` — for the base/TEXTURED rows, `vb_layout0_froxel` — 11 bindings `{0..9, 11}`,
`vb_layout0`'s own 0..7 plus `ClusterGrid`@8/`LightIndexList`@9 — for the FROXEL rows; Set 1 = the
Forward-family shadow set verbatim, Set 2 = the Decision-0 geometry table). `vb_resolve.comp.hlsl`
is the FUSED resolve (unpacks `vb_id`, re-fetches geometry, shades, writes `lit`);
`vb_shade.comp.hlsl` is its material-classified sibling (VB-P2 classification plan) — the shading
tail is character-identical between the two sources by construction (plan D3), so the `FROXEL` seam
is spliced identically into both. `TEXTURED` selects the bindless-material Set 3 eval (`vb_shade.comp.hlsl`
only — `vb_resolve.comp.hlsl` has no TEXTURED variant); `FROXEL` (VB-P1a, "dark infra") selects the
froxel-culled point/spot walk (`ClusterGrid`/`LightIndexList`) over the flat `[l0a_count,
light_count)` scan, gated at RUNTIME by `use_clusters` so an unarmed frame on a FROXEL-compiled
`.spv` still falls back to the identical flat walk. Since VB-P1k that gate is THREE terms, uniform
across all four `ClusterGrid` readers: `clusters_enabled != 0 && cluster_count != 0 &&
cluster_count <= grid_capacity`, where the capacity comes from `ClusterGrid.GetDimensions(...)` —
the BOUND descriptor's own element count (SPIR-V `OpArrayLength`), not a host-side mirror of it.
The two terms past the enabled bit are an out-of-bounds guard, not a style choice:
`robustBufferAccess` is OFF in this engine and no GPU-assisted validation runs, so an out-of-range
`ClusterGrid` read is real UB that no layer would report. `cluster_grid_read_bound.rs` pins the
bound's presence per artifact (`OpArrayLength` census) and closes the consumer set over the
committed HLSL roots. The base
(non-FROXEL) compile's tokens are byte-for-byte unperturbed by the `#else` arm — verified by re-DXC
(`vb_froxel_spv_sync.rs`).

⚠️ **A dark feature is not a free feature — MEASURED.** The VB-SV0 stage compiled an SDF
soft-shadow + contact-AO march into all ten rows of this family and the split family below, behind
a runtime light-header gate that shipped writing `0`. Every image golden held, and the cost was
real anyway: with **both** A/B phases dark, the fused lit-producer dispatch went 24 576 / 23 552 ns
before to 41 984 / 41 984 ns after — **+17.4..18.4 µs, ≈ +75 %**, on every VB frame of every VB
scene, purely for CARRYING the term. It was reverted for that (the ten rows are back at their
pre-SV0 bytes, `vb_resolve.comp.spv` 47 824 B). The lesson belongs in this manifest rather than
only in the plan: neither gate that stage ran could see it — image byte-identity is blind to cost
by construction, and the paired A/B cancelled it because both arms carried the same module. A rung
that widens a shipped `.spv` owes a dark-path measurement, not only a byte-identity pin.

| Source | Variant | `TEXTURED` | `FROXEL` | `.spv` | dxc `-T` | Interface delta vs base |
|---|---|---|---|---|---|---|
| `vb_resolve.comp.hlsl` | base | — | — | `vb_resolve.comp.spv` | `cs_6_0` | none (the flat all-lights scan). |
| `vb_resolve.comp.hlsl` | froxel | — | `1` | `vb_resolve_froxel.comp.spv` | `cs_6_0` | `+ StructuredBuffer<uint2> ClusterGrid` @8, `+ StructuredBuffer<uint> LightIndexList` @9 (Set 0 widens to `vb_layout0_froxel`); point/spot loop walks the froxel slice when `use_clusters`, else the base flat scan. |
| `vb_shade.comp.hlsl` | base | — | — | `vb_shade.comp.spv` | `cs_6_0` | none — the classify-table pixel-selection prologue only (plan D3), otherwise character-identical to `vb_resolve.comp.hlsl`'s own base row. |
| `vb_shade.comp.hlsl` | textured | `1` | — | `vb_shade_tex.comp.spv` | `cs_6_0` | + `PerInstanceMaterialTex` ring @1 (48 B, replaces the base 32 B `PerInstanceMaterial`) + Set 3 bindless texture-array table (`gTextures[]`@0, `gTexSampler`@1) — the TV0 material-eval splice. |
| `vb_shade.comp.hlsl` | froxel | — | `1` | `vb_shade_froxel.comp.spv` | `cs_6_0` | same delta as `vb_resolve.comp.hlsl`'s own froxel row. |
| `vb_shade.comp.hlsl` | textured + froxel | `1` | `1` | `vb_shade_tex_froxel.comp.spv` | `cs_6_0` | both deltas above, combined — the two `#ifdef`s are independent, non-overlapping spans (TEXTURED touches binding 1 + Set 3; FROXEL touches bindings 8/9). |

Reachability note (**re-derived 2026-07-27 — supersedes BOTH the VB-P1a text and its first
"correction", which mislocated the hardcode and the date**). The timeline, from the commits:

* **At VB-P1a (`78d0534`, 2026-07-24) the original note was ACCURATE as written.** The hardcode sat
  exactly where it said — `boyko_app`'s boot call site: `crates/boyko_app/src/runner.rs:454` in that
  commit is the literal `clusters_wanted: false`, whose own comment named VB-P1b as the rung that
  would read the real toggle. The RESOLVER was never the hardcoding site: at VB-P1a
  `render_path_config.rs:914` already carried today's live expression, `consumers.clusters_wanted &&
  matches!(path, RenderPath::VisibilityBuffer)`. Refuting the note by pointing at `resolve_rules`
  refutes a claim it never made.
* **It went STALE at VB-P1b (`d60d95b`, the same day)**, which replaced that literal with the live
  probe `app.world().try_resource::<LightingConfig>().is_some_and(|c| c.clusters_enabled)`. From
  that commit onward "never runs in production" was false for any scene that opts in — the note was
  simply never revisited.

Today: `froxel_light_cull` is a per-boot resolved bit (`consumers.clusters_wanted && path ==
VisibilityBuffer`, unchanged since VB-P1a; its `froxel_light_cull_is_vb_only` test pins the VB-only
scoping, and its FIELD doc in `render_path_config.rs` WAS updated at VB-P1b and is current), the
runner threads `clusters_wanted` from the booted scene's `LightingConfig::clusters_enabled`, and
`GpuSceneBundles::build_froxel_light_cull` runs under `if resolved_render_path.froxel_light_cull`
there — creating all three FROXEL pipelines together. The DEFAULT is unarmed (`clusters_enabled`
defaults `false`), so a scene that never opts in builds, declares and records nothing here and every
pre-VB-P1b golden stays byte-identical. "Defaults off" is not "never runs".

**Image-pin coverage is TWO rows of three, not "every FROXEL row"** (checked against
`goldens/PINS.toml`, which contains exactly two FROXEL-bearing pins, both blessed to real hashes on
the software AND hwrt legs):

| FROXEL row | Blessed image pin |
|---|---|
| `vb_resolve_froxel.comp.spv` | `[vb_mesh_froxel]` — VB-P1b (`d60d95b`), `fb220ff3…`, the FUSED arm |
| `vb_shade_tex_froxel.comp.spv` | `[vb_mesh_tex_froxel]` — VB-P1c (`b2b1240`), `6d7ea00d…`, the CLASSIFIED+TEXTURED arm |
| `vb_shade_froxel.comp.spv` | **NONE** — no image golden executes it (below) |

Both are EQUALITY pins: the same scene re-rendered with `BOYKO_VB_FROXEL_FORCE_OFF`
(`clusters_enabled = false`) must hash identically. `vb_shade_froxel` is the classified-but-UNTEXTURED
cell of the `(textured, froxel)` match, and `vb_use_classified = vb_force_classified ||
vb_tex_active` (`gpu_scene`) — so reaching it needs clustering armed AND the
`BOYKO_VB_FORCE_CLASSIFIED` dev override, since a textured armed frame lands in `(true, true)`
instead. No `PINS.toml` row sets that variable. Its pipeline IS created on every armed boot and its
match arm IS live, but no blessed pin ever dispatches it; the only gate on those bytes is the re-DXC
test below.

The `textured + froxel` row's host-side descriptor SET (`vb_set0_tex_froxel`, the TEXTURED ring +
`ClusterGrid`/`LightIndexList` combined) WAS the VB-P1a scope cut, and **VB-P1c (`b2b1240`) closed
it**: the set is built in `present/targets.rs` (the `vb_set0_tex_froxel` builder — `Some` iff
`vb_layout0_froxel` + `cluster_grid` + `light_index` + `vb_tex_instance_material_ring` +
`vb_shade_tex_froxel_pipeline` are all `Some`), and `present/passes/vb.rs`'s `(textured, froxel)`
match arm `expect`s BOTH `scene.vb_shade_tex_froxel_pipeline` and `targets.vb_set0_tex_froxel` to
be `Some` — a mis-resolved boot panics loudly there instead of silently falling back to the
non-froxel pipeline.

⚠️ **This paragraph used to end "Trust the `expect`, the field doc and the pins, not the prose
beside them" — a workaround standing in for a repair, and it under-counted the damage by an order
of magnitude.** It said the stale prose was at "BOTH of those sites". Grepping the class found
**thirteen**, across `graph_bridge.rs`, `passes/vb.rs`, `scene_types.rs`, `targets.rs` and
`vb_froxel_spv_sync.rs`, and `graph_bridge.rs` carried "VB-P1b **ARMED** the cull" 535 lines below
its own "hardcoded OFF". Every one is now repaired to the true statement: the arm is **default-OFF,
an owner opt-in** via `LightingConfig::clusters_enabled`, so an unarmed boot builds and records
nothing — which is all the 0%-gate byte-identity argument ever needed — while `vb_mesh_froxel` and
`vb_mesh_tex_froxel` arm it, reach the froxel cells and are golden-pinned with screenshot dumps.
The count needed three widenings to settle: the first grep was capped by `head`, the second was
restricted to `*.rs`. **A count in a finding is a lower bound even when the finding is right.**

`.spv` byte gate: `crates/boyko_rhi_vulkan/tests/vb_froxel_spv_sync.rs` —
`vb_froxel_variant_spv_byte_identical` re-DXCs the three FROXEL rows (`vb_resolve_froxel`,
`vb_shade_froxel`, `vb_shade_tex_froxel`), and `vb_base_variant_spv_unperturbed_by_the_froxel_seam`
re-DXCs the three base rows to prove the `#else` arm is physically inert. ⚠️ Both tests **SKIP
rather than fail** when no `dxc` resolves on the host — and `vb_shade_froxel` has nothing else.

## `vb_shade_split.comp.hlsl` — the R9 geo/shade-split lit producer (compute)

One source, a `{TEXTURED} x {HWRT}` matrix — the third VB lit producer, paired 1:1 with
`vb_geo.comp.hlsl`'s thin-aux geometry pass and selected instead of the fused
`vb_resolve`/`vb_shade` pair whenever `path_vb_split()` resolves (a pre-light consumer — SSAO,
DDGI, shadow-temporal, or the HWRT carrier — arms `mesh_geo_shade_split`). Set 0 is `vb_layout0`
verbatim; Set 1 is a DISTINCT 11-binding `vb_split_layout1` (the Forward shadow table @0-3 plus
`gSsao`@4, the DDGI atlas @5-9 and `gShadowVis`@10).

*Provenance: these four rows had **no manifest entry at all** until 2026-07-26 — the same
standing-rule gap `deferred_pbr_wrap.comp.spv` carried until `a4824a8`. Found while enumerating the
VB-SV0 blast radius, which had to re-pin all four; the section outlived that stage's revert because
the rows themselves ship regardless of it.*

| Variant | `TEXTURED` | `HWRT` | `.spv` | dxc `-T` | Interface delta vs base |
|---|---|---|---|---|---|
| base | — | — | `vb_shade_split.comp.spv` | `cs_6_0` | none (the flat all-lights scan; SSAO/DDGI read at their runtime gates). |
| textured | `1` | — | `vb_shade_split_tex.comp.spv` | `cs_6_0` | + `PerInstanceMaterialTex` ring @1 (48 B) + Set 3 bindless texture-array table (`gTextures[]`@0, `gTexSampler`@1) — the same TV0 splice `vb_shade.comp.hlsl` carries. |
| hwrt | — | `1` | `vb_shade_split_hwrt.comp.spv` | `cs_6_0` | the denoised mesh-shadow visibility `gShadowVis` (Set 1 @10) REPLACES the CSM sample for the primary directional; no ray is traced here (the `shadow_vis` producer pass owns the TLAS), so `cs_6_0` still suffices. |
| textured + hwrt | `1` | `1` | `vb_shade_split_tex_hwrt.comp.spv` | `cs_6_0` | both deltas above — the two `#ifdef`s are independent, non-overlapping spans. |

Reachability note: the two `HWRT` rows require `hwrt_denoise_or_vis_on`, which is exactly the
condition `ShadowSources::SDF_SOFT_MARCH` requires to be FALSE — so no SDF-march-sourced shadow
term can ever be armed while they are bound. What that buys is variant-matrix COMPLETENESS: the
`HWRT` rows are selected on `hwrt_denoise_or_vis_on`, so the exclusion is what keeps the four rows
above able to express every combination the resolver can arm. Break it and the resolver records a
shadow source no shipped row implements — an arm that binds cleanly, raises no validation message,
and is silently ignored.

⚠️ Do NOT read this as "the `HWRT` rows are missing an SDF-march arm the base rows have".
`vb_shade_split.comp.hlsl` has no `sdf_soft_shadow` arm in ANY of its four variants — `vis` starts
at `1.0` and min-combines CSM (`#else`) or `gShadowVis` (`#if HWRT`). Mesh pixels therefore never
receive an SDF-cast shadow under VB; that is a v1 scope cut, deliberate, and untouched by this
exclusion.

⚠️ An earlier revision of this block added *"and `deferred_pbr.hlsl` is the only shader in the tree
that implements that march at all"* — **false**, and fortified with a ⚠️ telling the next reader not
to re-derive it. Four shaders define such a march: `sdf_forward_march.comp.hlsl:297`,
`sdf_gbuffer_composite.hlsl:498` (plus `sdf_soft_shadow_mesh` at `:591`),
`sdf_probe_update.comp.hlsl:160`, and `deferred_pbr.hlsl:515`. The SDF leg's own pixels DO get an
SDF-cast shadow under VB, from `sdf_forward_march`; only mesh pixels do not. A ⚠️ is a request not
to re-check, so putting one around an unverified claim is worse than leaving the claim bare.

The exclusion is now checked in three places (it previously rested on the boot resolver's
predicate alone, with no check at the selection site):

* `ShadowSources::hwrt_vis_excludes_sdf_soft_march()` names the property, and `resolve_rules`
  `debug_assert!`s it at the one site that can violate it — the two `if`s that read the same
  `hwrt_denoise_or_vis_on` with opposite polarity.
* `sdf_soft_march_and_hwrt_vis_stay_exclusive_over_the_whole_input_space` sweeps 4 paths × 3 leg
  sets × 2 cap sets × 2^11 consumer masks through BOTH `resolve_rules` and `resolve_render_path`,
  asserting the property directly (so a release-mode run, where the `debug_assert!` is compiled
  out, is not blind to it). Its companion
  `the_exclusion_is_a_split_not_a_mutual_suppression` pins that the exclusion is a SPLIT — one of
  the two bits is always armed — so the sweep cannot pass by arming neither.
* `record_vb`'s `path_vb_hwrt_shadow()` arm `debug_assert!`s that the carrier reaching the
  recorder has no `SDF_SOFT_MARCH` bit, reading `ResolvedRenderPathGpu::shadow` through
  `GBufferScene::shadow_has_sdf_soft_march()`. This is the check AT the selection site.

Two limits, recorded so the next audit does not overstate the coverage. The selection-site check
is `debug_assert!` (the project's hot-path convention) and is `#[cfg(feature = "hwrt")]`, so it is
compiled only in a debug `--features hwrt` build; and no headless test reaches it, since recording
needs a device. The resolver sweep is the part that runs on every `cargo test`. The
`SHADOW_SOURCE_SDF_SOFT_MARCH` const the selection site reads is a restatement
(`boyko_rhi_vulkan` cannot depend on `boyko_render`), pinned against the owning definition by
`boyko_app`'s `shadow_source_sdf_soft_march_bit_matches_boyko_render`.

## `vb_geo.comp.hlsl` — the R9 thin-aux geometry pre-pass (compute)

One source, TWO independent axes (`MOTION`, `VB_SV0_TERM`). Paired 1:1 with `vb_shade_split.comp.hlsl`
above: the split producer's geometry half, dispatched when `path_vb_split()` resolves. It re-fetches
the covered triangle through `vb_geom_fetch.hlsli` and writes the thin aux targets the pre-light
consumers read.

| Variant | `MOTION` | `VB_SV0_TERM` | `.spv` | dxc `-T` | Delta |
|---|---|---|---|---|---|
| base | — | — | `vb_geo.comp.spv` | `cs_6_0` | the thin-aux write (`gThinNormal` + depth-derived view-space data) that SSAO / DDGI / the shadow-temporal reproject consume. |
| motion (R9d) | `1` | — | `vb_geo_mv.comp.spv` | `cs_6_0` | **+ the per-pixel camera-reprojected motion vector** for static geometry. No `rayQuery`, so the SAME `cs_6_0` target as base suffices — unlike the `deferred_pbr` HWRT rows, this axis does not move the profile. |
| sv0 term (VB-SV0 DP6b) | — | `1` | `vb_geo_sv0.comp.spv` | `cs_6_0` | **+ Set-1 `gSdfTerm` @3** (`rg8` UAV, **write** `RG(vis, ao)` under a wave-uniform `sv0_mode != 0` gate) **+ Set-1 `Buf` @4** (`register(t0)`, the SDF edit list, read). **Also a Set-0 delta, in the reflection though not in the layout: `vb_layout0`'s @3 `LightBuf` is declared by all three variants but READ only by this one, and DXC strips a declared-but-unread `StructuredBuffer` — `OpDecorate %LightBuf` count is 0 / 0 / 2 across base / motion / sv0.** The descriptor-set-layout object is shared and unchanged (a module that does not statically use a descriptor imposes no requirement on it — the R2 bound-but-unread contract); what differs is what each `.spv` reflects. The source-level `#define VB_SV0` is DERIVED from this flag, unlocking `vb_geom_fetch.hlsli`'s `tri_p0/1/2` + `vb_sv0_face_normal`; `light_table.hlsli` (ordered FIRST) + `sdf_field.hlsli` + `sdf_shadow_leaves.hlsli` are included, giving one `sdf_soft_shadow_ranged` march for the primary directional from the geometric face normal's lifted origin plus the 5-tap `sdf_ao` on the shading normal. Still `cs_6_0` — a software march, no `rayQuery`. |

**The `MOTION × VB_SV0_TERM` cross is provably empty and is deliberately NOT built** (DP6 design
Decision 2): `vb_geo_mv_active()` is `feature = "hwrt"` + `ray_query_enabled()` + the hwrt temporal
denoise arm, and `vb_sv0_host ⇒ !vb_geo_mv_active()` is a shipped property. Two axes, three modules —
the fourth cell has no boot that could select it.

**Why `-D` and not a runtime branch** (Decision 1): carrying the march compiled-in DARK measured
`+10 128 B` on this `15 888 B` kernel at `13f1c9a3` (and `+75 %` on `vb_resolve`). The variant as
shipped is `28 292 B` — **`+12 404 B`, `+78.1 %`** — so the dark tax the design refused to pay
unconditionally is *larger* on this host than the figure it refused it on.

**Byte gates:** `vb_raster_geo_classify_spv_sync.rs`'s row table (all three rows re-DXC'd under the
frozen recipe) **plus** `vb_geo_preprocess_sync.rs`, the two-sided `dxc -P` gate that proves the SV0
span is ADDITIVE — with the flag undefined the source preprocesses to its pre-DP6b program, which is
what keeps the base and motion `.spv` byte-frozen and every VB golden unmoved. At DP6b
`vb_geo_sv0.comp.spv` is selected by NOTHING, so those two gates are its entire coverage; the pick
arrives at DP6c.

*Provenance: this section was **missing** until 2026-07-26. `vb_geo` has shipped since rung R9 with a
real `-D` axis and no row — the same standing-rule violation `deferred_pbr_wrap` carried until
`a4824a8`. Found while enumerating the VB-SV0 blast radius, which had to prove both rows
**unperturbed**; the section outlived that stage's revert because the rows themselves ship
regardless of it. The `VB_SV0_TERM` row is VB-SV0 DP6b's, added with the axis rather than after it.*

## Deliberately ABSENT from this manifest: `vb_raster.{vs,fs}.hlsl`

Recorded so the next audit does not re-derive it. `vb_raster.vs.hlsl` → `vb_raster.vs.spv` (`vs_6_0`)
and `vb_raster.fs.hlsl` → `vb_raster.fs.spv` (`ps_6_0`) each compile to exactly ONE artifact and
contain **zero** preprocessor conditionals — `grep -c '#ifdef\|#if '` returns 0 on both. This file is
the registry of sources that compile to **N `.spv` via `-D`**; a single-artifact source has no
variant axis to document, so listing it would be padding rather than coverage.

That is worth stating rather than leaving implicit, because the id-encoding seam lives in exactly
these two files (`vb_raster.vs.hlsl` exports the flat instance id, `vb_raster.fs.hlsl` writes
`uint2(instance, SV_PrimitiveID)`). A future virtual-geometry rung re-encoding that pair to
`(instance, meshlet, local_tri)` would give them a real axis for the first time — at which point they
earn a section here, and every VB golden re-blesses with them.

## Shadow-denoise compute (separate shaders, not `-D` variants of the resolve)

These are distinct `.hlsl`, listed here for the temporal/spatial pipeline picture, not because they are
`-D` variants: `shadow_atrous.comp` (spatial edge-stopping à-trous over `gShadowVis`) and
`shadow_temporal.comp` (reproject + variance-clamp + accumulate against the cross-frame history pool).
The mode matrix (None / Spatial / Temporal / Both) is a **host** selection of which of these passes run
between the VIS producer and the DENOISED consumer — see `BOYKO_SHADOW_DENOISE`.

## `sdf_mesh_shadow.comp.hlsl` — the VB SDF-on-mesh shadow + contact-AO prepass (compute)

The VB-SV0 dedicated pass (plan Rev 10, `RENDER-PARITY-PLAN.md` §3.2 Option B): per covered pixel
(`vb_id != SENTINEL`) it re-fetches the triangle (`vb_geom_fetch`, under a SOURCE-level
`#define VB_SV0` that no lit-producer tail defines), marches `sdf_soft_shadow_ranged` from the
geometric face normal's lifted origin for the PRIMARY directional, runs the 5-tap `sdf_ao` along
the shading normal, and writes both into the R8G8 `gSdfTerm` it owns — the term the ten tails
`min`-combine (DP2). ONE variant, no `-D` axes:

| Variant | `.spv` | dxc `-T` | Interface |
|---|---|---|---|
| base | `sdf_mesh_shadow.comp.spv` | `cs_6_0` | Set 0: `gVbInstances`@0 · Camera@2 · `LightBuf`@3 · `gVbId`@5 · `gSdfTerm`@6 (rg8 UAV) · SDF `Buf`@10; Set 2: the geometry table. 64 B `view_proj` push. |

Byte gate: `sdf_mesh_shadow_spv_sync.rs` (re-DXC under the frozen recipe + a march-compiled-in
size floor). Why a separate pass and not `-D` variants of the tails: carrying the march compiled
into the tails cost ~+75% of the fused dispatch DARK (measured, reverted at `13f1c9a3`), and armed
it ran at 2.34× its dedicated-host cost — this file IS the dedicated host.

## `sdf_ssao.comp.hlsl` — the screen-space AO gather (compute)

One source `shaders/sdf_ssao.comp.hlsl`, ONE `-D` axis: `VB_THIN` (six `#if VB_THIN` spans). The
artifact count is 6, and it takes TWO independent multiplications to get there — only the second
one belongs to this file:

* The **quality** axis is NOT a `-D`. `boyko_shaderdsl::ssao::variant_hlsl` (driven by the
  `emit_ssao_variants` bin) re-emits the base text with ONLY the `static const SSAO_*` tuning header
  swapped, producing three committed sources `sdf_ssao_{low,medium,high}.comp.hlsl`. That is the
  "perf-justified, kept as 3 files" family the intro excludes — the `[unroll]` bounds must be
  compile-time literals.
* The **`VB_THIN`** axis IS a `-D`, and it belongs here: it DELETES a binding and renumbers the
  rest, which no specialization constant can do. Each of the three quality `.hlsl` is DXC'd twice —
  bare, and with `-D VB_THIN=1` — so 3 sources become 6 `.spv`.

The base `sdf_ssao.comp.spv` was RETIRED at Render P7-Q2: the base `.hlsl` is the single-source
glue template, not a shipped artifact (the engine loads the per-quality `.spv`), which is why the
`-Fo sdf_ssao.comp.spv` line still in that file's own header names no committed file.

| Variant | quality `.hlsl` | `VB_THIN` | `.spv` | dxc `-T` | Interface delta vs the base 5-binding table |
|---|---|---|---|---|---|
| base, Low | `sdf_ssao_low.comp.hlsl` | — | `sdf_ssao_low.comp.spv` | `cs_6_0` | none — `gNormal`@0 / `gMaterial`@1 / `gViewT`@2 / `ssao`@3 / Camera UBO@4 (`ssao_layout`). 2 slices × 3 steps. |
| base, Medium | `sdf_ssao_medium.comp.hlsl` | — | `sdf_ssao_medium.comp.spv` | `cs_6_0` | none. 2 × 4; bakes today's shipped module consts, so its `.hlsl` is byte-identical to the base template (the no-op proof). |
| base, High | `sdf_ssao_high.comp.hlsl` | — | `sdf_ssao_high.comp.spv` | `cs_6_0` | none. 8 × 6. |
| VB_THIN, Low | `sdf_ssao_low.comp.hlsl` | `1` | `sdf_ssao_vb_low.comp.spv` | `cs_6_0` | `gMaterial` **DROPPED ENTIRELY** (no slot reserved — unlike `sdf_forward_march`'s bound-but-unread precedent, because this variant gets its OWN layout, not a shared one); `gNormal` → `vb_geo`'s `gThinNormal` at the SAME slot 0; the mask test becomes `view_t < SSAO_VIEWT_BG` (`1e30`) off the already-bound `gViewT`. The survivors RENUMBER into a DENSE 4-binding table — `gThinNormal`@0 / `gViewT`@1 / `ssao`@2 / Camera@3 — a DIFFERENT host layout (`vb_ssao_layout`), not a hole in the 5-binding one. |
| VB_THIN, Medium | `sdf_ssao_medium.comp.hlsl` | `1` | `sdf_ssao_vb_medium.comp.spv` | `cs_6_0` | same delta. |
| VB_THIN, High | `sdf_ssao_high.comp.hlsl` | `1` | `sdf_ssao_vb_high.comp.spv` | `cs_6_0` | same delta. |

The march/dither/slice math, the tuning header, and the eDSL-GENERATED `ssao_horizon_step` span are
byte-identical TEXT on both sides of the axis — the span reads only function-local names
(`Pp`, `P`, `n`, `hc`), never a binding, so the renumbering cannot touch it. Recipe (cwd = the
shaders dir, so the relative `#include "ray_gen.hlsli"` resolves), frozen `dxc` 1.4.350.0:

```
dxc -spirv -T cs_6_0 -E main [-D VB_THIN=1] -fspv-target-env=vulkan1.3 \
    sdf_ssao_<quality>.comp.hlsl -Fo sdf_ssao_[vb_]<quality>.comp.spv
```

**Gating — every row is byte-gated, not merely described.** All in
`crates/boyko_rhi_vulkan/tests/ssao_edsl_sync.rs`:

* `ssao_spv_byte_identical` — the three **base** rows. Per quality it asserts three things: the
  committed variant `.hlsl` equals a fresh `variant_hlsl(base, preset)` re-emit, the committed
  `.spv` equals a fresh re-DXC of that `.hlsl`, and (Medium only) the variant `.hlsl` equals the
  base `.hlsl` byte-for-byte.
* `ssao_vb_thin_spv_byte_identical` — the three **`VB_THIN`** rows: a fresh `-D VB_THIN=1` re-DXC of
  the SAME committed non-VB quality `.hlsl` must equal the committed VB `.spv`. No separate VB
  `.hlsl` exists; the define is the entire difference.
* `ssao_horizon_step_matches_edsl_emit` — the eDSL `.contains` drift gate, iterated over the base
  template AND all three quality `.hlsl`.

⚠️ Both re-DXC tests **SKIP rather than fail** when `dxc` is absent from the host. A green run on a
machine without the Vulkan SDK proves nothing about these six blobs.

Reachability note: selection is by binding a different pipeline, never a dynamic branch. The base
rows go through `compute::sdf_ssao_spirv_variant(q)` against the 5-binding `ssao_layout`; the
`VB_THIN` rows through `compute::sdf_ssao_vb_spirv(q)` against `vb_ssao_layout`, gated by
`path_vb_ssao()` (`mesh_geo_shade_split && ssao.is_some()`) — so they are reachable only on the R9
VB geo/shade split, where there is no material G-buffer to read a mask from. All three `VB_THIN` pipelines are created at boot (a loop
over `SSAO_QUALITY_COUNT` in `boyko_app::gpu_scene`); `ResolvedSsao::variant` picks one per frame.

**Image-golden coverage is ONE row of six, and that is worth stating.** `[vb_mesh_ssao]` boots
`SsaoConfig { quality: High, atrous_levels: 3 }`, so it pins `sdf_ssao_vb_high.comp.spv` and only
that. `vb_mesh_ssao_screenshot_dump` is the ONLY SSAO-arming `test_name` in `PINS.toml`: no pin row
sets `BOYKO_SSAO`, so `grand_showcase_2mat` (the one pinned scene that reads it) resolves
`SsaoQuality::Off` under its blessed env (the 0%-gate), and `window_present_gbuffer`'s own six
`engine_ssao_*` dumps are unpinned. So **no blessed pin ever executes the base gather at all**. The other five rows rest entirely on the re-DXC byte gates
above plus the host-oracle mirrors (`ssao_horizon_step_host_matches_edsl_eval`,
`ssao_consts_host_match_edsl`, `ssao_params_table_host_match_edsl`).

## SSAO à-trous denoise compute — `ssao_atrous.comp.hlsl` (3 format-pin variants)

Render P7 POLISH follow-up: the SSAO denoise moved OUT of `deferred_pbr.hlsl`'s resolve (the former
inline 15x15 bilateral blur) into a dedicated multi-pass edge-avoiding à-trous compute chain, mirroring
`shadow_atrous.comp.hlsl` — Dammertz 5-tap B3-spline, `step = 1 << level` per dispatch — but
TRANSCENDENTAL-FREE (integer/mul/div/clamp/min/max only, no `exp`/`pow` edge-stops) so the bit-exact
host oracle (`golden_ssao_atrous`, `boyko_rhi_vulkan::goldens`) survives. Software (NOT `hwrt`-gated).

| Variant | `-D` flag | `.spv` | `gAoIn` pin | `gAoOut` pin | Used for |
|---|---|---|---|---|---|
| interior | (none) | `ssao_atrous.comp.spv` | `r16` | `r16` | every level except the first and last |
| read8 | `SSAO_ATROUS_READ_R8=1` | `ssao_atrous_read8.comp.spv` | `r8` | `r16` | level 0 (reads the raw `sdf_ssao` R8_UNORM gather) |
| write8 | `SSAO_ATROUS_WRITE_R8=1` | `ssao_atrous_write8.comp.spv` | `r16` | `r8` | the LAST level (writes back into the frozen `gSsao` R8_UNORM the resolve reads) |

The bind-group LAYOUT is IDENTICAL across all three (4 bindings: `gAoIn`/`gAoOut`/`gViewT`/Camera UBO
+ a 4-byte `{ uint step; }` push) — only the two `OpTypeImage` pins differ. Recipe: `cs_6_0`, e.g.
`dxc -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 -D SSAO_ATROUS_READ_R8=1 ssao_atrous.comp.hlsl
-Fo ssao_atrous_read8.comp.spv`.

## `cluster_cull.hlsl` — the Lighting-L1 clustered froxel light cull (compute)

One source `shaders/cluster_cull.hlsl`; VB-P1e rung H2 ("dark infra") grew an `#ifdef HIER`
seam: the hierarchical two-level cull (groupshared coarse AABB reduction + coarse-cull +
groupshared bitmask + fine walk over only the set bits — see
[docs/VB-P1E-HIERARCHICAL-CULL-PLAN.md](VB-P1E-HIERARCHICAL-CULL-PLAN.md) section 4),
compiled in only under `-D HIER=1`. The base (no `-D`) arm is the flat one-thread-per-froxel
scan, UNCHANGED token-for-token by construction — the seam is physically inert on that
compile (H2 gate (b), re-DXC-verified).

| Variant | `HIER` | `.spv` | numthreads | Interface delta vs base |
|---|---|---|---|---|
| base (flat) | — | `cluster_cull.comp.spv` | `[64,1,1]` | none. |
| hierarchical | `1` | `cluster_cull_hier.comp.spv` | `[256,1,1]` | + 6 `groupshared` arrays (coarse AABB reduction + bitmask, 6 276 B) + a 2-word push tail (`ClusterCullPush` widens 16 B → 24 B: `cluster_dims_packed`, `cluster_capacity` — boot-snapshot dims/capacity, D11). Same cull-set bindings (camera UBO @0, light table @1, ClusterGrid @2, LightIndexList @3, LightIndexAlloc @4) — no binding added or removed. |

Reachability note: the HIER pipeline is **built but never selected** this rung — no pipeline
object is created, and no host record site's dispatch is armed to choose it (that is H3/H4).
Every golden pin stays byte-identical for the trivial reason that the module is never loaded.
The output-set equality between the two arms is a `[P1]`-class **theorem** (plan section 5),
not a spec-constant collapse — a `-D` variant here changes the entry point's `numthreads` and
groupshared declarations, which a specialization constant cannot do.

## `vb_batch_cull.comp.hlsl` — the VisibilityBuffer batch/instance cull (compute)

One source `shaders/vb_batch_cull.comp.hlsl`; VG R3 piece 3 step P3-4 grew an
`#ifdef VB_CULL_DEBUG_PROBE` seam. The variant exists for ONE reason: the occlusion leaf's
differential (`crates/boyko_app/tests/hzb_verdict_oracle_gate.rs`) can observe only the PARTITION,
so a `depth_near` disagreeing with `boyko_render::hzb`'s by one ULP surfaces as a failure on the
exactly-equal boundary arm and as nothing else. That failure has been MEASURED. The variant adds a
sink the gate reads the leaf's own intermediates out of.

| Variant | `VB_CULL_DEBUG_PROBE` | `.spv` | dxc `-T` | Interface delta vs base (@0..@11) |
|---|---|---|---|---|
| base (shipping) | — | `vb_batch_cull.comp.spv` | `cs_6_0` | none. The ONLY module the engine ever loads. |
| diagnostic probe | `1` | `vb_batch_cull_debug.comp.spv` | `cs_6_0` | **+ `RWStructuredBuffer<uint> VbCullDebug` @12** — 8 `u32` per INSTANCE slot, written at every one of `occlusion_reject`'s seven exits: `{ stage, asuint(depth_near), asuint(occ), level, tap_x0, tap_x1, tap_y0, tap_y1 }`. No arithmetic differs; the projection fold's text, its `precise` qualifiers and its operation order are the same tokens in both. |

**Frozen-base discipline — a claim about THE SEAM, not a standing promise about the base.** Every
addition is inside `#ifdef VB_CULL_DEBUG_PROBE`, so with the macro undefined the file preprocesses
*character-identically* to its pre-diagnostic self: the compiler is handed the same token stream, so
**adding the probe** cannot move `vb_batch_cull.comp.spv`. That was executed and held — the byte gate
stayed green across the diagnostic step.

⚠️ It does **not** mean the base artifact never moves. The very measurement this probe produced led to
a decision change (the division-free verdict), which moved BOTH modules and re-pinned the census in
`crates/boyko_rhi_vulkan/tests/vb_batch_cull_spv_sync.rs` — including a field that had to be ADDED,
because `op_ford_less_than` went *down* when the verdict changed opcode and would have absorbed a
deleted decision silently. `vb_batch_cull_debug_spv_byte_identical` gates the new row the same way.

Word 1 of the record (`asuint(depth_near)`) is **diagnostic-only**: since the verdict became
division-free, the shipping module does not compute `depth_near` at all.

**Why a `-D` variant and not a runtime `occ_flags` bit.** A runtime-gated sink would have to DECLARE
@12 in the shipping module, and the engine's set layout has twelve bindings
(`VB_CULL_LAYOUT_BINDINGS`) — a module declaring a binding its bound layout does not provide is
invalid usage on EVERY engine frame. It would also add stores to PERFORMED that the framegraph never
DECLARED.

**Reachability note:** nothing in the engine binds, mints or dispatches this pipeline. Its only
consumer is the gate, which builds its own thirteen-binding layout
(`VB_CULL_DEBUG_LAYOUT_BINDINGS`) and dispatches BOTH modules over every boundary probe, asserting
their partitions agree — so the diagnostic artifact is a *measured* proxy for the shipping one, not
an assumed one.

## Cluster-buffer capacity bounds — census, and one TRACKED OPEN GAP

Two device buffers carry the L1 froxel cull's output: `ClusterGrid` (`uint2 {offset, count}` per
froxel) and `LightIndexList` (the flat surviving-index slices). Both are sized ONCE at scene boot
(`boyko_app/src/gpu_scene/mod.rs`, `build_froxel_light_cull`) and never re-allocated, while the
light-table header they are indexed through is republished from the LIVE `ClusterConfig` every
frame — so both need a bound that cannot drift from the allocation. `robustBufferAccess` is OFF
and no GPU-assisted validation runs, which is why a miss here is silent.

`ClusterGrid` has that bound on both sides. **`LightIndexList` does not, and that gap is open.**

| Buffer | Side | Bound | Anchored on |
|---|---|---|---|
| `ClusterGrid` | write (`cluster_cull.hlsl` base arm) | `min(dim_x*dim_y*dim_z, GetDimensions())` | **the allocation** (`OpArrayLength`) — VB-P1j |
| `ClusterGrid` | write (`cluster_cull.hlsl` HIER arm) | `fi < pc.cluster_capacity` | a pushed BOOT snapshot minted from the same `ClusterConfig` binding the buffer was allocated from (D11) |
| `ClusterGrid` | read (4 consumers) | `use_clusters &&= cluster_count <= GetDimensions()` | **the allocation** (`OpArrayLength`) — VB-P1k |
| `LightIndexList` | write (both cull arms) | `offset + write_count <= pc.index_list_cap` | a pushed BOOT snapshot (same construction as D11) |
| `LightIndexList` | read (4 consumers) | *(none local)* — `ps_offset`/`ps_count` are taken from `ClusterGrid[cluster]` verbatim | **nothing in the reading shader** — see below |

### Why the reads are in bounds today

A three-hop chain, no hop of which is local to the reading shader: the reader touches at most
`cell.x + cell.y - 1`; the cull publishes a cell only after clamping `offset + write_count <=
pc.index_list_cap`; and the host mints that push word and allocates the buffer from the SAME
`cluster_config.index_list_cap` in the same function, so the pushed cap equals the allocation's
element count and is never re-minted. **No reachable out-of-bounds read is claimed.** What is
claimed is that the reading shader bounds nothing itself: were a `ClusterGrid` cell ever read that
this cull did not produce, `cell.x`/`cell.y` are arbitrary and the read is bounded by nothing,
while the `ClusterGrid` read beside it would still be bounded by VB-P1k. The only guard against
that state today is the header's `cluster_count != 0` term, since `boyko_render::light` publishes
the dims lane only on a VB-froxel boot — an *enabled-bit* guard, not an allocation one.

### What closing it costs

MEASURED, not estimated. A probe compile of `vb_resolve.comp.hlsl` with the clamp added
(`LightIndexList.GetDimensions(ilc, ils); ps_count = (ps_offset >= ilc) ? 0u : min(ps_count, ilc -
ps_offset);`) compiles clean under the frozen recipe and emits the expected second
`OpArrayLength %uint %LightIndexList 0`, moving that one artifact **49 996 → 50 180 B (+184)**.

* **8 committed reader `.spv` move**: `deferred_pbr{,_wrap,_hwrt,_hwrt_denoised}.comp.spv`,
  `forward_opaque_froxel.fs.spv`, `vb_resolve_froxel.comp.spv`, `vb_shade{,_tex}_froxel.comp.spv`.
  (`deferred_pbr_hwrt_vis{,_mv}` do not — `SHADOW_STAGE=1` returns before lighting and DXC
  dead-strips the block. The cull's own two arms already clamp on the write side.)
* **All 8 are byte-gated** — five by `tests/cluster_grid_read_bound.rs`, three by
  `tests/vb_froxel_spv_sync.rs` — so the edit reds those re-DXC gates by construction and needs
  all eight re-blessed together.
* **Golden pins**: the clamp is inert on every consistent frame, so the pins are *expected* not to
  move — but expected is not measured, and confirming it needs fresh GPU runs of `forwardplus_mesh`,
  `vb_mesh_froxel` and `vb_mesh_tex_froxel`.
* **Not an eDSL edit**: all four read sites were VERIFIED outside every
  `// === GENERATED … BEGIN/END ===` region, so this is a legal hand-edit of the HLSL rather than a
  `boyko_shaderdsl` re-emit.

The gap is pinned executably by `the_light_index_list_capacity_bound_is_a_tracked_open_gap`
(`tests/cluster_grid_read_bound.rs`), which asserts 0 `OpArrayLength` on `%LightIndexList` across
all 10 artifacts that reference it. **A rise to 1 is the fix landing, not a regression** — re-bless
the moved `.spv`, re-run the froxel pins, and re-pin that test.

## Why these stay N `.spv` (do NOT try to spec-const-collapse)

A specialization constant is resolved at *pipeline-create* and can only change a **value** (a loop bound,
a count). It cannot: add or remove a descriptor-set binding (VIS's `gShadowVis`, HWRT's TLAS, MV's
`gMotionVec`), add a SPIR-V capability (`RayQueryKHR`), or delete half a shader (VIS strips lighting).
Every variant here does exactly one of those, so it is a genuine separate module. The good pattern is
already in place: **one `.hlsl`, N `.spv` via `-D`, ONE embed macro per `.spv`** — the source is
single, only the compiled artifact multiplies, and each artifact is a row above.

## Adding a new variant — checklist

1. **Shader**: guard the new code with `#if <FLAG>` in the existing `.hlsl` (never fork the file).
2. **Recipe**: compile offline with the frozen recipe + your `-D <FLAG>=<v>` (and `-T cs_6_5` if it needs
   `rayQuery`); commit the new `.spv` next to its siblings.
3. **Embed**: add ONE `embed_spirv! { /// doc … [#[cfg(feature = "hwrt")]] NAME_SPV, concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/<file>.spv") }` in `compute.rs` + a `pub fn <name>_spirv()` accessor. (No hand-counted size — the macro derives it.)
4. **Layout**: if the variant adds/removes a binding, add its pipeline-layout arm; keep the binding
   numbers consistent with the table above.
5. **Host**: select it by binding the right pipeline for the mode — never a runtime uniform branch.
6. **Row**: add it to this table. Gate byte-identity via the golden (`58f6c6c3`, GI-OFF) + the relevant
   `*_edsl_sync` re-emit test if the body is eDSL-generated.
