VERDICT: APPROVED WITH CHANGES; BLOCKING=1; IMPORTANT=4

# Architecture review: Physics throughput vs Jolt, measured first (Rev 1)

## Verdict
[ ] APPROVED
[X] CHANGES REQUESTED. P0's design is sound. The changes below are narrow, and B1 must be fixed before Step 0 starts.

The plan's §0 answer to the owner holds, and I checked it against the code. Sleeping cannot move the parity ratio: both sides turn it off (`D:/tmp/jolt/JoltPhysics/PerformanceTest/PyramidScene.h:44`, `D:/wt/joltab/crates/boyko_physics/benches/jolt_parity_pyramid.rs:217`). Our sleep only skips the solve and the integrate (`colored.rs:1606`, `:3282-3297`, `:3414-3426`).

## Blocking

#### B1. The B1 test asserts on a counter that counts manifolds, not warm-start hits
- **Where:** §4 Step 0.1: "Assert the warm-seed `seeded` count (colored.rs:1595) equals the point count on the wake step. Today I expect 0."
- **Problem:** `seeded += 1` runs once for every manifold pushed with at least one live point (`D:/wt/joltab/crates/boyko_physics/src/solver/colored.rs:1619-1624`). It runs whether or not `warm_read.get` found an entry (`:1741-1744`). `WarmSeedStats` has no field for hits (`row_identity.rs:258-267`); on an unchanged step, `translated` is just a copy of `seeded` (`colored.rs:1638`). On the wake step every manifold is pushed, so `seeded` equals the manifold count. It is never 0 and never the point count.
- **Consequence:** The assertion is red before the fix and still red after it, because it compares manifolds with points. It "confirms" B1 whether B1 is real or not, and it can never pass. The rule "if red, the fix gets its own plan" therefore always fires. This is the repository's "red for the wrong reason" failure.
- **Confidence:** CONFIRMED (file:line above). B1 itself is also confirmed by reading: frozen manifolds never reach `columns` (`:1606`), `store_and_swap` rebuilds the table from `columns` alone (`:3115-3123`), and `rebuild` zeroes every slot (`warm_start.rs:261-275`).
- **What is needed:**
  - Measure warm hits per point: either a new hit counter, or the count of points whose seeded impulse is non-zero after `build_columns`.
  - Add a twin pile that never slept, as the control. It must show hits equal to points at the same step, which proves the assertion can pass.
  - Expect hits = 0 before the fix.
  - Replace the threshold-free velocity check with a comparison against that twin.

## Important

#### W1. `[profile.parity]` uses the one codegen configuration this repository measured as a regression
- **Where:** H5, `inherits = "bench"`, `lto = "fat"`.
- **Problem:** `bench` sets `codegen-units = 1` (`D:/wt/joltab/Cargo.toml:115`). The root manifest records fat LTO plus `codegen-units = 1` as "the ONLY configuration in the whole matrix that is SLOWER than the pre-2026-09-04 default on any benchmark: query_ref_iter/10k regresses 17.4%" (`Cargo.toml:104-110`). The shipped profile is `release`: fat LTO with the default codegen units (`:111-112`). Jolt's side of the comparison is its shipped Distribution build.
- **Consequence:** The headline ratio at every W carries a codegen artifact of unknown size and sign. The lever gains measured under it may also not carry over to what ships.
- **Confidence:** CONFIRMED.
- **What is needed:**
  - Take the headline under our shipped profile (inherit `release`).
  - If single-codegen-unit reproducibility is still wanted for the A/A2 and lever A/Bs, keep it as a separately named profile, and say which number comes from which.

