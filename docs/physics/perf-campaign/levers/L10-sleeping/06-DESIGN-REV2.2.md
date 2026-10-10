# L10 rev 2.2: sleeping on by default and the frozen-pair skip, re-based on L11, L9 and L5

This is a delta to `L10-sleeping/04-DESIGN-REV2.md` plus the rev 2.1 items in `levers/00-RULINGS.md`. Anything not named here stands as closed.

**Basis**
- The design was read against `D:/wt/joltab`. I could not confirm the commit (no shell) and could not run graphify.
- L5, L9 and L11 have not landed in either tree. Their sites are cited by design section, and tree sites by file:line.
- "arith." means computed from P0 spans or the lever designs' per-class estimates. None of it is measured.

**Rulings kept**
- Public views stay LOGICAL.
- No copy of the sleeping store per step.
- `SleepSkip::Off` is the oracle.
- Correctness bounds are unchanged.

## 0. Summary of the delta

| # | change | cause |
|---|---|---|
| Δ1 | **Replay is dropped as a mode, and so is its commit C2b.** A resting pair that is not held is *computed*, as Off computes it. Restore also computes, from the kept store. L10's own `manifolds_prev` / `sensor_prev` carry, the Replay tag rule and the per-chunk replay join are deleted. | L9 makes the compute of a frozen pair cheap and exact (L9-L1, §3). Replay's remaining gain is on frozen islands that are not held, and J has none of those in its steady state. |
| Δ2 | **Warm impulses:** one `WarmRecord` per kept manifold is captured at move-in by the L11 D3 carry routine from `recs_r` (t−1 stream index). On restore, a sorted `restore_rec` becomes P-a's second search source. | L11, ruling O5 |
| Δ3 | **One `PairTag`** (L9's u16) and one pair carry (L9's `pairs_prev`, `pair_tag`, `pair_tag_prev`, `reuse`, `reuse_prev`). The skip tag is `HELD_SKIP = HIT\|SEPHIT` (0x0C00). | L9 O5/O7, amendment 1 |
| Δ4 | **The kept unit becomes a pair.** Every pair with a member endpoint whose Off state is `REC ∨ SEP ∨ manifold` gets a `KeptPair` (tag, plus a `ReuseRecord` if REC). Cross-record pairs are kept in **both** records (Invariant K). | L9 W2 (REC without a manifold carries value); amendment 2 as ruled was keyed per manifold, which is too narrow |
| Δ5 | **Restore source routing:** a pair with a RESTORED endpoint joins `restore_idx`, keyed by L9's own flip-surviving `key_prev`. Every other pair uses L9's `pairs_prev` join. | rev 2.1 W2 plus L9 W2 |
| Δ6 | **The mirror set** is "kept pairs with `PairTag::rekeys()`", the same function Off's narrowphase uses. | L9 W1 |
| Δ7 | **The skip sits in L5's per-pair body** (`np_chunk` and the inline loop). The chunk count uses the stream length. E8 is replaced by L5's theorem, extended. | L5 §Sleeping |
| Δ8 | **The epoch (D5) gains `contact_reuse` and the bits of `contact_reuse_distance`.** | A change of τ or of the flag changes Off's held-pair outputs (L9 D11) |
| Δ9 | The `row_frames` fill skips HELD rows. The jumper encoding is L9's `jumper_bits` only, and it is built at the start of the broadphase. | L9 O3; research Q5 |

## 1. Decisions

### D-A. Warm storage under L11

**A1. Capture.**
- **Placement.** It runs in the solve at step t, before L11's store swap, while `recs_r` holds step t−1's records.
- **Per moved-in kept manifold `s`:**
  - Compute `key = ord(warm_remap.manifold_pair(m_t(s)))`, where `m_t` is the kept manifold with its local rows mapped to t rows.
  - Get `run = D2::find([keys_r], key)`.
  - Then `(rec, hits) = D3::carry_by_fid(m_t, recs_r, run)`, the same routine L11 D3 runs for frozen stream manifolds (L11 ruling W1).
  - Store `kept_rec[s] = rec` and add `hits` to `held_warm` and to `HeldIsland.warm_hits`.
- **Why the result is exact.** It is literally the record Off writes into `recs_w` for m at step t.
  - Carry-by-fid is idempotent on its own output (the hit fids are a fixed set), so Off's later frozen steps rewrite the same record with the same hit count.
  - So `held_warm` counts carry *hits*, not `record.count`. This retires the gate that could not fail when fids are duplicated.
- **Index space (ruling O5).** The run D2 returns is the t−1 stream index `j`.
  - The reason: move-in members and anchors are non-jumpers (M1, M2), so the translation is monotone, there is no order flip, and production streams are strict.
  - A `debug_assert!` checks that `run` contains the `j` taken from `graph(t−1).island(a)`. A2.4 hands over no list: the kept slots of this step's move-ins are the capture list.
