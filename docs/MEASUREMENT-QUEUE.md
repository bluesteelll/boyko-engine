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
#    Compare against GRID_LO=2700 / GRID_HI=3000, labelled [MEASURED 2026-09-19, P0b §8] at
#    their site, where a const assert keeps GRID_HI >= the 2,978-body disparity crossover.
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

## ~~6. Physics — what the defect-A interim row identity costs~~

**RESULT, 2026-09-18. Struck: no rule fired.** Owner's workstation (AMD Ryzen 9 5900HS, 16 logical CPUs; RTX 3060
Laptop GPU, unused by these benches), `stable-x86_64-pc-windows-msvc` rustc 1.98.1, bench profile. Every arm's cargo
fingerprints record `target-cpu=x86-64-v3` from `.cargo/config.toml`; no `RUSTFLAGS`. Prebuilt exes were invoked
directly, each sha256-checked before its run; nothing compiled. §0 receipt: the owner declared the machine quiet at
about 19:00 and no other workflow ran. 10 runs, 21:05–21:17 +03:00. No cargo, rustc, link, lld, dxc or clippy process
existed at any receipt. The 10 s CPU average was 2.09–4.42 % before each run and 2.12–4.22 % after. Background: the
browser and Task Manager. Contaminated runs: 0.

| row | A = d5782d43 (ms) | B = b74f7ee8 (ms) | A/A | B/B | B/A | n |
|---|---|---|---|---|---|---|
| full_step/1 | 19.3185 / 18.8567 | 18.7951 / 18.7534 | 2.42 % | 0.22 % | 0.984 | 2+2 |
| full_step/4 | 11.0775 / 11.2249 | 11.1455 / 10.7047 | 1.32 % | 4.03 % | 0.980 | 2+2 |

row_identity_churn (B only, n = 2): each row ÷ the same run's `stable`, mean of B1 and B2.

| sleeping | swap_churn | archetype_shift | first_archetype_spawn | burst_despawn | burst_migrate |
|---|---|---|---|---|---|
| off | 1.002 | 0.990 | 0.994 | 0.994 | 0.997 |
| on | 1.008 | 1.010 | 1.003 | 0.998 | 1.003 |

- **R1 did not fire.** B/A was 0.984, not above 1.005, and inside the A/A spread. The always-on half stays; no gating on
  `has_structural_add_since`.
- **R2 did not fire.** swap_churn / stable was 1.002 (off) and 1.008 (on). Run B1-off alone read 1.020, and B2-off read
  0.985.
- **R3 did not fire.** burst_migrate / stable was at most 1.003, against a 10 % threshold, so stage 2 keeps sort +
  binary search. The aligned-walk rows were at most 1.010, against 2 %.
- **Limit:** the B/B spread on the churn rows reaches 3.87 %, above the 2 % thresholds. At n = 2, those clauses mean
  "not seen", not "absent".
- Structural receipt, identical in all 10 row_identity_churn logs (s6 and s7): stable built 0 maps; the churn rows
  resolved 4 / 5 / 2 / 124 / 2546 rows through stage 2.

Receipts: `docs/measurements/2026-09-18-physics-s6-s9/` (`timing_raw.md` §1 and §3; `raw/s6/`; `runlogs/s6_*`;
exe hashes in `exes.json`; ISA and no-compile proof in `build_report.md`).

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

## 7. Physics — what the shared body-set selection costs (defect A5) — TIMED 2026-09-18; OPEN

**RESULT, timed part, 2026-09-18.** Same host, toolchain, ISA proof and §0 conditions as §6. 20 runs, 21:17–22:01
+03:00. The 10 s CPU average was 1.78–4.10 % before each run and 1.75–5.61 % after. Contaminated runs: 0. Flagged:
`stable` B2 (receipts 4.1 % before, 5.4 % after).

| row | A = d552be05 (ms) | B = a56007ab (ms) | A/A | B/B | B/A | n |
|---|---|---|---|---|---|---|
| full_step/1 | 19.0483 / 19.3088 | 19.2060 / 19.2486 | 1.36 % | 0.22 % | 1.0025 | 2+2 |
| full_step/4 | 10.7231 / 10.7650 | 11.0553 / 10.6700 | 0.39 % | 3.55 % | 1.0110 | 2+2 |
| stable/sleeping_off | 17.0243 / 16.9737 / 17.1108 / 16.9773 | 17.1154 / 17.5472 / 17.1838 / 16.7985 | 0.30 % (n=2), 0.81 % (n=4) | 2.49 %, 4.36 % | 1.0195 (n=2), 1.0082 (n=4) | 4+4 |
| stable/sleeping_on | 16.7693 / 16.7577 / 16.6816 / 16.8479 | 17.0705 / 16.7199 / 16.6283 / 17.1120 | 0.07 %, 0.99 % | 2.08 %, 2.87 % | 1.0079, 1.0071 | 4+4 |
| soft_step_sp2/coupled/64 | 0.0873 / 0.0874 | 0.0869 / 0.0869 | 0.17 % | 0.08 % | 0.9950 | 2+2 |
| soft_step_sp2/coupled/256 | 0.3835 / 0.3815 | 0.3814 / 0.3823 | 0.52 % | 0.24 % | 0.9984 | 2+2 |

