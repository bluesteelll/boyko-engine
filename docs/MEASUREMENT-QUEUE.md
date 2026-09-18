# The measurement queue — what to run on an idle machine, and what will lie if you do not

**Purpose.** Several campaigns have reached the point where the remaining question is a number, and
this machine is the owner's workstation rather than a bench rig. Numbers taken while agents compile
are artifacts, and — the part that makes this dangerous — **two runs minutes apart AGREE with each
other under a steady load and are both wrong**, so the usual sanity check does not protect you. This
file is the single place those runs are queued, so that nobody has to reassemble them from campaign
documents and nobody quotes a loaded number later as if it were a measurement.

Written 2026-09-03. Every entry says what it decides, so an entry whose decision has already been
made elsewhere can be struck rather than run.

---

## 0. The precondition, and how to check it

The idle-machine precondition already lives in the campaign's measurement plan
(`docs/threadpool/KE16-DESIGN-MEASUREMENT.md`, added in `b51d03a7`). It is restated here in one line:
**before any timed step, confirm no agent workflow is compiling and no game is running**, and record
in the receipt that you did. A timed step without that receipt is not a measurement.

Structural verdicts — compiles / does not, vacuous / not, segfault, counts, a census result, an
instruction count from disassembly — are **not** load-sensitive and never belong in this queue.

---

## 1. Traps that have already cost this project a wrong conclusion

**`RUSTFLAGS` REPLACES `target.<triple>.rustflags`; it does not append.** Setting it silently drops
`-C target-cpu=x86-64-v3` from `.cargo/config.toml`, so the binary measured is not the binary that
ships. Measured directly:

```
A: no RUSTFLAGS (config only)            ->  target-cpu=x86-64-v3
B: RUSTFLAGS="-C target-feature=+avx2"   ->  target-feature=+avx2   (target-cpu GONE)
```

This one cause produced three separate symptoms in a single day: AVX2 never reaching CI, loom models
appearing to "crash before main" (a lost `-Clink-arg=-B<lld>` truncated an import table), and the ISA
census going red. If a flag must be added, use `cargo --config 'target."cfg(windows)".rustflags=[...]'`,
which MERGES. **Any bench header still instructing `RUSTFLAGS=...` is stale — fix it rather than
following it.**

⚠ **The key is `cfg(windows)`, not a triple, since 2026-09-10** — the day this tree's Windows
recipes moved from `stable-x86_64-pc-windows-gnu` to `stable-x86_64-pc-windows-msvc`. Note the
rustup DEFAULT host is still gnu on that date, so BOTH triples are reachable on this box, which
is exactly why a cfg-spec and not a triple. A `--config` key naming a
triple the build does not use contributes NOTHING and says nothing while doing it: a `--cfg loom`
spelled `target.x86_64-pc-windows-gnu.rustflags` on an msvc host produces a test binary with zero
models, `running 0 tests`, exit 0. A `cfg(windows)` spec matches either host, and cargo JOINS it
with the per-triple array in `.cargo/config.toml`, so the ISA baseline neither vanishes nor has to
be restated (measured 2026-09-10 off `cargo -v`). `[build] rustflags` is NOT a substitute: cargo
ignores it entirely whenever a `[target.*]` key matches, and the config file defines one for both
Windows triples.

**The bench profile is not the shipped profile, and neither one is simply "better".** The root
`Cargo.toml` carries `[profile.release] lto = "fat"` and `[profile.bench] codegen-units = 1` with
`lto = false` set EXPLICITLY so bench does not inherit release's fat LTO. So the two differ on the
codegen axis in BOTH directions: bench has one codegen unit and no LTO, release has sixteen and fat
LTO. Ratios between two arms of the same bench transfer; absolute numbers do not. An
identical-code-folding alias at one codegen unit proves nothing at sixteen.

⚠ This paragraph said the opposite until 2026-09-09 — that there was no `[profile.release]`
at all and the bench binary was therefore "better optimised". That was written 2026-09-03 and the
profile changed 2026-09-04, so it had been inverted in direction ever since.

