> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 14a — Results (Component Lifecycle Hooks)

**Status:** ✅ COMPLETE (uncommitted on branch `ecs`, ready to commit per-wave).
Functionally landed: full firing matrix verified, 0%-regression bench gate
passed, the hook plumbing is Tree-Borrows-clean under Miri. One pre-existing
(not 14a-introduced) Miri-TB finding (F4) documented + deferred.

## What shipped

Component types gain **four** innate lifecycle callbacks — `on_add`,
`on_insert`, `on_replace`, `on_remove` — fired synchronously at the exact
structural-op site, with **zero measurable cost when no hook is registered**.
(`on_despawn` was deliberately cut from 14a — see "Scope decisions".)

- **Gate:** a per-archetype `ArchetypeFlags(u16)` OR-computed once at archetype
  construction; the no-hook path is a single `u16` load + `test`/`jz` predicted
  not-taken. `#[cold] #[inline(never)]` `trigger_on_*` dispatch fns.
- **Hook context:** a read-only `DeferredEcsMaster` view (`get_component`,
  `resource`/`resource_mut`, `current_tick`, deferred `commands()`) — **no**
  `get_component_mut`, **no** `Deref` (the C2 aliasing hole is closed
  structurally, not by a documented obligation).
- **Reentrancy:** structural changes from a hook are deferred into a
  world-resident `deferred_hook_queue` and drained at the **outermost apply
  boundary**, gated by a **thread-local reentrancy depth counter** (single-owner
  drain). The drain reuses the proven single-`catch_unwind` +
  `handle_panic_recovery(0)` machinery via `CommandQueue::apply_via_raw_twin`.
- **Registration:** derive `#[component(on_add = f, on_insert = f, on_replace = f,
  on_remove = f)]` (any subset) **XOR** runtime
  `world.register_component_hooks::<C>().on_add(f)…`. A release-level staleness
  scan panics if a component is registered after it already appears in a live
  archetype.

## Pipeline (the iteration that got here)

| Stage | Outcome |
|---|---|
| Research | `docs/PHASE-14-RESEARCH.md` (Bevy `ComponentHooks` / flecs / Unity DOTS) |
| Architect R1 | `docs/PHASE-14-OBSERVERS-PLAN.md` |
| Critic R1 | **NEEDS REWORK** — C1 drain double-drive, C2 `get_component_mut` + live `&mut Archetype`, C3 vacuous bench gate, W1-W5 (`docs/PHASE-14-CRITIC-ROUND-1.md`) |
| Architect R2 | `docs/PHASE-14-OBSERVERS-PLAN-ROUND2.md` — single-owner drain + depth counter, read-only view, phased 6-site restructure, raw-twin drain, release staleness |
| Critic R2 | **REVISE** — confirmed C1/C2/C3 resolved; found NEW-1 (depth bracket leaks on early-`return Err`), NEW-2 (`on_insert` over-fires), W5-statement field-alias (`docs/PHASE-14-CRITIC-ROUND-2.md`) |
| R3 patches (§8) | P1 RAII guard, P2 `apply_via_raw_twin` sibling, P3 bundle-set firing, P4 bench, P5 tripwire |
| Critic R3 | P2 was over-corrected (phantom method); corrected to restore the catch-encapsulating sibling; P1 generalized to all 5 bracket sites → **APPROVED** (`docs/PHASE-14-CRITIC-ROUND-3.md`) |
| Dev Waves 1-3 | `ArchetypeFlags` + `HOOKS` table + deferred queue + view — code-review APPROVED |
| Dev Wave 4 | 5 `trigger_on_*` + wiring at all 6 sites — review CHANGES (on_despawn cut, 4 KB buffer hoist) → fixed |
| Dev Wave 5 | derive macro + runtime builder + staleness — review CHANGES (`overwrite_hooks` UB) → fixed (XOR design) |
| Tester (Wave 6) | full firing matrix + Miri + 0% bench gate; **found F1 + F2** |
| F1+F2 fix | TLS depth counter + drain self-bracket — Miri-verified |
| F4 investigation | **pre-existing** (baseline-Miri-confirmed), deferred (`docs/PHASE-14-F4-FINDING.md`) |

