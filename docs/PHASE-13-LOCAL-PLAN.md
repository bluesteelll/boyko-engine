# Architecture: Phase 13 — `Local<T>` SystemParam

---

## §0 Readiness & framing

This is the smallest SystemParam in the engine: one wrapper struct (`&'s mut T`),
one trait impl, two wiring lines, and a test suite. It is structurally a twin of
the already-shipped `EventReader<'s, E>` (Phase 12) minus the cached buffer
pointer. No hot-path impact, no new dependency, no `unsafe` block (only an
`unsafe impl SystemParam` whose contract is vacuous — `Local` declares zero access
and `get_param` is a pure borrow rebind).

The load-bearing artifact is the `Local<T>` source in §3, written against boyko's
**actual** `SystemParam` trait — NOT Bevy's docs. The two shapes diverge in four
places (no `Result`, no `change_tick`, `init_access` has no default body,
`init_state` takes `(&mut EcsMaster, &mut SystemMeta)`); the source below is
already adjusted. The developer should be able to near-copy it.

---

## §1 Goal + scope

Add `Local<'s, T>` — a per-system private state slot injected as a positional
SystemParam. Each `Local<T>` in a system's parameter list gets its own `T`,
default-initialized once at `initialize` and persisted across every run of that
cached system (frame-to-frame for scheduled systems). It declares no
component/resource access, so it adds no conflict-graph edge and never blocks
parallelism. Use cases: per-system counters, accumulators, frame-local scratch
buffers, "first run" flags, and the hand-rolled cursor pattern that Phase 12's
`EventReader` already implements ad hoc.

