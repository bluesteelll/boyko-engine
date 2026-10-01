# Tree broadphase C4: the lane record

Lane TREE-C4, 2026-09-30, branch `u/phys-tree-c4` from `16191fda` (`integ/unified`). The design is
`04-DESIGN-REV2.md` (C4 at "Commits", D7, "What moves") and the Jolt-gap plan's lever B1 / KD11:
C4 starts now, value-neutral, and merges at the next quiet window. No timing was taken in the lane;
every claim below is structural (bits, counts, codegen). The claim C4 needs is made in window 9
(section 6).

"arith." marks a number computed from code or counts, not measured.

## 1. What changed

| commit | what | value |
|---|---|---|
| C4-1 `1537eeb3` | `TREE_BRUTE_MAX_ROWS` 144 → 128 (`broadphase_tree/mod.rs`); `AUTO_TREE_LO/HI` 144/152 → 128/136 (`broadphase_policy.rs`); both PROVISIONAL (section 2). G-TH1 re-scened from the constants | neutral |
| C4-2 `b858c238` | Auto bands on `AUTO_TREE_LO/HI` and selects AllPairs ↔ Tree, never Grid; the coupling path pins `broadphase_select = Manual` beside its forced Grid (`plugin.rs`, D7). `GRID_LO/HI` stay documented `pub const`s that Auto no longer reads | neutral |
| C4-3 `7af2d090` | `PhysicsConfig::default().broadphase` and `BroadphaseKind`'s own `#[default]` → `Tree`. `parallel_broadphase` stays off; `broadphase_select` stays `Manual`; the default query kernel stays `LeafList` (`LeafListKd` is never auto-selected, ruling 4 of 2026-09-29) | neutral |

Why the flip moves no value: every kind emits AllPairs' exact pair set in `(min, max)` order (G1),
and the Tree runs `all_pairs_into` itself at or below `TREE_BRUTE_MAX_ROWS` rows.

## 2. The provisional thresholds

- **The reading** (window 8b's G4 block: `16191fda` + the G4-sizes patch, bench profile, K = 3,
  the Q3 recipe; figures quoted from the window's verified analysis
  `docs/measurements/2026-09-28-physics-window8b/analysis/f3.md:104-118`; PC-C4-1, the path of that
  record, is resolved):
  - `all_pairs` is claimed slower than `tree` at 144 in both families: all_pairs/tree 1.1294
    (uniform), 1.1003 (disparity); at 152: 1.1971 and 1.1432.
  - The recipe (`treebp/g4_g5_recipe.md`, ruled on 2026-09-25) reads LO 128 / HI 136 in the
    disparity family and under window 7's rule in the uniform family; under ruling 1 the uniform
    LO falls below the grid's bottom (128).
  - Window 7 wave 2's 144/152 did not reproduce. The move came from `all_pairs` (8b / window 7
    median 1.1380), not from `tree` (1.0189).
- **Status.** PROVISIONAL, by the 2026-09-29 ruling 5 as the lane spec applies it: window 9 re-reads
  G4 at 96..160 in steps of 8 under ruling 1 (section 6, block C4-G4), and a one-constant
  follow-up commit sets the final values, with G-TH1's reds re-shown. **Window 9a's resume re-read
  G4 and the values are now MEASURED: section 8.**
- **What imprecision costs** (arith. from `f3.md:104-118`): at 144 the two arms are 14.154 µs and
  12.532 µs apart by 1.6 µs; one 8-row grid step near the crossover is worth about 1 µs per step.

## 3. M9: the all_pairs slowdown between window 7 and window 8b (structural)

**Verdict (b): the timed code is byte-identical in the two measured binaries; only its placement
differs, and only at the 64-byte level.** No code cause is named. Whether a 64-B split explains
the 12-16 % shift, or the shift is a window term, is decided by window 9's bracket C4-BR.

