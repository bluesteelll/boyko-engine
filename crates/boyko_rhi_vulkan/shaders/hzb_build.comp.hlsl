// VG R3 piece 1, step P1-3: the hierarchical-Z depth-pyramid BUILD pass
// (`hzb_build.comp.hlsl`).
//
// One `R32_SFLOAT` image with a real Vulkan mip chain, reduced by `min`, level 0 being
// `prev_pow2` of each SOURCE axis rather than the source extent itself. This step BUILDS that
// pyramid and **NOTHING READS IT** — not the batch cull, not the raster, not any resolve. The
// cull, the occlusion test and the late pass are piece 3
// (`docs/VG-R3-P1-PYRAMID-PLAN.md` §1, "Out, deliberately"). A pass that is dispatched and read by
// nothing is the same null-control discipline rung R2c0 shipped `vb_batch_cull.comp.hlsl` under:
// the machinery present, dispatched, and provably changing nothing observable, one rung before the
// decision that consumes it arrives.
//
// The host oracle is `boyko_render::hzb::build_pyramid`, and this shader must agree with it
// BIT-EXACTLY. That is a decidable demand rather than an ambitious one: the ONLY float operation in
// the entire build is `min` — exact selection, no rounding, no ULP question — so "equals the
// oracle" is checkable to `to_bits()` at every texel of every level (plan §5, gate G3). Every
// integer map below (`first_source`, the level extents, the 2×2 footprint) is either PUSHED by the
// host from that same oracle or transcribed from it here with its derivation.
//
// # The reduce is `min`, and under reverse-Z that is the FARTHEST surface
//
// This engine renders hardware reverse-Z: `VK_COMPARE_OP_GREATER`, depth cleared to `0.0`, so a
// LARGER stored depth is NEARER and `0.0` is the far plane. The late-pass reject predicate piece 3
// will run is `depth_near < occ`, and its soundness needs `occ <= D[p]` for every source pixel `p`
// the sampled texels cover — a LOWER BOUND over a footprint, which is a `min`. Under reverse-Z the
// smallest depth is the surface FURTHEST from the eye, so each texel holds the depth of the thing
// furthest away in its footprint and an instance may only ever be rejected if it is behind even
// that.
//
// A `max` here would hold the NEAREST surface in each footprint and would reject anything behind
// the nearest occluder anywhere in its screen rect — deleting geometry visible through gaps. It is
// silently wrong in the one direction a static golden cannot see, which is why the direction is
// derived here rather than left to the reader.
//
// # TWO VARIANTS, ONE ENTRY POINT — and the discriminator is a push constant
//
// `pc.base_level == 0` <=> the BASE variant: level 0 is produced from the SOURCE depth through the
// oracle's `first_source` map. Otherwise the REDUCE variant: level `d` is produced from mip `d-1`
// of the pyramid itself through the ordinary 2×2 footprint.
//
// They are ONE `main` rather than two entry points and rather than a `-D` variant pair, because the
// part that is hard to get right — the tile geometry, the LDS chain, the four barriers, the
// boundary rule — is IDENTICAL in both and must therefore exist exactly once. Only the innermost
// tap gather differs, and it is two small leaf functions. A `-D` split would have duplicated the
// chain into two `.spv` that could drift, and would have owed the variant manifest a row for a
// difference of four lines.
//
// The discriminator is a push constant, so the branch is DYNAMICALLY UNIFORM across the whole
// dispatch: no barrier below sits inside it, and no lane in a group can take a different arm.
//
// # THE TILE GEOMETRY IS DERIVED IN THE FIRST OUTPUT LEVEL'S INDEX SPACE, NEVER THE SOURCE
//
// One workgroup owns a **32×32 tile of the pass's FIRST OUTPUT LEVEL `d`**. Each of its 16×16
// threads owns a 2×2 block of that level. The host dispatches `groups[a] = ceil(E_a(d) / 32)`.
//
// The output level is the ONLY index space in which the two variants have the same shape. Level 0
// is *not* a halving of the depth — `P = prev_pow2(S)` gives `P <= S < 2P`, so a level-0 texel's
// source preimage is 1 or 2 pixels per axis and the map is `first_source`, not a shift. Every LATER
// level *is* an exact halving of its predecessor, which is precisely what a power-of-two base buys
// (`boyko_render::hzb`, "Why the base is `prev_pow2`, not the source extent"). Deriving the tile in
// SOURCE space would therefore need two different tilings; deriving it in OUTPUT space needs one,
// and everything else falls out of it: <=16 taps per thread, 1360 bytes of LDS, four barriers, six
// levels per pass.
//
// # ONE PASS WRITES UP TO SIX LEVELS, `d .. d+5`
//
// `32 = 2^5`, so the tile collapses to exactly 1×1 at level `d+5`: the six levels a pass writes
// have **no cross-tile dependency**, because every texel of every one of them is a fold of level-`d`
// texels this group already owns. `ceil(MAX_HZB_LEVELS / 6) = ceil(17 / 6) = 3` passes cover the
// deepest pyramid the oracle admits.
//
// **32 rather than 64 is a GATE-REACHABILITY choice, not a performance one** (plan §2). At six
// levels per pass the THIRD dispatch is first reached when `levels >= 13`, i.e. when
// `prev_pow2(max(W, H)) >= 4096`, i.e. at a 4096-wide source — exactly Vulkan's guaranteed
// `maxImageDimension2D` floor, so the deepest structural case runs on ANY conformant device. A
// 64-texel tile would write seven levels per pass and would not need a third dispatch until 16384,
// which no minimum-conformant device can allocate: the deepest case would be unreachable by any
// gate, on any machine, and would ship untested.
//
// # THE BOUNDARY RULE — one rule, applied everywhere, and there is no clamp
//
// A lane's output texel at level `m` **EXISTS** iff `t.x < E_x(m) && t.y < E_y(m)`. A lane whose
// output texel does not exist **issues no tap and stores nothing**; its contribution to the next
// level is the `min` identity `+INFINITY` (`HZB_IDENTITY`). A live lane folds exactly those taps of
// its footprint that lie inside the SOURCE / FINE extent, and an out-of-extent tap is **not
// issued** — the extent test precedes the address computation, so an out-of-bounds `Load` is
// UNSPELLABLE here.
//
// ⚠️ **The reason is ORACLE AGREEMENT, not memory safety, and the difference matters.** An earlier
// draft of this header said the guard exists because `robustBufferAccess` is off. That feature
// governs BUFFERS — uniform/storage/texel buffers and vertex input. `gSrcDepth` is a sampled image
// and `gFine` a storage image, and Vulkan bounds image accesses UNCONDITIONALLY with no feature
// required: an out-of-range fetch returns undefined values and an out-of-range write is discarded.
// Neither can fault. So the guard is not belt-and-braces around a robustness bit that someone might
// later turn on — remove it and the fold takes UNDEFINED DATA where the oracle's
// `if (sx >= fine_w) { continue; }` contributes `+INFINITY`. Under reverse-Z an undefined read of
// zero is the FAR plane, so the pyramid comes out too small: conservative, invisible in any golden,
// and visible in G3 only if its pattern happens to expose it. Round 2 of the design review found
// exactly such a read (`first(5) = 9` against a 7-pixel source) and it was an oracle-agreement
// defect misfiled as a memory-safety one.
//
// Equivalence with the oracle is by **IDENTITY** — `min(+INFINITY, x) == x` exactly, for every
// finite `x` and for `+INFINITY` itself — never by idempotence. So folding a non-existent texel's
// `HZB_IDENTITY` is not "harmless because `min` repeats"; it is the same value the oracle's
// `if (sx >= fine_w) { continue; }` produces, bit for bit.
//
// The two cases the review demanded be shown exact:
//
//   * **Odd extents live only in the SOURCE**, absorbed by the base map's partition. No level
//     extent is ever odd except the terminal 1, since `E_a(k)` is `P_a >> k` or the clamped 1. At
//     `S = 7, P = 4` the level-0 footprints are `{0,1} {2,3} {4,5} {6}` — oddness handled by
//     footprint LENGTH, not by a clamp.
//   * **A clamped-to-1 axis** bottoms out: at `S = 3, P = 2`, `E_y(1) = 1`, and the second tap is
//     dropped — exactly the oracle's `if (sy >= fine_h) { continue; }`.
//
// The rule also propagates without a second test. If a level-`m` texel does not exist then NEITHER
// of its two children at level `m-1` exists: when `E(m-1) >= 2` the extents are exact halves so
// `2t >= 2·E(m) == E(m-1)`, and when `E(m-1) == 1` then `E(m) == 1` and `t >= 1` forces
// `2t >= 2 > 1`. So a dead lane's LDS entry is `HZB_IDENTITY` by construction at every level, and
// the extent test is performed EXACTLY ONCE per texel — where that texel is produced.
//
// # NaN POLICY — spelled out, because `min()` gets it BACKWARDS
//
// A NaN depth is UNKNOWN, and the only conservative reading of unknown is "infinitely far", so
// `hzb_min` collapses to `-INFINITY` and the verdict piece 3 computes can never be `Reject`. That
// is `boyko_render::hzb::conservative_min`, matched exactly.
//
// ⚠️ **HLSL's `min()` MUST NOT be used here.** DXC lowers it to `OpFMin` / `NMin`, under which a
// NaN operand is not propagated and not skipped — the OTHER operand is silently taken. This
// repository has a recorded incident on precisely that lowering (`clamp(NaN, 0, 1)` collapsing to
// `0`, i.e. a BLACK pixel). So the test is spelled `isnan(a) || isnan(b)` and the comparison is
// spelled `b < a ? b : a`, which lowers to a compare-and-select the `.spv` census can see and count.
//
// ## ⚠️ What this module does NOT ask for, and what rides on that
//
// The census pins `OpCapability` to exactly `["Shader"]` and reads the absences as benefits. One
// absence is not a benefit: **`SignedZeroInfNanPreserve` is not requested** (nor `DenormPreserve`),
// so without the matching `OpExecutionMode` an implementation is not obliged to preserve Inf, NaN or
// signed zero in fp32, nor to keep denormals. Three things in this file rest on guarantees it does
// not ask for:
//
//   * `HZB_IDENTITY` is `+INFINITY`, and it is the fold identity on EVERY partial tile — the
//     mainline path in the last pass of every pyramid, not a contract-only corner.
//   * `isnan` above IS the NaN policy. On an implementation permitted to assume no-NaN, `OpIsNan`
//     may fold to `false` and `hzb_min` degenerates to plain `min` — the very outcome the
//     `OpExtInst == 0` pin exists to prevent, reached from the other side.
//   * "the only float operation is `min`, so there is no ULP question" is true of the SELECTION and
//     not of the COMPARISON: `OpFOrdLessThan` under denorm-flush disagrees with the Rust oracle's
//     `<` on denormal operands, and reverse-Z puts distant geometry at exactly the near-zero depths
//     where denormals live.
//
// The practical risk is low — every desktop driver reports `shaderSignedZeroInfNanPreserveFloat32`
// — and the failure direction is conservative (a flushed `+INF` folds toward SMALLER, which cannot
// delete geometry). But the step's thesis is that a bit-exact comparison is DECIDABLE, and this is
// the assumption that thesis rests on. It is stated here rather than discovered as a red G3 on
// somebody else's hardware. Requesting the execution mode would mean enabling a device feature at
// context creation, which is a change with its own justification and is not piece 1's.
//
// # INVARIANT HZB-BARRIER-UNIFORM — all four barriers sit in uniform control flow
//
// Every `GroupMemoryBarrierWithGroupSync()` below is OUTSIDE every `tid`-dependent `if`, outside the
// `pc.base_level` arm, and outside every `pc.level_count` guard. A divergent barrier is undefined
// behaviour, and the tempting shape — early-out when `pc.level_count < 6` — is exactly how it gets
// introduced. **This shader never early-outs.** The whole chain runs on every dispatch and only the
// STORES are guarded, so the number of barriers a lane executes is a compile-time constant.
//
// # INVARIANT HZB-LDS-DISJOINT — one region per level, and that is what buys one barrier per step
//
// The `gs` array is partitioned into four disjoint regions, one per intermediate level:
//
//   | region        | level | grid  | offset | floats |
//   |---------------|-------|-------|--------|--------|
//   | `GS_BASE_D1`  | d+1   | 16×16 | 0      | 256    |
//   | `GS_BASE_D2`  | d+2   | 8×8   | 256    | 64     |
//   | `GS_BASE_D3`  | d+3   | 4×4   | 320    | 16     |
//   | `GS_BASE_D4`  | d+4   | 2×2   | 336    | 4      |
//
// 340 floats, 1360 bytes — far inside the 16 KiB `maxComputeSharedMemorySize` floor. Level `d+5` is
// a single texel, computed by thread (0,0) from the four `GS_BASE_D4` entries and written straight
// to `gDst5`, so it needs no region.
//
// A single reused 16×16 array would RACE its own reads: thread (0,0) reads index 1 while thread
// (1,0) writes index 1, in the same step. Fixing that in place needs a read-barrier-write sandwich —
// TWO barriers per step, which is what `vb_classify_scan.comp.hlsl`'s in-place Hillis-Steele scan
// pays. Disjoint regions make each step read-only in the region below and write-only in its own, so
// ONE barrier per step separates them. Four steps, four barriers.
//
// Every region is written in FULL before the barrier that publishes it — all 256 `GS_BASE_D1`
// entries by all 256 threads, all 64 `GS_BASE_D2` entries by the 64 threads with `tid < 8`, and so
// on — so no lane ever reads an undefined LDS word. The stores that are conditional are the IMAGE
// stores; the LDS stores never are.
//
// # NO INTRA-DISPATCH IMAGE BARRIER IS NEEDED
//
// No thread ever READS level `d` back out of `gDst0` — each keeps its four values in registers — and
// levels `d .. d+5` have no cross-tile dependency (the tile collapses to 1×1 at `d+5`). So the six
// mips a pass writes need no barrier BETWEEN each other. The only ordering this construction owes is
// PASS-to-PASS: the next dispatch reads mip `d+5` as its `gFine`, and that edge is the graph's to
// declare (step P1-5).
//
// # WHAT THE HOST BINDS, INCLUDING WHAT A PASS NEVER READS
//
// Both `gSrcDepth` (@0) and `gFine` (@1) are STATICALLY used by the module — the branch that skips
// one is a runtime branch, not a compile-time one — so the host must bind a VALID view to both on
// every pass. A base pass never accesses `gFine` and a reduce pass never accesses `gSrcDepth`; the
// `pc.base_level` branch is what guarantees it. Descriptors bound-but-unread are the same R2
// contract `sdf_forward_march.comp.hlsl` documents for its own reserved slot.
//
// The same holds for the six destinations: a pass with `pc.level_count < 6` never STORES to the
// tail bindings, but they are declared, so valid views must still be bound there.
//
// Compiled offline (hermetic build) with:
//   dxc.exe -spirv -T cs_6_0 -E main -fspv-target-env=vulkan1.3 hzb_build.comp.hlsl \
//       -Fo hzb_build.comp.spv

