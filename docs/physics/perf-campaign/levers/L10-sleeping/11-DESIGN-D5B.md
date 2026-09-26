# L10 D5b: warm start as a latched input (design rev 2)

**Status.** Rev 2 of the D5b note that D9b's Decision 6 owes (`10-DESIGN-D9B.md`). Rev 1 was critiqued
once (W1, O1–O3 and two open questions); rev 2 answers it, and the remark table is at the end.
Implemented as commit C3e of the L10 lane (`u/phys-l10b`), after C3d (D9b); the notes on how the
code realises it are in "Implementation (C3e)" at the end.

**Rev 2** answers critique r1: W1, O1–O3 and its open questions 1–2. After the note come the remark → resolution table and the rev 1 text that was removed or changed, quoted word for word.

**Tree read:** `D:/wt/merge` (branch `u/phys-l10`, before C3d). Line numbers below are the ones in that tree. After C3d (D9b) lands, the developer re-derives the lines in `systems.rs`, `sleep_sets.rs` and `colored.rs`.

**Binding inputs:**
- D9b, `10-DESIGN-D9B.md` (rev 3: rev 2 with the rev-3 patch applied). Decision 6 fixes the shape: warm start runs iff the solver's setup flag AND the latched `cfg.warm_start` are both true.
- D9b's round-2 critique, W3: "D5b moves no value" had not been established. §3 establishes it.
- The L10 folder (04 D5–D9, 08 E2′/E4′/E7′) and `plan.md`.

**Tools used:** no cargo, no git. There was no Agent tool, so the prior-art check was done directly on the web (Sources at the end).

---

## Goal

- Make warm start a per-step **input**: `PhysicsConfig::warm_start`, latched by D9b's `StepInputs` at the broadphase.
- Keep every setup-time choice exactly as it is today. The solver's constructor flag stays a setup value.
- Prove that `Off ≡ Sets` holds bit for bit across a toggle made while islands are held, both at a step boundary and inside a step's window.
- Prove that D5b moves **no value** in any existing pin. It moves none.
- State the knob's behaviour after a cold period for both ways of driving the solver:
  - the plugin pipeline (every step gathers the rows) Resets;
  - direct drive (never gathers the rows) resumes.
  - Both behaviours are pinned.

**Target cost:** under 2 ns per step, no heap allocation, no change to the access-set graph.

---

## 1. Inventory

### 1.1 Every read of the warm flag

| # | Site | What it gates | After C3e it reads |
|---|---|---|---|
| r1 | `solver/colored.rs:1639` `for_each_warm_seed`, stream lookups | `None` when off | `warm_effective` |
| r2 | `colored.rs:1663`, same function, kept records | same | `warm_effective` |
| r3 | `colored.rs:1676-1677` accessor `warm_start_enabled()`. Its only caller is `systems.rs:472`, which feeds `Prologue.warm`. That value is used for the mode presence test at `sleep_sets.rs:937-940` and becomes the `warm` byte of the epoch at `:975` → `:346` | D5 epoch | `S ∧ record.warm_start`, computed once in `broadphase_sets` |
| r4 | `colored.rs:1688` `drain_restore`, called at `systems.rs:525` | merges the D-H restore records into the read side | parameter `warm` from the broadphase |
| r5 | `colored.rs:1711` `capture_moved_in` | move-in capture, or EMPTY | `warm_effective` |
| r6 | `colored.rs:1845`, the destructure in `build_columns`, which feeds `:1860` (write-side resize), `:1875` (carried diagnostic), `:1894` (`frozen_points`), `:1909` (`plan_sources_restored` parameter → `warm_records.rs:636`), `:1943` (`recs_w`) and `:1971` (`translated`) | seeds, record shapes, stats | `warm_effective`, destructured |
| r7 | `colored.rs:3859` `store_and_swap` | store, swap, stamp | `warm_effective` |
| r8 | `colored.rs:4076` `counters.step` (`disabled_steps`, `#[cfg(test)]` at `:699-700`) | test-only diagnostic | `warm_effective` |
| r9 | `colored.rs:4109` warm-remap gate | cursor classification | `warm_effective` |
| r10 | `colored.rs:4337` E6′ logical carry | `carry_points` / `carry_hits` | `warm_effective` |
| r11 | `solver/soft_step.rs:347`, the destructure in `build_constraints`, which feeds `:364`, `:437`, `:478` | seeds, stats | parameter `warm` |
| r12 | `soft_step.rs:557` `store_and_swap` | store, stamp | parameter `warm` |
| r13 | `soft_step.rs:937` warm-remap gate (it runs **before** the early return at `:956`) | cursor classification | local `warm` |
| — | `warm_records.rs:583/592` (test-only `plan_sources`), `:622/636` (`plan_sources_restored`); `colored.rs:744-750` `SetupCounters::step` | already take the flag as a parameter | unchanged |

### 1.2 Writers and constructors

| # | Site | Value |
|---|---|---|
| w1 | `colored.rs:1546-1552` `Default` → `with_capacity` | `true` |
| w2 | `colored.rs:1558-1586` `with_capacity` (`:1573`) | `true` |
| w3 | `colored.rs:1590-1595` `with_warm_start(enabled)` | `enabled` |
| w4 | `src/solver/colored_tests.rs:5217`, a direct field write (in-crate G1 S-f, direct drive) | `step != 20` |
| w5 | `soft_step.rs:224-230` `Default` → `with_capacity` | `true` |
| w6 | `soft_step.rs:236-256` `with_capacity` (`:251`) | `true` |
| w7 | `soft_step.rs:265-270` `with_warm_start(enabled)` | `enabled` |

There is no other writer:
- both fields are private (`colored.rs:1520`, `soft_step.rs:213`) and there is no setter;
- the pipeline inserts `S::default()` (`plugin.rs:702`).

### 1.3 Every site that sets it, with the effective value today and after C3e

After C3e the effective value is `S ∧ cfg.warm_start`. `cfg.warm_start` is `true` in every one of these sites.

| Site | World | S | cfg | today → C3e |
|---|---|---|---|---|
| `tests/row_keyed_state_defect_a.rs:845-850` (`warm_start_scene`, via `warm_metrics(false)` at `:953-955`) | colored pipeline, sleeping off; solver replaced at setup, before any spawn or step | `false` | `true` | cold → cold |
| `tests/softstep.rs:1187` (`stack_sink_at_substeps`, called at `:1221` with false and `:1222` with true) | reference pipeline (`build_schedule::<SoftStepSolver>`, `:1179`); replaced before the first run | `warm` | `true` | unchanged |
| `tests/softstep.rs:1630` (`box_stack_sink_at_substeps`, `:1662` / `:1663`) | same | `warm` | `true` | unchanged |
| `src/solver/colored_tests.rs:5217` (S-f) | direct drive, never gathers; the G1 config is built on `Default` (`:4902-4907`) | toggled | `true` | unchanged: step 21 resumes under `Identity` (L3′), pinned by `PIN_S_F` |
| `tests/row_identity_remap.rs:504-508` `replace_solver` | pipeline; mid-run replacement with `Default` (outside the contract, D9b §1.5) | `true` | `true` | warm → warm |
| `plugin.rs:702`, and every direct-drive `::default()` / `with_capacity` in tests and benches | — | `true` | `true` | warm → warm |
| Scenes (`boyko_app/examples/playground.rs`, `_hud_probe.rs`), `boyko_demo`, `src/main.rs`, the `jolt_parity_pyramid` runner | — | — | — | none sets it (grep over the whole tree) |

