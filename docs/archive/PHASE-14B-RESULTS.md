> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 14b — Results: Component Lifecycle Observers

Branch `ecs`. **Status: COMPLETE + SOUND.** Observers are the
runtime-mutable sibling of the Phase 14a `ComponentHooks`: where a hook is a single fn-ptr bound
per-type at registration (write-once into the global `HOOKS` table), an **observer** is one of an
arbitrarily-long, `add`/`remove`-able list of fn-ptrs keyed by `(kind, component)`. They fire
synchronously at the same structural-op sites as hooks, AFTER the per-component hook, and mutate the
world only via deferred `commands()` on the read-only `DeferredEcsMaster` view.

Pipeline: research → architect R2+R3 → architecture-critic R1+R2 → 3 developer waves → code-review →
tester (×2) → bug-fix wave (2 pre-existing bugs) → code-review → tester. All gates green.

## What shipped

### Public API (`EcsMaster`)
- `observe_on_add::<C>(runner) -> ObserverId`, `observe_on_insert`, `observe_on_replace`,
  `observe_on_remove` — `runner: ObserverFn = unsafe fn(DeferredEcsMaster<'_>, ObserverContext)`.
- `add_observer(kind: ObserverKind, cid: ComponentId, runner) -> ObserverId`,
  `remove_observer(id: ObserverId) -> bool`.
- `get_component_mut::<T>(entity) -> Option<Mut<'_, T>>` (was `Option<&mut T>`) — change-detection-
  correct direct-API mutable access (zero internal callers, so the signature break was internally free).
- `ObserverContext { entity, component_id, kind }`, `ObserverKind { Add, Insert, Replace, Remove }`.

### Design (the decisions that mattered)
- **D1** — no `Despawn` kind: despawn fires `on_replace` + `on_remove` per dying component (mirrors hooks).
- **D2** — runner = bare fn-ptr (mirrors `HookFn`), NO `Box<dyn FnMut>`; capture goes through Resources.
  fn-ptr ⇒ `Send + Sync` by construction (no `unsafe impl`).
- **D3 (the C1 crux)** — the `ObserverRegistry` lives as a `pub(crate)` field on **`ArchetypeMaster`**,
  NOT on `EcsMaster`. This is the resolution of the critic's CRITICAL C1 (D3⊥D5): archetype
  construction seeds the per-archetype `ON_*_OBSERVER` flag bits from the registry, and construction
  recipes (`create_by_ids`/`register_component_inplace`/the slab bundle) take only `(ids, &Arena)` —
  they cannot reach `&EcsMaster`. Co-locating the registry on `ArchetypeMaster` lets the **single
  creation funnel `create_archetype`** seed observer bits at construction (borrow-split: read
  `&self.observer_registry` into a `Copy` `ArchetypeFlags`, then OR into the new slot via
  `self.archetypes.get_archetype_ptr_mut`). This is Bevy's per-`World` `Observers` model minus the
  parameter-threading our 3-layer slab recipe can't support.
- **D4** — component-targeted observers only (no global tier — preserves per-archetype bit selectivity).
- **D5** — dynamic `ArchetypeFlags` maintenance: seed at construction + add-first walk
  (`iter_archetypes_mut`, set the bit on archetypes containing cid) + remove-last recompute
  (`flags = (flags & !ON_kind_OBSERVER) | (any_sibling_observes_kind ? bit : 0)`, scanning the
  registry, preserving the hook bit) + a `#[cfg(debug_assertions)]` bit⇔registry tripwire at all
  seed/walk sites.
- **Storage** — `ObserverRegistry { lists: Option<Box<ObserverLists>>, next_id: u64 }`;
  `ObserverLists = [[Vec<ObserverEntry>; 512]; 4]` (`[kind][cid]` dense index). `Option<Box<_>>` (NOT
  `OnceLock` — registration is `&mut self` single-threaded, so `OnceLock`'s thread-safe one-shot init
  buys nothing and blocks repeated `&mut`-Vec mutation). Stays `None` until the first `add_observer` —
  zero allocation / zero 48 KiB cost when unused; `ArchetypeMaster::new` preserves the ~7 µs
  `EcsMaster::new` budget.

### C1 coverage (the airtight ∎)
The only two functions that set a bundle occupancy bit are `ArchetypeBundle::add_archetype` and
`add_archetype_from_components_fallible`. They are reachable on a master-owned bundle ONLY via (i) the
`create_archetype` funnel (seeds) or (ii) `add_existing_archetype` (seeds). The third route — a
`&mut ArchetypeBundle` from `ArchetypeMaster::archetype_bundle_mut` — was a `pub` bypass that would
mint unseeded archetypes; it was **narrowed to `pub(crate)`** (verified zero callers). `create_by_ids`
returns a detached by-value `Archetype` (no bundle bit). Every archetype reachable through the
public/crate API therefore has correct observer bits.

### Dispatch + 0%-gate
- 4 `#[cold] #[inline(never)]` `fire_*_observers` fns (`observers/dispatch.rs`). The **OBS-FIRE-LOOP**
  invariant: each loop turn re-derives a fresh registry `&`, copies one 16 B `ObserverEntry` by value,
  and lets every borrow end BEFORE `DeferredEcsMaster::from_world` is minted — no registry/`world` `&`
  spans the view mint or runner call (the registry lives *inside* the world, so a held `&` would be
  the F2-class Tree-Borrows hazard that produced UB in 14a).
