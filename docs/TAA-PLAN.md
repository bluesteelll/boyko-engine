# Architecture: Optional Temporal Anti-Aliasing (TAA)

> Status: **converged architect+critic, awaiting owner VALUES answers.** Branch `ecs`. Critic verdict APPROVED-WITH-CHANGES; all amendments (B1, M2-M4, m5-m8) folded inline — changelog at the end. Anchors verified against the working copy on 2026-07-02 unless marked *(relayed)*; those must be re-verified by the developer at the marked sites.

## Goal

Add an **optional** temporal anti-aliasing stage to the hybrid (SDF compute marcher + raster 3-MRT mesh) pipeline that eliminates the documented edge-resampling scintillation (docs/AUDIT-2026-07-PLAN.md:36 — "a 0.17° yaw flips 56% of" hard-edge pixels) via sub-pixel jitter + history accumulation, at ≤ 0.5 ms @1080p on an RTX-3060-class GPU, with **OFF = byte-identical to today's frame** (structural gate, not multiply-by-zero) and zero new CPU allocations per frame.

## Context and constraints

| Constraint | Anchor |
|---|---|
| Frame loop `render_gbuffer_frame`, `FRAMES_IN_FLIGHT = 2` | crates/boyko_rhi_vulkan/src/present/frame_driver.rs; present/mod.rs:66 |
| Framegraph drives every barrier; equiv test pins **23 img + 10 buf + 22 array calls** | docs/ARCHITECTURE-FRAME-GRAPH-PLAN.md:3-24; crates/boyko_rhi_vulkan/tests/framegraph_gbuffer_equiv.rs |
| Cross-frame seed API: `FrameGraph::add_image_seeded` / `add_buffer_seeded`; `ResSync{layout, flush_access, flush_stages, visible_access, visible_stages}`; ResId→VkImage bound at RECORD time | framegraph/graph.rs:145,154; framegraph/sync.rs:108-119, :14-16 |
| **Existing seed constructors pin `layout = UNDEFINED`** — a first-touch content DISCARD, correct for the re-written-each-frame B-002/B-003 resources, **fatal for a read-first history resource** (critic B1) | framegraph/sync.rs:104-106 (the "prior content is discarded" contract), :124-130 |
| G-buffer color = `R8G8B8A8_UNORM` (`GBUFFER_FORMAT`); `gViewT` = `R32_SFLOAT` ring (marcher surface-`t`, sentinel `1e30`); `lit` = `R8G8B8A8_UNORM` ring (resolve STORAGE out, present SAMPLED, TRANSFER_SRC); storage images live in `GENERAL` whole-life | present/targets.rs:137, :71, :66-70, :81, :356-378 |
| Present set is RINGED, **written ONCE at creation**, `present_set@i` samples `lit[i]`; **present sampler is NEAREST** *(relayed — critic M3)* | present/targets.rs:125-128; tests/window_present_gbuffer.rs:1685-1695 |
| Shared ray-gen: `generate_ray` takes the camera as **plain parameters**, pixel center hardcoded `+0.5`; perspective dir = `fwd + right·(ndc_x·aspect·tanHalfFov) + up·(ndc_y·tanHalfFov)` | shaders/ray_gen.hlsli:44-75 |
| `project_to_screen` = the exact inverse, `cam_forward` "contractually NORMALIZED"; SSCS + IGN dither + `gViewT.Load` + shared reconstruction | shaders/deferred_pbr.hlsl:738-770, :735, :798, :816-824 |
| Camera UBO tail is **OCCUPIED**, not headroom: 80 B camera header, M2 grid →128, M4 clip-map →224, MDF →256 | compute.rs:1925, :2147-2154, :2231, :2263-2269 |
| Deferred-resolve descriptor set at its **16/16 cap**; SSAO precedent = own separate set *(relayed)* | present/targets.rs:95 |
| eDSL pin map: marcher field/control-flow/brick/G-buffer-pack/oct/glue byte-identity-pinned; a NEW standalone shader touches zero pins *(relayed)* | boyko_shaderdsl, 22 live-splice pins |
| Pillar B (`GpuTransform3D{curr,prev}` + `frame_alpha`) = **zero code on branch**, plan only *(relayed)* | docs/ARCHITECTURE-FRAME-GRAPH-PLAN.md:241-252 |
| Resize/recreate: two choke points recreate target rings; `frame_index` resets to 0 — the natural invalidation signal *(relayed)* | present/targets.rs (create / `destroy_ring` :388); swapchain recreate |
| No temporal accumulation exists today: cascade cross-fade is analytic; SSCS/SSAO noise is static per-pixel — conventions must survive TAA OFF unchanged | deferred_pbr.hlsl:735; memory: SSAO Hilbert+R2 |

Invariants:
- **I1 (OFF byte-gate):** TAA OFF ⇒ no new passes declared, no new images created, every existing `.spv` artifact and every uploaded byte bit-identical to today. Proven by the 20-config windowed SHA256 harness + the framegraph equiv pins.
- **I2 (shared-depth alignment):** marcher and raster sample the SAME sub-pixel position `(px+0.5+jx, py+0.5+jy)` each frame.
- **I3 (no CPU hot-path allocation):** all TAA resources created at construction; per-frame CPU work = one 96 B UBO write + one index increment.
- **I4 (single scene of truth):** all consumers (marcher, resolve, SSCS, tile/froxel culls, raster) read ONE camera state per frame; jitter flows through that single state.
- **I5 (history is value-carrying):** no barrier derived for a history slot may carry `old_layout = UNDEFINED` after the one-shot init — that is a content discard (critic B1). Enforced by an equiv-test pin.

