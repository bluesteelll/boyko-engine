# Lever designs after P0 — orchestrator rulings (2026-09-19)

Each lever folder holds its research, its design (rev 1) and the architecture review of rev 1. This file
records what was decided on each review. The P0 measurement behind all three is
`docs/measurements/2026-09-19-physics-p0/ANALYSIS.md`; the campaign rulings are `../00-RULINGS.md`.

## L5 — parallel narrowphase (with L4 and L2 as the lane's first commits): APPROVED with changes

The review found no blocking remark and no determinism hole. Closed by ruling; the implementing developer
applies these to rev 1:
- **W1:** the algorithm matches D4's inline conditions, including `lanes < 2`; a narrowphase twin of
  G-L4-1 (1 worker, flag on: `narrowphase_dispatches` delta = 0) is added, with the mutation "drop the lanes
  term" required red; the runner's copy of `chunk_count` carries the same term.
- **W2:** the J-P1 runner row is retired in C1; ω₁ comes from a zero-work spawn/join microbench from then
  on (the campaign rulings allow it).
- **W3:** the compared axis-table state is `(key, axis)` per slot plus `occupied`, hashed by field — never
  raw bytes (the entry has padding). The lemmas speak of "table state".
- **Optional, all adopted:** O1 (open the dispatch zone after the `c < 2` decision), O2 (assert the pool
  workers share one MXCSR with each other and the default; no spawner-side FTZ mutation), O3 (the
  load-clear / grow non-vacuity items get a crate-internal observable), O4 (`--expect-pose` from C0/C1
  on), O5 (do not reach for the scratch-id fallback; the tiling assert stays), O6 (the broadphase order
  assert becomes strict `<`), O7 (the census citations and the debug S1c pin move too).
- **Open question 1, the +1 scope per step (D8):** accepted. A `pool.scope` costs one boxed `ScopeShared`
  and a block chunk; the solver already pays 133 per step and the census pins them exactly. The narrowphase
  adds one, pinned in the same census, until the allocator campaign removes scope allocation for all of
  them (KC-05, rung D-M2). This is a ruled exception to "no allocation on the hot path", not a precedent
  for new per-entity allocation.
- **Open question 2:** a parallel compaction is built only under the campaign build-if rule (≥ 5 % of T(W)
  and above the SE bar), not at the plan's 1 %.
- **Open question 3:** L2 is exempt from the end-to-end T(W) gate, because no measured row runs `Auto`; its
  gates are the structural ones in the plan.

## Broadphase redesign (`BroadphaseKind::Tree`): REVISE (rev 2) before implementation

No blocking remark; the exactness and determinism core holds (only the exact AllPairs set is bit-identical,
because the pair count sizes the axis cache). Five Important remarks are substantive enough for a revision:
- **W1:** the sleep hint is gated on `cfg.sleeping` and on the mask covering the current N; a structural
  gate `sleeper_rebuilds == 0` on a sleeping-off J run, with "hint not gated" required red.
- **W2:** a complete trigger list for the persistent static and sleeper sets; the cursor stamp; upper-bound
  gates (`static_rebuilds == 1` over 600 J steps; a bound after R-S freezes); rebuild costs at 1,240 / 10k /
  100k; the `row_identity_churn` arms in the gates; and a decision whether the bit+class verify is the sole
  correctness authority (then the remap trigger is dropped and M5 re-specified). Churn must not dissolve
  the sets in a spawn-heavy game.
- **W3:** the parallel query wave (C2) is gated on its own (serial tree against parallel tree, same binary,
  W=8, SE gate) and built only if it clears the build-if rule at that point; the design's own figures put it
  near 1 % of T(8), so expect it to be deferred.
- **W4:** negative radii in G1 (both negative, mixed), so M2 can go red.
- **W5:** per-path structural expectations for every new zone and counter, in the plan and in the runner's
  `check_step`.
- Optional O1–O3 adopted (the allocation wording; a per-row fallback for non-finite rows instead of a
  whole-step AllPairs; the scratch-id cohort placement as the review computed it).

## L10 — sleeping on by default, and the frozen-pair skip: REVISE (rev 2) after the broadphase rev 2

- **B1, the blocking remark — ruling:** the public views stay LOGICAL. `ContactPairs`, `Manifolds`,
  `ConstraintGraph`'s island queries, `IslandSleep::is_island_frozen` and `WarmSeedStats` report sleeping
  contacts and islands exactly as they do with sleeping off, the way Box2D v3 and Rapier expose sleeping
  contacts. The internal solve consumes only the awake stream; the logical views are provided WITHOUT
  copying the sleeping store every step (iterate the awake stream and the sleeping store together, or keep
  a logical index). No test is loosened: the suites that read these views keep their assertions.