- Wired at **all 7 hook-fire functions**: 3 direct-API (`create_entity`, `create_entity_at`,
  `fire_despawn_hooks`) + **4 deferred-command apply sites** (`SpawnAtCommand::apply`,
  `InsertCommand::apply_replace_in_place`, `migrate_entity_insert`, `migrate_entity_remove`). Each kind:
  outer gate unchanged (`!flags.is_empty()` or the widened combined gate), inner widened
  `ON_*_HOOK` → `ON_*_ANY`, hooks fire BEFORE observers, observer set == hook set per site.
- **0%-gate held** (bench-verified): the no-observer/no-hook hot path is byte-identical — for an
  archetype with `flags == 0` the fire block is skipped; `ON_*_ANY` is a different immediate in the
  same `test`/`jz`. The `fire_*_observers` cold helpers do not bloat the hot-site I-cache footprint.

## Verification

| Gate | Result |
|------|--------|
| `cargo test -p boyko-ecs` | **889 passed; 0 failed; 21 ignored** — zero regression over the 845 pre-14b baseline |
| Miri `-Zmiri-tree-borrows` (`miri_phase14b`) | **10/10 PASS, Tree-Borrows CLEAN** — fire loop, seed borrow-split, `get_component_mut` `Mut`, AND the command-apply paths (spawn / insert-migration / remove-migration / re-entrancy) |
| `cargo clippy --all-targets -- -D warnings` | clean (orchestrator-verified, not just dev-claimed) |
| 0%-gate (`phase14a_hooks_gate`) | no measurable regression on the no-observer hot path (structural byte-identity + within-noise A/B) |
| new `unsafe` | net-neutral-to-reduced; every block carries a SAFETY comment; the migration fix net-REMOVED ~7 unsafe blocks |

## Bugs surfaced + fixed during verification (2 pre-existing, not 14b regressions)
The tester's new command-apply coverage exercised paths no prior test reached:
- **NEW-1 (CRITICAL, Miri-TB UAF) — FIXED.** `migrate_entity_insert` stored bundle `&[u8]` from
  `Bundle::for_each_component_bytes` and read them back at Step 4 (`create_entity_with_ticks`) AFTER
  the closure returned — the macro contract (`boyko_macros/src/lib.rs:1062`) bounds those slices to
  the closure's stack frame, so they dangled (the **same class Phase 11 fixed** for
  spawn/`apply_replace_in_place`, but migrate-insert was missed). Fix: rewrote the path to consume
  bundle bytes INSIDE the closure (write straight into the target pools, lockstep commit, bundle-wins-
  on-overlap), removing the 3 stack `MaybeUninit` arrays — net-reduced `unsafe`. Miri-TB confirms clean.
- **NEW-2 (correctness, drain gap) — FIXED.** `run_cached_system` lacked the depth-gated
  `drain_deferred_hook_queue()` that `Schedule::run` has, so a command enqueued by a callback fired
  from a command-apply context was never drained. Fix: one depth-0-gated drain call after
  `system.apply` (production `Schedule::run` was already correct via its barrier drain).

## Bugs filed for later (pre-existing, out of 14b scope — "fix bugs before features" backlog)
- **#53** — pre-existing 8 B `Vec<InlandPoolId>` exit-leak on the deferred-spawn/pool-setup path
  (`bundle_column_cache.rs`), present in the 14a baseline; NOT UB.
- **#55** — insert-migration overlap drops nothing: inserting a bundle component that already exists in
  the source leaks the displaced value (no `Drop` runs); pre-existing; `on_replace` does fire.
- **#56** — deferred-added components are invisible to `Added<T>` filters next frame (tick-boundary
  property predating 14b, affecting deferred spawn identically); determine Bevy-parity vs real gap.

## Lessons
- **The tester (behavioral oracle) caught what architect R1+R2+R3, critic R1+R2, and code-review all
  missed:** the plan's "6 fire sites" undercounted the actual hook-fire inventory — Phase 14a also
  fires at 4 deferred-command apply sites, so observers were silent for the entire `Commands` API until
  the tester wrote tests against the user-facing API. "APPROVED per plan" does not catch an incomplete
  plan; only behavioral coverage does.
- **Miri-TB remains the only soundness oracle** — it found the pre-existing migration UAF that
  critic+review APPROVED reasoning never would (the 14a lesson, again). Each new code path needs its
  own Miri-TB exercise, not inherited confidence.
- **A read-only feasibility/inventory pass beats trusting a plan's enumeration:** an orchestrator grep
  of the complete `trigger_on_*` call inventory (before the fix) confirmed exactly 4 missed sites and
  no 5th — preventing a Dyn-4 round.
- **Re-verify dev "green" claims yourself:** dev wave 1 claimed `clippy --all-targets` clean but had 20
  test-target errors (missing `Debug` derives); the orchestrator's own gate run caught it. The compiler
  is the only reliable oracle.

## Files
- New: `crates/boyko_ecs/src/ecs/core/component/observers/{mod.rs, dispatch.rs}`.
- Modified: `component/mod.rs`, `component/hooks/archetype_flags.rs`, `archetype/archetype_master.rs`,
  `ecs_master/ecs_master.rs`, `iters/query/data.rs`, and the fire wiring +
  NEW-1 fix in `commands/{spawn_at_command, insert_command, migration_helpers}.rs`.
- Tests: `tests/phase14b_observers_firing.rs`, `phase14b_observers_deferred.rs`,
  `phase14b_get_component_mut.rs`, `phase14b_insert_migration_correctness.rs`,
  `compile_fail_observers.rs` (+ fixtures), `miri_phase14b.rs`.
