# boyko_demo — Dogfooding Findings (public-API ergonomics)

Surfaced while building `boyko_demo` against the real `boyko_ecs` public API. These
are **ergonomics gaps, NOT bugs** — the engine works correctly; the demo crate works
around each one locally with no core edits. Captured here as candidate follow-up ECS
ergonomics work (a future "Phase E — public API ergonomics" pass). Ordered by impact.

## E1 — No `prelude` module (HIGH impact on first-use)
Every type needs a deep module path:
`boyko_ecs::ecs::core::component::component::Component`,
`...::ecs_master::ecs_master::EcsMaster`,
`...::schedule::schedule_builder::ScheduleBuilder`,
`...::system::params::query::Query`, etc. `boyko_ecs/src/lib.rs` re-exports only
`EcsError`/`EcsResult`. The demo added a local `boyko_prelude.rs` re-exporting ~15
paths to stay readable.
**Proposal:** a `boyko_ecs::prelude` re-exporting `EcsMaster`, `Entity`, `Component`,
`Bundle`, `Resource`, `ScheduleBuilder`, `Schedule`, `Query`, `Res`/`ResMut`,
`Commands`, `State`/`NextState`/`States`, the conditions, `ThreadPool`/`ThreadPoolBuilder`,
and the derives. Pure additive re-export; zero behavior change. Biggest single
ergonomics win.

## E2 — No immediate bulk/bundle spawn on `EcsMaster` (MEDIUM)
Setup-time spawning outside the deferred `Commands` path is `create_entity()` +
one-at-a-time `add_component(e, c)?`. For 100k×3 components that is 300k calls +
300k archetype-move checks. `#[derive(Bundle)]` exists but is only consumable via
`Commands` (deferred, needs a schedule apply), so it is unavailable for direct
setup spawning.
**Proposal:** an immediate `EcsMaster::spawn<B: Bundle>(b) -> Entity` (and/or
`spawn_batch`) that resolves the target archetype once and writes all columns —
mirrors `Commands::spawn` but synchronous, for world setup. Would also make the
`Bundle` derive useful outside systems.

## E3 — `add_component` error type needs a deep import to handle (LOW)
`add_component` returns `Result<_, <deep path>>`; `.unwrap()` is fine but matching/
mapping the error requires importing the deep error path. Minor; folds into E1.

## E4 — `Bundle` derive usable only via `Commands` (LOW–MEDIUM)
Consistency gap with E2: a derived `Bundle` cannot be handed to the direct
`EcsMaster` spawn path. Resolving E2 resolves this.

## E5 — `ThreadPool` construction is verbose (LOW)
`Arc::new(ThreadPoolBuilder::new().num_threads(n).build())` with
`available_parallelism` handling on the caller. A `ThreadPool::with_available_parallelism()
-> Arc<ThreadPool>` convenience would help (the scheduler already wants an `Arc`).

