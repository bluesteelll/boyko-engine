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

### Confirmed-SOUND (not gaps — verified during the build)
- `#[derive(Component)]` + `bytemuck::Pod` coexist: the Component derive is a pure
  marker (no injected fields/Drop/ticks), so a `#[repr(C)]` Pod component column is
  directly `cast_slice`-able — the demo's zero-copy headline holds.
- `EcsMaster::query::<&T, ()>().for_each_chunk(...)` yields one contiguous `&[T]`
  per archetype (the SoA→GPU upload path); plain `&T` does not trip the
  change-detection panic (G2).
- The `for_each_chunk` upload threads cleanly through `App::update`'s `&mut world`
  (H4 resolved): the `'static` egui callback only draws.
