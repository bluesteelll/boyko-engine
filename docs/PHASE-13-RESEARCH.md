# Phase 13 Research — `Local<T>` SystemParam

Researcher's input to the architect. Sources: Bevy `system_param.rs`
(main), docs.rs, Bevy Cheat Book, flecs/EnTT/Unity docs, and boyko's
own `core/system/` tree. All file:line citations verified against the
current `ecs` branch.

## TL;DR for the architect

- `Local<T>` is the **simplest possible SystemParam** — structurally a
  twin of boyko's existing `EventReader<'s, E>`: per-system private
  state, `'s`-only lifetime, **zero declared access**, no `dyn`.
- `type State = T` directly (the `T` storage lives in
  `FunctionSystem::state` inside the param tuple). No `Box<dyn Any>`,
  no `TypeId` keying — principle #1 satisfied.
- **Multiple-same-`T` distinctness falls out for free** from boyko's
  existing tuple impl: `type State = ($($p::State,)*)`
  (`tuple_impl.rs:91`) gives each positional param its own slot. Two
  `Local<u32>` → two independent `T` slots. No design mechanism needed,
  just a test.
- **One shape divergence to internalize:** boyko's `get_param` returns
  `Self::Item<'w,'s>` directly (NO `Result`, NO `change_tick`), unlike
  current Bevy. Follow boyko's existing signature
  (`system_param.rs:149`), not Bevy's docs verbatim.
- **The one hard decision:** boyko's `SystemParam::State: Send + Sync`
  bound is stricter than Bevy's `Send`-only (Bevy launders `Sync` via
  `SyncCell`). See Decision A.

## 1. Bevy `Local<T>` reference

```rust
#[derive(Debug)]
pub struct Local<'s, T: FromWorld + Send + 'static>(pub(crate) &'s mut T);

unsafe impl<'a, T: FromWorld + Send + 'static> SystemParam for Local<'a, T> {
    type State = SyncCell<T>;
    type Item<'w, 's> = Local<'s, T>;

    fn init_state(world: &mut World) -> Self::State {
        SyncCell::new(T::from_world(world))
    }
    fn init_access(_state, _meta, _access_set, _world) { }   // empty — no access

    #[inline]
    unsafe fn get_param<'w, 's>(state, _meta, _world, _change_tick)
        -> Result<Self::Item<'w, 's>, SystemParamValidationError>
    { Ok(Local(state.get())) }
}
impl Deref/DerefMut for Local -> T
```

- `'s` = the **state scope** — lifetime of the borrow into the system's
  stored state slot. `Local` is just `&'s mut T`.
- `T: FromWorld + Send + 'static`. **`Sync` NOT required on `T`** —
  `SyncCell<T>` fabricates `Sync` from `Send` because it only ever
  hands out `&mut T` (`get(&mut self) -> &mut T`), never `&T`.
- `init_state` builds the value once via `T::from_world(world)`.
- `init_access` is **empty** — `Local` declares no component/resource
  access, does NOT participate in the conflict graph. Quote: "A local
  may only be accessed by the system itself and is not visible to
  other systems."
- `get_param` just rebinds `Local(state.get())` → `&'s mut T`. Zero
  world access.

### Distinctness (verified)

Official docs verbatim: "If multiple `SystemParam`s within the same
system each specify the same local type each will get their own
distinct data storage." Mechanism: `Local` is **positional, not
type-keyed** — each `Local<T>` is a distinct tuple element, so
`(Local<u32>, Local<u32>)::State = (SyncCell<u32>, SyncCell<u32>)`.
The tuple position IS the key. No `TypeId` map.

### Init value: `FromWorld` with a `Default` blanket

`impl<T: Default> FromWorld for T { fn from_world(_) -> Self { T::default() } }`
— so any `T: Default` satisfies `FromWorld`. `FromWorld` buys
`&mut World` access during init (initial value from a resource/asset).

## 2. Other engines (brief)

- **flecs:** no `Local<T>` analogue — per-system scratch via
  `ecs_system_desc_t.ctx` pointer or singleton component.
- **EnTT:** no system abstraction at all; per-system state is whatever
  the user captures in their lambda.
- **Unity DOTS:** nearest analogue is `SystemState` fields (named
  struct fields on the system, not an injected positional param).

**Verdict:** Bevy is the sole reference with a true positional
`Local<T>`. No competing design to weigh.