- **W1:** the coloring/write-guard predicate pair (`contact.rs:20-40`) stays identical by construction, or
  the invariant is enforced in release.
- **W2:** every runtime transition is defined (sleeping on/off, each mode change), "resting" requires the
  stream cursor, and the runtime toggles join S3.
- **W3:** the mirrored axis `set`s for clean islands run after `begin_frame`, and a Rows-step grow-clear
  while an island is clean joins S3.
- **W4:** S1–S3 run under both broadphase kinds with the parallel emit, plus an SDF pile with a mid-run
  field edit and a coupled soft body.
- **W5:** a census scene that wakes, re-freezes and churns rows after warm-up, and a mutation in the restore
  path.
- Optional O1–O5 adopted.
- Because L10's frozen-pair skip in the broadphase is the broadphase design's C5, rev 2 of L10 is written
  against the broadphase rev 2.

## Rev 2 (2026-09-19): both designs CLOSED by ruling

Both second reviews found no blocking remark (broadphase: 2 Important; L10: 3 Important), each with an
unambiguous "needed". The designs are closed; the implementing lane folds these in as rev 2.1 before its
first commit. Order of lanes: L5 (approved above) → broadphase → L10.

**Broadphase rev 2** (`broadphase/04-DESIGN-REV2.md`, review `05-REVIEW-OF-REV2.md`):
- **W1 (the patch rule cascades on a high jumper that is the min endpoint of ≥ 2 entries):** the keep test
  must not keep an entry with a non-monotone endpoint (jumper rows identified from `prev_row` during the
  verify); D3.5 states the true bound; a G4 maintenance arm at 10k / 100k moves one member to the top row
  with ≥ 2 higher-row partners under the same 2× stop rule; a unit test pins `a` for that case.
- **W2:** anchor A is defined over the dynamic rows (or "Q is empty") and its existence is asserted
  (A ≤ 300); a toggle-off script (sleeping on until frozen, then off with no row change) asserts
  Δ`hint_candidates` == 0 and Δ`sleeper_rebuilds` == 0, with "drop the `cfg.sleeping` term" shown red; a void
  in a cargo test is a red, never a pass.
- Optional remarks adopted where cheap.

**L10 rev 2** (`L10-sleeping/04-DESIGN-REV2.md`, review `05-REVIEW-OF-REV2.md`):
- **W1:** the Tree's hint is PRE_HELD minus the prologue restore list (or `release` runs for every restore on
  a Tree step); a debug assertion of Invariant V over `withheld` after the kind arm; the mutation
  "prologue-restored rows left in the hint".
- **W2:** restored runs are looked up by a key translation does not change (the t−1 ordinal through
  `prev_row`, or local-index pairs), or every pair of a jumper-restored record is recomputed; the A2.5
  filter runs before the A2.4 merge; the mutation "restored run searched by translated ordinal".
- **W3:** the classify cost is bounded (R3 only for rows a record, a CAND island, the D2 scan or the SL list
  can reach; skip the pass when nothing is held or CAND; or a narrower compare for static rows); a
  static-heavy "not claimed slower" arm (J plus 10k statics).
- Optional remarks adopted where cheap.

## Per-contact parity (owner, 2026-09-19): L9 and L11 designed and CLOSED by ruling

The owner set the goal: parity with Jolt per contact, not only per step. Per manifold at W=1 the solve is
already at parity (boyko with simd 1.24 µs against Jolt 1.30–1.49 µs); the gap is collision detection
(broadphase 0.465 + narrowphase 0.695 µs per manifold) and the serial solve setup (0.425 µs). Two new designs,
each with research and review (no blocking remark in either):

**L9 — contact reuse** (`L9-contact-reuse/`): L9a exact (early SAT exit + a cached separating axis, bit-identical);
L9b reuses a touching box pair's features within a tolerance and refreshes separation from the current poses
(value-changing, owner-approved). No "no contact" caching and no speculative margin (it would inflate the
manifold count). Rulings:
- **Build-if (review OQ1):** the owner's per-contact goal overrides the W=8 build-if that §9 applied; L9 is
  built and gated on its W=1 and per-contact gains (L9a gets a realized-gain rule on J-D, O6).
- **W1:** the axis-hint table stays keyed for hit pairs on every step where the key set changes (prefetched,
  cleared, grown) — the tag carries the axis through the row-translated join — so hysteresis survives and
  L10's E5 mirror stays equal to Off.
