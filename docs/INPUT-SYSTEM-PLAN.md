# boyko_input — Source-Agnostic Rebindable Action Mapping (System Design Plan)

> Intended path: `docs/INPUT-SYSTEM-PLAN.md`
> Status: design-complete, revised against architecture-critic round 1 (C1/C2/C3 + W1–W4 + O1–O3 resolved).
> Branch: `ecs`. Engine path is fully in-house: NO serde / toml / ron / winit / leafwing / eframe on the `boyko_input` engine path.

---

## 1. Goal

A new engine-path crate, `boyko_input`, that turns raw keyboard/mouse events from **any** source (native raw-FFI Win32 window, egui demo, synthetic test stream) into typed, rebindable **actions** consumed by ECS systems through `Res<…>` / a `SystemParam`.

Performance / functional targets:

- **Zero per-frame heap allocation** on the ingest path. All state is preallocated at plugin build; the per-frame system only writes into fixed buffers.
- **No `dyn` / `Box` / `HashMap` on the hot path.** Binding dispatch is a `match` over a `#[repr(u8)]` enum; action state is array-by-id (engine principle 1).
- **Source-agnostic.** `boyko_input` depends on NO windowing lib. Both the native Win32 window (`boyko_rhi_vulkan/src/window.rs`) and the egui demo feed it through a thin push API.
- **Deterministic w.r.t. the fixed-step sim.** Input is sampled once per frame on the Main schedule, edges are accumulated from the **event stream** (not level diff), and the fixed loop reads a frame-stable snapshot so a 0-substep or N-substep frame never misses nor double-counts a press (the Phase-20 footgun — resolved in Decision 7 against the *actual* clock API).
- **Cost budget:** ingest system ≤ a few ns per pending raw event + O(active bindings) per frame. For a typical 30-action / 60-binding map this is well under 1 µs/frame, off every hot loop. A `just_pressed` query is one bit test (~1 ns).

---

## 2. Scope (MVP vs deferred)

**v1 (in):** physical `KeyCode` + mouse (buttons, raw delta, cursor position, wheel); typed `Actionlike` enum via `#[derive]`; Button / Axis1D / Axis2D actions; chords / modifiers; WASD composite; contexts (single-active + a small priority stack); clash resolution (`PrioritizeLongest`); the `.keys` human-editable text persistence + override-delta semantics + versioning; runtime rebinding + conflict detection; Win32 + egui adapters.

**Deferred (seams left, listed in §13):** gamepad / analog stick (`BindSpec::Stick` variant reserved, parsed + round-tripped, ignored at runtime); touch; action recording/replay (the `RawInputQueue` ring is the natural record point); non-US keycap labels for the rebind UI (label-only, cosmetic); `generic_const_exprs` array sizing; threaded SPSC pump; IME / `WM_DEADCHAR` composition beyond basic `Text(char)`; `WM_INPUT` raw-delta (staged behind `WM_MOUSEMOVE` — see W3 resolution / §11).

---

## 3. Owner VALUE / SCOPE decisions (decide upfront)

These are not perf forks (the architect decides those). They are values / scope. Each has a recommended default so the plan is complete; each is independently reversible.

| # | Call | Recommended default | Why it is the owner's |
|---|------|---------------------|------------------------|
| V1 | Typed action enum vs string-named actions | Typed enum + `#[derive(Actionlike)]` (compile-time `COUNT` + `index()`) | API ergonomics + dependency philosophy |
| V2 | Persistence: in-house text vs binary | Human-editable text `.keys` (custom hand parser, no serde/toml) | User-facing / moddable values call |
| V3 | Contexts/sets in v1 | Yes — minimal: a fixed set of named contexts, single-active + a small priority stack | Scope |
| V4 | Gamepad / touch in v1 | Deferred to v2 (keyboard + mouse only) | Scope |
| V5 | Action recording / replay in v1 | Deferred (design leaves a seam at `RawInputQueue`) | Scope |
| V6 | `KeyCode` repr stability as public ABI | `#[repr(u16)]`, `#[non_exhaustive]`, append-only | Stability promise |
| V7 | Clash strategy default | `PrioritizeLongest`, per-context override in the file | Gameplay-UX value |
| V8 | Action-count cap | **256** (drops out of the C2 resolution — see Decision 5) | Affects how many actions one enum may declare |
| V9 | Chord cap | `[KeyCode; 4]` (covers Ctrl+Shift+Alt+key) | Binding expressiveness |
| V10 | One-frame input→fixed latency | Accept (standard, imperceptible ≥60 Hz; the price of determinism) | Gameplay-feel value |

The rest of the plan assumes these defaults.

---

## 4. Crate structure and dependency position

```
crates/boyko_input/
  Cargo.toml
  src/
    lib.rs            # re-exports, prelude
    prelude.rs
    raw/              # source-agnostic raw layer (no windowing deps)
      keycode.rs      # KeyCode, MouseButton, ButtonState, ScrollDelta
      event.rs        # RawInputEvent
      queue.rs        # RawInputQueue (ring), PhysicalInput
      scancode.rs     # static scancode -> KeyCode tables
    action/
      actionlike.rs   # Actionlike trait + ActionKind
      state.rs        # ActionState<A>  (SoA)
      map.rs          # InputMap<A>, BindSpec, InputRef, InputMapBuilder
      process.rs      # process_actions / update_action_state
      clash.rs        # subset-clash resolution
      rebind.rs       # RebindSession state machine
      resource_id.rs  # generic-resource ResourceId minting (C1 fix)
    persist/
      grammar.rs      # .keys parser (one-pass, paren-depth aware)
      writer.rs       # canonical serializer
    plugin.rs         # InputPlugin<A> (Phase-18 facade)
    adapter/
      win32.rs        # #[cfg(feature="adapter_win32")] translate(...)
      egui.rs         # #[cfg(feature="adapter_egui")] translate(...)
```

Dependency edge (engine path):

```
boyko_input ──► boyko_ecs        (Resource, Res/ResMut, SystemParam, App/Plugin, Schedule, FixedTime, Time)
            ──► boyko_utils      (BitSet256, growable BitSet, SparseMap)
            ──► boyko_macros      (#[derive(Resource)], new #[derive(Actionlike)])
```

`boyko_input` does **NOT** depend on `boyko_rhi_vulkan`, winit, winapi-as-dep, or eframe. The reverse glue (drain the OS queue → `translate` → `push_raw`) lives at the **edge** — in the runner binary / `boyko_demo` / the `boyko_rhi_vulkan` window — never in the input core.

