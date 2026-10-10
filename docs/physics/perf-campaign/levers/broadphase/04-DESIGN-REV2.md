# Architecture: Tree broadphase (`BroadphaseKind::Tree`) for `boyko_physics`, revision 2

This is a read-only design against tree `D:/wt/joltab` @ `9bd100fb`. I re-read every file:line cited here at that tree, apart from the rows marked "(rebased)". "arith." means the number is computed from code or counters, not measured. The binding inputs are:
- `docs/physics/perf-campaign/levers/00-RULINGS.md` (broadphase and L10 sections)
- `docs/physics/perf-campaign/00-RULINGS.md`
- `docs/measurements/2026-09-19-physics-p0/ANALYSIS.md`

## Changes from rev 1

| remark | resolution | where |
|---|---|---|
| **W1** — with sleeping off, the hint marks every row frozen | The hint is on only if all hold: `cfg.sleeping`, `IslandSleep` is present, and `awake_mask_rows()` equals the row count of the space the mask is read in (N on `Identity`, N_prev on `Rows`). `Reset` turns it off. On `Rows` the hint is read through `prev_row` instead of being switched off. `IslandSleep` gains a `mask_rows` field. Gates: on G2's sleeping-off J, `sleeper_rebuilds == 0 ∧ hint_candidates == 0`, with M6 red; M6b red on G1's stale-mask script; G5-C5 gains a sleeping-off J cfg-A row. | D3.4, step 0, G1, G2, G5 |
| **W2(a)** — the cursor is never stamped | Stamped once per tree-path step, after maintenance (protocol P, `row_identity.rs:210-213`). M5s ("stamp removed") must go red. | step 2, gates |
| **W2(b)** — churn dissolves the sets | A row move is translated, never rebuilt. A despawn is an eviction (kill the leaf, filter the lists), never a rebuild. New members wait as pending and are admitted incrementally by a rent rule. A tree is rebuilt only by admission or by compaction. | D3.2–D3.5 |
| **W2(c)** — N changes under direct drive | Covered by the carried-member count: a member that no row locates (despawned, or row ≥ N) is killed. | D3.2 |
| W2 — complete trigger list | table | D3.3 |
| W2 — upper-bound gates | J: `static_rebuilds == 1` over 600 steps. J-Son / R-S: after the all-frozen step A, at most 1 more sleeper rebuild and 0 evictions; totals pinned. Churn arms: no static rebuild, and the member count never falls by more than the despawned members. | G2 |
| W2 — rebuild cost at 1,240 / 10k / 100k | cost table, plus G4 maintenance arms | D3.5, G4 |
| W2 — `row_identity_churn` arms | G2: oracle plus structural checks on 6 arms × sleeping {off, on}. G4: timed, with a broadphase dimension. The reason they are not in G5 is stated there. | G2, G4 |
| **W2 / review OQ3** — is the verify the sole correctness authority? | **Yes.** Lemma D3.1. The remap trigger is dropped. M5 is redefined as "located through the map, lists not translated" (goes red). Rev 1's M5 is now an equivalent mutation, replaced by M5r (a perf mutation gated by churn counters). | D3.1, gates |
| **W3** — C2 is unmeasured and below build-if | C2 is **deferred** with its own gate. The Tree is serial. `parallel_broadphase` is not flipped. | D6 |
| **W4** — M2 cannot go red | G1 radius-sign classes: all-negative and mixed, plus a directed pair. G1 also **forces the tree path** (`brute_max_rows = 0`): rev 1's G1 at n ≤ 300 would mostly have run the brute loop. | G1 |
| **W5** — `check_step` does not cover the new zones | A per-path table. The runner derives the path from cfg, N and `brute_max_rows()`. The expectation arrays are typed by `SPAN_ZONE_COUNT` / `COUNTER_ZONE_COUNT`, so a zone without an expectation does not compile. | Zones |
| **O1** — allocation wording | "Zero heap allocations per step on every Tree path". C2, if ever built, adds the ruled +1 scope. | Goal, D6 |
| **O2** — whole-step fallback | Per-row kinds: Excluded rows never pair; Wide rows get an exact O(N) loop. | D8 |
| **O3** — cohort placement | 11 ids 399..389, slots 15..5, as the review computed; the floor assert moves to 389 | Data |
| **review OQ1** — how is a random hint injected? | The verify is generic over `H: SleepHint`. G1 moves in-crate (`src/broadphase_tree/tests.rs`) with a test `BitHint`. | D3.4, G1 |
| **review OQ2** — counting-allocator tests at W ≥ 2 | None move: `parallel_broadphase` stays off, and the Tree is serial and heap-free. The census re-run at C4 must show zero diff. | What moves |
| follows from W3 (D5) | The 128 B row blocks existed only for disjoint parallel writes. Serial queries append per-row segments to one stream instead. This removes the 31-entry cap, the spill path, M7, M8 and `spills`. | D5 |
| follows from the L10 ruling B1 | `ContactPairs` stays the exact logical set, so rev 1's OQ2 (axis cache sized P + \|SL\|) no longer applies. | Integration |
| zone anti-vacuity | `profiling_bit_identity.rs:185-200` requires every zone to open. Its harness sets Tree and `brute_max_rows(0)` at C1. | What moves |

## Goal

- **Replace the serial O(n²) AllPairs** with a broadphase that:
  - emits exactly AllPairs' pair set, in `(min, max)` order, so pose bytes are identical at every W and on any machine;
  - queries only moving rows. Statics and sleepers are never queried while their bits are unchanged, and spawn, despawn or migration never dissolves those sets;
  - keeps all durable state in ECS `ScratchColumn`s;
  - makes **zero heap allocations per step on every Tree path**. It runs serially on the calling thread with no `pool.scope`.
- **Targets (arith.; G4 checks them before any end-to-end run):**

| row | today (measured) | target |
|---|---|---|
| J broadphase, W=1 | 2.100 ms | 0.16–0.24 ms |
| J broadphase, W=8 | 1.945 ms | 0.15–0.23 ms (serial) |
| all-asleep broadphase (C5) | 1.93 ms | ≤ 0.03 ms |
| 10k bodies, W=1 | AllPairs 70.2 ms; Grid 8.0 / 15.8 ms | 1.9–2.4 ms |
| a churn step over a stable step, J scale | — | ≤ 0.05 ms |

