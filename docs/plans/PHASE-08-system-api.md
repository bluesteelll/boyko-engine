# Phase 8 — System API: typed SystemParam + query DSL

**Status:** ⚪ DRAFT — design exploration only. Architect → critic
cycle has **not** started. Implementation depends on Phase 7
completion.
**Branch (when active):** `ecs`.

## Goal

Give users an ergonomic, type-safe surface for writing per-frame
systems against `boyko_ecs`. Match Bevy's `Query<(&A, &B), With<C>>`
ergonomics in feel, but with zero virtual dispatch in the hot path
and zero allocation per `World::run`.

This is the phase the user originally asked for (*"максимально
красивое удобное в плане написания кода но и максимально
производительное API для systems"*). It was paused so Phase 7 could
make the random-access primitive fast enough that the API isn't just
nice syntax over a slow inner loop.

## Why after Phase 7

Documented in
[`feedback-foundations-before-apis`](../../../../../Users/flint/.claude/projects/D--claude-BoykoEngine/memory/feedback-foundations-before-apis.md):
the user explicitly redirected the design from "system API first" to
"foundation first" after seeing that `get_component_raw` was ~40 ns
through 9 cache lines. Phase 7 brings the primitive down to
~12-16 ns; Phase 8 builds the surface that relies on that primitive.

## Reference points

Researcher should compare against:

- **Bevy** — `SystemParam`, `Query<D, F>`, `World::run_system`,
  function-as-system via tuple-impl `IntoSystem`. Variadic via macro.
- **flecs** — C-level entity query iterators with cached column
  pointers; closest to our Phase 7 layout.
- **EnTT** — `view<Components...>(exclude<...>)` template-heavy API
  (Rust analogue is generic struct + trait).
- **Unity DOTS** — `IJobEntity`, source-generators for system code;
  parallel-by-default.
- **specs / hecs** — older Rust ECS surfaces, less relevant but
  illustrate alternative ergonomics.

## Draft scope — what Phase 8 must deliver

### 8a — `SystemParam` trait

A trait every system parameter implements:

```rust
pub unsafe trait SystemParam: Sized {
    type State;     // long-lived: archetype matches, channel handles
    type Item<'w>;  // short-lived: actual borrow into World

    fn init_state(world: &mut World) -> Self::State;
    unsafe fn fetch<'w>(state: &'w mut Self::State, world: UnsafeWorldCell<'w>) -> Self::Item<'w>;
    fn check_access(state: &Self::State, access: &mut Access);  // for scheduler
}
```

Built-in implementations:

- `Query<D, F>` — the central one.
- `Res<T>` / `ResMut<T>` — global resources (Phase 8 needs the
  resource subsystem too; see 8b).
- `EventReader<E>` / `EventWriter<E>` — thin wrappers over the
  Phase 6 dispatcher.
- `Commands` — deferred mutations (entity spawn / despawn) executed
  between systems.
- `Local<T>` — per-system persistent state.

### 8b — Resource subsystem

A new mini-subsystem (independent crate-internal module) for
`World`-global singletons:

- `Resources` — `[OnceCell<Box<dyn Any>>; MAX_RESOURCES]` or
  index-array keyed by `TypeId → ResourceId` (the
  Phase 4a newtype pattern reapplied).
- `Res<T>` borrows shared; `ResMut<T>` borrows exclusive.
- Scheduler integration (Phase 9): two systems requesting `ResMut<T>`
  on the same `T` must serialise.

### 8c — `Query<D, F>` typed DSL

The headline feature:

```rust
fn movement(mut query: Query<(&mut Position, &Velocity), Without<Frozen>>) {
    for (mut pos, vel) in &mut query {
        pos.x += vel.x;
        pos.y += vel.y;
    }
}
```

Components:

- `D: QueryData` — `(&A, &mut B, …)` tuple trait.
- `F: QueryFilter` — `With<C>`, `Without<C>`, `Or<(A, B)>`,
  `Changed<C>` (deferred).
- `QueryIter` — pointer-bump cursors driven by the Phase 5c
  dual-generation `QueryState` cache and the Phase 7 inline
  column table.

Internal mapping per row:

```text
QueryIter::next()
  → state.matched_ids().next() advances to next archetype
  → for the current archetype, columns[A], columns[B], columns[C]
  → pointer-bump per row; arity-N tuple yielded
  → on archetype exhaustion, advance to next matched archetype
```

This is essentially Phase 2d generalised to arbitrary arities and
mutability.

### 8d — `IntoSystem` + function-as-system

Match Bevy's tuple-impl pattern so a plain `fn(query, res, events)`
becomes a system without manual trait impl:

```rust
impl<F, P0, ..., Pn> IntoSystem<(P0, ..., Pn)> for F
where F: FnMut(P0, ..., Pn) + 'static,
      P0..Pn: SystemParam, ...
```

Variadic via macro expansion. Code-bloat-budget kept in check by
inlining the dispatch but keeping the body cold.

### 8e — `Commands` buffer

Deferred mutations. Standard Bevy pattern:

```rust
fn spawn_things(mut commands: Commands) {
    commands.spawn((Position { .. }, Velocity { .. }));
    commands.despawn(some_entity);
}
```

Flushed between systems by the scheduler (Phase 9). Single-threaded
flush is fine; the buffer is per-system, lock-free.

## High-level open questions (require architect cycle)

| Q | Decision needed | Notes |
|---|-----------------|-------|
| Q-8.1 | Single `Query<D, F>` struct vs separate `QueryRef<…>` / `QueryMut<…>`? | Bevy chose single struct with internal aliasing tracking. Pro: one API. Con: complex aliasing dance. |
| Q-8.2 | Variadic via macro or `T: QueryData` tuple trait? | Bevy: tuple trait with macro-emitted impls up to 15. Likely the right pattern. |
| Q-8.3 | `Changed<T>` / `Added<T>` change detection — Phase 8 or Phase 10? | Bevy ties change detection to a per-component `Tick` register. Significant memory cost. Defer. |
| Q-8.4 | `Commands` — flush eagerly per system or accumulated until frame end? | Bevy flushes between systems. Phase 9 scheduler decides. |
| Q-8.5 | `Local<T>` storage — per-system `Box<dyn Any>` or strongly typed in the system's `State`? | Bevy: `Box<dyn Any>` keyed by `TypeId`. Acceptable cold-path overhead. |
| Q-8.6 | `Res<T>` / `ResMut<T>` — single global `Resources` or per-`World` like Bevy? | We have one `EcsMaster`; per-`World` distinction collapses. |
| Q-8.7 | Should `&mut Query<…>` carry an exclusive lifetime to `EcsMaster`, or does the scheduler enforce non-overlap statically? | Phase 9 dictates this. Likely scheduler-enforced. |

## Performance targets

Cold-path (system registration, query state init): no budget.

Hot-path (system body execution, per frame):

| Operation | Target |
|-----------|--------|
| Empty system call (no params) | ≤ 5 ns dispatch overhead |
| `Res<T>` access in a system body | ≤ 5 ns (just a borrow) |
| `Query<&A>::iter().next()` per row | parity with Phase 2d `iter_one` (~5 ns) |
| `Query<(&A, &B)>::iter().next()` per row | parity with Phase 2d `iter_two` (~7-8 ns) |
| `EventReader<E>::read()` per event | ≤ 3 ns (parity with Phase 6 drain) |
| `Commands::spawn` enqueue | ≤ 20 ns (one push to per-system buffer) |
| `Commands` flush per command | ≤ 200 ns (calls `create_entity` once) |

## Out-of-scope risks

- **Trait-method indirection bloat** — every `SystemParam` impl
  pulls a per-monomorphisation copy of `fetch`. Watch I-cache.
- **Macro-emitted variadic impls bloat compile times** — Bevy limits
  to 15-arity. We should pick a number ≤ 8 unless measured otherwise.
- **`Box<dyn Any>` in `Local<T>` and `Resources`** — both are cold
  path, but make sure no `dyn Trait` leaks into the per-frame inner
  loop.

## What this phase does NOT do

- It does **not** introduce a scheduler — that is Phase 9. Phase 8
  systems are runnable one-at-a-time via `World::run_system_once`.
- It does **not** add parallel execution — that's Phase 9.
- It does **not** add change detection — Phase 10.
- It does **not** stabilise the public API.

## Cross-phase dependencies

- **Phase 7** must be complete — `get_component_raw` at target
  speed.
- **Phase 6** is consumed for `EventReader` / `EventWriter`.
- **Phase 5c** `QueryState` dual-generation cache is reused.
- **Phase 4a** newtype IDs flow into typed access.

## Estimated phasing

- **8a (SystemParam trait + Resource subsystem)** — 1 architect
  cycle, 2 developer sessions.
- **8b (Query<D, F> DSL)** — biggest sub-phase; 2 architect cycles,
  3-4 developer sessions. Variadic macro emission alone is a half
  session.
- **8c (IntoSystem + Commands)** — 1 architect cycle, 2 developer
  sessions.
- **8d (benches + tests)** — 1 developer session, 1 tester session,
  1 results-analyst.

Total estimate when launched: 6-8 sessions end-to-end.

## How to launch

When Phase 7 lands and the user gives explicit go-ahead:

1. Dispatch `researcher` to compare Bevy / flecs / EnTT system
   APIs in depth — produce a comparative table of trait shapes,
   variadic strategies, change-detection costs.
2. Dispatch `architect` for sub-phase 8a (smallest, foundational).
3. Cycle `architecture-critic`.
4. Repeat per sub-phase 8b / 8c / 8d.
5. Only after the full plan is approved do we move to `developer`.

## References

- User original ask (saved feedback memory):
  [`feedback-foundations-before-apis`](../../../../../Users/flint/.claude/projects/D--claude-BoykoEngine/memory/feedback-foundations-before-apis.md).
- Bevy reference: <https://bevyengine.org/learn/book/getting-started/ecs/>
  (book) + `bevy_ecs::system::Query` (rustdoc).
- flecs: <https://www.flecs.dev/flecs/md_docs_2Queries.html>.

## Plan template (for future phase files)

Copy this skeleton when adding a new `PHASE-NN-topic.md`:

```markdown
# Phase NN — <Title>

**Status:** ⚪ DRAFT / 🟡 PLANNED / 🟢 IN PROGRESS / ✅ DONE
**Branch:** `ecs`
**Detailed plan:** (link if any)
**Audit IDs touched:** (list)

## Goal — what success looks like

## Why now / why after PHASE-N

## High-level design (max 5 load-bearing decisions)

## N-step implementation plan / checklist

### Step 0 — <name>
**Files:**
**Action:**
**Acceptance:**

(repeat per step)

## Critical SAFETY contracts

## Risks and mitigations

## Out of scope (deferred to later phases)

## Cross-phase dependencies

## How to launch implementation

## References
```
