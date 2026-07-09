> STATUS: COMPLETED — archived 2026-07; implemented on branch `ecs`. See git history + the phase/feature RESULTS docs for the authoritative record.

# Phase 19 — Hierarchies / Parent-Child — Research

Research grounding for Phase 19 (entity parent-child relationships). Studied Bevy 0.16, flecs, EnTT,
Unity DOTS. Full citations at the bottom.

## Verdict: adopt the Bevy-0.16 relationship model on boyko's 14a/14b substrate

Parent-child = a **one-to-many relationship**: a foreign key on the child + a reverse collection on the
parent, kept consistent **by component hooks** — which is exactly Phase 14a/14b. Concretely:
- `ChildOf(Entity)` — source-of-truth component on the **child** (the FK / parent pointer).
- `Children(Vec<Entity>)` — reverse collection component on the **parent** (maintained reactively).

Reject the alternatives for boyko:
- **flecs pairs** `(ChildOf, parent)` — first-class, queryable, O(1), but **fragments archetypes**
  (each distinct parent = a distinct archetype member). boyko's archetype signature is a `BitSet` over
  `ComponentId`; per-target ids would explode the archetype space. **Reject.**
- **EnTT intrusive linked list** `{parent, first, prev, next}` — O(1), zero-alloc, but sibling
  traversal is **pointer-chasing / cache-hostile**, contradicting boyko's dense-iteration principle.
  Reject the list for the collection (keep the `parent`-FK idea = `ChildOf`). **Reject the list.**
- **Unity per-frame `ParentSystem` reconciliation + `PreviousParent`** — boyko has the *reactive*
  mechanism (hooks/observers) that makes deferred reconciliation + the extra cache component
  unnecessary (and it has a 1-frame staleness window). **Reject.**

## The Bevy-0.16 model (what to mirror)
- `Relationship` (on the child) registers `on_insert` + `on_replace` hooks; `RelationshipTarget` (on the
  parent) registers `on_replace` + `on_despawn`.
  - **`ChildOf` on_insert:** self-ref guard (parent==self → warn+remove); one-to-one evict old; dangling
    guard (parent doesn't exist → warn+remove); get-or-create parent's `Children`, push child.
  - **`ChildOf` on_replace** (fires on overwrite OR removal): remove child from the OLD parent's
    `Children` (swap-remove, scan from back — children removed LIFO); if empty, remove `Children`.
  - **Reparent = just inserting a new `ChildOf`**: old value's on_replace removes from old parent, new
    value's on_insert adds to new. No dedicated reparent path.
  - **Recursive despawn** = `Children` is `LINKED_SPAWN`; its on_despawn iterates the collection and
    enqueues `try_despawn` per child → recursion emerges through the **deferred command queue** (no
    manual recursive stack). In 0.16 plain `despawn()` is recursive; `despawn_related::<Children>()`
    despawns only children.
- `Children` collection = `Vec<Entity>` (dense, cache-friendly iteration; allocates on first child — a
  cold/structural op).

## boyko mapping — the mechanism already exists (14a/14b)
| Bevy concept | boyko equivalent |
|---|---|
| `Relationship::on_insert` / `on_replace` | `ComponentHooks` on_insert/on_replace for `ChildOf` (type-level, always-on) |
| `RelationshipTarget::on_despawn` cascade | despawn fires per-component `Replace`+`Remove` (ecs_master.rs:1099-1110) — route cascade through `Children`'s Remove hook (boyko has NO despawn kind — 14b D1 — and doesn't need one) |
| hook enqueues `try_despawn` | `DeferredEcsMaster::commands().entity(c).despawn()` (deferred_master.rs) |
| deferred drain at apply boundary | `drain_deferred_hook_queue` depth-0 (ecs_master.rs:1190, 2419) |
| re-entrancy guard | `DeferredScopeGuard` depth counter (ecs_master.rs:1123) |
| reverse collection | a `Children(Vec<Entity>)` `#[derive(Component)]` |
| 0%-when-unused | `ArchetypeFlags` u16 gate — no `ChildOf`/`Children` ⇒ no cost |

## CORE Phase 19 vs DEFERRABLE
- **CORE:** `ChildOf` + `Children` components; the hook wiring keeping them bidirectionally consistent
  (insert/replace/remove); self-ref + dangling guards (cheap reactive: warn+remove / debug_assert);
  recursive despawn via cascade; command ergonomics (`add_child`/`add_children`/`remove_children`/
  `set_parent`/recursive `despawn`) batching into the existing queue.