- **W2:** the spurious-wake path is CLOSED: records survive a row-order flip (the pose check is by the larger
  and smaller body; only `REF_IS_B` flips); a box-pile arm moving the support by less than τ_eff joins the A4
  tests so "all points lifted ⇒ no manifold ⇒ wake" can go red.
- Optional O1–O7 adopted (τ_eff also clamped by the smallest half-extent; record memory stated; the
  `row_frames` fill skips unread rows or runs in the parallel phase; the gate fixes — slab as body B, M-c3
  re-specified, tolerance scaled by the incident body's extent; tags written for every pair kind every step;
  one tag layout shared with L10). The Rapier description is corrected (parry keeps local normals, cos 1°,
  1 mm). `pairs_prev` is stamped with the gather sequence (OQ4).

**L11 — the solve setup per contact** (`L11-solve-setup/`): warm impulses stored per manifold (no per-point hash
probe/insert), cohort-shaped constraint blocks instead of the 26-column push, SIMD warm apply; bit-identical;
predicted setup 0.425 → 0.10–0.21 µs per manifold. Rulings:
- **W1:** `plan` names the whole equal-key run; the P-c seed and the D3 carry go through the single lookup
  routine D2 specifies; the mutation "fill/carry scan only the last equal-key record" is recorded red.
