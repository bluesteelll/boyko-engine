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

### Wave-6 finding (physics + `Changed<T>` dogfood)

Wave 6 added the Physics mode (bouncing balls + inter-ball collisions) and the
`Changed<Velocity>` "flash recently-collided balls" showcase (plan D13). Two
findings, both ergonomics — no bug, no core change.

#### W6-1 — precise `Changed<T>` requires writing through `Mut<T>` per row, not `&mut T` and not `for_each_chunk`
The plan asks for a `tint_collided` system using `Query<&mut GpuInstance,
Changed<Velocity>>` that flashes only the balls whose velocity changed this frame
(collision / wall bounce). The non-obvious part is **what makes a row count as
`Changed<Velocity>`**: only `Mut<T>::deref_mut` bumps the per-row `changed` tick.
A plain `&mut T` query item (the `QueryData` for `&mut Velocity`) has
`NEEDS_CHANGE_DETECTION = false` and writes the value WITHOUT touching the tick;
a `for_each_chunk` `&mut [Velocity]` does not touch ticks at all. So to make the
showcase precise, the velocity write-back (`apply_ball_motion`) must:
* take `Query<(&mut Position, Mut<Velocity>), With<BallTag>>` (note `Mut<Velocity>`,
  imported from `...::iters::query::data::Mut`), and
* deref-write velocity (`vel.x = ...`, which routes through `Mut::deref_mut`)
  **only for the rows the solver flagged touched**, leaving untouched rows'
  guards un-deref'd so their tick stays put.
