# Post-processing and anti-aliasing — the design for THIS engine

> **Status:** architect's design, 2026-09-25, **pass 2**. It is revised after the pass-1
> architecture critique (1 critical, 9 important, 6 optional remarks, 7 open questions). §15 is the
> review log: every remark, what was done, and where. It targets trunk **`6394bc5e`** (branch
> `integ/unified`), read through `D:/wt/docs`.
>
> **The survey it rests on** is [`POSTFX-AA-RESEARCH.md`](POSTFX-AA-RESEARCH.md). That document's
> §0 lists what the tree holds today. It numbers the techniques **T1–T31** and the pitfalls
> **P1–P29**, and this document cites both by number.
>
> **What was not done.** Nothing was timed for this document, and no `cargo` command was run. Every
> number is one of three kinds:
> - published elsewhere, cited with its rig;
> - derived, with the bytes, fetches and rates shown;
> - a labelled estimate **[E]**.
>
> **Citations.** Code is cited as a path plus a symbol. A `:line` appears only where it was
> re-opened at `6394bc5e`. This directory is not in `GATED_DOCS` (`tests/internal_docs_anchors.rs`).
>
> **Where decisions sit.** §12 lists the technical decisions taken here. §13 holds the only
> questions that go to the owner: values and scope.
>
> **This is a delta document** (§1). It extends
> [`REFLECTIONS-DESIGN-SPACE.md`](REFLECTIONS-DESIGN-SPACE.md) R4a (= transparency R11),
> [`../TAA-PLAN.md`](../TAA-PLAN.md), [`../RENDER-AA-AND-TAILS-PLAN.md`](../RENDER-AA-AND-TAILS-PLAN.md),
> and Track E, X-SPD and C-TSR of
> [`../OPTIMIZATION-PLAN-RENDER.md`](../OPTIMIZATION-PLAN-RENDER.md).
>
> **Related same-day documents**, all uncommitted when this was written, so they are cited by path
> and not linked: `docs/ENGINE-GAP-AUDIT-2026-09-25.md` (it also concludes "one fused post pass plus
> the neighbourhood passes"), `docs/render/VOLUMETRICS-DESIGN-SPACE.md`,
> `docs/render/REFLECTIONS-UPDATE-2026-09-25.md` and `docs/render/VFX-DESIGN-SPACE.md`. §5.3 hands
> each of them an obligation. Nothing else here depends on them.

## 0. The shape in one paragraph

**The engine's post chain is one fused compute pass**, `post_final`, which turns HDR scene colour
into the 8-bit `display` image. It is fed by a small number of neighbourhood passes, all of which
run **after the temporal resolve** and all of which are compute.

**The HDR move.** `post_final` is R4a's `tonemap` pass grown into an uber pass. The HDR scene colour
that feeds it is R4a as designed, with one amendment to its gate (§5.1):
- `lit` becomes B10G11R11, or RGBA16F on devices without it (RK-14);
- the tonemap and OETF tails leave eight producers;
- eight transition pins guard the move.

**Two contracts the HDR move creates** (new in pass 2):
- **Finite values.** Today every producer stores into RGBA8 UNORM, which turns NaN into 0 on the
  way in. HDR formats keep NaN and Inf, so a named sanitiser runs at every first reader of `lit`
  and on every cross-frame state write (§5.2).
- **Pre-exposure across frames.** Every radiance value is stored in the pre-exposure units of
  the frame that wrote it. Any pass that reads a previous frame's radiance rescales it by
  `pre_N / pre_{N−1}` at the read (§5.3).

**What feeds `post_final`**, in frame order:
1. **TAA**, or TAAU once the render scale drops below 1. The existing resolve moves pre-tonemap
   with its luma weighting already on. Its RGBA16F history becomes the HDR input of everything
   after it.
2. **DOF**: a half-resolution scatter-as-gather.
3. **Motion blur**: a McGuire tile reconstruction, camera-only first.
4. **Bloom**: a 13-tap/tent pyramid from half resolution.
5. **Exposure**: a histogram of absolute luminance built on the bloom chain's quarter-resolution
   level, adapted on the GPU inside the same frame.
6. **The grading LUT**, baked every frame it is armed.

**What `post_final` does**, per pixel, in one read of the scene:
- the RCAS sharpen (FSR 1's 5-tap kernel), moved here from its own pass;
- the exposure ratio;
- the bloom composite and lens dirt;
- CA and vignette;
- the analytic or LUT tonemap and grade;
- the OETF;
- dither, always at this 8-bit write;
- grain, when no spatial pass follows.

It writes `display`. Two spatial passes may follow it:
- **FXAA or SMAA**, when the owner picks spatial AA instead of TAA;
- **the spatial upscaler** (EASU), when the render scale is below 1 on a configuration without TAA.

When either runs, grain moves to `present_blit`. The present pass and the UI follow.

**On the ECS side.** Post settings are **components on the camera entity**:
- `Exposure`, `Tonemapping`, `ColorGrading`, `Bloom`, `LensEffects`, `DepthOfField`,
  `MotionBlur`.
- One cold system resolves the active camera's components into a `ResolvedPost` carrier, every
  frame.
- Image allocation is armed by the union of components present on any camera, recomputed every
  frame, so a world whose cameras carry no post component pays for R4a's tonemap only.

**Why this suits a hybrid engine.** Every post pass reads only `lit`, `gViewT` and the history,
and both legs write those images. The whole chain therefore has **no mesh-versus-SDF fork**. Its
cost is linear in pixels and flat in lights, objects and SDF edits. Where the hybrid does matter,
the design says so (§7):
- TAA is the only AA that reaches SDF pixels;
- rendering fewer pixels (TAAU, or EASU on paths without TAA) pays the per-pixel SDF march back
  almost linearly;
- specular AA and motion vectors have to split by leg.

---

## 1. What earlier documents decided — unchanged, changed, and why

| Earlier decision | Where | Here | Why |
|---|---|---|---|
| HDR `lit`: B10G11R11 behind RK-14, else RGBA16F; tonemap and OETF leave **eight** producers; transition pins, one per producer; "all 30 pins re-bless"; a `lit_prev` copy | [R4a](REFLECTIONS-DESIGN-SPACE.md), [R11](TRANSPARENCY-DESIGN-SPACE.md) | **Adopted** as rung **PX1**, with two corrections: the re-bless covers **61 blessed legs** (34 software + 27 `hwrt`, `goldens/PINS.toml`), not 30 (RESEARCH T1), and the transition pin becomes a ≤ 1/255 tolerance pin instead of a byte-equal hash (§5.1, A1). | It is the prerequisite of T1 and every HDR effect. A byte-equal hash cannot hold through a 5–6-bit-mantissa store (§5.1). |
| R4a's single `tonemap` pass: `lit (HDR) → display (LDR, OETF)` | R4a §2.4 | **Changed**: the pass is named `post_final` and is the uber pass of §5. At PX1 its arithmetic is exactly R4a's tonemap, so R4a's transition pins still apply. | One full-resolution read and write for the whole LDR tail. §6 shows five separate passes would cost about 5× the bandwidth. |
| R4a: SSAA becomes an HDR box filter `lit(2×) → lit(native)` | R4a | **Adopted, and the native image is named**: `lit_native`, lit format, native extent, single, seeded (§4). `aa_out` is no longer SSAA's output. | Pass 1 kept RGBA8 `aa_out` as SSAA's output, which would clip the quality-reference mode before exposure, bloom and tonemap (P2). |
| TAA writes `aa_out` (RGBA8), then the sharpen ping-pongs through `taa_resolved` | as built, `targets.rs` | **Changed at PX1**: TAA writes only `taa_hist[fi]`, which `post_final` reads. The sharpen moves into `post_final` as FSR 1's 5-tap RCAS. The kernel the tree ships under the `SharpenMode::Rcas` name is AMD **CAS** (3×3, 9 taps, `rcas.comp.hlsl`), and it is retired. `aa_out` is allocated only for FXAA and SMAA. | This saves a 4 B/px write inside TAA, removes an 8 B/px pass, and frees 4 × 4 B/px of images: −44 MB at 1440p with the sharpen armed (§6.3). RCAS reads 5 taps rather than CAS's 9, and it requires input in [0, 1], which the reversible space meets (§5.11). |
| The resolve's input is finite (implicitly: producers store into RGBA8 UNORM; the resolve comment "No NaN risk") | as built, `taa_resolve.comp.hlsl:410` | **Changed**: a named sanitiser `hdr_sanitize` (§5.2). | Pass 1 cited a TAA `clamp(out, 0, 64)` that does not exist. |
| `display` has full resolution; ringing unspecified | R4a table | **Decided**: a **single** image with a cross-frame seed (`seeded_readers`), not one per frame in flight. | There is one queue, so frames execute in submission order. A seeded WAR barrier orders frame N+1's write after frame N's present read, as the CSM cascade, shadow atlas and HZB already do (`add_image_mipped`'s doc). Saves 14.7 MB at 1440p. |
| TAA runs post-tonemap in LDR; "revisit if `lit` ever goes HDR" (Decision 4, open item 8) | [TAA-PLAN](../TAA-PLAN.md) | **Closed by PX1**: pre-tonemap, luma-weighted (the weighting is already on by default), **with the history rescaled by the pre-exposure ratio on read** (§5.3). | R4a booked this coupling; T21; P11; P26. |
| TAA-PLAN Decisions 1 (jitter), 2 (camera-only motion vectors first), 3 (RGBA16F history), 5 (structural OFF gate) | TAA-PLAN | **Unchanged.** | Still correct; the history format already suits HDR. |
| "No depth-based history rejection ships" | TAA-PLAN research basis | **Corrected.** FSR2 ships a depth clip (T22). PX7 wires `DisocclusionTest::OffScreenAndDepth` through `viewt[1-fi]`. | `viewt` is one image per frame in flight, so the previous frame's depth already exists and **no new image is needed** (RESEARCH C6). |
| "Post-TAA sharpen pass: roadmap or no?" (open item 4) | TAA-PLAN | **Resolved**: the sharpen exists and moves into `post_final`. | §5.11. |
| The `resolve → present` seam is the post insertion point; post shaders are hand-written HLSL and "the eDSL feedback rule does not apply" | [AA-AND-TAILS](../RENDER-AA-AND-TAILS-PLAN.md) §1 | **Changed**: the seam becomes `post_final`, with temporal AA before it and spatial passes after it. **Post math goes through the eDSL** (§9), on the existing `Cf` axis. | Owner rule 6. The tonemap curves already have two consumers today (GPU and `goldens.rs` `tonemap_and_oetf`) and gain a third, the LUT bake. |
| AA set FXAA / SMAA 1x / TAA / SSAA 2×; MSAA excluded | AA-AND-TAILS §2 | **Unchanged**. MSAA stays excluded, now with numbers (§5.12). No CMAA2 and no SMAA T2x. | §5.12. |
| C-AA analytic SDF-edge AA parked | [C-AA](../RENDER-C-AA-DESIGN-PARKED.md) | **Unchanged**, still parked. | TAA with `RasterAndBasis` already covers SDF edges. PX8 TAAU is its third prerequisite ("temporal AA supersedes analytic edge AA"). |
| Post is POLISH-TIER (M1) | [OPT-PLAN](../OPTIMIZATION-PLAN-RENDER.md) | **Partly changed.** Exposure, bloom and the LUT are no longer optional polish once `lit` is HDR, because the image needs *some* exposure and tonemap decision. DOF, motion blur and lens flare stay polish. The owner decides their scope (§13 V7). | T1. |
| E-TONE "needs E-EXP + E-BLOOM" | OPT-PLAN Track E | **Changed**: the tonemap is R4a's, landing at PX1 before either. | The tonemap is the HDR move itself. |
| E-EXP "needs P6-S" (the cross-frame semaphore seam, M3) | OPT-PLAN §1b | **Changed: P6-S is not a prerequisite.** Adapted exposure is a 1×1 GPU buffer carried across frames with `ResSync::seeded_writer`, the documented "cluster `alloc` counter" precedent. | TAA history and DDGI already carry state across frames on the single queue without anything named P6-S. P6-S's stated purpose is the acquire/present semaphore chain, which exposure does not touch. |
| E-BLOOM "consumes X-SPD" | OPT-PLAN M4 | **Changed**: bloom does not wait for X-SPD. It builds per-level dispatches on `add_image_mipped`. X-SPD stays a primitive for true 2×2 reductions (HZB, GTAO). | SPD reduces non-overlapping 2×2 blocks per level (T28). The 13-tap downsample reads a 6×6 overlapping footprint, and the up-chain is not a reduction at all, so neither is an SPD operation. |
| E-DOF: "separable disk / scatter-as-gather (complex-phasor separable)" | OPT-PLAN | **Changed**: scatter-as-gather at half resolution only. | §5.9: the phasor rings and needs 6+ channels (T12); gather is what UE and CoD ship (T11). |
| E-MBLUR "consuming P6's R16G16 motion vectors" | OPT-PLAN | **Changed**: camera-only velocity reconstructed inline from `gViewT` and `MotionCam` first (PX10). Per-object motion vectors follow at PX11. | Off `hwrt` there is no motion-vector buffer; `MvSource::PerObject` was declined and its doc says why. |
| E-LUT: grain "intentionally per-frame (non-deterministic by design)" | OPT-PLAN | **Changed**: grain varies per frame but is **keyed on the frame serial**. | P23: goldens must stay deterministic. |
| C-TSR: "Full FSR2/TSR … FSR2 algorithm ported, never linked" | OPT-PLAN | **Refined**: TAAU as an extension of the existing resolve (PX8), on a render extent decoupled from the display extent for **every** path, sub-rectangle capable so dynamic resolution needs no second retrofit (PX8a). FSR2's depth clip, reactive mask and locks are ported as measured needs arise. Never linked. | §5.13: the resolve already does reprojection, clipping and Catmull-Rom history. FSR2's persistent memory is 164 MB at 1440p Quality against 59 MB for the house history (T22). |
| (none) Paths without TAA have no resolution lever | — | **New**: an in-house port of FSR 1's EASU + RCAS for configurations with a render scale below 1 and no TAA (PX8s). | §5.14: the SDF march costs per pixel, and Forward's `sdf_forward_march` has no TAA. |
| TAA coverage mask `taa_cov` for translucents (R6) | [TRANSPARENCY](TRANSPARENCY-DESIGN-SPACE.md) | **Unchanged**; PX7 consumes it. | Not duplicated here. |
| Hashed alpha-test `MASKED` variant (R2) | TRANSPARENCY | **Unchanged**; it is the alpha-test AA answer (T26). | — |
| Mesh motion-vector MRT plus the dense `PrevInstanceModelCol` | [Rung 3b](../RENDER-RUNG3B-TAA-PLAN.md) | **Unchanged**; PX11 un-walls it from `hwrt`. | — |
| "Accept the LDR limitation for P0–P4, or schedule HDR first?" | [PARTICLES](../PARTICLES-PLAN.md) | **Answered by §13 V1 and PX1.** Under pre-exposure, `particle_draw` must multiply its output by the pre-exposure scalar (§5.3 E1). | `particle_draw.fs.hlsl` applies no exposure today. |
| G5: a user post node; `EngineImage::LitHdr` | [RENDER-GRAPH-API](../RENDER-GRAPH-API-PLAN.md) | **Unchanged**. After PX1: `AfterResolve` sees HDR, `BeforePresent` sees `display`, and `LitHdr` names a real image. | — |

---

## 2. The frame

### 2.1 Order (every path; brackets name the rung that adds the pass)

```
opaque producers → lit (HDR, × pre_exposure)          resolve | forward_opaque | vb_shade/vb_resolve | sdf_forward_march
       fog_apply (the volumetrics campaign; composes on HDR lit before the copy)
[R4a]  lit_prev copy → SSR / translucents / particles (their campaigns; all draw into lit, HDR, × pre_exposure)
taa_resolve        hdr_sanitize(lit) + viewt[fi] (+ viewt[1-fi], taa_cov at PX7)
                   + taa_hist[1-fi] × pre_ratio → taa_hist[fi]                    (Deferred, VB)
   or ssaa_downsample (AaMode::Ssaa, R4a's HDR box filter)    : lit(2×) → lit_native
[PX9]  dof_prefilter → dof_tiles → dof_gather → dof_composite          → post_hdr_a      (Deferred, VB)
[PX10] mb_tilemax → mb_neighbormax → mb_gather                           → post_hdr_b      (declared only on frames the camera moved, never on a cut)
[PX3/PX4] post_down L1, L2  →  [PX4] bloom_down L3..L6 → bloom_up L5..L1
[PX3]  exposure_histogram (reads L2, meters absolute luminance) → exposure_adapt (1×1, cross-frame, guarded)
[PX5]  post_lut_bake (32³ log2, or 48³ x/(x+1) when the curve is Tony)
post_final   scene (5 taps for RCAS) × exposure ratio + bloom (tent from L1) × dirt; CA, vignette;
             tonemap (analytic | LUT); OETF; dither; grain unless a spatial pass follows
                                                                           → display (RGBA8, single, seeded)
[AaMode::Fxaa | Smaa]  fxaa / smaa_edge → smaa_weight → smaa_blend : display → aa_out
[PX8s] easu : display | aa_out (render extent) → easu_out (display extent)       (RenderScale < 1, no TAA)
present_blit (+ RCAS after EASU; + grain when a spatial pass ran; + UI rects) : display | aa_out | easu_out → swapchain
```

**What each path runs.**
- **Forward and Forward+** run `post_final`, bloom, exposure, grading and lens effects, plus
  FXAA or SMAA once PX6 adds the seam there, plus EASU once PX8s lands.
- They do not run TAA, DOF or motion blur, because they have no `gViewT` for SDF pixels. The
  invariant in `declare_forward_graph` says so. The reflections design's RK-12 `fwd_viewt` is
  what would change that.

### 2.2 Placement relative to TAA — decided

The candidates are T21's three shipped orders (RESEARCH §3):

| Order | DOF and motion blur see | Cost | Fit |
|---|---|---|---|
| **(a) HDRP / FSR2**: temporal resolve first; DOF, motion blur, bloom, tonemap after | Stable, anti-aliased HDR | DOF and motion blur at display resolution | **Chosen** |
| (b) UE: DOF before TSR at render resolution; motion blur, bloom, tonemap after | Jittered input; DOF must be temporally robust (Abadie's slight out-of-focus problem, P12) | DOF at render resolution: 0.44× the pixels at Quality | Only pays under TAAU |
| (c) Everything before the temporal resolve | Motion blur and DOF inside the history make the variance clip reject blurred pixels | — | Rejected. No surveyed engine does this. |

**Why (a).** There is no upscaler until PX8, so (b)'s only advantage, render-resolution DOF, does
not exist yet. HDRP and FSR2 agree on (a). With (a), the temporal resolve's input is the scene
and nothing else.

**What would reopen it.** Under TAAU at 1.5×, DOF costs 2.25× more at display resolution than at
render resolution. §6 puts DOF's worst case at 0.67–1.14 ms at 1440p [E]. The trigger is recorded
in §12 D3: if PX9's zone shows DOF above 0.5 ms at the owner's target resolution once PX8 exists,
evaluate (b) with both numbers.

### 2.3 Compute or raster, and at what resolution

| Pass | Kind | Resolution under a render scale < 1 | New or existing |
|---|---|---|---|
| `taa_resolve` (becomes TAAU at PX8) | compute | display | existing |
| `dof_*`, `mb_*`, `post_down`, `bloom_*`, `exposure_*`, `post_lut_bake`, `post_final` | compute | display under TAAU; render under EASU (bloom and DOF internally at 1/2 and below) | new |
| `fxaa`, `smaa_*` | fragment | display under TAAU (not armed together); render under EASU | existing |
| `easu` | compute | display (reads render) | new (PX8s) |
| `ssaa_downsample` | fragment | native | existing (R4a changes its body and its target) |
| `present_blit` + UI | fragment | display | existing |

Every new pass is compute:
- It follows the tree's own precedent: `taa_resolve` and the sharpen are compute.
- It needs no render pass, attachment or blend state.
- It can early-out per tile through indirect dispatch. **The RHI gap is real (§8 G7):** the raw
  `vkCmdDispatchIndirect` is loaded and particles call it directly, but the trait method
  `RhiCommandEncoder::dispatch_indirect` is a silent no-op (`crates/boyko_rhi/src/encoder.rs:408`)
  that the Vulkan backend does not override. The sibling reflections update books the override as
  RK-16. PX9 depends on it.

---

## 3. Data model — components, resources, systems

### 3.1 Where the settings live: on the camera

**The choice.** T27 gives two models:
- a global resource, which is how `AaConfig`, `TaaConfig` and `LightingConfig` work today;
- components on the camera entity, which is Bevy's model for exactly these effects.

**Decided: components on the camera, resolved into one carrier.** Four reasons:

1. **Owner rule 3.** Capability must be structural. A camera that cannot bloom lacks `Bloom`, so no
   system iterates it and no bloom image exists for it.
2. **Several cameras, one view.** The engine allows many `Camera` entities but renders one per
   frame (RESEARCH §0.5). A cutscene camera can therefore carry its own DOF without a global
   toggle racing a camera switch.
3. **Proven practice.** Bevy ships this model (T27).
4. **AA and render scale stay global.** `AaConfig` stays a global `Resource`, because AA mode and
   render scale decide target extents, which belong to the output surface, not to a camera.

### 3.2 Components

All components live in `boyko_render`, in a new `post` module. Field lists are indicative; the
final shapes are an implementation decision.

| Component | Key fields | Arms |
|---|---|---|
| `Exposure` | `mode: ExposureMode { Manual { ev100 }, Auto(AutoExposure) }`, `compensation_ev`. **`Physical { aperture_f, shutter_s, iso }` is not offered until rung PU** (§5.4). | `Auto` arms `post_down` (L1, L2), the histogram, the adapt pass and the readback. `Manual` arms nothing, because it only changes the pre-exposure scalar. |
| `Tonemapping` | `curve: { Aces, Neutral, ReinhardJodie, AgX, Tony }` | `AgX` and `Tony` arm the LUT path (§5.6). `AgX` is conditional on its licence (§5.6). |
| `ColorGrading` | white balance (K, tint), saturation, contrast, lift, gamma, gain, `lut: Option<LutHandle>` | the LUT bake |
| `Bloom` | `intensity`, `mips` (≤ 6), `dirt: Option<TextureHandle>`, `dirt_intensity` | `post_down` plus the bloom chain |
| `LensEffects` | vignette (intensity, smoothness), chromatic aberration (px), grain (intensity, size), `dither: bool` | nothing: these are `post_final` (and, for grain, `present_blit`) mode words |
| `DepthOfField` | `Physical { focus_m, aperture_f, sensor_mm }` or `Artistic { near, far, max_coc_px }` | the DOF targets (Deferred and VB only). `Physical` here is thin-lens geometry in metres; it needs no light units. |
| `MotionBlur` | `shutter_angle` (default 0.5, i.e. 180°, as Bevy uses, T13), `samples`, `max_px` | the motion-blur tiles and output (Deferred and VB only) |

`LightingConfig::{exposure, tonemapper}` stay, as the **fallback** for a camera that carries no
`Exposure` or `Tonemapping`. Reasons:
- a world that never adds a post component renders byte-identically;
- moving these fields is a refactor, and the owner's rule is refactoring last.

The single writer below resolves the precedence, so there is never a split-brain carrier.

### 3.3 Resources and systems

| Item | Kind | Role |
|---|---|---|
| `ResolvedPost` | `Resource` (derived) | The carrier the render driver reads each frame: the pre-exposure scalar and the ratio `pre_ratio = pre_N / pre_{N−1}` (§5.3), mode words, the camera-cut flag, the arm bitset, and the uniform values for `post_final`, bloom, exposure, DOF and motion blur. It follows the existing Config → Resolved pattern (`ssao_config.rs`, `taa_config.rs`). |
| `resolve_post_policy` | system, cold, single writer | Every frame, it reads the active camera's post components, with `LightingConfig` as fallback and the `ExposureReadback` ring, and writes `ResolvedPost`. It also computes the arm bitset over all cameras and detects a camera cut. |
| `PostArm` | a bitset inside `ResolvedPost`, **recomputed every frame** | Which post images exist: the **union over every `Camera` entity's components**, a presence scan of O(cameras). `GBufferTargets::sync_gbuffer` already compares `aa_arm` and `hzb_arm` against the scene every frame; `PostArm` joins that predicate. Adding or removing a post component on any camera therefore triggers the same fence-safe rebuild on the next frame. |
| Camera cut | a `CameraCut` event, or a change of the active camera entity | `resolve_post_policy` sets `ResolvedPost::cut`. The runner's existing `TaaState::mark_reset` site consumes it, so `TaaState` keeps its single writer. On a cut: exposure adaptation snaps to its target (α = 1), TAA resets, and motion blur is not declared that frame. |
| `ExposureReadback` | `Resource`, one host-visible 16 B slot per frame in flight | The render driver copies frame N−2's adapted exposure into it after that slot's fence wait. The fence wait already happens, so there is no stall. |
| `PostPlugin` | optional plugin | Registers the component types, `ResolvedPost` and the policy. Without it, `ResolvedPost::default()` means "R4a tonemap only". Nothing is built, declared or recorded beyond that, so it costs zero. |

**Why arming is the union over cameras, not the active camera alone.** With active-camera arming,
a cut to a DOF camera triggers a target rebuild: a fence wait plus about 30 MB of allocation at
1440p, a visible hitch. With union arming, the images exist if any camera can use them. Each
frame, the active camera decides at declare time whether a pass is declared. The graph is
re-declared every frame anyway (`frame_driver.rs:896`), so an undeclared pass costs nothing.

**Where the runtime on/off lives.** Owner rule 3 allows a runtime flag only on an object that
carries the capability. So `Bloom { intensity: 0.0 }` on a carrying camera leaves the pass
declared, while removing `Bloom` removes the declaration. The same holds for `MotionBlur`, which
additionally skips its passes on frames where the camera did not move (§5.10).

**Why the camera cut matters.** The sibling states that read the past (TAA history, `lit_prev`,
the fog history, `exposure_state`) all belong to the camera that wrote them. Today TAA resets only
on a mode change or a resize (`TaaState`'s doc; `runner.rs`), so a camera switch blends two
cameras' histories. The cut flag is published in `ResolvedPost` so every campaign's temporal
state can consume it (§5.3).

**Hot-path check (rule 4).**
- `resolve_post_policy` reads at most 7 components of one entity plus two resources, scans the
  camera set for component presence, and writes a fixed-size struct.
- No allocation, no lock, no `dyn`.
- The LUT asset is a handle into an append-only asset column: the `EnvAsset` / `EnvHandle(u32)`
  pattern from the reflections design §1.1.

---

## 4. GPU resources

Sizes are decimal MB. `lit format` means B10G11R11 when RK-14 passes, otherwise RGBA16F; the
table sizes it at 4 B/px.

| Image | Format | Shape | B/px | 1080p | 1440p | 4K | Rung |
|---|---|---|---|---|---|---|---|
| `lit` (re-typed) | lit format | one per frame in flight, as today | 4 × 2 | same as today | same | same | PX1 (R4a) |
| `display` | `R8G8B8A8_UNORM` storage + sampled | **single**, `seeded_readers`; render extent under EASU | 4 | 8.3 | 14.7 | 33.2 | PX1 |
| `lit_native` | lit format, colour attachment + sampled | single, `seeded_readers`; **only under SSAA**; native extent | 4 | 8.3 | 14.7 | 33.2 | PX1 |
| `aa_out` | RGBA8 | one per frame in flight; **only for FXAA and SMAA** after PX1 | 8 | 16.6 | 29.5 | 66.4 | freed under TAA and SSAA |
| `taa_resolved` | RGBA8 | retired at PX1 (sharpen folded) | −8 | −16.6 | −29.5 | −66.4 | PX1 |
| `taa_hist` | RGBA16F | one per frame in flight, unchanged | 16 | 33.2 | 59.0 | 132.7 | exists |
| `post_chain` (L1..L6) | lit format | single, `add_image_mipped` | 1.33 | 2.8 | 4.9 | 11.1 | PX3 (L1, L2) / PX4 |
| `exposure_hist` + `exposure_state` + readback | buffers | single + host ring | — | ~1.1 KB | | | PX3 |
| `post_lut` | RGBA16F **3D** storage + sampled; 32³, or 48³ for Tony | single | — | 0.26 (0.88) | 0.26 (0.88) | 0.26 (0.88) | PX5 |
| `dof_half_a`, `dof_half_b` | RGBA16F, half resolution | single | 4 | 8.3 | 14.7 | 33.2 | PX9 |
| `post_hdr_a` | lit format | single | 4 | 8.3 | 14.7 | 33.2 | PX9 or PX10 |
| `post_hdr_b` | lit format | single; only when DOF **and** motion blur are armed | 4 | 8.3 | 14.7 | 33.2 | PX10 |
| `mb_tiles` (K = 20 px, max + neighbour-max) | RG16F | single | ≈ 0.02 | < 0.1 | < 0.1 | 0.2 | PX10 |
| `easu_out` | RGBA8 storage + sampled | single, `seeded_readers`; only when EASU is armed; display extent | 4 | 8.3 | 14.7 | 33.2 | PX8s |

**Net change at PX1 under TAA at 1440p:**

| Item | MB |
|---|---|
| `display` added | +14.7 |
| `aa_out` no longer needed | −29.5 |
| `taa_resolved` retired (when the sharpen is armed) | −29.5 |
| **Net, sharpen armed** | **−44.3** |
| **Net, sharpen off** | **−14.8** |

**Under SSAA**, `aa_out` (2 × 4 B/px) is replaced by `lit_native` (4 B/px; 8 on the RGBA16F arm)
and `display` (4 B/px): net 0 to +4 B/px at native resolution.

**The default look at 1440p** (PX1–PX6 under TAA) adds 4.9 MB of bloom chain and 0.3 MB of
exposure and LUT data, so post still ends **below** today's footprint.

**Every new image is framegraph-tracked:**
- In the Deferred declarator's fixed ResId block, each is appended after the last existing
  image and raises `FRAMEGRAPH_IMAGE_COUNT` by one.
- In the Forward and VB private spaces, the same append-last discipline applies.

`aa_out` itself moves into the graph at PX6, which removes P19.

---

## 5. Technique decisions (options scored against the owner rules)

The rules, in shorthand:

| Rule | Shorthand |
|---|---|
| R1 | Web-sourced |
| R2 | Hybrid performance decides |
| R3 | Principle 0: structural capability, zero cost when unused |
| R4 | Hot path: no allocation, locks or `dyn` |
| R5 | In-house by default |
| R6 | Shared math in the eDSL; variants in the manifest |

GPU numbers are from §6.

### 5.1 HDR scene colour — adopt R4a, with one amendment to its gate

R4a's format decision (B10G11R11, 4 B/px, the same bandwidth as RGBA8, behind RK-14, with an
RGBA16F fallback), its eight per-producer transition pins and its re-bless are **adopted**. What
this design adds on top is listed in §1.

**The amendment (A1, found in this pass).** R4a's transition pin says "the pre-move `.bmp` hash is
the post-move `display` hash". A hash cannot hold through an HDR store:
- B10G11R11 keeps 6 mantissa bits for R and G and 5 for B. The worst rounding error is therefore
  2⁻⁷ (0.78 %) for R and G and 2⁻⁶ (1.56 %) for B.
- After the tonemap and the `1/2.2` OETF, that is about 0.36 % and 0.71 % of the display value
  [E], taking the tonemap's log-slope near mid-grey as about 1. At a display value of 128 it
  moves a channel by up to 0.46 and 0.91 LSB.
- So many channel values flip by one LSB. Even the RGBA16F arm (2⁻¹¹) moves about 0.03 LSB at
  128, which flips the few per cent of values that sit that close to a rounding boundary [E].

The pin keeps its purpose, which is to catch a producer that keeps its tail and double-tonemaps.
A double tonemap moves values by tens of LSB, not by one. So the pin becomes:
- **max |Δ| ≤ 1/255 per channel** against the pre-move `.bmp`, on both arms;
- the fraction of changed values is reported, not gated.

The reflections design owns R4a, so §15 hands the amendment to that campaign.

**Particles.** `particle_draw` is a ninth writer of `lit`. Under R4a it blends in HDR, which is a
declared look change: particle colours authored display-referred now add in scene-linear. The PX1
re-bless covers it, and the particle pins carry the change note. It must also carry the
pre-exposure (§5.3 E1).

**Pre-exposure (T2).** The scalar exists from PX1: at PX1 it is simply
`LightingConfig::exposure`. It becomes adaptive at PX3. Two reasons it matters:
- **Precision.** B10G11R11 has a 5–6-bit mantissa and a finite maximum, so radiance must reach
  `lit` already near display scale.
- **TAA weighting.** TAA's luma weighting `1/(1+luma)` is scale-dependent (P11), so the resolve
  needs exposed values.

### 5.2 NaN and Inf at the HDR boundary — a named sanitiser

**Today's protection is implicit, and PX1 removes it.** Every producer stores into RGBA8 UNORM,
and the float → UNORM conversion maps NaN to 0 (RESEARCH §0.2). The resolve's own comment
depends on that: "No NaN risk: `cur_lit` is the resolve's own finite LDR output"
(`taa_resolve.comp.hlsl:410`). `taa_resolve.comp.hlsl` has no NaN, Inf or range clamp, and its
output is written raw (`:519`). Pass 1's claim of a TAA `clamp(out, 0, 64)` was wrong. After PX1,
`lit` is B10G11R11 or RGBA16F, and both keep NaN and Inf.

**What one NaN does after PX1, traced through the default arms:**
1. A NaN in `lit` at pixel P poisons the 3×3 moments of its neighbours (`mean += c` propagates it).
   Their clip box becomes NaN. At P itself, `cur_lit` is NaN, so the blend writes NaN into
   `taa_hist`.
2. Next frame, every pixel whose 4×4 Catmull-Rom footprint contains P reads it. `sum += Load() *
   weight` propagates NaN even at zero weight, because 0 × NaN = NaN
   (`sample_history_catmull_rom`).
3. `clip_toward_aabb_center` cannot remove it. `ma_unit` is NaN, `ma_unit > 1.0` is false, and the
   function returns its input.
   - HLSL `min`/`max` lower to `NMin`/`NMax`, which return the non-NaN operand
     ([`../VB-SV0-SDF-SHADOW-PLAN.md`](../VB-SV0-SDF-SHADOW-PLAN.md) records the lowering). They
     do not rescue this path, because the NaN arrives through additions, not through `min`/`max`.
4. So the NaN region grows by up to 2 px per axis per frame, even under a static camera, and never
   decays. The history is read on every frame, so it eventually covers the screen. The 8-bit write
   of a NaN gives black.
5. **The same NaN reaching the bloom chain or the histogram is worse.**
   - The 13-tap filter spreads it across the pyramid.
   - A NaN luminance cannot be binned.
   - A NaN adapted exposure, carried across frames in `exposure_state`, blacks the whole image
     permanently.

**Candidates:**

| Option | Cost at 1440p [E] | Covers | Verdict |
|---|---|---|---|
| HDRP-style separate "stop NaN" pass over `lit` | 8 B/px ≈ 0.12 ms | every reader | Rejected: a full read and write for a check the readers can do on bytes they already load. |
| **`hdr_sanitize` at every first reader of `lit`**, plus a guard on every cross-frame state write | ≈ 10 ALU per tap; TAA's 9 taps ≈ 0.2 GOP ≈ 0.02 ms, likely hidden under the memory-bound resolve | every reader and every state | **Chosen** |
| A guard only on the history write | ≈ 0 | TAA only | Rejected: it leaves bloom, the histogram and `exposure_state` exposed. |

**The sanitiser.**
- `hdr_sanitize(c) = finite(c) ? clamp(c, 0, HDR_MAX) : 0`.
- **`finite` is an exponent-bits test**, `(asuint(x) & 0x7F800000) != 0x7F800000`, which no
  float-mode assumption can optimise away.
- `HDR_MAX` = 64,512, the largest finite value of B10G11R11's 10-bit channel. It is also inside
  RGBA16F's range, so a sanitised value survives either store.
- The whole pixel becomes 0, as HDRP's NaN killer does, rather than one channel.

**Where it runs:**
- on every `lit` tap in `taa_resolve`, `ssaa_downsample`, `post_down` and `post_final`, when these
  read `lit` directly;
- on the `taa_hist` write (`finite(out) ? out : cur`);
- on the `exposure_state` write (a non-finite adapted value keeps the previous one);
- in the histogram's binning, where a luminance below ε goes to bin 0 (Tardif's rule, T4).

**Its cost to debugging.** The sanitiser hides a producer bug that emits NaN. A test-only NaN
counter (an atomic in a bench feature) reports how many pixels it replaced, so a golden run can
assert zero.

**The gate (PX1, red-first).** A test-only hook writes one NaN pixel into `lit` for one frame.
Sixteen frames later:
- `taa_hist` must hold no non-finite texel;
- the display may differ from a clean run only within 2 px of the site.

The fixture is red without the sanitiser, by the trace above.

### 5.3 Pre-exposure across frames — the temporal contract

Pass 1 claimed pre-exposure "affects only precision, never the result". That holds within one
frame. It fails for every consumer that blends a previous frame's radiance, because pre-exposure
changes every frame while exposure adapts. The critique's example [E]: a 16× luminance step,
τ = 1.1 at 60 fps, moves the adapted value about 27 % per frame early on. A history stored in
pre_{N−1} units is then about 27 % off:
- on low-variance surfaces the variance clip throws it away, and the image shimmers;
- on edges it survives and smears brightness.

**The contract.**

- **E1 — writers.** Every writer of `lit` writes pre-exposed radiance, radiance × pre_N.
  - The eight producers already do, through the final multiply by `H.exposure`.
  - `particle_draw` does not: `particle_draw.fs.hlsl` applies no exposure today.
  - Fog apply, the SSR composite and translucents must do the same.
- **E2 — readers of the past.** A pass that reads radiance written in an earlier frame multiplies
  it by `pre_ratio = pre_N / pre_M` at the read, where M is the writing frame. The multiply comes
  before any clip, comparison or blend.
  - For one-frame-old state that is `pre_N / pre_{N−1}`.
  - The CPU supplied both values, so it computes the ratio and puts it in the reader's per-frame
    UBO. No readback is needed.
  - The consumers are:
    - TAA's `taa_hist[1-fi]` (this design);
    - SSR's `lit_prev[1-fi]` and the reflection denoiser's history (REFLECTIONS R5, R7);
    - the volumetric fog history (VOLUMETRICS V3, once V9 applies the scalar at injection).
- **E3 — caches that outlive frames.** A cache that persists many frames, such as the DDGI probes,
  stores **un-exposed** radiance. Exposure stays the final multiply after the cache read, as it is
  today. Such a cache never reads `lit`, so E2 does not apply to it.
- **E4 — metering.** The histogram meters absolute luminance, `log2(luma / pre_N)`. Otherwise
  the metered value contains E_{N−2}, which is a delayed feedback loop.

**Shipped practice.** FSR2 does exactly this (RESEARCH T2):
- `PrepareRgb` divides every input by its pre-exposure and clamps it to `[0, FSR2_FP16_MAX]`;
- `ReprojectHistoryColor` prepares the history with `PreviousFramePreExposure()`;
- its README defines pre-exposure as the value by which the input is divided to recover the
  original signal.

**Why rescale on read rather than store un-pre-exposed history:**
- the history stays in `lit`'s units, so the variance clip compares like with like;
- an un-pre-exposed RGBA16F history loses the precision pre-exposure exists for, and it can
  overflow once lights are physical (§5.4);
- it is FSR2's choice;
- it costs one multiply per read.

**When it lands: PX1, not PX3.** TAA goes HDR at PX1. From then on, any runtime change of
`LightingConfig::exposure` is a pre-exposure change, and it would mix units in the history. At
constant exposure the ratio is exactly 1.0, and `x * 1.0 == x`, so the rescale is byte-identical.

**What is now guaranteed.**
- Within a frame, the image equals radiance × E_N, up to storage precision.
- Across frames, with E2 applied by every temporal consumer, the image does not depend on the
  sequence of pre-exposures, up to precision.

Two fixtures check the second guarantee. Each is red without the rescale:
- **PX1:** over a converged static scene, ramp `LightingConfig::exposure` by ×1.25 per frame for
  eight frames. The TAA output at each frame must match a converged constant-exposure run at that
  frame's exposure within 1/255.
- **PX3:** hold the exposure E fixed and force the pre-exposure to ramp. The display must match a
  constant-pre-exposure run within 1/255.

**One scalar, several campaigns.** The volumetrics design applies the same `LightHeader.exposure`
scalar at fog injection as its pre-exposure (its V9). Whichever rung lands first (PX1/PX3 or V9)
defines the writer: `resolve_post_policy` through `ResolvedPost`. The others consume it. They must
not define a second one, and they take on the contract above:
- **E2** for the volumetric fog history (V3);
- **E2** for SSR's `lit_prev` (R5) and the reflection denoiser (R7);
- **E1** for `particle_draw` (the VFX and particles campaigns);
- the camera-cut reset (§3.3) for all of them.

This document can edit only itself and its survey, so §15 records the hand-off.

**P6-S is not needed.** `exposure_state` is a 1×1 buffer carried across frames with
`ResSync::seeded_writer`, the documented precedent for a sibling frame's undrained write
(`framegraph/sync.rs`).

### 5.4 Units — the tree's radiance scale, and what exposure means on it

**The tree's scale is display-referred, not physical:**
- `DirectionalLight::illuminance` is documented as "Illuminance in lux (physical)"
  (`light.rs:316`).
- The test scenes light their suns at 3.1 and 2.8 (`crates/boyko_app/tests/forward_mesh.rs:125`,
  `crates/boyko_app/tests/csm_fit_eval.rs:74`). Direct sunlight is of order 10⁵ lux (recalled, not
  fetched), about 2¹⁵ higher [E].
- `LightingConfig::exposure` defaults to 1.0 (`light.rs:564`).
- `sky_diffuse` and `sky_spec` are legacy unitless constants.

**Consequence for a physical camera.** Under `exposure = 1/(1.2 · 2^EV100)` (T3):
- exposure 1.0 is EV100 = −log2 1.2 ≈ −0.26;
- `Physical { f/16, 1/100 s, ISO 100 }` is EV100 = log2(256 / 0.01) = 14.64, which gives an
  exposure of 3.26 × 10⁻⁵.

That renders **2^14.9 ≈ 30,700× darker** than today on every golden scene, while arithmetic-only
host tests pass.

**Decided (D21): the convention is relative until physical units land.**
- Exposure 1.0 is the identity.
- `Exposure::Manual { ev100 }` is read on that scale: EV100 −0.26 is today's look.
- `Auto` meters on that scale.
- `Exposure::Physical` and physical presets (Bevy's `SUNLIGHT` 15, `INDOOR` 7, T3) are **not
  offered** until rung **PU**: physical light, sky and emissive units. PU is an owner scope
  question (§13 V9). It is shared with the volumetrics design, whose fp16 concern (its P23 and
  K6) already assumes a physical sky.
- The `ev100_from_physical` leaf still lands at PX3 with its host test, as PU's prerequisite. No
  component exposes it.

**Rejected:** a per-scene EV offset that calibrates `Physical` to relative scenes. It would make
"f/16, 1/100 s, ISO 100" mean something different in each scene.

**The histogram range is derived, not copied:**
- With the reflected-light constant K = 12.5, `EV100 = log2(L · 100 / K) = log2 L + 3`. So a scene
  whose average luminance meters to EV100 has `log2 L_avg = EV100 − 3`.
- The histogram covers `[ev_min − 3 − 4, ev_max − 3 + 4]`. The ±4 stops keep the percentile tails
  of a scene whose average sits at a clamp inside the bins; that margin is a choice [E].
- **On the relative scale**, the default clamp is today's identity ± 7 stops: EV100 ∈
  [−7.3, +6.7]. The histogram then spans log2 L ∈ [−14.3, +7.7]: 22 stops over 256 bins,
  0.086 EV per bin.
- **Under PU**, the default clamp becomes a physical one, for example EV100 ∈ [−2, 16], and the
  bins follow. Tardif's −10…+2 (T4) fits today's scale only by coincidence, and it would clip a
  physical sun, whose log2 L is about 13.

**What `Auto` will look like on today's scenes [E].** A sunlit surface in engine units is about
E·ρ/π ≈ 3 × 0.5 / π ≈ 0.5. That meters to EV100 ≈ 2, an exposure of about 0.21, which is roughly
2.3 EV darker than exposure 1.0. The first `Auto` pins will look darker than today.
`compensation_ev` is the knob, and its default is an owner value (V3).

### 5.5 Exposure — GPU-resident same-frame, with pre-exposure through an N−2 readback

**Fork 1: where exposure is applied.**

| Option | Latency on the image | Producer change | Cost | Verdict |
|---|---|---|---|---|
| (a) **Histogram → adapt → `post_final` in the same frame, GPU-resident.** Producers multiply by a CPU-supplied pre-exposure, which is frame N−2's adapted value read back after that slot's fence; `post_final` multiplies by `E_N / pre_N`. | 0 frames | **none**: the producers already multiply by `H.exposure` (`light.rs:564`), which becomes the pre-exposure. Temporal readers take the ratio (§5.3). | Histogram + adapt ≈ 0.02–0.04 ms at 1440p [E]; readback < 0.1 µs CPU [E] | **Chosen** |
| (b) CPU readback only: the image uses frame N−2's exposure | 2 frames | none | same | Rejected: visible two-frame lag on every exposure change, with no benefit over (a). |
| (c) Producers read GPU-resident pre-exposure | 0 | a new binding in **8** producers; the deferred-resolve set was recorded at its 16/16 cap when TAA-PLAN was written (its constraints table; not re-counted here) | same | Rejected: eight interface changes and eight manifest rows. The ratio in (a) gives the same image. |

**Fork 2: the histogram source.**

| Source | Cost at 1440p | Quality | Verdict |
|---|---|---|---|
| Full resolution | 0.17 ms at 1080p on an RTX 2080 [M] (T4) → ≈ 0.36–0.6 ms at 1440p on the reference laptop part [E] | — | Rejected on cost. |
| L1 (half resolution) | ≈ 0.09–0.15 ms [E] | Tardif: "half size or less" | Acceptable. |
| **L2 (quarter resolution)** of the bloom chain | ≈ 0.02–0.04 ms [E]: 1/16 of the pixels | 640×360 samples at 1440p feeding 256 bins | **Chosen**. It reuses bytes the bloom chain already produced. When `Bloom` is absent, `post_down` builds L1 and L2 alone. |

**Is the Karis-averaged L2 biased? Yes, and intentionally.**
- The `1/(1+luma)` weight applies on the first downsample only.
- It suppresses a highlight smaller than its footprint, which lowers the metered mean when bright
  sources are small.
- A plain box average would instead spread one bright pixel over its whole L2 texel, which
  inflates its share of the L2 histogram up to 16× (the decimation).
- The high-percentile cut would discard such a small source at full resolution anyway.
- So the Karis weighting keeps L2's metering closer to what full-resolution percentile metering
  would do [E].
- PX3's step-response fixture includes a small bright source, so the effect is measured, not
  assumed.

**Arming `Bloom` does not change exposure.** `post_down` is the same pass with the same weights
whether or not `Bloom` is present; bloom only adds L3..L6. PX3 pins this: the adapted exposure is
bit-identical with and without `Bloom`.

**The algorithm.**
- 256 bins over the derived log2 range (§5.4), of absolute luminance (§5.3 E4).
- Percentile filtering, with UE's low/high percentages (T4).
- Asymmetric adaptation `L += (L_t − L)(1 − e^{−dt·τ})`, with separate τ for brightening and
  darkening (Tardif's τ = 1.1 as the single default; T4).
- An EV clamp `[min_ev, max_ev]`, and an optional centre-weighted metering mask.
- **On a camera cut** (§3.3) the adaptation snaps: α = 1.
- A 16×16 group builds the histogram with groupshared `InterlockedAdd`. The percentile scan is one
  256-thread group with an LDS prefix sum.
- Only BASIC subgroup support is needed, which the tree requires (RESEARCH §0.4).
- The adapted value is guarded (§5.2) before it is stored.

**Fixing the formulas.** The formulas are fixed by host tests, not quoted, because the Frostbite
notes could not be fetched (RESEARCH §7):
- `EV100 = log2(N² / t) − log2(S / 100)`
- `exposure = 1 / (1.2 · 2^EV100)` (T3, Bevy after Filament)
- the luminance → EV100 conversion with K = 12.5. Its value is standard practice but was **not
  verified this session**; the host test pins whatever value the implementer confirms against a
  primary source.

**Local exposure (T5)** is not in the ladder:
- There is no measured cost.
- Its failure modes (halos, gradient reversals) are documented.
- The published cost is an opinion (T5).

It becomes a candidate only after PX3 ships and a scene shows the need.

### 5.6 Tonemapping and grading — analytic in `post_final`; a LUT baked per frame when needed

**The two ways a curve is applied:**
- **Analytic.** ACES Hill, Khronos Neutral and Reinhard-Jodie are computed directly in
  `post_final`, bit-for-bit what the producers computed before PX1. That is what keeps R4a's
  transition pins valid.
- **LUT.** Chosen when `ColorGrading` is present or the curve is AgX or Tony. `post_lut_bake`
  writes one RGBA16F LUT combining grade and curve; `post_final` samples it once, trilinear.

**The LUT's shape follows the curve:**

| Curve | LUT | Shaper | Why |
|---|---|---|---|
| Analytic curves and AgX | 32³ | log2 | Bevy's AgX LUT is 32³ (T15). |
| **Tony McMapface** | **48³** | **`x/(x+1)`** | This is Tony's own shape (`tony_mc_mapface.hlsl`, T15 [F]). With identity grade, every baked texel centre coincides with a Tony texel centre, so the bake copies texels and adds **no second resampling**. A 32³ log2 re-bake would resample twice, and pass 1 had no gate on it. |

**LUT options:**

| Option | Cost | Verdict |
|---|---|---|
| **Bake every frame when armed** (32,768 or 110,592 threads) | ≈ 0.01 / 0.02 ms [E] | **Chosen.** No dirty tracking. HDRP bakes at step 7 of 16 every frame (T16). |
| Bake only on change | ~0 | Rejected: needs a dirty protocol for a 0.01 ms saving. |
| Artist-authored LDR LUT applied after tonemap | ~0 | Rejected as the primary path. UE marks LDR LUTs HDR-incompatible (T16), and it cannot express the HDR output of PX13. |

**LUT sources, in-house (R5):**
- **AgX.** The inset matrix and the log2 allocation [−12.47393, +4.026069] EV are published in
  Sobotka's `config.ocio` (T15 [F]). The contrast curve, however, is a baked 1D LUT
  (`AgX_Default_Contrast.spi1d`), not a formula, and no licence statement was found in that
  repository. So AgX ships at PX5 **only after its licence is confirmed** (the owner's check, V2).
  Otherwise the curve is authored in-house as a parametric sigmoid on the same log2 range, and it
  is named "AgX-like", not AgX.
- **Tony McMapface** is dual MIT / Apache-2.0 and ships as a baked 48³ LUT; its generator is
  unpublished (T15). An offline converter in a tool crate (never the runtime) turns it into the
  engine's LUT asset: a small header plus RGBA16F texels, hash-gated like the reflections design's
  `DfgLut.bin`. `boyko_image` decodes PNG only, and the runtime gains no container parser.
- **Grading parameters** (white balance, lift/gamma/gain, saturation, contrast) are eDSL leaves
  evaluated in the bake.

**The default curve** is an owner value (§13 V2). It stays ACES Hill, today's look, until the owner
chooses.

### 5.7 Bloom — the 13-tap / tent pyramid; no X-SPD, no FFT

| Option | Quality | Cost at 1440p [E] | Verdict |
|---|---|---|---|
| **Jimenez 13-tap down / tent up**, Karis average on L1, no threshold, energy-conserving mix (T6, T9) | Temporally stable (T6) | 0.23–0.45 ms (§6.1) | **Chosen**, as the tree's own E-BLOOM chose. |
| Dual filter (T7) | Designed for mobile bandwidth | 0.23–0.36 ms (5-tap down, 8-tap up) | Rejected. It saves at most ≈ 0.09 ms at 1440p, at the cost of the stability the Karis-averaged filter was built for. |
| UE-style Gaussian (T8) | "radius × radius" cost | higher | Rejected. |
| FFT convolution (T8) | cinematic | UE: "high-end hardware"; needs an FFT pipeline | Rejected (R2). |

**Details.**
- Six levels, L1 (half resolution) to L6.
- **A Bevy-style `max_mip_dimension` cap is not adopted.** Pass 1 said the cap makes the look
  depend on resolution, and that was backwards. Bevy sizes the first mip at
  `max_mip_dimension / viewport.y` of the viewport and derives the mip count from
  `max_mip_dimension` (T9 [F]), so its footprint is a fixed fraction of the screen. The decision
  stands on a different ground: at 4K a 512-px first level is a 2160 / 512 ≈ 4.2× first
  decimation. That undersamples small highlights, which the 13-tap filter and the Karis average
  assume arrive in 2× steps.
- The final half → full upsample is a tent inside `post_final` (4 bilinear taps of L1), so no
  full-resolution bloom target exists.
- Lens dirt is one bindless fetch multiplied into the bloom term.
- Screen-space lens flare (T10) is PX16: quarter resolution from L2, composited in `post_final`,
  ≈ 0.03 ms at 1440p [E].

### 5.8 Lens, film and output effects — where grain and dither run

Vignette, chromatic aberration (3 radially offset scene taps), grain and dither are mode words,
wave-uniform branches as in `taa_resolve`'s T2 words.

**Grain runs after every spatial pass, as HDRP and FSR 1 require.**
- HDRP runs FXAA at step 14 and grain and dither at steps 15 and 16 (T30).
- AMD's FSR 1 guidance: passes "that introduce noise … should be rendered after upscaling"
  (T31 [F]).
- Grain is large-amplitude noise. Fed into FXAA or SMAA, grain above the luma threshold is
  detected as edges and smeared into blobs, and SMAA's blend work grows with the edge count.

So grain has two sites, and exactly one is armed:
- in `post_final` when nothing spatial follows (TAA, SSAA, AA off);
- in `present_blit` when FXAA, SMAA or EASU follows.

`present_blit` already reads every pixel once, so moving grain there costs ALU only. The grain
leaf is one eDSL body spliced into both shaders.

**Dither stays at `post_final`'s 8-bit write in every mode**, a deliberate deviation from HDRP's
order. The reasons:
- **Dither works only at the quantisation of higher-precision data.** `display` is 8-bit, so
  `post_final`'s write is the only such point. Dither added after it is noise on already-banded
  values, and it removes no banding. HDRP can dither last because its intermediate stays float
  until the final pass.
- **SMAA cannot see it.** Triangular ±1 LSB dither is 2/255 peak-to-peak. SMAA's luma edge
  threshold is 0.1, about 25.5 LSB (`smaa_common.hlsli:41`).
- **FXAA sees it only in deep shadows, and the effect is bounded.**
  - FXAA's edge test is `luma_range < max(EDGE_THRESHOLD_MIN, luma_max · EDGE_THRESHOLD_MAX)`,
    with a floor of 0.0156 (`fxaa.fs.hlsl:48-49`) on a **square-root** luma (`rgb2luma`).
  - Since Δ√y ≈ Δy / (2√y) with Δy = 2/255, dither crosses that floor only where the Rec.601 luma
    of the display value is below about 0.0625, i.e. 16/255 [derived].
  - There FXAA's blend partially smooths the dither.
  - PX6 measures this on an FXAA dark-gradient fixture.

**What would reopen the dither choice.** If dither under FXAA recovers less than half of its
banding improvement on the TAA path, `display` moves to `A2B10G10R10_UNORM`. That is the same
4 B/px, and it is the format PX13 already uses. Dither then joins grain in `present_blit`, at the
8-bit swapchain quantisation. The price is a storage-format probe, because the mandatory
storage-image support for that format was not confirmed this pass.

| Aspect | Decision |
|---|---|
| Grain and dither noise | Keyed on the frame serial (P23). |
| **Dither default** | **On** once PX6 lands. It removes an 8-bit quantisation defect in sky and fog gradients, which is a correctness fix, not a look. It is off at PX1 so the R4a transition pins can hold. |
| Other lens defaults | Off. |

### 5.9 Depth of field — half-resolution scatter-as-gather

| Option | Quality | Cost at 1440p, worst case (every tile out of focus) [E] | Verdict |
|---|---|---|---|
| **Scatter-as-gather at half resolution** (CoD 2014, UE Diaphragm lineage, T11): CoC from `gViewT`; tile min/max CoC; near and far layers; ring gather; alpha-tracked composite | Correct foreground bleed over in-focus objects; bokeh shape | 0.67–1.14 ms (32 B/px DRAM + 30 fetch-equivalents per pixel, §6.1) | **Chosen** |
| Separable complex phasor (T12) | Cheap large radii, but rings with one component and needs 6+ float channels; two components cost ~8N instructions | ≈ 0.4–0.7 ms [E] | Rejected. Ringing, and no natural near-layer occlusion. |
| Full-resolution gather | Best | ≈ 4× the gather | Rejected (R2). |

**Tiles and dispatch.** In-focus tiles are classified out through indirect dispatch, so a scene
mostly in focus costs about the prefilter plus the composite: 0.47–0.60 ms at 1440p [E]. This
needs the trait's `dispatch_indirect` to work (§8 G7, RK-16). If RK-16 has not landed when PX9
starts, PX9 lands it; it is small.

**A texel-rate lever.** The gather's 48 taps read RGBA16F. If filtered 64-bit fetches run at half
rate (P29, RESEARCH §4.2), that doubles the gather's texel term. PX0's fetch-rate probe settles
it. If half rate is confirmed, the colour layers can move to the lit format with the CoC in a
separate R16F plane, halving that term.

**CoC input.** View z is `t · dot(rd, fwd)`, reconstructed from `gViewT` with the shared
`generate_ray` math. This is correct at mesh-to-SDF silhouettes, because both legs write `gViewT`
(§7).

**CoC stability under jitter (open question 2).** DOF runs after TAA, but `gViewT[fi]` is the
jittered current frame, so a per-pixel CoC flips with the jitter phase at silhouettes. The
half-resolution prefilter therefore takes each 2×2 quad's **closest** `gViewT`:
- It is near-field-conservative, as CoD-style prefilters are.
- The jitter is at most ±0.5 px, so the quad's closest depth does not change with the phase,
  except where a silhouette crosses the quad's boundary.

The gate (PX9): on a static-camera silhouette between the near and far planes, the per-pixel CoC
variance across the 8 jitter phases must fall below that of a full-resolution per-pixel CoC. The
per-pixel variant is the red.

**`Physical` mode** is thin-lens geometry in metres (§3.2). It takes its aperture from its own
fields until PU; after PU, it may take it from `Exposure::Physical`, so the look matches the
exposure.

### 5.10 Motion blur — McGuire tile reconstruction, camera-only first, free at rest

**The algorithm.** TileMax over K = 20 px tiles, then a 3×3 NeighborMax, then a gather of S
samples along the dominant velocity with McGuire's weights (T13). At S = 15 taps it costs
0.69–1.20 ms at 1440p when moving [E]. The cost is linear in S: each tap (a colour and a depth
fetch) costs 0.044–0.067 ms at 1440p [E]. PX10 sets S from PX0's calibration against its budget.

**Velocity for PX10** is reconstructed inline from `gViewT` and `MotionCam`, the same camera-only
reconstruction `taa_resolve` performs:
- No velocity image is written.
- Moving meshes and SDF bodies get no blur until PX11 (P15).

**Zero cost at rest.** With camera-only velocity, the CPU knows exactly whether any pixel can
move: the frame's `prev_view_proj` either equals `cur_view_proj` or it does not. The three motion
blur passes are declared only on frames where the camera moved, so a static camera costs **0 ms**
and is byte-identical to motion blur off. **On a camera cut** they are not declared either,
because a cut is not motion (§3.3).

**After PX11.** The gate becomes "camera moved, or any entity carries a moving-object motion
vector this frame". The CPU knows that too.

### 5.11 Sharpening — FSR 1 RCAS in `post_final`, in reversible space

**What the tree ships today is CAS, not RCAS.** The pass behind `SharpenMode::Rcas`
(`rcas.comp.hlsl`) is AMD **CAS**:
- a 3×3 neighbourhood of 9 taps;
- amplitude `sqrt(saturate(min(mn, 2 − mx) / mx))` (`:99`);
- a peak lerped from −1/8 to −1/5 (`:105`);
- it runs on RGBA8 display values.

FSR 1's **RCAS** is a different kernel (T18 [F]):
- a 5-tap cross;
- the lobe is `max(−hitMin, hitMax)`, with `hitMin = min(mn4, e) / (4 · mx4)` and
  `hitMax = (1 − max(mx4, e)) / (4 · mn4 − 4)`;
- it is limited to `FSR_RCAS_LIMIT = 0.25 − 1/16`;
- the source states "Each channel needs to be in the range [0, 1]".

Pass 1 described the fused kernel as a "5-tap cross", and that is true only of RCAS.

**Decided (D12): `post_final` fuses FSR 1 RCAS and the CAS pass is retired at PX1.**
- **Cost.** 5 taps instead of 9 is 4 fewer fetches per pixel, ≈ 0.09 ms at 1440p [E].
- **It is the kernel the mode names**, and FSR2 ends its pipeline with it (T22).
- **Its [0, 1] input requirement is met by the space it runs in.** The fused sharpen runs in the
  reversible tonemapped space `y = c / (1 + luma)` and then inverts, the pairing Karis uses for the
  resolve (T21). Raw HDR would break `1 − max` for any value above 1 and flip the lobe's sign.
- **The look changes anyway.** PX1 re-blesses every TAA pin, so the kernel change folds into the
  same declared re-bless and its diff report.

**What it saves:**
- the standalone pass: 8 B/px of DRAM plus 9 fetches, 0.20–0.32 ms at 1440p [E];
- the `taa_resolved` ring: 29.5 MB at 1440p.

**Why pass 1's gate could not work.** Pass 1 gated the fused path within 1/255 of the standalone
pass. Both sharpeners set their lobe from ratios of the neighbourhood's minimum and maximum, and a
nonlinear remap changes those ratios. Re-derived on the tree's actual CAS kernel (sharpness 0.2,
ACES by the Narkowicz fit, a 3-dark / 1-bright cross) [E]:
- a 0.3 | 0.6 display edge differs by about 0.95/255;
- a 0.1 | 0.9 edge differs by about 6/255.

The critique's example, computed on the RCAS lobe, gave 3/255. Either way, a 1/255 gate goes red on
real edges for something that is not a defect.

**The gates that replace it (PX1).** Each can both hold and fail:
1. **A declared change.** The sharpening change is part of PX1's declared TAA re-bless. Its diff
   report lists the mean and maximum per channel; the expected maximum is about 6/255 at
   high-contrast edges [E].
2. **Oracle agreement.** On a set of step-edge fixtures, the GPU output matches a host mirror of
   the fused math (RCAS in reversible space) within 1/255. It is red on any implementation
   error: the wrong space, a missing inverse, or a wrong tap.
3. **Acutance.** On a step fixture, the 10–90 % edge width is strictly smaller than with
   sharpening off. It is red with sharpening off.

### 5.12 Anti-aliasing overall

| Technique | Reaches SDF pixels | Cost at 1440p / 4K [E] | Verdict |
|---|---|---|---|
| **TAA (exists)**, `RasterAndBasis` | **yes**: silhouettes, interiors, shadows, specular | 0.62–1.09 / 1.40–2.46 ms | **Primary AA.** Upgraded at PX7, becomes TAAU at PX8. |
| FXAA (exists) | edges in the LDR image only | 0.20–0.37 / 0.37–0.70 ms (GTX 1080 [M], × 1.0–1.9 for the laptop part) | Kept for users without temporal AA. |
| SMAA 1x (exists) | same | 0.53–1.01 / 0.98–1.86 ms (same basis) | Kept. |
| SSAA 2× (exists) | yes | 4× the shading | Kept as the quality reference, now through `lit_native` in HDR. |
| CMAA2 | edges only | 0.27–0.51 / 0.38–0.72 ms (same basis) | **Not adopted.** PSNR 36.86 against FXAA's 36.75 (T19): a 0.11 dB gain does not pay for a fifth mode and a 3-kernel port. The base version is also not temporally stable. |
| SMAA T2x / Filmic SMAA | edges plus 2-frame | 0.9–1.05 ms at 1080p (T19) | **Not adopted.** Full TAA already exists. |
| **MSAA** | **no**: the SDF march and the deferred resolve are single-sample compute (T20) | G-buffer memory ×4, 80 MB at 1080p for 4× [M] | **Excluded** (unchanged). |

**Why MSAA stays excluded.** Its memory cost buys edge AA on the mesh leg only. UE offers MSAA
only in its forward renderer (T20).

**The Forward exception.** Forward is the one path whose raster could take MSAA. Its SDF forward
march is compute, so MSAA would leave SDF edges aliased beside anti-aliased mesh edges, which is
not a coherent AA. Forward gets FXAA and SMAA at PX6 instead, since `display` exists on every
path after PX1.

**The hybrid argument is §7(a).**

**TAA upgrades (PX7):**

- **Depth disocclusion** through `viewt[1-fi]`. It is one binding plus a cross-frame seed, and no
  new image. `DisocclusionTest::OffScreenAndDepth` and `TaaConfig::depth_tol` already exist and
  are forwarded to the UBO. Only the shader read is missing.
- **The translucent coverage mask** `taa_cov`, the transparency design's R6, when that has landed.
- **Texture mip bias** under jitter (P8; T22's `log2(r/d) − 1`, i.e. −1 at native resolution):
  - applied shader-side: VB scales its `SampleGrad` gradients by 2^bias, and the raster fragment
    shaders use `SampleBias`;
  - so no sampler is recreated when the AA mode changes live;
  - the extra texture bandwidth is measured in the VB shade zone (P24).
- **The three stale enum docs** from RESEARCH §0.6 are corrected in the same rung.

### 5.13 Temporal upscaling, the render extent, and dynamic resolution

| Option | Licence and distribution (R5) | Vulkan | Cost at 1440p Quality | Persistent memory at 1440p | Verdict |
|---|---|---|---|---|---|
| **In-house TAAU: extend `taa_resolve`** (PX8) | ours | yes | TAA-class at display resolution + reconstruction weights: ≈ 0.7–1.2 ms [E] | history as today: 59 MB; render-resolution targets shrink | **Chosen** |
| FSR 2.2.1 ported wholesale (6 stages) | MIT (T22) | the algorithm ports | 1.2 ms on an RX 6650 XT [V]; another vendor's GPU, **not a proxy for our cost** | 164 MB [V] | Rejected as a whole. Its parts (depth clip, locks, reactive mask) are ported as PX8 sub-steps when a fixture shows the need. |
| FSR 3.1 SDK / FSR 4 | MIT SDK without Vulkan / signed DX12 DLLs (T23) | no | — | — | **Impossible** on raw Vulkan (FSR 4); SDK not usable. |
| DLSS | proprietary; notify NVIDIA; attribution; NVIDIA GPUs only (T23) | via Streamline | no reliable figure | — | Not in the core (R5). Whether an optional external plugin is acceptable is an owner value question (§13 V4). |
| XeSS | binary, no modification (T23) | not confirmed | none found | — | Same as DLSS. |

**Why extend the house resolve rather than port FSR2 whole.**
- The house resolve already has most of FSR2's accumulate stage:
  - an exact camera reprojection;
  - variance clipping toward the centre;
  - a Catmull-Rom history;
  - luma weighting;
  - a history ring on the frame-graph discipline;
  - after PX1, the pre-exposure rescale FSR2 also applies (§5.3).
- What TAAU adds is small:
  - the decoupled render extent of PX8a (below);
  - `⌈8·ratio²⌉` jitter phases, 18 at 1.5× (T22);
  - the current sample splatted at its jittered position with a reconstruction kernel;
  - the mip bias `log2(r/d) − 1`.
- FSR2's six stages and 164 MB are the price of a general-purpose SDK. Our resolve pays only for
  what our fixtures need.

**The ratio-1 gate.** TAAU at ratio 1 must reduce to today's TAA: at ratio 1 the kernel is
evaluated at the pixel centre, so the new code path equals the old one.

**PX8a — the render extent, decoupled for every path, sub-rectangle capable.** Both TAAU and the
spatial upscaler (§5.14) need a render extent that differs from the display extent. SSAA already
decouples the two (`aa_extent` against the 2× `present_extent`), which is the precedent.
Dynamic resolution (T29), which pass 1 surveyed and then closed silently with a boot-frozen
`RenderScale`, is **decided here**:

| Option | Cost | Verdict |
|---|---|---|
| Boot-frozen scale only | none now | Rejected. Adding dynamic resolution later re-touches every producer's extent source, the allocation and the jitter-phase logic: the retrofit the critique names. |
| Reallocate on every scale change | the fence-safe rebuild `AaArm` uses, which waits the device idle | Rejected: a hitch per scale step. |
| **Targets allocated at `RenderScale::max`; a per-frame render extent `e_r ≤ e_alloc` carried in the camera UBO** | a uniform read and a UV clamp per sampled read [E ≈ 0]. The engineering cost is touching every producer's extent source once, which PX8a must do anyway to decouple render from display. | **Chosen** |

The contract:
- every producer dispatches over `e_r` and reads it from the UBO, never from the image's size;
- every sampled read of a render-extent image clamps its UV to the sub-rectangle, because a
  bilinear tap on the rectangle's edge would otherwise read stale texels beyond it;
- TAA and TAAU reproject with the current and previous frames' `e_r`;
- the jitter phase count follows the per-frame ratio.

The DRS **controller**, which adjusts the scale from the GPU zones against a frame-time target, is
a later rung, PX17. Its target is an owner value (§13 V8).

### 5.14 Spatial upscaling for configurations without TAA (PX8s)

**The gap.** §7(b)'s argument is that the SDF march costs per pixel, so rendering fewer pixels
pays back almost linearly. That argument applies most to Forward, whose `sdf_forward_march` gets
no TAA and hence no TAAU. Pass 1 gave those paths no resolution lever.

**The candidate: FSR 1** (T31 [F][V]). It is MIT-licensed and spatial: it takes "the current
anti-aliased frame" and needs no history or motion vectors. It is two passes:
- **EASU**, the edge-adaptive upscale;
- **RCAS**, the same kernel §5.11 already fuses.

AMD's guidance places it after tone mapping, "in perceptual color space", and places noise after
it. Its published ceiling at 1440p Ultra Quality (1.3× per axis) is "0.30 ms or less" on the
RX 6700 XT / RTX 3060 Ti class [V].

**The race, at 1440p display.** Let P be the part of the frame that scales with pixels: the SDF
march, forward fragment shading, and the post chain at render resolution. P is unmeasured, and
the Forward march has no zone today (§8 G3 adds one).

| Option | Pixels shaded | GPU time [E] | Verdict |
|---|---|---|---|
| Native (render scale 1) | 3.69 M | P | Default. |
| Bilinear upscale from 1.3× | 2.18 M | 0.59 P + ≈ 0.1 ms | Rejected: EASU costs ≈ 0.3 ms more and reconstructs edges; bilinear visibly softens them. |
| **EASU + RCAS from 1.3× (Ultra Quality)** | 2.18 M | 0.59 P + 0.4–0.5 ms | **Chosen when armed.** It wins when 0.41 P > 0.45 ms, i.e. P > about 1.1 ms. |
| EASU + RCAS from 1.5× (Quality) | 1.64 M | 0.44 P + 0.35–0.45 ms | The same code at another owner-chosen scale; it wins when P > about 0.8 ms. |

The EASU + RCAS cost is derived two ways:
- **From the vendor's ceiling:** "≤ 0.30 ms" [V] × the texel-rate ratio 253 / 166 between the
  RTX 3060 Ti and the laptop part (RESEARCH §4.2) ≈ 0.46 ms.
- **From this document's model:** about 12 fetches per output pixel plus 6.4 B/px for EASU, and
  5 more fetches for RCAS in `present_blit`, gives 0.38–0.47 ms.

**How large P is on an SDF frame [E].** A Forward frame with 4 edits, 48 primary steps, a 48-step
shadow march, 6 normal and 5 AO taps is about 107 folds × 4 edits × about 25 FLOP (the volumetrics
design's per-edit figure) ≈ 10.7 kFLOP per pixel. At 1440p that is 39 GFLOP, about 7–11 ms at 50 %
of the laptop part's 6.9–10.9 TFLOPS. Even a tenth of that, allowing for the mesh-depth early
kill and sky pixels, is far above the 1.1 ms break-even. A mesh-only Forward frame with cheap
materials may sit below it, and there native stays faster. So the lever is armed by scene choice,
not by default.

**The shape, when armed** (render scale < 1 and no TAA, on any path):
1. The producers and the post chain run at render extent.
2. `post_final` writes `display` at render extent, with dither and without grain.
3. FXAA or SMAA runs at render extent if armed. FSR 1 wants anti-aliased input, and PX8s's
   quality gate runs with SMAA.
4. `easu` writes `easu_out` at display extent.
5. `present_blit` applies RCAS (5 taps, fused), grain and the UI.

**Structural cost (R3).** With a render scale of 1, `easu` is not declared and `easu_out` does not
exist.

### 5.15 Specular aliasing — at the producers, split by leg, costed (PX12)

| Leg | Candidate | Paths it covers | Cost at 1440p [E] | Verdict |
|---|---|---|---|---|
| Raster mesh (`gbuffer_mrt`, `forward_opaque`) | Tokuyoshi–Kaplanyan roughness widening from hardware `ddx`/`ddy` of the normal (T24), clamped as Filament does (variance 0.15, threshold 0.05) | Deferred, Forward | ≈ 20 FLOP per pixel ≈ 75 MFLOP: < 0.03 ms | **Chosen** |
| VB mesh (`vb_shade`) | The same formula, with the normal's screen derivatives from the barycentric partials VB already computes for `SampleGrad` | VB | ≈ 40 FLOP per pixel: < 0.05 ms | **Chosen** |
| SDF (A) | Neighbour-normal variance over a 2×2 of stored octahedral normals in the resolve, depth-rejected through `gViewT` | Deferred and VB only. Forward's `sdf_forward_march` stores no normals and has no resolve. | 3 normal + 3 `gViewT` fetches per SDF pixel: ≤ 0.13 ms at full-screen coverage | Rejected, unless B fails its gate. It has no Forward coverage. It also mixes surfaces at silhouettes, which depth rejection bounds but does not remove, whereas hardware `ddx`/`ddy` stays on one primitive. |
| **SDF (B)**: footprint curvature | A render-only sibling of `sdf_normal`. The marcher knows the footprint radius `h_f = t · θ_px / 2`. Six extra field folds at `p ± h_f·e_i`, plus the hit's own `f(p)`, give the diagonal of the Hessian. Its trace is the mean curvature, and curvature × `h_f` is the normal's spread across the footprint, mapped to roughness widening. | **Every path.** The G-buffer composite and `sdf_forward_march` both compute the normal in the marcher. | +6 folds per SDF hit pixel whose footprint exceeds about 4 × GRAD_H. That is +2–10 % of the per-pixel fold count (267 at worst, `SDF-PERF-AUDIT.md` §1; 60–110 typical [E]), paid only on SDF pixels. | **Chosen.** |
| Normal-map mips (asset side) | Toksvig-style roughness from the averaged normal's length per mip, computed when textures load. **Toksvig was not fetched (RESEARCH §7), so the formula is fixed at implementation from a primary source.** | every mesh path | 0 at runtime | Chosen. |

**Notes on SDF (B):**
- **`sdf_normal` stays frozen.** It uses central differences at `GRAD_H = 0.0005`
  (`sdf_field.hlsli:42`, `:232`) and belongs to OPT-PLAN's P4 invariant, because physics reuses it
  through the same gateway. Making B's step the normal's own would save the 6 folds but break P4.
  So B is a separate function.
- **Where B applies.** At 1440p and a 60° vertical field of view, θ_px ≈ 8.0 × 10⁻⁴ rad, so `h_f`
  exceeds `GRAD_H` beyond about 1.25 m [E]. Nearly every visible SDF surface past arm's length
  takes the branch.
- **What B underestimates.** A saddle, where κ1 ≈ −κ2, has a near-zero trace. That is a
  documented limit.
- **What B handles.** A sharp CSG crease gives a large second difference, and so strong widening,
  which is correct, because creases alias specularly.
- **Where the widened roughness goes.** It is written where each producer already writes
  roughness. Whether the SDF G-buffer composite writes a roughness channel today was not checked;
  if it does not, PX12 adds one (small).
- **When A would come back.** If B's measured cost on the golden SDF scene exceeds 0.1 ms at 1440p,
  A replaces it on Deferred and VB, and Forward keeps B.

### 5.16 SDF-edge and alpha-test aliasing

- **SDF edges:** TAA with `RasterAndBasis` (T25), then TAAU. C-AA stays parked (§1).
- **Alpha test:** the transparency design's hashed `MASKED` variant (T26), which TAA integrates.
  There is no alpha-to-coverage, because there is no MSAA.

### 5.17 HDR display output (PX13, subject to owner scope, §13 V5)

| Option | Bandwidth | Composition | Verdict |
|---|---|---|---|
| **HDR10 PQ, `A2B10G10R10_UNORM_PACK32` + `HDR10_ST2084`** | 4 B/px, the same as today | full-screen game; no alpha-blended swapchain | **Chosen**, on Microsoft's rationale for its Option 2 (T17). Those stated conditions are DXGI's. **Whether Vulkan drivers expose `HDR10_ST2084` only while Windows HDR is on is not sourced** (pass 1 asserted it). PX13's first step enumerates the surface formats on the owner's machine with Windows HDR on and off and records the result. |
| scRGB FP16 + `EXTENDED_SRGB_LINEAR` | 8 B/px ("doubles GPU bandwidth and memory", T17) | works everywhere, including windowed | Fallback only if a driver does not expose HDR10. |

**What it needs** (§8 G6):
- `VK_EXT_swapchain_colorspace` and `VK_EXT_hdr_metadata`;
- SDR white through `QueryDisplayConfig` / `DISPLAYCONFIG_SDR_WHITE_LEVEL`, a user32 FFI;
- **display peak luminance from `IDXGIOutput6::GetDesc1` by default, with a user override.** That
  is decided here, because it is the standard technical answer. A DXGI COM call into an OS library
  is not a third-party dependency. Only whether a calibration *screen* is in scope goes to the
  owner (V5);
- polling `IDXGIFactory1::IsCurrent`, because Win32 has no change event (P6).

**How the image changes.**
- The LUT bake takes the output transform (sRGB/γ2.2, or PQ at N nits peak) as a parameter:
  "grade once, output many" (T16).
- `post_final` writes PQ directly under `-D OUTPUT_PQ`.
- The UI in `present_blit` is scaled by `SdrWhiteLevel / 80` and PQ-encoded under the same
  define (P6).

---

## 6. Cost model

### 6.1 The basis, re-based on the reference GPU

**The reference GPU is the RTX 3060 Laptop (6 GB)**, per
`OPTIMIZATION-PLAN-RENDER.md:44` ("GPU oracle = RTX 3060 Laptop (6 GB)"). Pass 1 used the desktop
RTX 3060's recalled texel rate, and that was wrong. RESEARCH §4.2 carries the fetched figures.

**DRAM:**
- 250 GB/s effective, TAA-PLAN's assumption, not measured.
- That is 74 % of the laptop part's 336 GB/s. On its 288 GB/s variant it is 87 %, which is
  optimistic: there every DRAM floor is ×1.17.
- ms per B/px: 0.0083 (1080p), 0.0147 (1440p), 0.0332 (4K).

**Texel rate:**
- 120 TMUs at a sustained 1.39 GHz gives 166 GT/s (the fetched table's figure at the top base
  clock). Boost clocks span 154–204 GT/s across the laptop part's power configurations.
- ms per fetch per pixel: 0.0125 (1080p), 0.0222 (1440p), 0.050 (4K).

**Per-format assumptions (PLAUSIBLE, unmeasured; PX0's probe settles them):**
- a filtered 64-bit fetch (RGBA16F bilinear) counts as 2;
- an unfiltered `Load` counts as 1 at either width;
- a 3D trilinear fetch counts as 2, and 4 if it is 64-bit.

**A pass's estimate is `[max(DRAM, texel), DRAM + texel]`, plus 3–5 µs per dispatch.** The lower
end assumes the two streams overlap fully and the upper end assumes they do not overlap at all.
Pass 1 costed most passes on DRAM alone and so missed their texel terms.

| Pass | DRAM B/px | Fetch-eq/px | 1080p | 1440p | 4K | Notes |
|---|---|---|---|---|---|---|
| `post_final`, R4a minimum (`lit` → `display`) | 8 | 1 | 0.07–0.08 | 0.12–0.14 | 0.27–0.32 | |
| `post_final`, default look without the sharpen (history + bloom tent + LUT) | 13 | 9 | 0.11–0.22 | 0.20–0.39 | 0.45–0.88 | tent = 4 bilinear of L1; LUT trilinear 3D RGBA16F = 4 |
| `post_final`, with the sharpen fused | 13 | 13 | 0.16–0.27 | 0.29–0.48 | 0.65–1.08 | +4 Loads (the centre is shared). An LDS tile would bring the cross to about 1.6 Loads per pixel. |
| `post_final`, PX1 under TAA (sharpen fused, no bloom or LUT) | 12 | 5 | 0.10–0.16 | 0.18–0.29 | 0.40–0.65 | |
| `taa_resolve` (HDR; history only; + previous depth at PX7) | ≈ 32 | 28 | 0.35–0.62 | 0.62–1.09 | 1.40–2.46 | 16 history + 9 `lit` + `viewt` + confidence + previous `viewt` Loads. TAA-PLAN's own 0.53–0.80 ms at 1440p sits inside. |
| Sharpen today (CAS, standalone; retired at PX1) | 8 | 9 | 0.11–0.18 | 0.20–0.32 | 0.45–0.72 | |
| Bloom, 6 levels, reading the TAA history | ≈ 13.7 | ≈ 8.9 | 0.14–0.28 | 0.23–0.45 | 0.49–0.95 | L1 down = 13 bilinear RGBA16F / 4 = 6.5; L2..L6 = 1.1; up-chain = 1.3; ~11 dispatches |
| Bloom reading `lit` (TAA off) | ≈ 9.7 | ≈ 5.7 | 0.11–0.20 | 0.17–0.32 | 0.35–0.65 | |
| `post_down` L1, L2 when bloom is off | 5–9 | 4–7 | 0.06–0.17 | 0.10–0.30 | 0.21–0.67 | |
| Exposure histogram (L2) + adapt | — | — | 0.01–0.02 | 0.02–0.04 | 0.05–0.09 | Tardif 0.17 ms full 1080p on an RTX 2080 [M] × 1/16 of the pixels × 1.2–2 for the laptop part [E] |
| LUT bake 32³ (48³ for Tony) | — | — | 0.01 (0.02) | 0.01 (0.02) | 0.01 (0.02) | 32,768 (110,592) threads [E] |
| DOF, worst (every tile out of focus) | 32 | 30 | 0.38–0.64 | 0.67–1.14 | 1.50–2.56 | 48 bilinear RGBA16F taps at half resolution = 24; prefilter 2; composite 4 |
| DOF, mostly in focus | ≈ 32 | 6 | 0.27–0.34 | 0.47–0.60 | 1.06–1.36 | tiles classified out |
| Motion blur, camera moving (S = 15) | 12 | 30–45 | 0.39–0.67 | 0.69–1.20 | 1.55–2.70 | per tap: colour (a Load = 1, or bilinear RGBA16F = 2) + depth (1) |
| Motion blur, camera static or cut | — | — | **0** | **0** | **0** | not declared (§5.10) |
| EASU + RCAS (PX8s, 1.3×) | 6.4 | 17 | 0.22–0.27 | 0.38–0.47 | 0.85–1.06 | about 12 fetches for EASU, 5 for RCAS in `present_blit` |
| Lens flare (PX16) | < 1 | — | ≈ 0.02 | ≈ 0.03 | ≈ 0.05 | quarter resolution [E] |
| FXAA (exists) | — | — | 0.11–0.21 | 0.20–0.37 | 0.37–0.70 | GTX 1080 [M] × 1.0–1.9, the upper end being the texel-rate ratio between the GTX 1080 and the laptop part |
| SMAA 1x (exists) | — | — | 0.30–0.57 | 0.53–1.01 | 0.98–1.86 | same basis |

**PX0's calibration replaces every row.** PX0 brackets the existing passes. It also adds a
bench-only fetch-rate probe: the same gather kernel over an RGBA8 and an RGBA16F image, filtered
and unfiltered. From the probe and the TAA zone, the per-fetch cost is calibrated, and every
texel-bound perf gate after PX0 is set to the calibrated prediction × 1.2.

### 6.2 Stacks

Sums of §6.1 [E]:

| Stack | 1080p | 1440p | 4K |
|---|---|---|---|
| Today with the sharpen armed: TAA + its `aa_out` write + CAS | 0.49–0.83 | 0.88–1.47 | 1.98–3.31 |
| PX1 under TAA (R4a + fused RCAS) | 0.45–0.78 | 0.80–1.38 | 1.80–3.11 (**≈ 0.1–0.2 ms saved**) |
| **Default look**: TAA, auto-exposure, bloom, grading LUT, vignette, grain, dither (PX1–PX6), sharpen off as today | **0.62–1.15** | **1.08–1.98** | **2.40–4.39** |
| Cinematic: + DOF (worst) + motion blur (moving) | 1.39–2.46 | 2.44–4.32 | 5.45–9.65 |
| Spatial-only: FXAA + auto-exposure + bloom (reading `lit`) + grading + lens | 0.35–0.63 | 0.60–1.07 | 1.23–2.20 |

Pass 1 compared the default stack with FSR 2.2.1 Quality on an RX 6650 XT and concluded it sat
"at or below a vendor temporal upscaler". That comparison set a native-resolution chain against an
upscaler that shades 0.44× the pixels, on another vendor's GPU. It established nothing, so it is
withdrawn.

### 6.3 CPU and memory

| Item | Per-frame cost | Basis |
|---|---|---|
| `resolve_post_policy` | < 1 µs | one entity's ≤ 7 components, 2 resources, and a presence scan over ≤ 16 cameras [E] |
| `ResolvedPost` → UBO | ≈ 256 B copy, < 0.1 µs | [E] |
| Exposure readback | a 4 B read from mapped memory after the frame-in-flight fence, < 0.1 µs | [E] |
| Declare + compile, +6 to 14 passes | ≈ +1–5 µs | [E]. **No in-tree measurement of per-pass compile cost exists.** PX0 brackets `declare_frame_graph` with the CPU profiler. |
| `PostArm` | O(cameras), every frame, inside the policy system | [E] |
| Heap allocation | **0 per frame** | fixed-size component reads; the graph's `reset` retains capacity (`frame_driver.rs:896`) |

Memory is in §4. Net VRAM **falls** by 14.8–44.3 MB at 1440p under TAA at PX1. The default look
adds 5.2 MB.

### 6.4 Scaling

| Driver | How post cost scales |
|---|---|
| Resolution | Linear in display pixels for everything after the temporal resolve. Under TAAU or EASU, the producers scale with render pixels. |
| Lights, objects, SDF edits, mesh triangles | **None**: every pass reads only screen images. Specular AA (B) is the one exception: +6 folds per SDF hit pixel, i.e. linear in edits like the march itself. |
| Scene content | DOF with the out-of-focus fraction (0.47–1.14 ms at 1440p); motion blur with camera motion (0, or 0.69–1.20 ms). |
| Exposure | Pixels / 16. |

---

## 7. Why this wins for the HYBRID engine (owner rule 2)

**The chain is leg-agnostic by construction.** Every post pass reads `lit`, the TAA history and
`gViewT`, and both legs (mesh raster and SDF march, alone or together) write exactly those images.
There is no candidate pair "SDF-native post versus classical post" to race. The per-technique
choices in §5 are races on per-pixel bytes and fetches, decided with §6's numbers.

The hybrid does decide six things:

**(a) Which AA.** TAA with `RasterAndBasis` is the only candidate that reaches every aliasing
source of both legs through one mechanism, at 0.62–1.09 ms at 1440p [E]:
- SDF silhouettes;
- SDF interiors (shading, soft-shadow and AO noise);
- mesh edges;
- specular.

The alternatives each cover part of this:
- **MSAA** reaches **zero** SDF pixels at 4× G-buffer memory (T20).
- **Analytic SDF coverage (C-AA)** reaches only SDF silhouettes, at 4 field taps per hit pixel,
  and is parked.
- **FXAA and SMAA** reach LDR edges on both legs but not interior shading noise.

The measured record in the tree: with `RasterOnly`, TAA changed 0 of 810,000 SDF pixels (T25).
That is exactly why the default became `RasterAndBasis`.

**(b) Why rendering fewer pixels is a performance rung, not polish.**
- **The SDF marcher's cost is per pixel.** It is "one compute thread per pixel", with up to
  ~4,300 primitive evaluations per fully lit pixel at 16 edits (`SDF-PERF-AUDIT.md` §1). Rendering
  at 1.5× Quality marches 0.44× the pixels, so the SDF leg's cost falls about 2.25×.
- **Only part of the mesh leg's cost is per pixel.** Vertex, culling and raster-setup work do not
  shrink with resolution.

So an SDF-heavy frame gains more than a classical frame does. TAAU (PX8) delivers this on
Deferred and VB, and EASU (PX8s) on every configuration without TAA, Forward included. Both perf
gates measure it on a golden scene with an SDF leg.

**(c) Depth at mesh-to-SDF silhouettes.** `gViewT` holds `t_mesh` or the SDF `t` per pixel, so
CoC (DOF) and camera velocity (motion blur, TAA) are correct across the hybrid boundary without a
merged depth buffer. The Forward path lacks this for SDF pixels (RK-12), which is why DOF and
motion blur run on Deferred and VB only.

**(d) Specular AA must split by leg** (§5.15). The classical derivative method has no input on the
SDF leg. The SDF branch is SDF-native: the marcher knows its footprint and measures the field's
curvature across it. That one mechanism covers every path, with no silhouette mixing.

**(e) Motion vectors must split by leg** (PX11):
- meshes get a raster MRT;
- SDF moving bodies need a per-instance transform delta;
- animated SDF edits have no motion vector at all, and camera-only reprojection plus the variance
  clip covers them, as TAA's measured "no ghosting" result showed (the `MvSource::PerObject` doc).

**(f) The shared radiance scale.** Mesh and SDF producers apply the same pre-exposure as their
final multiply. The temporal contract (§5.3) therefore holds for both legs with one ratio.

---

## 8. RHI and framegraph gaps

Checked against `boyko_rhi_vulkan` and `boyko_rhi` at `6394bc5e` (RESEARCH §0.4).

| # | Gap | Needed by | Size |
|---|---|---|---|
| G1 | `DeviceCaps::lit_hdr_format_ok` (RK-14): B10G11R11 `STORAGE_IMAGE` + `COLOR_ATTACHMENT` | PX1, and the bloom chain, `post_hdr_*` and `lit_native` formats | S. Specified in R4a. |
| G2 | A **3D storage image** usage path (RGBA16F 3D, `STORAGE \| SAMPLED`) | PX5 LUT bake | S. `enums.rs` already anticipates "the D3 storage image of a later rung"; the brick atlas proves 3D image creation. |
| G3 | A `POST` GPU zone family (`ZONE_BASE_POST = 4 × ZONE_FAMILY_WIDTH`; `ZONE_ID_SPAN` grows by one family); a zone on Forward's `sdf_forward_march`, which has none (the Deferred fine marcher has `ZONE_SV0_MARCHER`); a CPU zone on `declare_frame_graph`; the bench-only fetch-rate probe (§6.1) | PX0 | S |
| G4 | Framegraph appends: `display`, `lit_native`, `post_chain` (mipped), `post_lut`, `dof_*`, `post_hdr_*`, `mb_tiles`, `easu_out`, the exposure buffers. `aa_out` moves into the graph at PX6. | PX1–PX10, PX8s | M. The Deferred fixed ResId block (`FRAMEGRAPH_IMAGE_COUNT`, `graph_bridge.rs:765`/`:770`) and the private Forward and VB spaces. |
| G5 | Texture mip bias as a shader-side uniform | PX7, PX8 | S. No RHI change; one float in a per-frame uniform the sampling shaders already bind, chosen at implementation. TAA-PLAN's constraints table recorded the camera UBO tail as occupied; this pass did not re-check it. |
| G6 | `VK_EXT_swapchain_colorspace` (instance), `VK_EXT_hdr_metadata` (device), `QueryDisplayConfig` FFI, `IDXGIOutput6` COM FFI | PX13 | M |
| **G7** | **`RhiCommandEncoder::dispatch_indirect` is a silent no-op** (`crates/boyko_rhi/src/encoder.rs:408`, a `#[cold]` default body). The Vulkan backend does not override it; particles call the loaded `fns.cmd_dispatch_indirect` directly. | PX9 tile classification; every §2.3 early-out | S. The sibling reflections update books the override as **RK-16** (its decision U7). PX9 depends on it, or lands it. |
| G8 | The render-extent sub-rectangle contract (§5.13): the extent in the camera UBO, UV clamps on render-extent reads | PX8a | M. Not an RHI change; it touches every producer's extent source once. |
| — | **Available, not a gap:** specialization constants. `rhi_impl/device.rs:821` builds a `VkSpecializationInfo` at pipeline creation, and the `spec_constant_smoke` feature smoke-tests it. O6's option of stripping `post_final`'s unused arms per `PostArm` is therefore open (§9.2). | PX1 measurement | — |
| — | **Not needed:** MSAA sample counts, async compute, timeline semaphores, `synchronization2`, `DrawIndirectCount`, subgroup ARITHMETIC, `shaderFloat16`, `shaderStorageImageWriteWithoutFormat` | — | The histogram uses groupshared atomics; the percentile scan uses an LDS prefix; TAAU runs FP32. Async compute would hide post work as UE hides ~0.5 ms of TSR (T23). It stays with OPT-PLAN's P13. |

---

## 9. Shader eDSL surface and variant manifest

### 9.1 The eDSL (owner rule 6)

**A new `boyko_shaderdsl::post` module**, instantiated two ways:
- over the Eval side, as the host oracle that replaces `goldens.rs` `tonemap_and_oetf`'s hand
  mirror;
- over `Emit`, spliced between `// === GENERATED post_* BEGIN/END ===` sentinels.

**The new scalar operations `log2`, `exp2` and `pow` go on the existing `Cf` axis.** Pass 1
proposed a separate `PostScalar` trait instead. The tree already made this choice for
transcendentals, and stated why. The doc on `Cf::sin` in `cf.rs` says they live on the
control-flow axis, "whose Eval instantiation is a codegen-only ZST no physics-reachable code
calls", rather than on `FieldScalar`, "whose `f32` impl IS the physics leaf". `InterpBackend`
(`interp.rs`, gated on `feature = "emit"`) is the same firewall. Concretely:
- **The Eval arms** go on `EvalCf` with the shim `Cf::sin` uses: `core::intrinsics::{log2f32,
  exp2f32, powf32}` under the `nightly` feature, and `f32::{log2, exp2, powf}` (which link `std`)
  otherwise. Stable `core` has none of them, which is exactly why pass 1's claim that "the physics
  `no_std` build never sees `PostScalar`" needed a gate it did not state.
- **The Emit arms** are three intrinsic nodes, `Node::Log2`, `Node::Exp2` and `Node::Pow`. They
  mirror how `Node::Sin` serves both `Cf` and `InterpBackend`.
- **`pow` is an intrinsic node, not `exp2(y · log2 x)`.** The hand-written OETF `pow(x, 1/2.2)`
  lowers to `GLSL.std.450 Pow`. Rewriting it would change the SPIR-V, and PX2's byte-identity gate
  would then depend on how each driver lowers the two forms (O4).
- The physics-facing `FieldScalar` stays byte-frozen: the field eval, `smin`/`smax`, `combine`,
  the normal and the no-fast-math guarantee are OPT-PLAN's P4 invariant.

**Leaves** (generic over `C: Cf` where they need a transcendental; over `FieldScalar` otherwise):

| Group | Leaves |
|---|---|
| Tonemap and OETF | `aces_fitted`, `khronos_pbr_neutral`, `reinhard_jodie` (moved from `pbr_lighting.hlsli`); `oetf_gamma22` (today's OETF); `oetf_srgb_piecewise` (an option, not the default, because it would change every pin) |
| HDR boundary | `hdr_sanitize` (it needs a bitcast node for the exponent test; if the eDSL lacks one at PX2, it is added there), `pre_ratio_apply` |
| Exposure | `ev100_from_physical`, `exposure_from_ev100`, `ev100_from_luminance`, `histogram_bin`, `adapt` |
| Bloom and sharpen | `karis_weight`, `bloom13_weights`, `tent_weights`, `rcas_lobe` (FSR 1 RCAS, spliced into `post_final` and into `present_blit`) |
| Upscale | `easu_*` (PX8s) |
| LUT and grade | `lut_shaper_log2` and its inverse, `lut_shaper_tony` (`x/(x+1)`), `grade` (white balance, lift/gamma/gain, saturation, contrast), the AgX or AgX-like curve (§5.6) |
| Lens and film | `vignette`, `ca_offset`, `grain` (spliced into `post_final` and `present_blit`), `dither_tpdf` |
| DOF and motion blur | `coc_thin_lens`, McGuire's `cone` / `cylinder` / `soft_depth_compare` |
| Specular AA | `sdf_footprint_curvature` (a render-only sibling of `sdf_normal_body`, with the same field-call seam; it never replaces the frozen normal), `roughness_widen` |
| HDR output (PX13) | `st2084_encode` |

**Leaves that fetch** (bloom filters, RCAS, EASU, the gathers) use the SSAO leaf's discipline: the
arithmetic is the leaf, the fetch is a hand-written closure seam, `Eval` passes host values, and
`Emit` prints the symbol (`ssao.rs` module doc).

**Tolerance.** GPU `log2`/`exp2`/`pow` are not bit-exact against `f32` (P22), so post leaves are
compared **at 8-bit output resolution**, the precision the goldens already use. They are not
compared bit-for-bit.

The three rational tonemap curves are the exception. At PX2 they must reproduce the pre-PX2 image
byte-identically, and that is gated on the image. A SPIR-V census backs it: the spliced
`post_final`'s count of `GLSL.std.450` `Pow`, `Exp2` and `Log2` instructions must equal the
hand-written module's, in the style of `cluster_cull_spv_sync.rs`'s `NMin`/`NMax` census.

### 9.2 Variant manifest rows

These are new rows in [`../SHADER-VARIANT-MANIFEST.md`](../SHADER-VARIANT-MANIFEST.md), added by the
rung that adds each shader. `shaderStorageImageWriteWithoutFormat` is not enabled, so every HDR
**storage writer** needs one `.spv` per storage format (P21). Readers sample the image, so the
format does not matter to them. Nor does it matter to a fragment pass writing a colour attachment,
because there the format is pipeline state, not a shader qualifier.

| Shader | `-D` axis | `.spv` | Why it cannot be one module |
|---|---|---|---|
| `post_final.comp` | `OUTPUT_PQ` (PX13 only) | 1, then 2 | the output format qualifier: `rgba8` or `rgb10_a2` |
| `post_down.comp`, `bloom_down.comp`, `bloom_up.comp` | `HDR_FMT` ∈ {R11G11B10F, RGBA16F} | 2 each | RK-14 decides the storage format of `post_chain` |
| `dof_composite.comp`, `mb_gather.comp` | `HDR_FMT` | 2 each | they write `post_hdr_*` in the `lit` format |
| `ssaa_downsample.fs` (R4a's HDR body) | — | 1 | a fragment pass writing `lit_native` as a colour attachment: two pipelines (per RK-14) from one module. It is not a P21 storage writer. |
| `easu.comp` | — | 1 | writes RGBA8 `easu_out` |
| `dof_prefilter`, `dof_tiles`, `dof_gather`, `mb_tilemax`, `mb_neighbormax`, `exposure_histogram`, `exposure_adapt`, `post_lut_bake` | — | 1 each | fixed intermediate formats (RGBA16F, RG16F, buffers); not manifest rows |
| `taa_resolve.comp` | — | 1 (interface changes) | PX1 removes `gAaOut` @4 and adds `pre_ratio` to the `ResolvedTaa` UBO; PX7 adds `viewt[1-fi]` and `taa_cov`, bound-but-unread when disarmed (the R2 contract). The manifest records the binding changes. |
| `rcas.comp` | — | **retired** at PX1 | the CAS pass; RCAS is folded into `post_final` |
| `fullscreen_sample.fs` (present) | `OUTPUT_PQ` (PX13) | 1, then 2 | grain and RCAS-after-EASU are mode words; PQ is the format axis |
| `sdf_gbuffer_composite`, `sdf_forward_march` | — | unchanged count | PX12 adds the footprint-curvature branch under a mode word |

Wave-uniform mode words (the `taa_resolve` T2 idiom) carry every on/off inside `post_final`. That
keeps it one module until PX13.

**The occupancy question (O6).** One module means the PX1 minimum configuration pays the full uber
shader's register count. PX1 measures it: the minimum configuration against a build with the
unused arms compiled out. If the uber build loses more than 5 % at 1440p, the arms are stripped by
specialization constants chosen per `PostArm`, which the RHI already supports (§8). Spec constants
change no `.spv` count.

---

## 10. Rung plan

Every rung is red-first, has a golden gate and a perf gate, and ships OFF-by-default except where
it is a declared re-bless.

**Perf gates** are the §6.1 upper bound at 1440p until PX0 lands. After PX0, each texel-bound gate
is the PX0-calibrated prediction × 1.2.

| Rung | Content | Red-first | Golden | Perf (reference GPU, owner's box) |
|---|---|---|---|---|
| **PX0** Measurement | The `POST` zone family around FXAA, the SMAA passes, the SSAA downsample, TAA, the sharpen and the present; a zone on `sdf_forward_march`; a CPU zone on `declare_frame_graph`; the bench-only fetch-rate probe (RGBA8 / RGBA16F, filtered / unfiltered) | A zone-label test: `Measured` for each armed AA mode's passes and the Forward march, `NotBracketed` otherwise. Red before the brackets exist. | Every pin byte-identical | Baselines recorded at 1080p and 1440p, plus the probe's per-fetch costs, **with the owner's go-ahead before loading the machine** |
| **PX1** HDR + `post_final` | R4a (amended), plus: `tonemap` → `post_final`; TAA writes history only, **rescaled by `pre_ratio`**; RCAS fused; `hdr_sanitize` at every first HDR reader, with guarded history writes; SSAA writes `lit_native`; `aa_out` only for FXAA and SMAA; `display` single and seeded | R4a's 50× sun highlight fixture (clips today), **also under SSAA** (red with an RGBA8 SSAA output); the RK-14 fallback arm forced false; a framegraph pin showing `display`'s seeded WAR; **the NaN-injection fixture** (§5.2); **the exposure-ramp fixture** (§5.3); **the RCAS oracle and acutance fixtures** (§5.11) | R4a's **8 transition pins**, at ≤ 1/255 per channel (§5.1). **Declared re-bless** of TAA pins, with a diff report (mean and max per channel) that includes the RCAS change. | `post_final` ≤ 0.14 ms (TAA off) and ≤ 0.29 ms (TAA on, RCAS fused) at 1440p; the O6 occupancy comparison recorded |
| **PX2** eDSL `post` | `Cf::{log2, exp2, pow}` with `EvalCf` shims and intrinsic Emit nodes; the curves, OETF and `hdr_sanitize` as leaves; the splice; `goldens.rs` delegates to the Eval side | `post_edsl_sync`: re-emit ≠ committed span until spliced; the SPIR-V `Pow`/`Exp2`/`Log2` census (§9.1) | Image byte-identical to PX1 | `post_final` zone unchanged within noise |
| **PX3** Exposure | `Exposure` (Manual, Auto); pre-exposure through `ExposureReadback`; `post_down` L1, L2; histogram of absolute luminance over the derived range; adapt with the cut snap and the guard | Host tests: EV100 of f/16, 1/100 s, ISO 100 ≈ 14.64 (the leaf, for PU); the derived histogram range; the adaptation step response. GPU: a luminance step, including a small bright source, converges to 1 % in the predicted frame count; **the pre-exposure ramp** (§5.3); **histogram invariance**: forced ×4 pre-exposure gives the same adapted EV within 0.01 (red if the histogram does not divide by it); **bloom invariance**: adapted exposure bit-identical with and without `Bloom`; **camera cut**: after a cut the adapted value equals its target within one frame and the TAA reset flag is set; **a NaN in L2** leaves `exposure_state` finite | No `Exposure` ⇒ byte-identical to PX2 | Histogram + adapt ≤ 0.04 ms at 1440p |
| **PX4** Bloom + dirt | `Bloom`; the chain on `add_image_mipped`; energy-conserving mix; dirt | The 13-tap / tent weights sum to 1 (host); a flat-field energy test (mean preserved within 0.5%); a firefly fixture: frame-to-frame bloom variance with the Karis average < without (red with it disabled) | No `Bloom` ⇒ byte-identical | ≤ 0.45 ms at 1440p |
| **PX5** Grading + LUT curves | `ColorGrading`, `Tonemapping::Tony` (and `AgX` once its licence is confirmed, else AgX-like); `post_lut_bake` with the curve-shaped LUT; the engine LUT asset plus the offline converter | Identity grade + ACES through the LUT vs analytic ≤ 2/255; **the same test red on a 16³ LUT**, proving the gate can fail; **identity grade + Tony: every baked texel equals the converted Tony texel** (red with the log2 shaper swapped in); the LUT asset byte hash | No grade and an analytic curve ⇒ byte-identical | Bake ≤ 0.02 ms; `post_final` + ≤ 0.09 ms |
| **PX6** Lens + dither + Forward spatial AA | Vignette, CA, grain and dither in `post_final`; grain's `present_blit` site; `aa_out` into the graph; the `post_process_aa_supported` Forward term with its `targets.rs` path term | Dither: the longest equal-value run on an 8-bit gradient fixture > N without dither, ≤ N with; grain: two runs with the same serial give the same bytes; **grain placement**: with SMAA armed and strong grain on a flat field, SMAA's edge texture is empty (red if grain runs before SMAA); **dither under FXAA**: the dark-gradient run-length metric recorded against the TAA path (the §5.8 trigger) | Per-effect byte-identical when off; new Forward FXAA and SMAA pins; the dither default re-blesses (declared) | `post_final` + ≤ 0.03 ms (+0.09 ms with CA) |
| **PX7** TAA upgrades | Depth disocclusion through `viewt[1-fi]`; `taa_cov` (when transparency R6 exists); mip bias; the stale docs | A sliding-occluder fixture: ghost-trail pixel count red on `OffScreenOnly`, green on depth; a high-frequency texture fixture: converged Laplacian energy higher with bias | Non-TAA modes byte-identical; TAA pins re-bless (declared) | TAA + ≤ 0.05 ms at 1440p; the VB shade zone delta recorded |
| **PX8a** Render extent | `RenderScale` with the sub-rectangle contract (§5.13) on every path | Scale 1 ⇒ byte-identical; **a sub-rectangle fixture**: rendering at 0.75 inside max-size targets equals rendering at 0.75 in exact-size targets within 1/255 (red if any consumer samples past the rectangle) | `RenderScale == 1` ⇒ byte-identical | UV clamps within noise at scale 1 |
| **PX8** TAAU | `⌈8·ratio²⌉` jitter; jittered-sample reconstruction; mip bias `log2(r/d) − 1`; post at display resolution | Ratio 1 ⇒ byte-identical to TAA; static-scene convergence: PSNR(TAAU Quality, 16-frame SSAA reference) exceeds PSNR(bilinear upscale) by a margin fixed from the first run, red on a bilinear resolve | `RenderScale == 1` ⇒ byte-identical | Whole-frame GPU time at 1440p Quality vs native, on a golden scene with **both legs**; the SDF marcher zone scales ≈ 0.44× (§7b) |
| **PX8s** Spatial upscale | EASU + RCAS for a render scale < 1 without TAA, on every path (§5.14) | Scale 1 ⇒ `easu` not declared (a framegraph pin); PSNR(EASU UQ vs native) exceeds PSNR(bilinear vs native) by a margin fixed from the first run, red on a bilinear kernel | Scale 1 ⇒ byte-identical | Whole-frame GPU time at 1440p UQ vs native on the Forward golden scene with an SDF leg, recorded against §5.14's break-even; EASU + RCAS ≤ 0.47 ms |
| **PX9** DOF | `DepthOfField`; CoC from `gViewT` with quad-closest depth; tiles + indirect (RK-16); near/far gather; composite | Host CoC oracle (thin lens); in-focus plane ⇒ byte-identical to DOF off; a foreground-over-focus fixture, red with a naive far-only gather; **the CoC jitter-stability fixture** (§5.9) | No DOF ⇒ byte-identical | Worst case ≤ 1.14 ms at 1440p before PX0, then calibrated |
| **PX10** Motion blur (camera) | `MotionBlur`; TileMax, NeighborMax, gather; inline velocity; declared only when the camera moved and not on a cut | Static camera and a cut frame ⇒ pass not declared (framegraph pins); a yaw fixture: blur length = analytic px velocity × shutter within 1 px | No `MotionBlur` / static ⇒ byte-identical | ≤ 1.20 ms at 1440p moving at S = 15 before PX0; after PX0, S is set so the calibrated cost meets the owner's budget (V8); 0 static |
| **PX11** Per-object motion vectors | Un-wall the mesh MRT from `hwrt` (the refactor the `MvSource::PerObject` doc names: `MeshRenderScratch::prev_ring` and the `hwrt`-forked gather); SDF moving-body motion vectors (its own design item) | A moving-mesh ghost fixture; a moving object blurring under a static camera | Absent movers ⇒ byte-identical | TAA + ≤ 0.03 ms (8.3 MB velocity read at 1080p, TAA-PLAN T5) |
| **PX12** Specular AA | §5.15 per leg; SDF (B) footprint curvature; Toksvig-style mip roughness at texture load | Highlight temporal variance on a bumpy-normal fixture, red without; **a host oracle for (B)**: on a sphere of radius R at distance t, the predicted normal spread equals `h_f / R` within 10 %, and a box edge gives the crease response (red on a wrong mapping) | Per-leg byte-identical when off | Mesh legs < 0.05 ms; SDF (B) ≤ 0.1 ms at 1440p on the golden SDF scene, else A on Deferred and VB (§5.15) |
| PX13 HDR output | §5.17 | PQ encode host oracle; UI white scale; the surface-format enumeration with Windows HDR on and off recorded | SDR path byte-identical | Swapchain bandwidth unchanged (PQ) |
| PX14 Post volumes | `PostVolume { shape, priority, blend_distance }` + override components; a blend system into `ResolvedPost` | Blend-weight host tests | No volume ⇒ byte-identical | CPU ≤ 5 µs at 64 volumes [E] |
| PX15 Fuse into present | `post_final` inside `present_blit` when no spatial pass is armed | — | Byte-identical to the unfused path | Merge only if the measured saving is ≥ 0.1 ms at 1440p (derived 0.12 ms: one RGBA8 write + read) |
| PX16 Lens flare | Screen-space ghosts and halo from L2 | — | Off ⇒ byte-identical | ≤ 0.05 ms at 1440p |
| PX17 DRS controller | Adjust `RenderScale` inside PX8a's range from the GPU zones against a frame-time target | A step-load fixture: the frame time returns under the target within N frames | No controller ⇒ byte-identical | CPU ≤ 1 µs per frame |
| PU Physical units | Physical light, sky and emissive units; `Exposure::Physical` and presets; histogram defaults re-derived (§5.4). Shared with the volumetrics campaign. | Host tests: the Bevy presets map to the documented EV100; a sunlit golden scene at `Physical { f/16, 1/100 s, ISO 100 }` renders within the relative-scale pin's mean luminance ± 1 EV | A declared re-bless of every scene (owner scope, V9) | — |

**Order.** PX0 → PX1 → PX2 → PX3 → PX4 → PX5 → PX6 → PX7 → PX8a → PX8 → PX8s → PX9 → PX10 → PX11 →
PX12. PX13–PX17 and PU follow owner scope or measurement. The order is chosen for these reasons:

- **Measurement first**, so every later cost claim is checkable (P17), and so the texel-rate
  question is settled before any gate depends on it.
- **HDR second**, because everything depends on it, and the NaN and pre-exposure contracts come
  with it.
- **The eDSL third**, so every later leaf is born in the eDSL rather than migrated.
- **Exposure before bloom**, because pre-exposure fixes the value scale every later HDR effect is
  tuned against.
- **TAA upgrades before TAAU**, which inherits them.
- **The render extent before either upscaler**, which share it.
- **DOF and motion blur after TAAU**, so they are built once, at display resolution.

---

## 11. Risks

| # | Risk | Mitigation |
|---|---|---|
| K1 | B10G11R11's 5–6-bit mantissa bands dark ramps or the bloom chain's accumulation | Pre-exposure (§5.3) keeps values near display scale; the RK-14 RGBA16F arm exists; PX4's flat-field and PX6's gradient fixtures measure it |
| K2 | The N−2 pre-exposure lags a sudden exposure change, so for two frames `lit` may clip or lose precision | `post_final` multiplies by the exact ratio, so the image is right when values are in range; temporal readers rescale (§5.3); `hdr_sanitize` bounds values to 64,512 and replaces non-finite pixels (§5.2). Whether B10G11R11 turns an out-of-range store into Inf or the maximum finite value was not established; the sanitiser handles both. |
| K3 | HDR TAA fireflies | The luma weighting is already on by default; pre-exposure puts it on an exposed scale (P11) |
| K4 | The golden re-bless at PX1 hides a real regression | R4a's eight transition pins at ≤ 1/255 (they still catch a double tonemap by tens of LSB), and the TAA pin diff report |
| K5 | DOF and motion blur at display resolution under TAAU cost 2.25× render resolution | The §2.2 trigger (D3) re-evaluates UE's order with both numbers |
| K6 | Particles and translucents ghost under TAA (P7) | `taa_cov` (transparency R6) at PX7; per-object motion vectors at PX11 |
| K7 | Post images outside the graph repeat P19 | Every new image is graph-tracked (G4); `aa_out` joins at PX6 |
| K8 | Mip bias raises VB texture bandwidth (P24) | Measured in the VB shade zone at PX7; the bias is a uniform, so it can be tuned without a new variant |
| K9 | Auto-exposure flicker from histogram noise | Percentile filtering + asymmetric adaptation (T4); the PX3 step-response fixture |
| K10 | HDR output cannot be tested in CI (it needs an HDR display) | The SDR path stays byte-gated; the PQ path is a host oracle plus an owner visual check |
| K11 | Arming by union over cameras keeps unused images alive (e.g. a DOF camera never activated) | Bounded by §4, at most ≈ 44 MB at 1440p with DOF and motion blur both armed; the owner removes the component to free it |
| K12 | The AgX formulation or the Tony LUT differs from the reference look | The LUT asset hash is pinned; the owner does a visual check against Blender (AgX) and Bevy (Tony) before either becomes a default (V2) |
| K13 | The sanitiser masks a producer bug that emits NaN | The bench-only NaN counter (§5.2) lets a golden run assert zero replacements |
| K14 | A sibling campaign keeps a radiance history without the E2 rescale | §5.3's hand-off and §15; each campaign's own fixture is the exposure ramp |
| K15 | Texel-rate estimates are off by 2× (the 64-bit filter rate is unmeasured) | PX0's probe; every texel-bound gate is set from its calibration |
| K16 | `Auto` exposure darkens today's scenes by ≈ 2.3 EV [E] | `compensation_ev`; its default is an owner value (V3) |

---

## 12. Decisions taken here (technical, with the reason)

| # | Decision | Reason (section) |
|---|---|---|
| D1 | Adopt R4a/R11 as PX1, with the transition pin at ≤ 1/255 instead of a byte-equal hash | Prerequisite of every HDR effect; the hash cannot survive an HDR store (§5.1) |
| D2 | `post_final` is one fused compute uber pass writing a single, seeded `display` | One read and one write of the scene for the whole LDR tail; one queue makes the single image safe (§1, §5) |
| D3 | Temporal resolve first; DOF, motion blur, bloom and tonemap after it (HDRP / FSR2 order). **Trigger:** revisit UE's order if DOF exceeds 0.5 ms at the target resolution once TAAU exists | §2.2 |
| D4 | Post settings are camera components, resolved every frame by one cold system into `ResolvedPost`; arming is the union over cameras, recomputed every frame and compared in `sync_gbuffer`; `LightingConfig` is the fallback; a camera cut resets TAA, snaps exposure and skips motion blur | Owner rule 3; one view per frame; no hitch on camera cuts (§3) |
| D5 | Exposure is GPU-resident within the frame; pre-exposure comes from an N−2 readback into the existing `H.exposure`; **P6-S is not needed**. The image is exact within a frame by the ratio, and across frames only under the temporal contract (D22). | Zero lag; zero producer changes (§5.5) |
| D6 | The histogram reads bloom's Karis-averaged L2 and meters absolute luminance over a range derived from the EV clamp | 1/16 of the atomics of a full-resolution histogram, from bytes already produced; the Karis bias is intended (§5.4, §5.5) |
| D7 | Analytic curves stay analytic; the grade and LUT curves bake a LUT every frame when armed: 32³ log2, or 48³ `x/(x+1)` for Tony; AgX only once its licence is confirmed, else AgX-like; Tony goes through an offline converter | Rules 5 and 6; no second resampling of Tony (§5.6) |
| D8 | Bloom: 13-tap / tent, 6 levels, Karis on L1, energy-conserving; no X-SPD, no FFT, no `max_mip_dimension` cap (a 4.2× first decimation at 4K) | §5.7 |
| D9 | Dither on by default after PX6, always at `post_final`'s 8-bit write; grain after every spatial pass (`present_blit` when FXAA, SMAA or EASU runs); both keyed on the frame serial | Dither must sit at the quantisation; grain must not feed edge detection (§5.8) |
| D10 | DOF: half-resolution scatter-as-gather with indirect tile classification (RK-16) and a quad-closest CoC | §5.9 |
| D11 | Motion blur: McGuire tiles, inline camera velocity, declared only when the camera moved and never on a cut | §5.10 |
| D12 | The sharpen in `post_final` is FSR 1 RCAS (5 taps) in reversible tonemapped space; the tree's CAS pass is retired; its gates are a declared diff, oracle agreement and acutance | Cheaper than CAS; the [0, 1] requirement; a gate that can hold and fail (§5.11) |
| D13 | AA set: TAA primary; FXAA and SMAA kept; no CMAA2, no SMAA T2x, no MSAA | §5.12 |
| D14 | TAA gains depth disocclusion through `viewt[1-fi]`, `taa_cov`, and a shader-side mip bias | §5.12 |
| D15 | TAAU extends the house resolve; FSR2 parts are ported on need, never linked; no vendor SDK in the core | §5.13 |
| D16 | Specular AA at the producers, split by leg; SDF-native footprint curvature on every path; Toksvig-style mip roughness at texture load | §5.15 |
| D17 | HDR output is PQ (4 B/px), with scRGB as the fallback; peak luminance from DXGI with a user override | §5.17 |
| D18 | New post math lives in `boyko_shaderdsl::post`; `log2`/`exp2`/`pow` go on `Cf` with the `EvalCf` shim; `pow` is an intrinsic node; no new trait | The tree's own firewall for transcendentals (§9) |
| D19 | Every new post image is framegraph-tracked; `aa_out` joins at PX6 | P19 (§8 G4) |
| D20 | Measurement (PX0) is the first rung, with a fetch-rate probe | No post pass has a zone today (P17); the texel rate decides several gates (§6.1) |
| D21 | The radiance scale is relative until rung PU; `Exposure::Physical` waits for PU | The tree's suns are ≈ 2¹⁵ below physical (§5.4) |
| D22 | The temporal pre-exposure contract E1–E4, landing at PX1 | §5.3 |
| D23 | `hdr_sanitize` at every first HDR reader and every cross-frame state write | §5.2 |
| D24 | Under SSAA, the HDR box filter writes `lit_native` (lit format, single, seeded) | §4, §1 |
| D25 | The render extent is decoupled at PX8a with a sub-rectangle contract; the DRS controller is PX17 | §5.13 |
| D26 | EASU + RCAS for configurations with a render scale < 1 and no TAA (PX8s) | §5.14 |

---

## 13. Owner VALUE / SCOPE questions (and only those)

1. **HDR scene colour: when.** This is ballot 1 of R4a and R11, still open. Post cannot start
   PX3 onward without it, and particles have asked the same question. Recommended: **now, as
   PX1**. It saves about 0.1–0.2 ms on the GPU at 1440p under TAA with the sharpen armed (§6.2)
   and 15–44 MB (§4). It does re-bless every golden: 61 blessed legs at `6394bc5e` (RESEARCH T1).
2. **The default tonemap curve.** The choices are ACES Hill (today's look; hue-shifting, T14),
   Khronos Neutral, Reinhard-Jodie, AgX (**subject to a licence you confirm**; otherwise an
   in-house AgX-like curve) or Tony McMapface (T15). Recommended: keep ACES until you have seen
   them all on the golden scenes after PX5.
3. **The shipped look preset for a new camera.** Which of auto-exposure, bloom, vignette and grain
   are on by default, and what `compensation_ev` is? Note that `Auto` on today's scenes lands
   about 2.3 EV darker than exposure 1.0 [E] (§5.4). Dither is decided (D9). Recommended: all off,
   preserving the zero gate, with a named "cinematic" preset bundle offered separately.
4. **Vendor ML upscalers.** Is an optional external DLSS or XeSS plugin acceptable as a rule-5
   exception? DLSS requires notifying NVIDIA, splash attribution, and runs on NVIDIA only (T23).
   The core never links either (D15).
5. **HDR display output (PX13).** Is it in scope, and is a user calibration *screen* in scope?
   Peak luminance itself comes from DXGI with a user override (D17).
6. **Post volumes (PX14).** Should post settings blend spatially, as in UE or HDRP, or live on
   cameras only?
7. **DOF, motion blur and lens flare** (PX9, PX10, PX16). Ship them, or keep them cut as
   OPT-PLAN's polish tier?
8. **The post GPU budget** at your target resolution on the RTX 3060 Laptop, and whether dynamic
   resolution should hold a frame-time target (PX17).
   - The re-based estimates at 1440p are 1.08–1.98 ms for the default look and 2.44–4.32 ms with
     DOF and motion blur (§6.2).
   - Example budgets: ≤ 1.5 ms default and ≤ 3.0 ms cinematic. Holding the upper ends of those
     ranges within them would force the bilinear history filter, an LDS tile for RCAS, fewer
     bloom levels or fewer motion-blur taps.
   - The budget decides sample counts and the defaults of question 3. It is best fixed after
     PX0's measurement.
9. **Physical light units (rung PU).** Should lights, sky and emissive move to physical units, so
   that `Exposure::Physical` and physical presets become meaningful? It re-tunes and re-blesses
   every scene. The volumetrics design already assumes a physical sky, so the decision is shared.
   Recommended: yes, as one cross-campaign rung, after PX3.

---

## 14. Verification log

### 14.1 Pass 1

**Re-opened at `6394bc5e`:**
- `taa_config.rs`: `TaaConfig::default` and its line `:467`, `MvSource`, `DisocclusionTest`,
  `SharpenMode`.
- `aa_config.rs`: `AaMode`.
- `light.rs`: `Tonemapper`, `LightingConfig::exposure` `:564`.
- `render_path_config.rs`: `taa_supported` `:676`, `post_process_aa_supported` `:703`.
- `targets.rs`: `lit` `:127`, `viewt` `:133`, `motion_vec` `:181`, `aa_out` `:428`,
  `taa_hist` `:480`, `taa_resolved` `:515`, `GBUFFER_FORMAT` `:2279`.
- `graph_bridge.rs`: `particle_draw` `:598`, `taa_resolve` `:2527`, `FRAMEGRAPH_IMAGE_COUNT`
  `:765`/`:770`, `declare_frame_graph`.
- `frame_driver.rs:896`.
- `framegraph/graph.rs`: `add_image_mipped` `:392`, `add_buffer_seeded`.
- `framegraph/sync.rs`: `seeded_readers_at_layout` `:271`, `seeded_writer`.
- `gpu_zone.rs`: the zone families.
- `device.rs`: `find_queue_family`, `REQUIRED_SUBGROUP_OPERATIONS` `:3043`; no reference to
  `shaderStorageImageWriteWithoutFormat` or `shaderFloat16`.
- `rhi_impl/device.rs:1865`; `bindless.rs:185`.
- `surface.rs`: `pick_surface_format`.
- Shaders: `pbr_lighting.hlsli` `:202`/`:237`; `taa_resolve.comp.hlsl:104`;
  `vb_shade.comp.hlsl:367`; `gbuffer_mrt.fs.hlsl:230`.
- `goldens.rs:1917`.
- `camera.rs`: `ActiveCamera`.
- `instance_model.rs`: `PrevInstanceModelCol`.
- `boyko_shaderdsl`: `scalar.rs`, `cf.rs`, `ssao.rs`.

**Grep checks:**
- the golden legs in `goldens/PINS.toml`: 34 `sha256_software`, 34 `sha256_hwrt` of which 7 are
  `PENDING`, so 61 blessed;
- the eight tonemap producers;
- `ExtensionPoint` and `TransientImagePool` absent from `crates/`;
- no `lit_hdr_format_ok` in `crates/`.

**Re-fetched in pass 1:** FSR2 [42] (twice, for the bloom placement), HDRP [23], Microsoft HDR
[19], Tardif [8], Intel CMAA2 [33], SPD [56], CAS [32], Tony McMapface [13], Bevy tonemapping
[14][59], Bevy `Exposure` [58], and the Jimenez 2014 and s2018 pages [1][26].

### 14.2 Pass 2 (this revision)

**Re-opened at `6394bc5e`:**
- `taa_resolve.comp.hlsl`:
  - the reset comment `:410`, the history write `:519`;
  - `sample_history_catmull_rom`, `clip_toward_aabb_center`, the variance-clip block;
  - a grep for `isnan|isfinite|64.0` finds nothing.
- `rcas.comp.hlsl`: the CAS kernel, `amp` `:99`, `peak` `:105`, the 3×3 taps.
- `targets.rs`: the `aa_out` doc (SSAA's native `aa_extent`), `GBufferTargets::sync_gbuffer`'s
  per-frame `aa_arm`/`hzb_arm` predicate.
- `taa_state.rs`: `TaaState` and its reset triggers; `runner.rs`: the `mark_reset` call site.
- `light.rs`: `DirectionalLight::illuminance` `:316`, `DirectionalLight::new` `:1216`,
  `LightingConfig` `:562-568`.
- `crates/boyko_app/tests/forward_mesh.rs:125`, `crates/boyko_app/tests/csm_fit_eval.rs:74`.
- `boyko_shaderdsl`:
  - `cf.rs`: `Cf::sin`'s doc, `EvalCf::sin`'s `nightly`/`std` shim, `rsqrt`, `vec3_dot`;
  - `interp.rs`: its module doc;
  - `lib.rs` and `Cargo.toml`: the `no_std` / `nightly` / `emit` features;
  - `normal.rs`.
- `sdf_field.hlsli`: `GRAD_H` `:42`, `sdf_normal` `:232`; the shaders that call `sdf_normal`.
- `fxaa.fs.hlsl:48-49` and `rgb2luma`; `smaa_common.hlsli:41` and `SMAALumaEdgeDetectionPS`.
- `crates/boyko_rhi/src/encoder.rs:408`; the `cmd_dispatch_indirect` load and its particle
  callers.
- `gpu_zone.rs`: `ZONE_SV0_MARCHER`, `ZONE_VB_SDF_MESH`; `passes/forward.rs` has no zone.
- `rhi_impl/device.rs:821` (`VkSpecializationInfo`); `boyko_rhi_vulkan/Cargo.toml`
  `spec_constant_smoke`.
- `particle_draw.fs.hlsl`: no exposure.
- `docs/OPTIMIZATION-PLAN-RENDER.md:44`; `docs/SDF-PERF-AUDIT.md` §1;
  `docs/VB-SV0-SDF-SHADOW-PLAN.md` (the `NMax` lowering).
- `docs/render/REFLECTIONS-DESIGN-SPACE.md`: R4a's gate and its SSAA leg, R5's `lit_prev`.
- `docs/render/VOLUMETRICS-DESIGN-SPACE.md`: V3's history, V9's pre-exposure, K6.
- `docs/render/REFLECTIONS-UPDATE-2026-09-25.md`: RK-16 and U7.

**Fetched in pass 2** (RESEARCH §6, C13–C27):
- the FSR 2 shader sources (`PrepareRgb`, `ReprojectHistoryColor`, `fPreviousFramePreExposure`)
  [60];
- FSR 1's `ffx_fsr1.h` RCAS [61] and its GPUOpen page [62];
- the Wikipedia GeForce 30 series table (RTX 3060 Laptop, RTX 3060, RTX 3060 Ti) [63];
- `tony_mc_mapface.hlsl` [64];
- Bevy's bloom `mod.rs` [65];
- Sobotka's AgX repository and `config.ocio` [66].

**Could not be established:**
- the Vulkan mandatory format-support table, from which the fetched page returned nothing;
- the GTX 1080 row, whose extract was internally inconsistent, so it is used only as a range;
- any source for the Windows-HDR condition on Vulkan colour spaces.

The WebSearch budget was exhausted, so only known URLs were fetched.

---

## 15. Review log — pass 1 critique (`CHANGES_REQUESTED`, CRITICAL=1, IMPORTANT=9)

| # | Remark | Disposition | Where |
|---|---|---|---|
| C1 | Pre-exposure is not exact for anything that keeps history; D5 and the PX3 gate are wrong | **Accepted.** The temporal contract E1–E4 (rescale on read, as FSR2 does); a histogram of absolute luminance; ramp fixtures at PX1 and PX3, each red without the rescale; D5 re-worded; D22. **Hand-off:** this pass may write only these two documents. The obligations of E1/E2 go to VOLUMETRICS V3/V9, REFLECTIONS R5/R7 and the VFX/particles campaigns; §5.3 lists them. | §5.3, §5.5, §10 PX1/PX3, D5, D22 |
| W1 | PX1 removes an implicit NaN sanitiser; K2 cites a clamp that does not exist | **Accepted, and strengthened by a trace.** The NaN survives the Catmull-Rom sum (0 × NaN) and `clip_toward_aabb_center` (`NaN > 1.0` is false), so the blob grows even under a static camera. A NaN reaching `exposure_state` blacks the image permanently. The fix is `hdr_sanitize` at every first reader plus guarded state writes, costed. There is a red-first NaN fixture. K2 and RESEARCH §3 are corrected. | §5.2, K2, K13, D23, RESEARCH §0.2, §3 |
| W2 | SSAA's pre-tonemap output goes through an 8-bit image | **Accepted.** `lit_native` is named, with its format, sizing, memory row and manifest row. It is a fragment colour attachment, so P21's per-format split does not apply; that is stated in the manifest row. The 50× sun fixture runs under SSAA too. | §4, §9.2, §10 PX1, D24 |
| W3 | The fused-RCAS gate cannot pass on a real edge | **Accepted, with a correction to the premise.** The tree's standalone kernel is AMD CAS (3×3, 9 taps), not FSR 1 RCAS. Re-derived on CAS, the difference is about 0.95/255 on the critique's edge and about 6/255 on a 0.1 \| 0.9 edge, so the conclusion holds. The gates are replaced: a declared diff, oracle agreement and acutance. The fused kernel becomes FSR 1 RCAS (5 taps), whose [0, 1] requirement the reversible space meets. | §5.11, §1, D12 |
| W4 | `PostScalar` duplicates `Cf` | **Accepted.** `log2`/`exp2`/`pow` go on `Cf` with the `EvalCf` `nightly`/`std` shim `Cf::sin` uses. No new trait. | §9.1, D18 |
| W5 | The physical-camera model is ported without the tree's radiance scale | **Accepted.** The unit convention is stated: relative until rung PU (a new owner question). `Exposure::Physical` waits for PU. The histogram range is derived from the EV clamp. The ≈ 2.3 EV darkening of `Auto` on today's scenes is estimated. | §5.4, §3.2, §13 V3/V9, D21, PU |
| W6 | The cost model uses the desktop RTX 3060 | **Accepted.** Re-based on the RTX 3060 Laptop: 336 (288) GB/s and 166.4 (204.4) GT/s, fetched. The per-format texel assumptions are stated. Every pass now carries a texel term, so the stacks rise to 1.08–1.98 ms at 1440p for the default look. PX0 adds a fetch-rate probe, and the PX9/PX10 gates come from its calibration. | §6, §10, §13 V8, RESEARCH §4.2 |
| W7 | Forward paths get no resolution lever; DRS is never decided | **Accepted.** An in-house EASU + RCAS port (PX8s) is raced against native and bilinear, with a break-even of P ≈ 1.1 ms at Ultra Quality. DRS is decided: PX8a's sub-rectangle contract now, the controller at PX17. PX0 adds the missing Forward march zone that measures P. | §5.13, §5.14, §8 G3/G8, D25, D26 |
| W8 | Grain and dither run before spatial AA | **Accepted for grain; kept for dither, with evidence.** Grain moves after FXAA, SMAA and EASU into `present_blit`. Dither must stay at the 8-bit quantisation in `post_final`: after it, dither removes no banding. Its 2/255 amplitude is invisible to SMAA's 25.5-LSB threshold, and crosses FXAA's floor only below luma 16/255, which PX6 measures. The 10-bit `display` alternative is recorded with its trigger. | §5.8, §10 PX6, D9 |
| W9 | Specular AA splits by leg with no numbers and no SDF-native candidate | **Accepted.** Every candidate is costed with its coverage. SDF-native footprint curvature (B) is chosen: every path, no silhouette mixing, and `sdf_normal` stays frozen for P4. The 2×2 variant (A) is the fallback on a measured trigger. | §5.15, §10 PX12, D16 |
| O1 | `dispatch_indirect` is a silent no-op | **Adopted.** G7, tied to RK-16 (REFLECTIONS-UPDATE U7). | §2.3, §5.9, §8 G7 |
| O2 | The Tony LUT is 48³ with an `x/(x+1)` encoding; re-baking adds a second resampling with no gate | **Adopted.** The LUT's shape follows the curve (48³ `x/(x+1)` for Tony), and there is a texel-equality gate that is red with the log2 shaper. RESEARCH T15 and §7 are corrected. | §5.6, §10 PX5, D7 |
| O3 | The reason for rejecting Bevy's 512-px cap is backwards | **Adopted.** Corrected, and the decision is kept on the 4.2× first-decimation ground. | §5.7, D8, RESEARCH T9 |
| O4 | `pow` as `exp2(y·log2 x)` changes the SPIR-V | **Adopted.** An intrinsic `Pow` node, plus a SPIR-V instruction census at PX2. | §9.1, §10 PX2 |
| O5 | DXGI or a calibration screen is a technical fork | **Adopted.** DXGI with a user override is decided; only the calibration screen's scope stays with the owner. | §5.17, §13 V5, D17 |
| O6 | One uber module makes the minimum configuration pay full register pressure | **Adopted as a measurement.** PX1 compares against a stripped build, and specialization constants (supported, `rhi_impl/device.rs:821`) are used if the loss exceeds 5 %. | §9.2, §8, §10 PX1 |

**Open questions:**

| # | Question | Answer | Where |
|---|---|---|---|
| Q1 | Is the Karis L2 bias intended? Does arming `Bloom` change exposure? | Yes, it is intended: it discards small highlights roughly as full-resolution percentile metering would, where a plain box would inflate them up to 16×. No, bloom does not change exposure: `post_down` is shared, and PX3 pins bit-identity. | §5.5 |
| Q2 | DOF's CoC from jittered `gViewT` flips at silhouettes | Stabilised by quad-closest depth in the prefilter, with a jitter-stability fixture. | §5.9 |
| Q3 | On a camera cut, is adaptation reset, and TAA? | Both, through `ResolvedPost::cut` (an active-entity change or a `CameraCut` event): exposure snaps, TAA resets through the runner's existing site, and motion blur is skipped. The flag is published for the sibling histories. | §3.3, §5.5, §5.10 |
| Q4 | What detects a component added or removed if the union is computed only at rebuild? | The union is now recomputed every frame and compared in `sync_gbuffer`'s existing per-frame predicate. | §3.3 |
| Q5 | The primary source for AgX | Sobotka's `config.ocio` for the matrix and the log2 allocation. The curve is a baked `.spi1d`, and no licence statement was found, so AgX is conditional on a licence check; otherwise an in-house AgX-like curve. | §5.6, RESEARCH T15 |
| Q6 | The source for "HDR colour space exposed when OS HDR is on" | None. The claim is withdrawn, and PX13 enumerates it on the owner's machine. | §5.17 |
| Q7 | What does comparing a native stack with FSR2 Quality establish? | Nothing, so it is withdrawn. FSR2's figure stays only as a vendor number on another GPU. | §6.2, §5.13 |

**Found by the architect in this pass:**

| # | Finding | Action |
|---|---|---|
| A1 | R4a's byte-equal transition pin cannot hold through a B10G11R11 (or even an RGBA16F) store [E] | PX1 runs it as ≤ 1/255; handed to the reflections campaign, which owns R4a (§5.1) |
| A2 | `particle_draw` applies no exposure | Contract E1; handed to the VFX and particles campaigns (§5.3) |
| A3 | The tree's `SharpenMode::Rcas` kernel is CAS | Recorded as stale naming in RESEARCH §0.6; the kernel is replaced at PX1 (§5.11) |
| A4 | Pass 1 costed most passes on DRAM alone | Every pass now carries a fetch term (§6.1) |
