# Phase 17 — States / State Transitions — Results

Branch `ecs`. Bevy-style application/game states layered on the existing single
`Schedule` via the boyko-native **shape (b)**: enter/exit logic is ordinary
condition-gated systems in the one schedule, fed by a built-in per-frame
transition pass. Composes with Phase 15 (sets/ordering) and Phase 16 (run
conditions). Plan: [`docs/PHASE-17-PLAN.md`](PHASE-17-PLAN.md).

## Status: COMPLETE — full gate green

- **814 tests pass / 0 fail / 2 ignored** (workspace, default features), incl.
  23 Phase-17 integration + 15 state-module unit + 6 Miri integration.
- `cargo clippy --all-targets -- -D warnings` clean; `cargo build --release` clean.
- **Miri (Tree Borrows) clean**: 15 lib (`ecs::core::state::*`, incl. the
  `apply_state_transition` algorithm direct-drive) + 6 integration
  (`miri_phase17`, incl. 2 new condition-`run_unsafe` read-path tests).
- **0%-regression confirmed**: 50-systems executor bench 4.21 µs,
  criterion "No change in performance detected" (p = 0.19) vs the prior
  `p17` baseline.
- **Zero new `unsafe`** across the entire phase (plan §8 target met).

## What shipped

- **`States` trait** (`state/states.rs`) — marker, hand-impl (no derive):
  `Send + Sync + Sized + Clone + PartialEq + Eq + Hash + 'static`.
- **`State<S>`** (`#[repr(transparent)]`, current value) and **`NextState<S>`**
  (`enum { Unchanged, Pending(S) }`, queued request) resources.
- **A `TypeId`-keyed resource-id registry** (`state/state_resource_registry.rs`)
  — the load-bearing D3 decision: avoids the rust#22991 static-collapse trap
  that `#[derive(Resource)]` on a generic `State<S>` would hit (every `S` would
  alias one resource slot). Reuses the proven `query_type_registry` pattern.
- **`StateTransitionRecord<S>` + `Transition<S>`** (`state/transition_record.rs`)
  — a per-`S` resource recording "exited/entered this frame", written only by
  the pass, read by the conditions; plus `apply_state_transition::<S>` (the §5.1
  algorithm) and the type-erased `StateEntry`.
- **Run conditions** `in_state` / `on_enter` / `on_exit` / `on_transition`
  (`schedule/common_conditions.rs`), composing with Phase 16 `.run_if`.
- **Built-in transition pass** (`schedule/schedule.rs::run_state_transitions`,
  `#[cold]`) — runs once per `Schedule::run`, before condition eval, gated by
  `state_entries.is_empty()` (the 0%-gate). Holds `&mut world` directly (before
  `pool.install`) → no cell, no `unsafe`.
- **Builder + world entry points** — `ScheduleBuilder::{init_state, insert_state}`
  (record-then-realise, idempotent per `S` by `TypeId`) and
  `EcsMaster::{insert_state, init_state, state, set_next_state}`.
- **Initial `OnEnter(initial)`** synthesized once on the first `Schedule::run`
  (`none → initial`, `pending_initial` per `StateEntry`).
- **Multiple orthogonal `State<S>` types**, fully generic, per-type record.
- **`StateTransitionSet`** unit `SystemSet` — opt-in ordering hook (not auto-wired).
- **Identity `IntoSystem` blanket** (`into_system.rs`, IS2) — the long-anticipated
  `impl<S: System> IntoSystem<(), <S as System>::Out, S> for S`, landed to let a
  pre-built `System` (the conditions) be passed to `.run_if`.

## The F1 bug (caught by the tester; recurring lesson)

The headline conditions originally returned `impl FnMut(Res<…>) -> bool` and **did
not compile through `.run_if(..)`**. Root cause: boyko's `SystemParamFunction`
blanket requires the double-`FnMut` HRTB bound (`FnMut(P) -> Out` **and**
`for<'w,'s> FnMut(<P as SystemParam>::Item<'w,'s>) -> Out`); an opaque `impl FnMut`
return type carries only the first — the HRTB-projected one is lost across the
`impl Trait` boundary, so `IntoSystem<(), bool, M>` is never satisfied. Inline
closures work (rustc sees the concrete type); the opaque return does not.

**This passed architect + critic + dev + code-review** because nothing in the crate
ever compiled `.run_if(in_state(X))` end-to-end — zero in-crate usages, exactly the
"primitive verified, headline API never exercised" class of Phase 12.5 NCD6
("wired but never called") and Phase 14 ("Miri found what review missed").

