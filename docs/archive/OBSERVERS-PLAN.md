> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Entity-targeted observers + custom triggers + propagation + on_despawn (Feature 2 of 3) — resolved plan

Branch `ecs`, 2026-06-16. Extends the Phase-14b `(kind, component)` component-observer
registry with: (1) entity-targeted observers, (2) **custom `Trigger` events on ANY
user type** (user-confirmed requirement), (3) propagation up `ChildOf`, plus (4) the
`on_despawn` hook+observer that Phase-14a cut. Produced by research → architect → 2
critics; this folds the CRITICAL/MAJOR critique findings in as resolved decisions.
Full design at `D:\tmp\p1_wf\design_observers.md`. Developer must `graphify`-orient +
read source for exact signatures.

## Two event models (document this split — the user asked for "events on any event")

- **Observer events (THIS feature):** immediate, reactive, run **inline at trigger
  time** on the firing thread, entity-targeted + propagation. For reactive/sparse
  events ("door opened", "entity died", "took damage now"). Cost = O(observers for the
  event), NOT O(entities). NOT parallelized — do NOT use for bulk per-frame-per-entity.
- **Buffered events (Phase 12, exists):** bulk, frame-deferred, read by a polling
  system, parallel-friendly. For "many events processed in batch."
Both work on ANY user `#[derive(Trigger)]` / event type. Pick per use-case; doc it.

## Core design (decisions; see the design doc for full bodies)

- **D1 entity-targeted store** = a per-world `EntityObserverStore` (lazy `Option<Box>`),
  `SparseMap` keyed by `EntityId.0` (O(1) probe, no hashing, only observed entities
  occupy a slot) — NOT an `ObservedBy` component (that would archetype-move the watched
  entity). Per-entity list keyed by a unified `DispatchKey` (packs lifecycle-kind OR a
  dense custom `EventId`); a `generation` recycle-guard so a reused `EntityId` never
  inherits a dead observer.
- **D2 unified `DispatchKey`** (u32: high bit = custom-event vs lifecycle) so lifecycle
  + custom share one fire loop + one `ObserverFn` shape. `ObserverKind` gains
  `Despawn=4` (`NUM_OBSERVER_KINDS=5`); the dense `[[Vec;512];4]` widens to `[..;5]`.
- **D3 custom triggers** = `EcsMaster::trigger::<E>(target, ev)` / `trigger_global`;
  global observers in a lazy `TriggerRegistry` keyed by the **Phase-12 dense `EventId`**
  (no new id space, no HashMap); event read by `*const u8` (monomorphized reader fn-ptr,
  no `Box<dyn>`); entity-targeted custom observers live in the D1 store under
  `DispatchKey::custom(eid)`.
- **D5 propagation** = a `Traversal` trait (default `ChildOfTraversal`); bubbles up
  `ChildOf` ONLY when an observer opts in (`const AUTO_PROPAGATE` const-folds the walk to
  one hop for non-bubbling events; `propagate(false)` stops it). Re-derive `ChildOf` per
  hop (never span the view mint). `MAX_PROPAGATION_DEPTH` debug cycle tripwire.
- **D7 on_despawn** = `ComponentHooks.on_despawn` (the 14a cut, reversed) + `ObserverKind
  ::Despawn`; fires in `fire_despawn_hooks` BEFORE components drop.

## CRITICAL/MAJOR fixes folded in (the critique blockers)

### FIX W2/C4/C5 (CRITICAL) — `HAS_ENTITY_OBSERVER` must be a STICKY flag, not runtime-mutable
The design's Decision 4 made it a refcount-driven raise/CLEAR flag. The critics rejected
this: **every `ArchetypeFlags` bit today is set-once-at-archetype-creation and read
locklessly via a raw `*archetype_ptr` as a stable `u16`** — `EVER_ARCHETYPED` (Phase-21
H1) exists *precisely because* you cannot mutate a live archetype's flags. A
runtime-mutable bit races: (a) a lockless `(*ptr).flags` read during a structural fire
vs a concurrent mutate; (b) clear-on-last-detach races re-raise; (c) the migrating
entity's own despawn/on_remove fire reads the flag while maintenance is mid-move; (d) the
14b `remove_observer` recompute could stomp it.
**Resolution — make it STICKY (set-once, NEVER cleared, exactly like `EVER_ARCHETYPED`):**
an archetype that has EVER had an entity-observed member keeps bit 10 set forever. Raise
on attach (0→set) and on migration-to-a-new-archetype (set on the destination); **never
decrement, never clear.** The `SparseMap` probe then misses cheaply for un-observed
entities in a once-observed archetype (the acceptable cost — observed archetypes are
rare). This dissolves all four races (set-once = the existing immutable-flags model).
The 14b `remove_observer` flag-recompute MUST **OR-preserve** bit 10 (recompute owns bits
5-9 only; bit 10 is owned solely by the entity-observer subsystem and is never part of a
recompute). Drop `per_archetype_observed` refcounts entirely (sticky needs no count).
State the `observe_entity`-vs-apply-window ordering: `observe_entity` runs under
`&mut EcsMaster` (no fire in flight); the raise is a single `|=` on the entity's current
archetype flags, done before any fire reads them.