// The workgroup width AND height. One spelling in this file, used by `[numthreads]` below rather
// than a constant sitting beside a literal that could drift from it — the same shape
// `vb_batch_cull.comp.hlsl`'s `LOCAL_SIZE_X` and `vb_classify_scan.comp.hlsl`'s `SCAN_BLOCK` use.
//
// The host mirrors it as `boyko_rhi_vulkan::compute::HZB_BUILD_LOCAL_SIZE`. The two spellings CANNOT
// be one symbol across the language boundary, so they are held together at the ARTIFACT:
// `tests/hzb_build_spv_sync.rs` reads the compiled `LocalSize` out of the module and asserts it
// equals the host constant.
static const uint LOCAL_SIZE = 16u;

// The tile edge, in texels of the pass's FIRST OUTPUT LEVEL `d`. SPELLED AS `LOCAL_SIZE * 2` rather
// than as `32u`, because that IS the relation — each thread owns a 2×2 block — and a literal here
// could drift from `[numthreads]` while every other constant in the file stayed consistent.
//
// It is also the host's DISPATCH DIVISOR — `groups[a] = ceil(E_a(d) / TILE)` — mirrored as
// `HZB_BUILD_TILE`. A host that under-dispatches leaves tail texels holding the boot clear `0.0`,
// which under reverse-Z reads as the far plane and makes the pyramid claim that nothing is there.
//
// ⚠️ The cross-language tie is INDIRECT and the review that found it said so plainly: no census
// field reads a "tile" out of the module, because the tile appears there only as the anonymous
// multipliers `TILE >> k`. `tests/hzb_build_spv_sync.rs` therefore pins it through those multipliers
// — the module must DECLARE `%uint_TILE` and must NOT declare `%uint_(2*TILE)` — which is what
// catches a `TILE = 64u` here against an unchanged `[numthreads(16,16,1)]`: a drift that leaves
// every opcode count, the LocalSize, the bindings, the push layout and the LDS length untouched
// while half of every level goes unwritten.
static const uint TILE = LOCAL_SIZE * 2u;

