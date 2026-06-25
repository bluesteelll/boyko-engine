# Change Detection

Boyko's change-detection system lets a system observe **which entities had a
particular component added or modified since the system last ran**. It is
modelled after [Bevy ECS](https://bevyengine.org/) (post-PR #6547) and ships
with the following surface:

- A monotonic `Tick(u32)` counter, advanced ~2 ticks per `Schedule::run`.
- Per-row `added` and `changed` ticks stored alongside every component.
- The filters `Added<T>` and `Changed<T>` (non-archetypal, composable with
  `Or`, `With`, `Without`).
- The system parameters `Ref<T>` and `Mut<T>` for opt-in tick introspection
  on read / write paths.
- Escape hatches: `set_if_neq`, `bypass_change_detection`.
- Wraparound safety via `MAX_CHANGE_AGE` clamping (runs at most ~once per
  50 days of continuous play at 60 FPS).

If your systems do not use any of these, **Phase 10 adds zero overhead** to
their hot paths. The compiler's const-fold elides every per-row branch
that the change-detection filters would have inserted.

---

## 1. The Tick clock

At the heart of change detection is a single global counter:

```rust,ignore
// (internal field; access via the public getter)
pub fn current_tick(&self) -> Tick;
```

Each call to `Schedule::run` advances this counter with **two** atomic
`fetch_add(1, Relaxed)` bumps:

- A **frame-start bump**
  ([`schedule.rs:286`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/core/schedule/schedule.rs#L286))
  that publishes the new `this_run` read by every system, condition, and the
  state pass.
- An **apply-window bump** (Bug #56,
  [`schedule.rs:381`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/core/schedule/schedule.rs#L381))
  that lands deferred-command stamps at `this_run + 1`, strictly between this
  run's reader window and the next's.

So a plain run costs ~2 atomics; a fixed-timestep run with substeps costs a
few more. Every system about to dispatch records two snapshots:

- `this_run` — the world's tick captured by the frame-start bump.
- `last_run` — the system's `this_run` from its **previous run**.

Both snapshots are stored on `SystemMeta` and read by filters and `Ref` /
`Mut` accessors. The combination `(last_run, this_run]` is the **observation
window** for the system.

Boyko bumps `change_tick` **per `Schedule::run`** (per-run regime), rather
than once per system (Bevy's per-system regime). Practical consequences:

- All systems in one run share the same frame-start `this_run`.
- A handful of atomics per run (instead of `N` atomics for `N` systems).
- `Added<T>` granularity is **run-level** — the resolution remains
  sufficient for the conventional fixed-tick game loop.

---

## 2. The `Added<T>` filter

`Added<T>` matches rows whose component `T` was inserted into this archetype
within the system's observation window:

```rust,ignore
fn react_to_new_enemies(
    q: Query<&Enemy, Added<Enemy>>,
) {
    for e in &q {
        // Triggers once, the first frame `e` exists.
        println!("Spawned: {:?}", e.kind);
    }
}
```

Semantics:

- **First frame after spawn** — `Added<T>` matches.
- **Every subsequent frame** — `Added<T>` does NOT match (the row's
  `added_tick` is now older than `last_run`).
- Archetype migration (component-add to an existing entity) — `added_tick`
  resets in the destination archetype; treats the migration like a spawn.

`Added<T>` declares a **read** of `T` against the scheduler's conflict
graph. Systems can run in parallel with any other reader of `T` but
serialise behind a writer of `T`.

---

## 3. The `Changed<T>` filter

`Changed<T>` matches rows whose `changed_tick` is inside the window. The
tick is bumped whenever:

- The row is inserted (`changed = current_tick` at insert time).
- The system calls `Mut<T>::deref_mut` on the row.
- The system calls `Mut<T>::set_if_neq` AND the value changed.

```rust,ignore
fn recompute_world_state(
    q: Query<&Transform, Changed<Transform>>,
) {
    for t in &q {
        // Runs only for entities whose Transform was modified
        // since this system's last frame.
        update_world(t);
    }
}
```

`Changed<T>` is the **Bevy deref-bump semantic**: a `Mut<T>::deref_mut` is
considered a change even if the user-written value happens to be equal to
the previous one. To avoid that false positive, use `set_if_neq` (see §5).

---

## 4. Composing with `Or`, `With`, `Without`

`Added<T>` and `Changed<T>` are normal `QueryFilter` implementors and
compose with the rest of the DSL:

```rust,ignore
// Match rows where either Position changed OR Velocity was added.
fn react_to_motion(
    q: Query<&Transform, Or<(Changed<Position>, Added<Velocity>)>>,
) { /* ... */ }

// Match rows that have Tag AND whose Health changed.
fn process_tagged_changes(
    q: Query<&Health, (With<Tag>, Changed<Health>)>,
) { /* ... */ }
```

### Or composition caveat — null-base path

`Or<F>::aggregate_include` is a no-op, so an `Or` that includes a
`Changed<T>` arm will **walk every archetype** — including those that don't
contain `T`. The non-existent `T` column makes `Changed<T>::filter_fetch`
fall into a "null-base" branch that returns `false`. The other arms of the
`Or` may still succeed. The cost: roughly **0.5 ns of branch overhead per
row** on the null-base path.

If you only ever query specific archetypes, consider tightening the filter:

```rust,ignore
// Cheaper: post-filter via the archetypal With<T> arm first.
Query<&Health, (With<Health>, Or<(Added<Health>, Changed<Health>)>)>
```

---

## 5. `Mut<T>` — the write guard

`Mut<T>` is the SystemParam path for **change-tracked writes**. It wraps a
`&mut T` plus pointers to the row's tick slots:

```rust,ignore
fn apply_damage(
    mut q: Query<Mut<Health>>,
) {
    for mut h in &mut q {
        h.hp -= 10; // DerefMut triggers the tick bump.
    }
}
```

### Tick bump semantics

- Calling `*h` (`Deref`) — **does not** bump.
- Calling `*h = ...` or `&mut *h` (`DerefMut`) — **bumps** the changed tick.
- The bump is idempotent within a single `Mut<T>` guard: the first
  `DerefMut` writes `changed_tick = this_run`; subsequent calls on the
  same guard skip the store (a micro-optimisation).

### `set_if_neq`

`set_if_neq` provides Bevy's value-compare opt-in: only bump the tick when
the new value differs:

```rust,ignore
fn idle_assign(mut q: Query<Mut<Score>>) {
    for mut s in &mut q {
        // If the score is already 100, this is a no-op.
        s.set_if_neq(Score(100));
    }
}
```

Requires `T: PartialEq`. Returns `bool` — `true` iff the write happened.

### `bypass_change_detection`

`bypass_change_detection` returns a `&mut T` that does NOT bump the tick:

```rust,ignore
fn rebuild_in_place(mut q: Query<Mut<Buffer>>) {
    for mut b in &mut q {
        // Caller knows the rebuild is semantically a no-op.
        b.bypass_change_detection().rebuild_internal_indices();
    }
}
```

Use sparingly — invisible to downstream `Changed<T>` filters until the
next legitimate write.

The `Mut<T>` surface is exactly these four methods plus `Deref` / `DerefMut`
— see the impl block at
[`data.rs:1346`](https://github.com/bluesteelll/boyko-engine/blob/ecs/crates/boyko_ecs/src/ecs/core/iters/query/data.rs#L1346).
To forward to an API that takes `&mut T`, pass `bypass_change_detection()`
(no bump) or `&mut *guard` (`DerefMut`, bumps once).

---

## 6. `Ref<T>` — the read view with ticks

`Ref<T>` exposes the row's tick info **without forcing a filter**. Use it
when a system wants to know whether a value was added or changed and read
the underlying value in the same pass:

```rust,ignore
fn observe(
    q: Query<Ref<Health>>,
) {
    for h in &q {
        if h.is_added() {
            // Newly inserted this frame.
        }
        if h.is_changed() {
            // Mutated since the system last ran.
        }
        let value: u32 = h.hp;  // Deref to &Health.
    }
}
```

`Ref<T>` and `Mut<T>` use the **inclusive lower-bound** semantic: writes
performed by the SAME system in the current frame report as `is_changed`.
This matches Bevy's documented behaviour and keeps a "react-to-my-own-write"
pattern feasible inside a single system.

---

## 7. Wraparound and `MAX_CHANGE_AGE`

`Tick` is a `u32`. Comparison via `is_newer_than` uses **wrapping
subtraction**; the result is correct only when the relative ages of stored
ticks stay bounded below `2^31`. To keep the bound, the scheduler
periodically clamps every stored tick to `current - MAX_CHANGE_AGE` on a
cold-path **`check_ticks` scan**.

Constants:

- `CHECK_TICK_THRESHOLD = 518_400_000` — ticks between scans (Bevy mirror).
- `MAX_CHANGE_AGE = u32::MAX - (2 * CHECK_TICK_THRESHOLD - 1)` ≈ 3.26 B.

At 60 FPS the world advances ~2 ticks per run (the frame-start bump plus the
Bug #56 apply-window bump; see §1), so the threshold maps to roughly half the
naive single-bump figure:

| Metric | Value |
| --- | --- |
| Ticks per run | ~2 (more under fixed-timestep substeps) |
| Time between scans | ~50 days of continuous play at 60 FPS |
| Scan cost (100 k entities × 50 components) | ~3 ms cold |

Effectively, a player who runs your game non-stop for under a month and a
half will never observe a `check_ticks` scan. The clamp is a safety net, not
a hot path. You do not need to think about it.

---

## 8. Putting it together

A representative system that combines all three patterns:

```rust,ignore
use boyko_ecs::ecs::core::iters::query::{Added, Changed, Mut, Query, Ref, With};

fn integration_pipeline(
    spawned:   Query<&Transform, Added<Transform>>,
    movers:    Query<(Ref<Transform>, Mut<Velocity>), Changed<Transform>>,
    untracked: Query<&Material, With<RenderTag>>,
) {
    for t in &spawned {
        // Trigger once per new entity.
        seed_pathfinder(t);
    }

    for (t, mut v) in &mut movers {
        // Read the transform with tick info; recompute velocity.
        if t.is_changed() {
            v.target = compute_target(&t);
        }
    }

    for m in &untracked {
        // Phase 10 contributes 0 ns of overhead to this query.
        render_material(m);
    }
}
```

### How the scheduler frames it

- The run starts with the frame-start bump: `change_tick.fetch_add(1, Relaxed)`,
  capturing `this_run`.
- Each system about to dispatch runs `set_change_ticks(last_run, this_run)`,
  where the system's previous `this_run` becomes its new `last_run`.
- The apply-window bump (Bug #56) fires once more before deferred commands
  drain, so deferred-added components land at `this_run + 1`.
- Workers read `&SystemMeta` (shared) inside their tasks.
- `Mut<T>::deref_mut` writes the changed tick via `UnsafeCell<Tick>` — no
  atomic; the Phase 9 conflict graph guarantees no concurrent reader.
- `par_iter` workers write to disjoint slots in the same cache line; this
  is sound by the Rust abstract machine even though the lines see MESI
  ping-pong (false sharing). Boyko avoids regressions here via dedicated
  Miri coverage (`miri_par_iter_chunks_write_adjacent_ticks_disjoint_no_ub`).

---

## 9. Performance summary

| Operation | Target | Notes |
| --- | --- | --- |
| `Tick::is_newer_than` | ≤ 1 ns | 2 × `wrapping_sub` + cmp |
| `Changed<T>` filter, hot row | ≤ 1 ns/row | autovectorisable predicate |
| `Or<(_, Changed<T>)>`, null-base | ≤ 1.5 ns/row | branch + return false |
| `Mut<T>::deref_mut` bump | ≤ 1 ns | single u32 store |
| Per-run tick bumps | ≤ 5 ns each | ~2 `fetch_add(Relaxed)` per run |
| `check_ticks` scan, 100 k × 50 components | ≤ 10 ms cold | runs ~once per 50 days |
| `Query::iter` overhead without change detection | 0 ns | const-fold elision |
| Phase 9 dispatcher regression budget | 0 % | rides existing exclusivity |

The numbers are validated by the criterion bench suite
(`phase10_change_detection`); see `crates/boyko_ecs/benches/phase10_change_detection.rs`.

---

## 10. Quick reference

- **Filter a query for new entities**: `Query<&T, Added<T>>`.
- **Filter a query for modified entities**: `Query<&T, Changed<T>>`.
- **Read a component with tick info**: `Query<Ref<T>>` → `t.is_added()`,
  `t.is_changed()`.
- **Write a component and track the change**: `Query<Mut<T>>` →
  `*t = ...` (bumps) or `t.set_if_neq(...)` (bumps only on inequality).
- **Skip the tick bump**: `t.bypass_change_detection()`.

> A `SystemChangeTick` SystemParam that exposes a system's `this_run()` /
> `last_run()` snapshot directly is planned (Wave B+) but **not yet shipped**.
> Until then, read tick state through `Ref<T>` / `Mut<T>` accessors.

For the design rationale and full invariant catalogue, see
[`docs/PHASE-10-CHANGE-DETECTION-PLAN.md`](https://github.com/bluesteelll/boyko-engine/blob/ecs/docs/PHASE-10-CHANGE-DETECTION-PLAN.md).
