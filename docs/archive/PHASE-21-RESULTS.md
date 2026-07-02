# Phase 21 — Multi-world hardening — RESULTS

**Branch:** `ecs`. Closes direction #3 (multi-world). Design basis: the
project-analyst multi-world audit.

## Audit verdict (the foundation)

**Clean.** Every process-global in the engine is **metadata-only**: the
component / event / bundle-type / query-type / resource-type registries, the
`HOOKS` table, and the id counters store type-level facts, never
world-derived state. All world state (archetypes, pools, entities, caches,
observers, resources, event buffers, states) is world-owned, so N
`EcsMaster`s / `App`s coexist safely. Two hardening holes were found and are
fixed here.

## H1 — world-blind hooks staleness scan (FIXED)

`register_component_hooks` panic-scanned only SELF's archetypes, but `HOOKS`
is process-global: world B with `C` already live got **silently-skipped
hooks** (B's pre-install archetypes' `ArchetypeFlags` lack the bit, and the
flags are OR-computed once at construction).

Fix: process-global `EVER_ARCHETYPED` bitmask in `component_registry.rs`
(one bit per `ComponentId`; `MAX_COMPONENTS = 512` ⇒ 8 × `AtomicU64`;
Relaxed both sides — the panic is a config-time courtesy, not a soundness
fence). Set at **both** archetype-mint funnels
(`ArchetypeMaster::create_archetype` + the `add_existing_archetype` bypass —
the same two sites that seed observer flags); checked by
`register_component_hooks` instead of the per-world scan (which it strictly
subsumes: every archetype of any world is minted through a funnel). Bits are
never cleared — `clear()` of one world proves nothing about other worlds,
and the `HOOKS` slot is write-once for the process anyway.

Documented asymmetry (by design): **hooks are process-global per type;
observers are per-world** (`ObserverRegistry` lives on each world's
`ArchetypeMaster`).

## H2 — WorldId + schedule-world binding (FIXED, option (a) — Bevy parity)

A cross-world `Schedule::run` was undetectable, and the systems' per-world
caches (`EventReaderState`'s cached `NonNull<EventBuffer<E>>`, `QueryState`
generation collisions) made it a latent **use-after-free / aliasing
surface**.

- `WorldId(u64)` (`identifiers/primitives.rs`) minted per
  `EcsMaster::new`/`with_capacity` from a process-global `AtomicU64`;
  `EcsMaster::world_id()` accessor; field at the top of `EcsMaster` (Copy,
  no drop glue — the C5 drop-order contract is unaffected).
- `Schedule` records the build world's id at `ScheduleBuilder::try_build`
  (appended as the LAST field, M3 discipline — no pre-existing offsets
  change); `Schedule::run` **release-panics** `boyko-B9101` on a mismatch.
- `App` needs no assert: it owns its world and builds both schedules on it
  (structurally safe).

### P20-B1(a) amendment + bench A/B

The Phase 20 run-entry byte-identity gate is amended by exactly **one u64
compare** (+ an out-of-line `#[cold]` panic helper). A/B on
`phase9_schedule_run_50_exclusive_systems` (criterion, `--save-baseline
p21base` before the edit / `--baseline p21base` after):

| bench | pre (p21base) | post | verdict |
|---|---|---|---|
| 50-system schedule run | 4.2936–4.3129 µs (pt 4.3026 µs) | 4.2392–4.2672 µs (pt 4.2514 µs), change −0.94%..−0.17% (p = 0.01) | ✅ within ±2% (the one-compare is invisible) |
| empty schedule run | 5.6706–5.7420 ns (pt 5.7054 ns) | 5.7423–5.7958 ns (pt 5.7686 ns), change +0.19%..+2.30% | ✅ ≈ +0.06 ns — the honest cost of the one u64 compare, only measurable on a 5.7 ns frame |
| `EcsMaster::new` | gate ≤ 7.5 µs | 5.017–5.142 µs (pt 5.079 µs) | ✅ under the gate |

(`two_disjoint` −5.2% "improved" and `one_exclusive`/`par_iter` "no change"
are codegen/noise — nothing in this phase touches those paths beyond the one
compare.)

## Suite — `tests/multi_world.rs` (10 tests)

1. Two `EcsMaster`s: independent spawn/get/despawn, same bundle type in both
   (extends the `clear_respawn` pin); `clear()` of A leaves B intact;
   post-clear respawn works.
2. Two `App`s, SEPARATE pools, interleaved `update_with_delta` frames:
   change detection per-world (A never sees B's mutation).
3. Two `App`s, ONE shared pool (`App::with_pool`): events preregistered with
   `EventConfig::default_for(worker_count + 1)` (the H4 contract); an event
   sent in A is never visible in B (exact double-buffer counts asserted).
4. States: same `States` type in both worlds, independent transitions.
5. **H1 regression** (`should_panic`): world B has `C` archetyped; world A's
   `register_component_hooks::<C>` panics (pre-21: silent skip).
6. Observers: registered in A, does not fire in B.
7. Cross-world `Entity`: out-of-range foreign handle → `None`/`false`;
   `(id, generation)`-collision aliasing pinned as DOCUMENTED behavior.
8. **H2 regression** (`should_panic`): schedule built on A, run on B →
   `boyko-B9101` (positive half: run on A first succeeds).
9. `WorldId` process-uniqueness across both constructors.
10. Compile-time pins: `EcsMaster: Send + Sync`;
    `App: !Send + !Sync` (`assert_not_impl_any`).

Miri subset: the pool-less tests (1, 5, 6, 7, 9, 10) run under
`-Zmiri-tree-borrows`; pool tests carry the standard `#[cfg(not(miri))]`
gate.

### App `!Send` finding (stale comment fixed)

`app.rs` claimed `!Send + !Sync` was "inherited from EcsMaster" —
**wrong since Phase 9** (EcsMaster is `Send + Sync`, pinned in
`send_sync_negative.rs`). The real reasons: the staged
`StartupSystem = Box<dyn FnOnce(&mut EcsMaster)>` closures (no `+ Send`) and
the schedules' `StateEntry::insert` closures of the same shape. Comment
reworded; compiler-oracle pin added to the suite.

## Bevy-parity table (from the audit)

| Concern | Bevy | boyko (post-21) |
|---|---|---|
| Multiple worlds per process | ✅ | ✅ (audit: globals metadata-only) |
| `Entity` world-tagged | ❌ (8-byte handle) | ❌ — same trade, aliasing pinned as documented |
| Schedule bound to world | ✅ (panics on foreign world) | ✅ `boyko-B9101` (this phase) |
| Component ids | per-world | process-global metadata (ids stable across worlds — strictly more permissive) |
| Hooks scope | per-world | process-global per type + global staleness gate (this phase) |
| Observers scope | per-world | per-world |
| SubApp / extract | ✅ | future work |

## Gates

- `cargo check --workspace --all-targets` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- Full `cargo test --workspace --all-targets` — green: **1074 passed, 0
  failed** (incl. the 10 new multi_world tests).
- wasm32: `cargo check --target wasm32-unknown-unknown -p boyko_demo` —
  green.
- Miri (`-Zmiri-tree-borrows -Zmiri-ignore-leaks`): miri_fixed_loop,
  miri_pool_growth, miri_entity_store, multi_world (pool-less subset) —
  clean.
- Bench: see the A/B table above.
