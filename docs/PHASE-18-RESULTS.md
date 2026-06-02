# Phase 18 — Results: Plugin system (`App` builder facade + `Plugin` + `prelude`)

Branch `ecs`. Additive builder facade over the shipped `EcsMaster` + `ScheduleBuilder` +
`Schedule` + `ThreadPool`. **No ECS core changes** (the roadmap promise held: only 7 lines touched
in existing files — 2 re-export lines in `lib.rs`, 1 `pub mod app;` in `core/mod.rs`, 4 in
`Cargo.toml` for the bench entry).

## Status: COMPLETE — `App::new().add_plugins(..).run()` ships

Full pipeline: research → architect → architecture-critic (APPROVED WITH CHANGES) → developer →
code-review (APPROVED) → tester (all green). 14/14 Phase-18 tests + proptest pass; full suite
**829 pass**; 0%-gate bench holds; zero `unsafe`.

## What shipped

### `Plugin` trait (`core/app/plugin.rs`)
```rust
pub trait Plugin: 'static {
    fn build(&self, app: &mut App);
    fn name(&self) -> &'static str { core::any::type_name::<Self>() }
}
```
- ONE required method. `'static` only — **NO `Send + Sync`** (justified divergence from Bevy):
  boyko has no async/sub-app/`finish`/`cleanup` lifecycle, so plugins are **consumed at `build`**
  (called immediately in `add_plugin`, dropped on the dispatcher thread, never retained). Dropping
  the bound is strictly more permissive (a plugin may capture `!Send` setup data) at zero cost.

### `App` facade (`core/app/app.rs`)
- Owns `world: EcsMaster` + `pool: Arc<ThreadPool>` + a staged `Option<ScheduleBuilder>` →
  `Option<Schedule>` + a one-shot `startup` list + a `Vec<TypeId>` plugin-dedup set + a `finished`
  flag. `App` is `!Send + !Sync` (from `EcsMaster`) — correct, single-threaded-owned.
- **Constructors (E5 fold-in):** `new()` (pool defaults to `available_parallelism`),
  `with_threads(n)` (clamped `[1,64]`), `with_pool(Arc<ThreadPool>)`, `Default`. The App owns the
  pool internally — removes the demo's manual `Arc`/`ThreadPoolBuilder` dance.
- **Config:** `add_plugin`/`add_plugins`, `insert_resource`, `init_state`/`insert_state`,
  `add_systems` (unordered convenience), `add_systems_cfg(|b: &mut ScheduleBuilder| …)` (the
  PRIMARY ordered path — full Phase-15/16/17 chaining verbatim), `add_startup_system` (one-shot,
  runs once before the loop via `EcsMaster::run_system`), `world`/`world_mut`/`pool`. Each
  builder-touching method carries a `debug_assert!(self.builder.is_some())` config-phase guard.
- **Lifecycle:** `finish()` resolves the borrow wrinkle — `self.builder.take()` (releases the
  field) then `builder.build(&mut self.world)` (disjoint field borrow) then assign `schedule` then
  drain `startup`. Idempotent; auto-called on first `update`/`run`/`run_n`.
- **Runner:** `update()` (one frame, single cold auto-finish branch), `run_n(frames)`, `run()`
  (loops until a system sets `ResMut<AppExit>(true)`).

### `add_plugins` variadic (`core/app/plugins.rs`)
Sealed `Plugins<Marker>` trait: a `PluginMarker` leaf impl (single plugin) + `1..=12` tuple impls
(structural clone of the shipped `system/params/tuple_impl.rs` macro, but a **safe** `impl`). Marker
disambiguation (same `IntoSystem<_,_,M>` technique) makes single-vs-tuple non-overlapping; nesting
works (`add_plugins((A, (B, C)))`); left-to-right build order.

### Duplicate detection
`add_plugin` panics `boyko-B1801: plugin '<name>' added more than once` on a repeated `TypeId`
(cold `Vec<TypeId>` scan; panic extracted to a `#[cold] #[inline(never)]` helper). Panic, not
silent-skip — no compile-but-lie footgun.

### `boyko_ecs::prelude` (E1 — partial)
`core` prelude re-exporting the public **types** (App/Plugin/Plugins/AppExit, EcsMaster, Entity,
Component/Bundle/Resource traits, Schedule/ScheduleBuilder + conditions, Query, Res/ResMut/Local/
Commands/EventReader/EventWriter, State/NextState/States, ThreadPool/ThreadPoolBuilder, EcsError/
EcsResult). Collapses the demo's ~15 deep-path imports to one `use`.

## The `boyko-macros` cycle constraint (the one accepted limitation)