- **Warm Reset, or warm disabled.** The capture writes an empty record with 0 hits.
  - This equals Off: with Reset, `carry_frozen` misses every point and drops it permanently.
  - With warm disabled, D5 flushes before any move-in.

**A2. Storage.** `SleepSets::kept_rec: ScratchColumn<WarmRecord>` is indexed by kept-manifold slot, parallel to `HeldStore::kept`.
- `KeptWarm`, `warm_of` and `restore_warm` are deleted.
- Kept records are opaque 64 B copies. When L8 changes `WarmRecord`, only a size assert moves.

**A3. Restore.** In A2.5 (bbroadphase), for each restored record:
- Copy its `kept_rec` run into `SleepSets::restore_rec: WarmRecords { keys, recs, strict: true }`.
- The keys are the manifolds' t−1 identities: `ord(min(pa,pb), max(pa,pb))`, with `p = prev_row[row table]`, or `(1<<63)|pa` for SDF.
- Sort them with `sort_unstable` in place (no heap). The cost is O(r log r) on restore steps.
- In the solve, P-a's single D2 routine searches `keys_r` and then `restore_rec`. `Plan.src` sets bit 31 (`SRC_RESTORE`) when the run lies in `restore_rec`.
  - The two key sets are disjoint, because no stream manifold names a held row (rev 2 E5). A run therefore lies in one source.
  - A flipped pair or a Reset makes `manifold_pair` return `None` in both modes, so no special case is needed.
- After the solve reads `restore_rec` it is truncated to length 0, and it is stamped with the gather seq. A stale stamp is `debug_assert!`ed and treated as empty.
- **Rejected:**
  - rev 2's insert and rehash into `warm_read`: L11 has no table;
  - merging into `keys_r`: an O(M) copy on every restore step;
  - reading `kept_rec` in place: it adds a third index form to D2's routine, against W1.

**A4. Obsolete in rev 2 (and C3a):**
- A6's `restore_warm` insert and rehash, and A6's capture "exactly as `carry_frozen` does";
- C3a's need-sized `rebuild` + `len` + idle-buffer rehash (`warm_start.rs:274-288`);
- the `kept_warm` ≈ 0.39 MB memory row.

**C3a keeps the no-awake fast path** (`colored.rs:3487-3503`, substep loop `:3547-3617`). On that path it must still run P-a (so frozen stream manifolds get `keys_w`), D3, the swap and the stamp. Mutation N16 covers this.

### D-B. L9 tags, records and the kept store

**B1. One tag, one carry.**
- L10 uses L9's `PairTag` bits `6 SET | 7 SETTLED`. It writes them in the compute path:
  - SET = a box pair made a contact;
  - SETTLED = `hint == Some(chosen axis)`.
- L10's own `pairs_prev`, `pair_tag` and `pair_tag_prev` are deleted, which frees 3 ids.
- **`PairTag::HELD_SKIP = 0x0C00` (HIT and SEPHIT together).**
  - It cannot come out of a compute.
  - It has no REC and no SEP, so L9's join treats it as stateless.
  - It is its own class for the counters.
  - Every held-skipped slot writes it every step (L9 O5).
- **Cursor.** The kept store keeps L10's cursor, stamped in the broadphase. The A1.1 prologue also `peek`s L9's `carry_cursor`. If the two classes disagree, that is a D7 flush. So the restore key and L9's join always classify against the same gather.

**B2. The kept unit is a pair.**
- **Enumeration at move-in.** A2.4 scans `pairs_prev` against step t−1's tags and records, which at the broadphase epilogue are still `pair_tag` and `reuse` (L9 D9's swap happens in the narrowphase prologue). The cost is O(P_prev) on move-in steps only: ≈20 µs at J.
- **What is kept.** A pair with an endpoint in a moving-in island and `tag ∈ REC ∨ SEP ∨ PUSHED(manifold)` becomes a `KeptPair` at the record's local indices.
  - Its non-member endpoint joins the record's anchors, so D1a watches it.
  - If REC, the pair's `ReuseRecord` is copied to `kept_reuse`.
