# Remaining gaps — toward an industrial "ultimate" ECS

Forward-looking gap list (the old `PHASE-13-ROADMAP.md` is all-DONE). Snapshot
2026-06-15, branch `ecs`. Each "missing" item was confirmed absent by a source
probe (no `Option<&T>` query impl, no `EntityCloner`, no `serde`/serialization,
no `.pipe()`, no general `Relationship` trait, no `Reflect`, no runtime-layout
dynamic components, no `RequiredComponents`).

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
- **General Relations.** Only the hardcoded `ChildOf`/`Children` exists. flecs'
  headline feature — arbitrary entity↔entity relationships with query traversal
  (`(Likes, *)`, transitive, exclusive). Currently each relation would be
  hand-written like Phase 19.
- **Optional query data `Option<&T>` (+ `AnyOf`).** No `impl QueryData for
  Option<&T>`. A common, expected query ergonomic.

### P1 — modern ECS ergonomics / storage
- **Required components.** Bevy 0.15 — component A auto-inserts its deps B, C.
- **Entity cloning / `EntityCloner`.** Bevy 0.16 — deep/shallow copy an entity.
- **Sparse-set / hybrid component storage.** Currently archetype-only for real
  components (+ enable-bit for tags). For add/remove-heavy components a sparse-set
  backend avoids migration. (Roadmap previously deferred "indefinitely"; revisit
  if churny components appear.)
- **Full dynamic components (runtime layout).** Only dynamic *tags* (name-keyed
  ZST) exist; no runtime-registered components with arbitrary layout + get/set by
  id (needed for scripting / data-driven content).
- **Entity-targeted observers + `on_despawn`.** 14b shipped component-level
  observers; full Bevy-style entity-targeted triggers, custom `Trigger` events,
  propagation, and entity-level `on_despawn` remain.

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