## Findings (the value the process produced)

Three rounds of architecture review + two rounds of code review verified the
design on paper. **Miri found two real soundness bugs that all paper review
missed** — the strongest argument for the tester-with-Miri stage:

- **F1 (HIGH, fixed) — drain re-entrancy double-apply.** `drain_deferred_hook_queue`
  did not raise the depth across its own walk, so a drained `DespawnCommand` →
  `delete_entity` → its end-of-body drain re-entered at depth 0 and re-applied
  the command (stale-entity assert + cursor unwind). Production-reachable on the
  schedule path. **Fix:** the drain brackets its own walk with a depth scope, so
  any command it applies that calls a self-draining direct-API method sees
  depth ≥ 1 and no-ops.
- **F2 (CRITICAL, fixed) — `DeferredScopeGuard` Tree-Borrows UB.** The guard
  cached a `NonNull<EcsMaster>` minted from `&mut *self`; the bracketed body's
  `self.…` reborrows froze that tag under Tree Borrows, so the guard's `Drop`
  write was UB — **unconditional** on `create_entity`/`create_entity_at`/
  `delete_entity`, even with no hooks. The "matches `CursorSync`" paper approval
  was wrong about the TB behavior (CursorSync doesn't reborrow the parent it was
  derived from mid-region). **Fix:** the depth counter moved to a **thread-local**
  (the `IN_SYSTEM_RUN` precedent) — the guard holds no pointer into `EcsMaster`,
  so no `&mut *self` reborrow can freeze it. Verified: `miri_phase14a` tests 1-3
  pass under `-Zmiri-tree-borrows`.
- **F4 (pre-existing, deferred) — remove-migration TB-UB.** A reborrow through an
  `as_mut_ptr()`-derived pointer stored in `EntityInland` conflicts (under TB)
  with the sibling `current_index += 1` write of a later spawn into the same
  archetype. **Confirmed pre-existing**: reproduces byte-identically at baseline
  `b223350` with a no-hook probe; `remove_command.rs` is unchanged from baseline.
  It is a property of the Phase-11 slab-pointer storage model (affects ~12
  reborrow sites), the same class as the Phase-9.1 deferred multi-thread-Miri
  item; native execution is correct. Deferred with a full finding +
  recommended fix in `docs/PHASE-14-F4-FINDING.md`; the one Miri test that trips
  it is `#[ignore]`'d. **Phase 14a does not introduce or worsen it.**

## Scope decisions

- **`on_despawn` cut from 14a.** The Round-1 plan defined 5 hook kinds but its
  firing matrix wired only 4; `on_despawn` would have been a registrable
  no-op (silently never fires) — worse than the staleness footgun the design
  escalated to a release panic. Bevy fires `on_despawn` distinctly at entity
  despawn with an ordering 14a's conservative scope shouldn't improvise. Removed
  from the public surface (bit 4 reserved, not renumbered); deferred to 14b.
- **Derive XOR runtime per type.** A component declares hooks via the derive
  attribute OR the runtime builder, never both (eager panic on conflict). The
  merge alternative would require an `UnsafeCell`-backed `HOOKS` table; not worth
  it for a rare case. The "register once before first use" contract is already
  enforced by the staleness scan.
- **`get_component_mut` deferred to 14b.** The hook view is read-only for
  components in 14a; mutable component access needs a proven non-aliasing story
  against the live apply-loop borrows.

## Measured results

- **Correctness (native):** 10 firing-matrix integration tests + 17 unit + 6
  deferred-reentrancy + 5 trybuild compile-fail + the F1 schedule repro — all
  pass. The matrix is fully pinned: spawn (add-then-insert ordering), in-place
  insert (replace-then-insert, no add), insert-migration (add over `I\S`,
  insert over the bundle set `I`, retained fires nothing), remove (replace+remove
  pre-drop reading the SOURCE dying row), despawn (replace+remove for all,
  pre-remove), no-hook (counter stays 0), mixed archetype, derive XOR runtime,
  staleness panic.
