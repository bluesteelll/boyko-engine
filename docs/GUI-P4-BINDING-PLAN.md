All claims are now verified against the actual code. Critical findings confirmed:

1. **`get_component`** returns `Option<&T>` (no tick); `get_component_mut` returns `Mut<T>` (bumps changed tick on DerefMut). No `get_component_ref` / `get_component_with_tick`. `read_changed_tick` is `pub(crate)`, row-index-keyed. → Decision 5 needs a new boyko_ecs public read-with-tick accessor, OR source-side `Changed<C>`.
2. **`get_component_raw(entity, component_id) -> Option<*const u8>`** exists, is public, None-checks generation + null. This is the exact None-returning raw fetch the binding accessor needs.
3. **Registry**: parallel `[OnceLock<X>; MAX_COMPONENTS]` tables (SERIALIZE, CLONE, MAP_ENTITIES, HOOKS...). A `BIND_ACCESSORS` table is a boyko_ecs edit, but it mirrors the established `get_serialize_info` precedent exactly.
4. **`UiViewport.scale_factor`** exists (logical = physical / scale). DPI conversion has a home.
5. **`StackIndex(pub u32)`** default 0 — tie-break gap real.
6. **`PhysicalInput`** has no focus/cursor-inside flag; `cursor_pos` persists across frames, defaults `[0,0]`.

I have everything needed. Writing the full revised plan.

---

# Architecture: GUI Phase P4 — Action + Data Binding

## Goal
Wire boyko_ui widgets to gameplay through change-gated systems, with **zero callbacks**, **zero per-frame heap allocation**, and **zero reflection**:
- **Pointer interaction** — hit-test `ComputedRect` against the cursor (DPI-converted), Z-ordered with a *total deterministic order*, write `Interaction`/`RelativeCursorPosition`; reset on blur/cursor-leave.
- **Action dispatch** — lower an `Interaction` click-edge into an `ActionState` edge (UI as a symmetric action *source*, opposite direction from Win32/egui input adapters), composing correctly with the verified sticky-edge fixed-snapshot protocol.
- **Data binding** — push a change-gated ECS source field into a widget's inline text/value buffer via a **codegen accessor table** (the "codegen not reflection" decision shared with serialization and the P3 `.ui` dispatch).

**Target metrics:**
- Hit-test: O(N_visible) point-in-rect tests, ~6–10 cycles/node (two `f32` range compares ×2), no alloc. N = laid-out nodes in the hovered root.
- `ui_dispatch_system`: O(rows with `Changed<Interaction>`) for hover/submit edges; O(1) for the click edge (resolved index stamped at focus time). Empty iterator on a still frame → ~0 cost.
- `ui_data_bind_system`: O(rows with a changed bound source) when *any* bound source changed; ~0 when none changed (outer 0%-gate). One indirect `fn`-pointer call per *changed* bound widget on the generic `.ui` path; zero accessor indirection on the monomorphized `ui!` path.
- Allocations/frame in steady state: **0** (asserted by a test with a counting allocator).

## Context and constraints
- **Affected subsystems**: `boyko_ui` (new `interaction/`, `binding/`); `boyko_macros` (new `#[derive(Bindable)]`); `boyko_input` (**two** cross-crate edits — see Decisions 9 & 10); `boyko_ecs` (**one small, justified public read-with-tick accessor** — see Decision 5; the "NONE" claim of the prior draft is retracted).
- **Invariants preserved**: P1 `ComputedRect`/`StackIndex`/`ComputedClip` stay author/layout-owned (interaction only *reads* them). Phase-10 change detection drives the edge-systems' 0%-gate. EnableTag stays migration-free and tick-free. `Arena` stays `!Send`/`!Sync` (all three systems single-threaded — see Multithreading).
- **Locked**: three named systems; no `Box<dyn Fn>`; codegen accessor table; high-churn flags → EnableTag; egui adapter → dev-tools-only; English-only; every `unsafe` carries `// SAFETY:`.
- **Retracted from the prior draft**: "boyko_ecs: NONE" (Decision 5 needs one accessor); "the registry already stores serialize codegen hooks in a fn-pointer table" (the real shape is parallel `[OnceLock<X>; MAX_COMPONENTS]` tables — `SERIALIZE`, `CLONE`, etc. — and `BIND_ACCESSORS` mirrors that, which is a boyko_ecs edit, not a free slot); "boyko_input: one change only".

---

## Key decisions

### Decision 1: `Interaction` as a tick-bearing enum component; `Hovered`/`Pressed`/`Focused` ALSO as EnableTag bits — dual representation
**What**: `Interaction{None|Hovered|Pressed}` is a `#[repr(u8)]` enum component (one SoA column, set-if-changed). Additionally the focus system maintains EnableTag bits `UiHovered`, `UiPressed`, `UiFocused` for O(1) per-row query filtering by downstream systems (P5 render, P6 styling).
**Why**: EnableTag bitsets have no per-row tick storage, so `Changed<UiHovered>` is **compile-rejected** (verified: `filter.rs` const-assert `!C::STORAGE_IS_BITSET`). But the locked dispatch/bind systems are `Changed`-gated, which requires a tick-bearing column. Resolution: `Interaction` is the **tick-bearing edge source** (drives `Changed<Interaction>`); the EnableTag bits are the **O(1) filter surface** for consumers that only need "is this hovered right now?" without archetype migration. Both are written together in `ui_focus_system` from the same computed state, so they never diverge.
**EnableTag toggle is transition-gated (closes minor finding)**: `enable`/`disable` (verified `&mut self`, `enable_tag_api.rs`) are called **only on the genuine `Interaction` state-transition branch** — the *same* guard as the set-if-changed enum write. On a still frame the transition branch is not entered, so **zero** `enable`/`disable` calls are issued. A pure cursor-move *within* the same node (Interaction stays `Hovered`) toggles **no** bit. The still-frame test asserts zero enable-bit writes over 10k still nodes.
**Alternatives**:
- *EnableTag-only*: rejected — kills the `Changed<Interaction>` gate; dispatch would diff a last-frame shadow every frame (O(all interactive nodes)).
- *Interaction-column-only*: rejected — a "all hovered buttons" filter would pay an archetype migration on every hover/unhover (migration storm on high churn).
**Trade-off**: One extra `u8` column + 3 bitset bits per interactive node; two writes per state *transition*. Both O(1), both gated to genuine transitions. Memory: 1 B/node + ~0 (paged global bitset).

