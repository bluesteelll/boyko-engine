# Tree broadphase F3: the kd median-split leaf order, and the window-7 tree thresholds

Lane TREE-F3, 2026-09-26, branch `u/phys-tree-f3`. This is the design at implementable depth that
F3 did not have: the C3b design (`c3b/design.md:168-172`, out of tree) gave it five lines, and the
only executable definition was the model's `kd_order` (`c3b/sim.py:130-150`), which the model
applies to the active tree only. The lane's cut wrote this text and the lane's critique reviewed
it; its three important remarks are resolved here (§7).

Binding inputs:
- `levers/00-RULINGS.md`: "C3b SHIPS and F3 is TAKEN UP" (2026-09-24; c_q = 170–190 ns per queried
  row, above the 150 ns bar), the `--bp-kernel` ruling (2026-09-24), and the thresholds ruling
  after window 7 wave 2 (2026-09-25).
- `docs/measurements/2026-09-25-physics-window7/wave2/` (the G4 threshold block, queue §15.5).
- `04-DESIGN-REV2.md` (the tree broadphase), C3b's leaf-list query (`broadphase_tree/mod.rs`,
  "Query kernels").

"arith." marks a number computed from code or counts, not measured.

## 1. What F3 is

- **The order.** A second leaf order for the **active** tree: the model's top-down kd median
  split. The static and sleeper trees stay Morton under every kernel (the model's scope,
  `sim.py:272-273`; a kd static or sleeper tree would move the admission and compaction spans with
  no model behind it).
- **The selector.** `QueryKernel::LeafListKd`: the leaf-list query (C3b F1) over an active tree
  built in the kd order. The query pass is `leaf_list_pass`, unchanged; the kernel is the only
  one whose choice reaches the build (`QueryKernel::leaf_order`).
- **Why a kernel variant.** The 2026-09-24 ruling makes `--bp-kernel` the one switch for F3's
  same-binary A/B. The runner's names are fixed now: `rowwalk | leaflist | leaflist-kd` →
  `QueryKernel::{RowWalk, LeafList, LeafListKd}`. The runner parses by a string match with an
  error arm, never by an exhaustive match on `QueryKernel`.
- **Opt-in.** `QueryKernel::default()` stays `LeafList`. The flip is a one-line follow-up after
  the window's R1–R3 (§6).

## 2. The order (normative: equal to `kd_order` entry by entry)

Input: `items[0..n)`, the Q rows `build_active` pushes in row order, so the row rises with the
item index — the model's index tie rule is a row tie rule. `kd_sort` permutes `items` **in
place**; the build then packs slot `s` from `items[s]`.

Each item carries its kd key in `Item::key` (the first word of what was the item's padding; the
item stays 32 B). The sort key of an item is `(key << 32) | row`, unique because rows are.

1. `n ≤ 8`: nothing moves. The rows already ascend, which is the model's `list(ids)` at a root
   leaf.
2. A fixed stack of pieces `(start, len)`, initially `(0, n)`; `KD_STACK = 64`. A split pushes
   two pieces and pops one, so at most one sibling per depth is pending; the depth is at most
   `ceil(log2(2^21 leaf nodes)) + 1 = 22` for `n < 2^24`. The bound is `debug_assert`ed; past it
   the array index panics (no UB).
3. Pop a piece `(s, m)`:
   - **`m ≤ 8`** (a leaf): insertion-sort `items[s..s+m)` by the sort key. The keys still hold
     the PARENT's axis keys, so this is the parent's lexsort order restricted to the leaf: the
     model's `list(ids)`.
   - **`m > 8`**:
     1. **Axis.** The f32 min and max of `x`, `y`, `z` over the piece (finite: Normal rows only);
        `ext_k = f64::from(hi_k) − f64::from(lo_k)`, in **f64**, as numpy's
        `pts.max(0) − pts.min(0)` on `float64`. `axis = 0`; `if ext_1 > ext[axis] { axis = 1 }`;
        `if ext_2 > ext[axis] { axis = 2 }`: the first maximum wins, as `np.argmax`. (A zero's
        sign in `lo`/`hi` changes no extent's magnitude, so it cannot change the axis.)
     2. **Keys.** One sequential pass: `item.key = axis_key(coordinate on axis)`, with
        `axis_key(v) = { let b = (v + 0.0).to_bits(); if b >> 31 == 1 { !b } else { b | 0x8000_0000 } }`.
        Order-preserving; `+ 0.0` maps `−0.0` to `+0.0`, so the two compare equal and tie by row,
        exactly as numpy's `==` does. (LLVM does not fold `v + 0.0` away: it is not an identity
        for `−0.0`.)
     3. **Split.** `leaves = m.div_ceil(8)`; `left = leaves.div_ceil(2) · 8`, so `8 ≤ left < m`.
     4. **Select.** `items[s..s+m).select_nth_unstable_by_key(left, sort key)`. The keys are
        unique, so `items[s..s+left)` is exactly the set of the `left` smallest — the model's
        `o[:left]` — whatever order `select` leaves inside either part.
     5. **Recurse.** Push `(s + left, m − left)`, then `(s, left)`.