`boyko-macros` **depends on `boyko-ecs`** (it is documented in `boyko_macros/src/lib.rs:31` and its
derives emit downstream-only `::boyko_ecs::…` paths), so `boyko-ecs` keeps it as a **dev-dependency
only** — promoting it to a normal dependency would create a `boyko-ecs → boyko-macros → boyko-ecs`
cycle. Two consequences, both correctly handled:
1. **`AppExit` hand-impls `Resource`** (not `#[derive(Resource)]`) — the derive is unusable inside
   boyko-ecs lib code. The hand-impl mirrors the derive output byte-for-byte (`OnceLock<ResourceId>`
   + `ResourceId(register_new::<Self>())`); code-review confirmed semantic identity.
2. **The prelude omits the derive re-exports** — users `use boyko_macros::{Component, …}` directly
   (as `boyko_demo` already does). The "single-dep prelude including derives" (E1's full vision) is
   **DEFERRED**: it requires re-architecting the proc-macro path resolution (`extern crate self as
   boyko_ecs` + breaking/handling the cycle) — too risky to bundle into Phase 18, and the existing
   setup is a deliberate, documented design choice. Filed as a follow-up.

## Verification gate (all orchestrator/tester-run)

| Oracle | Result |
|--------|--------|
| **`tests/app_plugin.rs`** | **14 passed; 0 failed** (debug AND release) — parity-vs-manual, plugin build-order, dup-panic `boyko-B1801`, flat+nested tuple, startup-once, auto-finish, idempotent finish, pool default + `with_threads` clamp, `run` exits on `AppExit`, prelude compile-smoke, proptest (16 cases) |
| **`cargo test -p boyko-ecs`** | **829 passed** (lib 495 — identical to pre-Phase-18 baseline — + app_plugin 14 + 320 other integration). No regression. |
| **0%-gate bench** `phase18_app` | App `run_n` vs raw `Schedule::run` @ 50 systems: **+1.17% / −0.79%** across two back-to-back runs (sign-flipping ⇒ noise, well inside ±3%). App per-frame loop byte-equivalent to raw `schedule.run` (the W1 bind-locals-once design). Group-B 4.28 µs reproduces the Phase-9 4.27 µs figure. |
| `cargo clippy --all-targets -- -D warnings` | clean (code-reviewer confirmed **workspace-wide**) |
| Miri | N/A — zero `unsafe`, no new cross-thread state; `App` drives the already-Phase-9.1-Miri-proven executor |

**Pre-existing (NOT Phase 18):** `bundle_compile_fail` + `compile_fail_chunk` trybuild targets fail
on stale `.stderr` snapshots (rustc diagnostic drift). The tester proved pre-existing status by
stashing the Phase-18 changes and re-running (identical failures). Out of scope; needs a
`TRYBUILD=overwrite` re-bless (toolchain-sensitive) as a separate test-hygiene task.

## Pipeline + decisions

- **Critic's load-bearing catch (C1):** the architect's first cut had `AppExit` use `#[derive]`;
  the critic flagged `resource()` PANICS on a missing resource → `run()` would panic frame 1. Fixed
  by `run()` inserting `AppExit(false)` before the loop. (The dev then found the deeper cycle reason
  the derive can't be used at all → hand-impl.)
- **W1 (0%-gate hardening):** `run_n`/`run` bind `schedule` + `world` as locals ONCE (disjoint field
  borrows) so the loop body is a direct branch-free `schedule.run(world)`. Bench-confirmed flat.
- **Q7 (the schedule-count crux) = (a) single schedule** + one-shot startup list (not a second
  schedule, not a label map) — defended on the demo's actual single-schedule structure, the
  0%-gate, and "no core changes." Startup runs single-threaded before the loop.
- **Dev "stop-and-report" discipline:** the dev did NOT promote `boyko-macros` unannounced when it
  hit the cycle — it flagged the decision. Orchestrator ratified deferral.

## DEFERRED (boundaries)
SubApps/render-world; Plugin `finish`/`cleanup`/`ready`; `PluginGroup`/`DefaultPlugins`;
multi-schedule label map; `init_resource`/`FromWorld`; `set_runner`; the **single-dep prelude with
derives** (needs the macro-cycle refactor); the **`boyko_demo` port to `App`** (a restructure, and
the demo's wasm/no-pool path can't use `App` — `App` is native-multithreaded only).

## Files
- NEW: `crates/boyko_ecs/src/ecs/core/app/{app,plugin,plugins,app_exit,mod}.rs` (~605 lines),
  `crates/boyko_ecs/src/prelude.rs`, `crates/boyko_ecs/tests/app_plugin.rs`,
  `crates/boyko_ecs/benches/phase18_app.rs`.
- TOUCHED (additive, 7 lines): `crates/boyko_ecs/src/lib.rs` (+2 re-export),
  `crates/boyko_ecs/src/ecs/core/mod.rs` (+1 `pub mod app;`), `crates/boyko_ecs/Cargo.toml` (+4 bench entry).