---

## 2. The rule, its read sites, and exactness

### 2.1 Rule

`w(t) = S ∧ R_t.warm_start`, where:
- `S` is the solver resource's `warm_start_enabled`. It is a setup value: the pipeline never writes it, and replacing the solver after setup is outside the contract (D9b §1.5).
- `R_t` is the `StepInputs` value latched at the first statement of step t's broadphase (D9b Decision 1).
- In direct drive, `R_t` is the caller's `config`.

### 2.2 Read sites after C3e

| Where | Code shape | Value |
|---|---|---|
| `broadphase_sets` (`systems.rs:450-527`) | `let warm = solver.as_deref().map(\|s\| s.warm_start_enabled() && inputs.config().warm_start);` is computed once. It feeds `Prologue.warm` (epoch and mode presence) and `drain_restore(restore, w)` | `Some(w(t))`, or `None` without the colored solver (presence semantics unchanged) |
| `solve_colored_inner` (`colored.rs:4044`) | directly after the early return and `solved_steps += 1` (`:4072-4075`): `self.warm_effective = self.warm_start_enabled && config.warm_start;`. Every gate r5–r10 in the step reads `warm_effective`. `config` is `inputs.config()` (D9b code site 7) | `w(t)` |
| `SoftStepSolver::solve` (`soft_step.rs:908`) | first statement: `let warm = self.warm_start_enabled && config.warm_start;`. It is passed to `build_constraints(.., warm)` and `store_and_swap(rows, warm)` and gates the remap. It must come first because the build precedes the early return. `config` is `inputs.config()` (D9b code site 8) | `w(t)` |
| `for_each_warm_seed` (`colored.rs:1630`) | gate is `self.warm_effective` | `w` of the last solve that ran past the early return |

### 2.3 Exactness

**L1: one value per step.** Every read in step t evaluates to `w(t)`:
- `R_t` is latched before any reader (D9b);
- `S` is constant after setup;
- `warm_effective` is written once per solve, before its first reader;
- no stage after the broadphase declares `Res<PhysicsConfig>` (G-ACCESS).

**L2: a cold step does not touch the warm state.** If `w(t) = 0`, in both modes and in both drives:
- every plan run is `MISS` (`warm_records.rs:636-638`), so every seed is 0;
- there is no write-side resize and no `recs_w` (`colored.rs:1860`, `:1943`);
- the capture fills EMPTY records and adds 0 to `held_warm` (`:1711-1713`);
- the store returns before writing, swapping or stamping (`:3859-3861`; reference `soft_step.rs:557-559`);
- no remap runs (`colored.rs:4109-4113`; `soft_step.rs:937-941`);
- no E6′ addition (`colored.rs:4336`);
- no drain (`:1688`).

So the read side, the warm cursor stamp σ and `warm_cur` are unchanged by step t. The only change to Sets' kept state is that move-ins take EMPTY records.

**L3: in a pipeline that gathers every step, the first warm step after a cold one is a Reset.**
- **Scope:** a solver whose `RowIdentity` opens one gather per step. That covers every pipeline the plugin builds: `physics_gather` opens the gather at `systems.rs:270` and is registered for every pipeline at `plugin.rs:777` (`plugin.rs:964`: "every pipeline this module builds"). Both the colored and the reference solver are covered.
- Let t be a cold step that ran past the early return, and u the next warm step that runs past it.
- No solve stamps in [t, u): by L2, and because an early return never reaches the store. So σ ≤ seq(t−1) and seq(u) ≥ seq(t−1) + 2. A σ of 0 (never stamped) reads as `base` (`row_identity.rs:596-600`), which obeys the same bound.
- `classify` therefore returns Reset (`row_identity.rs:601-612`):
  - for the colored cursor (`colored.rs:4110`);
  - for the reference cursor, which goes through the same `classify` (`soft_step.rs:938`) and is stamped only by its store (`:577`);
  - for the colored capture's peek (`colored.rs:4329`), which comes before u's stamp (`:3923`).
- On Reset every lookup misses, including the restore source (`warm_records.rs:644`, `:666-691`; `RowRemap::pair` → `None` at `row_identity.rs:222`).
- Step u writes every key (`warm_records.rs:661-665`) and every record index of its write side (fill `colored.rs:1942-1964`, store `:3897-3914`) and swaps it in.
- So nothing stored before t is read at or after u. The one side that differs between the modes — Off's side from t−1, which holds frozen carries that Sets' side lacks — becomes u+1's write side and is overwritten before anything reads it.
- **Witnesses:** arm A (d), arm B's Reset checks, and arm D's sixth world (reference solver). No pin that exists today witnesses L3, because G1 drives the solver directly (L3′).

**L3′: in direct drive, the solver resumes.**
- **Mechanism:**
  - A caller that never opens a gather keeps `gather_seq == base`, so `classify` returns `Identity` for every cursor (`row_identity.rs:590-594`: "direct drive, whose rows are whatever the caller put in the snapshot").
  - The cold step stores and swaps nothing (L2), so the read side is still the one the last warm solve wrote. Step u seeds from it: every pair still in the stream hits, and `remap_resets` does not move.
- **Callers covered:** every `pub` entry driven without a gather:
  - `solve_colored` (`colored.rs:3985`);
  - `solve_colored_sleeping` (`:4011`);
  - `SoftStepSolver`'s `RigidSolver::solve` (`soft_step.rs:908`).
- **Pinned today for the setup knob:**
  - G1's `step_pipeline` (`colored_tests.rs:4964-5009`) never gathers. The only `begin_gather` in that file is `Churn::gather` (`:5077`), which S-c uses.
  - So S-f's step 21 is `Identity` and seeds from step 19's store, and `PIN_S_F` pins that resume.