- **DEFERRABLE (own follow-up):** transform propagation (`GlobalTransform = parent.global × local` — a
  *consumer*, kept separate in every engine); change-detection-gated partial propagation (exploit
  Phase-10 `Changed<T>` + a dirty-subtree marker); parallel two-phase tree walk (on the Phase-9 pool);
  deep cycle detection (nobody pays for it — leave as a documented footgun / debug_assert ancestor walk).

## Pitfalls (historically gotten wrong — must avoid)
1. **Remove relationship data BEFORE hooks fire** → hooks see dangling refs (Bevy #20106). boyko fires
   on_replace+on_remove **before** `remove_entity` (ecs_master.rs:1082-1110) — CORRECT; make it a
   binding design invariant + test.
2. Despawn parent without updating children → dangling `ChildOf` (Bevy #5584). Moot under cascade; else
   detect lazily via `Entity{generation}` guard.
3. Reparent non-atomically → child in two `Children` lists or stranded. Hook order (Replace then Insert)
   guarantees correctness.
4. `despawn_recursive` on a STALE descendant snapshot (Bevy #17883) → iterate the CURRENT collection at
   despawn time, not a cached list.
5. Cycles: no engine pays for deep detection; catch self-ref only (one compare). Don't gold-plate.
6. Re-entrancy (despawn hook enqueues despawns) → boyko's `DeferredScopeGuard` + single outermost drain
   already solves it; relationship hooks must follow the **OBS-FIRE-LOOP** discipline (re-derive the
   world borrow per loop turn — the 14a F2 Tree-Borrows lesson; observers/dispatch.rs).

## Open design questions for the architect
1. **THE fork — synchronous vs deferred `Children` mutation.** Bevy hooks mutate the target collection
   synchronously via a mutable `DeferredWorld` (`collection_mut_risky`); boyko's `DeferredEcsMaster` is
   **read-only for components** (deferred_master.rs, Q-A2 deliberately withholds `get_component_mut`).
   Options: **(a)** route `Children` updates through the deferred command queue (simpler, sound;
   `Children` consistent only post-drain — a semantic boyko already accepts for observer-driven
   mutation); **(b)** extend the deferred view with a narrow, soundness-reviewed `children_mut`-style
   accessor (an ECS-core change; needs its own Tree-Borrows review). Biggest decision.
2. **Hooks vs observers for the invariant.** The invariant is intrinsic + type-level → `ComponentHooks`
   (process-global, fn-ptr, `#[component(...)]` derive) fits Bevy exactly; leave runtime `Observers`
   for user-level reactions. Confirm.
3. **`Children` representation.** `Vec<Entity>` (24 B header, allocates on first child) vs an
   inline-small variant (alloc-free for few-children, larger inline footprint in archetype storage).
4. **Cascade default vs opt-in.** Bevy 0.16 made recursive despawn the default (`LINKED_SPAWN`). Default
   = ergonomic but a sharper footgun; opt-in (`despawn_recursive`) = safer. Pick.
5. **Reparent atomicity / despawn-ordering (#20106)** — confirm command ordering + pre-remove read as
   binding invariants + tests.
6. **Parent-child-only vs the generic `Relationship`/`RelationshipTarget` trait** (parent-child as the
   first instance — more work now, every future relation free). boyko's 14b substrate supports generic.

## Sources
- Bevy 0.16 `Relationship`/`RelationshipTarget`: docs.rs/bevy/0.16.1 + `bevy_ecs/src/relationship/mod.rs`,
  `relationship_source_collection.rs`, `hierarchy.rs`; 0.15→0.16 migration guide.
- Bevy pitfalls: issues #20106 (collection removed before observers), #17883 (stale descendant despawn),
  #5584 (dangling parent); PR #17840 + issue #4697 (parallel transform propagation).
- flecs Relationships + ComponentTraits (cleanup `(OnDeleteTarget, Delete)`).
- EnTT intrusive-list hierarchy (skypjack ECS-baf part 4).
- Unity DOTS `Parent`/`Child`/`PreviousParent` + `ParentSystem`/`LocalToWorldSystem`.