Target metrics: TAA resolve ≤ 0.5 ms @1080p on RTX 3060 (budget below: ~0.30-0.45 ms at a conservative 250 GB/s effective); +49.8 MB VRAM (33.2 history + 16.6 velocity, velocity only from T5); 0 allocations/frame; OFF = 0 ns, 0 bytes delta.

---

## Research basis (researcher report, condensed)

| Source | Facts used |
|---|---|
| Karis, SIGGRAPH 2014 (UE4) | Jitter via projection-matrix offset; Halton(2,3); bicubic (Catmull-Rom) history resample to stop blur accumulation; neighborhood clamp in YCoCg; blend α≈0.04; 3×3 closest-depth velocity dilation; blend in tonemapped space to kill fireflies. |
| Salvi, GDC 2016 | **Variance clipping**: 3×3 first/second moments in YCoCg, AABB = μ ± γσ, γ = 1.0-1.25, clip history *toward center*; strictly better than min/max clamp. |
| Pedersen (Playdead INSIDE), GDC 2016 + github playdeadgames/temporal | Clip-toward-center; **velocity-magnitude-modulated feedback k ∈ [0.88, 0.97]**; current sample texel-direct (no unjitter resample); closest-fragment velocity dilation. |
| Bevy `bevy_anti_aliasing/taa` | `Rgba16Float` ping-pong history; Halton(2,3) **8-sample** cycle on the projection; explicit `reset` for first frame/camera cut; `MipBias(-1)`; blend 0.015-0.1 via history confidence; inputs = color + depth + motion vectors; component-based opt-in (structural gate precedent). |
| Jimenez ("Filmic SMAA") | 9-tap Catmull-Rom collapses to **5 bilinear taps**. |
| Quake II RTX / ray-traced TAA practice | Ray generators jitter the **ray through the pixel footprint** (UV offset), not a matrix — equivalent to the raster matrix offset; validates the compute-marcher injection below. |
| Alex Evans, Dreams (SIGGRAPH 2015) | A pure-compute splat/SDF renderer treats TAA as *the* spatial reconstruction filter — stochastic per-frame offsets + temporal accumulation, no hardware raster needed. |
| Shipping practice (all three) | **No depth-based history rejection** ships — pure color-space variance clipping. |
| Intel (measured) | 1080p TAA resolve 0.9-1.9 ms on Iris Xe (~68-137 GB/s) ⇒ bandwidth-scaled to a 3060 ≈ 0.25-0.45 ms. |

---

## Key decisions

### Decision 1: Jitter enters as a CPU-side **camera-basis shear** (marcher) + projection-matrix offset (raster) — zero shader changes

**What:** Per submitted frame, with jitter `(jx, jy) ∈ [-0.5, 0.5)²` pixels from Halton(2,3), 8-cycle:
- Marcher/resolve/SSCS/culls: upload `fwd' = fwd + right·(2jx/w · aspect · tanHalfFov) + up·(-2jy/h · tanHalfFov)` in the existing 80 B camera block. Because `generate_ray` computes `dir = fwd + right·(ndc_x·a·t) + up·(ndc_y·t)` (ray_gen.hlsli:63-65), a constant basis shear is **algebraically exact on the generation side**: `dir(fwd', ndc) ≡ dir(fwd, ndc + Δndc)` — every ray passes through `(px+0.5+jx, py+0.5+jy)`.
- Raster mesh pass: offset the clip-space projection so its sample grid shifts by the SAME `(jx, jy)` pixels (`proj[2][0] += 2jx/w`-style; exact signs validated by the host golden — see risks).
- The deferred resolve un-jitters **nothing**: it reconstructs rays with the same sheared cbuffer, so shading, SSCS, and the culls stay self-consistent (I4). The TAA pass reprojects into **unjittered prev space** (Decision 4).
- **Structural zero (critic M3):** when TAA is OFF — and in any ON-path test that forces "no jitter" — the shear code is **skipped entirely** (`Option<[f32;2]>` presence, not `j = (0,0)`): computing `fwd + right·0 + up·(-0.0)` can flip sign bits (`-0.0`) and byte-change the UBO. The OFF path never executes shear arithmetic.

**Why:** (a) The frozen marcher/resolve `.spv` artifacts and every eDSL pin stay byte-untouched — jitter is pure camera *data*. (b) OFF is trivially byte-identical (shear structurally skipped ⇒ UBO bytes bit-equal). (c) Ray-gen jitter through the pixel footprint is the canonical compute-path injection (Quake II RTX, Dreams). (d) I2 holds by construction.

**Alternatives rejected:**
- *Explicit `jitter` float2 in the cbuffer + `generate_ray` change*: new marcher `.spv` variant + eDSL ray-gen surgery + a resolve variant, for zero quality gain.
- *Camera UBO tail ride*: the tail is occupied (M4 clip-map at 80..224, MDF at 224..256 — compute.rs:2147-2269).
- *Jitter only the raster matrix (Bevy-style)*: breaks I2 — mesh and SDF depth disagree up to 1 px at silhouettes; SDF edges get no AA.