- **Soundness (Miri, `-Zmiri-tree-borrows`):** the hook dispatch / deferred-drain
  / pre-drop-remove paths are TB-clean (F2 fixed). F4 (pre-existing) ignored.
  Multi-thread Miri deferred per Phase 9.1 (hooks fire only in the
  single-threaded apply window — SAFETY-4).
- **Performance — the 0%-regression gate (load-bearing):** clean A/B via
  `git stash` against baseline `b223350`. `spawn_batch_10k` (1-comp + 3-comp):
  criterion **"no change detected"** (CI straddles 0, p ≫ 0.05). `query_iter`:
  unchanged-to-faster (the iter path has no gate). Dedicated `create_entity` /
  `delete_entity` single-op no-hook benches (P4) confirm the `#[cold]` helpers
  don't perturb the hot prologue. **The flag-gate's one `u16` load + not-taken
  branch is below the measurement noise floor — 0% confirmed**, mirroring Phase
  10's "0% when unused."
- **Build:** `cargo build --release`, `cargo clippy -p boyko-ecs --lib -- -D
  warnings`, `cargo build --all-targets` all clean. 462 lib tests pass.

## Residuals / follow-ups (Phase 14b + housekeeping)

| Item | Status |
|---|---|
| `on_despawn` entity-level hook + Bevy-parity ordering | Phase 14b |
| Full Observers (entity-targeted, `CachedObservers`, custom events) | Phase 14b |
| `get_component_mut` in the hook view (mutable component access) | Phase 14b |
| derive + runtime **merge** (vs the current XOR) | Phase 14b (needs `UnsafeCell` `HOOKS` table) |
| **F4** — `EntityInland` `as_mut_ptr()` slab-storage TB-UB | Pre-existing; fix = `UnsafeCell`-rooted slab / exposed-provenance (touches Phase-11 storage model, ~12 sites) — `docs/PHASE-14-F4-FINDING.md` |
| `BundleColumnCache` by-design `Box::leak` (Phase 12.5 SBO6) | Not a defect; Miri suites need `-Zmiri-ignore-leaks` (Phase-9.1-class) |
| Multi-thread Miri on the deferred drain | Deferred per Phase 9.1 |
| `cargo asm` confirmation of the not-taken gate | `cargo-show-asm` not installed; the A/B bench is the load-bearing evidence |

## Key files

- New: `crates/boyko_ecs/src/ecs/core/component/hooks/{mod,archetype_flags,dispatch,deferred_master,builder,scope}.rs`.
- Touched: `archetype.rs` (`flags` field + compute), `archetype_bundle.rs`
  (slab init), `component_registry.rs` (`HOOKS` table + `get_hooks`/`install_hooks`/`try_set_hooks`),
  `component.rs` (`HAS_HOOKS` + `register_hooks`), `command_queue.rs`
  (`apply_via_raw_twin`), `ecs_master.rs` (`deferred_hook_queue` + drain +
  `register_component_hooks` + `fire_despawn_hooks`), `schedule.rs` (2 bracket
  sites), `boyko_macros/src/lib.rs` (`#[component(...)]`).
- Tests: `tests/phase14a_hooks_firing.rs`, `phase14a_hooks_deferred.rs`,
  `phase14a_repro_reentrant_drain.rs`, `miri_phase14a.rs`, `compile_fail_hooks.rs`
  (+ 5 fixtures); `benches/phase14a_hooks_gate.rs`.
- Docs: `PHASE-14-RESEARCH.md`, `PHASE-14-OBSERVERS-PLAN.md`,
  `PHASE-14-OBSERVERS-PLAN-ROUND2.md`, `PHASE-14-CRITIC-ROUND-{1,2,3}.md`,
  `PHASE-14-F4-FINDING.md`, this file.