- **Pinned for the new knob:** arm E.
- **Why L3′ does not touch the `Off ≡ Sets` proof:** `SleepSkip::Sets` is reachable only through `solve_colored_held` (`pub(crate)`, `colored.rs:4028`), which only the pipeline's solve system calls. L3 holds there.
- A hand-gathering direct caller (S-c's pattern) gets `classify`'s rules as written: a Reset iff it gathered at least twice since the last warm store.

**L4: flush steps admit no move-in.** Every flush restores all live records (`sleep_sets.rs:1013-1015`, `:1019-1029`) and runs with `resting_ok = !flush` (`:1009`), which `move_in` requires (`:1125`).

**Re-hold during a cold period (answers the critique's open question 1).** Move-in admission reads no warm datum:

| Stage | Where | Reads |
|---|---|---|
| `classify` | `sleep_sets.rs:1152-1220` | the latches, the baseline bits and inverse mass, the awake mask, the jumper bits, the frozen flag, the graph |
| M1 | `:1487-1502` | the admissible counts |
| D2 scan | `:1257` | the stream |
| `capture_island` | `:1720-1809` | the previous stream, the pair tags, and the reuse records (L9's narrowphase carry, not warm records) |
| compaction | `held_store.rs:842-897` | moves `kept_rec`, decides nothing |

- The flag could reach a tower's latch or its bits only through the tower's velocities. O8 skips a frozen island's solve and integrate (`colored.rs:4000-4003`).
- So in both modes a frozen tower is a candidate again at the first non-flush step (t+1 after a toggle at t).
- Arm A (c) asserts the re-hold. If this premise fails, the arm goes red; it does not pass vacuously.

**T1: boundary toggle 1→0 at step t while islands are held (pipeline).**
- Sets records an epoch every step (mode Sets never takes the `:946` early return). So `epoch(t).warm = 0 ≠ 1`, and Sets flushes at t (`:975-991`, counted as `d5_epoch`).
- Step t is cold in both modes (L2). The restored islands take their pairs from `restore_idx` (the pair half of E7′). No seed is read from `restore_rec`, because the plan is all `MISS`.
- Obs(t) is equal field by field:
  - bodies, latches, views and islands, by E7′;
  - `warm`, by L2 (translated, hits and carries are 0 in both; the logical manifold and point counts are equal);
  - `seeds`: both worlds are gated by `warm_effective = 0`, so both report `(key, fid, None)` over equal logical point lists.
- During the cold period both worlds stay inert (L2). Sets moves the frozen islands in again at t+1 (see "Re-hold"), with EMPTY records. E4′ holds with every warm quantity at 0.

**T2: boundary toggle 0→1 at step t2 (pipeline).**
- Both modes Reset (L3): every seed is 0, every carry EMPTY, every capture EMPTY.
- Sets flushes whatever it holds again (the epoch changed 0→1). Those records are EMPTY (L2). This flush is sufficient, not necessary; it is kept rather than adding a special case.
- Each world's new read side is a function of four inputs: the stream, the tags, a plan that is all `MISS`, and the solved impulses. All four are equal between the modes (E6/E7′).
- From t2+1 on, the standard induction (04 E3–E6, 08 E2′/E4′/E7′) starts from equal states.
- **Consequence (O1):** every island asleep at t2 — frozen under Off, held under Sets, including islands that fell asleep during the cold period — carries an EMPTY record from t2 on, in both modes, and wakes cold whenever it wakes.

**T3: mid-step toggle in step t.** Under D9b nothing in step t reads the live value (L1), so step t runs with the old `w` in both modes. The write lands at t+1's latch. This is T1 or T2 with the boundary at t+1. So `Off ≡ Sets` holds at t (nothing changes) and after it.

**T4: a transient write** (Early write, Late revert) is never latched, so it is invisible.

**T5: `S = 0`.** Then `w ≡ 0`, the epoch is constant, and toggling `cfg` changes nothing.

**T6: the toggle coincides with another flush cause** (mode, D6, D7, SDF edit, τ, reuse, dt). There is still one flush, which is exact whatever its cause, and L2/L3 apply unchanged.

**Why the epoch must carry the AND.** Suppose the epoch keeps today's `systems.rs:472` (tree R0 in §4):
- There is no flush at t. The held islands keep the `kept_rec` captured while warm (`held_warm > 0`) through the whole cold period.
- At t2, Off carries its frozen records through the Reset, so they become EMPTY. Sets' held manifolds are not in the stream, so they are not re-carried.
- Result:
  - E6′ adds `held_warm > 0` to Sets' `carry_hits` against Off's 0;
  - `for_each_warm_seed` reports `Some` against `None`;
  - the first restore after t2 seeds the woken island with pre-t impulses in Sets and with zeros in Off.

### 2.4 Amendments (carried in this note; 04/06/08 are not edited)

- **04 D9, D5 row:** "`warm_start_enabled`" becomes "the effective warm start `S ∧ R_t.warm_start`".
- **08 E7′:** "seeds them from `restore_rec`" becomes "seeds them from `restore_rec` iff `w(t)`; on a cold step nothing is seeded, in both modes (L2)".
- **D9b §1.5, refinement only (the contract is unchanged):**
  - In the pipeline, where every step gathers, a solver replacement that **flips** the flag is accidentally exact at a boundary: the epoch sees the new flag, and L3 makes the old store unreachable.
  - The non-exact cases are a replacement that keeps the flag, and any replacement inside the window, which is not deferral-equivalent.
  - Replacement stays outside the contract.

---

## 3. Value neutrality: D5b moves no value

**Substitution lemma.**
- Wherever `R_t.warm_start == true`, every new expression equals the old one: `S ∧ true = S`.
- Every world in the tree has `warm_start == true` at every step:
  - The only construction is `Default` (`resources.rs:570-672`). The only literal that names `sdf_narrowphase` — which every exhaustive literal must do — is `Default` itself (`:617`). So every other `PhysicsConfig { .. }` literal has a `..` base derived from `Default`.
  - Nothing writes the field, because it is new.
- So every branch takes the same arm, and the schedule and access sets are unchanged.
- The one new piece of state, `warm_effective`, is initialised to `S` and set to `S ∧ true = S`. So the gate in `for_each_warm_seed` equals today's gate (`S`) at every point where it can be called.
- D5b-4 keeps direct drive on `Identity`, so G1 S-f's step 21 still resumes from step 19's store, and `PIN_S_F` does not move.

**Artefacts that are not values:**
- `size_of::<PhysicsConfig>` and of the record may grow by 0–8 B. The developer pins both with an LSP hover.
- `Debug` of `PhysicsConfig` gains one field. No pinned output prints the whole config (grep). The runner writes its config JSON field by field (`jolt_parity_pyramid.rs:1517-1545`) and does not get the new key (OQ1).

| Pin | Path | Argument |
|---|---|---|
| GOLDEN `bodytype_determinism_golden.rs:81` + `golden_scalar_colored_equals_golden` | `add_physics_colored_solve` (`:189`), sleeping off → `solve_colored` → `inner(None, None)` | `warm_effective = true` at r6–r10, the same as today's field |
| Pyramid `default_world_pyramid_determinism.rs:96` | `add_physics_systems::<DefaultRigidSolver>` (`:254`), sleeping off | same |
| A7-R1 `sleep_settles_box_piles.rs:331` (`0x3a3c_3896`, release) | colored harness, sleeping off (`:1723`) | same |
| G-pins in `sleep_settles_box_piles` | colored, sleeping on | the epoch's `warm` byte is 1, as today, so the flush set is unchanged |
| J/R runner poses (`docs/measurements/2026-09-23-l10-sleeping/fixtures/*.pose`, `SHA256SUMS`) | runner, `S = true`, never writes the field | same as above; JSON unchanged |
| Sleeping fixtures: `sleep_skip_bit_identity` (R-S, J-Son, J-D-on, S1–S5), `frozen_island_warm_start`, `sleeping_pipeline_o8`, `support_loss_wakes_sleepers`, `default_world_sleep_worker_invariance`, `sdf_sleep_settles_and_wakes` | colored, sleeping on | epoch byte unchanged; `Obs.seeds` gate `= S` as today |
| G1 `PIN_S_A`…`PIN_S_G` (`colored_tests.rs`) | direct drive, config built on `Default` | `warm_effective = S` at the same steps; S-f still `Identity` at step 21 (L3′); `disabled_steps == 1` |
| The §1.3 sites | — | the table's "today → C3e" column |

**Confirming runs** (the tester runs them; this note runs none):
1. `cargo test -p boyko_physics --test bodytype_determinism_golden`
2. `cargo test -p boyko_physics --test default_world_pyramid_determinism`
3. `cargo test --release -p boyko_physics --test sleep_settles_box_piles` (A7-R1 runs in release only)
4. `cargo test -p boyko_physics --lib solver::colored_tests` (the G1 pins, including S-f)
5. `cargo test -p boyko_physics --test row_keyed_state_defect_a --test softstep --test row_identity_remap`
6. `cargo test -p boyko_physics --test sleep_skip_bit_identity --test frozen_island_warm_start --test sleeping_pipeline_o8 --test support_loss_wakes_sleepers --test default_world_sleep_worker_invariance --test sdf_sleep_settles_and_wakes`
   - **6b.** `cargo test --release -p boyko_physics --test sleep_skip_bit_identity`. Needed because `s1_rest_pile_more_workers` is `#[cfg(not(debug_assertions))]` (`:1046`) and `matrix_steps` depends on the profile (`:1021-1023`). Without 6b those rows are inferred from the lemma, not confirmed.
7. The lane's G3 runner rows (R, R-S, R-on, J-Son, J-A-on, J-D-on × W1/W8, msvc release) against the fixtures.
8. `cargo test --workspace --all-targets --no-fail-fast`

These are hash comparisons, not timing, so no quiet window is needed.

**If any of these runs moves, stop.** A moved value contradicts the lemma above, so it points to an implementation defect, not to a re-pin.

---

## 4. Red-first tests and mutations

### 4.1 What "before D5b" can mean

The arms cannot compile on C3d, because `warm_start` does not exist there (the same situation as D9b's G-ACCESS). There are two faithful realisations. The tester records both as scratch patches, not commits:
- **R-pre = C3d + the field and its `Default` only.** Nothing reads the field. This is exactly the pre-D5b semantics: the input does not exist.
- **R0 = C3e with the epoch/drain line in `broadphase_sets` reverted to today's `solver.as_deref().map(ColoredSoftStepSolver::warm_start_enabled)`.** This is the input without D5b's exactness mechanism.

**Finding: the lockstep forms have no red on C3d itself.**
- The only vehicle for a toggle on C3d is replacing the solver. In the pipeline, where L3 holds, a replacement that flips the flag is exact (L2 + L3, and the epoch reads the new solver). So a lockstep arm driven that way cannot fail on C3d.
- Only the deferral form has a red on C3d: a replacement inside the window takes effect at the same step's solve, so M ≠ B at `at`. The tester records that one as an informative scratch run.

### 4.2 Changes to the rig (`tests/sleep_skip_bit_identity.rs`, on top of D9b's `MidWrites`)

- `Evidence.warm: Vec<WarmSeedStats>`: the Sets world's per-step stats. They equal Off's by the per-step compare.
- **Scenes:** `towers()` plus D9b's witness ball, rolling at 2 m/s and always awake. It is the only solved contact on held steps, so cold versus warm is observable through `points > 0 && point_hits == 0`.
- **Held check:** `held_rows == 9` (or 3 on S4). It replaces `assert_held_before_events`, because the ball is dynamic and never held.
- **Direct-drive helper for arm E:** `SolverScratch::with_capacity`, `set_bodies`, and a local `build_graph`, in the shape of `colored_rigid_scratch_determinism.rs:110-160`. Integration-test only. No new test binary, because arm E lives in this file.

### 4.3 Arms

**A. `s3_warm_start_toggle_while_held_flushes`** (boundary lockstep, pipeline)
- Runs on `arm_variants()`, plus the S4 scene (Sdf pipeline; default and Tree-brute-off × W1/W8). 390 steps.
- Script: `cfg.warm_start = false` at `HELD_BY`, `= true` at `HELD_BY+60`, `wake_all()` at `HELD_BY+120`.
- Checks:
  - (a) the towers are held at `HELD_BY−1`, and `warm[HELD_BY−1].carry_hits > 0`;
  - (b) `warm[HELD_BY].points > 0 && point_hits == 0`, and `stats[HELD_BY].flushes == 1`;
  - (c) the towers are held again by `HELD_BY+59`, and `carry_points == carry_hits == 0` at `HELD_BY+59`. The trace predicts `HELD_BY+1` ("Re-hold"). The tester records the first held step; the assertion keeps the margin because the D-rule interplay after a flush is not re-proved here;
  - (d) the Reset lands on the toggle-back step (witness of L3):
    - `stats[+60].flushes == 1`;
    - `remap_resets` is constant over [`HELD_BY−1`, `+59`] and rises by exactly 1 at `+60`;
    - `point_hits == 0` at `+60` and `> 0` at `+61`;
  - (e) `warm[+100].carry_points > 0 && carry_hits == 0`: the cold period erased the warm memory of the frozen islands in both modes (the O1 consequence of T2);
  - (f) `rules.d5_epoch ≥ 2` and `rules.d6_wake_all ≥ 1`.
- Expected results:
  - **R-pre:** lockstep green, but void at (b), because `point_hits > 0`.
  - **R0:** red at `HELD_BY+60`. The first differing field is `seeds`: Sets' pre-toggle `kept_rec` against Off's Reset-emptied carry (mechanism in §2.3). `warm.carry_hits` also differs.
  - **C3e:** green.

**B. `s3_mid_step_warm_start_toggle_lands_next_step`** (MidStep rig, lockstep, pipeline)
- Same scenes. The `MidEarly.cfg` writer sets `warm_start = false` at `HELD_BY` and `true` at `HELD_BY+60`. A second run uses `MidLate`.
- Checks:
  - `applied` increases by 1 at each write;
  - `warm[HELD_BY].point_hits > 0`: the write's own step ran warm;
  - `stats[HELD_BY].held_rows == 9`, `flushes == 0`;
  - at `+1`: `point_hits == 0` and `flushes == 1`;
  - `point_hits == 0` at `+60`; Reset (+1 `remap_resets`) at `+61`; `point_hits > 0` at `+62`.
- Expected results:
  - **R-pre:** void at `+1`.
  - **R0:** red at `HELD_BY+61`, `seeds`.
  - **N3** (the solve reads the live config): red at the "own step ran warm" check, at `HELD_BY`.
  - **C3e:** green.

**C. `d5b_defer_warm_start`** (D9b's deferral driver, M/B/C)
- Pipelines: `Default` × modes {off, Off, Sets} × {Early, Late}; `SoftCoupledRef` × off × {Early, Late}.
- Write `warm_start = false` at `at = HELD_BY`.
- Checks: M == B every step; `applied == 1`; C ≠ B within [`at`, `at+10`] (the ball's contact).
- DA9 also gains the field as class "observable" on every DA9 shape. Its destructure forces the classification at compile time.
- Expected results:
  - **R-pre:** void (C == B).
  - **N3:** M ≠ B at `at`, and G-ACCESS is red.
  - **C3d scratch** (the vehicle is a replacement inside the window): M ≠ B at `at`.
  - **C3e:** green.

**D. `d5b_two_knobs_one_rule`** (setup arm, pipeline)
- Two shapes: the `Default` pipeline with sleeping on and Sets; `SoftCoupledRef` with sleeping off.
- Six worlds per shape, as (S, cfg):
  - (t,t), (t,f), (f,t), (f,f);
  - (f, toggled f→t→f at `HELD_BY` and `+60`);
  - **(t, toggled t→f→t at `HELD_BY` and `+60`)**.
  - S = false is set by `insert_resource(with_warm_start(false))` before step 0, which is setup and inside the contract.
- Checks:
  - the four worlds with a false half, (t,f), (f,t), (f,f) and (f, toggled), are `observe()`-equal at every step, and their `rule_counts().d5_epoch` values are equal;
  - (t,t) ≠ (t,f) in bodies at some step;
  - the toggled worlds hold islands before each toggle (Sets shape);
  - **the sixth world** (reference witness of L3):
    - `point_hits == 0` over [`HELD_BY`, `+59`];
    - `remap_resets` is constant over [`HELD_BY−1`, `+59`] and rises by exactly 1 at `+60`;
    - `point_hits == 0` at `+60` and `> 0` at `+61`.
- This pins the attribution arm at `row_keyed_state_defect_a.rs:849` (S = false stays cold whatever `cfg` says) and closes W3. The sixth world gives the reference solver the Reset evidence that arm A gives the colored one.
- Expected results:
  - **R-pre:** red, (t,f) ≠ (f,f) at the first step with a contact.
  - **N6 / N7 / N5a–c:** red (see the table below).

**E. `d5b_direct_drive_resumes_after_a_cold_step`** (direct drive; pins L3′ and the rustdoc's direct-drive sentence)
- Two drives on one scene:
  - `ColoredSoftStepSolver::default()` through `solve_colored`;
  - `SoftStepSolver::default()` through `RigidSolver::solve`.
- Scene: a row of spheres resting on a static floor; manifolds rebuilt from positions each step and sorted; graph built per step. No gather. 30 steps, `cfg = PhysicsConfig { dt: 1/60, warm_start: step != 20, ..Default }`.
- Checks per drive, reading `warm_seed_stats()` and `solved_steps()` after each step:
  - step 19: `point_hits > 0` (premise: warm steady state);
  - step 20: `solved_steps` advanced (the cold step ran past the early return) and `point_hits == 0`;
  - step 21: `point_hits > 0` and `remap_resets == 0`, i.e. a resume, not a Reset.
- Expected results:
  - **R-pre:** red at step 20 (`point_hits > 0`; the knob is ignored).
  - **N2 / N5a:** red at step 20 on the matching drive.
  - **N10:** red at step 21.
  - **C3e:** green.

### 4.4 Mutations (each recorded red before the commit)

| # | Mutation on C3e | Expected red |
|---|---|---|
| N1 | the epoch's `warm` = S alone (this is R0) | A at `+60`, B at `+61` (`seeds`) |
| N2 | the colored solve uses S alone | A and B void; C void on colored; D colored shape; E colored at step 20 |
| N3 | the colored solve reads the live `Res<PhysicsConfig>` for `warm_start` | C (M≠B at `at`); B's "own step warm" check; G-ACCESS |
| N4 | `for_each_warm_seed` gates on S, not `warm_effective` | A at `HELD_BY` (`seeds`: Off's t−1 side holds frozen carries, Sets' does not); B at `+1` |
| N5a | reference build (r11) gated on S | D reference shape; C void on `SoftCoupledRef`; E reference at step 20 |
| N5b | reference store (r12) gated on S | D sixth world, reference shape: the cold-window stores stamp the cursor, so there is no Reset at `+60` |
| N5c | reference remap (r13) gated on S | D sixth world, reference shape: `remap_resets` rises inside the cold window |
| N6 | `w = cfg` alone (rev 2's override) | D: (f,t) ≠ (f,f) |
| N7 | the epoch's `warm` = cfg alone | D: `d5_epoch` of (f, toggled) ≠ that of (f,f) |
| N8 | E6′ gated on S | A: `warm` differs on the first cold step after a move-in |
| N9 | the colored warm remap gated on S | A (d): `remap_resets` rises during the cold period |
| N10 | a cold step Resets direct drive (clears the read side, or drops the cursor stamp) | E at step 21 (`point_hits == 0`); in-crate `PIN_S_F` |

**Equivalent mutants** (no arm can kill them; they are stated so no one hunts for a kill):
- **`drain_restore` gated on S.** The merged records can never be read (L3), and `warm_effective` is 0 for the diagnostic.
- **`capture_moved_in` gated on S.** A capture with a non-empty range on a cold step needs a step that is not a flush. The first cold step is always a flush (the epoch changed; if it follows a non-Sets step, D7/mode). On every later cold step the peek is stale, so it is Reset and the capture is EMPTY anyway.

**Per-site kill map (answers O2).** A missed swap still compiles, because the setup field keeps its name. What catches it:

| Site | A missed swap (the site still reads S) is caught by |
|---|---|
| r1, r2 | N4: A at `HELD_BY`, B at `+1` |
| r3 (the epoch line in `broadphase_sets`) | N1: A at `+60`, B at `+61` |
| r4, r5 | equivalent mutants (above) |
| r6 | A (b) void at `HELD_BY`; E colored at step 20 |
| r7 | A (d), because the cold-window stamp removes the `+60` Reset. In debug, also `debug_assert_eq!(recs_w.len(), manifolds.len())` at `colored.rs:3882` and the fill-shape assert at `:3887-3891`: the write side was not resized on the cold step |
| r8 | **no arm.** `disabled_steps` is `#[cfg(test)]` (`colored.rs:699-700`), so integration tests cannot read it, and S-f runs with `cfg = true`. It moves no value and does not exist in a shipping build. Left to the census below |
| r9 | N9: A (d) |
| r10 | N8: A |
| r11, r12, r13 | N5a, N5b, N5c: D (reference shape), E reference |

- **Census (for the code reviewer):** after C3e, `rg -n "warm_start_enabled" crates/boyko_physics/src/solver/{colored,soft_step}.rs` hits exactly these sites:
  - the two field declarations, `with_capacity` (×2), `with_warm_start` (×2);
  - the `pub(crate)` accessor (read by `broadphase_sets` for S);
  - the single `warm_effective` writer (colored) and the single `let warm` (reference);
  - the debug asserts (×3);
  - doc comments.

  Any other hit is a missed swap.
- **Rejected: renaming the setup field to force a compile error.** The rename would pull `colored_tests.rs:5217` into the lock. It also forces a *visit*, not a *correct* choice: a mechanical rename passes. The kill map is what makes a wrong choice observable, and it covers every site with an observable effect.

**Debug assert** (in `build_columns`, `store_and_swap` and `for_each_warm_seed`): `debug_assert!(self.warm_start_enabled || !self.warm_effective, "invariant: warm start runs only if the setup flag allows it")`.

---

## 5. Cost on the Off path

- **Per step:**
  - broadphase: +1 byte load from the record (already in L1, since the prologue reads the config) and one `&&`;
  - colored solve: +1 load (the record's config, already hot from `:4053-4054`), one `&&`, and a 1-byte store to `warm_effective`;
  - reference solve: +1 load and one `&&`;
  - D9b's record copy: +0–8 B.
  - **Total: under 2 ns**, below 0.0001 % of a J step.
- **Nothing structural:**
  - no loop gains a branch: the in-loop gates (`:1894`, `:437`) already exist, are loop-invariant, and only change their operand;
  - `build_columns` reads a destructured `bool` field as before;
  - no heap use, no new resource, no new system, no access-set change, so the wave composition is identical;
  - I-cache: under 20 B.
- **When a user toggles** (rare; Box2D documents its toggle as for testing):
  - pipeline: one D5 flush per toggle on a Sets world, and a cold step when warm start comes back on. Every island asleep at that step wakes cold later (T2);
  - direct drive: no flush and no Reset (L3′).
- **No timing gate.** A "not claimed slower" row can go into C1b's quiet window.

---

## Key decisions

**D5b-1: the colored solver stores the step's effective flag in a `warm_effective: bool` field. The reference solver passes a local.**
- **Why:**
  - `build_columns` has 7 direct unit-test callers (`colored_tests.rs:290`, `:453`, `:1153`, `:1570`, `:1597`, `:1846`, `:2455`) and `capture_moved_in` has 1 (`:5382`). With the field initialised to `S`, all eight are untouched, so `colored_tests.rs` stays out of the lock.
  - The same field is the truthful gate for the diagnostic.
  - The reference solver's two callees each have one caller, so a local is simpler there.
- **Rejected:**
  - a `warm: bool` parameter on the colored callees: 8 test sites of churn and no gain;
  - a new parameter on `for_each_warm_seed`: it would move a public signature onto the test side;
  - renaming the setup field (§4.4 kill map).
- **Trade-off:** 1 B of per-step state, written at exactly one site. A missed swap compiles and is caught by the kill map and the census, not by the type checker.

**D5b-2: the cold path skips the store and seeding (today's semantics, ANDed). Rejected: the scale-only shape of Box2D v3 and Jolt.**
- Box2D v3 always stores impulses and multiplies seeds by `warmStartScale ∈ {0, 1}`; Jolt also scales cached impulses by a ratio.
- **Rejected because:**
  - it contradicts binding Decision 6, where the epoch carries the AND;
  - it would edit the O7-verified fill, where a seed times 0.0 produces −0 for negative tangent impulses and would break the kernels' ±0-tie proofs;
  - it pays the full store on every cold step;
  - it would give "off" two different meanings (S versus cfg).
- **Trade-off, in the pipeline:**
  - the first warm step after a cold period seeds zero everywhere (L3);
  - every island asleep at that step (frozen under Off, held under Sets, including islands that fell asleep during the cold period) keeps an EMPTY record and wakes cold whenever it wakes, even minutes later (T2; arm A (e) asserts it);
  - one flush per toggle on a Sets world.
- **In direct drive:** a resume instead (D5b-4).

**D5b-3: `drain_restore` takes `w` from the broadphase.**
- `warm_effective` would still hold the previous step's value at drain time.
- The mutant is equivalent (§4.4), but one rule — "a cold step reads, writes and moves no warm record" — is kept literally, and the merge is skipped.

**D5b-4: direct drive keeps resuming. No Reset is added for a caller that never gathers.**
- **Why:**
  - A Reset in direct drive moves `PIN_S_F`: S-f's step-21 seeds would change from step 19's impulses to zeros. That contradicts §3.
  - It needs new state on the cold path (clear the read side, or poison the cursor stamp), and no proof obligation asks for it: Sets is pipeline-only (`colored.rs:4028`).
  - A direct-drive caller owns its rows. `Identity` is exactly the contract "the rows are whatever the caller put" (`row_identity.rs:590-591`), and a caller that keeps its rows stable gets back the impulses it had. Box2D v3 also resumes rather than resetting; it keeps storing while cold, so it resumes from the cold steps' impulses.
- **Rejected:** one uniform Reset for both drives.
- **Trade-off:** what the knob does after a cold period depends on how the solver is driven. The rustdoc states both behaviours. A, B and D pin the pipeline; E and `PIN_S_F` pin direct drive.

---

## Data structures and API

```rust
// resources.rs — PhysicsConfig (+1 B, hot for a latch only)
/// Warm-start contacts from the previous step's impulses (default `true`). A solve runs warm iff
/// this AND the solver's setup flag (`ColoredSoftStepSolver::with_warm_start`,
/// `SoftStepSolver::with_warm_start`) are both true. The pipeline reads it once per step, at the
/// broadphase; a write takes effect at the next broadphase.
///
/// In the plugin's pipelines, whose rows are gathered every step, the first warm step after a cold
/// one seeds every contact with zero, and every island asleep at that step (frozen, or held by the
/// sleep-skip) keeps no warm memory and wakes cold whenever it wakes. On a sleeping colored world a
/// change of the effective value also restores every held island (L10 D5). A direct-drive caller
/// that never gathers rows (`solve_colored`, `solve_colored_sleeping`, `RigidSolver::solve`)
/// resumes from the impulses its last warm solve stored.
pub warm_start: bool,

// solver/colored.rs — ColoredSoftStepSolver (+1 B, cold)
warm_start_enabled: bool, // SETUP half; never written by the pipeline
warm_effective: bool,     // S ∧ config.warm_start of the last solve past the early return; init = S
```

**Public API:**
- One new `pub` field.
- `drain_restore` (crate-private) takes `warm: bool`.
- `SoftStepSolver::{build_constraints, store_and_swap}` (private) take `warm: bool`.
- Every public signature is unchanged.
- These docs are updated to name the two knobs, the AND, and the pipeline/direct-drive difference: `with_warm_start` (both solvers), `for_each_warm_seed`, `SleepSkip::Sets` (`resources.rs:161-165`), `SleepEpoch.warm` / `Prologue.warm` (`sleep_sets.rs:283-297`, `:576-578`), `BroadphaseStages.solver` (`systems.rs:441-443`).
- A custom `RigidSolver` should honour `config.warm_start`. This is documented, not enforced.

**Multithreading:** unchanged.
- `warm_effective` is written by the solve, which holds the solver resource exclusively. The diagnostic reads it outside the schedule.
- The broadphase reads only `S`.
- No atomics; `Send`/`Sync` unchanged; no `unsafe`.

---

## 6. Code sites and the lane's lock

1. **`resources.rs`:** the field and its doc (after `contact_damping`, `:199`); `Default` gets `warm_start: true` (`:570-672`); `SleepSkip::Sets` doc (`:161-165`).
2. **`solver/colored.rs`:**
   - the field docs and `warm_effective` (`:1519-1520`);
   - initialisation at `:1573` and in `with_warm_start` (`:1588-1595`);
   - `for_each_warm_seed` gate (`:1639`, `:1663`);
   - accessor doc (`:1673-1678`);
   - `drain_restore(restore, warm)` (`:1687-1694`);
   - `capture_moved_in` (`:1711`);
   - the `build_columns` destructure (`:1845`);
   - `store_and_swap` (`:3859`, doc `:3847`);
   - `solve_colored_inner`: set the field after `:4075`, then `:4076`, `:4109`, `:4337`;
   - `solve_colored` / `solve_colored_sleeping` docs (`:3985`, `:4011`): the direct-drive resume;
   - the debug asserts.
3. **`solver/soft_step.rs`:**
   - local `warm` (`:908-942`);
   - `build_constraints(.., warm)` (`:325-351`, `:364`, `:437`, `:478`);
   - `store_and_swap(rows, warm)` (`:556-557`, `:1050`);
   - docs (`:209-213`, `:258-270`, `:321`).
4. **`systems.rs`** (`broadphase_sets`, after D9b): compute `warm` once; `Prologue.warm` (`:472`); drain (`:522-526`); doc `:441-443`.
5. **`sleep_sets.rs`:** docs only (`:283-297`, `:576-578`).
6. **`step_inputs.rs`** (D9b): the `latch` round-trip destructure gains `warm_start` (compile-forced).
7. **`tests/sleep_skip_bit_identity.rs`:** `Evidence.warm`; the direct-drive helper; arms A–E; the DA9 row.

**Joins the lock:** `crates/boyko_physics/src/solver/soft_step.rs` only. Every other file above is already in D9b's lock set.

**Stays untouched:** `colored_tests.rs` (thanks to D5b-1 and D5b-4), `warm_records.rs`, `row_keyed_state_defect_a.rs`, `softstep.rs`, the benches, the runner.

**Outside the lock, forced by gates:** re-derive the UG-10 anchors (`tests/internal_docs_anchors.rs`) for docs that cite shifted lines in `colored.rs`, `soft_step.rs`, `systems.rs` and `resources.rs`. Not affected: no new resource id (rk12/B2 census), no zones, no allocations, no `#[ignore]`.

**Order:** C3d → **C3e** → C1b. C3e needs the record for T3 and arm C. It is value-neutral (§3), so it needs no quiet window.

---

## Implementation plan

1. `resources.rs`: the field, `Default`, docs.
2. `colored.rs`: `warm_effective`, its single writer, the read swaps, the `drain` parameter, the asserts, docs (including the direct-drive sentence on both `pub` entries).
3. `soft_step.rs`: local `warm` and the parameters.
4. `systems.rs`: `warm` computed once in `broadphase_sets`.
5. `sleep_sets.rs` docs; the `step_inputs.rs` destructure.
6. Tests: the rig additions, arms A–E, the DA9 row.
7. The code reviewer runs the §4.4 census.
8. The tester:
   - records reds on R-pre and R0 (§4.1), plus the informative C3d scratch run for C;
   - records green on C3e;
   - records N1–N10 (N5 as a, b, c);
   - records the first re-held step of arm A (predicted `HELD_BY+1`).
9. The value-neutrality runs (§3), including 6b.

## Open questions

1. **Runner JSON:** add `"warm_start"` to `jolt_parity_pyramid`'s `config_json`? Not in C3e, because it changes the runner's output bytes and no row sets the field. The orchestrator decides.
2. **Box2D-style "store while cold"** (resume warm after a cold period in the pipeline): rejected (D5b-2). Revisit only if a gameplay need for runtime toggling appears.
3. **Uncommitted changes in `D:/wt/merge`:** still unverified; this is also the critique's open question 2. The orchestrator checks `git status` before the developer re-derives lines after C3d.

**Checklist:** all stated — goal, metrics, decisions and rejections, layout, API, multithreading, edge cases (S = 0, early return, transient, combined flushes, direct drive versus gathered pipeline), neutrality and runs (with release), red trees, mutations with a per-site kill map, lock and order. `Drop` and `unsafe` are N/A (plain data, none added).

Sources:
- [Box2D v3 contact_solver.c: `warmStartScale` in prepare; impulses stored unconditionally](https://raw.githubusercontent.com/erincatto/box2d/main/src/contact_solver.c)
- [Box2D: World API (`b2World_EnableWarmStarting`, "for testing")](https://box2d.org/documentation/group__world.html)
- [Jolt Physics: PhysicsSettings](https://jrouwe.github.io/JoltPhysics/struct_physics_settings.html)
- [Jolt Physics: Contact Constraint System (warm-start impulse ratio)](https://deepwiki.com/jrouwe/JoltPhysics/2.7-contact-constraint-system)

Files:
- `D:/wt/merge/crates/boyko_physics/src/{resources.rs, systems.rs, plugin.rs, sleep_sets.rs, held_store.rs, row_identity.rs, solver/colored.rs, solver/soft_step.rs, solver/warm_records.rs, solver/colored_tests.rs}`
- `D:/wt/merge/crates/boyko_physics/tests/{sleep_skip_bit_identity.rs, row_keyed_state_defect_a.rs, softstep.rs, row_identity_remap.rs, bodytype_determinism_golden.rs, default_world_pyramid_determinism.rs, sleep_settles_box_piles.rs, colored_rigid_scratch_determinism.rs}`
- `D:/wt/merge/docs/physics/perf-campaign/levers/L10-sleeping/{04-DESIGN-REV2.md, 08-DESIGN-REV2.3.md}`

---

## Remark → resolution

| Remark | Resolution | Where |
|---|---|---|
| **W1** (Important): L3 is pipeline-only; the `PIN_S_F` citation is false; the rustdoc overclaims | **Accepted.** Every claim re-checked: `classify` returns `Identity` when never gathered (`row_identity.rs:590-594`); `gather_seq: base` (`:470`); G1's `step_pipeline` never gathers (`colored_tests.rs:4964-5009`; the only `begin_gather` is `:5077`, used by S-c). Changes: <br>• L3 is scoped to pipelines that gather every step (`systems.rs:270`, `plugin.rs:777`) and covers both solvers. <br>• New L3′: direct drive resumes, and `PIN_S_F` pins that resume for the setup knob. <br>• The false citation is removed; L3's witnesses are named: A (d), B, and D's sixth world. <br>• The rustdoc and the D5b-2 trade-off are scoped. <br>• New decision D5b-4: direct drive keeps resuming, because a Reset would move `PIN_S_F`. <br>• New arm E pins the direct-drive sentence for the new knob; new mutation N10. | §2.3 L3/L3′, T1/T2 headers, §2.4, §4.1, §4.3 D/E, §4.4, §5, D5b-2, D5b-4, rustdoc, §6 item 2 |
| **O1**: trade-off understated | **Accepted.** T2 now derives the consequence; D5b-2, §5 and the rustdoc state that every island asleep at the resume step wakes cold whenever it wakes; arm A (e) is named as its witness. | §2.3 T2, D5b-2, §5, rustdoc |
| **O2**: "structurally excluded" is a convention | **Accepted.** Replaced by a per-site kill map plus a grep census for the reviewer. r8 is stated as unkillable and harmless (`#[cfg(test)]`, `colored.rs:699-700`). The rename route is rejected with reasons. **New finding while building the map:** rev 1 killed no missed swap at r12 or r13, because D's (t,f) world never reads the store, so both worlds match. Fixed by D's sixth world (t, toggled) and mutations N5b/N5c. | §4.3 D, §4.4, D5b-1 |
| **O3**: run 6 is debug-only | **Accepted.** Run 6b added (`--release`). | §3 |
| **Critique OQ1**: does move-in read warm data? | **Answered by trace.** `classify`, M1, the D2 scan and `capture_island` read no warm datum; compaction moves `kept_rec` but decides nothing; O8 skips a frozen island's solve and integrate. The re-hold is predicted at `HELD_BY+1`; A (c) asserts it, so a failed premise is red, not void. | §2.3 "Re-hold", §4.3 A (c) |
| **Critique OQ2**: uncommitted edits | Left to the orchestrator (merged into OQ3). | Open questions |

## Rev 1 text removed or changed (verbatim)

1. **§2.3 L3 heading, removed:** "**L3: the first warm step after a cold one is a Reset.**" Replaced by the scoped heading and body.
2. **§2.3 L3 last paragraph, removed:** "This is already pinned today in direct drive: the Reset after G1 S-f's cold step 20 is folded into `PIN_S_F` (`colored_tests.rs:5214-5221`, `:5311-5320`; the stats hash includes `remap_resets`)." Replaced by the witness list and by L3′.
3. **Rustdoc, removed:** "Read once per step by the broadphase; a write takes effect at the next broadphase. The first warm step after a cold one starts every contact cold; on a sleeping colored world a change of the effective value restores every held island (L10 D5)." Replaced by the scoped two-paragraph doc.
4. **D5b-2, removed:** "**Trade-off:** the first warm step after a cold period is cold, plus one flush per toggle."
5. **§4.4, removed:** "**Structurally excluded:** the build and the store reading different flags. Both read the one field `warm_effective`." Replaced by the per-site kill map and the census.
6. **§5, removed:** "**When a user toggles** (rare; Box2D calls warm-start toggling a testing feature): one D5 flush per toggle, and one cold step when warm start comes back on."
7. **§4.1, changed:** "a replacement that flips the flag is exact (L2 + L3, and the epoch reads the new solver)" → prefixed with "In the pipeline, where L3 holds,".
8. **§2.4, changed:** "A solver replacement that **flips** the flag is accidentally exact at a boundary" → prefixed with "In the pipeline, where every step gathers,".
9. **§4.4 table, removed rows:**
   - "N2 | the colored solve uses S alone | A and B void; C void on colored; D colored";
   - "N5 | the reference solver uses S alone | D reference; C void on `SoftCoupledRef`".

   Replaced by N2 (+E) and N5a/N5b/N5c; N10 added.
10. **§4.3 D, removed:** "Five worlds, (S, cfg) = (t,t), (t,f), (f,t), (f,f), and (f, toggled f→t→f at `HELD_BY` and `+60`)." Now six worlds.
11. **§6 item 7, removed:** "`Evidence.warm`; arms A, B, C, D; the DA9 row." Now includes the direct-drive helper and arms A–E.
12. **Implementation plan, removed:** "6. Tests: the rig additions, arms A–D, the DA9 row." and "7. The tester records reds on R-pre and R0 (§4.1), plus the informative C3d scratch run for C, then green on C3e, then N1–N9." Replaced by steps 6–9.
13. **Checklist line, changed:** "edge cases (S = 0, early return, transient, combined flushes, direct drive)" → "…direct drive versus gathered pipeline"; "neutrality and runs" → "neutrality and runs (with release)"; "mutations" → "mutations with a per-site kill map".

**Sections to re-read, because their invariants carry these changes:**
- §2.3: L2, L3, L3′, T1, T2, and the new "Re-hold" paragraph;
- §3: the lemma's last bullet and run 6b;
- §4.3: arms A (c)/(e), D and E;
- §4.4 in full;
- D5b-2 and D5b-4;
- the rustdoc block.

---

## Implementation (C3e)

Implemented on `u/phys-l10b` after C3d (`cfa61a7e`, D9b). The line numbers above were read before
C3d; this section names the functions.

**Where each part landed:**
- `resources.rs`: `PhysicsConfig::warm_start`, with the rustdoc above, placed after
  `contact_damping`; `Default` gives it `true`. The `SleepSkip::Sets` doc names the effective warm
  start. `PhysicsConfig` stays 72 B, because the `bool` fits in padding, and `StepInputs` stays
  1 664 B (both measured).
- `solver/colored.rs`:
  - `warm_effective`, initialised to the setup flag by `with_capacity` and `with_warm_start`.
  - Its single writer, `solve_colored_inner`, runs directly after the early return and
    `solved_steps += 1`.
  - Gates r1, r2 (`for_each_warm_seed`), r5 (`capture_moved_in`), r6 (the `build_columns`
    destructure and its six uses), r7 (`store_and_swap`), r8 (`counters.step`), r9 (the warm remap)
    and r10 (E6′) read it.
  - `drain_restore(restore, warm)`.
  - The debug assert `warm_start_enabled || !warm_effective` sits in `build_columns`,
    `store_and_swap` and `for_each_warm_seed`.
  - The docs of `with_warm_start`, the accessor, `solve_colored` and `solve_colored_sleeping`.
- `solver/soft_step.rs`: `let warm = self.warm_start_enabled && config.warm_start` is the first
  statement of `solve`. `build_constraints(.., warm)` and `store_and_swap(rows, warm)` take it,
  and so does the remap gate. The docs of the field and `with_warm_start` are updated.
- `systems.rs`: `broadphase_sets` computes
  `warm = solver.map(|s| s.warm_start_enabled() && cfg.warm_start)` once, from the record's config.
  It feeds `Prologue.warm` (the epoch and the mode test) and `drain_restore`.
- `sleep_sets.rs`: the docs of `SleepEpoch.warm` and `Prologue.warm`.
- `step_inputs.rs`: the unit test's config literal and destructure gain `warm_start`.

**§4.4's census after C3e.** `rg -n warm_start_enabled` over `colored.rs` and `soft_step.rs`
hits exactly:
- the two field declarations;
- `with_capacity` twice and `with_warm_start` twice;
- the `pub(crate)` accessor;
- the one `warm_effective` writer and the reference's `let warm`;
- the three debug asserts;
- `build_columns`' destructure, which feeds its assert;
- doc comments.

`SetupCounters::step` also has a parameter of that name. It predates C3e and is unrelated. No
other hit exists.

**The arms, as run:**
- A and B run over `arm_variants()` on the towers and the ball (9 held rows), and on S4's tower
  and the ball over the S4 variants (3 held rows).
- The first re-held step is 151 in every cell, as predicted.
- C runs on the default pipeline in every mode, and on `SoftCoupledRef` with sleeping off (the
  soft scene), Early and Late.
- D runs its six worlds on the default pipeline (Sets) and on `SoftCoupledRef` (sleeping off), on
  the towers and the ball.
- **D deviates in one parameter:** `sleep_threshold = 1e-2` in all six worlds. A cold solve settles
  the towers more slowly, and with the default threshold the worlds that start cold never froze
  before the first toggle, so the check that the toggled worlds are held went void.
- E drives both solvers directly over six spheres resting on a static floor. The manifolds are
  rebuilt from the positions each step, in row order, and the graph is built per step.
- DA9 gains `warm_start` as observable on every shape.
- The informative "C3d scratch" run of arm C (a solver replaced inside the window) was not run.

**R-pre and R0** (scratch trees, not committed):
- R-pre (C3d + the field and its `Default` only): A void at (b), B red at the step after the write
  (the step still ran warm), C void, D red at step 0 (`(t,f)` seeds `Some` against `None`), E red
  at step 20, and DA9 `warm_start` "misclassified" on every shape.
- R0 = N1: A red at `HELD_BY + 60` and B red at `HELD_BY + 61`, both in `seeds` (Sets `Some`, Off
  `None`), as §2.3 derives.

**Mutations N1–N10.** Each was applied by content, run in debug, and restored with an md5 check.
All are red:

| # | red |
|---|---|
| N1 | A at +60 and B at +61, in `seeds` |
| N2 | A void (b), B at +1, C void, D colored, E colored at step 20 |
| N3 | G-ACCESS; B's "the write's own step ran cold"; C with M ≠ B at `at` |
| N4 | A at `HELD_BY` and B at +1, in `seeds` |
| N5a | D reference (the sixth world hit inside its cold period); E reference at step 20. C stayed green on `SoftCoupledRef`: the store still honours the configuration, so C differs from B one step later and is not void. The prediction "C void" did not hold, and D and E kill the mutant. |
| N5b | D reference: no `Reset` at +60 |
| N5c | D reference: `remap_resets` rises through the cold period |
| N6 | D, on the debug assert `warm_start_enabled \|\| !warm_effective`, which fires before the observation compare |
| N7 | D: world `(f, toggled)` differs from `(t,f)` at `HELD_BY`, in the tags (its epoch flushed) |
| N8 | A at `HELD_BY + 1`, in the logical carry (`warm`) |
| N9 | A (d): `remap_resets` rises through the cold period |
| N10 | E reference at step 21 (no resume) |
