# Rung 3b — OPT-IN Temporal Shadow-vis Denoise (TAA spine, Option B, staged→A)

Owner picked **Staged: B first, then A** — build the motion-vector/history/reproject spine on
shadow-vis (Option B), then extend to whole-frame color TAA (A) as a 2nd selectable mode later.
Design via architect->critic->refine (wf_4e0fd2d4-c16). Default None/Spatial = byte-identical
(58f6c6c3 + af934c50). Full design below.

---

Confirmed everything I need. Key finding for C2: the host uploads *light* view-proj matrices column-major (gpu_scene.rs:2237-2343) but never builds or persists a *camera* view-proj — the camera reaches shaders only as the basis. And `instance_rings` is already `[BoundBuffer; FRAMES_IN_FLIGHT]`-ringed (gpu_scene.rs:764), so a prev-instance ring follows the same discipline (O2).

I now have full evidence to revise. The critic is correct on all three blockers. The core realization: **C1+C2 together mean the mesh per-object motion vector cannot be reconstructed in the VIS pass** — there is no instance id and no camera matrix in the deferred domain. The honest fix is to move mesh MV generation to the raster pass (where `SV_InstanceID`, `position_local`, and the model matrix are live) via a 4th MRT variant, and to build the camera view-proj on the host. I'll retract the "no raster MRT" saving and re-cost.

Here is the final developer-ready plan.

---

# Architecture: Rung 3b — OPT-IN Temporal Shadow-vis Denoise (motion-vector + history + reproject spine) — FINAL

## Changelog vs the critiqued draft (what changed and why)

