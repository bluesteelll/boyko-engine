# VG R3 — HZB and two-pass occlusion culling

Status: **DESIGNED, NOT APPROVED FOR IMPLEMENTATION.** The soundness question is settled. The
implementation design has been reviewed twice and returned REJECTED with 8 blockers, listed at the
bottom; it needs one more revision round before the first file is touched.

---

## 1. The soundness question, and its answer

The cull runs *before* the raster, so no same-frame depth pyramid exists when the cull needs one.
Reading the previous frame's pyramid is not conservative on its own: an instance emerging from
behind an occluder can be rejected, and it vanishes. Every golden in this tree is a static scene and
cannot show that.

**The answer is two-pass, and the soundness lives entirely in the second pass.**

> **Theorem (same-frame occlusion soundness, reverse-Z).** Let `F` be the frustum survivors, `R1`
> whatever the early pass rasterized (ANY subset of `F`), `D` the depth buffer after `R1`, and
> `H` the min-reduced pyramid over `D`. The late pass tests each `i ∈ F \ R1` and rejects iff
> `depth_near(i) < occ(i)`, where `occ(i)` is the min over the sampled texels of `H`, and COVERAGE
> holds: the preimage in `D` of those texels contains `i`'s screen rect. Then **every truly visible
> `i` is drawn.**
>
> *Proof.* Take `i` visible: some pixel `p` in its footprint has `i`'s surface frontmost in the whole
> scene. `D` holds a SUBSET of the scene, so under reverse-Z (larger = nearer) `d_i(p) ≥ D[p]`. And
> `d_i(p)` is a surface of `i` inside its bound, so `d_i(p) ≤ depth_near(i)`. By COVERAGE, `p` lies
> in the preimage of a sampled texel, and `occ` is a min over that preimage, so
> `occ(i) ≤ D[p] ≤ d_i(p) ≤ depth_near(i)`. The reject predicate `depth_near(i) < occ(i)` is
> therefore false. `i` is kept, then drawn. ∎

**The proof quantifies over ALL `R1`, and that is the whole design.** The early pass is an
*unverified heuristic* whose only job is to fill `D` with a good occluder set. It may test against a
previous-frame pyramid, a stale bit, or nothing at all — its mistakes cost late-pass work and never
cost geometry. So "a previous-frame pyramid is not conservative" is true and **harmless**: it is
never the last word.

This is what every shipping implementation does, and the practice was confirmed against sources
rather than assumed: UE5 Nanite (main pass on the previous HZB, `BuildPreviousOccluderHZB`, post
pass re-tests everything the main pass rejected against the current HZB), Assassin's Creed Unity
(SIGGRAPH 2015: phase 1 culls on last frame's pyramid and renders; phase 2 refreshes the pyramid,
re-tests the culled and renders the false negatives), Granite, Bevy 0.16's four-step GPU two-phase
occlusion, and Unity 6's GPU Resident Drawer (expressed as an OR over both frames' depth).

⚠️ **Unity's own documentation warns the technique can be a NET LOSS** when a scene has little
occlusion, because the GPU pays for the extra pass regardless. That is directly relevant here — see
§4.

## 2. What the engine is missing

Three real gaps, each verified in-tree:

- **No per-mip image views.** `VulkanTexture` owns `view` (full subresource), `layer_views`
  (per array layer) and `array_view`; every view is created with `base_mip_level: 0`, and there is
  no API producing `base_mip_level = k, level_count = 1`. A mipped array texture is additionally
  forbidden by a `debug_assert`.
- **The framegraph tracks sync per-ResId, not per-subresource.** `transition()` takes no subresource
  argument; `SubRange` is copied verbatim into the emitted barrier and never consulted by the state
  machine.
- **No min/max reduction sampler.** `sampler_filter_minmax` exists only as a feature bit; there is
  no `VkSamplerReductionModeCreateInfo` plumbing, so the one-`texture()`-call 2×2 min that niagara
  uses is unavailable without new RHI surface.

