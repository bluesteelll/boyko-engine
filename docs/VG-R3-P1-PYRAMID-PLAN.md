# VG R3 — piece 1 of 4: the depth pyramid, alone

Status: **DESIGNED, one mechanical blocker resolved below, ready to implement.**

Three design rounds. Round 1: 5 blockers, 12 majors. Round 2: 33 prior items closed, 2 blockers
left. Round 3: 31 more closed, **one verdict `APPROVED_WITH_CHANGES`**, one new blocker — a Cargo
dependency cycle, resolved here. **Both reviewers confirmed the scope boundary held in all three
rounds**, which is what the decomposition was for.

## 1. Scope

**In:** the pyramid image, its per-mip views, the build passes, the descriptors the build needs,
its graph declarations, boot / recreate / resize, and the gates proving the built pyramid equals
`boyko_render::hzb`.

**Out, deliberately:** the cull, the occlusion test, the late pass, the raster split, survivor
lists, the `OcclusionCulling` component, `prev_view_proj`, any per-instance verdict, any edit to
`vb_batch_cull.comp.hlsl` or `vb_raster`. **The pyramid is built and read by nothing.** Default
`Off`; `Off` means no image, no views, no pipelines, no sets, no passes, and zero barriers on a
ResId that is still declared.

**No perf claim.** None is measurable here — see [OPEN-QUESTIONS.md](OPEN-QUESTIONS.md).

## 2. The shape

One `R32_SFLOAT` image with a real Vulkan mip chain, one framegraph ResId, `GENERAL` for life,
non-ringed, owned by `GBufferTargets` so it inherits that struct's verified drain-and-recreate.
Level 0 is `prev_pow2` of each source axis — S3's map, mirrored, not re-derived.

**The tile geometry, derived once in ONE index space:** the pass's FIRST OUTPUT LEVEL, never the
source. That is the only space in which the base and reduce variants are the same shape, because
level 0 is *not* a halving of the depth (`P = prev_pow2(S)`, `P ≤ S < 2P`) while every later level
is. Tile = 32×32 texels of level `d`; workgroup 16×16; each thread owns a 2×2 block. Everything
else falls out: `groups[a] = ceil(E_a(d)/32)`, ≤16 taps per thread, 1 KiB of LDS, four barriers,
6 levels per pass, ≤3 passes for `MAX_HZB_LEVELS = 17`.

32 rather than 64 is chosen on a **gate-reachability** ground, not a performance one: at 6
levels/pass the third dispatch is reached at a 4096-wide source, inside Vulkan's guaranteed
`maxImageDimension2D` floor — so the deepest structural case runs on any conformant device.

## 3. The boundary rule — one rule, and there is no clamp

A lane's output texel at level `m` **exists** iff `tx < E_x(m) && ty < E_y(m)`. A lane whose texel
does not exist **issues no tap and stores nothing**, contributing the min identity `+INFINITY`. A
live lane folds exactly those taps of its footprint that lie inside the source extent; an
out-of-extent tap is **not issued**. Equivalence with the oracle is by identity (`min(+∞, x) == x`
exactly), never by idempotence.

Why it is exact rather than approximate, in the two cases the review demanded:

- **Odd extents live only in the SOURCE**, absorbed by the base map's partition. No level extent is
  ever odd except the terminal 1, since `E_a(k)` is `P_a >> k` or the clamped 1. At `S=7, P=4` the
  footprints are `{0,1}{2,3}{4,5}{6}` — oddness handled by footprint LENGTH, not by a clamp.
- **A clamped-to-1 axis** bottoms out: at `S=3, P=2`, `E_y(1) = 1`, tap 1 is dropped — exactly the
  oracle's `if sy >= fine_h { continue; }`.

Round 2's defect (`first(5) = 9` against a 7-pixel source — an out-of-bounds Load with
`robustBufferAccess` OFF) is now **unspellable**, because the extent test precedes the address
computation.

## 4. ⚠️ The one blocker from round 3, and its resolution

**`boyko_render` depends on `boyko_rhi_vulkan` as a NORMAL dependency; the reverse edge exists only
under `[dev-dependencies]`.** So an owner struct in `crates/boyko_rhi_vulkan/src/hzb.rs` cannot name
`HzbLayout`, which lives in `boyko_render::hzb`. That is a Cargo cycle: the step would not compile
at all, let alone land green alone.

**Resolution (mine, an architecture call):** the RHI-side owner stores the **derived scalars** —
level count, per-level extents, per-level offsets — not the `HzbLayout` type. `boyko_render::hzb`
remains the sole oracle. The bit-exact comparison is unaffected because it runs in `boyko_app`'s
tests, which depend on both crates.

This also keeps the dependency direction honest rather than working around it: the RHI has no
business naming a render-layer type, and the scalars are what it actually needs.

## 5. The two gates, and why both are required

- **G3 — the shader equals the oracle, with no engine involved.** It creates its own depth image,
  pattern, views, pipelines and readback, and compares `to_bits()` at every texel of every level at
  **7×3, 8×16, 1×1, 511×1023 and 1920×1080**. Bit-exact is legitimate here because the only float
  operation in the whole build is `min` — exact selection, no rounding, no ULP question. Every
  golden pin in the tree is 512×512, a power of two, where a clamp or base-map bug cannot fire;
  four of these five extents fire it.
- **G8 — the pyramid the ENGINE built.** G3 cannot see a wrong source, a wrong extent, a stale
  descriptor, a missing barrier or a pass that never ran, *because* it builds its own everything.
  So `BOYKO_HZB_DUMP` copies the engine's own depth and every mip of the engine's own pyramid, and
  a host test rebuilds from the dumped depth and compares. Non-vacuity is asserted, not hoped: the
  scene fully covers the framebuffer and every dumped depth texel must be `> 0.0`, which restores
  the poison property (the boot clear is `0.0`, so any level the build failed to write mismatches
  everywhere), plus ≥2 distinct depths, plus the arm word, plus no texel is `+INFINITY`.

The recording seam for G8 was the second round-3 blocker and it is **found rather than invented**:
the census's recording half lives in `present/passes/vb.rs`'s `vb_id_readback` copy block, not in
`vg_census_dump` (which is only the host half). The pyramid dump takes that seam and improves on it
— the copy is a **declared framegraph pass** rather than a hand-written barrier, which is expressible
because `vkCmdCopyImageToBuffer` accepts `GENERAL` and the pyramid is `GENERAL` for life, so the
derived edge changes no layout and the seed is not falsified on a dump frame.

## 6. The one permanent edit outside the pyramid

`TRANSFER_SRC` on `ForwardTargets::depth`, in the shape `targets.rs` already argues for `vb_id`,
gated by the full golden set in its own step. It is what makes G8 possible at all.