| Critic | Disposition | Change |
|---|---|---|
| **C1** (no per-pixel instance id → mesh MV unbuildable in VIS) | **ACCEPTED** | Mesh motion vectors move to the **raster G-buffer pass** as a 4th MRT (`SV_Target3`, `motion_vec`), under a `-DMOTION_VECTORS` gbuffer variant where `SV_InstanceID` + `position_local` + `PrevInstanceModelCol[id]` are live. The "no 4th MRT / MV lives in one VIS pass" saving is **RETRACTED and re-costed**. SDF-pixel MV is written by the marcher/VIS front-matter (camera-only, deferred SDF-edit motion). See Decision 3 (rewritten). |
| **C2** (no camera `cur/prev_view_proj` in any shader; host does not persist it) | **ACCEPTED** | New host-built column-major `cur_view_proj` (matching the O1 light-matrix convention) + a persisted `prev_view_proj`, delivered to the **raster VS** (mesh MV) and the **VIS/marcher** (SDF MV) via a new `MotionCam` UBO. Proven inert for `mode ∈ {None,Spatial}` (UBO unbound, no MV MRT). See Decision 2b (new). |
| **C3** (`seeded_history` duplicates shipped `seeded_readers_at_layout`) | **ACCEPTED** | `seeded_history` **deleted**. The history ring uses the existing `ResSync::seeded_readers_at_layout` (sync.rs:186) + `add_image_seeded` (graph_bridge.rs:404), the DDGI precedent, per-slot. Implementation step 3 reframed as "wire the ring through the existing seed." See Decision 5. |
| **W1** (16→32 B UBO byte-identity asserted, not proven) | **ACCEPTED** | `ResolvedShadowDenoise` stays **16 B** (unchanged). Temporal params go in a **separate** 16 B `ResolvedTemporalShadow` UBO bound ONLY when `temporal_enabled()`. The `Spatial` upload byte-stream is provably untouched. See Decision 1. |
| **W2** (RG16 history can't detect same-pixel surface swap) | **ACCEPTED** | History widened to **`R16G16B16A16`** (R=vis, G=confidence, B=prev `view_t`/depth). Disocclusion fires on `|reproj_depth − hist_depth| > τ·depth`. See Decision 5. |
| **W3** (ray-amortization claim overstated under motion) | **ACCEPTED** | Value claim **scoped honestly**: ¼-cost holds for static/slow camera over static geometry; under motion the clamp/reset collapses toward single-frame — the in-motion eval decides. See Goal + Metrics. |
| **W4** (Option A / `TemporalMode::Color` smuggled into 3b surface) | **ACCEPTED** | All `TemporalMode::Color` symbols, "shared spine API", and jitter references **removed from the 3b surface**. 3b is self-contained shadow-vis temporal. Option A is a pure future note. Camera jitter is fully out of 3b. |
| **O1** (k=0 anchor insufficient) | **ACCEPTED** | Added a **static-camera k=0.95** convergence anchor (MV≡0 ⇒ output==current after convergence) isolating reprojection identity. See Metrics. |
| **O2** (prev-instance ring/fence parity) | **ACCEPTED** | Prev-instance SSBO is **`[BoundBuffer; FRAMES_IN_FLIGHT]`-ringed** exactly like `instance_rings` (gpu_scene.rs:764); explicit fence ordering stated. See Multithreading. |

Net effect of C1+C2: the primary ghosting fix is now buildable (raster MV with true per-object prev-transform), at the cost of a 4th gbuffer MRT variant + a host camera-matrix path. Re-cost: **~5.5–6.5 dev-days** (was ~4–5).

## Scope decision (A vs B vs staged) — DECIDED: **Option B only, self-contained**

**3b = a self-contained OPT-IN temporal shadow-visibility denoiser.** It reprojects the Rung-3a spatially-filtered shadow-vis (`final_vis_res`) through motion vectors into a per-FIF history ring, variance-clamps, accumulates, and feeds the DENOISED resolve's `gShadowVis` read. Default `None`/`Spatial` ⇒ byte-identical to `58f6c6c3` (±hwrt) + `af934c50`.

Justification unchanged from the draft (shadow-vis is a clamped [0,1] scalar already being spatially denoised ⇒ the lowest-risk temporal signal; reuses 3a's ringed RG16 targets, the SHADOW_STAGE variant discipline, `shadow_denoise_storage_ok()`, the ResId-append pattern). **Whole-frame color TAA (Option A) is fully deferred** — no Option-A symbols, no camera jitter, no "shared spine" API shaping in 3b (W4). If Option A ever ships, it refactors toward reuse then; 3b does not pre-pay for it.

Deferred out of 3b (explicit): whole-frame color TAA; camera jitter (Halton); RGBA16F `gLit` color history; **SDF-body per-edit motion** (SDF pixels reproject camera-only — moving SDF bodies ghost, bounded by clamp+reset, gated behind the in-motion eval).

## Goal

Opt-in temporal shadow-vis denoiser: default OFF ⇒ 0 bytes / 0 ns delta, both goldens reproduced. ON ⇒ temporal stabilization of the RT shadow term. **Value claim, honestly scoped (W3):** for a static or slowly-moving camera over static geometry, temporal accumulation amortizes RT shadow rays across frames (`ray_count=4 + temporal` approaching `ray_count=16 + spatial` quality) at ~¼ the TLAS traversals. **Under fast motion the variance clamp + disocclusion reset intentionally collapse the effective sample count toward the single-frame `ray_count`** (correctness over amortization) — so the amortization is a static/slow-motion win, not a general one; the in-motion eval is the arbiter.

Target metrics: reproject+accumulate ≤ 0.18 ms @1080p (RGBA16 history: ~16.6 MB read + 16.6 MB write + 8.3 MB MV + 8.3 MB current ≈ 50 MB ⇒ ~0.2 ms @250 GB/s); +33 MB VRAM (2× RGBA16 history ring) + 8.3 MB (RG16F MV) + 8.3 MB×2 (prev-instance ring is tiny, ~48 B×N) when ON; 0 alloc/frame; OFF delta = 0.

## Context and constraints

Affected (all `#[cfg(feature="hwrt")]`-walled, mirroring 3a):
- `boyko_render`: `shadow_denoise_config.rs` (mode enum extend + separate temporal UBO), `instance_model.rs` (prev column + sync), plugin.
- `boyko_rhi_vulkan`: `present/graph_bridge.rs` (ResIds + pass), `present/targets.rs` (MV + history rings), `shaders/shadow_temporal.comp.hlsl` (new), `shaders/gbuffer_mrt.{vs,fs}.hlsl` (`-DMOTION_VECTORS` 4th-MRT variant), the VIS/marcher SDF-MV write, `framegraph/sync.rs` (REUSE `seeded_readers_at_layout`).
- `boyko_app`: `gpu_scene.rs` (`MotionCam` UBO build + prev-instance ring + set build + gShadowVis rebind), `runner.rs` (boot env).

Invariants:
- **I1 (OFF byte-gate):** `mode ∈ {None,Spatial}` ⇒ no MV MRT variant, no MotionCam UBO bound, no temporal pass, no new image referenced ⇒ every `.spv` + upload byte-identical. Composes with 3a.
- **I2 (no jitter):** no jitter on ANY 3b path — projection byte-untouched (structural, not multiply-by-zero).
- **I3 (history value-carrying):** history ring seeded via `seeded_readers_at_layout` (GENERAL, not UNDEFINED) — reuses the DDGI-proven seed; equiv-pinned against the existing DDGI seed test.
- **I4 (single scene of truth):** one `prev_view_proj` + one per-object `PrevInstanceModelCol` per frame, both ECS-native.
- **I5 (3a is the disocclusion fallback):** invalid history ⇒ output = current `final_vis_res` (never worse than 3a).
- **I-O1 (majorness):** `cur/prev_view_proj` are **column-major**, matching the CSM/spot/point light matrices (deferred_pbr.hlsl:172,223; gpu_scene.rs:2237-2343) — no transpose-mismatch smear.

## Key decisions

### Decision 1: Mode selector — `ShadowDenoiseMode { None|Spatial|Temporal|Both }`, temporal params in a SEPARATE 16 B UBO (W1)

**What:** Grow `ShadowDenoiseMode` (shadow_denoise_config.rs:44) to 4 states. `ShadowDenoiseConfig` gains temporal fields. Derived: `spatial_enabled() = matches!(mode, Spatial|Both)`, `temporal_enabled() = matches!(mode, Temporal|Both)`.
- **`ResolvedShadowDenoise` stays 16 B, byte-unchanged** (the à-trous @20 UBO is untouched ⇒ `Spatial` upload provably identical to 3a — resolves W1 by construction, not by an open question).
- Temporal params live in a **new 16 B `ResolvedTemporalShadow`** UBO, bound ONLY when `temporal_enabled()`.

**Why:** One legible 4-state lattice (owner's "selectable by choice"); `Both` = à-trous THEN temporal (SVGF ordering — spatial pre-blur lowers the variance the clamp must tolerate). Separate UBO ⇒ zero perturbation of the shipped `Spatial` byte-stream. Mirrors SSAO's single-enum keying.

**Alternatives rejected:** growing `ResolvedShadowDenoise` to 32 B (W1: risks the `Spatial` upload stride — rejected); two independent `bool`s (non-structural, `Both` becomes cross-config coupling).

**Trade-off:** one extra 16 B UBO + descriptor binding, live only when temporal is on.

### Decision 2a: Option B does NOT jitter (I2)

Unchanged and reinforced by W4: no jitter code on any 3b path. The projection is byte-identical to today. Jitter is an Option-A concern, fully deferred. The shadow edge is already spatially widened (13-tap PCF, deferred_pbr.hlsl:565) + à-trous filtered; jitter adds nothing to a variance-clamped scalar and would endanger the golden.

### Decision 2b: Camera view-proj path (C2) — new host `cur_view_proj` (column-major) + persisted `prev_view_proj`, delivered via a `MotionCam` UBO

**What (C2 fix):** No shader has a camera view-proj today (only the basis at b5; the sole `view_proj` matrices are *light* matrices). Add:
- **Host:** build `cur_view_proj: [[f32;4];4]` column-major (same convention as `cascade_view_proj` at gpu_scene.rs:2237) from the existing camera projection+view the host already computes for the basis. Persist last frame's into `prev_view_proj` (one 64 B copy per frame, after submit). Store in the render-camera resource (ECS-native, alongside the basis source).
- **UBO:** a new `MotionCam { float4x4 cur_view_proj; float4x4 prev_view_proj; }` (128 B), delivered to BOTH the **raster gbuffer VS** (mesh MV) and the **VIS/marcher** (SDF MV). Bound ONLY when `temporal_enabled()`.

**Inertness proof (I1):** when `mode ∈ {None,Spatial}`: (a) the gbuffer VS is the frozen 3-MRT `.spv` variant — `MotionCam` is not in its layout, not bound; (b) the VIS/marcher SDF-MV write is `#if MOTION_VECTORS`-gated out; (c) no `MotionCam` UBO is created or uploaded. The camera basis math (b5) is byte-untouched — `MotionCam` is additive, consumed only by MV code that does not exist in the OFF variants. The golden is a pure function of the unjittered basis exactly as today.

**Why:** MV math needs `clip_to_uv(mul(cur_view_proj, P_world))` and `clip_to_uv(mul(prev_view_proj, prev_world))`. Column-major matches the O1 light-matrix pin (no transpose smear). The host already builds the projection for the basis — this reuses it, adding only the matrix assembly + one persisted copy.

**Alternatives rejected:** reconstructing view-proj in-shader from the basis+fov (redundant matrix rebuild every pixel, drift vs the host projection); reusing a light matrix (wrong frustum).

**Trade-off:** +128 B UBO + host matrix assembly + one 64 B/frame persist. Only live when temporal on.

### Decision 3 (REWRITTEN per C1): Motion vectors — mesh MV in the RASTER pass (4th MRT), SDF MV in the marcher/VIS front-matter

**What (C1 fix — the "no raster MRT" saving is retracted):**

**Mesh pixels (raster, the primary ghosting fix):** add `SV_Target3 motion_vec` to a **`-DMOTION_VECTORS` variant** of `gbuffer_mrt.{vs,fs}.hlsl`. This is where `SV_InstanceID`, `position_local`, and the model matrices are live (the deferred domain has none of these — C1). The VS:
- reads `PrevInstanceModelCol[SV_InstanceID]` from a new **ringed prev-instance SSBO** (byte-parallel to the `InstanceModelCol` instance ring the VS already reads);
- computes `prev_world = prev_m3 · position_local + prev_t`;
- outputs `cur_clip = mul(MotionCam.cur_view_proj, cur_world)` and `prev_clip = mul(MotionCam.prev_view_proj, prev_world)`.
The FS writes `Δuv = clip_to_uv(prev_clip) − clip_to_uv(cur_clip)` to `SV_Target3` (`R16G16_SFLOAT`). This gives the TRUE per-object motion vector for the moving boxes — the box's shadow-vis history is sampled from where the box *was*.

**SDF pixels (marcher/VIS front-matter, camera-only):** the marcher/VIS reconstructs surface `P` from `gViewT`; under `#if MOTION_VECTORS` it computes `Δuv = clip_to_uv(mul(prev_view_proj, P)) − clip_to_uv(mul(cur_view_proj, P))` (camera-only — SDF-edit motion deferred) and writes the same `motion_vec` image at the SDF pixel. Because mesh pixels are raster-owned and SDF pixels are marcher-owned (the r1 ownership gate), the two producers write disjoint pixels of one `motion_vec` target — no conflict.

**Target:** `motion_vec` (`R16G16_SFLOAT`, `COLOR_ATTACHMENT | STORAGE | SAMPLED`, ringed per-FIF). fp16 Δuv ULP at 64 px ≈ 0.03 px — sufficient.

**Why:** C1 proved the VIS pass cannot recover per-object mesh motion (no instance id, no `position_local`, only world `P`). The raster pass is the ONLY place these live. Accepting the 4th MRT is the honest cost of correct moving-box motion — the owner's #1 sensitivity. SDF camera-only MV is a bounded, surfaced gap.

**Alternatives rejected:** VIS-pass mesh MV (C1: unbuildable — circular, needs instance id to invert the model matrix); a G-buffer instance-id channel (a different 4th MRT + a per-pixel model-matrix fetch in VIS — strictly more work than raster MV, and still needs `position_local`, which is gone).

**Trade-off:** a new gbuffer `.spv` variant (2 shaders) + a 4th color attachment in the gbuffer pass when temporal on + the framegraph attachment-count delta for that variant. Walled to `temporal_enabled()`; the OFF path uses the frozen 3-MRT `.spv` (I1).

### Decision 4: ECS-native prev-transform carry — dense `PrevInstanceModelCol` sibling, ringed upload (Principle 0, O2)

**What:** `PrevInstanceModelCol` — a 48 B byte-identical dense sibling of `InstanceModelCol` (instance_model.rs:58). System `sync_prev_instance_model_cols` copies CURRENT `InstanceModelCol` → `PrevInstanceModelCol` each frame, ordered `.before(sync_instance_model_cols)` (instance_model.rs:110) so prev captures the value before curr is refreshed from the moving `GlobalTransform`. The GPU upload uses a **new `[BoundBuffer; FRAMES_IN_FLIGHT]` prev-instance ring**, byte-parallel to `instance_rings` (gpu_scene.rs:764) — the raster VS binds `prev_instance_rings[fi]` alongside `instance_rings[fi]`.

**Why (Principle 0 + O2):** the prev-transform is durable per-entity data ⇒ a dense `ComponentPool` column, NOT a side `Vec` (the SP4-race lesson). The `.before` edge guarantees prev-before-curr. **O2 fence parity:** the prev-instance SSBO is FIF-ringed exactly like `instance_rings`, so frame N writes `prev_instance_rings[fi]` and the GPU reads the same `[fi]` — the fence that gates `instance_rings[fi]` gates this slot identically (no write-before-fence race; the same discipline that fixed reference-crossframe-target-race).

**Alternatives rejected:** a CPU `Vec<PrevModel>` (Principle-0 violation, SP4 race); the `GpuTransform3D{curr,prev}` Pillar-B pair (unbuilt — "plan only").

**Trade-off:** +48 B/entity (RAM+VRAM) + one copy system + one ringed upload, only for the already-instanced M3 set (0%-gate: a non-instancing scene pays nothing).

### Decision 5 (per W2 + C3): History ring + reproject + variance-clamp + accumulate

**What:** `shadow_temporal_hist: [VulkanTexture; FRAMES_IN_FLIGHT]` — **`R16G16B16A16_UNORM`** (R=accumulated vis, G=confidence/frame-count, **B=prev `view_t`/depth (W2)**, A=reserved), ringed: frame `fi` writes `[fi]`, reads `[1-fi]`. **Seeded via the existing `ResSync::seeded_readers_at_layout` + `add_image_seeded`** (sync.rs:186, graph_bridge.rs:404 — the DDGI precedent), per slot, boot-initialized `UNDEFINED→GENERAL` once, then GENERAL for life (C3: no new primitive).

Reproject pass (`shadow_temporal.comp.hlsl`, 8×8 groups):
1. current vis = `final_vis_res.Load(px,py).r` (3a output; when `Temporal` without spatial, = raw VIS output). current depth = `gViewT.Load(px,py)`.
2. 3×3 neighborhood of current vis → μ, σ² → clamp AABB `[μ−γσ, μ+γσ]` (Salvi scalar clamp, γ=`variance_gamma`).
3. `Δuv = motion_vec.Load(px,py)`; prev UV = `(px+0.5,py+0.5)/extent + Δuv`.
4. **Disocclusion (W2):** reset (`out=current; conf=1; depth=cur_depth`, = I5 fallback) if ANY of: prev-UV off-screen; sampled `conf==0`; **`|hist.B_depth − cur_depth| > τ·cur_depth`** (same-pixel surface swap — the moving box sliding over the floor now fires because the history CARRIES prev depth in B).
5. Else: bilinear-sample `hist[1-fi]` at prev UV; clamp sampled vis to the AABB (ghosting ceiling); `k = lerp(feedback_max, feedback_min, saturate(|Δuv|·extent/VELOCITY_REF))`; `out = lerp(current, clamped_hist_vis, k)`; `conf = min(prev_conf+1, CONF_MAX)`.
6. Store `(out, conf, cur_depth, 0)` → `hist[fi]`; store `out` → the temporal-out ResId the DENOISED resolve reads at `gShadowVis` @21.

The DENOISED resolve `gShadowVis` (@21) is fed the temporal-out when `temporal_enabled()`, else `final_vis_res` (creation-time descriptor rebind, zero runtime cost — the deferred_pbr.hlsl DENOISED variant is NOT touched, it already reads @21).

**Why:** scalar variance clamp on a [0,1] term is the cheapest robust ghosting suppressor. `Both` reprojects the low-variance à-trous output ⇒ tighter clamp ⇒ less ghosting AND less noise (SVGF). **W2: storing prev depth in the history B channel is what makes disocclusion actually detectable** — without it, a same-pixel surface swap under motion smears; this is the layer-3 backstop for the moving-box case, so it must fire. `UNORM16` for a normalized vis (uniform precision, no exponent waste).

**Alternatives rejected:** RG16 history (W2: can't detect same-pixel surface swap — rejected); a new `seeded_history` primitive (C3: `seeded_readers_at_layout` already does this — rejected); pure EMA (unbounded ghosting).

**Trade-off:** RGBA16 doubles history bandwidth vs RG16 (~+16 MB VRAM, +~0.05 ms) — accepted because it buys correct disocclusion, the backstop for the now-common camera-independent moving-mesh case.

### Decision 6: Framegraph wiring — mesh-MV 4th attachment + `motion_vec` + history ResIds; pass after à-trous, before resolve

**What:** Under `hwrt`, append (last in the image block, so ResId 0..12 byte-unchanged): `motion_vec` (1 ResId) + `shadow_temporal_hist` ring (1 ResId pair `hist_read`/`hist_write` → physical `[1-fi]`/`[fi]`, one ResId, mirroring the ring pattern) + the temporal-out (1 ResId; may alias `final_vis_res` write-back or be dedicated). `FRAMEGRAPH_IMAGE_COUNT` hwrt bumps 13→**16** (motion_vec + hist + temporal-out); non-hwrt stays 11. The gbuffer MV MRT is an attachment on the existing gbuffer pass in the `-DMOTION_VECTORS` variant (not a new graph image beyond `motion_vec`). All `#[cfg(feature="hwrt")]`-walled.

Pass order: `gbuffer (writes motion_vec MRT for mesh, temporal variant)` → `VIS (writes gShadowVis + motion_vec for SDF)` → `à-trous ×levels (if spatial_enabled)` → **`shadow_temporal (reproject+accumulate)`** → `RESOLVE_DENOISED (reads temporal-out @21)`. Graph derives: gbuffer/VIS-write→temporal-read on `motion_vec`; à-trous-write→temporal-read on `final_vis_res`; temporal-write→resolve-read; the seeded cross-frame history barriers.

**Why:** identical shape to 3a's ResId-append + cfg-gate (graph_bridge.rs:104-109,168-173,717-798,1104-1140). Default `None`/`Spatial` ⇒ `scene.shadow_temporal==None` ⇒ no pass names the new ResIds ⇒ byte-identical.

**Trade-off:** hwrt `FRAMEGRAPH_IMAGE_COUNT` 13→16; the count assert (:168-173), the `images` literal (:1104-1140), and `framegraph_gbuffer_equiv.rs` ResId pins update by the same recipe 3a used for 11→13.

### Decision 7 (RISKIEST): per-object motion for moving meshes + ghosting under motion

Now **buildable** (C1 resolved by raster MV) and addressed head-on.

**The risk:** camera-only reprojection would ghost every moving box (the "wrong only while moving" fingerprint). With C1 accepted, the raster MV carries the TRUE per-object prev-transform ⇒ mesh boxes are correctly reprojected.

**Mitigation (three layers):**
1. **Correct per-object mesh MV (Decision 3, raster).** `prev_world = prev_m3·position_local + prev_t` from `PrevInstanceModelCol[SV_InstanceID]` → the box's shadow-vis history is sampled from where the box was. ECS-native (Principle 0). This is the primary fix, now real.
2. **Variance clamp backstop (Decision 5 step 5).** Any residual MV error (fp16; SDF-edit camera-only) is clamped to the current 3×3 vis AABB — a wrong reprojection pulls toward a valid neighbor, never an out-of-envelope double-image.
3. **Velocity-k + disocclusion reset with prev-depth (Decision 5 step 4, W2).** `k`↓ under motion; hard reset (→3a single-frame, I5) on off-screen / conf==0 / **prev-vs-cur depth mismatch** (the same-pixel surface-swap case now detectable via the history B channel).

**SDF-body motion (surfaced, deferred):** SDF pixels reproject camera-only (no SDF-edit prev-transform in 3b) ⇒ moving SDF bodies ghost, bounded by layers 2+3. This is the ONE uncovered case, gated behind the in-motion eval; a follow-up carries a dominant-SDF-edit prev-transform. The showcase's moving MESH boxes are fully covered.

**Verification:** the in-motion capture is MANDATORY (moving camera + moving boxes, before/after, multiple `k`) — ghosting is invisible in a settled capture (the engine's motion-only-bug history).

## Data structures

```rust
// boyko_render/src/shadow_denoise_config.rs (EXTENDED)
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ShadowDenoiseMode {
    #[default] None, Spatial, Temporal, Both,
}
#[derive(Resource, Clone, Copy, Debug)]
pub struct ShadowDenoiseConfig {
    pub mode: ShadowDenoiseMode,
    pub levels: u32, pub sigma_z: f32, pub sigma_n: f32,          // spatial (3a, unchanged)
    pub feedback_max: f32, pub feedback_min: f32, pub variance_gamma: f32, // temporal (3b)
    pub disocclusion_depth_tol: f32,                              // τ for the W2 depth reset
}
impl Default for ShadowDenoiseConfig {
    fn default() -> Self { Self {
        mode: ShadowDenoiseMode::None, levels: 3, sigma_z: 1.0, sigma_n: 128.0,
        feedback_max: 0.95, feedback_min: 0.85, variance_gamma: 1.0, disocclusion_depth_tol: 0.02,
    }}
}
impl ShadowDenoiseConfig {
    pub const fn spatial_enabled(&self) -> bool  { matches!(self.mode, ShadowDenoiseMode::Spatial | ShadowDenoiseMode::Both) }
    pub const fn temporal_enabled(&self) -> bool { matches!(self.mode, ShadowDenoiseMode::Temporal | ShadowDenoiseMode::Both) }
}

#[repr(C)] // UNCHANGED 16 B — the Spatial @20 UBO byte-stream is provably untouched (W1)
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ResolvedShadowDenoise { pub sigma_z: f32, pub sigma_n: f32, pub _pad0: f32, pub _pad1: f32 }
const _: () = assert!(size_of::<ResolvedShadowDenoise>() == 16); // pin unchanged (W1)

#[repr(C)] // NEW 16 B — bound ONLY when temporal_enabled()
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ResolvedTemporalShadow { pub feedback_max: f32, pub feedback_min: f32, pub variance_gamma: f32, pub depth_tol: f32 }
const _: () = assert!(size_of::<ResolvedTemporalShadow>() == 16);
```

```rust
// boyko_render/src/instance_model.rs (SIBLING + ringed carry)
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct PrevInstanceModelCol { pub rows: [[f32; 4]; 3] }  // byte-identical to InstanceModelCol
const _: () = assert!(size_of::<PrevInstanceModelCol>() == 48);

/// prev := curr, BEFORE sync_instance_model_cols refreshes curr. `.before(sync_instance_model_cols)`.
pub fn sync_prev_instance_model_cols(
    mut q: Query<(&InstanceModelCol, &mut PrevInstanceModelCol), Enabled<RenderEnabled>>,
) { for (cur, prev) in q.iter_mut() { prev.rows = cur.rows; } }
```

```rust
// boyko_app/src/gpu_scene.rs — host camera matrices (C2), column-major (I-O1)
#[repr(C)]
pub struct MotionCam { pub cur_view_proj: [[f32;4];4], pub prev_view_proj: [[f32;4];4] } // 128 B, temporal-only
// prev_instance_rings: [BoundBuffer; FRAMES_IN_FLIGHT]  — byte-parallel to instance_rings (O2)
```

```rust
// boyko_rhi_vulkan/src/present/targets.rs
pub struct GBufferTargets { /* … */
    pub motion_vec: Option<[VulkanTexture; FRAMES_IN_FLIGHT]>,          // RG16F Δuv, ringed
    pub shadow_temporal_hist: Option<[VulkanTexture; FRAMES_IN_FLIGHT]>,// RGBA16_UNORM (vis,conf,depth,_), seeded GENERAL (C3)
}
```

## Public API

```rust
// boyko_render — the ONLY new author surface (mirrors ShadowDenoiseConfig):
//   world.insert_resource(ShadowDenoiseConfig { mode: ShadowDenoiseMode::Both, .. });
// Plugin registers sync_prev_instance_model_cols.before(sync_instance_model_cols).
// Boot env: BOYKO_SHADOW_DENOISE = none|spatial|temporal|both.
// NO TemporalMode::Color / jitter symbols in 3b (W4).
```

## Algorithms for critical paths

**`sync_prev_instance_model_cols`** — O(visible rows), one 48 B column-to-column copy per row, sequential SoA, branch-free (`Enabled` filter), 0 alloc, SIMD-trivial.

**Raster VS mesh MV** — per vertex: `prev_world = prev_m3·pos_local + prev_t` (from `prev_instance_rings[fi][SV_InstanceID]`), `prev_clip = mul(prev_view_proj, prev_world)`, `cur_clip = mul(cur_view_proj, cur_world)`. FS: `Δuv = clip_to_uv(prev_clip) − clip_to_uv(cur_clip)`, RG16F store. ~2 mat-vec + 2 divides over existing VS work.

**Marcher/VIS SDF MV** — per SDF pixel: `Δuv = clip_to_uv(mul(prev_view_proj,P)) − clip_to_uv(mul(cur_view_proj,P))`, RG16F store. ~2 mat-vec over existing front-matter.

**`shadow_temporal` reproject+accumulate** — 2.07 Mpx, 8×8 groups: 9 vis loads (3×3 halo, L2-coherent), 1 depth load, 1 MV load, 1 history bilinear (4 RGBA16 taps), scalar moments+clamp+lerp (~20 FMA), 1 RGBA16 store. Streaming; divergence only at edge/disocclusion (spatially coherent). No LDS. O(pixels). Bandwidth ≈ 50 MB ⇒ ~0.2 ms.

**CPU camera prev (C2)** — after submit: `prev_view_proj = cur_view_proj` (64 B copy); `cur_view_proj` built column-major from the existing camera projection·view. O(1). `debug_assert!(temporal_enabled ⇒ prev_view_proj set after frame 0)`.

## Multithreading model

- CPU render loop single-threaded. `sync_prev_instance_model_cols` is an ordinary parallel ECS system (reads `InstanceModelCol`, writes `PrevInstanceModelCol`, `.before` curr-sync) — no shared mutable state, no atomics, no `Send/Sync` change.
- **GPU cross-frame — history ring:** frame N's READ of `hist[1-fi]` is ordered after N-1's WRITE by the `seeded_readers_at_layout` seed (GENERAL→GENERAL, content-preserving RAW, never a discard — I3, the DDGI-proven mechanism); frame N's WRITE of `hist[fi]` orders after N-1's COMPUTE read via the seed's WAR half. Same-queue submit order + the seed = total per-slot order.
- **GPU cross-frame — prev-instance ring (O2):** `prev_instance_rings[fi]` is FIF-ringed exactly like `instance_rings[fi]`; frame N writes and the GPU reads the same `[fi]` slot, gated by the identical per-slot fence that already protects `instance_rings` — no write-before-fence race (the reference-crossframe-target-race discipline).
- **Frame-private:** `motion_vec` (ringed, written by gbuffer+VIS and read by temporal within one frame) ⇒ plain `add_image`, undefined seed. `MotionCam` UBO is per-FIF like the other UBOs.
- Within-frame RAW/WAR: gbuffer/VIS→temporal (`motion_vec`), à-trous→temporal (`final_vis_res`), temporal→resolve (temporal-out) — all graph-derived.

## Integration

- **New files:** `shaders/shadow_temporal.comp.hlsl` (+committed `.spv`, hand-authored — no eDSL pin, like `shadow_atrous.comp.hlsl`); the `-DMOTION_VECTORS` gbuffer variant `.spv` (2 shaders); optional `temporal_shadow_plugin.rs` (or fold into `shadow_denoise_plugin.rs`).
- **Extended:** `shadow_denoise_config.rs` (4-mode enum + separate 16 B temporal UBO); `instance_model.rs` (`PrevInstanceModelCol`+sync); `graph_bridge.rs` (COUNT 13→16, `motion_vec`/`hist`/temporal-out ResIds, the temporal pass, sink literal, `add_image_seeded` for hist); `present/targets.rs` (MV+history rings, one-shot init, degrade probe, gShadowVis rebind); `gbuffer_mrt.{vs,fs}.hlsl` (`#if MOTION_VECTORS` 4th MRT + prev-instance ring read); VIS/marcher (`#if MOTION_VECTORS` SDF MV write); `gpu_scene.rs` (`MotionCam` build + prev-instance ring + set build + gShadowVis rebind); `runner.rs` (`BOYKO_SHADOW_DENOISE`); `framegraph_gbuffer_equiv.rs` (ResId pins + the I3 history-seed pin against the DDGI seed precedent).
- **REUSED, not created (C3):** `ResSync::seeded_readers_at_layout` (sync.rs:186) + `add_image_seeded` (graph_bridge.rs:404).
- **NOT touched:** `deferred_pbr.hlsl` DENOISED variant (reads `gShadowVis` @21 — consumes temporal-out via rebind, no shader edit); the frozen 3-MRT gbuffer `.spv`; the eDSL-pinned marcher/oct/pack spans (SDF MV is added to the hand-authored front-matter, not the eDSL bodies); `ResolvedShadowDenoise` (W1); every non-hwrt ResId; `boyko_ecs`.

## Implementation plan (committable sub-steps, each byte-identity-gated, default OFF)

1. **Config extension** (pure Rust): 4-mode enum + `spatial/temporal_enabled` + separate 16 B `ResolvedTemporalShadow`; `ResolvedShadowDenoise` UNCHANGED. GATE: unit tests (default None, resolve packs); both goldens byte-identical (W1: `Spatial` upload untouched).
2. **ECS prev column + host camera matrices** (pure Rust): `PrevInstanceModelCol`+sync+`.before` edge; `MotionCam` host build (column-major, I-O1) + `prev_view_proj` persist; prev-instance ring + `MotionCam` UBO host plumbing (unbound). GATE: 0%-gate test (no column ⇒ no work); both goldens byte-identical (no GPU consumer).
3. **History ring via the EXISTING seed** (C3): `shadow_temporal_hist` RGBA16 ring wired through `add_image_seeded` + `seeded_readers_at_layout`; NO pass yet. GATE: framegraph equiv green; the I3 pin reuses the DDGI-seed test shape.
4. **Targets + ResId re-base + sink** (hwrt): `motion_vec` + history rings + degrade probe; COUNT 13→16; count assert + sink literal + equiv ResId pins. NO passes. GATE: both goldens ±hwrt; non-hwrt=11.
5. **Motion-vector generation** (C1+C2): `-DMOTION_VECTORS` gbuffer variant (4th MRT, prev-instance ring, `MotionCam`) + the marcher/VIS SDF-MV write. GATE: `mode∈{None,Spatial}` ⇒ frozen 3-MRT variant, `MotionCam` unbound ⇒ both goldens; the RESOLVE/marcher OFF `.spv` untouched.
6. **`shadow_temporal.comp.hlsl` + pass + accumulate**: reproject + variance-clamp + velocity-k + prev-depth disocclusion reset (W2); wire seeded barriers; DENOISED gShadowVis←temporal-out rebind. Host gate `scene.shadow_temporal = Some(..)` iff `temporal_enabled() ∧ backend==HardwareTri ∧ has_primary_directional ∧ tlas_nonempty`. GATE: `None`/`Spatial` ⇒ both goldens; `Temporal`/`Both` ⇒ owner-eval.
7. **Host wiring + boot env + flip**: `BOYKO_SHADOW_DENOISE=temporal|both`; the algebraic + convergence anchors. GATE: full verification suite.

## As-built — step 1 (`44a4645`) + step 2 (this commit)

- **Step 1 (`44a4645`):** the 4-mode selector + separate 16 B `ResolvedTemporalShadow` UBO, `ResolvedShadowDenoise` unchanged (Decision 1 / W1). `Spatial` upload byte-stream provably untouched.
- **Step 2 (this commit) — commit-boundary refinement (orchestrator's implementation fork):** step 2 was tightened to a **pure `boyko_render` data-layer commit** with ZERO host/GPU surface, so byte-identity holds *by construction* and is fully unit-tested:
  - `PrevInstanceModelCol` (48 B `hwrt`-gated dense sibling) + `sync_prev_instance_model_cols`, registered `.before(sync_instance_model_cols)` in `EnginePlugins` — dormant (0%-gate: no scene carries the column yet).
  - `MotionCam` (128 B UBO) + `MotionCamState` (`Resource`, prev-`view_proj` persist) in a new `hwrt`-gated `motion_cam.rs`, unit-tested (column-major transpose; first-frame `prev==cur` ⇒ MV≡0; second-frame `prev`= last `cur`; static-camera stays zero-motion).
  - **`marcher_view_proj_rows` factored out of `gbuffer_push_from_view`** (view.rs) — the SINGLE marcher-aligned proj·view construction, shared by the raster push (runner.rs:724) and `MotionCam.cur`. Byte-identity of the push proven by the existing `perspective_bridge_reproduces_legacy_push_constants` unit test.
  - **Decision 2b refinement:** `MotionCam.cur/prev` are built from `marcher_view_proj_rows` (the marcher-aligned `pv`, clip.z=clip.w=forward·(P−eye), extent-derived aspect), **NOT** `ViewUniform::view_proj` — the MV endpoints must be placed with EXACTLY the projection the rasterizer/marcher used, or the static convergence anchor (MV≡0) breaks. The `prev` matrix is persisted via `MotionCamState::advance` (ECS-native, Principle 0), first frame `prev==cur`.
  - **Deferred to step 5** (folded with the consumers to avoid dead-code on unbound buffers): the `gpu_scene` `prev_instance_rings` + `motioncam_ring` `[BoundBuffer; FIF]` rings, the per-frame host `MotionCam` build/persist/upload, and the gather-into-prev-ring. The plan's step-2 "prev-instance ring + MotionCam UBO host plumbing (unbound)" moves there.
  - **Gate:** all 4 feature variants (`boyko-render`/`boyko-app` × `hwrt`/non-`hwrt`) `check` + `clippy -D warnings` green; `boyko-render` lib 125 tests + 4 new `motion_cam` tests pass; the push byte-identity unit test green. `boyko_rhi_vulkan` `grand_showcase` golden is unaffected by construction (it sits below `boyko_render` in the dep graph and does not use the bridge).

## As-built — steps 3+4 (this commit): framegraph temporal targets + ResId re-base (dormant)

Plan steps 3 and 4 **merged** into one dormant commit (one `13→16` ResId re-base beats two
successive COUNT bumps + equiv re-pins). All `#[cfg(feature="hwrt")]`, byte-identical (both goldens
`58f6c6c3` ±hwrt, verified).

- **RHI foundation (surfaced at impl start — plan open-Q5):** the plan's `R16G16B16A16_UNORM`
  history format was NOT in `boyko_rhi`'s `Format` enum. Added `R16G16B16A16Unorm = 91`
  (`VK_FORMAT_R16G16B16A16_UNORM`) + the `ffi` const + the `abi_guard` const-assert (the enumerant
  cross-check the other formats carry). Foundation-before-API.
- **Three hwrt framegraph images** appended LAST in the image block (ResId 0..12 byte-unchanged;
  buffers still begin at `FRAMEGRAPH_IMAGE_COUNT`): `motion_vec` (ResId 13, RG16F, frame-private) +
  `shadow_temporal_hist` (ResId 14, RGBA16, **`add_image_seeded` at GENERAL** — the I3/DDGI
  cross-frame content-preserving seed) + `temporal_out` (ResId 15, RG16, frame-private). `temporal_out`
  is a DEDICATED target (not an in-place à-trous write-back) so the reproject's 3×3 neighborhood read
  cannot race the accumulate write. `FRAMEGRAPH_IMAGE_COUNT` hwrt `13→16`; the `- FRAMEGRAPH_IMAGE_COUNT`
  buffer re-base keeps every buffer's LOGICAL sink slot invariant (absolute buffer ResIds +3).
- **targets.rs (orchestrator fork):** the three ring fields + create fns; built via a leak-safe
  `build_denoise_ring` helper **at the END of `create`** (after every fallible descriptor set) that
  **degrades to `None`** on any create failure — so they need ZERO teardown weaving into the ~8-site
  error ladder above (the "recorded-not-fail-fast" opt-in policy). Gated on the SAME
  `shadow_denoise_storage_ok()` probe as the vis rings; destroyed FIRST (reverse-acquisition) in
  `destroy`.
- **No pass names ResId 13/14/15 this step** (`let _ =` reserves the graph handles) ⇒ zero barriers
  ⇒ byte-identical. The MV producers (step 5) + temporal pass (step 6) add the accesses.
- **Gate:** `boyko_rhi` enum tests + `boyko_rhi_vulkan` compile ±hwrt + `clippy -D warnings` ±hwrt
  green; `framegraph_gbuffer_equiv` 10 (hwrt) / 7 (non-hwrt) pass — incl. the updated
  `hwrt_resid_18_sink_slot_mapping_pinned` (IMAGE_COUNT 16, buffer ResIds +3, sink slots unchanged);
  `grand_showcase` golden `58f6c6c3` byte-identical BOTH hwrt-ON and hwrt-OFF; `boyko-app` hwrt
  compiles.

## As-built — step 5a (mesh motion vectors) — SHIPPED (this commit)

Step 5 split into **5a (mesh MV, raster) + 5b (SDF MV, VIS)** — the mesh path is the owner's #1
in-motion sensitivity (moving boxes) and is self-contained (raster only, no resolve-set changes);
5b touches the resolve descriptor set (new bindings + a MAX_BIND_GROUP_BINDINGS bump) and lands
separately.

**Shaders DONE + byte-identity proven** (`gbuffer_mrt.{vs,fs}.hlsl`, `-D MOTION_VECTORS=1`):
- VS: under `#ifdef MOTION_VECTORS` adds binding 1 `prev_instances` SSBO + binding 2 `MotionCam`
  UBO (128 B, cur+prev marcher-aligned view-proj); the legacy arm emits camera-only clip (world =
  `input.position`, prev==cur world), the instanced arm reads `prev_instances[base+id]` for TRUE
  per-object motion; forwards `cur_clip`/`prev_clip` as varyings.
- FS: adds 4th MRT `SV_Target3 motion_vec` (R16G16_SFLOAT) = `clip_to_uv(prev_clip) −
  clip_to_uv(cur_clip)`. `clip_to_uv(c) = c.xy/c.w*0.5+0.5` — NO extra y-negation (the projection
  `marcher_view_proj_rows` already bakes `sy=-1/tan`). **Both clip positions are VS varyings divided
  through the identical `clip_to_uv`**, so a static pixel yields exactly `(0,0)` (mixing SV_Position
  for cur with an interpolated prev would break that; that is why the plan passes both).
- **GATE PASSED:** base recompile (no define) is BYTE-IDENTICAL to the frozen `gbuffer_mrt.vs.spv`
  (4480 B) + `gbuffer_mrt.fs.spv` (2252 B). MV variant compiles clean (`gbuffer_mrt_mv.vs.spv` 5780 B,
  `gbuffer_mrt_mv.fs.spv` 2732 B). `compute.rs`: `GBUFFER_MRT_MV_{VS,FS}_SPV<5780/2732>` +
  `gbuffer_mrt_mv_{vs,fs}_spirv()` accessors (hwrt-gated). dxc command confirmed by reproducing the
  frozen VIS `.spv` (`7b704fcb…`, 8032 B) byte-for-byte.

**Binding contract (host must match):** set 0 → binding 0 `instances` SSBO, binding 1 `prev_instances`
SSBO, binding 2 `MotionCam` UBO (128 B), ALL `ShaderStage::VERTEX`. MV pipeline = 4 color formats
`[R8G8B8A8Unorm×3, R16G16Sfloat]`, `D32Sfloat` depth, 88 B vertex push (unchanged), 40 B vertex layout.

**Host design decisions (orchestrator forks):**
- **MV resources** bundled into `#[cfg(feature="hwrt")] mv: Option<MotionVecResources>` on
  `GpuSceneBundles` — boot-built under `ctx.ray_query_enabled() && ctx.device_caps().
  shadow_denoise_storage_ok()` (matches the `motion_vec` target gate + the hwrt stack; the 3a
  "decouple set-build from the per-frame activation gate" lesson). Holds: the MV pipeline + 3-binding
  layout, `prev_instance_rings: [BoundBuffer; FIF]` (byte-parallel to `instance_rings`),
  `motion_cam_ubo: [BoundBuffer; FIF]` (128 B UNIFORM), `bind_groups: [VulkanBindGroup; FIF]`
  (instances@0, prev_instances@1, motion_cam@2).
- **Prev-gather PAIRED in ONE query** (`Option<&PrevInstanceModelCol>` added to the `gather_mesh_draws`
  query, hwrt-gated) filling a hwrt-gated `prev_ring` lane of `MeshRenderScratch` in the SAME
  mesh-bucketed order ⇒ index-aligned with `ring` by construction; a row missing the prev column
  falls back to its current `InstanceModelCol` (⇒ camera-only MV for that entity, safe). NOT a second
  independent gather (set-mismatch risk).
- **Scene threading:** add `#[cfg(feature="hwrt")] temporal_enabled: bool` (+ the MV pipeline/bind-group
  refs) to `GBufferScene`; `runner` reads `ShadowDenoiseConfig::temporal_enabled()`; `scene()` threads
  it. `MotionCamState` (Resource) persists prev-view-proj; runner builds `MotionCam` via
  `marcher_view_proj_rows(&view, cw, ch)` + `.advance()`, uploads the UBO + prev-ring.
- **Recording (record_gbuffer):** when `scene.temporal_enabled` (and `mv` present) bind the MV pipeline
  + MV bind group + a 4-attachment color array (append the `motion_vec` view). Pipeline color-attach
  count is fixed at create ⇒ the 4-format pipeline MUST pair with a 4-attachment `begin_rendering`
  (select both together). Push constants (88 B) unchanged.
- **Framegraph:** raster pass adds `g.image_access(motion_vec, COLOR_ATTACHMENT_OUTPUT, WRITE,
  COLOR_ATTACHMENT_OPTIMAL, COLOR)` ONLY when temporal ⇒ OFF path names no new ResId ⇒ byte-identical.
- **GATE (testable now):** mode∈{None,Spatial} + non-hwrt ⇒ both goldens `58f6c6c3` ±hwrt +
  eDSL-sync test green. Temporal-ON writes `motion_vec` but has NO consumer until step 6 (a harmless
  framegraph dangling write); MV VALUES validated by owner-eval at step 6/7 (in-motion).

**Code review (all 7 priority points CLEAN — binding contract, attachment lifetime, index-alignment,
O2 fence parity, MotionCam extent/view, soundness) + 4 findings fixed:**
- **W1 (gate divergence, would have bitten step 6):** the framegraph declared the `motion_vec` write
  barrier on `temporal_enabled` alone, but the recorder writes on `temporal_enabled && mv pipeline +
  bind group exist` — diverges on a `storage_ok && !ray_query` device (e.g. `BOYKO_FORCE_SOFTWARE=1`),
  which would have made step 6 read an uninitialized `motion_vec`. FIX: a single-source
  `GBufferScene::mesh_mv_active()` used by BOTH the graph declaration and the recorder.
- **W2 (OFF-path waste, Principle 1):** the hwrt gather ran the full O(N) prev-scatter every frame even
  temporal-OFF. FIX: gated `gather_prev_ring_into` on `ShadowDenoiseConfig::temporal_enabled()`
  (`Res`, always present via `EnginePlugins`+`ShadowDenoisePlugin`) — OFF hwrt frames now pay zero.
- **O1:** the 4th-attachment view is now `expect`-ed (not `unwrap_or(NULL)`) so a future MV-gate
  loosening trips loudly instead of binding a NULL color attachment.
- **O2:** de-magic'd the `/48` in the prev-ring overflow diagnostic (`size_of::<InstanceModelCol>()`).
- **False-green caught:** the hwrt golden first "passed" against a STALE `.bmp` — the hwrt test binary
  failed to compile (`GBufferScene` gained 3 fields the 4 in-test literals didn't set; `cargo check`
  skips test targets). FIX: set the fields in all 4 harness literals; re-ran with a delete-then-run
  so the hash reflects a real render. Lesson reinforced: golden verification MUST delete the artifact
  first + build `--all-targets`.
- **Final gate (this commit):** golden `58f6c6c3` byte-identical BOTH ±hwrt (real, delete-then-run);
  `check`/`clippy -D warnings` ±hwrt `--all-targets` green; `boyko-render` 125 lib tests + framegraph
  -equiv 10 (hwrt)/7 (non-hwrt) + eDSL-sync 2 pass.

## As-built — step 5b (SDF motion vectors, VIS pass) — SHIPPED (this commit)

Completes the motion-vector producers: the VIS pass writes each SDF pixel's camera-only motion
vector to the SAME `motion_vec` image the raster wrote mesh pixels into (disjoint pixels by the r1
ownership gate). SDF-edit motion (the SDF surface itself moving) is deferred — camera-only.

**Shader DONE + byte-identity proven** (`deferred_pbr.hlsl`, `-D HWRT=1 -D SHADOW_STAGE=1 -D
MOTION_VECTORS=1`):
- Under `#ifdef MOTION_VECTORS`: binding 22 `MotionCam` UBO (cur+prev marcher-aligned view-proj, the
  SAME 128 B pair the raster MV reads) + binding 23 `gMotionVec` (rg16 STORAGE) + `mv_clip_to_uv`
  (identical to the raster's `clip_to_uv`, so mesh + SDF MV share one UV space).
- The write is injected right after `float3 P = ro + rd * view_t` (inside `if (is_sdf_lit)`, BEFORE
  the light loop) — so EVERY SDF pixel writes `Δuv = mv_clip_to_uv(prev·P) − mv_clip_to_uv(cur·P)`
  exactly once. Static camera ⇒ Δuv = 0.
- **GATE PASSED:** ALL 4 base variants recompile BYTE-IDENTICAL (software 65456, hwrt 59424, VIS
  8032, DENOISED 57576) — the `#ifdef MOTION_VECTORS` blocks don't perturb them. New VIS-MV variant
  `deferred_pbr_hwrt_vis_mv.comp.spv` = 8932 B. `compute.rs`: `DEFERRED_PBR_VIS_MV_SPV<8932>` +
  `deferred_pbr_vis_mv_spirv()` (hwrt-gated).
- **`MAX_BIND_GROUP_BINDINGS` 22→24** (both `boyko_rhi/device.rs` + `boyko_rhi_vulkan/rhi_impl.rs`,
  the const-assert links them) — reserves bindings 22/23; BYTE-NEUTRAL (only the 24-binding VIS-MV
  set fills the two new tail slots; software 19 / RESOLVE_INLINE 21 / base VIS-DENOISED 22 untouched;
  the exact-fill `debug_assert` pins to `RESOLVE_SOFTWARE_BINDINGS`=19, not the cap).

**Host design (orchestrator forks):**
- **Reuse, not recreate:** the VIS-MV set binds 5a's `MotionVecResources.motion_cam_ubo[fi]` @22
  (runner already uploads it every temporal frame) + `targets.motion_vec[fi]` @23 (STORAGE view). NO
  new ring, NO new upload, NO runner change.
- VIS-MV pipeline + 24-binding layout boot-built under the SAME gate as `shadow_denoise_pipelines`
  (ray_query) + `shadow_denoise_storage_ok` (the motion_vec target); stored in `MotionVecResources`.
  The per-frame VIS-MV bind group = the 22 base VIS entries + the 2 new tail binds.
- **Framegraph raster→VIS WAW on `motion_vec`:** 5a's raster writes it as COLOR_ATTACHMENT (mesh
  pixels); 5b's VIS writes it as STORAGE/GENERAL (SDF pixels). The graph derives the ordering +
  the COLOR_ATTACHMENT_OPTIMAL→GENERAL transition from the two `image_access` declarations.
- **Single-source gate `sdf_mv_active()`** (the W1 lesson) — BOTH the framegraph VIS `image_access`
  and the VIS pass recording gate on it, so the declared STORAGE write and the actual write can't
  diverge.
- **GATE (testable now):** None/Spatial + non-hwrt ⇒ base VIS variant (no motion_vec write) ⇒ both
  goldens `58f6c6c3` ±hwrt + all base `.spv` byte-frozen. The VIS pass runs when spatial is armed, so
  5b's SDF-MV is exercised in mode `Both` (spatial+temporal); step 6 extends the VIS pass to run for
  pure `Temporal`. MV VALUES validated by owner-eval in-motion at step 6/7.

**Code review (all priority points CLEAN — binding layout↔shader match, WAW barrier, disjoint-pixel
invariant, byte-identity, teardown) + 3 findings resolved:**
- **C1 (Critical, GENUINE BUG):** `gMotionVec` was pinned `rg16` (= `R16G16_UNORM` in this codebase,
  the `gShadowVis` convention) on the `R16G16_SFLOAT` `motion_vec` image. Δuv is SIGNED and can exceed
  [0,1] — a UNORM store clamps negative/>1 SDF motion and disagrees with the raster's SFLOAT mesh
  pixels of the SAME image (a torn seam under motion the golden can't catch — OFF path). FIX: `rg16` →
  `rg16f`, regenerated (`SpirvBlob` 8932→9008). The raster (5a) was already correct (SFLOAT color
  attachment, no image_format pin).
- **W1 (reachable panic on a Spatial→Both mode change):** `build_shadow_vis_mv_resolve_set` gated on
  the PER-FRAME `scene.temporal_enabled`, so a mid-session toggle (set built when temporal off ⇒
  `None`) would hit the recorder `expect`. FIX: decouple the build from `temporal_enabled` — gate on
  the STABLE signals (like `build_shadow_denoise_sets`), so the set exists before `sdf_mv_active()`
  flips on. The memory lesson ("build denoise sets on stable boot config, not the per-frame gate") 
  applied. In `Spatial` the set is built-but-unused; the recorder gates USE on `sdf_mv_active()`.
- **W2 (sign convention):** verified the 5a raster (`gbuffer_mrt.fs.hlsl:154`) and the 5b VIS both
  emit `Δuv = uv_prev − uv_cur` — IDENTICAL order, so the mesh + SDF vectors agree across the seam.
  (The reviewer flagged it unverifiable because it missed the in-tree 5a `.hlsl`; the orders match.)
- **O1:** background/sky `motion_vec` = the raster attachment's `loadOp=CLEAR` (0,0) = no motion —
  already correct from 5a.
- **Final gate (this commit):** golden `58f6c6c3` byte-identical BOTH ±hwrt (real, delete-then-run);
  `check`/`clippy -D warnings` ±hwrt `--all-targets`; framegraph-equiv 10 + eDSL-sync 2. All 4 base
  resolve `.spv` recompile byte-frozen (65456/59424/8032/57576).

## Metrics and validation

- **Byte-identity:** `None`/`Spatial` reproduce `58f6c6c3` (±hwrt) + `af934c50`. Framegraph equiv ResId pins (16 imgs hwrt) + the **I3 pin** (no `UNDEFINED` old-layout on `shadow_temporal_hist` after init — reusing the DDGI-seed test shape, C3).
- **Algebraic anchor (k=0):** `Temporal, feedback_max=0` ⇒ output == current `final_vis_res` bit-exactly (pass-through at zero history weight).
- **Convergence anchor (O1, k=0.95, static camera + static geometry):** MV≡0 ⇒ prev-UV≡cur-UV; after K frames the output must equal the current `final_vis_res` (history==current) — isolates the reprojection/clamp identity from accumulation (catches MV-sign / `clip_to_uv` / clamp-AABB bugs that k=0 hides).
- **Temporal-stability metric:** `shadow_motion_ab_dump` (deferred_pbr.hlsl:561) under a scripted 3-mrad yaw — max shadow-edge frame-to-frame delta must drop vs `Spatial`; static-camera K=32 variance below threshold.
- **Ray-amortization (W3-scoped):** `ray_count=4 + Both` vs `ray_count=16 + Spatial` at a **static camera** — 3a's grain residual must be ≤ the 16-ray-spatial residual. Explicitly NOT claimed under motion (the clamp/reset collapse toward single-frame is the intended correctness behavior).
- **In-motion ghosting eval (MANDATORY, owner = visual oracle):** orchestrator captures moving-camera + moving-boxes, before (`Spatial`) / after (`Both`), at `feedback_max ∈ {0.85, 0.95}`, via an IN-MOTION capture (`shadow_lag_dump` class — NOT settled). Moving-box penumbra must not smear/double; SDF-body ghosting is the known-uncovered case (surfaced).
- **Determinism:** scripted path, serial-indexed, SHA-equal across two runs.

`debug_assert!`: `temporal_enabled ⇒ prev_view_proj set (after frame 0)`; `temporal_enabled ⇒ MotionCam bound`; reproject write ≤ RGBA16 stride; `temporal_enabled ⇒ camera_mode == PERSPECTIVE`; DENOISED gShadowVis binding == temporal-out ResId when temporal on.

## Riskiest point + concrete mitigation

**Riskiest:** ghosting of the moving showcase boxes under a moving camera — the owner's exact prior-campaign sensitivity ("wrong only while moving"). C1 made the naive VIS-pass fix unbuildable; the plan resolves it by (1) generating **true per-object mesh MV in the raster pass** from the ECS-native `PrevInstanceModelCol` (the box reprojects to where it was), (2) the **variance clamp** ceiling on any residual error, (3) **velocity-k + prev-depth disocclusion reset** (now detectable because history carries prev depth, W2), falling back to the spatially-denoised single frame (I5, never worse than 3a). The uncovered residual — **moving SDF bodies** (camera-only MV) — is surfaced, bounded by (2)+(3), and gated behind the mandatory in-motion owner eval; its full fix (a dominant-SDF-edit prev-transform) is an explicit follow-up. **Mitigation of the mitigation:** the verification REQUIRES an in-motion capture, because a settled capture masks exactly this class (the engine's documented motion-only-bug history).

## Open questions

**VALUES/SCOPE (owner):**
1. Accept the 4th-MRT raster-MV cost (buildable moving-mesh correctness, ~+1.5 dev-days) — the plan assumes yes (C1). Confirm.
2. `Both` (recommended) vs `Temporal` as the default "on" mode.
3. SDF-body per-edit prev-transform: build the follow-up now, or ship B mesh-correct + SDF-camera-only, gated behind the in-motion eval (the plan assumes the latter)?
4. `feedback_max` default 0.95 vs softer — decide after the in-motion eval.

**TECHNICAL:**
5. Whether `cur_view_proj` is already assembled anywhere on the host (the basis path may compute the matrices internally) or must be built fresh from projection·view — one grep at implementation start decides Decision 2b's host surface.

Relevant files (absolute): `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\shaders\gbuffer_mrt.fs.hlsl` (C1: id 0, world-P only; the `-DMOTION_VECTORS` 4th-MRT site), `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\shaders\gbuffer_mrt.vs.hlsl` (the mesh-MV VS site — `SV_InstanceID`/`position_local`/`MotionCam`/prev-instance ring), `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\shaders\deferred_pbr.hlsl:85-94` (C2: camera is a basis, no camera view-proj), `:172,:223,:2237-2343` majorness (I-O1), `:561-580` ("SPATIAL not temporal" + the A/B harness), `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\src\framegraph\sync.rs:186` (C3: `seeded_readers_at_layout` — REUSE), `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\src\present\graph_bridge.rs:104-109,168-173,404-419,717-798,1104-1140` (ResId append + `add_image_seeded`), `D:\claude\BoykoEngine\crates\boyko_render\src\shadow_denoise_config.rs:44` (mode enum), `D:\claude\BoykoEngine\crates\boyko_render\src\instance_model.rs:58,110` (`InstanceModelCol` + `sync_instance_model_cols` — the sibling+`.before` site), `D:\claude\BoykoEngine\crates\boyko_app\src\gpu_scene.rs:764,2237-2343` (`instance_rings` FIF ring — prev-instance-ring precedent, O2; column-major light-matrix upload — `MotionCam` precedent), `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\tests\framegraph_gbuffer_equiv.rs` (ResId + I3 pins), `D:\claude\BoykoEngine\docs\RENDER-RUNG3A-DENOISE-PLAN.md` (the 3a seam), `D:\claude\BoykoEngine\docs\TAA-PLAN.md` (Option A — fully deferred, W4).