## 3. The shape of the work

Nine steps, each intended to build green and commit alone, following R2d's inert-then-arm
discipline:

| | |
|---|---|
| **S1** | RHI: first-class `TextureView` — per-mip, per-layer, format-reinterpreting |
| **S2** | Framegraph: a mechanical trigger guard for per-subresource sync |
| **S3** | Host oracle and the proofs as property tests, **before any shader exists** |
| **S4** | The pyramid: images, per-mip views, two build passes — written and read by nobody |
| **S5** | Pyramid gates: `.spv` census against the BUILT modules + GPU-vs-oracle at an ODD extent |
| **S6** | Inert cull rung: reject tail + batch state + pyramid loaded, verdict discarded |
| **S7** | The late pass fully dispatched with `r == 0` everywhere — the null control |
| **S8** | **The arming**, plus the gate that can see a false reject |
| **S9** | The R5/R6 meshlet layout dividend, shipped inert |

## 4. What may and may not be claimed

**No occlusion perf claim is measurable in this repository.** The VG corpus is a triangle-density
instrument; its measured occlusion ceiling is **1 of 44 drawn instances** at the binding camera path
(`orbit_mid`) and 11 of 31 at `approach_close`, and those bound more than occlusion since a
sub-pixel instance also wins zero pixels. Combined with Unity's net-loss warning, this rung ships
**correctness-gated with no speed claim**, exactly as R2d shipped structural. The engine is
general-purpose and the feature pays on content this fixture does not have; that is a statement
about the fixture, not about the feature.

## 5. ⚠️ The 8 blockers to fix before implementation

1. **The design revives frustum-culled instances, deleting R2d-6's arming.** A single two-ended
   region cannot hold both the HZB rejects and the frustum rejects — there is no third bucket. The
   late candidate set must be `(frustum survivors) \ (early rasterized)`, and the region layout must
   express exactly that.
2. **Unknown bounds break the coverage premise, and the break survives BOTH passes** — a permanent
   false reject for any streaming-in mesh, the reserved slot, or the C0 zero-vertex mesh. The
   sentinel must short-circuit to KEEP **before any projection**, structurally, at the shared entry
   point — the same discipline the shipped frustum arm already uses.
3. **The primary gate cannot be built as specified.** `vb_depth` is `forward.depth[fi]`, created
   `DEPTH_STENCIL_ATTACHMENT | SAMPLED` with no `TRANSFER_SRC`, so `vkCmdCopyImageToBuffer` on it is
   invalid usage. The design lists the depth-readback path as UNVERIFIED while making it
   load-bearing, and assigns it no step.
4. **The late raster's render-pass handling is absent.** The pyramid build is compute, so it must
   sit *between* the two draw loops, splitting one dynamic-rendering scope into two. The existing
   scope opens `LOAD_OP_CLEAR` on both attachments, so a naive second block would discard the early
   pass entirely.
5. **The pyramid seed lies about the layout** — wrong-`oldLayout` UB. Its last access each frame is
   a storage WRITE leaving it in `GENERAL`; seeding the next frame at `SHADER_READ_ONLY_OPTIMAL`
   makes the first derived transition claim an `oldLayout` the image is not in.
6. **The per-object half of the carrier does not exist in any build.** `prev_ring`,
   `gather_prev_ring_into` and `upload_prev_instance_models` are all `#[cfg(feature = "hwrt")]`, and
   no plugin adds the component column. Either drop the per-object prev term (the theorem permits
   it — the early pass is a heuristic) or fund building it, with the cost stated.
7. **The raster split needs its own numbered step**, landed byte-identically with the second scope
   drawing nothing. CLEAR-then-LOAD-then-store must be *shown* equivalent, not assumed.
8. **The false-reject gate is folded into the arming step.** It must be its own step, landed BEFORE
   the arming and proven RED on a deliberate defect while occlusion is still disabled.