Adapters are **feature-gated** (`adapter_win32`, `adapter_egui`), each a thin free-fn module that is the only place naming a source type.

**Why this boundary** (Decision 1): mirrors the existing `boyko_rhi` / `boyko_rhi_vulkan` backend-behind-boundary split and the Bevy `bevy_input` / `bevy_winit` separation. Keeping windowing out of core means the ECS-facing action layer compiles on every target (incl. wasm) and one action map works for Win32 and egui with no `cfg` soup in core. **Rejected:** (a) module inside `boyko_ecs` — drags an input concern into the ECS core; (b) `boyko_input` → `boyko_rhi_vulkan` — couples input to the renderer + Windows, breaks wasm. **Trade-off:** the application owns the small glue that drains the OS message queue; that glue is unavoidable and belongs at the edge.

---

## 5. The source-agnostic raw-input layer

### 5.1 Canonical physical enums (Decision 2)

```rust
// repr(u16): a key indexes a flat static table with no hashing.
// non_exhaustive + append-only = stable public ABI (V6).
#[repr(u16)] #[non_exhaustive] #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum KeyCode {
    KeyA, /* … */ KeyZ, Digit0, /* … */ Digit9,
    F1, /* … */ F24, Space, Enter, Escape, Tab, Backspace,
    ArrowUp, ArrowDown, ArrowLeft, ArrowRight,
    Numpad0, /* … */ NumpadEnter,
    ShiftLeft, ShiftRight, ControlLeft, ControlRight, AltLeft, AltRight,
    SuperLeft, SuperRight, /* … */
    /// Carries the raw OS scancode for keys not in the canonical set —
    /// never drops an exotic key; round-trips through the `.keys` format (§9).
    Unidentified(u32),
}

#[repr(u8)] #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MouseButton { Left, Right, Middle, Back, Forward, Other(u16) }

#[repr(u8)] #[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonState { Pressed, Released }

#[derive(Clone, Copy, Debug)]
pub enum ScrollDelta { Lines { x: f32, y: f32 }, Pixels { x: f64, y: f64 } }
```

**Why physical, not virtual:** physical-position binding is mandatory correctness — virtual keycodes give the AZERTY/QWERTZ "default WASD lands on the wrong keys" bug. `#[repr(u16)]` lets a key index a flat `[…; KEY_TABLE_LEN]` lookup with no hashing. Engine ownership keeps winit/Win32 symbols out of core. `Unidentified(u32)` never loses a key. **Trade-off:** a rebind UI maps physical→keycap *label* only via a US-default `key_label(KeyCode) -> &'static str`; locale labels are the application's job (cosmetic).

### 5.2 The single seam type

```rust
#[repr(C)] #[derive(Clone, Copy, Debug)]
pub enum RawInputEvent {
    Key { code: KeyCode, state: ButtonState, repeat: bool },
    Text(char),                          // logical char — text fields only, never gameplay
    MouseButton { button: MouseButton, state: ButtonState },
    MouseMotion { dx: f64, dy: f64 },    // raw relative delta (WM_INPUT/DeviceEvent) — camera
    CursorMoved { x: f64, y: f64 },      // absolute window pos (WM_MOUSEMOVE) — UI
    Wheel(ScrollDelta),
}
```

### 5.3 The adapter seam = a push API + per-source `translate`, NOT a trait object (Decision 3)

The core exposes one push method on a resource:

```rust
impl RawInputQueue {
    /// Called O(events/frame) from the runner thread BEFORE the scheduler
    /// window. No allocation (fixed ring). Overflow policy: §5.4.
    pub fn push_raw(&mut self, ev: RawInputEvent);
}
```

Each frontend is a feature-gated, monomorphized free function that is the only code naming the source type:

```rust
#[cfg(feature = "adapter_win32")]
pub mod win32 {
    pub fn translate(msg: u32, wparam: usize, lparam: isize) -> Option<RawInputEvent>;
    pub fn keycode_from_scancode(scancode: u32, extended: bool) -> KeyCode; // static [KeyCode;256]×2
}
#[cfg(feature = "adapter_egui")]
pub mod egui_adapter {
    pub fn translate(ev: &egui::Event) -> Option<RawInputEvent>;
}
```

**Why a data seam, not `Box<dyn InputBackend>`:** a trait object would put virtual dispatch on the per-event path and force core to own a backend object — both forbidden. A push-data seam keeps core dependency-free, monomorphizes every adapter, and makes testing trivial (`push_raw` synthetic events). **Rejected:** `trait InputBackend { fn poll() }` held by core (dyn + core owns windowing). A blanket `From<source> for RawInputEvent` is kept as an ergonomic wrapper *on top of* `translate`, but the load-bearing seam is the push fn.

### 5.4 Ring buffer + overflow policy (resolves O1)

```rust
#[derive(Resource)]                       // non-generic — derive is sound here (C1 note)
pub struct RawInputQueue {
    buf: Box<[RawInputEvent]>,            // one-time alloc at build; cap = RAW_QUEUE_CAP (default 1024)
    head: u32, tail: u32,                // power-of-two cap → mask wrap, branchless
    high_water: u32,                     // debug observability
    dropped: u32,                        // count of drop-oldest evictions this frame
}
```

