# Code review: W8S instrument, commit `226bd99e` (u/phys-w8s, parent `a864fefb`) — build-free, 2026-09-27

(Written by the orchestrator from the code-reviewer's returned report; the reviewer's harness could not write files.)

> **Record note (2026-09-27, added when this review was recorded under window 8; the text below it is unchanged).**
> Every `file:N` citation in this review is a line of commit `226bd99e` (`u/phys-w8s`), which is **not** on the
> trunk `integ/unified` at the time of recording (`git merge-base --is-ancestor 226bd99e d95ee579` fails). Read them
> with `git show 226bd99e:<path>`; on any other revision the same numbers name other lines. The bare file names
> resolve, at `226bd99e`, to:
> - `colored.rs` = `crates/boyko_physics/src/solver/colored.rs`;
> - `profiling.rs` = `crates/boyko_physics/src/profiling.rs`;
> - `dispatch.rs` = `crates/boyko_physics/src/narrowphase/dispatch.rs`;
> - `jolt_parity_pyramid.rs` and "runner" = `crates/boyko_physics/benches/jolt_parity_pyramid.rs`;
> - `omega_b_region.rs` = `crates/boyko_physics/benches/omega_b_region.rs`;
> - `scope.rs` = `crates/boyko_threadpool/src/scope.rs`;
> - `01-DESIGN.md` and "design" = `docs/physics/perf-campaign/levers/scaling/01-DESIGN.md`, a document that exists
>   only on that branch at the time of recording.
>
> Spot-checked at `226bd99e` when recorded: `jolt_parity_pyramid.rs:2085` is `let in_solve`, `dispatch.rs:705` the
> `unsafe { np_chunk::<SETS, true>(..) }` call, `colored.rs:3565` a `continue;`, `colored.rs:4028`-`4029` the
> `stamps.reduce()` test and its `tally.add`, `scope.rs:863`/`:942` the two `pending.fetch_sub(1, AcqRel)` and
> `:984` the `pending.load(Acquire)`, `omega_b_region.rs:518` `fn omega_scope`, `01-DESIGN.md:448` the
> recruitment-ramp bullet, and `profiling.rs:651` / `:652` the `ramp:` / `tail:` fields of `WaveReading`.

## Verdict
CHANGES REQUESTED for the lane: two blocking items (B1, B2), both small, in cold armed-only code. No UB, no data race,
no pose risk. WINDOW: NO-GO for the per-wave split only; every other row GO under reduction rules N1-N3, N5, N7;
omega(W, gap) GO only as a lower bound (N4).

## ORCHESTRATOR DISPOSITION (2026-09-27)
- Window 8 runs NOW on `226bd99e` for the GO rows. The per-wave split (B1/B2) is read in window 8b on the S4-AB
  parent binary, which carries the fixed instrument.
- B1, B2 -> fix in the lane (new commit before `--bp-kernel`'s or right after it; cold armed path only; disarmed code
  untouched; the codegen receipt re-taken). Fold N3 (helped-wave count) into B1 and N6 (np route counter) in.
- N2 -> preferred fix: `TaskStamp` on its own 64-byte line and `next` on its own line (the cheap one); the D(W)
  bound is the reduction rule either way.
- N4 -> the bench-local participation receipt (bench exe only).
- N1, N5, N7 -> reduction rules (below); N8 -> no action.
- Open Q1 (`WAVE_RECORDS.fetch_add`, caller-side, uncontended): ACCEPTED as the review-W4 exception; the SAFETY/doc
  line names it.
- Open Q2: R_W is read as F*T plus the wall canary's executor boundary (~3.9 us, design §3).

## Answers
1. Placement: the four `phys_sb_*` zones tile `phys_solve_build` (200 ns median residue, W8/W16/rest). Runner
   `in_solve` (jolt_parity_pyramid.rs:2085) holds neither sub-zones nor colour spans — no double count. Instrument
   time (armed only): S6 probe + histogram + tally push -> u (+8.2 us W8, +8.8 us W16; u/solve 0.59 % median,
   1.96 % max, 3 % closure holds); per-wave reduce -> r (+37 us W8, +41 us W16, N1); stamps -> the wide colour span,
   i.e. into L(W) (N2). Per-wave means derivable as per-step sum / solve scopes; per route only from the CSV (steps are
   route-pure; the SUMMARY `w8s` block pools routes; on every lane row `route_external` = 0 and solve-on-dispatcher
   0/500). NOT derivable: first-wave recruitment ramp (B1), imbalance vs join inside the tail (B2).
2. Disarmed cost: per-step `zone_enabled!` gates + one per-colour `Option` test. Receipt GREEN on task closures,
   `solve_color_dispatch`, per-pair paths, `find`, the five simd kernels; not covering `fill_cohort`, `build_columns`,
   warm applies, gravity dispatcher (N8, acknowledged).
3. Atomics: all stamps Relaxed; task slot unique via `fetch_add` before the body; the record is read after the join,
   whose `pending` Acquire pairs with every task's AcqRel `fetch_sub` (scope.rs:863/942/984). No race. Poses cannot
   change (same cut, same closure; the `continue` at colored.rs:3565 skips nothing).
