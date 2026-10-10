# Research: L10 rev 2.2, the delta. Sleeping re-based on L11's per-manifold warm records, L9's per-pair tags and records, and L5's chunked narrowphase emit

**Scope and caveats**
- I read `D:/wt/joltab` as a working copy. I have no shell, so I could not confirm the commit is `47c5dabd`, and graphify could not be run.
- None of the designs this delta builds on has landed in `D:/wt/joltab/crates/boyko_physics/src`. A grep for `withheld|stage2_rows|HeldHint|parallel_narrowphase|pair_tag|pairs_prev|WarmRecord` matches 0 files.
- The L5 lane working copy (`D:/wt/l5np`) has L4 and L2 only:
  - `parallel_solve: true` at `resources.rs:506-507`;
  - the lanes term at `solver/colored.rs:3555-3557`;
  - `GRID_LO = 2_700` and `GRID_HI = 3_000` at `broadphase_policy.rs:89,101`;
  - there is no `narrowphase/dispatch.rs` and no `parallel_narrowphase` symbol. So the L5 chunk facts below come from L5 rev 1 plus its rulings, not from code.
- Every L10 rev 2 file:line I re-checked still matches this tree.
- "(derived)" marks a conclusion I drew from the cited text or code; it is not a sourced fact.

## Brief summary (TL;DR)
- **Reference engines keep the whole per-contact record across sleep.**
  - **Box2D v3** `memcpy`s each touching `b2ContactSim` (manifold with per-point impulses, plus flags) into the sleeping set. It also moves non-touching contacts whole into `b2_disabledSet`. On wake both are copied back and no flag is reset. Warm impulses are per contact: the old manifold is kept inside the contact sim and its points are matched by id.
  - **Box3D** keeps its reuse ("recycle") state on the persistent `b3Contact`, which never moves between sets. Collision only visits awake contacts, so the reuse state survives sleep untouched.
  - **Bepu** moves the pair key plus its collision cache (feature ids, constraint handle) into a sleeping subcache and re-inserts them into the hash map on wake. Impulses stay with the inactive constraints.
  - **Rapier** never visits sleeping pairs. It builds the list of changed pairs before the per-pair loop.
  - **Jolt** drops all contacts when a body sleeps, so its wakes start cold.
- **(a) L11.** The B1 carry becomes L11's D3 copy-by-fid for frozen manifolds still in the stream. The held store keeps one 64 B `WarmRecord` per kept manifold, copied at move-in from `recs_r` at the **t−1 stream index**, as ruled. The restore becomes a sorted second search source for P-a: rev 2's rehash, the `kept_warm`/`warm_of`/`restore_warm` machinery and C3a's table rebuild are obsolete.
- **(b) L9.** Three consequences:
  - One u16 tag layout and one shared carry (`pairs_prev`, `pair_tag`, `pair_tag_prev`).
  - The kept store must also carry L9 `ReuseRecord`s, including records of held pairs that have **no manifold** (a REC pair whose points all lifted). Losing such a record changes post-wake values and can cause a spurious wake.
  - A restored pair that is *computed* (not replayed) needs the kept record as its join source. L9's join over `pairs_prev` cannot find it.
- **(c) L5.** Two consequences:
  - The skip is a per-pair predicate in front of `collide_pair` inside `np_chunk` (L5 rev 1 §Sleeping). It writes `commit[k] = AXIS_NONE` and the tag, and no stage row.
  - Rev 2's E8 ("all L10 code is serial") no longer holds for the narrowphase arms. W-invariance must come from L5's theorem instead. Chunk counts must use the internal stream length, not rev 2's logical `PairsView`.
- **The axis mirror (E5) changes predicate.** Under L9 W1, Off re-keys hit pairs on every step where the key set changes. So the mirror set is "held pairs that Off's narrowphase would `set` this step", not "held box manifolds".

## Approaches in state-of-the-art engines (only what this delta needs)

### Box2D v3 (erincatto/box2d, `main`)
- **Sleep, touching contacts** (`solver_set.c`, `b2TrySleepIsland`). The whole struct is copied: `memcpy( sleepContactSim, awakeContactSim, sizeof( b2ContactSim ) );`, followed by re-encoding of the body indices (`b2SleepBodySimIndex`).
- **Sleep, non-touching contacts** (verbatim):
  - comment: `// Move non-touching contacts to the disabled set. // Non-touching contacts may exist between sleeping islands and there is no clear ownership.`
  - They move only when the other body is also asleep: `if ( otherBody->setIndex == b2_awakeSet ) { continue; }`, then `memcpy( disabledContactSim, contactSim, sizeof( b2ContactSim ) );`.