### Decision 2: Click = press-inside → release-inside-same-node (release-up), origin in `UiPointerState`; the resolved action index is **stamped at press time**
**What**: A click emits when the left button releases while the cursor is over the **same** node it was pressed over. The press-origin entity is held in `UiPointerState` (single resource, Decision 11 for shape). **At press time** (`clicked` over `hovered`) the focus system reads the origin node's `OnClick.0` (if present) and stamps it into `UiPointerState.pending_click: Option<(Entity, u16)>`. At release-inside-same-node, the dispatch system fires that stamped index. (Resolves the critic's "two click-resolution paths" incoherence — there is now **one** path: stamp-at-press.)
**Why**: Release-up is conventional click (drag-off cancels). Stamping at press makes the click O(1) and **independent of `Changed<Interaction>`** on the release frame (the release transitions to `None`, so `Changed<Interaction>` would *not* surface the press-origin reliably — the critic's exact correctness point). Stamp-at-press also fixes the click semantics under mid-press `OnClick` mutation: the action is the one that was bound when the user committed to the press.
**Despawn/reparent across the press window (closes major finding)**: `pending_click` stores `(Entity, u16)`. At release, dispatch re-validates the origin entity is **still alive with the matching generation** before firing: it calls `world.get_component_raw(origin, OnClick::component_id())` (verified: returns `None` on null slot OR generation mismatch, `ecs_master.rs:1808–1816`). If `None` → the click is silently dropped (origin despawned, recycled to a different generation, or lost its `OnClick`). Reparenting does **not** change the `Entity` id/generation, so a reparented-but-alive origin still fires correctly. The stamped `u16` is the press-time action, so a mid-press `OnClick` *value* change is intentionally ignored (documented contract).
**Alternatives**: press-down-fires (rejected — no drag-cancel); per-node `PressedHere` column (rejected — per-frame clear / migration churn).
**Trade-off**: A drag A→release-B fires nothing (correct UX). A press-down widget reads `Interaction == Pressed` directly. `OnClick` = release-up, action-stamped-at-press is the v1 contract.

### Decision 3: `OnClick(u16)` carries the dense `Actionlike::index()`, resolved at authoring/parse time — NOT generic `OnClick<A>`
**What**: `OnClick(pub u16)` stores the raw action index. Both `.ui` and `ui!` lower a named action to its index. The dispatch system is non-generic over `A` *in the click/hover/submit lowering* (it writes by `usize` index via `ui_press`).
**Why**: A generic `OnClick<A>` forces per-`A` monomorphization of dispatch and is un-authorable from `.ui` text (it carries only a name string). The dense `u16` index is the reflection-free common denominator: an integer, const-foldable in `ui!`, matching boyko's `Actionlike` closed-enum-with-dense-index design, and exactly what `ActionState::ui_press(index: usize)` takes (Decision 9).
**Alternatives**: `OnClick<A>` (rejected — multiplies dispatch per enum, un-authorable from `.ui`).
**Trade-off**: A `.ui` action-name typo is caught at parse time against a name→index table, not at Rust compile time. `ui!` validates against the enum if it is in scope. Same trust model as every other `.ui` value.

### Decision 4: Bind topology via a `BoundTo` source-pointer component on the widget — NOT a `BindIndex` resource. Change-gate is **source-side** (see Decision 5)
**What**: A bound widget carries `BindText{ source: Entity, comp: ComponentId, field: u8, field2: u8, template: TemplateId }` (or `BindValue{...}`). The `source` field *is* the link; no reverse `source→widgets` map.
**Why**: ECS-native, Principle-0 (no parallel reverse map). The bind system iterates **widget-side** (`Query<(&BindText, &mut UiTextBuffer)>`) and reads the source row. The change-gate that this requires is resolved in Decision 5 with a real, verified primitive — *not* a non-existent method.
**Alternatives**: `BindIndex` resource (`source → SmallVec<widget>`): rejected — a parallel side store needing spawn/despawn maintenance.
**Trade-off**: Widget-side iteration cannot apply the archetype-level `Changed<Source>` *query filter* directly (source ≠ iterated widget). Decision 5 closes this with a per-widget read-with-tick compare; Decision 6 gates the whole system so the loop only runs when *some* bound source changed.

### Decision 5: Bind change-gate = per-widget read-with-tick via a **new public boyko_ecs accessor** `get_component_changed_tick(entity, ComponentId) -> Option<Tick>`, compared to the bind system's `last_run` with the public `Tick::is_newer_than`
**What**: For each bound widget, fetch the source's stored `changed_tick` by `(Entity, ComponentId)` and compare `tick.is_newer_than(last_run, this_run)` (verified public, `tick.rs`). Reformat the sink only if newer. This is the same predicate Phase-10's `Changed<T>` filter uses internally.

This requires a **new, small, public, read-only boyko_ecs accessor** — the prior draft's `get_component_ref` / `get_component_mut`-tick claims were verified false:
- `get_component` returns `Option<&T>` with **no** tick (`ecs_master.rs:1963–1973`).
- `get_component_mut` returns `Mut<T>` whose ticks are pinned to `current_tick()` and whose `DerefMut` **bumps** the source's changed_tick (`ecs_master.rs:2008+`) — reading the source through it would (a) report the wrong predicate and (b) mark the **source** dirty, corrupting the very `Changed<Source>` signal Decision 6 reads. Forbidden.
- The only row-tick reader, `ComponentPool::read_changed_tick`, is `pub(crate)` and **row-index-keyed**, not entity-keyed (`component_pool.rs:1632`). Invisible to boyko_ui.

**The new accessor (the honest boyko_ecs edit):**
```rust
// boyko_ecs, on EcsMaster (read-only, &self, NEVER bumps any tick):
/// Returns the stored `changed_tick` of `entity`'s `component_id` column row,
/// or `None` if the entity is dead/stale or does not host the component.
/// Read-only: does NOT mutate change-detection state (unlike `get_component_mut`).
pub fn get_component_changed_tick(&self, entity: Entity, component_id: ComponentId) -> Option<Tick>;
```
It reuses the exact `get_component_raw` prologue (null + generation check, returns `None` on miss — `ecs_master.rs:1808`) and reads the row's `changed_tick` via the existing `pub(crate) read_changed_tick(row_index)` (now reachable because it is a same-crate call). One `// SAFETY:` note: the row index comes from the validated `EntityInland.unit_index()`, in-bounds for the resolved pool. Cost: one entity-keyed resolve (~3 ns, Phase-7) + one tick load + one compare per bound widget per dirty frame.
**Why this over source-side `Changed<C>` + reverse index**: the source-side filter (the critic's alternative) *does* read the per-row tick correctly and is the proven idiom — but it forces a reverse `source→widgets` fan-out (Decision 4 rejected as a parallel data system) OR per-`C` monomorphized bind systems that cannot serve `.ui`-dynamic bindings (open-ended source types). The widget-side read-with-tick keeps one bind system, serves both `ui!` and `.ui`, and needs only one tiny read-only accessor — strictly less surface than a reverse-index maintained on every bound-widget spawn/despawn.
**Alternatives**: reverse `BindIndex` + `Changed<Source>` (rejected, Decision 4); per-source `OnSet` observer (rejected — per-binding dispatch + registry, more expensive and not closure-free).
**Trade-off**: The widget-side loop is O(bound widgets) when dirty, not O(changed sources); dominated by the entity-keyed resolves. For tens-to-low-hundreds of bound widgets this is a few hundred ns worst case, and the outer gate (Decision 6) skips it entirely on still frames. The boyko_ecs surface grows by one read-only method (documented, no UB, no tick mutation).

### Decision 6: Outer 0%-gate — split discovery/apply; static `ui!` types via `Or<(Changed<…>)>`, dynamic `.ui` types via a **new id-keyed archetype-changed-since-tick probe**
**What**: `ui_data_bind_system` is split into **discovery** (cheap, sets a `dirty` flag on `UiBindScratch`) and **apply** (early-returns when `!dirty`), mirroring the verified `ui_layout_discovery`/`ui_layout_apply` pair (`layout.rs:81`/`137`).
- **Static `ui!`-bound types** use a const `Query<(), Or<(Changed<C1>, Changed<C2>, …)>>` probe (`iter().next().is_some()` — the exact 0%-gate idiom proven in `ui_layout_discovery`, `layout.rs:111`; `NEEDS_CHANGE_DETECTION` const-folds the dispatcher when unused).
- **Dynamic `.ui`-bound types** (source type not known at the bind site) use a **new public boyko_ecs probe** `any_changed_since(component_ids: &[ComponentId], last_run: Tick, this_run: Tick) -> bool`. This is the second honest boyko_ecs read-only edit. It scans **only the archetypes that host any of the registered bound `ComponentId`s** and tests each archetype's column change epoch — NOT every entity. The bound-component set is a small `[ComponentId]` registered at bind time (closed at runtime, grows only on a new `.ui` bind kind).
**Why a real primitive, not a hand-wave**: the prior draft said "checked via the archetype change-tick" without naming a primitive; the critic correctly flagged that no such id-keyed probe exists and a naive scan would be O(archetypes × ids) **every frame**, defeating the gate. The new `any_changed_since` is bounded to hosting archetypes and short-circuits on the first changed column. Both gate halves feed the same `dirty` flag.
**Complexity of the probe**: O(hosting-archetypes × bound-ids) worst case, but (a) it only iterates archetypes that actually host a bound id (typically 1–few), (b) it short-circuits on first hit, and (c) a still frame finds no changed column and returns `false` in O(hosting-archetypes). A bench asserts the `.ui`-dynamic still-frame path is ~0, in parallel with the `ui!`-static still-frame bench.
**Alternatives**: no outer gate (rejected — O(bound widgets) entity resolves every frame even when nothing changed); monomorphized-only `ui!` (rejected — drops the locked `.ui bind_text:` path).
**Trade-off**: Two boyko_ecs read-only probes (`get_component_changed_tick`, `any_changed_since`) instead of zero. Both are documented, read-only, and bounded. Honest accounting in Integration.

### Decision 7: Codegen accessor table — `#[derive(Bindable)]` emits `fmt_field`/`value_field` and installs a type-erased `BindAccessor` into a **new boyko_ecs parallel `[OnceLock<BindAccessor>; MAX_COMPONENTS]` table**, read only off the still-frame path
**What**: `#[derive(Bindable)]` emits (a) a `#[repr(u8)]` field enum, (b) `impl Bindable` with `fmt_field(&self, field, &mut dyn fmt::Write)` / `value_field(&self, field) -> f32` / `field_id(name) -> Option<u8>`, and (c) a registration of `BindAccessor { fmt: fn(*const u8, u8, &mut dyn fmt::Write) -> fmt::Result, value: fn(*const u8, u8) -> f32 }` into a new `BIND_ACCESSORS` table in `component_registry.rs`, keyed by `ComponentId`.
**Why the table home is corrected (closes critical finding)**: the prior draft claimed the accessor lands in "the existing ComponentRegistry entry, which already stores Layout and serialize hooks" with "boyko_ecs: NONE". Verified false: `ComponentLayout` is a fixed record; component metadata lives in **separate parallel `[OnceLock<X>; MAX_COMPONENTS]` tables** — `LAYOUTS`, `HOOKS`, `CLONE`, `MAP_ENTITIES`, `SERIALIZE` (`component_registry.rs:280/298/1531/1540/1918`), each with a public `get_*` reader and a `register_*`/`install_*` writer. Serialization is exactly this: `static SERIALIZE: [OnceLock<SerializeInfo>; MAX_COMPONENTS]` + `pub fn get_serialize_info(id) -> Option<&'static SerializeInfo>`, read "ONLY from the `boyko_serialize` crate — never on the hot path." `BIND_ACCESSORS` is a **carbon copy of that proven pattern**, read only from boyko_ui's bind-apply, never on a still frame. This is a small, idiomatic, in-crate boyko_ecs addition — and it is now declared as such (not "NONE"). The cited "serialize hooks" precedent is *real* (the SERIALIZE table), but it is a parallel table, not a `ComponentLayout` field — so it confirms a new parallel table, not a free slot.
**Why codegen not reflection**: field identity is a `u8` enum resolved once at parse/spawn — no runtime string compare, no `HashMap`, no `TypeId` downcast, no `Any`. The fn-pointer pair is the general path for `.ui` bindings (source type not statically known at the bind site); the `ui!` path monomorphizes and calls `fmt_field` directly.
**Residual indirection, named explicitly (closes minor finding)**: on the `.ui` path each changed field-format incurs **two** indirections — (1) the `BindAccessor` fn-pointer, and (2) the `&mut dyn fmt::Write` sink vtable inside `write!`. Both are off the still-frame path (change-gated). The `ui!` monomorphized path collapses (1) but still uses `dyn Write` for (2) unless the sink is made generic. **Decision: the `ui!` path uses a concrete `&mut UiTextBuffer` sink** (`UiTextBuffer: fmt::Write`), removing the vtable on the hot monomorphized path; the `.ui` type-erased trampoline keeps `&mut dyn fmt::Write` (unavoidable, and acceptable — change-gated). Float `Display` formatting is stack-buffered and alloc-free in both.
**Alternatives**: `bevy_reflect`-style reflection (rejected, banned); belly `Box<dyn Fn>` (rejected, Principle 1); a `ComponentLayout` field (rejected — `LAYOUTS` is fixed-shape; the parallel-table pattern is the established mechanism).
**Trade-off**: One `OnceLock<BindAccessor>` slot per `MAX_COMPONENTS` (512) in boyko_ecs static — same fixed cost as `SERIALIZE`/`CLONE`. One indirect call per *changed* bound widget on the `.ui` path.

### Decision 8: `bind_value` restricted to a closed `field` or `field / field` ratio, evaluated by `value_field` — NO expression interpreter
**What**: `BindValue{ source, comp, num_field, den_field }` where `den_field == NO_FIELD` (sentinel `0xFF`) means raw value. Result = `value_field(num)` or `value_field(num) / value_field(den)` with a div-by-zero → `0.0` guard.
**Why**: The ratio form covers health-bars, progress, sliders (the plan's `bind_value: Health.current / Health.max`). No runtime RPN/parser.
**Alternatives**: expression evaluator (rejected — runtime interpreter overkill).
**Trade-off**: `(a+b)/c` style unsupported in v1; compute it in a gameplay system and bind the single result field.

### Decision 9: Cross-crate action-write seam = `pub fn ui_press(index: usize)` / `ui_set_value(index, value)` on `ActionState<A>` (boyko_input edit #1)
**What**: Two public methods on `ActionState<A>` with explicit UI-source semantics. `ui_press` ORs the live rising edge + sets the level bit (a held UI button reads `pressed`); `ui_set_value` for slider/analog widgets. They wrap the existing `pub(crate)` recompute-from-scratch writers and are the only sanctioned UI→action path.
**Why**: A named, intention-revealing public method, not a blanket `pub` on the frame-recompute internals (which gameplay must not call — a gameplay call would be clobbered by the next `begin_frame`, `state.rs:114`). Symmetric to `RawInputQueue::push_raw` (the single device-side input seam) but on the *processed* side (UI is a post-processing action source, opposite direction from device adapters).
**Alternatives**: widen `set_just_pressed` to `pub` (rejected — exposes recompute internals; footgun); `UiActionWriter` newtype (rejected — extra type for no gain).
**Fixed-snapshot composition is split out into Decision 10** (the prior draft tangled it into Decision 9 and got the ordering self-contradictory). `ui_press` writes **only the live edge + level** — it does **not** touch `fixed_just_pressed` directly. The fixed-snapshot correctness is handled entirely by ordering (Decision 10), reusing the engine's existing, proven sticky-edge protocol.

### Decision 10: Schedule ordering for the UI action edge — run `ui_focus_system` + `ui_dispatch_system` **inside the input update window, before `freeze_fixed_snapshot`**, so the existing OR-accumulate + clear-on-consume protocol covers the UI edge with no second writer
**What**: Order the two interaction systems so the UI-injected live edge is present **when `freeze_fixed_snapshot` runs**, letting the *existing* engine mechanism carry it into the fixed loop. Concretely:
1. `clear_consumed_fixed_edges` runs at Main start, gated on `steps_this_frame > 0` (verified `state.rs` `clear_fixed_edges`).
2. The device path runs (`process_actions` → `begin_frame` clears live edges → re-aggregate from `PhysicalInput`).
3. **`ui_focus_system` then `ui_dispatch_system` run here**, calling `ui_press` to OR the UI rising edge onto the live `just_pressed` set (after device `begin_frame`, before the freeze).
4. `freeze_fixed_snapshot` runs (end of the input update window), OR-accumulating the live edges — now including the UI edge — into `fixed_just_pressed` (verified OR-accumulate, `state.rs:230–247`).

**Why this is correct with no double-count and no miss (closes the major finding with the proof the critic demanded)**: the UI edge is now an ordinary live `just_pressed` bit at freeze time, indistinguishable from a device edge. The engine's existing proof therefore applies verbatim:
- **No-miss across a 0-substep frame**: the freeze OR-accumulates the UI edge into `fixed_just_pressed`; `clear_fixed_edges` only clears it after a fixed batch consumes it (gated on `steps_this_frame > 0`, `state.rs`). A 0-substep frame leaves it sticky (the documented BUG-I4-C3 fix path).
- **No-double-count**: the live `just_pressed` UI bit is cleared by the *next* frame's `begin_frame` (`state.rs:114`), so it is not re-frozen; the frozen `fixed_just_pressed` bit is cleared exactly once after the first consuming batch. The UI edge is `fixed_just_pressed` for exactly one fixed batch — identical to a device edge.
- `ui_press` does **not** write `fixed_just_pressed` directly (the prior draft's second writer is **removed**), so there is no unsynchronized sticky-edge path and no possibility of double-set. One mechanism, one proof.

For the **Main schedule** (non-fixed consumers), the UI live edge is visible the same frame (it was OR'd before any Main consumer reads). 
**Mandatory test** (copied from the existing input no-miss/no-double-count test shape): inject a UI press on a 0-substep frame; assert it is observed by exactly one fixed batch and cleared once; assert a UI press on a multi-substep frame fires once.
**Alternatives**: run dispatch `.after(update_action_state)` + a second `fixed_just_pressed` writer in `ui_press` (the prior draft — rejected: a second unsynchronized sticky-edge writer, the exact footgun, and the source of the contradictory text). Run dispatch in a separate post-freeze system with its own re-freeze (rejected — duplicates the freeze mechanism).
**Trade-off**: `ui_focus_system`+`ui_dispatch_system` must be schedulable inside the input update window (before freeze). `ui_focus_system` is exclusive (`&mut EcsMaster`), so this window must permit an exclusive system between device aggregation and freeze; the input update is already an exclusive-capable region. The bind systems remain late in Main (they don't feed actions).

### Decision 11: Single-pointer v1, but `UiPointerState` shaped as a **fixed 1-slot array indexed by pointer id** so multi-pointer is a non-breaking later extension
**What**: v1 supports one pointer (mouse). `UiPointerState` holds `pointers: [PointerSlot; MAX_POINTERS]` with `MAX_POINTERS = 1` in v1, each slot `{ press_origin: Option<(Entity, u16)> /* pending_click */, press_gen: u32, click_fired: Option<(Entity, u16)>, reset_next_frame: Option<Entity> }`. The mouse uses slot 0.
**Why (closes major finding)**: a HUD/GUI engine is single-pointer-correct for v1, but the critic correctly flags that a *scalar* `UiPointerState` would have to be torn up for touch. Shaping it as a fixed array indexed by pointer id makes the later multi-touch extension additive (bump `MAX_POINTERS`, route touch ids to slots) with no API break and no scalar→array migration. The array is fixed-size POD in a `Resource` (Principle 0/5 — no per-frame alloc).
**Out-of-scope, stated explicitly**: multi-pointer/touch is a documented v1 scope boundary; `PhysicalInput` likewise carries a single `cursor_pos` + `u8` mouse mask, so touch also needs a `boyko_input` device-side extension later. v1 ships the slot-0 path.
**Alternatives**: scalar `UiPointerState` (rejected — breaking later); model full multi-touch now (rejected — out of v1 scope, no device source).
**Trade-off**: `MAX_POINTERS = 1` indexing is a trivial constant fold in v1; the array shape costs nothing now and saves a rewrite later.

---

## Components (exact Rust definitions)

All in `crates/boyko_ui/src/interaction/{components,action}.rs` and `binding/components.rs`. All POD `Copy`. `#[derive(Component, …)]` per the existing pattern (`components.rs`).

```rust
// ───────── interaction/components.rs ─────────

/// Pointer interaction state, recomputed each frame by `ui_focus_system`.
/// Tick-bearing column (drives `Changed<Interaction>`). Written set-if-changed
/// so a still frame bumps no tick (Decision 1).
#[repr(u8)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interaction {
    #[default]
    None,     // cursor not over this node (or occluded / invisible)
    Hovered,  // cursor over, button not pressed on it
    Pressed,  // button pressed on it (held)
}

/// Cursor position relative to the node, normalized to (-0.5..0.5)^2, center 0.
/// OPT-IN (sliders, drag handles). Written set-if-changed only on the hovered
/// node; on a leave (`cursor_over=false`) `normalized` is reset to the canonical
/// `[0.0, 0.0]` BEFORE the equality compare, so a leave does not leave residual
/// bytes that defeat the set-if-changed gate (closes the minor finding). 12 B.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct RelativeCursorPosition {
    pub cursor_over: bool,
    pub normalized: [f32; 2], // canonical [0,0] when !cursor_over
}

/// Propagation policy. `Block` stops HOVER RESOLUTION (no node below becomes the
/// hovered node); it does NOT skip the unconditional reset pass (Decision below).
/// OPT-IN, default `Pass`. `#[repr(u8)]`.
#[repr(u8)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FocusPolicy { Block, #[default] Pass }

/// Keyboard-focusable + linear tab order. OPT-IN. Cross-root order is total:
/// (root enumeration order, then `tab_index`, then `Entity`) — see focus step 7.
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Focusable { pub tab_index: u32 }
// `Focused` is NOT a component: the single `UiInputFocus` resource holds the one
// focused Entity, plus the `UiFocused` EnableTag bit for O(1) filtering.

// ───────── EnableTag bits (registered once at plugin setup) ─────────
// `UiHovered`, `UiPressed`, `UiFocused` — high-churn, O(1) toggle, no migration,
// NOT tick-bearing (the tick-bearing `Interaction` column is the Changed-gate).
// Registered at setup; ids cached in `UiInteractionConfig`.

// ───────── interaction/action.rs ─────────

/// Emit action `index` (dense `Actionlike::index()`) on a release-up click over
/// this node (Decisions 2/3). Reflection-free (an integer). `#[repr(transparent)]`.
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnClick(pub u16);

/// Emit action `index` on hover-enter (None→Hovered). OPT-IN.
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnHover(pub u16);

/// Emit action `index` on a submit edge (Enter while focused). OPT-IN.
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnSubmit(pub u16);

// ───────── binding/components.rs ─────────

/// Binds a formatted text view of a source field into this widget's
/// `UiTextBuffer`. `comp`/`field` resolved at parse/spawn (Decision 7). 16 B.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BindText {
    pub source: Entity,
    pub comp: ComponentId,
    pub field: u8,
    pub field2: u8,        // NO_FIELD (0xFF) = unused
    pub template: TemplateId,
}

/// Binds a normalized `f32` into this widget's `UiValue`. Decision 8. 16 B.
#[repr(C)]
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct BindValue {
    pub source: Entity,
    pub comp: ComponentId,
    pub num_field: u8,
    pub den_field: u8,     // NO_FIELD (0xFF) = raw value
}

/// Inline render-facing text buffer — the bind SINK. Mirrors the verified
/// `UiName` inline-buffer + `core::fmt::Write` pattern (alloc-free, POD Copy).
/// Tick-bearing so the P5 text-upload system is `Changed<UiTextBuffer>`-gated.
/// 256 B, `align(64)`.
#[repr(C, align(64))]
#[derive(Component, Clone, Copy)]
pub struct UiTextBuffer { bytes: [u8; Self::CAP /*247*/], len: u8, _pad: [u8; 8] }
impl core::fmt::Write for UiTextBuffer { /* push bytes, saturating at CAP */ }

/// Normalized bound scalar SINK (health bars, progress, sliders). 4 B.
#[repr(transparent)]
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct UiValue(pub f32);
```

**Authoring (P2 `ui!` / P3 `.ui`)**:
- `ui!`: `OnClick`/`OnHover`/`OnSubmit`/`BindText`/`BindValue` literals flow through the existing generic `.insert()` chain with **no macro change** (ordinary component literals). `ui!` resolves an action name to `index()` at expansion (enum in scope) and `Health.current` to `(ComponentId, field_u8)` via the `Bindable` impl's associated consts.
- `.ui`: add five arms to `parse_and_insert`'s `match name` (`dispatch.rs`): `OnClick`/`OnHover`/`OnSubmit`/`BindText`/`BindValue`. `OnClick(Jump)` parses the action name against a name→index table supplied by the registered action enum (recoverable per-line error on unknown name). `BindText { source: "#player", comp: "Health", field: "current", field2: "max", template: "{0}/{1}" }` resolves `#player` via the existing `UiName`→`Entity` index (`reconcile.rs` `build_global_named_index`), `Health.current` via the accessor table's `field_id` (one cold per-bind string→u8 resolve at parse time, never per frame).

---

## Public API

```rust
// boyko_ui plugins (only public surface; systems are internal fns)
pub struct UiInteractionPlugin<A: Actionlike>; // registers 3 EnableTags + resources + systems
pub struct UiBindingPlugin;

// codegen trait the derive implements (boyko_ui::binding)
pub trait Bindable: Component {
    const FIELD_COUNT: u8;
    fn fmt_field(&self, field: u8, out: &mut dyn core::fmt::Write) -> core::fmt::Result;
    fn value_field(&self, field: u8) -> f32;
    fn field_id(name: &str) -> Option<u8>;     // cold, parse-time
    fn register_bind_accessor();               // installs the BindAccessor once
}

// boyko_input — edit #1 (Decision 9)
impl<A: Actionlike> ActionState<A> {
    /// UI-source rising edge for action `index`: ORs the live edge + sets the
    /// level. Does NOT touch the fixed snapshot — ordering (Decision 10) lets the
    /// existing freeze carry it. The sanctioned UI→action path.
    pub fn ui_press(&mut self, index: usize);
    /// UI-source analog value for a Button/Axis1D action (sliders).
    pub fn ui_set_value(&mut self, index: usize, value: f32);
}
// boyko_input — edit #2 (Decision 12): a window-focus/cursor-inside signal on PhysicalInput.

// boyko_ecs — edit (Decision 5/6): two read-only accessors.
impl EcsMaster {
    pub fn get_component_changed_tick(&self, entity: Entity, component_id: ComponentId) -> Option<Tick>;
    pub fn any_changed_since(&self, ids: &[ComponentId], last_run: Tick, this_run: Tick) -> bool;
}
// boyko_ecs — registry: BIND_ACCESSORS table + reader/installer (mirrors SERIALIZE).
pub struct BindAccessor {
    pub fmt:   fn(*const u8, u8, &mut dyn core::fmt::Write) -> core::fmt::Result,
    pub value: fn(*const u8, u8) -> f32,
}
pub fn get_bind_accessor(component_id: usize) -> Option<&'static BindAccessor>;
pub fn install_bind_accessor(component_id: usize, acc: BindAccessor);

// proc-macro (boyko_macros)
#[proc_macro_derive(Bindable, attributes(bind))]
```

### Decision 12: Blur/cursor-leave is a real boyko_input seam (`PhysicalInput.cursor_inside` / window-focus), not a phantom flag (boyko_input edit #2)
**What**: Add an explicit boolean signal to `PhysicalInput` (e.g. `cursor_inside: bool`, set from the OS `CursorEntered`/`CursorLeft` events, and/or a `window_focused: bool` from `WindowFocus`). On `!cursor_inside || !window_focused`, `ui_focus_system` forces every interactive node's `Interaction` to `None` (set-if-changed), clears `UiHovered`/`UiPressed`, cancels every `UiPointerState` slot's `pending_click`, and clears `UiInputFocus` + `UiFocused`.
**Why (closes critical finding)**: verified — `PhysicalInput` (`queue.rs:155–200`) has **no** focus/cursor-inside flag, and `cursor_pos` is a **persisted level** (`begin_frame` keeps it, `queue.rs:211–218`) defaulting to `[0.0, 0.0]` — a real hittable top-left point. There is therefore **no** existing value that means "cursor left the window"; the prior draft's "PhysicalInput flag — or cursor_pos sentinel" was a phantom. A real signal is required, fed from the OS event the input adapter already receives. This makes the cited Bevy #17944 stuck-hover-on-blur actually defended, and lets a held-button drag-off-window-then-release-elsewhere be observed as a cancel.
**Alternatives**: a NaN/sentinel `cursor_pos` written on `CursorLeft` (rejected — overloads a position field with a state meaning; an explicit bool is clearer and cannot collide with `[0,0]`); defer blur handling entirely (rejected — leaves the documented stuck-hover bug live, and the plan claimed to handle it).
**Trade-off**: A second `boyko_input` edit (the prior draft's "one change only" is retracted; there are two: `ui_press`/`ui_set_value` and the focus signal). Both are small and justified. The input adapter (Win32/winit) must route `CursorEntered`/`CursorLeft`/`WindowFocus` into the new field — a one-line-per-event addition where raw events are already applied (`apply`, `queue.rs:225`).

### Decision 13: Cursor coordinate space — convert physical cursor to logical via `UiViewport.scale_factor` before any point-in-rect test; narrow f64→f32 once, after scaling
**What**: `cursor_pos` is physical pixels (`f64`, raw from the OS `CursorMoved`, `queue.rs:263`). `ComputedRect` is **logical px** (`components.rs:168–175`). The existing `UiViewport.scale_factor` (verified, `resources.rs:33`, doc: "logical = physical / scale") is the conversion. `ui_focus_system` computes `cursor_logical = [(cursor_pos[0] / scale) as f32, (cursor_pos[1] / scale) as f32]` **once**, narrowing f64→f32 a single time after the divide, then runs all hit-tests in logical space against `ComputedRect`.
**Why (closes critical finding)**: verified — no scaling exists anywhere on the cursor path, and `UiViewport` already carries `scale_factor` precisely for this (P5a applies it at upload; P4 applies it at hit-test so cursor and rects share a space). On any HiDPI display (1.5×/2.0×) an unscaled compare is wrong by the scale factor — the prior draft asserted "logical px, DPI" without specifying where the conversion happens. The single narrowing point gives a documented precision contract at sub-pixel rect edges (narrow once, after scaling; never compare f64 against f32-derived rects).
**Mandatory test**: a HiDPI hit-test with `scale_factor != 1.0` (e.g. 2.0) — the existing "corners → ±0.5" test passes at 1.0× and hides this; the new test sets `scale_factor=2.0`, a physical cursor at `(200,200)`, a logical rect at `(0,0,150,150)` → hit (logical cursor `100,100`), proving the conversion.
**Trade-off**: One divide + one narrow per frame (cursor resolved once, not per node) — negligible. If the host ever delivers logical cursor coords, set `scale_factor=1.0` (the conversion becomes identity); the contract is "cursor_pos is physical, viewport carries the scale."

---

## Algorithms for critical paths

### `ui_focus_system` — hit-test + interaction (exclusive)
**Signature**: exclusive system `&mut EcsMaster` (mirrors `ui_layout_apply`, `layout.rs:137`): hit-test reads a node's ancestors' `ComputedClip`/`FocusPolicy` while writing *other* nodes' `Interaction` — not expressible as a conflict-free parallel `Query`.
```rust
pub fn ui_focus_system(world: &mut EcsMaster);
// reads: Res<PhysicalInput> (cursor_pos, cursor_inside, window_focused, mouse edges),
//        Res<UiViewport> (scale_factor), ComputedRect, ComputedClip, StackIndex,
//        FocusPolicy, Children/ChildOf, (InheritedVisibility — P5; until then all visible)
// writes: Interaction (set-if-changed), RelativeCursorPosition (set-if-changed),
//         EnableTag UiHovered/UiPressed/UiFocused (transition-gated), UiPointerState,
//         UiInputFocus
```
**Steps:**
1. **Blur/leave short-circuit (Decision 12)**: if `!PhysicalInput.cursor_inside || !window_focused` → run the **unconditional reset pass** (step 3b) setting every interactive node to `None`, clear all EnableTag interaction bits, cancel all `UiPointerState` slots' `pending_click`, clear focus; return.
2. **Resolve cursor (Decision 13)**: `cursor = [(cursor_pos[0]/scale) as f32, (cursor_pos[1]/scale) as f32]` (one narrow). Derive edges: `clicked = mouse_just_pressed & LEFT_BIT`, `released = mouse_just_released & LEFT_BIT`, `held = mouse_pressed & LEFT_BIT` (`queue.rs:172–176`).
3a. **Hover resolution (total Z-order, Decision below)**: iterate roots (`query_entities(&[UiRoot::component_id()])`, cached in `UiInteractionScratch::roots`, refreshed on root-set change — `LayoutScratch` pattern). DFS each root via `Children::as_slice()` collecting `(entity, StackIndex, ComputedRect, ComputedClip, FocusPolicy)` into a preallocated scratch `Vec` (capacity retained — setup-time alloc, Principle 5), **building in paint order** (DFS document order = paint order). Determine the hovered node by the **total order** (see "Z-order determinism" below): the top-most node whose `point_in_rect(cursor, rect) && point_in_clip(cursor, clip)`. If that node is `FocusPolicy::Block`, hover resolution stops there (no node below it can be the hovered node).
3b. **Unconditional reset + write pass**: iterate **all** interactive nodes (the scratch). For the resolved hovered node: `Pressed` if `clicked || (held && prev==Pressed)`, else `Hovered`; if `released && prev==Pressed`, force `None`. For **every other** interactive node — including nodes occluded by a `Block` node — `None` (set-if-changed). Each genuine transition toggles the matching `UiHovered`/`UiPressed` EnableTag bit (transition-gated, Decision 1). **This pass is separate from the Block break (closes the major finding)**: the Block break in 3a only stops *who becomes hovered*; the reset pass in 3b still visits every node, so a node that was `Hovered` last frame and is now occluded by a `Block` node this frame is correctly reset to `None` and loses its `UiHovered` bit.
4. **`RelativeCursorPosition`** (set-if-changed, hovered node only, if it has the column): `normalized = [(cursor.x-rect.x)/rect.w - 0.5, (cursor.y-rect.y)/rect.h - 0.5]`, guarded `rect.w>0 && rect.h>0` (NaN-free). On any non-hovered/leave row, write the canonical `{cursor_over:false, normalized:[0,0]}` so the equality gate stays quiet (Decision-1 minor fix).
5. **Same-frame press+release deferral**: a node pressed AND released this frame is queued in the pointer slot's `reset_next_frame` and reset to `None` next frame, so a one-frame click is observable by dispatch this frame.
6. **Press-origin + action stamp (Decision 2)**: on `clicked` over `hovered`, set the pointer slot's `pending_click = Some((hovered, on_click_index_or_NONE))` (reading the origin's `OnClick.0` now) and bump `press_gen`. On `released` over the **same** origin (`cursor_over` on origin), set `click_fired = pending_click` and clear `pending_click`.
7. **Keyboard focus + tab (Decision: total cross-root order, closes minor finding)**: on a Tab edge (`keys_just_pressed`), advance `UiInputFocus.focused` to the next `Focusable` by the **total order (root enumeration order, then `tab_index`, then `Entity`)** over the cached focusable list (sorted at refresh; cyclic wrap). Update the `UiFocused` bit. **If the currently-focused entity was despawned** (validated via `get_component_raw(focused, Focusable::component_id()).is_none()`), clear `UiInputFocus` + `UiFocused` first (mirrors the blur-clears-focus rule). On blur, clear focus.

**Z-order determinism (closes critical finding)**: the total order is **(StackIndex descending, then paint/document order descending — later-painted/deeper child wins, then `Entity` as the final stable tie-break)**. Because `StackIndex` defaults to `0` (verified `pub u32`, `components.rs:184`) the overwhelmingly common overlap case (parent+child both `StackIndex(0)`) is resolved by paint order: the child (painted after, on top) wins — matching draw order. The scratch is built in **paint order** and the hovered node is selected by a **stable** comparison on `(StackIndex, paint_seq, Entity)`, so the winner is deterministic across frames and platforms. The property-test oracle computes this **same** total order (not just max StackIndex). It must match P5a's paint order so hit-test and render agree on "top-most" (documented seam to P5a).

**Complexity**: O(N) point tests + O(M log M) one-time sort of M visible nodes (scratch reused). **Cache**: sequential over the SoA-collected scratch; `ComputedRect` is 16 B contiguous. **Branching**: two `f32` range compares per node (branchless-friendly). **SIMD**: the point-in-rect loop is auto-vectorizable (4 nodes/AVX2 lane) — P4-opt, not v1.

### `ui_dispatch_system` — Interaction edge → action (per-edge-kind complexity)
**Signature** (parallel system; writes only `ResMut<ActionState<A>>`):
```rust
pub fn ui_dispatch_system<A: Actionlike>(
    hovered: Query<(&OnHover, &Interaction), Changed<Interaction>>, // hover edge only
    submit:  Res<UiPointerState>,   // pending_submit_action stamped by focus
    pointer: Res<UiPointerState>,
    world:   /* read-only entity validator for the click re-check */,
    mut acts: ResMut<ActionState<A>>,
);
```
**Steps (each with its honest complexity — closes the minor finding):**
1. **Click — O(1)** (Decision 2): read `pointer.slot[0].click_fired = Some((origin, index))`. **Re-validate** the origin is still alive via `get_component_raw(origin, OnClick::component_id()).is_some()` (None on despawn/stale-gen/lost-OnClick → drop silently). If valid and `index != NONE`, `acts.ui_press(index as usize)`. The click does **not** rely on the `Changed<Interaction>` iterator (the release transitions to `None`, which would not reliably surface the origin) — it is a stamped O(1) lookup.
2. **Hover (None→Hovered) — O(changed-interaction rows)**: for each `(OnHover, Interaction::Hovered)` row in the `Changed<Interaction>` query (empty on a still frame, `NEEDS_CHANGE_DETECTION` const-fold), `acts.ui_press(on_hover.0 as usize)`.
3. **Submit — O(1)**: `OnSubmit` stamped by the focus system on an Enter edge while focused → `pending_submit_action` → `acts.ui_press(index)`.
**No `dyn`** — the only indirection is the `ResMut<ActionState<A>>` deref. **Alloc**: none.

### `ui_data_bind_system` — source field → widget buffer (split, double-gated)
```rust
// discovery (cheap; sets dirty)
pub fn ui_bind_discovery(
    static_changed: Query<(), Or<(Changed<C1>, Changed<C2>, ...)>>, // ui!-bound types
    mut scratch: ResMut<UiBindScratch>,                              // dynamic .ui-bound:
    // any_changed_since(&scratch.dynamic_bound_ids, last_run, this_run) -> dirty (Decision 6)
);
// apply (exclusive — reads source on entity X, writes sink on entity Y)
pub fn ui_bind_apply(world: &mut EcsMaster);
```
**`ui_bind_apply` steps (only when `dirty`):**
1. Iterate bound widgets (`Query<(&BindText, &mut UiTextBuffer)>`, widget-side, Decision 4).
2. **Source resolve + tick gate (Decision 5)**: `let tick = world.get_component_changed_tick(bind.source, bind.comp)?;` — `None` (despawned/stale-gen/**hosting-miss**, i.e. the source's archetype does not host `bind.comp`) → **skip** (no accessor call). This None-returning raw resolve is the documented precondition the trampoline SAFETY relies on (closes the binding minor finding). If `!tick.is_newer_than(last_run, this_run)` → skip.
3. **Format (codegen, Decision 7)**: `let acc = get_bind_accessor(bind.comp.0)?;` `let row = world.get_component_raw(bind.source, bind.comp)?;` format into a **stack scratch `UiTextBuffer`** via `acc.fmt(row, bind.field, &mut scratch_buf)`; **set-if-changed** the sink (compare formatted bytes; write only if different — prevents the sink-tick feedback loop). The sink write goes through `Mut<UiTextBuffer>` whose `DerefMut` bumps the tick — so P5's `Changed<UiTextBuffer>` gate fires only on a real text change.
4. **`BindValue`**: same flow, `acc.value(row, num) / acc.value(row, den)` (div-by-zero → 0.0), set-if-changed `UiValue`.

**`ComponentId`-is-type-identity (closes the binding minor finding)**: a `ComponentId` *is* the type's identity, so a stale `BindText` whose `comp` was registered for type A but whose `source` holds a different type at that id is impossible by construction. The only failure modes are despawn and hosting-miss, both caught by the `None`-returning resolve. **No per-frame `TypeId`/`Any` check** — the path stays reflection-free.

**Generated-code shape (example):**
```rust
#[derive(Component, Bindable)] #[repr(C)]
struct Health { current: f32, max: f32 }

// #[derive(Bindable)] EMITS:
#[repr(u8)] enum HealthField { Current = 0, Max = 1 }
impl Bindable for Health {
    const FIELD_COUNT: u8 = 2;
    #[inline]
    fn fmt_field(&self, field: u8, out: &mut dyn core::fmt::Write) -> core::fmt::Result {
        match field { 0 => write!(out, "{}", self.current), 1 => write!(out, "{}", self.max), _ => Ok(()) }
    }
    #[inline]
    fn value_field(&self, field: u8) -> f32 {
        match field { 0 => self.current, 1 => self.max, _ => 0.0 }
    }
    fn field_id(name: &str) -> Option<u8> {        // cold, parse-time
        match name { "current" => Some(0), "max" => Some(1), _ => None }
    }
    fn register_bind_accessor() {
        fn fmt_erased(p: *const u8, f: u8, out: &mut dyn core::fmt::Write) -> core::fmt::Result {
            // SAFETY: `p` was obtained by the caller (`ui_bind_apply`) from
            // `get_component_raw(source, Health::component_id())`, which returns
            // `Some` only when `source` is alive AND its archetype hosts this exact
            // ComponentId — so the bytes at `p` are a live, aligned `Health` row.
            // ComponentId == type identity, so no TypeId check is needed.
            let this = unsafe { &*(p as *const Health) };
            this.fmt_field(f, out)
        }
        fn value_erased(p: *const u8, f: u8) -> f32 {
            // SAFETY: as above — `p` is a live aligned `*const Health` row.
            let this = unsafe { &*(p as *const Health) };
            this.value_field(f)
        }
        boyko_ecs::component::install_bind_accessor(
            <Health as Component>::component_id().0,
            BindAccessor { fmt: fmt_erased, value: value_erased },
        );
    }
}
```
The `ui!` path calls `health.fmt_field(field, &mut buf)` with a **concrete `&mut UiTextBuffer`** sink (zero accessor indirection AND no `dyn Write` vtable, Decision 7). The `.ui` path calls `acc.fmt(row, field, &mut dyn Write)` (accessor fn-ptr + sink vtable, both only when that source changed).

**Complexity**: O(bound widgets) tick-resolve + compare when dirty; ~0 when not dirty (outer gate). **Cache**: source resolves are random-access (Phase-7 ~3 ns), change-gated. **Alloc**: none.

---

## Multithreading model
- **All three systems single-threaded.** `ui_focus_system` and `ui_bind_apply` are **exclusive** (`&mut EcsMaster`): focus reads ancestors while writing other nodes' `Interaction`; bind reads entity X's source while writing entity Y's sink — neither is a conflict-free parallel `Query`. `Arena` is `!Send`/`!Sync` (Phase-9), so exclusive is the correct form; the scheduler runs them in an exclusive window. **No shared state, no atomics, no locks** — data-race-free by exclusive access.
- `ui_dispatch_system` is a parallel system writing only `ResMut<ActionState<A>>` (exclusive resource access via the conflict graph — Phase-9 serializes writers).
- **Send/Sync**: all new components POD `Copy` → `Send + Sync`. `BindAccessor` = two `fn` pointers → `Send + Sync` (the new `[OnceLock<BindAccessor>; MAX_COMPONENTS]` table is `Send + Sync` exactly like `SERIALIZE`). Resources (`UiPointerState`, `UiInputFocus`, scratch) plain structs → `Send + Sync`.
- **boyko_ecs read-only accessors** (`get_component_changed_tick`, `any_changed_since`) take `&self` and mutate nothing — they cannot introduce a data race; they are callable from the exclusive bind systems and the discovery probe.
- **Proof of data-race freedom**: exclusive `&mut EcsMaster` ⟹ unique world access ⟹ no concurrent reader/writer. The dispatch system's only mutation is one exclusive resource, serialized by the conflict graph. The fixed-snapshot composition (Decision 10) introduces no shared state — `ui_press` mutates the same `ActionState<A>` the input update owns, in-order, within the input window.

---

## Integration
- **New modules** (boyko_ui): `interaction/{mod,components,action,focus,dispatch}.rs`, `binding/{mod,components,bindable,accessor,bind_system}.rs`, plugin additions.
- **Modified (boyko_ui)**: `text/dispatch.rs` — five new `match` arms + leaf parsers; `lib.rs`/prelude re-exports.
- **Modified (boyko_input)** — **two** edits (the prior "one change only" is retracted):
  1. `action/state.rs` — `pub fn ui_press` / `ui_set_value` (Decision 9). `ui_press` writes the **live** edge + level only; no `fixed_just_pressed` write (Decision 10 carries it via the existing freeze).
  2. `raw/queue.rs` — add `cursor_inside: bool` (and/or `window_focused: bool`) to `PhysicalInput`, routed from `CursorEntered`/`CursorLeft`/`WindowFocus` in `apply` (Decision 12). The input adapter (Win32/winit) routes those OS events. The egui adapter (I7) stays dev-tools-only behind its feature gate; the game HUD feeds actions via `ui_press` (opposite direction from device adapters).
- **Modified (boyko_ecs)** — **NOT none** (prior draft retracted):
  1. `ecs_master.rs` — `pub fn get_component_changed_tick` (read-only, reuses the `get_component_raw` prologue + the now-same-crate `read_changed_tick`; one SAFETY note) and `pub fn any_changed_since` (read-only id-keyed archetype probe, Decision 6).
  2. `component_registry.rs` — `static BIND_ACCESSORS: [OnceLock<BindAccessor>; MAX_COMPONENTS]` + `pub fn get_bind_accessor` / `pub fn install_bind_accessor`, mirroring the `SERIALIZE`/`get_serialize_info`/installer pattern verbatim. `BindAccessor` type defined here.
- **Modified (boyko_macros)**: new `#[proc_macro_derive(Bindable)]` (~shape of the serialize derive).

---

## Implementation plan (for the developer)
1. **boyko_ecs** `component_registry.rs`: `BindAccessor` + `BIND_ACCESSORS` table + `get_bind_accessor`/`install_bind_accessor` (copy the `SERIALIZE` pattern). `ecs_master.rs`: `get_component_changed_tick` + `any_changed_since` (read-only; SAFETY notes). Unit-test: tick read matches `Changed<C>` semantics; `any_changed_since` short-circuits and is O(hosting-archetypes) on a still frame.
2. **boyko_input** `state.rs`: `ui_press`/`ui_set_value` (live-edge only). `queue.rs`: `cursor_inside`/`window_focused` field + `apply` routing. Unit-test: UI live edge survives the Decision-10 ordering into exactly one fixed batch (no-miss/no-double-count, copy the existing input test shape); blur clears interaction.
3. **boyko_ui** `interaction/components.rs` + `action.rs`: the six components.
4. **boyko_ui** `interaction/focus.rs`: `ui_focus_system` (exclusive), `UiPointerState` (1-slot array, Decision 11) / `UiInputFocus` / `UiInteractionScratch` resources + `UiInteractionConfig` (EnableTag ids), DPI conversion (Decision 13), blur reset (Decision 12), total Z-order, Block-then-unconditional-reset, same-frame deferral, stamp-at-press, total cross-root tab order + despawned-focus clear.
5. **boyko_ui** `interaction/dispatch.rs`: `ui_dispatch_system<A>` — O(1) stamped click with re-validation, O(changed) hover, O(1) submit.
6. **boyko_macros**: `#[derive(Bindable)]` (field enum, `fmt_field`/`value_field`/`field_id`/`register_bind_accessor`, erased trampolines with SAFETY).
7. **boyko_ui** `binding/{components,bindable,accessor}.rs`: `BindText`/`BindValue`/`UiTextBuffer`/`UiValue`, `Bindable` trait, `TemplateId`, accessor glue over `get_bind_accessor`.
8. **boyko_ui** `binding/bind_system.rs`: `ui_bind_discovery` (static `Or` + dynamic `any_changed_since`) + `ui_bind_apply` (tick gate via `get_component_changed_tick`, set-if-changed both sinks).
9. **boyko_ui** `text/dispatch.rs`: five `.ui` `match` arms + leaf parsers (action-name→index, field-name→u8, `#name`→Entity).
10. **boyko_ui** `plugin.rs`: `UiInteractionPlugin<A>` + `UiBindingPlugin`; ordering (`ui_focus_system` → `ui_dispatch_system` **inside the input window, before `freeze_fixed_snapshot`**, Decision 10; `ui_bind_discovery`/`apply` late in Main).
11. Tests + benches (below).

---

## Metrics and validation

**Unit tests (mandatory):**
- Click fidelity: press-inside + release-inside-same-node writes the `ActionState` edge; press-inside + release-**outside** writes nothing; press A, release B → nothing.
- **Click despawn/reparent (Decision 2)**: press a button, despawn it before release → no spurious action; press, **reparent** (same Entity/gen) → click still fires; press, despawn+recycle the id to a different gen → no fire (generation guard).
- **Fixed-snapshot no-miss/no-double-count (Decision 10)**: UI press on a 0-substep frame → observed by exactly one fixed batch, cleared once; UI press on a multi-substep frame → fires once.
- Z-order: two overlapping rects, higher `StackIndex` wins; **equal `StackIndex` (both 0) parent+child overlap → the child (later paint) wins, deterministically**; `Block` on top → node below is `None`; `Pass` → node below also `Hovered`.
- **Block resets occluded nodes (Decision)**: node A `Hovered` last frame, `Block` node B moves over it this frame → A becomes `None` this frame (the unconditional reset pass ran).
- **Blur/leave (Decision 12)**: held-press, then `cursor_inside=false` → `Interaction=None`, `UiPressed` cleared, `pending_click` cancelled, focus cleared.
- **HiDPI hit-test (Decision 13)**: `scale_factor=2.0`, physical cursor `(200,200)`, logical rect `(0,0,150,150)` → hit; same physical cursor, rect `(0,0,90,90)` → miss.
- `RelativeCursorPosition`: center → `[0,0]`; corners → `±0.5`; w/h==0 → no NaN, `cursor_over=false`; leave → canonical `[0,0]` (gate stays quiet).
- Same-frame press+release → observable `click_fired` for exactly one frame.
- Tab order: cyclic advance by the **total cross-root order**; window-blur clears focus + bit; **despawning the focused entity clears `UiInputFocus` + `UiFocused`**.
- Bind: changing `Health.current` updates `UiTextBuffer`; an unchanged-source frame does NOT touch the buffer (assert tick unchanged) and runs **zero** accessor calls; `BindValue` ratio `max==0` → 0.0 (no NaN); set-if-changed keeps the sink tick quiet on identical text; **hosting-miss** (source lost `Health`) → skip, no accessor call, no panic; reading the source does **not** bump the source's changed tick (assert `Changed<Health>` is not re-triggered by the bind read — guards against the `get_component_mut` footgun).
- Authoring: `ui!`-authored vs `.ui`-authored `OnClick`/`BindText` produce identical components (extends the P3 equivalence test).
- `#[derive(Bindable)]`: `field_id` round-trips; `fmt_field`/`value_field` out-of-range = no-op; erased trampolines format identically to the typed path.

**Property/fuzz:**
- Random cursor paths over random rect stacks → `Interaction` is always exactly one of {None,Hovered,Pressed}, and the hovered node is always the **total-order top-most hit** (oracle computes the SAME `(StackIndex desc, paint_seq desc, Entity)` order).
- `.ui` bind fuzz: random field/component names → recoverable error or a valid binding, never UB (mirrors the serialization loader fuzz discipline).

**Benchmarks (criterion):**
- `ui_focus_system` over 1k/10k nodes (hovered vs not-hovered); HiDPI vs 1.0×.
- `ui_dispatch_system` still-frame (~0) vs N clicks.
- `ui_bind_apply` still-frame (~0, asserts the outer gate) vs N changed sources; `ui!` monomorphized (concrete sink) vs `.ui` erased (fn-ptr + dyn Write) path.
- **`any_changed_since` `.ui`-dynamic still-frame (~0)** — parallel to the `ui!`-static still-frame bench, proving the dynamic gate is measured, not assumed.

**`debug_assert!` invariants:**
- `field < FIELD_COUNT` before every accessor call (release path = silent skip); `OnClick.0 < A::COUNT` at dispatch; `rect.w>=0 && rect.h>=0` before normalize; `bind.source` alive (else skip, not panic); `UiTextBuffer.len <= CAP`; `UiViewport.scale_factor > 0.0` before the divide.

**No-dyn / no-alloc assertions:**
- A counting-allocator test runs N still frames of all three systems → **0 allocations**, **0 EnableTag enable/disable calls** (transition-gated), **0 accessor calls** (change-gated).
- A CI grep/clippy gate: no `Box<dyn`, `Rc`, `RefCell`, `format!`, `String::` in `interaction/` or `binding/` hot paths.

---

## Out of scope (seams documented)
- **Rendering → P5**: `ComputedRect` + `UiTextBuffer` + `UiValue` + `StackIndex` are the render seam; P5's quad/text upload is `Changed<ComputedRect>`/`Changed<UiTextBuffer>`-gated (set-if-changed sinks make those gates effective). P5a must use the **same total Z/paint order** as the hit-test (Z-order seam, Decision in focus step).
- **Widget presets → P6**: P6 composes `OnClick`+`Interaction`+`BindText` into `#[derive(Bundle)]` presets; P4 ships the primitives.
- **Text shaping/SDF → P5b**: `UiTextBuffer` holds formatted UTF-8; shaping → glyphs is P5b.
- **Multi-pointer / touch (Decision 11)**: v1 is single-pointer (mouse, slot 0); `UiPointerState` is a fixed array so multi-touch is non-breaking later; touch also needs a `boyko_input` device-side extension.
- **2D/directional focus navigation**: v1 is linear total-order Tab only; directional is later.
- **`InheritedVisibility`**: P4 hit-test assumes all laid-out nodes visible; a `Visible` EnableTag bit lands in P5 (Open question 1).

## Open questions
1. **`InheritedVisibility` / `Visible` bit timing** — P4 hit-test should skip invisible nodes, but visibility is a P5 concern. v1 assumes all laid-out nodes visible; confirm whether a `Visible` EnableTag bit lands in P4 (cheap, one more bit) or waits for P5. Non-blocking: v1 is correct for the all-visible case.
2. **`any_changed_since` archetype-epoch granularity** — the dynamic gate tests per-archetype column change epochs. Confirm the existing archetype/column change-epoch surface exposes a per-`ComponentId` "changed since tick" at archetype granularity (the typed `Changed<C>` filter already reads per-row ticks; `any_changed_since` needs the archetype-level OR of those). If only per-row ticks exist, `any_changed_since` iterates the hosting archetypes' rows with short-circuit (still O(changed) with early-out; bench confirms ~0 still-frame). One-file confirmation before step 1.

## Changes from review

**Critical findings — all fixed:**
1. **Blur/cursor-leave phantom flag** (focus step 1): the prior "PhysicalInput flag — or cursor_pos sentinel" was verified non-existent (`cursor_pos` is a persisted level defaulting to `[0,0]`, a real hittable point). Added **Decision 12**: a real `boyko_input` edit (`cursor_inside`/`window_focused` on `PhysicalInput`, routed from OS `CursorLeft`/`WindowFocus`); focus step 1 is now a real blur short-circuit (reset all interaction, cancel pending clicks, clear focus). Retracted "boyko_input: one change only" — there are now two boyko_input edits.
2. **Cursor coordinate space / DPI** (focus steps 2/4): added **Decision 13** — convert physical `cursor_pos` to logical via the **existing** `UiViewport.scale_factor` (verified present, "logical = physical / scale"), narrowing f64→f32 once after the divide; all hit-tests run in logical space. Added a mandatory HiDPI (`scale_factor=2.0`) hit-test.
3. **Bind change-gate primitive** (Decision 5): verified `get_component_ref`/tick-returning accessor does **not** exist; `get_component` has no tick, `get_component_mut`'s `Mut` *bumps* the source tick (would corrupt `Changed<Source>`), `read_changed_tick` is `pub(crate)` + row-keyed. Rewrote Decision 5 to add a **new public read-only** `EcsMaster::get_component_changed_tick(entity, ComponentId) -> Option<Tick>` (reuses the `get_component_raw` prologue + the same-crate `read_changed_tick`; never mutates a tick). Retracted "boyko_ecs: NONE"; Integration now lists the boyko_ecs edits honestly. Added a test asserting the bind read does **not** re-trigger `Changed<source>`.
4. **`BindAccessor` home / false "registry already stores serialize hooks in a fn-ptr table" precedent** (Decision 7): verified the registry is parallel `[OnceLock<X>; MAX_COMPONENTS]` tables (`LAYOUTS`/`HOOKS`/`CLONE`/`MAP_ENTITIES`/`SERIALIZE`) — `ComponentLayout` is fixed-shape; serialization is `SERIALIZE` + `get_serialize_info`, a parallel table (so the precedent is *real* but is a table, not a free `ComponentLayout` slot). Committed Decision 7 to a **new `BIND_ACCESSORS` parallel table** mirroring `SERIALIZE` verbatim, declared as a boyko_ecs edit. Removed the "add a slot to the existing entry within existing extensibility / NONE" wording and the **Open-Q1 fork** (registry-vs-table is now a single decision, not a `we can use A or B`).
5. **Z-order tie-break determinism** (focus step 2): verified `StackIndex` is `pub u32` default 0 (the common overlap case has no defined winner under "max StackIndex"). Defined a **total order** `(StackIndex desc, paint/document order desc, Entity)`; the scratch is built in paint order and selected by a stable compare; the property-test oracle computes the same order; documented the P5a paint-order seam.

**Major findings — all fixed:**
- **Block must still reset lower nodes** (focus steps 2–3): split into 3a (hover resolution — Block stops *who becomes hovered*) and 3b (unconditional reset pass — visits **every** interactive node, so a node occluded by a new Block node is reset to `None`). Added the occlusion-reset test.
- **Multi-pointer unmodeled** (Decision 11): stated single-pointer as an explicit v1 scope boundary AND shaped `UiPointerState` as a fixed `[PointerSlot; MAX_POINTERS=1]` array so multi-touch is non-breaking later.
- **press_origin lifetime / two click paths** (Decision 2): committed to **one** path — stamp the resolved `(Entity, u16)` at press time; re-validate the origin's aliveness+generation via `get_component_raw` (verified None-on-stale) at release; reparent (same id/gen) still fires; despawn/recycle drops silently. Click is O(1), independent of `Changed<Interaction>` on the release frame (the critic's correctness point). Made dispatch complexity **per-edge-kind** (O(1) click, O(changed) hover, O(1) submit). Added despawn/reparent tests.
- **Schedule ordering vs fixed snapshot** (Decision 10): removed the contradictory "after update_action_state + a second `fixed_just_pressed` writer." Now `ui_focus_system`+`ui_dispatch_system` run **inside the input window, before `freeze_fixed_snapshot`**, so `ui_press` writes only the live edge and the **existing** OR-accumulate + clear-on-consume protocol carries it (verified `freeze_fixed_snapshot`/`clear_fixed_edges`). Gave the explicit no-miss/no-double-count proof and a mandatory 0-substep test.

**Minor findings — all addressed:**
- EnableTag toggle is **transition-gated** (same guard as the set-if-changed enum write); still-frame test asserts zero enable/disable calls (Decision 1).
- `RelativeCursorPosition` resets `normalized` to canonical `[0,0]` on leave so the equality gate stays quiet; the EnableTag toggle is driven only by the `Interaction` transition, not by cursor movement (Decision 1).
- Cross-root tab order made a **total** order; despawned-focus clears `UiInputFocus`+`UiFocused` (focus step 7).
- `.ui` path's **two** indirections (accessor fn-ptr + `dyn Write` vtable) named explicitly; the `ui!` path uses a **concrete `&mut UiTextBuffer`** sink to drop the vtable on the hot path; the `.ui` trampoline keeps `dyn Write` (unavoidable) (Decision 7).
- Bind missing/hosting-miss handling specified as a `None`-returning `get_component_raw` resolve that **skips** before any accessor call — this is the documented precondition the trampoline SAFETY relies on; `ComponentId`-is-type-identity argument stated so no per-frame `TypeId`/`Any` check creeps in.
- Dynamic `.ui` 0%-gate given a concrete primitive (`any_changed_since`, Decision 6) with a still-frame bench, replacing the hand-wave.

**Files verified during revision** (absolute paths): `D:\claude\BoykoEngine\crates\boyko_input\src\raw\queue.rs` (no focus/cursor-inside flag; `cursor_pos` persisted, default `[0,0]`), `D:\claude\BoykoEngine\crates\boyko_input\src\action\state.rs` (`freeze_fixed_snapshot` OR-accumulate, `clear_fixed_edges` gated on `steps_this_frame>0`), `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\ecs_master\ecs_master.rs` (`get_component` no tick, `get_component_mut` bumps tick, `get_component_raw` None-on-stale at lines 1803/1963/2008), `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\memory\component_pool.rs` (`read_changed_tick` pub(crate), row-keyed, line 1632), `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\component\component_registry.rs` (parallel `[OnceLock<X>; MAX_COMPONENTS]` tables; `SERIALIZE`/`get_serialize_info` precedent at 1918/1928), `D:\claude\BoykoEngine\crates\boyko_ui\src\components.rs` (`StackIndex(pub u32)` default 0; `ComputedRect` logical px), `D:\claude\BoykoEngine\crates\boyko_ui\src\resources.rs` (`UiViewport.scale_factor`, "logical = physical / scale").