### FIX W8 (MAJOR) — `SparseMap<U: Clone>` clones the whole Vec on `swap_remove`
`SparseMap<EntityObserverList>` would deep-clone the entries `Vec` on every detach/despawn
(boyko_utils `SparseMap` is `impl<U: Clone>` and `swap_remove` does `dense[i].clone()`) —
a hidden heap alloc on the despawn path (violates principle #5).
**Resolution:** store a `u32` HANDLE in the `SparseMap` (`SparseMap<u32>`, Copy — cheap
swap_remove) pointing into a side `Vec<EntityObserverList>` arena with a free-list for
slot reuse. The arena entry is `mem::take`-n on detach (no clone). (Alternative: extend
`boyko_utils::SparseMap` with a `Clone`-free `remove` via `mem::take`; the handle+arena is
preferred — it keeps `boyko_utils` untouched and the `SparseMap` payload POD.)

### FIX W9 — propagation TLS holds ONLY the `propagate` bool
The design's Decision-7 prose said "propagation state (propagate flag + original_target)"
is TLS, contradicting the `TriggerContext` struct which carries `original_target` by value.
**Resolution (the struct is right):** ONLY `propagate: bool` lives in a `thread_local!
Cell<bool>` (saved/restored per `trigger` call by a `PropagateGuard` RAII — re-entrancy
safe); `original_target` + `target` + `event_id` travel in `TriggerContext` BY VALUE
(re-entrancy-safe, no TLS). Fix the prose.

### FIX W10 — on_despawn cascade order: PARENT-first, pinned + tested
`on_despawn` fires Despawn→Replace→Remove WITHIN one entity (all pre-drop — sound). For
the **cascade**: the parent's despawn fires the parent's `on_despawn` FIRST, then the
parent's `Children` `on_replace` enqueues the children for deferred despawn — so the
**parent's `on_despawn` sees a fully-intact subtree (children still alive)**, and each
child's `on_despawn` fires later as the deferred cascade drains. This is PARENT-first.
**Drop the design's contradictory "children-before-parent" claim** — with the
cascade-via-`Children`-on_replace mechanism + Despawn-first, parent-first is the natural,
documented contract (parent cleanup can read its intact children). Mandatory test: a
3-level subtree asserts parent's `on_despawn` fires before its children's, and each
entity's local Despawn→Replace→Remove order. (If children-first is ever wanted, it's a
follow-up reorder; not v1.)

### FIX O3 — `Trigger: 'static` (drop `Send + Sync`)
The event is read by `*const u8` synchronously on ONE thread within `trigger` (never
crosses a thread boundary), so `Send + Sync` is not needed for soundness and would
exclude `Rc`/non-Sync payloads. `pub trait Trigger: 'static { const AUTO_PROPAGATE; type
Traversal; }`.

## Open decisions resolved

- **on_despawn within-entity order** = Despawn-first (handler reads the fully-intact
  dying entity; documented divergence from Bevy's Replace→Remove→Despawn — both pre-drop,
  both sound).
- **`HAS_ENTITY_OBSERVER` granularity** = archetype-level sticky bit (not a per-entity bit
  on the hot `EntityInland` record).
- **Deferred custom triggers** (a `commands().trigger(...)` from inside an observer) =
  **v1 fires immediately/re-entrantly**; a deferred path (boxing the payload into the
  queue) is out of scope v1.
- **`Traversal`** = the trait + a single `ChildOfTraversal` impl (the trait is free; more
  impls additive).
- **Macro vs runtime-builder** = runtime-builder API first (`observe`/`observe_entity*`/
  `register_component_hooks().on_despawn(...)`); `#[derive(Trigger)]` +
  `#[component(on_despawn=...)]` parsing is additive (this phase if cheap, else follow-up).

## Cross-feature contracts

