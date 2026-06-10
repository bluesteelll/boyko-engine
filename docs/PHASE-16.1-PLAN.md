# Phase 16.1 — Tick-Aware Run Conditions (COMPLETION) — Architect Plan (R2-FINAL, LANDED)

> Status: **LANDED** as `579c738` — see [PHASE-16.1-RESULTS.md](PHASE-16.1-RESULTS.md)
> for gates (W1 bench PASS, full Miri-TB sweep, workspace suites green).

Branch `ecs`. Implementation-ready. Research: `docs/PHASE-16-RESULTS.md` §0-P4 (the original footgun record) +
the Phase 16.1 research summary. **The "Changed/Added condition silently always-true" footgun is ALREADY
FIXED on-branch** (`schedule.rs:218-250` frame-start `set_change_ticks` bump for systems + own + set
conditions; `tests/phase16_1_tick_conditions.rs` + `run_condition_ticks_advance_per_frame`
`schedule.rs:~1580`). This plan closes the two REMAINING correctness gaps.

## Gap #1 (semantic — Bevy divergence): conditions miss dormant changes
boyko advances EVERY condition's `last_run` unconditionally every frame (`schedule.rs:241-250`), so a
condition dormant for N frames (gated by a false set/state condition, or members blocked by
`pred_remaining`) sees only "changes since last frame" on resume — silently MISSING dormant changes.
Bevy advances `last_run` only inside `run_unsafe` (only on a frame the condition actually runs) →
resume observes ALL changes since last actual run (Cheat Book: "you do not have to worry about missing
changes if your system only runs sometimes").

**Decision: adopt Bevy "since-last-actual-run" parity.** Delete the frame-start condition bump
(`schedule.rs:241-250`); move the checkpoint INTO the eval site (`run_condition`, `ecs_master.rs:~1971`):
```
run_condition(&mut self, condition, this_run: Tick) -> bool {
    condition.initialize(self);                         // FS1 no-op (unchanged)
    let prev = condition.meta().this_run();             // frozen across skipped frames
    condition.set_change_ticks(prev, this_run);         // advance ONLY when evaluated
    /* cell mint + run_unsafe unchanged */
}
```
`last_run`/`this_run` already live in each `BoolSystem`'s `SystemMeta` (no new storage). Thread frame
`this_run` via a new `Schedule.frame_this_run: Tick` field (appended LAST — M3 layout invariant), set at
`schedule.rs:~194`, read by `evaluate_ready_conditions`/`set_gate`. (Can't use `world.current_tick()` at
eval time — it's `this_run+1` after the #56 apply-window bump.)
- "ran" ≡ "was evaluated" (boyko's eager fold has no short-circuit — `should_run &= r`).
- Set conditions: checkpoint rides `run_condition` (the only place a set body runs, via `set_gate`) →
  fires once/frame when the set is reached, zero when no member becomes ready (correct dormancy).
- Behavior-preserving for the every-frame case (proof: for a condition evaluated every frame, `prev` =
  last frame's `this_run` = the value the old frame-start loop used → identical window). The existing
  `phase16_1_tick_conditions.rs` tests MUST still pass.

## Gap #2 (wraparound — latent correctness hole, bug-gate): scan omits system/condition last_run
`run_check_ticks_scan` (`check_ticks.rs:62-115`) clamps ONLY per-row pool ticks — no system's, no
condition's `last_run`. Bevy's `Schedule::check_change_ticks` clamps `systems` + `system_conditions` +
`set_conditions`. A `last_run` un-refreshed > `MAX_CHANGE_AGE` ticks flips `Tick::is_newer_than`. Currently
MASKED by the unconditional frame-start bumps (everything refreshed every frame); becomes REACHABLE once
Gap #1 makes condition `last_run` advance only-when-run.

**Design:** add `System::check_change_tick(&mut self, current: Tick)` (NO default body — a forgotten impl
must not compile, mirroring `set_change_ticks`); body = `self.meta.last_run = last_run.check_tick(current);
this_run = this_run.check_tick(current)` (`Tick::check_tick` exists, `tick.rs:156`). Add
`#[cold] Schedule::check_change_ticks(current)` over `systems` + `system_conditions` + `set_conditions`;
call it right after `run_check_ticks_scan` inside the existing `should_run_check_ticks` block
(`schedule.rs:~200-203`), using `current = this_run` (shared gate + `set_last_check_tick` → no drift).
Include regular systems (same cheap cold loop, same pre-existing hole).

## 0%-gate
Deleting `:241-250` REMOVES two empty-Vec loops (strictly faster for no-condition schedules). Eval-site
checkpoint is inside `evaluate_ready_conditions`/`set_gate` — reached only when `!has_condition.is_clear()`
(`schedule.rs:~464`). `frame_this_run` = one `Tick` store off the dispatch loop, field appended last
(offsets of the hot prefix unchanged). `check_change_ticks` only on the cold `CHECK_TICK_THRESHOLD` path.
`try_dispatch_ready`/`SystemBox` byte-identical. NCD note: the 2 `Tick` writes per evaluated condition
can't const-fold through `dyn System<Out=bool>` but match Bevy exactly (its `run_unsafe` always sets
`last_run`), into a `SystemMeta` hot in L1 — negligible, paid only by evaluated conditions in already-off-
gate schedules.

## SAFETY
Expected ZERO new `unsafe` (safe `meta()`/`set_change_ticks`/`check_change_tick` `&mut`; `frame_this_run`
plain field; `check_change_ticks` runs before `pool.install` ⇒ trivially exclusive).

## Integration
- `system/system.rs`: add `check_change_tick` to the trait (no default body).
- ~78 `impl System` sites (everything that impls `set_change_ticks` — `grep "fn set_change_ticks"`): add
  the 2-line `check_change_tick`. Compiler-enforced completeness.
- `ecs_master.rs`: `run_condition` gains `this_run: Tick` + checkpoint; fix its 2 unit tests.
- `schedule.rs`: add `frame_this_run` field (init `Tick::ZERO` in `build`); set at `:194`; DELETE
  `:241-250`; add `check_change_ticks` + call it in the `should_run_check_ticks` block;
  `evaluate_ready_conditions`/`set_gate` snapshot `frame_this_run` and pass to `run_condition`.

## Tests
Unit: `check_change_tick` clamps a stale `last_run`; `Schedule::check_change_ticks` clamps system + own +
set condition; a SKIPPED condition does NOT advance on the skipped frame (regression net for the deleted
frame-start loop). Integration (new `phase16_1_dormant_conditions.rs`): a state/set-gated `Changed<T>`
condition resumes and SEES dormant changes (THE Gap #1 proof — fails under the old unconditional bump);
every-frame behavior unchanged (re-run the `phase16_1_tick_conditions.rs` pattern); wraparound clamp for a
dormant condition AND a dormant system (Gap #2). Property: random dormant spans ≤
`CHECK_TICK_THRESHOLD + MAX_CHANGE_AGE`, a clamped `last_run` never false-positives. Bench: Phase 16
no-condition 50-systems = no change (0%-gate).

## Open questions for the critic
- **OQ-PRIME (orchestrator-added, HEADLINE): does Gap #1 ALSO apply to skipped SYSTEM BODIES?** The plan
  KEEPS the frame-start SYSTEM bump (`:218-221`) as "harmless Phase 10 contract." But by the identical
  logic as Gap #1, a `run_if`-skipped system whose `last_run` is bumped every frame would, on resume, MISS
  dormant changes via its OWN `Changed<T>` queries. Bevy's "no missed changes for sometimes-running
  systems" guarantee is PRIMARILY about system bodies (`last_run` set inside the system's own `run_unsafe`,
  only when it runs). So the unconditional system bump appears to have the SAME divergence — and for the
  MORE common case (users put `Changed<T>` in system bodies, not conditions). Critic: is the architect's
  "harmless" claim correct, or must Gap #1 cover skipped system bodies too (advance system `last_run` only
  when the system actually runs)? If the latter, this is a larger change to the Phase 10 tick contract with
  broader test implications — scope it (16.1 vs 16.2) and design it.
- **OQ-1 (architect's crux): eager-fold ⨯ set-gate.** boyko evaluates ALL of a ready system's own
  conditions every frame (no short-circuit), so an own `Changed<T>` condition after a false sibling runs
  every frame (empty windows, no dormancy benefit) — dormancy applies only via set/state conditions +
  `pred_remaining`-blocked members. Architect recommends keeping eager-no-short-circuit + documenting +
  testing dormancy via set/state conditions. Confirm or escalate short-circuiting to 16.2.
- **OQ-2:** include regular systems' `last_run` in the Gap #2 clamp (recommend YES — same hole, same loop)?
- **OQ-3:** `frame_this_run` field vs moving the #56 apply-window bump after `pool.install` (architect
  avoided touching the #56 contract). Confirm the field is acceptable.
- **OQ-4:** clamp `this_run` too in `check_change_tick` (architect: yes, matches Bevy `CheckChangeTicks`)?

---

# R2 (FINAL — folds critic CHANGES-REQUESTED; supersedes R1 where they differ)

Critic verdict on R1: **CHANGES-REQUESTED**. OQ-PRIME confirmed REAL + IN SCOPE — R1's "frame-start
SYSTEM bump is a harmless Phase 10 contract" was WRONG. R2 resolutions (all binding):

## C1 (CRITICAL) — skipped SYSTEM bodies miss dormant changes; fix in 16.1, OPTION (a)
A `run_if`/state/set-gated system skipped N frames has its ticks bumped every frame at `schedule.rs:218-221`
(before the skip decision), so on resume its `Changed<T>` body queries miss the dormant changes
(`FunctionSystem::run_unsafe` never re-stamps `last_run` — relies on the frame-start bump; Bevy stamps
inside `run_unsafe`, only when the system runs). **Fix = advance a system's ticks only on a frame it runs.**
**Option (a) (adopted, provably-0%):**
- Replace the unconditional frame-start loop (`:218-221`): bump ONLY systems with `has_condition[i]` CLEAR
  (they run every frame ⇒ "advance every frame ≡ advance when run", byte-identical plain-schedule path).
- Stamp GATED systems (`has_condition[i]` set) at the DISPATCH site, immediately before they run:
  concurrent path in `try_dispatch_ready`'s `to_spawn` loop BEFORE `scope.spawn` (`~:963-993`); inline-
  exclusive path BEFORE `run_unsafe` (`~:871-898`). Stamp = `let prev = sys.meta().this_run();
  sys.set_change_ticks(prev, self.frame_this_run);`.
- `mark_skipped` (`~:700-714`) stamps NOTHING (ticks stay frozen → resume sees the full dormant window).
- **Happens-before preserved:** a system's `SystemMeta` is read ONLY by its own run (Fetch copies
  last_run/this_run at set_table); the dispatch stamp is a dispatcher-sequential write before that system's
  own `scope.spawn` ⇒ same happens-before edge as the old frame-start loop. Inline-exclusive: same thread,
  program order.
- O1: first-eval-after-dormancy `prev` = `SystemMeta::new` sentinel (`current - MAX_CHANGE_AGE`) ⇒ correct
  "everything changed" first window (systems + conditions).

## W1 — 0%-gate proof
Conditionless 50-system bench MUST stay ≈0% (tester gate). Option (a): for `has_condition.is_clear()`
schedules the frame-start loop does identical work (the `contains(i)` branch uniformly not-taken; one
all-zero-bitset `contains` per system per frame, perfectly predicted) and the dispatch-stamp branch is
dead-not-taken ⇒ byte-identical hot path. Fallback if the per-system `contains` cost surfaces: wrap the
OLD unconditional loop in a single `if !has_condition.is_clear()` … else (old verbatim).

## W2 — frame_this_run single source + M3 doc
ONE `Schedule.frame_this_run: Tick` set once at `:194` (= frame-start `this_run`); read by condition
eval-site (Gap #1), system dispatch stamp (C1), state pass uses same local. NOT `world.current_tick()`
(it's `this_run+1` post-#56-bump). Append AFTER `state_entries` (a pointer-free scalar ⇒ hot prefix
offsets unchanged); UPDATE the M3 doc block `:128-138` so the "LAST field" invariant stays truthful. Init
`Tick::ZERO` in `ScheduleBuilder::build`.

## W3 — Gap #2: systems MANDATORY; is_apply_deferred moot; clamp BOTH
C1 makes system `last_run` dormancy-prone ⇒ `Schedule::check_change_ticks` MUST clamp `systems` +
`system_conditions` + `set_conditions`. `is_apply_deferred` skip OMITTED — grep-confirmed `insert_sync_points`
is an identity stub, NO synthetic ApplyDeferred system in `self.systems` (plain `for sb in &mut self.systems`).
`System::check_change_tick(&mut self, current: Tick)` — NO default body (compiler-enforced; mirrors
`set_change_ticks`); clamps BOTH `last_run` and `this_run` via `Tick::check_tick`. Real impl surface: 2
production (`FunctionSystem`, `ExclusiveFunctionSystem`) + ~5 `#[cfg(test)]` stubs (the "~78" was a grep
overcount). Call after `run_check_ticks_scan` in the `should_run_check_ticks` block (`:200-203`),
`current = this_run` (shared gate, no drift), before `pool.install` (exclusive `&mut`, no unsafe).

## W4 — discriminating tests (tester writes these): condition dormancy (Gap#1), **SYSTEM-body dormancy
(C1 — the key new one)**, every-frame behavior-preservation, wraparound clamp for dormant condition AND
dormant system, property over both surfaces. #56 coupling re-confirmed (a reached-this-frame gated system's
`this_run` == frame_this_run == `current_tick()-1`).

## OQ-R2-1 (residual — DEV/Miri to resolve): the C1 concurrent dispatch stamp takes `&mut self.systems[i]`
inside the `to_spawn` loop before the `systems_ptr = self.systems.as_mut_ptr()` raw lift (`~:961`). Architect
is confident the single-loop form is borrow-clean (stamp `&mut` released before the raw lift; sequenced).
**If the borrow checker OR Miri-TB objects, hoist the stamp to a pre-pass over `to_spawn` (stamp all gated
indices, THEN mint `systems_ptr`, THEN spawn)** — pure ordering refactor, happens-before + 0%-gate unchanged.

## OQ answers: OQ-1 keep eager-no-short-circuit (protects run_once/Local); OQ-2 systems included (mandatory);
OQ-3 frame_this_run field accepted (don't touch #56 bump); OQ-4 clamp this_run too. SAFETY: zero new unsafe.