## Context and constraints

- **Only the exact set is bit-identical.** `begin_frame(pairs.len())` sizes and clears the axis cache (`narrowphase/axis_cache.rs:261-281`).
- **The predicate** is `systems.rs:319-321`, with `r = body_bounding_radius` (`:1218-1223`).
  - For a sphere, `r` is the radius as given. Nothing validates its sign, so it can be negative.
  - For a box, `r = half_extents.length()`.
  - `Vec3::dot` evaluates left to right (`boyko_math/src/vec.rs:177-178`).
- **J scene:** the floor is row 0, static, r ≈ 70.7. The 1,240 boxes have r = √3. P = 9,561.
- **The cursor:**
  - `RemapCursor::remap` classifies each gather as `Identity`, `Rows` or `Reset`, and never stamps. `stamp` marks the state as keyed by the current gather (`row_identity.rs:208-254`).
  - A never-gathered `RowIdentity` classifies as `Identity` (`:470-472`).
  - `prev_row` is injective, because ids are distinct within a gather.
- **The hint mask:**
  - `IslandSleep::awake_rows` is written only by `begin_step` (`resources.rs:3560-3567`), keyed by the rows of the gather it ran on.
  - It is never permuted: `rekey_rows` (`:3310-3332`) permutes only the latch.
  - A read past its range reads NOT-awake (`:3242`, `:3914-3921`).
  - The resource exists on every colored world, whatever `cfg.sleeping` is (`plugin.rs:546-549`).
- **Classes:**
  - A static is `inv_mass == 0 && !kinematic` (`resources.rs:3701, 3717`).
  - Scene sync may rewrite a static's pose every step (`plugin.rs:599-603`).

## Key decisions

### D1. Emit exactly AllPairs' set (unchanged from rev 1)
- Cull bounds use `|r|` plus the slack ε = (‖p‖∞ + |r|)·2⁻²⁰ + 2⁻¹²⁶. That is ≥ 16u against a proven need of ~4.01u.
- The leaf test *is* the AllPairs expression, on bit copies of `(x, y, z, r)`.
- The predicate is symmetric bitwise: fl(a−b) = −fl(b−a), and f32 addition is commutative.
- **Rejected:** an AABB predicate (it changes P, and so values), and `layer`/`mask` filtering (a semantic change).