- **SDF manifolds** stay in `kept` with no pair entry.
- **Why SEP is kept.** SEP is exact, but keeping it makes the tags of computed slots byte-equal to Off's, so the gate is a plain compare. The cost is 20 B per SEP pair.
- **Rejected:**
  - per manifold only (the ruled amendment's wording): it drops REC state on pairs with no manifold, which is value-bearing (L9 review W2, spurious A4 wake);
  - rewriting kept state every step (Box3D): there is no persistent pair id; that is U7.

**B3. Cross-record pairs: Invariant K.** *For every pair with a member endpoint in a record held at t−1, every such record keeps the pair's state iff it is kept-class, and all copies equal Off's steady state.*
- A pair between a moving-in member `x` and a PRE_HELD member `y` (record Y) was skipped or withheld at t−1, so its t−1 slot holds no state.
  - The A2.2 D2 scan already walks stream pairs that have a held or candidate endpoint. It lists such `(x, y)` pairs in `SleepSets::scratch`. On Tree steps it walks `withheld` too.
  - A2.4 copies the pair's `KeptPair` and `ReuseRecord` from Y's run. The lookup is by local pair `(loc_Y(x), loc_Y(y))`. It is a binary search, valid because a record that touches a jumper is restored, so a non-restored row table is sorted.
- **Steady state.** At bit-equal poses Off's output is constant:
  - L9-L1 is exact: `q*⊗q` has a vec part of exactly 0 in IEEE, so `s = 0`, and `Δd = 0`;
  - an SEP axis stays negative.
  So a copy taken at any held step stays equal to Off's state.
- **Duplicates.** Both copies are in `restore_idx` when both records restore on one step. The first is taken, and their equality is `debug_assert!`ed.
- **Order within A2.** The A2.5 restore filter runs first: tombstone, filter `order`, build `restore_idx` and `restore_rec`. Then A2.4 captures; it may read a tombstoned record's copy. Then the `order` merge.
- **Compaction moves to A1.0b.** It compacts only records that died on earlier steps. So kept data of a record restored at t stay valid through the step-t narrowphase.
- **Rejected:**
  - group records (a union of islands): needs a root per member and `held_len` per island, and restores more than needed;
  - refusing move-in on cross pairs: debris fields would never be held.

**B4. Restore = compute from the kept source.**
- **Moment one (restore).** A2.5 writes `restore_idx: ScratchColumn<RestoreEntry { key: u64, tag: PairTag, _p: u16, rec: u32 }>` (16 B), sorted.
  - The key is L9's `carry::key_prev` form for the pair's t−1 rows; it survives a flip (L9 W2).
  - RowCls gains a RESTORED flag on the members of records restored this step.
- **Moment two (narrowphase).** A pair with a `RESTORED` endpoint does `lower_bound` in `restore_idx`. On a hit, `(tag_prev, reuse_prev)` = (entry tag, `kept_reuse[rec]`). L9's `collide_box_pair` then runs unchanged, with the parity debug-check waived for this source. A miss is cold, as in Off.
- **Every other pair** uses L9's `pairs_prev` join. Under Grid, a pair that was held at t−1 sits in `pairs_prev` with `HELD_SKIP` (stateless), but it always has a RESTORED or a still-HELD endpoint, so it never reaches that join.
- **What an L9b-reused contact goes through:**
  - **Freeze (CAND steps).** It is computed: REC|HIT at 60–100 ns, a 128 B record copy, a constant `refresh`.
  - **Move-in.** Record, tag and warm record are copied once.
  - **Held.** Nothing touches it.
  - **Wake.** It is computed from the kept record, a hit at unchanged poses. So the count is unchanged, there is no spurious A4, and a support moved by less than τ_eff behaves exactly as in Off.
- **Obsolete:** rev 2's "kept runs hold no pairs without a manifold", review OQ2's reliance on INV-A7b for REC pairs, and amendments 3 and 4 as worded (see B5).

**B5. Move-in rule M3, restated.** Evaluated per kept box pair from its t−1 tag, now visible to the enumeration:
- REC: always settled. L9-L1 covers both a hit and a miss at t−1.
- SEP: always settled (no table write).
- Non-REC with SET: requires PUSHED (INV-A7b over `box_box_classify`, `box_box.rs:642-734`) and SETTLED.
- Otherwise refuse.

Amendment 4's "HIT at t−1" condition is replaced by this weaker, still sufficient one.

**B6. `held_len` counts kept manifolds only** (the `begin_step` key, `resources.rs:3531-3549`). Kept pairs never enter the count (mutation N8).

### D-C. Axis mirror (L9 W1)

`PairTag::rekeys(t) := t.has(REC) || (t.has(SET) && !t.has(REC))` reads HIT-invariant bits only. It is the single function L9's commit (the "set on key-change steps" rule) and L10's mirror both call. If the L9 lane shipped the predicate inline, C0 extracts it, bit-identically.

- **When.** `begin_frame` returns `KeyChange { prefetched, cleared, grown }`. The mirror runs after it and before the L5 scope (serial). It re-keys:
  - every kept pair with `rekeys(tag)`, if cleared or grown;
  - only the pairs whose rows moved, if the step is a plain Rows step;

  using the axis in tag bits 0–3.
- **Why the lookups equal Off's.** Linear probing without deletion gives probe results that are independent of other keys' insertion order (L5 Lemma 1). Held keys and stream keys are disjoint. So every stream hint and every final lookup equals Off's, and so does `occupied`, hence the clear decisions.
- **What differs.** Table bytes, as rev 2 already states.
- **The SEP class** is not mirrored, because Off does not key it (N10).
- **Doc fix.** `axis_cache.rs:62-73` ("a hint picks the contact, never whether there is one") is amended: false for REC pairs under L9b; harmless under Sets because held REC pairs are never computed.

### D-D. L5's chunked emit

**D1. Placement.** One `#[inline] fn np_pair<const SETS: bool>(ctx, k)` is called by `np_chunk` and by the inline loop (L5 D6).

```text
if SETS && held_skip(cls[a], cls[b]):   // HELD endpoint ∧ !sensor pair (rev 2 A3(i))
    commit[k] = AXIS_NONE; tag[k] = HELD_SKIP; skips += 1; return   // no stage write: runs do not advance
src = if SETS && (cls[a]|cls[b]) & RESTORED { restore_idx cursor } else { L9 pairs_prev cursor }
... L9 collide / L5 routing unchanged
```

- `RowCls` is 8 B per row (10 KB at J), read-only during the scope and passed as `&[RowCls]` in `NpChunkCtx`.
- `restore_idx` and `kept_reuse` are read-only views taken before the scope.
- The per-chunk start of the restore cursor is a `lower_bound` of the chunk's first key, as for L9's join (N14).

**D2. Chunk count.** It uses the **stream** length, `ContactPairs::pairs_stream().len()` (`pub(crate)`). The public `pairs()` becomes a `PairsView` (rev 2 D6).
- Tree, all held: n = 0, so the path is inline and no scope opens.
- Grid, all held: n = P. There are 48 chunks of 1–2 ns-per-pair skips, which pays ω(8) ≈ 6.5 µs plus spawns. That is stated, not optimised (OQ4).

**D3. Stats.** A second function-local `[AtomicU32; NP_MAX_CHUNKS]` (1 KB of stack) takes one `Relaxed` store per chunk and is summed after the join. `held_skipped_pairs` therefore stays always-on with no per-pair atomics. L5's `(n_out<<32)|n_sen` meta is unchanged.

**D4. The SDF stage stays serial** (L5 did not build a parallel one), so A4 is unchanged.

### D-E. Epoch (D5 += 2 fields)

`SleepEpoch` gains `reuse: u8` and `tau_bits: u32`. A change means flush all.
- The reason: under Off, a new τ or `contact_reuse = false` recomputes held pairs on the next step with different outputs (L9 D11), so a held island would diverge.
- The field goes in the existing `_p` byte, plus 4 B.

## 2. Data structures (only what changes)

```rust
// Manifolds::held: HeldStore
kept_pair:  ScratchColumn<KeptPair>,          // per record, contiguous, sorted by (la, lb) local
kept_reuse: ScratchColumn<ReuseRecord>,       // 128 B, only REC pairs; cold
#[repr(C)] struct KeptPair { la: u32, lb: u32, m_slot: u32 /* kept manifold | NONE */,
                             r_slot: u32 /* kept_reuse | NONE */, tag: PairTag, _p: u16 } // 20 B
#[repr(C)] struct HeldIsland { root, rows_start, n_members, n_anchors, kept_start, kept_len,
    box_kept, points, flags, pairs_start, pairs_len, warm_hits: u32 }  // 48 B (was _p: [u32; 3])
// deleted from Manifolds: manifolds_prev, sensor_prev, pair_tag, pair_tag_prev (L9 owns the tags)

// SleepSets
kept_rec:    ScratchColumn<WarmRecord>,       // 64 B per kept manifold slot (replaces kept_warm + warm_of)
restore_rec: WarmRecords,                     // keys + recs, strict; restore steps only (replaces restore_warm)
restore_idx: ScratchColumn<RestoreEntry>,     // 16 B, sorted by L9 key_prev; restore steps only
restore_seq: u64,
// RowCls.flags: + RESTORED; − JUMPER (read jumper_bits); SleepEpoch: + reuse, tau_bits
pub enum SleepSkip { Off, #[default] Sets }   // Replay removed
```

**Memory at J, everything held (arith.):**

| column | size |
|---|---|
| kept manifolds | 0.70 MB (unchanged) |
| `kept_rec` | 4,524 × 64 B = 0.29 MB |
| `kept_pair` | ~9.6k × 20 B = 0.19 MB |
| `kept_reuse` | ≤ 4,524 × 128 B = 0.58 MB |
| **total** | **1.76 MB** |

- Rev 2's comparable columns came to 1.79 MB (kept 0.70 + kept warm 0.39 + L10 carry 0.70).
- L9's carry columns are not counted: L9 already owns them.

**Scratch ids:**
- rev 2 had 23;
- remove 8: 3 tag/pair carry, 2 replay carry, `kept_warm`, `warm_of`, `restore_warm`;
- add 5: `kept_pair`, `kept_reuse`, `restore_idx`, `restore_rec.keys`, `restore_rec.recs`;
- net **20**.

`SCRATCH_REGION_MIN_ID` still moves to MAX−160, and the census is re-run. The cohort width stays ≤ 64, and co-swept pairs are asserted (`scratch_ids.rs:666-688`).

**Hot/cold:** `RowCls` is the only new per-step read in the narrowphase. Everything else is touched only on transition steps.

## 3. Determinism (replaces rev 2's E2, E5 and E8; E4 and E6 are extended)

- **E2′. Compute is the only path.** Every non-skipped pair is computed. Its source (`pairs_prev` slot or `restore_idx` entry) holds Off's t−1 state:
  - by induction for stream slots;
  - by Invariant K for kept entries.
  - The join key function is L9's in both cases.
  - `row_frames` of non-HELD rows are filled.
  - Hints equal Off's (D-C).
  - So the outputs equal Off's.
- **E4′. A held island is a fixed point of Off's map.** This is rev 2's E4 plus:
  - Off's per-step output for each held pair is constant (L9-L1 exact; SEP stays negative; SETTLED for non-REC SET pairs);
  - Off's warm carry record is constant (D3 is idempotent);
  - the epoch covers τ and `contact_reuse`.
- **E5′. Shared state.**
  - Axis lookups and `occupied` equal Off's (D-C).
  - Warm lookups: stream records ⊎ `restore_rec` (at restore) equal Off's `keys_r`, by Lemma W plus E4′ (disjoint keys, same D2 routine).
  - Record bytes are **not** claimed: Sets' `mi` numbering excludes held manifolds.
- **E6′.** `carry_hits` = stream D3 hits + `held_warm`, where `held_warm` is exact by A1's idempotence.
- **E8′. W invariance** is L5's theorem, extended.
  - `np_pair`'s outputs (stage, commit, tag, reuse, skip count) are a pure function of inputs that are read-only during the scope: bodies, `RowCls`, L9's carry, `restore_idx`, `kept_reuse`, T₀ / `remapped`, `row_frames`.
  - Writes are per slot, runs are joined in order, and the mirror and the axis commit are serial.
  - Everything else in L10 is serial.
  - Replays on any machine: integer keys and sorts, byte copies, `to_bits` compares, IEEE ops shared with Off. No id identity is involved, because everything is row-keyed through `RowIdentity`, as in L9 and L11.

## 4. Expected gain (arith.)

Off′ is today's sleeping-on path with the Tree broadphase, L9, L11 and L5 in. Rev 2's Δ assumed a pre-L9 Off and is withdrawn.

**Off′, J-Son tail [264, 1000), all frozen**

| term | W=1 | derivation |
|---|---|---|
| bp (Tree, Off) | 0.09–0.24 | rev 2 Off 3.56–3.64 minus P0's np 2.92, graph 0.12, solve 0.30 and remainder 0.064–0.134 |
| np (L9, h = 1 by L9-L1) | 0.48–0.80 | 4,524 hits × 60–100 ns + 5,037 SEP × 35–60 ns + 9,561 joins × 2–4 ns + `row_frames` 0.012 |
| graph | 0.12 | P0 |
| solve (L11) | 0.07–0.30 | carry 0.03–0.07 (L11's R-S 0.05–0.10 × 4,524/6,662) + P-a ≈ 0.02 + O(N) 0.015–0.03; the upper end is P0 unchanged |
| remainder | 0.064–0.134 | P0 |
| **Off′(1)** | **0.82–1.59 ms** | |
| **Off′(8)** | **0.42–0.93 ms** | np → 0.12–0.20 = 0.48–0.80 / (8 × 0.681) + compaction 27–45 µs + dispatch 6.5 µs; graph 0.10; g = 0.035 |

| row | Off′ | Sets (rev 2 floor, unchanged) | Δ | realized-gain gate (0.6 × lower) |
|---|---|---|---|---|
| J-Son tail, W=1 | 0.82–1.59 | 0.11–0.23 | **0.59–1.48 ms** | 0.35 ms |
| J-Son tail, W=8 | 0.42–0.93 | 0.08–0.18 | **0.24–0.85 ms** | 0.14 ms |
| R and R-S, W ∈ {1, 8} | measured at the lane base (C0, armed) | 0.12–0.25 | Off′_R − floor | 0.6 × (Off′_R,C0 − 0.25) |
| J (sleeping off on both sides) | — | — | **0** | not claimed slower |

- **Per contact, J-Son tail, per logical manifold (4,519), W=1:**
  - Off′ is 0.18–0.35 µs;
  - Sets is **0.024–0.051 µs**, of which the narrowphase is 0 and the broadphase is the Tree verify.
- **Per-stage reporting** (µs per logical manifold): bp, np, graph, setup (build + warm + store) and colours.
- **Rev 2's Δ shrinks 3–6×.** That is the honest consequence of L9 and the Tree landing first.
- **Jolt reference.** Jolt's tail is 2.17 µs *total* with 0 contacts (it drops them on sleep), so no per-contact ratio exists. Only the per-step ratio is reported: Sets(1) ≈ 50–105× Jolt, against P0's 2,450×.

## 5. Changed file:line list (against rev 2's list)

| site | change |
|---|---|
| `solver/colored.rs:1443-1481`, `:3229-3319` | rev 2's A6 and C3a edits dropped. L11 C1 deletes `warm_read`/`warm_write`, `frozen_points` and `carry_frozen` |
| L11 `solver/warm_records.rs` (`find`, D3 `carry_by_fid`, `Plan`) | `find` takes the optional second source; `Plan.src` gets bit 31 `SRC_RESTORE`; `carry_by_fid` is `pub(crate)` for A1 |
| `solver/colored.rs:3459-3472` (warm classification), L11 P-a site | P-a searches `restore_rec`; A1's capture runs before the store (`:3627-3630`) |
| `solver/colored.rs:3487-3503`, `:3547-3617` | C3a fast path keeps P-a, D3, the swap and the stamp |
| `solver/warm_start.rs:274-288` | **no L10 edit** (it was rev 2 C3a) |
| `systems.rs:297-349` | L9's swap before `:306`. A1 is the prologue; A1.0b compaction and the build of `jumper_bits` (moved from L9's np prologue) go there. A2 is the epilogue, in the order A2.1–A2.3, A2.5a, A2.4, merge. The assert at `:344-347` becomes strict, over `PairsView` |
| `systems.rs:375-478` / L5 `narrowphase/dispatch.rs` (`NpChunkCtx`, `np_chunk`, `try_parallel`) | `np_pair<SETS>`; `cls` / `restore_idx` / `kept_reuse` views; skip meta; the chunk count from the stream (`:400` loop, `:437-446` box arm, `:455-465` routing, `:472-477` counters) |
| `systems.rs:391-392`; `narrowphase/axis_cache.rs:261-281`, `:367-388` | `begin_frame_synced(stream, P_logical) -> KeyChange`; mirrors via `set` (`:319-354`, unchanged); doc `:62-73` |
| L9 `narrowphase/carry.rs` | `PairTag::{HELD_SKIP, rekeys}`; `key_prev` shared with `restore_idx`; the parity waiver for the kept source |
| L9 `narrowphase/reuse.rs` / the `row_frames` fill | skip rows with `RowCls::HELD` |
| `resources.rs:442-530` | `SleepSkip { Off, Sets }` |
| `resources.rs:562-611` | `pairs()` → `PairsView`; `pairs_stream()` `pub(crate)` (`:597`); no L10 `pairs_prev` |
| `resources.rs:2269-2327` | `HeldStore` + `kept_pair`, `kept_reuse`; rev 2's 4 carry columns removed |
| `resources.rs:3531-3549` | `island_len = CSR + held_len` (manifolds only) |
| `row_identity.rs:214-245` | `RemapCursor::peek` (rev 2), used on L9's `carry_cursor` |
| `scratch_ids.rs:229`, `:309-311` | no L10 warm-table id |
| `scratch_ids.rs:729`, `:734-739` | floor to MAX−160 |
| `scratch_ids.rs:889`, `:937-939` | L10 cohort 20 ids |
| `tests/frozen_island_warm_start.rs:41-51` | mutation list: M11/M20 renamed, N1/N3 added |
| `tests/frozen_island_warm_start.rs:642`, `:663`, `:692`, `:737`, `:908`, `:1255` | run under Sets, assertions unchanged |
| `benches/jolt_parity_pyramid.rs` | closure `full + reused + sep_hits + non_box + held_skipped == stream`; `chunk_count(pairs().len() − withheld)`; `--sleep-skip off\|sets` |

## 6. Commit sequence (each green: `--workspace --all-targets --no-fail-fast`, clippy `-D warnings`, the Miri leg)

**Before C0.** On the lane base (L5 + Tree + L11 + L9 merged), record the pose hashes of R-S, J-Son and R at W ∈ {1, 8}.
- These become the fixtures. P0's hashes no longer apply, because L9 C4 moved values.
- Record the armed Off′ spans for J-Son and R-S at W ∈ {1, 8}, K=6, to feed the R gate.

| commit | content | value change |
|---|---|---|
| **C0** | rev 2 D14 (`IslandSleep` columns). Move the `jumper_bits` build to the broadphase start. Extract `PairTag::rekeys` if it is inline. | none |
| **C1a / C1b** | as rev 2. C1b is the only value mover. | C1b |
| **C2a** | Plumbing under `Off`: `SleepSkip { Off, Sets }`, `SleepSets` flags, epoch with reuse/τ, `effective_inv_mass`, coloured variants, the logical view types over an empty store, `np_pair<SETS>` with `SETS = false`, the skip meta array, counters. | none |
| ~~C2b~~ | deleted (Δ1) | — |
| **C3a** | L11-shaped no-awake fast path; runner per-path expectations | none |
| **C3b** | Sets: `HeldStore` with pairs and records, A1/A2 (Tree `NoHint`), A1 capture, `restore_rec`, `restore_idx`, the narrowphase skip and restore routing, mirrors, A4, A5, D8, D10 | none |
| **C3c** | Tree seam (bp C5 as T1–T6) | none |

## 7. Gates

Rev 2's gates stand except as noted here. The comparator gains two crate-internal compares:
- tags of computed slots are byte-equal to Off's;
- each held slot has `tag == HELD_SKIP`, and its kept tag, after `steady()` sets HIT iff REC and SEPHIT iff SEP, equals Off's slot tag.

**Warm lookups.** Warm lookups use the D2 comparator over stream records ⊎ kept records, for every logical point key.

**Scene matrix.** S1–S5 × {Tree, Grid-parallel} × `parallel_narrowphase` on × `contact_reuse` ∈ {on (default), off} × W ∈ {1, 8}; W ∈ {2, 4, 16} in release.

**S3 gains six arms:**
1. a held island whose member becomes a jumper with a row-order flip;
2. a support moved by less than τ_eff (L9 W2) while held, then a wake;
3. two adjacent islands with REC-without-manifold and SEP cross pairs, restored X-only, then Y-only, then both;
4. a τ change and a `contact_reuse` toggle while held;
5. ≥ 64 dead records, so that compaction fires;
6. Grid with everything held at W=8.

**Anti-vacuity, added.** `restore_idx` hits > 0; cross-pair copies > 0; `restore_rec` searches > 0 with hits > 0; a REC-without-manifold pair is held; flip arm 1 shows a warm miss in both modes.

**Mutations: obsolete, renamed and new**

| id | mutation | red via |
|---|---|---|
| ~~M2–M6~~ | Replay mutations | obsolete (no Replay) |
| M10 | the mirror placed before `begin_frame` | kept, now on the S3 grow-clear arm |
| M11′ | `restore_rec` not searched | `frozen_island_warm_start` exact wake under Sets |
| M20′ | `restore_rec` keyed in t rows | S3 Rows restore |
| N1 | capture after the store swap (it reads step t's records) | exact-wake seed compare |
| N2 | capture copies `recs_r[j]` verbatim, not through `carry_by_fid` | unit test: kept manifold with a duplicated fid |
| N3 | `restore_rec` keyed by the flip-surviving key | S3 arm 1 (Off misses, Sets hits) |
| N4 | `restore_idx` keyed by the warm key (`None` on a flip) | S3 arm 1 with reuse on (tag compare) |
| N5 | the kept store drops `ReuseRecord`s | S3 arm 2 (tag and pose) |
| N6 | REC pairs without a manifold not kept | S3 arm 3 |
| N7 | SEP pairs not kept | tag compare |
| N8 | `held_len` counts kept pairs | M12 class (keys and A4) |
| N9 / N10 | the mirror excludes REC hits / includes SEP pairs | axis lookups on the grow-clear arm |
| N11 | a held slot leaves its tag unwritten | the `HELD_SKIP` compare under Grid |
| N12 | skip only on the inline path | W=8 stream compare; "no stream manifold names a held row" assert |
| N13 | chunk count from `pairs().len()` | Tree all-held step: dispatch count must be 0; the cut bounds assert |
| N14 | restore cursor starts at `lo` | partition proptest (G-C-2 extended to restored runs) |
| N15 | epoch ignores reuse/τ | S3 arm 4 |
| N16 | compaction after this step's tombstones, or the fast path skipping P-a/D3 | S3 arm 5; S1 fast-path lookups |
| N17 | the `row_frames` skip keyed by PRE_HELD | restored-pair compare |
| N18 | cross pair captured from its t−1 slot, not from Y's copy | S3 arm 3, X-then-Y order |
| census | `Vec::with_capacity(1)` in the `restore_rec` build, the `restore_idx` build or cross capture | census arm S8 |

**Unit tests.**
- Deleted: "the warm rehash preserves every lookup".
- Added:
  - a `KeptPair` / `restore_idx` model against a `Vec` oracle;
  - `rekeys()` exhaustive over the u16 tag space against L9's commit rule;
  - `steady()` exhaustive.

**L5 gates re-stated.**
- G-L5-3's "+1 dispatch per step on W ≥ 2" becomes `+[chunk_count(stream_len, lanes) ≥ 2]` per step.
- G-L5-5's S1c pin of 134 scopes holds only on sleeping-off arms. The sleeping-on arms of the census (S8) pin their own scope count.

**End-to-end A/B (P0 protocol).** Median over K processes of the window mean; receipts per process; canary seen; order interleaved; claim iff |effect| > 2·√(SE_A² + SE_B²), SE = 1.2533·SD/√K.
- **In-binary `--sleep-skip off|sets`, K=6, for the realized-gain gates (§4):**
  - J-Son tail at W ∈ {1, 2, 4, 8, 16};
  - R and R-S at W ∈ {1, 2, 4, 8, 16}.
- **A K=6 rehearsal of the J-Son tail at C3b, before any claim.** This replaces rev 2's C2b rehearsal. If Off′ measures below 0.6 × 0.59 + 0.23 ms, the prediction is refuted and escalated.
- **Per contact:** armed rows at W ∈ {1, 8}, in µs per logical manifold per stage (bp, np, graph, setup, colours), for J-Son and R. They are reported beside the per-step table.
- **Not claimed slower:**
  - J and J-S0 at K=12;
  - J plus 10k statics (rev 2.1 W3), K=6;
  - the Grid all-held arm at W=8 against W=1.
- **`row_identity_churn`:** the sleeping-on arms × {Off, Sets}; Sets is claimed faster.
- **Pose hashes:** Off rows equal the pre-C0 fixtures; one hash per row across W and across Off/Sets.

## 8. What moves, and how each is re-measured

**C1b.** Rev 2's list is unchanged. The pins are now reuse-on trajectories (L9 C4 is in), and each is re-measured under its own file's rule.

**L5 census.** Default sleeping-on arms re-pin the narrowphase scope count, which now depends on the stream length. They are measured at C3b, not assumed.

**Runner.**
- `PHYS_NP_PAIRS` = the stream length.
- The L9 closure gains `held_skipped`.
- `chunk_count` takes the stream length.
- Receipts normalise per logical manifold.

**Tests edited by C2a.** Rev 2's list, assertions byte-unchanged.

**Docs.**
- `axis_cache.rs:62-73`.
- The L11 mapping table (`L11 02-DESIGN-REV1.md:289-297`) gets a pointer to this delta.
- L9's "Interactions → L10 amendments 2–4" are superseded by B2–B5.

**Benches.** `sleeping.rs` and `sleeping_pipeline.rs` are re-recorded after C3b and C3c.

## 9. What it cannot claim
- **Rev 2's Δ figures.** They are superseded by §4, which is 3–6× smaller because L9 and the Tree land first.
- **Replay's saving on frozen islands that are not held**: sensor-bearing ones, degenerate boxes, islands next to a hovering body, and CAND steps. They pay L9's compute (35–100 ns per pair).
- **Warm-record, axis-table or tag bytes equal to Off's on held slots.** Only lookups, computed-slot tags, `steady(kept tag)`, views, seeds and poses are claimed.
- **A broadphase or dispatch saving under Grid or AllPairs.** At W ≥ 2 the Grid still opens one scope of skips.
- **Any Jolt per-contact ratio for the sleeping tail.** Jolt keeps no contacts while asleep.
- **R's gain before the base measurement (C0).**
- **Any fix to the recorded wake gaps.** They are unchanged.

## 10. Open questions (orchestrator)
1. **Δ1 retires Replay and C2b from a closed design.** This needs a ruling. The alternative is keeping C2b, gated under the build-if rule on a row with a frozen island that is not held. No such P0 row exists.
2. **The cross-lane contract for L9:**
   - `PairTag::HELD_SKIP = 0x0C00` reserved;
   - `rekeys()` and `key_prev` as single functions;
   - the parity waiver for the kept source.
   If L9 ships otherwise, L10 C0 extracts them, bit-identically.
3. **Always-on `SleepSkipStats` from a stack array per chunk** (1 KB, one `Relaxed` store per chunk). Rev 2 had no parallel phase.
4. **Grid with everything held at W ≥ 2** still dispatches 48 chunks of skips. Should a live-pair count gate the dispatch? That would add one O(P) serial pass, so it should be built only if measured above the SE bar.

## Files
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L10-sleeping/04-DESIGN-REV2.md`, `05-REVIEW-OF-REV2.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/00-RULINGS.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L11-solve-setup/02-DESIGN-REV1.md`, `03-REVIEW-OF-REV1.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L9-contact-reuse/02-DESIGN-REV1.md`, `03-REVIEW-OF-REV1.md`
- `D:/wt/joltab/docs/physics/perf-campaign/levers/L5-narrowphase/02-DESIGN-REV1.md`
- `D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/ANALYSIS.md`
- `D:/wt/joltab/crates/boyko_physics/src/{systems.rs,resources.rs,row_identity.rs,scratch_ids.rs}`
- `D:/wt/joltab/crates/boyko_physics/src/solver/{colored.rs,warm_start.rs}`
- `D:/wt/joltab/crates/boyko_physics/src/narrowphase/axis_cache.rs`
- `D:/wt/joltab/crates/boyko_physics/tests/frozen_island_warm_start.rs`

Sources:
- [Box2D v3 solver_set.c](https://raw.githubusercontent.com/erincatto/box2d/main/src/solver_set.c)
- [Box2D v3 physics_world.c](https://raw.githubusercontent.com/erincatto/box2d/main/src/physics_world.c)
- [Box3D contact.h](https://raw.githubusercontent.com/erincatto/box3d/main/src/contact.h)
- [Bepu PairCache_Activity.cs](https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/CollisionDetection/PairCache_Activity.cs)
- [Rapier narrow_phase/contacts.rs](https://raw.githubusercontent.com/dimforge/rapier/master/src/geometry/narrow_phase/contacts.rs)
- [Jolt ContactListener.h](https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Collision/ContactListener.h)