- Optional O1–O5 adopted (the SIMD warm-apply mask is `active ∧ movable`, with a −0.0 scene; lanes ≥ nlanes
  read no body row, with a Miri multi-thread case; the restitution predicate is `!(e <= 0.0)`; the test-port
  inventory lists what is deleted; the L10 mapping uses t−1's stream index).
- **OQ1/OQ2:** measure the C1 share and the kernel's gather/scatter share before the C2/C3 rewrites.
- **Ruling after C2 (2026-09-21, the census adjudication):** the S1c release pins moved with C2 — chunk 230 → 134,
  dispatch MAX 365 → 269, scope unmoved, the debug pins unmoved — because the colour task's cell shrank with the
  view it captures (280 → 120 B; 34 cells per chunk, not 14) and no colour scope needs a second block any more.
  This is the census's own re-pin case (the 17 × 256-step long run against a C1 control, the per-frame structural
  assertion, the attribution binary's A/B arm, then the envelope with no headroom either way), NOT a widening; the
  design's G7 sentence "scope/chunk pins unchanged (C1–C3)" is corrected in place. Row D of the attribution binary
  is not loosened: it pins `chunk == scope` per frame at W = 2/4/8 and keeps its strict lane-growth assertion where
  the model predicts growth — on the scalar cut walk (`simd_solve = false`) at W=8 against W=4 — because on the
  shipped kernel a colour spawns at most 32 tasks at any lane count (the ruling's "48 at W=8" was the ceiling
  `lanes × CHUNKS_PER_WORKER`, not the count the cohort-snapped cut walk produces).
- **G9 on C2 (2026-09-22, window 4b; `docs/measurements/2026-09-22-l11-solve-setup/`, queue section 11):** C1+C2 against C0 (`f8873aae` against `146a1125`), K=12 interleaved under the P0 protocol, passes every realized-gain gate the design set on this tip — T(1) `J-As` −1.996 ms (10.693 → 8.697; bar −0.81), T(8) −1.103 (5.342 → 4.240; bar −0.61), solve_build −0.573 / −0.542 at W=1 / 8 (bar −0.29), store −0.141 / −0.134 (bar −0.11); wide colours −27.7 % / −28.5 % and cfg-A faster at every W, so both "not claimed slower" gates hold; the setup 1.643 → 0.832 ms, serial at both W; 0.885× / 1.650× Jolt v5.6.0 at W=1 / 8 (1.69× / 3.07× per manifold); poses and counters bit-identical on both binaries. **C3 still owes the warm_apply gate (−0.19 ms): the tip reads −0.092 from C1's merge-join, 0.519 ms against C3's 0.15–0.30 target, and the design's setup band (0.44–0.93) is met at 0.832 only with C3 outstanding;** C4's in-binary A/B at W=8 stays conditional on D10 (the setup is 20 % of the tip's 4.24 ms step). Facts for the design, not reds: on the all-frozen `R-S` the tip's solve_build is +16 µs per step and its store, 0.128 ms, sits above the predicted 0.05–0.10.
- **L12 is commissioned** (the effective-mass cache per inertia epoch: 5 of 12 sweeps compute, 7 load;
  bit-identical) as its own design after L11.

**Lane order (ruled, replaces the order above):** L5 (in flight) → then, in parallel because their files are
disjoint, the tree broadphase and L11 → L9 (after L5, which owns the narrowphase system) → L10 (rev 2.2: it
adopts L11's per-manifold warm storage and L9's tag layout) → L8 (friction per patch, on L11's block layout)
→ L12. Each lane is gated end to end and per contact under the P0 protocol.

## L8 (friction per patch) and L12 (the effective-mass cache): designed and CLOSED by ruling (2026-09-19)

Both reviews found no blocking remark (L8: 5 Important; L12: 4 Important). Each design is closed; the implementing
lane folds these items in as rev 1.1 before its first commit.

**L8 — friction per patch** (`L8-patch-friction/`; value-changing, owner-approved D2). Two tangent rows with a
circular clamp at μΣλn and a twist row clamped at μΣd_iλn_i at the patch centre, on L11's block layout — the model
Jolt adopted in v5.6.0 (#2039), Box3D, Bepu and Rapier's default. `SoftStepSolver` stays the per-point Coulomb
reference. Rulings:
- **W1:** every row that GATES the lever runs with `--sleeping off` pinned explicitly; sleeping-on rows are
  witnesses. Before C0 a contact-set-robust pass rule is registered: the primary claim is the solve timed on one
  recorded contact stream in both binaries (`solve_colored` is public), with per-sweep cost normalised by points and
  by manifolds, each with its own SE. The end-to-end T(W) row is a secondary witness, because the lever moves the
  trajectory and so the contact set (D1 moved it 12 %).
- **W2:** the new counters stay off `WarmSeedStats` (a colored-solver-only accessor); G-S6 keeps its literal form,
  no diff in `soft_step.rs`.
- **W3:** a cohort in which no lane is multi-point skips the patch step uniformly (masked ops change no value, so
  G-P5 holds), and a sphere-pile row is gated "not claimed slower".
- **W4:** in every G-P gate the colored arm is the gate and the reference arm is a receipt; if the reference misses
  a bound it is reported, and the colored bound is never adjusted to match it.
- **W5:** pre-registered bands on the A7-R2, G8 and R-S freeze steps (confirm / report / STOP, as A7-R1).
- **O1:** `vn0` stays cold, outside the RankBlock, so the slot is L12's. O2–O7 adopted (the C1 digest and the
  multi-point pins join the red set; the FMA census covers `contact.rs`; `rc` blended by `present`; wording against
  L10's `SleepSkip::Off`; A7-R1 reported against the C0 base reading; G-P4's stop time is linear AND angular rest).
- **Review OQ1:** G-P2's band is registered at C0 from the base run, before any value change, and the registration
  is the gate; a base reading outside the analytic [25, 33] is a finding, not a band change.
- **Not in L8:** friction skipped in bias sweeps (Box2D main, Box3D) is a separate value change; it is a later
  candidate (L8b), priced on its own.

**L12 — effective masses cached per inertia epoch** (`L12-mass-cache/`; bit-identical). Rulings:
- **W1, the build-if:** the per-contact override written for L9 extends to L12 — the owner's goal is parity per
  contact and "squeeze maximum performance" — so the bar is the SE bar at W=1 per contact, not 5 % of T(W).
- **W2:** T1's control arm has the parent's kernel shape (no store, no column); rolling back means reverting C1,
  never a flag.
- **W3:** the C0 decision uses a bench-only upper-bound probe (masses not recomputed on load sweeps; a value-changing
  probe is allowed in a bench); if even that bound does not clear the SE bar, L12 closes at C0 without C1.
- **W4:** every W>1 arm asserts that dispatch happened on a scene that dispatches (widest colour ≥ 256 and
  n_chunks ≥ 2, or a scope/chunk delta > 0), and at least one cohort's compute and load sweeps run in different
  tasks.
- O1–O5 adopted (a control where inertia cannot change; G1 replays a recorded event trace; D6's coverage stated
  truly; a census of mutable access sites; c′_R = Rl_off/8; the reserve formula in `MASS_LANES_PER_*` terms).
- **Review OQs:** sleeping is pinned off for T1/T2; the per-contact denominator is solved (awake) manifolds, not
  L10's logical view; L12's layout is fixed after L8 closes (L8 owns the friction-mass choice); if T2 fails its
  claim with C1 landed, C1 is reverted.

## L10 rev 2.2 (re-based on L11, L9 and L5): REVISE → rev 2.3 CLOSED by ruling (2026-09-21)

`L10-sleeping/06-DESIGN-REV2.2.md` answers the three re-basing questions (the B1 carry and restore path on L11's
per-manifold records; coexistence with L9's tags and records; the frozen-pair skip inside L5's chunked emit). The
first review (`07`) was taken on the design's last text block only — the architect's answer arrived in two
messages and only the second was passed on; the complete text was restored from the agent transcript and
re-reviewed. That review (`07-REVIEW-OF-REV2.2.md`) found 1 blocking remark — the `row_frames` fill skips HELD rows
while sensor pairs with a HELD endpoint are still computed, so after a row shift a box sensor over a held pile reads
another body's frame — and 5 important ones (no W ≥ 2 sleeping-on census arm; sensor pairs in the kept unit; `dt`
missing from the sleep epoch although L9 reads it; REC admitted on a "exactly zero" quaternion claim that
`Quat::mul` does not satisfy; transitions into Off). Revision 2.3, a delta, is being written against those rulings.

**Rev 2.3** (`L10-sleeping/08-DESIGN-REV2.3.md`, a delta to 06) closes every remark of `07` — the reviewer
verified each answer against the tree and the arithmetic (`09-REVIEW-OF-REV2.3.md`: 0 blocking, 4 important,
5 optional, 3 open questions). No blocking remark remains, so L10 is closed by ruling; the implementing developer
applies these to 06 + 08:
- **W1 (S8b's scope pin):** at W=4 the colored solver (`colored.rs:215`, one scope per wide colour per pass) and the
  Grid-parallel emit (`resources.rs:1982`, `:2074`) open scopes the formula `S8 + [chunk_count ≥ 2]` does not count, so
  the exact count is red on correct code. S8b pins in L5 C4's structural form on BOTH arms (`scope_delta −
  Δnp_dispatches − Δbp_dispatches ≡ 0 mod passes`, or the measured envelope, as G-L5-5 does); the Tree arm
  additionally asserts `widest < MIN` on every step and pins the exact count there. The 0-heap-beyond-scopes pin
  stays exact on both arms.
- **W2 (jumper pairs on the Restore route):** the Restore route reuses L9's `PairJoin` (the monotone cursor plus the
  `jumper_bits` binary search) over `restore_idx.keys`; a plain cursor is not an implementation of D1. Mutation:
  "jumper pair merged on the restore cursor" (L9 M-c2's twin), red via a swap-remove despawn of a held member's
  tail-row neighbour, joins the S3 table.
- **W3 (arm 9 is vacuous in a gravity pile):** with τ = 0 every pair with residual velocity is fast and never REC, so
  the arm exercised M3′ on zero REC pairs. Arm 9 is rebuilt at exactly-zero relative velocity (gravity off, bodies
  placed at rest) and counts REC pairs admitted with HIT = 0 ∧ SETTLED = 1; a count of 0 is a void, and a void is red.
- **W4 (`RowCls` on a Sets step with nothing held):** A1.3 rewrites every row's flags on every step L10 runs — the
  rev 2 W3 option "skip the pass when nothing is held or CAND" is withdrawn (a skipped pass would have to zero
  `row_cls`, which is the same pass with a constant). Mutation: "flags not rewritten on a nothing-held step", red on
  the step after any restore arm's wake.
- **O-1..O-5 adopted:** the Lemma 2 amendment says the held pair's slot is left to the mirror (D-C's argument, not
  Lemma 2's); N32 becomes a debug spec assert (`len == 0` on a Reset flush) because the described mutation cannot
  go red; the `& SENSOR` grep gate exempts (or routes through `sensor_pair`) the routing `debug_assert_eq!`; the
  `unsafe` delta is stated explicitly (0 new sites if Skip reuses L9's tag write and L5's commit write; otherwise
  each listed with the partition SAFETY); D-F records that its edge is under Grid (Tree already fills ~0 held rows
  through L9's ruled O3 mask) and the fill predicate's composition with L9's mask (AND) joins the Q2 cross-lane
  contract.
- **Open questions:** (1) D-F composes with L9's fill as a per-row predicate (AND); if L9 C1 lands a lazy fill in the
  parallel phase, the predicate moves into that fill's guard and D-F has no separate site — decided at L9 C1, recorded
  in Q2. (2) The sleeping-off solve arm keeps its signature and access set (zero cost when sleeping is off — the same
  bar as UG-15's "zero without mods"); the D-H flush with `m = Off` is drained by L10's own system before the solve,
  never by the solve. (3) Cross pairs are REC-without-manifold or SEP only (a manifold would have unioned the islands);
  B3 states it, since it is what makes `restore_rec` keys unique (A3′) and the per-record hit count well-defined.

Lane order stands: L5 (C1/C2 committed, C3/C4 in progress) → tree broadphase ∥ L11 → L9 → L10 (rev 2.3 + these
rulings) → L8 → L12. L10's implementation brief is 04 + 06 + 08 + this section; 05/07/09 are the reviews it answers.