#### W2. The L6 fork condition is always true
- **Where:** §2 defines ω(W) = L(W)/waves. L6's fork is "if ω(8)·waves ≥ 0.5·L(8), build a new pool primitive."
- **Problem:** By that definition ω(8)·waves is exactly L(8). The test reduces to L(8) ≥ 0.5·L(8), which holds for any L ≥ 0.
- **Consequence:** Whenever L6 clears its build-if rule, the fork always picks the high-cost branch: a new pool primitive plus loom models. It does so even when the loss is imbalance or quantisation, which the primitive does not remove. One known case: `MIN_SLOTS_PER_CHUNK` yields 9 chunks on about 500 slots, so at W=8 one chunk is a straggler (`colored.rs:2833-2835`). A gate that cannot fail breaks the plan's own rule.
- **Confidence:** CONFIRMED (the plan's own definitions).
- **What is needed:**
  - Compare a fixed per-wave dispatch cost against L(8). That cost could come from ω₁ (J-P1), a microbenched spawn/join cost at zero work, or the wake component.
  - Keep the imbalance share separate from it, for example max chunk time minus mean chunk time, or E measured with constant-retune only.

#### W3. Lever gates read the lever's own span, not the step time, so the plan's I(W) term is never gated
- **Where:**
  - L4 builds if t_wide(W) < t_wide(1); its gate is correctness oracles plus S16.
  - L5's "realized gain ≥ 0.6× predicted" does not say what it is measured on.
  - L7 has correctness gates only.
- **Problem:** Moving work onto workers shifts cost into I(W). After a color-parallel wave, body rows sit in other cores' caches, and the serial stages that follow re-fetch them (`warm_start_apply`, `position_integrate`, `refresh_inertia`, 4× per step, `colored.rs:3345-3384`). The plan defines I(W) = S(W) − S(1), but no build-if rule or gate reads it.
- **Consequence:** A lever whose own span shrinks can still make T(W) worse, and pass. Plausible at W=2, and on the R scene, whose wave count is unmeasured.
- **Confidence:** PLAUSIBLE for the mechanism; CONFIRMED that the gate text omits T.
- **What is needed:** Every bit-identical lever (L2–L7) gets an end-to-end gate on median T(W), judged against the A/A0 band, at each W in {1, 2, 4, 8, 16} on J and R, with I(W) reported before and after.

#### W4. The instrument can pass all its checks while recording nothing, and the validation tests are not isolated
- **Where:** §2 "Closure", and the Validation section.
- **Problem:**
  1. **Empty or partial profiles pass closure.**
     - With every span at 0: u = 0, g = T ≥ 0, Σ spans ≤ wall, overflow = 0, and waves 0 = 12×0. Every closure check passes.
     - With only the system spans missing, u goes negative, and closure never checks u < 0.
     - Only J-C's canary would catch it, and J-C is not every run.
  2. **Arming is process-global.**
     - The arm mask is one static (`boyko_diag/src/profiling_abi.rs:546-548`).
     - The store binds to one world per process; a second world is refused with E9204 and its fold silently returns (`boyko_ecs/src/ecs/core/profiling/mod.rs:164-170`, `plugin.rs:88-92`). The ECS's own tests serialise with an `exclusive()` guard for this reason.
     - As planned, `physics_zones_count_exactly` and `instrumented_step_is_bit_identical_armed_and_disarmed` would run concurrently with other physics tests. Other tests' samples would inflate the "exact" counts, the "disarmed" arm would actually be armed, or the second world would record nothing.
  3. **The wide side of the count test is vacuous.** On a "small colored pile" no color reaches 256 slots, so a bug that labels every color narrow passes.
  4. **The bit-identity test cannot fail.** Zones write no simulation data, so it passes with or without arming unless it also asserts that the armed run recorded samples.
- **Consequence:** A dead or half-dead profile yields f, E and ω numbers that look plausible, or NaN. Tests go red for the wrong reason or green from emptiness.
- **Confidence:** CONFIRMED (file:line above).
- **What is needed:**
  - Per-run anti-vacuity: each zone's count per step must equal its structural expectation (build 1; gravity, warm and biased 4 each; relax 8; one span per system), otherwise the row is void.
  - Add u < 0 to closure.
  - Give the validation tests their own `harness = false` binary, or the kernel's `exclusive()` discipline.
  - Use a fixture with at least one wide color (the `many_disjoint_pairs_are_one_wide_color_and_must_dispatch` shape).
  - Assert armed samples > 0 and disarmed samples = 0.

## Optional

- **O1. `zone!` does write shared state.** "No shared writes" is not accurate: `ZoneGuard::drop` does two `lock`-prefixed `fetch_add`s on the zone handle's static atomics (`profiling_abi.rs:500-501`). This is harmless at the planned sites, which run on one thread. Never put a zone inside a worker's chunk task, where it would contend.
- **O2. The identity leaves out a residue.** r = Σ pass spans − Σ color spans is missing, and the instrument's own per-color cost lands in r. The "checked in the gates below" line has no matching gate. Add r to the identity and bound it.
- **O3. L5 details:**
  - "Count, prefix, emit" cannot be done for narrowphase without running the SAT twice. The real shape is per-chunk buffers concatenated in pair order.
  - The predicted gain should subtract the serial snapshot pre-read, the serial axis commit and the compaction copy.
  - The `box_box` test counters are `thread_local` (`narrowphase/box_box.rs:746-756`). Tests reading them on the test thread will undercount once pairs run on workers.
  - The exactness argument itself checks out (see Positive).
- **O4. H4 is a no-op.** `PyramidScene::StartTest` never reads `inMotionQuality` (`PyramidScene.h:23-46`), and the default is Discrete (`BodyCreationSettings.h:100`). Keep `-q=Discrete` because it halves the runtime, but drop "motion quality unknown" as a caveat on the 09-10 numbers.
- **O5. J-Son vs `-allow_sleep` compares different sleep rules.**
  - The two engines sleep by different criteria: ours is 1e-4 speed² for 60 frames; Jolt's is about 0.03 m/s for 0.5 s.
  - Record the awake count per step on both sides and compare only the tails after everything is asleep, not the window means.
  - `-allow_sleep` must be threaded into `StartTest`, because the scene hard-codes `mAllowSleeping = false` per body (`:44`). Only `-no_sleep` exists today (`PerformanceTest.cpp:159`).
- **O6. Tier mismatch in the headline.** The headline runs under `BOYKO_PROFILE=dev`, while Jolt's Distribution compiles its profiler out. Add one disarmed `shipping` row, or state it.
- **O7. Binaries, pins and a no-op:**
  - L1's gate (B/A on J-S0) needs a binary built before L1. It is not in the window's build list.
  - The B1 fix changes simulation values on wake, so pins in the sleeping-on suites will move. Say so in its sub-plan.
  - Implementation step 5 is already done: the bench is `harness = false` (`boyko_physics/Cargo.toml:214-216`).
- **O8. Label a prediction.** §0.2's "would widen the gap" is an inference from code; mark it as a prediction that J-Son will measure.

## Positive (preserve)

- The instrument choice. Kernel `SystemSpan` plus `zone!`, folded outside the timed region, is right. Rejecting probe systems is right, because each would add an executor round and a wake, perturbing exactly the dispatch term being measured.
- The wide/narrow split is W-independent, as the plan claims. For a color of 256 or more slots, `n_chunks ≥ 2` always holds (`colored.rs:2780-2846`).
- L5's exactness argument is correct:
  - `set` never moves or evicts an entry within a frame; it only fills an empty slot or overwrites its own key (`axis_cache.rs:319-354`).
  - The only clears are in `begin_frame` (`:261-281`), which runs before the pair loop.
  - So a snapshot read of each key equals the in-loop read.
- The fixed-window runner matching Jolt's timed window (500 steps from t=0, per-frame CSV) is right; the Criterion window was confirmed broken (`bench:243-252`).
- The plan keeps the diagnosis honest:
  - the H6 receipts;
  - the canary, sized to exceed the noise band so the gate is proven able to fail;
  - quarantining the 09-09 and 09-10 numbers;
  - flagging every value-changing lever (L8–L10, D1) as an owner decision.
- The Jolt v5.6 figure checks out: the release notes report the new friction model as 15 % faster on Pyramid.

## Open questions

1. Does the runner build more than one world per process? Today's bench does, with a throwaway `build(1)` at `:231`. If so, E9204 refuses the profiler for the measured world.
2. At W=16 on 16 logical CPUs, does the dispatcher thread also execute the solver's scope tasks? If it does, boyko runs W+1 threads, and "equivalent to Jolt's W−1 workers plus the caller" needs re-checking.

Sources:
- [Jolt Physics v5.6.0 release (newreleases.io)](https://newreleases.io/project/github/jrouwe/JoltPhysics/release/v5.6.0)
- [Jolt Physics 5.6 Released – GameFromScratch](https://gamefromscratch.com/jolt-physics-5-6-released/)