## E6 — State conditions are the ONLY downstream route to transition info (LOW–MEDIUM)
Surfaced wiring the Wave-5 mode switch (Phase-17 dogfood). `StateTransitionRecord<S>`
and its `current()` accessor are `pub(crate)`; `Transition<S>`'s fields are
`pub(crate)`. So a downstream crate (the demo) CANNOT read "what transition fired
this frame" directly — the only public surface is the `on_enter`/`on_exit`/
`on_transition`/`in_state` condition functions, consumed via `.run_if(...)`.
This is fine in practice (the conditions cover every case the demo needs), and it
keeps the record opaque — but it does mean the critic-suggested fallback for C2
("collect ids in a `&self`/`Commands` system that self-gates on the transition
record") is **not available to an out-of-crate caller**: there is no public way to
ask the record whether `S` exited `X` this frame outside a `.run_if`. Luckily the
primary path works (see the Confirmed-SOUND note on exclusive `.run_if`), so the
fallback was never needed.
**Proposal (optional):** a public read-only `EcsMaster::state_transition::<S>() ->
Option<(Option<S>, S)>` (exited, entered) for code that wants to branch on the
transition outside a condition. Pure additive; no behavior change.

## E7 — `query_entities` allocates a fresh `Vec<Entity>` per call (LOW)
Despawn-on-exit (plan D16) is `let ids = world.query_entities(&[Tag::id()]); for e
in ids { world.delete_entity(e); }`. `query_entities` returns an owned
`Vec<Entity>` (it must — `delete_entity` needs `&mut self`, so the ids can't be
borrowed across the delete loop). This allocates once per mode switch, which is
fine (transition frames are rare), but a hot despawn-by-tag would want a
`query_entities_into(&mut Vec<Entity>)` reuse variant or a borrowing-then-draining
API. Not a Wave-5 problem (switches are infrequent); noted for completeness.

---

### Wave-4 stack finding (egui/eframe, NOT boyko_ecs)

Surfaced wiring the egui control panel (Wave 4). It is about the **rendering
stack**, not the ECS, and is recorded only so the next person on the demo does not
re-hit it. No core or ECS change implied.

#### W4-1 — No `egui_plot` published for egui 0.34 → FPS plot hand-rolled
`eframe 0.34.3` pins `egui 0.34.3`, but crates.io has no `egui_plot 0.34` (the
nearest published versions are `0.35.0` and `0.33.x`). Adding `egui_plot 0.35`
against `egui 0.34` pulls a **second** `egui` into the graph → the "two
`egui::Ui` types" mismatch the demo plan's H2 gate warns about. Per the plan's
documented fallback, the rolling frame-time plot is hand-rolled with
`egui::Painter` line segments instead (fed from the same fixed-size `FrameStats`
ring; no extra dependency, one egui version). Revisit `egui_plot` when a release
tracks the eframe-pinned egui minor; until then the hand-rolled sparkline is the
correct choice. (Aside for future waves: in egui 0.34 the pointer-capture query
is `Context::egui_wants_pointer_input`; the bare `wants_pointer_input` is
deprecated. And `Painter::rect_filled` takes a `CornerRadius` — `u8`-based — so a
bare float literal does not infer; use `CornerRadius::ZERO`/`::same(n)`.)

### Wave-5 finding (Phase-17 states dogfood)

The Wave-5 binding gate (plan §3 C2): does an **exclusive** system
(`fn(&mut EcsMaster)`) gated `.run_if(on_exit(Mode::X))` compile and fire exactly
on the exit frame? Phase-17 only ever gated *function* systems with `.run_if`, so
this combo had zero in-crate precedent.

#### W5-1 — exclusive system + `.run_if(state condition)` WORKS (gate passed)
`SystemConfig::run_if<C, M>` infers the condition's marker `M` independently of the
system's own `IntoSystem` marker. The exclusive blanket uses
`(ExclusiveSystemMarker, fn(&mut EcsMaster))`, disjoint from the condition's
function-system marker, so attaching `.run_if(on_exit(..))` / `.run_if(on_enter(..))`
to an exclusive `fn(&mut EcsMaster)` compiles and fires exactly on the transition
frame (`tests/state_exclusive_smoke.rs`, two passing cases + an `in_state`
function-system companion). Wave 5 therefore uses the critic's RECOMMENDED default
(exclusive despawn/spawn gated by state conditions), NOT the `Commands::despawn`
fallback. `.before`/`.after`/`.key()` also compose with exclusive systems, so the
intra-frame order despawn-old `.before` spawn-new `.before` sync (plan H3) is pinned
directly (`tests/mode_switch.rs`). This is the headline Phase-17 dogfood result:
the states feature drives a live, switchable two-mode sandbox end-to-end through
the public API with no core changes.

### Confirmed-SOUND (not gaps — verified during the build)
- `#[derive(Component)]` + `bytemuck::Pod` coexist: the Component derive is a pure
  marker (no injected fields/Drop/ticks), so a `#[repr(C)]` Pod component column is
  directly `cast_slice`-able — the demo's zero-copy headline holds.
- `EcsMaster::query::<&T, ()>().for_each_chunk(...)` yields one contiguous `&[T]`
  per archetype (the SoA→GPU upload path); plain `&T` does not trip the
  change-detection panic (G2).
- Boids `par_iter_mut` + reading a `Res` is SOUND (plan §6.5 / D12 worry): the
  `boid_forces` pass reads `Res<BoidSnapshot>` + `Res<SpatialGrid>` + `Res<BoidParams>`
  (shared, broadcast to every worker) while each boid writes only its own
  `&mut Velocity` row — no aliasing, no borrow issue. Snapshotting the pre-tick
  state (D12) is what makes it sound: workers never read a `Velocity`/`Position`
  row a sibling is writing. Identical shape to the already-shipped
  `sync_gpu_instance` par pass. No core change needed.
- The `for_each_chunk` upload threads cleanly through `App::update`'s `&mut world`
  (H4 resolved): the `'static` egui callback only draws.