// How many levels one dispatch of this shader writes, `d .. d+LEVELS_PER_PASS-1`. It is the SIX
// destination bindings below and the `TILE >> 5 == 1` collapse, not a bound any expression tests —
// `pc.level_count` carries the live count and is `1..=LEVELS_PER_PASS`. Mirrored as
// `HZB_LEVELS_PER_PASS`, which is what makes the host's pass count `ceil(levels / 6)`.
static const uint LEVELS_PER_PASS = 6u;

// The `min` IDENTITY, by BIT PATTERN so the value is unambiguous in the source rather than resting
// on how a literal is parsed: `0x7F800000` is `+INFINITY`. A lane with no output texel contributes
// this and taps nothing, and `min(+INFINITY, x) == x` exactly — see the boundary rule.
static const float HZB_IDENTITY = asfloat(0x7F800000u);

// The UNKNOWN-depth answer: `0xFF800000` is `-INFINITY`. `hzb_min` collapses to it on any NaN, so an
// unknown depth reads as "infinitely far" and can never let piece 3's predicate reject. Matches
// `boyko_render::hzb::conservative_min`.
static const float HZB_UNKNOWN = asfloat(0xFF800000u);

// binding 0: the SOURCE depth (read-only, point-fetched). BASE PASS ONLY — a reduce pass never
// touches it, and the host must still bind a valid view (see the header).
//
// Tapped with `.Load`, never a filtered sample — and the reason is the BASE MAP, not a format
// feature. A level-0 texel's preimage is a 1-or-2-pixel INTEGER interval that `first_source`
// computes exactly; a filtered sample would blend across it and there is no filter weight that
// equals `min`. (The `SAMPLED_IMAGE_FILTER_LINEAR` question is a real one but it belongs to a
// DIFFERENT binding: it is about `R32_SFLOAT`, i.e. the PYRAMID, which piece 3 will read — see plan
// §7's carried-forward note. This binding is the `D32_SFLOAT` depth attachment.)
[[vk::binding(0, 0)]] Texture2D<float> gSrcDepth : register(t0);

