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

Round 2's defect (`first(5) = 9` against a 7-pixel source) is now **unspellable**, because the
extent test precedes the address computation.

⚠️ **That defect was an ORACLE-AGREEMENT bug, not a memory-safety one, and this section used to say
otherwise.** It cited `robustBufferAccess` being off. That feature governs BUFFERS; Vulkan bounds
IMAGE accesses unconditionally, with no feature required — an out-of-range fetch returns undefined
values and an out-of-range write is discarded, and neither can fault. So the rule is not a guard
against a robustness bit somebody might later flip on: without it the fold takes UNDEFINED DATA
where the oracle's `continue` contributes `+INFINITY`, and under reverse-Z an undefined zero is the
far plane, so the pyramid comes out conservatively too small — invisible in every golden. The old
wording invited exactly the inference that deletes the property.

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

**Who DERIVES them, made explicit (the review's one real improvement to this section).** The
resolution above was silent on it, and silence invites a second implementation of `prev_pow2` /
`msb` / `level_extent` inside `boyko_rhi_vulkan`. It must not exist. `boyko_app` links both
crates and already authors the extent `GBufferTargets::create` sizes to, so **the host calls
`boyko_render::hzb::HzbLayout` ONCE and threads the resulting scalars onto `GBufferScene`
beside the arm bit** — the same seam `shadow_denoise_enabled` uses. The RHI stores what it is
handed and derives nothing. One implementation in the tree, no cross-crate formula to drift.

The `MAX_HZB_LEVELS` array-sizing constant is the one thing the RHI must still spell for itself
(a `[VulkanTextureView; N]` needs a `const`). That tie does **not** need a runtime test in
`boyko_app`: `boyko_render/src/gbuffer_depth.rs:44,82` already `use`s a `boyko_rhi_vulkan`
constant and pins it with `const _: () = assert!(...)`, so the equality is a **compile error**
on drift rather than a test that has to be run.

## 5. The two gates, and why both are required

- **G3 — the shader equals the oracle, with no engine involved.** It creates its own depth image,
  pattern, views, pipelines and readback, and compares `to_bits()` at every texel of every level at
  **7×3, 8×16, 1×1, 511×1023, 1920×1080, 1024×64 and 4096×4096**. Bit-exact is legitimate here
  because the only float operation in the whole build is `min` — exact selection, no rounding, no
  ULP question. Every golden pin in the tree is 512×512, a power of two, where a clamp or base-map
  bug cannot fire; four of these extents fire it.

  ⚠️ **The last two were added by the P1-3 review, and they close a gap that made §2's own
  justification untested.** The first five all produce at most TWO passes — three passes need
  `levels ≥ 13`, i.e. `max(W, H) ≥ 4096`. So the reason §2 chose a 32-texel tile over a 64-texel one
  ("the deepest structural case runs on any conformant device, rather than shipping untested") was
  not exercised by the gate it was made for. `4096×4096` reaches the third dispatch AND gives it a
  `level_count == 1` final pass, in which `gDst1..gDst5` are bound to views the pass must not write.
  `1024×64` is the other absent shape: an axis bottoming out EXACTLY at a pass boundary
  (`E(6) = [16, 1]` reading `fine = [32, 2]`). The reviewer could not break the shader on either by
  hand — but "checked by hand" is not this campaign's standard, which is the whole point.
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

## 7. Mechanical facts established by the P1-1 review (27 raised, 25 refuted)

The review of step P1-1 raised 27 findings and **25 were refuted** — but the refutations had to
read the code to kill the claims, and what they established is worth more than the claims were.
Each item below is anchored, and each removes a question the implementation would otherwise
have had to answer at its own cost.

- **Per-mip barrier spans are spellable today.** `SubRange`'s five fields are all `pub`
  (`framegraph/sync.rs:37-44`, re-exported at `framegraph/mod.rs:49`), so
  `SubRange { base_mip: k, mip_count: 1, .. }` is writable at any pass site. No new constructor.
- **⚠️ The barrier path bounds NOTHING.** `graph_bridge.rs:392-398` and `:576-582` copy
  `base_mip`/`mip_count` verbatim into `VkImageSubresourceRange`, and the sink holds bare
  `VkImage` handles with no mip count to check against (`:317`) — unlike the VIEW path, which
  does assert it (`texture.rs:648-651`). **Consequence: every `SubRange` the pyramid declares
  must carry the DERIVED level count. `color_mips(MAX_HZB_LEVELS)` — the natural constant
  spelling — is out of range at every real resolution**, and nothing would catch it.
- **A mipped `UNDEFINED` transition already has a precedent**, contrary to the "both precedents
  hardcode `level_count: 1`" reading: `boyko_render/src/texture.rs:246-262` emits
  `old_layout: Undefined` with `level_count: mip_levels` over "every mip". The boot transition
  is not novel work.
- **The pyramid does not force a global ResId bump.** The VB path has a PRIVATE ResId space
  (`graph_bridge.rs:2874-2886`, `VB_IMAGE_COUNT = 14`/`20`), documented as never related to
  `FRAMEGRAPH_IMAGE_COUNT`.
- **`VulkanTexture::create` holds for `mip_levels = N`, single layer**: exactly one view is
  built, `is_array` is false, nothing trips. The usage set is `STORAGE | SAMPLED | TRANSFER_SRC`
  — `SAMPLED` so the texture-owned `[0, N)` view is unambiguously legal (a multi-mip view is not
  bindable as a storage descriptor), `STORAGE` for the per-level views the build binds,
  `TRANSFER_SRC` for G8's dump copy. This is the engine's FIRST storage image with a mip chain;
  every other call site in the tree passes `mip_levels: 1`.
- **⚠️ Carried forward to piece 3:** `SAMPLED_IMAGE_FILTER_LINEAR` is **not** mandatory for
  `R32_SFLOAT` (only `SAMPLED_IMAGE` and `STORAGE_IMAGE` are). Whatever reads the pyramid must
  do so point-sampled — `.Load`, never a linear-filtered sample — or probe first.
- **⚠️ The pyramid is NON-RINGED while the depth it reduces IS ringed** (`targets.rs:769-771` —
  one `D32_SFLOAT` image per `FRAMES_IN_FLIGHT`). Two consequences, both carried forward. The
  BASE pass's descriptor set must be a `[VulkanBindGroup; FRAMES_IN_FLIGHT]` binding
  `core.depth[slot]`, exactly `viewt_from_depth_set`'s shape (`targets.rs:2958-2984`); the reduce
  passes' sets touch only pyramid mips and need no ring. And **piece 3 inherits a cross-frame WAR
  question that piece 1 cannot answer**: a single-buffered image written every frame is safe only
  while nothing reads it, which is exactly piece 1's situation and exactly not piece 3's. The
  engine has recorded this failure shape before ("wrong only in motion, stable when stopped").
- **⚠️ `sync_gbuffer` short-circuits on `(extent, aa_arm)` alone** (`targets.rs:7393-7399`), and
  `TargetsProfile` is a parameter, never a stored field (`:7381-7384`). So an arm bit that only
  rides on `GBufferScene` cannot survive a runtime flip at fixed extent. Either the HZB arm
  becomes a STORED field joining that predicate — the shape `aa_arm` already argues for at
  `:473-476` — or arming is boot-only *by construction* and says so. Piece 1 defaults `Off` and
  is read by nothing, so this is a design choice to make deliberately, not a latent bug.

## 8. Step P1-3 as built — the shader, and what its artifact gate can actually see

**One shader, one entry point, no `-D` variant** (`shaders/hzb_build.comp.hlsl`, `LocalSize 16 16 1`).
The base/reduce fork is a uniform branch on `pc.base_level == 0` rather than two artifacts: §2's
argument that the two variants have the SAME SHAPE in the first-output-level index space is
precisely the argument for sharing the body, and the tile/LDS/barrier code — the part that is hard —
then exists exactly once. No `docs/SHADER-VARIANT-MANIFEST.md` row, since that manifest registers
`-D` variants only.

**Hand-authored, not eDSL, and that is the rule rather than an exception to it.** The eDSL owns
numeric LEAVES — one generic Rust body instantiated over `f32` (the host oracle) and `Emit` (the
HLSL printer), so that nontrivial float math is bit-exact across the boundary. This shader's entire
float content is `min`. There is no leaf. What the shader *is* — the tiling, the LDS regions, the
barriers, the per-mip UAV addressing — is the category every generated shader in the tree
hand-authors AROUND its sentinels, and `bin/emit_probe_gi.rs:16-19` names it as such.

**The LDS layout is four DISJOINT regions, not one reused array**: 256 + 64 + 16 + 4 = 340 floats
(1360 B) for levels `d+1 .. d+4`; level `d+5` is one texel and needs none. §2's "1 KiB" counted only
the first region. Disjointness is what buys ONE barrier per step and hence §2's four: a single
reused 16×16 array races (thread (0,0) reads index 1 while thread (1,0) writes it), and the
read-barrier-write repair would double the count to eight.

**Every per-level extent is PUSHED — the shader derives none.** §4 put `prev_pow2`/`msb`/
`max(1, base >> k)` in `boyko_render::hzb` and only there; `HzbPlan::level_extent` already holds
every value, so the 72-byte push carries them and the shader re-derives nothing. This also means a
base-map disagreement can only ever be a SHADER bug, never a math one.

**One correctness detail §2 did not reach.** `first(t) = (t*S + P - 1)/P` is computed in `uint`, and
at `t == P` with `P == S == MAX_HZB_EXTENT == 65536` the product `P*S` is exactly `2^32` and WRAPS,
returning `0` where the answer is `S`. For `t <= P-1` there is no overflow (`t*S + P - 1 <= P*S - 1`,
using `P <= S`), so the `t == P` case is both necessary and sufficient, and it is reached on every
base pass — it is the exclusive END of the last live texel's preimage.

### What the gate proves, and the two corruptions that proved it proves anything

`tests/hzb_build_spv_sync.rs` — byte identity plus a module census. At this step **no image in the
repository can move if the shader is wrong**, because nothing builds the pyramid yet; that is why
the gate is structural rather than deferred to P1-7.

The strongest pin is `OpExtInst == 0` **and** `OpExtInstImport == 0`: HLSL's `min()` lowers to
`GLSL.std.450 NMin`, under which **a NaN operand is silently discarded rather than propagated**
(this engine has a recorded incident on exactly that). `hzb_build.comp.hlsl` therefore calls no
intrinsic at all — its reduce is `isnan` plus a compare-and-select — so a module that reaches no
extended instruction set is the artifact-level proof that no `NMin` is hiding in it.

Both directions were EXECUTED, not asserted in prose:

| corruption | observed |
|---|---|
| one `hzb_min` → `min()` | 1 import + **4** `OpExtInst` (the helper inlines at four sites); the NaN pin red |
| barrier 3 of 4 deleted | `op_control_barrier` 4 → 3; the barrier pin red |
| both restored | committed `.spv` byte-equals the re-DXC |

⚠️ **The byte gate stayed GREEN under both corruptions** — correctly, since the `.spv` was recompiled
and honestly matches the corrupted `.hlsl`. That is the whole reason the census is a separate test:
byte identity certifies provenance and says nothing whatever about content.

Ordering inside the census test is deliberate and was itself measured: with the whole-struct compare
first, the dropped barrier reported as a sixteen-field `SpvCensus` diff in which the one changed
field sat among fifteen identical ones, and the named message explaining what a missing barrier DOES
never ran. Every named property therefore fires ahead of the struct compare, which stays behind them
as the catch-all.

`op_ford_less_than == 17` is measured **and derived**, so it can be reasoned about when it moves:
1 (`hzb_base_texel`, dynamic bounds → rolled) + 1 (`hzb_fine_texel`, kept rolled) + 3 (the per-thread
`q[0..4]` fold) + 12 (`hzb_fold_lds` inlined at four sites, 3 apiece). The census corroborates the
rolled `main` loop independently: `op_image_fetch == 1` and `op_image_read == 1` rather than 4 each.

`OpCapability` is pinned to exactly `["Shader"]`, and what that proves is what is ABSENT: no
`StorageImageWriteWithoutFormat` (every RW view carries `[[vk::image_format("r32f")]]`), no
`StorageImageArrayDynamicIndexing` (six separate bindings rather than an array DXC might decline to
unroll), no `Int64` (the overflow argument is carried in 32-bit arithmetic by the `t == P` case).
Each absence is a device feature this pass does not require and would otherwise have discovered on
someone else's hardware.

### What the P1-3 review changed, and the one claim it got wrong

The review found **no oracle disagreement** — it walked the shader against `build_pyramid` on eight
extents, three of which the gate list did not contain (`1024×64`, `8192×8`, `4096×4096`), and
verified the LDS bounds at all four fold sites, the barrier uniformity, and the `+INFINITY`
propagation lemma in both directions. What it found instead was worth more than a math bug:

- **⚠️ A VACUOUS ASSERTION, in the file whose subject is vacuous gates.**
  `assert_eq!(local_size[0] * 2, HZB_BUILD_TILE)` could not fail: the assertion above it already
  established `local_size[0] == HZB_BUILD_LOCAL_SIZE`, and `const _: () = assert!(HZB_BUILD_TILE ==
  HZB_BUILD_LOCAL_SIZE * 2)` makes both sides the same expression at compile time. It read nothing
  from the module. **`TILE = 64u` against an unchanged `[numthreads(16,16,1)]` passed BOTH gates**
  — every opcode count, the LocalSize, the bindings, the push offsets, the capabilities and the LDS
  length are untouched by it — while half of every level goes unwritten and keeps the boot clear.
  The tile is now tied through the constants its `TILE >> k` multipliers declare, in both
  directions (`%uint_TILE` present, `%uint_(2*TILE)` absent), and that corruption was EXECUTED and
  turns the gate red.
- **⚠️ `robustBufferAccess` was the wrong threat model** for the boundary rule, here and in §3 above.
  It governs BUFFERS; Vulkan bounds image accesses unconditionally, with no feature required. The
  guard is mandatory for **ORACLE AGREEMENT**, not memory safety: without it the fold takes
  undefined data where the oracle contributes `+INFINITY`, and under reverse-Z an undefined zero is
  the far plane. The inference the old wording invited — "robustness handles it, the guard is
  belt-and-braces" — deletes the property the step rests on.
- **⚠️ The module requests no `SignedZeroInfNanPreserve`.** The capability pin reads every absence
  as a benefit; this one is not. `HZB_IDENTITY` is `+INFINITY` on every partial tile, `isnan` IS the
  NaN policy (and may fold to `false` where no-NaN is assumed — the `OpExtInst == 0` outcome reached
  from the other side), and "no ULP question" is true of the selection, not of the comparison under
  denorm-flush. Risk is low and the failure direction conservative, but the step's thesis is that
  bit-exactness is DECIDABLE, so the assumption is stated rather than discovered on other hardware.
- Three fixture and citation defects: `lds_words` read the LAST Workgroup variable rather than the
  sum (a second `groupshared` array — the natural way to extend the chain — would have left `340`
  green while shared memory grew); `op_image_fetch`/`op_image_read` were cited as INDEPENDENT
  corroboration of the rolled loop when they share its full-inlining premise (`op_image_write` is
  the count that does not); and binding 0's `.Load` justification cited `R32_SFLOAT`'s filter
  feature, which is about the PYRAMID, not the `D32_SFLOAT` depth this binding names.
- `op_image_write` and the binding-set length are now DERIVED from `HZB_LEVELS_PER_PASS` rather than
  spelled `6` and `[0..=7]` — two literals beside a host constant agree today; they are not a tie.

**The one finding that was wrong, and it is instructive.** The review read `[unroll]` on the
level-`d` loop as decoration, reasoning that `op_image_write == 6` rather than 9 proves DXC did not
unroll. That inference is sound and the conclusion is not. Removing the attribute was MEASURED to
change the module by exactly one token — `OpLoopMerge %90 %88 Unroll` becomes `... None`, nothing
else in 15856 bytes moves. The attribute does not unroll the loop in DXC; it RECORDS the request in
the SPIR-V for the DRIVER's backend, which is where the unroll happens and where `q[4]`'s dynamic
indexing gets promoted out of function-scope memory — precisely the concern the same finding raised
two sentences earlier. The hint's survival is now its own census field. Deleting a working
optimisation directive on a plausible inference is the exact failure this campaign records as
"verification is an ACTION": the claim was checkable in one recompile, and one recompile refuted it.

## 9. Step P1-4 as built — the pipeline and its sets, dispatched by nothing

One `VkDescriptorSetLayout` (8 bindings: `SampledImage` @0, `StorageImage` @1..@7) and one
`VkPipeline`, both minted UNCONDITIONALLY in `GpuSceneBundles::boot` beside the cull's. The arm
lives on the TARGETS — `HzbTargets` is `None` when `HzbConfig` is `Off`, so the SETS are absent
exactly when the pyramid is, and there is no second predicate that could disagree with them. (Rung
R2d-2 hit the mirror-image defect on `vb_cull_set`: five unconditional `Some` literals against an
armed resource.)

`HzbTargets::sets` is `[[Option<VulkanBindGroup>; MAX_HZB_PASSES]; FRAMES_IN_FLIGHT]`, `Some` iff
`p < levels.div_ceil(6)`. Per pass `p`: `d = 6p`, `n = min(6, levels - d)`; `@0` binds
`depth[slot]`, `@1` binds `level_views[d-1]` (or `level_views[0]` on the base pass — never read,
the `pc.base_level == 0` branch guarantees it), and `@2+k` binds `level_views[d+k]` for `k < n`,
PADDING with `level_views[d]` beyond that — a real view of a real mip that no store reaches, because
every store is guarded by `k < pc.level_count`.

**The sets are per-FIF because the DEPTH is ringed and the pyramid is not.** Only `@0` differs
between slots. Ringing all passes rather than only the base one costs `FRAMES_IN_FLIGHT × pass_count`
sets — four at every real extent — and removes a special case from the recorder P1-5 will write.

### Gates, and the control that makes them mean something

All **25 golden pins byte-identical**. The validation leg shows **19 messages armed and 19 unarmed,
identical after handle normalisation** — so the pyramid, its four descriptor sets and its pipeline
contribute ZERO. That leg is the load-bearing one here exactly as it was at P1-2: this is the
engine's first storage image with a mip chain and now also its first per-mip storage-image
descriptor, and an illegal view that no pass binds changes no pixel.

Byte-identity still does not prove the armed path RAN, so the two-sided control was executed again.
A panic in the set-build loop reported **`sets built = 4, pass_count = 2, levels = 10`** on the armed
pin — the exact predicted arithmetic for 512×512 (`msb(512) + 1 = 10` levels, `⌈10/6⌉ = 2` passes,
× 2 frames in flight) — while the unarmed pin stayed green with the same panic compiled in.

⚠️ **A harness fact worth recording: `golden.ps1` reports a red pin WITHOUT the panic text.** It does
not pass `--nocapture`, so the first run of this control showed only "RED" and no message, which is
"red without proof" — the mirror of the byte-identical-without-execution trap. The message had to be
recovered by invoking the test binary directly (with `--ignored`, since the GPU dumps are
`#[ignore]`d). A gate one cannot read the failure of is a gate one has to re-run differently to
believe.

### ⚠️ The wrong depth ring — raised by the implementer, and WORSE than the raising said

The implementer flagged that `@0` bound `GBufferTargets::depth`, the CORE ring, while
`TargetsProfile::ForwardMesh` rasterises into its own `ForwardTargets::depth` — and closed the
report with "correct under VB, where `VbTargets` carries no depth of its own". That last clause is
true and it is not the question. **`VbMesh` builds a `ForwardTargets` bundle too, precisely to REUSE
its depth ring**, and the VB raster binds it directly:
`present/passes/vb.rs` — `image_view: forward.depth[fi].view`, named `vb_depth` throughout.

So the binding was wrong under **VB as well** — the one profile this entire feature exists for — and
it would have handed the pyramid the DEFERRED depth, an image the VB frame never writes. Inert while
nothing dispatches; a silently empty pyramid the moment P1-5 does.

The resolution needs no profile `match`. `forward.is_some()` is exactly "this profile rasterises
somewhere other than the core ring", so the call site picks `forward.depth` when the bundle exists
and `targets.depth` otherwise — correct for all five profiles, one rule, stated as what it means:
**the pyramid reduces the depth THIS FRAME'S RASTER WROTE.**

Worth recording HOW it was caught, because the report contained the correct facts and the wrong
conclusion. The profile enum's own doc says `VbMesh` builds `ForwardTargets` "REUSED for the depth
ring", but it says it in prose about a BUNDLE, and the implementer read it as a bundle-level remark.
What settled it was reading the attachment the VB raster actually binds — a grep, not an inference.

## 10. Step P1-7 as built — gate G3, and the one place the GPU does not agree

`crates/boyko_app/tests/hzb_build_oracle_gate.rs`. It lives in `boyko_app` because that is one of
only two crates naming both `boyko_render` (the oracle) and `boyko_rhi_vulkan` (the shader) — §4's
Cargo resolution, arriving where it was always going to.

It builds its own everything: a `D32_SFLOAT` source uploaded with the DEPTH aspect, an
`R32_SFLOAT` pyramid with a real mip chain, one single-mip view per level, its own 8-binding
layout, its own pipeline from `hzb_build_spirv()`, one descriptor set per pass with the same
padding rule `HzbTargets` uses — and **hand-written barriers**, which is what let it land before the
framegraph work (§8's blocker) does.

### The result

**Seven extents, BIT-EXACT.** `7×3`, `8×16`, `1×1`, `511×1023`, `1920×1080`, `1024×64` and
`4096×4096` — 22 369 621 texels on the last, thirteen levels, **three dispatches**, every texel's
`to_bits()` equal to `boyko_render::hzb::build_pyramid`. The two extents the P1-3 review added are
the ones that exercise §2's own justification for a 32-texel tile; both pass.

Non-vacuity is asserted, not hoped: the pyramid is poisoned to `-1.0` through a buffer copy before
every dispatch and no texel may retain it; three depth probes confirm the upload; and `levels`,
`pass_count` and the per-pass `[d, n, groups_x, groups_y]` are checked against a hand-computed
table before a byte is allocated. The gate was then shown RED: replacing the base map's `⌈t·S/P⌉`
with a floor broke `7×3` at 5 of 8 level-0 texels.

### ⚠️ The signed-zero divergence, and why it is not a shader defect

The P1-3 review called the tree-vs-left-fold association "the single most fragile equivalence in
the step" and reasoned that both compute *the earliest minimal element in program order*. **That
reasoning is correct about the SOURCE and wrong about what runs.**

Measured, deterministic over three runs on an RTX 3060 Laptop: of two 2×2 footprints planted with
`+0.0` and `-0.0` in OPPOSITE operand orders, the one whose source semantics say `+0.0` comes back
`-0.0`; the one that should be `-0.0` agrees. That asymmetry is the whole diagnosis — **the driver
recognised `b < a ? b : a` and fused it into a hardware min**, whose `±0` tie-break returns the
negative zero regardless of operand order. It is allowed to: the two values compare EQUAL, so no
`<` in the program can distinguish the semantics, and the module requests no
`SignedZeroInfNanPreserve`. This is the first hard evidence for the P1-3 review's W3, which until
now was a stated assumption.

**Consequence for what a `.spv` census can claim.** `OpExtInst == 0` proves DXC emitted no `NMin`.
It cannot prove the DRIVER did not build one afterwards. That is a permanent limit on
artifact-level pins and it is now recorded in the shader header beside the pin it qualifies.

**The half with teeth was measured separately and SURVIVED.** `hzb_build_nan_collapses_to_negative_infinity`
plants a quiet NaN and requires `-INFINITY` at every level: bit-exact. The explicit `isnan` branch
was not fused away — had it been, the fold would have returned the OTHER operand, which is exactly
the `NMin` behaviour the shader is written to avoid, reached from outside the compiler it can pin.

**Accepted, not fixed.** `+0.0` and `-0.0` are numerically equal; the pyramid is a conservative
lower bound whose only consumer is `depth_near < occ`; a real reverse-Z rasteriser never produces
zero depth. Requesting the execution mode would cost a device feature, a new capability in the
census and a hardware constraint, to fix a difference that cannot reach a pixel. The gate therefore
asserts the NARROWED claim: **every** divergence in the chain is a ±0 tie, and there are EXACTLY
three — a count that is measured AND derived (level 1 `(1,1)`, then levels 2 and 3 inherit it),
so a change in it can be reasoned about rather than merely re-pinned.

## 11. Step P1-5a — per-mip framegraph sync state, as CORRECTED by its critique

Verdict `APPROVED_WITH_CHANGES`: the machine is sound and the byte-identity fold survives every
attack on the production corpus. What was defective was everything *around* the machine — one API
signature wrong on arrival, a migration set of four tests where the plan named one, a baseline that
could not be certified, and four gates that could not fail.

### The design, corrected

**State is keyed `(ResId, mip)`.** Layers stay uniform-by-requirement; the invariant survives,
narrowed to the layer axis. The asymmetry has a stronger justification than the plan offered:
`texture.rs:227-231` `debug_assert!(!(is_array && mip_levels > 1))` makes mipped and layered
**disjoint at creation**, so a flat `state_base + m` cannot collide with a layered resource. Every
layer span in the tree is a compile-time constant and every layered resource is written and read
whole-array. *(The plan's entry arithmetic was wrong: per-layer tracking would give **65**, not 69 —
a layered resource is REPLACED in the flat sum, not supplemented. `sum of mip_count x layer_count`.)*

**The seed is a REQUIRED argument: `add_image_mipped(name, mips, seed: ResSync)`.** The plan
deliberately omitted a seeded-and-mipped constructor, calling its absence "a compile error the day
it is needed". It is not; it is a **one-way door**. `add_image_mipped` would be the only declarable
route for a mipped resource, its seed would be `undefined()`, and the pyramid is NON-RINGED — so
frame N+1's first write to mip `d` would derive `TOP_OF_PIPE` with no dependency on frame N's
still-pipelined reads, and no gate inspects `res_seed`. `add_image_seeded` cannot substitute: it
yields `mip_count == 1` and the range check then rejects `base_mip > 0`. The declarator's convention
admits no counterexample — plain `add_image` for every ringed image, `add_image_seeded` for **every**
non-ringed one. Making the seed required turns piece 1's unanswered cross-frame WAR question into a
compile error, which is what the omission was supposed to achieve and did not.

**`resolved_layout` must be REBASED, not merely asserted.** `graph.rs:528` is
`state[res.index()]`; once `state` is mip-weighted the correct entry is
`state[res_shape[ri].state_base]`. The plan's proposed "assert the resource is single-mip" is
ORTHOGONAL to the precondition the index needs — `state_base(i) == i` iff every EARLIER resource is
single-mip. Declare `depth`(1), `pyramid`(10), `lit`(1) and `resolved_layout(lit)` reads the
pyramid's mip 2: the assert passes, the bounds check passes (and gets WEAKER, since `state.len()`
grew), and release returns a neighbour's layout. Five of the ten expectations at
`tests/framegraph_gbuffer_equiv.rs:233-242` are `GENERAL`, so the wrong read agrees by coincidence.

**`res_written` must be mip-weighted too.** It is `#[cfg(debug_assertions)]`, so this costs nothing
in release. Left per-ResId while `state` goes per-mip, a pure-read consumer of a mip its writer
never wrote is silent — the guard sees the ResId as written — and `transition` then takes the
first-touch arm and emits `UNDEFINED -> GENERAL`, which is verbatim the failure `graph.rs:350-352`
says the guard exists to prevent. `res_sub_witness` stays per-ResId (the invariant is layer-only).

### The gates, with the four that could not fail replaced

- **G-A1** the ordered barrier-stream pin, authored on the unmodified tree FIRST. Scope: the
  deferred replica only; VB and Forward have no barrier-level test, and the bridge is the
  release-live range check plus the fold.
- **G-A2** the differential against a frozen `compile_per_resource_reference`, plus proptest over
  single-mip graphs. It is **structurally incapable** of testing `state_base`: over an all-1s corpus
  `total += mip_count` and `total += 1` are the same function.
- **NEW, closing that hole:** a fixture declaring `[single, mipped(M>=3), single, mipped(M>=2)]` and
  asserting the derived barriers on the **LAST** resource. Every proposed G-B fixture used one
  mipped resource declared last, where the buggy and correct prefix sums agree. The escape is
  silent, not loud — the aliasing index stays in bounds — and it would detonate at P1-6, where the
  declarator puts ~20 resources after the image block.
- **G-B3 must be authored in the LAYOUT-CHANGING shape.** `transition` returns `None` only on a
  READ, so a write-authored "a free mip breaks the run" fixture tests nothing. Correct: write [0,3)
  GENERAL, read mip 1 alone at SRO, read [0,3) at SRO — asserting both barriers field-by-field.
- **G-E replaced.** `state.capacity()` across reset+recompile measures a property of `Vec`, not of
  the plan: `clear` retains capacity, and a fresh same-size allocation every frame would pass.
  Worse, `reset()` itself clears `state`, so a dropped `clear()` in the new seed loop is MASKED by a
  reset+recompile gate. Assert `res_state_total <= N` directly, and compile TWICE WITHOUT RESET.
  *(Correction the plan must absorb: `frame_driver.rs:188` is `with_capacity(16, 16, 64)` under a
  comment claiming zero-alloc, while the deferred declarator mints 33 resources — `state` already
  grows on frame 1 today, so "assert capacity unchanged" fails on the production shape.)*
- **Nothing reads RECORDED order.** Every gate reads `img_barriers()`/`pass_barriers()`, but
  `record.rs:92-99` groups by stage pair alone and never inspects `subresource`, and one group
  lowers to one `vkCmdPipelineBarrier`. Add a `BarrierSink` asserting, per pass, that no two
  barriers in one emitted group intersect in `(res, mip span, layer span)` and that per `(res, mip)`
  the recorded order equals the derived order.

### Commit order, which the findings make mandatory

- **C0 — DONE.** `#[cfg(debug_assertions)]` on the two `#[should_panic]` tests over `debug_assert!`
  guards. **The release leg was RED before this**: `12 passed; 2 failed`, measured, and CI runs
  `cargo test --workspace --all-targets --release`. A baseline cannot be authored on a leg that does
  not pass.
- **C1** — the G-A1 stream pin and the frozen reference, on the unmodified tree. Authoring them
  after the change would certify the new behaviour.
- **C2** — the machine, all four test migrations, and every gate, ATOMICALLY: the invariant retarget
  and the migration cannot be split without a red intermediate.

### The four test sites that migrate, which the plan named as one

`tests/framegraph_gbuffer_equiv.rs` pairs `add_image("pyramid")` with `SubRange::color_mips(4)` in
FOUR graphs: `:989`, `:1024`, `:1080`, `:1162`. Under the corrected design `add_image` means
`mips = 1` and the range check is release-live in `image_access` — a DECLARE-time function — so the
first three would panic with the RANGE-CHECK message and fail their `should_panic(expected = ...)`
substring, and the fourth (`subresource_guard_does_not_leak_across_reset_and_recompile`, green and
ungated in both legs today, asserting `img_barriers().len() == 2`) would go from green to a hard
failure. **The plan named none of them.**

Two further consequences: the range-check panic message must be FORBIDDEN from containing the
substring `HZB-SUBRESOURCE-UNIFORM`, or the cheapest repair fuses two distinct failure modes into
one — a third vacuous gate. And the layer-axis RED fixture must be authored NEW: no test in the
corpus varies `layer_count` on one resource, and `color_mips` hard-pins `base_layer: 0,
layer_count: 1`, so the layer-narrowed guard cannot be reached by flipping anything that exists.

### Mechanical facts the refutations established — do not re-derive

- `push_res` (`graph.rs:219-230`) is the SOLE writer of all four per-resource arenas; all four
  public constructors funnel through it. `ResId(` is constructed in exactly one place workspace-wide.
- `push_access` (`graph.rs:275`) is the SOLE writer of `acc_sub`; its only callers are
  `image_access` and `buffer_access` (which hardcodes `SubRange::COLOR`). **A check in
  `image_access` therefore DOMINATES every mip-weighted index** — provable, not assumed.
- `self.state` has exactly THREE index sites: `graph.rs:451`, `:525`, `:528`.
- `same_span` has exactly two sites; the rename is a 2-site change.
- **`SubRange::color_mips` has ZERO production call sites** workspace-wide.
- **`resolved_layout` has ZERO production callers** — its only ten sites are in the equiv test.
- `VK_REMAINING_MIP_LEVELS` is **not defined anywhere** in the engine; the range-check test must
  spell `u32::MAX`.
- There are **FOUR** production `compile()` sites, not three: the three declarators plus
  `frame_driver.rs:194-222`, a 4-image boot graph declared with no preceding `reset()`.
- `transition` returns `None` **only on a read**, for images.
- `MAX_PASS_BARRIERS = 16` is a stack CHUNK size, explicitly "NOT a hard cap"; oversized passes
  chunk soundly, and the bound is PER PASS.
- Release runs with `debug-assertions` and `overflow-checks` OFF (only `[profile.bench]` is declared).
- `Trans` derives `PartialEq`/`Eq`, so the merge's `t == rt` compiles; `SubRange` derives
  `PartialEq` over all five fields, and the equiv test's `has_img` compares `subresource` in FULL —
  so a `layer_count` transcription slip is caught by already-committed assertions.
- `texture.rs:227-231` makes mipped and layered DISJOINT at image creation.

### C2 as built, and what §11 got wrong

The machine landed as §11 specifies. `maximal_frame_barrier_stream_is_pinned` — captured in C1 on
the unmodified tree — is **green**: 23 image barriers, 10 buffer barriers, 11 per-pass ranges, every
field identical after re-keying the state machine. All 25 golden pins byte-identical. The framegraph
suite passes on BOTH legs. Zero production call sites changed.

`compile_derives_the_hzb_build_chain_at_a_real_extent` is the new load-bearing test: the real HZB
shape at a 512×512 extent (`levels = 10`, two passes), asserting all three derived barriers, of
which the third — `UNDEFINED → GENERAL` over mips `[6, 10)` — **is the measured bug stated as a
requirement**. The old machine derived `old_layout == new_layout == GENERAL` there and left those
mips untransitioned. Nothing else in the file catches it.

**Five things §11 asserts that turned out to be wrong or incomplete**, each found by the
implementer rather than by the plan:

1. **⚠️ §11's "mechanical facts" are all PRE-C1.** `graph.rs:451/:525/:528` and "`same_span` has
   exactly two sites" describe the tree *before* C1 added the frozen `compile_per_resource_reference`.
   Today `same_span` has three occurrences and `self.state` has four index sites. That produced a
   direct conflict between two of §11's own instructions — "rename `same_span` (2 sites)" and "do not
   modify the frozen reference". **A fact list captured at one commit and applied at a later one is a
   trap, and this one was laid by the same document that warns against re-deriving.**
2. **The `SubWitness` field rename cannot avoid the frozen body**, because `res_sub_witness` is ONE
   arena of ONE type shared by both `compile()` and the frozen copy. Resolution taken: the field is
   renamed (three identifier tokens inside the frozen body) while `same_span` is kept alive in
   `sync.rs` under its one caller's `cfg`, so the frozen body's PREDICATE is unchanged and it still
   answers as the C1 machine did on every input. Recorded in situ.
3. **§11 has no "algorithm block"** — the implementation brief cited one that does not exist in the
   document. Implemented from the prose plus G-B3's fixture shape instead.
4. **`add_image_mipped` forcing a seed makes `res_written`'s mip-weighting inert TODAY.** §11
   justifies it with "a pure-read consumer of a mip its writer never wrote goes silent" — but the
   seed is required, and `res_written` is initialised from `res_seeded`, so every mip of every mipped
   resource starts `true`. The mip-weighting is still correct (the arenas must stay in step, and it
   covers any future non-seeded route) but it is not a live catch, and §11 claimed it was.
5. **G-A2 cannot be an integration test.** `compile_per_resource_reference` is `#[cfg(test)]`, so it
   does not exist in the rlib that `tests/*.rs` link against. The differential must live in a
   `#[cfg(test)] mod tests` inside `src/`. Carried to part 2.

Two additions §11 did not name: `res_state_total()` (the replacement G-E needs, since
`state.capacity()` measures a property of `Vec` rather than of the plan) and a documented note that
`with_capacity(max_res, …)` sizes `state` by RESOURCE count, so a mipped declarator regrows it on
frame 1 — the same class of fact §11 already records for `frame_driver.rs:188`.
