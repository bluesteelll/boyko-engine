# Architecture: L10 rev 2 — sleeping on by default, plus the frozen-pair skip

This is a read-only design against `D:/wt/joltab` @ `9bd100fb`, written against broadphase rev 2 (the Tree). "arith." means computed from P0 spans or counts, not measured.

- Rulings applied: `levers/00-RULINGS.md` (L10 B1, W1–W5, O1–O5; broadphase section) and `perf-campaign/00-RULINGS.md` (P0 protocol, SE claim gate).
- Research: `01-RESEARCH.md`. I also re-checked on the web how Box2D v3 exposes sleeping contacts:
  - `b2Body_GetContactData` walks the body's own contact list whatever its set.
  - `b2GetContactSim` resolves a contact by `(setIndex, colorIndex, localIndex)`: a handle into either the awake graph colours or a sleeping set.
  - D6 adopts that handle scheme.

## Changes from rev 1

| remark | resolution | where |
|---|---|---|
| **B1** — Sets drops sleeping contacts and islands from the public views | Logical views are served as **stream ⊎ store**, with no per-step copy: `ContactPairs` merges the stream and `withheld`; `Manifolds` merges the stream and the kept store through a sorted index touched only on transition steps. Island ids are made exact by **pre-rooting** held islands inside the graph build. `is_island_frozen` is exact with no extra work. `WarmSeedStats` adds the store's totals. Handles plus `position_of` recover Off's positions. | D6, D7, A7, E6 |
| **W1** — the colouring predicate and the write guard can drift | Both sites are fed by one function over one per-row flag, `effective_inv_mass(inv_mass, held)`, so they are identical by construction. Held rows therefore also drop out of gravity, integrate, refresh, capture and write-back. Mutation M14. | D8 |
| **W2** — runtime transitions undefined | A transition table. One L10 cursor. "Resting" requires that cursor, the IslandSleep mask cursor and the gather baseline stamp. Every toggle is in S3. | D9 |
| **W3** — mirror-set placement | Mirror sets run in the narrowphase after `begin_frame`. S3 gains a Rows-step grow-clear while an island is held. Mutation M10. | A3 |
| **W4** — kinds, SDF, coupling not gated | S1–S3 run × {Tree, Grid with parallel emit}, plus AllPairs on S1. New S4: SDF pile with a mid-run field edit. New S5: coupled soft body. | Gates |
| **W5** — census cannot fail on transitions | Census arm S8 (wake, re-freeze, churn, field edit, toggles after warm-up). Mutations in restore, move-in and the warm rehash. | Gates |
| O1 — Rows steps can be every frame | Jumper rows are the only rows that break order (D12). A held island touching one is restored. Every translation is then monotone, so there is never a sort. The Rows-step floor is stated. | D12, Costs |
| O2 — floors understated | Every figure is a range. Remainder taken as 0.064–0.134 ms. | Gains |
| O3 — C3a voids its own timing rows | The runner gets per-path expectations, including the no-awake fast path. | Runner |
| O4 — unconditional swap | The `bodies` swap happens only when a skip mode is active, and it is stamped. | D3 |
| O5 — two more mutations | M-R2 (latch in place of mask) and M-R4 (Reset treated as Identity). | Gates |
| review OQ1–OQ3 | Off writes no carry and never stamps, so switching into Replay/Sets gives one Reset step. A K=6 rehearsal runs at C2b. `SleepSkip::Off` stays the public oracle. | D9, Gates, API |
| Broadphase alignment | Rev 1's A2 contract ("emit pairs with ≥1 non-resting endpoint") is replaced: the Tree emits the exact set; Z = held rows; SL is withheld in `ContactPairs`; a release API. Broadphase C5 is absorbed into this lane as **C3c**. | D11 |
| Simplifications | Dropped: rev 1's AllPairs fresh-loop variant (the Tree is the default); the `pair_out` and `src` columns (replaced by a merge-join); `nm_pairs` (no-manifold pairs already live in stream ⊎ withheld). Tags are per pair, not per manifold. | D4, D5 |
| Rev 1 D10 reversed | Sets also runs on SDF worlds, using a field-bits epoch. | D10 |

## Changes broadphase rev 2 still needs (for its rev 3; C5 items are built here in C3c)

| # | change | why |
|---|---|---|
| T1 | **C5 hint = `HeldHint`.** `frozen(r)` reads L10's HELD flag in current rows. It replaces `!is_row_awake(prev_row)`. Dropped: `IslandSleep::mask_rows`, the coverage conjunct, M6 and M6b. | The held set is rebuilt from the current step, so a stale mask cannot occur. Bp review 2 W2(b) (toggle-off) becomes L10's S3 toggle script, with mutation M19 ("flags not cleared on flush"). |
| T2 | Under Sets, **S-fit also requires `anchor_ok(r) = RESTING ∧ !is_sensor`**; `NoHint` returns true. | Invariant V: every withheld pair has two resting, non-sensor endpoints. Without it, a static whose rotation or shape changes while (x, y, z, r) stays equal keeps stale withheld pairs (M15). A sensor pair must be recomputed every step. |
| T3 | **SL is stored as `ContactPairs::withheld: ScratchColumn<(BodyIndex, BodyIndex)>`**, strictly sorted, using the Tree cohort's `sl` id. **The C5 assembly never merges SL into `pairs`.** P_logical = `pairs.len() + withheld.len()` feeds `phys_bp_pairs`, the axis cache and the debug order assert (asserted over the merged view). | Ruling B1 binds C5's assembly; this answers bp review 2 OQ1. The tuple element type lets the logical view hand out `&(BodyIndex, BodyIndex)`. |
| T4 | **`BroadphaseTree::release(rows, &mut ContactPairs)`**: kill the Z leaves of released rows, filter `withheld`, and sorted-merge the removed entries into `pairs`, all in the same step. O(\|withheld\| + \|pairs\|), only on release steps. | D2 and D3 are decided after the Tree has already withheld those pairs. |
| T5 | **Jumper diversion (fixes bp review 2 W1).** The patch rule's keep test diverts every entry with an endpoint in `RowIdentity::stage2_rows()` (D12). Kept entries then translate monotonically and are sorted by construction; a = the number of entries touching jumpers. Add the G4 arm and the unit test the review asked for. **`stage2_rows` lands in the bp lane**; L10 depends on it. | A high jumper that is the min endpoint of 2 or more entries no longer cascades. |
| T6 | `withheld` is non-empty only on Tree-path steps. A kind switch or a brute-path step clears it. | Otherwise pairs would be emitted twice. |
| T7 | G2 J-Son/R-S: A = the first step with every dynamic row held, asserted ≤ 300 (bp review 2 W2(a)). A void is red (W2(c)). `translations` counts Rows steps with ≥ 1 member (O1). Harness counters are pinned structurally (O4). | These gates otherwise cannot fail. |

## Goal
- **Functional.** Two changes:
  - `PhysicsConfig::sleeping` defaults to `true` (owner-approved D3). This is the only value change.
  - Under `SleepSkip::Sets` (the default), a frozen, clean island is **held**: it pays no broadphase query, no narrowphase, no SDF stage, no graph colouring and no solve. Its contacts stay visible in every public view.
