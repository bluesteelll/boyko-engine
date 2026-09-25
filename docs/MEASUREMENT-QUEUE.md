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

## 10. Physics — the per-stage profile, and the Jolt parity row re-taken like for like (perf campaign P0) — TIMED 2026-09-19 (window 1 VOID; window 2 under the ruled protocol); **window 3 TIMED 2026-09-21 (the measured default row, L4+L2 and L5)**

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
  (2.24–2.29× v5.3.0). **Measured 2026-09-21, window 3 (below): 10.596 ms at W=1 and 7.783 ms at W=8
  (1.078x and 3.029x Jolt v5.6.0, the reference since the owner's 2026-09-21 ruling); the estimate was
  4-5 % and 2-4 % high. Quote the measured row, not this estimate.**
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
- L4 `parallel_solve` default: build. **Built (`caac7d06`, on by default); timed in window 3 (2026-09-21): the
  default row (L4+L2, simd on) reads 10.596 / 7.783 ms at W=1 / 8.**
- L5 parallel narrowphase: build, predicted 2.4–2.6 ms (26–28 % of T(8)). **Built (`b8d9ab8f`, dormant) and on by default from L5 C4; timed in window 3: -2.435 ms at W=8 (C4 against C2) and 2.626 ms for the flag alone, claimed; predicted 2.4-2.6.**
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

**RESULT, 2026-09-21, window 3 (the measured default row).** One window, complete, under the 2026-09-19
protocol ruling (median over K separate processes of the [0,500) window mean; spread = min-max, IQR, with
the median's SE beside them; claimed iff |effect| > 2*hypot(spread_A, spread_B); 5-s receipts before and
after every process, > 5 % => re-run once at the end of the pass; 10-s receipt and three quiet 60-s polls
before each pass; no band void; P-none). **Owner ruling 2026-09-21 04:10 (binding): compare only with Jolt
v5.6.0 - every ratio, sentence and table column in this block is boyko against Jolt v5.6.0.** Owner's
workstation as on 09-19 (Ryzen 9 5900HS, 8C/16T, High performance, AC; the owner's agent sessions and a
browser open - see contamination). Timed 03:50:08-04:42:16 +03:00, after both running lanes finished (l5np
HEAD `de06b6c9` at 03:44, lighttable `8e9cd328` at 03:17; last lane build process seen 03:37). rustc 1.98.1
`host: x86_64-pc-windows-msvc`, no RUSTFLAGS, `CARGO_INCREMENTAL=0`, cargo `parity` (inherits `release`:
fat LTO, default CGU), zone tier `dev`, disarmed. Trees, built in detached worktrees `D:/wt/mq-<sha>`:
`aac562a7` (= L2 = C2; L4 `caac7d06` and L2 are the only code commits above the P0 tip `dbd85977`) ->
`runner_l4l2` sha256 `368d4104`; `de06b6c9` (L5 C4, on top of C3 `b8d9ab8f`) -> `runner_l5` `26d17a10`; the
P0 tip `runner_tip` `ef9325ef` re-used for the bridge. Jolt v5.6.0 `918fd2b7`, unchanged from 09-19, hash
re-verified (the P0b-reference Jolt build likewise; see the history footnote under the table). Ancestors
true for S5, KE16, B1, L1, `56c1e9e7`, `dbd85977`. `--cfg default` prints: colored, `simd_solve` on,
`parallel_solve` on, broadphase AllPairs under `broadphase_select` **Manual** (C2's Auto band is not
exercised by any row), sleeping off, substeps 4, relax 2; at `de06b6c9` also `parallel_narrowphase` on
(`--parallel-np off` for the same-binary A/B).

**Rows.** `D-L4L2` (default at aac562a7; W 1/2/4/8/16), `D-L5` (default at de06b6c9; W 1/2/4/8/16),
`D-L5-npoff` (W 8), `J-A` (tip, cfg-A, disarmed; W 1/8, the bridge), Jolt v5.6.0 timed
(`-s=Pyramid -q=Discrete -f`; W 1/2/4/8/16) and the P0b-reference Jolt build at the same W (footnote):
23 cells, 2 passes x 3 rounds, order interleaved by W with Jolt and boyko alternating, pass 1 reversed, one
untimed warm-up per pass; 138 slots, 31 re-runs, **K=6 in 22 cells and K=5 in `D-L4L2@W1`** (its original
and its re-run were both contaminated). 0 non-zero exits. Structural checks: one boyko pose
`0x32d5e235342b4143` in all 77 boyko processes (every row, every W; `--expect-pose` match in the 47 that
carried a reference, plus the untimed 500-step gate at W 1/8/16 with a 501-step red control), Jolt v5.6.0
one hash `0xb8522b4e3fc62cfe` in all 30 of its processes; void 0, ring traffic 0, drops 0; workers = W on
both sides; mask `0xffff` everywhere.

**Contamination (the difference from 09-19).** 203 5-s receipts: median 2.03 % busy, p90 6.6 %, max
22.5 %, **32 over 5 %** (09-19: 2 of 266), 30 of them in pass 0 (03:50-04:28) - `claude.exe` sessions and
one browser burst, never a build. 13 used pass-0 processes sit 6-31 % above their cell median with
bracketing receipts under 5 % (during-process witness 0.5-5.1 %); the medians absorb one such process per
cell, the min-max ranges of 12 cells do not (6-34 %). Pass 1 (04:30-04:42) was quiet (2 contaminations).
Where a claim below holds under IQR and SE but not min-max, that is the reason.

| W | boyko default L4+L2 (`D-L4L2`) | boyko default +L5 (`D-L5`) | J-A bridge (cfg-A) | Jolt v5.6.0 | D-L4L2 / v5.6.0 | D-L5 / v5.6.0 |
|---|---|---|---|---|---|---|
| 1 | 10.596 [10.549-10.759] (K=5) | 10.738 [10.684-10.793] | 19.483 [19.337-19.644] | 9.828 [9.518-11.364] | 1.078 | **1.093** |
| 2 | 9.097 [8.944-10.184] | 7.864 [7.767-8.744] | - | 5.770 [5.703-5.818] | 1.576 | 1.363 |
| 4 | 8.280 [8.196-9.561] | 6.173 [6.106-6.710] | - | 3.581 [3.519-3.663] | 2.312 | 1.724 |
| 8 | 7.783 [7.693-8.042] | 5.347 [5.307-5.651] | 9.174 [9.070-9.243] | 2.569 [2.501-3.124] | 3.029 | **2.081** |
| 16 | 8.161 [8.084-10.040] | 5.552 [5.480-6.838] | - | 2.388 [2.337-2.465] | 3.417 | 2.324 |

History (P0b's reference): Jolt v5.3.0 (Distribution `29b23ad1`, pose `0xee15b89965ec747`) was timed in this
window at every W as on 09-19; its timings stay in the raw receipts (`raw/runs.jsonl`; `analysis.md`
sections 1 and 4) and are not quoted here - the owner made v5.6.0 the reference on 2026-09-21.

- **The bridge holds.** J-A re-taken: 19.483 (W=1) and 9.174 ms (W=8) against 09-19's 19.671 and 9.162:
  -0.96 % and +0.13 %, not claimed under any reading (bars 8.3 / 1.6 / 1.5 and 5.0 / 1.8 / 0.9 %); the
  Jolt v5.6.0 cells reproduce within -2.4..+3.1 % (+1.85 / +1.10 / +0.67 / +3.05 / -2.38 % at W = 1-16),
  none claimed under range or IQR. This window's rows may be set beside the 09-19 rows.
- **The measured default row replaces the derived "cfg-A + simd_solve" estimate.** D-L4L2 against J-A,
  in-window: -45.6 % at W=1 and -15.2 % at W=8, claimed under all three readings. Against the estimate
  (11.0-11.1 / 7.9-8.1 ms) the row reads 10.596 / 7.783: 4-5 % and 2-4 % faster than derived. Against 09-19's
  armed J-B (Grid + simd): -25.4 % / -30.3 %, claimed - Grid's +3.07 / +3.09 ms with the sign reversed.
- **L4+L2 against Jolt v5.6.0.** 1.078x at W=1 (+7.8 %; claimed under IQR and SE, not min-max); at W=8
  3.029x (all three); at W = 2 / 4 / 16 1.576x / 2.312x / 3.417x (all three). Per manifold per step over
  [100,500), each side's own count (boyko 4,519.3; **v5.6.0 8,489.0, receipted for the first time**): 2.05,
  2.99, 4.32, 5.65, 6.31 at W = 1-16. H1 sub-windows [0,100) / [100,500): 1.013 / 1.091 at W=1, 3.216 / 3.006
  at W=8. Scaling T(1)/T(8) = 1.362 (Jolt v5.6.0 3.825); T(16) against T(8) +4.9 %, claimed under IQR only.
- **L5 (C4 against C2), the W3 gate on the default row.** At W=8: **-2.435 ms (-31.3 %)**, claimed under all
  three (bars 15.7 / 5.3 / 3.0 %); same binary, `--parallel-np off` against on: **+2.626 ms (+49.1 %)**,
  claimed under all three (36.3 / 9.6 / 7.0 %) - the prediction was 2.4-2.6 ms, the design's pass rule
  >= 1.33 ms. At W = 2 / 4 / 16: -13.6 / -25.5 / -32.0 %, claimed under IQR and SE. Equal knobs across the
  two binaries (`D-L5-npoff` against `D-L4L2` at W=8): +2.4 %, not claimed. D-L5 T(1)/T(8) = 2.008; T(16)
  against T(8) +3.8 %, not claimed. **At W=1 D-L5 reads +1.34 % over D-L4L2 (+0.142 ms): claimed under SE
  only** (bars 4.5 / 1.5 / 1.0 %); +0.2 % in pass 0 and +1.7 % in pass 1; different binaries, and the C4
  code takes the serial loop on a one-worker pool, so this is not the flag - a same-binary `--parallel-np off`
  W=1 row at K=12 prices it and was not run.
- **The owner's question (W=1, shipped default `de06b6c9`, against Jolt v5.6.0): slower by 9 % (1.093x),
  claimed under IQR and SE, not under min-max (one 11.364 ms Jolt process; without it 1.098x, still IQR and
  SE only). Per manifold 2.08x - slower on both readings; the contact set is 1.88x smaller (4,519.3 against
  8,489.0 manifolds per step), which is what separates the two readings. At W=8 2.081x and at W=16 2.324x,
  claimed under all three; the L4+L2 row (D-L4L2) reads 1.078x at W=1 (+7.8 %, IQR and SE). The default row
  is behind v5.6.0 at every W.**

**Not claimed / not measured.** D-L5 vs D-L4L2 at W=1 beyond SE; boyko W16 against W8 (D-L5 +3.8 %,
n/n/n; D-L4L2 IQR only); Jolt v5.6.0 W16 against W8 (-7.0 %, n/n/n); the binary drift at W=8 (+2.4 %);
boyko / v5.6.0 at W=1 under min-max. Not run: armed rows (no I(W), E(W) or spans for C2/C4), the J-C canary
on C4, the R rows, J-A at C2/C4 (the design's own row for the L5 rule), an Auto-broadphase row (no runner
flag), J-P1 (retired at L4). The claim-rule question (range / IQR / SE) is still OPEN in this block while
`00-RULINGS.md`'s post-P0 ruling names SE; every claim above is printed under all three.

Receipts: `docs/measurements/2026-09-21-physics-window3/`: `build_report.md`, `window_report.md`,
`analysis.md` (this reduction), `rows.json`, `rows_l5.json`, `wait_log.txt` (the lane flag poll),
`lanes_done.flag`, `bin/SHA256SUMS`, `logs/`, `raw/runs.jsonl`, `raw/manifest.json`,
`raw/window_state.json`, `raw/window_log.txt`, `raw/wait_log.txt`, `raw/pass-0{0,1}/`, `raw/receipts/`,
`raw/analysis.json`, `raw/tables.md`, `tools/`. `dry/` and `test/` are rehearsals, not measurements. Leave
out `__pycache__/` and the exes.

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

## 11. Physics — the Tree broadphase (C3) against AllPairs and Jolt v5.6.0: G4 bench and build-if, G5 end to end — TIMED 2026-09-22 (window 4); **the C4 default flip DEFERRED by the stop rules**

**RESULT, 2026-09-22, window 4.** One window, complete, under the window-3 protocol block (section 10:
median over K separate processes; spread = min-max, IQR, with the median's SE beside them; claimed iff
|effect| > 2*hypot(spread_A, spread_B), printed under all three readings r / i / s; 5-s receipts before and
after every process, > 5 % => re-run once, the original kept; 10-s receipt and three quiet 60-s polls before
each pass or block; no band void; P-none). **Jolt v5.6.0 only (owner ruling 2026-09-21): its window-3 cells
are reused, never re-run; no v5.3.0 row anywhere.** Owner's workstation as on 09-21 (Ryzen 9 5900HS, 8C/16T,
High performance, AC); the only contaminant was the Claude desktop app's window (G4: 10 after-receipts over
5 %, every one re-run clean; G5: 0 over 5 %, 0 build processes in 314 receipts). G4 timed 03:48:22-07:54:32
+03:00, G5 10:16:16-10:44:10, after a second timed window (`scratchpad/win4b`) had finished - the G5 driver's
first launch was stopped by the tester during its idle wait because that window's runners were invisible to
the lane-target rule (nothing timed). rustc 1.98.1 `x86_64-pc-windows-msvc`, no RUSTFLAGS, `CARGO_INCREMENTAL=0`,
`-C target-cpu=x86-64-v3`; one tree, lane `D:/wt/mq-de06b6c9` at **`a46b8287`** (C1 `ecbfe416` + C3; the
integration line carries the same `src/broadphase_tree/` at `c52ad183`, `tests.rs` +21/-5 apart): the parity
runner `runner_c3` sha256 `19456ab4` (cargo `parity`: fat LTO, default CGU, zone tier `dev`) for G5, the
`bench`-profile exes `broadphase` `efb84636` and `row_identity_churn` `d8ba73fc` for G4. `--broadphase
tree|allpairs` is the same-binary A/B; `--cfg default` is `PhysicsConfig::default()` as shipped (colored,
`simd_solve`, `parallel_solve`, `parallel_narrowphase` on, AllPairs under `broadphase_select: Manual`, sleeping
off, `tree_brute_max_rows 64`); `--cfg a` pins `simd_solve` off and `parallel_solve` = `parallel_narrowphase` =
(W > 1).