**Trade-off — corrected first-order error analysis (critic M2, replaces the original ε² claim):**
The shear makes the effective basis **non-orthonormal**, and `project_to_screen` (deferred_pbr.hlsl:745) inverts by orthonormal dot products. Generation stays exact; the **inverse against the sheared basis accrues a first-order error**. With shear coefficients `sx = (2jx/w)·aspect·tanHalfFov`, `sy = (2jy/h)·tanHalfFov` (at 1080p, 90° FOV, |j| ≤ 0.5 px: `sx = sy ≈ 9.26e-4`):
- View-depth relative error for off-axis points: `Δvz/vz = (vx/vz)·sx + (vy/vz)·sy = nx·aspect·tanHalf·sx + ny·tanHalf·sy` → **≈ 2.6e-3 at the far corners**, → 0 on-axis (second-order only at center).
- Round-trip pixel error of the sheared `project_to_screen ∘ generate_ray` pair: `err_px(nx,ny) ≈ |nx·(nx·A·sx + ny·T·sy)|·w/2` (A = aspect·tanHalf, T = tanHalf) — **≈ 0.5 px at mid-field, up to ≈ 2 px at the extreme corners**, varying per frame with the jitter phase.

Scope of the error: it affects **only same-frame consumers of the sheared world→screen inverse** — i.e., SSCS. TAA's own reprojection (`project_prev`, Decision 4) uses the **prev unjittered orthonormal basis and is exact**. Consumer audit:

| Consumer | Exposure under shear | Verdict |
|---|---|---|
| SSCS march (deferred_pbr.hlsl:784-829) | Samples the depth G-buffer up to ≈2 px off-true at far corners, phase-varying | SAFE-WITH-WATCH: within the IGN-dithered step (~`SSCS_CONTACT_LENGTH/SSCS_STEPS` px) + thickness-floored tolerance; depth *compare* uses the same sheared `dot(·,fwd')` on both sides (consistent scale). **T4 owner-visual item: corner-region SSCS shimmer.** |
| CSM cascade select (view-z based) | `Δvz/vz ≤ 2.6e-3` ⇒ split boundary wobble ≤ 0.26% of split distance | SAFE: orders below the analytic cross-fade band. |
| Froxel/cluster cull (cluster_cull.hlsl:36, :83) | Depth-bin edges shift ≤ 0.26% | SAFE: conservative light-radius binning dominates. **Amend the normalized-fwd comments** (below). |
| SSAO (HBAO-lite on gViewT) | Reconstruction is generation-side (exact); any screen projection inherits the ≤2 px corner class within its sample radius | SAFE (verify the projection use at impl). |
| sdf_depth_composite (mesh↔SDF depth) | Both sides generation-side, same sheared state | EXACT to fp — I2 intact. |
| Tile cull (marcher) | Ray-gen only | EXACT. |

**Required comment amendments** (comment-only ⇒ `.spv` unchanged ⇒ pins safe; routed through the eDSL where the span is emitted): deferred_pbr.hlsl:741, **cluster_cull.hlsl:36 and :83, compute.rs:1350** — the "contractually NORMALIZED" wording becomes "normalized up to a sub-pixel TAA jitter shear (`|s| ≤ ~1e-3`); world→screen round-trip error ≤ ~2 px at frame corners, view-depth relative error ≤ ~2.6e-3 — see docs/TAA-PLAN.md".

Ortho cameras cannot be sheared — **TAA is perspective-only**; the deterministic ortho goldens run TAA-structurally-off forever.

**Risk/mitigation:** raster sign conventions have bitten before (memory: "MVP basis-sign bug"). Mitigation: extend the host ray-golden (`composite_ray` mirror, compute.rs) with jittered cases asserting marcher-sample-pos == raster-sample-pos analytically (generation-side: **exact**, ulp-scale tolerance), plus a one-frame GPU dump at `(jx,jy)=(0.5,0)` showing a uniform half-pixel shift in BOTH mesh and SDF regions. **Host-mirror tolerance spec (critic M2):** the sheared round-trip mirror asserts `error ≤ err_px(nx,ny) + fp margin` from the formula above — NOT exact identity; only the unjittered/prev-basis inverse and all generation-side identities are asserted exact.

### Decision 2: Increment split for motion vectors — camera-only reprojection FIRST, no velocity buffer

**What:** T3 ships full TAA with **camera-only reprojection inside the resolve**: world pos `P = ro + rd·min(view_t, 1e6)` from `gViewT` (sentinel 1e30 → far-point ⇒ rotation-dominant reprojection for sky) + the shared ray reconstruction, then `project_prev(P)` with the **prev-frame unjittered orthonormal** basis → prev pixel → implicit velocity. Per-object velocity (T5) and SDF-edit velocity (T7) come later.

**Why:** (a) The world is static today; camera-only covers 100% of current content. (b) Zero G-buffer changes, zero raster changes, zero Pillar B dependency (green-field). (c) One basis projection per pixel (~12 FMA) beats a dead 16.6 MB/frame velocity round-trip. (d) Ghosting from future movers is bounded by variance clipping (what INSIDE ships for most content).

**Alternatives:** *velocity buffer from day one* — dead bandwidth for a static world + couples to nonexistent Pillar B code. *gViewT.zw reuse* — impossible: gViewT is single-channel `R32_SFLOAT` fully used (targets.rs:71).

**Trade-off:** animated SDF edits and future dynamic meshes ghost until T5/T7, bounded by the clamp to a 3×3 color envelope.

### Decision 3: History = 2× `R16G16B16A16_SFLOAT` on the FIF ring discipline, cross-frame-ordered via a NEW layout-carrying seed (critic B1 folded)

**What:** `taa_hist: [VulkanTexture; FRAMES_IN_FLIGHT]`, `VK_FORMAT_R16G16B16A16_SFLOAT`, usage `STORAGE | SAMPLED | TRANSFER_SRC`. Frame parity `fi` **writes** `taa_hist[fi]`, **reads** `taa_hist[1-fi]`. Lifecycle:

