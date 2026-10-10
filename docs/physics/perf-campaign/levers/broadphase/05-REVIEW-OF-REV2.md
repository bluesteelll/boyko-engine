VERDICT: CHANGES REQUESTED; BLOCKING=0; IMPORTANT=2

# Architecture review: Tree broadphase, rev 2 (delta review)

## Verdict
[ ] APPROVED
[X] CHANGES REQUESTED. All five rev-1 Important remarks are addressed in substance. The exactness core now rests on Lemma D3.1, and I re-derived it. Two new problems came in with the revision:
- the patch rule's cost claim is wrong for a reachable case;
- three of the new structural gates cannot fail for the defect they name.

## Rev-1 remarks, checked against the tree

| remark | status | evidence |
|---|---|---|
| W1 hint on with sleeping off | ✅ resolved; one conjunct left without a red check (see W2(b)) | The mask stays empty with sleeping off (`resources.rs:3560-3567`, only `begin_step` writes it). `mask_rows = 0 ≠ N` and `!cfg.sleeping` both turn the hint off. M6 goes red on sleeping-off J (candidates from step 1). |
| W2(a) stamp | ✅ | The stamp comes after maintenance, which is where protocol P (`row_identity.rs:210-213`) puts it. Under M5s the cursor gives `Reset` from gather 2 on (`:474-490`), so translations are 0 ≠ 40 and a row shift evicts the floor. Red. |
| W2(b) churn dissolves the sets | ✅ | Rows are translated through `prev_row`. The hint is read through `prev_row` against `N_prev`, which `begin_step` wrote at s−1 with `n_rows = N_{s−1}`. |
| W2(c) N changes under direct drive | ✅ | The vanished count and leaf scan cover it. |
| W2 trigger list | ✅ | I found no missing event: shape, teleport, class, kind, hint, despawn, tail, `Reset`, kind switch and brute crossing are all listed. |
| W2 upper-bound gates | ⚠ partly | J `static_rebuilds == 1` holds under the rent rule: step 1 has rent 1 < 1.25, step 2 has 2 ≥ 1.25. The post-freeze bound is vacuous as written (W2(a)). |
| W2 costs, churn arms | ✅ except the patch path (W1) | |
| W2 / OQ3 sole authority | ✅ | Lemma D3.1 holds. The predicate is bitwise symmetric (`systems.rs:319-321`) and reads only `(p, r)`. M5 is now a real mutation, and M5r is red on Δ`static_rebuilds`. |
| W3 | ✅ | C2 is deferred with its own gate. `parallel_broadphase` is untouched. |
| W4 | ✅ | The directed pair (−1, −1 at 1.5) gives 2.25 ≤ 4, so AllPairs emits it. Under M2 the inverted box never overlaps. Forcing `brute_max_rows = 0` was also needed. |
| W5 | ✅ | Arrays typed by `SPAN_ZONE_COUNT` / `COUNTER_ZONE_COUNT`, plus the order assert as in `profiling_zone_counts.rs:253,277`. |
| O1–O3, OQ1–OQ3 | ✅ | The cohort ids check out: 399 mod 64 = 15, 389 mod 64 = 5; `ROW_PREV` 403 is slot 19; `TOUCHED_AWAKE` = 475 is slot 27. |

## Remarks

### 🔴 Critical
None.

### 🟡 Important

#### W1. The patch rule cascades on a high jumper that is the min endpoint of 2 or more entries. D3.5's translation cost and the reason LIS was rejected are both wrong for that case, and no gate measures it.
**Where**: "The patch rule" (Algorithms); D3.5 "Rejected: LIS … The patch touches only the entries that moved"; G4 maintenance arms.

**Problem**: A body that migrates to a later archetype lands at a higher row R. Take its old run (m, p1), (m, p2), …, which is contiguous and sorted.
- It translates to (inv[p1], R), (inv[p2], R), … Each entry is greater than the last kept entry and smaller than its successor, so every entry of the run except the last is **kept**.
- Every following entry smaller than (inv[p_{k−1}], R) is then diverted.

Traced example: old list (5,9), (7,8), (7,12), (7,15), (8,9), (8,10), (9,12); row 7 moves to 20 and the later rows shift down by 1.
- Kept: (5,8), (7,20), (11,20).
- Diverted: (14,20), (7,8), (7,9), (8,11), and everything after them up to key (11,20).

Swap-remove (a low jumper) and monotone shifts are fine. Only the high-jumper-as-min case cascades.

**Consequence**:
- The cascade length is set by the row distance to the partners, and at 10k/100k that distance is arbitrary when spawn order is unrelated to space.
- Adding a component to a sleeping or static body at 100k (|L| ≈ 770k) therefore falls through to the whole-list sort (a > |L|/4). That is a radix sort of 770k keys, ~4–6 ms (arith.), against the 1.6–2.5 ms that D3.5 states.
- No gate sees it:
  - G4's maintenance arm is a one-row monotone shift.
  - Every churn-scene jumper is a sphere with a single floor entry (`benches/row_identity_churn.rs:12-15, 431-437, 466-473`), so a ≤ 1 there.