**Rows.** G5: `T-A-tree` / `T-A-allpairs` (cfg-A, W 1/2/4/8/16; the design's G5 headline row), `T-D-tree` /
`T-D-allpairs` (default, W 1/8; `T-D-allpairs` = window 3's `D-L5` re-taken, the bridge), `R-tree` /
`R-allpairs` (`--scene rest`, W 1/8), `S16-tree` / `S16-allpairs` (W 1, the brute path), `T-A-tree-armed` /
`T-A-allpairs-armed` (`--arm-profiler`, W 1/8), `T-C-tree` (the canary, `--canary-frac 0.05` against the same
pass's `T-A-tree`, W 1/8): 11 rows, 26 cells, 2 passes x 3 rounds, tree/allpairs alternating inside a W group,
pass 1 reversed, one untimed warm-up per pass; 156 timed processes + 1 re-run, **K=6 in every cell, 0 excluded
slots, 0 contaminated**. G4 (criterion, `bench` profile): `bp_g4_uniform`, `bp_g4_disparity`, `bp_g4_scene`
(n 17 .. 100k; the scene family 1240 / j100 / 10k / 100k) x `all_pairs`, `grid_w1`, `grid_w8`, `tree`;
`bp_g4_maintenance` (m 1240 / 10k / 100k x 7 arms) - K=3 whole-group processes, groups interleaved;
`row_identity_churn` 24 cells (6 arms x sleeping off/on x allpairs/tree), K=6, one cell per process: 156
processes + 10 re-runs, 117 cells, 0 excluded. Structural checks: one pose per (scene, cfg) across kinds and W -
J `0x32d5e235342b4143` (cfg default AND cfg a, the window-3 `D-L5` hash), R `0xee2a67a98434919a`, S16
`0x71313833f6a8e645`; `expect_pose` match in the 72 processes with a twin, none in 84, 0 mismatch on the used
set; void 0 everywhere; TreeDiag on every J and R tree process `static_rebuilds 1, members 1`, all other
counters 0; every allpairs / s16 process all zeros; G4's 78 structural receipt keys byte-identical across every
process of every cell and equal to the recipe's dry-run counts; every window mean recomputed from the 500
`wall_ns` values matches the driver's exactly.

| W | `T-D-tree` (default + Tree) | `T-D-allpairs` (default, = D-L5 re-taken) | `T-A-tree` (cfg-A + Tree) | Jolt v5.6.0 | T-D-tree / v5.6.0 | T-D-allpairs / v5.6.0 | T-A-tree / v5.6.0 |
|---|---|---|---|---|---|---|---|
| 1 | **9.026** [8.918-9.092] | 10.554 [10.464-10.738] | 17.889 [17.791-18.108] | 9.828 [9.518-11.364] | **0.918** (-8.2 %; n/Y/Y) | 1.074 (n/Y/Y) | 1.820 (Y/Y/Y) |
| 2 | - | - | 10.588 [10.511-10.692] | 5.770 [5.703-5.818] | - | - | 1.835 (Y/Y/Y) |
| 4 | - | - | 6.874 [6.826-7.087] | 3.581 [3.519-3.663] | - | - | 1.919 (Y/Y/Y) |
| 8 | **3.699** [3.688-3.771] | 5.491 [5.327-5.549] | 5.023 [5.001-5.081] | 2.569 [2.501-3.124] | **1.440** (+44.0 %; n/Y/Y) | 2.137 (Y/Y/Y) | 1.955 (Y/Y/Y) |
| 16 | - | - | 4.926 [4.869-5.173] | 2.388 [2.337-2.465] | - | - | 2.062 (Y/Y/Y) |

- **The headline against Jolt v5.6.0 (the number the owner asked for; a projection until C4 ships the
  default).** `T-D-tree` at W=8 **3.699 ms = 1.440x** (+44.0 %), claimed under IQR and SE, not under min-max -
  the "n" is Jolt's own 3.124 ms window-3 outlier (without it 1.453x, claimed under all three); at W=1 **0.918x**
  (-8.2 %, IQR and SE). Per manifold per step over [100,500), each side's own count (boyko 4,519.3, Jolt 8,489.0):
  1.72x at W=1, 2.70x at W=8 - the raw W=1 win is the 1.88x smaller contact set, as window 3 said of `D-L5`.
  Scaling T(1)/T(8): T-D-tree 2.440 (D-L5 2.008; Jolt 3.825). Against window 3's `D-L5` the default row with the
  Tree reads -15.9 % (W=1) and -30.8 % (W=8), Y/Y/Y.
- **The Tree against AllPairs, same binary.** cfg-A: -8.60 / -13.71 / -19.42 / **-24.37** / -24.06 % at
  W = 1 / 2 / 4 / 8 / 16, every one Y/Y/Y (delta 1.56-1.68 ms at every W; the design's table 1.72-1.94). Default:
  **-14.47 % (W=1), -32.63 % (W=8)**, Y/Y/Y. R: -11.49 % (W=1, Y/Y/Y), -22.46 % (W=8, n/Y/Y: one 7.440 ms allpairs
  process with a 6.49 % during-process witness and clean receipts). S16: -0.38 %, n/n/n - no claimed regression.
  Churn: `*_tree` faster than `*_allpairs` on all 12 arms under IQR and SE (-16.3..-19.6 % sleeping off,
  -28.2..-30.2 % on), 11 of 12 under min-max; `tree(arm) - tree(stable)` <= 0.05 ms by value on 1 of 10 arms,
  unclaimed under min-max on all 10, claimed under SE on 5 (+0.077..+0.268 ms) - the AllPairs arm pays the same
  churn (+0.06..+0.41 ms), and 0.05 ms is below K=6's SE bars.
- **The bridge (`T-D-allpairs` against window 3's `D-L5`, 10.738 / 5.347 ms):** -1.72 % (W=1; n/n/Y) and
  +2.68 % (W=8; n/n/n) - no claimed shift under range or IQR; the W=1 SE-only shift is the same magnitude and
  reading window 3 found on its own W=1 cells, across two binaries a whole commit apart, and is not attributed.
- **The armed rows (cfg-A, `--arm-profiler`; median over [100,500) per process, then over K).** The tree span
  (verify + build + query + assemble) **0.4756 ms at W=1, 0.4662 at W=8**, of which the query is 0.4140 / 0.4110
  (87-88 %; 334 ns per queried row, 43 ns per emitted pair); verify 7.8 / 6.6 us, build 26.7 / 22.8, assemble
  26.4 / 26.1. `sys_physics_broadphase` AllPairs 2.029 / 1.954 ms (P0b's 2.100 / 1.945 within -3.4 / +0.5 %).
  **Delta-bp(8) = 1.488 ms** (median reading; 1.484 by the [0,500) mean; W=1 1.553 / 1.517). Arming cost
  -0.10 % (n/n/n). Structure: `span_n_sets` (1,1,1,1), queried + members = 1241 on every step, one rebuild per
  process, void 0, `first_void` null on all 12 tree-armed processes.
- **The canary (`T-C-tree`):** span within -1.4..+0.6 % of the injected 5 % of T; the step rises +4.89 % (W=1,
  0.98x the canary, Y/Y/Y) and +6.30 % (W=8, 1.26x, n/Y/Y: one 6.039 ms process) - seen at both W, as P0 read it.
- **G4 pair finding (K=3; tree / all_pairs, claimed under range and SE).** Uniform: 9.73 (17), 1.62 (64),
  **0.879 (128)**, 0.501, 0.241, 0.040, 0.003 (100k); disparity 8.01, 2.00, **1.057 (128)**, 0.648, 0.352, 0.047,
  0.003; scene 0.239 at 1240 and j100 (**`tree/j100` 0.4429 ms**, `tree/1240` 0.4415), 0.039 at 10k (4.807 ms),
  0.003 at 100k (59.34 ms). Tree faster than `grid_w1` at every n > 64 in every family (0.08-0.44); `grid_w8`
  beats the serial tree by 9 % at uniform 100k only. Per row of the tree's step: uniform 153 / 302 / 398 ns at
  1k / 10k / 100k, scene 356 / 481 / 593 ns at 1240 / 10k / 100k - the design's c_q (100-150 / 150-200 / 250
  ns per row) is met by the uniform family within 1.0-1.6x and exceeded by the scene family 2.4-3.6x.
- **G4 maintenance (K=3, ms at m = 1240 / 10k / 100k):** stable 0.0327 / 0.278 / 4.37; admission_from_empty
  0.473 / 4.84 / 62.5; admission_of_64 0.0692 / 0.530 / 9.73; eviction_filter 0.0462 / 0.389 / 6.08;
  shift_translation 0.0554 / 0.468 / 9.20; compaction 0.207 / 2.36 / 30.4; high_jumper 0.0599 / 0.505 / 9.61.

**The stop rules (recipe 1.2, the design's D3.5 upper values) and the G5 gates (recipe 2.3).**

| gate | measured | limit | verdict |
|---|---|---|---|
| G4 rule 1, J snapshot `bp_g4_scene/tree/j100` | **0.4429 ms** (0.4424 / 0.4429 / 0.4440) | 0.30 | **FIRED** (1.48x) |
| G4 rule 2, tree vs `grid_w1` at n > 64 | tree faster everywhere, Y/Y | tree slower, claimed | held |
| G4 rules 3 / 4 / 5 / 7 / 8 (stable, translation, eviction, admission of 64, from empty; minus `stable`) | 0.56-0.61 / 0.37-0.38 / 0.28-0.29 / 0.30-0.31 / 0.95-0.96 of the limit at 1240 / 10k; 0.95 / 0.97 / 0.45 / 0.55 / 0.97 at 100k | - | held (3, 4, 8 at the limit at 100k) |
| G4 rule 6, compaction (`compaction - eviction_filter`) | **0.1607 / 1.966 / 24.27 ms** | 0.050 / 0.40 / 6.0 | **FIRED x3** (3.2 / 4.9 / 4.0x) - but the arm's timed step re-queries the h = m/2 evicted rows (a bench-construction defect, below) |
| G4 rule 9, high jumper minus stable | 0.0272 / 0.2269 / **5.239** | 0.062 / 0.50 / 5.0 | **FIRED at 100k** (1.05x; the list pass over 868k entries at 5.6 ns each) |
| G5 headline Delta: `T-A-tree` vs `T-A-allpairs` at W=8 claimed faster; Delta-bp(8) >= 1.03 ms | -24.37 % Y/Y/Y; **1.488 ms** | 1.03 | **PASS** (1.44x the bar; 0.83-0.87x the design's 1.72-1.80) |
| G5 every W, the default row, R, S16, structure, canary, pose | above | - | **PASS** (R W=8 and the canary W=8 not under min-max) |
| G5 the tree span (sigma of the four `phys_bp_*` spans, median over [100,500)) | **0.4756 / 0.4662 ms** | <= 0.36 / 0.35 | **FAIL - stop** (1.32x / 1.33x) |

**The C4 verdict: STOP-RULE FIRED - the flip of the default is DEFERRED by the recipe's letter.** Every
directional gate passes (the Tree is claimed faster than AllPairs on every row at every W under IQR and SE,
S16 shows no regression, one pose per scene, structure clean, canary seen), so the investigation can change the
flip's size, not its sign - unless it finds the excess is work the design forbids. **One quantity is behind
every fired rule:** the J-scale query costs 334 ns per row (43 ns per emitted pair) where the design's
arithmetic has 100-150 ns per row; verify, build, assemble, c_list (1.33-1.97 ns per entry) and the radix-only
c_build (bounded <= 11.6 / 22.9 / 53.3 ns per row from `admission_of_64`) are at or near their bands. By
arithmetic it explains rule 1 (0.414 of query + 0.03 of the rest), the tree-span FAIL, rule 8 at 95-97 % of its
limit (m x c_q inside the admission), rule 6 (h x c_q inside the compaction arm's timed step), the Delta-bp
shortfall against the design (the 0.24-0.32 ms the span is over), the `ADMIT_BUILD_RATIO` formula reading >= 1
and the D6 trigger firing. Rule 9 at 100k is separate and marginal. What the investigation must measure
(`analysis.md` section 8): per-query counts (8-wide node tests, leaf candidates, exact tests, pairs) on
`bp_g4_scene/tree/j100` against uniform / disparity 1000; the four spans on the compaction arm's timed step
(or the arm rebuilt so the half is admitted back untimed); a radix-only c_build arm; the 100k translation list
pass; then re-run j100 + the armed rows (K=6) and the two size families with sizes added between 64 and 256.

**The C4 constants (recipe 1.4).**
- `TREE_BRUTE_MAX_ROWS` = **64** (derived, unchanged from C1's provisional 64): the largest n at which
  `all_pairs` is not claimed slower than `tree`, the smaller of the two families - uniform 64 (at 128
  all_pairs / tree = 1.137, claimed), disparity 128 (0.946, not slower); log-log crossovers 111 uniform, 138
  disparity, both between grid points, so the recipe's own refinement run (sizes between 64 and 256, K=3) is
  owed before the value is final. 64 is the conservative side.
- `AUTO_TREE_LO / AUTO_TREE_HI` = **64 / 256** by the recipe's grid rule (HI = the smallest n at which `tree` is
  claimed faster: uniform 128, disparity 256; LO = the largest n at which `all_pairs` is not claimed slower),
  **126 / 140** by L2's actual procedure for `GRID_LO/HI` (the larger family's log-log crossover to two figures
  and a 10 % dead band). Neither band is exercised by any timed row (the default ships `Manual`). Commit only
  after the refinement run.
- `ADMIT_BUILD_RATIO`: **DEFERRED.** The recipe's formula reads 1.40 -> (7, 5) at 10k (1.10 / 1.79 at 1240 /
  100k) against the design's 0.14-0.24, and it is >= 1 by construction: its `c_build` term is
  `(admission_from_empty - stable) / m`, which by D3.5's own definition of admission contains c_q - the
  denominator. With the radix-only c_build bounded from `admission_of_64`, the ratio is <= 0.067 / 0.102 /
  0.211 at 1240 / 10k / 100k with the measured c_q (0.18 / 0.19 / 0.28 with the design's) - (1, 8) or below at
  10k against today's (1, 4) - but only because the measured c_q is 2.2-3.3x the design's. The denominator is
  the quantity under investigation; the recipe's formula needs the correction before the constant is read.

**The C2 decision (D6): DEFERRED.** t_q(J, W=1) = **0.4140 ms** [0.4135-0.4175] (0.411 at W=8); the 10k bound
from the bench 4.807 ms (scene) / 3.022 (uniform). D6 builds C2 iff t_q x (1 - 1/(8E)) - omega(8) >= 5 % of
T(8) on a gated row (omega(8) = 6.54 us from section 10; E unmeasured for a query wave, the narrowphase's 0.681
the only in-tree proxy). At the measured t_q the trigger fires on both gated rows for any E >= 0.23 (default
row: saving 0.331 ms = 8.9 % of 3.699 at E = 0.681) / 0.32 (cfg-A: 6.6 % of 5.023); at the design's t_q
(0.124-0.186 ms) the saving is 1.8-4.0 %, below 5 % - the design's own "deferred". Rule: **build C2 iff the
post-investigation t_q(J, W=1) >= 0.235 ms on the default row (0.316 on cfg-A) at E = 0.68.** D6's second
trigger (the 10k bound) needs an end-to-end row of >= 8k bodies that does not exist.

**Two defects in the recipe's arithmetic, found in the raw files, fixable before the re-run:** (a) the
`compaction` arm's timed step re-queries the h = m/2 evicted rows (Q rows on that step, D3.2), which
`- eviction_filter` does not remove; removing (h - 1) queries at the measured per-row cost leaves -0.046 /
+0.455 / +4.35 ms against the design's compaction band 0.015-0.025 / 0.12-0.20 / 2-3 ms - inside it at 1240,
2-4x above at 10k, 1.4-2.2x at 100k with the query estimate's own +-10 % covering most of the 100k excess; so
rule 6 fired on the formula and the compaction itself is not separable in this bench; (b) the `c_build`
definition above.

**Not claimed / not measured.** No W scaling of the Tree (serial by design; the tree rows' better T(1)/T(8) is
the removal of a serial 1.95 ms span); nothing downstream of `ContactPairs` (identical pair set, 9,559 final
pairs, one pose; the 0.13-0.19 ms by which the step deltas exceed the span deltas is inside the step bars); the
C5 sleeping floor (no `--sleeping` row); Jolt parity as a shipped number (a projection until C4 ships the
default); the design's Delta of 1.72-1.80 ms at W=8 (1.488 measured; the 1.03 bar is); the default cfg's own
tree span (the armed rows are cfg-A); under min-max: T-D-tree / Jolt at W = 1 and 8 (Jolt's two window-3
outliers), R at W=8, the canary at W=8, `burst_migrate/off`, and the bridge's W=1 SE-only shift. Open
(`analysis.md` section 9): **one W=1 determinism event in the shipped default, not in the tree** -
`raw/pass-01/075_r2_T-D-allpairs_W1` (default cfg, AllPairs, `pool_workers 1, dispatcher 1`, `parallel_solve`
/ `parallel_narrowphase` true) hashed `0xf6e397d168e0e8b8`, identical to its twins through step 438 and
diverging at step 439 in `top_y` with the pair count never differing; 1 of 13 default-cfg J W=1 processes, 0 of
30 cfg-A W=1 (flags off at W=1), 0 of 12 default W=8, 0 of 24 R; the cheapest next measurement is untimed
K >= 30 `T-D-allpairs@W1` / `T-D-tree@W1` with `--expect-pose`, then with the flags off; the default C4 would
ship carries this path with either broadphase. The claim-rule reading (range / IQR / SE) is still stated two
ways in the tree; every verdict above is printed under all three. The G4 grid is too coarse for the two
crossovers; the refinement run is owed before `AUTO_TREE_LO/HI` is committed.

Receipts: `docs/measurements/2026-09-22-broadphase-tree/`: `README.md` (the protocol block, the idle receipts,
the binaries, what was reused from window 3), `analysis.md` (this reduction, the analyst's), `window_report.md`
(the tester's), `rows.json`, `wait_log.txt`, `bin/SHA256SUMS` + `COMMIT.txt`, `logs/`, `gate/`, `raw/runs.jsonl`,
`raw/pass-0{0,1}/`, `raw/g4/runs.jsonl`, `raw/g4/<group>/<arm>/<param>/k*/estimates.json`, `raw/g4/logs/`,
`raw/g4/wait_log.txt`, `raw/wait_log.txt`, `raw/window_log.txt`, `raw/window_state.json`, `analyst/`, `tools/`
(`analyze_win4.py` regenerates `analyst/reduction.json` byte for byte from `raw/`). The Jolt v5.6.0 and `D-L5`
cells are window 3's `raw/runs.jsonl` and `raw/receipts/`, never re-run. Not in the tree: the exes, the untimed
rehearsals, criterion's `sample.json` / `benchmark.json` / `tukey.json` and its duplicate `new/` baselines.

---

## 12. Physics — the solve setup per contact (L11 C1+C2 against C0), the design's G9 — TIMED 2026-09-22 (window 4b)

**RESULT, 2026-09-22, window 4b (L11 C1+C2 against C0), G9 of `levers/L11-solve-setup/02-DESIGN-REV1.md`.**
One window, complete, under the window-3 protocol block (the 2026-09-19 ruling: median over K separate
processes of the window mean; spread = min-max, IQR, the median's SE; claimed iff |effect| >
2*hypot(spread_A, spread_B), printed under all three readings, G9's SE form the gate; 5-s receipts before and
after every process, > 5 % or a build process => re-run once at the end of the pass; 10-s receipt and three
quiet 60-s polls before each block; no band void; P-none; every process ran with `--expect-pose`).
**K = 12** (two passes x six rounds; G9's own K). **Jolt v5.6.0 is the only reference (owner ruling
2026-09-21) and it was not re-run: its window-3 cells are used** (`918fd2b7`, K=6: 9.828 / 5.770 / 3.581 /
2.569 / 2.388 ms at W = 1 / 2 / 4 / 8 / 16; window 3 is `docs/measurements/2026-09-21-physics-window3/` at
`2ce03b66` on `merge/ke16-into-ecsnative`, not on this lane). Owner's workstation (Ryzen 9 5900HS, 8C/16T,
High performance, AC; the owner's agent sessions, Telegram and a browser open - see contamination). Timed
08:20:42-10:12:19 +03:00, after the window-4 runner had finished. rustc 1.98.1 `host: x86_64-pc-windows-msvc`,
no RUSTFLAGS, `CARGO_INCREMENTAL=0`, cargo `parity` (inherits `release`: fat LTO, default CGU), zone tier
`dev`. Binaries: parent = C0 `146a1125` (`git archive` into a scratch tree, cold build into
`D:/wt/_targets/l11-parent-msvc`; the test-only setup digest + the `J-As` runner row, **no solver change**)
`runner_parent` sha256 `8dfd0143`; tip = C2 `f8873aae` (= C0 + C1 `691891c4` + C2, the lane's HEAD,
`D:/wt/lighttable` clean, built into `D:/wt/_targets/vkval-msvc`) `runner_tip` `29dbd993`; each build's
`Compiling boyko-physics (path)` line names its tree (`bin/COMMIT.txt`). Ancestry: window 3's `de06b6c9`
(L5 C4) is an ancestor of the parent; between them the only `boyko_physics` source commits are A1b
`8af0e3b9` and C0 itself. Poses per row bit-identical on both binaries in all 410 used processes (J rows
`0x32d5e235342b4143`, R `0x87e561d20589d4a5`, R-S `0x2a2b7926a48aab00`, S16 `0x8877dbb1192e9b92`, the C0
tester's values; `expect_pose: "match"` 410/410, against the parent's untimed gate pose, with a 501-step red
control that exits 4); counters identical (J rows 4,524.246 manifolds per step over [0,500), 109.32 waves;
R 6,662.3; R-S 6,675 all frozen; S16 16); void 0, drops 0, disarmed ring traffic 0, workers = W, mask
`0xffff`.

**Rows.** `J-As` (`--scene jolt --gap 0.5 --cfg as`: colored, `simd_solve` on, `parallel_solve` /
`parallel_broadphase` / `parallel_narrowphase` = W>1, AllPairs, sleeping off) and `J-A` (`--cfg a`: the same
with `simd_solve` off) at W 1/2/4/8/16, 500 steps, disarmed; `R` (`--scene rest --solver colored`, the
default config; `--parallel-solve` at W=8 is a no-op since L4) at W 1/8, 1100 steps, window [600,1100),
armed; `R-S` (`--scene rest --sleeping --frozen-by 300`) at W 1/8, 800 steps, window [300,800), armed; `S16`
(`--scene s16 --cfg a`) at W 1, 300 steps, armed; the armed `J-As-a` (`--cfg as --arm-profiler`) at W 1/8;
the canary `J-C` (`J-As-a` + `--canary-frac 0.05`) once per binary at W 1. Jolt not run. 34 cells,
parent/tip interleaved inside each W group, pass 1 reversed, one untimed warm-up per pass; 410 slots, 42
re-runs, **K = 12 in every cell**, 0 dropped, 0 non-zero exits.

**Contamination.** 499 5-s receipts: median 1.44 % busy, p90 4.81 %, max 12.35 %, **42 over 5 %** (37 of
them in main pass 1, 08:55-09:57: the owner's sessions, Telegram, a browser - never a build process); every
hot original has a clean re-run. Main pass 0 (08:20-08:52), the armed block (09:59-10:09) and the canary
(10:12) were quiet. The parent's W >= 2 cells sit 0.4-1.4 ms higher in pass 1 than in pass 0 (the tip's
0.1-0.4 ms), and the pooled min-max ranges carry that: where a claim below holds under IQR and SE but not
min-max, that is the reason. Pass 0 alone (K = 6 per side) claims every `J-As` cell under all three
readings. In every `J-As` cell the two binaries' min-max ranges are disjoint (the parent's fastest process
is slower than the tip's slowest).

| W | J-As parent | J-As tip | Delta ms (effect; claimed r/i/s) | J-A parent -> tip | Jolt v5.6.0 (window 3) | J-As tip / v5.6.0 |
|---|---|---|---|---|---|---|
| 1 | 10.693 [10.438-11.620] | **8.697** [8.395-9.308] | **-1.996 (-18.7 %; n/Y/Y; ranges disjoint)** | 19.582 -> 18.932 (-3.3 %, n/Y/Y) | 9.828 [9.518-11.364] | **0.885** |
| 2 | 7.659 [7.624-9.178] | 6.221 [6.169-7.062] | -1.438 (-18.8 %; n/n/Y; disjoint) | 12.325 -> 11.465 (-7.0 %, SE) | 5.770 | 1.078 |
| 4 | 6.176 [6.042-7.646] | 4.923 [4.890-5.334] | -1.253 (-20.3 %; n/Y/Y; disjoint) | 8.529 -> 7.769 (-8.9 %, SE) | 3.581 | 1.374 |
| 8 | 5.342 [5.215-6.903] | **4.240** [4.204-5.004] | **-1.103 (-20.6 %; n/n/Y; disjoint)** | 6.592 -> 5.910 (-10.4 %, SE) | 2.569 [2.501-3.124] | **1.650** |
| 16 | 5.469 [5.344-6.737] | 4.357 [4.329-5.201] | -1.112 (-20.3 %; n/n/Y; disjoint) | 6.663 -> 5.660 (-15.1 %, SE) | 2.388 | 1.824 |

`R`: 14.300 -> 11.161 ms at W=1 (-3.139, -22.0 %, n/Y/Y) and 6.781 -> 5.246 at W=8 (-1.535, -22.6 %, SE) -
the lever's largest absolute gain, on 6,662 manifolds per step. `R-S`: -1.5 % / -3.7 %, not claimed either
way. `S16`: -1.2 %, not claimed. Armed `J-As-a`: 10.658 -> 8.554 (W=1, -2.105 ms, -19.8 %) and 5.275 -> 4.242
(W=8, -1.033, -19.6 %), claimed under all three readings (SE 0.2-0.8 %); armed against disarmed is not
claimed on either binary (-1.65...+0.05 %), so the armed row is a fair proxy for T. Scaling: `J-As`
T(1)/T(8) = 2.002 (parent) and 2.051 (tip); W=16 against W=8 +2.4 % / +2.8 %, not claimed - neither binary
scales beyond eight workers.

- **G9 gates (the design's bars = 0.6 x the lower predicted Delta, as the design lists them): T(1) `J-As`
  -1.996 ms <= -0.81 PASS (2.5x the bar; the worst process pairing, tip-slowest against parent-fastest, is
  still -1.130); T(8) -1.103 <= -0.61 PASS (1.8x); solve_build -0.573 / -0.542 ms (W=1 / 8) <= -0.29 PASS
  (2x; worst pairing -0.550); store -0.141 / -0.134 <= -0.11 PASS (worst pairing -0.131 / -0.118); wide
  colours "not claimed slower" holds - faster, -27.7 % / -28.5 % at W=1 / 8, claimed above SE (the design's
  -10...-25 % is exceeded); cfg-A (`J-A`) not claimed slower at any W - faster at all five, claimed under SE
  (W=1 under IQR too). warm_apply's bar (-0.19 ms) is C3's (the 8-lane-wide apply) and C3 is not in this
  tip; C1's merge-join already reads -0.092 ms (0.611 -> 0.519 at W=1), reported, not gated.** The
  window-report's arithmetic gated against bars discounted twice (0.486 / 0.366 / 0.174 / 0.066); every gate
  passes under both; the design's are binding and are the ones quoted here.
- **Per stage, `J-As-a`, us per manifold on the design's 4,524.2 (the same denominator before and after -
  the lever is bit-identical), parent -> tip at W=1 / W=8 (reading A: per process the window mean, then the
  median over K):** solve_build 0.191 -> 0.064 / 0.189 -> 0.069 (0.864 -> 0.291 / 0.856 -> 0.314 ms, -66 %);
  warm_apply 0.135 -> 0.115 / 0.137 -> 0.118 (0.611 -> 0.519 / 0.621 -> 0.532 ms, -15 %); store 0.036 ->
  0.005 / 0.035 -> 0.006 (0.163 -> 0.022 / 0.160 -> 0.026 ms, -87 %); **setup total 0.363 -> 0.184 / 0.362 ->
  0.193 (1.643 -> 0.832 / 1.638 -> 0.871 ms, -49 %; serial at both W)**; wide colours 0.770 -> 0.557 /
  0.193 -> 0.138 (3.484 -> 2.520 / 0.875 -> 0.625 ms); the solve span 1.157 -> 0.762 / 0.583 -> 0.357
  (5.235 -> 3.445 / 2.635 -> 1.615 ms, -34 % / -39 %). Broadphase and narrowphase moved < 0.08 ms (not lever
  effects). Where the -2.1 ms at W=1 comes from: setup -0.811, wide colours -0.964, executor gap -0.156,
  broadphase -0.079, narrowphase -0.040, the rest -0.02; sum -2.07 against the T delta -2.105. Design
  targets: solve_build 0.25-0.55 ms (measured 0.291), store 0.04-0.08 (0.022, below the band), wide colours
  2.68-3.22 (2.520, below), T(1) 8.67-9.79 (8.697), T(8) 3.93-4.69 (4.240); the setup band 0.44-0.93 holds
  the measured 0.832 only with C3's warm_apply outstanding. The parent's baselines (solve_build 0.864,
  warm 0.611, store 0.163, setup 1.643) sit below the design's P0 column (1.038 / 0.614 / 0.270 / 1.922,
  taken from J-B on the P0 tip); the realized deltas are against the actual parent, as G9 says.
- **The bridge to window 3 holds:** the parent's `J-A`@W1 against window 3's `J-A`@W1 (equal knobs, both
  trees) 19.582 against 19.483, +0.5 % (n/n/n); its `J-As`@W1 against window 3's shipped default
  `D-L5`@W1 (the same code path at one worker) 10.693 against 10.738, -0.4 % (n/n/n); each median inside the
  other's min-max. `J-A`@W8 sits -2.58 ms under window 3's, which is L5's measured gain (window 3: -2.435 ms
  for C4 against C2). The W=8 `J-As`/`D-L5` pairing (-0.1 %) is knob-mismatched (`parallel_broadphase` on in
  `as`, off in the default) and is a caveat, not a bridge. Cross-window uncertainty for any mixed claim:
  about 0.5 % at W=1.
- **Against Jolt v5.6.0 (window 3's cells, the only reference): at W=1 the tip's `J-As`, 8.697 ms, is
  0.885x its 9.828 (-11.5 %; claimed under IQR and SE, not min-max - the Jolt W=1 cell holds one 11.364 ms
  process); per manifold per step over [100,500) with each side's own count (boyko 4,519.3; v5.6.0 8,489.0)
  1.69x (1.938 against 1.1445 us) - inside the design's predicted 1.68-1.90 at its good edge. At W=8 1.650x
  its 2.569 (+65 %, claimed under all three); per manifold 3.07x; inside the design's predicted 1.58-1.88 /
  2.95-3.52. The parent was 1.088x and 2.079x; window 3's shipped default `D-L5` was 1.093x and 2.081x.**
  At W = 2 / 4 / 16: 1.078x (SE only) / 1.374x / 1.824x. cfg-A (`J-A`) stays 1.9-2.4x at every W; it is
  not the shipped configuration. The honest W=1 statement, as in window 3: 0.89x on the step, 1.69x per
  manifold, the truth between (boyko carries 1.88x fewer manifolds). The remaining W=8 gap is the serial
  0.87 ms setup, the 1.92 ms AllPairs broadphase and the 0.52 ms narrowphase, none of which this lever
  addresses.
- **The canary (`J-C`, K = 1 per binary):** the span reads the injected spin on both binaries (parent 543,367
  ns against 543,045 injected, +0.06 %; tip 428,595 against 428,103, +0.12 %); the wall rise is seen on the
  parent (+0.494 ms for a 0.543 ms spin, outside the `J-As-a` cell's min-max) and lies inside the spread on
  the tip (+0.198 ms for 0.428, inside 8.384-8.960) - not resolvable at K = 1.
- **Facts for the design, not claims:** `R-S` (all frozen) holds the window's only tip-slower stages -
  solve_build +16 us per step (0.034 -> 0.050 ms, +48 %, IQR and SE) and narrow colours +1.5 us (+68 %) on
  6,675 frozen manifolds; its tip store, 0.127-0.129 ms, is above the design's predicted 0.05-0.10 for the
  B1 carry (the parent's 0.316 / 0.259 sit at the design's "today"); T is not claimed either way. The
  parent's `R`-row setup spans are bimodal by process (solve_build 1.19-1.43 in eight, 1.87-2.12 in four;
  store 0.24-0.31 / 0.51-0.68) where the tip's are tight - P0's "a process draws its own speed" on the
  per-point table and hash probe the lever deleted, and why the `R` setup deltas are claimed under SE only.

**Not claimed / not measured.** Per-contact parity with v5.6.0 (1.69x / 3.07x per manifold; the design said
a zero setup still leaves 1.77x, the remainder is collision detection - L5, the tree broadphase, L9); a Jolt
per-stage setup comparison (Jolt not run; its setup sits inside FindCollisions); setup scaling at W=8 (the
setup is serial on both binaries, 0.83-0.87 ms on the tip at both W; C4's in-binary A/B not run); **C3's
warm_apply gate** (-0.19 ms; C3 is not in this tip); memory receipts (the design's ~2 MB committed-page
shrink); the pooled min-max reading for the J and R rows (pass-1 contamination; pass 0 alone claims them);
`parallel_broadphase` isolated at W=8; W=16 against W=8 on either binary; the broadphase / narrowphase
deltas as lever effects; the wall rise of the canary on the tip at K = 1. Only values, poses, counters and
spans are claimed; every pose and counter is identical across the two binaries.

Receipts: `docs/measurements/2026-09-22-l11-solve-setup/`: `README.md` (the protocol block and the binaries
table), `analysis.md` (this reduction, verbatim), `window_report.md`, `rows.json`, `bin/SHA256SUMS`,
`bin/COMMIT.txt` (no exe committed), `logs/`, `raw/` (`runs.jsonl`, `manifest.json`, `window_state.json`,
`window_log.txt`, `wait_log.txt`, `main-p{0,1}/`, `armed-p{0,1}/`, `canary-p0/`, `analysis.json`,
`tables.md`, `sensitivity.md`), `gate/` (the pose files and the red control), `tools/`, `analyst/`
(`reduction.json`, `tables_analyst.md`). `dry/` and `test/` were rehearsals and are not in the tree, nor are
the exes and `__pycache__/`.

## 13. Physics — the combined tip (tree broadphase + L11 C3) against Jolt v5.6.0, and L11 C3's warm_apply gate (G9) — TIMED 2026-09-23 (window 5)

**RESULT, 2026-09-23, window 5: the headline on tip `cbd86a65` (L11 C0-C3 + the integration line's tree
broadphase C1+C3, A1b, A3, hwrt) and G9 on C3 against its parent `0ca312bd` (the same lane one commit earlier:
C0-C2 + the line, no C3).** One window, complete, under the window-3 protocol block (the 2026-09-19 ruling:
median over K separate processes of the window mean; spread = min-max, IQR, the median's SE; claimed iff
|effect| > 2*hypot(spread_A, spread_B), printed under all three readings r / i / s - this window requires BOTH
min-max and SE, G9's own form is SE; 5-s receipts before and after every process, > 5 % => re-run once at the
end of the pass; 10-s receipt and three quiet 60-s polls before each pass; a build or lane process at any point
of a timed pass voids the pass; no band void; P-none; every process ran with `--expect-pose`). **K = 6** (two
passes x three rounds per block; tree/AllPairs and parent/tip alternating inside each W group, pass 1
reversed). **Jolt v5.6.0 is the only reference (owner ruling 2026-09-21) and it was not re-run: its window-3
cells are used** (`918fd2b7`, K=6: 9.828 / 5.770 / 3.581 / 2.569 / 2.388 ms at W = 1 / 2 / 4 / 8 / 16;
window 3 is `docs/measurements/2026-09-21-physics-window3/`, on this lane since `0ca312bd`). Owner's
workstation (Ryzen 9 5900HS, 8C/16T, High performance, AC). Timed 00:40:17-03:12:50 +03:00; the blocks that
count: headline 02:16-02:48, g9 02:51-03:04, armed 03:07-03:12. rustc 1.98.1 `x86_64-pc-windows-msvc`, no
RUSTFLAGS, `CARGO_INCREMENTAL=0`, cargo `parity` (inherits `release`: fat LTO, default CGU), zone tier `dev`.
Binaries: parent = `0ca312bd` (`git archive` into a scratch tree, cold build into
`D:/wt/_targets/l11-parent-msvc`) `runner_parent` sha256 `3993684b`; tip = `cbd86a65` (the lane's HEAD,
`D:/wt/lighttable` clean, built into `D:/wt/_targets/vkval-msvc`) `runner_tip` `9c7caff1`; each build's
`Compiling boyko-physics (path)` line names its tree (`README.md`, `logs/build_*.log`). `cbd86a65^` =
`0ca312bd`, and C3 touches only `crates/boyko_physics` (`solver/colored.rs` and two test files). Poses
bit-identical on every binary and arm in all 196 processes that produced a result (J `0x32d5e235342b4143` on
`--cfg default` with either broadphase and on `--cfg as`; rest `0xee2a67a98434919a`; `expect_pose: "match"`
against the parent's untimed gate pose; six 501-step red controls, three per binary, exit 4 - the gate can
fail); counters identical on both binaries; TreeDiag `static_rebuilds 1, members 1`, all else 0, on every tree
process and all zero on AllPairs; void 0, workers = W, mask `0xffff`, printed config field-equal parent/tip.

**Rows.** Headline, tip only, disarmed, 500 steps: `HL-D-tree` / `HL-D-allpairs` (`--scene jolt --gap 0.5 --cfg
default --broadphase tree|allpairs`, window 4's `T-D-*`) at W 1/2/4/8; `HL-R-tree` / `HL-R-allpairs` (`--scene
rest --cfg default`) at W 1/8. G9: `J-As` (`--cfg as`: colored, `simd_solve` on, the three parallel flags =
W>1, AllPairs, sleeping off) parent/tip at W 1/8, disarmed; the armed twins `J-As-a` (`--arm-profiler`) at
W 1/8. 16 cells, 120 slots, 25 re-runs, 117 slots used; 3 dropped because the original and its re-run were
both hot, so `HL-D-tree` W1, `HL-D-allpairs` W2 and tip `J-As` W8 are K = 5 (a make-up outside the protocol
set ran each once more); 0 non-zero exits.

**Contamination.** Three attempts of the headline's pass 0 were VOIDED by build or lane processes (00:52 two
`rustc`, 00:57 `cargo` appearing, 01:07 `cargo` and another lane's test binary under
`D:/wt/_targets/joltab-msvc`); the idle rule then waited 69 polls (01:07-02:15). None of their 42 timed
processes is in a cell. Protocol set, 145 processes: before-receipts median 2.70 %, max 4.97 %, 0 over 5 %;
after-receipts median 2.95 %, p90 6.71 %, max 13.20 %, 28 over 5 % (25 hot originals, 3 hot re-runs). The
during-process witness over the used set: median 1.48 %, p95 4.55 %, max 7.20 % - about 4x window 4b's
0.33 %.

| W | tree (`--cfg default --broadphase tree`) | AllPairs (shipped default) | Jolt v5.6.0 (window 3) | tree / v5.6.0 (effect; claimed r/i/s) | AllPairs / v5.6.0 | tree / AllPairs |
|---|---|---|---|---|---|---|
| 1 | **7.106** [6.891-7.286] K=5 | 8.782 [8.528-9.136] | 9.828 [9.518-11.364] | **0.723 (-27.7 %; n/Y/Y)** | 0.894 (n/n/Y) | 0.809 (-19.1 %; Y/Y/Y) |
| 2 | 4.659 [4.469-5.559] | 6.435 [6.244-6.557] K=5 | 5.770 [5.703-5.818] | 0.807 (-19.3 %; n/Y/Y) | 1.115 (Y/Y/Y) | 0.724 (-27.6 %; n/Y/Y) |
| 4 | 3.315 [3.297-3.398] | 4.976 [4.867-5.561] | 3.581 [3.519-3.663] | 0.926 (-7.4 %; n/Y/Y) | 1.389 (Y/Y/Y) | 0.666 (-33.4 %; Y/Y/Y) |
| 8 | **2.716** [2.520-2.963] | 4.296 [4.213-4.539] | 2.569 [2.501-3.124] | **1.057 (+5.7 %; n/n/n)** | 1.672 (Y/Y/Y) | 0.632 (-36.8 %; Y/Y/Y) |

Rest (tip, no Jolt reference): tree against AllPairs 9.768 against 11.274 ms at W=1 (-13.4 %, n/Y/Y) and
3.803 against 5.397 at W=8 (-29.5 %, Y/Y/Y).

- **The headline: with the tree broadphase the tip's default config is 0.723x Jolt v5.6.0 at W=1 (7.106
  against 9.828 ms, -2.72 ms; claimed under IQR and SE, not min-max - the min-max failure is Jolt's own
  window-3 cell, which holds one 11.364 ms process; without it 0.726x under all three, a sensitivity reading,
  not the ruled statistic) and 1.057x at W=8 (2.716 against 2.569 ms, +0.147; not claimed under any reading -
  the two cannot be told apart at eight workers in this window).** 0.807x / 0.926x at W = 2 / 4 (IQR and SE).
  The shipped AllPairs default on the same binary is 0.894x (SE) / 1.115x / 1.389x / 1.672x at W 1/2/4/8, and
  the tree beats it on that binary at every W (all three readings at W 1/4/8, IQR and SE at W=2). Per manifold
  per step over [100,500), each side's own count (boyko 4,519.26, v5.6.0 8,489.0): tree 1.38x / 1.53x / 1.72x
  / 1.96x Jolt at W 1/2/4/8 - the per-step lead rests on boyko's 1.88x smaller contact set, and the truth lies
  between the two readings. T(1)/T(8): tree 2.616, AllPairs 2.044, Jolt 3.825. Since window 4 (`T-D-tree` on
  `a46b8287`): 9.026 -> 7.106 ms at W=1 (0.918x -> 0.723x) and 3.699 -> 2.716 at W=8 (1.440x -> 1.057x),
  cross-window, the code difference L11 C1-C3.
- **The tree is still not the shipped default, and C4 (the flip) stays DEFERRED:** every `--cfg default`
  process prints AllPairs under Manual select and the tree rows force the kind; no tree row here is armed, so
  window 4's stop rules (G4 rule 1 `J snapshot` 0.443 > 0.30, the G5 tree span 0.476 / 0.466 > 0.36 / 0.35,
  rules 6 and 9) were not re-taken and stand, as does the query-cost investigation they call for.
- **G9 on C3 - warm_apply PASS.** Armed `J-As-a` at W=1, reading B (per-process median over [100,500), the
  design's and window 4b's convention): parent 0.5285 [0.5265-0.5297] -> tip 0.2960 [0.2949-0.2978] ms, Delta
  -0.2325 ms (-44.0 %) against the -0.19 bar (the design's listed delta, already the 0.6x realized-gain bar,
  not discounted a second time), margin 0.0425, claimed under all three readings, the worst process pairing
  still -0.2287. At W=8 -0.2348 (0.5391 -> 0.3043), claimed; reading A (the window mean) -0.2264 / -0.2310,
  claimed. 0.74x the lower predicted Delta (-0.314); the tip's 0.296 ms is 0.0654 us per manifold, inside the
  design's 0.15-0.30 ms target at its slow edge; against window 4b's C2 value 0.519 (cross-window) -0.223.
  "Wide colours not claimed slower" holds (the kernel -0.077 / +0.0006 ms, n/n/n). Setup (build + warm +
  store) 0.876 -> 0.656 at W=1 and 0.890 -> 0.672 at W=8, inside the design's 0.44-0.93 band; solve_build,
  store, broadphase and narrowphase do not move (C3 does not touch them). Counters identical: 4,519 manifolds,
  9,559 pairs, 11 colours (9 wide), 108 waves, 17,054 points per step.
- **C3 on the step:** `J-As` 8.859 -> 8.627 ms at W=1 (-0.232, -2.6 %) and 4.726 -> 4.510 at W=8 (-0.216,
  -4.6 %), not claimed at K=6 - the step gain is the warm_apply gain, at K=6's SE resolution; the armed wall at
  W=1 is -0.348 ms (SE only). The design sets no separate step gate for C3 (C1+C2 passed T(1)/T(8) in window
  4b). Cumulative L11 C0 -> C3, chained across windows 4b and 5: -2.23 ms at W=1 and -1.32 ms at W=8.
- **Bridges.** (1) The tip's AllPairs default row against window 4's `T-D-allpairs` (10.554 / 5.491) does not
  reproduce literally and cannot - the tip carries L11 C0-C3, A1b and A3. With the lever's measured deltas
  subtracted, the chain predicts 8.326 / 4.172 against the measured 8.782 / 4.296: residuals +5.5 % / +3.0 %,
  inside the chain's bars, so it holds as a chain. (2) This window's parent `J-As` against window 4b's tip
  (the same solver code; the line merge between them): W=1 1.019 (reproduces, n/n/n); **W=8 1.115 (+11.5 %),
  does NOT reproduce under SE or IQR** (the armed twin +6.0 %, holds), with the code-identical kernel +4.1 % at
  W=8 and -0.2 % at W=1 - machine state at W=8 is the likely reading; a W8-only code cost is not ruled out.
  In-window comparisons are unaffected (interleaved; parent and tip moved together). **Every cross-window W=8
  ratio here (tree/Jolt 1.057, AllPairs/Jolt 1.672, window 4's 3.699 -> 2.716) is qualified**: a 6-11 %
  correction leaves tree/Jolt at W=8 unclaimed either way and AllPairs/Jolt claimed.
- **What the remaining W=8 gap is made of** (arithmetic from the tip's armed `J-As-a` spans with window 4's
  tree broadphase span swapped in; the budget reproduces the measured tree row to -2.5 % at W=8 and +0.9 % at
  W=1): 1.392 ms of the row does not shrink with W - the tree broadphase 0.4665 (query 0.411), serial-in-solve
  0.771 (solve_build 0.336, warm_apply 0.304 after C3, integrate 0.080, store 0.031, gravity 0.013),
  build_graph / gather / apply 0.154. At window 4's 334 ns per queried row (design 100-150) the query is
  0.225-0.287 ms over the design at W=8, larger than the whole +0.147 ms gap to Jolt (not measured: at the
  design's query cost the row would read 2.43-2.49 ms, 0.95-0.97x). L11's own C4 (D10, the parallel fill):
  solve_build(8) after C3 is 7.45 % of T(8) on `J-As` tip, above D10's 5 % trigger - reported, not decided.

**Not claimed / not measured.** Tree against Jolt under both spreads at any W (W 1/2/4 under IQR and SE only,
W=8 not at all); the tree as the shipped default (C4 deferred; no G4 arm, tree span or query cost
re-measured, since the tree rows are disarmed); a Jolt per-stage comparison (Jolt not run); the tree's C4/C5, the sleeper set,
churn, R-S and Auto-select (sleeping off everywhere, `sleeper_rebuilds` 0); **C3's other G9 rows** (J-A's cfg-A
"not claimed slower", R, R-S, S16, W 2/4/16, K=12, the canary) - C3 has passed warm_apply and "wide colours not
claimed slower" on `J-As-a`, the rest of its G9 is open; C3's step gain on the disarmed rows; tree against
AllPairs under min-max at W=2 and on rest W=1; the make-up processes. Open: bridge 2 at W=8 can be settled by
interleaving window 4b's `runner_tip` (`29dbd993`) with this window's parent on `J-As` at W=8 (about a minute
timed). L7 is retired after C3 (`levers/00-RULINGS.md`, the L11 block).

Receipts: `docs/measurements/2026-09-23-combined-tree-l11/`: `README.md` (the protocol block, the binaries
table with the `Compiling` lines, what was reused from window 3 and why), `analysis.md` (the analyst's
reduction, verbatim), `rows.json`, `bin/SHA256SUMS`, `bin/COMMIT.txt` (no exe committed), `logs/`, `raw/`
(`runs.jsonl`, `manifest.json`, `window_state.json`, `window_log.txt`, `wait_log.txt`, the counted
`headline-p0-v3/`, `headline-p1/`, `g9-p{0,1}/`, `armed-p{0,1}/`, the voided `headline-p0/`, `-v1/`, `-v2/`,
`makeup/`, `analysis.json`, `tables.md`, `process_receipts.md`; one `pose.bin` per distinct pose, not one per
process), `gate/` (the two reference poses and the red controls), `tools/`, `analyst/` (`reduction.json`,
`tables.txt`). `test/` was a rehearsal and is not in the tree, nor are the exes, `__pycache__/` and the
duplicate pose files.

---

## 14. Physics — L10's pre-C0 refutation, L9's C0 reading and the C4 decision, the tree query after C3b, and the rest of L11's G9 — TIMED 2026-09-24 (window 6)

**RESULT, 2026-09-24, window 6: four levers on one window, each read against its own design's rule.** One
window, complete, under the window-3 protocol block (the 2026-09-19 ruling: median over K separate processes of
the process statistic; spread = min-max, IQR, the median's SE; claimed iff |effect| > 2*hypot(spread_A,
spread_B) under BOTH min-max and SE, IQR printed, r / i / s below; 5-s receipts between processes, > 5 % =>
re-run once at the end of the pass; a 10-s receipt and three quiet 60-s polls before each pass, up to 30 polls; a
build or lane process at any point of a timed pass voids the pass; no band void; P-none; every process ran with
`--expect-pose` against its row's fixture). **K = 6** (two passes x three rounds per block, the arms alternating
inside each W group, pass 1 reversed; twelve blocks in priority order). Owner's workstation (Ryzen 9 5900HS,
8C/16T, High performance), inside the owner's quiet declaration 08:10-13:10 +03:00; timed 09:09:49-12:27:54,
complete, 0 voided passes, every binary re-checked against its sha256 after the window. rustc 1.98.1
`x86_64-pc-windows-msvc`, no RUSTFLAGS, `CARGO_INCREMENTAL=0`, cargo `parity` (the criterion exe: `bench`).
Binaries (sha256 prefixes, `bin/SHA256SUMS`), five built from `git archive` trees under one pruned `Cargo.lock`
(LF sha256 `530cc386…` = the trunk's tracked lock `5f8de754…` minus `boyko-symcensus`, which these trees do not
have), each build's `Compiling boyko-physics (path)` line naming its tree: `4db26681` (the trunk, L9 C0-C3, reuse
switchable) runner `da674e39` and class bench `8f084c48`; `6dd1f916` (C3b's parent, query RowWalk) `228f3514`;
`983480a9` (C3b's tip, LeafList) runner `f93e5fec` and criterion `broadphase` `e601fd46`; three copied
byte-identical: window 5's parent `0ca312bd` `3993684b` and tip `cbd86a65` `9c7caff1`, window 4b's tip `f8873aae`
`29dbd993`. 553 records: 529 processes (24 warm-ups, 444 originals, 61 re-runs) and 24 pass markers; 436 of 444
slots used (53 by their re-run); 8 dropped because both attempts were hot, so those cells are K = 5. Every used
process: exit 0, void 0, `expect_pose: match` with the hash equal to its fixture (nine fixtures, four of them new:
J with reuse `0x30c5438bc6ad9ffa`, L10's Off′ on J `0x3db47fae414b655c`, R with reuse `0xc8bbe34cf6a8afc6`, R-S
Off′ `0xb7f1e9e8f91f75ab`), workers = W, msvc, mask `0xffff`, TreeDiag `static_rebuilds 1, members 1, evictions
0` on tree rows and all zero on AllPairs, drops 0. The untimed pose gate: 126 processes, 0 failing; six 501-step
red controls exit 4. Contamination: 620 receipts, median 0.85 %, p90 5.13 %, max 17.21 %, 69 over 5 % (all after
the process; `claude.exe` on top in 67; no build or lane process at any); the during-process witness over the used
set median 0.26 %, p95 3.87 %, max 5.07 %; the idle rule: 24 waits, 78 polls, never a build or lane process.
**Jolt was not run**: v5.6.0's window-3 cells, recomputed, are the only reference (owner ruling 2026-09-21).

### 14.1 L10 — the pre-C0 refutation (P1) and the armed Off′ spans (P5b)

- **The refutation does NOT fire: L10 continues.** The bar (`levers/L10-sleeping/08-DESIGN-REV2.3.md:221`): stop
  before C0 if the armed Off′ span on J-Son at W=1, K=6, on the lane base reads below 0.584 ms (= 0.6 x 0.59 +
  0.23). `L10-Offp` (`4db26681`: `--scene jolt --gap 0.5 --cfg a --sleeping --broadphase tree --contact-reuse on`,
  1000 steps, armed) reads **1.4670 ms [1.4566-1.4826] over the literal window [264,1000)** and **1.3043 ms
  [1.2935-1.3218] over the all-frozen tail** (the pile freezes at step 274, not 264; the tail is the design's Off′,
  `06-DESIGN-REV2.2.md:253`): +0.883 / +0.720 ms (+151 % / +123 %) above the bar, the smallest process 2.2-2.5x
  the bar, claimed under all three readings. Both readings sit inside the design's 0.82-1.59 band; 0.292 us per
  logical manifold (design 0.18-0.35). The alternating arm `L10-Son` (AllPairs, reuse off) reads 4.6973 ms.
- **Off′ is 3.230 ms under J-Son, and that drop is the Tree (bp -1.722) plus L9b (np -1.677), not L10.** Off′(1)'s
  tail spans: bp 0.2520 (query 0.1949), np 0.6438, graph 0.1160, solve 0.2196, remainder 0.0687 ms. By arithmetic,
  L10's gain ceiling on the J-Son tail is 1.155-1.195 ms at W=1 (C3b ~0.715-0.740, C3a ~0.190-0.205, C3c ~0.247),
  3.3x its realized-gain gate of 0.35 ms, and 0.60-0.70 ms at W=8 (Off′(8) 0.817, tail 0.780) against 0.14 ms.
  C3c's 0.247 exists only with the Tree.
- **P5b, for the R gate:** R-S Off′ 1.594 / 0.966 ms at W = 1 / 8 (bp 0.245, np 0.835, graph 0.188, solve 0.266),
  R-S 5.778 / 3.121 ms, so the R gate is 0.6 x (Off′_RS - 0.25) = **0.806 / 0.429 ms**. J-Son(8) 2.799 ms.
- Not claimed: L10's own gain (C0-C3a are untimed; the split by commit is arithmetic).

### 14.2 L9 — the C0 reading and the C4 decision (P2, P5d, P5e, P5f)

- **(a) The class-bench band is CLEAR** (`levers/L9-contact-reuse/02-DESIGN-REV1.md:380`; FIRES iff high < 1.0,
  AMBIGUOUS iff low < 1.0 <= high): low **1.0771 ms** [1.0729-1.1125], high 1.6659; the low end is claimed above
  1.0 under all three readings, and all six per-process lows are >= 1.073. Per-class costs on J: separated 45.24
  ns, touching **435.56 ns**, stream 232.69 (N_sep 5,044, N_touch 4,515). On `4db26681` the separated class is
  already post-L9a, so the band prices essentially L9b alone.
- **(b) The realized-gain rule PASSES** (`:407-410`): one binary, J-A `--contact-reuse off` against `on`, W=1,
  [100,500): **18.1605 -> 16.3152 ms, Delta-T(1) = 1.845 ms (-10.16 %), claimed under all three readings, against
  a bar of 0.6 x 1.4994 = 0.900 ms** (the prediction 0.9897 x 4,515 x (435.56 - [100, 60]) ns = [1.4994, 1.6782]
  ms): 2.05x the bar, the worst process pairing +1.781. The armed twins: the narrowphase span Delta-t_np(1) =
  2.4093 -> 0.6846 = **1.725 ms**, solve -0.110 (manifolds 4,519.26 -> 4,467.67), reused 4,457.5 and full 46.7
  pairs per step (h >= 0.9896). At W=8 +0.142 / +0.179 ms ([0,500) / [100,500), IQR and SE). All of L9 (L9a,
  cross-binary, plus L9b): the narrowphase 3.076 -> 0.685 ms at W=1, 2.39 ms against the design's 1.4-2.5.
- **G-TW's other rows, off/on on one binary, all claimed faster:** J-D -19.57 % / -4.86 % at W 1 / 8; R -27.16 % /
  -6.09 % (manifolds +3.8 %); J-A -7.13 / -5.28 / -2.06 % at W 2 / 4 / 16. No claimed regression at any W. Poses:
  the reuse-off rows equal the C0 fixtures; reuse on gives J `0x30c5438bc6ad9ffa` at every W and on cfg a and
  default, and R `0xc8bbe34cf6a8afc6` at W 1 / 8.
- **Decision: BUILD C4**: `contact_reuse = true` by default, the only value-changing commit (`:161-165`, `:384`).
  It moves the J/R/J-Son/R-S pose fixtures (J500 -> `0x30c5438bc6ad9ffa`, R1100 -> `0xc8bbe34cf6a8afc6`),
  `PINNED_FINAL_HASH` and `A7_R1_D_MAX_BITS` (the reuse-off run stays at `0x3a3c_3896`), A7-R1's docs, A7-R2's
  freeze step, G2/G7/G8, H8's manifold count and every per-manifold denominator, the box-pile goldens and the tree
  lane's J-pose pins; from then on cross-window bridges use reuse-off rows (`analysis.md` § 2.3).
- Still owed to G-TW, on the C4 binary: the canary on the L9 binary, J-D and R at W 2/4/16, the armed W8
  per-contact table, and the formal A/B (this window's ran on C3's switch, the same code path).

### 14.3 The tree broadphase — the query after C3b (P3, P5g, P5c)

- **C3b SHIPS (LeafList stays).** Parent `6dd1f916` (RowWalk) against tip `983480a9` (LeafList) on the armed
  `C3b-TA-armed` row (`--cfg a --broadphase tree`): t_q (the per-process median of `phys_bp_query_ns` over
  [100,500)) **0.4044 -> 0.2102 ms at W=1 (0.520x) and 0.4126 -> 0.2129 at W=8 (0.516x)**, claimed under all three
  readings; c_q = t_q / 1,240 rows = 169.5 / 171.7 ns. Criterion on the tip, one binary: LeafList/RowWalk j100
  **0.330** (0.1446 against 0.4388 ms), 1240 0.330, uniform 0.525, disparity 0.457: none >= 1, J <= 0.55, uniform
  and disparity inside [0.30, 0.60]. The step: armed T -1.0 % / -5.0 %, the default tree row 6.946 -> 6.814 /
  2.618 -> 2.431 ms at W = 1 / 8 (none claimed under the window rule).
- **G4 rule 1 and the G5 span gate are cleared:** the tree span 0.4637 -> **0.2714** / 0.4738 -> **0.2758** ms
  against 0.36 / 0.35 (0.308 / 0.267 in the P5g block), and the J snapshot 0.1446 <= 0.30 ms. G5 on the tip, one
  binary: Delta-bp(8) = 1.698 ms against the >= 1.03 bar (W=1 1.808); tree against AllPairs -22.6 % / -44.2 % on
  the default row and -10.3 % / -30.6 % armed, at W 1 / 8, all claimed.
- **Attribution A is REFUTED, and F3 is TAKEN UP.** The same armed cell in the P5g block (same binary, same
  arguments but the output paths) reads **0.2354 ms at W=1, +11.96 % over the P3 block, claimed** (W=8
  reproduces, -1.7 %); the two blocks' value ranges do not overlap, and the cause is untested. t_q > 0.21 ms is
  claimed there, so attribution A is refuted (the measured c_q matches attribution B's disparity-calibrated
  172-192 ns, while the bench ratio matches A's model). c_q = 169.5-189.8 ns is above 150 ns in both blocks, so F3,
  the reserve kd median-split leaf order (`c3b/design.md:168-172`), is taken up. Pooled K=12: 0.2222 ms, c_q 179.2
  ns.
- **C2 is flagged, not built.** Its letter (t_q >= 0.235 ms) is not robust: -10.5 % below it, claimed, in the P3
  block; 0.2354, on the bar and not claimed, in P5g. D6 (`levers/broadphase/04-DESIGN-REV2.md:204-214`) fires in
  both blocks (saving 0.167 / 0.164 ms = 6.9 / 7.3 % of T(8)). The analysis recommends F3 first and C2 after F3's
  re-time.
- **The default flip (the tree lane's C4) is not decided here:** G4 rule 1 and the span gate are cleared; rules 6,
  8 and 9 remain (`c3b/design.md:237-241`).
- Against Jolt v5.6.0 (window 3's cells; a reading): the tip's tree default row is 0.693x / 0.662x at W=1 (P3 /
  P5g block, IQR and SE) and 0.946x (not claimed) / 0.874x (IQR and SE) at W=8; the AllPairs default 1.566x,
  claimed.
- Not claimed: a same-binary armed C3b A/B (the runner has no `--bp-kernel`); the cause of the W=1 block shift.

### 14.4 L11 — the rest of G9 on C3 (P4) and bridge 2 (P5a)

- **PASS: no row is claimed slower under either rule.** Window 5's parent `0ca312bd` against its tip `cbd86a65`
  (the same two binaries), tip against parent: J-A -0.11 % / -0.68 % (W 1 / 8), R -3.65 % / -4.70 %, R-S -0.46 % /
  -0.91 %, S16 -0.32 %, J-As -3.72 / -4.23 / -5.34 % (W 2 / 4 / 16; W16 claimed faster), J-As-a -4.07 %, J-C
  -4.09 %. Armed stages (reading B): warm_apply on J-As-a W1 **0.5278 -> 0.2979 ms (-0.2299, claimed)**, the -0.19
  bar passed again (window 5: -0.2325); R -0.291 / -0.301 ms; wide colours not claimed slower (-4.65 %, -1.70 %,
  +0.69 %); solve_build and store not claimed except R-S store -11 % (faster) and S16's store 60 -> 80 ns (two timer
  quanta, 0.02 % of an unclaimed step; a reading, not a FAIL).
- **The canary is SEEN:** its span reads 1.0008x / 1.0006x the injection (tip / parent, every process within
  0.42 %); the step rise is +0.4481 ms on the tip (105 %), claimed under all three, and +0.4694 on the parent
  (106 %), claimed under SE only (G9's own form).
- **G9 is not formally closed:** J-A at W 2/4/16 was not run, and the design's K=12 is not met (every cell is K = 6
  or 5).
- **Bridge 2 HOLDS** (window 4b's tip `f8873aae` against window 5's parent on J-As, interleaved): +0.16 % at W=1
  (not claimed), **+1.75 % (+0.078 ms) at W=8, SE only**. The line merge costs at most 1.75 % at W=8, so window 5's
  +11.5 % was mostly machine state.

**Bridges and what they qualify** (`analysis.md` § 6). Every same-binary cross-window bridge holds under the
window rule (at W=1 within 1.2 %; at W=8 -4.2 % under IQR and SE and +5.0 % under SE only). At W=8 SE alone sees a
+-4-6 % window term, which qualifies every cross-window W=8 ratio (tree/Jolt 0.874-0.946, window 5's 1.057, the L11
chain). Window 4's RowWalk t_q at W=1 does not reproduce on the parent binary (0.4140 -> 0.4044, -2.32 %,
claimed): today's parent cost is 326.1 / 332.7 ns per row at W = 1 / 8, not 333.9. Two in-window duplicates fail
(the tip's armed-tree t_q at W=1, +11.96 %; the tip's default tree row at W=8, -7.6 %), so a single-block
absolute-threshold decision (C2's 0.235, "into the band", D6, a single-block T(8) headline) is not robust;
interleaved within-block comparisons are unaffected.

**Not claimed / not measured.** L10's own gain; L9's G-TW on a C4 binary; a same-binary armed C3b A/B; G9's J-A
at W 2/4/16 and its K=12; the Tree as the shipped default; any re-run of Jolt.

Receipts: `docs/measurements/2026-09-24-physics-window6/`: `README.md` (the protocol block, the binaries table
with the `Compiling` lines and the pruned lock, the idle-rule summary, what was reused from windows 3, 4, 4b and 5
and why), `analysis.md` (the analyst's reduction, verbatim), `plan.md`, `rows6.json`, `run_window.sh`,
`dryrun.txt`, `wait_log.txt`, `progress.txt`, `WINDOW_DONE`, `bin/SHA256SUMS`, `bin/COMMIT.txt` (no exe
committed), `logs/` (the builds and `pruned_Cargo.lock.txt`), `raw/` (`runs.jsonl`, `manifest_1790230189.json`,
`window_state.json`, `window_log.txt`, the 24 pass directories, `reduction.json`, `tables.md`; one `pose.bin` per
distinct pose, not one per process), `gate/` (the gate log and JSONs and the nine fixtures), `tools/`,
`analyst/`. `test/`, the exported trees, the gate's per-process directories, the exes, `__pycache__/` and the
duplicate pose files are not in the tree.

---

## 15. Physics — the Jolt headline in one window, L9 C4's timed gate, G9's last cells, and ours against Jolt stage by stage — TIMED 2026-09-25 (window 7)

**RESULT, 2026-09-25, window 7:**
- the Jolt headline does not hold under its two-block rule;
- L9 C4's G-TW passes;
- G9 on C3 stays closed;
- the per-manifold gap against Jolt is the narrowphase at W=1 and our serial stages at W=8.

**Protocol.** One window, complete after one stop, under the window-3 protocol block (the 2026-09-19 ruling, as
section 14 states it):
- a cell is the median over K separate processes of the process statistic; the spreads are min-max, IQR and the
  median's SE; a difference is claimed iff |effect| > 2*hypot(spread_A, spread_B) under BOTH min-max and SE,
  with IQR printed (r / i / s below);
- 5-s receipts between processes, and a receipt > 5 % => re-run once at the end of the pass;
- a 10-s receipt and three quiet 60-s polls before each pass, up to 30 polls;
- a build or lane process at any point of a timed pass voids the pass;
- P-none;
- every runner process ran with `--expect-pose` against its row's fixture.

**K = 6**: two passes x three rounds per block. Six blocks, in order: P1A-jolt, P2-L9GTW, P3-G9JA, P4-trkspans,
P5-joltprof, P1B-jolt.

**Where and when.** The owner's workstation (Ryzen 9 5900HS, 8C/16T, High performance), timed 01:09:37-01:29:50
and 02:54:16-05:03:57 +03:00.

**The stop.** The owner started a game at about 01:30.
- The idle rule held P1A-jolt-p1 back for its 30 polls, and the driver stopped with exit 3
  (`WINDOW_DONE.prev-1790293553`). `--resume` at 02:45:53 continued.
- No process ran 01:29:50-02:54:06, and the game appears in no receipt.
- **But P1A-jolt-p1 (02:55-03:12) and P2-L9GTW-p0 (03:14-04:00) ran with the owner's browser active,** which the
  idle rule and the 5-s receipts let through. The used processes' during-process witness reads medians of
  2.28 % / 1.90 %, against 0.54-0.66 % in every pass from 04:02 on.
- P1A-jolt-p0 ran with an agent session active.
- The cutoff was 10:00, then 10:30 (`raw/shell_log.txt`), not `plan.md`'s 04:15. Nothing was cut.

**Build.** rustc 1.98.1 `x86_64-pc-windows-msvc`, no RUSTFLAGS, `CARGO_INCREMENTAL=0`, cargo `parity`.

**Binaries** (sha256 prefixes, `bin/SHA256SUMS`):
- the trunk `93b2615b` `24d52719` (reuse off by default);
- `u/phys-l9-c4` `989ca0f0` `c4a75f82` (reuse on by default; `--contact-reuse` gives both arms).

  Both were built from `git archive` trees under the lock both commits track (LF sha256 `5f8de754…`, not pruned).
  Each build's `Compiling boyko-physics (path)` line names its tree.
- window 5's `0ca312bd` `3993684b` and `cbd86a65` `9c7caff1`, copied byte-identical;
- Jolt v5.6.0: P0's Distribution build `918fd2b7` (window 3's), run in place, and a profiled build of the same
  source, `aa23db26` (`PROFILER_IN_DISTRIBUTION=ON`, P5 only).

**Counts.** 542 records: 530 processes (12 warm-ups, 450 originals, 68 re-runs) and 12 pass markers. 431 of 450
slots are used (49 by their re-run); 19 are dropped because both attempts were hot (15 in the loaded P1A block).

**Validity.**
- Every used runner process: exit 0, void 0, and `expect_pose: match` with the hash equal to its fixture. There are
  five fixtures, all byte-identical to committed ones: J `0x32d5e235342b4143`; J with reuse `0x30c5438bc6ad9ffa`
  for cfg a and default; R `0x87e561d20589d4a5`; R with reuse `0xc8bbe34cf6a8afc6`. Also: workers = W, msvc,
  mask `0xffff`, TreeDiag as ruled, drops 0.
- Every Jolt process: one stat line, threads = W, hash `0xb8522b4e3fc62cfe`, the patch banner, 500 frames.
- The untimed gate: 112 processes, 0 failing. Four 501-step red controls exit 4. Jolt's `-receipt` gives 8,489.0
  manifolds per frame over [100,500), equal to window 3's.
- Receipts: 628; median 1.92 %, p90 5.68 %, max 21.98 %; 88 over 5 % (`browser.exe` 55, `claude.exe` 29). No
  build or lane process at any.
- The analyst's reduction reproduces the tester's in every cell (151) and every comparison (124).

### 15.1 P1 — the Jolt headline, same window (trunk `93b2615b` against Jolt v5.6.0)

- **The W8 headline does NOT HOLD.** The rule (window 6 FOLLOW-UP 15): claimed pooled AND in block A AND in
  block B, in the same direction.
  - The default row (`--cfg default --broadphase tree`, reuse off) against Jolt, [0,500): **A 0.8355 (n/Y/Y),
    B 0.8721 (Y/Y/Y), pooled 0.8111 (n/n/Y)**. The block-B cells are 2.1878 [2.1850-2.2027] against
    2.5085 [2.4711-2.5936] ms.
  - The rule holds at no W for either of our rows (A / B / pooled):

    | W | tree / Jolt | AllPairs / Jolt |
    |---|---|---|
    | 1 | 0.566 / 0.620 / 0.582 | 0.715 / 0.798 / 0.746 |
    | 2 | 0.675 / 0.682 / 0.683 (both blocks claimed, pooled not) | 1.009 / 0.989 / 0.995 |
    | 4 | 0.781 / 0.758 / 0.740 | 1.236 / 1.247 / 1.214 |
    | 8 | 0.836 / 0.872 / 0.811 | 1.474 / 1.585 / 1.470 (both blocks claimed, pooled not) |
    | 16 | 0.865 / 0.981 / 0.959 | 1.466 / 1.713 / 1.612 |

- **Why it cannot decide.** Block A ran loaded and is slower on every row at every W (B vs A −5 to −24 %). A
  pooled min-max range spans that shift, so the pooled cell cannot claim even where both blocks do. The rule as
  ruled needs two quiet blocks.
- **Per manifold** ([100,500), each side's own count: ours 4,519.2575, Jolt 8,489.0):
  - **ours is 1.178x Jolt at W1 and 1.601x at W8 in block B**, and 1.28x / 1.40x / 1.79x at W 2/4/16;
  - pooled reads 1.100x / 1.479x, but the pooled Jolt cell is inflated by block A (+8.8 % at W1);
  - Jolt carries 1.878x our manifolds and 1.828x our points. Its count equals ours at frame 0 (4,495) and diverges
    during the collapse.
- **Scaling** T(1)/T(W), block B, at W 2/4/8/16: ours 1.52 / 2.15 / 2.72 / 2.52, Jolt 1.67 / 2.63 / 3.83 / 3.99.
  Our tree row is slower at W16 than at W8.
- Tree against AllPairs (block B): −22 to −45 %, claimed at every W.
- Window term (context): Jolt in block B reads 0.976-1.020x window 3 at every W.

### 15.2 L9 — G-TW on the C4 binary (P2)

- **G-TW PASSES** (`levers/L9-contact-reuse/02-DESIGN-REV1.md:405-414`): one binary, `989ca0f0`,
  `--contact-reuse off` against `on`.
  - **Realized gain:** J-A, W=1, [100,500): 18.4127 -> 16.4753 ms, **Delta-T(1) = 1.937 ms (-10.52 %), claimed
    under all three readings, 2.15x the 0.900 ms bar** (window 6's prediction: [1.499, 1.678] ms). The worst
    pairing is +1.612. Window 6 read 1.845 ms on C3's switch.
  - **No claimed regression at any W,** under either rule: J-A, J-D and R at W 1/2/4/8/16, over [0,500),
    [600,1100) and the J [0,100) witness.
    - Claimed faster: J-A W1 -9.25 %, R W1 -27.06 %.
    - Every other point estimate is faster except R W8 (+0.62 %) and J-D W16 [0,100) (+2.12 %), neither claimed.
  - **Resolution caveat:** pass 0 (the browser) widened the cells. The min-max bars at W >= 2 are 26-73 % (SE 5-15 %),
    so the claim-free reading there is weak.
    - Pass 1 alone (a diagnostic, K=3, quiet), at W 1/2/4/8/16, every row faster:
      - J-A: -9.3 / -8.2 / -5.5 / -2.4 / -1.9 %;
      - J-D: -20.5 / -15.7 / -10.1 / -6.0 / -6.3 %;
      - R: -26.8 / -17.0 / -13.2 / -7.7 / -3.0 %.
  - **The canary is seen under SE, not under the two-spread rule.**
    - Injected 0.845 ms (5.07 %); its span reads 1.0003-1.0007x the injection. The step rise is +0.896 ms
      (+5.38 %, 106 %), with bars 10.96 / 6.01 / 2.40 % (n/n/Y).
    - The design's own form is the SE claim rule (`:407`), and window 6 accepted G9's parent canary the same way.
    - Pass 0's load set the 4 % ranges. Pass 1 alone (0.16 / 0.18 %) sees the canary at 11x the bar.
    - Under min-max no K would see a 5 % canary at this window's spreads, because the expected range grows with K.
      A canary_frac of about 0.12 would be seen.
  - **Poses:** every `on` row reads one hash across W 1-16 (J `0x30c5438bc6ad9ffa` on cfg a and default, R
    `0xc8bbe34cf6a8afc6`), and every `off` row reads C0's.
  - **Per contact** (armed J-A, [100,500)):
    - np 2.6112 -> 0.7511 ms at W1 (-71.2 %, claimed): 0.168 us per manifold and 78.7 ns per pair with reuse on.
      At W8, 0.4526 -> 0.1591.
    - Manifolds 4,519.26 -> 4,467.66; reused 4,457.5 and full 46.7 pairs per step.
    - Narrow colours rise 6.5x (+0.15 ms, claimed), a recorded side effect.
- **So C4 can merge** under window 6's pin list (`levers/00-RULINGS.md`, the L9 block). The canary's SE form and
  the W >= 2 resolution are the orchestrator's to weigh (window 7 `analysis.md` FOLLOW-UP 1).

### 15.3 L11 — G9's J-A record (P3)

- **Not claimed slower at any W.** J-A, tip `cbd86a65` against parent `0ca312bd`, K=6: +0.14 % (11.5423 ->
  11.5580 ms, W2), +0.02 % (W4), -0.02 % (W16); n/n/n, bars 2.5-12.5 %.
- Every G9 row now has a cell and none is claimed slower, so **G9 on C3 stays CLOSED** (the ruling of 2026-09-24).

### 15.4 Ours against Jolt, stage by stage per manifold (P4, P5)

- **Our spans** (the trunk's default row, armed, [100,500), reading A), in ms:

  | W | bp | np | graph | setup | warm | colours | integrate group |
  |---|---|---|---|---|---|---|---|
  | 1 | 0.266 | 2.427 | 0.115 | 0.284 | 0.299 | 2.434 | 0.098 |
  | 8 | 0.268 | 0.382 | 0.117 | 0.312 | 0.308 | 0.619 | 0.122 |

  - **Five stages and gather/apply do not shrink from W1 to W8** (bp, graph, setup, warm, integrate: 0.80-0.99x).
    Together they are 1.10 ms at W1 (18 %) and 1.17 ms at W8 (53 % of the step).
- **Jolt's profile** (the profiled build). Its overhead is +36.1 % at W1 and +18.7 % at W8, so only shares are
  read.
  - About 65 % of the step is solve (velocity 54-58 %). About 27 % is one FindCollisions job: broadphase pairs
    4-5 %, narrowphase 24-27 %.
  - Its narrowphase runs on the body-pair cache. 8,488 of 8,489 manifolds per frame go through "Add Constraint
    From Cached Manifold", which also builds the contact constraint, and the SAT runs once per frame.
  - `SetupVelocityConstraints` (non-contact only) costs 0.5 us.
- **Per manifold, block B.** Jolt is scaled uniformly to its unprofiled step; at W1 a per-sample overhead model
  brackets it.
  - **W1:** ours 1,318 against Jolt 1,119 ns (the step). The whole gap is the narrowphase: np 537 against 263-301,
    and np + setup + warm 666 against 273-321. Our solve is 186-224 ns cheaper, and bp is at parity (59 against
    57-72).
  - **W8:** ours 482 against 301 ns. The gap is the five serial stages: bp +45, setup +69, warm +62, graph +24,
    integrate +23 ns. np (+12) and solve (-60) are at or below Jolt.
- **What C4 removes** (arithmetic: P4's np times the armed on/off ratio):
  - at W1, np -380 ns per manifold (537 -> 156; np + setup + warm 287, inside Jolt's bracket), so the W1
    per-manifold ratio goes from about **1.18x to 0.85x**;
  - at W8, np -54 ns, so **1.60x -> about 1.44x**.
- **After C4 the whole W8 gap is the serial stages.** Their owners (window 7 `analysis.md` § 6 and FOLLOW-UP 2-6):
  - the tree lane's C2 (the parallel query, after F3);
  - L11 C4 (D10's parallel fill: solve_build is 14 % of T(8), over its 5 % trigger);
  - three with no lane yet: the warm apply (14 % of T(8) at W8), the graph build, and the integrate group.

**Not claimed / not measured.**
- The P1 headline under its rule (block A was loaded).
- G-TW's "no regression" at W >= 2 as strong evidence.
- The canary under the two-spread rule.
- Any per-stage Jolt time (shares only).
- The post-C4 per-manifold ratios. They are arithmetic: no binary had both the Tree default and C4.
- Why Jolt's manifold count doubles during the collapse.

**Receipts:** `docs/measurements/2026-09-25-physics-window7/`:
- `README.md`: the protocol block, the stop and the resume, the binaries table with the `Compiling` lines and the
  lock, the idle-rule summary, what was reused and why;
- `analysis.md`: the analyst's reduction, verbatim (see `README.md`);
- `plan.md`, `rows7.json`, `run_window.sh`, `dryrun.txt`, `wait_log.txt`, `progress.txt`, `WINDOW_DONE`,
  `WINDOW_DONE.prev-1790293553`;
- `bin/SHA256SUMS`, `bin/COMMIT.txt` (no exe committed);
- `logs/`: the two runner builds and the profiled Jolt build;
- `raw/`: `runs.jsonl`, the two manifests, `window_state.json`, `window_log.txt`, the 12 pass directories with the
  P5 profile dumps, `reduction.json`, `tables.md`; one `pose.bin` per distinct pose, not one per process;
- `gate/`: the gate log and JSON, the five fixtures, the Jolt receipt;
- `tools/`, `analyst/`.

Not in the tree: `test/`, the exported trees, the gate's per-process directories, the exes, `__pycache__/` and the
duplicate pose files.

---

## When an entry is done

Strike it with the date and the receipt's location, rather than deleting it. An entry that was run
and produced a surprising number is worth more as history than as a blank line.