**Explicitly out of scope** (per research §5 #4): do NOT retrofit `EventReader`
onto `Local`. Bevy implements `EventReader = Local<EventCursor<E>>` internally;
boyko hand-rolled `EventReaderState<E>` with a `#[repr(C)]` cache-line layout and a
24 B size-pin (`event_reader.rs:50-71, 401-408`). Rewriting working, tested,
Miri-clean Phase 12 code onto a generic `Local` carries false-sharing-layout risk
for zero functional gain. `Local` is purely additive. Also out of scope: `FromWorld`
(Decision B), any `SyncCell` newtype (Decision A), and a `prelude` re-export (the
engine has none — see §4).

---

## §2 Decisions (A / B / F)

| ID | Decision | Choice | Justification |
|----|----------|--------|---------------|
| **A** | `State` type + Send/Sync bound | **A1 — `type State = T`, `T: Send + Sync + Default`.** Zero new types, zero `unsafe impl`. | boyko's `SystemParam::State: Send + Sync + 'static` is hard-required (`system_param.rs:85`) to migrate systems across Phase 9 workers. A1 satisfies it directly with `T: Send + Sync`. The entire Phase 13 use-case set (counters, `Vec` scratch, flags) is `Send + Sync` already. A2 (`SyncCell<T>`, `T: Send`-only) buys parity with Bevy's looser bound but costs a wrapper + an `unsafe impl Sync` + the cognitive load of a laundering type — pure complexity for a `Send`-but-`!Sync` `T` that no Phase 13 consumer needs. CLAUDE.md principle #1 and the lightweight-phase framing both favor A1. Widening to A2 later is backward-compatible: it only relaxes a bound (see §2.1). |
| **B** | Init value | **B1 — `T: Default`.** `init_state` returns `T::default()`. | Covers the whole brief; introduces no new trait. B2 (`FromWorld`) buys init-from-a-resource but adds a public trait + the coherence friction Bevy hit (issue #4265). No Phase 13 consumer needs world-aware init. Widening B1→B2 later is backward-compatible **provided** B2 ships the `impl<T: Default> FromWorld for T` blanket (see §2.2). |
| **F1** | `#[repr(transparent)]` on the wrapper | **Yes.** | `Local<'s, T>` is exactly one field (`&'s mut T`), so `transparent` is free and correct. Mirrors `Res`/`ResMut` (`res.rs:39`). Guarantees `size_of::<Local<T>>() == size_of::<&mut T>() == 8` (pin-asserted in §3). |
| **F2** | `Debug` impl | **Yes, on a separate `impl` block gated `T: Debug`** — NOT a `derive`. | A `#[derive(Debug)]` would put `T: Debug` on the struct definition, infecting every `Local<T>`. A standalone `impl<T: Debug> Debug for Local<'_, T>` gives `Debug` only where `T: Debug`, leaving `Local<NonDebugType>` usable. |

### §2.1 Why A1→A2 is backward-compatible

If a future phase needs `Send`-but-`!Sync` locals, swap `type State = T` for
`type State = SyncCell<T>` and relax the wrapper bound from `T: Send + Sync` to
`T: Send`. Relaxing a trait bound is non-breaking: every call site that satisfied
`T: Send + Sync` still satisfies `T: Send`. The only churn is internal
(`init_state` wraps in `SyncCell::new`, `get_param` calls `.get()`). A2's
`SyncCell<T>` would be a `#[repr(transparent)]` newtype over `T` with
`unsafe impl<T: Send> Sync for SyncCell<T>` justified by `fn get(&mut self) -> &mut T`
being the *only* accessor — it never hands out `&T`, and the `&'s mut` borrow of
the state slot enforces exclusivity. (Documented for completeness; NOT implemented
in Phase 13.)

### §2.2 Why B1→B2 is backward-compatible

B2 must ship `impl<T: Default> FromWorld for T { fn from_world(_) -> Self { T::default() } }`.
With that blanket, the `Local` bound changes from `T: Default` to `T: FromWorld`,
but every `T: Default` automatically satisfies `T: FromWorld` via the blanket — so
existing `Local<u32>`, `Local<Vec<_>>` keep working. The only new capability is a
`T` with a *manual* `FromWorld` (and no `Default`). This is Bevy's migration path.

---

## §3 The exact `Local<T>` source (load-bearing)

New file: `crates/boyko_ecs/src/ecs/core/system/params/local.rs`.

Written against boyko's real trait (`system_param.rs:80-171`): `get_param` returns
`Self::Item<'w, 's>` directly (no `Result`, no `change_tick`); `init_state` takes
`(&mut EcsMaster, &mut SystemMeta)`; `init_access` is a REQUIRED method with an
explicit empty body.

```rust
//! `Local<'s, T>` — per-system private state `SystemParam` (Phase 13).
//!
//! `Local<T>` injects a system-private `T` that is default-initialized once
//! (at `FunctionSystem::initialize`) and persisted across every run of the
//! cached system — frame-to-frame under the Phase 9 scheduler. It is the
//! simplest SystemParam in the engine: structurally a twin of
//! [`EventReader<'s, E>`](super::event_reader::EventReader) minus the cached
//! buffer pointer. It declares ZERO access, so it adds no conflict-graph edge
//! and never blocks parallel system execution.
//!
//! See Phase 13 plan §2 (Decisions A1/B1/F1/F2), §3 (this source), §7 (SAFETY).
//!
//! # Distinctness (no design mechanism — falls out of tuples)
//!
//! Two `Local<u32>` in one system get two independent `u32` slots, because
//! `Local` is a *positional* param: the system's `Param` is the tuple of its
//! arguments, and `(Local<u32>, Local<u32>)::State = (u32, u32)` (see
//! `tuple_impl.rs:91` + `:125`). The tuple position is the key; there is no
//! `TypeId` map. Verified by test, not by code (Phase 13 §6 test 2).

#![allow(dead_code)]

use std::fmt;
use std::ops::{Deref, DerefMut};

use crate::ecs::core::ecs_master::ecs_master::EcsMaster;
use crate::ecs::core::system::filtered_access_set::FilteredAccessSet;
use crate::ecs::core::system::system_meta::SystemMeta;
use crate::ecs::core::system::system_param::SystemParam;
use crate::ecs::core::system::unsafe_ecs_cell::UnsafeEcsCell;

/// A system-private value of type `T`, persisted across runs.
///
/// `Local<T>` borrows the `T` cached in the system's state slot for the
/// invocation scope `'s`. The value is `T::default()`-initialized once when the
/// system is first initialized, and survives across frames. `Deref` /
/// `DerefMut` make the wrapper transparent to system bodies.
///
/// # Distinctness
///
/// Multiple `Local<T>` of the same `T` in one system each receive their own
/// independent storage (positional, not type-keyed — see module docs).
///
/// # Lifetime
///
/// `'s` is the state scope. The `'w` world-access lifetime is unused (dropped at
/// the `Item<'w, 's>` projection — same as [`EventReader`]). `Local` performs no
/// world access whatsoever.
///
/// # Bounds (Phase 13 Decision A1 + B1)
///
/// `T: Send + Sync + Default + 'static`. `Send + Sync` is required by
/// [`SystemParam::State`] (the containing system must migrate across Phase 9
/// workers). `Default` supplies the one-time initial value.
///
/// [`EventReader`]: super::event_reader::EventReader
#[repr(transparent)]
pub struct Local<'s, T: Send + Sync + Default + 'static>(pub(crate) &'s mut T);

impl<T: Send + Sync + Default + 'static> Deref for Local<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.0
    }
}