4. Canary flags: in-zone spin inside the admitted arm after the guard opens (colored.rs:2035/2111/2126/4429/4433);
   refused without `--arm-profiler` (exit 2); void if spins != armed openings; `ZoneCanary::default()` slot 0 = no
   spin; no `--canary-frac` = no canary system; F >= 1 uses `(F*T).round()`.
5. omega_b_region: modes `omega-b` and `omega`, both routes, each asserts its route. Region protocol = rev 1 W1
   (fixed table, claim-word CAS exec-1 -> exec, publish word `(exec<<16)|stage` with END, exact `done` with reset,
   one-block stages inline, helpers PAUSE <= 5 then yield, never park). Release/Acquire correct. Caveats N4, N5.
6. Unsafe: one new block (dispatch.rs:705), SAFETY restates the disjoint-cut invariant, which holds. Bench: none.

## WINDOW-BLOCKING (for the per-wave split)
### B1. First-wave recruitment ramp not recoverable from per-step sums
profiling.rs:651 (ramp per wave), :705-735 (`WaveTally::add/push`, sums only); colored.rs:4028-4029, :4656-4658.
Design §6.1 (01-DESIGN.md:448-457) says W8S "measures today's first-wave ramp" — it prices S1's once-per-step
recruitment charge-back (0-0.07 ms) and the "open the region before P-a" mitigation; §3 (:147-154) expects a wake
cascade at pass-first waves. As built each step pushes the sum over ~96 waves with no first-wave / pass marker.
Step (a) all waves 0.78 us vs step (b) first 60 us + 95 x 0.15 us: same `phys_wave_ramp` (~75 us). On J-T W8 the sum
(~96 x 776 ns = 75 us) bounds the first-wave ramp only to <= 75 us — no tighter than the 0-70 us prior.
FIX (cold armed path): per-step counters for the step's first dispatched colour wave (ramp, tail); the sum of ramp over
the first wave of each pass (`solve_color_stamped` has `color`; colour index <= last stamped index = new pass).
+2..3 samples/step (266 of 1024 used). Fold N3's helped-wave count in. No-code alternative (not taken):
`phys_np_wave_ramp` as proxy.

### B2. Tail = end imbalance + join latency, unsplit
profiling.rs:652 (`tail = joined - max(spawned, caller_last)`); :636-638 already sort `ends` (max task end discarded).
Design §3 :152-154, §6.1 :446/:458 need dispatch loss apart from imbalance; the tail is the largest measured wave
loss (1.59 us W8, 2.18 us W16 vs ramp 0.78/0.76). The gap is in the cut's own definition, not the developer's work.
FIX: push the per-step sum of (joined - max task end) = join latency, same for the np wave; imbalance = tail - join.
+2 samples/step. Runner check `Any` with join <= tail.

## NON-BLOCKING
- N1 r(W>=2) is mostly the instrument's reduce (colored.rs:4027-4029; runner :2127): r per wide span 36 -> 423 ns
  (W8), 44 -> 467 ns (W16); per step 3.4 -> 40.6 us and 4.2 -> 44.9 us. RULE: subtract r_a(W) - r_a(1) from every
  engine term. Optional: rdtsc-time the reduce as one more counter.
- N2 stamps inflate the armed wide span at W>=2 (`WaveStamps::new()` profiling.rs:549-560 zeroes 3 KiB + global
  fetch_add inside the colour span, colored.rs:4013; per task `fetch_add` on `next` sharing a line with caller fields,
  24-byte slots ~2.7 per line written by different workers). Estimate 0.25-0.5 us/wave (~23 tasks/wave), 8-17 % of
  L(8) — PLAUSIBLE, not measured. RULE: D(W) = [T_a(W) - T_d(W)] - [T_a(1) - T_d(1)] - dr(W) - du(W), reported beside
  omega(W). Optional fix: 64-byte `TaskStamp`, `next` on its own line; or a doc(hidden) stamps-off setter.
- N3 unhelped wave records ramp 0 (profiling.rs:651): <= 1.5 % of waves at W8, <= 24 % at W16. Fold into B1.
- N4 omega(W, gap) has no participation receipt (omega_b_region.rs:518-531 `|| {}` tasks; :534-549 scope wall only).
  RULE: omega(80) - omega(0) = dispatcher's cost of waking the pool, a lower bound on park->wake. FIX: bench-local
  receipt (reps where any task ran off the caller; first helper start).
- N5 omega_b `stage_ns_median` includes per-region recruitment (omega_b_region.rs:488-513). RULE: two `--stages`
  values (36, 72), slope = per-stage cost.
- N6 np wave route not recorded (`push_np` profiling.rs:666-672). One counter if np ramp is used.
- N7 S6 probe lands in u, +8 us to armed S(1) (colored.rs:4423-4425, :1746), ~0.7 % of S(1). RULE: name it.
- N8 receipt misses inlined hot bodies (acknowledged); no window impact.

## Positive (keep)
Sub-zones tile `phys_solve_build`; disarmed spawn loops byte-equal; the `STAMPED` tag on `np_chunk`; the spawn
witness `wave_records_built`; the canary spin receipt (R1-5); runner recomputes the task count from its own cut walk;
per-step route class; the Relaxed + join happens-before argument; omega_b protocol and its bounded self-check (R1-6
reds).