- **R1 fired by its wording** on full_step/4 (+1.10 % against A/A 0.39 %) and on stable/sleeping_off (+1.95 % at
  n = 2; +0.82 % against 0.81 % at n = 4). The n = 4 firing rests on run B2 alone: without it, B/A is 1.0007. At n = 2,
  stable/sleeping_on fired too; at n = 4 it did not.
- **Against R1:** the B/B spread exceeds each excess. On the same pair of arms, full_step/1 reads +0.25 % and §8's
  `pyramid_sleeping_off` reads +0.28 %.
- **R1's prescription is still open:** inspect the release `physics_apply` listing for the filter-fetch initialisation.
  If it is present, move the selection into the data terms and re-measure. Do NOT revert to `Query<Mut<RigidBody>>`.
- **Attribution limit:** `a56007ab` also carries A4 (wake-on-contact-change). Its key loop is unreachable with sleeping
  off (`systems.rs:1085`), so the sleeping-off rows price A5 plus the rest of the diff. stable/sleeping_on prices A4 + A5.
- **R2 did not fire:** no full_step or stable row is below 0.995.
- **R3 did not fire:** coupled/64 is 0.33 pp outside its A/A spread, under the 2 pp threshold. coupled/256 is inside.
- **R4 NOT RUN.** `alloc_frame_census` S1a/S1b/S1c and `alloc_frame_attribution` are still to be run on both arms and
  compared with each other. This entry is not done until they are.

Receipts: `docs/measurements/2026-09-18-physics-s6-s9/` (`timing_raw.md`; `raw/s7/`; `runlogs/s7_*`; `runs.jsonl`).

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

## ~~8. Physics — what wake-on-contact-change costs (defect A4)~~

**RESULT, 2026-09-18. Struck, with an R2 follow-up queued.** The "NO TIMING HAS BEEN TAKEN YET" line above is now
history. Same host, toolchain, ISA proof and §0 conditions as §6. 4 runs, 21:30–21:34 +03:00. The 10 s CPU average was
2.02–3.43 % before each run and 1.88–3.85 % after. Contaminated runs: 0.

Arm A is `d552be05` with `a56007ab`'s bench and `[[bench]]` entry ported, and the `contact_wakes` helper body set to
`0` (diff: `s8_armA_port.diff`). Arm B is `a56007ab`.

| row | A (ms) | B (ms) | A/A | B/B | B/A | n |
|---|---|---|---|---|---|---|
| pyramid_sleeping_off | 17.7852 / 17.8312 | 18.1349 / 17.5806 | 0.26 % | 3.10 % | 1.0028 | 2+2 |
| pyramid_awake_sleeping_on | 16.7251 / 17.0295 | 17.3349 / 17.2416 | 1.80 % | 0.54 % | 1.0244 | 2+2 |
| pyramid_frozen_sleeping_on | 5.6345 / 5.7020 | 5.4530 / 5.6311 | 1.19 % | 3.21 % | 0.9777 | 2+2 |
| full_step/1 (cross-check; the §7 runs, same exes) | 19.0483 / 19.3088 | 19.2060 / 19.2486 | 1.36 % | 0.22 % | 1.0025 | 2+2 |

Frozen-arm receipts, both trees: `froze at step 61, manifolds 8555, islands 1, contact_wakes 0`. Arm A's 0 is the
stub.

- **R3 holds** and **R4 holds** (8555 manifolds, not 0), so the frozen row is valid.
- **R1 fired by its wording** (+0.28 % against A/A 0.26 %; B/B 3.10 %; the pairings span 0.986–1.020). Its premise does
  not hold on these arms:
  - At `a56007ab`, `begin_step` is reachable only through `solve_colored_sleeping`, which `physics_solve_colored` calls
    only under `cfg.sleeping` (`systems.rs:1085`).
  - `a56007ab` also carries A5, which is on the default path. So a move on this row cannot be the key loop.
- **R2 fired:** +2.44 % against 1.01 and A/A 1.80 %; every pairing is at least 1.0125. Prescribed: hoist the
  `island_of` slice in the key sweep (the loop at `resources.rs:3486-3518` in `a56007ab`), then re-measure this row
  before any behavioural change. The row prices A4 + A5; the sleeping-off rows put A5's share at about 0.3 %.
- **R2 follow-up, queued 2026-09-19.** The hoist is L1 of the physics perf campaign (`IslandSleep::begin_step`,
  bit-identical). The tree before it has `resources.rs` as at `1c31aeac`. Its re-measure is §10's L1 gate: R2's rule
  on J-S0, which is this row's scene (Jolt's pile, sleeping on, threshold 0, one worker) on the fixed-window runner.
- **Frozen row:** B is 2.2 % faster. That is inside B/B, and no rule applies; it is not a cost.

Receipts: `docs/measurements/2026-09-18-physics-s6-s9/` (`timing_raw.md`; `raw/s8/`; `runlogs/s8_*`;
`logs/*.s8receipt.out`; `s8_armA_port.diff`).

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
# `-- full_step/N` is the Criterion bench's filter; it exists on both pinned arms. On a tree where
# jolt_parity_pyramid is the fixed-window runner (§10) there is no such filter, and the cross-check
# is §10's J-A row at W=1, disarmed:
#   <runner exe> --scene jolt --gap 0.5 --cfg a --workers 1 --steps 500 --csv <file>
# The two time different windows (§10, H1), so a runner number is never divided by a Criterion one.
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