impl<T: Send + Sync + Default + 'static> DerefMut for Local<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        self.0
    }
}

// F2: conditional `Debug` — gated on `T: Debug` via a standalone impl so the
// struct definition does not force `T: Debug` onto every `Local<T>`.
impl<T: Send + Sync + Default + fmt::Debug + 'static> fmt::Debug for Local<'_, T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Local").field(&self.0).finish()
    }
}

// SAFETY (Phase 13 §7 — SP1, SP2, SP4):
//   - SP1: `init_access` declares NO component / resource access. A `Local`
//     touches only the system's own private state slot, owned solely by this
//     system (`FunctionSystem::state`). `Access` has only component / resource
//     bitmasks — no field a `Local` could register, so "no access" is complete.
//   - SP2: `get_param` performs a pure borrow rebind of the `&'s mut Self::State`
//     handed in by the caller — no `world` touch, no aliasing minted.
//   - SP4: `init_state` constructs `T::default()` — no archetype / resource
//     registry mutation (debug-asserted by `FunctionSystem::initialize`).
unsafe impl<T: Send + Sync + Default + 'static> SystemParam for Local<'_, T> {
    type State = T;
    type Item<'w, 's> = Local<'s, T>;

    #[inline]
    fn init_state(_world: &mut EcsMaster, _system_meta: &mut SystemMeta) -> Self::State {
        // B1: the one-time initial value. Runs once per system in `initialize`
        // (cold path). `T::default()` is infallible.
        T::default()
    }

    #[inline]
    fn init_access(
        _state: &Self::State,
        _system_meta: &mut SystemMeta,
        _access_set: &mut FilteredAccessSet,
        _world: &mut EcsMaster,
    ) {
        // Decision D: NO access declared. `Local` is invisible to the conflict
        // graph — mirror `Commands::init_access` / `EventReader::init_access`.
        // The required-method body is intentionally empty (the trait has no
        // default body — system_param.rs:125).
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        _system_meta: &SystemMeta,
        _world: UnsafeEcsCell<'w>,
    ) -> Self::Item<'w, 's> {
        // SAFETY (SP2): pure rebind of the caller-provided exclusive
        //   `&'s mut Self::State`. No `world` access, no pointer minting, no
        //   aliasing introduced. Identical shape to `EventReader::get_param`.
        Local(state)
    }
}

