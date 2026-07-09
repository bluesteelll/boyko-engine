# Remaining gaps — toward an industrial "ultimate" ECS

Forward-looking gap list (the old `PHASE-13-ROADMAP.md` is all-DONE). Snapshot
updated 2026-06-23, branch `ecs`. Several items previously listed here have since
SHIPPED and are confirmed present by a source probe: `Option<&T>`/`AnyOf` query
data, `EntityCloner`/Prefab cloning, `RequiredComponents` (`#[require]`),
serialization (separate `boyko_serialize` crate, S0–S3), entity-targeted observers
+ custom propagating `Trigger`s (Feature 2: `observers/{entity_store,trigger,
traversal,propagate}`), and a **general `Relationship` trait** (`Relationship` /
`RelationshipTarget` + `#[derive(...)]`, with `ChildOf`/`Children` refactored onto
it). The genuine remaining kernel absences are `.pipe()`, `Reflect`, and
runtime-layout dynamic components.

## Done (so the list is honest about scope)

Full ECS core + scheduler + the entire feature line is landed:
- Storage: archetype + chunked `ComponentPool` (per-pool VM reservations, X.I);
  **two tag backends** — archetype-signature tags (Phase 22) + **enable-bit /
  bitset tags** (EnableTag: `#[component(storage="bitset")]`, O(1) non-fragmenting
  toggle).
- Queries: typed `Query<D, F>`, `Or`, `With`/`Without`, change detection
  (`Added`/`Changed`/`Ref`/`Mut`), `Enabled`/`Disabled`, dynamic terms,
  `par_iter`, `for_each_chunk`, direct-API `QueryView`.
- Scheduler: Bevy-class parallel (Chase-Lev work-stealing, conflict graph,
  Tarjan/Kahn), sets/ordering (15), run conditions (16/16.1), states (17),
  plugins/App (18), fixed timestep + multi-schedule (20/20.1).
- Commands/EntityCommands, bundles (+ D4 typed `write_row_typed` batch write),
  events as SystemParam, resources, `Local`, `IntoSystem`, lifecycle hooks (14a)
  + component-level observers (14b), hierarchies `ChildOf`/`Children` (19).
- Perf vs Bevy 0.18.1: scheduler 1.6×, par_iter 3.4×, query-iter parity,
  **spawn_batch g5 — cold parity / warm 1.58× (D4, `2ff8818`)**.

## Remaining gaps (prioritized)

### P0 — foundational, unlock multiple downstream features
- **Reflection / runtime type registry — OPTIONAL / tooling-driven, NOT a perf
  risk.** No field-level type introspection (Bevy `bevy_reflect`). Enables
  editors, generic serialization, scripting, runtime inspection. **Perf note
  (a real concern, answered):** reflection is a BOUNDARY feature — the ECS hot
  path (query iter / spawn / schedule) is statically monomorphized and never
  touches it; reflection is invoked only at serialize/inspect/script boundaries
  (cold, infrequent). Designed Bevy/flecs-style — separate crate, opt-in
  `#[derive(Reflect)]`, `static` type-info tables, HARD-forbidden on the ECS hot
  path (same discipline as 14a hooks) → **0% hot-path cost**. It is **only worth
  doing IF an editor / generic serialization / scripting is a goal**; EnTT (the
  perf-king ECS) ships WITHOUT reflection, and save-load can use a manual
  per-component serializer without it. So: P0 *conditionally* (tooling), else
  skippable for a lean perf ECS.