## 3. boyko integration point (exact shapes)

### `SystemParam` trait (`system_param.rs:80-171`)

```rust
pub unsafe trait SystemParam: Sized {
    type State: Send + Sync + 'static;                            // 85
    type Item<'w, 's>: SystemParam<State = Self::State>;          // 94
    fn init_state(world: &mut EcsMaster, system_meta: &mut SystemMeta) -> Self::State;  // 109
    fn init_access(state, system_meta, access_set, world);       // 125 — REQUIRED, no default body
    unsafe fn get_param<'w, 's>(state: &'s mut Self::State,
        system_meta: &SystemMeta, world: UnsafeEcsCell<'w>) -> Self::Item<'w, 's>;  // 149 — direct return
    fn apply(...) {}                                              // 160 defaulted
    fn new_archetype(...) {}                                      // 165 defaulted
}
```

### Divergences from Bevy the architect MUST internalize

| Aspect | Bevy (main) | boyko |
|---|---|---|
| `init_state` args | `(&mut World)` | `(&mut EcsMaster, &mut SystemMeta)` |
| `init_access` | default empty body | **required, no default** — write explicit empty body |
| `get_param` return | `Result<Item, ValidationError>` | `Self::Item<'w,'s>` **directly** |
| `get_param` extra args | `change_tick: Tick` | **none** |
| `State` bound | `Send + 'static` (Sync via SyncCell) | **`Send + Sync + 'static`** (stricter) |

The doc comment at `system_param.rs:121-122` claims `init_access` has a
default empty body — INACCURATE; the method at line 125 has no default.
Every existing param writes an explicit empty body.

### Templates

- **`Commands` (`commands.rs:309-363`)** — `type State = CommandQueue`
  (state IS storage, no dyn); empty `init_access` (319-329); `get_param`
  rebinds `&'s mut State`.
- **`EventReader` (`event_reader.rs:312-353`)** — the **exact
  structural twin**: `type State = EventReaderState<E>`; `Item<'w,'s>
  = EventReader<'s, E>` (drops `'w`); empty `init_access` (336-343);
  `get_param` → `EventReader { state }` pure rebind, no world access;
  wrapper holds `state: &'s mut EventReaderState<E>` (87-89);
  `unsafe impl Send/Sync` on the state (70-71).
- **`Res` (`res.rs:85-139`)** — access-DECLARING template (what
  `Local` skips): calls `add_resource_read` in `init_access`.

### Per-system state storage (`function_system.rs`)

- `FunctionSystem<F, Marker>` holds
  `pub(crate) state: Option<<F::Param as SystemParam>::State>`
  (line 116) — single owner of all per-param state, persisted across
  frames.
- `initialize` (183-245): idempotent; `init_state` (222) →
  `self.state` (231); then `init_access` (224-229).
- `run_unsafe` (247-268): `state = self.state.as_mut()` (249-252);
  `get_param(state, &self.meta, world)` (263-265); `&mut` for the whole
  run — exactly the `&'s mut` `Local` needs.
- Tuples: `state = ($($p::State,)*)`, per-element destructure
  `let ($($p,)*) = state;` (`tuple_impl.rs:125`). **Distinctness
  mechanism.**

### Prior art: EventReader's hand-rolled cursor

