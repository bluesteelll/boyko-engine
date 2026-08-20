# Architecture: VB-SV0 DP6 — producer consolidation (design Rev 3)

> **Rev 3 delta** (closing the verify pass's N1..N6; everything else is Rev 2 verbatim):
> **N1 (P0), narrowed by the verify pass's N7** — `vb_sv0_host` gets its EXPRESSION:
> **`vb_sv0_host ≡ vb_sdf_mesh_armable() ∧ sdf_mesh_term_wanted`** — boot-frozen, carried on
> `ResolvedRenderPathGpu` as a second mirrored bool beside the existing `vb_sdf_mesh_armable`
> (`scene_types.rs:1514`, set at `gpu_scene/mod.rs:750` — the stated precedent; `sdf_mesh_term_wanted`
> is a `RenderPathConsumers` INPUT and needs this carrier to reach the declarator/recorder).
> Armable ALONE is too wide (N7): a VB×Both+SSAO boot with no SDF_MESH request resolves armable=true,
> and `host ≡ armable` would bind the +10 128 B sv0 variant AND declare a skipped `sdf_term` write on
> every such production frame — the Decision-1 dark tax paid unconditionally. With the conjunct:
> pure env host ⇒ term_wanted ⇒ vb_sv0_split ⇒ split ⇒ armable ⇒ host true, mode 0 (**arm B buildable**);
> production armed ⇒ term_wanted ∧ armable ⇒ host, mode ≠ 0 (**invariant 10 by construction, non-vacuous**);
> SSAO-only ⇒ term_wanted false ⇒ host false ⇒ `base` bound, no declared write (**invariant 6 restored**);
> `!rg8 ⇒ !armable ⇒ !host` (**Decision 5's degrade chain closes**). `armable`'s own definition is NOT
> narrowed (the conjunct lives in `host`, not in `mesh_geo_shade_split`), so `sv0_arm_matrix.rs BOOTS[1]`
> keeps `armable: true` — the P1-1 fixture table stands as written. The declared-write-but-skipped case is
> once again EXACTLY the measurement arm. One consequence folded in: `scene_types.rs:1512`'s claim that the
> armable mirror "emits no Vulkan command and cannot move a rendered byte" becomes false at DP6c (it is now
> a dispatch input) — corrected in the same commit.
> **N2 (P1)** — the `[vb_both_ssao]` FIXTURE is created at **DP6-0** (unpinned), so all four baseline cells
> are measurable before the producer moves; DP6c byte-pins it as planned.
> **N3** — seam corrected: `RenderPathConsumers` is built at `runner.rs:467` and resolved at `:509`; the env
> `host` read is a NEW site there (not "beside an existing read" — the only in-tree BOYKO_SDF_MESH reads are
> the two test/example hosts, whose `matches!` arms each gain `"host"` to keep the REQUEST bits false).
> **N4** — red mutation (6) respecified: drop the `sdf_term` write access while **arm A** (mode ≠ 0) runs;
> the detector is `graph.rs:643-690`'s debug authoring guard on the tails' read of a non-seeded transient —
> NOT sync validation, which this box's record says cannot see it.
> **N5** — invariant 6 restated where it lives: structural absence holds on frames that are **disarmed and
> not host**; the host arm is the one deliberate exception (declared write, in-shader skip) and says so.
> **N6** — the DP7-alone estimate shown with its derivation: `D + (F+M_ded)/4 = 8 832 + 0.75·D ≈ 9.6 µs`
> (band 8.8–10.4), not "10–12"; and it OMITS `U` — a depth/normal-aware upsample in EVERY tail, plus the
> fact that a bilinear term read needs a sampler in `vb_layout0`, the layout shared by the whole VB family
> ⇒ a family-wide `.spv` re-bless. DP6d.5's deliverables now include pricing `U` and the sampler change.
> Omitting `U` made DP7 look cheaper, which STRENGTHENS the anti-reordering conclusion — error direction
> recorded. **Δ_AB gains a stated trigger**: if `Δ_AB > 2 × 6 144 ns` (the retired clause's own line), that
> is the numeric argument that "half-res is wanted" — DP6d.5's disposition is then owner-eval PLUS this
> number, not owner-eval alone.

> Supersedes Rev 2 in full. Companion research: [VB-SV0-DP6-RESEARCH.md](VB-SV0-DP6-RESEARCH.md).

> Supersedes Rev 1 in full. Companion research: [VB-SV0-DP6-RESEARCH.md](VB-SV0-DP6-RESEARCH.md).

# Answer to the critic's question 1, first, because it decides the rung

**P0-1 is correct, and my Rev 1 contradicted itself.** Decision 3's own table said "≈0 to −8 µs" on the measured boot; the Goal and the metrics table headlined "35 328 → ≤ 12 288 ns". Both cannot be true. The table was right.

The fixture DP4 measured (`vb_both_sdf`, verified: `insert_resource` at `:95` and `:102` only — no `SsaoConfig`, no `DdgiConfig`) arms no pre-light consumer, so `mesh_geo_shade_split == false` and DP4's 35 328 ns is a **fused-boot** number. Counting full-screen `vb_geom_fetch` walks and dispatches per armed frame:

| boot class | today | after DP6 | Δ fetches | Δ dispatches |
|---|---|---|---|---|
| **fused** (`vb_both_sdf` — the measured one) | `vb_resolve` + `sdf_mesh_shadow` = **2 / 2** | `vb_geo` + `vb_shade_split` = **2 / 2** | **0** | **0** |
| **already-split** (VB×Both + SSAO/DDGI/temporal) | `vb_geo` + `sdf_mesh_shadow` + `vb_shade_split` = **3 / 3** | `vb_geo` + `vb_shade_split` = **2 / 2** | **−1** | **−1** |

On the measured boot DP6 **relocates** the fetch into a dispatch that boot newly runs. The `D + F ∈ [20 939, 29 184] ns` deletion is real **only on already-split boots**. Rev 1's headline was DP4's category error re-run in the opposite direction, and it is withdrawn.

**Decision: option (a), reframed as PRODUCER CONSOLIDATION, plus the P0-4 remedy from option (c) as a gate rather than a reordering.**

Why not (b) — keep the amortization frame, restrict claim + DP6e to split boots: that leaves **two producers shipped forever** (dedicated pass for fused, geo half for split), which is the two-code-paths/two-zone-stories/two-cost-models outcome the rung exists to avoid, and it makes the feature's cost a function of an unrelated consumer's arming. Rejected.

Why not (c) — measure DP7 (half-res) first and let it decide: the numbers do not separate them. In the **dedicated pass** half-res quarters *both* F and M (one thread per term texel), so DP7-alone ≈ `D + (F + M_ded)/4` ≈ **10–12 µs**. DP6-alone leaves SV0's marginal cost at `M_geo ∈ [6.1, 14.4] µs`. **Same band.** A full reordering is not justified by a tie. Two things break it: DP6 additionally buys consolidation (−1 shader, −1 `.spv`, −1 pipeline, −1 layout, −1 descriptor ring, −1 zone id, −1 `spv_sync` test, −75 lines of `record_vb`) which DP7 buys none of; and DP7 carries a quality cost (half-res soft shadow + contact AO crawl on silhouettes without an edge-aware upsample) which DP6 does not.

But **P0-4 stands and is not answered by that tie.** DP6 and DP7 are *partially antagonistic*: `vb_geo` is one thread per full-res `vb_id` pixel (`vb_geo.comp.hlsl:214-236`), so a half-res term in the geo host needs a second dispatch — which reintroduces `D` and converges back on DP7-alone. Retiring the dedicated pass therefore removes the only host shape DP7 can use. **Remedy: a DP7 FEASIBILITY PROBE (rung DP6d.5) runs in the dedicated pass while it still exists, and DP6e's gate carries its explicit disposition.** Probe it; do not build it. That converts the one-way door into a door with a measurement in front of it, at a fraction of a reordering's cost.

**Consequence for the gates: the 2× cost clause is NOT this rung's justification and is dropped as such.** A consolidation is justified by maintenance surface plus the split-boot win. It is replaced by:
- **G-NEUTRAL (fused boots):** `T(DP6, armed) − T(today, armed) ≤ +` the null-certified resolution. A consolidation may not cost performance.
- **G-REDUCE (split boots):** `T(DP6, armed) < T(today, armed)` by more than the null-certified resolution.
- **G-MARGINAL (informational, not gating):** `Δ_AB` against the 6 144 ns reference, reported with its 2× ratio so the inherited threshold's fate is on the record — but **not adjudicated as a pass/fail**, because the rung no longer claims it.

---

# Changelog — Rev 1 → Rev 2

| Finding | Closure |
|---|---|
| **P0-1** baseline / relocation | Rung reframed to consolidation. Per-boot-class cost table added (above and §Metrics). Headline "35 328 → ≤ 12 288" **withdrawn**. 2× clause demoted from gate to informational. Gates replaced by G-NEUTRAL / G-REDUCE. |
| **P0-2** bracket spans nothing | **Verified: `ZONE_VB_RUN` ends `vb.rs:3016`; `vb_geo` records `:3752`; split shade `:4277`.** Comparator respecified: split pair = **`ZONE_VB_GEO + ZONE_VB_SHADE` (a SUM of two disjoint intervals, not one span)**; fused comparator = **`ZONE_VB_SHADE` alone** (it is *defined* to bracket whichever of the three producers runs — `vb.rs:3388-3398`), plus `ZONE_VB_SDF_MESH` on today's armed frames. New rung **DP6-0** mints the zone **before** the producer moves, so baselines are paired, not remembered. |
| **P0-3** arm B unbuildable | ONE predicate `GBufferScene::vb_sv0_host` drives **both** the pipeline pick and the `sdf_term` access declaration. The shader's store is **moved inside `if (sv0_mode != 0u)`** — wave-uniform, behaviourally identical on every mode≠0 frame. Arm B's write is declared (safe over-declaration) *and* skipped in-shader. Both halves stated in §Decision 6. |
| **P0-4** one-way door | New rung **DP6d.5 — DP7 feasibility probe** in the dedicated pass. **DP6e's gate requires an explicit recorded DP7 disposition.** Antagonism (`vb_geo` = one thread per full-res pixel) stated as a first-class constraint. |
| **P1-1** truth tables turned | Enumerated in §Integration: `render_path_config.rs::sv0_never_arms_under_hwrt` (:2755) and `sv0_armable_only_on_vb_with_both_legs`; `sv0_arm_matrix.rs` `BOOTS[0]` (`:96-105`, `armable:true` → `false`, `why` rewritten) and `BOOTS[1]`'s `why` (its stated reason "armable exactly like the fused one" becomes false); `BOOTS` gains a fused+term-wanted row. |
| **P1-2** Rev-5 erratum | §Decision 4 records it verbatim and argues the W4 class cannot re-open (SV0 is VB-only, consumes no thin-aux channel, and *produces* rather than consumes). |
| **P1-3 / Q3** boot-committed + config surface | **`LightingConfig::vb_sdf_mesh_host` DELETED from the design.** The host arm is env-only (`BOYKO_SDF_MESH=host`) at the boot seam, like every sibling knob. Boot-frozen contract + frame-100 behaviour + `runner.rs` named + `vb_sdf_mesh_armable`'s "SINGLE capability predicate" doc correction, all in §Decision 4. |
| **P1-4** `register(t0)` | **Verified: `BUF_T0_DECL = "StructuredBuffer<uint> Buf : register(t0)"` (`:642`) and `find_decl_line` PANICS (`:567-574`).** `register(t12)` withdrawn — the span uses `register(t0)`, matching `sdf_mesh_shadow.comp.hlsl:96`. Only the **vk::binding** assertion is re-pointed. |
| **P1-5** `-P` gate is new work | Owned: new file `crates/boyko_rhi_vulkan/tests/vb_geo_preprocess_sync.rs`, landing at DP6b. Hash is **recomputed via `git show`**, not committed — rationale in §Validation. |
| **P1-6** @3 storage degrade | **Partially refuted.** `sdf_mesh_shadow_set0` **already carries the cap gate** — `targets.rs:4271-4279` conjoins `ctx.device_caps().rg8_unorm_storage_ok` with the VUID named in a comment. **No shipped hole.** What *is* real is doc-rot at `:639-643` ("built on every VB boot" omits the conjunct) — flagged for repair, not a fix. My own @3 hole is real and closed: placeholder-bind to `thin_normal[i]` on `!rg8_ok`, the shipped R9d idiom. |
| **P2-1** missing include | `#include "light_table.hlsli"` added, ordered before `sdf_field.hlsli` as in the shipped consumer. |
| **P2-2** pick is cfg-split | **Verified `vb.rs:3761-3774`** — two `cfg`-split bindings. Snippet corrected to extend both arms. |
| **P2-3** wrong doc-rot target | Corrected: the array doc (`:698-709`) is **current**; the stale text is the test-fn doc `:523` and the assertion message `:545-548`. |
| **P2-4** leg table | New split+SV0 row **and** the "armed" ambiguity resolved (occlusion-split vs geo/shade split get distinct words) in the same edit. |
| **Q4** | Answered: the dedicated pass becomes **unreachable at DP6c**, not at DP6e. Revert story restated accordingly. |
| **Q5** | Answered constructively: no pin covers split+SV0 today (`[vb_mesh_ssao]` is VB×**Mesh** — no SDF leg — so it can never arm SV0). DP6c **adds** `[vb_both_ssao]` (split, SV0 disarmed) and seeds `[vb_both_ssao_sv0]` PENDING. |
| **Preserve list** | All ten items byte-stable. Fetch arithmetic, zone story (11→12, hole at 10, auto-TopOfPipe via the `:333` exclusion), `gSdfTerm` double-role non-hazard, 2+1 tail reads, Decision 2's emptiness proof, span fidelity, `sdf_field.hlsli`'s Buf-only need, and Decision 6's paired-delta instrument all carried unchanged except where a P0 forced a stated edit. |

---

# Architecture: VB-SV0 DP6 — producer consolidation into the split path's geometry half

**Status:** DESIGN Rev 3 (critic-converged: 'fix N7's one conjunct and this is APPROVED' — the conjunct is fixed above).
**Parent:** `docs/VB-SV0-SDF-SHADOW-PLAN.md` Rev 10, DP4 adjudication block.

## Goal

Collapse SV0's **two** possible producers into **one**, hosted in `vb_geo.comp.hlsl` — the split path's geometry half, which already performs the per-covered-pixel `vb_geom_fetch`.

- **Maintenance:** −1 shader source, −1 committed `.spv`, −1 pipeline, −1 Set-0 layout, −1 per-FIF descriptor ring, −1 zone id, −1 `spv_sync` test, −75 lines of `record_vb`, −1 declared pass. One producer, one code path, one zone story, one cost model.
- **Performance, stated per boot class** (never as one headline):
  - already-split boots: **−1 fetch, −1 dispatch** = `D + F ∈ [20 939, 29 184] ns` deleted.
  - fused boots: **cost-neutral by construction** (2/2 → 2/2), gated as such.
- **Not a goal:** reducing SV0's marginal armed cost below the inherited 2× threshold. That is DP7's job, and DP6 must not foreclose it.

## Context and constraints

Invariants 1–7 from Rev 1 are carried unchanged, plus:

8. **`vb_geo`'s thread↔pixel mapping stays one-per-full-res-`vb_id`-pixel.** DP7's antagonism follows from it; any rung that changes it re-opens DP6e's disposition.
9. **`vb_sv0_host ⇒` (pipeline pick == sv0) `∧` (`sdf_term` write declared).** One predicate, two consumers — the O1 discipline.
10. **`vb_sdf_mesh_mode != 0 ⇒ vb_sv0_host`.** Host is the weaker predicate.

## Key decisions

Decisions 1, 2, 5, 7, 8 are carried from Rev 1 with the edits noted below. Decisions 3, 4, 6 are rewritten.

### Decision 1 (carried) — `-D VB_SV0_TERM=1` variant, not an unconditional runtime-gated span
Unchanged and unchallenged. `+10 128 B` on a `15 888 B` kernel is **+64 %** instruction footprint on the smallest kernel in the family, measured on this exact march at `13f1c9a3` (+75 % on `vb_resolve`). **P2-1 edit:** the guarded span now includes `#include "light_table.hlsli"` before `sdf_field.hlsli`. **P1-4 edit:** `Buf` keeps `register(t0)`.

### Decision 2 (carried) — exactly ONE new variant; `MOTION × VB_SV0_TERM` is provably empty
Unchanged; on the critic's preserve list. **P2-2 edit:** the record-site `debug_assert!` lands in the `#[cfg(feature = "hwrt")]` arm only (the `not(hwrt)` arm has no `vb_geo_mv_active()` to assert against, and the cross is vacuous there).

### Decision 3 (REWRITTEN) — consolidation: SV0 arming requires the split, and the dedicated pass is retired **at DP6c**, deleted at DP6e

**What.** `vb_sdf_mesh_armable()` gains `&& self.mesh_geo_shade_split`. From DP6c the dedicated pass is **unreachable** (an SV0-armed boot *is* split, so `mesh_leg && mode != 0` can no longer coexist with `path_vb_fused()`); DP6e deletes the now-dead code.

**Why (answering Q4 and the revert story).** Rev 1 claimed the two producers "coexist for exactly one rung". They do not — they are mutually exclusive from the moment the conjunct lands. So the honest revert story is: **DP6c is the semantic point of no return; DP6e is bookkeeping.** The revert target for a DP6d failure is therefore `DP6c^`, not `DP6e^`. DP6e's separation from DP6c buys one thing only: the dead code remains *in the tree* while DP6d.5's probe runs, so the probe has a host. That is its whole justification and it is now stated as such.

**Why consolidation and not two producers.** The surviving second producer would be the one that **failed its inherited cost clause at 5.75× and does not claim it**. Shipping it as the fused-boot path means the feature's cost jumps 2–3× depending on whether an unrelated consumer (SSAO) happens to be armed — a cliff the owner cannot predict from the config. Principle 10.

**Trade-off, priced.** A VB×Both boot wanting SV0 and nothing else now allocates `thin_normal`, runs `vb_geo`, and shades through `vb_shade_split`. **Gated by G-NEUTRAL**, which is the only reason this trade is acceptable: if it costs, the rung reds.

### Decision 4 (REWRITTEN) — env-only host arm; boot-frozen contract stated; Rev-5 erratum recorded

**What.**
```rust
// RenderPathConsumers — the ONLY new field. DEFAULT false.
pub sdf_mesh_term_wanted: bool,

// resolve_rules — SDF_SOFT_MARCH hoisted so it has exactly ONE spelling
let sdf_soft_march = sdf_leg && consumers.sdf_shadows_wanted && !consumers.hwrt_denoise_or_vis_on;
let vb_sv0_split   = matches!(path, RenderPath::VisibilityBuffer)
                  && consumers.sdf_mesh_term_wanted && sdf_soft_march;
let mesh_geo_shade_split = matches!(path, RenderPath::VisibilityBuffer)
                        && mesh_leg && (pre_light || vb_sv0_split);
// NORMAL union gains `|| vb_sv0_split`  (preserves `split => NORMAL`, R9b §7)
// later, unchanged in effect: if sdf_soft_march { shadow.insert(SDF_SOFT_MARCH) }
```
`sdf_mesh_term_wanted` is set at the **`boyko_app::gpu_scene` boot seam** from `LightingConfig::vb_sdf_mesh_shadow || ::vb_sdf_mesh_ao`, OR'd with an **env-only** host flag (`BOYKO_SDF_MESH == "host"`) read at that same seam. **No new `LightingConfig` field.**

**Q3 answered.** Rev 1 put a measurement knob on a production `Resource`; every sibling knob (`BOYKO_VB_ZONE`, `BOYKO_SDF_MESH=on|shadow|ao`, `BOYKO_AA`, `BOYKO_SSAO`, `BOYKO_CSM_OFF`) is env-gated. Nothing kept a shipped title from setting the field. Now nothing *can*: there is no field. The env read lives beside the existing `BOYKO_SDF_MESH` read, so it adds one match arm, not a plumbing hop.

**P1-3 — the boot-frozen contract, stated.** `resolve_render_path` runs **once**, at boot (`render_path_config.rs:1374-1376`); `LightingConfig` requests are re-asserted **every frame** (`light.rs:2231-2233`). Therefore:
> **Contract.** `sdf_mesh_term_wanted` is a **boot snapshot of the request**. A world that arms `vb_sdf_mesh_shadow`/`_ao` at frame 100 gets `mesh_geo_shade_split == false`, `vb_sdf_mesh_armable() == false`, and `sync_sv0_light_gate` clamps the request to 0 **for the process lifetime**, reported once by the cold latch. To arm SV0 the request must be present **before the first `resolve_render_path` call**.

This is the **same** contract `ssao_on` already carries (`SsaoConfig::enabled` is a boot read; a late SSAO enable is a no-op under VB), so it introduces no new class — but it was previously true only of *capabilities*, and it is now true of a *request*. Two consequences, both owned:
- **`runner.rs` is an affected file** (it publishes the request into the boot seam) and gains the contract comment.
- **`vb_sdf_mesh_armable()`'s doc is factually wrong after this rung.** "The SINGLE capability predicate every SV0 consumer reads" must become: *"the single ARMABILITY predicate — a capability conjunction plus, through `mesh_geo_shade_split`, a boot-frozen snapshot of the owner's request. It is no longer purely a statement about the device and the path."* Load-bearing for the arm matrix; corrected in the same commit.

**P1-2 — the Rev-5 erratum, recorded.**
> **Erratum (DP6) to Rev 5's MANDATORY single-predicate rule** (`render_path_config.rs:74-79`, `:985-987`, `:1009-1010`). Rev 5 says one union `pre_light` is the **sole** trigger for three flags. After DP6 that holds for **two** of them — `needs_depth_prepass` (Forward) and `sdf_geo_shade_split` (the SDF leg) still read `pre_light` alone. The **VB** flag reads `pre_light ∨ vb_sv0_split`. The rule is restated as: *one predicate for the Forward and SDF flags; the VB flag is that predicate OR the VB-only SV0 term.*

**Why the W4 hole class cannot re-open.** W4 was: *a MOTION-only pre-light consumer under Forward reads frame-stale motion because the prepass was not armed* — a **consumer** left without its **producer**. `vb_sv0_split` is the opposite shape: it conjoins `path == VisibilityBuffer` (so it can never reach `needs_depth_prepass`), it consumes **no** thin-aux channel, and it exists precisely to arm a **producer** (`vb_geo_sv0`) for an image it writes itself. There is no consumer it can leave unfed. The `|| vb_sv0_split` on the NORMAL union is likewise producer-side: it keeps `split ⇒ NORMAL` true so `vb_geo`'s unconditional `thin_normal` write and the mask agree (R9b's own stated reason: "the mask must stay the single truth").

**`split ⇒ NORMAL` cost, unchanged from Rev 1:** one `oct_encode` (~10 ALU) + two already-warm loads + one RGBA8 store, against 128 march iterations × N-edit field walks plus 5 AO taps. **<1 %.** Buying that back would cost an invariant, a 4th variant, and a test.

### Decision 5 (carried, P1-6 edit) — bindings, with the storage degrade closed

`vb_geo_aux_layout` 3 → 5 bindings, declared unconditionally (one layout object for all three pipelines):

| slot | reg | resource | base | `sv0` |
|---|---|---|---|---|
| @0 | u8 | `gThinNormal` RGBA8 | WRITE | WRITE |
| @1 | u9 | `gMotion` RG16F | `#if MOTION` | unread |
| @2 | b10 | `MotionCam` UBO | `#if MOTION` | unread |
| **@3** | **u11** | **`gSdfTerm` RWTexture2D\<float2\> rg8** | unread | **WRITE** |
| **@4** | **t0** | **`Buf`** (edit list) — **`register(t0)`, P1-4** | unread | READ |

**P1-6 degrade, stated.** `vb_geo_aux_set` is built on **every split boot** (`targets.rs:347-351`), including SSAO boots on devices without `rg8_unorm_storage_ok` — where the `sdf_term` ring is created **SAMPLED-only** (`:1144-1155`). A `StorageImage` entry over it would violate `VUID-VkWriteDescriptorSet-descriptorType-00339` at update time.
> **Degrade: placeholder-bind @3 to `thin_normal[i]` when `!ctx.device_caps().rg8_unorm_storage_ok`.** Same descriptor type (`STORAGE_IMAGE`), `thin_normal` always carries STORAGE usage, and this is the shipped R9d idiom verbatim (`vb_geo_aux_set`'s @1 motion slot is already placeholder-bound to `thin_normal[i]`, `targets.rs:347-351`). Provably inert: `!rg8_ok ⇒ !vb_sdf_mesh_armable ⇒ mode 0 ⇒ !vb_sv0_host ⇒ `sv0` module never bound ⇒ @3 never referenced by any executing module`. @4 needs no degrade — `edit_list` is always a valid `StorageBuffer`.

**Refutation of the critic's parenthetical, with the citation.** The shipped `sdf_mesh_shadow_set0` does **not** carry this hole. `targets.rs:4271-4279` gates its construction on `ctx.device_caps().rg8_unorm_storage_ok`, with a comment naming the exact hazard:

> ```rust
> let sdf_mesh_shadow_set0: Option<[VulkanBindGroup; FRAMES_IN_FLIGHT]> = if let (true, true, Some(layout)) = (
>     scene.path_is_vb(),
>     // The set's @6 is a STORAGE_IMAGE descriptor over the `sdf_term` ring — on a device
>     // without RG8 storage the ring was created SAMPLED-only ... and a storage descriptor
>     // over it would violate the update-time VUID; SV0 is unarmable there ...
>     ctx.device_caps().rg8_unorm_storage_ok,
> ```
> What is real is **doc-rot**: the field doc at `:639-643` says "built on every VB boot", omitting the cap conjunct. Filed for repair at DP6e (where the field is deleted anyway, so the repair is the deletion) — **not widened, and not a defect.**

### Decision 6 (REWRITTEN) — ONE predicate, the store gated in-shader, and the measurement respecified

**P0-3, both halves.**
```rust
// The ONE carrier: a second boot-frozen bool on `ResolvedRenderPathGpu`, beside the existing
// `vb_sdf_mesh_armable` mirror (`scene_types.rs:1514`, set at `gpu_scene/mod.rs:750` — the precedent):
//   vb_sv0_host ≡ vb_sdf_mesh_armable() ∧ sdf_mesh_term_wanted        (N7's narrowing)
// NOT armable alone — that is satisfied by every VB×Both+SSAO boot with no SDF_MESH request and
// would bind the +64 %-footprint sv0 variant + declare a skipped `sdf_term` write on ordinary
// production frames (the Decision-1 dark tax). With the conjunct: env host ⇒ term_wanted ⇒ split ⇒
// armable ⇒ host, mode 0 (arm B buildable); production armed ⇒ host ∧ mode ≠ 0 (invariant 10 by
// construction); SSAO-only ⇒ host false ⇒ `base`, no declared write; !rg8 ⇒ !armable ⇒ !host.
// Read at BOTH `declare_vb_graph` (the conditional `sdf_term` write access) and `record_vb` (the
// pipeline pick) — the O1 discipline, one predicate, two consumers. `armable`'s own definition is
// untouched, so the P1-1 fixture table (incl. BOOTS[1] armable:true) stands as written.
```
- **pick** (both `cfg` arms, P2-2): `if mv_active { mv } else if scene.vb_sv0_host { sv0 } else { base }`.
- **declaration**: `if scene.vb_sv0_host { g.image_access(sdf_term, COMPUTE, WRITE, GENERAL, COLOR); ... }`.
- **shader**: the store moves **inside** the mode gate —
  ```hlsl
  if (sv0_mode != 0u) { gSdfTerm[uint2(px, py)] = float2(vis, ao); }
  ```

Three properties this buys, all needed:
1. **Arm B is buildable.** `vb_sv0_host && mode == 0`: the `sv0` module is bound, the write is *declared* (safe over-declaration — a barrier for a write that does not occur is correctness-neutral; the converse is UB), and the shader skips it. A and B then differ in **exactly one taken branch**, which is what a control arm must mean.
2. **Semantic fidelity preserved** (critic's preserve list). `sdf_mesh_shadow.comp.hlsl:183` stores unconditionally, but is only *recorded* when `mode != 0` — so its store is unconditional-given-mode-nonzero. Gating in-shader is behaviourally identical on every `mode != 0` frame. The branch is on a **wave-uniform** scalar header read: one scalar compare, zero divergence.
3. **Invariant 6 intact.** On a production disarmed boot `vb_sv0_host == false` ⇒ no access, no barrier, no command.

**P0-2 — the brackets, named against verified line numbers.**
`ZONE_VB_RUN` ends at `vb.rs:3016`; `vb_geo` records at `:3752`; `ZONE_VB_SHADE` brackets the split shade at `:4277`/`:4442`, the classified producer at `:3416`/`:3585`, the fused one at `:3591`/`:3700`. So:

- **New `ZONE_VB_GEO = ZONE_BASE_VB + 11`** brackets `vb_geo`'s barriers→bind→dispatch. It is the family's only unbracketed dispatch today.
- **Split-pair quantity = `ZONE_VB_GEO + ZONE_VB_SHADE`, a SUM of two DISJOINT intervals** — the `vb_viewt` pass, the SSAO gather and the à-trous chain sit between them. It is *"the split pair's own dispatches"*, **not** wall-clock from geo-start to shade-end. Stated because a reader will otherwise treat it as a span.
- **Fused-side comparator = `ZONE_VB_SHADE` alone.** It is *defined* to bracket whichever of the three lit producers a frame selects (`vb.rs:3388-3398`), which is precisely what makes it cross-arm comparable.

**The cost table's four cells, all measurable with one new id:**

| | today | after DP6 |
|---|---|---|
| fused, SV0 armed | `ZONE_VB_SHADE + ZONE_VB_SDF_MESH` | `ZONE_VB_GEO + ZONE_VB_SHADE` |
| split, SV0 armed | `ZONE_VB_GEO + ZONE_VB_SHADE + ZONE_VB_SDF_MESH` | `ZONE_VB_GEO + ZONE_VB_SHADE` |

The "today, split" row needs `ZONE_VB_GEO` to exist **before** the producer moves — hence **rung DP6-0**, which mints the zone alone. Baselines are then a *paired* before/after on one instrument, not a comparison against a remembered number.

**Arms** (on `vb_both_sdf` for fused, on the new `[vb_both_ssao]` boot for split):
| arm | `BOYKO_SDF_MESH` | split | variant | mode | `vb_sv0_host` |
|---|---|---|---|---|---|
| A | `on` | armed | `sv0` | 3 | true |
| B | `host` | armed | `sv0` | **0** | **true** |
| C | (SSAO boot, no SV0) | armed | `base` | 0 | false |

`Δ_AB` = the term's marginal cost inside an already-fetching host — **the same shape as the 6 144 ns reference**, reported with its ratio but **not gating** (the rung no longer claims it). `Δ_BC` = the compiled-in-but-closed variant tax, budgeted at the null-certified resolution; it is the number that would *refute* Decision 1. Arm C's SSAO-chain confound is second-order (`ZONE_VB_GEO` brackets only the geo dispatch) and named.

**Clause 5 gates every row.** An uncertified row is INCONCLUSIVE — never PASS, never FAIL.

### Decision 7 (carried, P2-4 edit) — `ZONE_VB_GEO` minted, `ZONE_VB_SDF_MESH` retired in place

`VB_ZONE_COUNT` 11 → 12; slot 10 a permanent hole (one unused pair, zero commands — `NotBracketed`); id 11 auto-`TopOfPipe` via the `matches!(zone, LATE_UPLOAD..=RUN)` exclusion at `:333`. **On the preserve list, carried verbatim.**

**P2-4:** the leg table (`gpu_zone.rs:227-230`) currently has two rows labelled by *occlusion*-split arming and no SV0 row, so "armed" already means two things. The same edit (a) renames the axis to `occlusion split armed` / `occlusion split off`, (b) adds a `geo/shade split` dimension, (c) adds the SV0 row that `ZONE_VB_SDF_MESH`'s own doc describes but the table never had. Resolving the pre-existing ambiguity is in scope because the new row inherits it.

### Decision 8 (carried, P1-4 + P2-3 edits) — the shadow-leaves pins

```rust
const SHADOW_LEAVES_MIN_CONSUMERS: [&str; 2] = ["deferred_pbr.hlsl", "vb_geo.comp.hlsl"];
/// Each derived consumer's own expected vk::binding spelling. A consumer ABSENT from this
/// table is a RED: a new marcher host must state where it put the edit list.
const BUF_BINDING_BY_CONSUMER: [(&str, &str); 2] = [
    ("deferred_pbr.hlsl", "[[vk::binding(10)]]"),
    ("vb_geo.comp.hlsl",  "[[vk::binding(4, 1)]]"),
];
```
**`BUF_T0_DECL` is untouched** (P1-4) — the span writes `[[vk::binding(4, 1)]] StructuredBuffer<uint> Buf : register(t0);`, matching `sdf_mesh_shadow.comp.hlsl:96`'s own `vk::binding(10,0)` + `register(t0)` pairing. Only the **slot** assertion moves; the register pin, the ordering assertions and the const-block assertions all pass unchanged.

**P2-3 — the right doc-rot targets.** The array's own doc (`:698-709`) is **current** and needs only the third-turn update. The stale text is the **test-fn doc `:523`** ("`SHADOW_LEAVES_MIN_CONSUMERS` only asserts that the four known ones did not vanish" — the array holds two) and the **assertion message `:545-548`** ("The three VB lit-producer tails plus the deferred resolve are the whole reason the shared header exists"). All three are edited together; per the doc-rot lesson the new text names commits, not adjectives.

## Data structures

```rust
// boyko_render::render_path_config — ONE new field, default false.
pub struct RenderPathConsumers {
    /// **VB-SV0 DP6.** BOOT SNAPSHOT of "the owner asked for the SDF-on-mesh term (either
    /// half), or the env-only measurement host arm". Set at the `gpu_scene` boot seam.
    ///
    /// DEFAULT `false` — the mechanism by which zero goldens turn: no existing boot's
    /// resolution moves by one field.
    ///
    /// Gates `mesh_geo_shade_split` (VB only) because the term's PRODUCER is the split's
    /// geometry half. BOOT-FROZEN: a request first raised at frame 100 is clamped forever
    /// (see `vb_sdf_mesh_armable`'s corrected doc).
    pub sdf_mesh_term_wanted: bool,
}
// LightingConfig: NO new field. bits 5..6, both `_armed` fields, `shadow_gate_word` UNCHANGED.

// GBufferScene — the ONE predicate (invariants 9, 10).
pub vb_sv0_host: bool,
```

```hlsl
// vb_geo.comp.hlsl — every addition inside the guard; APPEND-ONLY; no new local outside it
// (the R9b hoisted-load lesson). `n` and `geo` are reused, not re-derived.
#ifdef VB_SV0_TERM
#define VB_SV0
#endif
// ... existing includes/bindings/oct_encode/push constant UNTOUCHED ...
#ifdef VB_SV0_TERM
[[vk::binding(3, 1)]] [[vk::image_format("rg8")]] RWTexture2D<float2> gSdfTerm : register(u11);
#include "light_table.hlsli"                              // P2-1: mode bits + light loads, FIRST
[[vk::binding(4, 1)]] StructuredBuffer<uint> Buf : register(t0);   // P1-4: register(t0) KEPT
#include "sdf_field.hlsli"
static const float EPS = 0.001;  static const float T_MAX = 10.0;
static const uint  MAX_IT = 128u; static const float SHADOW_K = 8.0;
static const float SHADOW_MINT = 16.0 * GRAD_H;
static const float SHADOW_MINT_STEP = 16.0 * GRAD_H;
static const float SHADOW_HIT_EPS = 2.0 * EPS;
static const float SHADOW_NDOTL_EPS = 0.0;
static const float SHADOW_NORMAL_BIAS = 0.02;
#include "sdf_shadow_leaves.hlsli"
#endif

void main(...) {
    // ... sentinel early-out, vb_geom_fetch, n, #if MOTION, materials, gThinNormal — UNTOUCHED
#ifdef VB_SV0_TERM
    uint sv0_mode = load_vb_sdf_mesh_mode(LightBuf);   // wave-uniform, hoisted once
    float vis = 1.0;
    if ((sv0_mode & VB_SDF_MESH_SHADOW_BIT) != 0u) { /* primary directional; ranged march,
        origin lifted along vb_sv0_face_normal(geo) — §4.2 fidelity preserved */ }
    float ao = 1.0;
    if ((sv0_mode & VB_SDF_MESH_AO_BIT) != 0u) { ao = sdf_ao(geo.world_pos, n); }
    // P0-3: the store is GATED, so arm B (host, mode 0) writes nothing while binding this
    // module. Wave-uniform branch; identical behaviour on every mode != 0 frame.
    if (sv0_mode != 0u) { gSdfTerm[uint2(px, py)] = float2(vis, ao); }
#endif
}
```

## Integration

**`declare_vb_graph`:** delete the `sv0_pass` block (`:4607-4628`); inside the `if split` arm, after `thin_normal`, add under `if scene.vb_sv0_host` the `sdf_term` WRITE and the `light_table` READ (`vb_id`/`vb_instance_ring` already declared by `vb_geo`). The three tail-side term reads (`:4747`, `:4820`, `:5269` — 2 fused + 1 split, preserve list) become `if scene.vb_sdf_mesh_mode != 0`, plus `debug_assert!(!scene.path_vb_fused() || scene.vb_sdf_mesh_mode == 0)`. `VB_IMAGE_COUNT` and every ResId **unchanged** (`sdf_term` keeps index 15 / 21).

**`record_vb`:** delete `:3098-3174`; extend **both** `cfg` arms of the pick (`:3761-3774`); wrap `:3752`→dispatch in `ZONE_VB_GEO`.

**P1-1 — every truth-table fixture the conjunct turns:**

| fixture | today | after | edit |
|---|---|---|---|
| `render_path_config.rs::sv0_never_arms_under_hwrt` `:2755` | `assert!(armable.vb_sdf_mesh_armable())` on VB×Both, no consumers | **reds** | `sv0_consumers()` gains `sdf_mesh_term_wanted: true` |
| `render_path_config.rs::sv0_never_arms_under_hwrt` `:2775`, `:2796` | `!hwrt.vb_sdf_mesh_armable()` | still passes | + `assert!(hwrt.mesh_geo_shade_split)` — the split *is* armed, SV0 still is not (Decision 2's proof) |
| `render_path_config.rs::sv0_armable_only_on_vb_with_both_legs` | 4 negative rows | all still `false` | `sv0_consumers()` change propagates; add a positive-control row |
| `sv0_arm_matrix.rs` `BOOTS[0]` `:96-105` "VB x Both (fused)" | `armable: true`, *why*: "the SDF soft march is the shadow source and there are mesh pixels to shade" | **`armable: false`** | *why* rewritten: "no split ⇒ no `vb_geo` ⇒ no producer for the term" |
| `sv0_arm_matrix.rs` `BOOTS[1]` `:106-115` "+ SSAO (split tail)" | `armable: true`, *why*: "…armable exactly like the fused one" | still `true`, **reason now false** | *why* rewritten: SSAO arms the split, which IS the producer |
| `sv0_arm_matrix.rs` `BOOTS` | 9 rows | **10** | new row: VB×Both + term-wanted, no SSAO → `armable: true` (SV0 arms its own split) |
| `sv0_arm_matrix.rs` `:92-94` header doc | "the armable rows are the configuration the `vb_both_sdf`/`_tex` fixtures boot under" | false | rewritten with the split requirement |
| `sv0_mode_nonzero_implies_the_mesh_leg` `:397` | — | — | replaced by `..._implies_the_split` (strictly stronger) |

**Also affected:** `runner.rs` (boot-freeze contract comment + the `host` env arm), `gpu_scene/mod.rs` (the boot seam sets `sdf_mesh_term_wanted` and `vb_sv0_host`), `docs/RENDER-PARITY-PLAN.md` §3.2 (erratum: Option B superseded **under VB**; its overdraw-invariance rationale is *preserved* — `vb_geo` is also exactly one march per covered pixel), `docs/SHADER-VARIANT-MANIFEST.md`, `docs/OPEN-QUESTIONS.md`.

## Implementation plan

Revert-red at every rung. **Semantic point of no return is DP6c** (Q4).

- **DP6-0 — the instrument, alone.** `ZONE_VB_GEO`; `VB_ZONE_COUNT` 12; leg-table edit (P2-4). No producer change. *Gate:* all goldens byte-identical; `vb_bench_query_validation` still `measured > 0`; **the four baseline cells recorded** on the unmodified producer.
- **DP6a — resolver.** The consumer bit, the hoist, `mesh_geo_shade_split`, the NORMAL union, the `armable` conjunct, the env `host` arm, the doc corrections, the Rev-5 erratum. *Gate:* the eight fixtures above green after their stated edits; `sdf_mesh_term_wanted == false ⇒ every ResolvedRenderPath field bit-identical to pre-DP6` (tested, not argued); **all goldens byte-identical**.
- **DP6b — the dark variant.** Guarded span; `vb_geo_aux_layout` @3/@4 + the `!rg8_ok` placeholder; `vb_geo_sv0.comp.spv`; `embed_spirv!`; boot pipeline. Selected by nothing. *Gate:* `vb_geo.comp.spv`/`vb_geo_mv.comp.spv` byte-identical; new `spv_sync` row 7; **the new two-sided `-P` gate**; `spirv-val`; `sdf_field_edsl_sync` re-pointed; manifest row; all goldens.
- **DP6c — select, declare, record.** `vb_sv0_host`; the graph diff; the pick; `ZONE_VB_GEO` recording. Dedicated pass becomes **unreachable**. *Gate:* **new pin `[vb_both_ssao]`** (VB×Both + SSAO, SV0 disarmed — the boot class DP6 changes most, unpinned today) byte-identical across the rung; `[vb_mesh_ssao]` byte-identical; **live pixel-signature armed**: `SV0_MIN_SHADOWED_PIXELS`/`SV0_MIN_AO_PIXELS` with **(ii-a) shadow alone and (ii-b) AO alone each moving pixels on its own**; `sv0_arm_matrix.ps1` re-pointed; declare↔record parity asserts.
- **DP6d — measure.** Arms A/B/C on both boot classes. *Gates:* clause 5 first; then **G-NEUTRAL** (fused: Δ ≤ +resolution) and **G-REDUCE** (split: Δ < −resolution); `Δ_BC` ≤ resolution; `Δ_AB` reported with its 2× ratio, **informational**.
- **DP6d.5 — DP7 feasibility probe (P0-4).** In the dedicated pass, still present: half-res dispatch grid + half-extent term + a bilinear read at the tail. *Deliverables:* the quarter-cost number, and an owner-eval visual on silhouette crawl. **Not shipped, not gated on** — a probe. Reverted after measurement.
- **DP6e — retire.** Delete the shader, `.spv`, pipeline, layout, `sdf_mesh_shadow_set0` (and with it the `:639-643` doc-rot), `VbPlan::sv0_pass`, `ZONE_VB_SDF_MESH`'s live use, `sdf_mesh_shadow_spv_sync.rs`; `SHADOW_LEAVES_MIN_CONSUMERS` → 2 entries; the three P2-3 prose sites. *Gate:* `--workspace --no-fail-fast` green; all goldens; **the DP6c live-pixel proof re-run**; and — **blocking** — **an explicit recorded DP7 disposition** from DP6d.5: either *"half-res is refused on quality, the door may close"* or *"half-res is wanted; here is the host shape it will use after `vb_geo` retires the dedicated pass"*. **No disposition ⇒ DP6e does not land.**

## Metrics and validation

**Per-boot-class table** (§Decision 6) is the headline. No single-number claim.

**Byte-identity:** all goldens at every rung; `vb_geo.comp.spv` / `vb_geo_mv.comp.spv`; all ten lit-producer `.spv`; the two-sided `-P` gate.

**P1-5 — the `-P` gate is NEW work.** No `.rs`/`.ps1` in-tree invokes `dxc -P`; only plan prose cites it (the recurring dead-datum shape — five instances on record). **Owning file: `crates/boyko_rhi_vulkan/tests/vb_geo_preprocess_sync.rs`, landing at DP6b**, cloning `find_dxc()`/temp-dir discipline from `cluster_cull_spv_sync.rs`. Two assertions: (i) `dxc -P vb_geo.comp.hlsl` (no defines) is **character-identical** to the pre-DP6b file's `-P`; (ii) with `-D VB_SV0_TERM=1` it **differs**. **The pre-DP6b hash is RECOMPUTED via `git show <DP6b^>:crates/.../vb_geo.comp.hlsl`, not committed** — a committed literal is a datum nobody re-derives and the first "fix" is to re-bless it; `git show` makes staleness impossible. Skips (with `eprintln!`) when no pinned `dxc` resolves, per house idiom.

**Property-based (quantified, not sampled):** `vb_sdf_mesh_armable() ⇒ mesh_geo_shade_split`; `mesh_geo_shade_split ⇒ thin_aux.NORMAL`; `vb_geo_mv_active() ⇒ !vb_sv0_host` (the variant-count proof); `vb_sdf_mesh_mode != 0 ⇒ vb_sv0_host` (invariant 10).

**Q5 answered.** No committed pin covers split+SV0-armed. `[vb_mesh_ssao]` is VB×**Mesh** — no SDF leg — so `SDF_SOFT_MARCH` never arms and it can *never* pin SV0; `[vb_both_sdf]` is fused. With the new bit defaulting false, none would arise by itself. **This is not left as intended-but-unpinned:** DP6c adds `[vb_both_ssao]` (split, SV0 **disarmed**) as a real byte pin on the boot class the rung most changes, and seeds `[vb_both_ssao_sv0]` **PENDING** for the owner-eval packet — the `[vb_both_sdf]` precedent. The armed combination stays proven by adequacy floors until the owner blesses the frame.

**Red mutations to DEMONSTRATE:** (1) move a statement outside `#ifdef VB_SV0_TERM` → `-P` gate reds; (2) drop the `mesh_geo_shade_split` conjunct → `sv0_armable_requires_the_split` reds; (3) default the consumer bit `true` → goldens red; (4) move `Buf`'s vk::binding without the table → `sdf_field_edsl_sync` reds; (5) drive the pick from `mode != 0` instead of `vb_sv0_host` → arm B binds `base` and `Δ_AB` collapses to the whole march (the P0-3 mutation); (6) drop the `sdf_term` access while keeping the pick → validation/sync red on arm B.

## Open questions

1. **G-NEUTRAL can fail on fused boots.** `vb_shade_split` is not bit-for-bit `vb_resolve` (different Set 1, SSAO combine, DDGI sampling — runtime-gated off but compiled in), and the split adds the `thin_normal` write. If the fused row reds, the disposition is **restrict DP6e to split boots and keep the dedicated pass for fused** — i.e. fall back to the critic's option (b) *with a measurement behind it* rather than as a premise. Pre-agreed here so it is not improvised under a red.
2. **DP6d.5 may say half-res is wanted.** Then DP6e must name the post-retirement host shape for it, and the honest answer may be "a new minimal half-res marcher pass" — which partially un-does the consolidation. Recorded as a real risk of Decision 3, not hidden.
3. **DP2's and DP4's null resolutions disagree** (~24 576 ns vs 1 024 ns, same date, different fixtures) and both are load-bearing for their PASS verdicts. DP6-0 must re-certify on its own fixtures and **inherit neither**. → `docs/OPEN-QUESTIONS.md`.
4. **`docs/RENDER-PARITY-PLAN.md` §3.2B does not exist** — repo-wide grep for `3.2B` returns zero; §3.2 is the A/B/C options list, next heading §3.3. The lever's entire prior written form is one sentence in DP4's disposition. DP6a adds the erratum rather than pretending the subsection existed.
5. **External corroboration still pending.** A `researcher` sweep on published `vb_geom_fetch` cost shares, dedicated-vs-inline practice and its occupancy/VGPR rationale, half-res march savings, and async-compute overlap has not returned to me. Two findings could still move Rev 2: a documented occupancy cliff for merging a long march into a geometry pass would strengthen G-NEUTRAL's risk (open question 1); a large published half-res saving would raise DP6d.5 from probe to blocking rung. **Neither can change the fetch/dispatch counting table**, which is measured on this box — which is why the rung's justification now rests on that table and on consolidation, not on external practice.