# Phase 17 — States / State Transitions

> Branch `ecs`. Bevy-style application/game states layered on the existing single `Schedule`. Implements the boyko-native **shape (b)**: enter/exit logic is ordinary condition-gated systems in the one schedule, fed by a built-in per-frame transition pass. Composes with Phase 15 (sets/ordering) and Phase 16 (run conditions) with **zero executor hot-path changes**.

This plan is implementation-ready. A `developer` agent follows it verbatim; no architecture-level decisions remain open (see §12 for the one micro-decision left to the developer, with a default).

---

## 1. Scope

**In**

- `States` trait (marker + bounds) — hand-impl, no derive (D1).
- `State<S>` resource — current value; `NextState<S>` resource — queued request (D2).
- A built-in **transition pass** that runs once per `Schedule::run`, before condition eval, recording per-`S` "exited/entered this frame" (D3, D4).
- Run conditions `in_state(s)`, `on_enter(s)`, `on_exit(s)` (D5, D8).
- Builder + world entry points: `ScheduleBuilder::init_state::<S>()` / `insert_state(value)` and `EcsMaster::insert_state(value)` / `init_state::<S>()` (D7, §3).
- Initial-state `OnEnter(initial)` fired exactly once on the first `Schedule::run` via a synthesized `none → initial` transition (D7).
- Multiple orthogonal `State<S>` types, fully generic, per-type record (D9).
- Identity-transition default = no-op; **re-entry deferred** (D6).
- `OnTransition` middle hook **included** — one extra cold compare per registered state (D11).

**Out (deferred — design hooks in §11)**

- Value-keyed sub-schedules (Bevy shape (a) / `Schedules` map / `try_run_schedule`).
- Computed states, sub-states, state-scoped entity auto-despawn.
- `StateTransitionEvent<S>` as a real `Event`.
- `#[derive(States)]` proc-macro.
- `Option<Res<R>>` SystemParam (we choose require-exists for `in_state`, D8).
- A built-in `StateTransitionSet` that auto-orders every enter/exit system before `Update` (we choose user-ordered for the first cut, D10).

---

## 2. Decisions

### D1 — `States` trait: bounds + hand-impl (no derive now)

**Choice.** Add a marker trait

```rust
pub trait States: Send + Sync + Sized + Clone + PartialEq + Eq + std::hash::Hash + 'static {}
```

Ship **no** `#[derive(States)]`. Users write `impl States for MyEnum {}`.