## ~~9. Physics — what the face-versus-edge rule costs in a default world (defect A7b, S5)~~

**RESULT, 2026-09-18. Struck. Arm B pinned: `8d656ad8`** (only parent `08fe7b9f`). The "NO TIMING HAS BEEN TAKEN" line
above is now history. Same host, toolchain, ISA proof and §0 conditions as §6.
- 28 runs (16 required and 12 supplementary, n = 3 and 4), 21:34–21:57 +03:00.
- The 10 s CPU average was 0.60–3.42 % before each run. Two runs waited one 59 s poll first.
- During-run load not caused by the bench was 1.72–4.10 % of the machine (recorded on 20 runs).
- Contaminated runs: 0. **Discarded:** `full_step/1` A2. Its after-receipt was 16.66 % (a browser burst), and its CI
  overlaps no other A run's.

| row | A = 08fe7b9f (ms) | B = 8d656ad8 (ms) | A/A | B/B | B/A | n |
|---|---|---|---|---|---|---|
| **pyramid_sleeping_off (PRIMARY)** | 18.8551 / 18.4564 / 18.9987 / 18.5245 | 26.1900 / 25.2639 / 27.2549 / 26.9695 | 2.14 % (n=2), 2.90 % (n=4) | 3.60 %, 7.54 % | **1.379 (n=2), 1.412 (n=4)**; pairings 1.330–1.477 | 4+4 |
| full_step/1 | 18.6569 / 19.4885 / 18.9712 | 20.7566 / 20.9655 / 21.1604 / 21.1147 | 4.37 % | 1.92 % | 1.103 | 3+4 |
| full_step/4 | 10.8959 / 11.0667 | 11.6027 / 11.9273 | 1.56 % | 2.76 % | 1.071 | 2+2 |
| awake_sleeping_on: NOT like for like, never S5's price | 17.8255 / 17.0423 / 18.1057 / 17.0879 | 21.5734 / 21.0114 / 21.8375 / 20.2263 | 6.07 % | 7.61 % | 1.208 | 4+4 |

**R2: live contact points per timed step, each arm over its own window** (probe: `s9_probe.diff`; series in
`probe/*.csv`).

| row | A pts | B pts | points B/A | time B/A |
|---|---|---|---|---|
| sleeping_off | 14734.0 (steps 286–705) | 22957.6 (steps 158–367) | 1.558 | 1.379 / 1.412 |
| full_step/1 | 15532.7 | 17499.4 | 1.127 | 1.103 |
| full_step/4 | 14853.5 | 17494.6 | 1.178 | 1.071 |
| awake | 13510.2 | 17167.6 | 1.271 | 1.208 |

- **R1:** in a default world, S5 costs **1.38× to 1.41×** per step on the resting height-15 pile over the bench's
  timed steps.
- **R2 did not fire.** Every time ratio is below its point ratio (time ÷ points 0.89–0.98), so no profiling of the
  realized-patch check or the fallback is prescribed.
  - The time ratio sits between the manifold ratio (1.254) and the point ratio (1.558).
  - The two arms timed different windows. This does not bias the result: B's pile is flat across both windows
    (6675 / 6672 manifolds, 22 958 / 22 972 points), so the B/A over the common window 286–705 is the same to about
    0.1 %.
- **R3 did not fire** (B/A is above 1.0).
- **Awake row:** it runs with `sleep_threshold = 0`, so neither arm latches in it. The freeze asymmetry described
  above is not what this row measures.

Receipts: `docs/measurements/2026-09-18-physics-s6-s9/` (`timing_raw.md` §1 and the R2 table; `raw/s9/`;
`runlogs/s9_*`; `probe/` with `r2_window.py` and `r2_table.txt`; `s9_probe.diff`).

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

⚠ **Arm B is the S5 commit itself, `8d656ad8`** — the commit whose only parent is `08fe7b9f` and
which introduces `FACE_AXIS_PREFERENCE` — not a later tree.

