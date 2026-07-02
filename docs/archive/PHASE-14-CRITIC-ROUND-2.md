> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 14a — Architecture Critic, Round 2

**Verdict: REVISE.** The Round 2 revision (`PHASE-14-OBSERVERS-PLAN-ROUND2.md`)
**RESOLVES all three Round 1 CRITICALs (C1, C2, C3)** in design and most W/O
findings cleanly, with markedly better source-grounding. But opening the real
source at the migration + direct-API sites surfaced **2 new must-fix blockers**
+ 1 must-fix statement defect that the phased pseudocode glossed over. All are
concrete and fixable (addressed by Round 3 Patches §8).

## Resolution status of Round 1 findings

| R1 finding | R2 status | Evidence |
|---|---|---|
| C1 drain double-drive + nested panic | **RESOLVED** | single-owner drain + `hook_drain_depth` gate; sequential (not nested) `catch_unwind` proof verified vs command_queue.rs:203-251. `delete_entity` from `DespawnCommand::apply` runs at depth>0 → end-of-method drain returns early. |
| C2 `get_component_mut` footgun + live `&mut Archetype` | **RESOLVED** (view) / see NEW-2 (insert site) | reduced read-only view statically removes the footgun (`get_component_raw_mut` unreachable). 4 of 6 sites' restructure verified realizable; insert-migration needs NEW-2. |
| C3 vacuous DCE'd bench gate | **RESOLVED** | gate → Wave 4 real not-taken branch guarding `#[cold]` trigger; Wave 1 = field + asserts; `cargo asm` named. |
| W1 per-despawn `to_vec` | **RESOLVED** | stack `[ComponentId; MAX_COMPONENTS]`, cold-only entry (see NEW-3 for frame-size caveat). |
| W2 false "zero growth" | **RESOLVED** (see W2-literal) | +8 B correction right (`component_mask.rs:7` `#[repr(align(32))]`); ComponentLayout==56 tripwire is a real addition. TRIPWIRE 1 placeholder is tautological (W2-literal). |
| W3 staleness silent-skip | **RESOLVED** | release panic + derive-immunity coherent. |
| W4 IN_SYSTEM_RUN unstated | **RESOLVED** | SAFETY-7 verified vs tls.rs:142-166 + schedule.rs:605-623 (guard dropped before `apply`). |
| W5 mem::take discard | **RESOLVED** (see W5-statement) | raw-twin reuse correct; but §2.2 statement reborrows `self` twice (W5-statement). |
| O1/O2/O3 | **RESOLVED** | fn-ptr Send+Sync; `define_trigger!`; insert-replace folded into C2. |
| §7 residual #1 dual-presence (NEW-4) | **RESOLVED** | `move_out`/`drop_at`/`Swapped` traced vs archetype.rs:540-554; no double-free, consistent snapshot. |
| §7 residual #2 depth bracket (NEW-5) | **RESOLVED** (modulo NEW-1) | re-entry forbidden (APP4/CQ7); per-apply bracket minimal + complete once NEW-1 fixed. |
| §7 residual #3 repoint asymmetry (NEW-6) | **RESOLVED** | insert reads target / remove reads source — Bevy parity; hoist touches `entity_master` not archetypes. |

## Must-fix (blocked "developer can start" — addressed by §8 Round 3 Patches)

### NEW-1 (CRITICAL) — depth bracket leaks on early-`return Err` / panic
`create_entity` (ecs_master.rs:582-593) and `create_entity_at` (:642, :689-691)
have early `return Err` paths between the linear `enter_deferred_scope` …
`exit_deferred_scope`. A leaked increment leaves `hook_drain_depth >= 1` forever →
`drain_deferred_hook_queue` silently stops draining for the rest of the process.
Reachable on ordinary error paths, no panic needed. **Fix:** RAII
`DeferredScopeGuard` (Drop decrements depth) mirroring `CursorSync`/`InSystemRunGuard`.
→ Patch P1.

### NEW-2 (HIGH) — §3.4 fires `on_insert` over all `target_ids` (over-fires retained)
`target = source ∪ bundle`; firing `on_insert` for every target id spuriously
fires it for retained-not-in-bundle components, diverging from Bevy + Round 1 Q7.
**Fix:** capture `bundle_ids` during Step-2 (before `move_out_entity`), fire
`on_insert` over the bundle set I (not T); `on_add` over `I\S`. → Patch P3.

### W5-statement (HIGH) — §2.2 drain reborrows `self` + `self.deferred_hook_queue` simultaneously
`self.deferred_hook_queue.apply_via_raw_twin(NonNull::from(&mut *self))` (line 545)
is the exact field-alias the raw-twin exists to avoid. **Fix:** re-derive the twin,
drop the `&mut` field borrow before applying via the twin against `world_ptr`.
→ Patch P2.

## Should-fix (pin before Wave 4)

### NEW-3 — 4 KB stack frame on `delete_entity`/`create_entity` no-hook path
The `[ComponentId; MAX_COMPONENTS]` (~4 KB) local can perturb no-hook codegen even
when un-entered; the C3 branch-bench would miss it. **Fix:** add `delete_entity`/
`create_entity` no-hook benches to the Step 13/17 gate; mitigation = hoist to an
`EcsMaster`-resident scratch or a `#[cold]` helper fn. → Patch P4.

### W2-literal — TRIPWIRE 1 is a tautology
`assert!(size_of::<Archetype>() == 0 + size_of::<Archetype>())` guards nothing.
**Fix:** measure + pin the concrete literal in Wave 1 Step 1. → Patch P5.

## Files verified
command_queue.rs, migration_helpers.rs, despawn_command.rs, spawn_at_command.rs,
insert_command.rs, ecs_master.rs (116-252, 501-700, 880-1021, 1280-1722),
archetype.rs (109-170, 480-614), component_mask.rs (repr(align(32))),
component_registry.rs (ComponentLayout 56 B doc-only :90), schedule.rs
(115, 232-366, 495-639), boyko_threadpool/tls.rs, entity_inland.rs.