// binding 1: mip `d-1` of the pyramid (RW view, read here). REDUCE PASSES ONLY — the base pass
// never touches it, and `pc.fine_extent` is unspecified on a base pass, which is the second reason
// the arm below is an `if`/`else` and never a `?:`.
[[vk::binding(1, 0)]] [[vk::image_format("r32f")]] RWTexture2D<float> gFine : register(u1);

// bindings 2..7: the six destination mips, `d` through `d+5`.
//
// Six SEPARATE named bindings rather than a `RWTexture2D<float> gDst[6]` array, for two reasons: an
// array would need `shaderStorageImageArrayDynamicIndexing` if DXC declined to unroll the index, and
// each store site here NAMES the level it writes, so a mis-wired mip is a readable diff rather than
// an off-by-one in a subscript.
//
// `[[vk::image_format("r32f")]]` on every RW view is MANDATORY: the image is `R32_SFLOAT`, and
// without the decoration the storage image is "unknown format", which needs
// `shaderStorageImageWriteWithoutFormat` (OFF at device creation).
[[vk::binding(2, 0)]] [[vk::image_format("r32f")]] RWTexture2D<float> gDst0 : register(u2);
[[vk::binding(3, 0)]] [[vk::image_format("r32f")]] RWTexture2D<float> gDst1 : register(u3);
[[vk::binding(4, 0)]] [[vk::image_format("r32f")]] RWTexture2D<float> gDst2 : register(u4);
[[vk::binding(5, 0)]] [[vk::image_format("r32f")]] RWTexture2D<float> gDst3 : register(u5);
[[vk::binding(6, 0)]] [[vk::image_format("r32f")]] RWTexture2D<float> gDst4 : register(u6);
[[vk::binding(7, 0)]] [[vk::image_format("r32f")]] RWTexture2D<float> gDst5 : register(u7);