// F1: `#[repr(transparent)]` over a single `&mut T` ⇒ pointer-sized.
const _: () = assert!(
    core::mem::size_of::<Local<'_, u32>>() == core::mem::size_of::<&mut u32>(),
    "Local<'s, T> must be pointer-sized (Phase 13 F1: #[repr(transparent)])",
);
```

### §3.1 Notes for the developer

- **No `unsafe` block inside any method.** The only `unsafe` is `unsafe impl
  SystemParam`, whose three-point SAFETY contract is in the comment above. No raw
  pointer dereference, nothing like `Res`'s `&*(ptr as *const R)`.
- **`get_param` is `Local(state)`** — not `Local { state }` (that is
  `EventReader`'s shape because its field is named `state`). `Local`'s field is
  the tuple position `.0`, so the constructor is `Local(state)`.
- **No `PhantomData`.** `T` appears directly in `State = T` and the field
  `&'s mut T`, so the type is bound structurally. No marker needed.
- **The `T: ... + 'static` bound** is needed because `State: 'static`
  (`system_param.rs:85`). Place it on the struct, all impls, and the `unsafe impl`
  consistently.
- **Verify the import paths** (`filtered_access_set`, `system_meta`,
  `unsafe_ecs_cell`, `ecs_master`) against `event_reader.rs` / `res.rs` `use`
  blocks — adjust if a module name differs.

---

## §4 Module placement + wiring

### §4.1 New file

`crates/boyko_ecs/src/ecs/core/system/params/local.rs` — the source in §3.

### §4.2 `params/mod.rs` — add `mod` + re-export

Mirror the `EventReader` wiring. Add the module declaration alphabetically:

```rust
pub mod local;
```

Add the re-export with the same `#[allow(unused_imports)]` rationale used by
`Commands` / `EventReader`:

```rust
// Phase 13: `Local<'s, T>` per-system private-state re-export. Same
// `#[allow(unused_imports)]` rationale as `Commands` / `EventReader`.
#[allow(unused_imports)]
pub use local::Local;
```

### §4.3 `system/mod.rs` — add to the aggregate re-export

Add `Local` to the `pub use params::{ ... }` block (alphabetical):

```rust
pub use params::{
    Commands, EventIter, EventReader, EventReaderState, EventWriter, EventWriterState,
    Local, MAX_SYSTEM_PARAM_ARITY, Res, ResMut, ResMutState, ResState,
};
```

(Verify the exact current contents of that block first — add `Local` in the right
alphabetical slot without dropping any existing name.)

### §4.4 Higher-level re-export — NONE beyond §4.3

There is no `prelude` module in `boyko_ecs` (`lib.rs` re-exports only
`EcsError`/`EcsResult`). `Res`/`Commands`/`EventReader` are reachable at exactly
one public path: `boyko_ecs::ecs::core::system::{...}`. `Local` mirrors this. The
canonical import:

```rust
use boyko_ecs::ecs::core::system::Local;
```

---

## §5 Step plan

Three steps. Each compiles independently and passes `cargo test --all-targets`.

| Step | Title | Files | What |
|------|-------|-------|------|
| **1** | Param impl | `params/local.rs` (new), `params/mod.rs` (`pub mod local;` only) | Write the entire §3 source: struct, `Deref`/`DerefMut`, conditional `Debug`, `unsafe impl SystemParam`, the `const _` size pin. Add the in-file `#[cfg(test)] mod tests` (§6.1). Also fold the R2 doc-comment fix (`system_param.rs:121-122`) into this step. After this step `cargo test --all-targets` is green. |
| **2** | Wiring | `params/mod.rs` (`pub use`), `system/mod.rs` | Add the §4.2 `pub use local::Local;` and the §4.3 aggregate re-export entry. `Local` reachable at `boyko_ecs::ecs::core::system::Local`. |
| **3** | Tests | `tests/phase13_local_systemparam.rs` (new); optional `tests/compile_fail_local/` + driver | Add the §6.2 integration tests + optional trybuild. |

