# Render campaign — Anti-Aliasing (all modern types) + rendering tails

**Owner mandate (2026-07-12, autonomous overnight):** implement all modern anti-aliasing
techniques and finish the outstanding rendering tails. Detailed plan first, then execute
autonomously, **commit + push per stage**. Owner will not visually check — the orchestrator
is the delegated visual oracle. Branch `feat/shadow-denoise-and-ssao-blur` (feature branch,
not `master` — nothing reaches `master` without an owner-controlled merge).

---

## 1. Architecture grounding (verified by reading the present pipeline)

The renderer is **deferred**:

```
gbuffer raster (mesh MRT) ─┐
                           ├─► SDF marcher (compute) ─► deferred_pbr resolve (compute)
SDF field ─────────────────┘                              │  writes lit[fi]  (STORAGE)
                                                           ▼
                                     present-blit: fullscreen-sample lit[fi] ─► swapchain
                                                    (+ optional UI rect subpass)
```

* The deferred resolve's output is the **`lit[fi]` STORAGE image ring** (`present/targets.rs`).
* The present-blit (`present/passes/present_blit.rs` / `gbuffer.rs` pass C) **fullscreen-samples
  `lit[fi]`** via the `fullscreen_sample.vs/fs.hlsl` triangle → swapchain.
* **The `resolve → present-blit` seam is the post-process AA insertion point.** A post-process
  pass reads `lit[fi]`, writes an AA target, and the present samples the AA target instead.

### Byte-identity discipline (INVIOLABLE)
Golden = SHA-256 of the readback BMP. Every AA mode must be **OFF by default** and the OFF path
must stay **byte-identical** to the current goldens (`grand_showcase a5ad662d`,
`grand_showcase_2mat f6147f90`, the UI/rect/text goldens). The template is the SSAO / shadow-
denoise **`Option`-guarded activation**: `None` ⇒ no pass, no target, present samples `lit` exactly
as today. Arming is **sync-static** (decided at `sync_gbuffer`, re-synced on extent change) from an
ECS-native config Resource — matching `BOYKO_SHADOW_DENOISE`'s boot-static model; no per-frame
target re-selection, so the OFF stream is provably unchanged.

### Config pattern (Principle 0 — ECS-native)
Mirror `SsaoConfig` exactly:
* `AaConfig { mode: AaMode }` — owner-set `#[derive(Resource)]` singleton (default `Off`).
* `resolve_aa_policy` — cold single-writer system mapping `AaConfig → ResolvedAa`.
* `ResolvedAa { variant: Option<..>, .. }` — derived carrier the render driver reads.
* `AaPlugin` — registers config + policy + resource.
No `std::Vec`/`HashMap`/`dyn`; capability is structural (`mode != Off`), no redundant `bool`.

### Shader regime
The eDSL (`boyko_shaderdsl`) owns **only** the SDF marcher family + SSAO + interp. Post-process,
present, and raster shaders are **hand-authored HLSL** (`fullscreen_sample`, `deferred_pbr`,
`gbuffer_mrt`), dxc-compiled, `.spv` hand-committed, with `-D` preprocessor variants when a shared
base must stay byte-frozen. **AA kernels are new hand-authored HLSL post-process shaders** — the
correct regime; the eDSL feedback rule does not apply to them.

### Verification (orchestrator = delegated oracle)
1. **OFF path** — automated golden gates stay byte-identical (the hard gate every stage passes).
2. **`.spv` byte-identity** — recompile, `cmp` frozen bases to prove additive/`-D` changes don't
   perturb them.