This is correct and works (the `Mut` guard's first `deref_mut` writes `this_run`
to the row's `changed` tick), but it has a consequence the SoA story should note:
the velocity write-back is a **per-row `iter_mut` walk, not a `par_iter_mut` or a
chunked SoA write**, precisely because the per-row tick bump is a per-row decision.
A blanket `par_iter_mut` over `Mut<Velocity>` would bump every row's tick
(flashing everything); a `for_each_chunk` cannot bump ticks at all. The collision
solve is already sequential (G12), so the sequential write-back is free here, but
the general lesson is: **selective change-marking is inherently per-row through the
`Mut` guard** — there is no "mark these N rows changed in bulk" API. A future
`Mut<T>::set_changed()` / a batch tick-marking helper would let a chunked writer
opt specific rows into change detection. Not needed for the demo (sequential
anyway); noted as a candidate ergonomics follow-up.

#### W6-2 — same-frame `Changed<T>` observation across ordered systems works (no extra frame of latency)
Confirmed (mirrors the core's `changed_filter_after_mutation` test): within ONE
`Schedule::run`, a downstream `Changed<Velocity>` reader (`tint_collided`) DOES
observe the tick bumped by an upstream writer (`apply_ball_motion`) in the same
run, **provided the two are ordered** (here via `.after(apply_ball_motion)` plus
the natural write-Velocity / read-Velocity conflict the scheduler already orders).
No extra frame of latency, no double-buffering needed. The flash appears on the
same frame the collision resolves. Good Phase-10 result for the demo: change
detection composes cleanly with explicit `.before`/`.after` ordering.

### Wave-7 finding (web / wasm32 build) — ✅ RESOLVED (was a core blocker)

> **RESOLUTION (verified):** the wasm core port landed (commits `670b49f` core,
> `c949c8a` + `9c27a71` demo). `cargo build -p boyko_demo --target
> wasm32-unknown-unknown` now Finishes **warning-free**; native build + clippy
> `--all-targets` + all 15 demo tests + the 494 `boyko_ecs` lib tests stay green.
> Fix = three parts: (1) gate the 28 pointer-width `const _` layout asserts behind
> `#[cfg(target_pointer_width = "64")]` in `boyko_ecs` (they still fire verbatim on
> x86_64); (2) move `rand` → `boyko_ecs` `[dev-dependencies]` (its only `src/` use
> is a `#[cfg(test)]` AVX2 test) so `getrandom` leaves the normal wasm build;
> (3) enable the `eframe` `webgl` backend feature on the wasm target + add a
> `#[cfg(wasm32)] fn main() {}` stub (real entry is `wasm_start`). No SIMD wall,
> no threadpool wall, no `Instant`-in-core wall — the core was wasm-portable bar
> the const-asserts. **Web deploy is ready pending a local `trunk build --release`
> + a browser check (neither runnable in the agent environment).** The historical
> blocker analysis below is retained for the record.

Wave 7 added the wasm (web) build: the `#[cfg(target_arch = "wasm32")]` sequential
runner (`sim/runner.rs`), the `wasm_bindgen(start)` → `WebRunner` entry
(`main.rs`), `index.html` + `Trunk.toml`, and the additive Pages-CI demo job. The
demo-side cfg-split is complete and **native stays clean**. But the binding gate —
`cargo build -p boyko_demo --target wasm32-unknown-unknown` — **FAILS**, and the
failure is **entirely upstream in `boyko_ecs`**, not in the demo.

#### W7-1 — `boyko_ecs` has ~19 hard-coded **64-bit** `const _` layout asserts that fail const-eval on the 32-bit wasm32 ABI

`wasm32-unknown-unknown` is a 32-bit target (`size_of::<usize>() == 4`,
`size_of::<*mut T>() == 4`, pointer `align == 4`). `boyko_ecs` pins many internal
struct layouts with **unconditional** compile-time asserts hard-coded to 64-bit
sizes/offsets/alignments, e.g. (`crates/boyko_ecs/src/ecs/core/entity/entity_inland.rs:30`):

```rust
#[repr(C)]
struct EntityInland { archetype_ptr: *mut Archetype, unit_index: u32, generation: u32 }
const _: () = assert!(std::mem::size_of::<EntityInland>() == 16);          // wasm32: 12 → FAILS
const _: () = assert!(std::mem::align_of::<EntityInland>() == 8);          // wasm32: 4  → FAILS
const _: () = assert!(std::mem::offset_of!(EntityInland, unit_index) == 8); // wasm32: 4 → FAILS
const _: () = assert!(std::mem::offset_of!(EntityInland, generation) == 12);// wasm32: 8 → FAILS
```

`rustc` evaluates these `const _: () = assert!(...)` items even for a library that
is only *built* (not run), so the whole crate fails to compile on wasm32. The full
set (19 × `error[E0080]: evaluation panicked`), all in `boyko_ecs` core:

| Type (`size`/`align`/`offset` asserts) | Why it is 64-bit-only |
|---|---|
| `EntityInland` (size 16, align 8, off 8/12) | `*mut Archetype` is 8 B on 64-bit, 4 B on wasm32 |
| `Column` (size 16, align 8, off `stride` 8 / `_reserved` 12) | embeds a pointer |
| `Archetype` (size 8480) | shrinks once its pointers are 4 B |
| `ComponentLayout` (size 56) | embeds `usize`/pointer fields |
| `BundleColumnRecord` (size 32) | embeds `usize`/pointer fields |
| `RemoveOutcome` (size 16) | `Option<EntityId>` niche layout |
| `Commands<'static>` (size 16) | two references → 8 B on wasm32 |
| `EntityCommands<'static,'static>` (size 24) | references → 12 B on wasm32 |
| `EntityCounter<'static>` (size 8, align 8) | one reference → 4 B on wasm32 |
| `EventReader<'s,E>` (size 8) | cached `NonNull` → 4 B on wasm32 |
| `EventWriter<'s,E>` (size 8) | cached `NonNull` → 4 B on wasm32 |

**This is out of scope for the demo** (only `crates/boyko_demo/**` + the Pages CI
may change here) and CANNOT be worked around demo-side: the failing asserts are in
the `boyko_ecs` dependency the demo links. The demo code itself is wasm-clean —
the compile reaches all the way through the demo's own deps (`getrandom` wasm
backend, `wasm-bindgen-futures`, the sequential runner type-checks) and dies in
the upstream crate.

**Proposed core fix (additive, no behavior change):** make each layout assert
pointer-width-aware, gating the 64-bit numbers behind
`#[cfg(target_pointer_width = "64")]` and either dropping the assert or asserting
the matching 32-bit number under `#[cfg(target_pointer_width = "32")]`. The
structs are already `#[repr(C)]`, so their wasm32 layout is well-defined; only the
*assertions* assume 64-bit. A targeted sweep of the ~11 types above unblocks the
wasm build with zero runtime change on native. (Bevy does this implicitly by not
hard-asserting absolute sizes; the boyko asserts are a deliberate
layout-regression guard that simply was never made portable.)

Until that lands, the web build cannot compile. The demo's wasm scaffolding
(sequential runner, entry, `index.html`/`Trunk.toml`, CI) is in place and
correct, so the web build comes online the moment the core asserts are gated.
The Pages CI demo step is therefore **non-fatal** (`continue-on-error`) so it
cannot break the existing mdBook/rustdoc deploy while the blocker stands.

#### W7-2 — `Arena` `!Send`/`!Sync` is NOT a wasm blocker (confirmed, not a gap)

The plan worried (D10) that the `!Send`/`!Sync` arena (TLS discipline) might break
even single-threaded wasm. It does not: the wasm runner constructs **no
`ThreadPool`** and calls **no `Schedule::run`** — the world, arena, and every
system run on the one main thread, so nothing is ever sent across a thread
boundary. `!Send`/`!Sync` only constrains cross-thread moves, which the sequential
path never performs. The single blocker is W7-1 (const-asserts), not threading.

#### W7-3 — `Schedule::run` hard-requires a pool ⇒ the wasm runner is hand-rolled (option (b))

Confirmed against the source: `ScheduleBuilder::new` takes an `Arc<ThreadPool>`
and `Schedule::run` enters `pool.install(...)` and dispatches every system body
through `Scope::spawn` (`schedule.rs`). There is **no** sequential / no-pool
execution path in the schedule, so the wasm runner cannot reuse `Schedule` (plan
D10 option (a) is unavailable). Instead it hand-rolls the dependency-ordered
sequential dispatch (option (b)): it drives the SAME per-mode system functions via
`EcsMaster::run_system` (sequential init + run + apply) in the exact `.after(...)`
order the native builder pins, and replicates the Phase-17 transition pass +
`on_enter`/`on_exit`/`in_state` gating inline (reading `NextState<Mode>` /
`State<Mode>`, doing the despawn-old → spawn-new → state-swap). The system bodies
and components are 100% shared across targets — only `runner.rs`'s dispatch is
cfg-split. `par_iter_mut` inside a body falls back to a sequential walk with no
pool attached (PAR7), so the bodies need no `#[cfg]`. A future
`EcsMaster::spawn_state::<S>` or a no-pool `Schedule::run_sequential` would let the
wasm path share the native transition logic instead of re-deriving it.

#### W7-4 — wasm `Changed<T>` is an always-true footgun under `run_system` (cosmetic divergence)

The native physics mode's `tint_collided` (`Changed<Velocity>`) is intentionally
NOT run on wasm. `EcsMaster::run_system` re-`initialize`s its system each call,
resetting the system's tick window to the `MAX_CHANGE_AGE` sentinel, so a
`Changed<T>` filter reads as ALWAYS-TRUE (flash every ball) — the documented
unguarded-tick footgun (W6-1). Dropping the flash on wasm makes the physics view
render every ball at its `sync_ball_gpu` base color; the collision *response* is
identical (it is tick-independent). This is the single behavioral difference from
native, and it is purely cosmetic. (To get precise `Changed<T>` on the wasm
sequential path one would need a persistent per-system tick across `run_system`
calls — i.e. a cached `FunctionSystem` whose `set_change_ticks` is advanced
per-frame, which the schedule does for native but the bare `run_system` does not.)

#### W7-5 — getrandom 0.3 on wasm needs BOTH a feature AND a rustflag

`rand` 0.9 pulls `getrandom` 0.3, which on `wasm32-unknown-unknown` requires the
`wasm_js` *Cargo feature* (declared in `Cargo.toml`) **and** the
`--cfg getrandom_backend="wasm_js"` *rustflag* (set in the demo's
`.cargo/config.toml`) — the feature alone emits a `compile_error!`
(getrandom 0.3.4 `src/backends.rs`). The config is scoped to the crate dir, so the
CI `cd`s into `crates/boyko_demo` before `trunk build` and the documented local
build runs from there; a workspace-root `--target wasm32` would not pick it up.
Not a boyko gap — an ecosystem build-config detail recorded for the next person.

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