```bash
# Arms: A = 08fe7b9f (A7a, before S5), B = the S5 commit (its only parent is 08fe7b9f; see above).
# Idle-machine receipt first (§0). No RUSTFLAGS (§1). Run A twice interleaved with B for the A/A spread.
cargo bench -p boyko-physics --bench sleeping_pipeline -- pyramid_sleeping_off       # PRIMARY: default config
cargo bench -p boyko-physics --bench jolt_parity_pyramid -- full_step/1              # Jolt's scene (gap 0.5)
cargo bench -p boyko-physics --bench jolt_parity_pyramid -- full_step/4
cargo bench -p boyko-physics --bench sleeping_pipeline -- pyramid_awake_sleeping_on  # sleeping on, never latching
# `-- full_step/N` is the Criterion bench's filter; it exists on both pinned arms. On a tree where
# jolt_parity_pyramid is the fixed-window runner (§10) there is no such filter, and the two Jolt-scene
# rows are §10's J-A row at W=1 and W=4, disarmed:
#   <runner exe> --scene jolt --gap 0.5 --cfg a --workers 1 --steps 500 --csv <file>
#   <runner exe> --scene jolt --gap 0.5 --cfg a --workers 4 --steps 500 --csv <file>
# The runner times steps 0..500 from spawn, not Criterion's time-sampled window after 20 warm steps,
# so a number from it is never divided by one of the Criterion numbers above.
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

## 10. Physics — the per-stage profile, and the Jolt parity row re-taken like for like (perf campaign P0) — TIMED 2026-09-19 (window 1 VOID; window 2 under the ruled protocol)

**RESULT, 2026-09-19.** Two windows. Window 1 VOID (below); window 2 ("P0b") timed under the orchestrator's
ruled protocol and complete. Owner's workstation: Ryzen 9 5900HS, 8C/16T (core k = CPUs {2k,2k+1}, 512 KiB L2
per core, 16 MiB L3), High performance, AC; Task Manager, Steam and a browser open. `D:/wt/joltab` at `dbd85977`,
clean; rustc 1.98.1 `host: x86_64-pc-windows-msvc`, no RUSTFLAGS, `x86-64-v3` baseline. Ancestors true: S5
`8d656ad8`, KE16 `67563d3b`, B1 `aff98fe7`, L1 `00c07d0f`; D1 + `simd_solve` default = `56c1e9e7`.
boyko runner: cargo `parity` (inherits `release`: fat LTO, default CGU), zone tier `dev` (SHIP: `shipping`);
sha256 tip `ef9325ef`, pre-instrument `b7c42334`, pre-L1 `a0da832f`, shipping `fdcac07f`. broadphase bench:
cargo `bench` (lto=false, CGU=1), `3c5cd8b4`. Jolt v5.3.0 Distribution `29b23ad1` / Release (`-p`) `de00c882`,
v5.6.0 Distribution `918fd2b7`; WinLibs GCC 16.1.0, IPO on, AVX2; source = the repository's `pyramid_scene.patch`.

**Window 1 — VOID (05:22:15–05:46:53 +03:00, passes 0–2 of 5).** A/A0 fired at W=2: B(2) = 2.311 % > 2 %
(d2/d1 = 0.98168, 0.98093, 0.97689). Not load: 307 of 308 receipts ≤ 4.9 % busy. Cause: each process of the
SAME binary draws its own speed for its whole run (W=2: 6 identical processes span 5.2 %; replaying the group
flips the sign of d2/d1); the band gated a median of paired step ratios while the quote is a window mean. No
timed number from window 1 is used. **Protocol ruling:** a cell's statistic is the median over K separate
processes of the window mean; resolution = the process spread (min–max, IQR; the median's SE
1.2533·SD/√K supplementary); a comparison is claimed only if |effect| > 2·hypot(spread_A, spread_B);
receipts gate each process (> 5 % ⇒ re-run once); no band-based void; the canary must be seen. **Placement
diagnostic** (06:13–06:36): pinning one thread per physical core did not narrow the spread in any
(engine, W∈{2,8}) cell (largest variance ratio 3.3 against F(5,5)₀.₀₅ = 7.15) and moved no median beyond
resolution ⇒ P-none (no affinity) for both engines.

**Window 2 (06:49:05–07:51:05).** K = 6 in all 43 cells (2 passes × 3 rounds, interleaved; pass 1 reversed;
one untimed warm-up per pass); 258 processes, 0 non-zero exits; 2 contaminated (after-receipts 14.0 % and
7.17 %) and re-run once. Receipts: 266, median 1.26 % busy; idle polls 9/9 quiet. Structural checks all
green: 0 void steps, drops 0, disarmed ring traffic 0; H7 12/12; 7 boyko pose groups with one pose each
(J family, sleep-off = sleep threshold 0 = pre-L1 = `0x32d5e235342b4143`); Jolt one hash per (row, W); waves
109.32 per step, identical at W=1 and 8, = 12 × wide colours on every step; closure: u ≤ 0.48 % of solve,
u, g ≥ 0; R-S first frozen step 248 (12/12); Jolt threads = W.

| W | boyko J-A (cfg-A, disarmed) ms | Jolt v5.3.0 | Jolt v5.6.0 | boyko/v5.3.0 | boyko/v5.6.0 |
|---|---|---|---|---|---|
| 1 | 19.671 [19.079–19.835] | 15.723 [14.987–16.088] | 9.650 [9.545–10.042] | 1.251 | 2.038 |
| 2 | 13.711 [13.445–13.819] | 8.794 [8.514–9.005] | 5.708 [5.666–5.785] | 1.559 | 2.402 |
| 4 | 10.701 [10.653–10.869] | 5.229 [5.139–5.374] | 3.557 [3.540–3.639] | 2.047 | 3.008 |
| 8 | 9.162 [9.103–9.250] | 3.533 [3.452–3.583] | 2.493 [2.467–2.589] | 2.593 | 3.675 |
| 16 | 9.172 [9.136–9.210] | 3.209 [3.189–3.277] | 2.447 [2.371–2.480] | 2.858 | 3.749 |

- Every ratio is claimed under range, IQR and SE. **cfg-A is not the tip's default**: it pins `simd_solve`
  off (default on since `56c1e9e7`) and sets `parallel_solve = W>1` (default off). Derived from J-B's spans,
  NOT a measured row: cfg-A + `simd_solve` ≈ 11.0–11.1 ms at W=1 (0.70× v5.3.0) and 7.9–8.1 ms at W=8
  (2.24–2.29× v5.3.0). Queue a measured row before quoting this.
- H8 fired, and it is the contact set: over [100,500) Jolt has 8,456 manifolds (receipt; confirmed by Jolt's
  own profiler, 8,456 cached-manifold adds per frame), boyko 4,519.3 (1.871×). Per manifold per step,
  boyko/v5.3.0 = 2.32, 2.88, 3.79, 4.73, 5.20 at W = 1–16; the truth lies between that and the raw ratio.
  v5.6.0's own count is not receipted.
- H1, boyko/v5.3.0 over [0,100) / [100,500): 1.295/1.240 at W=1, 2.903/2.527 at W=8.
- Scaling T(1)/T(8): boyko 2.147 (T(16) = T(8), +0.11 %, not claimed), v5.3.0 4.450, v5.6.0 3.870.

**Profile (armed J-A, W=1 / W=8).** S(1) = 7.398 ms; **f = S(1)/T(1) = 0.376**, f_fit = 0.393 (0.390 from
disarmed T), so f_fit − f = +0.017. P(1) = 12.205 ms. S(8) = 6.939 ms (75 % of T(8)); **I(8) = −0.459 ms**
(broadphase −7.4 %, narrowphase −6.3 %, both claimed). L(8) = 0.715 ms; E(8) = 0.681; ω(8) = 6.54 µs;
ω₁ = 0.855 µs (claimed only under SE). g = 89.0 / 34.6 µs; u = 1.41 / 1.37 µs; r = 4.32 / 4.73 µs;
identity residual −6.6 / +9.0 µs. The L6 fork ω₁·waves/L(8) = 0.13 (≤ 0.28 over every process pairing).
Armed J-A was run at W = 1 and 8 only, so I, E and L at W = 2, 4, 16 are not measured.

**W=8 gap (5.63 ms) beside Jolt `-p` (Release; profiled T is +24 % at W=1 and +15 % at W=8 over timed).**
- Collision: boyko bp + np is 4.891 ms and serial, against Jolt FindCollisions 0.67–0.90 ms (parallel, a
  cache replay of 8,456 of 8,541 pairs). That is 71–75 % of the gap.
- Solve: 4.145 ms against 2.62–2.84 ms, 23–27 % of the gap. boyko's serial in-solve work is 1.89 ms.
- Jolt's wall-coverage shares: SolveVelocity 66.19 %, FindCollisions 25.48 %, SolvePosition 6.93 %.
- Δ_J = +1.552 ms (+43.9 %, claimed).

**Rows.**
- J-B against J-A-a: −27.9 % at W=1, **+21.0 % (slower) at W=8**. Grid adds +3.07 and +3.09 ms;
  `simd_solve` makes the colour spans 3.41× and 2.34× faster.
- D1: R-ref/R = 1.682 (+68.2 %): the colored default saves 9.70 ms per step at W=1 on the gap-0 pile.
- Sleeping floor F (R-S): 6.049 ms (W=1) and 6.123 ms (W=8); 92 % of it is collision + graph.
  L10 on R: −57.5 % (W=1), −38.4 % (W=8).
- O5 tails [264, 1000): boyko 5.334 / 5.407 ms against Jolt 2.17 / 2.29 µs (≈2,400×).
- S16: 81.4 / 79.4 µs.

**Gates.**
- A/A1 (arming): +0.11 % at W=1, +0.74 % at W=8. Not claimed: pass.
- A/A2: +0.51 %, +0.30 %. Not claimed: pass, and the zones stay in `dev`.
- SHIP against dev: −0.55 %, −0.53 %. Not claimed.
- Canary (J-C against J-A-a): +5.06 % and +4.81 % against 5.00 % and 4.98 % injected. Seen under every
  reading; the span reads within +0.05 %. Against the disarmed J-A at W=1, the min–max bar (7.9 %) would
  hide it.
- L1's gate (J-S0 disarmed, pre-L1 against the tip): +0.68 %, not claimed ⇒ no regression. Bars: 8.9 %
  (range), 2.8 % (IQR), 1.7 % (SE). The +2.44 % L1 targets is not seen; under SE, a gain of 1 % or more is
  excluded.

**Broadphase crossover (K=1).** Uniform n\* ≈ 1,109; size-disparity n\* ≈ 2,978; the in-scene
Grid/AllPairs at n = 1240 is 2.46 (W=1) and 2.59 (W=8). `GRID_LO` = 96 and `GRID_HI` = 192 sit 6–31× below
the crossovers. O3 Gate 7 read 1.92× at 100k (w4 against w1), against ≥ 2.8×; this is not ruled on.

**Resolution at K=6.** The smallest claimable end-to-end change, as range / IQR / SE, is:
- J at W=1: 10.9 / 1.4 / 2.0 %;
- J at W=8: 4.5 / 0.5 / 0.75 %;
- R at W=1: 17.8 / 3.3 % (range / SE);
- R at W=8: 7.0 / 1.3 %.

Which spread the claim rule means (min–max against IQR against SE) is still OPEN
(`p0b/diag_report.md` §6). Resolving 1 % under SE needs K ≥ 12. A witness, not a gate: the fast W=1
processes are the ones whose thread stayed on one CPU (occupancy against deviation, r = −0.67, n = 48).

**Levers (plan §3 as amended by W2/W3).**
- L4 `parallel_solve` default: build. **Built (`caac7d06`, on by default); untimed until the L4/L5 window.**
- L5 parallel narrowphase: build, predicted 2.4–2.6 ms (26–28 % of T(8)). **Built (`b8d9ab8f`, dormant) and on by default from L5 C4; untimed until the window (C2 against C4).**
- L2: do not flip; recalibrate `GRID_LO`/`GRID_HI`.
- L6: not built (L(8) + t_narrow(8) = 8.2 % < 10 %).
- L7: blocked (gated after L6; 2.8–5.1 % predicted).
- L9: fails its rule once L5 lands (t_np(8) → 8.7 % < 20 %; Δ_J = 43.9 %).
- L8 (D2): clears after L5; ≤ 1.16 ms (cfg-A), ≤ 0.64 ms (simd).
- L10 (D3): parity 0; −38 to −58 % on the resting world.
- L3: shipped.
- Not a plan lever: the serial AllPairs broadphase is 21 % of T(8).

Receipts: `docs/measurements/2026-09-19-physics-p0/`:
- `p0/` (window 1 and the builds): `build_report.md`, `window_report.md`, `manifest.json`, `bin/SHA256SUMS`,
  `raw/runs_all.jsonl`, `raw/runs.jsonl`, `raw/reduction.json`, `raw/pass-0{0,1,2}/`, `raw/diag_w2/`.
- `p0b/` (the diagnostic and window 2): `diag_report.md`, `window_report.md`, `raw/runs.jsonl`,
  `raw/analysis*.json`, `raw/window/runs.jsonl`, `raw/window/pass-0{0,1}/`, `raw/window/broadphase/`,
  `raw/window/window_state.json`, `raw/window/wait_log.txt`.
- `p0/dry/`, `p0/wtest/` and `p0b/test/` are rehearsals, not measurements. Leave out `__pycache__/`.

---

**The run sheet as queued** (kept as history):

~~NO TIMING HAS BEEN TAKEN.~~ Timed 2026-09-19; see the RESULT block above. Queued 2026-09-19. The design is `docs/physics/perf-campaign/01-PLAN-REV1.md` §1–§2
together with `00-RULINGS.md`, which overrides the plan where the two differ. This entry is their run sheet. Where
it disagrees with either of them, they win and this entry is stale.

⚠ **Written before the things it runs existed.** `[profile.parity]`, the physics zone sites, the fixed-window
runner (the rewrite of `benches/jolt_parity_pyramid.rs`), the Jolt patch and the `tools/` driver had not landed.
The runner flags below are spelled as in the plan's implementation step 6. Check them against the runner's usage
text before building the window's binaries.

**Decides:**
- which bit-identical levers (plan §3, L2–L7) are built, and in what order. Each has a "build if" on this
  profile's spans. Each is then gated on median T(W), W ∈ {1, 2, 4, 8, 16}, on J and R, against the A/A0 band,
  with I(W) reported before and after (W3);
- the serial fraction as a measurement, f = S(1)/T(1), in place of the Amdahl fit behind the 0.43–0.53 in
  `OPEN-QUESTIONS.md`'s 2026-09-10 Jolt entry;
- the boyko/Jolt ratio per W, like for like (plan §1, H1–H12), on the msvc host. The 2026-09-09 and 2026-09-10
  rows are labelled "pre-A7, likely gnu" and are never compared with it;
- L1's gate (§8's R2 follow-up).

**Precondition.** §0, and the owner confirms the window first. The plan's estimate is about 40 minutes: 5 passes
of about 5.5 minutes each, plus receipts and polls. Nothing compiles inside the window.

**Built before the window** (msvc host, no `RUSTFLAGS`, §1). Every arm builds to the same exe path, so copy each
exe out and record its sha256 before the next build. A swapped source file is swapped and restored by copy, with
sha256 proof both ways.

| binary | tree | build | used for |
|---|---|---|---|
| runner | the tip | `parity`: inherits `release` (fat LTO, default codegen units), the shipped profile (W1) | every boyko row |
| runner, pre-instrument | the tip without the physics zone sites | `parity` | A/A2 |
| runner, pre-L1 | the tip with L1 reverted. `resources.rs` as at `1c31aeac` (blob `8ed4373a`) is that file if no later commit touched it; otherwise reverse-apply L1's diff | `parity` | L1's gate |
| runner, `shipping` tier | the tip, `BOYKO_PROFILE=shipping` | `parity` | one disarmed row, because Jolt's Distribution compiles its profiler out (O6) |
| `broadphase` bench | the tip | `bench` | the crossover (§4 step 4), which calibrates `GRID_LO` / `GRID_HI` |
| Jolt v5.3.0 `PerformanceTest`, patched | `D:/tmp/jolt` | Distribution | the headline |
| the same, patched | `D:/tmp/jolt` | Release | `-p` stage shares at W=1 and W=8; shares only, never times |
| Jolt v5.6.x `PerformanceTest`, patched | — | Distribution, same recipe | a second column: Jolt's per-manifold friction on this box (its release notes: −15 % on Pyramid) |

A single-codegen-unit profile may also exist for lever A/Bs. Every number names its profile (W1).

The Jolt patch (`crates/boyko_physics/benches/jolt_parity/pyramid_scene.patch`) does four things:
- sets damping to 0 in `PyramidScene.h` (H3);
- adds `-no_pair_cache`, which sets `mUseBodyPairContactCache = false` (H10);
- threads `-allow_sleep` into `StartTest`. The scene hard-codes `mAllowSleeping = false` per body, and only
  `-no_sleep` exists (O5);
- adds `-receipt`, untimed, which records per frame the manifold count (through a counting `ContactListener`)
  and the top box's y (H8).

If v5.6.x needs other CMake options, or changed defaults beyond friction, record them in the receipt rather than
patching them away.

```bash
# Before the window: one build per row of the table above, each exe copied out and hashed.
cargo bench --no-run --profile parity -p boyko-physics --bench jolt_parity_pyramid
cargo bench --no-run -p boyko-physics --bench broadphase
# In the window every binary runs by path. J-A at W=8, disarmed, for example:
<runner exe> --scene jolt --gap 0.5 --cfg a --workers 8 --steps 500 --csv <file>
```

**Receipts, per run (H6).** No receipt, no quoted ratio.
- HEAD, and `rustc -vV`'s `host:` line, which must read `x86_64-pc-windows-msvc`.
- `git merge-base --is-ancestor` for S5 (`8d656ad8`) and KE16 (`67563d3b`), both expected true, and for the B1 fix
  and L1 commits.
- Every binary's sha256, and the Jolt binary's compiler.
- The thread counts on both sides at every W. Jolt runs W−1 workers plus the calling thread. Record whether boyko's
  dispatcher thread runs scope tasks at W=16.
- The load receipt before and after every timed region (§0).

**The boyko rows.** Each is the runner at the tip, one world per process: the profiler store binds one world, and
a second world is refused with E9204 while its fold returns silently. Windows are step ranges that the driver
reduces from the per-step CSV. cfg-A is colored, `parallel_solve = (W > 1)`, `AllPairs`, `simd_solve` off — and from
L5 C3 on `parallel_narrowphase = (W > 1)` too, so a P0 cfg-A row is comparable with a post-L5 cfg-A row only as the
"before" of the L5 pairing (C2 against C4), never quoted against it directly. cfg-B
is cfg-A plus `Grid` plus `simd_solve`, and the runner asserts that the two give equal final-pose bytes (H7). Every
row runs armed for its profile. The rows compared on wall time also run disarmed: J-A (the parity row and A/A0)
and J-S0 (L1's gate).

| id | runner flags | W | steps (window) | purpose |
|---|---|---|---|---|
| J-A | `--scene jolt --gap 0.5 --cfg a` | 1, 2, 4, 8, 16 | 500 (0..500) | disarmed ×2: the parity row and A/A0; armed: the profile |
| J-B | `--scene jolt --gap 0.5 --cfg b` | 1, 8 | 500 (0..500) | what `Grid` and `simd_solve` buy, per stage |
| J-P1 | `--scene jolt --gap 0.5 --cfg a --parallel-solve` | 1 | 500 (0..500) | ω₁: the per-wave cost with no cross-thread wake. **Retired at L4** (lever rulings, L5 W2): a one-worker pool now solves inline, so the runner refuses this row; ω₁ comes from a zero-work spawn/join microbench |
| J-C | `--scene jolt --gap 0.5 --cfg a --canary-frac 0.05` | 1, 8 | 500 (0..500) | the canary: a system `.after(narrowphase).before(build_graph)` spinning 0.05·T(W) |
| J-Son | `--scene jolt --gap 0.5 --cfg a --sleeping` | 1, 8 | 1000 (0..1000) | the sleeping floors, against Jolt `-allow_sleep` |
| J-S0 | `--scene jolt --gap 0.5 --cfg a --sleeping --threshold 0` | 1 | 500 (0..500) | the sleep bookkeeping cost; L1's gate |
| R | `--scene rest --solver colored`; serial at W=1, `--parallel-solve` at W=8 | 1, 8 | 1100 (600..1100) | the resting cost per stage, `PhysicsConfig::default` |
| R-ref | `--scene rest --solver reference` | 1 | 1100 (600..1100) | prices D1 |
| R-S | `--scene rest --sleeping` | 1, 8 | 800 (300..800) | the sleeping floor F |
| S16 | `--scene s16 --cfg a` | 1, 8 | 300 (0..300) | small-scene regression guard |

`--scene rest` is `sleeping_pipeline.rs`'s `pyramid_sleeping_off` scene: the exactly touching (gap 0)
height-15 pile.

**The Jolt rows.** `-q=Discrete` is kept because it halves the runtime, not to match anything:
`PyramidScene::StartTest` never reads the motion quality, and the default is Discrete (O4).

| run | flags | W | pairs with |
|---|---|---|---|
| timed | `-s=Pyramid -q=Discrete -t=W -f` | 1, 2, 4, 8, 16 | J-A |
| no pair cache | the timed flags + `-no_pair_cache` | 8 | Δ_J in the W=8 attribution |
| sleeping | the timed flags + `-allow_sleep` | 1, 8 | J-Son |
| stage shares | the Release build, the timed flags + `-p` | 1, 8 | the W=8 attribution (shares only) |
| v5.6.x | the timed flags | 1, 2, 4, 8, 16 | the second column |
| receipt | `-s=Pyramid -q=Discrete -receipt`, untimed | any, recorded | H8 |

**Protocol (H12).** Prebuilt binaries are invoked by path, and their output is read as bytes, never with
`text=True`. Interleave by W, Jolt then boyko, and reverse the order on alternate passes. 5 passes. Every statistic
is a median over passes.

**Void and stop rules.** Each one can fail.
- **A/A0, the band.** Two disarmed J-A runs per pass, paired by step index. B(W) = max over passes of
  |median_k(t₂/t₁) − 1|. If B(W) > 2 %, the window is not quiet: void it.
- **A/A1, perturbation.** Armed against disarmed J-A, same pairing. Passes iff |median − 1| ≤ max(B(W), 0.5 %) at
  every W.
- **A/A2, the permanent sites.** The disarmed tip against the pre-instrument runner at W=1 and W=8, same criterion.
  If it fails, the physics zones move to a tier that `dev` does not compile.
- **The canary, J-C against J-A.** All three must hold:
  - the canary span reads 0.05·T ± 5 %;
  - the step's wall time rises by the same amount, within B(W);
  - the A/A1 statistic computed on J-C against J-A FAILS.

  The canary exceeds the 2 % ceiling by construction, so a green here means the gate is blind.
- **Anti-vacuity, per run (W4).** Each zone's count per step must equal its structural expectation, otherwise the
  row is void: build 1; gravity, warm apply, integrate and the biased pass 4 each; the relax pass 8; one span per
  system. Armed runs record samples > 0, and disarmed runs record 0. The jolt scene holds exactly 1240 dynamic
  bodies.
- **Determinism.** Armed, disarmed, cfg-A and cfg-B final-pose bytes are all equal.
- **Closure.** Any of these is an instrument defect and stops the analysis:
  - u(W) > 3 % of the solve span, or u < 0;
  - g < 0;
  - Σ system spans > the step's wall time;
  - the region overflow counter ≠ 0;
  - wave counts that differ across W ≥ 2, or that disagree with 12 × (colors with ≥ 256 slots) from the
    `phys_slots_*` counters.

  The residue r = Σ pass spans − Σ color spans joins the identity and is bounded (O2). Its bound is not set yet.
- **R-S.** If any dynamic row is not frozen at step 300, the row is void (A7-R2 froze at step 248 on the kernel as
  committed), and D3 is priced from J-Son alone.
- **H8.** If the two sides' resting-window manifold counts differ by more than 10 %, the headline also carries time
  per manifold per step.
- **O5.** The engines sleep by different rules: boyko at speed² below 1e-4 for 60 frames, Jolt at about 0.03 m/s for
  0.5 s. Record the awake count per step on both sides for J-Son and `-allow_sleep`. Compare only the tails after
  everything is asleep, never the window means.

**Derived** (plan §2). t_s(W) is the median over passes of span s's window mean.
- S(W) = Σ serial spans. f = S(1)/T(1). The fit f_fit, from T(1) and T(8), is printed beside it; f_fit − f is what
  the fit misattributed.
- P(1) = Σ wide-color spans at W=1 with `parallel_solve` off.
- T(W) = S(1) + I(W) + P(1)/W + L(W) + g(W) + u(W) + r(W), where:
  - I = S(W) − S(1);
  - L = t_wide(W) − P(1)/W;
  - g = T − Σ system spans;
  - u = solve span − Σ in-solve zones.
- E(W) = P(1)/(W·t_wide(W)). waves = wide spans per step.
- The L6 fork compares a FIXED per-wave dispatch cost against L(8): ω₁ from J-P1, or a microbenched spawn/join at
  zero work (the only source from L4 on, which retired J-P1). The imbalance share (max chunk − mean chunk) is
  reported separately (W2).
- The W=8 gap attribution: boyko's terms beside Jolt's `-p` stage shares, plus Δ_J = T_J(no pair cache) − T_J.

**L1's gate (O7).** J-S0 at W=1, disarmed: pre-L1 (A) against the tip (B). In each pass, as in §8, run A twice
interleaved with B for the A/A spread. L1 passes iff the B/A median is ≤ 1 within that spread. The correctness half
is untimed: the suites §8 names (`sleep_settles_box_piles.rs`, `support_loss_wakes_sleepers.rs`, and
`resources_tests.rs`'s `rekey_rows` group) are green on the tip.

**Quoting** (plan §1, W1). boyko/Jolt is the median over passes of the mean step time (Jolt's metric, 500 / Σt),
per W. It names the cfg and the profile, comes from the msvc host, and carries the H6 receipts. Report beside it the
sub-windows [0, 100) and [100, 500) (H1), and the time per manifold-sweep (H9).

**Report:**
- per W and row: the medians and B(W);
- the A/A1, A/A2, canary, anti-vacuity, determinism and closure verdicts;
- f beside f_fit, and every term of the identity;
- waves and ω;
- Jolt's stage shares and Δ_J;
- the H8 receipt, the v5.6.x column and the `shipping` row;
- L1's B/A with its A/A spread;
- the broadphase crossover n.

Receipts go in `docs/measurements/<date>-physics-p0/`.

---

## When an entry is done

Strike it with the date and the receipt's location, rather than deleting it. An entry that was run
and produced a surprising number is worth more as history than as a blank line.