3. **ON path** — orchestrator renders a windowed dump BMP with the mode on, inspects it (edges
   smoothed, no NaN/garbage), and **pins a NEW golden hash** for that mode so future regressions
   are caught. (Subagents cannot run fresh GPU exes — os-740 — so the GPU loop is the orchestrator's.)

---

## 2. AA technique set — "all actual/modern types"

| Mode  | Class            | Inputs                                   | Passes | Notes |
|-------|------------------|------------------------------------------|--------|-------|
| **FXAA** | spatial, cheap   | `lit` only                               | 1      | luma-edge; no history/jitter/MV. First — proves the whole framework. |
| **SMAA 1x** | spatial, quality | `lit` + Area/Search LUTs                  | 3      | edge-detect → blend-weights → neighborhood-blend. Higher quality, still single-frame. |
| **TAA** | temporal         | `lit` + motion vectors + color history + sub-pixel jitter | 1 (+jitter) | reprojection + neighborhood clamp. Highest quality; reuses the existing MV/jitter/history infra (un-walled from `hwrt`). |
| **SSAA 2×** | supersample      | render pipeline at 2× extent, box downsample | +1 downsample | brute-force quality mode; touches target-extent sizing. |
| ~~MSAA~~ | ~~hardware MSAA~~ | — | — | **Intentionally excluded**: MSAA on a deferred/compute-resolve renderer needs per-sample G-buffers + per-sample lighting (prohibitive), and does not fit the single-sample compute resolve. Documented, not implemented. |

This is the complete standard modern set: cheap-spatial (FXAA), quality-spatial (SMAA),
temporal (TAA), brute (SSAA), with MSAA reasoned-out.

---

## 3. Stages (each: architect where needed → critic → developer → code-reviewer → orchestrator GPU
oracle + golden gate → **commit + push**)

* **Stage 0 — base commit.** Commit the completed, gated textured-PBR campaign + groom pass
  (done, reviewed, byte-identity held). Clears the tree so AA work is atomic. Validate the GPU
  golden harness runs for the orchestrator (feasibility gate for the whole night). Push.
* **Stage 1 — AA framework + FXAA.** `AaMode`/`AaConfig`/`ResolvedAa`/`resolve_aa_policy`/`AaPlugin`
  + `Option`-guarded post-process pass slot + `aa_out` target ring + present-set selection +
  sync-static arming + the FXAA HLSL kernel + `.spv`. OFF byte-identical; FXAA-on golden pinned.
* **Stage 2 — SMAA 1x.** 3-pass; Area/Search LUTs embedded; two intermediate targets. New golden.
* **Stage 3 — TAA.** Un-wall camera sub-pixel jitter + motion vectors + color-history ring;
  reproject + variance/neighborhood clamp. The hard one — architect + critic in the loop. New golden.
* **Stage 4 — SSAA 2×.** Scaled-extent render + box downsample pass. New golden.
* **Stage 5+ — coupled render tails** (see §4), each its own commit.

Commits are author-only (Celtokisa), no AI markers, no `--force`/`--no-verify`.

---

## 4. Rendering tails — in-scope vs deferred (honest disposition)

**In scope (coupled to this campaign / genuinely completable to a shippable bar):**
* The **post-process framework** itself — the render audit flagged "NO post"; AA delivers it.
* **Wire dormant configs to live consumers.** `ssao_config.rs` states the SSAO Resource+policy+plugin
  exist but "the deferred-render ECS system that READS `ResolvedSsao` each frame … is the explicit
  larger follow-up." Completing that live wiring is a concrete, well-scoped tail.
* **TAA** also discharges the long-standing "TAA denoise" HW-RT follow-up (temporal color AA).

**Explicitly deferred (large independent features — half-implementing them overnight without owner
design input would violate "clean architecture the first time" + "fix bugs before features"):**
* SSR / screen-space reflections, volumetrics, particles, GPU skinning, transparency/OIT, upscaling
  (FSR/DLSS-class), occlusion culling. These were previously dispositioned/deferred by the owner and
  each warrants its own architect→critic cycle. Enumerated here so the owner sees they were
  considered and consciously left for a design pass, not forgotten.

---

## 5. Risk register
* **GPU harness may not run headless here** → validated in Stage 0; if it can't, ON-path visual
  verification is flagged for the owner and the stage still ships OFF-path-golden + review + `.spv`
  byte-identity proven.
* **Byte-identity drift** → every stage runs the full golden gate before commit; `.spv` bases `cmp`'d.
* **TAA convergence/ghosting** → neighborhood clamp + disocclusion reject; static-camera anchor
  (MV≡0) verified; the existing temporal-shadow infra is the proven precedent.
* **Scope creep into deferred features** → §4 boundary is firm; deferred features are not started.