**Coupling**: the `pub mod local;` line in `params/mod.rs` must land in Step 1 so
the module compiles; the `pub use` re-exports can wait for Step 2 (the
`#[allow(dead_code)]` on the module covers the unreferenced-module gap).

---

## §6 Test plan (architecture-level; tester writes bodies)

### §6.1 In-file unit tests (`params/local.rs::tests`, Step 1)

| Test | Setup | Assertion |
|------|-------|-----------|
| `local_is_system_param` | `fn assert_impl<T: SystemParam>() {}` shim (mirror `res.rs:170`) | `assert_impl::<Local<'static, u32>>()` compiles |
| `local_deref_reads_back_default` | `let mut v = 0u32; let l = Local(&mut v);` | `*l == 0` |
| `local_deref_mut_writes_through` | `let mut v = 0u32; let mut l = Local(&mut v); *l = 7;` | after drop, `v == 7` |
| `init_state_returns_default` | `<Local<'_, u32> as SystemParam>::init_state(&mut ecs, &mut meta)` | returns `0u32`; for a custom non-zero `Default`, returns that default |

### §6.2 Integration tests (`tests/phase13_local_systemparam.rs`, Step 3)

**Critical convention (R1)**: persistence requires a *cached* system. Build the
`FunctionSystem` once via `IntoSystem::into_system`, hoist it, then call
`ecs.run_cached_system(&mut sys)` repeatedly so state survives. Template:
`run_cached_system_reused_twice_reads_updated_resource` (`ecs_master.rs:2548-2587`).
**Do NOT** use `ecs.run_system(closure)` for persistence tests — it rebuilds a
fresh `FunctionSystem` (fresh default `Local`) every call (`ecs_master.rs:1459-1466`),
so it would reset the local each frame (observe 1,1,1 instead of 1,2,3).

| # | Test | Setup | Assertion |
|---|------|-------|-----------|
| 1 | `local_counter_persists_across_runs` | Hoist FS for `\|mut n: Local<u32>\| { *n += 1; probe.store(*n); }`. Call `run_cached_system` ×3. | probe reads 1, 2, 3 |
| 2 | `two_locals_same_type_independent` | Hoist FS for `\|mut a: Local<u32>, mut b: Local<u32>\| { *a += 1; *b += 10; ... }`. Call ×2. | `probe_a == 2`, `probe_b == 20` (positional distinctness) |
| 3 | `two_systems_independent_locals` | Hoist two FS, each `\|mut n: Local<u32>\| { *n += 1; ... }`. Run A ×2, B ×1. | `probe_a == 2`, `probe_b == 1` |
| 4 | `default_init_uses_default_value` | `#[derive(Default)]` (or manual `Default` giving 42) struct; hoist `\|c: Local<Counter>\| ...`; run once. | probe reads the `Default` value |
| 5 | `local_registers_no_access` | Build FS for `\|_: Local<u32>\|`. `initialize`, inspect `sys.access()`. | access is empty / does not conflict with `Access::universal()` (compare `res.rs:206-220` for the inverse). Proves no conflict-graph edge. |
| 6 | `two_local_systems_run_in_parallel` *(optional, stronger)* | Two systems, each `Local<u32>` + a shared `Res` read of the same resource; build schedule, run on 2-worker pool. | Schedule builds + runs without a conflict panic (the `Local` adds nothing; shared read is non-conflicting). If a full parallel schedule is heavier than the phase warrants, #5 alone proves the no-access claim. |

### §6.3 Compile-fail test (optional, Step 3)

| File | Expected error | Reason |
|------|----------------|--------|
| `local_non_default_rejected.rs` | `NoDefault: Default` not satisfied at the `Local<NoDefault>` use | Proves the A1+B1 bounds are enforced |

`trybuild` is already a dev-dependency. Marked optional — bound enforcement is
mechanical; tests 1-5 are mandatory.

### §6.4 Miri

