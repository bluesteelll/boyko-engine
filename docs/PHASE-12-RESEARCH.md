# Research: Events as SystemParam (Phase 12 — `EventReader<E>` / `EventWriter<E>`)

## §1 Executive summary

The state-of-the-art reference is **Bevy** — its `EventReader<'w, 's, E>` and `EventWriter<'w, E>` are SystemParam wrappers around an `Events<E>` resource that holds a double-buffered event sequence and a monotonically increasing `event_count`. Reader cursors are stored as `Local<EventCursor<E>>` per system (per-system state slot), storing only `last_event_count: usize`. Frame skipping is achieved by comparing the cursor's count to each buffer's `start_event_count`.

Key directional conclusions for boyko:

1. **EventId caching in State** is the unambiguous win. Bevy does it (via the typed `Res<Events<E>>` lookup). Boyko's `Phase 6 OnceLock<EventId>` cost is paid once at `init_state`, eliminating ~2-3 ns from every `send` / `read`.
2. **Reader cursor shape**: Bevy stores `last_event_count: usize` per `(system, event-type)` pair as `Local` state. Boyko already has a per-system `State` slot in `SystemParam` machinery — no new infrastructure needed. The boyko reader-buffer model is *single-buffer-after-swap* (different from Bevy's *two-buffer-overlap*), which simplifies cursor math significantly.
3. **Conflict graph integration**: Bevy serializes `EventReader<E>` ↔ `EventWriter<E>` for the same type by routing both through `Res/ResMut<Events<E>>`. Boyko's Phase 9 EVT1 per-lane discipline already provides correctness for `EventWriter`; only `EventReader<E>` ↔ `update_events` needs synchronization, and that already runs single-threaded between frames.
4. **Tick integration is optional** for boyko. The double-buffer's `update_events` semantics already drop events older than one frame. Bevy keeps `event_count` because its readers can hold cursors across multiple frames; boyko could choose simpler "this-frame-only" semantics or full Bevy parity.
5. **No flecs observer pattern**. Push-based observers do not fit boyko's pull-based filter discipline; they would re-introduce dyn dispatch and would not benefit from the scheduler's batching.

---

## §2 Bevy EventReader/EventWriter deep dive

### 2.1 `Events<E>` resource (Bevy 0.16, `collections.rs`)

```rust
pub struct Events<E: Event> {
    pub(crate) events_a: EventSequence<E>,   // older buffer
    pub(crate) events_b: EventSequence<E>,   // newer buffer
    pub(crate) event_count: usize,           // global monotonic counter
}
```

Each `EventSequence` is itself `Vec<EventInstance<E>>` plus a `start_event_count: usize` recording the global index of its first element. `EventInstance` carries `EventId<E>` but **no tick** — Bevy explicitly does not put change-ticks on events.

`Events::send` pushes to `events_b` and increments `event_count`. `Events::update` does:

```rust
pub fn update(&mut self) {
    core::mem::swap(&mut self.events_a, &mut self.events_b);
    self.events_b.clear();
}
```

This is **two-buffer overlap** (events stay live for at least one full frame even if produced near a swap), in contrast to boyko's *write-only-then-read-only* model where the writer lanes are zeroed at swap and only `reader_buf` is visible to readers.

### 2.2 `EventReader<'w, 's, E>` (Bevy 0.16, `reader.rs`)

```rust
#[derive(SystemParam, Debug)]
pub struct EventReader<'w, 's, E: Event> {
    pub(super) reader: Local<'s, EventCursor<E>>,
    events: Res<'w, Events<E>>,
}

impl<E: Event> EventReader<'_, '_, E> {
    pub fn read(&mut self) -> EventIterator<'_, E> {
        self.reader.read(&self.events)
    }
}
```

The derive composes the param from `Local<EventCursor<E>>` + `Res<Events<E>>`. Conflict declaration falls out of those two sub-params automatically.

### 2.3 `EventCursor<E>` (Bevy 0.16, `event_cursor.rs`)

```rust
pub struct EventCursor<E: Event> {
    last_event_count: usize,
    _marker: PhantomData<E>,
}

impl<E: Event> EventCursor<E> {
    pub fn read_with_id<'a>(&'a mut self, events: &'a Events<E>)
        -> EventIteratorWithId<'a, E>
    { EventIteratorWithId::new(self, events) }

    pub fn len(&self, events: &Events<E>) -> usize {
        events.event_count
              .saturating_sub(self.last_event_count)
              .min(events.len())
    }
}
```

The cursor is *just* a `usize`. The `.min(events.len())` clamp handles the case where a reader missed two updates and its cursor is older than `events_a.start_event_count` (the "missed events" path).

### 2.4 `EventIteratorWithId::new` — buffer chaining

Per the docs.rs source, the constructor calculates per-buffer offsets from the cursor:

```rust
let a_index = reader.last_event_count
    .saturating_sub(events.events_a.start_event_count);
let b_index = reader.last_event_count
    .saturating_sub(events.events_b.start_event_count);
let a = events.events_a.get(a_index..).unwrap_or_default();
let b = events.events_b.get(b_index..).unwrap_or_default();
let unread = a.len() + b.len();
let chain = a.iter().chain(b.iter());
```

On every `next()`, the iterator pulls from `chain`, increments `reader.last_event_count`, and decrements `unread`.

### 2.5 `EventWriter<'w, E>` (Bevy 0.16, `writer.rs`)

```rust
#[derive(SystemParam)]
pub struct EventWriter<'w, E: Event> {
    events: ResMut<'w, Events<E>>,
}

impl<E: Event> EventWriter<'_, E> {
    pub fn write(&mut self, event: E) -> EventId<E> { self.events.send(event) }
    pub fn write_batch(&mut self, events: impl IntoIterator<Item = E>) { ... }
    pub fn write_default(&mut self) -> EventId<E> where E: Default { ... }
}
```

**Critical consequence**: because `EventWriter<E>` declares `ResMut<Events<E>>`, two systems with `EventWriter<E>` for the **same** `E` cannot run in parallel. Two readers can run in parallel with each other. A reader cannot run in parallel with a writer of the same type. This is documented behavior and an intentional simplification.

### 2.6 Event update scheduling (Bevy 0.16, `update.rs`)

`event_update_system` runs on a registry of all `Events<E>` resources and calls `update()` per registered type. It is gated by `event_update_condition` whose state machine is `Always | Ready | Waiting`. The system holds a `Local<Tick>` for `last_change_tick`. Note: the tick is used to gate *when* `update` runs (FixedUpdate semantics), **not** to mark individual events.

---

## §3 flecs observers comparison

flecs offers **observers** (push model) as the primary event mechanism, in addition to query terms (pull). From the flecs docs:

- An **observer** = query + callback. Whenever an event matching the query occurs, the callback fires immediately.
- Events propagate along relationship edges (push down to children) and forward across relationship pairs (pull up from targets when a relationship is added).
- Multi-term observers evaluate the full query against the event source. Wildcard observers ("any event") "add significant overhead".
- flecs explicitly notes: *"a basic event queue is always going to outperform observers"* for simple cases.

**Implications for boyko**:

| Aspect | flecs observer | Boyko EventReader (pull) |
|--------|----------------|--------------------------|
| Dispatch cost | per-event dyn call | per-system one scan at run |
| Cache locality | poor (callback may touch arbitrary state) | good (linear scan of `reader_buf`) |
| Batching | none — fires immediately | natural — `Iterator::for_each` over a slice |
| Scheduler integration | requires runtime re-checks | fits the static access set cleanly |
| Order-independence | yes (`OnAdd` forwarding) | already handled by next-frame visibility |

Boyko's existing Phase 6 `EventDispatcher` is structurally identical to a "basic event queue" with a swap step — exactly the case flecs admits beats observers. There is no reason to add observers in Phase 12.

---

## §4 Event access tracking in conflict graph — boyko strategy

Phase 9 Round 3 C-NEW-1 explicitly kept events outside the conflict graph: the `Access` struct stays at 192 B / 3 cache lines without `event_read`/`event_write` bitmasks, on the grounds that EVT1 TLS per-lane single-writer discipline already guarantees correctness.

**What changes in Phase 12.** The Phase 9 plan §3232 already foreshadows: *"Phase 12 EventReader/EventWriter will revisit."* Two designs are available:

### 4.1 Option A — keep events out of the conflict graph (boyko status quo)

- `EventWriter<E>::send` uses `EventDispatcher::send_event` → TLS lane routing. Parallel writers of the same type are safe because they target distinct lanes.
- `EventReader<E>` reads `reader_buf` (immutable slice for the duration of a frame between two `update_events` calls). It is structurally `&[E]`.
- `update_events` runs on the dispatcher under `&mut EventDispatcher`. Parallel readers vs the swap are excluded because the scheduler tick runs `update_events` between frames (sync barrier).
- Result: `EventReader<E>` and `EventWriter<E>` can be issued by parallel systems for the same `E`. **This is strictly more permissive than Bevy.**
- Caveat: declares no access; intra-tick mutation of `reader_buf` is impossible by construction, but the architect must ensure `update_events` cannot be a system inside `Schedule::run` (per Phase 9 it is not — it is a frame boundary call).

### 4.2 Option B — Bevy parity (declare access in `FilteredAccessSet`)

- Add `event_read: BitSet256` and `event_write: BitSet256` to `Access` (16 + 16 = 32 B extra → `Access` grows to 224 B / 4 lines — breaks the 3-line invariant). Phase 9 §3232 cites this as the reason this was deferred.
- Alternative: pack event access into a separate `EventAccess` struct kept on `SystemMeta`, not on the per-conflict-edge `Access`. Conflict check during graph build runs an extra bitmask AND pass.
- Result: matches Bevy's serialization (one writer at a time per `E`), at the cost of `Access` layout change or an extra side-data structure.

Boyko has an unusual advantage over Bevy here: the per-lane writer architecture makes Option A correct, while Bevy's `Vec`-based `Events<E>` forces Option B. The architect should evaluate whether the extra parallelism in Option A is worth the deviation from Bevy's well-trodden contract.

---

## §5 Tick-aware reader cursor design

### 5.1 What Bevy does

Bevy's cursor is independent of `Tick` infrastructure. It uses a per-`Events<E>` monotonic `event_count: usize`. Tick interaction is indirect:

- `event_update_system` consults `Local<Tick>` `last_change_tick` to decide *whether* to run `update()` on a given frame (gating).
- Individual events do not carry ticks; readers know "what I have not yet read" by `last_event_count` comparison.

### 5.2 What boyko can do

Boyko's `update_events` is currently a flat-swap (next-frame visibility, no overlap). Two designs:

**Design 5.2.A — frame-only semantics (simplest)**. `EventReader<E>::read()` returns `&[E]` for the entire current frame's `reader_buf`. If a system runs N times in a frame (e.g. in a sub-schedule), it sees the same events each time. **No cursor needed**. The reader's State carries only the cached `EventId`. This deviates from Bevy and breaks the "do not re-read events" contract for systems that run more than once per frame.

**Design 5.2.B — Bevy-style cursor**. Add a per-dispatcher `frame_event_count: u64` per `EventId`, incremented on every `send`. State stores `last_event_count: u64`. `read()` returns events with index `> last_event_count` and updates the cursor. Requires per-`send` increment of the per-type counter — adds one `RelaxedFetchAdd` to the hot path of `send`. Matches Bevy exactly.

**Design 5.2.C — tick-keyed cursor**. Use Phase 10 `Tick`. `EventInstance` carries `Tick`. Reader compares `event.tick >= last_run`. Wastes 4-8 B per event and requires writes to read the system's `this_run` — but eliminates the per-type counter and aligns with the rest of Phase 10's change-detection vocabulary.

Bevy explicitly chose 5.2.B over a tick-based approach (`EventInstance` has no tick field). The cost of 5.2.C is small but the conceptual mismatch (events are not "components changed at tick X") is real.

---

## §6 Cached EventId in State

Phase 6 has `OnceLock<EventId>` minted by the `#[event]` macro, similar to the `OnceLock<ResourceId>` pattern in resources. The State pattern proven for `Res<R>`:

```rust
pub struct ResState<R: Resource> { pub(crate) id: ResourceId, ... }
```

`init_state` pays the `OnceLock::get_or_init` cost once; `get_param` does a direct slot indexing via the cached id.

For `EventReader<E>` / `EventWriter<E>`, the State would carry `EventId` (a `u64`). `get_param` then does:

```rust
let id = state.event_id;                        // u64, cached
let slot = unsafe { dispatcher.slot_unchecked(id) };
```

instead of `E::event_id()` (which is `OnceLock<EventId>::get_or_init` — Acquire-load on the fast path, ~1-3 ns; ~10x worse than direct field load on a cold cache line). Bevy achieves the same by storing `Res<Events<E>>` which internally uses the resource-id lookup with the same caching.

For boyko, an additional optimization is to cache the *slot pointer* (not just the id) in State, since `EventDispatcher` slot addresses are stable for the dispatcher's lifetime (Phase 6 SEND4 invariant). State would carry `*const EventTypeSlot` or `*const EventBuffer<E>` — eliminates the `slots[id]` bounds check and the `registered_mask.get(id)` check on every `get_param`. **Open question for architect**: does the dispatcher lifetime tie-in cleanly with `SystemParam::State: 'static`? Bevy avoids this by going through `Res<Events<E>>` (lifetime-managed by the resource system).

---

## §7 Public API surface examples

### 7.1 Writer

```rust
fn fire_explosion(mut events: EventWriter<Explosion>, query: Query<&Position, With<Bomb>>) {
    for pos in query.iter() {
        events.send(Explosion { pos: *pos, radius: 5.0 });
    }
}
```

`EventWriter::send` is the canonical name in boyko (Phase 6 uses `send`); Bevy renamed to `write` in 0.16 (deprecating `send`). Boyko has no migration debt — `send` can stay.

### 7.2 Reader

```rust
fn handle_explosions(mut events: EventReader<Explosion>, mut query: Query<&mut Health>) {
    for ev in events.read() {
        for mut hp in query.iter_mut() {
            hp.0 -= damage_falloff(ev.pos, ev.radius);
        }
    }
}
```

Bevy's `.read()` is `&mut self` (advances the cursor). For boyko Design 5.2.A this could be `&self` since there is no cursor, but a `&mut self` signature future-proofs against later cursor addition.

### 7.3 Batched writer

```rust
fn spawn_particles(mut events: EventWriter<Particle>) {
    events.send_many(particle_iter());
}
```

Phase 6 already has `EventDispatcher::send_many` with all-or-nothing semantics — directly exposable.

### 7.4 Multi-type combination

```rust
fn router(
    mut explosions: EventReader<Explosion>,
    mut sounds: EventWriter<SoundCue>,
    mut score: EventWriter<ScoreDelta>,
) { ... }
```

Each param gets its own State slot. With Option A from §4.1, all three can be in parallel with other systems reading/writing the same types.

---

## §8 Bench targets

User-supplied targets translated against measured Phase 6 baselines from the dispatcher tests:

| Operation | Target | Justification |
|---|---|---|
| `EventWriter::send` (cached state, 1 event) | ≤ 5 ns | Phase 6 baseline ~10 ns; subtract `OnceLock::get` (~2 ns) + `slots[id]` bounds check (~1 ns) + `registered_mask.get(id)` (~1 ns) when slot-ptr is cached |
| `EventReader::read` per-event iter | ≤ 2 ns | Slice iter + tick compare (if Design 5.2.B) — well within an L1d-hit add-and-compare |
| 10k events/frame send+read | ≤ 50 µs | 10 000 × (5 + 2) ns = 70 µs nominal; 50 µs requires `send_many` batching |
| `EventReader::read()` empty case | ≤ 3 ns | `reader_len.load(Acquire)` + `cursor == count` check |
| Init-state (per (system, event-type)) | ≤ 10 ns | `OnceLock::get_or_init` for `EventId` |

Bench fixtures needed: `event_reader_writer.rs` (cached state hot path), `event_reader_skip_old.rs` (cursor older than buffer, missed-update path).

---

## §9 Open architectural questions

1. **Cursor storage**: invent a `Local<T>` SystemParam now (Bevy's pattern, generally useful) versus inline `last_event_count` into `EventReaderState`. Phase 8a backlog mentions `Local<T>` is deferred.
2. **Per-event counter**: do we add `frame_event_count` to `EventDispatcher` (one `Relaxed` add per `send`) or accept frame-only semantics? Bevy adds the counter unconditionally.
3. **Slot-pointer caching in State**: store `*const EventTypeSlot` in State for zero-overhead `get_param`. Requires `SystemParam::State: 'static` reconciliation since the pointer is tied to the dispatcher's lifetime — but in practice the dispatcher is a singleton on `EcsMaster` that lives ≥ the system's lifetime.
4. **Access declaration scope**: Option A (events outside conflict graph) vs Option B (Bevy parity, breaks `Access` 192 B invariant or adds side-data). Option A leaves the door open to event-ordering races between sibling reader systems on the same frame, but boyko's `reader_buf` is read-only between swaps so this is observability-only, not soundness-affecting.
5. **EventReader vs ECS-level OnRemove / OnAdd**: Phase 12 specifies typed event reader on user events; structural events (component added/removed) are a separate Phase. Bevy unifies these via `Events<OnAdd>` etc.; flecs uses observers; boyko has not yet committed either way.
6. **Drain semantics**: Bevy's `EventReader` does not drain; `Events::update` clears. flecs observers drain implicitly. For boyko, `update_events` already drains the writer lanes. No new API needed.
7. **`EventMutator`** (Bevy 0.16 has `Mut<E>` access via `EventMutator`): out of scope for Phase 12 unless user requests.

---

## §10 References

[1] Bevy 0.16 `EventReader` source — `crates/bevy_ecs/src/event/reader.rs` — `Local<EventCursor<E>>` + `Res<Events<E>>` composition.
[2] Bevy 0.16 `EventCursor` source — `crates/bevy_ecs/src/event/event_cursor.rs` — `last_event_count: usize`, `len()` saturating math.
[3] Bevy 0.16 `EventWriter` source — `crates/bevy_ecs/src/event/writer.rs` — `ResMut<Events<E>>` wrapper.
[4] Bevy 0.16 `Events<E>` source — `crates/bevy_ecs/src/event/collections.rs` — double-buffer (`events_a` / `events_b`) + `event_count`.
[5] Bevy 0.16 `EventIteratorWithId` — `crates/bevy_ecs/src/event/iterators.rs` — chain construction and per-buffer index math.
[6] Bevy 0.16 `event_update_system` — `crates/bevy_ecs/src/event/update.rs` — state machine (`Always` / `Ready` / `Waiting`) with `Local<Tick>`.
[7] PR #1244 — *Make EventReader a SystemParam* — design rationale (boilerplate reduction, parallel-friendly abstraction over direct `Events` access).
[8] flecs Observers Manual — push vs pull, `"basic event queue outperforms observers"` quote, multi-event wildcard overhead.
[9] Boyko `event_dispatcher.rs` (current) — Phase 6 EventDispatcher with EVT1 TLS lane routing, 256 slots × 64 B aligned, `swap_and_flatten::<E>` monomorphized vtable.
[10] Boyko `PHASE-9-PARALLEL-SCHEDULER-PLAN.md` §3232 — explicit deferral of event access in conflict graph to Phase 12.
[11] Boyko `system_param.rs` — `unsafe trait SystemParam` with GAT `Item<'w, 's>`, `init_state`, `init_access`, `get_param`, `apply`.
[12] Boyko `res.rs` — cached-id state pattern (`ResState<R> { id: ResourceId, _marker }`) directly transferable to `EventReaderState<E>` / `EventWriterState<E>`.

### Relevant file paths in boyko (absolute)

- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\events\event_dispatcher.rs
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\events\event_buffer.rs
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\events\event.rs
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\system_param.rs
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\params\res.rs
- D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\system\system_meta.rs
- D:\claude\BoykoEngine\docs\PHASE-9-PARALLEL-SCHEDULER-PLAN.md

Sources:
- [bevy_ecs/src/event directory (v0.16.0)](https://github.com/bevyengine/bevy/tree/v0.16.0/crates/bevy_ecs/src/event)
- [Bevy PR #1244 — Make EventReader a SystemParam](https://github.com/bevyengine/bevy/pull/1244)
- [flecs Observers Manual](https://www.flecs.dev/flecs/md_docs_2ObserversManual.html)
- [Bevy EventReader (docs.rs 0.9.1)](https://docs.rs/bevy/0.9.1/bevy/ecs/event/struct.EventWriter.html)
- [Bevy Local<T> (docs.rs 0.16)](https://docs.rs/bevy_ecs/0.16.0/bevy_ecs/system/struct.Local.html)
- [Unofficial Bevy Cheat Book — Events](https://bevy-cheatbook.github.io/programming/events.html)
- [Bevy 0.16 → 0.17 Migration Guide (events → messages renaming)](https://bevy.org/learn/migration-guides/0-16-to-0-17/)
- [Sander Mertens — ECS FAQ repository](https://github.com/SanderMertens/ecs-faq)