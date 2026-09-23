# Architecture: L9, contact reuse: an exact separated-pair fast path plus reuse of manifolds within a tolerance

Read-only design, written against `D:/wt/joltab` at f236ebdd. Nothing was built, run or written. Figures marked "arith." are calculated from P0b spans and counts, not measured. Research input: the relayed brief. I had no Agent tool, so no further researcher run was possible, and all web facts come from that brief.

## Goal
- **Owner's goal: parity with Jolt per contact.** At W=1 the collision side is the gap, because solve per manifold is already at parity (1.24 µs against 1.30–1.49 µs).
- **What L9 attacks.** The narrowphase: 3.142 ms per step, 0.695 µs per manifold, 0.329 µs per pair. L5 gives exactly 0 here at W=1.
- **Two parts, gated separately:**
  - **L9a is exact.** Separated box pairs skip the SAT (Separating Axis Test) tail, and every box pair reads a per-row orientation frame instead of converting the quaternion again. Bit-identical: no pin moves.
  - **L9b changes values** (owner-approved as D4). A touching box pair whose relative pose has moved less than τ since its last full collision reuses its contact features. The kept points are carried with their bodies, and separation is recomputed from the current poses.
- **Targets (arith., W=1, J):**

| metric | now | after L9 |
|---|---|---|
| t_np per step | 3.142 ms | 0.64–1.74 ms (Δ −1.4 to −2.5) |
| t_np per manifold | 0.695 µs | 0.14–0.39 µs |
| t_np per pair | 0.329 µs | 0.07–0.18 µs |
| heap allocations per step | 0 | 0 |
| new scopes | — | 0 |

## Context and constraints
- **Counts per step (P0b, J):**
  - 9,561 pairs, 4,519–4,524 manifolds, 16,888 points.
  - 5,037 pairs (53 %) are box-box and SAT-separated. By the scene's geometry these are ~2,240 lateral, ~2,030 diagonal and ~1,015 slab-to-upper-layer pairs, all of which pass the bounding-sphere broadphase (`systems.rs:280-283`).
  - J has no spheres.
- **Tree facts the design rests on:**
  - `sat` evaluates all 15 axes, then rejects on the first negative one (`box_box.rs:275-299`).
  - `Obb::new` calls `Mat3::from_quat` per pair per body (`:156-170`).
  - Face points are kept iff `separation <= 0` (`:938-949`). Edge contacts are never filtered (`:1017`).
  - Manifold anchors are world points. `separation` is fixed for the whole step and feeds `bias_rate·separation` in every substep (`colored.rs:1778-1779`, `:2048`, `:2094-2098`).
  - There is no slop and no speculative handling: a positive separation gets the same soft bias (`soft_step.rs:82`).
  - Friction and restitution are read from bodies when the solve is built (`colored.rs:1760-1761`), not from the manifold.
  - The axis cache is an in-place table, set only when a pair makes a contact (`axis_cache.rs:319-354`).
  - Carries keyed by row go through `RowRemap::{pair,row}` plus a `RemapCursor` (`row_identity.rs:113-245`). A pair whose order flipped translates to `None` (`:146-157`).
  - `ColliderShape` is only `Sphere | Box`. The `Manifold` currency stays unchanged (`manifold.rs:84-122`), so **the solver is untouched**.
- **Binding rules:**
  - principle 0 (all state in ECS-owned `ScratchColumn`s);
  - no hot-path allocation;
  - lock-free;
  - bit identity across W;
  - KC-37 (replays reproduce on any machine, at any W, under any id assignment);
  - every gate can fail;
  - the P0 protocol plus per-contact reporting;
  - A7-R1 (10 mm), A7-R2, the sleep suites, `support_loss_wakes_sleepers` and `frozen_island_warm_start` are never loosened.

## Key decisions

### D1: No tolerance-based caching of "no contact". Separated pairs use an exact cached separating axis (L9a)
- **What:**
  - (i) `sat` returns at the first axis with `depth < 0` in canonical order.
  - (ii) A separated pair's axis is carried in its tag. On the next step that axis is evaluated first; if `depth < 0` still holds, the result is "separated".
  - (iii) If the axis now overlaps or is degenerate (`eval_axis` → `None`), the full SAT runs.