Phase 12's `EventReader` stores its cross-frame cursor
(`last_event_count: u64`, `event_reader.rs:56`) in `EventReaderState<E>`
— exactly what Bevy implements ON TOP of `Local`. boyko hand-rolled it.
Proof the pattern works; raises the "retrofit?" open question (§5 #4).

### Send/Sync mechanics

- `State: Send + Sync + 'static` hard-required (`system_param.rs:85`).
- boyko has **no `SyncCell` and no `FromWorld`** (grep-confirmed).
- Existing states satisfy `Send + Sync` trivially (`ResState: Copy`) or
  via explicit `unsafe impl` (`EventReaderState`, line 70-71).

## 4. Decision space for the architect

### Decision A — `State` type + the `Send + Sync` problem (the hard call)

- **A1 — `type State = T`, `T: Send + Sync + Default`.** Simplest, zero
  new types, zero unsafe. Stricter than Bevy (which allows `Send`-only)
  but the Phase 13 use cases (counters, accumulators, scratch `Vec`)
  are `Send + Sync` anyway. **Lowest complexity, fully sound.**
- **A2 — port a `SyncCell<T>` newtype**, `type State = SyncCell<T>`,
  `T: Send + 'static`. Matches Bevy exactly; allows
  `Send`-but-not-`Sync` `T`. Costs one wrapper with
  `unsafe impl<T: Send> Sync for SyncCell<T>` (sound — only hands out
  `&mut T`; the `&'s mut State` borrow guarantees no aliasing). This is
  the `EventReaderState` pattern generalized.

A1 is the minimal sound choice; A2 is full Bevy parity / future-proof.
No third sound option.

### Decision B — init value

- **B1 — `T: Default` only.** `init_state` → `T::default()`. Zero new
  traits. Covers the entire Phase 13 brief. **Recommended minimal.**
- **B2 — introduce `FromWorld`.** New trait + `Default` blanket; buys
  init-from-resources but creates coherence friction (bevy #4265).
  **Defer** unless a concrete world-aware-init consumer is named.
  Widening B1→B2 later is backward-compatible (the `Default` blanket
  keeps existing `Local<T: Default>` working).

### Decision C — distinctness

**No work.** Falls out of `tuple_impl.rs:91` + `:125`. Add a test only.

### Decision D — scheduler / `init_access`

**Empty body** (required method). Registers nothing in
`FilteredAccessSet`/`Access` → no conflict-graph edge. `Access`
(`access.rs:45-101`) has only component/resource bitmasks — no field a
`Local` could touch. "No access" is natural. Matches `Commands` /
`EventReader`.

### Decision E — lifetimes

`type Item<'w, 's> = Local<'s, T>` — use `'s` only, drop `'w`
(identical to `EventReader`). `get_param` rebinds `state: &'s mut State`
into `Local(&'s mut T)`. No world borrow.

### Decision F — Send/Sync on the system

`FunctionSystem` inherits `Send + Sync` from the param `State`
(`function_system.rs:100-107`). A1 satisfies directly; A2 via the
`unsafe impl`.

### `get_param` hot-path note

For `Local`: a single pointer rebind, no alloc/dyn/atomic/world-touch.
Principle #1 satisfied. `init_state` (cold) runs once in `initialize`.

## 5. Risks / open questions

1. **Send+Sync vs Bevy's Send-only (Decision A)** — pick A1 (stricter,
   minimal) or A2 (SyncCell, full parity). No third sound option.
2. **FromWorld scope (Decision B)** — `Default` (B1) covers the brief;
   `FromWorld` (B2) adds a public trait + coherence friction. Defer
   unless a concrete consumer is named. B1→B2 is backward-compatible.
3. **`get_param` signature skew (MUST resolve)** — write `Local`
   against boyko's `get_param(...) -> Item` (no Result, no tick), NOT
   Bevy's `Result<Item, _>` + `change_tick`. Don't be misled by Bevy
   docs.
4. **Refactor `EventReader` onto `Local`?** Bevy does
   (`EventReader = Local<EventCursor<E>>` internally). boyko hand-rolled
   `EventReaderState<E>` with `#[repr(C)]` cache-line layout + a 24 B
   size-pin assert (`event_reader.rs:401-408`). **Recommendation: leave
   it alone for Phase 13** — retrofitting working tested Phase 12 code
   with false-sharing layout commitments is out of scope. Note as prior
   art only.
5. **`init_access` doc inaccuracy (cosmetic)** — `system_param.rs:121-122`
   wrongly claims a default body. Architect may flag fixing the comment;
   doesn't affect the impl.
6. **`#[repr(transparent)]` on the wrapper** — applicable and free
   (`Local` is a single `&'s mut T`). `Res` uses it; `EventReader`/
   `Commands` don't. Minor.
7. **`Debug` derive** — Bevy derives it. If wanted, put `T: Debug` on a
   separate `impl`, not the struct. Minor.

## Sources

External: Bevy `system_param.rs` (main, GitHub), docs.rs `Local` /
`FromWorld` / `SystemParam`, Bevy Cheat Book "Local Resources", bevy
issues #4265 + #14860. flecs docs, EnTT wiki, Unity Entities systems
manual.

Internal (file:line): `system_param.rs:80-171`, `commands.rs:309-363`,
`event_reader.rs:51-71,87-89,312-353`, `res.rs:85-139`,
`tuple_impl.rs:90-160`, `function_system.rs:111-119,183-280`,
`access.rs:45-101`.