**Fix** (plan D5 F1-amendment): (1) conditions return `impl System<Out = bool>` —
the closure is concretized inside the body (both HRTB bounds resolve there) and
wrapped via `IntoSystem::into_system(..)`, exposed as `impl System` (a plain trait
that survives opacity); (2) added the IS2 identity blanket so `impl System`
re-bridges to `IntoSystem` for `.run_if`. Zero new `unsafe`, zero runtime cost
(identity `into_system` + still boxed into `BoolSystem`), coherent (marker `S`
disjoint from the function/exclusive 2-tuple markers; verified all 10 `impl System`
are named nominal types so no closure is ever a `System`), no inference regression
(closures still route through the function/exclusive blankets — `cargo check
--all-targets` confirms every existing `.run_if`/`.add_system` call site).

## Decisions (see plan §2 for full justifications)

- **D1** `States` hand-impl, no derive (a derive adds only a bound check).
- **D2** Two-resource split (current vs request) — avoids false conflicts in the
  Phase-9 graph; `in_state` reads only `State<S>` (shared).
- **D3** `TypeId`-keyed registry for the generic `Resource` ids (rust#22991 trap).
- **D4** Per-`S` `StateTransitionRecord` resource (`Clone`, not `Copy` — `States`
  bounds `Clone`); 0-cost when no state registered.
- **D5** Conditions ride Phase 16 (+ F1 amendment: `impl System` + IS2 blanket).
- **D6** Identity transition = no-op (re-entry deferred).
- **D7** Initial `OnEnter` synthesized on frame 1; pre-frame-1 `Pending`
  suppresses it (documented on the API surface — M4).
- **D8** `in_state` require-exists (panic if state unregistered; no `Option<Res>`).
- **D9** Multiple orthogonal states, generic, per-type.
- **D10** Enter/exit systems user-ordered (`StateTransitionSet` opt-in, not auto).
- **D11** `on_transition` included (one extra cold compare on the same record).

## Verification detail

- **Unit** (15, in `state/` modules): `state_resource_ids_distinct_per_type`
  (the rust#22991 regression guard), 5 `apply_state_transition` direct-drive
  algorithm tests, `next_state` semantics, layout sanity.
- **Integration** (23, `tests/phase17_states.rs`): all 13 plan §9 condition tests
  (initial-fires-once, enter/exit-on-right-frame, identity-no-op, last-write-wins,
  fires-exactly-once, `in_state`-gating, `on_transition`-exact-pair,
  orthogonal-independent, no-states-zero-overhead, Phase-15-ordering interaction,
  Phase-16-condition interaction, `set_next_state` direct API, `init_state`-twice
  idempotent) + the `StateTransitionSet` ordering test + 10 PART-1 state-machine
  tests (no conditions).
- **Miri**: 15 lib + 6 integration clean. The schedule-driving tests stay
  `#[cfg(not(miri))]` (Phase-9 `Scope::spawn` deferral, unchanged); the tester
  added 2 isolated condition-`run_unsafe` read-path Miri tests (driving a
  constructed `in_state`/`on_enter` via `run_system_once`, no pool) that close the
  previously-unexercised TB gap on the condition body's resource reborrow.
- **0%-regression**: the only hot-path addition is one `if
  !self.state_entries.is_empty()` in the once-per-frame `Schedule::run` preamble;
  `state_entries` is the **last** struct field (M3, no hot field shifted); the
  executor loop body is byte-identical. Confirmed 4.21 µs, "no change detected".
- **Microbench** (`benches/phase17_states.rs`, informational): transition pass
  0/1/4 states ≈ 1.24/1.41/2.50 µs; `in_state`-gated 16 systems ≈ 3.66 µs vs
  ungated 3.55 µs (~7 ns/system condition-eval overhead).

## Deviations from the plan (all benign, verified)

- **F1 fix** (above) — the condition encoding changed from `impl FnMut` to
  `impl System` + the IS2 identity blanket.
- **`Copy` → `Clone`** on `StateTransitionRecord`/`Transition` — `States` bounds
  `Clone`, not `Copy`, so `#[derive(Copy)]` would not compile.
- **`StateTransitionRecord`/`Transition` made `pub`** — the L1 `private_bounds`
  fallback (they appear in the public `impl System` condition bound); opaque PODs,
  fields stay `pub(crate)`, only the read-only `current()` is public.

## Deferred (design hooks preserved — plan §11)

Value-keyed sub-schedules (shape a), computed/sub-states, state-scoped entity
auto-despawn, `StateTransitionEvent<S>`, `#[derive(States)]`, `Option<Res<R>>`
SystemParam, auto-`StateTransitionSet` ordering. All addable non-breakingly.

## Pipeline

research → architect → critic (REVISE: 2 HIGH + 4 MEDIUM, all decision-clarity,
no redesign) → critic R2 (APPROVED) → developer (2 sequential waves: module+API
steps 1–11, builder+executor steps 12–13) → code-review (APPROVED, nits) → tester
(found F1 CRITICAL) → developer (F1 fix) → code-review (APPROVED) → tester
(re-validate, PASS). Zero new `unsafe`; 0%-regression held throughout.