// The 72-byte build push. EVERY per-level extent is PUSHED; this shader derives NONE of them.
//
// `prev_pow2`, `msb` and `max(1, base >> k)` live in `boyko_render::hzb` and ONLY there (plan §4) —
// a second implementation of those formulas anywhere else is exactly what the plan forbids, and
// `HzbPlan::level_extent` on the host already holds every value this struct carries. So the host
// hands them over and the shader re-derives nothing, which is also why a base-map disagreement can
// only ever be a SHADER bug and never a math one.
struct HzbBuildPush {
    uint2 src_extent;   // @0   S — the SOURCE depth extent. Base pass only.
    uint2 fine_extent;  // @8   E(d-1). Reduce passes only.
    uint2 out_extent0;  // @16  E(d). On the BASE pass this is also P = prev_pow2(S) per axis.
    uint2 out_extent1;  // @24  E(d+1)
    uint2 out_extent2;  // @32  E(d+2)
    uint2 out_extent3;  // @40  E(d+3)
    uint2 out_extent4;  // @48  E(d+4)
    uint2 out_extent5;  // @56  E(d+5)
    uint  base_level;   // @64  d — and the variant discriminator: 0 <=> the BASE variant.
    // @68 — how many levels THIS pass writes, in `1 ..= LEVELS_PER_PASS`. Levels past it carry
    // UNSPECIFIED padding in the fields above and MUST NOT be read; the `k < level_count` guard on
    // every store below is what keeps that true.
    uint  level_count;
};
[[vk::push_constant]] HzbBuildPush pc;