- **General Relations — SHIPPED (generic `Relationship` API).** A first-class
  `Relationship` / `RelationshipTarget` trait pair + `#[derive(Relationship)]` /
  `#[derive(RelationshipTarget)]` lets any typed entity↔entity relation be declared
  one-shot, on the **non-flecs, hook-maintained** model (a foreign-key component +
  a reverse-collection maintained reactively — NO archetype-pair fragmentation,
  unlike flecs). `ChildOf`/`Children` were REFACTORED onto this generic machinery
  (the hand-written hooks deleted), so the hardened Phase-19 suite is the generic
  machinery's regression gate. Custom propagating triggers bubble along ANY
  relation via the generic `Traversal`/`Toward<R>` seam; deep-clone + serialize
  entity-remap are generic. v1 = `Vec<Entity>` one-to-many, `RETAIN_EMPTY=true`;
  **v1.1 (1:1 `Exclusive` collection + eviction) — SHIPPED** (`2f47973`); only
  remove-on-empty is deferred (v1.2). **Relation-aware
  QUERY + OBSERVER DSL — SHIPPED** (`147446e` query side, `e3fa9a5` observer side):
  the `Related<R, D>` read-only join term (sequential-only, par_iter-rejected, usable
  as a `Query` SystemParam; aliasing build-panic via the existing conflict detector),
  `HasRelation`/`NoRelation`/`RelatedTo` filters (the last via the new value-carrying
  `query_filtered` entry), transitive accessors `targets`/`sources`/`ancestors`/
  `descendants` (depth-capped + `!ACYCLIC` visited guard), `OnLink<R>`/`OnUnlink<R>`
  edge observers (fired on the committed edge at the apply window), and `Broadcast<R>`
  downward trigger propagation (per-node prune). Landing it also fixed two pre-existing
  deep-clone reverse-index bugs (BUG-EDGE-CLONE-1/2). The flecs-style wildcard *pair*
  term (`(Likes, *)` yielding each pair) is intentionally not added — the `sources`/
  `targets` accessors + the join cover the non-fragmenting model. 1:1 collections
  (v1.1) are now SHIPPED too — the only deferred relation work is remove-on-empty
  (v1.2), a minor convenience (an emptied target keeps its empty reverse component).

### P1 — modern ECS ergonomics / storage
- ~~**Required components.**~~ SHIPPED (`#[require]`, component A auto-inserts B, C).
- ~~**Entity cloning / `EntityCloner`.**~~ SHIPPED (deep/shallow clone + Prefab; the
  `clone/` module, now generalized to remap any `Relationship` FK).
- **Sparse-set / hybrid component storage.** Currently archetype-only for real
  components (+ enable-bit for tags). For add/remove-heavy components a sparse-set
  backend avoids migration. (Roadmap previously deferred "indefinitely"; revisit
  if churny components appear.)
- **Full dynamic components (runtime layout).** Only dynamic *tags* (name-keyed
  ZST) exist; no runtime-registered components with arbitrary layout + get/set by
  id (needed for scripting / data-driven content).
- **Entity-targeted observers + custom triggers — SHIPPED (Feature 2).** Beyond the
  14b component-level observers: per-entity observers (`observers/entity_store`),
  custom propagating `Trigger`s (`pub trait Trigger { const AUTO_PROPAGATE; type
  Traversal; }`), and event bubbling along a relation (`observers/{traversal,
  propagate}`, `Toward<R>`) are built + tested (`feature2_observers_*`, Miri). What
  remains is only the smaller `on_despawn`-edge ergonomics noted in the phase
  residuals.

### P2 — separate crates / niceties (NOT core ECS)
- **Serialization / Save-Load — DEFERRED to a SEPARATE CRATE.** (User decision
  2026-06-15: this is a different system, not core ECS — make it its own crate
  later.) Needs Reflection (P0) first for a generic solution; a manual
  per-component (de)serializer is possible sooner. The fast-load memory suite
  (task #8: page-recycling on world drop / `world.reserve(n)` prewarm / huge
  pages) was motivated by save-load speed and is **parked with this** — it is
  core-memory work, independently usable for level transitions / streaming, but
  deprioritized with saves per the user.
- **System piping (`.pipe()`)**, one-shot-system ergonomics.
- **Asset system, networking / replication** — separate crates, out of core ECS.
- Misc deferred-by-phase: `init_resource`/`FromWorld`, SubApps/render-world,
  `PluginGroup`/`DefaultPlugins`, computed/sub-states, `#[derive(States)]`,
  value-keyed sub-schedules.

## Notes
- `g2` SystemParam query iter (~1.15× of Bevy) is a documented **bench artifact**
  (bare iteration is byte-identical to Bevy in asm; `black_box`-per-element blocks
  SIMD for both) — a parity floor, not a real engine deficit. Not a gap to chase.
- EnableTag follow-up **positive-term archetype cull** (task #5) is a perf
  refinement (skip non-present-A archetypes for `Query<&D, Enabled<A>>`), not a
  feature gap.