1. **One-shot init:** at creation AND after every swapchain recreate (with `history_valid = false` set first), both slots get a recorded `UNDEFINED → GENERAL` transition — the exact `lit`/`viewt` init precedent (targets.rs:81). This is the ONLY place `UNDEFINED` may ever appear on a history slot.
2. **Per-frame declaration:** both slots enter the graph as **seeded, layout-carrying** resources. The existing constructors pin `layout = UNDEFINED` (sync.rs:104-106, :124-130) — a first-touch content discard that would **destroy history every frame**. Therefore the framegraph gains:

```rust
impl ResSync {
    /// Seed for a VALUE-CARRYING cross-frame resource: content is live at a
    /// pinned `layout` (GENERAL here), last touched by the sibling frame at
    /// `stages`/`access`. Unlike `undefined()`/the B-002 seeds, the layout is
    /// NOT discardable — the state machine must never derive UNDEFINED from it.
    pub const fn seeded_history(layout: i32, stages: u32, access: u32) -> Self;
}
```

   - `hist_read` seed: `GENERAL`, sibling end state = TAA COMPUTE write flushed + made visible to FRAGMENT by the sibling's present sample ⇒ our COMPUTE read derives a visibility-extension barrier, `old == new == GENERAL`.
   - `hist_write` seed: `GENERAL`, last touched by the sibling's TAA COMPUTE *read* (+ the N-2 FRAGMENT present read, already CPU-fence-drained) ⇒ WAR, execution-only src.
   - **Contract amendment at sync.rs:104-106:** the "reset to `undefined()` each compile — prior content is discarded" doc contract is narrowed to transient resources; history-class resources seed via `seeded_history` and are value-preserving.
3. **Record time:** two ResIds (`hist_read`, `hist_write`) bound to physical slots `[1-fi]`/`[fi]` by the existing `[fi]`-resolver (sync.rs:14-16) — no other API changes.

**Why:** history is a class the framegraph hasn't had — **ringed AND value-carrying**. Today's ringed slots are frame-private (no seeds); today's seeded resources are re-written each frame (discardable `UNDEFINED` start is fine). The two mechanisms compose through `add_image_seeded` (graph.rs:145) + record-time binding, but ONLY with the new layout-carrying seed — hence `seeded_history`. The C1 keystone (the fence drains N-2, not the sibling N-1) is exactly why the sibling read/write must be barrier-seeded.

**Enforcement (I5):** the equiv test pins the `hist_read` in-barrier as `old == new == GENERAL`, src = FRAGMENT (or execution-only 0-access) — **any derived `UNDEFINED` on `hist_read` FAILS the test.**

**Format decision table:**

| Format | B/px | Verdict |
|---|---|---|
| `R16G16B16A16_SFLOAT` | 8 | **CHOSEN.** The feedback loop re-quantizes every frame; 16F never deadbands. Bevy ships exactly this. Alpha lane free for a future confidence term. |
| `R8G8B8A8_UNORM` (match `lit`) | 4 | Rejected: 8-bit feedback deadband — deltas under the blend factor round to zero ⇒ stuck/shimmering pixels. |
| `B10G11R11` / `RGB10A2` | 4 | Rejected: same deadband class at 10/11 bits under α = 0.03-0.1; no alpha lane. Revisit only if 33 MB VRAM matters. |