// The `groupshared` reduce chain. See INVARIANT HZB-LDS-DISJOINT for the region table and for why
// disjointness is what buys one barrier per step.
static const uint GS_BASE_D1 = 0u;    // 16×16, level d+1
static const uint GS_BASE_D2 = 256u;  //  8×8,  level d+2
static const uint GS_BASE_D3 = 320u;  //  4×4,  level d+3
static const uint GS_BASE_D4 = 336u;  //  2×2,  level d+4
static const uint GS_WORDS = 340u;    // 1360 bytes
groupshared float gs[GS_WORDS];

// The conservative reduce, matching `boyko_render::hzb::conservative_min` EXACTLY: `-INFINITY` if
// either operand is NaN, else the smaller.
//
// ⚠️ NOT `min(a, b)`. See the header's NaN section: `OpFMin`/`NMin` takes the OTHER operand on a NaN
// instead of propagating, and this repository has an incident on exactly that. The explicit
// `isnan` + compare-and-select is the lowering this file wants and the one the census can count.
float hzb_min(float a, float b) {
    if (isnan(a) || isnan(b)) {
        return HZB_UNKNOWN;
    }
    return b < a ? b : a;
}

// The FIRST source pixel of level-0 texel `t` on one axis: `ceil(t·s/p)`, the oracle's
// `HzbAxis::first_source` written in the `(t*s + p - 1) / p` integer form. `p >= 1` always (every
// level extent is `max(1, ...)`), so the division is defined.
//
// ⚠️ THE `t == p` CASE IS NOT COSMETIC. All arithmetic here is `uint`. For `t <= p-1` the numerator
// is bounded: `t*s + p - 1 <= (p-1)*s + p - 1 = p*s - s + p - 1 <= p*s - 1 <= 2^32 - 1`, using
// `p <= s <= 65536 = MAX_HZB_EXTENT` (so `p*s <= 2^32`) and `p - 1 <= s - 1`. No overflow. But at
// `t == p` with `p == s == 65536`, `p*s == 2^32` WRAPS TO 0 and the general form returns
// `(0 + 65535) / 65536 == 0` instead of `s`. The answer is `s` by definition — `first(p) == s` is
// what makes the preimages tile `[0, s)` — so it is returned directly. `t == p` is reached on every
// base pass, not only at the cap: it is the exclusive END of the last live texel's preimage.
uint hzb_first_source(uint t, uint s, uint p) {
    if (t == p) {
        return s;
    }
    return (t * s + p - 1u) / p;
}

// The BASE variant's tap gather for ONE live level-0 texel: the `min` over its source preimage
// `[first(t), first(t+1))` on each axis.
//
// Since `P = prev_pow2(S)` gives `P <= S < 2P`, consecutive `first` values differ by 1 or 2, so a
// thread's 2×2 block taps at most 4×4 = 16 source pixels.
//
// Every tap is in bounds without a guard, and that is a property of the map rather than luck:
// `first` is non-decreasing and `first(P) == S`, so a LIVE texel (`t < P`, hence `t + 1 <= P`) has
// `x_hi <= S`. The extent test that makes the lane live has already happened at the call site — the
// boundary rule's "the extent test precedes the address computation".
float hzb_base_texel(uint2 t) {
    const uint x_lo = hzb_first_source(t.x, pc.src_extent.x, pc.out_extent0.x);
    const uint x_hi = hzb_first_source(t.x + 1u, pc.src_extent.x, pc.out_extent0.x);
    const uint y_lo = hzb_first_source(t.y, pc.src_extent.y, pc.out_extent0.y);
    const uint y_hi = hzb_first_source(t.y + 1u, pc.src_extent.y, pc.out_extent0.y);

    float m = HZB_IDENTITY;
    for (uint y = y_lo; y < y_hi; ++y) {
        for (uint x = x_lo; x < x_hi; ++x) {
            // `.Load`, point, never a filtered sample — see binding 0.
            m = hzb_min(m, gSrcDepth.Load(int3(int(x), int(y), 0)));
        }
    }
    return m;
}