- **Wake** (`b2WakeSolverSet`, verbatim):
  - `// transfer touching contacts from sleeping set to contact graph` … `b2AddContactToGraph( world, contactSim, contact ); contact->setIndex = b2_awakeSet;`
  - `// Move non-touching contacts from disabled set to awake set.` … `memcpy( awakeContactSim, contactSim, sizeof( b2ContactSim ) );`
  - The touching loop only asserts on `simFlags`; it does not modify them.
- **Warm start is per contact, with no global table** (`contact.c`, `b2UpdateContact`). It saves `b2Manifold oldManifold = contactSim->manifold`, then matches points with `if ( mp1->id == id2 ) { mp2->normalImpulse = mp1->normalImpulse; mp2->tangentImpulse = mp1->tangentImpulse;`.
- **Parallel collide, skip by construction** (`physics_world.c`, `b2Collide`):
  - Contacts are gathered only from the graph colours and `solverSets.data[b2_awakeSet].contactSims`.
  - Workers write per-contact `simFlags` and set bits in a per-worker `contactStateBitSet`.
  - A serial pass unions the bitsets (`b2InPlaceUnion`) and processes begin/end touch in contact-id order.
- **Trade-off.** Sleeping costs O(island) copies on each transition and nothing per step. The wake is exact, and impulses and cached features are kept.

### Box3D (erincatto/box3d, `main`)
- **The reuse state lives on the persistent contact** (`contact.h`, `b3Contact`): `b3ContactCache cache`, `b3Quat cachedRotationA; b3Quat cachedRotationB; b3Transform cachedRelativePose`, and flags `b3_contactRecycleFlag = 0x00000010` and `b3_relativeTransformValid = 0x00800000`.
- **Collide gathers awake contacts only** (`physics_world.c`): colour arrays plus `awakeSet->contactIndices`.
- **Recycle test**, gated on `b3_relativeTransformValid`: it compares the current relative pose with `cachedRelativePose` (and the rotations). A hit refreshes the separation only: `mp->separation = mp->baseSeparation + b3Dot( dp, normal ); mp->persisted = true;`.
- **Across sleep:** I found no reliable evidence of where, or whether, sleep or wake clears `b3_relativeTransformValid`.
- **Consequence (derived):** a sleeping contact is never visited, so its cached pose is exactly the pre-sleep one. If the bodies did not move, the first collide after wake is a recycle hit.
- **Warm start** (`contact.c`, `b3ComputeConvexManifold`): `if ( pt2->featureId == pt1->featureId ) { pt2->normalImpulse = pt1->normalImpulse; pt2->persisted = true;`.

### Bepu v2 (`PairCache_Activity.cs`, `PairCache.cs`)
- **Sleep:** `SleepTypeBatchPairs` stores each contact pair with `builder.Add(pool, Mapping.Keys[elementIndex], cache)` and records `InactiveSetIndex` and `InactivePairIndex`.
- **The cache holds** a `ConstraintHandle` and `FeatureId0..3`.
- **Wake:** `AwakenSet` re-inserts with `Mapping.AddUnsafely(pair.Pair, pair.Cache)`.
- **Impulses** stay with the constraints in `solver.Sets[setIndex]`, not in the pair cache.
- This came from a summarised fetch; the method names and the two calls were quoted.

### Rapier (`src/geometry/narrow_phase/contacts.rs`)
- Skipping happens before the per-pair loop, in `collect_pairs_to_update()`: "Only iterate on pairs involving at least one changed collider instead of the whole graph, which can be very large when most pairs are asleep."
- The same filtered list feeds both the serial loop and the parallel one.
- Skipped pairs' manifolds and solver data are left untouched: no code runs on them.

### Jolt
- `ContactListener.h`: "as soon as a body goes to sleep the contacts between that body and all other bodies will receive an OnContactRemoved callback".
- In boyko this is the rejected "drop and cold wake" (rev 2 D1). B1 measured its cost as 4.54e-2 m/s of sag against a 1e-2 bound (`frozen_island_warm_start.rs:35`).

## Comparative table

| aspect | Box2D v3 | Box3D | Bepu v2 | Rapier | Jolt | boyko rev 2 → rev 2.2 |
|---|---|---|---|---|---|---|
| Warm impulses across sleep | kept (whole sim memcpy) | kept (in the sim) | kept (inactive constraints) | untouched | dropped | B1 carry per point today → L11 record per manifold |
| Per-pair reuse / feature cache across sleep | kept (sim struct) | untouched (persistent `b3Contact`) | kept (sleeping subcache) | untouched | dropped | rev 2 keeps manifolds only → must also keep L9 records and tags |
| Non-touching pair state | kept (`b2_disabledSet`) | untouched | not established | untouched | dropped | rev 2 keeps none → L9 REC-without-manifold state is value-bearing |
| Where the skip happens | the gather excludes sleeping sets | same | sleeping pairs are not in the active mapping | a list is built before the loop | broadphase over active bodies | per-pair predicate in `np_chunk` (L5); Tree withholds under Sets |
| Serial commit after the parallel phase | bitset in contact-id order | — | — | — | — | axis commit in pair order (L5 D3) + mirrors |
| Lookup key on wake | contact id (persistent) | contact id | hash on the pair key | graph edge | — | t−1 ordinal via `prev_row` (rev 2.1 W2) |