Fail-fast: `taa_history_format_ok` device check (STORAGE + SAMPLED on 16F, OPTIMAL) per the device.rs:1950-2008 precedent — evaluated only when TAA is enabled. (Present samples with the NEAREST sampler — no LINEAR requirement from the present side; the TAA pass's own history sampler needs LINEAR on 16F, checked in the same probe.)

**Trade-off:** +33.2 MB VRAM when ON; present reads 8 B/px instead of 4. Both zero when OFF.

### Decision 4: The resolve — new standalone compute shader `taa_resolve.hlsl`; projection body eDSL-emitted (critic M4); YCoCg variance clipping γ=1.0; 5-tap Catmull-Rom; velocity-modulated blend

**What:** One 8×8 compute pass, own descriptor set (deferred-resolve set at 16/16 — SSAO own-set precedent), own 96 B UBO. Per pixel:
1. Load current `lit` texel direct (**no unjitter resample** — Playdead/Bevy practice; the clamp absorbs the sub-pixel walk).
2. 3×3 neighborhood → YCoCg μ/σ → AABB = μ ± γσ, γ = 1.0.
3. Reconstruct `P` from `gViewT` + shared ray math; `project_prev(P)` with the prev **unjittered orthonormal** basis (exact inverse — no shear error here) → prev UV. History accumulates in unjittered space; this reprojection IS the un-jittering, done once.
4. Sample history at prev UV, Catmull-Rom (5 bilinear taps, LINEAR).
5. Clip history toward the AABB center (Salvi).
6. `k = lerp(k_max=0.97, k_min=0.88, saturate(|v_px|/40))` (Playdead); `out = lerp(current, clipped_history, k · confidence)`; `confidence = 0` on reset frames and off-screen prev UV.
7. Sanitize (`clamp(out, 0, 64)`) — NaN/Inf must never enter the feedback loop — store to `taa_hist[fi]`.

Present: the ONCE-written present set (targets.rs:125-128) is written to `taa_hist[fi]` instead of `lit[fi]` **at creation** when ON — structural rebind, zero runtime cost, present shader unchanged (COMBINED_IMAGE_SAMPLER + `Texture2D`/float4 sample type is format-agnostic between UNORM8 and SFLOAT16; no SRGB conversion anywhere in the chain). TAA operates post-tonemap LDR (`lit` is `R8G8B8A8_UNORM`): Karis-endorsed; revisit if `lit` ever goes HDR (§Open 8).

**Authorship (critic M4, decided):** `taa_resolve.hlsl` is a NEW TU (zero existing pins), but the **world→screen projection body is emitted by the eDSL from ONE source** — the same source of truth as the projection math family — into the new TU, with its **own cmp-`.spv` pin**; `deferred_pbr.hlsl` is **untouched** (its frozen artifact keeps its pins). The eDSL Eval CPU mirror of that emission doubles as the host-mirror test. Drift between `project_prev` and the canonical inverse is thereby structurally impossible, not merely tested-for. The rest of the TU (clip/blend/Catmull-Rom) is hand HLSL per the new-file rule; ray reconstruction comes from `#include ray_gen.hlsli`.

**Alternatives:** *bilinear history* — measurable blur accumulation (the reason bicubic exists — Karis). *Depth-based history rejection* — nothing ships it; variance clipping subsumes it for our content. *Min/max clamp* — worse ghosting/flicker frontier, same cost. *Extend the deferred resolve* — descriptor cap, eDSL pin exposure, couples the OFF gate to an eDSL-owned shader. *Hand-written `project_prev` + tolerance mirror (the pre-critic choice)* — rejected for M4's structural guarantee at ~equal effort.

**Trade-off:** γ=1.0 + k≤0.97 favors stability slightly over sharpness (owner knob, §Open 1). T3 acquires one eDSL emission + pin (was zero).

### Decision 5: The OFF gate is creation-time structural

**What:** `TaaConfig` is a **construction parameter** (alongside the existing targets/ssao config) — there is NO runtime `set_taa` in v1 (critic m8: nothing to enforce ordering against; the "pre-first-frame only" rule holds by construction). OFF ⇒ (a) history images never created; (b) `declare_gbuffer_graph` declares no TAA pass/resources ⇒ derived plan bit-equal ⇒ the 23/10/22 pins stand unmodified; (c) present set written to `lit[fi]` by the existing untouched code; (d) the camera write site never executes the shear (structural skip, Decision 1); (e) all existing `.spv` artifacts untouched on disk. The only per-frame TAA presence check is the config-gated declare function that already exists for SSAO/CSM optionals (W5 discipline).

**Why:** the only mechanism that *proves* byte-identity rather than argues it — the proof artifacts are the existing harnesses, which fail loudly on any deviation.

---

## Data structures

```rust
// crates/boyko_rhi_vulkan/src/present/taa.rs (NEW)

/// Halton(2,3) 8-cycle, centered to [-0.5, 0.5) pixels. Compile-time const;
/// index = submitted_frame_serial & 7 (advances ONLY on submitted frames — m7).
pub const TAA_JITTER: [[f32; 2]; 8] = [ /* halton(2,3)[1..=8] - 0.5 */ ];

/// Construction-time config (scene/viewer flag). None = structurally absent.
#[derive(Clone, Copy)]
pub struct TaaConfig {
    pub gamma: f32,   // variance-clip γ, default 1.0
    pub k_min: f32,   // feedback at high velocity, default 0.88
    pub k_max: f32,   // feedback at rest, default 0.97
}

pub(crate) struct TaaTargets {
    /// History ring: written [fi], read [1-fi]. R16G16B16A16_SFLOAT,
    /// STORAGE | SAMPLED | TRANSFER_SRC, GENERAL whole-life after the one-shot
    /// UNDEFINED→GENERAL init (re-recorded after every recreate). 16.6 MB/slot @1080p.
    pub hist: [VulkanTexture; FRAMES_IN_FLIGHT],
    /// TAA UBO ring, one 96 B slot per in-flight frame (TaaUniform), min-align padded.
    pub ubo: VulkanBuffer,
    /// Own descriptor set ring (resolve set is at 16/16): @0 lit[fi] SRV,
    /// @1 hist[1-fi] SRV + LINEAR sampler, @2 gViewT[fi] SRV, @3 hist[fi] STORAGE,
    /// @4 ubo slot, (@5 velocity[fi] SRV — T5). Written ONCE at creation.
    pub set: [VkDescriptorSet; FRAMES_IN_FLIGHT],
    pub pipeline: VkPipeline,          // taa_resolve.comp
    /// False ⇒ resolve runs with confidence=0 (output = current). Cleared at
    /// creation, on recreate, and by taa_reset().
    pub history_valid: bool,
    /// Prev-frame UNJITTERED camera basis + the serial of the SUBMITTED frame
    /// that produced it (m7): prev_cam advances only after a successful queue
    /// submit; early-outs (OUT_OF_DATE, skip) leave it untouched.
    pub prev_cam: TaaCamBasis,         // 64 B
    pub prev_cam_serial: u64,
}

/// 96 B, #[repr(C)], all-float4-aligned (std140-compatible).
#[repr(C)]
pub(crate) struct TaaUniform {
    pub prev_eye:       [f32; 4], // xyz prev eye,                w = γ
    pub prev_fwd:       [f32; 4], // xyz UNJITTERED prev forward, w = tan(fovY/2)
    pub prev_right:     [f32; 4], // xyz prev right,              w = aspect
    pub prev_up:        [f32; 4], // xyz prev up,                 w = confidence (0 on reset)
    pub jitter_kminmax: [f32; 4], // xy curr jitter px, z = k_min, w = k_max
    pub extent:         [f32; 4], // xy = w,h  zw = 1/w,1/h
}
// One 96 B write per frame = the entire per-frame CPU cost besides the shear (I3).
```

Velocity (T5 only): `taa_velocity: [VulkanTexture; FRAMES_IN_FLIGHT]`, `VK_FORMAT_R16G16_SFLOAT` (4 B/px, 8.3 MB/slot; fp16 ulp at |v|=64 px is 0.03 px — sufficient), cleared to `(3e4, 3e4)` sentinel per frame; raster 4th MRT overwrites mesh pixels; resolve selects `use_vel = vel.x < 1e4` (warp-coherent branch — mesh/SDF regions are spatially coherent).

No hot/cold split needed: `TaaTargets` is touched once per frame at declare/record, not in a loop; render loop is single-threaded (no false-sharing surface).

## Public API

```rust
// boyko_rhi_vulkan — construction-time opt-in (m8: no runtime setter in v1;
// ordering enforced by construction, nothing to assert).
pub struct RendererConfig { /* existing fields…, */ pub taa: Option<TaaConfig> }

impl Renderer {
    /// Camera cut / teleport: zero history confidence for the next frame.
    /// No-op when TAA is off.
    pub fn taa_reset(&mut self);
}
// Viewer/scene flag: `taa = on|off` → RendererConfig.taa.
// Everything else pub(crate).
```

## Algorithms for critical paths

**CPU per frame (TAA ON):** `j = TAA_JITTER[serial & 7]` → shear `fwd'` (9 FMA) → raster proj offset (2 adds) → write 96 B `TaaUniform` → **after successful submit**: `prev_cam = unjittered basis; prev_cam_serial = serial` (m7). O(1), no branches beyond `Option` presence, zero allocations. `debug_assert!(taa.history_valid → taa.prev_cam_serial + 1 == serial)` — history is only trusted if the previous SUBMITTED frame produced `prev_cam`.

**GPU resolve (2.07 Mpx, 8×8 groups = 240×135):** per pixel — 9 `lit` loads (L2-coherent), 1 `gViewT` load, ray reconstruct + `project_prev` (~25 FMA), 5 bilinear history taps, YCoCg moments (~60 FMA), clip + blend (~20 FMA). Streaming access; divergence only at off-screen-prev-UV (edge columns) and the T5 velocity select (warp-coherent). No LDS needed (L2 covers the 3×3 halo at 4 B/px). Trivially wave-vectorized.

**Bandwidth budget @1080p (unique bytes × reuse):** lit 8.3 MB×1.3 + hist-read 16.6 MB×1.4 + gViewT 8.3 MB + hist-write 16.6 MB ≈ **59 MB** ⇒ 0.24 ms floor at a **conservative 250 GB/s effective** (m6) ⇒ **estimate 0.30-0.45 ms, gate 0.5 ms** (consistent with Intel's 0.9-1.9 ms at ~4× less bandwidth). Present delta: +8.3 MB. T5: +~0.03 ms + 8.3 MB read.

## Multithreading model

- CPU: render loop single-threaded (house invariant). TAA adds a `u64` serial and a bool — no shared state, no atomics, no `Send`/`Sync` changes.
- GPU cross-frame: the only new hazards are sibling-frame ones on `taa_hist` (the fence drains N-2, not N-1 — C1). Race-freedom proof: frame N's READ of `hist[1-fi]` is ordered after N-1's WRITE by the seeded RAW barrier (seed carries the sibling's flushed COMPUTE write at `GENERAL`; the machine emits visibility-to-COMPUTE, `old == new == GENERAL` — never a discard, I5); frame N's WRITE of `hist[fi]` is ordered after N-1's COMPUTE read by the seeded WAR execution-only barrier (N-2 touches are fence-drained). Same-queue submission order + these barriers = a total order on every slot access — the audited B-002/B-003 mechanism plus the B1 layout-carrying seed.
- Within-frame: lit→TAA (RAW), TAA→present (RAW) derived by the untouched state machine.

## Interaction with the PCF/scintillation track

| Item | TAA relationship | Action |
|---|---|---|
| Hard-edge resampling scintillation (AUDIT-2026-07-PLAN.md:36 — the S2/S3 escalation driver) | **Superseded** — jitter+accumulation is the canonical fix; TAA's primary payoff. | T4 golden demonstrates it. |
| PCF penumbrae | **Complementary** — PCF filters within-frame; TAA removes residual shadow-edge crawl. PCF stays. | None. |
| SDF analytic/SSCS contact shadows + IGN dither (deferred_pbr.hlsl:735, :798) | Static dither today. With TAA ON, a temporal IGN term (`+5.588238·(frame & 63)`) makes the dither **resolve to ground truth**. Separately: the M2 shear error adds phase-varying ≤2 px SSCS sample offsets at corners. | T6 (opt-in, eDSL ON-variant + pin; OFF artifact frozen). **T4 owner-visual item: SSCS corner shimmer under jitter.** |
| SSAO blue-noise (Hilbert+R2) | R2 temporal rotation converges AO when TAA ON. | T6, same gating. |
| Oct-normal quantization speckle | **Complemented, not superseded**: jitter decorrelates the quantized-normal lighting error per frame; TAA averages it. Encoding untouched. | None. |
| Analytic cascade cross-fade | **Untouched** — never assumed temporal accumulation. Do NOT convert to dithered fade. | Explicit non-goal. |

When TAA is OFF, every convention above is bit-unchanged (T6 variants are separate artifacts selected only when ON).

## OFF gate — proof mechanism and test plan

Mechanism: Decision 5 (construction-time structural absence). Proof artifacts:
1. **Framegraph equiv extension** (tests/framegraph_gbuffer_equiv.rs): OFF ⇒ the pinned 23 img + 10 buf + 22 calls assert with the TAA code merged. ON ⇒ new pins: +1 pass, +2 seeded image ResIds, **+3 new image barriers + 1 changed dst** (m5): (i) `hist_read` in — RAW, `GENERAL→GENERAL`, src FRAGMENT/seed; (ii) `hist_write` in — WAR, execution-only, `GENERAL→GENERAL`; (iii) `hist_write`→present — RAW flush COMPUTE→FRAGMENT; (iv) the **existing** lit resolve→consumer barrier's dst changes FRAGMENT→COMPUTE (present no longer samples lit). Each pinned line-by-line like the B-002 five. **I5 pin: any `UNDEFINED` old-layout on a history slot outside the one-shot init FAILS.**
2. **Windowed byte-gate for OFF:** re-run the 20-config SHA256 dump harness with `taa=off` → all hashes equal the pre-TAA baselines. The 0%-gate verbatim.
3. **Jitter-determinism golden for ON:** scripted camera path, fixed seed, K=32 frames, serial-indexed jitter, reset-on-start ⇒ frame-31 dump SHA256 equal across two runs (compute + one fullscreen pass = GPU-deterministic per driver).
4. **T2 plumbing gate — exactness chain stated explicitly (critic M3):** passthrough resolve (copy lit→hist) with jitter **structurally absent** must hash-equal the OFF frame. This is exact because (a) the present sampler is **NEAREST** at 1:1 (window_present_gbuffer.rs:1685-1695) and (b) **u8-UNORM → f16 (RTE) → u8 round-trips exactly for all 256 values**. Both facts get their own guards: a 256-value CPU round-trip mirror test, and an assert that the present sampler filter is NEAREST (so a future LINEAR flip can't silently rot the gate's premise).
5. **CPU mirrors:** eDSL Eval mirror of the emitted projection body — `project_prev(unjittered) ∘ reconstruct == id` to a few fp32 ulps (exact case); the sheared round-trip mirror asserts `error ≤ err_px(nx,ny)` from the M2 formula (first-order case — NOT exact); YCoCg variance-clip mirror; Halton-table golden.
6. `debug_assert!`s: jitter index < 8; `history_valid ⇒ prev_cam_serial == last_submitted_serial - 1` (m7); UBO slot write ≤ 96 B; ON ⇒ `camera_mode == PERSPECTIVE`.

## Integration

- **New:** `present/taa.rs`; `shaders/taa_resolve.hlsl` (+ committed `.spv`; the projection body eDSL-emitted with its own pin — M4); `taa_history_format_ok` device probe; viewer `taa` flag.
- **Touched:** `framegraph/sync.rs` — **new `ResSync::seeded_history` constructor + the :104-106 contract amendment** (B1); the single camera write site (structural shear under the flag; compute.rs M4-block write path); the raster VP build site (proj offset under the flag); `declare_gbuffer_graph` + record sink (TAA pass + 2 seeded images, config-gated like SSAO); present-set creation (bind `taa_hist[fi]` when ON); the targets create/recreate path (history ring + one-shot `UNDEFINED→GENERAL` init + `history_valid=false` at both choke points); comment amendments deferred_pbr.hlsl:741, cluster_cull.hlsl:36/:83, compute.rs:1350 (comment-only, `.spv` unchanged, via eDSL where emitted).
- **Not touched:** every existing `.spv` artifact; all existing eDSL pins (T0-T2/T4: zero; **T3: one new emission + pin** — M4; T5: one variant pin if gbuffer_mrt's oct-pack span is eDSL-emitted — verify at impl; T6: yes by design); `boyko_ecs`; Pillar B (T5 consumes it when it exists).

## Implementation plan (increments — each independently green)

| Inc | Content | eDSL | Gate | Effort | GPU cost |
|---|---|---|---|---|---|
| **T0** | `RendererConfig.taa` plumbing, `Option<TaaTargets>` skeleton, equiv-pin re-run | none | byte-gates 1+2 green | 0.5 d | 0 |
| **T1** | Halton table + structural basis shear + raster proj offset; host ray-golden jittered cases (exact, generation-side) + sheared-round-trip tolerance mirror | none | goldens + half-pixel-shift GPU dump | 1 d | 0 |
| **T2** | `ResSync::seeded_history` + sync.rs contract amendment (B1); history ring + one-shot init + seeded declare + record binding + present rebind + passthrough resolve + reset/resize wiring; 256-value u8↔f16 mirror + NEAREST-sampler assert | none | passthrough (jitter absent) hash == OFF hash; ON equiv pins incl. the I5 no-UNDEFINED pin | 2-2.5 d | ~0.1 ms |
| **T3** | Full resolve: variance clip γ=1.0, Catmull-Rom×5, eDSL-emitted `project_prev` + pin, velocity-modulated k, sky sentinel, sanitize; CPU mirrors | **1 emission + pin** | mirrors + gate 3 + **timestamp ≤ 0.5 ms** | 2.5-3.5 d | 0.30-0.45 ms |
| **T4** | Determinism + convergence goldens; owner visual session (RTX, `BOYKO_DISABLE_VALIDATION=1`) incl. **SSCS corner-shimmer item** | none | two-run SHA equal; owner = visual oracle | 1 d | — |
| **T5** | *(after Pillar B)* R16G16F velocity MRT variants (`-DTAA_VELOCITY` vs/fs `.spv`s), prev-MVP from `GpuTransform3D.prev`, resolve velocity-select | 1 variant pin (verify) | moving-mesh ghosting golden | 2 d | +0.03 ms |
| **T6** | *(owner-gated)* temporal IGN/SSAO-R2 ON-variants | yes (pinned variants) | OFF artifacts frozen; visual | 1-2 d | 0 |
| **T7** | *(only if T5 shows objectionable SDF-edit ghosting)* dominant-edit velocity from the marcher | yes | visual | — | — |

Core (T0-T4): **7-8.5 days**. T5 blocked on Pillar B; T6/T7 owner-gated.

## Metrics and validation

GPU timestamp around the TAA dispatch (≤ 0.5 ms @1080p/3060 gate; 0.24 ms bandwidth floor at 250 GB/s); end-to-end ON-vs-OFF frame delta (≤ 0.5 ms + present delta); OFF delta == 0 **by hash**, not by measurement. Tests: §OFF-gate 1-6 + T4 goldens. Miri: no new unsafe beyond FFI image creation following the audited `create_gbuffer_image` pattern (every block carries SAFETY). Invariants: the four `debug_assert!`s + the equiv barrier pins + I5.

## Open questions

**VALUES (owner):**
1. **Sharpness vs stability default:** γ=1.0 / k_max=0.97 (recommended, Bevy-like) vs UE-like softer α≈0.04-equivalent. Exposed in `TaaConfig`; pick after the T4 visual.
2. **Viewer default:** recommend OFF until the T4 owner visual, then flip ON (matches "commit render changes only after visual OK").
3. **T6 (temporal noise re-index):** take it once the TAA baseline is seen, or leave dither static?
4. Post-TAA **sharpen pass** (RCAS-style, +~0.1 ms): roadmap or no?
5. Confirm **perspective-only** TAA is acceptable (ortho goldens/test cameras structurally excluded).

**TECHNICAL (resolved with the critic, retained for the developer):**
6. `seeded_history` composition for `hist_write` (sibling COMPUTE read WAR) — the state machine's WAR path is expected to need no `visible_stages` widening beyond the seed; the I5 equiv pin catches any deviation.
7. Whether `gbuffer_mrt.fs`'s oct-pack span is eDSL-emitted (decides if T5's variant needs an eDSL emission target or a `-D` recompile + pin).
8. `lit` staying LDR: if the deferred resolve ever goes HDR, TAA moves pre-tonemap with luma-weighted blending — coupled future decision.

## Changelog (critic round 1 → this revision)

- **B1 (blocker, folded):** Decision 3 rebuilt around a NEW layout-carrying `ResSync::seeded_history(layout, stages, access)` — the existing seeds pin `layout=UNDEFINED` = per-frame content discard, which would have destroyed history. Added: sync.rs:104-106 contract amendment, one-shot `UNDEFINED→GENERAL` init at creation AND after every recreate (with `history_valid=false` first), new invariant I5 + an equiv-test pin that FAILS on any derived `UNDEFINED` for `hist_read` (`old==new==GENERAL`, src FRAGMENT/0 pinned).
- **M2 (folded):** the shear trade-off analysis replaced — original ε² claim was wrong by 4-7 orders; corrected first-order bounds: `Δvz/vz ≈ 2.6e-3` at corners, round-trip ≈0.5 px mid-field / ≈2 px corners at 90° FOV/1080p, with the `err_px(nx,ny)` formula. Added the consumer-audit table (SSCS / CSM-select / froxel / SSAO / composite / tile-cull), extended comment amendments to cluster_cull.hlsl:36/:83 + compute.rs:1350, fixed the host-mirror tolerance spec (sheared round-trip asserts the first-order bound, NOT identity), added the SSCS corner-shimmer item to the T4 owner visual.
- **M3 (folded):** the T2 passthrough gate now states its two load-bearing exactness facts (NEAREST present sampler per window_present_gbuffer.rs:1685-1695; u8→f16(RTE)→u8 identity for all 256 values) and adds both guards (256-value CPU mirror, sampler-filter assert). Jitter=0 is specified as a **structural skip** of the shear (`Option` presence) — a computed `-0.0` would byte-change the UBO.
- **M4 (folded):** `project_prev` authorship decided — the projection body is eDSL-emitted from one source into the new TU with its own pin; `deferred_pbr.hlsl` untouched. T3's eDSL column changed from "none" to "1 emission + pin"; the eDSL Eval mirror doubles as the host mirror.
- **m5:** ON-pin barrier accounting corrected to **+3 new image barriers + 1 changed dst** (lit's consumer dst FRAGMENT→COMPUTE), not "+4".
- **m6:** bandwidth math restated at a conservative 250 GB/s effective → 0.24 ms floor, 0.30-0.45 ms estimate; the 0.5 ms gate stands.
- **m7:** `prev_cam` (and the jitter serial) advance **only on successfully submitted frames**; early-outs leave them untouched; `debug_assert!(history_valid ⇒ prev_cam_serial == last_submitted_serial - 1)` added.
- **m8:** runtime `set_taa` removed — `TaaConfig` is a `RendererConfig` construction parameter (ordering enforced by construction); `taa_reset()` remains the only runtime call.
- Effort updated: T2 2-2.5 d (B1 work), T3 2.5-3.5 d (M4 emission); core total 7-8.5 d.