- **Exactness.** Replay and Sets reproduce `SleepSkip::Off` (today's sleeping-on path) bit for bit:
  - poses, velocities, latches, island keys and `contact_wakes`;
  - the **logical** `ContactPairs`, `Manifolds`, island ids and queries, `is_island_frozen` and `WarmSeedStats`;
  - the warm seeds of solved points;
  - axis-cache and warm-table **lookups** (table bytes are equal under Replay only).
- **Wake triggers** stay today's: A4, merge, `wake_all`, energy. The recorded gaps are kept, not fixed.
- **Targets (arith., W=1; ranges per O2):**

| row | Off (Tree bp) | C2b Replay | C3b Sets (Tree without Z) | C3c Sets + Z |
|---|---|---|---|---|
| J-Son tail [264, 1000) | 3.56–3.64 ms | 0.86–1.24 | 0.30–0.50 | **0.11–0.23** |
| R (sleeping on) | 4.28–4.36 | 1.18–1.64 | 0.33–0.55 | **0.12–0.25** |
| J-S0 (sleeping on, nothing freezes) | — | ≤ +0.3 % | ≤ +0.3 % | ≤ +0.3 % |
| J parity (sleeping off both sides) | — | 0 | 0 | 0 |
| a Rows step while all is held, J scale | ~3.6 | — | — | 0.10–0.20 |

- At W=8 the executor gap is g = 0.035 ms instead of 0.089, so the C3c floor is 0.08–0.18 ms. Against Jolt that is ≈50–105× (P0: 2,450×).
- **Allocations:** 0 heap per step after warm-up on every path. Columns commit pages; they do not call the heap.

## Context and constraints
- **Affected:**
  - `systems.rs`: gather `:210-275`, bp `:297-349`, np `:375-478`, SDF `:588-635`, graph `:1038-1057`, solve `:1097-1119`;
  - `resources.rs`: ContactPairs `:562-611`, Manifolds `:2269-2354`, graph `:2453-2993`, IslandSleep `:3108-3670`, SolverScratch `:3949`;
  - `solver/colored.rs` `:1587-1729`, `:3229-3319`, `:3412-3674`;
  - `solver/contact.rs:20-40`, `solver/simd.rs:192-252`, `solver/warm_start.rs:274-288`;
  - `narrowphase/axis_cache.rs:261-422`, `row_identity.rs`, `plugin.rs:496-633`, `scratch_ids.rs:583-739`, `broadphase_tree/*`.
- **Invariants kept:**
  - IM-1 (gather and apply walk every row);
  - the canonical warm store;
  - sorted `(min, max)` order;
  - the colouring invariant;
  - `{1, N}` bit identity;
  - principle 0: every new durable datum lives in a `ScratchColumn` owned by a resource.
- **Facts the design rests on (re-read):**
  - `ConstraintGraph` island ids = the rank of the union-find root among roots (`resources.rs:2756-2781`). Union-by-size ties go to the first endpoint's root (`:2720-2728`). So a component's root depends only on its own union sequence.
  - `begin_step` keys = the island CSR count (`:3531-3549`).
  - `awake = NO_ISLAND ∨ !frozen` (`:3560-3567`).
  - The integrate kernels are gated on `simulated ∧ is_dynamic_row(eff.inv_mass)` (`simd.rs:199, 247`); refresh on `eff.inv_mass != 0` (`:153`).
  - The axis cache clears on grow, or on `occupied > len/2` (`axis_cache.rs:261-281`), from a count it is handed.
  - The warm table's lookup values depend only on the key set (`warm_start.rs:219-226`).
  - `box_box_contact` returns `Some` only with a realized contact, except on a degenerate face (`box_box.rs:653-734`); INV-A7b below.
  - `SdfField` is `Copy` and can be replaced wholesale, so its `gen` is not a unique stamp (`sdf_query.rs:45-82`).

## Key decisions

**D1. Replay is a cache; recompute is the oracle (unchanged).**
- A pair is replayed only when today's recompute would read bit-identical inputs: both `BodyState`s, the pair roles, and the hint (E2).
- Rejected: Jolt's drop-and-cold-wake (changes values; 4.54e-2 sag); change ticks (miss raw writes and enable toggles).

**D2. "Resting" is decided at the step boundary from t−1's frozen decision (unchanged, plus W2 cursors).**
- Row r is resting at t iff all of:
  - **R1**: r has a previous row p.
  - **R2**: at t−1, p was immovable (`bodies_prev[p].inv_mass == 0`) or not awake (`!awake_rows[p]`).
  - **R3**: `BodyState(r)` is field-wise bit-equal to `bodies_prev[p]`. The compare destructures exhaustively, so a new field fails to compile.
  - **R4**: the L10 cursor, the mask cursor and the gather baseline stamp each classify as Identity or Rows.
  - r is not a jumper (D12) and not a degenerate box.
- Why the mask and not the latch: a row latched at `end_step(t−1)` was still integrated at t−1 (M-R2).

**D3. `SolverScratch::bodies_prev`, swapped in the gather only when a skip mode is active (O4).**
- The gather swaps iff `cfg.colored ∧ cfg.sleeping ∧ sleep_skip != Off`, and stamps `baseline_seq`.
- The broadphase trusts `bodies_prev` only if the stamp equals the current gather. A user system that changes the mode between gather and broadphase therefore yields a Reset, not a wrong baseline.

**D4. One-step stream carry, joined by key, not by index.**
- Carried: `pairs_prev` (ContactPairs); `manifolds_prev`, `sensor_prev`, `pair_tag`, `pair_tag_prev` (Manifolds).
- A pair's replay source is found by a monotone merge-join of the current stream against `pairs_prev` / `manifolds_prev` (translated through `inv` on Rows steps, jumper entries skipped).
- Per-pair tag, `u8`: `axis:4 | SETTLED:1 | BOX:1 | SET:1 | PUSHED:1`.
- Rejected: rev 1's `pair_out` and `src` columns. They cost 2 ids and 4 B per pair, and still needed a merge.

**D5. The held store: copied once at move-in, rows island-local, owned by the resource whose view reports it.**
- `Manifolds` owns the records, row tables, kept manifolds (`body_a`/`body_b` are local indices into the island's row table) and the kept order index.
- `SleepSets` owns the kept warm entries.
- **Why island-local:** a Rows step translates only the row tables (4 B per held row or anchor). Rewriting kept rows would cost one cache line per manifold (≈1.5–3 ms at 100k).
- Rejected: Box3D-style indices into the stream (they dangle every step); global rows (the Rows-step tax above).

**D6. Logical views = stream ⊎ store; random access by handle (B1).**
- `ContactPairs::pairs()` → `PairsView`: stream ⊎ `withheld`.
- `Manifolds::manifolds()` → `ManifoldsView`: stream ⊎ kept, merged in canonical ordinal order. Body-body key `(a<<32)|b`; SDF key `(1<<63)|a`, which reproduces Off's "body-body, then SDF by row".
- Handle `h < HELD_BASE (2^31)` is a stream index; `h ≥ HELD_BASE` is kept slot `h − HELD_BASE`. `Manifolds::get(h)` resolves either. `position_of(h)` returns Off's position in O(log n). Handles are valid until the next step, as positions are today.
- **Cost:** 0 per step. The kept order index (4 B per kept manifold) is merged or filtered only on move-in and restore steps. Rows steps leave it sorted (D12).
- **Rejected:**
  - a per-step logical index (O(store) every step, which the ruling forbids);
  - a k-way merge on read (needs a heap allocation on the read path, and O(M log K));
  - one globally sorted kept column (memmoves 152 B manifolds per transition).
- **Trade-off:** `ManifoldsView` yields `Manifold` by value (rows reconstructed). There is no `Index` impl, so sites that mixed index spaces fail to compile rather than silently read the wrong manifold.

**D7. Island queries exact by pre-rooting held islands in the graph build.**
- `reset_islands` writes `parent[m] = root(record)` for held members; they are in no stream manifold, so no union touches them.
- `flatten` assigns ids in ascending root order over the **membership predicate** `is_dynamic_row(bodies[r].inv_mass)` (today's), which includes held rows.
- **Result, at the O(N) cost the build already pays:**
  - `n_islands`, `island_of` and ids equal Off's (a held island's root is Off's root: its union sequence is unchanged up to monotone renaming, E4);
  - `island_len(i)` = CSR count + `held_len[i]`;
  - `max_island_constraints` = max over `island_len`;
  - `island(i)` returns handles;
  - `begin_step` computes `frozen_islands` for held ids through the unchanged per-row fold (their latches are asleep), so `is_island_frozen` is Off's.
- Colours are the solver partition of `solver_manifolds()`: not an island query; documented.

**D8. The effective inverse mass (W1).**
- `pub(crate) fn effective_inv_mass(inv_mass: f32, held: bool) -> f32 { if held { 0.0 } else { inv_mass } }`, beside `is_dynamic_row` in `contact.rs`.
- The graph's colouring, union and filing predicate is `movable(r) = is_dynamic_row(effective_inv_mass(bodies[r].inv_mass, HELD[r]))`.
- `build_bodies` writes `eff.inv_mass = effective_inv_mass(...)` from the same flag column in the same step. The write guard, gravity, integrate, refresh, capture and `write_back` all read `eff.inv_mass`.
- **Same function, same flags**: identical by construction, so even a manifold naming a held row could not produce concurrent writes (only a value divergence, which the gates see).
- Held rows are also inert in the substep kernels, so the O8 capture and restore skip them.

**D9. Dirty, flush and transition rules (W2).**
- Each step L10 runs, it stamps **one** cursor; its baseline, carry and store are all keyed by that gather.

| event | detected in | action |
|---|---|---|
| D1: member not resting, vanished, or a jumper | broadphase prologue | restore the island |
| D1a: anchor (non-member manifold partner) not resting, vanished, or a jumper | prologue, per record | restore |
| D8: a member's latch not asleep after `end_step(t−1)` (covers `sleep_threshold` / `sleep_frames` changes exactly) | prologue | restore |
| D2: a stream pair joins a member to a non-resting row, pair not sensor | epilogue scan | restore + Tree `release` |
| D3: Identity step, `would_clear(P_logical)`, island keeps ≥ 1 box-box manifold | epilogue | restore + release |
| D5: epoch change (`sleep_skip`, `warm_start_enabled`, `sdf_narrowphase`, SDF edit bits) | prologue | flush all |
| D6: `wake_all` pending | prologue | flush all |
| D7: L10 cursor / mask cursor Reset, or baseline stamp stale | prologue | flush all; nothing resting this step |
| `sleeping` on→off | prologue (runs whenever the store is non-empty) | flush all; `restore_warm` still inserted; sleeping-off solve |
| `sleeping` off→on; Off→Replay/Sets | cursor stale | one Reset step, then normal |
| Sets→Replay / Off | prologue | flush all |
| Replay→Sets | — | move-ins start |
| kind switch / brute crossing | broadphase | `withheld` cleared; held unchanged (narrowphase skip) |

- **Flush** = restore every record this step.
- Each stage reads the mode that the broadphase recorded in `SleepSets::step_mode`, never `cfg`, so a mid-schedule config write cannot split a step.

**D10. Sets on SDF worlds.**
- The SDF stage skips held rows. Their SDF manifolds are kept.
- The epoch holds a copy of the live edit list (`[SdfEdit; MAX_SDF_EDITS]` + count, inline in `SleepSets`), compared field-wise by `to_bits` each step (~1 KB, ≈0.05 µs), plus the kernel choice.
- **Why:** SDF worlds are the owner's main target. Replay as their default would copy every sleeping manifold every step. `gen` cannot be trusted across a wholesale `*field = …`.

**D11. The broadphase seam.**
- Sets works under every kind: the narrowphase skip keys off `HELD`, not off the broadphase.
- Only the Tree also saves the broadphase: Z = held rows, and SL is withheld (T1–T6).
- Invariant V: withheld pairs have both endpoints resting, non-sensor, and at least one held.

**D12. Jumper rows.**
- `RowIdentity::stage2_rows()` lists (ascending) the rows resolved by stage 2 (`row_identity.rs:627-641`).
- Rows resolved by the aligned walk (E3) have strictly increasing previous rows (`:574-581`). So a translation restricted to non-jumper rows is monotone, and any list whose entries avoid jumpers stays sorted.
- L10 restores every held island touching a jumper (members or anchors) and skips jumper entries in the carry.
- Cost-only: budget exhaustion (`:38-42`) only enlarges the jumper set.

**D13. Wake semantics are unchanged; the at-rest velocity is kept** (rev 1 D8 and D9). Write-wake and support-destroy-wake are value changes and go to Open questions.

**D14. C0: `IslandSleep`'s four `Vec`s become two columns.**
- `latch: ScratchColumn<SleepLatch>` (asleep, below_count, island_key; it takes over the key's id and slot). The existing permute carry already uses this type, and `begin_step` reads one 8 B element per row instead of three arrays.
- `island_scratch: ScratchColumn<IslandScratch { energy: f32, frozen: u8, _p: [u8; 3] }>`.

**D15. Coloured variants.**
- `physics_broadphase_colored` and `physics_narrowphase_colored` are registered only where `IslandSleep` exists.
- The reference path's systems stay byte-untouched.
- The Off arm is today's loop, selected by a const generic hoisted out of the loop.

## Data structures

```rust
// resources.rs — PhysicsConfig (+1)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SleepSkip { Off, Replay, #[default] Sets } // every mode yields Off's observables

// SolverScratch (+1)
pub(crate) bodies_prev: ScratchColumn<BodyState>, // post-solve t-1; swapped at gather (D3)
pub(crate) baseline_seq: u64,

// ContactPairs (+2)
pub(crate) pairs_prev: ScratchColumn<(BodyIndex, BodyIndex)>, // stream of t-1, swapped at bp start
pub(crate) withheld: ScratchColumn<(BodyIndex, BodyIndex)>,   // Tree SL (T3); strictly sorted

// Manifolds (+10)
pub(crate) manifolds_prev: ScratchColumn<Manifold>,
pub(crate) sensor_prev: ScratchColumn<Manifold>,
pub(crate) pair_tag: ScratchColumn<u8>,            // per stream pair (D4)
pub(crate) pair_tag_prev: ScratchColumn<u8>,
pub(crate) held: HeldStore,

pub(crate) struct HeldStore {                      // cold: touched on transitions / Rows / restore
    records: ScratchColumn<HeldIsland>,            // 48 B each
    rows: ScratchColumn<u32>,                      // per record: members ascending, then anchors ascending (current rows)
    kept: ScratchColumn<KeptManifold>,             // 156 B; contiguous run per record, sorted by ordinal
    of_row: ScratchColumn<u32>,                    // row -> record | NONE (permuted on Rows steps)
    order: [ScratchColumn<u32>; 2], cur: u8,       // kept slots sorted by ordinal; ping-pong for merges
    live_kept: u32, live_points: u32, dead: u32,   // O(1) len / carry_points / compaction trigger
}
#[repr(C)] struct HeldIsland { root: u32, rows_start: u32, n_members: u32, n_anchors: u32,
    kept_start: u32, kept_len: u32, box_kept: u32, points: u32, flags: u32, _p: [u32; 3] } // 48 B
#[repr(C)] struct KeptManifold { m: Manifold /* body_a/b = local idx; SDF_SENTINEL kept */, tag: u8, _p: [u8; 3] }

// ConstraintGraph (+1)
island_info: ScratchColumn<IslandInfo>,            // per island id, written by flatten
#[repr(C)] struct IslandInfo { root: u32, members: u32, held: u32 /* record | NONE */, held_len: u32 } // 16 B

// IslandSleep (C0 + 1): latch, island_scratch (D14); mask_cursor: RemapCursor (stamped at end of begin_step)

// sleep_sets.rs (new resource, coloured path only)
#[derive(Resource)] pub struct SleepSets {
    row_cls: ScratchColumn<RowCls>,        // HOT: 8 B/row, read by bp/np/SDF/graph/solve (10 KB at J, L1)
    inv: ScratchColumn<u32>,               // prev row -> cur row (Rows steps)
    cand: ScratchColumn<u32>,              // per t-1 island: ok-member count | flags (candidate steps)
    kept_warm: ScratchColumn<KeptWarm>,    // 24 B, per record contiguous
    warm_of: ScratchColumn<(u32, u32)>,    // per record id: kept_warm range
    restore_warm: ScratchColumn<WarmEntry>,// this step's restored entries, keyed in t-1 rows
    events: ScratchColumn<u32>,            // this step's restored / moved-in records (packed kind)
    scratch: ScratchColumn<u64>,           // move-in ordinal sort, D2 hit list
    cursor: RemapCursor, epoch: SleepEpoch, step_mode: SleepSkip, flags_gen: u64,
    held_warm: u32,                        // Σ kept warm entries (logical carry_hits)
    stats: SleepSkipStats,
}
#[repr(C)] struct RowCls { island: u32 /* held record | t-1 candidate island | NONE */,
    flags: u32 /* RESTING HELD PRE_HELD JUMPER CAND SENSOR LATCHED */ } // 8 B
#[repr(C)] struct KeptWarm { a: u32, b: u32 /* local or SENTINEL */, feature: u32, n: f32, t: [f32; 2] } // 24 B
#[repr(C)] struct SleepEpoch { mode: u8, warm: u8, sdf_kernel: u8, _p: u8, sdf_len: u32,
    sdf_edits: [SdfEdit; MAX_SDF_EDITS] }       // inline, compared by to_bits
#[repr(C)] #[derive(Clone, Copy, Debug, Default)]
pub struct SleepSkipStats { held_islands: u32, held_rows: u32, held_manifolds: u32,
    withheld_pairs: u32, held_skipped_pairs: u32, replayed: u32, computed: u32,
    restored: u32, moved_in: u32, released_rows: u32, flushes: u32, fast_path: u32 } // 48 B

// RowIdentity (+1, lands in the bp lane, T5): stage2_rows: ScratchColumn<u32>
```

- **Hot/cold split:**
  - Hot, every step: `row_cls`, the stream, the carry buffers.
  - Cold: `HeldStore`, `kept_warm`, `order` (transition, Rows and restore steps only).
  - Read path only: view iteration.
- **Memory at J, all held (arith.):**
  - kept 0.70 MB; kept warm ≈0.39 MB;
  - carry buffers 0.70 MB; `bodies_prev` 0.2 MB;
  - order 2 × 18 KB; `row_cls` 10 KB.
  - Reservations are address space.
- **Scratch ids:**
  - 23 new; `withheld` reuses the Tree's `sl` id; `latch` reuses `SLEEP_ISLAND_KEY`.
  - A new `SLEEP_SETS` cohort goes below the Tree cohort (bottom 389), placed with `highest_id_clear_of` clear of slots {63 BODY_STATE, 62 CONTACT_PAIRS, 27 TOUCHED_AWAKE, 19 ROW_PREV, the Tree's 15..5, the graph and narrowphase cohorts}. Width ≤ 64. Every co-swept pair is asserted, as at `scratch_ids.rs:666-688`.
  - Only 5 ids remain above `SCRATCH_REGION_MIN_ID`. It moves from `MAX−128` to **`MAX−160` (352)** after re-running the component census its own docs require (`:715-729`; 142 production ids measured 2026-09-09 → 2.5× margin). The floor assert (`:734-739`) moves to the new cohort bottom.
- **Drop:** no `Drop` logic.
- **Threading:** no field is shared across threads.

## Public API

```rust
pub struct PhysicsConfig { /* … */ pub sleeping: bool /* default true */, pub sleep_skip: SleepSkip }

impl ContactPairs { pub fn pairs(&self) -> PairsView<'_>; }             // logical
pub struct PairsView<'a>;  // len O(1), is_empty, iter -> &'a (BodyIndex,BodyIndex) (merge),
                           // IntoIterator (by value and &), get(k) O(log n), contains(a,b), to_vec
impl Manifolds {
    pub fn manifolds(&self) -> ManifoldsView<'_>;       // logical, Off order
    pub fn solver_manifolds(&self) -> &[Manifold];      // the stream colours/solve use
    pub fn sensor_overlaps(&self) -> &[Manifold];       // unchanged: sensor pairs are never held
    pub fn get(&self, handle: u32) -> Option<Manifold>; // stream index or HELD_BASE|slot
    pub fn position_of(&self, handle: u32) -> Option<usize>; // Off's position, O(log n)
}
pub const HELD_BASE: u32 = 1 << 31;
pub struct ManifoldsView<'a>; // len O(1), is_empty, iter -> Manifold (by value), IntoIterator,
                              // get(pos) O(log n), to_vec
impl ConstraintGraph {        // n_islands, island_of, max_island_constraints: logical (D7)
    pub fn island(&self, i: u32) -> IslandManifolds<'_>; // handles; len O(1), iter -> u32, to_vec
    pub fn island_len(&self, i: u32) -> u32;
    pub fn n_colors(&self) -> u32; pub fn color(&self, c: u32) -> &[u32]; // solver partition of solver_manifolds()
}
impl IslandSleep { pub fn is_island_frozen(&self, island: u32) -> bool; } // logical ids, unchanged signature
impl ColoredSoftStepSolver { pub fn warm_seed_stats(&self) -> WarmSeedStats; } // carry fields logical
impl SleepSets { pub fn stats(&self) -> SleepSkipStats; pub fn is_row_held(&self, row: usize) -> bool; }
// pub(crate): effective_inv_mass, ConstraintGraph::build_with_held, BroadphaseTree::release,
// RowIdentity::stage2_rows, RemapCursor::peek, BoxAxisCache::{would_clear, mirror}
```

- `ConstraintGraph::build(manifolds, n, is_dynamic)` (direct drive) and `solve_colored_sleeping` keep their signatures and Off semantics.
- `SleepSkip::Off` is the public oracle setting.

## Algorithms for the critical paths

Stage order: gather → select → **bp [A1 prologue → kind arm → A2 epilogue]** → **np (A3)** → SDF (A4) → graph (A5) → solve (A6) → apply.

| # | step | complexity | cache / branching / SIMD |
|---|---|---|---|
| A1.0 | Mode: `m = if !cfg.sleeping {Off} else {cfg.sleep_skip}`. If `m != Sets` and the store is non-empty, flush. If `m == Off`, return without stamping. | O(1) | — |
| A1.1 | Cursors (L10, mask via `peek`, baseline stamp). Reset or stale → flush, and nothing is resting this step. | O(1) | — |
| A1.2 | Rows step: build `inv` (O(N_prev)); translate record rows and roots through `inv`; permute `of_row` (O(N)); set JUMPER from `stage2_rows` (O(#stage2)). | O(N + held rows) | sequential, cold |
| A1.3 | **Per-row pass.** p = prev(r). RESTING = R1–R4 (R2 first: a mask/`inv_mass` test; R3 only if R2 holds). Also PRE_HELD (from `of_row`), CAND (graph(t−1) frozen island of p, not held), LATCHED (`latch[p].asleep`), SENSOR. | O(N); R3 ≈ 2×160 B per candidate row | 2 lines of `BodyState` + 2 of `bodies_prev` per candidate; one predictable branch; R3 uses AVX2 over the f32 prefix plus a scalar tail with exhaustive destructure |
| A1.4 | Per record: D1/D1a/D8 over its row table (anchors through RESTING/JUMPER/vanished), then D5/D6 → restore list. | O(K + Σ rows) | cold unless something changed |
| kind | Tree with `HeldHint { frozen: PRE_HELD, anchor_ok: RESTING ∧ !SENSOR }`; Grid (serial or parallel emit); AllPairs; brute. | bp rev 2 | — |
| A2.1 | P_logical = \|stream\| + \|withheld\|. If Sets ∧ Identity ∧ `would_clear(P_logical)` → D3. | O(1) + O(K) | — |
| A2.2 | **D2 scan** (skipped when there is no PRE_HELD and no CAND row): per stream pair, `(f_a\|f_b)` tests; a non-sensor pair with a held/candidate endpoint and a non-resting other endpoint marks that island. | O(\|stream\|), 1–2 ns/pair | 8 B/row lookups in L1 |
| A2.3 | Release (Tree only, if D2/D3 hit PRE_HELD rows): `tree.release(...)` (T4). | O(\|withheld\| + \|stream\|) | cold |
| A2.4 | **Move-in**: a CAND island moves in iff M1 (every member present, resting, latched, non-jumper, non-sensor, non-degenerate: ok-count == `island_info.members`), M2 (every manifold partner resting, non-jumper), M3 (each of its box pairs at t−1 has `SET ⇒ PUSHED` and `PUSHED ⇒ SETTLED`), M4 (no D2/D3/D5–D7). Capture: row table, kept = its manifolds from the **t−1 stream** (`graph(t−1).island(a)` indices; the narrowphase has not swapped yet) translated to local indices, tags, root = `island_info[a].root` translated. Merge the new sorted ordinals into `order`. | O(island) + O(\|order\| + n log n) | cold (freeze and wake steps) |
| A2.5 | Restores: tombstone records; filter `order`; build `restore_warm` with t−1 keys (`prev_row` of the row table, non-jumper by D1); compact when `dead ≥ max(live, 64)`. HELD = PRE_HELD − restored + moved-in; bump `flags_gen`; stamp; stats. | O(\|order\|) on those steps | cold |
| A3 | **Narrowphase.** Swap the carry buffers. `begin_frame_synced(stream, P_logical) → (prefetched, cleared)`. Sets: **after** `begin_frame`, mirror held box keys (all of them if cleared or grown; on a plain Rows step only keys whose rows moved). Per pair, by const-generic arm: **Off** = today's loop. **Replay/Sets**: (i) HELD endpoint and not a sensor pair → skip; (ii) both RESTING: source = the kept run (endpoint in a record restored this step; binary search by ordinal), else the merge-join over `pairs_prev` / `manifolds_prev` / `sensor_prev`; replay a manifold, or "none" when the pair was visited without one; a box pair also needs SETTLED ∧ ¬(cleared ∧ ¬prefetched), with the mirror set on prefetched steps; (iii) otherwise compute (today's code) and write the tag. | O(\|stream\|) | replay copies 152 B sequentially; merge cursors are monotone; no non-temporal stores (the graph reads the result at once) |
| A4 | SDF stage: `continue` on HELD rows. No SDF replay (not worth a second kernel). | O(N) | — |
| A5 | `build_with_held`: reset writes `parent = root` for held members; unions, filing and colouring use `movable` (D8); flatten over membership; `island_info` records root, members, held, held_len; max = max(`island_len`). | today's O(N + M_stream) | — |
| A6 | **Solve:** `rekey_rows`; `begin_step` (keys via `island_len`); `build_bodies` with `effective_inv_mass`; insert `restore_warm` into `warm_read` — if 2·(len + new) > slots, rehash into the idle `warm_write` buffer and swap (no heap, exact lookups); `build_columns`; capture only rows with `!awake ∧ is_dynamic_row(eff.inv_mass)`; no-awake fast path (C3a); store and carry (stream frozen); **before the swap**, move-in warm capture: each moved-in record's kept points looked up in `warm_read` through `warm_remap` exactly as `carry_frozen` does (`colored.rs:3282-3319`); hits → `kept_warm`. `warm_stats.carry_points += live_points` (if warm is enabled); `carry_hits += held_warm`. | O(stream) + O(N) | — |
| A7 | **Views (read path):** `PairsView` = two-pointer merge; `ManifoldsView` = merge of the stream and `order` (ordinal from 2 row-table loads); `get` = k-th of the union by dual binary search. | O(P) or O(M) iteration, O(log) get | no allocation (only `to_vec` allocates) |

**Steady all-held step at J (arith.):**

| part | cost (ms) |
|---|---|
| A1.3 (0.4 MB compare) | 0.010–0.025 |
| Tree verify | 0.003–0.005 |
| A2 / A3 / A4 over an empty stream | ~0 |
| graph O(N) | 0.010–0.020 |
| solve O(N) | 0.015–0.030 |
| remainder | 0.064–0.134 |
| **total** | **0.11–0.23** |

**Rows step while all held (J):**

| part | cost (ms) |
|---|---|
| A1.2 | 0.005 |
| mirrors, 4.5k box keys × 15–30 ns | 0.07–0.14 |
| Tree SL translation (bp rev 2) | 0.02–0.03 |
| **total** | **≈0.10–0.20**, against Off's ≈3.6 |

- **Move-in of the whole J pile:** 0.2–0.35 ms, once.
- **Restore of an island with m manifolds:** m × 30–60 ns, plus the `order` filter.

## Exactness (determinism) argument

- **E1. Pair set.** Every kind emits the exact set as stream ⊎ `withheld` (bp D3.1). `release` moves entries without changing the set.

- **E2. Replay = recompute.** Both `BodyState`s are bit-equal to what t−1's narrowphase read (R2, R3). Roles are kept: jumper entries and order-flipped pairs are computed. The hint is the same:
  - Identity without a clear: only this pair writes its key, and a settled replay writes the same value;
  - Rows: the prefetch reads the old key before any write or clear;
  - Identity with a clear: computed.

  SETTLED means the same inputs chose the same axis, and `box_box_contact` is pure. "Visited, no manifold" is pose-only for non-degenerate shapes (A7b), and degenerate boxes never rest. A kept-store source is the same manifold captured from a stream step whose inputs are unchanged (E4).

- **E3. Downstream.** Graph, solve and store read only the stream, the bodies and the tables, so the induction over steps holds.

- **E4. A held island is a fixed point of Off's per-step map:**
  - members and anchors unchanged (R3, D1, D1a);
  - no fresh non-sensor overlap (D2);
  - kept manifolds equal Off's recompute (E2 at capture, D3 for hints; D12 keeps the order and the Off root, since a monotone renaming of an identical union sequence with the first-endpoint tie rule yields the renamed root);
  - its key equals its count;
  - latches asleep (D8) with constant energy;
  - epoch unchanged (D5).

  Off's work on it (recompute, frozen, integrate+restore, carry, `end_step`) therefore changes nothing Sets does not reproduce.

- **E5. Shared state:**
  - Colours and in-colour order of stream manifolds are unchanged: occupancy is per movable body, islands are body-disjoint, and no stream manifold names a held row, because A3 and A4 are the only entry points and both skip.
  - Axis table: same key set, values and `occupied`, by induction. The mirror covers every step on which Off inserts a new held key (Rows or clear/grow), and `begin_frame` receives P_logical. Layout differs; lookups do not.
  - Warm table: held keys are disjoint by row from stream keys (anchor keys differ in the partner row). Held entries are re-inserted with t−1 keys before any read of that step (restore). Lookups depend only on the key set.

- **E6. Logical views = Off's:**
  - pairs by E1;
  - manifolds = stream ⊎ kept, ordered by the same ordinal Off's stream has;
  - island ids and counts by D7;
  - `is_island_frozen` by the unchanged fold;
  - `carry_points` / `carry_hits` = stream + store totals, since held hits are constant (Off re-carries the same keys each step);
  - `island(i)` as a set equals Off's, and through `position_of` equals Off's index list.

- **E7. Transitions:** every row of D9 lands in a state whose next step is Off's (flush = restore = the stream path, with `restore_warm`).

- **E8. W invariance:** all L10 code is serial, so the `{1, N}` identity carries over. Replay and Sets are machine-independent: integer merges and keys, copies, `to_bits` compares.

- **INV-A7b** (`box_box` `Some ⇒ count ≥ 1` for non-degenerate boxes): debug-asserted in the narrowphase. Exactness does not rest on it, because M3 refuses a move-in if a tag shows `SET ∧ ¬PUSHED`.

## Multithreading model
- All L10 code runs serially inside chained stages. No new atomics or locks. No field is written by two threads.
- New access sets:
  - bp: `ResMut<ContactPairs, Manifolds, BroadphaseTree, SleepSets>`, `Res<ConstraintGraph, IslandSleep>`, `Option<Res<SdfField>>`;
  - np: `Res<SleepSets>`;
  - SDF and graph: `Res<SleepSets>`;
  - solve: `ResMut<SleepSets>`.
- The stages are already chained (`plugin.rs:629-633`). A user system that reads `Manifolds` now also conflicts with the bp stage.
- Data-race freedom follows from the executor's access sets and from D8: the colouring and the write guard read one derived value.

## Expected gain (from P0 spans; arith.)

| row, W | Off (Tree) | Sets (C3c) | Δ (ms) | SE bar (K=6) |
|---|---|---|---|---|
| J-Son tail, 1 | 3.56–3.64 | 0.11–0.23 | 3.33–3.53 | ≈2 % |
| J-Son tail, 8 | 3.63–3.71 | 0.08–0.18 | 3.45–3.63 | 0.75 % |
| R, 1 | 4.28–4.36 | 0.12–0.25 | 4.03–4.24 | 3.3 % |
| R, 8 | 4.34–4.42 | 0.09–0.20 | 4.14–4.33 | 1.3 % |

- Against P0's R (sleeping off, 14.226 ms): the C1b flip alone gives 6.049 (P0 F).
- Realized-gain gate: ≥ 0.6 × the **lower** predicted Δ.
- If L5 has landed, Off's np at W=8 shrinks. The gate is in-binary Off against Sets, so that is measured, not assumed.

## Integration: commit sequence, each commit green

| commit | content and sites | value change |
|---|---|---|
| **C0** | D14: `resources.rs:3108-3200`, `:3288-3292`, `:3346-3382`, `:3480-3568`, `:3615-3670`; `scratch_ids.rs` (latch layout, +1 id) | none |
| **C1a** | New `tests/sdf_sleep_settles_and_wakes.rs` and `tests/default_world_sleep_worker_invariance.rs`. Runner (`benches/jolt_parity_pyramid.rs:132-134, 405, 465-506, 565-578, 735-743, 1118, 1156-1178`): `--sleeping on\|off`, `--sleep-skip off\|replay\|sets`, `--expect-pose`. | none |
| **C1b** | `resources.rs:504` → `true`; docs `:328-330`, `:3463-3475`; `plugin.rs:347-349`, `:405-407`, `:541-549`; explicit `sleeping: false` in reds (rule below) | **only value mover** |
| **C2a** | Plumbing, default `sleep_skip = Off`: `bodies_prev` and stamp (`systems.rs:248-275`); L10 cursor, `peek` (`row_identity.rs:222-245`); `SleepSets` (flags only); carry buffers and tags (written only in Replay/Sets); **the view types with an empty store, so every accessor edit lands here under Off semantics**; `effective_inv_mass` (held always false); coloured bp/np variants (`plugin.rs:629-633`); `+1 span PHYS_SLEEP_CLASSIFY`, `+2 counters PHYS_SLEEP_HELD, PHYS_NP_REPLAYED` (`profiling.rs:77-131`; the typed arrays make a count conflict with L5/bp a compile error) | none |
| **C2b** | Replay (A3 Replay arm; default Replay): `systems.rs:375-478`, `axis_cache.rs:261-281` / `:367-388` (`cleared`, P_logical) | none |
| **C3a** | Solver floor: need-sized `rebuild` + `len` + idle-buffer rehash (`warm_start.rs:274-288`); no-awake fast path (`colored.rs:3487-3503`, `:3547-3652`); runner per-path expectations | none |
| **C3b** | Sets (default Sets): `HeldStore`, A1/A2 without the Tree seam (the Tree runs `NoHint`), A3 held-skip and mirrors, A4, A5 (`resources.rs:2655-2868`), A6 (`colored.rs:1587-1729`, `:3229-3319`, `:3412-3674`), D8 (`contact.rs:20-40`), D10, logical views live | none |
| **C3c** | Tree sleeper set (bp C5 as amended T1–T4, T6): `broadphase_tree/*`, `ContactPairs::withheld` | none |
| receipts | MEASUREMENT-QUEUE result blocks (tester/analyst) | — |

**Before C0:** on the lane base (bp C4 and L5 merged), record the pose hashes for R-S and J-Son. They must equal P0's (`p0b/raw/window/runs.jsonl`), since the earlier levers are bit-identical. Otherwise the base's hashes become the fixtures and the difference is escalated.

## Gates (each shown able to fail)

**Rules:**
- Every named mutation is recorded red before its commit lands.
- A mutation that turns out green gets a stronger scene, or it is replaced and the reason recorded.
- Voids are red.
- Timing follows the P0 protocol: the median of window means over K processes, receipts per process, the canary seen, order interleaved, claim iff |effect| > 2·√(SE_A² + SE_B²).

**Per commit:**
- **C0:**
  - The sleep suites stay green with identical freeze steps and drifts.
  - M0 (energy max → sum) turns `sleeping_pipeline_o8` red.
  - New census arm: sleeping-on default world, 0 heap allocations per step after warm-up. Mutation: `Vec::with_capacity(1)` in `begin_step`.
- **C1a:**
  - The SDF gate: freeze step printed; rest pose within the box-pile ε of the sleeping-off twin; a field edit wakes the pile on the next step. Mutation: SDF manifolds filed under no island.
  - W invariance, 400 steps, W ∈ {1, 2, 4, 8, 16}. Red-first: `sleep_frames = u16::MAX` must fail "a frozen step was observed".
- **C1b:**
  - Full `cargo test --workspace --all-targets --no-fail-fast`; the red set is the list of moves.
  - R's hash equals P0's R-S hash at W ∈ {1, 8}; J's hash is unchanged.
  - In-binary R-off against R at W ∈ {1, 8}, K=6: a consistency check, not a claim.
  - J parent against commit, K=12: not claimed slower.
- **C2a:**
  - Every runner row's pose hash equals C1b's at W ∈ {1, 8}.
  - Every accessor-edited test is green with its assertion lines byte-unchanged; the diff is reviewed for that.
  - J and J-S0 at K=12: not claimed slower.

**`tests/sleep_skip_bit_identity.rs` (C2b, then C3b and C3c).** Off against Replay, and Off against Sets, compared on every step.

- **What is compared:**
  - `BodyState` bits; latch, below count and key per row; `contact_wakes`;
  - `pairs().to_vec()`, `manifolds().to_vec()` (bytes), `sensor_overlaps()`;
  - `n_islands`, `island_of` per row, `island_len`, `island(i)` resolved by value **and** through `position_of` (must equal Off's `island(i)`), `max_island_constraints`, `is_island_frozen` per id;
  - `WarmSeedStats`; the seed triple of each solved point in canonical order;
  - axis lookups for every logical box pair; warm lookups for every logical point key;
  - table bytes under Replay only.
- **Scenes:**

| scene | content | variants |
|---|---|---|
| S1 | rest pile, 400 steps | Tree, Grid (`parallel_broadphase`), AllPairs |
| S2 | J, 1000 steps (300 in debug) | Tree, Grid-parallel |
| S3 | adversarial: spawn/despawn, including order-reversing moves and component insert/remove; **high and low jumpers of a held member and of a static anchor**; a projectile merging into a held pile (D2 plus release); member teleport and velocity write; floor teleport; **static rotation with (x, y, z, r) unchanged**; kinematic platform under a pile; sensor volume over a held pile; `wake_all`; `sleep_threshold` change mid-hold; `warm_start` toggle; **`sleeping` off→on→off; mode Sets→Replay→Off→Sets**; kind Tree↔Grid; a forced Identity-step clear; **a Rows-step grow-clear while held**; a degenerate box; a parked member; a missed gather | Tree with `brute_max_rows = 0`, Tree default, Grid-parallel |
| S4 | SDF pile on an SDF floor, held, then a mid-run edit, then a kernel toggle | Tree |
| S5 | `add_physics_soft(.., true)` (Grid forced): soft body on a held pile; reactions land | Grid |

- W ∈ {1, 8}; {2, 4, 16} in release.
- **Anti-vacuity:**
  - Sets: held rows == dynamic rows on ≥ 90 % of S1's frozen steps;
  - Tree arms: withheld > 0 after admission;
  - Replay: replayed > 0 on ≥ 90 % of frozen steps;
  - S3 observes every D-rule, move-in, release, and each flush kind (per-rule counters in `stats`); mirrors > 0 on a held Rows step; the grow-clear observed;
  - S4: the edit restores ≥ 1 island and the pile wakes;
  - S5: a reaction restores ≥ 1 island (D1).
- **Named mutations (each red):**

| id | mutation |
|---|---|
| M1 | R3 off |
| M2 | no mirror set on Rows steps (Replay) |
| M3 | no clear invalidation |
| M4 | reversed pairs replayed |
| M5 | SETTLED ignored |
| M6 | jumper-endpoint carry entries replayed |
| M7 | D2 off |
| M8 | D8 off |
| M9 | M3 (SETTLED/PUSHED) off |
| M10 | mirrors placed before `begin_frame` (W3) |
| M11 | no `restore_warm` (`frozen_island_warm_start` wake-exact goes red) |
| M12 | `begin_step` reads the CSR count only (spurious A4) |
| M13 | narrowphase does not skip held pairs |
| **M14** | `build_bodies` ignores `held` (W1) |
| M15 | Tree S-fit ignores RESTING (static-rotation case) |
| M16 | `release` skipped |
| M17 | jumpers do not restore |
| M18 | SDF epoch ignores the edit bits |
| M19 | flags not cleared on flush |
| M20 | `restore_warm` keyed in t rows |
| M-R2 | R2 replaced by the latch |
| M-R4 | Reset treated as Identity |
| M-V1 | `pairs()` returns the stream only |
| M-V2 | `manifolds()` returns the stream only |
| M-V3 | `n_islands` counts awake islands only |
| M-V4 | `carry_points` is the stream only |

- Also red: M-V1 and M-V2 in `sleep_settles_box_piles`; M-V2 in `support_loss_wakes_sleepers`; M-V4 in `frozen_island_warm_start`.
- **Also run under Sets with identical outputs and unchanged assertions:** `support_loss_wakes_sleepers`, `sleep_settles_box_piles` (G5/G6 P1/P2 and `islands_after`), `frozen_island_warm_start`, `sleeping_pipeline_o8`, `row_identity_remap`, `row_keyed_state_defect_a`, the W-invariance gate.

**Unit and property tests:**
- `stage2_rows`: the rows not in stage 2 have a monotone `prev_row` (property vs the `HashMap` oracle, `row_identity.rs:848-867`).
- `HeldStore` move-in / restore / compaction / translation against a `Vec` model.
- `PairsView` / `ManifoldsView` `iter`, `get` and `len` against a merged `Vec`.
- `would_clear == begin_frame`'s decision (proptest).
- The warm rehash preserves every lookup at load ≤ 0.5.
- Under Miri at small n.

**Census (W5).** New arm S8, W=1, parallel off: settle to all-held, then after warm-up, per step:
- a projectile wake and re-freeze;
- spawn/despawn/migrate every 3rd step;
- a field edit;
- a sleeping toggle;
- `wake_all`.

Expected: 0 heap allocations per step. Mutations, each red: `Vec::with_capacity(1)` in restore, in move-in, and in the warm rehash. Existing pins must re-run with zero diff.

**Runner (O3).** `check_step` derives the path from public state:
- the fast path = no dynamic row awake (per `is_row_awake`);
- gravity, warm, integrate, biased, relax, restitution and freeze spans are 0 on the fast path;
- `PHYS_NP_PAIRS` = `pairs().len() − stats.withheld_pairs − stats.held_skipped_pairs`;
- `PHYS_NP_MANIFOLDS` / `POINTS` come from `solver_manifolds()`;
- `PHYS_BP_PAIRS` = `pairs().len()`;
- `PHYS_SLEEP_HELD` = held rows.

Receipts normalise per **logical** manifold. Colours are counted over `solver_manifolds()`. Mutation: the span opened inside the per-pair loop is red.

**T(W):**
- In-binary Off against Replay against Sets: R at W ∈ {1, 2, 4, 8, 16}, K=6; J-Son tail at W ∈ {1, 8}, K=6.
- A K=6 rehearsal of the J-Son tail at C2b, before any claim (review OQ2).
- Armed spans (bp, np, graph, solve, classify) and I(W) reported.
- J-S0 and J at K=12: not claimed slower.
- C3c: J cfg-A sleeping off at W ∈ {1, 8} shows no claimed regression and prints `sleeper_rebuilds == 0`.
- `row_identity_churn` sleeping-on arms × {Off, Sets}, K=6: Sets is claimed faster on every arm (bench).

**`debug_assert!`s:**
- the merged pair and manifold views are strictly increasing;
- no stream manifold names a held row;
- every held box key is present in the axis table at the end of the step;
- held rows are not awake, and their latches are asleep;
- `order` is sorted and its length equals `live_kept`;
- the records' `kept_len` sum to `live_kept`;
- `flags_gen` in the solve equals the graph's;
- INV-A7b;
- the existing order assert, now over the view.

## What moves, and how each is re-measured

- **C1b (the only value move; no golden is re-blessed).**
  - A red test whose subject is not sleeping gets an explicit `sleeping: false`. Its pin then stays byte-identical, and its first freeze step is recorded as the reason.
  - Cannot move (horizon < 61 steps): `default_world_colored_simd` (10), `default_world_worker_invariance` (60), `scene_sync_s5`.
  - Move only if an island freezes inside the horizon: `bodytype_determinism_golden` (90), `default_world_pyramid_determinism` (120; doc `:6-8` updated), `colored_acceptance_o5`, `colored_acceptance_simd_o7`, `body_set_selection`, `sdf_collision`, `soft_body_sp2`.
  - Unaffected (flag already pinned): `apply_row_alignment:344`, `alloc_frame_census:1769`, `alloc_frame_attribution:1936`, `sleep_settles_box_piles:1702/:1896`, `support/profiling_harness.rs:294`.
- **C2a accessor edits — assertions byte-unchanged; each re-run green under Off:**
  - `color(c)` → `solver_manifolds()`:
    - `default_world_pyramid_determinism.rs:199-212`;
    - `default_world_worker_invariance.rs:165-171`;
    - `frozen_island_warm_start.rs:1070-1098`;
    - `profiling_zone_counts.rs:109-136`;
    - `jolt_parity_pyramid.rs:869-910`.
  - `manifolds[0]` → `manifolds().get(0).expect(..)`: `sdf_collision.rs:324`, `physics_seam.rs:389`.
  - `for &mi in g.island(i)` → `.iter()`: `constraint_graph_o4.rs:220`.
  - Internal: `systems.rs:345`, `:392`, `:400`, `:1056`, `:1108`, `:1117`, `:1142` use the stream.
  - Everything else compiles unchanged (`len`, `iter`, `to_vec`, `is_empty`, `for &(a, b) in pairs()`).
- **C3b:** the three B1 suites stay green under the new default with unchanged assertions. That is the ruling's test, and M-V1–M-V4 show it can fail.
- **Broadphase gates moved here:**
  - bp G2 J-Son/R-S rows (T7);
  - bp G5-C5 becomes the C3c T(W) gate;
  - bp G1 gains a release script (D2 hit, then the stream equals the brute set and `withheld` is filtered).
- **Census and attribution:** pins unchanged (re-run, zero diff); new arms from C0 and S8.
- **Benches, re-recorded after C2b and C3c, not gates:**
  - `sleeping.rs:183/218`, `sleeping_pipeline.rs:124`;
  - `row_identity_churn.rs:241` (+ Sets arms);
  - bp G4 adds a held-10k arm.
- **Runner:** the R row now means sleeping on with Sets; R-off keeps the old series. MEASUREMENT-QUEUE §10 is annotated (`:520`).
- **Docs:**
  - `resources.rs` docs: ContactPairs, Manifolds and ConstraintGraph become logical; the IslandSleep "Not covered" list (SDF + sleeping is now gated by S4; the soft→rigid gap remains and is now default-path);
  - `plugin.rs:347-349`, `:405-407`;
  - `systems.rs:1083-1091`;
  - `docs/SYSTEMS.md:2055-2065`;
  - `docs/FEATURE_MAP.md` (L10 entry);
  - the unification plan must be told: C0 pre-empts U6's `IslandSleep` Vec removal; U7's `PairCache` subsumes the carry, the held store and handles.
- **Book (doc-writer only):** `book/src/simulation/physics.md:329` and `:354-364`:
  - sleeping on by default;
  - `SleepSkip` and its oracle;
  - sleeping contacts stay visible in every contact view;
  - the SDF-with-sleeping gap is closed;
  - the soft→rigid wake gap remains.

## What it cannot claim
- Any change to the J parity headline: exactly 0, since sleeping is off on both sides.
- A mixed awake/asleep figure (no P0 row). Held islands cost O(1) per row per step. A frozen island that is not held costs Replay's per-manifold price.
- A floor near Jolt's. Gather, apply, the executor, and O(N) passes remain (0.11–0.23 ms). Removing them needs archetype-level sleep (U6 / X-5), which is refactor-last.
- A free Rows step. Held box keys are mirrored (O(held box manifolds)), and the Tree's SL is translated. This is the row-keyed tax that U5/U7 delete.
- Equal warm or axis table **bytes** under Sets. Only lookups, seeds, views and poses are claimed.
- A skip for degenerate boxes, sensor-bearing islands, or islands next to a hovering awake body (those pay Replay).
- A broadphase saving under Grid or AllPairs: np, graph and solve only.
- Any fix to the recorded wake gaps (no-count-change writes, count-preserving support moves, soft→rigid reactions). They persist bit for bit.
- `island(i)` and `color(c)` values equal to Off's indices. The contents are equal, and `position_of` maps handles to Off's positions exactly.

## Open questions (owner values only)
1. Should a user write to a frozen body always wake it (Rapier's rule), closing the "no count change" gap? This is a value change, outside L10.
2. The soft→rigid reaction gap is now on the default path. Should it become its own lever, with a red-first test?
3. Axis-cache eviction by live-key compaction instead of wholesale clear. This is a value change; measure clear frequency on a churn scene first.
4. **Lane order (orchestrator):** broadphase rev 3 (T5, `stage2_rows`) must land before L10 C2a. Broadphase C5 is built here as C3c.

## Files
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L10-sleeping/02-DESIGN-REV1.md`, `03-REVIEW-OF-REV1.md`
- `D:/wt/joltab/crates/boyko_physics/src/{systems.rs,resources.rs,row_identity.rs,plugin.rs,scratch_ids.rs,sdf_query.rs}`
- `D:/wt/joltab/crates/boyko_physics/src/solver/{colored.rs,contact.rs,simd.rs,warm_start.rs}`
- `D:/wt/joltab/crates/boyko_physics/src/narrowphase/{axis_cache.rs,box_box.rs}`
- `D:/wt/joltab/crates/boyko_physics/tests/{sleep_settles_box_piles.rs,support_loss_wakes_sleepers.rs,frozen_island_warm_start.rs,profiling_zone_counts.rs}`
- `D:/wt/joltab/crates/boyko_physics/benches/jolt_parity_pyramid.rs`

Sources:
- [Box2D v3 body.c (`b2Body_GetContactData`)](https://raw.githubusercontent.com/erincatto/box2d/main/src/body.c)
- [Box2D v3 contact.c (`b2GetContactSim`)](https://raw.githubusercontent.com/erincatto/box2d/main/src/contact.c)