**Justification.** The `SystemSet` derive (`boyko_macros::system_set_macro`) only earns its keep because it overrides `set_discriminant`/`set_name` per enum variant. A plain `States` type enumerates nothing (Bevy's `#[derive(States)]` likewise emits an empty body for non-computed states). The bounds are all the machinery needs: `PartialEq + Eq` for the transition comparison (`next != current`), `Clone` to move the value `Pending(S) → State<S>` and to capture it by value into the `in_state(s)`/`on_enter(s)`/`on_exit(s)` closures, `Hash` reserved for the deferred sub-state/computed-state map keys (kept now so adding them later is non-breaking), `Send + Sync + 'static` because `State<S>`/`NextState<S>` are `Resource`s carried across Phase-9 workers. A derive that adds only a bound check is pure cost (extra macro, extra compile surface, extra trybuild tests) with no codegen benefit — rejected. A developer who wants `#[derive(States)]` ergonomics later adds it non-breakingly (it would just emit `impl States for #name {}`). The `Hash` bound specifically is a **deliberate forward-compat parity decision, not an accident**: it mirrors Bevy's `States: Hash` and is zero-cost here (a derive on a fieldless enum that is never `.hash()`-ed this phase), kept solely so the deferred computed/sub-state map keys (§11) add non-breakingly.

**Alternative rejected.** A derive that also auto-implements `Default` to pick the initial variant — rejected: the initial state is an explicit `init_state` argument (D7), not a type-level default, so `Default` would be a redundant second source of truth and a footgun if the two disagreed.

### D2 — `State<S>` / `NextState<S>` representation

**Choice.**

```rust
#[repr(transparent)]
pub struct State<S: States>(S);                 // current value

pub enum NextState<S: States> {                 // queued request
    Unchanged,
    Pending(S),
}
```

Both implement `Resource` (D3 explains the id-minting subtlety). `State<S>` is `#[repr(transparent)]` so it is layout-identical to `S` (no wrapper cost; a `Res<State<S>>` deref is a no-op pointer reuse). `NextState<S>` defaults to `Unchanged` via a manual `Default` impl (used by `init_state`).

**Justification.** This is Bevy's exact two-resource split, the researcher's ground truth. Keeping current-vs-request as two separate resources means: (i) the transition pass needs `&mut` on exactly two slots, never on user data; (ii) `in_state` reads only `State<S>` (shared), so multiple `in_state`-gated systems never conflict on the request slot; (iii) `NextState<S>` writes from user systems go through `ResMut<NextState<S>>` which the conflict graph already serialises. SoA-at-the-resource level: the hot read path (`in_state`) touches one cache line (`State<S>` = `size_of::<S>()`, typically ≤ 8 B); the request slot is touched only by writers + the once/frame pass.

**Alternative rejected.** A single `StateData<S> { current, next }` resource — rejected: it forces every `in_state` reader to share a slot with every `NextState` writer, manufacturing false conflicts in the Phase-9 graph and serialising otherwise-parallel systems. Splitting is strictly more parallel.

### D3 — `Resource` id-minting for the generic `State<S>` / `NextState<S>` (the static-collapse trap)

**Choice.** `State<S>` and `NextState<S>` get their `ResourceId` through a **`TypeId`-keyed process-global registry**, NOT through a `static ID: OnceLock<ResourceId>` inside the generic `resource_id()` body, and NOT through `#[derive(Resource)]`.

```rust
// crates/boyko_ecs/src/ecs/core/state/state_resource_registry.rs  (NEW)
static REGISTRY: OnceLock<Mutex<HashMap<TypeId, ResourceId>>> = OnceLock::new();

pub(crate) fn resource_id_for<T: Resource>() -> ResourceId {
    let reg = REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = reg.lock().expect("state resource registry poisoned");
    if let Some(&id) = guard.get(&TypeId::of::<T>()) { return id; }
    // register_new does NOT call T::resource_id(); it mints from
    // NEXT_RESOURCE_ID + stores ResourceInfo::new_static::<T>(). No recursion.
    let id = ResourceId(resource_registry::register_new::<T>());
    guard.insert(TypeId::of::<T>(), id);
    id
}

impl<S: States> Resource for State<S> {
    fn resource_id() -> ResourceId { resource_id_for::<State<S>>() }
}
impl<S: States> Resource for NextState<S> {
    fn resource_id() -> ResourceId { resource_id_for::<NextState<S>>() }
}
```

**Justification.** This is the single most important correctness decision and it is forced by a verified Rust language fact, documented already in this repo at `crates/boyko_ecs/src/ecs/core/iters/query/query_type_registry.rs` lines 10–39 (citing rust-lang/rust#22991 and rfcs#2130): **a `static` declared inside a generic function body is NOT monomorphised — every instantiation shares one static.** If `State<S>::resource_id()` used the `#[derive(Resource)]`-style `static ID: OnceLock<…>` (see `boyko_macros::resource_macro`), every distinct `S` would collapse to the SAME `ResourceId`, so `State<AppState>` and `State<MenuState>` would alias one resource slot — silent, catastrophic. The `query_type_registry` already solved exactly this for `(D,F)` query pairs with a `OnceLock<Mutex<HashMap<TypeId, …>>>`; we reuse that proven pattern verbatim, keyed by `TypeId::of::<State<S>>()` / `TypeId::of::<NextState<S>>()`. Cost is paid at most once per `S` per process at registration (a `Mutex::lock` + `HashMap` probe, ~20–30 ns, cold), and never on the steady-state hot path — `Res<State<S>>::get_param` caches the resolved `ResourceId` in `ResState<State<S>>` at `init_state` (verified `crates/boyko_ecs/src/ecs/core/system/params/res.rs` lines 90–97 "pay the resource_id() load once at init"). So every per-frame `in_state` read goes through the cached id with zero map traffic.

`register_new::<State<S>>()` is safe to call here because `State<S>: Resource` and `register_new` does not re-enter `resource_id()` (verified `resource_registry.rs:164` — it mints from `NEXT_RESOURCE_ID` and stores `ResourceInfo::new_static`). The `TypeId`-keyed HashMap in front of it provides the monomorphisation-correct caching that the collapsing `static` cannot.

**Alternative rejected.** `#[derive(Resource)]` on `State<S>`/`NextState<S>` — impossible (derive emits the collapsing `static`), and the derive macro rejects nothing about generics so the bug would be silent. A per-`S` `OnceLock` field — there is nowhere to put a per-monomorphisation static without language support; the `TypeId` map IS the per-monomorphisation table.

### D4 — Transition-record data structure (what `on_enter`/`on_exit` read)

**Choice.** A **per-`S` resource** `StateTransitionRecord<S>` holding the last transition observed this frame:

```rust
#[derive(Clone, Copy)]
pub(crate) struct StateTransitionRecord<S: States> {
    /// Set by the transition pass on the frame a transition fired; `None`
    /// otherwise (cleared at the start of every transition pass).
    transition: Option<Transition<S>>,
    /// Frame tick the `transition` was recorded for. Defensive guard.
    /// NOT a change-detection tick (pitfall ii).
    recorded_tick: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct Transition<S: States> {
    pub(crate) exited: Option<S>,   // None only for the synthesized initial transition
    pub(crate) entered: S,
}
```

`StateTransitionRecord<S>` is itself a `Resource` (id minted via the same D3 registry, keyed `TypeId::of::<StateTransitionRecord<S>>()`). It is inserted by `init_state`/`insert_state` alongside `State<S>`/`NextState<S>`.

**Justification.**

- **Generic over `S`, per-type:** one record resource per `State<S>` type ⇒ orthogonal states are fully independent (D9) with no shared structure to contend on.
- **Allocation-free on the steady-state hot path:** the record is a tiny POD (`Copy`), stored in the existing 256-slot resource slab. No `Vec`/`HashMap`/`Box` per frame. Writing the record is one slab write to a cached `ResourceId`.
- **0-cost when no states are used:** if no `State<S>` is ever registered, no `StateTransitionRecord<S>` resource exists, the schedule's state-pass registry is empty, and the pass early-outs (§4 `has_states`). No record, no branch on the hot path.
- **Reset:** the transition pass clears `transition = None` at its own start each frame (it is the sole writer), so `on_enter`/`on_exit` read `Some(..)` only on the exact frame the pass recorded it. This is a plain `Option` read — **not** tick-based, so pitfall (ii) cannot apply. `recorded_tick` is belt-and-suspenders, compared to the dispatcher's `this_run`, never used as a "changed since" window.

**Storage location — resource, not a `Schedule`/world field.** The record lives in the resource slab (a `Resource`), NOT as a typed field, because (i) it must be generic over arbitrarily many `S` — a typed field cannot be, only a `TypeId`-addressed slab can; (ii) the conditions read it via the existing `Res<…>` SystemParam machinery for free; (iii) it costs zero when unused (absent slab slot). A `HashMap<TypeId, …>` on the world is rejected for the steady-state read — the resource slab IS the `TypeId`→slot map, O(1) by cached id.

**Why a map is acceptable in the COLD transition path but not the hot read.** The pass walks the schedule's **registered-states list** (`Vec<StateEntry>`, §4), tiny (1–4 entries). Not a HashMap, runs at most once per frame. The only map touched is the D3 `TypeId` registry, hit once per `S` at registration (cold). The per-frame read path (`in_state`/`on_enter` → `Res<…>` → cached `ResourceId` → slab index) has no map at all.

### D5 — `in_state` / `on_enter` / `on_exit` condition signatures

> **F1 AMENDMENT (post-implementation — the shipped encoding differs from the snippet below).** The planned return type `impl FnMut(Res<…>) -> bool + Clone` does **NOT** compile through `.run_if(..)`: boyko's `SystemParamFunction` blanket (`function_system_impls.rs`) requires the double-`FnMut` HRTB bound (`FnMut(P) -> Out` **and** `for<'w,'s> FnMut(<P as SystemParam>::Item<'w,'s>) -> Out`), and an opaque `impl FnMut` return type carries only the first bound — the HRTB-projected second one is lost across the `impl Trait` boundary, so `IntoSystem<(), bool, M>` is never satisfied. (An inline closure works because rustc sees the concrete type.) **Shipped fix:** (1) the four conditions return `impl System<Out = bool>` — the closure is concretized *inside* the body (where both bounds resolve) and wrapped via `IntoSystem::into_system(..)`, then exposed as `impl System` (a plain trait that survives the opaque boundary); (2) the long-anticipated **IS2 identity blanket** `impl<S: System> IntoSystem<(), <S as System>::Out, S> for S` was added to `into_system.rs` so the returned `impl System` re-bridges to `IntoSystem` for `.run_if`. Zero new `unsafe`, zero runtime cost (identity `into_system` + still boxed into `BoolSystem`), coherent (marker `S` is disjoint from `(IsFunctionSystem, M)` / `(ExclusiveSystemMarker, _)`), no inference regression (closures still route through the function/exclusive blankets). The comparison-logic bodies below are byte-identical to what shipped — only the return type + `into_system` wrapping changed. **Lesson:** the headline API (`.run_if(in_state(X))`) was never compiled end-to-end in-crate, so the gap passed architect + critic + dev + review; the tester caught it.

**Choice.** All three are free functions returning a closure that is `impl IntoSystem<(), bool, M>`, added to `common_conditions.rs` next to `run_once`:

```rust
pub fn in_state<S: States>(target: S) -> impl FnMut(Res<State<S>>) -> bool + Clone {
    move |current: Res<State<S>>| current.get() == &target
}

pub fn on_enter<S: States>(target: S) -> impl FnMut(Res<StateTransitionRecord<S>>) -> bool + Clone {
    move |rec: Res<StateTransitionRecord<S>>| matches!(rec.current(), Some(t) if t.entered == target)
}

pub fn on_exit<S: States>(target: S) -> impl FnMut(Res<StateTransitionRecord<S>>) -> bool + Clone {
    move |rec: Res<StateTransitionRecord<S>>| matches!(rec.current(), Some(t) if t.exited.as_ref() == Some(&target))
}
```

The record type stays `pub(crate)`; because the return is `impl Trait`, the type never appears in the public signature (callers only ever write `on_enter(MyState::X)`).

**Justification.**

- **Rides Phase 16 with zero executor changes.** Each is exactly the `run_once` shape: a closure whose params are `SystemParam`s ⇒ a `SystemParamFunction` ⇒ `IntoSystem<(), bool, M>` (verified `into_system.rs:78-89` + `res.rs` `Res: SystemParam`). Attached via the unchanged `SystemConfig::run_if` / `ConfigureSet::run_if` (`system_config.rs:183`, `schedule_builder.rs:614`), evaluated in the unchanged `evaluate_ready_conditions` barrier pass (`schedule.rs:506`), behind the unchanged `has_condition.is_clear()` 0%-gate (`schedule.rs:357`). No new condition machinery.
- **`in_state` is a pure equality read** of `State<S>` — no tick, sidestepping pitfall (ii). `on_enter`/`on_exit` are pure `Option`-pattern reads of the record the pass just wrote — also no tick.
- **Read-only ⇒ passes the Phase 16 `debug_assert_condition_read_only`** check (`schedule_builder.rs:707`, verified it checks `component_writes.is_empty() && resource_writes.is_empty()`): each declares only a `Res<…>` (resource read), no writes.
- **Per-frame `initialize` does not disturb the captured value — THE premise behind "rides Phase 16 with zero changes".** Phase 16 re-invokes `IntoSystem::initialize(self, world)` on the condition system on **every** evaluation (`ecs_master.rs:1781`; an FS1 no-op for `FunctionSystem`). For `run_once` the persisted bit lives in `FunctionSystem::state`'s `Local<bool>` and survives that re-init. The state conditions are stateful in a *different* place, and it survives for a different reason: (i) `target`/`from`/`to` are captured **by value into the boxed closure environment** inside `FunctionSystem` — they are part of the closure's own captured state, never touched by `initialize` (which only re-resolves `SystemParam` state, not closure captures); (ii) the only thing `initialize` re-runs for these is `Res<State<S>>`/`Res<StateTransitionRecord<S>>`'s `init_state`, which re-resolves `resource_id()` per frame — **idempotent**, since the D3 `TypeId` registry returns the *same* `ResourceId` for a given `S` every time, and it is the exact FS1-no-op cost every Phase-16 condition already pays; (iii) therefore the state conditions inherit `run_once`'s exact persistence semantics with no new mechanism — the captured value persists across frames precisely because it lives in the closure, not in re-initialized param state. This is the load-bearing reason D5 and D11 ride Phase 16 with zero executor changes.
- **`Clone` on the returned closure** for future combinator reuse; harmless today.

**Alternative rejected.** Making `on_enter`/`on_exit` read `State<S>` + a separate `PrevState<S>` resource and compare — rejected: re-derives the transition every member evaluation and needs an extra resource slot; the pre-computed record is one write/frame, read by all members.

### D6 — Identity transition default = no-op; re-entry deferred

**Choice.** When `NextState::Pending(v)` with `v == *current`, the pass records **no transition** (clears the record) and resets `NextState` to `Unchanged` — OnExit/OnTransition/OnEnter do NOT fire. **Opt-in re-entry is deferred** (no API this phase).

**Justification.** Matches Bevy's default. Firing enter/exit on `set(current_value)` is almost always a bug (double-init). The pass is written so re-entry is a non-breaking addition: add `NextState::PendingReenter(S)` + one branch recording `Transition { exited: Some(v), entered: v }` even when `v == current`. The record/condition layer already supports `exited == entered` (conditions compare `entered`/`exited` independently).

**Alternative rejected.** Always fire on any `Pending`, even identity — surprising, diverges from Bevy, breaks the common `set` idiom.

### D7 — Initial-state `OnEnter(initial)` at startup (synthesize `none → initial`)

**Choice.** `init_state::<S>()` / `insert_state(v)` insert `State<S> = initial`, `NextState<S> = Unchanged`, `StateTransitionRecord<S> = { transition: None }`, AND set a per-`S` `pending_initial: true` flag in the schedule's `StateEntry` (§4). On the **first** `Schedule::run`, the pass, for each entry with `pending_initial == true`, synthesizes `Transition { exited: None, entered: initial }`, records it, and clears `pending_initial`. So `on_enter(initial)`-gated systems run on frame 1; `on_exit(initial)` does not.

> **DECISION — pre-frame-1 `Pending` suppresses the initial `OnEnter` (accepted, documented prominently).** If a `Pending(req)` is queued for `S` **before the very first `Schedule::run`** (reachable only via a direct `EcsMaster::set_next_state::<S>(req)` between `build` and the first `run`), then on frame 1 the synthesized `none → initial` is recorded by step 2 and **immediately overwritten** by the real `initial → req` transition in step 3 (§5.1). Consequence: **`on_enter(initial)` does NOT fire — only `on_enter(req)` does.** We **keep this override** (option a): it is the same last-write-wins rule that governs every other frame, and special-casing frame 1 to fire two enters would diverge from the per-frame model and surprise the other direction. The surprise (the *initial* OnEnter being skipped) is mitigated by documenting it **on the API surface** (`set_next_state` and `init_state`/`insert_state` doc comments), not only here — see the doc-comment wording below.

**Doc-comment wording (apply to `EcsMaster::set_next_state`, `init_state`, `insert_state`, `ScheduleBuilder::init_state`, `insert_state`):**

```rust
/// # Initial-transition interaction
/// Calling `set_next_state::<S>(..)` (or otherwise queuing a `Pending`)
/// **before the first `Schedule::run`** suppresses the initial `OnEnter`:
/// the synthesized `none → initial` transition is overwritten in the same
/// first pass by the real `initial → requested` transition, so
/// `on_enter(initial)`-gated systems do NOT run — only `on_enter(requested)`
/// does. Queue the first transition from *inside* a system (it lands on the
/// next frame's pass) if you need the initial `OnEnter` to fire first.
```

**Justification.** Bevy fires `OnEnter(initial)` once at startup. Because boyko has **no `App` driver** (verified: user calls `ScheduleBuilder::build(world)` then `schedule.run(world)` manually — there is no startup schedule), the only universal place to fire it is the first `Schedule::run`. Synthesizing a `none → initial` transition reuses the same record + condition machinery — `on_enter(initial)` needs no special case. `exited: None` is the type-level signal "synthetic transition"; `on_exit` reads `exited.as_ref() == Some(&target)` so it is naturally false.

**Where the flag lives.** On the schedule's `StateEntry`, because "have we fired the initial yet" is per-schedule-run state (a state registered with two schedules must fire its own initial per schedule). NOT on the resource (world-global, shared across schedules).

**Alternative rejected.** A separate one-shot startup schedule (Bevy's `Startup`) — boyko has exactly one schedule; building a second + a startup-ran flag is a larger subsystem than synthesizing one transition, and multi-schedule machinery is out of scope.

### D8 — `Option<Res<R>>` vs require-exists for `in_state`

**Choice.** **Require-exists.** `in_state(s)` uses `Res<State<S>>`, which **panics** if `State<S>` was never inserted (verified `res.rs` → `missing_resource_panic`). Document: "register the state with `init_state::<S>()`/`insert_state` before adding any `in_state`/`on_enter`/`on_exit`-gated system."

**Justification.** boyko has **no `Option<Res<R>>` SystemParam** (verified: deferred in Phase 16, `common_conditions.rs:9-13` explicitly notes `resource_exists::<R>` is blocked on it). Adding it is its own SystemParam design task (conditional access decl; `None` without UB) — out of scope and not required: a state used in a condition is always one the user explicitly registered, so "missing" is a setup bug that SHOULD panic loudly, exactly like a missing resource anywhere else. Filing `Option<Res<R>>` as deferred (§11) keeps the door open: switching `in_state` to it later is a non-breaking internal change (same public signature).

**Alternative rejected.** Add `Option<Res<R>>` now — scope creep into the SystemParam layer; panic-on-missing is the correct severity for a registration bug.

### D9 — Multiple orthogonal `State<S>` types

**Choice.** Everything is generic over `S`. Each `State<S>`/`NextState<S>`/`StateTransitionRecord<S>` is a distinct resource with a distinct `ResourceId` (D3 map keyed by distinct `TypeId`s). The schedule holds a `Vec<StateEntry>` with one entry per registered `S` (§4); the pass loops over all entries. Conditions are generic, so `in_state(AppState::Menu)` and `in_state(NetState::Online)` coexist trivially.

**Justification.** Falls out of the per-type resource design (D2/D4) — no extra mechanism. The only shared structure is the schedule's `Vec<StateEntry>`, type-erased (each entry holds a monomorphised `fn(&mut EcsMaster, u32, bool)` apply pointer) so the schedule stays non-generic (mirrors how `SystemBox` erases `dyn System`), supporting N state types.

**Alternative rejected.** A single global state type — real games need orthogonal axes (app phase × network × pause). Generic-over-`S` is mandatory and free here.

### D10 — Ordering of enter/exit systems vs normal systems

**Choice.** **User-ordered** (no built-in auto-ordering for the first cut). An `on_enter`-gated system is an ordinary system; it runs in topological order per its own `.before/.after/.in_set`. The pass runs **before the executor loop** (§6), so the record is already written when ANY system is evaluated — no ordering hazard between "transition recorded" and "enter system runs".

**Justification.** Because the record is written by the pass **before** the first dispatch, every `on_enter(s)`-gated system sees the correct record regardless of its topo position — a built-in ordering set is NOT needed for correctness. It would only matter for the *relative* order of enter-systems vs normal-systems, which `.before/.after/.in_set` (Phase 15) already expresses. Auto-creating a `StateTransitionSet` forcing all enter/exit systems `.before(everything)` would (i) duplicate Phase-15, (ii) impose a global ordering edge, (iii) risk false-conflict serialisation, (iv) perturb every state-using schedule's conflict graph (violating §7). Provide the **building block** instead: a public `pub struct StateTransitionSet;` unit `SystemSet` users MAY opt into — zero cost if unused, full Phase-15 power if used. Including the marker (but not auto-wiring it) is the non-breaking hook for a future opt-in.

**Alternative rejected.** Auto-insert a `StateTransitionSet` ordered before all non-state systems — see (i)–(iv).

### D11 — Include `OnTransition`

**Choice.** **Include** an `on_transition(from, to)` condition reading the same record:

```rust
pub fn on_transition<S: States>(from: S, to: S) -> impl FnMut(Res<StateTransitionRecord<S>>) -> bool + Clone {
    move |rec: Res<StateTransitionRecord<S>>|
        matches!(rec.current(), Some(t) if t.exited.as_ref() == Some(&from) && t.entered == to)
}
```

**Justification.** Essentially free: the record already carries both `exited` and `entered`, so `on_transition` is one more pure read with the same shape as `on_enter`/`on_exit` — no new storage, no new pass logic, no hot-path cost (another optional condition behind the same 0%-gate). Rounds out Bevy parity at near-zero cost; deferring it would mean a whole extra critic/dev/test cycle for one function.

**Alternative rejected.** Defer `OnTransition` — the marginal cost is one tiny function; the record already supports it.

---

## 3. Public API

Module `boyko_ecs::ecs::core::state` (new).

### 3.1 Trait + types

```rust
// state/states.rs
pub trait States: Send + Sync + Sized + Clone + PartialEq + Eq + std::hash::Hash + 'static {}

// state/state.rs
#[repr(transparent)]
pub struct State<S: States>(S);
impl<S: States> State<S> {
    pub fn get(&self) -> &S;
    pub fn new(value: S) -> Self;
}
impl<S: States> Deref for State<S> { type Target = S; /* &self.0 */ }

// state/next_state.rs
pub enum NextState<S: States> { Unchanged, Pending(S) }
impl<S: States> NextState<S> {
    pub fn set(&mut self, value: S);          // last-write-wins within a frame
    pub fn pending(&self) -> Option<&S>;
}
impl<S: States> Default for NextState<S> { /* Unchanged */ }
```

### 3.2 Conditions (in `common_conditions.rs`, re-exported from `schedule`)

```rust
pub fn in_state<S: States>(s: S)              -> impl FnMut(Res<State<S>>) -> bool + Clone;
pub fn on_enter<S: States>(s: S)              -> impl FnMut(/*record*/) -> bool + Clone;   // opaque return
pub fn on_exit<S: States>(s: S)               -> impl FnMut(/*record*/) -> bool + Clone;
pub fn on_transition<S: States>(from: S, to: S) -> impl FnMut(/*record*/) -> bool + Clone;
```

(The record type stays `pub(crate)`; `impl Trait` return hides it from the public name.)

### 3.3 Entry points

```rust
impl EcsMaster {
    pub fn insert_state<S: States>(&mut self, initial: S);   // #[cold]
    pub fn init_state<S: States + Default>(&mut self);       // #[cold]
    pub fn state<S: States>(&self) -> &S;                    // #[inline], panics if unregistered
    pub fn set_next_state<S: States>(&mut self, value: S);   // shorthand for resource_mut::<NextState<S>>().set
}

impl ScheduleBuilder {
    pub fn insert_state<S: States>(&mut self, initial: S) -> &mut Self;
    pub fn init_state<S: States + Default>(&mut self) -> &mut Self;
}

// state/state_set.rs — opt-in ordering hook (D10), NOT auto-wired.
pub struct StateTransitionSet;
impl SystemSet for StateTransitionSet {}
```

**Split rationale.** `EcsMaster::insert_state` mutates the world (inserts resources). `ScheduleBuilder::insert_state` additionally records a `StateEntry` (§4) so the schedule's pass processes `S` and fires the initial OnEnter. The builder cannot insert resources at call time (it holds no `&mut EcsMaster`); it records the request in a `Vec<StateRegistration>` and `try_build(world)` drains it, calling `world.insert_state::<S>(initial)` and building `Vec<StateEntry>` (verified `try_build(self, world: &mut EcsMaster)` receives the world). This mirrors how Phase-16 set-conditions are recorded-then-realised.

**Idempotency contract (registering the same `S` twice).** Calling `init_state::<S>()` / `insert_state::<S>(..)` more than once for the **same** `S` on the **same** builder is **idempotent — the second and later calls are a no-op**. The builder checks for an existing registration of `S` (by `TypeId`) in its `Vec<StateRegistration>` and skips the duplicate, guaranteeing **exactly one `StateEntry` per `S`** in the built `Schedule`. This is mandatory for correctness: a double-insert would push two `StateEntry`s for one `S`, so the transition pass would synthesize the `none → initial` transition **twice** (and later drain `NextState<S>` twice per frame). The *value* of a duplicate `insert_state(v2)` after `insert_state(v1)` is ignored (first registration wins); document this on the builder methods. The world-side `EcsMaster::insert_state` (which only inserts resources) follows the slab's existing replace semantics and is not the source of truth for entry count — the builder's dedup is.

### 3.4 Usage snippets

```rust
// 1. Define states (hand-impl; no derive).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
enum AppState { #[default] Menu, InGame, Paused }
impl States for AppState {}

// 2. Register + gate.
let mut builder = ScheduleBuilder::new(pool);
builder.init_state::<AppState>(); // initial = Menu (Default)
builder.add_system(show_menu).run_if(on_enter(AppState::Menu));   // fires frame 1 (synth)
builder.add_system(tick_menu).run_if(in_state(AppState::Menu));
builder.add_system(start_game).run_if(in_state(AppState::Menu));  // sets NextState
builder.add_system(spawn_level).run_if(on_enter(AppState::InGame));
builder.add_system(teardown_menu).run_if(on_exit(AppState::Menu));
builder.add_system(run_physics).run_if(in_state(AppState::InGame));
let mut schedule = builder.build(&mut world);
loop { schedule.run(&mut world); } // transition pass runs at top of each run

// 3. Request a transition from inside a system.
fn start_game(input: Res<Input>, mut next: ResMut<NextState<AppState>>) {
    if input.pressed_enter() { next.set(AppState::InGame); } // applied next frame's pass
}

// 4. Orthogonal states — fully independent.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
enum NetState { #[default] Offline, Online }
impl States for NetState {}
builder.init_state::<NetState>();
builder.add_system(sync).run_if(in_state(NetState::Online));

// 5. Opt-in ordering (D10).
builder.add_system(spawn_level).run_if(on_enter(AppState::InGame)).in_set(StateTransitionSet);
builder.configure_set(StateTransitionSet).before(GameplaySet);
```

---

## 4. Data structures & storage

### 4.1 Resources (in the world's resource slab — `TypeId`-addressed, preallocated)

| Resource | Layout | Lifetime / writer |
|---|---|---|
| `State<S>` | `repr(transparent)` over `S` | Inserted at register; written ONLY by the pass. Read (shared) by `in_state`. |
| `NextState<S>` | `enum { Unchanged, Pending(S) }` | Written by user systems via `ResMut`; reset to `Unchanged` by the pass. |
| `StateTransitionRecord<S>` | `{ transition: Option<Transition<S>>, recorded_tick: u32 }`, `Copy` | Written ONLY by the pass. Read (shared) by `on_enter`/`on_exit`/`on_transition`. |

All three sit in the existing 256-slot resource slab (`resources.rs`), so they are **preallocated** (no per-frame alloc) and addressed by a cached `ResourceId` (resolved once at each condition's `init_state`). **No size cap on `S`** — the slab stores `*mut u8` to a heap `Box<R>`, not inline (verified `ResourceSlot { ptr, drop_fn, layout }`); the 32 B slot-metadata budget is `S`-independent.

**Slot budget (deliberate, accept-as-is).** The resource slab is **hard-capped at `RESOURCE_SLOT_COUNT = 256`** slots, locked to `BitSet256` width (`resource_registry.rs:51`) and shared by ALL engine + user resources, all minted from one global `NEXT_RESOURCE_ID`. **Each registered state type `S` burns exactly 3 slots** — `State<S>`, `NextState<S>`, `StateTransitionRecord<S>` — so N state types cost `3N` of the shared 256. This is accepted: a real game has a single-digit number of orthogonal state axes (app phase × network × pause ≈ 3–6 types ⇒ 9–18 slots), leaving ample headroom for ordinary resources. On exhaustion `register_new` does **not** corrupt memory — it `assert!`-panics loudly (`resource_registry.rs:176-180`, message `"ResourceRegistry exhausted: NEXT_RESOURCE_ID reached {raw}, RESOURCE_SLOT_COUNT = 256"`) at registration time (`#[cold]` setup path), never in the frame loop. The record is kept ON the slab (not moved to a side table) because doing so keeps D4/D5 intact — the conditions read it through the same cached-`ResourceId` `Res<…>` path with zero extra mechanism; a 3rd slot per state is the correct price for that uniformity. **3 slots/state is the accepted cost.**

### 4.2 Schedule-side registry (type-erased, one entry per registered `S`)

```rust
pub(crate) struct StateEntry {
    /// Monomorphised transition-apply, erased to a fn pointer so `Schedule`
    /// stays non-generic. Signature: fn(&mut EcsMaster, this_run, fire_initial).
    apply: fn(&mut EcsMaster, u32, bool),
    pending_initial: bool,        // true until the first run fires the synthesized initial OnEnter (D7)
    type_name: &'static str,      // diagnostics
}

pub struct Schedule {
    // ... existing fields ...
    pub(crate) state_entries: Vec<StateEntry>,   // EMPTY for a no-state schedule ⇒ pass early-outs
}
```

`apply` = `apply_state_transition::<S>` (§5). Built once at `build`, never mutated per frame (only `pending_initial` flips on frame 1). Not a HashMap — a linear walk over ≤ 4 fn-pointers once/frame is cheaper than any map and trivially cache-resident. **Field placement is load-bearing for §6.3 layout-neutrality:** the developer MUST append `state_entries` as the **last** field of `Schedule`, AFTER the current last field (`set_conditions`), so the hot `pool → systems → conflict_graph → executor_scratch → has_condition` field prefix (whose ordering is documented load-bearing at `schedule.rs:67-75`) stays **byte-for-byte unchanged in offset** — adding a 24 B `Vec` at the tail cannot shift any pre-existing hot field across a cache line. Confirm via the §7 bench, not by inspection.

### 4.3 The 0-cost-when-unused story

- No `State<S>` registered ⇒ `state_entries` empty ⇒ the §6 pass is one `if self.state_entries.is_empty()` (length compare, predicted-not-taken), the spiritual twin of Phase 16's `has_condition.is_clear()`. `has_states` ≡ `!self.state_entries.is_empty()`.
- No state resources ⇒ no extra slab traffic.
- No state conditions ⇒ `has_condition` stays clear ⇒ Phase-16 gate stays at 0% too.

---

## 5. The transition algorithm

### 5.1 Per-`S` monomorphised apply (the erased `StateEntry::apply`)

```text
fn apply_state_transition::<S: States>(world, this_run, fire_initial):
    # 1. Clear the record FIRST so a no-transition frame leaves transition = None.
    rec = world.resource_mut::<StateTransitionRecord<S>>()
    rec.transition = None; rec.recorded_tick = this_run

    # 2. Synthesized initial transition (D7).
    if fire_initial:
        initial = world.resource::<State<S>>().get().clone()
        world.resource_mut::<StateTransitionRecord<S>>().transition =
            Some(Transition { exited: None, entered: initial })
        # fallthrough to step 3 so an initial + immediately-queued Pending both apply.

    # 3. Drain NextState.
    match take(world.resource_mut::<NextState<S>>()):   # sets next = Unchanged
        Unchanged: return
        Pending(v):
            current = world.resource::<State<S>>().get().clone()
            if v == current: return                      # D6 identity → no-op
            rec = world.resource_mut::<StateTransitionRecord<S>>()
            rec.transition = Some(Transition { exited: Some(current), entered: v.clone() })
            rec.recorded_tick = this_run
            *world.resource_mut::<State<S>>() = State::new(v)
```

- **Fire-exactly-once.** `take(next)` resets to `Unchanged` in the same pass; the pass runs once per `Schedule::run` (§6) ⇒ each transition recorded exactly once. The next frame's pass clears the record back to `None` (step 1) before any condition runs ⇒ no double-fire across frames.
- **Skip identity.** Step 3's `v == current` guard (D6).
- **Last-write-wins.** `set(a); set(b)` leaves `Pending(b)` (plain overwrite); the pass drains the final value. One transition per frame.
- **Initial + immediate Pending (semantics pinned in D7).** If a `Pending` is queued before frame 1 AND `fire_initial`: step 2 records the initial `entered`, then step 3 **overwrites** it with the real `initial → req` transition + swaps `State`. The same-frame override wins, so `on_enter(initial)` is suppressed and only `on_enter(req)` fires — this is the **accepted decision** in the D7 callout, documented on `set_next_state`/`init_state`/`insert_state`, not merely noted here.

### 5.2 Ordering vs tick bump and condition eval

The pass MUST run: **after** `world.bump_change_tick()` (so `this_run` is available to stamp `recorded_tick`; verified `schedule.rs:173`); **after** the per-system + per-condition `set_change_ticks` loops (disjoint borrow — those mutate `self.*`, the pass mutates `world`); **before** `pool.install(...)` so the record is written before ANY system/condition is evaluated ⇒ `evaluate_ready_conditions` (inside the loop, inside `install`) observes it the same frame.

Exact insertion: immediately after the Phase-16.1 set-condition tick loop (`schedule.rs:226-229`) and before the `if self.systems.is_empty()` check (`schedule.rs:231`). See §6.

### 5.3 Startup initial synthesis

Handled by `pending_initial` per `StateEntry` (D7/§4). The schedule passes `entry.pending_initial` into `apply`, then sets it `false`. Per-entry ⇒ a state shared by two schedules fires its initial once per schedule (each has its own entry).

---

## 6. Integration with `Schedule::run`

### 6.1 The new gated pass

```rust
impl Schedule {
    /// Runs each registered state's transition apply once. Cold: ≤
    /// state_entries.len() (~4) monomorphised applies per frame. Gated by
    /// state_entries.is_empty() at the call site (THE 0%-gate, §7).
    #[cold]
    fn run_state_transitions(&mut self, world: &mut EcsMaster, this_run: u32) {
        for entry in self.state_entries.iter_mut() {
            let fire_initial = entry.pending_initial;
            entry.pending_initial = false;
            (entry.apply)(world, this_run, fire_initial);
        }
    }
}
```

Call site inside `Schedule::run`, between the Phase-16.1 set-condition tick loop and the empty-schedule short-circuit:

```rust
// ... existing: bump_change_tick (this_run), check_ticks, per-system + per-condition
//     set_change_ticks loops (schedule.rs:173-229) ...

// Phase 17 — state transition pass. THE 0%-GATE: a no-state schedule has
// state_entries empty ⇒ one is_empty() compare, predicted-not-taken. Runs
// BEFORE the executor loop so evaluate_ready_conditions (Step 1.5) observes
// the freshly-written record the SAME frame. Reuses `this_run` (no 2nd bump).
// Holds the dispatcher's unique &mut world; no worker exists yet
// (pool.install not entered) ⇒ trivially race-free, no cell, no unsafe.
if !self.state_entries.is_empty() {
    self.run_state_transitions(world, this_run);
}

if self.systems.is_empty() { return; }
// ... existing: pool.install(|scope| self.executor_main_loop(world, scope)) ...
```

### 6.2 How the pass gets `&mut EcsMaster`

**Directly.** `Schedule::run(&mut self, world: &mut EcsMaster)` already holds `&mut world` (verified `schedule.rs:155`). The pass runs **before** `pool.install` mints the `UnsafeEcsCell`, so it borrows `world` mutably — **no cell, no `unsafe`, no barrier**. Strictly simpler than the Phase-16 condition path (which needs the apply-window reborrow because it runs *inside* the loop). The pass is sequenced entirely before any worker exists.

### 6.3 0%-regression contract

- **Instruction-identity:** `try_dispatch_ready`, `apply_window_drain`, `evaluate_ready_conditions`, `set_gate`, `mark_skipped`, `SystemBox`, `ExecutorScratch`, `SpawnPointers` emit **BYTE-IDENTICAL instructions**. The state pass adds exactly one `if self.state_entries.is_empty()` branch on the once-per-frame preamble path (NOT in the per-round dispatch loop, NOT in the per-system path). The executor loop body is untouched; `state_entries` is read only in the gated preamble, never inside the hot loop.
- **Layout-neutrality (a distinct claim — instruction-identity does NOT imply it):** adding the `state_entries` field could in principle shift the offsets of `executor_scratch`/`has_condition` and push a hot field across a cache line (the field-order note at `schedule.rs:67-75` says ordering matters). This is neutralised structurally by appending `state_entries` as the **last** `Schedule` field (after `set_conditions`, §4.2), so every pre-existing field keeps its exact offset. The §7 bench validates layout-neutrality empirically; the instruction-identity above is validated by reading the emitted asm of the executor loop.
- Verification: the 50-systems criterion bench (the Phase 9/12.5/15/16 yardstick) must show "no change detected" vs current HEAD — the only added instruction on a no-state schedule is one `Vec::is_empty` once per `run`, dwarfed by the existing per-frame `reset_for_frame` + tick loops.

---

## 7. Hot-path & 0%-regression analysis

**Zero cost when no state is registered.** A no-state schedule's added code is one `if self.state_entries.is_empty()` per `Schedule::run` (once/frame, not per round, not per system): one `Vec`-len load + compare + predicted-taken skip, next to the already-present per-frame loops whose cost dominates by orders of magnitude. No state resources ⇒ no slab traffic; `has_condition` stays clear ⇒ Phase-16 gate at 0% too.

**States-present cost paid only where used.** The pass: ≤ ~4 calls, each a handful of slab reads/writes to cached ids, ~tens of ns total, once/frame, `#[cold]`. Each `in_state`/`on_enter`/`on_exit` condition: one `Res<…>` get_param (cached id → slab pointer, ~3 ns per `res.rs`) + an equality/`Option`-match, run once per gated system per frame at the barrier. **No per-entity cost** — these gate whole systems, not rows.

**No new branch on the steady-state per-entity or per-round path.** Only the per-frame `is_empty()` gate, elided to ~nothing when unused.

**Bench A/B plan.**
1. **0%-gate regression:** rerun the 50-systems schedule bench on this branch vs HEAD; criterion must report "no change" / ±2% (the standing Phase-15/16 contract). No state registered.
2. **States-present microbench (new `benches/phase17_states.rs`):** (a) `transition_pass_cost` — 1 and 4 registered states, measure `Schedule::run` delta vs the same schedule with states unregistered (target < 100 ns/frame for 4 states). (b) `in_state_gate_cost` — N systems all `.run_if(in_state(active))` vs ungated; per-frame condition-eval delta linear in N at the Phase-16 per-condition cost.

---

## 8. `unsafe` audit

**Target: zero new `unsafe`. Achieved.**

- The pass borrows `world: &mut EcsMaster` directly (it runs before `pool.install`) — no `UnsafeEcsCell`, no raw pointers, no reborrow — safe Rust.
- Resource access uses the existing safe facades `EcsMaster::resource`/`resource_mut` (their internal `unsafe` is pre-existing, unchanged).
- Conditions are ordinary `SystemParamFunction`s; their `Res<…>` get_param `unsafe` is pre-existing Phase-8a code, unchanged.
- The D3 `TypeId` registry is safe (`OnceLock` + `Mutex` + `HashMap`), identical to the existing safe `query_type_registry`.

If the three-arg fn-pointer erasure (`StateEntry::apply`) needs a coercion, use a plain monomorphised free function coerced to `fn(&mut EcsMaster, u32, bool)` — a safe reified-fn coercion, **no `unsafe`**. Do not introduce `unsafe` for the erasure.

---

## 9. Testing strategy

Unit tests beside the code; integration in `crates/boyko_ecs/tests/phase17_states.rs`; Miri runs that file.

**Unit (in `state/` modules)**
- `state_get_returns_value` — `State::new(v).get() == &v`; `Deref` round-trips.
- `next_state_set_overwrites` — `set(a); set(b)` ⇒ `pending() == Some(&b)`.
- `next_state_default_unchanged` — `NextState::<S>::default()` is `Unchanged`.
- `state_resource_ids_distinct_per_type` — `State::<A>::resource_id() != State::<B>::resource_id()` AND `State::<A>::resource_id() != NextState::<A>::resource_id()` (**the regression test for rust#22991** — FAILS if the collapsing `static` is used).
- `transition_record_layout_sanity` — informational `size_of` on `StateTransitionRecord<TestState>` (not a hard cap, §4.1).

**Integration (`tests/phase17_states.rs`)**
- `initial_on_enter_fires_once_at_startup` — `init_state` + system gated by `on_enter(initial)`; 3 frames; body ran exactly once (frame 1).
- `transition_fires_enter_and_exit_on_right_frame` — start `Menu`; frame 1 sets `NextState=InGame`; `on_exit(Menu)` + `on_enter(InGame)` run frame 2 only; `in_state(InGame)` from frame 2 on; `in_state(Menu)` stops after frame 1.
- `identity_transition_is_noop` — `set(current_value)`; NO enter/exit fires; `State` unchanged; `NextState` reset.
- `last_write_wins_one_transition_per_frame` — one frame `set(A); set(B)`; only `on_enter(B)` fires; final state `B`.
- `transition_fires_exactly_once` — counter gated by `on_enter(InGame)`; one transition; 5 frames; counter == 1.
- `in_state_gates_systems` — gated system runs iff current == target across Menu→InGame→Paused.
- `on_transition_fires_only_for_exact_pair` — `on_transition(Menu, InGame)` fires on Menu→InGame but not Menu→Paused nor InGame→Paused.
- `multiple_orthogonal_states_independent` — `AppState` + `NetState`; a `NetState` transition fires no `AppState` enter/exit and vice-versa.
- `no_states_zero_overhead_smoke` — a schedule with systems but no state runs normally (gate path inert).
- `interaction_with_phase15_ordering` — two `on_enter(InGame)` systems with `.before/.after` (or `.in_set(StateTransitionSet)` + `configure_set(...).before(...)`) run in declared order on the transition frame.
- `interaction_with_phase16_conditions` — a system gated by BOTH `in_state(InGame)` AND `run_once`: runs once, only while InGame (eager-AND fold, no short-circuit).
- `set_next_state_from_direct_api` — `EcsMaster::set_next_state` drives a transition on the next run.
- `init_state_twice_is_idempotent` — calling `init_state::<S>()` (or `insert_state`) twice for the same `S` on one builder yields exactly one `StateEntry`: the synthesized initial `on_enter(initial)` fires **once** (not twice) on frame 1, and a per-frame transition drains `NextState` once. Also covers the §13-OQ "built but never run" sub-question (a registered-but-unused state adds one inert entry).

**Miri** — `cargo +nightly miri test` on `tests/phase17_states.rs`. Expectation: clean (no new `unsafe`; only pre-existing Miri-clean resource-slab code exercised). Asserts no UB in `Res<State<S>>` get_param under the transition writes.

**Clippy / private-bound risk (L1).** The `pub fn on_enter/on_exit/on_transition` return an opaque `impl FnMut(Res<StateTransitionRecord<S>>) -> bool`. While the record's *type name* is hidden by `impl Trait`, the `Res<StateTransitionRecord<S>>` param type appears in the public `FnMut` bound, which `private_interfaces`/`private_bounds` may flag for a `pub(crate)` `StateTransitionRecord<S>`. The tester MUST run `cargo clippy --all-targets -- -D warnings` and confirm clean. **Fallback if the lint fires:** make `StateTransitionRecord<S>` and `Transition<S>` `pub` (they are opaque PODs with no public mutators — harmless, and Bevy keeps its `State`/transition types public). This is the only structural change L1 may force; it does not affect any decision (D4 already routes all reads through `Res<…>`).

**`debug_assert!` invariants to add**
- In `apply_state_transition::<S>`: after a real transition, `debug_assert!(world.resource::<State<S>>().get() == &entered)`.
- After draining: `debug_assert!(matches!(world.resource::<NextState<S>>(), NextState::Unchanged))`.
- In `run_state_transitions`: `debug_assert!(this_run == world.current_tick().get())`.
- `debug_assert!(!entry.pending_initial)` after the first run for each entry.

---

## 10. File-by-file change list

Legend: **[NEW]** create, **[MOD]** modify.

### Independent / parallelizable (new files)
1. **[NEW] `…/core/state/states.rs`** — the `States` trait (D1). *(Independent.)*
2. **[NEW] `…/core/state/state.rs`** — `State<S>` + `get`/`new`/`Deref` + `Resource` impl via D3. *(Needs 6.)*
3. **[NEW] `…/core/state/next_state.rs`** — `NextState<S>` + `set`/`pending`/`Default` + `Resource` impl. *(Needs 6.)*
4. **[NEW] `…/core/state/transition_record.rs`** — `StateTransitionRecord<S>` + `Transition<S>` + `current()` + `Resource` impl + `apply_state_transition::<S>` (§5.1) + `StateEntry`. *(Needs 2,3,6.)*
5. **[NEW] `…/core/state/state_set.rs`** — `StateTransitionSet` unit `SystemSet` (D10). *(Independent.)*
6. **[NEW] `…/core/state/state_resource_registry.rs`** — D3 `OnceLock<Mutex<HashMap<TypeId, ResourceId>>>` + `resource_id_for<T>()`. *(Do FIRST; 2,3,4 depend on it.)*
7. **[NEW] `…/core/state/mod.rs`** — module wiring + `pub use` (`States`, `State`, `NextState`, `StateTransitionSet`; `pub(crate)` record/transition/entry/apply/registry). *(Needs 1–6.)*

### Sequential (shared existing files)
8. **[MOD] `…/core/mod.rs`** — add `pub mod state;`. *(After 7.)*
9. **[MOD] `…/core/schedule/common_conditions.rs`** — add `in_state`/`on_enter`/`on_exit`/`on_transition` (D5/D11), same doc+test shape as `run_once`. *(After 2–4.)*
10. **[MOD] `…/core/schedule/mod.rs`** — `pub use common_conditions::{in_state, on_enter, on_exit, on_transition};`. *(After 9.)*
11. **[MOD] `…/core/ecs_master/ecs_master.rs`** — add `insert_state`/`init_state`/`state`/`set_next_state` to the resources facade (`#[cold]` insert, `#[inline]` reads). *(After 2–4.)*
12. **[MOD] `…/core/schedule/schedule_builder.rs`** — add `Vec<StateRegistration>` field + `insert_state`/`init_state` builder methods (record-then-realise) + drain in `try_build` (call `world.insert_state::<S>` and populate `Schedule::state_entries` with `StateEntry { apply: apply_state_transition::<S>, pending_initial: true, type_name }`). *(After 4,11.)*
13. **[MOD] `…/core/schedule/schedule.rs`** — add `state_entries: Vec<StateEntry>` field; add `run_state_transitions` (`#[cold]`); insert the gated call at the §6.1 point; flip `pending_initial`. *(After 4,12 — the executor touch; do last + verify 0%-bench.)*
14. **[NEW] `crates/boyko_ecs/tests/phase17_states.rs`** — §9 integration tests. *(After 8–13; tester runs it.)*
15. **[NEW] `crates/boyko_ecs/benches/phase17_states.rs`** — §7 microbenches + re-confirm the 50-systems regression bench. *(After 13.)*

**Wave plan for parallel developers**
- **Wave 1 (parallel):** 1, 5, 6.
- **Wave 2 (parallel):** 2, 3.
- **Wave 3:** 4.
- **Wave 4 (parallel):** 7, 9, 11.
- **Wave 5:** 8, 10, 12.
- **Wave 6:** 13 (executor touch — solo, then bench).
- **Wave 7:** 14, 15 (tester).

---

## 11. Explicitly deferred + the hooks that keep them addable

| Deferred item | Why deferred | Non-breaking hook |
|---|---|---|
| **Value-keyed sub-schedules (shape (a))** | Big new subsystem; nested-pool re-entrancy; per-schedule-config footgun. Orchestrator directed (b). | `apply_state_transition::<S>` is value-aware (`exited`/`entered`). A future keyed-schedule layer calls `world.try_run_schedule(OnEnter(entered))` from the pass IN ADDITION to writing the record; record + conditions stay valid ⇒ (a) and (b) coexist. Public API unchanged. |
| **Computed / sub-states** | Need a dependency graph among `State<S>` types + re-derive. | The pass loops a **list** of `StateEntry` and is generic + value-aware. A computed state is a `StateEntry` whose `apply` derives its value from another `State<P>`. The `Hash` bound (D1) is kept for the computed/sub-state map keys. |
| **State-scoped entity auto-despawn** | Needs a `StateScoped<S>` component + a despawn pass. | `on_exit(s)` already fires exactly once on the exit frame; a future `despawn_state_scoped::<S>` system is `.run_if(on_exit(prev))` — a library system on top, no core change. |
| **`StateTransitionEvent<S>`** | Events-as-resource is Phase 12; per-transition `Event` is extra surface. | The record already carries `{exited, entered}`. A future event is the same data sent via `EventWriter` from a built-in system gated on "record is Some". |
| **`OnTransition` as a keyed schedule** | We ship it as a **condition** (D11). | If (a) lands, the condition + the label coexist (same record). |
| **`Option<Res<R>>` SystemParam** | Its own SystemParam design; we chose require-exists (D8). | `in_state` keeps its public signature; swapping to `Option<Res<…>>` internally later is non-breaking. Filed with Phase-16's deferred `resource_exists`. |
| **`#[derive(States)]`** | Adds no codegen over the bound check (D1). | A future derive emits `impl States for #name {}` — additive; hand-impls keep compiling. |
| **Auto-`StateTransitionSet` ordering** | Would perturb every state-using schedule's graph (D10). | The marker ships now (opt-in). A future `builder.auto_order_state_systems()` wires it without changing existing code. |

---

## 12. The one micro-decision left to the developer (with default)

`StateEntry::apply` is **a single `fn(&mut EcsMaster, u32, bool)`** where the `bool` is `fire_initial` (one monomorphised `apply_state_transition::<S>` handling both initial + normal, §5.1/§6.1) — fewer pointers per entry, one codegen site per `S`, the `bool` a trivially predicted branch on a cold once/frame path. Internal-only (not public API); affects no other section.

**Borrow rule for the single-fn body (mandatory, not left to chance).** Every resource access inside `apply_state_transition::<S>` is a **fresh `world.resource{_mut}::<…>()` call scoped to its own statement**; the returned borrow ends at the statement boundary. The sequential calls in §5.1 (clear record → read `State` → write record → write `State`) therefore compile, because no two conflicting borrows are ever live at once. The developer **MUST NOT** bind a long-lived handle across a conflicting access — e.g. `let rec = world.resource_mut::<StateTransitionRecord<S>>(); … world.resource::<State<S>>();` will **not** compile (two simultaneous borrows of `world`). Re-fetch instead: read the value you need into a local (`let current = world.resource::<State<S>>().get().clone();`) and *then* take the `&mut Record`. Under this rule the single three-arg fn compiles with **no** `unsafe` and no second stored pointer; do not split it.

---

## 13. Open questions (for the critic)

1. **`ScheduleBuilder::insert_state` requires `&mut world` only at `build`** — the builder defers resource insertion to `try_build(world)` (consistent with Phase-16 set-conditions). A state registered on the builder is not present in the world until `build`. If a user calls `EcsMaster::resource::<State<S>>()` between builder registration and `build`, it panics. Documented; acceptable (world+builder are configured together before the frame loop). Alternative (eager `insert_state(&mut self, world, initial)`) rejected as ergonomically worse and inconsistent with the rest of the builder API.

2. **Pitfall (i)** (systems gated by `in_state` miss events buffered while inactive) is **documented, not fixed** (per the brief). The `in_state` doc comment notes: an `EventReader` in an `in_state`-gated system advances its cursor only on frames the system runs, so events sent while the state was inactive are skipped (standard Bevy behaviour). No code addresses it this phase.

3. **Resource-slab budget for states is bounded and deliberate (resolved, not open).** Each `State<S>` registration consumes **3** of the **256** shared resource slots (`State<S>`/`NextState<S>`/`StateTransitionRecord<S>`); the slab is shared with all other resources and minted from one `NEXT_RESOURCE_ID`. Exhaustion is a loud setup-time `assert!` panic (`resource_registry.rs:176-180`), not UB. Accepted as-is (§4.1): single-digit state-axis counts leave ample headroom, and keeping the record on the slab preserves the zero-extra-mechanism D4/D5 read path. Moving the record off-slab (to halve the per-state cost to 2) was considered and rejected — it would force a bespoke side-table lookup for `on_enter`/`on_exit`/`on_transition`, changing D4/D5 for a saving that does not matter at realistic state counts.

4. **`pub(crate)` record in a public `impl FnMut` bound may trip `private_bounds` (L1).** `on_enter`/`on_exit`/`on_transition` expose `Res<StateTransitionRecord<S>>` in their public closure bound while the record stays `pub(crate)`. If `cargo clippy -- -D warnings` flags it (verified by the tester, §9), the resolution is to make `StateTransitionRecord<S>`/`Transition<S>` `pub` — an opaque POD with no public mutators, matching Bevy's public state types. No design impact.