**A microbenchmark whose working set fits in L1 measures a quantity that does not exist at scale.**
Recorded after a chain figure of 1.7× shrank to 3–8% once the working set was realistic.

**Criterion's wall clock on a contended box is not a signal.** On the KE17 instrument the same row
read 4.86 ms in one run and 1.29 ms in another, from the same binary — an 8.9× spread that was other
agents' compile load. Where a structural column exists, gate on it and report the wall clock as
context only.

---

## 2. KE16 — the tournament

**Decides:** which candidate for the `par_iter`-serialises defect ships, on THROUGHPUT rather than on
core occupancy. The owner's standing criterion: *if synchronisation costs more than the gain from
loading the cores, do not do it.*

**Status:** the script is written and has never been run (`ke16-phase3b-tournament.js`, kept beside
the session memory). It is blocked only on a quiet machine and on phase 3a finishing its axes.

**Gate receipts already taken, and to be RE-TAKEN because they were measured under load:** default
`lanes_used = 1` / 0.99×; `a1` 12.58×, `a1-fifo` 12.62×, `a2` 7.59×, `a3` 1.92×, `a5` 12.20×; an ECS
figure of 409.8 ms → ~102.7 ms. Treat every one of those as provisional.

**Report:** throughput per candidate, not occupancy; and the losing candidates get frozen with their
return condition, never deleted (owner rule, `396d8b63`).

---

## 3. KE17 — the apply-window barrier

**Decides:** whether the split apply window is built or the ticket is closed with a number.

**The reading that decides it comes FIRST** and is structural, not timed: the bench's `split_sim`
column against `barrier_sim`. `8.1%` is what the barrier costs on an engine-like shape — an UPPER
BOUND — not what the split recovers, because a `Commands`-carrying system keeps its barrier. **If
`split_sim` lands within 2 percentage points of `barrier_sim`, KE17 closes with a number** and the
design is frozen rather than built.