## Key algorithms and techniques
- **Whole-record move on transitions.** Box2D and Bepu move one record per contact or pair: manifold, impulses, feature ids and flags. There is no per-step copy. That matches the ruling's "no copy of the sleeping store per step".
- **Persistent-id reuse state that never moves** (Box3D). It works because the contact id is persistent. boyko has no persistent pair id: U7 `PairCache` is refactor-last. So boyko must copy the state at move-in and look it up by a translated key at restore.
- **Skip before the per-pair kernel with the order preserved** (Box2D gather, Rapier candidate list, L5 §Sleeping). Output order comes from the pair order, and the serial phase after the join is ordered by a canonical index (Box2D's contact id; boyko's pair index `k`).

## Pitfalls
- **Dropping state that is not pose-only.** Under L9b, whether a pair has a manifold depends on the record as well as the poses (L9 review W2). A lost record can change the manifold count after wake, which triggers A4's wake-on-count-change (`resources.rs:3542-3548`). Jolt's drop gives cold wakes (B1's measured sag).
- **Two key functions for one pair.**
  - The warm key uses `RowRemap::pair`, which returns `None` on an order flip (`row_identity.rs:146-157`) and is kept by L11's Lemma W.
  - L9's record join survives a flip (L9 W2 ruling: only `REF_IS_B` flips).
  - Mixing the two changes the outcome on flip steps.
- **Stale tags.** A pair slot whose tag is not written survives two buffer swaps (L9 O5). Under Grid or AllPairs a held-skipped pair is exactly such a slot.

## Relevant academic works
- I found no academic work specific to this delta; engine source is the primary evidence.
- Earlier lever research cites Catto's "Simulation Islands" (box2d.org/posts/2023/10/simulation-islands/, in L10 `01-RESEARCH.md:49`) and "Determinism" (box2d.org/posts/2024/08/determinism/). I did not re-fetch either here.

## Applicability to boyko-engine: the delta

### (a) L11 per-manifold warm storage

**The code today (L11 replaces all of it)**
- Warm starting today uses a per-point hash table, `warm_read`/`warm_write` (`solver/colored.rs:1456-1459`).
- The freeze decision is made in `build_columns` (`:1648-1655`). It tags a frozen manifold `(u32::MAX, count)` and accumulates `frozen_points` (`:1637`, `:1653`, `:1728`).
- `store_and_swap` (`:3229-3267`) sizes the table by `cols.len() + frozen_points` (`:3237`) and calls `carry_frozen` (`:3282-3319`). That function looks each point up through `remap.manifold_pair(m)` with the read key and re-inserts it under the store key (`:3304-3314`).
- Rev 2's A6 and C3a were written against exactly this code.

**A1. Move-in warm capture**
- Rev 2 A6 looked kept points up in `warm_read` through `warm_remap`, as `carry_frozen` does, and stored them as `KeptWarm` (24 B per point).
- Under L11 (ruling O5, "the L10 mapping uses t−1's stream index"), the source is `recs_r[j]`, where `j` is the kept manifold's t−1 stream index (the entries of `graph(t−1).island(a)` that A2.4 already reads).
- **Timing (derived):**
  - `recs_r` holds step t−1's records only from t−1's swap until step t's store swap.
  - So the copy must happen in step t's solve, before the swap. Rev 2 already placed it there.
  - A2.4, which runs in the broadphase, must hand the list of `j` to the solve.
- `WarmRecord` holds no rows (L11 D1: rows live only in `keys`), so a kept record never needs remapping.
- The island was frozen at t−1, so `recs_r[j]` was written by L11 D3's carry. By Lemma W item 4 it equals Off's record.

**A2. Kept warm storage**
- It becomes one `WarmRecord` (64 B) per kept manifold, parallel to `HeldStore::kept`.
- `warm_of` becomes redundant: `HeldIsland` already has `kept_start`/`kept_len`, and there is one record per manifold (derived).
- Memory at J with everything held (arith.): 4,524 × 64 B ≈ 0.29 MB, against rev 2's ≈ 0.39 MB `kept_warm`.
- Future coupling: L8 changes `WarmRecord`'s value fields (L11 §L8). Kept records are opaque copies, so L10 would only change a size assertion (derived).

**A3. `held_warm` and `carry_hits`**
- Off's per-step D3 hits for a held manifold are constant after the first frozen step. The rev 2 review verified this for the table, and Lemma W item 4 carries it to records.
- So each record's `held_warm` contribution is the D2-routine hit count, computed once at capture.
- That equals `record.count` only when a manifold's fids are distinct, which is guaranteed "only by test" (L11 §Context).
- `carry_points += live_points` is unchanged (`colored.rs:1651-1654`).

**A4. Restore**
- Rev 2 inserted `restore_warm` into `warm_read`, rehashing into the idle buffer when needed. L11's mapping replaces that with a sorted `restore_rec` column: keys are ordinals in t−1 rows, and P-a searches it after `keys_r`. Keys are disjoint because held pairs are never in the t−1 stream, so no merge is needed.
- Constraints that follow from the rulings (derived):
  - **L11 W1:** the seed and the carry both go through the single D2 lookup routine, so `restore_rec` must be searched by that same routine. `Plan.src` also has to say which source it names (the `Plan` struct has `src: u32` plus flags).
  - **The key is the warm key**, `ord(RowRemap::manifold_pair(m))`, computed with the warm cursor's classification (`colored.rs:3465-3469`). A restored pair whose row order flipped therefore misses, as it does in Off.
  - **On a warm-cursor `Reset`, P-a must not consult `restore_rec`**, because Off misses every key.
- The narrowphase-side restore lookup is different. Rev 2.1 W2 keys it by the t−1 ordinal through `prev_row`, and L9 W2 makes it survive flips. So there are two keys (see Pitfalls).

**A5. The B1 carry for frozen manifolds that are not held**
- This covers Replay, CAND islands, and sensor, degenerate or hovering-neighbour islands.
- It becomes L11 D3's plan walk, which copies by fid from `recs_r[src]`.
- L11 C1 deletes `frozen_points`, `canonical` and `carry_frozen`.

**A6. C3a**
- The "need-sized `rebuild` + `len` + idle-buffer rehash" (`warm_start.rs:274-288`) is obsolete. L11's records are need-sized, and the table then serves only `SoftStepSolver`.
- The no-awake fast path stays (`colored.rs:3487-3503`; substep loop `:3547-3617`).
- Under L11 the fast path must still run P-a (write `keys_w`), the D3 carry, the swap and the stamp. Otherwise the next step's lookups miss for frozen manifolds still in the stream (derived).

**A7. E5 and E6 (warm clause)**
- "Same key set in the warm table" becomes: stream records ⊎ kept records ⊎ `restore_rec`, with disjoint keys and lookups equal to Off's by Lemma W plus E4.
- "Table bytes under Replay only" becomes "record bytes (`keys` + `recs`) under Replay only", as in L11's mapping. Under Sets the `mi` numbering excludes held manifolds, so record bytes differ.

### (b) L9's per-pair tag layout and records

**B1. One tag layout and one shared carry**
- Rev 2 D4's u8 tag (`axis:4 | SETTLED | BOX | SET | PUSHED`) becomes L9's u16 `PairTag`: bits 0–3 axis, 4 BOX, 5 PUSHED, 6 SET, 7 SETTLED, 8 REC, 9 SEP, 10 HIT, 11 SEPHIT, 12–15 `sep_axis`. The L9 review O7 found the bit orders differ, and the ruling set one shared layout.
- The shared carry (L9 amendment 1) is `ContactPairs::pairs_prev` plus `Manifolds::{pair_tag, pair_tag_prev}`. L10 keeps only `manifolds_prev` and `sensor_prev`, which saves 3 scratch ids.
- `pairs_prev` is stamped with the gather sequence (L9 OQ4 ruling).
- Swap timing from L9 D9:
  - `pairs` and `pairs_prev` are swapped at broadphase start, before the kind arm (`systems.rs:306`);
  - tags and records are swapped in the narrowphase prologue.
- So at L10's broadphase epilogue (A2.4 and A2.5), `pairs_prev`, `pair_tag` and `reuse` all still hold step t−1 (derived).

**B2. Tags for every pair, every step (L9 O5)**
- This includes pairs that Sets skips under Grid or AllPairs. Otherwise a stale REC tag survives two swaps.
- The value of the skip tag is not specified anywhere yet.

**B3. The kept store's unit**
- Rev 2 keeps manifolds only; the rev 2 review W2 notes that kept runs hold no pairs without a manifold. Under L9, held pairs can carry state with no manifold:
  - **REC with every point lifted.** L9 D6: it "emits no manifold, keeps its record". Dropping it is value-bearing. After the island is restored, Off evaluates `criterion(R, P')`, while Sets would do a cold miss. `refresh(R_old,P)` and `refresh(build(full(P)),P)` can differ in point count (L9 review W2). So Sets ≠ Off, and a spurious A4 wake is possible.
  - **SEP with a cached separating axis.** L9a is exact, so dropping it changes only cost and the SEPHIT/FULL counters.
- Box2D keeps exactly this class too (`b2_disabledSet`).
- Amendment 2 is worded "KeptManifold also keeps the pair's ReuseRecord and tag". That is keyed by manifold and does not cover REC pairs without a manifold (derived).
- **The count constraint.** Rev 2 D7's `island_len(i) = CSR count + held_len[i]` must count kept manifolds only, because `begin_step` keys on the island's manifold count (`resources.rs:3531-3549`) (derived).
- **Enumeration at move-in.** `graph(t−1).island(a)` lists manifolds only (rev 2 review OQ2). State per pair needs the island's t−1 pairs, i.e. a scan of `pairs_prev` plus tags with a `RowCls` lookup: O(P_prev) on move-in steps (derived).
- Memory at J with everything held (arith., upper bound): kept `ReuseRecord`s ≤ 4,524 × 128 B ≈ 0.58 MB, plus 2 B per held pair for tags.

**B4. An L9b-reused contact when its island freezes, and when it wakes**
- **Off (B1 path; the island is frozen but not held).** The narrowphase still visits the pair every step. At bit-equal poses L9-L1 gives a hit: tag REC|HIT, a 128 B `reuse_prev[j] → reuse[k]` copy, and a bit-constant `refresh(R,P)`. Feature ids are carried unchanged (`ReuseRecord::feature`), so the warm carry hits by fid.
- **Replay** copies manifold, record and tag (amendment 3). The copied tag must equal Off's REC|HIT even when step t−1 was a miss (review O7).
- **Sets (move-in).** Record, tag and warm record move in once; nothing is copied per step.
- **Move-in condition.** Amendment 4 treats a REC pair that *hit* at t−1 as settled.
  - L9-L1 and D7 also cover a REC pair that *missed* at t−1: its output at t equals its output at t−1, and only the HIT bit differs (derived). The ruling's condition is the stricter of the two.
  - Rev 2's "SET ⇒ PUSHED" test (M3/INV-A7b) cannot apply to REC pairs, because REC without PUSHED is legitimate under L9 D6.
  - INV-A7b must be restated over `box_box_classify`, which replaces `box_box_contact` at `box_box.rs:642-734`.
- **Wake with replay** (both endpoints resting). The source is the kept store, looked up by the t−1 ordinal (rev 2.1 W2, flip-surviving). The record goes into `reuse[k]` and the tag into `tag[k]`.
- **Wake with compute** (D1 member moved, D1a anchor moved, or D2 merge).
  - L9's join `j(k) = lower_bound(pairs_prev, key_prev(k))` cannot find a pair that was held or withheld at t−1. L9 then treats it as a cold miss, while Off would evaluate the record, so Sets ≠ Off (derived).
  - The kept record therefore has to be reachable from the compute path too. Two shapes:
    - merge the restored entries into `pairs_prev`/`tag_prev`/`reuse_prev` before the narrowphase (O(P) copies on restore steps);
    - or a second, read-only join source.
  - Both are read-only during L5's scope.
- **L9 W2's A4 arm** (support moved by less than τ_eff) under Sets: the anchor fails R3, D1a restores the island, and the pair is computed from its kept record, which is a hit. The count is unchanged, so there is no wake, as in Off. Without the kept record: a cold miss and a possible spurious wake.

**B5. The axis mirror (E5)**
- Under L9 W1, Off keeps the axis table keyed for hit pairs on every step where the key set changes (prefetched, cleared, grown), taking the axis from the tag. L9 D10: on other steps a hit reads and writes nothing.
- So the mirror set is "held pairs that Off would `set` this step": REC hits, plus non-REC pairs that made a contact (SET). The axis comes from the kept tag's bits 0–3.
- SEP pairs are not re-keyed in Off, so they are not mirrored.
- Whether an all-lifted REC hit re-keys is L9's implementation choice. The mirror must use the identical predicate (derived).
- The table accessors this touches are unchanged in the tree: `axis_cache.rs:261-281` (`begin_frame` clear and grow), `:319-354` (`set`), `:367-388` (`begin_frame_synced(pairs, …)` calls `begin_frame(pairs.len())`). Rev 2 needs P_logical and `cleared`; L9 W1 needs cleared/grown. Both edit this one signature.
- The doc at `axis_cache.rs:62-73` ("a hint picks the contact, never whether there is one") becomes false for REC pairs under L9b (L9 review W2).

**B6. Jumpers**
- L9 keeps `jumper_bits: ScratchColumn<u64>` and rev 2 sets `RowCls.JUMPER`: two encodings of `stage2_rows`, which does not exist in the tree yet (bp T5).

**B7. `row_frames`**
- Ruling O3 says the fill skips rows nobody reads. Under Sets, held rows are unread.
- Arith., from L9: about 12 µs for all rows at J, against rev 2's 0.11–0.23 ms floor.

**B8. The fast predicate**
- L9 D8 is a pure function of `BodyState`, so R3 implies the same class.
- Frozen bodies keep their at-rest velocity (rev 2 D13). A held pair can therefore be "fast": on today's path, with no REC, and following rev 2's SETTLED rule.

### (c) L5's parallel narrowphase

**C1. Where the skip sits**
- L5 rev 1 §Sleeping: "a per-pair predicate that goes in front of `collide_pair` on both paths and keeps the order."
- In `np_chunk`, for each k in `[lo, hi)` (derived):
  - a HELD endpoint on a non-sensor pair: no stage write, so neither run advances; `commit[k] = AXIS_NONE`; `tag[k]` written (B2);
  - a replay writes into the chunk's out-run, or its sensor run for a `sensor_prev` source.
- Lemma 3 holds because the routing predicate is the serial one.
- The inline path (`lanes < 2`, C < 2, flag off, no pool) uses the same function (L5 D6).

**C2. Inputs read-only during the scope**
- `RowCls`, the kept store or restore index, and `pairs_prev`, `tag_prev`, `reuse_prev`, `manifolds_prev`, `sensor_prev`.
- Per-chunk join starts:
  - L9 D9: "the join start per chunk = lower_bound";
  - L10's manifold replay joins `manifolds_prev` and `sensor_prev` by key. That needs its own per-chunk `lower_bound`, because manifold ordinals are not pair slots. L9 D9 rejected manifold-ordinal indexing for the same reason.

**C3. Mirrors**
- They stay serial and after `begin_frame` (rev 1 W3).
- Held keys always name a held row and stream keys never do. With linear probing and no deletions, inserting a held key cannot change a stream key's probe result, so L5's Lemma 1 extends to mirrors placed before the scope (derived).
- On prefetched steps the hints come from `remapped`, so where the mirror sits does not matter.
- Table bytes are W-invariant *within* a mode. They differ between Off and Sets (rev 2 already says "layout differs; lookups do not").

**C4. E8 is obsolete for the narrowphase arms**
- W-invariance becomes L5's theorem extended: each pair's output is a pure function of read-only inputs, writes are per slot, runs are joined in order and the commit is serial.
- Tree Borrows: no slice over stage, commit, tag or reuse until after the join (L5 §Tree Borrows).

**C5. Chunk count**
- `C = min(lanes·6, n/128)` must use the internal stream slice. Rev 2 D6 makes `ContactPairs::pairs()` a logical `PairsView`; today it is `resources.rs:597`, a slice.
- With the Tree under Sets and everything held, n → 0, so the narrowphase runs inline and opens no scope.
- L5's gates that assume "+1 dispatch per step on W ≥ 2 flag-on arms" (G-L5-3 non-vacuity) and S1c = 134 scopes (G-L5-5) become dependent on how much is held.
- Load balance. L9 estimates per-pair costs of 35–60 ns for SEP, 60–100 ns for a hit and 380–480 ns for a full compute. Rev 2 estimates a held skip at 1–2 ns and a replay at 152 B plus 128 B of copying. The ×6 oversubscription was sized for the 53 % early-out (L5 D4). No measurement exists for mixed held/awake scenes.

**C6. Statistics**
- `SleepSkipStats` is public and always on. It needs L5 D7's per-chunk meta (today it packs `n_out << 32 | n_sen`) or a serial scan of the tags after the join.
- L9's counters are armed-only and come from the tags.

**C7. The SDF stage**
- L5 does not build a parallel SDF stage, so rev 2 A4 (skip held rows) stays serial.

## File:line list: current sites the delta touches (D:/wt/joltab)

| file | lines | item |
|---|---|---|
| `solver/colored.rs` | `:1443-1481` (fields: `warm_read`/`warm_write` `:1456-1459`, `frozen_points` `:1465`, `warm_cursor` `:1475`) | A1–A6: L11 replaces these; rev 2's A6 insert/rehash sites vanish |
| | `:1587-1729` (freeze and carry tag `:1648-1655`; stats `:1691-1700`) | A3, A5; M-V4 stays |
| | `:1830-1843` `point_keys`; `:1863-1871` `manifold_frozen` | the warm key (the flip rule reaches it through `manifold_pair`) |
| | `:3229-3267` `store_and_swap`; `:3282-3319` `carry_frozen` | deleted by L11 D3; rev 2's A6 capture re-targets `recs_r[j]` |
| | `:3435-3441` early return; `:3459-3472` warm classification; `:3487-3503` / `:3547-3617` fast path; `:3627-3630` store | A4 (classification), A6 (fast path keeps P-a/D3/swap) |
| `solver/warm_start.rs` | `:274-288` `rebuild`; `:342` `insert`; `:378` `get` | rev 2 C3a edit dropped |
| `row_identity.rs` | `:113-123`, `:146-157` (flip → `None`), `:163-169` `manifold_pair`, `:214-245` `RemapCursor` | A4 key; B1 single cursor; `stage2_rows` absent |
| `systems.rs` | `:297-349` (swap point before `:306`; order assert `:344-347`) | B1 swap; strict order over the merged view |
| | `:375-478` (`begin_frame_synced` `:391-392`; loop `:400-467`; box arm `:437-446`; routing `:455-465`; counters `:472-477`) | C1 skip, B2 tags, B5 re-key and mirror, C5/C6 counters |
| `narrowphase/axis_cache.rs` | `:62-73`, `:261-281`, `:319-354`, `:367-388`, `:394-422`, `:435-443` | B5, C3 |
| `resources.rs` | `:562-611` `ContactPairs` (`pairs()` `:597`); `:2269-2327` `Manifolds`; `:2345` public `manifolds_build`; `:3109-3159` `IslandSleep`; `:3531-3549` island key = manifold count | B1, B3 |
| `scratch_ids.rs` | `:229`, `:309-311` warm-table ids; `:729`, `:734-739` region floor; `:889`, `:937-939` narrowphase cohort | ids: −3 from the shared carry; the rev 2 warm ids are replaced |
| `plugin.rs` | `:629-631` bp/np; `:663` graph; `:682` solve | coloured variants |
| `tests/frozen_island_warm_start.rs` | tests at `:642`, `:663`, `:692`, `:737`, `:908` (row move), `:1255` (W); mutation list `:41-51` | A1–A4 gates |

## Gates and mutations that change
- **Renamed onto records:** M11 ("no `restore_warm`") becomes "no `restore_rec`"; M20 becomes "`restore_rec` keyed in t rows".
- **New mutations, each needing a scene that can turn red:**
  - move-in warm capture reads `recs_r` after the swap, or at step t's index;
  - `restore_rec` keyed by the flip-surviving pair key (needs a held-island order-flip arm in S3);
  - the kept store drops `ReuseRecord`s (D1a anchor moved by less than τ_eff);
  - the kept store drops REC pairs that have no manifold (knife-edge pile);
  - Replay copies the t−1 tag verbatim (needs a crate-internal tag compare; the counters are armed-only);
  - `held_len` counts entries without a manifold (a spurious A4 wake, the M12 class);
  - the mirror excludes REC hits, or includes SEP pairs (Rows-step and grow-clear arms);
  - a held-skipped slot leaves `tag[k]` unwritten;
  - the skip predicate runs on the serial path only;
  - a restored pair that is computed does a cold miss (no second join source);
  - the chunk count is taken from the logical `pairs().len()`;
  - the replay join start per chunk is `lo`, not `lower_bound`.
- **May be unable to fail:** "`held_warm` = Σ `record.count`" can only go red with duplicate fids inside a manifold, which pipeline streams do not produce (L11 §Context).
- **Obsolete:**
  - the unit test "the warm rehash preserves every lookup at load ≤ 0.5";
  - the census mutation "`Vec::with_capacity(1)` in the warm rehash". Its replacement goes in the `restore_rec` build and in the kept-record capture.
- **Comparator changes:**
  - "warm lookups for every logical point key" now needs a crate-internal D2 lookup over stream records ⊎ kept records;
  - "table bytes under Replay" becomes "record bytes under Replay".
- **Scene matrix:** S1–S3 × {`parallel_narrowphase` on, W ∈ {1, 8}; {2, 4, 16} in release} × `contact_reuse` on. S3 gains:
  - a held-island row-order flip;
  - a support moved by less than τ_eff (L9 W2);
  - a pile containing REC-all-lifted and SEP pairs;
  - a wake after a Rows-step grow-clear.
- **Runner closure:** L9's `full + reused + sep_hits + non_box == pairs` needs the replayed and held-skipped classes. L5's recomputed `chunk_count` takes the stream length.
- **Baseline for G-TW:** Off now includes L9 and L11 (and L5 at W=8), so rev 2's predicted Δ must be re-derived.
  - L9 predicts about −2 ms of the 5.33 ms J-Son tail before L10.
  - L11 predicts the R-S store falling from 0.32–0.37 ms to 0.05–0.10 ms (both arith. in their designs).

## Rev 2 items that L11 or L9 make obsolete
- D4's u8 tag, and L10's own `pairs_prev`, `pair_tag` and `pair_tag_prev`.
- `KeptWarm`, `warm_of`, `restore_warm: ScratchColumn<WarmEntry>`, and `held_warm` as a sum of entries.
- C3a's rebuild, `len` and rehash (the fast path stays); A6's insert and rehash.
- A6's capture "exactly as `carry_frozen` does".
- The E5 warm-table clause; E8's "all L10 code is serial".
- The claim "kept runs hold no no-manifold pairs" (and the rev 2 review's OQ2 dependence on INV-A7b for REC pairs).
- The ≈0.39 MB `kept_warm` memory row.

## Open questions for the architect
1. **The kept unit:** one entry per kept manifold (the ruling's wording) or per held pair with state? B3 shows REC state without a manifold is value-bearing.
2. **Where kept `WarmRecord`s and kept `ReuseRecord`s live:** `Manifolds::held`, `SleepSets`, or solver-owned? This also decides the solve stage's access set.
3. **The join source for restored pairs:** merge into the t−1 carry, or a second read-only source?
4. **The skip tag's value, and one shared function** for Off's re-key predicate and the mirror predicate.
5. **One cursor** (L9 `carry_cursor` or the L10 cursor) and **one jumper encoding**?
6. **Amendment 4 (HIT at t−1) versus L9-L1**, which also covers a miss at t−1 given R3.
7. **Stats from chunks:** per-chunk meta, or a scan of the tags after the join?
8. **Whether Replay (C2b) still clears its build-if rule.** Under L9, Off's frozen pairs already hit at an estimated 60–100 ns.

## Sources
[1] https://raw.githubusercontent.com/erincatto/box2d/main/src/solver_set.c — `b2TrySleepIsland`/`b2WakeSolverSet`: whole-sim memcpy; non-touching contacts to and from `b2_disabledSet` (quoted verbatim)
[2] https://raw.githubusercontent.com/erincatto/box2d/main/src/contact.c — `b2UpdateContact` per-contact id matching; `b2GetContactSim`
[3] https://raw.githubusercontent.com/erincatto/box2d/main/src/physics_world.c — `b2Collide`: awake-only gather, per-worker bitset, serial processing in contact-id order
[4] https://raw.githubusercontent.com/erincatto/box3d/main/src/contact.h — `b3Contact` recycle fields and flags
[5] https://raw.githubusercontent.com/erincatto/box3d/main/src/physics_world.c — the recycle test and the awake-only gather
[6] https://raw.githubusercontent.com/erincatto/box3d/main/src/contact.c — impulse carry by feature id
[7] https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/CollisionDetection/PairCache_Activity.cs — `SleepTypeBatchPairs`/`AwakenSet`
[8] https://raw.githubusercontent.com/bepu/bepuphysics2/master/BepuPhysics/CollisionDetection/PairCache.cs — `ConstraintCache` (handle plus feature ids)
[9] https://raw.githubusercontent.com/dimforge/rapier/master/src/geometry/narrow_phase/contacts.rs — `collect_pairs_to_update`
[10] https://raw.githubusercontent.com/jrouwe/JoltPhysics/master/Jolt/Physics/Collision/ContactListener.h — `OnContactRemoved` on sleep
[11] `D:/wt/joltab/docs/physics/perf-campaign/levers/{00-RULINGS.md, L10-sleeping/04-DESIGN-REV2.md, L10-sleeping/05-REVIEW-OF-REV2.md, L11-solve-setup/02-DESIGN-REV1.md, L11-solve-setup/03-REVIEW-OF-REV1.md, L9-contact-reuse/02-DESIGN-REV1.md, L9-contact-reuse/03-REVIEW-OF-REV1.md, L5-narrowphase/02-DESIGN-REV1.md}`, `D:/wt/joltab/docs/physics/perf-campaign/00-RULINGS.md` and `D:/wt/joltab/docs/measurements/2026-09-19-physics-p0/ANALYSIS.md` — design base and rulings
[12] `D:/wt/joltab/crates/boyko_physics/src/{solver/colored.rs, solver/warm_start.rs, row_identity.rs, systems.rs, narrowphase/axis_cache.rs, resources.rs, scratch_ids.rs, plugin.rs}`, `D:/wt/joltab/crates/boyko_physics/tests/frozen_island_warm_start.rs`, and `D:/wt/l5np/crates/boyko_physics/src/{resources.rs, solver/colored.rs, broadphase_policy.rs}` — tree facts