- **Builds.** The G4 bench (`cargo bench --no-run -p boyko-physics --bench broadphase`, bench
  profile) at `16191fda` from the lane worktree, and at `93b2615b` from a `git archive` export
  (under `D:/wt/_targets/tmp`, cargo run from the lane worktree with `--manifest-path`, so the
  `.cargo/config.toml` read is the lane's, identical at both shas). rustc 1.98.1 `48a229cea`
  msvc, no RUSTFLAGS. Building the bench target (not `cargo rustc --lib`) links the same rlib the
  measured exes link, with dev-dependency features unified the same way.
- **Codegen.** `broadphase_tree::all_pairs_into` (what G4's `all_pairs` arm times): 117
  instructions at both shas; the inner j-loop `[0xe0, 0x192)` is 42 instructions, 178 B, at
  both. The only difference is the name a cold `grow_for_push` call relocates to (two merged
  identical functions). The `ContactPairs` writer (`pairs_build`, `push`, `clear`) and
  `body_bounding_radius` are inlined at both shas; nothing on the loop is out of line.
- **Placement in the measured exes.** The function's object bytes, relocations masked, are found
  exactly once in each measured exe (window 7 `bpbench_g4ref_93b2615b.exe` `f96a9c11…`, window
  8b `broadphase_g4ref_16191fda.exe` `b887850f…`), so each exe carries this code:

  | exe | fn RVA | mod 32 / 64 | loop head mod 32 / 64 | loop 32-B blocks / 64-B lines | jcc on or across 32 B |
  |---|---|---|---|---|---|
  | window 7 | `0x92da0` | 0 / 32 | 0 / 0 | 6 / 3 | none |
  | window 8b | `0xab080` | 0 / 0 | 0 / 32 | 6 / 4 | none |

  The 32-B layout, and so every jcc's 32-B position (JCC erratum), is the same in both. The loop
  sits in 3 cache lines in window 7's exe and 4 in window 8b's.
- **Placement is not a property of the lib commit.** Rebuilt without the one-line G4-sizes patch,
  `93b2615b` places the function at `0x92d80` (32 B from window 7's patched exe), and `16191fda`
  at `0xab070` (mod 32 = 16, with two jcc crossing a 32-B boundary). Any edit of the bench file or
  the physics crate can move this loop's alignment class.
- **The runner's AllPairs arm (J-A), shape only** (bench profile; the shipped runner is fat LTO):
  it is not byte-identical across the two trees. At `93b2615b` it is inline in
  `physics_broadphase` (inner loop 44 instructions, 201 B; body i's position reloaded every j);
  at `16191fda` it sits in the out-of-line `broadphase_arms` (L10's shared arms, three callers;
  inner loop 42 instructions, 184 B; body i's position hoisted, `n` reloaded from the stack every
  j). This is a J-A codegen change, not a G4 one; no timing here says which shape is faster.

## 4. Gates (the lane's evidence)

- **Red-first**, each recorded in its commit message. C4-1: the policy unit test; F3's G-TH1 scene
  blind to 128 (void red); the re-scened G-TH1's three reds. C4-2: the retargeted Auto tests on
  the old selector; G-AUTO1 against the deleted T6 guard, against Auto writing Grid, and the
  coupling pin against an Auto default. C4-3: section 5.
- **Codegen receipts** (bench profile, whole-object census of every function, `cg/cgcmp.py`):
  C4-1 → C4-2 differs in ONE function, `FunctionSystem<select_broadphase>`'s run (its constants
  2 999 / 2 701 → 135 / 129 and the kind 1 → 2); C4-2 → C4-3 differs in ONE function,
  `insert_physics_resources::<ColoredSoftStepSolver>` (the default kind 0 → 2 in two immediate
  stores). `physics_broadphase`, `physics_broadphase_colored(_sdf)`, `broadphase_arms`,
  `BroadphaseTree::run`, `step_hinted` and `all_pairs_into` are byte-identical across the lane.
- **Census.** `alloc_frame_census` (18 scenes) and `alloc_frame_attribution`, `--test-threads=1`,
  release and debug, on C4-2's and C4-3's trees: every steady row of every physics scene is
  identical, and all 18 gate rows are identical.
- **Pins.** Section 5.

## 5. C4-3: what the flip proves, and what it cost in coverage

- **The default, asserted.** `default_broadphase_is_the_tree` (`tests/broadphase_select_p3.rs`,
  no world, so it also runs under Miri) checks: `PhysicsConfig::default().broadphase` and
  `BroadphaseKind::default()` are `Tree`, `broadphase_select` is `Manual`, `parallel_broadphase`
  is off, and `QueryKernel::default()` is `LeafList`. Red on C4-2's tree: `left: AllPairs,
  right: Tree`.
- **The default-path pins see the Tree.** F3's order-leak mutation (the rev scatter of the
  assembly walks the Q rows in slot order) was run on C4-2's tree and on C4-3's.
  - **On C4-2's tree** `default_world_pyramid_determinism`, `sleep_settles_box_piles` and
    `bodytype_determinism_golden` stay green: the default ran AllPairs.
  - **On C4-3's tree** the pyramid is red. The 8-worker arms diverge from 1w after frame 3,
    because an unsorted stream is not W-invariant downstream (the parallel narrowphase's hint
    needs the strict order). The reference also left the pin (`0x0fc4db1199023f10` against
    `0xb583189fa681f3a6`), while the AllPairs arm stayed on it.
  - A7-R1 is red too: `D_max` `0x3a670260` against `0x3a2edc99`.
  - GOLDEN stays green: its scene has 7 rows, so the Tree's brute path runs there.
- **The census sees the Tree.** A per-step `Vec` at the top of `step_hinted` (every Tree step
  passes there):
  - on C4-2's tree it is red in S8-tree and S8b-tree only;
  - on C4-3's tree it is red in S1a, S1b, S1c, S1d and S1e too (256 `OTHER` over each window).
  - Design 04's census mutation is reachable for the first time.
- **Pins, unmoved** (release / debug):
  - pyramid `0xb583189fa681f3a6` / `0x839d94268d67b09f`, and reuse off `0xa38620b38cbca8d3` /
    `0xc7eb531b1e1ac19b`;
  - A7-R1 on `0x3a2edc99` (`D_max` 0.0006670445 m), off `0x3a3c3896`;
  - GOLDEN `0x1575326aeb803052`;
  - `bp_query_counts` 2/2 and `narrowphase_census` 1/1, in both profiles;
  - `boyko-physics --lib --tests`: 64 binaries in each profile, all green (release 730 passed
    once the omitted construction assert of G-L2-1 was fixed; debug 727 passed).
- **The parity runner** (the `parity` profile, fat LTO; `pins/runner_pins.tsv`, 40 of 40, at
  W 1/2/4/8/16):
  - **J, flagless.** The summary reads `broadphase Tree` and `tree_brute_max_rows 128`, with
    `0x30c5438bc6ad9ffa` at every W. The pose file is byte-equal to window 8b's `JT500.pose`,
    and `broadphase_tree` equals JT500's receipt at every W (1 static rebuild, 1 member, 77 500
    leaf-list leaves, 2 fallback leaves, `LeafList`).
  - **The other hashes:**

    | row | hash | pose file |
    |---|---|---|
    | `--contact-reuse off` | `0x32d5e235342b4143` | `JToff500` |
    | `--cfg a` | `0x30c5…` | `JA500` |
    | `--broadphase allpairs` / `grid` | `0x30c5…` | — |
    | `--scene rest` | `0x6cbe24bf8fafda26` | `RT500` |
    | R1100 | `0xc8bbe34cf6a8afc6` | — |
    | `--scene s16` flagless and with `--broadphase allpairs` | `0x4470add61a854f4c` | the two pose files byte-equal |

  - **Armed** flagless J at W1/W8 and s16 at W1: `void steps 0`.
- **The L10 pose gates**, run as copies with the fixture paths re-pointed at this worktree:
  `pose_gates_sync` 264/264 with both negative controls exiting 4 (its G2 J-D rows now run the
  Tree default against the AllPairs-era fixtures and match), `runner_checks_c3c` 26/26,
  `runner_checks_off` 17/17.
- **Coverage kept** (the flip would otherwise have removed it):
  - **The pyramid.** `default_world_pyramid_determinism` gains an AllPairs run that must equal
    the reference on every frame and end in the same pinned hash (the in-cargo G3).
  - **The sleep-skip rigs.** `sleep_skip_bit_identity`'s S4 variants (the SDF pile, DA1, DA2,
    LB1, LB2 and the warm arm) gain an explicit AllPairs arm beside "default", which now runs
    the Tree's brute path, and "Tree".
  - **The census.** S8 names AllPairs, so the census keeps the verbatim arm.
- **Two gates that stopped perturbing `broadphase`** (the critique's W1), repaired and shown red:
  - the step record's round trip now derives its perturbation from the default, and its
    anti-vacuity guard checks every field;
  - DA9's broadphase row writes a kind other than the one it lands on, and a per-row guard fails
    any write that leaves the configuration unchanged.
- **J-D ≡ J-T.** After the flip `--cfg default` and `--cfg default --broadphase tree` are one
  configuration, so the same-binary row of record is `--broadphase allpairs` against the new
  default (section 6).

## 6. Window 9: the commands (run by the orchestrator in the quiet window)

Protocol:
- Ruling 1 of the 2026-09-27 rulings: K = 9 per cell (3 passes × 3 rounds; p0 reversed, p1
  forward, p2 reversed). A claim needs IQR AND SE in every clean block and pooled; min-max is
  reported beside it, STRONG when it also passes.
- Ruling 8 of 2026-09-29: a dropped slot is re-run until the pass-cell has K = 3 or the pass
  ends; a pass-cell with K < 3 does not gate.
- Window 8b's idle rule, receipts, placement receipt, hot re-run, cutoff and STOP file.
- The machine form is the lane's `window9_c4_rows.json`, in window 8b's driver schema.

Binaries (sha256 recorded; no RUSTFLAGS; built from `git archive` exports, never a worktree):
- **TIP** is the lane tip and **PARENT** is `git merge-base u/phys-tree-c4 integ/unified`, each
  built with `cargo bench --no-run --locked --profile parity -p boyko-physics --bench
  jolt_parity_pyramid`.
  - If PARENT is `16191fda` under rustc 1.98.1 `48a229cea`, window 8b's
    `runner_tip_16191fda.exe` may be reused after its sha256 is re-checked.
- **g4rT** is TIP with window 8b's `g4ref.patch` applied by content (the `G4_SIZES` line; this
  lane shifted it by one line), built with `cargo bench --no-run --locked -p boyko-physics
  --bench broadphase` (the bench profile).
  - At prep, M9's placement receipt runs on it:
    `place.py apinto_16191fda.dis e0 192 <g4rT.exe>`. It must report one hit, and the alignment
    class is recorded beside the C4-BR rows.
- **Reused by sha256:**
  - g4r7 is window 7's `bpbench_g4ref_93b2615b.exe` (`f96a9c11…`);
  - g4r8b is window 8b's `broadphase_g4ref_16191fda.exe` (`b887850f…`);
  - j56 is Jolt 5.6 (`918fd2b7…`).

Untimed pre-flight, each at W1 over 500 steps with `--pose-out`:
- **TIP, flagless:** `broadphase Tree`, `tree_brute_max_rows 128`, `0x30c5438bc6ad9ffa`, and the
  pose equals `JT500.pose`.
- **PARENT, flagless:** `broadphase AllPairs` with the same hash.
- **TIP with `--broadphase allpairs`** equals PARENT flagless byte for byte.
- **`--arm-profiler`:** `void steps 0`.
- **Every criterion exe:** `--list` against its regex gives exactly the expected ids.

Blocks, in this order (ruling 5: the all-pairs slowdown against window 7 first):

1. **C4-BR, the all_pairs bracket.**
   - Criterion on g4r7, g4r8b and g4rT, adjacent in every round, with `CRITERION_HOME` per
     process: `^bp_g4_(uniform|disparity)/(all_pairs|tree)/(144|256)$` (8 ids).
   - K = 9 is 27 processes (about 47 min); K = 3 is 9 (about 16 min). Q5 decides which.
   - Read all_pairs(g4r8b)/all_pairs(g4r7) in the same window:
     - claimed above 1: a binary effect, and M9's only structural candidate is the 64-B split;
     - not claimed: a window term, and 128/136 must not be carried as measured.
   - `tree` is the in-binary control.
2. **C4-G4, the threshold re-read.**
   - g4rT: `^bp_g4_(uniform|disparity)/(all_pairs|tree)/(96|104|112|120|128|136|144|152|160)$`
     (36 ids, about 7.8 min per process, K = 9 about 70 min).
   - Plus `^bp_g4_(uniform|disparity)/tree_kd/(96|112|128)$` at K = 3 for F3's keep or freeze,
     not gating.
   - The rule (`treebp/g4_g5_recipe.md` under ruling 1), per family:
     - LO is the largest n at which all_pairs is not claimed slower; HI is the smallest n at
       which tree is claimed faster.
     - `TREE_BRUTE_MAX_ROWS` = min LO, and `AUTO_TREE_LO/HI` = (min LO, max HI).
   - Edges:
     - LO below 96, LO at 160 or above, or no HI in the grid: report, keep 128/136 marked NOT
       RE-READ, and extend the grid next window.
     - Never set a constant from an edge.
   - The result lands as a one-constant commit, with G-TH1's reds re-shown.
3. **C4-AB, the flip** (runner, 500 steps; metric windows [0, 100) and [100, 500)):

   | row | binaries | args | W |
   |---|---|---|---|
   | C4-JD | TIP | `--scene jolt --gap 0.5 --cfg default --sleeping off` | 1 2 4 8 16 |
   | C4-JDap | TIP | the same + `--broadphase allpairs` | 1 2 4 8 16 |
   | C4-JDpar | PARENT | `--scene jolt --gap 0.5 --cfg default --sleeping off` | 1 2 4 8 16 |
   | C4-JA | PARENT, TIP | `--scene jolt --gap 0.5 --cfg a` | 1 8 16 |
   | C4-JT | PARENT, TIP | `--cfg default --broadphase tree --sleeping off` on J | 1 8 16 |
   | C4-rung | TIP | C4-JD + `--canary-frac 1 --canary-ref-ns 60000` | 8 16 |

4. **C4-G5.**
   - Armed: C4-JD and C4-JDap with `--arm-profiler`, at W 1 and 8.
   - Unarmed:
     - `--scene rest --cfg default` against the same + `--broadphase allpairs`, at W 1 and 8;
     - `--scene s16` against `--broadphase allpairs`, at W1.
5. **C4-JOLT** (optional): `j56 -s=Pyramid -q=Discrete -f` at W 1/8/16.

Decisions:
- **Merge gate** (`jolt-gap/PLAN.md` M7: "C4 merges if no J-D W is slower and G5 is recorded"):
  C4-JD is not claimed slower than C4-JDap (same binary), nor than C4-JDpar, at any W; every
  hash equals its row's; `void steps 0`.
- **G5** (recipe `:202-212`):
  - the four `phys_bp_*` spans on C4-JD-armed, median over [100, 500): ≤ 0.36 ms at W1 and
    ≤ 0.35 ms at W8;
  - Δbp(8), the broadphase system span of C4-JDap-armed minus C4-JD-armed at W8: ≥ 1.03 ms;
  - s16 has no claimed regression, and rest is claimed faster.
  - Expected (arith., not a claim): Δ(W8) ≈ −1.70 ms (window 6).
- **Controls:** C4-JA and C4-JT, PARENT against TIP, are expected not claimed different. If
  either is claimed, the same-binary C4-JD against C4-JDap is the row of record.

## 7. Open for the orchestrator

- **The cut's questions.** The lane's cut asked nine questions (Q1–Q9). None was ruled before the
  developer stage, so the implementation took each recommendation:
  - **Q1 (lock extension):** `plugin.rs`, `systems.rs` doc lines, the three test files, the
    census's S8 and the step record's tests. No other open lane held any of them.
  - **Q3:** the AllPairs coverage of section 5.
  - **Q6:** `GRID_LO/HI` kept.
  - **Q8:** F3 untouched.
  - **Q9:** AllPairs on Auto's low side.
  - Each of these sits in its own commit and can be reverted.
- **Stop rules 6, 8 and 9** (`levers/00-RULINGS.md` "rules 6, 8 and 9 remain": maintenance at
  10k/100k static members) formally still stand against the flip.
  - The cut recommends a waiver with the reason written into the C4 ruling: those sizes are a
    regime no shipped scene or gated row reaches, and there AllPairs itself costs 70.2 ms per
    step at 10k (`04-DESIGN-REV2.md`).
  - Not ruled here (PC-C4-8).
- **`ADMIT_BUILD_RATIO`** is a G4 output in the design, but it stays at the design's starting
  1/4. It is not in this lane's spec.
- **PC lines** (not this lane's commits; the cut, section 6, lists all fourteen):
  - ~~window 8b's record path for section 2's figures, when u/win8b lands~~: resolved,
    `docs/measurements/2026-09-28-physics-window8b/` (PC-C4-1);
  - `SYSTEMS.md` and `FEATURE_MAP.md` ("opt-in until its commit C4", "`AllPairs` default");
  - the dated note under `OPEN-QUESTIONS.md`'s "AllPairs is the DEFAULT";
  - the mdBook physics page;
  - the rename of `auto_keeps_all_pairs_on_the_jolt_pyramid` in
    `L5-narrowphase/02-DESIGN-REV1.md`;
  - the benches that run `PhysicsConfig::default()` and now measure the Tree;
  - the runner's `--bp-kernel` refusal hint;
  - "J-D = AllPairs" in the window tooling;
  - the stale "(AllPairs)" comments in `soft_body_sp2.rs`.

## 8. Window 9a: what it read (2026-09-30, resumed 2026-10-01)

Section 6's blocks ran in window 9a, in two runs under one preparation. The numbers, the protocol
and the binaries are recorded in `docs/measurements/2026-09-30-physics-window9a/` (its `README.md`,
and the analyst's `analysis.md` with both parts); the rulings are in `levers/00-RULINGS.md`,
sections "After window 9a (2026-09-30)" and "After the window 9a resume (2026-10-01)". The tip
measured is this lane's `50e31f1a`.

- **M7 is met: C4 merges.** All 10 no-slower comparisons hold (C4-JD against C4-JDap, and against
  C4-JDpar, at W 1/2/4/8/16). In medians C4-JD is 1.58–1.68 ms below C4-JDap and 1.61–1.69 ms below
  C4-JDpar at every W, 93–100 % of the −1.70 ms of section 6, and no W is slower. By the letter "C4-JD claimed faster" is
  NOT CLAIMED, because C4-AB's pass 0 was contaminated; the merge rule asks only that it is not
  claimed slower. The controls (C4-JA and C4-JT, parent against tip) are not claimed different.
- **G5 is recorded, and every bar passes** (B1–B3 STRONG; B4 and B5 hold). B1: the four `phys_bp_*`
  spans on C4-JD-armed are 0.2502 ms at W1 against the 0.36 bar and 0.2608 ms at W8 against 0.35.
  B2: the realised Δbp(8) is +1.5779 ms against 1.03. B3: the Tree is claimed faster than AllPairs on
  the rest scene, −20.21 % at W1 and −39.61 % at W8. B4: s16 is not claimed slower (+1.59 %).
- **G4 re-read (C4-G4, resume run `033740`; g4rT, bench profile, K = 9, ruling 1): the thresholds
  are MEASURED at 128/136.** In both families all_pairs is STRONG faster than the tree at 96–120,
  nothing is claimed at 128 (so LO = 128), and the tree is STRONG faster at 136–160 (so HI = 136).
  `TREE_BRUTE_MAX_ROWS` 128 and `AUTO_TREE_LO/HI` 128/136 therefore equal the provisional values of
  section 2, which stand, and **window 7's 144/152 is REFUTED**. Ruling 7 of 2026-10-01 lifts the
  PROVISIONAL marks in the code by a doc-only commit; this record does not.
- **C4-BR (diagnostic, K = 3, nothing claimed): the all_pairs slowdown since window 7 is a binary
  placement term, about 1.12, and no fix lane opens.** all_pairs on g4r8b against g4r7 reads 1.1302
  / 1.1184 / 1.1200 / 1.1145 at the four ids (median 1.1192), separated at 4 of 4; the tree control
  moves by at most +2.7 %. This is consistent with section 3's verdict (b), not proven. The loop
  head of `all_pairs_into` sits at 0 mod 64 in g4r7 and at 32 mod 64 in g4rT and g4r8b. The revisit
  trigger of the cut's Q2 fires, and ruling 9 declines the bench-shipped reading: its stake is under
  1 µs per step.
- **F3 (`LeafListKd`) is FREEZE-AND-REMOVE** (ruling 8 of 2026-10-01): C4 never selects kd, below
  the threshold all_pairs beats kd by 25–47 %, and at 128 kd is faster than neither arm. The lane
  `c4-final` (branch `u/phys-c4-final`) freezes it under the annotated tag `phys/f3-kd-frozen` (on
  trunk `82b3867f`, which holds kd whole) and removes it. The return condition is a kd build form that claims t_qb lower at J
  W1 and is not slower at W8.
- **Not claimed, with the reasons in the window's analysis:** the Jolt standing (C4-AB's pass-0
  cells are short), the Rapier standing (cross-block, and the same short cells), and the cgu 1 twin
  (codegen-units 1 is not claimed faster at W8, so the profile stays at 16).