// The REDUCE variant's tap gather for ONE live level-`d` texel: the `min` over its 2×2 footprint in
// mip `d-1`, with an out-of-extent tap NOT ISSUED — the oracle's `if (sx >= fine_w) { continue; }`,
// which is what a bottomed-out (clamped-to-1) axis needs. Again <=16 taps per thread, since a 2×2
// block of level `d` reads a 4×4 block of level `d-1`.
float hzb_fine_texel(uint2 t) {
    const uint2 f = t * 2u;
    float m = HZB_IDENTITY;
    for (uint dy = 0u; dy < 2u; ++dy) {
        const uint sy = f.y + dy;
        if (sy >= pc.fine_extent.y) {
            continue;
        }
        for (uint dx = 0u; dx < 2u; ++dx) {
            const uint sx = f.x + dx;
            if (sx >= pc.fine_extent.x) {
                continue;
            }
            m = hzb_min(m, gFine[uint2(sx, sy)]);
        }
    }
    return m;
}

// The `min` over the 2×2 block at `c` of an LDS region whose grid is `fine_pitch` wide.
//
// No extent test, and none is owed: every entry of every region is written before the barrier that
// publishes it, and a non-existent texel's entry already holds `HZB_IDENTITY` (the boundary rule
// propagates — see the header). The extent test happens once, where the texel is produced.
//
// ⚠️ THE ASSOCIATION DIFFERS FROM THE ORACLE'S, AND THAT IS SAFE FOR A REASON WORTH KEEPING.
// `build_pyramid` folds a 2×2 as a LEFT FOLD seeded with `+INFINITY`; this is a BALANCED TREE. They
// agree only because `conservative_min` is not commutative on `±0.0` yet both shapes compute *the
// earliest minimal element in program order*: `hzb_min(a, b)` returns `b` only on a strict `b < a`,
// so it keeps `a` on a tie, and the tree's `hzb_min(L, R)` therefore keeps `L`, which is the
// earliest occurrence in the left pair and hence the earliest overall. The operand order
// ((0,0),(1,0),(0,1),(1,1)) is the same in both, and the seed is bit-neutral since
// `hzb_min(+INF, x) == x` including for `x == +INF`.
//
// This matters because G3 compares `to_bits()`, and `+0.0` and `-0.0` differ there. A future
// "simplification" that reassociates is still correct; one that reorders the operands, or reaches
// for a `min3`-style helper, is NOT — and G3 would only catch it if its pattern happens to contain
// both zero signs.
float hzb_fold_lds(uint region, uint fine_pitch, uint2 c) {
    const uint2 f = c * 2u;
    const uint r0 = region + f.y * fine_pitch + f.x;
    const uint r1 = region + (f.y + 1u) * fine_pitch + f.x;
    return hzb_min(hzb_min(gs[r0], gs[r0 + 1u]), hzb_min(gs[r1], gs[r1 + 1u]));
}