### D2. Implicit packed 8-wide BVH (unchanged)
- Morton-sorted leaves, 8 per node, all levels in one column.
- One kernel serves the active tree, the static tree and the sleeper tree.
- At J the tree is 179 nodes × 192 B ≈ 34 KB.
- **Rejected:** SAP (150–190k 1D candidates on a pile), a grid (cannot fit mixed sizes), an incremental pointer BVH (Jolt's prepare costs 66 µs/step, measured).

### D3. Persistent static (S) and sleeper (Z) sets, verified per row and carried through row changes

**What.**
- Every Normal row (D8) is in exactly one of:
  - **Q**: queried; lives in the active tree, which is rebuilt every step;
  - **S**: the static set;
  - **Z**: the sleeper set.
- Two sorted pair lists, keyed `(min << 32) | max`:
  - **SS**: static–static pairs;
  - **SL**: pairs with one Z endpoint and the other endpoint in S ∪ Z.
- One verify pass per step locates each row's previous record and decides whether the row is carried, evicted or pending.

**D3.1 The correctness authority (W2, review OQ3).**
- **Lemma.** Suppose X's lists were built from member records {(m, bits_m)}, and φ maps current rows to record rows injectively. If every current row r with φ(r) = m has bits(r) = bits_m, and every other member was removed with its pairs, then the lists mapped through φ⁻¹ and canonicalised equal {(i, j) : i < j, i and j both carry members, pred(bits_i, bits_j)}.
  - *Proof:* pred reads only bits, and φ⁻¹ is a bijection onto the carrying rows. ∎
- **Consequences:**
  - Correctness needs only four things: bit equality, an injective φ, the pairs of removed members filtered out, and every non-member row's pairs found by some query (Q rows query all three trees; Wide rows loop over all rows).
  - The lemma never reads entity identity, class, the hint or the cursor. Those choose φ and membership, which changes cost, never output.
  - **Decision: the bit compare, together with the carried count and the consumed mark, is the sole correctness authority. Rebuilding on `Rows` buys nothing and is dropped.**
  - The class check is membership hygiene (S holds statics), not correctness.
- **The locator φ:**
  - identity on `Identity`, on `Reset` and under direct drive;
  - `prev_row` on `Rows`.
  - Injectivity is enforced, not assumed: a located record is marked consumed, so a second locate fails and that row becomes Q.

**D3.2 Per-row states (one pass).**

| state | condition | effect |
|---|---|---|
| carried | the located record is a member of X, the row is Normal, bits are equal, and the class fits X (S: static; Z: hint frozen) | stays in X. The leaf slot moves with the record. On `Rows`, `inv[m] = r`. |
| evicted | the located record is a member but not carried | the leaf lane is killed, its pairs filtered; the row becomes Q (or Excluded or Wide) |
| vanished | \|X\| − carried − evicted > 0: members that no row located (despawned, row ≥ N, a duplicate in a broken map) | one scan over X's leaves kills them |
| pending | a Normal non-member that is either static and *still* (bits equal to its located record), or hint-frozen (Z) | Q this step; accrues rent |

**D3.3 Complete trigger list.**

| event | detected by | action | counter |
|---|---|---|---|
| rows unchanged | cursor `Identity` | verify in place | — |
| rows changed (spawn, despawn, migration, burst) | cursor `Rows` | locate via `prev_row`; rewrite list entries and leaf rows through `inv`; patch out-of-order entries | `translations`, `patches` |
| missed gather, another `RowIdentity`, or direct drive | cursor `Reset` / never gathered | identity locator (sound by D3.1); hint off this step | `locator_resets` |
| member moved, reshaped, teleported or woken | bit compare | evict | `evictions` |
| member's class no longer fits (static↔dynamic, kinematic, hint now awake, hint turned off) | class compare | evict | `evictions` |
| member not located (despawn, tail row ≥ N) | count mismatch | leaf scan, kill, filter | `evictions` |
| member becomes Excluded or Wide | kind | evict | `evictions` |
| new static (spawn, migrate in, class flip) | class static, not a member | Q; pending once still | — |
| new sleeper candidate | hint frozen, not a member | Q; pending | `hint_candidates` |
| rent threshold reached | rent rule | admission | `*_rebuilds` |
| dead lanes ≥ max(live, 64) | leaf counts | compaction (tree rebuilt, no queries) | `*_rebuilds` |
| N crosses `brute_max_rows` | N | brute loop; state untouched, cursor not stamped, so the next tree step is a `Reset` | — |

**D3.4 The sleep hint (W1, as ruled).**
- **On** ⇔ `cfg.sleeping` ∧ `IslandSleep` present ∧ `mask_rows == |space|`, where the space is N on `Identity` and N_prev on `Rows`. `Reset` ⇒ off.
- **Reading it:**
  - On `Identity`: frozen(r) = `!is_row_awake(r)`.
  - On `Rows`: `prev_row[r] == NO_ROW` means awake; otherwise frozen = `!is_row_awake(prev_row[r])`.
- **Off ⇒** no candidates, and every Z member fails the class fit and is evicted in one filter pass.
- **Z admission has no still test.** A correct hint already implies still (a frozen row is not integrated), so a still test would only hide a wrong hint, which is exactly what W1's gate has to see.
- **S admission requires still,** because statics have no hint. A static rewritten every step never qualifies and is queried every step, which is its true cost.
- A stale mask that still covers N only costs evictions for one step.
- The verify is generic over `H: SleepHint`: production uses `MaskHint` and `NoHint`, the tests use `BitHint`. The locator is a per-step branch hoisted out of the loop (a const generic). That makes at most 4 small loop instantiations, and only one runs per step.

**D3.5 Admission (rent rule), and costs.**
- **The rule:**
  - Each step, `rent[X] += pending[X]`. Rent resets on admission, and when `pending[X] == 0`.
  - Admit when `rent[X] ≥ pending[X] + ADMIT_BUILD_RATIO·(|X| + pending[X])`.
  - `ADMIT_BUILD_RATIO` starts at 0.25; G4 measures (c_build + k·c_list)/c_q, which is 0.14–0.24 by arith.
- **Admission is incremental and cold:**
  1. Rebuild X's tree over members ∪ pending (radix only; old members are not re-queried).
  2. Query each pending row against X (the partner is an old member, or a pending row with a larger row) and against the other set's tree.
  3. Sort the additions and merge them into SS/SL.
  - S is admitted before Z in the same step, so a new-static/new-sleeper pair is found exactly once.
- **Compaction** rebuilds the tree over live leaves only; the lists are already filtered.
- **Bound:** admission costs ≤ the rent paid + p queries. That is ≤ 2× the cheaper of "query pending rows forever" and "admit them at once" (deterministic ski rental, [Karlin et al. 1988](https://link.springer.com/rwe/10.1007/978-0-387-30162-4_378)). Compaction amortises to ≤ c_build per eviction.

**Costs (arith.).** Parameters: c_build is 12–20 ns/row (20–30 at 100k); c_q is 100–150 / 150–200 / 250 ns; c_list is 1.5–2.5 ns per entry; |L| ≈ 7.7·m.

| operation | m = 1,240 (\|L\| ≈ 9.5k) | 10k (77k) | 100k (770k) |
|---|---|---|---|
| verify, every step (`Identity`), N·2–4 ns | 2.5–5 µs | 20–40 µs | 0.2–0.4 ms |
| copy of persistent pairs, every step | 14–24 µs | 0.12–0.19 ms | 1.2–1.9 ms |
| translation (`Rows` step), N·4–6 ns + \|L\|·c_list | 19–31 µs | 0.16–0.25 ms | 1.6–2.5 ms |
| eviction filter (a step with ≥ 1 eviction) | 14–24 µs | 0.12–0.19 ms | 1.2–1.9 ms |
| compaction | 15–25 µs | 0.12–0.20 ms | 2–3 ms |
| admission of 64 rows | 36–60 µs | 0.25–0.40 ms | 3.2–4.9 ms |
| admission from empty (= rev 1's rebuild) | 0.15–0.23 ms | 1.7–2.4 ms | 28–30 ms |

- For 64 pending rows, admission comes after ≈ 6 / 40 / 390 steps. While they wait, they cost 6–10 / 10–13 / 16 µs per step as Q rows.
- Rev 1 paid the last row of the table on **every** `Rows` step. At 100k statics that is 28–30 ms per spawn.

**Rejected:**
- *Rebuild on `Rows` (rev 1):* costly, and D3.1 shows it adds no correctness.
- *An identity-only locator:* a row shift evicts everything in spawn-heavy scenes.
- *LIS-based eviction of jumpers:* O(m log m) on every `Rows` step. The patch touches only the entries that moved.
- *Full rebuild on admission:* 28–30 ms at 100k, against 3–5 ms incremental.
- *ECS change ticks and trusting the latch:* rev 1's reasons stand.

**Trade-off:**
- Two record buffers of N × 32 B each (80 KB at J).
- A `Rows` step costs O(N + |L|).
- Correctness costs nothing extra.

### D4. Re-find active pairs every step (unchanged)
A persistent active pair set would need pair identity across row changes, which does not exist before U5. Its gain, 0.05–0.1 ms, is below the SE bar.

### D5. Output: per-row segments in one stream, lexicographic by construction
- **Query stage:** Q rows, in Morton order, append a segment `[partners < row | partners > row]` to `aux`. The segment is sorted by insertion sort up to 32 entries, `sort_unstable` above. The record keeps `(seg, nrev, nfwd)`. Wide rows append after the Q rows.
- **Assembly:**
  1. Count rev entries per target row.
  2. Exclusive prefix → bucket starts (in `aux`, after the stream).
  3. Scatter, walking rows in ascending order, so every bucket arrives sorted.
  4. `out.resize(P)`.
  5. For each row in ascending order, merge its fwd part, its SS run, its SL run and its bucket (the SS/SL cursors only move forward). When only one run is non-empty, it is a straight copy.
- **Why (a consequence of W3):** the fixed 128 B blocks existed for disjoint parallel writes. Serially they only cost:
  - 159 KB written per step at J, against ~38 KB of segments;
  - a 31-entry cap with a spill path.
  - Segment reads in row order are prefetched 4 rows ahead.
- **Rejected:** a global radix sort of keys; a two-pass count-then-emit; querying in row order (it gives up Morton locality in the tree, which dominates at 100k).

### D6. Serial in this lane; the parallel query wave (C2) is deferred (W3)
- **The arithmetic:** C2 would save t_q·(1 − 1/(8E)) − ω(8) = 0.095–0.145 ms at J, W=8.
  - That is 1.0–1.6 % of T(8) = 9.162 ms, and 2.4–3.9 % of the 3.70–3.98 ms projection.
  - Both are below the 5 % build-if rule.
- **Trigger:** G4 measures t_q. C2 is built only if that predicts ≥ 5 % of a measured T(8) on a gated row, or if an end-to-end row with ≥ 8k bodies is added.
- **C2's own gate if built:**
  - same binary, a serial/parallel flag, W=8, K ≥ 6, the SE claim rule;
  - realized Δ ≥ 0.6 × predicted and ≥ 5 % of T(8). A serial tree gives Δ = 0 and fails.
  - It would add the ruled +1 pool scope per step, pinned in the census.
- **`parallel_broadphase`** keeps its meaning (Grid only) and its default, `false`.

### D7. Defaults
- `Tree` becomes the default kind (C4).
- When N ≤ `brute_max_rows` (default `TREE_BRUTE_MAX_ROWS`, set from G4), the Tree runs `all_pairs_into`.
- Auto's high side moves to Tree, rebased on L2's recalibrated thresholds.
- Grid stays for the SP2 coupling (forced there, with a Manual pin) and for Manual use.

### D8. Row kinds: a per-row fallback (O2)
- **Excluded:** any NaN in (x, y, z, r), or a non-finite position with |r| ≤ 2⁶⁰.
  - Against any non-Wide row, bound² ≤ 2¹²² is finite while len² is +inf or NaN, so the predicate is false. The row pairs with nothing but Wide rows.
- **Wide:** r non-finite, or |r| > 2⁶⁰, or a finite row with ‖p‖∞ + |r| > 2⁶⁰.
  - It gets an exact O(N) loop over all rows, including Excluded ones, skipping Wide rows below it (those rows own the pair).
- **Normal:** everything else. Every intermediate in the kernel stays finite (bound² ≤ 2¹²², |d|² < 2¹²⁶).
- A member whose kind leaves Normal is evicted.
- Counters: `wide_rows`, `excluded_rows`.

## Data structures

```rust
#[repr(C, align(32))] pub(crate) struct Node8 { p: [[f32; 8]; 6] } // 192 B = 3 lines (column base 64-aligned)
// internal: min_x,min_y,min_z,max_x,max_y,max_z (empty lane +inf/-inf)
// leaf: x, y, z, r, row bits, _ (empty or killed lane: x = +inf, row = u32::MAX; the test is always false)

pub(crate) struct PackedBvh8 {
    nodes: ScratchColumn<Node8>, // level 0 first, root last
    level_start: [u32; 13], levels: u8,
    leaves: u32,                 // live + dead
    dead: u32,                   // killed lanes (S and Z only)
}

/// One per row per step. Two buffers: swapped on Rows/Reset steps, updated in place on Identity.
#[repr(C, align(32))]
struct RowRec {
    x: f32, y: f32, z: f32, r: f32, // bits read this step: the verify and still reference
    tag: u32,  // leaf slot:24 | set:2 {None,S,Z} | kind:2 {Normal,Wide,Excluded} | still:1 | consumed:1 | pending:1 | _:1
    seg: u32,  // Q and Wide rows: segment start in `aux` (this step)
    nrev: u32, // entries < row at the front of the segment
    nfwd: u32, // entries > row after them
}                                   // 32 B: 2 rows per line

#[repr(C, align(32))] struct Item { x: f32, y: f32, z: f32, r: f32, row: u32, _p: [u32; 3] } // 32 B

#[derive(Resource)]
pub struct BroadphaseTree {
    active: PackedBvh8, statics: PackedBvh8, sleepers: PackedBvh8, // 3 columns
    sort_a: ScratchColumn<u64>, sort_b: ScratchColumn<u64>, // radix ping-pong; cold: patch entries, additions
    items: ScratchColumn<Item>,                             // Q rows in row order; cold: admission input
    rec: [ScratchColumn<RowRec>; 2], cur: u8,              // 2 columns
    ss: ScratchColumn<u64>, sl: ScratchColumn<u64>,         // persistent, strictly sorted (min<<32 | max)
    aux: ScratchColumn<u32>,  // Rows: inv map | query: segment stream | assembly: + bucket starts (N) + rev entries
    cursor: RemapCursor,      // 16 B
    rent: [u64; 2], brute_max_rows: u32,
    diag: TreeDiag,
}
```

- **Cohort `BROADPHASE_TREE`:** 11 ids, 399..389, below `SCRATCH_ID_ROW_IDENTITY_BOTTOM` = 400; stagger slots 15..5.
  - The `const` asserts prove the cohort clear of `BODY_STATE` (slot 63) and `CONTACT_PAIRS` (id 446, slot 62), which the verify and the assembly sweep beside it.
  - They also prove it clear of `ROW_PREV` (403, slot 19) and `TOUCHED_AWAKE` (in the solver cohort, slots 20..63), which the verify sweeps on `Rows` and hint steps.
  - Width ≤ 64.
  - The floor assert (`scratch_ids.rs:734-739`) moves to 389. `SCRATCH_REGION_MIN_ID` stays at 384.
- **Reserves:** `aux` ≥ 2 × the `ContactPairs` reserve + the rows reserve (the stream is ≤ P, the rev entries ≤ P, the bucket starts N).
- **Function-local scratch:** the traversal stack `[u32; 96]` (bound 7·levels + 1) and the radix histogram `[u32; 1024]`.
- **`IslandSleep`** gains `mask_rows: u32`, set beside `awake_rows.reset(n_rows)` (`resources.rs:3560`).
- **`TreeDiag`** (pub, `Copy`, all `u64`): `static_rebuilds`, `sleeper_rebuilds` (admissions + compactions), `evictions`, `translations`, `patches`, `hint_candidates`, `wide_rows`, `excluded_rows`, `locator_resets`, and `members` (the current |S| + |Z|).

## Public API

```rust
pub enum BroadphaseKind { AllPairs, Grid, Tree }   // Tree is the default from C4
pub const TREE_BRUTE_MAX_ROWS: u32;                // from G4
pub struct TreeDiag { /* counters above */ }
impl BroadphaseTree {
    pub fn with_capacity(rows: usize) -> Self;
    pub fn diag(&self) -> TreeDiag;
    pub fn set_brute_max_rows(&mut self, rows: u32); // 0 forces the tree path (tests, harness, benches)
    pub fn brute_max_rows(&self) -> u32;
}
#[inline] pub fn sphere_bound_feasible(pa: Vec3, ra: f32, pb: Vec3, rb: f32) -> bool; // the one predicate; Grid delegates
pub fn all_pairs_into(bodies: &[BodyState], out: &mut ContactPairs);                // brute path and the gates' oracle
pub fn physics_broadphase(scratch: Res<SolverScratch>, cfg: Res<PhysicsConfig>,
    grid: ResMut<BroadphaseGrid>, tree: ResMut<BroadphaseTree>,
    sleep: Option<Res<IslandSleep>>, pairs: ResMut<ContactPairs>);
// pub(crate), the seam for L10 rev 2: frozen_pairs() -> &[u64] (SL); row_set(row) -> RowSet
// pub(crate) on IslandSleep: awake_mask_rows() -> usize
```

## Algorithms for the critical paths (Tree arm, N > `brute_max_rows`)

| # | step (zone) | complexity | cache and branching | SIMD |
|---|---|---|---|---|
| 0 | Locator `cursor.remap(rows)`; the hint gate (D3.4) | O(1) | — | — |
| 1 | **Verify** (`phys_bp_verify`). Per row: bits (one radius sqrt), kind, class, hint, locate, then carried / evicted / pending; write the record; on `Rows`, `inv[m] = r` | O(N) | Streams `BodyState` (2 lines/row) and 1 record line per 2 rows. `Rows` adds `prev_row` (4 B/row) plus a near-sequential gather of old records. One predictable branch per row. | scalar |
| 2 | **Maintenance** (`#[cold] #[inline(never)]`, still inside the verify zone): vanished-leaf scan and kill; one list pass (translate through `inv` on `Rows`, drop dead endpoints, **patch**); compaction; admission of S, then of Z; **then stamp the cursor, unconditionally** | D3.5 | runs only on steps with a change | query kernel |
| 3 | **Active build** (`phys_bp_build`) over Q | O(\|Q\|) | as rev 1; `items` is ~40 KB at J | bounds, levels |
| 4 | **Query** (`phys_bp_query`): Q leaves in Morton order against the active tree (`row > a`), S and Z; the exact test; the segment goes to `aux`. Then the Wide rows' loops. | O(\|Q\|(log n + k) + \|Wide\|·N) | Trees in L1/L2 at J, ~10–17 8-wide tests per query; one lane-mask loop (tzcnt) | overlap: 6 compares + and. Exact: `t=dx*dx; t=t+dy*dy; t=t+dz*dz; t <= (r+q)*(r+q)` with `LE_OQ`, mul+add only, no FMA |
| 5 | **Assemble** (`phys_bp_assemble`): rev counts → bucket starts → scatter over rows ascending → `out.resize(P)` → per-row merge | O(N + P) | streaming; segments prefetched 4 rows ahead | — |

- **The patch rule.** A translated entry e stays in place iff it is greater than the last kept entry and smaller than the next entry's translated key. Otherwise it is diverted to `sort_a`.
  - The kept entries are strictly increasing, so the result is sorted by construction.
  - The diverted run is sorted and merged back from the end, in place.
  - Cost: O(|L| + a log a). If a > |L|/4, the whole list is sorted instead.
  - Spawns and row shifts are monotone (a = 0). A swap-remove or migration moves 1–2 "jumper" rows, and a covers only their entries.
- **J at W=1 (arith.):** verify 2.5–5 µs, build 15–25 µs, query 124–186 µs, assembly 16–27 µs ⇒ **0.16–0.24 ms**. The floor is Q at steps 0–1 and an S member from step 2.

## Multithreading model
- **Model:** one serial system on the scheduling thread. No pool, no atomics, no locks. Zones open only on the calling thread (ruling O1).
- **Reads:** `SolverScratch` (`bodies`, `rows`), `PhysicsConfig`, `IslandSleep`. **Writes:** `BroadphaseTree`, `ContactPairs`.
- **Scheduling:** `Res<IslandSleep>` conflicts only with the solve's `ResMut<IslandSleep>`. The solve is already ordered after the broadphase (`plugin.rs:629-633`, then narrowphase → graph → solve), so no new serialisation. The developer confirms this with the scheduler's conflict report.
- **Send/Sync:** a resource made only of `ScratchColumn`s, like `BroadphaseGrid`. Drop order is irrelevant: no column points into another.

## Determinism argument
1. D1's cull is conservative and the leaf test is exact, so each Q row finds exactly its partners among the tree rows.
2. The Wide loop is exact. Excluded rows pair only with Wide rows (D8).
3. The persistent pairs are exact by Lemma D3.1.
4. Each pair is emitted exactly once:

| pair kind | emitted by |
|---|---|
| Q–Q | the query of the smaller row |
| Q–member | the Q row's query |
| member–member | SS / SL |
| Wide–anything | the Wide row (the smaller one, if both are Wide) |
| Excluded–non-Wide | never emitted (the predicate is false) |

5. Placement uses only integer counts, a prefix over rows, and per-row sorted runs.

**Therefore `ContactPairs` is a function of the exact set alone.** It does not depend on:
- W;
- tree shape or Morton quantisation;
- admission or eviction history;
- the hint or the locator;
- AVX2 against the scalar kernel.

It is bit-identical to AllPairs on any machine. That gives replay determinism, and the axis cache sees the same P.

## Expected gain (from P0b; the "bp after" column is arith.)

| row, W | T before (measured) | bp before | bp after | Δ | share of T | SE bar (K=6) |
|---|---|---|---|---|---|---|
| J cfg-A, 1 | 19.671 | 2.100 | 0.16–0.24 | 1.86–1.94 | 9.5–9.9 % | 2.0 % |
| J cfg-A, 2 / 4 | 13.711 / 10.701 | not armed | 0.16–0.24 | ≈1.8–1.9 | ≈13–14 / 17–18 % | 1.4 / 1.2 % |
| J cfg-A, 8 | 9.162 | 1.945 | 0.15–0.23 | 1.72–1.80 | 18.8–19.6 % | 0.75 % |
| J cfg-A, 16 | 9.172 | not armed | as W=8 | ≈1.72–1.80 | ≈19 % | 0.5 % |
| R, 1 | 14.226 | 2.08 | 0.16–0.24 | 1.84–1.92 | 12.9–13.5 % | 3.3 % |
| R, 8 | 9.933 | not armed | 0.15–0.23 | ≈1.7–1.8 | ≈17–18 % | 1.3 % |

- **Sleeping floor (C5):** the broadphase becomes the verify plus the SL copy, 0.02–0.03 ms against 1.93 ms measured.
  - J-Son tail: 5.334 → ≈3.43 ms (−36 %).
  - R-S: 6.049 → ≈4.15 ms (−31 %).
  - Bit-identical to today's sleeping-on runs.
- **Projection (not a claim):** cfg-A + simd + L5 is 5.5–5.7 ms at W=8 (ANALYSIS §9). Minus Δ(8) gives 3.70–3.98 ms, i.e. 1.05–1.13× Jolt v5.3.0 (3.533).
- **Bench only:** 10k ≈ 1.9–2.4 ms and 100k ≈ 25 ms, at every W.

## Integration

**New files:**
- `D:/wt/joltab/crates/boyko_physics/src/broadphase_tree/mod.rs`: the resource, verify, maintenance, assembly and the arm.
- `D:/wt/joltab/crates/boyko_physics/src/broadphase_tree/bvh.rs`: build, query and kill.
- `D:/wt/joltab/crates/boyko_physics/src/broadphase_tree/kernel.rs`: the kernel. AVX2 under `cfg(target_feature="avx2")`; the scalar kernel also serves Miri.
- `D:/wt/joltab/crates/boyko_physics/src/broadphase_tree/tests.rs`: G0 and G1, `#[cfg(test)]`.
- `D:/wt/joltab/crates/boyko_physics/tests/broadphase_tree_scenes.rs`: G2 and G3, `cfg(not(miri))`.

**Edits:**

| file:line | change |
|---|---|
| `src/lib.rs` (module list) | `pub mod broadphase_tree;` |
| `src/systems.rs:277-349` | Docs, signature (`tree`, `sleep`) and the Tree arm. The AllPairs arm `:309-327` stays verbatim; `:344-348` unchanged. |
| `src/resources.rs:45-53` | the `Tree` variant |
| `:149-154`, `:225-249` | docs (`parallel_broadphase` affects Grid only) |
| `:458` | default kind → `Tree` (C4) |
| `:493` | unchanged (`false`) |
| `:1486-1493` | Grid's `feasible` delegates to `sphere_bound_feasible` |
| `:3108-3159`, `:3560` | `IslandSleep::mask_rows` and its accessor (C5) |
| `src/broadphase_policy.rs` (rebased) | Auto's high side → Tree; constants from G4 |
| `src/plugin.rs:513-517` | the coupling path also sets `broadphase_select: Manual` |
| `src/plugin.rs:526` | insert `BroadphaseTree::with_capacity(INITIAL_BODY_CAPACITY)` |
| `src/scratch_ids.rs` after `:704`; `:731-739` | the cohort, its asserts and its register function; the floor assert moves |
| `src/profiling.rs:77-131` | 4 spans, 3 counters, `SPAN_ZONE_COUNT` = 18, `COUNTER_ZONE_COUNT` = 9; module docs |
| `benches/broadphase.rs:205` | Tree arms and maintenance arms |
| `benches/row_identity_churn.rs` | a broadphase dimension; settle 300 for the sleeping-on arms |
| `benches/jolt_parity_pyramid.rs:936-997` (C1), `:723-749` and `:53-55` (C3) | per-path `check_step`; `--broadphase allpairs\|tree\|grid` applied after `configure` |
| `docs/SYSTEMS.md`, `docs/FEATURE_MAP.md` | entries |

- **Untouched:** `narrowphase/axis_cache.rs` (P is identical), `solver/*`, `soft/coupling.rs`.
- **L10 seam:** `ContactPairs` stays the exact logical set, as ruling B1 requires. SL and per-row membership are exposed `pub(crate)`, so L10 rev 2 can iterate the awake stream and the sleeping store together.

## Commit sequence (each commit green)
- **C1 `feat(physics): tree broadphase, serial`**
  - The module and the Tree arm (not the default); the S set with verify, translation, eviction and admission; Wide and Excluded kinds; the brute path; the cohort; the predicate extraction.
  - The zones and counters, per-path `check_step`, and the harness set to Tree with brute 0.
  - Gates G0, G1, and G2 on J plus the sleeping-off churn arms.
- **C2: DEFERRED** (D6).
- **C3 `bench(physics)`**
  - G4 arms, the churn dimension, the runner's `--broadphase` flag.
  - **Quiet window, the owner confirms first:** G4, then G5 on C3's binary.
- **C4 `perf(physics): tree by default`**
  - The kind flips; `parallel_broadphase` does not. Constants from G4; Auto retargeted; the coupling pinned to Manual.
  - Moved tests; census and attribution re-run with zero diff; G5 recorded under `docs/measurements/<date>-broadphase-tree/`.
- **C5 `perf(physics): sleeper set`**
  - Z, SL, the hint and `mask_rows`.
  - G1's hint scripts; G2's R-S, J-Son and sleeping-on churn arms; G5-C5.

## Gates (each shown able to fail)

- **G0 — kernel equals scalar (in-crate proptest).**
  - Inputs: exact-boundary constructions (|d| = |r_a + r_b| ± k ulp), coordinates up to ±1e6, subnormals, ±0, negative radii, ±inf, NaN.
  - M0 must go red.
- **G1 — pair set equals AllPairs on every step (in-crate; `brute_max_rows = 0` except in the brute case).**
  - **Single-step worlds:**
    - n ≤ 300, radius magnitudes 1e−3..1e3;
    - **sign classes all-positive / all-negative / mixed, each in ≥ 25 % of cases, plus the directed pair r = −1, −1 at distance 1.5**;
    - 0–30 % static, 0–10 % kinematic, duplicate positions;
    - Excluded rows (NaN, ±inf position) and Wide rows (±inf r, |r| > 2⁶⁰, ‖p‖∞ ≈ 2⁶¹);
    - one body with ≥ 40 partners;
    - the brute case at N ≤ `TREE_BRUTE_MAX_ROWS`.
  - **Multi-step scripts over a real `RowIdentity`,** driven the way `row_identity.rs`'s tests drive it:
    - row changes: an append spawn; a spawn into an earlier archetype (a shift); a swap-remove despawn of a member and of a non-member; a migration in each direction (low→high and high→low jumpers); a burst of 32; a recycled id respawned at the same pose; a missed gather (`Reset`); direct drive with N growing and shrinking, with a member in the removed tail;
    - static changes: a teleported or reshaped static; a static rewritten every step, which must never be admitted; a kinematic toggle; a static↔dynamic flip;
    - hints: a random `BitHint` every step; the **stale-mask script** (sleeping off, then a spawn, then sleeping on, which leaves `mask_rows ≠ N_prev`).
  - **Assertions:** `Vec` equality with `all_pairs_into` on every step, plus each script's structural counts.
  - **Miri:** the proptest at n ≤ 24, 16 cases, scalar kernel.
- **G2 — scene oracle on every step, plus structural bounds** (Tree forced; `all_pairs_into` on each snapshot):

| scene | structural assertions |
|---|---|
| J, 600 steps, sleeping off | `static_rebuilds == 1` (at step 2); **`sleeper_rebuilds == 0`, `hint_candidates == 0`**; evictions, translations, wide and excluded all 0; members == 1 from step 2 |
| J-Son and R-S, 600 steps | `static_rebuilds == 1`; after A (the first step with every row hint-frozen, recorded): Δ`sleeper_rebuilds` ≤ 1 and Δevictions == 0; the total `sleeper_rebuilds` is pinned (it is deterministic) |
| churn scene (`row_identity_churn`'s): 6 arms × sleeping {off, on}; settle 30 (off) / 300 (on), then 40 churn steps | Every arm: Δ`static_rebuilds` == 0. Non-stable arms: Δ`translations` == 40; stable: 0. Sleeping on: members ≥ 1,240 at window start (else void); **members(t) ≥ members(t−1) − despawned(t)**; Δevictions ≤ despawned; Δ`sleeper_rebuilds` pinned per arm. |

- **G3 — pose bytes.** The Tree's pose hash equals serial AllPairs' at W ∈ {1, 2, 4, 8, 16}, over J 600 steps and R-S 600 steps. The runner's `--expect-pose` is taken from the L5 lane, or added here if it is missing.
- **G4 — bench and build-if** (cargo `bench` profile):
  - **Pair finding:** Tree (W=1), AllPairs, and Grid (W ∈ {1, 8}) at n ∈ {17, 64, 128, 256, 1k, J snapshot, 10k, 100k} × {uniform, size disparity}.
    - Stop and investigate if the J snapshot exceeds 0.30 ms, or if the Tree is slower than the Grid at the same W above the brute threshold.
    - Outputs: `TREE_BRUTE_MAX_ROWS`, `AUTO_TREE_LO` / `AUTO_TREE_HI`.
  - **Maintenance** at m ∈ {1,240, 10k, 100k}: build from empty, admission of 64, eviction filter, one-row shift translation, compaction.
    - Stop if any exceeds 2× the upper value in D3.5.
    - Output: `ADMIT_BUILD_RATIO`.
  - **t_q** at J and 10k: the input to C2's decision.
  - **Churn** (`row_identity_churn` × {allpairs, tree}, sleeping-on settle 300): K=6 separate processes per cell, median and SE.
    - The Tree must be faster than AllPairs, claimed, on every arm.
    - Tree(arm) − Tree(stable) must be ≤ 0.05 ms, or not claimed above that.
  - **Why not G5:** the parity runner has no structural-change mode, and G5's claim is T(W) on J and R. The churn property is structural (G2's non-dissolution check, which can fail) plus this bench delta.
- **G5 — end to end under the P0 protocol.**
  - **Setup:** C3's binary, `--broadphase allpairs|tree`. Rows: J `--cfg a` (the measured headline row) and R `--cfg default`. W ∈ {1, 2, 4, 8, 16}, K=6.
  - **Protocol:** the median of window means over K processes; receipts gate each process; the canary J-C must be seen; order interleaved and reversed on alternate passes.
  - **Claim rule:** |effect| > 2·√(SE_A² + SE_B²).
  - **Armed runs at W=1 and W=8:**
    - the tree span must be ≤ 0.36 / 0.35 ms (1.5 × the predicted upper), else stop;
    - realized Δbp(8) must be ≥ 0.6 × 1.72 = 1.03 ms;
    - `check_step` must be green on every step.
  - S16: no claimed regression (the brute path). Pose hash equal (G3).
  - **C5** (C4's binary against C5's):
    - J-Son tail [264, 1000) and R-S at W=1 and W=8: the floor is claimed, and the pose hash equals C4's sleeping-on hash;
    - **J cfg-A sleeping off at W=1 and W=8: no claimed regression, and the receipt prints `sleeper_rebuilds == 0`.**
- **Mutations** (each required red):

| id | mutation | goes red in |
|---|---|---|
| M0 | the dz term via `_mm256_fmadd_ps` | G0 |
| M1 | bounds use r·(1−2⁻²⁰) | G1 boundary cases |
| M2 | bounds use `r`, not `\|r\|` | G1 negative and mixed radii (an inverted box never overlaps) |
| M3 | rev entries dropped | G1; G2 J from the admission step (2) |
| M4 | bit compare skipped | G1 teleport and random-hint scripts |
| M5 | on `Rows`, members located through `prev_row` but lists and leaf rows not translated | G1 shift/swap/migrate scripts; G2 churn arms, at the first churn step |
| M5s | cursor stamp removed | G2 churn arms (`translations` 0 ≠ 40); J-Son/R-S pinned total (hint off ⇒ 0) |
| M5r | rev 1's rule: rebuild both sets on every `Rows` step | G2 churn: Δ`static_rebuilds` == 0 fails |
| M6 | hint gate removed (both conditions) | G2 J sleeping off: `sleeper_rebuilds == 0` fails (admission at step 1) |
| M6b | coverage condition removed | G1 stale-mask script: Δ`hint_candidates` == 0 fails |
| M9 | consumed mark not written | G1 unit test with a hand-made non-injective `Rows` map (a duplicate row loses its pairs) |
| M10 | the Wide loop skips Excluded rows | G1 (an r = +inf row against a ±inf-position row) |
| M11 | the patch leaves out-of-order entries in place | G1 swap/migrate scripts |
| M12 | an evicted leaf is not killed | G1 teleport and despawn scripts |

- **Unit tests** (tests only, no mutation of their own): the rent-rule arithmetic, patch-merge edge cases, and `Morton` codes over huge ranges.
- **`debug_assert!`:**
  - `items` rows strictly increasing;
  - each segment sorted and split at its own row;
  - buckets sorted; SS and SL strictly sorted;
  - carried + evicted + vanished == |X|;
  - each member's leaf slot points back to its row;
  - no Excluded or Wide row in any tree;
  - `dead ≤ leaves`;
  - the existing output-order assert (`systems.rs:344-347`).
- **`unsafe`:** the AVX2 intrinsics (`target_feature`) and unchecked node indexing, each with `// SAFETY:` (node index < `level_start[L+1]`; stack depth ≤ 7·levels + 1).

## Zones and per-path expectations (W5)

- **Zones:** spans `phys_bp_verify`, `phys_bp_build`, `phys_bp_query` and `phys_bp_assemble`; counters `phys_bp_queried` (value = |Q| + |Wide|), `phys_bp_members` (|S| + |Z|) and `phys_bp_rebuilds` (the Δ of both rebuild counters this step).
- All are tier Deep and open only on the calling thread.
- The runner derives the path from `cfg.broadphase`, N and `tree.brute_max_rows()`. It never reads the path from the implementation.

| path | each of the 4 spans | queried (samples, value) | members | rebuilds | `phys_bp_pairs` |
|---|---|---|---|---|---|
| AllPairs; Grid; Tree with N ≤ brute | 0 | (0, 0) | (0, 0) | (0, 0) | (1, P) |
| Tree with N > brute | 1 | (1, q) | (1, m) | (1, Δ`diag`) | (1, P) |

- On J and R the runner also asserts q + m == N and that `diag().wide_rows` and `diag().excluded_rows` are unchanged.
- The expectation arrays are `[_; SPAN_ZONE_COUNT]` / `[_; COUNTER_ZONE_COUNT]`, checked in `SPAN_ZONES` order (as `profiling_zone_counts.rs:253,277` does).
- Mutation: a `phys_bp_query` guard opened inside the per-leaf loop gives count |Q| ≠ 1, so `check_step` goes red.

## What moves (everything is bit-identical, so no value pin moves)
- **`profiling.rs`:** the arrays grow to 18 / 9, plus the count consts; "twenty zones" becomes 27.
- **`tests/support/profiling_harness.rs`:** Tree + `set_brute_max_rows(0)` at C1. Otherwise `profiling_bit_identity` goes red with "never opened `phys_bp_verify`", which is its anti-vacuity working.
- **`profiling_zone_counts.rs`:** the new rows, plus an AllPairs-path case with zeros.
- **`alloc_frame_census.rs`, `alloc_frame_attribution.rs`:** no pin moves. C4 re-runs both, and any per-step diff is a defect. Mutation: a per-step `Vec` in the Tree path goes red in S1c.
- **`broadphase_select_p3.rs`, `broadphase_policy.rs` tests:** Auto's high side is Tree.
- **`default_world_pyramid_determinism.rs:6-8, 26-29`:** docs only.
- **`soft_body_sp2.rs`:** stays green with no edit (the Manual pin).

## What it cannot claim
- **Nothing downstream.** The pair set is unchanged, including the ~1,015 non-touching slab pairs.
- **No W scaling.** The broadphase is serial; at J it is ≈ 2 % of T(8), and C2 is deferred.
- **Persistent pairs are still copied every step.** Statics and sleepers are touched once per step by the verify (2–4 ns/row), and their pairs are copied every step. That copy is 1.2–1.9 ms at 100k with ~770k persistent pairs: the price of materialising the exact set. L10 rev 2 may replace the copy with a logical view.
- **A static rewritten every step** is queried every step. That is its correct cost, not a gain.
- **A Wide row** costs O(N) per step.
- **10k and 100k figures are bench-only.** No end-to-end row exercises admission at those sizes.
- **Grid's 2.5× loss** remains unexplained and unfixed.
- **Jolt parity is a projection.**
- **The cfg-A headline** pins AllPairs by definition; new rows must name their broadphase.

## Open questions
1. **Merge order with L2** (in the L5 lane). L2 recalibrates `GRID_LO` / `GRID_HI`; whichever lane merges second rebases, and Auto's high side must end as Tree. This is the orchestrator's call.
2. **C2 re-evaluation.** After L4, L5 and L3 land and T(8) is re-measured, re-run D6's arithmetic with G4's t_q. If it reaches ≥ 5 %, C2 is designed then: per-chunk streams with an overflow redo.

Sources:
- [Ski Rental Problem — Springer Encyclopedia of Algorithms](https://link.springer.com/rwe/10.1007/978-0-387-30162-4_378)
- [Ski rental problem — Wikipedia](https://en.wikipedia.org/wiki/Ski_rental_problem)