- **Why:**
  - Both (i) and (ii) return exactly what today's SAT returns, because today returns `None` iff any axis is negative and `eval_axis` is pure.
  - On J they cut a separated pair from 15 axis evaluations to 1 (a lateral pair splits on A's x or z face, a slab pair on y). Estimated: from 150–220 ns down to 35–60 ns per pair, on 53 % of pairs.
- **Rejected:**
  - Jolt's cached "no contact" and Box3D's non-touching recycle. Both are sound only with a speculative margin larger than the tolerance, and boyko keeps no points with separation above 0.
  - Adding a margin to make them sound: that raises the manifold count toward Jolt's 1.87× (+~4k manifolds at 1.24 µs is **+5 ms per step**). The per-manifold number would improve by inflating the denominator, while per-step time gets worse.
- **Trade-off:** the SAT keeps running for pairs that are about to touch (their cached axis flips to overlapping). That is the correct outcome.

### D2: A per-row orientation-frame column (L9a)
- **What:** `row_frames: ScratchColumn<RowFrame { axes: [Vec3; 3], radius: f32 }>` (40 B), filled once per step in the narrowphase prologue as `Mat3::from_quat(bodies[r].rotation)` columns plus `|half_extents|`. `Obb::from_frame` reads it.
- **Why:**
  - Each box pair today converts 2 quaternions, about 19k conversions per step. The fill costs 1,241 conversions (about 12 µs).
  - It is bit-identical, because it is the same function on the same snapshot.
  - The hit path (D5) also needs the frames.
- **Rejected:** writing the frames in the gather (where R₀ is formed for inertia). That touches `SolverScratch` and the gather, and bit identity with `Obb::new` would then rest on two call sites staying in step.

### D3: Only non-sensor box-box pairs with valid shapes get reuse
- **What:** L9b applies iff both shapes are `Box`, neither body is a sensor, and the pair is "slow" (D8). Everything else takes today's path: sphere-sphere, sphere-box, sensor pairs, SDF, degenerate boxes and fast pairs. L9a applies to every box-box pair, sensors included, because it is exact.
- **Why:**
  - A reuse check (~20 ns) plus a record copy costs about as much as `sphere_box_contact` or the sphere-sphere compare.
  - Sensor overlap begin and end must be exact every step (L10's Invariant V reaches the same conclusion).
  - The SDF stage has never been measured (L5 §SDF).
- **Trade-off:** mixed scenes get nothing from L9b on their spheres.

### D4: The reuse criterion is an arc bound in the frame of the larger body, scaled by extent
- **What:**
  - F is the body with the larger `radius`, S the other; on a bitwise tie, F = A.
  - `d = R_Fᵀ (c_S − c_F)` and `q = q_F* ⊗ q_S`.
  - Against the record: `Δd = d − d_ref` and `s = |vec(q_ref* ⊗ q)|` (= sin(Δθ/2), independent of the quaternion's sign).
  - τ_eff = min(τ, 0.05·min(r_A, r_B)), with default τ = **1 mm**.
  - **Hit ⇔ 2·(|Δd|² + 4 s² (r_S + τ_eff)²) ≤ τ_eff²**. Since (a+b)² ≤ 2(a²+b²), this implies |Δd| + 2s(r_S+τ_eff) ≤ τ_eff, and it needs no sqrt.
  - Both shapes must also equal the record's shapes bitwise.
- **Why (Lemma L9-L3 below):**
  - This bounds the relative displacement of every point that matters for the contact by τ_eff.
  - Measuring in the larger body's frame keeps slab pairs hitting. In A's frame a 141 m slab's far corners swing 14 mm for a 1e-4 rad box tilt.
  - The 5 % clamp bounds the error on small boxes relative to their size.
  - 1 mm is 1/10 of A7-R1's bound. On J the relative tilts are ~1e-5–2e-5 rad (the A7 C2 probe), which is ~0.03 mm at r = 1.73 m, so hits are near-certain once the pile settles.
- **Rejected:**
  - Jolt's 1 mm plus 2° not scaled by extent: 2° on a body with r = 1.73 m is a 60 mm arc. Jolt survives that because its position solver recomputes separation from local points on every iteration; boyko fixes separation per step.
  - Box3D and Rapier's frozen world normal: needs an absolute rotation limit (10° / 11.5°), and it keeps a stale normal on a pile rotating as one body.
  - Per-step anchoring: errors accumulate. All four engines that reuse contacts anchor to the last full collision, and so does this design.

### D5: Refresh by carrying the kept features with their bodies (face) and re-evaluating the cached edge (edge)
- **Face record:**
  - Stores the reference face (which body, local axis, sign) and each kept incident point in the incident body's frame.
  - Refresh: `n = ±ref.axes[i]` (the same bits as `face_contact`'s `ref_normal`) and `h = ref.half[i]`. For each point, `w = (c_inc − c_ref) + R_inc·lp` and `sep = w·n − h`, computed relative to the body centres (≈1e-7 m noise, better than today's world-frame ~2e-6). Then `p_inc = c_ref + w` and `p_ref = p_inc − n·sep`.
- **Edge record:**
  - Stores `(ea, eb)`. Refresh: `eval_axis` on the current cross product, then today's `edge_contact`.
  - If the axis is degenerate, the pair misses.
  - If `depth < 0`, the pair is separated exactly, and the tag becomes SEP.
- **Why:**
  - For original incident corners the refreshed separation *is* the exact distance to the current reference plane. That is what `face_contact` computes, minus the SAT and the clip.
  - Clip-created vertices slide by at most τ_eff, so their separation is off by at most (relative tilt) × τ_eff, about 1e-8 m on J.
  - An edge re-evaluation costs ~30 ns, so there is no reason to approximate it.
- **Rejected:** re-clipping on the cached face, Box3D-style. It costs 2–3× the hit, and it only fixes second-order errors.
- **Trade-off:** new features within τ_eff are not seen (Lemma L9-L3 bounds this), and lever arms are off by at most τ_eff.

### D6: Lifted points are dropped, not kept as speculative points
- **What:**
  - A refreshed face point with `sep > 0` is dropped, which is today's face filter. Edge points are not filtered, as today.
  - A hit whose filter leaves zero points emits no manifold, keeps its record, and is **not** turned into a miss.
- **Why:**
  - Boyko's solver applies `bias_rate·sep` to positive separations too (`colored.rs:2094-2098`). A kept lifted point would therefore resist approach early (a ghost contact).
  - Converting "all lifted" into a miss would make the output depend on the axis hint, which breaks Lemma L9-L1.

### D7: On a miss the emitted manifold is refresh(build(full compute)), never the raw full compute
- **Why:** it makes the output a pure function of (record, current poses) (Lemma L9-L1). Equal poses then give equal output bits. Three things need that:
  - `frozen_island_warm_start`'s bitwise wake-against-twin test;
  - L10's "replay = recompute";
  - W invariance.
- **Rejected:** emitting the raw full compute. The step after a miss at unchanged poses would then differ by ulps, and the frozen-twin exactness test goes red (mutation M-b5).
- **Price:** ~60–80 ns (build plus refresh) per slow miss.

### D8: A "fast" predicate keeps moving pairs on today's path
- **What:** a pair is fast ⇔ `3·dt²·(|v_S−v_F|² + |ω_F|²·|c_S−c_F|² + |ω_S−ω_F|²·(r_S+τ_eff)²) > (τ_eff/2)²`.
  - A fast pair runs today's path: no record and no refresh, only the full compute (with L9a).
- **Why:**
  - Without it, a pair that misses every step pays about +17 % (criterion plus build plus refresh on ~450 ns). With it the overhead is about +2 %. This matters for J's [0,100) fall and for any moving scene.
  - The predicate is a pure function of `BodyState`, so at bit-equal states it gives the same class (needed by L10).
- **Correctness does not depend on it**; only cost does.

### D9: Storage is a pair carry in stream order, joined by row translation. Records are indexed by pair slot and double-buffered
- **What:**
  - `ContactPairs::pairs_prev` is swapped in at the start of the broadphase, before the kind arm (the AllPairs arm stays verbatim).
  - `Manifolds::{pair_tag, pair_tag_prev}` hold `u16` tags; `Manifolds::{reuse, reuse_prev}` hold `ReuseRecord`s at the pair's slot k.
  - The source of pair k is `j(k) := lower_bound(pairs_prev, key_prev(k))`, a match iff equal, where `key_prev = RowRemap::pair(a, b)`.
  - On Identity steps `key_prev = (a, b)`. On Rows steps, pairs touching a stage-2 row (a "jumper", non-monotone) binary-search; all other pairs merge-join monotonically. On Reset, every pair misses.
  - A hit copies `reuse_prev[j] → reuse[k]` (128 B).
- **Why:**
  - The access is sequential and cache-streaming, and it can be written in parallel: each chunk owns its own tag and record slots, so no serial commit is needed.
  - The layout does not depend on W.
  - Records hold **no row data**, so a row move only changes the join.
  - It is the same carry L10 rev 2 D4 designs (`pairs_prev`, `pair_tag`, merge-join), so the two share it.
- **Rejected:**
  - An in-place hash table (Jolt's `ManifoldCache`, or the style of our axis cache): random access, the serial-commit rule from L5 D3, and a prefetch on Rows steps.
  - Records indexed by manifold ordinal: L5's per-chunk stage indices depend on W, so this needs an index column or a second compaction copy.
  - A record slab with stable slots (no copy on hit): needs a serial allocator and garbage collection, to save about 20–40 µs.
  - Binary-searching every pair on Rows steps: ~0.4 ms on every spawn step at J scale.
- **Trade-off:**
  - 2 × P × 128 B committed: 2.45 MB at J, on the same order as L5's 1.39 MiB `np_stage`.
  - Needs `RowIdentity::stage2_rows()` (bp lane T5). L9 adds it if T5 has not landed.
  - A pair that vanishes and returns loses its record (a cold miss).

### D10: The axis cache stays as the hint source for misses and is not written on hits
- A hit reads and writes nothing in the axis cache; L5 D3's `commit[k] = AXIS_NONE`.
- The entry stays the axis of the last full collision, which is exactly the record's.
- Using the tag's axis as a fallback hint was rejected: it changes values on flicker pairs, and U7 deletes both stores.

### D11: Configuration
- `PhysicsConfig::contact_reuse: bool`: `false` in C3, `true` in C4. It is a same-binary A/B switch and the census control.
- `contact_reuse_distance: f32 = 0.001` (τ), with a `debug_assert!` that it is finite and ≥ 0.
- L9a has no flag, since it is exact; its A/B is by commit.
- Runtime toggles need no epoch: with the flag off, REC tags are ignored and rewritten; with it on, the next miss builds a record. A change of τ takes effect at the next check.

### D12: Lane order: L5 → L9 → broadphase Tree → L10 rev 2.2
- **Why:** L9 is the only narrowphase lever at W=1. L10 is worth exactly 0 on parity (sleeping is off on both sides).
- The Tree emits exactly AllPairs' set in `(min, max)` order (`04-DESIGN-REV2.md:36`), so it is independent of L9.
- If L10 lands first, L9 widens its `u8` tag to `u16` and adds the record to its kept store (see Interactions).

## Data structures
```rust
// narrowphase/carry.rs
#[repr(transparent)] #[derive(Clone, Copy, Default)]
pub(crate) struct PairTag(u16);
// bits 0-3 axis: last chosen SAT axis (15 = none)  | 4 BOX | 5 PUSHED | 6 SET (L10) | 7 SETTLED (L10)
// bit 8 REC: reuse[k] valid, written this step | 9 SEP: bits 12-15 hold the separating axis
// bit 10 HIT: output came from a record | 11 SEPHIT: rejected by the cached separating axis
// bits 12-15 sep_axis
// Bits 0-7 are exactly L10 rev 2's u8 layout; L9 writes axis, BOX and PUSHED and reserves SET/SETTLED.

// narrowphase/reuse.rs
#[repr(C, align(16))] #[derive(Clone, Copy)]
pub(crate) struct ReuseRecord {      // 128 B = 2 lines; const-asserted
    d_ref: Vec3,                     // S's centre in F's frame at the last full collision
    count: u8,                       // kept features, 1..=4
    flags: u8,                       // REF_IS_B | REF_NEG | EDGE | PARITY (bit 7: step parity, debug check)
    feat: u8,                        // reference local axis 0..3, or (ea << 2 | eb) for EDGE
    _p0: u8,
    q_ref: Quat,                     // q_F* ⊗ q_S at the last full collision
    half_a: Vec3, half_b: Vec3,      // shape pin, compared bitwise
    lp: [Vec3; 4],                   // kept incident points, incident body's frame (FACE)
    feature: [u32; 4],               // warm-start feature ids, carried unchanged
    _p1: [u32; 2],
}
#[repr(C)] #[derive(Clone, Copy)]
pub(crate) struct RowFrame { axes: [Vec3; 3], radius: f32 }   // 40 B; per row per step

// resources.rs: ContactPairs (+1)
pub(crate) pairs_prev: ScratchColumn<(BodyIndex, BodyIndex)>, // swapped with `pairs` at broadphase start
// resources.rs: Manifolds (+6 columns, +1 cursor)
pub(crate) pair_tag: ScratchColumn<PairTag>,  pub(crate) pair_tag_prev: ScratchColumn<PairTag>,
pub(crate) reuse: ScratchColumn<ReuseRecord>, pub(crate) reuse_prev: ScratchColumn<ReuseRecord>,
pub(crate) row_frames: ScratchColumn<RowFrame>,
pub(crate) jumper_bits: ScratchColumn<u64>,   // written on Rows steps only
pub(crate) carry_cursor: RemapCursor,
```
- **Hot vs cold:**
  - Every step (hot): tags (2 B per pair), `pairs_prev`, `row_frames` (50 KB at J, fits in L2), and the records of REC pairs.
  - Rows steps only: `jumper_bits`.
- **Lengths:**
  - `reuse` and `pair_tag` only grow (`resize` fills the tail on growth only), like L5's `np_stage`.
  - A stale record slot is never read, by the invariant "REC ⇒ written in the same step" (debug-checked by the parity bit).
- **Scratch ids:** +7 (6 in the narrowphase cohort, 5 → 11 after L5; `pairs_prev` in the broadphase cohort).
  - The broadphase + narrowphase union grows from 19 to 26 wide, still ≤ `POOL_STAGGER_LINES` = 64.
  - The axis-carry assert (`scratch_ids.rs:680-688`) must still hold; if it fires, derive `SCRATCH_ID_AXIS_REMAP` with `highest_id_clear_of`.
  - If the region-floor assert (`:734-739`) fires, apply L10's floor move (MAX−160) with its census.
- No `Drop`, no new `unsafe impl`, and every element is POD.

## Public API
```rust
pub struct PhysicsConfig { /* … */ pub contact_reuse: bool, pub contact_reuse_distance: f32 }
// profiling (counters, computed after the loop from tags, armed only):
//   PHYS_NP_REUSED, PHYS_NP_SEP_HITS, PHYS_NP_FULL
pub fn box_box_contact(...) -> Option<BoxBoxContact>;  // unchanged signature: a wrapper over classify
```
Everything else is `pub(crate)`:
- `box_box_classify(a: &Obb, b: &Obb, body_a, body_b, hint) -> BoxBoxOutcome { Contact{c, feature: FeatureRef}, Separated(u8), NoContact }`;
- `sep_still_holds(a, b, axis) -> bool`;
- `Obb::from_frame`;
- `reuse::{criterion, build, refresh_face, refresh_edge, is_fast}`;
- `carry::{PairJoin, PairTag}`;
- `RowIdentity::stage2_rows()` (T5).

## Algorithms for critical paths
```text
physics_broadphase:  swap(pairs, pairs_prev)                      // before the kind match; O(1)
physics_narrowphase:
  prologue (serial): fill row_frames (O(N)); remap = carry_cursor.remap(rows)
     Rows ⇒ jumper_bits from stage2_rows (O(N/64 + |stage2|)) [cold]
     swap(pair_tag, pair_tag_prev); swap(reuse, reuse_prev); grow to P (cold)
     axis_cache.begin_frame_synced(...)                           // unchanged
  pairs: serial loop (W=1 or L5 inline) or L5 chunks; the join start per chunk = lower_bound
  epilogue: L5 compaction and axis commit (unchanged); carry_cursor.stamp(rows); counters (armed)

collide_box_pair(k):                           // pure in (bodies, frames, T0 hint, tag_prev[j], reuse_prev[j], cfg)
  A, B = Obb::from_frame(...); j = join(k); tp = tag_prev[j] or 0
  if tp.SEP and sep_still_holds(A, B, tp.sep_axis): return (None, NONE, SEP|SEPHIT|sep)   [L9a, exact]
  slow = cfg.contact_reuse && !sensor && !is_fast(...)
  if slow and tp.REC:
     r = reuse_prev[j]
     if shapes == (r.half_a, r.half_b) bitwise and criterion(r, ...):
        (m, sep_exact) = r.EDGE ? refresh_edge(A, B, r) : refresh_face(A, B, r)
        if sep_exact: return (None, NONE, SEP|sep)                // edge axis now negative: exact
        if EDGE degenerate: fall through to miss
        else: reuse[k] = r; return (m, NONE, REC|HIT|axis)        // m may be None (all lifted, D6)
  hint = hints.read(k)                                             // L5 Lemma 1 unchanged
  match box_box_classify(A, B, hint):
     Separated(s) → (None, NONE, SEP|s);  NoContact → (None, NONE, BOX)
     Contact{c, f} → commit = L5 D3 rule on c.axis
        slow: r = build(c, f, ...) [#[inline(never)]]; reuse[k] = r; (refresh(r), commit, REC|axis)
        else: (Some(c.manifold), commit, PUSHED|axis)             // today's path
```

**Per-pair cost (J, W=1, estimates; the C0 bench measures them):**

| class | share of pair-steps | today | after L9 | cache behaviour | branches |
|---|---|---|---|---|---|
| separated, cached axis holds | ~53 % | 150–220 ns | 35–60 ns | 2 frames + 2 body lines + tag; sequential | SEP branch; stable per pair |
| touching, hit | h × 47 % | 380–480 ns | 60–100 ns | + 128 B read and 128 B write, sequential; no hint probe | shape compare and criterion; ~always true |
| touching, slow miss | (1−h)·slow | 380–480 | +60–80 | as today + record | — |
| fast | moving pairs | as today | +5–10 | as today | — |
| join (Identity) | all | — | 2–4 ns | 8 B `pairs_prev` stream | monotone cursor |

- **Complexity:** O(P + N) per step; O(P log P) worst case on Rows steps, limited to pairs touching jumpers.
- **SIMD:** none new; the refresh of 4 points is a candidate later, not justified at ~30 ns.
- **I-cache:** the hit path (criterion plus refresh, ~1–2 KB) is the steady-state loop. `build` is `#[inline(never)]`, and the full SAT/clip becomes the cold-ish path.
- **Memory traffic at J, steady state:** ~2 MB streamed per step (records and manifolds), ≈20–40 µs.

## Determinism argument
- **L9-L2 (L9a is exact).** Today `sat` returns `None` iff some candidate has `depth < 0`. The early return and the cached-axis check both evaluate a subset of the same `eval_axis` calls with the same bits, and return `None` only when one of them is negative. So the output is unchanged. The overlapping path computes all 15 candidates exactly as today. `row_frames[r]` is `Mat3::from_quat(bodies[r].rotation)`, the call inside `Obb::new`. NaN depths compare false in both paths.
- **L9-J (join purity).** `j(k)` is defined as `lower_bound`. The merge implementation equals the definition because non-jumper keys are monotone: aligned-walk rows have strictly increasing `prev_row`, `row_identity.rs:574-581`. Jumper pairs use the definition directly (binary search). Hence `j` depends on neither the chunk partition nor the implementation.
- **L9-L1 (equal poses give equal output).** For a slow pair whose `BodyState`s are bitwise equal at t−1 and t:
  - The pair was SEP: the axis was negative at P, so it is negative again.
  - The pair was REC: at t−1 it was either a hit (record unchanged, `criterion(R, P)` true) or a miss (`R = build(full(P))`, so `d = d_ref` bitwise and `s ≈ 0`, hence a hit at t).
  - Either way the output at t is `refresh(R, P)`, the same as at t−1.
  - Fast pairs are on today's path, so L10's own SETTLED argument covers them.
  - This gives `frozen_island_warm_start`'s twin exactness: the sleeper's state at the wake step equals the twin's first step after the freeze (same record, same poses). It also gives L10's replay = recompute (given the record carry, see Interactions).
- **L9-L3 (criterion soundness, error bound).**
  - In F's frame, S's motion y' − y = Δd + (R_Δ − I)(y − d) moves every point of S, and every point of F within r_S + τ_eff of S's centre, by at most |Δd| + 2 sin(Δθ/2)(r_S+τ_eff) ≤ τ_eff.
  - Hence: an unseen feature penetrates at most τ_eff; the deepest point is within 2τ_eff of the full compute's, since the reduction always keeps the deepest point; a corner's separation after refresh is exact; a clip vertex's is off by at most tilt × τ_eff; lever arms are off by at most τ_eff.
- **W invariance.** L5's theorem extends:
  - the per-pair inputs are read-only during the scope (`tag_prev`, `reuse_prev`, `pairs_prev`, `row_frames`, T₀);
  - tag and record writes are per slot, and the cuts partition the slots;
  - streams go through L5 D1 and the axis commit through D3, which stays serial and in order, with hits writing `AXIS_NONE`;
  - the serial and the parallel path use the same predicate.
- **Machine and replay (KC-37).**
  - Only IEEE f32 add, mul, sqrt and compare; no `mul_add`, no FMA contraction, no order-changing reductions; same binary. The state starts empty on startup and evolves as a pure function of the run.
  - Keyed by row through `RowIdentity`, so it shares H-03 (the recycled-id window of one gather); A1b/U7 fix it for all consumers at once.
  - No new pacing or Main-read hazard: physics is in Fixed.

## Multithreading model
- Shared and read-only during L5's scope: bodies, pairs, `pairs_prev`, `tag_prev`, `reuse_prev`, `row_frames`, `jumper_bits`, the axis slots, `remapped`, and `RowRemap` (`Copy`, holding `&[u32]`).
- Exclusive per chunk: `stage`, `commit`, `tag`, `reuse` slots in `[lo, hi)`, and `meta[c]`.
- Synchronisation: L5's spawn and join only. No new atomics; loom is N/A.
- `unsafe`: +2 sites in `np_chunk` (tag and record writes through `ScratchSolveView::row_ptr`), each with a `// SAFETY:` citing the cut partition, `P ≤ len`, the absence of a concurrent reader, and write provenance from `solve_base`. No slice is taken over `tag` or `reuse` until after the join (Tree Borrows).
- False sharing: only at chunk boundaries (a 128 B record never shares a line with a neighbour).

## Expected gain (arith. from P0b; per-class costs are estimates until C0)

**Inputs, J, W=1:** t_np = 3.142 ms; N_sep = 5,037; N_touch = 4,524; h = 0.80–0.97 over [100,500) (the pile settles before ~188; Jolt hits ≈100 % from frame 200).
- The per-class costs are constrained by N_sep·t_sep + N_touch·t_touch + P·t_loop = 3.142.
- **Δt_np(1) = −1.4 to −2.5 ms**: L9a −0.45 to −0.93 ms (exact); L9b −0.95 to −1.6 ms; carry −0.03 to −0.06 ms.

| row | per step | per manifold (own count) | vs Jolt v5.3.0 / v5.6.0 per manifold |
|---|---|---|---|
| J cfg-A W=1: 19.67 → | 17.2–18.3 ms (−7 to −13 %) | 4.35 → 3.8–4.05 µs | — |
| J default + simd W=1 (derived 11.09): → | 8.6–9.7 ms (−13 to −22 %) | 2.45 → 1.90–2.15 µs | 1.02–1.16× / 1.67–1.88× |
| … plus Tree bp (−1.86 to −1.94) | 6.7–7.8 ms | 1.48–1.73 µs | **0.80–0.93×** / 1.30–1.52× |
| J W=8 after L5 (5.5–5.7): → | −0.26 to −0.55 ms (5–10 %) | — | — |
| … plus Tree bp | 3.3–3.8 ms | 0.73–0.84 µs | 1.75–2.0× / 2.5–2.8× |
| J W=8 if L5 were absent (np serial 2.946) | −1.3 to −2.3 ms | — | — |
| R W=1 (np 3.63 ms, 6,662 manifolds) | measured, not predicted | — | — |
| J-Son tail (all asleep, before L10) | ≈ −2 ms of 5.33 (frozen pairs hit) | — | — |

**Per stage, per manifold, W=1:**
- np: 0.695 → 0.14–0.39 µs.
- Collision plus setup (bp + np + solve_build): 1.38 → **0.40–0.67 µs** with the Tree, against Jolt's FindCollisions (which includes its broadphase and setup) at 0.355–0.544 µs per manifold (its own count).
- Per pair, np: 0.329 → 0.07–0.18 µs, against the parity budget of 41–139 ns.
- After L9, solve_build (0.215 µs per manifold) is the largest per-manifold cost on the collision side.

## Interactions
- **L5:**
  - `collide_pair` gains the carry context. Per-slot outputs need no new serial phase. The join start is one binary search per chunk.
  - L5's chunk floor (`NP_MIN_PAIRS_PER_CHUNK = 128`, from 0.329 µs per pair) becomes ~13 µs chunks. C5 re-derives it from the measured per-pair cost after L9 (≈400 pairs for 40 µs). This is bit-identical under L5's theorem.
- **Tree broadphase:** exact set, same order, so none. The persistent-pair-set remark (`04-DESIGN-REV2.md:188`) is untouched: L9 translates every step and keeps no persistent pair structure.
- **L10 rev 2.2 (required amendments, fail-closed):**
  1. `pair_tag` is L9's `u16`; `pairs_prev`, the tags and the merge-join are shared (L10 saves 3 ids).
  2. `KeptManifold` also keeps the pair's `ReuseRecord` and tag; restore re-enters them into the stream carry. Without this, Sets ≠ Off under L9 and L10's own S-gates go red.
  3. Replay copies record and tag with the manifold (by L9-L1, recompute = copy).
  4. The M3/SETTLED semantics treat a REC pair that hit at t−1 as settled.
- **Axis cache:** written only on misses; L5 Lemmas 1–2 hold (fewer `set`s).
- **B1 warm carry and warm tables:** feature ids are carried unchanged, so warm keys hit as today.
- **A4 (wake on count change):**
  - A hit keeps its manifold, unless every point lifted, which is today's semantics. A support moved by less than τ_eff does not change the count; today it usually does not either.
  - `support_loss_wakes_sleepers` uses sphere scenes, which L9b does not touch.
  - Ruling W2 added its box-pile arm after C3 (2026-09-23): `e_lowering_a_box_support_by_less_than_tau_eff_wakes_the_box_it_carried`, contact reuse forced on at τ = 2 mm, the support lowered by τ_eff/2. A refresh that keeps lifted points turns it red.
- **U7 and the row-keyed stores:** L9 and L10 share **one** carry, so there are 5 stores total, not 6. U5/U7 turn it into a persistent per-pair store without translation.

## Integration (file:line)
- **`narrowphase/box_box.rs`:**
  - `:142-180`: `Obb::from_frame`.
  - `:274-315`: `sat` returns at the first negative axis (`Result<SatResult, u8>`), plus `sep_still_holds`.
  - `:642-734`: `box_box_classify` returns `FeatureRef` (from `face_contact` `:833-990`: reference side, axis, sign; from `edge_contact`/`edge_fallback` `:778-807`, `:994-1028`: `(ea, eb)`).
  - `box_box_contact` becomes a wrapper.
  - The pre-change body is kept as a `#[cfg(test)]` oracle.
- **New `narrowphase/reuse.rs` and `narrowphase/carry.rs`**, with their unit tests and proptests; `narrowphase/mod.rs` gains 2 lines.
- **`systems.rs`:**
  - `:297-349`: the swap before `match cfg.broadphase` (the AllPairs arm stays verbatim).
  - `:375-478`: prologue, epilogue, collide dispatch, counters.
- **L5's `narrowphase/dispatch.rs`:** `NpChunkCtx` gains the carry views; `np_chunk` gets the join start and the 2 writes.
- **`resources.rs`:**
  - `:126-330` / `:442-530`: the 2 `PhysicsConfig` fields and their defaults.
  - `:562-611`: `pairs_prev`.
  - `:2269-2327`: the `Manifolds` fields and constructor.
- **`scratch_ids.rs:887-952`**: counts and layouts; asserts `:680-688`, `:919-932`.
- **`profiling.rs:92-131`**: +3 counters (`COUNTER_ZONES` 7 → 10) and the doc tables.
- **`row_identity.rs`:** `stage2_rows()` if T5 is absent; module docs `:3-17`, `:45-51` list the carry.
- **Runner `benches/jolt_parity_pyramid.rs`:**
  - flags `--contact-reuse on|off` and `--reuse-distance` (flags doc `:121-143`, `configure` `:723-749`, SUMMARY `:1160-1170`);
  - W4 closure per step: `full + reused + sep_hits + non_box == pairs`, else the step is void;
  - on reuse-on rows, `reused > 0` over [100,500), else void.
- **New:** `benches/narrowphase_classes.rs`, `tests/contact_reuse_bounds.rs`, `tests/narrowphase_carry.rs`.

## Commit sequence (each green under `--workspace --all-targets --no-fail-fast`, clippy `-D warnings`, the Miri leg)
0. **C0 instruments.**
   - The class bench: snapshots of J and R at step 300 plus a moving "rain" scene. It times `collide_pair` per class (separated, touching, fast) and reports ns per pair. Its structural asserts: class counts ≈ 5,037 / 4,524, and a void is red.
   - Record pose-hash fixtures on the base (J-A, J-D, R, R-S, J-Son; 600 steps; W ∈ {1, 8}).
   - **Refutation:** if the measured costs predict Δt_np(1) < 1.0 ms, stop and escalate.
1. **C1: L9a (i) and (iii).** Early return in the SAT, `row_frames`. Bit-identical.
2. **C2: the carry plus L9a (ii).** `pairs_prev` swap, tags, join, jumper bits, T5 if absent, cached separating axis. Bit-identical.
3. **C3: L9b dormant.** Record, criterion, refresh, build, fast predicate, counters, runner flags, L5 integration. `contact_reuse = false`. Includes the h(τ) readout (G-L9b-6).
4. **C4: L9b on by default.** The only commit that moves values; re-pins under each file's own rule.
5. **C5: L5 chunk floor** from the measured per-pair cost after L9. Bit-identical.
6. **Timing window**, after the owner confirms a quiet machine.

## Gates (each can fail)

| gate | commit | what | fails via |
|---|---|---|---|
| G-L9a-1 | C1 | proptest `box_box_classify` against the oracle: random poses including exactly touching integer lattices and degenerate extents; manifold bytes + axis, or `None` | M-a1: early return on `depth <= 0` (red on exact touching); M-a2: `row_frames` stores rows, not columns |
| G-L9a-2 | C1, C2 | runner `--expect-pose` equals the C0 fixtures, all rows, W ∈ {1, 8} | M-a3: `row_frames` refilled only on Rows steps |
| G-L9a-3 | C2 | proptest with random **stale** separation hints (0..15) against the oracle | M-a4: accept `depth <= 0`; M-a5: raw (unnormalised) cached axis |
| G-C-1 | C2 | scripted-gather join tests: Identity, shift, swap-remove jumper, append, order flip, Reset; `j(k)` = brute-force entity-pair lookup | M-c1: untranslated key on Rows; M-c2: jumper pairs merged; M-c3: cursor stamped before the loop |
| G-C-2 | C2 | L5 G-L5-1/G-L5-3 extended with random partitions: streams, tags, table bytes | M-c4: chunk join starts at `j = lo` |
| G-C-3 | C2 | allocation census: 0 heap per step, S1b/S1c scope pins unchanged | `Vec::with_capacity` in the jumper build |
| G-L9b-1 | C3 | criterion soundness proptest: corner separation after refresh equals the exact plane distance within 2e-6 m; deepest(full) ≤ deepest(refresh) + 2τ_eff + 1e-5 | M-b1: drop the rotation term; M-b3: normal carried by the incident body; M-b4: no τ_eff clamp (small boxes) |
| G-L9b-2 | C3 | L9-L1: frozen poses for 3 steps after a forced miss (teleport and back; shape change and back), outputs bitwise equal | M-b5: emit the raw full compute on a miss |
| G-L9b-3 | C3 | W ∈ {1,2,4,8,16} × parallel np × reuse forced on: streams, tags, REC records and poses equal the W=1 serial run | M-c4; M-b6: chunk reads `reuse`, not `reuse_prev` |
| G-L9b-4 | C3 | units: slab pair hits (F = larger body); shape change misses; sensor never REC; fast pair never REC; resting 2-box stack reaches 100 % hits after 1 step | M-b2: F = A always (red: slab pair misses) |
| G-L9b-5 | C3 | Miri (Tree Borrows): chunked tag and record writes | base from `as_read_slice().as_ptr().cast_mut()` |
| G-L9b-6 | C3 | h(τ) readout, counters only: J and R, τ ∈ {0.25, 0.5, 1, 2} mm | **refutation:** h_J(1 mm) < 0.7 on [100,500) stops C4 and escalates |
| G-L9b-7 | C4 | bounds (below) plus the new `contact_reuse_bounds.rs`: a slowly tipping box's new corner penetrates ≤ 2τ_eff; a slowly sliding box overhangs ≤ τ_eff + 1e-5 | M-b1 turns the tipping test red |
| G-TW | window | the P0 protocol, detailed below | a claimed regression at any W blocks |

**G-TW detail (P0 protocol: median over K processes, receipts, canary, SE claim rule):**
- L9a: C0 against C2, on J-A, J-D and R, W ∈ {1, 2, 4, 8, 16}, K = 6 (the SE bar is 2.0 % at W=1 against a predicted 4–8 %).
- L9b: same binary C4, `--contact-reuse off` against `on`, on J-A, J-D and R, at every W.
- Realized-gain rule: ΔT(1) on J-A ≥ 0.6 × the prediction from the bench and the measured h, claimed; otherwise refuted and investigated before merge.
- The J [0,100) sub-window is the moving-scene witness: "not claimed slower".
- The canary must be seen.
- Per contact: a table of µs per step, per manifold (each side's own count) and per pair, for bp, np, setup and colours, at W=1 and W=8, beside Jolt's bracket.
- Pose hashes: every C4 `--contact-reuse off` row equals C0's hash; `on` rows have one hash per row across W.

**Correctness bounds at C4 (never loosened):**
- **A7-R1:** `CREEP_BOUND_M = 0.01` (`sleep_settles_box_piles.rs:313`, `:1767-1770`). Bands registered in advance:
  - < 2 mm confirms;
  - 2–10 mm is acceptable and reported beside the reuse-off run;
  - ≥ 10 mm is STOP: no flip, halve τ once and re-read, escalate if it persists.
  - The reuse-off run on C4 must read 0.7180 mm exactly.
- **A7-R2:** the pile must still freeze.
- **G2 / G7 / G8:** budgets unchanged, with the file's triage protocol.
- **`frozen_island_warm_start`:** all six tests.
- **`support_loss_wakes_sleepers`:** all tests.
- **`sleeping_pipeline`:** the gravity-off frozen arm's structural asserts. Identity rotations make the refresh arithmetic exact there (a prediction; the asserts check it).

**Debug assertions:**
- A join match means `pairs_prev[j] == key_prev(k)`, and non-jumper keys are monotone within a chunk.
- A REC tag means the record's parity bit matches the previous step's parity.
- A record has `1 ≤ count ≤ 4` and a valid `feat`.
- A refreshed normal has |n|² ∈ 1 ± 1e-5.
- A hit means the shapes are bitwise equal.
- A SEP tag means `sep_axis < 15`.
- The counter closure holds; L5's list is unchanged.

## What moves, and how it is re-measured
- **C1, C2, C3, C5:** nothing. A moved pose, golden or hash is a defect.
- **C4:**
  - Every default-world trajectory with box piles: the runner's pose fixtures are re-recorded for reuse on.
  - The A7-R1 D_max reading and the "step 600: 6671 manifolds / 22 975 points" docs (`sleep_settles_box_piles.rs:64-72`, `:96-127`).
  - A7-R2's freeze step (`benches/sleeping_pipeline.rs:23-25` docs, 248 → read again).
  - G2/G7 observed counts (budgets untouched) and G8's freeze step.
  - P0's H8 manifold count (4,519 → read again; the per-manifold denominators move with it).
  - Any golden fed by box-pile physics.
  - The two exact pins added after C3 (2026-09-23): `default_world_pyramid_determinism.rs`'s `PINNED_FINAL_HASH` (release and debug) and A7-R1's `A7_R1_D_MAX_BITS` in `sleep_settles_box_piles.rs`. Each is re-pinned under its own doc's rule. A7-R1's reuse-off run keeps reading `0x3a3c_3896` (0.0007180063 m) exactly.
  - The red set is enumerated by running C4. Each pin is re-measured under its own file's rule. A test whose subject is the exact narrowphase sets `contact_reuse: false` explicitly, with a comment saying why.

## What it cannot claim
- v5.6.0 per-contact parity at W=1: ~1.4–2.1 ms is still missing after L9 plus the Tree, in solve_build, warm apply, store and L8.
- Any per-contact parity at W=8, where solve scaling dominates.
- The per-class costs before C0, and h before G-L9b-6.
- That L9b is value-neutral, or that it reproduces the no-reuse engine. Only L9-L3's bounds are claimed.
- A reduction of A7-R1's creep (a prediction at most).
- Reuse for spheres, SDF or capsules.
- Bitwise continuity across save and load: the carry is not serialized, and a load gives one Reset step, as the warm and axis tables do today.
- Independence from id assignment beyond `RowIdentity`'s (H-03 until A1b/U7).
- L10's exactness on top of L9 before the rev 2.2 amendments.
- Any Jolt ratio other than the ones G-TW measures.

## Open questions
1. **Build-if rule (a values call).**
   - Plan §3's L9-specific rule (t_np(8) ≥ 20 % after L5) fails.
   - The campaign rule (≥ 5 % of T(W), claimed) clears at W=1 (13–22 %) but only marginally at W=8 after L5 (5–10 %).
   - The owner's per-contact and W=1 goal suggests stating the gate at W=1 plus per contact. The owner or orchestrator should rule on it.
2. **τ = 1 mm** is fixed here. G-L9b-6 can refute it. Moving it after that is the orchestrator's call, with h(τ) numbers.
3. **Lane order (D12)** and folding the L10 rev 2.2 amendments into L10's closed rev 2.
4. **The steady-state no-copy variant** (records used in place when `pairs == pairs_prev`): about 20–40 µs at J. Build it only if C0's bench, re-run after C4, attributes ≥ 5 % of t_np to the carry.