**A requirement the split must keep if it IS built — EM2′-K.** Since EM2′ a `Commands` spawn claims
from the recycled-entity stack with `fetch_sub`, and every `&mut EntityMaster` operation settles that
stack with a plain store. So no apply window that mutates `EntityMaster` (any spawn, despawn,
allocate or rewind) may overlap a dispatched system with `may_defer[i] == true`: the overlap would
double-issue entity ids and race a stack push against a worker's entry read. Stated at the SCH7 site
(`apply_window_drain`'s gate) and on the `may_defer` field in `schedule.rs`.

```bash
cargo bench -p boyko-ecs --bench ke17_apply_window
```

**Report the `barrier/model` and `split_sim` columns ONLY.** They are computed from measured
per-system durations replayed through two models with dispatch latency removed by construction, which
is why they moved under one percentage point across six runs spanning quiet and heavily loaded
windows while the wall clock on the same row moved 8.9×. **Never quote this bench's wall clock as a
barrier measurement.** The `idle_pred` / `idle_nopred` counters are diagnostics, not a gate — one of
them was already mislabelled once (a root system CAN be barrier-blocked, through the conflict path).

---

## 4. Physics — what the default-off switches are worth

**Decides:** which optimisation switches to enable. `simd` has already been flipped to `true` on the
strength of its bit-identity gates; these runs price what it and its siblings buy.

```bash
# 0. Prove the ISA baseline reached the compiler before believing any arm.
cargo test --workspace --test isa_baseline_census --no-fail-fast
#    must print: running 2 tests / 2 passed

# 1. simd (O1) — kernel-local A/B, both arms in ONE binary, runtime-switched.
cargo bench -p boyko-physics --bench simd_o1
#    report o1_refresh_inertia/{scalar,avx2} and o1_gravity/{scalar,avx2}.
#    o1_position_integrate is INFORMATIONAL ONLY: that pass is hard-coded scalar
#    by a measured decision, and the flag does not gate it.

# 2. simd_solve (O7) — whole-step A/B on the colored solver.
cargo bench -p boyko-physics --bench colored_solve -- o7_simd_ab
#    report the PAIRS, never a single arm:
#      simd_solve_single_prod     vs scalar_colored_single_prod      <- production shape
#      simd_solve_single_solvedom vs scalar_colored_single_solvedom  <- Amdahl-free ratio
#      simd_solve_parallel_4w     vs scalar_colored_parallel_4w      <- RECORD, do not act on (KE16)

# 3. colored vs the shipped reference solver.
cargo bench -p boyko-physics --bench colored_solve -- solve_step
#    reference/N vs colored_solve/N vs colored_solve_plus_graph/N.
#    The third is the honest one: it includes the per-step graph build.

# 4. broadphase crossover — this is what CALIBRATES GRID_LO / GRID_HI.
cargo bench -p boyko-physics --bench broadphase
#    report the n where grid crosses all_pairs, and the same for the disparity fixture.
#    Compare against GRID_LO=96 / GRID_HI=192, which are labelled UNMEASURED at their site.
```

⚠ **Pass no `RUSTFLAGS`** — see §1. Any bench header still telling you to is stale.

**A gap, stated rather than hidden.** No bench prices `simd` end-to-end through `SoftStepSolver`,
which is the one number the decision to flip it would most benefit from. Building it means an A/B of
`SoftStepSolver::step` at `simd ∈ {false, true}` over the pyramid fixture. Expect a few percent, not
the 3.8× that circulates: that figure is a KERNEL ratio on a transcription built outside the
repository at a non-shipped profile, and the document that produced it says so at the site.

**Out of scope until KE16 lands:** `parallel_solve` and `parallel_broadphase`. Both dispatch through
`pool.scope`, which is exactly the defective route; `benches/ke16_solve_in_system.rs` exists to price
the worker-issued path against the dispatcher-issued one, and they must be shown to converge first.

---

## 5. UI — the first pixel

**Decides:** nothing, yet. The UI ladder's gates are pixel gates and structural censuses, not timings.
The one number that will eventually belong here is the UI sub-pass's frame cost, which is to be
**REPORTED, never gated** — this is a workstation, not a bench rig.

Recorded here so that a later reader does not add a wall-clock gate to that ladder by reflex.

---

## 6. Physics — what the defect-A interim row identity costs

**Decides:** whether the always-on half of the row identity map (the per-row `EntityId` + added-tick
read in `physics_gather`, the id compare, one branch per manifold / box pair) stays, and whether the
cold remap needs a hash instead of sort + binary search. Correctness is gated by
`row_keyed_state_defect_a.rs` and `row_identity_remap.rs`; this is price only.

⚠ `benches/sleeping.rs` CANNOT see this change: it drives the solver directly with no gather, so
every consumer classifies `Identity`. Do not use it for this entry.

⚠ **Arm B is the fix commit `b74f7ee8`, not a later tree** (pinned 2026-09-18). Its only parent is
arm A. A7a, the first step of the A7 lane, gives each clipped face-contact point its own feature id.
That changes the contact set every pile arm here measures: on A7-R1's height-15 pile at step 600 it
goes from 5044 manifolds / 12817 points to 5223 / 14605, and the warm-start keys move with the ids. A B
arm on a tree with A7a prices the fix plus a different workload. To measure on a later tree, port arm A
onto the same narrowphase first. A7b, the second step (S5: the face-versus-edge rule, the commit after
`08fe7b9f`), changes it again and more: 6671 manifolds / 22 975 points at the same step, and a
height-15 pile that never came to rest now freezes (A7-R2, step 248). The pin to `b74f7ee8` keeps
both out of this entry.

```bash
# Arms: A = d5782d43 (no fix), B = b74f7ee8 (the fix, before A7a; see above). Idle-machine receipt first (§0). No RUSTFLAGS (§1).
# Run A twice interleaved with B to get the A/A spread.
cargo bench -p boyko-physics --bench jolt_parity_pyramid -- full_step/1
cargo bench -p boyko-physics --bench jolt_parity_pyramid -- full_step/4
cargo bench -p boyko-physics --bench row_identity_churn     # B only: arms × sleeping {off,on}
#   stable                — pyramid, no structural change
#   swap_churn            — per step: despawn one non-last body, spawn one at rest (recycled id, row count constant)
#   archetype_shift       — per step: remove/insert a marker on one body in the first-walked archetype (full shift)
#   first_archetype_spawn — per step: spawn one body into the first-walked archetype, despawn one from the last
#   burst_despawn         — per step: despawn 32 bodies from the first-walked archetype and respawn 32 (flagged). Alignment
#                           is KEPT (removals realign through E4): about 32 stage-2 rows
#   burst_migrate         — per step: insert `Marker` on 32 (> REMAP_WINDOW) bodies of the later archetype, remove it on the
#                           next step. Alignment is LOST: exercises the budget cut-off and stage 2 over the rest of the walk
```

**Rules:**
- R1: if B/A `full_step/1` median > 1.005 and outside the A/A spread → gate ONLY the per-row added-tick
  read on `Archetype::has_structural_add_since(last_run, this_run)` (archetype.rs:1064; `pub`, no consumer
  wired per its doc at :1053-1060). It cannot gate the id compare: row removals do not stamp
  (archetype.rs:969-973), and the id compare is what detects swap-moves. It inherits the same
  one-run-late window as the added flag itself (same `is_newer_than`). Reaching it needs an archetype-level
  walk in the gather that exposes the archetype and the system's ticks — not verified to exist; possibly a
  kernel accessor.
- R2: if `swap_churn` exceeds `stable` by > 2 % → profile the cold path before changing anything.
- R3: if `burst_migrate` exceeds `stable` by > 10 % → replace stage 2's sort + binary search with an
  open-addressed `EntityId → row` table in a `ScratchColumn` (sized with the row count, so the census rule holds).
  If `burst_despawn`, `archetype_shift` or `first_archetype_spawn` exceeds `stable` by > 2 % → profile the aligned
  walk before changing it.

The structural companion to R3 is `row_identity_remap.rs` T3's `row_remap_searched` bounds, which need no
timing.

**Report:** medians and the A/A spread, not a single run.

---

## 7. Physics — what the shared body-set selection costs (defect A5)

**Decides:** whether `physics_apply` / `physics_soft_rigid_apply` / `physics_gather` keep the row
selection as the table filter `BodySetFilter = (With<RigidBodyMass>, With<Collider>)` inside
`BodyQuery`, or whether it moves into the data terms. Correctness is gated by
`apply_row_alignment.rs`, `soft_rigid_apply_row_alignment.rs` and `body_set_selection.rs`;
this is price only.

The claim under test: a table `With<C>` is archetypal
(`crates/boyko_ecs/src/ecs/core/iters/query/filter.rs:535`) and its `set_table_*` bodies are
entirely `if const { C::STORAGE_IS_DENSE }` (same file, `:620-669` — `:671-682` is `filter_fetch`,
not a `set_table_*` body, so the wider `:620-682` that `body_set.rs:44` cites overshoots by one
function), so the per-row loop,
fetch and cursor are unchanged; the only possible residue is one 32-byte fetch initialisation per
iteration start. Expected B/A = 1.000.

⚠ `benches/sleeping.rs` and `benches/parallel_solve.rs` run no gather and no apply. Do not use them.

⚠ **Arm B is the fix commit `a56007ab`, not a later tree** (pinned 2026-09-18). Its only parent is
arm A. The reason is the one given in §6: A7a changes the pile's contact set, so a B arm with A7a
prices a different workload — and A7b (S5) changes it again (§9). On A7-R1's resting height-15 pile
the rule alone adds 17.1 % contact points on identical poses; the +57 % §9 also quotes compares two
different trajectories at step 600. Neither ratio was measured on this entry's scenes, and neither
may be carried to them: R1 below would fire on a B arm with A7b for a reason that is not the
`physics_apply` filter fetch, by an amount nobody has measured.

```bash
# Arms: A = d552be05 (the defect), B = a56007ab (the fix, before A7a; see above). Idle-machine receipt first (§0). No RUSTFLAGS (§1).
# Run A twice interleaved with B for the A/A spread.
cargo bench -p boyko-physics --bench jolt_parity_pyramid -- full_step/1
cargo bench -p boyko-physics --bench jolt_parity_pyramid -- full_step/4
cargo bench -p boyko-physics --bench row_identity_churn -- stable          # sleeping_off and sleeping_on
cargo bench -p boyko-physics --bench soft_step_sp2 -- soft_step_sp2/coupled # fixture migration A/A
```

**Rules:**
- R1: `full_step/*` or `stable/*` B/A median > 1.005 and outside the A/A spread → inspect the release
  `physics_apply` listing for the filter-fetch initialisation; if present, move the selection into the
  data terms and re-measure. Do NOT revert to `Query<Mut<RigidBody>>`: that is defect A5.
- R2: B/A < 0.995 on these scenes → suspect the measurement: they hold no `RigidBody` carrier outside
  the body set, so the fix removes no work there.
- R3: `soft_step_sp2/coupled` B/A outside the A/A spread by > 2 % → the fixture migration changed the
  measured work; investigate before accepting it.
- R4: if `alloc_frame_census` S1a/S1b/S1c or any `alloc_frame_attribution` row moves at all between
  the two arms, stop — the fix is specified to add zero allocations. The S1c release pin itself moved on
  2026-09-18, when A7a re-drew its deterministic window (re-pinned from a long-run adjudication, no new
  allocation site; see the `alloc_frame_census.rs` header, "S1c after A7a"). That move is not A5's and
  does not trigger R4. A7b (S5) changes the pile's contacts again, so the deterministic window is
  re-drawn again; that move is not A5's either. Compare the two arms' own census runs, never either arm
  against today's pin.

**Report:** medians and the A/A spread, both worker counts, both sleeping settings.

---

## 8. Physics — what wake-on-contact-change costs (defect A4)

**Decides:** whether `IslandSleep::begin_step` keeps the per-row island contact key — the extra
sweep that reads `ConstraintGraph::island_starts()` per row, compares the stored count and stores
the new one — as shipped, or whether the key has to be folded into the freeze fold that already
walks the same rows. Correctness is gated by `sleep_settles_box_piles.rs`,
`support_loss_wakes_sleepers.rs` and `resources_tests.rs`'s `rekey_rows` group; this is price only.

**NO TIMING HAS BEEN TAKEN YET.** The bench exists (`crates/boyko_physics/benches/sleeping_pipeline.rs`,
`cargo test --release -p boyko-physics --bench sleeping_pipeline` runs its structural checks) and
the arms below are its three `bench_function` names; no number from it has been recorded anywhere,
here or in a campaign document. Anything quoted as an A4 price before this entry is run is an
invention.

⚠ `benches/sleeping.rs` (O8 Gate 9) CANNOT decide this entry. It drives the colored solve directly
with a fixed manifold set and no schedule, narrowphase or gather, so no island's count can change
between its steps and no contact-change wake can ever fire: it prices the store-only half on a
scene where the key is always equal. Use it for the O8 bookkeeping question, not for this one.

**The three arms, and what each holds fixed** (all on Jolt's height-15 pyramid, 1240 dynamic boxes
on a static floor, one `Schedule::run` per sample, serial pool — so the row is the sleep
bookkeeping, not dispatch):

- `pyramid_sleeping_off` — exactly touching pile (separation 0), gravity on, **sleeping off**.
  `begin_step` does not run at all, so a B/A move here is a leak into the default path, not a cost.
- `pyramid_awake_sleeping_on` — Jolt's pile (separation 0.5), gravity on, sleeping on with
  `sleep_threshold = 0` so **no island ever latches**. Holds the latch state fixed at "awake": the
  key loop runs its store-only path on every row, every step. This is the always-on price.
- `pyramid_frozen_sleeping_on` — exactly touching pile, **gravity off**, default threshold and
  debounce, stepped until every dynamic row is frozen. Holds the poses AND the contact set fixed:
  every contact is at separation exactly 0 and every velocity is exactly 0, so the pile latches on
  the step its debounce completes with or without the wake and freezes in its spawn pose. That is
  what makes the frozen state reachable identically on arm A and lets the two trees be compared.
  With gravity on, a pile of this height never comes to rest (defect A7) and the two trees freeze
  different contact sets, so this arm is gravity-free by necessity, not by preference. That holds on
  both arms, which predate A7a and A7b. On a tree with A7b the gravity-on pile does freeze (A7-R2:
  step 248 on the kernel as committed, 188 on the form first implemented; msvc release, 2026-09-18),
  but moving this arm onto such a tree also moves the contact set (§6), so the gravity-free design
  stands for this entry.

```bash
# Arms: A = d552be05 with the bench's `contact_wakes` helper body replaced by `0` (see below),
# B = a56007ab (the fix, before A7a; its only parent is d552be05; see §6 for why). Idle-machine receipt first (§0). No RUSTFLAGS (§1).
# Run A twice interleaved with B to get the A/A spread.
cargo bench -p boyko-physics --bench sleeping_pipeline   # all three arms
cargo bench -p boyko-physics --bench jolt_parity_pyramid -- full_step/1   # cross-check, no sleeping
```

**Arm A.** `d552be05` has no `IslandSleep::contact_wakes` counter and no key loop. The bench is
otherwise portable to it: copy `benches/sleeping_pipeline.rs` and its `[[bench]]` entry over, and
replace the body of its `contact_wakes` helper with `0` — that helper is the ONLY A4-only code in
the file. Its frozen arm then asserts the same freeze step, the same unmoved poses and the same
manifold/island structure, which is the point of the comparison.

**Rules:**
- R1: `pyramid_sleeping_off` B/A outside the A/A spread → stop. Sleeping is off in that arm, so A4
  must be unreachable there; a move means the key loop leaked into the default path.
- R2: `pyramid_awake_sleeping_on` B/A median > 1.01 and outside the A/A spread → the cheap lever
  first, before any behavioural change: the key sweep shares its loop with the freeze fold
  (`resources.rs:3490-3523`) and calls `graph.island_of(row)` per row, which re-derives the
  `island_of` read slice and does a bounds-checked `get` each time, while `island_starts()` is
  already hoisted. Hoist the `island_of` slice the same way — two flat slice loads per row — and
  re-measure.
- R3: `pyramid_frozen_sleeping_on` — check the two receipts print the SAME `froze at step`,
  manifold count and island count on both arms before comparing any time. If they differ, the two
  trees are not in the same state and the row is void, not slow.
- R4: the frozen arm's manifold count is 0 on either tree → the row is void. The pile's contacts
  sit at separation exactly 0, so a narrowphase epsilon change empties the scene while every other
  check in the arm still passes (the arm's own assertion catches this; do not raise it away).

**Report:** medians and the A/A spread for all three arms, plus both trees' frozen-arm receipts
(freeze step, manifolds, islands, `contact_wakes`).

---

## 9. Physics — what the face-versus-edge rule costs in a default world (defect A7b, S5)

**Decides:** what S5 costs, and whether that price calls for follow-up work. It does NOT decide whether
S5 ships: a resting pile that creeps is wrong, and a 1-point edge contact on a face-face pair is wrong
geometry, so the price is recorded, not traded against correctness. Correctness is gated by A7-N9..N11
(`narrowphase/box_box.rs`) and A7-R1 / A7-R2 (`sleep_settles_box_piles.rs`); this is price only.

**The default configuration is the primary number.** Sleeping is OFF by default
(`resources.rs:494`, `sleeping: false`), so every resting pile in a default world pays S5 on every step.
Measured structurally (msvc release, 2026-09-18; counts, not time), on A7-R1's resting height-15 pile,
on the kernel as committed (the form first implemented in brackets):

- at step 600, contact points 14 605 on arm A and 22 975 on arm B [22 974], manifolds 5223 and 6671
  [6678]: +57 %. That compares two DIFFERENT trajectories, each arm's own pile at its own step 600,
  so it is not the rule's price alone. **On identical poses the rule alone adds +17.1 %**: every
  narrowphase call of arm B's run over steps 600-3000 was also asked of a frozen copy of arm A's rule
  with the same poses and hint, and the two produced 54 989 018 and 46 942 494 contact points
  [54.84 M against 47.14 M, +16.3 %]. The support pairs carry their clipped patch of up to four points
  instead of one. Both ratios are this scene's; the bench's timed steps are another trajectory (R2).
- where the SAT answers an edge, the narrowphase now builds the face patch first: 3 531 050 times over
  steps 600-3000 (~1470 per step) [3 425 959]. It keeps the patch on all but 264 [260], every one of
  them a face that realized no patch, so the clip is wasted only there. The 5 mm comparison against
  the patch chose the edge 0 times: on this pile the constant's value does nothing.
- the cold fallback builds 12 572 edge contacts over those 2400 steps, ~5.2 per step [13 615]. On
  10 446 of those calls [11 294] arm A's rule, given the same poses and hint, built an edge contact
  too. The other 2126 [2321], under one per step, are manifolds arm A lacked, all on no-load
  same-layer knife-edge pairs.

With sleeping ON the comparison is not like for like: arm A's gravity-on pile never freezes and arm B's
does (A7-R2, step 248), so an opted-in world's steady cost falls to the sleeping floor on B alone. Report
it beside the primary number; never quote it as S5's price.

**NO TIMING HAS BEEN TAKEN.** Every figure above is a count.

⚠ **Arm B is the S5 commit itself** — the commit whose only parent is `08fe7b9f` and which introduces
`FACE_AXIS_PREFERENCE` — not a later tree. Pin its hash here once it exists.

```bash
# Arms: A = 08fe7b9f (A7a, before S5), B = the S5 commit (its only parent is 08fe7b9f; see above).
# Idle-machine receipt first (§0). No RUSTFLAGS (§1). Run A twice interleaved with B for the A/A spread.
cargo bench -p boyko-physics --bench sleeping_pipeline -- pyramid_sleeping_off       # PRIMARY: default config
cargo bench -p boyko-physics --bench jolt_parity_pyramid -- full_step/1              # Jolt's scene (gap 0.5)
cargo bench -p boyko-physics --bench jolt_parity_pyramid -- full_step/4
cargo bench -p boyko-physics --bench sleeping_pipeline -- pyramid_awake_sleeping_on  # sleeping on, never latching
```

**Rules:**
- R1: `pyramid_sleeping_off` B/A median, with the A/A spread, IS the number this entry exists for. It has
  no action threshold.
- R2: the bench times from its 30th step onward, before the vertical settle A7-R1 waits 600 steps for,
  so the row ratio that applies is the one over the bench's OWN timed steps, not A7-R1's step-600 census.
  Count the live contact points over each arm's timed run (a probe, like §8's receipts) before reading
  any ratio. A B/A time ratio above that row ratio by more than the A/A spread means the cost is not
  the extra rows alone: profile the realized-patch check and the fallback before anything else.
- R3: B/A below 1.0 on `pyramid_sleeping_off` → suspect the measurement: B solves strictly more rows.

**Report:** medians and the A/A spread for every row, both arms' timed-run point counts, and the
sleeping-on row labelled as not like for like.

---

## When an entry is done

Strike it with the date and the receipt's location, rather than deleting it. An entry that was run
and produced a surprising number is worth more as history than as a blank line.