**Consequences.**
- **Leaves are full.** Every piece starts at a multiple of 8, and a piece whose length is a
  multiple of 8 splits into two multiples of 8; only the rightmost chain carries the remainder.
  So each piece of `m ≤ 8` is one leaf node `s / 8`, full unless it is the last. `kd_sort`
  `debug_assert`s it per leaf piece; `LEAF_MAXROW` is computed as today.
- **Independent of the toolchain's `select`.** Unique keys fix each partition set; the leaf sort
  fixes each within-leaf order; the stack's processing order touches disjoint ranges.
- **No new column, no heap, no `unsafe`, no atomic.** `select_nth_unstable_by_key` lives in
  `core`. `kd_sort` is `#[inline(never)]`: a symbol of its own, so `build`'s Morton code keeps its
  shape (the G-F3-6 receipt).

### 2.1 Why in place: the build's working set and access pattern (critique W3)

The cut's first form sorted a key column `K = (axis key << 32) | item index` and read
`items[K[j] & 0xffff_ffff]` in both the extent pass and the key rewrite: after the first
`select`, `K` is in partition order, so both passes gather through an index at every level.

| n (Q rows) | items (32 B each) | element-levels (arith., `f3_arith.txt`) | index form: gathered 32-B loads per build | in-place form |
|---|---|---|---|---|
| 1 240 (J) | 39.7 KB | 9 112 (7.35 per row, depth 8) | 18 224 | 0: every pass reads a contiguous piece |
| 10 000 | 320 KB | 103 616 (10.36 per row) | 207 232 | 0 |
| 100 000 | 3.2 MB | 1 368 928 (13.69 per row, depth 14) | 2 737 856 across 3.2 MB (random at the upper levels, where a piece's indices span the whole row range) | 0 |

In place, a piece is contiguous: the extent pass and the key pass stream it once each, and
`select`'s partition scans it with two pointers. From the level at which a piece fits L2
(≈ 12.5k items = 400 KB, level 3 at 100k) the whole subtree below it stays there. The price is
that `select` swaps 32-B items rather than 8-B keys, and its comparator loads the 8-B sort key
(`row` and `key` are adjacent words) from each item. `TREE_ITEMS` is this tree's own column and
every user refills it before use (`build_active` every step; `admit`, `admit_sleepers`,
`compact` before their builds), so permuting it breaks no reader: after the build the step reads
the trees and the records, never `items`.

### 2.2 Cost (arith., the window decides)

- **Default path (`LeafList`):** `build_active` reads the kernel byte once per step, and the
  build branches once on the order: the one predictable branch per step C3b accepted for the
  kernel switch (`c3b/design.md:141`). G-F3-6 is the receipt.
- **kd path:** 7.35 / 10.36 / 13.69 element-levels per row at 1 240 / 10k / 100k. At 25–42
  instructions per element-level (extents, key, select), +184k to +339k instructions at J, about
  +14 to +39 µs at IPC 2–3 and 4.3 GHz — against a query saving of 29–57 µs (C3b's model: the
  kd figures of §5 against Morton's, attributions A and B). **The net at J is −43 to +10 µs**, so
  arithmetic cannot settle it. The design band for the J build (15–25 µs, `04-DESIGN-REV2.md:317`)
  leaves 3.1 µs over today's 21.9 µs and is replaced by the net rule R3 (§6; orchestrator ruling
  Q1 of the lane cut).

## 3. Bit identity: why F3 moves no pose byte (from the code)

1. **The pair set per row is order-free.** The leaf-list argument (`L`'s box contains its rows'
   query boxes; the prefilter is the walk's cull on the same bits; the max-row cut) holds for any
   tree `build` packs from any item order: leaf boxes are unions of the same padded lane boxes and
   every ancestor is a union of those. So every Q row's partner set P(r) is the same under Morton
   and kd.
2. **Segments are sorted** (`sort_leaf_list_segment`), so a segment's bytes are a function of
   P(r). Only its offset `seg` depends on slot order; `nrev` and `nfwd` depend on P(r) alone.
3. **The assembly is canonical** (`BroadphaseTree::assemble`): the rev entries are counted and
   scattered iterating the records in **row** order, so bucket `t` holds its larger rows
   ascending; the forward runs are read through each row's record; `SS` is strictly sorted;
   `merge_row` merges the three ascending lists per row, rows ascending. So `ContactPairs`' stream
   is P's pairs in lexicographic `(min, max)` order, independent of `seg`. `withheld` (SL) is not
   built from the active tree.
4. **The order DOES reach the pose**, which is why point 3 is load-bearing: `narrowphase_step`
   walks the stream in order and manifolds follow it; `ConstraintGraph` colours first-fit in
   manifold order; the box-axis hysteresis table is keyed over the stream. A debug build already
   guards the canonical order on every step (`physics_broadphase`'s strict `(min, max)`
   `debug_assert`). F3 keeps the order with the sort that already exists and **adds no sort: its
   cost here is 0**.
5. **Worker counts.** The Tree is serial and the kd order is a pure function of the rows, so
   W-invariance is unchanged.

## 4. The thresholds (window 7 wave 2, Q3)

- **The rule** (`treebp/g4_g5_recipe.md:131-132`, applied by wave 2 §3.2, ruled in
  `levers/00-RULINGS.md` on 2026-09-25): `TREE_BRUTE_MAX_ROWS = min(LO_uniform, LO_disparity) =
  144`; `AUTO_TREE_LO / AUTO_TREE_HI = (min LO, max HI) = 144 / 152`, both families; `LO ≥
  TREE_BRUTE_MAX_ROWS`. Measured on the shipped `LeafList` (Morton) kernel; re-derived by the same
  rule if the default kernel changes.
- **`AUTO_TREE_LO/HI`** land as const-asserted, documented `pub const`s in `broadphase_policy.rs`
  (placed after `select_broadphase`, so no line an anchor cites moves). `select_broadphase` is
  unchanged: wiring Auto's high side to Tree is tree C4's (`04-DESIGN-REV2.md` "Auto retargeted").
- **Value neutrality.** The brute path writes `all_pairs_into` — `for i { for j > i }`, pushing
  `(i, j)`, lexicographic. The tree path writes the canonical assembly (§3.3), also
  lexicographic, over the exact set. With sleeping off (the default and every trunk pin)
  `withheld` is empty on both paths, so `ContactPairs` is byte-identical. With the sleep-skip, the
  per-step pose equality holds by a chain of existing gates: Tree+Sets = Tree+Off
  (`sleep_skip_bit_identity`, Tree arms), Tree+Off = AllPairs+Off (identical `ContactPairs`),
  AllPairs+Off = AllPairs+Sets (its AllPairs arm).
- **Where 64 → 144 reaches.** The runner's `jolt` / `rest` (1 241 rows) are on the tree path at
  both values, `s16` (16) on the brute path at both; the rigs that force `brute_max_rows = 0`
  (the profiling harness, `broadphase_tree_scenes`, `bp_g4_scenes`, the G4 bench,
  `bp_query_counts`) do not read it. Moved: the debug `sleep_skip_bit_identity` piles (92 rows:
  every step becomes brute, so its Tree cells check the brute regime — T6: nothing withheld, no
  sleeper — and a debug-only brute-0 S2 arm keeps the tree regime covered beside S1's); and
  `alloc_frame_census`' S8b-tree (130–169 rows, then 89), whose frames 1–74 and ≥ 200 now run the
  brute path. The census file belongs to the w8s-s4 lane, which gives S8b-tree's Tree arm the
  `set_brute_max_rows(0)` S8-tree already has (orchestrator ruling Q6c).

## 5. What the model predicts (C3b's `sim.py`, the extraction reproduces every Morton pin first)

| scene | Morton (pinned) | kd (model) |
|---|---|---|
| J, contact reuse off | `[1953, 5663, 155, 6216, 15575, 9564]` | `[1809, 3417, 155, 3968, 10603, 9564]` |
| uniform / 1 000 | `[1228, 3465, 0, 3848, 7200, 2400]` | `[1095, 2247, 0, 2688, 5394, 2400]` |
| disparity / 1 000 | `[1487, 4096, 0, 4560, 11252, 6706]` | `[1354, 3098, 0, 3568, 8861, 6706]` |

Figure order: `[collection box tests, active candidates, static candidates, prefilter chunks, kept
exact tests, emitted]`. Leaves over the cap (candidate lists over 64 per leaf) at J: 13 → 0.
c_q (from 167.6 ns, t_q 0.2078 ms at J, W=1, the default row armed, wave 2): 121.6–135.5 ns
(attribution A, proportional) or 131.9–144.2 ns (B, a fixed residual) — both at or below 150 ns.

## 6. The claim (quiet window, same binary, after this lane and w8s-s4 merge)

Every process's pose is checked by its summary's `pose_hash`, against the 500-step hash of its
row, at every W (critique W1):
- J (`--scene jolt --gap 0.5`, `--cfg default` or `--cfg a`, `--broadphase tree`, 500 steps):
  `0x30c5438bc6ad9ffa` (the J500 family, `docs/measurements/2026-09-23-l9-contact-reuse/README.md`,
  12 of 12 at W 1 and 8);
- R (`--scene rest --cfg default --broadphase tree`, 500 steps): `0x6cbe24bf8fafda26` (rest-500,
  the same table);
- and, per (row, W), the `leaflist-kd` process's `--pose-out` file byte-equal to the `leaflist`
  process's: the kernel identity the A/B needs.

| rule | condition | on its own |
|---|---|---|
| **R1** c_q ≤ 150 ns | armed default row, W=1, `leaflist-kd`: t_q ≤ 0.186 ms, claimed under r AND s | — |
| **R2** the kernel works | t_q(kd) / t_q(leaflist) < 1, claimed, same binary, same block, W=1 | ¬R2 → F3 is rejected; the code stays under the switch or is removed (orchestrator) |
| **R3** the net (the ship rule) | t_qb = t_q + t_b: kd < leaflist claimed at W=1 and not claimed slower at W=8; the unarmed J and R step times not claimed slower at any W ∈ {1, 2, 4, 8, 16}; `bp_g4_scene/tree_kd` not claimed slower than `tree` at 10 000 and 100 000 | R2 without R3 → F3 stays opt-in (the build eats the gain). Record it with the build's per-size figures, and name the untried build forms as the follow-up's candidates — the presorted-axis-list build (it needs a scratch column: outside this lane) among them. Never "the only remaining form": in-lane variants of the in-place partition (e.g. a radix split on the cached `Item::key`) exist too. |
| **R4** value neutrality | every process's `pose_hash` equals its row's hash; the kd pose file equals the leaflist one | a mismatch voids the block and is a defect |

- **R1 ∧ R2 ∧ R3 → F3 SHIPS** as a follow-up commit: `QueryKernel::default()` → `LeafListKd`;
  `G_LL3_PINS` re-read by its rule (the default kernel changed by design); the thresholds
  re-derived by the recipe rule from the block's `tree_kd` arms.
- **Thresholds (claimed on the merged trunk).** On `tree`, the recipe rule must reproduce LO =
  144 and HI = 152 in both families; otherwise the orchestrator re-rules the constants. L10 C3c's
  verify changed after wave 2's instrument (`93b2615b`), so the re-read is not redundant.

The exact commands are the lane report's (`u/phys-tree-f3`, "window commands").

## 7. The critique's important remarks, resolved

| remark | resolution | where |
|---|---|---|
| **W1** — the window rows' `--expect-pose` fixtures are 600-step files at W 1 and 8 only, so every 500-step row, and every W 2/4/16 process, would exit 4 or fail to parse | The rows carry no `--expect-pose`: each process's `pose_hash` is compared with its row's 500-step hash (J `0x30c5438bc6ad9ffa`, R `0x6cbe24bf8fafda26`), and the kd process's pose file with the leaflist one at the same W | §6 |
| **W2** — G-F3-3's named mutations only reorder slots, which the oracle cannot see | G-F3-3's red-first is a mutation that loses and duplicates an item in the kd order (the leaf sort's write-back dropped); the reorder-only leaf-sort mutations are G-F3-1's; G-F3-4 (i) and G-F3-8 use the order-only leak "the rev scatter walks the Q rows in slot order" (there is no kd query pass to skip a sort in) | §8 |
| **W3** — the index form gathers at every level (2.7M indexed loads at 100k) and its fallback text named a form that needs a new column as "the only remaining form" | The kd sort partitions the items in place with the key cached in the item: no gather, no new column; the working set and access pattern per size are stated; R3's action no longer names an only form | §2, §2.1, §6 |

## 8. Gates (each shown able to fail; the lane records every red)

| gate | what | red under |
|---|---|---|
| G-F3-1 `kd_order_equals_the_reference` (`broadphase_tree/tests.rs`) | `kd_sort` against a `#[cfg(test)]` literal port of `kd_order` (a recursive full sort per node by `(f64 coordinate, index)` with `partial_cmp`, f64 extents, first-max argmax, `ceil(leaves/2)·8`, `list(ids)` at leaves), entry by entry: a proptest (n ≤ 300; 24 under Miri) and fixed cases — n ∈ {0, 1, 8, 9, 16, 17, 64, 65}, a jittered and an exact lattice (ties), all-equal positions, collinear rows, ±0.0 mixes, huge ranges (±2^59), and the f32/f64 axis case (x ∈ {0, 2^24}, y ∈ {−1, 2^24}: f64 picks y, f32 ties and picks x) | `total_cmp` on the coordinate, and `axis_key` without `+ 0.0` (the ±0.0 case); f32 extents (the axis case); last-max argmax (the exact lattice); `left = leaves / 2 · 8` (n = 17); `left = m − ceil(leaves/2)·8` (a partial leaf in the middle); the leaf sort skipped, or run over `m − 1` entries |
| G-F3-2 `kd_build_packs_the_kd_order` | after `build(Kd)`: slot `s` holds the `s`-th item of `kd_sort` on a copy; the rows are a permutation of the input's; `LEAF_MAXROW` is each leaf node's largest live row (the existing max-row test over both orders) | the Kd arm packs the items without calling `kd_sort` |
| G-F3-3 the oracle over three kernels | the kernel draws in G1's single-step worlds, the churn scripts and the sleeper-set scripts become one of `{RowWalk, LeafList, LeafListKd}`; every step checks the AllPairs oracle (and G-LL1 on the step's own tree) | the leaf sort's write-back dropped (an item lost, another duplicated) |
| G-F3-4 `kd_order_moves_the_stream_not_the_pairs` | on J at t = 0 (1 240 boxes), a lattice, a disparity scene and a mixed static scene, two trees stepped separately: (i) `ContactPairs` (stream and withheld) of the `LeafListKd` step equal the `LeafList` step's, bytes; (ii) the two steps' query streams **differ** on J and the lattice (anti-vacuity; a `query_stage` re-run does not rebuild, so the two come from two steps); (iii) G-LL1 on the kd tree | (ii) `leaf_order()` returns Morton for `LeafListKd`; (i) the rev scatter walks the Q rows in slot order (an order-only leak: the set is kept) |
| G-F3-5 the receipt | `TreeDiag::kd_order_builds` += 1 per tree-path step under `LeafListKd`, 0 under `LeafList` / `RowWalk` and on brute steps | the counter bumped outside the kd arm |
| G-F3-6 zero cost when off (report) | release lib, one codegen unit, no LTO, `--emit asm`: `PackedBvh8::build`, `build_active` / `run` and `kd_sort`, before against after | — |
| G-F3-7 build economics (report) | the instruction counts of `kd_sort`'s loops and comparator, and the comparisons a replica of the recursion makes on the 1 240-box pyramid; the predicted Δbuild beside the model's Δquery | — (no stop: the window decides) |
| G-F3-8 the kd pose gate (`tests/broadphase_tree_scenes.rs`) | every Tree rig (J, R, S16, the churn arms, J-Son and R-S sleeping on) under `LeafListKd`: the oracle and the pose bytes equal the AllPairs twin on every step; `kd_order_builds > 0` | the rev scatter walks the Q rows in slot order |
| G-F3-9 kd count pins (`tests/bp_query_counts.rs`) | the model's kd figures (§5) on reuse-off J, uniform / 1 000 and disparity / 1 000, asserted exactly; the reuse-on J kd figures are the tree's reading, recorded by the file's re-pin rule; kd ≠ Morton per scene | the widest axis replaced by the narrowest (all three model scenes) |
| G-TH1 `tree_threshold_band_is_value_neutral` (`tests/broadphase_tree_scenes.rs`) | a Tree world at the DEFAULT threshold and its AllPairs twin: 70 two-box towers on the floor (141 rows), one box spawned every 10 steps to 150 rows and despawned back, twice; sleeping off and on. Every step: the oracle, the pose bytes, and the path derived from the row count against `brute_max_rows()` equals the path the step took (a tree-path step answers leaf-list leaves or withholds pairs; a brute step moves no counter, withholds nothing, holds no sleeper). Anti-vacuity: brute and tree steps, two crossings each way, and (sleeping on) a 145 → 144 crossing with a live sleeper set | `TREE_BRUTE_MAX_ROWS` = 64 (void: no brute step); `all_pairs_into`'s inner loop reversed (the pose, at the first brute step: the order reaches the pose); `n < brute_max_rows` in `step_hinted` (rows 144) |

The ±0.0 rule is covered by G-F3-1 alone: the G4 scenes hold no `−0.0` coordinate
(`bp_g4_scenes.rs`), so G-F3-9 cannot see it.