**Confidence**: CONFIRMED (traced from the rule's text).

**Needed**: either a keep test that cannot keep an entry with a non-monotone endpoint (for example, jumper rows identified from `prev_row` during the verify), or a D3.5 that states the true bound. In both cases:
- add a G4 maintenance arm at 10k/100k: one member moved to the top row, with k ≥ 2 higher-row partners, under the same 2× stop rule;
- add a unit test pinning `a` for that case.

#### W2. Three of the new structural gates cannot fail for the defect they name

**(a) Anchor A is unreachable.**
- G2 defines A as "the first step with every row hint-frozen".
- `begin_step` marks every row with no island as awake (`resources.rs:3477-3479, 3563`). J's and R's static floor therefore never reads frozen, so A never occurs.
- "Δ`sleeper_rebuilds` ≤ 1 and Δevictions == 0 after A" then checks an empty range.
- **Consequence**: this was the only upper bound rev-1 W2 asked for after the freeze. What remains is the pinned total, and that pin is blessed from the implementation's own first run, so a post-freeze admission/eviction loop present at first bless ships green.
- **Needed**: define A over the dynamic rows (or as "Q is empty"), and assert that A exists (for example A ≤ 300; the P0 run saw all asleep from step 264).

**(b) The `cfg.sleeping` conjunct has no red-able check.**
- M6 removes both conditions. M6b removes coverage only. On sleeping-off J, coverage alone already turns the hint off (`mask_rows = 0`).
- The conjunct is load-bearing only after a runtime toggle-off. `physics_solve_colored` then never touches `IslandSleep` (`systems.rs:1113-1117`), so the mask stays at its last sleeping-on value with `mask_rows == N`.
- **Consequence**: an implementation without the `cfg.sleeping` term passes G1, G2 and G5. After a toggle-off, the formerly frozen pile, which now jitters, is re-admitted every ~2 steps (rent 1240 against 1550) and evicted the next step. That is W1's rev-1 cost again, ~0.1 ms per step at J. Runtime toggles are real: `profiling_zone_counts.rs:190-191`, and L10 ruling W2.
- **Needed**: a script that runs sleeping on until frozen, then turns it off with no row change, asserting Δ`hint_candidates` == 0 and Δ`sleeper_rebuilds` == 0 over the off phase, with the mutation "drop the `cfg.sleeping` term" shown red.

**(c) A void counts as a pass.** G2 churn, sleeping on, says "members ≥ 1,240 at window start (else void)". In a cargo test a void has to be red, or it is a green from emptiness.

**Confidence**: CONFIRMED for all three.

### 🟢 Optional
- **O1.** G2 J asserts `translations` == 0, but every world's first gather classifies as `Rows`: the previous gather is empty, so it is not stable (`row_identity.rs:440`), and a never-stamped cursor then gets `Rows` (`:482-487`). Define `translations` as "`Rows` steps with at least 1 member", or the assertion is red on a correct implementation at step 0.
- **O2.** Under M6, admission is at step 2, not step 1: the rent is 1241 against 1241 + 0.25·1241. The mutation is still red.
- **O3.** The L5 lane grows `SPAN_ZONES` from 14 to 17 and `COUNTER_ZONES` from 6 to 7 (`L5-narrowphase/02-DESIGN-REV1.md:302`), and it edits `check_step` too. The figures 18 / 9 and "27" hold only if this lane merges first. Add this to Open question 1. The typed arrays make the conflict a compile error, which is good.
- **O4.** `profiling_zone_counts` promises values recomputed from public state (`:1-3`). On the harness scene the new counters are structural: members = [step ≥ 2], rebuilds = [step == 2], queried = N − members. Pin those, rather than expectations read from `diag()`, which can only catch wiring errors.

## Positive
- Lemma D3.1 is the right move. It makes the cursor, the class and the hint cost-only, it removes rev 1's rebuild-on-`Rows`, and it turns M5 into a mutation that can go red. The consumed mark enforces injectivity rather than assuming it.
- Translation plus eviction plus rent-rule admission does keep the sets through churn.
- G1 now forces the tree path.
- The segment stream in D5 fits a serial design, and dropping the 31-entry cap and the spill path is correct.
- The per-row Excluded/Wide kinds are sound. I re-checked bound² ≤ 2¹²² and that Excluded–Excluded pairs are always false.
- Deferring C2, with a threshold that a serial tree fails, is the honest resolution.

## Open questions
1. Ruling B1 lists `ContactPairs` among the views "provided WITHOUT copying the sleeping store every step". C5 copies SL into `ContactPairs` every step (D5 step 5; the plan puts it at 1.2–1.9 ms at 100k). Does B1 bind C5's assembly, or only L10's stores? If it binds C5, D5's merge and `ContactPairs`' shape change before C5 lands.

Files: `D:/wt/joltab/crates/boyko_physics/src/row_identity.rs`, `D:/wt/joltab/crates/boyko_physics/src/resources.rs`, `D:/wt/joltab/crates/boyko_physics/src/systems.rs`, `D:/wt/joltab/crates/boyko_physics/benches/row_identity_churn.rs`, `D:/wt/joltab/crates/boyko_physics/benches/jolt_parity_pyramid.rs`, `D:/wt/joltab/docs/physics/perf-campaign/levers/L5-narrowphase/02-DESIGN-REV1.md`