- **Required components (Feature 1, shipped):** required auto-inserts already fire
  on_add/on_insert through the existing fire sites (Feature-1 C1 fix) → their observers
  (incl. entity-targeted) fire for free. NOTE the filed gap: `spawn_batch` fires NO
  lifecycle hooks for any component, so required-on-spawn_batch won't fire observers
  either (consistent; that gap is task `task_00223bbd`, separate).
- **Cloning (Feature 3, next):** clone-fires-observers will dispatch through this
  feature's fire loops; build Observers before Cloning so the fire infra exists.

## 0%-gate (sacred)

A spawn/insert/despawn/query touching none of these features stays byte-identical:
`ArchetypeFlags` stays 2 B (new bits 4/9/10 fit the existing `u16`; same one-load + test,
different immediate); `fire_despawn_hooks` is not even ENTERED unless `!flags.is_empty()`;
the entity-store/trigger-registry are lazy `Option<Box>` (one `is_none()` per relevant
site until first use); `const AUTO_PROPAGATE` const-folds the bubble walk; `ComponentHooks`
grows 4→5 fn-ptrs but it is a cold-table `OnceLock`, not `ComponentLayout` (TRIPWIRE 2
unaffected). Validate: despawn/spawn/query criterion benches with no observers = "no
change detected" + asm diff of the no-flag despawn path empty.

## Soundness (Miri-TB is the oracle — this touches the despawn hot path + new unsafe)

OBS-FIRE-LOOP / F2 / 9.3c discipline is the backbone: no `world`-derived `&` (incl. the
registry `&`) spans `DeferredEcsMaster::from_world` or a runner call; re-derive the store
`&` per fire turn, copy the `ObserverEntry`/`DispatchKey` out by value, drop the `&` before
minting the view. Cross-`Drop`/cross-fire mutable state (the `propagate` bool) is a TLS
`Cell`, NEVER a cached `NonNull<EcsMaster>` written in `Drop` (the 14a-F2 / 9.3c / Phase-19
class — caught only by Miri-TB after critics approved). Re-entrancy (observer
spawns/despawns/triggers) routes structural ops through `view.commands()` → the deferred
queue, drained by the outermost owner (existing TLS depth counter); a re-entrant `trigger`
saves/restores the propagate TLS via `PropagateGuard`. **Run Miri-TB** on: the
entity-observer fire, the custom-trigger walk, the on_despawn fire + cascade, the
sticky-flag raise. The event-value lifetime under propagation (read `*const u8` across
hops) — `event: E` is moved into `trigger` and lives until the walk ends; observers get
read-only `*const u8`, cannot move/free it.

## Build order (developer; the design's 7-step plan)

1. Bits + kind + table widening (no behavior): `ArchetypeFlags` bit 4/9/10 + masks;
   `ObserverKind::Despawn`; `[[Vec;512];4]→[..;5]`; `ComponentHooks.on_despawn`;
   `trigger_on_despawn` emitted; `fire_on_despawn_observers` via the macro;
   `insert_from_hooks`/`insert_from_observers` widened (recompute OR-preserves bit 10).
2. `on_despawn` fire site in `fire_despawn_hooks` (Despawn-first) + runtime builder
   `on_despawn(...)` (+ optional `#[component(on_despawn=...)]`).
3. `DispatchKey` + `Traversal` + `propagate` TLS (+ `PropagateGuard`).
4. `EntityObserverStore` (SparseMap<u32 handle> + side arena + free-list; STICKY bit
   raise on attach/migration; `fire_entity_observers`; `observe_entity*` API; maintenance
   at migration sites).
5. `TriggerRegistry` + custom triggers (`Trigger: 'static`, `register_trigger::<E>()`
   EventId mint, `observe`/`observe_entity_event`/`trigger`/`trigger_global`, the bubble
   walk).
6. `EntityCommands::observe` deferred wrapper (+ optional `#[derive(Trigger)]`).
7. 0%-gate verification + tests + Miri-TB.

## Tests (mandatory)

Entity-observer fires for its entity ONLY; survives migration (sticky bit); stale-handle
guard (despawn+reuse EntityId → no inherited fire); custom trigger global+entity;
`trigger_global` global-only; propagation up ChildOf + `propagate(false)` stop +
`AUTO_PROPAGATE=false` fires only at target; on_despawn before drop (handler reads intact
values); **cascade 3-level: parent on_despawn before children, local Despawn→Replace→
Remove order**; re-entrancy (observer spawns/despawns/triggers → deferred drain once at
outermost); property test (random attach/detach/migrate/despawn → no fire after detach, no
double-fire, sticky-bit invariant). 0%-gate benches (no observers = no change). Miri-TB on
the fire/trigger/despawn/sticky paths. gnu-1.96; clippy clean.
