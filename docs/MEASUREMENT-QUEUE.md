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

## When an entry is done

Strike it with the date and the receipt's location, rather than deleting it. An entry that was run
and produced a surprising number is worth more as history than as a blank line.