[numthreads(LOCAL_SIZE, LOCAL_SIZE, 1)]
void main(uint3 gid : SV_GroupID, uint3 tid : SV_GroupThreadID) {
    // Both in level-`d` index space — the ONE space this shader tiles in (see the header).
    const uint2 tile_base = gid.xy * TILE;
    const uint2 block_base = tile_base + tid.xy * 2u;

    // ---- LEVEL d: four texels per thread, from the source or from mip d-1. ----
    //
    // `q[i]` is `HZB_IDENTITY` for a texel that does not exist, and NO tap is issued for it.
    // ⚠️ `[unroll]` HERE IS LOAD-BEARING, AND NOT IN THE WAY IT LOOKS. It was read as decoration
    // once — "DXC plainly did not unroll it, since `op_image_write == 6` rather than 9" — and
    // removing it was MEASURED to change the module by exactly one token:
    // `OpLoopMerge %90 %88 Unroll` becomes `... None`, and nothing else in 15856 bytes moves.
    //
    // So the attribute does not unroll the loop in DXC; it RECORDS the request in the SPIR-V, for
    // the driver's own backend — which is where the unroll that matters happens, and where `q`'s
    // dynamic indexing gets promoted out of function-scope memory. Dropping it silently hands that
    // decision back to a compiler that was never told. The hint's survival is pinned as
    // `loop_unroll_hints` in `tests/hzb_build_spv_sync.rs`.
    //
    // Keeping the loop rolled IN THE MODULE is what lets `op_image_write == 6` mean "one store per
    // destination mip" — an unrolled body would make it 9 and the pin would lose its reading.
    float q[4];
    [unroll]
    for (uint i = 0u; i < 4u; ++i) {
        const uint2 t = block_base + uint2(i & 1u, i >> 1u);
        float m = HZB_IDENTITY;
        if (t.x < pc.out_extent0.x && t.y < pc.out_extent0.y) {
            // An `if`/`else` and deliberately NOT a `?:`: HLSL does not specify `?:` as
            // short-circuiting, and the untaken arm would read the OTHER pass's binding under the
            // OTHER pass's extent field — which is UNSPECIFIED on this pass. "No tap is issued" has
            // to be structural, not a property of an evaluation rule.
            if (pc.base_level == 0u) {
                m = hzb_base_texel(t);
            } else {
                m = hzb_fine_texel(t);
            }
            // `0 < level_count` always holds (a pass writes at least one level); kept in the same
            // shape as the five guards below so no reader has to check whether level `d` is special.
            if (0u < pc.level_count) {
                gDst0[t] = m;
            }
        }
        q[i] = m;
    }

    // ---- LEVEL d+1: the per-thread fold, and the LDS seed. ----
    const float v1 = hzb_min(hzb_min(q[0], q[1]), hzb_min(q[2], q[3]));
    const uint2 t1 = gid.xy * (TILE >> 1u) + tid.xy;
    if (1u < pc.level_count && t1.x < pc.out_extent1.x && t1.y < pc.out_extent1.y) {
        gDst1[t1] = v1;
    }
    // UNCONDITIONAL — the IMAGE store is guarded, the LDS store never is. All 256 entries must be
    // defined before the barrier publishes them, and a dead lane's `HZB_IDENTITY` is exactly what
    // the next fold needs (INVARIANT HZB-LDS-DISJOINT).
    gs[GS_BASE_D1 + tid.y * LOCAL_SIZE + tid.x] = v1;

    // Barrier 1 of 4. In UNIFORM control flow — INVARIANT HZB-BARRIER-UNIFORM.
    GroupMemoryBarrierWithGroupSync();

    // ---- LEVEL d+2: 8×8 threads fold the 16×16 region. ----
    if (tid.x < 8u && tid.y < 8u) {
        const float v2 = hzb_fold_lds(GS_BASE_D1, LOCAL_SIZE, tid.xy);
        const uint2 t2 = gid.xy * (TILE >> 2u) + tid.xy;
        if (2u < pc.level_count && t2.x < pc.out_extent2.x && t2.y < pc.out_extent2.y) {
            gDst2[t2] = v2;
        }
        gs[GS_BASE_D2 + tid.y * 8u + tid.x] = v2;
    }

    // Barrier 2 of 4.
    GroupMemoryBarrierWithGroupSync();

    // ---- LEVEL d+3: 4×4 threads fold the 8×8 region. ----
    if (tid.x < 4u && tid.y < 4u) {
        const float v3 = hzb_fold_lds(GS_BASE_D2, 8u, tid.xy);
        const uint2 t3 = gid.xy * (TILE >> 3u) + tid.xy;
        if (3u < pc.level_count && t3.x < pc.out_extent3.x && t3.y < pc.out_extent3.y) {
            gDst3[t3] = v3;
        }
        gs[GS_BASE_D3 + tid.y * 4u + tid.x] = v3;
    }

    // Barrier 3 of 4.
    GroupMemoryBarrierWithGroupSync();

    // ---- LEVEL d+4: 2×2 threads fold the 4×4 region. ----
    if (tid.x < 2u && tid.y < 2u) {
        const float v4 = hzb_fold_lds(GS_BASE_D3, 4u, tid.xy);
        const uint2 t4 = gid.xy * (TILE >> 4u) + tid.xy;
        if (4u < pc.level_count && t4.x < pc.out_extent4.x && t4.y < pc.out_extent4.y) {
            gDst4[t4] = v4;
        }
        gs[GS_BASE_D4 + tid.y * 2u + tid.x] = v4;
    }

    // Barrier 4 of 4.
    GroupMemoryBarrierWithGroupSync();

    // ---- LEVEL d+5: ONE texel, the whole tile. `TILE >> 5 == 1`, which is what makes the six
    // levels of a pass free of cross-tile dependencies — the tile's own level-`d+5` texel index is
    // the group id itself, and it needs no LDS region of its own.
    if (tid.x == 0u && tid.y == 0u) {
        const float v5 = hzb_fold_lds(GS_BASE_D4, 2u, uint2(0u, 0u));
        const uint2 t5 = gid.xy;
        if (5u < pc.level_count && t5.x < pc.out_extent5.x && t5.y < pc.out_extent5.y) {
            gDst5[t5] = v5;
        }
    }
}