**Overflow policy: drop-oldest.** On a slow frame with a key-repeat storm, the *newest* events (the player's latest intent) are the ones that must survive; the oldest stale repeats are evicted. `debug_assert!(self.dropped == 0, "RawInputQueue overflow — raise RAW_QUEUE_CAP")` fires in debug so the cap is tuned, never silently lossy in dev. The ring is drained fully each frame by `update_action_state`, so overflow only occurs within a single frame's burst.

### 5.5 Per-frame physical snapshot

```rust
#[derive(Resource)] #[repr(C, align(64))]
pub struct PhysicalInput {
    keys_pressed:       BitSet256,   // level: held this frame
    keys_just_pressed:  BitSet256,   // edge: accumulated from the EVENT STREAM (W4)
    keys_just_released: BitSet256,   // edge: accumulated from the EVENT STREAM (W4)
    mouse_pressed: u8, mouse_just_pressed: u8, mouse_just_released: u8,
    mouse_delta:  [f64; 2],          // summed raw motion this frame
    cursor_pos:   [f64; 2],          // last absolute position
    wheel:        [f64; 2],          // summed wheel this frame
}
```

Capacity choice (256) drops out of the C2 resolution — see Decision 5.

---

## 6. The action-map layer

### Decision 4 — Action identity: typed enum via `#[derive(Actionlike)]`

```rust
pub trait Actionlike: Copy + Eq + 'static {
    const COUNT: usize;                       // dense action count → array sizing
    fn index(self) -> usize;                  // dense 0..COUNT discriminant
    fn from_index(i: usize) -> Option<Self>;
    fn kind(self) -> ActionKind;              // Button | Axis1D | Axis2D, per-variant via #[actionlike(...)]
    fn name(self) -> &'static str;            // for the .keys format & rebind UI
}

#[repr(u8)] #[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActionKind { Button, Axis1D, Axis2D }
```

User code:
```rust
#[derive(Actionlike, Clone, Copy, PartialEq, Eq)]
enum PlayerAction {
    Jump,                              // default kind: Button
    #[actionlike(Axis2D)] Move,
    Fire,
    #[actionlike(Axis1D)] Throttle,
}
```

**Why a closed compile-time enum, not a `TypeId` registry:** `boyko_ecs` uses lazy `TypeId`→dense-id registries for `Component`/`Resource`/`State<S>` because those id spaces are open and discovered at runtime across crates. Actions are a single closed user enum known at compile time. `const COUNT` + `index()` lets `ActionState`/`InputMap` be fixed `[…; COUNT]`-shaped arrays — zero runtime registration, zero hashing, perfect cache layout — strictly better here. **Rejected:** leafwing's `HashMap<A, …>` (hash + box per binding); string actions (runtime id minting + hashing); a `TypeId` registry like `State<S>` (unnecessary indirection for a closed set). **Trade-off:** the action set is fixed per enum at compile time. You rebind *inputs*, not invent actions at runtime; mod-defined new actions would need a separate string-keyed backend (deferred — the `.keys` format already carries action *names*, leaving the seam).

### Decision 5 — `ActionState<A>` is SoA bitsets + parallel value arrays (resolves C2)

The critic correctly found that **`BitSet512` does not exist** in `boyko_utils` — only `BitSet256` (fixed, 32 B, `align(32)`, static-asserted) and the growable heap `BitSet<T>`. **Resolution: re-base the design on `BitSet256` and set the action-count cap at 256 (V8).** Consequences propagated below.

```rust
#[repr(C)]
pub struct ActionState<A: Actionlike> {
    // --- hot: 4 × 32 B = 128 B = 2 cache lines, no alloc ---
    pressed:        BitSet256,        // bit i = action i held this frame
    just_pressed:   BitSet256,        // rising edge, valid this frame's reads
    just_released:  BitSet256,        // falling edge
    consumed:       BitSet256,        // clash-suppressed OR user-consumed this frame
    // --- frame-stable snapshot the FIXED loop reads (C3) ---
    fixed_pressed:      BitSet256,    // level, frozen for the whole frame's fixed loop
    fixed_just_pressed: BitSet256,    // edge, frame-stable; single-consume owned by ingest (C3)
    fixed_just_released:BitSet256,
    // --- cold values: allocated ONCE at plugin build, never per frame ---
    button_value:   Box<[f32]>,       // len = A::COUNT; analog 0..1 / -1..1 for Button/Axis1D
    axis2:          Box<[[f32; 2]]>,  // len = A::COUNT; deadzoned+clamped 2D for Axis2D
    fixed_value:    Box<[f32]>,       // frame-stable mirror for the fixed loop
    fixed_axis2:    Box<[[f32; 2]]>,
    _pd: PhantomData<A>,
}
```

**Why SoA:** the hot query "is action N just_pressed?" is one bit test (branchless, cache-dense). Edge sets as bitsets make per-frame edge math a vectorizable `new = cur & !prev` over `u64` words (SIMD-friendly). Values live in separate dense arrays so a button-only game never touches `axis2` (hot/cold split). **Rejected:** AoS `[ActionData; N]` (leafwing) mixes hot bits with cold f32/Vec2, wasting cache on button-only queries.

**C2 cache-line math, corrected:** `BitSet256` is 32 B = half a cache line; 256 actions per set. The four hot edge/level sets = 128 B (2 lines), the four frozen fixed sets = another 128 B. A `just_pressed(a)` query touches exactly one 32 B set → one cache line. The "no `BitSet512`, no phantom type" debt is closed: the design now uses a type that exists and compiles.

**Action-count cap = 256** (`debug_assert!(A::COUNT <= 256, "action enum exceeds BitSet256 capacity")`). 256 distinct actions is well beyond any real input map (Bevy/leafwing games run 10–60). A `COUNT > 256` enum is a cold, exotic case; if it ever arises, a `BitSetN`-generic fallback is a localized future change, not a v1 concern. **Perf budget restated under 256:** edge recompute scans 4 `u64` words/set; `process_actions` for 30 actions/60 bindings stays < 1 µs; `just_pressed` ~1 ns.

**One-time setup heap** for the four value arrays (`Box<[f32]>` / `Box<[[f32;2]]>`) is allocated in `InputPlugin::build` (cold path), never per frame — does not violate the no-per-frame-alloc rule. Removable later under `generic_const_exprs` (nightly, deferred).

### Decision 6 — `InputMap<A>`: `#[repr(u8)]` binding records, `match`-dispatched, flat arena

```rust
#[repr(u8)] #[derive(Clone, Copy)]
pub enum InputRef { Key(KeyCode), Mouse(MouseButton) }

#[repr(u8)] #[derive(Clone, Copy)]
pub enum BindSpec {
    Key(KeyCode),
    Mouse(MouseButton),
    Chord { keys: [KeyCode; 4], len: u8 },                 // Ctrl+S etc.; len ≤ 4 (V9)
    Axis1 { neg: InputRef, pos: InputRef, dz: f32 },
    Axis2 { up: InputRef, down: InputRef, left: InputRef, right: InputRef,
            dz: f32, mode: AxisMode },                     // WASD→2D; DigitalNormalized default
    Stick { /* deferred gamepad seam; parsed + round-tripped, ignored at runtime */ },
    None,                                                  // explicit unbind
}

#[repr(u8)] #[derive(Clone, Copy)]
pub enum AxisMode { DigitalNormalized, DigitalRaw }

#[repr(u8)] #[derive(Clone, Copy)]
pub enum ClashStrategy { PrioritizeLongest, AllowAll }

pub struct InputMap<A: Actionlike> {
    bindings: Box<[BindSpec]>,        // flat arena, allocated once at build
    ranges:   Box<[(u32, u32)]>,      // (start,len) into `bindings`, indexed by A::index(); len = A::COUNT
    clash:    ClashStrategy,
    _pd: PhantomData<A>,
}
```

**Why:** `match` over a `#[repr(u8)]` enum monomorphizes to a jump table — no vtable, no box-per-binding (the leafwing anti-pattern). One flat `bindings` arena + per-action `(start,len)` ranges = dense, cache-sequential iteration during `process_actions`, zero per-action allocation. Aggregation rules: buttons OR/max, axes sum→deadzone→clamp — a tiny branchy `match`. **Rejected:** `Vec<Box<dyn Buttonlike>>` per action (alloc + dyn); `HashMap<A, Vec<…>>` (hash). **Trade-off:** `BindSpec` is a closed set — user-defined custom input kinds are not pluggable. Acceptable for keyboard+mouse v1 (principle 10: fast core, convenience on top).

### Decision 8 — Clash resolution: strict-subset suppression, `PrioritizeLongest` default

During `process_actions`, after collecting active bindings: if binding A's key-set ⊂ binding B's key-set and both active, mark A's action `consumed`. Implemented by sorting the (tiny) active-chord candidate set by length descending, masking consumed keys, and suppressing subsets. Per-context override via `clash = longest | all` in the `.keys` file (V7).

**Why:** the one genuinely non-obvious correctness feature — `Ctrl+S` must suppress bare `S`. O(active²) over only *active* bindings (typically < 10) — trivial, off every hot loop, no alloc (works over the fixed arena + a stack `BitSet256` mask). `debug_assert!(active_chords < CLASH_LIMIT)` guards the pathological case.

### Action sets / contexts (V3)

A fixed array of named contexts; each context owns its own `InputMap<A>` ranges and `clash` override. Activation model: a small **priority stack** of active contexts with `consume` semantics (an Unreal-style layered approach), defaulting to single-active when the stack depth is 1. `process_actions` walks contexts top-of-stack first; a context may `consume` an input so lower contexts do not also see it. v1 ships the stack but a game may use it as single-active by never pushing.

---

## 7. ECS integration

### 7.1 Generic-resource `ResourceId` minting (resolves C1 — blocker)

`#[derive(Resource)]` emits `static ID: OnceLock<ResourceId>` **inside** the generic `resource_id()` body (verified at `boyko_macros/src/lib.rs:1915-1924`). Per rust#22991, a `static` in a generic fn is **not** monomorphized — every `A` shares one static. So `ActionState<GameplayAction>` and `ActionState<MenuAction>` would collapse onto the **same** `ResourceId`, reinterpreting bytes of the wrong type = UB / heap corruption. The headline "two independent resources for free" is false under the derive.

**Resolution:** mint the `ResourceId` for the *generic* resources (`ActionState<A>`, `InputMap<A>`) via the proven `TypeId`-keyed registry pattern the codebase already uses for `State<S>` — `state_resource_registry::resource_id_for<T>()` (verified at `boyko_ecs/src/ecs/core/state/state_resource_registry.rs:67`, a process-global `OnceLock<Mutex<HashMap<TypeId, ResourceId>>>` minting via `resource_registry::register_new`). `boyko_input` adds its own `action/resource_id.rs` mirroring that module verbatim (or, preferably, `boyko_ecs` promotes `resource_id_for` to a `pub(crate)`→`pub` shared helper — see §10), and hand-implements `Resource` for the generic types:

```rust
// NOT #[derive(Resource)] — the generic-body static would collapse all A.
impl<A: Actionlike> Resource for ActionState<A> {
    #[inline] fn resource_id() -> ResourceId { crate::action::resource_id::id_for::<Self>() }
}
impl<A: Actionlike> Resource for InputMap<A> {
    #[inline] fn resource_id() -> ResourceId { crate::action::resource_id::id_for::<Self>() }
}
```

The **non-generic** resources (`RawInputQueue`, `PhysicalInput`) keep `#[derive(Resource)]` — their `resource_id()` body is monomorphic, so the per-type static is sound. A regression guard test (modeled on `state_resource_ids_distinct_per_type`) asserts `ActionState::<A>::resource_id() != ActionState::<B>::resource_id()`.

### 7.2 The ingest system + schedule placement (resolves W2)

One system, `update_action_state`, holds `ResMut<RawInputQueue>`, `ResMut<PhysicalInput>`, and per action enum a `(ResMut<ActionState<A>>, Res<InputMap<A>>)` pair. It drains the queue, recomputes physical + action state, and freezes the fixed snapshot.

It is registered on **`CoreSchedule::Main`** (runs exactly once per frame), ordered **before** the gameplay set. **W2 fix:** `SystemConfig::before` takes a `SystemKey`, not a `SystemSet` (verified `system_config.rs:66`); the set-level API is `SystemConfig::before_set<S: SystemSet>` (verified `:123`). Correct registration:

```rust
app.add_systems_cfg(CoreSchedule::Main, |b| {
    b.add_system(update_action_state).before_set(GameplaySet)   // before_set, not before
});
```

Resource access bits use the verified two-arg form `add_resource_write(id, param_name)` / `add_resource_read(id, param_name)` (`filtered_access_set.rs:154`), so the parallel conflict graph colors the ingest system as a writer of `ActionState`/`PhysicalInput`/`RawInputQueue` and a reader of `InputMap` — it will not co-run with any system reading `ActionState`. Correct by the existing graph; no new sync.

### 7.3 The fixed-step determinism mechanism (resolves C3 — deepest blocker)

The critic correctly proved there is **no mid-loop substep index** readable by a Fixed-schedule system: `fixed_advance` (`fixed_loop.rs:51-89`) runs the `while … expend(ts) { step(world) }` loop opaquely, and `FixedTime::steps_this_frame` is written only **after** the loop (`:83`). The original Decision-7 latch — "an internal `fixed_substep_index` gates so substeps 2..N see no second `just_pressed`" — referenced a counter that does not exist and cannot be read between substeps. **Unbuildable as designed.** Two facts also confirmed: Fixed (④) runs **before** Main (⑤) in the frame (`app/app.rs:664-676`), so input ingested in Main is first visible to the *next* frame's fixed loop (the 1-frame-latency framing is correct; the *mechanism* was broken).

**Resolution — move edge-consumption entirely out of the per-substep path; make the fixed view frame-stable and self-consistent across 0..N substeps. No engine change to `FixedTime`/`fixed_advance` is required.**

Design:

1. **Edges accumulate from the event stream, not from a level diff (this also resolves W4).** During `update_action_state` (Main), for every drained `RawInputEvent::Key{state:Pressed,..}` set the corresponding bit in `keys_just_pressed`; for `Released` set `keys_just_released`; maintain `keys_pressed` as the running level. A key that goes **down and up within one frame** (a same-frame tap) therefore sets `just_pressed` (and `just_released`) even though end-of-frame `cur == prev`. The level-diff `cur & !prev` is **abandoned** as the edge source; it is retained only as a `debug_assert!` cross-check, never authoritative. There is a single authoritative edge source: the event stream.

2. **Main writes a frozen fixed snapshot once per frame.** After computing the per-frame action edges/levels/values, `update_action_state` copies them into `fixed_just_pressed` / `fixed_just_released` / `fixed_pressed` / `fixed_value` / `fixed_axis2`. These are **frame-stable**: every substep of the *next* frame's fixed loop reads identical bytes. There is no per-substep mutation, so no substep index is needed.

3. **Single-consume is owned by ingest, not by substeps.** "An action edge is consumed by the fixed sim" is defined as: the fixed snapshot's `fixed_just_pressed` is **live for exactly the frame whose Main produced it**, and is cleared by the *next* frame's `update_action_state` before it writes the new snapshot. Because every substep within one frame reads the same frozen bits, a 3-substep frame applies the press on all three substeps **only if the gameplay system is written to act on level state**; for **edge** semantics the convention is: a Fixed system reads `fixed_just_pressed` and the engine guarantees it is true on every substep of exactly one frame, then false. This is deterministic and miss-free:
   - **0-substep frame:** the edge persists in the snapshot (it was written by Main) and is consumed by the *first* substep of whatever later frame first runs the fixed loop — the press is never lost (the original "press straddling a 0-substep frame is invisible" bug is fixed, because the snapshot is sticky until the next Main overwrites it, and Main only overwrites after at least one frame boundary).
   - **N-substep frame:** all N substeps see the same `fixed_just_pressed`. A system wanting "fire once per press" reads `fixed_just_pressed` and is expected to act idempotently per frame (the standard fixed-step input contract); a system wanting "fire once per substep while held" reads `fixed_pressed`. Both are well-defined and documented.

   The footgun the plan must prevent — *the same OS press being counted by a variable number of substeps* — is prevented because the snapshot is frame-stable, not re-derived per substep, and the edge is cleared on a frame boundary (deterministic count of exactly one "edge-active frame" per physical press), independent of how many substeps that frame ran.

4. **Why this needs no `FixedTime` change:** the only state required is "the frozen snapshot, valid for one frame," and the one place that knows the frame boundary is `update_action_state` on Main (which runs exactly once per frame). It clears the previous frame's `fixed_*` edges and writes the new ones. The `step` closure never touches edge bookkeeping. The verified clock API (`FixedTime::overstep`, `delta`, `steps_this_frame`) is read-only here and sufficient.

5. **Deterministic 1-frame latency** (V10): because Fixed (④) precedes Main (⑤), the snapshot written by frame F's Main is consumed by frame F+1's fixed loop. Standard, imperceptible ≥60 Hz, and the price of a race-free, double-count-free contract.

**`consume()` interaction (resolves O3):** `ActionState::consume(a)` clears `just_pressed`/`just_released` bits for action `a` in the **Main-facing** sets and in the **frozen fixed sets** for the current frame, and sets the `consumed` bit. Because the fixed view is frozen until the next Main, a `consume` call from a Main system is observed identically by every substep of the next frame's fixed loop — no desync. `consume` from a Fixed system mutates only the frozen snapshot for the remaining substeps of the current frame (documented; rarely used).

### 7.4 `InputPlugin<A>`

```rust
pub struct InputPlugin<A: Actionlike> { default_map: InputMap<A>, keys_path: Option<&'static str> }

impl<A: Actionlike> Plugin for InputPlugin<A> {
    fn build(&self, app: &mut App) {
        // cold path: allocate all fixed buffers ONCE
        app.insert_resource(RawInputQueue::with_capacity(RAW_QUEUE_CAP));
        app.insert_resource(PhysicalInput::default());
        app.insert_resource(ActionState::<A>::with_count(A::COUNT));   // Box<[..]> alloc here
        app.insert_resource(self.default_map.clone_arena());           // load .keys override if present
        app.add_systems_cfg(CoreSchedule::Main, |b|
            b.add_system(update_action_state::<A>).before_set(GameplaySet));
    }
}
```

---

## 8. Public API (signatures, no impl)

```rust
// Building maps (cold path)
impl<A: Actionlike> InputMap<A> { pub fn builder() -> InputMapBuilder<A>; }
impl<A: Actionlike> InputMapBuilder<A> {
    pub fn bind(self, action: A, spec: BindSpec) -> Self;
    pub fn wasd(self, action: A) -> Self;            // preset Axis2 DigitalNormalized
    pub fn clash(self, s: ClashStrategy) -> Self;
    pub fn context(self, name: &'static str) -> Self;
    pub fn build(self) -> InputMap<A>;
}

// Consumption (hot path, in systems) — branchless bit tests
impl<A: Actionlike> ActionState<A> {
    pub fn pressed(&self, a: A) -> bool;
    pub fn just_pressed(&self, a: A) -> bool;
    pub fn just_released(&self, a: A) -> bool;
    pub fn value(&self, a: A) -> f32;                // Button/Axis1D, clamped
    pub fn axis2(&self, a: A) -> [f32; 2];           // Axis2D, deadzoned+clamped
    pub fn consume(&mut self, a: A);                 // mark handled this frame (O3 semantics)
}

// The seam
impl RawInputQueue { pub fn push_raw(&mut self, ev: RawInputEvent); }

// Persistence (cold path)
pub fn save_keys<A: Actionlike>(map: &InputMap<A>, out: &mut String);          // canonical, idempotent
pub fn load_keys<A: Actionlike>(src: &str, into: &mut InputMapBuilder<A>) -> ParseReport;

// Runtime rebind (cold path)
pub enum RebindOutcome { Bound, Conflict { existing: &'static str }, Cancelled }
pub struct RebindSession<A: Actionlike> { /* listening for next input */ }
impl<A: Actionlike> RebindSession<A> {
    pub fn begin(action: A, slot: u8) -> Self;
    pub fn feed(&mut self, ev: &RawInputEvent, map: &mut InputMap<A>) -> Option<RebindOutcome>;
}

// Adapters (feature-gated; only these name windowing types) — see §5.3
```

---

## 9. The `.keys` human-editable persistence format (resolves V2, O2)

### 9.1 Grammar (EBNF, one-pass hand parser — no serde/toml/ron)

```
file        = { line } ;
line        = ws , ( comment | header | version | binding | empty ) , ws , [ comment ] , newline ;
comment     = "#" , { any-char-except-newline } ;          (* cut at first UNQUOTED '#' *)
version     = "version" , ws , "=" , ws , integer ;
header      = "[" , ident , [ ws , "clash" , ws , "=" , ws , ("longest"|"all") ] , "]" ;
binding     = action-name , ws , "=" , ws , spec-list ;
action-name = ident ;
spec-list   = spec , { ws , "," , ws , spec } ;            (* comma split is PAREN-DEPTH AWARE *)
spec        = key | mouse | chord | composite | "none" ;
key         = keyname | "raw(" , integer , ")" ;           (* raw(N) = Unidentified(N) round-trip *)
mouse       = "Mouse" , integer | "MouseBack" | "MouseFwd" | "MouseOther(" , integer , ")" ;
chord       = keyname , { "+" , keyname } ;                (* sorted on serialize *)
composite   = "wasd"
            | "axis2(" , param-list , ")"
            | "axis1(" , param-list , ")"
            | "stick(" , param-list , ")" ;                (* stick: parsed, preserved, ignored v1 *)
param-list  = param , { "," , ws , param } ;
param       = ident , ws , "=" , ws , value ;
```

### 9.2 Example

```ini
# boyko-engine keybinds — "action = primary, secondary, ..."
version = 1

[gameplay clash=longest]
move      = wasd, axis2(up=W, down=S, left=A, right=D, dz=0.15, mode=radial)
jump      = Space
fire      = Mouse1
quicksave = LCtrl+S                       # PrioritizeLongest suppresses bare S
exotic    = raw(0x56)                      # Unidentified(0x56) round-trips losslessly
look      = stick(src=Pad:RStick, dz=0.10) # ignored in v1, preserved on round-trip

[menu clash=all]
confirm   = Enter
cancel    = Escape
```

### 9.3 Rules (resolves O2)

- **Line-based, one-pass parser.** Comment stripped at the first **unquoted** `#`; a `#` inside a quoted token (`"…"`) is literal — needed because `Other`/`raw` and future labels may carry arbitrary bytes.
- **Top-level comma split tracks paren depth** (one counter) so `axis2(up=W, down=S)` is not split mid-spec. Called out so the developer never naively `split(',')`.
- **Raw / unmapped key round-trip (O2):** `KeyCode::Unidentified(n)` serializes as `raw(0xNN)`; `MouseButton::Other(n)` as `MouseOther(n)`. This makes `parse∘serialize == identity` hold even for keys the canonical table does not name.
- **Override-delta semantics:** absent action line ⇒ code default; present line ⇒ full override of that action's slots; `= none` ⇒ explicitly unbound; omitted `[context]` ⇒ inherit all defaults.
- **Versioning:** `version = N` must appear first; an unknown **higher** version ⇒ best-effort load + warn, never hard-fail (a user config must not brick the game). A lower version loads with documented forward-compat defaults.
- **Recoverable per-line errors:** an unparseable line is skipped and recorded in `ParseReport` (line number + reason); parsing never aborts the whole file.
- **Canonical round-trip:** the serializer emits `version` first, contexts in declaration order, actions in declaration order, chord parts sorted, params in fixed order ⇒ `parse∘serialize` is byte-identical on canonical output. The engine header is re-emitted; user comments are dropped on rewrite (documented; matches every surveyed engine).
- **Why text, not `boyko_serialize`:** verified — `boyko_serialize` walks archetypes/component columns only (`save.rs`), serializes entity data, and has **no resource path**. Keybinds are a `Resource` and are user-editable/shareable. Binary codegen is the wrong tool, and no serde/toml/ron dependency is added (CLAUDE.md).

### 9.4 Runtime rebinding + conflict detection

`RebindSession::begin(action, slot)` enters listen mode; `feed(ev, map)` captures the next gameplay-relevant `RawInputEvent`, writes it into `map`'s slot, and returns `RebindOutcome::Conflict { existing }` if the captured input already binds another action in the same context (the conflict scan is O(active context bindings), cold). The application drives the session from its UI; the engine never owns rebind UI.

---

## 10. Integration: required changes to existing code

- **New crate** `crates/boyko_input/` (structure §4). Workspace `Cargo.toml` gains the member.
- **`boyko_macros`**: add `#[derive(Actionlike)]` + the `#[actionlike(Button|Axis1D|Axis2D)]` field attribute, emitting `COUNT` / `index` / `from_index` / `kind` / `name`. This is the only macro addition.
- **`boyko_ecs`** (small, optional-but-preferred): promote `state_resource_registry::resource_id_for<T>` to a shared `pub fn resource_id_for<T: Resource>()` (or expose an equivalent generic-resource minting helper) so `boyko_input` reuses the *one* proven non-collapsing path instead of duplicating the registry. If the owner prefers zero `boyko_ecs` change, `boyko_input` duplicates the ~40-line registry in `action/resource_id.rs` (documented duplication). **Recommended:** promote, single source of truth.
- **`boyko_rhi_vulkan/src/window.rs`** (the renderer-crate change; resolves W3 by staging): extend `window_proc` (currently close/destroy-only with no user pointer, verified `:284-307`) to:
  - **Stage 1 (I6):** capture `WM_KEYDOWN/UP`, `WM_SYSKEYDOWN/UP`, `WM_CHAR`, `WM_*BUTTON{DOWN,UP}`, `WM_MOUSEMOVE` (absolute cursor → `CursorMoved`), `WM_MOUSEWHEEL/HWHEEL`. Plumb a captured-event ring reachable from the proc via `SetWindowLongPtr(GWLP_USERDATA)` (not present today — must be added). Expose `Window::drain_input(&mut self, &mut impl FnMut(CapturedMsg))`. These use **already-in-use** Win32 message constants; the only new FFI is `SetWindowLongPtr`/`GetWindowLongPtr` (small, declared in `ffi.rs`, ABI-guarded).
  - **Stage 2 (I6b, deferred per W3):** `RegisterRawInputDevices` + `WM_INPUT` + `GetRawInputData` + the `RAWINPUT`/`RAWINPUTHEADER`/`RAWINPUTDEVICE` `#[repr(C)]` structs for un-accelerated relative mouse delta (`MouseMotion`). This is a **new FFI block**, each `extern` + struct hand-declared, `// SAFETY:`-commented, and registered in `abi_guard.rs`. It is explicitly NOT presented as a thin change. Until I6b lands, camera-relative delta is derived from successive `WM_MOUSEMOVE` deltas (functional, OS-accelerated) so v1 is not blocked.

    All of this lives in `boyko_rhi_vulkan` (raw-FFI, no new dependency), NOT in `boyko_input`.
- **Runner / `boyko_demo` glue** (edge, not core): after `pump_events()` (native) or per egui frame, call `boyko_input::{win32,egui_adapter}::translate` and `RawInputQueue::push_raw`. `boyko_demo`'s existing `update_input` → `InputState` pattern migrates to read `Res<ActionState>`.

---

## 11. Multithreading model

- `update_action_state` holds `ResMut<ActionState<A>>`, `ResMut<PhysicalInput>`, `ResMut<RawInputQueue>`, `Res<InputMap<A>>`. The access bits make the parallel scheduler treat it as a writer of those resources — it will not co-run with any reader of `ActionState`. Correct by the existing conflict graph; no new synchronization primitive.
- `push_raw` is called from the **runner thread** (the OS message-pump thread) **before** `App::update_with_delta` runs — outside the scheduler's parallel window. No system touches `RawInputQueue` concurrently with the pump. v1 is pump-then-update, serial: no atomics needed.
- All `boyko_input` resources are POD → `Send + Sync`. No `!Send` handle (the OS window handle stays in `boyko_rhi_vulkan`, never crosses into `boyko_input`). The Arena's `!Send`/`!Sync` invariant is untouched.
- **Future threaded pump:** the ring is laid out for SPSC (single-producer pump, single-consumer ingest); promoting `head`/`tail` to atomics with `Acquire`/`Release` is a localized change, deferred.
- **Data-race freedom:** each input resource is written by exactly one system and read after; the producer (pump) and consumer (ingest) never overlap in time.

---

## 12. Algorithms for critical paths

**`update_action_state` (per frame, Main, early):**
1. **Clear** the previous frame's edge sets (`keys_just_pressed/released`, action `just_*`, `consumed`) and the previous `fixed_*` edges (C3 single-consume boundary).
2. **Drain `RawInputQueue`** accumulating mouse delta/wheel, updating `keys_pressed` level, and setting `just_pressed`/`just_released` bits **from the event stream** (W4 — same-frame taps survive).
3. **For each `InputMap` context** (top of priority stack first): iterate its flat `bindings` arena sequentially; aggregate active bindings into `ActionState` (buttons OR/max; axes sum→deadzone→clamp). Sequential cache access; `match` jump table; no alloc.
4. **Clash pass** (Decision 8) over the small active set.
5. **Freeze the fixed snapshot** (copy edges/levels/values into `fixed_*`) — frame-stable for the next frame's fixed loop (C3).

Complexity: O(raw_events + Σ context_bindings + active²). Cache: sequential over bitset words and the bindings arena (streaming). Branching: minimal; the central dispatch is a jump table. SIMD: the `BitSet256` edge/level word ops (4 `u64`s) are the natural vectorization target.

**Consumption** (`just_pressed` etc.): one `BitSet256` bit test → O(1), branchless, one cache line.

---

## 13. Deferred seams (explicit)

- Gamepad / `BindSpec::Stick` / `stick(...)` grammar — reserved, parsed, round-tripped, runtime-ignored.
- Touch input — new `RawInputEvent` variants behind a future feature.
- Recording / replay — tap `RawInputQueue` drain (deterministic event log).
- Non-US keycap labels — `key_label` returns US default; locale labels are app-supplied.
- `generic_const_exprs` array sizing — replaces the `Box<[f32]>` setup alloc.
- Threaded SPSC pump — atomic `head`/`tail`.
- IME / `WM_DEADCHAR` composition — beyond basic `Text(char)`.
- `WM_INPUT` raw delta — Stage 2 (§10, I6b).
- `COUNT > 256` action enums — `BitSetN`-generic fallback.

---

## 14. Implementation roadmap with per-phase test gates

**I1 — Raw layer + crate skeleton.** `KeyCode`/`MouseButton`/`ScrollDelta`/`ButtonState`/`RawInputEvent`/`RawInputQueue`/`PhysicalInput`; static scancode→`KeyCode` tables.
*Gate:* enum `repr` stability (`size_of`/discriminant) tests; scancode-table coverage; ring wrap + drop-oldest overflow (`high_water`/`dropped`) tests.

**I2 — `Actionlike` derive.** `boyko_macros` derive emitting `COUNT`/`index`/`from_index`/`kind`/`name`; `ActionKind`.
*Gate:* derive on Button/Axis enums; `index` density 0..COUNT; `from_index` round-trip; compile-fail on non-enum / on `COUNT > 256` (a `const` assert).

**I3 — `InputMap<A>` + `ActionState<A>` + `process_actions`.** Flat bindings arena, SoA action state (`BitSet256`), aggregation rules, clash resolution.
*Gate:* button OR/max; axis sum/deadzone/clamp; WASD diagonal normalization; `PrioritizeLongest` (Ctrl+S suppresses S); no-per-frame-alloc assertion (counting allocator in test); event-stream same-frame-tap fidelity (W4) test.

**I4 — ECS integration + `InputPlugin` + C1/C3.** Generic-resource `ResourceId` minting via the `TypeId` registry; `update_action_state`; register on `CoreSchedule::Main` `.before_set(GameplaySet)`; the frame-stable fixed snapshot (C3).
*Gate:* **C1 regression guard** — `ActionState::<A>::resource_id() != ActionState::<B>::resource_id()` (mirrors `state_resource_ids_distinct_per_type`); `Res<ActionState>` read in a system; **C3 edge-correctness matrix** — assert exactly-one edge-active frame per physical press across 0-substep, 1-substep, and 3-substep fixed frames (no miss, no double-count), driving `Time`/`FixedTime` directly via `fixed_advance` with a counting closure.

**I5 — Persistence (`.keys`).** Hand parser/serializer; paren-depth comma split; `raw(N)`/`MouseOther(N)` round-trip; versioning; override-delta; recoverable per-line errors.
*Gate:* `parse∘serialize` byte-identical (incl. `Unidentified`/`Other`); unknown-token recovery (`ParseReport`); version up/down behavior; chord/axis/stick parse; `#`-inside-quotes handling. Property test: `parse∘serialize == identity` on canonical output.

**I6 — Runtime rebind + Win32 source (Stage 1).** `RebindSession`; extend `window_proc` for keyboard/mouse-button/`WM_MOUSEMOVE`/wheel + `GWLP_USERDATA` ring + `Window::drain_input`; `win32::translate`.
*Gate:* rebind binds/conflicts/cancels; conflict detection within a context; manual/integration native keypress→action; `unsafe` scancode-indexing + ring under Miri; `GWLP_USERDATA` plumbing carries `// SAFETY:` + ABI-guard.

**I6b — `WM_INPUT` raw delta (deferred FFI block).** `RegisterRawInputDevices` + `WM_INPUT` + `RAWINPUT` structs + ABI guards; `MouseMotion` switches from `WM_MOUSEMOVE`-derived to raw.
*Gate:* ABI-guard tests for the new `#[repr(C)]` structs; relative-delta correctness vs cursor-derived baseline.

**I7 — egui adapter + demo migration.** `egui_adapter::translate`; migrate `boyko_demo` to `Res<ActionState>`.
*Gate:* egui event→action mapping; demo smoke; both adapters produce identical `ActionState` from equivalent input streams.

---

## 15. Metrics and validation

- **Benchmarks (criterion):** `process_actions` 30 actions/60 bindings (target < 1 µs); `just_pressed` query (target ~1 ns, one bit test); `BitSet256` edge recompute (4-word scan).
- **Unit tests:** all aggregation/clash/deadzone cases; round-trip persistence; rebind state machine; the C3 fixed/variable edge matrix (0,1,3 substeps); the C1 resource-id distinctness guard.
- **Property tests:** `parse∘serialize == identity` on canonical output; random `RawInputEvent` streams never panic and never allocate on ingest.
- **`debug_assert!` invariants:** `A::COUNT <= 256` (BitSet256 cap); `RawInputQueue.dropped == 0` (overflow tuning); `active_chords < CLASH_LIMIT`; `index() < COUNT`; per-action `(start,len)` ranges within the bindings arena.
- **Miri:** the Win32 adapter's `unsafe` scancode-table indexing and the ring buffer (the only `unsafe` in core). The renderer-side `GWLP_USERDATA` + `WM_INPUT` FFI is `unsafe` with `// SAFETY:` + ABI-guard and is Miri-N/A (FFI).

---

## 16. Changelog vs critic round 1

| Item | Resolution |
|------|-----------|
| **C1** (generic `#[derive(Resource)]` collapses all `A` onto one `ResourceId`) | **Fixed.** `ActionState<A>`/`InputMap<A>` hand-implement `Resource` via the `TypeId`-keyed registry (`state_resource_registry::resource_id_for`, verified `:67`), not the derive. Non-generic `RawInputQueue`/`PhysicalInput` keep the derive. Added a distinctness regression-guard test. §7.1. |
| **C2** (`BitSet512` does not exist) | **Fixed.** Re-based on `BitSet256` (verified the only fixed set in `boyko_utils`); action cap = 256 (V8); cache-line math and perf budget re-derived; `debug_assert!(A::COUNT <= 256)`. §5.5, Decision 5. |
| **C3** (fixed latch relied on a non-existent mid-loop substep index) | **Fixed.** Mechanism redesigned to a **frame-stable frozen snapshot** written/cleared by `update_action_state` on Main (the one per-frame boundary); edge single-consume owned by ingest, not substeps; **no `FixedTime`/`fixed_advance` change required** (verified `:51-89`, `:83`). §7.3. |
| **W1** (wrong-file `app.rs` anchors) | **Fixed.** All anchors disambiguated to `app/app.rs`; frame order verified at `app/app.rs:664-676`. |
| **W2** (`.before(GameplaySet)` wrong API) | **Fixed.** Uses `SystemConfig::before_set<S: SystemSet>` (verified `:123`); `before` takes `SystemKey` (`:66`). Access bits use the two-arg `add_resource_*(id, param_name)` (`:154`). §7.2. |
| **W3** (`WM_INPUT` FFI presented as thin) | **Fixed.** Staged: I6 ships `WM_MOUSEMOVE`-derived delta with only `SetWindowLongPtr` new FFI; I6b adds the `RAWINPUT` block as an explicit new ABI-guarded FFI phase. §10. |
| **W4** (same-frame tap fidelity unproven; level-diff vs event-overlay) | **Fixed.** Single authoritative edge source = the **event stream**; level-diff `cur & !prev` demoted to a debug cross-check. Same-frame down+up sets `just_pressed`. §7.3 step 1, §12. |
| **O1** (ring overflow policy) | **Resolved.** Drop-oldest + `dropped`/`high_water` counters + `debug_assert!`. §5.4. |
| **O2** (`#` in quotes; raw-key round-trip) | **Resolved.** Unquoted-`#` rule; `raw(N)`/`MouseOther(N)` round-trip forms. §9. |
| **O3** (`consume()` vs fixed view) | **Resolved.** `consume` clears Main-facing + frozen fixed edges for the frame; frozen view keeps substeps consistent. §7.3. |

---

## 17. Open questions for the owner

1. **`boyko_ecs` change (§10):** promote `resource_id_for<T>` to a shared `pub` helper (single source of truth, ~5-line `boyko_ecs` diff) vs duplicate the ~40-line registry inside `boyko_input` (zero `boyko_ecs` change). Recommended: promote.
2. **C3 contract surface:** confirm the documented fixed-step input contract — *edge actions (`fixed_just_pressed`) are guaranteed true on every substep of exactly one frame per physical press; per-substep "while held" reads `fixed_pressed`*. This is the determinism guarantee; if the owner wants strict "exactly one substep sees the edge," that requires the `FixedTime` substep-index engine change (scoped but not in v1) — flagged.
3. **Action cap 256 (V8):** confirm 256 is ample, or request the `BitSetN`-generic fallback in v1.
4. **Context model (V3):** ship the full priority-stack-with-consume, or cut to single-active for v1.

---

Relevant files verified during this revision (all absolute):
- `D:\claude\BoykoEngine\crates\boyko_macros\src\lib.rs:1906-1930` (the collapsing generic-body `static` — C1)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\state\state_resource_registry.rs:67` (`resource_id_for<T>` — C1 fix precedent)
- `D:\claude\BoykoEngine\crates\boyko_utils\src\bit_mask\bit_set_256.rs:21` and `bit_set.rs:80` (only fixed set is `BitSet256`; no `BitSet512` — C2)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\time\fixed_loop.rs:51-89` and `fixed_time.rs:169,204` (opaque loop; `steps_this_frame` written after loop; no mid-loop index — C3)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\schedule\system_config.rs:66,123` (`before` takes `SystemKey`; `before_set<S: SystemSet>` — W2)
- `D:\claude\BoykoEngine\crates\boyko_ecs\src\ecs\core\app\app.rs:664-676` (Fixed before Main — frame order, W1)
- `D:\claude\BoykoEngine\crates\boyko_rhi_vulkan\src\window.rs:222-307` (`pump_events` peek loop; `window_proc` close/destroy-only, no user pointer — W3)