Run tests 1-4 under `cargo +nightly miri test --test phase13_local_systemparam`
**single-threaded only**. Test 6 (parallel) excluded (Phase 9.1 `Scope::spawn`
deferral). Test 5 (no threads) is Miri-safe. No `unsafe` pointer work in `Local` —
Miri confirms the `&'s mut T` rebind is sound (trivially — it's a reborrow).

### §6.5 Benchmarks

**None.** Per roadmap ("No hot-path impact"); `get_param` is a single pointer
rebind. No new bench file warranted.

---

## §7 SAFETY invariants

`Local` has no `unsafe` blocks inside methods. The sole `unsafe impl SystemParam`
rests on three invariants:

1. **SP1 (no access)** — `init_access` declares nothing. Honest and complete:
   `Local` reads/writes *only* its own state slot, owned by `FunctionSystem::state`
   (function_system.rs:116) and unreachable from any other system or param.
   `Access` (access.rs:45-57) has only component/resource bitmasks. No
   conflict-graph edge. (Same class as `Commands`/`EventReader`.)
2. **SP2 (`&'s mut T` aliasing)** — `get_param` returns `Local(state)`, a pure
   rebind of the caller-provided `&'s mut Self::State`. The trait guarantees this
   borrow is exclusive for `'s`: `FunctionSystem::run_unsafe` holds the *only*
   reference to its own state slot (function_system.rs:249-265). No `world`
   touched; no aliasing minted. The `&'s mut T` cannot outlive `'s`.
3. **SP4 (`init_state`-once, no structural mutation)** — `init_state` returns
   `T::default()`, mutating no registry. Runs exactly once per system (FS1
   idempotence, function_system.rs:188-190). `initialize`'s
   `archetype_generation()` before/after assert holds vacuously.

No `debug_assert!` needed inside `Local` (the borrow rebind is unconditionally
sound). The one compile-time check is the `const _` size pin (§3).

---

## §8 Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| **R1 — Persistence test uses `run_system(closure)`** (rebuilds state each call → observes 1,1,1). | Medium (test-author trap) | §6.2 mandates the hoisted-`FunctionSystem` + `run_cached_system` pattern with the exact template cite (`ecs_master.rs:2548-2587`). |
| **R2 — `system_param.rs:121-122` doc claims `init_access` has a default body** (it does not — line 125 has no default). | Low (compiler catches it) | **Fix the comment in Phase 13, Step 1.** One-line doc edit; the only existing-source edit in the phase. Change to: "Implementations with no access — e.g. `Local<T>`, `Commands`, `EventReader` — provide an explicit empty body (this method has **no** default impl)." |
| **R3 — Roadmap says "managed by `SystemMeta`"** (loose; state lives in `FunctionSystem::state`, not `SystemMeta`). | Low | This plan is authoritative: `type State = T` lives in `FunctionSystem::state` via the param tuple. No `SystemMeta` change. |
| **R4 — `T: Send + Sync` stricter than Bevy's `Send`-only.** | Very low | Accepted (A1). Clear error; §2.1 documents the backward-compatible A2 upgrade path. No Phase 13 use case needs `Send`-but-`!Sync`. |

No open questions. All research Decisions A-F resolved (A1, B1, C = free
distinctness, D = empty `init_access`, E = `'s`-only `Item`, F1/F2 pinned).

---

**Readiness: ready for the developer (architecture-critic round skipped).**

Rationale: this phase is a structural clone of the already-critiqued, shipped,
Miri-clean `EventReader` (Phase 12) with strictly *less* surface — no cached
pointer, no `unsafe` block, no atomics, no false-sharing layout. All decisions are
resolved with documented backward-compat paths; the only existing-code edit is a
one-line doc fix; the SAFETY surface is three vacuous invariants. The single
non-obvious risk (R1) is pinned with an exact template cite. A critic round here
would be low-value process overhead on a design whose every choice is justified by
a directly analogous, already-approved precedent in the same module.
